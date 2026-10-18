use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, oneshot, RwLock};
use uuid::Uuid;

use super::commands::Command;
use super::craft_completion::{finish_craft, FinishedCraft};
use super::crafting::{cancel_crafting, process_crafting, start_crafting, CraftingOperation};
use super::events::GameEvent;
use super::extraction::{cancel_extraction, process_extractions, start_extraction, PendingItem};
use super::movement::process_movement;
use super::regeneration::process_regeneration;
use super::trading::{TradeManager, TRADE_RANGE};
use crate::models::{ActionState, ContainerType, Item, ItemType, PlayerPosition, PlayerState, Recipe, ResourceNodeState, ResourceType, StationState, StationType, TerrainConfig, WorldGenParams};
use crate::state::AppState;

use capitol_protocol::rest::CatalogResponse;
use capitol_protocol::rules::TICK_MS as TICK_RATE_MS;

/// Ticks between sweeps for unanswered trade requests (once a second)
const TRADE_EXPIRY_TICKS: u64 = 10;

/// The WebSocket connection currently serving a player
pub struct PlayerConnection {
    pub id: u64,
    /// Fired when a newer connection takes the player over, or their session ends (#78).
    /// `None` once fired
    superseded: Option<oneshot::Sender<()>>,
}

pub struct GameState {
    pub players: HashMap<Uuid, PlayerState>,
    /// player_id -> the connection serving them. Only that connection may remove them (B8, #59)
    pub connections: HashMap<Uuid, PlayerConnection>,
    next_connection_id: u64,
    pub resource_nodes: HashMap<Uuid, ResourceNodeState>,
    pub resource_types: HashMap<String, ResourceType>,
    pub recipes: HashMap<String, Recipe>,
    pub item_types: HashMap<String, ItemType>,
    pub container_types: HashMap<String, ContainerType>,
    /// `/catalog`'s answer for the static tables above, built by
    /// [`GameState::refresh_catalog`] once they're loaded
    pub catalog: Arc<CatalogResponse>,
    pub crafting_operations: HashMap<Uuid, CraftingOperation>,
    pub pending_craft_items: HashMap<Uuid, Vec<Item>>, // player_id -> validated items for crafting
    pub command_queue: Vec<Command>,
    pub event_sender: broadcast::Sender<GameEvent>,
    pub tick_count: u64,
    pub pending_items: Vec<PendingItem>,
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
    // World generation parameters (for admin tweaking)
    pub world_gen_params: WorldGenParams,
    // Performance tracking
    pub last_tps: f64,
    pub uptime_seconds: f64,
}

impl GameState {
    pub fn with_params(params: WorldGenParams) -> Self {
        let terrain = TerrainConfig::from_params(&params);
        Self::with_terrain(params, terrain)
    }

    /// A game on terrain already generated from `params`
    pub(crate) fn with_terrain(params: WorldGenParams, terrain: TerrainConfig) -> Self {
        let (event_sender, _) = broadcast::channel(1024);
        let terrain_version = terrain.version();
        Self {
            players: HashMap::new(),
            connections: HashMap::new(),
            next_connection_id: 0,
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
            pending_craft_items: HashMap::new(),
            command_queue: Vec::new(),
            event_sender,
            tick_count: 0,
            pending_items: Vec::new(),
            finished_crafts: Vec::new(),
            stations: HashMap::new(),
            station_types: HashMap::new(),
            trade_manager: TradeManager::new(),
            terrain,
            terrain_version,
            world_gen_params: params,
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
        };
        self.players.insert(player.id, player);
        let _ = self.event_sender.send(GameEvent::PlayerJoined(position));
    }

    /// Add `player` for a new connection. Any older connection serving the same player is
    /// superseded: its receiver fires, and it can no longer remove the player. Returns this
    /// connection's id and the receiver that fires when a newer connection supersedes it.
    pub fn connect_player(&mut self, player: PlayerState) -> (u64, oneshot::Receiver<()>) {
        self.next_connection_id += 1;
        let id = self.next_connection_id;
        let (superseded, rx) = oneshot::channel();
        let current = PlayerConnection { id, superseded: Some(superseded) };
        if let Some(old) = self.connections.insert(player.id, current) {
            tracing::info!("Connection {} supersedes {} for player {}", id, old.id, player.id);
            if let Some(tx) = old.superseded {
                let _ = tx.send(());
            }
        }
        self.add_player(player);
        (id, rx)
    }

    /// The player's session ended over REST: a login elsewhere replaced it, or it logged out.
    /// Their connection's receiver fires, so it says `auth_error` and hangs up (#78). It stays
    /// their current connection, so its own disconnect still removes the player. Returns
    /// whether there was a connection to tell.
    pub fn end_session(&mut self, player_id: Uuid) -> bool {
        match self.connections.get_mut(&player_id).and_then(|c| c.superseded.take()) {
            Some(tx) => {
                tracing::info!("Session ended for player {}: closing their connection", player_id);
                let _ = tx.send(());
                true
            }
            None => false,
        }
    }

    /// End connection `connection_id`. The player is removed only if it is still their current
    /// connection; a stale one (a half-open socket, or a client signed in elsewhere) leaves the
    /// newer connection's player alone. Returns whether the player was removed.
    pub fn disconnect_player(&mut self, player_id: Uuid, connection_id: u64) -> bool {
        match self.connections.get(&player_id) {
            Some(current) if current.id == connection_id => {
                self.connections.remove(&player_id);
                self.remove_player(player_id);
                true
            }
            _ => false,
        }
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
        // Remove any pending trade requests
        self.trade_manager.pending_requests.remove(&player_id);
        self.trade_manager.pending_requests.retain(|_, (initiator, _)| *initiator != player_id);

        self.players.remove(&player_id);
        let _ = self.event_sender.send(GameEvent::PlayerLeft { player_id });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GameEvent> {
        self.event_sender.subscribe()
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
                Command::StartExtraction { player_id, node_id } => {
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
                    if let Err(reason) = start_extraction(player, node, resource_type, &self.event_sender) {
                        self.extraction_failed(player_id, reason);
                    }
                }
                Command::CancelExtraction { player_id } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        cancel_extraction(player, &mut self.resource_nodes, &self.event_sender);
                    }
                }
                // M3: Crafting commands
                Command::StartCrafting { player_id, recipe_id, input_item_ids: _ } => {
                    // This command is only queued after items are validated
                    // The items are passed as pre-validated from the WebSocket handler
                    // We need to store input_items temporarily for the command
                    // Since we can't fetch from DB here, the WS handler must pre-validate
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
                            // Create placeholder items with qualities from pending_craft_items
                            if let Some(pending) = self.pending_craft_items.remove(&player_id) {
                                match start_crafting(
                                    player,
                                    &recipe,
                                    &pending,
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
                Command::CraftAtStation { player_id, station_id, recipe_id, input_item_ids: _ } => {
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
                        if let Some(player) = self.players.get_mut(&player_id) {
                            if let Some(pending) = self.pending_craft_items.remove(&player_id) {
                                match start_crafting(
                                    player,
                                    &recipe,
                                    &pending,
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
                    let initiator_name = self.players.get(&player_id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();

                    match self.trade_manager.request_trade(player_id, target_player_id) {
                        Ok(()) => {
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
                    let offer = items.iter().map(|item| (item.item_id, item.quantity)).collect();
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

        // Clone terrain reference to avoid borrow issues
        let terrain = self.terrain.clone();

        for player in self.players.values_mut() {
            let result = process_movement(player, &terrain);

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

    fn broadcast_positions(&self) {
        let positions: Vec<PlayerPosition> = self.players.values()
            .map(|p| PlayerPosition {
                id: p.id,
                name: p.name.clone(),
                x: p.x,
                y: p.y,
            })
            .collect();

        if !positions.is_empty() {
            let _ = self.event_sender.send(GameEvent::PlayerPositions(positions));
        }
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
    tracing::info!("Starting tick engine at {}ms interval", TICK_RATE_MS);

    let mut interval = tokio::time::interval(Duration::from_millis(TICK_RATE_MS));
    let mut last_log = Instant::now();
    let mut tick_count = 0u64;
    let loop_start = Instant::now();

    loop {
        interval.tick().await;

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

        // Persist any items created from extractions/crafting
        // and notify clients after database writes complete
        if !pending_items.is_empty() {
            let affected_players = persist_items(&app_state.db, pending_items).await;
            // Send inventory updated events AFTER persistence completes
            for (player_id, container_id) in affected_players {
                let _ = event_sender.send(GameEvent::InventoryUpdated {
                    player_id,
                    container_id,
                    slots: Vec::new(), // Client will refresh all slots
                });
            }
        }

        // Finish crafts: inputs re-checked, consumed and the output written in one
        // transaction (B10, #61). Only then is the crafter told, with one
        // `inventory_updated` per container that changed.
        for craft in finished_crafts {
            match finish_craft(&app_state.db, &craft).await {
                Ok(changed) => {
                    for output in &craft.outputs {
                        let _ = event_sender.send(GameEvent::CraftingCompleted {
                            player_id: craft.player_id,
                            operation_id: craft.operation_id,
                            item_type: output.item_type.clone(),
                            item_name: output.item_name.clone(),
                            quantity: output.quantity,
                            quality: output.quality,
                        });
                    }
                    for container_id in changed {
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

async fn persist_items(db: &sqlx::PgPool, items: Vec<PendingItem>) -> Vec<(Uuid, Uuid)> {
    let mut affected: Vec<(Uuid, Uuid)> = Vec::new();

    for item in items {
        // Determine target container type
        let container_type = item.target_container_type.as_deref().unwrap_or("player_inventory");

        // Get the appropriate container
        let container_result: Option<(Uuid, i32)> = sqlx::query_as(
            "SELECT c.id, ct.slot_count FROM containers c
             JOIN container_types ct ON c.container_type = ct.id
             WHERE c.owner_id = $1 AND c.container_type = $2
             LIMIT 1"
        )
        .bind(item.player_id)
        .bind(container_type)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();

        let (container_id, slot_count) = match container_result {
            Some((id, count)) => (id, count),
            None => {
                tracing::error!("No {} container for player {}", container_type, item.player_id);
                continue;
            }
        };

        // Check if there's an existing stack of same type to merge with
        let existing: Option<(Uuid, i32, i32, i32)> = sqlx::query_as(
            "SELECT id, quantity, quality, slot_index FROM items
             WHERE container_id = $1 AND item_type = $2
             LIMIT 1"
        )
        .bind(container_id)
        .bind(&item.item_type)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();

        if let Some((existing_id, existing_qty, existing_quality, _slot)) = existing {
            // Merge with existing stack (weighted average quality)
            let new_qty = existing_qty + item.quantity;
            let new_quality = (existing_quality * existing_qty + item.quality * item.quantity) / new_qty;

            let result = sqlx::query(
                "UPDATE items SET quantity = $1, quality = $2 WHERE id = $3"
            )
            .bind(new_qty)
            .bind(new_quality)
            .bind(existing_id)
            .execute(db)
            .await;

            if let Err(e) = result {
                tracing::error!("Failed to merge item: {:?}", e);
            } else {
                tracing::info!(
                    "Merged {} x{} (quality {}) into existing stack for player {}",
                    item.item_type,
                    item.quantity,
                    item.quality,
                    item.player_id
                );
                affected.push((item.player_id, container_id));
            }
        } else {
            // Find first empty slot
            let used_slots: Vec<(i32,)> = sqlx::query_as(
                "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL"
            )
            .bind(container_id)
            .fetch_all(db)
            .await
            .unwrap_or_default();

            let used: std::collections::HashSet<i32> = used_slots.into_iter().map(|(s,)| s).collect();
            let mut empty_slot: Option<i32> = None;
            for slot in 0..slot_count {
                if !used.contains(&slot) {
                    empty_slot = Some(slot);
                    break;
                }
            }

            let slot_index = match empty_slot {
                Some(s) => s,
                None => {
                    tracing::warn!("Container {} full for player {}, item lost!", container_type, item.player_id);
                    continue;
                }
            };

            // Create new item in slot
            let item_id = Uuid::new_v4();
            let result = sqlx::query(
                "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)"
            )
            .bind(item_id)
            .bind(&item.item_type)
            .bind(item.quality)
            .bind(item.quantity)
            .bind(item.player_id)
            .bind(container_id)
            .bind(slot_index)
            .execute(db)
            .await;

            if let Err(e) = result {
                tracing::error!("Failed to persist item: {:?}", e);
            } else {
                tracing::info!(
                    "Created {} x{} (quality {}) in slot {} for player {}",
                    item.item_type,
                    item.quantity,
                    item.quality,
                    slot_index,
                    item.player_id
                );
                affected.push((item.player_id, container_id));
            }
        }
    }

    affected
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::extraction::EXTRACTION_RANGE;
    use crate::models::{ActionState, NodeState};

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
        let a = player("Ada", 4000.0, 4000.0);
        let b = player("Bram", 4000.0 + gap, 4000.0);
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
        let c = player("Cato", 4010.0, 4010.0);
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
        let c = player("Cato", 4010.0, 4010.0);
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

    #[test]
    fn a_stale_connection_ending_leaves_the_newer_ones_player() {
        let mut game = game();
        let ada = player("Ada", 4000.0, 4000.0);
        let id = ada.id;
        let (conn_a, mut superseded_a) = game.connect_player(ada.clone());
        let (conn_b, mut superseded_b) = game.connect_player(ada);
        assert_ne!(conn_a, conn_b);
        assert!(superseded_a.try_recv().is_ok(), "A should be told B took over");
        assert!(superseded_b.try_recv().is_err(), "B is current");

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
    fn an_ended_session_closes_the_connection_and_its_disconnect_removes_the_player() {
        let mut game = game();
        let ada = player("Ada", 4000.0, 4000.0);
        let id = ada.id;
        assert!(!game.end_session(id), "no connection to tell");
        let (conn, mut ended) = game.connect_player(ada);

        assert!(game.end_session(id));
        assert!(ended.try_recv().is_ok(), "the connection should be told to hang up");
        assert!(!game.end_session(id), "told once only");

        let mut rx = game.subscribe();
        assert!(game.disconnect_player(id, conn), "it's still the player's connection");
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
        assert_eq!(trade.offer_a.items, vec![(rope.item_id, 2)]);
    }

    #[test]
    fn a_stale_connection_leaves_the_players_trade_open() {
        let (mut game, a, b, _) = two_players(10.0);
        let ada = game.players[&a].clone();
        let (stale, _) = game.connect_player(ada.clone());
        let (_current, _) = game.connect_player(ada);
        game.trade_manager.request_trade(a, b).unwrap();
        game.trade_manager.accept_request(b).unwrap();

        game.disconnect_player(a, stale);
        assert!(game.trade_manager.is_in_trade(a));
        assert!(game.trade_manager.is_in_trade(b));
    }

    // B9: a refused extraction says why, to the requester only

    /// Put a Wild Grass node `dx` units east of `at` into `game`, and return its id
    fn grass_node(game: &mut GameState, at: (f64, f64), dx: f64, state: NodeState) -> Uuid {
        game.resource_types.entry("grass".into()).or_insert_with(|| ResourceType {
            id: "grass".into(),
            name: "Wild Grass".into(),
            yields_item: "fiber".into(),
            yield_quantity_min: 1,
            yield_quantity_max: 3,
            base_quality_min: 40,
            base_quality_max: 70,
            required_tool: None,
            extraction_duration_ticks: 30,
            regeneration_ticks: 600,
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

    const AT: (f64, f64) = (4000.0, 4000.0);

    #[test]
    fn extract_an_unknown_node_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let command = Command::StartExtraction { player_id: a, node_id: Uuid::new_v4() };
        let reason = extraction_refused(&mut game, &mut rx, command, a);
        assert!(reason.contains("gone"), "{reason}");
    }

    #[test]
    fn extract_a_node_of_unknown_type_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.resource_nodes.get_mut(&node_id).unwrap().resource_type = "moonflower".into();
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id }, a);
        assert!(reason.contains("moonflower"), "{reason}");
    }

    #[test]
    fn extract_while_busy_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.players.get_mut(&a).unwrap().action_state =
            ActionState::Extracting { node_id: Uuid::new_v4(), progress: 0, duration: 30 };
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id }, a);
        assert!(reason.contains("Already performing an action"), "{reason}");
    }

    #[test]
    fn extract_a_node_someone_else_is_gathering_tells_the_second_player() {
        let (mut game, a, b, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.queue_command(Command::StartExtraction { player_id: a, node_id });
        game.process_commands();
        while rx.try_recv().is_ok() {}

        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: b, node_id }, b);
        assert!(reason.contains("Someone else is already gathering this Wild Grass"), "{reason}");
    }

    #[test]
    fn extract_a_depleted_node_tells_the_player() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Depleted);
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id }, a);
        assert!(reason.contains("grow back"), "{reason}");
    }

    #[test]
    fn extract_out_of_range_says_how_far() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, EXTRACTION_RANGE + 30.0, NodeState::Available);
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id }, a);
        assert!(reason.contains("Too far"), "{reason}");
        assert!(reason.contains(&format!("{:.0} units", EXTRACTION_RANGE + 30.0)), "{reason}");
    }

    #[test]
    fn extract_without_the_tool_names_it() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.resource_types.get_mut("grass").unwrap().required_tool = Some("sickle".into());
        let reason = extraction_refused(&mut game, &mut rx, Command::StartExtraction { player_id: a, node_id }, a);
        assert!(reason.contains("sickle"), "{reason}");
    }

    #[test]
    fn an_extraction_in_range_sends_no_failure() {
        let (mut game, a, _, mut rx) = two_players(10.0);
        let node_id = grass_node(&mut game, AT, 5.0, NodeState::Available);
        game.queue_command(Command::StartExtraction { player_id: a, node_id });
        game.process_commands();
        let mut started = false;
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(event, GameEvent::ExtractionFailed { .. }), "unexpected {event:?}");
            started |= matches!(event, GameEvent::ExtractionStarted { player_id, .. } if player_id == a);
        }
        assert!(started);
    }
}
