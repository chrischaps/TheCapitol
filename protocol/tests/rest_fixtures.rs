//! Round-trip REST bodies through their JSON fixtures, byte for byte.
//!
//! `fixtures/rest/catalog.json` is `GET /catalog` from a server on the seed
//! content. The Flint client's offline fallback is the same file
//! (`capitol-core/src/catalog_fallback.json`), and a test there keeps them equal.
//! `fixtures/rest/stations_nearby.json` is `GET /stations/nearby`, a station running (E4.2).
//! `fixtures/rest/split_stack.json` is `POST /items/{id}/split` taking named qualities (E1.8).
//! `fixtures/rest/place_order.json` is a sell order naming its units (E3.1).
//! `fixtures/rest/market_depth.json`, `market_history.json` and
//! `market_summary.json` are `GET /market/orders`, `/market/history` and
//! `/market/summary` (E3.3).
//! `fixtures/rest/bazaar_directory.json` and `bazaar_shop.json` are
//! `GET /bazaar` and `GET /bazaar/stalls/{id}` (E3.10).
//! `fixtures/rest/gazette.json` is `GET /gazette/latest` (E3.9), written by the
//! backend's `write_the_gazette_fixture` from a busy day's records.
//! `fixtures/rest/ledger.json` is `GET /admin/ledger` (C1.7), its load split
//! by the tick's phases and the loops beside it (D0.1).

use std::fs;
use std::path::Path;

use capitol_protocol::rest::{
    BazaarDirectory, BazaarShop,
    AutomationMode, AutomationState, CatalogResponse, GazetteResponse, MarketDepthResponse, MarketHistoryResponse,
    MarketSummaryResponse, NearbyStationsResponse, PlaceOrder, SplitStackResponse,
};
use capitol_protocol::ledger::{Ledger, Load, LoadWindow};
use capitol_protocol::rules::market::{sort_levels, GradeRule, OrderSide};

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
    let automation = nearby.stations[0].automation.as_ref().unwrap();
    assert_eq!((automation.mode, &automation.state), (AutomationMode::Count(20), &AutomationState::Running));
    assert!(!automation.is_finished());
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

#[test]
fn a_book_round_trips_in_book_order() {
    let json = fixture("market_depth.json");
    let depth: MarketDepthResponse = serde_json::from_str(&json).unwrap();
    let mut sorted = depth.levels.clone();
    sort_levels(&mut sorted);
    assert_eq!(sorted, depth.levels);
    assert_eq!(serde_json::to_string(&depth).unwrap(), json);
}

#[test]
fn history_and_the_summary_round_trip() {
    let json = fixture("market_history.json");
    let history: MarketHistoryResponse = serde_json::from_str(&json).unwrap();
    assert!(history.candles.iter().all(|c| c.low <= c.open.min(c.close) && c.high >= c.open.max(c.close)));
    assert_eq!(history.mine[0].trade, history.recent[0]);
    assert_eq!(serde_json::to_string(&history).unwrap(), json);
    let json = fixture("market_summary.json");
    let summary: MarketSummaryResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(summary.items[1].last_price, None);
    assert_eq!(serde_json::to_string(&summary).unwrap(), json);
}

#[test]
fn a_gazette_round_trips_with_every_section() {
    let json = fixture("gazette.json");
    let response: GazetteResponse = serde_json::from_str(&json).unwrap();
    let edition = response.edition.as_ref().unwrap();
    assert_eq!(edition.number, capitol_protocol::rules::gazette_number(edition.day));
    assert!(!edition.movers.is_empty() && !edition.prices.is_empty() && !edition.deeds.is_empty());
    assert!(edition.best_sale.is_some() && !edition.producers.is_empty());
    assert_eq!(serde_json::to_string(&response).unwrap(), json);
}

#[test]
fn the_bazaar_directory_round_trips_without_prices() {
    let json = fixture("bazaar_directory.json");
    let directory: BazaarDirectory = serde_json::from_str(&json).unwrap();
    assert_eq!(directory.stalls[0].shop, None);
    assert_eq!(directory.stalls[1].shop.as_ref().unwrap().item_types, ["fiber", "rope"]);
    for hidden in ["price", "units", "quantity", "grade"] {
        assert!(!json.contains(hidden), "{hidden}");
    }
    assert_eq!(serde_json::to_string(&directory).unwrap(), json);
}

#[test]
fn a_shop_round_trips_with_each_listings_units() {
    let json = fixture("bazaar_shop.json");
    let shop: BazaarShop = serde_json::from_str(&json).unwrap();
    assert_eq!(shop.listings[0].units.count(), 10);
    assert!(shop.listings[0].units.spans_grades());
    assert_eq!((shop.listings[0].durability, shop.listings[1].durability), (None, Some(31)));
    // The mixed ropes name no maker; the axe is one unit, so its maker shows (#242)
    assert_eq!(shop.listings[0].maker, None);
    assert_eq!(shop.listings[1].maker.as_ref().map(|m| m.name.as_str()), Some("ada"));
    assert_eq!(serde_json::to_string(&shop).unwrap(), json);
}

#[test]
fn the_ledger_round_trips_with_its_phases() {
    let json = fixture("ledger.json");
    let ledger: Ledger = serde_json::from_str(&json).unwrap();
    let load = &ledger.load;
    assert_eq!(load.phases.len(), 13);
    assert_eq!(load.loops.len(), 3);
    // The phases cover the whole tick
    let phases: f64 = load.phases.values().map(|p| p.secs).sum();
    assert!((phases - load.tick_secs).abs() < 1e-9, "{phases} of {}", load.tick_secs);
    assert_eq!(serde_json::to_string(&ledger).unwrap(), json);
}

#[test]
fn a_load_from_before_the_phases_still_reads() {
    let json = fixture("ledger.json");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let load = value["load"].as_object_mut().unwrap();
    load.remove("phases");
    load.remove("loops");
    let old: Load = serde_json::from_value(value["load"].clone()).unwrap();
    assert!(old.phases.is_empty() && old.loops.is_empty());
    let new: Ledger = serde_json::from_str(&json).unwrap();
    // A window from an old load to a new one counts each phase from nothing
    let window = LoadWindow::between(&old, &new.load, 60.0);
    let gathers = &window.phases["gathers"];
    assert_eq!(gathers.runs, 1180);
    assert!((gathers.mean_ms - 5.0).abs() < 1e-9);
    assert_eq!(gathers.p95_ms, Some(20.0));
    assert_eq!(window.loops["market"].p95_ms, Some(20.0));
}
