use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub use capitol_protocol::rest::InventoryItem;

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

impl Item {
    /// The client-facing view of this item
    pub fn to_inventory_item(&self, item_type_name: &str) -> InventoryItem {
        InventoryItem {
            id: self.id,
            item_type: self.item_type.clone(),
            name: item_type_name.to_string(),
            quality: self.quality,
            quality_grade: quality_to_grade(self.quality),
            quantity: self.quantity,
        }
    }
}

/// Convert quality (0-100) to letter grade
pub fn quality_to_grade(quality: i32) -> String {
    capitol_protocol::rules::quality_grade(quality).to_string()
}
