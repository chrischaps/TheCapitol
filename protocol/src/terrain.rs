//! The `GET /terrain` response: water rings with their bridges, the road network,
//! the terrain-level plots, the suburb's parks and the params of the land's height.
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
    /// The suburb's parks: open ground no lot took, big enough to use (#129).
    /// Defaults to none, so payloads from before them still parse.
    #[serde(default)]
    pub parks: Vec<TerrainPark>,
    /// What shapes the height of the land (W2, #195). Clients compute the
    /// height from these with `land::height_at`; the heights never go on the
    /// wire. Defaults to the default land, so payloads from before it parse.
    #[serde(default)]
    pub land: crate::land::LandParams,
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
    /// Zone this plot belongs to: "guild_district", "urban", "suburban", or "rural"
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

/// A park: a piece of a suburban neighbourhood's open ground that no lot
/// took, big enough to use (the backend's `suburb::parks`). No lot overlaps
/// one, so nothing can be claimed or built on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TerrainPark {
    /// `park_{sector}_{neighbourhood}_{k}`
    pub id: String,
    /// Its outline as closed rings (the last point doesn't repeat the
    /// first), filled even-odd: a ring inside another is a hole. A park can
    /// have more than one outer ring.
    pub rings: Vec<Vec<(f64, f64)>>,
    /// Its area in square units: the rings', holes taken out.
    pub area: f64,
}

impl TerrainPark {
    /// Whether `(x, y)` is in the park (even-odd over all its rings).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let mut inside = false;
        for ring in &self.rings {
            let n = ring.len();
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// The signed shoelace area of a ring: positive one way round, negative
    /// the other.
    pub fn signed_area(ring: &[(f64, f64)]) -> f64 {
        let n = ring.len();
        (0..n).map(|i| ring[i].0 * ring[(i + 1) % n].1 - ring[(i + 1) % n].0 * ring[i].1).sum::<f64>() / 2.0
    }
}

/// Whether a point is inside a lot's quad, or on its edge. The quad may wind
/// either way. A lot is convex, but this holds for any simple quad.
pub fn quad_contains(corners: &[(f64, f64); 4], x: f64, y: f64) -> bool {
    const EDGE: f64 = 1e-9;
    let mut inside = false;
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        // On the edge counts as on the plot.
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let cross = dx * (y - a.1) - dy * (x - a.0);
        let within = (x - a.0) * (x - b.0) <= EDGE && (y - a.1) * (y - b.1) <= EDGE;
        if cross.abs() <= EDGE * (dx.abs() + dy.abs()).max(1.0) && within {
            return true;
        }
        // Even-odd crossing test.
        if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * dx / dy {
            inside = !inside;
        }
    }
    inside
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
    fn quad_contains_inside_edge_and_outside() {
        // A rotated square (a diamond) round (10, 10), wound anticlockwise.
        let d = [(10.0, 0.0), (20.0, 10.0), (10.0, 20.0), (0.0, 10.0)];
        assert!(quad_contains(&d, 10.0, 10.0));
        assert!(quad_contains(&d, 15.0, 5.0), "on an edge");
        assert!(quad_contains(&d, 10.0, 0.0), "on a corner");
        assert!(!quad_contains(&d, 2.0, 2.0), "inside the box, outside the lot");
        assert!(!quad_contains(&d, 21.0, 10.0));
        let mut cw = d;
        cw.reverse();
        assert!(quad_contains(&cw, 10.0, 10.0) && !quad_contains(&cw, 2.0, 2.0));
    }

    #[test]
    fn a_park_with_a_hole() {
        let park = TerrainPark {
            id: "park_0_0_0".into(),
            rings: vec![
                vec![(0.0, 0.0), (30.0, 0.0), (30.0, 30.0), (0.0, 30.0)],
                vec![(10.0, 10.0), (10.0, 20.0), (20.0, 20.0), (20.0, 10.0)],
            ],
            area: 800.0,
        };
        assert!(park.contains(5.0, 5.0));
        assert!(!park.contains(15.0, 15.0), "in the hole");
        assert!(!park.contains(35.0, 5.0));
        let total: f64 = park.rings.iter().map(|r| TerrainPark::signed_area(r)).sum();
        assert_eq!(total.abs(), park.area);
    }

    #[test]
    fn shoelace_area() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert_eq!(TerrainPlot::calculate_area(&square), 100.0);
    }
}
