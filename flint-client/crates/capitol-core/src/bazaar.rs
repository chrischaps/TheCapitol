//! The bazaar (E3.10, economy plan D6) as a client holds it: the directory,
//! the shops seen at their stalls, and your own; and the walk to a stall.
//!
//! The directory works from anywhere, but a shop's prices and units are seen
//! at its stall, and stocking and buying happen there too, within
//! [`rules::bazaar::STALL_RANGE`] of its footprint. Every stall faces the
//! bazaar's inner lane, so [`approach`] stands on the lane's crown in front
//! of it, and [`route`] gets there through the gate.

use std::collections::HashMap;

use capitol_protocol::rest::{BazaarDirectory, BazaarListing, BazaarPurchase, BazaarShop, MyStallResponse};
use capitol_protocol::terrain::Landmark;
use capitol_protocol::ws::ServerMessage;
use uuid::Uuid;

/// How far out from a stall's door to stand: half the inner lane's 30, so
/// every stall's approach is on the lane's crown, inside its range.
pub const APPROACH: f64 = 15.0;

/// Where to stand to deal at `stall`: on the lane's crown in front of its door.
pub fn approach(stall: &Landmark) -> (f64, f64) {
    let (dx, dy) = (stall.door.0 - stall.position.0, stall.door.1 - stall.position.1);
    let d = dx.hypot(dy).max(1e-6);
    (stall.door.0 + dx / d * APPROACH, stall.door.1 + dy / d * APPROACH)
}

/// The walk from `from` to `stall` in `bazaar`, as waypoints: from outside,
/// in at the gate, up the lane to the stall; from inside, onto the lane and
/// along it. The way to the gate is the caller's: it's on the avenue.
pub fn route(bazaar: &Landmark, stall: &Landmark, from: (f64, f64)) -> Vec<(f64, f64)> {
    let to = approach(stall);
    // Inward, from the gate to the bazaar's middle
    let (nx, ny) = (bazaar.position.0 - bazaar.door.0, bazaar.position.1 - bazaar.door.1);
    let d = nx.hypot(ny).max(1e-6);
    let n = (nx / d, ny / d);
    // The point on the lane's crown level with `p`
    let on_lane = |p: (f64, f64)| {
        let across = (to.0 - p.0) * n.0 + (to.1 - p.1) * n.1;
        (p.0 + n.0 * across, p.1 + n.1 * across)
    };
    if bazaar.distance_to(from.0, from.1) == 0.0 {
        return vec![on_lane(from), to];
    }
    vec![bazaar.door, on_lane(bazaar.door), to]
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
            ServerMessage::StallLapsed { stall_id, .. } => {
                self.shops.remove(stall_id);
                if let Some(mine) = &mut self.mine {
                    if mine.shop.as_ref().is_some_and(|s| &s.stall_id == stall_id) {
                        mine.shop = None;
                    }
                }
            }
            _ => {}
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

    /// The default world's bazaar, west of the south avenue: lateral is -x,
    /// radial is +y. The gate is on the avenue side, at x = 40; the lane's
    /// crown runs at x = 0, 40 in; front stalls are 25 deep from the gate's
    /// face, back stalls 25 deep from the far wall at x = -40.
    fn bazaar() -> Landmark {
        Landmark {
            id: "bazaar".into(),
            kind: LandmarkKind::Bazaar,
            position: (0.0, 50.0),
            corners: [(40.0, 100.0), (40.0, 0.0), (-40.0, 0.0), (-40.0, 100.0)],
            entrance_edge: (0, 1),
            door: (40.0, 50.0),
        }
    }

    /// A stall from `face` to `back` (x), between `y0` and `y1`.
    fn stall(id: &str, face: f64, back: f64, y0: f64, y1: f64) -> Landmark {
        let mid = (y0 + y1) / 2.0;
        Landmark {
            id: id.into(),
            kind: LandmarkKind::Stall,
            position: ((face + back) / 2.0, mid),
            corners: [(face, y1), (face, y0), (back, y0), (back, y1)],
            entrance_edge: (0, 1),
            door: (face, mid),
        }
    }

    #[test]
    fn every_stall_is_approached_from_the_lanes_crown_within_range() {
        let front = stall("bazaar_stall_1", 15.0, 40.0, 86.0, 100.0);
        let back = stall("bazaar_stall_7", -15.0, -40.0, 0.0, 14.3);
        for s in [&front, &back] {
            let (x, y) = approach(s);
            assert!((x - 0.0).abs() < 1e-9, "{} at {x}", s.id);
            assert!((y - s.position.1).abs() < 1e-9);
            assert!(at_the_stall(s, x, y), "{}", s.id);
        }
    }

    #[test]
    fn the_route_comes_in_at_the_gate_and_up_the_lane() {
        let back = stall("bazaar_stall_7", -15.0, -40.0, 0.0, 14.3);
        let route = route(&bazaar(), &back, (300.0, 50.0));
        assert_eq!(route.len(), 3);
        assert_eq!(route[0], (40.0, 50.0));
        // Straight in from the gate to the crown, then along it
        assert!((route[1].0 - 0.0).abs() < 1e-9 && (route[1].1 - 50.0).abs() < 1e-9, "{:?}", route[1]);
        assert_eq!(route[2], approach(&back));
    }

    #[test]
    fn from_inside_the_route_steps_onto_the_lane_and_along_it() {
        let front = stall("bazaar_stall_1", 15.0, 40.0, 86.0, 100.0);
        let route = route(&bazaar(), &front, (-30.0, 20.0));
        assert_eq!(route.len(), 2);
        assert!((route[0].0 - 0.0).abs() < 1e-9 && (route[0].1 - 20.0).abs() < 1e-9, "{:?}", route[0]);
        assert_eq!(route[1], approach(&front));
    }

    fn listing(id: Uuid, units: Units) -> BazaarListing {
        BazaarListing { id, item_type: "rope".into(), price: 7, units, durability: None, listed_at: when() }
    }

    fn shop(owner: Uuid, listings: Vec<BazaarListing>) -> BazaarShop {
        BazaarShop {
            stall_id: "bazaar_stall_4".into(),
            name: "Ada's Ropes".into(),
            owner_id: owner,
            owner_name: "ada".into(),
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
    }
}
