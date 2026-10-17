//! Taking a station down (#88).
//!
//! REST `DELETE /stations/{id}` and WS `remove_station` both come here. In one
//! transaction the station's box must be empty, the kit that placed it goes back
//! into the owner's inventory at the quality it was placed with, and the station,
//! its container and its place on the plot's station count all go. Nothing is
//! destroyed: a station can be moved by taking it down and placing it again.

use sqlx::{PgPool, Postgres, Transaction};
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
