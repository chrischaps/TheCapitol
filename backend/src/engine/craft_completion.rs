//! Finishing a craft in the database (B10, #61).
//!
//! A craft's inputs are checked when it starts, and nothing stops them moving during the
//! seconds it runs: a trade, a merge or a split can all reach them. So at completion, in one
//! transaction, every input is checked again (it still exists, the crafter owns it, it holds
//! enough), the inputs are consumed and the output is written. If anything is wrong the craft
//! fails and nothing changes.

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// A craft that has run its time, waiting to be written
#[derive(Debug, Clone)]
pub struct FinishedCraft {
    pub player_id: Uuid,
    pub operation_id: Uuid,
    /// (item_id, quantity to consume)
    pub inputs: Vec<(Uuid, i32)>,
    pub outputs: Vec<CraftOutput>,
}

#[derive(Debug, Clone)]
pub struct CraftOutput {
    pub item_type: String,
    pub item_name: String,
    pub quantity: i32,
    pub quality: i32,
}

/// The reason shown when an input moved, split or changed hands while the craft ran
pub const INPUTS_MOVED: &str = "The craft's inputs moved before it finished";
/// The reason shown when the output slot holds something else
pub const OUTPUT_FULL: &str = "Your crafting output is full. Empty it and craft again";

/// Consume the inputs and write the outputs, together or not at all. Returns the containers
/// that changed, each once, for `inventory_updated`. The `Err` is the reason for
/// `crafting_failed`.
pub async fn finish_craft(db: &PgPool, craft: &FinishedCraft) -> Result<Vec<Uuid>, String> {
    let mut tx = db.begin().await.map_err(db_error)?;
    let mut changed: Vec<Uuid> = Vec::new();

    consume_inputs(&mut tx, craft, &mut changed).await?;
    for output in &craft.outputs {
        let container = place_output(&mut tx, craft.player_id, output).await?;
        if !changed.contains(&container) {
            changed.push(container);
        }
    }

    tx.commit().await.map_err(db_error)?;
    Ok(changed)
}

/// Check every input again and use it up. Rows are locked as they're read, so a move or trade
/// running at the same moment waits for this transaction rather than slipping between.
async fn consume_inputs(
    tx: &mut Transaction<'_, Postgres>,
    craft: &FinishedCraft,
    changed: &mut Vec<Uuid>,
) -> Result<(), String> {
    // The same stack listed twice must hold both amounts
    let mut wanted: Vec<(Uuid, i32)> = Vec::new();
    for &(id, qty) in &craft.inputs {
        match wanted.iter_mut().find(|(w, _)| *w == id) {
            Some((_, total)) => *total += qty,
            None => wanted.push((id, qty)),
        }
    }

    for (item_id, qty) in wanted {
        let held: Option<(Option<Uuid>, i32)> = sqlx::query_as(
            "SELECT container_id, quantity FROM items WHERE id = $1 AND owner_id = $2 FOR UPDATE",
        )
        .bind(item_id)
        .bind(craft.player_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_error)?;

        let (container_id, have) = match held {
            Some(row) if row.1 >= qty => row,
            _ => {
                tracing::info!(
                    "Craft {} by {} failed: input {} is gone, or no longer {}'s, or under {}",
                    craft.operation_id, craft.player_id, item_id, craft.player_id, qty
                );
                return Err(INPUTS_MOVED.to_string());
            }
        };

        if have == qty {
            sqlx::query("DELETE FROM items WHERE id = $1")
                .bind(item_id)
                .execute(&mut **tx)
                .await
                .map_err(db_error)?;
        } else {
            sqlx::query("UPDATE items SET quantity = $1 WHERE id = $2")
                .bind(have - qty)
                .bind(item_id)
                .execute(&mut **tx)
                .await
                .map_err(db_error)?;
        }
        if let Some(c) = container_id {
            if !changed.contains(&c) {
                changed.push(c);
            }
        }
    }
    Ok(())
}

/// Put `output` in the crafter's output container: onto a stack of the same type (quality
/// averaged by quantity), or into the first free slot. Returns the container.
async fn place_output(
    tx: &mut Transaction<'_, Postgres>,
    player_id: Uuid,
    output: &CraftOutput,
) -> Result<Uuid, String> {
    let (container_id, slot_count): (Uuid, i32) = sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.owner_id = $1 AND c.container_type = 'crafting_output'
         LIMIT 1",
    )
    .bind(player_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?
    .ok_or_else(|| {
        tracing::error!("No crafting_output container for player {}", player_id);
        "You have no crafting output".to_string()
    })?;

    let existing: Option<(Uuid, i32, i32)> = sqlx::query_as(
        "SELECT id, quantity, quality FROM items
         WHERE container_id = $1 AND item_type = $2
         LIMIT 1 FOR UPDATE",
    )
    .bind(container_id)
    .bind(&output.item_type)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?;

    if let Some((id, qty, quality)) = existing {
        let new_qty = qty + output.quantity;
        let new_quality = (quality * qty + output.quality * output.quantity) / new_qty;
        sqlx::query("UPDATE items SET quantity = $1, quality = $2 WHERE id = $3")
            .bind(new_qty)
            .bind(new_quality)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(db_error)?;
        return Ok(container_id);
    }

    let used: Vec<(i32,)> = sqlx::query_as(
        "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL",
    )
    .bind(container_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(db_error)?;
    let slot = (0..slot_count)
        .find(|s| !used.iter().any(|(u,)| u == s))
        .ok_or_else(|| OUTPUT_FULL.to_string())?;

    sqlx::query(
        "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(&output.item_type)
    .bind(output.quality)
    .bind(output.quantity)
    .bind(player_id)
    .bind(container_id)
    .bind(slot)
    .execute(&mut **tx)
    .await
    .map_err(db_error)?;
    Ok(container_id)
}

fn db_error(e: sqlx::Error) -> String {
    tracing::error!("Craft completion: database error: {:?}", e);
    "The craft couldn't be saved. Try again".to_string()
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh database that
/// `sqlx::test` creates from the migrations and drops afterwards. Run them with
/// `cargo test craft_completion -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;

    struct Crafter {
        id: Uuid,
        inventory: Uuid,
        input: Uuid,
        output: Uuid,
    }

    async fn crafter(db: &PgPool, name: &str) -> Crafter {
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
        for kind in ["player_inventory", "crafting_input", "crafting_output"] {
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
        Crafter { id, inventory: containers[0], input: containers[1], output: containers[2] }
    }

    async fn stack(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, item_type: &str, qty: i32) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, $2, 60, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(item_type)
        .bind(qty)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
        id
    }

    /// Basic rope: 2 fiber, 1 rope
    fn rope(player: &Crafter, fiber: Uuid) -> FinishedCraft {
        FinishedCraft {
            player_id: player.id,
            operation_id: Uuid::new_v4(),
            inputs: vec![(fiber, 2)],
            outputs: vec![CraftOutput {
                item_type: "rope".into(),
                item_name: "Rope".into(),
                quantity: 1,
                quality: 60,
            }],
        }
    }

    /// (owner, container, quantity) of an item, or `None` once it's gone
    async fn item(db: &PgPool, id: Uuid) -> Option<(Uuid, Uuid, i32)> {
        sqlx::query_as("SELECT owner_id, container_id, quantity FROM items WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await
            .unwrap()
    }

    /// The rope in `player`'s output, if any
    async fn rope_made(db: &PgPool, player: &Crafter) -> i32 {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT quantity FROM items WHERE container_id = $1 AND item_type = 'rope'",
        )
        .bind(player.output)
        .fetch_optional(db)
        .await
        .unwrap();
        row.map_or(0, |(q,)| q)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_craft_turns_its_inputs_into_its_output(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 5).await;

        let changed = finish_craft(&db, &rope(&ada, fiber)).await.unwrap();

        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(3), "two fiber used");
        assert_eq!(rope_made(&db, &ada).await, 1);
        assert_eq!(changed, vec![ada.input, ada.output], "each changed container once");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_last_of_a_stack_is_deleted(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;

        finish_craft(&db, &rope(&ada, fiber)).await.unwrap();

        assert_eq!(item(&db, fiber).await, None);
        assert_eq!(rope_made(&db, &ada).await, 1);
    }

    /// A trade of the whole stack keeps its id and changes its owner
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn fiber_traded_away_mid_craft_fails_the_craft(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let bram = crafter(&db, "bram").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 5).await;
        sqlx::query("UPDATE items SET owner_id = $1, container_id = $2, slot_index = 0 WHERE id = $3")
            .bind(bram.id)
            .bind(bram.inventory)
            .bind(fiber)
            .execute(&db)
            .await
            .unwrap();

        let result = finish_craft(&db, &rope(&ada, fiber)).await;

        assert_eq!(result, Err(INPUTS_MOVED.to_string()));
        assert_eq!(item(&db, fiber).await, Some((bram.id, bram.inventory, 5)), "Bram's fiber untouched");
        assert_eq!(rope_made(&db, &ada).await, 0, "no rope for Ada");
    }

    /// A merge deletes the source stack and adds it to the target
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn fiber_merged_away_mid_craft_fails_the_craft(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;
        let other = stack(&db, ada.id, ada.inventory, 0, "fiber", 3).await;
        sqlx::query("DELETE FROM items WHERE id = $1").bind(fiber).execute(&db).await.unwrap();
        sqlx::query("UPDATE items SET quantity = 5 WHERE id = $1").bind(other).execute(&db).await.unwrap();

        let result = finish_craft(&db, &rope(&ada, fiber)).await;

        assert_eq!(result, Err(INPUTS_MOVED.to_string()));
        assert_eq!(item(&db, other).await.map(|i| i.2), Some(5), "the merged stack untouched");
        assert_eq!(rope_made(&db, &ada).await, 0, "no free rope");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn fiber_split_below_the_recipe_fails_the_craft(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;
        sqlx::query("UPDATE items SET quantity = 1 WHERE id = $1").bind(fiber).execute(&db).await.unwrap();

        let result = finish_craft(&db, &rope(&ada, fiber)).await;

        assert_eq!(result, Err(INPUTS_MOVED.to_string()));
        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(1));
        assert_eq!(rope_made(&db, &ada).await, 0);
    }

    /// Still Ada's, just somewhere else: the craft goes ahead
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn fiber_moved_to_the_backpack_still_crafts(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.inventory, 4, "fiber", 4).await;

        let changed = finish_craft(&db, &rope(&ada, fiber)).await.unwrap();

        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(2));
        assert_eq!(changed, vec![ada.inventory, ada.output]);
    }

    /// Writing nothing beats losing the output: the inputs stay
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_output_fails_the_craft_and_keeps_the_inputs(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;
        stack(&db, ada.id, ada.output, 0, "fiber", 1).await;

        let result = finish_craft(&db, &rope(&ada, fiber)).await;

        assert_eq!(result, Err(OUTPUT_FULL.to_string()));
        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(2), "inputs rolled back");
        assert_eq!(rope_made(&db, &ada).await, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn rope_stacks_onto_rope_already_in_the_output(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;
        stack(&db, ada.id, ada.output, 0, "rope", 3).await;

        finish_craft(&db, &rope(&ada, fiber)).await.unwrap();

        assert_eq!(rope_made(&db, &ada).await, 4);
    }
}
