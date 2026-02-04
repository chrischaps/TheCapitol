use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// Static item type definition (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ItemType {
    pub id: String,
    pub name: String,
    pub category: String,
    pub stackable: bool,
    pub weight: f32,
}

/// Item instance in a player's inventory (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Item {
    pub id: Uuid,
    pub item_type: String,
    pub quality: i32,
    pub quantity: i32,
    pub owner_id: Option<Uuid>,
    pub container_id: Option<Uuid>,
    pub slot_index: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// Client-facing inventory item
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InventoryItem {
    /// Unique item instance ID
    pub id: Uuid,
    /// Item type identifier (e.g., "fiber", "rope")
    pub item_type: String,
    /// Display name of the item
    pub name: String,
    /// Quality value (0-100)
    pub quality: i32,
    /// Quality grade letter (A, B, C, D, F)
    pub quality_grade: String,
    /// Stack quantity
    pub quantity: i32,
}

impl InventoryItem {
    pub fn from_item(item: &Item, item_type_name: &str) -> Self {
        Self {
            id: item.id,
            item_type: item.item_type.clone(),
            name: item_type_name.to_string(),
            quality: item.quality,
            quality_grade: quality_to_grade(item.quality),
            quantity: item.quantity,
        }
    }
}

/// Convert quality (0-100) to letter grade
pub fn quality_to_grade(quality: i32) -> String {
    match quality {
        90..=100 => "A".to_string(),
        75..=89 => "B".to_string(),
        55..=74 => "C".to_string(),
        35..=54 => "D".to_string(),
        _ => "F".to_string(),
    }
}
