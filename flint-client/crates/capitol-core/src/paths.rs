//! Walking round the water (C0.5, #211): a walk graph from `/terrain`, and A*
//! over it.
//!
//! The server walks a player in a straight line to where it was sent, and
//! stops it at the water: the moat and canals except on their bridges, a
//! river or lake except at a ford or on a road, and ground steeper than
//! [`CLIFF_SLOPE`](ground::CLIFF_SLOPE). It has no pathfinding, so a bot sent across a canal
//! stands on the bank. [`WalkGraph::route`] plans a walk the server lets
//! through, as waypoints to send one `move` each.
//!
//! - **The graph** is the roads and the wild trails. Every vertex is a node,
//!   a long segment is cut into pieces of [`SPAN`], and roads that meet
//!   within [`WELD`] without sharing a vertex are joined. On the default
//!   world that's one network already (#341). Open ground joins whatever
//!   is left apart: a road's loose end, or a network still apart, gets a
//!   line to the nearest node of another within [`JOIN`]. The bridges are
//!   roads over the rings, the fords are on the trails, and a trail over a
//!   river is its bridge, so the graph crosses water only where the server
//!   lets you.
//! - **A walk** goes straight when it can. [`WalkGraph::walkable`] is the
//!   server's own test, run along the line more finely than the server's
//!   steps, so a leg it lets through the server walks from wherever on it
//!   the walk begins (#280). Otherwise it joins the graph at the nearest
//!   nodes in sight (in a straight line, or by A* over a grid when none is:
//!   a vein up a scree, a valley floor a kilometre from its trail), crosses
//!   it by A*, and leaves it the same way. Where there's no way,
//!   [`WalkGraph::plan`] says which end it's at.
//! - **The waypoints** are pulled tight. A stretch becomes one leg while its
//!   straight line is walkable, no slower than the way round, and within
//!   [`DEVIATION`] of it, so walkers keep to the streets and still send few
//!   `move`s. A connection may send 60 commands a minute.
//!
//! The graph is built once per terrain version and shared by every bot in
//! the process ([`shared`]).
//!
//! ```no_run
//! # use capitol_core::paths::WalkGraph;
//! # let terrain = capitol_core::terrain::read_terrain_file("fixtures/terrain.json".as_ref()).unwrap();
//! let walks = WalkGraph::new(&terrain);
//! // From the spawn on the south arterial to inside the outer canal, by its bridge.
//! if let Some(waypoints) = walks.route((6800.0, 10150.0), (7500.0, 6900.0)) {
//!     for (x, y) in waypoints {
//!         // send `move` to (x, y) and wait to arrive
//!     }
//! }
//! ```

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::{Arc, Mutex};

use capitol_protocol::land;
use capitol_protocol::rules::ground::{self, Footing, Ground};
use capitol_protocol::rules::{self, Frame, Zone, PLAYER_SPEED};
use capitol_protocol::terrain::{Landmark, LandmarkKind, TerrainPlot, TerrainResponse, WaterRing};
use capitol_protocol::tutorial::stops;

use crate::citizen::idle::Places;

mod walk;
pub use walk::{Planning, Walk};

/// The longest stretch of road between two nodes. A street that ends on a
/// long arterial segment has a node of it within half of this.
pub const SPAN: f64 = 40.0;
/// Nodes of different roads this close are joined, if the way between them
/// is walkable: a T-junction whose stem ends beside the through road.
pub const WELD: f64 = 30.0;
/// How far across open ground a loose road end, or a network apart from the
/// rest, is joined to another. It was 600 while the old rural trails
/// stopped short of the suburb and the passes (#211). Since the farms'
/// lanes weld, the default world's roads make one network without it, so it
/// joins nothing there, at any reach (#341). It's kept, short, for a world
/// whose roads don't all meet.
pub const JOIN: f64 = 150.0;
/// How far a pulled-tight leg may stray from the way it replaces.
pub const DEVIATION: f64 = 25.0;

/// How many rounds of joining: each joins networks the last one made.
const JOIN_ROUNDS: usize = 8;
/// Of a network apart from the largest, every this many nodes looks for a join.
const JOIN_SAMPLE: u32 = 8;
/// The nearest nodes of other networks a join tries.
const JOIN_TRIES: usize = 3;
/// How far apart a line's water tests are: a quarter of the server's
/// ([`Ground::step`]), whose tests fall anywhere along the line, so a line
/// that clips the corner of a lake by half a unit still finds it (#280).
const WATER_SAMPLE: f64 = 0.25;
/// How far apart the water is searched where a line goes in or out of it.
const FINE: f64 = 0.05;
/// How far apart a leg's speed is sampled for its cost.
const COST_SAMPLE: f64 = 10.0;
/// The side of the node index's buckets.
const BUCKET: f64 = 50.0;
/// Where to look for a way onto the graph from a point: near first, then
/// farther if nothing near is in sight.
const ACCESS_RADII: [f64; 2] = [250.0, 800.0];
/// The nearest nodes tried at each radius, besides the nearest in each of
/// [`ACCESS_SECTORS`] directions.
const ACCESS_NEAREST: usize = 4;
const ACCESS_SECTORS: usize = 8;
/// The grid a point with no node in sight crosses to reach one: a vein in
/// the ranges can be 1300 from the nearest trail (#280).
const GRID_CELL: f64 = 10.0;
const GRID_REACH: f64 = 1600.0;
/// How far from a cell of that grid a node counts as in sight of it.
const GRID_SIGHT: f64 = 30.0;
/// The nearest nodes the grid's search heads for. A weighted A* looks at
/// fewer cells, but the nearest nodes are often over a ridge, and it spends
/// them all against the ridge.
const GRID_TARGETS: usize = 16;
/// The most cells it looks at before it gives up: a pocket of ground walled
/// by crests, say, where since #347 no wild node is planted. Most ways out
/// of the ranges take under 9000, but about one wild node in 60 is far
/// round the crests from a trail and takes more, so it's given up on
/// ([#521](https://github.com/chrischaps/TheCapitol/issues/521)). Giving up
/// can take most of a second, which is why the Flint client plans a click
/// on a thread of its own ([`Planning`], #348).
const GRID_CELLS: usize = 20_000;
/// How many of the grid's cells it looks at between asking whether the walk
/// is still wanted ([`WalkGraph::plan_unless`]).
const GRID_ASK: usize = 64;
/// The slowest the cost lets a leg go, as a share of full speed.
const SLOWEST: f64 = 0.05;

type Point = (f64, f64);

/// The roads and trails, as a graph a walker can cross, and the ground and
/// water to test any other line against.
pub struct WalkGraph {
    version: Option<String>,
    center: Point,
    rings: Vec<WaterRing>,
    ground: Ground,
    landmarks: Vec<Landmark>,
    /// The bazaar quarter's streets (E3.12), for the walk to a stall.
    bazaar_streets: Vec<Vec<Point>>,
    /// The places worth being, for a citizen's free time (C2.1).
    places: Places,
    /// The lots near the road in's stops (#445), for a citizen walking it
    /// (#450): those within [`STOP_LOTS_WITHIN`] of each stop's point.
    stop_lots: Vec<TerrainPlot>,
    nodes: Vec<Point>,
    /// Each node's neighbours, and the cost of the way there.
    edges: Vec<Vec<(u32, f64)>>,
    buckets: HashMap<(i64, i64), Vec<u32>>,
}

/// Why [`WalkGraph::plan`] found no way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoWay {
    /// No way from where the walk starts onto the graph: a pocket of ground
    /// walled by crests, which a walker can't get into, so only a respawn or
    /// a regenerate puts one there. Nothing walks it out yet
    /// ([#522](https://github.com/chrischaps/TheCapitol/issues/522)).
    Stranded,
    /// No way from the graph to where it ends: in such a pocket, where since
    /// #347 no wild node is planted, or too far round the crests for the
    /// grid's search, as about one wild node in 60 is
    /// ([#521](https://github.com/chrischaps/TheCapitol/issues/521)).
    Unreachable,
}

/// A plan given up because nobody wants it any more ([`WalkGraph::plan_unless`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gone;

/// Which way an access leg runs: from a point onto the graph, or off it to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    On,
    Off,
}

impl Way {
    /// The leg between `point` and a node, in walking order.
    fn order(self, point: Point, node: Point) -> (Point, Point) {
        match self {
            Way::On => (point, node),
            Way::Off => (node, point),
        }
    }
}

/// How far from a stop's point on the road a lot may be and still be kept
/// for [`WalkGraph::road_stop`]: well past any stop's lot, so the rule sees
/// every lot it could choose.
pub const STOP_LOTS_WITHIN: f64 = 1000.0;

/// A stop on the road in (#445), as a walker finds it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadStop {
    /// The lot's id on `/terrain`
    pub lot: String,
    /// The lot's middle
    pub middle: Point,
    /// The point on the south arterial it was chosen from
    pub road: Point,
    /// Someone owns it
    pub taken: bool,
}

/// The lots within [`STOP_LOTS_WITHIN`] of a stop on the road in.
fn stop_lots(terrain: &TerrainResponse) -> Vec<TerrainPlot> {
    let points: Vec<Point> = stops::STOP_ZONES
        .iter()
        .filter_map(|&z| stops::stop_point(&Frame::TABLE, z))
        .collect();
    terrain
        .terrain_plots
        .iter()
        .filter(|l| points.iter().any(|&p| stops::distance_to_lot(p, &l.corners) <= STOP_LOTS_WITHIN))
        .cloned()
        .collect()
}

/// A way between a point and a node of the graph.
#[derive(Debug, Clone)]
struct Access {
    node: u32,
    cost: f64,
    /// The points between, in walking order, without the two ends.
    via: Vec<Point>,
}

impl WalkGraph {
    /// Builds the graph of `terrain`'s roads and trails.
    pub fn new(terrain: &TerrainResponse) -> Self {
        Self::joined_within(terrain, JOIN)
    }

    /// [`Self::new`], joining across open ground within `reach`.
    fn joined_within(terrain: &TerrainResponse, reach: f64) -> Self {
        let mut graph = WalkGraph {
            version: terrain.version.clone(),
            center: (terrain.center_x, terrain.center_y),
            rings: terrain.water_rings.clone(),
            ground: Ground::from_terrain(terrain),
            landmarks: terrain.landmarks.clone(),
            bazaar_streets: crate::bazaar::streets(&terrain.roads),
            places: Places::new(terrain),
            stop_lots: stop_lots(terrain),
            nodes: Vec::new(),
            edges: Vec::new(),
            buckets: HashMap::new(),
        };
        let mut ids = HashMap::new();
        for road in &terrain.roads {
            for w in road.points.windows(2) {
                let (a, b) = (w[0], w[1]);
                let pieces = (distance(a, b) / SPAN).ceil().max(1.0) as usize;
                let mut last = graph.node(&mut ids, a);
                for k in 1..=pieces {
                    let p = if k == pieces { b } else { lerp(a, b, k as f64 / pieces as f64) };
                    let next = graph.node(&mut ids, p);
                    graph.link(last, next);
                    last = next;
                }
            }
        }
        for i in 0..graph.nodes.len() as u32 {
            for j in graph.near(graph.nodes[i as usize], WELD) {
                if j > i && !graph.adjacent(i, j) {
                    graph.link(i, j);
                }
            }
        }
        graph.join(reach);
        graph
    }

    /// Joins the roads the open ground joins, if any don't meet. Each loose
    /// end, and each network still apart from the largest, gets a line to
    /// the nearest node of another network it can walk to within `reach`.
    fn join(&mut self, reach: f64) {
        for _ in 0..JOIN_ROUNDS {
            let networks = self.networks();
            let mut sizes = HashMap::<u32, usize>::new();
            for &c in &networks {
                *sizes.entry(c).or_default() += 1;
            }
            let largest = sizes.iter().max_by_key(|&(_, n)| *n).map_or(0, |(&c, _)| c);
            let mut links = Vec::new();
            let mut shortest: HashMap<u32, (f64, u32, u32)> = HashMap::new();
            for i in 0..self.nodes.len() as u32 {
                let network = networks[i as usize];
                let loose = self.edges[i as usize].len() <= 1;
                if !loose && (network == largest || i % JOIN_SAMPLE != 0) {
                    continue;
                }
                let p = self.nodes[i as usize];
                let other = self
                    .near(p, reach)
                    .into_iter()
                    .filter(|&j| networks[j as usize] != network)
                    .take(JOIN_TRIES)
                    .find(|&j| self.walkable(p, self.nodes[j as usize]));
                let Some(j) = other else { continue };
                let d = distance(p, self.nodes[j as usize]);
                if loose {
                    links.push((i, j));
                }
                if network != largest && shortest.get(&network).is_none_or(|s| d < s.0) {
                    shortest.insert(network, (d, i, j));
                }
            }
            links.extend(shortest.into_values().map(|(_, i, j)| (i, j)));
            let before = self.edges.iter().map(Vec::len).sum::<usize>();
            for (i, j) in links {
                if !self.adjacent(i, j) {
                    self.link(i, j);
                }
            }
            if self.edges.iter().map(Vec::len).sum::<usize>() == before {
                break;
            }
        }
    }

    /// Each node's network: the nodes joined to it, numbered by the least.
    fn networks(&self) -> Vec<u32> {
        let mut network = vec![u32::MAX; self.nodes.len()];
        for start in 0..self.nodes.len() as u32 {
            if network[start as usize] != u32::MAX {
                continue;
            }
            let mut stack = vec![start];
            network[start as usize] = start;
            while let Some(i) = stack.pop() {
                for &(j, _) in &self.edges[i as usize] {
                    if network[j as usize] == u32::MAX {
                        network[j as usize] = start;
                        stack.push(j);
                    }
                }
            }
        }
        network
    }

    fn adjacent(&self, i: u32, j: u32) -> bool {
        self.edges[i as usize].iter().any(|&(k, _)| k == j)
    }

    /// The `/terrain` version it was built from.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The first landmark of `kind`: the Exchange hall, say.
    pub fn landmark(&self, kind: LandmarkKind) -> Option<&Landmark> {
        self.landmarks.iter().find(|l| l.kind == kind)
    }

    /// The world's landmarks, from the same `/terrain`.
    pub fn landmarks(&self) -> &[Landmark] {
        &self.landmarks
    }

    /// The bazaar quarter's streets (`bazaar::streets`).
    pub fn bazaar_streets(&self) -> &[Vec<Point>] {
        &self.bazaar_streets
    }

    /// The places worth being (C2.1), from the same `/terrain`.
    pub fn places(&self) -> &Places {
        &self.places
    }

    /// The road in's stop in `zone` (#445), by the rule the client marks it
    /// by (`protocol::tutorial::stops`), so a citizen walking the road
    /// (#450) looks at the same lot a player's ring is round. `taken` says
    /// whether a lot, by its id, is owned.
    pub fn road_stop(&self, zone: Zone, taken: impl Fn(&str) -> bool) -> Option<RoadStop> {
        let road = stops::stop_point(&Frame::TABLE, zone)?;
        let stop = stops::stop(&self.stop_lots, &Frame::TABLE, zone, taken)?;
        let c = &stop.lot.corners;
        let middle = (c.iter().map(|p| p.0).sum::<f64>() / 4.0, c.iter().map(|p| p.1).sum::<f64>() / 4.0);
        Some(RoadStop {
            lot: stop.lot.id.clone(),
            middle,
            road,
            taken: stop.taken,
        })
    }

    /// How many nodes the graph has.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The ground and water it tests a line against: the land's params among
    /// them.
    pub fn ground(&self) -> &Ground {
        &self.ground
    }

    /// The graph's edges with an end within `radius` of `p`, each once, for
    /// the admin's walk layer (D5.3).
    pub fn edges_near(&self, p: Point, radius: f64) -> Vec<(Point, Point)> {
        let mut out = Vec::new();
        for i in self.near(p, radius) {
            for &(j, _) in &self.edges[i as usize] {
                let (a, b) = (self.nodes[i as usize], self.nodes[j as usize]);
                // Both ends near: only from the lower, so it comes once
                if i < j || distance(p, b) > radius {
                    out.push((a, b));
                }
            }
        }
        out
    }

    /// The waypoints of a walk from `from` to `to`, each a straight line the
    /// server lets through, ending at `to`. Just `[to]` when the straight
    /// line is walkable. `None` when no way is found ([`Self::plan`] says
    /// why).
    pub fn route(&self, from: Point, to: Point) -> Option<Vec<Point>> {
        self.plan(from, to).ok()
    }

    /// [`Self::route`], or why there's no way: `from` is somewhere the graph
    /// can't be reached from, or `to` somewhere it can't reach (in a lake,
    /// up a crag, in a pocket of ground walled by crests).
    pub fn plan(&self, from: Point, to: Point) -> Result<Vec<Point>, NoWay> {
        self.planned(from, to, &|| false).unwrap_or(Err(NoWay::Unreachable))
    }

    /// [`Self::plan`], given up (`None`) as soon as `gone` says nobody wants
    /// it: a click's plan that a newer click replaced (#348). A walk over the
    /// grid in the ranges can take most of a second.
    pub fn plan_unless(&self, from: Point, to: Point, gone: &dyn Fn() -> bool) -> Option<Result<Vec<Point>, NoWay>> {
        self.planned(from, to, gone).ok()
    }

    fn planned(&self, from: Point, to: Point, gone: &dyn Fn() -> bool) -> Result<Result<Vec<Point>, NoWay>, Gone> {
        if self.walkable(from, to) {
            return Ok(Ok(vec![to]));
        }
        let ons = self.access(from, Way::On, gone)?;
        if ons.is_empty() {
            return Ok(Err(NoWay::Stranded));
        }
        let offs = self.access(to, Way::Off, gone)?;
        if offs.is_empty() {
            return Ok(Err(NoWay::Unreachable));
        }
        let Some(points) = self.search(from, to, &ons, &offs) else {
            return Ok(Err(NoWay::Unreachable));
        };
        if gone() {
            return Err(Gone);
        }
        Ok(Ok(self.pull(&points)))
    }

    /// Whether the server lets a straight walk from `a` to `b` through: it
    /// stays in the world, crosses the moat and canals only on their bridges,
    /// keeps out of rivers and lakes away from their fords and roads, and
    /// climbs nothing steeper than [`CLIFF_SLOPE`](ground::CLIFF_SLOPE). A walk that starts in the
    /// water, or on a crag, may leave it, as on the server.
    pub fn walkable(&self, a: Point, b: Point) -> bool {
        rules::in_world(b.0, b.1) && self.rings_let_through(a, b) && self.ground_lets_through(a, b)
    }

    /// About how many seconds the straight walk from `a` to `b` takes,
    /// slowed by the slope and the streams.
    pub fn walk_secs(&self, a: Point, b: Point) -> f64 {
        self.cost(a, b) / PLAYER_SPEED
    }

    /// Every crossing of a ring's edge must be on one of its bridges, as the
    /// server tests each tick's step ([`rules::ring_crossing`], in
    /// [`Ground::step`]).
    fn rings_let_through(&self, a: Point, b: Point) -> bool {
        self.rings.iter().all(|ring| {
            [ring.inner_radius(), ring.outer_radius()].into_iter().all(|radius| {
                circle_crossings(self.center, a, b, radius).into_iter().flatten().all(|(x, y)| {
                    let angle = rules::angle_from_center(self.center, x, y);
                    ring.bridges.iter().any(|bridge| bridge.contains_angle(angle, ring.radius))
                })
            })
        })
    }

    /// The wild water and the slope along the line.
    fn ground_lets_through(&self, a: Point, b: Point) -> bool {
        distance(a, b) == 0.0 || (self.water_lets_through(a, b) && self.slope_lets_through(a, b))
    }

    /// The wild water, sampled along the line. Where the line goes in or out
    /// of the water, the gap is searched at [`FINE`] for the river between a
    /// ford's edge and the bank.
    fn water_lets_through(&self, a: Point, b: Point) -> bool {
        let n = (distance(a, b) / WATER_SAMPLE).ceil() as usize;
        let mut leaving = matches!(self.ground.footing(a.0, a.1), Footing::Blocked(_));
        let mut wading = matches!(self.ground.footing(a.0, a.1), Footing::Wading(_));
        let mut last = a;
        for i in 1..=n {
            let p = lerp(a, b, i as f64 / n as f64);
            match self.ground.footing(p.0, p.1) {
                Footing::Blocked(_) if !leaving => return false,
                Footing::Blocked(_) => {}
                footing => {
                    let wet = matches!(footing, Footing::Wading(_));
                    if wet != wading && !leaving && self.blocked_between(last, p) {
                        return false;
                    }
                    wading = wet;
                    leaving = false;
                }
            }
            last = p;
        }
        true
    }

    /// The slope along the line, by the rule the backend plants wild nodes
    /// by ([`ground::slope_lets_through`], #347), which looks again where it
    /// kinks at a valley's floor (#520, #567).
    fn slope_lets_through(&self, a: Point, b: Point) -> bool {
        let land = self.ground.land();
        ground::slope_lets_through(a, b, |x, y| land::slope_at(x, y, land))
    }

    /// Whether the water stops a walk between two samples of a line, `a`
    /// and `b`, searched every [`FINE`]: a ford can end a quarter short of
    /// its river's bank (#280).
    fn blocked_between(&self, a: Point, b: Point) -> bool {
        let n = (distance(a, b) / FINE).ceil() as usize;
        (1..n).any(|i| {
            let p = lerp(a, b, i as f64 / n as f64);
            matches!(self.ground.footing(p.0, p.1), Footing::Blocked(_))
        })
    }

    /// The straight walk's length on the flat that takes as long: its
    /// length, stretched by the slope and the streams.
    fn cost(&self, a: Point, b: Point) -> f64 {
        let length = distance(a, b);
        if length == 0.0 {
            return 0.0;
        }
        let heading = ((b.0 - a.0) / length, (b.1 - a.1) / length);
        let n = (length / COST_SAMPLE).ceil() as usize;
        let piece = length / n as f64;
        (0..n)
            .map(|i| {
                let p = lerp(a, b, (i as f64 + 0.5) / n as f64);
                piece / self.ground.speed_at(p.0, p.1, heading).max(SLOWEST)
            })
            .sum()
    }

    /// The node at `p`, made if it's new. Roads that share a vertex share its node.
    fn node(&mut self, ids: &mut HashMap<(i64, i64), u32>, p: Point) -> u32 {
        let key = ((p.0 * 100.0).round() as i64, (p.1 * 100.0).round() as i64);
        *ids.entry(key).or_insert_with(|| {
            let id = self.nodes.len() as u32;
            self.nodes.push(p);
            self.edges.push(Vec::new());
            self.buckets.entry(bucket_of(p)).or_default().push(id);
            id
        })
    }

    /// Joins `i` and `j` both ways, if the line between is walkable.
    fn link(&mut self, i: u32, j: u32) {
        let (a, b) = (self.nodes[i as usize], self.nodes[j as usize]);
        if i == j || !self.walkable(a, b) {
            return;
        }
        let (there, back) = (self.cost(a, b), self.cost(b, a));
        self.edges[i as usize].push((j, there));
        self.edges[j as usize].push((i, back));
    }

    /// The nodes within `radius` of `p`, nearest first.
    fn near(&self, p: Point, radius: f64) -> Vec<u32> {
        let reach = (radius / BUCKET).ceil() as i64;
        let (bx, by) = bucket_of(p);
        let mut found: Vec<(f64, u32)> = Vec::new();
        for j in by - reach..=by + reach {
            for i in bx - reach..=bx + reach {
                let Some(ids) = self.buckets.get(&(i, j)) else { continue };
                found.extend(ids.iter().map(|&k| (distance(p, self.nodes[k as usize]), k)).filter(|&(d, _)| d <= radius));
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.into_iter().map(|(_, k)| k).collect()
    }

    /// The ways between `p` and the graph, running `way`: straight lines to
    /// the nodes in sight, or one over the grid when none is.
    fn access(&self, p: Point, way: Way, gone: &dyn Fn() -> bool) -> Result<Vec<Access>, Gone> {
        for radius in ACCESS_RADII {
            let found: Vec<Access> = self
                .candidates(p, radius)
                .into_iter()
                .filter_map(|node| {
                    let (a, b) = way.order(p, self.nodes[node as usize]);
                    self.walkable(a, b).then(|| Access { node, cost: self.cost(a, b), via: Vec::new() })
                })
                .collect();
            if !found.is_empty() {
                return Ok(found);
            }
        }
        Ok(self.access_by_grid(p, way, gone)?.into_iter().collect())
    }

    /// The nodes within `radius` worth trying from `p`: the nearest few, and
    /// the nearest in each direction, so a street across a river from `p`
    /// doesn't hide the one behind it.
    fn candidates(&self, p: Point, radius: f64) -> Vec<u32> {
        let near: Vec<u32> = self.near(p, radius).into_iter().filter(|&k| !self.edges[k as usize].is_empty()).collect();
        let mut picked: Vec<u32> = near.iter().take(ACCESS_NEAREST).copied().collect();
        let mut sectors = [false; ACCESS_SECTORS];
        for &k in &near {
            let q = self.nodes[k as usize];
            let turn = (q.1 - p.1).atan2(q.0 - p.0) / std::f64::consts::TAU + 0.5;
            let sector = ((turn * ACCESS_SECTORS as f64) as usize).min(ACCESS_SECTORS - 1);
            if !sectors[sector] {
                sectors[sector] = true;
                if !picked.contains(&k) {
                    picked.push(k);
                }
            }
        }
        picked
    }

    /// A way between `p` and a node it can reach over a grid of
    /// [`GRID_CELL`] within [`GRID_REACH`]: round the bend of a lake shore,
    /// say, or down off a scree and along a valley to its trail. The search
    /// is A*, heading for the nearest nodes, and ends at the first cell with
    /// a node in sight within [`GRID_SIGHT`]. Every [`GRID_ASK`] cells it
    /// asks `gone` whether the walk is still wanted.
    fn access_by_grid(&self, p: Point, way: Way, gone: &dyn Fn() -> bool) -> Result<Option<Access>, Gone> {
        let targets: Vec<Point> = self
            .near(p, GRID_REACH)
            .into_iter()
            .filter(|&k| !self.edges[k as usize].is_empty())
            .take(GRID_TARGETS)
            .map(|k| self.nodes[k as usize])
            .collect();
        if targets.is_empty() {
            return Ok(None);
        }
        let reach = (GRID_REACH / GRID_CELL) as i64;
        let at = |(i, j): (i64, i64)| (p.0 + i as f64 * GRID_CELL, p.1 + j as f64 * GRID_CELL);
        let h = |q: Point| targets.iter().map(|&t| distance(q, t)).fold(f64::INFINITY, f64::min);
        // The cells reached, and the cells by number, for the open set
        let mut cells: HashMap<Cell, Reached> = HashMap::new();
        let mut keys: Vec<Cell> = vec![(0, 0)];
        let mut open = BinaryHeap::new();
        cells.insert((0, 0), Reached { id: 0, cost: 0.0, from: (0, 0) });
        open.push(Open { f: h(p), node: 0 });
        let mut looked = 0;
        while let Some(Open { f, node }) = open.pop() {
            let cell = keys[node as usize];
            let here = at(cell);
            let g = cells[&cell].cost;
            if f > g + h(here) + 1e-9 {
                continue;
            }
            looked += 1;
            if looked > GRID_CELLS {
                return Ok(None);
            }
            if looked % GRID_ASK == 0 && gone() {
                return Err(Gone);
            }
            for k in self.near(here, GRID_SIGHT) {
                if self.edges[k as usize].is_empty() {
                    continue;
                }
                let (a, b) = way.order(here, self.nodes[k as usize]);
                if self.walkable(a, b) {
                    let mut via = Vec::new();
                    let mut c = cell;
                    while c != (0, 0) {
                        via.push(at(c));
                        c = cells[&c].from;
                    }
                    if way == Way::On {
                        via.reverse();
                    }
                    return Ok(Some(Access { node: k, cost: g + self.cost(a, b), via }));
                }
            }
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let next = (cell.0 + di, cell.1 + dj);
                if next.0.abs() > reach || next.1.abs() > reach {
                    continue;
                }
                let there = at(next);
                let (a, b) = way.order(here, there);
                let through = g + self.cost(a, b);
                let known = cells.get(&next).copied();
                if known.is_some_and(|r| r.cost <= through) || !self.walkable(a, b) {
                    continue;
                }
                let id = known.map_or_else(
                    || {
                        keys.push(next);
                        keys.len() as u32 - 1
                    },
                    |r| r.id,
                );
                cells.insert(next, Reached { id, cost: through, from: cell });
                open.push(Open { f: through + h(there), node: id });
            }
        }
        Ok(None)
    }

    /// A* from the `ons` to the `offs`, as every point of the walk: `from`,
    /// the way onto the graph, the nodes, the way off and `to`.
    fn search(&self, from: Point, to: Point, ons: &[Access], offs: &[Access]) -> Option<Vec<Point>> {
        const START: u32 = u32::MAX;
        let n = self.nodes.len();
        let goal = n as u32;
        let mut cost = vec![f64::INFINITY; n + 1];
        let mut came = vec![START; n + 1];
        // Which way on reached each node that's reached straight from it,
        // and which way off the goal was reached by.
        let mut on_of: HashMap<u32, usize> = HashMap::new();
        let mut off_of: HashMap<u32, usize> = HashMap::new();
        for (k, off) in offs.iter().enumerate() {
            match off_of.get(&off.node) {
                Some(&o) if offs[o].cost <= off.cost => {}
                _ => {
                    off_of.insert(off.node, k);
                }
            }
        }
        let h = |node: u32| if node == goal { 0.0 } else { distance(self.nodes[node as usize], to) };
        let mut open = BinaryHeap::new();
        for (k, on) in ons.iter().enumerate() {
            if on.cost < cost[on.node as usize] {
                cost[on.node as usize] = on.cost;
                on_of.insert(on.node, k);
                open.push(Open { f: on.cost + h(on.node), node: on.node });
            }
        }
        while let Some(Open { f, node }) = open.pop() {
            if node == goal {
                break;
            }
            let g = cost[node as usize];
            if f > g + h(node) {
                continue;
            }
            let off = off_of.get(&node).map(|&k| (goal, offs[k].cost));
            for &(next, step) in self.edges[node as usize].iter().chain(off.iter()) {
                let through = g + step;
                if through < cost[next as usize] {
                    cost[next as usize] = through;
                    came[next as usize] = node;
                    open.push(Open { f: through + h(next), node: next });
                }
            }
        }
        if came[goal as usize] == START {
            return None;
        }
        let mut nodes = Vec::new();
        let mut node = came[goal as usize];
        while node != START {
            nodes.push(node);
            node = came[node as usize];
        }
        nodes.reverse();
        let (first, last) = (nodes[0], nodes[nodes.len() - 1]);
        let mut points = vec![from];
        points.extend(&ons[on_of[&first]].via);
        points.extend(nodes.iter().map(|&k| self.nodes[k as usize]));
        points.extend(&offs[off_of[&last]].via);
        points.push(to);
        Some(points)
    }

    /// The walk's points pulled tight into legs, without `points[0]` (where
    /// the walker stands). A leg runs on past a point while the straight line
    /// is within [`DEVIATION`] of every point it skips, no slower than the
    /// way round, and walkable.
    ///
    /// Each leg's end is found by galloping out from its start, a point, two,
    /// four, and halving back from the first that isn't tight. Trying every
    /// point in turn tested a line as long as the leg at each, and down a
    /// straight arterial that was most of a click's plan (#348).
    fn pull(&self, points: &[Point]) -> Vec<Point> {
        let mut along = vec![0.0];
        for w in points.windows(2) {
            along.push(along[along.len() - 1] + self.cost(w[0], w[1]));
        }
        let tight = |i: usize, k: usize| {
            let (a, b) = (points[i], points[k]);
            points[i + 1..k].iter().all(|&p| off_line(a, b, p) <= DEVIATION)
                && self.cost(a, b) <= along[k] - along[i] + 1e-6
                && self.walkable(a, b)
        };
        let last = points.len() - 1;
        let mut legs = Vec::new();
        let mut anchor = 0;
        while anchor < last {
            // The next point is a leg already, and past the last isn't one
            let (mut good, mut bad) = (anchor + 1, last + 1);
            let mut step = 1;
            let mut galloping = true;
            while good + 1 < bad {
                let probe = if galloping { (good + step).min(bad - 1) } else { good + (bad - good) / 2 };
                if tight(anchor, probe) {
                    good = probe;
                    step *= 2;
                } else {
                    bad = probe;
                    galloping = false;
                }
            }
            legs.push(points[good]);
            anchor = good;
        }
        legs.dedup();
        legs
    }
}

/// The walk graphs built in this process, newest last, so a swarm of bots
/// builds each terrain's once.
static SHARED: Mutex<Vec<Arc<WalkGraph>>> = Mutex::new(Vec::new());
/// How many terrains' graphs [`SHARED`] keeps.
const KEEP: usize = 2;

/// The graph of `terrain`, built now or by an earlier call for the same
/// version. Bots building at once take turns, and all but the first reuse it.
pub fn shared(terrain: &TerrainResponse) -> Arc<WalkGraph> {
    let mut graphs = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(graph) = graphs.iter().find(|g| g.version.is_some() && g.version == terrain.version) {
        return graph.clone();
    }
    let graph = Arc::new(WalkGraph::new(terrain));
    tracing::info!(
        "walk graph: {} nodes for terrain {}",
        graph.len(),
        terrain.version.as_deref().unwrap_or("without a version")
    );
    graphs.push(graph.clone());
    if graphs.len() > KEEP {
        graphs.remove(0);
    }
    graph
}

/// The newest graph [`shared`] built, to revalidate its terrain by version
/// rather than fetch it whole.
pub fn latest() -> Option<Arc<WalkGraph>> {
    let graphs = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    graphs.last().cloned()
}

/// A cell of [`WalkGraph::access_by_grid`]'s grid, counted from where it starts.
type Cell = (i64, i64);

/// A cell the grid's search has reached: its number, its cost from the
/// start, and the cell it was reached from.
#[derive(Debug, Clone, Copy)]
struct Reached {
    id: u32,
    cost: f64,
    from: Cell,
}

/// An entry in A*'s open set, cheapest first.
#[derive(Debug, Clone, Copy)]
struct Open {
    f: f64,
    node: u32,
}

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Open {}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f).then_with(|| other.node.cmp(&self.node))
    }
}

fn bucket_of(p: Point) -> (i64, i64) {
    ((p.0 / BUCKET).floor() as i64, (p.1 / BUCKET).floor() as i64)
}

fn distance(a: Point, b: Point) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// How far `p` is from the segment `ab`.
fn off_line(a: Point, b: Point, p: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    if length2 == 0.0 {
        return distance(a, p);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0);
    distance(p, (a.0 + dx * t, a.1 + dy * t))
}

/// Where the segment `ab` crosses the circle of `radius` round `center`:
/// none, one or two points.
fn circle_crossings(center: Point, a: Point, b: Point, radius: f64) -> [Option<Point>; 2] {
    let (x1, y1) = (a.0 - center.0, a.1 - center.1);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let qa = dx * dx + dy * dy;
    if qa == 0.0 {
        return [None, None];
    }
    let qb = 2.0 * (x1 * dx + y1 * dy);
    let qc = x1 * x1 + y1 * y1 - radius * radius;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return [None, None];
    }
    let root = disc.sqrt();
    [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)]
        .map(|t| (0.0..=1.0).contains(&t).then(|| lerp(a, b, t)))
}

#[cfg(test)]
mod tests;
