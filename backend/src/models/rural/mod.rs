//! The rural ring, r3000 to r4600 (`the-capitol-rural.md`, RD10).
//!
//! Today's layout is still `terrain.rs`'s radial trails. Rural v2 builds the
//! farms here, a work package at a time, behind [`RuralLayout::Farms`]. So far:
//! - `sectors`: the sectors between the arterials and the deck of their
//!   characters (RD1, RD2, #286);
//! - `roads`: each sector's two concession roads or winding lanes, the roads
//!   between them, and the roads to farm gates (RD3 to RD5, #286; RD10, #310);
//! - `metrics`: the ring's layout numbers and the plan's §5 targets (#284);
//! - `render`: the plan-view SVG and the `rural_plan_report` test (#284).
//!
//! The last two are test-only, as the suburb's are.

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

/// Which rural ring the world builds. `Trails` until the flip (R7, #290), so
/// the pin holds while the farms are built; R8 (#291) removes the switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuralLayout {
    /// The 12 radial trails and their lots (`terrain.rs`).
    #[default]
    Trails,
    /// Rural v2: two concession roads or winding lanes across every sector,
    /// and the roads between them (#286, #310). No farms on them until
    /// R4 (#287).
    Farms,
}

/// The rural ring's params. Every field has a default, so a saved
/// `world_config` from before them still loads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuralParams {
    pub layout: RuralLayout,
    /// The lanes, side roads and spurs (RD5).
    pub lane_width: f64,
}

impl Default for RuralParams {
    fn default() -> Self {
        Self { layout: RuralLayout::Trails, lane_width: 8.0 }
    }
}

/// The ring's roads under `Farms`, before the weld.
#[derive(Debug, Default)]
pub struct Plan {
    pub sectors: Vec<Sector>,
    /// `lane_{s}_{k}`: each sector's inner main road is `k = 0` and its outer
    /// one `k = 1`.
    pub roads: Vec<RoadSegment>,
    /// Where the side roads and spurs end. A loose end is allowed only here.
    pub gates: Vec<P>,
}

/// Plans the ring's roads. `roads` needs only the arterials, which cut the
/// ring into its sectors.
pub fn plan(params: &WorldGenParams, center: P, roads: &[RoadSegment]) -> Plan {
    let bearings = sectors::arterial_bearings(roads, center, params.suburban_outer_radius);
    let sectors = sectors::sectors(params.seed, &bearings);
    let ring = roads::Ring {
        center,
        inner: params.suburban_outer_radius,
        outer: params.rural_outer_radius,
        arterial_width: params.arterial_width,
        lane_width: params.rural.lane_width,
    };
    let mut plan = Plan::default();
    for sector in &sectors {
        let mut rng = SplitMix64::stream(params.seed, &[RURAL, sector.index as u64]);
        let laid = roads::lay(sector, &ring, &mut rng);
        plan.roads.extend(laid.roads);
        plan.gates.extend(laid.gates);
    }
    plan.sectors = sectors;
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
        let mut params = WorldGenParams { seed, ..Default::default() };
        params.rural.layout = RuralLayout::Farms;
        params
    }

    fn center(t: &TerrainConfig) -> P {
        (t.center_x, t.center_y)
    }

    fn same(a: P, b: P) -> bool {
        (a.0 - b.0).stable_hypot(a.1 - b.1) < 1e-6
    }

    #[test]
    fn a_world_config_without_rural_params_loads_as_the_trails() {
        let mut json = serde_json::to_value(WorldGenParams::default()).unwrap();
        json.as_object_mut().unwrap().remove("rural");
        let params: WorldGenParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.rural.layout, RuralLayout::Trails);
        let farms: WorldGenParams = serde_json::from_value(serde_json::json!({ "rural": { "layout": "farms" } })).unwrap();
        assert_eq!(farms.rural.layout, RuralLayout::Farms);
        assert_eq!(farms.rural.lane_width, 8.0);
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
    /// main road, and ends on the other's or at a gate.
    #[test]
    fn the_lanes_are_clean_and_join_the_arterials() {
        for seed in SEEDS {
            let params = farms_world(seed);
            let t = TerrainConfig::from_params(&params);
            assert!(t.roads.iter().all(|r| !r.id.starts_with("trail_")), "seed {seed}: the old trails are still there");
            let gates = plan(&params, center(&t), &t.roads).gates;
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
                    assert!(from.is_some(), "seed {seed}: {} doesn't start on a main road", lane.id);
                    let joins = from.is_some_and(|i| on(mains[1 - i], end));
                    assert!(joins || gates.iter().any(|&g| same(g, end)), "seed {seed}: {} ends on neither road nor gate", lane.id);
                }
                let counted = metrics::sector_roads(&t.roads, s);
                assert!(counted.mains == 2 && counted.links >= 1, "seed {seed}: sector {s} has {counted:?}");
            }
        }
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
