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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use capitol_core::bot::{Account, Bot, BotError};
use capitol_core::catalog::container_type;
use capitol_core::net::{Net, NetConfig, NetEvent, RestRequest, RestResponse};
use capitol_core::protocol::rest::{ApiKeyScope, AuthResponse, CreateApiKeyRequest, RegisterRequest, SubscribeRequest};
use capitol_core::protocol::rules;
use capitol_core::protocol::terrain::LandmarkKind;
use capitol_core::state::StateEvent;

/// A TCP relay to the server. [`Proxy::cut`] shuts every open connection at
/// both ends, which the client sees as a dropped socket and the server as a
/// closed one, and then the proxy keeps accepting. [`Proxy::outage`] also
/// turns new connections away for a while, as a restarting server does.
struct Proxy {
    addr: SocketAddr,
    open: Arc<Mutex<Vec<TcpStream>>>,
    down_until: Arc<Mutex<Option<Instant>>>,
}

impl Proxy {
    fn start(upstream: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let open = Arc::new(Mutex::new(Vec::new()));
        let streams = open.clone();
        let down_until = Arc::new(Mutex::new(None::<Instant>));
        let down = down_until.clone();
        thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { continue };
                if down.lock().unwrap().is_some_and(|until| Instant::now() < until) {
                    // Dropped, closing it
                    continue;
                }
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
        Proxy { addr, open, down_until }
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

    /// Drops every connection and turns new ones away for `how_long`.
    fn outage(&self, how_long: Duration) {
        *self.down_until.lock().unwrap() = Some(Instant::now() + how_long);
        self.cut();
    }
}

fn server_url() -> String {
    std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into())
}

/// Makes a REST call on `net` and waits for its answer.
fn rest(net: &Net, request: RestRequest, tag: u64) -> RestResponse {
    net.rest(request, tag);
    let end = Instant::now() + Duration::from_secs(10);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match net.recv_timeout(left) {
            Some(NetEvent::Rest(t, result)) if t == tag => return result.expect("the call succeeded"),
            Some(_) => {}
            None => break,
        }
    }
    panic!("no answer to call {tag}");
}

/// A fresh account and a `play` key for it, as a citizen's would be. The
/// owner is signed in over REST only, so the key's bot can hold the avatar;
/// connect `owner` with `auth` to act as them again.
fn play_key(what: &str) -> (Net, AuthResponse, String) {
    let account = fresh_account();
    let owner = Net::spawn(NetConfig::with_base_url(server_url())).unwrap();
    let RestResponse::Auth(auth) = rest(
        &owner,
        RestRequest::Register(RegisterRequest {
            email: account.email.replace("live", what),
            password: account.password,
            player_name: account.name.replace("Live", what),
        }),
        1,
    ) else {
        panic!("registered");
    };
    owner.connect(&auth);
    // Bots are for subscribers (#217): a debug server mocks the payment
    let RestResponse::Subscription(_) = rest(&owner, RestRequest::Subscribe(SubscribeRequest { days: 1 }), 10) else {
        panic!("subscribed");
    };
    let mint = CreateApiKeyRequest {
        name: what.into(),
        scopes: vec![ApiKeyScope::Play],
    };
    let RestResponse::CreatedApiKey(key) = rest(&owner, RestRequest::CreateApiKey(mint), 2) else {
        panic!("minted a key");
    };
    owner.disconnect();
    (owner, auth, key.secret)
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

/// A fresh account per call, so the script starts from an empty inventory.
/// Tests run side by side and start in the same millisecond, so the number
/// counts calls: two tests on one account share one avatar (E0.8).
fn fresh_account() -> Account {
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Account::numbered(&format!("live{}", stamp % 100_000_000), n)
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
    match bot.idle(Duration::from_secs(5)) {
        Err(BotError::Offline(reason)) => println!("dropped: {reason}"),
        other => panic!("the bot didn't notice the drop: {other:?}"),
    }
    bot.recover(Some(Instant::now() + Duration::from_secs(20)))
        .expect("reconnected and resynced");
    assert!(bot.state().is_connected());

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

/// E3.3 through capitol-core: one bot sells fiber, another watches fiber's
/// book, buys it, hears the fill and collects it; then places an order and
/// cancels it. Each step is checked in the watcher's book as the diffs keep it.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn bots_place_watch_fill_and_cancel_orders() {
    use capitol_core::market::MarketEvent;
    use capitol_core::protocol::rest::PlaceOrder;
    use capitol_core::protocol::rules::market::{GradeRule, OrderSide, OrderStatus};

    let config = || {
        NetConfig::with_base_url(
            std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into()),
        )
    };
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        % 100_000_000;
    let mut seller = Bot::sign_in(config(), &Account::numbered(&format!("seller{stamp}"), 1)).expect("seller signed in");
    let mut buyer = Bot::sign_in(config(), &Account::numbered(&format!("buyer{stamp}"), 1)).expect("buyer signed in");
    // Orders are placed and collected at the Exchange hall (E3.4)
    seller.go_to_exchange().expect("the seller reached the hall");
    buyer.go_to_exchange().expect("the buyer reached the hall");
    buyer.watch_market("fiber").expect("fiber's book arrived");
    // How much rests at a level now, by the buyer's copy of the book
    let at = |bot: &Bot, side: OrderSide, price: i64| -> i64 {
        bot.state()
            .market()
            .book("fiber")
            .map(|b| b.side(side).filter(|l| l.price == price).map(|l| l.quantity).sum())
            .unwrap_or(0)
    };
    // A price nobody else on a local stack quotes
    let price = 7 + (stamp as i64 % 20);
    let asks_before = at(&buyer, OrderSide::Sell, price);

    // The seller lists three of the exchange's fiber
    seller.withdraw_fiber(4).expect("withdrew fiber");
    let stack = seller
        .state()
        .inventory()
        .and_then(|inv| inv.items().find(|i| i.item_type == "fiber").map(|i| i.item_id))
        .expect("fiber in the inventory");
    let ask = seller
        .place_order(PlaceOrder {
            side: OrderSide::Sell,
            item_type: "fiber".into(),
            price,
            quantity: 3,
            item_id: Some(stack),
            qualities: None,
            min_grade: None,
            grade_rule: None,
        })
        .expect("placed the sell");
    assert_eq!(ask.status, OrderStatus::Open);
    buyer
        .until("the ask in the book", Duration::from_secs(10), |s, _| {
            s.market()
                .book("fiber")
                .is_some_and(|b| b.side(OrderSide::Sell).any(|l| l.price == price && l.quantity >= asks_before + 3))
        })
        .expect("the buyer's book shows the ask");

    // The buyer takes it: the fill, the trade on the channel, the ask gone
    let bid = buyer
        .place_order(PlaceOrder {
            side: OrderSide::Buy,
            item_type: "fiber".into(),
            price,
            quantity: 3,
            item_id: None,
            qualities: None,
            min_grade: Some("F".into()),
            grade_rule: Some(GradeRule::Average),
        })
        .expect("placed the buy");
    let seen = buyer
        .until("the fill", Duration::from_secs(10), |s, _| {
            s.market().order(bid.id).is_some_and(|o| o.status == OrderStatus::Filled)
                && s.market().trades("fiber").any(|t| t.buy_order_id == bid.id)
        })
        .expect("the buy filled");
    assert!(seen
        .iter()
        .any(|e| matches!(e, StateEvent::Market(m) if matches!(&**m, MarketEvent::OrderFilled { order, .. } if order.id == bid.id))));
    // The depth diff follows the fill, at the end of the matcher's cycle
    buyer
        .until("the ask gone from the book", Duration::from_secs(5), |s, _| {
            s.market().book("fiber").is_some_and(|b| {
                b.side(OrderSide::Sell).filter(|l| l.price == price).map(|l| l.quantity).sum::<i64>() == asks_before
            })
        })
        .expect("the filled ask left the book");
    seller
        .until("the seller hears it", Duration::from_secs(10), |s, _| {
            s.market().order(ask.id).is_some_and(|o| o.status == OrderStatus::Filled)
        })
        .expect("the sell filled");

    // The fiber waits in the buyer's market box until collected
    let fiber_before = buyer.count(container_type::PLAYER_INVENTORY, "fiber");
    let collected = buyer.collect_market_box().expect("collected");
    assert!(!collected.collected.is_empty());
    assert_eq!(buyer.count(container_type::PLAYER_INVENTORY, "fiber"), fiber_before + 3);

    // Place and cancel: the level comes and goes in the watcher's book
    let low = 1;
    let bids_before = at(&buyer, OrderSide::Buy, low);
    let resting = buyer
        .place_order(PlaceOrder {
            side: OrderSide::Buy,
            item_type: "fiber".into(),
            price: low,
            quantity: 2,
            item_id: None,
            qualities: None,
            min_grade: Some("A".into()),
            grade_rule: Some(GradeRule::EveryUnit),
        })
        .expect("placed a resting buy");
    buyer
        .until("the bid in the book", Duration::from_secs(10), |s, _| {
            s.market().book("fiber").is_some_and(|b| {
                b.side(OrderSide::Buy).any(|l| l.price == low && l.quantity >= bids_before + 2)
            })
        })
        .expect("the bid shows");
    assert!(buyer.my_orders().expect("my orders").iter().any(|o| o.id == resting.id));
    let cancelled = buyer.cancel_order(resting.id).expect("cancelled");
    assert_eq!(cancelled.status, OrderStatus::Cancelled);
    buyer
        .until("the bid gone", Duration::from_secs(10), |s, _| {
            s.market().order(resting.id).is_some_and(|o| o.status == OrderStatus::Cancelled)
                && s.market().book("fiber").is_some_and(|b| {
                    b.side(OrderSide::Buy).filter(|l| l.price == low).map(|l| l.quantity).sum::<i64>() == bids_before
                })
        })
        .expect("the cancel shows");
    assert!(buyer.state().market().open_orders().is_empty());
}

/// The server the C1.4 tests use: `CAPITOL_SERVER`, or the local default.
/// They take their goods and Strands from grants, so it needs `DEV_GRANTS=on`.
fn live() -> NetConfig {
    NetConfig::with_base_url(std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into()))
}

fn stamp() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() % 100_000_000
}

/// Grants `items` (type, quantity, quality) into the market box, then walks
/// to the hall and collects them.
fn granted(bot: &mut Bot, strands: i64, items: &[(&str, i32, i32)]) {
    use capitol_core::protocol::rest::{GrantItem, GrantRequest};
    bot.grant(GrantRequest {
        strands,
        items: items
            .iter()
            .map(|&(item_type, quantity, quality)| GrantItem { item_type: item_type.into(), quantity, quality })
            .collect(),
    })
    .expect("granted (the server needs DEV_GRANTS=on)");
    if !items.is_empty() {
        bot.go_to_exchange().expect("reached the hall");
        bot.fetch_market_box().expect("fetched the market box");
        bot.collect_market_box().expect("collected the grant");
    }
}

fn stack_of(bot: &Bot, item_type: &str) -> uuid::Uuid {
    bot.state()
        .inventory()
        .and_then(|inv| inv.items().find(|i| i.item_type == item_type).map(|i| i.item_id))
        .unwrap_or_else(|| panic!("no {item_type} in the inventory"))
}

/// C1.4, the bazaar through `Bot`: a merchant rents a stall (or keeps the one
/// it has), stocks rope there, and a customer finds the shop in the
/// directory, walks to it, browses and buys. The merchant hears the sale,
/// its store takes the unit off the listing, and it unlists the rest.
#[test]
#[ignore = "needs a local stack with DEV_GRANTS=on: docker-compose up -d, then cargo run in backend/"]
fn bots_keep_a_shop_and_buy_from_one() {
    use capitol_core::protocol::rest::{BuyListing, StockListing};
    use chrono::{DateTime, Utc};

    // One merchant every run: thirteen stalls would run out in a day of
    // fresh ones, since a stall can't be given up early (#268). It pays a
    // day more while that keeps it within the week.
    let mut merchant = Bot::sign_in(live(), &Account::numbered("merchant", 1)).expect("merchant signed in");
    let mut customer =
        Bot::sign_in(live(), &Account::numbered(&format!("customer{}", stamp()), 1)).expect("customer signed in");
    granted(&mut merchant, 500, &[("rope", 3, 64)]);
    granted(&mut customer, 500, &[]);

    let now = || DateTime::<Utc>::from(SystemTime::now());
    let name = format!("Ropes {}", stamp() % 1000);
    let stall_id = match merchant.my_stall().expect("our stall").shop {
        Some(shop) if shop.paid_until > now() + chrono::Duration::days(6) => shop.stall_id,
        Some(shop) => merchant.rent_stall(&shop.stall_id, 1, Some(&name)).expect("paid a day more").stall_id,
        None => {
            let free = merchant
                .bazaar()
                .expect("the directory")
                .stalls
                .into_iter()
                .find(|s| s.shop.is_none())
                .expect("a free stall");
            let shop = merchant.rent_stall(&free.stall_id, 1, Some(&name)).expect("rented");
            assert_eq!(Some(shop.owner_id), merchant.state().me());
            shop.stall_id
        }
    };
    assert_eq!(merchant.state().bazaar().my_shop().map(|s| s.stall_id.as_str()), Some(stall_id.as_str()));

    // Stock two of the three, at a price to tell this run's listing by
    let price = 50 + (stamp() as i64 % 50);
    let rope_before = merchant.count(container_type::PLAYER_INVENTORY, "rope");
    let listing = merchant
        .stock(StockListing { item_id: stack_of(&merchant, "rope"), quantity: 2, price, qualities: None })
        .expect("stocked");
    assert_eq!(listing.units.count(), 2);
    assert_eq!(merchant.count(container_type::PLAYER_INVENTORY, "rope"), rope_before - 2);
    assert!(merchant.state().bazaar().my_shop().unwrap().listings.iter().any(|l| l.id == listing.id));

    // The customer reads the directory from the spawn, then walks over and browses
    let directory = customer.bazaar().expect("the directory");
    let sign = directory.stalls.iter().find(|s| s.stall_id == stall_id).and_then(|s| s.shop.as_ref()).expect("the shop");
    assert!(sign.item_types.iter().any(|t| t == "rope"));
    let seen = customer.browse(&stall_id).expect("browsed");
    assert!(seen.listings.iter().any(|l| l.id == listing.id && l.price == price));
    assert!(customer.state().bazaar().shop(&stall_id).is_some(), "the store kept the shop");

    // ...and buys one
    let rope_held = customer.count(container_type::PLAYER_INVENTORY, "rope");
    let strands = customer.state().strands().expect("a balance");
    let bought = customer.buy(listing.id, BuyListing { quantity: 1, qualities: None }).expect("bought");
    assert_eq!(bought.units.count(), 1);
    assert_eq!(bought.paid, price);
    assert_eq!(customer.count(container_type::PLAYER_INVENTORY, "rope"), rope_held + 1);
    customer
        .until("the balance", Duration::from_secs(10), |s, _| s.strands() == Some(strands - price))
        .expect("paid for it");
    let still = |s: &capitol_core::state::GameState| {
        s.bazaar().shop(&stall_id).and_then(|shop| shop.listings.iter().find(|l| l.id == listing.id)).map(|l| l.units.count())
    };
    assert_eq!(still(customer.state()), Some(1));

    // The merchant hears it, and its shop shows one left
    merchant
        .until("the sale", Duration::from_secs(10), |_, seen| {
            seen.iter().any(|e| matches!(e, StateEvent::BazaarSale { paid, .. } if *paid == price))
        })
        .expect("the merchant heard the sale");
    let left = |m: &Bot| {
        m.state().bazaar().my_shop().and_then(|s| s.listings.iter().find(|l| l.id == listing.id)).map(|l| l.units.count())
    };
    assert_eq!(left(&merchant), Some(1));

    // Unlisting brings the last one home
    let rope_now = merchant.count(container_type::PLAYER_INVENTORY, "rope");
    merchant.unlist(listing.id).expect("unlisted");
    assert_eq!(left(&merchant), None);
    assert_eq!(merchant.count(container_type::PLAYER_INVENTORY, "rope"), rope_now + 1);
}

/// C1.4, a station through `Bot`: split a stack, put down a workbench, have a
/// stack that isn't a tool refused its slot, put a sickle in it, set it to
/// make knapped stone, start and stop it, take the sickle out, and take the
/// bench down again.
#[test]
#[ignore = "needs a local stack with DEV_GRANTS=on: docker-compose up -d, then cargo run in backend/"]
fn bots_split_stacks_and_set_tools_and_automation() {
    use capitol_core::bot::BotError;
    use capitol_core::protocol::rest::{AutomationMode, AutomationState};

    let mut bot = Bot::sign_in(live(), &Account::numbered(&format!("bench{}", stamp()), 1)).expect("signed in");
    let home = bot.position();
    granted(&mut bot, 0, &[("workbench_kit", 1, 60), ("stone_sickle", 1, 70), ("stone", 6, 50)]);

    // Split two stone off the six, into the first empty slot
    let stone = stack_of(&bot, "stone");
    let split = bot.split_stack(stone, 2, None).expect("split");
    let stacks: Vec<i32> = bot
        .state()
        .inventory()
        .unwrap()
        .items()
        .filter(|i| i.item_type == "stone")
        .map(|i| i.quantity)
        .collect();
    assert_eq!(split.item.quantity, 2);
    assert!(stacks.contains(&4) && stacks.contains(&2), "{stacks:?}");

    // A workbench back where we came in, on the first spot that takes it
    bot.walk_to(home.0, home.1).expect("walked home");
    let kit = stack_of(&bot, "workbench_kit");
    let mut bench = None;
    for k in 0..16 {
        let a = std::f64::consts::TAU * k as f64 / 8.0;
        let r = if k < 8 { 30.0 } else { 90.0 };
        let (x, y) = (home.0 + r * a.cos(), home.1 + r * a.sin());
        match bot.walk_to(x, y) {
            Ok(()) => {}
            Err(BotError::Blocked { .. }) => continue,
            Err(e) => panic!("{e}"),
        }
        match bot.place_station("workbench", kit, x, y) {
            Ok(placed) => {
                bench = Some(placed.id);
                break;
            }
            Err(BotError::Rest { .. }) => {}
            Err(e) => panic!("{e}"),
        }
    }
    let bench = bench.expect("a spot for the workbench");

    // A stack that isn't a tool is refused, with the server's reason
    let refused = bot.install_tool(bench, stone).expect_err("stone isn't a tool");
    assert!(matches!(refused, BotError::Failed(_)), "{refused}");
    println!("refused: {refused}");

    // The sickle goes in, and leaves the inventory
    let sickle = stack_of(&bot, "stone_sickle");
    let tool = bot.install_tool(bench, sickle).expect("installed");
    assert_eq!(tool.item_id, sickle);
    assert_eq!(bot.count(container_type::PLAYER_INVENTORY, "stone_sickle"), 0);

    // Configure, start, stop: the store's station keeps up
    let set = bot.configure_automation(bench, "knapped_stone", AutomationMode::Count(1)).expect("configured");
    assert_eq!(set.state, AutomationState::Stopped);
    assert_eq!(set.recipe_id, "knapped_stone");
    let running = bot.start_automation(bench).expect("started");
    // Its box is empty, so it may already wait for stone
    assert!(matches!(running.state, AutomationState::Running | AutomationState::Paused { .. }), "{running:?}");
    let stopped = bot.stop_automation(bench).expect("stopped");
    assert_eq!(stopped.state, AutomationState::Stopped);
    bot.until("the station to show it", Duration::from_secs(5), |s, _| {
        s.station(bench).and_then(|st| st.automation.as_ref()).is_some_and(|a| a.state == AutomationState::Stopped)
    })
    .expect("the store's station is stopped");

    // The sickle comes back out
    bot.remove_tool(bench).expect("removed");
    assert_eq!(bot.count(container_type::PLAYER_INVENTORY, "stone_sickle"), 1);
    assert!(bot.state().station(bench).is_some_and(|s| s.tool.is_none()));

    bot.remove_station(bench).expect("took the bench down");
}

/// C0.6: a bot on a key outlives the server going away. The proxy turns
/// every connection away for a while, as a restart does; the bot notices,
/// drops what it was doing, and is back in the world, holding the avatar
/// and walking, within 30 s of the server's return.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn a_key_bot_is_working_again_within_30_s_of_a_restart() {
    const OUTAGE: Duration = Duration::from_secs(8);
    let proxy = Proxy::start(upstream());
    let (_owner, _auth, key) = play_key("restart");
    let mut bot = Bot::with_api_key(NetConfig::with_base_url(proxy.url()), &key).expect("in the world");
    let (x, y) = bot.position();
    bot.walk_to(x + 10.0, y).expect("walked before");
    let inventory = bot.state().inventory().map(|c| c.used_slots());

    proxy.outage(OUTAGE);
    let back = Instant::now() + OUTAGE;
    let (x, y) = bot.position();
    match bot.walk_to(x - 10.0, y) {
        Err(BotError::Offline(reason)) => println!("dropped mid-walk: {reason}"),
        other => panic!("the walk didn't stop at the drop: {other:?}"),
    }
    bot.recover(Some(back + Duration::from_secs(30)))
        .expect("back in the world");
    let took = Instant::now().saturating_duration_since(back);
    println!("back {:.1} s after the server", took.as_secs_f64());
    assert!(took < Duration::from_secs(30), "took {took:?}");
    assert!(bot.state().plays_here(), "it holds the avatar again");
    assert_eq!(bot.state().inventory().map(|c| c.used_slots()), inventory);

    let (x, y) = bot.position();
    bot.walk_to(x + 10.0, y).expect("walks again");
}

/// C0.6: revoking a running bot's key stops it, with `Revoked`, however it
/// was carrying on.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn a_revoked_key_stops_the_bot_with_revoked() {
    let (owner, auth, key) = play_key("revoke");
    let result = thread::scope(|scope| {
        let running = scope.spawn(|| {
            let mut bot = Bot::with_api_key(NetConfig::with_base_url(server_url()), &key)
                .expect("in the world")
                .quiet();
            let deadline = Instant::now() + Duration::from_secs(30);
            bot.stay_up(Some(deadline), |bot| {
                while Instant::now() < deadline {
                    bot.idle(Duration::from_millis(250))?;
                }
                Ok(())
            })
        });
        // Give it time to sign in, then revoke its key as its owner.
        thread::sleep(Duration::from_secs(3));
        owner.connect(&auth);
        let RestResponse::ApiKeys(keys) = rest(&owner, RestRequest::ApiKeys, 3) else {
            panic!("listed the keys");
        };
        let key_id = keys.iter().find(|k| k.revoked_at.is_none()).expect("a live key").id;
        rest(&owner, RestRequest::RevokeApiKey { key_id }, 4);
        running.join().unwrap()
    });
    match result {
        Err(BotError::Revoked(message)) => assert!(message.contains("revoked"), "{message}"),
        other => panic!("the bot didn't stop with Revoked: {other:?}"),
    }
}

/// C0.5 (#211): a walk across the outer canal, away from its bridges, goes
/// round by one, and the hall is reached from outside the canals. Before
/// the walk graph, both stopped at the water with `movement_blocked`.
#[test]
#[ignore = "needs a local stack: docker-compose up -d, then cargo run in backend/"]
fn the_bot_walks_round_the_canal_by_a_bridge() {
    let mut bot = Bot::sign_in(live(), &fresh_account()).expect("signed in");
    let walks = bot.walks().expect("the walk graph");
    // 0° is east and 90° south. The outer canal is r 1000, bridged every 45°.
    let polar = |r: f64, degrees: f64| {
        let a = degrees.to_radians();
        (rules::CENTER_X + r * a.cos(), rules::CENTER_Y + r * a.sin())
    };
    let (outside, inside) = (polar(1100.0, 20.0), polar(900.0, 20.0));
    assert!(!walks.walkable(outside, inside), "no bridge at 20°");

    for (x, y) in [outside, inside] {
        bot.walk_to(x, y).unwrap_or_else(|e| panic!("walking to ({x:.0}, {y:.0}): {e}"));
        let (at_x, at_y) = bot.position();
        assert!((at_x - x).hypot(at_y - y) < 1.0, "at ({at_x:.0}, {at_y:.0})");
    }
    bot.walk_to(outside.0, outside.1).expect("back out");
    bot.go_to_exchange().expect("at the hall from outside the canal");
    let door = walks.landmark(LandmarkKind::ExchangeHall).expect("a hall").door;
    let (x, y) = bot.position();
    assert!((x - door.0).hypot(y - door.1) < 1.0);
}

/// C1.5: a citizen answers a person's trade where it stands. It lays out its
/// logs; a short offer is countered ("for that, you get this"), and then a
/// lowball is declined; a fair offer for the lot is taken, and the trade
/// goes through. The person plays the trade window as a person would.
#[test]
#[ignore = "needs a local stack with DEV_GRANTS=on: docker-compose up -d, then cargo run in backend/"]
fn a_citizen_counters_a_short_offer_declines_a_lowball_and_takes_a_fair_one() {
    use capitol_core::citizen::decide::{Mind, Temperament, Who};
    use capitol_core::citizen::meet::{self, Answering, Met};
    use capitol_core::state::{Offer, Trade, TradeEvent};

    let logs = |offer: &Offer| -> i32 { offer.items.iter().filter(|i| i.item_type == "log").map(|i| i.quantity).sum() };
    let mut citizen = Bot::sign_in(live(), &Account::numbered(&format!("citizen{}", stamp()), 1)).expect("citizen signed in");
    let mut person = Bot::sign_in(live(), &Account::numbered(&format!("person{}", stamp()), 1)).expect("person signed in");
    granted(&mut citizen, 0, &[("log", 40, 50)]);
    granted(&mut person, 5000, &[]);
    // Where the citizen collected its logs
    person.go_to_exchange().expect("the person reached the hall");
    citizen.watch_market("log").expect("the log book");
    person.watch_market("log").expect("the log book");
    // What the citizen will think a log is worth: the books, or a bid of
    // the person's if the book is bare
    if meet::worth(person.state(), "log").is_none() {
        use capitol_core::protocol::rest::PlaceOrder;
        use capitol_core::protocol::rules::market::OrderSide;
        person
            .place_order(PlaceOrder {
                side: OrderSide::Buy,
                item_type: "log".into(),
                price: 10,
                quantity: 1,
                item_id: None,
                qualities: None,
                min_grade: None,
                grade_rule: None,
            })
            .expect("a bid to price logs by");
        citizen.until("the bid", Duration::from_secs(5), |s, _| meet::worth(s, "log").is_some()).expect("a worth");
    }
    let worth = meet::worth(citizen.state(), "log").expect("logs have a worth");
    assert_eq!(meet::worth(person.state(), "log"), Some(worth), "both see the same books");

    // The citizen answers trades until it has answered two
    let them = citizen.state().me().expect("the citizen's id");
    citizen.answer_trades(true);
    let answering = thread::spawn(move || {
        let me = Answering {
            who: Who {
                name: citizen.name().to_string(),
                role: "gatherer-wood".into(),
                temperament: Temperament::default(),
            },
            product: Some("log"),
            hours: 0.0,
        };
        let mut mind = Mind::rules();
        let mut met = Vec::new();
        let end = Instant::now() + Duration::from_secs(150);
        while met.len() < 2 && Instant::now() < end {
            if citizen.asked_to_trade() {
                met.push(meet::answer(&mut citizen, &mut mind, &me).expect("answered"));
            } else {
                citizen.idle(Duration::from_millis(100)).expect("idle");
            }
        }
        (citizen, met)
    });
    let ask = |person: &mut Bot| {
        person.until("the citizen in sight", Duration::from_secs(10), |s, _| s.player(them).is_some()).expect("in sight");
        person.trade_act(|s, now| s.trade_initiate(now, them)).expect("asked");
        person
            .until("the citizen's wares", Duration::from_secs(15), |s, _| s.open_trade().is_some_and(|o| logs(&o.theirs) == 40))
            .expect("its forty logs laid out");
    };
    let offer = |person: &mut Bot, strands: i64| {
        person.trade_act(|s, now| s.trade_offer(now, Vec::new(), strands)).expect("offered");
    };

    // Short: ten logs' worth for forty. Countered down to ten logs, which
    // the citizen accepts; then the person comes down to two logs' worth
    ask(&mut person);
    offer(&mut person, 10 * worth);
    person
        .until("the counter", Duration::from_secs(15), |s, _| {
            s.open_trade().is_some_and(|o| logs(&o.theirs) == 10 && o.they_accepted)
        })
        .expect("countered to ten logs, and accepted");
    offer(&mut person, 2 * worth);
    let seen = person
        .until("the decline", Duration::from_secs(15), |s, _| matches!(s.trade(), Trade::Idle))
        .expect("declined");
    assert!(seen.iter().any(|e| matches!(e, StateEvent::Trade(TradeEvent::Cancelled { .. }))));

    // Fair: the whole forty at their worth, taken as it stands
    let before = person.state().strands().expect("a balance");
    ask(&mut person);
    offer(&mut person, 40 * worth);
    person
        .until("the citizen's accept", Duration::from_secs(15), |s, _| s.open_trade().is_some_and(|o| o.they_accepted))
        .expect("accepted");
    person.trade_act(|s, now| s.trade_accept(now)).expect("accepting");
    person
        .until("the trade", Duration::from_secs(15), |s, _| matches!(s.trade(), Trade::Idle))
        .expect("the trade went through");
    person.settle().expect("refetched");
    person
        .until("the logs", Duration::from_secs(10), |s, _| s.strands() == Some(before - 40 * worth))
        .expect("paid forty logs' worth");
    assert_eq!(person.count(container_type::PLAYER_INVENTORY, "log"), 40);

    let (_, met) = answering.join().expect("the citizen's thread");
    assert!(matches!(&met[0], Met::Declined { .. }), "{met:?}");
    assert!(matches!(&met[1], Met::Traded { .. }), "{met:?}");
}

/// C1.5's second check, against a running town (`capitol-town`, a server of
/// its own with `DEV_GRANTS=on` for the person only): a person sells ten
/// logs at the market's price, and somebody buys them within a game hour.
/// The market's price is the citizens' books: the last trade, else the
/// middle of the book, else whatever side there is; on a bare book, 5.
/// Prints how many game minutes the fill took.
#[test]
#[ignore = "needs a running town: capitol-town against CAPITOL_SERVER, which has DEV_GRANTS=on"]
fn a_persons_sell_order_fills_in_a_town_within_a_game_hour() {
    use capitol_core::citizen::meet;
    use capitol_core::protocol::rest::PlaceOrder;
    use capitol_core::protocol::rules::market::{OrderSide, OrderStatus};

    let mut person = Bot::sign_in(live(), &Account::numbered(&format!("seller{}", stamp()), 1)).expect("signed in");
    let scale = person.health().expect("health").sim_tick_scale.unwrap_or(1.0);
    granted(&mut person, 0, &[("log", 10, 50)]);
    person.watch_market("log").expect("the log book");
    let price = meet::worth(person.state(), "log").unwrap_or(5);
    let placed = person
        .place_order(PlaceOrder {
            side: OrderSide::Sell,
            item_type: "log".into(),
            price,
            quantity: 10,
            item_id: Some(stack_of(&person, "log")),
            qualities: None,
            min_grade: None,
            grade_rule: None,
        })
        .expect("the sell order");
    let hour = Duration::from_secs_f64(3600.0 / scale);
    let started = Instant::now();
    let filled = person.until("the fill", hour, |s, _| s.market().order(placed.id).is_some_and(|o| o.status != OrderStatus::Open));
    let minutes = started.elapsed().as_secs_f64() * scale / 60.0;
    match filled {
        Ok(_) => println!("ten logs at {price} filled in {minutes:.0} game minutes"),
        Err(e) => {
            let _ = person.cancel_order(placed.id);
            panic!("ten logs at {price} unfilled after {minutes:.0} game minutes: {e}");
        }
    }
}
