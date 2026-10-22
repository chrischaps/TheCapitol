//! The suburb, rebuilt as post-war neighbourhoods (`the-capitol-suburb-v2.md`).
//!
//! - `neighbourhood`: the sectors, their seeded splits, buffers and entrances (#106);
//! - `frame`: each neighbourhood's `F(u, v) → world` and its row fit (#106);
//! - `patterns`: the streets each neighbourhood is filled with, and their
//!   `StreetMeta` (pattern (a), #107);
//! - `loops`: pattern (b), loops and lollipops (#109);
//! - `central`: pattern (c), central loop (#110);
//! - `lots`: the lots that fill the blocks (#108);
//! - `parks`: the open ground the lots leave, where it's big enough to use (#111);
//! - `graph`: the street network as a graph;
//! - `metrics`: the layout numbers and the targets they're held to;
//! - `render`: the plan-view SVG and the `suburb_plan_report` test.
//!
//! Until something outside the tests uses `graph` and `metrics`, they're
//! test-only.

use capitol_protocol::terrain::RoadSegment;

use super::terrain::SuburbanGenParams;

// The server build doesn't yet use everything the patterns are given.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod central;
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod frame;
#[cfg(test)]
pub(crate) mod graph;
pub(crate) mod lots;
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod loops;
#[cfg(test)]
pub(crate) mod metrics;
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod neighbourhood;
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod parks;
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod patterns;
#[cfg(test)]
pub(crate) mod render;

use neighbourhood::Neighbourhood;
use patterns::StreetMeta;

/// The Neighbourhood layout of the suburb.
pub(crate) struct SuburbPlan {
    pub roads: Vec<RoadSegment>,
    /// One for each of `roads`, in the same order.
    pub meta: Vec<StreetMeta>,
    pub neighbourhoods: Vec<Neighbourhood>,
}

/// Lay out the suburb round `center`: the neighbourhoods, and each one's
/// streets in the pattern its own stream picks.
pub(crate) fn plan(params: &SuburbanGenParams, center: (f64, f64)) -> SuburbPlan {
    let mut neighbourhoods = neighbourhood::neighbourhoods(params, center);
    let (mut roads, mut meta) = (Vec::new(), Vec::new());
    for n in &mut neighbourhoods {
        let laid = patterns::lay(n, &params.neighbourhood);
        roads.extend(laid.roads);
        meta.extend(laid.meta);
    }
    SuburbPlan { roads, meta, neighbourhoods }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::f64::consts::FRAC_PI_4;

    use capitol_protocol::fmath::StableMath;

    use capitol_protocol::terrain::RoadType;

    use super::metrics;
    use super::neighbourhood::{ENTRANCE_OFFSET, ENTRANCE_SPACING};
    use crate::models::terrain::{point_to_segment_distance, TerrainConfig, WorldGenParams};

    /// The suburb as it ships: every pattern, mixed by the default weights.
    /// Plots v2's since the flip (#459): r2160 to r3600, lots of about 140
    /// × 134 (#456, #468).
    pub(crate) fn mixed_world(seed: u64) -> WorldGenParams {
        WorldGenParams { seed, ..Default::default() }
    }

    /// The wider canals the dev DB commits (#83).
    fn wide_canal_world(seed: u64) -> WorldGenParams {
        let mut params = mixed_world(seed);
        params.canal_inner_width = 50.0;
        params.canal_outer_width = 50.0;
        params
    }

    fn worlds() -> Vec<(String, WorldGenParams)> {
        let mut all: Vec<_> = [42, 7, 1234, 99999, 2024].into_iter().map(|s| (format!("seed {s}"), mixed_world(s))).collect();
        all.push(("wide canals, seed 42".into(), wide_canal_world(42)));
        all
    }

    /// Acceptance for #111 and #456 (Plots v2's suburb, V1.2; the default
    /// since the flip, #459): the mixed suburb at r2160 to r3600, at five
    /// seeds and the wide canals. About 1000 lots of about 18.5k, 90% of
    /// them taking 6 stations and the median 8 (#468). Every world is
    /// printed before any is asserted, so a miss shows them all.
    #[test]
    fn neighbourhood_suburb_meets_its_targets() {
        let measured: Vec<_> = worlds().into_iter().map(|(name, params)| (name, metrics::measure_world(&params, None))).collect();
        for (seed, world) in &measured {
            let m = &world.measured.metrics;
            let pct = |v: Option<f64>| 100.0 * v.unwrap_or(f64::NAN);
            let soft: Vec<String> = metrics::violations(m).iter().map(|v| format!("{} {}", v.label, metrics::fmt_num(v.value))).collect();
            eprintln!(
                "{seed}: {} lots, area {:.0}, {:.0} × {:.0}, {:.1}% take 6, {:.0}% take 8, stations median {:.0} mean {:.2}, lots {:.0}%, green {:.0}%, gap {:.1}%",
                m.lots,
                m.lot_area_median.unwrap_or(f64::NAN),
                m.lot_width_median.unwrap_or(f64::NAN),
                m.lot_depth_median.unwrap_or(f64::NAN),
                pct(m.takes_six_share),
                pct(m.takes_eight_share),
                m.stations_median.unwrap_or(f64::NAN),
                m.stations_mean.unwrap_or(f64::NAN),
                pct(m.cover_lots),
                pct(m.cover_green),
                pct(m.cover_gap),
            );
            eprintln!("{seed}: soft misses: {soft:?}");
        }
        for (seed, world) in measured {
            let m = world.measured.metrics;
            let why = format!("{seed}: {m:#?}");
            let allocated: Vec<usize> = world.neighbourhoods.iter().map(|n| n.entrances.len()).collect();
            assert_eq!(m.entrances, allocated, "{why}");
            assert!(m.entrances_min.unwrap() >= 2.0 && m.entrances_max.unwrap() <= 4.0, "{why}");
            metrics::Lots::assert_streets(&m, &why);
            assert!(m.culdesac_max_lots.unwrap() <= 20.0, "{why}");
            // About 1000 (plots plan v2 §3).
            assert!((900..=1100).contains(&m.lots), "{why}");
            // 90% take 6 stations, a whole workshop (#468): 92 to 93%.
            metrics::Lots { takes_six: 0.90, area_max: 19000.0, depth_min: 120.0, green_max: 0.25 }.assert_lots(&m, &why);
            assert!(m.stations_median.unwrap() >= 8.0, "{why}");
            let least = 0.5 * 140.0 * 134.0;
            assert!(m.parks > 0 && m.park_area_min.is_none_or(|a| a >= least - 1e-6), "{why}");
        }
    }

    /// A split sector's entrances are counted in each of its two
    /// neighbourhoods, not lumped together, and an entrance on the sector's
    /// edge counts though it sits on the arterial's centre line (#115).
    #[test]
    fn a_split_sector_counts_its_neighbourhoods_entrances_apart() {
        let params = mixed_world(42);
        let all = metrics::measure_world(&params, None);
        let split = (0..8u8)
            .find(|&s| all.neighbourhoods.iter().filter(|n| n.sector == s).count() == 2)
            .expect("seed 42 splits a sector");
        let one = metrics::measure_world(&params, Some(split));
        let allocated: Vec<usize> = one.neighbourhoods.iter().filter(|n| n.sector == split).map(|n| n.entrances.len()).collect();
        assert_eq!(allocated.len(), 2);
        assert_eq!(one.measured.metrics.entrances, allocated);
    }

    /// Entrances are coordinated across each arterial (plan §4.1.5; in Plots
    /// v2's suburb since #456): as laid,
    /// not just as allocated. Where suburban streets from the two sides
    /// meet an arterial they're at least `ENTRANCE_OFFSET` apart along it,
    /// so there's no 4-way, and one side's are `ENTRANCE_SPACING` apart.
    #[test]
    fn entrances_line_up_across_the_arterials() {
        entrances_line_up(worlds());
    }

    fn entrances_line_up(worlds: Vec<(String, WorldGenParams)>) {
        for (name, params) in worlds {
            let t = TerrainConfig::from_params(&params);
            let c = (t.center_x, t.center_y);
            let arterials: Vec<_> = t.roads.iter().filter(|r| r.road_type == RoadType::Arterial).collect();
            let on_arterial = |p: (f64, f64)| {
                arterials.iter().any(|a| {
                    a.points.windows(2).any(|w| point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1) < 1e-6)
                })
            };
            // (arterial, side, distance from the centre) for each entrance.
            let mut found: Vec<(u8, bool, f64)> = Vec::new();
            for r in t.roads.iter().filter(|r| r.road_type == RoadType::Suburban && r.points.len() >= 2) {
                let n = r.points.len();
                for (end, next) in [(r.points[0], r.points[1]), (r.points[n - 1], r.points[n - 2])] {
                    if !on_arterial(end) {
                        continue;
                    }
                    let rel = (end.0 - c.0, end.1 - c.1);
                    let angle = rel.1.stable_atan2(rel.0).rem_euclid(std::f64::consts::TAU);
                    let arterial = ((angle / FRAC_PI_4).round() as u8) % 8;
                    let side = rel.0 * (next.1 - end.1) - rel.1 * (next.0 - end.0) > 0.0;
                    found.push((arterial, side, rel.0.stable_hypot(rel.1)));
                }
            }
            assert!(found.len() >= 16 * 2, "{name}: only {} entrances", found.len());
            for (i, a) in found.iter().enumerate() {
                for b in &found[i + 1..] {
                    if a.0 != b.0 {
                        continue;
                    }
                    let apart = (a.2 - b.2).abs();
                    let least = if a.1 == b.1 { ENTRANCE_SPACING } else { ENTRANCE_OFFSET };
                    assert!(
                        apart >= least - 1e-6,
                        "{name}: arterial {} entrances {apart:.1} apart ({} side)",
                        a.0,
                        if a.1 == b.1 { "same" } else { "opposite" }
                    );
                }
            }
        }
    }
}
