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
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use capitol_protocol::ws::{ClientMessage, ServerMessage, TradeItemInfo, TradeOfferItem};

use crate::engine::container_access::{check_container_access, live_position};
use crate::engine::GameEvent;
use crate::models::PlayerState;
use crate::error::SESSIONS_UNAVAILABLE;
use crate::routes::auth::Claims;
use crate::state::AppState;

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

    // Add player to game state, superseding any older connection for them
    let (connection_id, mut superseded) = {
        let mut game = state.game.write().await;
        game.connect_player(player)
    };

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
            // A newer connection took this player over (the account signed in elsewhere, or
            // this socket is half-open and the client already reconnected past it), or a login
            // or logout over REST ended this session (#78)
            _ = &mut superseded => {
                let msg = ServerMessage::AuthError { message: "Session expired".into() };
                let _ = timeout(Duration::from_secs(2), async {
                    let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    let _ = sender.send(Message::Close(None)).await;
                })
                .await;
                break;
            }

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
                                    } else {
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::ExtractionFailed {
                                                player_id,
                                                reason: format!("Invalid resource node id '{}'", node_id),
                                            }).unwrap().into()
                                        )).await;
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
                                        Ok(source_container) => {
                                            // A move between containers changes both,
                                            // so name each one that changed (#77).
                                            for container_id in moved_containers(source_container, container_uuid) {
                                                let msg = ServerMessage::InventoryUpdated { player_id, container_id };
                                                let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                            }
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
                                    let item_tuples = match validate_trade_offer(&state, player_id, &items, strands).await {
                                        Ok(tuples) => tuples,
                                        Err(message) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error { message }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

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
                                                let _ = sender.send(Message::Text(
                                                    serde_json::to_string(&ServerMessage::Error { message: e }).unwrap().into()
                                                )).await;
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
                    Ok(GameEvent::ExtractionFailed { player_id: pid, reason }) => {
                        if pid == player_id {
                            let msg = ServerMessage::ExtractionFailed { player_id: pid, reason };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
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
                        if pid == player_id {
                            let msg = ServerMessage::CraftingFailed { player_id: pid, reason };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
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
                                let msg = ServerMessage::TradeAccepted { trade_id, player_id: accepting_player };
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
                    Ok(GameEvent::TradeFailed { player_id: pid, reason }) => {
                        if pid == player_id {
                            let msg = ServerMessage::Error { message: reason };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
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
                    Ok(GameEvent::MovementBlocked { player_id: pid, reason, water_id, stopped_x, stopped_y }) => {
                        // Only send to the player who was blocked
                        if pid == player_id {
                            let msg = ServerMessage::MovementBlocked { player_id: pid, reason, water_id, stopped_x, stopped_y };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::TerrainUpdated) => {
                        // Broadcast to all connected clients
                        let msg = ServerMessage::TerrainUpdated;
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

    // Remove player from game state, unless a newer connection now serves them
    let removed = {
        let mut game = state.game.write().await;
        game.disconnect_player(player_id, connection_id)
    };

    if removed {
        tracing::info!("Player {} disconnected", player_id);
    } else {
        tracing::info!("Player {}'s superseded connection {} ended", player_id, connection_id);
    }
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
    let stored_token: Option<String> = redis.get(&session_key).await.map_err(|e| {
        tracing::error!("Redis error checking a session: {:?}", e);
        SESSIONS_UNAVAILABLE.to_string()
    })?;

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

/// Check a `trade_offer` before it reaches the trade session: parse its item ids, check its
/// amounts, and check the player owns every item in the quantity offered and has the strands.
/// The `Err` is the message the client shows. `execute_trade` checks ownership again at the end,
/// since items can move between offer and accept.
async fn validate_trade_offer(
    state: &AppState,
    player_id: Uuid,
    items: &[TradeOfferItem],
    strands: i64,
) -> Result<Vec<(Uuid, i32)>, String> {
    let offer = parse_trade_offer(items, strands)?;

    let item_ids: Vec<Uuid> = offer.iter().map(|(id, _)| *id).collect();
    let owned: Vec<(Uuid, i32)> = if item_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as("SELECT id, quantity FROM items WHERE id = ANY($1) AND owner_id = $2")
            .bind(&item_ids)
            .bind(player_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| {
                tracing::error!("Trade offer ownership check failed: {}", e);
                "Couldn't check your items; try again".to_string()
            })?
    };
    let owned: HashMap<Uuid, i32> = owned.into_iter().collect();

    let balance: Option<(i64,)> = if strands > 0 {
        sqlx::query_as("SELECT strand_balance FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| {
                tracing::error!("Trade offer balance check failed: {}", e);
                "Couldn't check your strands; try again".to_string()
            })?
    } else {
        None
    };

    check_trade_offer_holdings(&offer, &owned, strands, balance.map(|(b,)| b).unwrap_or(0))?;
    Ok(offer)
}

/// The parts of a `trade_offer` that need no database: ids parse, quantities and strands make
/// sense, and no item appears twice.
fn parse_trade_offer(items: &[TradeOfferItem], strands: i64) -> Result<Vec<(Uuid, i32)>, String> {
    if strands < 0 {
        return Err("You can't offer a negative amount of strands".to_string());
    }
    let mut offer: Vec<(Uuid, i32)> = Vec::with_capacity(items.len());
    for item in items {
        let id = Uuid::parse_str(&item.item_id)
            .map_err(|_| format!("Invalid item ID: {}", item.item_id))?;
        if item.quantity <= 0 {
            return Err("Offered quantities must be at least 1".to_string());
        }
        if offer.iter().any(|(seen, _)| *seen == id) {
            return Err("The same item is in the offer twice".to_string());
        }
        offer.push((id, item.quantity));
    }
    Ok(offer)
}

/// Check the player holds everything they offer. `owned` maps the offered item ids the player
/// owns to the quantity they own.
fn check_trade_offer_holdings(
    offer: &[(Uuid, i32)],
    owned: &HashMap<Uuid, i32>,
    strands: i64,
    strand_balance: i64,
) -> Result<(), String> {
    for (id, qty) in offer {
        match owned.get(id) {
            None => return Err("You don't own an item in that offer".to_string()),
            Some(have) if have < qty => {
                return Err(format!("You only have {} of an item you offered {} of", have, qty))
            }
            Some(_) => {}
        }
    }
    if strands > strand_balance {
        return Err(format!(
            "You only have {} strands, not the {} offered",
            strand_balance, strands
        ));
    }
    Ok(())
}

/// Handle move item request directly via database
async fn handle_move_item(
    state: &AppState,
    player_id: Uuid,
    item_id: Uuid,
    target_container_id: Uuid,
    target_slot: i32,
) -> Result<Option<Uuid>, String> {
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

    // Both ends must be usable: the player's own containers, or a station's
    // within reach (B4). The source matters too, since a swap fills it.
    let at = live_position(&state.game, player_id).await;
    if let Some(source_container_id) = source_item.container_id {
        check_container_access(&state.db, player_id, source_container_id, at)
            .await
            .map_err(|e| e.message())?;
    }
    let target_container = check_container_access(&state.db, player_id, target_container_id, at)
        .await
        .map_err(|e| e.message())?;

    // Validate slot bounds (REST always has; WS didn't)
    let slot_count: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = $1")
        .bind(&target_container.container_type)
        .fetch_one(&state.db)
        .await
        .map_err(|e| format!("Database error: {}", e))?;
    if target_slot < 0 || target_slot >= slot_count {
        return Err(format!("Slot {} out of bounds (0-{})", target_slot, slot_count - 1));
    }

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

    // A shared station can hold other players' items. Merging into or swapping
    // with one would hand it over, so only empty slots or your own items.
    if target_item.as_ref().is_some_and(|t| t.owner_id != Some(player_id)) {
        return Err("That slot holds someone else's item".into());
    }

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

            Ok(source_item.container_id)
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

            Ok(source_item.container_id)
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

            Ok(source_item.container_id)
        }
    }
}

/// The containers a move changed, each once: the target, then the source if
/// it's a different one. A move, merge or swap empties or fills the source
/// slot, so a client refetching by `container_id` needs both.
fn moved_containers(source: Option<Uuid>, target: Uuid) -> Vec<Uuid> {
    let mut changed = vec![target];
    changed.extend(source.filter(|&s| s != target));
    changed
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
    // Where the player stands now. The players row lags by up to 10 s, so
    // it's only the fallback.
    let (player_x, player_y): (f64, f64) = match live_position(&state.game, player_id).await {
        Some(at) => at,
        None => sqlx::query_as("SELECT position_x, position_y FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| format!("Database error: {}", e))?,
    };

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
    let plot = crate::routes::plots::find_plot_at_position(&state.db, x, y)
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
    .bind(capitol_protocol::rules::STATION_MIN_SPACING * capitol_protocol::rules::STATION_MIN_SPACING)
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
    // Where the player stands now. The players row lags by up to 10 s, so
    // it's only the fallback.
    let (player_x, player_y): (f64, f64) = match live_position(&state.game, player_id).await {
        Some(at) => at,
        None => sqlx::query_as("SELECT position_x, position_y FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| format!("Database error: {}", e))?,
    };

    // Get station
    let station: crate::models::Station = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y, plot_id, placed_at
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

    // Hand the items over. Each stack leaves its container first (a split
    // takes a new stack off it), and only then is each placed in its new
    // owner's inventory, so a swap can use the slots it frees (#95). An
    // inventory with no room fails the trade, and the transaction rolls back.
    let mut placing: Vec<(Uuid, Uuid)> = Vec::new();
    for (items, to) in [(&session.offer_a.items, player_b), (&session.offer_b.items, player_a)] {
        for (item_id, qty) in items {
            placing.push((hand_over(&mut tx, *item_id, *qty, to).await?, to));
        }
    }
    for to in [player_a, player_b] {
        let stacks: Vec<Uuid> = placing
            .iter()
            .filter(|(_, owner)| *owner == to)
            .map(|(id, _)| *id)
            .collect();
        if !stacks.is_empty() {
            place_in_inventory(&mut tx, to, &stacks).await?;
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

/// Gives `qty` of the stack `item_id` to `to`, out of any container: the
/// whole stack, or a new stack split off it. Returns the stack `to` now
/// owns, for [`place_in_inventory`].
async fn hand_over(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    item_id: Uuid,
    qty: i32,
    to: Uuid,
) -> Result<Uuid, String> {
    let (item_type, quality, current_qty): (String, i32, i32) =
        sqlx::query_as("SELECT item_type, quality, quantity FROM items WHERE id = $1")
            .bind(item_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
    if current_qty == qty {
        sqlx::query(
            "UPDATE items SET owner_id = $1, container_id = NULL, slot_index = NULL WHERE id = $2",
        )
        .bind(to)
        .bind(item_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
        return Ok(item_id);
    }
    sqlx::query("UPDATE items SET quantity = quantity - $1 WHERE id = $2")
        .bind(qty)
        .bind(item_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;
    let new_item_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO items (id, item_type, quality, quantity, owner_id) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(new_item_id)
    .bind(&item_type)
    .bind(quality)
    .bind(qty)
    .bind(to)
    .execute(&mut **tx)
    .await
    .map_err(|e| format!("DB error: {}", e))?;
    Ok(new_item_id)
}

/// Puts each of `stacks` in a free slot of `player`'s inventory, lowest
/// first. Fails if there aren't enough free slots.
async fn place_in_inventory(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    player: Uuid,
    stacks: &[Uuid],
) -> Result<(), String> {
    let (container_id, slot_count): (Uuid, i32) = sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.owner_id = $1 AND c.container_type = 'player_inventory' LIMIT 1",
    )
    .bind(player)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| format!("DB error: {}", e))?;
    let used: std::collections::HashSet<i32> = sqlx::query_scalar(
        "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL",
    )
    .bind(container_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| format!("DB error: {}", e))?
    .into_iter()
    .collect();
    let free = free_slots(slot_count, &used, stacks.len())
        .ok_or_else(|| "Not enough room in an inventory for the trade".to_string())?;
    for (item_id, slot) in stacks.iter().zip(free) {
        sqlx::query("UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3")
            .bind(container_id)
            .bind(slot)
            .bind(item_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
    }
    Ok(())
}

/// The lowest `n` slots below `slot_count` not in `used`, or `None` if
/// there aren't that many.
fn free_slots(slot_count: i32, used: &std::collections::HashSet<i32>, n: usize) -> Option<Vec<i32>> {
    let free: Vec<i32> = (0..slot_count).filter(|s| !used.contains(s)).take(n).collect();
    (free.len() == n).then_some(free)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_names_the_target_then_a_different_source() {
        let (inv, chest) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(moved_containers(Some(inv), chest), vec![chest, inv]);
        assert_eq!(moved_containers(Some(chest), chest), vec![chest]);
        assert_eq!(moved_containers(None, chest), vec![chest]);
    }

    #[test]
    fn traded_stacks_take_the_lowest_free_slots_or_none() {
        let used = std::collections::HashSet::from([0, 1, 3]);
        assert_eq!(free_slots(6, &used, 2), Some(vec![2, 4]));
        assert_eq!(free_slots(6, &used, 3), Some(vec![2, 4, 5]));
        assert_eq!(free_slots(6, &used, 4), None);
        assert_eq!(free_slots(4, &used, 0), Some(vec![]));
    }

    fn offer_item(id: &str, quantity: i32) -> TradeOfferItem {
        TradeOfferItem { item_id: id.to_string(), quantity }
    }

    #[test]
    fn trade_offer_rejects_a_bad_item_id() {
        let err = parse_trade_offer(&[offer_item("not-a-uuid", 1)], 0).unwrap_err();
        assert!(err.contains("Invalid item ID"), "{err}");
    }

    #[test]
    fn trade_offer_rejects_zero_and_negative_quantities() {
        let id = Uuid::new_v4().to_string();
        assert!(parse_trade_offer(&[offer_item(&id, 0)], 0).is_err());
        assert!(parse_trade_offer(&[offer_item(&id, -3)], 0).is_err());
    }

    #[test]
    fn trade_offer_rejects_negative_strands() {
        let err = parse_trade_offer(&[], -1).unwrap_err();
        assert!(err.contains("negative"), "{err}");
    }

    #[test]
    fn trade_offer_rejects_the_same_item_twice() {
        let id = Uuid::new_v4().to_string();
        let err = parse_trade_offer(&[offer_item(&id, 1), offer_item(&id, 2)], 0).unwrap_err();
        assert!(err.contains("twice"), "{err}");
    }

    #[test]
    fn trade_offer_parses_a_good_offer() {
        let id = Uuid::new_v4();
        let offer = parse_trade_offer(&[offer_item(&id.to_string(), 4)], 10).unwrap();
        assert_eq!(offer, vec![(id, 4)]);
    }

    #[test]
    fn trade_offer_rejects_an_item_not_owned() {
        let offer = vec![(Uuid::new_v4(), 1)];
        let err = check_trade_offer_holdings(&offer, &HashMap::new(), 0, 0).unwrap_err();
        assert!(err.contains("don't own"), "{err}");
    }

    #[test]
    fn trade_offer_rejects_more_than_the_player_has() {
        let id = Uuid::new_v4();
        let owned = HashMap::from([(id, 2)]);
        let err = check_trade_offer_holdings(&[(id, 5)], &owned, 0, 0).unwrap_err();
        assert!(err.contains("only have 2"), "{err}");
    }

    #[test]
    fn trade_offer_rejects_more_strands_than_the_balance() {
        let err = check_trade_offer_holdings(&[], &HashMap::new(), 50, 20).unwrap_err();
        assert!(err.contains("only have 20 strands"), "{err}");
    }

    #[test]
    fn trade_offer_accepts_what_the_player_holds() {
        let id = Uuid::new_v4();
        let owned = HashMap::from([(id, 5)]);
        assert!(check_trade_offer_holdings(&[(id, 5)], &owned, 20, 20).is_ok());
    }
}
