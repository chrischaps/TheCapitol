//! The world's books (C1.7): the units made and destroyed since the server
//! started, and `GET /admin/ledger`'s snapshot of Strands, units and load.
//!
//! Strands keep their own books. Every balance write goes through
//! `currency`, which writes its audit row in the same statement, so the
//! snapshot reads them from the database. The new player's 100 is the one
//! balance with no row (`ledger::STARTING_STRANDS`).
//!
//! Units have no audit table. Each place that makes or destroys them collects
//! [`Flows`] and counts them once its transaction has committed ([`commit`],
//! or [`Committing`] where there's no transaction to hand over). Moves count
//! nothing: trades, splits, escrow, stalls, impounds. A place that makes or
//! destroys units and doesn't count them shows as drift when a soak
//! reconciles two snapshots (`capitol_protocol::ledger::reconcile`). A
//! snapshot and the counted writes take turns at a gate (#408), so its units
//! and its flows always agree. The places that count:
//!
//! - a gather, and the tool it wears out (`tick::persist_gather`)
//! - a craft by hand, its inputs, its outputs and its tool
//!   (`craft_completion`), and a station's automated craft
//!   (`automation_tick::craft_once`)
//! - fiber into and out of the Exchange (`routes::exchange`)
//! - a kit placed as a station, and a station back to its kit
//!   (`station_placement`, `station_removal`, and a foreclosure's or a
//!   terrain change's return of a plot's stations, with whatever found no
//!   room: `property_tax`, `plot_sync`)
//! - a dev grant (`grants`)
//!
//! Land has books of its own: the road in's deed grants (#440), land given
//! with no Strand moving, are counted as they stand (`deed_grants::books`),
//! with what their plots are assessed at.
//!
//! A craft also counts itself by recipe (D2.4): its outputs and their
//! quality, in [`Flows::crafted`], counted with its units, so the
//! Observatory's samples show each recipe's throughput and the quality it
//! makes.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use capitol_protocol::ledger::{AccountBooks, Crafts, Ledger, UnitFlow};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::{RwLock, RwLockReadGuard};
use uuid::Uuid;

use crate::state::AppState;

/// Units in (positive) and out (negative) since the server started, by item
/// type and flow
static FLOWS: LazyLock<Mutex<BTreeMap<(String, UnitFlow), i64>>> = LazyLock::new(Default::default);
/// Crafts finished since the server started, by recipe id (D2.4)
static CRAFTS: LazyLock<Mutex<BTreeMap<String, Crafts>>> = LazyLock::new(Default::default);
/// The gate between unit writes and snapshots (#408). A write holds it shared
/// from just before its commit until its flows are counted; a snapshot holds
/// it alone while it reads the flows and fixes its database snapshot, a few
/// milliseconds, so the two agree. Tokio's lock is fair: once a snapshot
/// asks, new writes queue behind it, so a busy server can't starve it.
/// Each test keeps its own ([`gate`]).
#[cfg(not(test))]
static GATE: LazyLock<RwLock<()>> = LazyLock::new(|| RwLock::new(()));
/// When this process first kept books: two snapshots with different boots
/// have different flows.
static BOOTED_MS: LazyLock<i64> = LazyLock::new(|| chrono::Utc::now().timestamp_millis());

/// How long a snapshot waits for the writes committing to finish. Past it
/// (a commit stuck on the database) it reads without the gate rather than
/// hold every other write up, and says so (`units_settled`).
const GATE_WAIT: Duration = Duration::from_millis(500);

/// The units a transaction makes and destroys, counted once it commits.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Flows {
    units: Vec<(String, UnitFlow, i64)>,
    /// The crafts it finishes: (recipe id, units made, their quality)
    crafts: Vec<(String, i64, i32)>,
}

impl Flows {
    pub fn new() -> Flows {
        Flows::default()
    }

    /// `n` units of `item_type` came in or went out by `flow`. A sink's are
    /// filed negative, so the flows sum to the change in units.
    pub fn add(&mut self, item_type: &str, flow: UnitFlow, n: i64) {
        if n != 0 {
            self.units.push((item_type.to_string(), flow, if flow.is_faucet() { n } else { -n }));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.units.is_empty() && self.crafts.is_empty()
    }

    /// A craft of `recipe_id` finished, making `units` at `quality`. Its
    /// outputs are counted apart, with [`add`](Self::add).
    pub fn crafted(&mut self, recipe_id: &str, units: i64, quality: i32) {
        self.crafts.push((recipe_id.to_string(), units, quality));
    }

    /// A use of a tool of `item_type` that may have been its last: `wear`
    /// deletes a tool it wears to nothing. Tools don't stack, so that's one.
    pub fn wore(&mut self, item_type: &str, wear: &super::tools::Wear) {
        use super::tools::Wear;
        if matches!(wear, Wear::Broke(_) | Wear::Worn { durability: 0 }) {
            self.add(item_type, UnitFlow::WornOut, 1);
        }
    }
}

/// A unit write about to commit: it holds the gate shared from
/// [`begin`](Self::begin) until it's counted by [`done`](Self::done) or
/// dropped. Begin it just before the commit, so a snapshot never reads units
/// its flows don't have yet, and await nothing but the commit while it's
/// open: new writes wait behind a snapshot that waits on this one.
pub struct Committing {
    _gate: RwLockReadGuard<'static, ()>,
}

impl Committing {
    pub async fn begin() -> Committing {
        LazyLock::force(&BOOTED_MS);
        Committing { _gate: gate().read().await }
    }

    /// The write committed: count its flows.
    pub fn done(self, flows: Flows) {
        #[cfg(test)]
        TEST_FLOWS.with(|mine| add(&mut mine.borrow_mut(), &flows));
        #[cfg(test)]
        TEST_CRAFTS.with(|mine| add_crafts(&mut mine.borrow_mut(), &flows));
        add(&mut FLOWS.lock().unwrap_or_else(|e| e.into_inner()), &flows);
        add_crafts(&mut CRAFTS.lock().unwrap_or_else(|e| e.into_inner()), &flows);
    }
}

fn add(all: &mut BTreeMap<(String, UnitFlow), i64>, flows: &Flows) {
    for (item_type, flow, n) in &flows.units {
        *all.entry((item_type.clone(), *flow)).or_insert(0) += n;
    }
}

fn add_crafts(all: &mut BTreeMap<String, Crafts>, flows: &Flows) {
    for (recipe_id, units, quality) in &flows.crafts {
        all.entry(recipe_id.clone()).or_default().add(*units, *quality);
    }
}

// The database tests run side by side in one process, each on a thread of
// its own (sqlx's test runtime is current-thread), so each reads the flows
// its own writes counted, and keeps a gate of its own for them. With one gate
// for the process, a test's snapshot waited on every other test's commits, to
// databases it doesn't read, and under ci.sh full's load that came to the
// whole GATE_WAIT (#569).
#[cfg(test)]
thread_local! {
    static TEST_FLOWS: std::cell::RefCell<BTreeMap<(String, UnitFlow), i64>> = Default::default();
    static TEST_CRAFTS: std::cell::RefCell<BTreeMap<String, Crafts>> = Default::default();
    static TEST_GATE: &'static RwLock<()> = Box::leak(Box::default());
}

/// The gate between unit writes and snapshots: the process's, or in a test
/// its thread's.
fn gate() -> &'static RwLock<()> {
    #[cfg(not(test))]
    return &GATE;
    #[cfg(test)]
    return TEST_GATE.with(|gate| *gate);
}

/// Commits `tx`, then counts `flows`.
pub async fn commit(tx: Transaction<'_, Postgres>, flows: Flows) -> Result<(), sqlx::Error> {
    if flows.is_empty() {
        return tx.commit().await;
    }
    let committing = Committing::begin().await;
    tx.commit().await?;
    committing.done(flows);
    Ok(())
}

/// The flows counted so far, by item type then flow.
fn flows_now() -> BTreeMap<String, BTreeMap<String, i64>> {
    #[cfg(not(test))]
    let all = FLOWS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    #[cfg(test)]
    let all = TEST_FLOWS.with(|mine| mine.borrow().clone());
    let mut by_item: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for ((item_type, flow), n) in all.iter() {
        by_item.entry(item_type.clone()).or_default().insert(flow.as_str().to_string(), *n);
    }
    by_item
}

/// The world's books now: `GET /admin/ledger`, the soak's and the
/// sampler's. The accounts, escrow and units are one database snapshot, and
/// the unit flows are read with it behind the gate, so the two agree. A
/// write that holds the gate past [`GATE_WAIT`] makes it read without, and
/// say so (`units_settled`).
pub async fn snapshot(state: &AppState) -> Result<Ledger, sqlx::Error> {
    let db = &state.db;
    let mut ledger = books(db).await?;
    ledger.load = super::load::read(tick_period(state.config.sim_tick_scale));
    ledger.load.players_in_world = state.game.read().await.players.len() as u64;
    ledger.load.db_pool_size = db.size();
    ledger.load.db_pool_idle = db.num_idle() as u32;
    ledger.load.db_transactions = db_transactions(db).await?;
    Ok(ledger)
}

/// The crafts counted so far, by recipe id.
fn crafts_now() -> BTreeMap<String, Crafts> {
    #[cfg(not(test))]
    return CRAFTS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    #[cfg(test)]
    return TEST_CRAFTS.with(|mine| mine.borrow().clone());
}

/// The books without the load: accounts, escrow, units and unit flows.
pub(crate) async fn books(db: &PgPool) -> Result<Ledger, sqlx::Error> {
    // The connection before the gate: a snapshot waiting on the pool with the
    // gate shut, while the writes holding every connection wait on the gate,
    // would wait for ever
    let mut tx = db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let gate = tokio::time::timeout(GATE_WAIT, gate().write()).await.ok();
    let units_settled = gate.is_some();
    let unit_flows = flows_now();
    let crafts = crafts_now();
    // The first statement takes the snapshot the rest share. Once it has, no
    // later commit can show, so the gate opens before the slow reads.
    sqlx::query("SELECT 1").execute(&mut *tx).await?;
    drop(gate);
    let units = units_held(&mut tx).await?;

    let rows: Vec<(Uuid, String, bool, i64)> = sqlx::query_as(
        "SELECT p.id, p.name, a.kind = 'citizen', p.strand_balance
         FROM players p JOIN accounts a ON a.id = p.account_id
         ORDER BY p.name, p.id",
    )
    .fetch_all(&mut *tx)
    .await?;
    let sums: Vec<(Uuid, String, i64)> = sqlx::query_as(
        "SELECT player_id, transaction_type, SUM(amount)::BIGINT FROM currency_transactions
         GROUP BY player_id, transaction_type",
    )
    .fetch_all(&mut *tx)
    .await?;
    let escrow_strands = held_strands(&mut tx).await?;
    let held_flows = super::guilds::held_flows(&mut tx).await?;
    let grants = super::deed_grants::books(&mut tx).await?;
    tx.commit().await?;

    let mut flows: BTreeMap<Uuid, BTreeMap<String, i64>> = BTreeMap::new();
    for (player_id, kind, sum) in sums {
        flows.entry(player_id).or_default().insert(kind, sum);
    }
    let accounts = rows
        .into_iter()
        .map(|(player_id, name, citizen, strands)| AccountBooks {
            player_id,
            name,
            citizen,
            strands,
            flows: flows.remove(&player_id).unwrap_or_default(),
        })
        .collect();

    Ok(Ledger {
        at_ms: chrono::Utc::now().timestamp_millis(),
        booted_ms: *BOOTED_MS,
        accounts,
        escrow_strands,
        units,
        unit_flows,
        units_settled,
        crafts,
        grants,
        held_flows,
        load: Default::default(),
    })
}

/// The tick's period at `scale`, as `tick::run_tick_loop` sets it.
fn tick_period(scale: f64) -> Duration {
    Duration::from_secs_f64(capitol_protocol::rules::TICK_MS as f64 / 1000.0 / scale)
}

/// Every Strand held for someone, outside every balance: the ledger's
/// `escrow_strands`, which its escrow flows explain (S0.2). Each table that
/// holds Strands adds its sum here, or the ledger reports its holdings as
/// drift: the market's buy orders, the court's commissions and wage offers,
/// and the guilds' treasuries (S4.2).
pub(crate) async fn held_strands(conn: &mut sqlx::PgConnection) -> Result<i64, sqlx::Error> {
    let market: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(escrow_strands), 0)::BIGINT FROM market_orders")
        .fetch_one(&mut *conn)
        .await?;
    Ok(market + court_held(conn).await? + super::guilds::held(conn).await?)
}

/// Strands the Contract Court holds: what's unspent of open commissions'
/// escrow (S2.1), and wage offers' (S2.6).
async fn court_held(conn: &mut sqlx::PgConnection) -> Result<i64, sqlx::Error> {
    Ok(super::commissions::held(conn).await? + super::wages::held(conn).await?)
}

/// Every unit in the world, by item type.
async fn units_held(conn: &mut sqlx::PgConnection) -> Result<BTreeMap<String, i64>, sqlx::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT i.item_type, SUM(u.count)::BIGINT FROM item_units u JOIN items i ON i.id = u.item_id
         GROUP BY i.item_type",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Transactions committed and rolled back in this database, by anyone.
async fn db_transactions(db: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT (xact_commit + xact_rollback)::BIGINT FROM pg_stat_database WHERE datname = current_database()",
    )
    .fetch_one(db)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sink_is_filed_negative_and_nothing_is_filed_for_none() {
        let mut flows = Flows::new();
        flows.add("log", UnitFlow::Gathered, 3);
        flows.add("log", UnitFlow::Consumed, 2);
        flows.add("plank", UnitFlow::Crafted, 0);
        assert_eq!(
            flows.units,
            vec![("log".to_string(), UnitFlow::Gathered, 3), ("log".to_string(), UnitFlow::Consumed, -2)]
        );
    }

    /// `quantity` of `kind` at Q60 in `slot` of `container`, owned by `owner`
    async fn put(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, kind: &str, quantity: i32) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, $2, 60, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(kind)
        .bind(quantity)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
        id
    }

    async fn container(db: &PgPool, kind: &str, owner: Option<Uuid>, station: Option<Uuid>) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id, station_id) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(kind)
            .bind(owner)
            .bind(station)
            .execute(db)
            .await
            .unwrap();
        id
    }

    /// A storage chest of `owner`'s, on `lot` if it's given, and its box
    async fn chest(db: &PgPool, owner: Uuid, lot: Option<Uuid>) -> (Uuid, Uuid) {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality)
             VALUES ($1, 'storage_chest', $2, 8000, 6800, $3, 50)",
        )
        .bind(id)
        .bind(owner)
        .bind(lot)
        .execute(db)
        .await
        .unwrap();
        (id, container(db, "station_inventory", None, Some(id)).await)
    }

    /// A fresh world (S0.2): the court's escrow box is there exactly once,
    /// nothing is held, and its books hold against an empty one.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_fresh_world_has_one_court_escrow_and_no_drift(db: PgPool) {
        use capitol_protocol::ledger::reconcile;

        let boxes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM containers WHERE container_type = 'court_escrow'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(boxes, 1);
        let second = sqlx::query("INSERT INTO containers (id, container_type) VALUES ($1, 'court_escrow')")
            .bind(Uuid::new_v4())
            .execute(&db)
            .await;
        assert!(second.is_err(), "a second court escrow box");

        let fresh = books(&db).await.unwrap();
        assert_eq!(fresh.escrow_strands, 0);
        let mut empty = fresh.clone();
        empty.accounts.clear();
        let books = reconcile(&empty, &fresh);
        assert!(books.strands.holds(), "{:#?}", books.strands);
        assert!(books.units.holds(), "{:#?}", books.units);
    }

    /// A day's work between two snapshots: a grant, a deposit and a
    /// withdrawal, a craft, a chest taken down, an hour's tax paid, and a
    /// foreclosure. Every Strand and every unit is explained.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_days_work_keeps_the_books_balanced(db: PgPool) {
        use crate::engine::plot_sync::tests::{player, plot};
        use capitol_protocol::ledger::reconcile;
        use capitol_protocol::rest::{GrantItem, GrantRequest};
        use capitol_protocol::rules::stacking::Units;

        // Ada works; Bo pays an hour's tax; Cy can't, and is foreclosed
        let ada = player(&db, "ada", 100).await;
        let ada_bag = container(&db, "player_inventory", Some(ada), None).await;
        container(&db, "crafting_output", Some(ada), None).await;
        let fiber = put(&db, ada, ada_bag, 0, "fiber", 20).await;
        let ten = put(&db, ada, ada_bag, 1, "fiber", 10).await;
        let (ada_chest, _) = chest(&db, ada, None).await;
        let bo = player(&db, "bo", 100).await;
        plot(&db, Some(bo), 6000).await;
        let cy = player(&db, "cy", 0).await;
        container(&db, "player_inventory", Some(cy), None).await;
        let cy_lot = plot(&db, Some(cy), 6000).await;
        let (_, cy_box) = chest(&db, cy, Some(cy_lot)).await;
        put(&db, cy, cy_box, 0, "fiber", 12).await;
        // An hour charges a 6000 plot 1 Strand from this carry, and Cy's 480th forecloses
        sqlx::query("UPDATE plots SET tax_carry = 48")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("UPDATE plots SET tax_owed = 479 WHERE id = $1")
            .bind(cy_lot)
            .execute(&db)
            .await
            .unwrap();

        let start = books(&db).await.unwrap();
        assert!(start.units_settled);

        let request = GrantRequest {
            strands: 500,
            items: vec![GrantItem {
                item_type: "fiber".into(),
                quantity: 5,
                quality: 50,
            }],
        };
        super::super::grants::grant(&db, ada, &request).await.unwrap();
        // 10 of her fiber into the Exchange, and 4 back out
        let fees = capitol_protocol::tunables::Fees::default();
        crate::routes::exchange::deposit(&db, ada, &[ten], &fees).await.unwrap();
        crate::routes::exchange::withdraw(&db, ada, 4, &fees).await.unwrap();
        // Two fiber into rope
        let craft = super::super::craft_completion::FinishedCraft {
            player_id: ada,
            operation_id: Uuid::new_v4(),
            recipe_id: "rope_basic".into(),
            inputs: vec![(fiber, Units::uniform(2, 60))],
            outputs: vec![super::super::craft_completion::CraftOutput {
                item_type: "rope".into(),
                item_name: "Rope".into(),
                quantity: 1,
                quality: 60,
                stackable: true,
                durability: None,
            }],
            tool: None,
            station_id: None,
            wage: None,
        };
        super::super::craft_completion::finish_craft(&db, &craft).await.unwrap();
        super::super::station_removal::remove_station(&db, ada, ada_chest).await.unwrap();
        let taxed = super::super::property_tax::process_property_taxes(&db, capitol_protocol::tunables::PROPERTY_TAX_RATE).await;
        assert_eq!(taxed.foreclosures.len(), 1, "Cy's plot");

        let end = books(&db).await.unwrap();
        let books = reconcile(&start, &end);
        assert!(books.strands.holds(), "{:#?}", books.strands);
        assert!(books.units.holds(), "{:#?}", books.units);
        // Each flow was seen, not just balanced
        let f = &books.strands.flows;
        assert_eq!(f["admin_adjustment"], 500);
        assert!(f["exchange_deposit"] > 0 && f["exchange_withdraw"] < 0);
        assert_eq!(f["property_tax"], -1, "Bo's hour, written down now");
        assert_eq!(f["foreclosure_refund"], 6000 - 480);
        let u = &books.units.flows;
        assert_eq!((u["granted"], u["deposited"], u["withdrawn"], u["consumed"], u["crafted"]), (5, -10, 4, -2, 1));
        assert_eq!(u["kit_returned"], 2, "Ada's chest and Cy's");
        assert_eq!(books.units.items["chest_kit"], (2, 2));
        // The rope is counted by its recipe too (D2.4)
        let rope = Crafts { runs: 1, units: 1, quality: 60 };
        assert_eq!(end.crafts.get("rope_basic").map(|c| c.since(&start.crafts.get("rope_basic").copied().unwrap_or_default())), Some(rope));
    }

    #[tokio::test]
    async fn a_counted_write_is_in_the_flows_and_opens_the_gate() {
        let mut flows = Flows::new();
        flows.add("ledger_test_widget", UnitFlow::Granted, 5);
        let committing = Committing::begin().await;
        assert!(gate().try_write().is_err(), "shut to a snapshot while it commits");
        committing.done(flows);
        assert_eq!(flows_now()["ledger_test_widget"]["granted"], 5);
        // A write dropped uncounted (its commit failed) opens it too
        drop(Committing::begin().await);
        assert!(gate().try_write().is_ok());
    }

    /// Unit writes that never stop while snapshots are taken (#408): no
    /// snapshot is starved, and every one agrees with the first. A snapshot
    /// may still read unsettled when a commit holds the gate past
    /// [`GATE_WAIT`], as one can on a machine too busy to answer it (#569),
    /// so the writers time how long each holds it: a snapshot that didn't
    /// settle must have met such a commit, never only a stream of quick ones.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn snapshots_settle_while_writes_never_stop(db: PgPool) {
        use crate::engine::plot_sync::tests::player;
        use capitol_protocol::ledger::reconcile;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        let ada = player(&db, "ada", 100).await;
        let bag = container(&db, "player_inventory", Some(ada), None).await;
        let stack = put(&db, ada, bag, 0, "fiber", 1).await;
        // The longest any write held the gate, in milliseconds
        let longest = Arc::new(AtomicU64::new(0));
        let writers: Vec<_> = (0..4)
            .map(|_| {
                let db = db.clone();
                let longest = longest.clone();
                tokio::spawn(async move {
                    for _ in 0..60 {
                        let mut tx = db.begin().await.unwrap();
                        sqlx::query("UPDATE item_units SET count = count + 1 WHERE item_id = $1")
                            .bind(stack)
                            .execute(&mut *tx)
                            .await
                            .unwrap();
                        let mut flows = Flows::new();
                        flows.add("fiber", UnitFlow::Granted, 1);
                        // commit's own steps, with the hold timed
                        let committing = Committing::begin().await;
                        let held = std::time::Instant::now();
                        tx.commit().await.unwrap();
                        longest.fetch_max(held.elapsed().as_millis() as u64, Ordering::Relaxed);
                        committing.done(flows);
                    }
                })
            })
            .collect();

        let first = books(&db).await.unwrap();
        let mut taken = 1;
        let mut unsettled = Vec::new();
        while !writers.iter().all(|w| w.is_finished()) {
            let now = books(&db).await.unwrap();
            if now.units_settled {
                let units = reconcile(&first, &now).units;
                assert!(units.holds(), "snapshot {taken}: {units:#?}");
            } else {
                unsettled.push(taken);
            }
            taken += 1;
        }
        for writer in writers {
            writer.await.unwrap();
        }
        let longest = longest.load(Ordering::Relaxed);
        assert!(
            unsettled.is_empty() || longest >= GATE_WAIT.as_millis() as u64,
            "snapshots {unsettled:?} of {taken} didn't settle, and no write held the gate past {GATE_WAIT:?} (the longest {longest} ms)"
        );
        let last = books(&db).await.unwrap();
        assert!(last.units_settled);
        assert!(reconcile(&first, &last).units.holds());
        assert_eq!(last.unit_flows.get("fiber").and_then(|f| f.get("granted")), Some(&240));
        assert!(taken > 2, "only {taken} snapshots alongside the writes");
    }
}
