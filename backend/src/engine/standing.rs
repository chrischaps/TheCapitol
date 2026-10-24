//! Standing (commons plan D8, S3.1): the record behind what the city says
//! of someone.
//!
//! The record is read where the court and the tax run already keep it:
//! deliveries from `commission_deliveries`, claims kept and lapsed from
//! `commission_claims`, wages paid and earned from `wage_shifts` (a shift's
//! `made` at its offer's `rate` is what was paid), foreclosures from
//! `deed_events`. Only the weeks of tax in good standing are kept here, by
//! the tax run ([`tax_hour`]): `tax_good_runs` counts the hours in a row with
//! something taxed and nothing owed, and every
//! `rules::standing::TAX_WEEK_HOURS` of them is a `tax_good_weeks` row.
//!
//! The score and the words are `rules::standing`'s, pure, so a citizen is
//! scored as a person is. Others see the words ([`words_of`]); you see your
//! number, its trend and your history ([`own`]).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use capitol_protocol::rest::OwnStanding;
use capitol_protocol::rules::standing::{self, Deed, Record};

/// One row of [`RECORDS`]: whose, the kind, when, units, Strands, goods, who with.
type Row = (Uuid, String, DateTime<Utc>, i32, i64, Option<String>, Option<String>);

/// Every record of the players in `$1`, in no order.
const RECORDS: &str = "
    SELECT d.deliverer_id, 'delivered', d.at, d.quantity, d.paid, c.item_type::TEXT, p.name::TEXT
    FROM commission_deliveries d
    JOIN commissions c ON c.id = d.commission_id JOIN players p ON p.id = c.poster_id
    WHERE d.deliverer_id = ANY($1)
  UNION ALL
    SELECT k.claimant_id, k.status::TEXT, k.ended_at, k.quantity, 0::BIGINT, c.item_type::TEXT, p.name::TEXT
    FROM commission_claims k
    JOIN commissions c ON c.id = k.commission_id JOIN players p ON p.id = c.poster_id
    WHERE k.claimant_id = ANY($1) AND k.status IN ('kept', 'lapsed')
  UNION ALL
    SELECT s.worker_id, 'earned_wages', COALESCE(s.ended_at, s.started_at), s.made, s.made::BIGINT * o.rate,
           COALESCE(r.outputs->0->>'item_type', rt.yields_item::TEXT), e.name::TEXT
    FROM wage_shifts s JOIN wage_offers o ON o.id = s.offer_id JOIN players e ON e.id = o.employer_id
    LEFT JOIN recipes r ON r.id = o.recipe_id LEFT JOIN resource_types rt ON rt.id = o.resource_type
    WHERE s.worker_id = ANY($1) AND s.made > 0
  UNION ALL
    SELECT o.employer_id, 'paid_wages', COALESCE(s.ended_at, s.started_at), s.made, s.made::BIGINT * o.rate,
           COALESCE(r.outputs->0->>'item_type', rt.yields_item::TEXT), w.name::TEXT
    FROM wage_shifts s JOIN wage_offers o ON o.id = s.offer_id JOIN players w ON w.id = s.worker_id
    LEFT JOIN recipes r ON r.id = o.recipe_id LEFT JOIN resource_types rt ON rt.id = o.resource_type
    WHERE o.employer_id = ANY($1) AND s.made > 0
  UNION ALL
    SELECT player_id, 'tax_week', at, 0, 0::BIGINT, NULL::TEXT, NULL::TEXT
    FROM tax_good_weeks WHERE player_id = ANY($1)
  UNION ALL
    SELECT player_id, 'foreclosed', at, 0, 0::BIGINT, NULL::TEXT, NULL::TEXT
    FROM deed_events WHERE kind = 'foreclosed' AND player_id = ANY($1)";

fn deed(kind: &str, units: i32, value: i64) -> Option<Deed> {
    Some(match kind {
        "delivered" => Deed::Delivered { units, value },
        "kept" => Deed::Kept { units },
        "lapsed" => Deed::Lapsed { units },
        "earned_wages" => Deed::EarnedWages { units, value },
        "paid_wages" => Deed::PaidWages { units, value },
        "tax_week" => Deed::TaxWeek,
        "foreclosed" => Deed::Foreclosed,
        _ => return None,
    })
}

/// The records of each of `players`, newest first. Someone with none isn't
/// in the map.
pub async fn records_of(conn: &mut PgConnection, players: &[Uuid]) -> Result<HashMap<Uuid, Vec<Record>>, sqlx::Error> {
    let mut by: HashMap<Uuid, Vec<Record>> = HashMap::new();
    if players.is_empty() {
        return Ok(by);
    }
    let rows: Vec<Row> = sqlx::query_as(RECORDS).bind(players).fetch_all(conn).await?;
    for (player, kind, at, units, value, item_type, other) in rows {
        if let Some(deed) = deed(&kind, units, value) {
            by.entry(player).or_default().push(Record { at, deed, item_type, other });
        }
    }
    for records in by.values_mut() {
        records.sort_by_key(|r| std::cmp::Reverse(r.at));
    }
    Ok(by)
}

/// What's said of each of `players` at `now`: every one of them is in the map.
pub async fn words_of(
    conn: &mut PgConnection,
    players: &[Uuid],
    now: DateTime<Utc>,
) -> Result<HashMap<Uuid, String>, sqlx::Error> {
    let mut wanted = players.to_vec();
    wanted.sort();
    wanted.dedup();
    let records = records_of(conn, &wanted).await?;
    Ok(wanted
        .into_iter()
        .map(|p| (p, records.get(&p).map_or_else(|| standing::NEW_IN_TOWN.to_string(), |r| standing::words(r, now))))
        .collect())
}

/// What's said of `player` at `now`.
pub async fn words_for(conn: &mut PgConnection, player: Uuid, now: DateTime<Utc>) -> Result<String, sqlx::Error> {
    Ok(words_of(conn, &[player], now).await?.remove(&player).unwrap_or_else(|| standing::NEW_IN_TOWN.into()))
}

/// [`words_of`] from the pool.
pub async fn words_in(db: &PgPool, players: &[Uuid], now: DateTime<Utc>) -> Result<HashMap<Uuid, String>, sqlx::Error> {
    let mut conn = db.acquire().await?;
    words_of(&mut conn, players, now).await
}

/// `player`'s own standing at `now`: the words, the number and its trend,
/// and the last `rules::standing::HISTORY` records.
pub async fn own(conn: &mut PgConnection, player: Uuid, now: DateTime<Utc>) -> Result<OwnStanding, sqlx::Error> {
    let records = records_of(conn, &[player]).await?.remove(&player).unwrap_or_default();
    Ok(OwnStanding {
        words: standing::words(&records, now),
        score: standing::score(&records, now),
        trend: standing::trend(&records, now),
        records: records.len() as i64,
        history: records.into_iter().take(standing::HISTORY).collect(),
    })
}

/// An hour of the tax run: each of `good` held land, was taxed and owes
/// nothing, and each of `owing` owes on some plot of theirs. A good hour
/// adds to the run, an owing one ends it, and each run of
/// `rules::standing::TAX_WEEK_HOURS` is a week in good standing. Returns
/// who made a week this hour.
pub async fn tax_hour(db: &PgPool, good: &[Uuid], owing: &[Uuid]) -> Result<Vec<Uuid>, sqlx::Error> {
    let mut tx = db.begin().await?;
    if !owing.is_empty() {
        sqlx::query("UPDATE tax_good_runs SET hours = 0 WHERE player_id = ANY($1)")
            .bind(owing)
            .execute(&mut *tx)
            .await?;
    }
    let good: Vec<Uuid> = good.iter().copied().filter(|p| !owing.contains(p)).collect();
    let mut weeks = Vec::new();
    if !good.is_empty() {
        let runs: Vec<(Uuid, i32)> = sqlx::query_as(
            "INSERT INTO tax_good_runs (player_id, hours) SELECT unnest($1::UUID[]), 1
             ON CONFLICT (player_id) DO UPDATE SET hours = tax_good_runs.hours + 1
             RETURNING player_id, hours",
        )
        .bind(&good)
        .fetch_all(&mut *tx)
        .await?;
        weeks = runs.into_iter().filter(|(_, h)| *h >= standing::TAX_WEEK_HOURS).map(|(p, _)| p).collect();
        if !weeks.is_empty() {
            sqlx::query("UPDATE tax_good_runs SET hours = 0 WHERE player_id = ANY($1)")
                .bind(&weeks)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO tax_good_weeks (id, player_id) SELECT gen_random_uuid(), unnest($1::UUID[])")
                .bind(&weeks)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(weeks)
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test standing -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::commissions::{claim, claims_due, deliver, post};
    use crate::engine::market::tests::player;
    use crate::engine::plot_sync::tests::plot;
    use crate::engine::units::test_units::stack_of;
    use capitol_protocol::rest::{ClaimCommission, DeliverCommission, PostCommission};
    use capitol_protocol::rules::stacking::Units;
    use chrono::Duration;

    fn ask(quantity: i32, price: i64) -> PostCommission {
        PostCommission { item_type: "rope".into(), min_quality: 50, quantity, price, due_days: 4 }
    }

    /// A shift `worker` worked on a gathering offer of `employer`'s: `made`
    /// units at `rate`, ended at `at`.
    async fn shift(db: &PgPool, employer: Uuid, worker: Uuid, made: i32, rate: i64, at: DateTime<Utc>) {
        let offer = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO wage_offers (id, employer_id, work_kind, resource_type, rate, cap, made, escrow_strands, fee, ends_at)
             VALUES ($1, $2, 'gather', 'grass', $3, 10, $4, $3 * (10 - $4), 1, $5)",
        )
        .bind(offer)
        .bind(employer)
        .bind(rate)
        .bind(made)
        .bind(at + Duration::days(1))
        .execute(db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO wage_shifts (id, offer_id, worker_id, started_at, ended_at, made, end_reason)
             VALUES ($1, $2, $3, $4, $4, $5, 'worker')",
        )
        .bind(Uuid::new_v4())
        .bind(offer)
        .bind(worker)
        .bind(at)
        .bind(made)
        .execute(db)
        .await
        .unwrap();
    }

    fn kinds(records: &[Record]) -> Vec<&'static str> {
        let mut kinds: Vec<&str> = records
            .iter()
            .map(|r| match r.deed {
                Deed::Delivered { .. } => "delivered",
                Deed::Kept { .. } => "kept",
                Deed::Lapsed { .. } => "lapsed",
                Deed::PaidWages { .. } => "paid_wages",
                Deed::EarnedWages { .. } => "earned_wages",
                Deed::TaxWeek => "tax_week",
                Deed::Foreclosed => "foreclosed",
            })
            .collect();
        kinds.sort();
        kinds
    }

    async fn run_of(db: &PgPool, player: Uuid) -> i32 {
        sqlx::query_scalar("SELECT hours FROM tax_good_runs WHERE player_id = $1")
            .bind(player)
            .fetch_optional(db)
            .await
            .unwrap()
            .unwrap_or(0)
    }

    async fn weeks_of(db: &PgPool, player: Uuid) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM tax_good_weeks WHERE player_id = $1")
            .bind(player)
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_record_is_read_from_the_court_the_wages_and_the_tax_run(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "Ada", 1000).await;
        let bo = player(&db, "Bo", 0).await;

        // Bo claims 4 of Ada's rope and brings them: a delivery and a claim kept
        let first = post(&db, ada.id, &ask(10, 5), now).await.unwrap();
        assert_eq!(first.poster_standing, standing::NEW_IN_TOWN, "a post says what's said of its poster");
        claim(&db, bo.id, first.id, &ClaimCommission { quantity: 4 }, now).await.unwrap();
        let rope = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::from_pairs([(60, 10)])).await;
        let hand = DeliverCommission { item_id: rope, qualities: None, quantity: Some(4) };
        deliver(&db, bo.id, first.id, &hand, now).await.unwrap();
        // and claims 2 of another, which run out
        let second = post(&db, ada.id, &ask(3, 5), now).await.unwrap();
        claim(&db, bo.id, second.id, &ClaimCommission { quantity: 2 }, now).await.unwrap();
        claims_due(&db, now + Duration::hours(25)).await.unwrap();
        // works a shift of Ada's, has a week of tax and loses a plot
        shift(&db, ada.id, bo.id, 3, 2, now).await;
        sqlx::query("INSERT INTO tax_good_weeks (id, player_id) VALUES ($1, $2)")
            .bind(Uuid::new_v4())
            .bind(bo.id)
            .execute(&db)
            .await
            .unwrap();
        let lot = plot(&db, None, 250).await;
        sqlx::query("INSERT INTO deed_events (plot_id, zone_id, kind, player_id) VALUES ($1, 'urban', 'foreclosed', $2)")
            .bind(lot)
            .bind(bo.id)
            .execute(&db)
            .await
            .unwrap();

        let mut conn = db.acquire().await.unwrap();
        let records = records_of(&mut conn, &[ada.id, bo.id]).await.unwrap();
        let bos = &records[&bo.id];
        assert_eq!(kinds(bos), ["delivered", "earned_wages", "foreclosed", "kept", "lapsed", "tax_week"]);
        assert!(bos.windows(2).all(|w| w[0].at >= w[1].at), "newest first");
        let delivered = bos.iter().find(|r| matches!(r.deed, Deed::Delivered { .. })).unwrap();
        assert_eq!(delivered.deed, Deed::Delivered { units: 4, value: 20 });
        assert_eq!((delivered.item_type.as_deref(), delivered.other.as_deref()), (Some("rope"), Some("Ada")));
        assert!(bos.iter().any(|r| r.deed == Deed::Kept { units: 4 }));
        assert!(bos.iter().any(|r| r.deed == Deed::Lapsed { units: 2 }));
        assert!(bos.iter().any(|r| r.deed == Deed::EarnedWages { units: 3, value: 6 } && r.other.as_deref() == Some("Ada")));
        // Ada paid the wages, and posting is no record of its own
        let adas = &records[&ada.id];
        assert_eq!(adas.len(), 1);
        assert_eq!((adas[0].deed, adas[0].other.as_deref()), (Deed::PaidWages { units: 3, value: 6 }, Some("Bo")));

        // In words: Bo has six records, a fresh foreclosure and a lapse
        let later = now + Duration::hours(26);
        let words = words_of(&mut conn, &[bo.id, ada.id, bo.id], later).await.unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[&bo.id], format!("{}, but {}", standing::FINDING_THEIR_FEET, standing::LET_DOWN_LATELY));
        assert_eq!(words[&ada.id], standing::NEW_IN_TOWN);
        assert_eq!(words_for(&mut conn, Uuid::new_v4(), now).await.unwrap(), standing::NEW_IN_TOWN);

        // Bo's own: the number under the words, and the history
        let mine = own(&mut conn, bo.id, later).await.unwrap();
        assert_eq!((mine.records, mine.history.len()), (6, 6));
        assert_eq!(mine.words, words[&bo.id]);
        // 2 ln 21 + 1 + ln 7 + 1, less the lapse an hour old and the foreclosure 13 game days old
        assert!((mine.score - 4.42).abs() < 0.01, "{}", mine.score);
        assert!(mine.trend < 0.0, "a lapse this week: {}", mine.trend);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_week_of_tax_hours_with_nothing_owed_is_a_week_in_good_standing(db: PgPool) {
        let ada = player(&db, "Ada", 0).await.id;
        let bo = player(&db, "Bo", 0).await.id;
        assert!(tax_hour(&db, &[ada, bo], &[]).await.unwrap().is_empty());
        sqlx::query("UPDATE tax_good_runs SET hours = $1")
            .bind(standing::TAX_WEEK_HOURS - 2)
            .execute(&db)
            .await
            .unwrap();
        // Bo owes for an hour: his run starts again
        assert!(tax_hour(&db, &[ada], &[bo]).await.unwrap().is_empty());
        assert_eq!(run_of(&db, bo).await, 0);
        assert_eq!(tax_hour(&db, &[ada, bo], &[]).await.unwrap(), vec![ada]);
        assert_eq!((weeks_of(&db, ada).await, weeks_of(&db, bo).await), (1, 0));
        // and Ada's next week starts from nothing
        assert_eq!((run_of(&db, ada).await, run_of(&db, bo).await), (0, 1));
        // Owing on one plot and not another is owing
        assert!(tax_hour(&db, &[bo], &[bo]).await.unwrap().is_empty());
        assert_eq!(run_of(&db, bo).await, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_tax_run_counts_an_hour_for_an_owner_who_owes_nothing(db: PgPool) {
        let ada = player(&db, "Ada", 1000).await.id;
        let bo = player(&db, "Bo", 0).await.id;
        plot(&db, Some(ada), 10_000).await;
        let owed = plot(&db, Some(bo), 10_000).await;
        sqlx::query("UPDATE plots SET tax_owed = 5 WHERE id = $1").bind(owed).execute(&db).await.unwrap();
        crate::engine::property_tax::process_property_taxes(&db, 0.02).await;
        assert_eq!((run_of(&db, ada).await, run_of(&db, bo).await), (1, 0));
    }
}
