//! Hand tools in the database (E1.4, plan D3): which of a player's tools gets
//! used, and wearing it down. The numbers are `capitol_protocol::rules::tools`.
//!
//! v1 has no equipment slot: the tool used is the best one of its type in the
//! player's inventory ([`best_tool`]). Each use takes [`tools::WEAR_PER_USE`]
//! ([`wear`]); the use that takes the last of it destroys the tool, and its
//! owner hears `tool_broke` ([`announce_broken`]).

use capitol_protocol::rules::tools::{self, Tool, ToolType};
use capitol_protocol::ws::ToolUse;
use sqlx::{PgConnection, PgPool};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::{GameEvent, GameState};

/// A tool a player holds, as [`best_tool`] found it.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldTool {
    pub item_id: Uuid,
    pub item_type: String,
    pub tool: Tool,
}

impl HeldTool {
    /// What the client hears about this tool after `wear`: what it has left.
    pub fn after(&self, wear: &Wear) -> ToolUse {
        ToolUse {
            item_id: self.item_id,
            item_type: self.item_type.clone(),
            durability: match wear {
                Wear::Worn { durability } => *durability,
                Wear::Broke(_) => 0,
            },
            max_durability: self.tool.max_durability,
        }
    }
}

/// What a use did to a tool.
#[derive(Debug, Clone, PartialEq)]
pub enum Wear {
    /// It has this much left.
    Worn { durability: i32 },
    /// That was its last: it's gone.
    Broke(BrokenTool),
}

/// A tool that wore out, for [`announce_broken`].
#[derive(Debug, Clone, PartialEq)]
pub struct BrokenTool {
    pub owner_id: Uuid,
    /// The container it was in, so the owner's client refetches it.
    pub container_id: Option<Uuid>,
    pub item_id: Uuid,
    pub item_type: String,
    pub item_name: String,
}

/// The best tool of `tool_type` in `player_id`'s inventory, by power × retention
/// ([`tools::best_tool`]), or `None` if they have none with any use left. Ties go
/// to the lower slot.
pub async fn best_tool(db: &PgPool, player_id: Uuid, tool_type: ToolType) -> Result<Option<HeldTool>, sqlx::Error> {
    let held: Vec<(Uuid, String, i32, i32, f64, i32)> = sqlx::query_as(
        "SELECT i.id, i.item_type, i.quality, i.durability, it.base_power, it.max_durability
         FROM items i
         JOIN containers c ON c.id = i.container_id
         JOIN item_types it ON it.id = i.item_type
         WHERE c.owner_id = $1 AND c.container_type = 'player_inventory'
           AND it.tool_type = $2 AND i.durability > 0
         ORDER BY i.slot_index, i.id",
    )
    .bind(player_id)
    .bind(tool_type.as_str())
    .fetch_all(db)
    .await?;

    let candidates = held.into_iter().map(|(item_id, item_type, quality, durability, base_power, base_durability)| {
        let tool = Tool {
            base_power,
            quality,
            durability,
            max_durability: tools::max_durability(base_durability, quality),
        };
        (HeldTool { item_id, item_type, tool }, tool)
    });
    Ok(tools::best_tool(candidates))
}

/// Take one use's wear off tool `item_id`, deleting it if that was its last. Runs on
/// `conn`, so a caller can wear the tool in the same transaction as the work it did;
/// once that commits, it passes a [`Wear::Broke`] to [`announce_broken`]. `None` if
/// the item is gone or isn't a tool.
pub async fn wear(conn: &mut PgConnection, item_id: Uuid) -> Result<Option<Wear>, sqlx::Error> {
    let worn: Option<(i32, Option<Uuid>, Option<Uuid>, String, String)> = sqlx::query_as(
        "UPDATE items i SET durability = GREATEST(i.durability - $2, 0)
         FROM item_types it
         WHERE i.id = $1 AND it.id = i.item_type AND i.durability IS NOT NULL
         RETURNING i.durability, i.owner_id, i.container_id, i.item_type, it.name",
    )
    .bind(item_id)
    .bind(tools::WEAR_PER_USE)
    .fetch_optional(&mut *conn)
    .await?;

    let Some((durability, owner_id, container_id, item_type, item_name)) = worn else {
        return Ok(None);
    };
    if durability > 0 {
        return Ok(Some(Wear::Worn { durability }));
    }
    sqlx::query("DELETE FROM items WHERE id = $1").bind(item_id).execute(&mut *conn).await?;
    let Some(owner_id) = owner_id else {
        // Nobody's to tell
        return Ok(Some(Wear::Worn { durability: 0 }));
    };
    Ok(Some(Wear::Broke(BrokenTool { owner_id, container_id, item_id, item_type, item_name })))
}

/// [`wear`], but only while the tool is still in `player_id`'s inventory: a gather
/// or craft wears the tool it started with, and one that has since been dropped,
/// stashed or traded away isn't theirs to wear. `None` if it isn't there. The row is
/// locked, so a move at the same moment waits for the caller's transaction.
pub async fn wear_held(conn: &mut PgConnection, item_id: Uuid, player_id: Uuid) -> Result<Option<Wear>, sqlx::Error> {
    let held: Option<Uuid> = sqlx::query_scalar(
        "SELECT i.id FROM items i
         JOIN containers c ON c.id = i.container_id
         WHERE i.id = $1 AND c.owner_id = $2 AND c.container_type = 'player_inventory'
         FOR UPDATE OF i",
    )
    .bind(item_id)
    .bind(player_id)
    .fetch_optional(&mut *conn)
    .await?;
    if held.is_none() {
        return Ok(None);
    }
    wear(conn, item_id).await
}

/// The best tool `player_id` holds for gathering `node_id`, of the type its resource
/// names (`resource_types.tool_type`). `None` if it names none, the player holds
/// none, or the node is gone (the tick says so). Look it up before queueing
/// `StartExtraction`, from REST and WS alike.
pub async fn tool_for_node(
    db: &PgPool,
    game: &RwLock<GameState>,
    player_id: Uuid,
    node_id: Uuid,
) -> Result<Option<HeldTool>, sqlx::Error> {
    let tool_type = {
        let game = game.read().await;
        game.resource_nodes
            .get(&node_id)
            .and_then(|n| game.resource_types.get(&n.resource_type))
            .and_then(|t| t.tool())
    };
    match tool_type {
        Some(t) => best_tool(db, player_id, t).await,
        None => Ok(None),
    }
}

/// The best tool `player_id` holds for crafting `recipe_id`, of the type it needs
/// (`crafting_requirements.tool_type`). `None` if it needs none or they hold none.
pub async fn tool_for_recipe(
    db: &PgPool,
    game: &RwLock<GameState>,
    player_id: Uuid,
    recipe_id: &str,
) -> Result<Option<HeldTool>, sqlx::Error> {
    let tool_type = game.read().await.recipes.get(recipe_id).and_then(|r| r.tool_type);
    match tool_type {
        Some(t) => best_tool(db, player_id, t).await,
        None => Ok(None),
    }
}

/// Tell a broken tool's owner: `tool_broke`, and `inventory_updated` for the
/// container it left.
pub fn announce_broken(events: &broadcast::Sender<GameEvent>, broken: &BrokenTool) {
    let _ = events.send(GameEvent::ToolBroke {
        player_id: broken.owner_id,
        item_id: broken.item_id,
        item_type: broken.item_type.clone(),
        item_name: broken.item_name.clone(),
    });
    if let Some(container_id) = broken.container_id {
        let _ = events.send(GameEvent::InventoryUpdated { player_id: broken.owner_id, container_id, slots: Vec::new() });
    }
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh database that
/// `sqlx::test` creates from the migrations and drops afterwards, with the content synced so
/// the tools exist. Run them with `cargo test engine::tools -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;

    struct Player {
        id: Uuid,
        inventory: Uuid,
        chest: Uuid,
    }

    async fn setup(db: &PgPool) -> Player {
        crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
        // A bronze and a steel sickle beside the stone one, for the GDD's example
        for (id, power) in [("bronze_sickle_test", 10.0), ("steel_sickle_test", 18.0)] {
            sqlx::query(
                "INSERT INTO item_types (id, name, category, stackable, weight, tool_type, tier, base_power, max_durability)
                 VALUES ($1, $1, 'tool', false, 1, 'sickle', 'bronze', $2, 100)",
            )
            .bind(id)
            .bind(power)
            .execute(db)
            .await
            .unwrap();
        }
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
        Player { id, inventory: containers[0], chest: containers[1] }
    }

    async fn tool(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, kind: &str, quality: i32, durability: i32) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
             VALUES ($1, $2, $3, 1, $4, $5, $6, $7) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(kind)
        .bind(quality)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .bind(durability)
        .fetch_one(db)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn excellent_bronze_is_picked_over_poor_steel(db: PgPool) {
        let ada = setup(&db).await;
        tool(&db, ada.id, ada.inventory, 0, "steel_sickle_test", 20, 60).await;
        let bronze = tool(&db, ada.id, ada.inventory, 1, "bronze_sickle_test", 90, 140).await;
        tool(&db, ada.id, ada.inventory, 2, "stone_sickle", 100, 90).await;

        let best = best_tool(&db, ada.id, ToolType::Sickle).await.unwrap().unwrap();

        assert_eq!(best.item_id, bronze);
        assert_eq!(best.tool.max_durability, 140, "100 × 1.4 at Q90");
        assert_eq!(best_tool(&db, ada.id, ToolType::Axe).await.unwrap(), None, "no axe");
    }

    /// Only the inventory counts: not the crafting grid, not a tool that's spent
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn only_a_usable_tool_in_the_inventory_is_picked(db: PgPool) {
        let ada = setup(&db).await;
        tool(&db, ada.id, ada.chest, 0, "steel_sickle_test", 100, 150).await;
        tool(&db, ada.id, ada.inventory, 0, "steel_sickle_test", 100, 0).await;
        let stone = tool(&db, ada.id, ada.inventory, 1, "stone_sickle", 10, 1).await;

        let best = best_tool(&db, ada.id, ToolType::Sickle).await.unwrap().unwrap();

        assert_eq!(best.item_id, stone);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_tool_wears_and_breaks_with_a_notice(db: PgPool) {
        let ada = setup(&db).await;
        let sickle = tool(&db, ada.id, ada.inventory, 3, "stone_sickle", 50, 2).await;
        let mut conn = db.acquire().await.unwrap();

        assert_eq!(wear(&mut conn, sickle).await.unwrap(), Some(Wear::Worn { durability: 1 }));
        let Some(Wear::Broke(broken)) = wear(&mut conn, sickle).await.unwrap() else {
            panic!("the last use breaks it");
        };
        assert_eq!(broken.owner_id, ada.id);
        assert_eq!(broken.container_id, Some(ada.inventory));
        assert_eq!(broken.item_name, "Stone Sickle");
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE id = $1")
            .bind(sickle)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left, 0, "a broken tool is gone");
        assert_eq!(wear(&mut conn, sickle).await.unwrap(), None, "nothing left to wear");

        let (events, mut heard) = broadcast::channel(4);
        announce_broken(&events, &broken);
        assert!(matches!(heard.try_recv().unwrap(),
            GameEvent::ToolBroke { player_id, item_id, ref item_type, .. }
                if player_id == ada.id && item_id == sickle && item_type == "stone_sickle"));
        assert!(matches!(heard.try_recv().unwrap(),
            GameEvent::InventoryUpdated { player_id, container_id, .. }
                if player_id == ada.id && container_id == ada.inventory));
    }

    /// A tool that has left the player's inventory isn't worn on their behalf
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn only_a_held_tool_is_worn(db: PgPool) {
        let ada = setup(&db).await;
        let held = tool(&db, ada.id, ada.inventory, 0, "stone_sickle", 50, 10).await;
        let stashed = tool(&db, ada.id, ada.chest, 0, "stone_sickle", 50, 10).await;
        let mut conn = db.acquire().await.unwrap();

        assert_eq!(wear_held(&mut conn, held, ada.id).await.unwrap(), Some(Wear::Worn { durability: 9 }));
        assert_eq!(wear_held(&mut conn, held, Uuid::new_v4()).await.unwrap(), None, "not theirs");
        assert_eq!(wear_held(&mut conn, stashed, ada.id).await.unwrap(), None, "not in the inventory");
        let left: i32 = sqlx::query_scalar("SELECT durability FROM items WHERE id = $1")
            .bind(stashed)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left, 10);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn wearing_something_that_isnt_a_tool_does_nothing(db: PgPool) {
        let ada = setup(&db).await;
        let fiber: Uuid = sqlx::query_scalar(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'fiber', 50, 3, $2, $3, 0) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(ada.id)
        .bind(ada.inventory)
        .fetch_one(&db)
        .await
        .unwrap();
        let mut conn = db.acquire().await.unwrap();

        assert_eq!(wear(&mut conn, fiber).await.unwrap(), None);
    }
}
