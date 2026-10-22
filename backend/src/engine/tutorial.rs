//! The road in (#438, `the-capitol-road-in.md`): each player's place on the
//! tutorial, kept in `tutorial_progress` because finishing it earns a deed
//! (#440).
//!
//! Every player has a row: `create_account` makes it, and the migration
//! made one for each player before it. A player with none (one a test
//! inserted straight into `players`) reads as a fresh tour, and [`mark`]
//! and [`set_dismissed`] make the row as they write it.
//!
//! The server judges each step from what the player did (#439, plan D7):
//! one [`saw`] at the point each action succeeds, never where it's asked
//! for, so no client can say "I did it". What counts for which step is
//! [`Did::step`]. The steps only the client can see (`Judge::Client`), and
//! those waived with words (`Judge::Waivable`), also come from the client's
//! `tutorial_seen` (#441, [`Did::Seen`]); it counts for no other step.
//!
//! The step that completes every chapter before the Deeds Office opens the
//! player's deed grant (`deed_grants`, #440).

use capitol_protocol::rules::market::OrderSide;
use capitol_protocol::rules::Zone;
use capitol_protocol::tutorial::{Step, Steps, TutorialProgress};
use capitol_protocol::ws::{Channel, ServerMessage};
use sqlx::PgPool;
use std::collections::HashSet;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::engine::{deed_grants, GameEvent};
use crate::state::AppState;

/// Every step's bits, as stored
fn every() -> i64 {
    Steps::every().bits() as i64
}

fn progress_of(steps: i64, dismissed: bool) -> TutorialProgress {
    TutorialProgress { done: Steps::from_bits(steps as u64), dismissed }
}

/// Where `player` is on the road in.
pub async fn progress(db: &PgPool, player: Uuid) -> Result<TutorialProgress, sqlx::Error> {
    let row: Option<(i64, bool)> =
        sqlx::query_as("SELECT steps, dismissed FROM tutorial_progress WHERE player_id = $1")
            .bind(player)
            .fetch_optional(db)
            .await?;
    Ok(row.map(|(steps, dismissed)| progress_of(steps, dismissed)).unwrap_or_default())
}

/// `player` did `step`. The first time, it's kept, `finished_at` set if it
/// was the last, and the progress after it returned; a step already done
/// returns `None` and writes nothing. Steps done out of order count.
pub async fn mark(db: &PgPool, player: Uuid, step: Step) -> Result<Option<TutorialProgress>, sqlx::Error> {
    let bit = step.bit() as i64;
    let row: Option<(i64, bool)> = sqlx::query_as(
        "INSERT INTO tutorial_progress (player_id, steps, finished_at)
         VALUES ($1, $2, CASE WHEN $2 = $3 THEN NOW() END)
         ON CONFLICT (player_id) DO UPDATE
         SET steps = tutorial_progress.steps | $2,
             finished_at = COALESCE(
                 tutorial_progress.finished_at,
                 CASE WHEN (tutorial_progress.steps | $2) & $3 = $3 THEN NOW() END)
         WHERE tutorial_progress.steps & $2 = 0
         RETURNING steps, dismissed",
    )
    .bind(player)
    .bind(bit)
    .bind(every())
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(steps, dismissed)| progress_of(steps, dismissed)))
}

/// What a player did that the road in may count, said where it succeeded
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Did<'a> {
    /// Ended a walk at least `WALK_DISTANCE` from where it began
    Walked,
    /// Gathered some of this item: a gather written, not a failed pull
    Gathered(&'a str),
    /// Made something by this recipe, by hand or at a station
    Crafted(&'a str),
    /// Put a station down: on their own plot, or off any (only a workbench can be)
    Placed { at_home: bool },
    /// Deposited fiber at the Exchange
    Deposited,
    /// Put an order on the book
    Ordered(OrderSide),
    /// Took something out of their market box
    Collected,
    /// Stood at a bazaar stall and asked for its prices. Range-checked, so it
    /// proves they were there. It counts with nobody renting it, so an empty
    /// bazaar never blocks the road (plan D10).
    LookedInAtStall,
    /// Opened the Gazette
    ReadGazette,
    /// Said a line
    Said(Channel),
    /// Opened someone else's card
    OpenedCard,
    /// Opened a letter in their in box
    ReadLetter,
    /// Looked at a plot in this zone
    SawPlot(Zone),
    /// Set a station's automation job
    SetAutomation,
    /// Claimed a plot
    Claimed,
    /// The client says it showed this step (`tutorial_seen`): only for a
    /// step the client may report (`Judge::client_reports`)
    Seen(Step),
}

impl Did<'_> {
    /// The step this counts for, if any
    pub fn step(self) -> Option<Step> {
        Some(match self {
            Did::Walked => Step::Walk,
            Did::Gathered("fiber") => Step::GatherGrass,
            Did::Gathered("sticks") => Step::GatherBranches,
            Did::Gathered("stone") => Step::GatherStone,
            // Only a tree or old growth gives logs, and both need the axe
            Did::Gathered("log") => Step::FellTree,
            Did::Crafted("rope_basic") => Step::TwistRope,
            Did::Crafted("craft_workbench") => Step::MakeWorkbench,
            Did::Crafted("tool_handle") => Step::MakeHandle,
            Did::Crafted("knapped_stone") => Step::KnapStone,
            Did::Crafted("stone_axe") => Step::MakeAxe,
            Did::Placed { at_home: false } => Step::PlaceWorkbench,
            Did::Placed { at_home: true } => Step::PlaceAtHome,
            Did::Deposited => Step::Deposit,
            Did::Ordered(OrderSide::Sell) => Step::SellOrder,
            Did::Ordered(OrderSide::Buy) => Step::BuyOrder,
            Did::Collected => Step::CollectBox,
            Did::LookedInAtStall => Step::VisitStall,
            Did::ReadGazette => Step::ReadGazette,
            Did::Said(Channel::Local) => Step::SayLocal,
            Did::OpenedCard => Step::OpenCard,
            Did::ReadLetter => Step::ReadLetter,
            Did::SawPlot(Zone::Rural) => Step::SeeRuralLot,
            Did::SawPlot(Zone::Suburban) => Step::SeeSuburbanLot,
            Did::SawPlot(Zone::Urban) => Step::SeeUrbanLot,
            Did::SetAutomation => Step::SetAutomation,
            Did::Claimed => Step::ClaimHome,
            Did::Seen(step) if step.judge().client_reports() => step,
            Did::Gathered(_) | Did::Crafted(_) | Did::Said(_) | Did::SawPlot(_) | Did::Seen(_) => return None,
        })
    }
}

/// `player` did `did`, which has succeeded. If it counts for a step not yet
/// done, the step is kept and each of their sockets told (`tutorial_step`).
/// Out of order counts.
///
/// Call it once the action is written, never before. It's one upsert on the
/// player's row when `did` counts for a step, and nothing when it doesn't. A
/// failure is logged and goes no further: the action stands either way.
pub async fn saw(db: &PgPool, events: &broadcast::Sender<GameEvent>, player: Uuid, did: Did<'_>) {
    let Some(step) = did.step() else { return };
    let after = match mark(db, player, step).await {
        Ok(Some(after)) => after,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!("Couldn't keep {player}'s {} on the road in: {e}", step.name());
            return;
        }
    };
    let _ = events.send(GameEvent::Tutorial {
        player_id: player,
        message: ServerMessage::TutorialStep { step, done: after.done },
    });
    if after.done.deed_earned() && !Steps::from_bits(after.done.bits() & !step.bit()).deed_earned() {
        open_grant(db, player).await;
    }
}

/// `player` has walked the road in: open their deed grant (#440). If this
/// fails, the claim opens it from the steps instead (`deed_grants::take_in`).
async fn open_grant(db: &PgPool, player: Uuid) {
    let opened = match db.acquire().await {
        Ok(mut conn) => deed_grants::open(&mut conn, player).await,
        Err(e) => Err(e),
    };
    match opened {
        Ok(true) => tracing::info!("{player} has walked the road in and earned their deed"),
        Ok(false) => {}
        Err(e) => tracing::warn!("Couldn't open {player}'s deed grant: {e}"),
    }
}

/// [`saw`], from a route that knows the account rather than the player. An
/// account with no player is passed over.
pub async fn saw_by_account(state: &AppState, account_id: Uuid, did: Did<'_>) {
    if did.step().is_none() {
        return;
    }
    let player: Option<Uuid> = match sqlx::query_scalar("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(player) => player,
        Err(e) => {
            tracing::warn!("Couldn't find account {account_id}'s player for the road in: {e}");
            return;
        }
    };
    if let Some(player) = player {
        let events = state.game.read().await.event_sender.clone();
        saw(&state.db, &events, player, did).await;
    }
}

/// The steps a long-lived task has already seen kept, so it asks once a run
/// for each player's: the tick's writer, where citizens gather and walk all
/// day long and every gather of fiber would otherwise be an upsert
#[derive(Debug, Default)]
pub struct Judged(HashSet<(Uuid, Step)>);

impl Judged {
    /// [`saw`], unless this run has already seen `player`'s step
    pub async fn saw(&mut self, db: &PgPool, events: &broadcast::Sender<GameEvent>, player: Uuid, did: Did<'_>) {
        if did.step().is_some_and(|step| self.0.insert((player, step))) {
            saw(db, events, player, did).await;
        }
    }
}

/// Hide the card for now (`dismissed`), or bring it back, then tell each of
/// `player`'s sockets, so it holds on every device (plan D2).
pub async fn set_dismissed(
    db: &PgPool,
    events: &broadcast::Sender<GameEvent>,
    player: Uuid,
    dismissed: bool,
) -> Result<TutorialProgress, sqlx::Error> {
    let (steps, dismissed): (i64, bool) = sqlx::query_as(
        "INSERT INTO tutorial_progress (player_id, dismissed) VALUES ($1, $2)
         ON CONFLICT (player_id) DO UPDATE SET dismissed = $2
         RETURNING steps, dismissed",
    )
    .bind(player)
    .bind(dismissed)
    .fetch_one(db)
    .await?;
    let progress = progress_of(steps, dismissed);
    let _ = events.send(GameEvent::Tutorial { player_id: player, message: message(progress) });
    Ok(progress)
}

/// The `tutorial` message for `progress`
pub fn message(progress: TutorialProgress) -> ServerMessage {
    ServerMessage::Tutorial { done: progress.done, dismissed: progress.dismissed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::player;

    async fn finished_at(db: &PgPool, player: Uuid) -> Option<chrono::DateTime<chrono::Utc>> {
        sqlx::query_scalar("SELECT finished_at FROM tutorial_progress WHERE player_id = $1")
            .bind(player)
            .fetch_optional(db)
            .await
            .unwrap()
            .flatten()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_new_account_starts_the_road_in_from_the_beginning(db: PgPool) {
        let mut tx = db.begin().await.unwrap();
        let (_, ada) = crate::routes::auth::create_account(
            &mut tx,
            "ada@test.local",
            "x",
            "Ada",
            crate::models::AccountKind::Human,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let row: Option<i64> = sqlx::query_scalar("SELECT steps FROM tutorial_progress WHERE player_id = $1")
            .bind(ada)
            .fetch_optional(&db)
            .await
            .unwrap();
        assert_eq!(row, Some(0), "create_account makes the row, empty");
        assert_eq!(progress(&db, ada).await.unwrap(), TutorialProgress::default());
    }

    /// A player with no row, as a test inserts, reads as a fresh tour and
    /// gets one when a step is marked
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_step_is_kept_once_and_out_of_order_counts(db: PgPool) {
        let ada = player(&db, "Ada", 0).await;
        assert_eq!(progress(&db, ada).await.unwrap(), TutorialProgress::default());

        let after = mark(&db, ada, Step::Deposit).await.unwrap().expect("first time");
        assert_eq!(after.done, Steps::from_iter([Step::Deposit]));
        assert_eq!(mark(&db, ada, Step::Deposit).await.unwrap(), None, "twice is once");
        mark(&db, ada, Step::Walk).await.unwrap().expect("first time");
        let now = progress(&db, ada).await.unwrap();
        assert_eq!(now.done, Steps::from_iter([Step::Walk, Step::Deposit]));
        assert_eq!(now.done.current(), Some(Step::GatherGrass));
        assert_eq!(finished_at(&db, ada).await, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_last_step_finishes_the_tour(db: PgPool) {
        let ada = player(&db, "Ada", 0).await;
        let (last, rest) = Step::ALL.split_last().unwrap();
        for step in rest {
            mark(&db, ada, *step).await.unwrap();
        }
        assert_eq!(finished_at(&db, ada).await, None);
        let after = mark(&db, ada, *last).await.unwrap().unwrap();
        assert!(after.finished());
        let first = finished_at(&db, ada).await.expect("finished");
        // A step marked again (none is left) doesn't move it
        assert_eq!(mark(&db, ada, *last).await.unwrap(), None);
        assert_eq!(finished_at(&db, ada).await, Some(first));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn hiding_the_card_keeps_the_steps_and_tells_the_sockets(db: PgPool) {
        let (ada, bram) = (player(&db, "Ada", 0).await, player(&db, "Bram", 0).await);
        mark(&db, ada, Step::Walk).await.unwrap();
        let (events, mut rx) = broadcast::channel(16);
        let hidden = set_dismissed(&db, &events, ada, true).await.unwrap();
        assert!(hidden.dismissed);
        assert_eq!(hidden.done, Steps::from_iter([Step::Walk]));
        assert_eq!(progress(&db, ada).await.unwrap(), hidden);
        match rx.try_recv().unwrap() {
            GameEvent::Tutorial { player_id, message } => {
                assert_eq!(player_id, ada);
                assert_eq!(message, super::message(hidden));
            }
            _ => panic!("not the tutorial"),
        }
        // Brought back; and a player with no row yet gets one
        assert!(!set_dismissed(&db, &events, ada, false).await.unwrap().dismissed);
        assert!(set_dismissed(&db, &events, bram, true).await.unwrap().dismissed);
    }

    /// The migration gave every player before it a row, empty
    #[sqlx::test(migrations = false)]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_migration_starts_everyone_on_the_road_in(db: PgPool) {
        const TUTORIAL_PROGRESS: i64 = 20261217;
        let all = sqlx::migrate!("./migrations");
        let mut before = sqlx::migrate!("./migrations");
        before.migrations = all.iter().filter(|m| m.version < TUTORIAL_PROGRESS).cloned().collect::<Vec<_>>().into();
        before.run(&db).await.unwrap();
        let (ada, cass) = (player(&db, "Ada", 0).await, player(&db, "Cass", 0).await);
        all.run(&db).await.unwrap();
        let rows: Vec<(Uuid, i64, bool)> =
            sqlx::query_as("SELECT player_id, steps, dismissed FROM tutorial_progress ORDER BY player_id")
                .fetch_all(&db)
                .await
                .unwrap();
        let mut want = vec![(ada, 0, false), (cass, 0, false)];
        want.sort();
        assert_eq!(rows, want);
    }

    // What the server judges (#439): one test a step, each through the route
    // or the engine function the step is judged in. The gathers, crafts and
    // walks are the tick's writer's, in `tick::tests`; the claim is
    // `routes::plots`'s and the automation job `automation`'s. A local line
    // is judged in the socket's handler, which has no test harness (#463), so
    // only its `Did` is here.

    use axum::extract::{Path, Query, State};
    use axum::{Extension, Json};
    use capitol_protocol::rest::{CollectRequest, DepositRequest, GazetteQuery, PlaceOrder};
    use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};
    use capitol_protocol::tutorial::Judge;

    use crate::middleware::auth::AuthUser;
    use crate::models::LandmarkKind;
    use crate::state::for_tests;

    async fn done(db: &PgPool, player: Uuid) -> Steps {
        progress(db, player).await.unwrap().done
    }

    /// `player`, signed in
    async fn as_them(db: &PgPool, player: Uuid) -> Extension<AuthUser> {
        let account_id = sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1")
            .bind(player)
            .fetch_one(db)
            .await
            .unwrap();
        Extension(AuthUser { account_id, is_admin: false, key: None })
    }

    /// `player` stands at (`x`, `y`), offline, as the routes' range checks read it
    async fn stand(db: &PgPool, player: Uuid, (x, y): (f64, f64)) {
        sqlx::query("UPDATE players SET position_x = $1, position_y = $2 WHERE id = $3")
            .bind(x)
            .bind(y)
            .bind(player)
            .execute(db)
            .await
            .unwrap();
    }

    async fn container(db: &PgPool, owner: Uuid, kind: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(kind)
            .bind(owner)
            .execute(db)
            .await
            .unwrap();
        id
    }

    async fn stack(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, item_type: &str, quantity: i32) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, $2, 50, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(item_type)
        .bind(quantity)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
        id
    }

    /// Outside the Exchange hall's door, in reach of it
    fn at_the_hall() -> (f64, f64) {
        let door = crate::models::TerrainConfig::get_default()
            .landmark(LandmarkKind::ExchangeHall)
            .expect("the default world has a hall")
            .door;
        (SPAWN_X, door.1)
    }

    /// A plot in `zone` with its box round (`x`, `y`)
    async fn plot_in(db: &PgPool, zone: &str, (x, y): (f64, f64)) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO plots (id, terrain_id, zone_id, grid_x, grid_y, world_x, world_y,
                                bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y, size_category)
             VALUES ($1, $2, $3, 0, 0, $4, $5, $4 - 10, $5 - 10, $4 + 10, $5 + 10, 'medium')",
        )
        .bind(id)
        .bind(format!("test-{id}"))
        .bind(zone)
        .bind(x as f32)
        .bind(y as f32)
        .execute(db)
        .await
        .unwrap();
        id
    }

    #[test]
    fn every_step_the_server_judges_has_one_thing_that_counts_for_it() {
        use capitol_protocol::rules::market::OrderSide;
        let deeds = [
            Did::Walked,
            Did::Gathered("fiber"),
            Did::Gathered("sticks"),
            Did::Gathered("stone"),
            Did::Gathered("log"),
            Did::Crafted("rope_basic"),
            Did::Crafted("craft_workbench"),
            Did::Crafted("tool_handle"),
            Did::Crafted("knapped_stone"),
            Did::Crafted("stone_axe"),
            Did::Placed { at_home: false },
            Did::Placed { at_home: true },
            Did::Deposited,
            Did::Ordered(OrderSide::Sell),
            Did::Ordered(OrderSide::Buy),
            Did::Collected,
            Did::LookedInAtStall,
            Did::ReadGazette,
            Did::Said(Channel::Local),
            Did::OpenedCard,
            Did::ReadLetter,
            Did::SawPlot(Zone::Rural),
            Did::SawPlot(Zone::Suburban),
            Did::SawPlot(Zone::Urban),
            Did::SetAutomation,
            Did::Claimed,
        ];
        let judged: Vec<Step> = deeds.iter().filter_map(|d| d.step()).collect();
        let servers: Vec<Step> = Step::ALL.into_iter().filter(|s| s.judge() != Judge::Client).collect();
        assert_eq!(judged.len(), deeds.len());
        assert_eq!(judged.iter().copied().collect::<Steps>(), servers.iter().copied().collect::<Steps>());
        assert_eq!(judged.len(), servers.len(), "one each");
    }

    #[test]
    fn what_isnt_on_the_road_counts_for_nothing() {
        for did in [
            Did::Gathered("clay"),
            Did::Gathered("iron_ore"),
            Did::Crafted("plank"),
            Did::Said(Channel::Capitol),
            Did::SawPlot(Zone::GuildDistrict),
            Did::SawPlot(Zone::Wilderness),
            // The client can't say it did what the server judges (#441)
            Did::Seen(Step::Walk),
            Did::Seen(Step::ClaimHome),
            Did::Seen(Step::Deposit),
        ] {
            assert_eq!(did.step(), None, "{did:?}");
        }
    }

    /// What only the screen shows, and what's waived with words, the client
    /// reports (`tutorial_seen`, #441)
    #[test]
    fn the_client_reports_only_the_steps_it_may() {
        for step in Step::ALL {
            let reported = Did::Seen(step).step();
            assert_eq!(reported.is_some(), step.judge() != Judge::Server, "{step:?}");
            assert!(reported.is_none_or(|s| s == step));
        }
        assert_eq!(Did::Seen(Step::OpenPack).step(), Some(Step::OpenPack));
        assert_eq!(Did::Seen(Step::CollectBox).step(), Some(Step::CollectBox));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_step_seen_by_the_client_is_kept_and_told(db: PgPool) {
        let ada = player(&db, "Ada", 0).await;
        let (events, mut rx) = broadcast::channel(16);
        saw(&db, &events, ada, Did::Seen(Step::Walk)).await;
        assert_eq!(done(&db, ada).await, Steps::NONE, "the server judges a walk");
        assert!(rx.try_recv().is_err());
        saw(&db, &events, ada, Did::Seen(Step::OpenPack)).await;
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::OpenPack]));
        assert!(matches!(rx.try_recv().unwrap(), GameEvent::Tutorial { player_id, .. } if player_id == ada));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_step_is_told_the_first_time_and_nothing_else_is_written(db: PgPool) {
        let (ada, bram) = (player(&db, "Ada", 0).await, player(&db, "Bram", 0).await);
        let (events, mut rx) = broadcast::channel(16);
        saw(&db, &events, ada, Did::Deposited).await;
        match rx.try_recv().unwrap() {
            GameEvent::Tutorial { player_id, message } => {
                assert_eq!(player_id, ada);
                assert_eq!(message, ServerMessage::TutorialStep { step: Step::Deposit, done: done(&db, ada).await });
            }
            _ => panic!("not the tutorial"),
        }
        saw(&db, &events, ada, Did::Deposited).await;
        assert!(rx.try_recv().is_err(), "once");
        // What counts for no step doesn't make a row
        saw(&db, &events, bram, Did::Gathered("clay")).await;
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tutorial_progress WHERE player_id = $1")
            .bind(bram)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(rows, 0);
        assert!(rx.try_recv().is_err());
    }

    /// The writer's cache asks once a run, so a second sight of a step is free
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_writers_cache_asks_once_a_run(db: PgPool) {
        let ada = player(&db, "Ada", 0).await;
        let (events, _rx) = broadcast::channel(16);
        let mut judged = Judged::default();
        judged.saw(&db, &events, ada, Did::Gathered("fiber")).await;
        assert!(done(&db, ada).await.has(Step::GatherGrass));
        sqlx::query("UPDATE tutorial_progress SET steps = 0 WHERE player_id = $1")
            .bind(ada)
            .execute(&db)
            .await
            .unwrap();
        judged.saw(&db, &events, ada, Did::Gathered("fiber")).await;
        assert_eq!(done(&db, ada).await, Steps::NONE, "not asked again");
        judged.saw(&db, &events, ada, Did::Walked).await;
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::Walk]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn a_workbench_counts_off_a_plot_and_a_station_at_home(db: PgPool) {
        use crate::engine::station_placement::place_station;
        let state = for_tests(db.clone()).await;
        let ada = player(&db, "Ada", 0).await;
        let pack = container(&db, ada, "player_inventory").await;
        let out = stack(&db, ada, pack, 0, "workbench_kit", 1).await;
        let home = stack(&db, ada, pack, 1, "workbench_kit", 1).await;

        stand(&db, ada, (0.0, 0.0)).await;
        place_station(&state, ada, "workbench", 0.0, 0.0, out).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::PlaceWorkbench]));

        let lot = crate::engine::plot_sync::tests::plot(&db, Some(ada), 0).await;
        stand(&db, ada, (8000.0, 6800.0)).await;
        let placed = place_station(&state, ada, "workbench", 8000.0, 6800.0, home).await.unwrap();
        assert_eq!(placed.plot_id, Some(lot));
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::PlaceWorkbench, Step::PlaceAtHome]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn a_deposit_counts_and_a_refused_one_doesnt(db: PgPool) {
        use crate::routes::exchange::deposit_fiber;
        let state = for_tests(db.clone()).await;
        let ada = player(&db, "Ada", 0).await;
        let pack = container(&db, ada, "player_inventory").await;
        let fiber = stack(&db, ada, pack, 0, "fiber", 10).await;

        let none = DepositRequest { item_ids: Vec::new() };
        assert!(deposit_fiber(State(state.clone()), as_them(&db, ada).await, Json(none)).await.is_err());
        assert_eq!(done(&db, ada).await, Steps::NONE);
        let one = DepositRequest { item_ids: vec![fiber] };
        let _ = deposit_fiber(State(state), as_them(&db, ada).await, Json(one)).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::Deposit]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn orders_count_at_the_hall_and_not_from_out_of_range(db: PgPool) {
        use crate::routes::market::place_order;
        let state = for_tests(db.clone()).await;
        let ada = player(&db, "Ada", 100).await;
        let pack = container(&db, ada, "player_inventory").await;
        let rope = stack(&db, ada, pack, 0, "rope", 2).await;
        let order = |json: serde_json::Value| Json(serde_json::from_value::<PlaceOrder>(json).unwrap());
        let sell = serde_json::json!({ "side": "sell", "item_type": "rope", "price": 20, "quantity": 1, "item_id": rope });
        let buy = serde_json::json!({ "side": "buy", "item_type": "rope", "price": 5, "quantity": 1 });

        stand(&db, ada, (SPAWN_X, SPAWN_Y)).await;
        assert!(place_order(State(state.clone()), as_them(&db, ada).await, order(sell.clone())).await.is_err());
        assert_eq!(done(&db, ada).await, Steps::NONE, "refused out of range");

        stand(&db, ada, at_the_hall()).await;
        let _ = place_order(State(state.clone()), as_them(&db, ada).await, order(sell)).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::SellOrder]));
        let _ = place_order(State(state), as_them(&db, ada).await, order(buy)).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::SellOrder, Step::BuyOrder]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn collecting_from_the_market_box_counts_once_something_comes_out(db: PgPool) {
        use crate::routes::market::collect;
        let state = for_tests(db.clone()).await;
        let ada = player(&db, "Ada", 0).await;
        container(&db, ada, "player_inventory").await;
        stand(&db, ada, at_the_hall()).await;
        let all = || Json(CollectRequest { item_ids: None });

        let emptied = collect(State(state.clone()), as_them(&db, ada).await, all()).await.unwrap();
        assert!(emptied.collected.is_empty());
        assert_eq!(done(&db, ada).await, Steps::NONE, "an empty box gave nothing");
        let market_box = crate::engine::market::market_box(&mut db.acquire().await.unwrap(), ada).await.unwrap();
        stack(&db, ada, market_box, 0, "rope", 1).await;
        let _ = collect(State(state), as_them(&db, ada).await, all()).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::CollectBox]));
    }

    /// From the lane, even with nobody renting it (D10); from afar, refused
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn looking_in_at_a_stall_counts_from_its_lane(db: PgPool) {
        use crate::routes::bazaar::get_shop;
        let state = for_tests(db.clone()).await;
        let terrain = crate::models::TerrainConfig::get_default();
        let stall = terrain.landmarks.iter().find(|l| l.id == "bazaar_stall_2").expect("a stall").clone();
        let lane_y = terrain.roads.iter().find(|r| r.id == "bazaar_lane").expect("the lane").points[0].1;
        let ada = player(&db, "Ada", 0).await;

        stand(&db, ada, (SPAWN_X, SPAWN_Y)).await;
        assert!(get_shop(State(state.clone()), as_them(&db, ada).await, Path(stall.id.clone())).await.is_err());
        assert_eq!(done(&db, ada).await, Steps::NONE, "too far");
        stand(&db, ada, (stall.door.0, lane_y)).await;
        let nobody = get_shop(State(state), as_them(&db, ada).await, Path(stall.id.clone())).await;
        assert!(nobody.is_err(), "nobody rents it");
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::VisitStall]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn opening_the_gazette_counts(db: PgPool) {
        use crate::routes::gazette::{by_date, latest};
        let state = for_tests(db.clone()).await;
        let (ada, bram) = (player(&db, "Ada", 0).await, player(&db, "Bram", 0).await);
        let _ = latest(State(state.clone()), as_them(&db, ada).await).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::ReadGazette]));
        let _ = by_date(State(state), as_them(&db, bram).await, Query(GazetteQuery { date: 1 })).await.unwrap();
        assert_eq!(done(&db, bram).await, Steps::from_iter([Step::ReadGazette]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn someone_elses_card_counts_and_your_own_doesnt(db: PgPool) {
        use crate::routes::players::get_card;
        let state = for_tests(db.clone()).await;
        let (ada, bram) = (player(&db, "Ada", 0).await, player(&db, "Bram", 0).await);
        let _ = get_card(State(state.clone()), as_them(&db, ada).await, Path(ada)).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::NONE, "your own");
        let _ = get_card(State(state), as_them(&db, ada).await, Path(bram)).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::OpenCard]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn reading_a_letter_counts_and_one_not_yours_doesnt(db: PgPool) {
        use crate::routes::letters::read;
        let state = for_tests(db.clone()).await;
        let (ada, bram) = (player(&db, "Ada", 0).await, player(&db, "Bram", 10).await);
        let sent = crate::engine::letters::send(&db, bram, ada, "Welcome to the city.", false, chrono::Utc::now())
            .await
            .unwrap();
        assert!(read(State(state.clone()), as_them(&db, bram).await, Path(sent.letter.id)).await.is_err());
        assert_eq!(done(&db, bram).await, Steps::NONE, "the writer can't open it");
        let _ = read(State(state), as_them(&db, ada).await, Path(sent.letter.id)).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::ReadLetter]));
    }

    /// A step a home zone, by id or by where it is; elsewhere, nothing
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn a_plot_looked_at_counts_for_its_zone(db: PgPool) {
        use crate::routes::plots::{get_plot, get_plot_at_position};
        let state = for_tests(db.clone()).await;
        let ada = player(&db, "Ada", 0).await;
        let guild = plot_in(&db, "guild_district", (100.0, 100.0)).await;
        let _ = get_plot(State(state.clone()), as_them(&db, ada).await, Path(guild)).await.unwrap();
        assert_eq!(done(&db, ada).await, Steps::NONE, "the guild district is no home");

        for (zone, at) in [("rural", (200.0, 100.0)), ("suburban", (300.0, 100.0))] {
            let lot = plot_in(&db, zone, at).await;
            let _ = get_plot(State(state.clone()), as_them(&db, ada).await, Path(lot)).await.unwrap();
        }
        assert_eq!(done(&db, ada).await, Steps::from_iter([Step::SeeRuralLot, Step::SeeSuburbanLot]));
        plot_in(&db, "urban", (400.0, 100.0)).await;
        let query = serde_json::from_value(serde_json::json!({ "x": 400.0, "y": 100.0 })).unwrap();
        let found = get_plot_at_position(State(state), as_them(&db, ada).await, Query(query)).await.unwrap();
        assert!(found.0.is_some());
        let home_zones = Steps::from_iter([Step::SeeRuralLot, Step::SeeSuburbanLot, Step::SeeUrbanLot]);
        assert_eq!(done(&db, ada).await, home_zones);
    }
}
