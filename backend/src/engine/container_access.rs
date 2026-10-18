//! Who may use a station, who may put items into, or take them out of, a
//! container, and who may gather the grass on a lot.
//!
//! The grass follows the same plot setting as the stations: a node on an
//! owned lot is the owner's unless its `station_use_level` is public
//! ([`may_gather`], #102).
//!
//! A player's own containers (inventory) carry `owner_id`. Station containers
//! don't: they carry `station_id`, and access follows the station. The owner
//! of the station may use it, and so may anyone the plot's `station_use_level`
//! admits, provided they stand within the station type's `interaction_range`.
//! That one rule ([`check_station_access`]) covers opening a station (WS
//! `open_station`, REST `/stations/{id}/container`), crafting at it (WS
//! `craft_at_station`) and moving items in and out of its box (REST
//! `/inventory/move`, WS `move_item`, through [`check_container_access`]).
//!
//! Items keep their depositor as owner when they go into someone else's box,
//! so a depositor can take back a mistake. The station's owner may take them
//! too, and taking one makes it theirs ([`movable_item`], #80).

use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::tick::GameState;
use crate::models::{Container, Item, PermissionLevel};

/// Why a player can't use a station or a container, or move an item.
#[derive(Debug)]
pub enum AccessError {
    /// No such container, or it belongs to another player's inventory.
    NotFound,
    /// No such item, or one the player may not move.
    ItemNotFound,
    /// No such station.
    StationNotFound,
    /// A station container the player has no right to use.
    Forbidden,
    /// A station container the player may use, but from further than its range.
    TooFar,
    Database(sqlx::Error),
}

impl AccessError {
    pub fn message(&self) -> String {
        match self {
            AccessError::NotFound => "Container not found".into(),
            AccessError::ItemNotFound => "Item not found".into(),
            AccessError::StationNotFound => "Station not found".into(),
            AccessError::Forbidden => "Not your station".into(),
            AccessError::TooFar => "Too far from station".into(),
            AccessError::Database(e) => format!("Database error: {}", e),
        }
    }
}

impl From<sqlx::Error> for AccessError {
    fn from(e: sqlx::Error) -> Self {
        AccessError::Database(e)
    }
}

impl From<AccessError> for crate::error::AppError {
    fn from(e: AccessError) -> Self {
        use crate::error::AppError;
        match e {
            AccessError::NotFound | AccessError::ItemNotFound | AccessError::StationNotFound => AppError::NotFound(e.message()),
            AccessError::Forbidden => AppError::Forbidden(e.message()),
            AccessError::TooFar => AppError::BadRequest(e.message()),
            AccessError::Database(db) => AppError::Database(db),
        }
    }
}

/// What a station container's access depends on.
#[derive(Debug, Clone)]
pub struct StationAccess {
    pub station_owner: Uuid,
    /// Owner of the plot the station stands on, if it stands on one.
    pub plot_owner: Option<Uuid>,
    /// The plot's `station_use_level`, if it stands on one.
    pub station_use_level: Option<String>,
    /// Player-to-station distance, in world units.
    pub distance: f64,
    pub interaction_range: f64,
}

/// Does a plot's `station_use_level` open what stands on it to everyone?
/// "friends" and "guild" admit no one beyond the owners until those systems
/// exist (#80).
fn open_to_all(station_use_level: Option<&str>) -> bool {
    matches!(station_use_level.and_then(PermissionLevel::from_str), Some(PermissionLevel::Public))
}

/// May `player_id` use a station container? Pure, so it can be tested without a database.
pub fn station_access(player_id: Uuid, s: &StationAccess) -> Result<(), AccessError> {
    let permitted = s.station_owner == player_id
        || s.plot_owner == Some(player_id)
        || open_to_all(s.station_use_level.as_deref());
    if !permitted {
        return Err(AccessError::Forbidden);
    }
    if s.distance > s.interaction_range {
        return Err(AccessError::TooFar);
    }
    Ok(())
}

/// Where `player_id` stands now, from the game state, if they're online.
///
/// Use this, not `players.position_x`, for any range check. The tick writes
/// positions to the database only every 100 ticks (10 s), so a player who
/// has just walked up to a station is still "too far" by the row.
pub async fn live_position(game: &RwLock<GameState>, player_id: Uuid) -> Option<(f64, f64)> {
    game.read().await.players.get(&player_id).map(|p| (p.x, p.y))
}

/// Check that `player_id` may use station `station_id`: open it, craft at it,
/// or move items in and out of its box. Returns the station's owner.
/// `at` is where the player stands now ([`live_position`]). When it's `None`,
/// the position comes from their `players` row, which can be up to 10 s old.
pub async fn check_station_access(
    db: &PgPool,
    player_id: Uuid,
    station_id: Uuid,
    at: Option<(f64, f64)>,
) -> Result<Uuid, AccessError> {
    let row: Option<(Uuid, f32, f32, f32, Option<Uuid>, Option<String>, f64, f64)> =
        sqlx::query_as(
            "SELECT s.owner_id, s.position_x, s.position_y, st.interaction_range,
                    p.owner_id, pp.station_use_level, pl.position_x, pl.position_y
             FROM stations s
             JOIN station_types st ON st.id = s.station_type
             JOIN players pl ON pl.id = $2
             LEFT JOIN plots p ON p.id = s.plot_id
             LEFT JOIN plot_permissions pp ON pp.plot_id = s.plot_id
             WHERE s.id = $1",
        )
        .bind(station_id)
        .bind(player_id)
        .fetch_optional(db)
        .await?;
    let Some((station_owner, sx, sy, range, plot_owner, station_use_level, px, py)) = row else {
        return Err(AccessError::StationNotFound);
    };

    let (px, py) = at.unwrap_or((px, py));
    let (dx, dy) = (px - sx as f64, py - sy as f64);
    station_access(
        player_id,
        &StationAccess {
            station_owner,
            plot_owner,
            station_use_level,
            distance: (dx * dx + dy * dy).sqrt(),
            interaction_range: range as f64,
        },
    )?;
    Ok(station_owner)
}

/// Load a container and check that `player_id` may move items into or out of it.
/// `at` is as for [`check_station_access`].
pub async fn check_container_access(
    db: &PgPool,
    player_id: Uuid,
    container_id: Uuid,
    at: Option<(f64, f64)>,
) -> Result<Container, AccessError> {
    let container: Container = sqlx::query_as(
        "SELECT id, container_type, owner_id, station_id, created_at
         FROM containers WHERE id = $1",
    )
    .bind(container_id)
    .fetch_optional(db)
    .await?
    .ok_or(AccessError::NotFound)?;

    if let Some(owner) = container.owner_id {
        return if owner == player_id {
            Ok(container)
        } else {
            Err(AccessError::NotFound)
        };
    }
    let Some(station_id) = container.station_id else {
        return Err(AccessError::NotFound);
    };
    check_station_access(db, player_id, station_id, at).await?;
    Ok(container)
}

/// May `player_id` move an item owned by `item_owner`, held in a container
/// whose station (if any) is owned by `station_owner`? Their own items, yes.
/// Someone else's only when it sits in the player's own station's box: that's
/// the station owner taking a deposit. Pure, so it can be tested without a database.
pub fn may_move_item(player_id: Uuid, item_owner: Option<Uuid>, station_owner: Option<Uuid>) -> bool {
    item_owner == Some(player_id) || station_owner == Some(player_id)
}

/// Load an item `player_id` may move ([`may_move_item`]). Moving it with
/// [`crate::engine::slot_moves::apply_move`] makes the mover its owner, so a
/// station owner who takes a deposit keeps it. This checks ownership only:
/// the caller still checks both containers with [`check_container_access`].
pub async fn movable_item(db: &PgPool, player_id: Uuid, item_id: Uuid) -> Result<Item, AccessError> {
    let row = sqlx::query_as::<_, ItemWithStationOwner>(
        "SELECT i.id, i.item_type, i.quality, i.quantity, i.owner_id, i.container_id,
                i.slot_index, i.created_at, i.durability, i.min_quality, s.owner_id AS station_owner,
                COALESCE((SELECT json_agg(json_build_array(u.quality, u.count) ORDER BY u.quality)
                          FROM item_units u WHERE u.item_id = i.id), '[]'::json) AS units
         FROM items i
         LEFT JOIN containers c ON c.id = i.container_id
         LEFT JOIN stations s ON s.id = c.station_id
         WHERE i.id = $1",
    )
    .bind(item_id)
    .fetch_optional(db)
    .await?
    .map(|r| (r.item, r.station_owner));
    match row {
        Some((item, station_owner)) if may_move_item(player_id, item.owner_id, station_owner) => Ok(item),
        _ => Err(AccessError::ItemNotFound),
    }
}

#[derive(sqlx::FromRow)]
struct ItemWithStationOwner {
    #[sqlx(flatten)]
    item: Item,
    station_owner: Option<Uuid>,
}

/// May `player_id` gather a node on a lot owned by `plot_owner`, whose
/// `station_use_level` is `level`? Grass on a lot belongs to the lot (#102):
/// a node on an unowned lot, or on none, is anyone's, and one on an owned lot
/// is its owner's, or anyone's if the owner opened the plot to the public. It
/// shares `station_use_level` with stations, so one setting means the same for
/// a plot's chests and its grass. Pure, so it can be tested without a database.
pub fn may_gather(player_id: Uuid, plot_owner: Option<Uuid>, level: Option<&str>) -> bool {
    plot_owner.is_none_or(|owner| owner == player_id || open_to_all(level))
}

/// Why `player_id` may not gather `node_id` because of whose lot it stands on,
/// in words the player can act on. `None` means the lot doesn't stand in the
/// way (the other reasons are [`super::extraction_refusal`]'s). Call it before
/// queueing `StartExtraction`, from REST and WS alike.
pub async fn gathering_refusal(
    db: &PgPool,
    game: &RwLock<GameState>,
    player_id: Uuid,
    node_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    let (lot, resource) = {
        let game = game.read().await;
        let Some(node) = game.resource_nodes.get(&node_id) else {
            return Ok(None); // the tick says the node is gone
        };
        let Some(lot) = game.terrain.lot_at(node.x, node.y) else {
            return Ok(None);
        };
        let resource = game
            .resource_types
            .get(&node.resource_type)
            .map_or_else(|| node.resource_type.clone(), |t| t.name.clone());
        (lot.id.clone(), resource)
    };

    let row: Option<(Uuid, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT p.owner_id, pp.station_use_level, pl.name
         FROM plots p
         LEFT JOIN plot_permissions pp ON pp.plot_id = p.id
         LEFT JOIN players pl ON pl.id = p.owner_id
         WHERE p.terrain_id = $1 AND p.owner_id IS NOT NULL",
    )
    .bind(&lot)
    .fetch_optional(db)
    .await?;
    let Some((owner, level, owner_name)) = row else {
        return Ok(None);
    };
    if may_gather(player_id, Some(owner), level.as_deref()) {
        return Ok(None);
    }
    let whose = owner_name.map_or_else(|| "someone else's".to_string(), |name| format!("{name}'s"));
    Ok(Some(format!("This {resource} grows on {whose} lot")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chest(owner: Uuid) -> StationAccess {
        StationAccess {
            station_owner: owner,
            plot_owner: Some(owner),
            station_use_level: Some("owner".into()),
            distance: 10.0,
            interaction_range: 50.0,
        }
    }

    #[test]
    fn own_chest_is_allowed() {
        let me = Uuid::new_v4();
        assert!(station_access(me, &chest(me)).is_ok());
    }

    #[test]
    fn own_chest_off_plot_is_allowed() {
        let me = Uuid::new_v4();
        let s = StationAccess { plot_owner: None, station_use_level: None, ..chest(me) };
        assert!(station_access(me, &s).is_ok());
    }

    #[test]
    fn a_bench_off_plot_is_its_owners_alone() {
        // No plot, so no plot owner and no setting to open it (#187).
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        let s = StationAccess { plot_owner: None, station_use_level: None, ..chest(them) };
        assert!(matches!(station_access(me, &s), Err(AccessError::Forbidden)));
    }

    #[test]
    fn someone_elses_chest_is_refused() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(matches!(station_access(me, &chest(them)), Err(AccessError::Forbidden)));
    }

    #[test]
    fn friends_and_guild_levels_admit_no_one_yet() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        for level in ["friends", "guild", "bogus"] {
            let s = StationAccess { station_use_level: Some(level.into()), ..chest(them) };
            assert!(matches!(station_access(me, &s), Err(AccessError::Forbidden)), "{level}");
        }
    }

    #[test]
    fn public_station_use_admits_anyone() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        let s = StationAccess { station_use_level: Some("public".into()), ..chest(them) };
        assert!(station_access(me, &s).is_ok());
    }

    #[test]
    fn plot_owner_may_use_a_station_on_their_plot() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        let s = StationAccess { plot_owner: Some(me), ..chest(them) };
        assert!(station_access(me, &s).is_ok());
    }

    #[test]
    fn out_of_range_is_refused() {
        let me = Uuid::new_v4();
        let s = StationAccess { distance: 50.1, ..chest(me) };
        assert!(matches!(station_access(me, &s), Err(AccessError::TooFar)));
    }

    #[test]
    fn a_player_may_move_their_own_item_anywhere() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(may_move_item(me, Some(me), None));
        assert!(may_move_item(me, Some(me), Some(them)), "a deposit in someone else's box");
    }

    #[test]
    fn a_station_owner_may_take_a_deposit() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(may_move_item(me, Some(them), Some(me)));
    }

    #[test]
    fn someone_elses_item_elsewhere_is_not_movable() {
        let (me, them, third) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        assert!(!may_move_item(me, Some(them), None), "their inventory");
        assert!(!may_move_item(me, Some(them), Some(third)), "a third player's box");
        assert!(!may_move_item(me, None, None));
    }

    #[test]
    fn grass_off_any_owned_lot_is_anyones() {
        let me = Uuid::new_v4();
        assert!(may_gather(me, None, None), "on no lot, or an unclaimed one");
    }

    #[test]
    fn grass_on_an_owned_lot_is_the_owners() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(may_gather(me, Some(me), Some("owner")));
        assert!(!may_gather(me, Some(them), Some("owner")));
        for level in ["friends", "guild", "bogus"] {
            assert!(!may_gather(me, Some(them), Some(level)), "{level}");
        }
        assert!(!may_gather(me, Some(them), None), "no permissions row");
    }

    #[test]
    fn a_public_lot_shares_its_grass() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(may_gather(me, Some(them), Some("public")));
    }

    #[test]
    fn forbidden_wins_over_too_far() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        let s = StationAccess { distance: 500.0, ..chest(them) };
        assert!(matches!(station_access(me, &s), Err(AccessError::Forbidden)));
    }
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test container_access -- --ignored`.
#[cfg(test)]
mod db_tests {
    use super::*;

    const HERE: Option<(f64, f64)> = Some((0.0, 0.0));

    async fn player(db: &PgPool, name: &str) -> (Uuid, Uuid) {
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
        (id, inventory)
    }

    /// A chest at the origin, owned by `owner`, on a plot of theirs whose
    /// `station_use_level` is `level`. Returns the station and its box.
    async fn chest_on_plot(db: &PgPool, owner: Uuid, level: &str) -> (Uuid, Uuid) {
        let plot: Uuid = sqlx::query_scalar(
            "INSERT INTO plots (zone_id, grid_x, grid_y, world_x, world_y, bounds_min_x,
                                bounds_min_y, bounds_max_x, bounds_max_y, size_category, owner_id)
             VALUES ((SELECT id FROM zones LIMIT 1), 9999, 9999, 0, 0, -10, -10, 10, 10, 'small', $1)
             RETURNING id",
        )
        .bind(owner)
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO plot_permissions (plot_id, station_use_level) VALUES ($1, $2)")
            .bind(plot)
            .bind(level)
            .execute(db)
            .await
            .unwrap();
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality)
             VALUES ($1, 'storage_chest', $2, 0, 0, $3, 50)",
        )
        .bind(station)
        .bind(owner)
        .bind(plot)
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

    async fn put(db: &PgPool, owner: Uuid, container: Uuid, slot: i32) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'fiber', 40, 3, $2, $3, $4) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(owner)
        .bind(container)
        .bind(slot)
        .fetch_one(db)
        .await
        .unwrap()
    }

    /// A game whose only lot is `test_lot`, 20 square round the origin, with a
    /// grass node on it and another off it. Returns the game and both nodes.
    fn game_with_a_lot() -> (RwLock<GameState>, Uuid, Uuid) {
        use crate::models::{NodeState, ResourceNodeState, TerrainConfig, TerrainPlot, WorldGenParams};
        let terrain = TerrainConfig {
            world_size: 8000.0,
            center_x: 4000.0,
            center_y: 4000.0,
            water_rings: Vec::new(),
            roads: Vec::new(),
            terrain_plots: vec![TerrainPlot {
                id: "test_lot".into(),
                corners: [(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)],
                street_facing_edge: (0, 1),
                area: 400.0,
                adjacent_street_id: String::new(),
                zone: "rural".into(),
            }],
            parks: Vec::new(),
        };
        let mut game = GameState::with_terrain(WorldGenParams::default(), terrain);
        let mut node = |x: f64| {
            let id = Uuid::new_v4();
            game.resource_nodes.insert(id, ResourceNodeState {
                id,
                resource_type: "grass".into(),
                x,
                y: 0.0,
                base_quality: 50,
                state: NodeState::Available,
                harvested_by: None,
                regenerates_at_tick: None,
            });
            id
        };
        let (on, off) = (node(0.0), node(50.0));
        (RwLock::new(game), on, off)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn grass_on_a_lot_belongs_to_the_lot(db: PgPool) {
        let (owner, _) = player(&db, "owner").await;
        let (guest, _) = player(&db, "guest").await;
        let (game, on_lot, off_lot) = game_with_a_lot();
        let plot: Uuid = sqlx::query_scalar(
            "INSERT INTO plots (zone_id, grid_x, grid_y, world_x, world_y, bounds_min_x,
                                bounds_min_y, bounds_max_x, bounds_max_y, size_category, terrain_id)
             VALUES ((SELECT id FROM zones LIMIT 1), 9998, 9998, 0, 0, -10, -10, 10, 10, 'small', 'test_lot')
             RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO plot_permissions (plot_id) VALUES ($1)").bind(plot).execute(&db).await.unwrap();
        let refusal = |who| gathering_refusal(&db, &game, who, on_lot);

        // Unclaimed, it's anyone's.
        assert_eq!(refusal(guest).await.unwrap(), None);

        // Claimed, it's the owner's, and the guest is told whose.
        sqlx::query("UPDATE plots SET owner_id = $1 WHERE id = $2").bind(owner).bind(plot).execute(&db).await.unwrap();
        assert_eq!(refusal(owner).await.unwrap(), None);
        assert_eq!(refusal(guest).await.unwrap().as_deref(), Some("This grass grows on owner's lot"));
        assert_eq!(gathering_refusal(&db, &game, guest, off_lot).await.unwrap(), None, "off the lot");

        // Opened to the public, it's anyone's again.
        sqlx::query("UPDATE plot_permissions SET station_use_level = 'public' WHERE plot_id = $1")
            .bind(plot)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(refusal(guest).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_public_plot_opens_its_station_to_a_guest(db: PgPool) {
        let (owner, _) = player(&db, "owner").await;
        let (guest, _) = player(&db, "guest").await;
        let (station, chest) = chest_on_plot(&db, owner, "public").await;

        assert_eq!(check_station_access(&db, guest, station, HERE).await.unwrap(), owner);
        assert!(check_container_access(&db, guest, chest, HERE).await.is_ok());
        assert!(matches!(
            check_station_access(&db, guest, station, Some((500.0, 500.0))).await,
            Err(AccessError::TooFar)
        ));
        assert!(matches!(
            check_station_access(&db, guest, Uuid::new_v4(), HERE).await,
            Err(AccessError::StationNotFound)
        ));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_owner_only_plot_keeps_its_station_to_the_owner(db: PgPool) {
        let (owner, _) = player(&db, "owner").await;
        let (guest, _) = player(&db, "guest").await;
        let (station, _) = chest_on_plot(&db, owner, "owner").await;

        assert!(check_station_access(&db, owner, station, HERE).await.is_ok());
        assert!(matches!(
            check_station_access(&db, guest, station, HERE).await,
            Err(AccessError::Forbidden)
        ));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn deposits_stay_the_depositors_until_the_owner_takes_them(db: PgPool) {
        let (owner, _) = player(&db, "owner").await;
        let (guest, guest_inventory) = player(&db, "guest").await;
        let (_, chest) = chest_on_plot(&db, owner, "public").await;
        let deposit = put(&db, guest, chest, 0).await;
        let owners_own = put(&db, owner, chest, 1).await;
        let in_guest_bag = put(&db, guest, guest_inventory, 0).await;

        // The depositor may take theirs back, and the owner may take it.
        assert!(movable_item(&db, guest, deposit).await.is_ok());
        assert!(movable_item(&db, owner, deposit).await.is_ok());
        // The guest may not take the owner's, and the owner can't reach the guest's bag.
        assert!(matches!(movable_item(&db, guest, owners_own).await, Err(AccessError::ItemNotFound)));
        assert!(matches!(movable_item(&db, owner, in_guest_bag).await, Err(AccessError::ItemNotFound)));
    }
}
