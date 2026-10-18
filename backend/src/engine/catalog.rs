//! The static game data as one response, for `GET /catalog` (E0.2).
//!
//! It's built from the tables `main` loads into [`GameState`] at startup, so
//! a client sees exactly what the server plays by. Every list is sorted by id
//! and the version is a hash of the rest, so the same content always gives
//! the same version, on any machine and across restarts.

use std::collections::HashMap;

use capitol_protocol::rest::{
    CatalogContainerType, CatalogItemType, CatalogResourceType, CatalogResponse,
    CatalogStationType,
};

use super::GameState;

/// The catalog for what `game` has loaded, `version` and all.
pub fn build(game: &GameState) -> CatalogResponse {
    let mut item_types: Vec<_> = game
        .item_types
        .values()
        .map(|t| CatalogItemType {
            id: t.id.clone(),
            name: t.name.clone(),
            category: t.category.clone(),
            stackable: t.stackable,
            weight: t.weight,
        })
        .collect();
    item_types.sort_by(|a, b| a.id.cmp(&b.id));

    let mut station_types: Vec<_> = game
        .station_types
        .values()
        .map(|t| CatalogStationType {
            id: t.id.clone(),
            name: t.name.clone(),
            category: t.category.clone(),
            slot_count: t.slot_count,
            layout_columns: t.layout_columns,
            interaction_range: t.interaction_range,
            icon: t.icon.clone(),
            description: t.description.clone(),
            kit: t.kit_item.clone(),
        })
        .collect();
    station_types.sort_by(|a, b| a.id.cmp(&b.id));

    let mut resource_types: Vec<_> = game
        .resource_types
        .values()
        .map(|t| CatalogResourceType {
            id: t.id.clone(),
            name: t.name.clone(),
            yields_item: t.yields_item.clone(),
            yield_quantity_min: t.yield_quantity_min,
            yield_quantity_max: t.yield_quantity_max,
            base_quality_min: t.base_quality_min,
            base_quality_max: t.base_quality_max,
            required_tool: t.required_tool.clone(),
            extraction_duration_ticks: t.extraction_duration_ticks,
            regeneration_ticks: t.regeneration_ticks,
        })
        .collect();
    resource_types.sort_by(|a, b| a.id.cmp(&b.id));

    let mut container_types: Vec<_> = game
        .container_types
        .values()
        .map(|t| CatalogContainerType {
            id: t.id.clone(),
            name: t.name.clone(),
            slot_count: t.slot_count,
            layout_columns: t.layout_columns,
        })
        .collect();
    container_types.sort_by(|a, b| a.id.cmp(&b.id));

    let item_names: HashMap<String, String> = game
        .item_types
        .iter()
        .map(|(id, t)| (id.clone(), t.name.clone()))
        .collect();
    let mut recipes: Vec<_> = game.recipes.values().map(|r| r.to_info(&item_names)).collect();
    recipes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut catalog = CatalogResponse {
        version: String::new(),
        item_types,
        station_types,
        resource_types,
        container_types,
        recipes,
    };
    catalog.version = version(&catalog);
    catalog
}

/// A hash of `catalog` without its own `version`: 64-bit FNV-1a over the
/// JSON, as the terrain's version is.
fn version(catalog: &CatalogResponse) -> String {
    let unversioned = CatalogResponse {
        version: String::new(),
        ..catalog.clone()
    };
    let json = serde_json::to_vec(&unversioned).expect("the catalog serializes");
    let hash = json.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
        (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ContainerType, ItemType, StationType, WorldGenParams};

    fn game() -> GameState {
        let mut game = GameState::with_params(WorldGenParams::default());
        for (id, name) in [("rope", "Rope"), ("fiber", "Plant Fiber")] {
            game.item_types.insert(
                id.into(),
                ItemType {
                    id: id.into(),
                    name: name.into(),
                    category: "raw_material".into(),
                    stackable: true,
                    weight: 0.1,
                },
            );
        }
        game.station_types.insert(
            "workbench".into(),
            StationType {
                id: "workbench".into(),
                name: "Workbench".into(),
                category: "crafting".into(),
                slot_count: 16,
                layout_columns: 4,
                interaction_range: 75.0,
                icon: Some("workbench".into()),
                description: None,
                kit_item: Some("workbench_kit".into()),
            },
        );
        game.container_types.insert(
            "crafting_output".into(),
            ContainerType {
                id: "crafting_output".into(),
                name: "Crafting Output".into(),
                slot_count: 1,
                layout_columns: 1,
            },
        );
        game
    }

    #[test]
    fn lists_are_sorted_and_stations_name_their_kit() {
        let catalog = build(&game());
        let ids: Vec<_> = catalog.item_types.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["fiber", "rope"]);
        assert_eq!(catalog.station_types[0].kit.as_deref(), Some("workbench_kit"));
        assert_eq!(catalog.container_types[0].slot_count, 1);
    }

    #[test]
    fn the_version_follows_the_content_and_nothing_else() {
        let mut game = game();
        let first = build(&game);
        assert_eq!(first.version.len(), 16);
        // Inserted in the other order: a map's order may change; the version doesn't.
        let mut items: Vec<_> = std::mem::take(&mut game.item_types).into_iter().collect();
        items.reverse();
        game.item_types = items.into_iter().collect();
        assert_eq!(build(&game).version, first.version);

        game.item_types.get_mut("rope").unwrap().name = "Twine".into();
        assert_ne!(build(&game).version, first.version);
    }
}
