//! The Capitol Gazette (E3.9, economy plan §1: "prices are public"): a daily
//! digest of the economy, so the market has a story anyone can read.
//!
//! At each in-game midnight (`rules::DAY_LENGTH_SECS`, two real hours) the
//! day just ended gets an edition. [`gather`] reads the day's records:
//!
//! - the Exchange's trades (`market_trades`): each item's range, its mean
//!   weighted by volume, and the day before's mean, for the movers
//! - the highest-quality sale, who sold it and who made it: the maker of
//!   most of the units sold, from their mark (`market_trades.maker_id`,
//!   #231). Unmarked units credit the seller alone.
//! - deeds claimed and foreclosed, by zone (`deed_events`, written by
//!   [`record_deed`] beside the claim and the foreclosure)
//! - what each player gathered and crafted (`production_tally`, added to by
//!   [`tally`] in the transaction that writes the work, or for automation by
//!   [`tally_many`] once its cycle's crafts commit)
//! - what sold at the bazaar's stalls (`bazaar_sales`, written by
//!   `bazaar::buy`, #240): counts and the busiest shops, never a price,
//!   since the bazaar keeps its prices between buyer and seller (D6). Its
//!   Strands count in the day's selling (`GazetteShares::sold`).
//! - the hall's board (commons S2.4): what was still wanted at the day's
//!   end, what was posted, filled and expired, each delivery
//!   (`commission_deliveries`, written by `commissions::deliver`), and the
//!   claims kept and lapsed. A lapse is only ever counted, never named.
//!
//! [`compose`] turns them into the edition. It's pure: the headlines come from
//! templates picked by a hash of the day, never an LLM, so a day always reads
//! the same. A player who turned their byline off (`players.gazette_named`)
//! is never printed by name.
//!
//! Citizens (C2.2, Citizens D3) are in it like anyone else, by name, as
//! sellers, makers and producers, and always marked: the prose calls them
//! "Citizen Ada", and the lists carry a `citizen` flag for the client's
//! glyph. On a day citizens did anything, the edition also counts them
//! apart from people ([`GazetteShares`], and a line in the markets, deeds
//! and work stories), so a reader can see how much of the economy is theirs.
//!
//! [`run_gazette_loop`] publishes. It catches up the day just ended on start,
//! if it has no edition, then sleeps to each midnight. Editions are kept in
//! `gazette_editions` as served, and `gazette_published` tells everyone.

use std::collections::{BTreeMap, HashMap};

use capitol_protocol::rest::{
    Gazette, GazetteDeeds, GazetteDelivery, GazetteMade, GazetteMover, GazettePrice, GazetteProducer, GazetteSale,
    GazetteShares, GazetteShop, GazetteStalls, GazetteWant, GazetteWanted,
};
use capitol_protocol::annotation::AnnotationKind;
use capitol_protocol::rules::{self, Zone};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::events::GameEvent;

/// How many movers and producers an edition lists
const MOVERS: usize = 5;
const PRODUCERS: usize = 5;
/// How many of the things made most an edition lists
const MADE: usize = 6;
/// How many of the bazaar's busiest shops an edition lists
const SHOPS: usize = 3;
/// How many of the board's wants, and of what went begging, an edition lists
const WANTS: usize = 5;
const BEGGING: usize = 3;
/// A change in price this big (in percent, either way) makes the front page
const FRONT_PAGE_MOVE: i64 = 15;

/// How the Gazette refers to someone who keeps their name out of it
const UNNAMED: &str = "a trader who keeps out of the papers";
const UNNAMED_CITIZEN: &str = "a citizen who keeps out of the papers";
/// How the Wanted column refers to a poster or deliverer who keeps their
/// name out of it (S2.4)
const RESIDENT: &str = "a resident";
const RESIDENT_CITIZEN: &str = "a citizen";

/// How the Gazette names someone in its prose: "Ada", "Citizen Cy", or one of
/// the unnamed, a citizen still marked as one
fn name_of(name: Option<&str>, citizen: bool) -> String {
    match (name, citizen) {
        (Some(name), false) => name.to_string(),
        (Some(name), true) => format!("Citizen {name}"),
        (None, false) => UNNAMED.to_string(),
        (None, true) => UNNAMED_CITIZEN.to_string(),
    }
}

/// How the Wanted column names a poster or deliverer: by name with their
/// leave, otherwise "a resident" (or "a citizen", still marked as one)
fn resident_of(name: Option<&str>, citizen: bool) -> String {
    match (name, citizen) {
        (Some(_), _) => name_of(name, citizen),
        (None, false) => RESIDENT.to_string(),
        (None, true) => RESIDENT_CITIZEN.to_string(),
    }
}

/// How the Gazette names a shop: "“The Rope Walk”, kept by Ada". A keeper
/// who keeps out of the papers keeps their shop out too, as its sign may
/// say whose it is.
fn shop_of(shop: &GazetteShop) -> String {
    match (&shop.name, &shop.keeper) {
        (Some(name), Some(keeper)) => format!("“{name}”, kept by {}", name_of(Some(keeper), shop.citizen)),
        _ if shop.citizen => "a shop whose citizen keeper keeps out of the papers".to_string(),
        _ => "a shop whose keeper keeps out of the papers".to_string(),
    }
}

/// Who to thank for a sale: "made and sold by Ada", "made by Ada and sold by
/// Bo", or "sold by Bo" when the units carry no mark the Gazette may print
fn credit(sale: &GazetteSale) -> String {
    let seller = name_of(sale.seller.as_deref(), sale.seller_citizen);
    match (&sale.maker, sale.own_work) {
        (_, true) => format!("made and sold by {seller}"),
        (Some(maker), false) => format!("made by {} and sold by {seller}", name_of(Some(maker), sale.maker_citizen)),
        (None, false) => format!("sold by {seller}"),
    }
}

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
    /// Claimed with the road in's deed grant (#440): a settler, not a buyer
    Granted,
    Foreclosed,
    /// Claimed by a guild as its hall (S4.3), its leader signing
    Hall(Uuid),
}

/// Note a deed claimed or foreclosed. Call it in the transaction that does it.
pub async fn record_deed(
    conn: &mut PgConnection,
    plot_id: Uuid,
    zone_id: &str,
    kind: DeedEvent,
    player_id: Uuid,
) -> Result<(), sqlx::Error> {
    let (kind, guild_id) = match kind {
        DeedEvent::Claimed => ("claimed", None),
        DeedEvent::Granted => ("granted", None),
        DeedEvent::Foreclosed => ("foreclosed", None),
        DeedEvent::Hall(guild_id) => ("hall", Some(guild_id)),
    };
    sqlx::query("INSERT INTO deed_events (plot_id, zone_id, kind, player_id, guild_id) VALUES ($1, $2, $3, $4, $5)")
        .bind(plot_id)
        .bind(zone_id)
        .bind(kind)
        .bind(player_id)
        .bind(guild_id)
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
    /// A citizen, named or not
    pub citizen: bool,
    pub item_type: String,
    pub item_name: String,
    pub quantity: i64,
}

/// One merchant's sales of one good at the bazaar for the day, before
/// they're ranked
#[derive(Debug, Clone, PartialEq)]
pub struct StallTrade {
    pub merchant_id: Uuid,
    /// Their shop's name at their last sale of the day
    pub shop: String,
    /// `None` if they keep their name out of the papers
    pub keeper: Option<String>,
    /// A citizen, named or not
    pub citizen: bool,
    pub item_type: String,
    pub item_name: String,
    pub sales: i64,
    pub units: i64,
    pub strands: i64,
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
    /// How many of those Strands went to sellers who are people
    pub strands_to_people: i64,
    pub best_sale: Option<GazetteSale>,
    /// `(zone id, claimed, foreclosed)`
    pub deeds: Vec<(String, i64, i64)>,
    /// How many of the deeds claimed went to people
    pub claimed_by_people: i64,
    pub work: Vec<Work>,
    /// What sold at the bazaar's stalls (#240)
    pub stalls: Vec<StallTrade>,
    /// Who settled with the road in's deed (#448), in the order they did
    pub settlers: Vec<Settler>,
    /// The guilds that took a hall (S4.3), in the order they did
    pub halls: Vec<String>,
    /// The day on the hall's board (S2.4)
    pub board: Board,
}

/// The day on the hall's board (commons S2.4), before it's ranked
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Board {
    /// Each commission still open at the day's end, with what it still wanted
    pub open: Vec<Asked>,
    /// Each commission that expired today, with what it never got
    pub expired: Vec<Asked>,
    /// The day's deliveries, in the order they were made
    pub deliveries: Vec<GazetteDelivery>,
    /// Commissions posted and filled today
    pub posted: i64,
    pub filled: i64,
    /// Claims that ended today kept, and lapsed
    pub kept: i64,
    pub lapsed: i64,
}

impl Board {
    fn is_empty(&self) -> bool {
        *self == Board::default()
    }
}

/// One commission's ask: `units` of an item at `price` each
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Asked {
    pub item_type: String,
    pub item_name: String,
    pub units: i64,
    pub price: i64,
}

/// Someone who claimed a home with the deed the road in earned (#448)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settler {
    /// Their name, if they let the Gazette print it
    pub name: Option<String>,
    pub citizen: bool,
    pub zone: String,
    /// The plot's middle, server units
    pub x: f64,
    pub y: f64,
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

    let strands_to_people: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(t.price * t.quantity), 0)::BIGINT
         FROM market_trades t
         JOIN market_orders o ON o.id = t.sell_order_id
         JOIN players p ON p.id = o.player_id
         LEFT JOIN accounts a ON a.id = p.account_id
         WHERE t.at >= $1 AND t.at < $2 AND a.kind IS DISTINCT FROM 'citizen'",
    )
    .bind(start)
    .bind(end)
    .fetch_one(db)
    .await?;

    type SaleRow = (String, String, i32, String, i64, i32, String, bool, bool, bool, Option<String>, Option<bool>, bool);
    let best: Option<SaleRow> = sqlx::query_as(
        "SELECT t.item_type, it.name, t.quality, t.grade, t.price, t.quantity, p.name, p.gazette_named,
                COALESCE(pa.kind = 'citizen', FALSE), t.maker_id IS NOT DISTINCT FROM o.player_id,
                m.name, m.gazette_named, COALESCE(ma.kind = 'citizen', FALSE)
         FROM market_trades t
         JOIN item_types it ON it.id = t.item_type
         JOIN market_orders o ON o.id = t.sell_order_id
         JOIN players p ON p.id = o.player_id
         LEFT JOIN accounts pa ON pa.id = p.account_id
         LEFT JOIN players m ON m.id = t.maker_id
         LEFT JOIN accounts ma ON ma.id = m.account_id
         WHERE t.at >= $1 AND t.at < $2
         ORDER BY t.quality DESC, t.price DESC, t.at, t.id
         LIMIT 1",
    )
    .bind(start)
    .bind(end)
    .fetch_optional(db)
    .await?;
    let best_sale = best.map(
        |(item_type, item_name, quality, grade, price, quantity, seller, named, seller_citizen, own_work, maker, maker_named, maker_citizen)| {
            // A maker who keeps out of the papers isn't credited at all
            let maker = maker.filter(|_| !own_work && maker_named == Some(true));
            GazetteSale {
                item_type,
                item_name,
                quality,
                grade,
                price,
                quantity,
                seller: named.then_some(seller),
                maker_citizen: maker_citizen && maker.is_some(),
                maker,
                own_work,
                seller_citizen,
            }
        },
    );

    let deed_rows: Vec<(String, String, bool, i64)> = sqlx::query_as(
        "SELECT d.zone_id, d.kind, COALESCE(a.kind = 'citizen', FALSE), COUNT(*)
         FROM deed_events d
         LEFT JOIN players p ON p.id = d.player_id
         LEFT JOIN accounts a ON a.id = p.account_id
         WHERE d.at >= $1 AND d.at < $2
         GROUP BY 1, 2, 3",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;
    let mut deeds: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut claimed_by_people = 0;
    for (zone, kind, citizen, n) in deed_rows {
        let entry = deeds.entry(zone).or_default();
        // A grant's claim is a claim in the land column (#440), and so is a
        // guild's hall (S4.3)
        match kind.as_str() {
            "claimed" | "granted" | "hall" => {
                entry.0 += n;
                if !citizen {
                    claimed_by_people += n;
                }
            }
            _ => entry.1 += n,
        }
    }

    let work: Vec<(Uuid, String, bool, bool, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT pt.player_id, p.name, p.gazette_named, COALESCE(a.kind = 'citizen', FALSE),
                pt.item_type, it.name, pt.quantity
         FROM production_tally pt
         JOIN players p ON p.id = pt.player_id
         LEFT JOIN accounts a ON a.id = p.account_id
         LEFT JOIN item_types it ON it.id = pt.item_type
         WHERE pt.day = $1",
    )
    .bind(day)
    .fetch_all(db)
    .await?;

    let settlers: Vec<(String, bool, bool, String, f32, f32)> = sqlx::query_as(
        "SELECT p.name, p.gazette_named, COALESCE(a.kind = 'citizen', FALSE), d.zone_id, pl.world_x, pl.world_y
         FROM deed_events d
         JOIN players p ON p.id = d.player_id
         LEFT JOIN accounts a ON a.id = p.account_id
         JOIN plots pl ON pl.id = d.plot_id
         WHERE d.kind = 'granted' AND d.at >= $1 AND d.at < $2
         ORDER BY d.at, d.id",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;

    let halls: Vec<String> = sqlx::query_scalar(
        "SELECT g.name FROM deed_events d JOIN guilds g ON g.id = d.guild_id
         WHERE d.kind = 'hall' AND d.at >= $1 AND d.at < $2
         ORDER BY d.at, d.id",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;

    type StallRow = (Uuid, String, String, bool, bool, String, Option<String>, i64, i64, i64);
    let stalls: Vec<StallRow> = sqlx::query_as(
        "SELECT s.merchant_id,
                (SELECT l.shop_name FROM bazaar_sales l
                 WHERE l.merchant_id = s.merchant_id AND l.at >= $1 AND l.at < $2
                 ORDER BY l.at DESC, l.id LIMIT 1),
                p.name, p.gazette_named, COALESCE(a.kind = 'citizen', FALSE),
                s.item_type, it.name, COUNT(*), SUM(s.quantity)::BIGINT, SUM(s.price * s.quantity)::BIGINT
         FROM bazaar_sales s
         JOIN players p ON p.id = s.merchant_id
         LEFT JOIN accounts a ON a.id = p.account_id
         LEFT JOIN item_types it ON it.id = s.item_type
         WHERE s.at >= $1 AND s.at < $2
         GROUP BY s.merchant_id, p.name, p.gazette_named, a.kind, s.item_type, it.name",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;

    let board = gather_board(db, (start, end)).await?;

    Ok(DayRecords {
        day,
        board,
        prices,
        trades,
        strands,
        strands_to_people,
        best_sale,
        deeds: deeds.into_iter().map(|(zone, (claimed, foreclosed))| (zone, claimed, foreclosed)).collect(),
        claimed_by_people,
        settlers: settlers
            .into_iter()
            .map(|(name, named, citizen, zone, x, y)| Settler {
                name: named.then_some(name),
                citizen,
                zone,
                x: x as f64,
                y: y as f64,
            })
            .collect(),
        halls,
        work: work
            .into_iter()
            .map(|(player_id, name, named, citizen, item_type, item_name, quantity)| Work {
                player_id,
                name: named.then_some(name),
                citizen,
                item_name: item_name.unwrap_or_else(|| item_type.clone()),
                item_type,
                quantity,
            })
            .collect(),
        stalls: stalls
            .into_iter()
            .map(|(merchant_id, shop, name, named, citizen, item_type, item_name, sales, units, strands)| StallTrade {
                merchant_id,
                shop,
                keeper: named.then_some(name),
                citizen,
                item_name: item_name.unwrap_or_else(|| item_type.clone()),
                item_type,
                sales,
                units,
                strands,
            })
            .collect(),
    })
}

/// Read the hall's board for the day `start..end` (S2.4).
async fn gather_board(db: &PgPool, (start, end): (DateTime<Utc>, DateTime<Utc>)) -> Result<Board, sqlx::Error> {
    let asked = |(item_type, item_name, units, price): (String, Option<String>, i64, i64)| Asked {
        item_name: item_name.unwrap_or_else(|| item_type.clone()),
        item_type,
        units,
        price,
    };
    // Open at the day's end: posted before it and not closed by it. What it
    // still wanted then is what it wants now and what was delivered since.
    let open: Vec<(String, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT c.item_type, it.name,
                (c.quantity - c.delivered + COALESCE((SELECT SUM(d.quantity) FROM commission_deliveries d
                                                      WHERE d.commission_id = c.id AND d.at >= $1), 0))::BIGINT,
                c.price_per_unit
         FROM commissions c LEFT JOIN item_types it ON it.id = c.item_type
         WHERE c.posted_at < $1 AND (c.closed_at IS NULL OR c.closed_at >= $1) AND NOT c.withdrawn
         ORDER BY c.posted_at, c.id",
    )
    .bind(end)
    .fetch_all(db)
    .await?;
    let expired: Vec<(String, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT c.item_type, it.name, (c.quantity - c.delivered)::BIGINT, c.price_per_unit
         FROM commissions c LEFT JOIN item_types it ON it.id = c.item_type
         WHERE c.status = 'expired' AND c.closed_at >= $1 AND c.closed_at < $2
         ORDER BY c.closed_at, c.id",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;
    let (posted, filled): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE posted_at >= $1 AND posted_at < $2),
                COUNT(*) FILTER (WHERE status = 'filled' AND closed_at >= $1 AND closed_at < $2)
         FROM commissions",
    )
    .bind(start)
    .bind(end)
    .fetch_one(db)
    .await?;
    let (kept, lapsed): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE status = 'kept'), COUNT(*) FILTER (WHERE status = 'lapsed')
         FROM commission_claims WHERE ended_at >= $1 AND ended_at < $2",
    )
    .bind(start)
    .bind(end)
    .fetch_one(db)
    .await?;
    type DeliveryRow = (String, bool, bool, String, bool, bool, String, Option<String>, i32, i64);
    let deliveries: Vec<DeliveryRow> = sqlx::query_as(
        "SELECT dp.name, dp.gazette_named, COALESCE(da.kind = 'citizen', FALSE),
                pp.name, pp.gazette_named, COALESCE(pa.kind = 'citizen', FALSE),
                c.item_type, it.name, d.quantity, d.paid
         FROM commission_deliveries d
         JOIN commissions c ON c.id = d.commission_id
         JOIN players dp ON dp.id = d.deliverer_id
         LEFT JOIN accounts da ON da.id = dp.account_id
         JOIN players pp ON pp.id = c.poster_id
         LEFT JOIN accounts pa ON pa.id = pp.account_id
         LEFT JOIN item_types it ON it.id = c.item_type
         WHERE d.at >= $1 AND d.at < $2
         ORDER BY d.at, d.id",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;
    Ok(Board {
        open: open.into_iter().map(asked).filter(|a| a.units > 0).collect(),
        expired: expired.into_iter().map(asked).filter(|a| a.units > 0).collect(),
        deliveries: deliveries
            .into_iter()
            .map(|(deliverer, d_named, d_citizen, poster, p_named, p_citizen, item_type, item_name, units, paid)| {
                GazetteDelivery {
                    deliverer: d_named.then_some(deliverer),
                    deliverer_citizen: d_citizen,
                    poster: p_named.then_some(poster),
                    poster_citizen: p_citizen,
                    item_name: item_name.unwrap_or_else(|| item_type.clone()),
                    item_type,
                    units: units as i64,
                    paid,
                }
            })
            .collect(),
        posted,
        filled,
        kept,
        lapsed,
    })
}

/// Asks by item: the most Strands down first, then by name
fn wants(asked: &[Asked]) -> Vec<GazetteWant> {
    let mut by_item: BTreeMap<&str, GazetteWant> = BTreeMap::new();
    for a in asked.iter().filter(|a| a.units > 0) {
        let w = by_item.entry(&a.item_type).or_insert_with(|| GazetteWant {
            item_type: a.item_type.clone(),
            item_name: a.item_name.clone(),
            ..Default::default()
        });
        w.commissions += 1;
        w.units += a.units;
        w.value += a.units * a.price;
        w.best_price = w.best_price.max(a.price);
    }
    let mut wants: Vec<GazetteWant> = by_item.into_values().collect();
    wants.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.item_name.cmp(&b.item_name)));
    wants
}

/// The Wanted column, or `None` on a day with nothing on the board or done
/// with there
pub fn wanted(board: &Board, day: i64) -> Option<GazetteWanted> {
    if board.is_empty() {
        return None;
    }
    let mut wanted = wants(&board.open);
    wanted.truncate(WANTS);
    let mut gone_begging = wants(&board.expired);
    gone_begging.truncate(BEGGING);
    // The most paid, then the most units, and of equals the first made
    let came_through = board
        .deliveries
        .iter()
        .enumerate()
        .max_by(|(i, a), (j, b)| a.paid.cmp(&b.paid).then_with(|| a.units.cmp(&b.units)).then_with(|| j.cmp(i)))
        .map(|(_, d)| d.clone());
    let mut column = GazetteWanted {
        story: String::new(),
        wanted,
        came_through,
        gone_begging,
        posted: board.posted,
        filled: board.filled,
        expired: board.expired.len() as i64,
        deliveries: board.deliveries.len() as i64,
        paid: board.deliveries.iter().map(|d| d.paid).sum(),
        kept: board.kept,
        lapsed: board.lapsed,
    };
    column.story = Pen { day }.wanted(&column);
    Some(column)
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
            citizen: w.citizen,
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

/// What the city made most of, by units, with people's part of each
pub fn made(work: &[Work]) -> Vec<GazetteMade> {
    let mut by_item: BTreeMap<&str, GazetteMade> = BTreeMap::new();
    for w in work.iter().filter(|w| w.quantity > 0) {
        let m = by_item.entry(&w.item_type).or_insert_with(|| GazetteMade {
            item_type: w.item_type.clone(),
            item_name: w.item_name.clone(),
            ..Default::default()
        });
        m.units += w.quantity;
        if !w.citizen {
            m.by_people += w.quantity;
        }
    }
    let mut made: Vec<GazetteMade> = by_item.into_values().collect();
    made.sort_by(|a, b| b.units.cmp(&a.units).then_with(|| a.item_name.cmp(&b.item_name)));
    made.truncate(MADE);
    made
}

/// The day at the bazaar, or `None` if nothing sold there. Shops rank by the
/// Strands they took, which the edition doesn't print (D6).
pub fn stalls(trades: &[StallTrade]) -> Option<GazetteStalls> {
    let trades: Vec<&StallTrade> = trades.iter().filter(|t| t.sales > 0).collect();
    if trades.is_empty() {
        return None;
    }
    // Each shop's Strands, its record, and its units of each good
    let mut by_shop: BTreeMap<Uuid, (i64, GazetteShop, BTreeMap<(&str, &str), i64>)> = BTreeMap::new();
    for t in &trades {
        let (strands, shop, goods) = by_shop.entry(t.merchant_id).or_insert_with(|| {
            let shop = GazetteShop {
                name: t.keeper.as_ref().map(|_| t.shop.clone()),
                keeper: t.keeper.clone(),
                citizen: t.citizen,
                ..Default::default()
            };
            (0, shop, BTreeMap::new())
        });
        *strands += t.strands;
        shop.sales += t.sales;
        shop.units += t.units;
        *goods.entry((&t.item_name, &t.item_type)).or_default() += t.units;
    }
    let mut shops: Vec<(Uuid, i64, GazetteShop)> = by_shop
        .into_iter()
        .map(|(id, (strands, mut shop, goods))| {
            // The most units, and of equals the first by name
            let chief = goods.into_iter().max_by(|(a, a_units), (b, b_units)| a_units.cmp(b_units).then_with(|| b.cmp(a)));
            if let Some(((name, item_type), _)) = chief {
                (shop.item_name, shop.item_type) = (name.to_string(), item_type.to_string());
            }
            (id, strands, shop)
        })
        .collect();
    shops.sort_by(|(a_id, a_strands, a), (b_id, b_strands, b)| {
        b_strands.cmp(a_strands).then_with(|| b.units.cmp(&a.units)).then_with(|| a_id.cmp(b_id))
    });
    let goods: std::collections::BTreeSet<&str> = trades.iter().map(|t| t.item_type.as_str()).collect();
    Some(GazetteStalls {
        sales: trades.iter().map(|t| t.sales).sum(),
        units: trades.iter().map(|t| t.units).sum(),
        strands: trades.iter().map(|t| t.strands).sum(),
        goods: goods.len() as i64,
        shops: shops.into_iter().take(SHOPS).map(|(.., shop)| shop).collect(),
    })
}

/// The day split between people and citizens, or `None` if citizens made,
/// sold and claimed nothing
pub fn shares(records: &DayRecords) -> Option<GazetteShares> {
    let made = |people_only: bool| -> i64 {
        records.work.iter().filter(|w| !(people_only && w.citizen)).map(|w| w.quantity.max(0)).sum()
    };
    let at_stalls = |people_only: bool| -> i64 {
        records.stalls.iter().filter(|t| !(people_only && t.citizen)).map(|t| t.strands).sum()
    };
    let shares = GazetteShares {
        made: made(false),
        made_by_people: made(true),
        sold: records.strands + at_stalls(false),
        sold_by_people: records.strands_to_people + at_stalls(true),
        claimed: records.deeds.iter().map(|(_, claimed, _)| claimed).sum(),
        claimed_by_people: records.claimed_by_people,
    };
    let citizens = shares.made_by_people < shares.made
        || shares.sold_by_people < shares.sold
        || shares.claimed_by_people < shares.claimed;
    citizens.then_some(shares)
}

/// Compose day `records.day`'s edition. The same records always make the same
/// edition, word for word.
pub fn compose(records: &DayRecords) -> Gazette {
    let day = records.day;
    let pen = Pen { day };
    let movers = movers(&records.prices);
    let producers = producers(&records.work);
    let made = made(&records.work);
    let shares = shares(records);
    let stalls = stalls(&records.stalls);
    let wanted = wanted(&records.board, day);
    let deeds: Vec<GazetteDeeds> = deeds_by_zone(&records.deeds);
    let claimed: i64 = deeds.iter().map(|d| d.claimed).sum();
    let foreclosed: i64 = deeds.iter().map(|d| d.foreclosed).sum();
    let weekday = rules::weekday(day);

    let mut markets = pen.markets(records, &movers);
    let mut stalls_story = pen.stalls(stalls.as_ref());
    let best_sale_story = pen.best_sale(records.best_sale.as_ref());
    let mut deeds_story = pen.deeds(&deeds, claimed, foreclosed);
    let mut producers_story = pen.producers(&producers);
    // People's part of the day, against citizens'
    if let Some(s) = &shares {
        // The day's selling is summed up after the last place it happened
        match &stalls {
            Some(_) => stalls_story.push_str(&sold_share(s, records.strands > 0)),
            None => markets.push_str(&sold_share(s, false)),
        }
        deeds_story.push_str(&claimed_share(s));
        producers_story.push_str(&made_share(s, made.first()));
    }
    deeds_story.push_str(&settled(&records.settlers));
    deeds_story.push_str(&halls_taken(&records.halls));

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
    } else if let Some(st) = &stalls {
        pen.stalls_story(st)
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
        made,
        shares,
        stalls,
        stalls_story,
        wanted,
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
        let who = name_of(sale.seller.as_deref(), sale.seller_citizen);
        let made = match (&sale.maker, sale.own_work) {
            (_, true) => " The work was their own.".to_string(),
            (Some(maker), false) => format!(" The work was {}'s.", name_of(Some(maker), sale.maker_citizen)),
            (None, false) => String::new(),
        };
        let headline = self
            .pick(4, &["A-Grade {item} Fetches {price}", "Fine {item} Sells for {price} a Unit", "Quality Tells: {item} at {price}"])
            .replace("{item}", &sale.item_name)
            .replace("{price}", &sale.price.to_string());
        let dek = self
            .pick(5, &["The day's finest work changes hands at quality {quality}", "Quality {quality}, and a price to match"])
            .replace("{quality}", &sale.quality.to_string());
        let lead = self
            .pick(6, &[
                "The best goods sold at the Exchange today were {quantity} {item} of quality {quality}, {credit}, at {price} Strands each. Good work is noticed in the Capitol, and paid for.",
                "{Who} brought {quantity} {item} of quality {quality} to the Exchange and sold at {price} Strands a unit.{made} No better goods traded all day.",
            ])
            .replace("{quantity}", &sale.quantity.to_string())
            .replace("{item}", &sale.item_name)
            .replace("{quality}", &sale.quality.to_string())
            .replace("{price}", &sale.price.to_string())
            .replace("{credit}", &credit(sale))
            .replace("{made}", &made)
            .replace("{Who}", &capitalise(&who));
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
        let who = name_of(top.name.as_deref(), top.citizen);
        let headline = match &top.name {
            Some(_) => self
                .pick(12, &["{name} Leads the Day's Work", "A Hard Day's Work for {name}"])
                .replace("{name}", &who),
            None if top.citizen => "An Unnamed Citizen Leads the Day's Work".to_string(),
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

    fn stalls_story(&self, st: &GazetteStalls) -> (String, String, String) {
        let headline = self
            .pick(21, &["Trade at the Stalls", "A Busy Day in the Bazaar", "{N} at the Bazaar's Stalls"])
            .replace("{N}", &capitalise(&plural(st.sales, "Sale", "Sales")));
        let dek = self
            .pick(22, &["What things fetched, only the buyers know", "Prices stay between buyer and seller, as the bazaar likes them"])
            .to_string();
        let busiest = st.shops.first().map_or_else(String::new, |shop| {
            format!(" The busiest shop was {}, selling chiefly {}.", shop_of(shop), shop.item_name)
        });
        let lead = self
            .pick(23, &[
                "The bazaar's lane was busy today: {sales} across its stalls, {units} in all.{busiest} What anything fetched there stays between buyer and seller; the way to find out is to walk the lane.",
                "Merchants at the bazaar made {sales} today, {units} changing hands across the counters.{busiest} The Gazette can't print their prices, because the stalls don't post them.",
            ])
            .replace("{sales}", &plural(st.sales, "sale", "sales"))
            .replace("{units}", &plural(st.units, "unit", "units"))
            .replace("{busiest}", &busiest);
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

    /// The bazaar's day: what sold and who sold most, never at what price
    fn stalls(&self, stalls: Option<&GazetteStalls>) -> String {
        let Some(st) = stalls else {
            return self
                .pick(24, &[
                    "Not a sale at the bazaar's stalls today; the lane was for strolling.",
                    "The bazaar's counters stood idle: nothing sold at the stalls.",
                ])
                .to_string();
        };
        let mut text = format!(
            "{} at the bazaar's stalls today, {} and {} Strands in all, across {}.",
            capitalise(&plural(st.sales, "sale", "sales")),
            plural(st.units, "unit", "units"),
            thousands(st.strands),
            plural(st.goods, "good", "goods"),
        );
        if let Some(top) = st.shops.first() {
            text.push_str(&format!(" The busiest was {}, chiefly {}.", shop_of(top), top.item_name));
        }
        if st.shops.len() > 1 {
            let rest: i64 = st.shops[1..].iter().map(|s| s.sales).sum();
            text.push_str(&format!(
                " The next {} made {} between them.",
                count_word(st.shops.len() as i64 - 1, false),
                plural(rest, "sale", "sales")
            ));
        }
        text.push_str(self.pick(25, &[
            " What anything fetched stays between buyer and seller.",
            " The stalls keep their prices to themselves: walk the lane to learn them.",
        ]));
        text
    }

    fn best_sale(&self, sale: Option<&GazetteSale>) -> String {
        match sale {
            None => "With nothing sold, the prize for the day's finest goods goes unclaimed.".to_string(),
            Some(s) => format!(
                "The finest goods sold today: {} {} of quality {} (grade {}), at {} Strands each, {}.",
                count_word(s.quantity as i64, false), s.item_name, s.quality, s.grade, s.price, credit(s)
            ),
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
        let who = name_of(top.name.as_deref(), top.citizen);
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

    /// The Wanted column's prose (S2.4): what the board still wants, who
    /// came through, the claims kept, and what went begging. Lapses are
    /// counted, never named.
    fn wanted(&self, w: &GazetteWanted) -> String {
        let mut text = match w.wanted.first() {
            Some(top) => {
                let mut text = self
                    .pick(26, &[
                        "The board in the hall still wants {units} {item}, at up to {price} Strands a unit, with {value} Strands down for it.",
                        "Most wanted on the hall's board tonight: {units} {item}, {value} Strands put down for it, up to {price} a unit.",
                    ])
                    .replace("{units}", &thousands(top.units))
                    .replace("{item}", &top.item_name)
                    .replace("{price}", &thousands(top.best_price))
                    .replace("{value}", &thousands(top.value));
                let also: Vec<String> =
                    w.wanted[1..].iter().take(2).map(|a| format!("{} {}", thousands(a.units), a.item_name)).collect();
                if !also.is_empty() {
                    text.push_str(&format!(" Also wanted: {}.", also.join(" and ")));
                }
                text
            }
            None => self
                .pick(26, &[
                    "Nothing stands wanted on the hall's board tonight.",
                    "The hall's board is bare tonight: whatever was asked for has been brought or taken down.",
                ])
                .to_string(),
        };
        match (w.posted, w.filled) {
            (0, 0) => {}
            (p, 0) => text.push_str(&format!(" {} went up today.", capitalise(&plural(p, "commission", "commissions")))),
            (0, f) => text.push_str(&format!(" {} filled today.", capitalise(&plural(f, "commission was", "commissions were")))),
            (p, f) => text.push_str(&format!(
                " {} went up today, and {} filled.",
                capitalise(&plural(p, "commission", "commissions")),
                count_word(f, false)
            )),
        }
        if let Some(d) = &w.came_through {
            let who = resident_of(d.deliverer.as_deref(), d.deliverer_citizen);
            let line = self
                .pick(27, &[
                    " {Who} came through for {whom} with {units} {item}, and was paid {paid} Strands from the court's escrow.",
                    " The day's biggest delivery: {units} {item}, brought by {who} for {whom}, {paid} Strands paid on the spot.",
                ])
                .replace("{Who}", &capitalise(&who))
                .replace("{who}", &who)
                .replace("{whom}", &resident_of(d.poster.as_deref(), d.poster_citizen))
                .replace("{units}", &thousands(d.units))
                .replace("{item}", &d.item_name)
                .replace("{paid}", &thousands(d.paid));
            text.push_str(&line);
            if w.deliveries > 1 {
                text.push_str(&format!(
                    " In all, {} paid {} Strands.",
                    plural(w.deliveries, "delivery", "deliveries"),
                    thousands(w.paid)
                ));
            }
        }
        match (w.kept, w.lapsed) {
            (0, 0) => {}
            (k, 0) => text.push_str(&format!(" {} kept, every one delivered in time.", capitalise(&plural(k, "claim was", "claims were")))),
            (0, l) => text.push_str(&format!(" {} ran out undelivered.", capitalise(&plural(l, "claim", "claims")))),
            (k, l) => text.push_str(&format!(
                " {} kept, and {} ran out.",
                capitalise(&plural(k, "claim was", "claims were")),
                count_word(l, false)
            )),
        }
        if let Some(begging) = w.gone_begging.first() {
            let line = self
                .pick(28, &[
                    " Gone begging: {units} {item} at {price} a unit, unfilled when its time ran out. Whoever makes it may find buyers waiting.",
                    " Nobody brought {units} {item}, though {price} Strands a unit was on offer, and the commission ran out. There's a want there for whoever can fill it.",
                ])
                .replace("{units}", &thousands(begging.units))
                .replace("{item}", &begging.item_name)
                .replace("{price}", &thousands(begging.best_price));
            text.push_str(&line);
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

/// The day's Strands from selling, people's against citizens'. `both` when
/// they were spent at the Exchange and the stalls, so the line says so.
fn sold_share(s: &GazetteShares, both: bool) -> String {
    let place = if both { " spent at the Exchange and the stalls" } else { "" };
    match (s.sold, s.sold_by_people) {
        (0, _) => String::new(),
        (all, people) if people == all => " Every Strand went to people.".to_string(),
        (_, 0) => " Every Strand went to citizens.".to_string(),
        (all, people) => format!(
            " Of the {} Strands{place}, {} went to people and {} to citizens.",
            thousands(all),
            thousands(people),
            thousands(all - people)
        ),
    }
}

/// The new deeds, people's against citizens'
/// How many settlers the land column names before it counts the rest
const SETTLERS_NAMED: usize = 4;

/// Who settled with the road in's deed (#448): " Settled: Ada, a farm out
/// to the south-west; Citizen Bo, a yard north-east of the tower." Roads
/// have no names yet, so a home is placed by its bearing from the tower
/// ([#483](https://github.com/chrischaps/TheCapitol/issues/483)).
fn settled(settlers: &[Settler]) -> String {
    if settlers.is_empty() {
        return String::new();
    }
    let homes: Vec<String> = settlers
        .iter()
        .take(SETTLERS_NAMED)
        .map(|s| format!("{}, {}", name_of(s.name.as_deref(), s.citizen), home_of(s)))
        .collect();
    let more = settlers.len().saturating_sub(SETTLERS_NAMED);
    let tail = match more {
        0 => String::new(),
        1 => "; and one more".to_string(),
        n => format!("; and {} more", count_word(n as i64, false)),
    };
    format!(" Settled, with the deed the road in earns: {}{tail}.", homes.join("; "))
}

/// The guilds that took a hall (S4.3), by name: " The Weavers' Guild took a
/// hall in the guild district." or " The Ore Guild, The Ropewalk and The
/// Weavers' Guild took halls in the guild district."
fn halls_taken(guilds: &[String]) -> String {
    match guilds {
        [] => String::new(),
        [one] => format!(" {one} took a hall in the guild district."),
        [rest @ .., last] => format!(" {} and {last} took halls in the guild district.", rest.join(", ")),
    }
}

/// "a farm out to the south-west", "a house on a street to the north",
/// "a yard north-east of the tower"
fn home_of(s: &Settler) -> String {
    let bearing = compass(s.x - rules::CENTER_X, s.y - rules::CENTER_Y);
    match Zone::from_str(&s.zone) {
        Some(Zone::Rural) => format!("a farm out to the {bearing}"),
        Some(Zone::Suburban) => format!("a house on a street to the {bearing}"),
        Some(Zone::Urban) => format!("a yard {bearing} of the tower"),
        _ => format!("a lot to the {bearing}"),
    }
}

/// The eight-point bearing of `(dx, dy)` from the tower, server units (y
/// points south)
fn compass(dx: f64, dy: f64) -> &'static str {
    use capitol_protocol::fmath::StableMath;
    const POINTS: [&str; 8] = ["east", "north-east", "north", "north-west", "west", "south-west", "south", "south-east"];
    let turn = (-dy).stable_atan2(dx) / std::f64::consts::TAU;
    let k = (turn * 8.0).round() as i64;
    POINTS[k.rem_euclid(8) as usize]
}

fn claimed_share(s: &GazetteShares) -> String {
    match (s.claimed, s.claimed_by_people) {
        (0, _) => String::new(),
        (all, people) if people == all => " People signed for every one.".to_string(),
        (_, 0) => " Citizens signed for every one.".to_string(),
        (all, people) => format!(" People signed for {}, citizens for {}.", count_word(people, false), count_word(all - people, false)),
    }
}

/// The city's work, and its busiest good, people's against citizens':
/// "The most made was Plank: 312, 40 of them by people."
fn made_share(s: &GazetteShares, most: Option<&GazetteMade>) -> String {
    if s.made == 0 {
        return String::new();
    }
    let mut text = format!(" In all the city made {} units, {}.", thousands(s.made), by_people(s.made, s.made_by_people));
    if let Some(m) = most.filter(|m| m.units < s.made) {
        text.push_str(&format!(" The most made was {}: {}, {}.", m.item_name, thousands(m.units), by_people(m.units, m.by_people)));
    }
    text
}

/// "all of them by people", "none of them by people", "40 of them by people"
fn by_people(all: i64, people: i64) -> String {
    match people {
        0 => "none of them by people".to_string(),
        p if p == all => "all of them by people".to_string(),
        p => format!("{} of them by people", count_word(p, false)),
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
    if inserted == 1 {
        let text = format!("The Gazette, No. {}: {}", edition.number, edition.headline);
        super::annotations::note(db, AnnotationKind::GazettePublished, &text).await;
    }
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
            citizen: false,
            item_type: item.to_lowercase(),
            item_name: item.into(),
            quantity,
        }
    }

    /// `work`, by a citizen
    fn citizen_work(id: u128, name: Option<&str>, item: &str, quantity: i64) -> Work {
        Work { citizen: true, ..work(id, name, item, quantity) }
    }

    /// `merchant`'s sales of `item` at their shop `shop`: `(sales, units,
    /// Strands)`
    fn stall(merchant: u128, keeper: Option<&str>, shop: &str, item: &str, (sales, units, strands): (i64, i64, i64)) -> StallTrade {
        StallTrade {
            merchant_id: Uuid::from_u128(merchant),
            shop: shop.into(),
            keeper: keeper.map(Into::into),
            citizen: false,
            item_type: item.to_lowercase(),
            item_name: item.into(),
            sales,
            units,
            strands,
        }
    }

    /// A day with something in every section, and citizens in it: Cy, and
    /// one who keeps out of the papers
    fn busy_day() -> DayRecords {
        DayRecords {
            day: DAY,
            prices: vec![price("Plank", 41, 120, Some(30)), price("Plant Fiber", 1, 900, Some(1)), price("Iron Ingot", 60, 8, None)],
            trades: 37,
            strands: 6300,
            strands_to_people: 5100,
            best_sale: Some(GazetteSale {
                item_type: "iron_axe".into(),
                item_name: "Iron Axe".into(),
                quality: 91,
                grade: "A".into(),
                price: 410,
                quantity: 1,
                seller: Some("Ada".into()),
                maker: Some("Cy".into()),
                own_work: false,
                seller_citizen: false,
                maker_citizen: true,
            }),
            deeds: vec![("rural".into(), 3, 0), ("suburban".into(), 1, 1)],
            claimed_by_people: 3,
            halls: Vec::new(),
            work: vec![
                work(1, Some("Ada"), "Plant Fiber", 300),
                work(1, Some("Ada"), "Rope", 40),
                work(2, None, "Logs", 500),
                citizen_work(3, Some("Cy"), "Plank", 20),
                citizen_work(4, None, "Logs", 120),
            ],
            stalls: vec![
                stall(1, Some("Ada"), "The Rope Walk", "Rope", (4, 25, 150)),
                stall(1, Some("Ada"), "The Rope Walk", "Plant Fiber", (1, 20, 20)),
                StallTrade { citizen: true, ..stall(3, Some("Cy"), "Cy's Planks", "Plank", (3, 6, 240)) },
                stall(2, None, "Bo's Bits", "Logs", (2, 10, 30)),
            ],
            // The fixture's day predates the road in's deed (#448)
            settlers: Vec::new(),
            board: busy_board(),
        }
    }

    fn asked(item: &str, units: i64, price: i64) -> Asked {
        Asked { item_type: item.to_lowercase(), item_name: item.into(), units, price }
    }

    /// `deliverer` brought `units` of `item` for `poster`, paid `paid`
    fn delivery(deliverer: Option<&str>, poster: Option<&str>, item: &str, units: i64, paid: i64) -> GazetteDelivery {
        GazetteDelivery {
            deliverer: deliverer.map(Into::into),
            poster: poster.map(Into::into),
            item_type: item.to_lowercase(),
            item_name: item.into(),
            units,
            paid,
            ..Default::default()
        }
    }

    /// A day on the board: planks and rope wanted, Ada's delivery for a poster
    /// who keeps out of the papers the biggest, Citizen Cy's for Ada, iron
    /// ingots gone begging, and claims kept and lapsed
    fn busy_board() -> Board {
        Board {
            open: vec![asked("Rope", 12, 6), asked("Plank", 10, 15), asked("Plank", 5, 20)],
            expired: vec![asked("Iron Ingot", 4, 60)],
            deliveries: vec![
                delivery(Some("Ada"), None, "Rope", 8, 48),
                GazetteDelivery { deliverer_citizen: true, ..delivery(Some("Cy"), Some("Ada"), "Plank", 3, 45) },
            ],
            posted: 3,
            filled: 1,
            kept: 2,
            lapsed: 1,
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
            ("stalls", &g.stalls_story),
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

    /// The finest sale thanks its maker as well as its seller (#231), and
    /// whichever template the day picks has no holes
    #[test]
    fn the_finest_sale_credits_its_maker() {
        let sale = |seller: Option<&str>, maker: Option<&str>, own_work| GazetteSale {
            item_type: "rope".into(),
            item_name: "Rope".into(),
            quality: 93,
            grade: "A".into(),
            price: 30,
            quantity: 4,
            seller: seller.map(Into::into),
            maker: maker.map(Into::into),
            own_work,
            seller_citizen: false,
            maker_citizen: false,
        };
        let cases = [
            (sale(Some("Ada"), None, true), "made and sold by Ada", " The work was their own."),
            (sale(Some("Bo"), Some("Ada"), false), "made by Ada and sold by Bo", " The work was Ada's."),
            (sale(None, Some("Ada"), false), "made by Ada and sold by a trader who keeps out of the papers", " The work was Ada's."),
            (sale(Some("Bo"), None, false), "sold by Bo", " No better"),
        ];
        for (sale, credited, made) in cases {
            assert_eq!(credit(&sale), credited);
            for day in DAY..DAY + 6 {
                let g = compose(&DayRecords { day, best_sale: Some(sale.clone()), ..Default::default() });
                assert!(g.best_sale_story.ends_with(&format!("{credited}.")), "{}", g.best_sale_story);
                assert!(!g.lead.contains('{'), "{}", g.lead);
                assert!(g.lead.contains(credited) || g.lead.contains(made), "{}", g.lead);
            }
        }
    }

    /// A deed spent on the road in makes the land column the next day
    /// (#448), named only as its holder lets the paper name them
    #[test]
    fn settlers_are_named_with_where_they_settled() {
        let at = |dx: f64, dy: f64| (rules::CENTER_X + dx, rules::CENTER_Y + dy);
        let settler = |name: Option<&str>, citizen, zone: &str, (x, y): (f64, f64)| Settler {
            name: name.map(str::to_owned),
            citizen,
            zone: zone.into(),
            x,
            y,
        };
        let records = DayRecords {
            day: DAY,
            deeds: vec![("rural".into(), 2, 0), ("urban".into(), 1, 0)],
            claimed_by_people: 2,
            settlers: vec![
                settler(Some("Ada"), false, "rural", at(-3000.0, 3000.0)),
                settler(Some("Bo"), true, "urban", at(0.0, -1500.0)),
                settler(None, false, "suburban", at(2500.0, 0.0)),
            ],
            ..Default::default()
        };
        let g = compose(&records);
        assert!(
            g.deeds_story.ends_with(
                " Settled, with the deed the road in earns: Ada, a farm out to the south-west; \
                 Citizen Bo, a yard north of the tower; a trader who keeps out of the papers, a house on a street to the east."
            ),
            "{}",
            g.deeds_story
        );
        let crowd = DayRecords { settlers: vec![records.settlers[0].clone(); 6], ..records.clone() };
        assert!(compose(&crowd).deeds_story.ends_with("; and two more."), "{}", compose(&crowd).deeds_story);
        assert!(!compose(&DayRecords { day: DAY, ..Default::default() }).deeds_story.contains("Settled"));
    }

    /// A guild's hall makes the land column (S4.3), by the guild's name
    #[test]
    fn halls_are_named_by_their_guild() {
        let records = DayRecords {
            day: DAY,
            deeds: vec![("guild_district".into(), 1, 0)],
            claimed_by_people: 1,
            halls: vec!["The Weavers' Guild".into()],
            ..Default::default()
        };
        let g = compose(&records);
        assert!(g.deeds_story.ends_with(" The Weavers' Guild took a hall in the guild district."), "{}", g.deeds_story);
        let three = DayRecords {
            halls: vec!["The Ore Guild".into(), "The Ropewalk".into(), "The Weavers' Guild".into()],
            ..records
        };
        assert!(
            compose(&three)
                .deeds_story
                .ends_with(" The Ore Guild, The Ropewalk and The Weavers' Guild took halls in the guild district."),
            "{}",
            compose(&three).deeds_story
        );
        assert!(!compose(&DayRecords { day: DAY, ..Default::default() }).deeds_story.contains("guild district"));
    }

    /// The hall's board makes a column (S2.4): what's wanted by the Strands
    /// down for it, the biggest delivery, what went begging and the claims
    /// kept, with every template's holes filled
    #[test]
    fn the_board_makes_the_wanted_column() {
        let w = compose(&busy_day()).wanted.expect("a day with commissions");
        let wanted: Vec<(&str, i64, i64, i64, i64)> =
            w.wanted.iter().map(|a| (a.item_name.as_str(), a.commissions, a.units, a.value, a.best_price)).collect();
        assert_eq!(wanted, [("Plank", 2, 15, 250, 20), ("Rope", 1, 12, 72, 6)]);
        assert_eq!(w.came_through, Some(delivery(Some("Ada"), None, "Rope", 8, 48)));
        assert_eq!(w.gone_begging.iter().map(|a| (a.item_name.as_str(), a.units)).collect::<Vec<_>>(), [("Iron Ingot", 4)]);
        assert_eq!((w.posted, w.filled, w.expired, w.deliveries, w.paid, w.kept, w.lapsed), (3, 1, 1, 2, 93, 2, 1));
        for day in DAY..DAY + 8 {
            let w = compose(&DayRecords { day, ..busy_day() }).wanted.unwrap();
            assert!(!w.story.contains('{'), "{}", w.story);
            for told in ["15 Plank", "250 Strands", "Also wanted: 12 Rope.", "Three commissions went up today, and one filled."] {
                assert!(w.story.contains(told), "{told}: {}", w.story);
            }
            assert!(w.story.contains("Ada came through for a resident") || w.story.contains("brought by Ada for a resident"), "{}", w.story);
            assert!(w.story.contains(" In all, two deliveries paid 93 Strands."), "{}", w.story);
            assert!(w.story.contains(" Two claims were kept, and one ran out."), "{}", w.story);
            assert!(w.story.contains("4 Iron Ingot") && w.story.contains("60"), "{}", w.story);
        }
    }

    /// A day nothing stood on the board or was done there has no column, as
    /// editions from before commissions had none
    #[test]
    fn a_day_without_commissions_has_no_wanted_column() {
        assert_eq!(compose(&DayRecords { day: DAY, ..Default::default() }).wanted, None);
        assert_eq!(compose(&DayRecords { board: Board::default(), ..busy_day() }).wanted, None);
        // A board that only emptied today still gets its column
        let emptied = Board { deliveries: vec![delivery(Some("Ada"), Some("Bo"), "Rope", 4, 40)], filled: 1, ..Default::default() };
        let w = compose(&DayRecords { day: DAY, board: emptied, ..Default::default() }).wanted.unwrap();
        assert!(w.wanted.is_empty());
        assert!(w.story.contains("board") && w.story.contains("One commission was filled today."), "{}", w.story);
    }

    /// A poster or deliverer who keeps out of the papers is "a resident", a
    /// citizen still "a citizen"; their names are nowhere in the edition
    #[test]
    fn the_wanted_column_names_only_with_leave() {
        let unnamed = |deliverer_citizen, poster_citizen| Board {
            deliveries: vec![GazetteDelivery { deliverer_citizen, poster_citizen, ..delivery(None, None, "Rope", 4, 40) }],
            ..Default::default()
        };
        for day in DAY..DAY + 8 {
            let story = |board| compose(&DayRecords { day, board, ..Default::default() }).wanted.unwrap().story;
            let s = story(unnamed(false, false));
            assert!(s.contains("A resident came through for a resident") || s.contains("brought by a resident for a resident"), "{s}");
            let s = story(unnamed(true, false));
            assert!(s.contains("A citizen came through for a resident") || s.contains("brought by a citizen for a resident"), "{s}");
            let s = story(Board { deliveries: vec![GazetteDelivery { deliverer_citizen: true, ..delivery(Some("Cy"), Some("Ada"), "Rope", 4, 40) }], ..Default::default() });
            assert!(s.contains("Citizen Cy came through for Ada") || s.contains("brought by Citizen Cy for Ada"), "{s}");
        }
    }

    /// A lapsed claim is counted, never named: the board's records carry no
    /// claimant at all
    #[test]
    fn lapses_are_counted_never_named() {
        let board = Board { open: vec![asked("Rope", 4, 10)], lapsed: 2, ..Default::default() };
        let w = compose(&DayRecords { day: DAY, board, ..Default::default() }).wanted.unwrap();
        assert_eq!((w.kept, w.lapsed), (0, 2));
        assert!(w.story.ends_with(" Two claims ran out undelivered."), "{}", w.story);
        assert_eq!(w.came_through, None);
    }

    #[test]
    fn a_quiet_day_still_fills_every_section() {
        let g = compose(&DayRecords { day: DAY, ..Default::default() });
        for text in [&g.headline, &g.dek, &g.lead, &g.weather, &g.markets, &g.best_sale_story, &g.deeds_story, &g.producers_story, &g.stalls_story] {
            assert!(!text.is_empty() && !text.contains('{'), "{text}");
        }
        assert!(g.headline.contains(rules::weekday(DAY)), "{}", g.headline);
        assert_eq!(g.stalls, None);
    }

    /// The bazaar's sales make the paper (#240): counts and the busiest
    /// shops, ranked by what they took, but never a price (D6)
    #[test]
    fn the_stalls_are_counted_but_their_prices_kept() {
        let g = compose(&busy_day());
        let st = g.stalls.as_ref().expect("the stalls sold");
        assert_eq!((st.sales, st.units, st.strands, st.goods), (10, 61, 440, 4));
        // Cy took the most Strands, Ada sold the most units
        let shops: Vec<(Option<&str>, Option<&str>, bool, i64, &str)> = st
            .shops
            .iter()
            .map(|s| (s.name.as_deref(), s.keeper.as_deref(), s.citizen, s.units, s.item_name.as_str()))
            .collect();
        assert_eq!(
            shops,
            [
                (Some("Cy's Planks"), Some("Cy"), true, 6, "Plank"),
                (Some("The Rope Walk"), Some("Ada"), false, 45, "Rope"),
                (None, None, false, 10, "Logs"),
            ]
        );
        assert!(
            g.stalls_story.starts_with(
                "Ten sales at the bazaar's stalls today, 61 units and 440 Strands in all, across four goods. \
                 The busiest was “Cy's Planks”, kept by Citizen Cy, chiefly Plank. The next two made seven sales between them."
            ),
            "{}",
            g.stalls_story
        );
        // A keeper who keeps out of the papers keeps their shop's name out too
        let json = serde_json::to_string(&g).unwrap();
        assert!(!json.contains("Bo's Bits"), "{json}");
        // No shop's price or takings
        let shops = serde_json::to_string(&st.shops).unwrap();
        assert!(!shops.contains("price") && !shops.contains("strands"), "{shops}");
    }

    /// On a day the stalls are the biggest story, they lead, and the day's
    /// selling counts them
    #[test]
    fn a_day_at_the_stalls_can_lead() {
        for day in DAY..DAY + 6 {
            let records = DayRecords {
                day,
                stalls: vec![stall(1, Some("Ada"), "The Rope Walk", "Rope", (3, 12, 90))],
                ..Default::default()
            };
            let g = compose(&records);
            assert!(!g.headline.contains('{') && !g.dek.contains('{') && !g.lead.contains('{'), "{g:?}");
            assert!(g.lead.contains("bazaar") && g.lead.contains("“The Rope Walk”, kept by Ada"), "{}", g.lead);
            assert!(g.markets.contains("No trades at the Exchange") || g.markets.contains("Exchange floor was empty"));
        }
        // Citizens at the stalls bring the day's shares, with the stalls in
        // them, and the line follows the stalls' story
        let mut records = busy_day();
        records.strands = 0;
        records.strands_to_people = 0;
        let g = compose(&records);
        let s = g.shares.unwrap();
        assert_eq!((s.sold, s.sold_by_people), (440, 200));
        assert!(g.stalls_story.ends_with(" Of the 440 Strands, 200 went to people and 240 to citizens."), "{}", g.stalls_story);
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
        // No deeds: the stalls
        day.deeds.clear();
        day.best_sale = None;
        assert!(compose(&day).lead.contains("bazaar"), "{}", compose(&day).lead);
        // Nothing sold at the stalls: the work
        day.stalls.clear();
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
        assert_eq!(p.iter().map(|p| p.units).collect::<Vec<_>>(), [500, 340, 120, 20]);
        assert_eq!(p[0].name, None);
        assert_eq!((p[1].name.as_deref(), p[1].item_name.as_str(), p[1].item_units), (Some("Ada"), "Plant Fiber", 300));
        let g = compose(&busy_day());
        assert!(g.producers_story.starts_with("A trader who keeps out of the papers"), "{}", g.producers_story);
    }

    /// Citizens are named like anyone else and always marked (C2.2): "Citizen
    /// Cy" in the prose, a flag in the lists, and the day's figures split
    /// between people and citizens
    #[test]
    fn citizens_are_named_marked_and_counted_apart() {
        let g = compose(&busy_day());
        let marks: Vec<(Option<&str>, bool)> = g.producers.iter().map(|p| (p.name.as_deref(), p.citizen)).collect();
        assert_eq!(marks, [(None, false), (Some("Ada"), false), (None, true), (Some("Cy"), true)]);
        assert!(g.best_sale_story.ends_with("made by Citizen Cy and sold by Ada."), "{}", g.best_sale_story);
        assert_eq!(
            g.shares,
            Some(GazetteShares {
                made: 980,
                made_by_people: 840,
                sold: 6740,
                sold_by_people: 5300,
                claimed: 4,
                claimed_by_people: 3,
            })
        );
        let logs = &g.made[0];
        assert_eq!((logs.item_name.as_str(), logs.units, logs.by_people), ("Logs", 620, 500));
        assert_eq!(g.made.iter().map(|m| m.units).sum::<i64>(), 980);
        // The Exchange's and the stalls' Strands together, after the stalls
        assert!(
            g.stalls_story.ends_with(" Of the 6,740 Strands spent at the Exchange and the stalls, 5,300 went to people and 1,440 to citizens."),
            "{}",
            g.stalls_story
        );
        assert!(!g.markets.contains("people"), "{}", g.markets);
        assert!(g.deeds_story.ends_with(" People signed for three, citizens for one."), "{}", g.deeds_story);
        assert!(
            g.producers_story.ends_with(" In all the city made 980 units, 840 of them by people. The most made was Logs: 620, 500 of them by people."),
            "{}",
            g.producers_story
        );

        // A citizen who leads the day's work leads it as one, named or not
        let day = |w: Work| compose(&DayRecords { day: DAY, work: vec![w], ..Default::default() });
        let g = day(citizen_work(3, Some("Cy"), "Plank", 20));
        assert!(g.headline.contains("Citizen Cy"), "{}", g.headline);
        assert!(g.producers_story.starts_with("Citizen Cy made the most"), "{}", g.producers_story);
        assert!(g.producers_story.ends_with(" In all the city made 20 units, none of them by people."), "{}", g.producers_story);
        let g = day(citizen_work(4, None, "Logs", 20));
        assert_eq!(g.headline, "An Unnamed Citizen Leads the Day's Work");
        assert!(g.producers_story.starts_with("A citizen who keeps out of the papers"), "{}", g.producers_story);
    }

    /// With no citizen at work, sale or deed, the edition reads as it did
    /// before there were citizens: no shares, and no line about them
    #[test]
    fn a_day_without_citizens_says_nothing_of_them() {
        let mut records = busy_day();
        records.work.retain(|w| !w.citizen);
        records.stalls.retain(|t| !t.citizen);
        records.strands_to_people = records.strands;
        records.claimed_by_people = 4;
        records.best_sale.as_mut().unwrap().maker_citizen = false;
        let g = compose(&records);
        assert_eq!(g.shares, None);
        assert!(g.producers.iter().all(|p| !p.citizen));
        assert!(g.made.iter().all(|m| m.by_people == m.units));
        for text in [&g.headline, &g.lead, &g.markets, &g.best_sale_story, &g.deeds_story, &g.producers_story, &g.stalls_story] {
            assert!(!text.contains("itizen") && !text.contains("people"), "{text}");
        }
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
            // Bo sold rope Ada made (#231)
            sqlx::query("UPDATE market_trades SET maker_id = $1 WHERE quality = 92").bind(ada.id).execute(&db).await.unwrap();

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
            assert_eq!((sale.maker.as_deref(), sale.own_work), (Some("Ada"), false));
            assert!(g.best_sale_story.contains("made by Ada and sold by a trader"), "{}", g.best_sale_story);
            assert_eq!(g.producers[0].name.as_deref(), Some("Ada"));
            assert_eq!((g.producers[0].units, g.producers[1].name.as_deref()), (320, None));
            if plots.len() == 2 {
                assert_eq!(g.deeds.iter().map(|d| d.claimed + d.foreclosed).sum::<i64>(), 2);
            }
            for text in [&g.headline, &g.dek, &g.lead, &g.weather, &g.markets, &g.best_sale_story, &g.deeds_story, &g.producers_story] {
                assert!(!text.is_empty() && !text.contains('{'), "{text}");
            }

            assert_eq!(publish(&db, day).await.unwrap(), None);
            // One marker for the console (D0.4), and none for the edition it already had
            let marks = crate::engine::annotations::since(&db, 0, 10).await.unwrap();
            assert_eq!(marks.len(), 1);
            assert_eq!(marks[0].kind, AnnotationKind::GazettePublished);
            assert!(marks[0].text.ends_with(&g.headline), "{}", marks[0].text);
            assert_eq!(edition(&db, None).await.unwrap(), Some(g.clone()));
            assert_eq!(edition(&db, Some(day)).await.unwrap(), Some(g));
            assert_eq!(edition(&db, Some(day - 5)).await.unwrap(), None);
        }

        /// A day with a person and a citizen in it (C2.2): the citizen is
        /// named and marked, and the figures split between them
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn an_edition_counts_citizens_apart_from_people(db: PgPool) {
            let ada = player(&db, "Ada", 0).await;
            let cog = crate::citizens::mint(&db, "Cog").await.unwrap().citizen.player_id;

            let day = rules::day_number(Utc::now().timestamp_millis()) - 1;
            let at = |hour: i64| DateTime::<Utc>::from_timestamp_millis(rules::day_start_ms(day) + hour * 300_000).unwrap();
            trade(&db, (ada.id, cog), "fiber", 50, 2, 100, at(3)).await;
            trade(&db, (cog, ada.id), "rope", 95, 10, 3, at(5)).await;
            // Cog made the rope it sold
            sqlx::query("UPDATE market_trades SET maker_id = $1 WHERE quality = 95").bind(cog).execute(&db).await.unwrap();

            let plots: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, zone_id FROM plots LIMIT 2").fetch_all(&db).await.unwrap();
            if plots.len() == 2 {
                let mut conn = db.acquire().await.unwrap();
                record_deed(&mut conn, plots[0].0, &plots[0].1, DeedEvent::Claimed, ada.id).await.unwrap();
                record_deed(&mut conn, plots[1].0, &plots[1].1, DeedEvent::Claimed, cog).await.unwrap();
                sqlx::query("UPDATE deed_events SET at = $1").bind(at(7)).execute(&mut *conn).await.unwrap();
            }
            for (player, item, n) in [(ada.id, "fiber", 300), (cog, "fiber", 100), (cog, "rope", 3)] {
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
            let sale = g.best_sale.as_ref().unwrap();
            assert_eq!((sale.seller.as_deref(), sale.seller_citizen, sale.own_work), (Some("Cog"), true, true));
            assert!(g.best_sale_story.ends_with("made and sold by Citizen Cog."), "{}", g.best_sale_story);
            let marks: Vec<(Option<&str>, bool, i64)> = g.producers.iter().map(|p| (p.name.as_deref(), p.citizen, p.units)).collect();
            assert_eq!(marks, [(Some("Ada"), false, 300), (Some("Cog"), true, 103)]);
            let fiber = &g.made[0];
            assert_eq!((fiber.item_type.as_str(), fiber.units, fiber.by_people), ("fiber", 400, 300));
            let shares = g.shares.expect("citizens were at work");
            assert_eq!((shares.made, shares.made_by_people, shares.sold, shares.sold_by_people), (403, 300, 230, 200));
            if plots.len() == 2 {
                assert_eq!((shares.claimed, shares.claimed_by_people), (2, 1));
                assert!(g.deeds_story.ends_with(" People signed for one, citizens for one."), "{}", g.deeds_story);
            }
            assert!(g.markets.ends_with(" Of the 230 Strands, 200 went to people and 30 to citizens."), "{}", g.markets);
            assert!(g.producers_story.contains("In all the city made 403 units, 300 of them by people."), "{}", g.producers_story);
            // It's kept as served
            assert_eq!(edition(&db, Some(day)).await.unwrap(), Some(g));
        }

        /// Real purchases at two stalls (#240): each is recorded, and the
        /// edition counts them and names the shop whose keeper allows it
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn the_stalls_sales_make_the_paper(db: PgPool) {
            use crate::engine::bazaar;
            use crate::engine::market::tests::Player;
            use crate::engine::units::test_units::stack_of;
            use capitol_protocol::rest::{BuyListing, StockListing};
            use capitol_protocol::rules::stacking::Units;

            let ada = player(&db, "Ada", 1000).await;
            let bo = player(&db, "Bo", 1000).await;
            sqlx::query("UPDATE players SET gazette_named = FALSE WHERE id = $1").bind(bo.id).execute(&db).await.unwrap();
            let day = rules::day_number(Utc::now().timestamp_millis()) - 1;
            let now = DateTime::<Utc>::from_timestamp_millis(rules::day_start_ms(day) + 5 * 300_000).unwrap();

            /// `who` rents `stall` as `shop` and lists ten `item` at `price`
            async fn open(db: &PgPool, who: &Player, (stall, shop): (&str, &str), item: &str, price: i64, now: DateTime<Utc>) -> Uuid {
                let stack = stack_of(db, who.id, who.inventory, 0, item, &Units::uniform(10, 80)).await;
                bazaar::rent(db, who.id, stall, 1, Some(shop), now, &capitol_protocol::tunables::Fees::default()).await.unwrap();
                let req = StockListing { item_id: stack, quantity: 10, price, qualities: None };
                bazaar::stock(db, who.id, &req, now).await.unwrap().id
            }
            let ropes = open(&db, &ada, ("bazaar_stall_3", "The Rope Walk"), "rope", 7, now).await;
            let fiber = open(&db, &bo, ("bazaar_stall_5", "Bo's Bits"), "fiber", 2, now).await;
            let buy = |quantity| BuyListing { quantity, qualities: None };
            bazaar::buy(&db, bo.id, ropes, "bazaar_stall_3", &buy(3), now).await.unwrap();
            bazaar::buy(&db, bo.id, ropes, "bazaar_stall_3", &buy(1), now).await.unwrap();
            bazaar::buy(&db, ada.id, fiber, "bazaar_stall_5", &buy(5), now).await.unwrap();
            let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bazaar_sales").fetch_one(&db).await.unwrap();
            assert_eq!(recorded, 3);

            let g = publish(&db, day).await.unwrap().expect("a new edition");
            let st = g.stalls.as_ref().expect("the stalls sold");
            assert_eq!((st.sales, st.units, st.strands, st.goods), (3, 9, 38, 2));
            let shops: Vec<(Option<&str>, Option<&str>, i64, i64, &str)> =
                st.shops.iter().map(|s| (s.name.as_deref(), s.keeper.as_deref(), s.sales, s.units, s.item_type.as_str())).collect();
            assert_eq!(shops, [(Some("The Rope Walk"), Some("Ada"), 2, 4, "rope"), (None, None, 1, 5, "fiber")]);
            assert!(g.stalls_story.contains("“The Rope Walk”, kept by Ada"), "{}", g.stalls_story);
            assert!(!serde_json::to_string(&g).unwrap().contains("Bo's Bits"));
            // The bazaar was the day's only story
            assert!(g.lead.contains("bazaar"), "{}", g.lead);
            assert_eq!(edition(&db, Some(day)).await.unwrap(), Some(g));
        }

        /// A day on the hall's board through the court itself (S2.4): a
        /// commission filled with its claim kept, one half delivered by
        /// someone who keeps out of the papers, a claim released too late,
        /// and one expired. The edition counts each, names only with leave,
        /// and never names the claimant who lapsed.
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn the_boards_day_makes_the_wanted_column(db: PgPool) {
            use crate::engine::commissions;
            use crate::engine::units::test_units::stack_of;
            use capitol_protocol::rest::{ClaimCommission, DeliverCommission, PostCommission};
            use capitol_protocol::rules::stacking::Units;
            use chrono::Duration;

            let ada = player(&db, "Ada", 10_000).await;
            let bo = player(&db, "Bo", 0).await;
            let cy = player(&db, "Cy", 0).await;
            let zed = player(&db, "Zed", 0).await;
            sqlx::query("UPDATE players SET gazette_named = FALSE WHERE id = $1").bind(cy.id).execute(&db).await.unwrap();
            let day = rules::day_number(Utc::now().timestamp_millis()) - 1;
            let at = |hour: i64| DateTime::<Utc>::from_timestamp_millis(rules::day_start_ms(day) + hour * 300_000).unwrap();
            let ask = |item: &str, quantity, price, due_days| PostCommission {
                item_type: item.into(),
                min_quality: 0,
                quantity,
                price,
                due_days,
            };
            let hand = |item_id, quantity| DeliverCommission { item_id, qualities: None, quantity: Some(quantity) };
            let bo_rope = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::uniform(10, 70)).await;
            let cy_rope = stack_of(&db, cy.id, cy.inventory, 0, "rope", &Units::uniform(10, 70)).await;

            // Filled, its claim kept
            let filled = commissions::post(&db, ada.id, &ask("rope", 4, 10, 2), at(1)).await.unwrap();
            commissions::claim(&db, bo.id, filled.id, &ClaimCommission { quantity: 4 }, at(1)).await.unwrap();
            commissions::deliver(&db, bo.id, filled.id, &hand(bo_rope, 4), at(2)).await.unwrap();
            // Half delivered by Cy, and one more the next day: 4 wanted at the end
            let half = commissions::post(&db, ada.id, &ask("rope", 6, 20, 3), at(3)).await.unwrap();
            commissions::deliver(&db, cy.id, half.id, &hand(cy_rope, 2), at(4)).await.unwrap();
            commissions::deliver(&db, cy.id, half.id, &hand(cy_rope, 1), at(30)).await.unwrap();
            // Zed's claim released in its last hour: a lapse
            let early = at(10) - Duration::minutes(23 * 60 + 30);
            let lapsing = commissions::post(&db, ada.id, &ask("fiber", 5, 3, 3), early).await.unwrap();
            commissions::claim(&db, zed.id, lapsing.id, &ClaimCommission { quantity: 5 }, early).await.unwrap();
            commissions::release(&db, zed.id, lapsing.id, at(10)).await.unwrap();
            // Posted the day before, expired today with nothing brought
            let begging = commissions::post(&db, ada.id, &ask("rope", 7, 9, 1), at(12) - Duration::days(1)).await.unwrap();
            assert_eq!(commissions::expire_due(&db, begging.due_at).await.unwrap().len(), 1);

            let g = publish(&db, day).await.unwrap().expect("a new edition");
            let w = g.wanted.as_ref().expect("the board had a day");
            assert_eq!((w.posted, w.filled, w.expired, w.deliveries, w.paid, w.kept, w.lapsed), (2, 1, 1, 2, 80, 1, 1));
            let wanted: Vec<(&str, i64, i64)> = w.wanted.iter().map(|a| (a.item_type.as_str(), a.units, a.value)).collect();
            assert_eq!(wanted, [("rope", 4, 80), ("fiber", 5, 15)]);
            assert_eq!(w.gone_begging.iter().map(|a| (a.item_type.as_str(), a.units)).collect::<Vec<_>>(), [("rope", 7)]);
            let came = w.came_through.as_ref().unwrap();
            assert_eq!((came.deliverer.as_deref(), came.poster.as_deref(), came.units, came.paid), (Some("Bo"), Some("Ada"), 4, 40));
            assert!(!w.story.contains('{'), "{}", w.story);
            // Cy keeps out of the papers, and Zed's lapse is only counted
            let json = serde_json::to_string(&g).unwrap();
            assert!(!json.contains("Cy") && !json.contains("Zed"), "{json}");
            assert_eq!(edition(&db, Some(day)).await.unwrap(), Some(g));
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
