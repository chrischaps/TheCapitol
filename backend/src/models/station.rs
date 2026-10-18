use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub use capitol_protocol::rest::StationInfo;

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
