//! The wild trails (W5, #198): the arterials run on past the rural edge, up
//! the valleys and over the ranges.
//!
//! A pure function of the land, the water and the roads already laid, so the
//! default world's trails are pinned with the rest of it
//! (`default_terrain_version_is_pinned`).
//!
//! Every trail is a least-cost path on a grid of [`CELL`] over the wilds. A
//! step costs its length, more the steeper it climbs or falls ([`GRADE_COST`]),
//! so a trail winds along the contours and up a valley's floor, and more again
//! into water: wading a stream, crossing at a ford, or bridging a river
//! ([`BRIDGE_COST`]). Ground steeper than [`TRAIL_SLOPE`] and the lakes are
//! out of bounds, so a trail that meets a lake goes round its shore.
//!
//! 1. **Valley trails** (`wild_trail_{i}`): from each arterial's end up its
//!    valley to the valley's head, about [`HEAD_RADIUS`] out.
//! 2. **Pass trails** (`wild_pass_{i}`): from each valley's head over the
//!    massif beside it to the next valley's trail, kept high
//!    ([`PASS_MIN_RADIUS`]) so they cross the range rather than skirt its foot.
//!    The valley and pass trails make one network with no dead ends.
//! 3. **Spurs** (`wild_spur_{k}`): from the network to every lake's shore and
//!    every ford it doesn't already pass, and **climbs** (`wild_climb_{k}`) up
//!    each massif to the highest ground a trail can reach in the middle of it.
//!    They're the branches of one shortest-path forest, so they share their
//!    ways out.
//!
//! Each path is pulled straight (Douglas-Peucker), rounded (Chaikin) until no
//! turn is sharper than [`MAX_TURN_DEG`], and checked: no point on a lake or
//! on ground at [`WALK_LIMIT`], well under the cliffs W4 stops you at
//! (`ground::CLIFF_SLOPE`). Where a trail crosses a river away from a
//! ford, the trail is the bridge (W4 treats a road over water as a crossing).
//! They're `trail` roads, so every client draws them as they are.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use capitol_protocol::fmath::StableMath;
use capitol_protocol::land::{height_at, slope_at, LandParams};
use capitol_protocol::rules::{
    CENTER_X, CENTER_Y, RURAL_OUTER_RADIUS, WILDERNESS_OUTER_RADIUS, WORLD_SIZE,
};
use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot};

use super::hydrology::Hydrology;
use super::street_plan;

type P = (f64, f64);

/// The grid's cell, in world units.
pub const CELL: f64 = 25.0;
/// The steepest ground a trail is laid over. Well under [`WALK_LIMIT`], so
/// rounding the path off its cells can't take it over.
pub const TRAIL_SLOPE: f64 = 0.75;
/// Steeper ground, up to this, a trail takes only where there's no other
/// way, at [`STEEP_COST`] times the cost: the way up to a tarn in a cirque.
pub const STEEP_SLOPE: f64 = 0.95;
const STEEP_COST: f64 = 20.0;
/// A step costs its length times `1 + GRADE_COST * grade²`, where the grade
/// is the step's rise over its run: across a slope is cheap, up it dear.
const GRADE_COST: f64 = 60.0;
/// The cost of each river cell a trail bridges.
const BRIDGE_COST: f64 = 400.0;
/// The cost of each ford cell a trail wades.
const FORD_COST: f64 = 10.0;
/// The cost of each stream cell a trail wades.
const WADE_COST: f64 = 30.0;
/// About where a valley trail ends: its valley's head, 1400 into the wilds
/// (r5200 on the 12000 world, moved out with the rural edge, #308).
pub const HEAD_RADIUS: f64 = RURAL_OUTER_RADIUS + 1400.0;
/// How far either side of [`HEAD_RADIUS`] a valley's head may be.
const HEAD_BAND: f64 = 100.0;
/// How far, in degrees, a valley's head may be off its arterial's bearing.
/// The massifs wander up to about 17° (`LandParams::massif_wander`).
const VALLEY_SPREAD_DEG: f64 = 20.0;
/// A pass trail keeps out past here, so it crosses its massif high. If the
/// massif has no way over, [`PASS_FALLBACK_RADII`] are tried in turn.
pub const PASS_MIN_RADIUS: f64 = RURAL_OUTER_RADIUS + 900.0;
const PASS_FALLBACK_RADII: [f64; 2] = [RURAL_OUTER_RADIUS + 500.0, RURAL_OUTER_RADIUS + 200.0];
/// How far, in degrees, a pass trail may stray past the bearings of the two
/// valleys it joins.
const PASS_ARC_MARGIN_DEG: f64 = 10.0;
/// Trails keep this far in from the world's edge.
const EDGE_MARGIN: f64 = 60.0;
/// How far the straightened path may stray from its cells, tried in turn
/// until the trail passes its checks.
const STRAIGHTEN: [f64; 4] = [20.0, 10.0, 5.0, 0.0];
/// How many times a trail's sharp corners are cut before a straightening
/// is given up.
const ROUNDING: usize = 10;
/// The sharpest turn a trail makes, in degrees off straight ahead: the
/// street plan's 45°, less a margin.
pub const MAX_TURN_DEG: f64 = 40.0;
/// A trail's vertices are no further apart than this, so another trail can
/// join it at a vertex near wherever it meets it.
const VERTEX_SPACING: f64 = 20.0;
/// A lake's spur ends within this of its shore.
pub const SHORE_REACH: f64 = 60.0;
/// A tarn no trail can reach the shore of gets one this near it, if any.
pub const TARN_REACH: f64 = 800.0;
/// A spur shorter than this is a step off the trail, which anyone can walk:
/// it isn't laid. A one-cell stub that the weld then ends on a crossing
/// turned through 77° (#290, land seed 7 under the farms).
const SPUR_MIN: f64 = 1.5 * CELL;
/// A spur that can't be made a trail all the way is cut back this many
/// cells at a time until it can.
const TRIM_CELLS: usize = 4;
/// Lakes and fords this near the city are on the open foot of the hills,
/// where anyone can walk straight to them: they get no spur.
pub const OPEN_GROUND: f64 = RURAL_OUTER_RADIUS + 200.0;
/// A ford with a trail vertex this near needs no spur.
const FORD_REACH: f64 = 80.0;
/// A massif's climb goes to the highest ground in the middle of it: past
/// this share of the way round from either valley.
const CLIMB_FROM: f64 = 0.25;
/// Spurs leave the network no nearer the rural edge than this past it.
const SPUR_FROM: f64 = 100.0;
/// How far round a road already laid the trails keep off.
const ROAD_CLEAR: f64 = 15.0;
/// No point of a trail is this steep (`land::slope_at`, rise over run).
/// W4's `ground::CLIFF_SLOPE` (1.2) is where a walk stops; a trail keeps
/// well under it, so it's an easy way up, not a scramble.
pub const WALK_LIMIT: f64 = 0.98;
/// How finely a trail is checked for slope and lakes.
const CHECK_STEP: f64 = 2.0;

fn radius(x: f64, y: f64) -> f64 {
    ((x - CENTER_X) * (x - CENTER_X) + (y - CENTER_Y) * (y - CENTER_Y)).sqrt()
}

fn bearing_deg(x: f64, y: f64) -> f64 {
    (y - CENTER_Y).stable_atan2(x - CENTER_X).to_degrees().rem_euclid(360.0)
}

fn angle_between(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn dist(a: P, b: P) -> f64 {
    ((a.0 - b.0) * (a.0 - b.0) + (a.1 - b.1) * (a.1 - b.1)).sqrt()
}

fn point_segment(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// The wild trails for this land, water and roads. `lots` are kept clear.
pub fn lay(land: &LandParams, water: &Hydrology, roads: &[RoadSegment], lots: &[TerrainPlot], width: f64) -> Vec<RoadSegment> {
    // The arterials that run out to the rural edge, by bearing.
    let mut starts: Vec<P> = roads
        .iter()
        .filter(|r| r.road_type == RoadType::Arterial && r.id.starts_with("arterial_"))
        .filter_map(|r| r.points.last().copied())
        .filter(|&(x, y)| radius(x, y) >= RURAL_OUTER_RADIUS - 1.0)
        .collect();
    starts.sort_by(|a, b| bearing_deg(a.0, a.1).total_cmp(&bearing_deg(b.0, b.1)));

    let grid = Grid::new(land, water, roads, lots, &starts);
    let mut trails: Vec<RoadSegment> = Vec::new();
    let road = |id: String, points: Vec<P>| RoadSegment { id, road_type: RoadType::Trail, points, width };

    // 1. Up the valleys.
    let mut valleys: Vec<Option<usize>> = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let bearing = bearing_deg(start.0, start.1);
        let from = grid.cell_of(start);
        let target = |c: usize| {
            let (x, y) = grid.centre(c);
            (radius(x, y) - HEAD_RADIUS).abs() <= HEAD_BAND && angle_between(bearing_deg(x, y), bearing) <= VALLEY_SPREAD_DEG
        };
        let found = grid.search(&[from], |_| true, target).and_then(|cells| {
            let mut raw = vec![start];
            raw.extend(cells[1..].iter().map(|&c| grid.centre(c)));
            grid.shape(&raw)
        });
        valleys.push(found.map(|points| {
            trails.push(road(format!("wild_trail_{i}"), points));
            trails.len() - 1
        }));
    }

    // 2. Over the massifs, from each valley's head to the next valley's trail.
    let n = starts.len();
    let bearings: Vec<f64> = starts.iter().map(|&(x, y)| bearing_deg(x, y)).collect();
    for i in 0..n {
        let (Some(from), Some(to)) = (valleys[i], valleys[(i + 1) % n]) else { continue };
        if n < 2 {
            break;
        }
        let head = *trails[from].points.last().expect("a trail has points");
        let mut laid = None;
        for min_r in std::iter::once(PASS_MIN_RADIUS).chain(PASS_FALLBACK_RADII) {
            let joins = grid.vertex_cells(&trails[to].points, min_r);
            // Over this massif, not round the world the other way.
            let (a, b) = (bearings[i] - PASS_ARC_MARGIN_DEG, bearings[(i + 1) % n] + PASS_ARC_MARGIN_DEG);
            let span = (b - a).rem_euclid(360.0);
            let allowed = |c: usize| {
                let (x, y) = grid.centre(c);
                radius(x, y) >= min_r && (bearing_deg(x, y) - a).rem_euclid(360.0) <= span
            };
            let found = grid.search(&[grid.cell_of(head)], allowed, |c| joins.contains_key(&c)).and_then(|cells| {
                let last = *cells.last().expect("a path has cells");
                let mut raw = vec![head];
                raw.extend(cells[1..cells.len() - 1].iter().map(|&c| grid.centre(c)));
                raw.push(joins[&last]);
                grid.shape(&raw)
            });
            if found.is_some() {
                laid = found;
                break;
            }
        }
        if let Some(points) = laid {
            trails.push(road(format!("wild_pass_{i}"), points));
        }
    }

    // 3. Spurs to the lakes and fords the network doesn't reach.
    trails.extend(spurs(&grid, water, &trails, &bearings, width));

    // Any crossing between trails becomes a shared vertex. Only the roads
    // that reach the rural edge can meet them, and those keep their vertices:
    // every trail that meets one starts on one of its ends.
    let edge: Vec<RoadSegment> =
        roads.iter().filter(|r| r.points.iter().any(|p| radius(p.0, p.1) >= RURAL_OUTER_RADIUS - CELL)).cloned().collect();
    let base = edge.len();
    let mut all: Vec<RoadSegment> = edge.into_iter().chain(trails).collect();
    street_plan::weld_junctions(&mut all, |r| r.id.starts_with("wild_"));
    all.split_off(base)
}

/// The spurs: one shortest-path forest grown from the network's vertices,
/// with a branch to each lake and ford it doesn't already reach, cheapest
/// first. A branch that meets one laid before it joins it there.
fn spurs(grid: &Grid, water: &Hydrology, network: &[RoadSegment], bearings: &[f64], width: f64) -> Vec<RoadSegment> {
    // Which road owns each cell: the network's vertices, then each spur's.
    let mut owned_by: HashMap<usize, usize> = HashMap::new();
    let mut roads: Vec<Vec<P>> = network.iter().map(|r| r.points.clone()).collect();
    for (k, r) in network.iter().enumerate() {
        // Not from right by the arterials' ends, where a spur would run
        // beside the arterial.
        for c in grid.vertex_cells(&r.points, RURAL_OUTER_RADIUS + SPUR_FROM).into_keys() {
            owned_by.entry(c).or_insert(k);
        }
    }
    let sources: Vec<usize> = {
        let mut s: Vec<usize> = owned_by.keys().copied().collect();
        s.sort_unstable();
        s
    };
    let forest = grid.forest(&sources);

    // Each destination: its cheapest cell, and where the spur ends. A lake
    // with a trail vertex on its shore already has its trail.
    let mut wanted: Vec<(f64, usize, P, Goal)> = Vec::new();
    for (k, shore) in grid.shores.iter().enumerate() {
        let near_plain = grid.lakes[k].2.iter().flatten().any(|p| radius(p.0, p.1) < OPEN_GROUND);
        if near_plain || shore.iter().any(|c| owned_by.contains_key(c)) {
            continue;
        }
        let best = shore
            .iter()
            .copied()
            .filter(|&c| forest.cost[c].is_finite())
            .min_by(|&a, &b| forest.cost[a].total_cmp(&forest.cost[b]).then(a.cmp(&b)))
            // A tarn walled in by steep ground: as near as a trail gets.
            .or_else(|| grid.nearest_reachable(k, &forest));
        if let Some(best) = best {
            wanted.push((forest.cost[best], best, grid.centre(best), Goal::Lake));
        }
    }
    for ford in &water.fords {
        let at = (ford.x, ford.y);
        if radius(at.0, at.1) < OPEN_GROUND || near_any(&roads, &[at], FORD_REACH) {
            continue;
        }
        let c = grid.cell_of(at);
        if forest.cost[c].is_finite() {
            wanted.push((forest.cost[c], c, at, Goal::Ford));
        }
    }
    // A climb into each massif, to the highest ground a trail can reach in
    // the middle of it.
    let mut tops: Vec<Option<usize>> = vec![None; bearings.len()];
    for c in (0..grid.n * grid.n).filter(|&c| forest.cost[c].is_finite() && grid.slope[c] <= TRAIL_SLOPE) {
        let (x, y) = grid.centre(c);
        let bearing = bearing_deg(x, y);
        for (k, top) in tops.iter_mut().enumerate() {
            let (a, b) = (bearings[k], bearings[(k + 1) % bearings.len()]);
            let span = (b - a).rem_euclid(360.0);
            let along = (bearing - a).rem_euclid(360.0);
            let middle = along > CLIMB_FROM * span && along < (1.0 - CLIMB_FROM) * span;
            if middle && top.is_none_or(|t| grid.height[c] > grid.height[t]) {
                *top = Some(c);
            }
        }
    }
    for top in tops.into_iter().flatten() {
        wanted.push((forest.cost[top], top, grid.centre(top), Goal::Climb));
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut out = Vec::new();
    let mut climbs = 0;
    for (_, dest, end, goal) in wanted {
        // Back down the forest to the network, or to a spur laid before.
        let mut cells = vec![dest];
        let mut c = dest;
        while !owned_by.contains_key(&c) {
            match forest.parent[c] {
                Some(p) => c = p,
                None => break,
            }
            cells.push(c);
        }
        let Some(&k) = owned_by.get(&c) else { continue };
        let join = nearest_vertex(&roads[k], grid.centre(c));
        cells.reverse();
        // A spur to a lake or a top that can't be made a trail all the way
        // stops short. One to a ford goes all the way or not at all.
        let mut points = None;
        let mut keep = cells.len();
        let shortest = if goal == Goal::Ford { cells.len().max(2) } else { 2 };
        while keep >= shortest && points.is_none() {
            let stop = if keep == cells.len() { end } else { grid.centre(cells[keep - 1]) };
            let mut raw = vec![join];
            raw.extend(cells[1..keep - 1].iter().map(|&c| grid.centre(c)));
            raw.push(stop);
            if dist(join, stop) < SPUR_MIN {
                break;
            }
            points = grid.shape(&raw);
            keep = keep.saturating_sub(TRIM_CELLS);
        }
        let Some(points) = points else { continue };
        let id = roads.len();
        for &c in &cells {
            owned_by.entry(c).or_insert(id);
        }
        for c in grid.vertex_cells(&points, 0.0).into_keys() {
            owned_by.entry(c).or_insert(id);
        }
        roads.push(points.clone());
        let id = if goal == Goal::Climb {
            climbs += 1;
            format!("wild_climb_{}", climbs - 1)
        } else {
            format!("wild_spur_{}", out.len() - climbs)
        };
        out.push(RoadSegment { id, road_type: RoadType::Trail, points, width });
    }
    out
}

/// Whether any of `points` is within `reach` of a vertex of `roads`.
fn near_any(roads: &[Vec<P>], points: &[P], reach: f64) -> bool {
    roads.iter().flatten().any(|&v| points.iter().any(|&p| dist(p, v) <= reach))
}

fn nearest_vertex(points: &[P], p: P) -> P {
    *points.iter().min_by(|a, b| dist(**a, p).total_cmp(&dist(**b, p))).expect("a road has points")
}

/// What a spur goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Goal {
    Lake,
    Ford,
    Climb,
}

/// What a cell holds, for a trail's cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wet {
    Dry,
    Stream,
    Ford,
    River,
}

struct Grid {
    n: usize,
    land: LandParams,
    /// The ground's slope at each cell's centre; infinite where no trail may go.
    slope: Vec<f64>,
    /// The ground's height at each cell's centre.
    height: Vec<f64>,
    wet: Vec<Wet>,
    /// Each lake's cells within [`SHORE_REACH`] of its shore, on dry land.
    shores: Vec<Vec<usize>>,
    /// The lakes, fattened by a cell, for the checks.
    lakes: Vec<(P, P, Vec<Vec<P>>)>,
}

struct Forest {
    cost: Vec<f64>,
    parent: Vec<Option<usize>>,
}

/// An entry in the search's queue: least cost first, then lowest cell.
#[derive(PartialEq)]
struct Queued(f64, usize);

impl Eq for Queued {}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const NEIGHBOURS: [(i64, i64); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];

impl Grid {
    fn new(land: &LandParams, water: &Hydrology, roads: &[RoadSegment], lots: &[TerrainPlot], starts: &[P]) -> Self {
        let n = (WORLD_SIZE / CELL).ceil() as usize;
        let mut grid = Grid {
            n,
            land: land.clone(),
            slope: vec![f64::INFINITY; n * n],
            height: vec![0.0; n * n],
            wet: vec![Wet::Dry; n * n],
            shores: Vec::new(),
            lakes: Vec::new(),
        };
        // The height on a lattice of half cells, and the slope at each cell's
        // centre, corners and edges from it: the ridges' crests are sharp, and
        // a cell gentle at its centre can hold a cliff's edge. One height a
        // lattice point is far cheaper than `slope_at` at each of nine.
        let in_ring = |x: f64, y: f64, pad: f64| {
            let r = radius(x, y);
            (RURAL_OUTER_RADIUS - CELL - pad..=WILDERNESS_OUTER_RADIUS - EDGE_MARGIN + pad).contains(&r)
        };
        let half = CELL / 2.0;
        let m = 2 * n + 1;
        let lattice: Vec<f64> = (0..m * m)
            .map(|k| {
                let (x, y) = ((k % m) as f64 * half, (k / m) as f64 * half);
                if in_ring(x, y, 2.0 * CELL) { height_at(x, y, land) } else { 0.0 }
            })
            .collect();
        let slope = |a: usize, b: usize| {
            let at = |a: usize, b: usize| lattice[b.min(m - 1) * m + a.min(m - 1)];
            let gx = (at(a + 1, b) - at(a.saturating_sub(1), b)) / CELL;
            let gy = (at(a, b + 1) - at(a, b.saturating_sub(1))) / CELL;
            (gx * gx + gy * gy).sqrt()
        };
        for j in 0..n {
            for i in 0..n {
                let (x, y) = grid.centre(j * n + i);
                if !in_ring(x, y, 0.0) {
                    continue;
                }
                let (a, b) = (2 * i + 1, 2 * j + 1);
                grid.height[j * n + i] = lattice[b * m + a];
                let s = (0..3).flat_map(|dj| (0..3).map(move |di| (a + di - 1, b + dj - 1)))
                    .map(|(a, b)| slope(a, b))
                    .fold(0.0, f64::max);
                if s <= STEEP_SLOPE {
                    grid.slope[j * n + i] = s;
                }
            }
        }
        // Keep off the roads and lots already there. Every road ends at the
        // rural edge, so past it the ground round their ends stays open: the
        // valley trails start from the arterials' ends.
        let start_cells: Vec<usize> = starts.iter().map(|&p| grid.cell_of(p)).collect();
        for road in roads {
            for w in road.points.windows(2) {
                let (a, b) = (w[0], w[1]);
                if radius(a.0, a.1).max(radius(b.0, b.1)) < RURAL_OUTER_RADIUS - 2.0 * CELL {
                    continue;
                }
                let reach = road.width / 2.0 + ROAD_CLEAR;
                grid.each_cell((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)), reach, |g, c| {
                    let (x, y) = g.centre(c);
                    if radius(x, y) < RURAL_OUTER_RADIUS && point_segment((x, y), a, b) <= reach && !start_cells.contains(&c) {
                        g.slope[c] = f64::INFINITY;
                    }
                });
            }
        }
        for lot in lots {
            let c = &lot.corners;
            if c.iter().all(|p| radius(p.0, p.1) < RURAL_OUTER_RADIUS - 2.0 * CELL) {
                continue;
            }
            let min = c.iter().fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
            let max = c.iter().fold((f64::MIN, f64::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
            grid.each_cell(min, max, CELL, |g, k| {
                let (x, y) = g.centre(k);
                let near = (0..4).any(|e| point_segment((x, y), c[e], c[(e + 1) % 4]) <= CELL);
                if (near || capitol_protocol::terrain::quad_contains(c, x, y)) && !start_cells.contains(&k) {
                    g.slope[k] = f64::INFINITY;
                }
            });
        }
        // The water: streams, then rivers over them, then the fords.
        for (list, kind) in [(&water.streams, Wet::Stream), (&water.rivers, Wet::River)] {
            for course in list {
                for w in course.points.windows(2) {
                    let (a, b) = ((w[0].x, w[0].y), (w[1].x, w[1].y));
                    let reach = 0.5 * w[0].width.max(w[1].width) + 0.75 * CELL;
                    grid.each_cell((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)), reach, |g, c| {
                        if point_segment(g.centre(c), a, b) <= reach && g.wet[c] != Wet::River {
                            g.wet[c] = kind;
                        }
                    });
                }
            }
        }
        for ford in &water.fords {
            let reach = ford.width / 2.0;
            grid.each_cell((ford.x, ford.y), (ford.x, ford.y), reach, |g, c| {
                if dist(g.centre(c), (ford.x, ford.y)) <= reach && g.wet[c] == Wet::River {
                    g.wet[c] = Wet::Ford;
                }
            });
        }
        for lake in &water.lakes {
            let pts = lake.rings.iter().flatten();
            let min = pts.clone().fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
            let max = pts.fold((f64::MIN, f64::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
            let mut shore = Vec::new();
            grid.each_cell(min, max, SHORE_REACH + CELL, |g, c| {
                let (x, y) = g.centre(c);
                let d = shore_distance(&lake.rings, (x, y));
                if lake.contains(x, y) || d <= CELL {
                    g.slope[c] = f64::INFINITY;
                } else if d <= SHORE_REACH {
                    shore.push(c);
                }
            });
            grid.shores.push(shore);
            grid.lakes.push((min, max, lake.rings.clone()));
        }
        // A shore cell walled in by the cliffs is no use.
        let slope = &grid.slope;
        for shore in &mut grid.shores {
            shore.retain(|&c| slope[c].is_finite());
        }
        // The starts are always open.
        for &c in &start_cells {
            if grid.slope[c].is_infinite() {
                grid.slope[c] = 0.0;
            }
        }
        grid
    }

    /// The cell a trail can reach (in `forest`) nearest lake `k`'s shore,
    /// within [`TARN_REACH`] of it.
    fn nearest_reachable(&self, k: usize, forest: &Forest) -> Option<usize> {
        let (min, max, rings) = &self.lakes[k];
        let last = self.n as i64 - 1;
        let at = |v: f64| ((v / CELL).floor() as i64).clamp(0, last) as usize;
        let mut best: Option<(f64, usize)> = None;
        for j in at(min.1 - TARN_REACH)..=at(max.1 + TARN_REACH) {
            for i in at(min.0 - TARN_REACH)..=at(max.0 + TARN_REACH) {
                let c = j * self.n + i;
                if !forest.cost[c].is_finite() {
                    continue;
                }
                let d = shore_distance(rings, self.centre(c));
                if d <= TARN_REACH && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, c));
                }
            }
        }
        best.map(|(_, c)| c)
    }

    fn centre(&self, c: usize) -> P {
        (((c % self.n) as f64 + 0.5) * CELL, ((c / self.n) as f64 + 0.5) * CELL)
    }

    fn cell_of(&self, p: P) -> usize {
        let last = self.n as i64 - 1;
        let i = ((p.0 / CELL).floor() as i64).clamp(0, last) as usize;
        let j = ((p.1 / CELL).floor() as i64).clamp(0, last) as usize;
        j * self.n + i
    }

    fn each_cell(&mut self, min: P, max: P, pad: f64, mut f: impl FnMut(&mut Grid, usize)) {
        let last = self.n as i64 - 1;
        let at = |v: f64| ((v / CELL).floor() as i64).clamp(0, last) as usize;
        for j in at(min.1 - pad)..=at(max.1 + pad) {
            for i in at(min.0 - pad)..=at(max.0 + pad) {
                f(self, j * self.n + i);
            }
        }
    }

    /// The cells holding `points`' vertices (past `min_r`), each with the
    /// first vertex in it.
    fn vertex_cells(&self, points: &[P], min_r: f64) -> HashMap<usize, P> {
        let mut out = HashMap::new();
        for &p in points {
            if radius(p.0, p.1) >= min_r {
                out.entry(self.cell_of(p)).or_insert(p);
            }
        }
        out
    }

    /// The cost of a step from `a` into `b`, or None if it can't be taken.
    fn step(&self, a: usize, (di, dj): (i64, i64)) -> Option<(usize, f64)> {
        let (i, j) = ((a % self.n) as i64 + di, (a / self.n) as i64 + dj);
        let n = self.n as i64;
        if !(0..n).contains(&i) || !(0..n).contains(&j) {
            return None;
        }
        let b = (j * n + i) as usize;
        let s = self.slope[b];
        if s.is_infinite() {
            return None;
        }
        // No cutting a corner of ground a trail can't take.
        if di != 0 && dj != 0 {
            let (ai, aj) = ((a % self.n) as i64, (a / self.n) as i64);
            if self.slope[(aj * n + i) as usize].is_infinite() || self.slope[(j * n + ai) as usize].is_infinite() {
                return None;
            }
        }
        let len = if di != 0 && dj != 0 { CELL * std::f64::consts::SQRT_2 } else { CELL };
        let wet = match self.wet[b] {
            Wet::Dry => 0.0,
            Wet::Stream => WADE_COST,
            Wet::Ford => FORD_COST,
            Wet::River => BRIDGE_COST,
        };
        let grade = (self.height[b] - self.height[a]) / len;
        let steep = if s > TRAIL_SLOPE { STEEP_COST } else { 1.0 };
        Some((b, steep * len * (1.0 + GRADE_COST * grade * grade) + wet))
    }

    /// The cheapest way from any of `from` to the first cell `target` takes,
    /// through cells `allowed` takes, as the cells along it.
    fn search(&self, from: &[usize], allowed: impl Fn(usize) -> bool, target: impl Fn(usize) -> bool) -> Option<Vec<usize>> {
        let mut cost = vec![f64::INFINITY; self.n * self.n];
        let mut parent: Vec<Option<usize>> = vec![None; self.n * self.n];
        let mut queue = BinaryHeap::new();
        for &c in from {
            cost[c] = 0.0;
            queue.push(Queued(0.0, c));
        }
        while let Some(Queued(d, c)) = queue.pop() {
            if d > cost[c] {
                continue;
            }
            if target(c) && !from.contains(&c) {
                let mut path = vec![c];
                let mut at = c;
                while let Some(p) = parent[at] {
                    path.push(p);
                    at = p;
                }
                path.reverse();
                return Some(path);
            }
            for &nb in &NEIGHBOURS {
                let Some((b, step)) = self.step(c, nb) else { continue };
                if !allowed(b) {
                    continue;
                }
                let nd = d + step;
                if nd < cost[b] {
                    cost[b] = nd;
                    parent[b] = Some(c);
                    queue.push(Queued(nd, b));
                }
            }
        }
        None
    }

    /// The cheapest way from `from` to every cell.
    fn forest(&self, from: &[usize]) -> Forest {
        let mut cost = vec![f64::INFINITY; self.n * self.n];
        let mut parent: Vec<Option<usize>> = vec![None; self.n * self.n];
        let mut queue = BinaryHeap::new();
        for &c in from {
            cost[c] = 0.0;
            queue.push(Queued(0.0, c));
        }
        while let Some(Queued(d, c)) = queue.pop() {
            if d > cost[c] {
                continue;
            }
            for &nb in &NEIGHBOURS {
                let Some((b, step)) = self.step(c, nb) else { continue };
                let nd = d + step;
                if nd < cost[b] {
                    cost[b] = nd;
                    parent[b] = Some(c);
                    queue.push(Queued(nd, b));
                }
            }
        }
        Forest { cost, parent }
    }

    /// A path through cell centres made a trail: straightened, rounded, and
    /// checked (no turn too sharp, no loop over itself, nowhere too steep or
    /// wet), with both ends kept exactly. None if no straightening passes.
    fn shape(&self, raw: &[P]) -> Option<Vec<P>> {
        let raw = street_plan::dedupe(raw);
        if raw.len() < 2 {
            return None;
        }
        STRAIGHTEN.iter().find_map(|&tolerance| {
            let mut p = simplify(&raw, tolerance);
            for _ in 0..ROUNDING {
                if sharpest_turn(&p) <= MAX_TURN_DEG {
                    break;
                }
                p = round_corners(&p, MAX_TURN_DEG);
            }
            let p = densify(&street_plan::dedupe(&p), VERTEX_SPACING);
            (sharpest_turn(&p) <= MAX_TURN_DEG && !street_plan::self_crosses(&p) && self.walkable(&p)).then_some(p)
        })
    }

    /// No point along `points` on a lake or on ground at the cliff slope.
    fn walkable(&self, points: &[P]) -> bool {
        points.windows(2).all(|w| {
            let steps = (dist(w[0], w[1]) / CHECK_STEP).ceil().max(1.0) as usize;
            (0..=steps).all(|k| {
                let t = k as f64 / steps as f64;
                let (x, y) = (w[0].0 + t * (w[1].0 - w[0].0), w[0].1 + t * (w[1].1 - w[0].1));
                slope_at(x, y, &self.land) < WALK_LIMIT
                    && !self.lakes.iter().any(|(min, max, rings)| {
                        x >= min.0 && x <= max.0 && y >= min.1 && y <= max.1 && rings_contain(rings, x, y)
                    })
            })
        })
    }
}

/// How far `p` is from the nearest edge of `rings`.
fn shore_distance(rings: &[Vec<P>], p: P) -> f64 {
    let mut best = f64::INFINITY;
    for ring in rings {
        for i in 0..ring.len() {
            best = best.min(point_segment(p, ring[i], ring[(i + 1) % ring.len()]));
        }
    }
    best
}

fn rings_contain(rings: &[Vec<P>], x: f64, y: f64) -> bool {
    let mut inside = false;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
                inside = !inside;
            }
        }
    }
    inside
}

/// Douglas-Peucker: the fewest of `points` that keep within `tolerance` of
/// them all. Both ends stay.
fn simplify(points: &[P], tolerance: f64) -> Vec<P> {
    if points.len() < 3 || tolerance <= 0.0 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (mut far, mut at) = (0.0, a);
        for k in a + 1..b {
            let d = point_segment(points[k], points[a], points[b]);
            if d > far {
                (far, at) = (d, k);
            }
        }
        if far > tolerance {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Chaikin's corner cutting at the corners sharper than `limit` degrees
/// only: each becomes two points a quarter of the way along its two
/// segments, which halves its turn. The ends stay.
fn round_corners(points: &[P], limit: f64) -> Vec<P> {
    let mut out = vec![points[0]];
    for k in 1..points.len().saturating_sub(1) {
        let (a, v, b) = (points[k - 1], points[k], points[k + 1]);
        if sharpest_turn(&[a, v, b]) > limit {
            out.push((0.75 * v.0 + 0.25 * a.0, 0.75 * v.1 + 0.25 * a.1));
            out.push((0.75 * v.0 + 0.25 * b.0, 0.75 * v.1 + 0.25 * b.1));
        } else {
            out.push(v);
        }
    }
    if points.len() > 1 {
        out.push(points[points.len() - 1]);
    }
    out
}

/// One round of Chaikin's corner cutting, keeping both ends.
#[cfg(test)]
fn chaikin(points: &[P]) -> Vec<P> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for (k, w) in points.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let q = (0.75 * a.0 + 0.25 * b.0, 0.75 * a.1 + 0.25 * b.1);
        let r = (0.25 * a.0 + 0.75 * b.0, 0.25 * a.1 + 0.75 * b.1);
        if k > 0 {
            out.push(q);
        }
        if k < points.len() - 2 {
            out.push(r);
        }
    }
    out.push(points[points.len() - 1]);
    out
}

/// `points` with a vertex put in every `spacing` along any longer segment.
fn densify(points: &[P], spacing: f64) -> Vec<P> {
    let mut out = Vec::with_capacity(points.len());
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let steps = (dist(a, b) / spacing).ceil().max(1.0) as usize;
        for k in 0..steps {
            let t = k as f64 / steps as f64;
            out.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    if let Some(&last) = points.last() {
        out.push(last);
    }
    out
}

/// The sharpest turn along `points`, in degrees off straight ahead.
fn sharpest_turn(points: &[P]) -> f64 {
    points
        .windows(3)
        .map(|w| {
            let (u, v) = ((w[1].0 - w[0].0, w[1].1 - w[0].1), (w[2].0 - w[1].0, w[2].1 - w[1].1));
            (u.0 * v.1 - u.1 * v.0).stable_atan2(u.0 * v.0 + u.1 * v.1).abs().to_degrees()
        })
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests;
