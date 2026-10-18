//! Reducer tests: one per `ServerMessage` variant (see `every_variant_has_a_test`),
//! then REST responses, connection changes and reconnects.

use std::time::Duration;

use capitol_protocol::rest::*;
use capitol_protocol::ws::{PlayerPosition, ServerMessage as M};

use super::trade::tests as trade;
use super::*;
use crate::net::{RestError, RestResponse as R};

const ME: Uuid = Uuid::from_u128(1);
const OTHER: Uuid = Uuid::from_u128(2);
const NODE: Uuid = Uuid::from_u128(10);
const STATION: Uuid = Uuid::from_u128(20);
const CHEST: Uuid = Uuid::from_u128(21);
const INVENTORY: Uuid = Uuid::from_u128(30);
const CRAFT_IN: Uuid = Uuid::from_u128(31);
const STATION_BOX: Uuid = Uuid::from_u128(32);
const OP: Uuid = Uuid::from_u128(40);
const PLOT: Uuid = Uuid::from_u128(50);
const ITEM: Uuid = Uuid::from_u128(70);

fn pos(id: Uuid, name: &str, x: f64, y: f64) -> PlayerPosition {
    PlayerPosition {
        id,
        name: name.into(),
        x,
        y,
    }
}

/// Connected as `ME`, with the first snapshot's events already drained.
fn connected() -> GameState {
    let mut state = GameState::new();
    state.apply_connection(&ConnectionState::Connected { player_id: ME });
    state
}

/// Connected, with both players in the world: me at the Capitol, the other in
/// the trade district.
fn with_players() -> GameState {
    let mut state = connected();
    state.apply_ws(&M::Positions {
        players: vec![
            pos(ME, "Ada", 4000.0, 4000.0),
            pos(OTHER, "Brin", 4300.0, 4000.0),
        ],
    });
    state
}

fn node(id: Uuid, state: &str) -> ResourceNodeInfo {
    ResourceNodeInfo {
        id,
        resource_type: "grass".into(),
        x: 4010.0,
        y: 4010.0,
        state: state.into(),
    }
}

fn station(id: Uuid, owner: Uuid) -> StationInfo {
    StationInfo {
        id,
        station_type: "workbench".into(),
        name: "Workbench".into(),
        category: "crafting".into(),
        icon: Some("workbench".into()),
        x: 4020.0,
        y: 4020.0,
        owner_id: owner,
        container_id: Some(STATION_BOX),
        interaction_range: 75.0,
    }
}

fn slot(item: u128, slot_index: i32, quantity: i32) -> SlotItem {
    SlotItem::new(
        Uuid::from_u128(item),
        "fiber".into(),
        "Plant Fiber".into(),
        60,
        quantity,
        slot_index,
    )
}

fn containers() -> R {
    let container = |id, container_type: &str, slot_count, slots| ContainerResponse {
        id,
        container_type: container_type.into(),
        slot_count,
        layout_columns: 6,
        slots,
    };
    R::Containers(ContainersResponse {
        containers: vec![
            container(INVENTORY, "player_inventory", 24, vec![slot(100, 0, 3)]),
            container(CRAFT_IN, "crafting_input", 4, vec![]),
        ],
    })
}

fn plot(owner: Option<Uuid>) -> PlotInfo {
    PlotInfo {
        id: PLOT,
        zone_id: "urban".into(),
        world_x: 5200.0,
        world_y: 4000.0,
        bounds: Bounds::new(5180.0, 3980.0, 5220.0, 4020.0),
        size_category: "small".into(),
        plot_type: "standard".into(),
        owner_id: owner,
        owner_name: owner.map(|_| "Ada".into()),
        claimed_at: None,
        assessed_value: 500,
        deed_cost: Some(500),
        station_count: 0,
        station_capacity: 2,
        terrain_id: None,
        corners: None,
    }
}

fn yield_of(item_type: &str, quality: i32) -> Yield {
    Yield {
        item_type: item_type.into(),
        item_name: item_type.into(),
        quantity: 1,
        quality,
    }
}

fn extraction_started(player_id: Uuid) -> M {
    M::ExtractionStarted {
        player_id,
        node_id: NODE,
        duration_ticks: 30,
    }
}

fn crafting_started(player_id: Uuid) -> M {
    M::CraftingStarted {
        player_id,
        operation_id: OP,
        recipe_id: "rope_basic".into(),
        recipe_name: "Basic Rope".into(),
        duration_ticks: 50,
    }
}

fn station_opened(player_id: Uuid) -> M {
    M::StationOpened {
        player_id,
        station_id: STATION,
        station_type: "workbench".into(),
        name: "Workbench".into(),
        container_id: STATION_BOX,
    }
}

// ---------------------------------------------------------------------------
// Coverage guard
// ---------------------------------------------------------------------------

/// The reducer test for each variant. The match is exhaustive, so a new
/// `ServerMessage` variant doesn't compile until it has a test here.
fn test_for(message: &M) -> fn() {
    match message {
        M::AuthOk { .. } => auth_ok_sets_the_local_player,
        M::AuthError { .. } => auth_error_and_subscribed_change_nothing,
        M::Subscribed { .. } => auth_error_and_subscribed_change_nothing,
        M::Positions { .. } => positions_replace_the_player_set,
        M::PlayerJoined { .. } => player_joined_adds_once,
        M::PlayerLeft { .. } => player_left_removes_the_player_and_their_activity,
        M::Error { .. } => error_is_reported,
        M::ExtractionStarted { .. } => extraction_started_is_local_only_for_me,
        M::ExtractionProgress { .. } => extraction_progress_updates_the_right_player,
        M::ExtractionCompleted { .. } => extraction_completed_reports_the_yield,
        M::ExtractionCancelled { .. } => extraction_cancelled_clears_only_that_player,
        M::ExtractionFailed { .. } => extraction_failed_reports_only_mine_and_ends_nothing,
        M::NodeDepleted { .. } => node_depleted_and_regenerated_flip_state,
        M::NodeRegenerated { .. } => node_depleted_and_regenerated_flip_state,
        M::NearbyNodes { .. } => nearby_nodes_merge,
        M::CraftingStarted { .. } => crafting_started_is_local_only_for_me,
        M::CraftingProgress { .. } => crafting_progress_updates_the_right_player,
        M::CraftingCompleted { .. } => crafting_completed_reports_the_yield,
        M::CraftingCancelled { .. } => crafting_cancelled_and_failed_clear_activity,
        M::CraftingFailed { .. } => crafting_cancelled_and_failed_clear_activity,
        M::InventoryUpdated { .. } => inventory_updated_is_filtered,
        M::ItemMoved { .. } => item_moved_reports_both_containers,
        M::ItemsMerged { .. } => items_merged_reports_the_inventory,
        M::StationPlaced { .. } => station_placed_fills_in_from_the_catalog,
        M::StationRemoved { .. } => station_removed_closes_it_if_open,
        M::StationOpened { .. } => station_opened_only_for_me,
        M::StationClosed { .. } => station_closed_only_for_me,
        M::NearbyStations { .. } => nearby_stations_merge,
        M::TradeRequested { .. } => trade::trade_requested_prompts,
        M::TradeRequestDeclined { .. } => trade::trade_request_declined_ends_our_request,
        M::TradeStarted { .. } => trade::trade_started_opens_from_any_state,
        M::TradeOfferUpdated { .. } => trade::offers_update_their_side_and_reset_accepts,
        M::TradeAccepted { .. } => trade::trade_accepted_marks_the_right_side,
        M::TradeExecuted { .. } => trade::trade_executed_counts_only_ours,
        M::TradeCancelled { .. } => trade::trade_cancelled_counts_only_ours,
        M::CurrencyChanged { .. } => currency_changed_sets_the_balance,
        M::ZoneChanged { .. } => zone_changed_updates_my_zone,
        M::MovementBlocked { .. } => movement_blocked_moves_me_back,
        M::TerrainUpdated => terrain_updated_is_reported,
        M::PlotClaimed { .. } => plot_claimed_is_reported,
        M::PlotReleased { .. } => plot_released_drops_it_from_my_plots,
    }
}

/// Runs every protocol fixture (the server's real bytes) through a populated
/// store, and the matching reducer test.
#[test]
fn every_variant_has_a_test() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../protocol/tests/fixtures/server");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("protocol fixtures") {
        let path = entry.unwrap().path();
        let json = std::fs::read_to_string(&path).unwrap();
        let message: M = serde_json::from_str(&json).unwrap();
        let mut state = with_players();
        state.apply_ws(&message);
        test_for(&message)();
        count += 1;
    }
    assert_eq!(count, 41, "one fixture per ServerMessage variant");
}

// ---------------------------------------------------------------------------
// Handshake and players
// ---------------------------------------------------------------------------

#[test]
fn auth_ok_sets_the_local_player() {
    let mut state = GameState::new();
    assert_eq!(state.me(), None);
    assert_eq!(state.time_of_day(), None, "no clock before AuthOk");
    // A server clock an hour ahead of this machine's.
    let hour = 3_600_000;
    let ahead = now_ms() + hour;
    let events = state.apply_ws(&M::AuthOk { player_id: ME, server_time_ms: ahead });
    assert!(events.is_empty());
    assert_eq!(state.me(), Some(ME));
    let server = state.server_time_ms().unwrap();
    assert!((server - ahead).abs() < 1000, "keeps the offset: {server} vs {ahead}");
    assert_eq!(state.time_of_day(), Some(capitol_protocol::rules::time_of_day(server)));

    // A reconnect rebases it.
    state.apply_ws(&M::AuthOk { player_id: ME, server_time_ms: now_ms() });
    assert!((state.server_time_ms().unwrap() - now_ms()).abs() < 1000);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[test]
fn auth_error_and_subscribed_change_nothing() {
    let mut state = with_players();
    let before = format!("{state:?}");
    for message in [
        M::AuthError {
            message: "Invalid token".into(),
        },
        M::Subscribed {
            channel: "position".into(),
        },
    ] {
        assert!(state.apply_ws(&message).is_empty());
    }
    assert_eq!(format!("{state:?}"), before);
}

#[test]
fn positions_replace_the_player_set() {
    let mut state = connected();
    let events = state.apply_ws(&M::Positions {
        players: vec![
            pos(ME, "Ada", 500.0, 500.0),
            pos(OTHER, "Brin", 600.0, 500.0),
        ],
    });
    assert!(events.contains(&StateEvent::PlayerJoined(ME)));
    assert!(events.contains(&StateEvent::PlayerJoined(OTHER)));
    assert!(events.contains(&StateEvent::PositionsUpdated));
    // My name and position come from the first snapshot that includes me.
    assert_eq!(state.local_player().unwrap().name, "Ada");

    // Later snapshots move players without re-joining them...
    let events = state.apply_ws(&M::Positions {
        players: vec![
            pos(ME, "Ada", 505.0, 500.0),
            pos(OTHER, "Brin", 600.0, 505.0),
        ],
    });
    assert_eq!(events, vec![StateEvent::PositionsUpdated]);
    assert_eq!(state.player(ME).unwrap().x, 505.0);

    // ...and anyone missing has gone, even without a player_left.
    let events = state.apply_ws(&M::Positions {
        players: vec![pos(ME, "Ada", 510.0, 500.0)],
    });
    assert_eq!(
        events,
        vec![StateEvent::PlayerLeft(OTHER), StateEvent::PositionsUpdated]
    );
    assert!(state.player(OTHER).is_none());
}

#[test]
fn first_position_sets_the_starting_zone() {
    let mut state = connected();
    let events = state.apply_ws(&M::Positions {
        players: vec![pos(ME, "Ada", 500.0, 500.0)],
    });
    assert!(events.contains(&StateEvent::ZoneChanged {
        from: None,
        to: Zone::Wilderness
    }));
    assert_eq!(state.zone(), Some(Zone::Wilderness));
    // Moving doesn't recompute it; the server says when the zone changes.
    let events = state.apply_ws(&M::Positions {
        players: vec![pos(ME, "Ada", 4000.0, 4000.0)],
    });
    assert_eq!(events, vec![StateEvent::PositionsUpdated]);
    assert_eq!(state.zone(), Some(Zone::Wilderness));
}

#[test]
fn player_joined_adds_once() {
    let mut state = connected();
    let joined = M::PlayerJoined {
        player: pos(OTHER, "Brin", 1.0, 2.0),
    };
    assert_eq!(
        state.apply_ws(&joined),
        vec![StateEvent::PlayerJoined(OTHER)]
    );
    assert!(state.apply_ws(&joined).is_empty());
    assert_eq!(state.player(OTHER).unwrap().name, "Brin");
}

#[test]
fn player_left_removes_the_player_and_their_activity() {
    let mut state = with_players();
    state.apply_ws(&extraction_started(OTHER));
    let events = state.apply_ws(&M::PlayerLeft { player_id: OTHER });
    assert_eq!(
        events,
        vec![
            StateEvent::PlayerLeft(OTHER),
            StateEvent::ActivityChanged { player_id: OTHER }
        ]
    );
    assert!(state.player(OTHER).is_none());
    assert!(state.activity(OTHER).is_none());
    assert!(state
        .apply_ws(&M::PlayerLeft { player_id: OTHER })
        .is_empty());
}

#[test]
fn error_is_reported() {
    let mut state = connected();
    let events = state.apply_ws(&M::Error {
        message: "Too far from node".into(),
        command: "extract".into(),
    });
    assert_eq!(
        events,
        vec![StateEvent::ServerError {
            message: "Too far from node".into(),
            command: "extract".into(),
        }]
    );
}

#[test]
fn movement_blocked_moves_me_back() {
    let mut state = with_players();
    let blocked = |player_id| M::MovementBlocked {
        player_id,
        reason: "Cannot cross the moat without a bridge".into(),
        water_id: "moat".into(),
        stopped_x: 3990.0,
        stopped_y: 4001.0,
    };
    // Sent only to the blocked player, but filter anyway.
    assert!(state.apply_ws(&blocked(OTHER)).is_empty());
    assert_eq!(state.player(OTHER).unwrap().x, 4300.0);

    let events = state.apply_ws(&blocked(ME));
    assert_eq!(
        events,
        vec![StateEvent::MovementBlocked {
            reason: "Cannot cross the moat without a bridge".into(),
            water: "moat".into(),
            x: 3990.0,
            y: 4001.0
        }]
    );
    let me = state.local_player().unwrap();
    assert_eq!((me.x, me.y), (3990.0, 4001.0));
}

#[test]
fn zone_changed_updates_my_zone() {
    let mut state = with_players();
    assert_eq!(state.zone(), Some(Zone::Capitol));
    let changed = |player_id, to: &str| M::ZoneChanged {
        player_id,
        from_zone: "capitol".into(),
        to_zone: to.into(),
        zone_name: "Trade District".into(),
    };
    assert!(state.apply_ws(&changed(OTHER, "rural")).is_empty());
    assert!(state.apply_ws(&changed(ME, "atlantis")).is_empty());
    assert_eq!(state.zone(), Some(Zone::Capitol));

    let events = state.apply_ws(&changed(ME, "trade_district"));
    assert_eq!(
        events,
        vec![StateEvent::ZoneChanged {
            from: Some(Zone::Capitol),
            to: Zone::TradeDistrict
        }]
    );
    assert_eq!(state.zone(), Some(Zone::TradeDistrict));
}

// ---------------------------------------------------------------------------
// Extraction and crafting
// ---------------------------------------------------------------------------

#[test]
fn extraction_started_is_local_only_for_me() {
    let mut state = with_players();
    let events = state.apply_ws(&extraction_started(OTHER));
    assert_eq!(
        events,
        vec![StateEvent::ActivityChanged { player_id: OTHER }]
    );
    assert!(!state.is_extracting());
    assert!(state.local_activity().is_none());
    assert!(matches!(
        state.activity(OTHER),
        Some(Activity::Extracting { node_id: NODE, .. })
    ));

    let events = state.apply_ws(&extraction_started(ME));
    assert_eq!(
        events,
        vec![StateEvent::ExtractionStarted { node_id: NODE }]
    );
    assert_eq!(state.extraction(), Some((NODE, 0, 30)));
}

#[test]
fn extraction_progress_updates_the_right_player() {
    let mut state = with_players();
    state.apply_ws(&extraction_started(ME));
    state.apply_ws(&extraction_started(OTHER));
    let progress = |player_id, progress| M::ExtractionProgress {
        player_id,
        progress,
        duration: 30,
    };
    assert!(state.apply_ws(&progress(OTHER, 20)).is_empty());
    assert_eq!(state.extraction(), Some((NODE, 0, 30)));
    state.apply_ws(&progress(ME, 15));
    assert_eq!(state.extraction(), Some((NODE, 15, 30)));
    assert_eq!(state.local_activity().unwrap().fraction(), 0.5);
    assert_eq!(state.activity(OTHER).unwrap().fraction(), 20.0 / 30.0);

    // Progress with no extraction on record is ignored.
    let mut fresh = with_players();
    fresh.apply_ws(&progress(ME, 5));
    assert!(fresh.local_activity().is_none());
}

#[test]
fn extraction_completed_reports_the_yield() {
    let mut state = with_players();
    state.apply_ws(&extraction_started(ME));
    state.apply_ws(&extraction_started(OTHER));
    let completed = |player_id| M::ExtractionCompleted {
        player_id,
        node_id: NODE,
        item_type: "fiber".into(),
        item_name: "fiber".into(),
        quantity: 1,
        quality: 62,
    };

    let events = state.apply_ws(&completed(OTHER));
    assert_eq!(
        events,
        vec![StateEvent::ActivityChanged { player_id: OTHER }]
    );
    assert!(
        state.is_extracting(),
        "another player's completion isn't mine"
    );

    let events = state.apply_ws(&completed(ME));
    assert_eq!(
        events,
        vec![StateEvent::ExtractionCompleted {
            node_id: NODE,
            item: yield_of("fiber", 62)
        }]
    );
    assert!(!state.is_extracting());
}

#[test]
fn extraction_cancelled_clears_only_that_player() {
    let mut state = with_players();
    state.apply_ws(&extraction_started(ME));
    state.apply_ws(&extraction_started(OTHER));
    let events = state.apply_ws(&M::ExtractionCancelled { player_id: OTHER });
    assert_eq!(
        events,
        vec![StateEvent::ActivityChanged { player_id: OTHER }]
    );
    assert!(state.is_extracting());
    let events = state.apply_ws(&M::ExtractionCancelled { player_id: ME });
    assert_eq!(events, vec![StateEvent::ExtractionCancelled]);
    assert!(!state.is_extracting());
    // A cancellation for a player with nothing on record says nothing.
    assert!(state
        .apply_ws(&M::ExtractionCancelled { player_id: OTHER })
        .is_empty());
}

#[test]
fn extraction_failed_reports_only_mine_and_ends_nothing() {
    let mut state = with_players();
    state.apply_ws(&extraction_started(ME));
    state.apply_ws(&extraction_started(OTHER));
    // Another player's refusal is theirs alone (the server doesn't send it
    // here, but if it did it changes nothing).
    let events = state.apply_ws(&M::ExtractionFailed {
        player_id: OTHER,
        reason: "Too far from resource".into(),
    });
    assert!(events.is_empty(), "{events:?}");
    assert!(state.activity(OTHER).is_some());

    // "Already performing an action": the extraction under way carries on.
    let events = state.apply_ws(&M::ExtractionFailed {
        player_id: ME,
        reason: "Already performing an action".into(),
    });
    assert_eq!(
        events,
        vec![StateEvent::ExtractionFailed {
            reason: "Already performing an action".into()
        }]
    );
    assert!(state.is_extracting());
}

#[test]
fn crafting_started_is_local_only_for_me() {
    let mut state = with_players();
    let events = state.apply_ws(&crafting_started(OTHER));
    assert_eq!(
        events,
        vec![StateEvent::ActivityChanged { player_id: OTHER }]
    );
    assert!(!state.is_crafting());

    let events = state.apply_ws(&crafting_started(ME));
    assert_eq!(
        events,
        vec![StateEvent::CraftingStarted {
            recipe_id: "rope_basic".into(),
            recipe_name: "Basic Rope".into()
        }]
    );
    assert!(state.is_crafting());
    assert!(!state.is_extracting());
}

#[test]
fn crafting_progress_updates_the_right_player() {
    let mut state = with_players();
    state.apply_ws(&crafting_started(ME));
    state.apply_ws(&crafting_started(OTHER));
    let progress = |player_id, progress| M::CraftingProgress {
        player_id,
        operation_id: OP,
        progress,
        duration: 50,
    };
    state.apply_ws(&progress(OTHER, 40));
    assert_eq!(state.local_activity().unwrap().fraction(), 0.0);
    state.apply_ws(&progress(ME, 25));
    assert_eq!(state.local_activity().unwrap().fraction(), 0.5);
    // An extraction progress doesn't touch a craft.
    state.apply_ws(&M::ExtractionProgress {
        player_id: ME,
        progress: 49,
        duration: 50,
    });
    assert_eq!(state.local_activity().unwrap().fraction(), 0.5);
}

#[test]
fn crafting_completed_reports_the_yield() {
    let mut state = with_players();
    state.apply_ws(&crafting_started(ME));
    state.apply_ws(&crafting_started(OTHER));
    let completed = |player_id| M::CraftingCompleted {
        player_id,
        operation_id: OP,
        item_type: "rope".into(),
        item_name: "rope".into(),
        quantity: 1,
        quality: 91,
    };
    state.apply_ws(&completed(OTHER));
    assert!(state.is_crafting());
    let events = state.apply_ws(&completed(ME));
    assert_eq!(
        events,
        vec![StateEvent::CraftingCompleted {
            item: yield_of("rope", 91)
        }]
    );
    assert!(!state.is_crafting());
}

#[test]
fn crafting_cancelled_and_failed_clear_activity() {
    let mut state = with_players();
    state.apply_ws(&crafting_started(ME));
    state.apply_ws(&crafting_started(OTHER));
    state.apply_ws(&M::CraftingFailed {
        player_id: OTHER,
        reason: "Missing inputs".into(),
    });
    assert!(state.is_crafting());
    assert!(state.activity(OTHER).is_none());

    let events = state.apply_ws(&M::CraftingCancelled {
        player_id: ME,
        operation_id: OP,
    });
    assert_eq!(events, vec![StateEvent::CraftingCancelled]);
    assert!(!state.is_crafting());

    state.apply_ws(&crafting_started(ME));
    let events = state.apply_ws(&M::CraftingFailed {
        player_id: ME,
        reason: "Missing inputs".into(),
    });
    assert_eq!(
        events,
        vec![StateEvent::CraftingFailed {
            reason: "Missing inputs".into()
        }]
    );
    assert!(!state.is_crafting());
}

/// The acceptance criterion in one place: a full burst of another player's
/// extraction and crafting leaves the local player's state alone.
#[test]
fn other_players_activity_never_touches_mine() {
    let mut state = with_players();
    state.apply_ws(&extraction_started(ME));
    let mine = state.local_activity().cloned();
    for message in [
        extraction_started(OTHER),
        M::ExtractionProgress {
            player_id: OTHER,
            progress: 29,
            duration: 30,
        },
        M::ExtractionCompleted {
            player_id: OTHER,
            node_id: NODE,
            item_type: "fiber".into(),
            item_name: "Plant Fiber".into(),
            quantity: 2,
            quality: 70,
        },
        M::ExtractionCancelled { player_id: OTHER },
        crafting_started(OTHER),
        M::CraftingProgress {
            player_id: OTHER,
            operation_id: OP,
            progress: 10,
            duration: 50,
        },
        M::CraftingCompleted {
            player_id: OTHER,
            operation_id: OP,
            item_type: "rope".into(),
            item_name: "Rope".into(),
            quantity: 1,
            quality: 80,
        },
        M::CraftingCancelled {
            player_id: OTHER,
            operation_id: OP,
        },
        M::CraftingFailed {
            player_id: OTHER,
            reason: "nope".into(),
        },
        station_opened(OTHER),
        M::StationClosed { player_id: OTHER },
        M::InventoryUpdated {
            player_id: OTHER,
            container_id: Uuid::from_u128(99),
        },
    ] {
        for event in state.apply_ws(&message) {
            assert!(
                matches!(event, StateEvent::ActivityChanged { player_id: OTHER }),
                "{} from another player produced {event:?}",
                message.type_name()
            );
        }
    }
    assert_eq!(state.local_activity().cloned(), mine);
    assert!(state.open_station().is_none());
}

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

#[test]
fn node_depleted_and_regenerated_flip_state() {
    let mut state = connected();
    state.apply_rest(&R::Nearby(NearbyResponse {
        nodes: vec![node(NODE, "available")],
    }));
    let events = state.apply_ws(&M::NodeDepleted { node_id: NODE });
    assert_eq!(events, vec![StateEvent::NodeChanged(NODE)]);
    assert_eq!(state.node(NODE).unwrap().state, "depleted");
    assert!(state
        .apply_ws(&M::NodeDepleted { node_id: NODE })
        .is_empty());

    let events = state.apply_ws(&M::NodeRegenerated { node_id: NODE });
    assert_eq!(events, vec![StateEvent::NodeChanged(NODE)]);
    assert_eq!(state.node(NODE).unwrap().state, "available");

    // Nodes never seen are ignored, as in the React client.
    let unknown = Uuid::from_u128(11);
    assert!(state
        .apply_ws(&M::NodeDepleted { node_id: unknown })
        .is_empty());
    assert!(state.node(unknown).is_none());
}

#[test]
fn a_node_someone_is_extracting_is_not_available() {
    let mut state = with_players();
    state.apply_rest(&R::Nearby(NearbyResponse {
        nodes: vec![node(NODE, "available")],
    }));
    assert!(state.node_available(NODE));
    state.apply_ws(&M::ExtractionStarted {
        player_id: OTHER,
        node_id: NODE,
        duration_ticks: 30,
    });
    assert_eq!(state.harvester(NODE), Some(OTHER));
    assert!(!state.node_available(NODE));
    state.apply_ws(&M::ExtractionCancelled { player_id: OTHER });
    assert_eq!(state.harvester(NODE), None);
    assert!(state.node_available(NODE));
    assert!(!state.node_available(Uuid::from_u128(11)));
}

#[test]
fn nearby_nodes_merge() {
    let mut state = connected();
    let far = Uuid::from_u128(12);
    let events = state.apply_ws(&M::NearbyNodes {
        nodes: vec![node(NODE, "available"), node(far, "available")],
    });
    assert_eq!(events.len(), 2);
    assert!(events.contains(&StateEvent::NodeAdded(NODE)));

    // A later answer that leaves a node out (walked away) keeps it; only a
    // real difference is reported.
    let events = state.apply_rest(&R::Nearby(NearbyResponse {
        nodes: vec![node(NODE, "harvesting")],
    }));
    assert_eq!(events, vec![StateEvent::NodeChanged(NODE)]);
    assert!(state.node(far).is_some());
    let events = state.apply_rest(&R::Nearby(NearbyResponse {
        nodes: vec![node(NODE, "harvesting")],
    }));
    assert!(events.is_empty());
}

// ---------------------------------------------------------------------------
// Inventory
// ---------------------------------------------------------------------------

#[test]
fn inventory_updated_is_filtered() {
    let mut state = connected();
    state.apply_rest(&containers());
    let updated = |player_id, container_id| M::InventoryUpdated {
        player_id,
        container_id,
    };
    assert_eq!(
        state.apply_ws(&updated(ME, INVENTORY)),
        vec![StateEvent::InventoryChanged {
            container_id: INVENTORY
        }]
    );
    // Another player's inventory: not ours.
    assert!(state
        .apply_ws(&updated(OTHER, Uuid::from_u128(99)))
        .is_empty());
    // Another player changing the station container we have open: ours too.
    state.apply_ws(&station_opened(ME));
    assert_eq!(
        state.apply_ws(&updated(OTHER, STATION_BOX)),
        vec![StateEvent::InventoryChanged {
            container_id: STATION_BOX
        }]
    );
}

#[test]
fn item_moved_reports_both_containers() {
    let mut state = connected();
    state.apply_rest(&containers());
    let moved = |player_id, to_container| M::ItemMoved {
        player_id,
        item_id: ITEM,
        from_container: INVENTORY,
        from_slot: 0,
        to_container,
        to_slot: 1,
    };
    assert_eq!(
        state.apply_ws(&moved(ME, CRAFT_IN)),
        vec![
            StateEvent::InventoryChanged {
                container_id: INVENTORY
            },
            StateEvent::InventoryChanged {
                container_id: CRAFT_IN
            }
        ]
    );
    assert_eq!(state.apply_ws(&moved(ME, INVENTORY)).len(), 1);
}

#[test]
fn items_merged_reports_the_inventory() {
    let mut state = connected();
    let merged = |player_id| M::ItemsMerged {
        player_id,
        source_item_id: ITEM,
        target_item_id: Uuid::from_u128(71),
        new_quantity: 4,
        new_quality: 61,
    };
    // No containers fetched yet: nothing to name.
    assert!(state.apply_ws(&merged(ME)).is_empty());
    state.apply_rest(&containers());
    assert!(state.apply_ws(&merged(OTHER)).is_empty());
    assert_eq!(
        state.apply_ws(&merged(ME)),
        vec![StateEvent::InventoryChanged {
            container_id: INVENTORY
        }]
    );
}

#[test]
fn containers_are_laid_out_by_slot() {
    let mut state = connected();
    assert_eq!(
        state.apply_rest(&containers()),
        vec![StateEvent::ContainersUpdated]
    );
    let inventory = state.inventory().unwrap();
    assert_eq!(inventory.id, INVENTORY);
    assert_eq!(inventory.slot_count(), 24);
    assert_eq!(inventory.used_slots(), 1);
    assert_eq!(inventory.slot(0).unwrap().quantity, 3);
    assert_eq!(inventory.first_empty(), Some(1));
    assert_eq!(state.crafting_input().unwrap().id, CRAFT_IN);
    assert!(state.crafting_output().is_none());
    assert!(state.container(CRAFT_IN).is_some());

    // A slot index outside the container is dropped, not a panic.
    let odd = Container::new(
        ITEM,
        "crafting_output",
        1,
        1,
        &[slot(1, 0, 1), slot(2, 5, 1)],
    );
    assert_eq!(odd.used_slots(), 1);
}

// ---------------------------------------------------------------------------
// Stations
// ---------------------------------------------------------------------------

#[test]
fn station_placed_fills_in_from_the_catalog() {
    let mut state = connected();
    let events = state.apply_ws(&M::StationPlaced {
        station_id: CHEST,
        station_type: "storage_chest".into(),
        name: "Storage Chest".into(),
        owner_id: ME,
        x: 4050.0,
        y: 4060.0,
        container_id: STATION_BOX,
    });
    assert_eq!(
        events,
        vec![
            StateEvent::StationAdded(CHEST),
            StateEvent::StationPlaced {
                station_id: CHEST,
                owner_id: ME
            }
        ]
    );
    let chest = state.station(CHEST).unwrap();
    assert_eq!(chest.category, "storage");
    assert_eq!(chest.icon.as_deref(), Some("chest"));
    assert_eq!(chest.interaction_range, 75.0);
    assert_eq!(chest.container_id, Some(STATION_BOX));
}

#[test]
fn station_removed_closes_it_if_open() {
    let mut state = connected();
    state.apply_rest(&R::NearbyStations(NearbyStationsResponse {
        stations: vec![station(STATION, ME), station(CHEST, OTHER)],
    }));
    state.apply_ws(&station_opened(ME));

    let events = state.apply_ws(&M::StationRemoved { station_id: CHEST });
    assert_eq!(events, vec![StateEvent::StationRemoved(CHEST)]);
    assert!(state.open_station().is_some());

    let events = state.apply_ws(&M::StationRemoved {
        station_id: STATION,
    });
    assert_eq!(
        events,
        vec![
            StateEvent::StationRemoved(STATION),
            StateEvent::StationClosed
        ]
    );
    assert!(state.open_station().is_none());
    assert!(state.stations().is_empty());
}

#[test]
fn station_opened_only_for_me() {
    let mut state = connected();
    assert!(state.apply_ws(&station_opened(OTHER)).is_empty());
    assert!(state.open_station().is_none());
    let events = state.apply_ws(&station_opened(ME));
    assert_eq!(
        events,
        vec![StateEvent::StationOpened {
            station_id: STATION,
            container_id: STATION_BOX
        }]
    );
    assert_eq!(state.open_station().unwrap().name, "Workbench");
    assert!(state.station_container().is_none(), "fetched separately");
}

#[test]
fn station_closed_only_for_me() {
    let mut state = connected();
    state.apply_ws(&station_opened(ME));
    assert!(state
        .apply_ws(&M::StationClosed { player_id: OTHER })
        .is_empty());
    assert!(state.open_station().is_some());
    assert_eq!(
        state.apply_ws(&M::StationClosed { player_id: ME }),
        vec![StateEvent::StationClosed]
    );
    assert!(state.open_station().is_none());
    assert!(state
        .apply_ws(&M::StationClosed { player_id: ME })
        .is_empty());
}

#[test]
fn nearby_stations_merge() {
    let mut state = connected();
    let events = state.apply_ws(&M::NearbyStations {
        stations: vec![station(STATION, ME)],
    });
    assert_eq!(events, vec![StateEvent::StationAdded(STATION)]);
    let mut moved = station(STATION, ME);
    moved.name = "Old Bench".into();
    let events = state.apply_rest(&R::NearbyStations(NearbyStationsResponse {
        stations: vec![moved],
    }));
    assert_eq!(events, vec![StateEvent::StationChanged(STATION)]);
    assert_eq!(state.station(STATION).unwrap().name, "Old Bench");
}

#[test]
fn station_container_is_kept_only_while_open() {
    let mut state = connected();
    let response = R::StationContainer(StationContainerResponse {
        id: STATION_BOX,
        container_type: "station_inventory".into(),
        slot_count: 16,
        layout_columns: 4,
        slots: vec![slot(200, 3, 1)],
    });
    // Arrives after the station closed: dropped.
    assert!(state.apply_rest(&response).is_empty());
    state.apply_ws(&station_opened(ME));
    assert_eq!(
        state.apply_rest(&response),
        vec![StateEvent::StationContainerUpdated]
    );
    let container = state.station_container().unwrap();
    assert_eq!(container.slot(3).unwrap().item_id, Uuid::from_u128(200));
    assert!(state.container(STATION_BOX).is_some());
    state.apply_ws(&M::StationClosed { player_id: ME });
    assert!(state.station_container().is_none());
}

// ---------------------------------------------------------------------------
// Currency, terrain (trades are in `trade/tests.rs`)
// ---------------------------------------------------------------------------

#[test]
fn currency_changed_sets_the_balance() {
    let mut state = connected();
    assert_eq!(state.strands(), None);
    assert_eq!(
        state.apply_ws(&M::CurrencyChanged { new_balance: 0 }),
        vec![StateEvent::CurrencyChanged { balance: 0 }]
    );
    assert!(state
        .apply_ws(&M::CurrencyChanged { new_balance: 0 })
        .is_empty());
    state.apply_ws(&M::CurrencyChanged { new_balance: 1250 });
    assert_eq!(state.strands(), Some(1250));
}

#[test]
fn terrain_updated_is_reported() {
    let mut state = connected();
    assert_eq!(
        state.apply_ws(&M::TerrainUpdated),
        vec![StateEvent::TerrainUpdated]
    );
}

#[test]
fn plot_claimed_is_reported() {
    let mut state = connected();
    let claimed = ClaimedPlot {
        terrain_id: "sub_2_0_3_L_4".into(),
        owner_id: OTHER,
        claimed_at: "2026-10-17T18:30:00Z".parse().unwrap(),
    };
    assert_eq!(
        state.apply_ws(&M::PlotClaimed {
            terrain_id: claimed.terrain_id.clone(),
            owner_id: claimed.owner_id,
            claimed_at: claimed.claimed_at,
        }),
        vec![StateEvent::LotClaimed(claimed)]
    );
}

#[test]
fn plot_released_drops_it_from_my_plots() {
    let mut state = connected();
    let mine = PlotInfo {
        terrain_id: Some("sub_2_0_3_L_4".into()),
        ..plot(Some(ME))
    };
    state.apply_rest(&R::Plots(ListPlotsResponse {
        plots: vec![mine.clone()],
        total: 1,
    }));
    state.apply_rest(&R::PlotAt(Some(mine)));
    let released = |id: &str| M::PlotReleased {
        terrain_id: id.into(),
    };

    // Someone else's lot: reported, but none of mine changed.
    assert_eq!(
        state.apply_ws(&released("rural_plot_trail_9")),
        vec![StateEvent::LotReleased {
            terrain_id: "rural_plot_trail_9".into()
        }]
    );
    assert_eq!(state.plots().len(), 1);

    assert_eq!(
        state.apply_ws(&released("sub_2_0_3_L_4")),
        vec![
            StateEvent::PlotsUpdated,
            StateEvent::LotReleased {
                terrain_id: "sub_2_0_3_L_4".into()
            }
        ]
    );
    assert!(state.plots().is_empty());
    let picked = state.picked_plot().unwrap();
    assert_eq!(
        (picked.owner_id, picked.owner_name.as_deref()),
        (None, None)
    );
}

// ---------------------------------------------------------------------------
// REST
// ---------------------------------------------------------------------------

#[test]
fn login_sets_the_local_player() {
    let mut state = GameState::new();
    state.apply_rest(&R::Auth(AuthResponse {
        token: "t".into(),
        account: AccountInfo {
            id: Uuid::from_u128(3),
            email: "a@b.c".into(),
            tier: "free".into(),
            is_admin: false,
        },
        player_id: ME,
    }));
    assert_eq!(state.me(), Some(ME));
}

fn me_response(online: bool) -> R {
    R::Me(PlayerMeResponse {
        player_id: ME,
        name: "Ada".into(),
        x: 4000.0,
        y: 7350.0,
        zone: "rural".into(),
        strand_balance: 250,
        online,
    })
}

#[test]
fn player_me_is_the_local_player_before_any_positions() {
    let mut state = connected();
    let events = state.apply_rest(&me_response(true));
    assert_eq!(
        events,
        vec![
            StateEvent::CurrencyChanged { balance: 250 },
            StateEvent::PlayerJoined(ME),
            StateEvent::ZoneChanged {
                from: None,
                to: Zone::Rural
            },
        ]
    );
    assert_eq!(state.local_player(), Some(&pos(ME, "Ada", 4000.0, 7350.0)));
    assert_eq!(state.strands(), Some(250));

    // The first positions moves me without joining me again.
    let events = state.apply_ws(&M::Positions {
        players: vec![pos(ME, "Ada", 4000.0, 7345.0)],
    });
    assert!(!events.contains(&StateEvent::PlayerJoined(ME)));
    assert_eq!(state.local_player().unwrap().y, 7345.0);
}

#[test]
fn player_me_never_moves_a_player_positions_already_placed() {
    let mut state = with_players();
    let events = state.apply_rest(&me_response(true));
    assert_eq!(events, vec![StateEvent::CurrencyChanged { balance: 250 }]);
    assert_eq!(state.local_player(), Some(&pos(ME, "Ada", 4000.0, 4000.0)));
}

#[test]
fn player_me_offline_or_disconnected_only_sets_the_balance() {
    // Offline: the position is the saved one, and the player isn't in the game.
    let mut state = connected();
    let events = state.apply_rest(&me_response(false));
    assert_eq!(events, vec![StateEvent::CurrencyChanged { balance: 250 }]);
    assert_eq!(state.local_player(), None);

    // A late answer after the connection dropped can't leave a ghost.
    let mut state = GameState::new();
    state.apply_rest(&me_response(true));
    assert_eq!(state.local_player(), None);
    assert_eq!(state.me(), Some(ME));
}

#[test]
fn balances_come_from_every_money_response() {
    let mut state = connected();
    let cases = [
        (
            R::Currency(CurrencyResponse {
                strand_balance: 100,
            }),
            100,
        ),
        (
            R::Deposit(DepositResponse {
                deposited_quantity: 10,
                fee: 1,
                strands_received: 9,
                new_balance: 109,
            }),
            109,
        ),
        (
            R::Withdraw(WithdrawResponse {
                strands_spent: 10,
                fee: 1,
                fiber_received: 9,
                new_balance: 99,
            }),
            99,
        ),
    ];
    for (response, balance) in cases {
        assert_eq!(
            state.apply_rest(&response),
            vec![StateEvent::CurrencyChanged { balance }]
        );
        assert_eq!(state.strands(), Some(balance));
    }
}

#[test]
fn recipes_and_plots_are_stored() {
    let mut state = connected();
    let recipe = RecipeInfo {
        id: "rope_basic".into(),
        name: "Basic Rope".into(),
        description: None,
        category: "hand_crafting".into(),
        inputs: vec![],
        outputs: vec![],
        duration_ticks: 50,
        required_station_type: None,
    };
    assert_eq!(
        state.apply_rest(&R::Recipes(RecipesResponse {
            recipes: vec![recipe]
        })),
        vec![StateEvent::RecipesUpdated]
    );
    assert_eq!(state.recipe("rope_basic").unwrap().duration_ticks, 50);

    assert_eq!(
        state.apply_rest(&R::Plots(ListPlotsResponse {
            plots: vec![plot(None)],
            total: 1
        })),
        vec![StateEvent::PlotsUpdated]
    );
    // A plot that turns out not to be yours leaves your list.
    assert_eq!(
        state.apply_rest(&R::Plot(plot(None))),
        vec![StateEvent::PlotsUpdated]
    );
    assert!(state.plots().is_empty());
    // One that is yours joins it, once.
    assert_eq!(
        state.apply_rest(&R::Plot(plot(Some(ME)))),
        vec![StateEvent::PlotsUpdated]
    );
    assert!(state.apply_rest(&R::Plot(plot(Some(ME)))).is_empty());
    assert_eq!(state.plots().len(), 1);
    assert!(state.my_plot_at(5200.0, 4000.0).is_some());
    assert!(state.my_plot_at(5300.0, 4000.0).is_none());
}

#[test]
fn a_picked_plot_is_kept_fresh_and_nothing_there_unpicks_it() {
    let mut state = connected();
    assert!(state.apply_rest(&R::PlotAt(None)).is_empty());
    assert_eq!(
        state.apply_rest(&R::PlotAt(Some(plot(None)))),
        vec![StateEvent::PlotPicked {
            plot_id: Some(PLOT)
        }]
    );
    // Someone else's, so not in your list.
    assert!(state.plots().is_empty());
    assert_eq!(state.picked_plot().unwrap().owner_id, None);
    // A refetch of it updates the panel.
    let mut theirs = plot(Some(OTHER));
    theirs.owner_name = Some("Brin".into());
    assert_eq!(
        state.apply_rest(&R::Plot(theirs)),
        vec![StateEvent::PlotsUpdated]
    );
    assert_eq!(state.picked_plot().unwrap().owner_id, Some(OTHER));
    assert_eq!(
        state.apply_rest(&R::PlotAt(None)),
        vec![StateEvent::PlotPicked { plot_id: None }]
    );
    assert!(state.picked_plot().is_none());
}

#[test]
fn claiming_a_plot_updates_it_and_the_balance() {
    let mut state = connected();
    state.apply_rest(&R::Currency(CurrencyResponse {
        strand_balance: 1000,
    }));
    state.apply_rest(&R::PlotAt(Some(plot(None))));
    let events = state.apply_rest(&R::ClaimPlot(ClaimPlotResponse {
        plot: plot(Some(ME)),
        new_balance: 500,
    }));
    assert_eq!(
        events,
        vec![
            StateEvent::PlotsUpdated,
            StateEvent::PlotClaimed { plot_id: PLOT },
            StateEvent::CurrencyChanged { balance: 500 }
        ]
    );
    assert_eq!(state.plot(PLOT).unwrap().owner_id, Some(ME));
    assert_eq!(state.plots().len(), 1);
    assert_eq!(state.picked_plot().unwrap().owner_id, Some(ME));
}

#[test]
fn placing_a_station_by_rest_adds_it() {
    let mut state = connected();
    let events = state.apply_rest(&R::PlaceStation(PlaceStationResponse {
        station: station(STATION, ME),
    }));
    assert_eq!(events, vec![StateEvent::StationAdded(STATION)]);
    // The station_placed broadcast that follows adds nothing new.
    let events = state.apply_ws(&M::StationPlaced {
        station_id: STATION,
        station_type: "workbench".into(),
        name: "Workbench".into(),
        owner_id: ME,
        x: 4020.0,
        y: 4020.0,
        container_id: STATION_BOX,
    });
    assert_eq!(
        events,
        vec![StateEvent::StationPlaced {
            station_id: STATION,
            owner_id: ME
        }]
    );
}

#[test]
fn rest_failures_and_unmirrored_responses_change_nothing() {
    let mut state = with_players();
    let before = format!("{state:?}");
    let failure = NetEvent::Rest(
        7,
        Err(RestError::Http {
            status: 401,
            message: "Unauthorized".into(),
        }),
    );
    assert!(state.apply(&failure).is_empty());
    for response in [
        R::Success(SuccessResponse::ok()),
        R::Extract(ExtractResponse {
            success: true,
            message: None,
        }),
        R::Inventory(InventoryResponse { items: vec![] }),
    ] {
        assert!(state.apply_rest(&response).is_empty());
    }
    assert_eq!(format!("{state:?}"), before);
}

// ---------------------------------------------------------------------------
// Connection and reconnect
// ---------------------------------------------------------------------------

#[test]
fn first_connect_asks_for_a_snapshot() {
    let mut state = GameState::new();
    assert!(!state.is_connected());
    let connecting = state.apply(&NetEvent::Connection(ConnectionState::Connecting));
    assert_eq!(
        connecting,
        vec![StateEvent::Connection(ConnectionState::Connecting)]
    );
    let connected = ConnectionState::Connected { player_id: ME };
    let events = state.apply(&NetEvent::Connection(connected.clone()));
    assert_eq!(
        events,
        vec![
            StateEvent::Connection(connected.clone()),
            StateEvent::SnapshotStale
        ]
    );
    assert!(state.is_connected());
    assert_eq!(state.me(), Some(ME));

    // Reconnecting and connecting again: the snapshot request comes from Resync.
    let reconnecting = ConnectionState::Reconnecting {
        attempt: 1,
        delay: Duration::from_millis(500),
        reason: "reset".into(),
    };
    state.apply(&NetEvent::Connection(reconnecting));
    assert!(!state.is_connected());
    let events = state.apply(&NetEvent::Connection(connected.clone()));
    assert_eq!(events, vec![StateEvent::Connection(connected)]);
}

#[test]
fn resync_drops_what_the_server_dropped() {
    let mut state = with_players();
    state.apply_ws(&extraction_started(ME));
    state.apply_ws(&crafting_started(OTHER));
    state.apply_ws(&station_opened(ME));

    let events = state.apply(&NetEvent::Resync);
    assert!(events.contains(&StateEvent::ActivityChanged { player_id: ME }));
    assert!(events.contains(&StateEvent::ActivityChanged { player_id: OTHER }));
    assert!(events.contains(&StateEvent::StationClosed));
    assert_eq!(events.last(), Some(&StateEvent::SnapshotStale));
    assert!(state.local_activity().is_none());
    assert!(state.activity(OTHER).is_none());
    assert!(state.open_station().is_none());
}

#[test]
fn after_resync_nearby_answers_replace() {
    let mut state = connected();
    let gone_node = Uuid::from_u128(13);
    state.apply_rest(&R::Nearby(NearbyResponse {
        nodes: vec![node(NODE, "available"), node(gone_node, "available")],
    }));
    state.apply_rest(&R::NearbyStations(NearbyStationsResponse {
        stations: vec![station(STATION, ME), station(CHEST, OTHER)],
    }));

    state.apply(&NetEvent::Resync);
    // The chest was removed while we were away.
    let events = state.apply_rest(&R::NearbyStations(NearbyStationsResponse {
        stations: vec![station(STATION, ME)],
    }));
    assert_eq!(events, vec![StateEvent::StationRemoved(CHEST)]);
    let events = state.apply_rest(&R::Nearby(NearbyResponse {
        nodes: vec![node(NODE, "depleted")],
    }));
    assert_eq!(
        events,
        vec![
            StateEvent::NodeRemoved(gone_node),
            StateEvent::NodeChanged(NODE)
        ]
    );

    // Only the first answer after a resync replaces; later ones merge again.
    let events = state.apply_rest(&R::Nearby(NearbyResponse { nodes: vec![] }));
    assert!(events.is_empty());
    assert!(state.node(NODE).is_some());
}

#[test]
fn auth_failed_is_reported_as_a_connection_state() {
    let mut state = connected();
    let failed = ConnectionState::AuthFailed {
        message: "Session expired".into(),
    };
    assert_eq!(
        state.apply(&NetEvent::Connection(failed.clone())),
        vec![StateEvent::Connection(failed.clone())]
    );
    assert_eq!(state.connection(), &failed);
    assert!(!state.is_connected());
}
