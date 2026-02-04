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
    // M5: Trading
    #[serde(rename = "trade_initiate")]
    TradeInitiate { target_player_id: String },
    #[serde(rename = "trade_accept_request")]
    TradeAcceptRequest,
    #[serde(rename = "trade_decline_request")]
    TradeDeclineRequest,
    #[serde(rename = "trade_offer")]
    TradeOffer {
        items: Vec<TradeOfferItem>,
        strands: i64,
    },
    #[serde(rename = "trade_accept")]
    TradeAccept,
    #[serde(rename = "trade_cancel")]
    TradeCancel,
}

#[derive(Debug, Deserialize)]
struct TradeOfferItem {
    item_id: String,
    quantity: i32,
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
    // M5: Trading events
    #[serde(rename = "trade_requested")]
    TradeRequested {
        from_player: Uuid,
        from_player_name: String,
    },
    #[serde(rename = "trade_request_declined")]
    TradeRequestDeclined {
        by_player: Uuid,
    },
    #[serde(rename = "trade_started")]
    TradeStarted {
        trade_id: Uuid,
        partner_id: Uuid,
        partner_name: String,
    },
    #[serde(rename = "trade_offer_updated")]
    TradeOfferUpdated {
        trade_id: Uuid,
        player_id: Uuid,
        items: Vec<TradeItemInfo>,
        strands: i64,
    },
    #[serde(rename = "trade_accepted")]
    TradeAcceptedMsg {
        trade_id: Uuid,
        player_id: Uuid,
    },
    #[serde(rename = "trade_executed")]
    TradeExecuted {
        trade_id: Uuid,
    },
    #[serde(rename = "trade_cancelled")]
    TradeCancelled {
        trade_id: Uuid,
        reason: String,
    },
    // M5: Currency events
    #[serde(rename = "currency_changed")]
    CurrencyChanged {
        new_balance: i64,
    },
    // M7: Zone events
    #[serde(rename = "zone_changed")]
    ZoneChanged {
        player_id: Uuid,
        from_zone: String,
        to_zone: String,
        zone_name: String,
    },
    // Terrain events
    #[serde(rename = "movement_blocked")]
    MovementBlocked {
        player_id: Uuid,
        reason: String,
        stopped_x: f64,
        stopped_y: f64,
    },
}

#[derive(Debug, Serialize)]
struct TradeItemInfo {
    item_id: Uuid,
    item_type: String,
    item_name: String,
    quantity: i32,
    quality: i32,
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
                                    // M7: World expanded to 8000x8000
                                    if x >= 0.0 && x <= 8000.0 && y >= 0.0 && y <= 8000.0 {
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
                                        Ok(result) => {
                                            let station_info = result.info;
                                            // Add to game state
                                            {
                                                let mut game = state.game.write().await;
                                                let station_state = crate::models::StationState {
                                                    id: station_info.id,
                                                    station_type: station_info.station_type.clone(),
                                                    owner_id: station_info.owner_id,
                                                    x: station_info.x,
                                                    y: station_info.y,
                                                    plot_id: result.plot_id,
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
                                // M5: Trading
                                ClientMessage::TradeInitiate { target_player_id } => {
                                    let target_uuid = match Uuid::parse_str(&target_player_id) {
                                        Ok(id) => id,
                                        Err(_) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error {
                                                    message: "Invalid player ID".to_string(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::InitiateTrade {
                                        player_id,
                                        target_player_id: target_uuid,
                                    });
                                }
                                ClientMessage::TradeAcceptRequest => {
                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::AcceptTradeRequest {
                                        player_id,
                                    });
                                }
                                ClientMessage::TradeDeclineRequest => {
                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::DeclineTradeRequest {
                                        player_id,
                                    });
                                }
                                ClientMessage::TradeOffer { items, strands } => {
                                    let item_tuples: Vec<(Uuid, i32)> = items
                                        .iter()
                                        .filter_map(|item| {
                                            Uuid::parse_str(&item.item_id)
                                                .ok()
                                                .map(|id| (id, item.quantity))
                                        })
                                        .collect();

                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::UpdateTradeOffer {
                                        player_id,
                                        items: item_tuples,
                                        strands,
                                    });
                                }
                                ClientMessage::TradeAccept => {
                                    // Handle acceptance directly (not queued) to avoid race condition
                                    let (should_execute, trade_id_opt) = {
                                        let mut game = state.game.write().await;
                                        match game.trade_manager.set_accepted(player_id, true) {
                                            Ok(both_accepted) => {
                                                if let Some(trade) = game.trade_manager.get_player_trade(player_id) {
                                                    let trade_id = trade.id;
                                                    // Broadcast that this player accepted
                                                    let _ = game.event_sender.send(GameEvent::TradeAccepted {
                                                        trade_id,
                                                        player_id,
                                                    });
                                                    (both_accepted, Some(trade_id))
                                                } else {
                                                    (false, None)
                                                }
                                            }
                                            Err(e) => {
                                                tracing::debug!("Trade accept failed: {}", e);
                                                (false, None)
                                            }
                                        }
                                    };

                                    if should_execute {
                                        // Execute trade with DB access
                                        match execute_trade(&state, player_id).await {
                                            Ok(trade_id) => {
                                                // Broadcast trade executed
                                                let game = state.game.read().await;
                                                let _ = game.event_sender.send(GameEvent::TradeExecuted { trade_id });
                                            }
                                            Err(e) => {
                                                tracing::error!("Trade execution failed: {}", e);
                                                // Cancel the trade
                                                let mut game = state.game.write().await;
                                                if let Some(trade_id) = trade_id_opt {
                                                    let _ = game.trade_manager.cancel_trade(player_id);
                                                    let _ = game.event_sender.send(GameEvent::TradeCancelled {
                                                        trade_id,
                                                        reason: e,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                                ClientMessage::TradeCancel => {
                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::CancelTrade {
                                        player_id,
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
                    // M5: Trading events
                    Ok(GameEvent::TradeRequested { from_player, from_player_name, to_player }) => {
                        // Only send to the target player
                        if to_player == player_id {
                            let msg = ServerMessage::TradeRequested { from_player, from_player_name };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::TradeRequestDeclined { by_player, initiator }) => {
                        // Send to initiator
                        if initiator == player_id {
                            let msg = ServerMessage::TradeRequestDeclined { by_player };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::TradeStarted { trade_id, player_a, player_b }) => {
                        // Send to both players
                        if player_id == player_a || player_id == player_b {
                            let partner_id = if player_id == player_a { player_b } else { player_a };
                            // Get partner name from game state
                            let partner_name = {
                                let game = state.game.read().await;
                                game.players.get(&partner_id).map(|p| p.name.clone()).unwrap_or_default()
                            };
                            let msg = ServerMessage::TradeStarted { trade_id, partner_id, partner_name };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::TradeOfferUpdated { trade_id, player_id: offering_player, items, strands }) => {
                        // Get trade participants
                        let participants = {
                            let game = state.game.read().await;
                            game.trade_manager.active_trades.get(&trade_id)
                                .map(|t| (t.player_a, t.player_b))
                        };

                        if let Some((player_a, player_b)) = participants {
                            if player_id == player_a || player_id == player_b {
                                // Fetch item details from database
                                let item_ids: Vec<Uuid> = items.iter().map(|i| i.item_id).collect();
                                let trade_items = if !item_ids.is_empty() {
                                    let db_items: Vec<(Uuid, String, i32, i32)> = sqlx::query_as(
                                        "SELECT i.id, i.item_type, i.quality, i.quantity FROM items i WHERE i.id = ANY($1)"
                                    )
                                    .bind(&item_ids)
                                    .fetch_all(&state.db)
                                    .await
                                    .unwrap_or_default();

                                    // Get item type names
                                    let type_ids: Vec<String> = db_items.iter().map(|(_, t, _, _)| t.clone()).collect();
                                    let type_names: std::collections::HashMap<String, String> = if !type_ids.is_empty() {
                                        sqlx::query_as::<_, (String, String)>(
                                            "SELECT id, name FROM item_types WHERE id = ANY($1)"
                                        )
                                        .bind(&type_ids)
                                        .fetch_all(&state.db)
                                        .await
                                        .unwrap_or_default()
                                        .into_iter()
                                        .collect()
                                    } else {
                                        std::collections::HashMap::new()
                                    };

                                    // Match up with offer quantities
                                    items.iter().filter_map(|offer_item| {
                                        db_items.iter().find(|(id, _, _, _)| *id == offer_item.item_id).map(|(id, item_type, quality, _)| {
                                            let item_name = type_names.get(item_type).cloned().unwrap_or_else(|| item_type.clone());
                                            TradeItemInfo {
                                                item_id: *id,
                                                item_type: item_type.clone(),
                                                item_name,
                                                quantity: offer_item.quantity,
                                                quality: *quality,
                                            }
                                        })
                                    }).collect()
                                } else {
                                    Vec::new()
                                };

                                let msg = ServerMessage::TradeOfferUpdated {
                                    trade_id,
                                    player_id: offering_player,
                                    items: trade_items,
                                    strands,
                                };
                                let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                            }
                        }
                    }
                    Ok(GameEvent::TradeAccepted { trade_id, player_id: accepting_player }) => {
                        // Get trade participants
                        let participants = {
                            let game = state.game.read().await;
                            game.trade_manager.active_trades.get(&trade_id)
                                .map(|t| (t.player_a, t.player_b))
                        };

                        if let Some((player_a, player_b)) = participants {
                            if player_id == player_a || player_id == player_b {
                                let msg = ServerMessage::TradeAcceptedMsg { trade_id, player_id: accepting_player };
                                let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                            }
                        }
                    }
                    Ok(GameEvent::TradeExecuted { trade_id }) => {
                        let msg = ServerMessage::TradeExecuted { trade_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::TradeCancelled { trade_id, reason }) => {
                        let msg = ServerMessage::TradeCancelled { trade_id, reason };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    // M5: Currency events
                    Ok(GameEvent::CurrencyChanged { player_id: pid, new_balance }) => {
                        if pid == player_id {
                            let msg = ServerMessage::CurrencyChanged { new_balance };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    // M7: Zone events
                    Ok(GameEvent::ZoneChanged { player_id: pid, from_zone, to_zone, zone_name }) => {
                        // Only send to the player who changed zones
                        if pid == player_id {
                            let msg = ServerMessage::ZoneChanged { player_id: pid, from_zone, to_zone, zone_name };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    // Terrain events
                    Ok(GameEvent::MovementBlocked { player_id: pid, reason, stopped_x, stopped_y }) => {
                        // Only send to the player who was blocked
                        if pid == player_id {
                            let msg = ServerMessage::MovementBlocked { player_id: pid, reason, stopped_x, stopped_y };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
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
        "SELECT id, account_id, name, position_x, position_y, destination_x, destination_y, speed, strand_balance, current_zone, created_at
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
/// Result of placing a station (includes plot_id for game state)
struct PlaceStationResult {
    info: crate::models::StationInfo,
    plot_id: Option<Uuid>,
}

async fn handle_place_station(
    state: &AppState,
    player_id: Uuid,
    station_type: &str,
    x: f64,
    y: f64,
    kit_item_id: Uuid,
) -> Result<PlaceStationResult, String> {
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

    // M7: Find plot at placement position
    let plot: crate::models::Plot = sqlx::query_as(
        "SELECT id, zone_id, grid_x, grid_y, world_x, world_y,
                bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y,
                size_category, plot_type, owner_id, claimed_at,
                assessed_value, last_tax_paid, tax_owed, station_count
         FROM plots
         WHERE $1 >= bounds_min_x AND $1 <= bounds_max_x
           AND $2 >= bounds_min_y AND $2 <= bounds_max_y
         LIMIT 1"
    )
    .bind(x as f32)
    .bind(y as f32)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Must place station on a plot".to_string())?;

    // M7: Verify player owns the plot
    if plot.owner_id != Some(player_id) {
        return Err("You don't own this plot".into());
    }

    // M7: Check plot capacity
    let (station_count, station_capacity): (i32, i32) = sqlx::query_as(
        "SELECT p.station_count, psc.station_capacity
         FROM plots p
         JOIN plot_size_categories psc ON p.size_category = psc.id
         WHERE p.id = $1"
    )
    .bind(plot.id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    if station_count >= station_capacity {
        return Err(format!("Plot is at capacity ({}/{})", station_count, station_capacity));
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

    // Create station with plot_id
    let station_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id)
         VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(station_id)
    .bind(station_type)
    .bind(player_id)
    .bind(x as f32)
    .bind(y as f32)
    .bind(plot.id)
    .execute(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    // M7: Increment plot station count
    sqlx::query("UPDATE plots SET station_count = station_count + 1 WHERE id = $1")
        .bind(plot.id)
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

    Ok(PlaceStationResult {
        info: crate::models::StationInfo {
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
        },
        plot_id: Some(plot.id),
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
        "SELECT id, station_type, owner_id, position_x, position_y, plot_id, placed_at
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

    // M7: Decrement plot station count if station was on a plot
    if let Some(plot_id) = station.plot_id {
        sqlx::query("UPDATE plots SET station_count = station_count - 1 WHERE id = $1")
            .bind(plot_id)
            .execute(&state.db)
            .await
            .map_err(|e| format!("Database error: {}", e))?;
    }

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

/// Execute a trade atomically
/// Transfers items and strands between both players
async fn execute_trade(state: &AppState, player_id: Uuid) -> Result<Uuid, String> {
    // Get the trade session
    let session = {
        let mut game = state.game.write().await;
        game.trade_manager.complete_trade(player_id)?
    };

    let trade_id = session.id;
    let player_a = session.player_a;
    let player_b = session.player_b;

    // Verify both players are still in range
    {
        let game = state.game.read().await;
        let pos_a = game.players.get(&player_a).map(|p| (p.x, p.y));
        let pos_b = game.players.get(&player_b).map(|p| (p.x, p.y));

        match (pos_a, pos_b) {
            (Some((ax, ay)), Some((bx, by))) => {
                let dx = ax - bx;
                let dy = ay - by;
                let distance = (dx * dx + dy * dy).sqrt();
                if distance > crate::engine::trading::TRADE_RANGE {
                    return Err("Players moved out of trade range".to_string());
                }
            }
            _ => return Err("Player not found".to_string()),
        }
    }

    // Start database transaction
    let mut tx = state.db.begin().await.map_err(|e| format!("DB error: {}", e))?;

    // Verify all items in offer A still exist and are owned by player A
    for (item_id, qty) in &session.offer_a.items {
        let item: Option<(i32,)> = sqlx::query_as(
            "SELECT quantity FROM items WHERE id = $1 AND owner_id = $2"
        )
        .bind(item_id)
        .bind(player_a)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        match item {
            Some((actual_qty,)) if actual_qty >= *qty => {}
            _ => return Err("Item no longer available".to_string()),
        }
    }

    // Verify all items in offer B still exist and are owned by player B
    for (item_id, qty) in &session.offer_b.items {
        let item: Option<(i32,)> = sqlx::query_as(
            "SELECT quantity FROM items WHERE id = $1 AND owner_id = $2"
        )
        .bind(item_id)
        .bind(player_b)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        match item {
            Some((actual_qty,)) if actual_qty >= *qty => {}
            _ => return Err("Item no longer available".to_string()),
        }
    }

    // Verify strand balances
    let balance_a: (i64,) = sqlx::query_as("SELECT strand_balance FROM players WHERE id = $1 FOR UPDATE")
        .bind(player_a)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    let balance_b: (i64,) = sqlx::query_as("SELECT strand_balance FROM players WHERE id = $1 FOR UPDATE")
        .bind(player_b)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    if balance_a.0 < session.offer_a.strands {
        return Err(format!("Player A has insufficient strands"));
    }
    if balance_b.0 < session.offer_b.strands {
        return Err(format!("Player B has insufficient strands"));
    }

    // Transfer items from A to B
    for (item_id, qty) in &session.offer_a.items {
        // Get item details
        let (item_type, quality, current_qty): (String, i32, i32) = sqlx::query_as(
            "SELECT item_type, quality, quantity FROM items WHERE id = $1"
        )
        .bind(item_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        if current_qty == *qty {
            // Transfer entire item
            sqlx::query("UPDATE items SET owner_id = $1 WHERE id = $2")
                .bind(player_b)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("DB error: {}", e))?;
        } else {
            // Split stack: reduce source quantity, create new item for recipient
            sqlx::query("UPDATE items SET quantity = quantity - $1 WHERE id = $2")
                .bind(qty)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("DB error: {}", e))?;

            let new_item_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO items (id, item_type, quality, quantity, owner_id)
                 VALUES ($1, $2, $3, $4, $5)"
            )
            .bind(new_item_id)
            .bind(&item_type)
            .bind(quality)
            .bind(qty)
            .bind(player_b)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        }
    }

    // Transfer items from B to A
    for (item_id, qty) in &session.offer_b.items {
        let (item_type, quality, current_qty): (String, i32, i32) = sqlx::query_as(
            "SELECT item_type, quality, quantity FROM items WHERE id = $1"
        )
        .bind(item_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

        if current_qty == *qty {
            sqlx::query("UPDATE items SET owner_id = $1 WHERE id = $2")
                .bind(player_a)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("DB error: {}", e))?;
        } else {
            sqlx::query("UPDATE items SET quantity = quantity - $1 WHERE id = $2")
                .bind(qty)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("DB error: {}", e))?;

            let new_item_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO items (id, item_type, quality, quantity, owner_id)
                 VALUES ($1, $2, $3, $4, $5)"
            )
            .bind(new_item_id)
            .bind(&item_type)
            .bind(quality)
            .bind(qty)
            .bind(player_a)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
        }
    }

    // Transfer strands
    let net_strands = session.offer_b.strands - session.offer_a.strands;
    let new_balance_a = balance_a.0 + net_strands;
    let new_balance_b = balance_b.0 - net_strands;

    sqlx::query("UPDATE players SET strand_balance = $1 WHERE id = $2")
        .bind(new_balance_a)
        .bind(player_a)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    sqlx::query("UPDATE players SET strand_balance = $1 WHERE id = $2")
        .bind(new_balance_b)
        .bind(player_b)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    // Add audit records for currency changes
    if session.offer_a.strands > 0 {
        sqlx::query(
            "INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, reference_id, description)
             VALUES ($1, $2, $3, $4, 'trade_sent', $5, 'Trade payment')"
        )
        .bind(player_a)
        .bind(-session.offer_a.strands)
        .bind(balance_a.0)
        .bind(balance_a.0 - session.offer_a.strands)
        .bind(trade_id)
        .execute(&mut *tx)
        .await
        .ok();
    }

    if session.offer_b.strands > 0 {
        sqlx::query(
            "INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, reference_id, description)
             VALUES ($1, $2, $3, $4, 'trade_sent', $5, 'Trade payment')"
        )
        .bind(player_b)
        .bind(-session.offer_b.strands)
        .bind(balance_b.0)
        .bind(balance_b.0 - session.offer_b.strands)
        .bind(trade_id)
        .execute(&mut *tx)
        .await
        .ok();
    }

    // Commit transaction
    tx.commit().await.map_err(|e| format!("DB error: {}", e))?;

    tracing::info!("Trade {} executed successfully between {} and {}", trade_id, player_a, player_b);

    Ok(trade_id)
}
