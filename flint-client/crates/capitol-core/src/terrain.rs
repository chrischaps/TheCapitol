//! Getting `/terrain`: a disk cache keyed by the server's `version`, and the
//! fetch that revalidates it.
//!
//! The terrain is about a megabyte (200 KB gzipped) and rarely changes. The
//! server answers `If-None-Match: W/"<version>"` with a 304 and no body while
//! it still has that version (B3), so a client that kept the last answer on
//! disk can draw the world at once and download it again only when it moved.
//!
//! ```no_run
//! # use capitol_core::net::*;
//! # use capitol_core::terrain::*;
//! # use std::time::Instant;
//! # let net = Net::spawn(NetConfig::default()).unwrap();
//! let cache = TerrainCache::new("cache", "http://localhost:3000");
//! let (mut fetch, cached) = TerrainFetch::start(Some(cache), 7);
//! // draw `cached` now, if there is one; then, once per frame:
//! if let Some((request, tag)) = fetch.tick(Instant::now()) {
//!     net.rest(request, tag);
//! }
//! for event in net.drain() {
//!     if let Some(terrain) = fetch.observe(&event, Instant::now()) {
//!         // the server had a newer terrain: rebuild the world from it
//!     }
//! }
//! ```
//!
//! The cache file is the plain `/terrain` JSON, so it reads the same way as
//! `fixtures/terrain.json` and either can be given to `--offline`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use capitol_protocol::terrain::TerrainResponse;

use crate::net::{NetEvent, RestRequest, RestResponse, Tag};

/// How long to wait before asking again after a fetch that got no answer.
pub const RETRY_AFTER: Duration = Duration::from_secs(5);

/// Reads a `/terrain` JSON file: a cache file, or a fixture for `--offline`.
pub fn read_terrain_file(path: &Path) -> io::Result<TerrainResponse> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// One server's terrain on disk.
#[derive(Debug, Clone)]
pub struct TerrainCache {
    path: PathBuf,
}

impl TerrainCache {
    /// The cache for the server at `base_url`, as a file in `dir`. Each server
    /// gets its own file, so a local and a remote server don't evict each other.
    pub fn new(dir: impl Into<PathBuf>, base_url: &str) -> Self {
        let server: String = base_url
            .trim_end_matches('/')
            .split("://")
            .last()
            .unwrap_or_default()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Self {
            path: dir.into().join(format!("terrain-{server}.json")),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The cached terrain. A missing file is `None`; so is an unreadable one,
    /// which is logged and then replaced by the next store.
    pub fn load(&self) -> Option<TerrainResponse> {
        match read_terrain_file(&self.path) {
            Ok(terrain) => Some(terrain),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                tracing::warn!(
                    "terrain cache {} unreadable, ignoring it: {e}",
                    self.path.display()
                );
                None
            }
        }
    }

    /// Writes `terrain`, through a temporary file and a rename, so a crash
    /// mid-write leaves the old cache rather than half a new one.
    pub fn store(&self, terrain: &TerrainResponse) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let bytes = serde_json::to_vec(terrain).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &self.path)
    }
}

/// Fetches `/terrain` once, revalidating the cache, and retries until the
/// server answers. Pure apart from the cache file: the game owns the [`Net`]
/// and the clock.
///
/// [`Net`]: crate::net::Net
#[derive(Debug)]
pub struct TerrainFetch {
    cache: Option<TerrainCache>,
    /// The version the world is drawn from, sent as `If-None-Match`.
    version: Option<String>,
    tag: Tag,
    state: FetchState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FetchState {
    /// Send at the next tick at or after this time.
    Due(Option<Instant>),
    InFlight,
    Done,
}

impl TerrainFetch {
    /// Reads the cache and returns the terrain in it, to draw while the
    /// server is asked whether it's still current. `tag` marks this fetch's
    /// [`NetEvent::Rest`] answers. With no cache, the fetch is a plain GET
    /// and nothing is stored.
    pub fn start(cache: Option<TerrainCache>, tag: Tag) -> (Self, Option<TerrainResponse>) {
        let cached = cache.as_ref().and_then(|c| {
            let terrain = c.load()?;
            tracing::info!(
                "terrain: read cache {} (version {}, {} roads, {} plots)",
                c.path().display(),
                terrain.version.as_deref().unwrap_or("none"),
                terrain.roads.len(),
                terrain.terrain_plots.len(),
            );
            Some(terrain)
        });
        let fetch = Self {
            cache,
            version: cached.as_ref().and_then(|t| t.version.clone()),
            tag,
            state: FetchState::Due(None),
        };
        (fetch, cached)
    }

    /// The request to send now, if one is due. Call once per frame.
    pub fn tick(&mut self, now: Instant) -> Option<(RestRequest, Tag)> {
        match self.state {
            FetchState::Due(at) if at.is_none_or(|at| now >= at) => {
                self.state = FetchState::InFlight;
                let request = RestRequest::Terrain {
                    cached_version: self.version.clone(),
                };
                Some((request, self.tag))
            }
            _ => None,
        }
    }

    /// The server answered (with either outcome), so nothing more will be sent.
    pub fn is_done(&self) -> bool {
        self.state == FetchState::Done
    }

    /// The version the world should be drawn from: the cache's, then the server's.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Feed every [`NetEvent`]. Returns a terrain when the server sent one
    /// the world doesn't have yet; it's in the cache by then.
    pub fn observe(&mut self, event: &NetEvent, now: Instant) -> Option<Box<TerrainResponse>> {
        let NetEvent::Rest(tag, result) = event else {
            return None;
        };
        if *tag != self.tag || self.state != FetchState::InFlight {
            return None;
        }
        match result {
            Ok(RestResponse::TerrainUnchanged) => {
                tracing::info!(
                    "terrain: cache is current (version {})",
                    self.version.as_deref().unwrap_or("none")
                );
                self.state = FetchState::Done;
                None
            }
            Ok(RestResponse::Terrain(terrain)) => {
                self.state = FetchState::Done;
                tracing::info!(
                    "terrain: fetched version {} ({} roads, {} plots)",
                    terrain.version.as_deref().unwrap_or("none"),
                    terrain.roads.len(),
                    terrain.terrain_plots.len(),
                );
                if let Some(cache) = &self.cache {
                    match cache.store(terrain) {
                        Ok(()) => tracing::info!("terrain: cached at {}", cache.path().display()),
                        Err(e) => tracing::warn!(
                            "terrain: couldn't write cache {}: {e}",
                            cache.path().display()
                        ),
                    }
                }
                self.version = terrain.version.clone();
                Some(terrain.clone())
            }
            Ok(other) => {
                tracing::warn!("terrain: unexpected answer {other:?}");
                self.state = FetchState::Done;
                None
            }
            Err(e) => {
                tracing::warn!("terrain: fetch failed ({e}); trying again in {RETRY_AFTER:?}");
                self.state = FetchState::Due(Some(now + RETRY_AFTER));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::RestError;

    fn terrain(version: &str) -> TerrainResponse {
        TerrainResponse {
            world_size: 8000.0,
            center_x: 4000.0,
            center_y: 4000.0,
            water_rings: vec![],
            roads: vec![],
            terrain_plots: vec![],
            version: Some(version.into()),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("capitol-terrain-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn each_server_gets_its_own_file() {
        let local = TerrainCache::new("c", "http://localhost:3000/");
        let remote = TerrainCache::new("c", "https://capitol.example");
        assert_eq!(
            local.path(),
            Path::new("c").join("terrain-localhost_3000.json")
        );
        assert_eq!(
            remote.path(),
            Path::new("c").join("terrain-capitol.example.json")
        );
    }

    #[test]
    fn the_cache_round_trips_and_ignores_junk() {
        let dir = temp_dir("roundtrip");
        let cache = TerrainCache::new(&dir, "http://localhost:3000");
        assert_eq!(cache.load(), None);
        cache.store(&terrain("abc")).unwrap();
        assert_eq!(cache.load(), Some(terrain("abc")));
        fs::write(cache.path(), b"{ not json").unwrap();
        assert_eq!(cache.load(), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_cache_fetches_in_full_and_stores_the_answer() {
        let dir = temp_dir("first");
        let cache = TerrainCache::new(&dir, "http://localhost:3000");
        let now = Instant::now();
        let (mut fetch, cached) = TerrainFetch::start(Some(cache.clone()), 9);
        assert_eq!(cached, None);
        assert_eq!(
            fetch.tick(now),
            Some((
                RestRequest::Terrain {
                    cached_version: None
                },
                9
            ))
        );
        assert_eq!(fetch.tick(now), None, "one request in flight");

        let answer = NetEvent::Rest(9, Ok(RestResponse::Terrain(Box::new(terrain("v1")))));
        assert_eq!(fetch.observe(&answer, now).as_deref(), Some(&terrain("v1")));
        assert!(fetch.is_done());
        assert_eq!(cache.load(), Some(terrain("v1")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cached_terrain_is_revalidated_by_version() {
        let dir = temp_dir("revalidate");
        let cache = TerrainCache::new(&dir, "http://localhost:3000");
        cache.store(&terrain("v1")).unwrap();
        let now = Instant::now();

        // Unchanged: nothing new to draw.
        let (mut fetch, cached) = TerrainFetch::start(Some(cache.clone()), 9);
        assert_eq!(cached, Some(terrain("v1")));
        let (request, _) = fetch.tick(now).unwrap();
        assert_eq!(
            request,
            RestRequest::Terrain {
                cached_version: Some("v1".into())
            }
        );
        assert_eq!(
            fetch.observe(&NetEvent::Rest(9, Ok(RestResponse::TerrainUnchanged)), now),
            None
        );
        assert!(fetch.is_done());

        // Changed: the new one is returned and replaces the cache.
        let (mut fetch, _) = TerrainFetch::start(Some(cache.clone()), 9);
        fetch.tick(now);
        let answer = NetEvent::Rest(9, Ok(RestResponse::Terrain(Box::new(terrain("v2")))));
        assert!(fetch.observe(&answer, now).is_some());
        assert_eq!(fetch.version(), Some("v2"));
        assert_eq!(cache.load(), Some(terrain("v2")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_fetch_is_retried_later_and_other_tags_are_ignored() {
        let now = Instant::now();
        let (mut fetch, _) = TerrainFetch::start(None, 9);
        fetch.tick(now).unwrap();

        let stranger = NetEvent::Rest(10, Ok(RestResponse::Terrain(Box::new(terrain("x")))));
        assert_eq!(fetch.observe(&stranger, now), None);

        let failed = NetEvent::Rest(9, Err(RestError::Transport("refused".into())));
        assert_eq!(fetch.observe(&failed, now), None);
        assert!(!fetch.is_done());
        assert_eq!(fetch.tick(now + RETRY_AFTER / 2), None);
        assert!(fetch.tick(now + RETRY_AFTER).is_some());
    }

    #[test]
    fn the_fixture_is_a_current_terrain() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = read_terrain_file(&path).unwrap();
        assert!(terrain.version.is_some(), "captured from a server with B3");
        assert_eq!(terrain.water_rings.len(), 3);
        assert!(terrain.terrain_plots.len() > 2000);
    }
}
