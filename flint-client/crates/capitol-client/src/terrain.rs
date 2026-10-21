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
use std::sync::{mpsc, Arc};
use std::time::Instant;

use anyhow::{Context, Result};
use capitol_core::net::{NetEvent, RestRequest, Tag};
use capitol_core::paths::WalkGraph;
use capitol_core::protocol::rules::ground::Ground;
use capitol_core::protocol::terrain::{Landmark, LandmarkKind, TerrainResponse};
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
    /// `current`'s land and wild water, indexed for walking on (W4).
    ground: Option<Arc<Ground>>,
    /// `current`'s walk graph, for click-to-walk round the water (#267).
    walks: Walks,
    origin: Origin,
}

/// A walk graph built on a thread of its own: about 0.3 s in a debug build.
#[derive(Default)]
struct Walks {
    built: Option<Arc<WalkGraph>>,
    building: Option<mpsc::Receiver<WalkGraph>>,
}

impl Walks {
    /// Starts building `terrain`'s graph. The last one's is dropped: until
    /// this one is in, walks go straight.
    fn build(terrain: &TerrainResponse) -> Self {
        let terrain = terrain.clone();
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("walk-graph".into()).spawn(move || {
            let start = Instant::now();
            let graph = WalkGraph::new(&terrain);
            tracing::info!("walk graph: {} nodes in {:.2?}", graph.len(), start.elapsed());
            let _ = tx.send(graph);
        });
        if let Err(e) = spawned {
            tracing::warn!("walk graph: couldn't start its thread ({e}); walks go straight");
        }
        Self { built: None, building: Some(rx) }
    }

    fn get(&mut self) -> Option<Arc<WalkGraph>> {
        if let Some(rx) = &self.building {
            match rx.try_recv() {
                Ok(graph) => {
                    self.built = Some(Arc::new(graph));
                    self.building = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.building = None,
            }
        }
        self.built.clone()
    }
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
            ground: Some(Arc::new(Ground::from_terrain(&terrain))),
            walks: Walks::build(&terrain),
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
            ground: cached.as_ref().map(|t| Arc::new(Ground::from_terrain(t))),
            walks: cached.as_ref().map(Walks::build).unwrap_or_default(),
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
        self.ground = Some(Arc::new(Ground::from_terrain(&terrain)));
        self.walks = Walks::build(&terrain);
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

    /// The current terrain's ground, for the local player's prediction.
    pub fn ground(&self) -> Option<&Arc<Ground>> {
        self.ground.as_ref()
    }

    /// The current terrain's walk graph, once it's built.
    pub fn walks(&mut self) -> Option<Arc<WalkGraph>> {
        self.walks.get()
    }

    /// The current terrain's landmark of `kind`, such as the Exchange hall.
    pub fn landmark(&self, kind: LandmarkKind) -> Option<&Landmark> {
        self.current.as_ref()?.landmarks.iter().find(|l| l.kind == kind)
    }

    /// The current terrain's landmark `id`, such as `bazaar_stall_3`.
    pub fn landmark_by_id(&self, id: &str) -> Option<&Landmark> {
        self.current.as_ref()?.landmarks.iter().find(|l| l.id == id)
    }

    /// The bazaar quarter's streets (E3.12), for the walk to a stall.
    pub fn bazaar_streets(&self) -> Vec<Vec<(f64, f64)>> {
        self.current.as_ref().map(|t| capitol_core::bazaar::streets(&t.roads)).unwrap_or_default()
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
