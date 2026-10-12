//! Drives `Net` against a mock server built on axum, the backend's own stack.
//! The mock mirrors `backend/src/ws/connection.rs` closely enough to exercise
//! the handshake, and adds misbehaviour the real server doesn't (malformed
//! frames, silence) to prove the client survives it.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use capitol_core::net::*;
use capitol_core::protocol::rest::{AccountInfo, AuthResponse, LoginRequest, RecipesResponse};
use capitol_core::protocol::ws::{ClientMessage, PlayerPosition, ServerMessage};
use uuid::Uuid;

const GOOD_TOKEN: &str = "good-token";
const PLAYER: Uuid = Uuid::from_u128(0x1111);
const OTHER: Uuid = Uuid::from_u128(0x2222);
const WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum Scenario {
    /// Behaves like the backend, plus two malformed frames after `subscribed`.
    Normal,
    /// Completes the handshake, then stops reading (so stops answering pings).
    GoesSilent,
    /// Accepts the socket and never answers `auth`.
    NeverAnswers,
}

#[derive(Clone)]
struct Mock {
    scenario: Scenario,
    /// What the server received on the socket, in order: raw JSON text,
    /// `"ping"` or `"close"`.
    log: Arc<Mutex<Vec<String>>>,
}

impl Mock {
    fn start(scenario: Scenario) -> (Mock, SocketAddr) {
        let mock = Mock {
            scenario,
            log: Arc::default(),
        };
        let app = Router::new()
            .route("/auth/login", post(login))
            .route("/recipes", get(recipes))
            .route(
                "/player/currency",
                get(|| async { Json(serde_json::json!({ "nope": 1 })) }),
            )
            .route("/ws", get(upgrade))
            .with_state(mock.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(listener.local_addr().unwrap()).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });
        (mock, rx.recv().unwrap())
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn received(&self) -> Vec<ClientMessage> {
        self.log()
            .iter()
            .filter_map(|t| serde_json::from_str(t).ok())
            .collect()
    }

    /// Polls the log until `done` holds.
    fn wait_until(&self, what: &str, done: impl Fn(&[String]) -> bool) {
        let start = Instant::now();
        while !done(&self.log()) {
            assert!(
                start.elapsed() < WAIT,
                "server never saw {what}; log: {:?}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

async fn login(Json(req): Json<LoginRequest>) -> Response {
    if req.password != "pw" {
        let body = serde_json::json!({ "error": "Invalid credentials" });
        return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
    }
    Json(AuthResponse {
        token: GOOD_TOKEN.into(),
        account: AccountInfo {
            id: Uuid::nil(),
            email: req.email,
            tier: "free".into(),
            is_admin: false,
        },
        player_id: PLAYER,
    })
    .into_response()
}

async fn recipes(headers: HeaderMap) -> Response {
    let expected = format!("Bearer {GOOD_TOKEN}");
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(expected.as_str()) {
        let body = serde_json::json!({ "error": "Missing authorization header" });
        return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
    }
    Json(RecipesResponse { recipes: vec![] }).into_response()
}

async fn upgrade(ws: WebSocketUpgrade, State(mock): State<Mock>) -> Response {
    ws.on_upgrade(move |socket| serve_socket(socket, mock))
}

async fn send(socket: &mut WebSocket, message: &ServerMessage) {
    let _ = socket
        .send(Message::text(serde_json::to_string(message).unwrap()))
        .await;
}

async fn serve_socket(mut socket: WebSocket, mock: Mock) {
    if let Scenario::NeverAnswers = mock.scenario {
        tokio::time::sleep(Duration::from_secs(30)).await;
        return;
    }
    while let Some(Ok(frame)) = socket.recv().await {
        let text = match frame {
            Message::Text(text) => text.to_string(),
            Message::Ping(_) => {
                mock.log.lock().unwrap().push("ping".into());
                continue;
            }
            Message::Close(_) => {
                mock.log.lock().unwrap().push("close".into());
                return;
            }
            _ => continue,
        };
        mock.log.lock().unwrap().push(text.clone());
        match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Auth { token }) if token == GOOD_TOKEN => {
                send(&mut socket, &ServerMessage::AuthOk { player_id: PLAYER }).await;
            }
            Ok(ClientMessage::Auth { .. }) => {
                send(
                    &mut socket,
                    &ServerMessage::AuthError {
                        message: "Invalid token".into(),
                    },
                )
                .await;
                return;
            }
            Ok(ClientMessage::Subscribe { channel }) => {
                send(&mut socket, &ServerMessage::Subscribed { channel }).await;
                if let Scenario::GoesSilent = mock.scenario {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    return;
                }
                let _ = socket.send(Message::text("this is not json")).await;
                let _ = socket
                    .send(Message::text(r#"{"type":"no_such_message","x":1}"#))
                    .await;
                send(&mut socket, &ServerMessage::PlayerLeft { player_id: OTHER }).await;
            }
            Ok(ClientMessage::Move { x, y }) => {
                let me = PlayerPosition {
                    id: PLAYER,
                    name: "Tester".into(),
                    x,
                    y,
                };
                send(&mut socket, &ServerMessage::Positions { players: vec![me] }).await;
            }
            _ => {}
        }
    }
}

fn config(addr: SocketAddr) -> NetConfig {
    NetConfig::with_base_url(format!("http://{addr}"))
}

/// Collects events until `done` matches one, and returns them all.
fn wait_for(net: &Net, what: &str, done: impl Fn(&NetEvent) -> bool) -> Vec<NetEvent> {
    let start = Instant::now();
    let mut seen = Vec::new();
    while let Some(remaining) = WAIT.checked_sub(start.elapsed()) {
        let Some(event) = net.recv_timeout(remaining) else {
            break;
        };
        let matched = done(&event);
        seen.push(event);
        if matched {
            return seen;
        }
    }
    panic!("timed out waiting for {what}; saw {seen:#?}");
}

fn is_connected(event: &NetEvent) -> bool {
    matches!(
        event,
        NetEvent::Connection(ConnectionState::Connected { .. })
    )
}

fn states(events: &[NetEvent]) -> Vec<ConnectionState> {
    events
        .iter()
        .filter_map(|e| match e {
            NetEvent::Connection(state) => Some(state.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn handshake_authenticates_then_subscribes() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(GOOD_TOKEN);

    let events = wait_for(&net, "Connected", is_connected);
    assert_eq!(
        states(&events),
        [
            ConnectionState::Connecting,
            ConnectionState::Authenticating,
            ConnectionState::Connected { player_id: PLAYER },
        ]
    );
    // The handshake replies are forwarded too.
    assert!(events.iter().any(
        |e| matches!(e, NetEvent::Ws(ServerMessage::AuthOk { player_id }) if *player_id == PLAYER)
    ));
    assert!(events
        .iter()
        .any(|e| matches!(e, NetEvent::Ws(ServerMessage::Subscribed { .. }))));

    assert_eq!(
        mock.received()[..2],
        [
            ClientMessage::Auth {
                token: GOOD_TOKEN.into()
            },
            ClientMessage::Subscribe {
                channel: "position".into()
            },
        ]
    );
}

#[test]
fn malformed_frames_are_skipped_and_the_session_survives() {
    let (_mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(GOOD_TOKEN);

    // The mock sends garbage, an unknown type, then `player_left`.
    let events = wait_for(&net, "player_left", |e| {
        matches!(e, NetEvent::Ws(ServerMessage::PlayerLeft { .. }))
    });
    assert!(!events.iter().any(|e| matches!(
        e,
        NetEvent::Connection(ConnectionState::Disconnected { .. })
    )));

    // Still usable afterwards.
    net.ws(ClientMessage::Move {
        x: 4100.0,
        y: 3900.0,
    });
    wait_for(&net, "positions", |e| {
        matches!(e, NetEvent::Ws(ServerMessage::Positions { .. }))
    });
}

#[test]
fn sends_and_receives_game_messages() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Connected", is_connected);

    net.ws(ClientMessage::Move {
        x: 4100.0,
        y: 3900.0,
    });
    let events = wait_for(&net, "positions", |e| {
        matches!(e, NetEvent::Ws(ServerMessage::Positions { .. }))
    });
    let Some(NetEvent::Ws(ServerMessage::Positions { players })) = events.last() else {
        unreachable!()
    };
    assert_eq!((players[0].x, players[0].y), (4100.0, 3900.0));
    assert!(mock.received().contains(&ClientMessage::Move {
        x: 4100.0,
        y: 3900.0
    }));
}

#[test]
fn messages_sent_during_the_handshake_wait_for_subscribe() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    net.ws(ClientMessage::Move { x: 1.0, y: 2.0 });

    wait_for(&net, "positions", |e| {
        matches!(e, NetEvent::Ws(ServerMessage::Positions { .. }))
    });
    assert_eq!(
        mock.received()[..3],
        [
            ClientMessage::Auth {
                token: GOOD_TOKEN.into()
            },
            ClientMessage::Subscribe {
                channel: "position".into()
            },
            ClientMessage::Move { x: 1.0, y: 2.0 },
        ]
    );
}

#[test]
fn a_rejected_token_ends_in_auth_failed() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect("stale-token");

    let events = wait_for(&net, "AuthFailed", |e| {
        matches!(e, NetEvent::Connection(ConnectionState::AuthFailed { .. }))
    });
    assert_eq!(
        states(&events).last(),
        Some(&ConnectionState::AuthFailed {
            message: "Invalid token".into()
        })
    );
    // No subscribe after a failed auth.
    assert_eq!(
        mock.received(),
        [ClientMessage::Auth {
            token: "stale-token".into()
        }]
    );
}

#[test]
fn a_silent_handshake_times_out() {
    let (_mock, addr) = Mock::start(Scenario::NeverAnswers);
    let net = Net::spawn(NetConfig {
        handshake_timeout: Duration::from_millis(300),
        ..config(addr)
    })
    .unwrap();
    net.connect(GOOD_TOKEN);

    let events = wait_for(&net, "Disconnected", |e| {
        matches!(
            e,
            NetEvent::Connection(ConnectionState::Disconnected { .. })
        )
    });
    assert_eq!(
        states(&events).last(),
        Some(&ConnectionState::Disconnected {
            reason: "handshake timed out".into()
        })
    );
}

#[test]
fn pings_keep_flowing_and_a_silent_server_is_dropped() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let fast = |addr| NetConfig {
        ping_interval: Duration::from_millis(50),
        idle_timeout: Duration::from_millis(400),
        ..config(addr)
    };
    let net = Net::spawn(fast(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Connected", is_connected);
    mock.wait_until("two pings", |log| {
        log.iter().filter(|t| *t == "ping").count() >= 2
    });

    // A server that stops reading stops answering pings, and is dropped.
    let (_mock, addr) = Mock::start(Scenario::GoesSilent);
    let net = Net::spawn(fast(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    let events = wait_for(&net, "Disconnected", |e| {
        matches!(
            e,
            NetEvent::Connection(ConnectionState::Disconnected { .. })
        )
    });
    assert!(states(&events).contains(&ConnectionState::Connected { player_id: PLAYER }));
    assert_eq!(
        states(&events).last(),
        Some(&ConnectionState::Disconnected {
            reason: "server stopped responding".into()
        })
    );
}

#[test]
fn disconnect_closes_the_socket() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Connected", is_connected);

    net.disconnect();
    wait_for(&net, "Disconnected", |e| {
        *e == NetEvent::Connection(ConnectionState::Disconnected {
            reason: "closed by client".into(),
        })
    });
    mock.wait_until("close", |log| log.iter().any(|t| t == "close"));

    // With no session, WS messages are dropped rather than queued.
    net.ws(ClientMessage::Move { x: 1.0, y: 1.0 });
    std::thread::sleep(Duration::from_millis(100));
    assert!(!mock
        .received()
        .contains(&ClientMessage::Move { x: 1.0, y: 1.0 }));
}

#[test]
fn reconnecting_replaces_the_old_session_in_order() {
    let (_mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Connected", is_connected);

    net.connect(GOOD_TOKEN);
    let events = wait_for(&net, "Connected again", is_connected);
    assert_eq!(
        states(&events),
        [
            ConnectionState::Disconnected {
                reason: "closed by client".into()
            },
            ConnectionState::Connecting,
            ConnectionState::Authenticating,
            ConnectionState::Connected { player_id: PLAYER },
        ]
    );
}

#[test]
fn rest_login_and_errors() {
    let (_mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();

    let login = |password: &str| {
        RestRequest::Login(LoginRequest {
            email: "t@example.com".into(),
            password: password.into(),
        })
    };
    net.rest(login("pw"), 1);
    net.rest(login("wrong"), 2);
    let mut results = std::collections::HashMap::new();
    while results.len() < 2 {
        if let NetEvent::Rest(tag, result) =
            wait_for(&net, "REST", |e| matches!(e, NetEvent::Rest(..)))
                .pop()
                .unwrap()
        {
            results.insert(tag, result);
        }
    }
    match &results[&1] {
        Ok(RestResponse::Auth(auth)) => {
            assert_eq!((auth.token.as_str(), auth.player_id), (GOOD_TOKEN, PLAYER))
        }
        other => panic!("login: {other:?}"),
    }
    let err = results[&2].clone().unwrap_err();
    assert_eq!(
        err,
        RestError::Http {
            status: 401,
            message: "Invalid credentials".into()
        }
    );
    assert!(err.is_unauthorized());

    // Without a token the server refuses; after `connect` it's sent as a bearer.
    net.rest(RestRequest::Recipes, 3);
    let events = wait_for(&net, "recipes", |e| matches!(e, NetEvent::Rest(3, _)));
    assert!(matches!(events.last(), Some(NetEvent::Rest(3, Err(e))) if e.is_unauthorized()));
    net.connect(GOOD_TOKEN);
    net.rest(RestRequest::Recipes, 4);
    let events = wait_for(&net, "recipes", |e| matches!(e, NetEvent::Rest(4, _)));
    assert!(matches!(
        events.last(),
        Some(NetEvent::Rest(4, Ok(RestResponse::Recipes(_))))
    ));

    // A 2xx of the wrong shape is a decode error, not a panic.
    net.rest(RestRequest::PlayerCurrency, 5);
    let events = wait_for(&net, "currency", |e| matches!(e, NetEvent::Rest(5, _)));
    assert!(matches!(
        events.last(),
        Some(NetEvent::Rest(5, Err(RestError::Decode(_))))
    ));
}

#[test]
fn unreachable_servers_report_rather_than_hang() {
    // Bind and drop to get a port that refuses connections.
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let net = Net::spawn(config(addr)).unwrap();

    net.rest(RestRequest::Health, 1);
    let events = wait_for(&net, "health", |e| matches!(e, NetEvent::Rest(1, _)));
    assert!(matches!(
        events.last(),
        Some(NetEvent::Rest(1, Err(RestError::Transport(_))))
    ));

    net.connect(GOOD_TOKEN);
    let events = wait_for(&net, "Disconnected", |e| {
        matches!(
            e,
            NetEvent::Connection(ConnectionState::Disconnected { .. })
        )
    });
    assert_eq!(states(&events)[0], ConnectionState::Connecting);
}

#[test]
fn dropping_net_closes_the_socket_and_joins() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Connected", is_connected);

    let start = Instant::now();
    drop(net);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "drop took {:?}",
        start.elapsed()
    );
    mock.wait_until("close", |log| log.iter().any(|t| t == "close"));
}
