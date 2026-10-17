//! Writing a slot move to the database: a move into an empty slot, a merge
//! onto a stack of the same type, or a swap with a stack of another type.
//! REST `/inventory/move` and WS `move_item` both check access, bounds and
//! ownership first, then call [`apply_move`].
//!
//! Each runs in one transaction, so a failure part way leaves nothing half
//! done. A swap parks the source (`slot_index = NULL`) before moving the
//! target into its place: `idx_items_container_slot` is a unique index on
//! `(container_id, slot_index)` that skips NULL slots, and checks each row as
//! it's written, so two straight UPDATEs collide on the first one (#81).

use sqlx::PgPool;
use uuid::Uuid;

use crate::models::Item;

/// What a move did.
#[derive(Debug, PartialEq)]
pub enum SlotMove {
    Moved,
    Merged { quantity: i32, quality: i32 },
    Swapped,
}

/// Put `source` into `(target_container_id, target_slot)`, which holds
/// `target` or nothing.
pub async fn apply_move(
    db: &PgPool,
    source: &Item,
    target_container_id: Uuid,
    target_slot: i32,
    target: Option<&Item>,
) -> Result<SlotMove, sqlx::Error> {
    let mut tx = db.begin().await?;
    let done = match target {
        None => {
            sqlx::query("UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3")
                .bind(target_container_id)
                .bind(target_slot)
                .bind(source.id)
                .execute(&mut *tx)
                .await?;
            SlotMove::Moved
        }
        Some(target) if target.item_type == source.item_type => {
            let quantity = source.quantity + target.quantity;
            let quality =
                (source.quality * source.quantity + target.quality * target.quantity) / quantity;
            sqlx::query("UPDATE items SET quantity = $1, quality = $2 WHERE id = $3")
                .bind(quantity)
                .bind(quality)
                .bind(target.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM items WHERE id = $1")
                .bind(source.id)
                .execute(&mut *tx)
                .await?;
            SlotMove::Merged { quantity, quality }
        }
        Some(target) => {
            // Park the source, so its slot is free for the target...
            sqlx::query("UPDATE items SET slot_index = NULL WHERE id = $1")
                .bind(source.id)
                .execute(&mut *tx)
                .await?;
            // ...move the target into it...
            sqlx::query("UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3")
                .bind(source.container_id)
                .bind(source.slot_index)
                .bind(target.id)
                .execute(&mut *tx)
                .await?;
            // ...and the source into the target's.
            sqlx::query("UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3")
                .bind(target_container_id)
                .bind(target_slot)
                .bind(source.id)
                .execute(&mut *tx)
                .await?;
            SlotMove::Swapped
        }
    };
    tx.commit().await?;
    Ok(done)
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh database that
/// `sqlx::test` creates from the migrations and drops afterwards. Run them with
/// `cargo test slot_moves -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;

    struct Player {
        inventory: Uuid,
        input: Uuid,
        id: Uuid,
    }

    async fn player(db: &PgPool) -> Player {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, 'ada@test', 'x')")
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, 'ada')")
            .bind(id)
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        let mut containers = Vec::new();
        for kind in ["player_inventory", "crafting_input"] {
            let c = Uuid::new_v4();
            sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, $2, $3)")
                .bind(c)
                .bind(kind)
                .bind(id)
                .execute(db)
                .await
                .unwrap();
            containers.push(c);
        }
        Player { inventory: containers[0], input: containers[1], id }
    }

    async fn stack(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, item_type: &str, qty: i32) -> Item {
        sqlx::query_as(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, $2, 60, $3, $4, $5, $6)
             RETURNING id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at",
        )
        .bind(Uuid::new_v4())
        .bind(item_type)
        .bind(qty)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .fetch_one(db)
        .await
        .unwrap()
    }

    /// (container, slot) of an item
    async fn place(db: &PgPool, id: Uuid) -> (Uuid, i32) {
        sqlx::query_as("SELECT container_id, slot_index FROM items WHERE id = $1")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_swap_within_one_container_trades_slots(db: PgPool) {
        let ada = player(&db).await;
        let fiber = stack(&db, ada.id, ada.inventory, 0, "fiber", 3).await;
        let rope = stack(&db, ada.id, ada.inventory, 5, "rope", 1).await;

        let done = apply_move(&db, &fiber, ada.inventory, 5, Some(&rope)).await.unwrap();

        assert_eq!(done, SlotMove::Swapped);
        assert_eq!(place(&db, fiber.id).await, (ada.inventory, 5));
        assert_eq!(place(&db, rope.id).await, (ada.inventory, 0));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_swap_between_two_containers_trades_slots(db: PgPool) {
        let ada = player(&db).await;
        let fiber = stack(&db, ada.id, ada.inventory, 2, "fiber", 3).await;
        let rope = stack(&db, ada.id, ada.input, 0, "rope", 1).await;

        let done = apply_move(&db, &fiber, ada.input, 0, Some(&rope)).await.unwrap();

        assert_eq!(done, SlotMove::Swapped);
        assert_eq!(place(&db, fiber.id).await, (ada.input, 0));
        assert_eq!(place(&db, rope.id).await, (ada.inventory, 2));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_failed_swap_leaves_both_items_where_they_were(db: PgPool) {
        let ada = player(&db).await;
        let fiber = stack(&db, ada.id, ada.inventory, 0, "fiber", 3).await;
        let rope = stack(&db, ada.id, ada.inventory, 5, "rope", 1).await;

        // The target's last step names a container that doesn't exist, so the
        // foreign key fails after the source is parked and the target moved.
        let err = apply_move(&db, &fiber, Uuid::new_v4(), 5, Some(&rope)).await;

        assert!(err.is_err());
        assert_eq!(place(&db, fiber.id).await, (ada.inventory, 0));
        assert_eq!(place(&db, rope.id).await, (ada.inventory, 5));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_merge_folds_the_source_into_the_target(db: PgPool) {
        let ada = player(&db).await;
        let a = stack(&db, ada.id, ada.inventory, 0, "fiber", 3).await;
        let b = stack(&db, ada.id, ada.inventory, 1, "fiber", 1).await;

        let done = apply_move(&db, &a, ada.inventory, 1, Some(&b)).await.unwrap();

        assert_eq!(done, SlotMove::Merged { quantity: 4, quality: 60 });
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE owner_id = $1")
            .bind(ada.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_move_into_an_empty_slot(db: PgPool) {
        let ada = player(&db).await;
        let fiber = stack(&db, ada.id, ada.inventory, 0, "fiber", 3).await;

        let done = apply_move(&db, &fiber, ada.input, 3, None).await.unwrap();

        assert_eq!(done, SlotMove::Moved);
        assert_eq!(place(&db, fiber.id).await, (ada.input, 3));
    }
}
