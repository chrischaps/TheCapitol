//! Writing a slot move to the database: a move into an empty slot, a merge
//! onto a stack of the same type, or a swap with a stack of another type (or
//! with one of the same type that doesn't stack, such as a tool or a kit).
//! REST `/inventory/move` and WS `move_item` both check access, bounds and
//! ownership first, then call [`apply_move`].
//!
//! Each runs in one transaction, so a failure part way leaves nothing half
//! done. A swap parks the source (`slot_index = NULL`) before moving the
//! target into its place: `idx_items_container_slot` is a unique index on
//! `(container_id, slot_index)` that skips NULL slots, and checks each row as
//! it's written, so two straight UPDATEs collide on the first one (#81).
//!
//! A player's own merge blends any grades (plan D2). The merges the server
//! makes unasked go through [`auto_stack`], which only finds a stack of the
//! same grade and min grade, and [`write_lot`]. [`split`] is the other way.

use capitol_protocol::rules::stacking::{auto_stacks_with, Lot};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::models::Item;

/// What a move did.
#[derive(Debug, PartialEq)]
pub enum SlotMove {
    Moved,
    Merged { quantity: i32, quality: i32, min_quality: i32 },
    Swapped,
}

/// Put `source` into `(target_container_id, target_slot)`, which holds
/// `target` or nothing. `mover` becomes the source's owner: that's a no-op for
/// their own items, and how a station owner takes a deposit from their box
/// (`container_access::movable_item`, #80). A merge folds the source into the
/// target, which the callers only allow when the mover owns the target.
pub async fn apply_move(
    db: &PgPool,
    mover: Uuid,
    source: &Item,
    target_container_id: Uuid,
    target_slot: i32,
    target: Option<&Item>,
) -> Result<SlotMove, sqlx::Error> {
    let mut tx = db.begin().await?;
    let stacks = match target {
        Some(target) if target.item_type == source.item_type => {
            sqlx::query_scalar::<_, bool>("SELECT stackable FROM item_types WHERE id = $1")
                .bind(&source.item_type)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or(true)
        }
        _ => false,
    };
    let done = match target {
        None => {
            sqlx::query(
                "UPDATE items SET container_id = $1, slot_index = $2, owner_id = $4 WHERE id = $3",
            )
            .bind(target_container_id)
            .bind(target_slot)
            .bind(source.id)
            .bind(mover)
            .execute(&mut *tx)
            .await?;
            SlotMove::Moved
        }
        Some(target) if stacks => {
            // Any grades blend: it's the player's own drop (D2)
            let merged = target
                .lot()
                .merge(source.lot())
                .ok_or_else(|| sqlx::Error::Protocol("Too many for one stack".into()))?;
            write_lot(&mut tx, target.id, &merged).await?;
            sqlx::query("DELETE FROM items WHERE id = $1")
                .bind(source.id)
                .execute(&mut *tx)
                .await?;
            SlotMove::Merged {
                quantity: merged.quantity,
                quality: merged.quality,
                min_quality: merged.min_quality,
            }
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
            sqlx::query(
                "UPDATE items SET container_id = $1, slot_index = $2, owner_id = $4 WHERE id = $3",
            )
            .bind(target_container_id)
            .bind(target_slot)
            .bind(source.id)
            .bind(mover)
            .execute(&mut *tx)
            .await?;
            SlotMove::Swapped
        }
    };
    tx.commit().await?;
    Ok(done)
}

/// The first stack of `item_type` in `container_id`, in slot order, that `lot`
/// may join without its owner asking (`rules::stacking::auto_stacks_with`):
/// the same grade and the same min grade. It's locked for the caller's
/// transaction. Craft output, extraction, station returns and the exchange
/// withdraw all look here, and take a slot of their own when it's `None`.
pub async fn auto_stack(
    conn: &mut PgConnection,
    container_id: Uuid,
    item_type: &str,
    lot: &Lot,
) -> Result<Option<(Uuid, Lot)>, sqlx::Error> {
    let stacks: Vec<(Uuid, i32, i32, i32)> = sqlx::query_as(
        "SELECT id, quantity, quality, min_quality FROM items
         WHERE container_id = $1 AND item_type = $2
         ORDER BY slot_index NULLS LAST, id
         FOR UPDATE",
    )
    .bind(container_id)
    .bind(item_type)
    .fetch_all(&mut *conn)
    .await?;
    Ok(stacks
        .into_iter()
        .map(|(id, quantity, quality, min_quality)| (id, Lot { quantity, quality, min_quality }))
        .find(|(_, stack)| auto_stacks_with(item_type, stack, item_type, lot)))
}

/// Write `lot` to the stack `id`: its quantity, quality and min.
pub async fn write_lot(conn: &mut PgConnection, id: Uuid, lot: &Lot) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE items SET quantity = $1, quality = $2, min_quality = $3 WHERE id = $4")
        .bind(lot.quantity)
        .bind(lot.quality)
        .bind(lot.min_quality)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Why a split was refused.
#[derive(Debug, PartialEq)]
pub enum SplitError {
    /// Not 1 to one less than the stack
    BadQuantity,
    /// The stack isn't in a container
    NotInAContainer,
    /// Outside the container
    BadSlot,
    /// Something is already there
    SlotTaken,
}

impl SplitError {
    pub fn message(&self) -> &'static str {
        match self {
            SplitError::BadQuantity => "Split off at least 1, and leave at least 1",
            SplitError::NotInAContainer => "That item isn't in a container",
            SplitError::BadSlot => "That slot doesn't exist",
            SplitError::SlotTaken => "That slot isn't empty",
        }
    }
}

/// Split `quantity` off `source` into the empty slot `to_slot` of its own
/// container, as a new stack the mover owns. Both stacks keep the quality and
/// the min: a blend stays a blend (D2). Returns the new stack. The callers
/// check the mover may use the source first, as for a move.
pub async fn split(
    db: &PgPool,
    mover: Uuid,
    source: &Item,
    quantity: i32,
    to_slot: i32,
) -> Result<Result<Item, SplitError>, sqlx::Error> {
    let Some(container_id) = source.container_id else {
        return Ok(Err(SplitError::NotInAContainer));
    };
    let mut tx = db.begin().await?;
    // The stack again, locked, in case it changed since the caller read it
    let current: Option<Item> = sqlx::query_as(&format!(
        "SELECT {} FROM items WHERE id = $1 FOR UPDATE",
        crate::models::ITEM_COLUMNS
    ))
    .bind(source.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(current) = current.filter(|c| c.container_id == Some(container_id)) else {
        return Ok(Err(SplitError::NotInAContainer));
    };
    if quantity < 1 || quantity >= current.quantity {
        return Ok(Err(SplitError::BadQuantity));
    }
    let slot_count: i32 = sqlx::query_scalar(
        "SELECT ct.slot_count FROM containers c
         JOIN container_types ct ON ct.id = c.container_type WHERE c.id = $1",
    )
    .bind(container_id)
    .fetch_one(&mut *tx)
    .await?;
    if to_slot < 0 || to_slot >= slot_count {
        return Ok(Err(SplitError::BadSlot));
    }
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM items WHERE container_id = $1 AND slot_index = $2)",
    )
    .bind(container_id)
    .bind(to_slot)
    .fetch_one(&mut *tx)
    .await?;
    if taken {
        return Ok(Err(SplitError::SlotTaken));
    }

    let (taken, left) = current.lot().split(quantity);
    write_lot(&mut tx, current.id, &left).await?;
    let new: Item = sqlx::query_as(&format!(
        "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, min_quality)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING {}",
        crate::models::ITEM_COLUMNS
    ))
    .bind(Uuid::new_v4())
    .bind(&current.item_type)
    .bind(taken.quality)
    .bind(taken.quantity)
    .bind(mover)
    .bind(container_id)
    .bind(to_slot)
    .bind(taken.min_quality)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Ok(new))
}

/// Stacks for the tests of every path that merges (D2)
#[cfg(test)]
pub mod test_stacks {
    use sqlx::PgPool;
    use uuid::Uuid;

    /// A stack of `qty` `item_type` at `quality`, blended down to `min_quality`
    #[allow(clippy::too_many_arguments)]
    pub async fn lot_in(
        db: &PgPool,
        owner: Uuid,
        container: Uuid,
        slot: i32,
        item_type: &str,
        quality: i32,
        min_quality: i32,
        qty: i32,
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, min_quality)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id)
        .bind(item_type)
        .bind(quality)
        .bind(qty)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .bind(min_quality)
        .execute(db)
        .await
        .unwrap();
        id
    }

    /// Each `item_type` stack in `container` as (slot, quantity, quality, min), in slot order
    pub async fn stacks_in(db: &PgPool, container: Uuid, item_type: &str) -> Vec<(i32, i32, i32, i32)> {
        sqlx::query_as(
            "SELECT slot_index, quantity, quality, min_quality FROM items
             WHERE container_id = $1 AND item_type = $2 ORDER BY slot_index",
        )
        .bind(container)
        .bind(item_type)
        .fetch_all(db)
        .await
        .unwrap()
    }
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
        player_named(db, "ada").await
    }

    async fn player_named(db: &PgPool, name: &str) -> Player {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(account)
            .bind(format!("{name}@test"))
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
             RETURNING id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at, durability, min_quality",
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

        let done = apply_move(&db, ada.id, &fiber, ada.inventory, 5, Some(&rope)).await.unwrap();

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

        let done = apply_move(&db, ada.id, &fiber, ada.input, 0, Some(&rope)).await.unwrap();

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
        let err = apply_move(&db, ada.id, &fiber, Uuid::new_v4(), 5, Some(&rope)).await;

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

        let done = apply_move(&db, ada.id, &a, ada.inventory, 1, Some(&b)).await.unwrap();

        assert_eq!(done, SlotMove::Merged { quantity: 4, quality: 60, min_quality: 60 });
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE owner_id = $1")
            .bind(ada.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left, 1);
    }

    /// Kits and tools don't stack: dropping one on another of its kind swaps them
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn two_of_an_unstackable_kind_swap_rather_than_merge(db: PgPool) {
        let ada = player(&db).await;
        let a = stack(&db, ada.id, ada.inventory, 0, "workbench_kit", 1).await;
        let b = stack(&db, ada.id, ada.inventory, 1, "workbench_kit", 1).await;

        let done = apply_move(&db, ada.id, &a, ada.inventory, 1, Some(&b)).await.unwrap();

        assert_eq!(done, SlotMove::Swapped);
        assert_eq!(place(&db, a.id).await, (ada.inventory, 1));
        assert_eq!(place(&db, b.id).await, (ada.inventory, 0));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_move_into_an_empty_slot(db: PgPool) {
        let ada = player(&db).await;
        let fiber = stack(&db, ada.id, ada.inventory, 0, "fiber", 3).await;

        let done = apply_move(&db, ada.id, &fiber, ada.input, 3, None).await.unwrap();

        assert_eq!(done, SlotMove::Moved);
        assert_eq!(place(&db, fiber.id).await, (ada.input, 3));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_mover_becomes_the_owner(db: PgPool) {
        // A station owner taking a deposit out of their box keeps it (#80).
        let ada = player_named(&db, "ada").await;
        let bob = player_named(&db, "bob").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 3).await;

        apply_move(&db, bob.id, &fiber, bob.inventory, 4, None).await.unwrap();

        assert_eq!(place(&db, fiber.id).await, (bob.inventory, 4));
        let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_id FROM items WHERE id = $1")
            .bind(fiber.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(owner, Some(bob.id));
    }

    /// A player's own drop blends any grades: the mean by quantity, and the lower min (D2)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_manual_merge_blends_any_grades_and_keeps_the_lower_min(db: PgPool) {
        use super::test_stacks::{lot_in, stacks_in};
        let ada = player(&db).await;
        let a = lot_in(&db, ada.id, ada.inventory, 0, "fiber", 96, 92, 3).await;
        let d = lot_in(&db, ada.id, ada.inventory, 1, "fiber", 45, 30, 1).await;
        let a = item(&db, a).await;
        let d = item(&db, d).await;

        let done = apply_move(&db, ada.id, &d, ada.inventory, 0, Some(&a)).await.unwrap();

        // (96×3 + 45) / 4 = 83.25, rounded down
        assert_eq!(done, SlotMove::Merged { quantity: 4, quality: 83, min_quality: 30 });
        assert_eq!(stacks_in(&db, ada.inventory, "fiber").await, vec![(0, 4, 83, 30)]);
    }

    async fn item(db: &PgPool, id: Uuid) -> Item {
        sqlx::query_as(&format!("SELECT {} FROM items WHERE id = $1", crate::models::ITEM_COLUMNS))
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// A split keeps both values on both stacks: a blend stays a blend
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_split_keeps_quality_and_min_on_both_stacks(db: PgPool) {
        use super::test_stacks::{lot_in, stacks_in};
        let ada = player(&db).await;
        let blend = lot_in(&db, ada.id, ada.inventory, 0, "fiber", 91, 40, 10).await;
        let blend = item(&db, blend).await;

        let new = split(&db, ada.id, &blend, 4, 5).await.unwrap().unwrap();

        assert_eq!((new.quantity, new.quality, new.min_quality, new.slot_index), (4, 91, 40, Some(5)));
        assert_eq!(new.owner_id, Some(ada.id));
        assert_eq!(stacks_in(&db, ada.inventory, "fiber").await, vec![(0, 6, 91, 40), (5, 4, 91, 40)]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_split_is_refused_when_it_cant_be_made(db: PgPool) {
        use super::test_stacks::{lot_in, stacks_in};
        let ada = player(&db).await;
        let fiber = item(&db, lot_in(&db, ada.id, ada.inventory, 0, "fiber", 60, 60, 5).await).await;
        stack(&db, ada.id, ada.inventory, 1, "rope", 1).await;

        let refused = |q, slot| split(&db, ada.id, &fiber, q, slot);
        assert_eq!(refused(0, 2).await.unwrap().unwrap_err(), SplitError::BadQuantity);
        assert_eq!(refused(5, 2).await.unwrap().unwrap_err(), SplitError::BadQuantity);
        assert_eq!(refused(2, 1).await.unwrap().unwrap_err(), SplitError::SlotTaken);
        assert_eq!(refused(2, -1).await.unwrap().unwrap_err(), SplitError::BadSlot);
        assert_eq!(refused(2, 10_000).await.unwrap().unwrap_err(), SplitError::BadSlot);
        assert_eq!(stacks_in(&db, ada.inventory, "fiber").await, vec![(0, 5, 60, 60)]);
    }
}
