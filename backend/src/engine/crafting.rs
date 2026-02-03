use std::collections::HashMap;
use rand::Rng;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::{ActionState, Item, ItemType, PlayerState, Recipe};
use super::events::GameEvent;
use super::extraction::PendingItem;

/// Active crafting operation tracked in game state
#[derive(Debug, Clone)]
pub struct CraftingOperation {
    pub operation_id: Uuid,
    pub player_id: Uuid,
    pub recipe_id: String,
    /// Items to consume: (item_id, quantity_to_consume)
    pub input_items_to_consume: Vec<(Uuid, i32)>,
    pub input_qualities: Vec<i32>,
}

/// Process crafting for all players actively crafting
/// Returns a list of items to be persisted and items to be consumed (item_id, quantity)
pub fn process_crafting(
    players: &mut HashMap<Uuid, PlayerState>,
    recipes: &HashMap<String, Recipe>,
    crafting_operations: &mut HashMap<Uuid, CraftingOperation>,
    item_types: &HashMap<String, ItemType>,
    event_sender: &broadcast::Sender<GameEvent>,
) -> (Vec<PendingItem>, Vec<(Uuid, i32)>) {
    let mut completed_operations: Vec<Uuid> = Vec::new();
    let mut progress_updates: Vec<(Uuid, Uuid, u32, u32)> = Vec::new();
    let mut pending_items: Vec<PendingItem> = Vec::new();
    let mut items_to_consume: Vec<(Uuid, i32)> = Vec::new();

    // Process each player's crafting progress
    for player in players.values_mut() {
        if let ActionState::Crafting { operation_id, recipe_id: _, progress, duration } = &mut player.action_state {
            *progress += 1;

            // Send progress update every 5 ticks
            if *progress % 5 == 0 {
                progress_updates.push((player.id, *operation_id, *progress as u32, *duration as u32));
            }

            if *progress >= *duration {
                completed_operations.push(*operation_id);
            }
        }
    }

    // Send progress updates
    for (player_id, operation_id, progress, duration) in progress_updates {
        let _ = event_sender.send(GameEvent::CraftingProgress {
            player_id,
            operation_id,
            progress,
            duration,
        });
    }

    // Handle completed crafting
    for operation_id in completed_operations {
        if let Some(operation) = crafting_operations.remove(&operation_id) {
            if let Some(recipe) = recipes.get(&operation.recipe_id) {
                // Calculate output quality from input qualities
                let output_quality = calculate_output_quality(&operation.input_qualities, recipe);

                // Generate output items
                let mut rng = rand::thread_rng();
                for output in &recipe.outputs {
                    let quantity = rng.gen_range(output.quantity_min..=output.quantity_max);

                    let item_name = item_types
                        .get(&output.item_type)
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| output.item_type.clone());

                    pending_items.push(PendingItem {
                        player_id: operation.player_id,
                        item_type: output.item_type.clone(),
                        quantity,
                        quality: output_quality,
                        target_container_type: Some("crafting_output".to_string()),
                    });

                    // Emit completion event
                    let _ = event_sender.send(GameEvent::CraftingCompleted {
                        player_id: operation.player_id,
                        operation_id: operation.operation_id,
                        item_type: output.item_type.clone(),
                        item_name,
                        quantity,
                        quality: output_quality,
                    });
                }

                // Mark input items for consumption (with quantities)
                items_to_consume.extend(operation.input_items_to_consume.clone());
            }

            // Reset player action state
            if let Some(player) = players.get_mut(&operation.player_id) {
                player.action_state = ActionState::Idle;
            }
        }
    }

    (pending_items, items_to_consume)
}

/// Calculate output quality based on input qualities and recipe weights
pub fn calculate_output_quality(input_qualities: &[i32], recipe: &Recipe) -> i32 {
    if input_qualities.is_empty() {
        return 50; // Default if no inputs
    }

    // Calculate weighted average of input qualities
    let avg_quality: f32 = input_qualities.iter().sum::<i32>() as f32 / input_qualities.len() as f32;

    // Apply variance
    let mut rng = rand::thread_rng();
    let variance = rng.gen_range(recipe.quality_weights.variance_min..=recipe.quality_weights.variance_max);

    // Clamp result to valid range
    (avg_quality as i32 + variance).clamp(0, 100)
}

/// Start crafting for a player
pub fn start_crafting(
    player: &mut PlayerState,
    recipe: &Recipe,
    input_items: &[Item],
    crafting_operations: &mut HashMap<Uuid, CraftingOperation>,
    event_sender: &broadcast::Sender<GameEvent>,
) -> Result<Uuid, String> {
    // Check if player is already doing something
    if !matches!(player.action_state, ActionState::Idle) {
        return Err("Already performing an action".to_string());
    }

    // Validate inputs match recipe requirements
    let mut remaining_requirements: HashMap<String, i32> = HashMap::new();
    for input in &recipe.inputs {
        remaining_requirements.insert(input.item_type.clone(), input.quantity);
    }

    let mut items_to_consume: Vec<(Uuid, i32)> = Vec::new();
    let mut input_qualities: Vec<i32> = Vec::new();

    for item in input_items {
        if let Some(required) = remaining_requirements.get_mut(&item.item_type) {
            if *required > 0 {
                let use_quantity = (*required).min(item.quantity);
                *required -= use_quantity;
                items_to_consume.push((item.id, use_quantity));
                // Add quality weighted by quantity used
                for _ in 0..use_quantity {
                    input_qualities.push(item.quality);
                }
            }
        }
    }

    // Check all requirements are met
    for (item_type, remaining) in &remaining_requirements {
        if *remaining > 0 {
            return Err(format!("Missing {} more {}", remaining, item_type));
        }
    }

    // Stop any movement
    player.dest_x = None;
    player.dest_y = None;

    // Create crafting operation
    let operation_id = Uuid::new_v4();
    let duration = recipe.base_duration_ticks;

    player.action_state = ActionState::Crafting {
        operation_id,
        recipe_id: recipe.id.clone(),
        progress: 0,
        duration,
    };

    crafting_operations.insert(operation_id, CraftingOperation {
        operation_id,
        player_id: player.id,
        recipe_id: recipe.id.clone(),
        input_items_to_consume: items_to_consume,
        input_qualities,
    });

    // Emit event
    let _ = event_sender.send(GameEvent::CraftingStarted {
        player_id: player.id,
        operation_id,
        recipe_id: recipe.id.clone(),
        recipe_name: recipe.name.clone(),
        duration_ticks: duration as u32,
    });

    Ok(operation_id)
}

/// Cancel crafting for a player
pub fn cancel_crafting(
    player: &mut PlayerState,
    crafting_operations: &mut HashMap<Uuid, CraftingOperation>,
    event_sender: &broadcast::Sender<GameEvent>,
) {
    if let ActionState::Crafting { operation_id, .. } = &player.action_state {
        let op_id = *operation_id;

        // Remove the operation (items won't be consumed)
        crafting_operations.remove(&op_id);

        // Emit cancellation event
        let _ = event_sender.send(GameEvent::CraftingCancelled {
            player_id: player.id,
            operation_id: op_id,
        });
    }

    player.action_state = ActionState::Idle;
}

