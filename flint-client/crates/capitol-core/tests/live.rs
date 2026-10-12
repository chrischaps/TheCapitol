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
use capitol_core::protocol::rest::{LoginRequest, RegisterRequest};
use capitol_core::protocol::ws::{ClientMessage, ServerMessage};

const WAIT: Duration = Duration::from_secs(10);

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
