//! The road in (#438, `the-capitol-road-in.md`): each player's place on the
//! tutorial, kept in `tutorial_progress` because finishing it earns a deed
//! (#440).
//!
//! Every player has a row: `create_account` makes it, and the migration
//! made one for each player before it. A player with none (one a test
//! inserted straight into `players`) reads as a fresh tour, and [`mark`]
//! and [`set_dismissed`] make the row as they write it. The steps are
//! judged at the point each action succeeds (#439), through [`mark`].

use capitol_protocol::tutorial::{Step, Steps, TutorialProgress};
use capitol_protocol::ws::ServerMessage;
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::engine::GameEvent;

/// Every step's bits, as stored
#[cfg_attr(not(test), allow(dead_code))] // `mark`'s; the server judges steps from #439
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
#[cfg_attr(not(test), allow(dead_code))] // the server judges steps from #439, at each action that succeeds
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
}
