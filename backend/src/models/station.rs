use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// Static station type definition (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct StationType {
    pub id: String,
    pub name: String,
    pub category: String,       // "crafting" or "storage"
    pub slot_count: i32,
    pub layout_columns: i32,
    pub interaction_range: f32,
    pub icon: Option<String>,
    pub description: Option<String>,
}

/// Station instance in the world (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Station {
    pub id: Uuid,
    pub station_type: String,
    pub owner_id: Uuid,
    pub position_x: f32,
    pub position_y: f32,
    pub plot_id: Option<Uuid>,
    pub placed_at: DateTime<Utc>,
}

/// Station runtime state for game engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationState {
    pub id: Uuid,
    pub station_type: String,
    pub owner_id: Uuid,
    pub x: f64,
    pub y: f64,
    pub plot_id: Option<Uuid>,
    pub container_id: Option<Uuid>,
}

impl StationState {
    pub fn from_station(station: &Station, container_id: Option<Uuid>) -> Self {
        Self {
            id: station.id,
            station_type: station.station_type.clone(),
            owner_id: station.owner_id,
            x: station.position_x as f64,
            y: station.position_y as f64,
            plot_id: station.plot_id,
            container_id,
        }
    }
}

/// Client-facing station info
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StationInfo {
    /// Unique station instance ID
    pub id: Uuid,
    /// Station type identifier (e.g., "workbench", "forge")
    pub station_type: String,
    /// Display name of the station
    pub name: String,
    /// Station category ("crafting" or "storage")
    pub category: String,
    /// Optional icon identifier
    pub icon: Option<String>,
    /// X position in world coordinates
    pub x: f64,
    /// Y position in world coordinates
    pub y: f64,
    /// Player who owns this station
    pub owner_id: Uuid,
    /// Container ID for station inventory
    pub container_id: Option<Uuid>,
    /// Maximum interaction distance
    pub interaction_range: f32,
}

impl StationInfo {
    pub fn from_state(state: &StationState, station_type: &StationType) -> Self {
        Self {
            id: state.id,
            station_type: state.station_type.clone(),
            name: station_type.name.clone(),
            category: station_type.category.clone(),
            icon: station_type.icon.clone(),
            x: state.x,
            y: state.y,
            owner_id: state.owner_id,
            container_id: state.container_id,
            interaction_range: station_type.interaction_range,
        }
    }
}

/// Request to place a station
#[derive(Debug, Clone, Deserialize)]
pub struct PlaceStationRequest {
    pub station_type: String,
    pub x: f64,
    pub y: f64,
}

/// Response after placing a station
#[derive(Debug, Clone, Serialize)]
pub struct PlaceStationResponse {
    pub station: StationInfo,
}
