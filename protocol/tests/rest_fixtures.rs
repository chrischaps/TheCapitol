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
//! `fixtures/rest/market_listings.json` is `GET /market/listings` (#493): a
//! blended lot of planks listed after a single quality of rope.
//! `fixtures/rest/bazaar_directory.json` and `bazaar_shop.json` are
//! `GET /bazaar` and `GET /bazaar/stalls/{id}` (E3.10).
//! `fixtures/rest/gazette.json` is `GET /gazette/latest` (E3.9), written by the
//! backend's `write_the_gazette_fixture` from a busy day's records.
//! `fixtures/rest/ledger.json` is `GET /admin/ledger` (C1.7), its load split
//! by the tick's phases and the loops beside it (D0.1), each with its slow
//! statements (D3.2), and the road in's deed grants (#440): three opened, one
//! handed back by a retired lot, two held.
//! `fixtures/rest/sample.json` is one of the Observatory's samples (D0.2): ten
//! seconds of a 20-citizen town, with its wealth pooling (D2.5), its two
//! reports' summary (D4.1), its wealth by trade (D2.2), and the books in logs
//! and planks (D2.3), planks with nobody bidding, and its deed grants (#440).
//! `fixtures/rest/admin_feed.json` is one of each of the rest of what
//! `GET /admin/stream` sends (D0.3); the sample is `sample.json`.
//! `fixtures/rest/town_report.json` is `POST /admin/town/report` (D4.1): a
//! citizen at work with two decisions, a merchant resting after a failure,
//! and one away.
//! `fixtures/rest/admin_walkers.json` is `GET /admin/walkers` (D5.3): a
//! citizen walking to its next leg, a person gathering, one crafting. It's
//! also what `GET /admin/positions` (D1.3) sends, for every player.
//! `fixtures/rest/admin_map.json` is `GET /admin/map` (D1.3): two zones, a citizen's farm
//! and a person's lot, their stations, and an hour's fills at the hall and two stalls.
//! `fixtures/rest/admin_books.json` is `GET /admin/books` (D2.1): every transaction
//! type this build knows and what it does to the Strands.
//! `fixtures/rest/admin_player.json` is `GET /admin/player` (D4.3): a citizen's
//! hour, a deposit, a buy order's escrow and a sale less its fee, out walking.
//! `fixtures/rest/player_card.json` is `GET /players/{id}/card` (S0.3): a
//! citizen with a farm, a week in the city, in no guild, seen by someone else.
//! `fixtures/rest/my_card.json` is your own card (S3.1): your standing's
//! number, its trend and the history under the words, which only you see.
//! `fixtures/rest/mutes.json` is `GET /player/mutes` (S1.4): a citizen and a
//! person muted.
//! `fixtures/rest/mailbox.json` is `GET /letters?box=in` (S1.5): a citizen's
//! letter unread, a person's read, and older ones to come;
//! `send_letter.json` is `POST /letters`, a reply in paragraphs.
//! `fixtures/rest/post_commission.json`, `deliver_commission.json`,
//! `commissions.json` and `commission_delivery.json` are `POST /commissions`,
//! `POST /commissions/{id}/deliver` and their answers (S2.1): a citizen's
//! commission part delivered beside a person's, and the delivery that fills it.
//! `fixtures/rest/claim_commission.json` and `claim_response.json` are
//! `POST /commissions/{id}/claim` and its answer (S2.2): a citizen's claim on
//! 4, and the commission holding them; `claims.json` is
//! `GET /commissions/{id}/claims`, that claim part delivered and a person's kept.
//! `fixtures/rest/my_guild.json` is `GET /player/guild` (S4.1) as a guild's
//! leader sees it: a citizen among the members, a citizen's application and
//! an officer's invitation waiting; `guilds.json` is `GET /guilds`, and
//! `found_guild.json` is `POST /guilds`, a charter in paragraphs. Since S4.2
//! the leader sees the treasury too, and `guild_books.json` is
//! `GET /guilds/{id}/books`: dues, an officer's withdrawal, a roster fee the
//! treasury paid and a citizen's deposit, newest first.
//! `fixtures/rest/wages.json` is `GET /wages?mine=true` (S2.6): a citizen on
//! a shift gathering on a named offer, and an open forge;
//! `offer_wages.json` is `POST /wages`, and `shift_ended.json` is
//! `DELETE /wages/{id}/shift`, the employer stopping it.
//! `fixtures/rest/tutorial.json` is `GET /tutorial` (#438): the road in
//! walked out of order, out to the hall, the card showing;
//! `tutorial_granted.json` is the road walked to the Deeds Office, its deed
//! open to spend (#448).
//! `fixtures/rest/free_lots.json` is `GET /bureaucracy/free-lots?zone=rural`
//! (#448): two farms, nearest the hall first.

use std::fs;
use std::path::Path;

use capitol_protocol::rest::{
    BazaarDirectory, BazaarShop,
    AutomationMode, AutomationState, CatalogResponse, GazetteResponse, MarketDepthResponse, MarketHistoryResponse,
    MarketListingsResponse, MarketSummaryResponse, NearbyStationsResponse, PlaceOrder, SplitStackResponse,
};
use capitol_protocol::ledger::{Ledger, Load, LoadWindow};
use capitol_protocol::metrics::{Feed, Sample, TownReport, ROLE_PERSON};
use capitol_protocol::watch::{Watch, WatchKind};
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
fn the_newest_listings_round_trip_newest_first() {
    let json = fixture("market_listings.json");
    let listings: MarketListingsResponse = serde_json::from_str(&json).unwrap();
    assert!(listings.listings.windows(2).all(|w| w[0].listed_at >= w[1].listed_at));
    assert!(listings.listings.iter().all(|l| l.min_quality <= l.quality));
    assert_eq!(serde_json::to_string(&listings).unwrap(), json);
}

#[test]
fn a_gazette_round_trips_with_every_section() {
    let json = fixture("gazette.json");
    let response: GazetteResponse = serde_json::from_str(&json).unwrap();
    let edition = response.edition.as_ref().unwrap();
    assert_eq!(edition.number, capitol_protocol::rules::gazette_number(edition.day));
    assert!(!edition.movers.is_empty() && !edition.prices.is_empty() && !edition.deeds.is_empty());
    assert!(edition.best_sale.is_some() && !edition.producers.is_empty());
    // The hall's board (S2.4)
    let wanted = edition.wanted.as_ref().unwrap();
    assert!(!wanted.story.is_empty() && !wanted.wanted.is_empty() && wanted.came_through.is_some());
    assert_eq!(serde_json::to_string(&response).unwrap(), json);
}

/// An edition kept from before commissions has no `wanted`, and still reads
/// as it was printed (S2.4)
#[test]
fn a_gazette_from_before_the_board_still_reads() {
    let mut old: serde_json::Value = serde_json::from_str(&fixture("gazette.json")).unwrap();
    old["edition"].as_object_mut().unwrap().remove("wanted");
    let json = old.to_string();
    let response: GazetteResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(response.edition.as_ref().unwrap().wanted, None);
    assert_eq!(serde_json::to_value(&response).unwrap(), old);
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
    assert_eq!((ledger.grants.opened, ledger.grants.used, ledger.grants.granted_value), (3, 2, 17200));
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

#[test]
fn a_load_counts_slow_statements_by_phase() {
    let json = fixture("ledger.json");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let new: Ledger = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(new.load.slow_statement_ms, 50.0);
    // A load from before D3.2 reads with none, and a window from it counts
    // each phase's from nothing
    let load = value["load"].as_object_mut().unwrap();
    load.remove("slow_statements_elsewhere");
    load.remove("slow_statement_ms");
    for phase in load["phases"].as_object_mut().unwrap().values_mut() {
        phase.as_object_mut().unwrap().remove("slow_statements");
    }
    let old: Load = serde_json::from_value(value["load"].clone()).unwrap();
    assert!(old.phases.values().all(|p| p.slow_statements == 0));
    let window = LoadWindow::between(&old, &new.load, 60.0);
    assert_eq!(window.phases["positions"].slow_statements, 4);
    assert_eq!(window.phases["movement"].slow_statements, 0);
    assert_eq!(window.loops["market"].slow_statements, 0, "the old load's loops kept theirs");
    assert_eq!(window.slow_statements_elsewhere, 1);
}

#[test]
fn a_sample_round_trips_with_its_window() {
    let json = fixture("sample.json");
    let sample: Sample = serde_json::from_str(&json).unwrap();
    assert_eq!(sample.since_ms, 10_000);
    assert_eq!(sample.load.phases.len(), 12);
    assert_eq!(sample.load.loops["sampler"].runs, 1);
    assert_eq!(sample.strands.total(), 61240 + 412 + 50000 + 1836);
    assert_eq!(sample.market["log"].vwap(), Some(36.0 / 7.0));
    assert_eq!(sample.books["log"].best_bid, Some(5));
    assert_eq!(sample.books["plank"].best_bid, None);
    assert_eq!(sample.players.people + sample.players.citizens, sample.load.players_in_world);
    assert_eq!(sample.watches, [Watch::of(WatchKind::Gini)]);
    let town = sample.town.as_ref().unwrap();
    assert_eq!(town.decisions(), 45);
    assert_eq!(town.standing["working"] + town.standing["resting"], sample.players.citizens);
    // People's and citizens' Strands, by trade, without the admins'
    let spread = sample.distribution.as_ref().unwrap();
    assert_eq!(spread.roles.values().map(|h| h.strands).sum::<i64>(), sample.strands.citizens + sample.strands.people);
    assert_eq!(spread.roles.values().map(|h| h.accounts).sum::<u64>(), spread.accounts);
    assert_eq!(spread.roles[ROLE_PERSON].accounts, sample.players.people);
    assert_eq!(sample.grants.returned, 1);
    assert_eq!(serde_json::to_string(&sample).unwrap(), json);
}

#[test]
fn a_sample_from_before_the_town_reported_still_reads() {
    let json = fixture("sample.json");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value.as_object_mut().unwrap().remove("town");
    let old: Sample = serde_json::from_value(value).unwrap();
    assert!(old.town.is_none());
    assert!(!serde_json::to_string(&old).unwrap().contains("\"town\""));
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value.as_object_mut().unwrap().remove("distribution");
    let old: Sample = serde_json::from_value(value).unwrap();
    assert!(old.distribution.is_none());
}

#[test]
fn a_town_report_round_trips_with_its_decisions() {
    let json = fixture("town_report.json");
    let report: TownReport = serde_json::from_str(&json).unwrap();
    assert_eq!(report.citizens.len(), 3);
    assert_eq!(report.citizens[0].decisions[1].trail[1].layer, "jev");
    assert!(report.citizens[2].decisions.is_empty(), "nothing new from one away");
    assert_eq!(report.layers["jev"].mean_ms(), Some(212.4));
    assert_eq!(report.standing()["away"], 1);
    assert!(!report.stopping);
    assert_eq!(serde_json::to_string(&report).unwrap(), json);
}

#[test]
fn the_admin_feed_round_trips_one_of_each() {
    let json = fixture("admin_feed.json");
    let feed: Vec<Feed> = serde_json::from_str(&json).unwrap();
    let types: Vec<&str> = feed.iter().map(Feed::type_name).collect();
    assert_eq!(types, ["annotation", "fill", "deed", "deed", "gazette", "warning", "lagged"]);
    assert_eq!(serde_json::to_string(&feed).unwrap(), json);
    // A sample goes out as itself, tagged
    let sample = fixture("sample.json");
    let tagged = serde_json::to_string(&Feed::Sample(Box::new(serde_json::from_str(&sample).unwrap()))).unwrap();
    assert_eq!(tagged, format!("{{\"type\":\"sample\",{}", &sample[1..]));
}

#[test]
fn walkers_round_trip() {
    use capitol_protocol::rest::{Doing, Walker};
    let json = fixture("admin_walkers.json");
    let walkers: Vec<Walker> = serde_json::from_str(&json).unwrap();
    assert_eq!(walkers.len(), 3);
    assert!(walkers[0].citizen && walkers[0].dest.is_some());
    assert!(matches!(walkers[1].doing, Doing::Extracting { .. }));
    assert_eq!(walkers[2].doing, Doing::Crafting { recipe_id: "rope".into() });
    assert_eq!(serde_json::to_string(&walkers).unwrap(), json);
}

#[test]
fn the_admin_map_round_trips() {
    use capitol_protocol::rest::AdminMap;
    let json = fixture("admin_map.json");
    let map: AdminMap = serde_json::from_str(&json).unwrap();
    assert!(map.plots[0].citizen && !map.plots[1].citizen);
    assert!(map.stations[0].citizen && !map.stations[1].citizen);
    assert_eq!(map.heat.market.vwap(), Some(412.0 / 96.0));
    assert_eq!(map.heat.stalls["bazaar_stall_12"].strands, 60);
    assert_eq!(serde_json::to_string(&map).unwrap(), json);
}

#[test]
fn a_players_detail_round_trips() {
    use capitol_protocol::rest::PlayerDetail;
    let json = fixture("admin_player.json");
    let player: PlayerDetail = serde_json::from_str(&json).unwrap();
    assert!(player.citizen && player.walker.is_some());
    // What they opened with and the range's flows add up to what they hold
    assert_eq!(player.opening + player.flows.values().sum::<i64>(), player.strands);
    assert_eq!(player.balance.last().unwrap().strands, player.strands);
    assert_eq!(player.recent[0].strands, player.strands);
    assert_eq!(player.recent[0].description.as_deref(), Some("Fee on 9 logs"));
    assert_eq!(player.recent[1].description, None);
    assert_eq!(serde_json::to_string(&player).unwrap(), json);
}

#[test]
fn the_admin_books_are_this_builds() {
    use capitol_protocol::rest::AdminBooks;
    let json = fixture("admin_books.json");
    let books: AdminBooks = serde_json::from_str(&json).unwrap();
    assert_eq!(books, AdminBooks::current());
    assert_eq!(serde_json::to_string(&books).unwrap(), json);
}

#[test]
fn a_sample_from_before_new_accounts_reads_as_none() {
    let json = fixture("sample.json");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value.as_object_mut().unwrap().remove("new_accounts");
    let old: Sample = serde_json::from_value(value).unwrap();
    assert_eq!(old.new_accounts, 0);
}

#[test]
fn a_sample_stored_before_the_lag_counts_still_reads() {
    let json = fixture("sample.json");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["load"].as_object_mut().unwrap().remove("events_lagged");
    value.as_object_mut().unwrap().remove("watches");
    let old: Sample = serde_json::from_value(value).unwrap();
    assert!(old.load.events_lagged.is_empty());
    assert!(old.watches.is_empty(), "a sample from before the watches");
}

#[test]
fn a_card_round_trips_leaving_out_what_isnt_so() {
    use capitol_protocol::rest::PlayerCard;
    use capitol_protocol::rules::{standing, Zone};
    let json = fixture("player_card.json");
    let card: PlayerCard = serde_json::from_str(&json).unwrap();
    assert!(card.citizen && !card.mine);
    assert_eq!(card.home.as_deref().and_then(Zone::from_str), Some(Zone::Rural));
    assert_eq!((card.standing.as_str(), card.guild.as_deref()), (standing::NEW_IN_TOWN, None));
    assert_eq!(serde_json::to_string(&card).unwrap(), json);
}

#[test]
fn your_own_card_round_trips_with_the_number_under_its_words() {
    use capitol_protocol::rest::PlayerCard;
    use capitol_protocol::rules::standing::{self, Deed};
    let json = fixture("my_card.json");
    let card: PlayerCard = serde_json::from_str(&json).unwrap();
    let own = card.own.as_ref().expect("your own card has your number");
    assert!(card.mine && own.words == card.standing && card.standing.ends_with(standing::LET_DOWN_LATELY));
    assert_eq!((own.score, own.trend, own.records, own.history.len()), (8.25, -1.5, 9, 5));
    assert_eq!(own.history[0].deed, Deed::Lapsed { units: 2 });
    assert_eq!(own.history[2].deed, Deed::TaxWeek);
    assert_eq!(standing::line(&own.history[3], "Log"), "Kept a claim: 10 log for Citizen Cy");
    assert_eq!(serde_json::to_string(&card).unwrap(), json);
}

#[test]
fn mutes_round_trip_marking_only_the_citizen() {
    use capitol_protocol::rest::MutesResponse;
    let json = fixture("mutes.json");
    let mutes: MutesResponse = serde_json::from_str(&json).unwrap();
    let citizens: Vec<bool> = mutes.mutes.iter().map(|m| m.citizen).collect();
    assert_eq!(citizens, [true, false]);
    assert_eq!(serde_json::to_string(&mutes).unwrap(), json);
}

#[test]
fn a_mailbox_round_trips_with_its_unread_and_paragraphs() {
    use capitol_protocol::rest::{Mailbox, SendLetter};
    use capitol_protocol::rules::talk;
    let json = fixture("mailbox.json");
    let mailbox: Mailbox = serde_json::from_str(&json).unwrap();
    assert_eq!((mailbox.unread, mailbox.more), (1, true));
    let [first, second] = &mailbox.letters[..] else { panic!("two letters") };
    assert!(first.from.citizen && !first.to.citizen && first.read_at.is_none());
    assert!(!second.from.citizen && second.read_at.is_some());
    assert_eq!(talk::first_line(&first.body), "Thank you for the rope, it held.");
    assert_eq!(serde_json::to_string(&mailbox).unwrap(), json);

    let json = fixture("send_letter.json");
    let letter: SendLetter = serde_json::from_str(&json).unwrap();
    assert_eq!(talk::clean_letter(&letter.body).unwrap(), letter.body, "already clean");
    assert_eq!(serde_json::to_string(&letter).unwrap(), json);
}

#[test]
fn commissions_round_trip_marking_only_the_citizen() {
    use capitol_protocol::rest::{CommissionDelivery, CommissionsResponse, DeliverCommission, PostCommission};
    use capitol_protocol::rules::court::{commission_value, CommissionStatus};
    use capitol_protocol::rules::stacking::Units;

    let json = fixture("post_commission.json");
    let post: PostCommission = serde_json::from_str(&json).unwrap();
    assert_eq!((post.min_quality, post.due_days), (55, 4));
    assert_eq!(serde_json::to_string(&post).unwrap(), json);
    // Without a quality, any
    let any: PostCommission = serde_json::from_str(r#"{"item_type":"plank","quantity":5,"price":30,"due_days":1}"#).unwrap();
    assert_eq!(any.min_quality, 0);

    let json = fixture("deliver_commission.json");
    let deliver: DeliverCommission = serde_json::from_str(&json).unwrap();
    assert_eq!((deliver.qualities.as_ref().map(Units::count), deliver.quantity), (Some(8), None));
    assert_eq!(serde_json::to_string(&deliver).unwrap(), json);

    let json = fixture("commissions.json");
    let board: CommissionsResponse = serde_json::from_str(&json).unwrap();
    let citizens: Vec<bool> = board.commissions.iter().map(|c| c.poster_citizen).collect();
    assert_eq!(citizens, [true, false]);
    // What's said of the poster is on the list, and left out when it isn't known (S3.1)
    let words: Vec<&str> = board.commissions.iter().map(|c| c.poster_standing.as_str()).collect();
    assert_eq!(words, ["dependable", ""]);
    for c in &board.commissions {
        assert_eq!(c.status, CommissionStatus::Open);
        assert_eq!(c.escrow_strands, commission_value(c.price, c.quantity - c.delivered));
    }
    assert_eq!(serde_json::to_string(&board).unwrap(), json);

    let json = fixture("commission_delivery.json");
    let done: CommissionDelivery = serde_json::from_str(&json).unwrap();
    assert_eq!((done.commission.status, done.commission.escrow_strands), (CommissionStatus::Filled, 0));
    assert_eq!(done.paid, commission_value(done.commission.price, done.units.count() as i32));
    assert_eq!(serde_json::to_string(&done).unwrap(), json);
}

#[test]
fn claims_round_trip_marking_only_the_citizen() {
    use capitol_protocol::rest::{ClaimCommission, ClaimResponse, ClaimsResponse};
    use capitol_protocol::rules::court::{claim_until, unclaimed, ClaimStatus};

    let json = fixture("claim_commission.json");
    let claim: ClaimCommission = serde_json::from_str(&json).unwrap();
    assert_eq!(claim.quantity, 4);
    assert_eq!(serde_json::to_string(&claim).unwrap(), json);

    let json = fixture("claim_response.json");
    let made: ClaimResponse = serde_json::from_str(&json).unwrap();
    let c = &made.commission;
    assert_eq!((made.claim.status, made.claim.quantity, c.claimed, c.withdrawn), (ClaimStatus::Open, 4, 4, false));
    assert_eq!(made.claim.until, claim_until(made.claim.claimed_at, c.due_at));
    assert_eq!(unclaimed(c.quantity - c.delivered, c.claimed, c.withdrawn), 8);
    assert_eq!(serde_json::to_string(&made).unwrap(), json);

    let json = fixture("claims.json");
    let claims: ClaimsResponse = serde_json::from_str(&json).unwrap();
    let marks: Vec<(bool, ClaimStatus)> = claims.claims.iter().map(|k| (k.claimant_citizen, k.status)).collect();
    assert_eq!(marks, [(true, ClaimStatus::Open), (false, ClaimStatus::Kept)]);
    assert_eq!(claims.claims[0].claimant_standing, "well regarded");
    assert_eq!(serde_json::to_string(&claims).unwrap(), json);
}

#[test]
fn a_guild_round_trips_as_its_leader_sees_it() {
    use capitol_protocol::rest::{FoundGuild, GuildBooks, GuildEntryKind, GuildInviteKind, GuildsResponse, MyGuild};
    use capitol_protocol::rules::guild::{self, GuildRole};
    let json = fixture("my_guild.json");
    let mine: MyGuild = serde_json::from_str(&json).unwrap();
    assert_eq!(mine.role, Some(GuildRole::Leader));
    assert!(mine.invites.is_empty());
    let guild = mine.guild.as_ref().unwrap();
    let roles: Vec<GuildRole> = guild.members.iter().map(|m| m.role).collect();
    assert_eq!(roles, [GuildRole::Leader, GuildRole::Officer, GuildRole::Member], "by role, then tenure");
    assert!(guild.members[2].citizen && !guild.members[0].citizen);
    assert_eq!(guild.leader, guild.founder);
    let kinds: Vec<GuildInviteKind> = guild.waiting.iter().map(|w| w.kind).collect();
    assert_eq!(kinds, [GuildInviteKind::Apply, GuildInviteKind::Invite]);
    assert_eq!(guild.waiting[0].by, guild.waiting[0].player, "an application is its own asking");
    let treasury = guild.treasury.as_ref().expect("a member sees the treasury");
    assert_eq!(treasury.weekly_dues, guild::weekly_dues(guild.members.len() as u32));
    assert_eq!(treasury.dissolves_at, guild::dissolves_at(guild.members.len() as u32));
    let hall = guild.hall.as_ref().expect("its hall (S4.3)");
    assert_eq!((hall.terrain_id.as_deref(), hall.tax_owed), (Some("guild_hall_3"), 12));
    assert_eq!(serde_json::to_string(&mine).unwrap(), json);

    let json = fixture("guild_books.json");
    let books: GuildBooks = serde_json::from_str(&json).unwrap();
    assert!(books.entries.windows(2).all(|w| w[0].at >= w[1].at), "newest first");
    // Each entry's treasury is the one before it moved by its Strands
    assert!(books.entries.windows(2).all(|w| w[1].treasury + w[0].strands == w[0].treasury));
    let kinds: Vec<GuildEntryKind> = books.entries.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [GuildEntryKind::Tax, GuildEntryKind::Dues, GuildEntryKind::Withdrawal, GuildEntryKind::Fee, GuildEntryKind::Deposit]
    );
    assert!(books.entries[0].by.is_none() && books.entries[1].by.is_none(), "the hall's tax and dues are no one's");
    for kind in kinds {
        assert_eq!(GuildEntryKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(serde_json::to_string(&books).unwrap(), json);

    let json = fixture("guilds.json");
    let guilds: GuildsResponse = serde_json::from_str(&json).unwrap();
    assert!(guilds.guilds.windows(2).all(|w| w[0].members >= w[1].members), "the largest first");
    assert_eq!(serde_json::to_string(&guilds).unwrap(), json);

    let json = fixture("found_guild.json");
    let found: FoundGuild = serde_json::from_str(&json).unwrap();
    assert_eq!(guild::clean_name(&found.name).unwrap(), found.name, "already clean");
    assert_eq!(guild::clean_tag(&found.tag).unwrap(), found.tag);
    assert_eq!(guild::clean_charter(&found.charter).unwrap(), found.charter);
    assert_eq!(serde_json::to_string(&found).unwrap(), json);
}

#[test]
fn wages_round_trip_with_their_work_and_shift() {
    use capitol_protocol::rest::{OfferWages, ShiftEnded, WagesResponse};
    use capitol_protocol::rules::court::court_fee;
    use capitol_protocol::rules::wages::{offer_value, OfferStatus, ShiftEnd, Work};
    let json = fixture("wages.json");
    let wages: WagesResponse = serde_json::from_str(&json).unwrap();
    let [field, forge] = &wages.offers[..] else { panic!("two offers") };
    assert!(matches!(field.work, Work::Gather { .. }) && matches!(forge.work, Work::Craft { .. }));
    let shift = field.shift.as_ref().expect("a shift on it");
    assert!(shift.worker_citizen && shift.ended_at.is_none() && Some(shift) == wages.shift.as_ref());
    assert_eq!((field.employer_standing.as_str(), forge.employer_standing.as_str()), ("a pillar of the Capitol", ""));
    assert_eq!(field.escrow_strands, offer_value(field.rate, field.cap - field.made));
    assert_eq!((forge.status, forge.fee), (OfferStatus::Open, court_fee(offer_value(forge.rate, forge.cap))));
    assert_eq!(serde_json::to_string(&wages).unwrap(), json);

    let json = fixture("offer_wages.json");
    let offer: OfferWages = serde_json::from_str(&json).unwrap();
    assert_eq!(offer.worker_id, None, "open to anyone");
    assert_eq!(serde_json::to_string(&offer).unwrap(), json);

    let json = fixture("shift_ended.json");
    let ended: ShiftEnded = serde_json::from_str(&json).unwrap();
    assert_eq!(ended.shift.end, Some(ShiftEnd::Employer));
    assert_eq!(ended.paid, 48);
    assert_eq!(serde_json::to_string(&ended).unwrap(), json);
}

#[test]
fn the_road_in_round_trips_in_the_tours_order() {
    use capitol_protocol::tutorial::{Chapter, Step, TutorialProgress};
    let json = fixture("tutorial.json");
    let progress: TutorialProgress = serde_json::from_str(&json).unwrap();
    assert!(progress.done.chapter_done(Chapter::Arrival));
    assert!(progress.done.has(Step::Deposit), "done out of order still counts");
    assert_eq!(progress.done.current(), Some(Step::SeeRuralLot));
    assert!(!progress.dismissed && !progress.finished());
    assert_eq!(serde_json::to_string(&progress).unwrap(), json);
}

#[test]
fn the_deed_waits_at_the_office_and_its_free_lots_round_trip() {
    use capitol_protocol::rest::FreeLotsResponse;
    use capitol_protocol::tutorial::{Grant, Step, TutorialProgress};
    let json = fixture("tutorial_granted.json");
    let progress: TutorialProgress = serde_json::from_str(&json).unwrap();
    assert!(progress.done.deed_earned());
    assert_eq!(progress.grant, Grant::Open);
    assert_eq!(progress.done.current(), Some(Step::ClaimHome));
    assert_eq!(serde_json::to_string(&progress).unwrap(), json);

    let json = fixture("free_lots.json");
    let lots: FreeLotsResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(lots.zone_id, "rural");
    assert!(lots.lots.windows(2).all(|w| w[0].walk_secs <= w[1].walk_secs), "nearest the hall first");
    assert_eq!(serde_json::to_string(&lots).unwrap(), json);
}
