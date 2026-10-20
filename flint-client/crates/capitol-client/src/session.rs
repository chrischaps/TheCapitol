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
use capitol_core::catalog::{Catalog, CatalogCache};
use capitol_core::craft::{self, Crafting};
use capitol_core::exchange::{Asked, DepositQuote, Receipt, WithdrawQuote};
use capitol_core::harvest::{Harvest, Note, Step};
use capitol_core::market::MarketEvent;
use capitol_core::motion::{Motion, Pose};
use capitol_core::net::{
    ConnectionState, Net, NetConfig, NetEvent, RestError, RestRequest, RestResponse, Tag,
};
use capitol_core::poll::{Fetch, Poller};
use capitol_core::protocol::rest::{
    CollectRequest, DepositRequest, MarketSummaryResponse, MoveItemRequest, MyOrdersQuery, PauseReason,
    PayTaxRequest, PlaceOrder, PositionQuery, SlotItem, SplitStackRequest, WithdrawRequest,
};
use capitol_core::protocol::rules::market::{OrderSide, OrderStatus};
use capitol_core::protocol::rules::Zone;
use capitol_core::protocol::terrain::{Landmark, TerrainResponse};
use capitol_core::protocol::ws::ClientMessage;
use capitol_core::state::{GameState, StateEvent, Trade, TradeAction, TradeEvent, Yield};
use capitol_core::station::{self, Kit, Placing, Visit};
use capitol_core::trading::{self, Approach, Draft};
use flint_runtime::PersistentStore;
use uuid::Uuid;

use crate::terrain::{TerrainSource, CACHE_DIR};
use crate::ui::slots::Move;

/// The auth machine's tags: sign-in, then health, then logout. Below
/// `poll::FIRST_TAG`, above `terrain.rs`'s.
pub const AUTH_TAG: Tag = 10;

/// The tag on `move_item` requests: above the auth machine's, below
/// `poll::FIRST_TAG`. The poller refetches on any `MoveItem` answer.
const MOVE_TAG: Tag = 20;

/// The tags on the plot panel's calls (P6.4): `/plots/at` and `/plots/{id}`
/// share one, a claim has its own, and so does paying its tax (#138).
const PLOT_TAG: Tag = 30;
const CLAIM_TAG: Tag = 31;
const PAY_TAX_TAG: Tag = 32;

/// The tag on the exchange panel's deposits and withdrawals (P7.2). The
/// poller refetches the containers and the balance on either answer.
const EXCHANGE_TAG: Tag = 40;

/// The tag on the login screen's `/catalog`, asked for when `/health`
/// reports a version the client doesn't have (E0.2). In the world the
/// poller fetches it instead.
const CATALOG_TAG: Tag = 50;

/// The tags on the market panel's calls (E3.5): placing, cancelling and
/// collecting share one; the summary, your orders and the market box another.
const MARKET_TAG: Tag = 60;
const MARKET_READ_TAG: Tag = 61;
/// The chart's history (E3.6): this, plus the ask's number, so only the
/// latest ask's answer is shown (`MarketStore::set_history`).
const CHART_TAG: Tag = 1 << 32;

/// The tag on the Gazette's calls (E3.9).
const GAZETTE_TAG: Tag = 70;

/// How often the open market panel asks again for the summary and your
/// orders. The books themselves come over the WS as they change.
const MARKET_REFRESH: Duration = Duration::from_secs(20);

/// How long the exchange's receipt stays up.
const RECEIPT_FOR: Duration = Duration::from_secs(6);

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
    /// The plot a tax payment is out for (#138).
    paying: Option<Uuid>,
    /// Why the last tax payment was refused, and when.
    pay_refused: Option<(String, Instant)>,
    /// Plots you claimed since the frame last took them.
    claimed: Vec<Uuid>,
    /// What's worth telling the player about picking a plot, since the
    /// frame last took them.
    plot_notes: Vec<PlotNote>,
    /// The names of tools that broke, for a toast each.
    broken_tools: Vec<(Uuid, String)>,
    /// Your stations that paused, live or on coming back (E4.4), for a
    /// toast each: the station, its type and why.
    automation_notes: Vec<(Uuid, String, PauseReason)>,
    /// Stations that finished a craft on their own since the frame last
    /// took them, for the world's puff.
    produced: Vec<Uuid>,
    /// Why the last automation command was refused, and when.
    automation_refused: Option<(String, Instant)>,
    /// The deposit or withdrawal the server is answering (P7.2).
    exchanging: Option<Asked>,
    /// Why the last one was refused, and when.
    exchange_refused: Option<(String, Instant)>,
    /// What the last one did, and when.
    receipt: Option<(Receipt, Instant)>,
    /// The login screen's `/catalog` is out.
    catalog_asked: bool,
    /// The market panel is open (E3.5): the summary and your orders are
    /// fetched while it is.
    market_open: bool,
    /// When they were last asked for; `None` asks at the next frame.
    market_asked: Option<Instant>,
    /// Every item with activity, from `/market/summary`.
    market_summary: Option<MarketSummaryResponse>,
    /// A place, cancel or collect is out.
    market_busy: bool,
    /// Why the last one was refused, and when.
    market_refused: Option<(String, Instant)>,
    /// What's worth a toast about your orders, since the extension last
    /// took it.
    market_notes: Vec<TradeNote>,
    /// The Capitol Gazette (E3.9): the editions, what you've read, your byline.
    gazette: capitol_core::gazette::Reader,
    /// The Gazette's call that's out.
    gazette_asked: Option<capitol_core::gazette::Ask>,
    /// Placing a station from a kit (P6.2), while it lasts.
    placing: Option<Placing>,
    /// What placing has to tell the player since the extension last took it.
    placing_notes: Vec<station::Note>,
    /// Clicking stations: open, or walk there and open; close on leaving (P6.3).
    visit: Visit,
    /// What's worth telling the player about stations, since the extension
    /// last took it.
    station_notes: Vec<station::visit::Note>,
    /// Shift-clicking a player: ask, or walk closer and ask (P7.3).
    approach: Approach,
    /// Clicking the Exchange hall: open the market, or walk to its door
    /// and open it there (E3.7).
    hall: capitol_core::hall::Visit,
    /// Your side of the open trade as you build it.
    draft: Draft,
    /// What's worth telling the player about trading, since the extension
    /// last took it.
    trade_notes: Vec<TradeNote>,
    /// Trades that went through since the extension last took them.
    traded: Vec<Traded>,
    /// The last time an offer in the open trade changed, and whose.
    offer_changed: Option<(bool, Instant)>,
}

/// Something to tell the player about trading, as a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeNote {
    pub title: String,
    pub body: String,
}

impl TradeNote {
    fn new(title: impl Into<String>, body: impl Into<String>) -> TradeNote {
        TradeNote {
            title: title.into(),
            body: body.into(),
        }
    }
}

/// A trade that went through, for the world to mark.
#[derive(Debug, Clone, PartialEq)]
pub struct Traded {
    pub partner: String,
    /// Where each of you stood, server units, if known.
    pub me: Option<(f64, f64)>,
    pub them: Option<(f64, f64)>,
}

/// Something to tell the player about picking a plot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlotNote {
    /// The alt-click found no plot there.
    NoneThere,
    /// The server couldn't say.
    Unanswered,
    /// One of your plots owes `tax_owed` in tax, and goes at `forecloses_at` (#131).
    TaxOwed { tax_owed: i64, forecloses_at: i64 },
    /// One of your plots was foreclosed for `tax_owed`, and `refund` came back.
    Foreclosed { tax_owed: i64, refund: i64 },
    /// Not about a plot, but a notice like the tax ones, and shown the same
    /// way: your subscription ran out at `until` (E4.1).
    SubscriptionLapsed {
        until: capitol_core::chrono::DateTime<capitol_core::chrono::Utc>,
    },
    /// A new lot was laid where your `station_type` stood off a plot, so it
    /// was taken down and what it held came back (#188).
    Displaced { station_type: String },
}

/// The server stopped you at the water's edge (`movement_blocked`).
#[derive(Debug, Clone, PartialEq)]
pub struct Blocked {
    /// Where you stand now, in server units.
    pub x: f64,
    pub y: f64,
    /// The server's words, such as "Cannot cross the moat without a bridge".
    pub reason: String,
    /// The ring that stopped you (`moat`, `canal_inner`, ...), empty from a
    /// server older than #74.
    pub water: String,
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
    /// Starts the net thread for `server`. If the file holds a session for
    /// this server, it's tried at once.
    pub fn start(server: &str, file: SessionFile) -> Result<Session> {
        let net =
            Net::spawn(NetConfig::with_base_url(server)).context("starting the net thread")?;
        let remembered = file.remembered(server);
        if remembered.refresh_token.is_some() {
            tracing::info!("session: returning to {server} with the stored session");
        }
        let (auth, effects) = Auth::new(remembered, AUTH_TAG);
        // The server's names and slot counts from last time, until `/catalog`
        // answers with the snapshot (E0.2). With no cache, the built-in seed copy.
        let mut state = GameState::new();
        let cache = CatalogCache::new(CACHE_DIR, server);
        match cache.load() {
            Some(catalog) => {
                tracing::info!(
                    "catalog: read cache {} (version {})",
                    cache.path().display(),
                    catalog.version()
                );
                state.set_catalog(catalog);
            }
            None => tracing::info!("catalog: no cache yet; using the built-in copy"),
        }
        let mut session = Session {
            server: server.to_owned(),
            net,
            auth,
            state,
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
            paying: None,
            pay_refused: None,
            claimed: Vec::new(),
            plot_notes: Vec::new(),
            broken_tools: Vec::new(),
            automation_notes: Vec::new(),
            produced: Vec::new(),
            automation_refused: None,
            exchanging: None,
            exchange_refused: None,
            receipt: None,
            catalog_asked: false,
            market_open: false,
            market_asked: None,
            market_summary: None,
            market_busy: false,
            market_refused: None,
            market_notes: Vec::new(),
            gazette: capitol_core::gazette::Reader::new(None),
            gazette_asked: None,
            placing: None,
            placing_notes: Vec::new(),
            visit: Visit::default(),
            station_notes: Vec::new(),
            approach: Approach::default(),
            hall: capitol_core::hall::Visit::default(),
            draft: Draft::default(),
            trade_notes: Vec::new(),
            traded: Vec::new(),
            offer_changed: None,
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
        self.visit.walked();
        self.approach.walked();
        self.hall.walked();
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

    /// You clicked a station: open it if it's in reach, or walk to its near
    /// side and open it on arrival (`capitol_core::station::Visit`). One the
    /// store doesn't know is walked to, as a click on the ground would be.
    pub fn open_station(&mut self, now: Instant, station: Uuid) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        // A click on a station is a click elsewhere for the node you were after.
        self.harvest.walked();
        self.approach.walked();
        self.hall.walked();
        if let Some(step) = self.visit.click(station, &self.state, now) {
            self.station_step(now, step);
        }
    }

    /// You clicked the Exchange hall: true if you're at it, so the market
    /// opens now. Otherwise you walk to its door, and [`Self::at_hall`]
    /// says when you get there (`capitol_core::hall`).
    pub fn visit_hall(&mut self, now: Instant, hall: &Landmark) -> bool {
        if self.auth.phase() != Phase::InWorld {
            return false;
        }
        let Some(me) = self.state.local_player() else {
            return false;
        };
        let at = [me.x, me.y];
        self.harvest.walked();
        self.visit.walked();
        self.approach.walked();
        match self.hall.click(hall, at) {
            capitol_core::hall::Step::Open => true,
            capitol_core::hall::Step::WalkTo { x, y } => {
                self.walk(now, x, y);
                false
            }
        }
    }

    /// True once, as a walk to the hall (`visit_hall`) comes into range.
    pub fn at_hall(&mut self, hall: &Landmark) -> bool {
        let Some(me) = self.state.local_player() else {
            return false;
        };
        let at = [me.x, me.y];
        self.hall.arrived(hall, at)
    }

    /// Opens the station nearest you, as a click on it would (the O key).
    pub fn open_nearest_station(&mut self, now: Instant) {
        let Some(me) = self.state.local_player() else {
            return;
        };
        let (x, y) = (me.x, me.y);
        let nearest = self
            .state
            .stations()
            .values()
            .min_by(|a, b| {
                let d = |s: &capitol_core::protocol::rest::StationInfo| (s.x - x).hypot(s.y - y);
                d(a).total_cmp(&d(b))
            })
            .map(|s| s.id);
        if let Some(id) = nearest {
            self.open_station(now, id);
        }
    }

    fn station_step(&mut self, now: Instant, step: station::visit::Step) {
        match step {
            station::visit::Step::Open(id) => self.net.ws(ClientMessage::OpenStation {
                station_id: id.to_string(),
            }),
            station::visit::Step::WalkTo { x, y } => self.walk(now, x, y),
            station::visit::Step::Close => self.net.ws(ClientMessage::CloseStation),
            station::visit::Step::Hold => {}
        }
    }

    /// Closes the open station (its panel's Close).
    pub fn close_station(&mut self) {
        if self.state.open_station().is_some() {
            self.net.ws(ClientMessage::CloseStation);
        }
    }

    /// Puts a tool from your inventory in a station's tool slot (E1.6). The
    /// server answers with `station_tool_changed`, or an `error` that
    /// [`Session::move_refused`] shows.
    pub fn install_tool(&mut self, station: Uuid, item_id: Uuid) {
        tracing::info!("station: installing {item_id} in {station}");
        self.net.ws(ClientMessage::InstallTool {
            station_id: station.to_string(),
            item_id: item_id.to_string(),
        });
    }

    /// Takes a station's tool out, into your inventory.
    pub fn remove_tool(&mut self, station: Uuid) {
        tracing::info!("station: taking the tool out of {station}");
        self.net.ws(ClientMessage::RemoveTool {
            station_id: station.to_string(),
        });
    }

    /// Sets a station to make `draft`'s recipe on its own and starts it
    /// (E4.4). Owner only: the server answers `station_automation_changed`,
    /// or an `error` that [`Session::automation_refused`] shows.
    pub fn start_automation(&mut self, station: Uuid, draft: &capitol_core::automation::Draft) {
        let set = self.state.station(station).and_then(|s| s.automation.as_ref());
        for message in draft.start(station, set) {
            tracing::info!("station: automation {} for {station}", message.type_name());
            self.net.ws(message);
        }
        self.automation_refused = None;
    }

    /// Stops a station working on its own. It keeps its setting and count.
    pub fn stop_automation(&mut self, station: Uuid) {
        tracing::info!("station: stopping automation at {station}");
        self.net.ws(ClientMessage::StopAutomation {
            station_id: station.to_string(),
        });
        self.automation_refused = None;
    }

    /// Why the last automation command was refused, if it was just now.
    pub fn automation_refused(&self, now: Instant) -> Option<&str> {
        self.automation_refused
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) < NOTICE_FOR)
            .map(|(why, _)| why.as_str())
    }

    /// Takes a station down. The server refuses unless it's yours.
    pub fn remove_station(&mut self, station: Uuid) {
        tracing::info!("station: removing {station}");
        self.net.ws(ClientMessage::RemoveStation {
            station_id: station.to_string(),
        });
    }

    /// Crafts `recipe_id` at the open station, from its box, if it's ready
    /// and nothing else is under way.
    pub fn craft_at_station(&mut self, now: Instant, recipe_id: &str) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        let (Some(open), Some(recipe), Some(bench)) = (
            self.state.open_station_info(),
            self.state.recipe(recipe_id),
            self.state.station_container(),
        ) else {
            return;
        };
        let station_id = open.id;
        let tool = craft::tool_in(&self.state, recipe, true);
        let check = craft::check_at(recipe, bench, self.state.crafting_output(), open.quality, tool);
        if let Some(message) = self.crafting.start_at(&check, station_id, &self.state, now) {
            tracing::info!(
                "craft: {} at {station_id} from {} stacks",
                check.recipe.id,
                check.inputs.len()
            );
            self.craft_refused = None;
            self.net.ws(message);
        }
    }

    /// What's worth telling the player about stations since the last call.
    pub fn take_station_notes(&mut self) -> Vec<station::visit::Note> {
        std::mem::take(&mut self.station_notes)
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
    /// answers. Sent over REST rather than the socket. The socket now names
    /// both containers (#77), but a refused move comes back on the request's
    /// own tag over REST, not as an anonymous socket `error` (#89).
    pub fn move_item(&mut self, m: Move) {
        // The open station's tool is in no container: a drag of it out of
        // its slot is a `remove_tool`, and it lands where the server puts it
        let installed = self
            .state
            .open_station_info()
            .filter(|s| s.tool.as_ref().is_some_and(|t| t.item_id == m.item_id))
            .map(|s| s.id);
        if let Some(station) = installed {
            self.remove_tool(station);
            return;
        }
        let request = match m.split {
            // A split goes into the same container: the server knows which
            Some(quantity) => RestRequest::SplitStack {
                item_id: m.item_id,
                request: SplitStackRequest {
                    quantity,
                    to_slot: m.target_slot,
                    qualities: m.qualities,
                },
            },
            None => RestRequest::MoveItem(MoveItemRequest {
                item_id: m.item_id,
                target_container_id: m.target_container_id,
                target_slot: m.target_slot,
            }),
        };
        self.net.rest(request, MOVE_TAG);
    }

    /// Why the server refused the last move, for a few seconds after: "Too
    /// far from station", "Not your station", "Slot 30 out of bounds".
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
                split: None,
                qualities: None,
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
        let tool = craft::tool_in(&self.state, recipe, false);
        let check = craft::check(recipe, input, self.state.crafting_output(), tool);
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

    /// Pays `amount` strands of the tax `plot_id` owes, unless a payment is
    /// already out.
    pub fn pay_tax(&mut self, plot_id: Uuid, amount: i64) {
        if self.auth.phase() != Phase::InWorld || self.paying.is_some() {
            return;
        }
        tracing::info!("plot: paying {amount} Strands of tax on {plot_id}");
        self.paying = Some(plot_id);
        self.pay_refused = None;
        let request = PayTaxRequest {
            amount: Some(amount),
        };
        self.net
            .rest(RestRequest::PayTax { plot_id, request }, PAY_TAX_TAG);
    }

    /// A tax payment is out.
    pub fn paying(&self) -> bool {
        self.paying.is_some()
    }

    /// Why the server refused the last tax payment, for a few seconds after.
    pub fn pay_refused(&self, now: Instant) -> Option<&str> {
        self.pay_refused
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) < NOTICE_FOR)
            .map(|(why, _)| why.as_str())
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

    /// The tools that broke since the last call, and their names.
    pub fn take_broken_tools(&mut self) -> Vec<(Uuid, String)> {
        std::mem::take(&mut self.broken_tools)
    }

    /// Your stations that paused since the frame last asked: the station,
    /// its type and why.
    pub fn take_automation_notes(&mut self) -> Vec<(Uuid, String, PauseReason)> {
        std::mem::take(&mut self.automation_notes)
    }

    /// Stations that finished a craft on their own since the frame last asked.
    pub fn take_produced(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.produced)
    }

    /// What to tell the player about picking plots, since the last call.
    pub fn take_plot_notes(&mut self) -> Vec<PlotNote> {
        std::mem::take(&mut self.plot_notes)
    }

    /// Deposits the fiber stacks `item_ids` at the exchange, as `quote`
    /// showed it, unless a deposit or withdrawal is already out.
    pub fn deposit(&mut self, item_ids: Vec<Uuid>, quote: DepositQuote) {
        if self.auth.phase() != Phase::InWorld || self.exchanging.is_some() || item_ids.is_empty() {
            return;
        }
        tracing::info!(
            "exchange: depositing {} fiber for {} Strands",
            quote.fiber,
            quote.strands
        );
        self.ask_exchange(Asked::Deposit(quote));
        self.net.rest(
            RestRequest::Deposit(DepositRequest { item_ids }),
            EXCHANGE_TAG,
        );
    }

    /// Withdraws `quote.fiber` fiber, as `quote` showed it.
    pub fn withdraw(&mut self, quote: WithdrawQuote) {
        let Ok(quantity) = i32::try_from(quote.fiber) else {
            return;
        };
        if self.auth.phase() != Phase::InWorld || self.exchanging.is_some() {
            return;
        }
        tracing::info!(
            "exchange: withdrawing {quantity} fiber for {} Strands",
            quote.strands
        );
        self.ask_exchange(Asked::Withdraw(quote));
        self.net.rest(
            RestRequest::Withdraw(WithdrawRequest { quantity }),
            EXCHANGE_TAG,
        );
    }

    fn ask_exchange(&mut self, asked: Asked) {
        self.exchanging = Some(asked);
        self.exchange_refused = None;
        self.receipt = None;
    }

    /// A deposit or withdrawal is out.
    pub fn exchanging(&self) -> bool {
        self.exchanging.is_some()
    }

    /// Why the server refused the last one, for a few seconds after.
    pub fn exchange_refused(&self, now: Instant) -> Option<&str> {
        self.exchange_refused
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) < NOTICE_FOR)
            .map(|(why, _)| why.as_str())
    }

    /// What the last one did, and how long ago, for a few seconds after.
    pub fn receipt(&self, now: Instant) -> Option<(Receipt, Duration)> {
        self.receipt
            .map(|(r, at)| (r, now.duration_since(at)))
            .filter(|(_, age)| *age < RECEIPT_FOR)
    }

    /// The market panel opened or closed. While it's open the summary and
    /// your orders are kept fresh.
    pub fn set_market_open(&mut self, open: bool) {
        if open && !self.market_open {
            self.market_asked = None;
        }
        self.market_open = open;
    }

    /// Watch `item_type`'s book, and only it: the panel shows one at a time.
    /// `None` watches nothing.
    pub fn watch_market(&mut self, item_type: Option<&str>) {
        let market = self.state.market_mut();
        let others: Vec<String> = market
            .watched()
            .filter(|w| Some(*w) != item_type)
            .map(str::to_owned)
            .collect();
        for other in others {
            market.unwatch(&other);
        }
        if let Some(item) = item_type {
            if !market.is_watching(item) {
                market.watch(item);
            }
        }
    }

    /// Show `key`'s chart, or none: a new one is fetched at once, and again
    /// with the panel's other refreshes. Trades fold in as they land.
    pub fn chart_market(&mut self, key: Option<capitol_core::market::ChartKey>) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        if let Some((ask, query)) = self.state.market_mut().chart(key, capitol_core::chrono::Utc::now()) {
            self.net.rest(RestRequest::MarketHistory(query), CHART_TAG + ask);
        }
    }

    /// Places `order` (`POST /market/orders`), unless a market call is out.
    pub fn place_order(&mut self, order: PlaceOrder) {
        if self.auth.phase() != Phase::InWorld || self.market_busy {
            return;
        }
        tracing::info!(
            "market: {} {} {} at {} each",
            order.side.as_str(),
            order.quantity,
            order.item_type,
            order.price
        );
        self.ask_market(RestRequest::PlaceOrder(order));
    }

    /// Cancels one of your open orders.
    pub fn cancel_order(&mut self, order_id: Uuid) {
        if self.auth.phase() != Phase::InWorld || self.market_busy {
            return;
        }
        tracing::info!("market: cancelling {order_id}");
        self.ask_market(RestRequest::CancelOrder { order_id });
    }

    /// Takes everything in the market box to the inventory, as far as it fits.
    pub fn collect_market_box(&mut self) {
        if self.auth.phase() != Phase::InWorld || self.market_busy {
            return;
        }
        tracing::info!("market: collecting the market box");
        self.ask_market(RestRequest::CollectMarketBox(CollectRequest::default()));
    }

    fn ask_market(&mut self, request: RestRequest) {
        self.market_busy = true;
        self.market_refused = None;
        self.net.rest(request, MARKET_TAG);
    }

    /// A place, cancel or collect is out.
    pub fn market_busy(&self) -> bool {
        self.market_busy
    }

    /// Why the server refused the last one, for a few seconds after.
    pub fn market_refused(&self, now: Instant) -> Option<&str> {
        self.market_refused
            .as_ref()
            .filter(|(_, at)| now.duration_since(*at) < NOTICE_FOR)
            .map(|(why, _)| why.as_str())
    }

    /// Every item with activity, once fetched.
    pub fn market_summary(&self) -> Option<&MarketSummaryResponse> {
        self.market_summary.as_ref()
    }

    /// What's worth a toast about your orders since the last call.
    pub fn take_market_notes(&mut self) -> Vec<TradeNote> {
        std::mem::take(&mut self.market_notes)
    }

    /// Sends what the market store wants said (watching, and asking again),
    /// and while the panel is open asks again for the summary, your orders
    /// and the market box every [`MARKET_REFRESH`].
    fn tend_market(&mut self, now: Instant) {
        for message in self.state.market_mut().take_outgoing() {
            self.net.ws(message);
        }
        if self.auth.phase() != Phase::InWorld || !self.market_open {
            return;
        }
        if self
            .market_asked
            .is_some_and(|at| now.duration_since(at) < MARKET_REFRESH)
        {
            return;
        }
        self.market_asked = Some(now);
        self.net.rest(RestRequest::MarketSummary, MARKET_READ_TAG);
        self.net
            .rest(RestRequest::MyOrders(MyOrdersQuery::default()), MARKET_READ_TAG);
        self.net.rest(RestRequest::MarketBox, MARKET_READ_TAG);
        if let Some((ask, query)) = self.state.market_mut().refresh_chart(capitol_core::chrono::Utc::now()) {
            self.net.rest(RestRequest::MarketHistory(query), CHART_TAG + ask);
        }
    }

    /// Sends the Gazette's next call, if it wants one and none is out.
    fn tend_gazette(&mut self) {
        if self.auth.phase() != Phase::InWorld || self.gazette_asked.is_some() {
            return;
        }
        if let Some(ask) = self.gazette.wants() {
            self.send_gazette(ask);
        }
    }

    fn send_gazette(&mut self, ask: capitol_core::gazette::Ask) {
        use capitol_core::gazette::Ask;
        let request = match ask {
            Ask::Latest => RestRequest::Gazette { day: None },
            Ask::Day(day) => RestRequest::Gazette { day: Some(day) },
            Ask::Byline(named) => RestRequest::SetByline(capitol_core::protocol::rest::GazetteByline { named }),
        };
        self.gazette.sent(ask);
        self.gazette_asked = Some(ask);
        self.net.rest(request, GAZETTE_TAG);
    }

    /// The Gazette as the client knows it.
    pub fn gazette(&self) -> &capitol_core::gazette::Reader {
        &self.gazette
    }

    pub fn gazette_mut(&mut self) -> &mut capitol_core::gazette::Reader {
        &mut self.gazette
    }

    /// Ask the Gazette to print your name, or not.
    pub fn set_byline(&mut self, named: bool) {
        if self.auth.phase() != Phase::InWorld || self.gazette_asked.is_some() {
            return;
        }
        if let Some(ask) = self.gazette.set_named(named) {
            self.send_gazette(ask);
        }
    }

    /// Turns your orders' fills and expiries into toasts.
    fn note_market(&mut self, event: &MarketEvent) {
        let catalog = self.state.catalog();
        let note = match event {
            MarketEvent::OrderFilled { order, trade } => {
                let name = catalog.item_name(&trade.item_type);
                let (verb, end) = match order.side {
                    OrderSide::Buy => ("Bought", "It waits in your market box."),
                    OrderSide::Sell => ("Sold", "The Strands are yours."),
                };
                let left = if order.quantity_remaining > 0 {
                    format!(", {} still on the book", order.quantity_remaining)
                } else {
                    String::new()
                };
                Some(TradeNote::new(
                    "Order filled",
                    format!(
                        "{verb} {} {name} ({}) at {} each{left}. {end}",
                        trade.quantity, trade.grade, trade.price
                    ),
                ))
            }
            MarketEvent::OrderClosed(order) if order.status == OrderStatus::Expired => {
                Some(TradeNote::new(
                    "Order expired",
                    format!(
                        "Your {} order for {} ran its seven days. What was left came back.",
                        order.side.as_str(),
                        catalog.item_name(&order.item_type)
                    ),
                ))
            }
            _ => None,
        };
        if let Some(note) = note {
            tracing::info!("market: {}: {}", note.title, note.body);
            self.market_notes.push(note);
        }
        // A fill moves the last price, and fills the box
        if matches!(event, MarketEvent::OrderFilled { .. }) {
            self.market_asked = None;
        }
    }

    /// Starts placing the station `item` makes, if it's a kit: the ghost
    /// follows the cursor until a click puts it down or it's cancelled.
    /// Another kit's placement gives way to this one.
    pub fn begin_placing(&mut self, item: &SlotItem) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        if let Some(kit) = Kit::of(self.state().catalog(), item) {
            tracing::info!("place: aiming a {}", kit.station_type);
            self.placing = Some(Placing::new(kit));
        }
    }

    /// Stops placing (a right click or Esc). A click already sent still
    /// lands if the server takes it.
    pub fn cancel_placing(&mut self) {
        if self.placing.take().is_some() {
            tracing::info!("place: cancelled");
        }
    }

    /// The placement under way, if any.
    pub fn placing(&self) -> Option<&Placing> {
        self.placing.as_ref()
    }

    /// A click at `(x, y)` server units while placing: put the station there,
    /// if it can go there. `terrain` tells open ground from a lot (#187).
    pub fn place(&mut self, now: Instant, x: f64, y: f64, terrain: Option<&TerrainResponse>) {
        let Some(placing) = &mut self.placing else {
            return;
        };
        if let Some(message) = placing.place([x, y], &self.state, terrain, now) {
            tracing::info!("place: {} at ({x:.0}, {y:.0})", placing.kit().station_type);
            self.net.ws(message);
        }
        self.placing_notes.extend(placing.take_notes());
    }

    /// What placing had to tell the player since the last call.
    pub fn take_placing_notes(&mut self) -> Vec<station::Note> {
        std::mem::take(&mut self.placing_notes)
    }

    /// You shift-clicked `player`: ask them to trade if they're in range, or
    /// walk closer and ask as you arrive. False if the click wasn't for
    /// trading (you, or nobody the store knows), so it should fall through.
    pub fn trade_with(&mut self, now: Instant, player: Uuid) -> bool {
        if self.auth.phase() != Phase::InWorld {
            return false;
        }
        let Some(step) = self.approach.click(player, &mut self.state, now) else {
            return false;
        };
        self.harvest.walked();
        self.visit.walked();
        self.hall.walked();
        self.trade_step(now, step);
        self.note_approach();
        true
    }

    /// Asks the player nearest you to trade (the T key), or accepts the
    /// request you've been sent.
    pub fn trade_key(&mut self, now: Instant) {
        if matches!(self.state.trade(), Trade::Incoming { .. }) {
            self.answer_trade(now, true);
            return;
        }
        let Some(me) = self.state.local_player() else {
            return;
        };
        let (id, x, y) = (me.id, me.x, me.y);
        let nearest = self
            .state
            .players()
            .values()
            .filter(|p| p.id != id)
            .min_by(|a, b| {
                (a.x - x)
                    .hypot(a.y - y)
                    .total_cmp(&(b.x - x).hypot(b.y - y))
            })
            .map(|p| p.id);
        match nearest {
            Some(player) => {
                self.trade_with(now, player);
            }
            None => self.trade_notes.push(TradeNote::new(
                "Nobody about",
                "There's no one here to trade with",
            )),
        }
    }

    fn trade_step(&mut self, now: Instant, step: trading::Step) {
        match step {
            trading::Step::Send(message) => {
                tracing::info!("trade: {}", message.type_name());
                self.net.ws(message);
            }
            trading::Step::WalkTo { x, y } => self.walk(now, x, y),
            trading::Step::Hold => {}
        }
    }

    fn note_approach(&mut self) {
        for note in self.approach.take_notes() {
            let note = match note {
                trading::Note::Refused(why) => TradeNote::new("Can't trade", why.to_string()),
                trading::Note::Lost { name } => {
                    TradeNote::new("Gone", format!("{name} left before you got there"))
                }
            };
            self.trade_notes.push(note);
        }
    }

    /// Accepts or declines the request you were sent.
    pub fn answer_trade(&mut self, now: Instant, accept: bool) {
        let sent = if accept {
            self.state.trade_accept_request(now)
        } else {
            self.state.trade_decline_request()
        };
        self.send_trade(sent);
    }

    /// Forgets the request you sent. The server can't take one back, so if
    /// they accept later the trade still opens.
    pub fn withdraw_trade(&mut self) {
        let _ = self.state.trade_withdraw();
    }

    /// Accepts the open trade as it stands.
    pub fn accept_trade(&mut self, now: Instant) {
        if self.draft.unsent() {
            return;
        }
        let sent = self.state.trade_accept(now);
        self.send_trade(sent);
    }

    /// Cancels the open trade.
    pub fn cancel_trade(&mut self, now: Instant) {
        let sent = self.state.trade_cancel(now);
        self.send_trade(sent);
    }

    fn send_trade(&mut self, sent: Result<ClientMessage, capitol_core::state::TradeRefusal>) {
        match sent {
            Ok(message) => {
                tracing::info!("trade: {}", message.type_name());
                self.net.ws(message);
            }
            Err(why) => {
                tracing::info!("trade: not sent: {why}");
                self.trade_notes
                    .push(TradeNote::new("Can't trade", why.to_string()));
            }
        }
    }

    /// Your side of the open trade as you're building it.
    pub fn draft(&self) -> &Draft {
        &self.draft
    }

    /// Change your side of the open trade. It's sent once it settles.
    pub fn draft_mut(&mut self) -> &mut Draft {
        &mut self.draft
    }

    /// The player you're walking toward to ask, for the HUD.
    pub fn trade_target(&self) -> Option<Uuid> {
        self.approach.target()
    }

    /// The last change to an offer in the open trade: whether it was
    /// theirs, and how long ago.
    pub fn offer_changed(&self, now: Instant) -> Option<(bool, Duration)> {
        self.offer_changed
            .map(|(theirs, at)| (theirs, now.duration_since(at)))
    }

    /// What's worth telling the player about trading since the last call.
    pub fn take_trade_notes(&mut self) -> Vec<TradeNote> {
        std::mem::take(&mut self.trade_notes)
    }

    /// Trades that went through since the last call.
    pub fn take_traded(&mut self) -> Vec<Traded> {
        std::mem::take(&mut self.traded)
    }

    /// Turns the store's trade events into toasts and marks.
    fn note_trades(&mut self, before: &Trade, changes: &[StateEvent], now: Instant) {
        let name = match before {
            Trade::Requested { name, .. } | Trade::Incoming { name, .. } => Some(name.clone()),
            Trade::Open(open) => Some(open.partner_name.clone()),
            Trade::Idle => None,
        };
        let them = name.unwrap_or_else(|| "They".into());
        let me = self.state.me();
        for change in changes {
            let StateEvent::Trade(event) = change else {
                continue;
            };
            tracing::info!("trade: {event:?}");
            let note = match event {
                TradeEvent::Declined { .. } => Some(TradeNote::new(
                    "Declined",
                    format!("{them} won't trade just now"),
                )),
                TradeEvent::Expired { .. } => Some(match before {
                    Trade::Incoming { .. } => TradeNote::new(
                        "Request expired",
                        format!("{them}'s request timed out"),
                    ),
                    _ => TradeNote::new(
                        "No answer",
                        format!("{them} didn't answer in time"),
                    ),
                }),
                TradeEvent::RequestGone { .. } => {
                    Some(TradeNote::new("Request gone", format!("{them} has gone")))
                }
                TradeEvent::Started { .. } => {
                    self.offer_changed = None;
                    None
                }
                TradeEvent::OfferChanged { player_id } => {
                    self.offer_changed = Some((Some(*player_id) != me, now));
                    None
                }
                TradeEvent::Executed { .. } => {
                    let partner = match before {
                        Trade::Open(open) => self.state.player(open.partner_id),
                        _ => None,
                    };
                    self.traded.push(Traded {
                        partner: them.clone(),
                        me: self.state.local_player().map(|p| (p.x, p.y)),
                        them: partner.map(|p| (p.x, p.y)),
                    });
                    Some(TradeNote::new(
                        "Traded",
                        format!("Your trade with {them} went through"),
                    ))
                }
                TradeEvent::Cancelled { reason, .. } => {
                    Some(TradeNote::new("Trade cancelled", reason.clone()))
                }
                // The request card says it's still out.
                TradeEvent::NoReply(TradeAction::Initiate) => None,
                TradeEvent::NoReply(action) => Some(TradeNote::new("No answer", action.hint())),
                // The `error` that carried it is the reason, and nothing
                // else shows the server's errors yet.
                TradeEvent::Refused { reason, .. } => {
                    Some(TradeNote::new("Can't trade", reason.clone()))
                }
                TradeEvent::Requested { .. } | TradeEvent::Accepted { .. } => None,
            };
            if let Some(note) = note {
                self.trade_notes.push(note);
            }
        }
    }

    /// Keeps the draft true to the trade, and sends it once it settles.
    fn send_draft(&mut self, now: Instant) {
        self.draft
            .sync(self.state.open_trade(), self.state.inventory(), now);
        let Some((items, strands)) = self.draft.due(now, self.state.strands()) else {
            return;
        };
        tracing::info!(
            "trade: offering {} stacks and {strands} Strands",
            items.len()
        );
        let sent = self.state.trade_offer(now, items, strands);
        self.send_trade(sent);
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
            self.refresh_catalog(event);
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
                NetEvent::Rest(EXCHANGE_TAG, result) => {
                    let asked = self.exchanging.take();
                    let receipt = match result {
                        Ok(RestResponse::Deposit(d)) => Ok(Receipt::deposited(
                            d,
                            match &asked {
                                Some(Asked::Deposit(q)) => Some(q),
                                _ => None,
                            },
                        )),
                        Ok(RestResponse::Withdraw(w)) => Ok(Receipt::withdrew(
                            w,
                            match &asked {
                                Some(Asked::Withdraw(q)) => Some(q),
                                _ => None,
                            },
                        )),
                        Err(e) => Err(refusal(e)),
                        Ok(_) => Err("The server's answer didn't make sense".into()),
                    };
                    match receipt {
                        Ok(r) => {
                            if r.as_quoted() {
                                tracing::info!("exchange: {r:?}");
                            } else {
                                tracing::warn!("exchange: not as quoted: {r:?}, asked {asked:?}");
                            }
                            self.receipt = Some((r, now));
                        }
                        Err(why) => {
                            tracing::info!("exchange: refused: {why}");
                            self.exchange_refused = Some((why, now));
                        }
                    }
                }
                NetEvent::Rest(MARKET_TAG, result) => {
                    self.market_busy = false;
                    match result {
                        Ok(RestResponse::Order(order)) => tracing::info!(
                            "market: {} order {} is {}",
                            order.side.as_str(),
                            order.id,
                            order.status.as_str()
                        ),
                        Ok(RestResponse::Collected(_)) => {
                            self.net.rest(RestRequest::MarketBox, MARKET_READ_TAG);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            let why = refusal(e);
                            tracing::info!("market: refused: {why}");
                            self.market_refused = Some((why, now));
                        }
                    }
                }
                NetEvent::Rest(GAZETTE_TAG, result) => {
                    let ask = self.gazette_asked.take();
                    match (ask, result) {
                        (Some(ask), Ok(RestResponse::Gazette(answer))) => {
                            self.gazette.answered(ask, (**answer).clone())
                        }
                        (Some(_), Ok(RestResponse::Byline(byline))) => self.gazette.byline(byline.named),
                        (Some(ask), result) => {
                            if let Err(e) = result {
                                tracing::warn!("gazette: {ask:?} failed: {e}");
                            }
                            self.gazette.failed(ask);
                        }
                        (None, _) => {}
                    }
                }
                NetEvent::Rest(MARKET_READ_TAG, Ok(RestResponse::MarketSummary(summary))) => {
                    self.market_summary = Some(summary.clone());
                }
                NetEvent::Rest(MARKET_READ_TAG, Err(e)) => {
                    tracing::warn!("market: a refresh failed: {e}");
                }
                NetEvent::Rest(tag, Ok(RestResponse::MarketHistory(history))) if *tag > CHART_TAG => {
                    self.state.market_mut().set_history(tag - CHART_TAG, history);
                }
                NetEvent::Rest(tag, Err(e)) if *tag > CHART_TAG => {
                    tracing::warn!("market: the chart's history failed: {e}");
                }
                NetEvent::Rest(PAY_TAX_TAG, Ok(RestResponse::PayTax(paid))) => {
                    tracing::info!(
                        "plot: paid tax on {}, {} still owed, {} Strands left",
                        paid.plot.id,
                        paid.plot.tax_owed,
                        paid.new_balance
                    );
                    self.paying = None;
                }
                NetEvent::Rest(PAY_TAX_TAG, result) => {
                    let why = match result {
                        Err(e) => refusal(e),
                        Ok(_) => "The server's answer didn't make sense".into(),
                    };
                    tracing::info!("plot: tax payment refused: {why}");
                    self.pay_refused = Some((why, now));
                    if let Some(plot_id) = self.paying.take() {
                        self.net.rest(RestRequest::Plot { plot_id }, PLOT_TAG);
                    }
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
                // An edition may have come out while the link was down
                if matches!(connection, ConnectionState::Connected { .. }) {
                    self.gazette.connected();
                }
            }
            let trade_before = self.state.trade().clone();
            let changes = self.state.apply(event);
            self.note_trades(&trade_before, &changes, now);
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
                    StateEvent::MovementBlocked {
                        reason,
                        water,
                        x,
                        y,
                    } => {
                        self.blocked.push(Blocked {
                            x: *x,
                            y: *y,
                            reason: reason.clone(),
                            water: water.clone(),
                            toward: self.last_move.take(),
                        });
                    }
                    StateEvent::ExtractionCompleted { node_id, item } => {
                        self.yields.push((*node_id, item.clone()));
                    }
                    StateEvent::CraftingCompleted { item } => self.crafted.push(item.clone()),
                    StateEvent::CatalogUpdated => self.cache_catalog(),
                    StateEvent::StationPlaced { station_id, .. } => {
                        self.placed.insert(*station_id);
                    }
                    StateEvent::TaxDelinquent {
                        tax_owed,
                        forecloses_at,
                        ..
                    } => self.plot_notes.push(PlotNote::TaxOwed {
                        tax_owed: *tax_owed,
                        forecloses_at: *forecloses_at,
                    }),
                    StateEvent::PlotForeclosed {
                        tax_owed, refund, ..
                    } => self.plot_notes.push(PlotNote::Foreclosed {
                        tax_owed: *tax_owed,
                        refund: *refund,
                    }),
                    StateEvent::StationDisplaced { station_type, .. } => self
                        .plot_notes
                        .push(PlotNote::Displaced {
                            station_type: station_type.clone(),
                        }),
                    StateEvent::SubscriptionLapsed { until } => self
                        .plot_notes
                        .push(PlotNote::SubscriptionLapsed { until: *until }),
                    StateEvent::ToolBroke {
                        item_id, item_name, ..
                    } => self.broken_tools.push((*item_id, item_name.clone())),
                    StateEvent::AutomationPaused {
                        station_id,
                        station_type,
                        reason,
                    } => self
                        .automation_notes
                        .push((*station_id, station_type.clone(), reason.clone())),
                    StateEvent::AutomationProduced { station_id } => self.produced.push(*station_id),
                    StateEvent::Market(event) => self.note_market(event),
                    StateEvent::GazettePublished { day, headline, .. } => {
                        self.gazette.published(*day, headline)
                    }
                    // A tool slot refused: shown where a refused move is
                    StateEvent::ServerError {
                        message,
                        command,
                    } if capitol_core::automation::is_automation_command(command) => {
                        tracing::info!("station: {command} refused: {message}");
                        self.automation_refused = Some((message.clone(), now));
                    }
                    StateEvent::ServerError {
                        message,
                        command,
                    } if command == "install_tool" || command == "remove_tool" => {
                        tracing::info!("station: {command} refused: {message}");
                        self.move_refused = Some((message.clone(), now));
                    }
                    _ => {}
                }
            }
            self.crafting.observe(&changes, &self.state, now);
            self.note_crafting(now);
            if let Some(placing) = &mut self.placing {
                placing.observe(&changes, &self.state);
            }
            self.settle_placing();
            if let Some(step) = self.harvest.observe(&changes, &self.state, now) {
                self.step(now, step);
            }
            if let Some(step) = self.visit.observe(&changes, &self.state, now) {
                self.station_step(now, step);
            }
            self.station_notes.extend(self.visit.take_notes());
            if let Some(step) = self.approach.observe(&changes, &mut self.state, now) {
                self.trade_step(now, step);
            }
            self.note_approach();
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
        // The prediction walks on the land the server does (W4)
        self.motion.set_ground(terrain.ground().cloned());
        if self.in_session() {
            self.harvest.tick(now);
            self.visit.tick(now);
            self.station_notes.extend(self.visit.take_notes());
            self.crafting.tick(now);
            self.note_crafting(now);
            if let Some(placing) = &mut self.placing {
                placing.tick(now);
            }
            self.settle_placing();
            let trade_before = self.state.trade().clone();
            let changes = self.state.tick(now);
            self.note_trades(&trade_before, &changes, now);
            self.send_draft(now);
            self.tend_market(now);
            self.tend_gazette();
            for (request, tag) in self.poller.tick(now) {
                self.net.rest(request, tag);
            }
        }
    }

    /// Before sign-in the store is idle, so the login screen's `/health`
    /// asks for a newer catalog here, and its answer is kept. Once in the
    /// world, the store and the poller do both.
    fn refresh_catalog(&mut self, event: &NetEvent) {
        let NetEvent::Rest(tag, result) = event else {
            return;
        };
        match result {
            Ok(RestResponse::Health(health)) if !self.in_session() && !self.catalog_asked => {
                let current = self.state.catalog().version();
                if health
                    .catalog_version
                    .as_deref()
                    .is_some_and(|v| v != current)
                {
                    self.net.rest(RestRequest::Catalog, CATALOG_TAG);
                    self.catalog_asked = true;
                }
            }
            Ok(RestResponse::Catalog(catalog)) if *tag == CATALOG_TAG => {
                self.catalog_asked = false;
                // In the world the store takes it, and caches it, by itself.
                if !self.in_session() {
                    self.state.set_catalog(Catalog::new((**catalog).clone()));
                    self.cache_catalog();
                }
            }
            Err(e) if *tag == CATALOG_TAG => {
                tracing::warn!("catalog: fetch failed ({e}); the next health check asks again");
                self.catalog_asked = false;
            }
            _ => {}
        }
    }

    /// Keeps the catalog just fetched for the next start.
    fn cache_catalog(&self) {
        let cache = CatalogCache::new(CACHE_DIR, &self.server);
        let catalog = self.state.catalog();
        match cache.store(catalog) {
            Ok(()) => tracing::info!(
                "catalog: version {} cached at {}",
                catalog.version(),
                cache.path().display()
            ),
            Err(e) => tracing::warn!("catalog: couldn't write {}: {e}", cache.path().display()),
        }
    }

    /// Between a sign-in and a sign-out: the store's events are this player's.
    fn in_session(&self) -> bool {
        matches!(self.auth.phase(), Phase::Entering { .. } | Phase::InWorld)
    }

    fn reset_world(&mut self) {
        if std::mem::take(&mut self.used) {
            // The catalog is the server's, not the player's: it outlives a sign-out.
            let catalog = self.state.catalog().clone();
            self.state = GameState::new();
            self.state.set_catalog(catalog);
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
            self.paying = None;
            self.pay_refused = None;
            self.claimed.clear();
            self.plot_notes.clear();
            self.automation_notes.clear();
            self.produced.clear();
            self.automation_refused = None;
            self.exchanging = None;
            self.exchange_refused = None;
            self.receipt = None;
            self.market_asked = None;
            self.market_summary = None;
            self.market_busy = false;
            self.market_refused = None;
            self.market_notes.clear();
            self.gazette = capitol_core::gazette::Reader::new(None);
            self.gazette_asked = None;
            self.placing = None;
            self.placing_notes.clear();
            self.visit = Visit::default();
            self.station_notes.clear();
            self.approach = Approach::default();
            self.draft = Draft::default();
            self.trade_notes.clear();
            self.traded.clear();
            self.offer_changed = None;
        }
    }

    /// Keeps the placement's notes, and ends it once it's done.
    fn settle_placing(&mut self) {
        let Some(placing) = &mut self.placing else {
            return;
        };
        for note in placing.take_notes() {
            tracing::info!("place: {note:?}");
            self.placing_notes.push(note);
        }
        if placing.is_done() {
            self.placing = None;
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
                Effect::Connect(credentials) => self.net.connect(credentials),
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
/// in, and its refresh token (E0.6), with the server the token belongs to. A
/// token is only tried on that server. The access tokens files held before
/// E0.6, under `token`, are ignored.
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
const TOKEN: &str = "refresh_token";
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

    /// The email, and the refresh token if it belongs to `server`.
    pub fn remembered(&self, server: &str) -> Remembered {
        let refresh_token = match self.get(TOKEN_SERVER) {
            Some(s) if s == server => self.get(TOKEN),
            _ => None,
        };
        Remembered {
            email: self.get(EMAIL),
            refresh_token,
        }
    }

    pub fn remember(&mut self, server: &str, remembered: &Remembered) {
        self.set(EMAIL, remembered.email.as_deref());
        self.set(TOKEN, remembered.refresh_token.as_deref());
        self.set(TOKEN_SERVER, remembered.refresh_token.as_ref().map(|_| server));
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
            split: None,
            qualities: None,
        };
        session.move_item(there.clone());
        wait(&mut session, "the move", &in_slot(to, item.item_id));
        session.move_item(Move {
            target_slot: item.slot_index,
            ..there.clone()
        });
        wait(
            &mut session,
            "the move back",
            &in_slot(item.slot_index, item.item_id),
        );

        session.move_item(Move {
            target_slot: 999,
            ..there.clone()
        });
        wait(&mut session, "the refusal", &|s| {
            s.move_refused(Instant::now()).is_some()
        });
        let why = session.move_refused(Instant::now()).unwrap().to_owned();
        assert!(why.contains("out of bounds"), "{why}");

        session.move_item(Move {
            target_container_id: uuid::Uuid::from_u128(1), // nobody's
            ..there.clone()
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
            let c = craft::check(&r, st.crafting_input().unwrap(), st.crafting_output(), None);
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
        wait(&mut session, "me and my balance", &|s| {
            let st = s.state();
            st.local_player().is_some() && st.strands().is_some()
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
        let offer = Offer::new(&plot, st.me(), st.strands());
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
        session.pick_plot(capitol_core::protocol::rules::CENTER_X, capitol_core::protocol::rules::CENTER_Y);
        wait(&mut session, "nothing there", &|s| {
            s.state().picked_plot().is_none()
        });
        assert_eq!(session.take_plot_notes(), vec![PlotNote::NoneThere]);
        session.log_out();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Against the local stack, the exchange panel's calls end to end:
    /// deposit every fiber stack, withdraw some back, and have the server
    /// refuse a withdrawal you can't afford. Each answer must be the
    /// panel's preview to the Strand, and the balance and inventory must
    /// follow. The account is `exchange1@bots.capitol.local`; stock it with
    /// `capitol-bot --prefix exchange --gather 12`.
    /// The market panel's calls, end to end (E3.5): a bot lists fiber, the
    /// session buys it at the price the preview quoted, hears the fill,
    /// collects it from the market box, then rests a bid and cancels it.
    /// Every fee is checked against the preview's.
    #[test]
    #[ignore = "needs the local stack"]
    fn a_market_order_is_placed_filled_collected_and_cancelled() {
        use capitol_core::bot::{Account, Bot};
        use capitol_core::market::{self, Draft};
        use capitol_core::net::NetConfig;
        use capitol_core::protocol::rest::PlaceOrder;
        use capitol_core::protocol::rules::market::{GradeRule, OrderSide, OrderStatus};

        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 100_000_000;
        // A price nobody else on a local stack quotes
        let price = 5 + (stamp as i64 % 15);

        // The seller: a bot with four of the exchange's fiber, three listed
        let mut seller = Bot::sign_in(
            NetConfig::with_base_url(&server),
            &Account::numbered(&format!("mseller{stamp}"), 1),
        )
        .expect("seller signed in");
        seller.withdraw_fiber(4).expect("withdrew fiber");
        let stack = seller
            .state()
            .inventory()
            .and_then(|inv| inv.items().find(|i| i.item_type == "fiber").map(|i| i.item_id))
            .expect("fiber in the inventory");
        let ask = seller
            .place_order(PlaceOrder {
                side: OrderSide::Sell,
                item_type: "fiber".into(),
                price,
                quantity: 3,
                item_id: Some(stack),
                qualities: None,
                min_grade: None,
                grade_rule: None,
            })
            .expect("placed the sell");

        // The buyer: registered by a bot, then signed in through the session
        let buyer = Account::numbered(&format!("mbuyer{stamp}"), 1);
        drop(Bot::sign_in(NetConfig::with_base_url(&server), &buyer).expect("buyer registered"));
        let dir = std::env::temp_dir().join(format!("capitol-market-{}", std::process::id()));
        let mut session = Session::start(&server, SessionFile::load(dir.join("s.toml"))).unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        session.log_in(&buyer.email, &buyer.password);
        let mut wait = |session: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done(session) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                session.update(Instant::now(), &mut terrain);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&mut session, "the inventory and the balance", &|s| {
            s.state().inventory().is_some() && s.state().strands().is_some()
        });

        // Open the panel on fiber: the summary comes, and the book shows the ask
        session.set_market_open(true);
        session.watch_market(Some("fiber"));
        wait(&mut session, "the summary and the ask", &|s| {
            s.market_summary().is_some()
                && s.state().market().book("fiber").is_some_and(|b| {
                    b.side(OrderSide::Sell).any(|l| l.price == price)
                })
        });

        // Buy the three at the ask's price: the preview says they fill now
        let draft = Draft {
            side: OrderSide::Buy,
            item_type: "fiber".into(),
            price,
            quantity: 3,
            min_grade: "F".into(),
            grade_rule: GradeRule::Average,
        };
        let st = session.state();
        let mine = st.market().open_orders();
        let quote = market::quote(&draft, None, st.market().book("fiber"), &mine, st.strands())
            .expect("a buy the buyer can afford");
        eprintln!("buy preview: {quote:?}");
        assert!(quote.fills.quantity >= 3, "{quote:?}");
        session.place_order(draft.to_place(None));
        assert!(session.market_busy());
        wait(&mut session, "the buy's answer", &|s| !s.market_busy());
        assert_eq!(session.market_refused(Instant::now()), None);
        wait(&mut session, "the fill", &|s| {
            s.state()
                .market()
                .open_orders()
                .is_empty()
        });
        let notes = session.take_market_notes();
        eprintln!("toasts: {notes:?}");
        assert!(notes.iter().any(|n| n.title == "Order filled"), "{notes:?}");
        let bought = session
            .state()
            .market()
            .trades("fiber")
            .find(|t| t.sell_order_id == ask.id)
            .map(|t| t.buy_order_id)
            .expect("the trade on the channel");
        let order = session.state().market().order(bought).cloned().unwrap();
        assert_eq!((order.status, order.fee), (OrderStatus::Filled, quote.fee));
        wait(&mut session, "the balance the preview said", &|s| {
            s.state().strands() == quote.balance_after
        });

        // Collect it from the market box
        wait(&mut session, "the fiber in the box", &|s| {
            s.state().market_box().is_some_and(|b| b.items().any(|i| i.item_type == "fiber"))
        });
        session.collect_market_box();
        wait(&mut session, "the collect", &|s| !s.market_busy());
        assert_eq!(session.market_refused(Instant::now()), None);
        wait(&mut session, "the fiber in the inventory", &|s| {
            s.state()
                .inventory()
                .is_some_and(|inv| inv.items().any(|i| i.item_type == "fiber" && i.quantity >= 3))
        });

        // Rest a bid under every ask, see it on the book, cancel it
        let low = Draft {
            price: 1,
            quantity: 2,
            ..draft
        };
        let st = session.state();
        let before = st.strands().unwrap();
        let rest = market::quote(&low, None, st.market().book("fiber"), &[], st.strands()).unwrap();
        assert_eq!(rest.fills.quantity, 0);
        session.place_order(low.to_place(None));
        wait(&mut session, "the bid on the book", &|s| {
            !s.market_busy() && s.state().market().open_orders().len() == 1
        });
        let id = session.state().market().open_orders()[0].id;
        assert_eq!(session.state().market().order(id).unwrap().fee, rest.fee);
        session.cancel_order(id);
        wait(&mut session, "the cancel", &|s| {
            !s.market_busy() && s.state().market().open_orders().is_empty()
        });
        wait(&mut session, "the escrow back, less the fee", &|s| {
            s.state().strands() == Some(before - rest.fee)
        });
        session.log_out();
        drop(seller);
    }

    /// E3.6 against the local stack: two bots trade fiber twice, then the
    /// buyer signs in through the session and opens the fiber chart. Its
    /// history holds both trades, marked as the buyer's, and a third trade,
    /// between the seller and another bot, folds in live.
    #[test]
    #[ignore = "needs the local stack"]
    fn the_chart_shows_the_trades_and_marks_yours() {
        use capitol_core::bot::{Account, Bot};
        use capitol_core::market::{Bucket, ChartKey};
        use capitol_core::net::NetConfig;
        use capitol_core::protocol::rest::PlaceOrder;
        use capitol_core::protocol::rules::market::{GradeRule, OrderSide};

        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 100_000_000;
        let price = 5 + (stamp as i64 % 15);
        let bot = |name: &str| {
            let mut b = Bot::sign_in(
                NetConfig::with_base_url(&server),
                &Account::numbered(&format!("{name}{stamp}"), 1),
            )
            .expect("signed in");
            b.go_to_exchange().expect("at the Exchange hall");
            b
        };
        let buy = |b: &mut Bot, quantity: i32| {
            b.place_order(PlaceOrder {
                side: OrderSide::Buy,
                item_type: "fiber".into(),
                price,
                quantity,
                item_id: None,
                qualities: None,
                min_grade: Some("F".into()),
                grade_rule: Some(GradeRule::Average),
            })
            .expect("placed a buy");
            let deadline = Instant::now() + Duration::from_secs(20);
            while !b.my_orders().expect("orders").is_empty() {
                assert!(Instant::now() < deadline, "timed out waiting for a fill");
                std::thread::sleep(Duration::from_millis(200));
            }
        };

        // The seller lists six; the buyer takes two, then three
        let mut seller = bot("cseller");
        seller.withdraw_fiber(6).expect("withdrew fiber");
        let stack = seller
            .state()
            .inventory()
            .and_then(|inv| inv.items().find(|i| i.item_type == "fiber").map(|i| i.item_id))
            .expect("fiber in the inventory");
        seller
            .place_order(PlaceOrder {
                side: OrderSide::Sell,
                item_type: "fiber".into(),
                price,
                quantity: 6,
                item_id: Some(stack),
                qualities: None,
                min_grade: None,
                grade_rule: None,
            })
            .expect("placed the sell");
        let buyer = Account::numbered(&format!("cbuyer{stamp}"), 1);
        let mut buying = Bot::sign_in(NetConfig::with_base_url(&server), &buyer).expect("buyer");
        buying.go_to_exchange().expect("at the hall");
        buy(&mut buying, 2);
        buy(&mut buying, 3);
        drop(buying);

        // The buyer, through the session, opens the chart
        let dir = std::env::temp_dir().join(format!("capitol-chart-{}", std::process::id()));
        let mut session = Session::start(&server, SessionFile::load(dir.join("s.toml"))).unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        session.log_in(&buyer.email, &buyer.password);
        let mut wait = |session: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done(session) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                session.update(Instant::now(), &mut terrain);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&mut session, "the world", &|s| s.state().inventory().is_some());
        session.set_market_open(true);
        session.watch_market(Some("fiber"));
        let key = ChartKey {
            item_type: "fiber".into(),
            bucket: Bucket::Hour,
            grade: None,
        };
        session.chart_market(Some(key.clone()));
        wait(&mut session, "the history", &|s| s.state().market().history().is_some());
        let history = session.state().market().history().unwrap().clone();
        assert_eq!(history.key, key);
        let ours: Vec<i32> = history
            .recent
            .iter()
            .filter(|t| t.price == price && history.is_mine(t) == Some(OrderSide::Buy))
            .map(|t| t.quantity)
            .collect();
        assert_eq!(ours, [3, 2], "both fills, newest first, marked yours");
        let candle = history.candles.last().expect("this hour's candle").clone();
        eprintln!("last candle: {candle:?}");
        assert!(candle.volume >= 5);

        // Wait for the book, so the trade channel is open, then another bot
        // takes one more: it folds into the chart, not as yours
        wait(&mut session, "the fiber book", &|s| s.state().market().book("fiber").is_some());
        let mut other = bot("cother");
        buy(&mut other, 1);
        wait(&mut session, "the third trade", &|s| {
            s.state().market().history().is_some_and(|h| h.recent.len() == history.recent.len() + 1)
        });
        let now = session.state().market().history().unwrap();
        let third = &now.recent[0];
        assert_eq!((third.quantity, now.is_mine(third)), (1, None));
        let last = now.candles.last().unwrap();
        if last.at == candle.at {
            assert_eq!((last.volume, last.close), (candle.volume + 1, price));
        }
        session.log_out();
        drop((seller, other));
    }

    #[test]
    #[ignore = "needs the local stack and Exchange1 holding fiber"]
    fn an_exchange_charges_what_it_previews() {
        use capitol_core::exchange::{self, Receipt, WithdrawQuote};

        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let dir = std::env::temp_dir().join(format!("capitol-exchange-{}", std::process::id()));
        let mut session = Session::start(&server, SessionFile::load(dir.join("s.toml"))).unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        session.log_in("exchange1@bots.capitol.local", "exchange-1-capitol-bot");
        let mut wait = |session: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done(session) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                session.update(Instant::now(), &mut terrain);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&mut session, "the inventory and the balance", &|s| {
            s.state().inventory().is_some() && s.state().strands().is_some()
        });

        // Deposit every fiber stack.
        let st = session.state();
        let inv = st.inventory().unwrap();
        let all = exchange::fiber_stacks(inv)
            .iter()
            .map(|i| i.item_id)
            .collect();
        let quote = exchange::deposit(inv, &all, st.strands()).expect("stock Exchange1 with fiber");
        let ids = exchange::deposit_ids(inv, &all);
        eprintln!("deposit preview: {quote:?}");
        session.deposit(ids, quote);
        assert!(session.exchanging());
        wait(&mut session, "the deposit", &|s| !s.exchanging());
        assert_eq!(session.exchange_refused(Instant::now()), None);
        let (receipt, _) = session.receipt(Instant::now()).unwrap();
        eprintln!("deposit receipt: {receipt:?}");
        assert!(receipt.as_quoted(), "{receipt:?} against {quote:?}");
        wait(&mut session, "the fiber to go", &|s| {
            exchange::fiber_stacks(s.state().inventory().unwrap()).is_empty()
        });
        assert_eq!(session.state().strands(), quote.balance_after);

        // Withdraw some back: it lands in an empty slot at quality 50.
        let quantity = 5.min(exchange::max_withdraw(quote.balance_after.unwrap()));
        let st = session.state();
        let quote = exchange::withdraw(st.inventory(), quantity, st.strands()).unwrap();
        eprintln!("withdraw preview: {quote:?}");
        session.withdraw(quote);
        wait(&mut session, "the withdrawal", &|s| !s.exchanging());
        let (receipt, _) = session.receipt(Instant::now()).unwrap();
        eprintln!("withdraw receipt: {receipt:?}");
        assert!(matches!(
            receipt,
            Receipt::Withdrew {
                as_quoted: true,
                ..
            }
        ));
        wait(&mut session, "the fiber to land", &|s| {
            !exchange::fiber_stacks(s.state().inventory().unwrap()).is_empty()
        });
        let st = session.state();
        let stacks = exchange::fiber_stacks(st.inventory().unwrap());
        assert_eq!(
            (stacks[0].quantity, stacks[0].quality),
            (quantity as i32, 50)
        );
        if let exchange::Lands::NewSlot(slot) = quote.lands {
            assert_eq!(stacks[0].slot_index, slot as i32);
        }
        wait(&mut session, "the balance", &|s| {
            s.state().strands() == quote.balance_after
        });

        // More than you hold: the core would refuse it, so ask the server
        // directly, and keep its reason.
        let balance = session.state().strands().unwrap();
        let fiber = balance + 10;
        let (strands, fee) = capitol_core::protocol::rules::withdraw_quote(fiber);
        session.withdraw(WithdrawQuote {
            fiber,
            fee,
            strands,
            balance_after: None,
            lands: exchange::Lands::OnStack {
                slot: None,
                quality_after: None,
            },
        });
        wait(&mut session, "the refusal", &|s| !s.exchanging());
        let why = session.exchange_refused(Instant::now()).unwrap();
        eprintln!("refused: {why}");
        assert!(why.contains("Insufficient"), "{why}");
        assert_eq!(session.state().strands(), Some(balance));
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
                refresh_token: Some("t0ken".into()),
            },
        );

        let file = SessionFile::load(&path);
        assert_eq!(file.server().as_deref(), Some("http://a:3000"));
        assert_eq!(
            file.remembered("http://a:3000").refresh_token.as_deref(),
            Some("t0ken")
        );
        let elsewhere = file.remembered("http://b:3000");
        assert_eq!(elsewhere.refresh_token, None);
        assert_eq!(elsewhere.email.as_deref(), Some("ada@capitol.local"));

        let mut file = file;
        file.remember(
            "http://a:3000",
            &Remembered {
                email: Some("ada@capitol.local".into()),
                refresh_token: None,
            },
        );
        assert_eq!(
            SessionFile::load(&path).remembered("http://a:3000").refresh_token,
            None
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Against the local stack, the station panel's rules end to end (P6.3):
    /// walk to a workbench and open it, Fill its box, craft a tool handle
    /// there, put the box back, open a chest and move the handle into it and
    /// out, walk out of reach and see it close, and take the chest down.
    ///
    /// Needs a workbench and a storage chest owned by `Station1`, each with
    /// its `station_inventory` container, inserted before the server started
    /// (the tick only learns stations at startup or from `place_station`).
    /// The SQL is under Stations in `flint-client/CLAUDE.md`, with the two
    /// sticks the handle takes: fallen branches have no nodes until E2.3
    /// (#162). A bot gathers the fiber and twists the rope first. The chest is
    /// gone afterwards.
    #[test]
    #[ignore = "needs the local stack, a workbench and chest for Station1; takes a minute or two"]
    fn a_station_opens_crafts_stores_closes_and_comes_down() {
        use capitol_core::bot::{Account, Bot};
        use capitol_core::catalog::container_type::CRAFTING_OUTPUT;
        use capitol_core::craft::{self, Phase};
        use capitol_core::state::Container;

        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let account = Account::numbered("station", 1);
        {
            let mut bot = Bot::sign_in(NetConfig::with_base_url(&server), &account).unwrap();
            // Crafted rope lands in the crafting output, and stays there.
            let rope = |bot: &Bot| {
                bot.count("player_inventory", "rope") + bot.count("crafting_output", "rope")
            };
            while rope(&bot) < 1 {
                bot.gather("fiber", 2).unwrap();
                bot.craft("rope_basic").unwrap();
            }
            // Already held, from the SQL, until branches grow (E2.3)
            bot.gather("sticks", 2).unwrap();
            // Back by the stations: the gathering may have wandered.
            bot.walk_to(4006.0, 7350.0).unwrap();
        }

        let dir = std::env::temp_dir().join(format!("capitol-station-{}", std::process::id()));
        let mut session = Session::start(&server, SessionFile::load(dir.join("s.toml"))).unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        session.log_in(&account.email, &account.password);
        let mut wait = |session: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(45);
            while !done(session) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                session.update(Instant::now(), &mut terrain);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&mut session, "the boxes, recipes and stations", &|s| {
            let st = s.state();
            st.inventory().is_some()
                && st.crafting_output().is_some()
                && !st.recipes().is_empty()
                && st.local_player().is_some()
                && st.stations().len() >= 2
        });
        let mine = |s: &Session, kind: &str| {
            let st = s.state();
            st.stations()
                .values()
                .find(|x| x.station_type == kind && Some(x.owner_id) == st.me())
                .map(|x| x.id)
                .unwrap_or_else(|| panic!("no {kind} of Station1 nearby: see the doc comment"))
        };
        let (bench, chest) = (mine(&session, "workbench"), mine(&session, "storage_chest"));
        let output = |s: &Session| -> Container {
            s.state()
                .containers()
                .values()
                .find(|c| c.container_type == CRAFTING_OUTPUT)
                .unwrap()
                .clone()
        };
        let station_box = |s: &Session| s.state().station_container().cloned();
        let open = |s: &Session| s.state().open_station().map(|o| o.station_id);
        let emptied = |session: &mut Session| {
            let shifts = craft::empty_into(
                &station_box(session).unwrap(),
                session.state().inventory().unwrap(),
            );
            session.move_items(&shifts);
        };

        // A clear output, so the handle has somewhere to go.
        let shifts = craft::empty_into(&output(&session), session.state().inventory().unwrap());
        session.move_items(&shifts);
        wait(&mut session, "a clear output", &|s| {
            output(s).used_slots() == 0
        });

        // The workbench, out of reach: walk, then it opens.
        session.open_station(Instant::now(), bench);
        wait(&mut session, "the workbench to open", &|s| {
            open(s) == Some(bench) && station_box(s).is_some()
        });
        emptied(&mut session);
        wait(&mut session, "an empty workbench", &|s| {
            station_box(s).is_some_and(|b| b.used_slots() == 0)
        });
        let handle = session
            .state()
            .recipe("tool_handle")
            .expect("the tool_handle content")
            .clone();
        assert_eq!(handle.required_station_type.as_deref(), Some("workbench"));
        let shifts = craft::fill(
            &handle,
            session.state().inventory().unwrap(),
            &station_box(&session).unwrap(),
        );
        session.move_items(&shifts);
        let ready = |s: &Session| {
            let st = s.state();
            let r = st.recipe("tool_handle").unwrap();
            station_box(s).is_some_and(|b| craft::check(r, &b, st.crafting_output(), None).ready())
        };
        wait(&mut session, "sticks and rope on the workbench", &ready);

        session.craft_at_station(Instant::now(), "tool_handle");
        wait(&mut session, "the handle to start", &|s| {
            s.state().is_crafting()
        });
        wait(&mut session, "the handle", &|s| {
            *s.crafting().phase() == Phase::Idle && !s.state().is_crafting()
        });
        assert_eq!(craft::count(&output(&session), "tool_handle"), 1);
        assert_eq!(session.take_crafted().len(), 1);
        let left = |s: &Session, t: &str| station_box(s).map_or(0, |b| craft::count(&b, t));
        eprintln!(
            "after the craft the workbench holds {} rope, {} sticks",
            left(&session, "rope"),
            left(&session, "sticks")
        );

        // Put the workbench's box back, so nothing is left on it.
        emptied(&mut session);
        wait(&mut session, "the workbench emptied", &|s| {
            station_box(s).is_some_and(|b| b.used_slots() == 0)
        });

        // The chest: in it and out again.
        session.open_station(Instant::now(), chest);
        wait(&mut session, "the chest to open", &|s| {
            open(s) == Some(chest) && station_box(s).is_some()
        });
        let kit = output(&session).items().next().unwrap().clone();
        let chest_box = station_box(&session).unwrap();
        // Into an empty slot, so nothing comes back out in a swap.
        session.move_item(Move {
            item_id: kit.item_id,
            target_container_id: chest_box.id,
            target_slot: chest_box.first_empty().expect("room in the chest") as i32,
            split: None,
            qualities: None,
        });
        wait(&mut session, "the kit in the chest", &|s| {
            station_box(s).is_some_and(|b| b.find(kit.item_id).is_some())
        });
        let inventory = session.state().inventory().unwrap().clone();
        session.move_item(Move {
            item_id: kit.item_id,
            target_container_id: inventory.id,
            target_slot: inventory.first_empty().unwrap() as i32,
            split: None,
            qualities: None,
        });
        wait(&mut session, "the kit back out", &|s| {
            s.state().inventory().unwrap().find(kit.item_id).is_some()
        });

        // Walk away: it closes.
        let me = session.state().local_player().unwrap().clone();
        session.move_to(Instant::now(), me.x + 120.0, me.y);
        wait(&mut session, "the chest to close", &|s| open(s).is_none());

        // Back, empty it as the panel asks, and take the chest down.
        session.open_station(Instant::now(), chest);
        wait(&mut session, "the chest to open again", &|s| {
            open(s) == Some(chest) && station_box(s).is_some()
        });
        emptied(&mut session);
        wait(&mut session, "the chest emptied", &|s| {
            station_box(s).is_some_and(|b| b.used_slots() == 0)
        });
        session.remove_station(chest);
        wait(&mut session, "the chest to come down", &|s| {
            s.state().station(chest).is_none() && open(s).is_none()
        });
        session.log_out();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A station works on its own, set up as the Automation tab sets it up
    /// (E4.4): two tool handles, made and logged, the run finished; then
    /// forever, until the box runs dry and it pauses with a notice; then
    /// Stop. Set up `Automate1` (`capitol-bot --prefix automate --gather 0`)
    /// with a workbench of its own by the spawn and 4 sticks and 2 rope in
    /// its box, then restart the backend so the tick knows the station:
    ///
    /// ```sql
    /// with s as (insert into stations (id, station_type, owner_id, position_x, position_y)
    ///   select gen_random_uuid(), 'workbench', id, 6000, 9330 from players where name = 'Automate1'
    ///   returning id),
    /// c as (insert into containers (id, container_type, owner_id, station_id)
    ///   select gen_random_uuid(), 'station_inventory', null, id from s returning id)
    /// insert into items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
    ///   select gen_random_uuid(), t, 50, q, null, c.id, k from c,
    ///   (values ('sticks', 4, 0), ('rope', 2, 1)) v(t, q, k);
    /// ```
    #[test]
    #[ignore = "needs the local stack and Automate1's stocked workbench; takes a minute"]
    fn a_station_runs_on_its_own_then_waits() {
        use capitol_core::automation::{Draft, Status};
        use capitol_core::bot::Account;
        use capitol_core::craft;
        use capitol_core::protocol::rest::PauseReason;

        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let account = Account::numbered("automate", 1);
        let dir = std::env::temp_dir().join(format!("capitol-automate-{}", std::process::id()));
        let mut session = Session::start(&server, SessionFile::load(dir.join("s.toml"))).unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        session.log_in(&account.email, &account.password);
        let mut wait = |session: &mut Session, what: &str, done: &dyn Fn(&Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(60);
            while !done(session) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                session.update(Instant::now(), &mut terrain);
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait(&mut session, "the recipes and stations", &|s| {
            let st = s.state();
            !st.recipes().is_empty() && st.local_player().is_some() && !st.stations().is_empty()
        });
        let bench = {
            let st = session.state();
            st.stations()
                .values()
                .find(|x| x.station_type == "workbench" && Some(x.owner_id) == st.me())
                .map(|x| x.id)
                .expect("no workbench of Automate1's nearby: see the doc comment")
        };
        let status = |s: &Session| {
            Status::of(s.state().station(bench).and_then(|b| b.automation.as_ref()))
        };
        let logged = |s: &Session| s.state().station(bench).map_or(0, |b| b.production.len());

        // Open, as the panel is: its box is where it works.
        session.open_station(Instant::now(), bench);
        wait(&mut session, "the workbench to open", &|s| {
            s.state().open_station().map(|o| o.station_id) == Some(bench)
                && s.state().station_container().is_some()
        });
        let held = |s: &Session, t: &str| s.state().station_container().map_or(0, |b| craft::count(b, t));
        assert!(
            held(&session, "sticks") >= 4 && held(&session, "rope") >= 2,
            "stock the workbench's box: see the doc comment"
        );

        // Two handles, as the tab's Count sets it.
        let before = logged(&session);
        let draft = Draft {
            recipe_id: Some("tool_handle".into()),
            forever: false,
            count: 2,
        };
        session.start_automation(bench, &draft);
        wait(&mut session, "it to start", &|s| {
            matches!(status(s), Status::Running | Status::Finished)
        });
        wait(&mut session, "two handles and the run finished", &|s| {
            status(s) == Status::Finished && logged(s) >= before + 2
        });
        assert_eq!(session.take_produced().iter().filter(|&&id| id == bench).count(), 2);
        let made = session.state().station(bench).unwrap().production.clone();
        assert!(made.iter().rev().take(2).all(|p| p.item_type == "tool_handle"));
        // The box's update follows each craft's notice
        wait(&mut session, "both handles in its own box", &|s| held(s, "tool_handle") == 2);

        // Forever: it makes what it can, then waits for more and says so.
        let forever = Draft {
            forever: true,
            ..draft
        };
        session.start_automation(bench, &forever);
        wait(&mut session, "it to run dry", &|s| {
            matches!(status(s), Status::Paused(PauseReason::MissingInput { .. }))
        });
        wait(&mut session, "the pause's notice", &|s| !s.automation_notes.is_empty());
        let notes = session.take_automation_notes();
        assert!(notes.iter().any(|(id, t, r)| *id == bench
            && t == "workbench"
            && matches!(r, PauseReason::MissingInput { .. })));

        session.stop_automation(bench);
        wait(&mut session, "it to stop", &|s| status(s) == Status::Stopped);
        assert!(session.automation_refused(Instant::now()).is_none());
        session.log_out();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Two clients trade through the rules the trade window uses: a
    /// shift-click's request, drafts that send themselves, a changed offer
    /// resetting the accepts, rope for Strands, and a disconnect mid-trade.
    /// Stock both accounts first: `capitol-bot --prefix trade --accounts 2`
    /// makes Trade1 and Trade2 and twists each some rope.
    #[test]
    #[ignore = "needs the local stack, and Trade1 and Trade2 holding rope"]
    fn two_clients_trade_rope_for_strands() {
        use capitol_core::state::{Trade, PARTNER_LEFT};
        use std::cell::RefCell;
        let server = std::env::var("CAPITOL_SERVER").unwrap_or(DEFAULT_SERVER.into());
        let dir = std::env::temp_dir().join(format!("capitol-trade-{}", std::process::id()));
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let start = |n: usize| {
            let mut s =
                Session::start(&server, SessionFile::load(dir.join(format!("s{n}.toml")))).unwrap();
            s.log_in(
                &format!("trade{n}@bots.capitol.local"),
                &format!("trade-{n}-capitol-bot"),
            );
            s
        };
        let (mut ada, mut bea) = (start(1), start(2));
        let mut terrain = TerrainSource::offline(&fixture).unwrap();
        let notes: RefCell<Vec<TradeNote>> = RefCell::new(Vec::new());
        let mut wait = |ada: &mut Session,
                        bea: &mut Session,
                        what: &str,
                        done: &dyn Fn(&Session, &Session) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !done(ada, bea) {
                let held = |s: &Session| {
                    let st = s.state();
                    let inv: Vec<_> = st
                        .inventory()
                        .map(|c| {
                            c.items()
                                .map(|i| (i.item_type.clone(), i.quantity))
                                .collect()
                        })
                        .unwrap_or_default();
                    format!(
                        "{:?} strands {:?} holding {inv:?}",
                        st.trade(),
                        st.strands()
                    )
                };
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {what}
Ada: {}
Bea: {}",
                    held(ada),
                    held(bea)
                );
                let now = Instant::now();
                ada.update(now, &mut terrain);
                bea.update(now, &mut terrain);
                notes.borrow_mut().extend(ada.take_trade_notes());
                bea.take_trade_notes();
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let ready = |s: &Session| s.state().inventory().is_some() && s.state().strands().is_some();
        wait(&mut ada, &mut bea, "both in the world", &|a, b| {
            ready(a)
                && ready(b)
                && b.state()
                    .me()
                    .is_some_and(|id| a.state().player(id).is_some())
        });
        // A craft's rope waits in the output slot: take it into the inventory.
        for s in [&mut ada, &mut bea] {
            let st = s.state();
            let (Some(out), Some(inv)) = (st.crafting_output(), st.inventory()) else {
                continue;
            };
            let moves: Vec<Move> = out
                .items()
                .zip(inv.slots.iter().enumerate().filter(|(_, i)| i.is_none()))
                .map(|(item, (slot, _))| Move {
                    item_id: item.item_id,
                    target_container_id: inv.id,
                    target_slot: slot as i32,
                    split: None,
                    qualities: None,
                })
                .collect();
            for m in moves {
                s.move_item(m);
            }
        }
        wait(&mut ada, &mut bea, "the rope in hand", &|a, b| {
            [a, b].iter().all(|s| {
                s.state()
                    .crafting_output()
                    .is_some_and(|c| c.items().next().is_none())
            })
        });
        let rope = |s: &Session| -> i32 {
            s.state()
                .inventory()
                .map(|c| {
                    c.items()
                        .filter(|i| i.item_type == "rope")
                        .map(|i| i.quantity)
                        .sum()
                })
                .unwrap_or(0)
        };
        let ada_rope = rope(&ada);
        assert!(
            ada_rope > 0,
            "stock Trade1 with rope: capitol-bot --prefix trade --accounts 2"
        );
        let (ada_strands, bea_strands, bea_rope) = (
            ada.state().strands().unwrap(),
            bea.state().strands().unwrap(),
            rope(&bea),
        );
        let bea_id = bea.state().me().unwrap();

        // Ada shift-clicks Bea: asked at once, or after a walk over.
        assert!(ada.trade_with(Instant::now(), bea_id));
        wait(&mut ada, &mut bea, "Bea to hear", &|_, b| {
            matches!(b.state().trade(), Trade::Incoming { .. })
        });
        bea.answer_trade(Instant::now(), true);
        wait(&mut ada, &mut bea, "the trade to open", &|a, b| {
            a.state().open_trade().is_some() && b.state().open_trade().is_some()
        });

        // Ada offers her rope; Bea offers 15 Strands. Neither presses a button.
        let stack = ada
            .state()
            .inventory()
            .unwrap()
            .items()
            .find(|i| i.item_type == "rope")
            .cloned()
            .unwrap();
        ada.draft_mut().add(&stack, Instant::now());
        bea.draft_mut().set_strands(15, Instant::now());
        let settled = |s: &Session| {
            s.state()
                .open_trade()
                .is_some_and(|o| !s.draft().unsent() && s.draft().confirmed(&o.mine))
                && s.state().trade_waiting().is_none()
        };
        wait(&mut ada, &mut bea, "both drafts confirmed", &|a, b| {
            settled(a)
                && settled(b)
                && a.state().open_trade().unwrap().theirs.strands == 15
                && b.state().open_trade().unwrap().theirs.items.len() == 1
        });

        // Bea accepts; then Ada changes her offer, and both accepts reset.
        bea.accept_trade(Instant::now());
        wait(&mut ada, &mut bea, "Bea's accept", &|a, _| {
            a.state().open_trade().unwrap().they_accepted
        });
        ada.draft_mut().set_strands(1, Instant::now());
        wait(
            &mut ada,
            &mut bea,
            "the change to reset the accepts",
            &|a, b| {
                settled(a)
                    && !a.state().open_trade().unwrap().they_accepted
                    && b.state().open_trade().unwrap().theirs.strands == 1
            },
        );
        assert!(ada
            .offer_changed(Instant::now())
            .is_some_and(|(theirs, _)| !theirs));
        assert!(bea
            .offer_changed(Instant::now())
            .is_some_and(|(theirs, _)| theirs));

        // Both accept, and it goes through.
        bea.accept_trade(Instant::now());
        wait(&mut ada, &mut bea, "Bea's second accept", &|a, _| {
            a.state().open_trade().unwrap().they_accepted
        });
        ada.accept_trade(Instant::now());
        wait(&mut ada, &mut bea, "the swap", &|a, b| {
            a.state().trade() == &Trade::Idle
                && a.state().strands() == Some(ada_strands + 15 - 1)
                && b.state().strands() == Some(bea_strands - 15 + 1)
                && rope(a) == ada_rope - stack.quantity
                && rope(b) == bea_rope + stack.quantity
        });
        assert!(
            notes.borrow().iter().any(|n| n.title == "Traded"),
            "{notes:?}"
        );
        let traded = ada.take_traded();
        assert_eq!(traded.len(), 1, "{traded:?}");
        assert!(ada.draft().is_empty());

        // Again, and Bea drops mid-trade: Ada's window closes, nothing moves.
        let (ada_strands, ada_rope) = (ada.state().strands(), rope(&ada));
        assert!(ada.trade_with(Instant::now(), bea_id));
        wait(&mut ada, &mut bea, "Bea to hear again", &|_, b| {
            matches!(b.state().trade(), Trade::Incoming { .. })
        });
        bea.answer_trade(Instant::now(), true);
        wait(&mut ada, &mut bea, "the second trade", &|a, b| {
            a.state().open_trade().is_some() && b.state().open_trade().is_some()
        });
        bea.draft_mut().set_strands(5, Instant::now());
        wait(&mut ada, &mut bea, "Bea's offer", &|a, _| {
            a.state().open_trade().unwrap().theirs.strands == 5
        });
        notes.borrow_mut().clear();
        bea.log_out();
        wait(&mut ada, &mut bea, "the cancel", &|a, _| {
            a.state().trade() == &Trade::Idle
        });
        assert!(
            notes
                .borrow()
                .iter()
                .any(|n| n.title == "Trade cancelled" && n.body == PARTNER_LEFT),
            "{notes:?}"
        );
        // Nothing was held in escrow, so a refetch shows the same.
        std::thread::sleep(Duration::from_secs(1));
        wait(&mut ada, &mut bea, "a settled inventory", &|a, _| {
            a.state().strands() == ada_strands && rope(a) == ada_rope
        });
        ada.log_out();
        let _ = std::fs::remove_dir_all(dir);
    }
}
