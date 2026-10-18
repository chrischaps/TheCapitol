//! Taking a station down (#88).
//!
//! REST `DELETE /stations/{id}` and WS `remove_station` both come here. In one
//! transaction the station's box must be empty, the kit that placed it goes back
//! into the owner's inventory at the quality it was placed with, and the station,
//! its container and its place on the plot's station count all go. Nothing is
//! destroyed: a station can be moved by taking it down and placing it again.
//!
//! A foreclosed plot's stations come down here too ([`return_plot_stations`],
//! #137), boxes and all: what's in them goes back to the inventories of
//! whoever owns it.

use std::collections::HashMap;

use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

/// What a removal changed, so the caller can update the game state and tell clients.
#[derive(Debug, Clone)]
pub struct RemovedStation {
    /// The owner's inventory container, which now holds the kit.
    pub inventory_id: Uuid,
    pub kit_item_id: Uuid,
}

/// Why a station can't be taken down.
#[derive(Debug)]
pub enum RemoveError {
    NotFound,
    NotYours,
    /// Its box still holds items. They may not all be the owner's (#80), so
    /// they aren't moved for them.
    BoxNotEmpty,
    /// No free inventory slot for the kit.
    NoRoomForKit,
    Database(sqlx::Error),
}

impl RemoveError {
    pub fn message(&self) -> String {
        match self {
            RemoveError::NotFound => "Station not found".into(),
            RemoveError::NotYours => "Not your station".into(),
            RemoveError::BoxNotEmpty => "Empty the station's box before taking it down".into(),
            RemoveError::NoRoomForKit => "No room in your inventory for the kit".into(),
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
    let (station_type, owner_id, plot_id, kit_quality): (String, Uuid, Option<Uuid>, i32) =
        sqlx::query_as(
            "SELECT station_type, owner_id, plot_id, kit_quality
             FROM stations WHERE id = $1 FOR UPDATE",
        )
        .bind(station_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RemoveError::NotFound)?;
    if owner_id != player_id {
        return Err(RemoveError::NotYours);
    }
    let kit = capitol_protocol::rules::station_kit_item(&station_type).ok_or_else(|| {
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

    // Kits don't stack, so the kit takes the first free inventory slot.
    let (inventory_id, slot_count): (Uuid, i32) = sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.owner_id = $1 AND c.container_type = 'player_inventory'
         LIMIT 1 FOR UPDATE OF c",
    )
    .bind(player_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RemoveError::NoRoomForKit)?;
    let used: Vec<i32> = sqlx::query_scalar(
        "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL",
    )
    .bind(inventory_id)
    .fetch_all(&mut **tx)
    .await?;
    let slot = (0..slot_count)
        .find(|s| !used.contains(s))
        .ok_or(RemoveError::NoRoomForKit)?;

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
    })
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
    /// The first stack of each stackable item type, as `(item, quantity, quality)`
    stacks: HashMap<String, (Uuid, i32, i32)>,
}

/// Empty every station on `plot_id` into inventories, before a foreclosure
/// releases the plot (#137). Each station's kit goes back to the station's
/// owner at the quality it was placed with, as [`remove_station`] does. Each
/// item in its boxes goes back to whoever owns the item: a guest's deposit is
/// the guest's (#80), and an item with no owner is the station owner's. A
/// stackable item joins the first stack of its type, at the weighted quality
/// [`merge_items`](super::inventory::merge_items) gives; anything else takes
/// the first free slot. Kits go first, so a station is the last thing lost.
/// The stations are left, empty or holding only what didn't fit, for
/// `release_plot` to delete.
pub async fn return_plot_stations(
    tx: &mut PgConnection,
    plot_id: Uuid,
) -> Result<ReturnedStations, sqlx::Error> {
    let stations: Vec<(Uuid, String, Uuid, i32)> = sqlx::query_as(
        "SELECT id, station_type, owner_id, kit_quality FROM stations
         WHERE plot_id = $1 ORDER BY placed_at, id FOR UPDATE",
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
    let contents: Vec<(Uuid, String, i32, i32, Option<Uuid>, Uuid, bool)> = sqlx::query_as(
        "SELECT i.id, i.item_type, i.quality, i.quantity, i.owner_id, c.station_id, it.stackable
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

    for (station_id, station_type, owner_id, kit_quality) in &stations {
        let Some(kit) = capitol_protocol::rules::station_kit_item(station_type) else {
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

    for (item_id, item_type, quality, quantity, item_owner, station_id, stackable) in contents {
        let player = item_owner.unwrap_or(station_owner[&station_id]);
        let Some(inv) = inventory(tx, &mut inventories, player).await? else {
            returned.lost.push((item_type, quantity));
            continue;
        };
        let inventory_id = inv.id;
        if let Some((stack_id, qty, q)) = inv.stacks.get_mut(&item_type).filter(|_| stackable) {
            let total = *qty + quantity;
            *q = (*q * *qty + quality * quantity) / total;
            *qty = total;
            sqlx::query("UPDATE items SET quantity = $1, quality = $2 WHERE id = $3")
                .bind(*qty)
                .bind(*q)
                .bind(*stack_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM items WHERE id = $1")
                .bind(item_id)
                .execute(&mut *tx)
                .await?;
        } else if !inv.free.is_empty() {
            let slot = inv.free.remove(0);
            if stackable {
                inv.stacks.insert(item_type, (item_id, quantity, quality));
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
    if !cache.contains_key(&player) {
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
                let held: Vec<(Uuid, String, i32, i32, Option<i32>, bool)> = sqlx::query_as(
                    "SELECT i.id, i.item_type, i.quantity, i.quality, i.slot_index, it.stackable
                     FROM items i JOIN item_types it ON it.id = i.item_type
                     WHERE i.container_id = $1 ORDER BY i.slot_index NULLS LAST, i.id
                     FOR UPDATE OF i",
                )
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
                let used: Vec<i32> = held.iter().filter_map(|h| h.4).collect();
                let mut stacks = HashMap::new();
                for (item, kind, qty, quality, _, stackable) in held {
                    if stackable {
                        stacks.entry(kind).or_insert((item, qty, quality));
                    }
                }
                let free = (0..slot_count).filter(|s| !used.contains(s)).collect();
                Some(Inventory { id, free, stacks })
            }
        };
        cache.insert(player, loaded);
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
}
