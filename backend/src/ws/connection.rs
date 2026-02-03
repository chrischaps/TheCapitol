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
    #[serde(rename = "extract")]
    Extract { node_id: String },
    #[serde(rename = "cancel_extraction")]
    CancelExtraction,
    // M3: Crafting
    #[serde(rename = "craft")]
    Craft { recipe_id: String, input_item_ids: Vec<String> },
    #[serde(rename = "cancel_crafting")]
    CancelCrafting,
    // M3.5: Inventory
    #[serde(rename = "move_item")]
    MoveItem {
        item_id: String,
        target_container_id: String,
        target_slot: i32,
    },
    // M4: Stations
    #[serde(rename = "place_station")]
    PlaceStation {
        station_type: String,
        x: f64,
        y: f64,
        kit_item_id: String,
    },
    #[serde(rename = "remove_station")]
    RemoveStation {
        station_id: String,
    },
    #[serde(rename = "open_station")]
    OpenStation {
        station_id: String,
    },
    #[serde(rename = "close_station")]
    CloseStation,
    #[serde(rename = "craft_at_station")]
    CraftAtStation {
        station_id: String,
        recipe_id: String,
        input_item_ids: Vec<String>,
    },
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
    // M2: Extraction events
    #[serde(rename = "extraction_started")]
    ExtractionStarted {
        player_id: Uuid,
        node_id: Uuid,
        duration_ticks: u32,
    },
    #[serde(rename = "extraction_progress")]
    ExtractionProgress {
        player_id: Uuid,
        progress: u32,
        duration: u32,
    },
    #[serde(rename = "extraction_completed")]
    ExtractionCompleted {
        player_id: Uuid,
        node_id: Uuid,
        item_type: String,
        item_name: String,
        quantity: i32,
        quality: i32,
    },
    #[serde(rename = "extraction_cancelled")]
    ExtractionCancelled { player_id: Uuid },
    #[serde(rename = "node_depleted")]
    NodeDepleted { node_id: Uuid },
    #[serde(rename = "node_regenerated")]
    NodeRegenerated { node_id: Uuid },
    #[serde(rename = "nearby_nodes")]
    NearbyNodes { nodes: Vec<crate::models::ResourceNodeInfo> },
    // M3: Crafting events
    #[serde(rename = "crafting_started")]
    CraftingStarted {
        player_id: Uuid,
        operation_id: Uuid,
        recipe_id: String,
        recipe_name: String,
        duration_ticks: u32,
    },
    #[serde(rename = "crafting_progress")]
    CraftingProgress {
        player_id: Uuid,
        operation_id: Uuid,
        progress: u32,
        duration: u32,
    },
    #[serde(rename = "crafting_completed")]
    CraftingCompleted {
        player_id: Uuid,
        operation_id: Uuid,
        item_type: String,
        item_name: String,
        quantity: i32,
        quality: i32,
    },
    #[serde(rename = "crafting_cancelled")]
    CraftingCancelled {
        player_id: Uuid,
        operation_id: Uuid,
    },
    #[serde(rename = "crafting_failed")]
    CraftingFailed {
        player_id: Uuid,
        reason: String,
    },
    // M3.5: Inventory events
    #[serde(rename = "inventory_updated")]
    InventoryUpdated {
        player_id: Uuid,
        container_id: Uuid,
    },
    #[serde(rename = "item_moved")]
    ItemMoved {
        player_id: Uuid,
        item_id: Uuid,
        from_container: Uuid,
        from_slot: i32,
        to_container: Uuid,
        to_slot: i32,
    },
    #[serde(rename = "items_merged")]
    ItemsMerged {
        player_id: Uuid,
        source_item_id: Uuid,
        target_item_id: Uuid,
        new_quantity: i32,
        new_quality: i32,
    },
    // M4: Station events
    #[serde(rename = "station_placed")]
    StationPlaced {
        station_id: Uuid,
        station_type: String,
        name: String,
        owner_id: Uuid,
        x: f64,
        y: f64,
        container_id: Uuid,
    },
    #[serde(rename = "station_removed")]
    StationRemoved {
        station_id: Uuid,
    },
    #[serde(rename = "station_opened")]
    StationOpened {
        player_id: Uuid,
        station_id: Uuid,
        station_type: String,
        name: String,
        container_id: Uuid,
    },
    #[serde(rename = "station_closed")]
    StationClosed {
        player_id: Uuid,
    },
    #[serde(rename = "nearby_stations")]
    NearbyStations {
        stations: Vec<crate::models::StationInfo>,
    },
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
                                ClientMessage::Extract { node_id } => {
                                    if let Ok(node_uuid) = Uuid::parse_str(&node_id) {
                                        let mut game = state.game.write().await;
                                        game.queue_command(crate::engine::Command::StartExtraction {
                                            player_id,
                                            node_id: node_uuid,
                                        });
                                    }
                                }
                                ClientMessage::CancelExtraction => {
                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::CancelExtraction {
                                        player_id,
                                    });
                                }
                                // M3: Crafting
                                ClientMessage::Craft { recipe_id, input_item_ids } => {
                                    // Parse input item IDs
                                    let item_uuids: Vec<Uuid> = input_item_ids
                                        .iter()
                                        .filter_map(|id| Uuid::parse_str(id).ok())
                                        .collect();

                                    // Fetch items from database to validate ownership and get qualities
                                    let items = fetch_player_items(&state, player_id, &item_uuids).await;

                                    if items.is_empty() && !item_uuids.is_empty() {
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::CraftingFailed {
                                                player_id,
                                                reason: "Invalid or unauthorized items".to_string(),
                                            }).unwrap().into()
                                        )).await;
                                        continue;
                                    }

                                    let mut game = state.game.write().await;
                                    // Store validated items for the command handler
                                    game.pending_craft_items.insert(player_id, items);
                                    game.queue_command(crate::engine::Command::StartCrafting {
                                        player_id,
                                        recipe_id,
                                        input_item_ids: item_uuids,
                                    });
                                }
                                ClientMessage::CancelCrafting => {
                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::CancelCrafting {
                                        player_id,
                                    });
                                }
                                // M3.5: Inventory
                                ClientMessage::MoveItem { item_id, target_container_id, target_slot } => {
                                    let item_uuid = match Uuid::parse_str(&item_id) {
                                        Ok(id) => id,
                                        Err(_) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error {
                                                    message: "Invalid item ID".to_string(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };
                                    let container_uuid = match Uuid::parse_str(&target_container_id) {
                                        Ok(id) => id,
                                        Err(_) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error {
                                                    message: "Invalid container ID".to_string(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    // Handle move directly via database (synchronous for immediate feedback)
                                    match handle_move_item(&state, player_id, item_uuid, container_uuid, target_slot).await {
                                        Ok(_response) => {
                                            // Notify client of inventory update
                                            let msg = ServerMessage::InventoryUpdated {
                                                player_id,
                                                container_id: container_uuid,
                                            };
                                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                        }
                                        Err(e) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error {
                                                    message: e,
                                                }).unwrap().into()
                                            )).await;
                                        }
                                    }
                                }
                                // M4: Stations
                                ClientMessage::PlaceStation { station_type, x, y, kit_item_id } => {
                                    let kit_uuid = match Uuid::parse_str(&kit_item_id) {
                                        Ok(id) => id,
                                        Err(_) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error {
                                                    message: "Invalid kit item ID".to_string(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    match handle_place_station(&state, player_id, &station_type, x, y, kit_uuid).await {
                                        Ok(station_info) => {
                                            // Add to game state
                                            {
                                                let mut game = state.game.write().await;
                                                let station_state = crate::models::StationState {
                                                    id: station_info.id,
                                                    station_type: station_info.station_type.clone(),
                                                    owner_id: station_info.owner_id,
                                                    x: station_info.x,
                                                    y: station_info.y,
                                                    container_id: station_info.container_id,
                                                };
                                                game.stations.insert(station_info.id, station_state);
                                            }

                                            // Notify client
                                            let msg = ServerMessage::StationPlaced {
                                                station_id: station_info.id,
                                                station_type: station_info.station_type,
                                                name: station_info.name,
                                                owner_id: station_info.owner_id,
                                                x: station_info.x,
                                                y: station_info.y,
                                                container_id: station_info.container_id.unwrap_or(Uuid::nil()),
                                            };
                                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                        }
                                        Err(e) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error { message: e }).unwrap().into()
                                            )).await;
                                        }
                                    }
                                }
                                ClientMessage::RemoveStation { station_id } => {
                                    let station_uuid = match Uuid::parse_str(&station_id) {
                                        Ok(id) => id,
                                        Err(_) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error {
                                                    message: "Invalid station ID".to_string(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    match handle_remove_station(&state, player_id, station_uuid).await {
                                        Ok(_) => {
                                            // Remove from game state
                                            {
                                                let mut game = state.game.write().await;
                                                game.stations.remove(&station_uuid);
                                            }

                                            let msg = ServerMessage::StationRemoved { station_id: station_uuid };
                                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                        }
                                        Err(e) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error { message: e }).unwrap().into()
                                            )).await;
                                        }
                                    }
                                }
                                ClientMessage::OpenStation { station_id } => {
                                    let station_uuid = match Uuid::parse_str(&station_id) {
                                        Ok(id) => id,
                                        Err(_) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error {
                                                    message: "Invalid station ID".to_string(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    match handle_open_station(&state, player_id, station_uuid).await {
                                        Ok((station_type, name, container_id)) => {
                                            let msg = ServerMessage::StationOpened {
                                                player_id,
                                                station_id: station_uuid,
                                                station_type,
                                                name,
                                                container_id,
                                            };
                                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                        }
                                        Err(e) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error { message: e }).unwrap().into()
                                            )).await;
                                        }
                                    }
                                }
                                ClientMessage::CloseStation => {
                                    let msg = ServerMessage::StationClosed { player_id };
                                    let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                }
                                ClientMessage::CraftAtStation { station_id, recipe_id, input_item_ids } => {
                                    let station_uuid = match Uuid::parse_str(&station_id) {
                                        Ok(id) => id,
                                        Err(_) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::CraftingFailed {
                                                    player_id,
                                                    reason: "Invalid station ID".to_string(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    let item_uuids: Vec<Uuid> = input_item_ids
                                        .iter()
                                        .filter_map(|id| Uuid::parse_str(id).ok())
                                        .collect();

                                    // Fetch items for crafting validation
                                    let items = fetch_player_items(&state, player_id, &item_uuids).await;

                                    if items.is_empty() && !item_uuids.is_empty() {
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::CraftingFailed {
                                                player_id,
                                                reason: "Invalid or unauthorized items".to_string(),
                                            }).unwrap().into()
                                        )).await;
                                        continue;
                                    }

                                    let mut game = state.game.write().await;
                                    game.pending_craft_items.insert(player_id, items);
                                    game.queue_command(crate::engine::Command::CraftAtStation {
                                        player_id,
                                        station_id: station_uuid,
                                        recipe_id,
                                        input_item_ids: item_uuids,
                                    });
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
                    // M2: Extraction events
                    Ok(GameEvent::ExtractionStarted { player_id: pid, node_id, duration_ticks }) => {
                        let msg = ServerMessage::ExtractionStarted { player_id: pid, node_id, duration_ticks };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::ExtractionProgress { player_id: pid, progress, duration }) => {
                        let msg = ServerMessage::ExtractionProgress { player_id: pid, progress, duration };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::ExtractionCompleted { player_id: pid, node_id, item_type, item_name, quantity, quality }) => {
                        let msg = ServerMessage::ExtractionCompleted { player_id: pid, node_id, item_type, item_name, quantity, quality };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::ExtractionCancelled { player_id: pid }) => {
                        let msg = ServerMessage::ExtractionCancelled { player_id: pid };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::NodeDepleted { node_id }) => {
                        let msg = ServerMessage::NodeDepleted { node_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::NodeRegenerated { node_id }) => {
                        let msg = ServerMessage::NodeRegenerated { node_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::NearbyNodes(nodes)) => {
                        let msg = ServerMessage::NearbyNodes { nodes };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    // M3: Crafting events
                    Ok(GameEvent::CraftingStarted { player_id: pid, operation_id, recipe_id, recipe_name, duration_ticks }) => {
                        let msg = ServerMessage::CraftingStarted { player_id: pid, operation_id, recipe_id, recipe_name, duration_ticks };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::CraftingProgress { player_id: pid, operation_id, progress, duration }) => {
                        let msg = ServerMessage::CraftingProgress { player_id: pid, operation_id, progress, duration };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::CraftingCompleted { player_id: pid, operation_id, item_type, item_name, quantity, quality }) => {
                        let msg = ServerMessage::CraftingCompleted { player_id: pid, operation_id, item_type, item_name, quantity, quality };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::CraftingCancelled { player_id: pid, operation_id }) => {
                        let msg = ServerMessage::CraftingCancelled { player_id: pid, operation_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::CraftingFailed { player_id: pid, reason }) => {
                        let msg = ServerMessage::CraftingFailed { player_id: pid, reason };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    // M3.5: Inventory events
                    Ok(GameEvent::InventoryUpdated { player_id: pid, container_id, slots: _ }) => {
                        let msg = ServerMessage::InventoryUpdated { player_id: pid, container_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::ItemMoved { player_id: pid, item_id, from_container, from_slot, to_container, to_slot }) => {
                        let msg = ServerMessage::ItemMoved { player_id: pid, item_id, from_container, from_slot, to_container, to_slot };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::ItemsMerged { player_id: pid, source_item_id, target_item_id, new_quantity, new_quality }) => {
                        let msg = ServerMessage::ItemsMerged { player_id: pid, source_item_id, target_item_id, new_quantity, new_quality };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    // M4: Station events
                    Ok(GameEvent::StationPlaced { station_id, station_type, name, owner_id, x, y, container_id }) => {
                        let msg = ServerMessage::StationPlaced { station_id, station_type, name, owner_id, x, y, container_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::StationRemoved { station_id }) => {
                        let msg = ServerMessage::StationRemoved { station_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::StationOpened { player_id: pid, station_id, station_type, name, container_id }) => {
                        let msg = ServerMessage::StationOpened { player_id: pid, station_id, station_type, name, container_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::StationClosed { player_id: pid }) => {
                        let msg = ServerMessage::StationClosed { player_id: pid };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::NearbyStations(stations)) => {
                        let msg = ServerMessage::NearbyStations { stations };
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

/// Fetch player items by IDs (validates ownership)
async fn fetch_player_items(state: &AppState, player_id: Uuid, item_ids: &[Uuid]) -> Vec<crate::models::Item> {
    if item_ids.is_empty() {
        return Vec::new();
    }

    // Fetch items owned by this player from the provided IDs
    let items: Vec<crate::models::Item> = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE id = ANY($1) AND owner_id = $2"
    )
    .bind(item_ids)
    .bind(player_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    items
}

/// Handle move item request directly via database
async fn handle_move_item(
    state: &AppState,
    player_id: Uuid,
    item_id: Uuid,
    target_container_id: Uuid,
    target_slot: i32,
) -> Result<String, String> {
    // Verify the item belongs to player and get its current location
    let source_item: crate::models::Item = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE id = $1 AND owner_id = $2"
    )
    .bind(item_id)
    .bind(player_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Item not found".to_string())?;

    // Verify target container belongs to player
    let _target_container: crate::models::Container = sqlx::query_as(
        "SELECT id, container_type, owner_id, station_id, created_at
         FROM containers WHERE id = $1 AND owner_id = $2"
    )
    .bind(target_container_id)
    .bind(player_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Container not found".to_string())?;

    // Check if target slot is occupied
    let target_item: Option<crate::models::Item> = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE container_id = $1 AND slot_index = $2"
    )
    .bind(target_container_id)
    .bind(target_slot)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    match target_item {
        None => {
            // Target empty: simple move
            sqlx::query(
                "UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3"
            )
            .bind(target_container_id)
            .bind(target_slot)
            .bind(item_id)
            .execute(&state.db)
            .await
            .map_err(|e| format!("Database error: {}", e))?;

            Ok("moved".to_string())
        }
        Some(target) if target.item_type == source_item.item_type => {
            // Same type: merge stacks
            let new_qty = source_item.quantity + target.quantity;
            let new_quality = (source_item.quality * source_item.quantity
                + target.quality * target.quantity)
                / new_qty;

            // Update target with merged values
            sqlx::query("UPDATE items SET quantity = $1, quality = $2 WHERE id = $3")
                .bind(new_qty)
                .bind(new_quality)
                .bind(target.id)
                .execute(&state.db)
                .await
                .map_err(|e| format!("Database error: {}", e))?;

            // Delete source item
            sqlx::query("DELETE FROM items WHERE id = $1")
                .bind(item_id)
                .execute(&state.db)
                .await
                .map_err(|e| format!("Database error: {}", e))?;

            Ok("merged".to_string())
        }
        Some(target) => {
            // Different type: swap
            let source_container_id = source_item.container_id;
            let source_slot = source_item.slot_index;

            // Move source to target's location
            sqlx::query("UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3")
                .bind(target_container_id)
                .bind(target_slot)
                .bind(item_id)
                .execute(&state.db)
                .await
                .map_err(|e| format!("Database error: {}", e))?;

            // Move target to source's location
            sqlx::query("UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3")
                .bind(source_container_id)
                .bind(source_slot)
                .bind(target.id)
                .execute(&state.db)
                .await
                .map_err(|e| format!("Database error: {}", e))?;

            Ok("swapped".to_string())
        }
    }
}

/// Handle place station request
async fn handle_place_station(
    state: &AppState,
    player_id: Uuid,
    station_type: &str,
    x: f64,
    y: f64,
    kit_item_id: Uuid,
) -> Result<crate::models::StationInfo, String> {
    // Get player position
    let (player_x, player_y): (f64, f64) = sqlx::query_as(
        "SELECT position_x, position_y FROM players WHERE id = $1"
    )
    .bind(player_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    // Verify station type exists
    let st: crate::models::StationType = sqlx::query_as(
        "SELECT id, name, category, slot_count, layout_columns, interaction_range, icon, description
         FROM station_types WHERE id = $1"
    )
    .bind(station_type)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| format!("Unknown station type: {}", station_type))?;

    // Check range from player
    let dx = x - player_x;
    let dy = y - player_y;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance > st.interaction_range as f64 {
        return Err("Too far to place station".into());
    }

    // Verify player owns the kit item and it's the right type
    let expected_kit = match station_type {
        "workbench" => "workbench_kit",
        "forge" => "forge_kit",
        "storage_chest" => "chest_kit",
        _ => return Err("Invalid station type".into()),
    };

    let kit_item: crate::models::Item = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE id = $1 AND owner_id = $2"
    )
    .bind(kit_item_id)
    .bind(player_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Kit item not found".to_string())?;

    if kit_item.item_type != expected_kit {
        return Err(format!("Wrong item type: expected {}, got {}", expected_kit, kit_item.item_type));
    }

    // Check minimum distance from other stations (50 units)
    let nearby: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM stations
         WHERE (position_x - $1)^2 + (position_y - $2)^2 < $3"
    )
    .bind(x)
    .bind(y)
    .bind(50.0 * 50.0)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    if nearby.is_some() {
        return Err("Too close to another station".into());
    }

    // Create station
    let station_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stations (id, station_type, owner_id, position_x, position_y)
         VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(station_id)
    .bind(station_type)
    .bind(player_id)
    .bind(x as f32)
    .bind(y as f32)
    .execute(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    // Create container for station
    let container_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO containers (id, container_type, station_id)
         VALUES ($1, 'station_inventory', $2)"
    )
    .bind(container_id)
    .bind(station_id)
    .execute(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    // Consume the kit item
    if kit_item.quantity <= 1 {
        sqlx::query("DELETE FROM items WHERE id = $1")
            .bind(kit_item_id)
            .execute(&state.db)
            .await
            .map_err(|e| format!("Database error: {}", e))?;
    } else {
        sqlx::query("UPDATE items SET quantity = quantity - 1 WHERE id = $1")
            .bind(kit_item_id)
            .execute(&state.db)
            .await
            .map_err(|e| format!("Database error: {}", e))?;
    }

    Ok(crate::models::StationInfo {
        id: station_id,
        station_type: station_type.to_string(),
        name: st.name,
        category: st.category,
        icon: st.icon,
        x,
        y,
        owner_id: player_id,
        container_id: Some(container_id),
        interaction_range: st.interaction_range,
    })
}

/// Handle remove station request
async fn handle_remove_station(
    state: &AppState,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<(), String> {
    // Verify ownership
    let station: crate::models::Station = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y, placed_at
         FROM stations WHERE id = $1"
    )
    .bind(station_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Station not found".to_string())?;

    if station.owner_id != player_id {
        return Err("Not your station".into());
    }

    // Delete station (cascade will delete container and items)
    sqlx::query("DELETE FROM stations WHERE id = $1")
        .bind(station_id)
        .execute(&state.db)
        .await
        .map_err(|e| format!("Database error: {}", e))?;

    Ok(())
}

/// Handle open station request
async fn handle_open_station(
    state: &AppState,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<(String, String, Uuid), String> {
    // Get player position
    let (player_x, player_y): (f64, f64) = sqlx::query_as(
        "SELECT position_x, position_y FROM players WHERE id = $1"
    )
    .bind(player_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    // Get station
    let station: crate::models::Station = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y, placed_at
         FROM stations WHERE id = $1"
    )
    .bind(station_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Station not found".to_string())?;

    // Verify ownership
    if station.owner_id != player_id {
        return Err("Not your station".into());
    }

    // Get station type for range check
    let st: crate::models::StationType = sqlx::query_as(
        "SELECT id, name, category, slot_count, layout_columns, interaction_range, icon, description
         FROM station_types WHERE id = $1"
    )
    .bind(&station.station_type)
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    // Check range
    let dx = player_x - station.position_x as f64;
    let dy = player_y - station.position_y as f64;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance > st.interaction_range as f64 {
        return Err("Too far from station".into());
    }

    // Get container
    let container: crate::models::Container = sqlx::query_as(
        "SELECT id, container_type, owner_id, station_id, created_at
         FROM containers WHERE station_id = $1"
    )
    .bind(station_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Station has no container".to_string())?;

    Ok((station.station_type, st.name, container.id))
}
