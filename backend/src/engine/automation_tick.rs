//! The automation tick (E4.3, TDD §3.7, GDD §3.7): stations that work on
//! their own.
//!
//! Every second a running station advances by the tick's 10-tick cadence.
//! When it has run its recipe's duration the craft completes, in one
//! transaction ([`craft_once`]):
//! - the inputs come out of the station's box, the best units of each stack
//!   first, as a craft by hand takes them ([`pick_inputs`]);
//! - the quality is the cascade with the station and its installed tool, and
//!   no minigame (`crafting::output_quality`);
//! - the output goes into the same box, owned by the station's owner, as
//!   `craft_completion::place_in` puts a craft's ([`place`]);
//! - the installed tool wears (`station_tools::wear_slot`);
//! - and the setting's `done` goes up, or the run stops once a count is made.
//!
//! A craft that can't complete changes nothing, and the station pauses with
//! the reason: `missing_input`, `output_full`, `tool_missing`, or `tool_broken`
//! when the craft just done used the tool up. A paused station tries again
//! every cycle, so it resumes on its own once the box is loaded or emptied or
//! a tool goes in. A free player's stations pause `owner_offline` while
//! they're away and resume when they're back; a subscriber's keep working
//! (D8). Every pause queues an `automation_paused` notice for the owner
//! (#138) in the transaction that writes it.
//!
//! What runs is in memory ([`Works`], on `GameState`), loaded from
//! `stations.automation` at startup, so a restart resumes where it was. The
//! routes keep it in step through `automation::announce`. The DB stays the
//! truth: every write locks the station row and checks the setting is still
//! the one the run was for, so a stop or a new setting that lands mid-cycle
//! wins. Like the market, it runs in a task of its own, and no lock is held
//! across an await.
//!
//! A cycle decides what each station does in memory, then writes them
//! [`WORKERS`] at a time, each worker on a connection of its own for the cycle
//! (#212). Stations lock different rows, so their crafts don't wait on each
//! other, and their commits flush together. A craft is about eight round
//! trips: the station's lock, its box and tool slot, the box's stacks and then
//! their units, the takes in one statement, the tool's wear, the output and
//! the setting. The Gazette's tally of what they made is one statement for
//! the cycle, once they're in (`gazette::tally_many`). A burst of completions
//! too big for a cycle makes it run long rather than lose anything, and a
//! restart spreads the stations' first crafts over their durations so they
//! don't all fall due together ([`Works::load`]).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, MutexGuard};
use tracing::Instrument;

use capitol_protocol::ledger::UnitFlow;
use capitol_protocol::rest::{
    AutomationState, PauseReason, Produced, StationAutomation, StationTool, Subscription, SubscriptionStatus,
    PRODUCTION_LOG_LEN,
};
use capitol_protocol::rules::stacking::{Pick, Units};
use capitol_protocol::rules::{quality, tools as tool_rules};
use capitol_protocol::ws::ServerMessage;
use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::{Connection, PgConnection, PgPool};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::craft_completion::CraftOutput;
use super::crafting::{output_quality, roll_outputs};
use super::events::GameEvent;
use super::station_tools::SlotTool;
use super::tools::{BrokenTool, HeldTool, Wear};
use super::units::{Marked, NewStack};
use super::{gazette, ledger, notices, station_tools, units, GameState};
use crate::models::{ItemType, Recipe};

/// How often the automation tick runs: the tick's 10-tick cadence (TDD §3.2).
pub const CYCLE_MS: u64 = 1000;
/// Ticks a running station advances each cycle.
pub const CYCLE_TICKS: i32 = 10;
/// How many stations' writes a cycle runs side by side, each on a connection
/// of its own (#212). The server's pool is sized for it (`main.rs`).
pub const WORKERS: usize = 4;

/// The stations that work on their own, as the automation tick sees them.
#[derive(Debug, Default)]
pub struct Works {
    runs: Mutex<HashMap<Uuid, Run>>,
}

#[derive(Debug, Clone, PartialEq)]
struct Run {
    automation: StationAutomation,
    /// Ticks into the craft under way
    progress: i32,
    /// The last [`PRODUCTION_LOG_LEN`] crafts, newest last
    log: VecDeque<Produced>,
    /// Loaded at startup: its first advance starts it part way into its
    /// craft, so the stations a restart resumes fall due spread out (#212)
    stagger: bool,
}

impl Works {
    /// Every station that isn't stopped, as `stations.automation` has it.
    /// How far each had got isn't kept, so each starts its craft part way in,
    /// at a point its id picks ([`head_start`]), not all at the start: started
    /// together, they'd complete together, every craft after.
    pub async fn load(db: &PgPool) -> Result<Works, sqlx::Error> {
        let rows: Vec<(Uuid, Json<StationAutomation>)> = sqlx::query_as(
            "SELECT id, automation FROM stations
             WHERE automation IS NOT NULL AND automation->>'state' <> 'stopped'",
        )
        .fetch_all(db)
        .await?;
        let works = Works::default();
        for (id, automation) in rows {
            works.set(id, &automation.0);
            if let Some(run) = works.runs().get_mut(&id) {
                run.stagger = true;
            }
        }
        Ok(works)
    }

    fn runs(&self) -> MutexGuard<'_, HashMap<Uuid, Run>> {
        // Each change is one call, so a poisoned lock still holds whole runs
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `station_id` is set to `automation` now. A new recipe, or a start from
    /// stopped, begins its craft again; a pause keeps how far it had got.
    pub fn set(&self, station_id: Uuid, automation: &StationAutomation) {
        let mut runs = self.runs();
        let run = runs.entry(station_id).or_insert_with(|| Run {
            automation: automation.clone(),
            progress: 0,
            log: VecDeque::new(),
            stagger: false,
        });
        let restarted = run.automation.recipe_id != automation.recipe_id
            || automation.state == AutomationState::Stopped
            || (run.automation.state == AutomationState::Stopped && automation.state != AutomationState::Stopped);
        if restarted {
            run.progress = 0;
            run.stagger = false;
        }
        run.automation = automation.clone();
    }

    /// `station_id` is gone.
    pub fn forget(&self, station_id: Uuid) {
        self.runs().remove(&station_id);
    }

    /// The production log of each of `station_ids` that has one, for `StationInfo`.
    pub fn production(&self, station_ids: &[Uuid]) -> HashMap<Uuid, Vec<Produced>> {
        let runs = self.runs();
        station_ids
            .iter()
            .filter_map(|id| {
                let run = runs.get(id)?;
                (!run.log.is_empty()).then(|| (*id, run.log.iter().cloned().collect()))
            })
            .collect()
    }

    /// The stations that mean to work: running or paused.
    fn active(&self) -> Vec<(Uuid, StationAutomation)> {
        self.runs()
            .iter()
            .filter(|(_, r)| r.automation.state != AutomationState::Stopped)
            .map(|(id, r)| (*id, r.automation.clone()))
            .collect()
    }

    /// Advance `station_id` a cycle. Whether its craft has run `duration`.
    fn advance(&self, station_id: Uuid, duration: i32) -> bool {
        let mut runs = self.runs();
        let Some(run) = runs.get_mut(&station_id) else { return false };
        if std::mem::take(&mut run.stagger) {
            run.progress = head_start(station_id, duration);
        }
        run.progress = (run.progress + CYCLE_TICKS).min(duration.max(1));
        run.progress >= duration
    }

    /// A craft completed: log what it made, and start the next.
    fn produced(&self, station_id: Uuid, automation: &StationAutomation, made: &[Produced]) {
        let mut runs = self.runs();
        let Some(run) = runs.get_mut(&station_id) else { return };
        run.automation = automation.clone();
        run.progress = 0;
        for p in made {
            if run.log.len() == PRODUCTION_LOG_LEN {
                run.log.pop_front();
            }
            run.log.push_back(p.clone());
        }
    }
}

/// How far into a craft of `duration` ticks a station resumed by a restart
/// starts: a point in `0..duration` its id picks, the same on every restart.
fn head_start(station_id: Uuid, duration: i32) -> i32 {
    let picked = u32::from_le_bytes(station_id.as_bytes()[..4].try_into().expect("four bytes"));
    (picked % duration.max(1) as u32) as i32
}

/// What one cycle did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cycle {
    pub crafts: usize,
    pub paused: usize,
    pub resumed: usize,
}

/// The owner of a station, and whether their stations may work now.
#[derive(Debug, Clone)]
struct Owner {
    player_id: Uuid,
    /// A subscriber's stations work while they're away (D8)
    subscribed: bool,
}

/// Run the automation tick forever, a cycle every `CYCLE_MS / scale` (the
/// economy sim's `SIM_TICK_SCALE`, E2.7, runs it faster with the world).
pub async fn run_automation_loop(game: std::sync::Arc<RwLock<GameState>>, db: PgPool, scale: f64) {
    let (works, events) = {
        let g = game.read().await;
        (g.works.clone(), g.event_sender.clone())
    };
    tracing::info!("Automation every {CYCLE_MS}ms, {} stations working", works.active().len());
    let period = std::time::Duration::from_secs_f64(CYCLE_MS as f64 / 1000.0 / scale.max(f64::MIN_POSITIVE));
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let started = std::time::Instant::now();
        let timed = super::load::Loop::Automation.timed();
        let cycle = run_cycle(&works, &db, &game, &events, Utc::now())
            .instrument(tracing::info_span!("loop.automation"))
            .await;
        drop(timed);
        if cycle != Cycle::default() {
            tracing::debug!("Automation: {cycle:?} in {:?}", started.elapsed());
        }
    }
}

/// One cycle at `now`.
pub async fn run_cycle(
    works: &Works,
    db: &PgPool,
    game: &RwLock<GameState>,
    events: &broadcast::Sender<GameEvent>,
    now: DateTime<Utc>,
) -> Cycle {
    let mut cycle = Cycle::default();
    let active = works.active();
    if active.is_empty() {
        return cycle;
    }
    let ids: Vec<Uuid> = active.iter().map(|(id, _)| *id).collect();
    let owners = match owners_of(db, &ids, now).await {
        Ok(owners) => owners,
        Err(e) => {
            tracing::warn!("Automation: reading owners: {e}");
            return cycle;
        }
    };
    let (online, recipes, item_types) = {
        let g = game.read().await;
        let online: HashSet<Uuid> = owners.values().map(|o| o.player_id).filter(|p| g.players.contains_key(p)).collect();
        let recipes: HashMap<String, Recipe> = active
            .iter()
            .filter_map(|(_, a)| Some((a.recipe_id.clone(), g.recipes.get(&a.recipe_id)?.clone())))
            .collect();
        let item_types: HashMap<String, ItemType> = recipes
            .values()
            .flat_map(|r| r.outputs.iter())
            .filter_map(|o| Some((o.item_type.clone(), g.item_types.get(&o.item_type)?.clone())))
            .collect();
        (online, recipes, item_types)
    };

    let mut jobs = Vec::new();
    for (station_id, automation) in active {
        let Some(owner) = owners.get(&station_id) else {
            // The station's gone
            works.forget(station_id);
            continue;
        };
        let may_work = owner.subscribed || online.contains(&owner.player_id);
        let step = match (&automation.state, may_work) {
            (AutomationState::Running, false) => Step::Pause(PauseReason::OwnerOffline),
            (AutomationState::Paused { reason: PauseReason::OwnerOffline }, true) => Step::Resume,
            (AutomationState::Paused { .. }, false) => continue,
            (AutomationState::Running, true) => {
                let Some(recipe) = recipes.get(&automation.recipe_id) else {
                    tracing::warn!("Automation: station {station_id} makes unknown recipe {}", automation.recipe_id);
                    continue;
                };
                if !works.advance(station_id, recipe.base_duration_ticks) {
                    continue;
                }
                Step::Craft
            }
            // Paused on its box or its tool: try the craft again
            (AutomationState::Paused { .. }, true) => Step::Craft,
            (AutomationState::Stopped, _) => continue,
        };
        jobs.push(Job { station_id, automation, owner_id: owner.player_id, step });
    }

    let results = work_all(db, &jobs, &recipes, &item_types, now).await;
    // What the owners made, for the Gazette (E3.9), once it's in
    let made: Vec<(Uuid, &str, i32)> = jobs
        .iter()
        .zip(&results)
        .filter_map(|(job, result)| match result {
            Some(Ok(Some(Done::Made(made)))) => Some((job.owner_id, made)),
            _ => None,
        })
        .flat_map(|(owner_id, made)| made.produced.iter().map(move |p| (owner_id, p.item_type.as_str(), p.quantity)))
        .collect();
    if let Err(e) = gazette::tally_many(db, now, &made).await {
        tracing::warn!("Automation: the Gazette misses this cycle's tally of {} outputs: {e}", made.len());
    }

    for (job, result) in jobs.into_iter().zip(results) {
        let Job { station_id, automation, owner_id, .. } = job;
        match result {
            // Nothing to write, or not reached this cycle
            Some(Ok(None)) | None => {}
            Some(Ok(Some(Done::Made(made)))) => {
                cycle.crafts += 1;
                if !matches!(automation.state, AutomationState::Running) {
                    cycle.resumed += 1;
                }
                announce_made(works, events, station_id, owner_id, &automation, made);
            }
            Some(Ok(Some(Done::State(Transition::Paused { automation: set, station_type, reason, notice_id })))) => {
                cycle.paused += 1;
                works.set(station_id, &set);
                let _ = events.send(GameEvent::AutomationPaused { owner_id, station_id, station_type, reason, notice_id });
                let _ = events.send(GameEvent::StationAutomationChanged { station_id, automation: set });
            }
            Some(Ok(Some(Done::State(Transition::Resumed(set))))) => {
                cycle.resumed += 1;
                works.set(station_id, &set);
                let _ = events.send(GameEvent::StationAutomationChanged { station_id, automation: set });
            }
            Some(Ok(Some(Done::State(Transition::Changed(Some(set)))))) => works.set(station_id, &set),
            Some(Ok(Some(Done::State(Transition::Changed(None))))) => works.forget(station_id),
            // The next cycle tries again
            Some(Err(e)) => tracing::warn!("Automation: station {station_id}: {e}"),
        }
    }
    cycle
}

/// One station's part of a cycle, decided in memory.
struct Job {
    station_id: Uuid,
    /// Its setting as the run has it
    automation: StationAutomation,
    owner_id: Uuid,
    step: Step,
}

type Outcome = Result<Option<Done>, sqlx::Error>;

/// Write `jobs`, [`WORKERS`] at a time. Each worker holds a connection for the
/// cycle, not one acquired (and pinged) a craft (#208), and takes the next job
/// as it finishes one. The results are in `jobs`' order. A worker stops at its
/// first error, so a lost database costs a few warnings a cycle, not one a
/// station, and the jobs it didn't reach are `None`: the next cycle tries them.
async fn work_all(
    db: &PgPool,
    jobs: &[Job],
    recipes: &HashMap<String, Recipe>,
    item_types: &HashMap<String, ItemType>,
    now: DateTime<Utc>,
) -> Vec<Option<Outcome>> {
    let next = Mutex::new(0..jobs.len());
    let worker = || async {
        let mut done = Vec::new();
        let mut conn = match db.acquire().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("Automation: no connection: {e}");
                return done;
            }
        };
        loop {
            let Some(i) = next.lock().unwrap_or_else(|e| e.into_inner()).next() else { break };
            let outcome = work(&mut conn, &jobs[i], recipes, item_types, now).await;
            let failed = outcome.is_err();
            done.push((i, outcome));
            if failed {
                break;
            }
        }
        done
    };
    let workers = (0..WORKERS.min(jobs.len())).map(|_| worker());
    let mut results: Vec<Option<Outcome>> = jobs.iter().map(|_| None).collect();
    for (i, outcome) in futures::future::join_all(workers).await.into_iter().flatten() {
        results[i] = Some(outcome);
    }
    results
}

/// Write one station's step. `None`: nothing to write (a craft blocked for
/// the reason its pause already gives).
async fn work(
    conn: &mut PgConnection,
    job: &Job,
    recipes: &HashMap<String, Recipe>,
    item_types: &HashMap<String, ItemType>,
    now: DateTime<Utc>,
) -> Outcome {
    let Job { station_id, automation, .. } = job;
    match &job.step {
        Step::Pause(reason) => Ok(Some(Done::State(pause(conn, *station_id, automation, reason.clone()).await?))),
        Step::Resume => Ok(Some(Done::State(set_running(conn, *station_id, automation).await?))),
        Step::Craft => {
            let Some(recipe) = recipes.get(&automation.recipe_id) else { return Ok(None) };
            Ok(match craft_once(conn, *station_id, automation, recipe, item_types, now).await? {
                Attempt::Made(made) => Some(Done::Made(*made)),
                Attempt::Blocked(reason) if still_says_why(&automation.state, &reason) => None,
                Attempt::Blocked(reason) => Some(Done::State(pause(conn, *station_id, automation, reason).await?)),
                Attempt::Changed(now_set) => Some(Done::State(Transition::Changed(now_set))),
            })
        }
    }
}

/// Does a station paused as `state` already say why a craft is `blocked`?
/// The same reason does, and so does `tool_broken` for the empty slot it left.
fn still_says_why(state: &AutomationState, blocked: &PauseReason) -> bool {
    match state {
        AutomationState::Paused { reason: PauseReason::ToolBroken } => {
            matches!(blocked, PauseReason::ToolBroken | PauseReason::ToolMissing)
        }
        AutomationState::Paused { reason } => reason == blocked,
        _ => false,
    }
}

enum Step {
    Pause(PauseReason),
    Resume,
    Craft,
}

enum Done {
    Made(Made),
    State(Transition),
}

/// A change to a station's setting other than a craft.
#[derive(Debug, Clone, PartialEq)]
enum Transition {
    Paused { automation: StationAutomation, station_type: String, reason: PauseReason, notice_id: Option<Uuid> },
    Resumed(StationAutomation),
    /// The DB has a different setting than the run was for (stopped,
    /// reconfigured), or `None`: the station's gone. Nothing was written.
    Changed(Option<StationAutomation>),
}

/// What a completed craft did.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    /// The setting after it: `done` up, and stopped or paused if that ended the run
    pub automation: StationAutomation,
    pub produced: Vec<Produced>,
    /// The station's box
    pub container_id: Uuid,
    /// What the tool slot holds now, when the recipe used it
    pub tool: Option<Option<StationTool>>,
    pub broken: Option<BrokenTool>,
    /// The `automation_paused` notice, when the craft used the tool up
    pub notice_id: Option<Uuid>,
    pub station_type: String,
}

/// The outcome of trying a craft.
#[derive(Debug, Clone, PartialEq)]
pub enum Attempt {
    Made(Box<Made>),
    /// It can't, and nothing changed
    Blocked(PauseReason),
    /// The setting isn't the one the run was for any more (or the station's
    /// gone: `None`). Nothing changed.
    Changed(Option<StationAutomation>),
}

fn announce_made(
    works: &Works,
    events: &broadcast::Sender<GameEvent>,
    station_id: Uuid,
    owner_id: Uuid,
    before: &StationAutomation,
    made: Made,
) {
    works.produced(station_id, &made.automation, &made.produced);
    for produced in &made.produced {
        let _ = events.send(GameEvent::AutomationProduced {
            owner_id,
            station_id,
            produced: produced.clone(),
            done: made.automation.done,
        });
    }
    let _ = events.send(GameEvent::InventoryUpdated {
        player_id: owner_id,
        container_id: made.container_id,
        slots: Vec::new(),
    });
    if let Some(tool) = made.tool {
        station_tools::announce(events, station_id, tool);
    }
    if let Some(broken) = &made.broken {
        super::tools::announce_broken(events, broken);
    }
    if let AutomationState::Paused { reason } = &made.automation.state {
        let _ = events.send(GameEvent::AutomationPaused {
            owner_id,
            station_id,
            station_type: made.station_type.clone(),
            reason: reason.clone(),
            notice_id: made.notice_id,
        });
    }
    // Everyone hears only a change of state: started again, paused or finished
    if made.automation.state != before.state {
        let _ = events.send(GameEvent::StationAutomationChanged { station_id, automation: made.automation });
    }
}

/// The owner of each of `station_ids` that still exists.
async fn owners_of(db: &PgPool, station_ids: &[Uuid], now: DateTime<Utc>) -> Result<HashMap<Uuid, Owner>, sqlx::Error> {
    let rows: Vec<(Uuid, Uuid, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT s.id, s.owner_id, a.subscription_status, a.subscription_until
         FROM stations s
         JOIN players p ON p.id = s.owner_id
         JOIN accounts a ON a.id = p.account_id
         WHERE s.id = ANY($1)",
    )
    .bind(station_ids)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, player_id, status, until)| {
            let subscription = Subscription { status: SubscriptionStatus::parse(&status), until };
            (id, Owner { player_id, subscribed: subscription.is_active_at(now) })
        })
        .collect())
}

/// The station row, locked, as a craft or a pause needs it.
struct Locked {
    owner_id: Uuid,
    station_type: String,
    quality: i32,
    automation: Option<StationAutomation>,
}

async fn lock_station(conn: &mut PgConnection, station_id: Uuid) -> Result<Option<Locked>, sqlx::Error> {
    let row: Option<(Uuid, String, i32, Option<Json<StationAutomation>>)> = sqlx::query_as(
        "SELECT owner_id, station_type, kit_quality, automation FROM stations WHERE id = $1 FOR UPDATE",
    )
    .bind(station_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|(owner_id, station_type, quality, automation)| Locked {
        owner_id,
        station_type,
        quality,
        automation: automation.map(|a| a.0),
    }))
}

/// Is `stored` still the run `expected` was for: the same recipe, not stopped?
fn same_run(stored: &Option<StationAutomation>, expected: &StationAutomation) -> bool {
    stored.as_ref().is_some_and(|s| s.recipe_id == expected.recipe_id && s.state != AutomationState::Stopped)
}

async fn save(conn: &mut PgConnection, station_id: Uuid, automation: &StationAutomation) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE stations SET automation = $2 WHERE id = $1")
        .bind(station_id)
        .bind(Json(automation))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Queue the owner's `automation_paused` notice for `station_id`.
async fn queue_pause(
    conn: &mut PgConnection,
    owner_id: Uuid,
    station_id: Uuid,
    station_type: &str,
    reason: &PauseReason,
) -> Result<Uuid, sqlx::Error> {
    let message = ServerMessage::AutomationPaused {
        station_id,
        station_type: station_type.to_string(),
        reason: reason.clone(),
    };
    notices::queue(&mut *conn, owner_id, &station_id.to_string(), &message).await
}

/// Pause `station_id` for `reason`, with a notice to its owner.
async fn pause(
    conn: &mut PgConnection,
    station_id: Uuid,
    expected: &StationAutomation,
    reason: PauseReason,
) -> Result<Transition, sqlx::Error> {
    let mut tx = conn.begin().await?;
    let Some(station) = lock_station(&mut tx, station_id).await? else {
        return Ok(Transition::Changed(None));
    };
    if !same_run(&station.automation, expected) {
        return Ok(Transition::Changed(station.automation));
    }
    let mut automation = station.automation.unwrap_or_else(|| expected.clone());
    automation.state = AutomationState::Paused { reason: reason.clone() };
    save(&mut tx, station_id, &automation).await?;
    let notice_id = queue_pause(&mut tx, station.owner_id, station_id, &station.station_type, &reason).await?;
    tx.commit().await?;
    Ok(Transition::Paused { automation, station_type: station.station_type, reason, notice_id: Some(notice_id) })
}

/// Set a paused `station_id` running again: its owner is back.
async fn set_running(
    conn: &mut PgConnection,
    station_id: Uuid,
    expected: &StationAutomation,
) -> Result<Transition, sqlx::Error> {
    let mut tx = conn.begin().await?;
    let Some(station) = lock_station(&mut tx, station_id).await? else {
        return Ok(Transition::Changed(None));
    };
    if !same_run(&station.automation, expected) {
        return Ok(Transition::Changed(station.automation));
    }
    let mut automation = station.automation.unwrap_or_else(|| expected.clone());
    automation.state = AutomationState::Running;
    save(&mut tx, station_id, &automation).await?;
    tx.commit().await?;
    Ok(Transition::Resumed(automation))
}

/// A stack in a station's box, as [`pick_inputs`] chooses from it.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxStack {
    pub id: Uuid,
    pub item_type: String,
    pub slot: Option<i32>,
    pub units: Units,
}

/// The units one craft of `recipe` takes from `stacks` (in slot order), and
/// the inputs its quality is worked out from: for each input in recipe order,
/// stacks in slot order until it's met, the best units of each first. That's
/// what a craft by hand takes from the same stacks (`crafting::start_crafting`).
/// `Err` names an input the box lacks.
pub fn pick_inputs(recipe: &Recipe, stacks: &[BoxStack]) -> Result<(Vec<(Uuid, Units)>, quality::Inputs), String> {
    let mut taken = Vec::new();
    let mut inputs = quality::Inputs::default();
    for need in &recipe.inputs {
        let mut left = need.quantity;
        for stack in stacks.iter().filter(|s| s.item_type == need.item_type) {
            if left <= 0 {
                break;
            }
            let n = left.min(stack.units.count() as i32);
            let Some(units) = stack.units.pick(n, Pick::Best) else { continue };
            left -= n;
            inputs.add_units(&stack.item_type, &units);
            taken.push((stack.id, units));
        }
        if left > 0 {
            return Err(need.item_type.clone());
        }
    }
    Ok((taken, inputs))
}

/// Where an output goes in a station's box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Onto this stack
    Onto(Uuid),
    /// A new stack in this slot
    Slot(i32),
}

/// Where `output` goes among `stacks`, a box of `slot_count` slots, as
/// `craft_completion::place_in` puts a craft's output: onto the first stack of
/// its type in slot order if it stacks and that stack can hold it, or else
/// the first free slot. `None`: the box is full.
fn place(stacks: &[BoxStack], slot_count: i32, output: &CraftOutput) -> Option<Place> {
    if output.stackable {
        let first = stacks
            .iter()
            .filter(|s| s.item_type == output.item_type)
            .min_by_key(|s| (s.slot.is_none(), s.slot, s.id));
        if let Some(first) = first {
            if first.units.clone().merge(&Units::uniform(output.quantity, output.quality)) {
                return Some(Place::Onto(first.id));
            }
        }
    }
    (0..slot_count).find(|slot| !stacks.iter().any(|s| s.slot == Some(*slot))).map(Place::Slot)
}

/// The stacks in `container_id`, in slot order, locked: the stacks in one
/// statement and their units in the next ([`units::units_of_many`]).
async fn box_stacks(conn: &mut PgConnection, container_id: Uuid) -> Result<Vec<BoxStack>, sqlx::Error> {
    let rows: Vec<(Uuid, String, Option<i32>)> = sqlx::query_as(
        "SELECT id, item_type, slot_index FROM items WHERE container_id = $1
         ORDER BY slot_index NULLS LAST, id FOR UPDATE",
    )
    .bind(container_id)
    .fetch_all(&mut *conn)
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|(id, ..)| *id).collect();
    let mut units = units::units_of_many(conn, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|(id, item_type, slot)| BoxStack { id, item_type, slot, units: units.remove(&id).unwrap_or_default() })
        .collect())
}

/// A station's box and what its tool slot holds, read under the station's
/// lock in one round trip.
#[derive(sqlx::FromRow)]
struct BoxAndSlot {
    container_id: Uuid,
    slot_count: i32,
    tool_owner: Option<Uuid>,
    item_id: Option<Uuid>,
    item_type: Option<String>,
    item_name: Option<String>,
    quality: Option<i32>,
    durability: Option<i32>,
    tool_type: Option<String>,
    base_power: Option<f64>,
    base_durability: Option<i32>,
    tier: Option<String>,
}

impl BoxAndSlot {
    /// The slot's tool, if it's a `tool_type` with uses left: what
    /// `station_tools::installed_tool` finds, and what a wear needs.
    fn tool(&self, station_id: Uuid, tool_type: tool_rules::ToolType) -> Option<(HeldTool, SlotTool)> {
        if self.tool_type.as_deref() != Some(tool_type.as_str()) {
            return None;
        }
        let (item_id, item_type, item_name, quality, durability) = (
            self.item_id?,
            self.item_type.clone()?,
            self.item_name.clone()?,
            self.quality?,
            self.durability.filter(|d| *d > 0)?,
        );
        let max_durability = tool_rules::max_durability(self.base_durability.unwrap_or(0), quality);
        let held = HeldTool {
            item_id,
            item_type: item_type.clone(),
            tool: tool_rules::Tool { base_power: self.base_power?, quality, durability, max_durability },
            tier: self.tier.as_deref().and_then(tool_rules::Tier::from_str),
            station_id: Some(station_id),
        };
        let slot = SlotTool { item_id, item_type, item_name, quality, max_durability, owner_id: self.tool_owner };
        Some((held, slot))
    }
}

/// [`BoxAndSlot`] for `station_id`; `None` if it has no box. Read it after
/// locking the station, in a statement of its own, so it sees any change to
/// the slot that the lock waited for.
async fn box_and_slot(conn: &mut PgConnection, station_id: Uuid) -> Result<Option<BoxAndSlot>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.id AS container_id, ct.slot_count, s.tool_owner, i.id AS item_id, i.item_type,
                it.name AS item_name, i.quality, i.durability, it.tool_type, it.base_power,
                it.max_durability AS base_durability, it.tier
         FROM stations s
         JOIN containers c ON c.station_id = s.id
         JOIN container_types ct ON ct.id = c.container_type
         LEFT JOIN items i ON i.id = s.installed_tool
         LEFT JOIN item_types it ON it.id = i.item_type
         WHERE s.id = $1
         LIMIT 1",
    )
    .bind(station_id)
    .fetch_optional(&mut *conn)
    .await
}

/// Complete one craft of `recipe` at `station_id`, in one transaction on
/// `conn`, or change nothing and say why not.
pub async fn craft_once(
    conn: &mut PgConnection,
    station_id: Uuid,
    expected: &StationAutomation,
    recipe: &Recipe,
    item_types: &HashMap<String, ItemType>,
    now: DateTime<Utc>,
) -> Result<Attempt, sqlx::Error> {
    let mut tx = conn.begin().await?;
    let Some(station) = lock_station(&mut tx, station_id).await? else {
        return Ok(Attempt::Changed(None));
    };
    if !same_run(&station.automation, expected) {
        return Ok(Attempt::Changed(station.automation));
    }
    let mut automation = station.automation.clone().unwrap_or_else(|| expected.clone());
    let Some(found) = box_and_slot(&mut tx, station_id).await? else {
        // configure refuses a station with no box, so there's nothing to run
        return Ok(Attempt::Changed(None));
    };
    let container_id = found.container_id;

    let tool = match recipe.tool_type {
        Some(tool_type) => match found.tool(station_id, tool_type) {
            Some(tool) => Some(tool),
            None => return Ok(Attempt::Blocked(PauseReason::ToolMissing)),
        },
        None => None,
    };

    let mut stacks = box_stacks(&mut tx, container_id).await?;
    let (taken, inputs) = match pick_inputs(recipe, &stacks) {
        Ok(picked) => picked,
        Err(item_type) => return Ok(Attempt::Blocked(PauseReason::MissingInput { item_type })),
    };
    // What it uses up, makes and wears out, counted once it's written (C1.7)
    let mut flows = ledger::Flows::new();
    // Take them in memory, then write every take in one statement:
    // (stack, taken, left), one entry a stack
    let mut takes: Vec<(Uuid, Units, Units)> = Vec::new();
    for (item_id, lot) in &taken {
        let stack = stacks.iter_mut().find(|s| s.id == *item_id).expect("picked from the box");
        let Some(units) = stack.units.take_units(lot) else {
            // Picked from these very units; but never make from nothing
            return Err(sqlx::Error::Protocol(format!("stack {item_id} no longer holds {lot:?}")));
        };
        flows.add(&stack.item_type, UnitFlow::Consumed, units.count());
        match takes.iter_mut().find(|(id, ..)| id == item_id) {
            Some((_, all, left)) => {
                all.merge(&units);
                *left = stack.units.clone();
            }
            None => takes.push((*item_id, units, stack.units.clone())),
        }
    }
    if !takes.is_empty() {
        let writes: Vec<(Uuid, &Units, &Units)> = takes.iter().map(|(id, taken, left)| (*id, taken, left)).collect();
        units::write_takes(&mut tx, &writes).await?;
    }
    // A stack used up is gone, and its slot free
    stacks.retain(|s| !takes.iter().any(|(id, _, left)| *id == s.id && left.is_empty()));

    let tool_quality = tool.as_ref().map_or(0, |(held, _)| tool_rules::term_quality(&held.tool));
    let quality = output_quality(&inputs, station.quality, tool_quality, recipe);
    let outputs = roll_outputs(recipe, item_types, quality);

    let (mut slot_now, mut broken) = (None, None);
    if let Some((_, slot)) = tool {
        let tool_type = slot.item_type.clone();
        let Some((wear, now_tool)) = station_tools::wear_slot(&mut tx, station_id, slot).await? else {
            return Ok(Attempt::Blocked(PauseReason::ToolMissing));
        };
        flows.wore(&tool_type, &wear);
        slot_now = Some(now_tool);
        if let Wear::Broke(b) = wear {
            broken = Some(b);
        }
    }

    flows.crafted(&recipe.id, outputs.iter().map(|o| o.quantity as i64).sum(), quality);
    for output in &outputs {
        flows.add(&output.item_type, UnitFlow::Crafted, output.quantity as i64);
        let made = Units::uniform(output.quantity, output.quality);
        // The station's owner made it (#231), as a craft by hand marks it
        // with the crafter
        let marked = Marked::by(Some(station.owner_id), &made);
        match place(&stacks, found.slot_count, output) {
            None => return Ok(Attempt::Blocked(PauseReason::OutputFull)),
            Some(Place::Onto(id)) => {
                units::add(&mut tx, id, &marked).await?;
                if let Some(stack) = stacks.iter_mut().find(|s| s.id == id) {
                    stack.units.merge(&made);
                }
            }
            Some(Place::Slot(slot)) => {
                let stack = NewStack {
                    item_type: &output.item_type,
                    owner_id: Some(station.owner_id),
                    container_id: Some(container_id),
                    slot_index: Some(slot),
                    durability: output.durability,
                };
                let id = units::insert_stack(&mut tx, stack, &marked).await?;
                stacks.push(BoxStack { id, item_type: output.item_type.clone(), slot: Some(slot), units: made });
            }
        }
    }

    automation.done += 1;
    let tool_gone = recipe.tool_type.is_some() && matches!(slot_now, Some(None));
    let mut notice_id = None;
    automation.state = if automation.is_finished() {
        AutomationState::Stopped
    } else if tool_gone {
        let reason = PauseReason::ToolBroken;
        notice_id = Some(queue_pause(&mut tx, station.owner_id, station_id, &station.station_type, &reason).await?);
        AutomationState::Paused { reason }
    } else {
        AutomationState::Running
    };
    save(&mut tx, station_id, &automation).await?;
    ledger::commit(tx, flows).await?;

    let produced = outputs
        .iter()
        .map(|o| Produced { item_type: o.item_type.clone(), quantity: o.quantity, quality: o.quality, at: now })
        .collect();
    Ok(Attempt::Made(Box::new(Made {
        automation,
        produced,
        container_id,
        tool: slot_now,
        broken,
        notice_id,
        station_type: station.station_type,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActionState, PlayerState, TerrainConfig, WorldGenParams};
    use capitol_protocol::rest::AutomationMode;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    fn shipped() -> (HashMap<String, Recipe>, HashMap<String, ItemType>) {
        let content = crate::content::load().unwrap();
        (
            content.recipes.iter().map(|r| (r.id.clone(), r.to_recipe())).collect(),
            content.items.iter().map(|i| (i.id.clone(), i.to_item_type())).collect(),
        )
    }

    fn running(recipe_id: &str, mode: AutomationMode) -> StationAutomation {
        StationAutomation { recipe_id: recipe_id.into(), mode, state: AutomationState::Running, done: 0 }
    }

    fn someone(id: Uuid) -> PlayerState {
        PlayerState {
            id,
            account_id: Uuid::new_v4(),
            name: "Ada".into(),
            x: 0.0,
            y: 0.0,
            dest_x: None,
            dest_y: None,
            speed: 50.0,
            strand_balance: 0,
            current_zone: None,
            action_state: ActionState::Idle,
            citizen: false,
            steer: None,
        }
    }

    #[test]
    fn a_set_keeps_a_pauses_progress_and_a_new_recipe_starts_again() {
        let works = Works::default();
        let id = Uuid::new_v4();
        let mut a = running("tool_handle", AutomationMode::Forever);
        works.set(id, &a);
        assert!(!works.advance(id, 30));
        assert!(!works.advance(id, 30));
        a.state = AutomationState::Paused { reason: PauseReason::OwnerOffline };
        works.set(id, &a);
        a.state = AutomationState::Running;
        works.set(id, &a);
        assert!(works.advance(id, 30), "a pause kept its 20 ticks");

        works.set(id, &running("knapped_stone", AutomationMode::Forever));
        assert!(!works.advance(id, 30), "a new recipe starts its craft again");
        a.state = AutomationState::Stopped;
        works.set(id, &a);
        assert!(works.active().is_empty());
    }

    #[test]
    fn the_log_keeps_the_last_fifty() {
        let works = Works::default();
        let id = Uuid::new_v4();
        let a = running("tool_handle", AutomationMode::Forever);
        works.set(id, &a);
        for q in 0..60 {
            let p = Produced { item_type: "tool_handle".into(), quantity: 1, quality: q, at: Utc::now() };
            works.produced(id, &a, &[p]);
        }
        let log = works.production(&[id]).remove(&id).unwrap();
        assert_eq!(log.len(), PRODUCTION_LOG_LEN);
        assert_eq!((log[0].quality, log[49].quality), (10, 59), "oldest first, the first ten gone");
        assert!(works.production(&[Uuid::new_v4()]).is_empty());
    }

    /// Automation takes the units a craft by hand takes from the same stacks,
    /// and with the variance pinned makes the very same quality: the cascade
    /// with the station and the tool, minus the minigame.
    #[test]
    fn automation_takes_and_makes_what_a_hand_craft_does() {
        use crate::engine::crafting::{process_crafting, start_crafting};
        let (recipes, kinds) = shipped();
        let mut rng = StdRng::seed_from_u64(178);
        for recipe_id in ["tool_handle", "iron_axe_head", "charcoal", "iron_ingot"] {
            let mut recipe = recipes[recipe_id].clone();
            recipe.quality_weights.variance_min = 0;
            recipe.quality_weights.variance_max = 0;
            for _ in 0..100 {
                // Two to four stacks of each input, of mixed qualities, shuffled
                let mut stacks = Vec::new();
                for need in &recipe.inputs {
                    for _ in 0..rng.gen_range(2..=4) {
                        let units = Units::from_pairs(
                            (0..rng.gen_range(1..=3)).map(|_| (rng.gen_range(0..=100), rng.gen_range(1..=need.quantity))),
                        );
                        stacks.push(BoxStack { id: Uuid::new_v4(), item_type: need.item_type.clone(), slot: None, units });
                    }
                }
                for i in (1..stacks.len()).rev() {
                    stacks.swap(i, rng.gen_range(0..=i));
                }
                let station = rng.gen_range(0..=100);
                let tool_quality = if recipe.tool_type.is_some() { rng.gen_range(0..=100) } else { 0 };

                let (taken, inputs) = pick_inputs(&recipe, &stacks).unwrap();
                let automated = output_quality(&inputs, station, tool_quality, &recipe);

                // By hand: the same stacks, each input's in slot order
                let items: Vec<crate::models::Item> = recipe
                    .inputs
                    .iter()
                    .flat_map(|need| stacks.iter().filter(move |s| s.item_type == need.item_type))
                    .map(|s| crate::models::Item {
                        id: s.id,
                        item_type: s.item_type.clone(),
                        quality: s.units.mean().unwrap(),
                        quantity: s.units.count() as i32,
                        owner_id: None,
                        container_id: None,
                        slot_index: None,
                        created_at: Utc::now(),
                        durability: None,
                        min_quality: s.units.min().unwrap(),
                        units: s.units.clone(),
                        maker: None,
                    })
                    .collect();
                let (events, _rx) = broadcast::channel(64);
                let mut ada = someone(Uuid::new_v4());
                // A whole tool's term is its quality
                let tool = recipe.tool_type.map(|_| crate::engine::tools::HeldTool {
                    item_id: Uuid::new_v4(),
                    item_type: "stone_hammer".into(),
                    tool: tool_rules::Tool { base_power: 6.0, quality: tool_quality, durability: 60, max_durability: 60 },
                    tier: None,
                    station_id: None,
                });
                let mut ops = HashMap::new();
                start_crafting(&mut ada, &recipe, &items, station, tool, &mut ops, &events).unwrap();
                let one: HashMap<_, _> = [(recipe.id.clone(), recipe.clone())].into();
                let mut players: HashMap<_, _> = [(ada.id, ada)].into();
                let mut done = Vec::new();
                while done.is_empty() {
                    done = process_crafting(&mut players, &one, &mut ops, &kinds, &events);
                }
                assert_eq!(done[0].inputs, taken, "{recipe_id}: the same units");
                assert_eq!(done[0].outputs[0].quality, automated, "{recipe_id}: the same quality");
            }
        }
        // Short of an input names it
        let recipe = &recipes["tool_handle"];
        let sticks = BoxStack { id: Uuid::new_v4(), item_type: "sticks".into(), slot: None, units: Units::uniform(5, 50) };
        assert_eq!(pick_inputs(recipe, &[sticks]).unwrap_err(), "rope");
    }

    fn output(item_type: &str, quantity: i32, stackable: bool) -> CraftOutput {
        CraftOutput {
            item_type: item_type.into(),
            item_name: item_type.into(),
            quantity,
            quality: 60,
            stackable,
            durability: None,
        }
    }

    fn stack_at(slot: Option<i32>, item_type: &str, units: Units) -> BoxStack {
        BoxStack { id: Uuid::new_v4(), item_type: item_type.into(), slot, units }
    }

    /// An output goes where `craft_completion::place_in` would put it
    #[test]
    fn an_output_joins_the_first_stack_of_its_kind_or_takes_the_first_free_slot() {
        let later = stack_at(Some(3), "tool_handle", Units::uniform(1, 40));
        let first = stack_at(Some(1), "tool_handle", Units::uniform(1, 90));
        let stacks = vec![stack_at(Some(0), "sticks", Units::uniform(4, 70)), later.clone(), first.clone()];
        assert_eq!(place(&stacks, 4, &output("tool_handle", 1, true)), Some(Place::Onto(first.id)), "in slot order");
        assert_eq!(place(&stacks, 4, &output("iron_axe", 1, false)), Some(Place::Slot(2)), "a tool takes a slot");
        assert_eq!(place(&stacks, 4, &output("rope", 1, true)), Some(Place::Slot(2)));
        let full = vec![stack_at(Some(0), "tool_handle", Units::uniform(i32::MAX, 50))];
        assert_eq!(place(&full, 2, &output("tool_handle", 1, true)), Some(Place::Slot(1)), "a stack that can't hold it");
        assert_eq!(place(&full, 1, &output("rope", 1, true)), None, "no room");
    }

    /// Stations a restart resumes start part way into their crafts, so they
    /// don't all complete in one cycle, and none waits longer than a craft
    #[test]
    fn a_restart_spreads_the_stations_first_crafts() {
        let works = Works::default();
        let ids: Vec<Uuid> = (0..200).map(|_| Uuid::new_v4()).collect();
        for id in &ids {
            works.set(*id, &running("tool_handle", AutomationMode::Forever));
            works.runs().get_mut(id).unwrap().stagger = true;
        }
        let duration = 600;
        let mut due_by_cycle = Vec::new();
        for _ in 0..duration / CYCLE_TICKS {
            due_by_cycle.push(ids.iter().filter(|id| works.advance(**id, duration)).count());
            for id in &ids {
                if works.runs()[id].progress >= duration {
                    works.produced(*id, &running("tool_handle", AutomationMode::Forever), &[]);
                }
            }
        }
        assert_eq!(due_by_cycle.iter().sum::<usize>(), ids.len(), "each once in its first craft's time");
        assert!(due_by_cycle.iter().all(|n| *n < 20), "spread out: {due_by_cycle:?}");
        assert_eq!(head_start(ids[0], duration), head_start(ids[0], duration), "the same on every restart");
    }

    /// These need the local Postgres (`docker-compose up -d`). Run them with
    /// `cargo test automation -- --ignored`.
    mod db {
        use super::*;
        use crate::engine::automation;
        use crate::engine::units::test_units::{all_in, stack_of};

        const HERE: Option<(f64, f64)> = Some((0.0, 0.0));

        struct World {
            db: PgPool,
            game: RwLock<GameState>,
            events: broadcast::Sender<GameEvent>,
            recipes: HashMap<String, Recipe>,
        }

        impl World {
            async fn new(db: PgPool) -> World {
                crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
                static TERRAIN: std::sync::OnceLock<TerrainConfig> = std::sync::OnceLock::new();
                let terrain = TERRAIN.get_or_init(TerrainConfig::get_default).clone();
                let mut game = GameState::with_terrain(WorldGenParams::default(), terrain);
                let (recipes, kinds) = shipped();
                game.recipes = recipes.clone();
                game.item_types = kinds;
                let events = game.event_sender.clone();
                World { db, game: RwLock::new(game), events, recipes }
            }

            async fn works(&self) -> std::sync::Arc<Works> {
                self.game.read().await.works.clone()
            }

            async fn cycle(&self) -> Cycle {
                let works = self.works().await;
                run_cycle(&works, &self.db, &self.game, &self.events, Utc::now()).await
            }

            /// Bring `station_id`'s craft to its end, and run the cycle that completes it
            async fn finish_craft(&self, station_id: Uuid) -> Cycle {
                if let Some(run) = self.works().await.runs().get_mut(&station_id) {
                    run.progress = i32::MAX / 2;
                }
                self.cycle().await
            }

            /// Set `station_id` making `recipe_id` and start it, as the routes do
            async fn start(&self, owner: Uuid, station_id: Uuid, recipe_id: &str, mode: AutomationMode) {
                automation::configure(&self.db, &self.recipes, owner, station_id, recipe_id, mode).await.unwrap();
                let a = automation::start(&self.db, &self.recipes, owner, station_id).await.unwrap();
                automation::announce(&*self.game.read().await, station_id, a);
            }
        }

        /// A player, subscribed for a day or not. (player, inventory)
        async fn player(db: &PgPool, name: &str, subscribed: bool) -> (Uuid, Uuid) {
            let account = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO accounts (id, email, password_hash, subscription_status, subscription_until)
                 VALUES ($1, $2, 'x', $3, $4)",
            )
            .bind(account)
            .bind(format!("{name}@test"))
            .bind(if subscribed { "active" } else { "free" })
            .bind(subscribed.then(|| Utc::now() + chrono::Duration::days(1)))
            .execute(db)
            .await
            .unwrap();
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(account)
                .bind(name)
                .execute(db)
                .await
                .unwrap();
            let inventory = Uuid::new_v4();
            sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
                .bind(inventory)
                .bind(id)
                .execute(db)
                .await
                .unwrap();
            (id, inventory)
        }

        /// A `kind` station of `owner`'s at the origin, of quality 50. (station, its box)
        async fn station(db: &PgPool, owner: Uuid, kind: &str) -> (Uuid, Uuid) {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
                 VALUES ($1, $2, $3, 0, 0, 50)",
            )
            .bind(id)
            .bind(kind)
            .bind(owner)
            .execute(db)
            .await
            .unwrap();
            let container = Uuid::new_v4();
            sqlx::query("INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2)")
                .bind(container)
                .bind(id)
                .execute(db)
                .await
                .unwrap();
            (id, container)
        }

        /// A hammer with `durability` left, installed in `anvil`
        async fn install_hammer(db: &PgPool, owner: Uuid, inventory: Uuid, anvil: Uuid, durability: i32) {
            let hammer: Uuid = sqlx::query_scalar(
                "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
                 VALUES ($1, 'stone_hammer', 60, 1, $2, $3, 0, $4) RETURNING id",
            )
            .bind(Uuid::new_v4())
            .bind(owner)
            .bind(inventory)
            .bind(durability)
            .fetch_one(db)
            .await
            .unwrap();
            crate::engine::station_tools::install_tool(db, owner, anvil, hammer, HERE).await.unwrap();
        }

        async fn stored(db: &PgPool, station_id: Uuid) -> StationAutomation {
            automation::automation_at(db, &[station_id]).await.unwrap().remove(&station_id).unwrap()
        }

        fn paused(reason: PauseReason) -> AutomationState {
            AutomationState::Paused { reason }
        }

        /// The `automation_paused` notices waiting for `player_id`, taking them
        async fn pause_notices(db: &PgPool, player_id: Uuid) -> Vec<PauseReason> {
            notices::take_all(db, player_id, capitol_protocol::tunables::PROPERTY_TAX_RATE)
                .await
                .unwrap()
                .into_iter()
                .filter_map(|m| match m {
                    ServerMessage::AutomationPaused { reason, .. } => Some(reason),
                    _ => None,
                })
                .collect()
        }

        /// Plenty to make handles from, in `inbox`
        async fn load_handles(db: &PgPool, owner: Uuid, inbox: Uuid) {
            stack_of(db, owner, inbox, 0, "sticks", &Units::uniform(20, 70)).await;
            stack_of(db, owner, inbox, 1, "rope", &Units::uniform(10, 50)).await;
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_running_station_crafts_from_its_box_into_its_box(db: PgPool) {
            let world = World::new(db.clone()).await;
            let (ada, _) = player(&db, "ada", true).await;
            let (bench, inbox) = station(&db, ada, "workbench").await;
            stack_of(&db, ada, inbox, 0, "sticks", &Units::uniform(4, 70)).await;
            stack_of(&db, ada, inbox, 1, "rope", &Units::uniform(2, 50)).await;
            world.start(ada, bench, "tool_handle", AutomationMode::Count(2)).await;
            let mut rx = world.events.subscribe();

            // Sixty ticks a handle: five cycles make nothing, the sixth one
            for _ in 0..5 {
                assert_eq!(world.cycle().await, Cycle::default());
            }
            assert_eq!(world.cycle().await.crafts, 1);
            let handle = all_in(&db, inbox, "tool_handle").await;
            assert_eq!(handle.count(), 1);
            let q = handle.mean().unwrap();
            let mut inputs = quality::Inputs::default();
            inputs.add("sticks", 2, 70);
            inputs.add("rope", 1, 50);
            let (low, high) = quality::output_range(&inputs, 50, 0, &world.recipes["tool_handle"].quality_weights);
            assert!(low <= q && q <= high, "Q{q} outside {low}..={high}");
            assert_eq!(all_in(&db, inbox, "sticks").await, Units::uniform(2, 70));
            assert_eq!(stored(&db, bench).await.done, 1);
            let owner: Uuid =
                sqlx::query_scalar("SELECT owner_id FROM items WHERE container_id = $1 AND item_type = 'tool_handle'")
                    .bind(inbox)
                    .fetch_one(&db)
                    .await
                    .unwrap();
            assert_eq!(owner, ada, "the output is the owner's");
            let tallied: i64 = sqlx::query_scalar(
                "SELECT quantity FROM production_tally WHERE player_id = $1 AND item_type = 'tool_handle'",
            )
            .bind(ada)
            .fetch_one(&db)
            .await
            .unwrap();
            assert_eq!(tallied, 1, "the Gazette counts it, once");

            let mut produced = 0;
            while let Ok(e) = rx.try_recv() {
                match e {
                    GameEvent::AutomationProduced { owner_id, station_id, produced: p, done } => {
                        assert_eq!((owner_id, station_id, p.quality, done), (ada, bench, q, 1));
                        produced += 1;
                    }
                    GameEvent::StationAutomationChanged { .. } => panic!("a craft that changes no state isn't broadcast"),
                    _ => {}
                }
            }
            assert_eq!(produced, 1);

            // The second makes the count, and the run stops
            for _ in 0..6 {
                world.cycle().await;
            }
            let a = stored(&db, bench).await;
            assert_eq!((a.done, &a.state), (2, &AutomationState::Stopped));
            assert_eq!(all_in(&db, inbox, "tool_handle").await.count(), 2, "onto the same stack");
            assert!(all_in(&db, inbox, "sticks").await.is_empty());
            assert_eq!(world.works().await.production(&[bench]).remove(&bench).unwrap().len(), 2);
            for _ in 0..12 {
                assert_eq!(world.cycle().await, Cycle::default(), "a stopped station rests");
            }
            assert!(pause_notices(&db, ada).await.is_empty(), "finishing isn't a pause");
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn it_pauses_on_an_empty_or_full_box_and_goes_on_once_it_clears(db: PgPool) {
            let world = World::new(db.clone()).await;
            let (ada, _) = player(&db, "ada", true).await;
            let (bench, inbox) = station(&db, ada, "workbench").await;
            stack_of(&db, ada, inbox, 0, "sticks", &Units::uniform(3, 70)).await;
            stack_of(&db, ada, inbox, 1, "rope", &Units::uniform(1, 50)).await;
            world.start(ada, bench, "tool_handle", AutomationMode::Forever).await;

            assert_eq!(world.finish_craft(bench).await.crafts, 1);
            // One stick and no rope left
            assert_eq!(world.finish_craft(bench).await.paused, 1);
            let missing = PauseReason::MissingInput { item_type: "sticks".into() };
            assert_eq!(stored(&db, bench).await.state, paused(missing.clone()));
            assert_eq!(pause_notices(&db, ada).await, vec![missing]);
            assert_eq!(world.cycle().await, Cycle::default(), "the same reason isn't said twice");
            assert!(pause_notices(&db, ada).await.is_empty());

            // The first handle went in slot 1, where the rope was
            stack_of(&db, ada, inbox, 5, "sticks", &Units::uniform(3, 70)).await;
            assert_eq!(world.cycle().await.paused, 1, "now it's the rope");
            assert_eq!(stored(&db, bench).await.state, paused(PauseReason::MissingInput { item_type: "rope".into() }));
            stack_of(&db, ada, inbox, 6, "rope", &Units::uniform(5, 50)).await;
            let resumed = world.cycle().await;
            assert_eq!((resumed.crafts, resumed.resumed), (1, 1), "loaded, it crafts at once");
            let a = stored(&db, bench).await;
            assert_eq!((a.done, &a.state), (2, &AutomationState::Running));

            // Take the handles out and fill every free slot: no room for the next
            sqlx::query("DELETE FROM items WHERE container_id = $1 AND item_type = 'tool_handle'")
                .bind(inbox)
                .execute(&db)
                .await
                .unwrap();
            // Sticks enough that the craft empties no stack, and so frees no slot
            stack_of(&db, ada, inbox, 0, "sticks", &Units::uniform(4, 70)).await;
            let used: Vec<i32> = sqlx::query_scalar("SELECT slot_index FROM items WHERE container_id = $1")
                .bind(inbox)
                .fetch_all(&db)
                .await
                .unwrap();
            let mut filler = Vec::new();
            for slot in (0..16).filter(|s| !used.contains(s)) {
                filler.push(stack_of(&db, ada, inbox, slot, "stone", &Units::uniform(1, 10)).await);
            }
            pause_notices(&db, ada).await;
            assert_eq!(world.finish_craft(bench).await.paused, 1);
            assert_eq!(stored(&db, bench).await.state, paused(PauseReason::OutputFull));
            assert_eq!(all_in(&db, inbox, "sticks").await.count(), 6, "a blocked craft takes nothing");
            assert_eq!(pause_notices(&db, ada).await, vec![PauseReason::OutputFull]);

            sqlx::query("DELETE FROM items WHERE id = $1").bind(filler[0]).execute(&db).await.unwrap();
            assert_eq!(world.cycle().await.crafts, 1, "emptied, it crafts");
            assert_eq!(stored(&db, bench).await.state, AutomationState::Running);
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_tool_that_breaks_or_goes_pauses_it_and_a_new_one_starts_it(db: PgPool) {
            let world = World::new(db.clone()).await;
            let (ada, inventory) = player(&db, "ada", true).await;
            let (anvil, inbox) = station(&db, ada, "anvil").await;
            stack_of(&db, ada, inbox, 0, "iron_ingot", &Units::uniform(8, 60)).await;
            install_hammer(&db, ada, inventory, anvil, 1).await;
            world.start(ada, anvil, "iron_axe_head", AutomationMode::Forever).await;
            let mut rx = world.events.subscribe();

            // Its last use makes the head, and breaks it
            assert_eq!(world.finish_craft(anvil).await.crafts, 1);
            assert_eq!(all_in(&db, inbox, "iron_axe_head").await.count(), 1);
            let a = stored(&db, anvil).await;
            assert_eq!((a.done, &a.state), (1, &paused(PauseReason::ToolBroken)));
            assert_eq!(pause_notices(&db, ada).await, vec![PauseReason::ToolBroken]);
            let mut heard = (false, false);
            while let Ok(e) = rx.try_recv() {
                match e {
                    GameEvent::AutomationPaused { reason: PauseReason::ToolBroken, owner_id, .. } => heard.0 = owner_id == ada,
                    GameEvent::ToolBroke { .. } => heard.1 = true,
                    _ => {}
                }
            }
            assert_eq!(heard, (true, true), "the owner hears the pause, and the tool break");
            assert_eq!(world.cycle().await, Cycle::default(), "no tool, no craft");

            install_hammer(&db, ada, inventory, anvil, 5).await;
            let again = world.cycle().await;
            assert_eq!((again.crafts, again.resumed), (1, 1), "a new hammer and it goes on");

            crate::engine::station_tools::remove_tool(&db, ada, anvil, HERE).await.unwrap();
            assert_eq!(world.finish_craft(anvil).await.paused, 1);
            assert_eq!(stored(&db, anvil).await.state, paused(PauseReason::ToolMissing));
            assert_eq!(all_in(&db, inbox, "iron_ingot").await.count(), 4, "two crafts' worth used, no more");
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_free_players_stations_wait_for_them_and_a_subscribers_dont(db: PgPool) {
            let world = World::new(db.clone()).await;
            let (ada, _) = player(&db, "ada", false).await;
            let (bram, _) = player(&db, "bram", true).await;
            let (ada_bench, ada_box) = station(&db, ada, "workbench").await;
            let (bram_bench, bram_box) = station(&db, bram, "workbench").await;
            load_handles(&db, ada, ada_box).await;
            load_handles(&db, bram, bram_box).await;
            world.start(ada, ada_bench, "tool_handle", AutomationMode::Forever).await;
            world.start(bram, bram_bench, "tool_handle", AutomationMode::Forever).await;

            // Neither is online
            assert_eq!(world.cycle().await.paused, 1);
            assert_eq!(stored(&db, ada_bench).await.state, paused(PauseReason::OwnerOffline));
            assert_eq!(stored(&db, bram_bench).await.state, AutomationState::Running);
            assert_eq!(pause_notices(&db, ada).await, vec![PauseReason::OwnerOffline], "waiting for her return");
            for _ in 0..5 {
                world.cycle().await;
            }
            assert_eq!(stored(&db, bram_bench).await.done, 1, "the subscriber's made one while away");
            assert_eq!(stored(&db, ada_bench).await.done, 0);

            // Ada comes back: hers goes on where it was
            world.game.write().await.add_player(someone(ada));
            assert_eq!(world.cycle().await.resumed, 1);
            assert_eq!(stored(&db, ada_bench).await.state, AutomationState::Running);
            for _ in 0..6 {
                world.cycle().await;
            }
            assert_eq!(stored(&db, ada_bench).await.done, 1);

            // And leaves again
            world.game.write().await.remove_player(ada);
            world.cycle().await;
            assert_eq!(stored(&db, ada_bench).await.state, paused(PauseReason::OwnerOffline));

            // A subscription that's run out is a free player's
            sqlx::query(
                "UPDATE accounts SET subscription_until = NOW() - INTERVAL '1 minute'
                 WHERE id = (SELECT account_id FROM players WHERE id = $1)",
            )
            .bind(bram)
            .execute(&db)
            .await
            .unwrap();
            world.cycle().await;
            assert_eq!(stored(&db, bram_bench).await.state, paused(PauseReason::OwnerOffline));
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_restart_resumes_where_it_was(db: PgPool) {
            let world = World::new(db.clone()).await;
            let (ada, _) = player(&db, "ada", true).await;
            let (bench, inbox) = station(&db, ada, "workbench").await;
            let (idle, _) = station(&db, ada, "workbench").await;
            let (empty, _) = station(&db, ada, "workbench").await;
            load_handles(&db, ada, inbox).await;
            world.start(ada, bench, "tool_handle", AutomationMode::Count(5)).await;
            world.start(ada, empty, "tool_handle", AutomationMode::Forever).await;
            automation::configure(&db, &world.recipes, ada, idle, "tool_handle", AutomationMode::Forever).await.unwrap();
            world.finish_craft(bench).await;
            world.finish_craft(empty).await;
            assert_eq!(stored(&db, bench).await.done, 1);
            assert!(matches!(stored(&db, empty).await.state, AutomationState::Paused { .. }));

            // A new server: what it loads is what was running or paused
            let reloaded = Works::load(&db).await.unwrap();
            let mut active = reloaded.active();
            active.sort_by_key(|(id, _)| *id != bench);
            assert_eq!(active.len(), 2, "the stopped one isn't loaded");
            assert_eq!(active[0], (bench, stored(&db, bench).await));
            assert_eq!(active[1], (empty, stored(&db, empty).await));

            world.game.write().await.works = std::sync::Arc::new(reloaded);
            for _ in 0..6 {
                world.cycle().await;
            }
            let a = stored(&db, bench).await;
            assert_eq!((a.done, &a.state), (2, &AutomationState::Running), "it counts on from 1");
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_stop_or_a_removal_mid_run_wins(db: PgPool) {
            let world = World::new(db.clone()).await;
            let (ada, _) = player(&db, "ada", true).await;
            let (bench, inbox) = station(&db, ada, "workbench").await;
            load_handles(&db, ada, inbox).await;
            world.start(ada, bench, "tool_handle", AutomationMode::Forever).await;

            // Stopped in the DB behind the runner's back: the craft isn't made
            automation::stop(&db, ada, bench).await.unwrap();
            assert_eq!(world.finish_craft(bench).await.crafts, 0);
            assert!(all_in(&db, inbox, "tool_handle").await.is_empty());
            assert!(world.works().await.active().is_empty(), "it learnt the station's stopped");

            world.start(ada, bench, "tool_handle", AutomationMode::Forever).await;
            sqlx::query("DELETE FROM items WHERE container_id = $1").bind(inbox).execute(&db).await.unwrap();
            sqlx::query("DELETE FROM containers WHERE station_id = $1").bind(bench).execute(&db).await.unwrap();
            sqlx::query("DELETE FROM stations WHERE id = $1").bind(bench).execute(&db).await.unwrap();
            assert_eq!(world.cycle().await, Cycle::default());
            assert!(world.works().await.active().is_empty(), "a station that's gone is forgotten");
        }

        /// The acceptance's budget: 500 stations running. Prints the time of a
        /// cycle where none completes (most of them), and of one where all do.
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn five_hundred_stations_in_a_cycle(db: PgPool) {
            let world = World::new(db.clone()).await;
            let (ada, _) = player(&db, "ada", true).await;
            let mut benches = Vec::new();
            for _ in 0..500 {
                let (bench, inbox) = station(&db, ada, "workbench").await;
                load_handles(&db, ada, inbox).await;
                world.start(ada, bench, "tool_handle", AutomationMode::Forever).await;
                benches.push(bench);
            }
            let started = std::time::Instant::now();
            assert_eq!(world.cycle().await, Cycle::default());
            let quiet = started.elapsed();

            let works = world.works().await;
            for bench in &benches {
                works.runs().get_mut(bench).unwrap().progress = i32::MAX / 2;
            }
            let started = std::time::Instant::now();
            assert_eq!(world.cycle().await.crafts, 500);
            let busy = started.elapsed();
            let tallied: i64 = sqlx::query_scalar("SELECT quantity FROM production_tally WHERE player_id = $1")
                .bind(ada)
                .fetch_one(&db)
                .await
                .unwrap();
            assert_eq!(tallied, 500, "one owner's 500 handles, in one row");
            println!("500 stations: a cycle with no craft due {quiet:?}, with all 500 completing {busy:?}");
            assert!(quiet < std::time::Duration::from_millis(100), "the usual cycle took {quiet:?}");
        }
    }
}
