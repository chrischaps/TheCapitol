//! The bazaar (E3.10, economy plan D6) as a client holds it: the directory,
//! the shops seen at their stalls, and your own; and the walk to a stall.
//!
//! The directory works from anywhere, but a shop's prices and units are seen
//! at its stall, and stocking and buying happen there too, within
//! [`rules::bazaar::STALL_RANGE`] of its footprint. Since E3.12 the bazaar is
//! a quarter of the Trade District: stalls along the trade ring, a lane in
//! from the avenue and an alley to a square, every stall facing its street.
//! [`approach`] stands on the street's crown in front of a stall, and
//! [`route`] gets there along the streets ([`streets`]), so a walk doesn't
//! cut through the stalls. [`Visit`] walks the route a leg at a time and
//! opens the shop as you come to it (E3.11).
//!
//! The shop's rules, so its panel previews what the server will do:
//! [`buy`] (the best units first, or one grade's), [`stock`] and [`rent`].

use std::collections::HashMap;

use capitol_protocol::rest::{
    BazaarDirectory, BazaarListing, BazaarPurchase, BazaarShop, BuyListing, MyStallResponse, RentStall, SlotItem,
    StockListing,
};
use capitol_protocol::tunables::Fees;
use capitol_protocol::rules::bazaar::{self as rules, at_the_stall, BazaarError};
use capitol_protocol::rules::quality_grade;
use capitol_protocol::rules::stacking::{Pick, Units};
use capitol_protocol::terrain::{Landmark, RoadSegment};
use capitol_protocol::ws::ServerMessage;
use chrono::{DateTime, Utc};
use uuid::Uuid;

type Point = (f64, f64);

/// The roads a walk in the bazaar keeps to: its market street (the trade
/// ring), its lane and its alley, and the avenue and the arterials that lead
/// to them.
const STREETS: [&str; 7] = [
    "trade_ring",
    "bazaar_lane",
    "bazaar_alley",
    "arterial_capitol",
    "arterial_inner_2",
    "arterial_inner_4",
    "arterial_inner_6",
];

/// The bazaar's streets, from `/terrain`'s roads, as polylines.
pub fn streets(roads: &[RoadSegment]) -> Vec<Vec<Point>> {
    roads.iter().filter(|r| STREETS.contains(&r.id.as_str())).map(|r| r.points.clone()).collect()
}

/// The nearest point to `p` on any of `streets`, and how far it is.
fn on_streets(streets: &[Vec<Point>], p: Point) -> Option<(Point, f64)> {
    streets
        .iter()
        .flat_map(|s| s.windows(2).map(|w| nearest(p, w[0], w[1])))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

fn nearest(p: Point, a: Point, b: Point) -> (Point, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let q = (a.0 + dx * t, a.1 + dy * t);
    (q, (p.0 - q.0).hypot(p.1 - q.1))
}

/// Where to stand to deal at `stall`: on its street's crown in front of its
/// door. With no streets (a world without the quarter), out from its door.
pub fn approach(stall: &Landmark, streets: &[Vec<Point>]) -> Point {
    match on_streets(streets, stall.door) {
        Some((p, _)) => p,
        None => {
            let (dx, dy) = (stall.door.0 - stall.position.0, stall.door.1 - stall.position.1);
            let d = dx.hypot(dy).max(1e-6);
            (stall.door.0 + dx / d * 15.0, stall.door.1 + dy / d * 15.0)
        }
    }
}

/// How near a street's end must come to another street to join it.
const JOIN: f64 = 25.0;

/// The walk from `from` to `stall`, as waypoints along `streets`: onto the
/// nearest street, along the streets (the shortest way, junction to
/// junction), and to the crown in front of the stall.
pub fn route(streets: &[Vec<Point>], stall: &Landmark, from: Point) -> Vec<Point> {
    let to = approach(stall, streets);
    let Some((start, _)) = on_streets(streets, from) else {
        return vec![to];
    };
    // The graph: every street's vertices, and where its ends meet another
    // street, split in
    let mut nodes: Vec<Point> = Vec::new();
    let mut segs: Vec<(usize, usize)> = Vec::new();
    for s in streets {
        let base = nodes.len();
        nodes.extend(s.iter().copied());
        segs.extend((0..s.len().saturating_sub(1)).map(|k| (base + k, base + k + 1)));
    }
    let ends: Vec<usize> = {
        let mut base = 0;
        let mut ends = Vec::new();
        for s in streets {
            if !s.is_empty() {
                ends.extend([base, base + s.len() - 1]);
            }
            base += s.len();
        }
        ends
    };
    let split = |nodes: &mut Vec<Point>, segs: &mut Vec<(usize, usize)>, p: Point, skip: Option<usize>| -> usize {
        // The segment nearest `p` (not one of `skip`'s own), split at its
        // nearest point
        let best = segs
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| Some(*a) != skip && Some(*b) != skip)
            .map(|(k, (a, b))| (k, nearest(p, nodes[*a], nodes[*b])))
            .min_by(|x, y| x.1 .1.total_cmp(&y.1 .1));
        let Some((k, (q, _))) = best else { return usize::MAX };
        let (a, b) = segs[k];
        if (q.0 - nodes[a].0).hypot(q.1 - nodes[a].1) < 0.5 {
            return a;
        }
        if (q.0 - nodes[b].0).hypot(q.1 - nodes[b].1) < 0.5 {
            return b;
        }
        let n = nodes.len();
        nodes.push(q);
        segs[k] = (a, n);
        segs.push((n, b));
        n
    };
    let mut joins = Vec::new();
    for &e in &ends {
        let p = nodes[e];
        let other = segs
            .iter()
            .filter(|(a, b)| *a != e && *b != e)
            .map(|(a, b)| nearest(p, nodes[*a], nodes[*b]).1)
            .fold(f64::MAX, f64::min);
        if other <= JOIN {
            let j = split(&mut nodes, &mut segs, p, Some(e));
            joins.push((e, j));
        }
    }
    let s = split(&mut nodes, &mut segs, start, None);
    let g = split(&mut nodes, &mut segs, to, None);
    let mut adjacent: Vec<Vec<(usize, f64)>> = vec![Vec::new(); nodes.len()];
    let dist = |a: Point, b: Point| (a.0 - b.0).hypot(a.1 - b.1);
    for &(a, b) in segs.iter().chain(joins.iter()) {
        let d = dist(nodes[a], nodes[b]);
        adjacent[a].push((b, d));
        adjacent[b].push((a, d));
    }
    // Dijkstra, small enough to scan
    let mut best = vec![f64::MAX; nodes.len()];
    let mut prev = vec![usize::MAX; nodes.len()];
    let mut done = vec![false; nodes.len()];
    best[s] = 0.0;
    while let Some(u) = (0..nodes.len()).filter(|&i| !done[i] && best[i] < f64::MAX).min_by(|&a, &b| best[a].total_cmp(&best[b])) {
        if u == g {
            break;
        }
        done[u] = true;
        for &(v, d) in &adjacent[u] {
            if best[u] + d < best[v] {
                best[v] = best[u] + d;
                prev[v] = u;
            }
        }
    }
    if best[g] == f64::MAX {
        return vec![start, to];
    }
    let mut path = vec![g];
    while let Some(&last) = path.last() {
        if last == s {
            break;
        }
        path.push(prev[last]);
    }
    path.reverse();
    let mut legs: Vec<Point> = path.into_iter().map(|i| nodes[i]).collect();
    legs.dedup_by(|a, b| dist(*a, *b) < 1.0);
    legs
}

/// How near a leg's end counts as there, in server units: a walk stops on
/// its point, but a nudge at a corner shouldn't strand the visit.
pub const WAYPOINT: f64 = 6.0;

/// What a click on a stall, or the walk to it, asks of the game.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Open the shop panel on the stall.
    Open,
    /// Walk to this server point: the next leg of the way.
    WalkTo { x: f64, y: f64 },
}

/// A walk to a stall, if one is under way: along the streets, and the shop
/// opens as you reach its counter.
#[derive(Debug, Default)]
pub struct Visit {
    /// The stall, and the end of the leg being walked
    walking: Option<(String, (f64, f64))>,
    /// The legs after this one
    legs: Vec<(f64, f64)>,
}

impl Visit {
    /// `stall` was clicked with you standing at `at`. In range, its shop
    /// opens now; otherwise the walk starts, along `streets` ([`route`]).
    pub fn click(&mut self, streets: &[Vec<Point>], stall: &Landmark, at: Point) -> Step {
        if at_the_stall(stall, at.0, at.1) {
            self.walked();
            return Step::Open;
        }
        let mut legs = route(streets, stall, at);
        let first = legs.remove(0);
        self.walking = Some((stall.id.clone(), first));
        self.legs = legs;
        Step::WalkTo { x: first.0, y: first.1 }
    }

    /// Each frame, with you at `at`: the next leg as one ends, and `Open`
    /// once, as the last leg brings you in range of the stall. A stall
    /// you pass on the way doesn't count: the market street passes many in
    /// range, and you've come to this one's counter.
    pub fn tick(&mut self, stall: &Landmark, at: (f64, f64)) -> Option<Step> {
        let (id, to) = self.walking.as_ref()?;
        if *id != stall.id {
            return None;
        }
        let there = (at.0 - to.0).hypot(at.1 - to.1) <= WAYPOINT;
        if self.legs.is_empty() {
            if there || at_the_stall(stall, at.0, at.1) {
                self.walked();
                return Some(Step::Open);
            }
            return None;
        }
        if !there {
            return None;
        }
        let next = self.legs.remove(0);
        self.walking = Some((stall.id.clone(), next));
        Some(Step::WalkTo { x: next.0, y: next.1 })
    }

    /// You walked somewhere else: the stall is no longer where you're going.
    pub fn walked(&mut self) {
        self.walking = None;
        self.legs.clear();
    }

    /// The stall being walked to.
    pub fn walking(&self) -> Option<&str> {
        self.walking.as_ref().map(|(id, _)| id.as_str())
    }
}

/// Why the shop panel can't send what it's asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// You keep this shop
    YourOwn,
    /// The listing has fewer units (of the grade asked) than that
    TooMany { has: i32 },
    /// Strands short of the cost
    Short { cost: i64, have: i64 },
    /// The stack has fewer than that
    NotHeld { has: i32 },
    /// The shop holds all the listings it may
    Full,
    /// Your rent is already paid a week ahead
    PaidUp,
    /// The rules' own: days, name, price or quantity out of bounds
    Rule(BazaarError),
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::YourOwn => "You keep this shop".into(),
            Refusal::TooMany { has } => format!("There are only {has}"),
            Refusal::Short { cost, have } => format!("It costs {cost} Strands; you have {have}"),
            Refusal::NotHeld { has } => format!("The stack holds {has}"),
            Refusal::Full => format!("A shop holds {} listings at most", rules::MAX_LISTINGS),
            Refusal::PaidUp => format!("Rent is paid {} days ahead at most", rules::MAX_RENT_DAYS),
            Refusal::Rule(e) => e.message(),
        }
    }
}

/// A purchase the panel would send, and what it comes to.
#[derive(Debug, Clone, PartialEq)]
pub struct Purchase {
    pub request: BuyListing,
    /// The units you'd get
    pub units: Units,
    pub cost: i64,
}

/// Buying `n` of `listing`: the best it has, as the server picks when no
/// units are named, or with `grade`, only units of that grade (the best of
/// them first), named. `me` refuses your own shop's, and `strands` what you
/// can't pay for.
pub fn buy(
    listing: &BazaarListing,
    n: i32,
    grade: Option<&str>,
    keeper: Uuid,
    me: Option<Uuid>,
    strands: Option<i64>,
) -> Result<Purchase, Refusal> {
    if me == Some(keeper) {
        return Err(Refusal::YourOwn);
    }
    rules::validate_quantity(n).map_err(Refusal::Rule)?;
    let pool = match grade {
        Some(g) => Units::from_pairs(listing.units.pairs().filter(|(q, _)| quality_grade(*q) == g)),
        None => listing.units.clone(),
    };
    let units = pool.pick(n, Pick::Best).ok_or(Refusal::TooMany { has: pool.count() as i32 })?;
    let cost = listing.price * i64::from(n);
    if let Some(have) = strands.filter(|have| *have < cost) {
        return Err(Refusal::Short { cost, have });
    }
    Ok(Purchase {
        request: BuyListing {
            quantity: n,
            qualities: grade.map(|_| units.clone()),
        },
        units,
        cost,
    })
}

/// Stocking your shop with `n` from `stack` at `price` a unit: the
/// stack's default pick, as the server takes it. `listings` is how many
/// the shop holds now.
pub fn stock(stack: &SlotItem, n: i32, price: i64, listings: usize) -> Result<StockListing, Refusal> {
    rules::validate_listing(price, n).map_err(Refusal::Rule)?;
    if n > stack.quantity {
        return Err(Refusal::NotHeld { has: stack.quantity });
    }
    if listings >= rules::MAX_LISTINGS {
        return Err(Refusal::Full);
    }
    Ok(StockListing {
        item_id: stack.item_id,
        quantity: n,
        price,
        qualities: None,
    })
}

/// The most days you may pay now: a week for a stall you don't keep, and
/// for your own, up to a week from `now`.
pub fn days_payable(mine: Option<&BazaarShop>, now: DateTime<Utc>) -> i64 {
    let Some(shop) = mine else {
        return rules::MAX_RENT_DAYS;
    };
    let ahead = (shop.paid_until - now).num_seconds().max(0);
    let room = rules::MAX_RENT_DAYS * rules::RENT_DAY_SECS - ahead;
    (room / rules::RENT_DAY_SECS).clamp(0, rules::MAX_RENT_DAYS)
}

/// Renting a stall, or paying more on your own (`mine`), for `days`. A new
/// rent needs a `name`; on your own stall a name renames it. What it costs
/// is `fees.rent`, at the server's fees (`GameState::fees`, D6.1).
pub fn rent(
    days: i64,
    name: &str,
    mine: Option<&BazaarShop>,
    strands: Option<i64>,
    now: DateTime<Utc>,
    fees: &Fees,
) -> Result<RentStall, Refusal> {
    rules::validate_days(days).map_err(Refusal::Rule)?;
    if days > days_payable(mine, now) {
        return Err(Refusal::PaidUp);
    }
    let name = match (name.trim(), mine) {
        ("", Some(_)) => None,
        (typed, Some(shop)) if typed == shop.name => None,
        (typed, _) => Some(rules::shop_name(typed).map_err(Refusal::Rule)?),
    };
    let cost = fees.rent(days);
    if let Some(have) = strands.filter(|have| *have < cost) {
        return Err(Refusal::Short { cost, have });
    }
    Ok(RentStall { days, name })
}

/// How a listing's units read: each grade it holds and how many, best
/// first, never their average (D2). "B 2 · C 3".
pub fn grades(units: &Units) -> Vec<(&'static str, i32)> {
    let mut out: Vec<(&'static str, i32)> = Vec::new();
    for (q, c) in units.pairs().rev() {
        let g = quality_grade(q);
        match out.last_mut() {
            Some((last, n)) if *last == g => *n += c,
            _ => out.push((g, c)),
        }
    }
    out
}

/// What the client knows of the bazaar: kept from the answers to its own
/// calls and from the notices the server sends a merchant.
#[derive(Debug, Clone, Default)]
pub struct BazaarStore {
    directory: Option<BazaarDirectory>,
    /// The shops seen at their stalls, by stall id, as last seen
    shops: HashMap<String, BazaarShop>,
    /// Your own shop, if you rent a stall, and goods from a lapsed one
    /// waiting for room in your market box. `None` until `/bazaar/mine`
    /// answers, or a rent.
    mine: Option<MyStallResponse>,
}

impl BazaarStore {
    pub fn directory(&self) -> Option<&BazaarDirectory> {
        self.directory.as_ref()
    }

    /// The shop at `stall_id`, as last seen there.
    pub fn shop(&self, stall_id: &str) -> Option<&BazaarShop> {
        self.shops.get(stall_id)
    }

    /// Your shop, if you rent a stall, as far as the store knows.
    pub fn my_shop(&self) -> Option<&BazaarShop> {
        self.mine.as_ref().and_then(|m| m.shop.as_ref())
    }

    /// Unsold goods from a lapsed stall, waiting for room in your market box.
    pub fn returning(&self) -> &[BazaarListing] {
        self.mine.as_ref().map_or(&[], |m| &m.returning)
    }

    /// The stall a listing seen anywhere is at.
    pub fn stall_of(&self, listing_id: Uuid) -> Option<&str> {
        self.my_shop()
            .into_iter()
            .chain(self.shops.values())
            .find(|s| s.listings.iter().any(|l| l.id == listing_id))
            .map(|s| s.stall_id.as_str())
    }

    pub fn set_directory(&mut self, directory: &BazaarDirectory) {
        self.directory = Some(directory.clone());
    }

    pub fn set_mine(&mut self, mine: &MyStallResponse) {
        self.mine = Some(mine.clone());
        if let Some(shop) = &mine.shop {
            self.shops.insert(shop.stall_id.clone(), shop.clone());
        }
    }

    /// A shop seen at its stall, or rented: yours if `me` owns it.
    pub fn seen(&mut self, shop: &BazaarShop, me: Option<Uuid>) {
        if Some(shop.owner_id) == me {
            let mine = self.mine.get_or_insert_with(|| MyStallResponse { shop: None, returning: Vec::new() });
            mine.shop = Some(shop.clone());
        }
        self.shops.insert(shop.stall_id.clone(), shop.clone());
    }

    /// A listing you stocked.
    pub fn listed(&mut self, listing: &BazaarListing) {
        self.each_of_mine(|shop| shop.listings.push(listing.clone()));
    }

    /// A listing you took off your shop.
    pub fn unlisted(&mut self, listing_id: Uuid) {
        self.each_of_mine(|shop| shop.listings.retain(|l| l.id != listing_id));
    }

    /// A purchase you made: the listing as it stands now, or gone.
    pub fn bought(&mut self, listing_id: Uuid, purchase: &BazaarPurchase) {
        for shop in self.shops.values_mut() {
            match &purchase.listing {
                Some(now) => shop.listings.iter_mut().filter(|l| l.id == listing_id).for_each(|l| *l = now.clone()),
                None => shop.listings.retain(|l| l.id != listing_id),
            }
        }
    }

    /// The notices to a merchant: a sale takes its units off your listing,
    /// and a lapse closes your shop. Anything else changes nothing.
    pub fn apply_ws(&mut self, message: &ServerMessage) {
        match message {
            ServerMessage::BazaarSale { listing_id, units, .. } => self.each_of_mine(|shop| {
                for listing in shop.listings.iter_mut().filter(|l| l.id == *listing_id) {
                    let _ = listing.units.take_units(units);
                }
                shop.listings.retain(|l| !l.units.is_empty());
            }),
            ServerMessage::StallLapsed { stall_id, .. } => self.closed(stall_id),
            _ => {}
        }
    }

    /// The shop at `stall_id` closed: its rent ran out, or you gave it up
    /// (#268). The goods it waits to send home come with `/bazaar/mine`.
    pub fn closed(&mut self, stall_id: &str) {
        self.shops.remove(stall_id);
        if let Some(mine) = &mut self.mine {
            if mine.shop.as_ref().is_some_and(|s| s.stall_id == stall_id) {
                mine.shop = None;
            }
        }
    }

    /// Runs `change` on your shop, in both places the store keeps it.
    fn each_of_mine(&mut self, mut change: impl FnMut(&mut BazaarShop)) {
        let Some(shop) = self.mine.as_mut().and_then(|m| m.shop.as_mut()) else {
            return;
        };
        change(shop);
        if let Some(seen) = self.shops.get_mut(&shop.stall_id) {
            change(seen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::bazaar::at_the_stall;
    use capitol_protocol::rules::stacking::Units;
    use capitol_protocol::terrain::LandmarkKind;
    use chrono::{DateTime, Utc};

    fn when() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    /// A small town: an avenue up x = 0, a lane west from it at y = 100, and
    /// a street down x = -150 that the lane runs into.
    fn town() -> Vec<Vec<Point>> {
        vec![
            vec![(0.0, 0.0), (0.0, 200.0)],
            vec![(0.0, 100.0), (-150.0, 100.0)],
            vec![(-150.0, 0.0), (-150.0, 200.0)],
        ]
    }

    /// A stall whose door is at `door`, `w` wide, going back `d` along
    /// `inward`.
    fn stall(id: &str, door: Point, inward: Point, w: f64, d: f64) -> Landmark {
        let across = (-inward.1, inward.0);
        let a = (door.0 - across.0 * w / 2.0, door.1 - across.1 * w / 2.0);
        let b = (door.0 + across.0 * w / 2.0, door.1 + across.1 * w / 2.0);
        let back = |p: Point| (p.0 + inward.0 * d, p.1 + inward.1 * d);
        Landmark {
            id: id.into(),
            kind: LandmarkKind::Stall,
            position: (door.0 + inward.0 * d / 2.0, door.1 + inward.1 * d / 2.0),
            corners: [b, a, back(a), back(b)],
            entrance_edge: (0, 1),
            door,
        }
    }

    /// On the far street, west of it, facing east onto it
    fn far_stall() -> Landmark {
        stall("bazaar_stall_9", (-168.0, 40.0), (-1.0, 0.0), 28.0, 28.0)
    }

    #[test]
    fn a_stall_is_approached_from_its_streets_crown_within_range() {
        let s = far_stall();
        let (x, y) = approach(&s, &town());
        assert!((x + 150.0).abs() < 1e-9 && (y - 40.0).abs() < 1e-9, "{x}, {y}");
        assert!(at_the_stall(&s, x, y));
        // With no streets, out from its door
        let (x, y) = approach(&s, &[]);
        assert!((x + 153.0).abs() < 1e-9 && (y - 40.0).abs() < 1e-9, "{x}, {y}");
    }

    #[test]
    fn the_route_keeps_to_the_streets_junction_to_junction() {
        let s = far_stall();
        let legs = route(&town(), &s, (10.0, 180.0));
        let at = |p: Point, q: Point| (p.0 - q.0).hypot(p.1 - q.1) < 1e-6;
        assert!(at(legs[0], (0.0, 180.0)), "onto the avenue first: {legs:?}");
        assert!(legs.iter().any(|p| at(*p, (0.0, 100.0))), "the lane's mouth: {legs:?}");
        assert!(legs.iter().any(|p| at(*p, (-150.0, 100.0))), "the lane's end: {legs:?}");
        assert!(at(*legs.last().unwrap(), approach(&s, &town())));
        // Every leg runs along a street
        for w in legs.windows(2) {
            for k in 0..=10 {
                let t = k as f64 / 10.0;
                let p = (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
                assert!(on_streets(&town(), p).unwrap().1 < 1e-6, "{p:?} is off the streets");
            }
        }
    }

    #[test]
    fn a_visit_walks_the_streets_and_opens_at_the_counter_only() {
        let s = far_stall();
        let mut v = Visit::default();
        let first = v.click(&town(), &s, (10.0, 180.0));
        assert_eq!(first, Step::WalkTo { x: 0.0, y: 180.0 });
        assert_eq!(v.walking(), Some("bazaar_stall_9"));
        assert_eq!(v.tick(&s, (9.0, 180.0)), None, "still on the way to the avenue");
        let (ax, ay) = approach(&s, &town());
        // Down the avenue and along the lane: legs, never Open
        assert_eq!(v.tick(&s, (0.0, 180.0)), Some(Step::WalkTo { x: 0.0, y: 100.0 }));
        assert_eq!(v.tick(&s, (0.0, 100.0)), Some(Step::WalkTo { x: -150.0, y: 100.0 }));
        assert_eq!(v.tick(&s, (-150.0, 100.0)), Some(Step::WalkTo { x: ax, y: ay }));
        assert_eq!(v.tick(&s, (-150.0, 75.0)), None, "on the street, not yet in range");
        assert_eq!(v.tick(&s, (ax, ay)), Some(Step::Open));
        assert_eq!(v.tick(&s, (ax, ay)), None, "only once");
        // In range at the start, it opens at once
        assert_eq!(Visit::default().click(&town(), &s, (ax, ay)), Step::Open);
    }

    #[test]
    fn walking_elsewhere_or_to_another_stall_drops_the_visit() {
        let s = far_stall();
        let other = stall("bazaar_stall_8", (-168.0, 70.0), (-1.0, 0.0), 28.0, 28.0);
        let mut v = Visit::default();
        v.click(&town(), &s, (10.0, 180.0));
        assert_eq!(v.tick(&other, (0.0, 180.0)), None);
        v.walked();
        assert_eq!(v.tick(&s, (0.0, 180.0)), None);
    }

    /// The default world's quarter (the fixture, E3.12): from the avenue
    /// opposite the hall, every stall is reached along the streets, through
    /// no other stall, and the walk ends in its range.
    #[test]
    fn every_stall_in_the_quarter_is_reached_along_the_streets() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = crate::terrain::read_terrain_file(&path).expect("fixture");
        let streets = streets(&terrain.roads);
        let stalls: Vec<&Landmark> = terrain.landmarks.iter().filter(|l| l.kind == LandmarkKind::Stall).collect();
        assert!(stalls.len() >= 34, "{} stalls: refresh the fixture", stalls.len());
        let mouth = terrain.landmarks.iter().find(|l| l.kind == LandmarkKind::Bazaar).unwrap();
        let avenue = terrain.center_x;
        for from in [(avenue, mouth.door.1), (avenue, terrain.center_y + 550.0)] {
            for s in &stalls {
                let legs = route(&streets, s, from);
                let end = *legs.last().unwrap();
                assert!(at_the_stall(s, end.0, end.1), "{} from {from:?}", s.id);
                let mut prev = from;
                for &p in &legs {
                    for k in 0..=20 {
                        let t = k as f64 / 20.0;
                        let q = (prev.0 + (p.0 - prev.0) * t, prev.1 + (p.1 - prev.1) * t);
                        for o in &stalls {
                            assert!(o.distance_to(q.0, q.1) > 0.0, "the walk to {} crosses {} at {q:?}", s.id, o.id);
                        }
                    }
                    prev = p;
                }
            }
        }
    }

    #[test]
    fn a_buy_takes_the_best_or_one_grades_best_and_names_them_only_then() {
        let (keeper, me) = (Uuid::from_u128(3), Uuid::from_u128(1));
        let l = listing(Uuid::from_u128(2), Units::from_pairs([(50, 2), (60, 1), (80, 2), (92, 1)]));
        let p = buy(&l, 2, None, keeper, Some(me), Some(100)).unwrap();
        assert_eq!(p.units, Units::from_pairs([(92, 1), (80, 1)]));
        assert_eq!(p.cost, 14);
        assert_eq!(p.request, BuyListing { quantity: 2, qualities: None }, "the server picks the best itself");

        let p = buy(&l, 2, Some("B"), keeper, Some(me), Some(100)).unwrap();
        assert_eq!(p.units, Units::uniform(2, 80), "the 92 is an A");
        assert_eq!(p.request.qualities, Some(Units::uniform(2, 80)));
        assert_eq!(buy(&l, 1, Some("C"), keeper, Some(me), Some(100)).unwrap().units, Units::uniform(1, 60));
        assert_eq!(buy(&l, 2, Some("C"), keeper, Some(me), Some(100)), Err(Refusal::TooMany { has: 1 }));

        assert_eq!(buy(&l, 3, Some("B"), keeper, Some(me), None), Err(Refusal::TooMany { has: 2 }));
        assert_eq!(buy(&l, 6, None, keeper, Some(me), Some(41)), Err(Refusal::Short { cost: 42, have: 41 }));
        assert_eq!(buy(&l, 1, None, keeper, Some(keeper), Some(100)), Err(Refusal::YourOwn));
        assert!(matches!(buy(&l, 0, None, keeper, Some(me), Some(100)), Err(Refusal::Rule(_))));
    }

    #[test]
    fn stocking_is_bounded_by_the_stack_the_price_and_the_shelf() {
        let stack = SlotItem::new(Uuid::from_u128(5), "rope".into(), "Rope".into(), 60, 4, 0);
        let s = stock(&stack, 4, 9, 0).unwrap();
        assert_eq!((s.item_id, s.quantity, s.price, s.qualities), (stack.item_id, 4, 9, None));
        assert_eq!(stock(&stack, 5, 9, 0), Err(Refusal::NotHeld { has: 4 }));
        assert_eq!(stock(&stack, 1, 9, rules::MAX_LISTINGS), Err(Refusal::Full));
        assert!(matches!(stock(&stack, 1, 0, 0), Err(Refusal::Rule(BazaarError::BadPrice))));
    }

    #[test]
    fn rent_is_a_week_at_most_ahead_and_a_new_shop_needs_a_name() {
        let now = when();
        let me = Uuid::from_u128(1);
        assert_eq!(days_payable(None, now), 7);
        assert_eq!(rent(3, " Ada's Ropes ", None, Some(60), now, &Fees::default()).unwrap(), RentStall {
            days: 3,
            name: Some("Ada's Ropes".into())
        });
        assert!(matches!(rent(1, "  ", None, Some(60), now, &Fees::default()), Err(Refusal::Rule(BazaarError::BadName))));
        assert_eq!(rent(3, "Ada's", None, Some(59), now, &Fees::default()), Err(Refusal::Short { cost: 60, have: 59 }));
        // At the rent the server sent (D6.1)
        let dear = Fees { stall_rent_per_day: 30, ..Fees::default() };
        assert_eq!(rent(3, "Ada's", None, Some(60), now, &dear), Err(Refusal::Short { cost: 90, have: 60 }));
        assert!(matches!(rent(8, "Ada's", None, None, now, &Fees::default()), Err(Refusal::Rule(BazaarError::BadDays))));

        // Your own, paid two and a half days ahead: four more fit in the week
        let mut own = shop(me, Vec::new());
        own.paid_until = now + chrono::Duration::hours(60);
        assert_eq!(days_payable(Some(&own), now), 4);
        assert_eq!(rent(4, "", Some(&own), None, now, &Fees::default()).unwrap(), RentStall { days: 4, name: None });
        assert_eq!(rent(4, "Ada's Ropes", Some(&own), None, now, &Fees::default()).unwrap().name, None, "the same name isn't a rename");
        assert_eq!(rent(1, "Ada's Twine", Some(&own), None, now, &Fees::default()).unwrap().name, Some("Ada's Twine".into()));
        assert_eq!(rent(5, "", Some(&own), None, now, &Fees::default()), Err(Refusal::PaidUp));
        own.paid_until = now + chrono::Duration::days(7);
        assert_eq!(days_payable(Some(&own), now), 0);
    }

    #[test]
    fn grades_read_best_first_and_never_averaged() {
        let units = Units::from_pairs([(50, 2), (60, 1), (80, 2), (92, 1)]);
        assert_eq!(grades(&units), vec![("A", 1), ("B", 2), ("C", 1), ("D", 2)]);
        assert!(grades(&Units::new()).is_empty());
    }

    fn listing(id: Uuid, units: Units) -> BazaarListing {
        BazaarListing { id, item_type: "rope".into(), price: 7, units, durability: None, listed_at: when(), maker: None }
    }

    fn shop(owner: Uuid, listings: Vec<BazaarListing>) -> BazaarShop {
        BazaarShop {
            stall_id: "bazaar_stall_4".into(),
            name: "Ada's Ropes".into(),
            owner_id: owner,
            owner_name: "ada".into(),
            owner_citizen: false,
            paid_until: when(),
            listings,
        }
    }

    #[test]
    fn a_sale_takes_its_units_off_your_listing_and_a_lapse_closes_the_shop() {
        let (me, id) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut store = BazaarStore::default();
        store.seen(&shop(me, vec![listing(id, Units::from_pairs([(60, 2), (70, 3)]))]), Some(me));
        let sale = |units: Units| ServerMessage::BazaarSale {
            stall_id: "bazaar_stall_4".into(),
            listing_id: id,
            item_type: "rope".into(),
            buyer_name: "bram".into(),
            buyer_citizen: false,
            units,
            price: 7,
            paid: 0,
        };
        store.apply_ws(&sale(Units::from_pairs([(70, 3)])));
        assert_eq!(store.my_shop().unwrap().listings[0].units, Units::uniform(2, 60));
        assert_eq!(store.shop("bazaar_stall_4").unwrap().listings[0].units, Units::uniform(2, 60));
        store.apply_ws(&sale(Units::uniform(2, 60)));
        assert!(store.my_shop().unwrap().listings.is_empty());

        store.apply_ws(&ServerMessage::StallLapsed {
            stall_id: "bazaar_stall_4".into(),
            name: "Ada's Ropes".into(),
            returned: 0,
            waiting: 0,
        });
        assert!(store.my_shop().is_none());
        assert!(store.shop("bazaar_stall_4").is_none());
    }

    #[test]
    fn someone_elses_shop_is_seen_but_not_yours_and_a_purchase_updates_it() {
        let (me, them, id) = (Uuid::from_u128(1), Uuid::from_u128(3), Uuid::from_u128(2));
        let mut store = BazaarStore::default();
        store.seen(&shop(them, vec![listing(id, Units::uniform(5, 50))]), Some(me));
        assert!(store.my_shop().is_none());
        assert_eq!(store.stall_of(id), Some("bazaar_stall_4"));

        let purchase = |left: Option<BazaarListing>| BazaarPurchase {
            units: Units::uniform(2, 50),
            paid: 14,
            item_id: Uuid::from_u128(9),
            into: "player_inventory".into(),
            listing: left,
        };
        store.bought(id, &purchase(Some(listing(id, Units::uniform(3, 50)))));
        assert_eq!(store.shop("bazaar_stall_4").unwrap().listings[0].units.count(), 3);
        store.bought(id, &purchase(None));
        assert!(store.shop("bazaar_stall_4").unwrap().listings.is_empty());
        assert_eq!(store.stall_of(id), None);
    }

    #[test]
    fn stocking_and_unlisting_change_your_shop() {
        let (me, id) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut store = BazaarStore::default();
        // Before the store knows your shop, there's nothing to add to
        store.listed(&listing(id, Units::uniform(1, 40)));
        assert!(store.my_shop().is_none());
        store.set_mine(&MyStallResponse { shop: Some(shop(me, Vec::new())), returning: Vec::new() });
        store.listed(&listing(id, Units::uniform(1, 40)));
        assert_eq!(store.my_shop().unwrap().listings.len(), 1);
        assert_eq!(store.shop("bazaar_stall_4").unwrap().listings.len(), 1);
        store.unlisted(id);
        assert!(store.my_shop().unwrap().listings.is_empty());
        assert!(store.shop("bazaar_stall_4").unwrap().listings.is_empty());
        // Giving it up closes it in both places (#268)
        store.closed("bazaar_stall_4");
        assert!(store.my_shop().is_none());
        assert!(store.shop("bazaar_stall_4").is_none());
    }
}
