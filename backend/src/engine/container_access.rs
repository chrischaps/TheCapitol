//! Who may use a station, who may put items into, or take them out of, a
//! container, and who may gather the grass on a lot.
//!
//! The grass follows the same plot setting as the stations: a node on an
//! owned lot is the owner's unless its `station_use_level` opens it
//! ([`may_gather`], #102).
//!
//! A `station_use_level` of `public` admits anyone, and `guild` (S4.3) the
//! members of the plot's guild, if it's a guild's hall, or of its owner's
//! guild ([`admits`]). Membership is read at each use, so leaving a guild
//! closes its plots to you at once. `friends` admits no one beyond the
//! owners: there are no friends yet.
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
//!
//! Two of a player's own containers have rules of their own for moves and
//! splits ([`move_refusal`]): nothing goes into the impound (#141), and the
//! market box is used only at the Exchange hall, and only to take things out
//! (#244).
//!
//! A worker on a wage shift (Commons D7) may open the station it's at and
//! craft the offer's recipe there ([`check_station_use`]), or gather the
//! offer's resource on its lot ([`gathering_terms`]), and nothing more: they
//! can't move things in or out of its box, or put a tool in it.

use capitol_protocol::rules::market::at_the_hall;
use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::impound::IMPOUND;
use super::market::MARKET_BOX;
use super::tick::GameState;
use super::wages::{self, ShiftWork};
use crate::models::{Container, Item, LandmarkKind, PermissionLevel};

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
    /// The player is in the plot's guild, or its owner's (S4.3)
    pub in_guild: bool,
    /// Player-to-station distance, in world units.
    pub distance: f64,
    pub interaction_range: f64,
}

/// Does a plot's `station_use_level` open what stands on it to someone who
/// isn't its owner? `public` to anyone, and `guild` to someone `in_guild`:
/// in the plot's guild, or its owner's (S4.3). `friends`, `owner` and
/// anything else admit no one more.
pub fn admits(station_use_level: Option<&str>, in_guild: bool) -> bool {
    match station_use_level.and_then(PermissionLevel::from_str) {
        Some(PermissionLevel::Public) => true,
        Some(PermissionLevel::Guild) => in_guild,
        Some(PermissionLevel::Friends | PermissionLevel::Owner) | None => false,
    }
}

/// SQL for [`StationAccess::in_guild`] over `plots p`: whether the player
/// `$2` is in the plot's guild, or in its owner's.
const IN_GUILD: &str = "EXISTS (SELECT 1 FROM guild_members me
            WHERE me.player_id = $2
              AND (me.guild_id = p.guild_id
                   OR me.guild_id = (SELECT o.guild_id FROM guild_members o WHERE o.player_id = p.owner_id)))";

/// May `player_id` use a station container? Pure, so it can be tested without a database.
pub fn station_access(player_id: Uuid, s: &StationAccess) -> Result<(), AccessError> {
    let permitted = s.station_owner == player_id
        || s.plot_owner == Some(player_id)
        || admits(s.station_use_level.as_deref(), s.in_guild);
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
    let access = load_station_access(db, player_id, station_id, at).await?;
    station_access(player_id, &access)?;
    Ok(access.station_owner)
}

/// How a player may use a station: as [`station_access`] admits them, or as
/// a worker on a wage shift there (Commons D7), to open it and craft the
/// offer's recipe alone (`wages::recipe_refusal`).
#[derive(Debug, Clone, PartialEq)]
pub enum StationUse {
    /// Its owner, its plot's owner, or anyone the plot admits
    Free { owner: Uuid },
    /// On this shift
    Shift(ShiftWork),
}

/// [`check_station_access`], or a running wage shift at the station: the
/// rule for opening it and crafting at it. Moving things in or out of its
/// box, and its tool slot, stay [`check_station_access`]'s. On a shift
/// there, the shift wins, so a public station's worker is still paid for the
/// offer's recipe.
pub async fn check_station_use(
    db: &PgPool,
    player_id: Uuid,
    station_id: Uuid,
    at: Option<(f64, f64)>,
) -> Result<StationUse, AccessError> {
    let access = load_station_access(db, player_id, station_id, at).await?;
    if let Some(work) = wages::shift_at_station(db, player_id, station_id).await? {
        if access.distance > access.interaction_range {
            return Err(AccessError::TooFar);
        }
        return Ok(StationUse::Shift(work));
    }
    station_access(player_id, &access)?;
    Ok(StationUse::Free { owner: access.station_owner })
}

/// What [`station_access`] decides `player_id`'s use of `station_id` on,
/// standing at `at` (or at their saved position).
async fn load_station_access(
    db: &PgPool,
    player_id: Uuid,
    station_id: Uuid,
    at: Option<(f64, f64)>,
) -> Result<StationAccess, AccessError> {
    type Row = (Uuid, f32, f32, f32, Option<Uuid>, Option<String>, f64, f64, bool);
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT s.owner_id, s.position_x, s.position_y, st.interaction_range,
                p.owner_id, pp.station_use_level, pl.position_x, pl.position_y,
                COALESCE({IN_GUILD}, FALSE)
         FROM stations s
         JOIN station_types st ON st.id = s.station_type
         JOIN players pl ON pl.id = $2
         LEFT JOIN plots p ON p.id = s.plot_id
         LEFT JOIN plot_permissions pp ON pp.plot_id = s.plot_id
         WHERE s.id = $1"
    ))
    .bind(station_id)
    .bind(player_id)
    .fetch_optional(db)
    .await?;
    let Some((station_owner, sx, sy, range, plot_owner, station_use_level, px, py, in_guild)) = row else {
        return Err(AccessError::StationNotFound);
    };

    let (px, py) = at.unwrap_or((px, py));
    let (dx, dy) = (px - sx as f64, py - sy as f64);
    Ok(StationAccess {
        station_owner,
        plot_owner,
        station_use_level,
        in_guild,
        distance: (dx * dx + dy * dy).sqrt(),
        interaction_range: range as f64,
    })
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

/// Why a move or a split may not touch the impound (#141), if it may not.
/// Nothing goes into it, so its stacks are only ever taken out. From it a
/// stack goes only to an empty slot, since a swap would put the other stack
/// back in. `source` and `target` are container types, and `target_held`
/// whether the target slot holds something. Pure, so it can be tested
/// without a database.
pub fn impound_refusal(source: Option<&str>, target: &str, target_held: bool) -> Option<&'static str> {
    if target == IMPOUND {
        Some("Nothing goes into the impound: things only come out")
    } else if source == Some(IMPOUND) && target_held {
        Some("Take it out of the impound to an empty slot")
    } else {
        None
    }
}

/// Why a move or a split may not touch the market box (#244), if it may not.
/// Goods wait in it at the Exchange hall until their owner comes (D6), so
/// it's used only there, as `POST /market/box/collect` is. Nothing goes into
/// it, either: it's where the market delivers fills and sends back what's
/// cancelled, and a box its owner had filled would refuse those
/// (`MarketBoxFull`). So, as from the impound, a stack leaves it only for an
/// empty slot, and shuffling within it is fine. `at_the_hall` is whether the
/// player stands there ([`at_the_market_hall`]). Pure, so it can be tested
/// without a database.
pub fn market_box_refusal(
    source: Option<&str>,
    target: &str,
    target_held: bool,
    at_the_hall: bool,
) -> Option<&'static str> {
    let (from_box, into_box) = (source == Some(MARKET_BOX), target == MARKET_BOX);
    if into_box && !from_box {
        Some("Nothing goes into the market box: it holds what the market sends you")
    } else if from_box && !at_the_hall {
        Some("Your market box is at the Exchange hall: take things out of it there")
    } else if from_box && !into_box && target_held {
        Some("Take it out of the market box to an empty slot")
    } else {
        None
    }
}

/// Whether `player_id` stands close enough to the Exchange hall to use their
/// market box: at `at` ([`live_position`]) if they're online, else at their
/// saved position. A world without a hall (an admin's custom one) is the hall
/// everywhere, as it is for placing orders.
pub async fn at_the_market_hall(
    game: &RwLock<GameState>,
    db: &PgPool,
    player_id: Uuid,
    at: Option<(f64, f64)>,
) -> Result<bool, sqlx::Error> {
    let hall = game.read().await.terrain.landmark(LandmarkKind::ExchangeHall).cloned();
    let Some(hall) = hall else { return Ok(true) };
    let (x, y) = match at {
        Some(at) => at,
        None => {
            sqlx::query_as("SELECT position_x, position_y FROM players WHERE id = $1")
                .bind(player_id)
                .fetch_one(db)
                .await?
        }
    };
    Ok(at_the_hall(&hall, x, y))
}

/// Why a move or a split from a container of type `source` to one of type
/// `target` is refused, if it is: the impound's rule ([`impound_refusal`])
/// and the market box's ([`market_box_refusal`]). `target_held` is whether
/// the target slot holds something (a split's never does), and `at` is as for
/// [`at_the_market_hall`], which runs only when the market box is involved.
pub async fn move_refusal(
    game: &RwLock<GameState>,
    db: &PgPool,
    player_id: Uuid,
    at: Option<(f64, f64)>,
    (source, target): (Option<&str>, &str),
    target_held: bool,
) -> Result<Option<&'static str>, sqlx::Error> {
    if let Some(why) = impound_refusal(source, target, target_held) {
        return Ok(Some(why));
    }
    if source != Some(MARKET_BOX) && target != MARKET_BOX {
        return Ok(None);
    }
    let at_the_hall = at_the_market_hall(game, db, player_id, at).await?;
    Ok(market_box_refusal(source, target, target_held, at_the_hall))
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
/// is its owner's, or whoever the owner opened the plot to ([`admits`]):
/// anyone, or the guild's members, `in_guild` being whether they're one. It
/// shares `station_use_level` with stations, so one setting means the same for
/// a plot's chests and its grass. Pure, so it can be tested without a database.
pub fn may_gather(player_id: Uuid, plot_owner: Option<Uuid>, level: Option<&str>, in_guild: bool) -> bool {
    plot_owner.is_none_or(|owner| owner == player_id || admits(level, in_guild))
}

/// Why `player_id` may not gather `node_id` because of whose lot it stands on,
/// in words the player can act on. `None` means the lot doesn't stand in the
/// way (the other reasons are [`super::extraction_refusal`]'s).
#[cfg(test)]
pub async fn gathering_refusal(
    db: &PgPool,
    game: &RwLock<GameState>,
    player_id: Uuid,
    node_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    Ok(gathering_terms(db, game, player_id, node_id).await?.err())
}

/// On what terms `player_id` may gather `node_id`, as far as whose lot it
/// stands on goes: `Ok(None)` as anyone may (their own lot, an open one, or
/// none), `Ok(Some(work))` for wages, on a running shift for this resource
/// on this lot (Commons D7), or `Err` with why not ([`gathering_refusal`]'s
/// words). Call it before queueing `StartExtraction`, from REST and WS
/// alike, and queue the work with it.
pub async fn gathering_terms(
    db: &PgPool,
    game: &RwLock<GameState>,
    player_id: Uuid,
    node_id: Uuid,
) -> Result<Result<Option<ShiftWork>, String>, sqlx::Error> {
    let (lot, resource_type, resource) = {
        let game = game.read().await;
        let Some(node) = game.resource_nodes.get(&node_id) else {
            return Ok(Ok(None)); // the tick says the node is gone
        };
        let Some(lot) = game.terrain.lot_at(node.x, node.y) else {
            return Ok(Ok(None));
        };
        let resource = game
            .resource_types
            .get(&node.resource_type)
            .map_or_else(|| node.resource_type.clone(), |t| t.name.clone());
        (lot.id.clone(), node.resource_type.clone(), resource)
    };
    if let Some(work) = wages::shift_on_lot(db, player_id, &lot, &resource_type).await? {
        return Ok(Ok(Some(work)));
    }

    let row: Option<(Uuid, Option<String>, Option<String>, Option<String>, bool)> = sqlx::query_as(&format!(
        "SELECT p.owner_id, pp.station_use_level, pl.name, g.name, COALESCE({IN_GUILD}, FALSE)
         FROM plots p
         LEFT JOIN plot_permissions pp ON pp.plot_id = p.id
         LEFT JOIN players pl ON pl.id = p.owner_id
         LEFT JOIN guilds g ON g.id = p.guild_id
         WHERE p.terrain_id = $1 AND p.owner_id IS NOT NULL"
    ))
    .bind(&lot)
    .bind(player_id)
    .fetch_optional(db)
    .await?;
    let Some((owner, level, owner_name, guild, in_guild)) = row else {
        return Ok(Ok(None));
    };
    if may_gather(player_id, Some(owner), level.as_deref(), in_guild) {
        return Ok(Ok(None));
    }
    // A hall's is the guild's, not its leader's (S4.3)
    let whose = guild.or(owner_name).map_or_else(|| "someone else's".to_string(), |name| format!("{name}'s"));
    let words = format!("This {resource} grows on {whose} lot");
    Ok(Err(super::extraction::refuse(&resource_type, (super::extraction::Refused::Lot, words))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_impound_only_lets_things_out_to_an_empty_slot() {
        let refused = |source, target, held| impound_refusal(source, target, held).is_some();
        assert!(refused(Some("player_inventory"), IMPOUND, false), "no deposit");
        assert!(refused(Some(IMPOUND), IMPOUND, false), "no shuffling inside either");
        assert!(refused(Some(IMPOUND), "player_inventory", true), "a swap would put the other back");
        assert!(!refused(Some(IMPOUND), "player_inventory", false));
        assert!(!refused(Some(IMPOUND), "station_inventory", false));
        assert!(!refused(Some("player_inventory"), "station_inventory", true));
    }

    #[test]
    fn the_market_box_only_lets_things_out_at_the_hall_to_an_empty_slot() {
        let refused = |source, target, held, at_hall| market_box_refusal(source, target, held, at_hall).is_some();
        // Away from the hall, nothing in or out
        assert!(refused(Some(MARKET_BOX), "player_inventory", false, false), "out, from anywhere");
        assert!(refused(Some(MARKET_BOX), MARKET_BOX, false, false), "shuffling, from anywhere");
        assert!(refused(Some("player_inventory"), MARKET_BOX, false, false), "in, from anywhere");
        // At the hall, out to an empty slot, and shuffling within it
        assert!(!refused(Some(MARKET_BOX), "player_inventory", false, true));
        assert!(!refused(Some(MARKET_BOX), MARKET_BOX, true, true));
        assert!(refused(Some(MARKET_BOX), "player_inventory", true, true), "a swap would put the other in");
        assert!(refused(Some("player_inventory"), MARKET_BOX, false, true), "no deposit, even there");
        assert!(refused(None, MARKET_BOX, false, true));
        // Other containers aren't its business
        assert!(!refused(Some("player_inventory"), "station_inventory", true, false));
    }

    /// The refusals as the moves see them (#244): the market box is used at
    /// the Exchange hall, by the live position when online and the saved one
    /// when not, and the impound's rule still holds.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn moves_use_the_market_box_only_at_the_exchange_hall(db: PgPool) {
        use crate::engine::market::tests::player;
        use crate::models::{TerrainConfig, WorldGenParams};
        use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};

        let terrain = TerrainConfig::get_default();
        let door = terrain.landmark(LandmarkKind::ExchangeHall).expect("the default world has a hall").door;
        let game = RwLock::new(GameState::with_terrain(WorldGenParams::default(), terrain));
        let ada = player(&db, "ada", 100).await;
        let save = |(x, y): (f64, f64)| {
            sqlx::query("UPDATE players SET position_x = $1, position_y = $2 WHERE id = $3")
                .bind(x)
                .bind(y)
                .bind(ada.id)
                .execute(&db)
        };
        let out = (Some(MARKET_BOX), "player_inventory");
        let outside = (SPAWN_X, door.1); // on the avenue, across the kerb from the door

        // Offline: the saved position counts
        save((SPAWN_X, SPAWN_Y)).await.unwrap();
        let why = move_refusal(&game, &db, ada.id, None, out, false).await.unwrap();
        assert!(why.is_some_and(|w| w.contains("at the Exchange hall")), "{why:?}");
        save(outside).await.unwrap();
        assert_eq!(move_refusal(&game, &db, ada.id, None, out, false).await.unwrap(), None);

        // Online: the live position counts, not the saved one
        assert!(move_refusal(&game, &db, ada.id, Some((SPAWN_X, SPAWN_Y)), out, false).await.unwrap().is_some());
        assert_eq!(move_refusal(&game, &db, ada.id, Some(outside), out, false).await.unwrap(), None);

        // Nothing goes in, even at the hall; the impound's rule is unchanged
        let into = (Some("player_inventory"), MARKET_BOX);
        assert!(move_refusal(&game, &db, ada.id, Some(outside), into, false).await.unwrap().is_some());
        let impounded = (Some(IMPOUND), "player_inventory");
        assert_eq!(move_refusal(&game, &db, ada.id, None, impounded, false).await.unwrap(), None);
        let deposit = (Some("player_inventory"), IMPOUND);
        assert!(move_refusal(&game, &db, ada.id, None, deposit, false).await.unwrap().is_some());
    }

    fn chest(owner: Uuid) -> StationAccess {
        StationAccess {
            station_owner: owner,
            plot_owner: Some(owner),
            station_use_level: Some("owner".into()),
            in_guild: false,
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
    fn friends_and_unknown_levels_admit_no_one() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        for level in ["friends", "bogus"] {
            for in_guild in [false, true] {
                let s = StationAccess { station_use_level: Some(level.into()), in_guild, ..chest(them) };
                assert!(matches!(station_access(me, &s), Err(AccessError::Forbidden)), "{level}");
            }
        }
    }

    /// #80's owner-only fallback is gone (S4.3): the guild level admits the
    /// guild's members, and only them
    #[test]
    fn the_guild_level_admits_the_guilds_members() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        let guild = StationAccess { station_use_level: Some("guild".into()), ..chest(them) };
        assert!(station_access(me, &StationAccess { in_guild: true, ..guild.clone() }).is_ok());
        assert!(matches!(station_access(me, &guild), Err(AccessError::Forbidden)), "not a member");
        let owner_only = StationAccess { in_guild: true, ..chest(them) };
        assert!(matches!(station_access(me, &owner_only), Err(AccessError::Forbidden)), "a member, on an owner-only plot");
        assert!(admits(Some("public"), false) && admits(Some("guild"), true));
        assert!(!admits(Some("guild"), false) && !admits(Some("owner"), true) && !admits(None, true));
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
        assert!(may_gather(me, None, None, false), "on no lot, or an unclaimed one");
    }

    #[test]
    fn grass_on_an_owned_lot_is_the_owners() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(may_gather(me, Some(me), Some("owner"), false));
        assert!(!may_gather(me, Some(them), Some("owner"), true));
        for level in ["friends", "guild", "bogus"] {
            assert!(!may_gather(me, Some(them), Some(level), false), "{level}");
        }
        assert!(!may_gather(me, Some(them), None, true), "no permissions row");
    }

    #[test]
    fn a_public_lot_shares_its_grass() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(may_gather(me, Some(them), Some("public"), false));
    }

    #[test]
    fn a_guild_lot_shares_its_grass_with_the_guild() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(may_gather(me, Some(them), Some("guild"), true));
        assert!(!may_gather(me, Some(them), Some("guild"), false));
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
            world_size: capitol_protocol::rules::WORLD_SIZE,
            center_x: capitol_protocol::rules::CENTER_X,
            center_y: capitol_protocol::rules::CENTER_Y,
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
            landmarks: Vec::new(),
            land: Default::default(),
            rivers: Vec::new(),
            streams: Vec::new(),
            lakes: Vec::new(),
            fords: Vec::new(),
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

    /// A wage shift for the lot's grass lets its worker gather it, as the
    /// employer's (Commons D7), and only while it runs
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_wage_shift_lets_its_worker_gather_on_the_lot(db: PgPool) {
        use capitol_protocol::rest::OfferWages;
        use capitol_protocol::rules::wages::Work;
        let (owner, _) = player(&db, "owner").await;
        let (worker, _) = player(&db, "worker").await;
        let (game, on_lot, _) = game_with_a_lot();
        let plot: Uuid = sqlx::query_scalar(
            "INSERT INTO plots (zone_id, grid_x, grid_y, world_x, world_y, bounds_min_x,
                                bounds_min_y, bounds_max_x, bounds_max_y, size_category, terrain_id, owner_id)
             VALUES ((SELECT id FROM zones LIMIT 1), 9998, 9998, 0, 0, -10, -10, 10, 10, 'small', 'test_lot', $1)
             RETURNING id",
        )
        .bind(owner)
        .fetch_one(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO plot_permissions (plot_id) VALUES ($1)").bind(plot).execute(&db).await.unwrap();
        sqlx::query("INSERT INTO resource_nodes (id, resource_type, position_x, position_y, base_quality) VALUES ($1, 'grass', 0, 0, 50)")
            .bind(on_lot)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("UPDATE players SET strand_balance = 100 WHERE id = $1").bind(owner).execute(&db).await.unwrap();
        let terms = |who| gathering_terms(&db, &game, who, on_lot);
        assert!(terms(worker).await.unwrap().is_err(), "the owner's grass");

        let now = chrono::Utc::now();
        let grass = OfferWages { work: Work::Gather { plot_id: plot, resource: "grass".into() }, rate: 1, cap: 5, term_days: 1, worker_id: None };
        let grows = wages::grows(&db, &game, &grass.work).await.unwrap();
        assert!(grows, "the grass on the lot");
        let (offer, _) = wages::offer(&db, owner, &grass, grows, now).await.unwrap();
        wages::take(&db, worker, offer.id, now).await.unwrap();
        let Ok(Some(work)) = terms(worker).await.unwrap() else { panic!("on a shift for it") };
        assert_eq!((work.offer_id, work.employer_id), (offer.id, owner));
        assert_eq!(terms(owner).await.unwrap(), Ok(None), "the owner gathers as ever");

        wages::stop(&db, worker, offer.id, now).await.unwrap();
        assert!(terms(worker).await.unwrap().is_err(), "the shift's over");
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

    /// A guild led by `leader` with `members` in it, as rows (S4.3)
    async fn guild(db: &PgPool, leader: Uuid, members: &[Uuid]) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO guilds (id, name, tag, charter, founded_at, founder_id, leader_id, leader_since)
             VALUES ($1, $2, 'TEST', '', NOW(), $3, $3, NOW())",
        )
        .bind(id)
        .bind(format!("Guild {id}"))
        .bind(leader)
        .execute(db)
        .await
        .unwrap();
        for (i, player) in std::iter::once(&leader).chain(members).enumerate() {
            sqlx::query("INSERT INTO guild_members (player_id, guild_id, role, joined_at) VALUES ($1, $2, $3, NOW())")
                .bind(player)
                .bind(id)
                .bind(if i == 0 { "leader" } else { "member" })
                .execute(db)
                .await
                .unwrap();
        }
        id
    }

    /// The guild level (S4.3) opens a plot's stations, their boxes and its
    /// grass to its owner's guild, not to anyone else, and leaving the guild
    /// closes them at once. A guild's hall opens to the hall's guild.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_guild_plot_opens_to_the_guild_alone(db: PgPool) {
        let (owner, _) = player(&db, "owner").await;
        let (member, _) = player(&db, "member").await;
        let (outsider, _) = player(&db, "outsider").await;
        let (station, chest) = chest_on_plot(&db, owner, "guild").await;
        let rope = guild(&db, owner, &[member]).await;
        let forbidden = |r: Result<(), AccessError>| matches!(r, Err(AccessError::Forbidden));

        assert!(check_station_access(&db, member, station, HERE).await.is_ok());
        assert!(check_container_access(&db, member, chest, HERE).await.is_ok(), "deposit and withdraw");
        assert!(forbidden(check_station_access(&db, outsider, station, HERE).await.map(|_| ())));
        assert!(forbidden(check_container_access(&db, outsider, chest, HERE).await.map(|_| ())));

        // Out of the guild, out of its plots, at once
        sqlx::query("DELETE FROM guild_members WHERE player_id = $1").bind(member).execute(&db).await.unwrap();
        assert!(forbidden(check_station_access(&db, member, station, HERE).await.map(|_| ())));

        // A hall is the guild's: its members are in, whoever the owner is
        let (other_leader, _) = player(&db, "other").await;
        sqlx::query("UPDATE plots SET guild_id = $1, owner_id = $2 WHERE id = (SELECT plot_id FROM stations WHERE id = $3)")
            .bind(rope)
            .bind(other_leader)
            .bind(station)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO guild_members (player_id, guild_id, role, joined_at) VALUES ($1, $2, 'member', NOW())")
            .bind(outsider)
            .bind(rope)
            .execute(&db)
            .await
            .unwrap();
        assert!(check_station_access(&db, outsider, station, HERE).await.is_ok(), "a member of the hall's guild");
        assert!(forbidden(check_station_access(&db, member, station, HERE).await.map(|_| ())));
    }

    /// A guild-level lot's grass is its guild's to gather (S4.3)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_guild_lot_shares_its_grass_with_the_guild(db: PgPool) {
        let (owner, _) = player(&db, "owner").await;
        let (member, _) = player(&db, "member").await;
        let (outsider, _) = player(&db, "outsider").await;
        let (game, on_lot, _) = game_with_a_lot();
        let plot: Uuid = sqlx::query_scalar(
            "INSERT INTO plots (zone_id, grid_x, grid_y, world_x, world_y, bounds_min_x,
                                bounds_min_y, bounds_max_x, bounds_max_y, size_category, terrain_id, owner_id)
             VALUES ((SELECT id FROM zones LIMIT 1), 9998, 9998, 0, 0, -10, -10, 10, 10, 'small', 'test_lot', $1)
             RETURNING id",
        )
        .bind(owner)
        .fetch_one(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO plot_permissions (plot_id, station_use_level) VALUES ($1, 'guild')")
            .bind(plot)
            .execute(&db)
            .await
            .unwrap();
        guild(&db, owner, &[member]).await;

        assert_eq!(gathering_refusal(&db, &game, member, on_lot).await.unwrap(), None);
        assert_eq!(
            gathering_refusal(&db, &game, outsider, on_lot).await.unwrap().as_deref(),
            Some("This grass grows on owner's lot")
        );
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
