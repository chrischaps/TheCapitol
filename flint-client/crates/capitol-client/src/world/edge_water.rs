//! The wilds' water past the world's edge (#229). W3's hydrology ends every
//! `mouth: edge` course on r6000, but the land runs on past it and settles
//! to flat by `land::OUTER` (W9). So the client runs each one on, as
//! scenery nobody can walk to, before the land is carved:
//!
//! - **Where it runs.** A priority-flood comes in over the [`settled`]
//!   land from a ring short of the flat ([`SINK`]), as W3's does from the
//!   edge, so every cell drains out over the lowest pass it has. A course
//!   leaves its mouth straight out, then follows that drainage.
//! - **Where they meet.** The widest go first. A course that reaches one
//!   already run on joins it there as a tributary.
//! - **Where they end.** Each trunk ends in a pond on the nearly flat
//!   ground by the ring, or in a neighbour's pond if it's close.
//! - **In their valleys.** The surface never rises downstream and keeps
//!   under the ground at both banks. Where the drainage has to cross a
//!   rise, the land falls to the water in a valley ([`Valleys`]) rather
//!   than a slot, and `land::carve` cuts the bed into its floor as it does
//!   the rest.
//! - **Off the world.** A course leaves its mouth straight out, then turns
//!   on a smooth curve. Past that first stretch every point keeps far
//!   enough out that its carve never reaches the world, and the valleys
//!   start at the edge, so the land inside it is still the server's.
//!
//! [`settled`]: crate::world::land::settled

use crate::world::land::{settled, OUTER};
use capitol_protocol::land::LandParams;
use capitol_protocol::rules::{CENTER_X, CENTER_Y, WILDERNESS_OUTER_RADIUS};
use capitol_protocol::terrain::{Lake, Mouth, WaterCourse, WaterPoint};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// The flood's grid (server units).
const CELL: f64 = 20.0;
/// The ring the courses drain to: short of the flat, so a pond and its
/// carve are in before [`OUTER`].
const SINK: f64 = OUTER - 250.0;
/// The flood's inner rim: past the edge by the widest course's carve.
const INNER: f64 = WILDERNESS_OUTER_RADIUS + 70.0;
/// How far a course's carve reaches past its bank (`land::COURSE_REACH`),
/// and a little more.
const REACH: f64 = 32.0;
/// How far (server units) a surface keeps under the ground at its banks.
const UNDER: f64 = 0.5;
/// Smoothing passes over a course's path, which comes off the grid in
/// steps of 45°.
const SMOOTHING: usize = 6;
/// How far along its path a course takes to turn from straight out of its
/// mouth onto the drainage: this many times its width, within limits.
const TURN_PER_WIDTH: f64 = 4.0;
const TURN_MIN: f64 = 80.0;
const TURN_MAX: f64 = 240.0;
/// The tightest a course may bend: its bends' radius in half-widths. A
/// ribbon bent tighter than its half-width folds over on the inside.
const BEND: f64 = 1.5;
/// The most passes that ease the tight bends out.
const EASING: usize = 400;
/// How steeply a valley's sides rise from the banks (rise over run), and
/// how far past the banks they reach before they're gone.
const VALLEY_SLOPE: f64 = 0.45;
const VALLEY_REACH: f64 = 300.0;
/// How far past the world's edge the valleys take to come in.
const VALLEY_IN: f64 = 60.0;
/// The valleys' index buckets (server units).
const VALLEY_BUCKET: f64 = 150.0;
/// A pond's radius: this many times its course's width, within limits.
const POND_PER_WIDTH: f64 = 2.5;
const POND_MIN: f64 = 35.0;
const POND_MAX: f64 = 140.0;
/// Points round a pond's shore.
const POND_POINTS: usize = 28;
/// Ponds closer than their radii and this share the nearer.
const POND_GAP: f64 = 20.0;

/// Runs every course in `rivers` and `streams` that ends at the edge on
/// down the land past it, adding the ponds they end in to `lakes`. The
/// rest are left as they are. The land's heights are sampled on `threads`.
pub fn run_on(
    rivers: &mut [WaterCourse],
    streams: &mut [WaterCourse],
    lakes: &mut Vec<Lake>,
    params: &LandParams,
    threads: usize,
) {
    let mut edge: Vec<&mut WaterCourse> = rivers
        .iter_mut()
        .chain(streams.iter_mut())
        .filter(|c| c.mouth == Mouth::Edge && c.points.len() >= 2)
        .collect();
    if edge.is_empty() {
        return;
    }
    // The widest claim the drainage first, so the others join them.
    edge.sort_by(|a, b| width(b).total_cmp(&width(a)).then(a.id.cmp(&b.id)));
    let ids: Vec<String> = edge.iter().map(|c| c.id.clone()).collect();
    let grid = Grid::flood(params, threads);
    let mut claims: Vec<Option<(usize, usize)>> = vec![None; grid.n * grid.n];
    // Each course's points past its mouth, once run on.
    let mut runs: Vec<Vec<WaterPoint>> = Vec::new();
    let mut ponds: Vec<(usize, [f64; 2], f64)> = Vec::new();
    // The valleys of the courses run on so far: each course's water keeps
    // under the ground as they leave it.
    let mut valleys = Valleys::default();
    for (k, course) in edge.iter_mut().enumerate() {
        let mouth = *course.points.last().expect("two points");
        let half = 0.5 * mouth.width;
        let clear = WILDERNESS_OUTER_RADIUS + half + REACH;
        // Straight out from the mouth, to where the drainage takes over.
        let out = unit([mouth.x - CENTER_X, mouth.y - CENTER_Y]);
        let lead_r = INNER.max(clear) + CELL;
        let lead = [CENTER_X + out[0] * lead_r, CENTER_Y + out[1] * lead_r];
        let mut cells = Vec::new();
        let mut joins = None;
        let Some(start) = grid.nearest(lead) else {
            runs.push(Vec::new());
            continue;
        };
        let mut at = Some(start);
        while let Some(c) = at {
            if let Some(claim) = claims[c] {
                joins = Some(claim);
                break;
            }
            cells.push(c);
            at = grid.parent[c];
        }
        let mut path = vec![[mouth.x, mouth.y], lead];
        path.extend(cells.iter().skip(1).map(|&c| grid.at(c)));
        if let Some((j, p)) = joins {
            let to = runs[j][p];
            path.push([to.x, to.y]);
        }
        smooth(&mut path, clear, mouth.width);
        let ground = |x: f64, y: f64| valleys.lower(settled(x, y, params), x, y);
        let mut points = surfaces(&path, mouth, &ground);
        let id_joined = match joins {
            Some((j, p)) => {
                let to = runs[j][p];
                let end = points.last_mut().expect("a mouth and a lead");
                end.surface = end.surface.min(to.surface);
                Mouth::Joins(ids[j].clone())
            }
            None => {
                let end = *points.last().expect("a mouth and a lead");
                let r = (POND_PER_WIDTH * end.width).clamp(POND_MIN, POND_MAX);
                let near = ponds.iter().find(|(_, c, pr)| dist(*c, [end.x, end.y]) < r + pr + POND_GAP);
                match near {
                    Some(&(lake, centre, _)) => {
                        let level = lakes[lake].level;
                        points.push(WaterPoint { x: centre[0], y: centre[1], surface: end.surface.min(level), ..end });
                        Mouth::Lake(lakes[lake].id.clone())
                    }
                    None => {
                        let pond = pond(&course.id, end, r, &ground);
                        ponds.push((lakes.len(), [end.x, end.y], r));
                        let id = pond.id.clone();
                        lakes.push(pond);
                        Mouth::Lake(id)
                    }
                }
            }
        };
        // Claim the drainage it took and the ground under its water, so a
        // later course that reaches either joins it.
        let run: Vec<WaterPoint> = points[1..].to_vec();
        for &c in &cells {
            let [x, y] = grid.at(c);
            let nearest = nearest_point(&run, [x, y]);
            claims[c].get_or_insert((k, nearest));
        }
        for (i, p) in run.iter().enumerate() {
            for c in grid.within([p.x, p.y], 0.5 * p.width + 0.5 * CELL) {
                claims[c].get_or_insert((k, i));
            }
        }
        valleys.add(&points);
        runs.push(run);
        course.points.extend_from_slice(&points[1..]);
        course.mouth = id_joined;
    }
}

fn width(c: &WaterCourse) -> f64 {
    c.points.last().map_or(0.0, |p| p.width)
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn unit([x, y]: [f64; 2]) -> [f64; 2] {
    let l = x.hypot(y).max(1e-9);
    [x / l, y / l]
}

fn nearest_point(run: &[WaterPoint], at: [f64; 2]) -> usize {
    (0..run.len())
        .min_by(|&i, &j| dist([run[i].x, run[i].y], at).total_cmp(&dist([run[j].x, run[j].y], at)))
        .unwrap_or(0)
}

/// Rounds the path's corners, keeping its mouth, its lead and its end;
/// turns it from straight out of the mouth onto the drainage on a curve;
/// and keeps every point past the lead `clear` of the world's centre.
fn smooth(path: &mut Vec<[f64; 2]>, clear: f64, width: f64) {
    let n = path.len();
    if n >= 4 {
        for _ in 0..SMOOTHING {
            let before = path.to_vec();
            for i in 2..n - 1 {
                for k in 0..2 {
                    path[i][k] = 0.25 * before[i - 1][k] + 0.5 * before[i][k] + 0.25 * before[i + 1][k];
                }
            }
        }
    }
    if n >= 3 {
        turn(path, (TURN_PER_WIDTH * width).clamp(TURN_MIN, TURN_MAX));
    }
    keep_clear(path, clear);
    // Ease out any bend still too tight for the course's width.
    let tightest = BEND * 0.5 * width;
    for _ in 0..EASING {
        let mut eased = false;
        for i in 2..path.len() - 1 {
            if bend(path[i - 1], path[i], path[i + 1]) < tightest {
                let (a, b) = (path[i - 1], path[i + 1]);
                path[i] = [0, 1].map(|c| 0.5 * path[i][c] + 0.25 * (a[c] + b[c]));
                eased = true;
            }
        }
        keep_clear(path, clear);
        if !eased {
            break;
        }
    }
}

/// Keeps every point past the lead `clear` of the world's centre.
fn keep_clear(path: &mut [[f64; 2]], clear: f64) {
    for p in path.iter_mut().skip(2) {
        let (dx, dy) = (p[0] - CENTER_X, p[1] - CENTER_Y);
        let r = dx.hypot(dy);
        if r < clear {
            *p = [CENTER_X + dx * clear / r, CENTER_Y + dy * clear / r];
        }
    }
}

/// The radius of the circle through three points: infinite on a line.
fn bend(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let (ab, bc) = (sub(b, a), sub(c, b));
    let twice_area = (ab[0] * bc[1] - ab[1] * bc[0]).abs();
    if twice_area < 1e-9 {
        return f64::INFINITY;
    }
    dist(a, b) * dist(b, c) * dist(a, c) / (2.0 * twice_area)
}

/// Replaces the path from its lead to `along` further on with a Hermite
/// curve that leaves the lead straight on from the mouth and meets the
/// path on its own heading, at points about a cell apart.
fn turn(path: &mut Vec<[f64; 2]>, along: f64) {
    let n = path.len();
    let (mouth, lead) = (path[0], path[1]);
    let mut run = 0.0;
    let mut k = n - 1;
    for i in 2..n {
        run += dist(path[i - 1], path[i]);
        if run >= along {
            k = i;
            break;
        }
    }
    let to = path[k];
    let heading = if k + 1 < n { unit(sub(path[k + 1], path[k - 1])) } else { unit(sub(to, path[k - 1])) };
    let out = unit(sub(lead, mouth));
    // Tangents as long as the stretch, not its chord, so a course that has
    // to turn back along the rim swings out wide enough for its width.
    let reach = run.max(dist(lead, to));
    let steps = (1.5 * reach / CELL).ceil().max(1.0) as usize;
    let curve: Vec<[f64; 2]> = (1..steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            let (t2, t3) = (t * t, t * t * t);
            let (h00, h10, h01, h11) = (2.0 * t3 - 3.0 * t2 + 1.0, t3 - 2.0 * t2 + t, -2.0 * t3 + 3.0 * t2, t3 - t2);
            [0, 1].map(|c| h00 * lead[c] + h10 * reach * out[c] + h01 * to[c] + h11 * reach * heading[c])
        })
        .collect();
    path.splice(2..k, curve);
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

/// The path's points, at the mouth's width and depth, each surface the
/// lowest of the one upstream and the `ground` at its middle and banks
/// less [`UNDER`].
fn surfaces(path: &[[f64; 2]], mouth: WaterPoint, ground: &impl Fn(f64, f64) -> f64) -> Vec<WaterPoint> {
    let half = 0.5 * mouth.width;
    let mut surface = mouth.surface;
    let mut points = vec![mouth];
    for i in 1..path.len() {
        let [x, y] = path[i];
        let next = path.get(i + 1).copied().unwrap_or([2.0 * x - path[i - 1][0], 2.0 * y - path[i - 1][1]]);
        let [dx, dy] = unit([next[0] - path[i - 1][0], next[1] - path[i - 1][1]]);
        let (nx, ny) = (-dy * half, dx * half);
        let low = ground(x, y).min(ground(x + nx, y + ny)).min(ground(x - nx, y - ny));
        surface = surface.min(low - UNDER);
        points.push(WaterPoint { x, y, surface, ..mouth });
    }
    points
}

/// A pond of about radius `r` round the end of course `id`, its shore a
/// little irregular (seeded from the id) and its level under all of it.
fn pond(id: &str, end: WaterPoint, r: f64, ground: &impl Fn(f64, f64) -> f64) -> Lake {
    let seed = id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    let phase = |shift: u32| ((seed >> shift) & 0xffff) as f64 / 65536.0 * std::f64::consts::TAU;
    let (a, b) = (phase(0), phase(16));
    let ring: Vec<(f64, f64)> = (0..POND_POINTS)
        .map(|k| {
            let t = k as f64 / POND_POINTS as f64 * std::f64::consts::TAU;
            let rho = r * (1.0 + 0.12 * (2.0 * t + a).sin() + 0.07 * (3.0 * t + b).sin());
            (end.x + rho * t.cos(), end.y + rho * t.sin())
        })
        .collect();
    let level = ring
        .iter()
        .map(|&(x, y)| ground(x, y))
        .fold(end.surface, f64::min)
        - UNDER;
    let area = (0..ring.len())
        .map(|i| {
            let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
            p.0 * q.1 - q.0 * p.1
        })
        .sum::<f64>()
        .abs()
        / 2.0;
    Lake {
        id: format!("pond_{id}"),
        level,
        rings: vec![ring],
        area,
    }
}

/// The valleys the courses past the world's edge run in. Within
/// [`VALLEY_REACH`] of a course's banks the ground comes down to no higher
/// than its surface plus [`VALLEY_SLOPE`] for every unit past the bank,
/// fading out with distance as the carve does. Only stretches past the edge
/// have one, and they come in over [`VALLEY_IN`] past it, so nothing inside
/// the world is lowered.
#[derive(Debug, Clone, Default)]
pub struct Valleys {
    reaches: Vec<Reach>,
    buckets: HashMap<(i64, i64), Vec<u32>>,
}

/// One stretch of a course past the edge: its ends, and the half-width
/// and surface at each.
#[derive(Debug, Clone)]
struct Reach {
    a: [f64; 2],
    b: [f64; 2],
    half: [f64; 2],
    surface: [f64; 2],
}

impl Valleys {
    /// The valleys of every course's stretches past the edge.
    pub fn new<'a>(courses: impl IntoIterator<Item = &'a WaterCourse>) -> Self {
        let mut valleys = Self::default();
        for course in courses {
            valleys.add(&course.points);
        }
        valleys
    }

    /// Adds the valleys of `points`' stretches past the edge.
    fn add(&mut self, points: &[WaterPoint]) {
        let past = |p: &WaterPoint| (p.x - CENTER_X).hypot(p.y - CENTER_Y) >= WILDERNESS_OUTER_RADIUS - 0.01;
        for w in points.windows(2) {
            let (p, q) = (w[0], w[1]);
            if !past(&p) || !past(&q) {
                continue;
            }
            let k = self.reaches.len() as u32;
            self.reaches.push(Reach {
                a: [p.x, p.y],
                b: [q.x, q.y],
                half: [0.5 * p.width, 0.5 * q.width],
                surface: [p.surface, q.surface],
            });
            let pad = 0.5 * p.width.max(q.width) + VALLEY_REACH;
            let cell = |v: f64| (v / VALLEY_BUCKET).floor() as i64;
            for j in cell(p.y.min(q.y) - pad)..=cell(p.y.max(q.y) + pad) {
                for i in cell(p.x.min(q.x) - pad)..=cell(p.x.max(q.x) + pad) {
                    self.buckets.entry((i, j)).or_default().push(k);
                }
            }
        }
    }

    /// The ground `h` at server (x, y), lowered into the valleys.
    pub fn lower(&self, h: f64, x: f64, y: f64) -> f64 {
        let r = (x - CENTER_X).hypot(y - CENTER_Y);
        if r <= WILDERNESS_OUTER_RADIUS {
            return h;
        }
        let key = ((x / VALLEY_BUCKET).floor() as i64, (y / VALLEY_BUCKET).floor() as i64);
        let Some(list) = self.buckets.get(&key) else { return h };
        let mut cut: f64 = 0.0;
        for &k in list {
            let s = &self.reaches[k as usize];
            let (t, d) = nearest_on_segment(s.a, s.b, [x, y]);
            let past = (d - (s.half[0] + t * (s.half[1] - s.half[0]))).max(0.0);
            if past >= VALLEY_REACH {
                continue;
            }
            let floor = s.surface[0] + t * (s.surface[1] - s.surface[0]) + past * VALLEY_SLOPE;
            cut = cut.max((h - floor).max(0.0) * (1.0 - smoothstep(0.0, VALLEY_REACH, past)));
        }
        h - cut * smoothstep(WILDERNESS_OUTER_RADIUS, WILDERNESS_OUTER_RADIUS + VALLEY_IN, r)
    }
}

/// Where on segment `ab` is nearest `p`, as (t along it, distance).
fn nearest_on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> (f64, f64) {
    let ab = sub(b, a);
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 { (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (t, dist([a[0] + t * ab[0], a[1] + t * ab[1]], p))
}

fn smoothstep(lo: f64, hi: f64, x: f64) -> f64 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The flood's grid over the ring from [`INNER`] to [`SINK`], square
/// round the centre: each cell's height, and the cell it drains to.
struct Grid {
    n: usize,
    origin: [f64; 2],
    parent: Vec<Option<usize>>,
}

/// A cell on the flood's front, lowest first, then first come.
struct Open {
    level: f64,
    seq: u64,
    at: usize,
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
        other.level.total_cmp(&self.level).then(other.seq.cmp(&self.seq))
    }
}

impl Grid {
    fn flood(params: &LandParams, threads: usize) -> Self {
        let n = (2.0 * SINK / CELL).ceil() as usize + 1;
        let mut grid = Grid {
            n,
            origin: [CENTER_X - SINK, CENTER_Y - SINK],
            parent: vec![None; n * n],
        };
        let heights = grid.heights(params, threads);
        let mut seen = vec![false; n * n];
        let mut open = BinaryHeap::new();
        let mut seq = 0;
        for (c, h) in heights.iter().enumerate() {
            if h.is_some() && grid.radius(c) >= SINK - 1.5 * CELL {
                seen[c] = true;
                open.push(Open { level: h.unwrap_or(0.0), seq, at: c });
                seq += 1;
            }
        }
        while let Some(Open { level, at, .. }) = open.pop() {
            let (i, j) = ((at % n) as i64, (at / n) as i64);
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (ni, nj) = (i + di, j + dj);
                if ni < 0 || nj < 0 || ni >= n as i64 || nj >= n as i64 {
                    continue;
                }
                let c = nj as usize * n + ni as usize;
                let Some(h) = heights[c] else { continue };
                if seen[c] {
                    continue;
                }
                seen[c] = true;
                grid.parent[c] = Some(at);
                open.push(Open { level: h.max(level), seq, at: c });
                seq += 1;
            }
        }
        grid
    }

    /// Each cell's settled height, or `None` off the ring. Rows are shared
    /// out over `threads`.
    fn heights(&self, params: &LandParams, threads: usize) -> Vec<Option<f64>> {
        let n = self.n;
        let mut heights = vec![None; n * n];
        let rows_each = n.div_ceil(threads.max(1));
        std::thread::scope(|scope| {
            for (k, rows) in heights.chunks_mut(rows_each * n).enumerate() {
                scope.spawn(move || {
                    for (dj, row) in rows.chunks_mut(n).enumerate() {
                        for (i, h) in row.iter_mut().enumerate() {
                            let c = (k * rows_each + dj) * n + i;
                            let r = self.radius(c);
                            if (INNER..=SINK).contains(&r) {
                                let [x, y] = self.at(c);
                                *h = Some(settled(x, y, params));
                            }
                        }
                    }
                });
            }
        });
        heights
    }

    fn at(&self, c: usize) -> [f64; 2] {
        let (i, j) = (c % self.n, c / self.n);
        [self.origin[0] + i as f64 * CELL, self.origin[1] + j as f64 * CELL]
    }

    fn radius(&self, c: usize) -> f64 {
        let [x, y] = self.at(c);
        (x - CENTER_X).hypot(y - CENTER_Y)
    }

    /// The cell on the ring nearest `p`, if one is within a cell of it.
    fn nearest(&self, p: [f64; 2]) -> Option<usize> {
        self.within(p, CELL)
            .filter(|&c| (INNER..=SINK).contains(&self.radius(c)))
            .min_by(|&a, &b| dist(self.at(a), p).total_cmp(&dist(self.at(b), p)))
    }

    /// The cells within `r` of `p`.
    fn within(&self, p: [f64; 2], r: f64) -> impl Iterator<Item = usize> + '_ {
        let n = self.n as i64;
        let cell = |v: f64, o: f64| ((v - o) / CELL).floor() as i64;
        let (i0, i1) = (cell(p[0] - r, self.origin[0]).max(0), (cell(p[0] + r, self.origin[0]) + 1).min(n - 1));
        let (j0, j1) = (cell(p[1] - r, self.origin[1]).max(0), (cell(p[1] + r, self.origin[1]) + 1).min(n - 1));
        (j0..=j1)
            .flat_map(move |j| (i0..=i1).map(move |i| (j * n + i) as usize))
            .filter(move |&c| dist(self.at(c), p) <= r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::land::{self, tests::fixture};
    use capitol_protocol::land::Water;
    use std::sync::OnceLock;

    fn polar(r: f64, deg: f64) -> [f64; 2] {
        let (s, c) = deg.to_radians().sin_cos();
        [CENTER_X + r * c, CENTER_Y + r * s]
    }

    fn radius(p: &WaterPoint) -> f64 {
        (p.x - CENTER_X).hypot(p.y - CENTER_Y)
    }

    /// The fixture's water, run on.
    fn run() -> &'static (Vec<WaterCourse>, Vec<WaterCourse>, Vec<Lake>) {
        static R: OnceLock<(Vec<WaterCourse>, Vec<WaterCourse>, Vec<Lake>)> = OnceLock::new();
        R.get_or_init(|| {
            let t = fixture();
            let (mut rivers, mut streams, mut lakes) = (t.rivers.clone(), t.streams.clone(), t.lakes.clone());
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let started = std::time::Instant::now();
            run_on(&mut rivers, &mut streams, &mut lakes, &t.land, threads);
            eprintln!("edge water: run on in {} ms", started.elapsed().as_millis());
            (rivers, streams, lakes)
        })
    }

    fn edge_courses() -> Vec<(&'static WaterCourse, &'static WaterCourse)> {
        let t = fixture();
        let (rivers, streams, _) = run();
        t.rivers
            .iter()
            .zip(rivers)
            .chain(t.streams.iter().zip(streams))
            .filter(|(was, _)| was.mouth == Mouth::Edge)
            .collect()
    }

    #[test]
    fn every_course_off_the_edge_runs_on_to_a_pond_or_another() {
        let (rivers, streams, lakes) = run();
        let all: Vec<&WaterCourse> = rivers.iter().chain(streams).collect();
        let courses = edge_courses();
        assert!(courses.len() > 20, "{}", courses.len());
        let mut ponds = 0;
        for (was, now) in courses {
            assert_eq!(&now.points[..was.points.len()], &was.points[..], "{}: the world's part is as it was", now.id);
            let end = now.points.last().unwrap();
            match &now.mouth {
                Mouth::Lake(id) => {
                    let pond = lakes.iter().find(|l| &l.id == id).unwrap();
                    assert!(id.starts_with("pond_"), "{}: {id}", now.id);
                    assert!(pond.contains(end.x, end.y), "{} ends in {id}", now.id);
                    assert!(radius(end) > SINK - 4.0 * CELL, "{} ends at r{}", now.id, radius(end));
                    ponds += 1;
                }
                Mouth::Joins(id) => {
                    let main = all.iter().find(|c| &c.id == id).unwrap();
                    assert!(main.points.iter().any(|p| p.x == end.x && p.y == end.y), "{} joins {id} at its point", now.id);
                }
                m => panic!("{} still ends {m:?}", now.id),
            }
        }
        assert!(ponds >= 3, "{ponds} ponds");
        let valleys = Valleys::new(rivers.iter().chain(streams));
        for pond in lakes.iter().filter(|l| l.id.starts_with("pond_")) {
            for &(x, y) in &pond.rings[0] {
                let r = (x - CENTER_X).hypot(y - CENTER_Y);
                assert!(r < OUTER - 30.0, "{} reaches r{r}", pond.id);
                let shore = valleys.lower(land::settled(x, y, &fixture().land), x, y);
                assert!(shore >= pond.level, "{}: the shore under the level", pond.id);
            }
        }
    }

    #[test]
    fn each_falls_and_keeps_to_its_bed() {
        let params = &fixture().land;
        let (rivers, streams, lakes) = run();
        let water = Water::new(rivers.iter().chain(streams), lakes);
        let valleys = Valleys::new(rivers.iter().chain(streams));
        let mut checked = 0;
        for (was, now) in edge_courses() {
            let run = &now.points[was.points.len() - 1..];
            for w in run.windows(2) {
                assert!(w[1].surface <= w[0].surface, "{}: the surface rises", now.id);
            }
            for p in &run[1..run.len() - 1] {
                let h = land::height(p.x, p.y, params, &water, &valleys);
                assert!(h <= p.surface - 0.5 * p.depth + 1e-6, "{} at r{}: ground {h} over {}", now.id, radius(p), p.surface);
                checked += 1;
            }
        }
        assert!(checked > 500, "{checked}");
    }

    #[test]
    fn they_run_in_valleys_not_slots() {
        let params = &fixture().land;
        let (rivers, streams, lakes) = run();
        let water = Water::new(rivers.iter().chain(streams), lakes);
        let valleys = Valleys::new(rivers.iter().chain(streams));
        let (mut sides, mut steep, mut worst) = (0, 0, 0.0f64);
        for (was, now) in edge_courses() {
            let run = &now.points[was.points.len()..];
            for w in run.windows(2) {
                let [dx, dy] = unit([w[1].x - w[0].x, w[1].y - w[0].y]);
                let half = 0.5 * w[0].width;
                for side in [-1.0, 1.0] {
                    let at = |past: f64| {
                        let d = side * (half + past);
                        land::height(w[0].x - dy * d, w[0].y + dx * d, params, &water, &valleys)
                    };
                    // The climb from the bank out over the valley's side.
                    let climb = (0..6).map(|k| (at(15.0 * (k + 1) as f64) - at(15.0 * k as f64)) / 15.0).fold(0.0, f64::max);
                    worst = worst.max(climb);
                    sides += 1;
                    if climb > 1.0 {
                        steep += 1;
                    }
                }
            }
        }
        eprintln!("valley sides: {steep} of {sides} climb over 1, the steepest {worst:.2}");
        assert!(sides > 1000, "{sides}");
        assert!(steep * 100 <= sides, "{steep} of {sides} sides climb steeper than 45°");
    }

    #[test]
    fn no_bend_is_too_tight_for_the_water() {
        let mut checked = 0;
        for (was, now) in edge_courses() {
            for w in now.points[was.points.len() - 1..].windows(3) {
                let [a, b, c] = [&w[0], &w[1], &w[2]].map(|p| [p.x, p.y]);
                let r = bend(a, b, c);
                assert!(r >= 0.95 * BEND * 0.5 * w[1].width, "{} bends at radius {r} at ({}, {})", now.id, w[1].x, w[1].y);
                checked += 1;
            }
        }
        assert!(checked > 1000, "{checked}");
    }

    #[test]
    fn the_land_inside_the_world_is_still_the_servers() {
        let t = fixture();
        let (rivers, streams, lakes) = run();
        let before = (Water::new(t.rivers.iter().chain(&t.streams), &t.lakes), Valleys::default());
        let after = (Water::new(rivers.iter().chain(streams), lakes), Valleys::new(rivers.iter().chain(streams)));
        let mouths: Vec<&WaterPoint> = edge_courses().iter().map(|(was, _)| was.points.last().unwrap()).collect();
        for k in 0..7200 {
            let deg = k as f64 * 0.05;
            for r in [5900.0, 5960.0, 5990.0, 5999.5] {
                let [x, y] = polar(r, deg);
                let a = land::height(x, y, &t.land, &before.0, &before.1);
                let b = land::height(x, y, &t.land, &after.0, &after.1);
                // Right at a mouth, the carve of the stretch straight out of
                // it can reach a hair past that of the course's last stretch.
                // No node stands within 30 of the edge.
                let bank = mouths.iter().map(|m| (m.x - x).hypot(m.y - y) - 0.5 * m.width).fold(f64::MAX, f64::min);
                if r > 5970.0 && bank < REACH {
                    assert!((a - b).abs() < 2.0, "r{r} at {deg}°, {bank} from a mouth's bank: {a} to {b}");
                } else {
                    assert_eq!(a, b, "r{r} at {deg}°");
                }
            }
        }
    }

    #[test]
    fn the_rest_is_left_alone() {
        let t = fixture();
        let (rivers, streams, lakes) = run();
        for (was, now) in t.rivers.iter().zip(rivers).chain(t.streams.iter().zip(streams)) {
            if was.mouth != Mouth::Edge {
                assert_eq!(was, now);
            }
        }
        assert_eq!(&lakes[..t.lakes.len()], &t.lakes[..]);
    }
}
