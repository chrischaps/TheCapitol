//! Against a real server. Ignored by default; run with a local stack up
//! (`docker-compose up -d`, then `cargo run` in `backend/`):
//!
//! ```text
//! cargo test -p capitol-core -- --ignored
//! ```
//!
//! `CAPITOL_SERVER` overrides the default `http://localhost:3000`.

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
fn terrain_is_fetched_gzipped_then_revalidated() {
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
    });
    assert!(rest(&net, wrong, 3).unwrap_err().is_unauthorized());

    let auth = match rest(
        &net,
        RestRequest::Login(LoginRequest { email, password }),
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
        RestRequest::Logout(capitol_core::protocol::rest::LogoutRequest { token: auth.token }),
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
    let mut frames: Vec<(f64, f64)> = Vec::new();
    let run = |state: &mut GameState,
               motion: &mut Motion,
               frames: &mut Vec<(f64, f64)>,
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
                    frames.push((pose.x, pose.y));
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

    let jump = frames
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .fold(0.0, f64::max);
    println!(
        "{} frames, largest step {jump:.2}, lag {:?}, {} corrections",
        frames.len(),
        motion.lag(),
        motion.corrections()
    );
    assert_eq!(
        motion.corrections(),
        0,
        "the server walked where we predicted"
    );
    // A frame is at most ~16 ms of walking (0.8 units), with slack for a
    // slow frame on a busy machine.
    assert!(jump < 3.0, "largest step {jump}");
    let last = frames.last().unwrap();
    assert!(
        (last.0 - start.x).hypot(last.1 - start.y) < 1e-6,
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

    // Brin accepts, then Ada; the trade executes and the balances refetch.
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
    assert!(ada.fetched.contains(&Fetch::Currency), "{:?}", ada.fetched);
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

    // A third trade, and Brin drops. Ada gets the dummy-id cancel, which the
    // store ignores, then Brin's leave, which ends it.
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

    ada.net.disconnect();
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
    let token = remembered.token.clone().expect("the token is remembered");

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

    // The next run: the stored token goes straight in, no form.
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
    assert_eq!(remembered.token, None);
    assert_eq!(remembered.email.as_deref(), Some(email.as_str()));
    let effects = auth.log_in(&email, "not the password");
    drive_auth(&net, &mut auth, effects, &mut remembered, "a 401", |a| {
        a.phase() == Phase::SignedOut
    });
    assert_eq!(auth.notice(), Some(&Notice::WrongPassword));

    // The old token is refused once the logout lands, which sends the form
    // back with a notice.
    let stale = Remembered {
        email: Some(email.clone()),
        token: Some(token),
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
    assert_eq!(kept.token, None);
}
