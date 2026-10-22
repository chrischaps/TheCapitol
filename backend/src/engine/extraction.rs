use std::collections::HashMap;
use capitol_protocol::rules::resources::ResourceYield;
use capitol_protocol::rules::tools;
use capitol_protocol::tunables::Tunables;
use rand::Rng;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::{ActionState, ItemType, NodeState, PlayerState, ResourceNodeState, ResourceType};
use super::events::GameEvent;
use super::tools::HeldTool;

/// Maximum distance from player to node for extraction (world units)
pub use capitol_protocol::rules::EXTRACTION_RANGE;

/// Quality a hand gather lands either side of its node's (`node ± 5`)
pub const HAND_VARIANCE: i32 = 5;

/// A gather that has run its time, waiting to be written. The tick loop
/// (`persist_gathers`) wears the tool, writes the item, and only then sends
/// `extraction_completed`. `quantity` 0 is a failed pull ([`roll_gather`]).
#[derive(Debug, Clone)]
pub struct PendingItem {
    pub player_id: Uuid,
    pub node_id: Uuid,
    pub item_type: String,
    pub item_name: String,
    pub quantity: i32,
    pub quality: i32,
    /// The tool that did it, if one did (E1.5)
    pub tool: Option<GatherTool>,
}

/// The tool behind a gather, and what the gather would have been without it.
#[derive(Debug, Clone)]
pub struct GatherTool {
    pub held: HeldTool,
    /// `(quantity, quality)` by hand, for when the tool left the player's
    /// inventory before the gather finished. `None` for a resource that needs
    /// the tool: then nothing is gathered.
    pub by_hand: Option<(i32, i32)>,
}

/// Process extraction for all players actively extracting.
/// `tools` holds the tool each gather started with ([`start_extraction`]).
/// `tunables` scale each resource's yield and how soon its node grows back
/// (D6.1). Returns the gathers that finished, for the tick loop to write.
#[allow(clippy::too_many_arguments)]
pub fn process_extractions(
    players: &mut HashMap<Uuid, PlayerState>,
    nodes: &mut HashMap<Uuid, ResourceNodeState>,
    resource_types: &HashMap<String, ResourceType>,
    item_types: &HashMap<String, ItemType>,
    tools: &mut HashMap<Uuid, HeldTool>,
    tick_count: u64,
    event_sender: &broadcast::Sender<GameEvent>,
    tunables: &Tunables,
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
        let tool = tools.remove(&player_id);
        if let Some(node) = nodes.get(&node_id) {
            if let Some(resource_type) = resource_types.get(&node.resource_type) {
                let mut rng = rand::thread_rng();
                let (quantity, quality, mut tool) =
                    roll_gather(&mut rng, node.base_quality, resource_type, tool);
                let multiplier = tunables.yield_multiplier(&resource_type.id);
                let quantity = scale_yield(&mut rng, quantity, multiplier);
                if let Some((by_hand, _)) = tool.as_mut().and_then(|t| t.by_hand.as_mut()) {
                    *by_hand = scale_yield(&mut rng, *by_hand, multiplier);
                }

                // Written, and announced, by the tick loop
                pending_items.push(PendingItem {
                    player_id,
                    node_id,
                    item_type: resource_type.yields_item.clone(),
                    item_name: item_types
                        .get(&resource_type.yields_item)
                        .map_or_else(|| resource_type.yields_item.clone(), |t| t.name.clone()),
                    quantity,
                    quality,
                    tool,
                });

                if let Some(node) = nodes.get_mut(&node_id) {
                    if quantity == 0 {
                        // A failed pull takes nothing, so the node stands for another try (D5)
                        node.state = NodeState::Available;
                    } else {
                        node.state = NodeState::Depleted;
                        node.harvested_by = Some(player_id);
                        node.regenerates_at_tick =
                            Some(tick_count + tunables.regeneration_ticks(resource_type.regeneration_ticks));
                        let _ = event_sender.send(GameEvent::NodeDepleted { node_id });
                    }
                }
            }
        }

        // Reset player action state
        if let Some(player) = players.get_mut(&player_id) {
            player.action_state = ActionState::Idle;
        }
    }

    pending_items
}

/// One gather's `(quantity, quality)`, and the tool behind it.
///
/// - By hand: a roll on the resource's `hand` yield row, and the node's quality
///   ± [`HAND_VARIANCE`], as it has always been.
/// - With a tool: a roll on the row for its tier (`rules::resources::yield_for`,
///   E2.2). The node's quality is the ceiling, the tool's wear scales it and a
///   tool weak for the resource's difficulty damages it
///   (`rules::tools::gathered_quality`).
///
/// Hands or a tool short of the resource's difficulty fail some pulls
/// (`rules::tools::fail_chance`, D5): a failed pull is `(0, 0)`, the tool still
/// wears, and the node isn't spent. The roll is the pull's, so when the tool
/// leaves the player's inventory before the finish, its `by_hand` fails too.
pub fn roll_gather(
    rng: &mut impl Rng,
    node_quality: i32,
    resource_type: &ResourceType,
    tool: Option<HeldTool>,
) -> (i32, i32, Option<GatherTool>) {
    let hand_quality = (node_quality + rng.gen_range(-HAND_VARIANCE..=HAND_VARIANCE)).clamp(0, 100);
    let by_hand = roll_yield(rng, resource_type.yield_for(None));
    let Some(held) = tool else {
        if fails(rng, tools::HAND_POWER, resource_type.difficulty) {
            return (0, 0, None);
        }
        return (by_hand, hand_quality, None);
    };
    let quantity = roll_yield(rng, resource_type.yield_for(held.tier));
    let power = held.tool.effective_power();
    let damage = rng.gen_range(0..=tools::max_damage(power, resource_type.difficulty));
    let quality = tools::gathered_quality(node_quality, held.tool.retention(), damage);
    let (quantity, quality, by_hand) = if fails(rng, power, resource_type.difficulty) {
        (0, 0, (0, 0))
    } else {
        (quantity, quality, (by_hand, hand_quality))
    };
    let by_hand = resource_type.required().is_none().then_some(by_hand);
    (quantity, quality, Some(GatherTool { held, by_hand }))
}

/// Whether a pull at `power` fails (`rules::tools::fail_chance`). Rolls only
/// when it can fail, so the gathers that never fail roll as they always have.
fn fails(rng: &mut impl Rng, power: f64, difficulty: f64) -> bool {
    let chance = tools::fail_chance(power, difficulty);
    chance > 0.0 && rng.gen_bool(chance)
}

/// `quantity` times a yield tunable's `multiplier`, rounded up or down at
/// random so that on average it's exact: ×1.5 on 1 gives 1 or 2 evenly. A
/// pull that got anything still gets at least 1, since 0 means it failed.
fn scale_yield(rng: &mut impl Rng, quantity: i32, multiplier: f64) -> i32 {
    if quantity <= 0 || multiplier == 1.0 {
        return quantity;
    }
    let exact = quantity as f64 * multiplier;
    let whole = exact.floor();
    let up = rng.gen_bool((exact - whole).clamp(0.0, 1.0));
    (whole as i32 + up as i32).max(1)
}

/// Units from one row of a yield table; none from a table without one.
fn roll_yield(rng: &mut impl Rng, row: Option<&ResourceYield>) -> i32 {
    row.map_or(0, |y| rng.gen_range(y.min..=y.max.max(y.min)))
}

/// How long a gather of `resource_type` takes with `tool`, or by hand: its
/// tier's row's own ticks, or the resource's, shortened by the tool's leverage
/// (`rules::tools::tool_duration`).
pub fn gather_duration(resource_type: &ResourceType, tool: Option<&HeldTool>) -> i32 {
    let row = resource_type.yield_for(tool.and_then(|held| held.tier));
    let base = row.and_then(|y| y.ticks).unwrap_or(resource_type.extraction_duration_ticks);
    match tool {
        Some(held) => tools::tool_duration(base, held.tool.effective_power(), resource_type.difficulty),
        None => base,
    }
}

/// Why `player` can't start gathering `node` right now, in words the player can act on.
/// `None` means they can. `tool` is the best one they hold of the type that works
/// it (`tools::tool_for_node`). The REST `/world/extract` check uses this too.
pub fn extraction_refusal(
    player: &PlayerState,
    node: &ResourceNodeState,
    resource_type: &ResourceType,
    tool: Option<&HeldTool>,
) -> Option<String> {
    // Check if player is already extracting
    if !matches!(player.action_state, ActionState::Idle) {
        return Some("Already performing an action".to_string());
    }

    // Check if node is available
    match node.state {
        NodeState::Available => {}
        NodeState::BeingHarvested => {
            return Some(format!("Someone else is already gathering this {}", resource_type.name));
        }
        NodeState::Depleted => {
            return Some(format!("This {} is spent; it will grow back", resource_type.name));
        }
    }

    // Check range
    let dx = player.x - node.x;
    let dy = player.y - node.y;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance > EXTRACTION_RANGE {
        return Some(format!(
            "Too far from resource ({:.0} units; gathering needs {:.0})",
            distance, EXTRACTION_RANGE
        ));
    }

    // Check tool requirement
    if let Some(required) = &resource_type.required_tool {
        if tool.is_none() {
            return Some(format!("You need a {} to gather {}", required, resource_type.name));
        }
    }

    None
}

/// Start extraction for a player on a node, with the best `tool` they hold for it.
/// The tool shortens the gather (`rules::tools::tool_duration`) and is kept in
/// `tools` until it finishes.
pub fn start_extraction(
    player: &mut PlayerState,
    node: &mut ResourceNodeState,
    resource_type: &ResourceType,
    tool: Option<HeldTool>,
    tools: &mut HashMap<Uuid, HeldTool>,
    event_sender: &broadcast::Sender<GameEvent>,
) -> Result<(), String> {
    if let Some(reason) = extraction_refusal(player, node, resource_type, tool.as_ref()) {
        return Err(reason);
    }

    // Stop any movement
    player.dest_x = None;
    player.dest_y = None;
    player.steer = None;

    // Start extraction
    let duration = gather_duration(resource_type, tool.as_ref());
    player.action_state = ActionState::Extracting {
        node_id: node.id,
        progress: 0,
        duration,
    };
    match tool {
        Some(held) => tools.insert(player.id, held),
        None => tools.remove(&player.id),
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

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::tools::{Tier, Tool};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    /// Wild Grass as the content has it
    fn grass() -> ResourceType {
        crate::content::resource_type("grass")
    }

    /// The stone sickle at `quality`, with `durability` of its uses left
    fn sickle(quality: i32, durability: i32) -> HeldTool {
        HeldTool {
            item_id: Uuid::new_v4(),
            item_type: "stone_sickle".into(),
            tool: Tool {
                base_power: 6.0,
                quality,
                durability,
                max_durability: tools::max_durability(60, quality),
            },
            tier: Some(Tier::Stone),
            station_id: None,
        }
    }

    /// A fresh Q50 tool of `tool_type` at `tier`, with D5's power for the tier
    fn fresh(tool_type: &str, tier: Tier) -> HeldTool {
        let (base_power, uses) = match tier {
            Tier::Stone => (10.0, 60),
            _ => (18.0, 200),
        };
        HeldTool {
            item_id: Uuid::new_v4(),
            item_type: format!("{}_{tool_type}", tier.as_str()),
            tool: Tool { base_power, quality: 50, durability: uses, max_durability: uses },
            tier: Some(tier),
            station_id: None,
        }
    }

    /// Every quantity `rolls` gathers of `resource` give, with `tool` or by hand
    fn quantities(resource: &ResourceType, tool: Option<&HeldTool>, rolls: usize) -> Vec<i32> {
        let mut rng = StdRng::seed_from_u64(161);
        (0..rolls).map(|_| roll_gather(&mut rng, 60, resource, tool.cloned()).0).collect()
    }

    /// E2.2's acceptance: each D5 resource gathered by hand and with its tool
    /// gives its own yield row, every value of it and none outside it, and a
    /// tool makes it quicker. One that needs a tool refuses hands. Hands or a
    /// tool short of the difficulty also fail about their share of pulls (#206).
    #[test]
    fn every_resource_gathers_with_and_without_its_tool() {
        let (events, _rx) = broadcast::channel(256);
        let content = crate::content::load().unwrap();
        for def in &content.resources {
            let resource = def.to_type();
            let rows: Vec<(Option<Tier>, &str)> =
                [(None, "hand"), (Some(Tier::Stone), "stone"), (Some(Tier::Iron), "iron")]
                    .into_iter()
                    .filter(|(_, row)| def.yields.contains_key(*row))
                    .collect();
            assert!(!rows.is_empty(), "{} has no rows", def.id);
            for (tier, row) in rows {
                let tool = tier.map(|tier| fresh(def.tool().unwrap(), tier));
                let expected = &def.yields[row];
                let got = quantities(&resource, tool.as_ref(), 500);
                for q in expected.min..=expected.max {
                    assert!(got.contains(&q), "{} at {row} never gave {q}", def.id);
                }
                let power = tool.as_ref().map_or(tools::HAND_POWER, |t| t.tool.effective_power());
                let chance = tools::fail_chance(power, def.difficulty);
                let (failed, pulled): (Vec<i32>, Vec<i32>) = got.into_iter().partition(|q| *q == 0);
                assert!(pulled.iter().all(|q| (expected.min..=expected.max).contains(q)), "{} at {row}: {pulled:?}", def.id);
                let rate = failed.len() as f64 / 500.0;
                assert!((rate - chance).abs() < 0.07, "{} at {row} failed {rate:.2} of pulls, not {chance:.2}", def.id);

                let (mut ada, mut node, mut held) = (player(), node(), HashMap::new());
                node.resource_type = def.id.clone();
                start_extraction(&mut ada, &mut node, &resource, tool, &mut held, &events).unwrap();
                let ActionState::Extracting { duration, .. } = ada.action_state else { unreachable!() };
                let base = expected.ticks.unwrap_or(def.extraction_duration_ticks);
                match tier {
                    None => assert_eq!(duration, base, "{} by hand", def.id),
                    Some(_) => assert!(duration < base, "{} at {row} took {duration} of {base}", def.id),
                }
            }
            if let Some(required) = &def.required_tool {
                let (mut ada, mut node, mut held) = (player(), node(), HashMap::new());
                let refusal = start_extraction(&mut ada, &mut node, &resource, None, &mut held, &events).unwrap_err();
                assert_eq!(refusal, format!("You need a {required} to gather {}", def.name));
            }
        }
    }

    /// Bronze has no row of its own anywhere: it reads stone's. Steel reads iron's.
    #[test]
    fn a_tier_with_no_row_gathers_by_the_one_below() {
        let tree = crate::content::resource_type("tree");
        let bronze = quantities(&tree, Some(&fresh("axe", Tier::Bronze)), 300);
        assert!(bronze.iter().all(|q| (1..=2).contains(q)), "{bronze:?}");
        let steel = quantities(&tree, Some(&fresh("axe", Tier::Steel)), 300);
        assert!(steel.iter().all(|q| (2..=3).contains(q)), "{steel:?}");
    }

    /// Rock gives to hands, but slowly: D5's one stone in 80 ticks, and hands
    /// (4 against 8) fail half their pulls. A stone pick (10) never does.
    #[test]
    fn stone_by_hand_is_slow() {
        let outcrop = crate::content::resource_type("rock_outcrop");
        assert_eq!(gather_duration(&outcrop, None), 80);
        assert_eq!(gather_duration(&outcrop, Some(&fresh("pickaxe", Tier::Stone))), 39);
        let by_hand = quantities(&outcrop, None, 1000);
        assert!(by_hand.iter().all(|q| [0, 1].contains(q)), "{by_hand:?}");
        let stones = by_hand.iter().sum::<i32>();
        assert!((440..=560).contains(&stones), "{stones} stones in 1000 pulls by hand");
        assert!(!quantities(&outcrop, Some(&fresh("pickaxe", Tier::Stone)), 500).contains(&0));
    }

    /// D5's example: a stone pick on an iron vein (10 against 20) fails about
    /// half its pulls, an iron one (18) about a tenth, and a fine stone pick
    /// (Q100, 15) a quarter. A failed pull gives nothing at no quality, and its
    /// by-hand fallback fails with it.
    #[test]
    fn a_weak_pick_fails_some_pulls_on_a_vein() {
        let vein = crate::content::resource_type("iron_vein");
        let rate = |tool: HeldTool| {
            let got = quantities(&vein, Some(&tool), 2000);
            got.iter().filter(|q| **q == 0).count() as f64 / got.len() as f64
        };
        let stone = rate(fresh("pickaxe", Tier::Stone));
        assert!((0.46..0.54).contains(&stone), "a stone pick failed {stone:.3}");
        let iron = rate(fresh("pickaxe", Tier::Iron));
        assert!((0.08..0.12).contains(&iron), "an iron pick failed {iron:.3}");
        let mut fine = fresh("pickaxe", Tier::Stone);
        fine.tool.quality = 100;
        let fine = rate(fine);
        assert!((0.22..0.28).contains(&fine), "a fine stone pick failed {fine:.3}");

        let mut rng = StdRng::seed_from_u64(206);
        let outcrop = crate::content::resource_type("rock_outcrop");
        let mut weak = fresh("pickaxe", Tier::Stone);
        weak.tool.base_power = 4.0; // 4 against 8: half its pulls fail
        let (mut failures, mut successes) = (0, 0);
        for _ in 0..400 {
            let (quantity, quality, tool) = roll_gather(&mut rng, 60, &outcrop, Some(weak.clone()));
            let by_hand = tool.unwrap().by_hand.unwrap();
            if quantity == 0 {
                failures += 1;
                assert_eq!((quality, by_hand), (0, (0, 0)), "nothing, by tool or by hand");
            } else {
                successes += 1;
                assert_eq!(by_hand.0, 1, "hands would have had their stone");
            }
        }
        assert!(failures > 100 && successes > 100, "{failures} failed, {successes} didn't");
    }

    /// A failed pull leaves the node standing for another try, and says
    /// nothing of it being spent; one that gives spends it as before
    #[test]
    fn a_failed_pull_leaves_the_node() {
        let (events, mut heard) = broadcast::channel(256);
        let vein = crate::content::resource_type("iron_vein");
        let types: HashMap<_, _> = [("iron_vein".to_string(), vein.clone())].into();
        let items = HashMap::new();
        let (mut failed, mut spent) = (0, 0);
        for _ in 0..40 {
            let (mut ada, mut node, mut held) = (player(), node(), HashMap::new());
            node.resource_type = "iron_vein".into();
            start_extraction(&mut ada, &mut node, &vein, Some(fresh("pickaxe", Tier::Stone)), &mut held, &events)
                .unwrap();
            let ActionState::Extracting { duration, .. } = ada.action_state else { unreachable!() };
            let (ada_id, node_id) = (ada.id, node.id);
            let mut players: HashMap<_, _> = [(ada.id, ada)].into();
            let mut nodes: HashMap<_, _> = [(node.id, node)].into();
            while heard.try_recv().is_ok() {}
            let mut done = Vec::new();
            for tick in 0..duration as u64 {
                done.extend(process_extractions(&mut players, &mut nodes, &types, &items, &mut held, tick, &events, &Tunables::default()));
            }
            assert_eq!(done.len(), 1);
            assert_eq!(players[&ada_id].action_state, ActionState::Idle);
            let depleted = std::iter::from_fn(|| heard.try_recv().ok())
                .any(|e| matches!(e, GameEvent::NodeDepleted { node_id: n } if n == node_id));
            if done[0].quantity == 0 {
                failed += 1;
                assert_eq!(nodes[&node_id].state, NodeState::Available, "still there to try");
                assert!(nodes[&node_id].regenerates_at_tick.is_none());
                assert!(!depleted);
                assert!(done[0].tool.is_some(), "the pick still wears");
            } else {
                spent += 1;
                assert_eq!(nodes[&node_id].state, NodeState::Depleted);
                assert!(depleted);
            }
        }
        assert!(failed > 0 && spent > 0, "{failed} failed, {spent} spent");
    }

    /// Hand gathering is what it was before tools: 1 to 3 fiber at the node's
    /// quality ± 5, every value reachable and none outside
    #[test]
    fn gathering_grass_by_hand_is_unchanged() {
        let mut rng = StdRng::seed_from_u64(157);
        let (mut quantities, mut qualities) = (Vec::new(), Vec::new());
        for _ in 0..2000 {
            let (quantity, quality, tool) = roll_gather(&mut rng, 55, &grass(), None);
            assert!(tool.is_none());
            quantities.push(quantity);
            qualities.push(quality);
        }
        for q in 1..=3 {
            assert!(quantities.contains(&q), "never gathered {q}");
        }
        assert!(quantities.iter().all(|q| (1..=3).contains(q)));
        for q in 50..=60 {
            assert!(qualities.contains(&q), "never came out at {q}");
        }
        assert!(qualities.iter().all(|q| (50..=60).contains(q)));

        // And it takes the resource's own time
        let (events, _rx) = broadcast::channel(16);
        let (mut ada, mut node, mut held) = (player(), node(), HashMap::new());
        start_extraction(&mut ada, &mut node, &grass(), None, &mut held, &events).unwrap();
        assert!(matches!(ada.action_state, ActionState::Extracting { duration: 30, .. }));
    }

    /// A stone sickle reads grass's stone row, 2 to 4 against hands' 1 to 3.
    /// A yield tunable scales a gather on average, never turns a pull into a
    /// failure, and leaves a failure failed (D6.1)
    #[test]
    fn a_yield_multiplier_scales_on_average() {
        let mut rng = StdRng::seed_from_u64(7);
        assert_eq!(scale_yield(&mut rng, 3, 1.0), 3);
        assert_eq!(scale_yield(&mut rng, 0, 4.0), 0);
        assert_eq!(scale_yield(&mut rng, 4, 2.0), 8);
        assert_eq!(scale_yield(&mut rng, 1, 0.1), 1);
        let total: i32 = (0..10_000).map(|_| scale_yield(&mut rng, 2, 1.25)).sum();
        assert!((24_500..=25_500).contains(&total), "{total}");
    }

    /// Grass is hands' work (difficulty 0, counted as 4), so a Q50 sickle of
    /// power 6 is leverage 1.5: never over the node's quality, at most 3 under it
    #[test]
    fn a_sickle_raises_the_fiber_yield() {
        let mut rng = StdRng::seed_from_u64(1157);
        let (mut by_hand, mut with_sickle) = (0, 0);
        for _ in 0..2000 {
            by_hand += roll_gather(&mut rng, 55, &grass(), None).0;
            let (quantity, quality, tool) = roll_gather(&mut rng, 55, &grass(), Some(sickle(50, 60)));
            with_sickle += quantity;
            assert!((2..=4).contains(&quantity), "{quantity}");
            assert!((52..=55).contains(&quality), "{quality}");
            let (hand_quantity, hand_quality) = tool.unwrap().by_hand.unwrap();
            assert!((1..=3).contains(&hand_quantity) && (50..=60).contains(&hand_quality));
        }
        let ratio = with_sickle as f64 / by_hand as f64;
        assert!((1.4..1.6).contains(&ratio), "a sickle gathered {ratio:.2}× as much");
    }

    #[test]
    fn a_worn_sickle_gathers_poorer_fiber() {
        let mut rng = StdRng::seed_from_u64(2157);
        for _ in 0..200 {
            // 3 of 60 left: 5%, half retention
            let (_, quality, _) = roll_gather(&mut rng, 60, &grass(), Some(sickle(50, 3)));
            assert!((25..=30).contains(&quality), "{quality}");
        }
    }

    #[test]
    fn a_sickle_shortens_the_gather() {
        let (events, _rx) = broadcast::channel(16);
        let (mut ada, mut node, mut held) = (player(), node(), HashMap::new());
        start_extraction(&mut ada, &mut node, &grass(), Some(sickle(50, 60)), &mut held, &events).unwrap();
        assert!(matches!(ada.action_state, ActionState::Extracting { duration: 22, .. }), "{:?}", ada.action_state);
        assert!(held.contains_key(&ada.id));
    }

    #[test]
    fn a_gated_resource_refuses_without_its_tool() {
        let mut reeds = grass();
        reeds.required_tool = Some("sickle".into());
        let (events, _rx) = broadcast::channel(16);
        let (mut ada, mut node, mut held) = (player(), node(), HashMap::new());
        let refusal = start_extraction(&mut ada, &mut node, &reeds, None, &mut held, &events).unwrap_err();
        assert_eq!(refusal, "You need a sickle to gather Wild Grass");
        assert_eq!(node.state, NodeState::Available, "the node is left alone");

        // With one it starts, and there's nothing to fall back on by hand
        start_extraction(&mut ada, &mut node, &reeds, Some(sickle(50, 60)), &mut held, &events).unwrap();
        let mut rng = StdRng::seed_from_u64(3157);
        let (_, _, tool) = roll_gather(&mut rng, 55, &reeds, Some(sickle(50, 60)));
        assert!(tool.unwrap().by_hand.is_none());
    }

    #[test]
    fn a_finished_gather_hands_its_tool_to_the_tick_loop() {
        let (events, _rx) = broadcast::channel(64);
        let (mut ada, mut node, mut held) = (player(), node(), HashMap::new());
        let sickle = sickle(50, 60);
        start_extraction(&mut ada, &mut node, &grass(), Some(sickle.clone()), &mut held, &events).unwrap();
        let (ada_id, node_id) = (ada.id, node.id);
        let mut players: HashMap<_, _> = [(ada.id, ada)].into();
        let mut nodes: HashMap<_, _> = [(node.id, node)].into();
        let types: HashMap<_, _> = [("grass".to_string(), grass())].into();
        let fiber = ItemType {
            id: "fiber".into(),
            name: "Plant Fiber".into(),
            category: "raw_material".into(),
            stackable: true,
            weight: 0.1,
            tool_type: None,
            tier: None,
            base_power: None,
            max_durability: None,
            target_price: Some(1),
        };
        let items: HashMap<_, _> = [("fiber".to_string(), fiber)].into();
        let mut done = Vec::new();
        for tick in 0..24 {
            done.extend(process_extractions(&mut players, &mut nodes, &types, &items, &mut held, tick, &events, &Tunables::default()));
        }
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].item_name, "Plant Fiber", "named from the item types");
        assert_eq!((done[0].player_id, done[0].node_id), (ada_id, node_id));
        assert_eq!(done[0].tool.as_ref().unwrap().held.item_id, sickle.item_id);
        assert!(held.is_empty());
        assert_eq!(players[&ada_id].action_state, ActionState::Idle);
    }

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
            citizen: false,
            steer: None,
        }
    }

    fn node() -> ResourceNodeState {
        ResourceNodeState {
            id: Uuid::new_v4(),
            resource_type: "grass".into(),
            x: 5.0,
            y: 0.0,
            base_quality: 55,
            state: NodeState::Available,
            harvested_by: None,
            regenerates_at_tick: None,
        }
    }
}
