use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use jsonwebtoken::{decode, DecodingKey, Validation};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::timeout;
use uuid::Uuid;

use capitol_protocol::rules::stacking::{Pick, Units};
use capitol_protocol::rest::{ApiKeyScope, Maker};
use capitol_protocol::ws::{ClientMessage, ServerMessage, TradeItemInfo, TradeOfferItem};

use crate::engine::container_access::{
    check_container_access, check_station_access, live_position, movable_item, move_refusal,
};
use crate::api_keys::{self, KeyAuth, KeyGrant};
use crate::engine::GameEvent;
use crate::engine::load::{self, SocketEnd};
use crate::engine::presence::{self, Holder, PresenceEvent};
use crate::engine::currency::announce_balances;
use crate::models::{ItemType, PlayerState};
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
    // Counted open until this returns, and ended however it last says (C1.7)
    let mut counted = load::Socket::opened();

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

    // Validate the token, or the API key (E0.9)
    let authed = match authenticate(&token, &state).await {
        Ok(authed) => authed,
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
    let player = match load_player(&state.db, authed.account_id).await {
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
    let key = authed.key;
    // A key with only `read` watches: it never holds the avatar (`api_keys`).
    let may_hold = key.is_none_or(|key| key.scopes.takes_presence());

    // One of the player's sockets, holding nothing yet (E0.8). It takes the
    // avatar, or learns who holds it, once `auth_ok` is out.
    let (connection_id, mut presence_events) = {
        let mut game = state.game.write().await;
        game.open_socket(player_id, authed.holder, may_hold)
    };
    // Fires when the socket's API key is revoked
    let (_unrevokable, unrevokable) = tokio::sync::oneshot::channel::<String>();
    let mut revoked = match key {
        Some(key) => state.game.write().await.watch_key(key.id),
        None => unrevokable,
    };
    // A revocation between the key's check and the watch above found no
    // socket to close, so look again now the watch is in place.
    if let Some(key) = key {
        if !api_keys::is_live(&state.db, key.id).await.unwrap_or(true) {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::AuthError { message: "API key revoked".into() })
                        .unwrap()
                        .into(),
                ))
                .await;
            let mut game = state.game.write().await;
            game.disconnect_player(player_id, connection_id);
            counted.ends(SocketEnd::KeyRevoked);
            return;
        }
    }

    // Send auth success, with the server's clock for the time of day and
    // the account's subscription (E4.1)
    let subscription = crate::subscriptions::of(&state.db, authed.account_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to read {}'s subscription: {:?}", authed.account_id, e);
            None
        })
        .unwrap_or_default();
    let auth_ok = ServerMessage::AuthOk {
        player_id,
        server_time_ms: chrono::Utc::now().timestamp_millis(),
        subscription,
        fees: state.tunables.get().fees,
        is_admin: authed.is_admin,
    };
    let _ = sender
        .send(Message::Text(serde_json::to_string(&auth_ok).unwrap().into()))
        .await;
    // Signed in: from here, one that ends without saying why just went
    counted.ends(SocketEnd::Vanished);

    // Take the avatar (D9): a client always does, so the newest client holds
    // it, and a bot only while none of the player's clients is connected.
    // A socket that doesn't hold it hears who does. Either way its
    // `presence` follows `auth_ok`.
    {
        let mut game = state.game.write().await;
        if !may_hold || game.claim_presence(player, connection_id).is_err() {
            game.tell_watcher(player_id, connection_id);
        }
    }

    // Subscribe to game events
    let mut event_rx = {
        let game = state.game.read().await;
        game.subscribe()
    };

    // Handle messages and events
    let mut subscribed_position = false;
    // The interest cell this socket's player was in at the last `positions`, and
    // whether the last one sent was empty, so one out of the world isn't sent
    // an empty list ten times a second (C0.4)
    let mut my_cell: Option<crate::engine::interest::Cell> = None;
    let mut sent_nobody = false;
    // The last `positions`, with `my_cell`: only the gathering and crafting of
    // the players in sight in it reach this socket (#275)
    let mut seen: Option<std::sync::Arc<crate::engine::interest::Positions>> = None;
    // Whether everyone in the world was in sight in it, so nothing needs filtering
    let mut sees_everyone = false;
    // The items whose market channel this connection watches (E3.3)
    let mut market_items: std::collections::HashSet<String> = std::collections::HashSet::new();
    // This connection's command allowance (E0.7)
    let limit_key = state.limiter.connection();

    loop {
        tokio::select! {
            // This socket took the avatar or lost it (E0.8), or a logout over REST ended
            // its session (#78)
            event = presence_events.recv() => {
                match event {
                    Some(PresenceEvent::Changed { yours, held_by }) => {
                        let msg = ServerMessage::Presence { yours, held_by };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Some(PresenceEvent::Ended) | None => {
                        let msg = ServerMessage::AuthError { message: "Session expired".into() };
                        let _ = timeout(Duration::from_secs(2), async {
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                            let _ = sender.send(Message::Close(None)).await;
                        })
                        .await;
                        counted.ends(SocketEnd::SessionEnded);
                        break;
                    }
                }
            }

            // The API key this socket opened with was revoked, or its
            // owner's subscription lapsed (#217)
            why = &mut revoked => {
                let message = why.unwrap_or_else(|_| "API key revoked".into());
                let msg = ServerMessage::AuthError { message };
                let _ = timeout(Duration::from_secs(2), async {
                    let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    let _ = sender.send(Message::Close(None)).await;
                })
                .await;
                counted.ends(SocketEnd::KeyRevoked);
                break;
            }

            // Handle incoming messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                            // Every `error` below names the command it refuses (#89).
                            let command = client_msg.type_name();
                            if let Err(wait) = state.limiter.check(limit_key) {
                                tracing::info!("Rate limited WS {command} from player {player_id}");
                                let _ = sender.send(Message::Text(
                                    serde_json::to_string(&ServerMessage::Error {
                                        message: crate::rate_limit::ws_message(wait),
                                        command: command.into(),
                                    }).unwrap().into()
                                )).await;
                                continue;
                            }
                            if let Some(key) = key {
                                let needed = api_keys::ws_scope(&client_msg);
                                if !key.scopes.allows(needed) {
                                    let msg = ServerMessage::Error {
                                        message: api_keys::refusal(command, Some(needed)),
                                        command: command.to_string(),
                                    };
                                    let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                    continue;
                                }
                            }
                            // Only the socket holding the avatar drives it. A watcher may
                            // still use the market and automation (E0.8).
                            if presence::drives_avatar_ws(&client_msg)
                                && !state.game.read().await.holds_presence(player_id, connection_id)
                            {
                                let msg = ServerMessage::Error {
                                    message: presence::NOT_HOLDING.into(),
                                    command: command.to_string(),
                                };
                                let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                continue;
                            }
                            match client_msg {
                                ClientMessage::Subscribe { channel } => {
                                    if channel == "position" {
                                        let first = !subscribed_position;
                                        subscribed_position = true;
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::Subscribed { channel }).unwrap().into()
                                        )).await;
                                        // What waited while they were away (#138). Events since
                                        // auth are already on `event_rx`; whichever path takes a
                                        // notice first sends it. A watcher takes none.
                                        if first && state.game.read().await.holds_presence(player_id, connection_id) {
                                            let rate = state.tunables.get().property_tax_rate;
                                            match crate::engine::notices::take_all(&state.db, player_id, rate).await {
                                                Ok(waiting) => {
                                                    for msg in waiting {
                                                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                                    }
                                                }
                                                Err(e) => tracing::error!("Failed to load notices for {}: {:?}", player_id, e),
                                            }
                                        }
                                    }
                                }
                                ClientMessage::SubscribeMarket { item_type } => {
                                    let refusal = if market_items.len() >= MARKET_CHANNELS && !market_items.contains(&item_type) {
                                        Some(format!("Watch at most {MARKET_CHANNELS} items at once"))
                                    } else {
                                        match sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM item_types WHERE id = $1)")
                                            .bind(&item_type)
                                            .fetch_one(&state.db)
                                            .await
                                        {
                                            Ok(true) => None,
                                            Ok(false) => Some("There's no such item".to_string()),
                                            Err(e) => Some(format!("Database error: {e}")),
                                        }
                                    };
                                    let msg = match refusal {
                                        Some(message) => ServerMessage::Error { message, command: command.into() },
                                        None => {
                                            market_items.insert(item_type.clone());
                                            // The depth as last published: the diffs already on
                                            // `event_rx` are at or below its `seq`, and the client skips them
                                            let depth = state.market.depth(&item_type);
                                            ServerMessage::MarketDepth { item_type, seq: depth.seq, full: true, levels: depth.levels }
                                        }
                                    };
                                    let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                }
                                ClientMessage::ClaimPresence => {
                                    // A fresh row, for when the avatar has to come back into
                                    // the world; one already there carries on as it is.
                                    let claimed = match load_player(&state.db, authed.account_id).await {
                                        Ok(player) => state
                                            .game
                                            .write()
                                            .await
                                            .claim_presence(player, connection_id)
                                            .map_err(|why| why.to_string()),
                                        Err(msg) => Err(msg),
                                    };
                                    if let Err(message) = claimed {
                                        let msg = ServerMessage::Error { message, command: command.into() };
                                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                    }
                                }
                                ClientMessage::UnsubscribeMarket { item_type } => match item_type {
                                    Some(item_type) => {
                                        market_items.remove(&item_type);
                                    }
                                    None => market_items.clear(),
                                },
                                ClientMessage::Move { x, y } => {
                                    // Only a destination on the world disc (W1, #194)
                                    if capitol_protocol::rules::in_world(x, y) {
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
                                        // Grass on someone's lot is theirs (#102)
                                        let refusal = crate::engine::container_access::gathering_refusal(
                                            &state.db, &state.game, player_id, node_uuid,
                                        ).await.unwrap_or_else(|e| Some(format!("Database error: {}", e)));
                                        // The best tool for it, if it takes one (E1.5)
                                        let tool = crate::engine::tools::tool_for_node(
                                            &state.db, &state.game, player_id, node_uuid,
                                        ).await;
                                        let (tool, refusal) = match tool {
                                            Ok(tool) => (tool, refusal),
                                            Err(e) => (None, refusal.or(Some(format!("Database error: {}", e)))),
                                        };
                                        if let Some(reason) = refusal {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::ExtractionFailed {
                                                    player_id,
                                                    reason,
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                        let mut game = state.game.write().await;
                                        game.queue_command(crate::engine::Command::StartExtraction {
                                            player_id,
                                            node_id: node_uuid,
                                            tool,
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

                                    let tool = match crate::engine::tools::tool_for_recipe(&state.db, &state.game, player_id, &recipe_id).await {
                                        Ok(tool) => tool,
                                        Err(e) => {
                                            tracing::error!("Tool lookup for {} failed: {:?}", player_id, e);
                                            None
                                        }
                                    };
                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::StartCrafting {
                                        player_id,
                                        recipe_id,
                                        inputs: items,
                                        tool,
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
                                                    command: command.into(),
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
                                                    command: command.into(),
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
                                                    command: command.into(),
                                                }).unwrap().into()
                                            )).await;
                                        }
                                    }
                                }
                                ClientMessage::SplitStack { item_id, quantity, to_slot, to_container_id, qualities } => {
                                    let parsed = Uuid::parse_str(&item_id)
                                        .map_err(|_| "Invalid item ID")
                                        .and_then(|item| match to_container_id.as_deref().map(Uuid::parse_str) {
                                            None => Ok((item, None)),
                                            Some(Ok(to)) => Ok((item, Some(to))),
                                            Some(Err(_)) => Err("Invalid container ID"),
                                        });
                                    let (item_uuid, to_container) = match parsed {
                                        Ok(ids) => ids,
                                        Err(message) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error {
                                                    message: message.to_string(),
                                                    command: command.into(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };
                                    match handle_split_stack(&state, player_id, item_uuid, quantity, to_container, to_slot, qualities.as_ref()).await {
                                        // Into another container, both changed (#190)
                                        Ok((source, target)) => {
                                            for container_id in moved_containers(Some(source), target) {
                                                let msg = ServerMessage::InventoryUpdated { player_id, container_id };
                                                let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                            }
                                        }
                                        Err(message) => {
                                            let msg = ServerMessage::Error { message, command: command.into() };
                                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
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
                                                    command: command.into(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    match crate::engine::station_placement::place_station(&state, player_id, &station_type, x, y, kit_uuid).await {
                                        Ok(result) => {
                                            result.remember(&state).await;
                                            let station_info = result.info;

                                            // Notify client
                                            let msg = ServerMessage::StationPlaced {
                                                station_id: station_info.id,
                                                station_type: station_info.station_type,
                                                name: station_info.name,
                                                owner_id: station_info.owner_id,
                                                x: station_info.x,
                                                y: station_info.y,
                                                container_id: station_info.container_id.unwrap_or(Uuid::nil()),
                                                quality: station_info.quality,
                                            };
                                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                                        }
                                        Err(e) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error { message: e.message(), command: command.into() }).unwrap().into()
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
                                                    command: command.into(),
                                                }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    match crate::engine::station_removal::remove_station(&state.db, player_id, station_uuid).await {
                                        Ok(removed) => {
                                            // Everyone sees it go, and the owner's inventory now holds the kit.
                                            let mut game = state.game.write().await;
                                            game.stations.remove(&station_uuid);
                                            let _ = game.event_sender.send(GameEvent::StationRemoved { station_id: station_uuid });
                                            let _ = game.event_sender.send(GameEvent::InventoryUpdated {
                                                player_id,
                                                container_id: removed.inventory_id,
                                                slots: vec![],
                                            });
                                        }
                                        Err(e) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error { message: e.message(), command: command.into() }).unwrap().into()
                                            )).await;
                                        }
                                    }
                                }
                                ClientMessage::InstallTool { station_id, item_id } => {
                                    let (Ok(station_uuid), Ok(item_uuid)) = (Uuid::parse_str(&station_id), Uuid::parse_str(&item_id)) else {
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::Error {
                                                message: "Invalid station or item ID".to_string(),
                                                command: command.into(),
                                            }).unwrap().into()
                                        )).await;
                                        continue;
                                    };
                                    let at = live_position(&state.game, player_id).await;
                                    let result = crate::engine::station_tools::install_tool(&state.db, player_id, station_uuid, item_uuid, at).await;
                                    announce_tool_slot(&state, &mut sender, player_id, station_uuid, command, result).await;
                                }
                                ClientMessage::RemoveTool { station_id } => {
                                    let Ok(station_uuid) = Uuid::parse_str(&station_id) else {
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::Error {
                                                message: "Invalid station ID".to_string(),
                                                command: command.into(),
                                            }).unwrap().into()
                                        )).await;
                                        continue;
                                    };
                                    let at = live_position(&state.game, player_id).await;
                                    let result = crate::engine::station_tools::remove_tool(&state.db, player_id, station_uuid, at).await;
                                    announce_tool_slot(&state, &mut sender, player_id, station_uuid, command, result).await;
                                }
                                ClientMessage::ConfigureAutomation { ref station_id, .. }
                                | ClientMessage::StartAutomation { ref station_id }
                                | ClientMessage::StopAutomation { ref station_id } => {
                                    let Ok(station_uuid) = Uuid::parse_str(station_id) else {
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::Error {
                                                message: "Invalid station ID".to_string(),
                                                command: command.into(),
                                            }).unwrap().into()
                                        )).await;
                                        continue;
                                    };
                                    let recipes = state.game.read().await.recipes.clone();
                                    let result = match &client_msg {
                                        ClientMessage::ConfigureAutomation { recipe_id, mode, .. } => {
                                            crate::engine::automation::configure(&state.db, &recipes, player_id, station_uuid, recipe_id, *mode).await
                                        }
                                        ClientMessage::StartAutomation { .. } => {
                                            crate::engine::automation::start(&state.db, &recipes, player_id, station_uuid).await
                                        }
                                        _ => crate::engine::automation::stop(&state.db, player_id, station_uuid).await,
                                    };
                                    match result {
                                        Ok(automation) => {
                                            let game = state.game.read().await;
                                            crate::engine::automation::announce(&game, station_uuid, automation);
                                        }
                                        Err(e) => {
                                            let msg = ServerMessage::Error { message: e.message(), command: command.into() };
                                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
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
                                                    command: command.into(),
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
                                                serde_json::to_string(&ServerMessage::Error { message: e, command: command.into() }).unwrap().into()
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

                                    // The same station rule as opening it (#80). The
                                    // tick checks range again, but not who may use it.
                                    let at = live_position(&state.game, player_id).await;
                                    if let Err(e) = check_station_access(&state.db, player_id, station_uuid, at).await {
                                        let _ = sender.send(Message::Text(
                                            serde_json::to_string(&ServerMessage::CraftingFailed {
                                                player_id,
                                                reason: e.message(),
                                            }).unwrap().into()
                                        )).await;
                                        continue;
                                    }

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

                                    // The station's own tool if it holds the right one (E1.6)
                                    let tool = match crate::engine::station_tools::tool_for_station_recipe(&state.db, &state.game, player_id, station_uuid, &recipe_id).await {
                                        Ok(tool) => tool,
                                        Err(e) => {
                                            tracing::error!("Tool lookup for {} failed: {:?}", player_id, e);
                                            None
                                        }
                                    };
                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::CraftAtStation {
                                        player_id,
                                        station_id: station_uuid,
                                        recipe_id,
                                        inputs: items,
                                        tool,
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
                                                    command: command.into(),
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
                                    let offer_lines = match validate_trade_offer(&state, player_id, &items, strands).await {
                                        Ok(lines) => lines,
                                        Err(message) => {
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&ServerMessage::Error { message, command: command.into() }).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    };

                                    let mut game = state.game.write().await;
                                    game.queue_command(crate::engine::Command::UpdateTradeOffer {
                                        player_id,
                                        items: offer_lines,
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
                                                    serde_json::to_string(&ServerMessage::Error { message: e, command: command.into() }).unwrap().into()
                                                )).await;
                                                (false, None)
                                            }
                                        }
                                    };

                                    if should_execute {
                                        // Execute trade with DB access
                                        match execute_trade(&state, player_id).await {
                                            Ok((trade_id, balances)) => {
                                                // Broadcast trade executed, and tell each side its balance
                                                let game = state.game.read().await;
                                                let _ = game.event_sender.send(GameEvent::TradeExecuted { trade_id });
                                                announce_balances(&game.event_sender, balances);
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
                    Some(Ok(Message::Close(_))) => {
                        counted.ends(SocketEnd::Closed);
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }

            // Handle game events
            heard = event_rx.recv() => {
                if !subscribed_position {
                    continue;
                }
                let event = match heard {
                    Ok(event) => Ok(event),
                    // Too far behind: the missed events are gone, so skip ahead
                    // and start this socket over from what's true now (C0.3)
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!("Player {player_id}'s connection {connection_id} fell {missed} events behind; resyncing it");
                        counted.resynced();
                        crate::engine::load::Receiver::Sockets.lagged(missed);
                        // The next `positions` tells it again what everyone in sight is doing
                        seen = None;
                        sees_everyone = false;
                        let messages = resync(&state.db, &state.game, &state.market, player_id, connection_id, missed, &market_items).await;
                        let mut sent = Ok(());
                        for msg in messages {
                            sent = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                            if sent.is_err() {
                                break;
                            }
                        }
                        if sent.is_err() {
                            counted.ends(SocketEnd::SendFailed);
                            break;
                        }
                        continue;
                    }
                    Err(RecvError::Closed) => Err(()),
                };

                match event {
                    // Only the players near this socket's (C0.4), each cell serialised once
                    // for every socket that sees it. A watcher sees round the avatar.
                    Ok(GameEvent::PlayerPositions(positions)) => {
                        let was_cell = std::mem::replace(&mut my_cell, positions.cell_of(player_id));
                        let arrived: Vec<Uuid> = positions
                            .come_into_sight(my_cell, seen.as_deref().map(|before| (before, was_cell)))
                            .into_iter()
                            .filter(|id| *id != player_id)
                            .collect();
                        sees_everyone = positions.sees_everyone(my_cell);
                        seen = Some(std::sync::Arc::clone(&positions));
                        if my_cell.is_none() && std::mem::replace(&mut sent_nobody, true) {
                            continue;
                        }
                        sent_nobody = my_cell.is_none();
                        if sender.send(Message::Text(positions.message(my_cell).into())).await.is_err() {
                            counted.ends(SocketEnd::SendFailed);
                            break;
                        }
                        // Someone come into sight part way through a gather or a craft (#275)
                        let mut sent = Ok(());
                        for msg in arrived.into_iter().flat_map(|id| positions.catch_up(id)) {
                            sent = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                            if sent.is_err() {
                                break;
                            }
                        }
                        if sent.is_err() {
                            counted.ends(SocketEnd::SendFailed);
                            break;
                        }
                    }
                    // Joining far off isn't news; the positions bring them when they're near
                    Ok(GameEvent::PlayerJoined(player)) => {
                        let cell = crate::engine::interest::cell_at(player.x, player.y);
                        if player.id == player_id || my_cell.is_none_or(|mine| crate::engine::interest::near(mine, cell)) {
                            let msg = ServerMessage::PlayerJoined { player };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    // Leaving goes to everyone, near or far: it ends their trades
                    Ok(GameEvent::PlayerLeft { player_id: left_id }) => {
                        let msg = ServerMessage::PlayerLeft { player_id: left_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    // M2: Extraction events, from the players in sight (#275)
                    Ok(GameEvent::ExtractionStarted { player_id: pid, node_id, duration_ticks }) => {
                        if pid == player_id || sees_everyone || seen.as_ref().is_some_and(|p| p.sees(my_cell, pid)) {
                            let msg = ServerMessage::ExtractionStarted { player_id: pid, node_id, duration_ticks };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::ExtractionProgress { player_id: pid, progress, duration }) => {
                        if pid == player_id || sees_everyone || seen.as_ref().is_some_and(|p| p.sees(my_cell, pid)) {
                            let msg = ServerMessage::ExtractionProgress { player_id: pid, progress, duration };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::ExtractionCompleted { player_id: pid, node_id, item_type, item_name, quantity, quality, tool }) => {
                        if pid == player_id || sees_everyone || seen.as_ref().is_some_and(|p| p.sees(my_cell, pid)) {
                            let msg = ServerMessage::ExtractionCompleted { player_id: pid, node_id, item_type, item_name, quantity, quality, tool };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::ExtractionCancelled { player_id: pid }) => {
                        if pid == player_id || sees_everyone || seen.as_ref().is_some_and(|p| p.sees(my_cell, pid)) {
                            let msg = ServerMessage::ExtractionCancelled { player_id: pid };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
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
                    // M3: Crafting events, from the players in sight (#275)
                    Ok(GameEvent::CraftingStarted { player_id: pid, operation_id, recipe_id, recipe_name, duration_ticks }) => {
                        if pid == player_id || sees_everyone || seen.as_ref().is_some_and(|p| p.sees(my_cell, pid)) {
                            let msg = ServerMessage::CraftingStarted { player_id: pid, operation_id, recipe_id, recipe_name, duration_ticks };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::CraftingProgress { player_id: pid, operation_id, progress, duration }) => {
                        if pid == player_id || sees_everyone || seen.as_ref().is_some_and(|p| p.sees(my_cell, pid)) {
                            let msg = ServerMessage::CraftingProgress { player_id: pid, operation_id, progress, duration };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::CraftingCompleted { player_id: pid, operation_id, item_type, item_name, quantity, quality, tool }) => {
                        if pid == player_id || sees_everyone || seen.as_ref().is_some_and(|p| p.sees(my_cell, pid)) {
                            let msg = ServerMessage::CraftingCompleted { player_id: pid, operation_id, item_type, item_name, quantity, quality, tool };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::CraftingCancelled { player_id: pid, operation_id }) => {
                        if pid == player_id || sees_everyone || seen.as_ref().is_some_and(|p| p.sees(my_cell, pid)) {
                            let msg = ServerMessage::CraftingCancelled { player_id: pid, operation_id };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
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
                    // M4: Station events
                    Ok(GameEvent::StationRemoved { station_id }) => {
                        let msg = ServerMessage::StationRemoved { station_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::StationToolChanged { station_id, tool }) => {
                        let msg = ServerMessage::StationToolChanged { station_id, tool };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::StationAutomationChanged { station_id, automation }) => {
                        let msg = ServerMessage::StationAutomationChanged { station_id, automation };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    // E4.3: a station working on its own, to its owner
                    Ok(GameEvent::AutomationProduced { owner_id, station_id, produced, done }) => {
                        if owner_id == player_id {
                            let msg = ServerMessage::AutomationProduced { station_id, produced, done };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::AutomationPaused { owner_id, station_id, station_type, reason, notice_id }) => {
                        if owner_id == player_id && take_notice(&state.db, notice_id).await {
                            let msg = ServerMessage::AutomationPaused { station_id, station_type, reason };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::StationDisplaced { owner_id, station_id, station_type, terrain_id, notice_id }) => {
                        if owner_id == player_id && take_notice(&state.db, notice_id).await {
                            let msg = ServerMessage::StationDisplaced { station_id, station_type, terrain_id };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::BazaarNotice { player_id: to, notice_id, message }) => {
                        if to == player_id && take_notice(&state.db, Some(notice_id)).await {
                            let _ = sender.send(Message::Text(serde_json::to_string(&message).unwrap().into())).await;
                        }
                    }
                    // M5: Trading events
                    Ok(GameEvent::TradeRequested { from_player, from_player_name, from_citizen, to_player }) => {
                        // Only send to the target player
                        if to_player == player_id {
                            let msg = ServerMessage::TradeRequested { from_player, from_player_name, from_citizen };
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
                    Ok(GameEvent::TradeRequestExpired { initiator, target }) => {
                        // To both: the asker stops waiting, the one asked loses the prompt
                        if player_id == initiator || player_id == target {
                            let msg = ServerMessage::TradeRequestExpired { from_player: initiator, to_player: target };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::TradeStarted { trade_id, player_a, player_b }) => {
                        // Send to both players
                        if player_id == player_a || player_id == player_b {
                            let partner_id = if player_id == player_a { player_b } else { player_a };
                            // Get partner name from game state
                            let (partner_name, partner_citizen) = {
                                let game = state.game.read().await;
                                game.players.get(&partner_id).map(|p| (p.name.clone(), p.citizen)).unwrap_or_default()
                            };
                            let msg = ServerMessage::TradeStarted { trade_id, partner_id, partner_name, partner_citizen };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::TradeOfferUpdated { trade_id, player_id: offering_player, items, strands }) => {
                        let ours = {
                            let game = state.game.read().await;
                            game.trade_manager.active_trades.get(&trade_id)
                                .is_some_and(|t| player_id == t.player_a || player_id == t.player_b)
                        };
                        if ours {
                            let msg = ServerMessage::TradeOfferUpdated {
                                trade_id,
                                player_id: offering_player,
                                items,
                                strands,
                            };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
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
                    Ok(GameEvent::TradeFailed { player_id: pid, command, reason }) => {
                        if pid == player_id {
                            let msg = ServerMessage::Error { message: reason, command: command.into() };
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
                    Ok(GameEvent::ToolBroke { player_id: pid, item_id, item_type, item_name }) => {
                        if pid == player_id {
                            let msg = ServerMessage::ToolBroke { item_id, item_type, item_name };
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
                    // B13: Plot ownership, to everyone
                    Ok(GameEvent::PlotClaimed { terrain_id, owner_id, claimed_at }) => {
                        let msg = ServerMessage::PlotClaimed { terrain_id, owner_id, claimed_at };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Ok(GameEvent::PlotReleased { terrain_id }) => {
                        let msg = ServerMessage::PlotReleased { terrain_id };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    // #131: Property tax, to the plot's owner only, if they take its notice (#138).
                    // A watcher sends it without taking it, leaving it for the holder.
                    Ok(GameEvent::TaxDelinquent { owner_id, terrain_id, tax_owed, forecloses_at, notice_id }) => {
                        if owner_id == player_id && (!holds(&state, player_id, connection_id).await || take_notice(&state.db, notice_id).await) {
                            let msg = ServerMessage::TaxDelinquent { terrain_id, tax_owed, forecloses_at };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::PlotForeclosed { owner_id, terrain_id, tax_owed, refund, impounded, lost, notice_id }) => {
                        if owner_id == player_id && (!holds(&state, player_id, connection_id).await || take_notice(&state.db, notice_id).await) {
                            let msg = ServerMessage::PlotForeclosed { terrain_id, tax_owed, refund, impounded, lost };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::SubscriptionLapsed { player_id: whose, until, notice_id }) => {
                        if whose == player_id && take_notice(&state.db, notice_id).await {
                            let msg = ServerMessage::SubscriptionLapsed { until };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    // E3.2: fills and expiries, to the order's owner
                    Ok(GameEvent::OrderFilled { player_id: whose, order, trade }) => {
                        if whose == player_id {
                            let msg = ServerMessage::OrderFilled { order, trade };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::OrderClosed { player_id: whose, order }) => {
                        if whose == player_id {
                            let msg = ServerMessage::OrderCancelled { order };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    // E3.3: placing, and the items' market channels
                    Ok(GameEvent::OrderPlaced { player_id: whose, order }) => {
                        if whose == player_id {
                            let msg = ServerMessage::OrderPlaced { order };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::MarketDepth { item_type, seq, levels }) => {
                        if market_items.contains(&item_type) {
                            let msg = ServerMessage::MarketDepth { item_type, seq, full: false, levels };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    Ok(GameEvent::MarketTrade { trade }) => {
                        if market_items.contains(&trade.item_type) {
                            let msg = ServerMessage::MarketTrade { trade };
                            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                        }
                    }
                    // E3.9: the Gazette, to everyone
                    Ok(GameEvent::GazettePublished { day, number, headline }) => {
                        let msg = ServerMessage::GazettePublished { day, number, headline };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    // D6.1: a fee moved, to everyone, so previews follow
                    Ok(GameEvent::FeesChanged { fees }) => {
                        let msg = ServerMessage::FeesChanged { fees };
                        let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
                    }
                    Err(()) => {
                        // Channel closed
                        counted.ends(SocketEnd::ServerStopping);
                        break;
                    }
                }
            }
        }
    }

    // Remove the player from the world, if this socket held their avatar
    let removed = {
        let mut game = state.game.write().await;
        game.disconnect_player(player_id, connection_id)
    };

    if removed {
        tracing::info!("Player {} disconnected", player_id);
    } else {
        tracing::info!("Player {}'s watching connection {} ended", player_id, connection_id);
    }
}

/// What a socket that fell `missed` events behind is sent before it carries on
/// (C0.3): `resync`, which tells the client to refetch what it fetches over
/// REST, then afresh what the server only pushes: the positions near it, the
/// player's balance, the depth of each item it watches, and, if it holds the
/// avatar, the notices waiting. A holder's trade and trade requests end first
/// ([`GameState::socket_lagged`](crate::engine::GameState::socket_lagged)),
/// and the cancels follow on the event stream.
async fn resync(
    db: &sqlx::PgPool,
    game: &tokio::sync::RwLock<crate::engine::GameState>,
    market: &crate::engine::market_matcher::Market,
    player_id: Uuid,
    connection_id: u64,
    missed: u64,
    market_items: &std::collections::HashSet<String>,
) -> Vec<ServerMessage> {
    let (players, holds) = {
        let mut game = game.write().await;
        game.socket_lagged(player_id, connection_id);
        let positions = game.positions();
        (positions.near(positions.cell_of(player_id)), game.holds_presence(player_id, connection_id))
    };
    let mut out = vec![ServerMessage::Resync { missed }, ServerMessage::Positions { players }];
    match sqlx::query_scalar::<_, i64>("SELECT strand_balance FROM players WHERE id = $1")
        .bind(player_id)
        .fetch_one(db)
        .await
    {
        Ok(new_balance) => out.push(ServerMessage::CurrencyChanged { new_balance }),
        Err(e) => tracing::error!("Failed to read {}'s balance to resync: {:?}", player_id, e),
    }
    let mut watched: Vec<&String> = market_items.iter().collect();
    watched.sort();
    for item_type in watched {
        let depth = market.depth(item_type);
        out.push(ServerMessage::MarketDepth { item_type: item_type.clone(), seq: depth.seq, full: true, levels: depth.levels });
    }
    if holds {
        let rate = game.read().await.tunables.get().property_tax_rate;
        match crate::engine::notices::take_all(db, player_id, rate).await {
            Ok(waiting) => out.extend(waiting),
            Err(e) => tracing::error!("Failed to load notices for {}: {:?}", player_id, e),
        }
    }
    out
}

/// Whether socket `connection_id` holds the player's avatar (E0.8).
async fn holds(state: &AppState, player_id: Uuid, connection_id: u64) -> bool {
    state.game.read().await.holds_presence(player_id, connection_id)
}

/// The most items one connection may watch the market channels of (E3.3)
const MARKET_CHANNELS: usize = 32;

/// Take a queued notice to send it live (#138). An event with no notice
/// (queuing it failed) is sent as it is.
async fn take_notice(db: &sqlx::PgPool, notice_id: Option<Uuid>) -> bool {
    match notice_id {
        None => true,
        Some(id) => crate::engine::notices::take(db, id).await.unwrap_or_else(|e| {
            tracing::error!("Failed to take notice {}: {:?}", id, e);
            true
        }),
    }
}

/// Who a socket's `auth` message signs in as.
struct Authed {
    account_id: Uuid,
    /// The token says so; a key never does, as `/admin` is closed to keys
    is_admin: bool,
    /// The API key it signed in with, if it did
    key: Option<KeyGrant>,
    /// Its session, or its key, for presence (E0.8)
    holder: Holder,
}

async fn authenticate(token: &str, state: &AppState) -> Result<Authed, String> {
    if !api_keys::is_secret(token) {
        let claims = validate_token(token, state).await?;
        return Ok(Authed {
            account_id: claims.sub,
            is_admin: claims.is_admin,
            key: None,
            holder: Holder::Session(claims.sid),
        });
    }
    match api_keys::authenticate(&state.db, token).await {
        // The town's report key (D4.1) can't so much as watch
        Ok(KeyAuth::Live(user)) if !user.grant.scopes.allows(ApiKeyScope::Read) => {
            Err(api_keys::refusal("connect", Some(ApiKeyScope::Read)))
        }
        Ok(KeyAuth::Live(user)) => Ok(Authed {
            account_id: user.account_id,
            is_admin: false,
            key: Some(user.grant),
            holder: Holder::Key(user.grant.id),
        }),
        Ok(KeyAuth::Unknown) => Err("Invalid or revoked API key".into()),
        Ok(KeyAuth::Unsubscribed(subscription)) => Err(api_keys::unsubscribed(&subscription)),
        // Retried by capitol-core, like any other database error (`auth_error_is_final`)
        Err(e) => Err(format!("Database error: {}", e)),
    }
}

async fn validate_token(token: &str, state: &AppState) -> Result<Claims, String> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| "Invalid token".to_string())?;

    // Verify the token's session is still the account's (sessions.rs)
    let mut redis = state.redis.clone();
    let current = crate::sessions::is_current(&mut redis, token_data.claims.sub, &token_data.claims.sid)
        .await
        .map_err(|e| {
            tracing::error!("Redis error checking a session: {:?}", e);
            SESSIONS_UNAVAILABLE.to_string()
        })?;

    if !current {
        return Err("Session expired".into());
    }

    Ok(token_data.claims)
}

/// The account's player as the game holds it, marked if it's a citizen's (C0.1)
pub(crate) async fn load_player(db: &sqlx::PgPool, account_id: Uuid) -> Result<PlayerState, String> {
    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        player: crate::models::Player,
        citizen: bool,
    }
    let row = sqlx::query_as::<_, Row>(
        "SELECT p.id, p.account_id, p.name, p.position_x, p.position_y, p.destination_x, p.destination_y,
                p.speed, p.strand_balance, p.current_zone, p.created_at, a.kind = 'citizen' AS citizen
         FROM players p JOIN accounts a ON a.id = p.account_id
         WHERE p.account_id = $1 LIMIT 1"
    )
    .bind(account_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .ok_or_else(|| "Player not found".to_string())?;

    let mut player: PlayerState = row.player.into();
    player.citizen = row.citizen;
    Ok(player)
}

/// Fetch player items by IDs (validates ownership)
async fn fetch_player_items(state: &AppState, player_id: Uuid, item_ids: &[Uuid]) -> Vec<crate::models::Item> {
    if item_ids.is_empty() {
        return Vec::new();
    }

    // Fetch items owned by this player from the provided IDs
    let mut items: Vec<crate::models::Item> = sqlx::query_as(&format!("SELECT {} FROM items WHERE id = ANY($1) AND owner_id = $2", crate::models::ITEM_COLUMNS))
        .bind(item_ids)
        .bind(player_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    // In the order asked for: a craft takes its units from the stacks in that
    // order, as the client's quality range assumes (`capitol-core` `craft::check`).
    items.sort_by_key(|item| item_ids.iter().position(|id| *id == item.id));
    items
}

/// Check a `trade_offer` before it reaches the trade session: parse its item ids, check its
/// amounts, and check the player holds every unit offered and has the strands. Each line is
/// settled to exact units here (the ones named, or the lowest), so the other player sees, and
/// gets, exactly those (E1.8). The `Err` is the message the client shows. `execute_trade`
/// checks them again at the end, since items can move between offer and accept.
async fn validate_trade_offer(
    state: &AppState,
    player_id: Uuid,
    items: &[TradeOfferItem],
    strands: i64,
) -> Result<Vec<TradeItemInfo>, String> {
    let offer = parse_trade_offer(items, strands)?;

    let item_ids: Vec<Uuid> = offer.iter().map(|line| line.0).collect();
    let owned: Vec<crate::models::Item> = if item_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(&format!("SELECT {} FROM items WHERE id = ANY($1) AND owner_id = $2", crate::models::ITEM_COLUMNS))
            .bind(&item_ids)
            .bind(player_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| {
                tracing::error!("Trade offer ownership check failed: {}", e);
                "Couldn't check your items; try again".to_string()
            })?
    };
    let owned: HashMap<Uuid, OwnedItem> =
        owned.into_iter().map(|item| (item.id, (item.item_type, item.units, item.maker.map(|m| m.0)))).collect();
    let held: HashMap<Uuid, Units> = owned.iter().map(|(id, (_, units, _))| (*id, units.clone())).collect();

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

    let settled = check_trade_offer_holdings(&offer, &held, strands, balance.map(|(b,)| b).unwrap_or(0))?;
    let game = state.game.read().await;
    Ok(trade_offer_lines(&settled, &owned, &game.item_types))
}

/// An item a trade offer names, as the player holds it: `(item_type, units, maker)`
type OwnedItem = (String, Units, Option<Maker>);

/// One line of an offer as parsed: the item, how many, and which units if named
type OfferLine = (Uuid, i32, Option<Units>);

/// The offer as `trade_offer_updated` reports it: each line with the item's type and name, and
/// the units offered and their mean. Run it after [`check_trade_offer_holdings`], which makes
/// sure every item is in `owned`. A type missing from `item_types` shows its id as its name.
fn trade_offer_lines(
    offer: &[(Uuid, Units)],
    owned: &HashMap<Uuid, OwnedItem>,
    item_types: &HashMap<String, ItemType>,
) -> Vec<TradeItemInfo> {
    offer
        .iter()
        .filter_map(|(id, units)| {
            let (item_type, _, maker) = owned.get(id)?;
            Some(TradeItemInfo {
                item_id: *id,
                item_type: item_type.clone(),
                item_name: item_types.get(item_type).map_or_else(|| item_type.clone(), |t| t.name.clone()),
                quantity: units.count() as i32,
                quality: units.mean()?,
                units: units.clone(),
                maker: maker.clone(),
            })
        })
        .collect()
}

/// The parts of a `trade_offer` that need no database: ids parse, quantities and strands make
/// sense, named units add up to the quantity, and no item appears twice.
fn parse_trade_offer(items: &[TradeOfferItem], strands: i64) -> Result<Vec<OfferLine>, String> {
    if strands < 0 {
        return Err("You can't offer a negative amount of strands".to_string());
    }
    let mut offer: Vec<OfferLine> = Vec::with_capacity(items.len());
    for item in items {
        let id = Uuid::parse_str(&item.item_id)
            .map_err(|_| format!("Invalid item ID: {}", item.item_id))?;
        if item.quantity <= 0 {
            return Err("Offered quantities must be at least 1".to_string());
        }
        if item.qualities.as_ref().is_some_and(|q| q.count() != item.quantity as i64) {
            return Err("The qualities offered don't add up to the quantity".to_string());
        }
        if offer.iter().any(|(seen, _, _)| *seen == id) {
            return Err("The same item is in the offer twice".to_string());
        }
        offer.push((id, item.quantity, item.qualities.clone()));
    }
    Ok(offer)
}

/// Check the player holds everything they offer, and settle each line to exact units: the
/// ones named, or the lowest (`Pick::default`). `held` maps the offered item ids the player
/// owns to their units.
fn check_trade_offer_holdings(
    offer: &[OfferLine],
    held: &HashMap<Uuid, Units>,
    strands: i64,
    strand_balance: i64,
) -> Result<Vec<(Uuid, Units)>, String> {
    let mut settled = Vec::with_capacity(offer.len());
    for (id, qty, named) in offer {
        let Some(have) = held.get(id) else {
            return Err("You don't own an item in that offer".to_string());
        };
        let units = match named {
            Some(named) if have.contains(named) => named.clone(),
            Some(_) => return Err("You don't hold those qualities of an item you offered".to_string()),
            None => have.pick(*qty, Pick::default()).ok_or_else(|| {
                format!("You only have {} of an item you offered {} of", have.count(), qty)
            })?,
        };
        settled.push((*id, units));
    }
    if strands > strand_balance {
        return Err(format!(
            "You only have {} strands, not the {} offered",
            strand_balance, strands
        ));
    }
    Ok(settled)
}

/// Handle move item request directly via database
async fn handle_move_item(
    state: &AppState,
    player_id: Uuid,
    item_id: Uuid,
    target_container_id: Uuid,
    target_slot: i32,
) -> Result<Option<Uuid>, String> {
    // The player's own item, or a deposit in their own station's box (#80)
    let source_item = movable_item(&state.db, player_id, item_id)
        .await
        .map_err(|e| e.message())?;

    // Both ends must be usable: the player's own containers, or a station's
    // within reach (B4). The source matters too, since a swap fills it.
    let at = live_position(&state.game, player_id).await;
    let source_container = match source_item.container_id {
        Some(id) => Some(check_container_access(&state.db, player_id, id, at).await.map_err(|e| e.message())?),
        None => None,
    };
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
    let target_item: Option<crate::models::Item> = sqlx::query_as(&format!("SELECT {} FROM items WHERE container_id = $1 AND slot_index = $2", crate::models::ITEM_COLUMNS))
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
    // Things only come out of the impound (#141), and out of the market box
    // only at the Exchange hall (#244)
    let source_type = source_container.as_ref().map(|c| c.container_type.as_str());
    let types = (source_type, target_container.container_type.as_str());
    if let Some(why) = move_refusal(&state.game, &state.db, player_id, at, types, target_item.is_some())
        .await
        .map_err(|e| format!("Database error: {}", e))?
    {
        return Err(why.into());
    }

    crate::engine::slot_moves::apply_move(
        &state.db,
        player_id,
        &source_item,
        target_container_id,
        target_slot,
        target_item.as_ref(),
    )
    .await
    .map_err(|e| format!("Database error: {}", e))?;
    Ok(source_item.container_id)
}

/// Split `quantity` off a stack into the empty `to_slot` of its container,
/// or of `to_container` (E1.2, #190): the units `qualities` names, or the
/// lowest (E1.8). Both containers are checked as a move's are. Returns the
/// source's container and the target's, which may be the same.
async fn handle_split_stack(
    state: &AppState,
    player_id: Uuid,
    item_id: Uuid,
    quantity: i32,
    to_container: Option<Uuid>,
    to_slot: i32,
    qualities: Option<&Units>,
) -> Result<(Uuid, Uuid), String> {
    let source = movable_item(&state.db, player_id, item_id)
        .await
        .map_err(|e| e.message())?;
    let container_id = source
        .container_id
        .ok_or_else(|| "That item isn't in a container".to_string())?;
    let at = live_position(&state.game, player_id).await;
    let from = check_container_access(&state.db, player_id, container_id, at)
        .await
        .map_err(|e| e.message())?;
    let to_container = to_container.unwrap_or(container_id);
    let to = if to_container != container_id {
        check_container_access(&state.db, player_id, to_container, at)
            .await
            .map_err(|e| e.message())?
    } else {
        from.clone()
    };
    // Things only come out of the impound (#141), and out of the market box
    // only at the Exchange hall (#244); a split goes only to an empty slot
    let types = (Some(from.container_type.as_str()), to.container_type.as_str());
    if let Some(why) = move_refusal(&state.game, &state.db, player_id, at, types, false)
        .await
        .map_err(|e| format!("Database error: {}", e))?
    {
        return Err(why.into());
    }
    crate::engine::slot_moves::split(&state.db, player_id, &source, quantity, to_container, to_slot, qualities)
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .map_err(|e| e.message().to_string())?;
    Ok((container_id, to_container))
}

/// The containers a move changed, each once: the target, then the source if
/// it's a different one. A move, merge or swap empties or fills the source
/// slot, so a client refetching by `container_id` needs both.
fn moved_containers(source: Option<Uuid>, target: Uuid) -> Vec<Uuid> {
    let mut changed = vec![target];
    changed.extend(source.filter(|&s| s != target));
    changed
}

/// Answer `install_tool` or `remove_tool` (E1.6): everyone hears what the
/// station's slot holds now, and the player's inventory refetches. A refusal
/// goes to the player alone.
async fn announce_tool_slot(
    state: &AppState,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    player_id: Uuid,
    station_id: Uuid,
    command: &str,
    result: Result<crate::engine::station_tools::SlotChange, crate::engine::station_tools::ToolSlotError>,
) {
    match result {
        Ok(change) => {
            let game = state.game.read().await;
            crate::engine::station_tools::announce(&game.event_sender, station_id, change.tool);
            let _ = game.event_sender.send(GameEvent::InventoryUpdated {
                player_id,
                container_id: change.inventory_id,
                slots: vec![],
            });
        }
        Err(e) => {
            let msg = ServerMessage::Error { message: e.message(), command: command.into() };
            let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
        }
    }
}

/// Handle open station request
async fn handle_open_station(
    state: &AppState,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<(String, String, Uuid), String> {
    // The one station rule: its owner, the plot's owner, or anyone the plot's
    // station_use_level admits, within range (#80)
    let at = live_position(&state.game, player_id).await;
    check_station_access(&state.db, player_id, station_id, at)
        .await
        .map_err(|e| e.message())?;

    let (station_type, name): (String, String) = sqlx::query_as(
        "SELECT st.id, st.name FROM stations s
         JOIN station_types st ON st.id = s.station_type
         WHERE s.id = $1"
    )
    .bind(station_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

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

    Ok((station_type, name, container.id))
}

/// Execute a trade atomically
/// Transfers items and strands between both players
/// Returns the trade's id, and each side's new balance if strands changed hands.
async fn execute_trade(state: &AppState, player_id: Uuid) -> Result<(Uuid, Vec<(Uuid, i64)>), String> {
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

    // Verify every item offered still exists, is its offerer's, and holds the units offered
    for (items, owner) in [(&session.offer_a.items, player_a), (&session.offer_b.items, player_b)] {
        for (item_id, units) in items {
            let owned: Option<(i32,)> = sqlx::query_as(
                "SELECT 1 FROM items WHERE id = $1 AND owner_id = $2 FOR UPDATE"
            )
            .bind(item_id)
            .bind(owner)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
            let held = crate::engine::units::units_of(&mut tx, *item_id)
                .await
                .map_err(|e| format!("DB error: {}", e))?;
            if owned.is_none() || !held.contains(units) {
                return Err("Item no longer available".to_string());
            }
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
        return Err("Player A has insufficient strands".to_string());
    }
    if balance_b.0 < session.offer_b.strands {
        return Err("Player B has insufficient strands".to_string());
    }

    // Hand the items over. Each stack leaves its container first (a split
    // takes a new stack off it), and only then is each placed in its new
    // owner's inventory, so a swap can use the slots it frees (#95). An
    // inventory with no room fails the trade, and the transaction rolls back.
    let mut placing: Vec<(Uuid, Uuid)> = Vec::new();
    for (items, to) in [(&session.offer_a.items, player_b), (&session.offer_b.items, player_a)] {
        for (item_id, units) in items {
            placing.push((hand_over(&mut tx, *item_id, units, to).await?, to));
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

    let balances = if net_strands != 0 {
        vec![(player_a, new_balance_a), (player_b, new_balance_b)]
    } else {
        Vec::new()
    };
    Ok((trade_id, balances))
}

/// Gives `units` of the stack `item_id` to `to`, out of any container: the
/// whole stack, or a new stack split off it, each unit at its own quality
/// (E1.8). Returns the stack `to` now owns, for [`place_in_inventory`].
async fn hand_over(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    item_id: Uuid,
    units: &Units,
    to: Uuid,
) -> Result<Uuid, String> {
    let db_error = |e: sqlx::Error| format!("DB error: {}", e);
    let item_type: String = sqlx::query_scalar("SELECT item_type FROM items WHERE id = $1")
        .bind(item_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_error)?;
    let held = crate::engine::units::units_of(tx, item_id).await.map_err(db_error)?;
    if held == *units {
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
    let taken = crate::engine::units::take_units(tx, item_id, units)
        .await
        .map_err(db_error)?
        .ok_or_else(|| "Item no longer available".to_string())?;
    crate::engine::units::insert_stack(
        tx,
        crate::engine::units::NewStack {
            item_type: &item_type,
            owner_id: Some(to),
            container_id: None,
            slot_index: None,
            durability: None,
        },
        &taken.marked,
    )
    .await
    .map_err(db_error)
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
        TradeOfferItem { item_id: id.to_string(), quantity, qualities: None }
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
        assert_eq!(offer, vec![(id, 4, None)]);
    }

    #[test]
    fn trade_offer_rejects_qualities_that_dont_add_up() {
        let id = Uuid::new_v4().to_string();
        let mut line = offer_item(&id, 4);
        line.qualities = Some(Units::uniform(3, 90));
        assert!(parse_trade_offer(&[line], 0).unwrap_err().contains("add up"));
    }

    #[test]
    fn trade_offer_rejects_an_item_not_owned() {
        let offer = vec![(Uuid::new_v4(), 1, None)];
        let err = check_trade_offer_holdings(&offer, &HashMap::new(), 0, 0).unwrap_err();
        assert!(err.contains("don't own"), "{err}");
    }

    #[test]
    fn trade_offer_rejects_more_than_the_player_has() {
        let id = Uuid::new_v4();
        let held = HashMap::from([(id, Units::uniform(2, 60))]);
        let err = check_trade_offer_holdings(&[(id, 5, None)], &held, 0, 0).unwrap_err();
        assert!(err.contains("only have 2"), "{err}");
    }

    #[test]
    fn trade_offer_rejects_qualities_the_player_doesnt_hold() {
        let id = Uuid::new_v4();
        let held = HashMap::from([(id, Units::from_pairs([(50, 2), (90, 1)]))]);
        let err = check_trade_offer_holdings(&[(id, 2, Some(Units::uniform(2, 90)))], &held, 0, 0).unwrap_err();
        assert!(err.contains("qualities"), "{err}");
    }

    #[test]
    fn trade_offer_rejects_more_strands_than_the_balance() {
        let err = check_trade_offer_holdings(&[], &HashMap::new(), 50, 20).unwrap_err();
        assert!(err.contains("only have 20 strands"), "{err}");
    }

    /// An offer settles to exact units: the ones named, or the lowest first
    #[test]
    fn trade_offer_settles_to_the_units_named_or_the_lowest() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mixed = Units::from_pairs([(50, 2), (90, 2)]);
        let held = HashMap::from([(a, mixed.clone()), (b, mixed)]);
        let settled = check_trade_offer_holdings(
            &[(a, 2, Some(Units::uniform(2, 90))), (b, 3, None)],
            &held,
            20,
            20,
        )
        .unwrap();
        assert_eq!(settled, vec![(a, Units::uniform(2, 90)), (b, Units::from_pairs([(50, 2), (90, 1)]))]);
    }

    #[test]
    fn trading_offer_lines_carry_type_name_and_units() {
        let (rope, mystery) = (Uuid::new_v4(), Uuid::new_v4());
        let owned = HashMap::from([
            (rope, ("rope".to_string(), Units::from_pairs([(80, 4), (86, 3)]), Some(ada()))),
            (mystery, ("relic".to_string(), Units::uniform(1, 40), None)),
        ]);
        let item_types = HashMap::from([(
            "rope".to_string(),
            ItemType { id: "rope".into(), name: "Rope".into(), category: "material".into(), stackable: true, weight: 1.0, ..Default::default() },
        )]);
        let offered = Units::from_pairs([(80, 1), (86, 2)]);

        let lines = trade_offer_lines(&[(rope, offered.clone()), (mystery, Units::uniform(1, 40))], &owned, &item_types);
        assert_eq!(lines, vec![
            TradeItemInfo { item_id: rope, item_type: "rope".into(), item_name: "Rope".into(), quantity: 3, quality: 84, units: offered, maker: Some(ada()) },
            // An unknown type keeps its id as its name
            TradeItemInfo { item_id: mystery, item_type: "relic".into(), item_name: "relic".into(), quantity: 1, quality: 40, units: Units::uniform(1, 40), maker: None },
        ]);
    }

    fn ada() -> Maker {
        Maker { name: "Ada".into(), citizen: false }
    }

    /// A socket flooded past the event channel's capacity hears that it lagged,
    /// not that the channel closed, is sent a resync with what's true now, and
    /// carries on hearing live events (C0.3)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_socket_flooded_past_the_channel_resyncs_and_stays_up(db: sqlx::PgPool) {
        use crate::engine::market_matcher::Market;
        use crate::engine::presence::Holder;
        use crate::models::{TerrainConfig, WorldGenParams};

        let ada = crate::engine::plot_sync::tests::player(&db, "ada", 1234).await;
        let gone = ServerMessage::PlotForeclosed { terrain_id: "lot".into(), tax_owed: 80, refund: 20, impounded: vec![], lost: vec![] };
        crate::engine::notices::queue(&db, ada, "lot", &gone).await.unwrap();
        let game = tokio::sync::RwLock::new(crate::engine::GameState::with_terrain(
            WorldGenParams::default(),
            TerrainConfig::get_default(),
        ));
        let account: Uuid = sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1")
            .bind(ada)
            .fetch_one(&db)
            .await
            .unwrap();
        let player = load_player(&db, account).await.unwrap();
        let (socket, mut rx) = {
            let mut game = game.write().await;
            let (socket, _) = game.open_socket(ada, Holder::Session("desk".into()), true);
            game.claim_presence(player, socket).unwrap();
            (socket, game.subscribe())
        };
        let market = Market::default();
        let watched: std::collections::HashSet<String> = ["plank".to_string()].into();

        // The game runs on while the socket's sends are stuck
        let events = game.read().await.event_sender.clone();
        for _ in 0..crate::engine::GameState::EVENT_CAPACITY + 100 {
            events.send(GameEvent::TerrainUpdated).unwrap();
        }
        let missed = match rx.recv().await {
            Err(RecvError::Lagged(missed)) => missed,
            other => panic!("a flooded socket heard {other:?}, not that it lagged"),
        };
        assert_eq!(missed, 100);

        let sent = resync(&db, &game, &market, ada, socket, missed, &watched).await;
        let tags: Vec<&str> = sent.iter().map(ServerMessage::type_name).collect();
        assert_eq!(tags, ["resync", "positions", "currency_changed", "market_depth", "plot_foreclosed"]);
        assert_eq!(sent[0], ServerMessage::Resync { missed: 100 });
        assert!(matches!(&sent[1], ServerMessage::Positions { players } if players.iter().any(|p| p.id == ada)));
        assert_eq!(sent[2], ServerMessage::CurrencyChanged { new_balance: 1234 });
        assert!(matches!(&sent[3], ServerMessage::MarketDepth { item_type, full: true, .. } if item_type == "plank"));
        assert_eq!(sent[4], gone);
        assert!(crate::engine::notices::take_all(&db, ada, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap().is_empty(), "the notice was left to go again");

        // Skipped ahead: what the channel still holds, then what comes next
        for _ in 0..crate::engine::GameState::EVENT_CAPACITY {
            assert!(matches!(rx.recv().await, Ok(GameEvent::TerrainUpdated)));
        }
        events.send(GameEvent::GazettePublished { day: 1, number: 1, headline: "On".into() }).unwrap();
        assert!(matches!(rx.recv().await, Ok(GameEvent::GazettePublished { .. })));
    }

    /// A trade hands over exactly the units offered, part of a stack or all of it (E1.8)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_trade_hands_over_exactly_the_units_offered(db: sqlx::PgPool) {
        use crate::engine::units::test_units::{all_held, marked, marked_stack_of, stack_of, units};
        use crate::engine::units::Marked;
        let mut players = Vec::new();
        for name in ["ada", "bob"] {
            let account = Uuid::new_v4();
            sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
                .bind(account)
                .bind(format!("{name}@test"))
                .execute(&db)
                .await
                .unwrap();
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(account)
                .bind(name)
                .execute(&db)
                .await
                .unwrap();
            players.push(id);
        }
        let (ada, bob) = (players[0], players[1]);
        let inventory = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(inventory)
            .bind(ada)
            .execute(&db)
            .await
            .unwrap();
        let mixed = Units::from_pairs([(40, 2), (92, 6)]);
        // Ada gathered all but the two 40s herself (#231)
        let marks = Marked::from_rows([(40, None, 2), (92, Some(ada), 6)]);
        let fiber = marked_stack_of(&db, ada, inventory, 0, "fiber", &marks).await;
        let rope = stack_of(&db, ada, inventory, 1, "rope", &Units::from_pairs([(55, 1), (80, 1)])).await;

        let mut tx = db.begin().await.unwrap();
        let part = hand_over(&mut tx, fiber, &Units::uniform(4, 92), bob).await.unwrap();
        let whole = hand_over(&mut tx, rope, &Units::from_pairs([(55, 1), (80, 1)]), bob).await.unwrap();
        tx.commit().await.unwrap();

        let owner = |id: Uuid| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, Uuid>("SELECT owner_id FROM items WHERE id = $1")
                    .bind(id)
                    .fetch_one(&db)
                    .await
                    .unwrap()
            }
        };
        assert_eq!((owner(part).await, units(&db, part).await), (bob, Units::uniform(4, 92)));
        assert_eq!((owner(fiber).await, units(&db, fiber).await), (ada, Units::from_pairs([(40, 2), (92, 2)])));
        assert_eq!(whole, rope);
        assert_eq!(owner(rope).await, bob);
        let mut all = all_held(&db, ada, "fiber").await;
        all.merge(&all_held(&db, bob, "fiber").await);
        assert_eq!(all, mixed, "no unit lost its quality");
        assert_eq!(marked(&db, part).await, Marked::by(Some(ada), &Units::uniform(4, 92)), "nor its maker's mark");
    }
}
