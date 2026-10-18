//! A station's tool slot (E1.6, plan D3).
//!
//! A crafting station holds one tool. Anyone who may use the station
//! (`container_access::check_station_access`) may put one of their tools in
//! ([`install_tool`]). Taking it out follows the box rule (#80): the tool is
//! still whoever put it in's, so they may take it back, and so may the
//! station's owner, which makes it theirs ([`remove_tool`]).
//!
//! While it's in, the tool is an item in no container and owned by no one, and
//! the station keeps who put it there (`stations.tool_owner`). A craft at the
//! station whose recipe needs a tool of its type uses it, in place of one in
//! the crafter's hands ([`tool_for_station_recipe`]), and wears it
//! ([`wear_installed`]). Taking the station down gives it back with the kit
//! (`station_removal::remove_station`), and so does foreclosure
//! (`station_removal::return_plot_stations`).

use std::collections::HashMap;

use capitol_protocol::rest::StationTool;
use capitol_protocol::rules::tools::{self, Tool, ToolType};
use sqlx::{PgConnection, PgPool};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::container_access::{check_station_access, may_move_item, AccessError};
use super::station_removal::free_slots;
use super::tools::{BrokenTool, HeldTool, Wear};
use super::{GameEvent, GameState};

/// Why a tool can't go in or come out.
#[derive(Debug)]
pub enum ToolSlotError {
    /// The station rule refused, or there's no such station or item.
    Access(AccessError),
    /// A storage station has no tool slot.
    NoToolSlot,
    /// The slot already holds a tool.
    SlotFull,
    /// The item isn't a tool with any use left.
    NotATool,
    /// The slot is empty.
    NoTool,
    /// Someone else's tool, in someone else's station.
    NotYourTool,
    /// No free inventory slot for it.
    NoRoom,
}

impl ToolSlotError {
    pub fn message(&self) -> String {
        match self {
            ToolSlotError::Access(e) => e.message(),
            ToolSlotError::NoToolSlot => "This station has no tool slot".into(),
            ToolSlotError::SlotFull => "The station already holds a tool. Take it out first".into(),
            ToolSlotError::NotATool => "Only a tool with uses left goes in a tool slot".into(),
            ToolSlotError::NoTool => "The station holds no tool".into(),
            ToolSlotError::NotYourTool => "That tool isn't yours to take".into(),
            ToolSlotError::NoRoom => "No room in your inventory for the tool".into(),
        }
    }
}

impl From<AccessError> for ToolSlotError {
    fn from(e: AccessError) -> Self {
        ToolSlotError::Access(e)
    }
}

impl From<sqlx::Error> for ToolSlotError {
    fn from(e: sqlx::Error) -> Self {
        ToolSlotError::Access(AccessError::Database(e))
    }
}

/// What an install or a removal changed.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotChange {
    /// The player's inventory, which the tool left or went into
    pub inventory_id: Uuid,
    /// The slot now: the tool installed, or `None` after a removal
    pub tool: Option<StationTool>,
}

/// Put tool `item_id` from `player_id`'s inventory into `station_id`'s empty
/// tool slot. `at` is where the player stands now (`container_access::live_position`).
pub async fn install_tool(
    db: &PgPool,
    player_id: Uuid,
    station_id: Uuid,
    item_id: Uuid,
    at: Option<(f64, f64)>,
) -> Result<SlotChange, ToolSlotError> {
    check_station_access(db, player_id, station_id, at).await?;
    let mut tx = db.begin().await?;

    let (installed, category): (Option<Uuid>, String) = sqlx::query_as(
        "SELECT s.installed_tool, st.category FROM stations s
         JOIN station_types st ON st.id = s.station_type
         WHERE s.id = $1 FOR UPDATE OF s",
    )
    .bind(station_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AccessError::StationNotFound)?;
    if category != "crafting" {
        return Err(ToolSlotError::NoToolSlot);
    }
    if installed.is_some() {
        return Err(ToolSlotError::SlotFull);
    }

    let (inventory_id, item_type, quality, durability, tool_type, base_durability): (
        Uuid,
        String,
        i32,
        Option<i32>,
        Option<String>,
        Option<i32>,
    ) = sqlx::query_as(
        "SELECT c.id, i.item_type, i.quality, i.durability, it.tool_type, it.max_durability
         FROM items i
         JOIN containers c ON c.id = i.container_id
         JOIN item_types it ON it.id = i.item_type
         WHERE i.id = $1 AND c.owner_id = $2 AND c.container_type = 'player_inventory'
         FOR UPDATE OF i",
    )
    .bind(item_id)
    .bind(player_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AccessError::ItemNotFound)?;
    let (Some(_), Some(durability), Some(base_durability)) = (tool_type, durability, base_durability) else {
        return Err(ToolSlotError::NotATool);
    };
    if durability <= 0 {
        return Err(ToolSlotError::NotATool);
    }

    sqlx::query("UPDATE items SET container_id = NULL, slot_index = NULL, owner_id = NULL WHERE id = $1")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE stations SET installed_tool = $2, tool_owner = $3 WHERE id = $1")
        .bind(station_id)
        .bind(item_id)
        .bind(player_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(SlotChange {
        inventory_id,
        tool: Some(StationTool {
            item_id,
            item_type,
            quality,
            durability,
            max_durability: tools::max_durability(base_durability, quality),
            owner_id: Some(player_id),
        }),
    })
}

/// Take `station_id`'s tool into `player_id`'s inventory: their own tool, or
/// anyone's from their own station, which makes it theirs (#80).
pub async fn remove_tool(
    db: &PgPool,
    player_id: Uuid,
    station_id: Uuid,
    at: Option<(f64, f64)>,
) -> Result<SlotChange, ToolSlotError> {
    let station_owner = check_station_access(db, player_id, station_id, at).await?;
    let mut tx = db.begin().await?;

    let (installed, tool_owner): (Option<Uuid>, Option<Uuid>) =
        sqlx::query_as("SELECT installed_tool, tool_owner FROM stations WHERE id = $1 FOR UPDATE")
            .bind(station_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AccessError::StationNotFound)?;
    let item_id = installed.ok_or(ToolSlotError::NoTool)?;
    if !may_move_item(player_id, tool_owner, Some(station_owner)) {
        return Err(ToolSlotError::NotYourTool);
    }
    let (inventory_id, slots) = free_slots(&mut tx, player_id, 1).await?.ok_or(ToolSlotError::NoRoom)?;

    give_back(&mut tx, station_id, item_id, player_id, inventory_id, slots[0]).await?;
    tx.commit().await?;
    Ok(SlotChange { inventory_id, tool: None })
}

/// Move a station's tool into `player_id`'s inventory at `slot`, theirs now,
/// and empty the station's slot.
pub(crate) async fn give_back(
    conn: &mut PgConnection,
    station_id: Uuid,
    item_id: Uuid,
    player_id: Uuid,
    inventory_id: Uuid,
    slot: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE items SET container_id = $1, slot_index = $2, owner_id = $3 WHERE id = $4")
        .bind(inventory_id)
        .bind(slot)
        .bind(player_id)
        .bind(item_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE stations SET installed_tool = NULL, tool_owner = NULL WHERE id = $1")
        .bind(station_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The tool in `station_id`'s slot, if it's a `tool_type` with uses left.
pub async fn installed_tool(db: &PgPool, station_id: Uuid, tool_type: ToolType) -> Result<Option<HeldTool>, sqlx::Error> {
    let row: Option<(Uuid, String, i32, i32, f64, i32)> = sqlx::query_as(
        "SELECT i.id, i.item_type, i.quality, i.durability, it.base_power, it.max_durability
         FROM stations s
         JOIN items i ON i.id = s.installed_tool
         JOIN item_types it ON it.id = i.item_type
         WHERE s.id = $1 AND it.tool_type = $2 AND i.durability > 0",
    )
    .bind(station_id)
    .bind(tool_type.as_str())
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(item_id, item_type, quality, durability, base_power, base_durability)| HeldTool {
        item_id,
        item_type,
        tool: Tool { base_power, quality, durability, max_durability: tools::max_durability(base_durability, quality) },
        station_id: Some(station_id),
    }))
}

/// The tool for crafting `recipe_id` at `station_id`: the station's own, if its
/// slot holds the type the recipe needs, or else the best `player_id` holds
/// (`tools::tool_for_recipe`). Look it up before queueing `CraftAtStation`.
pub async fn tool_for_station_recipe(
    db: &PgPool,
    game: &RwLock<GameState>,
    player_id: Uuid,
    station_id: Uuid,
    recipe_id: &str,
) -> Result<Option<HeldTool>, sqlx::Error> {
    let tool_type = game.read().await.recipes.get(recipe_id).and_then(|r| r.tool_type);
    let Some(tool_type) = tool_type else {
        return Ok(None);
    };
    if let Some(installed) = installed_tool(db, station_id, tool_type).await? {
        return Ok(Some(installed));
    }
    super::tools::best_tool(db, player_id, tool_type).await
}

/// [`super::tools::wear`] for a station's tool, but only while it's still in
/// `station_id`'s slot. `None` if it's been taken out. The station row is
/// locked, so a removal at the same moment waits for the caller's transaction.
/// The use that breaks it empties the slot, and its owner hears `tool_broke`.
/// Alongside, what the slot holds now, for `station_tool_changed`.
pub async fn wear_installed(
    conn: &mut PgConnection,
    station_id: Uuid,
    item_id: Uuid,
) -> Result<Option<(Wear, Option<StationTool>)>, sqlx::Error> {
    let found: Option<(Option<Uuid>, String, String)> = sqlx::query_as(
        "SELECT s.tool_owner, i.item_type, it.name FROM stations s
         JOIN items i ON i.id = s.installed_tool
         JOIN item_types it ON it.id = i.item_type
         WHERE s.id = $1 AND s.installed_tool = $2
         FOR UPDATE OF s",
    )
    .bind(station_id)
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((tool_owner, item_type, item_name)) = found else {
        return Ok(None);
    };
    let Some(wear) = super::tools::wear(conn, item_id).await? else {
        return Ok(None);
    };
    match wear {
        Wear::Worn { durability } if durability > 0 => {
            let now = station_tool(conn, station_id).await?;
            Ok(Some((wear, now)))
        }
        // That was its last use: `wear` deleted it, which emptied the slot
        _ => {
            sqlx::query("UPDATE stations SET installed_tool = NULL, tool_owner = NULL WHERE id = $1")
                .bind(station_id)
                .execute(&mut *conn)
                .await?;
            let wear = match tool_owner {
                Some(owner_id) => {
                    Wear::Broke(BrokenTool { owner_id, container_id: None, item_id, item_type, item_name })
                }
                None => Wear::Worn { durability: 0 },
            };
            Ok(Some((wear, None)))
        }
    }
}

/// What `station_id`'s tool slot holds.
pub async fn station_tool(conn: &mut PgConnection, station_id: Uuid) -> Result<Option<StationTool>, sqlx::Error> {
    let row: Option<StationToolRow> = sqlx::query_as(&format!("{TOOL_SELECT} WHERE s.id = $1"))
        .bind(station_id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map(|r| r.into_tool().1))
}

/// The tools installed in any of `station_ids`, by station, for `StationInfo`.
pub async fn tools_at(db: &PgPool, station_ids: &[Uuid]) -> Result<HashMap<Uuid, StationTool>, sqlx::Error> {
    if station_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<StationToolRow> = sqlx::query_as(&format!("{TOOL_SELECT} WHERE s.id = ANY($1)"))
        .bind(station_ids)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(StationToolRow::into_tool).collect())
}

const TOOL_SELECT: &str = "SELECT s.id AS station_id, s.tool_owner, i.id AS item_id, i.item_type, i.quality,
        i.durability, it.max_durability
    FROM stations s
    JOIN items i ON i.id = s.installed_tool
    JOIN item_types it ON it.id = i.item_type";

#[derive(sqlx::FromRow)]
struct StationToolRow {
    station_id: Uuid,
    tool_owner: Option<Uuid>,
    item_id: Uuid,
    item_type: String,
    quality: i32,
    durability: Option<i32>,
    max_durability: Option<i32>,
}

impl StationToolRow {
    fn into_tool(self) -> (Uuid, StationTool) {
        let tool = StationTool {
            item_id: self.item_id,
            item_type: self.item_type,
            quality: self.quality,
            durability: self.durability.unwrap_or(0),
            max_durability: tools::max_durability(self.max_durability.unwrap_or(0), self.quality),
            owner_id: self.tool_owner,
        };
        (self.station_id, tool)
    }
}

/// Tell everyone what `station_id`'s slot holds now.
pub fn announce(events: &broadcast::Sender<GameEvent>, station_id: Uuid, tool: Option<StationTool>) {
    let _ = events.send(GameEvent::StationToolChanged { station_id, tool });
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh database that
/// `sqlx::test` creates from the migrations and drops afterwards, with the content synced so
/// the stone sickle exists. Run them with `cargo test station_tools -- --ignored`.
#[cfg(test)]
mod tests {
    use super::super::craft_completion::{finish_craft, CraftOutput, FinishedCraft, STATION_TOOL_MOVED};
    use super::*;

    const HERE: Option<(f64, f64)> = Some((0.0, 0.0));

    struct Player {
        id: Uuid,
        inventory: Uuid,
        input: Uuid,
    }

    async fn player(db: &PgPool, name: &str) -> Player {
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
        Player { id, inventory: containers[0], input: containers[1] }
    }

    /// A station of `kind` owned by `owner`, off any plot, at the origin
    async fn station(db: &PgPool, owner: Uuid, kind: &str) -> Uuid {
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
        id
    }

    async fn item(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, kind: &str, qty: i32, durability: Option<i32>) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
             VALUES ($1, $2, 50, $3, $4, $5, $6, $7) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(kind)
        .bind(qty)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .bind(durability)
        .fetch_one(db)
        .await
        .unwrap()
    }

    async fn setup(db: &PgPool) -> (Player, Uuid) {
        crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
        let ada = player(db, "ada").await;
        let bench = station(db, ada.id, "workbench").await;
        (ada, bench)
    }

    /// (owner, container, slot, durability) of an item, or `None` once it's gone
    async fn where_is(db: &PgPool, id: Uuid) -> Option<(Option<Uuid>, Option<Uuid>, Option<i32>, Option<i32>)> {
        sqlx::query_as("SELECT owner_id, container_id, slot_index, durability FROM items WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await
            .unwrap()
    }

    async fn slot_of(db: &PgPool, station_id: Uuid) -> (Option<Uuid>, Option<Uuid>) {
        sqlx::query_as("SELECT installed_tool, tool_owner FROM stations WHERE id = $1")
            .bind(station_id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// Rope from 2 fiber, made with `tool`
    fn rope(player: &Player, fiber: Uuid, tool: HeldTool) -> FinishedCraft {
        FinishedCraft {
            player_id: player.id,
            operation_id: Uuid::new_v4(),
            inputs: vec![(fiber, capitol_protocol::rules::stacking::Units::uniform(2, 50))],
            outputs: vec![CraftOutput {
                item_type: "rope".into(),
                item_name: "Rope".into(),
                quantity: 1,
                quality: 60,
                stackable: true,
                durability: None,
            }],
            tool: Some(tool),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn install_craft_and_remove(db: PgPool) {
        let (ada, bench) = setup(&db).await;
        let sickle = item(&db, ada.id, ada.inventory, 2, "stone_sickle", 1, Some(3)).await;
        let fiber = item(&db, ada.id, ada.input, 0, "fiber", 4, None).await;

        let change = install_tool(&db, ada.id, bench, sickle, HERE).await.unwrap();
        assert_eq!(change.inventory_id, ada.inventory);
        let tool = change.tool.unwrap();
        assert_eq!((tool.item_id, tool.durability, tool.max_durability, tool.owner_id), (sickle, 3, 60, Some(ada.id)));
        assert_eq!(where_is(&db, sickle).await, Some((None, None, None, Some(3))), "in no one's hands");
        assert_eq!(slot_of(&db, bench).await, (Some(sickle), Some(ada.id)));
        assert_eq!(tools_at(&db, &[bench]).await.unwrap().get(&bench), Some(&tool));
        // Hands no longer find it; the station does
        assert_eq!(super::super::tools::best_tool(&db, ada.id, ToolType::Sickle).await.unwrap(), None);
        assert_eq!(installed_tool(&db, bench, ToolType::Axe).await.unwrap(), None, "not an axe");
        let installed = installed_tool(&db, bench, ToolType::Sickle).await.unwrap().unwrap();
        assert_eq!((installed.item_id, installed.station_id), (sickle, Some(bench)));

        // A craft at the bench wears the bench's tool, and everyone hears what's left
        let done = finish_craft(&db, &rope(&ada, fiber, installed)).await.unwrap();
        assert_eq!(done.tool.map(|t| t.durability), Some(2));
        assert_eq!(done.broken, None);
        let (at, now) = done.station_tool.unwrap();
        assert_eq!((at, now.map(|t| t.durability)), (bench, Some(2)));

        // One slot, one tool
        let spare = item(&db, ada.id, ada.inventory, 3, "stone_sickle", 1, Some(9)).await;
        assert!(matches!(install_tool(&db, ada.id, bench, spare, HERE).await, Err(ToolSlotError::SlotFull)));

        let change = remove_tool(&db, ada.id, bench, HERE).await.unwrap();
        assert_eq!(change, SlotChange { inventory_id: ada.inventory, tool: None });
        assert_eq!(where_is(&db, sickle).await, Some((Some(ada.id), Some(ada.inventory), Some(0), Some(2))));
        assert_eq!(slot_of(&db, bench).await, (None, None));
        assert!(matches!(remove_tool(&db, ada.id, bench, HERE).await, Err(ToolSlotError::NoTool)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_last_craft_breaks_the_station_tool(db: PgPool) {
        let (ada, bench) = setup(&db).await;
        let sickle = item(&db, ada.id, ada.inventory, 0, "stone_sickle", 1, Some(1)).await;
        let fiber = item(&db, ada.id, ada.input, 0, "fiber", 4, None).await;
        install_tool(&db, ada.id, bench, sickle, HERE).await.unwrap();
        let installed = installed_tool(&db, bench, ToolType::Sickle).await.unwrap().unwrap();

        let done = finish_craft(&db, &rope(&ada, fiber, installed)).await.unwrap();

        assert_eq!(done.tool.map(|t| t.durability), Some(0));
        let broken = done.broken.expect("the owner hears it broke");
        assert_eq!((broken.owner_id, broken.container_id, broken.item_name.as_str()), (ada.id, None, "Stone Sickle"));
        assert_eq!(done.station_tool, Some((bench, None)));
        assert_eq!(where_is(&db, sickle).await, None);
        assert_eq!(slot_of(&db, bench).await, (None, None));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_station_tool_taken_out_mid_craft_fails_it(db: PgPool) {
        let (ada, bench) = setup(&db).await;
        let sickle = item(&db, ada.id, ada.inventory, 0, "stone_sickle", 1, Some(5)).await;
        let fiber = item(&db, ada.id, ada.input, 0, "fiber", 4, None).await;
        install_tool(&db, ada.id, bench, sickle, HERE).await.unwrap();
        let installed = installed_tool(&db, bench, ToolType::Sickle).await.unwrap().unwrap();
        remove_tool(&db, ada.id, bench, HERE).await.unwrap();

        let result = finish_craft(&db, &rope(&ada, fiber, installed)).await;

        assert_eq!(result.unwrap_err(), STATION_TOOL_MOVED);
        assert_eq!(where_is(&db, fiber).await.map(|_| ()), Some(()), "the fiber stays");
        assert_eq!(where_is(&db, sickle).await.and_then(|w| w.3), Some(5), "not worn");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn what_a_tool_slot_refuses(db: PgPool) {
        let (ada, bench) = setup(&db).await;
        let bob = player(&db, "bob").await;
        let chest = station(&db, ada.id, "storage_chest").await;
        let sickle = item(&db, ada.id, ada.inventory, 0, "stone_sickle", 1, Some(5)).await;
        let fiber = item(&db, ada.id, ada.inventory, 1, "fiber", 4, None).await;
        let bobs = item(&db, bob.id, bob.inventory, 0, "stone_sickle", 1, Some(5)).await;

        assert!(matches!(install_tool(&db, ada.id, chest, sickle, HERE).await, Err(ToolSlotError::NoToolSlot)));
        assert!(matches!(install_tool(&db, ada.id, bench, fiber, HERE).await, Err(ToolSlotError::NotATool)));
        assert!(matches!(install_tool(&db, ada.id, bench, bobs, HERE).await,
            Err(ToolSlotError::Access(AccessError::ItemNotFound))), "only from your own inventory");
        assert!(matches!(install_tool(&db, ada.id, bench, sickle, Some((500.0, 0.0))).await,
            Err(ToolSlotError::Access(AccessError::TooFar))));
        // Bob may not use the bench, so he can't change its tool
        assert!(matches!(install_tool(&db, bob.id, bench, bobs, HERE).await,
            Err(ToolSlotError::Access(AccessError::Forbidden))));
        install_tool(&db, ada.id, bench, sickle, HERE).await.unwrap();
        assert!(matches!(remove_tool(&db, bob.id, bench, HERE).await,
            Err(ToolSlotError::Access(AccessError::Forbidden))));
        assert_eq!(slot_of(&db, bench).await.0, Some(sickle));
    }

    /// A guest's tool is the guest's, but the station's owner may take it, as from a box (#80)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_station_owner_may_take_a_guests_tool(db: PgPool) {
        let (ada, bench) = setup(&db).await;
        let bob = player(&db, "bob").await;
        let bobs = item(&db, bob.id, bob.inventory, 0, "stone_sickle", 1, Some(5)).await;
        sqlx::query("UPDATE items SET container_id = NULL, slot_index = NULL, owner_id = NULL WHERE id = $1")
            .bind(bobs)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("UPDATE stations SET installed_tool = $2, tool_owner = $3 WHERE id = $1")
            .bind(bench)
            .bind(bobs)
            .bind(bob.id)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(tools_at(&db, &[bench]).await.unwrap()[&bench].owner_id, Some(bob.id));

        remove_tool(&db, ada.id, bench, HERE).await.unwrap();

        assert_eq!(where_is(&db, bobs).await, Some((Some(ada.id), Some(ada.inventory), Some(0), Some(5))));
    }

    /// However a station goes, its tool doesn't linger in no container
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_station_deleted_outright_takes_its_tool(db: PgPool) {
        let (ada, bench) = setup(&db).await;
        let sickle = item(&db, ada.id, ada.inventory, 0, "stone_sickle", 1, Some(5)).await;
        install_tool(&db, ada.id, bench, sickle, HERE).await.unwrap();

        sqlx::query("DELETE FROM stations WHERE id = $1").bind(bench).execute(&db).await.unwrap();

        assert_eq!(where_is(&db, sickle).await, None);
    }
}
