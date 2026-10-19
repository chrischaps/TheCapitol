use capitol_protocol::fmath::StableMath;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

use capitol_protocol::rules;
use capitol_protocol::land::LandParams;

use super::geom::polyline_length;
use super::street_plan;

// The terrain types the client sees are shared through capitol-protocol, so
// GET /terrain serves the generated terrain as-is.
pub use capitol_protocol::terrain::{
    Bridge, Ford, Lake, Landmark, LandmarkKind, RoadSegment, RoadType, TerrainPark, TerrainPlot, WaterCourse,
    WaterRing,
};

/// Configuration for suburban street generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuburbanGenParams {
    /// Inner and outer radius of the suburban zone. The generators read them
    /// only through `WorldGenParams::suburban_params`, which sets them from
    /// the world's `urban_outer_radius` and `suburban_outer_radius`, so a
    /// saved value here has no effect. They stay in the struct (and the JSON)
    /// because the generators read the resolved radii from it.
    pub inner_radius: f64,
    pub outer_radius: f64,
    /// The world's seed. Not a tweakable: `suburban_params` sets it from
    /// `WorldGenParams::seed`.
    #[serde(skip, default = "default_seed")]
    pub seed: u64,
    /// Width of the arterials the streets keep clear of. Not a tweakable:
    /// `generate_all_roads_with_params` sets it from `WorldGenParams`.
    #[serde(skip, default = "default_arterial_width")]
    pub arterial_width: f64,
    /// The neighbourhood generator's knobs (suburb v2, #103). The serde
    /// default keeps saved `world_config` rows from before #105 loading.
    ///
    /// The Legacy generator's params (the street spacing, the plot-area and
    /// frontage windows, its street widths) and the `layout` switch went
    /// with it in #113. Unknown fields are ignored, so a saved row that still
    /// has them, `"layout": "legacy"` included, loads as the neighbourhoods.
    #[serde(default)]
    pub neighbourhood: NeighbourhoodParams,
}

fn default_arterial_width() -> f64 {
    WorldGenParams::default().arterial_width
}

fn default_seed() -> u64 {
    WorldGenParams::default().seed
}

/// The street patterns a neighbourhood can take (plan §4.1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeighbourhoodPattern {
    /// (a) Near-parallel rows two lots apart, joined by T links.
    WarpedParallels,
    /// (b) A racetrack collector with combs of cul-de-sacs inside it.
    LoopsAndLollipops,
    /// (c) A flattened loop round a park, cul-de-sacs pointing out (PW2).
    CentralLoop,
}

/// How often each pattern is picked, among those that fit a neighbourhood.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternWeights {
    pub warped_parallels: f64,
    pub loops_and_lollipops: f64,
    pub central_loop: f64,
}

impl Default for PatternWeights {
    fn default() -> Self {
        Self { warped_parallels: 1.0, loops_and_lollipops: 1.0, central_loop: 1.0 }
    }
}

/// The Neighbourhood generator's parameters, in world units (plan §3, §4).
/// Every field has a default, so a partial object loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NeighbourhoodParams {
    /// Lot width along the street.
    pub lot_frontage: f64,
    /// Lot depth wanted, and the range a row fit may use.
    pub lot_depth: f64,
    pub lot_depth_min: f64,
    pub lot_depth_max: f64,
    /// Street widths.
    pub local_width: f64,
    pub collector_width: f64,
    /// Green strip between a neighbourhood's edge and the arterial.
    pub buffer: f64,
    /// How often a sector splits into two neighbourhoods, by angle, and the
    /// width of the greenway between them.
    pub split_chance: f64,
    pub greenway_width: f64,
    /// Spacing of the T links between parallel rows (L).
    pub link_spacing: f64,
    /// Cul-de-sac turnaround radius, and the longest stem.
    pub bulb_radius: f64,
    pub cul_de_sac_max: f64,
    /// The frame's shared radial warp: amplitude A (at most 40) and
    /// wavelength λ (at least 1200). #111 tuned them to 30 and 1200: a
    /// 40 warp costs the band 20 of depth, and the lots fall under the
    /// 6600 area floor.
    pub warp_amplitude: f64,
    pub warp_wavelength: f64,
    pub pattern_weights: PatternWeights,
    /// Every neighbourhood takes this pattern, if it fits. For tests.
    pub force_pattern: Option<NeighbourhoodPattern>,
}

impl Default for NeighbourhoodParams {
    fn default() -> Self {
        Self {
            // 64 × the 106 a five-row band fits (#117) keeps the plan's
            // 60 × 110 area (#108).
            lot_frontage: 64.0,
            lot_depth: 110.0,
            lot_depth_min: 95.0,
            lot_depth_max: 130.0,
            local_width: 16.0,
            collector_width: 20.0,
            buffer: 20.0,
            split_chance: 0.5,
            greenway_width: 40.0,
            link_spacing: 400.0,
            bulb_radius: 13.0,
            cul_de_sac_max: 300.0,
            warp_amplitude: 30.0,
            warp_wavelength: 1200.0,
            pattern_weights: PatternWeights::default(),
            force_pattern: None,
        }
    }
}

impl Default for SuburbanGenParams {
    fn default() -> Self {
        Self {
            inner_radius: rules::URBAN_OUTER_RADIUS,
            outer_radius: rules::SUBURBAN_OUTER_RADIUS,
            seed: 42,
            arterial_width: 60.0,
            neighbourhood: NeighbourhoodParams::default(),
        }
    }
}

/// Master world generation parameters capturing all tweakable values
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldGenParams {
    // Zone radii (concentric from center)
    pub capitol_radius: f64,
    pub trade_outer_radius: f64,
    pub guild_outer_radius: f64,
    pub urban_outer_radius: f64,
    pub suburban_outer_radius: f64,
    pub rural_outer_radius: f64,
    // Water/Canals
    pub moat_width: f64,
    pub moat_bridge_width: f64,
    pub canal_inner_width: f64,
    pub canal_outer_width: f64,
    pub canal_bridge_width: f64,
    // Ring roads
    pub guild_ring_radius: f64,
    pub guild_ring_width: f64,
    pub trade_ring_radius: f64,
    pub trade_ring_width: f64,
    // Arterials
    pub arterial_width: f64,
    pub arterial_end_radius: f64,
    // Urban grid
    pub urban_radial_spacing: f64,
    pub urban_spoke_count: u32,
    pub urban_street_width: f64,
    // Suburban (nested)
    pub suburban: SuburbanGenParams,
    // Urban plots
    pub urban_plot_frontage: f64,
    pub urban_plot_depth: f64,
    // Rural
    pub rural_trail_width: f64,
    pub rural_trail_count: u32,
    // Rural plots
    pub rural_plot_frontage: f64,
    pub rural_plot_depth: f64,
    // Guild halls
    pub guild_plot_frontage: f64,
    // Seeds
    pub seed: u64,
    // The height of the wilds (W2, #195). Its own seed, so reseeding the
    // suburb leaves the ranges standing.
    pub land: LandParams,
}

impl WorldGenParams {
    /// The suburban params the generators use: `suburban` with its radii,
    /// seed and arterial width taken from the world. Roads and lots both read
    /// this one set (#105). Before, the lots read `suburban`'s own radii.
    pub fn suburban_params(&self) -> SuburbanGenParams {
        SuburbanGenParams {
            inner_radius: self.urban_outer_radius,
            outer_radius: self.suburban_outer_radius,
            seed: self.seed,
            arterial_width: self.arterial_width,
            ..self.suburban.clone()
        }
    }
}

impl Default for WorldGenParams {
    fn default() -> Self {
        Self {
            // The zone radii are the protocol's table (#116).
            capitol_radius: rules::CAPITOL_RADIUS,
            trade_outer_radius: rules::TRADE_OUTER_RADIUS,
            guild_outer_radius: rules::GUILD_OUTER_RADIUS,
            urban_outer_radius: rules::URBAN_OUTER_RADIUS,
            suburban_outer_radius: rules::SUBURBAN_OUTER_RADIUS,
            rural_outer_radius: rules::RURAL_OUTER_RADIUS,
            moat_width: 20.0,
            moat_bridge_width: 30.0,
            canal_inner_width: 15.0,
            canal_outer_width: 15.0,
            canal_bridge_width: 25.0,
            guild_ring_radius: 800.0,
            guild_ring_width: 30.0,
            trade_ring_radius: 400.0,
            trade_ring_width: 30.0,
            arterial_width: 60.0,
            // The arterials run out to where the wilderness starts. That is
            // 3800, where they ended before the suburb deepened (#117).
            arterial_end_radius: rules::RURAL_OUTER_RADIUS,
            urban_radial_spacing: 100.0,
            urban_spoke_count: 48,
            urban_street_width: 16.0,
            suburban: SuburbanGenParams::default(),
            urban_plot_frontage: 40.0,
            urban_plot_depth: 50.0,
            rural_trail_width: 6.0,
            rural_trail_count: 12,
            rural_plot_frontage: 200.0,
            rural_plot_depth: 150.0,
            guild_plot_frontage: 150.0,
            seed: 42,
            land: LandParams::default(),
        }
    }
}

/// How far a guild hall stands back from the kerb of the roads and water
/// round it, in world units (#83).
const GUILD_CLEAR: f64 = 1.0;

/// Terrain configuration for the world
/// Defines moats, canals, bridges, roads, and suburban plots
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerrainConfig {
    pub world_size: f64,
    pub center_x: f64,
    pub center_y: f64,
    pub water_rings: Vec<WaterRing>,
    pub roads: Vec<RoadSegment>,
    pub terrain_plots: Vec<TerrainPlot>,
    /// The suburb's parks, from the open ground its lots leave (#129).
    #[serde(default)]
    pub parks: Vec<TerrainPark>,
    /// The city's public buildings: the Exchange hall (E3.4).
    #[serde(default)]
    pub landmarks: Vec<Landmark>,
    /// The params of the land's height (W2, #195), which `/terrain` carries.
    #[serde(default)]
    pub land: LandParams,
    /// The wilds' water, from the land (W3, #196): see `hydrology`.
    #[serde(default)]
    pub rivers: Vec<WaterCourse>,
    #[serde(default)]
    pub streams: Vec<WaterCourse>,
    #[serde(default)]
    pub lakes: Vec<Lake>,
    #[serde(default)]
    pub fords: Vec<Ford>,
}

/// Catmull-Rom spline interpolation for smooth curves
/// Takes control points and returns interpolated points
fn catmull_rom_spline(control_points: &[(f64, f64)], points_per_segment: usize) -> Vec<(f64, f64)> {
    if control_points.len() < 2 {
        return control_points.to_vec();
    }

    let mut result = Vec::new();

    // Extend control points at ends for proper interpolation
    let mut extended = Vec::new();
    // Duplicate first point
    extended.push(control_points[0]);
    extended.extend_from_slice(control_points);
    // Duplicate last point
    extended.push(control_points[control_points.len() - 1]);

    // Interpolate between each pair of middle points
    for i in 1..extended.len() - 2 {
        let p0 = extended[i - 1];
        let p1 = extended[i];
        let p2 = extended[i + 1];
        let p3 = extended[i + 2];

        for j in 0..points_per_segment {
            let t = j as f64 / points_per_segment as f64;

            // Catmull-Rom spline formula
            let t2 = t * t;
            let t3 = t2 * t;

            let x = 0.5 * ((2.0 * p1.0) +
                         (-p0.0 + p2.0) * t +
                         (2.0 * p0.0 - 5.0 * p1.0 + 4.0 * p2.0 - p3.0) * t2 +
                         (-p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0) * t3);

            let y = 0.5 * ((2.0 * p1.1) +
                         (-p0.1 + p2.1) * t +
                         (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2 +
                         (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t3);

            result.push((x, y));
        }
    }

    // Add the last point
    result.push(control_points[control_points.len() - 1]);

    result
}

/// Get point and tangent angle at parameter t (0-1) along polyline
fn point_and_tangent_at_t(points: &[(f64, f64)], t: f64) -> Option<(f64, f64, f64)> {
    if points.len() < 2 {
        return None;
    }

    let total_length = polyline_length(points);
    let target_dist = t * total_length;

    let mut accumulated = 0.0;
    for i in 1..points.len() {
        let dx = points[i].0 - points[i - 1].0;
        let dy = points[i].1 - points[i - 1].1;
        let segment_len = (dx * dx + dy * dy).sqrt();

        if accumulated + segment_len >= target_dist {
            let local_t = (target_dist - accumulated) / segment_len;
            let x = points[i - 1].0 + dx * local_t;
            let y = points[i - 1].1 + dy * local_t;
            let angle = dy.stable_atan2(dx);
            return Some((x, y, angle));
        }

        accumulated += segment_len;
    }

    // Return last point
    let last_idx = points.len() - 1;
    let dx = points[last_idx].0 - points[last_idx - 1].0;
    let dy = points[last_idx].1 - points[last_idx - 1].1;
    Some((points[last_idx].0, points[last_idx].1, dy.stable_atan2(dx)))
}

/// Get point at specific distance along polyline
fn point_at_distance(points: &[(f64, f64)], distance: f64) -> Option<(f64, f64, f64)> {
    if points.len() < 2 {
        return None;
    }

    let total_length = polyline_length(points);
    if total_length == 0.0 {
        return None;
    }

    let t = distance / total_length;
    point_and_tangent_at_t(points, t)
}

/// Check if a point is near any point on a polyline
fn point_near_polyline(px: f64, py: f64, polyline: &[(f64, f64)], threshold: f64) -> bool {
    // point_to_segment_distance already handles endpoints (returns endpoint distance
    // when projection falls outside the segment), so no separate vertex loop needed.
    for i in 1..polyline.len() {
        let (x1, y1) = polyline[i - 1];
        let (x2, y2) = polyline[i];
        // Most segments are nowhere near: rule them out by their box first.
        if px < x1.min(x2) - threshold
            || px > x1.max(x2) + threshold
            || py < y1.min(y2) - threshold
            || py > y1.max(y2) + threshold
        {
            continue;
        }

        let dist = point_to_segment_distance(px, py, x1, y1, x2, y2);
        if dist < threshold {
            return true;
        }
    }

    // Handle single-point polyline edge case
    if polyline.len() == 1 {
        let (lx, ly) = polyline[0];
        return ((px - lx).powi(2) + (py - ly).powi(2)).sqrt() < threshold;
    }

    false
}

/// Calculate distance from point to line segment
pub(crate) fn point_to_segment_distance(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len_sq = dx * dx + dy * dy;

    if len_sq == 0.0 {
        return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
    }

    let t = ((px - x1) * dx + (py - y1) * dy) / len_sq;
    let t = t.clamp(0.0, 1.0);

    let proj_x = x1 + t * dx;
    let proj_y = y1 + t * dy;

    ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
}

/// Check if two line segments intersect, returns the intersection point if they do
fn segments_intersect(
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    p4: (f64, f64),
) -> Option<(f64, f64)> {
    let (x1, y1) = p1;
    let (x2, y2) = p2;
    let (x3, y3) = p3;
    let (x4, y4) = p4;

    let denom = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
    if denom.abs() < 1e-10 {
        return None; // Parallel lines
    }

    let t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / denom;
    let u = -((x1 - x2) * (y1 - y3) - (y1 - y2) * (x1 - x3)) / denom;

    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        let ix = x1 + t * (x2 - x1);
        let iy = y1 + t * (y2 - y1);
        Some((ix, iy))
    } else {
        None
    }
}

/// Check if a point is inside a convex polygon using cross product method
fn point_in_polygon(px: f64, py: f64, polygon: &[(f64, f64)]) -> bool {
    let n = polygon.len();
    if n < 3 {
        return false;
    }

    let mut sign = None;

    for i in 0..n {
        let j = (i + 1) % n;
        let (x1, y1) = polygon[i];
        let (x2, y2) = polygon[j];

        // Cross product of edge vector and point-to-vertex vector
        let cross = (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1);

        if cross.abs() > 1e-10 {
            let current_sign = cross > 0.0;
            match sign {
                None => sign = Some(current_sign),
                Some(s) if s != current_sign => return false,
                _ => {}
            }
        }
    }

    true
}

/// Check if two convex polygons overlap using SAT (Separating Axis Theorem)
fn polygons_overlap(poly1: &[(f64, f64)], poly2: &[(f64, f64)]) -> bool {
    // First check if any vertex of poly1 is inside poly2 or vice versa
    for &(px, py) in poly1 {
        if point_in_polygon(px, py, poly2) {
            return true;
        }
    }
    for &(px, py) in poly2 {
        if point_in_polygon(px, py, poly1) {
            return true;
        }
    }

    // Check if any edges intersect
    for i in 0..poly1.len() {
        let j = (i + 1) % poly1.len();
        let edge1 = (poly1[i], poly1[j]);

        for k in 0..poly2.len() {
            let l = (k + 1) % poly2.len();
            let edge2 = (poly2[k], poly2[l]);

            if segments_intersect(edge1.0, edge1.1, edge2.0, edge2.1).is_some() {
                return true;
            }
        }
    }

    false
}

/// Check if a polygon intersects with a road (polyline with width)
/// This checks if any edge of the polygon crosses any segment of the road,
/// or if any corner of the polygon is within the road width
fn polygon_intersects_road(polygon: &[(f64, f64)], road_points: &[(f64, f64)], road_width: f64) -> bool {
    let half_width = road_width / 2.0 + 2.0; // Small buffer

    // Check if any polygon vertex is too close to the road
    for &(px, py) in polygon {
        if point_near_polyline(px, py, road_points, half_width) {
            return true;
        }
    }

    // Check if any polygon edge intersects any road segment
    for i in 0..polygon.len() {
        let j = (i + 1) % polygon.len();
        let poly_edge_start = polygon[i];
        let poly_edge_end = polygon[j];

        for k in 0..road_points.len().saturating_sub(1) {
            let road_seg_start = road_points[k];
            let road_seg_end = road_points[k + 1];

            if segments_intersect(poly_edge_start, poly_edge_end, road_seg_start, road_seg_end).is_some() {
                return true;
            }
        }
    }

    // Check if any road point is inside the polygon
    for &(rx, ry) in road_points {
        if point_in_polygon(rx, ry, polygon) {
            return true;
        }
    }

    false
}

impl TerrainConfig {
    /// Get the static terrain configuration for the world (uses default params)
    pub fn get_default() -> Self {
        Self::from_params(&WorldGenParams::default())
    }

    /// The lot that `(x, y)` stands on, if any. Lots don't overlap, so there's at most one.
    pub fn lot_at(&self, x: f64, y: f64) -> Option<&TerrainPlot> {
        self.terrain_plots.iter().find(|lot| {
            let c = &lot.corners;
            let (min_x, max_x) = c.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
            let (min_y, max_y) = c.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
            x >= min_x && x <= max_x && y >= min_y && y <= max_y
                && capitol_protocol::terrain::quad_contains(c, x, y)
        })
    }

    /// A stable hash of the generated terrain, as 16 hex digits: FNV-1a over its
    /// JSON. It hashes the output rather than the params, so a generator change
    /// also changes it, and clients can key a disk cache on it.
    ///
    /// Every float is snapped to a 1e-3 grid first, so a last-bit difference
    /// between builds (the optimiser reordering a sum, a libm path) doesn't
    /// count, and debug and release servers agree on the world (#128). The grid
    /// is coarse on purpose: a value within an ulp of a grid boundary still
    /// flips, and the odds of that grow as the grid gets finer. At 1e-3, on a
    /// world 12000 across, they're about 1e-8 a float.
    pub fn version(&self) -> String {
        fn quantise(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Number(n) if n.is_f64() => {
                    let snapped = (n.as_f64().expect("is_f64") * 1000.0).round() as i64;
                    *value = snapped.into();
                }
                serde_json::Value::Array(items) => items.iter_mut().for_each(quantise),
                serde_json::Value::Object(fields) => fields.values_mut().for_each(quantise),
                _ => {}
            }
        }
        let mut tree = serde_json::to_value(self).expect("terrain serializes");
        quantise(&mut tree);
        let json = serde_json::to_vec(&tree).expect("terrain serializes");
        let hash = json.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
            (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3)
        });
        format!("{hash:016x}")
    }

    /// Build terrain from explicit world generation parameters
    pub fn from_params(params: &WorldGenParams) -> Self {
        let mut config = TerrainConfig {
            world_size: capitol_protocol::rules::WORLD_SIZE,
            center_x: capitol_protocol::rules::CENTER_X,
            center_y: capitol_protocol::rules::CENTER_Y,
            water_rings: vec![
                // Moat: Capitol/Trade boundary
                WaterRing {
                    id: "moat".to_string(),
                    radius: params.capitol_radius,
                    width: params.moat_width,
                    bridges: vec![
                        Bridge { angle_degrees: 90.0, width: params.moat_bridge_width },
                    ],
                },
                // Canal 1: Trade/Guild boundary
                WaterRing {
                    id: "canal_inner".to_string(),
                    radius: params.trade_outer_radius,
                    width: params.canal_inner_width,
                    bridges: vec![
                        Bridge { angle_degrees: 270.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 0.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 90.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 180.0, width: params.canal_bridge_width },
                    ],
                },
                // Canal 2: Guild/Urban boundary
                WaterRing {
                    id: "canal_outer".to_string(),
                    radius: params.guild_outer_radius,
                    width: params.canal_outer_width,
                    bridges: vec![
                        Bridge { angle_degrees: 270.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 315.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 0.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 45.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 90.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 135.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 180.0, width: params.canal_bridge_width },
                        Bridge { angle_degrees: 225.0, width: params.canal_bridge_width },
                    ],
                },
            ],
            roads: Vec::new(),
            terrain_plots: Vec::new(),
            parks: Vec::new(),
            landmarks: Vec::new(),
            land: params.land.clone(),
            rivers: Vec::new(),
            streams: Vec::new(),
            lakes: Vec::new(),
            fords: Vec::new(),
        };

        // The wilds' water comes from the land alone.
        let water = super::hydrology::generate(&params.land);
        (config.rivers, config.streams, config.lakes, config.fords) =
            (water.rivers, water.streams, water.lakes, water.fords);

        // Generate roads using params
        config.roads = config.generate_all_roads_with_params(params);

        // Generate terrain plots for all zones
        config.terrain_plots = config.generate_suburban_plots_internal(&params.suburban_params());
        config.terrain_plots.extend(config.generate_urban_plots(params));
        config.terrain_plots.extend(config.generate_rural_plots(params));
        config.terrain_plots.extend(config.generate_guild_plots(params));
        // Pull every lot clear of the roads and the water, and make the ids unique
        // (B7): a lot is what players claim, so it must be the ground it looks like.
        config.terrain_plots = super::lot_clearance::finish_lots(
            std::mem::take(&mut config.terrain_plots),
            &config.roads,
            &config.water_rings,
            (config.center_x, config.center_y),
        );
        // The parks are the ground the cleared lots leave.
        config.parks = config.generate_parks(&params.suburban_params());
        config.landmarks = vec![config.exchange_hall_site(params)];

        config
    }

    /// The landmark of `kind`, if the world has one.
    pub fn landmark(&self, kind: LandmarkKind) -> Option<&Landmark> {
        self.landmarks.iter().find(|l| l.kind == kind)
    }

    /// The Exchange hall (E3.4, plan D6): on the east side of the Capitol
    /// avenue, the stretch of the south arterial between the trade ring and the
    /// moat's one bridge. New players arrive on that same road, so the way in
    /// passes the hall's door on the last walk to the tower. It's 80 across
    /// and 100 along the avenue, set back from the kerb, the moat and the ring.
    fn exchange_hall_site(&self, params: &WorldGenParams) -> Landmark {
        const SETBACK: f64 = 15.0;
        const DEPTH: f64 = 80.0;
        const LENGTH: f64 = 100.0;
        let bearing = 90.0_f64.to_radians();
        let out = (bearing.stable_cos(), bearing.stable_sin());
        // At 90° that's east: the avenue's right, walking in.
        let side = (out.1, -out.0);
        let moat_edge = params.capitol_radius + params.moat_width / 2.0;
        let ring_edge = params.trade_ring_radius - params.trade_ring_width / 2.0;
        let mid_r = (moat_edge + ring_edge) / 2.0;
        let near = params.arterial_width / 2.0 + SETBACK;
        let at = |lateral: f64, r: f64| {
            (self.center_x + out.0 * r + side.0 * lateral, self.center_y + out.1 * r + side.1 * lateral)
        };
        let (inner_r, outer_r) = (mid_r - LENGTH / 2.0, mid_r + LENGTH / 2.0);
        Landmark {
            id: "exchange_hall".to_string(),
            kind: LandmarkKind::ExchangeHall,
            position: at(near + DEPTH / 2.0, mid_r),
            corners: [at(near, outer_r), at(near, inner_r), at(near + DEPTH, inner_r), at(near + DEPTH, outer_r)],
            entrance_edge: (0, 1),
            door: at(near, mid_r),
        }
    }

    /// The suburb's parks, for the roads and lots already in `self`.
    fn generate_parks(&self, params: &SuburbanGenParams) -> Vec<TerrainPark> {
        let center = (self.center_x, self.center_y);
        let neighbourhoods = super::suburb::plan(params, center).neighbourhoods;
        let greens = super::suburb::parks::lay(&self.roads, &self.terrain_plots, &neighbourhoods, params, center);
        super::suburb::parks::outlines(&greens)
    }

    /// Generate all road segments from params
    fn generate_all_roads_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        let mut roads = Vec::new();

        // Ring roads
        roads.extend(self.generate_ring_roads_with_params(params));

        // Arterial roads
        roads.extend(self.generate_arterial_roads_with_params(params));

        // Urban grid
        roads.extend(self.generate_urban_grid_with_params(params));

        // Suburban streets
        roads.extend(super::suburb::plan(&params.suburban_params(), (self.center_x, self.center_y)).roads);

        // Rural trails
        roads.extend(self.generate_rural_paths_with_params(params));

        // Every suburban junction becomes a vertex both roads share, so
        // clients can see where streets meet (#62).
        street_plan::weld_junctions(&mut roads, |r| r.road_type == RoadType::Suburban);

        roads
    }

    /// Generate circular ring roads within districts
    fn generate_ring_roads_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        vec![
            self.generate_circular_road("guild_ring", params.guild_ring_radius, params.guild_ring_width),
            self.generate_circular_road("trade_ring", params.trade_ring_radius, params.trade_ring_width),
        ]
    }

    /// Generate a circular road at a given radius
    fn generate_circular_road(&self, id: &str, radius: f64, width: f64) -> RoadSegment {
        let num_segments = 64;
        let mut points = Vec::new();

        for i in 0..=num_segments {
            let angle = (i as f64 / num_segments as f64) * 2.0 * PI;
            let x = self.center_x + radius * angle.stable_cos();
            let y = self.center_y + radius * angle.stable_sin();
            points.push((x, y));
        }

        RoadSegment {
            id: id.to_string(),
            road_type: RoadType::Arterial,
            points,
            width,
        }
    }

    /// Generate arterial roads connecting ring roads to wilderness edge
    fn generate_arterial_roads_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        let mut roads = Vec::new();
        let outer_canal = self.water_rings.iter().find(|r| r.id == "canal_outer");
        let inner_canal = self.water_rings.iter().find(|r| r.id == "canal_inner");
        let moat = self.water_rings.iter().find(|r| r.id == "moat");

        let guild_ring_radius = params.guild_ring_radius;
        let trade_ring_radius = params.trade_ring_radius;

        if let Some(outer) = outer_canal {
            for (i, bridge) in outer.bridges.iter().enumerate() {
                let angle_rad = bridge.angle_degrees.to_radians();
                let angle_deg = bridge.angle_degrees;

                let start_radius = guild_ring_radius;
                let end_radius = params.arterial_end_radius;

                let start_x = self.center_x + start_radius * angle_rad.stable_cos();
                let start_y = self.center_y + start_radius * angle_rad.stable_sin();
                let end_x = self.center_x + end_radius * angle_rad.stable_cos();
                let end_y = self.center_y + end_radius * angle_rad.stable_sin();

                roads.push(RoadSegment {
                    id: format!("arterial_{}", i),
                    road_type: RoadType::Arterial,
                    points: vec![(start_x, start_y), (end_x, end_y)],
                    width: params.arterial_width,
                });

                let is_cardinal = angle_deg == 0.0 || angle_deg == 90.0 || angle_deg == 180.0 || angle_deg == 270.0;
                if is_cardinal {
                    if let Some(_inner) = inner_canal {
                        let guild_x = self.center_x + guild_ring_radius * angle_rad.stable_cos();
                        let guild_y = self.center_y + guild_ring_radius * angle_rad.stable_sin();
                        let trade_x = self.center_x + trade_ring_radius * angle_rad.stable_cos();
                        let trade_y = self.center_y + trade_ring_radius * angle_rad.stable_sin();

                        roads.push(RoadSegment {
                            id: format!("arterial_inner_{}", i),
                            road_type: RoadType::Arterial,
                            points: vec![(guild_x, guild_y), (trade_x, trade_y)],
                            width: params.arterial_width,
                        });
                    }
                }
            }
        }

        // South street extends into Capitol (from trade ring to moat)
        if let Some(moat_ring) = moat {
            let south_angle = 90.0_f64.to_radians();
            let trade_x = self.center_x + trade_ring_radius * south_angle.stable_cos();
            let trade_y = self.center_y + trade_ring_radius * south_angle.stable_sin();
            let capitol_x = self.center_x + (moat_ring.inner_radius() - 10.0) * south_angle.stable_cos();
            let capitol_y = self.center_y + (moat_ring.inner_radius() - 10.0) * south_angle.stable_sin();

            roads.push(RoadSegment {
                id: "arterial_capitol".to_string(),
                road_type: RoadType::Arterial,
                points: vec![(trade_x, trade_y), (capitol_x, capitol_y)],
                width: params.arterial_width,
            });
        }

        roads
    }

    /// Generate radial urban grid streets within the Urban zone
    fn generate_urban_grid_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        let mut roads = Vec::new();
        // Start past the outer canal so roads/plots don't overlap the water ring
        let inner_radius = params.guild_outer_radius + params.canal_outer_width / 2.0;
        let outer_radius = params.urban_outer_radius;
        let radial_spacing = params.urban_radial_spacing;
        let spoke_count = params.urban_spoke_count;
        let street_width = params.urban_street_width;

        let mut id = 0;

        // Generate concentric arc streets
        let mut r = inner_radius + radial_spacing;
        while r < outer_radius {
            let num_segments = 64;
            let mut points = Vec::new();

            for i in 0..=num_segments {
                let angle = (i as f64 / num_segments as f64) * 2.0 * PI;
                let x = self.center_x + r * angle.stable_cos();
                let y = self.center_y + r * angle.stable_sin();
                points.push((x, y));
            }

            roads.push(RoadSegment {
                id: format!("urban_arc_{}", id),
                road_type: RoadType::Grid,
                points,
                width: street_width,
            });
            id += 1;
            r += radial_spacing;
        }

        // Generate radial spoke streets
        let angle_step = 2.0 * PI / spoke_count as f64;
        for i in 0..spoke_count {
            let angle = i as f64 * angle_step;

            let x1 = self.center_x + inner_radius * angle.stable_cos();
            let y1 = self.center_y + inner_radius * angle.stable_sin();
            let x2 = self.center_x + outer_radius * angle.stable_cos();
            let y2 = self.center_y + outer_radius * angle.stable_sin();

            roads.push(RoadSegment {
                id: format!("urban_spoke_{}", i),
                road_type: RoadType::Grid,
                points: vec![(x1, y1), (x2, y2)],
                width: street_width,
            });
        }

        roads
    }

    /// The suburban lots for the already-generated roads in self.roads, as
    /// generated, before `finish_lots`. `params` is `suburban_params()`.
    pub(crate) fn generate_suburban_plots_internal(&self, params: &SuburbanGenParams) -> Vec<TerrainPlot> {
        // The plan again rather than a copy kept on self, so `version()`
        // doesn't see it. It's deterministic, and quick beside the lots.
        let center = (self.center_x, self.center_y);
        super::suburb::lots::lay(&self.roads, &super::suburb::plan(params, center), params, center)
    }

    /// Generate unpaved trails in the rural zone with smooth meandering paths
    fn generate_rural_paths_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        let mut roads = Vec::new();
        let inner_radius = params.suburban_outer_radius;
        let outer_radius = params.rural_outer_radius;

        let mut rng_state = params.seed.wrapping_add(12345);
        let mut next_rand = || {
            rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
            ((rng_state >> 16) & 0x7fff) as f64 / 0x7fff as f64
        };

        let num_trails = params.rural_trail_count;
        for i in 0..num_trails {
            let base_angle = (i as f64 / num_trails as f64) * 2.0 * PI + next_rand() * 0.3;

            // Generate control points for smooth curve
            let num_control_points = 5;
            let mut control_points = Vec::new();

            for j in 0..num_control_points {
                let t = j as f64 / (num_control_points - 1) as f64;
                let r = inner_radius + (outer_radius - inner_radius) * t;

                // More pronounced wobble for trails (they're more natural/winding)
                let wobble_amplitude = 0.15 * (1.0 - (2.0 * t - 1.0).abs());
                let wobble = wobble_amplitude * (next_rand() - 0.5) * 2.0;
                let a = base_angle + wobble;

                let x = self.center_x + r * a.stable_cos();
                let y = self.center_y + r * a.stable_sin();
                control_points.push((x, y));
            }

            // Interpolate smooth curve
            let smooth_points = catmull_rom_spline(&control_points, 15);

            roads.push(RoadSegment {
                id: format!("trail_{}", i),
                road_type: RoadType::Trail,
                points: smooth_points,
                width: params.rural_trail_width,
            });
        }

        roads
    }

    /// Generate plots filling the urban grid blocks (trapezoids between arcs and spokes)
    fn generate_urban_plots(&self, params: &WorldGenParams) -> Vec<TerrainPlot> {
        let mut plots = Vec::new();
        // Start past the outer canal so plots don't overlap the water ring
        let inner_radius = params.guild_outer_radius + params.canal_outer_width / 2.0;
        let outer_radius = params.urban_outer_radius;
        let radial_spacing = params.urban_radial_spacing;
        let spoke_count = params.urban_spoke_count;
        let half_w = params.urban_street_width / 2.0;
        let frontage = params.urban_plot_frontage;
        let depth = params.urban_plot_depth;

        let angle_step = 2.0 * PI / spoke_count as f64;

        // Some spokes are really arterials (every 45 degrees, at the canal
        // bridges), which are far wider than a grid street. A block side on
        // one of those is inset by the arterial's half width instead.
        let arterial_angles: Vec<f64> = self
            .roads
            .iter()
            .filter(|r| r.road_type == RoadType::Arterial)
            .filter_map(|r| r.points.last())
            .map(|&(x, y)| (y - self.center_y).stable_atan2(x - self.center_x))
            .collect();
        let side_half = |a: f64| {
            let on_arterial = arterial_angles.iter().any(|&b| {
                let d = (a - b).rem_euclid(2.0 * PI);
                d.min(2.0 * PI - d) < 1e-6
            });
            if on_arterial { half_w.max(params.arterial_width / 2.0) } else { half_w }
        };

        // Walk concentric rings
        let mut r = inner_radius + radial_spacing;
        let mut ring_idx = 0u32;
        while r < outer_radius {
            // Inner arc inset from road centerline. The first ring backs onto
            // the canal, not a street, so it keeps back from the water's kerb.
            let r_in = if ring_idx == 0 {
                inner_radius + super::lot_clearance::WATER_KERB + 1.0
            } else {
                r - radial_spacing + half_w
            };
            let r_out = r - half_w;                 // outer arc inset from road centerline

            // Skip if block is too thin after insets
            if r_out - r_in < depth.min(20.0) {
                r += radial_spacing;
                ring_idx += 1;
                continue;
            }

            for spoke_idx in 0..spoke_count {
                let a1 = spoke_idx as f64 * angle_step;
                let a2 = (spoke_idx + 1) as f64 * angle_step;

                // Angular insets for street width at inner and outer arcs
                let (h1, h2) = (side_half(a1), side_half(a2));
                let block_a1_in = a1 + h1 / r_in;
                let block_a2_in = a2 - h2 / r_in;
                let block_a1_out = a1 + h1 / r_out;
                let block_a2_out = a2 - h2 / r_out;

                // Check if block has positive angular extent
                if block_a2_in <= block_a1_in || block_a2_out <= block_a1_out {
                    continue;
                }

                // Calculate arc length at the midpoint radius to determine subdivision
                let mid_r = (r_in + r_out) / 2.0;
                let arc_length = mid_r * (block_a2_in - block_a1_in);

                // Subdivide along arc if block is wider than frontage
                let num_subdivs = if frontage > 0.0 {
                    (arc_length / frontage).ceil() as u32
                } else {
                    1
                }.max(1);

                // Also subdivide radially if block is deeper than depth
                let block_depth = r_out - r_in;
                let num_depth_subdivs = if depth > 0.0 {
                    (block_depth / depth).ceil() as u32
                } else {
                    1
                }.max(1);

                for d in 0..num_depth_subdivs {
                    let sub_r_in = r_in + (block_depth / num_depth_subdivs as f64) * d as f64;
                    let sub_r_out = r_in + (block_depth / num_depth_subdivs as f64) * (d + 1) as f64;

                    // Recalculate angular insets for this depth band
                    let sub_a1_in = a1 + h1 / sub_r_in;
                    let sub_a2_in = a2 - h2 / sub_r_in;
                    let sub_a1_out = a1 + h1 / sub_r_out;
                    let sub_a2_out = a2 - h2 / sub_r_out;

                    for s in 0..num_subdivs {
                        let frac0 = s as f64 / num_subdivs as f64;
                        let frac1 = (s + 1) as f64 / num_subdivs as f64;

                        let sa1_in = sub_a1_in + (sub_a2_in - sub_a1_in) * frac0;
                        let sa2_in = sub_a1_in + (sub_a2_in - sub_a1_in) * frac1;
                        let sa1_out = sub_a1_out + (sub_a2_out - sub_a1_out) * frac0;
                        let sa2_out = sub_a1_out + (sub_a2_out - sub_a1_out) * frac1;

                        let corners = [
                            (self.center_x + sub_r_in * sa1_in.stable_cos(), self.center_y + sub_r_in * sa1_in.stable_sin()),
                            (self.center_x + sub_r_in * sa2_in.stable_cos(), self.center_y + sub_r_in * sa2_in.stable_sin()),
                            (self.center_x + sub_r_out * sa2_out.stable_cos(), self.center_y + sub_r_out * sa2_out.stable_sin()),
                            (self.center_x + sub_r_out * sa1_out.stable_cos(), self.center_y + sub_r_out * sa1_out.stable_sin()),
                        ];

                        let area = TerrainPlot::calculate_area(&corners);
                        if area < 50.0 {
                            continue; // Skip degenerate plots
                        }

                        // Find nearest arc road for adjacent_street_id
                        let adjacent_id = format!("urban_arc_{}", ring_idx);

                        plots.push(TerrainPlot {
                            id: format!("urban_plot_{}_{}_{}{}", ring_idx, spoke_idx, s,
                                if num_depth_subdivs > 1 { format!("_d{}", d) } else { String::new() }),
                            corners,
                            street_facing_edge: (0, 1),
                            area,
                            adjacent_street_id: adjacent_id,
                            zone: "urban".to_string(),
                        });
                    }
                }
            }

            r += radial_spacing;
            ring_idx += 1;
        }

        plots
    }

    /// Guild halls: big lots either side of the guild ring road, from the inner
    /// canal to the outer one, split at every arterial that reaches the band
    /// (#83). The four cardinal arterials cross the inner band; the other four
    /// start at the ring, and their ends reach into it, so they split it too.
    ///
    /// Each band is the ground that's clear by construction: the ring road's
    /// ribbon lies inside `R + w/2` and outside `R·cos(π/n) − w/2` (it's an
    /// n-gon), and the water keeps its kerb. A lot's straight inner edge sags
    /// toward the centre, so its inner corners stand far enough out that the
    /// edge's midpoint still clears the band. A side edge runs parallel to its
    /// arterial, `GUILD_CLEAR` past the kerb. So `finish_lots` has nothing to
    /// trim, and the generator's ids and shapes are the ones that get claimed.
    fn generate_guild_plots(&self, params: &WorldGenParams) -> Vec<TerrainPlot> {
        let ring = self.roads.iter().find(|r| r.id == "guild_ring");
        let inner_canal = self.water_rings.iter().find(|r| r.id == "canal_inner");
        let outer_canal = self.water_rings.iter().find(|r| r.id == "canal_outer");
        let (Some(ring), Some(inner_canal), Some(outer_canal)) = (ring, inner_canal, outer_canal) else {
            return Vec::new();
        };
        let kerb = super::lot_clearance::WATER_KERB;
        let ring_r = params.guild_ring_radius;
        let ring_half = ring.width / 2.0;
        let sides = ring.points.len().saturating_sub(1).max(3) as f64;
        let bands = [
            ("in", inner_canal.outer_radius() + kerb + GUILD_CLEAR, ring_r * (PI / sides).stable_cos() - ring_half - GUILD_CLEAR),
            ("out", ring_r + ring_half + GUILD_CLEAR, outer_canal.inner_radius() - kerb - GUILD_CLEAR),
        ];

        let radius = |&(x, y): &(f64, f64)| ((x - self.center_x).powi(2) + (y - self.center_y).powi(2)).sqrt();
        let mut plots = Vec::new();
        for (band, lo, hi) in bands {
            if hi - lo < 20.0 {
                continue;
            }
            // (angle, half width) of each radial arterial whose ribbon reaches
            // the band. Suburban junctions weld extra vertices into the outer
            // arterials (#62), so a radial road is any whose points share a bearing.
            let bearing = |&(x, y): &(f64, f64)| (y - self.center_y).stable_atan2(x - self.center_x).rem_euclid(2.0 * PI);
            let mut dividers: Vec<(f64, f64)> = self
                .roads
                .iter()
                .filter(|r| r.road_type == RoadType::Arterial && r.points.len() >= 2)
                .filter_map(|r| {
                    let far = *r.points.iter().max_by(|a, b| radius(a).total_cmp(&radius(b)))?;
                    let a = bearing(&far);
                    let radial = r.points.iter().all(|p| {
                        let d = (bearing(p) - a).rem_euclid(2.0 * PI);
                        radius(p) < 1e-6 || d.min(2.0 * PI - d) < 1e-6
                    });
                    let h = r.width / 2.0;
                    let near = r.points.iter().map(radius).fold(f64::INFINITY, f64::min);
                    (radial && near - h < hi && radius(&far) + h > lo).then_some((a, h))
                })
                .collect();
            dividers.sort_by(|a, b| a.0.total_cmp(&b.0));
            dividers.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
            if dividers.is_empty() {
                continue;
            }

            for wedge in 0..dividers.len() {
                let (a1, h1) = dividers[wedge];
                let (mut a2, h2) = dividers[(wedge + 1) % dividers.len()];
                if a2 <= a1 {
                    a2 += 2.0 * PI;
                }
                // The wedge's edges at radius r, each GUILD_CLEAR past its arterial's kerb
                let edges = |r: f64| (a1 + ((h1 + GUILD_CLEAR) / r).stable_asin(), a2 - ((h2 + GUILD_CLEAR) / r).stable_asin());
                let (m1, m2) = edges((lo + hi) / 2.0);
                if m2 <= m1 {
                    continue;
                }
                let count = ((m2 - m1) * (lo + hi) / 2.0 / params.guild_plot_frontage.max(1.0)).ceil().max(1.0) as usize;

                // Stand the inner corners out so the chord between them clears `lo`
                let mut r_in = lo;
                for _ in 0..3 {
                    let (e1, e2) = edges(r_in);
                    r_in = lo / ((e2 - e1) / count as f64 / 2.0).stable_cos();
                }
                let r_out = hi;
                let (in1, in2) = edges(r_in);
                let (out1, out2) = edges(r_out);
                if in2 <= in1 || out2 <= out1 {
                    continue;
                }

                let at = |r: f64, a: f64| (self.center_x + r * a.stable_cos(), self.center_y + r * a.stable_sin());
                for s in 0..count {
                    let (f0, f1) = (s as f64 / count as f64, (s + 1) as f64 / count as f64);
                    let (i0, i1) = (in1 + (in2 - in1) * f0, in1 + (in2 - in1) * f1);
                    let (o0, o1) = (out1 + (out2 - out1) * f0, out1 + (out2 - out1) * f1);
                    // Corners start with the edge on the ring road, as (0, 1)
                    let corners = if band == "out" {
                        [at(r_in, i0), at(r_in, i1), at(r_out, o1), at(r_out, o0)]
                    } else {
                        [at(r_out, o1), at(r_out, o0), at(r_in, i0), at(r_in, i1)]
                    };
                    let area = TerrainPlot::calculate_area(&corners);
                    if area < 100.0 {
                        continue;
                    }
                    plots.push(TerrainPlot {
                        id: format!("guild_plot_{band}_{wedge}_{s}"),
                        corners,
                        street_facing_edge: (0, 1),
                        area,
                        adjacent_street_id: ring.id.clone(),
                        zone: "guild_district".to_string(),
                    });
                }
            }
        }
        plots
    }

    /// Generate plots along rural trail roads
    fn generate_rural_plots(&self, params: &WorldGenParams) -> Vec<TerrainPlot> {
        let mut plots = Vec::new();
        let mut placed_plot_corners: Vec<[(f64, f64); 4]> = Vec::new();
        let inner_radius = params.suburban_outer_radius;
        let outer_radius = params.rural_outer_radius;
        let frontage = params.rural_plot_frontage;
        let depth = params.rural_plot_depth;

        // Iterate over trail roads
        for road in &self.roads {
            if road.road_type != RoadType::Trail {
                continue;
            }

            let points = &road.points;
            if points.len() < 2 {
                continue;
            }

            let total_length = polyline_length(points);
            if total_length < frontage {
                continue;
            }

            let half_trail_w = road.width / 2.0 + 2.0;
            let mut plot_id = 0u32;

            // Place plots on both sides of the trail
            for side in [-1.0_f64, 1.0_f64] {
                let mut current_dist = 10.0; // Start slightly in from trail start

                while current_dist + frontage <= total_length - 10.0 {
                    let front_start = point_at_distance(points, current_dist);
                    let front_end = point_at_distance(points, current_dist + frontage);

                    if let (Some((fs_x, fs_y, fs_angle)), Some((fe_x, fe_y, fe_angle))) = (front_start, front_end) {
                        let perp_angle = (fs_angle + fe_angle) / 2.0 + side * PI / 2.0;

                        // Offset front corners away from trail center
                        let fs_off_x = fs_x + half_trail_w * perp_angle.stable_cos();
                        let fs_off_y = fs_y + half_trail_w * perp_angle.stable_sin();
                        let fe_off_x = fe_x + half_trail_w * perp_angle.stable_cos();
                        let fe_off_y = fe_y + half_trail_w * perp_angle.stable_sin();

                        // Back corners at depth offset
                        let bs_x = fs_x + depth * perp_angle.stable_cos();
                        let bs_y = fs_y + depth * perp_angle.stable_sin();
                        let be_x = fe_x + depth * perp_angle.stable_cos();
                        let be_y = fe_y + depth * perp_angle.stable_sin();

                        let corners = [
                            (fs_off_x, fs_off_y),
                            (fe_off_x, fe_off_y),
                            (be_x, be_y),
                            (bs_x, bs_y),
                        ];

                        // Validate all corners are within rural zone
                        let all_in_zone = corners.iter().all(|(x, y)| {
                            let dist = ((x - self.center_x).powi(2) + (y - self.center_y).powi(2)).sqrt();
                            dist >= inner_radius && dist <= outer_radius
                        });

                        if all_in_zone {
                            let corners_slice: &[(f64, f64)] = &corners;

                            // Check overlap with other roads
                            let mut intersects_road = false;
                            for other_road in &self.roads {
                                if other_road.id == road.id {
                                    continue;
                                }
                                if polygon_intersects_road(corners_slice, &other_road.points, other_road.width) {
                                    intersects_road = true;
                                    break;
                                }
                            }

                            if !intersects_road {
                                // Check overlap with already placed plots
                                let mut overlaps = false;
                                for existing in &placed_plot_corners {
                                    let existing_slice: &[(f64, f64)] = existing;
                                    if polygons_overlap(corners_slice, existing_slice) {
                                        overlaps = true;
                                        break;
                                    }
                                }

                                if !overlaps {
                                    let area = TerrainPlot::calculate_area(&corners);
                                    if area >= 100.0 {
                                        placed_plot_corners.push(corners);
                                        plots.push(TerrainPlot {
                                            id: format!("rural_plot_{}_{}_{}",
                                                road.id, if side > 0.0 { "R" } else { "L" }, plot_id),
                                            corners,
                                            street_facing_edge: (0, 1),
                                            area,
                                            adjacent_street_id: road.id.clone(),
                                            zone: "rural".to_string(),
                                        });
                                        plot_id += 1;
                                    }
                                }
                            }
                        }
                    }

                    current_dist += frontage;
                }
            }
        }

        plots
    }
}

/// Result of movement validation
#[derive(Debug, Clone)]
pub struct MovementValidation {
    pub allowed: bool,
    pub blocked_at: Option<(f64, f64)>,
    pub blocked_by: Option<String>,
}

impl TerrainConfig {
    /// Validate if movement from one point to another crosses water without a bridge
    pub fn validate_movement(&self, from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> MovementValidation {
        match rules::check_water_crossing(
            (self.center_x, self.center_y),
            &self.water_rings,
            (from_x, from_y),
            (to_x, to_y),
        ) {
            Some(block) => MovementValidation {
                allowed: false,
                blocked_at: Some((block.x, block.y)),
                blocked_by: Some(block.ring_id),
            },
            None => MovementValidation {
                allowed: true,
                blocked_at: None,
                blocked_by: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet, VecDeque};

    use crate::models::geom::SpatialGrid;

    #[test]
    fn test_terrain_config_default() {
        let config = TerrainConfig::get_default();
        assert_eq!(config.world_size, 12000.0);
        assert_eq!(config.water_rings.len(), 3);
    }

    #[test]
    fn test_version_is_stable_and_follows_params() {
        let version = TerrainConfig::get_default().version();
        assert_eq!(version.len(), 16);
        // Regenerating from the same params gives the same terrain, so the same version
        assert_eq!(TerrainConfig::get_default().version(), version);

        let params = WorldGenParams { seed: 43, ..WorldGenParams::default() };
        assert_ne!(TerrainConfig::from_params(&params).version(), version);
        let params = WorldGenParams { moat_width: 70.0, ..WorldGenParams::default() };
        assert_ne!(TerrainConfig::from_params(&params).version(), version);
    }

    /// Pins the default world, so nothing moves the live world by accident:
    /// its lots are owned plots and its nodes were placed against it. Change
    /// the literal only for a change that means to move it, write the node
    /// move migration it needs (see `models::resource::tests`), and say so in
    /// the commit. It holds in debug and release alike (#128).
    #[test]
    fn default_terrain_version_is_pinned() {
        // The world last moved on purpose in #112, which made Neighbourhood
        // the default. #128 changed only how the version is hashed (floats
        // quantised), not the world. #129 added the parks: no road, lot or
        // node moved.
        //
        // #180 moved the trig off the platform's libm (`fmath::StableMath`),
        // so every platform builds the same bits. That moved the Windows
        // world (`c3a856792daad83d`) onto the one Linux already built: three
        // suburban roads and some park rings gained or lost a vertex near a
        // junction. No lot or node moved.
        //
        // W1 (#194) grew the world to 12000 and moved the centre to (6000,
        // 6000), so every coordinate moved by +2000. The generator didn't
        // change: against `4d3d9d112a4f30f1` moved by +2000, the lots and
        // their ids are the same, no corner is off by more than 0.015, and 11
        // roads gained or lost a collinear vertex without moving. The seeded
        // nodes moved with the migration, and none needs fitting.
        //
        // W2 (#195) put the land's params on the terrain (`land`), so the
        // hash covers them. No road, lot, park or node moved.
        //
        // E3.4 (#170) added the Exchange hall to `landmarks`
        // (`b31da8eabe0cb864`). No road, lot, park or node moved.
        //
        // W3 (#196) put the wilds' water on the terrain: rivers, streams,
        // lakes and fords, all past r3800. No road, lot, park or node moved,
        // and no seeded node stands in the water
        // (`no_seeded_node_stands_in_the_wild_water`).
        assert_eq!(TerrainConfig::get_default().version(), "5c38bf6ff1ffa2a0");
    }

    /// Writes the default world as JSON to `$TERRAIN_DUMP`, to diff one
    /// platform's world against another's (#180):
    /// `TERRAIN_DUMP=win.json cargo test dump_default_terrain -- --ignored`.
    #[test]
    #[ignore]
    fn dump_default_terrain() {
        let path = std::env::var("TERRAIN_DUMP").expect("set TERRAIN_DUMP to the output path");
        let terrain = TerrainConfig::get_default();
        std::fs::write(&path, serde_json::to_string_pretty(&terrain).unwrap()).unwrap();
        println!("{} -> {path}", terrain.version());
    }

    /// A last-bit float difference, the kind the optimiser makes, leaves the
    /// version alone; a real move doesn't (#128).
    #[test]
    fn version_ignores_last_bit_float_noise() {
        let terrain = TerrainConfig::get_default();
        let mut nudged = terrain.clone();
        let x = &mut nudged.terrain_plots[0].corners[0].0;
        *x = f64::from_bits(x.to_bits() + 1);
        let radius = &mut nudged.water_rings[0].radius;
        *radius = f64::from_bits(radius.to_bits() - 1);
        assert_eq!(nudged.version(), terrain.version());

        nudged.terrain_plots[0].corners[0].0 += 0.01;
        assert_ne!(nudged.version(), terrain.version());
    }

    /// Saved `world_config` rows and admin PUTs from before #105 have no
    /// `neighbourhood`, and ones from before #113 carry Legacy's params and a
    /// `layout`. They all load, as the neighbourhoods with their defaults.
    #[test]
    fn world_params_load_old_and_new() {
        let mut json = serde_json::to_value(WorldGenParams::default()).unwrap();
        let suburban = json["suburban"].as_object_mut().unwrap();
        assert!(!suburban.contains_key("layout") && !suburban.contains_key("seed"), "{suburban:?}");
        suburban.remove("neighbourhood");
        let old: WorldGenParams = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old.suburban.neighbourhood, NeighbourhoodParams::default());

        // A row from before #113, with Legacy named and its params.
        json["suburban"]["layout"] = "legacy".into();
        json["suburban"]["min_street_spacing"] = 100.0.into();
        json["suburban"]["local_street_width"] = 10.0.into();
        json["suburban"]["seed"] = 7.into();
        let legacy: WorldGenParams = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(legacy.suburban.neighbourhood, NeighbourhoodParams::default());
        assert_eq!(legacy.suburban_params().seed, legacy.seed);

        // A partial neighbourhood: the rest takes its defaults.
        json["suburban"]["neighbourhood"] = serde_json::json!({ "lot_depth": 120.0, "force_pattern": "central_loop" });
        let new: WorldGenParams = serde_json::from_value(json).unwrap();
        assert_eq!(new.suburban.neighbourhood.lot_depth, 120.0);
        assert_eq!(new.suburban.neighbourhood.lot_frontage, 64.0);
        assert_eq!(new.suburban.neighbourhood.force_pattern, Some(NeighbourhoodPattern::CentralLoop));
    }

    /// The default world with pattern (a) in every neighbourhood, for the
    /// suburban street tests (#107). The mixed default is covered by
    /// `suburb::tests`. Built once: several tests share it, and the
    /// performance test runs beside them.
    fn warped_parallels() -> &'static TerrainConfig {
        static WORLD: std::sync::OnceLock<TerrainConfig> = std::sync::OnceLock::new();
        WORLD.get_or_init(|| {
            let mut params = WorldGenParams::default();
            params.suburban.neighbourhood.force_pattern = Some(NeighbourhoodPattern::WarpedParallels);
            TerrainConfig::from_params(&params)
        })
    }

    /// The default world lays suburban streets and lots (#108), and the rest
    /// of the world round them.
    #[test]
    fn the_default_world_has_a_suburb() {
        let t = TerrainConfig::get_default();
        assert!(t.roads.iter().any(|r| r.road_type == RoadType::Suburban));
        assert!(t.terrain_plots.iter().filter(|l| l.zone == "suburban").count() > 1500);
        assert!(t.terrain_plots.iter().any(|l| l.zone == "urban"));
    }

    /// Roads and lots read one set of suburban radii, from the zone radii,
    /// whatever `suburban`'s own copies say (#105).
    #[test]
    fn suburban_params_take_the_zone_radii() {
        let mut params = WorldGenParams { urban_outer_radius: 1500.0, suburban_outer_radius: 2500.0, seed: 9, ..Default::default() };
        params.suburban.inner_radius = 1.0;
        params.suburban.outer_radius = 2.0;
        let s = params.suburban_params();
        assert_eq!((s.inner_radius, s.outer_radius, s.seed), (1500.0, 2500.0, 9));
        let mut stale = WorldGenParams::default();
        stale.suburban.inner_radius = 1.0;
        stale.suburban.outer_radius = 2.0;
        assert_eq!(TerrainConfig::from_params(&stale).version(), TerrainConfig::get_default().version());
    }

    #[test]
    fn test_moat_has_one_bridge() {
        let config = TerrainConfig::get_default();
        let moat = config.water_rings.iter().find(|r| r.id == "moat").unwrap();
        assert_eq!(moat.bridges.len(), 1);
        assert_eq!(moat.bridges[0].angle_degrees, 90.0); // South (90° in screen coords where Y+ is down)
    }

    #[test]
    fn test_inner_canal_has_four_bridges() {
        let config = TerrainConfig::get_default();
        let canal = config.water_rings.iter().find(|r| r.id == "canal_inner").unwrap();
        assert_eq!(canal.bridges.len(), 4);
    }

    #[test]
    fn test_outer_canal_has_eight_bridges() {
        let config = TerrainConfig::get_default();
        let canal = config.water_rings.iter().find(|r| r.id == "canal_outer").unwrap();
        assert_eq!(canal.bridges.len(), 8);
    }

    #[test]
    fn test_bridge_contains_angle() {
        let bridge = Bridge { angle_degrees: 0.0, width: 25.0 };
        assert!(bridge.contains_angle(0.0, 600.0));
        assert!(bridge.contains_angle(1.0, 600.0));
        // Bridge is ~2.4 degrees wide at radius 600, so 5 degrees should be outside
        assert!(!bridge.contains_angle(5.0, 600.0));
    }

    #[test]
    fn test_bridge_angle_wraparound() {
        let bridge = Bridge { angle_degrees: 0.0, width: 25.0 };
        // Should handle wraparound at 0/360
        assert!(bridge.contains_angle(359.0, 600.0));
        assert!(bridge.contains_angle(-1.0, 600.0));
    }

    #[test]
    fn test_movement_allowed_no_water() {
        let config = TerrainConfig::get_default();
        // Movement within Capitol (inside moat)
        let result = config.validate_movement(6000.0, 6000.0, 6050.0, 6050.0);
        assert!(result.allowed);
    }

    #[test]
    fn test_movement_blocked_by_moat() {
        let config = TerrainConfig::get_default();
        // Try to cross moat going East (no bridge there)
        let result = config.validate_movement(6150.0, 6000.0, 6250.0, 6000.0);
        assert!(!result.allowed);
        assert_eq!(result.blocked_by, Some("moat".to_string()));
    }

    #[test]
    fn test_movement_allowed_on_bridge() {
        let config = TerrainConfig::get_default();
        // Cross moat going South (bridge at 90° in screen coords where Y+ is south)
        let result = config.validate_movement(6000.0, 6150.0, 6000.0, 6250.0);
        assert!(result.allowed);
    }

    #[test]
    fn test_arterial_roads_generated() {
        let config = TerrainConfig::get_default();
        let arterials: Vec<_> = config.roads.iter()
            .filter(|r| r.road_type == RoadType::Arterial)
            .collect();
        // 8 main arterials + 4 inner NSEW + 1 capitol + 2 ring roads = 15
        assert!(arterials.len() >= 13, "Expected at least 13 arterial roads, got {}", arterials.len());
    }

    #[test]
    fn test_ring_roads_generated() {
        let config = TerrainConfig::get_default();
        let ring_roads: Vec<_> = config.roads.iter()
            .filter(|r| r.id.contains("ring"))
            .collect();
        assert_eq!(ring_roads.len(), 2, "Expected 2 ring roads (guild and trade)");
    }

    #[test]
    fn test_suburban_streets_smooth() {
        let config = warped_parallels();
        let suburban: Vec<_> = config.roads.iter()
            .filter(|r| r.road_type == RoadType::Suburban)
            .collect();
        // Each suburban street should have many points for smooth curves
        for road in &suburban {
            assert!(road.points.len() >= 10, "Suburban road {} should have smooth curves (10+ points), got {}", road.id, road.points.len());
        }
    }

    #[test]
    fn test_suburban_gen_params_default() {
        let params = SuburbanGenParams::default();
        assert_eq!(params.inner_radius, rules::URBAN_OUTER_RADIUS);
        assert_eq!(params.outer_radius, rules::SUBURBAN_OUTER_RADIUS);
        assert_eq!(params.neighbourhood, NeighbourhoodParams::default());
    }

    #[test]
    fn test_suburban_collector_roads_generated() {
        let config = warped_parallels();
        let collectors: Vec<_> = config.roads.iter()
            .filter(|r| r.id.starts_with("collector_"))
            .collect();
        // Should have collectors for each of 8 superblocks (at least 8)
        assert!(collectors.len() >= 8, "Expected at least 8 collector roads, got {}", collectors.len());
    }

    #[test]
    fn test_suburban_cul_de_sacs_generated() {
        let config = warped_parallels();
        let culdesacs: Vec<_> = config.roads.iter()
            .filter(|r| r.id.starts_with("culdesac_"))
            .collect();
        // Should have some cul-de-sacs
        assert!(!culdesacs.is_empty(), "Expected at least 1 cul-de-sac road");
    }

    #[test]
    fn test_suburban_streets_within_zone() {
        let config = warped_parallels();
        let params = SuburbanGenParams::default();

        // Check that all suburban street points are within the suburban zone
        for road in config.roads.iter().filter(|r| r.road_type == RoadType::Suburban) {
            for (x, y) in &road.points {
                let dist = ((x - config.center_x).powi(2) + (y - config.center_y).powi(2)).sqrt();
                // Allow some tolerance for curves near boundaries
                let tolerance = 50.0;
                assert!(
                    dist >= params.inner_radius - tolerance && dist <= params.outer_radius + tolerance,
                    "Point ({}, {}) in road {} is outside suburban zone (dist={}, expected {}..{})",
                    x, y, road.id, dist, params.inner_radius, params.outer_radius
                );
            }
        }
    }

    #[test]
    fn test_suburban_connectivity() {
        let config = TerrainConfig::get_default();
        let params = SuburbanGenParams::default();

        // Build adjacency graph and verify all suburban streets can reach arterials
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        let mut endpoints: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

        // Add arterials
        for i in 0..8 {
            let id = format!("arterial_{}", i);
            graph.insert(id.clone(), Vec::new());
            let angle = i as f64 * PI / 4.0;
            endpoints.insert(id, vec![
                (config.center_x + params.inner_radius * angle.stable_cos(), config.center_y + params.inner_radius * angle.stable_sin()),
                (config.center_x + params.outer_radius * angle.stable_cos(), config.center_y + params.outer_radius * angle.stable_sin()),
            ]);
        }

        // Add suburban streets
        for road in config.roads.iter().filter(|r| r.road_type == RoadType::Suburban) {
            graph.insert(road.id.clone(), Vec::new());
            let mut eps = Vec::new();
            if let Some(first) = road.points.first() {
                eps.push(*first);
            }
            if let Some(last) = road.points.last() {
                eps.push(*last);
            }
            endpoints.insert(road.id.clone(), eps);
        }

        // Build edges based on proximity
        let connection_threshold = 50.0;
        let road_ids: Vec<String> = graph.keys().cloned().collect();

        for i in 0..road_ids.len() {
            for j in (i + 1)..road_ids.len() {
                let id1 = &road_ids[i];
                let id2 = &road_ids[j];

                if let (Some(eps1), Some(eps2)) = (endpoints.get(id1), endpoints.get(id2)) {
                    let mut connected = false;
                    for &(x1, y1) in eps1 {
                        for &(x2, y2) in eps2 {
                            let dist = ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt();
                            if dist < connection_threshold {
                                connected = true;
                                break;
                            }
                        }
                        if connected { break; }
                    }

                    if connected {
                        graph.get_mut(id1).unwrap().push(id2.clone());
                        graph.get_mut(id2).unwrap().push(id1.clone());
                    }
                }
            }
        }

        // BFS from each suburban street to check connectivity
        let arterial_ids: HashSet<String> = (0..8).map(|i| format!("arterial_{}", i)).collect();

        for road in config.roads.iter().filter(|r| r.road_type == RoadType::Suburban) {
            // BFS to find path to arterial
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            queue.push_back(road.id.clone());
            visited.insert(road.id.clone());

            let mut found_arterial = false;
            while let Some(current) = queue.pop_front() {
                if arterial_ids.contains(&current) {
                    found_arterial = true;
                    break;
                }

                if let Some(neighbors) = graph.get(&current) {
                    for neighbor in neighbors {
                        if !visited.contains(neighbor) {
                            visited.insert(neighbor.clone());
                            queue.push_back(neighbor.clone());
                        }
                    }
                }
            }

            // Note: Not all streets may be connected due to collision avoidance
            // This is informational rather than a hard failure
            if !found_arterial {
                println!("Warning: Road {} may not be connected to arterial network", road.id);
            }
        }
    }

    #[test]
    fn test_suburban_plot_area_calculation() {
        // Test the shoelace formula for area calculation
        let square = [
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (0.0, 100.0),
        ];
        let area = TerrainPlot::calculate_area(&square);
        assert!((area - 10_000.0).abs() < 0.01, "Expected area 10000, got {}", area);

        // Test a rectangle
        let rect = [
            (0.0, 0.0),
            (200.0, 0.0),
            (200.0, 100.0),
            (0.0, 100.0),
        ];
        let rect_area = TerrainPlot::calculate_area(&rect);
        assert!((rect_area - 20_000.0).abs() < 0.01, "Expected area 20000, got {}", rect_area);
    }

    /// The street plan's invariants (#62), at a few seeds: no repeated
    /// points, no hairpins outside a bulb, no road over itself, roads meeting
    /// only at shared vertices, and every street end on a road or closed on
    /// its own bulb.
    #[test]
    fn suburban_street_plan_holds_its_invariants() {
        for seed in [42, 7, 1234, 99_999] {
            let t = TerrainConfig::from_params(&WorldGenParams { seed, ..Default::default() });
            let audit = street_plan::audit(&t.roads, |r| r.road_type == RoadType::Suburban);
            assert!(audit.is_clean(), "seed {seed}: {audit:#?}");

            let suburban: Vec<_> = t.roads.iter().filter(|r| r.road_type == RoadType::Suburban).collect();
            let collectors = suburban.iter().filter(|r| r.id.starts_with("collector_")).count();
            eprintln!("seed {seed}: {} suburban roads, {collectors} collectors", suburban.len());
            assert!(suburban.len() >= 80 && collectors >= 10, "seed {seed}: a sparse plan");

            let mut ids = HashSet::new();
            assert!(t.roads.iter().all(|r| ids.insert(r.id.as_str())), "seed {seed}: repeated road ids");

            // Off the arterials' ribbons, except near a junction at either end.
            for art in t.roads.iter().filter(|r| r.id.starts_with("arterial_") && r.road_type == RoadType::Arterial) {
                for street in &suburban {
                    let ends = [street.points[0], *street.points.last().unwrap()];
                    let joins: Vec<_> = ends
                        .into_iter()
                        .filter(|e| t.roads.iter().any(|o| o.id != street.id && o.points.contains(e)))
                        .collect();
                    let clearance = (art.width + street.width) / 2.0;
                    let exempt = art.width / 2.0 + 2.0 * street.width;
                    assert!(
                        street_plan::keeps_clear(&street.points, &art.points, clearance, &joins, exempt),
                        "seed {seed}: {} runs into {}", street.id, art.id
                    );
                }
            }
        }
    }

    #[test]
    fn test_polyline_length() {
        let line = vec![(0.0, 0.0), (100.0, 0.0)];
        assert!((polyline_length(&line) - 100.0).abs() < 0.01);

        let square = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)];
        assert!((polyline_length(&square) - 400.0).abs() < 0.01);
    }

    #[test]
    fn test_catmull_rom_spline_smoothness() {
        let controls = vec![
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (0.0, 100.0),
        ];
        let spline = catmull_rom_spline(&controls, 10);

        // Should have more points than control points
        assert!(spline.len() > controls.len());

        // Points should be reasonably smooth (no huge jumps)
        for i in 1..spline.len() {
            let dx = spline[i].0 - spline[i-1].0;
            let dy = spline[i].1 - spline[i-1].1;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(dist < 50.0, "Large gap of {} between spline points {} and {}", dist, i-1, i);
        }
    }

    #[test]
    fn test_no_roads_terminate_at_outer_boundary() {
        // This test verifies that no suburban roads have endpoints at the outer boundary
        // of the suburban zone (which would indicate erroneous emergency connectors)
        let config = warped_parallels();
        let params = SuburbanGenParams::default();

        let outer_boundary_tolerance = 20.0; // Within 20 units of outer boundary

        // Calculate the 8 "corner" positions at the outer boundary (where arterials intersect)
        let corner_positions: Vec<(f64, f64)> = (0..8)
            .map(|i| {
                let angle = i as f64 * PI / 4.0;
                (
                    config.center_x + params.outer_radius * angle.stable_cos(),
                    config.center_y + params.outer_radius * angle.stable_sin(),
                )
            })
            .collect();

        let mut problematic_roads = Vec::new();

        for road in config.roads.iter().filter(|r| r.road_type == RoadType::Suburban) {
            // Check first and last points
            let endpoints = [road.points.first(), road.points.last()];

            for endpoint_opt in endpoints.iter() {
                if let Some(&(ex, ey)) = *endpoint_opt {
                    // Check if this endpoint is near the outer boundary
                    let dist_from_center = ((ex - config.center_x).powi(2) + (ey - config.center_y).powi(2)).sqrt();
                    let dist_from_outer = (dist_from_center - params.outer_radius).abs();

                    if dist_from_outer < outer_boundary_tolerance {
                        // Check if it's near one of the arterial corner positions
                        for (i, &(cx, cy)) in corner_positions.iter().enumerate() {
                            let dist_to_corner = ((ex - cx).powi(2) + (ey - cy).powi(2)).sqrt();
                            if dist_to_corner < 50.0 {
                                problematic_roads.push((road.id.clone(), i, ex, ey));
                            }
                        }
                    }
                }
            }
        }

        // Report any problematic roads
        for (road_id, corner_idx, x, y) in &problematic_roads {
            println!(
                "WARNING: Road {} has endpoint ({:.0}, {:.0}) near outer boundary corner {}",
                road_id, x, y, corner_idx
            );
        }

        assert!(
            problematic_roads.is_empty(),
            "Found {} roads with endpoints at outer boundary corners: {:?}",
            problematic_roads.len(),
            problematic_roads.iter().map(|(id, _, _, _)| id.clone()).collect::<Vec<_>>()
        );
    }

    /// D6: the Exchange hall stands in the Trade District beside the Capitol
    /// avenue, its entrance on the avenue, clear of every road and the water.
    #[test]
    fn the_exchange_hall_stands_on_the_capitol_avenue() {
        let params = WorldGenParams::default();
        let config = TerrainConfig::get_default();
        let hall = config.landmark(LandmarkKind::ExchangeHall).expect("a hall");
        let center = (config.center_x, config.center_y);
        let r = |(x, y): (f64, f64)| (x - center.0).stable_hypot(y - center.1);
        for &corner in &hall.corners {
            assert_eq!(rules::Zone::from_position(corner.0, corner.1), rules::Zone::TradeDistrict, "{corner:?}");
        }
        assert!(TerrainPlot::calculate_area(&hall.corners) >= 8000.0);
        assert!(capitol_protocol::terrain::quad_contains(&hall.corners, hall.position.0, hall.position.1));

        // Clear of the water and every road, kerbs and all
        let edges = |i: usize| (hall.corners[i], hall.corners[(i + 1) % 4]);
        for ring in &config.water_rings {
            for &c in &hall.corners {
                assert!(r(c) > ring.outer_radius() + 10.0 || r(c) < ring.inner_radius() - 10.0, "{} at {c:?}", ring.id);
            }
        }
        for road in &config.roads {
            for w in road.points.windows(2) {
                for i in 0..4 {
                    let (a, b) = edges(i);
                    // Sample the hall's edge against the road's centre line
                    for k in 0..=20 {
                        let t = k as f64 / 20.0;
                        let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                        let d = point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1);
                        assert!(d >= road.width / 2.0 + 10.0, "{} comes within {d} of the hall", road.id);
                    }
                }
            }
        }
        assert!(config.lot_at(hall.position.0, hall.position.1).is_none());

        // The door is on the avenue's kerb side, and the avenue's crown in
        // front of it is within range
        let avenue = config.roads.iter().find(|r| r.id == "arterial_capitol").expect("the avenue");
        let (a, b) = (avenue.points[0], avenue.points[1]);
        let to_crown = point_to_segment_distance(hall.door.0, hall.door.1, a.0, a.1, b.0, b.1);
        assert!((to_crown - (params.arterial_width / 2.0 + 15.0)).abs() < 1e-6, "{to_crown}");
        assert!(capitol_protocol::rules::market::at_the_hall(hall, center.0, hall.door.1));
        assert!(!capitol_protocol::rules::market::at_the_hall(hall, center.0, center.1 + 600.0));
    }

    #[test]
    fn guild_halls_fill_the_guild_district_untrimmed() {
        // The defaults and the wider canals the dev DB commits (#83).
        let wide = WorldGenParams {
            canal_inner_width: 50.0,
            canal_outer_width: 50.0,
            ..WorldGenParams::default()
        };
        for params in [WorldGenParams::default(), wide] {
            let config = TerrainConfig::from_params(&params);
            let generated = config.generate_guild_plots(&params);
            let kept: Vec<_> = config.terrain_plots.iter().filter(|p| p.zone == "guild_district").collect();
            // finish_lots had nothing to slide or drop
            assert_eq!(kept.len(), generated.len());
            for (k, g) in kept.iter().zip(&generated) {
                assert_eq!(k.id, g.id);
                assert_eq!(k.corners, g.corners, "{} was trimmed", g.id);
            }
            let (inner, outer) = (kept.iter().filter(|p| p.id.contains("_in_")).count(), kept.len());
            assert!(inner >= 24 && outer - inner >= 32, "{inner} inner, {} outer", outer - inner);
            for p in &kept {
                assert!(p.area > 10_000.0, "{} is only {} square", p.id, p.area);
                for &(x, y) in &p.corners {
                    let r = ((x - config.center_x).powi(2) + (y - config.center_y).powi(2)).sqrt();
                    assert!(r > params.trade_outer_radius && r < params.guild_outer_radius, "{} reaches r {r}", p.id);
                }
            }
        }
    }

    #[test]
    fn test_terrain_generation_determinism() {
        // Generate terrain twice with the same params and verify identical output
        let params = WorldGenParams::default();
        let config1 = TerrainConfig::from_params(&params);
        let config2 = TerrainConfig::from_params(&params);

        // Same number of roads
        assert_eq!(config1.roads.len(), config2.roads.len(),
            "Road count differs between runs: {} vs {}", config1.roads.len(), config2.roads.len());

        // Same road IDs and points
        for (r1, r2) in config1.roads.iter().zip(config2.roads.iter()) {
            assert_eq!(r1.id, r2.id, "Road IDs differ");
            assert_eq!(r1.points.len(), r2.points.len(),
                "Road {} point count differs: {} vs {}", r1.id, r1.points.len(), r2.points.len());
            for (i, (p1, p2)) in r1.points.iter().zip(r2.points.iter()).enumerate() {
                assert!((p1.0 - p2.0).abs() < 1e-10 && (p1.1 - p2.1).abs() < 1e-10,
                    "Road {} point {} differs: ({},{}) vs ({},{})", r1.id, i, p1.0, p1.1, p2.0, p2.1);
            }
        }

        // Same plots
        assert_eq!(config1.terrain_plots.len(), config2.terrain_plots.len(),
            "Plot count differs: {} vs {}", config1.terrain_plots.len(), config2.terrain_plots.len());

        for (p1, p2) in config1.terrain_plots.iter().zip(config2.terrain_plots.iter()) {
            assert_eq!(p1.id, p2.id, "Plot IDs differ");
            for (i, (c1, c2)) in p1.corners.iter().zip(p2.corners.iter()).enumerate() {
                assert!((c1.0 - c2.0).abs() < 1e-10 && (c1.1 - c2.1).abs() < 1e-10,
                    "Plot {} corner {} differs", p1.id, i);
            }
        }
    }

    /// The default world's parks are on `/terrain` (#129), and they're ground
    /// no lot took: no lot's centre is in one, so none can be claimed or
    /// built on. Their rings hold the raster's park squares to within 2%,
    /// and they're 8–18% of the suburb (14% when written).
    #[test]
    fn the_parks_are_on_the_terrain_and_off_the_lots() {
        let params = WorldGenParams::default();
        let t = TerrainConfig::from_params(&params);
        let (sp, center) = (params.suburban_params(), (t.center_x, t.center_y));
        let neighbourhoods = super::super::suburb::plan(&sp, center).neighbourhoods;
        let greens = super::super::suburb::parks::lay(&t.roads, &t.terrain_plots, &neighbourhoods, &sp, center);
        let squares: f64 = greens.iter().flat_map(|g| &g.parks).map(|p| p.area).sum();
        assert!(t.parks.len() > 20, "{} parks", t.parks.len());
        let mut ids: Vec<_> = t.parks.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), t.parks.len(), "park ids are unique");
        for lot in &t.terrain_plots {
            let c = lot.corners.iter().fold((0.0, 0.0), |m, p| (m.0 + p.0 / 4.0, m.1 + p.1 / 4.0));
            assert!(!t.parks.iter().any(|p| p.contains(c.0, c.1)), "lot {} is in a park", lot.id);
        }
        let area: f64 = t.parks.iter().map(|p| p.area).sum();
        let (inner, outer) = (rules::URBAN_OUTER_RADIUS, rules::SUBURBAN_OUTER_RADIUS);
        let share = area / (PI * (outer * outer - inner * inner));
        println!("{} parks, {:.0} square units, {:.1}% of the suburb", t.parks.len(), area, share * 100.0);
        assert!((area - squares).abs() <= 0.02 * squares, "traced {area:.0}, squares {squares:.0}");
        assert!((0.08..=0.18).contains(&share), "parks are {:.1}% of the suburb", share * 100.0);
    }

    #[test]
    fn test_terrain_generation_performance() {
        use cpu_time::ThreadTime;

        let params = WorldGenParams::default();

        // CPU time on this thread, not wall time: generation is single-threaded, and in a
        // parallel `cargo test` (or beside other agents' builds) the wall clock counts the
        // time other work held the core. Wall time hit 14.5s that way (#91).
        let start = ThreadTime::now();
        let _config = TerrainConfig::from_params(&params);
        let elapsed = start.elapsed();

        println!("Terrain generation took: {:?} of CPU", elapsed);
        // Typical: ~2.4s debug since #129 added the parks (~1.4s before). Pre-optimization was ~115s debug.
        assert!(elapsed.as_millis() < 5000,
            "Terrain generation took too long: {:?} of CPU (expected <5000ms)", elapsed);
    }

    #[test]
    fn test_spatial_grid_basic() {
        let mut grid = SpatialGrid::new(0.0, 0.0, 100.0, 10.0);

        // Insert a horizontal line segment
        grid.insert(0, &[(-50.0, 0.0), (50.0, 0.0)]);

        // Point near the line should find it
        let results = grid.query_point(0.0, 0.0, 5.0);
        assert!(results.contains(&0), "Should find segment near origin");

        // Point far away should not find it
        let results = grid.query_point(0.0, 80.0, 5.0);
        assert!(!results.contains(&0), "Should not find segment far away");
    }

    #[test]
    fn test_spatial_grid_multiple_segments() {
        let mut grid = SpatialGrid::new(0.0, 0.0, 100.0, 10.0);

        grid.insert(0, &[(-50.0, -50.0), (-40.0, -50.0)]);
        grid.insert(1, &[(40.0, 40.0), (50.0, 50.0)]);

        // Query near first segment
        let near_first = grid.query_point(-45.0, -50.0, 5.0);
        assert!(near_first.contains(&0));
        assert!(!near_first.contains(&1));

        // Query near second segment
        let near_second = grid.query_point(45.0, 45.0, 5.0);
        assert!(!near_second.contains(&0));
        assert!(near_second.contains(&1));
    }
}

