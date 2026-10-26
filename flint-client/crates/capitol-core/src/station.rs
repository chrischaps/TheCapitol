//! Stations in the world (P6): whether you can reach one, and where you may
//! put one down.
//!
//! The server lets you use a station from up to its type's
//! `interaction_range` away (75 units for every seeded type), measured
//! from your server position to the station's. React draws its orange
//! ring on the same test (`GameCanvas.tsx`, `distance <= 75`), and so
//! does the Flint client. Opening one, or walking to it first, is
//! [`visit`] (P6.3).
//!
//! **Placing (P6.2).** A right click on a `*_kit` starts [`Placing`]. The
//! ghost turns green or red on [`check`], which runs the server's own
//! tests in the server's order (`handle_place_station` in
//! `backend/src/ws/connection.rs`):
//! - within 75 of you
//! - on a lot: one you own (the terrain lot, B7); upstairs, inside its
//!   building, on a floor that's built and up, and not a fire (the
//!   catalog's `ground_only`, V3.3, #475); with room where the spot is:
//!   indoors on that storey, inside the building that comes with the deed,
//!   or in the yard (V3.1, #473)
//! - on no lot: a type that may stand off a plot (the catalog's
//!   `off_plot`, only the workbench), on the ground, on dry ground (#187)
//! - at least 50 from every other station in the same place: on the same
//!   storey of the same building, or both outdoors. A wall stands between
//!   the two, and a floor between storeys.
//!
//! Your own plots come from the store; whether any lot covers the spot, and
//! whether it's water, come from `/terrain`. Without a terrain, every spot
//! off your plots is refused as [`Refusal::NotYourPlot`].
//!
//! A click where they all pass sends `place_station` over the socket. The
//! REST route places the station in the database but not in the game
//! state, so it has to be the socket. The server answers `station_placed`
//! or an `error`, and [`Placing`] waits for one or the other.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use capitol_protocol::rest::{PlotInfo, SlotItem, StationInfo};
use capitol_protocol::rules::{self, building, floors, Zone, STATION_INTERACTION_RANGE, STATION_MIN_SPACING};
use capitol_protocol::terrain::{quad_contains, TerrainResponse};
use capitol_protocol::ws::ClientMessage;
use uuid::Uuid;

use crate::catalog::Catalog;
use crate::sim::lot::Building;
use crate::state::{answers, GameState, StateEvent};

pub mod visit;
pub use visit::{approach, is_storage, recipes_at, Visit};

/// How long a `place_station` may go unanswered. The server answers within
/// a few database round trips.
pub const UNANSWERED: Duration = Duration::from_secs(4);

/// Whether a player at `at` (server units) is close enough to use
/// `station`: within its interaction range, the edge included.
pub fn in_reach(at: [f64; 2], station: &StationInfo) -> bool {
    let d = (station.x - at[0]).hypot(station.y - at[1]);
    d <= station.interaction_range as f64
}

/// Why a station can't go down where the ghost is. The server refuses the
/// same cases, in this order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// More than 75 units from where you stand.
    TooFar,
    /// On a lot that isn't yours: someone else's, or nobody's.
    NotYourPlot,
    /// On your plot, but the place the spot is in holds all it can: indoors
    /// (`Some(true)`), the yard (`Some(false)`), or a lot with no building.
    PlotFull { capacity: i32, indoors: Option<bool> },
    /// On no lot, and this station only goes on a plot.
    NeedsAPlot,
    /// On no lot, but in the water.
    InWater,
    /// Upstairs, but not inside a building (V3.3).
    NotIndoors,
    /// Upstairs on a storey the building doesn't have.
    NoSuchFloor(i32),
    /// Upstairs on the floor still going up.
    FloorUnderWay(i32),
    /// Upstairs, and it's a fire.
    GroundOnly,
    /// Under 50 units from another station.
    TooClose,
}

impl Refusal {
    /// What the ghost says under itself.
    pub fn words(&self) -> String {
        match self {
            Refusal::TooFar => "Too far from you".into(),
            Refusal::NotYourPlot => "Not on a plot you own".into(),
            Refusal::PlotFull { capacity, indoors: None } => format!("This plot is full ({capacity}/{capacity})"),
            Refusal::PlotFull { capacity, indoors: Some(true) } => format!("No room left indoors ({capacity}/{capacity})"),
            Refusal::PlotFull { capacity, indoors: Some(false) } => format!("No room left in the yard ({capacity}/{capacity})"),
            Refusal::NeedsAPlot => "This only goes on a plot you own".into(),
            Refusal::InWater => "Not on the water".into(),
            Refusal::NotIndoors => "Only a building has upstairs".into(),
            Refusal::NoSuchFloor(floor) => {
                format!("This building has no {}", floors::storey_name(*floor).trim_start_matches("the "))
            }
            Refusal::FloorUnderWay(floor) => format!("{} is still going up", floors::storey_title(*floor)),
            Refusal::GroundOnly => "Fire can't go upstairs".into(),
            Refusal::TooClose => "Too close to another station".into(),
        }
    }
}

/// What's being put down, as far as where it may go: whether its type may
/// stand off a plot, whether it's a fire (`ground_only`), and the storey
/// asked for, judged at `now` on the server's clock (V3.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub off_plot: bool,
    pub ground_only: bool,
    /// 0, the ground, or the kth floor
    pub floor: i32,
    pub now: DateTime<Utc>,
}

impl Placement {
    /// On the ground, where a fire may go and no floor's readiness matters
    pub fn ground(off_plot: bool) -> Self {
        Placement { off_plot, ground_only: false, floor: 0, now: DateTime::<Utc>::MIN_UTC }
    }
}

/// [`check_placement`] on the ground floor. `off_plot` is whether the
/// station type may stand off a plot.
pub fn check<'a>(
    at: [f64; 2],
    you: [f64; 2],
    me: Uuid,
    off_plot: bool,
    stations: impl IntoIterator<Item = &'a StationInfo>,
    plots: &[PlotInfo],
    terrain: Option<&TerrainResponse>,
) -> Result<Option<Uuid>, Refusal> {
    check_placement(at, you, me, Placement::ground(off_plot), stations, plots, terrain)
}

/// Whether a station can go down at `at` (server units), as `what` says,
/// for the player `me` standing at `you`. `plots` may hold anyone's; only
/// `me`'s count. `terrain` tells open ground from a lot and from water.
/// `Ok` names the plot the station would stand on, `None` for open ground.
pub fn check_placement<'a>(
    at: [f64; 2],
    you: [f64; 2],
    me: Uuid,
    what: Placement,
    stations: impl IntoIterator<Item = &'a StationInfo>,
    plots: &[PlotInfo],
    terrain: Option<&TerrainResponse>,
) -> Result<Option<Uuid>, Refusal> {
    let [x, y] = at;
    let floor = what.floor;
    if floor < 0 {
        return Err(Refusal::NoSuchFloor(floor));
    }
    // Every seeded type's range is 75; the server tests the type's own.
    if (x - you[0]).hypot(y - you[1]) > STATION_INTERACTION_RANGE {
        return Err(Refusal::TooFar);
    }
    let stations: Vec<&StationInfo> = stations.into_iter().collect();
    let plot = match plots
        .iter()
        .find(|p| p.owner_id == Some(me) && p.contains(x, y))
    {
        Some(plot) => {
            let building = Building::of(plot);
            if floor > 0 {
                if !building.is_some_and(|b| b.contains((x, y))) {
                    return Err(Refusal::NotIndoors);
                }
                if floor > plot.floors {
                    return Err(Refusal::NoSuchFloor(floor));
                }
                if floor > plot.open_floors(what.now) {
                    return Err(Refusal::FloorUnderWay(floor));
                }
                if what.ground_only {
                    return Err(Refusal::GroundOnly);
                }
            }
            // Its building's room where the spot is, on its storey, or the
            // lot's as a whole
            let (count, capacity, indoors) = match building {
                Some(b) => {
                    let indoors = b.contains((x, y));
                    let count = stations
                        .iter()
                        .filter(|s| {
                            plot.contains(s.x, s.y) && b.contains((s.x, s.y)) == indoors && s.floor == floor
                        })
                        .count() as i32;
                    let room = if indoors { b.indoor_room } else { b.yard_room };
                    (count, room as i32, Some(indoors))
                }
                None => (plot.station_count, plot.station_capacity, None),
            };
            if count >= capacity {
                return Err(Refusal::PlotFull { capacity, indoors });
            }
            Some(plot.id)
        }
        None => {
            let terrain = terrain.ok_or(Refusal::NotYourPlot)?;
            if terrain
                .terrain_plots
                .iter()
                .any(|lot| quad_contains(&lot.corners, x, y))
            {
                return Err(Refusal::NotYourPlot);
            }
            if !what.off_plot {
                return Err(Refusal::NeedsAPlot);
            }
            if floor > 0 {
                return Err(Refusal::NotIndoors);
            }
            let center = (terrain.center_x, terrain.center_y);
            if rules::in_water(center, &terrain.water_rings, (x, y)).is_some()
                || rules::ground::water_at(terrain, x, y).is_some()
            {
                return Err(Refusal::InWater);
            }
            None
        }
    };
    // Only in the same place: on the same storey of the same building, or
    // both outdoors, where everything is on the ground
    let place = building_at(x, y, plots, terrain).map(|f| (f, floor));
    if stations.iter().any(|s| {
        (s.x - x).hypot(s.y - y) < STATION_MIN_SPACING
            && building_at(s.x, s.y, plots, terrain).map(|f| (f, s.floor)) == place
    }) {
        return Err(Refusal::TooClose);
    }
    Ok(plot)
}

/// The footprint of the building `(x, y)` is inside, or `None` outdoors: a
/// lot's from `/terrain`, or, without it, one of `plots`' (V3.1, #473).
fn building_at(x: f64, y: f64, plots: &[PlotInfo], terrain: Option<&TerrainResponse>) -> Option<[(f64, f64); 4]> {
    let inside = |corners: &[(f64, f64); 4], zone: &str| {
        let footprint = building::footprint(corners, Zone::from_str(zone)?)?;
        quad_contains(&footprint, x, y).then_some(footprint)
    };
    match terrain {
        Some(t) => t
            .terrain_plots
            .iter()
            .filter(|lot| quad_contains(&lot.corners, x, y))
            .find_map(|lot| inside(&lot.corners, &lot.zone)),
        None => plots
            .iter()
            .filter(|p| p.contains(x, y))
            .find_map(|p| inside(&crate::sim::lot::corners(p), &p.zone_id)),
    }
}

/// The kit being placed.
#[derive(Debug, Clone, PartialEq)]
pub struct Kit {
    pub item_id: Uuid,
    pub item_name: String,
    /// The station it makes, "workbench" say.
    pub station_type: String,
}

impl Kit {
    /// The kit in `item`, if the catalog says it is one.
    pub fn of(catalog: &Catalog, item: &SlotItem) -> Option<Kit> {
        Some(Kit {
            item_id: item.item_id,
            item_name: item.item_name.clone(),
            station_type: catalog.station_for_kit(&item.item_type)?.id.clone(),
        })
    }
}

/// Where a placement stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// The ghost follows the cursor; a click puts it down.
    Aiming,
    /// `place_station` went out for `at`; no answer yet.
    Asking { at: [f64; 2], since: Instant },
}

/// Something worth telling the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// A click where the station can't go.
    Refused(Refusal),
    /// The server refused, in its words ("Too far to place station").
    Failed(String),
    /// A `place_station` went out and nothing came back.
    Unanswered,
    /// It's down.
    Placed { station_id: Uuid },
}

/// Placing one kit: aiming, then asking, until the station is down or the
/// kit is gone. Feed it every batch of state changes and tick it each
/// frame. [`Placing::is_done`] says when to leave placement mode.
#[derive(Debug, Clone)]
pub struct Placing {
    kit: Kit,
    /// The storey it goes on: 0, the ground, until a panel picks one (#477)
    floor: i32,
    phase: Phase,
    notes: Vec<Note>,
    done: bool,
}

impl Placing {
    pub fn new(kit: Kit) -> Self {
        Self {
            kit,
            floor: 0,
            phase: Phase::Aiming,
            notes: Vec::new(),
            done: false,
        }
    }

    pub fn kit(&self) -> &Kit {
        &self.kit
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The storey it goes on (V3.3)
    pub fn floor(&self) -> i32 {
        self.floor
    }

    /// Put it on storey `floor` instead: 0 the ground, k the kth floor
    pub fn on_floor(&mut self, floor: i32) {
        self.floor = floor.max(0);
    }

    /// The station is down, or the kit has gone.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Whether the station can go down at `at` now, by the store and
    /// `terrain`. Until the server has placed you, it can't go anywhere.
    pub fn check(
        &self,
        at: [f64; 2],
        state: &GameState,
        terrain: Option<&TerrainResponse>,
    ) -> Result<Option<Uuid>, Refusal> {
        let (Some(me), Some(you)) = (state.me(), state.local_player()) else {
            return Err(Refusal::TooFar);
        };
        let kind = state.catalog().station_type(&self.kit.station_type);
        let now = state
            .server_time_ms()
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        let what = Placement {
            off_plot: kind.is_some_and(|t| t.off_plot),
            ground_only: kind.is_some_and(|t| t.ground_only),
            floor: self.floor,
            now,
        };
        check_placement(
            at,
            [you.x, you.y],
            me,
            what,
            state.stations().values(),
            state.plots(),
            terrain,
        )
    }

    /// A click at `at`. Returns the message to send if the station can go
    /// there and nothing is out already; a refusal is noted instead.
    pub fn place(
        &mut self,
        at: [f64; 2],
        state: &GameState,
        terrain: Option<&TerrainResponse>,
        now: Instant,
    ) -> Option<ClientMessage> {
        if self.phase != Phase::Aiming || self.done {
            return None;
        }
        if let Err(why) = self.check(at, state, terrain) {
            self.notes.push(Note::Refused(why));
            return None;
        }
        self.phase = Phase::Asking { at, since: now };
        Some(ClientMessage::PlaceStation {
            station_type: self.kit.station_type.clone(),
            x: at[0],
            y: at[1],
            kit_item_id: self.kit.item_id.to_string(),
            floor: self.floor,
        })
    }

    /// After every batch of state changes.
    pub fn observe(&mut self, changes: &[StateEvent], state: &GameState) {
        for change in changes {
            match (change, &self.phase) {
                (
                    StateEvent::StationPlaced {
                        station_id,
                        owner_id,
                    },
                    Phase::Asking { .. },
                ) if state.me() == Some(*owner_id) => {
                    self.notes.push(Note::Placed {
                        station_id: *station_id,
                    });
                    self.done = true;
                }
                // A refused placement (#89). Errors for other commands aren't ours.
                (StateEvent::ServerError { message, command }, Phase::Asking { .. })
                    if answers(command, "place_station") =>
                {
                    self.notes.push(Note::Failed(message.clone()));
                    self.phase = Phase::Aiming;
                }
                // A reconnect drops the ask; the snapshot shows whether it landed.
                (StateEvent::SnapshotStale, Phase::Asking { .. }) => self.phase = Phase::Aiming,
                // Moved into a station's box, traded, or used: nothing to place.
                (StateEvent::ContainersUpdated, Phase::Aiming)
                    if !holds(state, self.kit.item_id) =>
                {
                    self.done = true;
                }
                _ => {}
            }
        }
    }

    /// Once per frame: gives up on an unanswered ask.
    pub fn tick(&mut self, now: Instant) {
        if let Phase::Asking { since, .. } = self.phase {
            if now.duration_since(since) >= UNANSWERED {
                self.phase = Phase::Aiming;
                self.notes.push(Note::Unanswered);
            }
        }
    }

    /// What's worth telling the player since the last call.
    pub fn take_notes(&mut self) -> Vec<Note> {
        std::mem::take(&mut self.notes)
    }
}

/// Whether any of the player's own containers holds the stack `item_id`.
fn holds(state: &GameState, item_id: Uuid) -> bool {
    state
        .containers()
        .values()
        .any(|c| c.items().any(|i| i.item_id == item_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::Bounds;

    fn station(x: f64, y: f64) -> StationInfo {
        StationInfo {
            id: Uuid::nil(),
            station_type: "workbench".into(),
            name: "Workbench".into(),
            category: "crafting".into(),
            icon: None,
            x,
            y,
            owner_id: Uuid::nil(),
            container_id: None,
            interaction_range: 75.0,
            quality: 50,
            tool: None,
            automation: None,
            production: Vec::new(),
            indoors: false,
            condition: 100,
            floor: 0,
        }
    }

    const ME: Uuid = Uuid::from_u128(1);
    const THEM: Uuid = Uuid::from_u128(2);

    /// A 200-unit square lot from (x, y), held by `owner`, with `count`
    /// of its 16 stations down.
    fn plot(id: u128, x: f64, y: f64, owner: Option<Uuid>, count: i32) -> PlotInfo {
        let (x1, y1) = (x + 200.0, y + 200.0);
        PlotInfo {
            guild: None,
            id: Uuid::from_u128(id),
            zone_id: "suburban".into(),
            world_x: x + 100.0,
            world_y: y + 100.0,
            bounds: Bounds::new(x, y, x1, y1),
            size_category: "large".into(),
            plot_type: "standard".into(),
            owner_id: owner,
            owner_name: None,
            claimed_at: None,
            assessed_value: 0,
            tax_owed: 0,
            forecloses_at: None,
            deed_cost: Some(1000),
            station_count: count,
            station_capacity: 16,
            terrain_id: Some(format!("lot_{id}")),
            corners: Some([(x, y), (x1, y), (x1, y1), (x, y1)]),
            indoor_room: None,
            yard_room: None,
            owner_citizen: false,
            floor_ready_at: None,
            floors: 0,
            next_floor: None,
        }
    }

    #[test]
    fn the_ring_toggles_at_75_units() {
        let s = station(4000.0, 7350.0);
        assert!(in_reach([4000.0, 7350.0], &s));
        assert!(in_reach([4075.0, 7350.0], &s), "the edge counts");
        assert!(in_reach([4045.0, 7410.0], &s), "75 on the diagonal");
        assert!(!in_reach([4075.1, 7350.0], &s));
        assert!(!in_reach([4000.0, 7274.0], &s));
    }

    #[test]
    fn the_range_comes_from_the_station() {
        let mut s = station(0.0, 0.0);
        s.interaction_range = 120.0;
        assert!(in_reach([100.0, 0.0], &s));
    }

    /// The terrain under `plots`: their lots, and a moat of radius 200 and
    /// width 20 round (4000, 4000).
    fn terrain(plots: &[PlotInfo]) -> TerrainResponse {
        use capitol_protocol::terrain::{TerrainPlot, WaterRing};
        TerrainResponse {
            world_size: 8000.0,
            center_x: 4000.0,
            center_y: 4000.0,
            water_rings: vec![WaterRing {
                id: "moat".into(),
                radius: 200.0,
                width: 20.0,
                bridges: vec![],
            }],
            roads: vec![],
            terrain_plots: plots
                .iter()
                .map(|p| TerrainPlot {
                    id: p.terrain_id.clone().unwrap(),
                    corners: p.corners.unwrap(),
                    street_facing_edge: (0, 1),
                    area: 40_000.0,
                    adjacent_street_id: "street".into(),
                    zone: "suburban".into(),
                })
                .collect(),
            parks: vec![],
            landmarks: vec![],
            land: Default::default(),
            rivers: vec![],
            streams: vec![],
            lakes: vec![],
            fords: vec![],
            version: None,
        }
    }

    #[test]
    fn a_station_goes_on_your_plot_near_you_and_clear_of_others() {
        let mine = plot(10, 1000.0, 1000.0, Some(ME), 0);
        let theirs = plot(11, 1200.0, 1000.0, Some(THEM), 0);
        let free = plot(12, 1000.0, 1200.0, None, 0);
        let plots = [mine.clone(), theirs, free];
        let ground = terrain(&plots);
        let you = [1150.0, 1150.0];
        let none: [StationInfo; 0] = [];
        let at = |x, y, stations: &[StationInfo]| {
            check([x, y], you, ME, true, stations, &plots, Some(&ground))
        };

        assert_eq!(at(1100.0, 1100.0, &none), Ok(Some(mine.id)));
        assert_eq!(
            at(1199.0, 1100.0, &none),
            Ok(Some(mine.id)),
            "near the edge"
        );
        assert_eq!(
            at(1210.0, 1150.0, &none),
            Err(Refusal::NotYourPlot),
            "theirs"
        );
        assert_eq!(
            at(1150.0, 1210.0, &none),
            Err(Refusal::NotYourPlot),
            "nobody's"
        );
        assert_eq!(at(1150.0, 1226.0, &none), Err(Refusal::TooFar), "76 away");
        assert_eq!(
            at(1150.0, 1225.0, &none),
            Err(Refusal::NotYourPlot),
            "75 is in range"
        );

        // Both inside the house, which runs from 6 to 126 back (#473)
        let near = [station(1100.0, 1051.0)];
        assert_eq!(at(1100.0, 1100.0, &near), Err(Refusal::TooClose), "49 away");
        assert_eq!(at(1100.0, 1101.0, &near), Ok(Some(mine.id)), "50 is clear");
    }

    #[test]
    fn a_workbench_goes_on_open_ground_but_not_on_the_water() {
        // Your plot sits in the far corner; the spots below are on no lot.
        let mine = plot(10, 1000.0, 1000.0, Some(ME), 0);
        let plots = [mine];
        let ground = terrain(&plots);
        let at = |off_plot, x: f64, y: f64, ground| {
            check([x, y], [x, y - 10.0], ME, off_plot, [], &plots, ground)
        };

        assert_eq!(
            at(true, 4000.0, 4100.0, Some(&ground)),
            Ok(None),
            "inside the moat"
        );
        assert_eq!(
            at(true, 4000.0, 4300.0, Some(&ground)),
            Ok(None),
            "outside it"
        );
        assert_eq!(
            at(true, 4000.0, 4200.0, Some(&ground)),
            Err(Refusal::InWater)
        );
        assert_eq!(
            at(false, 4000.0, 4300.0, Some(&ground)),
            Err(Refusal::NeedsAPlot),
            "a forge in a field"
        );
        assert_eq!(
            at(false, 4000.0, 4200.0, Some(&ground)),
            Err(Refusal::NeedsAPlot),
            "the type before the water, as the server"
        );
        assert_eq!(
            at(true, 4000.0, 4300.0, None),
            Err(Refusal::NotYourPlot),
            "no terrain, no telling it's open"
        );
        let crowded = [station(4000.0, 4320.0)];
        let r = check(
            [4000.0, 4300.0],
            [4000.0, 4290.0],
            ME,
            true,
            &crowded,
            &plots,
            Some(&ground),
        );
        assert_eq!(r, Err(Refusal::TooClose));
    }

    /// The building's room and the yard's are separate, and a wall stands
    /// between them: a full house refuses a spot inside, and a spot in the
    /// yard is spaced only from the yard's stations (V3.1, #473)
    #[test]
    fn indoors_and_the_yard_are_each_their_own_place() {
        let mut home = plot(10, 1000.0, 1000.0, Some(ME), 4);
        home.corners = Some([(1000.0, 1000.0), (1130.0, 1000.0), (1130.0, 1130.0), (1000.0, 1130.0)]);
        home.bounds = Bounds::new(1000.0, 1000.0, 1130.0, 1130.0);
        (home.indoor_room, home.yard_room, home.station_capacity) = (Some(4), Some(3), 7);
        // The house runs from 6 back to 84, so its back wall is at y = 1084
        let inside = [station(1030.0, 1030.0), station(1090.0, 1030.0), station(1030.0, 1075.0), station(1090.0, 1075.0)];
        let full = check([1060.0, 1052.0], [1060.0, 1052.0], ME, false, &inside, &[home.clone()], None);
        assert_eq!(full, Err(Refusal::PlotFull { capacity: 4, indoors: Some(true) }));
        assert_eq!(full.unwrap_err().words(), "No room left indoors (4/4)");
        let yard = check([1060.0, 1100.0], [1060.0, 1100.0], ME, false, &inside, &[home.clone()], None);
        assert_eq!(yard, Ok(Some(home.id)), "39 from a station inside, through the wall");
        let mut both = inside.to_vec();
        both.push(station(1100.0, 1110.0));
        let crowded = check([1060.0, 1100.0], [1060.0, 1100.0], ME, false, &both, &[home.clone()], None);
        assert_eq!(crowded, Err(Refusal::TooClose), "41 from one in the yard");
    }

    /// Each floor is a place of its own over the one below, the last can't
    /// be used until it's up, and fire stays on the ground (V3.3, #475)
    #[test]
    fn upstairs_is_a_storey_of_its_own() {
        let mut home = plot(10, 1000.0, 1000.0, Some(ME), 9);
        home.corners = Some([(1000.0, 1000.0), (1130.0, 1000.0), (1130.0, 1130.0), (1000.0, 1130.0)]);
        home.bounds = Bounds::new(1000.0, 1000.0, 1130.0, 1130.0);
        (home.indoor_room, home.yard_room, home.station_capacity) = (Some(4), Some(3), 15);
        let now: DateTime<Utc> = "2026-10-25T12:00:00Z".parse().unwrap();
        (home.floors, home.floor_ready_at) = (2, Some(now + chrono::Duration::minutes(10)));
        let on = |floor, mut s: StationInfo| {
            (s.floor, s.indoors) = (floor, true);
            s
        };
        // 55 apart each way, inside the house's walls (6 to 84 back)
        let spots = [(1030.0, 1020.0), (1090.0, 1020.0), (1030.0, 1075.0), (1090.0, 1075.0)];
        let ground: Vec<StationInfo> = spots.iter().map(|&(x, y)| station(x, y)).collect();
        let first: Vec<StationInfo> = ground.iter().take(3).map(|s| on(1, s.clone())).collect();
        let all: Vec<StationInfo> = ground.iter().chain(&first).cloned().collect();
        let bench = |floor| Placement { off_plot: true, ground_only: false, floor, now };
        let at = |what, (x, y): (f64, f64)| check_placement([x, y], [x, y], ME, what, &all, &[home.clone()], None);

        assert_eq!(at(bench(1), spots[3]), Ok(Some(home.id)), "over the one below, and the first floor has room");
        assert_eq!(at(bench(1), (1092.0, 1050.0)), Err(Refusal::TooClose), "upstairs, beside one upstairs");
        assert_eq!(at(bench(0), (1060.0, 1052.0)), Err(Refusal::PlotFull { capacity: 4, indoors: Some(true) }));
        let going_up = at(bench(2), spots[0]);
        assert_eq!(going_up, Err(Refusal::FloorUnderWay(2)));
        assert_eq!(going_up.unwrap_err().words(), "The second floor is still going up");
        let later = Placement { now: now + chrono::Duration::minutes(11), ..bench(2) };
        assert_eq!(at(later, spots[0]), Ok(Some(home.id)), "up now");
        let none = at(bench(3), spots[0]).unwrap_err();
        assert_eq!(none.words(), "This building has no third floor");
        assert_eq!(at(bench(1), (1060.0, 1110.0)), Err(Refusal::NotIndoors), "the yard has no upstairs");
        let kiln = Placement { off_plot: false, ground_only: true, ..bench(1) };
        assert_eq!(at(kiln, spots[3]), Err(Refusal::GroundOnly));
    }

    #[test]
    fn a_full_plot_takes_no_more() {
        let full = plot(10, 1000.0, 1000.0, Some(ME), 16);
        let r = check(
            [1100.0, 1100.0],
            [1100.0, 1100.0],
            ME,
            true,
            [],
            &[full],
            None,
        );
        assert_eq!(r, Err(Refusal::PlotFull { capacity: 16, indoors: None }));
        assert_eq!(r.unwrap_err().words(), "This plot is full (16/16)");
    }

    #[test]
    fn the_checks_run_in_the_servers_order() {
        // Far, off-plot and crowded all at once: the server says "too far".
        let crowded = [station(5000.0, 5001.0)];
        let r = check([5000.0, 5000.0], [0.0, 0.0], ME, true, &crowded, &[], None);
        assert_eq!(r, Err(Refusal::TooFar));
        // On a full plot and crowded: the server says "full".
        let full = plot(10, 4900.0, 4900.0, Some(ME), 16);
        let r = check(
            [5000.0, 5000.0],
            [5000.0, 5050.0],
            ME,
            true,
            &crowded,
            &[full],
            None,
        );
        assert_eq!(r, Err(Refusal::PlotFull { capacity: 16, indoors: None }));
    }

    #[test]
    fn only_kits_are_placed() {
        let item = |t: &str| SlotItem::new(Uuid::from_u128(5), t.into(), "Thing".into(), 50, 1, 0);
        let catalog = Catalog::fallback();
        let kit = |t: &str| Kit::of(&catalog, &item(t)).map(|k| k.station_type);
        assert_eq!(kit("forge_kit").as_deref(), Some("forge"));
        assert_eq!(kit("chest_kit").as_deref(), Some("storage_chest"));
        // D5's new stations (E2.4)
        assert_eq!(kit("sawbench_kit").as_deref(), Some("sawbench"));
        assert_eq!(kit("kiln_kit").as_deref(), Some("kiln"));
        assert_eq!(kit("anvil_kit").as_deref(), Some("anvil"));
        assert_eq!(kit("rope"), None);
    }

    mod placing {
        use super::*;
        use crate::net::{ConnectionState, NetEvent, RestResponse};
        use capitol_protocol::rest::{ContainerResponse, ContainersResponse, ListPlotsResponse};
        use capitol_protocol::ws::{PlayerPosition, ServerMessage as M};

        const KIT: Uuid = Uuid::from_u128(50);

        fn kit_item() -> SlotItem {
            SlotItem::new(
                KIT,
                "workbench_kit".into(),
                "Workbench Kit".into(),
                60,
                1,
                0,
            )
        }

        /// Signed in as ME, standing at (1100, 1030) on our own plot, the
        /// workbench kit in the inventory.
        fn world() -> GameState {
            let mut s = GameState::new();
            s.apply(&NetEvent::Connection(ConnectionState::Connected {
                player_id: ME,
            }));
            s.apply(&NetEvent::Ws(M::Positions {
                players: vec![PlayerPosition {
                    id: ME,
                    name: "Ada".into(),
                    x: 1100.0,
                    y: 1030.0,
                    citizen: false,
                    tag: None,
                }],
            }));
            s.apply_rest(&RestResponse::Plots(ListPlotsResponse {
                plots: vec![plot(10, 1000.0, 1000.0, Some(ME), 0)],
                total: 1,
            }));
            set_inventory(&mut s, true);
            s
        }

        fn set_inventory(s: &mut GameState, with_kit: bool) -> Vec<StateEvent> {
            s.apply_rest(&RestResponse::Containers(ContainersResponse {
                containers: vec![ContainerResponse {
                    id: Uuid::from_u128(90),
                    container_type: "player_inventory".into(),
                    slot_count: 24,
                    layout_columns: 6,
                    slots: if with_kit { vec![kit_item()] } else { vec![] },
                }],
            }))
        }

        fn placing() -> Placing {
            Placing::new(Kit::of(&Catalog::fallback(), &kit_item()).unwrap())
        }

        fn placed(station: u128, owner: Uuid) -> StateEvent {
            StateEvent::StationPlaced {
                station_id: Uuid::from_u128(station),
                owner_id: owner,
            }
        }

        #[test]
        fn a_good_click_asks_once_and_ends_when_its_placed() {
            let state = world();
            let t0 = Instant::now();
            let mut p = placing();
            let sent = p.place([1120.0, 1040.0], &state, None, t0);
            assert_eq!(
                sent,
                Some(ClientMessage::PlaceStation {
                    station_type: "workbench".into(),
                    x: 1120.0,
                    y: 1040.0,
                    kit_item_id: KIT.to_string(),
                    floor: 0,
                })
            );
            assert!(
                p.place([1120.0, 1040.0], &state, None, t0).is_none(),
                "one at a time"
            );
            // Someone else's station going down isn't the answer.
            p.observe(&[placed(70, THEM)], &state);
            assert!(!p.is_done());
            p.observe(&[placed(71, ME)], &state);
            assert!(p.is_done());
            assert_eq!(
                p.take_notes(),
                vec![Note::Placed {
                    station_id: Uuid::from_u128(71)
                }]
            );
        }

        #[test]
        fn a_bad_click_is_refused_and_sends_nothing() {
            let state = world();
            let mut p = placing();
            assert!(p
                .place([1100.0, 990.0], &state, None, Instant::now())
                .is_none());
            assert!(p
                .place([1300.0, 1100.0], &state, None, Instant::now())
                .is_none());
            assert_eq!(
                p.take_notes(),
                vec![
                    Note::Refused(Refusal::NotYourPlot),
                    Note::Refused(Refusal::TooFar)
                ]
            );
            assert_eq!(*p.phase(), Phase::Aiming);
        }

        #[test]
        fn a_server_refusal_or_silence_goes_back_to_aiming() {
            let state = world();
            let t0 = Instant::now();
            let mut p = placing();
            p.place([1120.0, 1040.0], &state, None, t0).unwrap();
            // A trade refused meanwhile isn't the placement's answer (#89).
            let other = StateEvent::ServerError {
                message: "That player is offline".into(),
                command: "trade_initiate".into(),
            };
            p.observe(&[other], &state);
            assert!(matches!(p.phase(), Phase::Asking { .. }));
            assert!(p.take_notes().is_empty());

            let refused = StateEvent::ServerError {
                message: "Too far to place station".into(),
                command: "place_station".into(),
            };
            p.observe(&[refused], &state);
            assert_eq!(*p.phase(), Phase::Aiming);
            assert_eq!(
                p.take_notes(),
                vec![Note::Failed("Too far to place station".into())]
            );

            p.place([1120.0, 1040.0], &state, None, t0).unwrap();
            p.tick(t0 + UNANSWERED - Duration::from_millis(1));
            assert!(matches!(p.phase(), Phase::Asking { .. }));
            p.tick(t0 + UNANSWERED);
            assert_eq!(*p.phase(), Phase::Aiming);
            assert_eq!(p.take_notes(), vec![Note::Unanswered]);
            assert!(!p.is_done());
        }

        #[test]
        fn it_ends_when_the_kit_leaves_the_inventory() {
            let mut state = world();
            let mut p = placing();
            let changes = set_inventory(&mut state, true);
            p.observe(&changes, &state);
            assert!(!p.is_done());
            let changes = set_inventory(&mut state, false);
            p.observe(&changes, &state);
            assert!(p.is_done());
        }

        #[test]
        fn the_ghost_reads_the_store() {
            let state = world();
            let p = placing();
            assert_eq!(
                p.check([1120.0, 1100.0], &state, None),
                Ok(Some(Uuid::from_u128(10)))
            );
            assert_eq!(
                p.check([1120.0, 1190.0], &state, None),
                Err(Refusal::TooFar)
            );
            // Signed out, it can't go anywhere.
            assert_eq!(
                p.check([1120.0, 1100.0], &GameState::new(), None),
                Err(Refusal::TooFar)
            );
        }
    }
}
