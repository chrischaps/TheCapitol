//! The wilds' water, from the height of the land (W3, #196).
//!
//! A pure function of [`LandParams`], so the default world's water is pinned
//! with the rest of it (`default_terrain_version_is_pinned`).
//!
//! 1. **Sample** `land::height_at` on a grid of [`CELL`] over the wilds: from
//!    [`FOOT_RADIUS`], just past the rural edge, out to the world's edge.
//! 2. **Priority-flood** in from the grid's two rims: the world's edge, and
//!    the foot of the hills, where the ground runs flat to the city. Each cell
//!    drains to the cell that flooded it, so the drainage runs through every
//!    pit and flat to a rim, and each pit fills to the level it spills at.
//! 3. **Lakes** are the pits deep and wide enough, each with its level and a
//!    shore traced by marching squares. Water that comes down to the foot of
//!    the hills can't run on into the city, so it ends in a **mere** there,
//!    dug into the flat ground at level 0.
//! 4. **Flow** accumulates down the drainage. Streams are where it passes
//!    [`STREAM_CELLS`] and rivers where it passes [`RIVER_CELLS`]. Each course
//!    follows its largest inflow up, and every other inflow is a course of its
//!    own that joins it. Courses are smoothed, then their surfaces are made to
//!    fall downstream: the carve (`land::carve`) cuts them through what's left.
//! 5. **Fords** are the shallowest stretches of each river, spaced apart.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use capitol_protocol::fmath::StableMath;
use capitol_protocol::land::{height_at, LandParams};
use capitol_protocol::rules::{CENTER_X, CENTER_Y, RURAL_OUTER_RADIUS, WILDERNESS_OUTER_RADIUS, WORLD_SIZE};
use capitol_protocol::terrain::{Ford, Lake, Mouth, WaterCourse, WaterPoint};

/// The grid's cell, in world units.
pub const CELL: f64 = 30.0;
/// The grid's inner rim: the foot of the hills. Water reaching it ends in a mere.
pub const FOOT_RADIUS: f64 = RURAL_OUTER_RADIUS + 80.0;
/// How far a mere's inner shore stays out from the rural edge.
const MERE_CLEAR: f64 = 25.0;
/// Upstream cells a stream drains, at least.
pub const STREAM_CELLS: u32 = 120;
/// Upstream cells a river drains, at least.
pub const RIVER_CELLS: u32 = 800;
/// A pit becomes a lake when it covers this many cells...
const LAKE_MIN_CELLS: usize = 20;
/// ...and is this deep somewhere.
const LAKE_MIN_DEPTH: f64 = 4.0;
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

/// The wilds' water for this land.
pub fn generate(land: &LandParams) -> Hydrology {
    let grid = Grid::sample(land);
    let flood = grid.flood();
    let pits = grid.pits(&flood);
    let network = Network::trace(&grid, &flood, &pits);
    network.finish(&grid, land, &pits)
}

// ---------------------------------------------------------------------------
// The grid

const OUT: u32 = u32::MAX;

struct Grid {
    n: usize,
    /// The height at each cell's centre; NaN outside the wilds.
    h: Vec<f64>,
}

/// The eight neighbours, as (di, dj).
const NEIGHBOURS: [(i64, i64); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rim {
    /// The world's edge.
    Edge,
    /// The foot of the hills, where the ground runs flat to the city.
    Foot,
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
    fn sample(land: &LandParams) -> Self {
        let n = (WORLD_SIZE / CELL).round() as usize;
        let mut h = vec![f64::NAN; n * n];
        for j in 0..n {
            for i in 0..n {
                let (x, y) = centre(i, j);
                let r = radius(x, y);
                if (FOOT_RADIUS..=WILDERNESS_OUTER_RADIUS).contains(&r) {
                    h[j * n + i] = height_at(x, y, land);
                }
            }
        }
        Grid { n, h }
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
            // A rim cell has a neighbour outside the wilds: past the edge, or
            // inside the foot.
            if !self.inside(c) || self.neighbours(c).count() == 8 && self.neighbours(c).all(|m| self.inside(m)) {
                continue;
            }
            let (x, y) = self.point(c);
            let edge = radius(x, y) > 0.5 * (FOOT_RADIUS + WILDERNESS_OUTER_RADIUS);
            rim.insert(c as u32, if edge { Rim::Edge } else { Rim::Foot });
            {
                filled[c] = self.h[c];
                queued[c] = true;
                heap.push(Entry { level: self.h[c], seq, cell: c as u32 });
                seq += 1;
            }
        }
        let mut order = Vec::with_capacity(cells);
        while let Some(Entry { level, cell, .. }) = heap.pop() {
            order.push(cell);
            for m in self.neighbours(cell as usize) {
                if queued[m] || !self.inside(m) {
                    continue;
                }
                queued[m] = true;
                filled[m] = self.h[m].max(level);
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

    /// The pits that hold lakes, and the mere each foot cell drains into is
    /// made later. A pit is a connected run of cells filled above their height
    /// to one level.
    fn pits(&self, flood: &Flood) -> Pits {
        let cells = self.h.len();
        let mut lake_of = vec![OUT; cells];
        let mut pits = Vec::new();
        let mut seen = vec![false; cells];
        for start in 0..cells {
            if seen[start] || !self.inside(start) || flood.filled[start] - self.h[start] <= 1e-9 {
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
                    if !seen[m] && self.inside(m) && flood.filled[m] == level && level - self.h[m] > 1e-9 {
                        seen[m] = true;
                        group.push(m);
                    }
                }
            }
            let depth = group.iter().map(|&c| level - self.h[c]).fold(0.0, f64::max);
            if group.len() >= LAKE_MIN_CELLS && depth >= LAKE_MIN_DEPTH {
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

fn radius(x: f64, y: f64) -> f64 {
    let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
    (dx * dx + dy * dy).sqrt()
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
    Foot,
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
    acc: Vec<u32>,
    courses: Vec<Traced>,
}

impl Network {
    fn trace(grid: &Grid, flood: &Flood, pits: &Pits) -> Self {
        let cells = grid.h.len();
        let mut acc = vec![0u32; cells];
        for &c in &flood.order {
            acc[c as usize] = 1;
        }
        for &c in flood.order.iter().rev() {
            let d = flood.down[c as usize];
            if d != OUT {
                acc[d as usize] += acc[c as usize];
            }
        }
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
                        Rim::Foot => End::Foot,
                    };
                    mouths.push((c, end));
                }
            } else if channel(d as usize) {
                ups[d as usize].push(c as u32);
            } else if channel(c) && lake(d as usize) {
                mouths.push((c, End::Pit(pits.lake_of[d as usize])));
            }
        }
        // Largest mouths first, so the ids run from the biggest water down.
        mouths.sort_by(|a, b| acc[b.0].cmp(&acc[a.0]).then(a.0.cmp(&b.0)));

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
                inflow.sort_by(|&a, &b| acc[b].cmp(&acc[a]).then(a.cmp(&b)));
                for &other in &inflow[1..] {
                    if channel(other) {
                        tasks.push_back((other, End::Joins { course: k, cell: cur }));
                    }
                }
                let best = inflow[0];
                if lake(best) {
                    from_pit = Some(pits.lake_of[best]);
                    path.push(best);
                    break;
                }
                path.push(best);
                cur = best;
            }
            path.reverse();
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
    /// carries what the lake spills from its first point, not the lake cell's own.
    /// It splits from a stream into a river where this passes [`RIVER_CELLS`].
    fn course_acc(&self, c: &Traced) -> Vec<u32> {
        let mut acc: Vec<u32> = c.cells.iter().map(|&cell| self.acc[cell]).collect();
        let n = acc.len();
        if c.from_pit.is_some() && n > 1 {
            acc[0] = acc[1];
        }
        // Its last cell is in the lake or the course it runs into.
        if matches!(c.end, End::Pit(_) | End::Joins { .. }) && n > 1 {
            acc[n - 1] = acc[n - 2];
        }
        acc
    }

    fn finish(self, grid: &Grid, land: &LandParams, pits: &Pits) -> Hydrology {
        // Lakes are numbered pits first, then the meres.
        let mut lakes: Vec<Lake> = pits.pits.iter().map(|pit| pit_lake(grid, pit)).collect();

        // Each course's points, smoothed, with the cell each stands for.
        let mut shapes: Vec<Vec<(f64, f64)>> = self.courses.iter().map(|c| smooth(grid, land, &c.cells)).collect();
        // A course that joins another ends on that one's point for the cell.
        for k in 0..self.courses.len() {
            if let End::Joins { course, cell } = self.courses[k].end {
                let at = self.courses[course].cells.iter().position(|&c| c == cell).expect("joins on its course");
                let p = shapes[course][at];
                *shapes[k].last_mut().unwrap() = p;
            }
        }

        // Meres where water comes down to the foot of the hills.
        let mut mere_of = vec![None; self.courses.len()];
        let mut meres: Vec<(f64, f64, f64, Vec<usize>)> = Vec::new();
        for (k, c) in self.courses.iter().enumerate() {
            if c.end == End::Foot {
                let (x, y) = *shapes[k].last().unwrap();
                let q = flow(self.acc[c.cells[c.cells.len() - 1]]);
                meres.push((x, y, q, vec![k]));
            }
        }
        let meres = merge_meres(meres);
        for (cells, mere) in meres {
            let id = lakes.len();
            for &k in &cells {
                mere_of[k] = Some(id);
            }
            lakes.push(mere);
        }

        // Width, depth and surface at every point, mains before what joins them.
        let mut points: Vec<Vec<WaterPoint>> = Vec::with_capacity(self.courses.len());
        for (k, c) in self.courses.iter().enumerate() {
            let shape = &mut shapes[k];
            if let Some(m) = mere_of[k] {
                // On into the mere, to its middle.
                let (cx, cy) = mere_centre(&lakes[m]);
                shape.push((cx, cy));
            }
            if c.end == End::Edge {
                // On to the edge itself.
                let (x, y) = *shape.last().unwrap();
                let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
                let s = WILDERNESS_OUTER_RADIUS / (dx * dx + dy * dy).sqrt();
                shape.push((CENTER_X + dx * s, CENTER_Y + dy * s));
            }
            let mut acc = self.course_acc(c);
            while acc.len() < shape.len() {
                acc.push(*acc.last().unwrap());
            }
            let mut surface: Vec<f64> = shape.iter().map(|&(x, y)| height_at(x, y, land)).collect();
            if let Some(p) = c.from_pit {
                surface[0] = pits.pits[p as usize].level;
            }
            for i in 1..surface.len() {
                surface[i] = surface[i].min(surface[i - 1]);
            }
            let floor = match c.end {
                End::Pit(p) => Some(pits.pits[p as usize].level),
                End::Foot => Some(0.0),
                End::Joins { course, cell } => {
                    let at = self.courses[course].cells.iter().position(|&x| x == cell).unwrap();
                    Some(points[course][at].surface)
                }
                End::Edge => None,
            };
            if let Some(floor) = floor {
                for s in surface.iter_mut() {
                    *s = s.max(floor);
                }
                *surface.last_mut().unwrap() = floor;
            }
            let fall = falls(shape, &surface);
            let course: Vec<WaterPoint> = (0..shape.len())
                .map(|i| {
                    let (width, depth) = section(flow(acc[i]), fall[i]);
                    WaterPoint { x: round2(shape[i].0), y: round2(shape[i].1), width, depth, surface: round2(surface[i]) }
                })
                .collect();
            points.push(course);
        }

        // Split each course where it becomes a river; give out the ids.
        let mut rivers = Vec::new();
        let mut streams = Vec::new();
        // For each course: (stream id, river id, the index its river part starts at).
        let mut parts: Vec<(Option<String>, Option<String>, usize)> = Vec::new();
        let (mut n_rivers, mut n_streams) = (0, 0);
        for (k, c) in self.courses.iter().enumerate() {
            // Flow only grows downstream, so a course is a stream, then a river.
            let split = self.course_acc(c).iter().position(|&a| a >= RIVER_CELLS);
            let river_id = split.map(|_| {
                n_rivers += 1;
                format!("river_{}", n_rivers - 1)
            });
            let stream_id = (split != Some(0)).then(|| {
                n_streams += 1;
                format!("stream_{}", n_streams - 1)
            });
            parts.push((stream_id, river_id, split.unwrap_or(points[k].len())));
        }
        let lake_id = |p: usize| format!("lake_{p}");
        for (k, c) in self.courses.iter().enumerate() {
            let (stream_id, river_id, split) = &parts[k];
            let mouth = match c.end {
                End::Edge => Mouth::Edge,
                End::Foot => Mouth::Lake(lake_id(mere_of[k].unwrap())),
                End::Pit(p) => Mouth::Lake(lake_id(p as usize)),
                End::Joins { course, cell } => {
                    let at = self.courses[course].cells.iter().position(|&x| x == cell).unwrap();
                    let (s, r, from) = &parts[course];
                    Mouth::Joins(if at >= *from { r.clone().unwrap() } else { s.clone().unwrap() })
                }
            };
            let source = c.from_pit.map(|p| lake_id(p as usize));
            let pts = &points[k];
            match (stream_id, river_id) {
                (Some(s), Some(r)) => {
                    streams.push(WaterCourse {
                        id: s.clone(),
                        source,
                        mouth: Mouth::Joins(r.clone()),
                        points: pts[..=*split].to_vec(),
                    });
                    rivers.push(WaterCourse { id: r.clone(), source: None, mouth, points: pts[*split..].to_vec() });
                }
                (Some(s), None) => streams.push(WaterCourse { id: s.clone(), source, mouth, points: pts.clone() }),
                (None, Some(r)) => rivers.push(WaterCourse { id: r.clone(), source, mouth, points: pts.clone() }),
                (None, None) => unreachable!(),
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

/// A course's points: the cells' centres, smoothed so the grid's 45° steps
/// don't show. Then it meanders: steepest descent runs dead straight down a
/// smooth slope, so on gentle ground it's swung from side to side, the more
/// the flatter. On steep ground it keeps to the valley floor. The ends stay put.
fn smooth(grid: &Grid, land: &LandParams, cells: &[usize]) -> Vec<(f64, f64)> {
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
        let off = MEANDER * taper * swing / (1.0 + slope * slope);
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
fn flow(cells: u32) -> f64 {
    (cells as f64 / RIVER_CELLS as f64).sqrt()
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

/// The meres at the foot of the hills: one where each course comes down,
/// lying along the ring, and any that would overlap made one.
/// Each is (x, y, flow, courses) in; (courses, lake) out.
fn merge_meres(mut meres: Vec<(f64, f64, f64, Vec<usize>)>) -> Vec<(Vec<usize>, Lake)> {
    let half_length = |q: f64| (30.0 + 40.0 * q).min(160.0);
    // Merge any two whose lengths along the ring overlap, until none do.
    loop {
        let mut merged = false;
        'outer: for a in 0..meres.len() {
            for b in a + 1..meres.len() {
                let (ax, ay, aq, _) = meres[a];
                let (bx, by, bq, _) = meres[b];
                let d = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                if d < half_length(aq) + half_length(bq) + 20.0 {
                    let (_, _, _, cb) = meres.remove(b);
                    let q = (aq * aq + bq * bq).sqrt();
                    let wa = aq * aq / (aq * aq + bq * bq);
                    let m = &mut meres[a];
                    m.0 = ax * wa + bx * (1.0 - wa);
                    m.1 = ay * wa + by * (1.0 - wa);
                    m.2 = q;
                    m.3.extend(cb);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
    meres
        .into_iter()
        .map(|(x, y, q, courses)| {
            let a = half_length(q);
            let b = (0.45 * a).clamp(20.0, 45.0);
            let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
            let r = (dx * dx + dy * dy).sqrt();
            let (ux, uy) = (dx / r, dy / r);
            let rc = RURAL_OUTER_RADIUS + MERE_CLEAR + b;
            let (cx, cy) = (CENTER_X + ux * rc, CENTER_Y + uy * rc);
            // An ellipse along the ring, its shore wobbling a little.
            let phase = (x * 0.013 + y * 0.007).stable_sin() * 3.0;
            let ring: Vec<(f64, f64)> = (0..36)
                .map(|k| {
                    let t = k as f64 / 36.0 * std::f64::consts::TAU;
                    let wobble = 1.0 + 0.08 * (3.0 * t + phase).stable_sin() + 0.05 * (5.0 * t - phase).stable_cos();
                    let (s, c) = (t.stable_sin(), t.stable_cos());
                    // Along the ring is (-uy, ux); out from the centre is (ux, uy).
                    let (along, out) = (a * c * wobble, b * s * wobble.min(1.0));
                    (cx - uy * along + ux * out, cy + ux * along + uy * out)
                })
                .collect();
            (courses, lake_from_rings(0.0, vec![ring]))
        })
        .collect()
}

/// A mere's middle: the mean of its shore.
fn mere_centre(lake: &Lake) -> (f64, f64) {
    let ring = &lake.rings[0];
    let n = ring.len() as f64;
    (ring.iter().map(|p| p.0).sum::<f64>() / n, ring.iter().map(|p| p.1).sum::<f64>() / n)
}

#[cfg(test)]
mod tests;
