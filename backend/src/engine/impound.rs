//! The impound (#141): each player's store for what a foreclosure, a forced
//! terrain commit or a lot laid under a workbench took down and their
//! inventory had no room for (`station_removal::return_stations`).
//!
//! It's made the first time it's needed, and holds 480 slots: an estate's
//! 25 stations with full boxes, their kits and their tools. Things only come
//! out of it: `POST /inventory/impound/collect`, from anywhere, or a move to
//! an empty slot
//! ([`container_access::impound_refusal`](super::container_access::impound_refusal)).

use sqlx::PgConnection;
use uuid::Uuid;

/// The impound's container type
pub const IMPOUND: &str = "impound";

/// `player_id`'s impound, made the first time it's needed, and locked for the
/// caller's transaction (`FOR NO KEY UPDATE`, so a foreign-key check on it
/// doesn't wait: see [`market_box`](super::market::market_box), #333).
pub async fn impound(conn: &mut PgConnection, player_id: Uuid) -> Result<Uuid, sqlx::Error> {
    if let Some(id) = held(conn, player_id).await? {
        return Ok(id);
    }
    sqlx::query(
        "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'impound', $2)
         ON CONFLICT (owner_id) WHERE container_type = 'impound' DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(player_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query_scalar("SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'impound' FOR NO KEY UPDATE")
        .bind(player_id)
        .fetch_one(&mut *conn)
        .await
}

/// `player_id`'s impound, locked, if they have one yet.
pub async fn held(conn: &mut PgConnection, player_id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'impound' FOR NO KEY UPDATE")
        .bind(player_id)
        .fetch_optional(&mut *conn)
        .await
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test impound -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::units::test_units::{all_held, stack_of, units};
    use crate::routes::market::collect_from;
    use capitol_protocol::rules::stacking::Units;
    use sqlx::PgPool;

    /// A player and their inventory
    async fn player(db: &PgPool) -> (Uuid, Uuid) {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, 'ada@test', 'x')")
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        let id: Uuid = sqlx::query_scalar("INSERT INTO players (id, account_id, name) VALUES ($1, $2, 'ada') RETURNING id")
            .bind(Uuid::new_v4())
            .bind(account)
            .fetch_one(db)
            .await
            .unwrap();
        let bag: Uuid = sqlx::query_scalar(
            "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap();
        (id, bag)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn one_impound_a_player_and_a_collect_empties_it_into_the_inventory(db: PgPool) {
        let (ada, bag) = player(&db).await;
        let mut conn = db.acquire().await.unwrap();
        let pound = impound(&mut conn, ada).await.unwrap();
        assert_eq!(impound(&mut conn, ada).await.unwrap(), pound, "made once");
        drop(conn);
        let held_fiber = stack_of(&db, ada, bag, 0, "fiber", &Units::uniform(5, 70)).await;
        stack_of(&db, ada, pound, 0, "fiber", &Units::from_pairs([(40, 2), (90, 1)])).await;
        let rope = stack_of(&db, ada, pound, 7, "rope", &Units::uniform(3, 60)).await;
        let before = all_held(&db, ada, "fiber").await;

        let mut tx = db.begin().await.unwrap();
        let (done, inventory) = collect_from(&mut tx, ada, pound, "impound", None).await.unwrap();
        tx.commit().await.unwrap();

        assert_eq!(inventory, bag);
        assert_eq!(done.remaining, 0);
        assert_eq!(done.collected.len(), 2);
        // The fiber joins the stack already there, every unit keeping its quality
        assert_eq!(units(&db, held_fiber).await, Units::from_pairs([(40, 2), (70, 5), (90, 1)]));
        assert_eq!(all_held(&db, ada, "fiber").await, before);
        let rope_at: (Uuid, i32) = sqlx::query_as("SELECT container_id, slot_index FROM items WHERE id = $1")
            .bind(rope)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(rope_at, (bag, 1));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_collect_names_only_what_is_in_the_impound(db: PgPool) {
        let (ada, bag) = player(&db).await;
        let mut conn = db.acquire().await.unwrap();
        let pound = impound(&mut conn, ada).await.unwrap();
        drop(conn);
        let mine = stack_of(&db, ada, bag, 0, "fiber", &Units::uniform(5, 70)).await;

        let mut tx = db.begin().await.unwrap();
        let refused = collect_from(&mut tx, ada, pound, "impound", Some(&[mine])).await;

        assert!(matches!(refused, Err(crate::error::AppError::BadRequest(m)) if m.ends_with("isn't in your impound")));
    }
}
