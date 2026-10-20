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
mod render;

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
mod tests {
    use std::f64::consts::FRAC_PI_4;

    use capitol_protocol::fmath::StableMath;

    use capitol_protocol::terrain::RoadType;

    use super::metrics::{self, Metrics};
    use super::neighbourhood::{ENTRANCE_OFFSET, ENTRANCE_SPACING};
    use crate::models::terrain::{point_to_segment_distance, TerrainConfig, WorldGenParams};

    /// The suburb as it ships: every pattern, mixed by the default weights.
    fn mixed_world(seed: u64) -> WorldGenParams {
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

    /// Acceptance for #111: the mixed suburb, at five seeds and the wide
    /// canals. It asserts the hard targets, and those the patterns already
    /// hold one by one, and prints the soft ones it misses.
    #[test]
    fn neighbourhood_suburb_meets_its_targets() {
        for (name, params) in worlds() {
            let world = metrics::measure_world(&params, None);
            let m: Metrics = world.measured.metrics;
            let why = format!("{name}: {m:#?}");
            // Entrances, counted per neighbourhood by the street they come
            // from (#115): as many as each one allocated.
            let allocated: Vec<usize> = world.neighbourhoods.iter().map(|n| n.entrances.len()).collect();
            assert_eq!(m.entrances, allocated, "{why}");
            assert!(m.entrances_min.unwrap() >= 2.0 && m.entrances_max.unwrap() <= 4.0, "{why}");
            // Streets.
            assert_eq!(m.audit_issues, 0, "{why}");
            assert_eq!(m.four_ways, 0, "{why}");
            assert!(m.min_junction_angle.unwrap() >= 70.0, "{why}");
            assert!(m.t_spacing_min.unwrap() >= 40.0, "{why}");
            assert!(m.culdesac_max_len.unwrap() <= 300.0, "{why}");
            assert!(m.culdesac_max_lots.unwrap() <= 20.0, "{why}");
            assert_eq!(m.connected_share, Some(1.0), "{why}");
            // Lots.
            assert_eq!(m.overlaps, 0, "{why}");
            assert_eq!(m.convex_share, Some(1.0), "{why}");
            assert_eq!(m.arterial_fronting, 0, "{why}");
            assert!((6600.0..=7500.0).contains(&m.lot_area_median.unwrap()), "{why}");
            assert!(m.lot_area_p5.unwrap() >= 4000.0 && m.lot_area_p95.unwrap() <= 11000.0, "{why}");
            assert!((55.0..=65.0).contains(&m.lot_width_median.unwrap()), "{why}");
            assert!((100.0..=125.0).contains(&m.lot_depth_median.unwrap()), "{why}");
            assert!(m.trimmed_share.unwrap() <= 0.10, "{why}");
            assert!(m.dropped_share.unwrap() <= 0.03, "{why}");
            // Ground: with the parks, the gap is what's too thin to use.
            assert!(m.cover_lots.unwrap() >= 0.55, "{why}");
            assert!((0.08..=0.20).contains(&m.cover_green.unwrap()), "{why}");
            assert!(m.cover_gap.unwrap() <= 0.05, "{why}");
            assert!(m.parks > 0 && m.park_area_min.is_none_or(|a| a >= 0.5 * 64.0 * 110.0 - 1e-6), "{why}");

            let soft: Vec<String> = metrics::violations(&m).iter().map(|v| format!("{} {}", v.label, metrics::fmt_num(v.value))).collect();
            eprintln!("{name}: {} lots, {} parks, gap {:.3}; soft misses: {soft:?}", m.lots, m.parks, m.cover_gap.unwrap());
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

    /// Entrances are coordinated across each arterial (plan §4.1.5): as laid,
    /// not just as allocated. Where suburban streets from the two sides
    /// meet an arterial they're at least `ENTRANCE_OFFSET` apart along it,
    /// so there's no 4-way, and one side's are `ENTRANCE_SPACING` apart.
    #[test]
    fn entrances_line_up_across_the_arterials() {
        for (name, params) in worlds() {
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
