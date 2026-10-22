//! The water of the wilds and the rural plain, from the height of the land
//! (W3: #196, #220, #222).
//!
//! A pure function of [`LandParams`] and the roads, so the default world's
//! water is pinned with the rest of it (`default_terrain_version_is_pinned`).
//!
//! 1. **Sample** `land::height_at` on a grid of [`CELL`] from the water's
//!    inner rim (`rules::WATER_INNER_RADIUS`), just out from the suburb, to
//!    the world's edge. The rings are the land's frame (`LandParams::frame`),
//!    so a world on other rings has its water follow them (PV2, #454).
//!    Water is steered by a
//!    routing field: the height, plus, on the flat rural plain, a gentle fall
//!    towards the city with some noise, and a berm along every road. The
//!    ground itself stays as it is.
//! 2. **Priority-flood** in from the grid's two rims, the world's edge and the
//!    inner rim. Each cell drains down its steepest fall on the filled field,
//!    or along the flood across a flat.
//! 3. **Lakes** are the pits in the hills deep and wide enough, each with its
//!    level and a shore traced by marching squares.
//! 4. **Flow** accumulates down the drainage, and on the plain it soaks away
//!    ([`soak`]): a stream gives out soon, a big river runs on further. Streams
//!    are where the flow passes [`STREAM_CELLS`] and rivers where it passes
//!    [`RIVER_CELLS`], so a river soaking away runs on as a stream. Each course
//!    follows its largest inflow up, and every other inflow is a course of its
//!    own that joins it. A channel running beside a bigger one joins it there
//!    ([`gather`]). Courses are smoothed and meander on flat ground, then their
//!    surfaces are made to fall downstream: the carve (`land::carve`) cuts them
//!    through what's left.
//! 5. **Where the plain soaks a course up** it ends in a pond, more often the
//!    more it carried, or it peters out.
//! 6. **Fords** are the shallowest stretches of each river, spaced apart.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use capitol_protocol::fmath::StableMath;
use capitol_protocol::land::{height_at, LandParams};
use capitol_protocol::terrain::{Ford, Lake, Mouth, RoadSegment, WaterCourse, WaterPoint};

/// The grid's cell, in world units.
pub const CELL: f64 = 30.0;
/// How far past the plain's edge its routing field fades out.
const PLAIN_BLEND: f64 = 200.0;
/// The plain's fall towards the city, in the routing field, from the plain's
/// edge (`LandParams::plain_edge`, where the land starts to rise).
const PLAIN_FALL: f64 = 0.01;
/// How high the routing field stands at the plain's edge. It's the fall's
/// 750 in from there, as it was across the 12000 world's plain, so the
/// widened plain (#308) tilts as steeply and holds the foothills' water back
/// no higher. Measured from the inner rim, the 1600-deep plain stood twice
/// as high at its edge, and dammed most of the water off it.
const PLAIN_EDGE_RISE: f64 = 7.5;
/// How high the routing field's noise on the plain stands.
const PLAIN_NOISE: f64 = 4.0;
/// How high a road stands in the routing field, so water skirts it.
const BERM: f64 = 3.0;
/// How fast the plain soaks up a course's flow: see [`soak`].
const SOAK: f64 = 12.0;
/// Upstream cells a stream drains, at least.
pub const STREAM_CELLS: f64 = 120.0;
/// Upstream cells a river drains, at least.
pub const RIVER_CELLS: f64 = 600.0;
/// A pit becomes a lake when it covers this many cells...
const LAKE_MIN_CELLS: usize = 20;
/// ...and is this deep somewhere.
const LAKE_MIN_DEPTH: f64 = 4.0;
/// A course out of a lake rises at least this far under its level, so its
/// source is well inside the shore: see `Grid::path_into_the_lake`.
const SOURCE_DEPTH: f64 = 0.5;
/// A ford is no deeper than this.
const FORD_DEPTH: f64 = 1.5;
/// Fords on one river are at least this far apart along it.
const FORD_SPACING: f64 = 600.0;

/// The water of the wilds, as `/terrain` carries it.
#[derive(Debug, Clone, Default)]
pub struct Hydrology {
    pub rivers: Vec<WaterCourse>,
    pub streams: Vec<WaterCourse>,
    pub lakes: Vec<Lake>,
    pub fords: Vec<Ford>,
}

/// The rings the water is laid in, from the land's frame.
#[derive(Debug, Clone, Copy)]
struct Rings {
    center: (f64, f64),
    /// The grid's inner rim (`Frame::water_inner_radius`). No water comes in
    /// past it, so the suburb stays dry.
    inner: f64,
    /// Where the plain ends and the land starts to rise (`land::height_at`
    /// is 0 inside it).
    plain_edge: f64,
    /// The world's edge.
    outer: f64,
    /// The side of the world's square.
    size: f64,
}

impl Rings {
    fn of(land: &LandParams) -> Self {
        let frame = &land.frame;
        Rings {
            center: frame.center,
            inner: frame.water_inner_radius(),
            plain_edge: land.plain_edge(),
            outer: frame.world_edge(),
            size: frame.world_size(),
        }
    }

    fn radius(&self, x: f64, y: f64) -> f64 {
        let (dx, dy) = (x - self.center.0, y - self.center.1);
        (dx * dx + dy * dy).sqrt()
    }

    /// How much of the plain's routing field stands at radius `r`: all of it
    /// on the plain, none once the land has risen.
    fn plain_weight(&self, r: f64) -> f64 {
        1.0 - smoothstep((r - self.plain_edge) / PLAIN_BLEND)
    }
}

/// The water for this land, round these roads (those past the suburb steer
/// it; the rest are ignored). It's laid in the land's frame.
pub fn generate(land: &LandParams, roads: &[RoadSegment]) -> Hydrology {
    let rings = Rings::of(land);
    let roads = Roads::near(roads, &rings);
    let grid = Grid::sample(land, &roads, rings);
    let mut flood = grid.flood();
    let pits = grid.pits(&flood);
    gather(&grid, &mut flood, &pits);
    let network = Network::trace(&grid, &flood, &pits, land.seed);
    network.finish(&grid, land, &pits, &roads)
}

/// The distance from `p` to segment `ab`.
fn point_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (ex, ey) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (ex * ex + ey * ey).sqrt()
}

/// The roads' segments past the suburb, as (a, b, half width), bucketed.
struct Roads {
    segments: Vec<((f64, f64), (f64, f64), f64)>,
    buckets: HashMap<(i64, i64), Vec<usize>>,
}

/// The side of a [`Roads`] bucket.
const ROAD_BUCKET: f64 = 120.0;

impl Roads {
    fn near(roads: &[RoadSegment], rings: &Rings) -> Self {
        let mut segments = Vec::new();
        let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        let key = |v: f64| (v / ROAD_BUCKET).floor() as i64;
        for road in roads {
            for w in road.points.windows(2) {
                let (a, b) = (w[0], w[1]);
                if rings.radius(a.0, a.1).max(rings.radius(b.0, b.1)) < rings.inner - 100.0 {
                    continue;
                }
                let half = road.width / 2.0;
                let k = segments.len();
                segments.push((a, b, half));
                for j in key(a.1.min(b.1) - half)..=key(a.1.max(b.1) + half) {
                    for i in key(a.0.min(b.0) - half)..=key(a.0.max(b.0) + half) {
                        buckets.entry((i, j)).or_default().push(k);
                    }
                }
            }
        }
        Roads { segments, buckets }
    }

    /// How far `p` is from the kerb of the nearest road, up to `reach` (a
    /// point on a road is 0).
    fn clearance(&self, p: (f64, f64), reach: f64) -> f64 {
        let key = |v: f64| (v / ROAD_BUCKET).floor() as i64;
        let mut best = reach;
        for j in key(p.1 - reach)..=key(p.1 + reach) {
            for i in key(p.0 - reach)..=key(p.0 + reach) {
                for &k in self.buckets.get(&(i, j)).map(Vec::as_slice).unwrap_or(&[]) {
                    let (a, b, half) = self.segments[k];
                    best = best.min((point_segment(p, a, b) - half).max(0.0));
                }
            }
        }
        best
    }
}

// ---------------------------------------------------------------------------
// The grid

const OUT: u32 = u32::MAX;

struct Grid {
    n: usize,
    /// The height at each cell's centre; NaN outside the grid's ring.
    h: Vec<f64>,
    /// What steers the water: the height, and on the plain its fall, noise
    /// and the roads' berms.
    route: Vec<f64>,
    rings: Rings,
}

/// Smooth value noise in [0, 1] at about `wavelength`.
fn value_noise(seed: u64, x: f64, y: f64, wavelength: f64) -> f64 {
    let (u, v) = (x / wavelength, y / wavelength);
    let (i, j) = (u.floor(), v.floor());
    let (fu, fv) = (u - i, v - j);
    let corner = |di: i64, dj: i64| {
        let h = splitmix64(seed ^ splitmix64((i as i64 + di) as u64 ^ splitmix64((j as i64 + dj) as u64)));
        (h >> 11) as f64 / (1u64 << 53) as f64
    };
    let (su, sv) = (smoothstep(fu), smoothstep(fv));
    let bottom = corner(0, 0) + su * (corner(1, 0) - corner(0, 0));
    let top = corner(0, 1) + su * (corner(1, 1) - corner(0, 1));
    bottom + sv * (top - bottom)
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// Each of hydrology's noise fields draws from its own stream of the land's seed.
const PLAIN_FIELD: u64 = 0x504C_4149_4E00_0001;
const SOAK_FIELD: u64 = 0x534F_414B_0000_0002;
const POND_FIELD: u64 = 0x504F_4E44_0000_0003;

/// The eight neighbours, as (di, dj).
const NEIGHBOURS: [(i64, i64); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rim {
    /// The world's edge.
    Edge,
    /// The inner rim, just out from the suburb.
    Inner,
}

struct Flood {
    /// The level each cell fills to: its height, or the level its pit spills at.
    filled: Vec<f64>,
    /// The cell each drains to, or [`OUT`] for a rim cell.
    down: Vec<u32>,
    /// The rim a rim cell drains off.
    rim: HashMap<u32, Rim>,
    /// The cells in the order the flood reached them: every cell after the
    /// one it drains to.
    order: Vec<u32>,
}

impl Grid {
    fn sample(land: &LandParams, roads: &Roads, rings: Rings) -> Self {
        let n = (rings.size / CELL).round() as usize;
        let mut h = vec![f64::NAN; n * n];
        let mut route = vec![f64::NAN; n * n];
        for j in 0..n {
            for i in 0..n {
                let (x, y) = centre(i, j);
                let r = rings.radius(x, y);
                if !(rings.inner..=rings.outer).contains(&r) {
                    continue;
                }
                let c = j * n + i;
                h[c] = height_at(x, y, land);
                route[c] = h[c];
                let w = rings.plain_weight(r);
                if w > 0.0 {
                    let noise = value_noise(land.seed ^ PLAIN_FIELD, x, y, 450.0) - 0.5;
                    let berm = if roads.clearance((x, y), CELL) < CELL { BERM } else { 0.0 };
                    route[c] += w * (PLAIN_EDGE_RISE + PLAIN_FALL * (r - rings.plain_edge) + PLAIN_NOISE * noise + berm);
                }
            }
        }
        Grid { n, h, route, rings }
    }

    fn inside(&self, c: usize) -> bool {
        !self.h[c].is_nan()
    }

    fn neighbours(&self, c: usize) -> impl Iterator<Item = usize> + '_ {
        let (i, j) = ((c % self.n) as i64, (c / self.n) as i64);
        let n = self.n as i64;
        NEIGHBOURS.iter().filter_map(move |&(di, dj)| {
            let (a, b) = (i + di, j + dj);
            (a >= 0 && b >= 0 && a < n && b < n).then(|| (b * n + a) as usize)
        })
    }

    fn point(&self, c: usize) -> (f64, f64) {
        centre(c % self.n, c / self.n)
    }

    /// The cell a point is in.
    fn cell_at(&self, (x, y): (f64, f64)) -> usize {
        let at = |v: f64| ((v / CELL).floor().max(0.0) as usize).min(self.n - 1);
        at(y) * self.n + at(x)
    }

    /// From a lake's cell `from` to the nearest of its cells at least
    /// [`SOURCE_DEPTH`] under its level, by its cells: where a course out of
    /// the lake rises. A cell only just under the level can be one the shore
    /// leaves out, where it meets the rest of the lake corner to corner and
    /// its own ring is too small to keep (`lake_from_rings`). Outward from
    /// `from`, so the path runs mouth to source, as [`Network::trace`] builds.
    fn path_into_the_lake(&self, from: usize, pits: &Pits) -> Vec<usize> {
        let p = pits.lake_of[from];
        let level = pits.pits[p as usize].level;
        let mut came = HashMap::from([(from, from)]);
        let mut queue = std::collections::VecDeque::from([from]);
        while let Some(c) = queue.pop_front() {
            if level - self.h[c] >= SOURCE_DEPTH {
                let mut path = vec![c];
                while *path.last().unwrap() != from {
                    path.push(came[path.last().unwrap()]);
                }
                path.reverse();
                return path;
            }
            for m in self.neighbours(c) {
                if pits.lake_of[m] == p && !came.contains_key(&m) {
                    came.insert(m, c);
                    queue.push_back(m);
                }
            }
        }
        vec![from]
    }

    /// Floods in from the rims, lowest first; equal levels go first in, first
    /// out, so water crosses a flat towards where it spills.
    fn flood(&self) -> Flood {
        let cells = self.h.len();
        let mut filled = vec![f64::NAN; cells];
        let mut down = vec![OUT; cells];
        let mut rim = HashMap::new();
        let mut queued = vec![false; cells];
        let mut heap = BinaryHeap::new();
        let mut seq = 0u64;
        for c in 0..cells {
            // A rim cell has a neighbour outside the ring: past the edge, or
            // inside the inner rim.
            if !self.inside(c) || self.neighbours(c).count() == 8 && self.neighbours(c).all(|m| self.inside(m)) {
                continue;
            }
            let (x, y) = self.point(c);
            let edge = self.rings.radius(x, y) > 0.5 * (self.rings.inner + self.rings.outer);
            rim.insert(c as u32, if edge { Rim::Edge } else { Rim::Inner });
            filled[c] = self.route[c];
            queued[c] = true;
            heap.push(Entry { level: self.route[c], seq, cell: c as u32 });
            seq += 1;
        }
        let mut order = Vec::with_capacity(cells);
        while let Some(Entry { level, cell, .. }) = heap.pop() {
            order.push(cell);
            for m in self.neighbours(cell as usize) {
                if queued[m] || !self.inside(m) {
                    continue;
                }
                queued[m] = true;
                filled[m] = self.route[m].max(level);
                down[m] = cell;
                heap.push(Entry { level: filled[m], seq, cell: m as u32 });
                seq += 1;
            }
        }
        // Where the filled ground falls away, drain down its steepest fall
        // instead: the flood's own paths run straight across gentle ground.
        // Cells pop in order of level, so `order` stays upstream-last.
        for c in 0..cells {
            if !self.inside(c) || rim.contains_key(&(c as u32)) {
                continue;
            }
            let (ci, cj) = (c % self.n, c / self.n);
            let mut best = (0.0, OUT);
            for m in self.neighbours(c) {
                if !self.inside(m) || filled[m] >= filled[c] {
                    continue;
                }
                let diagonal = m % self.n != ci && m / self.n != cj;
                let fall = (filled[c] - filled[m]) / if diagonal { std::f64::consts::SQRT_2 } else { 1.0 };
                if fall > best.0 {
                    best = (fall, m as u32);
                }
            }
            if best.1 != OUT {
                down[c] = best.1;
            }
        }
        Flood { filled, down, rim, order }
    }

    /// The pits that hold lakes. A pit is a connected run of cells filled
    /// above their height to one level. Only the hills' pits are lakes: on the
    /// plain the routing field isn't the ground, so its pits hold no water.
    fn pits(&self, flood: &Flood) -> Pits {
        let cells = self.h.len();
        let mut lake_of = vec![OUT; cells];
        let mut pits = Vec::new();
        let mut seen = vec![false; cells];
        for start in 0..cells {
            if seen[start] || !self.inside(start) || flood.filled[start] - self.route[start] <= 1e-9 {
                continue;
            }
            let level = flood.filled[start];
            let mut group = vec![start];
            seen[start] = true;
            let mut k = 0;
            while k < group.len() {
                let c = group[k];
                k += 1;
                for m in self.neighbours(c) {
                    if !seen[m] && self.inside(m) && flood.filled[m] == level && level - self.route[m] > 1e-9 {
                        seen[m] = true;
                        group.push(m);
                    }
                }
            }
            let depth = group.iter().map(|&c| level - self.h[c]).fold(0.0, f64::max);
            let in_hills = group.iter().all(|&c| {
                let (x, y) = self.point(c);
                self.rings.plain_weight(self.rings.radius(x, y)) == 0.0
            });
            if in_hills && group.len() >= LAKE_MIN_CELLS && depth >= LAKE_MIN_DEPTH {
                let id = pits.len() as u32;
                for &c in &group {
                    lake_of[c] = id;
                }
                pits.push(Pit { level, cells: group });
            }
        }
        Pits { lake_of, pits }
    }
}

struct Pit {
    level: f64,
    cells: Vec<usize>,
}

struct Pits {
    /// The pit each cell is under the water of, or [`OUT`].
    lake_of: Vec<u32>,
    pits: Vec<Pit>,
}

fn centre(i: usize, j: usize) -> (f64, f64) {
    ((i as f64 + 0.5) * CELL, (j as f64 + 0.5) * CELL)
}

/// A min-heap entry: lowest level first, then first pushed.
struct Entry {
    level: f64,
    seq: u64,
    cell: u32,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.level.total_cmp(&self.level).then(other.seq.cmp(&self.seq))
    }
}

// ---------------------------------------------------------------------------
// The courses

/// Where a traced course ends, before ids are given out.
#[derive(Clone, Copy, PartialEq, Debug)]
enum End {
    Edge,
    /// It reached the inner rim.
    Inner,
    /// The plain soaked it up.
    Soaked,
    Pit(u32),
    /// Joins course `k` at grid cell `c`.
    Joins { course: usize, cell: usize },
}

struct Traced {
    /// Grid cells, source to mouth. A course into a pit or another course ends
    /// on a cell of it; one from a pit starts on a cell of it.
    cells: Vec<usize>,
    from_pit: Option<u32>,
    end: End,
}

struct Network {
    /// The flow through each cell, after the plain's soaking.
    acc: Vec<f64>,
    courses: Vec<Traced>,
}

/// How far in from the plain's edge the soaking may start, at most.
const SOAK_ONSET: f64 = 400.0;

/// The flow through each cell, as on the plain the ground soaks it up.
///
/// Each cell adds one, as [`accumulate`] counts, but on the plain each loses
/// `SOAK × √flow`, at a rate that varies from place to place. So a stream
/// gives out soon after it comes down and a big river runs on, and where the
/// soaking starts varies too, so the courses don't all end on one circle.
fn soak(grid: &Grid, flood: &Flood, seed: u64) -> Vec<f64> {
    let mut flow = vec![0.0f64; flood.down.len()];
    for &c in &flood.order {
        flow[c as usize] = 1.0;
    }
    for &c in flood.order.iter().rev() {
        let c = c as usize;
        let (x, y) = grid.point(c);
        let onset = grid.rings.plain_edge - SOAK_ONSET * value_noise(seed ^ SOAK_FIELD, x, y, 900.0);
        let w = smoothstep((onset - grid.rings.radius(x, y)) / 100.0);
        if w > 0.0 {
            let rate = SOAK * w * (0.3 + 1.4 * value_noise(seed ^ SOAK_FIELD ^ 1, x, y, 350.0));
            flow[c] = (flow[c] - rate * flow[c].sqrt()).max(0.0);
        }
        let d = flood.down[c];
        if d != OUT {
            flow[d as usize] += flow[c];
        }
    }
    flow
}

/// How many cells drain through each, itself included.
fn accumulate(flood: &Flood) -> Vec<u32> {
    let mut acc = vec![0u32; flood.down.len()];
    for &c in &flood.order {
        acc[c as usize] = 1;
    }
    for &c in flood.order.iter().rev() {
        let d = flood.down[c as usize];
        if d != OUT {
            acc[d as usize] += acc[c as usize];
        }
    }
    acc
}

/// How near, in cells, a channel runs to a bigger one before it joins it.
const GATHER: i64 = 1;

/// Joins each channel to a bigger one it runs beside. On a smooth slope,
/// steepest descent sends neighbouring channels down side by side to the
/// rim, and smoothed they'd cross. Here a channel within [`GATHER`] cells of
/// a bigger one drains into it instead, unless it would climb to get there.
///
/// Then where two channels' drains cross on the diagonals of a square of
/// cells, the smaller drains into the bigger's cell.
///
/// Flow only grows downstream, so every drain, old or new, runs to a cell
/// that drained more before: the drainage stays a tree, and ordering the
/// cells by that flow, most first, keeps `order` upstream-last.
fn gather(grid: &Grid, flood: &mut Flood, pits: &Pits) {
    let acc = accumulate(flood);
    // The drainage as the flood left it: the walks follow it, not drains
    // this pass has already moved.
    let was = flood.down.clone();
    let channel = |c: usize| acc[c] as f64 >= STREAM_CELLS && pits.lake_of[c] == OUT;
    let n = grid.n as i64;
    for &c in &flood.order {
        let c = c as usize;
        if !channel(c) {
            continue;
        }
        let (i, j) = ((c % grid.n) as i64, (c / grid.n) as i64);
        let mut best: Option<usize> = None;
        // The biggest channel near, other than itself.
        for dj in -GATHER..=GATHER {
            for di in -GATHER..=GATHER {
                let (a, b) = (i + di, j + dj);
                if a < 0 || b < 0 || a >= n || b >= n {
                    continue;
                }
                let m = (b * n + a) as usize;
                if m == c || !channel(m) || acc[m] <= acc[c] || flood.filled[m] > flood.filled[c] + GATHER_CLIMB {
                    continue;
                }
                if best.is_none_or(|b| acc[m] > acc[b] || acc[m] == acc[b] && m < b) {
                    best = Some(m);
                }
            }
        }
        // Already on its way there, or into something as big: leave it be.
        // Flow only grows downstream, so the walk stops by the size of `m`.
        let Some(m) = best else { continue };
        let mut d = was[c];
        let mut on_its_way = false;
        while d != OUT && acc[d as usize] <= acc[m] {
            if d as usize == m {
                on_its_way = true;
                break;
            }
            d = was[d as usize];
        }
        let now = flood.down[c];
        if !on_its_way && (now == OUT || acc[now as usize] < acc[m]) {
            flood.down[c] = m as u32;
        }
    }
    // Two drains across the diagonals of one square of cells cross: turn the
    // smaller into the bigger's cell instead.
    for c in 0..flood.down.len() {
        let d = flood.down[c];
        if d == OUT || !channel(c) {
            continue;
        }
        let (i, j) = ((c % grid.n) as i64, (c / grid.n) as i64);
        let (di, dj) = ((d as usize % grid.n) as i64 - i, (d as usize / grid.n) as i64 - j);
        if di.abs() != 1 || dj.abs() != 1 {
            continue;
        }
        let (a, b) = ((j * n + i + di) as usize, ((j + dj) * n + i) as usize);
        for (from, to) in [(a, b), (b, a)] {
            if flood.down[from] == to as u32 && channel(from) {
                let (small, big) = if acc[from] < acc[c] { (from, c) } else { (c, from) };
                if acc[small] < acc[big] {
                    flood.down[small] = big as u32;
                }
            }
        }
    }
    flood.order.sort_by(|&a, &b| acc[b as usize].cmp(&acc[a as usize]).then(a.cmp(&b)));
}

/// How far a channel may climb to join a bigger one beside it.
const GATHER_CLIMB: f64 = 2.0;

impl Network {
    fn trace(grid: &Grid, flood: &Flood, pits: &Pits, seed: u64) -> Self {
        let cells = grid.h.len();
        let acc = soak(grid, flood, seed);
        let lake = |c: usize| pits.lake_of[c] != OUT;
        let channel = |c: usize| grid.inside(c) && acc[c] >= STREAM_CELLS && !lake(c);
        // What flows into each channel cell: channel cells and lake cells.
        let mut ups: Vec<Vec<u32>> = vec![Vec::new(); cells];
        let mut mouths = Vec::new();
        for &c in &flood.order {
            let c = c as usize;
            if !channel(c) && !lake(c) {
                continue;
            }
            let d = flood.down[c];
            if d == OUT {
                if channel(c) {
                    let end = match flood.rim[&(c as u32)] {
                        Rim::Edge => End::Edge,
                        Rim::Inner => End::Inner,
                    };
                    mouths.push((c, end));
                }
            } else if channel(d as usize) {
                ups[d as usize].push(c as u32);
            } else if channel(c) && lake(d as usize) {
                mouths.push((c, End::Pit(pits.lake_of[d as usize])));
            } else if channel(c) {
                mouths.push((c, End::Soaked));
            }
        }
        // Largest mouths first, so the ids run from the biggest water down.
        mouths.sort_by(|a, b| acc[b.0].total_cmp(&acc[a.0]).then(a.0.cmp(&b.0)));

        let mut courses = Vec::new();
        let mut tasks: std::collections::VecDeque<(usize, End)> = mouths.into_iter().collect();
        while let Some((start, end)) = tasks.pop_front() {
            let k = courses.len();
            let mut path = vec![start];
            let mut from_pit = None;
            let mut cur = start;
            loop {
                let mut inflow: Vec<usize> = ups[cur].iter().map(|&u| u as usize).collect();
                if inflow.is_empty() {
                    break;
                }
                inflow.sort_by(|&a, &b| acc[b].total_cmp(&acc[a]).then(a.cmp(&b)));
                for &other in &inflow[1..] {
                    if channel(other) {
                        tasks.push_back((other, End::Joins { course: k, cell: cur }));
                    }
                }
                let best = inflow[0];
                if lake(best) {
                    from_pit = Some(pits.lake_of[best]);
                    path.extend(grid.path_into_the_lake(best, pits));
                    break;
                }
                path.push(best);
                cur = best;
            }
            path.reverse();
            if end == End::Soaked && path.len() < 2 {
                // A channel that soaked away the moment it formed. Nothing
                // joins it, as nothing ran into it.
                continue;
            }
            if let End::Pit(p) = end {
                // End on the water: the cell the course spills into.
                path.push(flood.down[start] as usize);
                debug_assert_eq!(pits.lake_of[*path.last().unwrap()], p);
            }
            if let End::Joins { cell, .. } = end {
                path.push(cell);
            }
            courses.push(Traced { cells: path, from_pit, end });
        }
        Network { acc, courses }
    }

    /// What each course drains at each of its cells. A course out of a lake
    /// carries what the lake spills from its first point, not the lake cells'
    /// own. It's a river where this passes [`RIVER_CELLS`], and a stream elsewhere.
    fn course_acc(&self, c: &Traced, pits: &Pits) -> Vec<f64> {
        let mut acc: Vec<f64> = c.cells.iter().map(|&cell| self.acc[cell]).collect();
        let n = acc.len();
        if let Some(p) = c.from_pit {
            let out = c.cells.iter().position(|&cell| pits.lake_of[cell] != p).unwrap_or(n - 1).min(n - 1);
            for k in 0..out {
                acc[k] = acc[out];
            }
        }
        // Its last cell is in the lake or the course it runs into.
        if matches!(c.end, End::Pit(_) | End::Joins { .. }) && n > 1 {
            acc[n - 1] = acc[n - 2];
        }
        acc
    }

    fn finish(self, grid: &Grid, land: &LandParams, pits: &Pits, roads: &Roads) -> Hydrology {
        // Lakes are numbered pits first, then the ponds.
        let mut lakes: Vec<Lake> = pits.pits.iter().map(|pit| pit_lake(grid, pit)).collect();

        // Each course's points, smoothed, with the cell each stands for.
        let smoothed: Vec<Vec<(f64, f64)>> = self.courses.iter().map(|c| smooth(grid, &c.cells)).collect();
        let room = clearances(&smoothed, roads);
        let mut shapes: Vec<Vec<(f64, f64)>> = smoothed
            .into_iter()
            .zip(&room)
            .zip(&self.courses)
            .map(|((shape, room), c)| meander(land, &c.cells, shape, room))
            .collect();
        // A course that joins another ends on that one's point for the cell.
        for k in 0..self.courses.len() {
            if let End::Joins { course, cell } = self.courses[k].end {
                let at = self.courses[course].cells.iter().position(|&c| c == cell).expect("joins on its course");
                let p = shapes[course][at];
                *shapes[k].last_mut().unwrap() = p;
            }
        }

        // Where the plain soaks a course up: a pond, or it peters out.
        let mut pond_of = vec![None; self.courses.len()];
        for (k, c) in self.courses.iter().enumerate() {
            if !matches!(c.end, End::Soaked | End::Inner) {
                continue;
            }
            let peak = self.course_acc(c, pits).into_iter().fold(0.0, f64::max);
            let last = *c.cells.last().unwrap();
            let roll = (splitmix64(land.seed ^ POND_FIELD ^ last as u64) >> 11) as f64 / (1u64 << 53) as f64;
            let chance = if c.end == End::Inner { 1.0 } else { (POND_CHANCE * (flow(peak) - 0.4)).clamp(0.05, 0.8) };
            if roll >= chance {
                continue;
            }
            // Its own tributaries may run to the pond's shore.
            let others = shapes.iter().enumerate().filter(|&(m, _)| {
                m != k && !matches!(self.courses[m].end, End::Joins { course, .. } if course == k)
            });
            let others = others.map(|(_, s)| s.as_slice());
            if let Some(pond) = pond(&shapes[k], flow(peak), roll, &lakes, others, roads, &grid.rings) {
                pond_of[k] = Some(lakes.len());
                lakes.push(pond);
            }
        }

        // Width, depth and surface at every point, mains before what joins them.
        let mut points: Vec<Vec<WaterPoint>> = Vec::with_capacity(self.courses.len());
        let mut accs: Vec<Vec<f64>> = Vec::with_capacity(self.courses.len());
        for (k, c) in self.courses.iter().enumerate() {
            let shape = &mut shapes[k];
            if let Some(p) = pond_of[k] {
                // On into the pond, to its middle.
                shape.push(lake_centre(&lakes[p]));
            }
            if c.end == End::Edge {
                // On to the edge itself.
                let (x, y) = *shape.last().unwrap();
                let Rings { center: (cx, cy), outer, .. } = grid.rings;
                let (dx, dy) = (x - cx, y - cy);
                let s = outer / (dx * dx + dy * dy).sqrt();
                shape.push((cx + dx * s, cy + dy * s));
            }
            let mut acc = self.course_acc(c, pits);
            while acc.len() < shape.len() {
                acc.push(*acc.last().unwrap());
            }
            let mut surface: Vec<f64> = shape.iter().map(|&(x, y)| height_at(x, y, land)).collect();
            if let Some(p) = c.from_pit {
                // At the lake's level until it leaves the lake's cells.
                let level = pits.pits[p as usize].level;
                surface[0] = level;
                for (i, &q) in shape.iter().enumerate().skip(1) {
                    if pits.lake_of[grid.cell_at(q)] != p {
                        break;
                    }
                    surface[i] = level;
                }
            }
            for i in 1..surface.len() {
                surface[i] = surface[i].min(surface[i - 1]);
            }
            let floor = match c.end {
                End::Pit(p) => Some(pits.pits[p as usize].level),
                End::Joins { course, cell } => {
                    let at = self.courses[course].cells.iter().position(|&x| x == cell).unwrap();
                    Some(points[course][at].surface)
                }
                // A pond stands at the course's last surface.
                _ => pond_of[k].map(|p| {
                    lakes[p].level = round2(*surface.last().unwrap());
                    lakes[p].level
                }),
            };
            if let Some(floor) = floor {
                for s in surface.iter_mut() {
                    *s = s.max(floor);
                }
                *surface.last_mut().unwrap() = floor;
            }
            let fall = falls(shape, &surface);
            let n = shape.len();
            let peters_out = matches!(c.end, End::Soaked | End::Inner) && pond_of[k].is_none();
            let course: Vec<WaterPoint> = (0..n)
                .map(|i| {
                    let (mut width, mut depth) = section(flow(acc[i]), fall[i]);
                    if peters_out {
                        // It narrows to nothing over its last few points.
                        let left = (n - 1 - i) as f64 / PETER_OUT_POINTS;
                        let t = 0.15 + 0.85 * left.min(1.0);
                        (width, depth) = (round2(width * t), round2(depth * t));
                    }
                    WaterPoint { x: round2(shape[i].0), y: round2(shape[i].1), width, depth, surface: round2(surface[i]) }
                })
                .collect();
            points.push(course);
            accs.push(acc);
        }

        // Cut each course into runs of river and stream, and give out the ids.
        // A course grows into a river in the hills, and may soak back into a
        // stream on the plain. Each run starts on the last point of the one
        // before, and joins it there.
        let (mut n_rivers, mut n_streams) = (0, 0);
        // For each course, each run's id and first point.
        let parts: Vec<Vec<(String, usize)>> = accs
            .iter()
            .map(|acc| {
                let mut runs = Vec::new();
                for i in 0..acc.len() {
                    let river = acc[i] >= RIVER_CELLS;
                    if i == 0 || river != (acc[i - 1] >= RIVER_CELLS) {
                        let id = if river {
                            n_rivers += 1;
                            format!("river_{}", n_rivers - 1)
                        } else {
                            n_streams += 1;
                            format!("stream_{}", n_streams - 1)
                        };
                        runs.push((id, i.saturating_sub(1)));
                    }
                }
                // A run of one point (the flow crossed over at once) folds
                // into the next.
                let starts: Vec<usize> = runs.iter().map(|r| r.1).collect();
                let mut r = 0;
                runs.retain(|_| {
                    r += 1;
                    starts.get(r) != Some(&starts[r - 1])
                });
                runs
            })
            .collect();
        let lake_id = |p: usize| format!("lake_{p}");
        let (mut rivers, mut streams) = (Vec::new(), Vec::new());
        for (k, c) in self.courses.iter().enumerate() {
            let mouth = match c.end {
                End::Edge => Mouth::Edge,
                End::Pit(p) => Mouth::Lake(lake_id(p as usize)),
                End::Joins { course, cell } => {
                    let at = self.courses[course].cells.iter().position(|&x| x == cell).unwrap();
                    let run = parts[course].iter().rev().find(|(_, from)| *from <= at).unwrap();
                    Mouth::Joins(run.0.clone())
                }
                End::Soaked | End::Inner => match pond_of[k] {
                    Some(p) => Mouth::Lake(lake_id(p)),
                    None => Mouth::PetersOut,
                },
            };
            let pts = &points[k];
            let runs = &parts[k];
            for (r, (id, from)) in runs.iter().enumerate() {
                let to = runs.get(r + 1).map(|next| next.1).unwrap_or(pts.len() - 1);
                let course = WaterCourse {
                    id: id.clone(),
                    source: if r == 0 { c.from_pit.map(|p| lake_id(p as usize)) } else { None },
                    mouth: match runs.get(r + 1) {
                        Some(next) => Mouth::Joins(next.0.clone()),
                        None => mouth.clone(),
                    },
                    points: pts[*from..=to].to_vec(),
                };
                if id.starts_with("river") {
                    rivers.push(course);
                } else {
                    streams.push(course);
                }
            }
        }
        rivers.sort_by_key(|c| id_number(&c.id));
        streams.sort_by_key(|c| id_number(&c.id));
        for (i, lake) in lakes.iter_mut().enumerate() {
            lake.id = lake_id(i);
        }
        let fords = fords(&rivers);
        Hydrology { rivers, streams, lakes, fords }
    }
}

fn id_number(id: &str) -> usize {
    id.rsplit('_').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// How far a course meanders either side of its cells on flat ground.
const MEANDER: f64 = 55.0;
/// The slope at which the meander has halved.
const MEANDER_SLOPE: f64 = 0.15;

/// How much of the way to the nearest other water a course may swing. Two
/// courses each swinging less than half can't cross.
const MEANDER_SHARE: f64 = 0.3;

/// A course's points: the cells' centres, smoothed so the grid's 45° steps
/// don't show. The ends stay put.
fn smooth(grid: &Grid, cells: &[usize]) -> Vec<(f64, f64)> {
    let mut p: Vec<(f64, f64)> = cells.iter().map(|&c| grid.point(c)).collect();
    for _ in 0..3 {
        let q = p.clone();
        for i in 1..p.len().saturating_sub(1) {
            p[i] = (
                0.25 * q[i - 1].0 + 0.5 * q[i].0 + 0.25 * q[i + 1].0,
                0.25 * q[i - 1].1 + 0.5 * q[i].1 + 0.25 * q[i + 1].1,
            );
        }
    }
    p
}

/// How far each point of each course is from the nearest point of another,
/// or from a road's kerb, up to as far as a meander could use.
fn clearances(shapes: &[Vec<(f64, f64)>], roads: &Roads) -> Vec<Vec<f64>> {
    let reach = MEANDER / MEANDER_SHARE + CELL;
    let key = |p: (f64, f64)| ((p.0 / reach).floor() as i64, (p.1 / reach).floor() as i64);
    let mut buckets: HashMap<(i64, i64), Vec<(usize, (f64, f64))>> = HashMap::new();
    for (k, shape) in shapes.iter().enumerate() {
        for &p in shape {
            buckets.entry(key(p)).or_default().push((k, p));
        }
    }
    shapes
        .iter()
        .enumerate()
        .map(|(k, shape)| {
            shape
                .iter()
                .map(|&p| {
                    let (bi, bj) = key(p);
                    // A road counts as other water: a course crosses it
                    // straight, and doesn't swing onto it.
                    let mut best = roads.clearance(p, reach);
                    for bj in bj - 1..=bj + 1 {
                        for bi in bi - 1..=bi + 1 {
                            for &(other, q) in buckets.get(&(bi, bj)).map(Vec::as_slice).unwrap_or(&[]) {
                                if other != k {
                                    best = best.min(((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt());
                                }
                            }
                        }
                    }
                    best
                })
                .collect()
        })
        .collect()
}

/// Swings a smoothed course from side to side. Steepest descent runs dead
/// straight down a smooth slope, so on gentle ground it meanders, the more
/// the flatter. On steep ground it keeps to the valley floor, and it never
/// swings more than [`MEANDER_SHARE`] of the way to other water. The ends
/// stay put.
fn meander(land: &LandParams, cells: &[usize], mut p: Vec<(f64, f64)>, clearance: &[f64]) -> Vec<(f64, f64)> {
    let n = p.len();
    let mut along = vec![0.0; n];
    for i in 1..n {
        along[i] = along[i - 1] + ((p[i].0 - p[i - 1].0).powi(2) + (p[i].1 - p[i - 1].1).powi(2)).sqrt();
    }
    let total = along[n - 1];
    // Each course swings to its own phase, from where it rises.
    let phase = (cells[0] % 997) as f64 * 0.37;
    let q = p.clone();
    for i in 1..n.saturating_sub(1) {
        let (dx, dy) = (q[i + 1].0 - q[i - 1].0, q[i + 1].1 - q[i - 1].1);
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            continue;
        }
        let s = along[i];
        let taper = smoothstep(s.min(total - s) / 150.0);
        let slope = capitol_protocol::land::slope_at(q[i].0, q[i].1, land) / MEANDER_SLOPE;
        let swing = 0.65 * (s / 420.0 * std::f64::consts::TAU + phase).stable_sin()
            + 0.35 * (s / 170.0 * std::f64::consts::TAU + 2.0 * phase).stable_sin();
        let room = MEANDER_SHARE * (clearance[i] - CELL / 2.0).max(0.0);
        let off = (MEANDER / (1.0 + slope * slope)).min(room) * taper * swing;
        p[i] = (q[i].0 - dy / len * off, q[i].1 + dx / len * off);
    }
    p
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much water a course carries, from the cells it drains: about 1 where
/// it becomes a river.
fn flow(cells: f64) -> f64 {
    (cells / RIVER_CELLS).sqrt()
}

/// The surface's fall per unit length round each point, over about 150 units.
fn falls(shape: &[(f64, f64)], surface: &[f64]) -> Vec<f64> {
    let n = shape.len();
    let mut along = vec![0.0; n];
    for i in 1..n {
        let (dx, dy) = (shape[i].0 - shape[i - 1].0, shape[i].1 - shape[i - 1].1);
        along[i] = along[i - 1] + (dx * dx + dy * dy).sqrt();
    }
    (0..n)
        .map(|i| {
            let a = (0..=i).rev().find(|&j| along[i] - along[j] >= 75.0).unwrap_or(0);
            let b = (i..n).find(|&j| along[j] - along[i] >= 75.0).unwrap_or(n - 1);
            let run = along[b] - along[a];
            if run > 0.0 { (surface[a] - surface[b]) / run } else { 0.0 }
        })
        .collect()
}

/// A course's width and depth from its flow, and how steeply it falls: on a
/// flat it spreads wide and shallow, on a steep run it's narrow and deep.
fn section(flow: f64, fall: f64) -> (f64, f64) {
    let flat = 1.0 / (1.0 + fall / 0.03);
    let width = (5.0 + 16.0 * flow) * (0.7 + 0.7 * flat);
    let depth = (0.4 + 1.0 * flow) * (1.3 - 0.7 * flat);
    (round2(width.clamp(3.0, 80.0)), round2(depth.clamp(0.3, 8.0)))
}

/// The fords: on each river, the shallowest points that are wide and shallow
/// enough, at least [`FORD_SPACING`] apart along it, and off its ends.
fn fords(rivers: &[WaterCourse]) -> Vec<Ford> {
    let mut fords = Vec::new();
    for river in rivers {
        let p = &river.points;
        let mut along = vec![0.0; p.len()];
        for i in 1..p.len() {
            along[i] = along[i - 1] + ((p[i].x - p[i - 1].x).powi(2) + (p[i].y - p[i - 1].y).powi(2)).sqrt();
        }
        let total = *along.last().unwrap_or(&0.0);
        let mut picks: Vec<usize> = (0..p.len())
            .filter(|&i| p[i].depth <= FORD_DEPTH && along[i] > 60.0 && total - along[i] > 60.0)
            .collect();
        picks.sort_by(|&a, &b| p[a].depth.total_cmp(&p[b].depth).then(a.cmp(&b)));
        let mut taken: Vec<usize> = Vec::new();
        for i in picks {
            if taken.iter().all(|&t| (along[t] - along[i]).abs() >= FORD_SPACING) {
                taken.push(i);
            }
        }
        taken.sort();
        for i in taken {
            fords.push(Ford {
                id: format!("ford_{}", fords.len()),
                river_id: river.id.clone(),
                x: p[i].x,
                y: p[i].y,
                width: round2((p[i].width * 1.5).max(20.0)),
                depth: p[i].depth,
            });
        }
    }
    fords
}

// ---------------------------------------------------------------------------
// Lakes

/// A pit's lake: its shore is where the ground meets its level, traced by
/// marching squares over the cells round it.
fn pit_lake(grid: &Grid, pit: &Pit) -> Lake {
    let n = grid.n;
    let (mut i0, mut j0, mut i1, mut j1) = (n, n, 0, 0);
    for &c in &pit.cells {
        let (i, j) = (c % n, c / n);
        (i0, j0, i1, j1) = (i0.min(i), j0.min(j), i1.max(i), j1.max(j));
    }
    // One cell of shore all round, so every contour closes.
    let (i0, j0) = (i0.saturating_sub(1), j0.saturating_sub(1));
    let (i1, j1) = ((i1 + 1).min(n - 1), (j1 + 1).min(n - 1));
    let w = i1 - i0 + 1;
    let mut wet = vec![-1.0; w * (j1 - j0 + 1)];
    for &c in &pit.cells {
        let (i, j) = (c % n - i0, c / n - j0);
        wet[j * w + i] = pit.level - grid.h[c];
    }
    for j in 0..=(j1 - j0) {
        for i in 0..w {
            let c = (j + j0) * n + i + i0;
            if wet[j * w + i] < 0.0 && grid.inside(c) {
                // The shore: how far above the level, as a negative.
                wet[j * w + i] = (pit.level - grid.h[c]).min(-1e-3);
            }
        }
    }
    let rings = marching_squares(&wet, w, j1 - j0 + 1, |i, j| centre(i + i0, j + j0));
    lake_from_rings(pit.level, rings)
}

fn lake_from_rings(level: f64, rings: Vec<Vec<(f64, f64)>>) -> Lake {
    let rings: Vec<Vec<(f64, f64)>> = rings
        .into_iter()
        .filter(|r| signed_area(r).abs() >= CELL * CELL * 0.5)
        .map(|r| r.into_iter().map(|(x, y)| (round2(x), round2(y))).collect())
        .collect();
    // A ring inside an odd number of others is an island.
    let mut area = 0.0;
    for (k, ring) in rings.iter().enumerate() {
        let (x, y) = ring[0];
        let depth = rings.iter().enumerate().filter(|&(m, other)| m != k && contains(other, x, y)).count();
        let a = signed_area(ring).abs();
        area += if depth % 2 == 0 { a } else { -a };
    }
    Lake { id: String::new(), level: round2(level), rings, area: round2(area) }
}

fn signed_area(ring: &[(f64, f64)]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| ring[i].0 * ring[(i + 1) % n].1 - ring[(i + 1) % n].0 * ring[i].1).sum::<f64>() / 2.0
}

fn contains(ring: &[(f64, f64)], x: f64, y: f64) -> bool {
    let n = ring.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
            inside = !inside;
        }
    }
    inside
}

/// The closed contours where `f` crosses 0, over a `w` by `h` grid of samples
/// at `at(i, j)`. The border must be negative, so every contour closes.
fn marching_squares(f: &[f64], w: usize, h: usize, at: impl Fn(usize, usize) -> (f64, f64)) -> Vec<Vec<(f64, f64)>> {
    // An edge of the sample grid: (i, j, 0) runs to (i+1, j), (i, j, 1) to (i, j+1).
    type Edge = (usize, usize, u8);
    let value = |i: usize, j: usize| f[j * w + i];
    let cross = |e: Edge| {
        let (i, j, d) = e;
        let (i2, j2) = if d == 0 { (i + 1, j) } else { (i, j + 1) };
        let (a, b) = (value(i, j), value(i2, j2));
        let t = (a / (a - b)).clamp(0.0, 1.0);
        let (p, q) = (at(i, j), at(i2, j2));
        (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1))
    };
    let mut next: HashMap<Edge, Edge> = HashMap::new();
    for j in 0..h - 1 {
        for i in 0..w - 1 {
            let inside = |a: usize, b: usize| value(a, b) > 0.0;
            let (bl, br, tr, tl) = (inside(i, j), inside(i + 1, j), inside(i + 1, j + 1), inside(i, j + 1));
            let (bottom, right, top, left) = ((i, j, 0), (i + 1, j, 1), (i, j + 1, 0), (i, j, 1));
            // Each segment runs with the water on its right (y up).
            let mut seg = |from: Edge, to: Edge| {
                next.insert(from, to);
            };
            match (bl, br, tr, tl) {
                (false, false, false, false) | (true, true, true, true) => {}
                (true, false, false, false) => seg(left, bottom),
                (false, true, false, false) => seg(bottom, right),
                (false, false, true, false) => seg(right, top),
                (false, false, false, true) => seg(top, left),
                (true, true, false, false) => seg(left, right),
                (false, true, true, false) => seg(bottom, top),
                (false, false, true, true) => seg(right, left),
                (true, false, false, true) => seg(top, bottom),
                (false, true, true, true) => seg(bottom, left),
                (true, false, true, true) => seg(right, bottom),
                (true, true, false, true) => seg(top, right),
                (true, true, true, false) => seg(left, top),
                (true, false, true, false) | (false, true, false, true) => {
                    let mid = (value(i, j) + value(i + 1, j) + value(i + 1, j + 1) + value(i, j + 1)) / 4.0;
                    match (bl, mid > 0.0) {
                        (true, true) => {
                            seg(left, top);
                            seg(right, bottom);
                        }
                        (true, false) => {
                            seg(left, bottom);
                            seg(right, top);
                        }
                        (false, true) => {
                            seg(bottom, left);
                            seg(top, right);
                        }
                        (false, false) => {
                            seg(bottom, right);
                            seg(top, left);
                        }
                    }
                }
            }
        }
    }
    let mut starts: Vec<Edge> = next.keys().copied().collect();
    starts.sort();
    let mut rings = Vec::new();
    let mut used = std::collections::HashSet::new();
    for start in starts {
        if used.contains(&start) {
            continue;
        }
        let mut ring = Vec::new();
        let mut e = start;
        while used.insert(e) {
            ring.push(cross(e));
            match next.get(&e) {
                Some(&n) => e = n,
                None => break,
            }
        }
        if ring.len() >= 3 {
            rings.push(ring);
        }
    }
    rings
}

/// How strongly a course's flow makes for a pond where it soaks away.
const POND_CHANCE: f64 = 0.6;
/// Over how many points a course that peters out narrows to nothing.
const PETER_OUT_POINTS: f64 = 6.0;
/// How far a pond's shore keeps from a road's kerb.
const POND_KERB: f64 = 8.0;

/// A pond where a course soaks away, lying along its last stretch, sized by
/// its flow `q`. It's made smaller to fit, and given up (the course peters
/// out instead) if even small it would reach a road, another lake or
/// another course, or come in past the inner rim (`Rings::inner`).
fn pond<'a>(
    shape: &[(f64, f64)],
    q: f64,
    roll: f64,
    lakes: &[Lake],
    others: impl Iterator<Item = &'a [(f64, f64)]>,
    roads: &Roads,
    rings: &Rings,
) -> Option<Lake> {
    let n = shape.len();
    let end = shape[n - 1];
    let back = shape[n.saturating_sub(4)];
    let (dx, dy) = (end.0 - back.0, end.1 - back.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    let others: Vec<&[(f64, f64)]> = others.collect();
    let phase = roll * 37.0;
    for scale in [1.0, 0.75, 0.55] {
        let a = (28.0 + 40.0 * q).min(150.0) * scale;
        let b = (0.55 * a).max(16.0);
        let (cx, cy) = (end.0 + ux * 0.6 * a, end.1 + uy * 0.6 * a);
        // An oval along the course, its shore wobbling a little.
        let ring: Vec<(f64, f64)> = (0..36)
            .map(|k| {
                let t = k as f64 / 36.0 * std::f64::consts::TAU;
                let wobble = 1.0 + 0.1 * (3.0 * t + phase).stable_sin() + 0.06 * (5.0 * t - phase).stable_cos();
                let (along, across) = (a * t.stable_cos() * wobble, b * t.stable_sin() * wobble);
                (cx + ux * along - uy * across, cy + uy * along + ux * across)
            })
            .collect();
        let fits = ring.iter().chain([&(cx, cy)]).all(|&p| {
            rings.radius(p.0, p.1) >= rings.inner + 10.0
                && roads.clearance(p, POND_KERB) >= POND_KERB
                && !lakes.iter().any(|l| l.contains(p.0, p.1))
        }) && !lakes.iter().any(|l| l.rings.iter().flatten().any(|&(x, y)| contains(&ring, x, y)))
            && !others.iter().any(|o| o.iter().any(|&(x, y)| contains(&ring, x, y)));
        if fits {
            return Some(lake_from_rings(0.0, vec![ring]));
        }
    }
    None
}

/// A lake's middle: the mean of its first shore.
fn lake_centre(lake: &Lake) -> (f64, f64) {
    let ring = &lake.rings[0];
    let n = ring.len() as f64;
    (ring.iter().map(|p| p.0).sum::<f64>() / n, ring.iter().map(|p| p.1).sum::<f64>() / n)
}

#[cfg(test)]
mod tests;
