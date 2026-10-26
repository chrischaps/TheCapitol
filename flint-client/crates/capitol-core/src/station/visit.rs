//! Visiting a station (P6.3): click one to open it in reach, or walk to its
//! near side and open it on arrival. React's rule, with these changes:
//!
//! - **Further away:** React walks you onto the station, and you click
//!   again. [`Visit`] walks you to the near side, [`APPROACH`] of the range
//!   short of it, and opens it as you come into reach, as
//!   [`crate::harvest`] does for a node.
//! - **Walking out of reach** of the open station sends `close_station`.
//!   React leaves the panel open, and every move then fails with "Too far".
//! - **Refused:** the server answers a refused `open_station` with
//!   `error{message}` ("Not your station", "Too far from station"), which
//!   [`Visit::take_notes`] hands on. Only an error naming `open_station`
//!   counts (#89).

use std::time::{Duration, Instant};

use capitol_protocol::rest::{RecipeInfo, StationInfo};
use uuid::Uuid;

use super::in_reach;
use crate::catalog::Catalog;
use crate::state::{answers, GameState, StateEvent};

/// How long an `open_station` may go unanswered. The server answers at
/// once, with `station_opened` or `error`.
pub const UNANSWERED: Duration = Duration::from_secs(3);

/// Where a walk to a station stops: this fraction of its range from it,
/// on your side. That's inside reach with room to spare, and clear of the model.
pub const APPROACH: f64 = 0.5;

/// Where to walk to use `station` from `at`: on the line between you, at
/// [`APPROACH`] of its range from it. If you're standing on it, onto it.
pub fn approach(at: [f64; 2], station: &StationInfo) -> [f64; 2] {
    let (dx, dy) = (at[0] - station.x, at[1] - station.y);
    let d = dx.hypot(dy);
    if d < 1e-6 {
        return [station.x, station.y];
    }
    let r = station.interaction_range as f64 * APPROACH;
    [station.x + dx / d * r, station.y + dy / d * r]
}

/// The recipes made at a station of `station_type`.
pub fn recipes_at<'a>(
    recipes: &'a [RecipeInfo],
    station_type: &'a str,
) -> impl Iterator<Item = &'a RecipeInfo> {
    recipes
        .iter()
        .filter(move |r| r.required_station_type.as_deref() == Some(station_type))
}

/// A station type that only stores and crafts nothing (the chest).
pub fn is_storage(catalog: &Catalog, station_type: &str) -> bool {
    catalog
        .station_type(station_type)
        .is_some_and(|t| t.category == "storage")
}

/// What the game should do for a click, an arrival or a walk away.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Send `open_station` for this station.
    Open(Uuid),
    /// Walk to this server point. The station stays the target.
    WalkTo { x: f64, y: f64 },
    /// Send `close_station`: you walked out of reach of the open one.
    Close,
    /// The click is taken, and there's nothing to send: it's open already.
    Hold,
}

/// Something worth telling the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// The server refused to open it, in its words.
    Refused(String),
    /// An `open_station` went out and nothing came back.
    Unanswered,
    /// The station you were walking to went away on the way.
    Lost,
}

#[derive(Debug, Default)]
pub struct Visit {
    /// The station you're walking to, to open on arrival.
    target: Option<Uuid>,
    /// The `open_station` sent, and when, until it's answered.
    asked: Option<(Uuid, Instant)>,
    /// A `close_station` is out for this station: don't send another.
    closing: Option<Uuid>,
    notes: Vec<Note>,
}

impl Visit {
    /// The station you're walking to, to open on arrival.
    pub fn target(&self) -> Option<Uuid> {
        self.target
    }

    /// The station an `open_station` is out for, unanswered.
    pub fn asked(&self) -> Option<Uuid> {
        self.asked.map(|(id, _)| id)
    }

    /// You clicked `station`. `None` if the store doesn't know it, so the
    /// click should fall through.
    pub fn click(&mut self, station: Uuid, state: &GameState, now: Instant) -> Option<Step> {
        let s = state.station(station)?;
        let me = state.local_player()?;
        if state.open_station().map(|o| o.station_id) == Some(station) {
            self.target = None;
            return Some(Step::Hold);
        }
        let at = [me.x, me.y];
        if in_reach(at, s) {
            self.target = None;
            self.asked = Some((station, now));
            Some(Step::Open(station))
        } else {
            self.target = Some(station);
            let [x, y] = approach(at, s);
            Some(Step::WalkTo { x, y })
        }
    }

    /// You walked somewhere else: the target is dropped.
    pub fn walked(&mut self) {
        self.target = None;
    }

    /// After every batch of state changes. Returns `Open` when you've come
    /// into reach of the target, and `Close` when you've left the open one.
    pub fn observe(
        &mut self,
        changes: &[StateEvent],
        state: &GameState,
        now: Instant,
    ) -> Option<Step> {
        for change in changes {
            match change {
                StateEvent::StationOpened { .. } => {
                    self.asked = None;
                    self.target = None;
                    self.closing = None;
                }
                StateEvent::StationClosed => self.closing = None,
                StateEvent::ServerError { message, command }
                    if self.asked.is_some() && answers(command, "open_station") =>
                {
                    self.asked = None;
                    self.notes.push(Note::Refused(message.clone()));
                }
                StateEvent::MovementBlocked { .. } => self.target = None,
                _ => {}
            }
        }
        let me = state.local_player()?;
        let at = [me.x, me.y];
        if let Some(open) = state.open_station() {
            let gone = match state.station(open.station_id) {
                Some(s) => !in_reach(at, s),
                // Dropped from the nearby poll, so well out of reach.
                None => true,
            };
            if gone && self.closing != Some(open.station_id) {
                self.closing = Some(open.station_id);
                return Some(Step::Close);
            }
        }
        let target = self.target?;
        let Some(s) = state.station(target) else {
            self.target = None;
            self.notes.push(Note::Lost);
            return None;
        };
        if in_reach(at, s) {
            self.target = None;
            self.asked = Some((target, now));
            return Some(Step::Open(target));
        }
        None
    }

    /// Call once per frame: calls an unanswered `open_station` refused.
    pub fn tick(&mut self, now: Instant) {
        if let Some((_, at)) = self.asked {
            if now.duration_since(at) >= UNANSWERED {
                self.asked = None;
                self.notes.push(Note::Unanswered);
            }
        }
    }

    /// Everything worth telling the player since the last call.
    pub fn take_notes(&mut self) -> Vec<Note> {
        std::mem::take(&mut self.notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{ConnectionState, RestResponse};
    use capitol_protocol::rest::NearbyStationsResponse;
    use capitol_protocol::ws::{PlayerPosition, ServerMessage as M};

    const ME: Uuid = Uuid::from_u128(1);
    const BENCH: Uuid = Uuid::from_u128(20);
    const CONTAINER: Uuid = Uuid::from_u128(21);

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

    fn at(state: &mut GameState, x: f64, y: f64) -> Vec<StateEvent> {
        state.apply_ws(&M::Positions {
            players: vec![PlayerPosition {
                id: ME,
                name: "Ada".into(),
                x,
                y,
                citizen: false,
                tag: None,
            }],
        })
    }

    /// Connected, standing at (4000, 4000), with a workbench at (4200, 4000).
    fn world() -> GameState {
        let mut state = GameState::new();
        state.apply_connection(&ConnectionState::Connected { player_id: ME });
        at(&mut state, 4000.0, 4000.0);
        let mut bench = station(4200.0, 4000.0);
        bench.id = BENCH;
        bench.owner_id = ME;
        state.apply_rest(&RestResponse::NearbyStations(NearbyStationsResponse {
            stations: vec![bench],
        }));
        state
    }

    fn opened(state: &mut GameState) -> Vec<StateEvent> {
        state.apply_ws(&M::StationOpened {
            player_id: ME,
            station_id: BENCH,
            station_type: "workbench".into(),
            name: "Workbench".into(),
            container_id: CONTAINER,
        })
    }

    #[test]
    fn the_walk_stops_on_your_side_inside_reach() {
        let s = station(4200.0, 4000.0);
        let [x, y] = approach([4000.0, 4000.0], &s);
        assert!((x - 4162.5).abs() < 1e-9 && y == 4000.0);
        assert!(in_reach([x, y], &s));
        assert_eq!(approach([4200.0, 4000.0], &s), [4200.0, 4000.0]);
    }

    #[test]
    fn in_reach_opens_at_once() {
        let mut state = world();
        at(&mut state, 4150.0, 4000.0);
        let mut v = Visit::default();
        let now = Instant::now();
        assert_eq!(v.click(BENCH, &state, now), Some(Step::Open(BENCH)));
        assert_eq!(v.asked(), Some(BENCH));
        let changes = opened(&mut state);
        assert_eq!(v.observe(&changes, &state, now), None);
        assert_eq!(v.asked(), None);
        // Clicking it again does nothing.
        assert_eq!(v.click(BENCH, &state, now), Some(Step::Hold));
    }

    #[test]
    fn out_of_reach_walks_then_opens_on_arrival() {
        let mut state = world();
        let mut v = Visit::default();
        let now = Instant::now();
        assert_eq!(
            v.click(BENCH, &state, now),
            Some(Step::WalkTo {
                x: 4162.5,
                y: 4000.0
            })
        );
        let changes = at(&mut state, 4100.0, 4000.0);
        assert_eq!(v.observe(&changes, &state, now), None);
        let changes = at(&mut state, 4130.0, 4000.0);
        assert_eq!(v.observe(&changes, &state, now), Some(Step::Open(BENCH)));
        assert_eq!(v.target(), None);
        assert_eq!(v.observe(&changes, &state, now), None, "only once");
    }

    #[test]
    fn walking_elsewhere_or_into_water_drops_the_target() {
        let mut state = world();
        let mut v = Visit::default();
        let now = Instant::now();
        v.click(BENCH, &state, now);
        v.walked();
        let changes = at(&mut state, 4150.0, 4000.0);
        assert_eq!(v.observe(&changes, &state, now), None);

        at(&mut state, 4000.0, 4000.0);
        v.click(BENCH, &state, now);
        let changes = state.apply_ws(&M::MovementBlocked {
            player_id: ME,
            reason: "Cannot cross the moat without a bridge".into(),
            water_id: "moat".into(),
            stopped_x: 4050.0,
            stopped_y: 4000.0,
        });
        assert_eq!(v.observe(&changes, &state, now), None);
        assert_eq!(v.target(), None);
    }

    #[test]
    fn walking_out_of_reach_closes_it_once() {
        let mut state = world();
        at(&mut state, 4150.0, 4000.0);
        let mut v = Visit::default();
        let now = Instant::now();
        v.click(BENCH, &state, now);
        let changes = opened(&mut state);
        v.observe(&changes, &state, now);
        let changes = at(&mut state, 4130.0, 4000.0);
        assert_eq!(v.observe(&changes, &state, now), None, "still in reach");
        let changes = at(&mut state, 4100.0, 4000.0);
        assert_eq!(v.observe(&changes, &state, now), Some(Step::Close));
        let changes = at(&mut state, 4090.0, 4000.0);
        assert_eq!(v.observe(&changes, &state, now), None, "sent once");
        let changes = state.apply_ws(&M::StationClosed { player_id: ME });
        assert_eq!(v.observe(&changes, &state, now), None);
        assert!(state.open_station().is_none());
    }

    #[test]
    fn a_refusal_is_passed_on_in_the_servers_words() {
        let mut state = world();
        at(&mut state, 4150.0, 4000.0);
        let mut v = Visit::default();
        let now = Instant::now();
        v.click(BENCH, &state, now);
        // Another command's error while the open is out isn't its answer (#89).
        let changes = state.apply_ws(&M::Error {
            message: "Slot is occupied".into(),
            command: "move_item".into(),
        });
        v.observe(&changes, &state, now);
        assert!(v.take_notes().is_empty());
        assert!(v.asked().is_some());

        let changes = state.apply_ws(&M::Error {
            message: "Not your station".into(),
            command: "open_station".into(),
        });
        v.observe(&changes, &state, now);
        assert_eq!(
            v.take_notes(),
            vec![Note::Refused("Not your station".into())]
        );
        assert_eq!(v.asked(), None);

        // An error with nothing asked isn't ours.
        let changes = state.apply_ws(&M::Error {
            message: "Item not found".into(),
            command: String::new(),
        });
        v.observe(&changes, &state, now);
        assert!(v.take_notes().is_empty());
    }

    #[test]
    fn an_unanswered_open_is_called_refused() {
        let mut state = world();
        at(&mut state, 4150.0, 4000.0);
        let mut v = Visit::default();
        let now = Instant::now();
        v.click(BENCH, &state, now);
        v.tick(now + UNANSWERED / 2);
        assert!(v.take_notes().is_empty());
        v.tick(now + UNANSWERED);
        assert_eq!(v.take_notes(), vec![Note::Unanswered]);
    }

    #[test]
    fn recipes_are_filtered_by_station_and_the_chest_only_stores() {
        let recipe = |id: &str, at: Option<&str>| RecipeInfo {
            tool_type: None,
            id: id.into(),
            name: id.into(),
            description: None,
            category: "x".into(),
            inputs: vec![],
            outputs: vec![],
            duration_ticks: 10,
            required_station_type: at.map(Into::into),
            quality_weights: capitol_protocol::rules::quality::Weights::for_recipe(
                [],
                None,
                None,
                at.is_some(),
                -3,
                3,
            ),
        };
        let all = [
            recipe("rope_basic", None),
            recipe("craft_forge", Some("workbench")),
            recipe("ingot", Some("forge")),
        ];
        let ids: Vec<_> = recipes_at(&all, "workbench")
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(ids, ["craft_forge"]);
        let catalog = Catalog::fallback();
        assert!(is_storage(&catalog, "storage_chest"));
        assert!(!is_storage(&catalog, "workbench"));
        assert!(!is_storage(&catalog, "unknown"));
    }
}
