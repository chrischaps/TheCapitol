use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub use capitol_protocol::ws::PlayerPosition;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Player {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub position_x: f64,
    pub position_y: f64,
    pub destination_x: Option<f64>,
    pub destination_y: Option<f64>,
    pub speed: f32,
    pub strand_balance: i64,
    pub current_zone: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Player action state for the game engine
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum ActionState {
    #[default]
    Idle,
    Extracting {
        node_id: Uuid,
        progress: i32,
        duration: i32,
    },
    Crafting {
        operation_id: Uuid,
        recipe_id: String,
        progress: i32,
        duration: i32,
    },
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub dest_x: Option<f64>,
    pub dest_y: Option<f64>,
    pub speed: f32,
    pub strand_balance: i64,
    pub current_zone: Option<String>,
    pub action_state: ActionState,
    /// A citizen's, not a person's (C0.1): marked to everyone who sees them
    #[serde(default)]
    pub citizen: bool,
    /// Walking on a heading (controls T0.2), instead of to `dest_x`/`dest_y`.
    /// Never saved: a reload stands still.
    #[serde(skip)]
    pub steer: Option<Steering>,
}

/// A player walking on a heading, from `steer`s (controls T0.2): the
/// protocol's, so the Flint client's loopback walks the same steer (T0.4).
pub use capitol_protocol::rules::steer::Steering;

impl From<Player> for PlayerState {
    fn from(p: Player) -> Self {
        Self {
            id: p.id,
            account_id: p.account_id,
            name: p.name,
            x: p.position_x,
            y: p.position_y,
            dest_x: p.destination_x,
            dest_y: p.destination_y,
            speed: p.speed,
            strand_balance: p.strand_balance,
            current_zone: p.current_zone,
            action_state: ActionState::Idle,
            // `load_player` reads it from the account
            citizen: false,
            steer: None,
        }
    }
}
