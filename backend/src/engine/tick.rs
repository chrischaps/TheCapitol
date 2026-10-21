use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use uuid::Uuid;

use super::presence::{self, Holder, PresenceEvent, Sockets};

use super::commands::Command;
use super::craft_completion::{finish_craft, FinishedCraft};
use super::crafting::{cancel_crafting, process_crafting, start_crafting, CraftingOperation};
use super::events::GameEvent;
use super::extraction::{cancel_extraction, process_extractions, start_extraction, PendingItem};
use super::ledger;
use super::units;
use super::tools::{self, HeldTool, Wear};
use super::movement::process_movement;
use super::regeneration::process_regeneration;
use super::trading::{TradeManager, TRADE_RANGE};
use crate::models::{ActionState, ContainerType, ItemType, PlayerPosition, PlayerState, Recipe, ResourceNodeState, ResourceType, StationState, StationType, TerrainConfig, WorldGenParams};
use crate::state::AppState;

use capitol_protocol::ledger::UnitFlow;
use capitol_protocol::rest::{CatalogResponse, Subscription, SubscriptionStatus};
use capitol_protocol::rules::ground::Ground;
use capitol_protocol::rules::quality;
use capitol_protocol::rules::stacking::Units;
use capitol_protocol::rules::TICK_MS as TICK_RATE_MS;

/// Ticks between sweeps for unanswered trade requests (once a second)
const TRADE_EXPIRY_TICKS: u64 = 10;

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
    /// The last tick's positions, so the next can say who changed cell (#275)
    last_positions: Option<Arc<super::interest::Positions>>,
    pub command_queue: Vec<Command>,
    pub event_sender: broadcast::Sender<GameEvent>,
    pub tick_count: u64,
    pub pending_items: Vec<PendingItem>,
    /// The tool each gather under way started with, by player (E1.5)
    pub gather_tools: HashMap<Uuid, HeldTool>,
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
    /// The terrain's land and wild water, indexed for walking on (W4)
    pub ground: Arc<Ground>,
    // World generation parameters (for admin tweaking)
    pub world_gen_params: WorldGenParams,
    /// The stations working on their own (E4.3), run by their own task
    pub works: Arc<super::automation_tick::Works>,
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
            }),
            crafting_operations: HashMap::new(),
            last_positions: None,
            command_queue: Vec::new(),
            event_sender,
            tick_count: 0,
            pending_items: Vec::new(),
            gather_tools: HashMap::new(),
            finished_crafts: Vec::new(),
            stations: HashMap::new(),
            station_types: HashMap::new(),
            trade_manager: TradeManager::new(),
            terrain,
            terrain_version,
            ground,
            world_gen_params: params,
            works: Arc::default(),
            last_tps: 0.0,
            uptime_seconds: 0.0,
        }
    }

    /// Rebuild `/catalog`'s answer from the static tables. Call it after
    /// loading or changing them.
    pub fn refresh_catalog(&mut self) {
        self.catalog = Arc::new(super::catalog::build(self));
    }

    /// Replace the terrain, and its version with it
    pub fn set_terrain(&mut self, terrain: TerrainConfig) {
        self.terrain_version = terrain.version();
        self.ground = Arc::new(terrain.ground());
        self.terrain = terrain;
    }

    pub fn take_pending_items(&mut self) -> Vec<PendingItem> {
        std::mem::take(&mut self.pending_items)
    }

    pub fn take_finished_crafts(&mut self) -> Vec<FinishedCraft> {
        std::mem::take(&mut self.finished_crafts)
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
        if !self.players.contains_key(&player_id) {
            self.add_player(player);
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
        super::interest::Positions::of(self.players.values(), &self.recipes)
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

        for command in commands {
            match command {
                Command::Move { player_id, dest_x, dest_y } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        // Cancel any current action if moving
                        match &player.action_state {
                            ActionState::Extracting { .. } => {
                                cancel_extraction(player, &mut self.resource_nodes, &self.event_sender);
                            }
                            ActionState::Crafting { .. } => {
                                cancel_crafting(player, &mut self.crafting_operations, &self.event_sender);
                            }
                            ActionState::Idle => {}
                        }
                        player.dest_x = Some(dest_x);
                        player.dest_y = Some(dest_y);
                    }
                }
                Command::StartExtraction { player_id, node_id, tool } => {
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
                    if let Err(reason) =
                        start_extraction(player, node, resource_type, tool, &mut self.gather_tools, &self.event_sender)
                    {
                        self.extraction_failed(player_id, reason);
                    }
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
                Command::CraftAtStation { player_id, station_id, recipe_id, inputs, tool } => {
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
                                Ok(_) => {}
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
            let result = process_movement(player, &self.terrain, &self.ground);

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

            // Broadcast movement blocked if player was stopped by water
            if let Some(blocked) = result.blocked {
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

    fn tick(&mut self) {
        self.tick_count += 1;
        self.process_commands();
        self.process_movement();
        self.process_extractions();
        self.process_crafting();
        self.process_regeneration();
        if self.tick_count.is_multiple_of(TRADE_EXPIRY_TICKS) {
            self.expire_trade_requests(Instant::now());
        }
        self.broadcast_positions();
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
        );
        self.pending_items.extend(new_items);
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

    let mut interval = tokio::time::interval(period);
    let mut last_log = Instant::now();
    let mut tick_count = 0u64;
    let loop_start = Instant::now();

    loop {
        interval.tick().await;
        // The tick's work, from here to the loop's end (C1.7)
        let began = Instant::now();

        let pending_items: Vec<PendingItem>;
        let finished_crafts: Vec<FinishedCraft>;
        let event_sender;
        {
            let mut game = game_state.write().await;
            game.tick();
            pending_items = game.take_pending_items();
            finished_crafts = game.take_finished_crafts();
            event_sender = game.event_sender.clone();
        }

        // Write the gathers, wearing their tools, and only then tell the gatherers
        for item in pending_items {
            persist_gather(&app_state.db, item, &event_sender).await;
        }

        // Finish crafts: inputs re-checked, consumed and the output written in one
        // transaction (B10, #61). Only then is the crafter told, with one
        // `inventory_updated` per container that changed.
        for craft in finished_crafts {
            match finish_craft(&app_state.db, &craft).await {
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
                        tools::announce_broken(&event_sender, broken);
                    }
                    if let Some((station_id, tool)) = done.station_tool {
                        super::station_tools::announce(&event_sender, station_id, tool);
                    }
                    for container_id in done.changed {
                        let _ = event_sender.send(GameEvent::InventoryUpdated {
                            player_id: craft.player_id,
                            container_id,
                            slots: Vec::new(),
                        });
                    }
                }
                Err(reason) => {
                    let _ = event_sender.send(GameEvent::CraftingFailed {
                        player_id: craft.player_id,
                        reason,
                    });
                }
            }
        }

        tick_count += 1;

        // Log tick rate every 10 seconds
        if last_log.elapsed() >= Duration::from_secs(10) {
            let tps = tick_count as f64 / last_log.elapsed().as_secs_f64();
            tracing::debug!("Tick rate: {:.1} ticks/sec", tps);
            {
                let mut game = game_state.write().await;
                game.last_tps = tps;
                game.uptime_seconds = loop_start.elapsed().as_secs_f64();
            }
            last_log = Instant::now();
            tick_count = 0;
        }

        // Periodically persist player positions (every 10 seconds)
        if tick_count.is_multiple_of(100) {
            persist_positions(&game_state, &app_state).await;
        }

        // M7: Process property taxes hourly (36000 ticks = 1 hour at 10 ticks/sec)
        // In practice, 10 ticks/sec * 60 sec * 60 min = 36000 ticks per hour
        let game_tick_count = game_state.read().await.tick_count;
        if game_tick_count > 0 && game_tick_count % 36000 == 0 {
            tracing::info!("Processing hourly property taxes at tick {}", game_tick_count);
            let report = super::property_tax::process_property_taxes(&app_state.db).await;
            let mut game = game_state.write().await;
            let game = &mut *game;
            super::property_tax::announce(report, &mut game.stations, &game.event_sender);
        }

        // E4.1: subscriptions that ran out, hourly with the tax but half an
        // hour apart from it. The Subscriber extractor and a key's sign-in
        // refuse them on time anyway; this marks them lapsed, tells their
        // players and hangs up their bots (#217). The keys aren't revoked:
        // they sign in again once the player renews.
        if game_tick_count > 0 && game_tick_count % 36000 == 18000 {
            let lapses = crate::subscriptions::lapse_expired(&app_state.db).await;
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
        }

        super::load::tick_took(began.elapsed());
    }
}

async fn persist_positions(game_state: &Arc<RwLock<GameState>>, app_state: &AppState) {
    let game = game_state.read().await;

    for player in game.players.values() {
        let result = sqlx::query(
            "UPDATE players SET position_x = $1, position_y = $2, destination_x = $3, destination_y = $4 WHERE id = $5"
        )
        .bind(player.x)
        .bind(player.y)
        .bind(player.dest_x)
        .bind(player.dest_y)
        .bind(player.id)
        .execute(&app_state.db)
        .await;

        if let Err(e) = result {
            tracing::error!("Failed to persist player position: {:?}", e);
        }
    }
}

/// Write one finished gather, then tell the gatherer: `extraction_completed` with
/// what the tool has left, `inventory_updated`, and `tool_broke` if that was its
/// last use. A failed pull (quantity 0, D5) writes only the tool's wear.
///
/// The tool wears in the same transaction that writes the item. If it left the
/// player's inventory while they gathered, the gather is what bare hands would
/// have given, or nothing for a resource that needs the tool.
async fn persist_gather(db: &sqlx::PgPool, mut item: PendingItem, events: &broadcast::Sender<GameEvent>) {
    let mut tx = match db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to start saving a gather: {:?}", e);
            return;
        }
    };
    let (mut used, mut broken) = (None, None);
    // What it made and wore out, counted once it's written (C1.7)
    let mut flows = ledger::Flows::new();
    if let Some(gather) = item.tool.take() {
        match tools::wear_held(&mut tx, gather.held.item_id, item.player_id).await {
            Ok(Some(wear)) => {
                flows.wore(&gather.held.item_type, &wear);
                used = Some(gather.held.after(&wear));
                if let Wear::Broke(b) = wear {
                    broken = Some(b);
                }
            }
            Ok(None) => match gather.by_hand {
                Some((quantity, quality)) => (item.quantity, item.quality) = (quantity, quality),
                None => {
                    let _ = events.send(GameEvent::ExtractionFailed {
                        player_id: item.player_id,
                        reason: format!(
                            "Your {} left your inventory before the gather finished",
                            gather.held.item_type
                        ),
                    });
                    return;
                }
            },
            Err(e) => {
                tracing::error!("Failed to wear tool {}: {:?}", gather.held.item_id, e);
                return;
            }
        }
    }
    // The inventory that changed: the one the units went into, or after a
    // failed pull (D5) the one whose tool wore. A failed pull by hand changed none.
    let container_id = if item.quantity == 0 && used.is_none() {
        None
    } else if item.quantity == 0 {
        match inventory_of(&mut tx, item.player_id).await {
            Ok(found) => found.map(|(container_id, _)| container_id),
            Err(e) => {
                tracing::error!("Failed to find {}'s inventory: {:?}", item.player_id, e);
                return;
            }
        }
    } else {
        let container_id = match place_gathered(&mut tx, &item).await {
            Ok(Some(container_id)) => container_id,
            Ok(None) => return,
            Err(e) => {
                tracing::error!("Failed to persist {} for {}: {:?}", item.item_type, item.player_id, e);
                return;
            }
        };
        if let Err(e) = super::gazette::tally(&mut tx, item.player_id, &item.item_type, item.quantity).await {
            tracing::error!("Failed to tally a gather for the Gazette: {:?}", e);
            return;
        }
        flows.add(&item.item_type, UnitFlow::Gathered, item.quantity as i64);
        Some(container_id)
    };
    if let Err(e) = ledger::commit(tx, flows).await {
        tracing::error!("Failed to save a gather: {:?}", e);
        return;
    }

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
            player_id: item.player_id,
            container_id,
            slots: Vec::new(), // Client will refresh all slots
        });
    }
    if let Some(broken) = &broken {
        tools::announce_broken(events, broken);
    }
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
/// type, whatever its grade (`units::join_first_stack`: every unit keeps its
/// quality), or into the first free slot. Its units carry the gatherer's mark
/// (#231). Returns the
/// container, or `None` if there's no room (the item is lost, as before).
async fn place_gathered(conn: &mut sqlx::PgConnection, item: &PendingItem) -> Result<Option<Uuid>, sqlx::Error> {
    let Some((container_id, slot_count)) = inventory_of(conn, item.player_id).await? else {
        tracing::error!("No player_inventory container for player {}", item.player_id);
        return Ok(None);
    };

    let gathered = units::Marked::by(Some(item.player_id), &Units::uniform(item.quantity, item.quality));
    if units::join_first_stack(conn, container_id, &item.item_type, &gathered).await? {
        tracing::info!(
            "Merged {} x{} (quality {}) into existing stack for player {}",
            item.item_type, item.quantity, item.quality, item.player_id
        );
        return Ok(Some(container_id));
    }

    // Find first empty slot
    let used: Vec<(i32,)> = sqlx::query_as(
        "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL"
    )
    .bind(container_id)
    .fetch_all(&mut *conn)
    .await?;
    let Some(slot_index) = (0..slot_count).find(|s| !used.iter().any(|(u,)| u == s)) else {
        tracing::warn!("Inventory full for player {}, item lost!", item.player_id);
        return Ok(None);
    };

    let stack = units::NewStack {
        item_type: &item.item_type,
        owner_id: Some(item.player_id),
        container_id: Some(container_id),
        slot_index: Some(slot_index),
        durability: None,
    };
    units::insert_stack(conn, stack, &gathered).await?;
    tracing::info!(
        "Created {} x{} (quality {}) in slot {} for player {}",
        item.item_type, item.quantity, item.quality, slot_index, item.player_id
    );
    Ok(Some(container_id))
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::extraction::EXTRACTION_RANGE;
    use crate::models::{ActionState, NodeState};
    use capitol_protocol::rules::{CENTER_X, CENTER_Y};
    use capitol_protocol::ws::PresenceHolder;

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
        let command = Command::StartExtraction { player_id: a, node_id: Uuid::new_v4(), tool: None };
        let reason = extraction_refused(&mut game, &mut rx, command, a);
        assert!(reason.contains("gone"), "{reason}");
    }

    #[test]
    fn extract_a_node_of_unknown_type_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.resource_nodes.get_mut(&node_id).unwrap().resource_type = "moonflower".into();
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None }, a);
        assert!(reason.contains("moonflower"), "{reason}");
    }

    #[test]
    fn extract_while_busy_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.players.get_mut(&a).unwrap().action_state =
            ActionState::Extracting { node_id: Uuid::new_v4(), progress: 0, duration: 30 };
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None }, a);
        assert!(reason.contains("Already performing an action"), "{reason}");
    }

    #[test]
    fn extract_a_node_someone_else_is_gathering_tells_the_second_player() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.queue_command(Command::StartExtraction { player_id: a, node_id, tool: None });
        game.process_commands();
        while rx.try_recv().is_ok() {}

        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: b, node_id, tool: None }, b);
        assert!(reason.contains("Someone else is already gathering this Wild Grass"), "{reason}");
    }

    #[test]
    fn extract_a_depleted_node_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Depleted);
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None }, a);
        assert!(reason.contains("grow back"), "{reason}");
    }

    #[test]
    fn extract_out_of_range_says_how_far() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, EXTRACTION_RANGE + 30.0, NodeState::Available);
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None }, a);
        assert!(reason.contains("Too far"), "{reason}");
        assert!(reason.contains(&format!("{:.0} units", EXTRACTION_RANGE + 30.0)), "{reason}");
    }

    #[test]
    fn extract_without_the_tool_names_it() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.resource_types.get_mut("grass").unwrap().required_tool = Some("sickle".into());
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id, tool: None }, a);
        assert!(reason.contains("sickle"), "{reason}");
    }

    #[test]
    fn an_extraction_in_range_sends_no_failure() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.queue_command(Command::StartExtraction { player_id: a, node_id, tool: None });
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
