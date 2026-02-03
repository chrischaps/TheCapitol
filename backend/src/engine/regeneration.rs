use std::collections::HashMap;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::{NodeState, ResourceNodeState};
use super::events::GameEvent;

/// Process regeneration for all depleted nodes
pub fn process_regeneration(
    nodes: &mut HashMap<Uuid, ResourceNodeState>,
    tick_count: u64,
    event_sender: &broadcast::Sender<GameEvent>,
) {
    let mut regenerated_nodes: Vec<Uuid> = Vec::new();

    // Find nodes that should regenerate
    for node in nodes.values() {
        if node.state == NodeState::Depleted {
            if let Some(regen_tick) = node.regenerates_at_tick {
                if tick_count >= regen_tick {
                    regenerated_nodes.push(node.id);
                }
            }
        }
    }

    // Regenerate nodes
    for node_id in regenerated_nodes {
        if let Some(node) = nodes.get_mut(&node_id) {
            node.state = NodeState::Available;
            node.harvested_by = None;
            node.regenerates_at_tick = None;

            // Emit regeneration event
            let _ = event_sender.send(GameEvent::NodeRegenerated { node_id });
        }
    }
}
