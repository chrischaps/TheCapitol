//! The last pass over the generated lots: keep each one clear of the roads
//! and the water, and give each one an id of its own.
//!
//! The lot generators place lots against their own idea of the streets
//! nearby. The urban grid insets its blocks by the grid street's half width,
//! even along a spoke that is really a 60-wide arterial. So some lots reached
//! into roads and canals, and four ids were shared by eleven lots. A lot is
//! what a player claims, and what station placement tests against, so it has
//! to be exactly the ground it looks like (B7, #42).
//!
//! **Clearing.** A corner that stands on a road or in the water slides along
//! one of its two edges, towards that neighbour, until it's clear. Of the two
//! edges it takes the shorter slide. A slid corner stays on the lot's old
//! outline, so the lot stays a convex quad no bigger than it was, and
//! `corners: [_; 4]` holds on the wire. A road or channel that crosses a lot
//! without covering a corner can't be cleared that way, so that lot is
//! dropped, as is one cut to under `MIN_KEPT` of its area.
//!
//! **Ids.** The first lot with an id keeps it. Later lots with the same id get
//! `_2`, `_3` and so on, in generation order, so ids stay stable for a given
//! set of world params.

use std::collections::HashMap;

use capitol_protocol::terrain::{RoadSegment, TerrainPlot, WaterRing};

/// How far a lot stays back from the water's edge. That's the kerb stone every
/// client draws round a channel: the Flint client's `water::KERB` is 1 m, which
/// is 10 server units.
pub const WATER_KERB: f64 = 10.0;

/// A lot cut to less than this share of its area is dropped.
const MIN_KEPT: f64 = 0.5;

/// A lot may touch a road or kerb edge; it only has to stay out of it. Without
/// this, a lot laid exactly along a street's edge reads as on it by rounding.
const TOUCH: f64 = 0.01;

/// Sample spacing along a lot's edges and across its interior when checking it's clear.
const SAMPLE_STEP: f64 = 2.0;

/// A road as the clearance pass sees it: its segments, its half width, and its box.
struct Ribbon {
    segments: Vec<((f64, f64), (f64, f64))>,
    half: f64,
    min: (f64, f64),
    max: (f64, f64),
}

/// What a lot must stay out of.
struct Obstacles {
    ribbons: Vec<Ribbon>,
    /// Each water ring as (inner, outer) radius, widened by the kerb.
    bands: Vec<(f64, f64)>,
    center: (f64, f64),
}

impl Obstacles {
    fn new(roads: &[RoadSegment], rings: &[WaterRing], center: (f64, f64)) -> Self {
        let ribbons = roads
            .iter()
            .filter(|r| r.points.len() >= 2)
            .map(|r| {
                let half = r.width / 2.0;
                let mut min = (f64::INFINITY, f64::INFINITY);
                let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
                for &(x, y) in &r.points {
                    min = (min.0.min(x - half), min.1.min(y - half));
                    max = (max.0.max(x + half), max.1.max(y + half));
                }
                Ribbon {
                    segments: r.points.windows(2).map(|w| (w[0], w[1])).collect(),
                    half,
                    min,
                    max,
                }
            })
            .collect();
        let bands = rings
            .iter()
            .map(|r| (r.inner_radius() - WATER_KERB, r.outer_radius() + WATER_KERB))
            .collect();
        Self { ribbons, bands, center }
    }

    /// The road segments whose ribbons could touch the box `min`..`max`, each
    /// with its ribbon's half width. Filtered by segment, not by road: an
    /// urban arc's box is the whole ring.
    fn near(&self, min: (f64, f64), max: (f64, f64)) -> Vec<Segment> {
        let mut out = Vec::new();
        for r in &self.ribbons {
            if r.max.0 < min.0 || r.min.0 > max.0 || r.max.1 < min.1 || r.min.1 > max.1 {
                continue;
            }
            for &(a, b) in &r.segments {
                let h = r.half;
                if a.0.max(b.0) + h < min.0
                    || a.0.min(b.0) - h > max.0
                    || a.1.max(b.1) + h < min.1
                    || a.1.min(b.1) - h > max.1
                {
                    continue;
                }
                out.push(Segment { a, b, half: r.half });
            }
        }
        out
    }

    fn blocked(&self, p: (f64, f64), near: &[Segment]) -> bool {
        let r = ((p.0 - self.center.0).powi(2) + (p.1 - self.center.1).powi(2)).sqrt();
        if self.bands.iter().any(|&(lo, hi)| r > lo + TOUCH && r < hi - TOUCH) {
            return true;
        }
        near.iter().any(|s| distance_to_segment(p, s.a, s.b) < s.half - TOUCH)
    }
}

/// One stretch of a road's centreline, with the road's half width.
struct Segment {
    a: (f64, f64),
    b: (f64, f64),
    half: f64,
}

fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

fn lerp(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn bounds(corners: &[(f64, f64); 4]) -> ((f64, f64), (f64, f64)) {
    let mut min = (f64::INFINITY, f64::INFINITY);
    let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &(x, y) in corners {
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    }
    (min, max)
}

/// How far (as a share of the edge, up to 0.6) `from` must slide towards `to`
/// to come clear, or `None` if it doesn't within that.
fn slide(from: (f64, f64), to: (f64, f64), obstacles: &Obstacles, near: &[Segment]) -> Option<f64> {
    const STEPS: usize = 60;
    let len = dist(from, to);
    if len <= 0.0 {
        return None;
    }
    // Walk out in steps of at most half a unit, then bisect the last one.
    let step = (0.5 / len).min(0.6 / STEPS as f64);
    let mut t = 0.0;
    while t < 0.6 {
        let next = t + step;
        if !obstacles.blocked(lerp(from, to, next), near) {
            let (mut lo, mut hi) = (t, next);
            for _ in 0..12 {
                let mid = (lo + hi) / 2.0;
                if obstacles.blocked(lerp(from, to, mid), near) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            return Some(hi);
        }
        t = next;
    }
    None
}

/// Whether every sampled point of the edge `a`..`b` is clear.
fn edge_clear(a: (f64, f64), b: (f64, f64), obstacles: &Obstacles, near: &[Segment]) -> bool {
    let n = (dist(a, b) / SAMPLE_STEP).ceil().max(1.0) as usize;
    let sample = |k: usize, _: usize| lerp(a, b, k as f64 / n as f64);
    !obstacles.any_blocked(near, &sample, (0, n), (0, 0))
}

/// Whether every sampled point of the lot, on its edges and across it, is clear.
fn clear(corners: &[(f64, f64); 4], obstacles: &Obstacles, near: &[Segment]) -> bool {
    if !(0..4).all(|i| edge_clear(corners[i], corners[(i + 1) % 4], obstacles, near)) {
        return false;
    }
    // Across the lot: lerp between the street edge (0→1) and the back edge (3→2).
    let depth = dist(corners[0], corners[3]).max(dist(corners[1], corners[2]));
    let width = dist(corners[0], corners[1]).max(dist(corners[3], corners[2]));
    let (nd, nw) = (
        (depth / (SAMPLE_STEP * 2.0)).ceil().max(1.0) as usize,
        (width / (SAMPLE_STEP * 2.0)).ceil().max(1.0) as usize,
    );
    if nd < 2 || nw < 2 {
        return true;
    }
    let sample = |i: usize, j: usize| {
        let v = i as f64 / nd as f64;
        let (l, r) = (lerp(corners[0], corners[3], v), lerp(corners[1], corners[2], v));
        lerp(l, r, j as f64 / nw as f64)
    };
    !obstacles.any_blocked(near, &sample, (1, nd - 1), (1, nw - 1))
}

/// Slack for rounding in [`Obstacles::any_blocked`]'s bounds, in world
/// units: far more than a sample's rounding (about 1e-11 at these
/// coordinates), far less than [`TOUCH`].
const BOUND_SLACK: f64 = 1e-6;

impl Obstacles {
    /// Whether any sample `at(i, j)`, for `i` in `is` and `j` in `js` (both
    /// inclusive), is [`blocked`](Self::blocked): the same answer as asking
    /// of every one in turn, for a fraction of the work (#458).
    ///
    /// `at` must be bilinear in (i, j), as a lot's samples are (lerps of
    /// lerps), or linear, as an edge's are. Then every sample of a block of
    /// them lies in the hull of the block's four corner samples, so within
    /// `reach` of their middle, and an obstacle farther than that from the
    /// middle can't block any of them. A block drops those and splits in two
    /// until none are left, or it's one sample, which is asked as `blocked`
    /// asks it. So the clearing is unchanged to the bit: only obstacles that
    /// provably miss a sample, with [`BOUND_SLACK`] to spare, go unasked.
    fn any_blocked(
        &self,
        near: &[Segment],
        at: &dyn Fn(usize, usize) -> (f64, f64),
        is: (usize, usize),
        js: (usize, usize),
    ) -> bool {
        let near: Vec<&Segment> = near.iter().collect();
        self.blocked_in(&near, &self.bands, at, is, js)
    }

    fn blocked_in(
        &self,
        near: &[&Segment],
        bands: &[(f64, f64)],
        at: &dyn Fn(usize, usize) -> (f64, f64),
        is: (usize, usize),
        js: (usize, usize),
    ) -> bool {
        if is == (is.0, is.0) && js == (js.0, js.0) {
            // As `blocked`, over what's left.
            let p = at(is.0, js.0);
            let r = ((p.0 - self.center.0).powi(2) + (p.1 - self.center.1).powi(2)).sqrt();
            return bands.iter().any(|&(lo, hi)| r > lo + TOUCH && r < hi - TOUCH)
                || near.iter().any(|s| distance_to_segment(p, s.a, s.b) < s.half - TOUCH);
        }
        let corners = [at(is.0, js.0), at(is.1, js.0), at(is.0, js.1), at(is.1, js.1)];
        let mid = (
            corners.iter().map(|c| c.0).sum::<f64>() / 4.0,
            corners.iter().map(|c| c.1).sum::<f64>() / 4.0,
        );
        let reach = corners.iter().map(|&c| dist(mid, c)).fold(0.0, f64::max) + BOUND_SLACK;
        let r = dist(mid, self.center);
        let bands: Vec<(f64, f64)> =
            bands.iter().copied().filter(|&(lo, hi)| r + reach > lo + TOUCH && r - reach < hi - TOUCH).collect();
        let near: Vec<&Segment> =
            near.iter().copied().filter(|s| distance_to_segment(mid, s.a, s.b) - reach < s.half - TOUCH).collect();
        if bands.is_empty() && near.is_empty() {
            return false;
        }
        // Split the longer way.
        if is.1 - is.0 >= js.1 - js.0 {
            let m = (is.0 + is.1) / 2;
            self.blocked_in(&near, &bands, at, (is.0, m), js) || self.blocked_in(&near, &bands, at, (m + 1, is.1), js)
        } else {
            let m = (js.0 + js.1) / 2;
            self.blocked_in(&near, &bands, at, is, (js.0, m)) || self.blocked_in(&near, &bands, at, is, (m + 1, js.1))
        }
    }
}

/// Pull one lot clear of the obstacles, or `None` if it can't be.
fn clear_lot(mut corners: [(f64, f64); 4], obstacles: &Obstacles) -> Option<[(f64, f64); 4]> {
    let original = TerrainPlot::calculate_area(&corners);
    let (min, max) = bounds(&corners);
    let near = obstacles.near(min, max);
    // Each pass slides every blocked corner once. A corner can need a second
    // pass after its neighbour moves, so allow a few.
    for _ in 0..4 {
        let mut moved = false;
        for i in 0..4 {
            if !obstacles.blocked(corners[i], &near) {
                continue;
            }
            let (prev, next) = (corners[(i + 3) % 4], corners[(i + 1) % 4]);
            let options = [
                slide(corners[i], prev, obstacles, &near).map(|t| (t * dist(corners[i], prev), prev, t)),
                slide(corners[i], next, obstacles, &near).map(|t| (t * dist(corners[i], next), next, t)),
            ];
            let best = options
                .into_iter()
                .flatten()
                .min_by(|a, b| a.0.total_cmp(&b.0))?;
            corners[i] = lerp(corners[i], best.1, best.2);
            moved = true;
        }
        if !moved {
            break;
        }
    }
    // A corner can be clear while the edge between two clear corners isn't: a
    // straight edge cuts inside a curved kerb, and a road can cross an edge.
    // Move such an edge inwards, both ends along their other edges together.
    for _ in 0..4 {
        let mut moved = false;
        for i in 0..4 {
            let (a, b) = (i, (i + 1) % 4);
            let (a_to, b_to) = (corners[(i + 3) % 4], corners[(i + 2) % 4]);
            if edge_clear(corners[a], corners[b], obstacles, &near) {
                continue;
            }
            let shifted = |t: f64| (lerp(corners[a], a_to, t), lerp(corners[b], b_to, t));
            let len = dist(corners[a], a_to).max(dist(corners[b], b_to));
            let step = (0.5 / len.max(1.0)).min(0.01);
            let mut t = step;
            while t <= 0.5 && {
                let (p, q) = shifted(t);
                !edge_clear(p, q, obstacles, &near)
            } {
                t += step;
            }
            if t > 0.5 {
                return None;
            }
            let (p, q) = shifted(t);
            corners[a] = p;
            corners[b] = q;
            moved = true;
        }
        if !moved {
            break;
        }
    }
    let area = TerrainPlot::calculate_area(&corners);
    (area >= original * MIN_KEPT && clear(&corners, obstacles, &near)).then_some(corners)
}

/// What clearing does to each lot: its corners once cleared, or `None` where
/// it's dropped. The suburb plan report (#104) uses it to show which lots
/// `finish_lots` trims and drops.
pub(crate) fn clear_each(
    plots: &[TerrainPlot],
    roads: &[RoadSegment],
    rings: &[WaterRing],
    center: (f64, f64),
) -> Vec<Option<[(f64, f64); 4]>> {
    let obstacles = Obstacles::new(roads, rings, center);
    plots.iter().map(|plot| clear_lot(plot.corners, &obstacles)).collect()
}

/// Clear every lot of the roads and the water, drop the ones that can't be,
/// and make the ids unique. Order is kept.
pub fn finish_lots(
    plots: Vec<TerrainPlot>,
    roads: &[RoadSegment],
    rings: &[WaterRing],
    center: (f64, f64),
) -> Vec<TerrainPlot> {
    let cleared = clear_each(&plots, roads, rings, center);
    let mut seen: HashMap<String, u32> = HashMap::new();
    plots
        .into_iter()
        .zip(cleared)
        .filter_map(|(mut plot, corners)| {
            let corners = corners?;
            if corners != plot.corners {
                plot.corners = corners;
                plot.area = TerrainPlot::calculate_area(&corners);
            }
            Some(plot)
        })
        .map(|mut plot| {
            let n = seen.entry(plot.id.clone()).or_insert(0);
            *n += 1;
            if *n > 1 {
                plot.id = format!("{}_{}", plot.id, n);
            }
            plot
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::terrain::RoadType;

    fn lot(id: &str, corners: [(f64, f64); 4]) -> TerrainPlot {
        TerrainPlot {
            id: id.into(),
            corners,
            street_facing_edge: (0, 1),
            area: TerrainPlot::calculate_area(&corners),
            adjacent_street_id: "s".into(),
            zone: "urban".into(),
        }
    }

    fn road(points: Vec<(f64, f64)>, width: f64) -> RoadSegment {
        RoadSegment { id: "r".into(), road_type: RoadType::Arterial, points, width }
    }

    // Far from the centre, so no ring is near.
    const FAR: (f64, f64) = (-10_000.0, -10_000.0);

    #[test]
    fn a_road_along_one_side_trims_that_side() {
        // A 40x40 lot whose left 10 units lie under a 20-wide road up x = -10.
        let plots = vec![lot("a", [(-10.0, 0.0), (30.0, 0.0), (30.0, 40.0), (-10.0, 40.0)])];
        let roads = vec![road(vec![(-10.0, -100.0), (-10.0, 100.0)], 20.0)];
        let out = finish_lots(plots, &roads, &[], FAR);
        assert_eq!(out.len(), 1);
        for &(x, _) in &out[0].corners {
            assert!(x >= -0.01, "corner left at x = {x}");
        }
        assert!((out[0].area - 1200.0).abs() < 5.0, "area {}", out[0].area);
    }

    #[test]
    fn a_road_through_the_middle_drops_the_lot() {
        let plots = vec![lot("a", [(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)])];
        let roads = vec![road(vec![(20.0, -100.0), (20.0, 100.0)], 6.0)];
        assert!(finish_lots(plots, &roads, &[], FAR).is_empty());
    }

    #[test]
    fn repeated_ids_get_suffixes() {
        let sq = |x: f64| [(x, 0.0), (x + 10.0, 0.0), (x + 10.0, 10.0), (x, 10.0)];
        let plots = vec![lot("a", sq(0.0)), lot("b", sq(20.0)), lot("a", sq(40.0)), lot("a", sq(60.0))];
        let ids: Vec<_> = finish_lots(plots, &[], &[], FAR).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["a", "b", "a_2", "a_3"]);
    }

    #[test]
    fn lots_stay_out_of_the_water_and_its_kerb() {
        // A ring at r = 100, 20 wide: water 90..110, kerbed 80..120. A lot from r 115 to 155.
        let rings = vec![WaterRing { id: "c".into(), radius: 100.0, width: 20.0, bridges: vec![] }];
        let plots = vec![lot("a", [(115.0, -20.0), (115.0, 20.0), (155.0, 20.0), (155.0, -20.0)])];
        let out = finish_lots(plots, &[], &rings, (0.0, 0.0));
        assert_eq!(out.len(), 1);
        for &(x, y) in &out[0].corners {
            assert!((x * x + y * y).sqrt() >= 119.99, "corner at r = {}", (x * x + y * y).sqrt());
        }
    }

    #[test]
    fn generated_lots_are_clear_and_unique() {
        use crate::models::{TerrainConfig, WorldGenParams};
        // The defaults, the wider canals the dev DB commits, and a few other
        // seeds, which lay out other suburban streets (#62).
        let wide = WorldGenParams {
            canal_inner_width: 50.0,
            canal_outer_width: 50.0,
            moat_width: 30.0,
            ..WorldGenParams::default()
        };
        let seeded = [7, 1234, 99_999].map(|seed| WorldGenParams { seed, ..WorldGenParams::default() });
        for params in [WorldGenParams::default(), wide].into_iter().chain(seeded) {
            let t = TerrainConfig::from_params(&params);
            let obstacles = Obstacles::new(&t.roads, &t.water_rings, (t.center_x, t.center_y));
            let mut ids = std::collections::HashSet::new();
            let mut zones: HashMap<&str, usize> = HashMap::new();
            for p in &t.terrain_plots {
                assert!(ids.insert(p.id.as_str()), "repeated id {}", p.id);
                let (min, max) = bounds(&p.corners);
                assert!(clear(&p.corners, &obstacles, &obstacles.near(min, max)), "{} isn't clear", p.id);
                *zones.entry(p.zone.as_str()).or_default() += 1;
            }
            eprintln!("{} lots: {:?}", t.terrain_plots.len(), zones);
            // Plots v2's rings (#459): about 1000 urban lots (#455), 1000
            // suburban (#456) and 900 farms (#457).
            assert!(zones["urban"] > 900 && zones["suburban"] > 900 && zones["rural"] > 800, "{zones:?}");
            assert!(zones.get("guild_district").copied().unwrap_or(0) >= 40, "{zones:?}");
        }
    }
}
