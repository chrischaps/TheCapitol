use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

use crate::models::{Bridge, RoadSegment, RoadType, SuburbanPlot, TerrainConfig, WaterRing};
use crate::state::AppState;

/// Response for terrain data endpoint
#[derive(Debug, Serialize, ToSchema)]
pub struct TerrainResponse {
    /// World size in units
    pub world_size: f64,
    /// World center X coordinate
    pub center_x: f64,
    /// World center Y coordinate
    pub center_y: f64,
    /// Concentric water features (moats, canals)
    pub water_rings: Vec<WaterRingResponse>,
    /// Road network
    pub roads: Vec<RoadResponse>,
    /// Suburban zone plots
    pub suburban_plots: Vec<SuburbanPlotResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WaterRingResponse {
    /// Ring identifier
    pub id: String,
    /// Distance from world center
    pub radius: f64,
    /// Water channel width
    pub width: f64,
    /// Bridge crossing points
    pub bridges: Vec<BridgeResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BridgeResponse {
    /// Angle in degrees from east (0=E, 90=S, 180=W, 270=N)
    pub angle_degrees: f64,
    /// Bridge width
    pub width: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoadResponse {
    /// Road segment identifier
    pub id: String,
    /// Road type: arterial, grid, suburban, trail
    pub road_type: String,
    /// Polyline points [(x,y), ...]
    pub points: Vec<(f64, f64)>,
    /// Road width
    pub width: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuburbanPlotResponse {
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
}

impl From<&TerrainConfig> for TerrainResponse {
    fn from(config: &TerrainConfig) -> Self {
        TerrainResponse {
            world_size: config.world_size,
            center_x: config.center_x,
            center_y: config.center_y,
            water_rings: config.water_rings.iter().map(|r| r.into()).collect(),
            roads: config.roads.iter().map(|r| r.into()).collect(),
            suburban_plots: config.suburban_plots.iter().map(|p| p.into()).collect(),
        }
    }
}

impl From<&WaterRing> for WaterRingResponse {
    fn from(ring: &WaterRing) -> Self {
        WaterRingResponse {
            id: ring.id.clone(),
            radius: ring.radius,
            width: ring.width,
            bridges: ring.bridges.iter().map(|b| b.into()).collect(),
        }
    }
}

impl From<&Bridge> for BridgeResponse {
    fn from(bridge: &Bridge) -> Self {
        BridgeResponse {
            angle_degrees: bridge.angle_degrees,
            width: bridge.width,
        }
    }
}

impl From<&RoadSegment> for RoadResponse {
    fn from(road: &RoadSegment) -> Self {
        RoadResponse {
            id: road.id.clone(),
            road_type: match road.road_type {
                RoadType::Arterial => "arterial".to_string(),
                RoadType::Grid => "grid".to_string(),
                RoadType::Suburban => "suburban".to_string(),
                RoadType::Trail => "trail".to_string(),
            },
            points: road.points.clone(),
            width: road.width,
        }
    }
}

impl From<&SuburbanPlot> for SuburbanPlotResponse {
    fn from(plot: &SuburbanPlot) -> Self {
        SuburbanPlotResponse {
            id: plot.id.clone(),
            corners: plot.corners,
            street_facing_edge: plot.street_facing_edge,
            area: plot.area,
            adjacent_street_id: plot.adjacent_street_id.clone(),
        }
    }
}

/// Get terrain configuration (roads, water, bridges, plots)
#[utoipa::path(
    get,
    path = "/terrain",
    responses(
        (status = 200, description = "Terrain data", body = TerrainResponse)
    ),
    tag = "Terrain"
)]
pub async fn get_terrain(State(state): State<AppState>) -> Json<TerrainResponse> {
    let game = state.game.read().await;
    let response: TerrainResponse = (&game.terrain).into();
    Json(response)
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(get_terrain))
        .with_state(state)
}
