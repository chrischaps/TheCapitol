//! The static game data as one response, for `GET /catalog` (E0.2).
//!
//! It's built from the tables `main` loads into [`GameState`] at startup, so
//! a client sees exactly what the server plays by. Every list is sorted by id
//! and the version is a hash of the rest, so the same content always gives
//! the same version, on any machine and across restarts.

use std::collections::HashMap;

use capitol_protocol::rest::{
    CatalogContainerType, CatalogHabitat, CatalogItemType, CatalogResourceType, CatalogResponse,
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
            target_price: t.target_price,
            tool: t.tool(),
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
            off_plot: t.off_plot,
            ground_only: t.ground_only,
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
            tool_type: t.tool_type.clone(),
            difficulty: t.difficulty,
            yields: t.yields.0.clone(),
            cluster: t.cluster.clone(),
            spawns: t.spawns.0.clone(),
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

    // From the content, not a table: only the planner reads `wilds.toml`
    let mut habitats: Vec<_> = crate::content::wilds()
        .habitat
        .iter()
        .map(|h| CatalogHabitat { resource: h.resource.clone(), habitat: h.habitat() })
        .collect();
    habitats.sort_by(|a, b| a.resource.cmp(&b.resource));

    let mut catalog = CatalogResponse {
        version: String::new(),
        item_types,
        station_types,
        resource_types,
        container_types,
        recipes,
        habitats,
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
                    ..Default::default()
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
                off_plot: true,
                ground_only: false,
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
        assert!(catalog.station_types[0].off_plot);
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

    /// E2.2: every resource's yields, cluster and spawn table reach the
    /// catalog as the content has them, and come back the same off the wire.
    #[test]
    fn resource_tables_round_trip_through_the_catalog() {
        let mut game = game();
        let content = crate::content::load().unwrap();
        for def in &content.resources {
            game.resource_types.insert(def.id.clone(), def.to_type());
        }
        let catalog = build(&game);
        let wire: CatalogResponse = serde_json::from_str(&serde_json::to_string(&catalog).unwrap()).unwrap();
        assert_eq!(wire, catalog);
        assert_eq!(wire.resource_types.len(), content.resources.len());
        for def in &content.resources {
            let t = wire.resource_types.iter().find(|t| t.id == def.id).unwrap();
            assert_eq!(t.spawns, def.spawn_table(), "{}", def.id);
            assert_eq!(t.yields, def.yield_table(), "{}", def.id);
            assert_eq!((t.cluster.as_str(), t.difficulty), (def.cluster.as_str(), def.difficulty), "{}", def.id);
        }
        let tree = wire.resource_types.iter().find(|t| t.id == "tree").unwrap();
        assert_eq!((tree.yield_quantity_min, tree.yield_quantity_max), (1, 2), "a gated resource's least tier");
    }

    /// #413: every habitat in `wilds.toml` is on the wire, sorted, and
    /// judges ground as the planner's own does.
    #[test]
    fn the_habitats_are_on_the_wire() {
        let catalog = build(&game());
        let wilds = crate::content::wilds();
        assert_eq!(catalog.habitats.len(), wilds.habitat.len());
        assert!(catalog.habitats.windows(2).all(|w| w[0].resource < w[1].resource), "sorted by resource");
        for def in &wilds.habitat {
            let wire = catalog.habitats.iter().find(|h| h.resource == def.resource).unwrap();
            assert_eq!(wire.habitat, def.habitat(), "{}", def.resource);
        }
        let copper = catalog.habitats.iter().find(|h| h.resource == "copper_vein").unwrap();
        assert!(copper.habitat.suits(500.0, 300.0, 0.8) && !copper.habitat.suits(500.0, 100.0, 0.8));
        assert!(copper.habitat.on_massif(Some(2)) && !copper.habitat.on_massif(Some(3)), "copper keeps to the even massifs");
    }

    /// The protocol's catalog fixture, which is also the Flint client's
    /// offline fallback, is what a server on this content serves. After a
    /// content change, run it with `CATALOG_FIXTURE=write` to rewrite both.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_catalog_fixture_is_the_content(db: sqlx::PgPool) {
        use crate::models::{RecipeRow, ResourceType, ITEM_TYPE_COLUMNS, STATION_TYPE_COLUMNS};
        crate::content::load_and_sync(&db).await.unwrap();
        // The tables as `main` loads them
        let mut game = GameState::with_params(WorldGenParams::default());
        let resources: Vec<ResourceType> = sqlx::query_as(
            "SELECT id, name, yields_item, yield_quantity_min, yield_quantity_max,
             base_quality_min, base_quality_max, required_tool, extraction_duration_ticks, regeneration_ticks,
             tool_type, difficulty, yields, cluster, spawns
             FROM resource_types",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        game.resource_types = resources.into_iter().map(|t| (t.id.clone(), t)).collect();
        let items: Vec<ItemType> = sqlx::query_as(&format!("SELECT {ITEM_TYPE_COLUMNS} FROM item_types"))
            .fetch_all(&db)
            .await
            .unwrap();
        game.item_types = items.into_iter().map(|t| (t.id.clone(), t)).collect();
        let recipes: Vec<RecipeRow> = sqlx::query_as(
            "SELECT id, name, description, category, inputs, outputs, base_duration_ticks, quality_weights,
             crafting_requirements, required_station_type
             FROM recipes",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        game.recipes = recipes.into_iter().map(|r| (r.id.clone(), r.into())).collect();
        let stations: Vec<StationType> = sqlx::query_as(&format!("SELECT {STATION_TYPE_COLUMNS} FROM station_types"))
            .fetch_all(&db)
            .await
            .unwrap();
        game.station_types = stations.into_iter().map(|t| (t.id.clone(), t)).collect();
        let containers: Vec<ContainerType> =
            sqlx::query_as("SELECT id, name, slot_count, layout_columns FROM container_types")
                .fetch_all(&db)
                .await
                .unwrap();
        game.container_types = containers.into_iter().map(|t| (t.id.clone(), t)).collect();

        // Written as captured: no newline at the end
        let json = serde_json::to_string(&build(&game)).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let fixture = root.join("protocol/tests/fixtures/rest/catalog.json");
        let fallback = root.join("flint-client/crates/capitol-core/src/catalog_fallback.json");
        if std::env::var("CATALOG_FIXTURE").as_deref() == Ok("write") {
            for path in [&fixture, &fallback] {
                std::fs::write(path, &json).unwrap();
            }
        }
        let on_disk = std::fs::read_to_string(&fixture).unwrap();
        assert!(
            on_disk.trim_end() == json,
            "the catalog fixture is stale: run this test with CATALOG_FIXTURE=write"
        );
    }
}
