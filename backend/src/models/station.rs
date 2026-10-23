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
    /// The item that places it, and that taking it down gives back (E0.3)
    pub kit_item: Option<String>,
    /// May it stand off a plot, on dry ground no lot covers (#187)?
    pub off_plot: bool,
}

/// The columns of [`StationType`], for `SELECT {STATION_TYPE_COLUMNS} FROM station_types`.
pub const STATION_TYPE_COLUMNS: &str =
    "id, name, category, slot_count, layout_columns, interaction_range, icon, description, kit_item, off_plot";

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
    /// The quality of the kit that placed it (`stations.kit_quality`, #88):
    /// the station term of the cascade for crafts made here (E1.3)
    pub quality: i32,
    /// Inside its lot's building (V3.1)
    pub indoors: bool,
    /// Its condition as the weather last left it (V3.2), 0 to 100
    pub condition: f64,
}

/// The columns of [`Station`], for `SELECT {STATION_COLUMNS} FROM stations`.
/// The table keeps the name `kit_quality`; code calls it `quality`.
pub const STATION_COLUMNS: &str =
    "id, station_type, owner_id, position_x, position_y, plot_id, placed_at, kit_quality AS quality, indoors, condition";

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
    /// The station's quality, the station term for crafts made here
    pub quality: i32,
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
            quality: station.quality,
        }
    }
}
