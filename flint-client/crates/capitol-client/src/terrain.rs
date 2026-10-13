//! Where the world's terrain comes from: a file (`--offline`), or the
//! server's `/terrain` through the disk cache (`capitol_core::terrain`).
//!
//! Online, a cached terrain is drawn on the first frame while the server is
//! asked whether it's still current. A 304 changes nothing; a newer terrain
//! replaces the cache and the world is rebuilt from it.
//!
//! With `--live` the client also signs in and opens the WebSocket, so it
//! hears `terrain_updated` (an admin regenerating the world from the React
//! client's F2 tweaker). The poller refetches `/terrain`, and the world is
//! rebuilt from the answer while the game runs. Until P4.1's login screen
//! there's no one to sign in as, so it signs in as a dev account of its own,
//! `flint1@bots.capitol.local` (`Flint1`), registered the first time.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use capitol_core::bot::Account;
use capitol_core::net::{ConnectionState, Net, NetConfig, NetEvent, RestResponse, Tag};
use capitol_core::poll::{Fetch, Poller};
use capitol_core::protocol::terrain::TerrainResponse;
use capitol_core::signin::{SignIn, Step};
use capitol_core::state::GameState;
use capitol_core::terrain::{read_terrain_file, TerrainCache, TerrainFetch};

/// The game's REST tags sit below `poll::FIRST_TAG`.
const TERRAIN_TAG: Tag = 1;
const SIGN_IN_TAG: Tag = 2;

/// Who `--live` signs in as until P4.1 asks the player.
fn dev_account() -> Account {
    Account::numbered("flint", 1)
}

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
    /// `--live` only.
    live: Option<Live>,
}

/// A session, held only to hear `terrain_updated`. The store and the poller
/// are the ones P4 runs the game on; P4.1 moves them up into the extension.
struct Live {
    sign_in: SignIn,
    state: GameState,
    poller: Poller,
    /// `None` until signed in.
    connection: Option<ConnectionState>,
    failed: Option<String>,
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
            online: None,
            current: Some(terrain),
            changed: true,
            origin: Origin::File,
        })
    }

    /// Fetches from `base_url`, drawing the cached copy until it answers.
    /// `live` also signs in and listens for `terrain_updated`.
    pub fn online(base_url: &str, live: bool) -> Result<Self> {
        let net =
            Net::spawn(NetConfig::with_base_url(base_url)).context("starting the net thread")?;
        let cache = TerrainCache::new(CACHE_DIR, base_url);
        let (fetch, cached) = TerrainFetch::start(Some(cache), TERRAIN_TAG);
        if cached.is_none() {
            tracing::info!("terrain: no cache yet; fetching from {base_url}");
        }
        let live = live.then(|| {
            let (sign_in, request) = SignIn::start(dev_account(), SIGN_IN_TAG);
            tracing::info!("live: signing in as {}", sign_in.account().email);
            net.rest(request, SIGN_IN_TAG);
            Live {
                sign_in,
                state: GameState::new(),
                poller: Poller::new(),
                connection: None,
                failed: None,
            }
        });
        Ok(Self {
            online: Some(Online { net, fetch, live }),
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
        let mut newest = None;
        for event in online.net.drain() {
            if let Some(terrain) = online.fetch.observe(&event, now) {
                newest = Some((*terrain, Origin::Server));
            }
            let Some(live) = &mut online.live else {
                continue;
            };
            if let Some(terrain) = live.observe(&event, &online.net) {
                tracing::info!(
                    "terrain: updated to version {} ({} roads, {} plots)",
                    terrain.version.as_deref().unwrap_or("none"),
                    terrain.roads.len(),
                    terrain.terrain_plots.len(),
                );
                online.fetch.adopt(&terrain);
                newest = Some((terrain, Origin::Updated));
            }
        }
        if let Some(live) = &mut online.live {
            for (request, tag) in live.poller.tick(now) {
                online.net.rest(request, tag);
            }
        }
        if let Some((terrain, origin)) = newest {
            self.current = Some(terrain);
            self.changed = true;
            self.origin = origin;
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
        let line = match self.origin {
            Origin::File => format!("offline · terrain {version}"),
            Origin::Cache if checked => format!("terrain {version} · cache current"),
            Origin::Cache => format!("terrain {version} · cached, checking…"),
            Origin::Server => format!("terrain {version} · fetched"),
            Origin::Updated => format!("terrain {version} · updated live"),
            Origin::Waiting => "fetching terrain…".into(),
        };
        match self.online.as_ref().and_then(|o| o.live.as_ref()) {
            Some(live) => format!("{line}\n{}", live.status()),
            None => line,
        }
    }
}

impl Live {
    /// Feeds one event to the sign-in, the store and the poller. Returns the
    /// terrain when the event is the poller's refetch after `terrain_updated`.
    fn observe(&mut self, event: &NetEvent, net: &Net) -> Option<TerrainResponse> {
        match self.sign_in.observe(event) {
            Some(Step::Send(request)) => {
                tracing::info!("live: no such account; registering it");
                net.rest(request, self.sign_in.tag());
            }
            Some(Step::SignedIn(auth)) => {
                tracing::info!("live: signed in; connecting");
                self.connection = Some(ConnectionState::Connecting);
                net.connect(auth.token);
            }
            Some(Step::Failed(why)) => {
                tracing::warn!("live: couldn't sign in ({why}); terrain updates won't arrive");
                self.failed = Some(why);
            }
            None => {}
        }
        if let NetEvent::Connection(connection) = event {
            match connection {
                ConnectionState::Connected { .. } => tracing::info!("live: listening"),
                ConnectionState::Reconnecting { reason, .. } => {
                    tracing::warn!("live: connection dropped ({reason}); reconnecting")
                }
                ConnectionState::AuthFailed { message } => {
                    tracing::warn!("live: the server ended the session: {message}")
                }
                _ => {}
            }
            self.connection = Some(connection.clone());
        }
        let changes = self.state.apply(event);
        let answered = self.poller.observe(event, &changes, &self.state);
        match (answered, event) {
            (Some(Fetch::Terrain), NetEvent::Rest(_, Ok(RestResponse::Terrain(terrain)))) => {
                Some((**terrain).clone())
            }
            (Some(Fetch::Terrain), NetEvent::Rest(_, Err(e))) => {
                tracing::warn!("terrain: refetch after terrain_updated failed: {e}");
                None
            }
            _ => None,
        }
    }

    /// The HUD's second line.
    fn status(&self) -> String {
        let name = &self.sign_in.account().name;
        if let Some(why) = &self.failed {
            return format!("live: couldn't sign in ({why})");
        }
        match &self.connection {
            None => format!("live: signing in as {name}…"),
            Some(ConnectionState::Connected { .. }) => format!("live as {name}"),
            Some(ConnectionState::Reconnecting { .. }) => {
                format!("live as {name} · reconnecting…")
            }
            Some(ConnectionState::AuthFailed { message }) => {
                format!("live: session ended ({message})")
            }
            Some(ConnectionState::Disconnected { .. }) => "live: disconnected".into(),
            Some(_) => format!("live: connecting as {name}…"),
        }
    }
}
