//! Mutes (Commons S1.4, plan D2): anyone can stop hearing someone.
//!
//! A mute is the player's, in `player_mutes`, so it holds on every device
//! and across a restart. Each socket loads its player's mutes ([`of`]) when
//! it subscribes, and again on [`GameEvent::MutesChanged`], which [`set`]
//! sends. A line from someone muted is dropped before it's sent
//! (`talk::hears`), the Capitol backlog's included, so it never reaches
//! the muter at all. A letter from them (S1.5) is taken and paid for as
//! any other, but only into the writer's out box, so the writer can't tell. Citizens can be muted like anyone.

use std::collections::HashSet;

use capitol_protocol::rest::MutedPlayer;
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::engine::GameEvent;
use crate::error::AppError;

/// Why you can't mute yourself
pub const NOT_YOURSELF: &str = "You can't mute yourself";

/// The ids `player` has muted.
pub async fn of(db: &PgPool, player: Uuid) -> Result<HashSet<Uuid>, sqlx::Error> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT muted_id FROM player_mutes WHERE player_id = $1")
        .bind(player)
        .fetch_all(db)
        .await?;
    Ok(ids.into_iter().collect())
}

/// The people `player` has muted, newest first.
pub async fn list(db: &PgPool, player: Uuid) -> Result<Vec<MutedPlayer>, sqlx::Error> {
    let rows: Vec<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT p.id, p.name, a.kind = 'citizen'
         FROM player_mutes m
         JOIN players p ON p.id = m.muted_id
         JOIN accounts a ON a.id = p.account_id
         WHERE m.player_id = $1
         ORDER BY m.created_at DESC, p.name",
    )
    .bind(player)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|(id, name, citizen)| MutedPlayer { id, name, citizen }).collect())
}

/// `player` mutes `other` (`on`), or stops (`!on`), then tells the
/// player's sockets. Muting twice, or unmuting someone not muted, changes
/// nothing and is no error. `other` must be someone, and not `player`.
pub async fn set(
    db: &PgPool,
    events: &broadcast::Sender<GameEvent>,
    player: Uuid,
    other: Uuid,
    on: bool,
) -> Result<(), AppError> {
    if on {
        if other == player {
            return Err(AppError::BadRequest(NOT_YOURSELF.into()));
        }
        let muted = sqlx::query(
            "INSERT INTO player_mutes (player_id, muted_id)
             SELECT $1, id FROM players WHERE id = $2
             ON CONFLICT DO NOTHING",
        )
        .bind(player)
        .bind(other)
        .execute(db)
        .await?
        .rows_affected();
        if muted == 0 {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM players WHERE id = $1)")
                .bind(other)
                .fetch_one(db)
                .await?;
            if !exists {
                return Err(AppError::NotFound("No one goes by that".into()));
            }
        }
    } else {
        sqlx::query("DELETE FROM player_mutes WHERE player_id = $1 AND muted_id = $2")
            .bind(player)
            .bind(other)
            .execute(db)
            .await?;
    }
    let _ = events.send(GameEvent::MutesChanged { player_id: player });
    Ok(())
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test mute -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::player;

    fn changed(rx: &mut broadcast::Receiver<GameEvent>) -> Vec<Uuid> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|e| match e {
                GameEvent::MutesChanged { player_id } => Some(player_id),
                _ => None,
            })
            .collect()
    }

    /// A restart is a fresh read of the table, and a second session reads
    /// it too, when the change reaches it
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_mute_is_kept_and_tells_the_muters_sockets(db: PgPool) {
        let (ada, bram, cass) = (player(&db, "Ada", 0).await, player(&db, "Bram", 0).await, player(&db, "Cass", 0).await);
        let (events, mut rx) = broadcast::channel(16);
        set(&db, &events, ada, bram, true).await.unwrap();
        set(&db, &events, ada, cass, true).await.unwrap();
        assert_eq!(changed(&mut rx), [ada, ada]);
        assert_eq!(of(&db, ada).await.unwrap(), HashSet::from([bram, cass]));
        assert!(of(&db, bram).await.unwrap().is_empty(), "a mute is one way");
        let names: Vec<String> = list(&db, ada).await.unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(names.len(), 2);

        // Twice is once, and unmuting someone not muted is no error
        set(&db, &events, ada, bram, true).await.unwrap();
        set(&db, &events, ada, bram, false).await.unwrap();
        set(&db, &events, ada, bram, false).await.unwrap();
        assert_eq!(of(&db, ada).await.unwrap(), HashSet::from([cass]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn you_cant_mute_yourself_or_no_one(db: PgPool) {
        let ada = player(&db, "Ada", 0).await;
        let (events, mut rx) = broadcast::channel(16);
        let err = set(&db, &events, ada, ada, true).await.unwrap_err();
        assert!(matches!(&err, AppError::BadRequest(m) if m == NOT_YOURSELF), "{err:?}");
        let err = set(&db, &events, ada, Uuid::new_v4(), true).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
        assert!(changed(&mut rx).is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_citizen_can_be_muted_and_is_marked_so(db: PgPool) {
        let ada = player(&db, "Ada", 0).await;
        // An account never changes kind, so it's a citizen from the first
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash, kind) VALUES ($1, $2, 'x', 'citizen')")
            .bind(account)
            .bind(format!("{account}@citizens.test"))
            .execute(&db)
            .await
            .unwrap();
        let brin = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, 'Brin')")
            .bind(brin)
            .bind(account)
            .execute(&db)
            .await
            .unwrap();
        let (events, _rx) = broadcast::channel(16);
        set(&db, &events, ada, brin, true).await.unwrap();
        let [muted] = list(&db, ada).await.unwrap().try_into().unwrap();
        assert_eq!((muted.id, muted.name.as_str(), muted.citizen), (brin, "Brin", true));
    }
}
