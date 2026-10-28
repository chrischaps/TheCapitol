//! Drives `Net` against a mock server built on axum, the backend's own stack.
//! The mock mirrors `backend/src/ws/connection.rs` closely enough to exercise
//! the handshake, and adds misbehaviour the real server doesn't (malformed
//! frames, silence) to prove the client survives it.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use capitol_core::net::*;
use capitol_core::protocol::rest::{
    AccountInfo, AuthResponse, LoginRequest, RecipesResponse, RefreshRequest,
};
use capitol_core::protocol::ws::{ClientMessage, PlayerPosition, ServerMessage};
use uuid::Uuid;

const GOOD_TOKEN: &str = "good-token";
/// A bot's API key (E0.9), which `/recipes` takes as `ApiKey`.
const GOOD_KEY: &str = "cap_good-key";
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
    /// Hangs up without a Close frame straight after `subscribed`, the way
    /// the backend cuts a client that falls 1024 events behind. Only the
    /// first `times` connections; after that it's `Normal`.
    DropsAfterSubscribe { times: usize },
    /// The first connection is dropped after `subscribed`. Every later one
    /// is refused with `Session expired`, as if the account logged in
    /// somewhere else meanwhile.
    RevokedWhileAway,
    /// The first `auth` gets `Database error: …`, the rest succeed.
    DatabaseHiccup,
}

#[derive(Clone)]
struct Mock {
    scenario: Scenario,
    /// What the server received on the socket, in order: raw JSON text,
    /// `"ping"` or `"close"`.
    log: Arc<Mutex<Vec<String>>>,
    /// WebSocket connections accepted so far.
    connections: Arc<AtomicUsize>,
    /// Refreshes granted so far. The good refresh token is `r{n + 1}`.
    refreshes: Arc<AtomicUsize>,
}

impl Mock {
    fn start(scenario: Scenario) -> (Mock, SocketAddr) {
        let mock = Mock {
            scenario,
            log: Arc::default(),
            connections: Arc::default(),
            refreshes: Arc::default(),
        };
        let app = Router::new()
            .route("/auth/login", post(login))
            .route("/auth/refresh", post(refresh))
            .route("/recipes", get(recipes))
            .route("/terrain", get(terrain))
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
        expires_in: 0,
        refresh_token: None,
        account: AccountInfo {
            id: Uuid::nil(),
            email: req.email,
            tier: "free".into(),
            is_admin: false,
            kind: Default::default(),
        },
        player_id: PLAYER,
    })
    .into_response()
}

/// Like the backend's (E0.6): each refresh token works once, and is swapped
/// for `GOOD_TOKEN`, good for an hour, and the next refresh token.
async fn refresh(State(mock): State<Mock>, Json(req): Json<RefreshRequest>) -> Response {
    let granted = mock.refreshes.load(Ordering::SeqCst);
    if req.refresh_token != format!("r{}", granted + 1) {
        let body = serde_json::json!({ "error": "Invalid refresh token" });
        return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
    }
    mock.refreshes.fetch_add(1, Ordering::SeqCst);
    Json(AuthResponse {
        token: GOOD_TOKEN.into(),
        expires_in: 3600,
        refresh_token: Some(format!("r{}", granted + 2)),
        account: AccountInfo {
            id: Uuid::nil(),
            email: "t@example.com".into(),
            tier: "free".into(),
            is_admin: false,
            kind: Default::default(),
        },
        player_id: PLAYER,
    })
    .into_response()
}

async fn recipes(headers: HeaderMap) -> Response {
    let given = headers.get("authorization").and_then(|v| v.to_str().ok());
    if given != Some(&format!("Bearer {GOOD_TOKEN}")) && given != Some(&format!("ApiKey {GOOD_KEY}")) {
        let body = serde_json::json!({ "error": "Missing authorization header" });
        return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
    }
    Json(RecipesResponse { recipes: vec![] }).into_response()
}

/// Like the backend's `/terrain` since B3: version `v1`, a weak ETag, and a
/// 304 for a matching `If-None-Match`. A client that doesn't offer gzip is
/// refused, since the real one would be sent a megabyte uncompressed.
async fn terrain(headers: HeaderMap) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    };
    if !header("accept-encoding").contains("gzip") {
        let body = serde_json::json!({ "error": "no gzip offered" });
        return (StatusCode::BAD_REQUEST, Json(body)).into_response();
    }
    if header("if-none-match") == "W/\"v1\"" {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    Json(serde_json::json!({
        "world_size": 13600.0, "center_x": 6800.0, "center_y": 6800.0,
        "water_rings": [], "roads": [], "terrain_plots": [], "version": "v1"
    }))
    .into_response()
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
    let connection = mock.connections.fetch_add(1, Ordering::SeqCst);
    if let Scenario::NeverAnswers = mock.scenario {
        tokio::time::sleep(Duration::from_secs(30)).await;
        return;
    }
    let refusal = match mock.scenario {
        Scenario::RevokedWhileAway if connection > 0 => Some("Session expired"),
        Scenario::DatabaseHiccup if connection == 0 => {
            Some("Database error: pool timed out while waiting for an open connection")
        }
        _ => None,
    };
    let drops_after_subscribe = match mock.scenario {
        Scenario::DropsAfterSubscribe { times } => connection < times,
        Scenario::RevokedWhileAway => connection == 0,
        _ => false,
    };
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
            Ok(ClientMessage::Auth { .. }) if refusal.is_some() => {
                let message = refusal.unwrap().into();
                send(&mut socket, &ServerMessage::AuthError { message }).await;
                return;
            }
            Ok(ClientMessage::Auth { token }) if token == GOOD_TOKEN => {
                send(
                    &mut socket,
                    &ServerMessage::AuthOk {
                        player_id: PLAYER,
                        server_time_ms: 0,
                        subscription: Default::default(),
                        fees: Default::default(),
                        is_admin: false,
                        day_offset_ms: 0,
                    },
                )
                .await;
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
                if drops_after_subscribe {
                    return;
                }
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
                    citizen: false,
                    tag: None,
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

/// A server that accepts TCP and hangs up straight away. A refused port
/// would do, but Windows takes about 2 s to report each refusal.
fn hangs_up_at_once() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
        }
    });
    addr
}

/// Reconnects quickly, so tests don't wait on the real backoff.
fn quick_reconnect(addr: SocketAddr) -> NetConfig {
    NetConfig {
        reconnect_initial: Duration::from_millis(20),
        reconnect_max: Duration::from_millis(200),
        ..config(addr)
    }
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

fn is_reconnecting(event: &NetEvent) -> bool {
    matches!(
        event,
        NetEvent::Connection(ConnectionState::Reconnecting { .. })
    )
}

/// The reason given by the first `Reconnecting` in `events`.
fn reconnect_reason(events: &[NetEvent]) -> &str {
    events
        .iter()
        .find_map(|e| match e {
            NetEvent::Connection(ConnectionState::Reconnecting { reason, .. }) => {
                Some(reason.as_str())
            }
            _ => None,
        })
        .expect("a Reconnecting event")
}

/// Asserts nothing more arrives for a while: no retry, no stray event.
fn assert_quiet(net: &Net, for_: Duration) {
    if let Some(event) = net.recv_timeout(for_) {
        panic!("expected silence, got {event:?}");
    }
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
        |e| matches!(e, NetEvent::Ws(ServerMessage::AuthOk { player_id, .. }) if *player_id == PLAYER)
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
    let net = Net::spawn(quick_reconnect(addr)).unwrap();
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
    // No subscribe after a failed auth, and no second attempt, even though
    // a retry would have come after 20 ms.
    assert_quiet(&net, Duration::from_millis(300));
    assert_eq!(
        mock.received(),
        [ClientMessage::Auth {
            token: "stale-token".into()
        }]
    );
    assert_eq!(mock.connections.load(Ordering::SeqCst), 1);
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

    let events = wait_for(&net, "Reconnecting", is_reconnecting);
    assert_eq!(reconnect_reason(&events), "handshake timed out");
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
    let events = wait_for(&net, "Reconnecting", is_reconnecting);
    assert!(states(&events).contains(&ConnectionState::Connected { player_id: PLAYER }));
    assert_eq!(reconnect_reason(&events), "server stopped responding");
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
fn terrain_offers_gzip_and_revalidates_by_version() {
    let (_mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    let fetch = |cached_version: Option<&str>, tag: Tag| {
        net.rest(
            RestRequest::Terrain {
                cached_version: cached_version.map(String::from),
            },
            tag,
        );
        match wait_for(
            &net,
            "terrain",
            |e| matches!(e, NetEvent::Rest(t, _) if *t == tag),
        )
        .pop()
        {
            Some(NetEvent::Rest(_, result)) => result,
            other => panic!("{other:?}"),
        }
    };
    match fetch(None, 1) {
        Ok(RestResponse::Terrain(t)) => assert_eq!(t.version.as_deref(), Some("v1")),
        other => panic!("full fetch: {other:?}"),
    }
    assert_eq!(fetch(Some("v1"), 2), Ok(RestResponse::TerrainUnchanged));
    assert!(matches!(fetch(Some("v0"), 3), Ok(RestResponse::Terrain(_))));
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
    let events = wait_for(&net, "Reconnecting", is_reconnecting);
    assert_eq!(states(&events)[0], ConnectionState::Connecting);
    assert!(reconnect_reason(&events).starts_with("could not connect"));
}

/// The server answers one request per connection and keeps the connection
/// alive, then hangs up on the next request that arrives on it. That's what
/// a dropped server does to the client's pooled REST connections. A GET is
/// sent again on a new connection; anything else reports the failure.
#[test]
fn a_get_on_a_dead_pooled_connection_is_sent_again() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    std::thread::spawn(move || {
        // Reads one request head. None of these requests has a body that matters.
        let read_request = |stream: &mut std::net::TcpStream| {
            let mut head = Vec::new();
            let mut byte = [0u8];
            while !head.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    return false;
                }
                head.push(byte[0]);
            }
            counted.fetch_add(1, Ordering::SeqCst);
            true
        };
        let body = r#"{"status":"healthy","database":"ok","redis":"ok","server_time_ms":0}"#;
        let answer = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            if read_request(&mut stream) {
                stream.write_all(answer.as_bytes()).unwrap();
                read_request(&mut stream);
            }
        }
    });
    let net = Net::spawn(config(addr)).unwrap();
    let call = |request: RestRequest, tag| {
        net.rest(request, tag);
        match wait_for(
            &net,
            "REST",
            |e| matches!(e, NetEvent::Rest(t, _) if *t == tag),
        )
        .pop()
        {
            Some(NetEvent::Rest(_, result)) => result,
            _ => unreachable!(),
        }
    };

    assert!(matches!(
        call(RestRequest::Health, 1),
        Ok(RestResponse::Health(_))
    ));
    // Pooled, then hung up on, so it goes again on a fresh connection.
    assert!(matches!(
        call(RestRequest::Health, 2),
        Ok(RestResponse::Health(_))
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 3);

    // A POST isn't repeated: the server may have acted on it.
    let logout = RestRequest::Logout(capitol_core::protocol::rest::LogoutRequest {
        token: Some("t".into()),
        refresh_token: None,
        ..Default::default()
    });
    assert!(matches!(call(logout, 3), Err(RestError::Transport(_))));
    assert_eq!(requests.load(Ordering::SeqCst), 4);
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

#[test]
fn a_dropped_connection_reconnects_reauths_resubscribes_and_resyncs() {
    let (mock, addr) = Mock::start(Scenario::DropsAfterSubscribe { times: 1 });
    let net = Net::spawn(quick_reconnect(addr)).unwrap();
    net.connect(GOOD_TOKEN);

    let first = wait_for(&net, "Connected", is_connected);
    assert!(!first.contains(&NetEvent::Resync), "nothing to resync yet");
    let events = wait_for(&net, "Resync", |e| *e == NetEvent::Resync);
    let states = states(&events);
    assert!(
        matches!(
            states[..],
            [
                ConnectionState::Reconnecting { attempt: 1, .. },
                ConnectionState::Connecting,
                ConnectionState::Authenticating,
                ConnectionState::Connected { player_id: PLAYER },
            ]
        ),
        "{states:#?}"
    );
    // Resync comes straight after the second Connected.
    assert!(matches!(
        events[events.len() - 2..],
        [
            NetEvent::Connection(ConnectionState::Connected { .. }),
            NetEvent::Resync
        ]
    ));

    let handshake = [
        ClientMessage::Auth {
            token: GOOD_TOKEN.into(),
        },
        ClientMessage::Subscribe {
            channel: "position".into(),
        },
    ];
    assert_eq!(mock.received(), [handshake.clone(), handshake].concat());

    // And the new session works.
    net.ws(ClientMessage::Move {
        x: 4100.0,
        y: 3900.0,
    });
    wait_for(&net, "positions", |e| {
        matches!(e, NetEvent::Ws(ServerMessage::Positions { .. }))
    });
}

#[test]
fn backoff_grows_while_the_server_is_down() {
    let addr = hangs_up_at_once();
    let net = Net::spawn(NetConfig {
        reconnect_initial: Duration::from_millis(10),
        reconnect_max: Duration::from_millis(40),
        ..config(addr)
    })
    .unwrap();
    net.connect(GOOD_TOKEN);

    let mut waits = Vec::new();
    while waits.len() < 5 {
        let events = wait_for(&net, "Reconnecting", is_reconnecting);
        if let Some(NetEvent::Connection(ConnectionState::Reconnecting {
            attempt, delay, ..
        })) = events.last()
        {
            waits.push((*attempt, *delay));
        }
    }
    let nominal_ms = [10, 20, 40, 40, 40];
    for (i, (attempt, delay)) in waits.iter().enumerate() {
        assert_eq!(*attempt as usize, i + 1);
        let nominal = Duration::from_millis(nominal_ms[i]);
        assert!(
            *delay <= nominal && *delay >= nominal.mul_f64(0.8),
            "attempt {attempt} waited {delay:?}, expected about {nominal:?}"
        );
    }
}

#[test]
fn a_token_revoked_while_away_ends_in_auth_failed() {
    let (mock, addr) = Mock::start(Scenario::RevokedWhileAway);
    let net = Net::spawn(quick_reconnect(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Connected", is_connected);

    let events = wait_for(&net, "AuthFailed", |e| {
        matches!(e, NetEvent::Connection(ConnectionState::AuthFailed { .. }))
    });
    assert_eq!(
        states(&events).last(),
        Some(&ConnectionState::AuthFailed {
            message: "Session expired".into()
        })
    );
    assert_quiet(&net, Duration::from_millis(300));
    assert_eq!(mock.connections.load(Ordering::SeqCst), 2);
}

#[test]
fn a_database_hiccup_during_auth_is_retried() {
    let (_mock, addr) = Mock::start(Scenario::DatabaseHiccup);
    let net = Net::spawn(quick_reconnect(addr)).unwrap();
    net.connect(GOOD_TOKEN);

    let events = wait_for(&net, "Connected", is_connected);
    assert!(reconnect_reason(&events).starts_with("auth_error: Database error"));
    assert!(!states(&events)
        .iter()
        .any(|s| matches!(s, ConnectionState::AuthFailed { .. })));
    // The first connection never went live, so there's nothing to resync.
    // A Resync would land before the `player_left` the mock sends next.
    let events = wait_for(&net, "player_left", |e| {
        matches!(e, NetEvent::Ws(ServerMessage::PlayerLeft { .. }))
    });
    assert!(!events.contains(&NetEvent::Resync));
}

#[test]
fn disconnect_during_backoff_stops_retrying() {
    let addr = hangs_up_at_once();
    let net = Net::spawn(NetConfig {
        reconnect_initial: Duration::from_secs(30),
        ..config(addr)
    })
    .unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Reconnecting", is_reconnecting);

    // Sent while waiting to reconnect: dropped, not queued.
    net.ws(ClientMessage::Move { x: 1.0, y: 1.0 });
    let start = Instant::now();
    net.disconnect();
    let events = wait_for(&net, "Disconnected", |e| {
        matches!(
            e,
            NetEvent::Connection(ConnectionState::Disconnected { .. })
        )
    });
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(
        states(&events),
        [ConnectionState::Disconnected {
            reason: "closed by client".into()
        }]
    );
    assert_quiet(&net, Duration::from_millis(200));
}

fn is_refreshed(event: &NetEvent) -> bool {
    matches!(event, NetEvent::Refreshed { .. })
}

#[test]
fn a_401_refreshes_the_token_and_sends_the_request_again() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(Credentials {
        token: Some("expired".into()),
        refresh_token: Some("r1".into()),
        expires_in: None,
    });
    net.rest(RestRequest::Recipes, 1);
    let events = wait_for(&net, "the recipes", |e| matches!(e, NetEvent::Rest(1, _)));
    assert!(
        matches!(
            events.last(),
            Some(NetEvent::Rest(1, Ok(RestResponse::Recipes(_))))
        ),
        "{events:#?}"
    );
    assert!(events.contains(&NetEvent::Refreshed {
        refresh_token: "r2".into()
    }));
    // The socket got a fresh token too, since it asks the keyring each time.
    wait_for(&net, "Connected", is_connected);
    // One refresh between them: the socket's and the request's didn't both
    // spend `r1`.
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 1);
}

#[test]
fn an_api_key_is_sent_as_an_api_key_and_never_refreshed() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(GOOD_KEY);
    net.rest(RestRequest::Recipes, 1);
    let events = wait_for(&net, "the recipes", |e| matches!(e, NetEvent::Rest(1, _)));
    assert!(
        matches!(
            events.last(),
            Some(NetEvent::Rest(1, Ok(RestResponse::Recipes(_))))
        ),
        "{events:#?}"
    );
    mock.wait_until("the key on the socket", |log| {
        log.iter().any(|t| t.contains(GOOD_KEY))
    });
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 0);
}

#[test]
fn a_token_is_refreshed_before_it_expires() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(Credentials {
        token: Some(GOOD_TOKEN.into()),
        refresh_token: Some("r1".into()),
        expires_in: Some(Duration::from_secs(1)),
    });
    let started = Instant::now();
    wait_for(&net, "a refresh", is_refreshed);
    assert!(
        started.elapsed() >= Duration::from_millis(700),
        "refreshed too soon: {:?}",
        started.elapsed()
    );
    // The new token is good for an hour, so that's the only one.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_resumed_session_refreshes_before_it_connects() {
    let (mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(Credentials::resume("r1"));
    let events = wait_for(&net, "Connected", is_connected);
    assert!(events.iter().any(is_refreshed), "{events:#?}");
    assert!(mock.received().contains(&ClientMessage::Auth {
        token: GOOD_TOKEN.into()
    }));
}

#[test]
fn a_refused_refresh_token_ends_the_session_with_the_servers_reason() {
    let (_mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    net.connect(Credentials::resume("spent"));
    let events = wait_for(&net, "AuthFailed", |e| {
        matches!(e, NetEvent::Connection(ConnectionState::AuthFailed { .. }))
    });
    assert_eq!(
        events.last(),
        Some(&NetEvent::Connection(ConnectionState::AuthFailed {
            message: "Invalid refresh token".into()
        }))
    );
    // And a request made with no token left gets the server's 401.
    net.rest(RestRequest::Recipes, 1);
    let events = wait_for(&net, "the recipes", |e| matches!(e, NetEvent::Rest(1, _)));
    assert!(matches!(events.last(), Some(NetEvent::Rest(1, Err(e))) if e.is_unauthorized()));
}

/// A server whose rate limit refuses the first request (E0.7). The client
/// waits out `Retry-After` and sends it again, POST or not, since a `429` was
/// refused before anything happened. The caller sees only the answer.
#[test]
fn a_rate_limited_request_waits_and_goes_again() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    std::thread::spawn(move || {
        let auth = serde_json::to_string(&AuthResponse {
            token: GOOD_TOKEN.into(),
            expires_in: 0,
            refresh_token: None,
            account: AccountInfo {
                id: Uuid::nil(),
                email: "a@b.c".into(),
                tier: "free".into(),
                is_admin: false,
                kind: Default::default(),
            },
            player_id: PLAYER,
        })
        .unwrap();
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            // The head, then the body by its content-length.
            let mut head = Vec::new();
            let mut byte = [0u8];
            while !head.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    break;
                }
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|n| n.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let mut body = vec![0u8; length];
            stream.read_exact(&mut body).unwrap();
            let answer = if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                let body = r#"{"error":"Too many requests"}"#;
                format!(
                    "HTTP/1.1 429 Too Many Requests\r\nretry-after: 1\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                )
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{auth}",
                    auth.len()
                )
            };
            stream.write_all(answer.as_bytes()).unwrap();
        }
    });
    let net = Net::spawn(config(addr)).unwrap();
    let login = RestRequest::Login(LoginRequest {
        email: "a@b.c".into(),
        password: "pw".into(),
    });
    let start = Instant::now();
    net.rest(login, 1);
    let result = match wait_for(&net, "REST", |e| matches!(e, NetEvent::Rest(1, _))).pop() {
        Some(NetEvent::Rest(_, result)) => result,
        _ => unreachable!(),
    };
    assert!(matches!(result, Ok(RestResponse::Auth(_))), "{result:?}");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert!(
        start.elapsed() >= Duration::from_secs(1),
        "didn't wait: {:?}",
        start.elapsed()
    );
}

/// Polls `net.stats()` until `done` holds.
fn wait_for_stats(net: &Net, what: &str, done: impl Fn(&NetStats) -> bool) -> NetStats {
    let start = Instant::now();
    loop {
        let stats = net.stats();
        if done(&stats) {
            return stats;
        }
        assert!(start.elapsed() < WAIT, "stats never showed {what}: {stats:#?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn stats_time_the_round_trip_and_count_both_ways() {
    let (_mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(config(addr)).unwrap();
    assert_eq!(net.stats(), NetStats::default());
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Connected", is_connected);
    net.ws(ClientMessage::Move {
        x: 4100.0,
        y: 3900.0,
    });
    wait_for(&net, "positions", |e| {
        matches!(e, NetEvent::Ws(ServerMessage::Positions { .. }))
    });

    // A Ping goes as soon as the session is live, not 15 s later.
    let stats = wait_for_stats(&net, "a round trip", |s| s.rtt.is_some());
    let rtt = stats.rtt.unwrap();
    assert!(rtt < Duration::from_secs(1), "{rtt:?} on loopback");
    assert_eq!(stats.rtt_avg, Some(rtt));
    // auth, subscribe, the Ping and the move.
    assert!(stats.outbound.messages >= 4, "{stats:#?}");
    // auth_ok, subscribed, two malformed frames, player_left, positions, the Pong.
    let stats = wait_for_stats(&net, "everything in", |s| s.inbound.messages >= 7);
    assert!(stats.inbound.bytes > stats.outbound.bytes / 4, "{stats:#?}");
    assert_eq!(stats.reconnects, 0);
    assert_eq!(stats.motion, None);
}

#[test]
fn stats_rate_each_type_over_the_last_second() {
    let (_mock, addr) = Mock::start(Scenario::Normal);
    let net = Net::spawn(NetConfig {
        ping_interval: Duration::from_millis(50),
        ..config(addr)
    })
    .unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Connected", is_connected);

    // A whole second of pinging every 50 ms.
    let stats = wait_for_stats(&net, "pings by the second", |s| {
        s.outbound.rate(PING).is_some_and(|r| r.messages >= 10)
            && s.inbound.rate(PONG).is_some_and(|r| r.messages >= 10)
    });
    let pings = stats.outbound.rate(PING).unwrap();
    assert_eq!(pings.bytes, 8 * pings.messages as u64, "each Ping carries its number");
    assert!(pings.messages <= 21, "{pings:?}");
}

#[test]
fn stats_count_reconnects() {
    let (_mock, addr) = Mock::start(Scenario::DropsAfterSubscribe { times: 2 });
    let net = Net::spawn(quick_reconnect(addr)).unwrap();
    net.connect(GOOD_TOKEN);
    wait_for(&net, "Resync", |e| *e == NetEvent::Resync);
    wait_for(&net, "Resync", |e| *e == NetEvent::Resync);
    assert_eq!(net.stats().reconnects, 2);
}
