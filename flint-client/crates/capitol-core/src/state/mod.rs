//! The world mirror. [`GameState`] takes what the net layer reports (a
//! [`ServerMessage`], a REST result, a connection change) and returns the
//! [`StateEvent`]s the UI and the 3D world react to. It does no IO and never
//! reads the clock, so every rule here is a plain reducer test. The trade
//! timeouts run on the `now` the game passes to [`GameState::tick`].
//!
//! ```no_run
//! # use capitol_core::{net::*, state::*};
//! # let net = Net::spawn(NetConfig::default()).unwrap();
//! let mut state = GameState::new();
//! // once per frame:
//! for event in net.drain() {
//!     for change in state.apply(&event) {
//!         // spawn a node, toast a completion, refetch containers, ...
//!     }
//! }
//! for change in state.tick(std::time::Instant::now()) {
//!     // a trade command the server never answered
//! }
//! ```
//!
//! **Filtering.** The server broadcasts extraction, crafting, station and
//! `inventory_updated` events to everyone. Each carries a `player_id`, and only
//! the local player's change local state. Other players' activity goes in a
//! per-player map so the world can show them working.
//!
//! **Facts, not requests.** The store never decides what to refetch. It emits
//! [`StateEvent::SnapshotStale`], [`StateEvent::InventoryChanged`] and so on, and
//! the polling scheduler ([`crate::poll::Poller`]) turns those into REST calls.
//!
//! Make a new `GameState` for each login. It assumes one local player.

mod container;
mod trade;

use std::collections::HashMap;

use capitol_protocol::rest::{
    CatalogResponse, ClaimedPlot, PauseReason, PlotInfo, RecipeInfo, ResourceNodeInfo,
    StationInfo, Subscription, SubscriptionStatus, PRODUCTION_LOG_LEN,
};
use capitol_protocol::rules::{self, Zone};
use capitol_protocol::tunables::Fees;
use capitol_protocol::ws::{ItemCount, PlayerPosition, PresenceHolder, ServerMessage};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::bazaar::BazaarStore;
use crate::catalog::{container_type, Catalog};
use crate::market::{MarketEvent, MarketStore};
use crate::net::{ConnectionState, NetEvent, RestResponse};

pub use container::Container;
pub use trade::{
    Offer, OpenTrade, Trade, TradeAction, TradeEvent, TradeRefusal, CANCEL_UNCONFIRMED,
    CONNECTION_LOST, PARTNER_LEFT, REPLY_TIMEOUT,
};

/// What one player is busy with. The server allows one action at a time
/// (`ActionState` in `backend/src/models/player.rs`).
#[derive(Debug, Clone, PartialEq)]
pub enum Activity {
    Extracting {
        node_id: Uuid,
        progress: u32,
        duration: u32,
    },
    Crafting {
        operation_id: Uuid,
        recipe_id: String,
        recipe_name: String,
        progress: u32,
        duration: u32,
    },
}

impl Activity {
    /// Progress from 0 to 1, for a ring or a bar.
    pub fn fraction(&self) -> f32 {
        let (progress, duration) = match self {
            Activity::Extracting {
                progress, duration, ..
            }
            | Activity::Crafting {
                progress, duration, ..
            } => (*progress, *duration),
        };
        if duration == 0 {
            0.0
        } else {
            (progress as f32 / duration as f32).min(1.0)
        }
    }
}

/// The station the local player has open (`station_opened`).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenStation {
    pub station_id: Uuid,
    pub station_type: String,
    pub name: String,
    pub container_id: Uuid,
}

/// What an extraction or a craft produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Yield {
    pub item_type: String,
    pub item_name: String,
    /// 0 for a failed pull (D5, #206): a gather with hands or a tool too weak
    /// for the resource can come away with nothing, and the node stays.
    pub quantity: i32,
    /// 0–100. Grade it with [`rules::quality_grade`].
    pub quality: i32,
}

/// Whether an `error` naming `error_command` answers a `command` we sent
/// (both `ClientMessage` tags). An error from a server older than #89 names
/// no command, so it's taken as the answer to whatever is out.
pub fn answers(error_command: &str, command: &str) -> bool {
    error_command.is_empty() || error_command == command
}

/// A change the store made, for the UI and the world to react to. The state
/// itself is already updated when these come out.
#[derive(Debug, Clone, PartialEq)]
pub enum StateEvent {
    /// The connection state changed. See [`GameState::connection`].
    Connection(ConnectionState),
    /// What the client holds is missing or out of date: refetch containers,
    /// nearby nodes and stations, currency, plots and recipes. Sent on the
    /// first `Connected` and after every reconnect.
    SnapshotStale,
    /// The server's `error` message, usually a rejected command.
    /// `command` is the refused message's `"type"` (`"place_station"`),
    /// empty from a server older than #89. Test it with [`answers`].
    ServerError {
        message: String,
        command: String,
    },

    /// A player came into sight: online near us, walked near (C0.4), or
    /// was in the snapshot after a reconnect.
    PlayerJoined(Uuid),
    /// A player went out of sight: they left the world, or walked out of
    /// our interest cells (C0.4).
    PlayerLeft(Uuid),
    /// A `positions` snapshot arrived. Positions are server truth, 100 ms apart.
    PositionsUpdated,
    /// The local player hit water. Their server position is now `(x, y)`.
    /// `water` is the ring's id (`moat`, `canal_inner`, ...), empty from a
    /// server older than #74.
    MovementBlocked {
        reason: String,
        water: String,
        x: f64,
        y: f64,
    },
    /// The local player's zone. `from` is `None` for the first one, which is
    /// worked out from the first known position, not sent by the server.
    ZoneChanged {
        from: Option<Zone>,
        to: Zone,
    },

    /// The local player started extracting.
    ExtractionStarted {
        node_id: Uuid,
    },
    ExtractionCompleted {
        node_id: Uuid,
        item: Yield,
    },
    ExtractionCancelled,
    /// The server refused the local player's `extract`. Nothing had started,
    /// so whatever the player was doing carries on.
    ExtractionFailed {
        reason: String,
    },
    /// The local player started crafting, by hand or at a station.
    CraftingStarted {
        recipe_id: String,
        recipe_name: String,
    },
    CraftingCompleted {
        item: Yield,
    },
    CraftingCancelled,
    CraftingFailed {
        reason: String,
    },
    /// Another player's activity started or stopped. See [`GameState::activity`].
    ActivityChanged {
        player_id: Uuid,
    },

    NodeAdded(Uuid),
    NodeChanged(Uuid),
    NodeRemoved(Uuid),

    StationAdded(Uuid),
    StationChanged(Uuid),
    StationRemoved(Uuid),
    /// `station_placed`: someone placed a station just now. When `owner_id` is
    /// the local player, a kit left their inventory.
    StationPlaced {
        station_id: Uuid,
        owner_id: Uuid,
    },
    /// The local player opened a station. Its container still has to be fetched.
    StationOpened {
        station_id: Uuid,
        container_id: Uuid,
    },
    StationClosed,

    /// The server says a container the local player holds or has open changed.
    InventoryChanged {
        container_id: Uuid,
    },
    /// New `/inventory/containers` contents.
    ContainersUpdated,
    /// New contents for the open station's container.
    StationContainerUpdated,
    RecipesUpdated,
    /// A new `/catalog`: names, slot counts, station kinds. Worth caching.
    CatalogUpdated,
    /// `/health` reports a catalog version other than the store's.
    CatalogStale,
    CurrencyChanged {
        balance: i64,
    },
    /// Your plots changed (`GameState::plots`), or the picked one did.
    PlotsUpdated,
    PlotClaimed {
        plot_id: Uuid,
    },
    /// A `/plots/at` answer picked a plot for the plot panel, or found
    /// none there (`None`), which closes it. See [`GameState::picked_plot`].
    PlotPicked {
        plot_id: Option<Uuid>,
    },
    /// Refetch `/terrain`.
    TerrainUpdated,
    /// The Gazette's edition about `day` is out (`gazette_published`, E3.9)
    GazettePublished {
        day: i64,
        number: i64,
        headline: String,
    },
    /// Someone said a line you hear (`said`, Commons S1.1): a local one near
    /// you, or one on the Capitol channel. Speech over heads (S1.2) and the
    /// chat panel (S1.3) show it.
    Said {
        from: Uuid,
        name: String,
        /// The speaker is a citizen (C0.1)
        citizen: bool,
        channel: crate::protocol::ws::Channel,
        text: String,
        x: f64,
        y: f64,
    },
    /// A letter came for you (`letter_arrived`, Commons S1.5): live, or as
    /// you come back. The mailbox (`crate::mail`) counts it unread.
    LetterArrived {
        id: Uuid,
        from: Uuid,
        name: String,
        /// The writer is a citizen (C0.1)
        citizen: bool,
        first_line: String,
    },
    /// Your guild changed, or you joined or left one (`guild_changed`,
    /// Commons S4.1): read `GET /player/guild` again. The guild panel is
    /// S4.4's (#400).
    GuildChanged { guild_id: Uuid },
    /// Something in a guild that names you (`guild_notice`, S4.1): live, or
    /// as you come back.
    GuildNotice {
        guild_id: Uuid,
        guild: String,
        tag: String,
        kind: crate::protocol::ws::GuildNoticeKind,
        /// Who invited you, applied, expelled you or held the lead
        who: Uuid,
        name: String,
        /// `who` is a citizen (C0.1)
        citizen: bool,
    },
    /// Someone bought from your bazaar stall (`bazaar_sale`, E3.10). Flint
    /// toasts it (E3.11).
    BazaarSale {
        stall_id: String,
        item_type: String,
        buyer_name: String,
        /// The buyer is a citizen (C0.1)
        buyer_citizen: bool,
        quantity: i64,
        /// Which units went, each at its quality (D2)
        units: crate::protocol::rules::stacking::Units,
        paid: i64,
    },
    /// Your stall's rent ran out and the shop closed (`stall_lapsed`,
    /// E3.10): `returned` stacks went to your market box, `waiting` wait for
    /// room there.
    StallLapsed {
        stall_id: String,
        name: String,
        returned: u32,
        waiting: u32,
    },
    /// Someone delivered to your commission (`commission_delivered`,
    /// Commons S2.1) and was paid from its escrow. `left` 0 means it's
    /// filled. Flint toasts it, as a bazaar sale.
    CommissionDelivered {
        item_type: String,
        deliverer_name: String,
        /// The deliverer is a citizen (C0.1)
        deliverer_citizen: bool,
        units: crate::protocol::rules::stacking::Units,
        paid: i64,
        left: i32,
    },
    /// Your commission ran past its due date (`commission_expired`, Commons
    /// S2.1), and `refunded` of its escrow came back.
    CommissionExpired {
        item_type: String,
        delivered: i32,
        quantity: i32,
        refunded: i64,
    },
    /// A bazaar answer changed [`GameState::bazaar`]: the directory, a shop
    /// seen at its stall, or yours.
    BazaarUpdated,
    /// Someone, maybe you, claimed a lot (`plot_claimed`, B13). It's in
    /// [`GameState::claimed_lots`] now.
    LotClaimed(ClaimedPlot),
    /// `/plots/claimed` answered: [`GameState::claimed_lots`] is the list.
    ClaimedLotsUpdated,
    /// A lot has no owner any more (`plot_released`, B13). It's gone from
    /// [`GameState::claimed_lots`], and if it was yours, from
    /// [`GameState::plots`] too, and `PlotsUpdated` came first.
    LotReleased {
        terrain_id: String,
    },
    /// Your plot `terrain_id` owes `tax_owed` in property tax, and is
    /// foreclosed at `forecloses_at` (`tax_delinquent`, #131).
    TaxDelinquent {
        terrain_id: String,
        tax_owed: i64,
        forecloses_at: i64,
    },
    /// Your plot `terrain_id` was foreclosed for unpaid tax, and `refund`
    /// of the deed came back (`plot_foreclosed`, #131). `impounded` is what
    /// your inventory had no room for, waiting in your impound (#141), and
    /// `lost` what found no room even there. `LotReleased` follows.
    PlotForeclosed {
        terrain_id: String,
        tax_owed: i64,
        refund: i64,
        impounded: Vec<ItemCount>,
        lost: Vec<ItemCount>,
    },
    /// Your workbench stood off a plot where a terrain change laid the lot
    /// `terrain_id`, so the server took it down and gave back what it held
    /// (`station_displaced`, #188). `StationRemoved` comes alongside.
    StationDisplaced {
        station_id: Uuid,
        station_type: String,
        terrain_id: String,
    },
    /// Your subscription ran out at `until` (`subscription_lapsed`, E4.1).
    SubscriptionLapsed {
        until: DateTime<Utc>,
    },
    /// Your station paused for `reason`, and goes on by itself once that
    /// clears (`automation_paused`, E4.3).
    AutomationPaused {
        station_id: Uuid,
        station_type: String,
        reason: PauseReason,
    },
    /// Your station made something on its own (`automation_produced`,
    /// E4.3). It's on the station's production log already.
    AutomationProduced {
        station_id: Uuid,
    },

    /// One of your tools used its last durability and is gone (`tool_broke`,
    /// E1.4). An `inventory_updated` for its container comes with it.
    ToolBroke {
        item_id: Uuid,
        item_type: String,
        item_name: String,
    },

    /// The local player's trading changed. See [`GameState::trade`].
    Trade(TradeEvent),

    /// The market changed: a book, a trade, your orders (E3.3). See
    /// [`GameState::market`]. Boxed: an order and its trade are big.
    Market(Box<MarketEvent>),

    /// This client took the avatar, or another took it from it (E0.8).
    /// See [`GameState::presence`].
    Presence(Presence),
}

/// Whether this client drives the avatar (`presence`, E0.8, D9). An account
/// can be signed in on several devices, but only one plays; the others watch
/// and can still use the market and automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presence {
    pub yours: bool,
    /// Who plays when it isn't you: another of your clients, or a bot.
    /// `None`: nobody, and your avatar is out of the world.
    pub held_by: Option<PresenceHolder>,
}

/// The client's mirror of the world.
#[derive(Debug, Clone)]
pub struct GameState {
    connection: ConnectionState,
    ever_connected: bool,
    me: Option<Uuid>,
    /// The server's clock minus this machine's, in ms, from the last `AuthOk`.
    clock_offset_ms: Option<i64>,

    players: HashMap<Uuid, PlayerPosition>,
    activities: HashMap<Uuid, Activity>,
    zone: Option<Zone>,

    nodes: HashMap<Uuid, ResourceNodeInfo>,
    stations: HashMap<Uuid, StationInfo>,
    /// Set by a reconnect: the next nearby response replaces rather than merges,
    /// which drops whatever was removed while the client was away. Nodes
    /// need no flag: the whole list (`/world/nodes`) always replaces, and
    /// the nearby one only ever merges.
    stations_stale: bool,

    containers: HashMap<Uuid, Container>,
    open_station: Option<OpenStation>,
    station_container: Option<Container>,

    /// The static game data: the fallback until a cache or `/catalog` replaces it.
    catalog: Catalog,
    /// From `/catalog` (or an old `/recipes`); empty until the server answers.
    recipes: Vec<RecipeInfo>,
    strands: Option<i64>,
    /// Your account's subscription, from `auth_ok` and `/player/me` (E4.1).
    subscription: Option<Subscription>,
    /// The fees the server charges now, from `auth_ok` and `fees_changed`
    /// (D6.1). The defaults until it says.
    fees: Fees,
    /// The token is an admin's, by `auth_ok` (D5.3)
    is_admin: bool,
    /// Whether this client plays the avatar, as the server last said (E0.8).
    /// `None` while not connected, or from a server too old to say.
    presence: Option<Presence>,
    /// Your own plots: the poller lists them by owner (P6.4).
    plots: Vec<PlotInfo>,
    /// The plot the plot panel shows, picked with `/plots/at`.
    picked_plot: Option<PlotInfo>,
    /// Everyone's claimed lots, by `terrain_id` (P8.6b): `/plots/claimed`
    /// with the snapshot, then WS `plot_claimed` and `plot_released`.
    claimed: HashMap<String, ClaimedPlot>,
    /// Counts every change to `claimed`, so a drawer can tell it moved.
    claimed_version: u64,

    trade: trade::TradeBook,
    /// The books you watch and your orders (E3.3)
    market: MarketStore,
    /// The bazaar's directory, the shops seen, and yours (E3.10)
    bazaar: BazaarStore,
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

impl GameState {
    pub fn new() -> Self {
        Self {
            connection: ConnectionState::Disconnected {
                reason: "not connected".into(),
            },
            ever_connected: false,
            me: None,
            clock_offset_ms: None,
            players: HashMap::new(),
            activities: HashMap::new(),
            zone: None,
            nodes: HashMap::new(),
            stations: HashMap::new(),
            stations_stale: false,
            containers: HashMap::new(),
            open_station: None,
            station_container: None,
            catalog: Catalog::fallback(),
            recipes: Vec::new(),
            strands: None,
            subscription: None,
            fees: Fees::default(),
            is_admin: false,
            presence: None,
            plots: Vec::new(),
            picked_plot: None,
            claimed: HashMap::new(),
            claimed_version: 0,
            trade: trade::TradeBook::default(),
            market: MarketStore::new(),
            bazaar: BazaarStore::default(),
        }
    }

    // -----------------------------------------------------------------------
    // Applying
    // -----------------------------------------------------------------------

    /// Applies anything the net layer reports. REST failures change nothing;
    /// the caller sees them on the `NetEvent` itself.
    pub fn apply(&mut self, event: &NetEvent) -> Vec<StateEvent> {
        match event {
            NetEvent::Ws(message) => self.apply_ws(message),
            NetEvent::Rest(_, Ok(response)) => self.apply_rest(response),
            NetEvent::Rest(_, Err(_)) => Vec::new(),
            NetEvent::Connection(state) => self.apply_connection(state),
            NetEvent::Resync => self.apply_resync(),
            // The session's business (`auth`), not the world's.
            NetEvent::Refreshed { .. } => Vec::new(),
        }
    }

    pub fn apply_connection(&mut self, state: &ConnectionState) -> Vec<StateEvent> {
        let mut out = Vec::new();
        self.connection = state.clone();
        out.push(StateEvent::Connection(state.clone()));
        if !matches!(state, ConnectionState::Connected { .. }) {
            self.trade_connection_lost(&mut out);
            // A new connection hears its own.
            self.presence = None;
        }
        if let ConnectionState::Connected { player_id } = state {
            self.me = Some(*player_id);
            // A new session watches no market channel until asked again
            self.market.connected();
            if !self.ever_connected {
                self.ever_connected = true;
                out.push(StateEvent::SnapshotStale);
            }
        }
        out
    }

    /// The session reconnected. The server dropped the player while it was
    /// away, which cancelled their action and any trade, closed any open
    /// station, and whatever happened meanwhile was missed. A socket that fell
    /// behind the server's events is told `resync` and lands here too (C0.3):
    /// its trade was cancelled, and it missed events just the same.
    pub fn apply_resync(&mut self) -> Vec<StateEvent> {
        let mut out = Vec::new();
        self.trade_connection_lost(&mut out);
        for (player_id, _) in self.activities.drain() {
            out.push(StateEvent::ActivityChanged { player_id });
        }
        if self.open_station.take().is_some() {
            self.station_container = None;
            out.push(StateEvent::StationClosed);
        }
        self.stations_stale = true;
        out.push(StateEvent::SnapshotStale);
        out
    }

    pub fn apply_ws(&mut self, message: &ServerMessage) -> Vec<StateEvent> {
        use ServerMessage as M;
        let mut out = Vec::new();
        match message {
            M::AuthOk {
                player_id,
                server_time_ms,
                subscription,
                fees,
                is_admin,
            } => {
                self.me = Some(*player_id);
                self.subscription = Some(*subscription);
                self.fees = *fees;
                self.is_admin = *is_admin;
                // Rebased on every AuthOk, so a reconnect corrects any drift.
                self.clock_offset_ms = Some(server_time_ms - local_unix_ms());
            }
            // The net layer turns these into connection states.
            M::AuthError { .. } | M::Subscribed { .. } => {}
            M::Error { message, command } => {
                out.push(StateEvent::ServerError {
                    message: message.clone(),
                    command: command.clone(),
                });
                self.trade_error(message, command, &mut out);
            }

            M::Positions { players } => self.positions(players, &mut out),
            M::PlayerJoined { player } => {
                if self.players.insert(player.id, player.clone()).is_none() {
                    out.push(StateEvent::PlayerJoined(player.id));
                }
                self.learn_zone(&mut out);
            }
            M::PlayerLeft { player_id } => self.remove_player(*player_id, &mut out),
            M::MovementBlocked {
                player_id,
                reason,
                water_id,
                stopped_x,
                stopped_y,
            } => {
                if self.is_me(*player_id) {
                    if let Some(me) = self.players.get_mut(player_id) {
                        me.x = *stopped_x;
                        me.y = *stopped_y;
                    }
                    out.push(StateEvent::MovementBlocked {
                        reason: reason.clone(),
                        water: water_id.clone(),
                        x: *stopped_x,
                        y: *stopped_y,
                    });
                }
            }
            M::ZoneChanged {
                player_id, to_zone, ..
            } => {
                if self.is_me(*player_id) {
                    match Zone::from_str(to_zone) {
                        Some(to) => self.set_zone(to, &mut out),
                        None => tracing::warn!("zone_changed to unknown zone {to_zone:?}"),
                    }
                }
            }

            M::ExtractionStarted {
                player_id,
                node_id,
                duration_ticks,
            } => {
                let activity = Activity::Extracting {
                    node_id: *node_id,
                    progress: 0,
                    duration: *duration_ticks,
                };
                let local = StateEvent::ExtractionStarted { node_id: *node_id };
                self.start(*player_id, activity, local, &mut out);
            }
            M::ExtractionProgress {
                player_id,
                progress: p,
                duration: d,
            } => {
                if let Some(Activity::Extracting {
                    progress, duration, ..
                }) = self.activities.get_mut(player_id)
                {
                    *progress = *p;
                    *duration = *d;
                }
            }
            M::ExtractionCompleted {
                player_id,
                node_id,
                item_type,
                item_name,
                quantity,
                quality,
                // The tool's wear arrives with the inventory refetch that follows
                tool: _,
            } => {
                let local = StateEvent::ExtractionCompleted {
                    node_id: *node_id,
                    item: Yield {
                        item_type: item_type.clone(),
                        item_name: item_name.clone(),
                        quantity: *quantity,
                        quality: *quality,
                    },
                };
                self.finish(*player_id, local, &mut out);
            }
            M::ExtractionCancelled { player_id } => {
                self.finish(*player_id, StateEvent::ExtractionCancelled, &mut out)
            }
            // Not `finish`: a refusal ends nothing. "Already performing an
            // action" arrives while the activity it names is still running.
            M::ExtractionFailed { player_id, reason } => {
                if self.is_me(*player_id) {
                    out.push(StateEvent::ExtractionFailed {
                        reason: reason.clone(),
                    });
                }
            }
            M::NodeDepleted { node_id } => self.set_node_state(*node_id, "depleted", &mut out),
            M::NodeRegenerated { node_id } => self.set_node_state(*node_id, "available", &mut out),

            M::CraftingStarted {
                player_id,
                operation_id,
                recipe_id,
                recipe_name,
                duration_ticks,
            } => {
                let activity = Activity::Crafting {
                    operation_id: *operation_id,
                    recipe_id: recipe_id.clone(),
                    recipe_name: recipe_name.clone(),
                    progress: 0,
                    duration: *duration_ticks,
                };
                let local = StateEvent::CraftingStarted {
                    recipe_id: recipe_id.clone(),
                    recipe_name: recipe_name.clone(),
                };
                self.start(*player_id, activity, local, &mut out);
            }
            M::CraftingProgress {
                player_id,
                progress: p,
                duration: d,
                ..
            } => {
                if let Some(Activity::Crafting {
                    progress, duration, ..
                }) = self.activities.get_mut(player_id)
                {
                    *progress = *p;
                    *duration = *d;
                }
            }
            M::CraftingCompleted {
                player_id,
                item_type,
                item_name,
                quantity,
                quality,
                ..
            } => {
                let local = StateEvent::CraftingCompleted {
                    item: Yield {
                        item_type: item_type.clone(),
                        item_name: item_name.clone(),
                        quantity: *quantity,
                        quality: *quality,
                    },
                };
                self.finish(*player_id, local, &mut out);
            }
            M::CraftingCancelled { player_id, .. } => {
                self.finish(*player_id, StateEvent::CraftingCancelled, &mut out)
            }
            M::CraftingFailed { player_id, reason } => {
                let local = StateEvent::CraftingFailed {
                    reason: reason.clone(),
                };
                self.finish(*player_id, local, &mut out);
            }

            // The player who changed a container isn't always the one holding it
            // (two players at one storage chest), so either counts.
            M::InventoryUpdated {
                player_id,
                container_id,
            } => self.inventory_changed(*player_id, *container_id, &mut out),

            M::StationPlaced {
                station_id,
                station_type,
                name,
                owner_id,
                x,
                y,
                container_id,
                quality,
            } => {
                let kind = self.catalog.station_type(station_type);
                let station = StationInfo {
                    id: *station_id,
                    station_type: station_type.clone(),
                    name: name.clone(),
                    category: kind.map_or("crafting", |k| k.category.as_str()).to_string(),
                    icon: kind.and_then(|k| k.icon.clone()),
                    x: *x,
                    y: *y,
                    owner_id: *owner_id,
                    container_id: Some(*container_id),
                    interaction_range: kind.map_or(rules::STATION_INTERACTION_RANGE as f32, |k| {
                        k.interaction_range
                    }),
                    quality: *quality,
                    tool: None,
                    automation: None,
                    production: Vec::new(),
                };
                self.upsert_station(station, &mut out);
                out.push(StateEvent::StationPlaced {
                    station_id: *station_id,
                    owner_id: *owner_id,
                });
            }
            M::StationRemoved { station_id } => {
                if self.stations.remove(station_id).is_some() {
                    out.push(StateEvent::StationRemoved(*station_id));
                }
                if self.open_station.as_ref().map(|s| s.station_id) == Some(*station_id) {
                    self.close_station(&mut out);
                }
            }
            M::StationOpened {
                player_id,
                station_id,
                station_type,
                name,
                container_id,
            } => {
                if self.is_me(*player_id) {
                    self.open_station = Some(OpenStation {
                        station_id: *station_id,
                        station_type: station_type.clone(),
                        name: name.clone(),
                        container_id: *container_id,
                    });
                    self.station_container = None;
                    out.push(StateEvent::StationOpened {
                        station_id: *station_id,
                        container_id: *container_id,
                    });
                }
            }
            // Kept on the station for its panel's tool slot (E1.7)
            M::StationToolChanged { station_id, tool } => {
                if let Some(station) = self.stations.get_mut(station_id) {
                    station.tool = tool.clone();
                }
            }
            // Kept on the station for E4.4's automation panel
            M::StationAutomationChanged { station_id, automation } => {
                if let Some(station) = self.stations.get_mut(station_id) {
                    station.automation = Some(automation.clone());
                }
            }
            M::StationClosed { player_id } => {
                if self.is_me(*player_id) && self.open_station.is_some() {
                    self.close_station(&mut out);
                }
            }

            M::TradeRequested { .. }
            | M::TradeRequestDeclined { .. }
            | M::TradeRequestExpired { .. }
            | M::TradeStarted { .. }
            | M::TradeOfferUpdated { .. }
            | M::TradeAccepted { .. }
            | M::TradeExecuted { .. }
            | M::TradeCancelled { .. } => self.apply_trade(message, &mut out),

            // Only ever sent to the player whose balance it is.
            M::CurrencyChanged { new_balance } => self.set_strands(*new_balance, &mut out),
            // Only ever sent to the tool's owner.
            M::ToolBroke {
                item_id,
                item_type,
                item_name,
            } => out.push(StateEvent::ToolBroke {
                item_id: *item_id,
                item_type: item_type.clone(),
                item_name: item_name.clone(),
            }),
            M::TerrainUpdated => out.push(StateEvent::TerrainUpdated),
            // E3.9: to everyone. The session asks for the edition.
            // D6.1: read where they're quoted, so nothing more to do
            M::FeesChanged { fees } => self.fees = *fees,
            M::GazettePublished { day, number, headline } => out.push(StateEvent::GazettePublished {
                day: *day,
                number: *number,
                headline: headline.clone(),
            }),
            // Commons S1.1: the server sends only what you hear
            M::Said { from, name, citizen, channel, text, x, y } => out.push(StateEvent::Said {
                from: *from,
                name: name.clone(),
                citizen: *citizen,
                channel: *channel,
                text: text.clone(),
                x: *x,
                y: *y,
            }),
            M::LetterArrived { id, from, name, citizen, first_line } => out.push(StateEvent::LetterArrived {
                id: *id,
                from: *from,
                name: name.clone(),
                citizen: *citizen,
                first_line: first_line.clone(),
            }),
            M::GuildChanged { guild_id } => out.push(StateEvent::GuildChanged { guild_id: *guild_id }),
            M::GuildNotice { guild_id, guild, tag, kind, who, name, citizen } => out.push(StateEvent::GuildNotice {
                guild_id: *guild_id,
                guild: guild.clone(),
                tag: tag.clone(),
                kind: *kind,
                who: *who,
                name: name.clone(),
                citizen: *citizen,
            }),
            // E3.10: to the merchant. The store takes the sale off your
            // listing, or closes your shop.
            M::BazaarSale { stall_id, item_type, buyer_name, buyer_citizen, units, paid, .. } => {
                self.bazaar.apply_ws(message);
                out.push(StateEvent::BazaarSale {
                    stall_id: stall_id.clone(),
                    item_type: item_type.clone(),
                    buyer_name: buyer_name.clone(),
                    buyer_citizen: *buyer_citizen,
                    quantity: units.count(),
                    units: units.clone(),
                    paid: *paid,
                });
            }
            // Commons S2.1: the hall's board, to everyone. Nothing here keeps
            // the board yet: the Wanted page's store does (S2.3, #389).
            M::CommissionPosted { .. } | M::CommissionUpdated { .. } | M::CommissionClosed { .. } => {}
            // Commons S2.1: to the poster
            M::CommissionDelivered { item_type, deliverer_name, deliverer_citizen, units, paid, left, .. } => {
                out.push(StateEvent::CommissionDelivered {
                    item_type: item_type.clone(),
                    deliverer_name: deliverer_name.clone(),
                    deliverer_citizen: *deliverer_citizen,
                    units: units.clone(),
                    paid: *paid,
                    left: *left,
                })
            }
            M::CommissionExpired { item_type, delivered, quantity, refunded, .. } => {
                out.push(StateEvent::CommissionExpired {
                    item_type: item_type.clone(),
                    delivered: *delivered,
                    quantity: *quantity,
                    refunded: *refunded,
                })
            }
            M::StallLapsed { stall_id, name, returned, waiting } => {
                self.bazaar.apply_ws(message);
                out.push(StateEvent::StallLapsed {
                    stall_id: stall_id.clone(),
                    name: name.clone(),
                    returned: *returned,
                    waiting: *waiting,
                });
            }
            M::PlotClaimed {
                terrain_id,
                owner_id,
                claimed_at,
            } => {
                let lot = ClaimedPlot {
                    terrain_id: terrain_id.clone(),
                    owner_id: *owner_id,
                    claimed_at: *claimed_at,
                };
                self.claimed.insert(terrain_id.clone(), lot.clone());
                self.claimed_version += 1;
                out.push(StateEvent::LotClaimed(lot));
            }
            M::PlotReleased { terrain_id } => {
                if self.claimed.remove(terrain_id).is_some() {
                    self.claimed_version += 1;
                }
                if self.release_plot(terrain_id) {
                    out.push(StateEvent::PlotsUpdated);
                }
                out.push(StateEvent::LotReleased {
                    terrain_id: terrain_id.clone(),
                });
            }
            // Only ever sent to the plot's owner. The plot panel shows the debt
            // as it grows, not as it was when the plot was picked (#138).
            M::TaxDelinquent {
                terrain_id,
                tax_owed,
                forecloses_at,
            } => {
                if self.owe_tax(terrain_id, *tax_owed, *forecloses_at) {
                    out.push(StateEvent::PlotsUpdated);
                }
                out.push(StateEvent::TaxDelinquent {
                    terrain_id: terrain_id.clone(),
                    tax_owed: *tax_owed,
                    forecloses_at: *forecloses_at,
                })
            }
            M::PlotForeclosed {
                terrain_id,
                tax_owed,
                refund,
                impounded,
                lost,
            } => out.push(StateEvent::PlotForeclosed {
                terrain_id: terrain_id.clone(),
                tax_owed: *tax_owed,
                refund: *refund,
                impounded: impounded.clone(),
                lost: lost.clone(),
            }),
            M::StationDisplaced {
                station_id,
                station_type,
                terrain_id,
            } => out.push(StateEvent::StationDisplaced {
                station_id: *station_id,
                station_type: station_type.clone(),
                terrain_id: terrain_id.clone(),
            }),
            M::Resync { .. } => out.extend(self.apply_resync()),
            M::Presence { yours, held_by } => {
                let presence = Presence {
                    yours: *yours,
                    held_by: *held_by,
                };
                self.presence = Some(presence);
                out.push(StateEvent::Presence(presence));
            }
            M::SubscriptionLapsed { until } => {
                self.subscription = Some(Subscription {
                    status: SubscriptionStatus::Lapsed,
                    until: Some(*until),
                });
                out.push(StateEvent::SubscriptionLapsed { until: *until });
            }
            // E3.3: the market store holds these; the panel is E3.5 (#171)
            M::OrderPlaced { .. }
            | M::OrderFilled { .. }
            | M::OrderCancelled { .. }
            | M::MarketDepth { .. }
            | M::MarketTrade { .. } => {
                out.extend(self.market.apply_ws(message).into_iter().map(|e| StateEvent::Market(Box::new(e))));
            }
            // E4.3: your station made something on its own. Kept on the
            // station for E4.4's production log, the newest last.
            M::AutomationProduced {
                station_id,
                produced,
                done,
            } => {
                if let Some(station) = self.stations.get_mut(station_id) {
                    if let Some(automation) = station.automation.as_mut() {
                        automation.done = *done;
                    }
                    if station.production.len() == PRODUCTION_LOG_LEN {
                        station.production.remove(0);
                    }
                    station.production.push(produced.clone());
                    out.push(StateEvent::AutomationProduced {
                        station_id: *station_id,
                    });
                }
            }
            // Its state comes in `station_automation_changed`; this is the
            // owner's notice of why, live or on coming back
            M::AutomationPaused {
                station_id,
                station_type,
                reason,
            } => out.push(StateEvent::AutomationPaused {
                station_id: *station_id,
                station_type: station_type.clone(),
                reason: reason.clone(),
            }),
        }
        out
    }

    /// Applies a successful REST response. Responses the world doesn't mirror
    /// (terrain, health, move and extract acks, permissions, zones, deed
    /// prices, the flat `/player/inventory`) change nothing.
    pub fn apply_rest(&mut self, response: &RestResponse) -> Vec<StateEvent> {
        use RestResponse as R;
        let mut out = Vec::new();
        match response {
            R::Auth(auth) => self.me = Some(auth.player_id),
            R::Me(me) => {
                self.me = Some(me.player_id);
                self.set_strands(me.strand_balance, &mut out);
                self.subscription = Some(me.subscription);
                // Before the first `positions`, this is how we learn who and
                // where we are. After it, `positions` is newer than any REST
                // answer, so a player already known keeps their position.
                if me.online && self.is_connected() && !self.players.contains_key(&me.player_id) {
                    self.players.insert(
                        me.player_id,
                        PlayerPosition {
                            id: me.player_id,
                            name: me.name.clone(),
                            x: me.x,
                            y: me.y,
                            citizen: me.citizen,
                        },
                    );
                    out.push(StateEvent::PlayerJoined(me.player_id));
                    self.learn_zone(&mut out);
                }
            }
            R::Nearby(nearby) => self.merge_nodes(&nearby.nodes, false, &mut out),
            R::AllNodes(all) => self.merge_nodes(&all.nodes, true, &mut out),
            R::NearbyStations(nearby) => self.merge_stations(&nearby.stations, &mut out),
            R::PlaceStation(placed) => self.upsert_station(placed.station.clone(), &mut out),
            R::Containers(response) => {
                self.containers = response
                    .containers
                    .iter()
                    .map(|c| (c.id, Container::from(c)))
                    .collect();
                out.push(StateEvent::ContainersUpdated);
            }
            R::StationContainer(container) => {
                // A late answer for a station that has since closed is dropped.
                if self.open_station.as_ref().map(|s| s.container_id) == Some(container.id) {
                    self.station_container = Some(Container::from(container));
                    out.push(StateEvent::StationContainerUpdated);
                }
            }
            R::Recipes(response) => {
                self.recipes = response.recipes.clone();
                out.push(StateEvent::RecipesUpdated);
            }
            R::Catalog(response) => {
                self.recipes = response.recipes.clone();
                self.catalog = Catalog::new(CatalogResponse::clone(response));
                out.push(StateEvent::CatalogUpdated);
                out.push(StateEvent::RecipesUpdated);
            }
            R::Health(health) => {
                if health
                    .catalog_version
                    .as_deref()
                    .is_some_and(|v| v != self.catalog.version())
                {
                    out.push(StateEvent::CatalogStale);
                }
            }
            R::Currency(c) => self.set_strands(c.strand_balance, &mut out),
            R::Subscription(s) => self.subscription = Some(*s),
            R::Granted(g) => self.set_strands(g.strands, &mut out),
            R::Order(order) => out.extend(self.market.answered(order).into_iter().map(|e| StateEvent::Market(Box::new(e)))),
            R::MyOrders(orders) => out.extend(self.market.set_orders(orders).into_iter().map(|e| StateEvent::Market(Box::new(e)))),
            R::MarketDepth(depth) => out.extend(self.market.set_depth(depth).into_iter().map(|e| StateEvent::Market(Box::new(e)))),
            R::MarketBox(container) => {
                self.containers.insert(container.id, Container::from(container));
                out.push(StateEvent::ContainersUpdated);
            }
            R::Deposit(d) => self.set_strands(d.new_balance, &mut out),
            R::Withdraw(w) => self.set_strands(w.new_balance, &mut out),
            // The poller lists your own plots. Another listing (a bot looking
            // for unclaimed land, E2.7) mustn't replace them.
            R::Plots(response) if response.plots.iter().any(|p| p.owner_id != self.me) => {}
            R::Plots(response) => {
                self.plots = response.plots.clone();
                // A picked plot of yours is fresher in the list.
                if let Some(picked) = &mut self.picked_plot {
                    if let Some(p) = self.plots.iter().find(|p| p.id == picked.id) {
                        *picked = p.clone();
                    }
                }
                out.push(StateEvent::PlotsUpdated);
            }
            R::ClaimedPlots(lots) => {
                self.claimed = lots
                    .iter()
                    .map(|lot| (lot.terrain_id.clone(), lot.clone()))
                    .collect();
                self.claimed_version += 1;
                out.push(StateEvent::ClaimedLotsUpdated);
            }
            R::Plot(plot) => {
                if self.learn_plot(plot) {
                    out.push(StateEvent::PlotsUpdated);
                }
            }
            R::PlotAt(Some(plot)) => {
                self.learn_plot(plot);
                self.picked_plot = Some(plot.clone());
                out.push(StateEvent::PlotPicked {
                    plot_id: Some(plot.id),
                });
            }
            R::PlotAt(None) => {
                if self.picked_plot.take().is_some() {
                    out.push(StateEvent::PlotPicked { plot_id: None });
                }
            }
            R::PayTax(paid) => {
                if self.learn_plot(&paid.plot) {
                    out.push(StateEvent::PlotsUpdated);
                }
                self.set_strands(paid.new_balance, &mut out);
            }
            R::ClaimPlot(claim) => {
                self.learn_plot(&claim.plot);
                out.push(StateEvent::PlotsUpdated);
                out.push(StateEvent::PlotClaimed {
                    plot_id: claim.plot.id,
                });
                self.set_strands(claim.new_balance, &mut out);
            }
            // `station_automation_changed` says the same to everyone
            R::Automation { station_id, automation } => {
                if let Some(station) = self.stations.get_mut(station_id) {
                    station.automation = Some(automation.clone());
                }
            }
            R::Bazaar(directory) => {
                self.bazaar.set_directory(directory);
                out.push(StateEvent::BazaarUpdated);
            }
            R::MyStall(mine) => {
                self.bazaar.set_mine(mine);
                out.push(StateEvent::BazaarUpdated);
            }
            R::Shop(shop) => {
                self.bazaar.seen(shop, self.me);
                out.push(StateEvent::BazaarUpdated);
            }
            R::Listed(listing) => {
                self.bazaar.listed(listing);
                out.push(StateEvent::BazaarUpdated);
            }
            R::Unlisted { listing_id } => {
                self.bazaar.unlisted(*listing_id);
                out.push(StateEvent::BazaarUpdated);
            }
            R::Bought { listing_id, purchase } => {
                self.bazaar.bought(*listing_id, purchase);
                out.push(StateEvent::BazaarUpdated);
            }
            R::Success(_)
            | R::DeedPrices(_)
            | R::Terrain(_)
            | R::TerrainUnchanged
            | R::Move(_)
            | R::Inventory(_)
            | R::Extract(_)
            | R::MoveItem(_)
            | R::SplitStack(_)
            | R::PlotPermissions(_)
            | R::Zones(_)
            | R::MarketHistory(_)
            | R::MarketSummary(_)
            | R::Collected(_)
            | R::Gazette(_)
            | R::Card(_)
            | R::Mutes(_)
            | R::Mailbox(_)
            | R::Letter(_)
            | R::LettersUnread(_)
            | R::Byline(_)
            | R::CreatedApiKey(_)
            | R::ApiKeys(_)
            | R::Citizen(_)
            | R::Citizens(_)
            | R::Ledger(_)
            | R::Series(_)
            | R::Annotations(_)
            | R::Tunables(_)
            | R::Walkers(_) => {}
        }
        out
    }

    // -----------------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------------

    pub fn connection(&self) -> &ConnectionState {
        &self.connection
    }

    /// Authenticated and subscribed: events are flowing.
    pub fn is_connected(&self) -> bool {
        matches!(self.connection, ConnectionState::Connected { .. })
    }

    /// The local player's ID, from login or `auth_ok`.
    pub fn me(&self) -> Option<Uuid> {
        self.me
    }

    /// The server's clock now, in Unix ms. `None` until the first `AuthOk`.
    pub fn server_time_ms(&self) -> Option<i64> {
        self.clock_offset_ms.map(|offset| local_unix_ms() + offset)
    }

    /// [`rules::time_of_day`] on the server's clock: 0 midnight, 0.5 noon.
    /// `None` until the first `AuthOk`; offline has no server clock.
    pub fn time_of_day(&self) -> Option<f64> {
        self.server_time_ms().map(capitol_protocol::rules::time_of_day)
    }

    /// The local player's name and server position. Known from `GET
    /// /player/me` in the connect snapshot, or the first `positions` that
    /// includes them, whichever lands first.
    pub fn local_player(&self) -> Option<&PlayerPosition> {
        self.me.and_then(|id| self.players.get(&id))
    }

    /// Everyone online, the local player included, at their last server position.
    pub fn players(&self) -> &HashMap<Uuid, PlayerPosition> {
        &self.players
    }

    pub fn player(&self, id: Uuid) -> Option<&PlayerPosition> {
        self.players.get(&id)
    }

    /// What a player is doing, if anything.
    pub fn activity(&self, player_id: Uuid) -> Option<&Activity> {
        self.activities.get(&player_id)
    }

    pub fn local_activity(&self) -> Option<&Activity> {
        self.me.and_then(|id| self.activities.get(&id))
    }

    /// The local player's extraction: node, progress and duration in ticks.
    pub fn extraction(&self) -> Option<(Uuid, u32, u32)> {
        match self.local_activity()? {
            Activity::Extracting {
                node_id,
                progress,
                duration,
            } => Some((*node_id, *progress, *duration)),
            Activity::Crafting { .. } => None,
        }
    }

    pub fn is_extracting(&self) -> bool {
        self.extraction().is_some()
    }

    pub fn is_crafting(&self) -> bool {
        matches!(self.local_activity(), Some(Activity::Crafting { .. }))
    }

    pub fn zone(&self) -> Option<Zone> {
        self.zone
    }

    /// Resource nodes seen so far. Nodes never move, so ones out of range stay.
    pub fn nodes(&self) -> &HashMap<Uuid, ResourceNodeInfo> {
        &self.nodes
    }

    pub fn node(&self, id: Uuid) -> Option<&ResourceNodeInfo> {
        self.nodes.get(&id)
    }

    /// Who is extracting from a node, as far as the events say. The node's
    /// own `state` only turns to "harvesting" at the next poll.
    pub fn harvester(&self, node_id: Uuid) -> Option<Uuid> {
        self.activities.iter().find_map(|(player, activity)| {
            matches!(activity, Activity::Extracting { node_id: n, .. } if *n == node_id)
                .then_some(*player)
        })
    }

    /// A node that can be extracted from now: known, "available", and
    /// nobody has started on it since the last poll.
    pub fn node_available(&self, id: Uuid) -> bool {
        self.nodes
            .get(&id)
            .is_some_and(|n| n.state == "available" && self.harvester(id).is_none())
    }

    /// Stations seen so far.
    pub fn stations(&self) -> &HashMap<Uuid, StationInfo> {
        &self.stations
    }

    pub fn station(&self, id: Uuid) -> Option<&StationInfo> {
        self.stations.get(&id)
    }

    /// The local player's containers, as `/inventory/containers` last returned them.
    pub fn containers(&self) -> &HashMap<Uuid, Container> {
        &self.containers
    }

    /// A container by ID: one of the player's, or the open station's.
    pub fn container(&self, id: Uuid) -> Option<&Container> {
        self.station_container
            .as_ref()
            .filter(|c| c.id == id)
            .or_else(|| self.containers.get(&id))
    }

    fn container_of_type(&self, kind: &str) -> Option<&Container> {
        self.containers.values().find(|c| c.container_type == kind)
    }

    pub fn inventory(&self) -> Option<&Container> {
        self.container_of_type(container_type::PLAYER_INVENTORY)
    }

    pub fn crafting_input(&self) -> Option<&Container> {
        self.container_of_type(container_type::CRAFTING_INPUT)
    }

    pub fn crafting_output(&self) -> Option<&Container> {
        self.container_of_type(container_type::CRAFTING_OUTPUT)
    }

    /// Your market box (E3.1), once fetched: what you bought, and what came
    /// back from a cancel or an expiry.
    pub fn market_box(&self) -> Option<&Container> {
        self.container_of_type(container_type::MARKET_BOX)
    }

    /// Your impound (#141), if you have one: what a foreclosure, or a lot
    /// laid under your workbench, gave back when your inventory was full.
    pub fn impound(&self) -> Option<&Container> {
        self.container_of_type(container_type::IMPOUND)
    }

    pub fn open_station(&self) -> Option<&OpenStation> {
        self.open_station.as_ref()
    }

    /// The open station as the world lists it, with its quality.
    pub fn open_station_info(&self) -> Option<&StationInfo> {
        self.station(self.open_station.as_ref()?.station_id)
    }

    /// The open station's container, once fetched.
    pub fn station_container(&self) -> Option<&Container> {
        self.station_container.as_ref()
    }

    /// The static game data.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Starts from a catalog the game had already: the disk cache, or the
    /// one from before a sign-out. `/catalog` replaces it with the snapshot.
    pub fn set_catalog(&mut self, catalog: Catalog) {
        self.catalog = catalog;
    }

    pub fn recipes(&self) -> &[RecipeInfo] {
        &self.recipes
    }

    pub fn recipe(&self, id: &str) -> Option<&RecipeInfo> {
        self.recipes.iter().find(|r| r.id == id)
    }

    /// The Strand balance, once fetched.
    pub fn strands(&self) -> Option<i64> {
        self.strands
    }

    /// Whether this client plays the avatar, as the server last said (E0.8).
    /// `None` until it says.
    pub fn presence(&self) -> Option<Presence> {
        self.presence
    }

    /// Whether this client drives the avatar: it holds it, or the server
    /// hasn't said otherwise (an older server always lets it).
    pub fn plays_here(&self) -> bool {
        self.presence.is_none_or(|p| p.yours)
    }

    /// The fees the server charges now (D6.1): quote the exchange, the
    /// market's listing fee and a stall's rent from these.
    pub fn fees(&self) -> Fees {
        self.fees
    }

    /// Whether the server said the token is an admin's (D5.3), for the
    /// admin overlay. The admin routes check the token again whatever this says.
    pub fn is_admin(&self) -> bool {
        self.is_admin
    }

    /// Your account's subscription, as of now. `None` until the server says.
    pub fn subscription(&self) -> Option<Subscription> {
        self.subscription.map(|s| s.as_of(self.utc_now()))
    }

    /// The server's clock now if it's known, else this machine's: when a
    /// subscription ends, or a stall's rent, is the server's to say.
    pub fn utc_now(&self) -> DateTime<Utc> {
        let ms = self.server_time_ms().unwrap_or_else(local_unix_ms);
        DateTime::from_timestamp_millis(ms).unwrap_or_default()
    }

    /// Whether you're a subscriber now: the header's Patron chip (E4.1).
    /// The market: the books you watch and your orders (E3.3).
    pub fn market(&self) -> &MarketStore {
        &self.market
    }

    /// To watch an item, or take the messages the market wants sent.
    pub fn market_mut(&mut self) -> &mut MarketStore {
        &mut self.market
    }

    /// The bazaar: its directory, the shops seen at their stalls, and yours
    /// (E3.10).
    pub fn bazaar(&self) -> &BazaarStore {
        &self.bazaar
    }

    pub fn is_patron(&self) -> bool {
        self.subscription
            .is_some_and(|s| s.is_active_at(self.utc_now()))
    }

    /// Your own plots, as last listed.
    pub fn plots(&self) -> &[PlotInfo] {
        &self.plots
    }

    /// Everyone's claimed lots, by `TerrainPlot::id` (P8.6b's houses).
    pub fn claimed_lots(&self) -> &HashMap<String, ClaimedPlot> {
        &self.claimed
    }

    /// Goes up whenever [`claimed_lots`](Self::claimed_lots) changes.
    pub fn claimed_version(&self) -> u64 {
        self.claimed_version
    }

    /// One of your plots.
    pub fn plot(&self, id: Uuid) -> Option<&PlotInfo> {
        self.plots.iter().find(|p| p.id == id)
    }

    /// Your plot that the server point `(x, y)` is on, the way the server
    /// tests a station's placement.
    pub fn my_plot_at(&self, x: f64, y: f64) -> Option<&PlotInfo> {
        self.plots.iter().find(|p| p.contains(x, y))
    }

    /// The plot the plot panel shows, kept fresh by whatever answer about it
    /// comes in next (a claim, a refetch, the plot list).
    pub fn picked_plot(&self) -> Option<&PlotInfo> {
        self.picked_plot.as_ref()
    }

    /// Closes the plot panel.
    pub fn unpick_plot(&mut self) {
        self.picked_plot = None;
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn is_me(&self, player_id: Uuid) -> bool {
        self.me == Some(player_id)
    }

    fn holds_container(&self, id: Uuid) -> bool {
        self.containers.contains_key(&id)
            || self.open_station.as_ref().map(|s| s.container_id) == Some(id)
    }

    fn inventory_changed(&self, player_id: Uuid, container_id: Uuid, out: &mut Vec<StateEvent>) {
        if self.is_me(player_id) || self.holds_container(container_id) {
            out.push(StateEvent::InventoryChanged { container_id });
        }
    }

    /// `positions` lists every player near us (C0.4: our cell of the interest
    /// grid and the eight round it, `rules::INTEREST_CELL`), so it replaces the
    /// set we can see. One missing has gone out of sight, which isn't leaving:
    /// a trade with them carries on until a `player_left`.
    fn positions(&mut self, players: &[PlayerPosition], out: &mut Vec<StateEvent>) {
        let gone: Vec<Uuid> = self
            .players
            .keys()
            .filter(|id| !players.iter().any(|p| p.id == **id))
            .copied()
            .collect();
        for id in gone {
            self.lose_sight(id, out);
        }
        for player in players {
            if self.players.insert(player.id, player.clone()).is_none() {
                out.push(StateEvent::PlayerJoined(player.id));
            }
        }
        out.push(StateEvent::PositionsUpdated);
        self.learn_zone(out);
    }

    /// They left the world: out of sight, and their trades with us are over
    fn remove_player(&mut self, id: Uuid, out: &mut Vec<StateEvent>) {
        if self.players.remove(&id).is_some() {
            out.push(StateEvent::PlayerLeft(id));
        }
        self.trade_player_left(id, out);
        self.forget_activity(id, out);
    }

    /// They walked out of our interest cells (C0.4), or we walked away from them
    fn lose_sight(&mut self, id: Uuid, out: &mut Vec<StateEvent>) {
        if self.players.remove(&id).is_some() {
            out.push(StateEvent::PlayerLeft(id));
        }
        self.forget_activity(id, out);
    }

    fn forget_activity(&mut self, id: Uuid, out: &mut Vec<StateEvent>) {
        if self.activities.remove(&id).is_some() && !self.is_me(id) {
            out.push(StateEvent::ActivityChanged { player_id: id });
        }
    }

    /// The server only sends `zone_changed` on a crossing, so the first zone
    /// comes from the first known position.
    fn learn_zone(&mut self, out: &mut Vec<StateEvent>) {
        if self.zone.is_none() {
            if let Some(me) = self.local_player() {
                let zone = Zone::from_position(me.x, me.y);
                self.set_zone(zone, out);
            }
        }
    }

    fn set_zone(&mut self, to: Zone, out: &mut Vec<StateEvent>) {
        let from = self.zone.replace(to);
        if from != Some(to) {
            out.push(StateEvent::ZoneChanged { from, to });
        }
    }

    fn start(
        &mut self,
        player_id: Uuid,
        activity: Activity,
        local: StateEvent,
        out: &mut Vec<StateEvent>,
    ) {
        self.activities.insert(player_id, activity);
        out.push(if self.is_me(player_id) {
            local
        } else {
            StateEvent::ActivityChanged { player_id }
        });
    }

    /// Completion, cancellation and failure. The local player's always reports,
    /// even with no start on record (it began before a reconnect).
    fn finish(&mut self, player_id: Uuid, local: StateEvent, out: &mut Vec<StateEvent>) {
        let had = self.activities.remove(&player_id).is_some();
        if self.is_me(player_id) {
            out.push(local);
        } else if had {
            out.push(StateEvent::ActivityChanged { player_id });
        }
    }

    fn set_node_state(&mut self, id: Uuid, state: &str, out: &mut Vec<StateEvent>) {
        if let Some(node) = self.nodes.get_mut(&id) {
            if node.state != state {
                node.state = state.to_string();
                out.push(StateEvent::NodeChanged(id));
            }
        }
    }

    fn merge_nodes(
        &mut self,
        nodes: &[ResourceNodeInfo],
        replace: bool,
        out: &mut Vec<StateEvent>,
    ) {
        let diff = merge(
            &mut self.nodes,
            nodes.iter().map(|n| (n.id, n.clone())),
            replace,
        );
        out.extend(diff.removed.into_iter().map(StateEvent::NodeRemoved));
        out.extend(diff.added.into_iter().map(StateEvent::NodeAdded));
        out.extend(diff.changed.into_iter().map(StateEvent::NodeChanged));
    }

    fn merge_stations(&mut self, stations: &[StationInfo], out: &mut Vec<StateEvent>) {
        let replace = std::mem::take(&mut self.stations_stale);
        let diff = merge(
            &mut self.stations,
            stations.iter().map(|s| (s.id, s.clone())),
            replace,
        );
        out.extend(diff.removed.into_iter().map(StateEvent::StationRemoved));
        out.extend(diff.added.into_iter().map(StateEvent::StationAdded));
        out.extend(diff.changed.into_iter().map(StateEvent::StationChanged));
    }

    fn upsert_station(&mut self, station: StationInfo, out: &mut Vec<StateEvent>) {
        let diff = merge(&mut self.stations, [(station.id, station)], false);
        out.extend(diff.added.into_iter().map(StateEvent::StationAdded));
        out.extend(diff.changed.into_iter().map(StateEvent::StationChanged));
    }

    fn close_station(&mut self, out: &mut Vec<StateEvent>) {
        self.open_station = None;
        self.station_container = None;
        out.push(StateEvent::StationClosed);
    }

    fn set_strands(&mut self, balance: i64, out: &mut Vec<StateEvent>) {
        if self.strands.replace(balance) != Some(balance) {
            out.push(StateEvent::CurrencyChanged { balance });
        }
    }

    /// Takes in one plot's latest: the picked plot if it's that one, and
    /// your list, which it joins if it's yours and leaves if it isn't.
    /// Returns whether anything changed.
    fn learn_plot(&mut self, plot: &PlotInfo) -> bool {
        let mut changed = false;
        if let Some(picked) = &mut self.picked_plot {
            if picked.id == plot.id && picked != plot {
                *picked = plot.clone();
                changed = true;
            }
        }
        if self.me.is_some() && plot.owner_id == self.me {
            changed |= self.upsert_plot(plot);
        } else if let Some(i) = self.plots.iter().position(|p| p.id == plot.id) {
            self.plots.remove(i);
            changed = true;
        }
        changed
    }

    /// Your plot on the lot `terrain_id` owes `tax_owed`: in your list, and
    /// the picked plot if it's that one. Returns whether anything changed.
    fn owe_tax(&mut self, terrain_id: &str, tax_owed: i64, forecloses_at: i64) -> bool {
        let mut changed = false;
        let picked = self.picked_plot.iter_mut();
        for plot in self.plots.iter_mut().chain(picked) {
            if plot.terrain_id.as_deref() == Some(terrain_id)
                && (plot.tax_owed, plot.forecloses_at) != (tax_owed, Some(forecloses_at))
            {
                plot.tax_owed = tax_owed;
                plot.forecloses_at = Some(forecloses_at);
                changed = true;
            }
        }
        changed
    }

    /// The lot `terrain_id` lost its owner: it's no longer one of yours, and
    /// the picked plot, if it's that one, is unowned. Returns whether anything changed.
    fn release_plot(&mut self, terrain_id: &str) -> bool {
        let is_it = |p: &PlotInfo| p.terrain_id.as_deref() == Some(terrain_id);
        let mut changed = false;
        if let Some(i) = self.plots.iter().position(is_it) {
            self.plots.remove(i);
            changed = true;
        }
        if let Some(picked) = self
            .picked_plot
            .as_mut()
            .filter(|p| is_it(p) && p.owner_id.is_some())
        {
            picked.owner_id = None;
            picked.owner_name = None;
            picked.claimed_at = None;
            changed = true;
        }
        changed
    }

    /// Returns whether anything changed.
    fn upsert_plot(&mut self, plot: &PlotInfo) -> bool {
        match self.plots.iter_mut().find(|p| p.id == plot.id) {
            Some(existing) if existing == plot => false,
            Some(existing) => {
                *existing = plot.clone();
                true
            }
            None => {
                self.plots.push(plot.clone());
                true
            }
        }
    }
}

#[derive(Debug, Default)]
struct Diff {
    added: Vec<Uuid>,
    changed: Vec<Uuid>,
    removed: Vec<Uuid>,
}

/// Upserts `incoming` into `map`, reporting what actually changed. With
/// `replace`, entries missing from `incoming` are removed too.
fn merge<T: PartialEq>(
    map: &mut HashMap<Uuid, T>,
    incoming: impl IntoIterator<Item = (Uuid, T)>,
    replace: bool,
) -> Diff {
    let mut diff = Diff::default();
    let mut seen = Vec::new();
    for (id, value) in incoming {
        seen.push(id);
        match map.get_mut(&id) {
            Some(existing) if *existing == value => {}
            Some(existing) => {
                *existing = value;
                diff.changed.push(id);
            }
            None => {
                map.insert(id, value);
                diff.added.push(id);
            }
        }
    }
    if replace {
        map.retain(|id, _| {
            let keep = seen.contains(id);
            if !keep {
                diff.removed.push(*id);
            }
            keep
        });
    }
    diff
}

#[cfg(test)]
mod tests;

/// This machine's clock, in Unix ms.
fn local_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
