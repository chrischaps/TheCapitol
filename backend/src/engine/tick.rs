use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tracing::Instrument;
use uuid::Uuid;

use super::presence::{self, Holder, PresenceEvent, Sockets};

use super::commands::Command;
use super::craft_completion::{finish_craft, FinishedCraft};
use super::crafting::{cancel_crafting, process_crafting, start_crafting, CraftingOperation};
use super::events::GameEvent;
use super::extraction::{cancel_extraction, process_extractions, start_extraction, PendingItem};
use super::ledger;
use super::load::{Loop, Phase};
use super::units;
use super::tools::{self, HeldTool, Wear};
use super::movement::process_movement;
use super::stranded::{WayOut, WildReach};
use super::regeneration::process_regeneration;
use super::trading::{TradeManager, TRADE_RANGE};
use super::tutorial::{Did, Judged};
use super::wages::{self, ShiftWork};
use crate::models::{ActionState, ContainerType, ItemType, PlayerPosition, PlayerState, Recipe, ResourceNodeState, ResourceType, StationState, StationType, TerrainConfig, WorldGenParams};
use crate::state::{AppState, DB_POOL_SIZE};

use capitol_protocol::ledger::UnitFlow;
use capitol_protocol::rest::{CatalogResponse, Subscription, SubscriptionStatus};
use capitol_protocol::fmath::StableMath;
use capitol_protocol::rules::ground::{Ground, TOO_STEEP};
use capitol_protocol::rules::quality;
use capitol_protocol::rules::stacking::Units;
use capitol_protocol::rules::TICK_MS as TICK_RATE_MS;
use capitol_protocol::tutorial::WALK_DISTANCE;

/// Runs `$body` as one phase of the tick (D0.1): in the span `$span`, and
/// timed as [`Phase`]`::$phase`. For the phases that await, the loop uses
/// [`Phase::timed`] with `.instrument` instead: an entered span can't be held
/// across an `.await`.
macro_rules! phase {
    ($phase:ident, $span:literal, $body:expr) => {{
        // Timed first, so the span's enter and exit count in the phase
        let _timed = Phase::$phase.timed();
        let _span = tracing::info_span!($span).entered();
        $body
    }};
}

/// Ticks between sweeps for unanswered trade requests (once a second)
const TRADE_EXPIRY_TICKS: u64 = 10;

/// A steering player held against something hears `movement_blocked` at
/// most once in this many ticks (2 s); they keep the steer meanwhile (T0.2)
const STEER_BLOCKED_TICKS: u64 = capitol_protocol::rules::steer::BLOCKED_TICKS;

/// Why a trade ended when one side's socket fell behind (C0.3)
pub const TRADE_LAGGED: &str = "Connection fell behind";

pub struct GameState {
    pub players: HashMap<Uuid, PlayerState>,
    /// player_id -> their open sockets, and which holds the avatar (E0.8). Only the holder's
    /// close removes the player (B8, #59)
    pub connections: HashMap<Uuid, Sockets>,
    next_connection_id: u64,
    /// API key id -> what fires to close each socket open on it, with why (E0.9)
    key_sockets: HashMap<Uuid, Vec<oneshot::Sender<String>>>,
    pub resource_nodes: HashMap<Uuid, ResourceNodeState>,
    pub resource_types: HashMap<String, ResourceType>,
    pub recipes: HashMap<String, Recipe>,
    pub item_types: HashMap<String, ItemType>,
    pub container_types: HashMap<String, ContainerType>,
    /// `/catalog`'s answer for the static tables above, built by
    /// [`GameState::refresh_catalog`] once they're loaded
    pub catalog: Arc<CatalogResponse>,
    pub crafting_operations: HashMap<Uuid, CraftingOperation>,
    /// Each guild member's tag (S4.4), by player, for the names over their
    /// heads: loaded at startup, and read again after each guild change
    /// (`guilds::retag`)
    pub guild_tags: HashMap<Uuid, String>,
    /// The last tick's positions, so the next can say who changed cell (#275)
    last_positions: Option<Arc<super::interest::Positions>>,
    pub command_queue: Vec<Command>,
    pub event_sender: broadcast::Sender<GameEvent>,
    pub tick_count: u64,
    pub pending_items: Vec<PendingItem>,
    /// The tool each gather under way started with, by player (E1.5)
    pub gather_tools: HashMap<Uuid, HeldTool>,
    /// The wage shift each gather under way is on, by player (Commons D7)
    gather_wages: HashMap<Uuid, ShiftWork>,
    /// The tick each steering player was last told they're blocked, so a
    /// walk held into a bank hears it once every [`STEER_BLOCKED_TICKS`]
    steer_blocked_at: HashMap<Uuid, u64>,
    /// Where each walk under way began, by player, so its end can be judged
    /// for the road in's first step (#439)
    walks_from: HashMap<Uuid, (f64, f64)>,
    /// Players whose walk ended this tick at least `WALK_DISTANCE` from where
    /// it began, for the writer to judge
    walked: Vec<Uuid>,
    /// Crafts that ran their time this tick, for the tick loop to write
    pub finished_crafts: Vec<FinishedCraft>,
    // M4: Stations
    pub stations: HashMap<Uuid, StationState>,
    pub station_types: HashMap<String, StationType>,
    // M5: Trading
    pub trade_manager: TradeManager,
    // Terrain configuration (moats, canals, bridges, roads)
    pub terrain: TerrainConfig,
    // `terrain.version()`, kept beside it so `/terrain` doesn't rehash on every request
    pub terrain_version: String,
    /// Every lot's address, lot id to "3 Sheepwash Lane" (`TerrainConfig::addresses`,
    /// #483), kept beside the terrain so a plot or a card doesn't work them out again
    pub addresses: Arc<HashMap<String, String>>,
    /// The terrain's land and wild water, indexed for walking on (W4)
    pub ground: Arc<Ground>,
    /// The ground of the wilds a walker can reach from the rural edge, for
    /// lifting a stranded one out (#522): worked out in the background
    pub wild_reach: Arc<WildReach>,
    /// Where each player was last stopped "Too steep" and found able to walk
    /// out, so a steer held against a cliff is looked at once (#522)
    walks_out_from: HashMap<Uuid, (f64, f64)>,
    // World generation parameters (for admin tweaking)
    pub world_gen_params: WorldGenParams,
    /// The stations working on their own (E4.3), run by their own task
    pub works: Arc<super::automation_tick::Works>,
    /// The economy's live knobs (D6.1): a gather's yield and a node's regrowth here
    pub tunables: super::tunables::Live,
    // Performance tracking
    pub last_tps: f64,
    pub uptime_seconds: f64,
}

impl GameState {
    /// How many events the broadcast keeps for a socket that's behind (C0.3). A
    /// socket further behind than this skips ahead and resyncs. With 200 citizens
    /// at work, gathers' and crafts' progress alone is up to 400 events a second,
    /// so 1024 was under 3 s of slack for a client on a slow link; 4096 is about
    /// 10 s. Each event is held once, whatever the number of sockets, and the
    /// biggest, `PlayerPositions`, is 10 of them a second.
    pub const EVENT_CAPACITY: usize = 4096;

    pub fn with_params(params: WorldGenParams) -> Self {
        let terrain = TerrainConfig::from_params(&params);
        Self::with_terrain(params, terrain)
    }

    /// A game on terrain already generated from `params`
    pub(crate) fn with_terrain(params: WorldGenParams, terrain: TerrainConfig) -> Self {
        let (event_sender, _) = broadcast::channel(Self::EVENT_CAPACITY);
        let terrain_version = terrain.version();
        let ground = Arc::new(terrain.ground());
        let addresses = Arc::new(terrain.addresses());
        Self {
            players: HashMap::new(),
            connections: HashMap::new(),
            next_connection_id: 0,
            key_sockets: HashMap::new(),
            resource_nodes: HashMap::new(),
            resource_types: HashMap::new(),
            recipes: HashMap::new(),
            item_types: HashMap::new(),
            container_types: HashMap::new(),
            catalog: Arc::new(CatalogResponse {
                version: String::new(),
                item_types: Vec::new(),
                station_types: Vec::new(),
                resource_types: Vec::new(),
                container_types: Vec::new(),
                recipes: Vec::new(),
                habitats: Vec::new(),
            }),
            crafting_operations: HashMap::new(),
            guild_tags: HashMap::new(),
            last_positions: None,
            command_queue: Vec::new(),
            event_sender,
            tick_count: 0,
            pending_items: Vec::new(),
            gather_tools: HashMap::new(),
            gather_wages: HashMap::new(),
            steer_blocked_at: HashMap::new(),
            walks_from: HashMap::new(),
            walked: Vec::new(),
            finished_crafts: Vec::new(),
            stations: HashMap::new(),
            station_types: HashMap::new(),
            trade_manager: TradeManager::new(),
            terrain,
            terrain_version,
            addresses,
            ground,
            wild_reach: Arc::default(),
            walks_out_from: HashMap::new(),
            world_gen_params: params,
            works: Arc::default(),
            tunables: Default::default(),
            last_tps: 0.0,
            uptime_seconds: 0.0,
        }
    }

    /// Rebuild `/catalog`'s answer from the static tables. Call it after
    /// loading or changing them.
    pub fn refresh_catalog(&mut self) {
        self.catalog = Arc::new(super::catalog::build(self));
    }

    /// Replace the terrain, and its version and addresses with it
    pub fn set_terrain(&mut self, terrain: TerrainConfig) {
        self.terrain_version = terrain.version();
        self.addresses = Arc::new(terrain.addresses());
        self.ground = Arc::new(terrain.ground());
        self.wild_reach = Arc::default();
        self.terrain = terrain;
    }

    pub fn take_pending_items(&mut self) -> Vec<PendingItem> {
        std::mem::take(&mut self.pending_items)
    }

    pub fn take_finished_crafts(&mut self) -> Vec<FinishedCraft> {
        std::mem::take(&mut self.finished_crafts)
    }

    pub fn take_walked(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.walked)
    }

    pub fn queue_command(&mut self, command: Command) {
        self.command_queue.push(command);
    }

    pub fn add_player(&mut self, player: PlayerState) {
        let position = PlayerPosition {
            id: player.id,
            name: player.name.clone(),
            x: player.x,
            y: player.y,
            citizen: player.citizen,
            tag: self.guild_tags.get(&player.id).cloned(),
        };
        self.players.insert(player.id, player);
        let _ = self.event_sender.send(GameEvent::PlayerJoined(position));
    }

    /// A socket opened for player `player_id`, holding nothing yet (E0.8). `may_hold` is false
    /// for a `read` key. Returns its id and what it hears about its presence.
    pub fn open_socket(
        &mut self,
        player_id: Uuid,
        holder: Holder,
        may_hold: bool,
    ) -> (u64, mpsc::UnboundedReceiver<PresenceEvent>) {
        self.next_connection_id += 1;
        let id = self.next_connection_id;
        let rx = self.connections.entry(player_id).or_default().open(id, holder, may_hold);
        (id, rx)
    }

    /// Hand `player`'s avatar to socket `socket`, from whichever held it. If nobody did, the
    /// avatar comes into the world as `player` (just read from the database); otherwise the one
    /// already in the world carries on. A refused claim tells nobody anything.
    pub fn claim_presence(&mut self, player: PlayerState, socket: u64) -> Result<(), presence::Refused> {
        let player_id = player.id;
        let sockets = self.connections.get_mut(&player_id).ok_or(presence::Refused::Closed)?;
        sockets.claim(socket)?;
        tracing::info!("Connection {} holds player {}", socket, player_id);
        match self.players.get_mut(&player_id) {
            // The new holder didn't send the old one's steer (T0.2)
            Some(held) => held.steer = None,
            None => self.add_player(player),
        }
        Ok(())
    }

    /// Tell socket `socket` who holds the player's avatar, when it doesn't itself.
    pub fn tell_watcher(&self, player_id: Uuid, socket: u64) {
        if let Some(sockets) = self.connections.get(&player_id) {
            sockets.tell_watcher(socket);
        }
    }

    /// Whether socket `socket` holds the player's avatar.
    pub fn holds_presence(&self, player_id: Uuid, socket: u64) -> bool {
        self.connections.get(&player_id).is_some_and(|s| s.holds(socket))
    }

    /// Who holds the account's avatar, if anyone does: for REST calls that drive it.
    pub fn presence_of_account(&self, account_id: Uuid) -> Option<&Holder> {
        self.connections
            .iter()
            .find(|(player_id, _)| self.players.get(player_id).is_some_and(|p| p.account_id == account_id))
            .and_then(|(_, sockets)| sockets.holder())
    }

    /// The account's sessions `sids` ended over REST (a logout, a revoked refresh family, or
    /// sessions past the cap): their sockets say `auth_error` and hang up. Their own close then
    /// gives up the avatar, if one held it. Returns how many were told.
    pub fn end_sessions(&mut self, player_id: Uuid, sids: &[String]) -> usize {
        let told = self.connections.get(&player_id).map_or(0, |s| s.end_sessions(sids));
        if told > 0 {
            tracing::info!("Sessions ended for player {}: closing {} connection(s)", player_id, told);
        }
        told
    }

    /// A socket opened on API key `key_id`. The receiver fires, with the
    /// reason, when the key is revoked or its owner's subscription lapses.
    pub fn watch_key(&mut self, key_id: Uuid) -> oneshot::Receiver<String> {
        let (tx, rx) = oneshot::channel();
        let sockets = self.key_sockets.entry(key_id).or_default();
        // Forget sockets that have closed since.
        sockets.retain(|tx| !tx.is_closed());
        sockets.push(tx);
        rx
    }

    /// API key `key_id` was revoked: every socket open on it hangs up. Returns how many.
    pub fn revoke_key(&mut self, key_id: Uuid) -> usize {
        self.hang_up_key(key_id, "API key revoked")
    }

    /// Every socket open on API key `key_id` hangs up, telling its bot `why`.
    /// Returns how many.
    pub fn hang_up_key(&mut self, key_id: Uuid, why: &str) -> usize {
        self.key_sockets
            .remove(&key_id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tx| tx.send(why.to_string()).ok())
            .count()
    }

    /// End connection `connection_id`. The player leaves the world only if it held their
    /// avatar; a watcher (a client playing elsewhere, a half-open socket a reconnect took over
    /// from, a bot) leaves the holder's player alone. Returns whether the player was removed.
    pub fn disconnect_player(&mut self, player_id: Uuid, connection_id: u64) -> bool {
        let Some(sockets) = self.connections.get_mut(&player_id) else {
            return false;
        };
        let held = sockets.close(connection_id);
        if sockets.is_empty() {
            self.connections.remove(&player_id);
        }
        if held {
            self.remove_player(player_id);
        }
        held
    }

    pub fn remove_player(&mut self, player_id: Uuid) {
        // Cancel any active trade when player disconnects
        if let Some((trade_id, other)) = self.trade_manager.get_player_trade(player_id)
            .and_then(|t| Some((t.id, t.other_player(player_id)?)))
        {
            if self.trade_manager.cancel_trade(player_id).is_ok() {
                // The other player needs to know the trade was cancelled
                let _ = self.event_sender.send(GameEvent::TradeCancelled {
                    trade_id,
                    reason: "Player disconnected".to_string(),
                });
                tracing::info!("Trade cancelled due to player {} disconnect, notified {}", player_id, other);
            }
        }
        self.gather_tools.remove(&player_id);
        self.gather_wages.remove(&player_id);
        self.steer_blocked_at.remove(&player_id);
        self.walks_out_from.remove(&player_id);
        self.walks_from.remove(&player_id);
        // Remove any pending trade requests
        self.trade_manager.pending_requests.remove(&player_id);
        self.trade_manager.pending_requests.retain(|_, (initiator, _)| *initiator != player_id);

        self.players.remove(&player_id);
        let _ = self.event_sender.send(GameEvent::PlayerLeft { player_id });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GameEvent> {
        self.event_sender.subscribe()
    }

    /// Socket `connection_id` fell behind the event stream and lost events
    /// (C0.3). If it drives the avatar, the player's trade and any trade
    /// request to or from them end, and both sides hear: the client can't
    /// know the offer it would be accepting is the one on the table.
    pub fn socket_lagged(&mut self, player_id: Uuid, connection_id: u64) {
        if !self.holds_presence(player_id, connection_id) {
            return;
        }
        if let Some(trade_id) = self.trade_manager.get_player_trade(player_id).map(|t| t.id) {
            if self.trade_manager.cancel_trade(player_id).is_ok() {
                let _ = self.event_sender.send(GameEvent::TradeCancelled {
                    trade_id,
                    reason: TRADE_LAGGED.to_string(),
                });
            }
        }
        let mut ended = Vec::new();
        self.trade_manager.pending_requests.retain(|target, (initiator, _)| {
            let ours = *target == player_id || *initiator == player_id;
            if ours {
                ended.push((*initiator, *target));
            }
            !ours
        });
        for (initiator, target) in ended {
            let _ = self.event_sender.send(GameEvent::TradeRequestExpired { initiator, target });
        }
    }

    /// Every player's position, by interest cell, as `positions` sends them each tick (C0.4)
    pub fn positions(&self) -> super::interest::Positions {
        super::interest::Positions::of(self.players.values(), &self.recipes, &self.guild_tags)
    }

    /// Tell `player_id`, and only them, why their trade `command` (its `ClientMessage` tag) was refused
    fn trade_failed(&self, player_id: Uuid, command: &'static str, reason: impl Into<String>) {
        let reason = reason.into();
        tracing::debug!("{} from {} refused: {}", command, player_id, reason);
        let _ = self.event_sender.send(GameEvent::TradeFailed { player_id, command, reason });
    }

    /// Tell `player_id`, and only them, why their extraction didn't start
    fn extraction_failed(&self, player_id: Uuid, reason: impl Into<String>) {
        let reason = reason.into();
        tracing::debug!("Extraction by {} refused: {}", player_id, reason);
        let _ = self.event_sender.send(GameEvent::ExtractionFailed { player_id, reason });
    }

    fn process_commands(&mut self) {
        let commands: Vec<Command> = std::mem::take(&mut self.command_queue);
        super::load::commands_taken(commands.len());

        for command in commands {
            match command {
                Command::Move { player_id, dest_x, dest_y } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        start_moving(player, &mut self.resource_nodes, &mut self.crafting_operations, &self.event_sender);
                        player.dest_x = Some(dest_x);
                        player.dest_y = Some(dest_y);
                        player.steer = None;
                    }
                }
                Command::Steer { player_id, heading, throttle, at } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        if throttle > 0 {
                            start_moving(player, &mut self.resource_nodes, &mut self.crafting_operations, &self.event_sender);
                        }
                        super::movement::steer(player, heading, throttle, at);
                    }
                }
                Command::StartExtraction { player_id, node_id, tool, wage } => {
                    let Some(node) = self.resource_nodes.get_mut(&node_id) else {
                        self.extraction_failed(player_id, "That resource is gone");
                        continue;
                    };
                    let Some(resource_type) = self.resource_types.get(&node.resource_type) else {
                        let reason = format!("Unknown resource type '{}'", node.resource_type);
                        self.extraction_failed(player_id, reason);
                        continue;
                    };
                    let Some(player) = self.players.get_mut(&player_id) else {
                        // Nobody to tell: they left between queueing and this tick
                        continue;
                    };
                    match start_extraction(player, node, resource_type, tool, &mut self.gather_tools, &self.event_sender) {
                        Ok(()) => match wage {
                            Some(work) => self.gather_wages.insert(player_id, work),
                            None => self.gather_wages.remove(&player_id),
                        },
                        Err(reason) => {
                            self.extraction_failed(player_id, reason);
                            None
                        }
                    };
                }
                Command::CancelExtraction { player_id } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        cancel_extraction(player, &mut self.resource_nodes, &self.event_sender);
                    }
                }
                // M3: Crafting commands
                Command::StartCrafting { player_id, recipe_id, inputs, tool } => {
                    // The WS handler fetched `inputs` by the ids the client sent,
                    // checking they're the player's (`fetch_player_items`)
                    if let Some(recipe) = self.recipes.get(&recipe_id).cloned() {
                        // Check if recipe requires a station (hand crafting should not use station recipes)
                        if let Some(required_station) = &recipe.required_station_type {
                            let _ = self.event_sender.send(GameEvent::CraftingFailed {
                                player_id,
                                reason: format!("Recipe requires {} station", required_station),
                            });
                            continue;
                        }

                        if let Some(player) = self.players.get_mut(&player_id) {
                            match start_crafting(
                                player,
                                &recipe,
                                &inputs,
                                quality::NO_STATION,
                                tool,
                                &mut self.crafting_operations,
                                &self.event_sender,
                            ) {
                                Ok(_) => {}
                                Err(e) => {
                                    let _ = self.event_sender.send(GameEvent::CraftingFailed {
                                        player_id,
                                        reason: e,
                                    });
                                }
                            }
                        }
                    } else {
                        let _ = self.event_sender.send(GameEvent::CraftingFailed {
                            player_id,
                            reason: "Recipe not found".to_string(),
                        });
                    }
                }
                Command::CancelCrafting { player_id } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        cancel_crafting(player, &mut self.crafting_operations, &self.event_sender);
                    }
                }
                Command::CraftAtStation { player_id, station_id, recipe_id, inputs, tool, wage } => {
                    // Validate range and start crafting. Who may use the station was
                    // checked against the plot's permissions before this was queued
                    // (`container_access::check_station_access`, #80).
                    let station_valid = self.stations.get(&station_id)
                        .map(|s| {
                            let player = self.players.get(&player_id);
                            player.is_some_and(|p| {
                                super::stations::is_player_near_station_type(p, s, &self.station_types)
                            })
                        })
                        .unwrap_or(false);

                    if !station_valid {
                        let _ = self.event_sender.send(GameEvent::CraftingFailed {
                            player_id,
                            reason: "Cannot access station".to_string(),
                        });
                        continue;
                    }

                    // Check recipe requires this station type
                    if let (Some(recipe), Some(station)) = (
                        self.recipes.get(&recipe_id).cloned(),
                        self.stations.get(&station_id)
                    ) {
                        if let Some(required) = &recipe.required_station_type {
                            if required != &station.station_type {
                                let _ = self.event_sender.send(GameEvent::CraftingFailed {
                                    player_id,
                                    reason: format!("Recipe requires {}", required),
                                });
                                continue;
                            }
                        }

                        // Start crafting (uses same flow as hand crafting)
                        let station_quality = station.quality;
                        if let Some(player) = self.players.get_mut(&player_id) {
                            match start_crafting(
                                player,
                                &recipe,
                                &inputs,
                                station_quality,
                                tool,
                                &mut self.crafting_operations,
                                &self.event_sender,
                            ) {
                                Ok(operation_id) => {
                                    // Its output goes into the station's box (#489), and on
                                    // a wage shift it's paid for when it's written
                                    if let Some(op) = self.crafting_operations.get_mut(&operation_id) {
                                        op.station_id = Some(station_id);
                                        op.wage = wage;
                                    }
                                }
                                Err(e) => {
                                    let _ = self.event_sender.send(GameEvent::CraftingFailed {
                                        player_id,
                                        reason: e,
                                    });
                                }
                            }
                        }
                    }
                }
                // M5: Trading commands
                Command::InitiateTrade { player_id, target_player_id } => {
                    if target_player_id == player_id {
                        self.trade_failed(player_id, "trade_initiate", "You can't trade with yourself");
                        continue;
                    }

                    // Validate both players exist and are in range
                    let my_pos = match self.players.get(&player_id) {
                        Some(me) => (me.x, me.y),
                        None => continue, // the requester left before the tick; nobody to tell
                    };
                    let (target_pos, target_name) = match self.players.get(&target_player_id) {
                        Some(target) => ((target.x, target.y), target.name.clone()),
                        None => {
                            self.trade_failed(player_id, "trade_initiate", "That player is offline");
                            continue;
                        }
                    };

                    let dx = my_pos.0 - target_pos.0;
                    let dy = my_pos.1 - target_pos.1;
                    let distance = (dx * dx + dy * dy).sqrt();

                    if distance > TRADE_RANGE {
                        self.trade_failed(
                            player_id,
                            "trade_initiate",
                            format!(
                                "{} is too far away to trade ({:.0} units; trading needs {:.0})",
                                target_name, distance, TRADE_RANGE
                            ),
                        );
                        continue;
                    }

                    // Get initiator name for the request event
                    let (initiator_name, initiator_citizen) = self.players.get(&player_id)
                        .map(|p| (p.name.clone(), p.citizen))
                        .unwrap_or_default();

                    match self.trade_manager.request_trade(player_id, target_player_id) {
                        Ok(replaced) => {
                            // The request this one replaced is gone: its asker stops waiting.
                            // The target's client ignores it, since its prompt now names us
                            if let Some(initiator) = replaced {
                                let _ = self.event_sender.send(GameEvent::TradeRequestExpired {
                                    initiator,
                                    target: target_player_id,
                                });
                            }
                            // Check if trade started immediately (both requested each other)
                            if let Some(trade) = self.trade_manager.get_player_trade(player_id) {
                                let _ = self.event_sender.send(GameEvent::TradeStarted {
                                    trade_id: trade.id,
                                    player_a: trade.player_a,
                                    player_b: trade.player_b,
                                });
                            } else {
                                // Send trade request notification to target
                                let _ = self.event_sender.send(GameEvent::TradeRequested {
                                    from_player: player_id,
                                    from_player_name: initiator_name,
                                    from_citizen: initiator_citizen,
                                    to_player: target_player_id,
                                });
                            }
                        }
                        Err(e) => self.trade_failed(player_id, "trade_initiate", e),
                    }
                }
                Command::AcceptTradeRequest { player_id } => {
                    match self.trade_manager.accept_request(player_id) {
                        Ok(trade) => {
                            let _ = self.event_sender.send(GameEvent::TradeStarted {
                                trade_id: trade.id,
                                player_a: trade.player_a,
                                player_b: trade.player_b,
                            });
                        }
                        Err(e) => self.trade_failed(player_id, "trade_accept_request", e),
                    }
                }
                Command::DeclineTradeRequest { player_id } => {
                    match self.trade_manager.decline_request(player_id) {
                        Ok(initiator) => {
                            let _ = self.event_sender.send(GameEvent::TradeRequestDeclined {
                                by_player: player_id,
                                initiator,
                            });
                        }
                        Err(e) => self.trade_failed(player_id, "trade_decline_request", e),
                    }
                }
                Command::UpdateTradeOffer { player_id, items, strands } => {
                    let offer = items.iter().map(|item| (item.item_id, item.units.clone())).collect();
                    if let Err(e) = self.trade_manager.update_offer(player_id, offer, strands) {
                        self.trade_failed(player_id, "trade_offer", e);
                        continue;
                    }

                    if let Some(trade) = self.trade_manager.get_player_trade(player_id) {
                        let _ = self.event_sender.send(GameEvent::TradeOfferUpdated {
                            trade_id: trade.id,
                            player_id,
                            items,
                            strands,
                        });
                    }
                }
                Command::CancelTrade { player_id } => {
                    if let Some(trade) = self.trade_manager.get_player_trade(player_id) {
                        let trade_id = trade.id;
                        if self.trade_manager.cancel_trade(player_id).is_ok() {
                            let _ = self.event_sender.send(GameEvent::TradeCancelled {
                                trade_id,
                                reason: "Cancelled by player".to_string(),
                            });
                        }
                    } else {
                        self.trade_failed(player_id, "trade_cancel", "Not in a trade");
                    }
                }
            }
        }
    }

    fn process_movement(&mut self) {
        use crate::models::Zone;

        for player in self.players.values_mut() {
            let steering = player.steer.is_some();
            if steering || player.dest_x.is_some() {
                self.walks_from.entry(player.id).or_insert((player.x, player.y));
            }
            let dest = (player.dest_x, player.dest_y);
            let mut result = process_movement(player, &self.ground);
            // Stopped "Too steep" where there's no walking out: in a pocket
            // walled by crests, it's lifted to the nearest reached ground,
            // and its walk goes on from there (#522)
            if result.blocked.as_ref().is_some_and(|b| b.reason == TOO_STEEP)
                && self.walks_out_from.get(&player.id) != Some(&(player.x, player.y))
            {
                match self.wild_reach.get().map(|reach| reach.way_out((player.x, player.y))) {
                    None => self.wild_reach.start(&self.terrain),
                    Some(WayOut::Walks) => {
                        self.walks_out_from.insert(player.id, (player.x, player.y));
                    }
                    Some(WayOut::Lift((x, y))) => {
                        tracing::info!("{} was stranded at ({:.0}, {:.0}), lifted out to ({x:.0}, {y:.0})", player.name, player.x, player.y);
                        (player.x, player.y) = (x, y);
                        (player.dest_x, player.dest_y) = dest;
                        result.blocked = None;
                    }
                    Some(WayOut::Lost) => {
                        tracing::warn!("{} is stranded at ({:.0}, {:.0}), with no reached ground near", player.name, player.x, player.y);
                        self.walks_out_from.insert(player.id, (player.x, player.y));
                    }
                }
            }
            // A walk that's over, here or by whatever cleared it since, is
            // judged by how far it ended from where it began
            if player.steer.is_none() && player.dest_x.is_none() {
                if let Some((x, y)) = self.walks_from.remove(&player.id) {
                    if (player.x - x).stable_hypot(player.y - y) >= WALK_DISTANCE {
                        self.walked.push(player.id);
                    }
                }
            }

            // Broadcast zone transition if player changed zones
            if let Some(transition) = result.zone_transition {
                let zone = Zone::from_str(&transition.to_zone);
                let zone_name = zone.map(|z| z.display_name()).unwrap_or(&transition.to_zone);

                let _ = self.event_sender.send(GameEvent::ZoneChanged {
                    player_id: player.id,
                    from_zone: transition.from_zone,
                    to_zone: transition.to_zone.clone(),
                    zone_name: zone_name.to_string(),
                });
            }

            // Broadcast movement blocked if player was stopped by water. A
            // steer held into it hears once every STEER_BLOCKED_TICKS.
            if let Some(blocked) = result.blocked {
                if steering {
                    let said = self.steer_blocked_at.get(&player.id);
                    if said.is_some_and(|&said| self.tick_count < said + STEER_BLOCKED_TICKS) {
                        continue;
                    }
                    self.steer_blocked_at.insert(player.id, self.tick_count);
                }
                let _ = self.event_sender.send(GameEvent::MovementBlocked {
                    player_id: player.id,
                    reason: blocked.reason,
                    water_id: blocked.water_id,
                    stopped_x: blocked.stopped_x,
                    stopped_y: blocked.stopped_y,
                });
            }
        }
    }

    fn broadcast_positions(&mut self) {
        let positions = Arc::new(self.positions().after(self.last_positions.as_deref()));
        if !positions.is_empty() {
            let _ = self.event_sender.send(GameEvent::PlayerPositions(Arc::clone(&positions)));
        }
        self.last_positions = Some(positions);
    }

    /// The world's tick, phase by phase (D0.1): each in a `tick.<phase>`
    /// span, and timed into [`load`](super::load).
    fn tick(&mut self) {
        self.tick_count += 1;
        phase!(Commands, "tick.commands", self.process_commands());
        phase!(Movement, "tick.movement", self.process_movement());
        phase!(Extraction, "tick.extraction", self.process_extractions());
        phase!(Crafting, "tick.crafting", self.process_crafting());
        phase!(Regeneration, "tick.regeneration", self.process_regeneration());
        if self.tick_count.is_multiple_of(TRADE_EXPIRY_TICKS) {
            phase!(Trades, "tick.trades", self.expire_trade_requests(Instant::now()));
        }
        phase!(Broadcast, "tick.broadcast", self.broadcast_positions());
    }

    /// Drop the trade requests nobody answered in time, and tell both players of each
    fn expire_trade_requests(&mut self, now: Instant) {
        for (initiator, target) in self.trade_manager.expire_requests(now) {
            tracing::debug!("Trade request from {} to {} expired", initiator, target);
            let _ = self.event_sender.send(GameEvent::TradeRequestExpired { initiator, target });
        }
    }

    fn process_extractions(&mut self) {
        let new_items = process_extractions(
            &mut self.players,
            &mut self.resource_nodes,
            &self.resource_types,
            &self.item_types,
            &mut self.gather_tools,
            self.tick_count,
            &self.event_sender,
            &self.tunables.get(),
        );
        self.pending_items.extend(new_items.into_iter().map(|mut item| {
            item.wage = self.gather_wages.remove(&item.player_id);
            item
        }));
    }

    fn process_regeneration(&mut self) {
        process_regeneration(
            &mut self.resource_nodes,
            self.tick_count,
            &self.event_sender,
        );
    }

    fn process_crafting(&mut self) {
        let finished = process_crafting(
            &mut self.players,
            &self.recipes,
            &mut self.crafting_operations,
            &self.item_types,
            &self.event_sender,
        );
        self.finished_crafts.extend(finished);
    }
}

/// A player sets off, by a `move` or a steer that walks (T0.2): a gather or
/// a craft under way stops.
fn start_moving(
    player: &mut PlayerState,
    nodes: &mut HashMap<Uuid, ResourceNodeState>,
    crafting_operations: &mut HashMap<Uuid, CraftingOperation>,
    event_sender: &broadcast::Sender<GameEvent>,
) {
    match &player.action_state {
        ActionState::Extracting { .. } => cancel_extraction(player, nodes, event_sender),
        ActionState::Crafting { .. } => cancel_crafting(player, crafting_operations, event_sender),
        ActionState::Idle => {}
    }
}

/// Every player's position is saved each this many ticks: every 10 s at 1x
const POSITION_SAVE_TICKS: u64 = 100;

/// Ticks a game hour (V3.2): how often the weather's sweep runs
const WEATHER_TICKS: u64 = (capitol_protocol::rules::DAY_LENGTH_SECS as u64 / 24) * 10;

/// What one tick finished, for the writer to save (#304)
struct Finished {
    gathers: Vec<PendingItem>,
    crafts: Vec<FinishedCraft>,
    /// Walks that ended far enough to judge for the road in (#439)
    walks: Vec<Uuid>,
    /// In the writer's backlog until it's written (#407)
    handed: super::load::Handed,
}

impl Finished {
    fn new(gathers: Vec<PendingItem>, crafts: Vec<FinishedCraft>, walks: Vec<Uuid>) -> Finished {
        let handed = super::load::Handed::new(gathers.len() + crafts.len());
        Finished { gathers, crafts, walks, handed }
    }

    /// A tick's saves dealt to the writer's [`WRITERS`] lanes, each by the box
    /// it writes into ([`lane`]), in the tick's order within each. Lanes with
    /// nothing to save are left out.
    fn dealt(gathers: Vec<PendingItem>, crafts: Vec<FinishedCraft>, walks: Vec<Uuid>) -> Vec<(usize, Finished)> {
        let mut lanes: Vec<(Vec<PendingItem>, Vec<FinishedCraft>, Vec<Uuid>)> =
            (0..WRITERS).map(|_| Default::default()).collect();
        for item in gathers {
            // On a wage shift it goes into the employer's market box
            let into = item.wage.as_ref().map_or(item.player_id, |w| w.employer_id);
            lanes[lane(into)].0.push(item);
        }
        for craft in crafts {
            // At a station, its own or on a wage shift, its output goes into
            // the station's box
            let shift_station = craft.wage.as_ref().and_then(|w| match &w.work {
                capitol_protocol::rules::wages::Work::Craft { station_id, .. } => Some(*station_id),
                capitol_protocol::rules::wages::Work::Gather { .. } => None,
            });
            lanes[lane(craft.station_id.or(shift_station).unwrap_or(craft.player_id))].1.push(craft);
        }
        for player in walks {
            lanes[lane(player)].2.push(player);
        }
        lanes
            .into_iter()
            .enumerate()
            .filter(|(_, (g, c, w))| !(g.is_empty() && c.is_empty() && w.is_empty()))
            .map(|(i, (g, c, w))| (i, Finished::new(g, c, w)))
            .collect()
    }
}

/// Lanes the writer saves in side by side (#407). A gather was about 11
/// round trips (the pool's ping, `BEGIN`, seven statements and `COMMIT`)
/// and a commit's flush: about 5 ms on an idle database and 10 in a loaded
/// town, nearly all of it waiting. One lane was about 40% busy at 200
/// citizens. On the local Postgres, four lanes wrote 2.4 times as many
/// gathers a second as one, about 550, and eight no more than four: past
/// that the database's commits are the ceiling. Since #524 a gather is two
/// statements between its `BEGIN` and `COMMIT` (the tool's wear, and the
/// units placed with the Gazette's tally), and in the same A/B on a busier
/// machine one took 3.5 ms where it took 6, and four lanes wrote about 590
/// a second where they wrote 385 (`the_writers_lanes_gathers_a_second`).
/// Since #622 the pool doesn't ping a connection that was just busy, so a
/// gather is one round trip shorter: taken in turns on a busy machine, 3.3
/// to 4.2 ms where it took 3.6 to 4.7 (`a_gather_waits_a_round_trip_less`).
const WRITERS: usize = 4;

/// The writer's lane for saves into `key`'s box: a player's inventory, an
/// employer's market box, a station's box. Saves into one box are written
/// in order, one after the other, as when there was one writer. Saves into
/// different boxes run side by side; where they still meet (a station craft
/// taking its inputs from the crafter's inventory), the database's locks
/// order them, and a gather that loses a race is tried again
/// ([`persist_gather`]).
fn lane(key: Uuid) -> usize {
    (key.as_u128() % WRITERS as u128) as usize
}

pub async fn run_tick_loop(game_state: Arc<RwLock<GameState>>, app_state: AppState) {
    // The economy sim's time multiplier (E2.7); 1, real time, everywhere else
    let scale = app_state.config.sim_tick_scale;
    let period = Duration::from_secs_f64(TICK_RATE_MS as f64 / 1000.0 / scale);
    if scale == 1.0 {
        tracing::info!("Starting tick engine at {}ms interval", TICK_RATE_MS);
    } else {
        tracing::warn!(
            "Starting tick engine at {:?} interval: SIM_TICK_SCALE runs the world {}x real time",
            period,
            scale
        );
    }

    // The tick awaits no database (#304): what it finishes goes to the
    // writer, which saves it a tick's worth at a time, in lanes (#407)
    let event_sender = game_state.read().await.event_sender.clone();
    let to_writers: Vec<_> = (0..WRITERS)
        .map(|_| {
            let (to_writer, from_tick) = mpsc::unbounded_channel();
            tokio::spawn(run_writer(app_state.db.clone(), event_sender.clone(), from_tick));
            to_writer
        })
        .collect();
    // The position save in flight, so a slow one is never overtaken by the next
    let mut saving_positions: Option<tokio::task::JoinHandle<()>> = None;

    let mut interval = tokio::time::interval(period);
    let mut last_log = Instant::now();
    let mut tick_count = 0u64;
    let loop_start = Instant::now();

    loop {
        interval.tick().await;
        // The tick's work, from here to the loop's end (C1.7)
        let began = Instant::now();
        // In Tracy (D3.3), each tick is a frame and its work a "tick" frame,
        // so the frame graph shows the work against the 100 ms
        #[cfg(feature = "profile")]
        let _frame = tracy_client::Client::running().map(|tracy| {
            tracy.frame_mark();
            tracy.non_continuous_frame(tracy_client::frame_name!("tick"))
        });

        let gathers: Vec<PendingItem>;
        let crafts: Vec<FinishedCraft>;
        let walks: Vec<Uuid>;
        let positions: Option<Vec<SavedPosition>>;
        let game_tick_count;
        {
            let mut game = {
                let _timed = Phase::Lock.timed();
                game_state.write().instrument(tracing::info_span!("tick.lock")).await
            };
            game.tick();
            gathers = game.take_pending_items();
            crafts = game.take_finished_crafts();
            walks = game.take_walked();
            game_tick_count = game.tick_count;
            // Read under the tick's own lock, and written once it's let go.
            // The world's count, never reset, so this comes round every 100
            // ticks; the loop's own count restarts at each tick-rate log.
            positions = game_tick_count.is_multiple_of(POSITION_SAVE_TICKS).then(|| {
                let _timed = Phase::Positions.timed();
                saved_positions(&game)
            });
        }

        // Gathers are written wearing their tools, and crafts with their inputs
        // re-checked and consumed in one transaction (B10, #61). The writer
        // tells the gatherers and crafters only once each is written.
        if !gathers.is_empty() || !crafts.is_empty() || !walks.is_empty() {
            let _timed = Phase::Writes.timed();
            for (lane, finished) in Finished::dealt(gathers, crafts, walks) {
                if to_writers[lane].send(finished).is_err() {
                    tracing::error!("Writer {lane} has stopped: this tick's gathers and crafts in it weren't saved");
                }
            }
        }

        tick_count += 1;

        // Log tick rate every 10 seconds
        if last_log.elapsed() >= Duration::from_secs(10) {
            let tps = tick_count as f64 / last_log.elapsed().as_secs_f64();
            tracing::debug!("Tick rate: {:.1} ticks/sec", tps);
            {
                let _timed = Phase::Lock.timed();
                let mut game = game_state.write().instrument(tracing::info_span!("tick.lock")).await;
                game.last_tps = tps;
                game.uptime_seconds = loop_start.elapsed().as_secs_f64();
            }
            last_log = Instant::now();
            tick_count = 0;
        }

        // Every player's position, in one statement, off the tick
        if let Some(positions) = positions {
            if saving_positions.as_ref().is_some_and(|s| !s.is_finished()) {
                tracing::warn!("The last position save is still running; skipping this one");
            } else {
                let db = app_state.db.clone();
                saving_positions = Some(tokio::spawn(
                    async move {
                        let _timed = Loop::Positions.timed();
                        if let Err(e) = persist_positions(&db, &positions).await {
                            tracing::error!("Failed to persist player positions: {:?}", e);
                        }
                    }
                    .instrument(tracing::info_span!("loop.positions")),
                ));
            }
        }

        // M7: Process property taxes hourly (36000 ticks = 1 hour at 10 ticks/sec),
        // in a task of its own
        if game_tick_count > 0 && game_tick_count % 36000 == 0 {
            let _timed = Phase::Tax.timed();
            let (game_state, db) = (game_state.clone(), app_state.db.clone());
            // The live rate (D6.1), as it is when the hour strikes
            let rate = app_state.tunables.get().property_tax_rate;
            tokio::spawn(
                async move {
                    let _timed = Loop::Tax.timed();
                    tracing::info!("Processing hourly property taxes at tick {}", game_tick_count);
                    let report = super::property_tax::process_property_taxes(&db, rate).await;
                    let events = {
                        let mut game = game_state.write().await;
                        let game = &mut *game;
                        super::property_tax::announce(report, &mut game.stations, &game.event_sender);
                        game.event_sender.clone()
                    };
                    // S4.2: the guilds' dues, from their treasuries, the same
                    // hour. A guild dissolved for them gives up its hall (S4.3)
                    for mut told in super::guilds::collect_dues(&db, chrono::Utc::now()).await {
                        if !told.halls.is_empty() {
                            let mut game = game_state.write().await;
                            let game = &mut *game;
                            told.release_halls(&mut game.stations, &game.event_sender);
                        }
                        told.send(&db, &events).await;
                    }
                }
                .instrument(tracing::info_span!("loop.tax")),
            );
        }

        // V3.2: the weather, once a game hour (5 real minutes), half way
        // between the tax's, in a task of its own: outdoor stations wear on
        // to now, and those on public land at 0 fall down
        if game_tick_count > 0 && game_tick_count % WEATHER_TICKS == WEATHER_TICKS / 2 {
            let (game_state, db) = (game_state.clone(), app_state.db.clone());
            let tunables = app_state.tunables.get();
            let scale = app_state.config.sim_tick_scale;
            tokio::spawn(
                async move {
                    match super::weather::sweep(&db, &tunables, scale, chrono::Utc::now()).await {
                        Ok(report) => {
                            let mut game = game_state.write().await;
                            let game = &mut *game;
                            super::weather::announce(report, &mut game.stations, &game.event_sender);
                        }
                        Err(e) => tracing::error!("The weather's sweep failed: {e}"),
                    }
                }
                .instrument(tracing::info_span!("loop.weather")),
            );
        }

        // E4.1: subscriptions that ran out, hourly with the tax but half an
        // hour apart from it, in a task of its own. The Subscriber extractor
        // and a key's sign-in refuse them on time anyway; this marks them
        // lapsed, tells their players and hangs up their bots (#217). The keys
        // aren't revoked: they sign in again once the player renews.
        if game_tick_count > 0 && game_tick_count % 36000 == 18000 {
            let _timed = Phase::Subscriptions.timed();
            let (game_state, db) = (game_state.clone(), app_state.db.clone());
            tokio::spawn(
                async move {
                    let _timed = Loop::Subscriptions.timed();
                    let lapses = crate::subscriptions::lapse_expired(&db).await;
                    let mut game = game_state.write().await;
                    for l in lapses {
                        let lapsed = Subscription { status: SubscriptionStatus::Lapsed, until: Some(l.until) };
                        for key in &l.key_ids {
                            game.hang_up_key(*key, &crate::api_keys::unsubscribed(&lapsed));
                        }
                        let _ = game.event_sender.send(GameEvent::SubscriptionLapsed {
                            player_id: l.player_id,
                            until: l.until,
                            notice_id: l.notice_id,
                        });
                    }
                    // S4.1: a guild whose leader has been lapsed four weeks
                    // passes the lead on
                    let events = game.event_sender.clone();
                    drop(game);
                    for told in super::guilds::pass_lapsed_leads(&db, chrono::Utc::now()).await {
                        told.send(&db, &events).await;
                    }
                }
                .instrument(tracing::info_span!("loop.subscriptions")),
            );
        }

        super::load::tick_took(began.elapsed());
        if cfg!(debug_assertions) {
            super::slow_work::warn_awaits();
        }
    }
}

/// Saves what the ticks finished in one of the writer's lanes, a tick at a
/// time and in the order they finished it, until the tick loop stops (#304).
/// Two gathers into one inventory are in one lane ([`lane`]), so they're
/// written one after the other, as they were when the tick wrote them
/// itself. The lanes run side by side (#407), each its own
/// `loops.writer` cycles, so the loop's busy share can pass 100%.
///
/// Once each is written, it's judged for the road in (#439), as is each walk
/// that ended. A player's step is asked about once a run ([`Judged`]).
async fn run_writer(db: sqlx::PgPool, events: broadcast::Sender<GameEvent>, mut from_tick: mpsc::UnboundedReceiver<Finished>) {
    let mut judged = Judged::default();
    while let Some(Finished { gathers, crafts, walks, handed }) = from_tick.recv().await {
        let _timed = Loop::Writer.timed();
        async {
            for item in gathers {
                let (player, item_type) = (item.player_id, item.item_type.clone());
                if persist_gather(&db, item, &events).await {
                    judged.saw(&db, &events, player, Did::Gathered(&item_type)).await;
                }
            }
            if !crafts.is_empty() {
                finish_crafts(&db, crafts, &events, &mut judged).await;
            }
            for player in walks {
                judged.saw(&db, &events, player, Did::Walked).await;
            }
        }
        .instrument(tracing::info_span!("loop.writer"))
        .await;
        drop(handed);
    }
}

/// Finish crafts: inputs re-checked, consumed and the output written in one
/// transaction (B10, #61). Only then is the crafter told, with one
/// `inventory_updated` per container that changed, and the craft judged for
/// the road in.
async fn finish_crafts(
    db: &sqlx::PgPool,
    finished: Vec<FinishedCraft>,
    event_sender: &broadcast::Sender<GameEvent>,
    judged: &mut Judged,
) {
    for craft in finished {
        match finish_craft(db, &craft).await {
            Ok(done) => {
                for output in &craft.outputs {
                    let _ = event_sender.send(GameEvent::CraftingCompleted {
                        player_id: craft.player_id,
                        operation_id: craft.operation_id,
                        item_type: output.item_type.clone(),
                        item_name: output.item_name.clone(),
                        quantity: output.quantity,
                        quality: output.quality,
                        tool: done.tool.clone(),
                    });
                }
                if let Some(broken) = &done.broken {
                    tools::announce_broken(event_sender, broken);
                }
                if let Some((station_id, tool)) = done.station_tool {
                    super::station_tools::announce(event_sender, station_id, tool);
                }
                if let Some(news) = done.wage {
                    wages::announce(event_sender, news);
                }
                for container_id in done.changed {
                    let _ = event_sender.send(GameEvent::InventoryUpdated {
                        player_id: craft.player_id,
                        container_id,
                        slots: Vec::new(),
                    });
                }
                judged.saw(db, event_sender, craft.player_id, Did::Crafted(&craft.recipe_id)).await;
            }
            Err(reason) => {
                let _ = event_sender.send(GameEvent::CraftingFailed {
                    player_id: craft.player_id,
                    reason,
                });
            }
        }
    }
}

/// A player's position as the tick left it, to be saved
#[derive(Debug, Clone, PartialEq)]
struct SavedPosition {
    id: Uuid,
    x: f64,
    y: f64,
    dest_x: Option<f64>,
    dest_y: Option<f64>,
}

fn saved_positions(game: &GameState) -> Vec<SavedPosition> {
    game.players
        .values()
        .map(|p| SavedPosition { id: p.id, x: p.x, y: p.y, dest_x: p.dest_x, dest_y: p.dest_y })
        .collect()
}

/// Every player's position in one statement, not a round trip each (#304)
async fn persist_positions(db: &sqlx::PgPool, positions: &[SavedPosition]) -> Result<(), sqlx::Error> {
    if positions.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = positions.iter().map(|p| p.id).collect();
    let xs: Vec<f64> = positions.iter().map(|p| p.x).collect();
    let ys: Vec<f64> = positions.iter().map(|p| p.y).collect();
    let dest_xs: Vec<Option<f64>> = positions.iter().map(|p| p.dest_x).collect();
    let dest_ys: Vec<Option<f64>> = positions.iter().map(|p| p.dest_y).collect();
    sqlx::query(
        "UPDATE players AS p
         SET position_x = u.x, position_y = u.y, destination_x = u.dest_x, destination_y = u.dest_y
         FROM UNNEST($1::uuid[], $2::float8[], $3::float8[], $4::float8[], $5::float8[]) AS u(id, x, y, dest_x, dest_y)
         WHERE p.id = u.id",
    )
    .bind(ids)
    .bind(xs)
    .bind(ys)
    .bind(dest_xs)
    .bind(dest_ys)
    .execute(db)
    .await?;
    Ok(())
}

/// Write one finished gather, then tell the gatherer: `extraction_completed` with
/// what the tool has left, `inventory_updated`, and `tool_broke` if that was its
/// last use. A failed pull (quantity 0, D5) writes only the tool's wear.
///
/// The tool wears in the same transaction that writes the item. If it left the
/// player's inventory while they gathered, the gather is what bare hands would
/// have given, or nothing for a resource that needs the tool.
///
/// A gather on a wage shift (Commons D7) goes into the employer's market box,
/// marked as the gatherer's make, and is paid for in the same transaction
/// ([`place_for_wages`]). If the shift ended while it ran, nothing is
/// written. If the employer's box has no room, it's the gatherer's, unpaid,
/// and the shift ends (`box_full`).
///
/// Returns whether something was gathered and written: not a failed pull,
/// nor a gather that wrote nothing.
///
/// Its box can still be written by someone else at the same moment: a route
/// moving things in the inventory, or a station craft in another of the
/// writer's lanes (#407) taking its inputs from it. A gather that loses the
/// race for a free slot, or a lock, is rolled back and tried again
/// ([`raced`]), up to [`GATHER_TRIES`] times, where before it was lost.
///
/// The pool hands out a connection that's been busy without pinging it
/// first (#622), so after the database restarts one can be dead. A gather
/// that finds its connection broken before it commits is tried again on
/// another ([`broken`]): the dead one is closed as it goes back, so with one
/// try more than the pool holds, one is sure to be new.
pub(crate) async fn persist_gather(db: &sqlx::PgPool, item: PendingItem, events: &broadcast::Sender<GameEvent>) -> bool {
    let mut tries = 1;
    loop {
        match write_gather(db, item.clone(), events).await {
            Ok(gathered) => return gathered,
            Err(Unsaved { doing, cause }) if tries < GATHER_TRIES && raced(&cause) => {
                tracing::debug!("A gather by {} lost a race {doing} and is tried again: {cause}", item.player_id);
                tries += 1;
            }
            // A broken COMMIT may have been written, so it isn't tried again
            Err(Unsaved { doing, cause }) if tries <= DB_POOL_SIZE as usize && doing != COMMITTING && broken(&cause) => {
                tracing::info!("A gather by {} found its connection broken {doing} and is tried again: {cause}", item.player_id);
                tries += 1;
            }
            Err(Unsaved { doing, cause }) => {
                tracing::error!("Failed {doing} for a gather by {}: {:?}", item.player_id, cause);
                return false;
            }
        }
    }
}

/// Times a gather is tried that loses a race with another of the writer's
/// lanes (#407)
const GATHER_TRIES: usize = 3;

/// A gather's transaction failed: rolled back, nothing told
struct Unsaved {
    /// What it was doing, for the log
    doing: &'static str,
    cause: sqlx::Error,
}

/// Whether `e` lost a race with another transaction, so trying again from
/// the start can succeed: a unique slot taken first (23505), a deadlock
/// (40P01) or a serialization failure (40001)
fn raced(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db) => db.is_unique_violation() || matches!(db.code().as_deref(), Some("40P01" | "40001")),
        _ => false,
    }
}

/// What a gather's [`Unsaved`] says it was doing when its `COMMIT` failed
const COMMITTING: &str = "committing";

/// Whether `e` says the connection is gone, so nothing it was doing in its
/// transaction was kept: its socket failed (an I/O error), the server ended
/// it (57P01 or 57P02, as a restart or `pg_terminate_backend` does), or it was
/// lost (class 08). Another connection can succeed. (#622)
fn broken(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Io(_) => true,
        sqlx::Error::Database(db) => {
            db.code().is_some_and(|code| code.starts_with("08") || matches!(code.as_ref(), "57P01" | "57P02"))
        }
        _ => false,
    }
}

/// One try at [`persist_gather`]: all of it, or none of it.
async fn write_gather(db: &sqlx::PgPool, mut item: PendingItem, events: &broadcast::Sender<GameEvent>) -> Result<bool, Unsaved> {
    let unsaved = |doing| move |cause| Unsaved { doing, cause };
    let mut tx = db.begin().await.map_err(unsaved("starting"))?;
    let (mut used, mut broken) = (None, None);
    // On a wage shift: what the pay has to tell people, and whose box it went in
    let (mut wage_news, mut box_owner) = (None, None);
    // What it made and wore out, counted once it's written (C1.7)
    let mut flows = ledger::Flows::new();
    if let Some(gather) = item.tool.take() {
        match tools::wear_held(&mut tx, gather.held.item_id, item.player_id).await.map_err(unsaved("wearing its tool"))? {
            Some(wear) => {
                flows.wore(&gather.held.item_type, &wear);
                used = Some(gather.held.after(&wear));
                if let Wear::Broke(b) = wear {
                    broken = Some(b);
                }
            }
            None => match gather.by_hand {
                Some((quantity, quality)) => (item.quantity, item.quality) = (quantity, quality),
                None => {
                    let _ = events.send(GameEvent::ExtractionFailed {
                        player_id: item.player_id,
                        reason: format!(
                            "Your {} left your inventory before the gather finished",
                            gather.held.item_type
                        ),
                    });
                    return Ok(false);
                }
            },
        }
    }
    // The inventory that changed: the one the units went into, or after a
    // failed pull (D5) the one whose tool wore. A failed pull by hand changed none.
    let container_id = if item.quantity == 0 && used.is_none() {
        None
    } else if item.quantity == 0 {
        inventory_of(&mut tx, item.player_id)
            .await
            .map_err(unsaved("finding the inventory"))?
            .map(|(container_id, _)| container_id)
    } else {
        let paid = match &item.wage {
            Some(work) => match place_for_wages(&mut tx, &item, work).await {
                Ok(Ok(placed)) => placed,
                Ok(Err(why)) => {
                    let _ = events.send(GameEvent::ExtractionFailed { player_id: item.player_id, reason: why.to_string() });
                    return Ok(false);
                }
                Err(wages::WageError::Db(cause)) => return Err(Unsaved { doing: "paying wages", cause }),
                Err(e) => {
                    tracing::error!("Failed to pay wages on a gather by {}: {}", item.player_id, e.message());
                    return Ok(false);
                }
            },
            None => None,
        };
        let container_id = match paid {
            Some((container_id, news)) => {
                wage_news = Some(news);
                box_owner = item.wage.as_ref().map(|w| w.employer_id);
                super::gazette::tally(&mut tx, item.player_id, &item.item_type, item.quantity)
                    .await
                    .map_err(unsaved("tallying it for the Gazette"))?;
                container_id
            }
            // Tallied for the Gazette as it's placed
            None => match place_gathered(&mut tx, &item).await.map_err(unsaved("placing it"))? {
                Some(container_id) => container_id,
                None => return Ok(false),
            },
        };
        // The employer's box had no room: the gatherer keeps it, unpaid, and the shift ends
        if let (Some(work), None) = (&item.wage, &wage_news) {
            match wages::end_shift(&mut tx, work, capitol_protocol::rules::wages::ShiftEnd::BoxFull, chrono::Utc::now()).await {
                Ok(news) => wage_news = Some(news),
                Err(wages::WageError::Db(cause)) => return Err(Unsaved { doing: "ending a shift on a full box", cause }),
                Err(e) => {
                    tracing::error!("Failed to end {}'s shift on a full box: {}", item.player_id, e.message());
                    return Ok(false);
                }
            }
        }
        flows.add(&item.item_type, UnitFlow::Gathered, item.quantity as i64);
        Some(container_id)
    };
    ledger::commit(tx, flows).await.map_err(unsaved(COMMITTING))?;

    let _ = events.send(GameEvent::ExtractionCompleted {
        player_id: item.player_id,
        node_id: item.node_id,
        item_type: item.item_type.clone(),
        item_name: item.item_name.clone(),
        quantity: item.quantity,
        quality: item.quality,
        tool: used,
    });
    if let Some(container_id) = container_id {
        let _ = events.send(GameEvent::InventoryUpdated {
            player_id: box_owner.unwrap_or(item.player_id),
            container_id,
            slots: Vec::new(), // Client will refresh all slots
        });
    }
    if let Some(broken) = &broken {
        tools::announce_broken(events, broken);
    }
    if let Some(news) = wage_news {
        wages::announce(events, news);
    }
    Ok(item.quantity > 0)
}

/// Put a gather made on the wage shift `work` into the employer's market box,
/// as theirs, marked as the gatherer's make, and pay for it (Commons D7).
/// `Ok(Ok(None))`: their box has no room, and nothing was written. `Ok(Err)`:
/// the shift is over; the caller writes nothing.
async fn place_for_wages(
    conn: &mut sqlx::PgConnection,
    item: &PendingItem,
    work: &ShiftWork,
) -> Result<Result<Option<(Uuid, wages::News)>, &'static str>, wages::WageError> {
    let into = super::market::market_box(conn, work.employer_id).await?;
    let gathered = units::Marked::by(Some(item.player_id), &Units::uniform(item.quantity, item.quality));
    if !units::join_first_stack(conn, into, &item.item_type, &gathered).await? {
        let Some(slot) = super::market::free_slot(conn, into).await? else {
            return Ok(Ok(None));
        };
        let stack = units::NewStack {
            item_type: &item.item_type,
            owner_id: Some(work.employer_id),
            container_id: Some(into),
            slot_index: Some(slot),
            durability: None,
        };
        units::insert_stack(conn, stack, &gathered).await?;
    }
    let news = wages::pay(conn, work, &item.item_type, &gathered.units(), chrono::Utc::now()).await?;
    Ok(news.map(|news| Some((into, news))))
}

/// `player_id`'s inventory and how many slots it has, if they have one.
async fn inventory_of(conn: &mut sqlx::PgConnection, player_id: Uuid) -> Result<Option<(Uuid, i32)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.owner_id = $1 AND c.container_type = 'player_inventory'
         LIMIT 1"
    )
    .bind(player_id)
    .fetch_optional(&mut *conn)
    .await
}

/// Put a gathered item in its gatherer's inventory: onto the first stack of its
/// type, whatever its grade (as `units::join_first_stack`: every unit keeps its
/// quality), or into the first free slot. Its units carry the gatherer's mark
/// (#231), and it's tallied for the Gazette (as `gazette::tally`). Returns the
/// container, or `None` if there's no room (the item is lost, as before).
///
/// All of it is one statement (#524): the inventory, the stack locked and
/// joined, or a free slot and a new stack, and the tally. They were five to
/// seven, each a round trip. A stack too full to take them (`i32::MAX`
/// units) is passed over for a free slot, as `Units::merge` refused it.
async fn place_gathered(conn: &mut sqlx::PgConnection, item: &PendingItem) -> Result<Option<Uuid>, sqlx::Error> {
    // The statement's sub-statements all see one snapshot, so each one that
    // depends on another reads what it wrote from its RETURNING
    let (container_id, joined, slot_index): (Option<Uuid>, Option<Uuid>, Option<i32>) = sqlx::query_as(
        "WITH inv AS (
             SELECT c.id, ct.slot_count FROM containers c
             JOIN container_types ct ON c.container_type = ct.id
             WHERE c.owner_id = $1 AND c.container_type = 'player_inventory'
             LIMIT 1
         ),
         stack AS (
             SELECT i.id, i.quantity FROM items i JOIN inv ON i.container_id = inv.id
             WHERE i.item_type = $2
             ORDER BY i.slot_index NULLS LAST, i.id
             LIMIT 1
             FOR UPDATE OF i
         ),
         joined AS (
             INSERT INTO item_units (item_id, quality, maker_id, count)
             SELECT id, $3::SMALLINT, $1, $4 FROM stack WHERE stack.quantity::BIGINT + $4 <= 2147483647
             ON CONFLICT ON CONSTRAINT item_units_key DO UPDATE SET count = item_units.count + EXCLUDED.count
             RETURNING item_id
         ),
         slot AS (
             SELECT s FROM inv, generate_series(0, inv.slot_count - 1) AS s
             WHERE NOT EXISTS (SELECT 1 FROM joined)
               AND NOT EXISTS (SELECT 1 FROM items WHERE container_id = inv.id AND slot_index = s)
             ORDER BY s
             LIMIT 1
         ),
         -- At quantity 0 the new stack seeds no units of its own: the
         -- summaries' trigger sets them from new_units, at the statement's end
         new_stack AS (
             INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             SELECT $5, $2, $3, 0, $1, inv.id, slot.s FROM inv, slot
             RETURNING id
         ),
         new_units AS (
             INSERT INTO item_units (item_id, quality, maker_id, count)
             SELECT id, $3::SMALLINT, $1, $4 FROM new_stack
         ),
         tally AS (
             INSERT INTO production_tally (day, player_id, item_type, quantity)
             SELECT $6, $1, $2, $4::BIGINT FROM (SELECT item_id FROM joined UNION ALL SELECT id FROM new_stack) placed
             ON CONFLICT (day, player_id, item_type) DO UPDATE SET quantity = production_tally.quantity + EXCLUDED.quantity
         )
         SELECT (SELECT id FROM inv), (SELECT item_id FROM joined), (SELECT s FROM slot)",
    )
    .bind(item.player_id)
    .bind(&item.item_type)
    .bind(item.quality)
    .bind(item.quantity)
    .bind(Uuid::new_v4())
    .bind(capitol_protocol::rules::day_number(chrono::Utc::now().timestamp_millis()))
    .fetch_one(&mut *conn)
    .await?;

    let Some(container_id) = container_id else {
        tracing::error!("No player_inventory container for player {}", item.player_id);
        return Ok(None);
    };
    match (joined, slot_index) {
        (Some(_), _) => tracing::info!(
            "Merged {} x{} (quality {}) into existing stack for player {}",
            item.item_type, item.quantity, item.quality, item.player_id
        ),
        (None, Some(slot_index)) => tracing::info!(
            "Created {} x{} (quality {}) in slot {} for player {}",
            item.item_type, item.quantity, item.quality, slot_index, item.player_id
        ),
        (None, None) => {
            tracing::warn!("Inventory full for player {}, item lost!", item.player_id);
            return Ok(None);
        }
    }
    Ok(Some(container_id))
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::extraction::EXTRACTION_RANGE;
    use crate::models::{ActionState, NodeState};
    use capitol_protocol::rules::{CENTER_X, CENTER_Y};
    use capitol_protocol::ws::PresenceHolder;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

    fn player(name: &str, x: f64, y: f64) -> PlayerState {
        PlayerState {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            name: name.into(),
            x,
            y,
            dest_x: None,
            dest_y: None,
            speed: 50.0,
            strand_balance: 0,
            current_zone: None,
            action_state: ActionState::Idle,
            citizen: false,
            steer: None,
        }
    }

    /// A game on the default terrain, generated once for all these tests. Generating it per
    /// test (about a second each in debug) slows the parallel run enough to fail
    /// `test_terrain_generation_performance`.
    fn game() -> GameState {
        static TERRAIN: std::sync::OnceLock<TerrainConfig> = std::sync::OnceLock::new();
        let terrain = TERRAIN.get_or_init(TerrainConfig::get_default).clone();
        GameState::with_terrain(WorldGenParams::default(), terrain)
    }

    #[test]
    fn a_games_addresses_follow_its_terrain() {
        let mut game = game();
        assert_eq!(*game.addresses, game.terrain.addresses());
        assert_eq!(game.addresses.len(), game.terrain.terrain_plots.len(), "every lot has one (#483)");
        let mut fewer = game.terrain.clone();
        fewer.terrain_plots.truncate(10);
        game.set_terrain(fewer);
        assert_eq!(game.addresses.len(), 10, "a new terrain brings its own (#573)");
    }

    /// A walker in a pocket walled by crests, walking straight for the hall
    /// as a citizen with no way out does, crosses the pocket until the crest
    /// stops it, and is lifted to the nearest reached ground with its walk
    /// still on, without hearing "Too steep" for it (#522).
    #[test]
    fn a_walker_stranded_in_a_pocket_is_lifted_out_and_walks_on() {
        use super::super::stranded::{default_reach, POCKET};
        let mut game = game();
        game.wild_reach = default_reach();
        let mut hazel = player("Hazel", POCKET.0, POCKET.1);
        (hazel.dest_x, hazel.dest_y) = (Some(CENTER_X), Some(CENTER_Y));
        let id = hazel.id;
        game.add_player(hazel);
        let mut rx = game.subscribe();
        let at = |game: &GameState| (game.players[&id].x, game.players[&id].y);
        let lift = (0..600).find_map(|_| {
            let before = at(&game);
            let way_out = game.wild_reach.wait().way_out(before);
            game.process_movement();
            let after = at(&game);
            // Nobody walks further than a step in a tick
            ((after.0 - before.0).stable_hypot(after.1 - before.1) > 20.0).then_some((before, way_out, after))
        });
        let Some((from, way_out, to)) = lift else { panic!("never lifted out; at {:?}", at(&game)) };
        eprintln!("lifted from {from:?} to {to:?}");
        assert_eq!(way_out, WayOut::Lift(to), "to the nearest reached ground");
        assert!(game.players[&id].dest_x.is_some(), "its walk goes on");
        assert_eq!(game.wild_reach.wait().way_out(to), WayOut::Walks);
        while let Ok(e) = rx.try_recv() {
            assert!(!matches!(e, GameEvent::MovementBlocked { player_id, .. } if player_id == id), "{e:?}");
        }
    }

    /// From the ground the pocket's walker was lifted to, a walk back into
    /// it is stopped by the crest, and left there: it can walk out.
    #[test]
    fn a_walker_held_against_a_crest_it_can_walk_away_from_stays_put() {
        use super::super::stranded::{default_reach, POCKET};
        let mut game = game();
        game.wild_reach = default_reach();
        let WayOut::Lift(out) = game.wild_reach.wait().way_out(POCKET) else { panic!("{POCKET:?} is in a pocket") };
        let mut wren = player("Wren", out.0, out.1);
        (wren.dest_x, wren.dest_y) = (Some(POCKET.0), Some(POCKET.1));
        let id = wren.id;
        game.add_player(wren);
        let mut rx = game.subscribe();
        for _ in 0..600 {
            game.process_movement();
            if game.players[&id].dest_x.is_none() {
                break;
            }
        }
        let p = &game.players[&id];
        assert!(p.dest_x.is_none(), "stopped short of the pocket");
        assert_eq!(game.wild_reach.wait().way_out((p.x, p.y)), WayOut::Walks, "at ({}, {})", p.x, p.y);
        let mut steep = false;
        while let Ok(e) = rx.try_recv() {
            steep |= matches!(e, GameEvent::MovementBlocked { player_id, ref reason, .. } if player_id == id && reason == TOO_STEEP);
        }
        assert!(steep, "it hears the crest stop it");
    }

    /// A game with two players `gap` units apart, and a receiver subscribed after they joined
    fn two_players(gap: f64) -> (GameState, Uuid, Uuid, broadcast::Receiver<GameEvent>) {
        let mut game = game();
        let a = player("Ada", CENTER_X, CENTER_Y);
        let b = player("Bram", CENTER_X + gap, CENTER_Y);
        let (a_id, b_id) = (a.id, b.id);
        game.add_player(a);
        game.add_player(b);
        let rx = game.subscribe();
        (game, a_id, b_id, rx)
    }

    /// Run `command` and return the reason of the one `TradeFailed` it sent, checking who it
    /// went to and that it names the command it refuses (#89)
    fn refusal(game: &mut GameState, rx: &mut broadcast::Receiver<GameEvent>, command: Command, to: Uuid) -> String {
        let tag = match &command {
            Command::InitiateTrade { .. } => "trade_initiate",
            Command::AcceptTradeRequest { .. } => "trade_accept_request",
            Command::DeclineTradeRequest { .. } => "trade_decline_request",
            Command::UpdateTradeOffer { .. } => "trade_offer",
            Command::CancelTrade { .. } => "trade_cancel",
            other => panic!("{other:?} isn't a trade command"),
        };
        game.queue_command(command);
        game.process_commands();
        let mut reasons = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let GameEvent::TradeFailed { player_id, command, reason } = event {
                assert_eq!(player_id, to, "TradeFailed went to the wrong player");
                assert_eq!(command, tag, "TradeFailed named the wrong command");
                reasons.push(reason);
            }
        }
        assert_eq!(reasons.len(), 1, "expected exactly one TradeFailed, got {reasons:?}");
        reasons.pop().unwrap()
    }

    #[test]
    fn initiate_out_of_range_tells_the_asker() {
        let (mut game, a, b, mut rx) = two_players(TRADE_RANGE + 50.0);
        let reason = refusal(&mut game, &mut rx, Command::InitiateTrade { player_id: a, target_player_id: b }, a);
        assert!(reason.contains("too far"), "{reason}");
        assert!(reason.contains("Bram"), "{reason}");
    }

    #[test]
    fn initiate_with_an_offline_player_tells_the_asker() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let reason = refusal(
            &mut game,
            &mut rx,
            Command::InitiateTrade { player_id: a, target_player_id: Uuid::new_v4() },
            a,
        );
        assert!(reason.contains("offline"), "{reason}");
    }

    #[test]
    fn initiate_with_yourself_is_refused() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let reason = refusal(&mut game, &mut rx, Command::InitiateTrade { player_id: a, target_player_id: a }, a);
        assert!(reason.contains("yourself"), "{reason}");
    }

    #[test]
    fn initiate_while_the_target_is_trading_tells_the_asker() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        let c = player("Cato", CENTER_X + 10.0, CENTER_Y + 10.0);
        let c_id = c.id;
        game.add_player(c);
        game.trade_manager.request_trade(b, c_id).unwrap();
        game.trade_manager.accept_request(c_id).unwrap();
        while rx.try_recv().is_ok() {}

        let reason = refusal(&mut game, &mut rx, Command::InitiateTrade { player_id: a, target_player_id: b }, a);
        assert!(reason.contains("already in a trade"), "{reason}");
    }

    #[test]
    fn initiate_while_already_trading_tells_the_asker() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        let c = player("Cato", CENTER_X + 10.0, CENTER_Y + 10.0);
        let c_id = c.id;
        game.add_player(c);
        game.trade_manager.request_trade(a, c_id).unwrap();
        game.trade_manager.accept_request(c_id).unwrap();
        while rx.try_recv().is_ok() {}

        let reason = refusal(&mut game, &mut rx, Command::InitiateTrade { player_id: a, target_player_id: b }, a);
        assert!(reason.contains("You are already in a trade"), "{reason}");
    }

    #[test]
    fn repeat_request_is_refused() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        game.queue_command(Command::InitiateTrade { player_id: a, target_player_id: b });
        game.process_commands();
        while rx.try_recv().is_ok() {}

        let reason = refusal(&mut game, &mut rx, Command::InitiateTrade { player_id: a, target_player_id: b }, a);
        assert!(reason.contains("already pending"), "{reason}");
    }

    #[test]
    fn accept_request_with_none_pending_tells_the_player() {
        let (mut game, _, b, mut rx) = two_players(10.0);
        let reason = refusal(&mut game, &mut rx, Command::AcceptTradeRequest { player_id: b }, b);
        assert!(reason.contains("No pending trade request"), "{reason}");
    }

    #[test]
    fn decline_request_with_none_pending_tells_the_player() {
        let (mut game, _, b, mut rx) = two_players(10.0);
        let reason = refusal(&mut game, &mut rx, Command::DeclineTradeRequest { player_id: b }, b);
        assert!(reason.contains("No pending trade request"), "{reason}");
    }

    #[test]
    fn offer_outside_a_trade_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let reason = refusal(
            &mut game,
            &mut rx,
            Command::UpdateTradeOffer { player_id: a, items: vec![], strands: 5 },
            a,
        );
        assert!(reason.contains("Not in a trade"), "{reason}");
    }

    #[test]
    fn cancel_outside_a_trade_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let reason = refusal(&mut game, &mut rx, Command::CancelTrade { player_id: a }, a);
        assert!(reason.contains("Not in a trade"), "{reason}");
    }

    #[test]
    fn a_request_in_range_sends_no_error() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        game.queue_command(Command::InitiateTrade { player_id: a, target_player_id: b });
        game.process_commands();
        let mut requested = false;
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(event, GameEvent::TradeFailed { .. }), "unexpected {event:?}");
            requested |= matches!(event, GameEvent::TradeRequested { to_player, .. } if to_player == b);
        }
        assert!(requested);
    }

    /// Everyone sees a citizen marked (C0.1): as it joins, in every `positions`, and in
    /// the trade requests it makes. A person isn't.
    #[test]
    fn a_citizen_is_marked_wherever_it_is_named() {
        let mut game = game();
        let mut rx = game.subscribe();
        let ada = player("Ada", CENTER_X, CENTER_Y);
        let wren = PlayerState { citizen: true, ..player("Wren", CENTER_X + 10.0, CENTER_Y) };
        let (ada_id, wren_id) = (ada.id, wren.id);
        game.add_player(ada);
        game.add_player(wren);
        game.broadcast_positions();
        game.queue_command(Command::InitiateTrade { player_id: wren_id, target_player_id: ada_id });
        game.process_commands();

        let (mut joined, mut positions, mut requested) = (Vec::new(), None, None);
        while let Ok(event) = rx.try_recv() {
            match event {
                GameEvent::PlayerJoined(p) => joined.push((p.id, p.citizen)),
                GameEvent::PlayerPositions(ps) => positions = Some(ps),
                GameEvent::TradeRequested { from_player, from_citizen, .. } => requested = Some((from_player, from_citizen)),
                _ => {}
            }
        }
        assert_eq!(joined, [(ada_id, false), (wren_id, true)]);
        let positions = positions.expect("positions");
        let positions = positions.near(positions.cell_of(ada_id));
        let marked = |id| positions.iter().find(|p| p.id == id).map(|p| p.citizen);
        assert_eq!((marked(ada_id), marked(wren_id)), (Some(false), Some(true)));
        assert_eq!(requested, Some((wren_id, true)));
    }

    #[test]
    fn a_disconnect_cancels_with_the_real_trade_id_before_the_leave() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        game.trade_manager.request_trade(a, b).unwrap();
        let trade_id = game.trade_manager.accept_request(b).unwrap().id;
        while rx.try_recv().is_ok() {}

        game.remove_player(a);
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(&events[..], [
                GameEvent::TradeCancelled { trade_id: id, reason },
                GameEvent::PlayerLeft { player_id },
            ] if *id == trade_id && reason == "Player disconnected" && *player_id == a),
            "{events:?}"
        );
        assert!(!game.trade_manager.is_in_trade(b));
    }

    /// Every PlayerLeft for `id` waiting on `rx`
    fn leaves(rx: &mut broadcast::Receiver<GameEvent>, id: Uuid) -> usize {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|e| matches!(e, GameEvent::PlayerLeft { player_id } if *player_id == id))
            .count()
    }

    /// Open a client socket on session `sid` for `player` and take the avatar, as the WS
    /// handler does for a client.
    fn connect(
        game: &mut GameState,
        player: PlayerState,
        sid: &str,
    ) -> (u64, mpsc::UnboundedReceiver<PresenceEvent>) {
        let (id, rx) = game.open_socket(player.id, Holder::Session(sid.into()), true);
        game.claim_presence(player, id).unwrap();
        (id, rx)
    }

    fn presence_told(rx: &mut mpsc::UnboundedReceiver<PresenceEvent>) -> Vec<PresenceEvent> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    /// Ada and Bram, each on a client holding their avatar, and a receiver
    fn two_holders() -> (GameState, (Uuid, u64), (Uuid, u64), broadcast::Receiver<GameEvent>) {
        let mut game = game();
        let ada = player("Ada", CENTER_X, CENTER_Y);
        let bram = player("Bram", CENTER_X + 10.0, CENTER_Y);
        let (a, b) = (ada.id, bram.id);
        let (conn_a, _) = connect(&mut game, ada, "desk");
        let (conn_b, _) = connect(&mut game, bram, "laptop");
        let rx = game.subscribe();
        (game, (a, conn_a), (b, conn_b), rx)
    }

    #[test]
    fn a_holder_that_lags_ends_its_trade_for_both() {
        let (mut game, (a, conn_a), (b, _), mut rx) = two_holders();
        game.queue_command(Command::InitiateTrade { player_id: a, target_player_id: b });
        game.queue_command(Command::AcceptTradeRequest { player_id: b });
        game.process_commands();
        let trade_id = game.trade_manager.get_player_trade(a).expect("the trade opened").id;
        while rx.try_recv().is_ok() {}

        game.socket_lagged(a, conn_a);
        assert!(!game.trade_manager.is_in_trade(a) && !game.trade_manager.is_in_trade(b));
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(&events[..], [GameEvent::TradeCancelled { trade_id: t, reason }] if *t == trade_id && reason == TRADE_LAGGED),
            "{events:?}"
        );
    }

    #[test]
    fn a_holder_that_lags_ends_the_requests_to_and_from_it() {
        let (mut game, (a, conn_a), (b, _), mut rx) = two_holders();
        let cato = player("Cato", CENTER_X, CENTER_Y + 10.0);
        let c = cato.id;
        connect(&mut game, cato, "tablet");
        game.queue_command(Command::InitiateTrade { player_id: b, target_player_id: a });
        game.queue_command(Command::InitiateTrade { player_id: a, target_player_id: c });
        game.process_commands();
        assert_eq!(game.trade_manager.pending_requests.len(), 2);
        while rx.try_recv().is_ok() {}

        game.socket_lagged(a, conn_a);
        assert!(game.trade_manager.pending_requests.is_empty());
        let mut ended: Vec<(Uuid, Uuid)> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|e| match e {
                GameEvent::TradeRequestExpired { initiator, target } => (initiator, target),
                other => panic!("expected only expiries, got {other:?}"),
            })
            .collect();
        ended.sort();
        let mut want = vec![(b, a), (a, c)];
        want.sort();
        assert_eq!(ended, want);
    }

    #[test]
    fn a_watcher_that_lags_leaves_the_holders_trade_alone() {
        let (mut game, (a, _), (b, _), mut rx) = two_holders();
        let (watcher, _) = game.open_socket(a, Holder::Session("phone".into()), true);
        game.tell_watcher(a, watcher);
        game.queue_command(Command::InitiateTrade { player_id: a, target_player_id: b });
        game.queue_command(Command::AcceptTradeRequest { player_id: b });
        game.process_commands();
        while rx.try_recv().is_ok() {}

        game.socket_lagged(a, watcher);
        assert!(game.trade_manager.is_in_trade(a), "a watcher's lag ended the trade");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_stale_connection_ending_leaves_the_newer_ones_player() {
        let mut game = game();
        let ada = player("Ada", CENTER_X, CENTER_Y);
        let id = ada.id;
        let (conn_a, mut told_a) = connect(&mut game, ada.clone(), "desk");
        let (conn_b, mut told_b) = connect(&mut game, ada, "phone");
        assert_ne!(conn_a, conn_b);
        assert_eq!(
            presence_told(&mut told_a).last(),
            Some(&PresenceEvent::Changed { yours: false, held_by: Some(PresenceHolder::Client) }),
            "A should be told B took over"
        );
        assert_eq!(presence_told(&mut told_b), [PresenceEvent::Changed { yours: true, held_by: None }]);
        assert!(game.holds_presence(id, conn_b) && !game.holds_presence(id, conn_a));
        assert_eq!(game.presence_of_account(game.players[&id].account_id), Some(&Holder::Session("phone".into())));

        let mut rx = game.subscribe();
        assert!(!game.disconnect_player(id, conn_a));
        assert!(game.players.contains_key(&id), "ending A removed B's player");
        assert_eq!(leaves(&mut rx, id), 0, "ending A broadcast player_left");

        assert!(game.disconnect_player(id, conn_b));
        assert!(!game.players.contains_key(&id));
        assert!(!game.connections.contains_key(&id));
        assert_eq!(leaves(&mut rx, id), 1);
    }

    #[test]
    fn taking_the_avatar_back_keeps_the_player_where_they_stand() {
        let mut game = game();
        let ada = player("Ada", CENTER_X, CENTER_Y);
        let id = ada.id;
        let (desk, _) = connect(&mut game, ada.clone(), "desk");
        let (_phone, _) = connect(&mut game, ada.clone(), "phone");
        game.players.get_mut(&id).unwrap().x += 40.0;

        let mut rx = game.subscribe();
        // The desk claims with the row it read, which is older than the world.
        game.claim_presence(ada, desk).unwrap();
        assert!(game.holds_presence(id, desk));
        assert_eq!(game.players[&id].x, CENTER_X + 40.0, "the avatar didn't jump back");
        assert!(rx.try_recv().is_err(), "nobody joined or left");
    }

    #[test]
    fn when_the_holder_leaves_a_watcher_is_told_and_its_claim_brings_the_avatar_back() {
        let mut game = game();
        let ada = player("Ada", CENTER_X, CENTER_Y);
        let id = ada.id;
        let (bot, mut told_bot) = game.open_socket(id, Holder::Key(Uuid::new_v4()), true);
        let (desk, _) = connect(&mut game, ada.clone(), "desk");
        assert_eq!(game.claim_presence(ada.clone(), bot), Err(presence::Refused::ClientConnected));
        presence_told(&mut told_bot);

        assert!(game.disconnect_player(id, desk), "the desk held it");
        assert!(!game.players.contains_key(&id));
        assert_eq!(presence_told(&mut told_bot), [PresenceEvent::Changed { yours: false, held_by: None }]);
        assert_eq!(game.presence_of_account(ada.account_id), None);

        game.claim_presence(ada, bot).unwrap();
        assert!(game.players.contains_key(&id), "the bot brought the avatar back");
        assert!(game.disconnect_player(id, bot));
        assert!(!game.connections.contains_key(&id));
    }

    #[test]
    fn revoking_a_key_closes_only_its_open_sockets() {
        let mut game = game();
        let (key, other) = (Uuid::new_v4(), Uuid::new_v4());
        let mut a = game.watch_key(key);
        let closed = game.watch_key(key);
        drop(closed);
        let mut b = game.watch_key(key);
        let mut c = game.watch_key(other);

        assert_eq!(game.revoke_key(key), 2, "the closed socket isn't told");
        assert_eq!(a.try_recv().as_deref(), Ok("API key revoked"));
        assert!(b.try_recv().is_ok());
        assert!(c.try_recv().is_err(), "another key's socket stays open");
        assert_eq!(game.revoke_key(key), 0, "told once only");
    }

    #[test]
    fn an_ended_session_closes_its_connection_and_its_disconnect_removes_the_player() {
        let mut game = game();
        let ada = player("Ada", CENTER_X, CENTER_Y);
        let id = ada.id;
        let ended = ["desk".to_string()];
        assert_eq!(game.end_sessions(id, &ended), 0, "no connection to tell");
        let (conn, mut told) = connect(&mut game, ada.clone(), "desk");
        let (_phone, mut told_phone) = game.open_socket(id, Holder::Session("phone".into()), true);
        presence_told(&mut told);

        assert_eq!(game.end_sessions(id, &ended), 1);
        assert_eq!(presence_told(&mut told), [PresenceEvent::Ended], "the connection should be told to hang up");
        assert!(presence_told(&mut told_phone).is_empty(), "the other session's socket stays");

        let mut rx = game.subscribe();
        assert!(game.disconnect_player(id, conn), "it held the avatar");
        assert!(!game.players.contains_key(&id));
        assert_eq!(leaves(&mut rx, id), 1);
    }

    // #147: an unanswered request expires, and both players hear of it

    #[test]
    fn trading_expires_an_unanswered_request_and_tells_both_players() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        game.queue_command(Command::InitiateTrade { player_id: a, target_player_id: b });
        game.process_commands();
        let asked_at = game.trade_manager.pending_requests[&b].1;
        while rx.try_recv().is_ok() {}

        game.expire_trade_requests(asked_at + super::super::trading::TRADE_REQUEST_TTL / 2);
        assert!(rx.try_recv().is_err(), "a fresh request expired");

        game.expire_trade_requests(asked_at + super::super::trading::TRADE_REQUEST_TTL);
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(&events[..], [GameEvent::TradeRequestExpired { initiator, target }] if *initiator == a && *target == b),
            "{events:?}"
        );
        let reason = refusal(&mut game, &mut rx, Command::AcceptTradeRequest { player_id: b }, b);
        assert!(reason.contains("No pending trade request"), "{reason}");
    }

    #[test]
    fn trading_a_replaced_request_tells_its_asker() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        let c = player("Cato", CENTER_X + 10.0, CENTER_Y + 10.0);
        let c_id = c.id;
        game.add_player(c);
        game.queue_command(Command::InitiateTrade { player_id: b, target_player_id: a });
        game.process_commands();
        while rx.try_recv().is_ok() {}

        // Cato asks Ada too: Bram's request goes, and Bram is told before Ada hears of Cato's
        game.queue_command(Command::InitiateTrade { player_id: c_id, target_player_id: a });
        game.process_commands();
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(
                &events[..],
                [
                    GameEvent::TradeRequestExpired { initiator, target },
                    GameEvent::TradeRequested { from_player, to_player, .. },
                ] if *initiator == b && *target == a && *from_player == c_id && *to_player == a
            ),
            "{events:?}"
        );
        assert_eq!(game.trade_manager.pending_requests[&a].0, c_id);
    }

    #[test]
    fn trading_requests_are_swept_by_the_tick() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        game.trade_manager.request_trade(a, b).unwrap();
        // Make the request old enough to expire on the next sweep
        game.trade_manager.pending_requests.get_mut(&b).unwrap().1 -= super::super::trading::TRADE_REQUEST_TTL;
        for _ in 0..TRADE_EXPIRY_TICKS {
            game.tick();
        }
        let expired = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|e| matches!(e, GameEvent::TradeRequestExpired { .. }))
            .count();
        assert_eq!(expired, 1);
        assert!(game.trade_manager.pending_requests.is_empty());
    }

    #[test]
    fn trading_offer_events_carry_the_lines_they_were_given() {
        use capitol_protocol::ws::TradeItemInfo;
        let (mut game, a, b, mut rx) = two_players(10.0);
        game.trade_manager.request_trade(a, b).unwrap();
        let trade_id = game.trade_manager.accept_request(b).unwrap().id;
        let rope = TradeItemInfo {
            item_id: Uuid::new_v4(),
            item_type: "rope".into(),
            item_name: "Rope".into(),
            quantity: 2,
            quality: 77,
            units: Units::uniform(2, 77),
            maker: None,
        };
        game.queue_command(Command::UpdateTradeOffer { player_id: a, items: vec![rope.clone()], strands: 5 });
        game.process_commands();

        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            matches!(&events[..], [GameEvent::TradeOfferUpdated { trade_id: id, player_id, items, strands: 5 }]
                if *id == trade_id && *player_id == a && *items == vec![rope.clone()]),
            "{events:?}"
        );
        let trade = game.trade_manager.get_player_trade(a).unwrap();
        assert_eq!(trade.offer_a.items, vec![(rope.item_id, Units::uniform(2, 77))]);
    }

    #[test]
    fn a_stale_connection_leaves_the_players_trade_open() {
        let (mut game, a, b, _) = two_players(10.0);
        let ada = game.players[&a].clone();
        let (stale, _) = connect(&mut game, ada.clone(), "desk");
        let (_current, _) = connect(&mut game, ada, "phone");
        game.trade_manager.request_trade(a, b).unwrap();
        game.trade_manager.accept_request(b).unwrap();

        game.disconnect_player(a, stale);
        assert!(game.trade_manager.is_in_trade(a));
        assert!(game.trade_manager.is_in_trade(b));
    }

    // Steering (controls T0.2, #316)

    fn steer(game: &mut GameState, player_id: Uuid, heading: u16, throttle: u8, at: Option<(f64, f64)>) {
        game.queue_command(Command::Steer { player_id, heading, throttle, at });
        game.process_commands();
    }

    #[test]
    fn a_walking_steer_stops_a_gather_and_a_move_ends_the_steer() {
        let (mut game, a, _, _rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.queue_command(Command::StartExtraction { player_id: a, node_id, tool: None, wage: None });
        game.process_commands();
        assert!(matches!(game.players[&a].action_state, ActionState::Extracting { .. }));

        // Letting go of W just after E leaves the gather alone
        steer(&mut game, a, 0, 0, Some(AT));
        assert!(matches!(game.players[&a].action_state, ActionState::Extracting { .. }));
        assert!(game.players[&a].steer.is_none());

        steer(&mut game, a, 0, 10, None);
        assert_eq!(game.players[&a].action_state, ActionState::Idle, "walking off stops it, as a move does");
        assert!(game.players[&a].steer.is_some());

        game.queue_command(Command::Move { player_id: a, dest_x: CENTER_X, dest_y: CENTER_Y + 20.0 });
        game.process_commands();
        let ada = &game.players[&a];
        assert!(ada.steer.is_none());
        assert_eq!((ada.dest_x, ada.dest_y), (Some(CENTER_X), Some(CENTER_Y + 20.0)));
    }

    #[test]
    fn starting_a_gather_ends_the_steer() {
        let (mut game, a, _, _rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        steer(&mut game, a, 0, 10, None);
        game.queue_command(Command::StartExtraction { player_id: a, node_id, tool: None, wage: None });
        game.process_commands();
        assert!(matches!(game.players[&a].action_state, ActionState::Extracting { .. }));
        assert!(game.players[&a].steer.is_none());
        game.process_movement();
        assert_eq!((game.players[&a].x, game.players[&a].y), AT, "they stand to gather");
    }

    #[test]
    fn the_avatar_moving_to_another_device_ends_the_steer() {
        let mut game = game();
        let ada = player("Ada", CENTER_X, CENTER_Y);
        let id = ada.id;
        connect(&mut game, ada.clone(), "desk");
        steer(&mut game, id, 0, 10, None);
        assert!(game.players[&id].steer.is_some());
        connect(&mut game, ada, "phone");
        assert!(game.players[&id].steer.is_none(), "the phone never sent it");
    }

    #[test]
    fn a_steer_held_into_the_edge_hears_it_once_every_two_seconds() {
        use capitol_protocol::rules::WILDERNESS_OUTER_RADIUS;
        let mut game = game();
        let ada = player("Ada", CENTER_X + WILDERNESS_OUTER_RADIUS - 1.0, CENTER_Y);
        let id = ada.id;
        game.add_player(ada);
        let mut rx = game.subscribe();
        let mut told = Vec::new();
        for tick in 0..50 {
            game.tick_count = tick;
            // Held east, renewed every second
            if tick % 10 == 0 {
                steer(&mut game, id, 0, 10, None);
            }
            game.process_movement();
            while let Ok(event) = rx.try_recv() {
                if matches!(event, GameEvent::MovementBlocked { player_id, .. } if player_id == id) {
                    told.push(tick);
                }
            }
        }
        // From tick 1: a steer's first tick stands (#480)
        assert_eq!(told, [1, 21, 41]);
        assert!(game.players[&id].steer.is_some(), "still pushing");
    }

    // B9: a refused extraction says why, to the requester only

    /// Put a Wild Grass node `dx` units east of `at` into `game`, and return its id
    fn grass_node(game: &mut GameState, at: (f64, f64), dx: f64, state: NodeState) -> Uuid {
        game.resource_types.entry("grass".into()).or_insert_with(|| {
            ResourceType { tool_type: None, ..crate::content::resource_type("grass") }
        });
        let id = Uuid::new_v4();
        game.resource_nodes.insert(id, ResourceNodeState {
            id,
            resource_type: "grass".into(),
            x: at.0 + dx,
            y: at.1,
            base_quality: 50,
            state,
            harvested_by: None,
            regenerates_at_tick: None,
        });
        id
    }

    /// Run `command` and return the reason of the one `ExtractionFailed` it sent, checking who it went to
    fn extraction_refused(game: &mut GameState, rx: &mut broadcast::Receiver<GameEvent>, command: Command, to: Uuid) -> String {
        game.queue_command(command);
        game.process_commands();
        let mut reasons = Vec::new();
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(event, GameEvent::ExtractionStarted { .. }), "unexpected {event:?}");
            if let GameEvent::ExtractionFailed { player_id, reason } = event {
                assert_eq!(player_id, to, "ExtractionFailed went to the wrong player");
                reasons.push(reason);
            }
        }
        assert_eq!(reasons.len(), 1, "expected exactly one ExtractionFailed, got {reasons:?}");
        reasons.pop().unwrap()
    }

    const AT: (f64, f64) = (CENTER_X, CENTER_Y);

    #[test]
    fn extract_an_unknown_node_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let command = Command::StartExtraction { player_id: a, node_id: Uuid::new_v4(), tool: None, wage: None };
        let reason = extraction_refused(&mut game, &mut rx, command, a);
        assert!(reason.contains("gone"), "{reason}");
    }

    #[test]
    fn extract_a_node_of_unknown_type_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.resource_nodes.get_mut(&node_id).unwrap().resource_type = "moonflower".into();
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None, wage: None }, a);
        assert!(reason.contains("moonflower"), "{reason}");
    }

    #[test]
    fn extract_while_busy_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.players.get_mut(&a).unwrap().action_state =
            ActionState::Extracting { node_id: Uuid::new_v4(), progress: 0, duration: 30 };
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None, wage: None }, a);
        assert!(reason.contains("Already performing an action"), "{reason}");
    }

    #[test]
    fn extract_a_node_someone_else_is_gathering_tells_the_second_player() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.queue_command(Command::StartExtraction { player_id: a, node_id, tool: None, wage: None });
        game.process_commands();
        while rx.try_recv().is_ok() {}

        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: b, node_id, tool: None, wage: None }, b);
        assert!(reason.contains("Someone else is already gathering this Wild Grass"), "{reason}");
    }

    #[test]
    fn extract_a_depleted_node_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Depleted);
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None, wage: None }, a);
        assert!(reason.contains("grow back"), "{reason}");
    }

    #[test]
    fn extract_out_of_range_says_how_far() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, EXTRACTION_RANGE + 30.0, NodeState::Available);
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None, wage: None }, a);
        assert!(reason.contains("Too far"), "{reason}");
        assert!(reason.contains(&format!("{:.0} units", EXTRACTION_RANGE + 30.0)), "{reason}");
    }

    #[test]
    fn extract_without_the_tool_names_it() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.resource_types.get_mut("grass").unwrap().required_tool = Some("sickle".into());
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None, wage: None }, a);
        assert!(reason.contains("sickle"), "{reason}");
    }

    #[test]
    fn an_extraction_in_range_sends_no_failure() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.queue_command(Command::StartExtraction { player_id: a, node_id, tool: None, wage: None });
        game.process_commands();
        let mut started = false;
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(event, GameEvent::ExtractionFailed { .. }), "unexpected {event:?}");
            started |= matches!(event, GameEvent::ExtractionStarted { player_id, .. } if player_id == a);
        }
        assert!(started);
    }

    // E1.5: a gather wears its tool as it's written

    /// A player with an inventory holding a stone sickle with `durability` left, in a
    /// fresh database. Returns (player, inventory, the sickle as `best_tool` finds it).
    async fn gatherer(db: &sqlx::PgPool, durability: i32) -> (Uuid, Uuid, HeldTool) {
        crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, 'ada@test', 'x')")
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        let (player, inventory) = (Uuid::new_v4(), Uuid::new_v4());
        sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, 'ada')")
            .bind(player)
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(inventory)
            .bind(player)
            .execute(db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
             VALUES ($1, 'stone_sickle', 50, 1, $2, $3, 0, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(player)
        .bind(inventory)
        .bind(durability)
        .execute(db)
        .await
        .unwrap();
        let sickle = tools::best_tool(db, player, capitol_protocol::rules::tools::ToolType::Sickle)
            .await
            .unwrap()
            .unwrap();
        (player, inventory, sickle)
    }

    fn sickle_gather(player: Uuid, sickle: &HeldTool) -> PendingItem {
        PendingItem {
            player_id: player,
            node_id: Uuid::new_v4(),
            item_type: "fiber".into(),
            item_name: "Plant Fiber".into(),
            quantity: 5,
            quality: 52,
            tool: Some(super::super::extraction::GatherTool { held: sickle.clone(), by_hand: Some((3, 58)) }),
            wage: None,
        }
    }

    async fn fiber(db: &sqlx::PgPool, inventory: Uuid) -> Option<(i32, i32)> {
        sqlx::query_as("SELECT quantity, quality FROM items WHERE container_id = $1 AND item_type = 'fiber'")
            .bind(inventory)
            .fetch_optional(db)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn each_pull_wears_the_sickle(db: sqlx::PgPool) {
        let (ada, inventory, sickle) = gatherer(&db, 2).await;
        let (events, mut heard) = broadcast::channel(16);

        persist_gather(&db, sickle_gather(ada, &sickle), &events).await;
        assert_eq!(fiber(&db, inventory).await, Some((5, 52)), "the sickle's yield");
        let GameEvent::ExtractionCompleted { quantity: 5, tool: Some(used), .. } = heard.try_recv().unwrap() else {
            panic!("expected extraction_completed with the sickle");
        };
        assert_eq!((used.item_id, used.durability, used.max_durability), (sickle.item_id, 1, 60));
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::InventoryUpdated { .. }));

        // The second pull is its last
        persist_gather(&db, sickle_gather(ada, &sickle), &events).await;
        assert_eq!(fiber(&db, inventory).await.map(|f| f.0), Some(10));
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::ExtractionCompleted { tool: Some(ref t), .. } if t.durability == 0));
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::InventoryUpdated { .. }));
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::ToolBroke { item_id, .. } if item_id == sickle.item_id));
    }

    /// The statements sqlx logs (its `sqlx::query` events), by their
    /// summary, its first words
    #[derive(Clone, Default)]
    struct Statements(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl Statements {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Statements {
        fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
            struct Summary(String);
            impl tracing::field::Visit for Summary {
                fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                    if field.name() == "summary" {
                        self.0 = value.to_string();
                    }
                }
                fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                    if field.name() == "summary" {
                        self.0 = format!("{value:?}");
                    }
                }
            }
            if event.metadata().target() == "sqlx::query" {
                let mut summary = Summary(String::new());
                event.record(&mut summary);
                self.0.lock().unwrap().push(summary.0);
            }
        }
    }

    /// A gather onto a stack it joins, wearing its tool, is two statements
    /// and its transaction's: the tool's wear, and the units placed with the
    /// Gazette's tally (#524). It was seven. Prints how long one takes on
    /// one lane, which #524 measured at 5.4 ms on an idle database.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_gather_is_two_statements_and_a_commit(db: sqlx::PgPool) {
        use tracing_subscriber::layer::SubscriberExt;
        const GATHERS: usize = 200;
        let (ada, inventory, sickle) = gatherer(&db, 10_000).await;
        let (events, _heard) = broadcast::channel(4 * GATHERS);
        let statements = Statements::default();
        let _default = tracing::subscriber::set_default(tracing_subscriber::registry().with(statements.clone()));
        // The first makes the stack the rest join, and hits sqlx's callsites
        // so they can be asked again with this thread's subscriber (#412)
        assert!(persist_gather(&db, sickle_gather(ada, &sickle), &events).await);
        tracing::callsite::rebuild_interest_cache();
        statements.take();

        assert!(persist_gather(&db, sickle_gather(ada, &sickle), &events).await);
        let one = statements.take();
        eprintln!("A gather sends {one:?}");
        let began = Instant::now();
        for _ in 0..GATHERS {
            assert!(persist_gather(&db, sickle_gather(ada, &sickle), &events).await);
        }
        let took = began.elapsed();
        let sent = statements.take().len();
        eprintln!(
            "{GATHERS} gathers on one lane: {:.2} ms a gather, {:.1} statements a gather",
            took.as_secs_f64() * 1000.0 / GATHERS as f64,
            sent as f64 / GATHERS as f64,
        );
        assert_eq!(fiber(&db, inventory).await.map(|f| f.0), Some(5 * (GATHERS as i32 + 2)), "every one written");
        assert_eq!(one.len(), 3, "{one:?}");
        assert!(one[0].starts_with("UPDATE items i") && one[1].starts_with("WITH inv AS"), "{one:?}");
        assert_eq!(sent, one.len() * GATHERS, "each the same");
    }

    /// #622: the pool hands out a connection that was just busy without a
    /// ping, so after the database restarts it can be dead. A gather that
    /// finds it so is tried again on a new one, where before it was lost.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_gather_on_a_dead_connection_is_tried_again(pool: PgPoolOptions, connect: PgConnectOptions) {
        use crate::state::tests::{kill, one_connection};
        let db = pool.connect_with(connect.clone()).await.unwrap();
        let (ada, inventory, sickle) = gatherer(&db, 2).await;
        let (events, mut heard) = broadcast::channel(16);
        let (writer, pid) = one_connection(&connect, Duration::from_secs(60)).await;
        kill(&db, pid).await;

        assert!(persist_gather(&writer, sickle_gather(ada, &sickle), &events).await, "not written");
        assert_eq!(fiber(&db, inventory).await, Some((5, 52)), "written once");
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::ExtractionCompleted { quantity: 5, .. }), "told once");
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::InventoryUpdated { .. }));
        assert!(heard.try_recv().is_err());
    }

    /// Only a connection lost before anything was committed is worth another
    /// try: a broken socket, a server that ended the session, a lost one.
    /// Not a statement that failed on a live connection (#622).
    #[test]
    fn a_broken_connection_is_told_from_a_failed_statement() {
        assert!(broken(&sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset))));
        assert!(broken(&sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))));
        assert!(!broken(&sqlx::Error::RowNotFound));
        assert!(!broken(&sqlx::Error::PoolTimedOut), "a database that's down isn't helped by trying at once");
        assert!(!broken(&sqlx::Error::Protocol("unexpected message".into())));
    }

    /// #622's measure: a `SELECT 1`, then a gather, on each pool in turn, so
    /// the machine's load falls on both alike. Prints each one's median: the
    /// server's pool should save a round trip on each, about 0.26 ms on an
    /// idle database. Nothing to assert, as for the lanes below.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_gather_waits_a_round_trip_less(pool: PgPoolOptions, connect: PgConnectOptions) {
        const EACH: usize = 400;
        let db = pool.connect_with(connect.clone()).await.unwrap();
        let (ada, _, sickle) = gatherer(&db, 100_000).await;
        let (events, _heard) = broadcast::channel(4 * EACH);
        let pinging = PgPoolOptions::new().max_connections(DB_POOL_SIZE).connect_with(connect.clone()).await.unwrap();
        let idle = crate::state::db_pool_options(crate::state::PING_AFTER_IDLE).connect_with(connect).await.unwrap();
        let pools = [("pinging every acquire", &pinging), ("pinging after idle", &idle)];
        // The stack the rest join, and each pool's connections opened
        for (_, pool) in pools {
            assert!(persist_gather(pool, sickle_gather(ada, &sickle), &events).await);
        }
        let ms = |began: Instant| began.elapsed().as_secs_f64() * 1000.0;
        let (mut selects, mut gathers) = ([Vec::new(), Vec::new()], [Vec::new(), Vec::new()]);
        for _ in 0..EACH {
            for (i, (_, pool)) in pools.iter().enumerate() {
                let began = Instant::now();
                sqlx::query("SELECT 1").execute(*pool).await.unwrap();
                selects[i].push(ms(began));
                let began = Instant::now();
                assert!(persist_gather(pool, sickle_gather(ada, &sickle), &events).await);
                gathers[i].push(ms(began));
            }
        }
        let median = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        for (i, (name, _)) in pools.iter().enumerate() {
            eprintln!(
                "{name}: a SELECT 1 in {:.3} ms, a gather in {:.3} ms (medians of {EACH})",
                median(selects[i].clone()),
                median(gathers[i].clone()),
            );
        }
    }

    /// #524's other measure: 1000 gathers by 200 players, each wearing a
    /// sickle, saved on one of the writer's lanes and then on [`WRITERS`].
    /// Prints the gathers a second each managed. Nothing to assert: it's a
    /// measure of the machine as much as the code.
    ///
    /// Since #622 it's an A/B of the pool, taken in turns at each count of
    /// lanes: sqlx's default, which pings every connection on its way out,
    /// and the server's, which pings only one that waited.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_writers_lanes_gathers_a_second(pool: PgPoolOptions, connect: PgConnectOptions) {
        let db = pool.connect_with(connect.clone()).await.unwrap();
        let pools = [
            ("pinging every acquire", PgPoolOptions::new().max_connections(DB_POOL_SIZE)),
            ("pinging after idle", crate::state::db_pool_options(crate::state::PING_AFTER_IDLE)),
        ];
        let mut writers_pools = Vec::new();
        for (name, options) in pools {
            writers_pools.push((name, options.connect_with(connect.clone()).await.unwrap()));
        }
        const PLAYERS: usize = 200;
        const EACH: usize = 5;
        let (_, _, first) = gatherer(&db, 10_000).await;
        let mut sickles = Vec::new();
        for n in 0..PLAYERS {
            let p = crate::engine::market::tests::player(&db, &format!("p{n}"), 0).await;
            let item_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
                 VALUES ($1, 'stone_sickle', 50, 1, $2, $3, 0, 10000)",
            )
            .bind(item_id)
            .bind(p.id)
            .bind(p.inventory)
            .execute(&db)
            .await
            .unwrap();
            sickles.push((p.id, HeldTool { item_id, ..first.clone() }));
        }
        let (events, _heard) = broadcast::channel(16);
        // Each makes its stack first, so every lane timed joins one
        for (p, s) in &sickles {
            assert!(persist_gather(&db, sickle_gather(*p, s), &events).await);
        }
        let gathers = || -> Vec<PendingItem> {
            (0..EACH).flat_map(|_| sickles.iter().map(|(p, s)| sickle_gather(*p, s))).collect()
        };
        for lanes in [1, WRITERS] {
            for (name, writers_db) in &writers_pools {
                let writers: Vec<_> = (0..lanes)
                    .map(|_| {
                        let (to_writer, from_tick) = mpsc::unbounded_channel();
                        (to_writer, tokio::spawn(run_writer(writers_db.clone(), events.clone(), from_tick)))
                    })
                    .collect();
                let began = Instant::now();
                for (lane, finished) in Finished::dealt(gathers(), Vec::new(), Vec::new()) {
                    writers[lane % lanes].0.send(finished).unwrap();
                }
                for (to_writer, writer) in writers {
                    drop(to_writer);
                    writer.await.unwrap();
                }
                let took = began.elapsed().as_secs_f64();
                eprintln!(
                    "{} gathers on {lanes} lane(s), {name}: {:.2} ms a gather, {:.0} gathers a second",
                    PLAYERS * EACH,
                    took * 1000.0 / (PLAYERS * EACH) as f64,
                    (PLAYERS * EACH) as f64 / took,
                );
            }
        }
    }

    /// The writer saves the tick's gathers in order, off the tick, and tells the
    /// gatherer only once each is written (#304)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_writer_saves_gathers_in_order_then_tells(db: sqlx::PgPool) {
        let (ada, inventory, sickle) = gatherer(&db, 2).await;
        let (events, mut heard) = broadcast::channel(16);
        let (to_writer, from_tick) = mpsc::unbounded_channel();
        let writer = tokio::spawn(run_writer(db.clone(), events, from_tick));

        let both = vec![sickle_gather(ada, &sickle), sickle_gather(ada, &sickle)];
        to_writer.send(Finished::new(both, Vec::new(), Vec::new())).unwrap();
        drop(to_writer);
        writer.await.unwrap();

        assert_eq!(fiber(&db, inventory).await.map(|f| f.0), Some(10), "both written");
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::ExtractionCompleted { tool: Some(ref t), .. } if t.durability == 1));
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::InventoryUpdated { .. }));
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::Tutorial { .. }), "the first counts on the road in");
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::ExtractionCompleted { tool: Some(ref t), .. } if t.durability == 0));
    }

    /// A tick's saves are dealt by the box they write into, in order within a
    /// lane: a wage gather by its employer, a station craft by its station
    /// (#407)
    #[test]
    fn saves_are_dealt_to_lanes_by_the_box_they_write_into() {
        // Keys that land in lanes 0, 1 and 2
        let key = |n: u128| Uuid::from_u128(n * 1_000 * WRITERS as u128 + n);
        let (ada, employer, station) = (key(0), key(1), key(2));
        let gather = |quantity: i32, wage: Option<Uuid>| PendingItem {
            player_id: ada,
            node_id: Uuid::nil(),
            item_type: "fiber".into(),
            item_name: "Plant Fiber".into(),
            quantity,
            quality: 50,
            tool: None,
            wage: wage.map(|employer_id| ShiftWork {
                offer_id: Uuid::nil(),
                shift_id: Uuid::nil(),
                employer_id,
                worker_id: ada,
                work: capitol_protocol::rules::wages::Work::Gather { plot_id: Uuid::nil(), resource: "grass".into() },
            }),
        };
        let craft = |station_id: Option<Uuid>| FinishedCraft {
            player_id: ada,
            operation_id: Uuid::nil(),
            recipe_id: "rope_basic".into(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            tool: None,
            station_id,
            wage: None,
        };
        let dealt = Finished::dealt(
            vec![gather(1, None), gather(2, Some(employer)), gather(3, None)],
            vec![craft(Some(station)), craft(None)],
            vec![ada],
        );
        let lanes: Vec<_> = dealt
            .iter()
            .map(|(lane, f)| {
                let gathers: Vec<i32> = f.gathers.iter().map(|g| g.quantity).collect();
                (*lane, gathers, f.crafts.iter().map(|c| c.station_id).collect::<Vec<_>>(), f.walks.len())
            })
            .collect();
        assert_eq!(
            lanes,
            vec![
                (lane(ada), vec![1, 3], vec![None], 1),
                (lane(employer), vec![2], vec![], 0),
                (lane(station), vec![], vec![Some(station)], 0),
            ]
        );
    }

    /// Two lanes gathering into one inventory at once race for its free
    /// slots, and the loser is tried again: every gather is written (#407)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn gathers_that_race_for_a_slot_are_tried_again(db: sqlx::PgPool) {
        let (ada, inventory, _sickle) = gatherer(&db, 1).await;
        let (events, _heard) = broadcast::channel(256);
        let types: Vec<String> = sqlx::query_scalar("SELECT id FROM item_types WHERE id <> 'stone_sickle' ORDER BY id LIMIT 16")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(types.len(), 16, "enough item types for a new stack each");
        let gather = |item_type: &str| PendingItem {
            player_id: ada,
            node_id: Uuid::new_v4(),
            item_type: item_type.into(),
            item_name: item_type.into(),
            quantity: 1,
            quality: 50,
            tool: None,
            wage: None,
        };
        // Each pair is two new stacks wanting the same first free slot
        for pair in types.chunks(2) {
            let (a, b) = tokio::join!(
                persist_gather(&db, gather(&pair[0]), &events),
                persist_gather(&db, gather(&pair[1]), &events),
            );
            assert!(a && b, "both of {pair:?} written");
        }
        let stacks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE container_id = $1 AND item_type <> 'stone_sickle'")
            .bind(inventory)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(stacks, 16);
    }

    /// A walk is judged where it ends: one that ends `WALK_DISTANCE` or more
    /// from where it began is handed on to the writer, a step aside isn't
    /// (#439)
    #[test]
    fn a_walk_that_ends_far_enough_is_handed_on_to_judge() {
        use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};
        let mut game = game();
        let ada = player("ada", SPAWN_X, SPAWN_Y);
        let id = ada.id;
        game.add_player(ada);
        let walk = |game: &mut GameState, dy: f64| {
            game.queue_command(Command::Move { player_id: id, dest_x: SPAWN_X, dest_y: SPAWN_Y - dy });
            for _ in 0..20 {
                game.tick();
            }
        };
        walk(&mut game, WALK_DISTANCE / 2.0);
        assert!(game.take_walked().is_empty(), "a step aside isn't a walk");
        walk(&mut game, WALK_DISTANCE * 2.0);
        assert_eq!(game.players[&id].y, SPAWN_Y - WALK_DISTANCE * 2.0, "it ended");
        assert_eq!(game.take_walked(), vec![id]);
        assert!(game.walks_from.is_empty());
    }

    /// What the writer wrote is judged for the road in: a gather, a craft and
    /// a walk each count once, and a failed pull counts for nothing (#439)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_writer_judges_what_it_wrote_for_the_road_in(db: sqlx::PgPool) {
        use super::super::craft_completion::tests::{crafter, rope, stack};
        use super::super::tutorial::progress;
        use capitol_protocol::tutorial::{Step, Steps};
        use capitol_protocol::ws::ServerMessage;

        let (ada, _, sickle) = gatherer(&db, 10).await;
        let cy = crate::engine::plot_sync::tests::player(&db, "cy", 0).await;
        let bo = crafter(&db, "bo").await;
        let fiber = stack(&db, bo.id, bo.input, 0, "fiber", 4).await;
        let mut failed = sickle_gather(cy, &sickle);
        (failed.quantity, failed.quality, failed.tool) = (0, 0, None);

        let (events, mut heard) = broadcast::channel(64);
        let (to_writer, from_tick) = mpsc::unbounded_channel();
        let writer = tokio::spawn(run_writer(db.clone(), events, from_tick));
        let gathers = vec![sickle_gather(ada, &sickle), failed, sickle_gather(ada, &sickle)];
        to_writer.send(Finished::new(gathers, vec![rope(&bo, fiber)], vec![ada, ada])).unwrap();
        drop(to_writer);
        writer.await.unwrap();

        let done = |player| progress(&db, player);
        assert_eq!(done(ada).await.unwrap().done, Steps::from_iter([Step::Walk, Step::GatherGrass]));
        assert_eq!(done(bo.id).await.unwrap().done, Steps::from_iter([Step::TwistRope]));
        assert_eq!(done(cy).await.unwrap().done, Steps::NONE, "a failed pull gathered nothing");
        let mut told = Vec::new();
        while let Ok(event) = heard.try_recv() {
            if let GameEvent::Tutorial { player_id, message: ServerMessage::TutorialStep { step, .. } } = event {
                told.push((player_id, step));
            }
        }
        assert_eq!(told, [(ada, Step::GatherGrass), (bo.id, Step::TwistRope), (ada, Step::Walk)], "each once");
    }

    /// A steering player's position is saved, and they come back standing (T0.2)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_steering_player_is_saved_and_reloads_with_no_steer(db: sqlx::PgPool) {
        let (ada, _, _) = gatherer(&db, 2).await;
        let account: Uuid = sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1")
            .bind(ada)
            .fetch_one(&db)
            .await
            .unwrap();
        let mut game = game();
        let player = crate::ws::load_player(&db, account).await.unwrap();
        game.add_player(PlayerState { x: CENTER_X, y: CENTER_Y, ..player });
        steer(&mut game, ada, 0, 10, Some((CENTER_X + 3.0, CENTER_Y)));
        // The first tick stands (#480), then two walk
        game.process_movement();
        game.process_movement();
        game.process_movement();
        assert!(game.players[&ada].steer.is_some());

        persist_positions(&db, &saved_positions(&game)).await.unwrap();
        let back = crate::ws::load_player(&db, account).await.unwrap();
        assert_eq!((back.x, back.y), (CENTER_X + 8.0, CENTER_Y));
        assert_eq!((back.dest_x, back.dest_y), (None, None));
        assert!(back.steer.is_none());
    }

    /// Every player's position in one statement, a destination or none (#304)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn positions_are_saved_in_one_statement(db: sqlx::PgPool) {
        let (ada, _, _) = gatherer(&db, 2).await;
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, 'bo@test', 'x')")
            .bind(account)
            .execute(&db)
            .await
            .unwrap();
        let bo = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name, destination_x, destination_y) VALUES ($1, $2, 'bo', 1.0, 2.0)")
            .bind(bo)
            .bind(account)
            .execute(&db)
            .await
            .unwrap();

        let saved = [
            SavedPosition { id: ada, x: 6810.5, y: 6790.25, dest_x: Some(6900.0), dest_y: Some(6700.0) },
            SavedPosition { id: bo, x: 7000.0, y: 6000.0, dest_x: None, dest_y: None },
        ];
        persist_positions(&db, &saved).await.unwrap();
        persist_positions(&db, &[]).await.unwrap();

        for want in &saved {
            let (x, y, dest_x, dest_y): (f64, f64, Option<f64>, Option<f64>) = sqlx::query_as(
                "SELECT position_x, position_y, destination_x, destination_y FROM players WHERE id = $1",
            )
            .bind(want.id)
            .fetch_one(&db)
            .await
            .unwrap();
            assert_eq!(SavedPosition { id: want.id, x, y, dest_x, dest_y }, *want);
        }
    }

    /// A sickle put down mid-gather: the gather is what hands would have given,
    /// and a resource that needs the sickle gives nothing
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_gather_whose_tool_left_falls_back_to_hands(db: sqlx::PgPool) {
        let (ada, inventory, sickle) = gatherer(&db, 10).await;
        sqlx::query("UPDATE items SET container_id = NULL, slot_index = NULL WHERE id = $1")
            .bind(sickle.item_id)
            .execute(&db)
            .await
            .unwrap();
        let (events, mut heard) = broadcast::channel(16);

        persist_gather(&db, sickle_gather(ada, &sickle), &events).await;
        assert_eq!(fiber(&db, inventory).await, Some((3, 58)), "by hand");
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::ExtractionCompleted { quantity: 3, tool: None, .. }));

        let mut gated = sickle_gather(ada, &sickle);
        gated.tool.as_mut().unwrap().by_hand = None;
        persist_gather(&db, gated, &events).await;
        assert_eq!(fiber(&db, inventory).await, Some((3, 58)), "nothing more");
        let _ = heard.try_recv(); // the first gather's inventory_updated
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::ExtractionFailed { player_id, .. } if player_id == ada));
        let left: i32 = sqlx::query_scalar("SELECT durability FROM items WHERE id = $1")
            .bind(sickle.item_id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left, 10, "a tool that wasn't used doesn't wear");
    }

    /// A failed pull (D5, #206) writes nothing and tallies nothing, but the tool
    /// wears and the gatherer hears of it, with the inventory to refetch
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_failed_pull_gives_nothing_and_wears_the_tool(db: sqlx::PgPool) {
        let (ada, inventory, sickle) = gatherer(&db, 10).await;
        let (events, mut heard) = broadcast::channel(16);

        let mut failed = sickle_gather(ada, &sickle);
        (failed.quantity, failed.quality) = (0, 0);
        failed.tool.as_mut().unwrap().by_hand = Some((0, 0));
        persist_gather(&db, failed, &events).await;
        assert_eq!(fiber(&db, inventory).await, None, "nothing gathered");
        let GameEvent::ExtractionCompleted { quantity: 0, tool: Some(used), .. } = heard.try_recv().unwrap() else {
            panic!("expected an empty extraction_completed with the sickle");
        };
        assert_eq!(used.durability, 9, "the pull wore it");
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::InventoryUpdated { container_id, .. } if container_id == inventory));
        let tallied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM production_tally")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(tallied, 0, "the Gazette counts no failed pull");

        // By hand there's no tool to wear, so no inventory changed
        let mut by_hand = sickle_gather(ada, &sickle);
        (by_hand.quantity, by_hand.quality, by_hand.tool) = (0, 0, None);
        persist_gather(&db, by_hand, &events).await;
        assert!(matches!(heard.try_recv().unwrap(), GameEvent::ExtractionCompleted { quantity: 0, tool: None, .. }));
        assert!(heard.try_recv().is_err(), "nothing else");
    }

    /// Gathered fiber joins the first fiber stack, whatever its grade, and keeps its quality (D2, E1.8)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_gather_joins_the_first_stack_and_keeps_its_quality(db: sqlx::PgPool) {
        use crate::engine::units::test_units::{all_in, stack_of, units};
        let (ada, inventory, _) = gatherer(&db, 10).await; // the sickle is in slot 0
        let first = stack_of(&db, ada, inventory, 1, "fiber", &Units::uniform(5, 95)).await;
        stack_of(&db, ada, inventory, 2, "fiber", &Units::uniform(5, 40)).await;
        let gathered = |quality| PendingItem {
            player_id: ada,
            node_id: Uuid::new_v4(),
            item_type: "fiber".into(),
            item_name: "Plant Fiber".into(),
            quantity: 5,
            quality,
            tool: None,
            wage: None,
        };
        let mut conn = db.acquire().await.unwrap();

        place_gathered(&mut conn, &gathered(50)).await.unwrap();
        place_gathered(&mut conn, &gathered(60)).await.unwrap();
        drop(conn);

        assert_eq!(units(&db, first).await, Units::from_pairs([(50, 5), (60, 5), (95, 5)]));
        assert_eq!(
            all_in(&db, inventory, "fiber").await,
            Units::from_pairs([(40, 5), (50, 5), (60, 5), (95, 5)]),
            "every unit kept its quality"
        );

        // Gathered units carry the gatherer's mark (#231), joined or in a slot
        // of their own
        use crate::engine::units::{test_units::all_marked, Marked};
        let mut conn = db.acquire().await.unwrap();
        sqlx::query("DELETE FROM items WHERE container_id = $1 AND item_type = 'fiber'")
            .bind(inventory)
            .execute(&mut *conn)
            .await
            .unwrap();
        place_gathered(&mut conn, &gathered(70)).await.unwrap();
        place_gathered(&mut conn, &gathered(70)).await.unwrap();
        drop(conn);
        assert_eq!(all_marked(&db, ada, "fiber").await, Marked::by(Some(ada), &Units::uniform(10, 70)));
    }

    /// A gather passes over a stack too full to take it for the first free
    /// slot, is tallied for the Gazette once wherever it goes, and with no
    /// slot free is lost and tallied nowhere (#524: all one statement now)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_gather_passes_over_a_full_stack_and_is_tallied_where_it_goes(db: sqlx::PgPool) {
        use crate::engine::units::test_units::{stack_of, units};
        let (ada, inventory, _) = gatherer(&db, 10).await; // the sickle is in slot 0
        let full = stack_of(&db, ada, inventory, 1, "fiber", &Units::uniform(i32::MAX - 2, 40)).await;
        let gathered = PendingItem {
            player_id: ada,
            node_id: Uuid::new_v4(),
            item_type: "fiber".into(),
            item_name: "Plant Fiber".into(),
            quantity: 5,
            quality: 60,
            tool: None,
            wage: None,
        };
        let tallied = || async {
            sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(quantity), 0)::BIGINT FROM production_tally WHERE player_id = $1")
                .bind(ada)
                .fetch_one(&db)
                .await
                .unwrap()
        };
        let mut conn = db.acquire().await.unwrap();
        assert_eq!(place_gathered(&mut conn, &gathered).await.unwrap(), Some(inventory));
        drop(conn);
        assert_eq!(units(&db, full).await, Units::uniform(i32::MAX - 2, 40), "the full stack is left alone");
        let (slot, placed): (i32, Uuid) =
            sqlx::query_as("SELECT slot_index, id FROM items WHERE container_id = $1 AND item_type = 'fiber' AND id <> $2")
                .bind(inventory)
                .bind(full)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!((slot, units(&db, placed).await), (2, Units::uniform(5, 60)), "a stack of its own in the first free slot");
        assert_eq!(tallied().await, 5);

        // With every slot taken there's nowhere to put it
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             SELECT gen_random_uuid(), 'stone_sickle', 50, 1, $1, $2, s
             FROM generate_series(3, (SELECT ct.slot_count - 1 FROM container_types ct WHERE ct.id = 'player_inventory')) s",
        )
        .bind(ada)
        .bind(inventory)
        .execute(&db)
        .await
        .unwrap();
        let mut conn = db.acquire().await.unwrap();
        let mut more = gathered.clone();
        more.item_type = "sticks".into();
        assert_eq!(place_gathered(&mut conn, &more).await.unwrap(), None);
        drop(conn);
        assert_eq!(tallied().await, 5, "nothing tallied for what was lost");
    }

    /// Rope from fiber, by hand
    fn rope_recipe() -> Recipe {
        use crate::models::{RecipeInput, RecipeOutput};
        Recipe {
            id: "craft_rope".into(),
            name: "Rope".into(),
            description: None,
            category: "crafting".into(),
            inputs: vec![RecipeInput { item_type: "fiber".into(), quantity: 3, quality_weight: 1.0 }],
            outputs: vec![RecipeOutput { item_type: "rope".into(), quantity_min: 1, quantity_max: 1 }],
            base_duration_ticks: 10,
            quality_weights: quality::Weights::for_recipe([("fiber", 1.0)], None, None, false, 0, 0),
            required_station_type: None,
            tool_type: None,
        }
    }

    fn fiber_stack(owner: Uuid, n: i32, quality: i32) -> crate::models::Item {
        crate::models::Item {
            id: Uuid::new_v4(),
            item_type: "fiber".into(),
            quality,
            quantity: n,
            owner_id: Some(owner),
            container_id: None,
            slot_index: None,
            created_at: chrono::Utc::now(),
            durability: None,
            min_quality: quality,
            units: Units::uniform(n, quality),
            maker: None,
        }
    }

    /// The craft takes the stacks its own command names. Before #183 they went
    /// through a table keyed by player, so a second `craft` sent before the
    /// tick swapped the first one's inputs for its own, and was itself dropped
    /// without a word.
    #[test]
    fn a_craft_takes_the_stacks_its_own_command_names() {
        let mut game = game();
        let recipe = rope_recipe();
        game.recipes.insert(recipe.id.clone(), recipe.clone());
        let ada = player("Ada", CENTER_X, CENTER_Y);
        let a = ada.id;
        game.add_player(ada);
        let mut rx = game.subscribe();

        let (poor, fine) = (fiber_stack(a, 3, 20), fiber_stack(a, 3, 90));
        for stack in [&poor, &fine] {
            game.queue_command(Command::StartCrafting {
                player_id: a,
                recipe_id: recipe.id.clone(),
                inputs: vec![stack.clone()],
                tool: None,
            });
        }
        game.process_commands();

        let ops: Vec<_> = game.crafting_operations.values().collect();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].input_items_to_consume, vec![(poor.id, Units::uniform(3, 20))], "the first craft spends the poor fiber");
        let mut refusals = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let GameEvent::CraftingFailed { player_id, reason } = event {
                assert_eq!(player_id, a);
                refusals.push(reason);
            }
        }
        assert_eq!(refusals, ["Already performing an action"], "the second craft is refused out loud");
    }
}
