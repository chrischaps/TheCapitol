//! The Observatory's sampler (D0.2, #355): the world every ten seconds.
//!
//! [`run_sampler_loop`] takes a [`Sample`] every `METRICS_SAMPLE_SECS`
//! ([`SAMPLE_SECS`] unless set; 0 turns it off): a ledger
//! (`ledger::snapshot_within`, which doesn't wait for a quiet moment)
//! against the one before, the window's market fills and
//! bazaar sales, who is in the world and how far the event bus has backed up.
//! The newest day of them sits in [`Samples`], a ring in `AppState`. One a
//! minute is written to `metrics_samples`, covering the whole minute since
//! the one stored before, so the table's samples add up as the ring's do.
//! Those older than [`KEEP_DAYS`] are pruned each hour.
//!
//! It runs beside the tick, as a loop of its own (`load::Loop::Sampler`). It
//! takes the game's lock only to read, twice a sample: the ledger's count of
//! the players, and its own.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use capitol_protocol::ledger::Ledger;
use capitol_protocol::metrics::{
    EventBus, Players, Sample, Trades, KEEP_DAYS, RING_SAMPLES, SAMPLE_SECS, STORE_EVERY,
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::ledger;
use super::load::Loop;
use crate::state::AppState;

/// Samples a prune comes round: an hour's
const PRUNE_EVERY: u64 = 60 * 60 / SAMPLE_SECS;
/// Tries a sample's ledger makes for a quiet moment. A busy town has few,
/// and each try reads every unit again: 50 of them, as `/admin/ledger`
/// makes, doubled the tick's mean in a 224-citizen town. The flows add up
/// either way, and `units_settled` says whether a sample's units and flows
/// agree. A cheap way to settle would let it wait again (#408).
const LEDGER_TRIES: u32 = 1;

/// The newest day of samples, oldest first.
#[derive(Default)]
pub struct Samples {
    ring: Mutex<VecDeque<Sample>>,
}

impl Samples {
    pub fn new() -> Samples {
        Samples::default()
    }

    /// The newest; the oldest goes once there's a day of them.
    pub fn push(&self, sample: Sample) {
        let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        if ring.len() >= RING_SAMPLES {
            ring.pop_front();
        }
        ring.push_back(sample);
    }

    /// The samples taken after `since_ms` (Unix ms), oldest first.
    pub fn since(&self, since_ms: i64) -> Vec<Sample> {
        let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        let first = ring.partition_point(|s| s.at_ms <= since_ms);
        ring.range(first..).cloned().collect()
    }

    pub fn latest(&self) -> Option<Sample> {
        self.ring.lock().unwrap_or_else(|e| e.into_inner()).back().cloned()
    }

    pub fn len(&self) -> usize {
        self.ring.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A sample every `every`, for as long as the server runs.
pub async fn run_sampler_loop(state: AppState, every: Duration) {
    let mut interval = tokio::time::interval(every);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    // The first ledger is only the start of the first window
    let mut last = loop {
        match ledger::snapshot_within(&state, LEDGER_TRIES).await {
            Ok(ledger) => break ledger,
            Err(e) => tracing::warn!("The sampler couldn't read the ledger: {e}"),
        }
        interval.tick().await;
    };
    // Where the stored sample's window starts: a stored sample covers its
    // whole minute, so the table's samples add up as the ring's do
    let mut minute = last.clone();
    let mut taken: u64 = 0;
    loop {
        interval.tick().await;
        let timed = Loop::Sampler.timed();
        let (sample, ledger, admins) = match take(&state, &last).await {
            Ok(taken) => taken,
            Err(e) => {
                // The next window runs from the same start, and is longer
                tracing::warn!("The sampler missed a sample: {e}");
                continue;
            }
        };
        taken += 1;
        if taken.is_multiple_of(STORE_EVERY) {
            let stored = match window(&state.db, &minute, &ledger, &admins).await {
                Ok(stored) => store(&state.db, &Sample { players: sample.players, events: sample.events, ..stored }).await,
                Err(e) => Err(e),
            };
            match stored {
                Ok(()) => minute = ledger.clone(),
                // The next minute's stored sample starts from the same place
                Err(e) => tracing::warn!("Couldn't store a sample: {e}"),
            }
        }
        if taken % PRUNE_EVERY == 1 {
            match prune(&state.db, Utc::now() - chrono::Duration::days(KEEP_DAYS)).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!("Pruned {n} samples past {KEEP_DAYS} days"),
                Err(e) => tracing::warn!("Couldn't prune the samples: {e}"),
            }
        }
        state.samples.push(sample);
        last = ledger;
        drop(timed);
    }
}

/// The sample from `last` to now, the ledger it ends on, and the admins.
async fn take(state: &AppState, last: &Ledger) -> Result<(Sample, Ledger, HashSet<Uuid>), sqlx::Error> {
    let now = ledger::snapshot_within(state, LEDGER_TRIES).await?;
    let admins = admins(&state.db).await?;
    let mut sample = window(&state.db, last, &now, &admins).await?;
    let game = state.game.read().await;
    let citizens = game.players.values().filter(|p| p.citizen).count() as u64;
    sample.players = Players { people: game.players.len() as u64 - citizens, citizens };
    sample.events = EventBus {
        queued: game.event_sender.len() as u64,
        capacity: super::tick::GameState::EVENT_CAPACITY as u64,
    };
    drop(game);
    Ok((sample, now, admins))
}

/// The ledgers' sample from `start` to `end`, with the trades between.
async fn window(db: &PgPool, start: &Ledger, end: &Ledger, admins: &HashSet<Uuid>) -> Result<Sample, sqlx::Error> {
    let mut sample = Sample::between(start, end, admins);
    sample.market = trades(db, Venue::Market, start.at_ms, end.at_ms).await?;
    sample.bazaar = trades(db, Venue::Bazaar, start.at_ms, end.at_ms).await?;
    Ok(sample)
}

/// The players whose accounts are admins'.
async fn admins(db: &PgPool) -> Result<HashSet<Uuid>, sqlx::Error> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT p.id FROM players p JOIN accounts a ON a.id = p.account_id WHERE a.is_admin")
            .fetch_all(db)
            .await?;
    Ok(ids.into_iter().collect())
}

#[derive(Debug, Clone, Copy)]
enum Venue {
    Market,
    Bazaar,
}

/// The trades at `venue` after `from_ms` and up to `to_ms`, by item type.
async fn trades(db: &PgPool, venue: Venue, from_ms: i64, to_ms: i64) -> Result<BTreeMap<String, Trades>, sqlx::Error> {
    let query = match venue {
        Venue::Market => {
            "SELECT item_type, COUNT(*)::BIGINT, SUM(quantity)::BIGINT, SUM(price * quantity)::BIGINT
             FROM market_trades WHERE at > $1 AND at <= $2 GROUP BY item_type"
        }
        Venue::Bazaar => {
            "SELECT item_type, COUNT(*)::BIGINT, SUM(quantity)::BIGINT, SUM(price * quantity)::BIGINT
             FROM bazaar_sales WHERE at > $1 AND at <= $2 GROUP BY item_type"
        }
    };
    let time = |ms: i64| DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or(DateTime::<Utc>::MIN_UTC);
    let rows: Vec<(String, i64, i64, i64)> =
        sqlx::query_as(query).bind(time(from_ms)).bind(time(to_ms)).fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .map(|(item, trades, units, strands)| (item, Trades { trades: trades as u64, units, strands }))
        .collect())
}

/// Writes `sample` to `metrics_samples`, at its own time.
pub async fn store(db: &PgPool, sample: &Sample) -> Result<(), sqlx::Error> {
    let at = DateTime::<Utc>::from_timestamp_millis(sample.at_ms).unwrap_or_else(Utc::now);
    sqlx::query("INSERT INTO metrics_samples (at, sample) VALUES ($1, $2) ON CONFLICT (at) DO NOTHING")
        .bind(at)
        .bind(sqlx::types::Json(sample))
        .execute(db)
        .await?;
    Ok(())
}

/// Deletes the stored samples from before `before`, and says how many.
pub async fn prune(db: &PgPool, before: DateTime<Utc>) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM metrics_samples WHERE at < $1").bind(before).execute(db).await?.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::ledger::Load;

    fn sample(at_ms: i64) -> Sample {
        let ledger = |at_ms| Ledger {
            at_ms,
            booted_ms: 1,
            accounts: vec![],
            escrow_strands: 0,
            units: BTreeMap::new(),
            unit_flows: BTreeMap::new(),
            units_settled: true,
            load: Load::default(),
        };
        Sample::between(&ledger(at_ms - 10_000), &ledger(at_ms), &HashSet::new())
    }

    #[test]
    fn the_ring_keeps_a_day_and_reads_from_a_time() {
        let samples = Samples::new();
        assert!(samples.latest().is_none());
        for i in 1..=RING_SAMPLES as i64 + 5 {
            samples.push(sample(i * 10_000));
        }
        assert_eq!(samples.len(), RING_SAMPLES);
        assert_eq!(samples.latest().unwrap().at_ms, (RING_SAMPLES as i64 + 5) * 10_000);
        let since = samples.since((RING_SAMPLES as i64 + 2) * 10_000);
        assert_eq!(since.iter().map(|s| s.at_ms / 10_000).collect::<Vec<_>>(), vec![8643, 8644, 8645]);
        // From before the oldest: the whole ring, the first five gone
        assert_eq!(samples.since(0)[0].at_ms, 6 * 10_000);
    }

    mod db {
        use super::*;
        use crate::engine::plot_sync::tests::player;
        use capitol_protocol::ledger::reconcile;
        use capitol_protocol::metrics::tally;
        use capitol_protocol::rest::{GrantItem, GrantRequest};

        // `/admin/series` (D0.3) will read the table like this
        /// The stored samples after `since_ms` (Unix ms), oldest first.
        async fn stored_since(db: &PgPool, since_ms: i64) -> Result<Vec<Sample>, sqlx::Error> {
            let since = DateTime::<Utc>::from_timestamp_millis(since_ms).unwrap_or(DateTime::<Utc>::MIN_UTC);
            let rows: Vec<sqlx::types::Json<Sample>> =
                sqlx::query_scalar("SELECT sample FROM metrics_samples WHERE at > $1 ORDER BY at")
                    .bind(since)
                    .fetch_all(db)
                    .await?;
            Ok(rows.into_iter().map(|s| s.0).collect())
        }

        /// A run of samples over grants, a deposit and a fill adds up to what
        /// the ledgers at its ends reconcile to, and the fill shows in its
        /// window's market trades
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn samples_add_up_to_the_ledger(db: PgPool) {
            let ada = player(&db, "ada", 100).await;
            sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
                .bind(Uuid::new_v4())
                .bind(ada)
                .execute(&db)
                .await
                .unwrap();
            let admins = admins(&db).await.unwrap();
            let grant = |strands, quantity| GrantRequest {
                strands,
                items: (quantity > 0)
                    .then(|| GrantItem { item_type: "fiber".into(), quantity, quality: 50 })
                    .into_iter()
                    .collect(),
            };

            let mut ledgers = vec![ledger::books(&db).await.unwrap()];
            crate::engine::grants::grant(&db, ada, &grant(300, 5)).await.unwrap();
            ledgers.push(ledger::books(&db).await.unwrap());
            crate::engine::grants::grant(&db, ada, &grant(0, 7)).await.unwrap();
            ledgers.push(ledger::books(&db).await.unwrap());
            crate::engine::grants::grant(&db, ada, &grant(50, 0)).await.unwrap();
            ledgers.push(ledger::books(&db).await.unwrap());

            let samples: Vec<Sample> = ledgers.windows(2).map(|w| Sample::between(&w[0], &w[1], &admins)).collect();
            let books = reconcile(&ledgers[0], ledgers.last().unwrap());
            assert!(books.holds(), "{books:#?}");
            let tally = tally(&samples);
            assert_eq!(tally.strand_flows, books.strands.flows);
            assert_eq!(tally.unit_flows, books.units.flows);
            assert_eq!(tally.strand_flows["admin_adjustment"], 350);
            assert_eq!(tally.unit_flows["granted"], 12);
            assert_eq!(samples[1].unit_flows["fiber"]["granted"], 7);
            assert_eq!(samples[2].strands.people, ledgers[3].accounts.iter().map(|a| a.strands).sum::<i64>());
        }

        /// A window's trades are the ones after its start and up to its end
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_windows_trades_are_its_own(db: PgPool) {
            let ada = player(&db, "ada", 100).await;
            let sale = |at: &'static str, quantity: i32, price: i64| {
                let db = db.clone();
                async move {
                    sqlx::query(
                        "INSERT INTO bazaar_sales (id, listing_id, stall_id, shop_name, merchant_id, buyer_id,
                                                   item_type, quality, price, quantity, at)
                         VALUES ($1, $1, 'stall_0', 'Ada''s', $2, $2, 'fiber', 50, $3, $4, $5::TIMESTAMPTZ)",
                    )
                    .bind(Uuid::new_v4())
                    .bind(ada)
                    .bind(price)
                    .bind(quantity)
                    .bind(at)
                    .execute(&db)
                    .await
                    .unwrap();
                }
            };
            sale("2026-10-21T12:00:00Z", 9, 9).await;
            sale("2026-10-21T12:00:05Z", 2, 4).await;
            sale("2026-10-21T12:00:10Z", 3, 5).await;
            sale("2026-10-21T12:00:11Z", 9, 9).await;
            let ms = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis();
            let window = trades(&db, Venue::Bazaar, ms("2026-10-21T12:00:00Z"), ms("2026-10-21T12:00:10Z"))
                .await
                .unwrap();
            let fiber = window["fiber"];
            assert_eq!(fiber, Trades { trades: 2, units: 5, strands: 23 });
            assert_eq!(fiber.vwap(), Some(4.6));
            assert!(trades(&db, Venue::Market, 0, ms("2026-10-21T12:00:10Z")).await.unwrap().is_empty());
        }

        /// A stored sample reads back as it was, and a prune takes only the old
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn samples_are_stored_and_pruned(db: PgPool) {
            let now = Utc::now().timestamp_millis();
            let old = sample(now - (KEEP_DAYS + 1) * 24 * 3600 * 1000);
            let new = sample(now);
            store(&db, &old).await.unwrap();
            store(&db, &new).await.unwrap();
            store(&db, &new).await.unwrap();
            assert_eq!(stored_since(&db, 0).await.unwrap(), vec![old.clone(), new.clone()]);
            let pruned = prune(&db, Utc::now() - chrono::Duration::days(KEEP_DAYS)).await.unwrap();
            assert_eq!(pruned, 1);
            assert_eq!(stored_since(&db, 0).await.unwrap(), vec![new]);
        }
    }
}
