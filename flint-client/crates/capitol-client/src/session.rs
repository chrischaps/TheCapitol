//! The client's one connection to the server: the `Net`, the way in
//! (`capitol_core::auth`), and once signed in, the store, the poller and
//! the players' motion the living world runs on (P4).
//!
//! It exists whenever the client is online, signed in or not, because
//! `/terrain` and `/health` need no account. [`Session::update`] runs once
//! per frame. It sends the terrain fetch and the auth machine's requests,
//! drains the net and feeds every event to the terrain, the auth machine,
//! and the store, the poller and the motion.
//!
//! The store, poller and motion only see events between a sign-in and a sign-out,
//! and start fresh each time, so one player's world never leaks into the
//! next one's.
//!
//! The server URL, the last email and the token are kept in
//! `save/session.toml` ([`SessionFile`]).

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capitol_core::auth::{Auth, Effect, Phase, Remembered};
use capitol_core::craft::{self, Crafting};
use capitol_core::harvest::{Harvest, Note, Step};
use capitol_core::motion::{Motion, Pose};
use capitol_core::net::{
    ConnectionState, Net, NetConfig, NetEvent, RestError, RestRequest, RestResponse, Tag,
};
use capitol_core::poll::{Fetch, Poller};
use capitol_core::protocol::rest::{MoveItemRequest, PositionQuery};
use capitol_core::protocol::rules::Zone;
use capitol_core::protocol::ws::ClientMessage;
use capitol_core::state::{GameState, StateEvent, Yield};
use flint_runtime::PersistentStore;
use uuid::Uuid;

use crate::terrain::TerrainSource;
use crate::ui::slots::Move;

/// The auth machine's tags: sign-in, then health, then logout. Below
/// `poll::FIRST_TAG`, above `terrain.rs`'s.
pub const AUTH_TAG: Tag = 10;

/// The tag on `move_item` requests: above the auth machine's, below
/// `poll::FIRST_TAG`. The poller refetches on any `MoveItem` answer.
const MOVE_TAG: Tag = 20;

/// The tags on the plot panel's calls (P6.4): `/plots/at` and `/plots/{id}`
/// share one, a claim has its own.
const PLOT_TAG: Tag = 30;
const CLAIM_TAG: Tag = 31;

/// How long a refused move's reason stays up.
const NOTICE_FOR: Duration = Duration::from_secs(5);

/// The server when neither `--server` nor the file names one.
pub const DEFAULT_SERVER: &str = "http://localhost:3000";

pub struct Session {
    server: String,
    net: Net,
    auth: Auth,
    state: GameState,
    poller: Poller,
    /// Every player's pose, smoothed between snapshots (P2.5).
    motion: Motion,
    /// The connection's outage, if it's down, for the overlay (P4.5).
    link: Link,
    /// The zone you last entered, for the banner (P4.5).
    arrival: Option<Arrival>,
    /// The store, poller and motion have seen events since they were last reset.
    used: bool,
    /// Why the last move was refused, and when.
    move_refused: Option<(String, Instant)>,
    file: SessionFile,
    /// Where the last click sent you, in server units (P4.3).
    last_move: Option<(f64, f64)>,
    /// Water you walked into since the extension last took them (P4.3).
    blocked: Vec<Blocked>,
    /// Clicking nodes: extract, or walk there and extract (P4.4).
    harvest: Harvest,
    /// What your extractions yielded, by node, since the extension last
    /// took them.
    yields: Vec<(Uuid, Yield)>,
    /// Crafting by hand (P5.5): the craft under way, from the ask to the
    /// settled inventory.
    crafting: Crafting,
    /// What your crafts made since the extension last took them.
    crafted: Vec<Yield>,
    /// Stations placed (`station_placed`) since the frame last took them.
    placed: HashSet<Uuid>,
    /// Why the last craft didn't happen, and when.
    craft_refused: Option<(String, Instant)>,
    /// The plot a claim is out for (P6.4).
    claiming: Option<Uuid>,
    /// Why the last claim was refused, and when.
    claim_refused: Option<(String, Instant)>,
    /// Plots you claimed since the frame last took them.
    claimed: Vec<Uuid>,
    /// What's worth telling the player about picking a plot, since the
    /// frame last took them.
    plot_notes: Vec<PlotNote>,
}

/// Something to tell the player about picking a plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotNote {
    /// The alt-click found no plot there.
    NoneThere,
    /// The server couldn't say.
    Unanswered,
}

/// The server stopped you at the water's edge (`movement_blocked`).
#[derive(Debug, Clone, PartialEq)]
pub struct Blocked {
    /// Where you stand now, in server units.
    pub x: f64,
    pub y: f64,
    /// The server's words, such as "Cannot cross Moat without a bridge".
    pub reason: String,
    /// Where you were going when the water stopped you, if it was a click.
    pub toward: Option<(f64, f64)>,
}

/// How long after the first zone is known its banner waits: about as long
/// as the camera takes to come down onto you from the login circle, so the
/// name lands with you.
pub const FIRST_BANNER_DELAY: Duration = Duration::from_millis(2600);

/// A zone the player entered: the banner names it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrival {
    pub zone: Zone,
    /// When the banner starts, which for the first zone is a little later.
    pub at: Instant,
    /// The zone you came into the world in, rather than walked into.
    pub first: bool,
}

impl Session {
    /// Starts the net thread for `server`. If the file holds a token for
    /// this server, it's tried at once.
    pub fn start(server: &str, file: SessionFile) -> Result<Session> {
        let net =
            Net::spawn(NetConfig::with_base_url(server)).context("starting the net thread")?;
        let remembered = file.remembered(server);
        if remembered.token.is_some() {
            tracing::info!("session: returning to {server} with the stored token");
        }
        let (auth, effects) = Auth::new(remembered, AUTH_TAG);
        let mut session = Session {
            server: server.to_owned(),
            net,
            auth,
            state: GameState::new(),
            poller: Poller::new(),
            motion: Motion::new(),
            link: Link::default(),
            arrival: None,
            used: false,
            move_refused: None,
            file,
            last_move: None,
            blocked: Vec::new(),
            harvest: Harvest::default(),
            yields: Vec::new(),
            crafting: Crafting::default(),
            crafted: Vec::new(),
            placed: HashSet::new(),
            craft_refused: None,
            claiming: None,
            claim_refused: None,
            claimed: Vec::new(),
            plot_notes: Vec::new(),
        };
        session.apply(effects);
        Ok(session)
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn auth(&self) -> &Auth {
        &self.auth
    }

    /// The world mirror. Empty until signed in.
    pub fn state(&self) -> &GameState {
        &self.state
    }

    /// Every player's pose at `now`, the local player first (once the first
    /// `positions` has placed them), and the store they were drawn from.
    pub fn sample(&mut self, now: Instant) -> (&[Pose], &GameState) {
        (self.motion.sample(now), &self.state)
    }

    /// The connection's outage while it's down.
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// The zone you last entered, and when.
    pub fn arrival(&self) -> Option<Arrival> {
        self.arrival
    }

    /// Who's signed in: the player's name once the server has said it,
    /// otherwise the email.
    pub fn who(&self) -> String {
        match self.state.local_player() {
            Some(me) => me.name.clone(),
            None => self.auth.email().unwrap_or_default().to_owned(),
        }
    }

    /// Walk to `(x, y)` in server units: the prediction starts at once and
    /// the server hears `move`. Only in the world.
    pub fn move_to(&mut self, now: Instant, x: f64, y: f64) {
        self.harvest.walked();
        self.walk(now, x, y);
    }

    fn walk(&mut self, now: Instant, x: f64, y: f64) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        self.motion.move_to(now, x, y);
        self.net.ws(ClientMessage::Move { x, y });
        self.last_move = Some((x, y));
    }

    /// You clicked a resource node: extract from it if it's in range, or
    /// walk there and extract on arrival (`capitol_core::harvest`). A node
    /// that can't be taken now is walked to, as React does.
    pub fn extract(&mut self, now: Instant, node: Uuid) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        match self.harvest.click(node, &self.state, now) {
            Some(step) => self.step(now, step),
            None => {
                if let Some(n) = self.state.node(node) {
                    let (x, y) = (n.x, n.y);
                    self.move_to(now, x, y);
                }
            }
        }
    }

    fn step(&mut self, now: Instant, step: Step) {
        match step {
            Step::Extract(node) => self.net.ws(ClientMessage::Extract {
                node_id: node.to_string(),
            }),
            Step::WalkTo { x, y } => self.walk(now, x, y),
            Step::Hold => {}
        }
    }

    /// The node you're walking to, to extract on arrival, or the one an
    /// `extract` is out for: the node to mark as targeted.
    pub fn harvest_target(&self) -> Option<Uuid> {
        self.harvest.target().or(self.harvest.asked())
    }

    /// What your extractions yielded since the last call, by node.
    pub fn take_yields(&mut self) -> Vec<(Uuid, Yield)> {
        std::mem::take(&mut self.yields)
    }

    /// What's worth telling the player about extracting, since the last call.
    pub fn take_harvest_notes(&mut self) -> Vec<Note> {
        self.harvest.take_notes()
    }

    /// Where you're walking to, in server units, for the marker.
    pub fn destination(&self) -> Option<(f64, f64)> {
        self.motion.destination()
    }

    /// Every time the water stopped you since the last call.
    pub fn take_blocked(&mut self) -> Vec<Blocked> {
        std::mem::take(&mut self.blocked)
    }

    /// Asks the server to move an item (a drop on a slot grid). It moves,
    /// merges or swaps, and the poller refetches the containers when it
    /// answers. Sent over REST rather than the socket: the socket's answer
    /// names only the target container, so a move out of a station's box
    /// would leave the inventory stale. A workaround: #77.
    pub fn move_item(&mut self, m: Move) {
        self.net.rest(
            RestRequest::MoveItem(MoveItemRequest {
                item_id: m.item_id,
                target_container_id: m.target_container_id,
                target_slot: m.target_slot,
            }),
            MOVE_TAG,
        );
    }

    /// Why the server refused the last move, for a few seconds after. A
    /// station's box refuses every move until B4 (#41) lands.
    pub fn move_refused(&self, now: Instant) -> Option<&str> {
        self.move_refused
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) < NOTICE_FOR)
            .map(|(why, _)| why.as_str())
    }

    /// Sends several moves at once (the crafting panel's Fill and Clear).
    /// Their targets must differ, or two could race for one slot.
    pub fn move_items(&mut self, shifts: &[craft::Shift]) {
        for &(item_id, target_container_id, target_slot) in shifts {
            self.move_item(Move {
                item_id,
                target_container_id,
                target_slot,
            });
        }
    }

    /// The craft under way, for the crafting panel.
    pub fn crafting(&self) -> &Crafting {
        &self.crafting
    }

    /// Crafts `recipe_id` from the input box by hand, if it's ready and
    /// nothing else is under way.
    pub fn craft(&mut self, now: Instant, recipe_id: &str) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        let (Some(recipe), Some(input)) =
            (self.state.recipe(recipe_id), self.state.crafting_input())
        else {
            return;
        };
        let check = craft::check(recipe, input, self.state.crafting_output());
        if let Some(message) = self.crafting.start(&check, &self.state, now) {
            tracing::info!(
                "craft: {} from {} stacks",
                check.recipe.id,
                check.inputs.len()
            );
            self.craft_refused = None;
            self.net.ws(message);
        }
    }

    /// Stops the craft under way.
    pub fn cancel_craft(&mut self) {
        if let Some(message) = self.crafting.cancel(&self.state) {
            self.net.ws(message);
        }
    }

    /// Why the last craft didn't happen, for a few seconds after.
    pub fn craft_refused(&self, now: Instant) -> Option<&str> {
        self.craft_refused
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) < NOTICE_FOR)
            .map(|(why, _)| why.as_str())
    }

    /// What your crafts made since the last call.
    pub fn take_crafted(&mut self) -> Vec<Yield> {
        std::mem::take(&mut self.crafted)
    }

    /// The stations placed since this was last called, so they can drop
    /// in rather than rise (P6.1).
    pub fn take_placed(&mut self) -> HashSet<Uuid> {
        std::mem::take(&mut self.placed)
    }

    /// Picks the plot at the server point `(x, y)` for the plot panel
    /// (an alt-click). The answer opens the panel, or closes it if there's
    /// no plot there.
    pub fn pick_plot(&mut self, x: f64, y: f64) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        self.claim_refused = None;
        self.net
            .rest(RestRequest::PlotAt(PositionQuery { x, y }), PLOT_TAG);
    }

    /// Closes the plot panel.
    pub fn close_plot(&mut self) {
        self.state.unpick_plot();
        self.claim_refused = None;
    }

    /// Buys the deed to `plot_id`, unless a claim is already out.
    pub fn claim_plot(&mut self, plot_id: Uuid) {
        if self.auth.phase() != Phase::InWorld || self.claiming.is_some() {
            return;
        }
        tracing::info!("plot: claiming {plot_id}");
        self.claiming = Some(plot_id);
        self.claim_refused = None;
        self.net.rest(RestRequest::ClaimPlot { plot_id }, CLAIM_TAG);
    }

    /// A claim is out.
    pub fn claiming(&self) -> bool {
        self.claiming.is_some()
    }

    /// Why the server refused the last claim, for a few seconds after.
    pub fn claim_refused(&self, now: Instant) -> Option<&str> {
        self.claim_refused
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) < NOTICE_FOR)
            .map(|(why, _)| why.as_str())
    }

    /// The plots you claimed since the last call.
    pub fn take_claimed(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.claimed)
    }

    /// What to tell the player about picking plots, since the last call.
    pub fn take_plot_notes(&mut self) -> Vec<PlotNote> {
        std::mem::take(&mut self.plot_notes)
    }

    /// Sends a message on the socket. Dropped while not connected.
    pub fn ws(&self, message: ClientMessage) {
        self.net.ws(message);
    }

    pub fn log_in(&mut self, email: &str, password: &str) {
        let effects = self.auth.log_in(email, password);
        self.apply(effects);
    }

    pub fn register(&mut self, name: &str, email: &str, password: &str) {
        let effects = self.auth.register(name, email, password);
        self.apply(effects);
    }

    /// Leaves the world, or stops trying to enter it.
    pub fn log_out(&mut self) {
        let effects = self.auth.log_out();
        self.apply(effects);
        self.reset_world();
    }

    /// Hands the file over, for a session on another server.
    pub fn into_file(self) -> SessionFile {
        self.file
    }

    /// Call once per frame.
    pub fn update(&mut self, now: Instant, terrain: &mut TerrainSource) {
        if let Some((request, tag)) = terrain.tick(now) {
            self.net.rest(request, tag);
        }
        let effects = self.auth.tick(now);
        self.apply(effects);

        let events: Vec<NetEvent> = self.net.drain().collect();
        for event in &events {
            terrain.observe(event, now);
            let effects = self.auth.observe(event);
            self.apply(effects);
            if !self.in_session() {
                self.reset_world();
                continue;
            }
            self.used = true;
            match event {
                NetEvent::Rest(MOVE_TAG, Err(e)) => {
                    tracing::info!("inventory: move refused: {e}");
                    self.move_refused = Some((refusal(e), now));
                }
                NetEvent::Rest(MOVE_TAG, Ok(_)) => self.move_refused = None,
                NetEvent::Rest(PLOT_TAG, Ok(RestResponse::PlotAt(None))) => {
                    self.plot_notes.push(PlotNote::NoneThere)
                }
                NetEvent::Rest(PLOT_TAG, Err(e)) => {
                    tracing::warn!("plot: lookup failed: {e}");
                    self.plot_notes.push(PlotNote::Unanswered);
                }
                NetEvent::Rest(CLAIM_TAG, Ok(RestResponse::ClaimPlot(claim))) => {
                    tracing::info!(
                        "plot: claimed {} for {} Strands, {} left",
                        claim.plot.id,
                        claim.plot.assessed_value,
                        claim.new_balance
                    );
                    self.claiming = None;
                    self.claimed.push(claim.plot.id);
                }
                NetEvent::Rest(CLAIM_TAG, result) => {
                    let why = match result {
                        Err(e) => refusal(e),
                        Ok(_) => "The server's answer didn't make sense".into(),
                    };
                    tracing::info!("plot: claim refused: {why}");
                    self.claim_refused = Some((why, now));
                    // Someone may have bought it first: show the panel what's true.
                    if let Some(plot_id) = self.claiming.take() {
                        self.net.rest(RestRequest::Plot { plot_id }, PLOT_TAG);
                    }
                }
                _ => {}
            }
            if let NetEvent::Connection(connection) = event {
                self.link.observe(connection, now);
            }
            let changes = self.state.apply(event);
            for change in &changes {
                if let StateEvent::ZoneChanged { from, to } = change {
                    let first = from.is_none();
                    self.arrival = Some(Arrival {
                        zone: *to,
                        at: now
                            + if first {
                                FIRST_BANNER_DELAY
                            } else {
                                Duration::ZERO
                            },
                        first,
                    });
                }
            }
            self.motion.observe(now, &changes, &self.state);
            for change in &changes {
                match change {
                    StateEvent::MovementBlocked { reason, x, y } => {
                        self.blocked.push(Blocked {
                            x: *x,
                            y: *y,
                            reason: reason.clone(),
                            toward: self.last_move.take(),
                        });
                    }
                    StateEvent::ExtractionCompleted { node_id, item } => {
                        self.yields.push((*node_id, item.clone()));
                    }
                    StateEvent::CraftingCompleted { item } => self.crafted.push(item.clone()),
                    StateEvent::StationPlaced { station_id, .. } => {
                        self.placed.insert(*station_id);
                    }
                    _ => {}
                }
            }
            self.crafting.observe(&changes, &self.state, now);
            self.note_crafting(now);
            if let Some(step) = self.harvest.observe(&changes, &self.state, now) {
                self.step(now, step);
            }
            let answered = self.poller.observe(event, &changes, &self.state);
            match (answered, event) {
                (Some(Fetch::Terrain), NetEvent::Rest(_, Ok(RestResponse::Terrain(t)))) => {
                    terrain.updated((**t).clone());
                }
                (Some(Fetch::Terrain), NetEvent::Rest(_, Err(e))) => {
                    tracing::warn!("terrain: refetch after terrain_updated failed: {e}");
                }
                _ => {}
            }
        }
        if self.in_session() {
            self.harvest.tick(now);
            self.crafting.tick(now);
            self.note_crafting(now);
            self.state.tick(now);
            for (request, tag) in self.poller.tick(now) {
                self.net.rest(request, tag);
            }
        }
    }

    /// Between a sign-in and a sign-out: the store's events are this player's.
    fn in_session(&self) -> bool {
        matches!(self.auth.phase(), Phase::Entering { .. } | Phase::InWorld)
    }

    fn reset_world(&mut self) {
        if std::mem::take(&mut self.used) {
            self.state = GameState::new();
            self.poller = Poller::new();
            self.motion = Motion::new();
            self.last_move = None;
            self.blocked.clear();
            self.move_refused = None;
            self.link = Link::default();
            self.arrival = None;
            self.harvest = Harvest::default();
            self.yields.clear();
            self.crafting = Crafting::default();
            self.crafted.clear();
            self.craft_refused = None;
            self.claiming = None;
            self.claim_refused = None;
            self.claimed.clear();
            self.plot_notes.clear();
        }
    }

    /// Keeps the crafting machine's last note for the panel to show.
    fn note_crafting(&mut self, now: Instant) {
        for note in self.crafting.take_notes() {
            let why = match note {
                craft::Note::Failed(reason) => reason,
                craft::Note::Unanswered => "The server didn't answer. Try again".into(),
            };
            tracing::info!("craft: refused: {why}");
            self.craft_refused = Some((why, now));
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Rest(request, tag) => self.net.rest(request, tag),
                Effect::Connect(token) => self.net.connect(token),
                Effect::Disconnect => self.net.disconnect(),
                Effect::Remember(remembered) => self.file.remember(&self.server, &remembered),
            }
        }
    }
}

/// A refused request's reason, as a player should read it: the server's own
/// words when it gave some.
fn refusal(e: &RestError) -> String {
    match e {
        RestError::Http { message, .. } if !message.is_empty() => message.clone(),
        RestError::Http { status, .. } => format!("The server refused (HTTP {status})"),
        RestError::Transport(_) => "Couldn't reach the server".into(),
        RestError::Decode(_) => "The server's answer didn't make sense".into(),
    }
}

/// The connection seen from the overlay: up, or down since when, on which
/// attempt, and when the next try starts. `ConnectionState` says each step
/// as it happens; this keeps what the overlay needs between them, such as
/// the attempt number while that attempt is connecting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Link {
    outage: Option<Outage>,
}

/// A dropped connection that's being retried.
#[derive(Debug, Clone, PartialEq)]
pub struct Outage {
    /// When it dropped.
    pub since: Instant,
    /// The retry under way or waited for, counting from 1.
    pub attempt: u32,
    /// Why the last try ended, in the net layer's words.
    pub reason: String,
    /// The wait before this attempt.
    pub delay: Duration,
    /// When the attempt starts. Past it, the attempt is connecting.
    pub next_try: Instant,
}

impl Link {
    pub fn observe(&mut self, state: &ConnectionState, now: Instant) {
        match state {
            ConnectionState::Reconnecting {
                attempt,
                delay,
                reason,
            } => {
                let since = self.outage.as_ref().map_or(now, |o| o.since);
                self.outage = Some(Outage {
                    since,
                    attempt: *attempt,
                    reason: reason.clone(),
                    delay: *delay,
                    next_try: now + *delay,
                });
            }
            // An attempt under way: the outage holds until it's through.
            ConnectionState::Connecting | ConnectionState::Authenticating => {}
            // Back, or the session is over and the form takes it from here.
            ConnectionState::Connected { .. }
            | ConnectionState::Disconnected { .. }
            | ConnectionState::AuthFailed { .. } => self.outage = None,
        }
    }

    /// The outage, if the connection is down.
    pub fn outage(&self) -> Option<&Outage> {
        self.outage.as_ref()
    }
}

/// `save/session.toml`: the server last chosen, the last email that signed
/// in, and its token, with the server the token belongs to. A token is only
/// tried on that server.
///
/// It's a file of its own, not the engine's `save/persist.toml`. Scripts can
/// read and write every key in that store (`persist_get`), and a session
/// token has no business there. Extensions can't reach that store anyway
/// (issue #71).
pub struct SessionFile {
    path: PathBuf,
    store: PersistentStore,
}

const SERVER: &str = "server";
const EMAIL: &str = "email";
const TOKEN: &str = "token";
const TOKEN_SERVER: &str = "token_server";

impl SessionFile {
    pub const PATH: &str = "save/session.toml";

    /// Reads `path`. A missing or unreadable file is an empty one.
    pub fn load(path: impl Into<PathBuf>) -> SessionFile {
        let path = path.into();
        let mut store = PersistentStore::new();
        if path.is_file() {
            if let Err(e) = store.load_from_file(&path) {
                tracing::warn!("session: ignoring {}: {e}", path.display());
            }
        }
        SessionFile { path, store }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.store
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }

    fn set(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(value) => self.store.set(key, toml::Value::String(value.to_owned())),
            None => {
                self.store.remove(key);
            }
        }
    }

    /// The server chosen in the form, if any.
    pub fn server(&self) -> Option<String> {
        self.get(SERVER)
    }

    pub fn set_server(&mut self, server: &str) {
        self.set(SERVER, Some(server));
        self.save();
    }

    /// The email, and the token if it belongs to `server`.
    pub fn remembered(&self, server: &str) -> Remembered {
        let token = match self.get(TOKEN_SERVER) {
            Some(s) if s == server => self.get(TOKEN),
            _ => None,
        };
        Remembered {
            email: self.get(EMAIL),
            token,
        }
    }

    pub fn remember(&mut self, server: &str, remembered: &Remembered) {
        self.set(EMAIL, remembered.email.as_deref());
        self.set(TOKEN, remembered.token.as_deref());
        self.set(TOKEN_SERVER, remembered.token.as_ref().map(|_| server));
        self.save();
    }

    fn save(&mut self) {
        if let Some(dir) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!("session: can't create {}: {e}", dir.display());
                return;
            }
        }
        match self.store.save_to_file(&self.path) {
            Ok(()) => self.store.mark_clean(),
            Err(e) => tracing::warn!("session: can't write {}: {e}", self.path.display()),
        }
    }
}

/// A server URL as typed into the form: trimmed, `http://` added when no
/// scheme was given, and no trailing slash. `None` for nothing usable.
pub fn normalize_server(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    let url = if s.contains("://") {
        s.to_owned()
    } else {
        format!("http://{s}")
    };
    let url = url.trim_end_matches('/');
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    (!rest.is_empty()).then(|| url.to_owned())
}

/// The server without its scheme, for the form's footer.
pub fn server_label(server: &str) -> &str {
    server.split("://").last().unwrap_or(server)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outage_lasts_from_the_drop_to_the_reconnect() {
        let t0 = Instant::now();
        let s = Duration::from_secs_f32;
        let mut link = Link::default();
        link.observe(
            &ConnectionState::Connected {
                player_id: Default::default(),
            },
            t0,
        );
        assert_eq!(link.outage(), None);

        let reconnecting = |attempt, delay| ConnectionState::Reconnecting {
            attempt,
            delay,
            reason: "connection reset".into(),
        };
        link.observe(&reconnecting(1, s(0.5)), t0);
        link.observe(&ConnectionState::Connecting, t0 + s(0.5));
        link.observe(&reconnecting(2, s(1.0)), t0 + s(0.6));
        // The attempt and its wait move on; when it dropped doesn't.
        let o = link.outage().unwrap();
        assert_eq!((o.since, o.attempt, o.delay), (t0, 2, s(1.0)));
        assert_eq!(o.next_try, t0 + s(1.6));
        // While that attempt connects, the overlay still says which it is.
        link.observe(&ConnectionState::Authenticating, t0 + s(1.7));
        assert_eq!(link.outage().unwrap().attempt, 2);

        link.observe(
            &ConnectionState::Connected {
                player_id: Default::default(),
            },
            t0 + s(1.8),
        );
        assert_eq!(link.outage(), None);

        // A token refused while away ends it too: the form takes over.
        link.observe(&reconnecting(1, s(0.5)), t0 + s(5.0));
        link.observe(
            &ConnectionState::AuthFailed {
                message: "Session expired".into(),
            },
            t0 + s(5.6),
        );
        assert_eq!(link.outage(), None);
    }

    #[test]
    fn server_urls_are_tidied() {
        assert_eq!(
            normalize_server(" localhost:3000/ ").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(
            normalize_server("https://capitol.example").as_deref(),
            Some("https://capitol.example")
        );
        assert_eq!(
            normalize_server("192.168.1.20:3000").as_deref(),
            Some("http://192.168.1.20:3000")
        );
        assert_eq!(normalize_server(""), None);
        assert_eq!(normalize_server("ftp://x"), None);
        assert_eq!(normalize_server("http://"), None);
        assert_eq!(normalize_server("local host"), None);
        assert_eq!(server_label("http://localhost:3000"), "localhost:3000");
    }

    /// Against the local stack: a slot grid's drop goes to the server, the
    /// refetch brings the move back, and a refused move's reason is kept to
    /// show. The drag that makes the `Move` is tested headless in
    /// `ui::slots`. Run `capitol-bot --prefix slots` once first, so the
    /// account holds something.
    #[test]
    #[ignore = "needs the local stack and the capitol-bot --prefix slots account"]
    fn a_move_reaches_the_server_and_comes_back() {
        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let dir = std::env::temp_dir().join(format!("capitol-move-{}", std::process::id()));
        let mut session = Session::start(&server, SessionFile::load(dir.join("s.toml"))).unwrap();
        // Offline terrain: an online one would write its cache in the crate.
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        session.log_in("slots1@bots.capitol.local", "slots-1-capitol-bot");

        let mut wait = |session: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done(session) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                session.update(Instant::now(), &mut terrain);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&mut session, "the inventory", &|s| {
            s.state().inventory().is_some()
        });
        let inventory = session.state().inventory().unwrap().clone();
        let item = inventory
            .items()
            .next()
            .expect("the account holds something: run capitol-bot --prefix slots")
            .clone();
        let to = inventory.first_empty().expect("a free slot") as i32;
        let in_slot = |slot: i32, id: uuid::Uuid| {
            move |s: &Session| {
                s.state()
                    .inventory()
                    .and_then(|c| c.slot(slot as usize))
                    .is_some_and(|i| i.item_id == id)
            }
        };

        let there = Move {
            item_id: item.item_id,
            target_container_id: inventory.id,
            target_slot: to,
        };
        session.move_item(there);
        wait(&mut session, "the move", &in_slot(to, item.item_id));
        session.move_item(Move {
            target_slot: item.slot_index,
            ..there
        });
        wait(
            &mut session,
            "the move back",
            &in_slot(item.slot_index, item.item_id),
        );

        session.move_item(Move {
            target_slot: 999,
            ..there
        });
        wait(&mut session, "the refusal", &|s| {
            s.move_refused(Instant::now()).is_some()
        });
        let why = session.move_refused(Instant::now()).unwrap().to_owned();
        assert!(why.contains("out of bounds"), "{why}");

        session.move_item(Move {
            target_container_id: uuid::Uuid::from_u128(1), // nobody's
            ..there
        });
        wait(&mut session, "the second refusal", &|s| {
            s.move_refused(Instant::now())
                .is_some_and(|w| !w.contains("out of bounds"))
        });
        eprintln!(
            "refused: {why} / {}",
            session.move_refused(Instant::now()).unwrap()
        );
        session.log_out();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Against the local stack, the crafting panel's rules end to end: Fill,
    /// a rope, the rope then blocking a workbench kit's output, a cancel
    /// that uses nothing, and the kit. A bot gathers the fiber first, on
    /// `craft1@bots.capitol.local`.
    #[test]
    #[ignore = "needs the local stack; gathers fiber, so takes a minute"]
    fn rope_then_a_workbench_kit_through_the_panel_rules() {
        use capitol_core::bot::{Account, Bot};
        use capitol_core::catalog::container_type::{CRAFTING_INPUT, CRAFTING_OUTPUT};
        use capitol_core::craft::{self, Phase};
        use capitol_core::state::Container;

        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let account = Account::numbered("craft", 1);
        {
            let mut bot = Bot::sign_in(NetConfig::with_base_url(&server), &account).unwrap();
            bot.gather("fiber", 12).unwrap();
        }

        let dir = std::env::temp_dir().join(format!("capitol-craft-{}", std::process::id()));
        let mut session = Session::start(&server, SessionFile::load(dir.join("s.toml"))).unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        session.log_in(&account.email, &account.password);
        let mut wait = |session: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !done(session) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                session.update(Instant::now(), &mut terrain);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&mut session, "the boxes and recipes", &|s| {
            let st = s.state();
            st.inventory().is_some()
                && st.crafting_input().is_some()
                && st.crafting_output().is_some()
                && !st.recipes().is_empty()
        });
        let boxed = |s: &Session, kind: &str| -> Container {
            s.state()
                .containers()
                .values()
                .find(|c| c.container_type == kind)
                .unwrap()
                .clone()
        };
        let held = |s: &Session, kind: &str, t: &str| craft::count(&boxed(s, kind), t);
        // Everything in a box back into the inventory.
        let drain = |session: &mut Session, kind: &str| {
            let shifts =
                craft::empty_into(&boxed(session, kind), session.state().inventory().unwrap());
            session.move_items(&shifts);
        };
        let fill = |session: &mut Session, id: &str| {
            let r = session.state().recipe(id).unwrap().clone();
            let st = session.state();
            let shifts = craft::fill(&r, st.inventory().unwrap(), st.crafting_input().unwrap());
            session.move_items(&shifts);
        };
        let check = |s: &Session, id: &str| {
            let st = s.state();
            let r = st.recipe(id).unwrap().clone();
            let c = craft::check(&r, st.crafting_input().unwrap(), st.crafting_output());
            (c.satisfied(), c.blocked.is_some())
        };
        let idle = |s: &Session| *s.crafting().phase() == Phase::Idle && !s.state().is_crafting();

        // Start clean: both boxes back into the inventory.
        for kind in [CRAFTING_INPUT, CRAFTING_OUTPUT] {
            drain(&mut session, kind);
            wait(&mut session, kind, &|s| boxed(s, kind).used_slots() == 0);
        }

        // Rope.
        fill(&mut session, "rope_basic");
        wait(&mut session, "fiber for rope", &|s| {
            check(s, "rope_basic").0
        });
        let before = held(&session, CRAFTING_INPUT, "fiber");
        session.craft(Instant::now(), "rope_basic");
        wait(&mut session, "the rope to start", &|s| {
            s.state().is_crafting()
        });
        wait(&mut session, "the rope", &idle);
        assert_eq!(held(&session, CRAFTING_OUTPUT, "rope"), 1);
        assert_eq!(held(&session, CRAFTING_INPUT, "fiber"), before - 2);
        assert_eq!(session.take_crafted().len(), 1);

        // The rope sits in the output: a workbench kit would be lost.
        drain(&mut session, CRAFTING_INPUT);
        wait(&mut session, "the inputs back", &|s| {
            boxed(s, CRAFTING_INPUT).used_slots() == 0
        });
        fill(&mut session, "craft_workbench");
        wait(&mut session, "fiber for the kit", &|s| {
            check(s, "craft_workbench").0
        });
        assert_eq!(check(&session, "craft_workbench"), (true, true));
        session.craft(Instant::now(), "craft_workbench");
        assert_eq!(
            *session.crafting().phase(),
            Phase::Idle,
            "a blocked craft isn't sent"
        );

        // Move the rope out, then start and cancel: nothing is used.
        drain(&mut session, CRAFTING_OUTPUT);
        wait(&mut session, "the output to clear", &|s| {
            boxed(s, CRAFTING_OUTPUT).used_slots() == 0
        });
        let before = held(&session, CRAFTING_INPUT, "fiber");
        session.craft(Instant::now(), "craft_workbench");
        wait(&mut session, "the kit to start", &|s| {
            s.state().is_crafting()
        });
        session.cancel_craft();
        wait(&mut session, "the cancel", &idle);
        assert_eq!(held(&session, CRAFTING_INPUT, "fiber"), before);
        assert_eq!(held(&session, CRAFTING_OUTPUT, "workbench_kit"), 0);

        // And the kit.
        session.craft(Instant::now(), "craft_workbench");
        wait(&mut session, "the kit to start again", &|s| {
            s.state().is_crafting()
        });
        wait(&mut session, "the kit", &idle);
        assert_eq!(held(&session, CRAFTING_OUTPUT, "workbench_kit"), 1);
        assert_eq!(held(&session, CRAFTING_INPUT, "fiber"), before - 10);
        let made = session.take_crafted();
        eprintln!("made {made:?}");
        assert_eq!(made[0].item_type, "workbench_kit");
        session.log_out();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Against the local stack, the plot panel's calls end to end: pick
    /// the plot you stand on, see its deed price, claim it, and find it in
    /// your plots (the list the gold is drawn from) with the deed paid.
    /// Then a pick over open water finds nothing. The account is
    /// `plots1@bots.capitol.local` (`capitol-bot --prefix plots --gather 0`
    /// makes it), standing on an unclaimed plot with the deed's price in
    /// hand. A plot it already owns passes as "yours" without a claim.
    #[test]
    #[ignore = "needs the local stack and Plots1 on an unclaimed plot with Strands"]
    fn a_plot_is_picked_and_claimed() {
        use capitol_core::plot::Offer;

        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let dir = std::env::temp_dir().join(format!("capitol-plot-{}", std::process::id()));
        let mut session = Session::start(&server, SessionFile::load(dir.join("s.toml"))).unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        session.log_in("plots1@bots.capitol.local", "plots-1-capitol-bot");
        let mut wait = |session: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done(session) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                session.update(Instant::now(), &mut terrain);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&mut session, "me, my balance and the deed prices", &|s| {
            let st = s.state();
            st.local_player().is_some() && st.strands().is_some() && st.deed_prices().is_some()
        });
        let (x, y) = {
            let me = session.state().local_player().unwrap();
            (me.x, me.y)
        };
        session.pick_plot(x, y);
        wait(&mut session, "the pick", &|s| {
            s.state().picked_plot().is_some()
        });
        let st = session.state();
        let plot = st.picked_plot().unwrap().clone();
        assert!(plot.contains(x, y), "the picked plot is under you");
        let offer = Offer::new(&plot, st.me(), st.deed_prices(), st.strands());
        eprintln!("picked {} ({}): {offer:?}", plot.id, plot.zone_id);
        match offer {
            Offer::Claimable { cost } => {
                let before = st.strands().unwrap();
                session.claim_plot(plot.id);
                assert!(session.claiming());
                wait(&mut session, "the claim", &|s| !s.claiming());
                assert_eq!(session.claim_refused(Instant::now()), None);
                assert_eq!(session.take_claimed(), vec![plot.id]);
                let st = session.state();
                assert_eq!(st.strands(), Some(before - cost));
                assert_eq!(st.picked_plot().unwrap().owner_id, st.me());
                assert!(st.plot(plot.id).is_some(), "your plots hold it");
                assert!(st.my_plot_at(x, y).is_some());
            }
            Offer::Yours => assert!(st.plot(plot.id).is_some()),
            other => panic!("set Plots1 up first: {other:?}"),
        }
        // A claim on a plot that's now yours is refused, with the reason kept.
        session.claim_plot(plot.id);
        wait(&mut session, "the refusal", &|s| {
            s.claim_refused(Instant::now()).is_some()
        });
        eprintln!(
            "refused: {}",
            session.claim_refused(Instant::now()).unwrap()
        );

        // The moat, at the centre: no plot there closes the panel.
        session.pick_plot(4000.0, 4000.0);
        wait(&mut session, "nothing there", &|s| {
            s.state().picked_plot().is_none()
        });
        assert_eq!(session.take_plot_notes(), vec![PlotNote::NoneThere]);
        session.log_out();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_token_is_only_tried_on_its_own_server() {
        let dir = std::env::temp_dir().join(format!("capitol-session-{}", std::process::id()));
        let path = dir.join("session.toml");
        let mut file = SessionFile::load(&path);
        file.set_server("http://a:3000");
        file.remember(
            "http://a:3000",
            &Remembered {
                email: Some("ada@capitol.local".into()),
                token: Some("t0ken".into()),
            },
        );

        let file = SessionFile::load(&path);
        assert_eq!(file.server().as_deref(), Some("http://a:3000"));
        assert_eq!(
            file.remembered("http://a:3000").token.as_deref(),
            Some("t0ken")
        );
        let elsewhere = file.remembered("http://b:3000");
        assert_eq!(elsewhere.token, None);
        assert_eq!(elsewhere.email.as_deref(), Some("ada@capitol.local"));

        let mut file = file;
        file.remember(
            "http://a:3000",
            &Remembered {
                email: Some("ada@capitol.local".into()),
                token: None,
            },
        );
        assert_eq!(
            SessionFile::load(&path).remembered("http://a:3000").token,
            None
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
