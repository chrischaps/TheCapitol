//! The rural ring, r3000 to r4600 (`the-capitol-rural.md`, RD10).
//!
//! Rural v2's farms on lanes, since the flip (R7, #290). R8 (#291) removed
//! the old radial trails they replaced. The modules:
//! - `sectors`: the sectors between the arterials and the deck of their
//!   characters (RD1, RD2, #286);
//! - `roads`: each sector's two concession roads or winding lanes, the roads
//!   between them, and the roads to farm gates (RD3 to RD5, #286; RD10, #310);
//! - `farms`: the farms along every lane and the arterials (RD6, #287);
//! - `balance`: spurs into the ground the farms leave open (RD7, #287);
//! - `metrics`: the ring's layout numbers and the plan's §5 targets (#284);
//! - `render`: the plan-view SVG and the `rural_plan_report` test (#284).
//!
//! The last two are test-only, as the suburb's are.

pub mod balance;
pub mod farms;
#[cfg(test)]
pub(crate) mod metrics;
#[cfg(test)]
mod render;
pub mod roads;
pub mod sectors;

use capitol_protocol::terrain::RoadSegment;
use serde::{Deserialize, Serialize};

use crate::models::geom::SplitMix64;
use crate::models::terrain::WorldGenParams;
use sectors::{Sector, RURAL};

type P = (f64, f64);

/// The rural ring's params: two concession roads or winding lanes across
/// every sector, the roads between them (#286, #310), and farms along them
/// and the arterials, clear of the water (#287). Every field has a default,
/// so a saved `world_config` from before them still loads. One that still
/// names the old switch (`layout`, even `trails`) loads as the farms: serde
/// skips it (#291).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuralParams {
    /// The lanes, side roads and spurs (RD5).
    pub lane_width: f64,
    /// A farm's frontage along its road, drawn in this range (RD6)...
    pub frontage: (f64, f64),
    /// ...the field before the next farm...
    pub field_gap: (f64, f64),
    /// ...and its depth, before the region and the other roads clip it.
    pub depth: (f64, f64),
    /// Where a farm doesn't fit, its frontage is cut down to no less than
    /// this before the walk slides on...
    pub min_frontage: f64,
    /// ...and a farm shallower than this at either side isn't one.
    pub min_depth: f64,
    /// The stations a farm must take at 50 spacing (`station_room::takes`),
    /// or the walk cuts it down or slides on, and the overlap pass leaves
    /// one it pulls back short of it as a field. 0 asks none, as today.
    pub room: usize,
    /// How far the winding lanes sway, as a share of today's (RD4).
    pub sway: f64,
    /// A winding sector's main lanes follow one another: one set of sines
    /// for them all, so they run alongside as contour lanes do, rather than
    /// each sway on its own and pinch the farms between them (RD4).
    pub lanes_follow: bool,
    /// The main roads across every sector, from arterial to arterial, inner
    /// first: the band of the ring's depth each keeps to, as shares of it
    /// (RD10). Two today, at 330 to 470 and 1130 to 1270 into the 1600-deep
    /// ring; Plots v2's ring takes three (V1.3, #457).
    pub mains: Vec<(f64, f64)>,
}

/// The ring's depth today's bands were drawn in, so their shares give the
/// same roads (RD10).
const DEPTH_TODAY: f64 = 1600.0;

impl Default for RuralParams {
    fn default() -> Self {
        Self {
            lane_width: 8.0,
            frontage: (120.0, 160.0),
            field_gap: (50.0, 100.0),
            depth: (230.0, 310.0),
            min_frontage: 100.0,
            min_depth: 110.0,
            room: 0,
            sway: 1.0,
            lanes_follow: false,
            mains: vec![
                (330.0 / DEPTH_TODAY, 470.0 / DEPTH_TODAY),
                (1130.0 / DEPTH_TODAY, 1270.0 / DEPTH_TODAY),
            ],
        }
    }
}

impl RuralParams {
    /// Plots v2's farms (`the-capitol-plots-v2.md`, V1.3, #457), for its
    /// ring of r3600 to r6100: three main roads a sector, evenly down the
    /// 2500 of depth, and narrower farms closer along them, about 1000 of
    /// about 34k. Test-only until the flip (#459) makes them the default.
    #[cfg(test)]
    pub fn v2() -> Self {
        Self {
            frontage: (132.0, 146.0),
            field_gap: (40.0, 46.0),
            depth: (230.0, 290.0),
            min_frontage: 132.0,
            min_depth: 185.0,
            room: 12,
            sway: 0.5,
            lanes_follow: true,
            mains: vec![(0.113, 0.147), (0.36, 0.394), (0.606, 0.64), (0.853, 0.887)],
            ..Self::default()
        }
    }
}

/// The ring's roads, before the weld.
#[derive(Debug, Default)]
pub struct Plan {
    pub sectors: Vec<Sector>,
    /// `lane_{s}_{k}`: each sector's main roads are `k = 0` (the inner one) up
    /// to one less than `RuralParams::mains` has, and the roads between them
    /// and to a gate come next. The balance pass's spurs come after a
    /// sector's own.
    pub roads: Vec<RoadSegment>,
    /// Where the side roads and spurs end. A loose end is allowed only here.
    pub gates: Vec<P>,
}

/// The ring's sectors, cut by the arterials in `roads`, with their characters.
pub fn sectors_of(params: &WorldGenParams, center: P, roads: &[RoadSegment]) -> Vec<Sector> {
    let bearings = sectors::arterial_bearings(roads, center, params.suburban_outer_radius);
    sectors::sectors(params.seed, &bearings)
}

/// Plans the ring's roads. `roads` needs only the arterials, which cut the
/// ring into its sectors.
pub fn plan(params: &WorldGenParams, center: P, roads: &[RoadSegment]) -> Plan {
    let sectors = sectors_of(params, center, roads);
    let ring = roads::Ring {
        center,
        inner: params.suburban_outer_radius,
        outer: params.rural_outer_radius,
        arterial_width: params.arterial_width,
        lane_width: params.rural.lane_width,
        sway: params.rural.sway,
        lanes_follow: params.rural.lanes_follow,
    };
    let mut plan = Plan::default();
    for sector in &sectors {
        let mut rng = SplitMix64::stream(params.seed, &[RURAL, sector.index as u64]);
        let laid = roads::lay(sector, &ring, &params.rural.mains, &mut rng);
        plan.roads.extend(laid.roads);
        plan.gates.extend(laid.gates);
    }
    plan.sectors = sectors;
    plan
}

/// [`plan`], then the balance pass's spurs (RD7). `roads` is the world's so
/// far, without the ring's.
pub fn plan_balanced(params: &WorldGenParams, center: P, roads: &[RoadSegment]) -> Plan {
    let mut plan = plan(params, center, roads);
    let all: Vec<RoadSegment> = roads.iter().chain(&plan.roads).cloned().collect();
    let (spurs, gates) = balance::spurs(params, center, &all, &plan.sectors);
    plan.roads.extend(spurs);
    plan.gates.extend(gates);
    plan
}

/// Whether `road` is one of the farms' lanes, for the weld.
pub fn is_lane(road: &RoadSegment) -> bool {
    road.id.starts_with("lane_")
}

#[cfg(test)]
pub(crate) mod tests {
    use capitol_protocol::fmath::StableMath;

    use super::sectors::Character;
    use super::*;
    use crate::models::street_plan;
    use crate::models::terrain::TerrainConfig;

    /// The four seeds the plan measures (§5).
    const SEEDS: [u64; 4] = [42, 7, 1234, 99_999];

    pub(crate) fn farms_world(seed: u64) -> WorldGenParams {
        WorldGenParams { seed, ..Default::default() }
    }

    fn center(t: &TerrainConfig) -> P {
        (t.center_x, t.center_y)
    }

    fn same(a: P, b: P) -> bool {
        (a.0 - b.0).stable_hypot(a.1 - b.1) < 1e-6
    }

    #[test]
    fn a_world_config_without_rural_params_loads_as_the_farms() {
        let mut json = serde_json::to_value(WorldGenParams::default()).unwrap();
        json.as_object_mut().unwrap().remove("rural");
        let params: WorldGenParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.rural.lane_width, 8.0);
        assert_eq!(params.rural.frontage, RuralParams::default().frontage);
    }

    /// A saved `world_config` from before R8 (#291) names the old trails'
    /// params and the layout switch, even `trails`. It loads as the farms,
    /// and builds the default world.
    #[test]
    fn a_world_config_naming_the_old_trails_loads_as_the_farms() {
        let default = TerrainConfig::from_params(&WorldGenParams::default()).version();
        let mut json = serde_json::to_value(WorldGenParams::default()).unwrap();
        let object = json.as_object_mut().unwrap();
        for (key, value) in [("rural_trail_width", 6.0), ("rural_trail_count", 12.0), ("rural_plot_frontage", 200.0), ("rural_plot_depth", 150.0)] {
            object.insert(key.into(), value.into());
        }
        json["rural"]["layout"] = "trails".into();
        let params: WorldGenParams = serde_json::from_value(json).unwrap();
        let t = TerrainConfig::from_params(&params);
        assert!(t.roads.iter().all(|r| !r.id.starts_with("trail_")), "the old trails are back");
        assert!(t.terrain_plots.iter().all(|l| !l.id.starts_with("rural_plot_")), "the old lots are back");
        assert_eq!(t.version(), default);
    }

    #[test]
    fn every_seed_deals_at_least_three_sectors_of_each_character() {
        // The world's own arterials cut eight sectors...
        let t = TerrainConfig::from_params(&farms_world(42));
        let bearings = sectors::arterial_bearings(&t.roads, center(&t), 3000.0);
        assert_eq!(bearings.len(), 8);
        for (i, b) in bearings.iter().enumerate() {
            assert!((b.to_degrees() - 45.0 * i as f64).abs() < 1e-6, "arterial {i} at {:.3}°", b.to_degrees());
        }
        // ...and every seed deals them evenly.
        for seed in SEEDS.into_iter().chain(0..200) {
            let dealt = sectors::sectors(seed, &bearings);
            for character in [Character::Concession, Character::Winding] {
                let n = dealt.iter().filter(|s| s.character == character).count();
                assert!(n >= 3, "seed {seed}: {n} {character:?} sectors");
            }
        }
    }

    /// R3's and RW3's acceptance: under `Farms`, the audit is clean on the
    /// lanes but for loose ends at farm gates. Every sector has two main
    /// roads that meet an arterial at a shared vertex at both ends, and at
    /// least one road between them. Every other road starts on a vertex of a
    /// main road, and ends on the other's or at a gate, but for the balance
    /// pass's spurs (#287), which may leave any of the sector's roads.
    #[test]
    fn the_lanes_are_clean_and_join_the_arterials() {
        for seed in SEEDS {
            let params = farms_world(seed);
            let t = TerrainConfig::from_params(&params);
            assert!(t.roads.iter().all(|r| !r.id.starts_with("trail_")), "seed {seed}: the old trails are still there");
            let gates = t.generate_roads_and_rural_plan(&params).1.gates;
            let audit = street_plan::audit(&t.roads, is_lane);
            let off_gate: Vec<_> = audit.loose_ends.iter().filter(|(_, p)| !gates.iter().any(|&g| same(g, *p))).collect();
            assert!(
                audit.short_segments.is_empty()
                    && audit.sharp_turns.is_empty()
                    && audit.self_crossings.is_empty()
                    && audit.unjoined_crossings.is_empty()
                    && off_gate.is_empty(),
                "seed {seed}: {audit:?}, loose ends off a gate {off_gate:?}"
            );
            assert_eq!(audit.loose_ends.len(), gates.len(), "seed {seed}: every gate is a loose end");

            let arterials: Vec<_> =
                t.roads.iter().filter(|r| r.id.starts_with("arterial_") && !r.id.starts_with("arterial_inner")).collect();
            let road = |id: &str| t.roads.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("seed {seed}: no {id}"));
            let on = |r: &RoadSegment, p: P| r.points.iter().any(|&q| same(q, p));
            for s in 0..8 {
                let mains = [road(&format!("lane_{s}_0")), road(&format!("lane_{s}_1"))];
                for main in mains {
                    for end in [main.points[0], *main.points.last().unwrap()] {
                        let on_arterial = arterials.iter().any(|a| on(a, end));
                        assert!(on_arterial, "seed {seed}: {} ends at {end:?}, not on an arterial's vertex", main.id);
                    }
                }
                let prefix = format!("lane_{s}_");
                for lane in t.roads.iter().filter(|r| r.id.starts_with(&prefix) && mains.iter().all(|m| m.id != r.id)) {
                    let (start, end) = (lane.points[0], *lane.points.last().unwrap());
                    let from = mains.iter().position(|m| on(m, start));
                    let at_gate = gates.iter().any(|&g| same(g, end));
                    let off_another = t.roads.iter().any(|r| r.id.starts_with(&prefix) && r.id != lane.id && on(r, start));
                    assert!(from.is_some() || (at_gate && off_another), "seed {seed}: {} starts on no road of its sector", lane.id);
                    let joins = from.is_some_and(|i| on(mains[1 - i], end));
                    assert!(joins || at_gate, "seed {seed}: {} ends on neither road nor gate", lane.id);
                }
                let counted = metrics::sector_roads(&t.roads, s);
                assert!(counted.mains == 2 && counted.links >= 1, "seed {seed}: sector {s} has {counted:?}");
            }
        }
    }

    /// R5's acceptance (plan §5): every hard target holds on the four seeds,
    /// and the soft ones missed are printed, as the suburb's are.
    #[test]
    fn farms_meet_their_targets() {
        let missed = |v: &[crate::models::suburb::metrics::Violation]| -> Vec<String> {
            v.iter().map(|v| format!("{} {} (target {})", v.label, crate::models::suburb::metrics::fmt_num(v.value), v.target)).collect()
        };
        let mut failed = Vec::new();
        for seed in SEEDS {
            let m = metrics::measure_world(&farms_world(seed), None).measured.metrics;
            // A metric with nothing to measure misses nothing, so the hard
            // ones must all have been measured.
            for t in metrics::TARGETS.iter().filter(|t| t.hard) {
                assert!(m.value(t.metric).is_some(), "seed {seed}: {} wasn't measured", t.label);
            }
            let (hard, soft): (Vec<_>, Vec<_>) = metrics::violations(&m).into_iter().partition(|v| v.hard);
            eprintln!(
                "seed {seed}: {} farms of {:.0}k, {:.0}% built, half-sectors {:.0} to {:.0}% (CV {:.2}); soft misses: {:?}",
                m.lots,
                m.lot_area_mean.unwrap() / 1000.0,
                m.built_share.unwrap() * 100.0,
                m.half_sector_min.unwrap() * 100.0,
                m.half_sector_max.unwrap() * 100.0,
                m.half_sector_cv.unwrap(),
                missed(&soft)
            );
            failed.extend(hard.iter().map(|v| format!("seed {seed}: {}", missed(std::slice::from_ref(v))[0])));
        }
        assert!(failed.is_empty(), "hard misses: {failed:?}");
    }

    /// Plots v2's ring (V1.3, #457): the grown world's rings, r3600 to
    /// r6100, with `RuralParams::v2`'s farms.
    pub(crate) fn v2_world(seed: u64) -> WorldGenParams {
        WorldGenParams { seed, rural: RuralParams::v2(), ..WorldGenParams::grown() }
    }

    /// V1.3's acceptance (`the-capitol-plots-v2.md` §3): on the four seeds,
    /// Plots v2's ring holds every hard row of `metrics::V2_TARGETS`: about
    /// 1000 farms of about 34k, built 40 to 50%, nearly all taking 12
    /// stations, with today's rows for the sectors, the roads, the audit,
    /// the water and the gaps. The water still comes onto the plain: its
    /// deepest course reaches at least 700 in past the rural edge, and none
    /// comes in past the water's rim (how far it should is #472). Each seed
    /// builds a grown world of its own, so this is the one v2 test that does.
    #[test]
    fn v2_farms_meet_their_targets() {
        let mut failed = Vec::new();
        for seed in SEEDS {
            let params = v2_world(seed);
            let measured = metrics::measure_world(&params, None);
            let m = &measured.measured.metrics;
            for t in metrics::V2_TARGETS.iter().filter(|t| t.hard) {
                assert!(m.value(t.metric).is_some(), "seed {seed}: {} wasn't measured", t.label);
            }
            let frame = params.frame();
            let t = &measured.terrain;
            let reach = t.rivers.iter().chain(&t.streams).flat_map(|c| &c.points).map(|p| frame.radius(p.x, p.y)).fold(f64::MAX, f64::min);
            eprintln!(
                "seed {seed}: {} farms of {:.1}k, {:.1}% built, {:.1}% take 12, half-sectors {:.0} to {:.0}%, water in to r{reach:.0}, {:.0} ms",
                m.lots,
                m.lot_area_mean.unwrap() / 1000.0,
                m.built_share.unwrap() * 100.0,
                m.take_12_share.unwrap() * 100.0,
                m.half_sector_min.unwrap() * 100.0,
                m.half_sector_max.unwrap() * 100.0,
                m.gen_ms
            );
            assert!(reach <= frame.rural_edge() - 700.0, "seed {seed}: the water reaches in only to r{reach:.0}");
            assert!(reach > frame.water_inner_radius() - crate::models::hydrology::CELL, "seed {seed}: water at r{reach:.0}");
            let hard = metrics::violations_against(m, metrics::V2_TARGETS).into_iter().filter(|v| v.hard);
            failed.extend(hard.map(|v| format!("seed {seed}: {} {} (target {})", v.label, v.value, v.target)));
        }
        assert!(failed.is_empty(), "hard misses: {failed:?}");
    }

    #[test]
    fn the_lanes_stay_clear_of_the_wild_trails_weld() {
        for seed in SEEDS {
            let t = TerrainConfig::from_params(&farms_world(seed));
            let c = center(&t);
            let far = t
                .roads
                .iter()
                .filter(|r| is_lane(r))
                .flat_map(|r| r.points.iter())
                .map(|p| (p.0 - c.0).stable_hypot(p.1 - c.1))
                .fold(0.0, f64::max);
            assert!(far < capitol_protocol::rules::RURAL_OUTER_RADIUS - 20.0, "seed {seed}: a lane reaches r{far:.0}");
        }
    }
}
