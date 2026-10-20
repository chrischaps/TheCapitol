//! The static game data: item, station, resource and container types, and
//! recipes (E0.2).
//!
//! Online it's `GET /catalog`, fetched with every snapshot and again
//! whenever `/health` reports a version the client doesn't have. The last
//! answer is kept on disk per server ([`CatalogCache`]), so the game starts
//! with the server's names before the first fetch lands. [`Catalog::fallback`]
//! is a copy of the server's seed content, for `--offline` and for a server
//! that hasn't answered yet and was never cached.
//!
//! The fallback is `catalog_fallback.json`, the same bytes as the protocol's
//! fixture (`protocol/tests/fixtures/rest/catalog.json`); a test keeps them
//! equal. Refresh both from a live server when the seed content changes.
//!
//! IDs and category strings are the wire values.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use capitol_protocol::rest::{
    CatalogContainerType, CatalogItemType, CatalogResourceType, CatalogResponse,
    CatalogStationType, RecipeInfo,
};

const FALLBACK_JSON: &str = include_str!("catalog_fallback.json");

/// One server's static game data. Cheap to clone.
#[derive(Debug, Clone, PartialEq)]
pub struct Catalog {
    data: Arc<CatalogResponse>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::fallback()
    }
}

impl Catalog {
    pub fn new(data: CatalogResponse) -> Self {
        Self {
            data: Arc::new(data),
        }
    }

    /// The seed content the client was built with.
    pub fn fallback() -> Self {
        static FALLBACK: OnceLock<Catalog> = OnceLock::new();
        FALLBACK
            .get_or_init(|| {
                Catalog::new(
                    serde_json::from_str(FALLBACK_JSON).expect("catalog_fallback.json parses"),
                )
            })
            .clone()
    }

    /// The server's hash of this content.
    pub fn version(&self) -> &str {
        &self.data.version
    }

    pub fn data(&self) -> &CatalogResponse {
        &self.data
    }

    pub fn item_types(&self) -> &[CatalogItemType] {
        &self.data.item_types
    }

    pub fn item_type(&self, id: &str) -> Option<&CatalogItemType> {
        self.data.item_types.iter().find(|t| t.id == id)
    }

    /// The item's display name. One the catalog doesn't know is named from
    /// its id, `lantern_oil` as "Lantern Oil", rather than shown as a blank.
    pub fn item_name(&self, id: &str) -> String {
        match self.item_type(id) {
            Some(t) => t.name.clone(),
            None => title_case(id),
        }
    }

    pub fn station_types(&self) -> &[CatalogStationType] {
        &self.data.station_types
    }

    pub fn station_type(&self, id: &str) -> Option<&CatalogStationType> {
        self.data.station_types.iter().find(|t| t.id == id)
    }

    /// The station type a kit places, if `item_type` is a kit: the one
    /// whose `kit` it is (#181).
    pub fn station_for_kit(&self, item_type: &str) -> Option<&CatalogStationType> {
        self.station_types()
            .iter()
            .find(|t| t.kit.as_deref() == Some(item_type))
    }

    /// The kit item a station type's placement consumes.
    pub fn kit_for_station(&self, station_type: &str) -> Option<&str> {
        self.station_type(station_type)?.kit.as_deref()
    }

    pub fn resource_types(&self) -> &[CatalogResourceType] {
        &self.data.resource_types
    }

    pub fn resource_type(&self, id: &str) -> Option<&CatalogResourceType> {
        self.data.resource_types.iter().find(|t| t.id == id)
    }

    pub fn container_type(&self, id: &str) -> Option<&CatalogContainerType> {
        self.data.container_types.iter().find(|t| t.id == id)
    }

    /// Every recipe, hand and station alike.
    pub fn recipes(&self) -> &[RecipeInfo] {
        &self.data.recipes
    }
}

/// `snake_case` as "Snake Case".
fn title_case(id: &str) -> String {
    id.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Container type IDs (`container_types`). `/inventory/containers` returns one
/// of each of the first three per player.
pub mod container_type {
    pub const PLAYER_INVENTORY: &str = "player_inventory";
    pub const CRAFTING_INPUT: &str = "crafting_input";
    pub const CRAFTING_OUTPUT: &str = "crafting_output";
    pub const STATION_INVENTORY: &str = "station_inventory";
    /// Where bought goods and returned units wait (E3.1)
    pub const MARKET_BOX: &str = "market_box";
    /// What a foreclosure gave back that the inventory had no room for (#141)
    pub const IMPOUND: &str = "impound";
}

/// One server's last `/catalog` on disk, beside its terrain.
#[derive(Debug, Clone)]
pub struct CatalogCache {
    path: PathBuf,
}

impl CatalogCache {
    /// The cache for the server at `base_url`, as a file in `dir`.
    pub fn new(dir: impl Into<PathBuf>, base_url: &str) -> Self {
        Self {
            path: crate::terrain::server_file(dir.into(), "catalog", base_url),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The cached catalog. A missing file is `None`; so is an unreadable
    /// one, which is logged and then replaced by the next store.
    pub fn load(&self) -> Option<Catalog> {
        let read = || -> io::Result<CatalogResponse> {
            let bytes = fs::read(&self.path)?;
            serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        };
        match read() {
            Ok(data) => Some(Catalog::new(data)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                tracing::warn!(
                    "catalog cache {} unreadable, ignoring it: {e}",
                    self.path.display()
                );
                None
            }
        }
    }

    /// Writes `catalog` through a temporary file and a rename.
    pub fn store(&self, catalog: &Catalog) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let bytes = serde_json::to_vec(catalog.data()).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fallback_is_the_protocol_fixture() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../protocol/tests/fixtures/rest/catalog.json");
        let fixture = fs::read_to_string(&fixture).expect("the protocol's catalog fixture");
        assert_eq!(FALLBACK_JSON.trim_end(), fixture.trim_end());
    }

    #[test]
    fn every_kit_places_a_known_station() {
        let catalog = Catalog::fallback();
        let placeable = catalog
            .item_types()
            .iter()
            .filter(|t| t.category == "placeable");
        for item in placeable {
            let station = catalog.station_for_kit(&item.id).expect(&item.id);
            assert_eq!(catalog.kit_for_station(&station.id), Some(item.id.as_str()));
            assert!(!item.stackable);
        }
        assert!(catalog.station_for_kit("rope").is_none());
        assert_eq!(catalog.kit_for_station("throne"), None);
    }

    #[test]
    fn a_kit_the_client_never_heard_of_comes_from_the_catalog() {
        let mut data = Catalog::fallback().data().clone();
        let mut loom = data.station_types[0].clone();
        loom.id = "loom".into();
        loom.kit = Some("loom_kit".into());
        data.station_types.push(loom);
        let catalog = Catalog::new(data);
        assert_eq!(catalog.station_for_kit("loom_kit").unwrap().id, "loom");
        assert_eq!(catalog.kit_for_station("loom"), Some("loom_kit"));
    }

    #[test]
    fn resources_and_recipes_name_known_items() {
        let catalog = Catalog::fallback();
        for resource in catalog.resource_types() {
            assert!(catalog.item_type(&resource.yields_item).is_some());
        }
        for recipe in catalog.recipes() {
            for item in recipe.inputs.iter().map(|i| &i.item_type) {
                assert!(catalog.item_type(item).is_some(), "{item}");
            }
            for item in recipe.outputs.iter().map(|o| &o.item_type) {
                assert!(catalog.item_type(item).is_some(), "{item}");
            }
        }
        assert_eq!(catalog.resource_type("grass").unwrap().name, "Wild Grass");
        assert!(catalog
            .container_type(container_type::CRAFTING_OUTPUT)
            .is_some());
    }

    #[test]
    fn an_unknown_item_is_named_from_its_id() {
        let catalog = Catalog::fallback();
        assert_eq!(catalog.item_name("fiber"), "Plant Fiber");
        assert_eq!(catalog.item_name("lantern_oil"), "Lantern Oil");
    }

    #[test]
    fn the_cache_round_trips_and_ignores_junk() {
        let dir = std::env::temp_dir().join(format!("capitol-catalog-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cache = CatalogCache::new(&dir, "http://localhost:3000");
        assert!(cache.path().ends_with("catalog-localhost_3000.json"));
        assert_eq!(cache.load(), None);
        let mut data = Catalog::fallback().data().clone();
        data.version = "abc".into();
        cache.store(&Catalog::new(data.clone())).unwrap();
        assert_eq!(cache.load().unwrap().data(), &data);
        fs::write(cache.path(), b"{ not json").unwrap();
        assert_eq!(cache.load(), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
