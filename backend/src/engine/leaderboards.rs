//! The weekly leaderboards (commons plan D9, S3.2): the GDD's scoreboard,
//! from records the city already keeps.
//!
//! A Capitol week is seven game days, Towerday to Wildday
//! (`rules::week_of`). At the next week's first midnight, as the Gazette
//! publishes the edition about the week's last day, [`compute`] works out
//! the week's four boards and keeps them in `leaderboards`. A server down at
//! that midnight works out the week just ended when it starts, so the
//! wealth is read late but the week isn't lost; only one further back is.
//! The boards:
//!
//! - **wealth:** Strands (with what's in escrow behind the player's open
//!   market orders), deeds at assessed value, and goods at each item's last
//!   trade price before the week ended. A good that's never traded adds
//!   nothing. It's read as the boards are worked out, so it's the wealth at
//!   the week's end, give or take the moments since midnight.
//! - **maker:** what each player gathered and crafted in the week
//!   (`production_tally`), each unit at its item's target price (D5's value
//!   chain, `target_price`), or its last trade where it has none.
//! - **trader:** the Strands of each market fill, bazaar sale and
//!   commission delivery in the week, counted for both sides.
//! - **kept their word:** the value of the claims each player kept in the
//!   week, the units claimed at the commission's price.
//!
//! Each board is kept whole-city (zone `''`) and again for each home zone,
//! the zone of a player's oldest plot. Everyone with anything to count has a
//! row, not only the top 20, so anyone can be told where they stand
//! ([`read`]). Ties share a rank ("1, 2, 2, 4"). Citizens are on the boards
//! like anyone else and marked; `people_rank` ranks people alone, for the
//! filter that leaves citizens out. Someone who keeps their name out of the
//! papers (`players.gazette_named`) is listed as "a resident".

use std::collections::{BTreeMap, BTreeSet};

use capitol_protocol::rest::{GazetteBoard, Leaderboard, LeaderboardEntry, LeaderboardKind, LeaderboardQuery, LeaderboardsResponse};
use capitol_protocol::rules::{self, Zone};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// How many lines a board shows
pub const TOP: usize = 20;
/// How many of each board the Gazette names
pub const NAMED: usize = 3;
/// The zone key a whole-city board is kept under
const CITY: &str = "";

/// One player's value on a board, before it's ranked
#[derive(Debug, Clone, PartialEq)]
pub struct Tally {
    pub player_id: Uuid,
    pub citizen: bool,
    /// The zone of their oldest plot, if they have one
    pub home: Option<String>,
    pub value: i64,
}

/// One line of a board as it's kept
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    /// `''` for the whole city
    pub zone: String,
    pub rank: i32,
    /// The rank among people alone; `None` for a citizen
    pub people_rank: Option<i32>,
    pub player_id: Uuid,
    pub value: i64,
}

/// Rank `tallies` whole-city and in each home zone. Only a value over 0 is
/// on a board. The most first; ties share a rank and are listed by id.
pub fn rank(tallies: &[Tally]) -> Vec<Ranked> {
    let mut on: Vec<&Tally> = tallies.iter().filter(|t| t.value > 0).collect();
    on.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.player_id.cmp(&b.player_id)));
    let zones: BTreeSet<&str> = on.iter().filter_map(|t| t.home.as_deref()).collect();
    let mut out = Vec::new();
    for zone in std::iter::once(CITY).chain(zones) {
        let scope: Vec<&&Tally> = on.iter().filter(|t| zone == CITY || t.home.as_deref() == Some(zone)).collect();
        // (place, rank, the last value) for everyone, and for people alone
        let mut all = (0, 0, i64::MIN);
        let mut people = (0, 0, i64::MIN);
        for t in scope {
            let step = |(place, rank, last): &mut (i32, i32, i64)| {
                *place += 1;
                if t.value != *last {
                    *rank = *place;
                    *last = t.value;
                }
                *rank
            };
            let rank = step(&mut all);
            let people_rank = (!t.citizen).then(|| step(&mut people));
            out.push(Ranked { zone: zone.to_string(), rank, people_rank, player_id: t.player_id, value: t.value });
        }
    }
    out
}

/// Week `week`'s every board, from the records, before they're ranked
pub async fn gather(db: &PgPool, week: i64) -> Result<BTreeMap<LeaderboardKind, Vec<Tally>>, sqlx::Error> {
    let first = rules::week_first_day(week);
    let at = |day: i64| DateTime::<Utc>::from_timestamp_millis(rules::day_start_ms(day)).unwrap_or_default();
    let (start, end) = (at(first), at(first + 7));

    let players: Vec<(Uuid, bool, Option<String>)> = sqlx::query_as(
        "SELECT p.id, COALESCE(a.kind = 'citizen', FALSE),
                (SELECT pl.zone_id FROM plots pl WHERE pl.owner_id = p.id
                 ORDER BY pl.claimed_at NULLS LAST, pl.id LIMIT 1)
         FROM players p LEFT JOIN accounts a ON a.id = p.account_id",
    )
    .fetch_all(db)
    .await?;

    // Each item's last trade before the week ended
    const LAST: &str = "last AS (SELECT DISTINCT ON (item_type) item_type, price FROM market_trades
                                 WHERE at < $1 ORDER BY item_type, at DESC, id)";
    let wealth: Vec<(Uuid, i64)> = sqlx::query_as(&format!(
        "WITH {LAST},
         goods AS (SELECT i.owner_id AS player_id, SUM(i.quantity::BIGINT * l.price) AS v
                   FROM items i JOIN last l ON l.item_type = i.item_type
                   WHERE i.owner_id IS NOT NULL GROUP BY 1),
         land AS (SELECT owner_id AS player_id, SUM(assessed_value)::BIGINT AS v
                  FROM plots WHERE owner_id IS NOT NULL GROUP BY 1),
         bids AS (SELECT player_id, SUM(escrow_strands)::BIGINT AS v
                  FROM market_orders WHERE status = 'open' GROUP BY 1)
         SELECT p.id, (p.strand_balance + COALESCE(g.v, 0) + COALESCE(l.v, 0) + COALESCE(b.v, 0))::BIGINT
         FROM players p
         LEFT JOIN goods g ON g.player_id = p.id
         LEFT JOIN land l ON l.player_id = p.id
         LEFT JOIN bids b ON b.player_id = p.id"
    ))
    .bind(end)
    .fetch_all(db)
    .await?;

    let maker: Vec<(Uuid, i64)> = sqlx::query_as(&format!(
        "WITH {LAST}
         SELECT pt.player_id, SUM(pt.quantity * COALESCE(it.target_price, l.price, 0))::BIGINT
         FROM production_tally pt
         LEFT JOIN item_types it ON it.id = pt.item_type
         LEFT JOIN last l ON l.item_type = pt.item_type
         WHERE pt.day >= $2 AND pt.day < $3
         GROUP BY 1"
    ))
    .bind(end)
    .bind(first)
    .bind(first + 7)
    .fetch_all(db)
    .await?;

    let trader: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT who, SUM(v)::BIGINT FROM (
             SELECT o.player_id AS who, t.price * t.quantity AS v
             FROM market_trades t JOIN market_orders o ON o.id = t.sell_order_id
             WHERE t.at >= $1 AND t.at < $2
             UNION ALL
             SELECT o.player_id, t.price * t.quantity
             FROM market_trades t JOIN market_orders o ON o.id = t.buy_order_id
             WHERE t.at >= $1 AND t.at < $2
             UNION ALL
             SELECT merchant_id, price * quantity FROM bazaar_sales WHERE at >= $1 AND at < $2
             UNION ALL
             SELECT buyer_id, price * quantity FROM bazaar_sales WHERE at >= $1 AND at < $2
             UNION ALL
             SELECT deliverer_id, paid FROM commission_deliveries WHERE at >= $1 AND at < $2
             UNION ALL
             SELECT c.poster_id, d.paid
             FROM commission_deliveries d JOIN commissions c ON c.id = d.commission_id
             WHERE d.at >= $1 AND d.at < $2
         ) x GROUP BY who",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;

    let kept: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT cl.claimant_id, SUM(cl.quantity::BIGINT * c.price_per_unit)::BIGINT
         FROM commission_claims cl JOIN commissions c ON c.id = cl.commission_id
         WHERE cl.status = 'kept' AND cl.ended_at >= $1 AND cl.ended_at < $2
         GROUP BY 1",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;

    let who: BTreeMap<Uuid, (bool, Option<String>)> =
        players.into_iter().map(|(id, citizen, home)| (id, (citizen, home))).collect();
    let tallies = |values: Vec<(Uuid, i64)>| -> Vec<Tally> {
        values
            .into_iter()
            .filter_map(|(player_id, value)| {
                let (citizen, home) = who.get(&player_id)?.clone();
                Some(Tally { player_id, citizen, home, value })
            })
            .collect()
    };
    Ok(BTreeMap::from([
        (LeaderboardKind::Wealth, tallies(wealth)),
        (LeaderboardKind::Maker, tallies(maker)),
        (LeaderboardKind::Trader, tallies(trader)),
        (LeaderboardKind::KeptWord, tallies(kept)),
    ]))
}

/// Work out and keep week `week`'s boards. `true` if they're new; `false`
/// if the week already had them (another start, or a race), which are left
/// as they were.
pub async fn compute(db: &PgPool, week: i64) -> Result<bool, sqlx::Error> {
    let done: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM leaderboard_weeks WHERE week = $1)")
        .bind(week)
        .fetch_one(db)
        .await?;
    if done {
        return Ok(false);
    }
    let boards = gather(db, week).await?;
    let mut tx = db.begin().await?;
    let new = sqlx::query("INSERT INTO leaderboard_weeks (week) VALUES ($1) ON CONFLICT (week) DO NOTHING")
        .bind(week)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    if !new {
        tx.rollback().await?;
        return Ok(false);
    }
    for (board, tallies) in &boards {
        keep(&mut tx, week, *board, &rank(tallies)).await?;
    }
    tx.commit().await?;
    Ok(true)
}

async fn keep(tx: &mut Transaction<'_, Postgres>, week: i64, board: LeaderboardKind, rows: &[Ranked]) -> Result<(), sqlx::Error> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO leaderboards (week, board, zone, rank, people_rank, player_id, value)
         SELECT $1, $2, * FROM UNNEST($3::TEXT[], $4::INT[], $5::INT[], $6::UUID[], $7::BIGINT[])",
    )
    .bind(week)
    .bind(board.key())
    .bind(rows.iter().map(|r| r.zone.clone()).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.rank).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.people_rank).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.player_id).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.value).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The latest week with boards
pub async fn latest(db: &PgPool) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT MAX(week) FROM leaderboard_weeks").fetch_one(db).await
}

/// `GET /leaderboards`: the boards `query` asks for, as `me` sees them
pub async fn read(db: &PgPool, query: &LeaderboardQuery, me: Option<Uuid>) -> Result<LeaderboardsResponse, sqlx::Error> {
    let latest = latest(db).await?;
    let zone = query.zone.clone().filter(|z| !z.is_empty());
    let mut response = LeaderboardsResponse { week: None, latest, zone: zone.clone(), people: query.people, ..Default::default() };
    let Some(week) = query.week.or(latest) else {
        return Ok(response);
    };
    response.week = Some(week);
    let zones: Vec<String> = sqlx::query_scalar("SELECT DISTINCT zone FROM leaderboards WHERE week = $1 AND zone <> ''")
        .bind(week)
        .fetch_all(db)
        .await?;
    response.zones = by_ring(zones);
    let boards: Vec<LeaderboardKind> = match query.board.as_deref() {
        Some(key) => LeaderboardKind::from_key(key).into_iter().collect(),
        None => LeaderboardKind::ALL.to_vec(),
    };
    for board in boards {
        let (top, mine, entries) = lines(db, week, board, zone.as_deref().unwrap_or(CITY), query.people, me, TOP).await?;
        response.boards.push(Leaderboard { board, top, mine, entries });
    }
    Ok(response)
}

/// Week `week`'s top `n` whole-city on each board, citizens among them,
/// for the Gazette. Boards nobody is on are left out.
pub async fn names(db: &PgPool, week: i64) -> Result<Vec<GazetteBoard>, sqlx::Error> {
    let mut out = Vec::new();
    for board in LeaderboardKind::ALL {
        let (names, ..) = lines(db, week, board, CITY, false, None, NAMED).await?;
        if !names.is_empty() {
            out.push(GazetteBoard { board, names });
        }
    }
    Ok(out)
}

/// One board's top `n`, `me`'s own line, and how many are on it
async fn lines(
    db: &PgPool,
    week: i64,
    board: LeaderboardKind,
    zone: &str,
    people: bool,
    me: Option<Uuid>,
    n: usize,
) -> Result<(Vec<LeaderboardEntry>, Option<LeaderboardEntry>, i64), sqlx::Error> {
    type Row = (Uuid, i32, i64, String, bool, bool, i64);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT * FROM (
             SELECT l.player_id, CASE WHEN $4 THEN l.people_rank ELSE l.rank END AS r, l.value,
                    p.name, p.gazette_named, COALESCE(a.kind = 'citizen', FALSE), COUNT(*) OVER ()
             FROM leaderboards l
             JOIN players p ON p.id = l.player_id
             LEFT JOIN accounts a ON a.id = p.account_id
             WHERE l.week = $1 AND l.board = $2 AND l.zone = $3 AND (NOT $4 OR l.people_rank IS NOT NULL)
         ) b
         WHERE r <= $5 OR player_id = $6
         ORDER BY r, player_id",
    )
    .bind(week)
    .bind(board.key())
    .bind(zone)
    .bind(people)
    .bind(n as i32)
    .bind(me)
    .fetch_all(db)
    .await?;
    let entries = rows.first().map_or(0, |r| r.6);
    let mut top = Vec::new();
    let mut mine = None;
    for (player_id, rank, value, name, named, citizen, _) in rows {
        let entry = LeaderboardEntry {
            rank: rank as i64,
            name: named.then_some(name),
            player_id: named.then_some(player_id),
            citizen,
            value,
            me: Some(player_id) == me,
        };
        if entry.me {
            mine = Some(entry.clone());
        }
        if top.len() < n && rank as usize <= n {
            top.push(entry);
        }
    }
    Ok((top, mine, entries))
}

/// Zone ids from the centre out; ones the rules don't know last, by id
fn by_ring(mut zones: Vec<String>) -> Vec<String> {
    let order = |zone: &str| Zone::from_str(zone).map_or(usize::MAX, |z| Zone::ALL.iter().position(|a| *a == z).unwrap_or(0));
    zones.sort_by(|a, b| order(a).cmp(&order(b)).then_with(|| a.cmp(b)));
    zones
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(id: u128, citizen: bool, home: Option<&str>, value: i64) -> Tally {
        Tally { player_id: Uuid::from_u128(id), citizen, home: home.map(Into::into), value }
    }

    fn of(rows: &[Ranked], zone: &str) -> Vec<(u128, i32, Option<i32>)> {
        rows.iter().filter(|r| r.zone == zone).map(|r| (r.player_id.as_u128(), r.rank, r.people_rank)).collect()
    }

    #[test]
    fn ties_share_a_rank_and_people_rank_without_citizens() {
        let rows = rank(&[
            tally(1, false, Some("urban"), 50),
            tally(2, true, Some("rural"), 90),
            tally(3, false, Some("urban"), 50),
            tally(4, false, None, 10),
            tally(5, true, None, 0),
        ]);
        // Nobody with nothing to count; the citizen first, the tie shared
        assert_eq!(of(&rows, ""), [(2, 1, None), (1, 2, Some(1)), (3, 2, Some(1)), (4, 4, Some(3))]);
        assert_eq!(of(&rows, "urban"), [(1, 1, Some(1)), (3, 1, Some(1))]);
        assert_eq!(of(&rows, "rural"), [(2, 1, None)]);
        // No home, no zone board
        assert_eq!(rows.iter().filter(|r| r.player_id.as_u128() == 4).count(), 1);
    }

    #[test]
    fn zones_run_from_the_centre_out() {
        let zones = by_ring(vec!["rural".into(), "nowhere".into(), "urban".into(), "suburban".into()]);
        assert_eq!(zones, ["urban", "suburban", "rural", "nowhere"]);
    }

    mod db {
        use super::super::*;
        use crate::engine::market::tests::player;

        /// A fill of `quantity` `item` at `price` from `seller` to `buyer`, at `at`
        async fn trade(db: &PgPool, (seller, buyer): (Uuid, Uuid), item: &str, price: i64, quantity: i32, at: DateTime<Utc>) {
            let mut ids = Vec::new();
            for (side, who) in [("sell", seller), ("buy", buyer)] {
                let id = Uuid::new_v4();
                sqlx::query(
                    "INSERT INTO market_orders (id, player_id, side, item_type, min_grade, grade_rule, price,
                         quantity_total, quantity_remaining, status, fee, expires_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, 'filled', 1, NOW() + INTERVAL '7 days')",
                )
                .bind(id)
                .bind(who)
                .bind(side)
                .bind(item)
                .bind((side == "buy").then_some("F"))
                .bind((side == "buy").then_some("average"))
                .bind(price)
                .bind(quantity)
                .execute(db)
                .await
                .unwrap();
                ids.push(id);
            }
            sqlx::query(
                "INSERT INTO market_trades (id, buy_order_id, sell_order_id, item_type, grade, quality, min_quality, price, quantity, at)
                 VALUES ($1, $2, $3, $4, 'C', 50, 50, $5, $6, $7)",
            )
            .bind(Uuid::new_v4())
            .bind(ids[1])
            .bind(ids[0])
            .bind(item)
            .bind(price)
            .bind(quantity)
            .bind(at)
            .execute(db)
            .await
            .unwrap();
        }

        /// `quantity` of `item` held by `owner`
        async fn hold(db: &PgPool, owner: Uuid, item: &str, quantity: i32) {
            sqlx::query("INSERT INTO items (id, item_type, quality, quantity, owner_id) VALUES ($1, $2, 50, $3, $4)")
                .bind(Uuid::new_v4())
                .bind(item)
                .bind(quantity)
                .bind(owner)
                .execute(db)
                .await
                .unwrap();
        }

        /// A commission by `poster` for `units` at `price`, all of it claimed
        /// and kept by `claimant` at `at`
        async fn kept_claim(db: &PgPool, poster: Uuid, claimant: Uuid, units: i32, price: i64, at: DateTime<Utc>) {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO commissions (id, poster_id, item_type, min_quality, quantity, delivered, price_per_unit,
                     escrow_strands, fee, posted_at, due_at, status, closed_at)
                 VALUES ($1, $2, 'rope', 0, $3, $3, $4, 0, 1, $5, $5 + INTERVAL '1 day', 'filled', $5)",
            )
            .bind(id)
            .bind(poster)
            .bind(units)
            .bind(price)
            .bind(at)
            .execute(db)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO commission_claims (id, commission_id, claimant_id, quantity, delivered, claimed_at, until, status, ended_at)
                 VALUES ($1, $2, $3, $4, $4, $5 - INTERVAL '1 hour', $5 + INTERVAL '1 hour', 'kept', $5)",
            )
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(claimant)
            .bind(units)
            .bind(at)
            .execute(db)
            .await
            .unwrap();
        }

        fn value_of(r: &LeaderboardsResponse, board: LeaderboardKind, name: &str) -> Option<i64> {
            let b = r.boards.iter().find(|b| b.board == board)?;
            b.top.iter().find(|e| e.name.as_deref() == Some(name)).map(|e| e.value)
        }

        /// A seeded week: each board's value, goods with no trade worth
        /// nothing, the unnamed as a resident, people-only ranks, your own
        /// line outside the top 20, and a second compute leaves it be
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_seeded_week_makes_every_board(db: PgPool) {
            let week = rules::week_of(rules::day_number(Utc::now().timestamp_millis())) - 1;
            let first = rules::week_first_day(week);
            let at = |day: i64, hour: i64| DateTime::<Utc>::from_timestamp_millis(rules::day_start_ms(first + day) + hour * 300_000).unwrap();
            sqlx::query("UPDATE item_types SET target_price = NULL WHERE id = 'fiber'").execute(&db).await.unwrap();
            sqlx::query("UPDATE item_types SET target_price = 3 WHERE id = 'rope'").execute(&db).await.unwrap();

            let ada = player(&db, "Ada", 1000).await.id;
            let bo = player(&db, "Bo", 100).await.id;
            sqlx::query("UPDATE players SET gazette_named = FALSE WHERE id = $1").bind(bo).execute(&db).await.unwrap();
            let cog = crate::citizens::mint(&db, "Cog").await.unwrap().citizen.player_id;
            sqlx::query("UPDATE players SET strand_balance = 5000 WHERE id = $1").bind(cog).execute(&db).await.unwrap();

            trade(&db, (ada, bo), "fiber", 2, 100, at(-1, 3)).await; // the week before: wealth's price only
            trade(&db, (ada, bo), "fiber", 3, 100, at(2, 3)).await;
            trade(&db, (bo, ada), "fiber", 9, 10, at(7, 3)).await; // the week after: in nothing
            // Fiber last traded at 3 in the week; rope never has
            hold(&db, ada, "fiber", 50).await;
            hold(&db, ada, "rope", 40).await;
            for (who, item, day, n) in [(ada, "rope", first + 1, 10), (ada, "fiber", first + 6, 20), (cog, "rope", first + 7, 99)] {
                sqlx::query("INSERT INTO production_tally (day, player_id, item_type, quantity) VALUES ($1, $2, $3, $4)")
                    .bind(day)
                    .bind(who)
                    .bind(item)
                    .bind(n as i64)
                    .execute(&db)
                    .await
                    .unwrap();
            }
            kept_claim(&db, ada, cog, 4, 25, at(3, 0)).await;
            kept_claim(&db, ada, bo, 2, 30, at(-2, 0)).await; // before the week
            // Twenty-two richer than Dee, so Dee's line is past the top 20
            for i in 0..22 {
                player(&db, &format!("Rich {i}"), 2000 + i).await;
            }
            let dee = player(&db, "Dee", 7).await.id;

            assert!(compute(&db, week).await.unwrap());
            assert!(!compute(&db, week).await.unwrap(), "a week is worked out once");

            let all = LeaderboardQuery::default();
            let r = read(&db, &all, Some(ada)).await.unwrap();
            assert_eq!((r.week, r.latest), (Some(week), Some(week)));
            // Ada: 1000 Strands and 50 fiber at 3; the rope adds nothing
            assert_eq!(value_of(&r, LeaderboardKind::Wealth, "Cog"), Some(5000));
            let wealth = &r.boards[0];
            let mine = wealth.mine.clone().expect("Ada's own line");
            assert_eq!((mine.value, mine.me), (1150, true));
            // Maker: 10 rope at its target of 3, 20 fiber at its last trade of 3
            assert_eq!(value_of(&r, LeaderboardKind::Maker, "Ada"), Some(90));
            // Trader: both sides of the week's fill
            assert_eq!(value_of(&r, LeaderboardKind::Trader, "Ada"), Some(300));
            let trader = &r.boards[2];
            let unnamed = trader.top.iter().find(|e| e.name.is_none()).expect("Bo, unnamed");
            assert_eq!((unnamed.value, unnamed.rank), (300, 1));
            assert_eq!(unnamed.player_id, None, "a resident stays one");
            assert_eq!(trader.top.iter().find(|e| e.me).and_then(|e| e.player_id), Some(ada));
            assert_eq!(trader.entries, 2);
            // Kept their word: Cog's claim in the week; Bo's was before it
            let kept = &r.boards[3];
            assert_eq!(kept.top.len(), 1);
            assert_eq!((kept.top[0].name.as_deref(), kept.top[0].citizen, kept.top[0].value), (Some("Cog"), true, 100));

            // Cog tops wealth; people only, it's gone and the ranks count again
            assert_eq!(wealth.top[0].name.as_deref(), Some("Cog"));
            let people = read(&db, &LeaderboardQuery { people: true, ..all.clone() }, Some(ada)).await.unwrap();
            let pw = &people.boards[0];
            assert!(pw.top.iter().all(|e| !e.citizen));
            assert_eq!(pw.top[0].rank, 1);
            assert!(pw.top[0].value < 5000);
            assert_eq!(pw.entries, wealth.entries - 1);
            assert!(people.boards[3].top.is_empty() && people.boards[3].mine.is_none());

            // Dee, past the top 20, still hears where they stand
            let r = read(&db, &all, Some(dee)).await.unwrap();
            let wealth = &r.boards[0];
            assert_eq!(wealth.top.len(), TOP);
            let mine = wealth.mine.clone().expect("Dee's own line");
            assert!(mine.rank as usize > TOP && !wealth.top.iter().any(|e| e.me), "{mine:?}");
            assert_eq!(mine.value, 7);
            // One board alone
            let one = read(&db, &LeaderboardQuery { board: Some("maker".into()), ..all.clone() }, Some(dee)).await.unwrap();
            assert_eq!(one.boards.len(), 1);
            assert!(one.boards[0].mine.is_none(), "Dee made nothing");
            // A week with no boards
            let none = read(&db, &LeaderboardQuery { week: Some(week - 5), ..all }, Some(dee)).await.unwrap();
            assert!(none.boards.iter().all(|b| b.top.is_empty() && b.entries == 0));
        }

        /// A home zone's board holds only those whose oldest plot is in it
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_home_zone_has_its_own_board(db: PgPool) {
            let plots: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, zone_id FROM plots ORDER BY id LIMIT 2").fetch_all(&db).await.unwrap();
            if plots.len() < 2 {
                return;
            }
            let week = rules::week_of(rules::day_number(Utc::now().timestamp_millis())) - 1;
            let ada = player(&db, "Ada", 10).await.id;
            let bo = player(&db, "Bo", 20).await.id;
            // Ada's oldest plot is the first; her newer one is in the second's zone
            for (plot, who, ago, value) in [(&plots[0], ada, 9, 300), (&plots[1], ada, 1, 100)] {
                sqlx::query("UPDATE plots SET owner_id = $1, claimed_at = NOW() - make_interval(days => $2), assessed_value = $3 WHERE id = $4")
                    .bind(who)
                    .bind(ago)
                    .bind(value)
                    .bind(plot.0)
                    .execute(&db)
                    .await
                    .unwrap();
            }
            assert!(compute(&db, week).await.unwrap());
            let home = plots[0].1.clone();
            let r = read(&db, &LeaderboardQuery { zone: Some(home.clone()), ..Default::default() }, Some(bo)).await.unwrap();
            assert_eq!(r.zones, [home]);
            let wealth = &r.boards[0];
            assert_eq!(wealth.top.len(), 1);
            // Deeds at their assessed value
            assert_eq!((wealth.top[0].name.as_deref(), wealth.top[0].value), (Some("Ada"), 410));
            assert!(wealth.mine.is_none(), "Bo has no home there");
        }
    }
}
