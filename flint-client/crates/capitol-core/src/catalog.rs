//! Static game data the server has no endpoint for yet: item types, station
//! types, resource types and container types. Copied from the seed rows in
//! `backend/migrations/` (m2 resources, m3 crafting, slot inventory, stations).
//! When the backend grows endpoints for these, fetch them instead.
//!
//! IDs and category strings are the wire values. The station-to-kit mapping
//! lives in [`crate::protocol::rules`] because the backend uses it too.

/// An item type (`item_types`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemType {
    pub id: &'static str,
    pub name: &'static str,
    /// `raw_material`, `processed_material` or `placeable`
    pub category: &'static str,
    /// Stackable items merge into one slot; kits don't.
    pub stackable: bool,
}

pub const ITEM_TYPES: &[ItemType] = &[
    ItemType {
        id: "fiber",
        name: "Plant Fiber",
        category: "raw_material",
        stackable: true,
    },
    ItemType {
        id: "rope",
        name: "Rope",
        category: "processed_material",
        stackable: true,
    },
    ItemType {
        id: "workbench_kit",
        name: "Workbench Kit",
        category: "placeable",
        stackable: false,
    },
    ItemType {
        id: "forge_kit",
        name: "Forge Kit",
        category: "placeable",
        stackable: false,
    },
    ItemType {
        id: "chest_kit",
        name: "Storage Chest Kit",
        category: "placeable",
        stackable: false,
    },
];

pub fn item_type(id: &str) -> Option<&'static ItemType> {
    ITEM_TYPES.iter().find(|t| t.id == id)
}

/// A station type (`station_types`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StationType {
    pub id: &'static str,
    pub name: &'static str,
    /// `crafting` or `storage`
    pub category: &'static str,
    pub slot_count: i32,
    pub layout_columns: i32,
    pub icon: &'static str,
    pub description: &'static str,
    /// Every seeded type uses the column default, 75.
    pub interaction_range: f32,
}

pub const STATION_TYPES: &[StationType] = &[
    StationType {
        id: "workbench",
        name: "Workbench",
        category: "crafting",
        slot_count: 16,
        layout_columns: 4,
        icon: "workbench",
        description: "Craft wooden items and tools",
        interaction_range: 75.0,
    },
    StationType {
        id: "forge",
        name: "Forge",
        category: "crafting",
        slot_count: 12,
        layout_columns: 4,
        icon: "forge",
        description: "Smelt ores and craft metal equipment",
        interaction_range: 75.0,
    },
    StationType {
        id: "storage_chest",
        name: "Storage Chest",
        category: "storage",
        slot_count: 16,
        layout_columns: 4,
        icon: "chest",
        description: "Store items safely",
        interaction_range: 75.0,
    },
];

pub fn station_type(id: &str) -> Option<&'static StationType> {
    STATION_TYPES.iter().find(|t| t.id == id)
}

/// A resource type (`resource_types`): what a node yields and how fast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceType {
    pub id: &'static str,
    pub name: &'static str,
    /// The item type an extraction yields.
    pub yields_item: &'static str,
    pub yield_quantity: (i32, i32),
    pub extraction_duration_ticks: u32,
    pub regeneration_ticks: u32,
}

pub const RESOURCE_TYPES: &[ResourceType] = &[ResourceType {
    id: "grass",
    name: "Wild Grass",
    yields_item: "fiber",
    yield_quantity: (1, 3),
    extraction_duration_ticks: 30,
    regeneration_ticks: 600,
}];

pub fn resource_type(id: &str) -> Option<&'static ResourceType> {
    RESOURCE_TYPES.iter().find(|t| t.id == id)
}

/// Container type IDs (`container_types`). `/inventory/containers` returns one
/// of each of the first three per player.
pub mod container_type {
    pub const PLAYER_INVENTORY: &str = "player_inventory";
    pub const CRAFTING_INPUT: &str = "crafting_input";
    pub const CRAFTING_OUTPUT: &str = "crafting_output";
    pub const STATION_INVENTORY: &str = "station_inventory";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::rules;

    #[test]
    fn every_kit_places_a_known_station() {
        for item in ITEM_TYPES.iter().filter(|t| t.category == "placeable") {
            let station = rules::station_type_for_kit(item.id).expect(item.id);
            assert!(station_type(station).is_some(), "{station}");
            assert!(!item.stackable);
        }
    }

    #[test]
    fn resources_yield_known_items() {
        for resource in RESOURCE_TYPES {
            assert!(item_type(resource.yields_item).is_some());
        }
        assert_eq!(resource_type("grass").unwrap().name, "Wild Grass");
    }
}
