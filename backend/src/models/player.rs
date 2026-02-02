use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

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
    pub created_at: DateTime<Utc>,
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
}

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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerPosition {
    pub id: Uuid,
    pub name: String,
    pub x: f64,
    pub y: f64,
}
