use std::collections::HashMap;
use capitol_protocol::rules::stacking::{Pick, Units};
use capitol_protocol::rules::{quality, tools};
use rand::Rng;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::{ActionState, Item, ItemType, PlayerState, Recipe};
use super::events::GameEvent;
use super::craft_completion::{CraftOutput, FinishedCraft};
use super::tools::HeldTool;

/// Active crafting operation tracked in game state
#[derive(Debug, Clone)]
pub struct CraftingOperation {
    pub operation_id: Uuid,
    pub player_id: Uuid,
    pub recipe_id: String,
    /// Items to consume: (item_id, the very units to take from it)
    pub input_items_to_consume: Vec<(Uuid, Units)>,
    /// The units taken, by type, for each input's mean quality.
    pub inputs: quality::Inputs,
    /// The quality of the station it's made at, as it was at the start
    /// (`quality::NO_STATION` by hand).
    pub station_quality: i32,
    /// The tool the recipe needs, as it was at the start (E1.5)
    pub tool: Option<HeldTool>,
}

/// Process crafting for all players actively crafting.
/// Returns the crafts that finished this tick. The tick loop writes each one to the database
/// (`craft_completion::finish_craft`) and only then sends `crafting_completed` or
/// `crafting_failed`, since the inputs may have moved while it ran.
pub fn process_crafting(
    players: &mut HashMap<Uuid, PlayerState>,
    recipes: &HashMap<String, Recipe>,
    crafting_operations: &mut HashMap<Uuid, CraftingOperation>,
    item_types: &HashMap<String, ItemType>,
    event_sender: &broadcast::Sender<GameEvent>,
) -> Vec<FinishedCraft> {
    let mut completed_operations: Vec<Uuid> = Vec::new();
    let mut progress_updates: Vec<(Uuid, Uuid, u32, u32)> = Vec::new();
    let mut finished: Vec<FinishedCraft> = Vec::new();

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
                let tool_quality = operation.tool.as_ref().map_or(0, |t| tools::term_quality(&t.tool));
                let output_quality = output_quality(&operation.inputs, operation.station_quality, tool_quality, recipe);

                // Roll the outputs; they're written, and announced, by the tick loop
                let mut rng = rand::thread_rng();
                let outputs = recipe.outputs.iter().map(|output| {
                    let kind = item_types.get(&output.item_type);
                    CraftOutput {
                        item_type: output.item_type.clone(),
                        item_name: kind.map_or_else(|| output.item_type.clone(), |t| t.name.clone()),
                        quantity: rng.gen_range(output.quantity_min..=output.quantity_max),
                        quality: output_quality,
                        stackable: kind.is_none_or(|t| t.stackable),
                        durability: kind.and_then(|t| t.durability_at(output_quality)),
                    }
                }).collect();

                finished.push(FinishedCraft {
                    player_id: operation.player_id,
                    operation_id: operation.operation_id,
                    inputs: operation.input_items_to_consume.clone(),
                    outputs,
                    tool: operation.tool.clone(),
                });
            }

            // Reset player action state
            if let Some(player) = players.get_mut(&operation.player_id) {
                player.action_state = ActionState::Idle;
            }
        }
    }

    finished
}

/// The output's quality: the cascade (`rules::quality::output`) with a rolled
/// variance. `tool_quality` is the tool's quality as its wear leaves it
/// (`rules::tools::term_quality`), 0 with no tool.
pub fn output_quality(inputs: &quality::Inputs, station_quality: i32, tool_quality: i32, recipe: &Recipe) -> i32 {
    let weights = &recipe.quality_weights;
    let variance = rand::thread_rng().gen_range(weights.variance_min..=weights.variance_max);
    quality::output(inputs, station_quality, tool_quality, weights, variance)
}

/// Start crafting for a player, at a station of `station_quality`
/// (`quality::NO_STATION` by hand), with the best `tool` they hold of the type the
/// recipe needs (`tools::tool_for_recipe`)
pub fn start_crafting(
    player: &mut PlayerState,
    recipe: &Recipe,
    input_items: &[Item],
    station_quality: i32,
    tool: Option<HeldTool>,
    crafting_operations: &mut HashMap<Uuid, CraftingOperation>,
    event_sender: &broadcast::Sender<GameEvent>,
) -> Result<Uuid, String> {
    // Check if player is already doing something
    if !matches!(player.action_state, ActionState::Idle) {
        return Err("Already performing an action".to_string());
    }

    // A recipe that needs a tool takes one from the inventory
    if let Some(tool_type) = recipe.tool_type {
        if tool.is_none() {
            return Err(format!("You need a {} to make {}", tool_type, recipe.name));
        }
    }

    // Validate inputs match recipe requirements
    let mut remaining_requirements: HashMap<String, i32> = HashMap::new();
    for input in &recipe.inputs {
        remaining_requirements.insert(input.item_type.clone(), input.quantity);
    }

    let mut items_to_consume: Vec<(Uuid, Units)> = Vec::new();
    let mut inputs = quality::Inputs::default();

    // In the order the client sent them, and the best units of each stack
    // first (E1.8), so its range matches what is taken
    for item in input_items {
        if let Some(required) = remaining_requirements.get_mut(&item.item_type) {
            if *required > 0 {
                let use_quantity = (*required).min(item.quantity);
                let Some(taken) = item.units.pick(use_quantity, Pick::Best) else {
                    continue;
                };
                *required -= use_quantity;
                inputs.add_units(&item.item_type, &taken);
                items_to_consume.push((item.id, taken));
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
        inputs,
        station_quality,
        tool,
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RecipeInput, RecipeOutput};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn player() -> PlayerState {
        PlayerState {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            name: "Ada".into(),
            x: 0.0,
            y: 0.0,
            dest_x: None,
            dest_y: None,
            speed: 50.0,
            strand_balance: 0,
            current_zone: None,
            action_state: ActionState::Idle,
        }
    }

    /// The forge kit as the content has it: rope 0.7 and fiber 0.3 at a workbench.
    fn forge() -> Recipe {
        Recipe {
            id: "craft_forge".into(),
            name: "Forge Kit".into(),
            description: None,
            category: "station_crafting".into(),
            inputs: vec![
                RecipeInput { item_type: "rope".into(), quantity: 4, quality_weight: 0.7 },
                RecipeInput { item_type: "fiber".into(), quantity: 6, quality_weight: 0.3 },
            ],
            outputs: vec![RecipeOutput {
                item_type: "forge_kit".into(),
                quantity_min: 1,
                quantity_max: 1,
            }],
            base_duration_ticks: 1,
            quality_weights: quality::Weights::for_recipe(
                [("rope", 0.7), ("fiber", 0.3)],
                None,
                None,
                true,
                -3,
                3,
            ),
            required_station_type: Some("workbench".into()),
            tool_type: None,
        }
    }

    /// A stack of `kind` with up to three qualities in it (E1.8)
    fn stack(rng: &mut StdRng, kind: &str) -> Item {
        let units = Units::from_pairs((0..rng.gen_range(1..=3)).map(|_| (rng.gen_range(0..=100), rng.gen_range(1..=3))));
        Item {
            id: Uuid::new_v4(),
            item_type: kind.into(),
            quality: units.mean().unwrap(),
            quantity: units.count() as i32,
            owner_id: None,
            container_id: None,
            slot_index: None,
            created_at: chrono::Utc::now(),
            durability: None,
            min_quality: units.min().unwrap(),
            units,
        }
    }

    /// `n` of `kind`, all at one quality
    fn uniform(rng: &mut StdRng, kind: &str, n: i32) -> Item {
        let mut item = stack(rng, kind);
        let quality = rng.gen_range(0..=100);
        item.units = Units::uniform(n, quality);
        (item.quantity, item.quality, item.min_quality) = (n, quality, quality);
        item
    }

    /// What a client sends and expects (`capitol-core` `craft::check`): for
    /// each input in recipe order, stacks in slot order until it's met, the
    /// best units of each first, and the range over the units that takes, at
    /// a station of `station`.
    fn client_side(recipe: &Recipe, slots: &[Item], station: i32) -> (Vec<Item>, Vec<(Uuid, Units)>, (i32, i32)) {
        let (mut sent, mut taken, mut units) = (Vec::new(), Vec::new(), quality::Inputs::default());
        for need in &recipe.inputs {
            let mut left = need.quantity;
            for item in slots.iter().filter(|i| i.item_type == need.item_type) {
                if left <= 0 {
                    break;
                }
                let used = item.units.pick(left.min(item.quantity), Pick::Best).unwrap();
                left -= used.count() as i32;
                units.add_units(&item.item_type, &used);
                sent.push(item.clone());
                taken.push((item.id, used));
            }
        }
        let range = quality::output_range(&units, station, 0, &recipe.quality_weights);
        (sent, taken, range)
    }

    #[test]
    fn every_craft_lands_in_the_range_the_client_shows() {
        let recipe = forge();
        let recipes: HashMap<_, _> = [(recipe.id.clone(), recipe.clone())].into();
        let (events, _rx) = broadcast::channel(64);
        let mut rng = StdRng::seed_from_u64(153);
        for _ in 0..1000 {
            // Enough of both, in a shuffled slot order.
            let mut slots: Vec<Item> = Vec::new();
            for (kind, need) in [("rope", 4), ("fiber", 6)] {
                let mut have = 0;
                while have < need || rng.gen_bool(0.3) {
                    let s = stack(&mut rng, kind);
                    have += s.quantity;
                    slots.push(s);
                }
            }
            for i in (1..slots.len()).rev() {
                slots.swap(i, rng.gen_range(0..=i));
            }
            let station = rng.gen_range(0..=100);
            let (sent, taken, (low, high)) = client_side(&recipe, &slots, station);

            let mut ada = player();
            let mut ops = HashMap::new();
            start_crafting(&mut ada, &recipe, &sent, station, None, &mut ops, &events).unwrap();
            let mut players: HashMap<_, _> = [(ada.id, ada)].into();
            let done = process_crafting(&mut players, &recipes, &mut ops, &HashMap::new(), &events);
            assert_eq!(done[0].inputs, taken, "the server takes what the client counted");
            let q = done[0].outputs[0].quality;
            assert!(low <= q && q <= high, "{q} outside {low}..={high}");
        }
    }

    /// The same inputs made at an A-grade station and a D-grade one: the
    /// station's 0.2 share is 11 points between 95 and 40, more than the
    /// variance's whole spread of 6, so the A station always wins.
    #[test]
    fn a_better_station_makes_better_output() {
        let recipe = forge();
        let recipes: HashMap<_, _> = [(recipe.id.clone(), recipe.clone())].into();
        let (events, _rx) = broadcast::channel(64);
        let mut rng = StdRng::seed_from_u64(155);
        let craft = |items: &[Item], station: i32| {
            let mut ada = player();
            let mut ops = HashMap::new();
            start_crafting(&mut ada, &recipe, items, station, None, &mut ops, &events).unwrap();
            let mut players: HashMap<_, _> = [(ada.id, ada)].into();
            process_crafting(&mut players, &recipes, &mut ops, &HashMap::new(), &events)[0].outputs[0].quality
        };
        for _ in 0..200 {
            let items = vec![uniform(&mut rng, "rope", 4), uniform(&mut rng, "fiber", 6)];
            let (a, d) = (craft(&items, 95), craft(&items, 40));
            assert!(a > d, "an A station made {a}, a D station {d}");
        }
    }

    /// The forge kit, as if it took a hammer with a 0.25 share
    fn hammered() -> Recipe {
        let mut recipe = forge();
        recipe.tool_type = Some(tools::ToolType::Hammer);
        recipe.quality_weights = quality::Weights::for_recipe(
            [("rope", 0.7), ("fiber", 0.3)],
            None,
            Some(0.25),
            true,
            -3,
            3,
        );
        recipe
    }

    fn hammer(quality: i32, durability: i32) -> HeldTool {
        HeldTool {
            item_id: Uuid::new_v4(),
            item_type: "stone_hammer".into(),
            tool: tools::Tool { base_power: 6.0, quality, durability, max_durability: 60 },
            station_id: None,
        }
    }

    #[test]
    fn a_recipe_that_needs_a_tool_refuses_without_one() {
        let recipe = hammered();
        let (events, _rx) = broadcast::channel(64);
        let mut rng = StdRng::seed_from_u64(157);
        let items = vec![uniform(&mut rng, "rope", 4), uniform(&mut rng, "fiber", 6)];
        let mut ada = player();
        let mut ops = HashMap::new();
        let refusal = start_crafting(&mut ada, &recipe, &items, 50, None, &mut ops, &events).unwrap_err();
        assert_eq!(refusal, "You need a hammer to make Forge Kit");
        assert!(ops.is_empty());
        assert_eq!(ada.action_state, ActionState::Idle);
    }

    /// The tool term: a fine hammer lifts the output over a poor one, and a
    /// worn-out fine one falls back. The finished craft carries the tool to wear.
    #[test]
    fn the_best_tool_supplies_the_tool_term_and_goes_to_be_worn() {
        let recipe = hammered();
        let recipes: HashMap<_, _> = [(recipe.id.clone(), recipe.clone())].into();
        let (events, _rx) = broadcast::channel(64);
        let mut rng = StdRng::seed_from_u64(1157);
        let craft = |items: &[Item], tool: HeldTool| {
            let mut ada = player();
            let mut ops = HashMap::new();
            start_crafting(&mut ada, &recipe, items, 50, Some(tool), &mut ops, &events).unwrap();
            let mut players: HashMap<_, _> = [(ada.id, ada)].into();
            process_crafting(&mut players, &recipes, &mut ops, &HashMap::new(), &events).remove(0)
        };
        for _ in 0..200 {
            let items = vec![uniform(&mut rng, "rope", 4), uniform(&mut rng, "fiber", 6)];
            let fine = hammer(95, 60);
            let done = craft(&items, fine.clone());
            assert_eq!(done.tool.as_ref().map(|t| t.item_id), Some(fine.item_id));
            // 0.25 × (95 − 20) is 18.75 points, past the variance's spread of 6
            let (good, poor) = (done.outputs[0].quality, craft(&items, hammer(20, 60)).outputs[0].quality);
            assert!(good > poor, "a Q95 hammer made {good}, a Q20 one {poor}");
            // 1 use of 60 left: the cliff
            let worn = craft(&items, hammer(95, 1)).outputs[0].quality;
            assert!(good > worn, "a fresh hammer made {good}, a spent one {worn}");
        }
    }
}
