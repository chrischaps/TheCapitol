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
//!   The frontage is 100 since #422 (64 before), so most lots take 4 stations.
//!   With a `lot_width_min`, a run takes fewer lots rather than any narrower
//!   (#456: Plots v2's lots, which must be about 130 wide to take 8).
//! - **Depth.** A ray-march to the first road or the neighbourhood's edge. A
//!   fronted road within `2·lot_depth_max` gives half the distance, so lots
//!   back to back share a rear line. Otherwise `min(hit, lot_depth)`, or up
//!   to `lot_depth_max` where the ray leaves the neighbourhood within
//!   `2·lot_depth_max`, to take up the edge strips. On the
//!   concave side of a curve, the ray stops at `0.85/κ` from the centre line.
//! - **Bulbs.** 2–3 pie lots round each, on rays from the bulb centre, between
//!   the last rays of the stem's two sides: as many as keep the most ground
//!   once each is clear of the roads, and of those that keep nearly as much,
//!   the fewest (#422; 4–5 before, which made slivers). With a `pie_depth`
//!   (Plots v2's suburb, #468), each pie's rays then reach at least that far
//!   from its front, where the ground allows: past the rear line it shares
//!   with the lots behind, which give the ground back in the overlap pass.
//!   A side of the stem with no lot near the bulb gives no last ray. The
//!   pies swept only a right angle from the stem's line there, and three
//!   came out as slivers. With a `pie_depth` they wrap on to 135°, short
//!   of the back of that side's nearest stem lot (#470).
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
//!
//! The geometry under it, the road index, rays, regions and the overlap
//! pass, is `lot_geometry`'s, shared with the rural farms (R2, #285). This
//! file is the suburb's walker over it.

use capitol_protocol::fmath::StableMath;
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use capitol_protocol::terrain::{RoadSegment, TerrainPlot};

use super::neighbourhood::Neighbourhood;
use super::patterns::{Role, StreetMeta};
use super::SuburbPlan;
use crate::models::geom::{quad_is_convex, Polyline};
use crate::models::lot_geometry::{
    at, blend, curvature, dist, dist_to_seg, dot, place, point, push_fronts, push_fronts_by, quad, seg_seg_dist, shrink_to_fit, tangent,
    Candidate, LotParams, Ray, Region, World,
};
use crate::models::terrain::{NeighbourhoodParams, SuburbanGenParams};

type P = (f64, f64);

/// Ray spacing when finding a side's runs.
const SAMPLE: f64 = 5.0;
/// On a curve's concave side a ray stops at this share of the radius.
const CONCAVE: f64 = 0.85;
/// A ray shallower than this share of `lot_depth` starts no lot...
const MIN_DEPTH: f64 = 0.5;
/// ...and a run narrower than this share of `lot_frontage` holds none.
const MIN_RUN: f64 = 0.6;
/// A stem's run ending this near its bulb hands its last ray to the pies.
const BULB_REACH: f64 = 40.0;
/// A run's end ray follows a road's edge within this of its front...
const HUG_REACH: f64 = 1.0;
/// ...if it's within about 40° of the ray.
const HUG_COS: f64 = 0.76;
/// The most pies round a bulb, and the fewest. Since #422, 3 at most: a
/// quarter of the bulb's round is about the narrowest pie that takes 4
/// stations, as the plots plan wants of a suburban lot.
const PIES: (usize, usize) = (3, 2);
/// A bulb takes the fewest pies that keep this share of the most ground any
/// count keeps.
const PIE_KEEP: f64 = 0.95;
/// How far a pie's front may be pushed out along its ray to clear the bulb.
/// A wide pie's front runs from the stem's last ray to a ray well round the
/// bulb, so it has further to go than a lot on a curve (`push_fronts`).
const PIE_PUSH: f64 = 40.0;
/// A stem's lots stop this far short of its bulb, so the pies wrap round it.
/// At 20 a wide pie's front, from the stem's last ray, cuts the bulb's neck
/// however far out its other end goes (#422). Under `BULB_REACH`, or the stem
/// keeps its last ray and the pies lose their side.
const STEM_SHORT: f64 = 30.0;
/// With a `pie_depth`, how far round from the stem's line a side with no
/// lot near the bulb lets the pies reach (#470). At a right angle, as
/// before, three pies share half the round and come out as slivers about
/// 25 across the front.
const PIE_WRAP: f64 = 3.0 * FRAC_PI_4;
/// The wrap steps back by this towards the right angle until its ray stands.
const PIE_WRAP_STEP: f64 = FRAC_PI_2 / 6.0;

/// The suburb's lots: as wide and deep as the neighbourhood params say, off
/// the road's edge, short of an edge they back onto and of the line they
/// share with lots back to back by a hair, and sharing their side edges.
fn lot_params(p: &NeighbourhoodParams) -> LotParams {
    LotParams {
        frontage: p.lot_frontage,
        depth: p.lot_depth,
        depth_max: p.lot_depth_max,
        front_gap: 0.05,
        edge_gap: 0.05,
        rear_gap: 0.25,
        lot_gap: 0.0,
        min_shrink: 0.6,
        min_area: 0.45,
    }
}

/// Where a neighbourhood's lots may go: between its side lines, in the zone.
fn region_of(n: &Neighbourhood, params: &SuburbanGenParams, center: P) -> Region {
    let (lo, hi) = (n.frame.lo, n.frame.hi);
    Region::ring(center, params.inner_radius, params.outer_radius).ccw_of(lo.angle, lo.inset).cw_of(hi.angle, hi.inset)
}

/// The suburban lots for a Neighbourhood plan, before `finish_lots`. `roads`
/// is the whole world's, welded; `plan` the one they were laid from.
pub(crate) fn lay(roads: &[RoadSegment], plan: &SuburbPlan, params: &SuburbanGenParams, center: P) -> Vec<TerrainPlot> {
    let p = lot_params(&params.neighbourhood);
    let by_id: HashMap<&str, usize> = roads.iter().enumerate().map(|(i, r)| (r.id.as_str(), i)).collect();
    // Which sides of each road have lots. Pattern (c)'s loop has lots only
    // inside (#110).
    let sides: Vec<(bool, bool)> = {
        let mut m = vec![(false, false); roads.len()];
        for s in &plan.meta {
            if let Some(&i) = by_id.get(s.id.as_str()) {
                m[i] = s.fronted;
            }
        }
        m
    };
    let world = World::new(roads, sides, center, params.outer_radius + 2.0 * p.depth_max);

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
            let region = region_of(n, params, center);
            let np = &params.neighbourhood;
            let (bulb_radius, width_min, pie_depth) = (np.bulb_radius, np.lot_width_min, np.pie_depth);
            candidates.extend(Street { world: &world, road, meta: m, street, region, p: &p, bulb_radius, width_min, pie_depth }.lots());
        }
    }
    // The overlap pass's order: 0 for pies, 1 for through roads, 2 for
    // cul-de-sac stems. Stable, so each class keeps plan order.
    candidates.sort_by_key(|&(class, _)| class);
    place(candidates.into_iter().map(|(_, c)| c), &world, &p, "suburban")
}

/// One street's lots.
struct Street<'a> {
    world: &'a World<'a>,
    road: usize,
    meta: &'a StreetMeta,
    street: usize,
    region: Region,
    p: &'a LotParams,
    bulb_radius: f64,
    /// `NeighbourhoodParams::lot_width_min`.
    width_min: f64,
    /// `NeighbourhoodParams::pie_depth`.
    pie_depth: f64,
}

impl Street<'_> {
    fn lots(&self) -> Vec<(u8, Candidate)> {
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
        // Each fronted side's stem lot nearest the bulb, by its far ray,
        // however far back it ends (#470). `None` for a side with no lots.
        let mut tail = [Some(None), Some(None)];
        for (si, fronted) in [self.meta.fronted.0, self.meta.fronted.1].into_iter().enumerate() {
            if !fronted {
                tail[si] = None;
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
                if rays.len() >= 2 {
                    tail[si] = Some(rays.last().copied());
                }
                if s1 >= hi - BULB_REACH {
                    last[si] = rays.last().copied();
                }
            }
        }
        if let Some(centre) = self.meta.bulb {
            let stem_tip = pts[stem_end];
            out.extend(self.pies(centre, stem_tip, last, tail));
        }
        out
    }

    fn candidate(&self, class: u8, side: &str, k: usize, corners: [P; 4]) -> (u8, Candidate) {
        let m = self.meta;
        let id = format!("sub_{}_{}_{}_{side}_{k}", m.sector, m.nbhd, self.street);
        (class, Candidate { id, road: self.road, corners })
    }

    fn half(&self) -> f64 {
        self.world.roads[self.road].width / 2.0
    }

    /// The ray at `s` along the line on `side` (1 left, −1 right), if its
    /// front is clear. It may still be too shallow for a lot (`deep`).
    fn ray(&self, line: &Polyline, side: f64, s: f64) -> Option<Ray> {
        let (o, tangent) = (point(line, s), tangent(line, s));
        let d = (-tangent.1 * side, tangent.0 * side);
        let f = self.half() + self.p.front_gap;
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
        r.b - r.f >= MIN_DEPTH * self.p.depth
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
                // Named, so every clippy's `nonminimal_bool` accepts it (#266)
                let hugged = (i == 0 && hugs[0].is_some()) || (i == last && hugs[1].is_some());
                !hugged
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
        if total < MIN_RUN * self.p.frontage {
            return Vec::new();
        }
        let mut lots = (total / self.p.frontage).round().max(1.0) as usize;
        // Fewer lots rather than any under the narrowest (#456).
        while lots > 1 && total / (lots as f64) < self.width_min {
            lots -= 1;
        }
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
        let o = at(front, away, (self.p.front_gap - gap).max(0.0));
        if !self.region.inside(o) || !w.point_clear(o) {
            return None;
        }
        let b = w.back(&self.region, o, e, 0.0, self.p);
        let hug = Ray { o, d: e, f: 0.0, b, fixed: true };
        self.deep(&hug).then_some(hug)
    }

    /// Pie lots round a bulb at `centre`, whose stem ends at `tip`, between
    /// the last rays of the stem's left and right runs. A side whose runs
    /// end short of the bulb gives no last ray: with a `pie_depth`, the pies
    /// wrap on round towards the stem there (#470), as far as `tail`, that
    /// side's stem lot nearest the bulb, allows.
    fn pies(&self, centre: P, tip: P, last: [Option<Ray>; 2], tail: [Option<Option<Ray>>; 2]) -> Vec<(u8, Candidate)> {
        let toward = (centre.1 - tip.1).stable_atan2(centre.0 - tip.0);
        let bearing = |p: P| (p.1 - centre.1).stable_atan2(p.0 - centre.0);
        let angle = |r: &Ray| bearing(r.mid());
        let f = self.bulb_radius + self.half() + self.p.front_gap;
        let radial = |phi: f64| -> Option<Ray> {
            let d = (phi.stable_cos(), phi.stable_sin());
            let front = at(centre, d, f);
            if !self.region.inside(front) || !self.world.point_clear(front) {
                return None;
            }
            let b = self.world.back_from(&self.region, centre, d, f, self.half() + self.p.front_gap, self.p.depth_max, self.p);
            (b - f >= MIN_DEPTH * self.p.depth).then_some(Ray { o: centre, d, f, b, fixed: false })
        };
        // How far round from `toward` a side with no last ray reaches: a
        // right angle, or with a `pie_depth` up to `PIE_WRAP`, short of the
        // back of the side's nearest stem lot, which a pie's ray mustn't
        // cross, and as far as a pie's ray still stands.
        let wrap = |si: usize| -> f64 {
            let Some(tail) = tail[si].filter(|_| self.pie_depth > 0.0) else {
                return FRAC_PI_2;
            };
            let sign = [1.0, -1.0][si];
            let most = tail.map_or(PIE_WRAP, |r| match ((bearing(r.back()) - toward) * sign).rem_euclid(TAU) {
                a if a > PI => FRAC_PI_2,
                a => a.min(PIE_WRAP),
            });
            let steps = ((most - FRAC_PI_2) / PIE_WRAP_STEP).floor().max(0.0) as usize;
            (0..=steps)
                .map(|i| FRAC_PI_2 + (most - FRAC_PI_2) * (steps - i) as f64 / steps.max(1) as f64)
                .find(|&a| a <= FRAC_PI_2 || radial(toward + sign * a).is_some())
                .unwrap_or(FRAC_PI_2)
        };
        let from = last[0].as_ref().map_or_else(|| toward + wrap(0), angle);
        let to = last[1].as_ref().map_or_else(|| toward - wrap(1), angle);
        // Clockwise from the left round the bulb's far side to the right.
        let sweep = (from - to).rem_euclid(TAU);
        if sweep < 1e-3 {
            return Vec::new();
        }
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
        // As many pies as keep the most ground once each is clear of the
        // roads (#422): a wide pie reaches the next street and is cut back or
        // dropped, and a narrow one is a sliver. Of the counts that keep
        // nearly the most, the fewest, so the pies stay wide.
        let kept: Vec<(Vec<(usize, [P; 4])>, f64)> = (PIES.1..=PIES.0)
            .map(|count| {
                let quads = self.pie_quads(fan(count));
                let area = quads.iter().map(|(_, q)| self.kept_area(q)).sum();
                (quads, area)
            })
            .collect();
        let most = kept.iter().map(|k| k.1).fold(0.0, f64::max);
        let (quads, _) = kept.into_iter().find(|k| k.1 >= PIE_KEEP * most).expect("the most is among them");
        quads.into_iter().map(|(k, q)| self.candidate(0, "B", k, q)).collect()
    }

    /// The pies between `rays`, numbered round the bulb. Fronts are pushed
    /// only along unbroken stretches of rays.
    fn pie_quads(&self, mut rays: Vec<Option<Ray>>) -> Vec<(usize, [P; 4])> {
        let mut out = Vec::new();
        let mut j = 0;
        while j < rays.len() {
            let start = j;
            while j < rays.len() && rays[j].is_some() {
                j += 1;
            }
            let mut stretch: Vec<Ray> = rays[start..j].iter_mut().map(|r| r.take().unwrap()).collect();
            push_fronts_by(&mut stretch, self.world, PIE_PUSH);
            if self.pie_depth > 0.0 {
                stretch.iter_mut().for_each(|r| self.deepen(r));
            }
            for (k, w) in stretch.windows(2).enumerate() {
                out.push((start + k, quad(&w[0], &w[1])));
            }
            j += 1;
        }
        out
    }

    /// A pie's ray, run on to `pie_depth` from its front, short of the
    /// region's edge and the first road or water (#468). A pie narrows to
    /// its bulb, so it takes 6 stations only some 20 deeper than a row lot
    /// that does. Pies go first in the overlap pass, so the lots it reaches
    /// behind, a stem's or the next row's, give it the ground. A ray the
    /// stem's last lot shares runs on too: that lot keeps its own back.
    fn deepen(&self, r: &mut Ray) {
        let want = r.f + self.pie_depth;
        if r.b >= want {
            return;
        }
        let exit = self.region.exit(r.o, r.d, r.f) - self.p.edge_gap;
        let hit = self.world.first_hit(r.o, r.d, r.f, want).map_or(f64::INFINITY, |(t, _)| t - self.p.edge_gap);
        r.b = r.b.max(want.min(exit).min(hit));
    }

    /// What's left of a lot once the overlap pass has pulled it clear of
    /// the roads and the water, as `place` would, or 0 if it's dropped.
    fn kept_area(&self, q: &[P; 4]) -> f64 {
        let least = self.p.min_area * self.p.frontage * self.p.depth;
        let ok = |q: &[P; 4]| quad_is_convex(q) && TerrainPlot::calculate_area(q) >= least && self.world.quad_clear(q);
        shrink_to_fit(*q, self.p.min_shrink, ok).map_or(0.0, |q| TerrainPlot::calculate_area(&q))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::geom::{quads_overlap_strict, SpatialGrid};
    use crate::models::lot_geometry::bounds;
    use crate::models::suburb::patterns::tests::parallels_world;
    use crate::models::terrain::TerrainConfig;

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

