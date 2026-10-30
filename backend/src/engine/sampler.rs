//! The Observatory's sampler (D0.2, #355): the world every ten seconds.
//!
//! [`run_sampler_loop`] takes a [`Sample`] every `METRICS_SAMPLE_SECS`
//! ([`SAMPLE_SECS`] unless set; 0 turns it off): a ledger
//! (`ledger::snapshot`) against the one before, the window's market fills and
//! bazaar sales, who is in the world and how far the event bus has backed up.
//! The newest day of them sits in [`Samples`], a ring in `AppState`. One a
//! minute is written to `metrics_samples`, covering the whole minute since
//! the one stored before, so the table's samples add up as the ring's do.
//! Those older than [`KEEP_DAYS`] are pruned each hour. Since D2.5 each
//! sample is put to the watches (`watches::Watcher`) before it goes out, and
//! says which are firing.
//!
//! Since D4.1 each sample carries a summary of the town's reports over its
//! window (`engine::town`), and the stored minute the minute's. Each sample
//! the sampler also looks whether the town has gone quiet, and marks it down.
//!
//! Since D2.2 each sample says how people's and citizens' Strands are spread
//! (`Distribution`), by the trades the town's newest report names: since
//! Callings K3.3, each citizen's favourite job, or `all-rounder`. Since #559
//! by its calling's style too, and each bucket counts its accounts' escrow,
//! as the ledger has it per account (#553).
//!
//! Since D2.3 each sample carries the market's books as it was taken: the
//! units resting on each side of each item and the best price either side.
//!
//! Since #562 each sample says how the grass stands as it was taken
//! ([`grass`]): fiber's one source, so the faucet's. The gathers the server
//! refused over the window come with the ledger (`Ledger::refused`).
//!
//! Since #572 [`work`] sums the tick's phases and the loops beside it over a
//! range in bins, for `/admin/work`: a day of them is a few kB, where every
//! sample's was about 17 MB.
//!
//! It runs beside the tick, as a loop of its own (`load::Loop::Sampler`). It
//! takes the game's lock only to read, twice a sample: the ledger's count of
//! the players, and its own.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use capitol_protocol::ledger::Ledger;
use capitol_protocol::metrics::{
    Book, Buckets, Channel, Distribution, EventBus, Feed, Flows, Grass, Paid, Players, Sample, Trades, WorkBin,
    WorkSeries, WorkTotal, KEEP_DAYS, RING_SAMPLES, SAMPLE_SECS, SERIES_TIMES, STORE_EVERY,
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::ledger;
use super::load::Loop;
use crate::models::{NodeState, ResourceNodeState};
use crate::state::AppState;

/// Samples a prune comes round: an hour's
const PRUNE_EVERY: u64 = 60 * 60 / SAMPLE_SECS;
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

    /// When the oldest sample was taken; `None` when there are none
    pub fn oldest_ms(&self) -> Option<i64> {
        self.ring.lock().unwrap_or_else(|e| e.into_inner()).front().map(|s| s.at_ms)
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
        match ledger::snapshot(&state).await {
            Ok(ledger) => break ledger,
            Err(e) => tracing::warn!("The sampler couldn't read the ledger: {e}"),
        }
        interval.tick().await;
    };
    // Where the stored sample's window starts: a stored sample covers its
    // whole minute, so the table's samples add up as the ring's do
    let mut minute = last.clone();
    let mut taken: u64 = 0;
    let mut watcher = super::watches::Watcher::new(targets(&state).await);
    loop {
        interval.tick().await;
        let timed = Loop::Sampler.timed();
        if let Some(mark) = state.town.quiet(Utc::now().timestamp_millis()) {
            tracing::info!("{}", mark.text);
            super::annotations::note(&state.db, mark.kind, &mark.text).await;
        }
        let (mut sample, ledger, admins) = match take(&state, &last).await {
            Ok(taken) => taken,
            Err(e) => {
                // The next window runs from the same start, and is longer
                tracing::warn!("The sampler missed a sample: {e}");
                continue;
            }
        };
        taken += 1;
        watcher.check(&state, &last, &ledger, &admins, &mut sample).await;
        if taken.is_multiple_of(STORE_EVERY) {
            let town = state.town.take_minute();
            let stored = match window(&state.db, &minute, &ledger, &admins, &state.town.buckets()).await {
                Ok(stored) => {
                    let whole = Sample {
                        books: sample.books.clone(),
                        players: sample.players,
                        events: sample.events,
                        watches: sample.watches.clone(),
                        drift: sample.drift.clone(),
                        town: town.clone(),
                        distribution: sample.distribution.clone(),
                        grass: sample.grass,
                        ..stored
                    };
                    store(&state.db, &whole).await
                }
                Err(e) => Err(e),
            };
            match stored {
                Ok(()) => minute = ledger.clone(),
                // The next minute's stored sample starts from the same place
                Err(e) => {
                    tracing::warn!("Couldn't store a sample: {e}");
                    if let Some(town) = town {
                        state.town.restore_minute(town);
                    }
                }
            }
        }
        if taken % PRUNE_EVERY == 1 {
            match prune(&state.db, Utc::now() - chrono::Duration::days(KEEP_DAYS)).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!("Pruned {n} samples past {KEEP_DAYS} days"),
                Err(e) => tracing::warn!("Couldn't prune the samples: {e}"),
            }
        }
        super::admin_feed::send(Feed::Sample(Box::new(sample.clone())));
        state.samples.push(sample);
        last = ledger;
        drop(timed);
    }
}

/// Each item's target price (D2.3), Strands a unit, as the game loaded the
/// content.
async fn targets(state: &AppState) -> BTreeMap<String, f64> {
    let game = state.game.read().await;
    game.item_types.values().filter_map(|t| Some((t.id.clone(), t.target_price? as f64))).collect()
}

/// The sample from `last` to now, the ledger it ends on, and the admins.
async fn take(state: &AppState, last: &Ledger) -> Result<(Sample, Ledger, HashSet<Uuid>), sqlx::Error> {
    let now = ledger::snapshot(state).await?;
    let admins = admins(&state.db).await?;
    let buckets = state.town.buckets();
    let mut sample = window(&state.db, last, &now, &admins, &buckets).await?;
    sample.books = books(&state.db).await?;
    let game = state.game.read().await;
    let citizens = game.players.values().filter(|p| p.citizen).count() as u64;
    sample.players = Players { people: game.players.len() as u64 - citizens, citizens };
    sample.events = EventBus {
        queued: game.event_sender.len() as u64,
        capacity: super::tick::GameState::EVENT_CAPACITY as u64,
    };
    sample.grass = Some(grass(game.resource_nodes.values()));
    drop(game);
    sample.distribution = Some(Distribution::of(&now, &admins, |name| buckets.get(name)));
    // Last, once nothing can fail: a missed sample leaves the town's reports
    // for the next
    sample.town = state.town.take_sample();
    Ok((sample, now, admins))
}

/// The ledgers' sample from `start` to `end`, with the trades between and
/// who paid whom (#561), its citizens grouped by `buckets`.
async fn window(
    db: &PgPool,
    start: &Ledger,
    end: &Ledger,
    admins: &HashSet<Uuid>,
    buckets: &HashMap<String, Buckets>,
) -> Result<Sample, sqlx::Error> {
    let mut sample = Sample::between(start, end, admins);
    sample.market = trades(db, Venue::Market, start.at_ms, end.at_ms).await?;
    sample.bazaar = trades(db, Venue::Bazaar, start.at_ms, end.at_ms).await?;
    let paid = paid(db, start.at_ms, end.at_ms).await?;
    sample.flows = Flows::between(start, end, admins, |name| buckets.get(name), &paid);
    Ok(sample)
}

/// What accounts paid one another after `from_ms` and up to `to_ms`, by
/// payer, payee and channel (#561), each from its channel's own rows, which
/// name both sides: the market's fills (the bid's player to the ask's, at
/// the fill's price), the bazaar's sales, the court's payouts (from the
/// commission's poster or the wage offer's employer) and a trade window's
/// Strands (its `trade_sent` row's player to its `trade_received` row's,
/// written together, at one time).
async fn paid(db: &PgPool, from_ms: i64, to_ms: i64) -> Result<Vec<Paid>, sqlx::Error> {
    let time = |ms: i64| DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or(DateTime::<Utc>::MIN_UTC);
    let rows: Vec<(Uuid, Uuid, String, i64)> = sqlx::query_as(
        "SELECT b.player_id, s.player_id, 'goods', SUM(t.price * t.quantity)::BIGINT
         FROM market_trades t
         JOIN market_orders b ON b.id = t.buy_order_id
         JOIN market_orders s ON s.id = t.sell_order_id
         WHERE t.at > $1 AND t.at <= $2
         GROUP BY 1, 2
         UNION ALL
         SELECT buyer_id, merchant_id, 'goods', SUM(price * quantity)::BIGINT
         FROM bazaar_sales WHERE at > $1 AND at <= $2
         GROUP BY 1, 2
         UNION ALL
         SELECT COALESCE(c.poster_id, w.employer_id), p.player_id,
                CASE WHEN c.id IS NULL THEN 'wages' ELSE 'commissions' END, SUM(p.amount)::BIGINT
         FROM currency_transactions p
         LEFT JOIN commissions c ON c.id = p.reference_id
         LEFT JOIN wage_offers w ON w.id = p.reference_id
         WHERE p.transaction_type = 'court_payout' AND p.occurred_at > $1 AND p.occurred_at <= $2
           AND COALESCE(c.poster_id, w.employer_id) IS NOT NULL
         GROUP BY 1, 2, 3
         UNION ALL
         SELECT s.player_id, r.player_id, 'trades', SUM(r.amount)::BIGINT
         FROM currency_transactions r
         JOIN currency_transactions s
           ON s.reference_id = r.reference_id AND s.occurred_at = r.occurred_at AND s.transaction_type = 'trade_sent'
         WHERE r.transaction_type = 'trade_received' AND r.occurred_at > $1 AND r.occurred_at <= $2
         GROUP BY 1, 2",
    )
    .bind(time(from_ms))
    .bind(time(to_ms))
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(from, to, channel, strands)| {
            let channel = match channel.as_str() {
                "goods" => Channel::Goods,
                "wages" => Channel::Wages,
                "commissions" => Channel::Commissions,
                _ => Channel::Trades,
            };
            Paid { from, to, channel, strands }
        })
        .collect())
}

/// How the grass nodes among `nodes` stand (#562).
fn grass<'a>(nodes: impl IntoIterator<Item = &'a ResourceNodeState>) -> Grass {
    let mut grass = Grass::default();
    for node in nodes.into_iter().filter(|n| n.resource_type == GRASS) {
        *match node.state {
            NodeState::Available => &mut grass.ready,
            NodeState::BeingHarvested => &mut grass.gathering,
            NodeState::Depleted => &mut grass.spent,
        } += 1;
    }
    grass
}

/// The resource fiber comes from
const GRASS: &str = "grass";

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

/// The market's open orders now, by item type (D2.3).
async fn books(db: &PgPool) -> Result<BTreeMap<String, Book>, sqlx::Error> {
    let rows: Vec<(String, i64, i64, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT item_type,
                COALESCE(SUM(quantity_remaining) FILTER (WHERE side = 'buy'), 0)::BIGINT,
                COALESCE(SUM(quantity_remaining) FILTER (WHERE side = 'sell'), 0)::BIGINT,
                MAX(price) FILTER (WHERE side = 'buy'),
                MIN(price) FILTER (WHERE side = 'sell')
         FROM market_orders
         WHERE status = 'open' AND quantity_remaining > 0 AND expires_at > NOW()
         GROUP BY item_type",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(item, bids, asks, best_bid, best_ask)| (item, Book { bids, asks, best_bid, best_ask }))
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

/// The samples after `from_ms` and up to `to_ms` (Unix ms), oldest first,
/// for `/admin/series` (D0.3): from the ring where it reaches, and before
/// that from `metrics_samples`, a minute apart. With `fields`, each holds
/// only those and its times (`SERIES_TIMES`). A field is a path into the
/// sample (#411), its keys split on `.`, and `*` stands for every key of an
/// object there: `load.tick_p95_ms`, `load.phases.*.secs`. What a path
/// reaches keeps its place, so a projected sample is a partial `Sample`.
pub async fn series(
    ring: &Samples,
    db: &PgPool,
    from_ms: i64,
    to_ms: i64,
    fields: Option<&[String]>,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let keep: Option<Vec<Vec<String>>> = fields.map(|f| {
        SERIES_TIMES
            .iter()
            .map(|t| t.to_string())
            .chain(f.iter().cloned())
            .map(|f| f.split('.').map(str::to_string).collect())
            .collect()
    });
    // Read the ring first, so the table is read only up to where the ring
    // was then: a day's oldest sample popped in between leaves no gap
    let ring_from = ring.oldest_ms().unwrap_or(i64::MAX);
    let ring: Vec<Sample> = ring.since(from_ms).into_iter().filter(|s| s.at_ms <= to_ms).collect();
    let mut samples = if from_ms < ring_from {
        stored(db, from_ms, to_ms.min(ring_from - 1), keep.as_deref()).await?
    } else {
        Vec::new()
    };
    samples.extend(ring.iter().filter_map(|s| {
        let value = serde_json::to_value(s).ok()?;
        Some(match &keep {
            Some(keep) => project(&value, keep),
            None => value,
        })
    }));
    Ok(samples)
}

/// `sample` with only what the paths in `keep` reach, each where it was.
fn project(sample: &serde_json::Value, keep: &[Vec<String>]) -> serde_json::Value {
    let mut out = serde_json::Value::Object(Default::default());
    for path in keep {
        if let Some(part) = reach(sample, path) {
            merge(&mut out, part);
        }
    }
    out
}

/// What `path` reaches in `value`, inside the objects that lead to it, or
/// nothing when a key on the way isn't there. `*` takes every key of an
/// object, and keeps those the rest of the path reaches.
fn reach(value: &serde_json::Value, path: &[String]) -> Option<serde_json::Value> {
    let Some((key, rest)) = path.split_first() else {
        return Some(value.clone());
    };
    let serde_json::Value::Object(map) = value else {
        return None;
    };
    if key == "*" {
        Some(map.iter().filter_map(|(k, v)| Some((k.clone(), reach(v, rest)?))).collect())
    } else {
        let part = reach(map.get(key)?, rest)?;
        Some(serde_json::Value::Object([(key.clone(), part)].into_iter().collect()))
    }
}

/// Puts `part` into `into`, object by object, so two paths into one object
/// both keep what they reached.
fn merge(into: &mut serde_json::Value, part: serde_json::Value) {
    match (into, part) {
        (serde_json::Value::Object(into), serde_json::Value::Object(part)) => {
            for (k, v) in part {
                match into.get_mut(&k) {
                    Some(had) => merge(had, v),
                    None => {
                        into.insert(k, v);
                    }
                }
            }
        }
        (into, part) => *into = part,
    }
}

/// What [`work`] reads of each sample: its ticks and its work (#572)
const WORK_FIELDS: [&str; 4] = ["load.ticks", "load.ticks_over", "load.phases", "load.loops"];

/// The tick's phases and the loops beside it after `from_ms` and up to
/// `to_ms` (Unix ms), summed in bins `bin_ms` long, for `/admin/work`
/// (#572). It reads what [`series`] does, so a range past the ring reads
/// the stored minutes.
pub async fn work(
    ring: &Samples,
    db: &PgPool,
    from_ms: i64,
    to_ms: i64,
    bin_ms: i64,
) -> Result<WorkSeries, sqlx::Error> {
    let fields: Vec<String> = WORK_FIELDS.iter().map(|f| f.to_string()).collect();
    Ok(bin_work(&series(ring, db, from_ms, to_ms, Some(&fields)).await?, bin_ms))
}

/// `samples`, partial or whole, summed into the bins their `at_ms` fall in.
/// A sample stored before a field was reads as without it.
fn bin_work(samples: &[serde_json::Value], bin_ms: i64) -> WorkSeries {
    #[derive(serde::Deserialize)]
    struct Part {
        at_ms: i64,
        #[serde(default)]
        load: PartLoad,
    }
    #[derive(serde::Deserialize, Default)]
    struct PartLoad {
        #[serde(default)]
        ticks: u64,
        #[serde(default)]
        ticks_over: u64,
        #[serde(default)]
        phases: BTreeMap<String, Run>,
        #[serde(default)]
        loops: BTreeMap<String, Run>,
    }
    #[derive(serde::Deserialize)]
    struct Run {
        #[serde(default)]
        runs: u64,
        #[serde(default)]
        secs: f64,
        #[serde(default)]
        p95_ms: Option<f64>,
        #[serde(default)]
        slow_statements: u64,
    }
    let add = |into: &mut BTreeMap<String, WorkTotal>, runs: BTreeMap<String, Run>| {
        for (name, r) in runs {
            into.entry(name).or_default().add(r.runs, r.secs, r.p95_ms, r.slow_statements);
        }
    };
    let mut bins: BTreeMap<i64, WorkBin> = BTreeMap::new();
    let mut to_ms = None;
    for sample in samples {
        let Ok(part) = <Part as serde::Deserialize>::deserialize(sample) else { continue };
        let at_ms = part.at_ms.div_euclid(bin_ms) * bin_ms;
        let bin = bins.entry(at_ms).or_insert_with(|| WorkBin { at_ms, ..WorkBin::default() });
        bin.ticks += part.load.ticks;
        bin.ticks_over += part.load.ticks_over;
        add(&mut bin.phases, part.load.phases);
        add(&mut bin.loops, part.load.loops);
        to_ms = to_ms.max(Some(part.at_ms));
    }
    WorkSeries { bin_ms, to_ms, bins: bins.into_values().collect() }
}

/// The keys of `path` before its first `*`: as far as the database can
/// reach for it.
fn plain_prefix(path: &[String]) -> String {
    path.iter().take_while(|k| *k != "*").cloned().collect::<Vec<_>>().join(".")
}

/// The stored samples after `from_ms` and up to `to_ms` (Unix ms), oldest
/// first; with `keep`, only what those paths reach in each.
async fn stored(
    db: &PgPool,
    from_ms: i64,
    to_ms: i64,
    keep: Option<&[Vec<String>]>,
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let from = DateTime::<Utc>::from_timestamp_millis(from_ms).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let to = DateTime::<Utc>::from_timestamp_millis(to_ms).unwrap_or_else(Utc::now);
    let rows: Vec<serde_json::Value> = match keep {
        // Projected in the database, so a month of whole samples never
        // crosses the wire for one chart's field. The database reaches each
        // path up to its first `*` (`#>`, one `[value]` or `[]` a path, in
        // order), and the rest is done here, in the same `project` as the ring
        Some(keep) => {
            let mut prefixes: Vec<String> = keep.iter().map(|p| plain_prefix(p)).collect();
            prefixes.sort();
            prefixes.dedup();
            let reached: Vec<serde_json::Value> = sqlx::query_scalar(
                "SELECT COALESCE((
                     SELECT jsonb_agg(CASE WHEN sample #> string_to_array(p, '.') IS NULL THEN '[]'::jsonb
                                           ELSE jsonb_build_array(sample #> string_to_array(p, '.')) END
                                      ORDER BY i)
                     FROM unnest($3::text[]) WITH ORDINALITY AS t(p, i)), '[]'::jsonb)
                 FROM metrics_samples WHERE at > $1 AND at <= $2 ORDER BY at",
            )
            .bind(from)
            .bind(to)
            .bind(&prefixes)
            .fetch_all(db)
            .await?;
            reached
                .into_iter()
                .map(|row| {
                    let mut partial = serde_json::Value::Object(Default::default());
                    let found = row.as_array().map(Vec::as_slice).unwrap_or_default();
                    for (prefix, value) in prefixes.iter().zip(found) {
                        let Some(value) = value.as_array().and_then(|v| v.first()) else { continue };
                        let path: Vec<String> =
                            if prefix.is_empty() { vec![] } else { prefix.split('.').map(str::to_string).collect() };
                        let placed = path.iter().rev().fold(value.clone(), |v, k| {
                            serde_json::Value::Object([(k.clone(), v)].into_iter().collect())
                        });
                        merge(&mut partial, placed);
                    }
                    project(&partial, keep)
                })
                .collect()
        }
        None => {
            sqlx::query_scalar("SELECT sample FROM metrics_samples WHERE at > $1 AND at <= $2 ORDER BY at")
                .bind(from)
                .bind(to)
                .fetch_all(db)
                .await?
        }
    };
    Ok(rows)
}

/// Deletes the stored samples from before `before`, and says how many.
pub async fn prune(db: &PgPool, before: DateTime<Utc>) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM metrics_samples WHERE at < $1").bind(before).execute(db).await?.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::ledger::Load;

    /// The grass is counted by its nodes' states, and nothing else is
    /// (#562)
    #[test]
    fn the_grass_is_counted_by_state() {
        let node = |resource: &str, state| ResourceNodeState {
            id: Uuid::new_v4(),
            resource_type: resource.into(),
            x: 0.0,
            y: 0.0,
            base_quality: 50,
            state,
            harvested_by: None,
            regenerates_at_tick: None,
        };
        let nodes = [
            node("grass", NodeState::Available),
            node("grass", NodeState::Available),
            node("grass", NodeState::BeingHarvested),
            node("grass", NodeState::Depleted),
            node("grass", NodeState::Depleted),
            node("grass", NodeState::Depleted),
            node("tree", NodeState::Depleted),
            node("tree", NodeState::Available),
        ];
        assert_eq!(grass(&nodes), Grass { ready: 2, gathering: 1, spent: 3 });
        assert_eq!(grass(&nodes[6..]), Grass::default());
    }

    fn sample(at_ms: i64) -> Sample {
        let ledger = |at_ms| Ledger {
            at_ms,
            booted_ms: 1,
            accounts: vec![],
            escrow_strands: 0,
            units: BTreeMap::new(),
            unit_flows: BTreeMap::new(),
            units_settled: true,
            crafts: BTreeMap::new(),
            grants: Default::default(),
            refused: Default::default(),
            held_flows: Default::default(),
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

    fn paths(fields: &[&str]) -> Vec<Vec<String>> {
        fields.iter().map(|f| f.split('.').map(str::to_string).collect()).collect()
    }

    /// A sample whose tick ran every phase and loop, as a busy server's does
    fn busy_sample() -> serde_json::Value {
        use super::super::load::{Loop, Phase};
        use capitol_protocol::ledger::{PhaseLoad, TICK_BUCKETS_MS};
        let load = |n: u64| {
            let work = |k: u64| PhaseLoad {
                runs: 100 * n,
                secs: 0.0123 * (n * k) as f64,
                max_ms: 4.5,
                buckets: (0..=TICK_BUCKETS_MS.len() as u64).map(|b| n * k * (b + 1)).collect(),
                slow_statements: n,
            };
            Load {
                tick_period_ms: 100.0,
                ticks: 100 * n,
                tick_secs: 0.4 * n as f64,
                tick_buckets: (0..=TICK_BUCKETS_MS.len() as u64).map(|b| n * (b + 3)).collect(),
                phases: Phase::ALL.iter().zip(1..).map(|(p, k)| (p.as_str().to_string(), work(k))).collect(),
                loops: Loop::ALL.iter().zip(1..).map(|(l, k)| (l.as_str().to_string(), work(k))).collect(),
                sockets_open: 3,
                ..Load::default()
            }
        };
        let ledger = |at_ms, n| Ledger {
            at_ms,
            booted_ms: 1,
            accounts: vec![],
            escrow_strands: 0,
            units: BTreeMap::new(),
            unit_flows: BTreeMap::new(),
            units_settled: true,
            crafts: BTreeMap::new(),
            grants: Default::default(),
            refused: Default::default(),
            held_flows: Default::default(),
            load: load(n),
        };
        serde_json::to_value(Sample::between(&ledger(90_000, 1), &ledger(100_000, 2), &HashSet::new())).unwrap()
    }

    /// A path keeps what it reaches where it was, `*` takes each key of an
    /// object, two paths into one object both keep theirs, and a path that
    /// reaches nothing adds nothing
    #[test]
    fn fields_reach_inside_a_sample() {
        let sample = busy_sample();
        let kept = project(&sample, &paths(&["at_ms", "load.tick_p95_ms", "load.ticks", "load.phases.movement.mean_ms", "nope.at.all", "load.nope"]));
        assert_eq!(
            kept,
            serde_json::json!({
                "at_ms": 100_000,
                "load": {
                    "tick_p95_ms": sample["load"]["tick_p95_ms"],
                    "ticks": 100,
                    "phases": { "movement": { "mean_ms": sample["load"]["phases"]["movement"]["mean_ms"] } },
                },
            })
        );
        let secs = project(&sample, &paths(&["load.phases.*.secs"]));
        let phases = secs["load"]["phases"].as_object().unwrap();
        assert_eq!(phases.len(), 12);
        assert!(phases.values().all(|p| p.as_object().unwrap().keys().eq(["secs"])), "{phases:?}");
        // The whole and a part of it: the whole
        assert_eq!(project(&sample, &paths(&["load.ticks", "load"]))["load"], sample["load"]);
        assert_eq!(project(&sample, &paths(&["load", "load.ticks"]))["load"], sample["load"]);
        // A key under a number reaches nothing
        assert_eq!(project(&sample, &paths(&["at_ms.x"])), serde_json::json!({}));
        // What the paths keep reads as a partial sample
        let tick = project(&sample, &paths(&["at_ms", "booted_ms", "since_ms", "load.tick_mean_ms", "load.tick_p95_ms"]));
        assert_eq!(tick["load"]["tick_mean_ms"], sample["load"]["tick_mean_ms"]);
    }

    /// A day of one phase is a small part of a day of the whole tick (#411):
    /// the console's tick chart asks for four numbers, not every phase
    #[test]
    fn a_day_of_one_phase_is_a_sliver_of_the_tick() {
        let sample = busy_sample();
        let day = |fields: &[&str]| {
            let mut keep = paths(&SERIES_TIMES);
            keep.extend(paths(fields));
            serde_json::to_vec(&project(&sample, &keep)).unwrap().len() * RING_SAMPLES
        };
        let whole = day(&["load"]);
        let tick = day(&["load.tick_mean_ms", "load.tick_p95_ms", "load.tick_period_ms", "load.ticks_over"]);
        let phase = day(&["load.phases.movement"]);
        let secs = day(&["load.phases.*.secs"]);
        println!("a day of: the whole load {whole} B, the tick chart {tick} B, one phase {phase} B, every phase's secs {secs} B");
        assert!(tick * 8 < whole, "{tick} against {whole}");
        assert!(phase * 8 < whole, "{phase} against {whole}");
        assert!(secs * 2 < whole, "{secs} against {whole}");
    }

    /// A day of the tick's work in the console's hour bins is a few kB, where
    /// four numbers of every phase and loop in every sample was about 17 MB
    /// (#572), and each bin is its samples summed
    #[test]
    fn a_day_of_work_in_bins_is_a_few_kb() {
        let busy = busy_sample();
        let day: Vec<serde_json::Value> = (1..=RING_SAMPLES as i64)
            .map(|i| {
                let mut s = busy.clone();
                s["at_ms"] = (i * 10_000).into();
                s
            })
            .collect();
        let mut keep = paths(&SERIES_TIMES);
        keep.extend(paths(&["load.phases.*.runs", "load.phases.*.secs", "load.phases.*.p95_ms", "load.phases.*.slow_statements"]));
        keep.extend(paths(&["load.loops.*.runs", "load.loops.*.secs", "load.loops.*.p95_ms", "load.loops.*.slow_statements"]));
        let before = serde_json::to_vec(&project(&busy, &keep)).unwrap().len() * RING_SAMPLES;

        let work = bin_work(&day, 3_600_000);
        let after = serde_json::to_vec(&work).unwrap().len();
        println!("a day of the phase table: every sample's work {before} B, in hour bins {after} B");
        assert!(after < 64_000, "{after} B");
        assert!(after * 200 < before, "{after} against {before}");

        // The day's 86,400 s start at 10 s, so the first and last hours are part ones
        assert_eq!(work.bins.len(), 25);
        assert_eq!(work.to_ms, Some(RING_SAMPLES as i64 * 10_000));
        assert!(work.bins.iter().all(|b| b.at_ms % 3_600_000 == 0));
        let hour = &work.bins[1];
        assert_eq!(hour.at_ms, 3_600_000);
        assert_eq!(hour.ticks, 360 * busy["load"]["ticks"].as_u64().unwrap());
        assert_eq!(hour.phases.len(), 12);
        assert_eq!(hour.loops.len(), 8);
        let one = &busy["load"]["phases"]["movement"];
        let movement = &hour.phases["movement"];
        assert_eq!(movement.runs, 360 * one["runs"].as_u64().unwrap());
        assert!((movement.secs - 360.0 * one["secs"].as_f64().unwrap()).abs() < 1e-9);
        assert_eq!(movement.p95_ms, one["p95_ms"].as_f64());
        assert_eq!(movement.slow_statements, 360 * one["slow_statements"].as_u64().unwrap());
        // Every sample is counted once
        let runs: u64 = work.bins.iter().map(|b| b.phases["movement"].runs).sum();
        assert_eq!(runs, RING_SAMPLES as u64 * one["runs"].as_u64().unwrap());
    }

    /// A bin's p95 is its worst sample's, a p95 past the last bucket is the
    /// worst of all, and a sample where a phase didn't run says nothing of it
    #[test]
    fn a_bins_p95_is_its_worst_samples() {
        let at = |at_ms: i64, runs: u64, p95: Option<f64>| {
            serde_json::json!({
                "at_ms": at_ms,
                "load": { "ticks": 100, "phases": { "movement": { "runs": runs, "secs": 0.1, "p95_ms": p95 } } },
            })
        };
        let work = bin_work(
            &[
                at(10_000, 0, None),
                at(20_000, 5, Some(2.0)),
                at(30_000, 5, Some(4.0)),
                at(40_000, 0, None),
                at(70_000, 5, Some(1.0)),
                at(80_000, 5, None),
                at(90_000, 5, Some(9.0)),
                // An old sample with no load, and one that isn't a sample
                serde_json::json!({ "at_ms": 100_000 }),
                serde_json::json!({ "strands": 3 }),
            ],
            60_000,
        );
        let p95: Vec<_> = work.bins.iter().map(|b| (b.at_ms, b.phases.get("movement").and_then(|m| m.p95_ms))).collect();
        assert_eq!(p95, [(0, Some(4.0)), (60_000, None)]);
        assert_eq!(work.bins[0].phases["movement"].runs, 10);
        assert_eq!(work.bins[1].ticks, 300);
        assert_eq!(work.to_ms, Some(100_000));
        assert_eq!(bin_work(&[], 60_000), WorkSeries { bin_ms: 60_000, to_ms: None, bins: vec![] });
    }

    mod db {
        use super::*;
        use crate::engine::plot_sync::tests::player;
        use capitol_protocol::ledger::reconcile;
        use capitol_protocol::metrics::tally;
        use capitol_protocol::rest::{GrantItem, GrantRequest};

        /// The stored samples after `since_ms` (Unix ms), oldest first.
        async fn stored_since(db: &PgPool, since_ms: i64) -> Result<Vec<Sample>, sqlx::Error> {
            let rows = stored(db, since_ms, Utc::now().timestamp_millis() + 60_000, None).await?;
            Ok(rows.into_iter().map(|s| serde_json::from_value(s).unwrap()).collect())
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

        /// Who paid whom in a window (#561): a fill from the bid's player to
        /// the ask's, a stall's sale, a commission's and a wage's payouts
        /// from whoever posted them, and a trade window's Strands from the
        /// side that sent them, each only inside the window, and a payout
        /// the court can't place left out
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_windows_payments_name_both_sides(db: PgPool) {
            let (ada, bo, cy) = (player(&db, "ada", 100).await, player(&db, "bo", 100).await, player(&db, "cy", 100).await);
            let exec = |sql: &'static str, binds: Vec<String>| {
                let db = db.clone();
                async move {
                    let mut q = sqlx::query(sql);
                    for b in binds {
                        q = q.bind(b);
                    }
                    q.execute(&db).await.unwrap();
                }
            };
            let s = |u: Uuid| u.to_string();
            let order = |id: Uuid, who: Uuid, side: &'static str| {
                exec(
                    "INSERT INTO market_orders (id, player_id, side, item_type, min_grade, grade_rule, price,
                                                quantity_total, quantity_remaining, status, fee, expires_at)
                     VALUES ($1::UUID, $2::UUID, $3, 'fiber', CASE WHEN $3 = 'buy' THEN 'F' END,
                             CASE WHEN $3 = 'buy' THEN 'average' END, 5, 10, 2, 'open', 1, NOW() + INTERVAL '1 hour')",
                    vec![s(id), s(who), side.into()],
                )
            };
            let (bid, ask) = (Uuid::new_v4(), Uuid::new_v4());
            order(bid, ada, "buy").await;
            order(ask, bo, "sell").await;
            let fill = |quantity: i32, at: &'static str| {
                exec(
                    "INSERT INTO market_trades (id, buy_order_id, sell_order_id, item_type, grade, quality, min_quality,
                                                price, quantity, at)
                     VALUES ($1::UUID, $2::UUID, $3::UUID, 'fiber', 'C', 50, 50, 5, $4::INTEGER, $5::TIMESTAMPTZ)",
                    vec![s(Uuid::new_v4()), s(bid), s(ask), quantity.to_string(), at.into()],
                )
            };
            fill(4, "2026-10-21T12:00:05Z").await;
            fill(3, "2026-10-21T12:00:05Z").await;
            fill(9, "2026-10-21T12:00:11Z").await;
            exec(
                "INSERT INTO bazaar_sales (id, listing_id, stall_id, shop_name, merchant_id, buyer_id,
                                           item_type, quality, price, quantity, at)
                 VALUES ($1::UUID, $1::UUID, 'stall_0', 'Bo''s', $2::UUID, $3::UUID, 'fiber', 50, 3, 2, '2026-10-21T12:00:05Z')",
                vec![s(Uuid::new_v4()), s(bo), s(cy)],
            )
            .await;
            let (commission, offer, trade) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
            exec(
                "INSERT INTO commissions (id, poster_id, item_type, min_quality, quantity, price_per_unit, escrow_strands,
                                          fee, due_at)
                 VALUES ($1::UUID, $2::UUID, 'fiber', 0, 10, 6, 48, 1, NOW() + INTERVAL '1 day')",
                vec![s(commission), s(cy)],
            )
            .await;
            exec(
                "INSERT INTO wage_offers (id, employer_id, work_kind, resource_type, rate, cap, escrow_strands, fee, ends_at)
                 VALUES ($1::UUID, $2::UUID, 'gather', 'grass', 2, 10, 16, 1, NOW() + INTERVAL '1 day')",
                vec![s(offer), s(bo)],
            )
            .await;
            let row = |who: Uuid, amount: i64, kind: &'static str, reference: Uuid, at: &'static str| {
                exec(
                    "INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type,
                                                        reference_id, occurred_at)
                     VALUES ($1::UUID, $2::BIGINT, 100, 100 + $2::BIGINT, $3, $4::UUID, $5::TIMESTAMPTZ)",
                    vec![s(who), amount.to_string(), kind.into(), s(reference), at.into()],
                )
            };
            row(ada, 12, "court_payout", commission, "2026-10-21T12:00:03Z").await;
            row(cy, 4, "court_payout", offer, "2026-10-21T12:00:04Z").await;
            row(cy, 2, "court_payout", offer, "2026-10-21T12:00:10Z").await;
            row(cy, 99, "court_payout", Uuid::new_v4(), "2026-10-21T12:00:04Z").await;
            row(ada, -9, "trade_sent", trade, "2026-10-21T12:00:06Z").await;
            row(cy, 9, "trade_received", trade, "2026-10-21T12:00:06Z").await;
            // Before the window
            row(ada, 50, "court_payout", commission, "2026-10-21T12:00:00Z").await;

            let ms = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis();
            let mut paid = paid(&db, ms("2026-10-21T12:00:00Z"), ms("2026-10-21T12:00:10Z")).await.unwrap();
            paid.sort_by_key(|p| (p.channel, p.from, p.to));
            let mut want = vec![
                Paid { from: ada, to: bo, channel: Channel::Goods, strands: 35 },
                Paid { from: cy, to: bo, channel: Channel::Goods, strands: 6 },
                Paid { from: bo, to: cy, channel: Channel::Wages, strands: 6 },
                Paid { from: cy, to: ada, channel: Channel::Commissions, strands: 12 },
                Paid { from: ada, to: cy, channel: Channel::Trades, strands: 9 },
            ];
            want.sort_by_key(|p| (p.channel, p.from, p.to));
            assert_eq!(paid, want);
        }

        /// A book is what's still open and unexpired on each side, with the
        /// best price either side
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn books_count_whats_open(db: PgPool) {
            let ada = player(&db, "ada", 100).await;
            let order = |side: &'static str, price: i64, remaining: i32, status: &'static str, expires: &'static str| {
                let db = db.clone();
                async move {
                    let buy = side == "buy";
                    sqlx::query(
                        "INSERT INTO market_orders (id, player_id, side, item_type, min_grade, grade_rule, price,
                                                    quantity_total, quantity_remaining, status, fee, expires_at)
                         VALUES ($1, $2, $3, 'fiber', $4, $5, $6, 10, $7, $8, 1, NOW() + $9::INTERVAL)",
                    )
                    .bind(Uuid::new_v4())
                    .bind(ada)
                    .bind(side)
                    .bind(buy.then_some("F"))
                    .bind(buy.then_some("average"))
                    .bind(price)
                    .bind(remaining)
                    .bind(status)
                    .bind(expires)
                    .execute(&db)
                    .await
                    .unwrap();
                }
            };
            order("buy", 4, 10, "open", "1 hour").await;
            order("buy", 5, 3, "open", "1 hour").await;
            order("sell", 7, 6, "open", "1 hour").await;
            order("sell", 6, 2, "open", "1 hour").await;
            // None of these rest on the book
            order("buy", 9, 0, "filled", "1 hour").await;
            order("sell", 2, 5, "cancelled", "1 hour").await;
            order("sell", 1, 5, "open", "-1 minute").await;

            let books = books(&db).await.unwrap();
            assert_eq!(books.len(), 1);
            assert_eq!(books["fiber"], Book { bids: 13, asks: 8, best_bid: Some(5), best_ask: Some(6) });
        }

        /// A series reads the table up to where the ring starts and the ring
        /// after, and asked for fields keeps only those and the times
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_series_runs_from_the_table_into_the_ring(db: PgPool) {
            let now = Utc::now().timestamp_millis() / 60_000 * 60_000;
            let ring = Samples::new();
            // The table, a minute apart: four minutes ago to now
            for minutes in (0..=4).rev() {
                store(&db, &sample(now - minutes * 60_000)).await.unwrap();
            }
            // The ring, ten seconds apart, the last two minutes
            for tens in (0..12).rev() {
                ring.push(sample(now - tens * 10_000));
            }
            let at = |s: &serde_json::Value| s["at_ms"].as_i64().unwrap();
            let all = series(&ring, &db, now - 5 * 60_000, now, None).await.unwrap();
            let ats: Vec<i64> = all.iter().map(|s| (at(s) - now) / 10_000).collect();
            assert_eq!(ats, [-24, -18, -12, -11, -10, -9, -8, -7, -6, -5, -4, -3, -2, -1, 0]);
            let whole: Vec<Sample> = all.into_iter().map(|s| serde_json::from_value(s).unwrap()).collect();
            assert_eq!(whole[0], sample(now - 4 * 60_000));

            let fields = ["strands".to_string()];
            let some = series(&ring, &db, now - 5 * 60_000, now - 100_000, Some(&fields)).await.unwrap();
            assert_eq!(some.iter().map(at).collect::<Vec<_>>(), [now - 240_000, now - 180_000, now - 120_000, now - 110_000, now - 100_000]);
            for s in &some {
                let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
                keys.sort();
                assert_eq!(keys, ["at_ms", "booted_ms", "since_ms", "strands"], "{s}");
            }
            // A path reaches inside a sample the same in the table as in the
            // ring (#411), `*` and all
            let fields = ["load.ticks".to_string(), "load.phases.*.secs".to_string(), "load.nope".to_string()];
            let some = series(&ring, &db, now - 5 * 60_000, now, Some(&fields)).await.unwrap();
            assert_eq!(some.len(), 15);
            for s in &some {
                assert_eq!(s["load"], serde_json::json!({ "ticks": 0, "phases": {} }), "{s}");
                assert_eq!(s.as_object().unwrap().len(), 4, "{s}");
            }
            // Within the ring, the table isn't read
            assert_eq!(series(&ring, &db, now - 30_000, now, None).await.unwrap().len(), 3);
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
