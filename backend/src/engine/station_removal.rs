//! Taking a station down (#88).
//!
//! REST `DELETE /stations/{id}` and WS `remove_station` both come here. In one
//! transaction the station's box must be empty, the kit that placed it goes back
//! into the owner's inventory at the quality it was placed with, with the tool
//! in its tool slot if it holds one (E1.6), and the station,
//! its container and its place on the plot's station count all go. Nothing is
//! destroyed: a station can be moved by taking it down and placing it again.
//!
//! A foreclosed plot's stations come down here too ([`return_plot_stations`],
//! #137), boxes and tools and all: what's in them goes back to the inventories
//! of whoever owns it.

use std::collections::HashMap;

use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;


/// What a removal changed, so the caller can update the game state and tell clients.
#[derive(Debug, Clone)]
pub struct RemovedStation {
    /// The owner's inventory container, which now holds the kit.
    pub inventory_id: Uuid,
#[cfg_attr(not(test), allow(dead_code))] // the tests read it; callers only need the inventory
    pub kit_item_id: Uuid,
    /// The tool that was in its tool slot, now beside the kit (E1.6)
    #[cfg_attr(not(test), allow(dead_code))]
    pub tool_item_id: Option<Uuid>,
}

/// Why a station can't be taken down.
#[derive(Debug)]
pub enum RemoveError {
    NotFound,
    NotYours,
    /// Its box still holds items. They may not all be the owner's (#80), so
    /// they aren't moved for them.
    BoxNotEmpty,
    /// No free inventory slot for the kit, or for the kit and its tool.
    NoRoomForKit,
    Database(sqlx::Error),
}

impl RemoveError {
    pub fn message(&self) -> String {
        match self {
            RemoveError::NotFound => "Station not found".into(),
            RemoveError::NotYours => "Not your station".into(),
            RemoveError::BoxNotEmpty => "Empty the station's box before taking it down".into(),
            RemoveError::NoRoomForKit => "No room in your inventory for the kit and its tool".into(),
            RemoveError::Database(e) => format!("Database error: {}", e),
        }
    }
}

impl From<sqlx::Error> for RemoveError {
    fn from(e: sqlx::Error) -> Self {
        RemoveError::Database(e)
    }
}

impl From<RemoveError> for crate::error::AppError {
    fn from(e: RemoveError) -> Self {
        use crate::error::AppError;
        match e {
            RemoveError::NotFound => AppError::NotFound(e.message()),
            RemoveError::NotYours => AppError::Forbidden(e.message()),
            RemoveError::BoxNotEmpty | RemoveError::NoRoomForKit => {
                AppError::BadRequest(e.message())
            }
            RemoveError::Database(db) => AppError::Database(db),
        }
    }
}

/// Take `station_id` down for `player_id`, who must own it.
pub async fn remove_station(
    db: &PgPool,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<RemovedStation, RemoveError> {
    let mut tx = db.begin().await?;
    let removed = remove_in(&mut tx, player_id, station_id).await?;
    tx.commit().await?;
    Ok(removed)
}

async fn remove_in(
    tx: &mut Transaction<'_, Postgres>,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<RemovedStation, RemoveError> {
    let (station_type, owner_id, plot_id, kit_quality, kit, tool): (String, Uuid, Option<Uuid>, i32, Option<String>, Option<Uuid>) =
        sqlx::query_as(
            "SELECT s.station_type, s.owner_id, s.plot_id, s.kit_quality, st.kit_item, s.installed_tool
             FROM stations s LEFT JOIN station_types st ON st.id = s.station_type
             WHERE s.id = $1 FOR UPDATE OF s",
        )
        .bind(station_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RemoveError::NotFound)?;
    if owner_id != player_id {
        return Err(RemoveError::NotYours);
    }
    let kit = kit.ok_or_else(|| {
        tracing::error!(
            "Station {} has type {} with no kit",
            station_id,
            station_type
        );
        RemoveError::NotFound
    })?;

    // Lock the box before looking in it. A deposit's foreign-key check needs a
    // share lock on this row, so one that started first is counted below, and
    // one that starts now waits and then fails against the deleted container.
    let boxes: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM containers WHERE station_id = $1 FOR UPDATE")
            .bind(station_id)
            .fetch_all(&mut **tx)
            .await?;
    let held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE container_id = ANY($1)")
        .bind(boxes.iter().map(|(id,)| *id).collect::<Vec<_>>())
        .fetch_one(&mut **tx)
        .await?;
    if held > 0 {
        return Err(RemoveError::BoxNotEmpty);
    }

    // Kits and tools don't stack, so each takes the first free inventory slot.
    // The tool comes out to the station's owner, who may take any tool from
    // their own station (`station_tools::remove_tool`).
    let wanted = 1 + tool.is_some() as usize;
    let (inventory_id, slots) = free_slots(tx, player_id, wanted)
        .await?
        .ok_or(RemoveError::NoRoomForKit)?;
    let slot = slots[0];
    if let Some(tool) = tool {
        super::station_tools::give_back(tx, station_id, tool, player_id, inventory_id, slots[1]).await?;
    }

    let kit_item_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
         VALUES ($1, $2, $3, 1, $4, $5, $6)",
    )
    .bind(kit_item_id)
    .bind(kit)
    .bind(kit_quality)
    .bind(player_id)
    .bind(inventory_id)
    .bind(slot)
    .execute(&mut **tx)
    .await?;

    // The container goes with the station (ON DELETE CASCADE), and it's empty.
    sqlx::query("DELETE FROM stations WHERE id = $1")
        .bind(station_id)
        .execute(&mut **tx)
        .await?;
    if let Some(plot_id) = plot_id {
        sqlx::query(
            "UPDATE plots SET station_count = station_count - 1
             WHERE id = $1 AND station_count > 0",
        )
        .bind(plot_id)
        .execute(&mut **tx)
        .await?;
    }

    Ok(RemovedStation {
        inventory_id,
        kit_item_id,
        tool_item_id: tool,
    })
}

/// The first `n` free slots of `player_id`'s inventory, which is locked, or
/// `None` if it has fewer (or they have none).
pub(crate) async fn free_slots(
    conn: &mut PgConnection,
    player_id: Uuid,
    n: usize,
) -> Result<Option<(Uuid, Vec<i32>)>, sqlx::Error> {
    let found: Option<(Uuid, i32)> = sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.owner_id = $1 AND c.container_type = 'player_inventory'
         LIMIT 1 FOR UPDATE OF c",
    )
    .bind(player_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((inventory_id, slot_count)) = found else {
        return Ok(None);
    };
    let used: Vec<i32> = sqlx::query_scalar(
        "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL",
    )
    .bind(inventory_id)
    .fetch_all(&mut *conn)
    .await?;
    let free: Vec<i32> = (0..slot_count).filter(|s| !used.contains(s)).take(n).collect();
    Ok((free.len() == n).then_some((inventory_id, free)))
}

/// What giving a plot's stations back did (#137).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReturnedStations {
    /// `(player, inventory)` for each inventory that took something, in the order they first did
    pub inventories: Vec<(Uuid, Uuid)>,
    /// What found no room, as `(item_type, quantity)`. It's still in the
    /// stations, and goes when they do. Keeping it somewhere is #141.
    pub lost: Vec<(String, i32)>,
}

/// A player's inventory, locked, as things are put into it
struct Inventory {
    id: Uuid,
    free: Vec<i32>,
    /// The stacks of each stackable item type, in slot order, with how many units each holds
    stacks: HashMap<String, Vec<(Uuid, i64)>>,
}

/// Empty every station on `plot_id` into inventories, before a foreclosure
/// releases the plot (#137). Each station's kit goes back to the station's
/// owner at the quality it was placed with, as [`remove_station`] does. Each
/// item in its boxes goes back to whoever owns the item: a guest's deposit is
/// the guest's (#80), and an item with no owner is the station owner's. A
/// stackable item joins the first stack of its type, grade and min grade
/// (`rules::stacking::auto_stacks_with`, plan D2), at the two stacks' quality
/// averaged by quantity; anything else takes the first free slot. A tool in a
/// station's tool slot goes back to whoever put it there (E1.6), or the
/// station owner if they're gone. Kits go first, so a station is the last
/// thing lost, then tools. A tool that finds no room goes with its station.
/// The stations are left, empty or holding only what didn't fit, for
/// `release_plot` to delete.
pub async fn return_plot_stations(
    tx: &mut PgConnection,
    plot_id: Uuid,
) -> Result<ReturnedStations, sqlx::Error> {
    let stations: Vec<(Uuid, String, Uuid, i32, Option<String>, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT s.id, s.station_type, s.owner_id, s.kit_quality, st.kit_item, s.installed_tool, s.tool_owner
         FROM stations s LEFT JOIN station_types st ON st.id = s.station_type
         WHERE s.plot_id = $1 ORDER BY s.placed_at, s.id FOR UPDATE OF s",
    )
    .bind(plot_id)
    .fetch_all(&mut *tx)
    .await?;
    if stations.is_empty() {
        return Ok(ReturnedStations::default());
    }
    let station_ids: Vec<Uuid> = stations.iter().map(|s| s.0).collect();
    let station_owner: HashMap<Uuid, Uuid> = stations.iter().map(|s| (s.0, s.2)).collect();
    // The boxes are locked too, so a deposit racing this waits (see `remove_in`)
    let contents: Vec<(Uuid, String, i32, Option<Uuid>, Uuid, bool)> = sqlx::query_as(
        "SELECT i.id, i.item_type, i.quantity, i.owner_id, c.station_id, it.stackable
         FROM items i
         JOIN containers c ON i.container_id = c.id
         JOIN item_types it ON it.id = i.item_type
         WHERE c.station_id = ANY($1)
         ORDER BY c.station_id, i.slot_index NULLS LAST, i.id
         FOR UPDATE OF i, c",
    )
    .bind(&station_ids)
    .fetch_all(&mut *tx)
    .await?;

    let mut returned = ReturnedStations::default();
    let mut inventories: HashMap<Uuid, Option<Inventory>> = HashMap::new();

    for (station_id, station_type, owner_id, kit_quality, kit, _, _) in &stations {
        let Some(kit) = kit else {
            tracing::error!("Station {} has type {} with no kit", station_id, station_type);
            continue;
        };
        let slot = match inventory(tx, &mut inventories, *owner_id).await? {
            Some(inv) if !inv.free.is_empty() => Some((inv.id, inv.free.remove(0))),
            _ => None,
        };
        let Some((inventory_id, slot)) = slot else {
            returned.lost.push((kit.to_string(), 1));
            continue;
        };
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, $2, $3, 1, $4, $5, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(kit)
        .bind(kit_quality)
        .bind(owner_id)
        .bind(inventory_id)
        .bind(slot)
        .execute(&mut *tx)
        .await?;
        took(&mut returned, *owner_id, inventory_id);
    }

    for (station_id, _, owner_id, _, _, tool, tool_owner) in &stations {
        let Some(tool) = tool else { continue };
        let player = tool_owner.unwrap_or(*owner_id);
        let slot = match inventory(tx, &mut inventories, player).await? {
            Some(inv) if !inv.free.is_empty() => Some((inv.id, inv.free.remove(0))),
            _ => None,
        };
        let Some((inventory_id, slot)) = slot else {
            let item_type: String = sqlx::query_scalar("SELECT item_type FROM items WHERE id = $1")
                .bind(tool)
                .fetch_one(&mut *tx)
                .await?;
            returned.lost.push((item_type, 1));
            continue;
        };
        super::station_tools::give_back(tx, *station_id, *tool, player, inventory_id, slot).await?;
        took(&mut returned, player, inventory_id);
    }

    for (item_id, item_type, quantity, item_owner, station_id, stackable) in contents {
        let player = item_owner.unwrap_or(station_owner[&station_id]);
        let Some(inv) = inventory(tx, &mut inventories, player).await? else {
            returned.lost.push((item_type, quantity));
            continue;
        };
        let inventory_id = inv.id;
        // Onto the first stack of its type with room, whatever its grade: every
        // unit keeps its quality (D2, E1.8)
        let joins = inv.stacks.get_mut(&item_type).filter(|_| stackable).and_then(|stacks| {
            stacks.iter_mut().find(|(_, held)| *held + quantity as i64 <= i32::MAX as i64)
        });
        if let Some((stack_id, held)) = joins {
            *held += quantity as i64;
            super::units::merge_into(&mut *tx, item_id, *stack_id).await?;
        } else if !inv.free.is_empty() {
            let slot = inv.free.remove(0);
            if stackable {
                inv.stacks.entry(item_type).or_default().push((item_id, quantity as i64));
            }
            sqlx::query(
                "UPDATE items SET container_id = $1, slot_index = $2, owner_id = $3 WHERE id = $4",
            )
            .bind(inventory_id)
            .bind(slot)
            .bind(player)
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
        } else {
            returned.lost.push((item_type, quantity));
            continue;
        }
        took(&mut returned, player, inventory_id);
    }
    Ok(returned)
}

fn took(returned: &mut ReturnedStations, player: Uuid, inventory: Uuid) {
    if !returned.inventories.contains(&(player, inventory)) {
        returned.inventories.push((player, inventory));
    }
}

/// `player`'s inventory, locked and loaded the first time it's asked for.
/// `None` if they have none.
async fn inventory<'a>(
    tx: &mut PgConnection,
    cache: &'a mut HashMap<Uuid, Option<Inventory>>,
    player: Uuid,
) -> Result<Option<&'a mut Inventory>, sqlx::Error> {
    if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(player) {
        let found: Option<(Uuid, i32)> = sqlx::query_as(
            "SELECT c.id, ct.slot_count FROM containers c
             JOIN container_types ct ON c.container_type = ct.id
             WHERE c.owner_id = $1 AND c.container_type = 'player_inventory'
             LIMIT 1 FOR UPDATE OF c",
        )
        .bind(player)
        .fetch_optional(&mut *tx)
        .await?;
        let loaded = match found {
            None => None,
            Some((id, slot_count)) => {
                let held: Vec<(Uuid, String, i32, Option<i32>, bool)> = sqlx::query_as(
                    "SELECT i.id, i.item_type, i.quantity, i.slot_index, it.stackable
                     FROM items i JOIN item_types it ON it.id = i.item_type
                     WHERE i.container_id = $1 ORDER BY i.slot_index NULLS LAST, i.id
                     FOR UPDATE OF i",
                )
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
                let used: Vec<i32> = held.iter().filter_map(|h| h.3).collect();
                let mut stacks: HashMap<String, Vec<(Uuid, i64)>> = HashMap::new();
                for (item, kind, quantity, _, stackable) in held {
                    if stackable {
                        stacks.entry(kind).or_default().push((item, quantity as i64));
                    }
                }
                let free = (0..slot_count).filter(|s| !used.contains(s)).collect();
                Some(Inventory { id, free, stacks })
            }
        };
        e.insert(loaded);
    }
    Ok(cache.get_mut(&player).and_then(Option::as_mut))
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh database that
/// `sqlx::test` creates from the migrations and drops afterwards. Run them with
/// `cargo test station_removal -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;

    struct Owner {
        id: Uuid,
        inventory: Uuid,
    }

    async fn player(db: &PgPool, name: &str) -> Owner {
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
        let inventory = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)",
        )
        .bind(inventory)
        .bind(id)
        .execute(db)
        .await
        .unwrap();
        Owner { id, inventory }
    }

    /// A storage chest owned by `owner`, off any plot, placed from a kit of `quality`.
    /// Returns the station and its box.
    async fn chest(db: &PgPool, owner: Uuid, quality: i32) -> (Uuid, Uuid) {
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
             VALUES ($1, 'storage_chest', $2, 0, 0, $3)",
        )
        .bind(station)
        .bind(owner)
        .bind(quality)
        .execute(db)
        .await
        .unwrap();
        let container = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2)",
        )
        .bind(container)
        .bind(station)
        .execute(db)
        .await
        .unwrap();
        (station, container)
    }

    async fn put(db: &PgPool, owner: Uuid, container: Uuid, slot: i32) {
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'fiber', 40, 3, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
    }

    async fn station_exists(db: &PgPool, id: Uuid) -> bool {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stations WHERE id = $1)")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_empty_station_comes_down_and_its_kit_comes_back(db: PgPool) {
        let me = player(&db, "builder").await;
        put(&db, me.id, me.inventory, 0).await;
        let (station, container) = chest(&db, me.id, 73).await;

        let removed = remove_station(&db, me.id, station).await.unwrap();

        assert_eq!(removed.inventory_id, me.inventory);
        let (kind, quality, slot): (String, i32, i32) = sqlx::query_as(
            "SELECT item_type, quality, slot_index FROM items WHERE id = $1 AND owner_id = $2 AND container_id = $3",
        )
        .bind(removed.kit_item_id)
        .bind(me.id)
        .bind(me.inventory)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!((kind.as_str(), quality, slot), ("chest_kit", 73, 1));
        assert!(!station_exists(&db, station).await);
        let boxes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM containers WHERE id = $1")
            .bind(container)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(boxes, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_station_with_things_in_its_box_stays_up(db: PgPool) {
        let me = player(&db, "builder").await;
        let guest = player(&db, "guest").await;
        let (station, container) = chest(&db, me.id, 50).await;
        put(&db, guest.id, container, 4).await;

        let err = remove_station(&db, me.id, station).await.unwrap_err();

        assert!(matches!(err, RemoveError::BoxNotEmpty), "{err:?}");
        assert!(station_exists(&db, station).await);
        let held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE container_id = $1")
            .bind(container)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(held, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_inventory_keeps_the_station_up(db: PgPool) {
        let me = player(&db, "builder").await;
        let slots: i32 = sqlx::query_scalar(
            "SELECT slot_count FROM container_types WHERE id = 'player_inventory'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        for slot in 0..slots {
            put(&db, me.id, me.inventory, slot).await;
        }
        let (station, _) = chest(&db, me.id, 50).await;

        let err = remove_station(&db, me.id, station).await.unwrap_err();

        assert!(matches!(err, RemoveError::NoRoomForKit), "{err:?}");
        assert!(station_exists(&db, station).await);
    }

    /// A workbench whose tool slot holds a stone sickle with 7 uses left, put
    /// there by `tool_owner`. Returns the station and the sickle.
    async fn bench_with_tool(db: &PgPool, owner: Uuid, tool_owner: Uuid) -> (Uuid, Uuid) {
        crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
             VALUES ($1, 'workbench', $2, 0, 0, 61)",
        )
        .bind(station)
        .bind(owner)
        .execute(db)
        .await
        .unwrap();
        let sickle: Uuid = sqlx::query_scalar(
            "INSERT INTO items (id, item_type, quality, quantity, durability)
             VALUES ($1, 'stone_sickle', 50, 1, 7) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query("UPDATE stations SET installed_tool = $2, tool_owner = $3 WHERE id = $1")
            .bind(station)
            .bind(sickle)
            .bind(tool_owner)
            .execute(db)
            .await
            .unwrap();
        (station, sickle)
    }

    /// The tool comes back with the kit, to the station's owner (E1.6)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_station_comes_down_with_its_tool(db: PgPool) {
        let me = player(&db, "builder").await;
        let guest = player(&db, "guest").await;
        let (station, sickle) = bench_with_tool(&db, me.id, guest.id).await;

        let removed = remove_station(&db, me.id, station).await.unwrap();

        assert_eq!(removed.tool_item_id, Some(sickle));
        let held: Vec<(Uuid, String, i32, Option<i32>)> = sqlx::query_as(
            "SELECT owner_id, item_type, slot_index, durability FROM items
             WHERE container_id = $1 ORDER BY slot_index",
        )
        .bind(me.inventory)
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(
            held,
            vec![
                (me.id, "workbench_kit".to_string(), 0, None),
                (me.id, "stone_sickle".to_string(), 1, Some(7)),
            ]
        );
        assert!(!station_exists(&db, station).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn no_room_for_the_kit_and_its_tool_keeps_the_station_up(db: PgPool) {
        let me = player(&db, "builder").await;
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'player_inventory'")
            .fetch_one(&db)
            .await
            .unwrap();
        for slot in 1..slots {
            put(&db, me.id, me.inventory, slot).await;
        }
        let (station, sickle) = bench_with_tool(&db, me.id, me.id).await;

        let err = remove_station(&db, me.id, station).await.unwrap_err();

        assert!(matches!(err, RemoveError::NoRoomForKit), "{err:?}");
        assert!(station_exists(&db, station).await);
        let installed: Option<Uuid> = sqlx::query_scalar("SELECT installed_tool FROM stations WHERE id = $1")
            .bind(station)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(installed, Some(sickle));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn only_the_owner_takes_a_station_down(db: PgPool) {
        let me = player(&db, "builder").await;
        let other = player(&db, "passer-by").await;
        let (station, _) = chest(&db, me.id, 50).await;

        let err = remove_station(&db, other.id, station).await.unwrap_err();

        assert!(matches!(err, RemoveError::NotYours), "{err:?}");
        assert!(station_exists(&db, station).await);
        let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE owner_id = $1")
            .bind(other.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(items, 0);
    }

    /// A foreclosure's returns join the first stack of their type, whatever its
    /// grade, and every unit keeps its quality (D2, E1.8)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn returns_join_the_first_stack_and_keep_every_unit(db: PgPool) {
        use crate::engine::units::test_units::{all_held, stack_of, units};
        use capitol_protocol::rules::stacking::Units;
        let ada = player(&db, "ada").await;
        let plot: Uuid = sqlx::query_scalar(
            "INSERT INTO plots (zone_id, grid_x, grid_y, world_x, world_y, bounds_min_x,
                                bounds_min_y, bounds_max_x, bounds_max_y, size_category, owner_id)
             VALUES ((SELECT id FROM zones LIMIT 1), 9999, 9999, 0, 0, -10, -10, 10, 10, 'small', $1)
             RETURNING id",
        )
        .bind(ada.id)
        .fetch_one(&db)
        .await
        .unwrap();
        let (station, chest_box) = chest(&db, ada.id, 50).await;
        sqlx::query("UPDATE stations SET plot_id = $1 WHERE id = $2")
            .bind(plot)
            .bind(station)
            .execute(&db)
            .await
            .unwrap();
        let first = stack_of(&db, ada.id, ada.inventory, 0, "fiber", &Units::uniform(5, 95)).await;
        let second = stack_of(&db, ada.id, ada.inventory, 1, "fiber", &Units::uniform(4, 60)).await;
        stack_of(&db, ada.id, chest_box, 0, "fiber", &Units::from_pairs([(40, 1), (92, 1)])).await;
        stack_of(&db, ada.id, chest_box, 1, "fiber", &Units::uniform(3, 65)).await;
        stack_of(&db, ada.id, chest_box, 2, "fiber", &Units::uniform(1, 93)).await;
        let before = all_held(&db, ada.id, "fiber").await;

        let mut tx = db.begin().await.unwrap();
        let returned = return_plot_stations(&mut tx, plot).await.unwrap();
        tx.commit().await.unwrap();

        assert_eq!(returned.lost, vec![]);
        // Everything joined the first stack; the second is as it was
        assert_eq!(
            units(&db, first).await,
            Units::from_pairs([(40, 1), (65, 3), (92, 1), (93, 1), (95, 5)]),
        );
        assert_eq!(units(&db, second).await, Units::uniform(4, 60));
        assert_eq!(all_held(&db, ada.id, "fiber").await, before, "no unit lost its quality");
    }
}
