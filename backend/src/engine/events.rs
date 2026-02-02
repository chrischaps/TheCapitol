use serde::Serialize;
use uuid::Uuid;

use crate::models::PlayerPosition;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum GameEvent {
    PlayerPositions(Vec<PlayerPosition>),
    PlayerJoined(PlayerPosition),
    PlayerLeft { player_id: Uuid },
}
