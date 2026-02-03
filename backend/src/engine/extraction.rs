use std::collections::HashMap;
use rand::Rng;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::{ActionState, NodeState, PlayerState, ResourceNodeState, ResourceType};
use super::events::GameEvent;

/// Maximum distance from player to node for extraction (world units)
pub const EXTRACTION_RANGE: f64 = 50.0;

/// Represents an item to be created after extraction/crafting
#[derive(Debug, Clone)]
pub struct PendingItem {
    pub player_id: Uuid,
    pub item_type: String,
    pub quantity: i32,
    pub quality: i32,
    /// Target container type (e.g., "crafting_output"). If None, goes to player_inventory.
    pub target_container_type: Option<String>,
}

/// Process extraction for all players actively extracting
/// Returns a list of items to be persisted to the database
pub fn process_extractions(
    players: &mut HashMap<Uuid, PlayerState>,
    nodes: &mut HashMap<Uuid, ResourceNodeState>,
    resource_types: &HashMap<String, ResourceType>,
    tick_count: u64,
    event_sender: &broadcast::Sender<GameEvent>,
) -> Vec<PendingItem> {
    let mut completed_extractions: Vec<(Uuid, Uuid)> = Vec::new();
    let mut progress_updates: Vec<(Uuid, u32, u32)> = Vec::new();
    let mut pending_items: Vec<PendingItem> = Vec::new();

    // Process each player's extraction progress
    for player in players.values_mut() {
        if let ActionState::Extracting { node_id, progress, duration } = &mut player.action_state {
            *progress += 1;

            // Send progress update every 5 ticks
            if *progress % 5 == 0 {
                progress_updates.push((player.id, *progress as u32, *duration as u32));
            }

            if *progress >= *duration {
                completed_extractions.push((player.id, *node_id));
            }
        }
    }

    // Send progress updates
    for (player_id, progress, duration) in progress_updates {
        let _ = event_sender.send(GameEvent::ExtractionProgress {
            player_id,
            progress,
            duration,
        });
    }

    // Handle completed extractions
    for (player_id, node_id) in completed_extractions {
        if let Some(node) = nodes.get(&node_id) {
            if let Some(resource_type) = resource_types.get(&node.resource_type) {
                // Calculate yield
                let mut rng = rand::thread_rng();
                let quantity = rng.gen_range(resource_type.yield_quantity_min..=resource_type.yield_quantity_max);

                // Calculate quality: node base quality +/- 5
                let quality_variance = rng.gen_range(-5..=5);
                let quality = (node.base_quality + quality_variance).clamp(0, 100);

                // Add to pending items for database persistence
                pending_items.push(PendingItem {
                    player_id,
                    item_type: resource_type.yields_item.clone(),
                    quantity,
                    quality,
                    target_container_type: None, // Goes to player inventory
                });

                // Emit completion event
                let _ = event_sender.send(GameEvent::ExtractionCompleted {
                    player_id,
                    node_id,
                    item_type: resource_type.yields_item.clone(),
                    item_name: get_item_name(&resource_type.yields_item),
                    quantity,
                    quality,
                });

                // Mark node as depleted
                if let Some(node) = nodes.get_mut(&node_id) {
                    node.state = NodeState::Depleted;
                    node.harvested_by = Some(player_id);
                    node.regenerates_at_tick = Some(tick_count + resource_type.regeneration_ticks as u64);
                }

                // Emit node depleted event
                let _ = event_sender.send(GameEvent::NodeDepleted { node_id });
            }
        }

        // Reset player action state
        if let Some(player) = players.get_mut(&player_id) {
            player.action_state = ActionState::Idle;
        }
    }

    pending_items
}

/// Start extraction for a player on a node
pub fn start_extraction(
    player: &mut PlayerState,
    node: &mut ResourceNodeState,
    resource_type: &ResourceType,
    event_sender: &broadcast::Sender<GameEvent>,
) -> Result<(), String> {
    // Check if player is already extracting
    if !matches!(player.action_state, ActionState::Idle) {
        return Err("Already performing an action".to_string());
    }

    // Check if node is available
    if node.state != NodeState::Available {
        return Err("Resource is not available".to_string());
    }

    // Check range
    let dx = player.x - node.x;
    let dy = player.y - node.y;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance > EXTRACTION_RANGE {
        return Err("Too far from resource".to_string());
    }

    // Check tool requirement
    if resource_type.required_tool.is_some() {
        // For now, we only have hand-gatherable resources
        return Err("Required tool not equipped".to_string());
    }

    // Stop any movement
    player.dest_x = None;
    player.dest_y = None;

    // Start extraction
    let duration = resource_type.extraction_duration_ticks;
    player.action_state = ActionState::Extracting {
        node_id: node.id,
        progress: 0,
        duration,
    };

    // Mark node as being harvested
    node.state = NodeState::BeingHarvested;

    // Emit event
    let _ = event_sender.send(GameEvent::ExtractionStarted {
        player_id: player.id,
        node_id: node.id,
        duration_ticks: duration as u32,
    });

    Ok(())
}

/// Cancel extraction for a player
pub fn cancel_extraction(
    player: &mut PlayerState,
    nodes: &mut HashMap<Uuid, ResourceNodeState>,
    event_sender: &broadcast::Sender<GameEvent>,
) {
    if let ActionState::Extracting { node_id, .. } = &player.action_state {
        // Restore node to available state
        if let Some(node) = nodes.get_mut(node_id) {
            node.state = NodeState::Available;
        }

        // Emit cancellation event
        let _ = event_sender.send(GameEvent::ExtractionCancelled {
            player_id: player.id,
        });
    }

    player.action_state = ActionState::Idle;
}

fn get_item_name(item_type: &str) -> String {
    match item_type {
        "fiber" => "Plant Fiber".to_string(),
        _ => item_type.to_string(),
    }
}
