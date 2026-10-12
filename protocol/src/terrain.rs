//! The `GET /terrain` response: water rings with their bridges, the road network,
//! and the terrain-level plots.
//!
//! The backend's generator builds these types directly, so the response is the
//! generated terrain as-is.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// Response for terrain data endpoint
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TerrainResponse {
    /// World size in units
    pub world_size: f64,
    /// World center X coordinate
    pub center_x: f64,
    /// World center Y coordinate
    pub center_y: f64,
    /// Concentric water features (moats, canals)
    pub water_rings: Vec<WaterRing>,
    /// Road network
    pub roads: Vec<RoadSegment>,
    /// Terrain-level plots (urban, suburban, rural)
    pub terrain_plots: Vec<TerrainPlot>,
    /// A stable hash of this terrain, which changes whenever the terrain does.
    /// Also sent as the response's ETag. Optional so older payloads still parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// A circular water feature (moat or canal) around the world center
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct WaterRing {
    /// Ring identifier
    pub id: String,
    /// Distance from world center
    pub radius: f64,
    /// Water channel width
    pub width: f64,
    /// Bridge crossing points
    pub bridges: Vec<Bridge>,
}

impl WaterRing {
    /// Inner edge of the water
    pub fn inner_radius(&self) -> f64 {
        self.radius - self.width / 2.0
    }

    /// Outer edge of the water
    pub fn outer_radius(&self) -> f64 {
        self.radius + self.width / 2.0
    }
}

/// A bridge crossing a water ring
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Bridge {
    /// Angle in degrees from east (0=E, 90=S, 180=W, 270=N)
    pub angle_degrees: f64,
    /// Bridge width
    pub width: f64,
}

impl Bridge {
    /// The bridge's angular half-width in degrees on a ring of this radius.
    /// (Its full width, as an arc, is twice this: the test is deliberately generous.)
    pub fn angular_half_width(&self, ring_radius: f64) -> f64 {
        if ring_radius <= 0.0 {
            return 180.0;
        }
        (self.width / ring_radius) * (180.0 / PI)
    }

    /// Whether an angle (degrees, any winding) falls on this bridge.
    pub fn contains_angle(&self, angle: f64, ring_radius: f64) -> bool {
        let half_width = self.angular_half_width(ring_radius);
        let diff = normalize_angle_diff(angle, self.angle_degrees);
        diff.abs() <= half_width
    }
}

/// The difference between two angles, normalised to [-180, 180].
pub fn normalize_angle_diff(a: f64, b: f64) -> f64 {
    let mut diff = a - b;
    while diff > 180.0 {
        diff -= 360.0;
    }
    while diff < -180.0 {
        diff += 360.0;
    }
    diff
}

/// A road segment
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RoadSegment {
    /// Road segment identifier
    pub id: String,
    /// Road type: arterial, grid, suburban, trail
    pub road_type: RoadType,
    /// Polyline points [(x,y), ...]
    pub points: Vec<(f64, f64)>,
    /// Road width
    pub width: f64,
}

/// Type of road
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RoadType {
    /// Major roads from urban to wilderness
    Arterial,
    /// Urban grid streets
    Grid,
    /// Winding suburban streets
    Suburban,
    /// Unpaved rural paths
    Trail,
}

/// A terrain-level plot with 4-sided polygon shape (urban, suburban, or rural)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TerrainPlot {
    /// Plot identifier
    pub id: String,
    /// Four corners of the plot (clockwise from street-left)
    pub corners: [(f64, f64); 4],
    /// Indices of corners forming the street-facing edge
    pub street_facing_edge: (usize, usize),
    /// Plot area in square units
    pub area: f64,
    /// ID of the adjacent street
    pub adjacent_street_id: String,
    /// Zone this plot belongs to: "urban", "suburban", or "rural"
    pub zone: String,
}

impl TerrainPlot {
    /// Area of a quadrilateral, by the shoelace formula
    pub fn calculate_area(corners: &[(f64, f64); 4]) -> f64 {
        let n = corners.len();
        let mut area = 0.0;
        for i in 0..n {
            let j = (i + 1) % n;
            area += corners[i].0 * corners[j].1;
            area -= corners[j].0 * corners[i].1;
        }
        (area / 2.0).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_contains_angle_wraps() {
        let bridge = Bridge {
            angle_degrees: 0.0,
            width: 30.0,
        };
        assert!(bridge.contains_angle(0.0, 600.0));
        assert!(bridge.contains_angle(1.0, 600.0));
        assert!(!bridge.contains_angle(5.0, 600.0));
        assert!(bridge.contains_angle(359.0, 600.0));
        assert!(bridge.contains_angle(-1.0, 600.0));
    }

    #[test]
    fn shoelace_area() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert_eq!(TerrainPlot::calculate_area(&square), 100.0);
    }
}
