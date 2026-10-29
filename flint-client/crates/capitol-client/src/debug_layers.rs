//! The admin's world layers (D5.3, devtools plan §6): debug drawing over
//! the world, through the Rhai `draw_*` calls (`scripts/debug.rhai`, from
//! `cap_debug()`). F6 steps through them, for an admin's token only:
//! - **interest**: your cell of the server's interest grid (C0.4) and the
//!   eight round it, the square the server sends you players from;
//! - **nodes**: every node near you, standing on its ground, with the
//!   ground's height, slope and massif, and for a node in the wilds whether
//!   that ground suits its habitat: `wilds.toml`'s, off `/catalog`, judged by
//!   the planner's own rule (`rules::habitat`, #413), so a vein on the wrong
//!   ground shows at a glance. It doesn't yet shade the ground that would
//!   suit ([#607](https://github.com/chrischaps/TheCapitol/issues/607));
//! - **walks**: the walk graph round you (#267) that every click and every
//!   citizen walks on;
//! - **walkers**: where the server is walking each player near you, from
//!   `/admin/walkers`: a citizen's next leg, a person's click, and what each
//!   is doing. A citizen's goal past its leg waits on the town's report
//!   ([#414](https://github.com/chrischaps/TheCapitol/issues/414));
//!
//! then all of them at once, then off. Each is gathered only while it's
//! shown, from what the store and the walk graph already hold, and the
//! walkers from the server once a second.

use capitol_core::motion::Pose;
use capitol_core::paths::WalkGraph;
use capitol_core::protocol::land;
use capitol_core::protocol::rest::{Doing, Heading, Walker};
use capitol_core::protocol::rules::habitat::{Massifs, Verdict};
use capitol_core::protocol::rules::{interest_cell, INTEREST_CELL};
use capitol_core::state::GameState;

use crate::coords::to_world;

/// How far round you the nodes and the walk graph are drawn, in server
/// units: about the interest square's.
pub const REACH: f64 = 600.0;

/// Which layer F6 has on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Layer {
    #[default]
    Off,
    Interest,
    Nodes,
    Walks,
    Walkers,
    All,
}

impl Layer {
    /// The next one F6 steps to.
    pub fn next(self) -> Layer {
        match self {
            Layer::Off => Layer::Interest,
            Layer::Interest => Layer::Nodes,
            Layer::Nodes => Layer::Walks,
            Layer::Walks => Layer::Walkers,
            Layer::Walkers => Layer::All,
            Layer::All => Layer::Off,
        }
    }

    /// Its name, for the toast and the overlay.
    pub fn name(self) -> &'static str {
        match self {
            Layer::Off => "off",
            Layer::Interest => "interest cells",
            Layer::Nodes => "nodes on their ground",
            Layer::Walks => "the walk graph",
            Layer::Walkers => "where the server walks them",
            Layer::All => "every layer",
        }
    }

    /// Whether it shows `part`, one of the four.
    pub fn shows(self, part: Layer) -> bool {
        self != Layer::Off && part != Layer::Off && (self == part || self == Layer::All)
    }

    /// Whether it wants the walkers from the server.
    pub fn wants_walkers(self) -> bool {
        self.shows(Layer::Walkers)
    }
}

/// What `debug.rhai` draws this frame, in world metres.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mark {
    pub layer: &'static str,
    pub cells: Vec<Cell>,
    pub nodes: Vec<NodeMark>,
    /// The walk graph's edges, `[x0, z0, x1, z1]`.
    pub edges: Vec<[f32; 4]>,
    pub walkers: Vec<WalkerMark>,
}

/// One cell of the interest grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// `(22, 31)`
    pub label: String,
    /// The one you stand in.
    pub mine: bool,
}

/// A node, and the ground it stands on.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeMark {
    pub at: [f32; 2],
    /// `iron_vein`
    pub kind: String,
    /// `h 412 · slope 0.81 · massif 3`
    pub ground: String,
    pub available: bool,
    /// Whether its ground suits its habitat; `None` for a node off the
    /// wilds, one with no habitat, or before the land is known.
    pub fit: Option<Fit>,
}

/// What a node's habitat makes of the ground it stands on.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    pub suits: bool,
    /// `suits`, `wrong ground: h 120, wants 380 to 900`, or `wrong massif:
    /// keeps to the odd`
    pub words: String,
}

/// A player the server is moving.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkerMark {
    /// Where they're drawn: their pose, or the server's position for one
    /// out of sight.
    pub at: [f32; 2],
    /// Where the server is walking them.
    pub dest: Option<[f32; 2]>,
    pub name: String,
    pub citizen: bool,
    /// `crafting rope`; empty when idle.
    pub doing: String,
    /// Where a citizen's walk is bound for, past the leg, and why (#414).
    pub goal: Option<GoalMark>,
}

/// Where a citizen's walk ends, from its town (#414).
#[derive(Debug, Clone, PartialEq)]
pub struct GoalMark {
    pub at: [f32; 2],
    /// `gather clay · 340 m`: what it set out to do, and how far it still
    /// has to go as the crow flies
    pub words: String,
    /// The leg ends there: this is the walk's last
    pub last_leg: bool,
}

/// Gathers what `layer` shows round `me` (server units). `poses` place the
/// walkers where they're drawn.
pub fn gather(
    layer: Layer,
    me: (f64, f64),
    state: &GameState,
    graph: Option<&WalkGraph>,
    walkers: &[Walker],
    poses: &[Pose],
) -> Mark {
    let mut mark = Mark { layer: layer.name(), ..Default::default() };
    if layer.shows(Layer::Interest) {
        mark.cells = cells(me);
    }
    if layer.shows(Layer::Nodes) {
        let params = graph.map(|g| g.ground().land());
        let mut near: Vec<_> = state
            .nodes()
            .values()
            .filter(|n| (n.x - me.0).powi(2) + (n.y - me.1).powi(2) <= REACH * REACH)
            .collect();
        // Nearest first, so a crowd keeps its nearest labels on top
        near.sort_by(|a, b| {
            let d = |n: &&capitol_core::protocol::rest::ResourceNodeInfo| (n.x - me.0).powi(2) + (n.y - me.1).powi(2);
            d(a).total_cmp(&d(b))
        });
        mark.nodes = near
            .into_iter()
            .map(|n| NodeMark {
                at: to_world(n.x, n.y),
                kind: n.resource_type.clone(),
                ground: params.map_or_else(String::new, |p| ground_words(n.x, n.y, p)),
                available: n.state == "available",
                fit: params.and_then(|p| fit(&n.resource_type, n.x, n.y, state, p)),
            })
            .collect();
    }
    if layer.shows(Layer::Walks) {
        if let Some(graph) = graph {
            mark.edges = graph
                .edges_near(me, REACH)
                .into_iter()
                .map(|(a, b)| {
                    let (a, b) = (to_world(a.0, a.1), to_world(b.0, b.1));
                    [a[0], a[1], b[0], b[1]]
                })
                .collect();
        }
    }
    if layer.shows(Layer::Walkers) {
        mark.walkers = walkers
            .iter()
            .filter(|w| Some(w.player_id) != state.me() || w.dest.is_some())
            .map(|w| {
                let at = poses
                    .iter()
                    .find(|p| p.id == w.player_id)
                    .map_or_else(|| to_world(w.x, w.y), |p| to_world(p.x, p.y));
                let dest = w.dest.map(|[x, y]| to_world(x, y));
                WalkerMark {
                    at,
                    dest,
                    name: w.name.clone(),
                    citizen: w.citizen,
                    doing: doing_words(&w.doing, state),
                    goal: w.heading.as_ref().map(|h| goal(at, dest, h)),
                }
            })
            .collect();
    }
    mark
}

/// A heading as the layer draws it: its end, and its words with the way
/// still to go.
fn goal(at: [f32; 2], dest: Option<[f32; 2]>, heading: &Heading) -> GoalMark {
    let to = to_world(heading.to[0], heading.to[1]);
    let far = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    // Within a step of the leg's end, the walk has one leg left
    let last_leg = dest.is_some_and(|d| far(d, to) < 1.0);
    let words = match far(at, to) {
        m if m < 10.0 => format!("{} · {m:.1} m", heading.why),
        m => format!("{} · {m:.0} m", heading.why),
    };
    GoalMark { at: to, words, last_leg }
}

/// Your cell of the interest grid and the eight round it.
fn cells(me: (f64, f64)) -> Vec<Cell> {
    let (cx, cy) = interest_cell(me.0, me.1);
    let mut out = Vec::with_capacity(9);
    for j in cy - 1..=cy + 1 {
        for i in cx - 1..=cx + 1 {
            let (x0, y0) = (i as f64 * INTEREST_CELL, j as f64 * INTEREST_CELL);
            out.push(Cell {
                min: to_world(x0, y0),
                max: to_world(x0 + INTEREST_CELL, y0 + INTEREST_CELL),
                label: format!("({i}, {j})"),
                mine: (i, j) == (cx, cy),
            });
        }
    }
    out
}

/// The ground at (x, y) as the habitats read it: `h 412 · slope 0.81 ·
/// massif 3`. Flat ground says so.
pub fn ground_words(x: f64, y: f64, params: &land::LandParams) -> String {
    let h = land::height_at(x, y, params);
    if h <= 0.0 {
        return "flat".to_owned();
    }
    let slope = land::slope_at(x, y, params);
    match land::massif_at(x, y, params) {
        Some(m) => format!("h {h:.0} · slope {slope:.2} · massif {m}"),
        None => format!("h {h:.0} · slope {slope:.2}"),
    }
}

/// What `resource`'s habitat makes of the ground at (x, y): judged only in
/// the wilds, where the habitats rule (W6), and only for a resource that
/// has one.
pub fn fit(resource: &str, x: f64, y: f64, state: &GameState, params: &land::LandParams) -> Option<Fit> {
    let frame = &params.frame;
    let r = (x - frame.center.0).hypot(y - frame.center.1) - frame.rural_edge();
    if r < 0.0 {
        return None;
    }
    let habitat = state.catalog().habitat(resource)?;
    let words = match habitat.verdict_at(x, y, params) {
        Verdict::Suits => "suits".to_owned(),
        Verdict::WrongMassif => match habitat.massifs {
            Some(Massifs::Even) => "wrong massif: keeps to the even".to_owned(),
            _ => "wrong massif: keeps to the odd".to_owned(),
        },
        Verdict::WrongGround => {
            let measures = [
                ("h", land::height_at(x, y, params), habitat.height, 0),
                ("slope", land::slope_at(x, y, params), habitat.slope, 2),
                ("depth", r, habitat.depth, 0),
            ];
            let out: Vec<String> = measures
                .into_iter()
                .filter_map(|(name, v, b, places)| {
                    let [lo, hi] = b?;
                    (!(lo..=hi).contains(&v))
                        .then(|| format!("{name} {v:.places$}, wants {lo:.places$} to {hi:.places$}"))
                })
                .collect();
            format!("wrong ground: {}", out.join(", "))
        }
    };
    Some(Fit { suits: words == "suits", words })
}

/// What a walker is doing, in a few words; empty when idle.
fn doing_words(doing: &Doing, state: &GameState) -> String {
    match doing {
        Doing::Idle => String::new(),
        Doing::Extracting { node_id } => match state.node(*node_id) {
            Some(n) => format!("gathering {}", n.resource_type.replace('_', " ")),
            None => "gathering".to_owned(),
        },
        Doing::Crafting { recipe_id } => format!("crafting {}", recipe_id.replace('_', " ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::protocol::rest::ResourceNodeInfo;
    use uuid::Uuid;

    #[test]
    fn f6_steps_through_them_and_back_to_off() {
        let mut layer = Layer::Off;
        let mut seen = Vec::new();
        for _ in 0..6 {
            layer = layer.next();
            seen.push(layer);
        }
        assert_eq!(
            seen,
            [Layer::Interest, Layer::Nodes, Layer::Walks, Layer::Walkers, Layer::All, Layer::Off]
        );
        assert!(Layer::All.shows(Layer::Nodes) && Layer::All.wants_walkers());
        assert!(!Layer::Nodes.shows(Layer::Interest));
        assert!(!Layer::Off.shows(Layer::Off) && !Layer::Off.wants_walkers());
    }

    #[test]
    fn the_cells_are_yours_and_the_eight_round_it() {
        let c = INTEREST_CELL;
        let cells = cells((22.0 * c + 10.0, 31.0 * c + 290.0));
        assert_eq!(cells.len(), 9);
        let mine: Vec<_> = cells.iter().filter(|c| c.mine).collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].label, "(22, 31)");
        assert_eq!(mine[0].min, to_world(22.0 * c, 31.0 * c));
        assert_eq!(cells[0].label, "(21, 30)");
        assert_eq!(cells[8].max, to_world(24.0 * c, 33.0 * c));
    }

    #[test]
    fn nodes_and_walkers_are_gathered_round_you() {
        let mut state = GameState::new();
        let near = ResourceNodeInfo {
            id: Uuid::from_u128(1),
            resource_type: "iron_vein".into(),
            x: 6900.0,
            y: 6800.0,
            state: "available".into(),
            base_quality: None,
        };
        let far = ResourceNodeInfo { id: Uuid::from_u128(2), x: 6800.0 + REACH + 10.0, ..near.clone() };
        state.apply_rest(&capitol_core::net::RestResponse::AllNodes(capitol_core::protocol::rest::NearbyResponse {
            nodes: vec![near.clone(), far],
        }));
        let walker = Walker {
            player_id: Uuid::from_u128(9),
            name: "Ada".into(),
            citizen: true,
            x: 6810.0,
            y: 6790.0,
            dest: Some([6900.0, 6800.0]),
            doing: Doing::Extracting { node_id: near.id },
            heading: None,
        };
        let pose = Pose { id: walker.player_id, x: 6820.0, y: 6795.0, facing: 0.0, moving: true };
        let off = gather(Layer::Off, (6800.0, 6800.0), &state, None, std::slice::from_ref(&walker), &[pose]);
        assert_eq!(off, Mark { layer: "off", ..Default::default() });

        let all = gather(Layer::All, (6800.0, 6800.0), &state, None, &[walker], &[pose]);
        assert_eq!(all.cells.len(), 9);
        assert_eq!(all.nodes.len(), 1, "only the node within reach");
        assert_eq!(all.nodes[0].kind, "iron_vein");
        assert!(all.nodes[0].available);
        assert!(all.edges.is_empty(), "no graph, no edges");
        let ada = &all.walkers[0];
        assert_eq!(ada.at, to_world(6820.0, 6795.0), "drawn where its pose is");
        assert_eq!(ada.dest, Some(to_world(6900.0, 6800.0)));
        assert_eq!(ada.doing, "gathering iron vein");
        assert_eq!(ada.goal, None, "no town, no goal");
    }

    /// #414: a citizen's walk is drawn on past its leg to where it's bound,
    /// with what it set out to do and how far it has to go
    #[test]
    fn a_citizens_goal_lies_past_its_leg() {
        let state = GameState::new();
        let walker = |dest: [f64; 2]| Walker {
            player_id: Uuid::from_u128(9),
            name: "Ada".into(),
            citizen: true,
            x: 6800.0,
            y: 6800.0,
            dest: Some(dest),
            doing: Doing::Idle,
            heading: Some(Heading { to: [6800.0, 6100.0], why: "gather clay".into() }),
        };
        let first = gather(Layer::Walkers, (6800.0, 6800.0), &state, None, &[walker([6900.0, 6700.0])], &[]);
        let goal = first.walkers[0].goal.as_ref().unwrap();
        assert_eq!(goal.at, to_world(6800.0, 6100.0));
        let metres = (to_world(6800.0, 6100.0)[1] - to_world(6800.0, 6800.0)[1]).abs();
        assert_eq!(goal.words, format!("gather clay · {metres:.0} m"));
        assert!(!goal.last_leg, "a leg to the bridge first");
        let last = gather(Layer::Walkers, (6800.0, 6800.0), &state, None, &[walker([6800.0, 6100.0])], &[]);
        assert!(last.walkers[0].goal.as_ref().unwrap().last_leg, "the leg is the walk's last");
    }

    #[test]
    fn the_ground_in_words() {
        let params = land::LandParams::default();
        // The plain is flat; the ranges are not
        assert_eq!(ground_words(8300.0, 8300.0, &params), "flat");
        let high = (0..360)
            .map(|d| {
                let a = (d as f64).to_radians();
                (8300.0 + 7500.0 * a.cos(), 8300.0 + 7500.0 * a.sin())
            })
            .map(|(x, y)| ground_words(x, y, &params))
            .find(|w| w.contains("massif"));
        assert!(high.as_ref().is_some_and(|w| w.starts_with("h ") && w.contains("slope")), "{high:?}");
    }

    /// #413: the layer judges a node's ground by its habitat off the
    /// catalog, in the wilds only, and says what's wrong.
    #[test]
    fn a_node_is_judged_by_its_habitat() {
        let state = GameState::new();
        let params = land::LandParams::default();
        let (cx, cy) = params.frame.center;
        // Inside the rural edge nothing is judged; grass has no habitat
        assert_eq!(fit("iron_vein", cx, cy + 100.0, &state, &params), None);
        let wild = (cx, cy + params.frame.rural_edge() + 50.0);
        assert_eq!(fit("grass", wild.0, wild.1, &state, &params), None);
        // Just past the rural edge is flat: no place for a vein, fine for branches
        let vein = fit("iron_vein", wild.0, wild.1, &state, &params).unwrap();
        assert!(!vein.suits && vein.words.starts_with("wrong ground: h 0, wants 380 to 900"), "{}", vein.words);
        assert_eq!(fit("fallen_branches", wild.0, wild.1, &state, &params).map(|f| f.suits), Some(true));
        // Somewhere in the ranges each of copper and tin is on its own massif
        let ring = (0..720).map(|d| {
            let a = (d as f64 / 2.0).to_radians();
            (cx + 7600.0 * a.cos(), cy + 7600.0 * a.sin())
        });
        let massif_words: Vec<_> = ring
            .filter_map(|(x, y)| fit("copper_vein", x, y, &state, &params))
            .map(|f| f.words)
            .filter(|w| w.starts_with("wrong massif") || w == "suits")
            .collect();
        assert!(massif_words.iter().any(|w| w == "suits"), "copper suits somewhere on the ring");
        assert!(massif_words.iter().any(|w| w == "wrong massif: keeps to the even"), "{massif_words:?}");
    }
}
