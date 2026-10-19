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
//! - on a lot: one you own (the terrain lot, B7), with room on it
//! - on no lot: a type that may stand off a plot (the catalog's
//!   `off_plot`, only the workbench), on dry ground (#187)
//! - at least 50 from every other station
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

use capitol_protocol::rest::{PlotInfo, SlotItem, StationInfo};
use capitol_protocol::rules::{self, STATION_INTERACTION_RANGE, STATION_MIN_SPACING};
use capitol_protocol::terrain::{quad_contains, TerrainResponse};
use capitol_protocol::ws::ClientMessage;
use uuid::Uuid;

use crate::catalog::Catalog;
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
    /// On your plot, but it holds all the stations it can.
    PlotFull { capacity: i32 },
    /// On no lot, and this station only goes on a plot.
    NeedsAPlot,
    /// On no lot, but in the water.
    InWater,
    /// Under 50 units from another station.
    TooClose,
}

impl Refusal {
    /// What the ghost says under itself.
    pub fn words(&self) -> String {
        match self {
            Refusal::TooFar => "Too far from you".into(),
            Refusal::NotYourPlot => "Not on a plot you own".into(),
            Refusal::PlotFull { capacity } => format!("This plot is full ({capacity}/{capacity})"),
            Refusal::NeedsAPlot => "This only goes on a plot you own".into(),
            Refusal::InWater => "Not on the water".into(),
            Refusal::TooClose => "Too close to another station".into(),
        }
    }
}

/// Whether a station can go down at `at` (server units), for the player
/// `me` standing at `you`. `plots` may hold anyone's; only `me`'s count.
/// `off_plot` is whether the station type may stand off a plot, and
/// `terrain` tells open ground from a lot and from water.
/// `Ok` names the plot the station would stand on, `None` for open ground.
pub fn check<'a>(
    at: [f64; 2],
    you: [f64; 2],
    me: Uuid,
    off_plot: bool,
    stations: impl IntoIterator<Item = &'a StationInfo>,
    plots: &[PlotInfo],
    terrain: Option<&TerrainResponse>,
) -> Result<Option<Uuid>, Refusal> {
    let [x, y] = at;
    // Every seeded type's range is 75; the server tests the type's own.
    if (x - you[0]).hypot(y - you[1]) > STATION_INTERACTION_RANGE {
        return Err(Refusal::TooFar);
    }
    let plot = match plots
        .iter()
        .find(|p| p.owner_id == Some(me) && p.contains(x, y))
    {
        Some(plot) if plot.station_count >= plot.station_capacity => {
            return Err(Refusal::PlotFull {
                capacity: plot.station_capacity,
            });
        }
        Some(plot) => Some(plot.id),
        None => {
            let terrain = terrain.ok_or(Refusal::NotYourPlot)?;
            if terrain
                .terrain_plots
                .iter()
                .any(|lot| quad_contains(&lot.corners, x, y))
            {
                return Err(Refusal::NotYourPlot);
            }
            if !off_plot {
                return Err(Refusal::NeedsAPlot);
            }
            let center = (terrain.center_x, terrain.center_y);
            if rules::in_water(center, &terrain.water_rings, (x, y)).is_some() {
                return Err(Refusal::InWater);
            }
            None
        }
    };
    if stations
        .into_iter()
        .any(|s| (s.x - x).hypot(s.y - y) < STATION_MIN_SPACING)
    {
        return Err(Refusal::TooClose);
    }
    Ok(plot)
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
    phase: Phase,
    notes: Vec<Note>,
    done: bool,
}

impl Placing {
    pub fn new(kit: Kit) -> Self {
        Self {
            kit,
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
        let off_plot = state
            .catalog()
            .station_type(&self.kit.station_type)
            .is_some_and(|t| t.off_plot);
        check(
            at,
            [you.x, you.y],
            me,
            off_plot,
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
        }
    }

    const ME: Uuid = Uuid::from_u128(1);
    const THEM: Uuid = Uuid::from_u128(2);

    /// A 200-unit square lot from (x, y), held by `owner`, with `count`
    /// of its 16 stations down.
    fn plot(id: u128, x: f64, y: f64, owner: Option<Uuid>, count: i32) -> PlotInfo {
        let (x1, y1) = (x + 200.0, y + 200.0);
        PlotInfo {
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

        let near = [station(1100.0, 1149.0)];
        assert_eq!(at(1100.0, 1100.0, &near), Err(Refusal::TooClose), "49 away");
        assert_eq!(at(1100.0, 1099.0, &near), Ok(Some(mine.id)), "50 is clear");
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
        assert_eq!(r, Err(Refusal::PlotFull { capacity: 16 }));
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
        assert_eq!(r, Err(Refusal::PlotFull { capacity: 16 }));
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
