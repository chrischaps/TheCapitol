//! The bot against a real server, through a proxy the test can cut. Ignored by
//! default; run with a local stack up (`docker-compose up -d`, then
//! `cargo run` in `backend/`):
//!
//! ```text
//! cargo test -p capitol-core -- --ignored
//! ```
//!
//! `CAPITOL_SERVER` overrides the default `http://localhost:3000`.

use std::io;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use capitol_core::bot::{Account, Bot};
use capitol_core::catalog::container_type;
use capitol_core::net::{ConnectionState, NetConfig};
use capitol_core::state::StateEvent;

/// A TCP relay to the server. [`Proxy::cut`] shuts every open connection at
/// both ends, which the client sees as a dropped socket and the server as a
/// closed one, and then the proxy keeps accepting.
struct Proxy {
    addr: SocketAddr,
    open: Arc<Mutex<Vec<TcpStream>>>,
}

impl Proxy {
    fn start(upstream: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let open = Arc::new(Mutex::new(Vec::new()));
        let streams = open.clone();
        thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { continue };
                let Ok(server) = TcpStream::connect(upstream) else {
                    continue;
                };
                let _ = client.set_nodelay(true);
                let _ = server.set_nodelay(true);
                let pipe = |from: &TcpStream, to: &TcpStream| -> io::Result<()> {
                    let (mut from, mut to) = (from.try_clone()?, to.try_clone()?);
                    thread::spawn(move || {
                        let _ = io::copy(&mut from, &mut to);
                        let _ = to.shutdown(Shutdown::Write);
                    });
                    Ok(())
                };
                if pipe(&client, &server).and(pipe(&server, &client)).is_err() {
                    continue;
                }
                streams.lock().unwrap().extend([client, server]);
            }
        });
        Proxy { addr, open }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Drops every connection open now, REST and WebSocket alike.
    fn cut(&self) -> usize {
        let streams: Vec<_> = self.open.lock().unwrap().drain(..).collect();
        for stream in &streams {
            let _ = stream.shutdown(Shutdown::Both);
        }
        streams.len() / 2
    }
}

/// The server's address from `CAPITOL_SERVER`, IPv4 first: `localhost`
/// resolves to `::1` first, and the backend listens on IPv4 only.
fn upstream() -> SocketAddr {
    let base = std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into());
    let host = base
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();
    assert!(
        !base.starts_with("https://"),
        "the proxy relays plain http only: {base}"
    );
    let mut addrs: Vec<_> = host.to_socket_addrs().unwrap().collect();
    addrs.sort_by_key(|a| !a.is_ipv4());
    addrs[0]
}

/// A fresh account per run, so the script starts from an empty inventory.
fn fresh_account() -> Account {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    Account::numbered(&format!("live{}", stamp % 100_000_000), 1)
}

/// The P2 exit test: the bot logs in, moves, extracts fiber and crafts rope.
/// Then the connection drops, and the bot reconnects, resyncs and carries on.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn the_bot_crafts_rope_and_survives_a_dropped_connection() {
    let proxy = Proxy::start(upstream());
    let mut bot =
        Bot::sign_in(NetConfig::with_base_url(proxy.url()), &fresh_account()).expect("signed in");
    assert_eq!(bot.count(container_type::CRAFTING_OUTPUT, "rope"), 0);

    let report = bot.run_script().expect("the script runs");
    println!("{report}");
    assert_eq!(report.crafted.item_type, "rope");
    assert!(!report.extractions.is_empty());
    assert!(report.walked > 0.0, "it moved");
    let rope = bot.count(container_type::CRAFTING_OUTPUT, "rope");
    assert_eq!(rope, report.crafted.quantity);
    let fiber = bot.count(container_type::PLAYER_INVENTORY, "fiber");

    // Drop everything. The client should notice, back off, reconnect through
    // the proxy, and refetch what it might have missed.
    let cut = proxy.cut();
    println!("cut {cut} connections");
    let seen = bot
        .until(
            "a reconnect and resync",
            Duration::from_secs(20),
            |s, seen| {
                let resynced = seen.iter().position(|e| *e == StateEvent::SnapshotStale);
                s.is_connected()
                    && resynced.is_some_and(|i| seen[i..].contains(&StateEvent::ContainersUpdated))
            },
        )
        .expect("reconnected");
    let reconnects = seen
        .iter()
        .filter(|e| {
            matches!(
                e,
                StateEvent::Connection(ConnectionState::Reconnecting { .. })
            )
        })
        .count();
    assert_eq!(reconnects, 1, "one drop, one reconnect");

    // The refetched inventory agrees with the one from before the drop.
    assert_eq!(bot.count(container_type::CRAFTING_OUTPUT, "rope"), rope);
    assert_eq!(bot.count(container_type::PLAYER_INVENTORY, "fiber"), fiber);

    // And the new session works: a short walk lands.
    let (x, y) = bot.position();
    bot.walk_to(x + 20.0, y)
        .expect("walked after the reconnect");
    let (after_x, _) = bot.position();
    assert!((after_x - (x + 20.0)).abs() < 1.0);
}

/// `--wander --near`, seen from the player they wander around: every
/// wanderer shows up, walks over, and keeps moving nearby.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn wanderers_gather_around_a_player() {
    const FLEET: usize = 3;
    const RADIUS: f64 = 60.0;
    let config = || {
        NetConfig::with_base_url(
            std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into()),
        )
    };
    // The watcher steps away from the spawn, so following them is a walk.
    let mut watcher = Bot::sign_in(config(), &fresh_account()).expect("watcher signed in");
    let (x, y) = watcher.position();
    watcher.walk_to(x + 200.0, y).expect("stepped away");
    let (wx, wy) = watcher.position();
    let watcher_name = watcher.name().to_string();
    let prefix = format!(
        "wander{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 100_000_000
    );
    let names: Vec<String> = (1..=FLEET)
        .map(|n| Account::numbered(&prefix, n).name)
        .collect();

    thread::scope(|scope| {
        for n in 1..=FLEET {
            let (prefix, near, config) = (&prefix, &watcher_name, config());
            scope.spawn(move || {
                let mut bot = Bot::sign_in(config, &Account::numbered(prefix, n))
                    .expect("wanderer signed in")
                    .quiet();
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                bot.wander(RADIUS, Some(near), Some(deadline))
                    .expect("wandered");
            });
        }

        // Where each wanderer was first seen; done once each has moved and
        // is within reach of the watcher.
        let mut first: Vec<Option<(f64, f64)>> = vec![None; FLEET];
        watcher
            .until(
                "the wanderers to gather",
                Duration::from_secs(25),
                |s, _| {
                    names.iter().zip(first.iter_mut()).all(|(name, start)| {
                        let Some(p) = s.players().values().find(|p| &p.name == name) else {
                            return false;
                        };
                        let (x, y) = *start.get_or_insert((p.x, p.y));
                        (p.x - x).hypot(p.y - y) > 5.0 && (p.x - wx).hypot(p.y - wy) <= RADIUS + 1.0
                    })
                },
            )
            .expect("saw them gather");
        println!("{FLEET} wanderers gathered around {watcher_name}");
    });
}
