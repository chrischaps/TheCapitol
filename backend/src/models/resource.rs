use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub use capitol_protocol::rest::ResourceNodeInfo;

/// Static resource type definition (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResourceType {
    pub id: String,
    pub name: String,
    pub yields_item: String,
    pub yield_quantity_min: i32,
    pub yield_quantity_max: i32,
    pub base_quality_min: i32,
    pub base_quality_max: i32,
    pub required_tool: Option<String>,
    pub extraction_duration_ticks: i32,
    pub regeneration_ticks: i32,
}

/// Resource node in the world (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResourceNode {
    pub id: Uuid,
    pub resource_type: String,
    pub position_x: f64,
    pub position_y: f64,
    pub base_quality: i32,
    pub state: String,
    pub harvested_by: Option<Uuid>,
    pub regenerates_at_tick: Option<i64>,
    pub created_at: DateTime<Utc>,
}

/// Node state for in-memory game state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    Available,
    BeingHarvested,
    Depleted,
}

impl From<&str> for NodeState {
    fn from(s: &str) -> Self {
        match s {
            "available" => NodeState::Available,
            "harvesting" => NodeState::BeingHarvested,
            "depleted" => NodeState::Depleted,
            _ => NodeState::Available,
        }
    }
}

impl From<NodeState> for String {
    fn from(state: NodeState) -> Self {
        match state {
            NodeState::Available => "available".to_string(),
            NodeState::BeingHarvested => "harvesting".to_string(),
            NodeState::Depleted => "depleted".to_string(),
        }
    }
}

/// In-memory resource node state for the game engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceNodeState {
    pub id: Uuid,
    pub resource_type: String,
    pub x: f64,
    pub y: f64,
    pub base_quality: i32,
    pub state: NodeState,
    pub harvested_by: Option<Uuid>,
    pub regenerates_at_tick: Option<u64>,
}

impl From<ResourceNode> for ResourceNodeState {
    fn from(node: ResourceNode) -> Self {
        Self {
            id: node.id,
            resource_type: node.resource_type,
            x: node.position_x,
            y: node.position_y,
            base_quality: node.base_quality,
            state: NodeState::from(node.state.as_str()),
            harvested_by: node.harvested_by,
            regenerates_at_tick: node.regenerates_at_tick.map(|t| t as u64),
        }
    }
}

impl From<&ResourceNodeState> for ResourceNodeInfo {
    fn from(node: &ResourceNodeState) -> Self {
        Self {
            id: node.id,
            resource_type: node.resource_type.clone(),
            x: node.x,
            y: node.y,
            state: String::from(node.state),
        }
    }
}

#[cfg(test)]
mod tests {
    use capitol_protocol::rules::{
        check_water_crossing, Zone, CENTER_X, CENTER_Y, NEARBY_NODE_RANGE, SPAWN_SCATTER,
        SPAWN_X, SPAWN_Y,
    };

    use crate::models::terrain::{point_to_segment_distance, TerrainConfig};

    const WORLD_SCALE_MIGRATION: &str =
        include_str!("../../migrations/20260212_001_world_scale.sql");

    /// The `(x, y, quality)` rows the world-scale migration seeds.
    fn seeded_nodes() -> Vec<(f64, f64, i32)> {
        WORLD_SCALE_MIGRATION
            .lines()
            .filter_map(|line| {
                let row = line.trim().strip_prefix('(')?;
                let row = row.trim_end_matches(',').strip_suffix(')')?;
                let mut fields = row.split(',').map(str::trim);
                let x = fields.next()?.parse().ok()?;
                let y = fields.next()?.parse().ok()?;
                let quality = fields.next()?.parse().ok()?;
                Some((x, y, quality))
            })
            .collect()
    }

    /// How far a point is from the edge of the nearest road; negative inside one.
    fn road_clearance(terrain: &TerrainConfig, x: f64, y: f64) -> f64 {
        terrain
            .roads
            .iter()
            .flat_map(|road| {
                road.points.windows(2).map(move |w| {
                    point_to_segment_distance(x, y, w[0].0, w[0].1, w[1].0, w[1].1)
                        - road.width / 2.0
                })
            })
            .fold(f64::MAX, f64::min)
    }

    /// Checks against the default world only. The live world can be regenerated
    /// with other params, and nothing moves the nodes then (#70).
    #[test]
    fn seeded_nodes_sit_clear_of_the_default_terrain() {
        let terrain = TerrainConfig::get_default();
        let nodes = seeded_nodes();
        assert_eq!(nodes.len(), 156);

        for &(x, y, quality) in &nodes {
            let clearance = road_clearance(&terrain, x, y);
            assert!(clearance >= 19.9, "node at ({x}, {y}) is {clearance:.1} from a road");
            assert!((40..=70).contains(&quality), "node at ({x}, {y}) has quality {quality}");
            let near_spawn = (x - SPAWN_X).hypot(y - SPAWN_Y) <= NEARBY_NODE_RANGE;
            let zone = Zone::from_position(x, y);
            assert!(
                near_spawn || matches!(zone, Zone::Suburban | Zone::Rural),
                "node at ({x}, {y}) is in {zone}"
            );
        }
    }

    #[test]
    fn a_new_player_arrives_on_the_road_in_with_grass_in_sight() {
        let terrain = TerrainConfig::get_default();
        assert_eq!(Zone::from_position(SPAWN_X, SPAWN_Y), Zone::Rural);

        // On the arterial, even at the edge of the scatter, with room for the
        // player's own radius.
        for dx in [-SPAWN_SCATTER, 0.0, SPAWN_SCATTER] {
            let clearance = road_clearance(&terrain, SPAWN_X + dx, SPAWN_Y);
            assert!(clearance <= -8.0, "spawn + {dx} is {clearance:.1} from the road edge");
        }

        // The walk straight in reaches the Capitol over bridges.
        let center = (CENTER_X, CENTER_Y);
        let blocked =
            check_water_crossing(center, &terrain.water_rings, (SPAWN_X, SPAWN_Y), center);
        assert!(blocked.is_none(), "the road in is cut by {blocked:?}");

        // `/world/nearby` from the spawn shows grass.
        let in_sight = seeded_nodes()
            .iter()
            .filter(|&&(x, y, _)| (x - SPAWN_X).hypot(y - SPAWN_Y) <= NEARBY_NODE_RANGE)
            .count();
        assert!(in_sight >= 4, "only {in_sight} nodes in sight of the spawn");
    }
}
