//! Lots v2 (plan §4.2): blocks filled by back-to-back lots.
//!
//! A walker over the welded roads, using each street's `StreetMeta`:
//! - **Rays.** Along each fronted side, a lot's side edges are *rays*: from
//!   just off the road's edge, out along the road's normal. Neighbouring lots
//!   share a ray, so a run of lots tiles exactly.
//! - **Runs.** A side splits into runs wherever a ray can't start: its front
//!   point is on another road (a rail, a T's stem), outside the
//!   neighbourhood's side lines or the zone, or too shallow to hold a lot.
//!   A rail fronts no lots, so the rows own the corners at each T. Where a
//!   fronted street's end meets another fronted street partway along it, the
//!   through road owns the corner: the end's first lot depth is left to it.
//! - **Widths.** A run is measured at mid-depth, so a lot on a curve becomes
//!   a pie, and is cut into `n = round(L / frontage)` equal widths there.
//! - **Depth.** A ray-march to the first road or the neighbourhood's edge. A
//!   fronted road within `2·lot_depth_max` gives half the distance, so lots
//!   back to back share a rear line. Otherwise `min(hit, lot_depth)`, or up
//!   to `lot_depth_max` where the ray leaves the neighbourhood within
//!   `2·lot_depth_max`, to take up the edge strips. On the
//!   concave side of a curve, the ray stops at `0.85/κ` from the centre line.
//! - **Bulbs.** 4–5 pie lots round each, on rays from the bulb centre, between
//!   the last rays of the stem's two sides.
//! - **Overlap pass.** Pies first, then through roads, then cul-de-sac
//!   stems. A lot that overlaps one placed before it (`quads_overlap_strict`),
//!   or isn't clear of every road, shrinks by bisection down to 60% depth, or
//!   is dropped. Pies go first, not last as the plan first had it: a row's
//!   ray passing beside a bulb finds nothing behind it and runs a full lot
//!   deep into the pies' ground. A rectangular row lot gives that back
//!   easily, but a pie shrunk that far falls under the area floor.
//!   `finish_lots` then gets lots it needn't touch.
//! - **Ids.** `sub_{sector}_{nbhd}_{street}_{L|R|B}_{k}`, where `street` is
//!   the street's place among its neighbourhood's, and B marks bulb pies.
//!   They're numbered before the overlap pass, so dropping one lot doesn't
//!   re-id the rest.

use capitol_protocol::fmath::StableMath;
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, TAU};

use capitol_protocol::terrain::{RoadSegment, TerrainPlot};

use super::neighbourhood::Neighbourhood;
use super::patterns::{Role, StreetMeta};
use super::SuburbPlan;
use crate::models::geom::{quad_is_convex, quads_overlap_strict, Polyline, SpatialGrid};
use crate::models::terrain::{NeighbourhoodParams, SuburbanGenParams};

type P = (f64, f64);

/// Ray spacing when finding a side's runs.
const SAMPLE: f64 = 5.0;
/// Lots stand this far off a road's edge...
const FRONT_GAP: f64 = 0.05;
/// ...stop this short of an edge they back onto...
const EDGE_GAP: f64 = 0.05;
/// ...and this short of the line they share with lots back to back.
const REAR_GAP: f64 = 0.25;
/// A lot may touch a road's edge, within this.
const TOUCH: f64 = 0.01;
/// On a curve's concave side a ray stops at this share of the radius.
const CONCAVE: f64 = 0.85;
/// The overlap pass shrinks a lot to no less than this share of its depth.
const MIN_SHRINK: f64 = 0.6;
/// A ray shallower than this share of `lot_depth` starts no lot...
const MIN_DEPTH: f64 = 0.5;
/// ...a run narrower than this share of `lot_frontage` holds none...
const MIN_RUN: f64 = 0.6;
/// ...and a lot under this share of frontage × depth is dropped.
const MIN_AREA: f64 = 0.45;
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
/// A stem's run ending this near its bulb hands its last ray to the pies.
const BULB_REACH: f64 = 40.0;
/// A run's end ray follows a road's edge within this of its front...
const HUG_REACH: f64 = 1.0;
/// ...if it's within about 40° of the ray.
const HUG_COS: f64 = 0.76;
/// The most pies round a bulb, and the fewest.
const PIES: (usize, usize) = (5, 2);
/// A pie smaller than this share of frontage × depth is too small.
const MIN_PIE: f64 = 0.65;
/// A stem's lots stop this far short of its bulb, so the pies wrap round it.
const STEM_SHORT: f64 = 20.0;

/// The suburban lots for a Neighbourhood plan, before `finish_lots`. `roads`
/// is the whole world's, welded; `plan` the one they were laid from.
pub(crate) fn lay(roads: &[RoadSegment], plan: &SuburbPlan, params: &SuburbanGenParams, center: P) -> Vec<TerrainPlot> {
    let p = &params.neighbourhood;
    let by_id: HashMap<&str, usize> = roads.iter().enumerate().map(|(i, r)| (r.id.as_str(), i)).collect();
    let meta: Vec<Option<&StreetMeta>> = {
        let mut m = vec![None; roads.len()];
        for s in &plan.meta {
            if let Some(&i) = by_id.get(s.id.as_str()) {
                m[i] = Some(s);
            }
        }
        m
    };
    let world = World::new(roads, &meta, center, params);

    let mut street_no: HashMap<(u8, u8), usize> = HashMap::new();
    let mut candidates = Vec::new();
    for m in &plan.meta {
        let next = street_no.entry((m.sector, m.nbhd)).or_default();
        let street = *next;
        *next += 1;
        let (Some(&road), Some(n)) = (
            by_id.get(m.id.as_str()),
            plan.neighbourhoods.iter().find(|n| (n.sector, n.index) == (m.sector, m.nbhd)),
        ) else {
            continue;
        };
        if m.fronted != (false, false) {
            let region = Region::of(n, params, center);
            candidates.extend(Street { world: &world, road, meta: m, street, region, p }.lots());
        }
    }
    // Stable, so each class keeps plan order.
    candidates.sort_by_key(|c| c.class);
    place(candidates, &world, p)
}

/// A lot before the overlap pass.
struct Candidate {
    /// The overlap pass's order: 0 for pies, 1 for through roads, 2 for
    /// cul-de-sac stems.
    class: u8,
    id: String,
    road: usize,
    /// Street-left, street-right, rear-right, rear-left.
    corners: [P; 4],
}

/// One stretch of a road's centre line, with the road's half width.
struct Seg {
    a: P,
    b: P,
    half: f64,
    road: usize,
}

/// Every road a lot must stay out of, and which of them have lots.
struct World<'a> {
    roads: &'a [RoadSegment],
    segs: Vec<Seg>,
    grid: SpatialGrid,
    max_half: f64,
    /// Which sides of each road have lots, left and right along its points.
    /// Pattern (c)'s loop has lots only inside (#110).
    sides: Vec<(bool, bool)>,
    /// Each fronted road's vertices, so a street's end can find a road it
    /// meets partway along.
    vertices: HashMap<(u64, u64), Vec<(usize, usize)>>,
    center: P,
    outer: f64,
}

fn key(p: P) -> (u64, u64) {
    (p.0.to_bits(), p.1.to_bits())
}

impl<'a> World<'a> {
    fn new(roads: &'a [RoadSegment], meta: &[Option<&StreetMeta>], center: P, params: &SuburbanGenParams) -> Self {
        let outer = params.outer_radius;
        let mut grid = SpatialGrid::new(center.0, center.1, outer + 2.0 * params.neighbourhood.lot_depth_max, 40.0);
        let mut segs = Vec::new();
        for (road, r) in roads.iter().enumerate() {
            for w in r.points.windows(2) {
                grid.insert(segs.len(), w);
                segs.push(Seg { a: w[0], b: w[1], half: r.width / 2.0, road });
            }
        }
        let max_half = roads.iter().map(|r| r.width / 2.0).fold(0.0, f64::max);
        let sides: Vec<(bool, bool)> = meta.iter().map(|m| m.map_or((false, false), |m| m.fronted)).collect();
        let fronted: Vec<bool> = sides.iter().map(|&s| s != (false, false)).collect();
        let mut vertices: HashMap<(u64, u64), Vec<(usize, usize)>> = HashMap::new();
        for (road, r) in roads.iter().enumerate().filter(|&(i, _)| fronted[i]) {
            for (i, &q) in r.points.iter().enumerate() {
                vertices.entry(key(q)).or_default().push((road, i));
            }
        }
        Self { roads, segs, grid, max_half, sides, vertices, center, outer }
    }

    fn near(&self, lo: P, hi: P) -> impl Iterator<Item = &Seg> {
        let m = self.max_half;
        self.grid.query_aabb(lo.0 - m, lo.1 - m, hi.0 + m, hi.1 + m).into_iter().map(|i| &self.segs[i])
    }

    /// Whether `q` is clear of every road.
    fn point_clear(&self, q: P) -> bool {
        self.near(q, q).all(|s| dist_to_seg(q, s.a, s.b) >= s.half - TOUCH)
    }

    /// Whether the segment `a`..`b` is clear of every road.
    fn edge_clear(&self, a: P, b: P) -> bool {
        let (lo, hi) = bounds(&[a, b]);
        self.near(lo, hi).all(|s| seg_seg_dist(a, b, s.a, s.b) >= s.half - TOUCH)
    }

    /// Whether a convex quad is clear of every road.
    fn quad_clear(&self, q: &[P; 4]) -> bool {
        let (lo, hi) = bounds(q);
        self.near(lo, hi).all(|s| {
            !inside_convex(q, s.a) && (0..4).all(|i| seg_seg_dist(q[i], q[(i + 1) % 4], s.a, s.b) >= s.half - TOUCH)
        })
    }

    /// The first road the ray `o + t·d` enters after `t0` and by `limit`,
    /// and whether it has lots on the side the ray comes from.
    fn first_hit(&self, o: P, d: P, t0: f64, limit: f64) -> Option<(f64, usize, bool)> {
        let (a, b) = (at(o, d, t0), at(o, d, limit));
        let (lo, hi) = bounds(&[a, b]);
        self.near(lo, hi)
            .filter_map(|s| {
                capsule_entry(o, d, t0, s.a, s.b, s.half).map(|t| (t, s.road, self.fronts(s.road, s.a, s.b, o)))
            })
            .filter(|&(t, _, _)| t <= limit)
            .min_by(|x, y| x.0.total_cmp(&y.0))
    }

    /// Whether `road` has lots on the side of its stretch `a`..`b` that `q` is on.
    fn fronts(&self, road: usize, a: P, b: P, q: P) -> bool {
        let (l, r) = self.sides[road];
        let left = (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0) > 0.0;
        if left {
            l
        } else {
            r
        }
    }

    /// How far out the ray from `o` along `d`, starting at `f`, can go.
    fn back(&self, region: &Region, o: P, d: P, f: f64, p: &NeighbourhoodParams) -> f64 {
        self.back_from(region, o, d, f, f, p.lot_depth, p)
    }

    /// `back`, with lots back to back meeting half way between `near` and
    /// the other road, and `open` deep onto open ground inside the
    /// neighbourhood. A pie's ray starts at its bulb's edge, but its half
    /// way is measured from a street's edge like the stem lots it backs
    /// onto, or it reaches past their shared rear line. Pies facing the
    /// middle of a pattern (b) band take `lot_depth_max`, to fill it.
    #[allow(clippy::too_many_arguments)]
    fn back_from(&self, region: &Region, o: P, d: P, f: f64, near: f64, open: f64, p: &NeighbourhoodParams) -> f64 {
        let exit = region.exit(o, d, f);
        match self.first_hit(o, d, f, exit.min(f + 2.0 * p.lot_depth_max)) {
            // Back to back: half way to the lots fronting the other road.
            Some((t, _, true)) => ((near + t) / 2.0 - REAR_GAP).max(f),
            Some((t, _, false)) => (t - EDGE_GAP).min(f + p.lot_depth),
            // Open ground to the neighbourhood's edge: as deep as a lot may
            // be, to take up the edge strip.
            None if exit - f <= 2.0 * p.lot_depth_max => (exit - EDGE_GAP).min(f + p.lot_depth_max),
            // Open ground inside it is for other streets' lots, such as a
            // comb's beside the racetrack (#109): a lot's usual depth.
            None => f + open,
        }
    }

    /// The points of a fronted street that carries on from `road`'s point
    /// `end` (its first or last), walking away from it, up to `LEAD` along.
    fn lead(&self, road: usize, end: usize) -> Vec<P> {
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
    /// beside this road and find open ground inside the neighbourhood, so
    /// they're `lot_depth` deep (pattern (b)'s combs, #109). A road with no
    /// lots on this road's side owns nothing (pattern (c)'s loop, #110).
    fn owned_ends(&self, road: usize, length: f64, p: &NeighbourhoodParams) -> Vec<(f64, f64)> {
        let pts = &self.roads[road].points;
        let n = pts.len();
        let mut out = Vec::new();
        for (end, next, s) in [(pts[0], pts[1], 0.0), (pts[n - 1], pts[n - 2], length)] {
            let through = self.vertices.get(&key(end)).into_iter().flatten().find(|&&(r, i)| {
                let rp = &self.roads[r].points;
                r != road && i > 0 && i + 1 < rp.len() && self.fronts(r, rp[i - 1], rp[i + 1], next)
            });
            if let Some(&(r, _)) = through {
                let len = self.roads[r].width / 2.0 + p.lot_depth + OWNED_SLACK;
                out.push(if s == 0.0 { (0.0, len) } else { (length - len, length) });
            }
        }
        out
    }
}

/// Where a neighbourhood's lots may go: between its side lines, in the zone.
struct Region {
    center: P,
    /// Each side line as a half-plane `n·(q − center) ≥ c`.
    sides: [(P, f64); 2],
    inner: f64,
    outer: f64,
}

impl Region {
    fn of(n: &Neighbourhood, params: &SuburbanGenParams, center: P) -> Self {
        let (lo, hi) = (n.frame.lo, n.frame.hi);
        Region {
            center,
            sides: [
                ((-lo.angle.stable_sin(), lo.angle.stable_cos()), lo.inset),
                ((hi.angle.stable_sin(), -hi.angle.stable_cos()), hi.inset),
            ],
            inner: params.inner_radius,
            outer: params.outer_radius,
        }
    }

    fn inside(&self, q: P) -> bool {
        let rel = (q.0 - self.center.0, q.1 - self.center.1);
        let r = rel.0.stable_hypot(rel.1);
        self.sides.iter().all(|&(n, c)| dot(n, rel) >= c - 1e-9) && r >= self.inner && r <= self.outer
    }

    /// Where the ray `o + t·d`, inside at `t0`, leaves.
    fn exit(&self, o: P, d: P, t0: f64) -> f64 {
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
            let disc = b * b - (q - r * r);
            if disc >= 0.0 {
                let t = if entering { -b - disc.sqrt() } else { -b + disc.sqrt() };
                if t >= t0 {
                    best = best.min(t);
                }
            }
        }
        best
    }
}

/// A lot's side edge: from `o + f·d` out to `o + b·d`.
#[derive(Debug, Clone, Copy)]
struct Ray {
    o: P,
    d: P,
    f: f64,
    b: f64,
    /// Shared with lots already made, so it mustn't move.
    fixed: bool,
}

impl Ray {
    fn front(&self) -> P {
        at(self.o, self.d, self.f)
    }

    fn back(&self) -> P {
        at(self.o, self.d, self.b)
    }

    fn mid(&self) -> P {
        at(self.o, self.d, (self.f + self.b) / 2.0)
    }
}

/// A ray `t` of the way from `a` to `b`. Sample rays are a few units apart,
/// so this is as good as casting one there, and it can't land in a shadow.
fn blend(a: &Ray, b: &Ray, t: f64) -> Ray {
    let front = lerp(a.front(), b.front(), t);
    let back = lerp(a.back(), b.back(), t);
    let len = dist(front, back);
    Ray { o: front, d: ((back.0 - front.0) / len, (back.1 - front.1) / len), f: 0.0, b: len, fixed: false }
}

/// The lot between two rays: street-left, street-right, rear-right, rear-left.
fn quad(a: &Ray, b: &Ray) -> [P; 4] {
    [a.front(), b.front(), b.back(), a.back()]
}

/// Push fronts out until each lot's front edge is clear of the roads: on a
/// curve's convex side the chord between two rays cuts into the road.
fn push_fronts(rays: &mut [Ray], world: &World) {
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

/// One street's lots.
struct Street<'a> {
    world: &'a World<'a>,
    road: usize,
    meta: &'a StreetMeta,
    street: usize,
    region: Region,
    p: &'a NeighbourhoodParams,
}

impl Street<'_> {
    fn lots(&self) -> Vec<Candidate> {
        let pts = &self.world.roads[self.road].points;
        // A cul-de-sac's stem ends where its bulb closes on it.
        let stem_end = match self.meta.bulb {
            Some(_) => (0..pts.len() - 1).find(|&i| pts[i] == pts[pts.len() - 1]).unwrap_or(pts.len() - 1),
            None => pts.len() - 1,
        };
        // The line runs on a little into any fronted street this one
        // continues, so the two cast the same ray where they meet.
        let stem = &pts[..=stem_end];
        let mut before = self.world.lead(self.road, 0);
        before.reverse();
        let after = if self.meta.bulb.is_none() { self.world.lead(self.road, pts.len() - 1) } else { Vec::new() };
        let lo = Polyline::new(before.iter().chain(&stem[..1]).copied().collect()).length();
        let line = Polyline::new(before.into_iter().chain(stem.iter().copied()).chain(after).collect());
        let hi = lo + Polyline::new(stem.to_vec()).length();
        let mut owned: Vec<(f64, f64)> =
            self.world.owned_ends(self.road, hi - lo, self.p).into_iter().map(|(a, b)| (a + lo, b + lo)).collect();
        if self.meta.bulb.is_some() {
            owned.push((hi - STEM_SHORT, hi));
        }
        let class = if self.meta.role == Role::CulDeSac { 2 } else { 1 };

        let mut out = Vec::new();
        let mut last = [None, None];
        for (si, fronted) in [self.meta.fronted.0, self.meta.fronted.1].into_iter().enumerate() {
            if !fronted {
                continue;
            }
            let side = [1.0, -1.0][si];
            let mut k = 0;
            for (s0, s1) in self.runs(&line, (lo, hi), side, &owned) {
                let rays = self.lay_run(&line, (lo, hi), side, s0, s1);
                for w in rays.windows(2) {
                    out.push(self.candidate(class, ["L", "R"][si], k, quad(&w[0], &w[1])));
                    k += 1;
                }
                if s1 >= hi - BULB_REACH {
                    last[si] = rays.last().copied();
                }
            }
        }
        if let Some(centre) = self.meta.bulb {
            let stem_tip = pts[stem_end];
            out.extend(self.pies(centre, stem_tip, last));
        }
        out
    }

    fn candidate(&self, class: u8, side: &str, k: usize, corners: [P; 4]) -> Candidate {
        let m = self.meta;
        Candidate {
            class,
            id: format!("sub_{}_{}_{}_{side}_{k}", m.sector, m.nbhd, self.street),
            road: self.road,
            corners,
        }
    }

    fn half(&self) -> f64 {
        self.world.roads[self.road].width / 2.0
    }

    /// The ray at `s` along the line on `side` (1 left, −1 right), if its
    /// front is clear. It may still be too shallow for a lot (`deep`).
    fn ray(&self, line: &Polyline, side: f64, s: f64) -> Option<Ray> {
        let (o, tangent) = (point(line, s), tangent(line, s));
        let d = (-tangent.1 * side, tangent.0 * side);
        let f = self.half() + FRONT_GAP;
        let front = at(o, d, f);
        if !self.region.inside(front) || !self.world.point_clear(front) {
            return None;
        }
        let mut b = self.world.back(&self.region, o, d, f, self.p);
        // Positive κ turns left, so the left side is concave.
        let k = curvature(line, s) * side;
        if k > 0.0 {
            b = b.min(CONCAVE / k);
        }
        Some(Ray { o, d, f, b, fixed: false })
    }

    fn deep(&self, r: &Ray) -> bool {
        r.b - r.f >= MIN_DEPTH * self.p.lot_depth
    }

    /// The stretches of the line where lots can stand on `side`, each from
    /// its first good ray to its last.
    fn runs(&self, line: &Polyline, (lo, hi): (f64, f64), side: f64, owned: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let good = |s: f64| !owned.iter().any(|&(a, b)| s >= a && s <= b) && self.ray(line, side, s).is_some();
        // The boundary between a bad `bad` and a good `ok`, on the good side.
        let edge = |mut bad: f64, mut ok: f64| {
            for _ in 0..8 {
                let mid = (bad + ok) / 2.0;
                if good(mid) {
                    ok = mid;
                } else {
                    bad = mid;
                }
            }
            ok
        };
        let n = ((hi - lo) / SAMPLE).ceil().max(1.0) as usize;
        let samples: Vec<f64> = (0..=n).map(|i| lo + (hi - lo) * i as f64 / n as f64).collect();
        let flags: Vec<bool> = samples.iter().map(|&s| good(s)).collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < samples.len() {
            if !flags[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i + 1 < samples.len() && flags[i + 1] {
                i += 1;
            }
            let s0 = if start > 0 { edge(samples[start - 1], samples[start]) } else { samples[start] };
            let s1 = if i + 1 < samples.len() { edge(samples[i + 1], samples[i]) } else { samples[i] };
            if s1 > s0 {
                out.push((s0, s1));
            }
            i += 1;
        }
        out
    }

    /// A run's rays, spaced so its lots are equally wide at mid-depth.
    fn lay_run(&self, line: &Polyline, (lo, hi): (f64, f64), side: f64, s0: f64, s1: f64) -> Vec<Ray> {
        let n = ((s1 - s0) / SAMPLE).ceil().max(1.0) as usize;
        let mut samples: Vec<Ray> = (0..=n)
            .filter_map(|i| self.ray(line, side, s0 + (s1 - s0) * i as f64 / n as f64))
            .collect();
        if samples.len() < 2 {
            return Vec::new();
        }
        // An end against a road turns to follow its edge. The rays it
        // shadows, which would cross it, go.
        let hugs = [
            (s0 > lo).then(|| self.hug(samples[0])).flatten(),
            (s1 < hi).then(|| self.hug(*samples.last().unwrap())).flatten(),
        ];
        let crosses = |r: &Ray, h: &Option<Ray>| {
            h.is_some_and(|h| seg_seg_dist(r.front(), r.back(), h.front(), h.back()) <= 1e-9)
        };
        let last = samples.len() - 1;
        let kept: Vec<Ray> = samples
            .drain(..)
            .enumerate()
            .filter(|&(i, r)| {
                !(i == 0 && hugs[0].is_some())
                    && !(i == last && hugs[1].is_some())
                    && self.deep(&r)
                    && !crosses(&r, &hugs[0])
                    && !crosses(&r, &hugs[1])
            })
            .map(|(_, r)| r)
            .collect();
        let samples: Vec<Ray> = hugs[0].into_iter().chain(kept).chain(hugs[1]).collect();
        if samples.len() < 2 {
            return Vec::new();
        }
        let mut cum = vec![0.0];
        for w in samples.windows(2) {
            let last = *cum.last().unwrap();
            cum.push(last + dist(w[0].mid(), w[1].mid()));
        }
        let total = *cum.last().unwrap();
        if total < MIN_RUN * self.p.lot_frontage {
            return Vec::new();
        }
        let lots = (total / self.p.lot_frontage).round().max(1.0) as usize;
        let mut rays: Vec<Ray> = (0..=lots)
            .map(|j| {
                let want = total * j as f64 / lots as f64;
                let i = cum.partition_point(|&c| c < want).clamp(1, cum.len() - 1);
                let span = cum[i] - cum[i - 1];
                let t = if span > 0.0 { (want - cum[i - 1]) / span } else { 0.0 };
                match t {
                    t if t <= 1e-9 => samples[i - 1],
                    t if t >= 1.0 - 1e-9 => samples[i],
                    t => blend(&samples[i - 1], &samples[i], t),
                }
            })
            .collect();
        push_fronts(&mut rays, self.world);
        rays
    }

    /// A run's end ray, turned to run along the edge of the road that ends
    /// the run, such as a slanted rail, so the end lot isn't a wedge.
    fn hug(&self, ray: Ray) -> Option<Ray> {
        let front = ray.front();
        let w = self.world;
        let beside = w
            .near(front, front)
            .filter(|s| s.road != self.road)
            .map(|s| (dist_to_seg(front, s.a, s.b) - s.half, s))
            .filter(|&(gap, _)| gap < HUG_REACH)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let (gap, s) = beside?;
        let len = dist(s.a, s.b);
        if len <= 0.0 {
            return None;
        }
        let mut e = ((s.b.0 - s.a.0) / len, (s.b.1 - s.a.1) / len);
        if dot(e, ray.d) < 0.0 {
            e = (-e.0, -e.1);
        }
        if dot(e, ray.d) < HUG_COS {
            return None;
        }
        // Off the edge by a hair more than the front already is.
        let foot = {
            let (dx, dy) = (s.b.0 - s.a.0, s.b.1 - s.a.1);
            let t = (((front.0 - s.a.0) * dx + (front.1 - s.a.1) * dy) / (len * len)).clamp(0.0, 1.0);
            (s.a.0 + t * dx, s.a.1 + t * dy)
        };
        let off = dist(front, foot);
        if off <= 0.0 {
            return None;
        }
        let away = ((front.0 - foot.0) / off, (front.1 - foot.1) / off);
        let o = at(front, away, (FRONT_GAP - gap).max(0.0));
        if !self.region.inside(o) || !w.point_clear(o) {
            return None;
        }
        let b = w.back(&self.region, o, e, 0.0, self.p);
        let hug = Ray { o, d: e, f: 0.0, b, fixed: true };
        self.deep(&hug).then_some(hug)
    }

    /// Pie lots round a bulb at `centre`, whose stem ends at `tip`, between
    /// the last rays of the stem's left and right runs.
    fn pies(&self, centre: P, tip: P, last: [Option<Ray>; 2]) -> Vec<Candidate> {
        let toward = (centre.1 - tip.1).stable_atan2(centre.0 - tip.0);
        let angle = |r: &Ray| {
            let m = r.mid();
            (m.1 - centre.1).stable_atan2(m.0 - centre.0)
        };
        let from = last[0].as_ref().map_or(toward + FRAC_PI_2, angle);
        let to = last[1].as_ref().map_or(toward - FRAC_PI_2, angle);
        // Clockwise from the left round the bulb's far side to the right.
        let sweep = (from - to).rem_euclid(TAU);
        if sweep < 1e-3 {
            return Vec::new();
        }
        let f = self.p.bulb_radius + self.half() + FRONT_GAP;
        let radial = |phi: f64| -> Option<Ray> {
            let d = (phi.stable_cos(), phi.stable_sin());
            let front = at(centre, d, f);
            if !self.region.inside(front) || !self.world.point_clear(front) {
                return None;
            }
            let b = self.world.back_from(&self.region, centre, d, f, self.half() + FRONT_GAP, self.p.lot_depth_max, self.p);
            (b - f >= MIN_DEPTH * self.p.lot_depth).then_some(Ray { o: centre, d, f, b, fixed: false })
        };
        let fixed = |r: Option<Ray>| r.map(|r| Ray { fixed: true, ..r });
        let fan = |count: usize| -> Vec<Option<Ray>> {
            (0..=count)
                .map(|j| match j {
                    0 => fixed(last[0]).or_else(|| radial(from)),
                    j if j == count => fixed(last[1]).or_else(|| radial(to)),
                    j => radial(from - sweep * j as f64 / count as f64),
                })
                .collect()
        };
        // The most pies whose every lot is big enough. Narrow pies hold too
        // little ground, so a tight bulb takes fewer, wider ones.
        let least = MIN_PIE * self.p.lot_frontage * self.p.lot_depth;
        let big_enough = |rays: &[Option<Ray>]| {
            rays.windows(2).all(|w| match (w[0], w[1]) {
                (Some(a), Some(b)) => TerrainPlot::calculate_area(&quad(&a, &b)) >= least,
                _ => true,
            })
        };
        let mut rays = (PIES.1..=PIES.0).rev().map(fan).find(|r| big_enough(r)).unwrap_or_else(|| fan(PIES.1));
        // Fronts are pushed only along unbroken stretches of rays.
        let mut out = Vec::new();
        let mut j = 0;
        while j < rays.len() {
            let start = j;
            while j < rays.len() && rays[j].is_some() {
                j += 1;
            }
            let mut stretch: Vec<Ray> = rays[start..j].iter_mut().map(|r| r.take().unwrap()).collect();
            push_fronts(&mut stretch, self.world);
            for (k, w) in stretch.windows(2).enumerate() {
                out.push(self.candidate(0, "B", start + k, quad(&w[0], &w[1])));
            }
            j += 1;
        }
        out
    }
}

/// The overlap pass: candidates in priority order, each shrunk or dropped
/// to keep clear of the roads and of the lots before it.
fn place(candidates: Vec<Candidate>, world: &World, p: &NeighbourhoodParams) -> Vec<TerrainPlot> {
    let reach = world.outer + 2.0 * p.lot_depth_max;
    let mut grid = SpatialGrid::new(world.center.0, world.center.1, reach, 50.0);
    let mut placed: Vec<[P; 4]> = Vec::new();
    let min_area = MIN_AREA * p.lot_frontage * p.lot_depth;
    let mut out = Vec::new();
    for c in candidates {
        let ok = |q: &[P; 4]| {
            let (lo, hi) = bounds(q);
            quad_is_convex(q)
                && TerrainPlot::calculate_area(q) >= min_area
                && world.quad_clear(q)
                && grid.query_aabb(lo.0, lo.1, hi.0, hi.1).into_iter().all(|i| !quads_overlap_strict(q, &placed[i]))
        };
        let Some(corners) = shrink_to_fit(c.corners, ok) else { continue };
        grid.insert(placed.len(), &corners);
        placed.push(corners);
        out.push(TerrainPlot {
            id: c.id,
            corners,
            street_facing_edge: (0, 1),
            area: TerrainPlot::calculate_area(&corners),
            adjacent_street_id: world.roads[c.road].id.clone(),
            zone: "suburban".to_string(),
        });
    }
    out
}

/// The lot as it is if it's `ok`, or pulled back towards the street to the
/// deepest that is, down to `MIN_SHRINK` of its depth.
fn shrink_to_fit(c: [P; 4], ok: impl Fn(&[P; 4]) -> bool) -> Option<[P; 4]> {
    if ok(&c) {
        return Some(c);
    }
    let at = |k: f64| [c[0], c[1], lerp(c[1], c[2], k), lerp(c[0], c[3], k)];
    if !ok(&at(MIN_SHRINK)) {
        return None;
    }
    let (mut good, mut bad) = (MIN_SHRINK, 1.0);
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

fn point(line: &Polyline, s: f64) -> P {
    let (x, y, _) = line.at(s).expect("a line of two points or more");
    (x, y)
}

/// The line's direction at `s`, averaged over `TANGENT_REACH` either side so
/// a ray at a vertex doesn't take one segment's side.
fn tangent(line: &Polyline, s: f64) -> P {
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
fn curvature(line: &Polyline, s: f64) -> f64 {
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

fn dot(a: P, b: P) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

fn at(o: P, d: P, t: f64) -> P {
    (o.0 + t * d.0, o.1 + t * d.1)
}

fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).stable_hypot(a.1 - b.1)
}

fn bounds(q: &[P]) -> (P, P) {
    q.iter().fold(((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY)), |(lo, hi), &(x, y)| {
        ((lo.0.min(x), lo.1.min(y)), (hi.0.max(x), hi.1.max(y)))
    })
}

fn dist_to_seg(q: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((q.0 - a.0) * dx + (q.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist(q, (a.0 + t * dx, a.1 + t * dy))
}

fn cross(o: P, a: P, b: P) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// The distance between two segments: 0 where they cross.
fn seg_seg_dist(p1: P, p2: P, q1: P, q2: P) -> f64 {
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
    use crate::models::suburb::metrics;
    use crate::models::suburb::patterns::tests::parallels_world;
    use crate::models::terrain::TerrainConfig;

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

    fn suburban(t: &TerrainConfig) -> Vec<&TerrainPlot> {
        t.terrain_plots.iter().filter(|l| l.zone == "suburban").collect()
    }

    /// Acceptance for #108: no two lots overlap, suburban or otherwise, at
    /// the four report seeds.
    #[test]
    fn neighbourhood_lots_never_overlap() {
        for seed in [42, 7, 1234, 99999] {
            let t = TerrainConfig::from_params(&parallels_world(seed));
            let sub = suburban(&t);
            assert!(sub.len() > 600, "seed {seed}: {} suburban lots", sub.len());
            let mut grid = SpatialGrid::new(t.center_x, t.center_y, t.world_size, 50.0);
            for (i, l) in t.terrain_plots.iter().enumerate() {
                grid.insert(i, &l.corners);
            }
            for a in &sub {
                let (lo, hi) = bounds(&a.corners);
                for i in grid.query_aabb(lo.0, lo.1, hi.0, hi.1) {
                    let b = &t.terrain_plots[i];
                    assert!(
                        b.id == a.id || !quads_overlap_strict(&a.corners, &b.corners),
                        "seed {seed}: {} overlaps {}",
                        a.id,
                        b.id
                    );
                }
            }
        }
    }

    /// The plan's lot targets (§4.4), in a world of Parallels neighbourhoods.
    #[test]
    fn neighbourhood_lots_meet_their_targets() {
        for seed in [42, 7, 1234, 99999] {
            let params = parallels_world(seed);
            let m = metrics::measure_world(&params, None).measured.metrics;
            let why = format!("seed {seed}: {m:#?}");
            assert_eq!(m.overlaps, 0, "{why}");
            assert_eq!(m.convex_share, Some(1.0), "{why}");
            assert_eq!(m.arterial_fronting, 0, "{why}");
            assert!((6600.0..=7500.0).contains(&m.lot_area_median.unwrap()), "{why}");
            assert!(m.lot_area_p5.unwrap() >= 4000.0 && m.lot_area_p95.unwrap() <= 11000.0, "{why}");
            assert!((55.0..=65.0).contains(&m.lot_width_median.unwrap()), "{why}");
            assert!((100.0..=125.0).contains(&m.lot_depth_median.unwrap()), "{why}");
            assert!(m.trimmed_share.unwrap() <= 0.10, "{why}");
            assert!(m.dropped_share.unwrap() <= 0.03, "{why}");
            // The open ground left is parks (#111): what's too thin to use is the gap.
            assert!(m.cover_gap.unwrap() <= 0.05, "{why}");
            assert!(m.culdesac_max_lots.unwrap() <= 20.0, "{why}");
        }
    }

    #[test]
    fn lot_ids_name_their_street() {
        let t = TerrainConfig::from_params(&parallels_world(42));
        for l in suburban(&t) {
            let parts: Vec<&str> = l.id.split('_').collect();
            assert_eq!(parts.len(), 6, "{}", l.id);
            assert_eq!(parts[0], "sub");
            assert!(["L", "R", "B"].contains(&parts[4]), "{}", l.id);
            assert!(t.roads.iter().any(|r| r.id == l.adjacent_street_id), "{}", l.id);
        }
    }
}

