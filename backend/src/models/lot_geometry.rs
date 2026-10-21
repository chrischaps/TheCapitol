//! Lot geometry shared by the suburb's lots and the rural farms (R2, #285).
//!
//! Lifted out of `suburb/lots.rs`, which keeps its street walker, its bulbs
//! and its ids, and calls this through a thin adapter. The farms (R4, #287)
//! will walk their lanes with it too. Here:
//! - **`World`.** Every road a lot must stay out of, indexed, with the sides
//!   of each road that have lots, and the water (below).
//! - **`Region`.** Where a walker's lots may go: an annulus, cut by
//!   half-planes, and by any polygon.
//! - **`Ray`.** A lot's side edge, out from just off a road's edge. The ray
//!   march (`World::back_from`) finds how deep it may go.
//! - **The overlap pass** (`place`, `shrink_to_fit`): lots in priority
//!   order, each shrunk towards its street or dropped to keep clear of the
//!   roads, the water and the lots before it.
//! - **`LotParams`.** The sizes and gaps all of that works to.
//!
//! **Water.** A river or stream is a capsule round each stretch, as wide as
//! the wider of its two points (as R1's metrics measure it), and a lake is
//! its even-odd rings, so a lot may stand on an island. Each keeps `bank`
//! clear. A ray stops at a bank as it stops at a road with no lots on its
//! side, so a farm backs onto the water rather than into it. The suburb has
//! no water and passes none, so nothing here changes it.

use capitol_protocol::fmath::StableMath;
use std::collections::HashMap;

use capitol_protocol::terrain::{Lake, RoadSegment, TerrainPlot, WaterCourse};

use crate::models::geom::{quad_is_convex, quads_overlap_strict, Polyline, SpatialGrid};

type P = (f64, f64);

/// A lot may touch a road's edge, within this.
const TOUCH: f64 = 0.01;
/// Half the stretch a ray's direction is averaged over, and the curvature's.
const TANGENT_REACH: f64 = 5.0;
const CURVE_REACH: f64 = 20.0;
/// How far a lot's front may be pushed out to clear the road's curve.
const MAX_FRONT_PUSH: f64 = 12.0;
const PUSH_STEP: f64 = 0.25;
/// A corner a through road owns runs this much past its lots' deepest reach,
/// which starts a hair off the road's edge.
const OWNED_SLACK: f64 = 1.0;
/// How far a street's line runs on into the street it continues.
const LEAD: f64 = 30.0;

/// The sizes and gaps lots are laid to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LotParams {
    /// Width along the road.
    pub frontage: f64,
    /// The usual depth, and the most a lot takes up to an edge strip.
    pub depth: f64,
    pub depth_max: f64,
    /// Lots stand this far off a road's edge...
    pub front_gap: f64,
    /// ...stop this short of an edge they back onto...
    pub edge_gap: f64,
    /// ...and this short of the line they share with lots back to back.
    pub rear_gap: f64,
    /// The least gap between two lots. At 0 they may share an edge.
    pub lot_gap: f64,
    /// The overlap pass shrinks a lot to no less than this share of its
    /// depth...
    pub min_shrink: f64,
    /// ...and drops one under this share of frontage × depth.
    pub min_area: f64,
}

/// One stretch of a road's centre line, with the road's half width.
pub(crate) struct Seg {
    pub a: P,
    pub b: P,
    pub half: f64,
    pub road: usize,
}

/// Every road a lot must stay out of, which of them have lots, and the
/// water.
pub(crate) struct World<'a> {
    pub roads: &'a [RoadSegment],
    segs: Vec<Seg>,
    grid: SpatialGrid,
    max_half: f64,
    /// Which sides of each road have lots, left and right along its points.
    sides: Vec<(bool, bool)>,
    /// Each fronted road's vertices, so a street's end can find a road it
    /// meets partway along.
    vertices: HashMap<(u64, u64), Vec<(usize, usize)>>,
    pub center: P,
    /// How far from `center` the indexes reach.
    pub reach: f64,
    water: Water<'a>,
}

fn key(p: P) -> (u64, u64) {
    (p.0.to_bits(), p.1.to_bits())
}

impl<'a> World<'a> {
    /// `sides` is each road's fronted sides, left and right. `reach` is how
    /// far from `center` a lot may go, its depth included.
    pub(crate) fn new(roads: &'a [RoadSegment], sides: Vec<(bool, bool)>, center: P, reach: f64) -> Self {
        let mut grid = SpatialGrid::new(center.0, center.1, reach, 40.0);
        let mut segs = Vec::new();
        for (road, r) in roads.iter().enumerate() {
            for w in r.points.windows(2) {
                grid.insert(segs.len(), w);
                segs.push(Seg { a: w[0], b: w[1], half: r.width / 2.0, road });
            }
        }
        let max_half = roads.iter().map(|r| r.width / 2.0).fold(0.0, f64::max);
        let fronted: Vec<bool> = sides.iter().map(|&s| s != (false, false)).collect();
        let mut vertices: HashMap<(u64, u64), Vec<(usize, usize)>> = HashMap::new();
        for (road, r) in roads.iter().enumerate().filter(|&(i, _)| fronted[i]) {
            for (i, &q) in r.points.iter().enumerate() {
                vertices.entry(key(q)).or_default().push((road, i));
            }
        }
        let water = Water::none(center, reach);
        Self { roads, segs, grid, max_half, sides, vertices, center, reach, water }
    }

    /// The same world, with rivers and streams, and lakes, that lots keep
    /// `bank` clear of.
    #[cfg_attr(not(test), allow(dead_code))] // the farms (R4, #287) lay round the water
    pub(crate) fn with_water<'c>(
        mut self,
        courses: impl IntoIterator<Item = &'c WaterCourse>,
        lakes: &'a [Lake],
        bank: f64,
    ) -> Self {
        self.water = Water::new(courses, lakes, bank, self.center, self.reach);
        self
    }

    /// The roads' stretches near the box `lo`..`hi`.
    pub(crate) fn near(&self, lo: P, hi: P) -> impl Iterator<Item = &Seg> {
        let m = self.max_half;
        self.grid.query_aabb(lo.0 - m, lo.1 - m, hi.0 + m, hi.1 + m).into_iter().map(|i| &self.segs[i])
    }

    /// Whether `q` is clear of every road and the water.
    pub(crate) fn point_clear(&self, q: P) -> bool {
        self.near(q, q).all(|s| dist_to_seg(q, s.a, s.b) >= s.half - TOUCH) && self.water.point_clear(q)
    }

    /// Whether the segment `a`..`b` is clear of every road and the water.
    pub(crate) fn edge_clear(&self, a: P, b: P) -> bool {
        let (lo, hi) = bounds(&[a, b]);
        self.near(lo, hi).all(|s| seg_seg_dist(a, b, s.a, s.b) >= s.half - TOUCH) && self.water.edge_clear(a, b)
    }

    /// Whether a convex quad is clear of every road and the water.
    pub(crate) fn quad_clear(&self, q: &[P; 4]) -> bool {
        let (lo, hi) = bounds(q);
        self.near(lo, hi).all(|s| {
            !inside_convex(q, s.a) && (0..4).all(|i| seg_seg_dist(q[i], q[(i + 1) % 4], s.a, s.b) >= s.half - TOUCH)
        }) && self.water.quad_clear(q)
    }

    /// The first road or bank the ray `o + t·d` enters after `t0` and by
    /// `limit`, and whether it's a road with lots on the side the ray
    /// comes from. A bank has none.
    pub(crate) fn first_hit(&self, o: P, d: P, t0: f64, limit: f64) -> Option<(f64, bool)> {
        let (a, b) = (at(o, d, t0), at(o, d, limit));
        let (lo, hi) = bounds(&[a, b]);
        let road = self
            .near(lo, hi)
            .filter_map(|s| {
                capsule_entry(o, d, t0, s.a, s.b, s.half).map(|t| (t, s.road, self.fronts(s.road, s.a, s.b, o)))
            })
            .filter(|&(t, _, _)| t <= limit)
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .map(|(t, _, fronted)| (t, fronted));
        match (road, self.water.first_hit(o, d, t0, limit)) {
            (Some((t, _)), Some(w)) if w < t => Some((w, false)),
            (None, Some(w)) => Some((w, false)),
            (road, _) => road,
        }
    }

    /// Whether `road` has lots on the side of its stretch `a`..`b` that `q` is on.
    pub(crate) fn fronts(&self, road: usize, a: P, b: P, q: P) -> bool {
        let (l, r) = self.sides[road];
        let left = (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0) > 0.0;
        if left {
            l
        } else {
            r
        }
    }

    /// How far out the ray from `o` along `d`, starting at `f`, can go.
    pub(crate) fn back(&self, region: &Region, o: P, d: P, f: f64, p: &LotParams) -> f64 {
        self.back_from(region, o, d, f, f, p.depth, p)
    }

    /// `back`, with lots back to back meeting half way between `near` and
    /// the other road, and `open` deep onto open ground inside the region.
    /// A pie's ray starts at its bulb's edge, but its half way is measured
    /// from a street's edge like the stem lots it backs onto, or it reaches
    /// past their shared rear line. Pies facing the middle of a pattern (b)
    /// band take `depth_max`, to fill it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn back_from(&self, region: &Region, o: P, d: P, f: f64, near: f64, open: f64, p: &LotParams) -> f64 {
        let exit = region.exit(o, d, f);
        match self.first_hit(o, d, f, exit.min(f + 2.0 * p.depth_max)) {
            // Back to back: half way to the lots fronting the other road.
            Some((t, true)) => ((near + t) / 2.0 - p.rear_gap).max(f),
            // A road with no lots this side, or a bank.
            Some((t, false)) => (t - p.edge_gap).min(f + p.depth),
            // Open ground to the region's edge: as deep as a lot may be, to
            // take up the edge strip.
            None if exit - f <= 2.0 * p.depth_max => (exit - p.edge_gap).min(f + p.depth_max),
            // Open ground inside it is for other streets' lots, such as a
            // comb's beside the racetrack (#109): a lot's usual depth.
            None => f + open,
        }
    }

    /// The points of a fronted street that carries on from `road`'s point
    /// `end` (its first or last), walking away from it, up to `LEAD` along.
    pub(crate) fn lead(&self, road: usize, end: usize) -> Vec<P> {
        let at_end = self.roads[road].points[end];
        let Some(&(r, i)) = self.vertices.get(&key(at_end)).into_iter().flatten().find(|&&(r, i)| {
            let n = self.roads[r].points.len();
            (r, i) != (road, end) && (i == 0 || i + 1 == n)
        }) else {
            return Vec::new();
        };
        let pts = &self.roads[r].points;
        let walk: Box<dyn Iterator<Item = &P>> = if i == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        let mut out = Vec::new();
        let (mut prev, mut along) = (at_end, 0.0);
        for &q in walk.skip(1) {
            along += dist(prev, q);
            out.push(q);
            prev = q;
            if along >= LEAD {
                break;
            }
        }
        out
    }

    /// Stretches of a fronted road, from its ends, that a fronted road it
    /// meets partway along owns: that road's lots fill the corner. They run
    /// beside this road and find open ground inside the region, so they're
    /// `depth` deep (pattern (b)'s combs, #109). A road with no lots on this
    /// road's side owns nothing (pattern (c)'s loop, #110).
    pub(crate) fn owned_ends(&self, road: usize, length: f64, p: &LotParams) -> Vec<(f64, f64)> {
        let pts = &self.roads[road].points;
        let n = pts.len();
        let mut out = Vec::new();
        for (end, next, s) in [(pts[0], pts[1], 0.0), (pts[n - 1], pts[n - 2], length)] {
            let through = self.vertices.get(&key(end)).into_iter().flatten().find(|&&(r, i)| {
                let rp = &self.roads[r].points;
                r != road && i > 0 && i + 1 < rp.len() && self.fronts(r, rp[i - 1], rp[i + 1], next)
            });
            if let Some(&(r, _)) = through {
                let len = self.roads[r].width / 2.0 + p.depth + OWNED_SLACK;
                out.push(if s == 0.0 { (0.0, len) } else { (length - len, length) });
            }
        }
        out
    }
}

/// One stretch of water's edge, as a capsule: a river's or stream's centre
/// line with half its width and the bank, or a lake's shore with the bank.
struct Bank {
    a: P,
    b: P,
    half: f64,
}

/// The water a world's lots keep clear of.
struct Water<'a> {
    banks: Vec<Bank>,
    grid: SpatialGrid,
    max_half: f64,
    /// Each lake, with its bounding box, for the even-odd test.
    lakes: Vec<(&'a Lake, (P, P))>,
}

impl<'a> Water<'a> {
    fn none(center: P, reach: f64) -> Self {
        // One cell: an empty index needs no more.
        Self { banks: Vec::new(), grid: SpatialGrid::new(center.0, center.1, reach, 2.0 * reach), max_half: 0.0, lakes: Vec::new() }
    }

    #[cfg_attr(not(test), allow(dead_code))] // through `World::with_water`
    fn new<'c>(courses: impl IntoIterator<Item = &'c WaterCourse>, lakes: &'a [Lake], bank: f64, center: P, reach: f64) -> Self {
        let mut banks = Vec::new();
        for course in courses {
            for w in course.points.windows(2) {
                let half = w[0].width.max(w[1].width) / 2.0 + bank;
                banks.push(Bank { a: (w[0].x, w[0].y), b: (w[1].x, w[1].y), half });
            }
        }
        for lake in lakes {
            for ring in &lake.rings {
                for i in 0..ring.len() {
                    banks.push(Bank { a: ring[i], b: ring[(i + 1) % ring.len()], half: bank });
                }
            }
        }
        // Water runs on past the lots' reach, into the wilds; it needn't be indexed there.
        let (lo, hi) = ((center.0 - reach, center.1 - reach), (center.0 + reach, center.1 + reach));
        banks.retain(|s| {
            let (blo, bhi) = bounds(&[s.a, s.b]);
            bhi.0 + s.half >= lo.0 && blo.0 - s.half <= hi.0 && bhi.1 + s.half >= lo.1 && blo.1 - s.half <= hi.1
        });
        let mut grid = SpatialGrid::new(center.0, center.1, reach, 40.0);
        for (i, s) in banks.iter().enumerate() {
            grid.insert(i, &[s.a, s.b]);
        }
        let max_half = banks.iter().map(|s| s.half).fold(0.0, f64::max);
        let lakes = lakes.iter().map(|l| (l, bounds(&l.rings.concat()))).collect();
        Self { banks, grid, max_half, lakes }
    }

    fn near(&self, lo: P, hi: P) -> impl Iterator<Item = &Bank> {
        let m = self.max_half;
        let found = if self.banks.is_empty() { Vec::new() } else { self.grid.query_aabb(lo.0 - m, lo.1 - m, hi.0 + m, hi.1 + m) };
        found.into_iter().map(|i| &self.banks[i])
    }

    fn in_lake(&self, q: P) -> bool {
        self.lakes
            .iter()
            .any(|(l, (lo, hi))| q.0 >= lo.0 && q.0 <= hi.0 && q.1 >= lo.1 && q.1 <= hi.1 && l.contains(q.0, q.1))
    }

    // A bank is strict, unlike a road's edge: with no bank, a lake's shore
    // is a line a lot mustn't cross, not one it may lie along.
    fn point_clear(&self, q: P) -> bool {
        self.near(q, q).all(|s| dist_to_seg(q, s.a, s.b) > s.half) && !self.in_lake(q)
    }

    fn edge_clear(&self, a: P, b: P) -> bool {
        let (lo, hi) = bounds(&[a, b]);
        // Clear of every shore, it's wholly on the water or off it.
        self.near(lo, hi).all(|s| seg_seg_dist(a, b, s.a, s.b) > s.half) && !self.in_lake(a)
    }

    fn quad_clear(&self, q: &[P; 4]) -> bool {
        let (lo, hi) = bounds(q);
        // A pond inside the lot has a shore point inside it; a lot on the
        // open water has its corners in the lake.
        self.near(lo, hi)
            .all(|s| !inside_convex(q, s.a) && (0..4).all(|i| seg_seg_dist(q[i], q[(i + 1) % 4], s.a, s.b) > s.half))
            && !self.in_lake(q[0])
    }

    /// Where the ray `o + t·d` first meets a bank after `t0`, by `limit`.
    fn first_hit(&self, o: P, d: P, t0: f64, limit: f64) -> Option<f64> {
        let (lo, hi) = bounds(&[at(o, d, t0), at(o, d, limit)]);
        self.near(lo, hi)
            .filter_map(|s| capsule_entry(o, d, t0, s.a, s.b, s.half))
            .filter(|&t| t <= limit)
            .min_by(f64::total_cmp)
    }
}

/// Where a walker's lots may go: in an annulus round `center`, on the inside
/// of each side line, and inside `within` (even-odd), if it has rings.
pub(crate) struct Region {
    center: P,
    /// Each side line as a half-plane `n·(q − center) ≥ c`.
    sides: Vec<(P, f64)>,
    inner: f64,
    outer: f64,
    within: Vec<Vec<P>>,
}

impl Region {
    /// The annulus from `inner` to `outer` round `center`.
    pub(crate) fn ring(center: P, inner: f64, outer: f64) -> Self {
        Region { center, sides: Vec::new(), inner, outer, within: Vec::new() }
    }

    /// The inside of `rings`, filled even-odd.
    #[cfg_attr(not(test), allow(dead_code))] // for a walker whose ground isn't a sector
    pub(crate) fn polygon(center: P, rings: Vec<Vec<P>>) -> Self {
        Region { within: rings, ..Self::ring(center, 0.0, f64::INFINITY) }
    }

    /// Cut to the counter-clockwise side of the ray from the centre at
    /// `angle`, offset `inset` from it.
    pub(crate) fn ccw_of(mut self, angle: f64, inset: f64) -> Self {
        self.sides.push(((-angle.stable_sin(), angle.stable_cos()), inset));
        self
    }

    /// Cut to the clockwise side of the ray from the centre at `angle`,
    /// offset `inset` from it.
    pub(crate) fn cw_of(mut self, angle: f64, inset: f64) -> Self {
        self.sides.push(((angle.stable_sin(), -angle.stable_cos()), inset));
        self
    }

    pub(crate) fn inside(&self, q: P) -> bool {
        let rel = (q.0 - self.center.0, q.1 - self.center.1);
        let r = rel.0.stable_hypot(rel.1);
        self.sides.iter().all(|&(n, c)| dot(n, rel) >= c - 1e-9)
            && r >= self.inner
            && r <= self.outer
            && (self.within.is_empty() || rings_contain(&self.within, q))
    }

    /// Where the ray `o + t·d`, inside at `t0`, leaves.
    pub(crate) fn exit(&self, o: P, d: P, t0: f64) -> f64 {
        let rel = (o.0 - self.center.0, o.1 - self.center.1);
        let mut best = f64::INFINITY;
        for &(n, c) in &self.sides {
            let nd = dot(n, d);
            if nd < 0.0 {
                let t = (c - dot(n, rel)) / nd;
                if t >= t0 {
                    best = best.min(t);
                }
            }
        }
        let (b, q) = (dot(rel, d), dot(rel, rel));
        for (r, entering) in [(self.outer, false), (self.inner, true)] {
            // No inner circle to meet, and an outer one too far to.
            if (entering && r <= 0.0) || r.is_infinite() {
                continue;
            }
            let disc = b * b - (q - r * r);
            if disc >= 0.0 {
                let t = if entering { -b - disc.sqrt() } else { -b + disc.sqrt() };
                if t >= t0 {
                    best = best.min(t);
                }
            }
        }
        // Inside at t0, the first shore crossed after it is the way out.
        for ring in &self.within {
            for i in 0..ring.len() {
                if let Some(t) = ray_cross(o, d, ring[i], ring[(i + 1) % ring.len()]) {
                    if t >= t0 {
                        best = best.min(t);
                    }
                }
            }
        }
        best
    }
}

/// Even-odd over closed rings: inside an odd number of them.
fn rings_contain(rings: &[Vec<P>], q: P) -> bool {
    let mut inside = false;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            if (a.1 > q.1) != (b.1 > q.1) && q.0 < a.0 + (q.1 - a.1) * (b.0 - a.0) / (b.1 - a.1) {
                inside = !inside;
            }
        }
    }
    inside
}

/// Where the ray `o + t·d` crosses the segment `a`..`b`, if it does.
fn ray_cross(o: P, d: P, a: P, b: P) -> Option<f64> {
    let e = (b.0 - a.0, b.1 - a.1);
    let den = d.0 * e.1 - d.1 * e.0;
    if den == 0.0 {
        return None;
    }
    let w = (a.0 - o.0, a.1 - o.1);
    let t = (w.0 * e.1 - w.1 * e.0) / den;
    let u = (w.0 * d.1 - w.1 * d.0) / den;
    (0.0..=1.0).contains(&u).then_some(t)
}

/// A lot's side edge: from `o + f·d` out to `o + b·d`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ray {
    pub o: P,
    pub d: P,
    pub f: f64,
    pub b: f64,
    /// Shared with lots already made, so it mustn't move.
    pub fixed: bool,
}

impl Ray {
    pub(crate) fn front(&self) -> P {
        at(self.o, self.d, self.f)
    }

    pub(crate) fn back(&self) -> P {
        at(self.o, self.d, self.b)
    }

    pub(crate) fn mid(&self) -> P {
        at(self.o, self.d, (self.f + self.b) / 2.0)
    }
}

/// A ray `t` of the way from `a` to `b`. Sample rays are a few units apart,
/// so this is as good as casting one there, and it can't land in a shadow.
pub(crate) fn blend(a: &Ray, b: &Ray, t: f64) -> Ray {
    let front = lerp(a.front(), b.front(), t);
    let back = lerp(a.back(), b.back(), t);
    let len = dist(front, back);
    Ray { o: front, d: ((back.0 - front.0) / len, (back.1 - front.1) / len), f: 0.0, b: len, fixed: false }
}

/// The lot between two rays: street-left, street-right, rear-right, rear-left.
pub(crate) fn quad(a: &Ray, b: &Ray) -> [P; 4] {
    [a.front(), b.front(), b.back(), a.back()]
}

/// Push fronts out until each lot's front edge is clear of the roads: on a
/// curve's convex side the chord between two rays cuts into the road.
pub(crate) fn push_fronts(rays: &mut [Ray], world: &World) {
    for j in 0..rays.len().saturating_sub(1) {
        let mut pushed = 0.0;
        while pushed < MAX_FRONT_PUSH && !world.edge_clear(rays[j].front(), rays[j + 1].front()) {
            for k in [j, j + 1] {
                if !rays[k].fixed && rays[k].f + PUSH_STEP < rays[k].b {
                    rays[k].f += PUSH_STEP;
                }
            }
            pushed += PUSH_STEP;
        }
    }
}

/// A lot before the overlap pass.
pub(crate) struct Candidate {
    pub id: String,
    /// The road it fronts.
    pub road: usize,
    /// Street-left, street-right, rear-right, rear-left.
    pub corners: [P; 4],
}

/// The overlap pass: candidates in priority order, each shrunk or dropped
/// to keep clear of the roads, the water and the lots before it.
pub(crate) fn place(candidates: impl IntoIterator<Item = Candidate>, world: &World, p: &LotParams, zone: &str) -> Vec<TerrainPlot> {
    let mut grid = SpatialGrid::new(world.center.0, world.center.1, world.reach, 50.0);
    let mut placed: Vec<[P; 4]> = Vec::new();
    let min_area = p.min_area * p.frontage * p.depth;
    let g = p.lot_gap;
    let mut out = Vec::new();
    for c in candidates {
        let ok = |q: &[P; 4]| {
            let (lo, hi) = bounds(q);
            quad_is_convex(q)
                && TerrainPlot::calculate_area(q) >= min_area
                && world.quad_clear(q)
                && grid
                    .query_aabb(lo.0 - g, lo.1 - g, hi.0 + g, hi.1 + g)
                    .into_iter()
                    .all(|i| !quads_overlap_strict(q, &placed[i]) && (g <= 0.0 || quad_gap(q, &placed[i]) >= g))
        };
        let Some(corners) = shrink_to_fit(c.corners, p.min_shrink, ok) else { continue };
        grid.insert(placed.len(), &corners);
        placed.push(corners);
        out.push(TerrainPlot {
            id: c.id,
            corners,
            street_facing_edge: (0, 1),
            area: TerrainPlot::calculate_area(&corners),
            adjacent_street_id: world.roads[c.road].id.clone(),
            zone: zone.to_string(),
        });
    }
    out
}

/// The lot as it is if it's `ok`, or pulled back towards the street to the
/// deepest that is, down to `min_shrink` of its depth.
pub(crate) fn shrink_to_fit(c: [P; 4], min_shrink: f64, ok: impl Fn(&[P; 4]) -> bool) -> Option<[P; 4]> {
    if ok(&c) {
        return Some(c);
    }
    let at = |k: f64| [c[0], c[1], lerp(c[1], c[2], k), lerp(c[0], c[3], k)];
    if !ok(&at(min_shrink)) {
        return None;
    }
    let (mut good, mut bad) = (min_shrink, 1.0);
    for _ in 0..10 {
        let mid = (good + bad) / 2.0;
        if ok(&at(mid)) {
            good = mid;
        } else {
            bad = mid;
        }
    }
    Some(at(good))
}

/// The gap between two quads that don't overlap, edge to edge.
fn quad_gap(a: &[P; 4], b: &[P; 4]) -> f64 {
    let mut gap = f64::INFINITY;
    for i in 0..4 {
        for k in 0..4 {
            gap = gap.min(seg_seg_dist(a[i], a[(i + 1) % 4], b[k], b[(k + 1) % 4]));
        }
    }
    gap
}

pub(crate) fn point(line: &Polyline, s: f64) -> P {
    let (x, y, _) = line.at(s).expect("a line of two points or more");
    (x, y)
}

/// The line's direction at `s`, averaged over `TANGENT_REACH` either side so
/// a ray at a vertex doesn't take one segment's side.
pub(crate) fn tangent(line: &Polyline, s: f64) -> P {
    let (a, b) = (point(line, s - TANGENT_REACH), point(line, s + TANGENT_REACH));
    let len = dist(a, b);
    if len > 0.0 {
        ((b.0 - a.0) / len, (b.1 - a.1) / len)
    } else {
        let (_, _, h) = line.at(s).expect("a line");
        (h.stable_cos(), h.stable_sin())
    }
}

/// Signed curvature at `s` (positive turning left), from the turn over
/// `CURVE_REACH` either side.
pub(crate) fn curvature(line: &Polyline, s: f64) -> f64 {
    let (s0, s1) = ((s - CURVE_REACH).max(0.0), (s + CURVE_REACH).min(line.length()));
    if s1 - s0 <= 0.0 {
        return 0.0;
    }
    let (a, b) = (tangent(line, s0), tangent(line, s1));
    let turn = (a.0 * b.1 - a.1 * b.0).stable_atan2(dot(a, b));
    turn / (s1 - s0)
}

/// Where the ray `o + t·d` first meets the capsule round `a`..`b` of radius
/// `h`, at or after `t0`.
fn capsule_entry(o: P, d: P, t0: f64, a: P, b: P, h: f64) -> Option<f64> {
    if dist_to_seg(at(o, d, t0), a, b) <= h {
        return Some(t0);
    }
    // The least boundary crossing past t0 is the entry: past a crossing
    // that isn't on the boundary, the ray is already inside.
    let mut best: Option<f64> = None;
    let mut take = |t: f64| {
        if t >= t0 && best.is_none_or(|x| t < x) {
            best = Some(t);
        }
    };
    for c in [a, b] {
        let q = (o.0 - c.0, o.1 - c.1);
        let (bq, cq) = (dot(q, d), dot(q, q) - h * h);
        let disc = bq * bq - cq;
        if disc >= 0.0 {
            take(-bq - disc.sqrt());
            take(-bq + disc.sqrt());
        }
    }
    let len = dist(a, b);
    if len > 0.0 {
        let e = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        let m = (-e.1, e.0);
        let dm = dot(d, m);
        if dm != 0.0 {
            let om = dot((o.0 - a.0, o.1 - a.1), m);
            for off in [h, -h] {
                let t = (off - om) / dm;
                let along = dot((o.0 + t * d.0 - a.0, o.1 + t * d.1 - a.1), e);
                if (0.0..=len).contains(&along) {
                    take(t);
                }
            }
        }
    }
    best
}

pub(crate) fn dot(a: P, b: P) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

pub(crate) fn at(o: P, d: P, t: f64) -> P {
    (o.0 + t * d.0, o.1 + t * d.1)
}

pub(crate) fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

pub(crate) fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).stable_hypot(a.1 - b.1)
}

pub(crate) fn bounds(q: &[P]) -> (P, P) {
    q.iter().fold(((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY)), |(lo, hi), &(x, y)| {
        ((lo.0.min(x), lo.1.min(y)), (hi.0.max(x), hi.1.max(y)))
    })
}

pub(crate) fn dist_to_seg(q: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((q.0 - a.0) * dx + (q.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist(q, (a.0 + t * dx, a.1 + t * dy))
}

fn cross(o: P, a: P, b: P) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// The distance between two segments: 0 where they cross.
pub(crate) fn seg_seg_dist(p1: P, p2: P, q1: P, q2: P) -> f64 {
    let (d1, d2) = (cross(q1, q2, p1), cross(q1, q2, p2));
    let (d3, d4) = (cross(p1, p2, q1), cross(p1, p2, q2));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return 0.0;
    }
    dist_to_seg(p1, q1, q2).min(dist_to_seg(p2, q1, q2)).min(dist_to_seg(q1, p1, p2)).min(dist_to_seg(q2, p1, p2))
}

/// Whether `q` is strictly inside a convex quad, either winding.
fn inside_convex(c: &[P; 4], q: P) -> bool {
    let s: Vec<f64> = (0..4).map(|i| cross(c[i], c[(i + 1) % 4], q)).collect();
    s.iter().all(|&x| x > 0.0) || s.iter().all(|&x| x < 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::terrain::{Mouth, RoadType, WaterPoint};
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn a_ray_enters_a_capsule_at_its_edge() {
        let (a, b) = ((0.0, 10.0), (100.0, 10.0));
        // Straight up into the side, at x = 50: the edge is at y = 6.
        assert!((capsule_entry((50.0, 0.0), (0.0, 1.0), 0.0, a, b, 4.0).unwrap() - 6.0).abs() < 1e-9);
        // Into the round end past x = 100.
        let t = capsule_entry((102.0, 0.0), (0.0, 1.0), 0.0, a, b, 4.0).unwrap();
        assert!((t - (10.0 - 12f64.sqrt())).abs() < 1e-9);
        // Away from it, and past it.
        assert_eq!(capsule_entry((50.0, 0.0), (0.0, -1.0), 0.0, a, b, 4.0), None);
        assert_eq!(capsule_entry((150.0, 0.0), (0.0, 1.0), 0.0, a, b, 4.0), None);
        // Starting inside.
        assert_eq!(capsule_entry((50.0, 8.0), (0.0, 1.0), 0.0, a, b, 4.0), Some(0.0));
    }

    #[test]
    fn segments_measure_their_gap() {
        assert_eq!(seg_seg_dist((0.0, 0.0), (10.0, 10.0), (0.0, 10.0), (10.0, 0.0)), 0.0);
        assert!((seg_seg_dist((0.0, 0.0), (10.0, 0.0), (5.0, 3.0), (5.0, 9.0)) - 3.0).abs() < 1e-12);
        assert!((seg_seg_dist((0.0, 0.0), (10.0, 0.0), (13.0, 4.0), (20.0, 4.0)) - 5.0).abs() < 1e-12);
    }

    const PARAMS: LotParams = LotParams {
        frontage: 100.0,
        depth: 100.0,
        depth_max: 120.0,
        front_gap: 0.05,
        edge_gap: 0.05,
        rear_gap: 0.25,
        lot_gap: 0.0,
        min_shrink: 0.6,
        min_area: 0.45,
    };

    /// A road along y = 0 from x = −1000 to 1000, with lots on its left
    /// (y > 0), in a world round the origin.
    fn road() -> Vec<RoadSegment> {
        vec![RoadSegment {
            id: "lane_0".into(),
            road_type: RoadType::Trail,
            points: vec![(-1000.0, 0.0), (1000.0, 0.0)],
            width: 10.0,
        }]
    }

    fn course(points: &[(f64, f64)], width: f64) -> WaterCourse {
        WaterCourse {
            id: "stream_0".into(),
            source: None,
            mouth: Mouth::PetersOut,
            points: points.iter().map(|&(x, y)| WaterPoint { x, y, width, depth: 1.0, surface: 0.0 }).collect(),
        }
    }

    fn square(x: f64, y: f64, half: f64) -> Vec<P> {
        vec![(x - half, y - half), (x + half, y - half), (x + half, y + half), (x - half, y + half)]
    }

    /// A lake round (0, 300), 200 across, with an island 100 across in it.
    fn lake() -> Lake {
        Lake { id: "lake_0".into(), level: 0.0, rings: vec![square(0.0, 300.0, 100.0), square(0.0, 300.0, 50.0)], area: 30000.0 }
    }

    /// A lot `w` wide from `x` along the road, `d` deep, at `y0` off it.
    fn lot(id: &str, x: f64, y0: f64, w: f64, d: f64) -> Candidate {
        Candidate { id: id.into(), road: 0, corners: [(x, y0), (x + w, y0), (x + w, y0 + d), (x, y0 + d)] }
    }

    fn ids(lots: &[TerrainPlot]) -> Vec<&str> {
        lots.iter().map(|l| l.id.as_str()).collect()
    }

    #[test]
    fn a_lot_is_refused_across_a_stream_and_backs_onto_one_behind_it() {
        let roads = road();
        // A stream 8 wide across the lots' ground at x = 50, and one along
        // y = 90 behind them further on.
        let streams = [course(&[(50.0, -20.0), (50.0, 400.0)], 8.0), course(&[(400.0, 90.0), (800.0, 90.0)], 8.0)];
        let lakes = [];
        let world = World::new(&roads, vec![(true, false)], (0.0, 0.0), 2000.0).with_water(&streams, &lakes, 5.0);
        let laid = place([lot("across", 0.0, 5.05, 100.0, 100.0), lot("behind", 500.0, 5.05, 100.0, 100.0)], &world, &PARAMS, "rural");
        assert_eq!(ids(&laid), ["behind"], "the stream runs through the first from front to back");
        // Shrunk to the bank: the stream's edge is at 86, and the bank 5 more.
        let rear = laid[0].corners[2].1;
        assert!(rear <= 81.0 && rear > 75.0, "rear at {rear}");
        // A ray stops at the bank as at a road without lots, short by the edge gap.
        let region = Region::ring((0.0, 0.0), 0.0, f64::INFINITY);
        let b = world.back(&region, (600.0, 0.0), (0.0, 1.0), 5.05, &PARAMS);
        assert!((b - (81.0 - PARAMS.edge_gap)).abs() < 1e-9, "back at {b}");
        // Clear of it, a ray takes its usual depth.
        assert!((world.back(&region, (-500.0, 0.0), (0.0, 1.0), 5.05, &PARAMS) - 105.05).abs() < 1e-9);
        // The bridge's ends: a front in the stream's bank is no front.
        assert!(!world.point_clear((52.0, 5.05)) && world.point_clear((70.0, 5.05)));
    }

    #[test]
    fn a_lot_is_refused_in_a_lake_and_allowed_on_its_island() {
        let roads = road();
        let lakes = [lake()];
        let world = World::new(&roads, vec![(true, false)], (0.0, 0.0), 2000.0).with_water([], &lakes, 5.0);
        let laid = place(
            [
                // Out on the water between the shore and the island.
                lot("water", -90.0, 210.0, 30.0, 30.0),
                // Across the shore.
                lot("shore", -150.0, 180.0, 60.0, 60.0),
                // On the island, clear of its shore by the bank.
                lot("island", -40.0, 260.0, 80.0, 80.0),
                // On the island, but within the bank of its shore.
                lot("wet_feet", -48.0, 260.0, 40.0, 40.0),
                // Round a whole pond: none of its corners is in it.
                lot("round", -120.0, 160.0, 240.0, 280.0),
            ],
            &world,
            &PARAMS,
            "rural",
        );
        assert_eq!(ids(&laid), ["island"]);
        assert!(world.point_clear((0.0, 300.0)), "the island is dry");
        assert!(!world.point_clear((0.0, 220.0)), "the water isn't");
    }

    #[test]
    fn lots_keep_their_gap() {
        let roads = road();
        let world = World::new(&roads, vec![(true, false)], (0.0, 0.0), 2000.0);
        let apart = LotParams { lot_gap: 40.0, ..PARAMS };
        let lots = || [lot("a", 0.0, 5.05, 100.0, 100.0), lot("b", 130.0, 5.05, 100.0, 100.0), lot("c", 240.0, 5.05, 100.0, 100.0)];
        assert_eq!(ids(&place(lots(), &world, &PARAMS, "rural")), ["a", "b", "c"]);
        assert_eq!(ids(&place(lots(), &world, &apart, "rural")), ["a", "c"], "b is 30 from a");
    }

    #[test]
    fn a_region_is_an_annulus_cut_by_sides_or_a_polygon() {
        let sector = Region::ring((0.0, 0.0), 100.0, 200.0).ccw_of(0.0, 10.0).cw_of(FRAC_PI_2, 10.0);
        assert!(sector.inside((100.0, 100.0)));
        assert!(!sector.inside((150.0, 5.0)), "within the inset of the x axis");
        assert!(!sector.inside((5.0, 150.0)), "within the inset of the y axis");
        assert!(!sector.inside((60.0, 60.0)) && !sector.inside((150.0, 150.0)));
        // Out along x from (110, 50): the outer circle at x = sqrt(200² − 50²).
        assert!((sector.exit((110.0, 50.0), (1.0, 0.0), 0.0) - (37500f64.sqrt() - 110.0)).abs() < 1e-9);

        // A field with a pond in it, even-odd.
        let field = Region::polygon((0.0, 0.0), vec![square(0.0, 0.0, 100.0), square(0.0, 0.0, 20.0)]);
        assert!(field.inside((50.0, 0.0)) && !field.inside((0.0, 0.0)) && !field.inside((150.0, 0.0)));
        assert!((field.exit((50.0, 0.0), (1.0, 0.0), 0.0) - 50.0).abs() < 1e-9);
        assert!((field.exit((50.0, 0.0), (-1.0, 0.0), 0.0) - 30.0).abs() < 1e-9, "at the pond's edge");
    }
}
