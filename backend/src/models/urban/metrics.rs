//! The urban grid's numbers, against the plots plan's §3 targets (#421)
//! and Plots v2's (#455).

use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot};
use serde::Serialize;

use super::UrbanParams;
use crate::models::lot_clearance::finish_lots;
use crate::models::station_room;
use crate::models::street_plan;
use crate::models::terrain::{point_to_segment_distance, TerrainConfig, WorldGenParams};

type P = (f64, f64);

#[derive(Debug, Clone, Default, Serialize)]
pub struct Metrics {
    /// Urban lots in the finished world.
    pub lots: usize,
    /// The outer ring street's radius, from the centre.
    pub outer_street: f64,
    /// The longest block, along its outer ring street from kerb to kerb.
    pub longest_block: f64,
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
    /// Lots that take 5: an urban lot holds some stations, never a whole
    /// workshop (Plots v2 §1), so none should.
    pub takes_five: usize,
    /// The lots' share of the ring from the canal's water to the urban edge.
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

/// Plots v2's world (#455): the urban edge at r2160 and five rings of blocks,
/// with the suburb and the rural ring moved out to match. A stand-in for
/// #454's `WorldGenParams::grown()`. The land and the water don't follow these
/// radii until #454, and the urban ring doesn't need them, so it's measured
/// with `measure_grid`, not as a whole world.
pub fn v2(seed: u64) -> WorldGenParams {
    WorldGenParams {
        seed,
        urban_outer_radius: 2160.0,
        suburban_outer_radius: 3600.0,
        rural_outer_radius: 6100.0,
        // 32 bearings round the outer ring of blocks, past r1800, so no
        // block is much over 500 long: at 24 they'd be 540 out there.
        urban: UrbanParams { rings: 5, outer_spokes: 32, ..UrbanParams::default() },
        ..WorldGenParams::default()
    }
}

/// The world `params` makes, and its urban numbers.
pub fn measure(params: &WorldGenParams) -> (TerrainConfig, Metrics) {
    let terrain = TerrainConfig::from_params(params);
    let m = numbers(params, &terrain);
    (terrain, m)
}

/// The urban ring alone, and its numbers: the grid on the city's ring roads
/// and arterials, with the suburb's streets beyond it, its lots put through
/// `finish_lots` against those roads and the water rings. No land, water
/// courses, farms or other zones' lots, so it measures a ring whose radii the
/// rest of the world doesn't follow yet (#455).
pub fn measure_grid(params: &WorldGenParams) -> (TerrainConfig, Metrics) {
    let mut terrain = TerrainConfig::bare(params);
    let center = (terrain.center_x, terrain.center_y);
    let mut roads = terrain.generate_ring_roads_with_params(params);
    roads.extend(terrain.generate_arterial_roads_with_params(params));
    let grid = super::grid(params, center, &roads);
    roads.extend(grid.roads());
    roads.extend(crate::models::suburb::plan(&params.suburban_params(), center).roads);
    terrain.terrain_plots = finish_lots(grid.lots(&params.urban), &roads, &terrain.water_rings, center);
    terrain.roads = roads;
    let m = numbers(params, &terrain);
    (terrain, m)
}

/// The urban numbers of `terrain`, against the lots the grid lays on its roads.
fn numbers(params: &WorldGenParams, terrain: &TerrainConfig) -> Metrics {
    let layout = super::grid(params, (terrain.center_x, terrain.center_y), &terrain.roads);
    let laid: HashMap<String, TerrainPlot> =
        layout.lots(&params.urban).into_iter().map(|l| (l.id.clone(), l)).collect();
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
    let fives = lots.iter().filter(|l| station_room::takes(&l.corners, 5)).count();
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
    Metrics {
        lots: lots.len(),
        outer_street: layout.streets.last().copied().unwrap_or(f64::NAN),
        longest_block: (1..layout.streets.len())
            .flat_map(|k| {
                let (r, at) = (layout.streets[k], &layout.bearings(k - 1).at);
                (0..at.len()).map(move |b| {
                    let ((a1, h1), (a2, h2)) = (at[b], at[(b + 1) % at.len()]);
                    r * (a2 - a1).rem_euclid(TAU) - h1 - h2
                })
            })
            .fold(0.0, f64::max),
        trimmed,
        dropped: laid.len().saturating_sub(lots.len()),
        area_median: quantile(&areas, 0.5),
        area_p5: quantile(&areas, 0.05),
        area_p95: quantile(&areas, 0.95),
        width_median: quantile(&widths, 0.5),
        width_min: widths.first().copied().unwrap_or(f64::NAN),
        depth_median: quantile(&depths, 0.5),
        takes_four_share: fours as f64 / lots.len().max(1) as f64,
        takes_five: fives,
        cover: areas.iter().sum::<f64>() / band,
        audit_issues: audit.short_segments.len()
            + audit.sharp_turns.len()
            + audit.self_crossings.len()
            + audit.unjoined_crossings.len()
            + audit.loose_ends.len()
            + off_vertex,
        streets: terrain.roads.iter().filter(|r| super::is_grid_street(r)).count(),
    }
}
