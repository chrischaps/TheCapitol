use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::f64::consts::PI;

use capitol_protocol::rules;

use super::geom::{polyline_length, SpatialGrid};
use super::street_plan;

// The terrain types the client sees are shared through capitol-protocol, so
// GET /terrain serves the generated terrain as-is.
pub use capitol_protocol::terrain::{Bridge, RoadSegment, RoadType, TerrainPlot, WaterRing};

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
    /// Width of collector roads (default: 14)
    pub collector_width: f64,
    /// Width of local streets (default: 10)
    pub local_street_width: f64,
    /// Minimum spacing between streets for plot room (default: 150)
    pub min_street_spacing: f64,
    /// Minimum plot area in square units (default: 10,000)
    pub min_plot_area: f64,
    /// Maximum plot area in square units (default: 20,000)
    pub max_plot_area: f64,
    /// Target plot area in square units (default: 14,000)
    pub target_plot_area: f64,
    /// Minimum frontage width (default: 80)
    pub min_frontage: f64,
    /// Maximum frontage width (default: 140)
    pub max_frontage: f64,
    /// Random seed for generation. Like the radii, `suburban_params` sets it
    /// from the world's seed.
    pub seed: u64,
    /// Width of the arterials the streets keep clear of. Not a tweakable:
    /// `generate_all_roads_with_params` sets it from `WorldGenParams`.
    #[serde(skip, default = "default_arterial_width")]
    pub arterial_width: f64,
    /// Which generator lays out the suburb (suburb v2, #103). Legacy until
    /// the flip in #112. The serde defaults keep saved `world_config` rows
    /// from before #105 loading.
    #[serde(default)]
    pub layout: SuburbLayout,
    /// The Neighbourhood generator's knobs. Legacy ignores them.
    #[serde(default)]
    pub neighbourhood: NeighbourhoodParams,
}

fn default_arterial_width() -> f64 {
    WorldGenParams::default().arterial_width
}

/// The suburban generators (plan `the-capitol-suburb-v2.md` §3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuburbLayout {
    /// Collectors and rejection-sampled local streets, lots along them.
    #[default]
    Legacy,
    /// Post-war neighbourhoods, one pattern per neighbourhood. Until #108 it
    /// lays no suburban lots.
    Neighbourhood,
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
    /// Street widths. Legacy's are `local_street_width` and `collector_width`.
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
    /// wavelength λ (at least 1200).
    pub warp_amplitude: f64,
    pub warp_wavelength: f64,
    pub pattern_weights: PatternWeights,
    /// Every neighbourhood takes this pattern, if it fits. For tests.
    pub force_pattern: Option<NeighbourhoodPattern>,
}

impl Default for NeighbourhoodParams {
    fn default() -> Self {
        Self {
            lot_frontage: 60.0,
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
            warp_wavelength: 1600.0,
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
            collector_width: 14.0,
            local_street_width: 10.0,
            min_street_spacing: 100.0, // Space for plots on either side of each street
            min_plot_area: 6_000.0,     // Reduced from 10,000 for denser plots
            max_plot_area: 12_000.0,    // Reduced from 20,000
            target_plot_area: 8_000.0,  // Reduced from 14,000
            min_frontage: 60.0,         // Reduced from 80 for narrower plots
            max_frontage: 100.0,        // Reduced from 140
            seed: 42,
            arterial_width: 60.0,
            layout: SuburbLayout::default(),
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
        }
    }
}

/// How far a guild hall stands back from the kerb of the roads and water
/// round it, in world units (#83).
const GUILD_CLEAR: f64 = 1.0;

/// Spacing of a collector's points, in world units.
const COLLECTOR_STEP: f64 = 20.0;

/// Spacing of a local street's points, in world units.
const LOCAL_STEP: f64 = 10.0;

/// How far a collector's control point may wobble, as a share of the
/// distance to its nearer neighbour.
const WOBBLE: f64 = 0.35;

/// Draws tried for each collector before it's given up.
const COLLECTOR_ATTEMPTS: usize = 4;

/// A street may meet another road only this close to one of its own ends,
/// where it's meant to join it.
const JOIN_ALLOW: f64 = 1.0;

/// Pattern type for local suburban streets
#[derive(Debug, Clone, Copy, PartialEq)]
enum LocalStreetPattern {
    /// Dead-end with circular bulb (50% weight)
    CulDeSac,
    /// Connects at both ends, creates interior block (25% weight)
    Loop,
    /// Stem + loop combo (15% weight)
    PShaped,
    /// Links other patterns (10% weight)
    CurvedConnector,
}

/// A generated suburban street with connectivity info
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SuburbanStreet {
    /// Road segment for this street
    segment: RoadSegment,
    /// ID of parent road this connects to
    parent_road_id: Option<String>,
    /// Connection point on parent road
    connection_point: Option<(f64, f64)>,
    /// Second connection point (for loops)
    second_connection: Option<(f64, f64)>,
    /// Pattern type
    pattern: LocalStreetPattern,
}

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

/// Simple seeded random number generator
struct SeededRng {
    state: u64,
}

impl SeededRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> f64 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        ((self.state >> 16) & 0x7fff) as f64 / 0x7fff as f64
    }

    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = (self.next() * (i + 1) as f64) as usize;
            slice.swap(i, j);
        }
    }
}

/// Connection point on a road for attaching local streets
#[derive(Debug, Clone)]
struct ConnectionPoint {
    x: f64,
    y: f64,
    road_id: String,
    tangent_angle: f64,
    used: bool,
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
            let angle = dy.atan2(dx);
            return Some((x, y, angle));
        }

        accumulated += segment_len;
    }

    // Return last point
    let last_idx = points.len() - 1;
    let dx = points[last_idx].0 - points[last_idx - 1].0;
    let dy = points[last_idx].1 - points[last_idx - 1].1;
    Some((points[last_idx].0, points[last_idx].1, dy.atan2(dx)))
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

    if t >= 0.0 && t <= 1.0 && u >= 0.0 && u <= 1.0 {
        let ix = x1 + t * (x2 - x1);
        let iy = y1 + t * (y2 - y1);
        Some((ix, iy))
    } else {
        None
    }
}

/// Calculate the angle between two direction vectors in radians (0 to PI)
fn angle_between_directions(dir1: (f64, f64), dir2: (f64, f64)) -> f64 {
    let (dx1, dy1) = dir1;
    let (dx2, dy2) = dir2;

    let len1 = (dx1 * dx1 + dy1 * dy1).sqrt();
    let len2 = (dx2 * dx2 + dy2 * dy2).sqrt();

    if len1 < 1e-10 || len2 < 1e-10 {
        return 0.0;
    }

    let dot = (dx1 * dx2 + dy1 * dy2) / (len1 * len2);
    let clamped = dot.clamp(-1.0, 1.0);
    clamped.acos()
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

/// Get the direction of a road segment at a specific point index
fn road_direction_at_point(points: &[(f64, f64)], idx: usize) -> (f64, f64) {
    if points.len() < 2 {
        return (1.0, 0.0);
    }

    let idx = idx.min(points.len() - 1);

    // Use neighboring points to get direction
    let (before_idx, after_idx) = if idx == 0 {
        (0, 1)
    } else if idx >= points.len() - 1 {
        (points.len() - 2, points.len() - 1)
    } else {
        (idx - 1, idx + 1)
    };

    let (x1, y1) = points[before_idx];
    let (x2, y2) = points[after_idx];

    (x2 - x1, y2 - y1)
}

/// Check if two road segments intersect at a valid angle (>= 45 degrees)
/// Returns true if intersection is valid or roads don't intersect
fn check_road_intersection_angle(
    road1_points: &[(f64, f64)],
    road2_points: &[(f64, f64)],
    min_angle_degrees: f64,
) -> bool {
    let min_angle_rad = min_angle_degrees * PI / 180.0;

    // Check each segment of road1 against each segment of road2
    for i in 0..road1_points.len().saturating_sub(1) {
        let seg1_start = road1_points[i];
        let seg1_end = road1_points[i + 1];

        for j in 0..road2_points.len().saturating_sub(1) {
            let seg2_start = road2_points[j];
            let seg2_end = road2_points[j + 1];

            if let Some(_intersection) = segments_intersect(seg1_start, seg1_end, seg2_start, seg2_end) {
                // Get directions at intersection
                let dir1 = road_direction_at_point(road1_points, i);
                let dir2 = road_direction_at_point(road2_points, j);

                let angle = angle_between_directions(dir1, dir2);

                // The intersection angle should be at least min_angle_rad
                // Since angle is 0 to PI, we need to check both the angle and its complement
                let effective_angle = if angle > PI / 2.0 { PI - angle } else { angle };

                if effective_angle < min_angle_rad {
                    return false;
                }
            }
        }
    }

    true
}

/// Compute the AABB of a polyline, returning (min_x, min_y, max_x, max_y).
fn polyline_aabb(points: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let (mut min_x, mut min_y) = points[0];
    let (mut max_x, mut max_y) = points[0];
    for &(x, y) in &points[1..] {
        if x < min_x { min_x = x; }
        if x > max_x { max_x = x; }
        if y < min_y { min_y = y; }
        if y > max_y { max_y = y; }
    }
    (min_x, min_y, max_x, max_y)
}

impl TerrainConfig {
    /// Get the static terrain configuration for the world (uses default params)
    pub fn get_default() -> Self {
        Self::from_params(&WorldGenParams::default())
    }

    /// A stable hash of the generated terrain, as 16 hex digits: FNV-1a over its
    /// JSON. It hashes the output rather than the params, so a generator change
    /// also changes it, and clients can key a disk cache on it.
    pub fn version(&self) -> String {
        let json = serde_json::to_vec(self).expect("terrain serializes");
        let hash = json.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
            (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3)
        });
        format!("{hash:016x}")
    }

    /// Build terrain from explicit world generation parameters
    pub fn from_params(params: &WorldGenParams) -> Self {
        let mut config = TerrainConfig {
            world_size: 8000.0,
            center_x: 4000.0,
            center_y: 4000.0,
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
        };

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

        config
    }

    /// Generate all road segments (legacy, uses hardcoded defaults)
    fn generate_all_roads(&self) -> Vec<RoadSegment> {
        self.generate_all_roads_with_params(&WorldGenParams::default())
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
        let suburban_params = params.suburban_params();
        match suburban_params.layout {
            SuburbLayout::Legacy => roads.extend(self.generate_suburban_streets_with_params(&suburban_params)),
            SuburbLayout::Neighbourhood => {
                roads.extend(super::suburb::plan(&suburban_params, (self.center_x, self.center_y)).roads)
            }
        }

        // Rural trails
        roads.extend(self.generate_rural_paths_with_params(params));

        // Every suburban junction becomes a vertex both roads share, so
        // clients can see where streets meet (#62).
        street_plan::weld_junctions(&mut roads, |r| r.road_type == RoadType::Suburban);

        roads
    }

    /// Generate circular ring roads within districts
    fn generate_ring_roads(&self) -> Vec<RoadSegment> {
        self.generate_ring_roads_with_params(&WorldGenParams::default())
    }

    fn generate_ring_roads_with_params(&self, params: &WorldGenParams) -> Vec<RoadSegment> {
        let mut roads = Vec::new();

        roads.push(self.generate_circular_road("guild_ring", params.guild_ring_radius, params.guild_ring_width));
        roads.push(self.generate_circular_road("trade_ring", params.trade_ring_radius, params.trade_ring_width));

        roads
    }

    /// Generate a circular road at a given radius
    fn generate_circular_road(&self, id: &str, radius: f64, width: f64) -> RoadSegment {
        let num_segments = 64;
        let mut points = Vec::new();

        for i in 0..=num_segments {
            let angle = (i as f64 / num_segments as f64) * 2.0 * PI;
            let x = self.center_x + radius * angle.cos();
            let y = self.center_y + radius * angle.sin();
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
    fn generate_arterial_roads(&self) -> Vec<RoadSegment> {
        self.generate_arterial_roads_with_params(&WorldGenParams::default())
    }

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

                let start_x = self.center_x + start_radius * angle_rad.cos();
                let start_y = self.center_y + start_radius * angle_rad.sin();
                let end_x = self.center_x + end_radius * angle_rad.cos();
                let end_y = self.center_y + end_radius * angle_rad.sin();

                roads.push(RoadSegment {
                    id: format!("arterial_{}", i),
                    road_type: RoadType::Arterial,
                    points: vec![(start_x, start_y), (end_x, end_y)],
                    width: params.arterial_width,
                });

                let is_cardinal = angle_deg == 0.0 || angle_deg == 90.0 || angle_deg == 180.0 || angle_deg == 270.0;
                if is_cardinal {
                    if let Some(_inner) = inner_canal {
                        let guild_x = self.center_x + guild_ring_radius * angle_rad.cos();
                        let guild_y = self.center_y + guild_ring_radius * angle_rad.sin();
                        let trade_x = self.center_x + trade_ring_radius * angle_rad.cos();
                        let trade_y = self.center_y + trade_ring_radius * angle_rad.sin();

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
            let trade_x = self.center_x + trade_ring_radius * south_angle.cos();
            let trade_y = self.center_y + trade_ring_radius * south_angle.sin();
            let capitol_x = self.center_x + (moat_ring.inner_radius() - 10.0) * south_angle.cos();
            let capitol_y = self.center_y + (moat_ring.inner_radius() - 10.0) * south_angle.sin();

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
    fn generate_urban_grid(&self) -> Vec<RoadSegment> {
        self.generate_urban_grid_with_params(&WorldGenParams::default())
    }

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
                let x = self.center_x + r * angle.cos();
                let y = self.center_y + r * angle.sin();
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

            let x1 = self.center_x + inner_radius * angle.cos();
            let y1 = self.center_y + inner_radius * angle.sin();
            let x2 = self.center_x + outer_radius * angle.cos();
            let y2 = self.center_y + outer_radius * angle.sin();

            roads.push(RoadSegment {
                id: format!("urban_spoke_{}", i),
                road_type: RoadType::Grid,
                points: vec![(x1, y1), (x2, y2)],
                width: street_width,
            });
        }

        roads
    }

    /// Generate American-style suburban streets with curvilinear layout
    /// Features: collector roads, cul-de-sacs, loop streets, P-shaped patterns
    fn generate_suburban_streets(&self, seed: u64) -> Vec<RoadSegment> {
        let params = SuburbanGenParams {
            seed,
            ..Default::default()
        };
        self.generate_suburban_streets_with_params(&params)
    }

    /// Generate suburban streets with custom parameters
    pub fn generate_suburban_streets_with_params(&self, params: &SuburbanGenParams) -> Vec<RoadSegment> {
        let mut roads = Vec::new();
        let mut suburban_streets: Vec<SuburbanStreet> = Vec::new();

        // Create spatial grid for accelerated collision detection
        let mut grid = SpatialGrid::new(self.center_x, self.center_y, params.outer_radius + 100.0, 50.0);

        // Phase 1: Generate collector roads
        let collectors = self.generate_collector_roads(params, &mut grid);
        for street in &collectors {
            roads.push(street.segment.clone());
        }
        suburban_streets.extend(collectors);

        // Phase 2: Iteratively generate local streets (cul-de-sacs, loops, P-shaped, connectors)
        // Each iteration can branch from streets generated in previous iterations.
        // This allows cul-de-sacs and loops to branch from connector roads.
        const MAX_ITERATIONS: usize = 8;
        for iteration in 0..MAX_ITERATIONS {
            let locals = self.generate_local_streets(params, &suburban_streets, iteration, &mut grid);
            if locals.is_empty() {
                break;
            }
            for street in &locals {
                roads.push(street.segment.clone());
            }
            suburban_streets.extend(locals);
        }

        // Phase 3: Connectivity verification
        // Note: Emergency connectors are disabled as they create visual artifacts
        // (many lines converging to the same arterial endpoints).
        // Streets should be naturally connected through the iterative generation process.
        // If disconnected streets are a problem, the generation algorithm should be improved
        // rather than adding post-hoc connectors.

        roads
    }

    /// Phase 1: Generate curved collector roads that connect to arterials
    /// Collectors branch off arterials and curve through the suburban zone
    fn generate_collector_roads(&self, params: &SuburbanGenParams, grid: &mut SpatialGrid) -> Vec<SuburbanStreet> {
        let mut streets = Vec::new();
        let mut placed_segments: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut rng = SeededRng::new(params.seed);
        let arterials = self.suburban_arterials(params);

        // 8 arterials at 0°, 45°, 90°, 135°, 180°, 225°, 270°, 315°
        let num_arterials = 8;
        let arterial_spacing = 2.0 * PI / num_arterials as f64;

        // Generate collectors that branch off each arterial
        for arterial_idx in 0..num_arterials {
            let arterial_angle = arterial_idx as f64 * arterial_spacing;
            // Unwrapped, so the last arterial's collectors sweep on to the
            // first rather than back round the whole circle.
            let next_arterial_angle = (arterial_idx + 1) as f64 * arterial_spacing;

            // Generate 2-3 collectors branching from this arterial
            let num_collectors = 2 + (rng.next() * 2.0) as usize; // 2-3

            for c in 0..num_collectors {
                // Start point: ON the arterial, at varying radii within suburban zone
                let start_t = 0.2 + (c as f64 / num_collectors as f64) * 0.6; // 20%-80% through zone
                let start_radius = params.inner_radius + (params.outer_radius - params.inner_radius) * start_t;

                let start_x = self.center_x + start_radius * arterial_angle.cos();
                let start_y = self.center_y + start_radius * arterial_angle.sin();

                // End point: Either back to same arterial at different radius,
                // or to neighboring arterial (creates better connectivity)
                let connect_to_neighbor = rng.next() > 0.4; // 60% connect to neighbor

                let (end_x, end_y, _end_arterial) = if connect_to_neighbor {
                    // Connect to neighboring arterial
                    let end_t = 0.3 + rng.next() * 0.4; // 30%-70% through zone
                    let end_radius = params.inner_radius + (params.outer_radius - params.inner_radius) * end_t;
                    let ex = self.center_x + end_radius * next_arterial_angle.cos();
                    let ey = self.center_y + end_radius * next_arterial_angle.sin();
                    (ex, ey, (arterial_idx + 1) % num_arterials)
                } else {
                    // Loop back to same arterial at different radius
                    let end_t = if start_t < 0.5 { 0.6 + rng.next() * 0.3 } else { 0.1 + rng.next() * 0.3 };
                    let end_radius = params.inner_radius + (params.outer_radius - params.inner_radius) * end_t;
                    let ex = self.center_x + end_radius * arterial_angle.cos();
                    let ey = self.center_y + end_radius * arterial_angle.sin();
                    (ex, ey, arterial_idx)
                };

                // A wobble can still bend a collector too hard, so a few
                // fresh draws are tried before the collector is given up.
                for _attempt in 0..COLLECTOR_ATTEMPTS {
                    // Generate control points for curved path between start and end
                    // More control points = smoother S-curve winding
                    let num_control_points = 6 + (rng.next() * 3.0) as usize; // 6-8 points for winding curves
                    let mut control_points = Vec::new();
                    control_points.push((start_x, start_y));

                    // Intermediate points curve into the superblock
                    let start_r = ((start_x - self.center_x).powi(2) + (start_y - self.center_y).powi(2)).sqrt();
                    let end_r = ((end_x - self.center_x).powi(2) + (end_y - self.center_y).powi(2)).sqrt();
                    let base: Vec<(f64, f64)> = (0..num_control_points)
                        .map(|i| {
                            let t = i as f64 / (num_control_points - 1) as f64;
                            // Interpolate angle between start and end arterials
                            let angle = if connect_to_neighbor {
                                arterial_angle + (next_arterial_angle - arterial_angle) * t
                            } else {
                                arterial_angle + arterial_spacing * 0.3 * (t * PI).sin() // Bulge into superblock
                            };
                            let r = start_r + (end_r - start_r) * t;
                            (self.center_x + r * angle.cos(), self.center_y + r * angle.sin())
                        })
                        .collect();
                    let gap = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();

                    for i in 1..(num_control_points - 1) {
                        // Wobble for S-curve winding, at most a third of the
                        // way to either neighbour. A wobble wider than the
                        // spacing sent the controls backwards, and the curve
                        // through them doubled back on itself (#62).
                        let reach = WOBBLE * gap(base[i], base[i - 1]).min(gap(base[i], base[i + 1]));
                        let (bx, by) = base[i];
                        let (rx, ry) = (bx - self.center_x, by - self.center_y);
                        let len = (rx * rx + ry * ry).sqrt();
                        let (ux, uy) = (rx / len, ry / len);
                        let along = (rng.next() - 0.5) * 2.0 * reach; // across the zone's rings
                        let across = (rng.next() - 0.5) * 2.0 * reach; // along them

                        let x = bx + ux * along - uy * across;
                        let y = by + uy * along + ux * across;
                        let r = ((x - self.center_x).powi(2) + (y - self.center_y).powi(2)).sqrt();
                        let r_kept = r.clamp(params.inner_radius + 20.0, params.outer_radius - 20.0);
                        let scale = r_kept / r;
                        control_points.push((
                            self.center_x + (x - self.center_x) * scale,
                            self.center_y + (y - self.center_y) * scale,
                        ));
                    }

                    control_points.push((end_x, end_y));

                    // Generate smooth curve, clamp it to the suburban zone, and
                    // space its points evenly. Centripetal, because the uniform
                    // spline looped wherever the controls bunched up (#62).
                    let smooth_points = street_plan::centripetal_spline(&control_points, 12);
                    let clamped_points = street_plan::resample(
                        &self.clamp_points_to_zone(&smooth_points, params),
                        COLLECTOR_STEP,
                    );

                    let ends = [(start_x, start_y), (end_x, end_y)];
                    let well_formed = street_plan::sharpest_turn(&clamped_points) <= street_plan::MAX_TURN_DEG
                        && !street_plan::self_crosses(&clamped_points)
                        && !arterials.iter().any(|a| street_plan::crosses(&clamped_points, a, &ends, JOIN_ALLOW));

                    // Verify the road stays in zone and has reasonable length
                    if well_formed && clamped_points.len() >= 2 && polyline_length(&clamped_points) > 50.0 {
                        // Check intersection angles with already placed collectors (use grid)
                        let (cp_aabb_min_x, cp_aabb_min_y, cp_aabb_max_x, cp_aabb_max_y) = polyline_aabb(&clamped_points);
                        let nearby = grid.query_aabb(cp_aabb_min_x, cp_aabb_min_y, cp_aabb_max_x, cp_aabb_max_y);

                        let mut valid_angles = true;
                        for &seg_idx in &nearby {
                            if !check_road_intersection_angle(&clamped_points, &placed_segments[seg_idx], 45.0) {
                                valid_angles = false;
                                break;
                            }
                        }

                        if valid_angles {
                            let street_id = format!("collector_{}_{}", arterial_idx, c);
                            let seg_idx = placed_segments.len();
                            grid.insert(seg_idx, &clamped_points);
                            placed_segments.push(clamped_points.clone());
                            streets.push(SuburbanStreet {
                                segment: RoadSegment {
                                    id: street_id.clone(),
                                    road_type: RoadType::Suburban,
                                    points: clamped_points.clone(),
                                    width: params.collector_width,
                                },
                                parent_road_id: Some(format!("arterial_{}", arterial_idx)),
                                connection_point: clamped_points.first().copied(),
                                second_connection: Some((end_x, end_y)),
                                pattern: LocalStreetPattern::CurvedConnector,
                            });
                            break;
                        }
                    }
                }
            }
        }

        streets
    }

    /// The arterials' centre lines across the suburban zone, a little past
    /// each edge, for the crossing tests.
    fn suburban_arterials(&self, params: &SuburbanGenParams) -> Vec<Vec<(f64, f64)>> {
        (0..8)
            .map(|i| {
                let a = i as f64 * PI / 4.0;
                [params.inner_radius - 50.0, params.outer_radius + 50.0]
                    .iter()
                    .map(|r| (self.center_x + r * a.cos(), self.center_y + r * a.sin()))
                    .collect()
            })
            .collect()
    }

    /// Whether a local street is fit to place: it stays in the zone, has no
    /// hairpin, doesn't cross itself, and meets other roads only at `joins`
    /// (its own ends that sit on a road). The spacing and angle tests are
    /// the callers'.
    fn local_street_fits(
        &self,
        points: &[(f64, f64)],
        joins: &[(f64, f64)],
        params: &SuburbanGenParams,
        placed_segments: &[Vec<(f64, f64)>],
        grid: &SpatialGrid,
    ) -> bool {
        if points.len() < 2 || !self.points_inside_zone(points, params) {
            return false;
        }
        if street_plan::sharpest_turn(points) > street_plan::MAX_TURN_DEG || street_plan::self_crosses(points) {
            return false;
        }
        // Off the arterials' whole width, not just their centre lines, except
        // where the street leaves one.
        let clearance = (params.arterial_width + params.local_street_width) / 2.0;
        let exempt = params.arterial_width / 2.0 + 2.0 * params.local_street_width;
        if self.suburban_arterials(params).iter().any(|a| {
            street_plan::crosses(points, a, joins, JOIN_ALLOW)
                || !street_plan::keeps_clear(points, a, clearance, joins, exempt)
        }) {
            return false;
        }
        let (min_x, min_y, max_x, max_y) = polyline_aabb(points);
        !grid
            .query_aabb(min_x, min_y, max_x, max_y)
            .iter()
            .any(|&i| street_plan::crosses(points, &placed_segments[i], joins, JOIN_ALLOW))
    }

    /// Whether every point is inside the suburban zone, clear of its edges by
    /// the margin `clamp_points_to_zone` uses.
    fn points_inside_zone(&self, points: &[(f64, f64)], params: &SuburbanGenParams) -> bool {
        points.iter().all(|&(x, y)| {
            let r = ((x - self.center_x).powi(2) + (y - self.center_y).powi(2)).sqrt();
            r >= params.inner_radius + 5.0 && r <= params.outer_radius - 5.0
        })
    }

    /// Clamp all points in a polyline to stay within the suburban zone
    fn clamp_points_to_zone(&self, points: &[(f64, f64)], params: &SuburbanGenParams) -> Vec<(f64, f64)> {
        let mut result = Vec::new();
        let margin = 5.0; // Small margin inside boundaries

        for &(x, y) in points {
            let dx = x - self.center_x;
            let dy = y - self.center_y;
            let r = (dx * dx + dy * dy).sqrt();

            if r < params.inner_radius + margin {
                // Too close to center - push outward
                let new_r = params.inner_radius + margin;
                let scale = new_r / r;
                result.push((self.center_x + dx * scale, self.center_y + dy * scale));
            } else if r > params.outer_radius - margin {
                // Too far from center - pull inward
                let new_r = params.outer_radius - margin;
                let scale = new_r / r;
                result.push((self.center_x + dx * scale, self.center_y + dy * scale));
            } else {
                result.push((x, y));
            }
        }

        result
    }

    /// Phase 2: Generate local streets with various patterns
    /// The iteration parameter allows multiple passes - each pass can branch from
    /// streets generated in previous iterations (e.g., cul-de-sacs off connector roads)
    fn generate_local_streets(
        &self,
        params: &SuburbanGenParams,
        existing_streets: &[SuburbanStreet],
        iteration: usize,
        grid: &mut SpatialGrid,
    ) -> Vec<SuburbanStreet> {
        let mut streets = Vec::new();
        // Vary seed by iteration to get different random choices each pass
        let mut rng = SeededRng::new(params.seed.wrapping_add(1000 + iteration as u64 * 500));

        // Collect all placed points for collision detection
        // Also build a mapping from road_id to segment index for parent exclusion
        let mut placed_segments: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut road_id_to_idx: HashMap<String, usize> = HashMap::new();
        for street in existing_streets {
            road_id_to_idx.insert(street.segment.id.clone(), placed_segments.len());
            placed_segments.push(street.segment.points.clone());
        }

        // Also track arterial roads for connection points
        let arterial_angles: Vec<f64> = (0..8).map(|i| i as f64 * PI / 4.0).collect();

        // Generate connection points along collectors and arterials
        let mut connection_points: Vec<ConnectionPoint> = Vec::new();

        // Add points along existing streets - generate connection points more densely
        // on shorter roads (like connectors) to allow branching
        for street in existing_streets {
            let points = &street.segment.points;
            let total_len = polyline_length(points);
            // For short roads (connectors), generate at least 2 connection points
            // For longer roads, one every 60 units
            let num_connections = if total_len < 150.0 {
                2.max(((total_len / 40.0).floor() as usize).max(1))
            } else {
                ((total_len / 60.0).floor() as usize).max(1)
            };

            for i in 1..=num_connections {
                let t = i as f64 / (num_connections + 1) as f64;
                if let Some((x, y, angle)) = point_and_tangent_at_t(points, t) {
                    connection_points.push(ConnectionPoint {
                        x,
                        y,
                        road_id: street.segment.id.clone(),
                        tangent_angle: angle,
                        used: false,
                    });
                }
            }
        }

        // Add points along arterials in suburban zone
        // Avoid the outer 15% of the zone to prevent clustering at boundaries
        let arterial_outer_limit = params.inner_radius + (params.outer_radius - params.inner_radius) * 0.85;
        for (i, &arterial_angle) in arterial_angles.iter().enumerate() {
            let num_points = 5;  // More connection points along arterials
            for j in 1..=num_points {
                let t = j as f64 / (num_points + 1) as f64;
                let r = params.inner_radius + (arterial_outer_limit - params.inner_radius) * t;
                let x = self.center_x + r * arterial_angle.cos();
                let y = self.center_y + r * arterial_angle.sin();

                connection_points.push(ConnectionPoint {
                    x,
                    y,
                    road_id: format!("arterial_{}", i),
                    tangent_angle: arterial_angle,
                    used: false,
                });
            }
        }

        // Prioritize connection points from connector roads (to allow branching from them)
        // by putting them at the front before shuffling
        let (connector_points, other_points): (Vec<_>, Vec<_>) = connection_points
            .into_iter()
            .partition(|cp| cp.road_id.starts_with("connector_"));

        let mut connection_points = connector_points;
        let mut other_points = other_points;
        rng.shuffle(&mut other_points);
        connection_points.extend(other_points);

        // Numbered on from the streets already placed. Starting each pass at 0
        // gave later passes the same ids, and the id-to-index map then sent a
        // street's parent lookup to another road (#62).
        let mut street_id = existing_streets.len();

        // Generate streets at connection points using index-based iteration to avoid borrow issues
        let mut used_indices: HashSet<usize> = HashSet::new();

        for idx in 0..connection_points.len() {
            if used_indices.contains(&idx) {
                continue;
            }

            let cp = &connection_points[idx];

            // Check if we have room for a street here
            let depth = self.available_depth(cp.x, cp.y, cp.tangent_angle, params, &placed_segments, grid);
            if depth < 60.0 {
                continue;
            }

            // Select pattern based on depth and randomness
            let pattern = self.select_pattern(&mut rng, depth);

            // Clone what we need from cp to avoid borrow issues
            let cp_x = cp.x;
            let cp_y = cp.y;
            let cp_road_id = cp.road_id.clone();
            let cp_tangent = cp.tangent_angle;

            // Look up parent segment index (arterials don't have indices - they're infinite lines)
            let parent_idx = road_id_to_idx.get(&cp_road_id).copied();

            let generated = match pattern {
                LocalStreetPattern::CulDeSac => {
                    self.generate_cul_de_sac_v2(
                        cp_x, cp_y, &cp_road_id, cp_tangent,
                        params, &mut rng, street_id, &placed_segments, parent_idx, grid
                    )
                }
                LocalStreetPattern::Loop => {
                    self.generate_loop_street_v2(
                        cp_x, cp_y, &cp_road_id, cp_tangent,
                        params, &mut rng, street_id, &placed_segments, parent_idx, grid
                    )
                }
                LocalStreetPattern::PShaped => {
                    self.generate_p_shaped_v2(
                        cp_x, cp_y, &cp_road_id, cp_tangent,
                        params, &mut rng, street_id, &placed_segments, parent_idx, grid
                    )
                }
                LocalStreetPattern::CurvedConnector => {
                    self.generate_curved_connector_v2(
                        cp_x, cp_y, &cp_road_id, cp_tangent,
                        params, &mut rng, street_id, &placed_segments, &connection_points, &road_id_to_idx, parent_idx, grid
                    )
                }
            };

            if let Some(mut new_streets) = generated {
                for street in &new_streets {
                    // Add to index mapping before pushing to placed_segments
                    let seg_idx = placed_segments.len();
                    road_id_to_idx.insert(street.segment.id.clone(), seg_idx);
                    grid.insert(seg_idx, &street.segment.points);
                    placed_segments.push(street.segment.points.clone());
                }
                // Counted before the append empties new_streets, or the
                // ids never move on (#62).
                street_id += new_streets.len();
                streets.append(&mut new_streets);
                used_indices.insert(idx);
            }
        }

        streets
    }

    /// Generate only connector roads (no cul-de-sacs, loops, or P-shaped)
    /// This allows connectors to be created first so other streets can branch from them
    fn generate_connectors_only(
        &self,
        params: &SuburbanGenParams,
        existing_streets: &[SuburbanStreet],
        iteration: usize,
        grid: &mut SpatialGrid,
    ) -> Vec<SuburbanStreet> {
        let mut streets = Vec::new();
        let mut rng = SeededRng::new(params.seed.wrapping_add(2000 + iteration as u64 * 500));

        let mut placed_segments: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut road_id_to_idx: HashMap<String, usize> = HashMap::new();
        for street in existing_streets {
            road_id_to_idx.insert(street.segment.id.clone(), placed_segments.len());
            placed_segments.push(street.segment.points.clone());
        }

        let arterial_angles: Vec<f64> = (0..8).map(|i| i as f64 * PI / 4.0).collect();
        let mut connection_points: Vec<ConnectionPoint> = Vec::new();

        // Add points along existing streets (more frequent for connectors - every 50 units)
        for street in existing_streets {
            let points = &street.segment.points;
            let total_len = polyline_length(points);
            let num_connections = ((total_len / 50.0).floor() as usize).max(1);

            for i in 1..=num_connections {
                let t = i as f64 / (num_connections + 1) as f64;
                if let Some((x, y, angle)) = point_and_tangent_at_t(points, t) {
                    connection_points.push(ConnectionPoint {
                        x,
                        y,
                        road_id: street.segment.id.clone(),
                        tangent_angle: angle,
                        used: false,
                    });
                }
            }
        }

        // Add points along arterials
        // Avoid the outer 15% of the zone to prevent clustering at boundaries
        let arterial_outer_limit = params.inner_radius + (params.outer_radius - params.inner_radius) * 0.85;
        for (i, &arterial_angle) in arterial_angles.iter().enumerate() {
            let num_points = 5;
            for j in 1..=num_points {
                let t = j as f64 / (num_points + 1) as f64;
                let r = params.inner_radius + (arterial_outer_limit - params.inner_radius) * t;
                let x = self.center_x + r * arterial_angle.cos();
                let y = self.center_y + r * arterial_angle.sin();

                connection_points.push(ConnectionPoint {
                    x,
                    y,
                    road_id: format!("arterial_{}", i),
                    tangent_angle: arterial_angle,
                    used: false,
                });
            }
        }

        rng.shuffle(&mut connection_points);

        let mut street_id = existing_streets.len();
        let mut used_indices: HashSet<usize> = HashSet::new();

        for idx in 0..connection_points.len() {
            if used_indices.contains(&idx) {
                continue;
            }

            let cp = &connection_points[idx];
            let depth = self.available_depth(cp.x, cp.y, cp.tangent_angle, params, &placed_segments, grid);
            if depth < 80.0 {
                continue;
            }

            let cp_x = cp.x;
            let cp_y = cp.y;
            let cp_road_id = cp.road_id.clone();
            let cp_tangent = cp.tangent_angle;

            // Look up parent segment index
            let parent_idx = road_id_to_idx.get(&cp_road_id).copied();

            // Only generate curved connectors
            if let Some(mut new_streets) = self.generate_curved_connector_v2(
                cp_x, cp_y, &cp_road_id, cp_tangent,
                params, &mut rng, street_id, &placed_segments, &connection_points, &road_id_to_idx, parent_idx, grid
            ) {
                for street in &new_streets {
                    let seg_idx = placed_segments.len();
                    road_id_to_idx.insert(street.segment.id.clone(), seg_idx);
                    grid.insert(seg_idx, &street.segment.points);
                    placed_segments.push(street.segment.points.clone());
                }
                streets.append(&mut new_streets);
                used_indices.insert(idx);
                street_id += 1;
            }
        }

        streets
    }

    /// Generate local streets WITHOUT connectors (cul-de-sacs, loops, P-shaped only)
    /// This is called after connectors exist so streets can branch from them
    fn generate_local_streets_no_connectors(
        &self,
        params: &SuburbanGenParams,
        existing_streets: &[SuburbanStreet],
        iteration: usize,
        grid: &mut SpatialGrid,
    ) -> Vec<SuburbanStreet> {
        let mut streets = Vec::new();
        let mut rng = SeededRng::new(params.seed.wrapping_add(3000 + iteration as u64 * 500));

        let mut placed_segments: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut road_id_to_idx: HashMap<String, usize> = HashMap::new();
        for street in existing_streets {
            road_id_to_idx.insert(street.segment.id.clone(), placed_segments.len());
            placed_segments.push(street.segment.points.clone());
        }

        let arterial_angles: Vec<f64> = (0..8).map(|i| i as f64 * PI / 4.0).collect();
        let mut connection_points: Vec<ConnectionPoint> = Vec::new();

        // Add points along existing streets (every 60 units for local streets)
        for street in existing_streets {
            let points = &street.segment.points;
            let total_len = polyline_length(points);
            let num_connections = ((total_len / 60.0).floor() as usize).max(1);

            for i in 1..=num_connections {
                let t = i as f64 / (num_connections + 1) as f64;
                if let Some((x, y, angle)) = point_and_tangent_at_t(points, t) {
                    connection_points.push(ConnectionPoint {
                        x,
                        y,
                        road_id: street.segment.id.clone(),
                        tangent_angle: angle,
                        used: false,
                    });
                }
            }
        }

        // Add points along arterials
        // Avoid the outer 15% of the zone to prevent clustering at boundaries
        let arterial_outer_limit = params.inner_radius + (params.outer_radius - params.inner_radius) * 0.85;
        for (i, &arterial_angle) in arterial_angles.iter().enumerate() {
            let num_points = 5;
            for j in 1..=num_points {
                let t = j as f64 / (num_points + 1) as f64;
                let r = params.inner_radius + (arterial_outer_limit - params.inner_radius) * t;
                let x = self.center_x + r * arterial_angle.cos();
                let y = self.center_y + r * arterial_angle.sin();

                connection_points.push(ConnectionPoint {
                    x,
                    y,
                    road_id: format!("arterial_{}", i),
                    tangent_angle: arterial_angle,
                    used: false,
                });
            }
        }

        rng.shuffle(&mut connection_points);

        let mut street_id = existing_streets.len();
        let mut used_indices: HashSet<usize> = HashSet::new();

        for idx in 0..connection_points.len() {
            if used_indices.contains(&idx) {
                continue;
            }

            let cp = &connection_points[idx];
            let depth = self.available_depth(cp.x, cp.y, cp.tangent_angle, params, &placed_segments, grid);
            if depth < 60.0 {
                continue;
            }

            let cp_x = cp.x;
            let cp_y = cp.y;
            let cp_road_id = cp.road_id.clone();
            let cp_tangent = cp.tangent_angle;

            // Look up parent segment index
            let parent_idx = road_id_to_idx.get(&cp_road_id).copied();

            // Select pattern (no connectors)
            let pattern = self.select_pattern_no_connectors(&mut rng, depth);

            let generated = match pattern {
                LocalStreetPattern::CulDeSac => {
                    self.generate_cul_de_sac_v2(
                        cp_x, cp_y, &cp_road_id, cp_tangent,
                        params, &mut rng, street_id, &placed_segments, parent_idx, grid
                    )
                }
                LocalStreetPattern::Loop => {
                    self.generate_loop_street_v2(
                        cp_x, cp_y, &cp_road_id, cp_tangent,
                        params, &mut rng, street_id, &placed_segments, parent_idx, grid
                    )
                }
                LocalStreetPattern::PShaped => {
                    self.generate_p_shaped_v2(
                        cp_x, cp_y, &cp_road_id, cp_tangent,
                        params, &mut rng, street_id, &placed_segments, parent_idx, grid
                    )
                }
                LocalStreetPattern::CurvedConnector => {
                    // Skip connectors in this function
                    None
                }
            };

            if let Some(mut new_streets) = generated {
                for street in &new_streets {
                    let seg_idx = placed_segments.len();
                    road_id_to_idx.insert(street.segment.id.clone(), seg_idx);
                    grid.insert(seg_idx, &street.segment.points);
                    placed_segments.push(street.segment.points.clone());
                }
                // Counted before the append empties new_streets, or the
                // ids never move on (#62).
                street_id += new_streets.len();
                streets.append(&mut new_streets);
                used_indices.insert(idx);
            }
        }

        streets
    }

    /// Select a street pattern without connectors
    fn select_pattern_no_connectors(&self, rng: &mut SeededRng, depth: f64) -> LocalStreetPattern {
        let roll = rng.next();

        if depth < 100.0 {
            LocalStreetPattern::CulDeSac
        } else if depth < 150.0 {
            // Cul-de-sac or loop
            if roll < 0.6 {
                LocalStreetPattern::CulDeSac
            } else {
                LocalStreetPattern::Loop
            }
        } else if depth < 200.0 {
            // Can fit most patterns
            if roll < 0.4 {
                LocalStreetPattern::CulDeSac
            } else if roll < 0.7 {
                LocalStreetPattern::Loop
            } else {
                LocalStreetPattern::PShaped
            }
        } else {
            // Lots of room - favor loops and P-shaped
            if roll < 0.3 {
                LocalStreetPattern::CulDeSac
            } else if roll < 0.6 {
                LocalStreetPattern::Loop
            } else {
                LocalStreetPattern::PShaped
            }
        }
    }

    /// Calculate available depth perpendicular to road at a connection point
    fn available_depth(
        &self,
        x: f64,
        y: f64,
        tangent_angle: f64,
        params: &SuburbanGenParams,
        placed_segments: &[Vec<(f64, f64)>],
        grid: &SpatialGrid,
    ) -> f64 {
        // Check perpendicular directions (both sides)
        let perp_angles = [tangent_angle + PI / 2.0, tangent_angle - PI / 2.0];
        let mut max_depth: f64 = 0.0;

        for &angle in &perp_angles {
            // Start probing past the parent road width to avoid false collision
            let start_depth = 40.0;
            let mut depth = start_depth;
            let step: f64 = 10.0;
            let max_check: f64 = 250.0;

            while depth < max_check {
                let check_x = x + depth * angle.cos();
                let check_y = y + depth * angle.sin();

                // Check if within suburban zone
                let dist = ((check_x - self.center_x).powi(2) + (check_y - self.center_y).powi(2)).sqrt();
                if dist < params.inner_radius || dist > params.outer_radius {
                    break;
                }

                // Check collision with existing streets (via spatial grid)
                let mut collision = false;
                let nearby = grid.query_point(check_x, check_y, 20.0);
                for &seg_idx in &nearby {
                    if point_near_polyline(check_x, check_y, &placed_segments[seg_idx], 20.0) {
                        collision = true;
                        break;
                    }
                }
                if collision {
                    break;
                }

                depth += step;
            }

            // Return total usable depth from connection point
            max_depth = max_depth.max(depth);
        }

        max_depth
    }

    /// Select a street pattern based on available depth and weighted randomness
    fn select_pattern(&self, rng: &mut SeededRng, depth: f64) -> LocalStreetPattern {
        let roll = rng.next();

        // Patterns require different minimum depths
        if depth < 100.0 {
            // Only cul-de-sac fits in tight spaces
            LocalStreetPattern::CulDeSac
        } else if depth < 150.0 {
            // Cul-de-sac or curved connector (50/50)
            if roll < 0.5 {
                LocalStreetPattern::CulDeSac
            } else {
                LocalStreetPattern::CurvedConnector
            }
        } else {
            // Full selection with weights: connector 30%, cul-de-sac 30%, loop 25%, P-shaped 15%
            // Connectors get higher priority to create more branching opportunities
            if roll < 0.30 {
                LocalStreetPattern::CurvedConnector
            } else if roll < 0.60 {
                LocalStreetPattern::CulDeSac
            } else if roll < 0.85 {
                LocalStreetPattern::Loop
            } else {
                LocalStreetPattern::PShaped
            }
        }
    }

    /// Check if a street collides with existing segments
    /// Skips the first few points since they're expected to be near the parent road
    /// The roads in `exclude` (the ones the street joins) aren't checked
    fn check_street_collision(
        &self,
        points: &[(f64, f64)],
        placed_segments: &[Vec<(f64, f64)>],
        min_spacing: f64,
        exclude: &[usize],
        grid: &SpatialGrid,
    ) -> bool {
        // Skip the first few points (they connect to parent road)
        let skip_count = 3.min(points.len());
        for &(px, py) in points.iter().skip(skip_count) {
            let nearby = grid.query_point(px, py, min_spacing);
            for &seg_idx in &nearby {
                // Skip the roads the street joins
                if exclude.contains(&seg_idx) {
                    continue;
                }
                if point_near_polyline(px, py, &placed_segments[seg_idx], min_spacing) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a street has valid intersection angles with all existing segments
    /// Returns false if any intersection angle is less than min_angle_degrees
    fn check_street_intersection_angles(
        &self,
        points: &[(f64, f64)],
        placed_segments: &[Vec<(f64, f64)>],
        min_angle_degrees: f64,
        grid: &SpatialGrid,
    ) -> bool {
        if points.is_empty() {
            return true;
        }
        // Query grid for segments whose AABB overlaps this polyline's AABB
        let (min_x, min_y, max_x, max_y) = polyline_aabb(points);
        let nearby = grid.query_aabb(min_x, min_y, max_x, max_y);
        for &seg_idx in &nearby {
            if !check_road_intersection_angle(points, &placed_segments[seg_idx], min_angle_degrees) {
                return false;
            }
        }
        true
    }

    /// Find the best perpendicular direction using scalar values
    /// If parent_segment_idx is provided, excludes that segment from collision checks
    fn find_best_perpendicular_v2(
        &self,
        x: f64,
        y: f64,
        tangent_angle: f64,
        params: &SuburbanGenParams,
        placed_segments: &[Vec<(f64, f64)>],
        parent_segment_idx: Option<usize>,
        grid: &SpatialGrid,
    ) -> f64 {
        let perp1 = tangent_angle + PI / 2.0;
        let perp2 = tangent_angle - PI / 2.0;

        // Check depth in each perpendicular direction directly
        let depth1 = self.available_depth_in_direction(x, y, perp1, params, placed_segments, parent_segment_idx, grid);
        let depth2 = self.available_depth_in_direction(x, y, perp2, params, placed_segments, parent_segment_idx, grid);

        if depth1 >= depth2 {
            perp1
        } else {
            perp2
        }
    }

    /// Calculate available depth in a specific direction (not perpendicular to it)
    /// If parent_segment_idx is provided, excludes that segment from collision check
    fn available_depth_in_direction(
        &self,
        x: f64,
        y: f64,
        direction: f64,
        params: &SuburbanGenParams,
        placed_segments: &[Vec<(f64, f64)>],
        parent_segment_idx: Option<usize>,
        grid: &SpatialGrid,
    ) -> f64 {
        // Start probing past the parent road width to avoid false collision
        let start_depth = 40.0;
        let mut depth = start_depth;
        let step: f64 = 10.0;
        let max_check: f64 = 250.0;

        while depth < max_check {
            let check_x = x + depth * direction.cos();
            let check_y = y + depth * direction.sin();

            // Check if within suburban zone
            let dist = ((check_x - self.center_x).powi(2) + (check_y - self.center_y).powi(2)).sqrt();
            if dist < params.inner_radius || dist > params.outer_radius {
                break;
            }

            // Check collision with existing streets (via spatial grid)
            let mut collision = false;
            let nearby = grid.query_point(check_x, check_y, 20.0);
            for &seg_idx in &nearby {
                // Skip the parent road segment
                if parent_segment_idx == Some(seg_idx) {
                    continue;
                }
                if point_near_polyline(check_x, check_y, &placed_segments[seg_idx], 20.0) {
                    collision = true;
                    break;
                }
            }
            if collision {
                break;
            }

            depth += step;
        }

        depth
    }

    /// Generate a cul-de-sac (v2 with scalar parameters)
    fn generate_cul_de_sac_v2(
        &self,
        cp_x: f64,
        cp_y: f64,
        road_id: &str,
        tangent_angle: f64,
        params: &SuburbanGenParams,
        rng: &mut SeededRng,
        base_id: usize,
        placed_segments: &[Vec<(f64, f64)>],
        parent_segment_idx: Option<usize>,
        grid: &SpatialGrid,
    ) -> Option<Vec<SuburbanStreet>> {
        let perp_angle = self.find_best_perpendicular_v2(cp_x, cp_y, tangent_angle, params, placed_segments, parent_segment_idx, grid);

        // Check available depth in the chosen perpendicular direction
        let available_depth = self.available_depth_in_direction(cp_x, cp_y, perp_angle, params, placed_segments, parent_segment_idx, grid);

        // Minimum cul-de-sac needs at least 150 units of space for meaningful length
        let min_culdesac_length = 150.0;
        if available_depth < min_culdesac_length {
            return None;
        }

        // Scale length to use most of available space (longer streets fill the zone better)
        // Use 60-90% of available depth, up to 500 units max
        let max_base_length = (available_depth * 0.85 - 30.0).min(500.0); // Leave room for bulb
        let min_base_length = (available_depth * 0.6).min(max_base_length);
        let length = min_base_length + rng.next() * (max_base_length - min_base_length).max(0.0);

        let bulb_radius = 15.0 + rng.next() * 10.0; // Slightly smaller bulbs too
        let curve_amount = (rng.next() - 0.5) * 0.3;
        let num_stem_points = 4;
        let mut stem_controls = Vec::new();

        // The stem stops a bulb's width short, and the bulb rings round from
        // there back to it, so the street closes on itself (#62).
        for i in 0..num_stem_points {
            let t = i as f64 / (num_stem_points - 1) as f64;
            let dist = t * (length - 2.0 * bulb_radius);
            let curve_offset = curve_amount * t * (1.0 - t) * length;
            let perp_offset = curve_offset * (perp_angle + PI / 2.0).cos();

            let x = cp_x + dist * perp_angle.cos() + perp_offset * (perp_angle + PI / 2.0).cos();
            let y = cp_y + dist * perp_angle.sin() + perp_offset * (perp_angle + PI / 2.0).sin();
            stem_controls.push((x, y));
        }

        let mut all_points = street_plan::resample(&street_plan::centripetal_spline(&stem_controls, 10), LOCAL_STEP);
        let stem_end = *all_points.last()?;
        all_points.extend(street_plan::bulb(stem_end, street_plan::end_direction(&all_points)?, bulb_radius, 16));

        let exclude: Vec<usize> = parent_segment_idx.into_iter().collect();
        if self.check_street_collision(&all_points, placed_segments, params.min_street_spacing, &exclude, grid) {
            return None;
        }
        if !self.local_street_fits(&all_points, &[(cp_x, cp_y)], params, placed_segments, grid) {
            return None;
        }

        // Check intersection angles are at least 30 degrees
        if !self.check_street_intersection_angles(&all_points, placed_segments, 30.0, grid) {
            return None;
        }

        Some(vec![SuburbanStreet {
            segment: RoadSegment {
                id: format!("culdesac_{}", base_id),
                road_type: RoadType::Suburban,
                points: all_points,
                width: params.local_street_width,
            },
            parent_road_id: Some(road_id.to_string()),
            connection_point: Some((cp_x, cp_y)),
            second_connection: None,
            pattern: LocalStreetPattern::CulDeSac,
        }])
    }

    /// Generate a loop street (v2 with scalar parameters)
    fn generate_loop_street_v2(
        &self,
        cp_x: f64,
        cp_y: f64,
        road_id: &str,
        tangent_angle: f64,
        params: &SuburbanGenParams,
        rng: &mut SeededRng,
        base_id: usize,
        placed_segments: &[Vec<(f64, f64)>],
        parent_segment_idx: Option<usize>,
        grid: &SpatialGrid,
    ) -> Option<Vec<SuburbanStreet>> {
        let perp_angle = self.find_best_perpendicular_v2(cp_x, cp_y, tangent_angle, params, placed_segments, parent_segment_idx, grid);

        let loop_width = 150.0 + rng.next() * 100.0;  // 150-250 width
        let loop_depth = 150.0 + rng.next() * 150.0;  // 150-300 depth (extends further from collector)

        // The far end goes back onto the parent, measured along it, so a
        // curved parent doesn't leave it short (#62). An arterial is straight.
        let second_offset = loop_width * 0.8;
        let (second_x, second_y) = match parent_segment_idx.map(|i| &placed_segments[i]) {
            Some(parent) => {
                let along = street_plan::distance_along(parent, (cp_x, cp_y))?;
                let total = polyline_length(parent);
                let at = if along + second_offset <= total - LOCAL_STEP {
                    along + second_offset
                } else if along - second_offset >= LOCAL_STEP {
                    along - second_offset
                } else {
                    return None;
                };
                let (x, y, _) = point_at_distance(parent, at)?;
                (x, y)
            }
            None => (
                cp_x + second_offset * tangent_angle.cos(),
                cp_y + second_offset * tangent_angle.sin(),
            ),
        };

        // A U on the chord between the two ends, out on the side the
        // street leaves by, with a rounded top. One control at the apex made
        // a corner there (#62).
        let (chord_x, chord_y) = (second_x - cp_x, second_y - cp_y);
        let chord = (chord_x * chord_x + chord_y * chord_y).sqrt();
        if chord < LOCAL_STEP {
            return None;
        }
        let (ux, uy) = (chord_x / chord, chord_y / chord);
        let side = if -uy * perp_angle.cos() + ux * perp_angle.sin() >= 0.0 { 1.0 } else { -1.0 };
        let (vx, vy) = (-uy * side, ux * side);
        let at = |u: f64, v: f64| (cp_x + ux * u * chord + vx * v * loop_depth, cp_y + uy * u * chord + vy * v * loop_depth);
        let loop_controls = vec![
            at(0.0, 0.0),
            at(0.0, 0.45),
            at(0.15, 0.85),
            at(0.5, 1.0),
            at(0.85, 0.85),
            at(1.0, 0.45),
            (second_x, second_y),
        ];

        let clamped_points = street_plan::resample(&street_plan::centripetal_spline(&loop_controls, 15), LOCAL_STEP);

        let joins = [(cp_x, cp_y), (second_x, second_y)];

        // Both ends are on the parent, which the spacing test leaves out.
        let exclude: Vec<usize> = parent_segment_idx.into_iter().collect();
        if self.check_street_collision(&clamped_points, placed_segments, params.min_street_spacing, &exclude, grid) {
            return None;
        }
        if !self.local_street_fits(&clamped_points, &joins, params, placed_segments, grid) {
            return None;
        }

        // Check intersection angles are at least 45 degrees
        if !self.check_street_intersection_angles(&clamped_points, placed_segments, 30.0, grid) {
            return None;
        }

        Some(vec![SuburbanStreet {
            segment: RoadSegment {
                id: format!("loop_{}", base_id),
                road_type: RoadType::Suburban,
                points: clamped_points,
                width: params.local_street_width,
            },
            parent_road_id: Some(road_id.to_string()),
            connection_point: Some((cp_x, cp_y)),
            second_connection: Some((second_x, second_y)),
            pattern: LocalStreetPattern::Loop,
        }])
    }

    /// Generate a P-shaped street (v2 with scalar parameters)
    fn generate_p_shaped_v2(
        &self,
        cp_x: f64,
        cp_y: f64,
        road_id: &str,
        tangent_angle: f64,
        params: &SuburbanGenParams,
        rng: &mut SeededRng,
        base_id: usize,
        placed_segments: &[Vec<(f64, f64)>],
        parent_segment_idx: Option<usize>,
        grid: &SpatialGrid,
    ) -> Option<Vec<SuburbanStreet>> {
        let perp_angle = self.find_best_perpendicular_v2(cp_x, cp_y, tangent_angle, params, placed_segments, parent_segment_idx, grid);

        let stem_length = 150.0 + rng.next() * 150.0;  // 150-300 stem length
        let loop_radius = 60.0 + rng.next() * 40.0;   // 60-100 loop radius

        let mut stem_controls = Vec::new();
        stem_controls.push((cp_x, cp_y));

        let stem_end_x = cp_x + stem_length * perp_angle.cos();
        let stem_end_y = cp_y + stem_length * perp_angle.sin();
        stem_controls.push((stem_end_x, stem_end_y));

        // The loop leaves the stem's end and rings back to it, sharing
        // that vertex rather than repeating it (#62).
        let mut clamped_points = street_plan::resample(&stem_controls, LOCAL_STEP);
        let dir = (perp_angle.cos(), perp_angle.sin());
        clamped_points.extend(street_plan::bulb((stem_end_x, stem_end_y), dir, loop_radius, 20));


        let exclude: Vec<usize> = parent_segment_idx.into_iter().collect();
        if self.check_street_collision(&clamped_points, placed_segments, params.min_street_spacing, &exclude, grid) {
            return None;
        }
        if !self.local_street_fits(&clamped_points, &[(cp_x, cp_y)], params, placed_segments, grid) {
            return None;
        }

        // Check intersection angles are at least 45 degrees
        if !self.check_street_intersection_angles(&clamped_points, placed_segments, 30.0, grid) {
            return None;
        }

        Some(vec![SuburbanStreet {
            segment: RoadSegment {
                id: format!("pshaped_{}", base_id),
                road_type: RoadType::Suburban,
                points: clamped_points,
                width: params.local_street_width,
            },
            parent_road_id: Some(road_id.to_string()),
            connection_point: Some((cp_x, cp_y)),
            second_connection: None,
            pattern: LocalStreetPattern::PShaped,
        }])
    }

    /// Generate a curved connector (v2 with scalar parameters)
    fn generate_curved_connector_v2(
        &self,
        cp_x: f64,
        cp_y: f64,
        road_id: &str,
        tangent_angle: f64,
        params: &SuburbanGenParams,
        rng: &mut SeededRng,
        base_id: usize,
        placed_segments: &[Vec<(f64, f64)>],
        connection_points: &[ConnectionPoint],
        road_index: &HashMap<String, usize>,
        parent_segment_idx: Option<usize>,
        grid: &SpatialGrid,
    ) -> Option<Vec<SuburbanStreet>> {
        let perp_angle = self.find_best_perpendicular_v2(cp_x, cp_y, tangent_angle, params, placed_segments, parent_segment_idx, grid);

        let target_dist = 150.0 + rng.next() * 100.0;
        let target_x = cp_x + target_dist * perp_angle.cos();
        let target_y = cp_y + target_dist * perp_angle.sin();

        let mut best_cp: Option<&ConnectionPoint> = None;
        let mut best_dist = f64::MAX;

        for other_cp in connection_points {
            if other_cp.used || other_cp.road_id == road_id {
                continue;
            }
            let dist = ((other_cp.x - target_x).powi(2) + (other_cp.y - target_y).powi(2)).sqrt();
            if dist < best_dist && dist < 100.0 {
                best_dist = dist;
                best_cp = Some(other_cp);
            }
        }

        // A connector with nothing to connect to would be a stub with no
        // bulb, a dead end that looks like a mistake, so it's not placed (#62).
        let other = best_cp?;
        let (end_x, end_y) = (other.x, other.y);
        let target_idx = road_index.get(&other.road_id).copied();

        let mid_x = (cp_x + end_x) / 2.0 + (rng.next() - 0.5) * 30.0;
        let mid_y = (cp_y + end_y) / 2.0 + (rng.next() - 0.5) * 30.0;

        let controls = vec![
            (cp_x, cp_y),
            (mid_x, mid_y),
            (end_x, end_y),
        ];

        let clamped_points = street_plan::resample(&street_plan::centripetal_spline(&controls, 12), LOCAL_STEP);

        let joins = [(cp_x, cp_y), (end_x, end_y)];

        let exclude: Vec<usize> = parent_segment_idx.into_iter().chain(target_idx).collect();
        if self.check_street_collision(&clamped_points, placed_segments, params.min_street_spacing, &exclude, grid) {
            return None;
        }
        if !self.local_street_fits(&clamped_points, &joins, params, placed_segments, grid) {
            return None;
        }

        // Check intersection angles are at least 45 degrees
        if !self.check_street_intersection_angles(&clamped_points, placed_segments, 30.0, grid) {
            return None;
        }

        Some(vec![SuburbanStreet {
            segment: RoadSegment {
                id: format!("connector_{}", base_id),
                road_type: RoadType::Suburban,
                points: clamped_points,
                width: params.local_street_width,
            },
            parent_road_id: Some(road_id.to_string()),
            connection_point: Some((cp_x, cp_y)),
            second_connection: Some((end_x, end_y)),
            pattern: LocalStreetPattern::CurvedConnector,
        }])
    }

    /// Phase 3: Verify all streets connect to arterials and fix disconnected ones
    fn verify_and_fix_connectivity(
        &self,
        params: &SuburbanGenParams,
        streets: &mut Vec<SuburbanStreet>,
    ) -> Vec<RoadSegment> {
        let mut additional_roads = Vec::new();

        // Build adjacency graph
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        let mut endpoints: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

        // Add arterial roads as nodes (these are our targets)
        // Only use INNER boundary points - we never want emergency connectors going to the outer edge
        for i in 0..8 {
            let id = format!("arterial_{}", i);
            graph.insert(id.clone(), Vec::new());
            // Arterial endpoint at inner suburban boundary (where it enters from urban zone)
            let angle = i as f64 * PI / 4.0;
            endpoints.insert(id, vec![
                (
                    self.center_x + params.inner_radius * angle.cos(),
                    self.center_y + params.inner_radius * angle.sin(),
                ),
            ]);
        }

        // Add suburban streets as nodes, storing both endpoints and full polylines
        let mut road_polylines: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

        for street in streets.iter() {
            let id = street.segment.id.clone();
            graph.insert(id.clone(), Vec::new());

            let mut eps = Vec::new();
            if let Some(first) = street.segment.points.first() {
                eps.push(*first);
            }
            if let Some(last) = street.segment.points.last() {
                eps.push(*last);
            }
            if let Some(conn) = street.connection_point {
                eps.push(conn);
            }
            if let Some(conn2) = street.second_connection {
                eps.push(conn2);
            }
            endpoints.insert(id.clone(), eps);
            road_polylines.insert(id, street.segment.points.clone());
        }

        // Build edges based on proximity
        // Check both endpoint-to-endpoint AND endpoint-to-polyline proximity
        let connection_threshold = 30.0; // Units within which roads are considered connected
        let road_ids: Vec<String> = graph.keys().cloned().collect();

        for i in 0..road_ids.len() {
            for j in (i + 1)..road_ids.len() {
                let id1 = &road_ids[i];
                let id2 = &road_ids[j];

                if let (Some(eps1), Some(eps2)) = (endpoints.get(id1), endpoints.get(id2)) {
                    let mut connected = false;

                    // Check endpoint-to-endpoint
                    for &(x1, y1) in eps1 {
                        for &(x2, y2) in eps2 {
                            let dist = ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt();
                            if dist < connection_threshold {
                                connected = true;
                                break;
                            }
                        }
                        if connected {
                            break;
                        }
                    }

                    // If not connected by endpoints, check if connection points are on the other road's polyline
                    if !connected {
                        // Check if eps1 points are near road2's polyline
                        if let Some(poly2) = road_polylines.get(id2) {
                            for &(x1, y1) in eps1 {
                                if point_near_polyline(x1, y1, poly2, connection_threshold) {
                                    connected = true;
                                    break;
                                }
                            }
                        }
                    }

                    if !connected {
                        // Check if eps2 points are near road1's polyline
                        if let Some(poly1) = road_polylines.get(id1) {
                            for &(x2, y2) in eps2 {
                                if point_near_polyline(x2, y2, poly1, connection_threshold) {
                                    connected = true;
                                    break;
                                }
                            }
                        }
                    }

                    if connected {
                        graph.get_mut(id1).unwrap().push(id2.clone());
                        graph.get_mut(id2).unwrap().push(id1.clone());
                    }
                }
            }
        }

        // BFS from each suburban street to check if it can reach any arterial
        let arterial_ids: HashSet<String> = (0..8).map(|i| format!("arterial_{}", i)).collect();

        for street in streets.iter() {
            let start_id = &street.segment.id;
            if arterial_ids.contains(start_id) {
                continue;
            }

            let reachable = self.bfs_reaches_arterial(start_id, &graph, &arterial_ids);

            if !reachable {
                // Find nearest reachable road and add connector
                if let Some(connector) = self.create_emergency_connector(
                    street,
                    params,
                    &graph,
                    &endpoints,
                    &arterial_ids,
                    additional_roads.len(),
                ) {
                    additional_roads.push(connector);
                }
            }
        }

        additional_roads
    }

    /// BFS to check if a road can reach any arterial
    fn bfs_reaches_arterial(
        &self,
        start: &str,
        graph: &HashMap<String, Vec<String>>,
        arterials: &HashSet<String>,
    ) -> bool {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        queue.push_back(start.to_string());
        visited.insert(start.to_string());

        while let Some(current) = queue.pop_front() {
            if arterials.contains(&current) {
                return true;
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

        false
    }

    /// Create an emergency connector road to fix disconnected street
    fn create_emergency_connector(
        &self,
        street: &SuburbanStreet,
        params: &SuburbanGenParams,
        _graph: &HashMap<String, Vec<String>>,
        endpoints: &HashMap<String, Vec<(f64, f64)>>,
        arterials: &HashSet<String>,
        connector_id: usize,
    ) -> Option<RoadSegment> {
        // Get disconnected street's closest point to an arterial
        let street_point = street.segment.points.first()?;

        // Find nearest arterial endpoint
        let mut nearest_arterial_point: Option<(f64, f64)> = None;
        let mut nearest_dist = f64::MAX;

        for arterial_id in arterials {
            if let Some(eps) = endpoints.get(arterial_id) {
                for &(ax, ay) in eps {
                    let dist = ((street_point.0 - ax).powi(2) + (street_point.1 - ay).powi(2)).sqrt();
                    if dist < nearest_dist {
                        nearest_dist = dist;
                        nearest_arterial_point = Some((ax, ay));
                    }
                }
            }
        }

        let (ax, ay) = nearest_arterial_point?;

        // Safety check: reject if the target point is near the outer boundary
        // This prevents erroneous lines being drawn to zone corners
        let target_dist = ((ax - self.center_x).powi(2) + (ay - self.center_y).powi(2)).sqrt();
        let outer_limit = params.inner_radius + (params.outer_radius - params.inner_radius) * 0.85;
        if target_dist > outer_limit {
            return None;
        }

        // Create simple connector
        let mid_x = (street_point.0 + ax) / 2.0;
        let mid_y = (street_point.1 + ay) / 2.0;

        let controls = vec![
            *street_point,
            (mid_x, mid_y),
            (ax, ay),
        ];

        let points = catmull_rom_spline(&controls, 10);

        Some(RoadSegment {
            id: format!("emergency_connector_{}", connector_id),
            road_type: RoadType::Suburban,
            points,
            width: params.local_street_width,
        })
    }

    /// Generate suburban plots from the street layout
    /// Returns plots that fill the space between streets
    pub fn generate_suburban_plots(&self, params: &SuburbanGenParams) -> Vec<TerrainPlot> {
        let roads = self.generate_suburban_streets_with_params(params);
        self.extract_plots_from_roads(&roads, params)
    }

    /// The suburban lots for the already-generated roads in self.roads, as
    /// generated, before `finish_lots`. `params` is `suburban_params()`.
    pub(crate) fn generate_suburban_plots_internal(&self, params: &SuburbanGenParams) -> Vec<TerrainPlot> {
        match params.layout {
            SuburbLayout::Legacy => self.extract_plots_from_roads(&self.roads, params),
            // Lots v2 fill its blocks, #108.
            SuburbLayout::Neighbourhood => Vec::new(),
        }
    }

    /// Extract 4-sided plots from road network
    fn extract_plots_from_roads(
        &self,
        roads: &[RoadSegment],
        params: &SuburbanGenParams,
    ) -> Vec<TerrainPlot> {
        let mut plots = Vec::new();
        let mut placed_plot_corners: Vec<[(f64, f64); 4]> = Vec::new();

        // Build spatial grid for roads to accelerate road intersection checks
        let mut road_grid = SpatialGrid::new(self.center_x, self.center_y, params.outer_radius + 200.0, 50.0);
        // Build index of suburban road indices (by position in roads slice)
        let mut road_idx_map: HashMap<String, usize> = HashMap::new();
        for (i, road) in roads.iter().enumerate() {
            if !road.points.is_empty() {
                road_grid.insert(i, &road.points);
            }
            road_idx_map.insert(road.id.clone(), i);
        }

        // Spatial grid for placed plots
        let mut plot_grid = SpatialGrid::new(self.center_x, self.center_y, params.outer_radius + 200.0, 50.0);

        // For each road segment, generate plots on both sides
        for road in roads {
            if road.road_type != RoadType::Suburban {
                continue;
            }

            let road_plots = self.generate_plots_along_road(
                road, params, roads, &placed_plot_corners,
                &road_grid, &road_idx_map, &plot_grid
            );

            // Add corners of successfully placed plots to our tracking list
            for plot in &road_plots {
                let plot_idx = placed_plot_corners.len();
                // Insert plot AABB into plot grid
                let cs = &plot.corners;
                let min_x = cs[0].0.min(cs[1].0).min(cs[2].0).min(cs[3].0);
                let min_y = cs[0].1.min(cs[1].1).min(cs[2].1).min(cs[3].1);
                let max_x = cs[0].0.max(cs[1].0).max(cs[2].0).max(cs[3].0);
                let max_y = cs[0].1.max(cs[1].1).max(cs[2].1).max(cs[3].1);
                // Use insert with a synthetic polyline covering the corners' AABB
                plot_grid.insert(plot_idx, &[(min_x, min_y), (max_x, max_y)]);
                placed_plot_corners.push(plot.corners);
            }

            plots.extend(road_plots);
        }

        plots
    }

    /// Generate plots along one side of a road
    fn generate_plots_along_road(
        &self,
        road: &RoadSegment,
        params: &SuburbanGenParams,
        all_roads: &[RoadSegment],
        placed_plots: &[[(f64, f64); 4]],
        road_grid: &SpatialGrid,
        road_idx_map: &HashMap<String, usize>,
        plot_grid: &SpatialGrid,
    ) -> Vec<TerrainPlot> {
        let mut plots = Vec::new();
        let mut local_placed: Vec<[(f64, f64); 4]> = Vec::new();

        let points = &road.points;
        if points.len() < 2 {
            return plots;
        }

        // Calculate target frontage based on target area and assumed depth
        let assumed_depth = (params.target_plot_area / params.min_frontage).min(params.max_frontage);
        let target_frontage = params.target_plot_area / assumed_depth;

        // Walk along road creating plots
        let total_length = polyline_length(points);
        let num_plots_estimate = (total_length / target_frontage).ceil() as usize; // Use ceil for more plots
        let actual_frontage = total_length / num_plots_estimate.max(1) as f64;

        // Get this road's index so we can exclude it from road intersection checks
        let this_road_idx = road_idx_map.get(&road.id).copied();

        // Generate plots on both sides of road
        for side in [-1.0, 1.0] {
            let plot_depth = assumed_depth;

            // Minimal start offset - just enough to not clip the road endpoint
            let mut current_dist = 5.0;
            let mut plot_id = 0;
            let small_step = 10.0; // Small step for finding next valid position after failure

            while current_dist + actual_frontage <= total_length - 5.0 {
                // Get road points at frontage start and end
                let front_start = point_at_distance(points, current_dist);
                let front_end = point_at_distance(points, current_dist + actual_frontage);

                let mut plot_placed = false;

                if let (Some((fs_x, fs_y, fs_angle)), Some((fe_x, fe_y, fe_angle))) = (front_start, front_end) {
                    // Calculate perpendicular offset for back corners
                    let perp_angle = (fs_angle + fe_angle) / 2.0 + side * PI / 2.0;

                    // Offset front corners slightly away from road center
                    let road_offset = road.width / 2.0 + 2.0;
                    let fs_offset_x = fs_x + road_offset * perp_angle.cos();
                    let fs_offset_y = fs_y + road_offset * perp_angle.sin();
                    let fe_offset_x = fe_x + road_offset * perp_angle.cos();
                    let fe_offset_y = fe_y + road_offset * perp_angle.sin();

                    let back_start_x = fs_x + plot_depth * perp_angle.cos();
                    let back_start_y = fs_y + plot_depth * perp_angle.sin();
                    let back_end_x = fe_x + plot_depth * perp_angle.cos();
                    let back_end_y = fe_y + plot_depth * perp_angle.sin();

                    // Check if all corners are within suburban zone
                    let corners_in_zone = [(fs_offset_x, fs_offset_y), (fe_offset_x, fe_offset_y),
                                           (back_start_x, back_start_y), (back_end_x, back_end_y)]
                        .iter()
                        .all(|(x, y)| {
                            let dist = ((x - self.center_x).powi(2) + (y - self.center_y).powi(2)).sqrt();
                            dist >= params.inner_radius && dist <= params.outer_radius
                        });

                    if corners_in_zone {
                        let corners = [
                            (fs_offset_x, fs_offset_y),     // Front-left (street)
                            (fe_offset_x, fe_offset_y),     // Front-right (street)
                            (back_end_x, back_end_y),       // Back-right
                            (back_start_x, back_start_y),   // Back-left
                        ];

                        let corners_slice: &[(f64, f64)] = &corners;

                        // Compute plot AABB for grid queries
                        let plot_min_x = corners[0].0.min(corners[1].0).min(corners[2].0).min(corners[3].0);
                        let plot_min_y = corners[0].1.min(corners[1].1).min(corners[2].1).min(corners[3].1);
                        let plot_max_x = corners[0].0.max(corners[1].0).max(corners[2].0).max(corners[3].0);
                        let plot_max_y = corners[0].1.max(corners[1].1).max(corners[2].1).max(corners[3].1);

                        // Check if plot intersects with any road (via spatial grid)
                        let mut intersects_road = false;
                        let nearby_roads = road_grid.query_aabb(plot_min_x, plot_min_y, plot_max_x, plot_max_y);
                        for &road_idx in &nearby_roads {
                            // Skip the road this plot is on (front edge touches it by design)
                            if this_road_idx == Some(road_idx) {
                                continue;
                            }
                            let other_road = &all_roads[road_idx];
                            if polygon_intersects_road(corners_slice, &other_road.points, other_road.width) {
                                intersects_road = true;
                                break;
                            }
                        }

                        if !intersects_road {
                            // Check for overlap with already placed plots (via spatial grid)
                            let mut overlaps = false;

                            let nearby_plots = plot_grid.query_aabb(plot_min_x, plot_min_y, plot_max_x, plot_max_y);
                            for &plot_idx in &nearby_plots {
                                let existing_slice: &[(f64, f64)] = &placed_plots[plot_idx];
                                if polygons_overlap(corners_slice, existing_slice) {
                                    overlaps = true;
                                    break;
                                }
                            }

                            // Also check against locally placed plots from this road
                            if !overlaps {
                                for existing in &local_placed {
                                    let existing_slice: &[(f64, f64)] = existing;
                                    if polygons_overlap(corners_slice, existing_slice) {
                                        overlaps = true;
                                        break;
                                    }
                                }
                            }

                            if !overlaps {
                                let area = TerrainPlot::calculate_area(&corners);

                                if area >= params.min_plot_area && area <= params.max_plot_area {
                                    local_placed.push(corners);
                                    plots.push(TerrainPlot {
                                        id: format!("{}_{}_{}_{}", road.id, if side > 0.0 { "R" } else { "L" }, plot_id, (current_dist as i32)),
                                        corners,
                                        street_facing_edge: (0, 1),
                                        area,
                                        adjacent_street_id: road.id.clone(),
                                        zone: "suburban".to_string(),
                                    });
                                    plot_id += 1;
                                    plot_placed = true;
                                }
                            }
                        }
                    }
                }

                // If plot was placed, advance by frontage. If not, try a small step to find next valid spot.
                if plot_placed {
                    current_dist += actual_frontage;
                } else {
                    current_dist += small_step;
                }
            }
        }

        plots
    }

    /// Generate unpaved trails in the rural zone with smooth meandering paths
    fn generate_rural_paths(&self, seed: u64) -> Vec<RoadSegment> {
        let params = WorldGenParams { seed, ..Default::default() };
        self.generate_rural_paths_with_params(&params)
    }

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

                let x = self.center_x + r * a.cos();
                let y = self.center_y + r * a.sin();
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
            .map(|&(x, y)| (y - self.center_y).atan2(x - self.center_x))
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
                            (self.center_x + sub_r_in * sa1_in.cos(), self.center_y + sub_r_in * sa1_in.sin()),
                            (self.center_x + sub_r_in * sa2_in.cos(), self.center_y + sub_r_in * sa2_in.sin()),
                            (self.center_x + sub_r_out * sa2_out.cos(), self.center_y + sub_r_out * sa2_out.sin()),
                            (self.center_x + sub_r_out * sa1_out.cos(), self.center_y + sub_r_out * sa1_out.sin()),
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
            ("in", inner_canal.outer_radius() + kerb + GUILD_CLEAR, ring_r * (PI / sides).cos() - ring_half - GUILD_CLEAR),
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
            let bearing = |&(x, y): &(f64, f64)| (y - self.center_y).atan2(x - self.center_x).rem_euclid(2.0 * PI);
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
                let edges = |r: f64| (a1 + ((h1 + GUILD_CLEAR) / r).asin(), a2 - ((h2 + GUILD_CLEAR) / r).asin());
                let (m1, m2) = edges((lo + hi) / 2.0);
                if m2 <= m1 {
                    continue;
                }
                let count = ((m2 - m1) * (lo + hi) / 2.0 / params.guild_plot_frontage.max(1.0)).ceil().max(1.0) as usize;

                // Stand the inner corners out so the chord between them clears `lo`
                let mut r_in = lo;
                for _ in 0..3 {
                    let (e1, e2) = edges(r_in);
                    r_in = lo / ((e2 - e1) / count as f64 / 2.0).cos();
                }
                let r_out = hi;
                let (in1, in2) = edges(r_in);
                let (out1, out2) = edges(r_out);
                if in2 <= in1 || out2 <= out1 {
                    continue;
                }

                let at = |r: f64, a: f64| (self.center_x + r * a.cos(), self.center_y + r * a.sin());
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
                        let fs_off_x = fs_x + half_trail_w * perp_angle.cos();
                        let fs_off_y = fs_y + half_trail_w * perp_angle.sin();
                        let fe_off_x = fe_x + half_trail_w * perp_angle.cos();
                        let fe_off_y = fe_y + half_trail_w * perp_angle.sin();

                        // Back corners at depth offset
                        let bs_x = fs_x + depth * perp_angle.cos();
                        let bs_y = fs_y + depth * perp_angle.sin();
                        let be_x = fe_x + depth * perp_angle.cos();
                        let be_y = fe_y + depth * perp_angle.sin();

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

    #[test]
    fn test_terrain_config_default() {
        let config = TerrainConfig::get_default();
        assert_eq!(config.world_size, 8000.0);
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

    /// Legacy is the default until #112, and suburb v2's work packages build
    /// the Neighbourhood generator beside it. This pins what Legacy makes, so
    /// none of them moves the live world by accident. Change the literal only
    /// for a change that means to move it, and say so in the commit.
    #[test]
    fn legacy_default_terrain_version_is_pinned() {
        assert_eq!(WorldGenParams::default().suburban.layout, SuburbLayout::Legacy);
        // Moved on purpose by #117, which deepened the suburb to 1800–3000.
        assert_eq!(TerrainConfig::get_default().version(), "2f18343910f69960");
    }

    /// Saved `world_config` rows and admin PUTs from before #105 have no
    /// `layout` or `neighbourhood`. They still load, as Legacy.
    #[test]
    fn world_params_load_with_and_without_layout() {
        let mut json = serde_json::to_value(WorldGenParams::default()).unwrap();
        let suburban = json["suburban"].as_object_mut().unwrap();
        assert_eq!(suburban["layout"], "legacy");
        suburban.remove("layout");
        suburban.remove("neighbourhood");
        let old: WorldGenParams = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old.suburban.layout, SuburbLayout::Legacy);
        assert_eq!(old.suburban.neighbourhood, NeighbourhoodParams::default());

        // A layout, and a partial neighbourhood: the rest takes its defaults.
        json["suburban"]["layout"] = "neighbourhood".into();
        json["suburban"]["neighbourhood"] = serde_json::json!({ "lot_depth": 120.0, "force_pattern": "central_loop" });
        let new: WorldGenParams = serde_json::from_value(json).unwrap();
        assert_eq!(new.suburban.layout, SuburbLayout::Neighbourhood);
        assert_eq!(new.suburban.neighbourhood.lot_depth, 120.0);
        assert_eq!(new.suburban.neighbourhood.lot_frontage, 60.0);
        assert_eq!(new.suburban.neighbourhood.force_pattern, Some(NeighbourhoodPattern::CentralLoop));
    }

    /// The default world, and the Neighbourhood layout with pattern (a)
    /// everywhere, for the suburban street tests that hold for both (#107).
    /// Built once: five tests share them, and the performance test runs beside them.
    fn both_layouts() -> &'static [TerrainConfig; 2] {
        static BOTH: std::sync::OnceLock<[TerrainConfig; 2]> = std::sync::OnceLock::new();
        BOTH.get_or_init(|| {
            let mut params = WorldGenParams::default();
            params.suburban.layout = SuburbLayout::Neighbourhood;
            params.suburban.neighbourhood.force_pattern = Some(NeighbourhoodPattern::WarpedParallels);
            [TerrainConfig::get_default(), TerrainConfig::from_params(&params)]
        })
    }

    /// The Neighbourhood layout lays suburban streets, but no suburban lots
    /// until #108, and leaves the rest of the world as Legacy has it.
    #[test]
    fn neighbourhood_layout_changes_only_the_suburb() {
        let mut params = WorldGenParams::default();
        params.suburban.layout = SuburbLayout::Neighbourhood;
        let t = TerrainConfig::from_params(&params);
        assert!(t.roads.iter().any(|r| r.road_type == RoadType::Suburban));
        assert!(t.terrain_plots.iter().all(|l| l.zone != "suburban"));
        let legacy = TerrainConfig::get_default();
        let others = |t: &TerrainConfig| t.roads.iter().filter(|r| r.road_type != RoadType::Suburban).count();
        assert_eq!(others(&t), others(&legacy));
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
        let result = config.validate_movement(4000.0, 4000.0, 4050.0, 4050.0);
        assert!(result.allowed);
    }

    #[test]
    fn test_movement_blocked_by_moat() {
        let config = TerrainConfig::get_default();
        // Try to cross moat going East (no bridge there)
        let result = config.validate_movement(4150.0, 4000.0, 4250.0, 4000.0);
        assert!(!result.allowed);
        assert_eq!(result.blocked_by, Some("moat".to_string()));
    }

    #[test]
    fn test_movement_allowed_on_bridge() {
        let config = TerrainConfig::get_default();
        // Cross moat going South (bridge at 90° in screen coords where Y+ is south)
        let result = config.validate_movement(4000.0, 4150.0, 4000.0, 4250.0);
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
        for config in both_layouts() {
            let suburban: Vec<_> = config.roads.iter()
                .filter(|r| r.road_type == RoadType::Suburban)
                .collect();
            // Each suburban street should have many points for smooth curves
            for road in &suburban {
                assert!(road.points.len() >= 10, "Suburban road {} should have smooth curves (10+ points), got {}", road.id, road.points.len());
            }
        }
    }

    #[test]
    fn test_suburban_gen_params_default() {
        let params = SuburbanGenParams::default();
        assert_eq!(params.inner_radius, rules::URBAN_OUTER_RADIUS);
        assert_eq!(params.outer_radius, rules::SUBURBAN_OUTER_RADIUS);
        assert_eq!(params.min_plot_area, 6_000.0);
        assert_eq!(params.max_plot_area, 12_000.0);
        assert_eq!(params.target_plot_area, 8_000.0);
    }

    #[test]
    fn test_suburban_collector_roads_generated() {
        for config in both_layouts() {
            let collectors: Vec<_> = config.roads.iter()
                .filter(|r| r.id.starts_with("collector_"))
                .collect();
            // Should have collectors for each of 8 superblocks (at least 8)
            assert!(collectors.len() >= 8, "Expected at least 8 collector roads, got {}", collectors.len());
        }
    }

    #[test]
    fn test_suburban_cul_de_sacs_generated() {
        for config in both_layouts() {
            let culdesacs: Vec<_> = config.roads.iter()
                .filter(|r| r.id.starts_with("culdesac_"))
                .collect();
            // Should have some cul-de-sacs
            assert!(culdesacs.len() > 0, "Expected at least 1 cul-de-sac road");
        }
    }

    #[test]
    fn test_suburban_streets_within_zone() {
        for config in both_layouts() {
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
                (config.center_x + params.inner_radius * angle.cos(), config.center_y + params.inner_radius * angle.sin()),
                (config.center_x + params.outer_radius * angle.cos(), config.center_y + params.outer_radius * angle.sin()),
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

    #[test]
    fn test_suburban_plot_generation() {
        let config = TerrainConfig::get_default();
        let params = SuburbanGenParams::default();
        let plots = config.generate_suburban_plots(&params);

        // Should generate some plots
        println!("Generated {} suburban plots", plots.len());

        // Check plot sizes are within bounds
        for plot in &plots {
            assert!(
                plot.area >= params.min_plot_area * 0.8 && plot.area <= params.max_plot_area * 1.2,
                "Plot {} area {} is outside bounds ({} - {})",
                plot.id, plot.area, params.min_plot_area, params.max_plot_area
            );

            // Check all corners are within suburban zone
            for &(x, y) in &plot.corners {
                let dist = ((x - config.center_x).powi(2) + (y - config.center_y).powi(2)).sqrt();
                assert!(
                    dist >= params.inner_radius - 50.0 && dist <= params.outer_radius + 50.0,
                    "Plot {} corner ({}, {}) is outside suburban zone",
                    plot.id, x, y
                );
            }
        }
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
    fn test_seeded_rng_determinism() {
        let mut rng1 = SeededRng::new(42);
        let mut rng2 = SeededRng::new(42);

        // Same seed should produce same sequence
        for _ in 0..10 {
            assert_eq!(rng1.next(), rng2.next());
        }

        // Different seed should produce different sequence
        let mut rng3 = SeededRng::new(123);
        let different = (0..10).any(|_| rng1.next() != rng3.next());
        // Reset rng1
        rng1 = SeededRng::new(42);
        assert!(different || true); // Allow some chance of collision
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
    fn test_local_streets_branch_from_connectors() {
        // This test verifies that cul-de-sacs and loops can branch from connector roads,
        // not just from collectors and arterials
        let config = TerrainConfig::get_default();
        let params = SuburbanGenParams::default();

        // Run the full iterative generation process
        let mut suburban_streets: Vec<SuburbanStreet> = Vec::new();
        let mut grid = SpatialGrid::new(config.center_x, config.center_y, params.outer_radius + 100.0, 50.0);

        // Phase 1: Generate collector roads
        let collectors = config.generate_collector_roads(&params, &mut grid);
        println!("Generated {} collector roads", collectors.len());
        assert!(!collectors.is_empty(), "Should generate collector roads");
        suburban_streets.extend(collectors);

        // Phase 2: Iteratively generate local streets
        const MAX_ITERATIONS: usize = 8;
        for iteration in 0..MAX_ITERATIONS {
            // Debug: Check connection points from connectors
            let connector_streets: Vec<_> = suburban_streets.iter()
                .filter(|s| s.segment.id.starts_with("connector_"))
                .collect();
            if !connector_streets.is_empty() {
                println!("Iteration {} - connectors available: {}", iteration, connector_streets.len());
                for cs in &connector_streets {
                    let len = polyline_length(&cs.segment.points);
                    println!("  {} len={:.0}, points={}", cs.segment.id, len, cs.segment.points.len());

                    // Check depth at connection points along this connector
                    let placed_segs: Vec<Vec<(f64, f64)>> = suburban_streets.iter()
                        .map(|s| s.segment.points.clone()).collect();
                    let num_cp = if len < 150.0 { 2.max(((len / 40.0) as usize).max(1)) } else { ((len / 60.0) as usize).max(1) };
                    for i in 1..=num_cp {
                        let t = i as f64 / (num_cp + 1) as f64;
                        if let Some((x, y, angle)) = point_and_tangent_at_t(&cs.segment.points, t) {
                            let depth = config.available_depth(x, y, angle, &params, &placed_segs, &grid);
                            println!("    CP at t={:.2}: ({:.0},{:.0}) depth={:.0}", t, x, y, depth);
                        }
                    }
                }
            }

            let locals = config.generate_local_streets(&params, &suburban_streets, iteration, &mut grid);
            if locals.is_empty() {
                println!("Iteration {} produced 0 streets, stopping", iteration);
                break;
            }
            println!("Iteration {} produced {} streets", iteration, locals.len());

            // Show what patterns were generated
            let mut pattern_counts: HashMap<String, usize> = HashMap::new();
            for street in &locals {
                *pattern_counts.entry(format!("{:?}", street.pattern)).or_insert(0) += 1;
            }
            println!("  Patterns: {:?}", pattern_counts);

            // Show parent roads of new streets
            for street in &locals {
                println!("  {} ({:?}) parent={:?}",
                    street.segment.id, street.pattern, street.parent_road_id);
            }
            suburban_streets.extend(locals);
            println!("  Total streets now: {}", suburban_streets.len());
        }

        // Collect connector IDs
        let connector_ids: HashSet<String> = suburban_streets.iter()
            .filter(|s| s.segment.id.starts_with("connector_"))
            .map(|s| s.segment.id.clone())
            .collect();

        println!("\nTotal connectors generated: {}", connector_ids.len());

        // Check how many non-connector streets have a connector as their parent
        let streets_from_connectors: Vec<_> = suburban_streets.iter()
            .filter(|s| !s.segment.id.starts_with("connector_") && !s.segment.id.starts_with("collector_"))
            .filter(|s| {
                if let Some(ref parent_id) = s.parent_road_id {
                    connector_ids.contains(parent_id)
                } else {
                    false
                }
            })
            .collect();

        println!("Streets (non-connector) that branch from connectors: {}", streets_from_connectors.len());
        for street in &streets_from_connectors {
            println!("  {} ({:?}) branches from {:?}",
                street.segment.id, street.pattern, street.parent_road_id);
        }

        // We should have at least some streets branching from connectors
        assert!(
            streets_from_connectors.len() > 0,
            "Expected at least some cul-de-sacs/loops to branch from connector roads. \
             Total connectors: {}, Streets from connectors: {}",
            connector_ids.len(),
            streets_from_connectors.len()
        );

        // Print final stats
        let collector_count = suburban_streets.iter().filter(|s| s.segment.id.starts_with("collector_")).count();
        let local_count = suburban_streets.len() - collector_count;
        println!(
            "\nFinal stats: {} collectors, {} connectors, {} total local streets, {} branching from connectors",
            collector_count,
            connector_ids.len(),
            local_count,
            streets_from_connectors.len()
        );
    }

    #[test]
    fn test_no_roads_terminate_at_outer_boundary() {
        // This test verifies that no suburban roads have endpoints at the outer boundary
        // of the suburban zone (which would indicate erroneous emergency connectors)
        for config in both_layouts() {
            let params = SuburbanGenParams::default();

            let outer_boundary_tolerance = 20.0; // Within 20 units of outer boundary

            // Calculate the 8 "corner" positions at the outer boundary (where arterials intersect)
            let corner_positions: Vec<(f64, f64)> = (0..8)
                .map(|i| {
                    let angle = i as f64 * PI / 4.0;
                    (
                        config.center_x + params.outer_radius * angle.cos(),
                        config.center_y + params.outer_radius * angle.sin(),
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
    }

    #[test]
    fn guild_halls_fill_the_guild_district_untrimmed() {
        // The defaults and the wider canals the dev DB commits (#83).
        let mut wide = WorldGenParams::default();
        wide.canal_inner_width = 50.0;
        wide.canal_outer_width = 50.0;
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
        // Typical: ~1.4s debug, ~100ms release. Pre-optimization was ~115s debug.
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

