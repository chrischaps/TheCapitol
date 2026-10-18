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
use capitol_core::exchange::{Asked, DepositQuote, Receipt, WithdrawQuote};
use capitol_core::harvest::{Harvest, Note, Step};
use capitol_core::motion::{Motion, Pose};
use capitol_core::net::{
    ConnectionState, Net, NetConfig, NetEvent, RestError, RestRequest, RestResponse, Tag,
};
use capitol_core::poll::{Fetch, Poller};
use capitol_core::protocol::rest::{
    DepositRequest, MoveItemRequest, PositionQuery, SlotItem, WithdrawRequest,
};
use capitol_core::protocol::rules::Zone;
use capitol_core::protocol::ws::ClientMessage;
use capitol_core::state::{GameState, StateEvent, Trade, TradeAction, TradeEvent, Yield};
use capitol_core::station::{self, Kit, Placing, Visit};
use capitol_core::trading::{self, Approach, Draft};
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

/// The tag on the exchange panel's deposits and withdrawals (P7.2). The
/// poller refetches the containers and the balance on either answer.
const EXCHANGE_TAG: Tag = 40;

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
    /// Plots you claimed since the frame last took them.
    claimed: Vec<Uuid>,
    /// What's worth telling the player about picking a plot, since the
    /// frame last took them.
    plot_notes: Vec<PlotNote>,
    /// The deposit or withdrawal the server is answering (P7.2).
    exchanging: Option<Asked>,
    /// Why the last one was refused, and when.
    exchange_refused: Option<(String, Instant)>,
    /// What the last one did, and when.
    receipt: Option<(Receipt, Instant)>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotNote {
    /// The alt-click found no plot there.
    NoneThere,
    /// The server couldn't say.
    Unanswered,
    /// One of your plots owes `tax_owed` in tax, and goes at `forecloses_at` (#131).
    TaxOwed { tax_owed: i64, forecloses_at: i64 },
    /// One of your plots was foreclosed for `tax_owed`, and `refund` came back.
    Foreclosed { tax_owed: i64, refund: i64 },
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
            exchanging: None,
            exchange_refused: None,
            receipt: None,
            placing: None,
            placing_notes: Vec::new(),
            visit: Visit::default(),
            station_notes: Vec::new(),
            approach: Approach::default(),
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
        if let Some(step) = self.visit.click(station, &self.state, now) {
            self.station_step(now, step);
        }
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
            self.state.open_station(),
            self.state.recipe(recipe_id),
            self.state.station_container(),
        ) else {
            return;
        };
        let station_id = open.station_id;
        let check = craft::check(recipe, bench, self.state.crafting_output());
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
        self.net.rest(
            RestRequest::MoveItem(MoveItemRequest {
                item_id: m.item_id,
                target_container_id: m.target_container_id,
                target_slot: m.target_slot,
            }),
            MOVE_TAG,
        );
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

    /// Starts placing the station `item` makes, if it's a kit: the ghost
    /// follows the cursor until a click puts it down or it's cancelled.
    /// Another kit's placement gives way to this one.
    pub fn begin_placing(&mut self, item: &SlotItem) {
        if self.auth.phase() != Phase::InWorld {
            return;
        }
        if let Some(kit) = Kit::of(item) {
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
    /// if it can go there.
    pub fn place(&mut self, now: Instant, x: f64, y: f64) {
        let Some(placing) = &mut self.placing else {
            return;
        };
        if let Some(message) = placing.place([x, y], &self.state, now) {
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
            self.exchanging = None;
            self.exchange_refused = None;
            self.receipt = None;
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
        session.pick_plot(4000.0, 4000.0);
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

    /// Against the local stack, the station panel's rules end to end (P6.3):
    /// walk to a workbench and open it, Fill its box, craft a forge kit
    /// there, put the box back, open a chest and move the kit into it and
    /// out, walk out of reach and see it close, and take the chest down.
    ///
    /// Needs a workbench and a storage chest owned by `Station1`, each with
    /// its `station_inventory` container, inserted before the server started
    /// (the tick only learns stations at startup or from `place_station`).
    /// The SQL is under Stations in `flint-client/CLAUDE.md`. A bot gathers
    /// the fiber and twists the rope first. The chest is gone afterwards.
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
            while rope(&bot) < 4 {
                bot.gather("fiber", 2).unwrap();
                bot.craft("rope_basic").unwrap();
            }
            bot.gather("fiber", 6).unwrap();
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

        // A clear output, so the kit has somewhere to go.
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
        let forge = session
            .state()
            .recipe("craft_forge")
            .expect("the craft_forge migration")
            .clone();
        assert_eq!(forge.required_station_type.as_deref(), Some("workbench"));
        let shifts = craft::fill(
            &forge,
            session.state().inventory().unwrap(),
            &station_box(&session).unwrap(),
        );
        session.move_items(&shifts);
        let ready = |s: &Session| {
            let st = s.state();
            let r = st.recipe("craft_forge").unwrap();
            station_box(s).is_some_and(|b| craft::check(r, &b, st.crafting_output()).ready())
        };
        wait(&mut session, "rope and fiber on the workbench", &ready);

        session.craft_at_station(Instant::now(), "craft_forge");
        wait(&mut session, "the forge kit to start", &|s| {
            s.state().is_crafting()
        });
        wait(&mut session, "the forge kit", &|s| {
            *s.crafting().phase() == Phase::Idle && !s.state().is_crafting()
        });
        assert_eq!(craft::count(&output(&session), "forge_kit"), 1);
        assert_eq!(session.take_crafted().len(), 1);
        let left = |s: &Session, t: &str| station_box(s).map_or(0, |b| craft::count(&b, t));
        eprintln!(
            "after the craft the workbench holds {} rope, {} fiber",
            left(&session, "rope"),
            left(&session, "fiber")
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
        });
        wait(&mut session, "the kit in the chest", &|s| {
            station_box(s).is_some_and(|b| b.find(kit.item_id).is_some())
        });
        let inventory = session.state().inventory().unwrap().clone();
        session.move_item(Move {
            item_id: kit.item_id,
            target_container_id: inventory.id,
            target_slot: inventory.first_empty().unwrap() as i32,
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
