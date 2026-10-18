//! Round-trip every WebSocket message type through its JSON fixture.
//!
//! `fixtures/server/*.json` were captured from the backend's own `ServerMessage`
//! enum before it moved into this crate, so they are the server's exact bytes.
//! `fixtures/client/*.json` match what the React client sends (`GameContext.tsx`).
//! A message must parse from its fixture and serialise back to the same bytes.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use capitol_protocol::ws::{ClientMessage, ServerMessage};
use serde::{de::DeserializeOwned, Serialize};

const SERVER_TYPES: &[&str] = &[
    "auth_ok",
    "auth_error",
    "subscribed",
    "positions",
    "player_joined",
    "player_left",
    "error",
    "extraction_started",
    "extraction_progress",
    "extraction_completed",
    "extraction_cancelled",
    "extraction_failed",
    "node_depleted",
    "node_regenerated",
    "nearby_nodes",
    "crafting_started",
    "crafting_progress",
    "crafting_completed",
    "crafting_cancelled",
    "crafting_failed",
    "inventory_updated",
    "item_moved",
    "items_merged",
    "station_placed",
    "station_removed",
    "station_opened",
    "station_closed",
    "nearby_stations",
    "trade_requested",
    "trade_request_declined",
    "trade_started",
    "trade_offer_updated",
    "trade_accepted",
    "trade_executed",
    "trade_cancelled",
    "currency_changed",
    "zone_changed",
    "movement_blocked",
    "terrain_updated",
    "plot_claimed",
    "plot_released",
];

const CLIENT_TYPES: &[&str] = &[
    "auth",
    "subscribe",
    "move",
    "extract",
    "cancel_extraction",
    "craft",
    "cancel_crafting",
    "move_item",
    "place_station",
    "remove_station",
    "open_station",
    "close_station",
    "craft_at_station",
    "trade_initiate",
    "trade_accept_request",
    "trade_decline_request",
    "trade_offer",
    "trade_accept",
    "trade_cancel",
];

fn fixtures(dir: &str) -> Vec<(String, String)> {
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir);
    let mut out: Vec<(String, String)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            let path = entry.unwrap().path();
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            let json = fs::read_to_string(&path).unwrap().trim_end().to_string();
            (stem, json)
        })
        .collect();
    out.sort();
    out
}

/// Parse each fixture, check its tag matches the file name, and re-serialise it
/// byte for byte. Returns the set of tags seen.
fn round_trip<T>(dir: &str, type_name: impl Fn(&T) -> &'static str) -> BTreeSet<&'static str>
where
    T: Serialize + DeserializeOwned + std::fmt::Debug,
{
    let mut seen = BTreeSet::new();
    for (stem, json) in fixtures(dir) {
        let msg: T = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("{dir}/{stem}.json doesn't parse: {e}"));
        let tag = type_name(&msg);
        assert_eq!(tag, stem, "{dir}/{stem}.json parsed as a {tag}");
        let again = serde_json::to_string(&msg).unwrap();
        assert_eq!(again, json, "{dir}/{stem}.json changed on the way back out");
        seen.insert(tag);
    }
    seen
}

#[test]
fn server_messages_round_trip_byte_for_byte() {
    let seen = round_trip::<ServerMessage>("server", ServerMessage::type_name);
    let expected: BTreeSet<_> = SERVER_TYPES.iter().copied().collect();
    assert_eq!(
        seen, expected,
        "every ServerMessage type needs exactly one fixture"
    );
}

#[test]
fn client_messages_round_trip_byte_for_byte() {
    let seen = round_trip::<ClientMessage>("client", ClientMessage::type_name);
    let expected: BTreeSet<_> = CLIENT_TYPES.iter().copied().collect();
    assert_eq!(
        seen, expected,
        "every ClientMessage type needs exactly one fixture"
    );
}

#[test]
fn javascript_integers_parse_as_coordinates() {
    // JSON.stringify writes 4100, not 4100.0.
    let msg: ClientMessage = serde_json::from_str(r#"{"type":"move","x":4100,"y":3900}"#).unwrap();
    assert_eq!(
        msg,
        ClientMessage::Move {
            x: 4100.0,
            y: 3900.0
        }
    );
}

#[test]
fn unit_variants_are_just_a_tag() {
    assert_eq!(
        serde_json::to_string(&ServerMessage::TerrainUpdated).unwrap(),
        r#"{"type":"terrain_updated"}"#
    );
    assert_eq!(
        serde_json::to_string(&ClientMessage::CancelExtraction).unwrap(),
        r#"{"type":"cancel_extraction"}"#
    );
}

#[test]
fn unknown_types_are_rejected() {
    assert!(serde_json::from_str::<ServerMessage>(r#"{"type":"teleported"}"#).is_err());
}

#[test]
fn an_error_from_an_older_server_names_no_command() {
    // Before #89, `error` carried only `message`.
    let msg: ServerMessage = serde_json::from_str(r#"{"type":"error","message":"Too far away"}"#).unwrap();
    assert_eq!(
        msg,
        ServerMessage::Error {
            message: "Too far away".into(),
            command: String::new(),
        }
    );
}
