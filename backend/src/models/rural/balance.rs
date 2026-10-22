//! The balance pass (plan RD7, #287): spurs into the ring's empty ground.
//!
//! The roads are planned, and then provisional farms are laid on them,
//! ignoring the water (`farms::lay_sector`). The farms and roads go onto a
//! raster of each sector, the parks' way (`suburb::parks`), and a chamfer
//! finds the largest open disc in each of its two half-sectors. While a
//! half-sector is built under [`BUILT_MIN`] and a hole wider than
//! [`HOLE_RADIUS`] is left in it, a spur runs toward the hole from the
//! nearest of the sector's roads: straight in a concession sector, winding in
//! a winding one (`roads::spur_to`). It ends at a farm gate. A sector takes
//! at most [`MAX_SPURS`].
//!
//! The pass runs before the water, so the roads are final when the water
//! skirts them. The farms the water costs aren't made up for: a farm lost to
//! a pond leaves a field.

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot};

use super::farms;
use super::roads::{self, Ring};
use super::sectors::{Sector, RURAL};
use super::RuralParams;
use crate::models::geom::SplitMix64;
use crate::models::lot_geometry::{dist_to_seg, Region};
use crate::models::suburb::parks::chamfer_with;
use crate::models::terrain::WorldGenParams;

type P = (f64, f64);

/// A half-sector built under this share wants a spur. The plan's hard floor
/// is 20% (§5); the margin is for the farms the water takes afterwards.
pub const BUILT_MIN: f64 = 0.25;
/// A spur goes only toward a hole wider than this, about a farm deep each
/// side of a road.
pub const HOLE_RADIUS: f64 = 380.0;
/// The most spurs the pass adds to a sector.
pub const MAX_SPURS: usize = 3;
/// The raster's square.
const CELL: f64 = 10.0;
/// The balance streams' key after the sector's, "BAL" in ASCII.
const BALANCE: u64 = 0x42_414c;

/// One half-sector's ground, as the pass sees it.
#[derive(Debug, Clone, Copy)]
pub struct Half {
    /// The share of its ring a farm stands on.
    pub built: f64,
    /// Its largest open disc: the centre and the radius.
    pub hole: P,
    pub radius: f64,
}

/// The spurs the pass adds to every sector, as `lane_{s}_{k}` roads after
/// the sector's own, and the farm gates they end at. `roads` is the world's
/// so far, the lanes in it.
pub fn spurs(params: &WorldGenParams, center: P, roads: &[RoadSegment], sectors: &[Sector]) -> (Vec<RoadSegment>, Vec<P>) {
    // Only the roads that reach the ring matter to its farms.
    let reach = params.suburban_outer_radius - params.rural.depth.1;
    let near: Vec<RoadSegment> = roads
        .iter()
        .filter(|r| r.points.iter().any(|p| (p.0 - center.0).stable_hypot(p.1 - center.1) >= reach))
        .cloned()
        .collect();
    let ring = Ring {
        center,
        inner: params.suburban_outer_radius,
        outer: params.rural_outer_radius,
        arterial_width: params.arterial_width,
        lane_width: params.rural.lane_width,
        sway: params.rural.sway,
        lanes_follow: params.rural.lanes_follow,
    };
    // The provisional farms only measure the ground, so they skip the room
    // rule's packing, which is the slow part of laying them (V1.3, #457).
    let params = &WorldGenParams { rural: RuralParams { room: 0, ..params.rural.clone() }, ..params.clone() };
    let (mut out, mut gates) = (Vec::new(), Vec::new());
    for sector in sectors {
        let mut rng = SplitMix64::stream(params.seed, &[RURAL, sector.index as u64, BALANCE]);
        let prefix = format!("lane_{}_", sector.index);
        let first = near
            .iter()
            .filter_map(|r| r.id.strip_prefix(&prefix).and_then(|k| k.parse::<usize>().ok()))
            .max()
            .map_or(0, |k| k + 1);
        let mut world_roads = near.clone();
        for next in first..first + MAX_SPURS {
            let world = farms::world(params, center, &world_roads, None);
            let laid = farms::lay_sector(params, center, &world, sector);
            let region = farms::region(params, center, sector);
            let mut halves: Vec<Half> =
                halves(params, center, sector, &region, &laid, &world_roads).into_iter().filter(|h| h.built < BUILT_MIN && h.radius > HOLE_RADIUS).collect();
            halves.sort_by(|a, b| b.radius.total_cmp(&a.radius));
            let lanes: Vec<&[P]> = world_roads.iter().filter(|r| r.id.starts_with(&prefix)).map(|r| r.points.as_slice()).collect();
            let Some(spur) = halves.iter().find_map(|h| roads::spur_to(sector, &ring, &mut rng, &lanes, h.hole, h.radius)) else {
                break;
            };
            gates.push(*spur.last().expect("a spur has two points or more"));
            let road = RoadSegment {
                id: format!("lane_{}_{next}", sector.index),
                road_type: RoadType::Trail,
                points: spur,
                width: params.rural.lane_width,
            };
            world_roads.push(road.clone());
            out.push(road);
        }
    }
    (out, gates)
}

/// A sector's two half-sectors, from `from` to its bisector and on to `to`:
/// how built each is by `lots`, and its largest disc clear of `lots`, `roads`
/// and the region's edge.
pub fn halves(params: &WorldGenParams, center: P, sector: &Sector, region: &Region, lots: &[TerrainPlot], roads: &[RoadSegment]) -> [Half; 2] {
    let (inner, outer) = (params.suburban_outer_radius, params.rural_outer_radius);
    // The box round the sector's arcs at both radii.
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for k in 0..=32 {
        let a = sector.bearing(k as f64 / 32.0);
        for r in [inner, outer] {
            let q = super::sectors::polar(center, r, a);
            lo = (lo.0.min(q.0), lo.1.min(q.1));
            hi = (hi.0.max(q.0), hi.1.max(q.1));
        }
    }
    let origin = (lo.0 - CELL, lo.1 - CELL);
    let cols = ((hi.0 - origin.0) / CELL).ceil() as usize + 1;
    let rows = ((hi.1 - origin.1) / CELL).ceil() as usize + 1;
    let centre_of = |c: usize, r: usize| (origin.0 + (c as f64 + 0.5) * CELL, origin.1 + (r as f64 + 0.5) * CELL);
    let span = |lo: f64, hi: f64, o: f64, n: usize| {
        let a = ((lo - o) / CELL).floor().max(0.0) as usize;
        let b = (((hi - o) / CELL).ceil().max(0.0) as usize).min(n);
        a..b
    };

    // Which half each square of the sector's ring is in, if either.
    let mut half = vec![None; cols * rows];
    for r in 0..rows {
        for c in 0..cols {
            let q = centre_of(c, r);
            let d = (q.0 - center.0).stable_hypot(q.1 - center.1);
            if d < inner || d > outer {
                continue;
            }
            let a = (q.1 - center.1).stable_atan2(q.0 - center.0);
            let t = (a - sector.from).rem_euclid(std::f64::consts::TAU) / sector.span();
            if t < 1.0 {
                half[r * cols + c] = Some(usize::from(t >= 0.5));
            }
        }
    }
    let mut built = vec![false; cols * rows];
    for lot in lots {
        let q = &lot.corners;
        let (x0, x1) = q.iter().fold((f64::MAX, f64::MIN), |m, p| (m.0.min(p.0), m.1.max(p.0)));
        let (y0, y1) = q.iter().fold((f64::MAX, f64::MIN), |m, p| (m.0.min(p.1), m.1.max(p.1)));
        for r in span(y0, y1, origin.1, rows) {
            for c in span(x0, x1, origin.0, cols) {
                if inside(q, centre_of(c, r)) {
                    built[r * cols + c] = true;
                }
            }
        }
    }
    // Open ground is in the region, off the farms and off the roads.
    let mut open: Vec<bool> = (0..cols * rows).map(|i| !built[i] && region.inside(centre_of(i % cols, i / cols))).collect();
    for road in roads {
        let h = road.width / 2.0;
        for w in road.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            for r in span(a.1.min(b.1) - h, a.1.max(b.1) + h, origin.1, rows) {
                for c in span(a.0.min(b.0) - h, a.0.max(b.0) + h, origin.0, cols) {
                    if dist_to_seg(centre_of(c, r), a, b) < h {
                        open[r * cols + c] = false;
                    }
                }
            }
        }
    }
    let clear = chamfer_with(cols, rows, CELL, |i| !open[i]);

    let mut out = [Half { built: 0.0, hole: center, radius: 0.0 }; 2];
    let mut counts = [(0usize, 0usize); 2];
    for i in 0..cols * rows {
        let Some(h) = half[i] else { continue };
        counts[h].1 += 1;
        counts[h].0 += usize::from(built[i]);
        if clear[i] > out[h].radius {
            out[h].radius = clear[i];
            out[h].hole = centre_of(i % cols, i / cols);
        }
    }
    for (half, (built, all)) in out.iter_mut().zip(counts) {
        half.built = if all > 0 { built as f64 / all as f64 } else { 0.0 };
    }
    out
}

/// Whether `q` is inside the quad `c`, either winding.
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
    use crate::models::rural::tests::farms_world;
    use crate::models::terrain::{point_to_segment_distance, TerrainConfig};

    /// The roads already spread the farms evenly, so on the plan's seeds the
    /// pass finds nothing to do. Take a sector's outer road away, and the
    /// band it served is a hole: the pass runs spurs into it, from the road
    /// that's left, to gates.
    #[test]
    fn a_sector_without_its_outer_road_gets_spurs_into_the_empty_band() {
        let params = farms_world(42);
        let t = TerrainConfig::from_params(&params);
        let c = (t.center_x, t.center_y);
        let sectors = crate::models::rural::sectors_of(&params, c, &t.roads);
        let (none, _) = spurs(&params, c, &t.roads, &sectors);
        assert!(none.is_empty(), "seed 42 shouldn't need a spur: {:?}", none.iter().map(|r| &r.id).collect::<Vec<_>>());

        let s = 2;
        let keep = format!("lane_{s}_0");
        let prefix = format!("lane_{s}_");
        let roads: Vec<RoadSegment> = t.roads.iter().filter(|r| !r.id.starts_with(&prefix) || r.id == keep).cloned().collect();
        let sector = &sectors[s];
        let region = farms::region(&params, c, sector);
        let world = farms::world(&params, c, &roads, None);
        let before = halves(&params, c, sector, &region, &farms::lay_sector(&params, c, &world, sector), &roads);
        assert!(before.iter().any(|h| h.built < BUILT_MIN && h.radius > HOLE_RADIUS), "{before:?}");

        let (added, gates) = spurs(&params, c, &roads, &sectors);
        assert!(!added.is_empty() && added.len() <= MAX_SPURS, "{} spurs", added.len());
        assert_eq!(added.len(), gates.len());
        let inner = &roads.iter().find(|r| r.id == keep).unwrap().points;
        for (k, (spur, gate)) in added.iter().zip(&gates).enumerate() {
            assert_eq!(spur.id, format!("lane_{s}_{}", k + 1), "numbered after the sector's own");
            assert_eq!(spur.points.last(), Some(gate));
            let start = spur.points[0];
            let on_a_road = std::iter::once(inner).chain(added[..k].iter().map(|r| &r.points)).any(|road| {
                road.windows(2).any(|w| point_to_segment_distance(start.0, start.1, w[0].0, w[0].1, w[1].0, w[1].1) < 1e-6)
            });
            assert!(on_a_road, "{} leaves no road of its sector", spur.id);
        }
        let mut with: Vec<RoadSegment> = roads.clone();
        with.extend(added);
        let world = farms::world(&params, c, &with, None);
        let after = halves(&params, c, sector, &region, &farms::lay_sector(&params, c, &world, sector), &with);
        let (was, now) = (before[0].built + before[1].built, after[0].built + after[1].built);
        assert!(now > was + 0.05, "the spurs' farms built {was:.2} up to only {now:.2}");
    }
}
