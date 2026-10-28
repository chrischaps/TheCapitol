//! Clicking a resource node (P4.4): extract it from where you stand, or walk
//! to it and extract on arrival. This is React's rule (`GameContext.tsx`,
//! `moveToAndExtract` and the effect after it), kept out of the engine so it
//! is a plain test.
//!
//! - **In range** (within [`EXTRACTION_RANGE`] of your server position):
//!   send `extract`.
//! - **Further away:** walk to the node and remember it as the *target*.
//!   Each `positions` checks the distance again, and the first one in range
//!   sends `extract`. A target that someone else takes, or that depletes,
//!   is dropped. So is one you walk away from by clicking elsewhere
//!   ([`Harvest::walked`]).
//! - **Refused:** the server answers a refused `extract` with
//!   `extraction_failed` and its reason (B9, #60), passed on as
//!   [`Note::Refused`].
//! - **Unanswered:** a backstop. If neither answer comes within
//!   [`UNANSWERED`], [`Harvest::tick`] says so, so the UI can say something
//!   rather than nothing.
//!
//! ```no_run
//! # use std::time::Instant;
//! # use capitol_core::{harvest::*, state::*};
//! # let mut state = GameState::new(); let node = uuid::Uuid::nil(); let now = Instant::now();
//! let mut harvest = Harvest::default();
//! match harvest.click(node, &state, now) {
//!     Some(Step::Extract(id)) => { /* send ClientMessage::Extract */ }
//!     Some(Step::WalkTo { x, y }) => { /* walk there, as a click would */ }
//!     Some(Step::Hold) => { /* busy: take the click, do nothing */ }
//!     None => { /* not a node you can take: walk to it instead */ }
//! }
//! ```

use std::time::{Duration, Instant};

use capitol_protocol::rules::EXTRACTION_RANGE;
use uuid::Uuid;

use crate::state::{GameState, StateEvent};

/// How long an `extract` may go unanswered before the client calls it
/// refused. The server answers within a tick or two (100 ms each).
pub const UNANSWERED: Duration = Duration::from_millis(1200);

/// What the game should do for a click or an arrival.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Send `extract` for this node.
    Extract(Uuid),
    /// Walk to this server point. The node stays the target.
    WalkTo { x: f64, y: f64 },
    /// The click is taken, but there's nothing to send: you're busy, and a
    /// walk would cancel what you're doing.
    Hold,
}

/// Something worth telling the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// The node you were walking to was taken or depleted on the way.
    Lost(Uuid),
    /// The server refused the `extract`, and said why.
    Refused(String),
    /// An `extract` went out and nothing came back, not even a refusal.
    Unanswered(Uuid),
    /// You're busy with another action (extracting or crafting).
    Busy,
}

#[derive(Debug, Default)]
pub struct Harvest {
    /// The node you're walking to, to extract on arrival.
    target: Option<Uuid>,
    /// The `extract` sent, and when, until it's answered.
    asked: Option<(Uuid, Instant)>,
    notes: Vec<Note>,
}

impl Harvest {
    /// The node you're walking to, for the targeting line.
    pub fn target(&self) -> Option<Uuid> {
        self.target
    }

    /// The node an `extract` is out for, unanswered.
    pub fn asked(&self) -> Option<Uuid> {
        self.asked.map(|(id, _)| id)
    }

    /// You clicked `node`. `None` if it isn't one you can take now, so the
    /// click should walk instead (React falls through the same way).
    pub fn click(&mut self, node: Uuid, state: &GameState, now: Instant) -> Option<Step> {
        if !state.node_available(node) {
            return None;
        }
        let n = state.node(node)?;
        let me = state.local_player()?;
        if state.local_activity().is_some() {
            self.notes.push(Note::Busy);
            return Some(Step::Hold);
        }
        if (me.x - n.x).hypot(me.y - n.y) <= EXTRACTION_RANGE {
            self.target = None;
            self.asked = Some((node, now));
            Some(Step::Extract(node))
        } else {
            self.target = Some(node);
            Some(Step::WalkTo { x: n.x, y: n.y })
        }
    }

    /// You walked somewhere else: the target is dropped.
    pub fn walked(&mut self) {
        self.target = None;
    }

    /// After every batch of state changes. Returns the `extract` to send
    /// when you've come within range of the target.
    pub fn observe(
        &mut self,
        changes: &[StateEvent],
        state: &GameState,
        now: Instant,
    ) -> Option<Step> {
        for change in changes {
            match change {
                StateEvent::ExtractionStarted { .. } => {
                    self.asked = None;
                    self.target = None;
                }
                StateEvent::ExtractionFailed { reason } => {
                    self.asked = None;
                    self.notes.push(Note::Refused(reason.clone()));
                }
                // Walked into water: the walk is over, and so is the plan.
                StateEvent::MovementBlocked { .. } => self.target = None,
                _ => {}
            }
        }
        let target = self.target?;
        if !state.node_available(target) {
            self.target = None;
            self.notes.push(Note::Lost(target));
            return None;
        }
        let (n, me) = (state.node(target)?, state.local_player()?);
        if (me.x - n.x).hypot(me.y - n.y) <= EXTRACTION_RANGE {
            self.target = None;
            self.asked = Some((target, now));
            return Some(Step::Extract(target));
        }
        None
    }

    /// Call once per frame: calls an unanswered `extract` refused.
    pub fn tick(&mut self, now: Instant) {
        if let Some((node, at)) = self.asked {
            if now.duration_since(at) >= UNANSWERED {
                self.asked = None;
                self.notes.push(Note::Unanswered(node));
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
    use capitol_protocol::rest::{NearbyResponse, ResourceNodeInfo};
    use capitol_protocol::ws::{PlayerPosition, ServerMessage as M};

    const ME: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);
    const NODE: Uuid = Uuid::from_u128(10);

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

    /// Connected, standing at (4000, 4000), with a node at (4200, 4000).
    fn world() -> GameState {
        let mut state = GameState::new();
        state.apply_connection(&ConnectionState::Connected { player_id: ME });
        at(&mut state, 4000.0, 4000.0);
        state.apply_rest(&RestResponse::Nearby(NearbyResponse {
            nodes: vec![ResourceNodeInfo {
                id: NODE,
                resource_type: "grass".into(),
                x: 4200.0,
                y: 4000.0,
                state: "available".into(),
                base_quality: None,
            }],
        }));
        state
    }

    #[test]
    fn in_range_extracts_at_once() {
        let mut state = world();
        at(&mut state, 4160.0, 4000.0);
        let mut h = Harvest::default();
        let now = Instant::now();
        assert_eq!(h.click(NODE, &state, now), Some(Step::Extract(NODE)));
        assert_eq!(h.target(), None);
        assert_eq!(h.asked(), Some(NODE));
    }

    #[test]
    fn out_of_range_walks_then_extracts_on_arrival() {
        let mut state = world();
        let mut h = Harvest::default();
        let now = Instant::now();
        assert_eq!(
            h.click(NODE, &state, now),
            Some(Step::WalkTo {
                x: 4200.0,
                y: 4000.0
            })
        );
        assert_eq!(h.target(), Some(NODE));
        let changes = at(&mut state, 4100.0, 4000.0);
        assert_eq!(h.observe(&changes, &state, now), None);
        let changes = at(&mut state, 4151.0, 4000.0);
        assert_eq!(h.observe(&changes, &state, now), Some(Step::Extract(NODE)));
        assert_eq!(h.target(), None);
        // Only once.
        assert_eq!(h.observe(&changes, &state, now), None);
    }

    #[test]
    fn a_target_taken_on_the_way_is_dropped() {
        let mut state = world();
        let mut h = Harvest::default();
        let now = Instant::now();
        h.click(NODE, &state, now);
        let changes = state.apply_ws(&M::ExtractionStarted {
            player_id: OTHER,
            node_id: NODE,
            duration_ticks: 30,
        });
        assert_eq!(h.observe(&changes, &state, now), None);
        assert_eq!(h.target(), None);
        assert_eq!(h.take_notes(), vec![Note::Lost(NODE)]);
    }

    #[test]
    fn walking_elsewhere_or_into_water_drops_the_target() {
        let mut state = world();
        let mut h = Harvest::default();
        let now = Instant::now();
        h.click(NODE, &state, now);
        h.walked();
        assert_eq!(h.target(), None);

        h.click(NODE, &state, now);
        let changes = state.apply_ws(&M::MovementBlocked {
            player_id: ME,
            reason: "Cannot cross the moat without a bridge".into(),
            water_id: "moat".into(),
            stopped_x: 4100.0,
            stopped_y: 4000.0,
        });
        assert_eq!(h.observe(&changes, &state, now), None);
        assert_eq!(h.target(), None);
        assert!(h.take_notes().is_empty());
    }

    #[test]
    fn depleted_or_busy_nodes_dont_take_the_click() {
        let mut state = world();
        let mut h = Harvest::default();
        let now = Instant::now();
        state.apply_ws(&M::NodeDepleted { node_id: NODE });
        assert_eq!(h.click(NODE, &state, now), None);
        assert_eq!(h.click(Uuid::from_u128(99), &state, now), None);
        assert!(h.take_notes().is_empty());
    }

    #[test]
    fn busy_says_so_and_sends_nothing() {
        let mut state = world();
        state.apply_ws(&M::CraftingStarted {
            player_id: ME,
            operation_id: Uuid::from_u128(40),
            recipe_id: "rope_basic".into(),
            recipe_name: "Rope".into(),
            duration_ticks: 50,
        });
        let mut h = Harvest::default();
        assert_eq!(h.click(NODE, &state, Instant::now()), Some(Step::Hold));
        assert_eq!(h.take_notes(), vec![Note::Busy]);
        assert_eq!(h.target(), None);
    }

    #[test]
    fn an_unanswered_extract_is_called_refused() {
        let mut state = world();
        at(&mut state, 4190.0, 4000.0);
        let mut h = Harvest::default();
        let now = Instant::now();
        h.click(NODE, &state, now);
        h.tick(now + UNANSWERED / 2);
        assert!(h.take_notes().is_empty());
        h.tick(now + UNANSWERED);
        assert_eq!(h.take_notes(), vec![Note::Unanswered(NODE)]);
        assert_eq!(h.asked(), None);

        // An answer in time clears it.
        h.click(NODE, &state, now);
        let changes = state.apply_ws(&M::ExtractionStarted {
            player_id: ME,
            node_id: NODE,
            duration_ticks: 30,
        });
        h.observe(&changes, &state, now);
        h.tick(now + UNANSWERED * 2);
        assert!(h.take_notes().is_empty());
    }

    #[test]
    fn a_refusal_passes_on_the_servers_reason_once() {
        let mut state = world();
        at(&mut state, 4190.0, 4000.0);
        let mut h = Harvest::default();
        let now = Instant::now();
        h.click(NODE, &state, now);
        let changes = state.apply_ws(&M::ExtractionFailed {
            player_id: ME,
            reason: "Someone else is already gathering this Wild Grass".into(),
        });
        h.observe(&changes, &state, now);
        assert_eq!(
            h.take_notes(),
            vec![Note::Refused(
                "Someone else is already gathering this Wild Grass".into()
            )]
        );
        assert_eq!(h.asked(), None);
        // The backstop doesn't fire as well.
        h.tick(now + UNANSWERED * 2);
        assert!(h.take_notes().is_empty());
    }
}
