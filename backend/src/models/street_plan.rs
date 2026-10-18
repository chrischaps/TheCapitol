//! Street-plan geometry for the suburban generator: clean polylines, the
//! tests a candidate street must pass, and the pass that makes every junction
//! explicit (#62).
//!
//! The generator used to emit raw spline samples. Uniform Catmull-Rom
//! overshoots and loops when control points are unevenly spaced, so
//! collectors came out with hairpins and self-crossings. Streets began
//! part-way along a parent segment, with no vertex there, so nothing marked
//! the junction. Clients draw roads at their true width now, and both
//! defects show.
//!
//! What this module guarantees for every suburban road, once
//! [`weld_junctions`] has run (`audit` checks it in the tests):
//! - no segment shorter than [`MIN_STEP`]
//! - no turn sharper than [`MAX_TURN_DEG`], except at a road's own junction
//!   with itself (where a cul-de-sac's bulb leaves and rejoins its stem)
//! - a road crosses itself, or another road, only at a vertex both share
//! - each end is a vertex of another road, or of the road itself (a closed bulb)

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::RoadSegment;

type P = (f64, f64);

/// Points closer than this are one point.
pub(crate) const MIN_STEP: f64 = 1.0;

/// A junction this close to an existing vertex uses that vertex, so no
/// junction leaves a sliver of a segment behind.
pub(crate) const SNAP: f64 = 4.0;

/// The sharpest turn allowed at a vertex, in degrees off straight ahead.
#[cfg(test)]
pub(crate) const MAX_TURN_DEG: f64 = 45.0;

/// How far past another road's edge a street end may stop and still be
/// taken as joining it. The old generator left ends up to 15 units short.
pub(crate) const JOIN_REACH: f64 = 15.0;

/// Two points this close are the same vertex.
const SAME: f64 = 1e-6;

fn dist(a: P, b: P) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Centripetal Catmull-Rom (alpha 0.5) through `controls`, `per_span` samples
/// a span. Unlike the uniform kind it can't form a cusp or a loop within a
/// span, whatever the spacing of the controls. The end tangents come from
/// reflected phantom points, so the curve leaves and arrives head on.
#[cfg(test)]
pub(crate) fn centripetal_spline(controls: &[P], per_span: usize) -> Vec<P> {
    let controls = dedupe(controls);
    let n = controls.len();
    if n < 3 {
        return controls;
    }
    let reflect = |a: P, b: P| (2.0 * a.0 - b.0, 2.0 * a.1 - b.1);
    let mut ext = Vec::with_capacity(n + 2);
    ext.push(reflect(controls[0], controls[1]));
    ext.extend_from_slice(&controls);
    ext.push(reflect(controls[n - 1], controls[n - 2]));

    let knot = |a: P, b: P| dist(a, b).sqrt().max(1e-9);
    let mut out = Vec::with_capacity((n - 1) * per_span + 1);
    for i in 1..ext.len() - 2 {
        let (p0, p1, p2, p3) = (ext[i - 1], ext[i], ext[i + 1], ext[i + 2]);
        let t0 = 0.0;
        let t1 = t0 + knot(p0, p1);
        let t2 = t1 + knot(p1, p2);
        let t3 = t2 + knot(p2, p3);
        for j in 0..per_span {
            let t = t1 + (t2 - t1) * j as f64 / per_span as f64;
            let a1 = lerp(p0, p1, (t - t0) / (t1 - t0));
            let a2 = lerp(p1, p2, (t - t1) / (t2 - t1));
            let a3 = lerp(p2, p3, (t - t2) / (t3 - t2));
            let b1 = lerp(a1, a2, (t - t0) / (t2 - t0));
            let b2 = lerp(a2, a3, (t - t1) / (t3 - t1));
            out.push(lerp(b1, b2, (t - t1) / (t2 - t1)));
        }
    }
    out.push(controls[n - 1]);
    out
}

/// `points` without the repeats: a point within [`MIN_STEP`] of the last one
/// kept is dropped. Both ends stay exactly where they were.
pub(crate) fn dedupe(points: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_none_or(|&q| dist(q, p) >= MIN_STEP) {
            out.push(p);
        }
    }
    // The last point kept may be a near repeat standing in for the true end.
    if let Some(&last) = points.last() {
        if out.len() >= 2 {
            let n = out.len();
            out[n - 1] = last;
            if n >= 3 && dist(out[n - 2], last) < MIN_STEP {
                out.remove(n - 2);
            }
        }
    }
    out
}

/// `points` resampled at even steps of about `spacing` along its length,
/// with both ends exact.
pub(crate) fn resample(points: &[P], spacing: f64) -> Vec<P> {
    let points = dedupe(points);
    if points.len() < 2 {
        return points;
    }
    let lengths: Vec<f64> = points.windows(2).map(|w| dist(w[0], w[1])).collect();
    let total: f64 = lengths.iter().sum();
    let steps = (total / spacing).ceil().max(1.0) as usize;
    let step = total / steps as f64;
    let mut out = Vec::with_capacity(steps + 1);
    out.push(points[0]);
    let (mut seg, mut before) = (0, 0.0);
    for k in 1..steps {
        let at = step * k as f64;
        while seg < lengths.len() - 1 && before + lengths[seg] < at {
            before += lengths[seg];
            seg += 1;
        }
        let t = ((at - before) / lengths[seg]).clamp(0.0, 1.0);
        out.push(lerp(points[seg], points[seg + 1], t));
    }
    out.push(*points.last().expect("two points"));
    out
}

/// The turn at each interior vertex, in degrees off straight ahead, as
/// (vertex index, degrees).
#[cfg(test)]
pub(crate) fn turns(points: &[P]) -> impl Iterator<Item = (usize, f64)> + '_ {
    (1..points.len().saturating_sub(1)).map(move |i| {
        let (a, b, c) = (points[i - 1], points[i], points[i + 1]);
        let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
        let cross = u.0 * v.1 - u.1 * v.0;
        let dot = u.0 * v.0 + u.1 * v.1;
        (i, cross.stable_atan2(dot).abs().to_degrees())
    })
}

/// Whether vertex `i` is a junction of the road with itself: another of its
/// vertices sits at the same place.
fn self_junction(points: &[P], i: usize) -> bool {
    points.iter().enumerate().any(|(j, &q)| j != i && dist(q, points[i]) < SAME)
}

/// The sharpest turn in `points`, leaving out its junctions with itself.
#[cfg(test)]
pub(crate) fn sharpest_turn(points: &[P]) -> f64 {
    turns(points)
        .filter(|&(i, _)| !self_junction(points, i))
        .map(|(_, deg)| deg)
        .fold(0.0, f64::max)
}

/// Where segments `a0-a1` and `b0-b1` cross, as (t along a, u along b, point).
fn crossing(a0: P, a1: P, b0: P, b1: P) -> Option<(f64, f64, P)> {
    let (r, s) = ((a1.0 - a0.0, a1.1 - a0.1), (b1.0 - b0.0, b1.1 - b0.1));
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom.abs() < 1e-12 {
        return None;
    }
    let q = (b0.0 - a0.0, b0.1 - a0.1);
    let t = (q.0 * s.1 - q.1 * s.0) / denom;
    let u = (q.0 * r.1 - q.1 * r.0) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| (t, u, lerp(a0, a1, t)))
}

fn boxes_meet(a0: P, a1: P, b0: P, b1: P, pad: f64) -> bool {
    a0.0.min(a1.0) <= b0.0.max(b1.0) + pad
        && b0.0.min(b1.0) <= a0.0.max(a1.0) + pad
        && a0.1.min(a1.1) <= b0.1.max(b1.1) + pad
        && b0.1.min(b1.1) <= a0.1.max(a1.1) + pad
}

/// Whether the crossing at `x` is a vertex of both segments.
fn at_shared_vertex(x: P, a: [P; 2], b: [P; 2]) -> bool {
    a.iter().any(|&p| dist(p, x) < SAME) && b.iter().any(|&p| dist(p, x) < SAME)
}

/// Whether `points` crosses itself anywhere but at a vertex it visits twice.
#[cfg(test)]
pub(crate) fn self_crosses(points: &[P]) -> bool {
    let n = points.len();
    for i in 0..n.saturating_sub(1) {
        for j in i + 2..n - 1 {
            let (a, b) = ([points[i], points[i + 1]], [points[j], points[j + 1]]);
            if !boxes_meet(a[0], a[1], b[0], b[1], 0.0) {
                continue;
            }
            if let Some((_, _, x)) = crossing(a[0], a[1], b[0], b[1]) {
                if !at_shared_vertex(x, a, b) {
                    return true;
                }
            }
        }
    }
    false
}

/// The box round `points`, as (min, max).
fn bounds(points: &[P]) -> (P, P) {
    points.iter().fold(
        ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY)),
        |(lo, hi), &(x, y)| ((lo.0.min(x), lo.1.min(y)), (hi.0.max(x), hi.1.max(y))),
    )
}

/// The segments of `road` that reach into the box, with their indices. A
/// ring road's own box covers the whole zone, so this goes by segment.
fn segments_in(road: &[P], (lo, hi): (P, P)) -> Vec<(usize, [P; 2])> {
    road.windows(2)
        .enumerate()
        .filter(|(_, w)| boxes_meet(w[0], w[1], lo, hi, 0.0))
        .map(|(i, w)| (i, [w[0], w[1]]))
        .collect()
}

/// Whether every point of `street` stays `clearance` from `road`, except
/// within `exempt` of one of the `ends` (where the street joins it).
#[cfg(test)]
pub(crate) fn keeps_clear(street: &[P], road: &[P], clearance: f64, ends: &[P], exempt: f64) -> bool {
    street.iter().all(|&p| {
        ends.iter().any(|&e| dist(e, p) <= exempt)
            || nearest_on(road, p).is_none_or(|(_, _, d)| d >= clearance)
    })
}

/// The nearest point to `p` on `points`: (segment index, point, distance).
fn nearest_on(points: &[P], p: P) -> Option<(usize, P, f64)> {
    let mut best: Option<(usize, P, f64)> = None;
    for (i, w) in points.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let d = (b.0 - a.0, b.1 - a.1);
        let len2 = d.0 * d.0 + d.1 * d.1;
        let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * d.0 + (p.1 - a.1) * d.1) / len2).clamp(0.0, 1.0) };
        let q = lerp(a, b, t);
        let e = dist(p, q);
        if best.is_none_or(|(_, _, bd)| e < bd) {
            best = Some((i, q, e));
        }
    }
    best
}

/// The nearer of segment `seg`'s two vertices to `x`, if it's within
/// [`SNAP`]. The nearer, not the first: when `x` sits on the far vertex and
/// the near one is also within `SNAP`, the far one is the junction (#114).
fn snap_vertex(points: &[P], seg: usize, x: P) -> Option<usize> {
    [seg, seg + 1]
        .into_iter()
        .map(|k| (k, dist(points[k], x)))
        .filter(|&(_, d)| d < SNAP)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// Puts `x` into segment `seg` of `points` as a vertex, or uses the
/// segment's own vertex if one is within [`SNAP`]. Returns the vertex used.
fn insert_vertex(points: &mut Vec<P>, seg: usize, x: P) -> P {
    if let Some(k) = snap_vertex(points, seg, x) {
        return points[k];
    }
    points.insert(seg + 1, x);
    x
}

/// Moves the vertex of `points` nearest `x` along segment `seg` onto `x` if
/// it's within [`SNAP`], or puts `x` in as a new vertex.
fn bring_to(points: &mut Vec<P>, seg: usize, x: P) {
    if let Some(k) = snap_vertex(points, seg, x) {
        points[k] = x;
        return;
    }
    points.insert(seg + 1, x);
}

/// Makes every junction a shared vertex. Each end of a road `weld` picks
/// that stops on or within [`JOIN_REACH`] of another road's edge is moved
/// onto it, and the point goes into that road too. Then every crossing
/// between a picked road and any other gets a vertex in both.
pub(crate) fn weld_junctions(roads: &mut [RoadSegment], weld: impl Fn(&RoadSegment) -> bool) {
    let picked: Vec<usize> = (0..roads.len()).filter(|&i| weld(&roads[i])).collect();

    // Ends onto the roads they stop at.
    for &i in &picked {
        for first in [true, false] {
            let pts = &roads[i].points;
            if pts.len() < 2 {
                continue;
            }
            let end_idx = if first { 0 } else { pts.len() - 1 };
            let end = pts[end_idx];
            if self_junction(pts, end_idx) {
                continue;
            }
            let mut best: Option<(usize, usize, P, f64)> = None;
            for (k, other) in roads.iter().enumerate() {
                if k == i || other.points.len() < 2 {
                    continue;
                }
                if let Some((seg, q, d)) = nearest_on(&other.points, end) {
                    let reach = other.width / 2.0 + JOIN_REACH;
                    if d <= reach && best.is_none_or(|b| d < b.3) {
                        best = Some((k, seg, q, d));
                    }
                }
            }
            if let Some((k, seg, q, _)) = best {
                let v = insert_vertex(&mut roads[k].points, seg, q);
                let pts = &mut roads[i].points;
                pts[end_idx] = v;
                *pts = dedupe(pts);
            }
        }
    }

    // Crossings into shared vertices. Each fix splits a segment, so go round
    // until a pass finds nothing.
    for _ in 0..8 {
        let mut fixed = false;
        for &i in &picked {
            for k in 0..roads.len() {
                if k == i || (k < i && picked.contains(&k)) {
                    continue;
                }
                while let Some((si, sk, x)) = first_unjoined(&roads[i].points, &roads[k].points) {
                    let v = insert_vertex(&mut roads[k].points, sk, x);
                    bring_to(&mut roads[i].points, si, v);
                    fixed = true;
                }
            }
        }
        if !fixed {
            break;
        }
    }
}

/// The first place `a` crosses `b` other than at a vertex both share:
/// (segment of a, segment of b, point).
fn first_unjoined(a: &[P], b: &[P]) -> Option<(usize, usize, P)> {
    let near = segments_in(b, bounds(a));
    if near.is_empty() {
        return None;
    }
    for (i, sa) in a.windows(2).enumerate() {
        for &(k, sb) in &near {
            if !boxes_meet(sa[0], sa[1], sb[0], sb[1], 0.0) {
                continue;
            }
            if let Some((_, _, x)) = crossing(sa[0], sa[1], sb[0], sb[1]) {
                let shared = [sa[0], sa[1]].iter().any(|&p| [sb[0], sb[1]].iter().any(|&q| dist(p, q) < SAME));
                if !at_shared_vertex(x, [sa[0], sa[1]], [sb[0], sb[1]]) && !shared {
                    return Some((i, k, x));
                }
            }
        }
    }
    None
}

/// A cul-de-sac's bulb, or a P-street's loop: a ring of `r` that leaves
/// `entry` heading `dir` (a unit vector) and comes back round to it. The
/// points start after `entry` and end exactly on it.
pub(crate) fn bulb(entry: P, dir: P, r: f64, segments: usize) -> Vec<P> {
    let centre = (entry.0 + r * dir.0, entry.1 + r * dir.1);
    let start = (-dir.1).stable_atan2(-dir.0);
    let mut out: Vec<P> = (1..segments)
        .map(|k| {
            let a = start + k as f64 / segments as f64 * std::f64::consts::TAU;
            (centre.0 + r * a.stable_cos(), centre.1 + r * a.stable_sin())
        })
        .collect();
    out.push(entry);
    out
}

/// The unit direction a polyline arrives at its last point.
pub(crate) fn end_direction(points: &[P]) -> Option<P> {
    let n = points.len();
    if n < 2 {
        return None;
    }
    let (a, b) = (points[n - 2], points[n - 1]);
    let l = dist(a, b);
    (l > 0.0).then(|| ((b.0 - a.0) / l, (b.1 - a.1) / l))
}

/// What's wrong with a street plan, for the invariant tests.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct Audit {
    pub short_segments: Vec<(String, usize)>,
    pub sharp_turns: Vec<(String, usize, f64)>,
    pub self_crossings: Vec<String>,
    pub unjoined_crossings: Vec<(String, String, P)>,
    pub loose_ends: Vec<(String, P)>,
}

#[cfg(test)]
impl Audit {
    pub fn is_clean(&self) -> bool {
        self.short_segments.is_empty()
            && self.sharp_turns.is_empty()
            && self.self_crossings.is_empty()
            && self.unjoined_crossings.is_empty()
            && self.loose_ends.is_empty()
    }
}

/// Checks the roads `check` picks against the invariants in the module docs.
#[cfg(test)]
pub(crate) fn audit(roads: &[RoadSegment], check: impl Fn(&RoadSegment) -> bool) -> Audit {
    let mut out = Audit::default();
    for (i, road) in roads.iter().enumerate() {
        if !check(road) {
            continue;
        }
        let pts = &road.points;
        for (s, w) in pts.windows(2).enumerate() {
            if dist(w[0], w[1]) < MIN_STEP {
                out.short_segments.push((road.id.clone(), s));
            }
        }
        for (v, deg) in turns(pts) {
            if deg > MAX_TURN_DEG && !self_junction(pts, v) {
                out.sharp_turns.push((road.id.clone(), v, deg));
            }
        }
        if self_crosses(pts) {
            out.self_crossings.push(road.id.clone());
        }
        for (k, other) in roads.iter().enumerate() {
            if k == i || (k < i && check(other)) {
                continue;
            }
            if let Some((_, _, x)) = first_unjoined(pts, &other.points) {
                out.unjoined_crossings.push((road.id.clone(), other.id.clone(), x));
            }
        }
        for end_idx in [0, pts.len() - 1] {
            let end = pts[end_idx];
            let joined = self_junction(pts, end_idx)
                || roads.iter().enumerate().any(|(k, o)| k != i && o.points.iter().any(|&q| dist(q, end) < SAME));
            if !joined {
                out.loose_ends.push((road.id.clone(), end));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centripetal_spline_has_no_loops_on_uneven_controls() {
        // Uniform Catmull-Rom loops on a short span between two long ones.
        let controls = [(0.0, 0.0), (100.0, 0.0), (103.0, 2.0), (200.0, 0.0)];
        let pts = centripetal_spline(&controls, 12);
        assert!(!self_crosses(&pts));
        assert_eq!(pts.first(), Some(&(0.0, 0.0)));
        assert_eq!(pts.last(), Some(&(200.0, 0.0)));
    }

    #[test]
    fn resample_keeps_ends_and_evens_steps() {
        let pts = resample(&[(0.0, 0.0), (0.2, 0.0), (10.0, 0.0), (10.0, 10.0)], 3.0);
        assert_eq!(pts.first(), Some(&(0.0, 0.0)));
        assert_eq!(pts.last(), Some(&(10.0, 10.0)));
        let steps: Vec<f64> = pts.windows(2).map(|w| dist(w[0], w[1])).collect();
        assert!(steps.iter().all(|&s| s > 2.0 && s <= 3.0 + 1e-9), "{steps:?}");
    }

    #[test]
    fn dedupe_drops_repeats_but_keeps_the_end() {
        let pts = dedupe(&[(0.0, 0.0), (0.0, 0.0), (5.0, 0.0), (5.5, 0.0)]);
        assert_eq!(pts, vec![(0.0, 0.0), (5.5, 0.0)]);
    }

    #[test]
    fn a_t_junction_and_a_crossing_become_shared_vertices() {
        let road = |id: &str, points: Vec<P>| RoadSegment {
            id: id.into(),
            road_type: capitol_protocol::terrain::RoadType::Suburban,
            points,
            width: 10.0,
        };
        let mut roads = vec![
            road("main", vec![(0.0, 0.0), (100.0, 0.0)]),
            // Stops 8 short of main's edge.
            road("t", vec![(50.0, 13.0), (50.0, 80.0)]),
            road("x", vec![(20.0, -40.0), (26.0, 40.0)]),
        ];
        weld_junctions(&mut roads, |r| r.id != "main");
        let a = audit(&roads, |r| r.id != "main");
        assert!(a.unjoined_crossings.is_empty(), "{a:?}");
        assert_eq!(roads[1].points[0], (50.0, 0.0));
        assert!(roads[0].points.contains(&(50.0, 0.0)));
        assert_eq!(roads[0].points.len(), 4, "{:?}", roads[0].points);
        // t's far end and both of x's are dead ends.
        assert_eq!(a.loose_ends.len(), 3, "{a:?}");
    }

    #[test]
    fn an_end_on_a_vertex_snaps_to_it_not_to_its_close_neighbour() {
        let road = |id: &str, points: Vec<P>| RoadSegment {
            id: id.into(),
            road_type: capitol_protocol::terrain::RoadType::Suburban,
            points,
            width: 10.0,
        };
        // Main has a vertex at x = 50 and another 3 before it, inside SNAP.
        // The nearest segment to t's end is the short one, with the point
        // at its far end (#114).
        let mut roads = vec![
            road("main", vec![(0.0, 0.0), (47.0, 0.0), (50.0, 0.0), (100.0, 0.0)]),
            road("t", vec![(50.0, 13.0), (50.0, 80.0)]),
        ];
        weld_junctions(&mut roads, |r| r.id != "main");
        assert_eq!(roads[1].points[0], (50.0, 0.0));
        assert_eq!(roads[0].points, vec![(0.0, 0.0), (47.0, 0.0), (50.0, 0.0), (100.0, 0.0)]);
    }

    #[test]
    fn a_lollipop_is_not_a_self_crossing_or_a_sharp_turn() {
        let mut pts = vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)];
        let (c, r) = ((30.0, 0.0), 10.0);
        for k in 1..=16 {
            let a = std::f64::consts::PI + k as f64 / 16.0 * std::f64::consts::TAU;
            pts.push((c.0 + r * a.stable_cos(), c.1 + r * a.stable_sin()));
        }
        *pts.last_mut().unwrap() = (20.0, 0.0);
        assert!(!self_crosses(&pts));
        assert!(sharpest_turn(&pts) <= MAX_TURN_DEG, "{}", sharpest_turn(&pts));
    }
}
