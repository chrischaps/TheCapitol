//! The Capitol Gazette (E3.9, economy plan §1: "prices are public"): a daily
//! digest of the economy, so the market has a story anyone can read.
//!
//! At each in-game midnight (`rules::DAY_LENGTH_SECS`, two real hours) the
//! day just ended gets an edition. [`gather`] reads the day's records:
//!
//! - the Exchange's trades (`market_trades`): each item's range, its mean
//!   weighted by volume, and the day before's mean, for the movers
//! - the highest-quality sale, and who sold it. Units don't remember who made
//!   them, so the seller is the one the Gazette can name (#231).
//! - deeds claimed and foreclosed, by zone (`deed_events`, written by
//!   [`record_deed`] beside the claim and the foreclosure)
//! - what each player gathered and crafted (`production_tally`, added to by
//!   [`tally`] in the transaction that writes the work, or for automation by
//!   [`tally_many`] once its cycle's crafts commit)
//!
//! [`compose`] turns them into the edition. It's pure: the headlines come from
//! templates picked by a hash of the day, never an LLM, so a day always reads
//! the same. A player who turned their byline off (`players.gazette_named`)
//! is never printed by name.
//!
//! [`run_gazette_loop`] publishes. It catches up the day just ended on start,
//! if it has no edition, then sleeps to each midnight. Editions are kept in
//! `gazette_editions` as served, and `gazette_published` tells everyone.

use std::collections::{BTreeMap, HashMap};

use capitol_protocol::rest::{Gazette, GazetteDeeds, GazetteMover, GazettePrice, GazetteProducer, GazetteSale};
use capitol_protocol::rules::{self, Zone};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::events::GameEvent;

/// How many movers and producers an edition lists
const MOVERS: usize = 5;
const PRODUCERS: usize = 5;
/// A change in price this big (in percent, either way) makes the front page
const FRONT_PAGE_MOVE: i64 = 15;

/// How the Gazette refers to someone who keeps their name out of it
const UNNAMED: &str = "a trader who keeps out of the papers";

/// Add `quantity` of `item_type` to what `player_id` made today. Call it in
/// the transaction that writes the work, so the tally never counts a gather
/// or craft that failed.
pub async fn tally(conn: &mut PgConnection, player_id: Uuid, item_type: &str, quantity: i32) -> Result<(), sqlx::Error> {
    if quantity <= 0 {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO production_tally (day, player_id, item_type, quantity) VALUES ($1, $2, $3, $4)
         ON CONFLICT (day, player_id, item_type) DO UPDATE SET quantity = production_tally.quantity + EXCLUDED.quantity",
    )
    .bind(rules::day_number(Utc::now().timestamp_millis()))
    .bind(player_id)
    .bind(item_type)
    .bind(quantity as i64)
    .execute(conn)
    .await?;
    Ok(())
}

/// [`tally`] many at once, on the day of `at`: each of `made` is (player,
/// item type, quantity). The automation tick calls it once a cycle, after its
/// crafts commit, not in each craft's transaction: an owner's stations would
/// all wait on the same row, held through every commit (#212). A crash
/// between the two loses that cycle's tally; it never counts a craft that
/// failed.
pub async fn tally_many(
    db: impl sqlx::PgExecutor<'_>,
    at: DateTime<Utc>,
    made: &[(Uuid, &str, i32)],
) -> Result<(), sqlx::Error> {
    // One row a player and item type: an upsert can't touch a row twice
    let mut totals: BTreeMap<(Uuid, &str), i64> = BTreeMap::new();
    for (player_id, item_type, quantity) in made.iter().filter(|(.., q)| *q > 0) {
        *totals.entry((*player_id, *item_type)).or_default() += *quantity as i64;
    }
    if totals.is_empty() {
        return Ok(());
    }
    let (keys, quantities): (Vec<_>, Vec<i64>) = totals.into_iter().unzip();
    let (players, item_types): (Vec<Uuid>, Vec<&str>) = keys.into_iter().unzip();
    sqlx::query(
        "INSERT INTO production_tally (day, player_id, item_type, quantity)
         SELECT $1, p, t, q FROM unnest($2::uuid[], $3::text[], $4::bigint[]) AS u(p, t, q)
         ON CONFLICT (day, player_id, item_type) DO UPDATE SET quantity = production_tally.quantity + EXCLUDED.quantity",
    )
    .bind(rules::day_number(at.timestamp_millis()))
    .bind(players)
    .bind(item_types)
    .bind(quantities)
    .execute(db)
    .await?;
    Ok(())
}

/// What happened to a deed, for the Gazette's land column
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeedEvent {
    Claimed,
    Foreclosed,
}

/// Note a deed claimed or foreclosed. Call it in the transaction that does it.
pub async fn record_deed(
    conn: &mut PgConnection,
    plot_id: Uuid,
    zone_id: &str,
    kind: DeedEvent,
    player_id: Uuid,
) -> Result<(), sqlx::Error> {
    let kind = match kind {
        DeedEvent::Claimed => "claimed",
        DeedEvent::Foreclosed => "foreclosed",
    };
    sqlx::query("INSERT INTO deed_events (plot_id, zone_id, kind, player_id) VALUES ($1, $2, $3, $4)")
        .bind(plot_id)
        .bind(zone_id)
        .bind(kind)
        .bind(player_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// One player's work for the day, before it's ranked
#[derive(Debug, Clone, PartialEq)]
pub struct Work {
    pub player_id: Uuid,
    /// `None` if they keep their name out of the papers
    pub name: Option<String>,
    pub item_type: String,
    pub item_name: String,
    pub quantity: i64,
}

/// Everything an edition is made from: one day's records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DayRecords {
    pub day: i64,
    /// Every item that traded, with the day before's mean where it traded then
    pub prices: Vec<GazettePrice>,
    /// How many fills, and how many Strands changed hands
    pub trades: i64,
    pub strands: i64,
    pub best_sale: Option<GazetteSale>,
    /// `(zone id, claimed, foreclosed)`
    pub deeds: Vec<(String, i64, i64)>,
    pub work: Vec<Work>,
}

/// Read day `day`'s records.
pub async fn gather(db: &PgPool, day: i64) -> Result<DayRecords, sqlx::Error> {
    let at = |ms: i64| DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default();
    let (start, end) = (at(rules::day_start_ms(day)), at(rules::day_start_ms(day + 1)));
    let before = at(rules::day_start_ms(day - 1));

    type PriceRow = (String, String, i64, i64, i64, i64, i64, i64);
    let price_sql = "SELECT t.item_type, it.name, MIN(t.price), MAX(t.price),
                            ROUND(SUM(t.price * t.quantity)::NUMERIC / SUM(t.quantity))::BIGINT,
                            SUM(t.quantity)::BIGINT, COUNT(*), SUM(t.price * t.quantity)::BIGINT
                     FROM market_trades t JOIN item_types it ON it.id = t.item_type
                     WHERE t.at >= $1 AND t.at < $2
                     GROUP BY t.item_type, it.name";
    let today: Vec<PriceRow> = sqlx::query_as(price_sql).bind(start).bind(end).fetch_all(db).await?;
    let yesterday: HashMap<String, i64> = sqlx::query_as::<_, PriceRow>(price_sql)
        .bind(before)
        .bind(start)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|row| (row.0, row.4))
        .collect();
    let (mut trades, mut strands) = (0, 0);
    let mut prices: Vec<GazettePrice> = today
        .into_iter()
        .map(|(item_type, item_name, low, high, mean, volume, fills, value)| {
            trades += fills;
            strands += value;
            let previous = yesterday.get(&item_type).copied();
            GazettePrice { item_type, item_name, low, high, mean, volume, previous }
        })
        .collect();
    prices.sort_by(|a, b| a.item_name.cmp(&b.item_name).then_with(|| a.item_type.cmp(&b.item_type)));

    type SaleRow = (String, String, i32, String, i64, i32, String, bool);
    let best: Option<SaleRow> = sqlx::query_as(
        "SELECT t.item_type, it.name, t.quality, t.grade, t.price, t.quantity, p.name, p.gazette_named
         FROM market_trades t
         JOIN item_types it ON it.id = t.item_type
         JOIN market_orders o ON o.id = t.sell_order_id
         JOIN players p ON p.id = o.player_id
         WHERE t.at >= $1 AND t.at < $2
         ORDER BY t.quality DESC, t.price DESC, t.at, t.id
         LIMIT 1",
    )
    .bind(start)
    .bind(end)
    .fetch_optional(db)
    .await?;
    let best_sale = best.map(|(item_type, item_name, quality, grade, price, quantity, seller, named)| GazetteSale {
        item_type,
        item_name,
        quality,
        grade,
        price,
        quantity,
        seller: named.then_some(seller),
    });

    let deed_rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT zone_id, kind, COUNT(*) FROM deed_events WHERE at >= $1 AND at < $2 GROUP BY zone_id, kind",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;
    let mut deeds: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for (zone, kind, n) in deed_rows {
        let entry = deeds.entry(zone).or_default();
        match kind.as_str() {
            "claimed" => entry.0 += n,
            _ => entry.1 += n,
        }
    }

    let work: Vec<(Uuid, String, bool, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT pt.player_id, p.name, p.gazette_named, pt.item_type, it.name, pt.quantity
         FROM production_tally pt
         JOIN players p ON p.id = pt.player_id
         LEFT JOIN item_types it ON it.id = pt.item_type
         WHERE pt.day = $1",
    )
    .bind(day)
    .fetch_all(db)
    .await?;

    Ok(DayRecords {
        day,
        prices,
        trades,
        strands,
        best_sale,
        deeds: deeds.into_iter().map(|(zone, (claimed, foreclosed))| (zone, claimed, foreclosed)).collect(),
        work: work
            .into_iter()
            .map(|(player_id, name, named, item_type, item_name, quantity)| Work {
                player_id,
                name: named.then_some(name),
                item_name: item_name.unwrap_or_else(|| item_type.clone()),
                item_type,
                quantity,
            })
            .collect(),
    })
}

/// The day's movers: items that traded both days, by the size of their
/// change, the biggest first
pub fn movers(prices: &[GazettePrice]) -> Vec<GazetteMover> {
    let mut movers: Vec<GazetteMover> = prices
        .iter()
        .filter_map(|p| {
            let previous = p.previous.filter(|&v| v > 0)?;
            Some(GazetteMover {
                item_type: p.item_type.clone(),
                item_name: p.item_name.clone(),
                previous,
                mean: p.mean,
                change_pct: (p.mean - previous) * 100 / previous,
            })
        })
        .collect();
    movers.sort_by(|a, b| {
        b.change_pct
            .abs()
            .cmp(&a.change_pct.abs())
            .then_with(|| b.change_pct.cmp(&a.change_pct))
            .then_with(|| a.item_name.cmp(&b.item_name))
    });
    movers.truncate(MOVERS);
    movers
}

/// The day's top producers, by every unit they made
pub fn producers(work: &[Work]) -> Vec<GazetteProducer> {
    let mut by_player: BTreeMap<Uuid, GazetteProducer> = BTreeMap::new();
    for w in work {
        let p = by_player.entry(w.player_id).or_insert_with(|| GazetteProducer {
            name: w.name.clone(),
            ..Default::default()
        });
        p.units += w.quantity;
        let better = w.quantity > p.item_units || (w.quantity == p.item_units && w.item_name < p.item_name);
        if better {
            (p.item_type, p.item_name, p.item_units) = (w.item_type.clone(), w.item_name.clone(), w.quantity);
        }
    }
    let mut producers: Vec<(Uuid, GazetteProducer)> = by_player.into_iter().filter(|(_, p)| p.units > 0).collect();
    producers.sort_by(|(a_id, a), (b_id, b)| b.units.cmp(&a.units).then_with(|| a_id.cmp(b_id)));
    producers.into_iter().take(PRODUCERS).map(|(_, p)| p).collect()
}

/// Compose day `records.day`'s edition. The same records always make the same
/// edition, word for word.
pub fn compose(records: &DayRecords) -> Gazette {
    let day = records.day;
    let pen = Pen { day };
    let movers = movers(&records.prices);
    let producers = producers(&records.work);
    let deeds: Vec<GazetteDeeds> = deeds_by_zone(&records.deeds);
    let claimed: i64 = deeds.iter().map(|d| d.claimed).sum();
    let foreclosed: i64 = deeds.iter().map(|d| d.foreclosed).sum();
    let weekday = rules::weekday(day);

    let markets = pen.markets(records, &movers);
    let best_sale_story = pen.best_sale(records.best_sale.as_ref());
    let deeds_story = pen.deeds(&deeds, claimed, foreclosed);
    let producers_story = pen.producers(&producers);

    // The front page: the biggest story the day has
    let big_move = movers.first().filter(|m| m.change_pct.abs() >= FRONT_PAGE_MOVE);
    let fine_sale = records.best_sale.as_ref().filter(|s| s.grade == "A");
    let (headline, dek, lead) = if let Some(m) = big_move {
        pen.move_story(m, records)
    } else if let Some(sale) = fine_sale {
        pen.sale_story(sale)
    } else if foreclosed > 0 {
        pen.foreclosure_story(&deeds, foreclosed)
    } else if claimed > 0 {
        pen.claim_story(&deeds, claimed)
    } else if let Some(top) = producers.first() {
        pen.work_story(top)
    } else {
        pen.quiet_story(weekday)
    };

    Gazette {
        day,
        number: rules::gazette_number(day),
        headline,
        dek,
        lead,
        weather: pen.weather(),
        markets,
        movers,
        prices: records.prices.clone(),
        best_sale: records.best_sale.clone(),
        best_sale_story,
        deeds,
        deeds_story,
        producers,
        producers_story,
    }
}

/// Deeds by zone, from the centre out; a zone the Gazette doesn't know last
fn deeds_by_zone(deeds: &[(String, i64, i64)]) -> Vec<GazetteDeeds> {
    let mut out: Vec<GazetteDeeds> = deeds
        .iter()
        .filter(|(_, c, f)| *c > 0 || *f > 0)
        .map(|(zone, claimed, foreclosed)| GazetteDeeds {
            zone: zone.clone(),
            zone_name: Zone::from_str(zone).map_or_else(|| zone.clone(), |z| z.display_name().to_string()),
            claimed: *claimed,
            foreclosed: *foreclosed,
        })
        .collect();
    let order = |zone: &str| Zone::from_str(zone).map_or(usize::MAX, |z| Zone::ALL.iter().position(|a| *a == z).unwrap_or(0));
    out.sort_by(|a, b| order(&a.zone).cmp(&order(&b.zone)).then_with(|| a.zone.cmp(&b.zone)));
    out
}

/// The Gazette's writer: picks among templates by a hash of the day, so each
/// day has its own wording and keeps it.
struct Pen {
    day: i64,
}

impl Pen {
    /// One of `options`, the same for this day and `salt` every time
    fn pick<'a>(&self, salt: u64, options: &[&'a str]) -> &'a str {
        let mut z = (self.day as u64) ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        // SplitMix64's finaliser
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        options[(z % options.len() as u64) as usize]
    }

    fn move_story(&self, m: &GazetteMover, records: &DayRecords) -> (String, String, String) {
        let pct = m.change_pct.abs();
        let volume = records.prices.iter().find(|p| p.item_type == m.item_type).map_or(0, |p| p.volume);
        let f = |template: &str| fill(template, m, pct, volume, self.day);
        if m.change_pct > 0 {
            let headline = self.pick(1, &["{item} Climbs {pct}%", "{item} Up {pct}% at the Exchange", "Buyers Chase {item}, Up {pct}%"]);
            let dek = self.pick(2, &[
                "Bids crowd the hall as the price runs from {from} to {to}",
                "From {from} Strands to {to}: sellers name their price",
                "A day of eager buyers lifts {item} to {to}",
            ]);
            let lead = self.pick(3, &[
                "{item} closed the day at a mean of {to} Strands, against {from} the day before. {volume} changed hands at the Exchange, and the floor wanted more. Those holding stock may find this a good week to sell; those who need it may wish they had bought on {yesterday}.",
                "It was a seller's day for {item}. The mean price rose from {from} to {to} Strands over {volume} units, a rise of {pct}%. Whether it holds will depend on who goes out to make more of it.",
            ]);
            (f(headline), f(dek), f(lead))
        } else {
            let headline = self.pick(1, &["{item} Slides {pct}%", "{item} Off {pct}% as Sellers Crowd the Hall", "A Glut of {item}: Down {pct}%"]);
            let dek = self.pick(2, &[
                "From {from} Strands to {to}, and few takers",
                "Sellers outnumber buyers as {item} falls to {to}",
                "The mean drops to {to} on a heavy day",
            ]);
            let lead = self.pick(3, &[
                "{item} fetched a mean of {to} Strands, down from {from} the day before. {volume} changed hands as sellers undercut one another to reach the few buyers there were. Those who make things of it will find their inputs cheap.",
                "Plenty of {item} came to the Exchange, and the price gave way: {from} Strands down to {to}, {pct}% off, over {volume} units. Crafters who use it may want to stock up while it lasts.",
            ]);
            (f(headline), f(dek), f(lead))
        }
    }

    fn sale_story(&self, sale: &GazetteSale) -> (String, String, String) {
        let who = sale.seller.clone().unwrap_or_else(|| UNNAMED.to_string());
        let headline = self
            .pick(4, &["A-Grade {item} Fetches {price}", "Fine {item} Sells for {price} a Unit", "Quality Tells: {item} at {price}"])
            .replace("{item}", &sale.item_name)
            .replace("{price}", &sale.price.to_string());
        let dek = self
            .pick(5, &["The day's finest work changes hands at quality {quality}", "Quality {quality}, and a price to match"])
            .replace("{quality}", &sale.quality.to_string());
        let lead = self
            .pick(6, &[
                "The best goods sold at the Exchange today were {quantity} {item} of quality {quality}, sold by {who} at {price} Strands each. Good work is noticed in the Capitol, and paid for.",
                "{Who} brought {quantity} {item} of quality {quality} to the Exchange and sold at {price} Strands a unit. No better goods traded all day.",
            ])
            .replace("{quantity}", &sale.quantity.to_string())
            .replace("{item}", &sale.item_name)
            .replace("{quality}", &sale.quality.to_string())
            .replace("{price}", &sale.price.to_string())
            .replace("{Who}", &capitalise(&who))
            .replace("{who}", &who);
        (headline, dek, lead)
    }

    fn foreclosure_story(&self, deeds: &[GazetteDeeds], foreclosed: i64) -> (String, String, String) {
        let worst = deeds.iter().max_by_key(|d| (d.foreclosed, std::cmp::Reverse(d.zone.clone()))).map(|d| d.zone_name.as_str()).unwrap_or("");
        let headline = if foreclosed == 1 {
            self.pick(7, &["A Deed Lost to Unpaid Tax", "The Tax Office Takes Back a Lot"]).to_string()
        } else {
            self.pick(7, &["{n} Deeds Lost to Unpaid Tax", "The Tax Office Takes Back {n} Lots"]).replace("{n}", &count_word(foreclosed, true))
        };
        let dek = format!("Most of the trouble in the {worst}");
        let lead = self
            .pick(8, &[
                "Four weeks of unpaid tax is the limit, and today {n} reached it. The land goes back on the market and the deed is refunded, less the debt. The Gazette reminds its readers that tax can be paid at any time from the plot's own panel.",
                "The tax office foreclosed on {n} today. Whatever stood on the land has come down, and the lots are free for whoever wants them next.",
            ])
            .replace("{n}", &plural(foreclosed, "lot", "lots"));
        (headline, dek, lead)
    }

    fn claim_story(&self, deeds: &[GazetteDeeds], claimed: i64) -> (String, String, String) {
        let busiest = deeds.iter().max_by_key(|d| (d.claimed, std::cmp::Reverse(d.zone.clone()))).map(|d| d.zone_name.as_str()).unwrap_or("");
        let headline = if claimed == 1 {
            self.pick(9, &["A New Deed in the {zone}", "Someone Settles in the {zone}"]).replace("{zone}", busiest)
        } else {
            self.pick(9, &["{n} New Deeds, Most in the {zone}", "The Land Office Has a Busy Day"])
                .replace("{n}", &count_word(claimed, true))
                .replace("{zone}", busiest)
        };
        let dek = self.pick(10, &["The city grows by a lot at a time", "Stakes driven, deeds signed"]).to_string();
        let lead = self
            .pick(11, &[
                "The land office recorded {n} today, the most of them in the {zone}. A deed is a promise to pay tax every hour, so new owners will be busy.",
                "{N} changed from open ground to someone's land today. Look out for new workshops in the {zone}.",
            ])
            .replace("{n}", &plural(claimed, "new deed", "new deeds"))
            .replace("{N}", &capitalise(&plural(claimed, "lot", "lots")))
            .replace("{zone}", busiest);
        (headline, dek, lead)
    }

    fn work_story(&self, top: &GazetteProducer) -> (String, String, String) {
        let who = top.name.clone().unwrap_or_else(|| UNNAMED.to_string());
        let headline = match &top.name {
            Some(name) => self
                .pick(12, &["{name} Leads the Day's Work", "A Hard Day's Work for {name}"])
                .replace("{name}", name),
            None => "An Unnamed Hand Leads the Day's Work".to_string(),
        };
        let dek = format!("{} units made, {} of them {}", thousands(top.units), thousands(top.item_units), top.item_name);
        let lead = self
            .pick(13, &[
                "Nothing much moved at the Exchange today, but the city kept working. {Who} made the most: {units} units in all, chiefly {item}.",
                "A quiet market, and busy hands. {Who} gathered and crafted {units} units, more than anyone, most of it {item}.",
            ])
            .replace("{Who}", &capitalise(&who))
            .replace("{units}", &thousands(top.units))
            .replace("{item}", &top.item_name);
        (headline, dek, lead)
    }

    fn quiet_story(&self, weekday: &str) -> (String, String, String) {
        let headline = self.pick(14, &["A Quiet {weekday} in the Capitol", "All Calm on {weekday}"]).replace("{weekday}", weekday);
        let dek = self.pick(15, &["No trades, no deeds, no news", "The city sleeps, and so does the market"]).to_string();
        let lead = self
            .pick(16, &[
                "Nothing traded at the Exchange, no deeds were claimed and none were lost. The Gazette went to press anyway. A quiet day is a chance: the first to list goods tomorrow will set the price.",
                "It was a still day. The Exchange's clock kept time over an empty floor, and the land office kept its ink dry. Whoever trades tomorrow trades first.",
            ])
            .to_string();
        (headline, dek, lead)
    }

    fn markets(&self, records: &DayRecords, movers: &[GazetteMover]) -> String {
        if records.prices.is_empty() {
            return self
                .pick(17, &[
                    "No trades at the Exchange today. The book stands open for whoever comes first.",
                    "The Exchange floor was empty today: not a unit changed hands.",
                ])
                .to_string();
        }
        let busiest = records.prices.iter().max_by(|a, b| a.volume.cmp(&b.volume).then_with(|| b.item_name.cmp(&a.item_name)));
        let mut text = format!(
            "{} at the Exchange today, {} Strands in all, across {}.",
            plural(records.trades, "trade", "trades"),
            thousands(records.strands),
            plural(records.prices.len() as i64, "good", "goods"),
        );
        if let Some(b) = busiest {
            text.push_str(&format!(" The busiest was {}, with {} units at a mean of {}.", b.item_name, thousands(b.volume), b.mean));
        }
        match movers.iter().find(|m| m.change_pct != 0) {
            Some(m) if m.change_pct > 0 => {
                text.push_str(&format!(" {} rose most, {}% to {}.", m.item_name, m.change_pct, m.mean))
            }
            Some(m) => text.push_str(&format!(" {} fell furthest, {}% to {}.", m.item_name, -m.change_pct, m.mean)),
            None if movers.is_empty() => text.push_str(" Nothing traded on both days, so there's nothing to compare yet."),
            None => text.push_str(" Prices held where they were the day before."),
        }
        text
    }

    fn best_sale(&self, sale: Option<&GazetteSale>) -> String {
        match sale {
            None => "With nothing sold, the prize for the day's finest goods goes unclaimed.".to_string(),
            Some(s) => {
                let who = s.seller.clone().unwrap_or_else(|| UNNAMED.to_string());
                format!(
                    "The finest goods sold today: {} {} of quality {} (grade {}), at {} Strands each, sold by {}.",
                    count_word(s.quantity as i64, false), s.item_name, s.quality, s.grade, s.price, who
                )
            }
        }
    }

    fn deeds(&self, deeds: &[GazetteDeeds], claimed: i64, foreclosed: i64) -> String {
        if deeds.is_empty() {
            return self
                .pick(18, &[
                    "No deeds changed hands. The land office kept its doors open and its ink dry.",
                    "Not a lot claimed nor lost today.",
                ])
                .to_string();
        }
        let zones: Vec<String> = deeds
            .iter()
            .map(|d| match (d.claimed, d.foreclosed) {
                (c, 0) => format!("{} claimed in the {}", count_word(c, false), d.zone_name),
                (0, f) => format!("{} foreclosed in the {}", count_word(f, false), d.zone_name),
                (c, f) => format!(
                    "{} claimed and {} foreclosed in the {}",
                    count_word(c, false),
                    count_word(f, false),
                    d.zone_name
                ),
            })
            .collect();
        let total = match (claimed, foreclosed) {
            (c, 0) => format!("{} recorded", plural(c, "new deed", "new deeds")),
            (0, f) => format!("{} lost to unpaid tax", plural(f, "deed", "deeds")),
            (c, f) => format!(
                "{} recorded and {} lost to unpaid tax",
                plural(c, "new deed", "new deeds"),
                count_word(f, false)
            ),
        };
        format!("{}: {}.", capitalise(&total), zones.join("; "))
    }

    fn producers(&self, producers: &[GazetteProducer]) -> String {
        let Some(top) = producers.first() else {
            return "Nobody gathered or crafted a thing today. Even the grass had the day off.".to_string();
        };
        let who = top.name.clone().unwrap_or_else(|| UNNAMED.to_string());
        let mut text = format!(
            "{} made the most today: {} units, chiefly {} ({}).",
            capitalise(&who),
            thousands(top.units),
            top.item_name,
            thousands(top.item_units)
        );
        if producers.len() > 1 {
            let rest: i64 = producers[1..].iter().map(|p| p.units).sum();
            text.push_str(&format!(" The next {} made {} between them.", count_word(producers.len() as i64 - 1, false), thousands(rest)));
        }
        text
    }

    /// A line on the sky, from the day/night clock: the sun rises at a
    /// quarter of the day and sets at three quarters, as every client draws it
    fn weather(&self) -> String {
        let sky = self.pick(19, &[
            "Fair over the rings",
            "Clear and still",
            "High, thin cloud over the ranges",
            "A bright, cold morning",
            "Mist on the moat at dawn, burnt off by mid-morning",
            "A soft haze over the wilds",
        ]);
        let night = self.pick(20, &[
            "the tower's lantern lit at dusk",
            "stars over the wilds after dark",
            "the moon up over the eastern ranges by night",
            "lamps in the urban windows from seven",
        ]);
        format!("{sky}. Sunrise at six, sunset at eighteen; {night}.")
    }
}

/// A mover's template, filled in
fn fill(template: &str, m: &GazetteMover, pct: i64, volume: i64, day: i64) -> String {
    template
        .replace("{item}", &m.item_name)
        .replace("{pct}", &pct.to_string())
        .replace("{from}", &m.previous.to_string())
        .replace("{to}", &m.mean.to_string())
        .replace("{volume}", &plural(volume, "unit", "units"))
        .replace("{yesterday}", rules::weekday(day - 1))
}

/// `1,234,567`
fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// "one lot", "3 lots": small counts in words, as a paper prints them
fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{} {}", count_word(n, false), if n == 1 { one } else { many })
}

/// A count as the Gazette prints it: words up to ten, figures above
fn count_word(n: i64, title: bool) -> String {
    const WORDS: [&str; 11] = ["no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
    match usize::try_from(n).ok().and_then(|i| WORDS.get(i)) {
        Some(w) if title => capitalise(w),
        Some(w) => w.to_string(),
        None => thousands(n),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Compose and keep day `day`'s edition. `Some` if it's new; `None` if the day
/// already had one (another start, or a race), which is left as it was.
pub async fn publish(db: &PgPool, day: i64) -> Result<Option<Gazette>, sqlx::Error> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM gazette_editions WHERE day = $1)")
        .bind(day)
        .fetch_one(db)
        .await?;
    if exists {
        return Ok(None);
    }
    let edition = compose(&gather(db, day).await?);
    let json = serde_json::to_value(&edition).expect("an edition serialises");
    let inserted = sqlx::query("INSERT INTO gazette_editions (day, edition) VALUES ($1, $2) ON CONFLICT (day) DO NOTHING")
        .bind(day)
        .bind(json)
        .execute(db)
        .await?
        .rows_affected();
    Ok((inserted == 1).then_some(edition))
}

/// Day `day`'s edition, or with `None` the latest
pub async fn edition(db: &PgPool, day: Option<i64>) -> Result<Option<Gazette>, sqlx::Error> {
    let row: Option<sqlx::types::Json<Gazette>> = match day {
        Some(day) => sqlx::query_scalar("SELECT edition FROM gazette_editions WHERE day = $1").bind(day).fetch_optional(db).await?,
        None => {
            sqlx::query_scalar("SELECT edition FROM gazette_editions ORDER BY day DESC LIMIT 1")
                .fetch_optional(db)
                .await?
        }
    };
    Ok(row.map(|json| json.0))
}

/// Publish each day's edition at the midnight that ends it, for as long as the
/// server runs. On start it catches up the day just ended, if it has none.
pub async fn run_gazette_loop(db: PgPool, events: broadcast::Sender<GameEvent>) {
    loop {
        let now = Utc::now().timestamp_millis();
        let today = rules::day_number(now);
        match publish(&db, today - 1).await {
            Ok(Some(edition)) => {
                tracing::info!("The Gazette, No. {}: {}", edition.number, edition.headline);
                let _ = events.send(GameEvent::GazettePublished {
                    day: edition.day,
                    number: edition.number,
                    headline: edition.headline,
                });
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!("The Gazette for day {} failed: {e}", today - 1);
                // Try again in a minute rather than wait a whole day
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                continue;
            }
        }
        // A moment past the next midnight, so the day's last trades are in
        let wait = rules::day_start_ms(today + 1) - Utc::now().timestamp_millis() + 2000;
        tokio::time::sleep(std::time::Duration::from_millis(wait.max(1000) as u64)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = rules::GAZETTE_FIRST_DAY + 12;

    fn price(item: &str, mean: i64, volume: i64, previous: Option<i64>) -> GazettePrice {
        GazettePrice {
            item_type: item.to_lowercase(),
            item_name: item.into(),
            low: (mean - 2).max(1),
            high: mean + 3,
            mean,
            volume,
            previous,
        }
    }

    fn work(id: u128, name: Option<&str>, item: &str, quantity: i64) -> Work {
        Work {
            player_id: Uuid::from_u128(id),
            name: name.map(Into::into),
            item_type: item.to_lowercase(),
            item_name: item.into(),
            quantity,
        }
    }

    /// A day with something in every section
    fn busy_day() -> DayRecords {
        DayRecords {
            day: DAY,
            prices: vec![price("Plank", 41, 120, Some(30)), price("Plant Fiber", 1, 900, Some(1)), price("Iron Ingot", 60, 8, None)],
            trades: 37,
            strands: 6300,
            best_sale: Some(GazetteSale {
                item_type: "iron_axe".into(),
                item_name: "Iron Axe".into(),
                quality: 91,
                grade: "A".into(),
                price: 410,
                quantity: 1,
                seller: Some("Ada".into()),
            }),
            deeds: vec![("rural".into(), 3, 0), ("suburban".into(), 1, 1)],
            work: vec![
                work(1, Some("Ada"), "Plant Fiber", 300),
                work(1, Some("Ada"), "Rope", 40),
                work(2, None, "Logs", 500),
                work(3, Some("Cy"), "Plank", 20),
            ],
        }
    }

    #[test]
    fn a_busy_day_fills_every_section() {
        let g = compose(&busy_day());
        assert_eq!((g.day, g.number), (DAY, 13));
        for (section, text) in [
            ("headline", &g.headline),
            ("dek", &g.dek),
            ("lead", &g.lead),
            ("weather", &g.weather),
            ("markets", &g.markets),
            ("best sale", &g.best_sale_story),
            ("deeds", &g.deeds_story),
            ("producers", &g.producers_story),
        ] {
            assert!(!text.is_empty(), "{section} is empty");
            assert!(!text.contains('{'), "{section} has a hole: {text}");
        }
        assert!(!g.movers.is_empty() && !g.prices.is_empty() && !g.deeds.is_empty() && !g.producers.is_empty());
        assert!(g.best_sale.is_some());
        // Planks rose 36%, which makes the front page
        assert_eq!(g.movers[0].item_type, "plank");
        assert_eq!(g.movers[0].change_pct, 36);
        assert!(g.headline.contains("Plank") && g.headline.contains("36"), "{}", g.headline);
    }

    #[test]
    fn a_quiet_day_still_fills_every_section() {
        let g = compose(&DayRecords { day: DAY, ..Default::default() });
        for text in [&g.headline, &g.dek, &g.lead, &g.weather, &g.markets, &g.best_sale_story, &g.deeds_story, &g.producers_story] {
            assert!(!text.is_empty() && !text.contains('{'), "{text}");
        }
        assert!(g.headline.contains(rules::weekday(DAY)), "{}", g.headline);
    }

    #[test]
    fn a_day_reads_the_same_every_time_and_days_differ() {
        assert_eq!(compose(&busy_day()), compose(&busy_day()));
        let weathers: std::collections::BTreeSet<String> =
            (0..20).map(|d| compose(&DayRecords { day: DAY + d, ..Default::default() }).weather).collect();
        assert!(weathers.len() > 4, "the weather hardly varies: {weathers:?}");
    }

    #[test]
    fn the_front_page_takes_the_biggest_story() {
        let mut day = busy_day();
        // No big move: the A-grade sale leads
        day.prices = vec![price("Plank", 31, 120, Some(30))];
        assert!(compose(&day).headline.contains("Iron Axe") || compose(&day).headline.contains("410"));
        // No fine sale: the foreclosure
        day.best_sale.as_mut().unwrap().grade = "B".into();
        assert!(compose(&day).lead.contains("foreclosed") || compose(&day).lead.contains("tax"), "{}", compose(&day).lead);
        // No foreclosure: the claims
        day.deeds = vec![("rural".into(), 3, 0)];
        assert!(compose(&day).headline.contains("Rural") || compose(&day).headline.contains("Land Office"));
        // No deeds: the work
        day.deeds.clear();
        day.best_sale = None;
        assert!(compose(&day).dek.contains("units made"), "{}", compose(&day).dek);
    }

    #[test]
    fn movers_are_ranked_by_the_size_of_their_change() {
        let prices = vec![price("A", 90, 1, Some(100)), price("B", 150, 1, Some(100)), price("C", 5, 1, None), price("D", 100, 1, Some(100))];
        let m = movers(&prices);
        assert_eq!(m.iter().map(|m| (m.item_name.as_str(), m.change_pct)).collect::<Vec<_>>(), [("B", 50), ("A", -10), ("D", 0)]);
    }

    #[test]
    fn producers_are_ranked_by_units_and_named_only_with_leave() {
        let p = producers(&busy_day().work);
        assert_eq!(p.iter().map(|p| p.units).collect::<Vec<_>>(), [500, 340, 20]);
        assert_eq!(p[0].name, None);
        assert_eq!((p[1].name.as_deref(), p[1].item_name.as_str(), p[1].item_units), (Some("Ada"), "Plant Fiber", 300));
        let g = compose(&busy_day());
        assert!(g.producers_story.starts_with("A trader who keeps out of the papers"), "{}", g.producers_story);
    }

    #[test]
    fn deeds_run_from_the_centre_out() {
        let d = deeds_by_zone(&[("wilderness".into(), 1, 0), ("urban".into(), 0, 2), ("rural".into(), 0, 0)]);
        assert_eq!(d.iter().map(|d| d.zone.as_str()).collect::<Vec<_>>(), ["urban", "wilderness"]);
        assert_eq!(d[0].zone_name, "Urban Zone");
    }

    #[test]
    fn numbers_read_as_a_paper_prints_them() {
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(999), "999");
        assert_eq!(plural(1, "lot", "lots"), "one lot");
        assert_eq!(plural(12, "lot", "lots"), "12 lots");
        assert_eq!(count_word(3, true), "Three");
    }

    /// Writes `protocol/tests/fixtures/rest/gazette.json` from the busy day.
    /// Run with `--ignored` when the edition's shape changes.
    #[test]
    #[ignore = "writes the protocol's fixture"]
    fn write_the_gazette_fixture() {
        let response = capitol_protocol::rest::GazetteResponse { edition: Some(compose(&busy_day())), named: true };
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../protocol/tests/fixtures/rest/gazette.json");
        std::fs::write(path, serde_json::to_string(&response).unwrap() + "\n").unwrap();
    }

    mod db {
        use super::super::*;
        use crate::engine::market::tests::player;

        /// A fill of `quantity` `item` at `price`, from `(seller, buyer)`, at `at`
        async fn trade(db: &PgPool, (seller, buyer): (Uuid, Uuid), item: &str, quality: i32, price: i64, quantity: i32, at: DateTime<Utc>) {
            let order = |side: &'static str, player: Uuid| {
                let id = Uuid::new_v4();
                let q = sqlx::query(
                    "INSERT INTO market_orders (id, player_id, side, item_type, min_grade, grade_rule, price,
                         quantity_total, quantity_remaining, status, fee, expires_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, 'filled', 1, NOW() + INTERVAL '7 days')",
                )
                .bind(id)
                .bind(player)
                .bind(side)
                .bind(item)
                .bind((side == "buy").then_some("F"))
                .bind((side == "buy").then_some("average"))
                .bind(price)
                .bind(quantity);
                (id, q)
            };
            let (sell, q) = order("sell", seller);
            q.execute(db).await.unwrap();
            let (buy, q) = order("buy", buyer);
            q.execute(db).await.unwrap();
            sqlx::query(
                "INSERT INTO market_trades (id, buy_order_id, sell_order_id, item_type, grade, quality, min_quality, price, quantity, at)
                 VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8, $9)",
            )
            .bind(Uuid::new_v4())
            .bind(buy)
            .bind(sell)
            .bind(item)
            .bind(rules::quality_grade(quality))
            .bind(quality)
            .bind(price)
            .bind(quantity)
            .bind(at)
            .execute(db)
            .await
            .unwrap();
        }

        /// A sim day: trades on two days, a claim and a foreclosure, work by
        /// two players, one of whom keeps their name out of it. The edition
        /// fills every section, and a second publish leaves it be.
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_sim_day_makes_an_edition_with_every_section_filled(db: PgPool) {
            let ada = player(&db, "Ada", 0).await;
            let bo = player(&db, "Bo", 0).await;
            sqlx::query("UPDATE players SET gazette_named = FALSE WHERE id = $1").bind(bo.id).execute(&db).await.unwrap();

            let day = rules::day_number(Utc::now().timestamp_millis()) - 1;
            let at = |d: i64, hour: i64| DateTime::<Utc>::from_timestamp_millis(rules::day_start_ms(d) + hour * 300_000).unwrap();
            trade(&db, (ada.id, bo.id), "fiber", 50, 2, 100, at(day - 1, 3)).await;
            trade(&db, (ada.id, bo.id), "fiber", 60, 3, 100, at(day, 3)).await;
            trade(&db, (bo.id, ada.id), "rope", 92, 9, 5, at(day, 5)).await;
            trade(&db, (ada.id, bo.id), "rope", 40, 4, 5, at(day + 1, 5)).await; // the next day: not in it

            let plots: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, zone_id FROM plots LIMIT 2").fetch_all(&db).await.unwrap();
            if plots.len() == 2 {
                let mut conn = db.acquire().await.unwrap();
                record_deed(&mut conn, plots[0].0, &plots[0].1, DeedEvent::Claimed, ada.id).await.unwrap();
                record_deed(&mut conn, plots[1].0, &plots[1].1, DeedEvent::Foreclosed, bo.id).await.unwrap();
                sqlx::query("UPDATE deed_events SET at = $1").bind(at(day, 7)).execute(&mut *conn).await.unwrap();
            }
            for (player, item, n) in [(ada.id, "fiber", 300), (ada.id, "rope", 20), (bo.id, "fiber", 50)] {
                sqlx::query("INSERT INTO production_tally (day, player_id, item_type, quantity) VALUES ($1, $2, $3, $4)")
                    .bind(day)
                    .bind(player)
                    .bind(item)
                    .bind(n as i64)
                    .execute(&db)
                    .await
                    .unwrap();
            }

            let g = publish(&db, day).await.unwrap().expect("a new edition");
            assert_eq!(g.prices.len(), 2);
            let fiber = g.prices.iter().find(|p| p.item_type == "fiber").unwrap();
            assert_eq!((fiber.mean, fiber.volume, fiber.previous), (3, 100, Some(2)));
            assert_eq!(g.movers[0].change_pct, 50);
            let sale = g.best_sale.as_ref().unwrap();
            assert_eq!((sale.item_type.as_str(), sale.quality, sale.seller.as_deref()), ("rope", 92, None));
            assert_eq!(g.producers[0].name.as_deref(), Some("Ada"));
            assert_eq!((g.producers[0].units, g.producers[1].name.as_deref()), (320, None));
            if plots.len() == 2 {
                assert_eq!(g.deeds.iter().map(|d| d.claimed + d.foreclosed).sum::<i64>(), 2);
            }
            for text in [&g.headline, &g.dek, &g.lead, &g.weather, &g.markets, &g.best_sale_story, &g.deeds_story, &g.producers_story] {
                assert!(!text.is_empty() && !text.contains('{'), "{text}");
            }

            assert_eq!(publish(&db, day).await.unwrap(), None);
            assert_eq!(edition(&db, None).await.unwrap(), Some(g.clone()));
            assert_eq!(edition(&db, Some(day)).await.unwrap(), Some(g));
            assert_eq!(edition(&db, Some(day - 5)).await.unwrap(), None);
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn the_tally_adds_up(db: PgPool) {
            let ada = player(&db, "Ada", 0).await;
            let mut conn = db.acquire().await.unwrap();
            for n in [3, 4, 0] {
                tally(&mut conn, ada.id, "fiber", n).await.unwrap();
            }
            let total: i64 = sqlx::query_scalar("SELECT quantity FROM production_tally WHERE player_id = $1")
                .bind(ada.id)
                .fetch_one(&mut *conn)
                .await
                .unwrap();
            assert_eq!(total, 7);
        }
    }
}
