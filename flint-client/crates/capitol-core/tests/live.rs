//! Against a real server. Ignored by default; run with a local stack up
//! (`docker-compose up -d`, then `cargo run` in `backend/`):
//!
//! ```text
//! cargo test -p capitol-core -- --ignored
//! ```
//!
//! `CAPITOL_SERVER` overrides the default `http://localhost:3000`.

use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capitol_core::motion::Motion;
use capitol_core::net::*;
use capitol_core::poll::{Fetch, Poller};
use capitol_core::protocol::rest::{LoginRequest, RegisterRequest};
use capitol_core::protocol::rules;
use capitol_core::protocol::ws::{ClientMessage, ServerMessage};
use capitol_core::state::{GameState, StateEvent, Trade, TradeEvent, PARTNER_LEFT};

const WAIT: Duration = Duration::from_secs(10);
const FRAME: Duration = Duration::from_millis(16);

/// The tests share the server, except the walk, which times it. The other
/// live sessions on a debug backend can hold up a walk by more than the
/// prediction's window (#98).
static SERVER: RwLock<()> = RwLock::new(());

fn shared() -> RwLockReadGuard<'static, ()> {
    SERVER
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn alone() -> RwLockWriteGuard<'static, ()> {
    SERVER
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn net() -> Net {
    let base = std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into());
    Net::spawn(NetConfig::with_base_url(base)).unwrap()
}

fn wait_for(net: &Net, what: &str, done: impl Fn(&NetEvent) -> bool) -> NetEvent {
    let start = Instant::now();
    let mut seen = Vec::new();
    while let Some(remaining) = WAIT.checked_sub(start.elapsed()) {
        let Some(event) = net.recv_timeout(remaining) else {
            break;
        };
        if done(&event) {
            return event;
        }
        if seen.len() < 20 {
            seen.push(event);
        }
    }
    panic!("timed out waiting for {what}; first events: {seen:#?}");
}

fn rest(net: &Net, request: RestRequest, tag: Tag) -> Result<RestResponse, RestError> {
    net.rest(request, tag);
    match wait_for(
        net,
        "REST reply",
        |e| matches!(e, NetEvent::Rest(t, _) if *t == tag),
    ) {
        NetEvent::Rest(_, result) => result,
        _ => unreachable!(),
    }
}

#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn the_catalog_is_public_and_health_names_its_version() {
    let _server = shared();
    let net = net();
    let catalog = match rest(&net, RestRequest::Catalog, 1) {
        Ok(RestResponse::Catalog(catalog)) => catalog,
        other => panic!("/catalog: {other:?}"),
    };
    assert_eq!(catalog.version.len(), 16);
    assert!(catalog.item_types.iter().any(|t| t.id == "fiber"));
    assert!(catalog.recipes.iter().any(|r| r.id == "rope_basic"));
    let health = match rest(&net, RestRequest::Health, 2) {
        Ok(RestResponse::Health(health)) => health,
        other => panic!("/health: {other:?}"),
    };
    assert_eq!(
        health.catalog_version.as_deref(),
        Some(catalog.version.as_str())
    );

    // The store takes it whole, and a matching health asks for nothing more.
    let mut state = GameState::new();
    let changes = state.apply(&NetEvent::Rest(
        1,
        Ok(RestResponse::Catalog(catalog.clone())),
    ));
    assert!(changes.contains(&StateEvent::CatalogUpdated));
    assert_eq!(state.catalog().data(), &*catalog);
    let changes = state.apply(&NetEvent::Rest(2, Ok(RestResponse::Health(health))));
    assert!(!changes.contains(&StateEvent::CatalogStale));
}

#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn terrain_is_fetched_gzipped_then_revalidated() {
    let _server = shared();
    let net = net();
    // No token: /terrain is public. (That gzip is offered is checked against
    // the mock in tests/net.rs.)
    let Ok(RestResponse::Terrain(terrain)) = rest(
        &net,
        RestRequest::Terrain {
            cached_version: None,
        },
        1,
    ) else {
        panic!("expected the terrain");
    };
    let version = terrain.version.clone().expect("B3 adds a version");
    assert!(terrain.terrain_plots.len() > 2000);

    let unchanged = rest(
        &net,
        RestRequest::Terrain {
            cached_version: Some(version),
        },
        2,
    );
    assert_eq!(unchanged, Ok(RestResponse::TerrainUnchanged));

    let stale = rest(
        &net,
        RestRequest::Terrain {
            cached_version: Some("0000000000000000".into()),
        },
        3,
    );
    assert!(matches!(stale, Ok(RestResponse::Terrain(_))));
}

#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn register_login_connect_and_move() {
    let _server = shared();
    let net = net();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let email = format!("flint-net-{stamp}@test.local");
    let password = "correct horse battery staple".to_string();

    let registered = match rest(
        &net,
        RestRequest::Register(RegisterRequest {
            email: email.clone(),
            password: password.clone(),
            player_name: format!("Net{}", stamp % 100_000),
            refresh: false,
        }),
        1,
    ) {
        Ok(RestResponse::Auth(auth)) => auth,
        other => panic!("register: {other:?}"),
    };

    // Registering again is a conflict, decoded from the server's {"error": ...}.
    let again = RestRequest::Register(RegisterRequest {
        email: email.clone(),
        password: password.clone(),
        player_name: "Dup".into(),
        refresh: false,
    });
    assert_eq!(
        rest(&net, again, 2).unwrap_err(),
        RestError::Http {
            status: 409,
            message: "Email already registered".into()
        }
    );

    let wrong = RestRequest::Login(LoginRequest {
        email: email.clone(),
        password: "nope".into(),
        refresh: false,
    });
    assert!(rest(&net, wrong, 3).unwrap_err().is_unauthorized());

    let auth = match rest(
        &net,
        RestRequest::Login(LoginRequest {
            email,
            password,
            refresh: false,
        }),
        4,
    ) {
        Ok(RestResponse::Auth(auth)) => auth,
        other => panic!("login: {other:?}"),
    };
    assert_eq!(auth.player_id, registered.player_id);

    net.connect(auth.token.clone());
    let player_id = match wait_for(&net, "Connected", |e| {
        matches!(
            e,
            NetEvent::Connection(ConnectionState::Connected { .. })
                | NetEvent::Connection(ConnectionState::AuthFailed { .. })
        )
    }) {
        NetEvent::Connection(ConnectionState::Connected { player_id }) => player_id,
        other => panic!("handshake: {other:?}"),
    };
    assert_eq!(player_id, auth.player_id);

    // The bearer token rides along on authenticated routes.
    assert!(matches!(
        rest(&net, RestRequest::Containers, 5),
        Ok(RestResponse::Containers(_))
    ));
    assert!(matches!(
        rest(&net, RestRequest::Recipes, 6),
        Ok(RestResponse::Recipes(_))
    ));

    // Subscribed, so positions flow; a move shows up in them.
    let me = |e: &NetEvent| match e {
        NetEvent::Ws(ServerMessage::Positions { players }) => {
            players.iter().find(|p| p.id == player_id).cloned()
        }
        _ => None,
    };
    let start = me(&wait_for(&net, "our position", |e| me(e).is_some())).unwrap();
    // North, up the south arterial from the spawn: no water for 200 units.
    net.ws(ClientMessage::Move {
        x: start.x,
        y: start.y - 200.0,
    });
    wait_for(&net, "movement", |e| {
        me(e).is_some_and(|p| start.y - p.y > 50.0)
    });

    // Mid-walk, /player/me is the tick loop's position, not the row saved at
    // registration: it matches the next positions to within a tick or two.
    let Ok(RestResponse::Me(live)) = rest(&net, RestRequest::PlayerMe, 8) else {
        panic!("expected /player/me");
    };
    let next = me(&wait_for(&net, "positions after /player/me", |e| {
        me(e).is_some()
    }))
    .unwrap();
    assert!(live.online);
    assert_eq!(live.player_id, player_id);
    assert_eq!(live.name, start.name);
    assert_eq!(
        live.zone,
        rules::Zone::from_position(live.x, live.y).as_str()
    );
    assert!(
        start.y - live.y > 50.0,
        "{live:?} is the saved spawn, not live"
    );
    assert!(
        (live.x - next.x).abs() < 15.0 && (live.y - next.y).abs() < 15.0,
        "{live:?} vs positions {next:?}"
    );

    net.rest(
        RestRequest::Logout(capitol_core::protocol::rest::LogoutRequest {
            token: Some(auth.token),
            refresh_token: None,
        }),
        7,
    );
    net.disconnect();
    wait_for(&net, "logout", |e| matches!(e, NetEvent::Rest(7, Ok(_))));
}

/// A live session wired the way the game wires it: the store applies each
/// event and the poller turns the results into refetches.
struct Mirror {
    net: Net,
    state: GameState,
    poller: Poller,
    /// Everything the poller sent, for the assertions.
    fetched: Vec<Fetch>,
    /// The `trade_id` of every `trade_cancelled`, ours or not.
    cancels: Vec<uuid::Uuid>,
}

impl Mirror {
    /// Applies events until `done` holds and the poller has nothing queued or
    /// in flight. Returns the events seen.
    fn pump_until(
        &mut self,
        what: &str,
        done: impl Fn(&GameState, &[StateEvent]) -> bool,
    ) -> Vec<StateEvent> {
        let start = Instant::now();
        let mut seen = Vec::new();
        while let Some(remaining) = WAIT.checked_sub(start.elapsed()) {
            // Short waits stand in for frames, so the poller's timer runs.
            if let Some(event) = self.net.recv_timeout(remaining.min(FRAME)) {
                if let NetEvent::Rest(_, Err(e)) = &event {
                    panic!("REST failed while waiting for {what}: {e}");
                }
                if let NetEvent::Ws(ServerMessage::TradeCancelled { trade_id, .. }) = &event {
                    self.cancels.push(*trade_id);
                }
                let changes = self.state.apply(&event);
                self.poller.observe(&event, &changes, &self.state);
                seen.extend(changes);
            }
            seen.extend(self.state.tick(Instant::now()));
            for (request, tag) in self.poller.tick(Instant::now()) {
                self.fetched.push(self.poller.in_flight(tag).unwrap());
                self.net.rest(request, tag);
            }
            if self.poller.is_idle() && done(&self.state, &seen) {
                return seen;
            }
        }
        let tail: Vec<_> = seen.iter().rev().take(20).collect();
        panic!("timed out waiting for {what}; last events: {tail:#?}");
    }
}

#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn the_store_mirrors_a_real_session() {
    let _server = shared();
    let net = net();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let name = format!("Mirror{}", stamp % 100_000);
    let auth = match rest(
        &net,
        RestRequest::Register(RegisterRequest {
            email: format!("flint-state-{stamp}@test.local"),
            password: "correct horse battery staple".into(),
            player_name: name.clone(),
            refresh: false,
        }),
        1,
    ) {
        Ok(RestResponse::Auth(auth)) => auth,
        other => panic!("register: {other:?}"),
    };
    let mut mirror = Mirror {
        net,
        state: GameState::new(),
        poller: Poller::new(),
        fetched: Vec::new(),
        cancels: Vec::new(),
    };
    mirror.net.connect(auth.token.clone());

    // The first Connected asks for a snapshot; the answers fill the store.
    mirror.pump_until("a full snapshot", |s, _| {
        s.local_player().is_some()
            && s.inventory().is_some()
            && !s.recipes().is_empty()
            && s.strands().is_some()
            && s.zone().is_some()
    });
    let state = &mirror.state;
    assert_eq!(mirror.fetched, Fetch::SNAPSHOT, "the snapshot, once each");
    // Everyone's claimed lots came with it (P8.6b), even if there are none.
    assert!(state.claimed_version() > 0, "no /plots/claimed answer");
    assert_eq!(state.me(), Some(auth.player_id));
    assert_eq!(state.local_player().unwrap().name, name);
    assert!(state.crafting_input().is_some() && state.crafting_output().is_some());
    assert!(state.recipe("rope_basic").is_some());
    println!(
        "snapshot: {} players, {} nodes, {} stations, {} plots, zone {:?}, {} strands",
        state.players().len(),
        state.nodes().len(),
        state.stations().len(),
        state.plots().len(),
        state.zone(),
        state.strands().unwrap()
    );

    // Walk to the nearest available node and extract it.
    let me = state.local_player().unwrap().clone();
    let distance = |x: f64, y: f64| ((x - me.x).powi(2) + (y - me.y).powi(2)).sqrt();
    let Some(node) = state
        .nodes()
        .values()
        .filter(|n| n.state == "available")
        .min_by(|a, b| distance(a.x, a.y).total_cmp(&distance(b.x, b.y)))
        .cloned()
    else {
        println!("no available node near spawn; skipping extraction");
        return;
    };
    mirror.net.ws(ClientMessage::Move {
        x: node.x,
        y: node.y,
    });
    mirror.pump_until("arriving at the node", |s, _| {
        let p = s.local_player().unwrap();
        ((p.x - node.x).powi(2) + (p.y - node.y).powi(2)).sqrt() < rules::EXTRACTION_RANGE - 5.0
    });
    mirror.fetched.clear();
    mirror.net.ws(ClientMessage::Extract {
        node_id: node.id.to_string(),
    });
    let events = mirror.pump_until("the extraction to finish", |_, seen| {
        seen.iter()
            .any(|e| matches!(e, StateEvent::ExtractionCompleted { .. }))
    });
    assert!(events.contains(&StateEvent::ExtractionStarted { node_id: node.id }));
    assert!(!mirror.state.is_extracting());

    // inventory_updated → refetch → the fiber is in a slot.
    mirror.pump_until("fiber in the inventory", |s, _| {
        s.inventory()
            .is_some_and(|inv| inv.items().any(|i| i.item_type == "fiber"))
    });

    assert!(
        mirror.fetched.contains(&Fetch::Containers),
        "{:?}",
        mirror.fetched
    );

    mirror.net.disconnect();
}

#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn the_prediction_agrees_with_a_real_walk() {
    let _server = alone();
    let net = net();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let auth = match rest(
        &net,
        RestRequest::Register(RegisterRequest {
            email: format!("flint-motion-{stamp}@test.local"),
            password: "correct horse battery staple".into(),
            player_name: format!("Walker{}", stamp % 100_000),
            refresh: false,
        }),
        1,
    ) {
        Ok(RestResponse::Auth(auth)) => auth,
        other => panic!("register: {other:?}"),
    };
    net.connect(auth.token.clone());

    let mut state = GameState::new();
    let mut motion = Motion::new();
    // Frames until `done`, drawing the local player each frame.
    let mut frames: Vec<(Instant, f64, f64)> = Vec::new();
    let run = |state: &mut GameState,
               motion: &mut Motion,
               frames: &mut Vec<(Instant, f64, f64)>,
               what: &str,
               done: &dyn Fn(&GameState, &Motion) -> bool| {
        let start = Instant::now();
        while start.elapsed() < WAIT {
            if let Some(event) = net.recv_timeout(FRAME) {
                let changes = state.apply(&event);
                motion.observe(Instant::now(), &changes, state);
            }
            let now = Instant::now();
            if let Some(me) = state.me() {
                motion.sample(now);
                if let Some(pose) = motion.pose(me) {
                    frames.push((now, pose.x, pose.y));
                }
            }
            if done(state, motion) {
                return;
            }
        }
        panic!("timed out waiting for {what}");
    };
    run(
        &mut state,
        &mut motion,
        &mut frames,
        "our position",
        &|s, m| s.me().and_then(|me| m.pose(me)).is_some(),
    );
    let start = state.local_player().unwrap().clone();

    // Out 25 east and back, the second click mid-walk, as a player would.
    let legs = [(start.x + 25.0, start.y), (start.x, start.y)];
    for (i, &(x, y)) in legs.iter().enumerate() {
        motion.move_to(Instant::now(), x, y);
        net.ws(ClientMessage::Move { x, y });
        let at = |s: &GameState| {
            let p = s.local_player().unwrap();
            (p.x - x).hypot(p.y - y) < 1e-6
        };
        if i + 1 < legs.len() {
            run(&mut state, &mut motion, &mut frames, "halfway", &|s, _| {
                (s.local_player().unwrap().x - start.x) > 15.0
            });
        } else {
            run(
                &mut state,
                &mut motion,
                &mut frames,
                "the walk home",
                &|s, m| at(s) && m.destination().is_none(),
            );
        }
    }
    // A few more frames, so the server's last snapshots are checked too.
    let settle = Instant::now();
    run(&mut state, &mut motion, &mut frames, "settling", &|_, _| {
        settle.elapsed() > Duration::from_millis(800)
    });

    // How far each frame stepped beyond walking speed for its own length.
    // A slow frame on a busy machine steps further, but only a correction
    // (a blend or a snap) steps faster than a walk, bar the last half unit
    // that arrival snaps across (#98).
    let speed = motion.config().speed;
    let (jump, excess) = frames.windows(2).fold((0.0, 0.0), |(jump, excess), w| {
        let step = (w[1].1 - w[0].1).hypot(w[1].2 - w[0].2);
        let walk = speed * (w[1].0 - w[0].0).as_secs_f64();
        (f64::max(jump, step), f64::max(excess, step - walk))
    });
    println!(
        "{} frames, largest step {jump:.2}, beyond a walk {excess:.4}, lag {:?}, {} corrections",
        frames.len(),
        motion.lag(),
        motion.corrections()
    );
    assert_eq!(
        motion.corrections(),
        0,
        "the server walked where we predicted"
    );
    assert!(
        excess <= rules::ARRIVAL_EPSILON,
        "a frame stepped {excess} beyond a walk"
    );
    let last = frames.last().unwrap();
    assert!(
        (last.1 - start.x).hypot(last.2 - start.y) < 1e-6,
        "home: {last:?}"
    );
    net.disconnect();
}

impl Mirror {
    /// Registers a fresh account and connects it, up to a full snapshot.
    fn register(label: &str) -> Mirror {
        let net = net();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let auth = match rest(
            &net,
            RestRequest::Register(RegisterRequest {
                email: format!("flint-{label}-{stamp}@test.local"),
                password: "correct horse battery staple".into(),
                player_name: format!("{label}{}", stamp % 100_000),
                refresh: false,
            }),
            1,
        ) {
            Ok(RestResponse::Auth(auth)) => auth,
            other => panic!("register {label}: {other:?}"),
        };
        let mut mirror = Mirror {
            net,
            state: GameState::new(),
            poller: Poller::new(),
            fetched: Vec::new(),
            cancels: Vec::new(),
        };
        mirror.net.connect(auth.token);
        mirror.pump_until("a snapshot", |s, _| {
            s.local_player().is_some() && s.strands().is_some()
        });
        mirror
    }

    fn me(&self) -> uuid::Uuid {
        self.state.me().unwrap()
    }

    /// Runs a trade action and sends the message it returns.
    fn act(
        &mut self,
        action: impl FnOnce(
            &mut GameState,
            Instant,
        ) -> Result<ClientMessage, capitol_core::state::TradeRefusal>,
    ) {
        let message = action(&mut self.state, Instant::now()).expect("the action is allowed");
        self.net.ws(message);
    }
}

fn has(seen: &[StateEvent], wanted: impl Fn(&TradeEvent) -> bool) -> bool {
    seen.iter()
        .any(|e| matches!(e, StateEvent::Trade(t) if wanted(t)))
}

/// Two fresh players (both spawn at the same point, so in range) trade 10
/// strands, then open and cancel a trade, then lose one to a disconnect.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn two_players_trade() {
    let _server = shared();
    let mut ada = Mirror::register("Ada");
    let mut brin = Mirror::register("Brin");
    let (ada_id, brin_id) = (ada.me(), brin.me());
    ada.pump_until("Brin in the world", |s, _| s.player(brin_id).is_some());
    let start = ada.state.strands().unwrap();
    let brin_start = brin.state.strands().unwrap();

    // Request, accept, open.
    ada.act(|s, now| s.trade_initiate(now, brin_id));
    brin.pump_until(
        "Ada's request",
        |s, _| matches!(s.trade(), Trade::Incoming { from, .. } if *from == ada_id),
    );
    brin.act(|s, now| s.trade_accept_request(now));
    brin.pump_until("the trade to open", |s, _| s.open_trade().is_some());
    ada.pump_until("the trade to open", |s, _| s.open_trade().is_some());
    let trade_id = ada.state.open_trade().unwrap().trade_id;
    assert_eq!(brin.state.open_trade().unwrap().trade_id, trade_id);

    // Ada offers 10 strands; both see it.
    ada.act(|s, now| s.trade_offer(now, vec![], 10));
    ada.pump_until("our offer echoed", |s, _| {
        s.open_trade().unwrap().mine.strands == 10 && s.trade_waiting().is_none()
    });
    brin.pump_until("Ada's offer", |s, _| {
        s.open_trade().unwrap().theirs.strands == 10
    });

    // Brin accepts, then Ada; the trade executes and the server pushes
    // both balances (`currency_changed`, #148).
    brin.act(|s, now| s.trade_accept(now));
    ada.pump_until("Brin's accept", |s, _| {
        s.open_trade().unwrap().they_accepted
    });
    ada.fetched.clear();
    ada.act(|s, now| s.trade_accept(now));
    let seen = ada.pump_until("the trade to execute", |s, _| {
        s.trade() == &Trade::Idle && s.strands() == Some(start - 10)
    });
    assert!(has(&seen, |t| *t == TradeEvent::Executed { trade_id }));
    brin.pump_until("Brin's strands", |s, _| {
        s.trade() == &Trade::Idle && s.strands() == Some(brin_start + 10)
    });

    // Brin asks this time; Ada accepts, then cancels.
    brin.act(|s, now| s.trade_initiate(now, ada_id));
    ada.pump_until("Brin's request", |s, _| {
        matches!(s.trade(), Trade::Incoming { .. })
    });
    ada.act(|s, now| s.trade_accept_request(now));
    ada.pump_until("the second trade", |s, _| s.open_trade().is_some());
    brin.pump_until("the second trade", |s, _| s.open_trade().is_some());
    ada.act(|s, now| s.trade_cancel(now));
    let seen = brin.pump_until("the cancel", |s, _| s.trade() == &Trade::Idle);
    assert!(has(
        &seen,
        |t| matches!(t, TradeEvent::Cancelled { reason, .. } if reason == "Cancelled by player")
    ));
    ada.pump_until("the cancel", |s, _| s.trade() == &Trade::Idle);

    // A third trade, and Brin drops. Ada gets the cancel with the trade's real
    // id (B6, #48), which ends it, then Brin's leave.
    ada.act(|s, now| s.trade_initiate(now, brin_id));
    brin.pump_until("the third request", |s, _| {
        matches!(s.trade(), Trade::Incoming { .. })
    });
    brin.act(|s, now| s.trade_accept_request(now));
    ada.pump_until("the third trade", |s, _| s.open_trade().is_some());
    let third = ada.state.open_trade().unwrap().trade_id;
    brin.net.disconnect();
    let seen = ada.pump_until("Brin's leave to end it", |s, _| s.trade() == &Trade::Idle);
    assert!(has(&seen, |t| *t
        == TradeEvent::Cancelled {
            trade_id: third,
            reason: PARTNER_LEFT.into()
        }));
    assert!(ada.cancels.contains(&third), "{:?}", ada.cancels);

    ada.net.disconnect();
}

/// B5 (#47): the server answers refused trade commands with `error`, and the
/// store turns one that arrives while an action waits into `Refused`.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn trade_refusals_come_back_as_errors() {
    use capitol_core::protocol::ws::TradeOfferItem;
    use capitol_core::state::TradeAction;
    let _server = shared();

    fn refused<'a>(seen: &'a [StateEvent], action: TradeAction) -> Option<&'a str> {
        seen.iter().find_map(|e| match e {
            StateEvent::Trade(TradeEvent::Refused { action: a, reason }) if *a == action => {
                Some(reason.as_str())
            }
            _ => None,
        })
    }

    let mut ada = Mirror::register("Ada");
    let mut brin = Mirror::register("Brin");
    let mut cato = Mirror::register("Cato");
    let (ada_id, brin_id) = (ada.me(), brin.me());

    // Accepting a request nobody sent. The store would refuse it, so send it raw.
    brin.net.ws(ClientMessage::TradeAcceptRequest);
    brin.pump_until("the server's refusal", |_, seen| {
        seen.iter().any(|e| {
            matches!(e, StateEvent::ServerError { message, command }
                if message == "No pending trade request" && command == "trade_accept_request")
        })
    });

    // Ada and Brin open a trade.
    ada.pump_until("Brin in the world", |s, _| s.player(brin_id).is_some());
    ada.act(|s, now| s.trade_initiate(now, brin_id));
    brin.pump_until("Ada's request", |s, _| {
        matches!(s.trade(), Trade::Incoming { .. })
    });
    brin.act(|s, now| s.trade_accept_request(now));
    ada.pump_until("the trade to open", |s, _| s.open_trade().is_some());

    // Cato asks Ada, who's busy.
    cato.pump_until("Ada in the world", |s, _| s.player(ada_id).is_some());
    cato.act(|s, now| s.trade_initiate(now, ada_id));
    let seen = cato.pump_until("the refusal", |s, _| s.trade_waiting().is_none());
    let reason = refused(&seen, TradeAction::Initiate).expect("a refused initiate");
    assert!(reason.contains("already in a trade"), "{reason}");
    assert_eq!(cato.state.trade(), &Trade::Idle);

    // Ada offers an item she doesn't own, then more strands than she has.
    ada.act(|s, now| {
        s.trade_offer(
            now,
            vec![TradeOfferItem {
                item_id: uuid::Uuid::from_u128(0xdead_beef).to_string(),
                quantity: 1,
                qualities: None,
            }],
            0,
        )
    });
    let seen = ada.pump_until("the refusal", |s, _| s.trade_waiting().is_none());
    let reason = refused(&seen, TradeAction::Offer).expect("a refused offer");
    assert!(reason.contains("don't own"), "{reason}");

    let balance = ada.state.strands().unwrap();
    ada.act(|s, now| s.trade_offer(now, vec![], balance + 1));
    let seen = ada.pump_until("the refusal", |s, _| s.trade_waiting().is_none());
    let reason = refused(&seen, TradeAction::Offer).expect("a refused offer");
    assert!(reason.contains("strands"), "{reason}");
    assert_eq!(ada.state.open_trade().unwrap().mine.strands, 0);

    ada.act(|s, now| s.trade_cancel(now));
    ada.pump_until("the cancel", |s, _| s.trade() == &Trade::Idle);
    for mirror in [&ada, &brin, &cato] {
        mirror.net.disconnect();
    }
}

/// B9 (#60): a refused `extract` comes back as `extraction_failed` with the
/// reason, to the player who asked and nobody else.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn extraction_refusals_say_why() {
    let _server = shared();
    fn refusal(seen: &[StateEvent]) -> Option<&str> {
        seen.iter().find_map(|e| match e {
            StateEvent::ExtractionFailed { reason } => Some(reason.as_str()),
            _ => None,
        })
    }

    let mut ada = Mirror::register("Ada");
    let mut brin = Mirror::register("Brin");
    let brin_id = brin.me();
    ada.pump_until("Brin in the world", |s, _| s.player(brin_id).is_some());

    // A node that doesn't exist, and an id that isn't one.
    ada.net.ws(ClientMessage::Extract {
        node_id: uuid::Uuid::from_u128(0xdead_beef).to_string(),
    });
    let seen = ada.pump_until("the refusal", |_, seen| refusal(seen).is_some());
    assert!(refusal(&seen).unwrap().contains("gone"), "{seen:?}");
    ada.net.ws(ClientMessage::Extract {
        node_id: "not-a-node".into(),
    });
    let seen = ada.pump_until("the refusal", |_, seen| refusal(seen).is_some());
    assert!(refusal(&seen).unwrap().contains("not-a-node"), "{seen:?}");

    // The furthest node in sight is out of reach from the spawn.
    let me = ada.state.local_player().unwrap().clone();
    let far = ada
        .state
        .nodes()
        .values()
        .max_by(|a, b| {
            (a.x - me.x)
                .hypot(a.y - me.y)
                .total_cmp(&(b.x - me.x).hypot(b.y - me.y))
        })
        .expect("nodes near the spawn (B1)")
        .clone();
    let distance = (far.x - me.x).hypot(far.y - me.y);
    assert!(distance > rules::EXTRACTION_RANGE, "{distance}");
    ada.net.ws(ClientMessage::Extract {
        node_id: far.id.to_string(),
    });
    let seen = ada.pump_until("the refusal", |_, seen| refusal(seen).is_some());
    let reason = refusal(&seen).unwrap();
    assert!(reason.starts_with("Too far"), "{reason}");
    assert!(!ada.state.is_extracting());

    // None of Ada's refusals reached Brin.
    let mut seen = Vec::new();
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        seen.extend(brin.pump_until("a frame", |_, _| true));
    }
    assert_eq!(refusal(&seen), None, "{seen:?}");

    for mirror in [&ada, &brin] {
        mirror.net.disconnect();
    }
}

/// Carries out an [`Auth`]'s effects on `net`, keeping what it asks to
/// remember, and feeds it events until `done` holds.
fn drive_auth(
    net: &Net,
    auth: &mut capitol_core::auth::Auth,
    effects: Vec<capitol_core::auth::Effect>,
    remembered: &mut capitol_core::auth::Remembered,
    what: &str,
    done: impl Fn(&capitol_core::auth::Auth) -> bool,
) {
    use capitol_core::auth::Effect;
    let mut pending = effects;
    let start = Instant::now();
    loop {
        for effect in pending.drain(..) {
            match effect {
                Effect::Rest(request, tag) => net.rest(request, tag),
                Effect::Connect(token) => net.connect(token),
                Effect::Disconnect => net.disconnect(),
                Effect::Remember(r) => *remembered = r,
            }
        }
        if done(auth) {
            return;
        }
        assert!(
            start.elapsed() < WAIT,
            "timed out waiting for {what}: {:?}, {:?}",
            auth.phase(),
            auth.notice()
        );
        pending.extend(auth.tick(Instant::now()));
        if let Some(event) = net.recv_timeout(FRAME) {
            pending.extend(auth.observe(&event));
        }
    }
}

#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn the_login_form_signs_in_out_and_back() {
    use capitol_core::auth::{Auth, Notice, Phase, Remembered};
    let _server = shared();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let email = format!("flint-auth-{stamp}@test.local");
    let password = "correct horse battery staple";
    let mut remembered = Remembered::default();

    // A first run: nothing remembered, so the form. The server's health is
    // known before anything is submitted.
    let net = net();
    let (mut auth, effects) = Auth::new(remembered.clone(), 10);
    assert_eq!(auth.phase(), Phase::SignedOut);
    drive_auth(&net, &mut auth, effects, &mut remembered, "health", |a| {
        a.reachable() == Some(true)
    });

    // Signing in to an account that doesn't exist is a wrong password, and
    // never registers it.
    let effects = auth.log_in(&email, password);
    drive_auth(&net, &mut auth, effects, &mut remembered, "a 401", |a| {
        a.phase() == Phase::SignedOut
    });
    assert_eq!(auth.notice(), Some(&Notice::WrongPassword));

    // Create account, then into the world.
    let name = format!("Auth{}", stamp % 100_000);
    let effects = auth.register(&name, &email, password);
    drive_auth(
        &net,
        &mut auth,
        effects,
        &mut remembered,
        "the world",
        |a| a.phase() == Phase::InWorld,
    );
    assert_eq!(remembered.email.as_deref(), Some(email.as_str()));
    let token = remembered
        .refresh_token
        .clone()
        .expect("the session is remembered");

    // Registering it again: taken. (A second Auth, signed out.)
    let other = self::net();
    let mut scratch = Remembered::default();
    let (mut second, _) = Auth::new(Remembered::default(), 10);
    let effects = second.register(&name, &email, password);
    drive_auth(&other, &mut second, effects, &mut scratch, "a 409", |a| {
        a.phase() == Phase::SignedOut
    });
    assert_eq!(second.notice(), Some(&Notice::EmailTaken));
    drop(other);

    // The next run: the stored session goes straight in, no form. Its
    // refresh token is swapped for the next, and that one is kept.
    drop(net);
    let net = self::net();
    let (mut auth, effects) = Auth::new(remembered.clone(), 10);
    assert_eq!(auth.phase(), Phase::Entering { resumed: true });
    drive_auth(
        &net,
        &mut auth,
        effects,
        &mut remembered,
        "the world again",
        |a| a.phase() == Phase::InWorld,
    );
    let rotated = remembered.refresh_token.clone();
    assert!(
        rotated.is_some() && rotated.as_ref() != Some(&token),
        "{rotated:?}"
    );

    // Sign out: the email stays, the token goes. Then a wrong password on
    // the real account says so.
    let effects = auth.log_out();
    drive_auth(
        &net,
        &mut auth,
        effects,
        &mut remembered,
        "signed out",
        |a| a.phase() == Phase::SignedOut,
    );
    assert_eq!(remembered.refresh_token, None);
    assert_eq!(remembered.email.as_deref(), Some(email.as_str()));
    let effects = auth.log_in(&email, "not the password");
    drive_auth(&net, &mut auth, effects, &mut remembered, "a 401", |a| {
        a.phase() == Phase::SignedOut
    });
    assert_eq!(auth.notice(), Some(&Notice::WrongPassword));

    // The first refresh token is spent, and its session has ended: it's
    // refused, which sends the form back with a notice.
    let stale = Remembered {
        email: Some(email.clone()),
        refresh_token: Some(token),
    };
    let mut kept = stale.clone();
    let (mut auth, effects) = Auth::new(stale, 10);
    drive_auth(
        &net,
        &mut auth,
        effects,
        &mut kept,
        "the stale token refused",
        |a| a.phase() == Phase::SignedOut,
    );
    assert!(matches!(auth.notice(), Some(Notice::SessionEnded { .. })));
    assert_eq!(kept.refresh_token, None);
}

#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn a_read_key_watches_cant_move_and_revoking_it_drops_its_socket() {
    use capitol_core::protocol::rest::{ApiKeyScope, CreateApiKeyRequest};
    let _server = shared();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let owner = net();
    let auth = match rest(
        &owner,
        RestRequest::Register(RegisterRequest {
            email: format!("flint-key-{stamp}@test.local"),
            password: "correct horse battery staple".into(),
            player_name: format!("Key{}", stamp % 100_000),
            refresh: true,
        }),
        1,
    ) {
        Ok(RestResponse::Auth(auth)) => auth,
        other => panic!("register: {other:?}"),
    };
    owner.connect(&auth);
    wait_for(&owner, "the owner connected", |e| {
        matches!(e, NetEvent::Connection(ConnectionState::Connected { .. }))
    });
    let mint = CreateApiKeyRequest {
        name: "watcher".into(),
        scopes: vec![ApiKeyScope::Read],
    };
    let key = match rest(&owner, RestRequest::CreateApiKey(mint), 2) {
        Ok(RestResponse::CreatedApiKey(created)) => created,
        other => panic!("mint: {other:?}"),
    };
    assert_eq!(key.key.scopes, vec![ApiKeyScope::Read]);

    // The key watches as the owner's player...
    let watcher = net();
    watcher.connect(key.secret.as_str());
    let NetEvent::Connection(ConnectionState::Connected { player_id }) =
        wait_for(&watcher, "the watcher connected", |e| {
            matches!(e, NetEvent::Connection(ConnectionState::Connected { .. }))
        })
    else {
        unreachable!()
    };
    assert_eq!(player_id, auth.player_id);
    // ...and sees the world, but can't move.
    assert!(matches!(
        rest(&watcher, RestRequest::PlayerMe, 3),
        Ok(RestResponse::Me(_))
    ));
    watcher.ws(ClientMessage::Move { x: rules::CENTER_X, y: rules::CENTER_Y });
    let NetEvent::Ws(ServerMessage::Error { message, command }) =
        wait_for(&watcher, "the move refused", |e| {
            matches!(e, NetEvent::Ws(ServerMessage::Error { .. }))
        })
    else {
        unreachable!()
    };
    assert_eq!(command, "move");
    assert!(message.contains("`play`"), "{message}");
    // A key can't mint keys, even its own account's.
    let refused = rest(&watcher, RestRequest::ApiKeys, 4).unwrap_err();
    assert_eq!(refused.status(), Some(403), "{refused:?}");

    // Revoking it closes its socket, for good.
    assert!(matches!(
        rest(&owner, RestRequest::RevokeApiKey { key_id: key.key.id }, 5),
        Ok(RestResponse::Success(_))
    ));
    let NetEvent::Connection(ConnectionState::AuthFailed { message }) =
        wait_for(&watcher, "the watcher dropped", |e| {
            matches!(e, NetEvent::Connection(ConnectionState::AuthFailed { .. }))
        })
    else {
        unreachable!()
    };
    assert!(message.contains("revoked"), "{message}");
    assert!(rest(&watcher, RestRequest::PlayerMe, 6).unwrap_err().is_unauthorized());

    // All the while, the owner stayed connected: a watcher doesn't take
    // the player over.
    while let Some(event) = owner.try_recv() {
        assert!(
            !matches!(event, NetEvent::Connection(ConnectionState::AuthFailed { .. })),
            "the owner was signed out: {event:?}"
        );
    }
    let keys = match rest(&owner, RestRequest::ApiKeys, 7) {
        Ok(RestResponse::ApiKeys(keys)) => keys,
        other => panic!("list: {other:?}"),
    };
    assert!(keys[0].revoked_at.is_some() && keys[0].last_used_at.is_some());
}
