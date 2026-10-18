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

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capitol_protocol::rest::{
    LoginRequest, RegisterRequest, ResourceNodeInfo, SubscribeRequest, Subscription,
};
use capitol_protocol::rules::{self, Zone};
use capitol_protocol::ws::ClientMessage;
use uuid::Uuid;

use crate::catalog::container_type;
use crate::net::{
    ConnectionState, Net, NetConfig, NetEvent, RestError, RestRequest, RestResponse, Tag,
};
use crate::poll::{Fetch, Poller};
use crate::state::{GameState, StateEvent, Yield};

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
    #[error("blocked at ({x:.0}, {y:.0}): {reason}")]
    Blocked { reason: String, x: f64, y: f64 },
    /// The server didn't start an `extract`: someone got there first, or the
    /// node is out of range. `reason` is the server's, or says it never answered.
    #[error("the server didn't start extracting node {node_id}: {reason}")]
    Refused { node_id: Uuid, reason: String },
    #[error("{0}")]
    Failed(String),
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
}

impl Bot {
    /// Logs in, or registers if the account doesn't exist, then connects and
    /// waits for a full snapshot.
    pub fn sign_in(config: NetConfig, account: &Account) -> Result<Bot, BotError> {
        let mut bot = Bot::new(config, account.name.clone(), &account.email)?;
        let login = RestRequest::Login(LoginRequest {
            email: account.email.clone(),
            password: account.password.clone(),
            refresh: true,
        });
        let auth = match bot.call("login", login) {
            Ok(RestResponse::Auth(auth)) => auth,
            Err(BotError::Rest { source, .. }) if source.is_unauthorized() => {
                let register = RestRequest::Register(RegisterRequest {
                    email: account.email.clone(),
                    password: account.password.clone(),
                    player_name: account.name.clone(),
                    refresh: true,
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
            let changes = self.frame();
            for change in &changes {
                if let StateEvent::Connection(ConnectionState::AuthFailed { message }) = change {
                    return Err(BotError::AuthFailed(message.clone()));
                }
            }
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
    fn call(&mut self, what: &'static str, request: RestRequest) -> Result<RestResponse, BotError> {
        let tag = self.next_tag;
        self.next_tag += 1;
        self.net.rest(request, tag);
        let start = Instant::now();
        while start.elapsed() < CALL_TIMEOUT {
            self.frame();
            if let Some(i) = self.replies.iter().position(|(t, _)| *t == tag) {
                let (_, result) = self.replies.swap_remove(i);
                return result.map_err(|source| BotError::Rest { what, source });
            }
        }
        Err(BotError::Timeout {
            what: what.to_string(),
            waited: CALL_TIMEOUT,
        })
    }

    /// Walks to `(x, y)` and waits until the server has us there.
    pub fn walk_to(&mut self, x: f64, y: f64) -> Result<(), BotError> {
        let (from_x, from_y) = self.position();
        let distance = (x - from_x).hypot(y - from_y);
        if distance < ARRIVED {
            return Ok(());
        }
        self.send(ClientMessage::Move { x, y });
        let timeout = Duration::from_secs_f64(distance / rules::PLAYER_SPEED * 1.5 + 5.0);
        let seen = self.until(
            &format!("arriving at ({x:.0}, {y:.0})"),
            timeout,
            |s, seen| {
                blocked(seen).is_some()
                    || s.local_player()
                        .is_some_and(|p| (p.x - x).hypot(p.y - y) < ARRIVED)
            },
        )?;
        match blocked(&seen) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// How many of `item_type` are in the container of type `container`.
    pub fn count(&self, container: &str, item_type: &str) -> i32 {
        let held = match container {
            container_type::PLAYER_INVENTORY => self.state.inventory(),
            container_type::CRAFTING_INPUT => self.state.crafting_input(),
            container_type::CRAFTING_OUTPUT => self.state.crafting_output(),
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
            if let Some(e) = blocked(&changes) {
                return Err(e);
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
        self.send(ClientMessage::Craft {
            recipe_id: recipe.id.clone(),
            input_item_ids: inputs,
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
        };
        let draws: Vec<f64> = (0..1000).map(|_| bot.random()).collect();
        assert!(draws.iter().all(|r| (0.0..1.0).contains(r)));
        let mean = draws.iter().sum::<f64>() / draws.len() as f64;
        assert!((0.4..0.6).contains(&mean), "mean {mean}");
    }
}
