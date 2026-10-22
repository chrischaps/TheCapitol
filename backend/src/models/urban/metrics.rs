//! The urban grid's numbers, against the plots plan's §3 targets (#421).

use std::collections::HashMap;
use std::f64::consts::PI;

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot};
use serde::Serialize;

use crate::models::station_room;
use crate::models::street_plan;
use crate::models::terrain::{point_to_segment_distance, TerrainConfig, WorldGenParams};

type P = (f64, f64);

#[derive(Debug, Clone, Default, Serialize)]
pub struct Metrics {
    /// Urban lots in the finished world.
    pub lots: usize,
    /// Lots `finish_lots` cut back from the roads or the water, and dropped.
    pub trimmed: usize,
    pub dropped: usize,
    pub area_median: f64,
    pub area_p5: f64,
    pub area_p95: f64,
    /// Along the street edge, and from its middle to the back edge's.
    pub width_median: f64,
    pub width_min: f64,
    pub depth_median: f64,
    /// The share of lots that take 4 stations at 50 spacing.
    pub takes_four_share: f64,
    /// The lots' share of the ring from the canal's water to r1800.
    pub cover: f64,
    /// The grid's streets: their audit's findings, all told, and any
    /// crossing with an arterial off a vertex of the ring street's.
    pub audit_issues: usize,
    pub streets: usize,
}

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).stable_hypot(a.1 - b.1)
}

fn mid(a: P, b: P) -> P {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

/// The world `params` makes, and its urban numbers.
pub fn measure(params: &WorldGenParams) -> (TerrainConfig, Metrics) {
    let terrain = TerrainConfig::from_params(params);
    let laid: HashMap<String, TerrainPlot> = super::grid(params, (terrain.center_x, terrain.center_y), &terrain.roads)
        .lots(&params.urban)
        .into_iter()
        .map(|l| (l.id.clone(), l))
        .collect();
    let lots: Vec<&TerrainPlot> = terrain.terrain_plots.iter().filter(|l| l.zone == "urban").collect();
    let trimmed = lots.iter().filter(|l| laid.get(&l.id).is_none_or(|g| g.corners != l.corners)).count();
    let mut areas: Vec<f64> = lots.iter().map(|l| l.area).collect();
    let mut widths: Vec<f64> = lots.iter().map(|l| dist(l.corners[0], l.corners[1])).collect();
    let mut depths: Vec<f64> =
        lots.iter().map(|l| dist(mid(l.corners[0], l.corners[1]), mid(l.corners[2], l.corners[3]))).collect();
    for v in [&mut areas, &mut widths, &mut depths] {
        v.sort_by(f64::total_cmp);
    }
    let fours = lots.iter().filter(|l| station_room::takes(&l.corners, 4)).count();
    let inner = params.guild_outer_radius + params.canal_outer_width / 2.0;
    let band = PI * (params.urban_outer_radius.powi(2) - inner.powi(2));
    // The grid's streets among themselves, and each crossing of a ring
    // street and an arterial at a vertex of the ring's on its centre line.
    let grid: Vec<RoadSegment> = terrain.roads.iter().filter(|r| super::is_grid_street(r)).cloned().collect();
    let audit = street_plan::audit(&grid, |_| true);
    let arterials: Vec<&RoadSegment> =
        terrain.roads.iter().filter(|r| r.road_type == RoadType::Arterial && r.id.starts_with("arterial_")).collect();
    let off_vertex = grid
        .iter()
        .filter(|r| r.id.starts_with("urb_ring_"))
        .flat_map(|ring| arterials.iter().map(move |a| (ring, a)))
        .filter(|(ring, a)| {
            let (p, q) = (a.points[0], a.points[a.points.len() - 1]);
            let on = |v: &P| point_to_segment_distance(v.0, v.1, p.0, p.1, q.0, q.1) < 1e-6;
            let crosses = ring.points.windows(2).any(|w| {
                let side = |v: P| (q.0 - p.0) * (v.1 - p.1) - (q.1 - p.1) * (v.0 - p.0);
                side(w[0]) * side(w[1]) < 0.0
                    && point_to_segment_distance(w[0].0, w[0].1, p.0, p.1, q.0, q.1) < 50.0
            });
            crosses && !ring.points.iter().any(on)
        })
        .count();
    let m = Metrics {
        lots: lots.len(),
        trimmed,
        dropped: laid.len().saturating_sub(lots.len()),
        area_median: quantile(&areas, 0.5),
        area_p5: quantile(&areas, 0.05),
        area_p95: quantile(&areas, 0.95),
        width_median: quantile(&widths, 0.5),
        width_min: widths.first().copied().unwrap_or(f64::NAN),
        depth_median: quantile(&depths, 0.5),
        takes_four_share: fours as f64 / lots.len().max(1) as f64,
        cover: areas.iter().sum::<f64>() / band,
        audit_issues: audit.short_segments.len()
            + audit.sharp_turns.len()
            + audit.self_crossings.len()
            + audit.unjoined_crossings.len()
            + audit.loose_ends.len()
            + off_vertex,
        streets: terrain.roads.iter().filter(|r| super::is_grid_street(r)).count(),
    };
    (terrain, m)
}
