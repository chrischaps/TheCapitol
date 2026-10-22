//! The admin's world layers (D5.3, devtools plan §6): debug drawing over
//! the world, through the Rhai `draw_*` calls (`scripts/debug.rhai`, from
//! `cap_debug()`). F6 steps through them, for an admin's token only:
//! - **interest**: your cell of the server's interest grid (C0.4) and the
//!   eight round it, the square the server sends you players from;
//! - **nodes**: every node near you, standing on its ground, with the
//!   ground's height, slope and massif, which is what `wilds.toml`'s habitats
//!   ask of it. The habitats themselves aren't on the wire, so it can't say
//!   whether the ground suits ([#413](https://github.com/chrischaps/TheCapitol/issues/413));
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
use capitol_core::protocol::rest::{Doing, Walker};
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
                WalkerMark {
                    at,
                    dest: w.dest.map(|[x, y]| to_world(x, y)),
                    name: w.name.clone(),
                    citizen: w.citizen,
                    doing: doing_words(&w.doing, state),
                }
            })
            .collect();
    }
    mark
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
    }

    #[test]
    fn the_ground_in_words() {
        let params = land::LandParams::default();
        // The plain is flat; the ranges are not
        assert_eq!(ground_words(6800.0, 6800.0, &params), "flat");
        let high = (0..360)
            .map(|d| {
                let a = (d as f64).to_radians();
                (6800.0 + 6000.0 * a.cos(), 6800.0 + 6000.0 * a.sin())
            })
            .map(|(x, y)| ground_words(x, y, &params))
            .find(|w| w.contains("massif"));
        assert!(high.as_ref().is_some_and(|w| w.starts_with("h ") && w.contains("slope")), "{high:?}");
    }
}
