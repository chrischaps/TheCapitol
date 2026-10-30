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
    check_container_access, check_station_use, live_position, movable_item, move_refusal, StationUse,
};
use crate::api_keys::{self, KeyAuth, KeyGrant};
use crate::engine::GameEvent;
use crate::engine::load::{self, SocketEnd};
use crate::engine::presence::{self, Holder, PresenceEvent};
use crate::engine::currency::{add_strands_in, announce_balances, subtract_strands_in, TransactionType};
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
    // What it knows of its player between messages (#463)
    let mut socket = Socket::new(&state, &player, authed.account_id, key, connection_id);
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
        day_offset_ms: state.day_offset_ms(),
    };
    let _ = sender
        .send(Message::Text(serde_json::to_string(&auth_ok).unwrap().into()))
        .await;
    // Where they are on the road in (#438), straight after
    match crate::engine::tutorial::progress(&state.db, player_id).await {
        Ok(progress) => {
            let tutorial = crate::engine::tutorial::message(progress);
            let _ = sender.send(Message::Text(serde_json::to_string(&tutorial).unwrap().into())).await;
            // The Registrar's letter, if People has opened and it never went
            // (#447). It's queued, so the first subscribe sends it.
            if progress.done.chapter_open(capitol_protocol::tutorial::Chapter::People) && !progress.finished() {
                crate::engine::tutorial::registrar_writes(&state.db, None, player_id).await;
            }
        }
        Err(e) => tracing::error!("Failed to read {}'s tutorial: {:?}", player_id, e),
    }
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

    loop {
        tokio::select! {
            // This socket took the avatar or lost it (E0.8), or a logout over REST ended
            // its session (#78)
            event = presence_events.recv() => {
                match event {
                    Some(PresenceEvent::Changed { yours, held_by }) => {
                        socket.holds_avatar = yours;
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
                            for msg in socket.handle(&state, client_msg).await {
                                let _ = sender.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await;
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

            // Handle game events (#566: `Socket::heard`)
            heard = event_rx.recv() => {
                let event = match heard {
                    Ok(event) => event,
                    // Too far behind: the missed events are gone (C0.3)
                    Err(RecvError::Lagged(_)) if !socket.subscribed_position => continue,
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!("Player {player_id}'s connection {connection_id} fell {missed} events behind; resyncing it");
                        counted.resynced();
                        let mut sent = Ok(());
                        for msg in socket.resynced(&state, missed).await {
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
                    Err(RecvError::Closed) => {
                        counted.ends(SocketEnd::ServerStopping);
                        break;
                    }
                };
                // A `positions` that can't be sent ends the socket; any other
                // send's failure shows on the next read
                let must_send = matches!(event, GameEvent::PlayerPositions(_));
                let mut sent = Ok(());
                for out in socket.heard(&state, event).await {
                    let text = match out {
                        Outgoing::Message(msg) => serde_json::to_string(&msg).unwrap(),
                        Outgoing::Text(text) => text,
                    };
                    sent = sender.send(Message::Text(text.into())).await;
                    if sent.is_err() && must_send {
                        break;
                    }
                }
                if sent.is_err() && must_send {
                    counted.ends(SocketEnd::SendFailed);
                    break;
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

/// What a socket sends: a message, or a `positions` already serialised once
/// for every socket in its cell (C0.4)
/// (It lives a moment, so a message is kept whole rather than boxed.)
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum Outgoing {
    Message(ServerMessage),
    Text(String),
}

/// What one socket knows of its player between messages (#463): who it is,
/// what it has asked to hear, and what's in sight. The socket's loop keeps
/// it, and `handle` and `heard` read and change it, so a test can drive it
/// without a socket.
pub(crate) struct Socket {
    player_id: Uuid,
    account_id: Uuid,
    connection_id: u64,
    /// The API key it signed in with, if it did
    key: Option<KeyGrant>,
    /// Who says this socket's lines, and from where while the avatar's away (S1.1)
    name: String,
    citizen: bool,
    home: (f64, f64),
    /// This connection's command allowance (E0.7)
    limit_key: crate::rate_limit::Key,
    subscribed_position: bool,
    /// The items whose market channel this connection watches (E3.3)
    market_items: std::collections::HashSet<String>,
    /// Whether this socket holds the avatar, as its last `presence` said: only
    /// then does it hear local talk (Commons D1)
    holds_avatar: bool,
    /// The newest Capitol line this socket has, from the backlog or since
    heard_capitol: u64,
    /// Who this socket's player has muted (S1.4): read on subscribe, and
    /// again whenever they mute or unmute someone, on any device
    muted: std::collections::HashSet<Uuid>,
    /// The guild this socket's player is in (S4.4), whose talk it hears: read
    /// on subscribe, and again whenever their guild changes
    my_guild: Option<Uuid>,
    /// The interest cell this socket's player was in at the last `positions`,
    /// and whether the last one sent was empty, so one out of the world isn't
    /// sent an empty list ten times a second (C0.4)
    my_cell: Option<crate::engine::interest::Cell>,
    sent_nobody: bool,
    /// The last `positions`, with `my_cell`: only the gathering and crafting of
    /// the players in sight in it reach this socket (#275)
    seen: Option<std::sync::Arc<crate::engine::interest::Positions>>,
    /// Whether everyone in the world was in sight in it, so nothing needs filtering
    sees_everyone: bool,
}

impl Socket {
    fn new(state: &AppState, player: &PlayerState, account_id: Uuid, key: Option<KeyGrant>, connection_id: u64) -> Self {
        Socket {
            player_id: player.id,
            account_id,
            connection_id,
            key,
            name: player.name.clone(),
            citizen: player.citizen,
            home: (player.x, player.y),
            limit_key: state.limiter.connection(),
            subscribed_position: false,
            market_items: Default::default(),
            holds_avatar: false,
            heard_capitol: 0,
            muted: Default::default(),
            my_guild: None,
            my_cell: None,
            sent_nobody: false,
            seen: None,
            sees_everyone: false,
        }
    }

    /// One message from this socket's client (#463): what it may do, judged,
    /// and done. What goes back to this socket alone is returned, in order;
    /// what others hear goes out on the game's events.
    async fn handle(&mut self, state: &AppState, client_msg: ClientMessage) -> Vec<ServerMessage> {
        let (player_id, connection_id) = (self.player_id, self.connection_id);
        let mut out = Vec::new();
        // Every `error` below names the command it refuses (#89).
        let command = client_msg.type_name();
        // Talk has its own bucket, so it never costs a step (Commons D2),
        // and so does steering, so a turn never costs a command (controls T0.3)
        let bucket = crate::rate_limit::Key::of_ws(self.limit_key, &client_msg);
        let allowance = bucket.map_or(Ok(()), |bucket| state.limiter.check(bucket));
        if allowance.is_err() && matches!(client_msg, ClientMessage::Steer { .. }) {
            // Dropped unanswered: the client never waits on a steer
            tracing::debug!("Rate limited WS steer from player {player_id}");
            return out;
        }
        if let Err(wait) = allowance {
            tracing::info!("Rate limited WS {command} from player {player_id}");
            out.push(ServerMessage::Error {
                    message: crate::rate_limit::ws_message(wait),
                    command: command.into(),
                });
            return out;
        }
        if let Some(key) = self.key {
            let needed = api_keys::ws_scope(&client_msg);
            if !key.scopes.allows(needed) {
                let msg = ServerMessage::Error {
                    message: api_keys::refusal(command, Some(needed)),
                    command: command.to_string(),
                };
                out.push(msg);
                return out;
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
            out.push(msg);
            return out;
        }
        match client_msg {
            ClientMessage::Subscribe { channel } => {
                if channel == "position" {
                    let first = !self.subscribed_position;
                    self.subscribed_position = true;
                    out.push(ServerMessage::Subscribed { channel });
                    // What waited while they were away (#138). Events since
                    // auth are already on `event_rx`; whichever path takes a
                    // notice first sends it. A watcher takes none.
                    if first && state.game.read().await.holds_presence(player_id, connection_id) {
                        let rate = state.tunables.get().property_tax_rate;
                        match crate::engine::notices::take_all(&state.db, player_id, rate).await {
                            Ok(waiting) => {
                                for msg in waiting {
                                    out.push(msg);
                                }
                            }
                            Err(e) => tracing::error!("Failed to load notices for {}: {:?}", player_id, e),
                        }
                    }
                    // The Capitol channel's last lines (S1.1), but the muted's
                    // (S1.4); any still on `event_rx` are numbered at or
                    // below `heard_capitol`
                    if first {
                        self.muted = read_mutes(state, player_id, std::mem::take(&mut self.muted)).await;
                        self.my_guild = read_guild(state, player_id, self.my_guild).await;
                        let (seq, lines) = state.talk.backlog(&self.muted);
                        self.heard_capitol = seq;
                        for msg in lines {
                            out.push(msg);
                        }
                    }
                }
            }
            ClientMessage::SubscribeMarket { item_type } => {
                let refusal = if self.market_items.len() >= MARKET_CHANNELS && !self.market_items.contains(&item_type) {
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
                        self.market_items.insert(item_type.clone());
                        // The depth as last published: the diffs already on
                        // `event_rx` are at or below its `seq`, and the client skips them
                        let depth = state.market.depth(&item_type);
                        ServerMessage::MarketDepth { item_type, seq: depth.seq, full: true, levels: depth.levels }
                    }
                };
                out.push(msg);
            }
            ClientMessage::ClaimPresence => {
                // A fresh row, for when the avatar has to come back into
                // the world; one already there carries on as it is.
                let claimed = match load_player(&state.db, self.account_id).await {
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
                    out.push(msg);
                }
            }
            // Talk (S1.1): the avatar's place now, or where it was last
            ClientMessage::Say { channel, text } => {
                let (events, (x, y)) = {
                    let game = state.game.read().await;
                    let at = game.players.get(&player_id).map_or(self.home, |p| (p.x, p.y));
                    (game.event_sender.clone(), at)
                };
                let speaker = crate::engine::talk::Speaker {
                    player_id,
                    account_id: self.account_id,
                    name: self.name.clone(),
                    citizen: self.citizen,
                    x,
                    y,
                    holds_avatar: self.holds_avatar,
                    guild: self.my_guild,
                };
                match state.talk.say(&events, &state.limiter, speaker, channel, &text) {
                    // Judged here: `tests::a_say_counts_for_the_road_in` (#463)
                    Ok(()) => {
                        crate::engine::tutorial::saw(&state.db, &events, player_id, crate::engine::tutorial::Did::Said(channel)).await;
                    }
                    Err(message) => {
                        let msg = ServerMessage::Error { message, command: command.into() };
                        out.push(msg);
                    }
                }
            }
            // What only the screen shows (#441, plan D7): taken for the
            // steps the client may report, and refused for the rest. The
            // market box's waiver is judged too (#446): `tutorial::seen`
            ClientMessage::TutorialSeen { step } => {
                let events = state.game.read().await.event_sender.clone();
                if let Err(message) = crate::engine::tutorial::seen(&state.db, &events, player_id, step).await {
                    let msg = ServerMessage::Error { message, command: command.into() };
                    out.push(msg);
                }
            }
            ClientMessage::UnsubscribeMarket { item_type } => match item_type {
                Some(item_type) => {
                    self.market_items.remove(&item_type);
                }
                None => self.market_items.clear(),
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
            // Walk on a heading (T0.2): the tick checks `at` against where
            // the player is
            ClientMessage::Steer { heading, throttle, at } => {
                state.game.write().await.queue_command(crate::engine::Command::Steer {
                    player_id,
                    heading,
                    throttle,
                    at: at.map(|[x, y]| (x, y)),
                });
            }
            ClientMessage::Extract { node_id } => {
                if let Ok(node_uuid) = Uuid::parse_str(&node_id) {
                    // Grass on someone's lot is theirs (#102), unless
                    // they're on a wage shift for it (Commons D7)
                    let (wage, refusal) = match crate::engine::container_access::gathering_terms(
                        &state.db, &state.game, player_id, node_uuid,
                    ).await {
                        Ok(Ok(wage)) => (wage, None),
                        Ok(Err(why)) => (None, Some(why)),
                        Err(e) => (None, Some(format!("Database error: {}", e))),
                    };
                    // The best tool for it, if it takes one (E1.5)
                    let tool = crate::engine::tools::tool_for_node(
                        &state.db, &state.game, player_id, node_uuid,
                    ).await;
                    let (tool, refusal) = match tool {
                        Ok(tool) => (tool, refusal),
                        Err(e) => (None, refusal.or(Some(format!("Database error: {}", e)))),
                    };
                    if let Some(reason) = refusal {
                        out.push(ServerMessage::ExtractionFailed {
                                player_id,
                                reason,
                            });
                        return out;
                    }
                    let mut game = state.game.write().await;
                    game.queue_command(crate::engine::Command::StartExtraction {
                        player_id,
                        node_id: node_uuid,
                        tool,
                        wage,
                    });
                } else {
                    out.push(ServerMessage::ExtractionFailed {
                            player_id,
                            reason: format!("Invalid resource node id '{}'", node_id),
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
                let items = fetch_player_items(state, player_id, &item_uuids).await;

                if items.is_empty() && !item_uuids.is_empty() {
                    out.push(ServerMessage::CraftingFailed {
                            player_id,
                            reason: "Invalid or unauthorized items".to_string(),
                        });
                    return out;
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
                        out.push(ServerMessage::Error {
                                message: "Invalid item ID".to_string(),
                                command: command.into(),
                            });
                        return out;
                    }
                };
                let container_uuid = match Uuid::parse_str(&target_container_id) {
                    Ok(id) => id,
                    Err(_) => {
                        out.push(ServerMessage::Error {
                                message: "Invalid container ID".to_string(),
                                command: command.into(),
                            });
                        return out;
                    }
                };

                // Handle move directly via database (synchronous for immediate feedback)
                match handle_move_item(state, player_id, item_uuid, container_uuid, target_slot).await {
                    Ok(source_container) => {
                        // A move between containers changes both,
                        // so name each one that changed (#77).
                        for container_id in moved_containers(source_container, container_uuid) {
                            let msg = ServerMessage::InventoryUpdated { player_id, container_id };
                            out.push(msg);
                        }
                    }
                    Err(e) => {
                        out.push(ServerMessage::Error {
                                message: e,
                                command: command.into(),
                            });
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
                        out.push(ServerMessage::Error {
                                message: message.to_string(),
                                command: command.into(),
                            });
                        return out;
                    }
                };
                match handle_split_stack(state, player_id, item_uuid, quantity, to_container, to_slot, qualities.as_ref()).await {
                    // Into another container, both changed (#190)
                    Ok((source, target)) => {
                        for container_id in moved_containers(Some(source), target) {
                            let msg = ServerMessage::InventoryUpdated { player_id, container_id };
                            out.push(msg);
                        }
                    }
                    Err(message) => {
                        let msg = ServerMessage::Error { message, command: command.into() };
                        out.push(msg);
                    }
                }
            }
            // M4: Stations
            ClientMessage::PlaceStation { station_type, x, y, kit_item_id, floor } => {
                let kit_uuid = match Uuid::parse_str(&kit_item_id) {
                    Ok(id) => id,
                    Err(_) => {
                        out.push(ServerMessage::Error {
                                message: "Invalid kit item ID".to_string(),
                                command: command.into(),
                            });
                        return out;
                    }
                };

                match crate::engine::station_placement::place_station(state, player_id, &station_type, x, y, floor, kit_uuid).await {
                    Ok(result) => {
                        result.remember(state).await;
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
                            indoors: station_info.indoors,
                            floor: station_info.floor,
                        };
                        out.push(msg);
                    }
                    Err(e) => {
                        out.push(ServerMessage::Error { message: e.message(), command: command.into() });
                    }
                }
            }
            ClientMessage::RemoveStation { station_id } => {
                let station_uuid = match Uuid::parse_str(&station_id) {
                    Ok(id) => id,
                    Err(_) => {
                        out.push(ServerMessage::Error {
                                message: "Invalid station ID".to_string(),
                                command: command.into(),
                            });
                        return out;
                    }
                };

                match crate::engine::station_removal::remove_station(&state.db, player_id, station_uuid).await {
                    Ok(removed) => {
                        // Everyone sees it go. The owner's inventory now holds the
                        // kit and their things from the box, and a guest's deposit
                        // went back to the guest (#489).
                        let mut game = state.game.write().await;
                        game.stations.remove(&station_uuid);
                        let _ = game.event_sender.send(GameEvent::StationRemoved { station_id: station_uuid });
                        for (player_id, container_id) in removed.returned_to {
                            let _ = game.event_sender.send(GameEvent::InventoryUpdated {
                                player_id,
                                container_id,
                                slots: vec![],
                            });
                        }
                    }
                    Err(e) => {
                        out.push(ServerMessage::Error { message: e.message(), command: command.into() });
                    }
                }
            }
            ClientMessage::InstallTool { station_id, item_id } => {
                let (Ok(station_uuid), Ok(item_uuid)) = (Uuid::parse_str(&station_id), Uuid::parse_str(&item_id)) else {
                    out.push(ServerMessage::Error {
                            message: "Invalid station or item ID".to_string(),
                            command: command.into(),
                        });
                    return out;
                };
                let at = live_position(&state.game, player_id).await;
                let result = crate::engine::station_tools::install_tool(&state.db, player_id, station_uuid, item_uuid, at).await;
                announce_tool_slot(state, &mut out, player_id, station_uuid, command, result).await;
            }
            ClientMessage::RemoveTool { station_id } => {
                let Ok(station_uuid) = Uuid::parse_str(&station_id) else {
                    out.push(ServerMessage::Error {
                            message: "Invalid station ID".to_string(),
                            command: command.into(),
                        });
                    return out;
                };
                let at = live_position(&state.game, player_id).await;
                let result = crate::engine::station_tools::remove_tool(&state.db, player_id, station_uuid, at).await;
                announce_tool_slot(state, &mut out, player_id, station_uuid, command, result).await;
            }
            ClientMessage::ConfigureAutomation { ref station_id, .. }
            | ClientMessage::StartAutomation { ref station_id }
            | ClientMessage::StopAutomation { ref station_id } => {
                let Ok(station_uuid) = Uuid::parse_str(station_id) else {
                    out.push(ServerMessage::Error {
                            message: "Invalid station ID".to_string(),
                            command: command.into(),
                        });
                    return out;
                };
                let (recipes, events) = {
                    let game = state.game.read().await;
                    (game.recipes.clone(), game.event_sender.clone())
                };
                let result = match &client_msg {
                    ClientMessage::ConfigureAutomation { recipe_id, mode, .. } => {
                        crate::engine::automation::configure(&state.db, &events, &recipes, player_id, station_uuid, recipe_id, *mode).await
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
                        out.push(msg);
                    }
                }
            }
            ClientMessage::OpenStation { station_id } => {
                let station_uuid = match Uuid::parse_str(&station_id) {
                    Ok(id) => id,
                    Err(_) => {
                        out.push(ServerMessage::Error {
                                message: "Invalid station ID".to_string(),
                                command: command.into(),
                            });
                        return out;
                    }
                };

                match handle_open_station(state, player_id, station_uuid).await {
                    Ok((station_type, name, container_id)) => {
                        let msg = ServerMessage::StationOpened {
                            player_id,
                            station_id: station_uuid,
                            station_type,
                            name,
                            container_id,
                        };
                        out.push(msg);
                    }
                    Err(e) => {
                        out.push(ServerMessage::Error { message: e, command: command.into() });
                    }
                }
            }
            ClientMessage::CloseStation => {
                let msg = ServerMessage::StationClosed { player_id };
                out.push(msg);
            }
            ClientMessage::CraftAtStation { station_id, recipe_id, input_item_ids } => {
                let station_uuid = match Uuid::parse_str(&station_id) {
                    Ok(id) => id,
                    Err(_) => {
                        out.push(ServerMessage::CraftingFailed {
                                player_id,
                                reason: "Invalid station ID".to_string(),
                            });
                        return out;
                    }
                };

                // The same station rule as opening it (#80). The
                // tick checks range again, but not who may use it.
                let at = live_position(&state.game, player_id).await;
                let wage = match check_station_use(&state.db, player_id, station_uuid, at).await {
                    Ok(StationUse::Free { .. }) => None,
                    Ok(StationUse::Shift(work)) => Some(work),
                    Err(e) => {
                        out.push(ServerMessage::CraftingFailed {
                                player_id,
                                reason: e.message(),
                            });
                        return out;
                    }
                };
                // Worn out by the weather, it's out of use by hand too (V3.2)
                if let Ok(Some(reason)) = crate::engine::weather::out_of_use(&state.db, station_uuid).await {
                    out.push(ServerMessage::CraftingFailed { player_id, reason });
                    return out;
                }
                // On a wage shift, only the offer's recipe (Commons D7)
                if let Some(reason) = wage.as_ref().and_then(|w| crate::engine::wages::recipe_refusal(w, &recipe_id)) {
                    out.push(ServerMessage::CraftingFailed { player_id, reason });
                    return out;
                }

                let item_uuids: Vec<Uuid> = input_item_ids
                    .iter()
                    .filter_map(|id| Uuid::parse_str(id).ok())
                    .collect();

                // Fetch items for crafting validation: the player's
                // own, or on a wage shift the station box's
                let items = match &wage {
                    Some(_) => fetch_box_items(state, station_uuid, &item_uuids).await,
                    None => fetch_player_items(state, player_id, &item_uuids).await,
                };

                if items.is_empty() && !item_uuids.is_empty() {
                    out.push(ServerMessage::CraftingFailed {
                            player_id,
                            reason: "Invalid or unauthorized items".to_string(),
                        });
                    return out;
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
                    wage,
                });
            }
            // M5: Trading
            ClientMessage::TradeInitiate { target_player_id } => {
                let target_uuid = match Uuid::parse_str(&target_player_id) {
                    Ok(id) => id,
                    Err(_) => {
                        out.push(ServerMessage::Error {
                                message: "Invalid player ID".to_string(),
                                command: command.into(),
                            });
                        return out;
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
                let offer_lines = match validate_trade_offer(state, player_id, &items, strands).await {
                    Ok(lines) => lines,
                    Err(message) => {
                        out.push(ServerMessage::Error { message, command: command.into() });
                        return out;
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
                            out.push(ServerMessage::Error { message: e, command: command.into() });
                            (false, None)
                        }
                    }
                };

                if should_execute {
                    // Execute trade with DB access
                    match execute_trade(state, player_id).await {
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
        out
    }

    /// One of the game's events, as this socket hears it (#566): only once
    /// it has subscribed to `position`, and only what's for its player, in
    /// sight of them, or on a channel it watches. What it sends is returned,
    /// in order.
    async fn heard(&mut self, state: &AppState, event: GameEvent) -> Vec<Outgoing> {
        let (player_id, connection_id) = (self.player_id, self.connection_id);
        let mut out = Vec::new();
        if !self.subscribed_position {
            return out;
        }
        match event {
            // Only the players near this socket's (C0.4), each cell serialised once
            // for every socket that sees it. A watcher sees round the avatar.
            GameEvent::PlayerPositions(positions) => {
                let was_cell = std::mem::replace(&mut self.my_cell, positions.cell_of(player_id));
                let arrived: Vec<Uuid> = positions
                    .come_into_sight(self.my_cell, self.seen.as_deref().map(|before| (before, was_cell)))
                    .into_iter()
                    .filter(|id| *id != player_id)
                    .collect();
                self.sees_everyone = positions.sees_everyone(self.my_cell);
                self.seen = Some(std::sync::Arc::clone(&positions));
                if self.my_cell.is_none() && std::mem::replace(&mut self.sent_nobody, true) {
                    return out;
                }
                self.sent_nobody = self.my_cell.is_none();
                out.push(Outgoing::Text(positions.message(self.my_cell)));
                // Someone come into sight part way through a gather or a craft (#275)
                out.extend(arrived.into_iter().flat_map(|id| positions.catch_up(id)).map(Outgoing::Message));
            }
            // Joining far off isn't news; the positions bring them when they're near
            GameEvent::PlayerJoined(player) => {
                let cell = crate::engine::interest::cell_at(player.x, player.y);
                if player.id == player_id || self.my_cell.is_none_or(|mine| crate::engine::interest::near(mine, cell)) {
                    let msg = ServerMessage::PlayerJoined { player };
                    out.push(Outgoing::Message(msg));
                }
            }
            // Talk (S1.1): local by the cell and then the distance, Capitol to all
            GameEvent::Said { seq, line } => {
                let fresh = line.channel != capitol_protocol::ws::Channel::Capitol || seq > self.heard_capitol;
                if fresh && crate::engine::talk::hears(&line, player_id, self.holds_avatar, self.seen.as_deref(), self.my_cell, &self.muted, self.my_guild) {
                    self.heard_capitol = self.heard_capitol.max(seq);
                    out.push(Outgoing::Message(line.message(seq)));
                }
            }
            // They muted or unmuted someone, here or on another device (S1.4)
            GameEvent::MutesChanged { player_id: who } => {
                if who == player_id {
                    self.muted = read_mutes(state, player_id, std::mem::take(&mut self.muted)).await;
                }
            }
            // Their guild changed, or they joined or left one (S4.1)
            GameEvent::Tutorial { player_id: to, message } => {
                if to == player_id {
                    out.push(Outgoing::Message(message));
                }
            }
            GameEvent::GuildChanged { guild_id, players } => {
                if players.contains(&player_id) {
                    // In, out or still in: whose talk it hears (S4.4)
                    self.my_guild = read_guild(state, player_id, self.my_guild).await;
                    let msg = ServerMessage::GuildChanged { guild_id };
                    out.push(Outgoing::Message(msg));
                }
            }
            // Leaving goes to everyone, near or far: it ends their trades
            GameEvent::PlayerLeft { player_id: left_id } => {
                let msg = ServerMessage::PlayerLeft { player_id: left_id };
                out.push(Outgoing::Message(msg));
            }
            // M2: Extraction events, from the players in sight (#275)
            GameEvent::ExtractionStarted { player_id: pid, node_id, duration_ticks } => {
                if pid == player_id || self.sees_everyone || self.seen.as_ref().is_some_and(|p| p.sees(self.my_cell, pid)) {
                    let msg = ServerMessage::ExtractionStarted { player_id: pid, node_id, duration_ticks };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::ExtractionProgress { player_id: pid, progress, duration } => {
                if pid == player_id || self.sees_everyone || self.seen.as_ref().is_some_and(|p| p.sees(self.my_cell, pid)) {
                    let msg = ServerMessage::ExtractionProgress { player_id: pid, progress, duration };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::ExtractionCompleted { player_id: pid, node_id, item_type, item_name, quantity, quality, tool } => {
                if pid == player_id || self.sees_everyone || self.seen.as_ref().is_some_and(|p| p.sees(self.my_cell, pid)) {
                    let msg = ServerMessage::ExtractionCompleted { player_id: pid, node_id, item_type, item_name, quantity, quality, tool };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::ExtractionCancelled { player_id: pid } => {
                if pid == player_id || self.sees_everyone || self.seen.as_ref().is_some_and(|p| p.sees(self.my_cell, pid)) {
                    let msg = ServerMessage::ExtractionCancelled { player_id: pid };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::ExtractionFailed { player_id: pid, reason } => {
                if pid == player_id {
                    let msg = ServerMessage::ExtractionFailed { player_id: pid, reason };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::NodeDepleted { node_id } => {
                let msg = ServerMessage::NodeDepleted { node_id };
                out.push(Outgoing::Message(msg));
            }
            GameEvent::NodeRegenerated { node_id } => {
                let msg = ServerMessage::NodeRegenerated { node_id };
                out.push(Outgoing::Message(msg));
            }
            // M3: Crafting events, from the players in sight (#275)
            GameEvent::CraftingStarted { player_id: pid, operation_id, recipe_id, recipe_name, duration_ticks } => {
                if pid == player_id || self.sees_everyone || self.seen.as_ref().is_some_and(|p| p.sees(self.my_cell, pid)) {
                    let msg = ServerMessage::CraftingStarted { player_id: pid, operation_id, recipe_id, recipe_name, duration_ticks };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::CraftingProgress { player_id: pid, operation_id, progress, duration } => {
                if pid == player_id || self.sees_everyone || self.seen.as_ref().is_some_and(|p| p.sees(self.my_cell, pid)) {
                    let msg = ServerMessage::CraftingProgress { player_id: pid, operation_id, progress, duration };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::CraftingCompleted { player_id: pid, operation_id, item_type, item_name, quantity, quality, tool } => {
                if pid == player_id || self.sees_everyone || self.seen.as_ref().is_some_and(|p| p.sees(self.my_cell, pid)) {
                    let msg = ServerMessage::CraftingCompleted { player_id: pid, operation_id, item_type, item_name, quantity, quality, tool };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::CraftingCancelled { player_id: pid, operation_id } => {
                if pid == player_id || self.sees_everyone || self.seen.as_ref().is_some_and(|p| p.sees(self.my_cell, pid)) {
                    let msg = ServerMessage::CraftingCancelled { player_id: pid, operation_id };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::CraftingFailed { player_id: pid, reason } => {
                if pid == player_id {
                    let msg = ServerMessage::CraftingFailed { player_id: pid, reason };
                    out.push(Outgoing::Message(msg));
                }
            }
            // M3.5: Inventory events
            GameEvent::InventoryUpdated { player_id: pid, container_id, slots: _ } => {
                let msg = ServerMessage::InventoryUpdated { player_id: pid, container_id };
                out.push(Outgoing::Message(msg));
            }
            // M4: Station events
            GameEvent::StationRemoved { station_id } => {
                let msg = ServerMessage::StationRemoved { station_id };
                out.push(Outgoing::Message(msg));
            }
            GameEvent::StationCondition { station_id, condition } => {
                let msg = ServerMessage::StationCondition { station_id, condition };
                out.push(Outgoing::Message(msg));
            }
            GameEvent::StationToolChanged { station_id, tool } => {
                let msg = ServerMessage::StationToolChanged { station_id, tool };
                out.push(Outgoing::Message(msg));
            }
            GameEvent::StationAutomationChanged { station_id, automation } => {
                let msg = ServerMessage::StationAutomationChanged { station_id, automation };
                out.push(Outgoing::Message(msg));
            }
            // E4.3: a station working on its own, to its owner
            GameEvent::AutomationProduced { owner_id, station_id, produced, done } => {
                if owner_id == player_id {
                    let msg = ServerMessage::AutomationProduced { station_id, produced, done };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::AutomationPaused { owner_id, station_id, station_type, reason, notice_id } => {
                if owner_id == player_id && take_notice(&state.db, notice_id).await {
                    let msg = ServerMessage::AutomationPaused { station_id, station_type, reason };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::StationDisplaced { owner_id, station_id, station_type, terrain_id, notice_id } => {
                if owner_id == player_id && take_notice(&state.db, notice_id).await {
                    let msg = ServerMessage::StationDisplaced { station_id, station_type, terrain_id };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::BazaarNotice { player_id: to, notice_id, message }
                | GameEvent::Notice { player_id: to, notice_id, message } => {
                if to == player_id && take_notice(&state.db, Some(notice_id)).await {
                    out.push(Outgoing::Message(message));
                }
            }
            // M5: Trading events
            GameEvent::TradeRequested { from_player, from_player_name, from_citizen, to_player } => {
                // Only send to the target player
                if to_player == player_id {
                    let msg = ServerMessage::TradeRequested { from_player, from_player_name, from_citizen };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::TradeRequestDeclined { by_player, initiator } => {
                // Send to initiator
                if initiator == player_id {
                    let msg = ServerMessage::TradeRequestDeclined { by_player };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::TradeRequestExpired { initiator, target } => {
                // To both: the asker stops waiting, the one asked loses the prompt
                if player_id == initiator || player_id == target {
                    let msg = ServerMessage::TradeRequestExpired { from_player: initiator, to_player: target };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::TradeStarted { trade_id, player_a, player_b } => {
                // Send to both players
                if player_id == player_a || player_id == player_b {
                    let partner_id = if player_id == player_a { player_b } else { player_a };
                    // Get partner name from game state
                    let (partner_name, partner_citizen) = {
                        let game = state.game.read().await;
                        game.players.get(&partner_id).map(|p| (p.name.clone(), p.citizen)).unwrap_or_default()
                    };
                    let msg = ServerMessage::TradeStarted { trade_id, partner_id, partner_name, partner_citizen };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::TradeOfferUpdated { trade_id, player_id: offering_player, items, strands } => {
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
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::TradeAccepted { trade_id, player_id: accepting_player } => {
                // Get trade participants
                let participants = {
                    let game = state.game.read().await;
                    game.trade_manager.active_trades.get(&trade_id)
                        .map(|t| (t.player_a, t.player_b))
                };

                if let Some((player_a, player_b)) = participants {
                    if player_id == player_a || player_id == player_b {
                        let msg = ServerMessage::TradeAccepted { trade_id, player_id: accepting_player };
                        out.push(Outgoing::Message(msg));
                    }
                }
            }
            GameEvent::TradeExecuted { trade_id } => {
                let msg = ServerMessage::TradeExecuted { trade_id };
                out.push(Outgoing::Message(msg));
            }
            GameEvent::TradeCancelled { trade_id, reason } => {
                let msg = ServerMessage::TradeCancelled { trade_id, reason };
                out.push(Outgoing::Message(msg));
            }
            GameEvent::TradeFailed { player_id: pid, command, reason } => {
                if pid == player_id {
                    let msg = ServerMessage::Error { message: reason, command: command.into() };
                    out.push(Outgoing::Message(msg));
                }
            }
            // M5: Currency events
            GameEvent::CurrencyChanged { player_id: pid, new_balance } => {
                if pid == player_id {
                    let msg = ServerMessage::CurrencyChanged { new_balance };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::ToolBroke { player_id: pid, item_id, item_type, item_name } => {
                if pid == player_id {
                    let msg = ServerMessage::ToolBroke { item_id, item_type, item_name };
                    out.push(Outgoing::Message(msg));
                }
            }
            // M7: Zone events
            GameEvent::ZoneChanged { player_id: pid, from_zone, to_zone, zone_name } => {
                // Only send to the player who changed zones
                if pid == player_id {
                    let msg = ServerMessage::ZoneChanged { player_id: pid, from_zone, to_zone, zone_name };
                    out.push(Outgoing::Message(msg));
                }
            }
            // Terrain events
            GameEvent::MovementBlocked { player_id: pid, reason, water_id, stopped_x, stopped_y } => {
                // Only send to the player who was blocked
                if pid == player_id {
                    let msg = ServerMessage::MovementBlocked { player_id: pid, reason, water_id, stopped_x, stopped_y };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::TerrainUpdated => {
                // Broadcast to all connected clients
                let msg = ServerMessage::TerrainUpdated;
                out.push(Outgoing::Message(msg));
            }
            // B13: Plot ownership, to everyone
            GameEvent::PlotClaimed { terrain_id, owner_id, claimed_at, guild } => {
                let msg = ServerMessage::PlotClaimed { terrain_id, owner_id, claimed_at, guild };
                out.push(Outgoing::Message(msg));
            }
            GameEvent::PlotReleased { terrain_id } => {
                let msg = ServerMessage::PlotReleased { terrain_id };
                out.push(Outgoing::Message(msg));
            }
            GameEvent::PlotFloorBuilt { terrain_id, owner_id, floors, floor_ready_at } => {
                let msg = ServerMessage::PlotFloorBuilt { terrain_id, owner_id, floors, floor_ready_at };
                out.push(Outgoing::Message(msg));
            }
            // #131: Property tax, to the plot's owner only, if they take its notice (#138).
            // A watcher sends it without taking it, leaving it for the holder.
            GameEvent::TaxDelinquent { owner_id, terrain_id, tax_owed, forecloses_at, notice_id } => {
                if owner_id == player_id && (!holds(state, player_id, connection_id).await || take_notice(&state.db, notice_id).await) {
                    let msg = ServerMessage::TaxDelinquent { terrain_id, tax_owed, forecloses_at };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::PlotForeclosed { owner_id, terrain_id, tax_owed, refund, impounded, lost, notice_id } => {
                if owner_id == player_id && (!holds(state, player_id, connection_id).await || take_notice(&state.db, notice_id).await) {
                    let msg = ServerMessage::PlotForeclosed { terrain_id, tax_owed, refund, impounded, lost };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::SubscriptionLapsed { player_id: whose, until, notice_id } => {
                if whose == player_id && take_notice(&state.db, notice_id).await {
                    let msg = ServerMessage::SubscriptionLapsed { until };
                    out.push(Outgoing::Message(msg));
                }
            }
            // E3.2: fills and expiries, to the order's owner
            GameEvent::OrderFilled { player_id: whose, order, trade } => {
                if whose == player_id {
                    let msg = ServerMessage::OrderFilled { order, trade };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::OrderClosed { player_id: whose, order } => {
                if whose == player_id {
                    let msg = ServerMessage::OrderCancelled { order };
                    out.push(Outgoing::Message(msg));
                }
            }
            // E3.3: placing, and the items' market channels
            GameEvent::OrderPlaced { player_id: whose, order } => {
                if whose == player_id {
                    let msg = ServerMessage::OrderPlaced { order };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::MarketDepth { item_type, seq, levels } => {
                if self.market_items.contains(&item_type) {
                    let msg = ServerMessage::MarketDepth { item_type, seq, full: false, levels };
                    out.push(Outgoing::Message(msg));
                }
            }
            GameEvent::MarketTrade { trade } => {
                if self.market_items.contains(&trade.item_type) {
                    let msg = ServerMessage::MarketTrade { trade };
                    out.push(Outgoing::Message(msg));
                }
            }
            // E3.9: the Gazette, to everyone
            GameEvent::GazettePublished { day, number, headline } => {
                let msg = ServerMessage::GazettePublished { day, number, headline };
                out.push(Outgoing::Message(msg));
            }
            // D6.1: a fee moved, to everyone, so previews follow
            GameEvent::FeesChanged { fees } => {
                let msg = ServerMessage::FeesChanged { fees };
                out.push(Outgoing::Message(msg));
            }
            // #617: the deed grants switched off or on: their grant as it
            // stands now, so the Deeds Office doesn't offer one a claim refuses
            GameEvent::GrantsSwitched => match crate::engine::tutorial::progress(&state.db, player_id).await {
                Ok(progress) => out.push(Outgoing::Message(crate::engine::tutorial::message(progress))),
                Err(e) => tracing::error!("Failed to read {}'s tutorial for the grants: {:?}", player_id, e),
            },
            // Commons S2.1: the hall's board, to everyone
            GameEvent::Board { change, commission } => {
                let msg = change.message(commission);
                out.push(Outgoing::Message(msg));
            }
            // Commons S2.6: the board's Work page to everyone, a
            // worker's pay to them alone
            GameEvent::Wages { to, message } => {
                if to.is_none_or(|to| to == player_id) {
                    out.push(Outgoing::Message(message));
                }
            }
        }
        out
    }

    /// The events this socket missed are gone (C0.3): skip ahead and start it
    /// over from what's true now. The next `positions` tells it again what
    /// everyone in sight is doing.
    async fn resynced(&mut self, state: &AppState, missed: u64) -> Vec<ServerMessage> {
        crate::engine::load::Receiver::Sockets.lagged(missed);
        self.seen = None;
        self.sees_everyone = false;
        // A `mutes_changed` may be among what it missed
        self.muted = read_mutes(state, self.player_id, std::mem::take(&mut self.muted)).await;
        // And a `guild_changed`
        self.my_guild = read_guild(state, self.player_id, self.my_guild).await;
        resync(&state.db, &state.game, &state.market, self.player_id, self.connection_id, missed, &self.market_items).await
    }
}

/// What a socket that fell `missed` events behind is sent before it carries on
/// (C0.3): `resync`, which tells the client to refetch what it fetches over
/// REST, then afresh what the server only pushes: the positions near it, the
/// player's balance, where they are on the road in (#617), the depth of each
/// item it watches, and, if it holds the
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
    // Where they are on the road in: a step, or the grants switching, may be
    // among what it missed (#617)
    match crate::engine::tutorial::progress(db, player_id).await {
        Ok(progress) => out.push(crate::engine::tutorial::message(progress)),
        Err(e) => tracing::error!("Failed to read {}'s tutorial to resync: {:?}", player_id, e),
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
/// Who `player_id` has muted now (S1.4). If the database can't say, the
/// mutes the socket already had stand.
async fn read_mutes(
    state: &AppState,
    player_id: Uuid,
    had: std::collections::HashSet<Uuid>,
) -> std::collections::HashSet<Uuid> {
    match crate::mutes::of(&state.db, player_id).await {
        Ok(muted) => muted,
        Err(e) => {
            tracing::error!("Failed to read {player_id}'s mutes: {e:?}");
            had
        }
    }
}

/// The guild `player_id` is in now (S4.4). If the database can't say, the
/// guild the socket already had stands.
async fn read_guild(state: &AppState, player_id: Uuid, had: Option<Uuid>) -> Option<Uuid> {
    match crate::engine::guilds::guild_of_player(&state.db, player_id).await {
        Ok(guild) => guild,
        Err(e) => {
            tracing::error!("Failed to read {player_id}'s guild: {e:?}");
            had
        }
    }
}

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

/// The stacks in `station_id`'s box a wage shift's craft may take from
/// (Commons D7), whoever's they are: those named, in the order asked for, or
/// with none named, every one in slot order.
async fn fetch_box_items(state: &AppState, station_id: Uuid, item_ids: &[Uuid]) -> Vec<crate::models::Item> {
    let mut items: Vec<crate::models::Item> = sqlx::query_as(&format!(
        "SELECT {} FROM items
         WHERE container_id = (SELECT id FROM containers WHERE station_id = $1)
           AND (cardinality($2::uuid[]) = 0 OR id = ANY($2))
         ORDER BY slot_index NULLS LAST, id",
        crate::models::ITEM_COLUMNS
    ))
    .bind(station_id)
    .bind(item_ids)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    if !item_ids.is_empty() {
        items.sort_by_key(|item| item_ids.iter().position(|id| *id == item.id));
    }
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
    out: &mut Vec<ServerMessage>,
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
            out.push(ServerMessage::Error { message: e.message(), command: command.into() });
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
    // station_use_level admits, within range (#80), or a worker on a wage
    // shift there (Commons D7)
    let at = live_position(&state.game, player_id).await;
    check_station_use(&state.db, player_id, station_id, at)
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

    let mut tx = state.db.begin().await.map_err(|e| format!("DB error: {}", e))?;
    let balances = settle_trade(&mut tx, &session).await?;
    tx.commit().await.map_err(|e| format!("DB error: {}", e))?;

    tracing::info!("Trade {} executed successfully between {} and {}", trade_id, player_a, player_b);
    Ok((trade_id, balances))
}

/// A trade's writes, inside the caller's transaction: checks both offers
/// still hold, hands the items over, and moves the Strands. Returns each
/// side's new balance if Strands changed hands. Any error should roll the
/// transaction back.
async fn settle_trade(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session: &crate::engine::trading::TradeSession,
) -> Result<Vec<(Uuid, i64)>, String> {
    let (trade_id, player_a, player_b) = (session.id, session.player_a, session.player_b);

    // Verify every item offered still exists, is its offerer's, and holds the units offered
    for (items, owner) in [(&session.offer_a.items, player_a), (&session.offer_b.items, player_b)] {
        for (item_id, units) in items {
            let owned: Option<(i32,)> = sqlx::query_as(
                "SELECT 1 FROM items WHERE id = $1 AND owner_id = $2 FOR UPDATE"
            )
            .bind(item_id)
            .bind(owner)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| format!("DB error: {}", e))?;
            let held = crate::engine::units::units_of(tx, *item_id)
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
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    let balance_b: (i64,) = sqlx::query_as("SELECT strand_balance FROM players WHERE id = $1 FOR UPDATE")
        .bind(player_b)
        .fetch_one(&mut **tx)
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
            placing.push((hand_over(tx, *item_id, units, to).await?, to));
        }
    }
    for to in [player_a, player_b] {
        let stacks: Vec<Uuid> = placing
            .iter()
            .filter(|(_, owner)| *owner == to)
            .map(|(id, _)| *id)
            .collect();
        if !stacks.is_empty() {
            place_in_inventory(tx, to, &stacks).await?;
        }
    }

    // Move the Strands: the net, one audited row a side, so the payer's
    // trade_sent and the payee's trade_received net to zero in the ledger,
    // and each trail explains its balance (#506). Both offers were checked
    // against their gross above.
    let net_strands = session.offer_b.strands - session.offer_a.strands;
    if net_strands == 0 {
        return Ok(Vec::new());
    }
    let (payer, payee) = if net_strands > 0 { (player_b, player_a) } else { (player_a, player_b) };
    let paid = net_strands.abs();
    let payer_balance =
        subtract_strands_in(tx, payer, paid, TransactionType::TradeSent, Some("Trade payment"), Some(trade_id)).await?;
    let payee_balance =
        add_strands_in(tx, payee, paid, TransactionType::TradeReceived, Some("Trade payment"), Some(trade_id)).await?;
    Ok(vec![(payer, payer_balance), (payee, payee_balance)])
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
        assert_eq!(tags, ["resync", "positions", "currency_changed", "tutorial", "market_depth", "plot_foreclosed"]);
        assert_eq!(sent[0], ServerMessage::Resync { missed: 100 });
        assert!(matches!(&sent[1], ServerMessage::Positions { players } if players.iter().any(|p| p.id == ada)));
        assert_eq!(sent[2], ServerMessage::CurrencyChanged { new_balance: 1234 });
        assert!(matches!(&sent[3], ServerMessage::Tutorial { grant: capitol_protocol::tutorial::Grant::Unearned, .. }));
        assert!(matches!(&sent[4], ServerMessage::MarketDepth { item_type, full: true, .. } if item_type == "plank"));
        assert_eq!(sent[5], gone);
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

    /// Strands offered both ways move as the net, one row a side: the
    /// payer's trade_sent and the payee's trade_received net to zero, and
    /// every trail explains its balance (#506)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_trade_audits_its_strands_on_both_sides(db: sqlx::PgPool) {
        use crate::engine::conservation::{assert_audited, balances_of};
        use crate::engine::plot_sync::tests::player;
        use capitol_protocol::ledger::StrandFlow;

        let ada = player(&db, "ada", 100).await;
        let bob = player(&db, "bob", 100).await;
        let start = balances_of(&db, [ada, bob]).await;
        let rows = |trade: Uuid| {
            let db = db.clone();
            async move {
                sqlx::query_as::<_, (Uuid, i64, i64, i64, String)>(
                    "SELECT player_id, amount, balance_before, balance_after, transaction_type
                     FROM currency_transactions WHERE reference_id = $1 ORDER BY amount",
                )
                .bind(trade)
                .fetch_all(&db)
                .await
                .unwrap()
            }
        };

        // Ada offers 15 and Bob 40: Bob pays Ada the 25 between them
        let mut session = crate::engine::trading::TradeSession::new(ada, bob);
        session.offer_a.strands = 15;
        session.offer_b.strands = 40;
        let mut tx = db.begin().await.unwrap();
        let balances = settle_trade(&mut tx, &session).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(balances, vec![(bob, 75), (ada, 125)]);
        assert_eq!(
            rows(session.id).await,
            vec![
                (bob, -25, 100, 75, "trade_sent".to_string()),
                (ada, 25, 100, 125, "trade_received".to_string()),
            ]
        );
        assert_audited(&db, &start).await;

        // The other way round, and an even trade moves nothing
        let mut back = crate::engine::trading::TradeSession::new(ada, bob);
        back.offer_a.strands = 30;
        let mut even = crate::engine::trading::TradeSession::new(ada, bob);
        (even.offer_a.strands, even.offer_b.strands) = (10, 10);
        let mut tx = db.begin().await.unwrap();
        assert_eq!(settle_trade(&mut tx, &back).await.unwrap(), vec![(ada, 95), (bob, 105)]);
        assert_eq!(settle_trade(&mut tx, &even).await.unwrap(), vec![]);
        tx.commit().await.unwrap();
        assert!(rows(even.id).await.is_empty());
        assert_audited(&db, &start).await;
        for kind in ["trade_sent", "trade_received"] {
            assert_eq!(StrandFlow::of(kind), StrandFlow::Transfer);
        }

        // An offer past the balance refuses the trade, whatever the net
        let mut broke = crate::engine::trading::TradeSession::new(ada, bob);
        (broke.offer_a.strands, broke.offer_b.strands) = (500, 499);
        let mut tx = db.begin().await.unwrap();
        assert!(settle_trade(&mut tx, &broke).await.unwrap_err().contains("insufficient"));
    }

    /// A socket driven without one (#463): `player` signed in on a test
    /// `AppState`, holding the avatar as a client does once its `presence`
    /// is out. `send` hands its `Socket` a message and returns what came back
    /// to it; `heard` is what went out on the game's events meanwhile.
    struct Harness {
        state: AppState,
        socket: Socket,
        events: tokio::sync::broadcast::Receiver<GameEvent>,
    }

    impl Harness {
        async fn signed_in(state: AppState, player: Uuid) -> Harness {
            let account: Uuid = sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1")
                .bind(player)
                .fetch_one(&state.db)
                .await
                .unwrap();
            let row = load_player(&state.db, account).await.unwrap();
            let (socket, events) = {
                let mut game = state.game.write().await;
                let (connection, _) = game.open_socket(player, Holder::Session("test".into()), true);
                let mut socket = Socket::new(&state, &row, account, None, connection);
                game.claim_presence(row, connection).unwrap();
                socket.holds_avatar = true;
                (socket, game.subscribe())
            };
            Harness { state, socket, events }
        }

        async fn send(&mut self, msg: ClientMessage) -> Vec<ServerMessage> {
            self.socket.handle(&self.state, msg).await
        }

        fn heard(&mut self) -> Vec<GameEvent> {
            std::iter::from_fn(|| self.events.try_recv().ok()).collect()
        }

        /// The commands it queued for the tick, taken
        async fn queued(&self) -> Vec<crate::engine::Command> {
            std::mem::take(&mut self.state.game.write().await.command_queue)
        }

        /// What this socket sends on hearing `event` (#566), read back as the
        /// client would, `positions` too
        async fn hears(&mut self, event: GameEvent) -> Vec<ServerMessage> {
            self.socket
                .heard(&self.state, event)
                .await
                .into_iter()
                .map(|out| match out {
                    Outgoing::Message(msg) => msg,
                    Outgoing::Text(text) => serde_json::from_str(&text).unwrap(),
                })
                .collect()
        }

        /// `player` in the world at (`x`, `y`), playing on some other socket
        async fn put(&self, player: Uuid, (x, y): (f64, f64)) {
            let account: Uuid = sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1")
                .bind(player)
                .fetch_one(&self.state.db)
                .await
                .unwrap();
            let row = load_player(&self.state.db, account).await.unwrap();
            self.state.game.write().await.add_player(PlayerState { x, y, ..row });
        }

        /// The `positions` the tick would send now
        async fn positions(&self) -> GameEvent {
            GameEvent::PlayerPositions(std::sync::Arc::new(self.state.game.read().await.positions()))
        }
    }

    /// A local line goes out to the world, and counts for the road in's
    /// `say_local`: the step is kept and the player's sockets told (#439)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn a_say_counts_for_the_road_in(db: sqlx::PgPool) {
        use capitol_protocol::tutorial::Step;
        use capitol_protocol::ws::Channel;
        let ada = crate::engine::plot_sync::tests::player(&db, "Ada", 0).await;
        let mut socket = Harness::signed_in(crate::state::for_tests(db.clone()).await, ada).await;

        let back = socket.send(ClientMessage::Say { channel: Channel::Local, text: "Morning, all".into() }).await;
        assert_eq!(back, vec![], "a line said comes back on the events, like anyone's");
        let heard = socket.heard();
        assert!(matches!(&heard[0], GameEvent::Said { line, .. } if line.from == ada && line.text == "Morning, all" && line.channel == Channel::Local));
        assert!(matches!(
            &heard[1],
            GameEvent::Tutorial { player_id, message: ServerMessage::TutorialStep { step: Step::SayLocal, done } }
                if *player_id == ada && done.has(Step::SayLocal)
        ));
        assert_eq!(heard.len(), 2);
        let done = crate::engine::tutorial::progress(&db, ada).await.unwrap().done;
        assert!(done.has(Step::SayLocal));

        // The Capitol channel is talk, but not the step
        socket.send(ClientMessage::Say { channel: Channel::Capitol, text: "Hello, Capitol".into() }).await;
        let heard = socket.heard();
        assert_eq!(heard.len(), 1);
        assert!(matches!(&heard[0], GameEvent::Said { seq: 1, line } if line.channel == Channel::Capitol));
    }

    /// Subscribing to `position` answers `subscribed`, then what waited while
    /// they were away (#138), then the Capitol channel's backlog (S1.1)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn subscribing_sends_what_waited_then_the_capitols_lines(db: sqlx::PgPool) {
        use capitol_protocol::ws::Channel;
        let ada = crate::engine::plot_sync::tests::player(&db, "Ada", 0).await;
        let gone = ServerMessage::PlotForeclosed { terrain_id: "lot".into(), tax_owed: 80, refund: 20, impounded: vec![], lost: vec![] };
        crate::engine::notices::queue(&db, ada, "lot", &gone).await.unwrap();
        let mut socket = Harness::signed_in(crate::state::for_tests(db.clone()).await, ada).await;
        socket.send(ClientMessage::Say { channel: Channel::Capitol, text: "Anyone about?".into() }).await;
        assert!(!socket.socket.subscribed_position);

        let back = socket.send(ClientMessage::Subscribe { channel: "position".into() }).await;
        let tags: Vec<&str> = back.iter().map(ServerMessage::type_name).collect();
        assert_eq!(tags, ["subscribed", "plot_foreclosed", "said"]);
        assert_eq!(back[1], gone);
        assert!(socket.socket.subscribed_position);
        assert_eq!(socket.socket.heard_capitol, 1);

        // Again, it's only answered: the notice is taken, the backlog heard
        let back = socket.send(ClientMessage::Subscribe { channel: "position".into() }).await;
        assert_eq!(back, vec![ServerMessage::Subscribed { channel: "position".into() }]);
    }

    /// Placing a workbench from its kit answers `station_placed`; opening it
    /// answers `station_opened` with its box, and closing it `station_closed`
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn a_station_placed_opens_and_closes(db: sqlx::PgPool) {
        use crate::engine::units::test_units::stack_of;
        let ada = crate::engine::plot_sync::tests::player(&db, "Ada", 0).await;
        let pack = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(pack)
            .bind(ada)
            .execute(&db)
            .await
            .unwrap();
        let kit = stack_of(&db, ada, pack, 0, "workbench_kit", &Units::uniform(1, 50)).await;
        sqlx::query("UPDATE players SET position_x = 0, position_y = 0 WHERE id = $1").bind(ada).execute(&db).await.unwrap();
        let mut socket = Harness::signed_in(crate::state::for_tests(db.clone()).await, ada).await;

        let back = socket
            .send(ClientMessage::PlaceStation { station_type: "workbench".into(), x: 0.0, y: 0.0, kit_item_id: kit.to_string(), floor: 0 })
            .await;
        let [ServerMessage::StationPlaced { station_id, owner_id, container_id, .. }] = back.as_slice() else {
            panic!("placing answered {back:?}");
        };
        let (station_id, container_id) = (*station_id, *container_id);
        assert_eq!(*owner_id, ada);
        assert!(socket.state.game.read().await.stations.contains_key(&station_id), "the game has it");

        let back = socket.send(ClientMessage::OpenStation { station_id: station_id.to_string() }).await;
        assert!(
            matches!(
                back.as_slice(),
                [ServerMessage::StationOpened { player_id, station_id: opened, station_type, container_id: its_box, .. }]
                    if *player_id == ada && *opened == station_id && station_type == "workbench" && *its_box == container_id
            ),
            "opening answered {back:?}"
        );
        let back = socket.send(ClientMessage::CloseStation).await;
        assert_eq!(back, vec![ServerMessage::StationClosed { player_id: ada }]);
    }

    /// Steering and walking are queued for the tick, unanswered (T0.2); a
    /// socket that doesn't hold the avatar is refused, and queues nothing (E0.8)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn steering_is_queued_for_the_holder_alone(db: sqlx::PgPool) {
        use crate::engine::Command;
        let ada = crate::engine::plot_sync::tests::player(&db, "Ada", 0).await;
        let mut socket = Harness::signed_in(crate::state::for_tests(db.clone()).await, ada).await;

        let back = socket.send(ClientMessage::Steer { heading: 90, throttle: 255, at: Some([10.0, 20.0]) }).await;
        assert_eq!(back, vec![]);
        let back = socket.send(ClientMessage::Move { x: 8300.0, y: 8300.0 }).await;
        assert_eq!(back, vec![]);
        let queued = socket.queued().await;
        assert!(
            matches!(
                queued.as_slice(),
                [
                    Command::Steer { player_id, heading: 90, throttle: 255, at: Some((x, y)) },
                    Command::Move { dest_x, dest_y, .. },
                ] if *player_id == ada && (*x, *y) == (10.0, 20.0) && (*dest_x, *dest_y) == (8300.0, 8300.0)
            ),
            "{queued:?}"
        );

        // Another of Ada's sockets, watching
        let watcher = socket.state.game.write().await.open_socket(ada, Holder::Session("phone".into()), true).0;
        socket.socket.connection_id = watcher;
        let back = socket.send(ClientMessage::Steer { heading: 0, throttle: 255, at: None }).await;
        assert_eq!(back, vec![ServerMessage::Error { message: presence::NOT_HOLDING.into(), command: "steer".into() }]);
        assert!(socket.queued().await.is_empty());
    }

    /// When the deed grants switch off or on, a socket sends its player's
    /// grant as it stands then, read afresh each time (#617)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn the_grants_switching_tells_each_socket_its_grant(db: sqlx::PgPool) {
        use crate::engine::deed_grants::tests::{the_road, walked};
        use capitol_protocol::tutorial::Grant;
        let ada = crate::engine::plot_sync::tests::player(&db, "Ada", 0).await;
        let mut socket = Harness::signed_in(crate::state::for_tests(db.clone()).await, ada).await;
        socket.send(ClientMessage::Subscribe { channel: "position".into() }).await;
        let grant = |sent: Vec<ServerMessage>| match sent.as_slice() {
            [ServerMessage::Tutorial { grant, .. }] => *grant,
            other => panic!("sent {other:?}, not one tutorial"),
        };

        assert_eq!(grant(socket.hears(GameEvent::GrantsSwitched).await), Grant::Unearned);
        walked(&db, ada, the_road()).await;
        assert_eq!(grant(socket.hears(GameEvent::GrantsSwitched).await), Grant::Open, "read again, not kept");
    }

    /// A line said at (`x`, `y`) by `from`: a Capitol one numbered 1, as the
    /// first since the socket's backlog (a socket skips one it has heard)
    fn line(from: Uuid, channel: capitol_protocol::ws::Channel, (x, y): (f64, f64)) -> GameEvent {
        let line = crate::engine::talk::Line { from, name: "Someone".into(), citizen: false, channel, text: "Hello".into(), x, y, guild: None };
        let seq = u64::from(channel == capitol_protocol::ws::Channel::Capitol);
        GameEvent::Said { seq, line }
    }

    /// A socket hears none of the game until it subscribes to `position`;
    /// then what's for everyone, what's for its player and not another's,
    /// and an item's market only once it watches it (#566)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn a_socket_hears_what_its_subscribed_to_and_its_own(db: sqlx::PgPool) {
        crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
        let ada = crate::engine::plot_sync::tests::player(&db, "Ada", 0).await;
        let bob = crate::engine::plot_sync::tests::player(&db, "Bob", 0).await;
        let mut socket = Harness::signed_in(crate::state::for_tests(db.clone()).await, ada).await;
        let paid = |player_id| GameEvent::CurrencyChanged { player_id, new_balance: 5 };

        assert_eq!(socket.hears(GameEvent::TerrainUpdated).await, vec![]);
        assert_eq!(socket.hears(paid(ada)).await, vec![], "not until it subscribes");

        socket.send(ClientMessage::Subscribe { channel: "position".into() }).await;
        assert_eq!(socket.hears(GameEvent::TerrainUpdated).await, vec![ServerMessage::TerrainUpdated]);
        assert_eq!(socket.hears(paid(ada)).await, vec![ServerMessage::CurrencyChanged { new_balance: 5 }]);
        assert_eq!(socket.hears(paid(bob)).await, vec![], "Bob's balance is his own");
        let asked = |to_player| GameEvent::TradeRequested { from_player: bob, from_player_name: "Bob".into(), from_citizen: false, to_player };
        assert_eq!(
            socket.hears(asked(ada)).await,
            vec![ServerMessage::TradeRequested { from_player: bob, from_player_name: "Bob".into(), from_citizen: false }]
        );
        assert_eq!(socket.hears(asked(bob)).await, vec![]);

        // The plank market, unwatched and then watched
        let depth = || GameEvent::MarketDepth { item_type: "plank".into(), seq: 7, levels: vec![] };
        assert_eq!(socket.hears(depth()).await, vec![]);
        let back = socket.send(ClientMessage::SubscribeMarket { item_type: "plank".into() }).await;
        assert!(matches!(back.as_slice(), [ServerMessage::MarketDepth { full: true, .. }]), "{back:?}");
        assert_eq!(
            socket.hears(depth()).await,
            vec![ServerMessage::MarketDepth { item_type: "plank".into(), seq: 7, full: false, levels: vec![] }]
        );
        socket.send(ClientMessage::UnsubscribeMarket { item_type: None }).await;
        assert_eq!(socket.hears(depth()).await, vec![]);
    }

    /// A socket hears who's in sight of its player: their `positions`, their
    /// gathering, their local talk, but not someone's across the city, nor
    /// anyone it has muted. A Capitol line reaches it from anywhere (#566)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d) and Redis"]
    async fn a_socket_hears_who_is_in_sight(db: sqlx::PgPool) {
        use capitol_protocol::ws::Channel;
        let (here, near, far) = ((8300.0, 8300.0), (8310.0, 8300.0), (11300.0, 8300.0));
        let ada = crate::engine::plot_sync::tests::player(&db, "Ada", 0).await;
        let bob = crate::engine::plot_sync::tests::player(&db, "Bob", 0).await;
        let cal = crate::engine::plot_sync::tests::player(&db, "Cal", 0).await;
        sqlx::query("UPDATE players SET position_x = $1, position_y = $2 WHERE id = $3")
            .bind(here.0)
            .bind(here.1)
            .bind(ada)
            .execute(&db)
            .await
            .unwrap();
        let mut socket = Harness::signed_in(crate::state::for_tests(db.clone()).await, ada).await;
        socket.put(bob, near).await;
        socket.put(cal, far).await;
        socket.send(ClientMessage::Subscribe { channel: "position".into() }).await;

        let positions = socket.positions().await;
        let back = socket.hears(positions).await;
        let [ServerMessage::Positions { players }] = back.as_slice() else { panic!("positions answered {back:?}") };
        let mut seen: Vec<Uuid> = players.iter().map(|p| p.id).collect();
        seen.sort();
        let mut expected = vec![ada, bob];
        expected.sort();
        assert_eq!(seen, expected, "Cal is across the city");

        let gathers = |player_id| GameEvent::ExtractionStarted { player_id, node_id: Uuid::nil(), duration_ticks: 10 };
        assert_eq!(socket.hears(gathers(bob)).await.len(), 1);
        assert_eq!(socket.hears(gathers(cal)).await, vec![]);

        assert_eq!(socket.hears(line(bob, Channel::Local, near)).await.len(), 1);
        assert_eq!(socket.hears(line(cal, Channel::Local, far)).await, vec![]);
        assert_eq!(socket.hears(line(cal, Channel::Capitol, far)).await.len(), 1);

        // Ada mutes Bob, on any device: this socket reads its mutes again
        let events = socket.state.game.read().await.event_sender.clone();
        crate::mutes::set(&db, &events, ada, bob, true).await.unwrap();
        assert!(socket.heard().iter().any(|e| matches!(e, GameEvent::MutesChanged { player_id } if *player_id == ada)));
        assert_eq!(socket.hears(GameEvent::MutesChanged { player_id: ada }).await, vec![]);
        assert_eq!(socket.hears(line(bob, Channel::Local, near)).await, vec![], "muted");
        assert_eq!(socket.hears(gathers(bob)).await.len(), 1, "muting is for talk");
    }
}
