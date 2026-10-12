//! Where the world's terrain comes from: a file (`--offline`), or the
//! server's `/terrain` through the disk cache (`capitol_core::terrain`).
//!
//! Online, a cached terrain is drawn on the first frame while the server is
//! asked whether it's still current. A 304 changes nothing; a newer terrain
//! replaces the cache and the world is rebuilt from it.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use capitol_core::net::{Net, NetConfig, Tag};
use capitol_core::protocol::terrain::TerrainResponse;
use capitol_core::terrain::{read_terrain_file, TerrainCache, TerrainFetch};

/// The game's REST tags sit below `poll::FIRST_TAG`.
const TERRAIN_TAG: Tag = 1;

/// Where the cache lives, relative to the project root (`flint-client/`),
/// next to the engine's `save/`. Gitignored.
pub const CACHE_DIR: &str = "cache";

pub struct TerrainSource {
    /// `None` offline.
    online: Option<Online>,
    current: Option<TerrainResponse>,
    /// `current` changed since the world was last built from it.
    changed: bool,
    origin: Origin,
}

/// The network side, for as long as the terrain is all the client fetches.
/// P4 moves the `Net` up into the extension, which then routes its events here.
struct Online {
    net: Net,
    fetch: TerrainFetch,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Origin {
    File,
    Cache,
    Server,
    Waiting,
}

impl TerrainSource {
    /// The terrain in `path` is the whole world; nothing touches the network.
    pub fn offline(path: &Path) -> Result<Self> {
        let terrain = read_terrain_file(path)
            .with_context(|| format!("reading terrain {}", path.display()))?;
        tracing::info!(
            "terrain: offline from {} (version {}, {} roads, {} plots)",
            path.display(),
            terrain.version.as_deref().unwrap_or("none"),
            terrain.roads.len(),
            terrain.terrain_plots.len(),
        );
        Ok(Self {
            online: None,
            current: Some(terrain),
            changed: true,
            origin: Origin::File,
        })
    }

    /// Fetches from `base_url`, drawing the cached copy until it answers.
    pub fn online(base_url: &str) -> Result<Self> {
        let net =
            Net::spawn(NetConfig::with_base_url(base_url)).context("starting the net thread")?;
        let cache = TerrainCache::new(CACHE_DIR, base_url);
        let (fetch, cached) = TerrainFetch::start(Some(cache), TERRAIN_TAG);
        if cached.is_none() {
            tracing::info!("terrain: no cache yet; fetching from {base_url}");
        }
        Ok(Self {
            online: Some(Online { net, fetch }),
            origin: if cached.is_some() {
                Origin::Cache
            } else {
                Origin::Waiting
            },
            changed: cached.is_some(),
            current: cached,
        })
    }

    /// Call once per frame. Sends and receives on the network when online.
    pub fn update(&mut self) {
        let Some(online) = &mut self.online else {
            return;
        };
        let now = Instant::now();
        if let Some((request, tag)) = online.fetch.tick(now) {
            online.net.rest(request, tag);
        }
        for event in online.net.drain() {
            if let Some(terrain) = online.fetch.observe(&event, now) {
                self.current = Some(*terrain);
                self.changed = true;
                self.origin = Origin::Server;
            }
        }
    }

    /// The terrain, once, each time it changes: build the world from it.
    pub fn take_changed(&mut self) -> Option<&TerrainResponse> {
        if !std::mem::take(&mut self.changed) {
            return None;
        }
        self.current.as_ref()
    }

    /// One line for the HUD: where the terrain came from, and its version.
    pub fn status(&self) -> String {
        let version = self
            .current
            .as_ref()
            .and_then(|t| t.version.as_deref())
            .map(|v| v.get(..8).unwrap_or(v))
            .unwrap_or("unversioned");
        let checked = self.online.as_ref().is_some_and(|o| o.fetch.is_done());
        match self.origin {
            Origin::File => format!("offline · terrain {version}"),
            Origin::Cache if checked => format!("terrain {version} · cache current"),
            Origin::Cache => format!("terrain {version} · cached, checking…"),
            Origin::Server => format!("terrain {version} · fetched"),
            Origin::Waiting => "fetching terrain…".into(),
        }
    }
}
