//! Citizens who have left the roster (#585).
//!
//! A citizen gives up a stall it has stopped using as it plays (#523), but
//! one dropped from `town.toml` never signs in again, so nothing ran for it:
//! its stall held until the rent lapsed, and its open orders, commissions
//! and wage offers kept their escrow until they expired. The roster is
//! never rewritten, and a name that's gone from it takes its key with it,
//! so the town keeps every key it has run beside the roster, in
//! `town.known.json` ([`known_path`]).
//!
//! As a town starts ([`settle_departed`]), each known citizen that's no
//! longer on the roster signs in once, by REST alone, and leaves the city
//! tidy ([`settle`]): its stall closed (its goods to its market box), its
//! orders cancelled, its commissions taken down, its wage offers withdrawn
//! and its claims on others' commissions given up. Then its key is
//! forgotten. One the server can't be reached for is kept, and tried again
//! at the next start; one whose key is refused is forgotten, since there's
//! nothing it could take down.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use capitol_protocol::rest::{CommissionsQuery, MyOrdersQuery, WagesQuery};
use capitol_protocol::rules::wages::OfferStatus;

use super::town::{self, Roster};
use crate::net::{self, NetConfig, RestError, RestRequest, RestResponse};

/// Where the town keeps the keys of every citizen it has run: beside the
/// roster, `town.known.json`, a key a citizen by name. It holds keys, so it's
/// git-ignored as the roster is.
pub fn known_path(roster: &Path) -> PathBuf {
    roster.with_extension("known.json")
}

/// The keys kept at `path`, by name. A file that's missing or won't read is
/// none: the departed it named lapse by the bazaar's sweep, as before.
pub fn read_known(path: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("capitol-town: {} doesn't read, so no departed citizen is settled: {e}", path.display());
        BTreeMap::new()
    })
}

/// Writes `known` to `path`, by way of a file beside it, so a town stopped
/// mid-write never tears it.
pub fn write_known(path: &Path, known: &BTreeMap<String, String>) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(known).map_err(std::io::Error::other)?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

/// `known` with every citizen on the roster in it, by its key as it reads
/// now. One whose key file won't read keeps the key it was known by.
pub fn with_roster(mut known: BTreeMap<String, String>, roster: &Roster) -> BTreeMap<String, String> {
    for citizen in &roster.citizens {
        if let Ok(key) = citizen.key() {
            known.retain(|name, _| !name.eq_ignore_ascii_case(&citizen.name));
            known.insert(citizen.name.clone(), key);
        }
    }
    known
}

/// Writes the roster's citizens into the known keys beside it: after a mint,
/// so a citizen dropped before the town ever ran is known too.
pub fn remember(roster: &Roster) -> std::io::Result<()> {
    let path = known_path(&roster.path);
    write_known(&path, &with_roster(read_known(&path), roster))
}

/// The known citizens no longer on the roster, by name, with their keys.
/// Names compare as the roster's do, without case.
pub fn departed<'a>(roster: &Roster, known: &'a BTreeMap<String, String>) -> Vec<(&'a str, &'a str)> {
    known
        .iter()
        .filter(|(name, _)| !roster.citizens.iter().any(|c| c.name.eq_ignore_ascii_case(name)))
        .map(|(name, key)| (name.as_str(), key.as_str()))
        .collect()
}

/// What a departed citizen left behind and took down.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settled {
    /// The stall it gave up, by its shop's name, and the stacks that went
    /// to its market box
    pub stall: Option<(String, u32)>,
    pub orders: usize,
    pub commissions: usize,
    pub wages: usize,
    pub claims: usize,
    /// What the server wouldn't take down, said
    pub refused: Vec<String>,
}

impl Settled {
    /// One line for the town's log.
    pub fn line(&self, name: &str) -> String {
        let mut parts = Vec::new();
        if let Some((shop, stacks)) = &self.stall {
            parts.push(format!("closed {shop} ({stacks} stacks to its market box)"));
        }
        let mut count = |n: usize, one: &str, many: &str| {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        };
        count(self.orders, "order cancelled", "orders cancelled");
        count(self.commissions, "commission taken down", "commissions taken down");
        count(self.wages, "wage offer withdrawn", "wage offers withdrawn");
        count(self.claims, "claim given up", "claims given up");
        let mut line = if parts.is_empty() {
            format!("{name} has left the roster, with nothing to take down")
        } else {
            format!("{name} has left the roster: {}", parts.join(", "))
        };
        if !self.refused.is_empty() {
            line.push_str(&format!("; refused: {}", self.refused.join("; ")));
        }
        line
    }
}

/// Why a departed citizen couldn't be settled.
#[derive(Debug, Clone, PartialEq)]
pub enum Unsettled {
    /// The server refused its key: nothing of its can be taken down, so it's
    /// forgotten.
    KeyRefused(String),
    /// The server couldn't be reached, or failed: it's kept, for the next
    /// start.
    Later(String),
}

/// Whether a failed call means the citizen should be tried again later: no
/// answer, the server's own failure, or a 403, a key the server won't yet let
/// do it (as `/bazaar/mine/close` was, before #585). Any other refusal (4xx)
/// is the server's final word on that one thing.
fn later(e: &RestError) -> bool {
    match e {
        RestError::Http { status, .. } => *status >= 500 || *status == 403,
        RestError::Transport(_) | RestError::Decode(_) => true,
    }
}

/// Signs a departed citizen in by `call` (REST with its key) once, and takes
/// down all it left: its stall, its open orders, its commissions, its wage
/// offers (those it offered, by its `name`; the board also lists those it's
/// named on) and its claims. A refusal of one thing is said and the rest go
/// on.
pub fn settle(
    name: &str,
    mut call: impl FnMut(&RestRequest) -> Result<RestResponse, RestError>,
) -> Result<Settled, Unsettled> {
    let mut settled = Settled::default();
    // The first call says whether the key still works
    let stall = match call(&RestRequest::MyStall) {
        Ok(RestResponse::MyStall(mine)) => mine.shop,
        Ok(other) => return Err(Unsettled::Later(format!("reading its stall: unexpected {other:?}"))),
        Err(e) if e.status() == Some(401) || e.status() == Some(403) => return Err(Unsettled::KeyRefused(e.to_string())),
        Err(e) => return Err(Unsettled::Later(format!("reading its stall: {e}"))),
    };
    // Each later failure that isn't a refusal stops the pass, to try again
    let mut take = |what: &str, request: RestRequest, settled: &mut Settled| -> Result<Option<RestResponse>, Unsettled> {
        match call(&request) {
            Ok(answer) => Ok(Some(answer)),
            Err(e) if later(&e) => Err(Unsettled::Later(format!("{what}: {e}"))),
            Err(e) => {
                settled.refused.push(format!("{what}: {e}"));
                Ok(None)
            }
        }
    };
    if stall.is_some() {
        if let Some(RestResponse::ClosedStall(closed)) = take("closing its stall", RestRequest::CloseStall, &mut settled)? {
            settled.stall = Some((closed.name, closed.returned + closed.waiting));
        }
    }
    if let Some(RestResponse::MyOrders(orders)) =
        take("reading its orders", RestRequest::MyOrders(MyOrdersQuery::default()), &mut settled)?
    {
        for order in orders {
            let what = format!("cancelling its {} order", order.item_type);
            if take(&what, RestRequest::CancelOrder { order_id: order.id }, &mut settled)?.is_some() {
                settled.orders += 1;
            }
        }
    }
    let mine = CommissionsQuery {
        mine: true,
        ..CommissionsQuery::default()
    };
    if let Some(RestResponse::Commissions(board)) = take("reading its commissions", RestRequest::Commissions(mine), &mut settled)? {
        // One withdrawn while claims stand is already down to what they owe
        for commission in board.commissions.into_iter().filter(|c| !c.withdrawn) {
            let what = format!("taking down its {} commission", commission.item_type);
            let request = RestRequest::CancelCommission {
                commission_id: commission.id,
            };
            if take(&what, request, &mut settled)?.is_some() {
                settled.commissions += 1;
            }
        }
    }
    let mine = WagesQuery {
        mine: true,
        ..WagesQuery::default()
    };
    if let Some(RestResponse::Wages(wages)) = take("reading its wage offers", RestRequest::Wages(mine), &mut settled)? {
        let offered = wages
            .offers
            .into_iter()
            .filter(|o| o.status == OfferStatus::Open && o.employer_name.eq_ignore_ascii_case(name));
        for offer in offered {
            if take("withdrawing a wage offer", RestRequest::WithdrawWages { offer_id: offer.id }, &mut settled)?.is_some() {
                settled.wages += 1;
            }
        }
    }
    if let Some(RestResponse::Claims(claims)) = take("reading its claims", RestRequest::MyClaims, &mut settled)? {
        for claim in claims.claims {
            let request = RestRequest::ReleaseClaim {
                commission_id: claim.commission_id,
            };
            if take("giving up a claim", request, &mut settled)?.is_some() {
                settled.claims += 1;
            }
        }
    }
    Ok(settled)
}

/// As a town starts: settles every known citizen that's left the roster
/// against `server`, then keeps the roster's keys and those it couldn't
/// reach, and drops the stints of those settled. Returns a line each, for
/// the town's log. A known file that won't write is said, and the departed
/// are settled again next time, which finds nothing left to take down.
pub fn settle_departed(roster: &Roster, server: &str) -> Vec<String> {
    let path = known_path(&roster.path);
    let known = read_known(&path);
    let config = NetConfig::with_base_url(server);
    let mut lines = Vec::new();
    let mut gone = Vec::new();
    let mut kept = BTreeMap::new();
    for (name, key) in departed(roster, &known) {
        match settle(name, |request| net::call_once(&config, Some(key), request)) {
            Ok(settled) => {
                lines.push(settled.line(name));
                gone.push(name.to_string());
            }
            Err(Unsettled::KeyRefused(why)) => {
                lines.push(format!("{name} has left the roster, and its key is refused ({why}): forgotten"));
                gone.push(name.to_string());
            }
            Err(Unsettled::Later(why)) => {
                lines.push(format!("{name} has left the roster, but couldn't be settled ({why}): trying again next start"));
                kept.insert(name.to_string(), key.to_string());
            }
        }
    }
    let known = with_roster(kept, roster);
    if let Err(e) = write_known(&path, &known) {
        lines.push(format!("writing {}: {e}", path.display()));
    }
    if !gone.is_empty() {
        let stints_path = town::stints_path(&roster.path);
        let mut stints = town::read_stints(&stints_path);
        let had = stints.len();
        stints.retain(|name, _| !gone.iter().any(|g| g.eq_ignore_ascii_case(name)));
        if stints.len() != had {
            if let Err(e) = town::write_stints(&stints_path, &stints) {
                lines.push(format!("writing {}: {e}", stints_path.display()));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::{
        BazaarShop, ClaimsResponse, ClosedStall, CommissionsResponse, MarketOrder, MyStallResponse, WagesResponse,
    };
    use capitol_protocol::rules::market::{OrderSide, OrderStatus};
    use capitol_protocol::rules::stacking::Units;
    use chrono::DateTime;
    use uuid::Uuid;

    const FIXTURES: &str = "../../../protocol/tests/fixtures/rest";

    fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES).join(name);
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn order(n: u128) -> MarketOrder {
        let at = DateTime::from_timestamp(0, 0).unwrap();
        MarketOrder {
            id: Uuid::from_u128(n),
            player_id: Uuid::nil(),
            side: OrderSide::Buy,
            item_type: "plank".into(),
            price: 12,
            quantity_total: 10,
            quantity_remaining: 10,
            status: OrderStatus::Open,
            min_grade: None,
            grade_rule: None,
            durability: None,
            max_durability: None,
            min_durability: None,
            units: Units::default(),
            escrow_strands: 120,
            fee: 1,
            created_at: at,
            expires_at: at,
        }
    }

    /// A server that answers as the fixtures do, for a citizen named
    /// "Ada Reeder" who keeps a stall, two orders, the board's commissions,
    /// the Work page's offers and two claims; and records what it was asked.
    struct Server {
        asked: Vec<String>,
        shop: Option<BazaarShop>,
        refuse: Option<&'static str>,
        forbid: Option<&'static str>,
        fail: Option<&'static str>,
    }

    impl Server {
        fn new() -> Server {
            Server {
                asked: Vec::new(),
                shop: Some(fixture("bazaar_shop.json")),
                refuse: None,
                forbid: None,
                fail: None,
            }
        }

        fn call(&mut self, request: &RestRequest) -> Result<RestResponse, RestError> {
            let what = format!("{request:?}");
            let kind = what.split([' ', '(', '{']).next().unwrap().to_string();
            self.asked.push(kind.clone());
            if self.refuse == Some(kind.as_str()) {
                return Err(RestError::Http {
                    status: if kind == "MyStall" { 401 } else { 400 },
                    message: "no".into(),
                });
            }
            if self.forbid == Some(kind.as_str()) {
                return Err(RestError::Http {
                    status: 403,
                    message: "An API key can't".into(),
                });
            }
            if self.fail == Some(kind.as_str()) {
                return Err(RestError::Transport("refused".into()));
            }
            Ok(match request {
                RestRequest::MyStall => RestResponse::MyStall(MyStallResponse {
                    shop: self.shop.clone(),
                    returning: Vec::new(),
                }),
                RestRequest::CloseStall => RestResponse::ClosedStall(ClosedStall {
                    stall_id: "bazaar_stall_2".into(),
                    name: "Ada's Ropes".into(),
                    returned: 2,
                    waiting: 1,
                }),
                RestRequest::MyOrders(_) => RestResponse::MyOrders(vec![order(1), order(2)]),
                RestRequest::CancelOrder { order_id } => RestResponse::Order(MarketOrder {
                    status: OrderStatus::Cancelled,
                    ..order(order_id.as_u128())
                }),
                RestRequest::Commissions(_) => RestResponse::Commissions(fixture::<CommissionsResponse>("commissions.json")),
                RestRequest::CancelCommission { .. } => {
                    RestResponse::Commission(fixture::<CommissionsResponse>("commissions.json").commissions.remove(0))
                }
                RestRequest::Wages(_) => RestResponse::Wages(fixture::<WagesResponse>("wages.json")),
                RestRequest::WithdrawWages { .. } => {
                    RestResponse::WageOffer(fixture::<WagesResponse>("wages.json").offers.remove(0))
                }
                RestRequest::MyClaims => RestResponse::Claims(fixture::<ClaimsResponse>("claims.json")),
                RestRequest::ReleaseClaim { .. } => RestResponse::Claim(fixture("claim_response.json")),
                other => panic!("not asked of a departed citizen: {other:?}"),
            })
        }
    }

    fn count(asked: &[String], kind: &str) -> usize {
        asked.iter().filter(|a| *a == kind).count()
    }

    #[test]
    fn a_departed_citizen_closes_its_stall_and_takes_its_escrow_down() {
        let mut server = Server::new();
        let settled = settle("Ada Reeder", |r| server.call(r)).unwrap();
        let commissions = fixture::<CommissionsResponse>("commissions.json").commissions.len();
        let offered = fixture::<WagesResponse>("wages.json")
            .offers
            .iter()
            .filter(|o| o.status == OfferStatus::Open && o.employer_name == "Ada Reeder")
            .count();
        let claims = fixture::<ClaimsResponse>("claims.json").claims.len();
        assert!(offered > 0, "the fixture has an offer of Ada's");
        assert_eq!(settled.stall, Some(("Ada's Ropes".to_string(), 3)));
        assert_eq!(
            (settled.orders, settled.commissions, settled.wages, settled.claims),
            (2, commissions, offered, claims)
        );
        assert!(settled.refused.is_empty());
        assert_eq!(count(&server.asked, "CloseStall"), 1);
        assert_eq!(count(&server.asked, "WithdrawWages"), offered, "only offers Ada made, not those it's named on");
        let line = settled.line("Ada Reeder");
        assert!(line.starts_with("Ada Reeder has left the roster: closed Ada's Ropes (3 stacks"), "{line}");
        assert!(line.contains("2 orders cancelled"), "{line}");
    }

    #[test]
    fn one_with_no_stall_asks_nothing_be_closed() {
        let mut server = Server::new();
        server.shop = None;
        let settled = settle("Ada Reeder", |r| server.call(r)).unwrap();
        assert_eq!(settled.stall, None);
        assert_eq!(count(&server.asked, "CloseStall"), 0);
        assert_eq!(settled.orders, 2, "the rest is taken down all the same");
    }

    #[test]
    fn a_refused_key_is_forgotten_and_a_server_that_isnt_up_is_tried_again() {
        let mut server = Server::new();
        server.refuse = Some("MyStall");
        assert!(matches!(settle("Ada Reeder", |r| server.call(r)), Err(Unsettled::KeyRefused(_))));
        assert_eq!(server.asked, ["MyStall"]);

        let mut server = Server::new();
        server.fail = Some("MyStall");
        assert!(matches!(settle("Ada Reeder", |r| server.call(r)), Err(Unsettled::Later(_))));

        // Mid-pass too: what's left is tried again next start
        let mut server = Server::new();
        server.fail = Some("Wages");
        assert!(matches!(settle("Ada Reeder", |r| server.call(r)), Err(Unsettled::Later(_))));

        // And a key the server won't let close a stall yet
        let mut server = Server::new();
        server.forbid = Some("CloseStall");
        assert!(matches!(settle("Ada Reeder", |r| server.call(r)), Err(Unsettled::Later(_))));
    }

    #[test]
    fn a_refusal_of_one_thing_is_said_and_the_rest_go_on() {
        let mut server = Server::new();
        server.refuse = Some("CancelOrder");
        let settled = settle("Ada Reeder", |r| server.call(r)).unwrap();
        assert_eq!(settled.orders, 0);
        assert_eq!(settled.refused.len(), 2, "{:?}", settled.refused);
        assert!(settled.claims > 0);
        assert!(settled.line("Ada Reeder").contains("; refused: cancelling its plank order: no (HTTP 400)"));
    }

    fn roster(names: &[&str]) -> Roster {
        let mut text = String::new();
        for name in names {
            text.push_str(&format!("[[citizen]]\nname = \"{name}\"\nrole = \"gatherer-fiber\"\nkey = \"key-{name}\"\n"));
        }
        Roster::parse(&text, Path::new("town.toml")).unwrap()
    }

    #[test]
    fn the_departed_are_the_known_no_longer_on_the_roster() {
        let known = BTreeMap::from([
            ("Ada Reeder".to_string(), "a".to_string()),
            ("bo tanner".to_string(), "b".to_string()),
            ("Cy Smith".to_string(), "c".to_string()),
        ]);
        let roster = roster(&["Bo Tanner", "Di Wright"]);
        assert_eq!(departed(&roster, &known), [("Ada Reeder", "a"), ("Cy Smith", "c")]);
        // The roster's keys replace what they were known by, whatever the case
        let now = with_roster(known, &roster);
        assert_eq!(now.get("Bo Tanner").map(String::as_str), Some("key-Bo Tanner"));
        assert!(!now.contains_key("bo tanner"));
        assert_eq!(now.get("Di Wright").map(String::as_str), Some("key-Di Wright"));
        assert_eq!(now.len(), 4);
    }

    /// Live (#585): a citizen rents a stall, bids at the Exchange and posts a
    /// commission, then leaves the roster; the town's start settles it.
    /// Needs a server (`CAPITOL_SERVER`) and two citizens' keys and names
    /// from `capitol-town mint --count 2`: `STAYS_KEY`, `LEAVES_KEY` and
    /// `LEAVES_NAME`. About a minute at `SIM_TICK_SCALE=10`.
    #[test]
    #[ignore = "needs a live server and two citizens"]
    fn a_citizen_dropped_from_the_roster_gives_its_stall_back() {
        use crate::bot::Bot;
        use capitol_protocol::rest::{PlaceOrder, PostCommission};
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let server = std::env::var("CAPITOL_SERVER").unwrap_or_else(|_| "http://localhost:3000".into());
        let (leaves, key) = (env("LEAVES_NAME"), env("LEAVES_KEY"));
        {
            let mut bot = Bot::with_api_key(NetConfig::with_base_url(&server), &key).unwrap();
            let directory = bot.bazaar().unwrap();
            let free = directory.stalls.iter().find(|s| s.shop.is_none()).expect("a free stall").stall_id.clone();
            bot.rent_stall(&free, 1, Some("Leaving Soon")).unwrap();
            bot.go_to_exchange().unwrap();
            bot.place_order(PlaceOrder {
                side: OrderSide::Buy,
                item_type: "plank".into(),
                price: 10,
                quantity: 4,
                item_id: None,
                qualities: None,
                min_grade: None,
                grade_rule: None,
                min_durability: None,
            })
            .unwrap();
            bot.post_commission(PostCommission {
                item_type: "fiber".into(),
                min_quality: 0,
                quantity: 5,
                price: 2,
                due_days: 2,
            })
            .unwrap();
        }
        let dir = std::env::temp_dir().join(format!("capitol-departed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("town.toml");
        let text = format!("[[citizen]]\nname = \"Stays\"\nrole = \"gatherer-fiber\"\nkey = \"{}\"\n", env("STAYS_KEY"));
        std::fs::write(&path, &text).unwrap();
        let roster = Roster::parse(&text, &path).unwrap();
        write_known(&known_path(&path), &BTreeMap::from([(leaves.clone(), key.clone())])).unwrap();

        let lines = settle_departed(&roster, &server);
        for line in &lines {
            println!("capitol-town: {line}");
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("closed Leaving Soon"), "{}", lines[0]);
        assert!(lines[0].contains("1 order cancelled") && lines[0].contains("1 commission taken down"), "{}", lines[0]);
        let known = read_known(&known_path(&path));
        assert_eq!(known.keys().collect::<Vec<_>>(), ["Stays"], "the departed is forgotten, the roster known");
        let config = NetConfig::with_base_url(&server);
        match net::call_once(&config, Some(&key), &RestRequest::MyStall).unwrap() {
            RestResponse::MyStall(mine) => assert!(mine.shop.is_none(), "its stall is free again"),
            other => panic!("{other:?}"),
        }
        match net::call_once(&config, Some(&key), &RestRequest::MyOrders(MyOrdersQuery::default())).unwrap() {
            RestResponse::MyOrders(orders) => assert!(orders.is_empty(), "{orders:?}"),
            other => panic!("{other:?}"),
        }
        // A second start finds no one left to settle
        assert!(settle_departed(&roster, &server).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_keys_are_kept_beside_the_roster_and_read_back() {
        let dir = std::env::temp_dir().join(format!("capitol-known-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = known_path(&dir.join("town.toml"));
        assert_eq!(path.file_name().unwrap(), "town.known.json");
        assert!(read_known(&path).is_empty());
        let known = BTreeMap::from([("Ada Reeder".to_string(), "a".to_string())]);
        write_known(&path, &known).unwrap();
        assert_eq!(read_known(&path), known);
        std::fs::write(&path, "{ torn").unwrap();
        assert!(read_known(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
