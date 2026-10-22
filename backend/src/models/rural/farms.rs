//! The farms (plan RD6, #287).
//!
//! - **The walk.** Every lane, side road and spur has farms on both sides,
//!   and each sector has the side of each arterial that faces it. A side is
//!   walked from one end with alternating draws (`RuralParams`, tuned in R5,
//!   #288, and made long lots in #423): a farm's frontage U[120, 160], then a
//!   field gap U[50, 100] before the next. Each farm's depth is drawn
//!   U[230, 310], then clipped by the region and the other roads. Where no farm fits (at a junction, or by the region's edge) the
//!   frontage is cut down to 100, and then the walk slides on.
//! - **A farm** is a trapezoid on its road's chord: its front edge runs
//!   between the two points the frontage spans, a verge off the road, and its
//!   sides run square to that edge. Where the road bows toward it the front
//!   is pushed out until the road is a verge off all along it. Lots back to back meet half way, short of
//!   it by half the gap between farms.
//! - **The verge.** A farm stands back 30 from an arterial's edge and 17
//!   from a lane's, so farms facing each other across a lane are 42 apart,
//!   past the overlap pass's 40. The region is cut along each arterial at its verge, so
//!   a lane's farms stand clear of where it meets one.
//! - **The overlap pass** is `lot_geometry::place`, with the farms 40 apart.
//!   The arterials' farms go first, so the spawn road has farms along it,
//!   then each sector's main roads, then the roads between them and to the
//!   gates. The water, when there is some, is `World::with_water`'s.
//! - **Ids** are `farm_{s}_{road}_{L|R}_{k}`: `road` is `k` of the lane
//!   `lane_{s}_{k}`, or `a0` and `a1` for the arterials at the sector's
//!   `from` and `to`. L and R are the sides along the road's points. They're
//!   numbered as the walk lays them, before the overlap pass, and each side
//!   draws from its own stream, so dropping one farm doesn't re-id the rest.

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{Lake, RoadSegment, RoadType, TerrainPlot, WaterCourse};

use super::sectors::{Sector, RURAL};
use super::RuralParams;
use crate::models::geom::{Polyline, SplitMix64};
use crate::models::station_room;
use crate::models::lot_geometry::{
    at, bounds, dist, place, point, seg_seg_dist, shrink_to_fit, tangent, Candidate, LotParams, Region, World,
};
use crate::models::terrain::WorldGenParams;

type P = (f64, f64);

/// Farms keep this far apart (RD6).
pub const LOT_GAP: f64 = 40.0;
/// A farm stands this far back from an arterial's edge...
pub const ARTERIAL_VERGE: f64 = 30.0;
/// ...and this far from a lane's: two verges and a lane are over
/// [`LOT_GAP`], so the overlap pass keeps the farms on both sides of it (R5).
pub const LANE_VERGE: f64 = 17.0;
/// The farms' ground stops this far inside the ring's edges.
pub const EDGE_INSET: f64 = 20.0;
/// Farms keep this far from a river's, a stream's or a lake's edge.
pub const BANK: f64 = 10.0;
/// A farm's frontage is cut down a step at a time, to no less than
/// `RuralParams::min_frontage`, before the walk gives up on it and slides
/// on...
const SHORTEN: f64 = 20.0;
/// ...this far.
const SLIDE: f64 = 10.0;
/// On a bend's outside, a farm's front is pushed out this much at a time,
/// to at most this past its verge.
const PUSH_STEP: f64 = 2.0;
const MAX_PUSH: f64 = 60.0;
/// The farms' streams' key after the sector's, "FARMS" in ASCII.
const FARMS: u64 = 0x46_4152_4d53;

/// The overlap pass's params: `place` takes `min_area` as a share of
/// `frontage × depth`, so those are the draws' middles.
pub(crate) fn lot_params(rural: &RuralParams) -> LotParams {
    let mid = |(lo, hi): (f64, f64)| (lo + hi) / 2.0;
    LotParams {
        frontage: mid(rural.frontage),
        depth: mid(rural.depth),
        depth_max: rural.depth.1,
        front_gap: LANE_VERGE,
        edge_gap: LOT_GAP / 2.0,
        rear_gap: LOT_GAP / 2.0,
        lot_gap: LOT_GAP,
        min_shrink: 0.6,
        min_area: 0.4,
    }
}

/// The water farms keep clear of: rivers, streams and lakes.
#[derive(Clone, Copy)]
pub struct Water<'a> {
    pub rivers: &'a [WaterCourse],
    pub streams: &'a [WaterCourse],
    pub lakes: &'a [Lake],
}

/// Whether `road` fronts farms: a lane, or an arterial into the ring.
pub fn fronts_farms(road: &RoadSegment) -> bool {
    super::is_lane(road) || is_arterial(road)
}

/// One of the eight arterials out of the guild ring, not the inner stubs.
fn is_arterial(road: &RoadSegment) -> bool {
    road.road_type == RoadType::Arterial && road.id.strip_prefix("arterial_").is_some_and(|n| n.parse::<u32>().is_ok())
}

/// Where sector `sector`'s farms may go: the ring inset by [`EDGE_INSET`],
/// between its arterials' verges.
pub(crate) fn region(params: &WorldGenParams, center: P, sector: &Sector) -> Region {
    // A hair inside the verge, so an arterial's farms, a verge off it, are in.
    let inset = params.arterial_width / 2.0 + ARTERIAL_VERGE - 1.0;
    Region::ring(center, params.suburban_outer_radius + EDGE_INSET, params.rural_outer_radius - EDGE_INSET)
        .ccw_of(sector.from, inset)
        .cw_of(sector.to, inset)
}

/// The world the farms are laid in: every road, those with farms fronted on
/// both sides, and the water if there's some.
pub(crate) fn world<'a>(params: &WorldGenParams, center: P, roads: &'a [RoadSegment], water: Option<Water<'a>>) -> World<'a> {
    let sides = roads.iter().map(|r| if fronts_farms(r) { (true, true) } else { (false, false) }).collect();
    let world = World::new(roads, sides, center, params.rural_outer_radius + 2.0 * params.rural.depth.1);
    match water {
        Some(w) => world.with_water(w.rivers.iter().chain(w.streams), w.lakes, BANK),
        None => world,
    }
}

/// Every sector's farms, after the overlap pass.
pub fn lay(params: &WorldGenParams, center: P, roads: &[RoadSegment], sectors: &[Sector], water: Option<Water>) -> Vec<TerrainPlot> {
    let world = world(params, center, roads, water);
    lay_in(params, center, &world, sectors)
}

/// One sector's farms, after the overlap pass. No farm of one sector comes
/// near another's, so this is `lay`'s for the sector.
pub(crate) fn lay_sector(params: &WorldGenParams, center: P, world: &World, sector: &Sector) -> Vec<TerrainPlot> {
    lay_in(params, center, world, std::slice::from_ref(sector))
}

/// The farms of `sectors` in `world`: every road's walk, then one overlap
/// pass over them in class order, and only those with room kept.
fn lay_in(params: &WorldGenParams, center: P, world: &World, sectors: &[Sector]) -> Vec<TerrainPlot> {
    let mut candidates: Vec<(u8, Candidate)> = sectors.iter().flat_map(|s| candidates(params, center, world, s)).collect();
    // Stable, so each class keeps sector and road order.
    candidates.sort_by_key(|&(class, _)| class);
    let placed = place(candidates.into_iter().map(|(_, c)| c), world, &lot_params(&params.rural), "rural");
    with_room(&params.rural, placed)
}

/// Whether a farm with these corners has the room `RuralParams::room` asks
/// of one: always, when it asks none.
fn roomy(draws: &RuralParams, corners: &[P; 4]) -> bool {
    draws.room == 0 || station_room::takes(corners, draws.room)
}

/// The farms the overlap pass left with the room a farm needs. It may have
/// pulled one back short of it, and that one is left as a field.
fn with_room(draws: &RuralParams, farms: Vec<TerrainPlot>) -> Vec<TerrainPlot> {
    if draws.room == 0 {
        return farms;
    }
    farms.into_iter().filter(|f| roomy(draws, &f.corners)).collect()
}

/// A sector's farms before the overlap pass, each with its class: 0 on an
/// arterial, 1 on a main road, 2 on the rest.
fn candidates(params: &WorldGenParams, center: P, world: &World, sector: &Sector) -> Vec<(u8, Candidate)> {
    let s = sector.index;
    let region = region(params, center, sector);
    let mut out = Vec::new();

    // The arterials at the sector's sides, each on the side that faces it.
    let bearing = |p: P| (p.1 - center.1).stable_atan2(p.0 - center.0).rem_euclid(std::f64::consts::TAU);
    let same = |a: f64, b: f64| (a - b).rem_euclid(std::f64::consts::TAU).min((b - a).rem_euclid(std::f64::consts::TAU)) < 1e-6;
    for (tag, edge) in [("a0", sector.from), ("a1", sector.to)] {
        let Some(road) = world.roads.iter().position(|r| is_arterial(r) && r.points.last().is_some_and(|&p| same(bearing(p), edge)))
        else {
            continue;
        };
        let line = Polyline::new(world.roads[road].points.clone());
        let mid = super::sectors::polar(center, (params.suburban_outer_radius + params.rural_outer_radius) / 2.0, edge);
        let walk = Walk { world, region: &region, road, verge: ARTERIAL_VERGE, draws: &params.rural };
        for (side, letter) in [(1.0, "L"), (-1.0, "R")] {
            // The side whose verge is in this sector's region.
            let toward = walk.normal(&line, side, line.length() / 2.0);
            if !region.inside(at(mid, toward, params.arterial_width / 2.0 + ARTERIAL_VERGE + 1.0)) {
                continue;
            }
            let mut rng = SplitMix64::stream(params.seed, &[RURAL, s as u64, FARMS, 1000 + (tag == "a1") as u64, side_key(side)]);
            let farms = walk.farms(&line, side, &mut rng, |k| format!("farm_{s}_{tag}_{letter}_{k}"));
            out.extend(farms.into_iter().map(|c| (0, c)));
        }
    }

    // The sector's own roads.
    let prefix = format!("lane_{s}_");
    for (road, r) in world.roads.iter().enumerate() {
        let Some(k) = r.id.strip_prefix(&prefix).and_then(|k| k.parse::<u64>().ok()) else { continue };
        let line = Polyline::new(r.points.clone());
        let walk = Walk { world, region: &region, road, verge: LANE_VERGE, draws: &params.rural };
        let class = if k < params.rural.mains.len() as u64 { 1 } else { 2 };
        for (side, letter) in [(1.0, "L"), (-1.0, "R")] {
            let mut rng = SplitMix64::stream(params.seed, &[RURAL, s as u64, FARMS, k, side_key(side)]);
            let farms = walk.farms(&line, side, &mut rng, |n| format!("farm_{s}_{k}_{letter}_{n}"));
            out.extend(farms.into_iter().map(|c| (class, c)));
        }
    }
    out
}

fn side_key(side: f64) -> u64 {
    if side > 0.0 {
        0
    } else {
        1
    }
}

/// One side of one road, walked.
struct Walk<'a> {
    world: &'a World<'a>,
    region: &'a Region,
    road: usize,
    verge: f64,
    /// The frontage, field gap and depth draws.
    draws: &'a RuralParams,
}

impl Walk<'_> {
    fn half(&self) -> f64 {
        self.world.roads[self.road].width / 2.0
    }

    /// The road's normal at `s` on `side` (1 left, −1 right).
    fn normal(&self, line: &Polyline, side: f64, s: f64) -> P {
        let t = tangent(line, s);
        (-t.1 * side, t.0 * side)
    }

    /// The farms down one side, as the walk lays them.
    fn farms(&self, line: &Polyline, side: f64, rng: &mut SplitMix64, id: impl Fn(usize) -> String) -> Vec<Candidate> {
        let length = line.length();
        let mut out = Vec::new();
        let mut s = 0.0;
        while s + self.draws.min_frontage <= length {
            if !self.starts(line, side, s) {
                s += SLIDE;
                continue;
            }
            let (frontage, depth) = (rng.range(self.draws.frontage.0, self.draws.frontage.1), rng.range(self.draws.depth.0, self.draws.depth.1));
            let mut w = frontage.min(length - s);
            let mut laid = None;
            while w >= self.draws.min_frontage {
                if let Some(corners) = self.farm(line, side, s, w, depth).filter(|c| roomy(self.draws, c)) {
                    laid = Some(corners);
                    break;
                }
                w -= SHORTEN;
            }
            match laid {
                Some(corners) => {
                    out.push(Candidate { id: id(out.len()), road: self.road, corners });
                    s += w + rng.range(self.draws.field_gap.0, self.draws.field_gap.1);
                }
                None => s += SLIDE,
            }
        }
        out
    }

    /// Whether a farm could start at `s`: its corner there, a verge off the
    /// road, is in the region and clear.
    fn starts(&self, line: &Polyline, side: f64, s: f64) -> bool {
        let q = at(point(line, s), self.normal(line, side, s), self.half() + self.verge);
        self.region.inside(q) && self.world.point_clear(q)
    }

    /// The farm `w` along from `s`, `depth` deep, if one fits there.
    fn farm(&self, line: &Polyline, side: f64, s: f64, w: f64, depth: f64) -> Option<[P; 4]> {
        let (p0, p1) = (point(line, s), point(line, s + w));
        let chord = dist(p0, p1);
        // A road that doubles back has no room for a farm across the bend.
        if chord < 0.9 * w {
            return None;
        }
        let e = ((p1.0 - p0.0) / chord, (p1.1 - p0.1) / chord);
        let n = (-e.1 * side, e.0 * side);
        let half = self.half();
        // Off the bend: the front a verge off its own road all along, even
        // where the road bows toward it between the two ends. Half a verge
        // there let the farms across a lane come within the gap (R5).
        let own_clear = |a: P, b: P| {
            let (lo, hi) = bounds(&[a, b]);
            self.world.near(lo, hi).filter(|g| g.road == self.road).all(|g| seg_seg_dist(a, b, g.a, g.b) >= g.half + self.verge - 1e-6)
        };
        let mut f = half + self.verge;
        while !own_clear(at(p0, n, f), at(p1, n, f)) {
            f += PUSH_STEP;
            if f > half + self.verge + MAX_PUSH {
                return None;
            }
        }
        let fronts = [at(p0, n, f), at(p1, n, f)];
        if !fronts.iter().all(|&q| self.region.inside(q) && self.world.point_clear(q)) || !self.world.edge_clear(fronts[0], fronts[1]) {
            return None;
        }
        // As deep as the draw, or half way to the farms behind, or short of a
        // road without farms, the water, or the region's edge.
        let p = LotParams { depth, depth_max: depth, ..lot_params(self.draws) };
        let backs = [p0, p1].map(|o| self.world.back_from(self.region, o, n, f, half, depth, &p).min(f + depth));
        let shallow = backs[0].min(backs[1]) - f;
        let min_depth = self.draws.min_depth;
        if shallow < min_depth {
            return None;
        }
        // A road or the water that runs in between the sides: pulled back
        // short of it here, so the walk can cut the frontage or slide on past
        // it, rather than the overlap pass drop the farm and leave a field.
        let corners = [fronts[0], fronts[1], at(p1, n, backs[1]), at(p0, n, backs[0])];
        shrink_to_fit(corners, min_depth / shallow, |q| self.world.quad_clear(q))
    }
}

#[cfg(test)]
mod tests {
    use capitol_protocol::fmath::StableMath;

    use super::*;
    use crate::models::rural::metrics;
    use crate::models::rural::tests::farms_world;
    use crate::models::terrain::{point_to_segment_distance, TerrainConfig};

    /// The four seeds the plan measures (§5).
    const SEEDS: [u64; 4] = [42, 7, 1234, 99_999];

    /// R4's acceptance: under `Farms`, no farm on the water, none within 40
    /// of another, and no half-sector under the plan's floor of 20% built.
    #[test]
    fn the_farms_keep_off_the_water_and_apart_and_fill_every_half_sector() {
        for seed in SEEDS {
            let m = metrics::measure_world(&farms_world(seed), None).measured.metrics;
            assert_eq!(m.lots_on_water, 0, "seed {seed}: farms on the water");
            assert_eq!(m.overlaps, 0, "seed {seed}: farms overlap");
            assert!(m.gap_min.unwrap() >= LOT_GAP - 1e-6, "seed {seed}: two farms {:.1} apart", m.gap_min.unwrap());
            let least = m.half_sector_min.unwrap();
            assert!(least >= 0.20, "seed {seed}: a half-sector {least:.2} built, of {:?}", m.half_sector_shares);
            assert!((850..=1050).contains(&m.lots), "seed {seed}: {} farms", m.lots);
        }
    }

    /// The ids are the plan's, every farm stands a verge back from the road it
    /// fronts, and the arterials have farms too, the spawn road among them.
    #[test]
    fn farms_front_the_lanes_and_the_arterials_a_verge_back() {
        let params = farms_world(42);
        let t = TerrainConfig::from_params(&params);
        let c = (t.center_x, t.center_y);
        let farms: Vec<&TerrainPlot> = t.terrain_plots.iter().filter(|l| l.zone == "rural").collect();
        assert!(!farms.is_empty());
        for farm in &farms {
            let parts: Vec<&str> = farm.id.split('_').collect();
            let well_formed = parts.len() == 5
                && parts[0] == "farm"
                && parts[1].parse::<u8>().is_ok_and(|s| s < 8)
                && (parts[2] == "a0" || parts[2] == "a1" || parts[2].parse::<u32>().is_ok())
                && (parts[3] == "L" || parts[3] == "R")
                && parts[4].parse::<u32>().is_ok();
            assert!(well_formed, "{} isn't farm_s_road_side_k", farm.id);

            let road = t.roads.iter().find(|r| r.id == farm.adjacent_street_id).expect("a farm's road is in the world");
            let verge = if road.road_type == RoadType::Arterial { ARTERIAL_VERGE } else { LANE_VERGE };
            let off = |q: P| {
                road.points
                    .windows(2)
                    .map(|w| point_to_segment_distance(q.0, q.1, w[0].0, w[0].1, w[1].0, w[1].1))
                    .fold(f64::INFINITY, f64::min)
            };
            for q in [farm.corners[0], farm.corners[1]] {
                // finish_lots may trim a hair off.
                assert!(off(q) >= road.width / 2.0 + verge - 0.5, "{}'s front is {:.1} off {}", farm.id, off(q), road.id);
            }
        }
        let on_arterials = farms.iter().filter(|f| f.id.split('_').nth(2).is_some_and(|r| r.starts_with('a'))).count();
        assert!(on_arterials >= 16, "only {on_arterials} farms front the arterials");
        // The spawn road runs south from the tower: bearing 90°, y down.
        let spawn = t
            .roads
            .iter()
            .find(|r| {
                is_arterial(r)
                    && r.points.last().is_some_and(|p| ((p.1 - c.1).stable_atan2(p.0 - c.0).to_degrees() - 90.0).abs() < 1e-6)
            })
            .expect("a south arterial");
        let along = farms.iter().filter(|f| f.adjacent_street_id == spawn.id).count();
        assert!(along >= 2, "{along} farms along the spawn road, {}", spawn.id);
    }

    /// The farms ignore the water until they're given it, and given it they
    /// keep off it: the dry walk lays farms where the water runs.
    #[test]
    fn the_real_farms_are_laid_round_the_water() {
        let params = farms_world(1234);
        let t = TerrainConfig::from_params(&params);
        let c = (t.center_x, t.center_y);
        let sectors = crate::models::rural::sectors_of(&params, c, &t.roads);
        let water = Water { rivers: &t.rivers, streams: &t.streams, lakes: &t.lakes };
        let dry = lay(&params, c, &t.roads, &sectors, None);
        let wet = lay(&params, c, &t.roads, &sectors, Some(water));
        let wet_world = world(&params, c, &t.roads, Some(water));
        let dry_on_water = dry.iter().filter(|l| !wet_world.quad_clear(&l.corners)).count();
        assert!(dry_on_water > 0, "seed 1234's water should run under some of the dry farms");
        assert!(wet.iter().all(|l| wet_world.quad_clear(&l.corners)), "a real farm on the water");
    }
}
