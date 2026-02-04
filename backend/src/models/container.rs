use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use super::item::quality_to_grade;

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

/// Item in a specific slot (for API responses)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SlotItem {
    /// Unique item instance ID
    pub item_id: Uuid,
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Quality value (0-100)
    pub quality: i32,
    /// Quality grade letter (A, B, C, D, F)
    pub quality_grade: String,
    /// Stack quantity
    pub quantity: i32,
    /// Slot index within the container
    pub slot_index: i32,
}

impl SlotItem {
    pub fn new(
        item_id: Uuid,
        item_type: String,
        item_name: String,
        quality: i32,
        quantity: i32,
        slot_index: i32,
    ) -> Self {
        Self {
            item_id,
            item_type,
            item_name,
            quality,
            quality_grade: quality_to_grade(quality),
            quantity,
            slot_index,
        }
    }
}

/// Container state with all its slots (for API responses)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerState {
    pub id: Uuid,
    pub container_type: String,
    pub slot_count: i32,
    pub layout_columns: i32,
    pub slots: HashMap<i32, SlotItem>,
}

impl ContainerState {
    pub fn new(container: &Container, container_type: &ContainerType) -> Self {
        Self {
            id: container.id,
            container_type: container.container_type.clone(),
            slot_count: container_type.slot_count,
            layout_columns: container_type.layout_columns,
            slots: HashMap::new(),
        }
    }

    /// Add an item to a slot
    pub fn set_slot(&mut self, slot_index: i32, item: SlotItem) {
        self.slots.insert(slot_index, item);
    }

    /// Get an item from a slot
    pub fn get_slot(&self, slot_index: i32) -> Option<&SlotItem> {
        self.slots.get(&slot_index)
    }

    /// Find first empty slot
    pub fn find_empty_slot(&self) -> Option<i32> {
        for i in 0..self.slot_count {
            if !self.slots.contains_key(&i) {
                return Some(i);
            }
        }
        None
    }

    /// Find slot containing item type
    pub fn find_slot_with_type(&self, item_type: &str) -> Option<i32> {
        for (slot_idx, item) in &self.slots {
            if item.item_type == item_type {
                return Some(*slot_idx);
            }
        }
        None
    }
}

/// Slot update for events (what changed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotUpdate {
    pub slot_index: i32,
    pub item: Option<SlotItem>, // None means slot is now empty
}

/// Result of a move operation
#[derive(Debug, Clone)]
pub enum MoveResult {
    Moved {
        item_id: Uuid,
        to_slot: i32,
    },
    Swapped {
        item1_id: Uuid,
        item1_to_slot: i32,
        item2_id: Uuid,
        item2_to_slot: i32,
    },
    Merged {
        source_item_id: Uuid,
        target_item_id: Uuid,
        new_quantity: i32,
        new_quality: i32,
    },
}
