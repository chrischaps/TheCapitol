//! Parks (plan §4.1.9, #111): the open ground a neighbourhood's lots leave,
//! where it's big enough to use.
//!
//! - **Ground.** Each neighbourhood's ground runs from its arterials' edges
//!   (or its greenway's centre line) between its side lines, inside the
//!   zone. It's rasterised in `CELL` squares. A square is *built* where a
//!   road or a lot covers its centre, *buffer* between a side line and the
//!   arterial or greenway, and *open* otherwise.
//! - **Usable.** Open ground is opened (eroded, then dilated) by
//!   `MIN_HALF_WIDTH`, so only ground at least twice that across is kept.
//!   Slivers between a lot and a road, or between two lots' rear lines,
//!   aren't a park.
//! - **Parks.** Each piece of usable ground left is a park if it holds at
//!   least `MIN_AREA` × frontage × depth, or if it runs into a buffer, so it
//!   widens the green strip along an arterial or a greenway. Whatever open
//!   ground is left is the gap the metrics count.
//!
//! The patterns leave the ground parks take on purpose: (b)'s spine down the
//! middle of its band, (c)'s green inside the loop and its corner wedges, and
//! (a)'s corners between a U-turn's fan and a side line. No lot is laid on a
//! park, because a park is only ground no lot took.
//!
//! **On the wire** (#129). `outlines` traces each park's squares into rings
//! for `/terrain`: the squares' boundary edges, chained into loops, then
//! each loop through its edges' midpoints (so a staircase becomes its
//! diagonal) simplified to `SIMPLIFY`. A park can be more than one piece
//! where a buffer joins them, and can hold holes, so the rings are filled
//! even-odd. A ring may cut a square's corner or `SIMPLIFY` past it, but no
//! further: a park's edge stays within a few units of where its ground ends.

use capitol_protocol::fmath::StableMath;
use std::collections::VecDeque;
use std::f64::consts::SQRT_2;

use capitol_protocol::terrain::{RoadSegment, TerrainPark, TerrainPlot};

use super::neighbourhood::{Facing, Neighbourhood};
use crate::models::terrain::SuburbanGenParams;

type P = (f64, f64);

/// The raster's square.
pub(crate) const CELL: f64 = 5.0;
/// Open ground narrower than twice this isn't usable.
pub(crate) const MIN_HALF_WIDTH: f64 = 10.0;
/// A park away from the buffers holds at least this share of a lot.
pub(crate) const MIN_AREA: f64 = 0.5;
/// How far a park's traced outline may stray from its squares' edges.
pub(crate) const SIMPLIFY: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ground {
    /// Not this neighbourhood's, or under a road or a lot.
    Built,
    /// Between a side line and the arterial or greenway.
    Buffer,
    Park,
    /// Open, but too narrow or too small to use.
    Gap,
}

/// One park: a connected piece of a neighbourhood's usable ground.
#[derive(Debug, Clone)]
pub(crate) struct Park {
    pub area: f64,
    /// Whether it runs into a buffer.
    pub on_buffer: bool,
    /// Its squares, as indices into `Greens::cells`.
    pub cells: Vec<usize>,
}

/// A neighbourhood's ground, square by square.
#[derive(Debug, Clone)]
pub(crate) struct Greens {
    pub sector: u8,
    pub nbhd: u8,
    /// The first square's corner.
    pub origin: P,
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Ground>,
    pub parks: Vec<Park>,
}

impl Greens {
    /// The ground at `p`, if it's in this neighbourhood's raster.
    pub(crate) fn at(&self, p: P) -> Option<Ground> {
        let (c, r) = ((p.0 - self.origin.0) / CELL, (p.1 - self.origin.1) / CELL);
        if c < 0.0 || r < 0.0 || c >= self.cols as f64 || r >= self.rows as f64 {
            return None;
        }
        Some(self.cells[r as usize * self.cols + c as usize])
    }

    /// The square at column `c`, row `r`: its corner.
    pub(crate) fn corner(&self, c: usize, r: usize) -> P {
        (self.origin.0 + c as f64 * CELL, self.origin.1 + r as f64 * CELL)
    }
}

/// The ground of every neighbourhood: `roads` and `lots` are the world's.
pub(crate) fn lay(
    roads: &[RoadSegment],
    lots: &[TerrainPlot],
    neighbourhoods: &[Neighbourhood],
    params: &SuburbanGenParams,
    center: P,
) -> Vec<Greens> {
    neighbourhoods.iter().map(|n| of(n, roads, lots, params, center)).collect()
}

/// A side line, or the arterial or greenway beyond it, as a half-plane
/// `n·(q − center) ≥ c`.
fn half_plane(angle: f64, inset: f64, hi: bool) -> (P, f64) {
    let n = if hi { (angle.stable_sin(), -angle.stable_cos()) } else { (-angle.stable_sin(), angle.stable_cos()) };
    (n, inset)
}

fn of(n: &Neighbourhood, roads: &[RoadSegment], lots: &[TerrainPlot], params: &SuburbanGenParams, center: P) -> Greens {
    let (inner, outer) = (params.inner_radius, params.outer_radius);
    let (lo, hi) = (n.frame.lo, n.frame.hi);
    // The ground stops at the arterial's edge, or the greenway's centre line.
    let reach = |f: Facing| match f {
        Facing::Arterial(_) => params.arterial_width / 2.0,
        Facing::Greenway => 0.0,
    };
    let sides = [half_plane(lo.angle, lo.inset, false), half_plane(hi.angle, hi.inset, true)];
    let ground = [half_plane(lo.angle, reach(n.lo), false), half_plane(hi.angle, reach(n.hi), true)];
    let within = |planes: &[(P, f64); 2], rel: P| planes.iter().all(|&(m, c)| m.0 * rel.0 + m.1 * rel.1 >= c);

    // The bounding box: the sector's arcs at both radii.
    let (mut bl, mut bh) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for k in 0..=32 {
        let a = lo.angle + (hi.angle - lo.angle) * k as f64 / 32.0;
        for r in [inner, outer] {
            let q = (center.0 + r * a.stable_cos(), center.1 + r * a.stable_sin());
            bl = (bl.0.min(q.0), bl.1.min(q.1));
            bh = (bh.0.max(q.0), bh.1.max(q.1));
        }
    }
    let origin = (bl.0 - CELL, bl.1 - CELL);
    let cols = ((bh.0 - origin.0) / CELL).ceil() as usize + 1;
    let rows = ((bh.1 - origin.1) / CELL).ceil() as usize + 1;
    let centre_of = |c: usize, r: usize| (origin.0 + (c as f64 + 0.5) * CELL, origin.1 + (r as f64 + 0.5) * CELL);

    let mut cells = vec![Ground::Built; cols * rows];
    for r in 0..rows {
        for c in 0..cols {
            let q = centre_of(c, r);
            let rel = (q.0 - center.0, q.1 - center.1);
            let d = rel.0.stable_hypot(rel.1);
            if d < inner || d > outer || !within(&ground, rel) {
                continue;
            }
            cells[r * cols + c] = if within(&sides, rel) { Ground::Gap } else { Ground::Buffer };
        }
    }

    // Roads and lots cover what they reach.
    let span = |lo: f64, hi: f64, o: f64, n: usize| {
        let a = (((lo - o) / CELL).floor().max(0.0)) as usize;
        let b = (((hi - o) / CELL).ceil().max(0.0) as usize).min(n);
        a..b
    };
    for road in roads {
        let h = road.width / 2.0;
        for w in road.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (x0, x1) = (a.0.min(b.0) - h, a.0.max(b.0) + h);
            let (y0, y1) = (a.1.min(b.1) - h, a.1.max(b.1) + h);
            if x1 < origin.0 || y1 < origin.1 || x0 > bh.0 + CELL || y0 > bh.1 + CELL {
                continue;
            }
            for r in span(y0, y1, origin.1, rows) {
                for c in span(x0, x1, origin.0, cols) {
                    if dist_to_seg(centre_of(c, r), a, b) < h {
                        cells[r * cols + c] = Ground::Built;
                    }
                }
            }
        }
    }
    for lot in lots {
        let q = &lot.corners;
        let (x0, x1) = q.iter().fold((f64::MAX, f64::MIN), |m, p| (m.0.min(p.0), m.1.max(p.0)));
        let (y0, y1) = q.iter().fold((f64::MAX, f64::MIN), |m, p| (m.0.min(p.1), m.1.max(p.1)));
        if x1 < origin.0 || y1 < origin.1 || x0 > bh.0 + CELL || y0 > bh.1 + CELL {
            continue;
        }
        for r in span(y0, y1, origin.1, rows) {
            for c in span(x0, x1, origin.0, cols) {
                if inside(q, centre_of(c, r)) {
                    cells[r * cols + c] = Ground::Built;
                }
            }
        }
    }

    // Open the open ground: squares at least MIN_HALF_WIDTH from anything
    // built, then everything within MIN_HALF_WIDTH of one of those.
    let open: Vec<bool> = cells.iter().map(|&g| g != Ground::Built).collect();
    let clear = chamfer(cols, rows, |i| !open[i]);
    let usable_seed: Vec<bool> = clear.iter().map(|&d| d >= MIN_HALF_WIDTH).collect();
    let from_seed = chamfer(cols, rows, |i| usable_seed[i]);
    let usable: Vec<bool> = (0..cells.len()).map(|i| open[i] && from_seed[i] <= MIN_HALF_WIDTH).collect();

    // Each piece of usable ground big enough, or touching a buffer, is a park.
    let min_area = MIN_AREA * params.neighbourhood.lot_frontage * params.neighbourhood.lot_depth;
    let mut seen = vec![false; cells.len()];
    let mut parks = Vec::new();
    for start in 0..cells.len() {
        if !usable[start] || seen[start] {
            continue;
        }
        let mut piece = Vec::new();
        let mut queue = VecDeque::from([start]);
        seen[start] = true;
        while let Some(i) = queue.pop_front() {
            piece.push(i);
            let (c, r) = (i % cols, i / cols);
            let mut visit = |j: usize| {
                if usable[j] && !seen[j] {
                    seen[j] = true;
                    queue.push_back(j);
                }
            };
            if c > 0 {
                visit(i - 1);
            }
            if c + 1 < cols {
                visit(i + 1);
            }
            if r > 0 {
                visit(i - cols);
            }
            if r + 1 < rows {
                visit(i + cols);
            }
        }
        let on_buffer = piece.iter().any(|&i| cells[i] == Ground::Buffer);
        let own: Vec<usize> = piece.into_iter().filter(|&i| cells[i] == Ground::Gap).collect();
        let area = own.len() as f64 * CELL * CELL;
        if own.is_empty() || (!on_buffer && area < min_area) {
            continue;
        }
        for &i in &own {
            cells[i] = Ground::Park;
        }
        parks.push(Park { area, on_buffer, cells: own });
    }

    Greens { sector: n.sector, nbhd: n.index, origin, cols, rows, cells, parks }
}

/// Every park in `greens`, traced for the wire, with ids
/// `park_{sector}_{nbhd}_{k}` in the order `lay` found them.
pub(crate) fn outlines(greens: &[Greens]) -> Vec<TerrainPark> {
    let mut out = Vec::new();
    for g in greens {
        for (k, park) in g.parks.iter().enumerate() {
            let rings: Vec<Vec<P>> = trace(g, &park.cells).into_iter().filter(|r| r.len() >= 3).collect();
            let area = rings.iter().map(|r| TerrainPark::signed_area(r)).sum::<f64>().abs();
            out.push(TerrainPark { id: format!("park_{}_{}_{k}", g.sector, g.nbhd), rings, area });
        }
    }
    out
}

/// The squares `cells` of `g` as closed rings in world units, simplified.
fn trace(g: &Greens, cells: &[usize]) -> Vec<Vec<P>> {
    let cols = g.cols;
    let mut mine = vec![false; g.cells.len()];
    for &i in cells {
        mine[i] = true;
    }
    let has = |c: i64, r: i64| c >= 0 && r >= 0 && (c as usize) < cols && (r as usize) < g.rows && mine[r as usize * cols + c as usize];

    // Each boundary edge, between grid vertices, with the park on one side.
    // Every vertex has as many edges out as in, so they chain into loops
    // whichever way a vertex two loops touch at is taken.
    let mut out: std::collections::HashMap<(i64, i64), Vec<(i64, i64)>> = Default::default();
    for &i in cells {
        let (c, r) = ((i % cols) as i64, (i / cols) as i64);
        let mut edge = |a: (i64, i64), b: (i64, i64)| out.entry(a).or_default().push(b);
        if !has(c, r - 1) {
            edge((c, r), (c + 1, r));
        }
        if !has(c + 1, r) {
            edge((c + 1, r), (c + 1, r + 1));
        }
        if !has(c, r + 1) {
            edge((c + 1, r + 1), (c, r + 1));
        }
        if !has(c - 1, r) {
            edge((c, r + 1), (c, r));
        }
    }
    let mut starts: Vec<(i64, i64)> = out.keys().copied().collect();
    starts.sort_unstable();

    let mut rings = Vec::new();
    for start in starts {
        while out.get(&start).is_some_and(|v| !v.is_empty()) {
            let mut lp = vec![start];
            let mut at = start;
            loop {
                let next = out.get_mut(&at).and_then(|v| v.pop()).expect("a boundary edge goes on");
                if next == start {
                    break;
                }
                lp.push(next);
                at = next;
            }
            // Through the edges' midpoints, in world units.
            let n = lp.len();
            let mids: Vec<P> = (0..n)
                .map(|k| {
                    let (a, b) = (lp[k], lp[(k + 1) % n]);
                    let m = ((a.0 + b.0) as f64 / 2.0, (a.1 + b.1) as f64 / 2.0);
                    (g.origin.0 + m.0 * CELL, g.origin.1 + m.1 * CELL)
                })
                .collect();
            rings.push(simplify_ring(&mids, SIMPLIFY));
        }
    }
    rings
}

/// Douglas-Peucker on a closed ring: split at the point farthest from the
/// first, and simplify each half.
fn simplify_ring(ring: &[P], tolerance: f64) -> Vec<P> {
    let n = ring.len();
    if n < 4 {
        return ring.to_vec();
    }
    let far = (1..n)
        .max_by(|&a, &b| {
            let d = |k: usize| (ring[k].0 - ring[0].0).stable_hypot(ring[k].1 - ring[0].1);
            d(a).total_cmp(&d(b))
        })
        .unwrap();
    let mut first: Vec<P> = ring[..=far].to_vec();
    let mut second: Vec<P> = ring[far..].to_vec();
    second.push(ring[0]);
    first = simplify(&first, tolerance);
    second = simplify(&second, tolerance);
    first.pop();
    second.pop();
    first.extend(second);
    first
}

/// Douglas-Peucker on an open polyline, keeping both ends.
fn simplify(line: &[P], tolerance: f64) -> Vec<P> {
    if line.len() < 3 {
        return line.to_vec();
    }
    let (a, b) = (line[0], line[line.len() - 1]);
    let (k, d) = (1..line.len() - 1)
        .map(|k| (k, dist_to_seg(line[k], a, b)))
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap();
    if d <= tolerance {
        return vec![a, b];
    }
    let mut left = simplify(&line[..=k], tolerance);
    left.pop();
    left.extend(simplify(&line[k..], tolerance));
    left
}

/// Each square's distance to the nearest `seed` square, in world units
/// (a 3×3 chamfer, within 8% of the true distance).
fn chamfer(cols: usize, rows: usize, seed: impl Fn(usize) -> bool) -> Vec<f64> {
    chamfer_with(cols, rows, CELL, seed)
}

/// `chamfer` on squares `cell` across. The farms' balance pass (#287) uses
/// it on a coarser raster than the parks'.
pub(crate) fn chamfer_with(cols: usize, rows: usize, cell: f64, seed: impl Fn(usize) -> bool) -> Vec<f64> {
    let (straight, diagonal) = (cell, cell * SQRT_2);
    let mut d: Vec<f64> = (0..cols * rows).map(|i| if seed(i) { 0.0 } else { f64::INFINITY }).collect();
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            let mut best = d[i];
            if c > 0 {
                best = best.min(d[i - 1] + straight);
            }
            if r > 0 {
                best = best.min(d[i - cols] + straight);
                if c > 0 {
                    best = best.min(d[i - cols - 1] + diagonal);
                }
                if c + 1 < cols {
                    best = best.min(d[i - cols + 1] + diagonal);
                }
            }
            d[i] = best;
        }
    }
    for r in (0..rows).rev() {
        for c in (0..cols).rev() {
            let i = r * cols + c;
            let mut best = d[i];
            if c + 1 < cols {
                best = best.min(d[i + 1] + straight);
            }
            if r + 1 < rows {
                best = best.min(d[i + cols] + straight);
                if c + 1 < cols {
                    best = best.min(d[i + cols + 1] + diagonal);
                }
                if c > 0 {
                    best = best.min(d[i + cols - 1] + diagonal);
                }
            }
            d[i] = best;
        }
    }
    d
}

fn dist_to_seg(q: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((q.0 - a.0) * dx + (q.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (q.0 - a.0 - t * dx).stable_hypot(q.1 - a.1 - t * dy)
}

/// Whether `q` is inside the quad `c` (even-odd).
fn inside(c: &[P; 4], q: P) -> bool {
    let mut odd = false;
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 3) % 4]);
        if (a.1 > q.1) != (b.1 > q.1) && q.0 < (b.0 - a.0) * (q.1 - a.1) / (b.1 - a.1) + a.0 {
            odd = !odd;
        }
    }
    odd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chamfer_measures_from_its_seeds() {
        // One seed in the middle of a 5×5 grid.
        let d = chamfer(5, 5, |i| i == 12);
        assert_eq!(d[12], 0.0);
        assert_eq!(d[13], CELL);
        assert!((d[18] - CELL * SQRT_2).abs() < 1e-9);
        assert_eq!(d[14], 2.0 * CELL);
    }

    /// A 6×6 block of squares with a 2×2 hole, and a lone square off its
    /// corner: two outer rings and a hole, about the squares' area.
    #[test]
    fn a_park_traces_into_its_rings() {
        let (cols, rows) = (10, 10);
        let mut cells = vec![Ground::Built; cols * rows];
        let mut own = Vec::new();
        for r in 1..7 {
            for c in 1..7 {
                if !(3..5).contains(&r) || !(3..5).contains(&c) {
                    own.push(r * cols + c);
                }
            }
        }
        own.push(8 * cols + 8);
        for &i in &own {
            cells[i] = Ground::Park;
        }
        let area = own.len() as f64 * CELL * CELL;
        let g = Greens {
            sector: 3,
            nbhd: 1,
            origin: (100.0, 200.0),
            cols,
            rows,
            cells,
            parks: vec![Park { area, on_buffer: false, cells: own }],
        };
        let parks = outlines(std::slice::from_ref(&g));
        assert_eq!(parks.len(), 1);
        let p = &parks[0];
        assert_eq!(p.id, "park_3_1_0");
        assert_eq!(p.rings.len(), 3, "{:?}", p.rings);
        // The midpoints cut each corner by an eighth of a square.
        assert!((p.area - area).abs() < 0.1 * area, "{} vs {area}", p.area);
        let at = |c: f64, r: f64| (100.0 + c * CELL, 200.0 + r * CELL);
        for (q, want) in [(at(1.5, 1.5), true), (at(4.0, 4.0), false), (at(8.5, 8.5), true), (at(7.5, 7.5), false), (at(0.5, 0.5), false)] {
            assert_eq!(p.contains(q.0, q.1), want, "{q:?}");
        }
    }
}

