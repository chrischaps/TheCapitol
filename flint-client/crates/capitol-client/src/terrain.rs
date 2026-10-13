//! Where the world's terrain comes from: a file (`--offline`), or the
//! server's `/terrain` through the disk cache (`capitol_core::terrain`).
//!
//! Online, a cached terrain is drawn on the first frame while the server is
//! asked whether it's still current. A 304 changes nothing; a newer terrain
//! replaces the cache and the world is rebuilt from it. `/terrain` needs no
//! sign-in, so the world is there behind the login form.
//!
//! The session (`session.rs`) owns the `Net`. It sends what
//! [`TerrainSource::tick`] asks for and feeds back every event. Once signed
//! in it also hears `terrain_updated` (an admin regenerating the world from
//! the React client's F2 tweaker). The poller refetches `/terrain`, and the
//! answer arrives here through [`TerrainSource::updated`], so the world is
//! rebuilt while the game runs.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use capitol_core::net::{NetEvent, RestRequest, Tag};
use capitol_core::protocol::terrain::TerrainResponse;
use capitol_core::terrain::{read_terrain_file, TerrainCache, TerrainFetch};

/// The game's REST tags sit below `poll::FIRST_TAG`. The session's start at
/// `session::AUTH_TAG`.
const TERRAIN_TAG: Tag = 1;

/// Where the cache lives, relative to the project root (`flint-client/`),
/// next to the engine's `save/`. Gitignored.
pub const CACHE_DIR: &str = "cache";

pub struct TerrainSource {
    /// `None` offline.
    fetch: Option<TerrainFetch>,
    current: Option<TerrainResponse>,
    /// `current` changed since the world was last built from it.
    changed: bool,
    origin: Origin,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Origin {
    File,
    Cache,
    Server,
    /// Refetched on `terrain_updated`.
    Updated,
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
            fetch: None,
            current: Some(terrain),
            changed: true,
            origin: Origin::File,
        })
    }

    /// Fetches from `base_url`, drawing the cached copy until it answers.
    pub fn online(base_url: &str) -> Self {
        let cache = TerrainCache::new(CACHE_DIR, base_url);
        let (fetch, cached) = TerrainFetch::start(Some(cache), TERRAIN_TAG);
        if cached.is_none() {
            tracing::info!("terrain: no cache yet; fetching from {base_url}");
        }
        Self {
            fetch: Some(fetch),
            origin: if cached.is_some() {
                Origin::Cache
            } else {
                Origin::Waiting
            },
            changed: cached.is_some(),
            current: cached,
        }
    }

    /// The request to send now, if one is due. Call once per frame.
    pub fn tick(&mut self, now: Instant) -> Option<(RestRequest, Tag)> {
        self.fetch.as_mut()?.tick(now)
    }

    /// Feed every event; only the fetch's own answers count.
    pub fn observe(&mut self, event: &NetEvent, now: Instant) {
        let Some(fetch) = &mut self.fetch else {
            return;
        };
        if let Some(terrain) = fetch.observe(event, now) {
            self.replace(*terrain, Origin::Server);
        }
    }

    /// The poller's refetch after `terrain_updated`: cache it and rebuild.
    pub fn updated(&mut self, terrain: TerrainResponse) {
        tracing::info!(
            "terrain: updated to version {} ({} roads, {} plots)",
            terrain.version.as_deref().unwrap_or("none"),
            terrain.roads.len(),
            terrain.terrain_plots.len(),
        );
        if let Some(fetch) = &mut self.fetch {
            fetch.adopt(&terrain);
        }
        self.replace(terrain, Origin::Updated);
    }

    fn replace(&mut self, terrain: TerrainResponse, origin: Origin) {
        self.current = Some(terrain);
        self.changed = true;
        self.origin = origin;
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
        let checked = self.fetch.as_ref().is_some_and(|f| f.is_done());
        match self.origin {
            Origin::File => format!("offline · terrain {version}"),
            Origin::Cache if checked => format!("terrain {version} · cache current"),
            Origin::Cache => format!("terrain {version} · cached, checking…"),
            Origin::Server => format!("terrain {version} · fetched"),
            Origin::Updated => format!("terrain {version} · updated live"),
            Origin::Waiting => "fetching terrain…".into(),
        }
    }
}
