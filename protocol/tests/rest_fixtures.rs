//! Round-trip REST bodies through their JSON fixtures, byte for byte.
//!
//! `fixtures/rest/catalog.json` is `GET /catalog` from a server on the seed
//! content. The Flint client's offline fallback is the same file
//! (`capitol-core/src/catalog_fallback.json`), and a test there keeps them equal.
//! `fixtures/rest/stations_nearby.json` is `GET /stations/nearby`.
//! `fixtures/rest/split_stack.json` is `POST /items/{id}/split` taking named qualities (E1.8).
//! `fixtures/rest/place_order.json` is a sell order naming its units (E3.1).

use std::fs;
use std::path::Path;

use capitol_protocol::rest::{CatalogResponse, NearbyStationsResponse, PlaceOrder, SplitStackResponse};
use capitol_protocol::rules::market::{GradeRule, OrderSide};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rest")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .trim_end()
        .to_string()
}

#[test]
fn the_catalog_round_trips_byte_for_byte() {
    let json = fixture("catalog.json");
    let catalog: CatalogResponse = serde_json::from_str(&json).unwrap();
    assert!(!catalog.version.is_empty());
    assert!(catalog.item_types.iter().any(|t| t.id == "fiber"));
    assert_eq!(serde_json::to_string(&catalog).unwrap(), json);
}

#[test]
fn nearby_stations_round_trip_with_their_quality() {
    let json = fixture("stations_nearby.json");
    let nearby: NearbyStationsResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(nearby.stations[0].quality, 82);
    assert_eq!(serde_json::to_string(&nearby).unwrap(), json);
}

#[test]
fn a_split_round_trips_with_its_units() {
    let json = fixture("split_stack.json");
    let split: SplitStackResponse = serde_json::from_str(&json).unwrap();
    assert_eq!((split.item.quality_grade.as_str(), split.item.min_quality), ("A", 88));
    assert_eq!(split.item.units.mean(), Some(split.item.quality));
    assert!(split.item.units.spans_grades());
    assert_eq!(serde_json::to_string(&split).unwrap(), json);
}

#[test]
fn a_sell_order_round_trips_with_its_units() {
    let json = fixture("place_order.json");
    let order: PlaceOrder = serde_json::from_str(&json).unwrap();
    assert_eq!(order.side, OrderSide::Sell);
    assert_eq!(order.qualities.as_ref().map(|u| u.count()), Some(order.quantity as i64));
    assert_eq!(serde_json::to_string(&order).unwrap(), json);
}

#[test]
fn a_bare_buy_order_takes_the_defaults() {
    let order: PlaceOrder =
        serde_json::from_str(r#"{"side":"buy","item_type":"plank","price":12,"quantity":20}"#).unwrap();
    assert_eq!((order.item_id, order.qualities, order.min_grade, order.grade_rule), (None, None, None, None));
    assert_eq!(GradeRule::default(), GradeRule::Average);
}
