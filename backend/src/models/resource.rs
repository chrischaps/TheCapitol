use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

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

/// Client-facing resource node data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceNodeInfo {
    pub id: Uuid,
    pub resource_type: String,
    pub x: f64,
    pub y: f64,
    pub state: String,
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
