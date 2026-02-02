use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use jsonwebtoken::{decode, DecodingKey, Validation};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use crate::engine::GameEvent;
use crate::models::PlayerState;
use crate::routes::auth::Claims;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "auth")]
    Auth { token: String },
    #[serde(rename = "subscribe")]
    Subscribe { channel: String },
    #[serde(rename = "move")]
    Move { x: f64, y: f64 },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "auth_ok")]
    AuthOk { player_id: Uuid },
    #[serde(rename = "auth_error")]
    AuthError { message: String },
    #[serde(rename = "subscribed")]
    Subscribed { channel: String },
    #[serde(rename = "positions")]
    Positions { players: Vec<crate::models::PlayerPosition> },
    #[serde(rename = "player_joined")]
    PlayerJoined { player: crate::models::PlayerPosition },
    #[serde(rename = "player_left")]
    PlayerLeft { player_id: Uuid },
    #[serde(rename = "error")]
    Error { message: String },
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Wait for auth message with timeout
    let auth_result = timeout(Duration::from_secs(10), async {
        while let Some(msg) = receiver.next().await {
            if let Ok(Message::Text(text)) = msg {
                if let Ok(ClientMessage::Auth { token }) = serde_json::from_str(&text) {
                    return Some(token);
                }
            }
        }
        None
    })
    .await;

    let token = match auth_result {
        Ok(Some(token)) => token,
        _ => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::AuthError {
                        message: "Auth timeout".into(),
                    })
                    .unwrap().into(),
                ))
                .await;
            return;
        }
    };

    // Validate token
    let claims = match validate_token(&token, &state).await {
        Ok(claims) => claims,
        Err(msg) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::AuthError { message: msg }).unwrap().into(),
                ))
                .await;
            return;
        }
    };

    // Load player into game state
    let player = match load_player(&state, claims.sub).await {
        Ok(player) => player,
        Err(msg) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::AuthError { message: msg }).unwrap().into(),
                ))
                .await;
            return;
        }
    };

    let player_id = player.id;

    // Add player to game state
    {
        let mut game = state.game.write().await;
        game.add_player(player);
    }

    // Send auth success
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&ServerMessage::AuthOk { player_id }).unwrap().into(),
        ))
        .await;

    // Subscribe to game events
    let mut event_rx = {
        let game = state.game.read().await;
        game.subscribe()
    };

    // Handle messages and events
    let mut subscribed_position = false;

    loop {
        tokio::select! {
            // Handle incoming messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                            match client_msg {
                                ClientMessage::Subscribe { channel } => {
                                    if channel == "position" {
                                        subscribed_position = true;
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::Subscribed { channel }).unwrap().into()
                                        )).await;
                                    }
                                }
                                ClientMessage::Move { x, y } => {
                                    if x >= 0.0 && x <= 1000.0 && y >= 0.0 && y <= 1000.0 {
                                        let mut game = state.game.write().await;
                                        game.queue_command(crate::engine::Command::Move {
                                            player_id,
                                            dest_x: x,
                                            dest_y: y,
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    _ => {}
                }
            }

            // Handle game events
            event = event_rx.recv() => {
                if !subscribed_position {
                    continue;
                }

                match event {
                    Ok(GameEvent::PlayerPositions(players)) => {
                        let msg = ServerMessage::Positions { players };
                        if sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(GameEvent::PlayerJoined(player)) => {
                        let msg = ServerMessage::PlayerJoined { player };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::PlayerLeft { player_id: left_id }) => {
                        let msg = ServerMessage::PlayerLeft { player_id: left_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Err(_) => {
                        // Channel closed
                        break;
                    }
                }
            }
        }
    }

    // Remove player from game state
    {
        let mut game = state.game.write().await;
        game.remove_player(player_id);
    }

    tracing::info!("Player {} disconnected", player_id);
}

async fn validate_token(token: &str, state: &AppState) -> Result<Claims, String> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| "Invalid token".to_string())?;

    // Verify session in Redis
    let mut redis = state.redis.clone();
    let session_key = format!("session:{}", token_data.claims.sub);
    let stored_token: Option<String> = redis.get(&session_key).await.ok().flatten();

    if stored_token.as_deref() != Some(token) {
        return Err("Session expired".into());
    }

    Ok(token_data.claims)
}

async fn load_player(state: &AppState, account_id: Uuid) -> Result<PlayerState, String> {
    let player = sqlx::query_as::<_, crate::models::Player>(
        "SELECT id, account_id, name, position_x, position_y, destination_x, destination_y, speed, created_at
         FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(account_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Player not found".to_string())?;

    Ok(player.into())
}
