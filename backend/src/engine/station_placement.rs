//! Putting a station down. WS `place_station` and REST `/stations/place`
//! both come here.
//!
//! The tests run in this order, and the Flint client's ghost
//! (`capitol-core/src/station.rs`) runs the ones it can in the same order:
//! - the station type exists
//! - the spot is within the type's `interaction_range` of where you stand
//! - where it stands ([`site`]):
//!   - on a lot: the lot is yours; upstairs, the spot is inside the
//!     building, the storey is built and up, and the type isn't a fire
//!     (V3.3, #475); and there's room in its place: on its storey indoors,
//!     or in the yard (V3.1, #473)
//!   - on no lot: the type is `off_plot` (only the workbench), it's on the
//!     ground, and the ground is dry (#187)
//! - you hold its kit
//! - it's at least `STATION_MIN_SPACING` from every other station in the
//!   same place: outdoors, or indoors on the same storey of the same
//!   building. A wall stands between a station indoors and one out, or one
//!   next door, and a floor between one storey and the next.
//!
//! A home lot's building stands on `rules::building::footprint`. A station
//! inside it is indoors (`stations.indoors`), and counts against the room of
//! its storey (`stations.floor`), `plots.indoor_room` on each; one outside
//! counts against the yard's, `plots.yard_room` (Plots v2 §5.6). A storey
//! upstairs is a floor the owner built (`floors`), and the last can't be
//! used until it's up (`floor_ready_at`). A station upstairs keeps its real
//! position, so it's reached from the street as one on the ground is. A
//! guild hall has no building, and its size category's capacity holds for
//! the lot as a whole.
//!
//! A station off a plot has no `plot_id`. Only its owner may use it
//! (`container_access::station_access`), it pays no tax, and no plot's
//! `station_count` includes it. A terrain change that lays a lot under one
//! takes it down (`plot_sync`, #188).

use capitol_protocol::rest::open_floors;
use capitol_protocol::rules::floors::{storey_name, storey_title};
use capitol_protocol::rules::stacking::Pick;
use capitol_protocol::rules::{building, in_water, water_name, Zone, STATION_MIN_SPACING};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::container_access::live_position;
use super::tutorial::Did;
use crate::models::{StationInfo, StationType, STATION_TYPE_COLUMNS};
use crate::state::AppState;

/// Why a station can't go down where it was asked for.
#[derive(Debug)]
pub enum PlacementError {
    UnknownType(String),
    TooFar,
    /// On a lot that isn't the player's: someone else's, or nobody's.
    NotYourPlot,
    /// The place the spot is in holds all it can: indoors (`Some(true)`),
    /// the yard (`Some(false)`), or a lot with no building as a whole.
    PlotFull { count: i32, capacity: i32, indoors: Option<bool> },
    /// On no lot, and the type isn't one that may stand off a plot.
    NeedsAPlot,
    /// On no lot, in the water of this ring.
    InWater(String),
    /// Upstairs, but not inside a building (V3.3)
    NotIndoors,
    /// Upstairs on a storey the building doesn't have
    NoSuchFloor(i32),
    /// Upstairs on the floor still going up
    FloorUnderWay(i32),
    /// Upstairs, and the type is a fire (`ground_only`): its name
    GroundOnly(String),
    KitNotFound,
    WrongKit { expected: String, got: String },
    TooClose,
    Database(sqlx::Error),
}

impl PlacementError {
    pub fn message(&self) -> String {
        match self {
            PlacementError::UnknownType(t) => format!("Unknown station type: {t}"),
            PlacementError::TooFar => "Too far to place station".into(),
            PlacementError::NotYourPlot => "You don't own this plot".into(),
            PlacementError::PlotFull { count, capacity, indoors: None } => format!("Plot is at capacity ({count}/{capacity})"),
            PlacementError::PlotFull { count, capacity, indoors: Some(true) } => {
                format!("No room left indoors ({count}/{capacity})")
            }
            PlacementError::PlotFull { count, capacity, indoors: Some(false) } => {
                format!("No room left in the yard ({count}/{capacity})")
            }
            PlacementError::NeedsAPlot => "Must place station on a plot".into(),
            PlacementError::InWater(ring) => format!("Can't build in {}", water_name(ring)),
            PlacementError::NotIndoors => "Only a building has upstairs".into(),
            PlacementError::NoSuchFloor(floor) => {
                format!("This building has no {}", storey_name(*floor).trim_start_matches("the "))
            }
            PlacementError::FloorUnderWay(floor) => format!("{} is still going up", storey_title(*floor)),
            PlacementError::GroundOnly(name) => {
                format!("A {} needs the ground floor: fire can't go upstairs", name.to_lowercase())
            }
            PlacementError::KitNotFound => "Kit item not found".into(),
            PlacementError::WrongKit { expected, got } => format!("Wrong item type: expected {expected}, got {got}"),
            PlacementError::TooClose => "Too close to another station".into(),
            PlacementError::Database(e) => format!("Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for PlacementError {
    fn from(e: sqlx::Error) -> Self {
        PlacementError::Database(e)
    }
}

impl From<PlacementError> for crate::error::AppError {
    fn from(e: PlacementError) -> Self {
        use crate::error::AppError;
        match e {
            PlacementError::NotYourPlot => AppError::Forbidden(e.message()),
            PlacementError::KitNotFound => AppError::NotFound(e.message()),
            PlacementError::Database(db) => AppError::Database(db),
            _ => AppError::BadRequest(e.message()),
        }
    }
}

/// The lot under a spot, as far as placing on it goes.
#[derive(Debug, Clone)]
pub struct Lot {
    pub id: Uuid,
    pub owner_id: Option<Uuid>,
    /// It's a guild's hall, and the one placing is an officer of that guild
    /// (S4.3), so they may build on it as its leader may
    pub officer: bool,
    /// Whether the spot is inside the lot's building, or `None` for a lot
    /// with none (a guild hall)
    pub indoors: Option<bool>,
    /// The floors built over the ground, the last perhaps still going up
    pub floors: i32,
    /// The floors that are up, which stations may go on
    pub open_floors: i32,
    /// The stations in the spot's place, indoors on the storey asked for or
    /// in the yard, or on the whole lot where it has no building
    pub station_count: i32,
    /// What that place holds
    pub station_capacity: i32,
}

/// What a station type's rules say about where it may stand
#[derive(Debug, Clone, Default)]
pub struct Kind {
    /// It may stand off a plot (#187)
    pub off_plot: bool,
    /// It's a fire: the ground floor only (V3.3)
    pub ground_only: bool,
    /// Its name, for a refusal
    pub name: String,
}

/// Where a station of `kind` would stand, on storey `floor`: on `lot`, or
/// on open ground, `water` being the ring whose water the spot is in, if
/// any. `Ok` is the plot it stands on, `None` for open ground. Pure, so it
/// can be tested without a database.
pub fn site(player_id: Uuid, kind: &Kind, floor: i32, lot: Option<&Lot>, water: Option<&str>) -> Result<Option<Uuid>, PlacementError> {
    if floor < 0 {
        return Err(PlacementError::NoSuchFloor(floor));
    }
    match lot {
        Some(lot) => {
            if lot.owner_id != Some(player_id) && !lot.officer {
                return Err(PlacementError::NotYourPlot);
            }
            if floor > 0 {
                if lot.indoors != Some(true) {
                    return Err(PlacementError::NotIndoors);
                }
                if floor > lot.floors {
                    return Err(PlacementError::NoSuchFloor(floor));
                }
                if floor > lot.open_floors {
                    return Err(PlacementError::FloorUnderWay(floor));
                }
                if kind.ground_only {
                    return Err(PlacementError::GroundOnly(kind.name.clone()));
                }
            }
            if lot.station_count >= lot.station_capacity {
                return Err(PlacementError::PlotFull {
                    count: lot.station_count,
                    capacity: lot.station_capacity,
                    indoors: lot.indoors,
                });
            }
            Ok(Some(lot.id))
        }
        None if !kind.off_plot => Err(PlacementError::NeedsAPlot),
        None if floor > 0 => Err(PlacementError::NotIndoors),
        None => match water {
            Some(ring) => Err(PlacementError::InWater(ring.to_string())),
            None => Ok(None),
        },
    }
}

/// A station just put down.
pub struct Placed {
    pub info: StationInfo,
    /// The plot it stands on; `None` off a plot.
    pub plot_id: Option<Uuid>,
}

impl Placed {
    /// Adds the station to the game state, where the tick finds it for
    /// crafting. WS and REST placement both call this; before E2.7 only WS
    /// did, so a station placed over REST couldn't be crafted at until a
    /// restart loaded it.
    pub async fn remember(&self, state: &AppState) {
        let station = crate::models::StationState {
            id: self.info.id,
            station_type: self.info.station_type.clone(),
            owner_id: self.info.owner_id,
            x: self.info.x,
            y: self.info.y,
            plot_id: self.plot_id,
            container_id: self.info.container_id,
            quality: self.info.quality,
        };
        state.game.write().await.stations.insert(station.id, station);
    }
}

/// Put a station of `station_type` down at (`x`, `y`) for `player_id`, from
/// their kit `kit_item_id`. `at` is where they stand now
/// ([`live_position`]); when it's `None`, the position comes from their
/// `players` row, which can be up to 10 s old. The game state isn't touched:
/// the caller adds the station to it with [`Placed::remember`]. Once it's
/// down, it's judged for the road in: off a plot, or at home (#439).
pub async fn place_station(
    state: &AppState,
    player_id: Uuid,
    station_type: &str,
    x: f64,
    y: f64,
    floor: i32,
    kit_item_id: Uuid,
) -> Result<Placed, PlacementError> {
    let at = match live_position(&state.game, player_id).await {
        Some(at) => at,
        None => sqlx::query_as("SELECT position_x, position_y FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(&state.db)
            .await?,
    };
    let water = {
        let game = state.game.read().await;
        let terrain = &game.terrain;
        in_water((terrain.center_x, terrain.center_y), &terrain.water_rings, (x, y))
            .map(|r| r.id.clone())
            // The wilds' rivers, streams and lakes too (W4)
            .or_else(|| game.ground.water_at(x, y).map(String::from))
    };
    let placed = place(&state.db, Request { player_id, station_type, x, y, floor, kit_item_id, at, water: water.as_deref() }).await?;
    let events = state.game.read().await.event_sender.clone();
    super::tutorial::saw(&state.db, &events, player_id, Did::Placed { at_home: placed.plot_id.is_some() }).await;
    Ok(placed)
}

/// A placement, with what the game state knows about it already read
struct Request<'a> {
    player_id: Uuid,
    station_type: &'a str,
    x: f64,
    y: f64,
    /// The storey: 0 the ground
    floor: i32,
    kit_item_id: Uuid,
    /// Where the player stands
    at: (f64, f64),
    /// The ring or wild water the spot is in, if any
    water: Option<&'a str>,
}

/// Every check and every write of a placement, in one transaction (#300).
/// The kit, and the lot it goes on, are locked before they're checked, so a
/// second placement of the same kit, or a trade or split of it in between,
/// waits and then finds it gone. Two placements at once take turns
/// ([`PLACEMENT_LOCK`]), so they can't both pass the spacing test.
async fn place(db: &PgPool, req: Request<'_>) -> Result<Placed, PlacementError> {
    let mut tx = db.begin().await?;
    let station_type = req.station_type;
    let placed = place_in(db, &mut tx, req).await?;
    // Its kit went into it, counted once it commits (C1.7)
    let kit: Option<String> = sqlx::query_scalar("SELECT kit_item FROM station_types WHERE id = $1")
        .bind(station_type)
        .fetch_one(&mut *tx)
        .await?;
    let mut flows = super::ledger::Flows::new();
    if let Some(kit) = kit {
        flows.add(&kit, capitol_protocol::ledger::UnitFlow::KitPlaced, 1);
    }
    super::ledger::commit(tx, flows).await?;
    Ok(placed)
}

/// [`place`] in `tx`, which the caller commits
async fn place_in(db: &PgPool, tx: &mut PgConnection, req: Request<'_>) -> Result<Placed, PlacementError> {
    let Request { player_id, station_type, x, y, floor, kit_item_id, at: (px, py), water } = req;
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(PLACEMENT_LOCK).execute(&mut *tx).await?;

    let st: StationType = sqlx::query_as(&format!("SELECT {STATION_TYPE_COLUMNS} FROM station_types WHERE id = $1"))
        .bind(station_type)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| PlacementError::UnknownType(station_type.to_string()))?;

    let (dx, dy) = (x - px, y - py);
    if (dx * dx + dy * dy).sqrt() > st.interaction_range as f64 {
        return Err(PlacementError::TooFar);
    }

    let lot = lot_at(db, tx, player_id, x, y, floor).await?;
    let kind = Kind { off_plot: st.off_plot, ground_only: st.ground_only, name: st.name.clone() };
    let plot_id = site(player_id, &kind, floor, lot.as_ref(), water)?;

    let expected_kit = st.kit_item.clone().ok_or_else(|| PlacementError::UnknownType(station_type.to_string()))?;
    let kit_type: String = sqlx::query_scalar("SELECT item_type FROM items WHERE id = $1 AND owner_id = $2 FOR UPDATE")
        .bind(kit_item_id)
        .bind(player_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PlacementError::KitNotFound)?;
    if kit_type != expected_kit {
        return Err(PlacementError::WrongKit { expected: expected_kit, got: kit_type });
    }
    // Kits don't stack, so this is the kit's one unit; an old stack of them
    // gives up its lowest (E1.8)
    let held = super::units::units_of(tx, kit_item_id).await?;
    let kit = held.pick(1, Pick::default()).ok_or(PlacementError::KitNotFound)?;
    let Some(kit_quality) = kit.min() else {
        return Err(PlacementError::KitNotFound);
    };

    // Only in the same place: on this storey indoors in this building, or
    // outdoors, where every station is on the ground
    let indoors = lot.as_ref().and_then(|l| l.indoors).unwrap_or(false);
    let nearby: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM stations
         WHERE (position_x - $1)^2 + (position_y - $2)^2 < $3
           AND indoors = $4 AND (NOT $4 OR plot_id = $5) AND floor = $6",
    )
    .bind(x)
    .bind(y)
    .bind(STATION_MIN_SPACING * STATION_MIN_SPACING)
    .bind(indoors)
    .bind(plot_id)
    .bind(floor)
    .fetch_optional(&mut *tx)
    .await?;
    if nearby.is_some() {
        return Err(PlacementError::TooClose);
    }

    // The kit goes first: with it locked the take can't miss, but if it did
    // there'd be no station from nothing, only an error and a rollback
    let taken = super::units::take_units(tx, kit_item_id, &kit).await?.ok_or(PlacementError::KitNotFound)?;
    // The station keeps the kit's maker with its quality, so the kit it
    // gives back is still their work (#241)
    let kit_maker = taken.marked.lead_maker();

    let station_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality, kit_maker_id, indoors, floor)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(station_id)
    .bind(station_type)
    .bind(player_id)
    .bind(x as f32)
    .bind(y as f32)
    .bind(plot_id)
    .bind(kit_quality)
    .bind(kit_maker)
    .bind(indoors)
    .bind(floor)
    .execute(&mut *tx)
    .await?;

    if let Some(plot_id) = plot_id {
        sqlx::query("UPDATE plots SET station_count = station_count + 1 WHERE id = $1")
            .bind(plot_id)
            .execute(&mut *tx)
            .await?;
    }

    let container_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO containers (id, container_type, station_id)
         VALUES ($1, 'station_inventory', $2)",
    )
    .bind(container_id)
    .bind(station_id)
    .execute(&mut *tx)
    .await?;

    Ok(Placed {
        info: StationInfo {
            id: station_id,
            station_type: station_type.to_string(),
            name: st.name,
            category: st.category,
            icon: st.icon,
            x,
            y,
            owner_id: player_id,
            container_id: Some(container_id),
            interaction_range: st.interaction_range,
            quality: kit_quality,
            tool: None,
            automation: None,
            production: Vec::new(),
            indoors,
            condition: capitol_protocol::rules::weather::FULL as i32,
            floor,
        },
        plot_id,
    })
}

/// The advisory lock a placement holds until it commits, so placements take
/// turns. They're rare, and it's what keeps two at once from both passing the
/// spacing test.
const PLACEMENT_LOCK: i64 = 0x0300_57a7;

/// The lot under (`x`, `y`), with its room for stations on storey `floor`,
/// its row locked in `tx` until the placement commits, as `player_id` would
/// place on it. Which lot is under a spot only changes with the terrain, so
/// finding it needn't be in the transaction.
async fn lot_at(db: &PgPool, tx: &mut PgConnection, player_id: Uuid, x: f64, y: f64, floor: i32) -> Result<Option<Lot>, sqlx::Error> {
    let Some(plot) = crate::routes::plots::find_plot_at_position(db, x, y).await? else {
        return Ok(None);
    };
    type Row = (Option<Uuid>, i32, i32, Option<i32>, Option<i32>, bool, i32, Option<DateTime<Utc>>);
    let (owner_id, station_count, station_capacity, indoor_room, yard_room, officer, floors, floor_ready_at): Row = sqlx::query_as(
        "SELECT p.owner_id, p.station_count, psc.station_capacity, p.indoor_room, p.yard_room,
                EXISTS (SELECT 1 FROM guild_members m
                        WHERE m.player_id = $2 AND m.guild_id = p.guild_id AND m.role IN ('leader', 'officer')),
                p.floors, p.floor_ready_at
         FROM plots p
         JOIN plot_size_categories psc ON p.size_category = psc.id
         WHERE p.id = $1
         FOR UPDATE OF p",
    )
    .bind(plot.id)
    .bind(player_id)
    .fetch_one(&mut *tx)
    .await?;
    let open_floors = open_floors(floors, floor_ready_at, Utc::now());
    let (Some(indoor_room), Some(yard_room), Some(corners), Some(zone)) =
        (indoor_room, yard_room, plot.quad(), Zone::from_str(&plot.zone_id))
    else {
        // No building: the lot as a whole
        return Ok(Some(Lot {
            id: plot.id,
            owner_id,
            officer,
            indoors: None,
            floors: 0,
            open_floors: 0,
            station_count,
            station_capacity,
        }));
    };
    let indoors = building::indoors(&corners, zone, x, y);
    // Outdoors everything is on the ground
    let storey = if indoors { floor } else { 0 };
    let in_place: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stations WHERE plot_id = $1 AND indoors = $2 AND floor = $3")
            .bind(plot.id)
            .bind(indoors)
            .bind(storey)
            .fetch_one(&mut *tx)
            .await?;
    Ok(Some(Lot {
        id: plot.id,
        owner_id,
        officer,
        indoors: Some(indoors),
        floors,
        open_floors,
        station_count: in_place as i32,
        station_capacity: if indoors { indoor_room } else { yard_room },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(owner: Option<Uuid>, count: i32) -> Lot {
        Lot {
            id: Uuid::from_u128(7),
            owner_id: owner,
            officer: false,
            indoors: Some(true),
            floors: 0,
            open_floors: 0,
            station_count: count,
            station_capacity: 4,
        }
    }

    fn kind(off_plot: bool) -> Kind {
        Kind { off_plot, ground_only: false, name: "Workbench".into() }
    }

    /// A hall's officers build on it as its leader does (S4.3)
    #[test]
    fn a_halls_officers_build_on_it() {
        let (me, leader) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let hall = Lot { officer: true, indoors: None, ..lot(Some(leader), 0) };
        assert_eq!(site(me, &kind(false), 0, Some(&hall), None).unwrap(), Some(Uuid::from_u128(7)));
        let full = Lot { station_count: 4, ..hall };
        assert!(matches!(site(me, &kind(false), 0, Some(&full), None), Err(PlacementError::PlotFull { .. })));
    }

    #[test]
    fn on_a_lot_it_must_be_yours_with_room() {
        let me = Uuid::from_u128(1);
        let them = Uuid::from_u128(2);
        for off_plot in [false, true] {
            let k = kind(off_plot);
            assert_eq!(site(me, &k, 0, Some(&lot(Some(me), 3)), None).unwrap(), Some(Uuid::from_u128(7)));
            assert!(matches!(site(me, &k, 0, Some(&lot(Some(them), 0)), None), Err(PlacementError::NotYourPlot)));
            assert!(
                matches!(site(me, &k, 0, Some(&lot(None, 0)), None), Err(PlacementError::NotYourPlot)),
                "an unclaimed lot is still a lot: claim it first"
            );
            assert!(matches!(
                site(me, &k, 0, Some(&lot(Some(me), 4)), None),
                Err(PlacementError::PlotFull { count: 4, capacity: 4, indoors: Some(true) })
            ));
        }
        let full = site(me, &kind(false), 0, Some(&lot(Some(me), 4)), None).unwrap_err();
        assert_eq!(full.message(), "No room left indoors (4/4)");
    }

    #[test]
    fn off_every_lot_only_an_off_plot_type_goes_and_only_on_dry_ground() {
        let me = Uuid::from_u128(1);
        assert_eq!(site(me, &kind(true), 0, None, None).unwrap(), None, "a workbench in a field");
        assert!(matches!(site(me, &kind(false), 0, None, None), Err(PlacementError::NeedsAPlot)), "a forge in a field");
        let wet = site(me, &kind(true), 0, None, Some("canal_outer")).unwrap_err();
        assert_eq!(wet.message(), "Can't build in the outer canal");
        assert!(matches!(site(me, &kind(false), 0, None, Some("moat")), Err(PlacementError::NeedsAPlot)), "the type first");
        assert!(matches!(site(me, &kind(true), 1, None, None), Err(PlacementError::NotIndoors)), "no upstairs in a field");
    }

    /// Upstairs is inside, on a floor that's built and up, and never a fire
    /// (V3.3, #475)
    #[test]
    fn upstairs_is_indoors_on_a_floor_that_is_up_and_never_a_fire() {
        let me = Uuid::from_u128(1);
        let two = Lot { floors: 2, open_floors: 1, ..lot(Some(me), 0) };
        let bench = kind(false);
        assert_eq!(site(me, &bench, 1, Some(&two), None).unwrap(), Some(Uuid::from_u128(7)));
        let going_up = site(me, &bench, 2, Some(&two), None).unwrap_err();
        assert_eq!(going_up.message(), "The second floor is still going up");
        let none = site(me, &bench, 3, Some(&two), None).unwrap_err();
        assert_eq!(none.message(), "This building has no third floor");
        let yard = Lot { indoors: Some(false), ..two.clone() };
        assert!(matches!(site(me, &bench, 1, Some(&yard), None), Err(PlacementError::NotIndoors)));
        let kiln = Kind { off_plot: false, ground_only: true, name: "Kiln".into() };
        let fire = site(me, &kiln, 1, Some(&two), None).unwrap_err();
        assert_eq!(fire.message(), "A kiln needs the ground floor: fire can't go upstairs");
        assert_eq!(site(me, &kiln, 0, Some(&two), None).unwrap(), Some(Uuid::from_u128(7)), "downstairs it's fine");
        let theirs = Lot { owner_id: Some(Uuid::from_u128(2)), ..two.clone() };
        assert!(matches!(site(me, &bench, 3, Some(&theirs), None), Err(PlacementError::NotYourPlot)), "whose first");
        assert!(matches!(site(me, &bench, -1, Some(&two), None), Err(PlacementError::NoSuchFloor(-1))));
    }

    // These need the local Postgres (`docker-compose up -d`). Each runs in a
    // fresh database that `sqlx::test` creates from the migrations and drops
    // afterwards. Run them with `cargo test station_placement -- --ignored`.

    /// A player standing at the origin, with a workbench kit of `quality` in
    /// their inventory. Returns them and the kit.
    async fn builder(db: &PgPool, name: &str, quality: i32) -> (Uuid, Uuid) {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(account)
            .bind(format!("{name}@test"))
            .execute(db)
            .await
            .unwrap();
        let player = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, $3)")
            .bind(player)
            .bind(account)
            .bind(name)
            .execute(db)
            .await
            .unwrap();
        let inventory = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(inventory)
            .bind(player)
            .execute(db)
            .await
            .unwrap();
        (player, kit(db, player, inventory, 0, quality).await)
    }

    async fn kit(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, quality: i32) -> Uuid {
        let kit = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'workbench_kit', $2, 1, $3, $4, $5)",
        )
        .bind(kit)
        .bind(quality)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
        kit
    }

    /// A workbench on open ground at (`x`, 0), placed by someone standing there
    fn bench(player_id: Uuid, kit_item_id: Uuid, x: f64) -> Request<'static> {
        Request { player_id, station_type: "workbench", x, y: 0.0, floor: 0, kit_item_id, at: (x, 0.0), water: None }
    }

    async fn count(db: &PgPool, sql: &str, id: Uuid) -> i64 {
        sqlx::query_scalar(sql).bind(id).fetch_one(db).await.unwrap()
    }

    async fn stations_of(db: &PgPool, owner: Uuid) -> i64 {
        count(db, "SELECT COUNT(*) FROM stations WHERE owner_id = $1", owner).await
    }

    async fn kit_exists(db: &PgPool, kit: Uuid) -> bool {
        count(db, "SELECT COUNT(*) FROM items WHERE id = $1", kit).await == 1
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_kit_becomes_a_station_with_its_box_and_is_gone(db: PgPool) {
        let (me, kit) = builder(&db, "builder", 64).await;

        let placed = place(&db, bench(me, kit, 0.0)).await.unwrap();

        assert_eq!(placed.info.quality, 64);
        assert_eq!(placed.plot_id, None);
        assert!(!kit_exists(&db, kit).await);
        let kit_quality: i32 = sqlx::query_scalar("SELECT kit_quality FROM stations WHERE id = $1")
            .bind(placed.info.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(kit_quality, 64);
        let container = placed.info.container_id.unwrap();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM containers WHERE id = $1", container).await, 1);
    }

    /// A kit Ada made, placed and taken down by Bo, comes back still Ada's
    /// work (#241)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_kit_keeps_its_makers_mark_through_its_station(db: PgPool) {
        use crate::engine::units::test_units::{all_marked, marked_stack_of};
        use crate::engine::units::Marked;
        use capitol_protocol::rules::stacking::Units;
        let (ada, _) = builder(&db, "ada", 50).await;
        let (bo, plain) = builder(&db, "bo", 50).await;
        let inventory: Uuid = sqlx::query_scalar("SELECT container_id FROM items WHERE id = $1")
            .bind(plain)
            .fetch_one(&db)
            .await
            .unwrap();
        let made = Marked::by(Some(ada), &Units::uniform(1, 71));
        let kit = marked_stack_of(&db, bo, inventory, 1, "workbench_kit", &made).await;

        let placed = place(&db, bench(bo, kit, 0.0)).await.unwrap();
        let kept: Option<Uuid> = sqlx::query_scalar("SELECT kit_maker_id FROM stations WHERE id = $1")
            .bind(placed.info.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(kept, Some(ada));
        crate::engine::station_removal::remove_station(&db, bo, placed.info.id).await.unwrap();

        let kit_back = all_marked(&db, bo, "workbench_kit").await;
        // Beside the plain kit it already held
        assert_eq!(kit_back, Marked::from_rows([(50, None, 1), (71, Some(ada), 1)]));
    }

    /// Places `first` and holds its transaction open while `second` is
    /// asked for, then commits it. Returns what `second` came to.
    async fn race(db: &PgPool, first: Request<'static>, second: Request<'static>) -> Result<Placed, PlacementError> {
        let mut tx = db.begin().await.unwrap();
        place_in(db, &mut tx, first).await.unwrap();
        let second = tokio::spawn({
            let db = db.clone();
            async move { place(&db, second).await }
        });
        // Long enough for the second to get as far as it can
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(!second.is_finished(), "the second placement waits for the first");
        tx.commit().await.unwrap();
        second.await.unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn one_kit_placed_twice_at_once_makes_one_station(db: PgPool) {
        let (me, kit) = builder(&db, "builder", 50).await;

        // Far enough apart that spacing doesn't decide it
        let lost = race(&db, bench(me, kit, 0.0), bench(me, kit, 500.0)).await.err();

        assert!(matches!(lost, Some(PlacementError::KitNotFound)), "the second finds the kit gone: {lost:?}");
        assert_eq!(stations_of(&db, me).await, 1);
        assert!(!kit_exists(&db, kit).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_kit_gone_before_it_is_placed_leaves_nothing_behind(db: PgPool) {
        let (me, kit) = builder(&db, "builder", 50).await;
        let (them, _) = builder(&db, "trader", 50).await;
        // Traded away between the client's look and the placement
        sqlx::query("UPDATE items SET owner_id = $1 WHERE id = $2").bind(them).bind(kit).execute(&db).await.unwrap();

        let err = place(&db, bench(me, kit, 0.0)).await.err();

        assert!(matches!(err, Some(PlacementError::KitNotFound)), "{err:?}");
        assert_eq!(stations_of(&db, me).await, 0);
        assert!(kit_exists(&db, kit).await, "the kit stays with whoever holds it now");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn two_kits_on_one_spot_at_once_make_one_station(db: PgPool) {
        let (me, first) = builder(&db, "builder", 50).await;
        let inventory: Uuid = sqlx::query_scalar("SELECT container_id FROM items WHERE id = $1")
            .bind(first)
            .fetch_one(&db)
            .await
            .unwrap();
        let second = kit(&db, me, inventory, 1, 50).await;

        let lost = race(&db, bench(me, first, 0.0), bench(me, second, 0.0)).await.err();

        assert!(matches!(lost, Some(PlacementError::TooClose)), "{lost:?}");
        assert_eq!(stations_of(&db, me).await, 1);
        assert!(!kit_exists(&db, first).await);
        assert!(kit_exists(&db, second).await, "the second kit stays a kit");
    }

    /// `owner`'s lot of `zone`, a box `w` wide along the street (y = 1000)
    /// and `h` deep, with its room worked out as the startup sync does
    async fn home(db: &PgPool, owner: Uuid, zone: &str, w: f64, h: f64) -> Uuid {
        let id = Uuid::new_v4();
        let corners = vec![1000.0, 1000.0, 1000.0 + w, 1000.0, 1000.0 + w, 1000.0 + h, 1000.0, 1000.0 + h];
        sqlx::query(
            "INSERT INTO plots (id, terrain_id, zone_id, grid_x, grid_y, world_x, world_y,
                                bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y,
                                size_category, owner_id, corners)
             VALUES ($1, $2, $3, 0, 0, $4, $5, 1000, 1000, $6, $7, 'large', $8, $9)",
        )
        .bind(id)
        .bind(format!("test-{id}"))
        .bind(zone)
        .bind((1000.0 + w / 2.0) as f32)
        .bind((1000.0 + h / 2.0) as f32)
        .bind((1000.0 + w) as f32)
        .bind((1000.0 + h) as f32)
        .bind(owner)
        .bind(corners)
        .execute(db)
        .await
        .unwrap();
        let mut tx = db.begin().await.unwrap();
        crate::engine::plot_sync::fill_rooms(&mut tx).await.unwrap();
        tx.commit().await.unwrap();
        id
    }

    /// `n` more workbench kits in `me`'s inventory, after the one `builder` gave
    async fn kits(db: &PgPool, me: Uuid, first: Uuid, n: i32) -> Vec<Uuid> {
        let inventory: Uuid = sqlx::query_scalar("SELECT container_id FROM items WHERE id = $1")
            .bind(first)
            .fetch_one(db)
            .await
            .unwrap();
        let mut out = vec![first];
        for slot in 1..=n {
            out.push(kit(db, me, inventory, slot, 50).await);
        }
        out
    }

    fn at(player_id: Uuid, kit_item_id: Uuid, x: f64, y: f64) -> Request<'static> {
        Request { player_id, station_type: "workbench", x, y, floor: 0, kit_item_id, at: (x, y), water: None }
    }

    /// An urban lot holds 4 indoors and nothing in the yard behind (V3.1, #473)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_urban_lot_holds_four_indoors_and_no_yard(db: PgPool) {
        let (me, first) = builder(&db, "builder", 50).await;
        let plot = home(&db, me, "urban", 90.0, 93.0).await;
        let rooms: (Option<i32>, Option<i32>) =
            sqlx::query_as("SELECT indoor_room, yard_room FROM plots WHERE id = $1").bind(plot).fetch_one(&db).await.unwrap();
        assert_eq!(rooms, (Some(4), Some(0)));
        let k = kits(&db, me, first, 5).await;
        for (i, (x, y)) in [(1017.0, 1015.0), (1072.0, 1015.0), (1017.0, 1070.0), (1072.0, 1070.0)].into_iter().enumerate() {
            let placed = place(&db, at(me, k[i], x, y)).await.unwrap();
            assert_eq!(placed.plot_id, Some(plot));
        }
        let indoors = count(&db, "SELECT COUNT(*) FROM stations WHERE plot_id = $1 AND indoors", plot).await;
        assert_eq!(indoors, 4);
        let full = place(&db, at(me, k[4], 1045.0, 1042.0)).await.err().unwrap();
        assert!(matches!(full, PlacementError::PlotFull { count: 4, capacity: 4, indoors: Some(true) }), "{full:?}");
        let yard = place(&db, at(me, k[5], 1045.0, 1088.0)).await.err().unwrap();
        assert!(matches!(yard, PlacementError::PlotFull { count: 0, capacity: 0, indoors: Some(false) }), "{yard:?}");
    }

    /// A wall stands between a station indoors and one in the yard, so the
    /// spacing holds within each place, not across (V3.1, #473)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_spacing_holds_within_a_place_not_through_a_wall(db: PgPool) {
        let (me, first) = builder(&db, "builder", 50).await;
        let plot = home(&db, me, "suburban", 130.0, 130.0).await;
        let k = kits(&db, me, first, 2).await;
        // The house runs 78 deep from 6 back, to y = 1084
        place(&db, at(me, k[0], 1065.0, 1078.0)).await.unwrap();
        place(&db, at(me, k[1], 1065.0, 1100.0)).await.unwrap();
        let flags: Vec<bool> = sqlx::query_scalar("SELECT indoors FROM stations WHERE plot_id = $1 ORDER BY position_y")
            .bind(plot)
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(flags, vec![true, false], "22 apart through the back wall");
        let crowded = place(&db, at(me, k[2], 1090.0, 1110.0)).await.err().unwrap();
        assert!(matches!(crowded, PlacementError::TooClose), "{crowded:?}");
    }

    /// Each floor holds the building's indoor room again, a place of its
    /// own: stations stand over each other, the last floor waits until it's
    /// up, and fire stays on the ground (V3.3, #475)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn each_floor_holds_four_more_over_the_four_below(db: PgPool) {
        crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
        let (me, first) = builder(&db, "builder", 50).await;
        let plot = home(&db, me, "urban", 90.0, 93.0).await;
        sqlx::query("UPDATE plots SET floors = 2, floor_ready_at = NOW() + INTERVAL '10 minutes' WHERE id = $1")
            .bind(plot)
            .execute(&db)
            .await
            .unwrap();
        let k = kits(&db, me, first, 9).await;
        let spots = [(1017.0, 1015.0), (1072.0, 1015.0), (1017.0, 1070.0), (1072.0, 1070.0)];
        let up = |kit, (x, y): (f64, f64), floor| Request { floor, ..at(me, kit, x, y) };
        for (i, &spot) in spots.iter().enumerate() {
            place(&db, up(k[i], spot, 0)).await.unwrap();
            let placed = place(&db, up(k[4 + i], spot, 1)).await.unwrap();
            assert_eq!((placed.plot_id, placed.info.floor, placed.info.indoors), (Some(plot), 1, true), "over the one below");
        }
        let full = place(&db, up(k[8], (1045.0, 1042.0), 1)).await.err().unwrap();
        assert!(matches!(full, PlacementError::PlotFull { count: 4, capacity: 4, indoors: Some(true) }), "{full:?}");
        let going_up = place(&db, up(k[8], spots[0], 2)).await.err().unwrap();
        assert!(matches!(going_up, PlacementError::FloorUnderWay(2)), "{going_up:?}");
        let none = place(&db, up(k[8], spots[0], 3)).await.err().unwrap();
        assert!(matches!(none, PlacementError::NoSuchFloor(3)), "{none:?}");
        let floors: Vec<i32> = sqlx::query_scalar("SELECT floor FROM stations WHERE plot_id = $1 ORDER BY floor")
            .bind(plot)
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(floors, vec![0, 0, 0, 0, 1, 1, 1, 1]);
        let capacity: i32 = sqlx::query_scalar(&format!(
            "SELECT {} FROM plots p JOIN plot_size_categories psc ON psc.id = p.size_category WHERE p.id = $1",
            crate::models::STATION_CAPACITY
        ))
        .bind(plot)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(capacity, 12, "4 a storey, the one going up counted");

        // Up now: a bench goes on the second floor, but no kiln does
        sqlx::query("UPDATE plots SET floor_ready_at = NOW() - INTERVAL '1 minute' WHERE id = $1")
            .bind(plot)
            .execute(&db)
            .await
            .unwrap();
        place(&db, up(k[8], spots[0], 2)).await.unwrap();
        let inventory: Uuid =
            sqlx::query_scalar("SELECT id FROM containers WHERE owner_id = $1").bind(me).fetch_one(&db).await.unwrap();
        let kiln = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'kiln_kit', 50, 1, $2, $3, 20)",
        )
        .bind(kiln)
        .bind(me)
        .bind(inventory)
        .execute(&db)
        .await
        .unwrap();
        let fire = place(&db, Request { station_type: "kiln", ..up(kiln, spots[1], 2) }).await.err().unwrap();
        assert_eq!(fire.message(), "A kiln needs the ground floor: fire can't go upstairs");
    }
}
