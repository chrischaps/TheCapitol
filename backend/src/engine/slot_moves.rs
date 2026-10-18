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
//! A merge adds the source's units to the target's, and every unit keeps its
//! quality (plan D2, E1.8). [`split`] takes units back out, the ones named or
//! the lowest first.

use capitol_protocol::rules::stacking::{Pick, Units};
use sqlx::PgPool;
use uuid::Uuid;

use super::units::{self, NewStack};
use crate::models::Item;

/// What a move did.
#[derive(Debug, PartialEq)]
pub enum SlotMove {
    Moved,
    /// The target holds these units now
    Merged { units: Units },
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
        Some(target) if stacks => SlotMove::Merged {
            units: units::merge_into(&mut tx, source.id, target.id).await?,
        },
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

/// Why a split was refused.
#[derive(Debug, PartialEq)]
pub enum SplitError {
    /// Not 1 to one less than the stack, or named units that don't add up to it
    BadQuantity,
    /// Named units the stack doesn't hold
    NotHeld,
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
            SplitError::NotHeld => "That stack doesn't hold those qualities",
            SplitError::NotInAContainer => "That item isn't in a container",
            SplitError::BadSlot => "That slot doesn't exist",
            SplitError::SlotTaken => "That slot isn't empty",
        }
    }
}

/// Split `quantity` off `source` into the empty slot `to_slot` of its own
/// container, as a new stack the mover owns. `qualities` names the units,
/// adding up to `quantity`; without it the lowest go (`Pick::default`). Every
/// unit keeps its quality (D2). Returns the new stack. The callers check the
/// mover may use the source first, as for a move.
pub async fn split(
    db: &PgPool,
    mover: Uuid,
    source: &Item,
    quantity: i32,
    to_slot: i32,
    qualities: Option<&Units>,
) -> Result<Result<Item, SplitError>, sqlx::Error> {
    let Some(container_id) = source.container_id else {
        return Ok(Err(SplitError::NotInAContainer));
    };
    let mut tx = db.begin().await?;
    // The stack again, locked, in case it changed since the caller read it
    let current: Option<(Option<Uuid>, String)> =
        sqlx::query_as("SELECT container_id, item_type FROM items WHERE id = $1 FOR UPDATE")
            .bind(source.id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((_, item_type)) = current.filter(|(c, _)| *c == Some(container_id)) else {
        return Ok(Err(SplitError::NotInAContainer));
    };
    let held = units::units_of(&mut tx, source.id).await?;
    if quantity < 1 || quantity as i64 >= held.count() {
        return Ok(Err(SplitError::BadQuantity));
    }
    let wanted = match qualities {
        Some(named) if named.count() != quantity as i64 => return Ok(Err(SplitError::BadQuantity)),
        Some(named) if !held.contains(named) => return Ok(Err(SplitError::NotHeld)),
        Some(named) => named.clone(),
        None => match held.pick(quantity, Pick::default()) {
            Some(lowest) => lowest,
            None => return Ok(Err(SplitError::BadQuantity)),
        },
    };
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

    let Some(taken) = units::take_units(&mut tx, source.id, &wanted).await? else {
        return Ok(Err(SplitError::NotHeld));
    };
    let new_id = units::insert_stack(
        &mut tx,
        NewStack {
            item_type: &item_type,
            owner_id: Some(mover),
            container_id: Some(container_id),
            slot_index: Some(to_slot),
            durability: None,
        },
        &taken.units,
    )
    .await?;
    let new: Item = sqlx::query_as(&format!("SELECT {} FROM items WHERE id = $1", crate::models::ITEM_COLUMNS))
        .bind(new_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Ok(new))
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh database that
/// `sqlx::test` creates from the migrations and drops afterwards. Run them with
/// `cargo test slot_moves -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::units::test_units::{all_in, stack_of, units};

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
        item(db, stack_of(db, owner, container, slot, item_type, &Units::uniform(qty, 60)).await).await
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

        assert_eq!(done, SlotMove::Merged { units: Units::uniform(4, 60) });
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

    /// A player's drop joins any grades, and every unit keeps its quality (D2, E1.8)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_merge_keeps_every_unit(db: PgPool) {
        let ada = player(&db).await;
        let a = item(&db, stack_of(&db, ada.id, ada.inventory, 0, "fiber", &Units::from_pairs([(92, 2), (96, 1)])).await).await;
        let d = item(&db, stack_of(&db, ada.id, ada.inventory, 1, "fiber", &Units::from_pairs([(30, 1)])).await).await;
        let before = all_in(&db, ada.inventory, "fiber").await;

        let done = apply_move(&db, ada.id, &d, ada.inventory, 0, Some(&a)).await.unwrap();

        let merged = Units::from_pairs([(30, 1), (92, 2), (96, 1)]);
        assert_eq!(done, SlotMove::Merged { units: merged.clone() });
        // (30 + 92×2 + 96) / 4 = 77.5, shown rounded down
        assert_eq!(merged.mean(), Some(77));
        assert_eq!(units(&db, a.id).await, merged);
        assert_eq!(all_in(&db, ada.inventory, "fiber").await, before);
    }

    async fn item(db: &PgPool, id: Uuid) -> Item {
        sqlx::query_as(&format!("SELECT {} FROM items WHERE id = $1", crate::models::ITEM_COLUMNS))
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// A split by count takes the lowest first; one by quality takes those
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_split_takes_the_lowest_or_the_units_named(db: PgPool) {
        let ada = player(&db).await;
        let mixed = Units::from_pairs([(50, 2), (90, 2)]);
        let stack = item(&db, stack_of(&db, ada.id, ada.inventory, 0, "fiber", &mixed).await).await;
        assert_eq!(stack.quality, 70);

        let ninety = split(&db, ada.id, &stack, 1, 5, Some(&Units::uniform(1, 90))).await.unwrap().unwrap();
        assert_eq!((ninety.quantity, ninety.quality, ninety.slot_index), (1, 90, Some(5)));
        assert_eq!(ninety.units, Units::uniform(1, 90));
        assert_eq!(ninety.owner_id, Some(ada.id));
        let left = units(&db, stack.id).await;
        assert_eq!((left.count(), left.mean()), (3, Some(63)));

        let lowest = split(&db, ada.id, &stack, 2, 6, None).await.unwrap().unwrap();
        assert_eq!(lowest.units, Units::uniform(2, 50));
        assert_eq!(units(&db, stack.id).await, Units::uniform(1, 90));
        assert_eq!(all_in(&db, ada.inventory, "fiber").await, mixed, "no unit lost its quality");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_split_is_refused_when_it_cant_be_made(db: PgPool) {
        let ada = player(&db).await;
        let fiber = stack(&db, ada.id, ada.inventory, 0, "fiber", 5).await;
        stack(&db, ada.id, ada.inventory, 1, "rope", 1).await;

        let refused = |q, slot| split(&db, ada.id, &fiber, q, slot, None);
        assert_eq!(refused(0, 2).await.unwrap().unwrap_err(), SplitError::BadQuantity);
        assert_eq!(refused(5, 2).await.unwrap().unwrap_err(), SplitError::BadQuantity);
        assert_eq!(refused(2, 1).await.unwrap().unwrap_err(), SplitError::SlotTaken);
        assert_eq!(refused(2, -1).await.unwrap().unwrap_err(), SplitError::BadSlot);
        assert_eq!(refused(2, 10_000).await.unwrap().unwrap_err(), SplitError::BadSlot);
        let nineties = Units::uniform(2, 90);
        let short = Units::uniform(2, 60);
        assert_eq!(split(&db, ada.id, &fiber, 2, 2, Some(&nineties)).await.unwrap().unwrap_err(), SplitError::NotHeld);
        assert_eq!(split(&db, ada.id, &fiber, 3, 2, Some(&short)).await.unwrap().unwrap_err(), SplitError::BadQuantity);
        assert_eq!(units(&db, fiber.id).await, Units::uniform(5, 60));
    }
}
