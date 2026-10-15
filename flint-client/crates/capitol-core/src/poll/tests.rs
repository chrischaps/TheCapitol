//! Scheduler tests. Every trigger is a real `NetEvent` run through a real
//! `GameState`, so they cover the store's filtering as well as the rules.

use capitol_protocol::rest::*;
use capitol_protocol::ws::{PlayerPosition, ServerMessage as M};

use super::*;
use crate::net::RestError;

const ME: Uuid = Uuid::from_u128(1);
const OTHER: Uuid = Uuid::from_u128(2);
const NODE: Uuid = Uuid::from_u128(10);
const STATION: Uuid = Uuid::from_u128(20);
const INVENTORY: Uuid = Uuid::from_u128(30);
const STATION_BOX: Uuid = Uuid::from_u128(32);
const STRANGERS_BOX: Uuid = Uuid::from_u128(33);
const TRADE: Uuid = Uuid::from_u128(60);
const STRANGERS_TRADE: Uuid = Uuid::from_u128(61);
/// A tag of the game's own, below [`FIRST_TAG`].
const GAME_TAG: Tag = 7;

const STATION_FETCH: Fetch = Fetch::StationContainer {
    station_id: STATION,
};

/// A store and a poller wired the way the game wires them, on a fake clock.
struct Rig {
    state: GameState,
    poller: Poller,
    now: Instant,
}

impl Rig {
    fn new() -> Self {
        Self {
            state: GameState::new(),
            poller: Poller::new(),
            now: Instant::now(),
        }
    }

    /// Connected as `ME` at the Capitol, the snapshot answered, nothing pending.
    fn settled() -> Self {
        let mut rig = Self::new();
        rig.feed(NetEvent::Connection(ConnectionState::Connected {
            player_id: ME,
        }));
        rig.feed(NetEvent::Ws(M::Positions {
            players: vec![pos(ME, 4000.0, 4000.0), pos(OTHER, 4300.0, 4000.0)],
        }));
        rig.answer_all(); // the snapshot
        assert!(rig.poller.is_idle());
        rig
    }

    /// Settled, in a trade with `OTHER`.
    fn trading() -> Self {
        let mut rig = Self::settled();
        rig.feed(NetEvent::Ws(M::TradeStarted {
            trade_id: TRADE,
            partner_id: OTHER,
            partner_name: "Brin".into(),
        }));
        rig
    }

    /// Settled, with the workbench open.
    fn at_station() -> Self {
        let mut rig = Self::settled();
        rig.feed(NetEvent::Ws(M::StationOpened {
            player_id: ME,
            station_id: STATION,
            station_type: "workbench".into(),
            name: "Workbench".into(),
            container_id: STATION_BOX,
        }));
        rig.answer_all();
        rig
    }

    fn feed(&mut self, event: NetEvent) -> Option<Fetch> {
        let changes = self.state.apply(&event);
        self.poller.observe(&event, &changes, &self.state)
    }

    /// One frame: what the poller sends now.
    fn tick(&mut self) -> Vec<(RestRequest, Tag)> {
        self.poller.tick(self.now)
    }

    fn tick_fetches(&mut self) -> Vec<Fetch> {
        let sent = self.tick();
        sent.iter()
            .map(|(request, tag)| {
                let fetch = self
                    .poller
                    .in_flight(*tag)
                    .expect("sent tags are in flight");
                assert_eq!(*request, fetch.request());
                fetch
            })
            .collect()
    }

    fn advance(&mut self, by: Duration) {
        self.now += by;
    }

    /// Answers `tag` with a realistic success.
    fn answer(&mut self, tag: Tag) -> Option<Fetch> {
        let fetch = self
            .poller
            .in_flight(tag)
            .expect("answering an unknown tag");
        self.feed(NetEvent::Rest(tag, Ok(answer_for(fetch))))
    }

    /// Answers everything in flight, ticking until nothing is left.
    fn answer_all(&mut self) {
        loop {
            self.tick();
            let mut tags: Vec<Tag> = self.poller.in_flight.keys().copied().collect();
            if tags.is_empty() {
                return;
            }
            tags.sort();
            for tag in tags {
                self.answer(tag);
            }
        }
    }
}

fn pos(id: Uuid, x: f64, y: f64) -> PlayerPosition {
    PlayerPosition {
        id,
        name: format!("P{id}"),
        x,
        y,
    }
}

fn container(id: Uuid, container_type: &str) -> ContainerResponse {
    ContainerResponse {
        id,
        container_type: container_type.into(),
        slot_count: 24,
        layout_columns: 6,
        slots: vec![],
    }
}

fn answer_for(fetch: Fetch) -> RestResponse {
    match fetch {
        Fetch::Me => RestResponse::Me(PlayerMeResponse {
            player_id: ME,
            name: "Me".into(),
            x: 4000.0,
            y: 7350.0,
            zone: "rural".into(),
            strand_balance: 100,
            online: true,
        }),
        Fetch::Containers => RestResponse::Containers(ContainersResponse {
            containers: vec![container(INVENTORY, "player_inventory")],
        }),
        Fetch::StationContainer { .. } => {
            RestResponse::StationContainer(StationContainerResponse {
                id: STATION_BOX,
                container_type: "station_storage".into(),
                slot_count: 12,
                layout_columns: 4,
                slots: vec![],
            })
        }
        Fetch::NearbyNodes => RestResponse::Nearby(NearbyResponse { nodes: vec![] }),
        Fetch::NearbyStations => {
            RestResponse::NearbyStations(NearbyStationsResponse { stations: vec![] })
        }
        Fetch::Currency => RestResponse::Currency(CurrencyResponse {
            strand_balance: 100,
        }),
        Fetch::Plots => RestResponse::Plots(ListPlotsResponse {
            plots: vec![],
            total: 0,
        }),
        Fetch::Recipes => RestResponse::Recipes(RecipesResponse { recipes: vec![] }),
        Fetch::Terrain => panic!("terrain is answered by hand in its test"),
    }
}

fn yielded() -> (String, String, i32, i32) {
    ("fiber".into(), "Plant Fiber".into(), 1, 60)
}

fn deposit() -> RestResponse {
    RestResponse::Deposit(DepositResponse {
        deposited_quantity: 10,
        fee: 1,
        strands_received: 9,
        new_balance: 109,
    })
}

fn withdraw() -> RestResponse {
    RestResponse::Withdraw(WithdrawResponse {
        strands_spent: 10,
        fee: 1,
        fiber_received: 9,
        new_balance: 90,
    })
}

fn move_item() -> RestResponse {
    RestResponse::MoveItem(MoveItemResponse {
        success: true,
        action: "moved".into(),
        merged_quantity: None,
        merged_quality: None,
    })
}

// ---------------------------------------------------------------------------
// The rules, one row per trigger
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Start {
    /// `GameState::new()`, never connected.
    Fresh,
    Settled,
    AtStation,
    /// Settled, in a trade with `OTHER`.
    Trading,
}

/// Each row: where it starts, the events of one frame, and exactly what the
/// next tick must send, in order.
fn rules() -> Vec<(&'static str, Start, Vec<NetEvent>, Vec<Fetch>)> {
    use Start::*;
    let (item_type, item_name, quantity, quality) = yielded();
    vec![
        (
            "first Connected fetches the snapshot",
            Fresh,
            vec![NetEvent::Connection(ConnectionState::Connected {
                player_id: ME,
            })],
            Fetch::SNAPSHOT.to_vec(),
        ),
        (
            "a resync fetches the snapshot",
            Settled,
            vec![
                NetEvent::Connection(ConnectionState::Connected { player_id: ME }),
                NetEvent::Resync,
            ],
            Fetch::SNAPSHOT.to_vec(),
        ),
        (
            "my inventory_updated fetches containers",
            Settled,
            vec![NetEvent::Ws(M::InventoryUpdated {
                player_id: ME,
                container_id: INVENTORY,
            })],
            vec![Fetch::Containers],
        ),
        (
            "a stranger's inventory_updated fetches nothing",
            Settled,
            vec![NetEvent::Ws(M::InventoryUpdated {
                player_id: OTHER,
                container_id: STRANGERS_BOX,
            })],
            vec![],
        ),
        (
            "anyone changing the open station's box fetches that box",
            AtStation,
            vec![NetEvent::Ws(M::InventoryUpdated {
                player_id: OTHER,
                container_id: STATION_BOX,
            })],
            vec![STATION_FETCH],
        ),
        (
            "item_moved between inventory and station fetches both",
            AtStation,
            vec![NetEvent::Ws(M::ItemMoved {
                player_id: ME,
                item_id: Uuid::from_u128(70),
                from_container: INVENTORY,
                from_slot: 0,
                to_container: STATION_BOX,
                to_slot: 0,
            })],
            vec![Fetch::Containers, STATION_FETCH],
        ),
        (
            "my station_opened fetches its box",
            Settled,
            vec![NetEvent::Ws(M::StationOpened {
                player_id: ME,
                station_id: STATION,
                station_type: "workbench".into(),
                name: "Workbench".into(),
                container_id: STATION_BOX,
            })],
            vec![STATION_FETCH],
        ),
        (
            "a stranger's station_opened fetches nothing",
            Settled,
            vec![NetEvent::Ws(M::StationOpened {
                player_id: OTHER,
                station_id: STATION,
                station_type: "workbench".into(),
                name: "Workbench".into(),
                container_id: STATION_BOX,
            })],
            vec![],
        ),
        (
            "placing my own station fetches containers (the kit is gone)",
            Settled,
            vec![NetEvent::Ws(M::StationPlaced {
                station_id: STATION,
                station_type: "workbench".into(),
                name: "Workbench".into(),
                owner_id: ME,
                x: 4020.0,
                y: 4020.0,
                container_id: STATION_BOX,
            })],
            vec![Fetch::Containers],
        ),
        (
            "a stranger placing a station fetches nothing",
            Settled,
            vec![NetEvent::Ws(M::StationPlaced {
                station_id: STATION,
                station_type: "workbench".into(),
                name: "Workbench".into(),
                owner_id: OTHER,
                x: 4020.0,
                y: 4020.0,
                container_id: STATION_BOX,
            })],
            vec![],
        ),
        (
            "my extraction completing fetches containers",
            Settled,
            vec![NetEvent::Ws(M::ExtractionCompleted {
                player_id: ME,
                node_id: NODE,
                item_type: item_type.clone(),
                item_name: item_name.clone(),
                quantity,
                quality,
            })],
            vec![Fetch::Containers],
        ),
        (
            "a stranger's extraction completing fetches nothing",
            Settled,
            vec![NetEvent::Ws(M::ExtractionCompleted {
                player_id: OTHER,
                node_id: NODE,
                item_type: item_type.clone(),
                item_name: item_name.clone(),
                quantity,
                quality,
            })],
            vec![],
        ),
        (
            "my crafting completing fetches containers",
            Settled,
            vec![NetEvent::Ws(M::CraftingCompleted {
                player_id: ME,
                operation_id: Uuid::from_u128(40),
                item_type: "rope".into(),
                item_name: "Basic Rope".into(),
                quantity: 1,
                quality: 70,
            })],
            vec![Fetch::Containers],
        ),
        (
            "completion and inventory_updated in one frame fetch containers once",
            Settled,
            vec![
                NetEvent::Ws(M::ExtractionCompleted {
                    player_id: ME,
                    node_id: NODE,
                    item_type,
                    item_name,
                    quantity,
                    quality,
                }),
                NetEvent::Ws(M::InventoryUpdated {
                    player_id: ME,
                    container_id: INVENTORY,
                }),
            ],
            vec![Fetch::Containers],
        ),
        (
            "our trade_executed fetches containers and currency",
            Trading,
            vec![NetEvent::Ws(M::TradeExecuted { trade_id: TRADE })],
            vec![Fetch::Containers, Fetch::Currency],
        ),
        (
            "a stranger's trade_executed fetches nothing",
            Trading,
            vec![NetEvent::Ws(M::TradeExecuted {
                trade_id: STRANGERS_TRADE,
            })],
            vec![],
        ),
        (
            "trade_cancelled fetches nothing (no escrow)",
            Trading,
            vec![NetEvent::Ws(M::TradeCancelled {
                trade_id: TRADE,
                reason: "Cancelled".into(),
            })],
            vec![],
        ),
        (
            "a deposit fetches containers and currency",
            Settled,
            vec![NetEvent::Rest(GAME_TAG, Ok(deposit()))],
            vec![Fetch::Containers, Fetch::Currency],
        ),
        (
            "a withdrawal fetches containers and currency",
            Settled,
            vec![NetEvent::Rest(GAME_TAG, Ok(withdraw()))],
            vec![Fetch::Containers, Fetch::Currency],
        ),
        (
            "a failed deposit fetches nothing",
            Settled,
            vec![NetEvent::Rest(
                GAME_TAG,
                Err(RestError::Http {
                    status: 400,
                    message: "No fiber".into(),
                }),
            )],
            vec![],
        ),
        (
            "a REST move_item fetches containers",
            Settled,
            vec![NetEvent::Rest(GAME_TAG, Ok(move_item()))],
            vec![Fetch::Containers],
        ),
        (
            "a REST move_item at a station fetches its box too",
            AtStation,
            vec![NetEvent::Rest(GAME_TAG, Ok(move_item()))],
            vec![Fetch::Containers, STATION_FETCH],
        ),
        (
            "my zone_changed fetches plots",
            Settled,
            vec![NetEvent::Ws(M::ZoneChanged {
                player_id: ME,
                from_zone: "capitol".into(),
                to_zone: "trade_district".into(),
                zone_name: "Trade District".into(),
            })],
            vec![Fetch::Plots],
        ),
        (
            "a stranger's zone_changed fetches nothing",
            Settled,
            vec![NetEvent::Ws(M::ZoneChanged {
                player_id: OTHER,
                from_zone: "capitol".into(),
                to_zone: "trade_district".into(),
                zone_name: "Trade District".into(),
            })],
            vec![],
        ),
        (
            "terrain_updated fetches terrain",
            Settled,
            vec![NetEvent::Ws(M::TerrainUpdated)],
            vec![Fetch::Terrain],
        ),
        (
            "positions fetch nothing",
            Settled,
            vec![NetEvent::Ws(M::Positions {
                players: vec![pos(ME, 4010.0, 4000.0)],
            })],
            vec![],
        ),
        (
            "a failed station ack fetches nothing",
            Settled,
            vec![NetEvent::Ws(M::Error {
                message: "Too far".into(),
            })],
            vec![],
        ),
    ]
}

#[test]
fn each_trigger_sends_exactly_its_fetches() {
    for (name, start, events, expected) in rules() {
        let mut rig = match start {
            Start::Fresh => Rig::new(),
            Start::Settled => Rig::settled(),
            Start::AtStation => Rig::at_station(),
            Start::Trading => Rig::trading(),
        };
        for event in events {
            rig.feed(event);
        }
        assert_eq!(rig.tick_fetches(), expected, "{name}");
        // Exactly: nothing else turns up on the following frame.
        assert_eq!(rig.tick_fetches(), vec![], "{name}: a later frame");
    }
}

#[test]
fn the_first_zone_is_not_a_crossing() {
    // The first zone comes with the snapshot, which already fetches plots.
    let mut rig = Rig::new();
    rig.feed(NetEvent::Connection(ConnectionState::Connected {
        player_id: ME,
    }));
    let changes = rig.state.apply(&NetEvent::Ws(M::Positions {
        players: vec![pos(ME, 4000.0, 4000.0)],
    }));
    assert!(changes
        .iter()
        .any(|c| matches!(c, StateEvent::ZoneChanged { from: None, .. })));
    assert_eq!(rig.tick_fetches(), Fetch::SNAPSHOT.to_vec());
}

#[test]
fn plots_are_listed_like_react() {
    assert_eq!(
        Fetch::Plots.request(),
        RestRequest::ListPlots(ListPlotsQuery {
            limit: Some(100),
            ..Default::default()
        })
    );
}

// ---------------------------------------------------------------------------
// No duplicates
// ---------------------------------------------------------------------------

fn my_inventory_updated() -> NetEvent {
    NetEvent::Ws(M::InventoryUpdated {
        player_id: ME,
        container_id: INVENTORY,
    })
}

#[test]
fn nothing_is_sent_twice_while_pending() {
    let mut rig = Rig::settled();
    rig.feed(my_inventory_updated());
    let sent = rig.tick();
    assert_eq!(sent.len(), 1);
    let (_, tag) = sent[0];

    // More triggers while it's in flight send nothing now...
    for _ in 0..3 {
        rig.feed(my_inventory_updated());
        assert_eq!(rig.tick(), vec![]);
    }
    assert!(rig.poller.is_pending(Fetch::Containers));

    // ...and exactly one follow-up once it's answered, since that answer may
    // predate the change.
    assert_eq!(rig.answer(tag), Some(Fetch::Containers));
    let again = rig.tick();
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].1, tag, "a fresh tag");
    rig.answer(again[0].1);
    assert_eq!(rig.tick(), vec![]);
    assert!(rig.poller.is_idle());
}

#[test]
fn an_answer_with_nothing_new_sends_nothing() {
    let mut rig = Rig::settled();
    rig.feed(my_inventory_updated());
    let (_, tag) = rig.tick()[0];
    rig.answer(tag);
    assert_eq!(rig.tick(), vec![]);
}

#[test]
fn a_failed_answer_still_clears_the_fetch() {
    let mut rig = Rig::settled();
    rig.feed(my_inventory_updated());
    let (_, tag) = rig.tick()[0];
    let failed = NetEvent::Rest(tag, Err(RestError::Transport("reset".into())));
    assert_eq!(rig.feed(failed), Some(Fetch::Containers));
    assert!(rig.poller.is_idle());

    // The next trigger goes out again.
    rig.feed(my_inventory_updated());
    assert_eq!(rig.tick_fetches(), vec![Fetch::Containers]);
}

#[test]
fn different_fetches_go_out_together() {
    let mut rig = Rig::trading();
    rig.feed(my_inventory_updated());
    rig.tick();
    rig.feed(NetEvent::Ws(M::TradeExecuted { trade_id: TRADE }));
    // Containers is in flight, so only currency goes now.
    assert_eq!(rig.tick_fetches(), vec![Fetch::Currency]);
}

#[test]
fn the_games_tags_pass_through() {
    let mut rig = Rig::settled();
    let reply = NetEvent::Rest(
        GAME_TAG,
        Ok(RestResponse::Currency(CurrencyResponse {
            strand_balance: 5,
        })),
    );
    assert_eq!(rig.feed(reply), None);
    assert_eq!(rig.tick(), vec![]);
}

#[test]
fn tags_start_above_the_games() {
    let mut rig = Rig::new();
    rig.feed(NetEvent::Connection(ConnectionState::Connected {
        player_id: ME,
    }));
    let tags: Vec<Tag> = rig.tick().into_iter().map(|(_, tag)| tag).collect();
    assert_eq!(tags, (FIRST_TAG..FIRST_TAG + 6).collect::<Vec<_>>());
}

#[test]
fn answers_name_their_fetch() {
    let mut rig = Rig::settled();
    rig.feed(NetEvent::Ws(M::TerrainUpdated));
    let (request, tag) = rig.tick().remove(0);
    assert_eq!(
        request,
        RestRequest::Terrain {
            cached_version: None
        }
    );
    // The game sees which fetch an event answered, to route `/terrain` to the world.
    let terrain = NetEvent::Rest(tag, Err(RestError::Transport("test".into())));
    assert_eq!(rig.feed(terrain), Some(Fetch::Terrain));
}

// ---------------------------------------------------------------------------
// Stations
// ---------------------------------------------------------------------------

#[test]
fn closing_a_station_drops_its_queued_fetch() {
    let mut rig = Rig::at_station();
    rig.feed(NetEvent::Ws(M::InventoryUpdated {
        player_id: OTHER,
        container_id: STATION_BOX,
    }));
    rig.feed(NetEvent::Ws(M::StationClosed { player_id: ME }));
    assert_eq!(rig.tick(), vec![]);
}

#[test]
fn closing_a_station_drops_its_follow_up() {
    let mut rig = Rig::at_station();
    let changed = || {
        NetEvent::Ws(M::InventoryUpdated {
            player_id: OTHER,
            container_id: STATION_BOX,
        })
    };
    rig.feed(changed());
    let (_, tag) = rig.tick()[0];
    rig.feed(changed()); // marked to go again
    rig.feed(NetEvent::Ws(M::StationClosed { player_id: ME }));
    rig.answer(tag);
    assert_eq!(rig.tick(), vec![]);
    assert!(rig.poller.is_idle());
}

// ---------------------------------------------------------------------------
// The timer
// ---------------------------------------------------------------------------

#[test]
fn polls_every_interval_while_connected() {
    let mut rig = Rig::settled();
    // The snapshot restarted the timer at the frame it was sent.
    rig.advance(POLL_INTERVAL - Duration::from_millis(1));
    assert_eq!(rig.tick(), vec![]);
    rig.advance(Duration::from_millis(1));
    assert_eq!(rig.tick_fetches(), Fetch::POLLED.to_vec());
    rig.answer_all();

    rig.advance(POLL_INTERVAL / 2);
    assert_eq!(rig.tick(), vec![]);
    rig.advance(POLL_INTERVAL / 2);
    assert_eq!(rig.tick_fetches(), Fetch::POLLED.to_vec());
}

#[test]
fn a_slow_poll_is_not_doubled() {
    let mut rig = Rig::settled();
    rig.advance(POLL_INTERVAL);
    let first = rig.tick();
    assert_eq!(first.len(), Fetch::POLLED.len());

    // The next interval passes with all of them still out: nothing new is sent,
    // and each goes once more when its answer arrives.
    rig.advance(POLL_INTERVAL);
    assert_eq!(rig.tick(), vec![]);
    for (_, tag) in first {
        rig.answer(tag);
    }
    assert_eq!(rig.tick_fetches(), Fetch::POLLED.to_vec());
}

#[test]
fn a_stall_does_not_burst() {
    let mut rig = Rig::settled();
    rig.advance(POLL_INTERVAL * 10);
    assert_eq!(rig.tick_fetches(), Fetch::POLLED.to_vec());
    rig.answer_all();
    // One poll for the whole stall; the next is a full interval later.
    rig.advance(POLL_INTERVAL - Duration::from_millis(1));
    assert_eq!(rig.tick(), vec![]);
    rig.advance(Duration::from_millis(1));
    assert_eq!(rig.tick().len(), Fetch::POLLED.len());
}

#[test]
fn no_polling_before_connecting() {
    let mut rig = Rig::new();
    for _ in 0..3 {
        rig.advance(POLL_INTERVAL);
        assert_eq!(rig.tick(), vec![]);
    }
}

#[test]
fn losing_the_connection_stops_polling_and_clears_the_queue() {
    let mut rig = Rig::settled();
    rig.feed(my_inventory_updated());
    rig.feed(NetEvent::Connection(ConnectionState::Reconnecting {
        attempt: 1,
        delay: Duration::from_millis(500),
        reason: "reset".into(),
    }));
    assert_eq!(rig.tick(), vec![], "the resync will refetch it");
    for _ in 0..3 {
        rig.advance(POLL_INTERVAL);
        assert_eq!(rig.tick(), vec![]);
    }

    // Back up: the resync fetches the snapshot, and the timer restarts from there.
    rig.feed(NetEvent::Connection(ConnectionState::Connected {
        player_id: ME,
    }));
    rig.feed(NetEvent::Resync);
    assert_eq!(rig.tick_fetches(), Fetch::SNAPSHOT.to_vec());
    rig.answer_all();
    rig.advance(POLL_INTERVAL);
    assert_eq!(rig.tick_fetches(), Fetch::POLLED.to_vec());
}

#[test]
fn a_request_before_connecting_survives_the_handshake() {
    let mut rig = Rig::new();
    rig.poller.request(Fetch::Terrain);
    rig.feed(NetEvent::Connection(ConnectionState::Connecting));
    rig.feed(NetEvent::Connection(ConnectionState::Authenticating));
    assert_eq!(rig.tick_fetches(), vec![Fetch::Terrain]);
}

#[test]
fn logging_out_clears_the_queue() {
    let mut rig = Rig::settled();
    rig.feed(my_inventory_updated());
    rig.feed(NetEvent::Connection(ConnectionState::Disconnected {
        reason: "logout".into(),
    }));
    assert_eq!(rig.tick(), vec![]);
}

#[test]
fn an_answer_from_before_a_drop_still_resolves() {
    let mut rig = Rig::settled();
    rig.feed(my_inventory_updated());
    let (_, tag) = rig.tick()[0];
    rig.feed(NetEvent::Connection(ConnectionState::Reconnecting {
        attempt: 1,
        delay: Duration::from_millis(500),
        reason: "reset".into(),
    }));
    rig.feed(NetEvent::Connection(ConnectionState::Connected {
        player_id: ME,
    }));
    rig.feed(NetEvent::Resync);
    // Containers was still out, so the snapshot sends the other five now...
    let mut expected = Fetch::SNAPSHOT.to_vec();
    expected.retain(|f| *f != Fetch::Containers);
    assert_eq!(rig.tick_fetches(), expected);
    // ...and containers again once the old answer is in.
    rig.answer(tag);
    assert_eq!(rig.tick_fetches(), vec![Fetch::Containers]);
}

#[test]
fn manual_requests_go_out_on_the_next_tick() {
    let mut rig = Rig::settled();
    rig.poller.request(Fetch::Recipes);
    rig.poller.request(Fetch::Recipes);
    assert_eq!(rig.tick_fetches(), vec![Fetch::Recipes]);
}
