use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::commands::Command;
use super::crafting::{cancel_crafting, process_crafting, start_crafting, CraftingOperation};
use super::events::GameEvent;
use super::extraction::{cancel_extraction, process_extractions, start_extraction, PendingItem};
use super::movement::process_movement;
use super::regeneration::process_regeneration;
use crate::models::{ActionState, Item, ItemType, PlayerPosition, PlayerState, Recipe, ResourceNodeState, ResourceType, StationState, StationType};
use crate::state::AppState;

const TICK_RATE_MS: u64 = 100;

pub struct GameState {
    pub players: HashMap<Uuid, PlayerState>,
    pub resource_nodes: HashMap<Uuid, ResourceNodeState>,
    pub resource_types: HashMap<String, ResourceType>,
    pub recipes: HashMap<String, Recipe>,
    pub item_types: HashMap<String, ItemType>,
    pub crafting_operations: HashMap<Uuid, CraftingOperation>,
    pub pending_craft_items: HashMap<Uuid, Vec<Item>>, // player_id -> validated items for crafting
    pub command_queue: Vec<Command>,
    pub event_sender: broadcast::Sender<GameEvent>,
    pub tick_count: u64,
    pub pending_items: Vec<PendingItem>,
    /// Items to consume: (item_id, quantity_to_consume)
    pub items_to_consume: Vec<(Uuid, i32)>,
    // M4: Stations
    pub stations: HashMap<Uuid, StationState>,
    pub station_types: HashMap<String, StationType>,
}

impl GameState {
    pub fn new() -> Self {
        let (event_sender, _) = broadcast::channel(1024);
        Self {
            players: HashMap::new(),
            resource_nodes: HashMap::new(),
            resource_types: HashMap::new(),
            recipes: HashMap::new(),
            item_types: HashMap::new(),
            crafting_operations: HashMap::new(),
            pending_craft_items: HashMap::new(),
            command_queue: Vec::new(),
            event_sender,
            tick_count: 0,
            pending_items: Vec::new(),
            items_to_consume: Vec::new(),
            stations: HashMap::new(),
            station_types: HashMap::new(),
        }
    }

    pub fn take_pending_items(&mut self) -> Vec<PendingItem> {
        std::mem::take(&mut self.pending_items)
    }

    pub fn take_items_to_consume(&mut self) -> Vec<(Uuid, i32)> {
        std::mem::take(&mut self.items_to_consume)
    }

    pub fn get_tick_count(&self) -> u64 {
        self.tick_count
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

    pub fn remove_player(&mut self, player_id: Uuid) {
        self.players.remove(&player_id);
        let _ = self.event_sender.send(GameEvent::PlayerLeft { player_id });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GameEvent> {
        self.event_sender.subscribe()
    }

    fn process_commands(&mut self) {
        let commands: Vec<Command> = self.command_queue.drain(..).collect();

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
                Command::StopMoving { player_id } => {
                    if let Some(player) = self.players.get_mut(&player_id) {
                        player.dest_x = None;
                        player.dest_y = None;
                    }
                }
                Command::StartExtraction { player_id, node_id } => {
                    // Get resource type first
                    let resource_type = self.resource_nodes.get(&node_id)
                        .and_then(|node| self.resource_types.get(&node.resource_type))
                        .cloned();

                    if let Some(resource_type) = resource_type {
                        if let (Some(player), Some(node)) = (
                            self.players.get_mut(&player_id),
                            self.resource_nodes.get_mut(&node_id),
                        ) {
                            if let Err(e) = start_extraction(player, node, &resource_type, &self.event_sender) {
                                tracing::debug!("Extraction failed: {}", e);
                            }
                        }
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
                // M3.5: Inventory commands - processed via database, not in-memory
                Command::MoveItem { .. } => {
                    // MoveItem is handled directly in WebSocket handler via database
                    // This command exists for future in-memory container state if needed
                }
                // M4: Station commands - most are handled via WebSocket/database
                Command::PlaceStation { .. } => {
                    // Handled in WebSocket handler - requires database operations
                }
                Command::RemoveStation { .. } => {
                    // Handled in WebSocket handler - requires database operations
                }
                Command::OpenStation { .. } => {
                    // Handled in WebSocket handler
                }
                Command::CloseStation { .. } => {
                    // Handled in WebSocket handler
                }
                Command::CraftAtStation { player_id, station_id, recipe_id, input_item_ids: _ } => {
                    // Validate station access and start crafting
                    let station_valid = self.stations.get(&station_id)
                        .map(|s| {
                            let player = self.players.get(&player_id);
                            player.map_or(false, |p| {
                                super::stations::is_player_near_station_type(p, s, &self.station_types)
                                    && super::stations::can_player_access_station(p, s)
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
            }
        }
    }

    fn process_movement(&mut self) {
        for player in self.players.values_mut() {
            process_movement(player);
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
        self.broadcast_positions();
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
        let (new_items, consumed_items) = process_crafting(
            &mut self.players,
            &self.recipes,
            &mut self.crafting_operations,
            &self.item_types,
            &self.event_sender,
        );
        self.pending_items.extend(new_items);
        self.items_to_consume.extend(consumed_items);
    }
}

pub async fn run_tick_loop(game_state: Arc<RwLock<GameState>>, app_state: AppState) {
    tracing::info!("Starting tick engine at {}ms interval", TICK_RATE_MS);

    let mut interval = tokio::time::interval(Duration::from_millis(TICK_RATE_MS));
    let mut last_log = Instant::now();
    let mut tick_count = 0u64;

    loop {
        interval.tick().await;

        let pending_items: Vec<PendingItem>;
        let items_to_consume: Vec<(Uuid, i32)>;
        let event_sender;
        {
            let mut game = game_state.write().await;
            game.tick();
            pending_items = game.take_pending_items();
            items_to_consume = game.take_items_to_consume();
            event_sender = game.event_sender.clone();
        }

        // Persist any items created from extractions/crafting
        // and notify clients after database writes complete
        if !pending_items.is_empty() {
            let affected_players = persist_items(&app_state, pending_items).await;
            // Send inventory updated events AFTER persistence completes
            for (player_id, container_id) in affected_players {
                let _ = event_sender.send(GameEvent::InventoryUpdated {
                    player_id,
                    container_id,
                    slots: Vec::new(), // Client will refresh all slots
                });
            }
        }

        // Delete consumed items (crafting inputs)
        if !items_to_consume.is_empty() {
            let affected = consume_items(&app_state, items_to_consume).await;
            // Send inventory updated events for consumed items
            for (player_id, container_id) in affected {
                let _ = event_sender.send(GameEvent::InventoryUpdated {
                    player_id,
                    container_id,
                    slots: Vec::new(),
                });
            }
        }

        tick_count += 1;

        // Log tick rate every 10 seconds
        if last_log.elapsed() >= Duration::from_secs(10) {
            let tps = tick_count as f64 / last_log.elapsed().as_secs_f64();
            tracing::debug!("Tick rate: {:.1} ticks/sec", tps);
            last_log = Instant::now();
            tick_count = 0;
        }

        // Periodically persist player positions
        if tick_count % 100 == 0 {
            persist_positions(&game_state, &app_state).await;
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

async fn persist_items(app_state: &AppState, items: Vec<PendingItem>) -> Vec<(Uuid, Uuid)> {
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
        .fetch_optional(&app_state.db)
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
        .fetch_optional(&app_state.db)
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
            .execute(&app_state.db)
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
            .fetch_all(&app_state.db)
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
            .execute(&app_state.db)
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

async fn consume_items(app_state: &AppState, items: Vec<(Uuid, i32)>) -> Vec<(Uuid, Uuid)> {
    let mut affected: Vec<(Uuid, Uuid)> = Vec::new();

    for (item_id, quantity_to_consume) in items {
        // Get item info
        let item_info: Option<(Uuid, Option<Uuid>, i32)> = sqlx::query_as(
            "SELECT owner_id, container_id, quantity FROM items WHERE id = $1"
        )
        .bind(item_id)
        .fetch_optional(&app_state.db)
        .await
        .ok()
        .flatten();

        let Some((owner_id, container_id, current_quantity)) = item_info else {
            tracing::error!("Item {} not found for consumption", item_id);
            continue;
        };

        let new_quantity = current_quantity - quantity_to_consume;

        let result = if new_quantity <= 0 {
            // Delete the item entirely
            sqlx::query("DELETE FROM items WHERE id = $1")
                .bind(item_id)
                .execute(&app_state.db)
                .await
        } else {
            // Just decrement the quantity
            sqlx::query("UPDATE items SET quantity = $1 WHERE id = $2")
                .bind(new_quantity)
                .bind(item_id)
                .execute(&app_state.db)
                .await
        };

        if let Err(e) = result {
            tracing::error!("Failed to consume item {}: {:?}", item_id, e);
        } else {
            tracing::debug!(
                "Consumed {} of item {} (had {}, now {})",
                quantity_to_consume,
                item_id,
                current_quantity,
                new_quantity.max(0)
            );
            if let Some(cid) = container_id {
                affected.push((owner_id, cid));
            }
        }
    }

    affected
}
