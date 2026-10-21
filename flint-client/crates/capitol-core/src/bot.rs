//! A headless player. [`Bot`] wires [`Net`], [`GameState`] and [`Poller`]
//! together the way the game does, then plays: it signs in, walks, extracts
//! fiber and twists it into rope. The `capitol-bot` binary runs it from the
//! command line, as a smoke test or as other players to watch. The live tests
//! use it as a harness.
//!
//! ```no_run
//! # use capitol_core::{bot::*, net::NetConfig};
//! let mut bot = Bot::sign_in(NetConfig::default(), &Account::numbered("bot", 1))?;
//! let report = bot.run_script()?;
//! println!("{report}");
//! # Ok::<(), BotError>(())
//! ```
//!
//! Everything blocks the calling thread, and a bot runs frames only inside its
//! own calls. Between calls the events wait in the channel, so give each bot
//! its own thread.
//!
//! **Refusals.** The server answers a refused `extract` with
//! `extraction_failed` and a reason (B9, #60). [`Bot::extract`] stops on it at
//! once and remembers the node. [`START_TIMEOUT`] stays as a backstop for an
//! answer that never comes.
//!
//! **Staying up** (C0.6). A dropped socket or a server restart doesn't end a
//! bot. The net layer reconnects by itself, with capped backoff and jitter,
//! and signs in again with the same key. From the drop until
//! [`Bot::recover`] has the bot back in the world, every call fails at once
//! with [`BotError::Offline`], so whatever was in flight is dropped, never
//! counted: the server cancelled it when the socket went. [`Bot::stay_up`]
//! runs a loop that can start over (wandering, trading, making a market)
//! through any number of drops. A revoked key ends the bot with
//! [`BotError::Revoked`].

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capitol_protocol::rest::{
    AutomationMode, BazaarDirectory, BazaarListing, BazaarPurchase, BazaarShop, BuyListing, CollectRequest,
    CollectResponse, ConfigureAutomation, DepositRequest, DepositResponse, GrantRequest, GrantResponse,
    HealthResponse, ListPlotsQuery, LoginRequest, MarketOrder, MarketSummaryResponse, MoveItemRequest,
    MyOrdersQuery, MyStallResponse, PlaceOrder, PlaceStationRequest, PlotInfo, RegisterRequest, RentStall,
    ResourceNodeInfo, SplitStackRequest, SplitStackResponse, StationAutomation, StationInfo, StationTool,
    StockListing, SubscribeRequest, Subscription, WithdrawRequest, WithdrawResponse,
};
use capitol_protocol::rules::{self, Zone};
use capitol_protocol::terrain::{Landmark, LandmarkKind};
use capitol_protocol::ws::{ClientMessage, ServerMessage};
use uuid::Uuid;

use crate::bazaar;
use crate::catalog::container_type;
use crate::net::{
    auth_error_is_final, ConnectionState, Net, NetConfig, NetEvent, RestError, RestRequest, RestResponse, Tag,
};
use crate::market::MarketEvent;
use crate::paths::{self, WalkGraph};
use crate::poll::{Fetch, Poller};
use crate::state::{answers, GameState, StateEvent, Yield};

/// How long one frame waits for the network. The poller and the trade timers
/// run once per frame, so this is the bot's clock resolution.
const FRAME: Duration = Duration::from_millis(16);
/// A REST call, sign-in included.
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
/// From `connect` to a full snapshot.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long an `extract` or `craft` may go unanswered before the bot decides
/// the server refused it.
pub const START_TIMEOUT: Duration = Duration::from_secs(3);
/// After a finish, how long the refetch may take to show the item.
const REFETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// While scouting, how often to ask `/world/nearby`. The poller's own 5 s is
/// 250 units of walking; asking every second finds a node as soon as it is
/// in range.
const SCOUT_POLL: Duration = Duration::from_secs(1);
/// Scouting stops this far from the centre: the moat and the Capitol are there.
const SCOUT_STOP: f64 = 400.0;
/// Close enough to call a walk done. The server stops within
/// [`rules::ARRIVAL_EPSILON`].
const ARRIVED: f64 = 1.0;
/// How long a node that refused us is skipped: two polls, by which time the
/// store has the server's word on it.
const REFUSAL_MEMORY: Duration = Duration::from_secs(10);
/// Extraction attempts per gather before giving up.
const MAX_EXTRACTIONS: usize = 10;
/// The first tag the bot's own calls use. The poller's start at
/// [`crate::poll::FIRST_TAG`].
const FIRST_TAG: Tag = 1;

/// How close a person comes before a greeting walk stops beside them
/// (C2.1): about three metres.
const GREET_RANGE: f64 = 30.0;
/// How long it stands with them.
const GREET_MOMENT: Duration = Duration::from_secs(2);
/// How long before it stops for the same person again.
const GREET_AGAIN: Duration = Duration::from_secs(120);
/// The step [`Bot::face`] takes to turn: 15 cm, under a tick's walk, but
/// enough for a client to see which way it went.
const FACE_STEP: f64 = 1.5;

/// After a changed offer, how long [`Bot::trade_with`] waits before it
/// accepts.
pub const TRADE_THINK: Duration = Duration::from_millis(1500);
/// How long [`Bot::trade_with`] leaves an unanswered request before asking
/// again.
const ASK_AGAIN: Duration = Duration::from_secs(20);

/// What [`Bot::trade_with`] puts on its side of a trade.
#[derive(Debug, Clone, Default)]
pub struct TradeOffer {
    /// Strands offered.
    pub strands: i64,
    /// Every stack of this item type in the inventory, if any.
    pub item_type: Option<String>,
}

impl TradeOffer {
    fn items(&self, bot: &Bot) -> Vec<capitol_protocol::ws::TradeOfferItem> {
        let (Some(kind), Some(inventory)) = (&self.item_type, bot.state.inventory()) else {
            return Vec::new();
        };
        inventory
            .items()
            .filter(|i| &i.item_type == kind)
            .map(|i| capitol_protocol::ws::TradeOfferItem {
                item_id: i.item_id.to_string(),
                quantity: i.quantity,
                qualities: None,
            })
            .collect()
    }
}

/// Who a bot logs in as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub email: String,
    pub password: String,
    /// The player name, used only when the account is registered.
    pub name: String,
}

impl Account {
    /// Bot `n` of a fleet: `bot3@bots.capitol.local`, named `Bot3`. The same
    /// prefix and number log in as the same player every run.
    pub fn numbered(prefix: &str, n: usize) -> Account {
        let mut name = String::new();
        let mut chars = prefix.chars();
        if let Some(first) = chars.next() {
            name.extend(first.to_uppercase());
            name.push_str(chars.as_str());
        }
        Account {
            email: format!("{prefix}{n}@bots.capitol.local"),
            password: format!("{prefix}-{n}-capitol-bot"),
            name: format!("{name}{n}"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error("{what}: {source}")]
    Rest {
        what: &'static str,
        source: RestError,
    },
    #[error("waited {waited:?} for {what}")]
    Timeout { what: String, waited: Duration },
    #[error("the server ended the session: {0}")]
    AuthFailed(String),
    /// The server refused the bot's API key: it was revoked, or its account
    /// is gone. The bot can't come back, so a supervisor should retire it.
    #[error("the server refused the bot's key: {0}")]
    Revoked(String),
    /// The connection dropped, and what was in flight with it. The net layer
    /// is reconnecting: [`Bot::recover`] waits for it.
    #[error("lost the server: {0}")]
    Offline(String),
    #[error("blocked at ({x:.0}, {y:.0}): {reason}")]
    Blocked { reason: String, x: f64, y: f64 },
    /// The server didn't start an `extract`: someone got there first, or the
    /// node is out of range. `reason` is the server's, or says it never answered.
    #[error("the server didn't start extracting node {node_id}: {reason}")]
    Refused { node_id: Uuid, reason: String },
    /// Someone asked to trade (C1.5), and a bot that
    /// [answers trades](Bot::answer_trades) stopped what it was doing: a
    /// walk, where it stands. Nothing went wrong; the step can be taken again.
    #[error("{0} asked to trade")]
    Interrupted(String),
    #[error("{0}")]
    Failed(String),
}

impl BotError {
    /// The server won't have the bot back: its key was revoked, or its
    /// session ended. Anything else is worth carrying on after.
    pub fn is_final(&self) -> bool {
        matches!(self, BotError::Revoked(_) | BotError::AuthFailed(_))
    }
}

/// What [`Bot::run_script`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub name: String,
    /// What each successful extraction yielded.
    pub extractions: Vec<Yield>,
    pub crafted: Yield,
    /// How far the server walked us, in world units.
    pub walked: f64,
    pub elapsed: Duration,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fiber: i32 = self.extractions.iter().map(|y| y.quantity).sum();
        write!(
            f,
            "{} walked {:.0} units, pulled {fiber} fiber from {} node{}, and made {} × {} \
             (quality {}, grade {}) in {:.1} s",
            self.name,
            self.walked,
            self.extractions.len(),
            if self.extractions.len() == 1 { "" } else { "s" },
            self.crafted.quantity,
            self.crafted.item_name,
            self.crafted.quality,
            rules::quality_grade(self.crafted.quality),
            self.elapsed.as_secs_f64()
        )
    }
}

/// One headless player: its connection, its mirror of the world, its poller.
pub struct Bot {
    net: Net,
    state: GameState,
    poller: Poller,
    name: String,
    next_tag: Tag,
    /// Replies to the bot's own calls, by tag, until [`Bot::call`] collects them.
    replies: Vec<(Tag, Result<RestResponse, RestError>)>,
    /// Nodes the server wouldn't let us extract, and when. The store can
    /// still call them available until the next poll, so they're skipped
    /// for [`REFUSAL_MEMORY`].
    refused: HashMap<Uuid, Instant>,
    /// Server positions seen, for [`Report::walked`].
    last_position: Option<(f64, f64)>,
    walked: f64,
    quiet: bool,
    rng: u64,
    /// What [`Bot::record`] keeps for [`Bot::take_journal`]: the finishes,
    /// breakages and balance changes seen in every frame, whichever call ran it.
    journal: Option<Vec<StateEvent>>,
    /// The walk graph (#211), once [`Bot::walks`] has fetched the terrain,
    /// with the world's landmarks. Dropped when the server says the terrain
    /// changed.
    walks: Option<Arc<WalkGraph>>,
    /// Signed in with an API key, so a refused sign-in means the key is no
    /// good any more.
    keyed: bool,
    /// The session has been live once. Before that, a failed attempt isn't
    /// a drop: the sign-in's own timeout covers a server that isn't up yet.
    live: bool,
    /// Why the connection dropped, from the drop until [`Bot::recover`] has
    /// the bot back.
    dropped: Option<String>,
    /// Why the server ended the session for good.
    ended: Option<String>,
    /// Walks stop for a trade request ([`Bot::answer_trades`]).
    answers_trades: bool,
    /// Walks pause beside a person ([`Bot::greet_people`]): whom, and when.
    greets: Option<HashMap<Uuid, Instant>>,
}

impl Bot {
    /// Logs in, or registers if the account doesn't exist, then connects and
    /// waits for a full snapshot.
    pub fn sign_in(config: NetConfig, account: &Account) -> Result<Bot, BotError> {
        let mut bot = Bot::new(config, account.name.clone(), &account.email)?;
        let login = RestRequest::Login(LoginRequest {
            email: account.email.clone(),
            password: account.password.clone(),
        });
        let auth = match bot.call("login", login) {
            Ok(RestResponse::Auth(auth)) => auth,
            Err(BotError::Rest { source, .. }) if source.is_unauthorized() => {
                let register = RestRequest::Register(RegisterRequest {
                    email: account.email.clone(),
                    password: account.password.clone(),
                    player_name: account.name.clone(),
                });
                match bot.call("register", register)? {
                    RestResponse::Auth(auth) => {
                        bot.say(format!("registered {}", account.email));
                        auth
                    }
                    other => return Err(unexpected("register", &other)),
                }
            }
            Ok(other) => return Err(unexpected("login", &other)),
            Err(e) => return Err(e),
        };
        bot.net.connect(&auth);
        bot.enter()?;
        Ok(bot)
    }

    /// Signs in with an API key (E0.9) in place of a password, as the key's
    /// account, then connects and waits for a full snapshot. A key with only
    /// `read` watches: the server refuses its moves, so it can't run a script.
    pub fn with_api_key(config: NetConfig, key: &str) -> Result<Bot, BotError> {
        // Named after its player once the snapshot is in.
        let shown: String = key.chars().take(12).collect();
        let mut bot = Bot::new(config, shown, key)?;
        bot.keyed = true;
        bot.net.connect(key);
        // A watching key's player isn't in the world, so wait for the rest first.
        bot.until("a snapshot", SNAPSHOT_TIMEOUT, |s, _| {
            s.inventory().is_some() && s.crafting_output().is_some() && !s.recipes().is_empty()
        })?;
        if bot.until("the player", Duration::from_secs(3), |s, _| s.local_player().is_some()).is_err() {
            return Err(BotError::Failed(
                "this key only watches (it has no `play` or `trade` scope), so it can't play".into(),
            ));
        }
        bot.enter()?;
        Ok(bot)
    }

    fn new(config: NetConfig, name: String, seeded_by: &str) -> Result<Bot, BotError> {
        let net = Net::spawn(config)
            .map_err(|e| BotError::Failed(format!("starting the net thread: {e}")))?;
        Ok(Bot {
            net,
            state: GameState::new(),
            poller: Poller::new(),
            name,
            next_tag: FIRST_TAG,
            replies: Vec::new(),
            refused: HashMap::new(),
            last_position: None,
            walked: 0.0,
            quiet: false,
            rng: seed(seeded_by),
            journal: None,
            walks: None,
            keyed: false,
            live: false,
            dropped: None,
            ended: None,
            answers_trades: false,
            greets: None,
        })
    }

    /// Waits for the snapshot after connecting.
    fn enter(&mut self) -> Result<(), BotError> {
        self.until("a snapshot", SNAPSHOT_TIMEOUT, |s, _| {
            s.local_player().is_some()
                && s.inventory().is_some()
                && s.crafting_output().is_some()
                && !s.recipes().is_empty()
        })?;
        if let Some(me) = self.state.local_player() {
            self.name = me.name.clone();
        }
        let (x, y) = self.position();
        self.say(format!("in the world at ({x:.0}, {y:.0})"));
        Ok(())
    }

    /// Adds `days` to the account's subscription, with the server's mock
    /// activation (E4.1). A server with `MOCK_SUBSCRIPTIONS` off refuses it.
    pub fn subscribe(&mut self, days: i32) -> Result<Subscription, BotError> {
        match self.call(
            "subscribe",
            RestRequest::Subscribe(SubscribeRequest { days }),
        )? {
            RestResponse::Subscription(subscription) => {
                self.state
                    .apply_rest(&RestResponse::Subscription(subscription));
                if let Some(until) = subscription.until {
                    self.say(format!(
                        "subscribed until {}",
                        until.format("%-d %b %Y %H:%M UTC")
                    ));
                }
                Ok(subscription)
            }
            other => Err(unexpected("subscribe", &other)),
        }
    }

    /// Stops the bot narrating to stdout.
    pub fn quiet(mut self) -> Bot {
        self.quiet = true;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// From now on a trade request stops a walk where the bot stands, with
    /// [`BotError::Interrupted`], so a citizen can answer it within the
    /// request's 30 s and the trade's range (C1.5). Off by default.
    pub fn answer_trades(&mut self, on: bool) {
        self.answers_trades = on;
    }

    /// Runs one of the store's trade actions (`trade_accept_request`,
    /// `trade_offer` and the rest) and sends what it returns. A refusal is
    /// the store's: nothing is sent.
    pub fn trade_act(
        &mut self,
        action: impl FnOnce(&mut GameState, Instant) -> Result<ClientMessage, crate::state::TradeRefusal>,
    ) -> Result<(), crate::state::TradeRefusal> {
        let message = action(&mut self.state, Instant::now())?;
        self.send(message);
        Ok(())
    }

    /// From now on a walk that passes a person (not a citizen) stops beside
    /// them a moment, turned their way, then walks on (C2.1). The same
    /// person is passed by without a pause for [`GREET_AGAIN`]. Off by
    /// default.
    pub fn greet_people(&mut self, on: bool) {
        self.greets = on.then(HashMap::new);
    }

    /// Someone has asked to trade, and the bot answers trades.
    pub fn asked_to_trade(&self) -> bool {
        self.answers_trades && asked(&self.state)
    }

    /// Stops where it stands for a trade request.
    fn interrupted(&mut self) -> BotError {
        let (x, y) = self.position();
        self.send(ClientMessage::Move { x, y });
        let who = match self.state.trade() {
            crate::state::Trade::Incoming { name, .. } => name.clone(),
            _ => "someone".into(),
        };
        BotError::Interrupted(who)
    }

    pub fn state(&self) -> &GameState {
        &self.state
    }

    /// The local player's server position.
    pub fn position(&self) -> (f64, f64) {
        self.state.local_player().map_or((0.0, 0.0), |p| (p.x, p.y))
    }

    /// Sends a WebSocket message as is.
    pub fn send(&self, message: ClientMessage) {
        self.net.ws(message);
    }

    /// Asks for a refetch now rather than when the rules or the timer would.
    pub fn refetch(&mut self, fetch: Fetch) {
        self.poller.request(fetch);
    }

    /// One frame: waits up to 16 ms for the network, applies everything that
    /// arrived, runs the timers and sends what the poller asks for.
    pub fn frame(&mut self) -> Vec<StateEvent> {
        let mut changes = Vec::new();
        let first = self.net.recv_timeout(FRAME);
        let rest: Vec<_> = self.net.drain().collect();
        for event in first.into_iter().chain(rest) {
            self.watch_link(&event);
            let applied = self.state.apply(&event);
            self.poller.observe(&event, &applied, &self.state);
            changes.extend(applied);
            if let NetEvent::Rest(tag, result) = event {
                if tag < crate::poll::FIRST_TAG {
                    self.replies.push((tag, result));
                } else if let Err(e) = result {
                    self.say(format!("a refetch failed: {e}"));
                }
            }
        }
        if changes.iter().any(|c| matches!(c, StateEvent::TerrainUpdated)) {
            self.walks = None;
        }
        if let Some(journal) = &mut self.journal {
            journal.extend(changes.iter().filter(|c| journaled(c)).cloned());
        }
        // Nobody plays the avatar: pick it up (E0.8). A bot on a key is
        // refused while one of the player's own clients is connected, and
        // hears `held_by: None` again when the last one goes.
        let free = changes.iter().any(|c| {
            matches!(c, StateEvent::Presence(p) if !p.yours && p.held_by.is_none())
        });
        if free {
            self.say("nobody is playing the avatar; taking it");
            self.net.ws(ClientMessage::ClaimPresence);
        }
        // What the market store wants said: watching, and asking again
        for message in self.state.market_mut().take_outgoing() {
            self.net.ws(message);
        }
        let now = Instant::now();
        changes.extend(self.state.tick(now));
        for (request, tag) in self.poller.tick(now) {
            self.net.rest(request, tag);
        }
        if let Some(p) = self.state.local_player() {
            if let Some((x, y)) = self.last_position {
                self.walked += (p.x - x).hypot(p.y - y);
            }
            self.last_position = Some((p.x, p.y));
        }
        changes
    }

    /// Notes a drop, and the server ending the session, for [`Bot::link`].
    fn watch_link(&mut self, event: &NetEvent) {
        match event {
            NetEvent::Connection(ConnectionState::Connected { .. }) => self.live = true,
            NetEvent::Connection(ConnectionState::Reconnecting { reason, .. })
                if self.live && self.dropped.is_none() =>
            {
                self.say(format!("lost the server ({reason}); reconnecting"));
                self.dropped = Some(reason.clone());
            }
            NetEvent::Connection(ConnectionState::AuthFailed { message }) => {
                self.ended.get_or_insert_with(|| message.clone());
            }
            // A live socket whose key is revoked hears this before it's
            // closed (`backend/src/ws/connection.rs`).
            NetEvent::Ws(ServerMessage::AuthError { message })
                if self.keyed && auth_error_is_final(message) =>
            {
                self.ended.get_or_insert_with(|| message.clone());
            }
            _ => {}
        }
    }

    /// `Err` once the server has ended the session, or from a drop until
    /// [`recover`](Self::recover). Every call checks it, so nothing carries on
    /// over a connection that went away under it.
    fn link(&self) -> Result<(), BotError> {
        if let Some(message) = &self.ended {
            return Err(if self.keyed {
                BotError::Revoked(message.clone())
            } else {
                BotError::AuthFailed(message.clone())
            });
        }
        match &self.dropped {
            Some(reason) => Err(BotError::Offline(reason.clone())),
            None => Ok(()),
        }
    }

    /// Picks the bot back up after the connection dropped
    /// ([`BotError::Offline`]). The net layer reconnects and signs in again by
    /// itself; this waits for that, then for the world: who holds the avatar
    /// (a bot claims it again if nobody does, in [`frame`](Self::frame)), a
    /// position, the refetches a reconnect starts (containers, nodes,
    /// stations, plots, currency), and the bot's orders and shop, whose fills
    /// and sales the drop may have hidden. The server cancelled any action or trade it had going.
    ///
    /// Waits until `deadline`, or for ever. Fails at once with
    /// [`BotError::Revoked`] (or [`BotError::AuthFailed`]) if the server won't
    /// have the bot back. With no drop to recover from, it does nothing.
    pub fn recover(&mut self, deadline: Option<Instant>) -> Result<(), BotError> {
        let start = Instant::now();
        let past = |what: &str| BotError::Timeout {
            what: what.into(),
            waited: start.elapsed(),
        };
        while self.dropped.is_some() {
            loop {
                self.frame();
                if self.ended.is_some() {
                    return self.link();
                }
                if self.state.is_connected() {
                    break;
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return Err(past("the server to come back"));
                }
            }
            self.dropped = None;
            // Dropped again while resyncing: wait for the next connection.
            match self.resync() {
                Err(BotError::Offline(_)) => continue,
                result => result?,
            }
        }
        self.link()
    }

    /// After a reconnect: the snapshot again, then the bot's orders, and its
    /// shop if it keeps one: a sale in the drop sent no `BazaarSale`.
    fn resync(&mut self) -> Result<(), BotError> {
        self.until("the world again", SNAPSHOT_TIMEOUT, |s, seen| {
            s.presence().is_some() && seen.contains(&StateEvent::PositionsUpdated)
        })?;
        self.settle()?;
        self.my_orders()?;
        if self.state.bazaar().my_shop().is_some() {
            self.my_stall()?;
        }
        self.refused.clear();
        // The server may have put us back elsewhere; that's no walk.
        self.last_position = None;
        let (x, y) = self.position();
        match self.state.presence() {
            Some(p) if !p.yours => self.say("back, watching: someone else holds the avatar"),
            _ => self.say(format!("back in the world at ({x:.0}, {y:.0})")),
        }
        Ok(())
    }

    /// Runs `work`, and whenever the connection drops under it, waits to
    /// [`recover`](Self::recover) and runs it again from the top. Returns
    /// what `work` returns otherwise, or why the bot couldn't get back by
    /// `deadline`. For loops that can start over: wandering, trading,
    /// making a market.
    pub fn stay_up<T>(
        &mut self,
        deadline: Option<Instant>,
        mut work: impl FnMut(&mut Bot) -> Result<T, BotError>,
    ) -> Result<T, BotError> {
        loop {
            match work(self) {
                Err(BotError::Offline(_)) => self.recover(deadline)?,
                result => return result,
            }
        }
    }

    /// Runs frames until `done` holds. It sees the state and every event since
    /// the call began, and the events are returned.
    pub fn until(
        &mut self,
        what: &str,
        timeout: Duration,
        mut done: impl FnMut(&GameState, &[StateEvent]) -> bool,
    ) -> Result<Vec<StateEvent>, BotError> {
        let start = Instant::now();
        let mut seen = Vec::new();
        loop {
            self.link()?;
            let changes = self.frame();
            self.link()?;
            seen.extend(changes);
            if done(&self.state, &seen) {
                return Ok(seen);
            }
            if start.elapsed() > timeout {
                return Err(BotError::Timeout {
                    what: what.to_string(),
                    waited: timeout,
                });
            }
        }
    }

    /// Runs frames for `how_long`, standing still.
    pub fn idle(&mut self, how_long: Duration) -> Result<Vec<StateEvent>, BotError> {
        let end = Instant::now() + how_long;
        let mut seen = self.until("nothing", how_long + FRAME, |_, _| Instant::now() >= end)?;
        seen.retain(|e| !matches!(e, StateEvent::PositionsUpdated));
        Ok(seen)
    }

    /// Runs frames until the poller has nothing queued or in flight, so the
    /// store holds the server's latest answer to everything it was asked.
    ///
    /// A finish can need two refetches. The tick writes a craft's output and
    /// sends `inventory_updated`, then deletes its inputs and sends another
    /// (`backend/src/engine/tick.rs`, after `game.tick()`). A refetch that
    /// lands between the two shows the rope with the fiber still there.
    pub fn settle(&mut self) -> Result<(), BotError> {
        let start = Instant::now();
        while !self.poller.is_idle() {
            self.frame();
            self.link()?;
            if start.elapsed() > REFETCH_TIMEOUT {
                return Err(BotError::Timeout {
                    what: "the refetches to settle".into(),
                    waited: REFETCH_TIMEOUT,
                });
            }
        }
        Ok(())
    }

    /// A REST call of the bot's own, answered in order with everything else.
    pub(crate) fn call(&mut self, what: &'static str, request: RestRequest) -> Result<RestResponse, BotError> {
        self.link()?;
        let tag = self.next_tag;
        self.next_tag += 1;
        self.net.rest(request, tag);
        let start = Instant::now();
        while start.elapsed() < CALL_TIMEOUT {
            self.frame();
            if let Some(i) = self.replies.iter().position(|(t, _)| *t == tag) {
                let (_, result) = self.replies.swap_remove(i);
                return result.map_err(|source| match source {
                    // A key doesn't expire, so a 401 means it was revoked.
                    source if self.keyed && source.is_unauthorized() => BotError::Revoked(source.to_string()),
                    source => BotError::Rest { what, source },
                });
            }
        }
        Err(BotError::Timeout {
            what: what.to_string(),
            waited: CALL_TIMEOUT,
        })
    }

    /// Walks to `(x, y)` and waits until the server has us there. The walk
    /// goes round the water, by the bridges and fords ([`WalkGraph::route`]),
    /// a `move` a leg. With no terrain, or no way round, it goes straight and
    /// stops at whatever is in the way.
    pub fn walk_to(&mut self, x: f64, y: f64) -> Result<(), BotError> {
        let from = self.position();
        if (x - from.0).hypot(y - from.1) < ARRIVED {
            return Ok(());
        }
        let walks = match self.walks() {
            Ok(walks) => Some(walks),
            // A drop or a refused key stops the walk like any other call
            Err(e @ (BotError::Offline(_) | BotError::Revoked(_) | BotError::AuthFailed(_))) => return Err(e),
            Err(e) => {
                self.say(format!("no terrain to plan the walk on ({e}); going straight"));
                None
            }
        };
        let legs = match walks.as_ref().map(|w| w.route(from, (x, y))) {
            Some(Some(legs)) => legs,
            Some(None) => {
                self.say(format!("no way round to ({x:.0}, {y:.0}); going straight"));
                vec![(x, y)]
            }
            None => vec![(x, y)],
        };
        for (lx, ly) in legs {
            let secs = walks.as_ref().map(|w| w.walk_secs(self.position(), (lx, ly)));
            self.walk_straight(lx, ly, secs)?;
        }
        Ok(())
    }

    /// The walk graph of the world's terrain: fetched the first time, then
    /// kept until the terrain changes. Every bot in the process shares one
    /// per terrain version ([`paths::shared`]), so a swarm revalidates the
    /// newest by version rather than each building its own.
    pub fn walks(&mut self) -> Result<Arc<WalkGraph>, BotError> {
        if let Some(walks) = &self.walks {
            return Ok(walks.clone());
        }
        let latest = paths::latest();
        let what = "fetching the terrain";
        let request = RestRequest::Terrain {
            cached_version: latest.as_ref().and_then(|w| w.version()).map(String::from),
        };
        let walks = match (self.call(what, request)?, latest) {
            (RestResponse::Terrain(terrain), _) => paths::shared(&terrain),
            (RestResponse::TerrainUnchanged, Some(latest)) => latest,
            (other, _) => return Err(unexpected(what, &other)),
        };
        self.walks = Some(walks.clone());
        Ok(walks)
    }

    /// Walks straight to `(x, y)`, with no planning, and waits until the
    /// server has us there. `secs` is how long the walk should take, if the
    /// slope is known; otherwise it's timed as on the flat. A bot that
    /// greets people ([`Bot::greet_people`]) pauses beside one it passes,
    /// then walks on.
    pub fn walk_straight(&mut self, x: f64, y: f64, secs: Option<f64>) -> Result<(), BotError> {
        let (from_x, from_y) = self.position();
        let whole = (x - from_x).hypot(y - from_y);
        loop {
            let (from_x, from_y) = self.position();
            let distance = (x - from_x).hypot(y - from_y);
            if distance < ARRIVED {
                return Ok(());
            }
            if self.asked_to_trade() {
                return Err(self.interrupted());
            }
            self.send(ClientMessage::Move { x, y });
            // What's left of the leg, after a pause
            let secs = secs.map_or(distance / rules::PLAYER_SPEED, |s| s * distance / whole.max(distance));
            let timeout = Duration::from_secs_f64(secs * 1.5 + 5.0);
            let answers = self.answers_trades;
            let passed = self.passed();
            let seen = self.until(
                &format!("arriving at ({x:.0}, {y:.0})"),
                timeout,
                |s, seen| {
                    blocked(seen).is_some()
                        || (answers && asked(s))
                        || passed.as_ref().is_some_and(|p| passer(s, p).is_some())
                        || s.local_player()
                            .is_some_and(|p| (p.x - x).hypot(p.y - y) < ARRIVED)
                },
            )?;
            if let Some(e) = blocked(&seen) {
                return Err(e);
            }
            if self.asked_to_trade() {
                return Err(self.interrupted());
            }
            match passed.and_then(|p| passer(&self.state, &p)) {
                Some(person) => self.pause_beside(person)?,
                None => return Ok(()),
            }
        }
    }

    /// Whom a greeting walk passes by without a pause: itself, and everyone
    /// it greeted in the last [`GREET_AGAIN`]. `None` if it doesn't greet.
    fn passed(&mut self) -> Option<Vec<Uuid>> {
        let greets = self.greets.as_mut()?;
        let now = Instant::now();
        greets.retain(|_, at| now.duration_since(*at) < GREET_AGAIN);
        Some(greets.keys().copied().chain(self.state.me()).collect())
    }

    /// Stops beside `person`, turns to them, and stands a moment. A trade
    /// request cuts it short; the walk's next leg answers it.
    fn pause_beside(&mut self, (id, x, y): (Uuid, f64, f64)) -> Result<(), BotError> {
        if let Some(greets) = self.greets.as_mut() {
            greets.insert(id, Instant::now());
        }
        let (px, py) = self.position();
        self.send(ClientMessage::Move { x: px, y: py });
        self.face(x, y)?;
        let end = Instant::now() + GREET_MOMENT;
        let answers = self.answers_trades;
        self.until("a moment", GREET_MOMENT + FRAME * 4, |s, _| {
            Instant::now() >= end || (answers && asked(s))
        })?;
        Ok(())
    }

    /// Turns toward `(x, y)` where it stands: a step of [`FACE_STEP`] that
    /// way, since a figure faces the way it last moved. Too close to step
    /// toward, it stays as it is.
    pub fn face(&mut self, x: f64, y: f64) -> Result<(), BotError> {
        let (px, py) = self.position();
        let (dx, dy) = (x - px, y - py);
        let d = dx.hypot(dy);
        if d < 2.0 * FACE_STEP {
            return Ok(());
        }
        let (tx, ty) = (px + dx / d * FACE_STEP, py + dy / d * FACE_STEP);
        self.send(ClientMessage::Move { x: tx, y: ty });
        let seen = self.until("turning", Duration::from_secs(2), |s, seen| {
            blocked(seen).is_some() || s.local_player().is_some_and(|p| (p.x - tx).hypot(p.y - ty) < ARRIVED)
        });
        match seen {
            // A turn that doesn't land is no matter
            Ok(_) | Err(BotError::Timeout { .. }) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// How many of `item_type` are in the container of type `container`.
    pub fn count(&self, container: &str, item_type: &str) -> i32 {
        let held = match container {
            container_type::PLAYER_INVENTORY => self.state.inventory(),
            container_type::CRAFTING_INPUT => self.state.crafting_input(),
            container_type::CRAFTING_OUTPUT => self.state.crafting_output(),
            container_type::MARKET_BOX => self.state.market_box(),
            _ => None,
        };
        held.map_or(0, |c| {
            c.items()
                .filter(|i| i.item_type == item_type)
                .map(|i| i.quantity)
                .sum()
        })
    }

    /// The closest node we know of that's available, yields `item_type`, and
    /// hasn't refused us.
    pub fn nearest_node(&self, item_type: &str) -> Option<ResourceNodeInfo> {
        let (x, y) = self.position();
        self.state
            .nodes()
            .values()
            .filter(|n| {
                n.state == "available"
                    && self
                        .refused
                        .get(&n.id)
                        .is_none_or(|at| at.elapsed() > REFUSAL_MEMORY)
            })
            .filter(|n| {
                self.state
                    .catalog()
                    .resource_type(&n.resource_type)
                    .is_some_and(|t| t.yields_item == item_type)
            })
            .min_by(|a, b| {
                (a.x - x)
                    .hypot(a.y - y)
                    .total_cmp(&(b.x - x).hypot(b.y - y))
            })
            .cloned()
    }

    /// Finds a node that yields `item_type`. If none is known, walks toward
    /// the Capitol, asking `/world/nearby` every second, until one turns up.
    /// Since B1 (#29) a fresh player sees grass from the spawn, so the walk is
    /// for a player who has wandered off, or whose nearby nodes are all taken.
    pub fn scout(&mut self, item_type: &str) -> Result<ResourceNodeInfo, BotError> {
        if let Some(node) = self.nearest_node(item_type) {
            return Ok(node);
        }
        let (cx, cy) = (Zone::CENTER_X, Zone::CENTER_Y);
        let (x, y) = self.position();
        let to_go = (cx - x).hypot(cy - y) - SCOUT_STOP;
        if to_go <= 0.0 {
            return Err(BotError::Failed(format!(
                "no {item_type} anywhere near here"
            )));
        }
        // Stop short of the moat, on the line in.
        let along = to_go / (to_go + SCOUT_STOP);
        let stop = (x + (cx - x) * along, y + (cy - y) * along);
        self.say(format!(
            "no {item_type} in sight; scouting toward the Capitol"
        ));
        self.send(ClientMessage::Move {
            x: stop.0,
            y: stop.1,
        });
        self.refetch(Fetch::NearbyNodes);
        let timeout = Duration::from_secs_f64(to_go / rules::PLAYER_SPEED * 1.5 + 5.0);
        let start = Instant::now();
        let mut asked = start;
        loop {
            let changes = self.frame();
            self.link()?;
            if let Some(e) = blocked(&changes) {
                return Err(e);
            }
            if self.asked_to_trade() {
                return Err(self.interrupted());
            }
            if changes
                .iter()
                .any(|c| matches!(c, StateEvent::NodeAdded(_) | StateEvent::NodeChanged(_)))
            {
                if let Some(node) = self.nearest_node(item_type) {
                    // Stop here; the walk to the node replaces the scouting move.
                    let (x, y) = self.position();
                    self.say(format!(
                        "found {} at ({:.0}, {:.0}), {:.0} away",
                        node.resource_type,
                        node.x,
                        node.y,
                        (node.x - x).hypot(node.y - y)
                    ));
                    return Ok(node);
                }
            }
            let (x, y) = self.position();
            if (x - stop.0).hypot(y - stop.1) < ARRIVED || start.elapsed() > timeout {
                return Err(BotError::Failed(format!(
                    "walked to ({x:.0}, {y:.0}) and found no {item_type}"
                )));
            }
            if asked.elapsed() >= SCOUT_POLL {
                asked = Instant::now();
                self.refetch(Fetch::NearbyNodes);
            }
        }
    }

    /// Walks to `node`, extracts it, and waits for the yield to show up in
    /// the inventory and the refetches to [`settle`](Self::settle).
    pub fn extract(&mut self, node: &ResourceNodeInfo) -> Result<Yield, BotError> {
        self.walk_to(node.x, node.y)?;
        self.send(ClientMessage::Extract {
            node_id: node.id.to_string(),
        });
        let started = StateEvent::ExtractionStarted { node_id: node.id };
        let answered =
            |e: &StateEvent| *e == started || matches!(e, StateEvent::ExtractionFailed { .. });
        let refusal = match self.until("the extraction to start", START_TIMEOUT, |_, seen| {
            seen.iter().any(answered)
        }) {
            Ok(seen) if seen.contains(&started) => None,
            Ok(seen) => seen.into_iter().find_map(|e| match e {
                StateEvent::ExtractionFailed { reason } => Some(reason),
                _ => None,
            }),
            Err(BotError::Timeout { waited, .. }) => {
                Some(format!("no answer in {:.0} s", waited.as_secs_f64()))
            }
            Err(e) => return Err(e),
        };
        if let Some(reason) = refusal {
            self.refused.insert(node.id, Instant::now());
            return Err(BotError::Refused {
                node_id: node.id,
                reason,
            });
        }
        let duration = self
            .state
            .catalog()
            .resource_type(&node.resource_type)
            .map_or(30, |t| t.extraction_duration_ticks);
        let timeout = Duration::from_millis(duration as u64 * rules::TICK_MS) + REFETCH_TIMEOUT;
        let seen = self.until("the extraction to finish", timeout, |_, seen| {
            seen.iter().any(|e| {
                matches!(
                    e,
                    StateEvent::ExtractionCompleted { .. } | StateEvent::ExtractionCancelled
                )
            })
        })?;
        let item = seen
            .into_iter()
            .find_map(|e| match e {
                StateEvent::ExtractionCompleted { item, .. } => Some(item),
                _ => None,
            })
            .ok_or_else(|| BotError::Failed("the extraction was cancelled".into()))?;
        self.wait_for_items(container_type::PLAYER_INVENTORY, &item)?;
        self.settle()?;
        Ok(item)
    }

    /// Extracts until the inventory holds at least `want` of `item_type`.
    /// Returns what each extraction yielded.
    pub fn gather(&mut self, item_type: &str, want: i32) -> Result<Vec<Yield>, BotError> {
        let mut yields = Vec::new();
        for _ in 0..MAX_EXTRACTIONS {
            let have = self.count(container_type::PLAYER_INVENTORY, item_type);
            if have >= want {
                return Ok(yields);
            }
            let node = self.scout(item_type)?;
            match self.extract(&node) {
                Ok(item) if item.quantity == 0 => {
                    self.say(format!("came away from the {} with nothing", node.resource_type));
                    yields.push(item);
                }
                Ok(item) => {
                    self.say(format!(
                        "extracted {} {} (quality {}); holding {}",
                        item.quantity,
                        item.item_name,
                        item.quality,
                        self.count(container_type::PLAYER_INVENTORY, item_type)
                    ));
                    yields.push(item);
                }
                Err(BotError::Refused { node_id, reason }) => {
                    self.say(format!("node {node_id} refused ({reason}); trying another"));
                }
                Err(e) => return Err(e),
            }
        }
        Err(BotError::Failed(format!(
            "{MAX_EXTRACTIONS} extractions and still short of {want} {item_type}"
        )))
    }

    /// Crafts `recipe_id` by hand from the inventory, and waits for the output
    /// to show up in the crafting output slot and the inputs to leave.
    pub fn craft(&mut self, recipe_id: &str) -> Result<Yield, BotError> {
        self.craft_at(recipe_id, None)
    }

    /// Crafts `recipe_id` from the inventory, at `station` if one is given
    /// (stand within its reach first) or by hand. Otherwise as [`craft`](Self::craft).
    pub fn craft_at(&mut self, recipe_id: &str, station: Option<Uuid>) -> Result<Yield, BotError> {
        let recipe = self
            .state
            .recipe(recipe_id)
            .cloned()
            .ok_or_else(|| BotError::Failed(format!("no recipe {recipe_id}")))?;
        let inventory = self
            .state
            .inventory()
            .ok_or_else(|| BotError::Failed("no inventory".into()))?;
        let mut inputs = Vec::new();
        for input in &recipe.inputs {
            let mut need = input.quantity;
            for item in inventory.items().filter(|i| i.item_type == input.item_type) {
                if need <= 0 {
                    break;
                }
                inputs.push(item.item_id.to_string());
                need -= item.quantity;
            }
            if need > 0 {
                return Err(BotError::Failed(format!(
                    "{} needs {} {}, and we're {need} short",
                    recipe.name, input.quantity, input.item_name
                )));
            }
        }
        // What the inventory should hold once the inputs are gone.
        let leftover: Vec<(String, i32)> = recipe
            .inputs
            .iter()
            .map(|i| {
                let have = self.count(container_type::PLAYER_INVENTORY, &i.item_type);
                (i.item_type.clone(), have - i.quantity)
            })
            .collect();
        self.send(match station {
            Some(station_id) => ClientMessage::CraftAtStation {
                station_id: station_id.to_string(),
                recipe_id: recipe.id.clone(),
                input_item_ids: inputs,
            },
            None => ClientMessage::Craft {
                recipe_id: recipe.id.clone(),
                input_item_ids: inputs,
            },
        });
        let timeout = Duration::from_millis(recipe.duration_ticks.max(0) as u64 * rules::TICK_MS)
            + START_TIMEOUT
            + REFETCH_TIMEOUT;
        let seen = self.until(&format!("crafting {}", recipe.name), timeout, |_, seen| {
            seen.iter().any(|e| {
                matches!(
                    e,
                    StateEvent::CraftingCompleted { .. }
                        | StateEvent::CraftingFailed { .. }
                        | StateEvent::CraftingCancelled
                )
            })
        })?;
        let item = seen
            .into_iter()
            .find_map(|e| match e {
                StateEvent::CraftingCompleted { item } => Some(Ok(item)),
                StateEvent::CraftingFailed { reason } => Some(Err(BotError::Failed(reason))),
                StateEvent::CraftingCancelled => {
                    Some(Err(BotError::Failed("the craft was cancelled".into())))
                }
                _ => None,
            })
            .expect("until stopped on one of these")?;
        self.wait_for_items(container_type::CRAFTING_OUTPUT, &item)?;
        // Output and inputs are written together (B10), but the refetch that
        // shows the output may not be the one that shows the inputs gone.
        self.wait_until("the inputs to be used up", |bot| {
            leftover
                .iter()
                .all(|(t, n)| bot.count(container_type::PLAYER_INVENTORY, t) <= *n)
        })?;
        self.settle()?;
        Ok(item)
    }

    /// Moves whatever is in the crafting output into the inventory: onto a
    /// stack of the same type, or into the first empty slot. A craft whose
    /// output differs fails while the output is full, and only the inventory
    /// lends its tools to gathering and crafting.
    pub fn collect_output(&mut self) -> Result<(), BotError> {
        let Some(output) = self.state.crafting_output() else {
            return Ok(());
        };
        let made: Vec<_> = output.items().cloned().collect();
        for item in made {
            let inventory = self
                .state
                .inventory()
                .ok_or_else(|| BotError::Failed("no inventory".into()))?;
            let stackable = self
                .state
                .catalog()
                .item_type(&item.item_type)
                .is_none_or(|t| t.stackable);
            let onto = inventory
                .items()
                .find(|i| stackable && i.item_type == item.item_type)
                .map(|i| i.slot_index as usize)
                .or_else(|| inventory.first_empty())
                .ok_or_else(|| BotError::Failed("the inventory is full".into()))?;
            let request = RestRequest::MoveItem(MoveItemRequest {
                item_id: item.item_id,
                target_container_id: inventory.id,
                target_slot: onto as i32,
            });
            self.call("collecting the crafting output", request)?;
        }
        self.wait_until("the crafting output to empty", |bot| {
            bot.state.crafting_output().is_none_or(|c| c.used_slots() == 0)
        })?;
        self.settle()
    }

    /// Puts a station down at `(x, y)` from the kit `kit_item_id`, over REST.
    /// Stand within the station type's reach of the spot first.
    pub fn place_station(
        &mut self,
        station_type: &str,
        kit_item_id: Uuid,
        x: f64,
        y: f64,
    ) -> Result<StationInfo, BotError> {
        let request = RestRequest::PlaceStation(PlaceStationRequest {
            station_type: station_type.to_string(),
            x,
            y,
            kit_item_id,
        });
        match self.call("placing a station", request)? {
            RestResponse::PlaceStation(placed) => {
                self.refetch(Fetch::Containers);
                self.refetch(Fetch::Plots);
                self.settle()?;
                Ok(placed.station)
            }
            other => Err(unexpected("placing a station", &other)),
        }
    }

    /// Takes our station `station_id` down, over REST.
    pub fn remove_station(&mut self, station_id: Uuid) -> Result<(), BotError> {
        match self.call("removing a station", RestRequest::RemoveStation { station_id })? {
            RestResponse::Success(_) => {
                self.refetch(Fetch::Containers);
                self.refetch(Fetch::NearbyStations);
                self.settle()
            }
            other => Err(unexpected("removing a station", &other)),
        }
    }

    /// Puts the tool `item_id` from the inventory into `station_id`'s empty
    /// tool slot (E1.6), and waits for the slot to show it and the inventory
    /// to lose it. Stand within the station's reach first.
    pub fn install_tool(&mut self, station_id: Uuid, item_id: Uuid) -> Result<StationTool, BotError> {
        self.know_station(station_id)?;
        self.send(ClientMessage::InstallTool {
            station_id: station_id.to_string(),
            item_id: item_id.to_string(),
        });
        let installed = |s: &GameState| {
            s.station(station_id)
                .and_then(|st| st.tool.as_ref())
                .is_some_and(|t| t.item_id == item_id)
        };
        self.tool_slot("install_tool", installed)?;
        // The inventory's `inventory_updated` follows the slot's change
        self.wait_until("the tool to leave the inventory", |bot| {
            bot.state.inventory().is_some_and(|inv| inv.find(item_id).is_none())
        })?;
        self.settle()?;
        self.state
            .station(station_id)
            .and_then(|s| s.tool.clone())
            .ok_or_else(|| BotError::Failed("the tool left the slot again".into()))
    }

    /// Takes `station_id`'s tool into the inventory (E1.6): our own, or
    /// anyone's from our own station, and waits for it to arrive. Stand
    /// within its reach first.
    pub fn remove_tool(&mut self, station_id: Uuid) -> Result<(), BotError> {
        self.know_station(station_id)?;
        let tool = self
            .state
            .station(station_id)
            .and_then(|s| s.tool.as_ref())
            .map(|t| t.item_id)
            .ok_or_else(|| BotError::Failed("the station's tool slot is empty".into()))?;
        self.send(ClientMessage::RemoveTool {
            station_id: station_id.to_string(),
        });
        self.tool_slot("remove_tool", |s| s.station(station_id).is_some_and(|st| st.tool.is_none()))?;
        self.wait_until("the tool to reach the inventory", |bot| {
            bot.state.inventory().is_some_and(|inv| inv.find(tool).is_some())
        })?;
        self.settle()
    }

    /// Waits for a tool slot's change to show, or for the server to refuse
    /// `command`.
    fn tool_slot(&mut self, command: &str, done: impl Fn(&GameState) -> bool) -> Result<(), BotError> {
        let seen = self.until(&format!("{command} to answer"), CALL_TIMEOUT, |s, seen| {
            done(s) || refusal(seen, command).is_some()
        })?;
        match refusal(&seen, command) {
            Some(reason) if !done(&self.state) => Err(BotError::Failed(reason)),
            _ => Ok(()),
        }
    }

    /// Fetches the stations nearby unless `station_id` is among those the
    /// store holds, since a slot's change is kept only on a station it knows.
    fn know_station(&mut self, station_id: Uuid) -> Result<(), BotError> {
        if self.state.station(station_id).is_none() {
            self.refetch(Fetch::NearbyStations);
            self.settle()?;
        }
        match self.state.station(station_id) {
            Some(_) => Ok(()),
            None => Err(BotError::Failed(format!("no station {station_id} near here"))),
        }
    }

    /// Sets what our station `station_id` makes on its own (E4.2): `recipe_id`
    /// from and into its own box, `mode` crafts or forever. It stops the
    /// station if it was running.
    pub fn configure_automation(
        &mut self,
        station_id: Uuid,
        recipe_id: &str,
        mode: AutomationMode,
    ) -> Result<StationAutomation, BotError> {
        let request = RestRequest::ConfigureAutomation {
            station_id,
            request: ConfigureAutomation {
                recipe_id: recipe_id.to_string(),
                mode,
            },
        };
        self.automation("configuring automation", request)
    }

    /// Sets our configured station running (E4.2). It pauses, rather than
    /// fails, while its box lacks an input.
    pub fn start_automation(&mut self, station_id: Uuid) -> Result<StationAutomation, BotError> {
        self.automation("starting automation", RestRequest::StartAutomation { station_id })
    }

    /// Stops our station (E4.2). It keeps its setting and its count.
    pub fn stop_automation(&mut self, station_id: Uuid) -> Result<StationAutomation, BotError> {
        self.automation("stopping automation", RestRequest::StopAutomation { station_id })
    }

    fn automation(&mut self, what: &'static str, request: RestRequest) -> Result<StationAutomation, BotError> {
        match self.call(what, request)? {
            RestResponse::Automation { automation, .. } => Ok(automation),
            other => Err(unexpected(what, &other)),
        }
    }

    /// Splits `quantity` off the stack `item_id` into an empty slot of its
    /// own container: `to_slot`, or the first empty one. The lowest units go
    /// (E1.8). Waits for the containers to settle.
    pub fn split_stack(
        &mut self,
        item_id: Uuid,
        quantity: i32,
        to_slot: Option<i32>,
    ) -> Result<SplitStackResponse, BotError> {
        let container = self
            .state
            .containers()
            .values()
            .find(|c| c.find(item_id).is_some())
            .ok_or_else(|| BotError::Failed(format!("no stack {item_id} in our containers")))?;
        let to_slot = match to_slot {
            Some(slot) => slot,
            None => container
                .first_empty()
                .ok_or_else(|| BotError::Failed("no empty slot to split into".into()))? as i32,
        };
        let request = RestRequest::SplitStack {
            item_id,
            request: SplitStackRequest {
                quantity,
                to_slot,
                to_container_id: None,
                qualities: None,
            },
        };
        match self.call("splitting a stack", request)? {
            RestResponse::SplitStack(split) => {
                self.settle()?;
                Ok(split)
            }
            other => Err(unexpected("splitting a stack", &other)),
        }
    }

    /// Up to 100 unclaimed plots in `zone`.
    pub fn unclaimed_plots(&mut self, zone: &str) -> Result<Vec<PlotInfo>, BotError> {
        let request = RestRequest::ListPlots(ListPlotsQuery {
            zone: Some(zone.to_string()),
            unclaimed: Some(true),
            limit: Some(100),
            ..Default::default()
        });
        match self.call("listing plots", request)? {
            RestResponse::Plots(list) => Ok(list.plots),
            other => Err(unexpected("listing plots", &other)),
        }
    }

    /// Buys the deed to `plot_id`. Returns the plot, now ours.
    pub fn claim_plot(&mut self, plot_id: Uuid) -> Result<PlotInfo, BotError> {
        match self.call("claiming a plot", RestRequest::ClaimPlot { plot_id })? {
            RestResponse::ClaimPlot(claim) => Ok(claim.plot),
            other => Err(unexpected("claiming a plot", &other)),
        }
    }

    /// Deposits every fiber stack in the inventory at the exchange.
    pub fn deposit_fiber(&mut self) -> Result<DepositResponse, BotError> {
        let item_ids: Vec<Uuid> = self
            .state
            .inventory()
            .map(|inv| crate::exchange::fiber_stacks(inv).iter().map(|i| i.item_id).collect())
            .unwrap_or_default();
        if item_ids.is_empty() {
            return Err(BotError::Failed("no fiber to deposit".into()));
        }
        match self.call("depositing fiber", RestRequest::Deposit(DepositRequest { item_ids }))? {
            RestResponse::Deposit(deposit) => {
                self.settle()?;
                Ok(deposit)
            }
            other => Err(unexpected("depositing fiber", &other)),
        }
    }

    /// Watches `item_type`'s market channel and waits for its whole book.
    pub fn watch_market(&mut self, item_type: &str) -> Result<(), BotError> {
        self.state.market_mut().watch(item_type);
        let item = item_type.to_string();
        self.until(&format!("{item_type}'s book"), CALL_TIMEOUT, |s, _| s.market().book(&item).is_some())?;
        Ok(())
    }

    /// Walks to the Exchange hall's door (D6, E3.4), where orders are placed
    /// and the market box collected, from wherever the bot is: the walk goes
    /// round the canals by their bridges (#211). A world without a hall needs
    /// no walk.
    pub fn go_to_exchange(&mut self) -> Result<(), BotError> {
        let Some(hall) = self.landmark(|l| l.kind == LandmarkKind::ExchangeHall)? else {
            return Ok(());
        };
        self.walk_to(hall.door.0, hall.door.1)
    }

    /// Walks to the bazaar's gate (E3.10), on the avenue across from the
    /// hall, round the water from wherever the bot is (#211). A world
    /// without a bazaar needs no walk.
    pub fn go_to_bazaar(&mut self) -> Result<(), BotError> {
        let Some(bazaar) = self.landmark(|l| l.kind == LandmarkKind::Bazaar)? else {
            return Ok(());
        };
        let (x, y) = self.position();
        if bazaar.distance_to(x, y) > 0.0 {
            self.down_the_avenue(bazaar.door.1)?;
        }
        self.walk_to(bazaar.door.0, bazaar.door.1)
    }

    /// Onto the south arterial's crown at `y`: the way in to the bazaar,
    /// which faces the hall across it. The walk goes round the water from
    /// wherever the bot is (#211).
    fn down_the_avenue(&mut self, y: f64) -> Result<(), BotError> {
        self.walk_to(rules::CENTER_X, y)
    }

    /// The first landmark `which` picks, from the `/terrain` the walk graph
    /// was built from.
    fn landmark(&mut self, which: impl Fn(&Landmark) -> bool) -> Result<Option<Landmark>, BotError> {
        Ok(self.walks()?.landmarks().iter().find(|l| which(l)).cloned())
    }

    /// Places an order (E3.3) and returns it as placed. The bot must be at the
    /// Exchange hall ([`Bot::go_to_exchange`]).
    pub fn place_order(&mut self, order: PlaceOrder) -> Result<MarketOrder, BotError> {
        match self.call("placing an order", RestRequest::PlaceOrder(order))? {
            RestResponse::Order(order) => {
                self.say(format!(
                    "{} {} {} at {} each",
                    order.side.as_str(),
                    order.quantity_total,
                    order.item_type,
                    order.price
                ));
                Ok(order)
            }
            other => Err(unexpected("placing an order", &other)),
        }
    }

    /// Cancels one of the bot's open orders and returns it as cancelled.
    pub fn cancel_order(&mut self, order_id: Uuid) -> Result<MarketOrder, BotError> {
        match self.call("cancelling an order", RestRequest::CancelOrder { order_id })? {
            RestResponse::Order(order) => {
                self.say(format!("cancelled {} {}", order.side.as_str(), order.item_type));
                Ok(order)
            }
            other => Err(unexpected("cancelling an order", &other)),
        }
    }

    /// The bot's open orders, from the server, into the store.
    pub fn my_orders(&mut self) -> Result<Vec<MarketOrder>, BotError> {
        match self.call("listing my orders", RestRequest::MyOrders(MyOrdersQuery::default()))? {
            RestResponse::MyOrders(orders) => Ok(orders),
            other => Err(unexpected("listing my orders", &other)),
        }
    }

    /// Takes everything in the market box to the inventory, as far as it fits.
    /// The bot must be at the Exchange hall ([`Bot::go_to_exchange`]).
    pub fn collect_market_box(&mut self) -> Result<CollectResponse, BotError> {
        match self.call(
            "collecting from the market box",
            RestRequest::CollectMarketBox(CollectRequest::default()),
        )? {
            RestResponse::Collected(collected) => {
                self.settle()?;
                Ok(collected)
            }
            other => Err(unexpected("collecting from the market box", &other)),
        }
    }

    /// Fetches the market box into the store.
    pub fn fetch_market_box(&mut self) -> Result<(), BotError> {
        match self.call("fetching the market box", RestRequest::MarketBox)? {
            RestResponse::MarketBox(_) => Ok(()),
            other => Err(unexpected("fetching the market box", &other)),
        }
    }

    /// Every item on the market: last price, best bid and ask.
    pub fn market_summary(&mut self) -> Result<MarketSummaryResponse, BotError> {
        match self.call("asking the market's summary", RestRequest::MarketSummary)? {
            RestResponse::MarketSummary(summary) => Ok(summary),
            other => Err(unexpected("asking the market's summary", &other)),
        }
    }

    /// The bazaar's directory (E3.10): every stall, and the shop in it, from
    /// anywhere.
    pub fn bazaar(&mut self) -> Result<BazaarDirectory, BotError> {
        match self.call("reading the bazaar's directory", RestRequest::Bazaar)? {
            RestResponse::Bazaar(directory) => Ok(directory),
            other => Err(unexpected("reading the bazaar's directory", &other)),
        }
    }

    /// Our own shop, if we rent a stall, and goods from a lapsed one, from
    /// anywhere.
    pub fn my_stall(&mut self) -> Result<MyStallResponse, BotError> {
        match self.call("reading our stall", RestRequest::MyStall)? {
            RestResponse::MyStall(mine) => Ok(mine),
            other => Err(unexpected("reading our stall", &other)),
        }
    }

    /// Walks to the stall `stall_id`, onto its street's crown in front of it
    /// ([`bazaar::approach`]): down the avenue to the bazaar's mouth if it's
    /// outside the quarter, then along the streets ([`bazaar::route`]).
    /// Stays put within its range.
    pub fn go_to_stall(&mut self, stall_id: &str) -> Result<(), BotError> {
        let stall = self
            .landmark(|l| l.kind == LandmarkKind::Stall && l.id == stall_id)?
            .ok_or_else(|| BotError::Failed(format!("no stall {stall_id}")))?;
        let (x, y) = self.position();
        if rules::bazaar::at_the_stall(&stall, x, y) {
            return Ok(());
        }
        let mouth = self
            .landmark(|l| l.kind == LandmarkKind::Bazaar)?
            .ok_or_else(|| BotError::Failed("the world has no bazaar".into()))?;
        let streets = self.walks()?.bazaar_streets().to_vec();
        // From out in the city, the avenue first: it's the way in over the
        // water, and the quarter opens off it
        let r = (x - rules::CENTER_X).hypot(y - rules::CENTER_Y);
        if r > rules::TRADE_OUTER_RADIUS {
            self.down_the_avenue(mouth.door.1)?;
        }
        for (x, y) in bazaar::route(&streets, &stall, self.position()) {
            self.walk_to(x, y)?;
        }
        Ok(())
    }

    /// Walks to `stall_id` and reads its shop: every listing's price and units.
    pub fn browse(&mut self, stall_id: &str) -> Result<BazaarShop, BotError> {
        self.go_to_stall(stall_id)?;
        let request = RestRequest::Shop {
            stall_id: stall_id.to_string(),
        };
        match self.call("browsing a stall", request)? {
            RestResponse::Shop(shop) => Ok(shop),
            other => Err(unexpected("browsing a stall", &other)),
        }
    }

    /// Walks to the free stall `stall_id` and rents it for `days` as `name`;
    /// or, on our own, pays `days` more and renames it if `name` is given.
    pub fn rent_stall(&mut self, stall_id: &str, days: i64, name: Option<&str>) -> Result<BazaarShop, BotError> {
        self.go_to_stall(stall_id)?;
        let request = RestRequest::RentStall {
            stall_id: stall_id.to_string(),
            request: RentStall {
                days,
                name: name.map(str::to_string),
            },
        };
        match self.call("renting a stall", request)? {
            RestResponse::Shop(shop) => {
                self.say(format!("renting {stall_id} as {} for {days} days", shop.name));
                Ok(shop)
            }
            other => Err(unexpected("renting a stall", &other)),
        }
    }

    /// Walks to our stall and lists units of an inventory stack there.
    pub fn stock(&mut self, listing: StockListing) -> Result<BazaarListing, BotError> {
        self.go_to_my_stall()?;
        let held = |bot: &Bot, kind: &str| bot.count(container_type::PLAYER_INVENTORY, kind);
        let kind = self
            .state
            .inventory()
            .and_then(|inv| inv.find(listing.item_id))
            .map(|i| i.item_type.clone());
        let before = kind.as_deref().map_or(0, |k| held(self, k));
        match self.call("stocking our stall", RestRequest::Stock(listing))? {
            RestResponse::Listed(listing) => {
                self.say(format!(
                    "listed {} {} at {} each",
                    listing.units.count(),
                    listing.item_type,
                    listing.price
                ));
                // The inventory's `inventory_updated` may come after the answer
                let want = before as i64 - listing.units.count();
                self.wait_until("the units to leave the inventory", |bot| {
                    held(bot, &listing.item_type) as i64 <= want
                })?;
                self.settle()?;
                Ok(listing)
            }
            other => Err(unexpected("stocking our stall", &other)),
        }
    }

    /// Walks to our stall and takes a listing off it. Its units come back to
    /// the inventory, or the market box if that's full.
    pub fn unlist(&mut self, listing_id: Uuid) -> Result<(), BotError> {
        self.go_to_my_stall()?;
        let coming = self
            .state
            .bazaar()
            .my_shop()
            .and_then(|s| s.listings.iter().find(|l| l.id == listing_id))
            .map(|l| (l.item_type.clone(), l.units.count()));
        let before = coming.as_ref().map_or(0, |(kind, _)| self.held(kind));
        match self.call("unlisting", RestRequest::Unlist { listing_id })? {
            RestResponse::Unlisted { .. } => {
                if let Some((kind, n)) = coming {
                    self.wait_until("the units to come back", |bot| bot.held(&kind) >= before + n)?;
                }
                self.settle()
            }
            other => Err(unexpected("unlisting", &other)),
        }
    }

    /// How many of `item_type` the inventory and the market box hold.
    fn held(&self, item_type: &str) -> i64 {
        (self.count(container_type::PLAYER_INVENTORY, item_type) + self.count(container_type::MARKET_BOX, item_type))
            as i64
    }

    /// Walks to the stall of `listing_id`, which a [`browse`](Self::browse)
    /// must have seen, and buys from it. The units go to the inventory, or
    /// the market box if that's full.
    pub fn buy(&mut self, listing_id: Uuid, request: BuyListing) -> Result<BazaarPurchase, BotError> {
        let stall_id = self
            .state
            .bazaar()
            .stall_of(listing_id)
            .map(str::to_string)
            .ok_or_else(|| BotError::Failed(format!("listing {listing_id} isn't at any stall we've browsed")))?;
        self.go_to_stall(&stall_id)?;
        let kind = self
            .state
            .bazaar()
            .shop(&stall_id)
            .and_then(|s| s.listings.iter().find(|l| l.id == listing_id))
            .map(|l| l.item_type.clone());
        let before = kind.as_deref().map_or(0, |k| self.held(k));
        match self.call("buying at a stall", RestRequest::Buy { listing_id, request })? {
            RestResponse::Bought { purchase, .. } => {
                self.say(format!(
                    "bought {} at {stall_id} for {}",
                    purchase.units.count(),
                    purchase.paid
                ));
                if let Some(kind) = kind {
                    let want = before + purchase.units.count();
                    self.wait_until("the goods to arrive", |bot| bot.held(&kind) >= want)?;
                }
                self.settle()?;
                Ok(purchase)
            }
            other => Err(unexpected("buying at a stall", &other)),
        }
    }

    /// Walks to the stall we rent, asking the server which if the store
    /// doesn't know.
    fn go_to_my_stall(&mut self) -> Result<(), BotError> {
        let stall_id = match self.state.bazaar().my_shop() {
            Some(shop) => shop.stall_id.clone(),
            None => self
                .my_stall()?
                .shop
                .map(|s| s.stall_id)
                .ok_or_else(|| BotError::Failed("we rent no stall".into()))?,
        };
        self.go_to_stall(&stall_id)
    }

    /// Strands and goods from nowhere, on a server with `DEV_GRANTS` on (E3.8).
    /// The goods land in the market box.
    pub fn grant(&mut self, request: GrantRequest) -> Result<GrantResponse, BotError> {
        match self.call("asking for a grant", RestRequest::Grant(request))? {
            RestResponse::Granted(granted) => Ok(granted),
            other => Err(unexpected("asking for a grant", &other)),
        }
    }

    /// Buys `quantity` fiber from the exchange with Strands.
    pub fn withdraw_fiber(&mut self, quantity: i32) -> Result<WithdrawResponse, BotError> {
        match self.call(
            "withdrawing fiber",
            RestRequest::Withdraw(WithdrawRequest { quantity }),
        )? {
            RestResponse::Withdraw(withdrawn) => {
                self.settle()?;
                Ok(withdrawn)
            }
            other => Err(unexpected("withdrawing fiber", &other)),
        }
    }

    /// `GET /health`.
    pub fn health(&mut self) -> Result<HealthResponse, BotError> {
        match self.call("asking the server's health", RestRequest::Health)? {
            RestResponse::Health(health) => Ok(health),
            other => Err(unexpected("asking the server's health", &other)),
        }
    }

    /// Keeps the finishes, tool breakages, balance changes, and the bot's
    /// orders placed and filled, that every frame sees from now on, for [`take_journal`](Self::take_journal). The
    /// economy sim counts from them.
    pub fn record(&mut self) {
        self.journal.get_or_insert_with(Vec::new);
    }

    /// What [`record`](Self::record) kept since the last call.
    pub fn take_journal(&mut self) -> Vec<StateEvent> {
        self.journal.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// The script: gather enough fiber for `rope_basic`, craft it, report.
    pub fn run_script(&mut self) -> Result<Report, BotError> {
        const RECIPE: &str = "rope_basic";
        let start = Instant::now();
        let walked = self.walked;
        let fiber = self
            .state
            .recipe(RECIPE)
            .and_then(|r| r.inputs.iter().find(|i| i.item_type == "fiber"))
            .map(|i| i.quantity)
            .ok_or_else(|| BotError::Failed(format!("{RECIPE} doesn't take fiber")))?;
        let extractions = self.gather("fiber", fiber)?;
        let crafted = self.craft(RECIPE)?;
        let report = Report {
            name: self.name.clone(),
            extractions,
            crafted,
            walked: self.walked - walked,
            elapsed: start.elapsed(),
        };
        Ok(report)
    }

    /// Wanders within `radius` of where it stands, pausing between walks,
    /// until `deadline` (or forever). Something to watch.
    ///
    /// With `near`, it wanders around that player instead (matched by name,
    /// ignoring case) and follows them as they move. If they aren't online,
    /// it stays where it is until they are.
    pub fn wander(
        &mut self,
        radius: f64,
        near: Option<&str>,
        deadline: Option<Instant>,
    ) -> Result<(), BotError> {
        let home = self.position();
        match near {
            Some(name) => self.say(format!("wandering within {radius:.0} of {name}")),
            None => self.say(format!(
                "wandering within {radius:.0} of ({:.0}, {:.0})",
                home.0, home.1
            )),
        }
        let mut lost = false;
        while deadline.is_none_or(|d| Instant::now() < d) {
            let centre = match near {
                None => home,
                Some(name) => match self.player_named(name) {
                    Some(at) => {
                        lost = false;
                        at
                    }
                    None => {
                        if !lost {
                            self.say(format!("{name} isn't online; waiting here"));
                            lost = true;
                        }
                        self.idle(Duration::from_secs(1))?;
                        continue;
                    }
                },
            };
            // Uniform over the disc.
            let r = radius * self.random().sqrt();
            let a = std::f64::consts::TAU * self.random();
            match self.walk_to(centre.0 + r * a.cos(), centre.1 + r * a.sin()) {
                Ok(()) | Err(BotError::Blocked { .. }) => {}
                Err(e) => return Err(e),
            }
            let pause = 0.5 + 2.5 * self.random();
            self.idle(Duration::from_secs_f64(pause))?;
        }
        Ok(())
    }

    /// Trades with `name` until `deadline` (or forever): a partner for
    /// testing a client's trade window (P7.3).
    ///
    /// It stays within trading range of them, answers their requests, and
    /// with `ask` asks them itself whenever it's idle beside them. In a
    /// trade it offers what `offer` says, then accepts once their offer has
    /// something in it, or once they've accepted. A changed offer resets
    /// both accepts on the server, so it waits [`TRADE_THINK`] after one
    /// before accepting again, for a person to see the reset.
    pub fn trade_with(
        &mut self,
        name: &str,
        offer: &TradeOffer,
        ask: bool,
        deadline: Option<Instant>,
    ) -> Result<(), BotError> {
        use crate::state::{Trade, TradeEvent};
        self.say(format!("trading with {name}"));
        let mut offered: Option<Uuid> = None;
        let mut thinking_since = Instant::now();
        let mut asked_at: Option<Instant> = None;
        while deadline.is_none_or(|d| Instant::now() < d) {
            let changes = self.frame();
            self.link()?;
            let now = Instant::now();
            for change in &changes {
                let StateEvent::Trade(event) = change else {
                    continue;
                };
                match event {
                    TradeEvent::OfferChanged { .. } => thinking_since = now,
                    TradeEvent::Executed { .. } => {
                        self.say("trade done");
                        offered = None;
                    }
                    TradeEvent::Cancelled { reason, .. } => {
                        self.say(format!("trade cancelled: {reason}"));
                        offered = None;
                    }
                    TradeEvent::Refused { reason, .. } => self.say(format!("refused: {reason}")),
                    _ => {}
                }
            }
            let Some(them) = self.player_id_named(name) else {
                continue;
            };
            let trade = self.state.trade().clone();
            match trade {
                Trade::Incoming { from, .. } if from == them => {
                    if let Ok(m) = self.state.trade_accept_request(now) {
                        self.say("accepting their request");
                        self.send(m);
                    }
                }
                Trade::Incoming { .. } => {
                    if let Ok(m) = self.state.trade_decline_request() {
                        self.send(m);
                    }
                }
                Trade::Open(open) => {
                    if offered != Some(open.trade_id) {
                        offered = Some(open.trade_id);
                        let items = offer.items(self);
                        self.say(format!(
                            "offering {} stacks and {} Strands",
                            items.len(),
                            offer.strands
                        ));
                        if let Ok(m) = self.state.trade_offer(now, items, offer.strands) {
                            self.send(m);
                        }
                        thinking_since = now;
                    }
                    let theirs = !open.theirs.items.is_empty() || open.theirs.strands > 0;
                    if !open.i_accepted
                        && (theirs || open.they_accepted)
                        && self.state.trade_waiting().is_none()
                        && now.duration_since(thinking_since) >= TRADE_THINK
                    {
                        if let Ok(m) = self.state.trade_accept(now) {
                            self.say("accepting");
                            self.send(m);
                        }
                    }
                }
                Trade::Idle | Trade::Requested { .. } => {
                    let Some((x, y)) = self.player_named(name) else {
                        continue;
                    };
                    let (mx, my) = self.position();
                    let apart = (x - mx).hypot(y - my);
                    if apart < rules::TRADE_RANGE * 0.25 {
                        // Standing on them: step a few metres off, so both
                        // can be seen.
                        let a = std::f64::consts::TAU * self.random();
                        let r = rules::TRADE_RANGE * 0.5;
                        match self.walk_to(x + r * a.cos(), y + r * a.sin()) {
                            Ok(()) | Err(BotError::Blocked { .. }) => {}
                            Err(e) => return Err(e),
                        }
                    } else if apart > rules::TRADE_RANGE * 0.8 {
                        let (dx, dy) = (mx - x, my - y);
                        let d = dx.hypot(dy).max(1e-6);
                        let r = rules::TRADE_RANGE * 0.5;
                        match self.walk_to(x + dx / d * r, y + dy / d * r) {
                            Ok(()) | Err(BotError::Blocked { .. }) => {}
                            Err(e) => return Err(e),
                        }
                    } else if ask
                        && matches!(trade, Trade::Idle)
                        && asked_at.is_none_or(|t| now.duration_since(t) > ASK_AGAIN)
                    {
                        asked_at = Some(now);
                        if let Ok(m) = self.state.trade_initiate(now, them) {
                            self.say(format!("asking {name} to trade"));
                            self.send(m);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn player_id_named(&self, name: &str) -> Option<Uuid> {
        let me = self.state.me();
        self.state
            .players()
            .values()
            .find(|p| Some(p.id) != me && p.name.eq_ignore_ascii_case(name))
            .map(|p| p.id)
    }

    /// Where another player is, by name, ignoring case.
    fn player_named(&self, name: &str) -> Option<(f64, f64)> {
        let me = self.state.me();
        self.state
            .players()
            .values()
            .find(|p| Some(p.id) != me && p.name.eq_ignore_ascii_case(name))
            .map(|p| (p.x, p.y))
    }

    /// Prints `Name: message`, unless the bot is [`quiet`](Self::quiet).
    pub fn say(&self, message: impl fmt::Display) {
        if !self.quiet {
            println!("{}: {message}", self.name);
        }
    }

    /// Waits until a container holds at least `item.quantity` more of
    /// `item.item_type` than it did, which takes a refetch after the finish.
    fn wait_for_items(&mut self, container: &str, item: &Yield) -> Result<(), BotError> {
        // The finish has just arrived and its refetch can't have answered
        // yet, so what the store holds now is the "before".
        let want = self.count(container, &item.item_type) + item.quantity;
        self.wait_until(
            &format!("{want} {} in {container}", item.item_type),
            |bot| bot.count(container, &item.item_type) >= want,
        )
    }

    /// Runs frames until `done` holds, for up to [`REFETCH_TIMEOUT`].
    fn wait_until(&mut self, what: &str, done: impl Fn(&Bot) -> bool) -> Result<(), BotError> {
        let start = Instant::now();
        while !done(self) {
            self.frame();
            self.link()?;
            if start.elapsed() > REFETCH_TIMEOUT {
                return Err(BotError::Timeout {
                    what: what.to_string(),
                    waited: REFETCH_TIMEOUT,
                });
            }
        }
        Ok(())
    }

    /// Uniform in [0, 1). xorshift64*, which is plenty for picking where to walk.
    fn random(&mut self) -> f64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Someone has asked us to trade, and we haven't answered.
fn asked(state: &GameState) -> bool {
    matches!(state.trade(), crate::state::Trade::Incoming { .. })
}

/// A person (not a citizen) within [`GREET_RANGE`], other than those in
/// `passed`: their id and where they stand.
fn passer(state: &GameState, passed: &[Uuid]) -> Option<(Uuid, f64, f64)> {
    let me = state.local_player()?;
    state
        .players()
        .values()
        .filter(|p| !p.citizen && !passed.contains(&p.id))
        .find(|p| (p.x - me.x).hypot(p.y - me.y) < GREET_RANGE)
        .map(|p| (p.id, p.x, p.y))
}

/// The server's refusal of `command`, if `seen` holds one.
fn refusal(seen: &[StateEvent], command: &str) -> Option<String> {
    seen.iter().find_map(|e| match e {
        StateEvent::ServerError { message, command: c } if answers(c, command) => Some(message.clone()),
        _ => None,
    })
}

fn blocked(seen: &[StateEvent]) -> Option<BotError> {
    seen.iter().find_map(|e| match e {
        StateEvent::MovementBlocked { reason, x, y, .. } => Some(BotError::Blocked {
            reason: reason.clone(),
            x: *x,
            y: *y,
        }),
        _ => None,
    })
}

/// What [`Bot::record`] keeps.
fn journaled(change: &StateEvent) -> bool {
    matches!(
        change,
        StateEvent::ExtractionCompleted { .. }
            | StateEvent::CraftingCompleted { .. }
            | StateEvent::ToolBroke { .. }
            | StateEvent::CurrencyChanged { .. }
    ) || matches!(
        change,
        StateEvent::Market(m) if matches!(**m, MarketEvent::OrderPlaced(_) | MarketEvent::OrderFilled { .. })
    )
}

fn unexpected(what: &str, response: &RestResponse) -> BotError {
    BotError::Failed(format!("{what} answered with {response:?}"))
}

/// Differs per account and per run, so a fleet doesn't wander in step.
fn seed(email: &str) -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    // FNV-1a over the email.
    let hash = email.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    });
    (nanos ^ hash) | 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_accounts_are_stable_and_named() {
        let a = Account::numbered("bot", 3);
        assert_eq!(a.email, "bot3@bots.capitol.local");
        assert_eq!(a.name, "Bot3");
        assert_eq!(a, Account::numbered("bot", 3));
        assert_ne!(a.password, Account::numbered("bot", 4).password);
        assert_eq!(Account::numbered("éclair", 1).name, "Éclair1");
    }

    #[test]
    fn the_report_reads_as_a_sentence() {
        let fiber = |quantity| Yield {
            item_type: "fiber".into(),
            item_name: "Plant Fiber".into(),
            quantity,
            quality: 55,
        };
        let report = Report {
            name: "Bot1".into(),
            extractions: vec![fiber(1), fiber(3)],
            crafted: Yield {
                item_type: "rope".into(),
                item_name: "Rope".into(),
                quantity: 1,
                quality: 61,
            },
            walked: 1234.4,
            elapsed: Duration::from_millis(31_300),
        };
        assert_eq!(
            report.to_string(),
            "Bot1 walked 1234 units, pulled 4 fiber from 2 nodes, and made 1 × Rope \
             (quality 61, grade C) in 31.3 s"
        );
    }

    /// A bot whose net goes nowhere: what it hears is fed to [`Bot::watch_link`].
    fn offline_bot(keyed: bool) -> Bot {
        let mut bot = Bot::new(NetConfig::with_base_url("http://127.0.0.1:9"), "T".into(), "t@x").unwrap();
        bot.quiet = true;
        bot.keyed = keyed;
        bot
    }

    fn hear(bot: &mut Bot, state: ConnectionState) {
        bot.watch_link(&NetEvent::Connection(state));
    }

    fn reconnecting(reason: &str) -> ConnectionState {
        ConnectionState::Reconnecting {
            attempt: 1,
            delay: Duration::from_millis(500),
            reason: reason.into(),
        }
    }

    #[test]
    fn a_drop_after_the_session_was_live_fails_every_call_until_recovered() {
        let mut bot = offline_bot(true);
        // Not up yet: the sign-in's own timeout covers it.
        hear(&mut bot, reconnecting("could not connect"));
        assert!(bot.link().is_ok());

        hear(&mut bot, ConnectionState::Connected { player_id: Uuid::nil() });
        hear(&mut bot, reconnecting("connection closed by server"));
        hear(&mut bot, reconnecting("could not connect"));
        let e = bot.link().unwrap_err();
        assert!(matches!(&e, BotError::Offline(r) if r == "connection closed by server"), "{e:?}");
        assert!(!e.is_final());
        assert!(matches!(bot.idle(Duration::from_secs(5)), Err(BotError::Offline(_))));
        assert!(matches!(bot.health(), Err(BotError::Offline(_))), "a REST call isn't sent");

        // This net never comes back, so recovering runs out of time.
        let deadline = Instant::now() + Duration::from_millis(50);
        let e = bot.recover(Some(deadline)).unwrap_err();
        assert!(matches!(e, BotError::Timeout { .. }), "{e:?}");
    }

    #[test]
    fn with_nothing_dropped_recovering_does_nothing() {
        let mut bot = offline_bot(true);
        hear(&mut bot, ConnectionState::Connected { player_id: Uuid::nil() });
        assert!(bot.recover(Some(Instant::now())).is_ok());
    }

    #[test]
    fn a_refused_key_is_revoked_and_final() {
        let mut bot = offline_bot(true);
        hear(&mut bot, ConnectionState::Connected { player_id: Uuid::nil() });
        // Not about the key: retried.
        bot.watch_link(&NetEvent::Ws(ServerMessage::AuthError {
            message: "Auth timeout".into(),
        }));
        assert!(bot.link().is_ok());
        // A live socket's key revoked: it's told, then closed, then reconnects.
        bot.watch_link(&NetEvent::Ws(ServerMessage::AuthError {
            message: "API key revoked".into(),
        }));
        hear(&mut bot, reconnecting("connection closed by server"));
        let e = bot.link().unwrap_err();
        assert!(matches!(&e, BotError::Revoked(m) if m == "API key revoked"), "{e:?}");
        assert!(e.is_final());
        let e = bot.recover(None).unwrap_err();
        assert!(matches!(e, BotError::Revoked(_)), "recovering doesn't wait: {e:?}");
    }

    #[test]
    fn a_password_session_refused_is_auth_failed() {
        let mut bot = offline_bot(false);
        hear(&mut bot, ConnectionState::Connected { player_id: Uuid::nil() });
        hear(&mut bot, ConnectionState::AuthFailed {
            message: "Session expired".into(),
        });
        let e = bot.link().unwrap_err();
        assert!(matches!(e, BotError::AuthFailed(_)), "{e:?}");
        assert!(e.is_final());
    }

    #[test]
    fn stay_up_runs_the_work_again_after_a_drop_it_recovers_from() {
        let mut bot = offline_bot(true);
        let mut runs = 0;
        let result: Result<u32, BotError> = bot.stay_up(None, |bot| {
            runs += 1;
            match runs {
                // As if the work saw a drop, and the net was back by the time
                // it was noticed.
                1 => {
                    hear(bot, ConnectionState::Connected { player_id: Uuid::nil() });
                    Err(BotError::Offline("connection lost".into()))
                }
                n => Ok(n),
            }
        });
        // The first recover found nothing dropped (the work only said so), so
        // it went straight back to the work.
        assert_eq!(result.unwrap(), 2);
        let mut bot = offline_bot(true);
        let result: Result<(), BotError> = bot.stay_up(None, |_| Err(BotError::Revoked("gone".into())));
        assert!(matches!(result, Err(BotError::Revoked(_))));
    }

    #[test]
    fn random_stays_in_the_unit_interval_and_moves() {
        let net = Net::spawn(NetConfig::with_base_url("http://127.0.0.1:9")).unwrap();
        let mut bot = Bot {
            net,
            state: GameState::new(),
            poller: Poller::new(),
            name: "T".into(),
            next_tag: FIRST_TAG,
            replies: Vec::new(),
            refused: HashMap::new(),
            last_position: None,
            walked: 0.0,
            quiet: true,
            rng: seed("t@x"),
            journal: None,
            walks: None,
            keyed: false,
            live: false,
            dropped: None,
            ended: None,
            answers_trades: false,
            greets: None,
        };
        let draws: Vec<f64> = (0..1000).map(|_| bot.random()).collect();
        assert!(draws.iter().all(|r| (0.0..1.0).contains(r)));
        let mean = draws.iter().sum::<f64>() / draws.len() as f64;
        assert!((0.4..0.6).contains(&mean), "mean {mean}");
    }
}
