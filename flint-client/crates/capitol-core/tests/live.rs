//! Against a real server. Ignored by default; run with a local stack up
//! (`docker-compose up -d`, then `cargo run` in `backend/`):
//!
//! ```text
//! cargo test -p capitol-core -- --ignored
//! ```
//!
//! `CAPITOL_SERVER` overrides the default `http://localhost:3000`.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capitol_core::net::*;
use capitol_core::poll::{Fetch, Poller};
use capitol_core::protocol::rest::{LoginRequest, RegisterRequest};
use capitol_core::protocol::rules;
use capitol_core::protocol::ws::{ClientMessage, ServerMessage};
use capitol_core::state::{GameState, StateEvent};

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
    net.ws(ClientMessage::Move {
        x: start.x + 20.0,
        y: start.y,
    });
    wait_for(&net, "movement", |e| {
        me(e).is_some_and(|p| (p.x - start.x).abs() > 1.0 || (p.y - start.y).abs() > 1.0)
    });

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
