use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub use capitol_protocol::rest::SlotItem;

/// Static container type definition (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ContainerType {
    pub id: String,
    pub name: String,
    pub slot_count: i32,
    pub layout_columns: i32,
}

/// Container instance (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Container {
    pub id: Uuid,
    pub container_type: String,
    pub owner_id: Option<Uuid>,
    pub station_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

