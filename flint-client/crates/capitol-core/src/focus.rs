//! What's in front of you (controls T1.5, plan D5): over the shoulder the
//! cursor is captured, so the game picks what E, or a left click, acts on.
//! Only the choosing is new. Acting goes through the same `Session` paths a
//! click does, so reach is still judged from the server position and a
//! target a step away is walked to, as today.
//!
//! - **Candidates** ([`Candidate`]): the available nodes, the stations, the
//!   other players ([`from_state`]), and the hall, the bazaar's stalls, the
//!   Gazette's board and the Deeds Office, which the client adds from where
//!   it draws them ([`Candidate::area`]).
//! - **In front:** within [`MAX_ANGLE`] of the view, and within the kind's
//!   reach plus [`SLACK`]. A footprint counts by its nearest point in front.
//! - **Scoring:** the angle off the view counts twice what the distance
//!   does, each as a share of its limit, so a target in the middle of the
//!   view beats a nearer one off to the side. Ties go to the kind (nodes
//!   first, then stations, players, the board, the office, stalls, the
//!   hall), then the id, so the same scene always picks the same target.
//! - **Holding:** the target that had the focus keeps it until another
//!   scores [`HOLD`] better ([`focus_holding`]), so it doesn't flicker
//!   between two about as far off the view.
//!
//! All in server units: x east, y south, as the wire has them.

use capitol_protocol::rules::{self, EXTRACTION_RANGE, STATION_INTERACTION_RANGE, TRADE_RANGE};
use capitol_protocol::rest::ResourceNodeInfo;
use uuid::Uuid;

use crate::catalog::Catalog;
use crate::state::GameState;

/// The widest a target may be off the view and still be in front: 35°.
pub const MAX_ANGLE: f64 = 35.0 * std::f64::consts::PI / 180.0;

/// How far past its reach a target is still in front: 3 m. E on one there
/// walks the step to it, as a click does.
pub const SLACK: f64 = 30.0;

/// The reach of things that work from anywhere (the Gazette's board, the
/// Deeds Office): near enough that you're looking at them, not across the
/// square at them. 5 m, a node's.
pub const LOOK_REACH: f64 = EXTRACTION_RANGE;

/// How much better another target must score to take the focus from the
/// one that has it: about 3.5° off the view, or a fifth of the reach.
pub const HOLD: f64 = 0.1;

/// How many points along each side of a footprint are tried for the
/// nearest one in front.
const EDGE_SAMPLES: usize = 16;

/// What a target is. The order is the tie-break: earlier wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Node,
    Station,
    Player,
    Board,
    Office,
    Stall,
    Hall,
}

impl Kind {
    /// How far off a target of this kind can be acted on without a walk.
    pub fn reach(self) -> f64 {
        match self {
            Kind::Node => EXTRACTION_RANGE,
            Kind::Station => STATION_INTERACTION_RANGE,
            Kind::Player => TRADE_RANGE,
            Kind::Board | Kind::Office => LOOK_REACH,
            Kind::Stall => rules::bazaar::STALL_RANGE,
            Kind::Hall => rules::market::HALL_RANGE,
        }
    }

    /// The kind [`Kind::name`] names.
    pub fn named(name: &str) -> Option<Kind> {
        [Kind::Node, Kind::Station, Kind::Player, Kind::Board, Kind::Office, Kind::Stall, Kind::Hall]
            .into_iter()
            .find(|k| k.name() == name)
    }

    /// Its name for the scripts.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Node => "node",
            Kind::Station => "station",
            Kind::Player => "player",
            Kind::Board => "board",
            Kind::Office => "office",
            Kind::Stall => "stall",
            Kind::Hall => "hall",
        }
    }
}

/// Where a target is: a point, or a footprint (a convex quad).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Point(f64, f64),
    Area([(f64, f64); 4]),
}

/// Something you might act on.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub kind: Kind,
    pub id: String,
    /// What E does, in words: "Gather wild grass".
    pub label: String,
    /// What Shift-E does, if it does something else: "Trade with Ada".
    pub shift_label: Option<String>,
    pub shape: Shape,
}

/// The target in front of you.
#[derive(Debug, Clone, PartialEq)]
pub struct Focus {
    pub kind: Kind,
    pub id: String,
    pub label: String,
    pub shift_label: Option<String>,
    /// The point of it you're looking at: the middle of a node, a station
    /// or a player, the nearest point in front on a footprint.
    pub at: (f64, f64),
    /// How far it is, from you to `at`.
    pub distance: f64,
}

impl Candidate {
    pub fn point(kind: Kind, id: impl Into<String>, label: impl Into<String>, at: (f64, f64)) -> Self {
        Candidate { kind, id: id.into(), label: label.into(), shift_label: None, shape: Shape::Point(at.0, at.1) }
    }

    pub fn area(kind: Kind, id: impl Into<String>, label: impl Into<String>, corners: [(f64, f64); 4]) -> Self {
        Candidate { kind, id: id.into(), label: label.into(), shift_label: None, shape: Shape::Area(corners) }
    }

    /// Where it is if it's in front of you, scored: lower is better.
    fn judge(&self, you: (f64, f64), view: (f64, f64)) -> Option<(f64, (f64, f64), f64)> {
        let limit = self.kind.reach() + SLACK;
        match self.shape {
            Shape::Point(x, y) => {
                let d = (x - you.0).hypot(y - you.1);
                let a = off_view(you, view, (x, y))?;
                (d <= limit && a <= MAX_ANGLE).then(|| (score(a, d, limit), (x, y), d))
            }
            Shape::Area(corners) => {
                if contains(&corners, you) {
                    return Some((0.0, you, 0.0));
                }
                let mut best: Option<(f64, (f64, f64), f64)> = None;
                for i in 0..4 {
                    let (p, q) = (corners[i], corners[(i + 1) % 4]);
                    for k in 0..=EDGE_SAMPLES {
                        let t = k as f64 / EDGE_SAMPLES as f64;
                        let at = (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
                        let d = (at.0 - you.0).hypot(at.1 - you.1);
                        let Some(a) = off_view(you, view, at) else {
                            continue;
                        };
                        if d > limit || a > MAX_ANGLE {
                            continue;
                        }
                        let s = score(a, d, limit);
                        if best.is_none_or(|(b, _, _)| s < b) {
                            best = Some((s, at, d));
                        }
                    }
                }
                best
            }
        }
    }
}

/// The angle between the view and the way to `at`, or `None` if there's
/// no view, or you stand on it.
fn off_view(you: (f64, f64), view: (f64, f64), at: (f64, f64)) -> Option<f64> {
    let (dx, dy) = (at.0 - you.0, at.1 - you.1);
    let (d, v) = (dx.hypot(dy), view.0.hypot(view.1));
    if v < 1e-9 {
        return None;
    }
    if d < 1e-9 {
        return Some(0.0);
    }
    let cos = ((dx * view.0 + dy * view.1) / (d * v)).clamp(-1.0, 1.0);
    Some(cos.acos())
}

fn score(angle: f64, distance: f64, limit: f64) -> f64 {
    angle / MAX_ANGLE + 0.5 * distance / limit
}

/// Whether `p` is inside the convex quad.
fn contains(corners: &[(f64, f64); 4], p: (f64, f64)) -> bool {
    let side = |a: (f64, f64), b: (f64, f64)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let s: Vec<f64> = (0..4).map(|i| side(corners[i], corners[(i + 1) % 4])).collect();
    s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0)
}

/// The target in front of you at `you`, looking along `view` (any length),
/// among `candidates`. `None` when nothing is.
pub fn focus(candidates: &[Candidate], you: (f64, f64), view: (f64, f64)) -> Option<Focus> {
    focus_holding(candidates, you, view, None)
}

/// [`focus`], but the target `held` last frame keeps the focus while it's
/// still in front, until another beats it by [`HOLD`]. Without it, two
/// targets about as far off the view swap back and forth as the camera
/// settles, and E lands on whichever had it that frame.
pub fn focus_holding(
    candidates: &[Candidate],
    you: (f64, f64),
    view: (f64, f64),
    held: Option<(Kind, &str)>,
) -> Option<Focus> {
    candidates
        .iter()
        .filter_map(|c| c.judge(you, view).map(|j| (c, j)))
        .map(|(c, (s, at, d))| {
            let kept = held.is_some_and(|(kind, id)| kind == c.kind && id == c.id);
            (c, (if kept { s - HOLD } else { s }, at, d))
        })
        .min_by(|(a, (sa, _, _)), (b, (sb, _, _))| {
            sa.total_cmp(sb).then(a.kind.cmp(&b.kind)).then(a.id.cmp(&b.id))
        })
        .map(|(c, (_, at, distance))| Focus {
            kind: c.kind,
            id: c.id.clone(),
            label: c.label.clone(),
            shift_label: c.shift_label.clone(),
            at,
            distance,
        })
}

/// "Gather wild grass": the resource's name from the catalog, lowered.
pub fn gather_label(catalog: &Catalog, resource_type: &str) -> String {
    let name = catalog
        .resource_type(resource_type)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| resource_type.replace('_', " "));
    format!("Gather {}", name.to_lowercase())
}

/// "Open the forge": the station's name, lowered.
pub fn open_label(name: &str) -> String {
    format!("Open the {}", name.to_lowercase())
}

/// A node to gather, by itself: offline samples have no store to ask
/// whether it's available.
pub fn node(n: &ResourceNodeInfo, catalog: &Catalog) -> Candidate {
    Candidate::point(Kind::Node, n.id.to_string(), gather_label(catalog, &n.resource_type), (n.x, n.y))
}

/// The nodes, the stations and the other players the store knows: every
/// available node, every station, and everyone but you. A player's E is
/// their card, as a click is, and Shift-E asks them to trade, as a
/// shift-click does.
pub fn from_state(state: &GameState) -> Vec<Candidate> {
    let catalog = state.catalog();
    let mut out = Vec::new();
    for n in state.nodes().values() {
        if state.node_available(n.id) {
            out.push(node(n, catalog));
        }
    }
    for s in state.stations().values() {
        out.push(Candidate::point(Kind::Station, s.id.to_string(), open_label(&s.name), (s.x, s.y)));
    }
    let me: Option<Uuid> = state.me();
    for p in state.players().values().filter(|p| Some(p.id) != me) {
        let mut c = Candidate::point(Kind::Player, p.id.to_string(), p.name.clone(), (p.x, p.y));
        c.shift_label = Some(format!("Trade with {}", p.name));
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, x: f64, y: f64) -> Candidate {
        Candidate::point(Kind::Node, id, format!("Gather {id}"), (x, y))
    }

    const EAST: (f64, f64) = (1.0, 0.0);

    #[test]
    fn prefers_the_centred_target_over_a_nearer_one_off_to_the_side() {
        // 40 straight ahead, and 20 off at 30°
        let side = (20.0 * 30f64.to_radians().cos(), 20.0 * 30f64.to_radians().sin());
        let cs = [node("side", side.0, side.1), node("ahead", 40.0, 0.0)];
        let f = focus(&cs, (0.0, 0.0), EAST).unwrap();
        assert_eq!(f.id, "ahead");
        assert!((f.distance - 40.0).abs() < 1e-9);
        // Alone, the one to the side is in front
        assert_eq!(focus(&cs[..1], (0.0, 0.0), EAST).unwrap().id, "side");
    }

    #[test]
    fn gives_up_past_35_degrees() {
        let at = |deg: f64| node("n", 30.0 * deg.to_radians().cos(), 30.0 * deg.to_radians().sin());
        assert!(focus(&[at(34.0)], (0.0, 0.0), EAST).is_some());
        assert!(focus(&[at(-34.0)], (0.0, 0.0), EAST).is_some());
        assert!(focus(&[at(36.0)], (0.0, 0.0), EAST).is_none());
        assert!(focus(&[at(180.0)], (0.0, 0.0), EAST).is_none(), "behind you");
    }

    #[test]
    fn respects_each_kinds_reach() {
        for kind in [Kind::Node, Kind::Station, Kind::Player, Kind::Board, Kind::Office] {
            let edge = kind.reach() + SLACK;
            let near = Candidate::point(kind, "a", "a", (edge - 1.0, 0.0));
            let far = Candidate::point(kind, "a", "a", (edge + 1.0, 0.0));
            assert!(focus(&[near], (0.0, 0.0), EAST).is_some(), "{kind:?} in reach");
            assert!(focus(&[far], (0.0, 0.0), EAST).is_none(), "{kind:?} past reach");
        }
        // A station reaches further than a node
        assert!(Kind::Station.reach() > Kind::Node.reach());
        let n = node("n", 100.0, 0.0);
        let s = Candidate::point(Kind::Station, "s", "Open the forge", (100.0, 0.0));
        assert!(focus(&[n], (0.0, 0.0), EAST).is_none());
        assert!(focus(&[s], (0.0, 0.0), EAST).is_some());
    }

    #[test]
    fn breaks_ties_deterministically() {
        // Two nodes on the same spot, in either order: the lower id
        let (a, b) = (node("a", 30.0, 0.0), node("b", 30.0, 0.0));
        let one = focus(&[a.clone(), b.clone()], (0.0, 0.0), EAST).unwrap();
        let two = focus(&[b, a], (0.0, 0.0), EAST).unwrap();
        assert_eq!((one.id.as_str(), two.id.as_str()), ("a", "a"));
        // The same spot, the same reach, two kinds: the earlier kind
        let board = Candidate::point(Kind::Board, "a", "The Gazette", (30.0, 0.0));
        let office = Candidate::point(Kind::Office, "0", "The Deeds Office", (30.0, 0.0));
        assert_eq!(focus(&[office.clone(), board.clone()], (0.0, 0.0), EAST).unwrap().kind, Kind::Board);
        assert_eq!(focus(&[board, office], (0.0, 0.0), EAST).unwrap().kind, Kind::Board);
        // Mirror images either side of the view score alike: the lower id
        let l = node("left", 30.0, 10.0);
        let r = node("right", 30.0, -10.0);
        assert_eq!(focus(&[r.clone(), l.clone()], (0.0, 0.0), EAST).unwrap().id, "left");
        assert_eq!(focus(&[l, r], (0.0, 0.0), EAST).unwrap().id, "left");
    }

    #[test]
    fn a_footprint_counts_by_its_nearest_point_in_front() {
        // A hall 80 wide, its near wall 60 off, square across the view
        let hall = [(60.0, -40.0), (140.0, -40.0), (140.0, 40.0), (60.0, 40.0)];
        let c = Candidate::area(Kind::Hall, "exchange_hall", "The Exchange hall", hall);
        let f = focus(std::slice::from_ref(&c), (0.0, 0.0), EAST).unwrap();
        assert_eq!(f.at, (60.0, 0.0));
        assert!((f.distance - 60.0).abs() < 1e-9);
        // Looking at its corner, 45° off square: the wall near the corner
        let slant = focus(std::slice::from_ref(&c), (0.0, 0.0), (1.0, 1.0)).unwrap();
        assert!(slant.at.0 == 60.0 && slant.at.1 > 0.0, "{:?}", slant.at);
        // Looking south, the corner is 56° off: not in front
        assert!(focus(std::slice::from_ref(&c), (0.0, 0.0), (0.0, 1.0)).is_none());
        // Looking away, nothing
        assert!(focus(std::slice::from_ref(&c), (0.0, 0.0), (-1.0, 0.0)).is_none());
        // Inside it, it's in front whichever way you look
        let inside = focus(&[c], (100.0, 0.0), (-1.0, 0.0)).unwrap();
        assert_eq!(inside.distance, 0.0);
    }

    #[test]
    fn the_focus_holds_until_another_is_clearly_better() {
        // 5.7° and 4.8° either side of the view
        let (a, b) = (node("a", 30.0, 3.0), node("b", 30.0, -2.5));
        let cs = [a, b];
        // b is a hair nearer the middle, so it has the focus from nothing
        assert_eq!(focus(&cs, (0.0, 0.0), EAST).unwrap().id, "b");
        // but a, which had it, keeps it
        let held = focus_holding(&cs, (0.0, 0.0), EAST, Some((Kind::Node, "a"))).unwrap();
        assert_eq!(held.id, "a");
        // until the view comes round well onto b
        let toward_b = (1.0, -0.1); // 6° round: b 1°, a 12° off
        assert_eq!(focus_holding(&cs, (0.0, 0.0), toward_b, Some((Kind::Node, "a"))).unwrap().id, "b");
        // and one that's gone from in front holds nothing
        assert_eq!(focus_holding(&cs[1..], (0.0, 0.0), EAST, Some((Kind::Node, "a"))).unwrap().id, "b");
    }

    #[test]
    fn no_view_no_focus() {
        assert!(focus(&[node("n", 10.0, 0.0)], (0.0, 0.0), (0.0, 0.0)).is_none());
    }
}
