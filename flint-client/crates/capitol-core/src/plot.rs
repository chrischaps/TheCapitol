//! Plots (P6.4): what the plot panel offers for a plot. That depends on who
//! owns it, what its zone's deed costs and what you hold. Pure, so it's
//! tested without a server.
//!
//! **The deed cost is the plot's `deed_cost`.** The server fills it from the
//! zone's `zones.deed_cost`, which `POST /plots/{id}/claim` charges (#87).
//! Not `assessed_value`: that's 0 until the plot is first bought.
//!
//! **Build upward (V3.5, #477)** is [`upward`]: on a lot of yours with a
//! building, the floor going up and the time it has left, or the next
//! floor's bill (`PlotInfo::next_floor`, at the server's live permit scale)
//! against your Strands and your pack, which `POST /plots/{id}/floors`
//! charges.

use capitol_protocol::rest::PlotInfo;
use capitol_protocol::rules::floors;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::catalog::Catalog;
use crate::state::Container;

/// What the plot panel can offer for a plot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// It's yours.
    Yours,
    /// Someone else holds it.
    Taken { owner: String },
    /// Unclaimed, and you can pay for it (or your balance isn't known yet,
    /// in which case the server decides).
    Claimable { cost: i64 },
    /// Unclaimed, but you're `short` of its `cost`.
    TooDear { cost: i64, short: i64 },
    /// Unclaimed, in a zone whose plots can't be bought.
    NotForSale,
}

impl Offer {
    /// For `plot`, seen by the player `me` holding `balance` Strands.
    pub fn new(plot: &PlotInfo, me: Option<Uuid>, balance: Option<i64>) -> Offer {
        if let Some(owner) = plot.owner_id {
            return if Some(owner) == me {
                Offer::Yours
            } else {
                Offer::Taken {
                    owner: plot.owner_name.clone().unwrap_or_else(|| "Unknown".into()),
                }
            };
        }
        let Some(cost) = plot.deed_cost else {
            return Offer::NotForSale;
        };
        let cost = cost as i64;
        match balance {
            Some(held) if held < cost => Offer::TooDear {
                cost,
                short: cost - held,
            },
            _ => Offer::Claimable { cost },
        }
    }

    /// Whether the Claim button does anything.
    pub fn can_claim(&self) -> bool {
        matches!(self, Offer::Claimable { .. })
    }
}

/// One line of a bill (a floor's goods, a repair's): what it takes of an
/// item and what your pack holds of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    pub item_type: String,
    /// As the catalog names it, "Plank"
    pub name: String,
    pub need: i32,
    pub held: i32,
}

impl Need {
    /// `need` of `item_type` against `pack`, named by `catalog`.
    pub fn of(item_type: &str, need: i32, pack: Option<&Container>, catalog: &Catalog) -> Need {
        Need {
            item_type: item_type.to_owned(),
            name: catalog.item_name(item_type),
            need,
            held: pack.map_or(0, |p| crate::craft::count(p, item_type)),
        }
    }

    /// How many more you'd need: 0 when you hold enough.
    pub fn short(&self) -> i32 {
        (self.need - self.held).max(0)
    }
}

/// What the plot panel offers to build up on a plot (V3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upward {
    /// `floor` is going up: `left` seconds to go, of `secs` in all.
    GoingUp { floor: i32, left: i64, secs: i64 },
    /// The next floor: its permit in Strands (`short` of your balance, 0
    /// when you hold it or it isn't known), its goods against your pack,
    /// how long it takes, and the stations it holds.
    Next {
        floor: i32,
        permit: i64,
        short: i64,
        goods: Vec<Need>,
        build_secs: i64,
        room: i32,
    },
}

impl Upward {
    /// Whether Build does anything: you hold the permit and every good.
    pub fn can_build(&self) -> bool {
        match self {
            Upward::GoingUp { .. } => false,
            Upward::Next { short, goods, .. } => *short == 0 && goods.iter().all(|g| g.short() == 0),
        }
    }
}

/// What the plot panel offers to build up on `plot` at `now` (the server's
/// clock): `None` on a lot the server won't build on (no building, a
/// guild's hall) or that isn't `me`'s. `strands` and `pack` are yours.
pub fn upward(
    plot: &PlotInfo,
    me: Option<Uuid>,
    now: DateTime<Utc>,
    strands: Option<i64>,
    pack: Option<&Container>,
    catalog: &Catalog,
) -> Option<Upward> {
    if me.is_none() || plot.owner_id != me {
        return None;
    }
    let next = plot.next_floor.as_ref()?;
    if let Some(ready) = plot.floor_ready_at.filter(|at| *at > now && plot.floors > 0) {
        // Floor k takes k times the first's game time, and the bill's is the
        // next floor's. `left` is on the wall clock: under a dev server's
        // `SIM_TICK_SCALE` it's shorter, and the bar starts part way
        let k = plot.floors as i64;
        let secs = next.build_secs.saturating_mul(k) / (k + 1).max(1);
        let left = (ready - now).num_seconds().max(1);
        return Some(Upward::GoingUp { floor: plot.floors, left, secs: secs.max(left) });
    }
    Some(Upward::Next {
        floor: next.floor,
        permit: next.permit,
        short: strands.map_or(0, |s| (next.permit - s).max(0)),
        goods: next.goods.iter().map(|g| Need::of(&g.item_type, g.quantity, pack, catalog)).collect(),
        build_secs: next.build_secs,
        room: plot.indoor_room.unwrap_or(0),
    })
}

/// The plot panel's line for the storeys a building has: "Ground floor",
/// "Ground and 1 floor", "Ground and 3 floors".
pub fn storeys_words(floors: i32) -> String {
    match floors {
        i32::MIN..=0 => "Ground floor".into(),
        1 => "Ground and 1 floor".into(),
        n => format!("Ground and {n} floors"),
    }
}

/// How long a floor takes or has left, to the minute past a few: "40 s",
/// "12 min", "1 h 30 min".
pub fn build_words(secs: i64) -> String {
    let secs = secs.max(0);
    match (secs / 3600, secs % 3600 / 60) {
        (0, 0) => format!("{} s", secs.max(1)),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// The Build button's words for `floor`: "Build the first floor".
pub fn build_label(floor: i32) -> String {
    format!("Build {}", floors::storey_name(floor))
}

/// The toast as a floor you bought starts going up: "The first floor is
/// going up", "Up in 15 min. Then it holds 4 more stations indoors".
pub fn going_up_words(floor: i32, secs: i64, room: i32) -> (String, String) {
    let more = if room > 0 { format!(". Then it holds {room} more stations indoors") } else { String::new() };
    (format!("{} is going up", floors::storey_title(floor)), format!("Up in {}{more}", build_words(secs)))
}

/// The toast as a floor of yours is up: "The first floor is up", "Room
/// for 4 more stations indoors. Place one, then PgUp to go upstairs".
pub fn floor_up_words(floor: i32, room: i32) -> (String, String) {
    let body = if room > 0 {
        format!("Room for {room} more stations indoors. Place one, then PgUp to go upstairs")
    } else {
        "Place a station, then PgUp to go upstairs".to_string()
    };
    (format!("{} is up", floors::storey_title(floor)), body)
}

/// A plot's size or type as people say it: "medium" → "Medium",
/// "guild_hall" → "Guild hall".
pub fn words(raw: &str) -> String {
    let spaced = raw.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::Bounds;

    const ME: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);

    fn plot(deed_cost: Option<i32>, owner: Option<Uuid>) -> PlotInfo {
        PlotInfo {
            guild: None,
            id: Uuid::from_u128(9),
            zone_id: "urban".into(),
            world_x: 0.0,
            world_y: 0.0,
            bounds: Bounds::new(0.0, 0.0, 10.0, 10.0),
            size_category: "medium".into(),
            plot_type: "standard".into(),
            owner_id: owner,
            owner_name: owner.map(|_| "Brin".into()),
            claimed_at: None,
            assessed_value: 0,
            tax_owed: 0,
            forecloses_at: None,
            deed_cost,
            station_count: 0,
            station_capacity: 9,
            terrain_id: None,
            address: None,
            corners: None,
            indoor_room: None,
            yard_room: None,
            owner_citizen: false,
            floor_ready_at: None,
            floors: 0,
            next_floor: None,
        }
    }

    #[test]
    fn an_owned_plot_is_yours_or_theirs() {
        assert_eq!(
            Offer::new(&plot(Some(1000), Some(ME)), Some(ME), Some(0)),
            Offer::Yours
        );
        assert_eq!(
            Offer::new(&plot(Some(1000), Some(OTHER)), Some(ME), Some(5000)),
            Offer::Taken {
                owner: "Brin".into()
            }
        );
    }

    #[test]
    fn an_unclaimed_plot_costs_its_deed() {
        let open = plot(Some(1000), None);
        let offer = Offer::new(&open, Some(ME), Some(1000));
        assert_eq!(offer, Offer::Claimable { cost: 1000 });
        assert!(offer.can_claim());
        assert_eq!(
            Offer::new(&open, Some(ME), Some(400)),
            Offer::TooDear {
                cost: 1000,
                short: 600
            }
        );
        // The server decides while the balance is unknown.
        assert!(Offer::new(&open, Some(ME), None).can_claim());
        // Its assessed value plays no part: that's 0 until it's bought.
        assert_eq!(open.assessed_value, 0);
        assert_eq!(
            Offer::new(&plot(None, None), Some(ME), Some(9999)),
            Offer::NotForSale
        );
    }

    fn bill(floor: i32) -> capitol_protocol::rest::FloorBill {
        let cost = floors::cost(floor);
        capitol_protocol::rest::FloorBill {
            floor,
            permit: cost.permit,
            goods: cost
                .goods
                .iter()
                .map(|(t, n)| capitol_protocol::ws::ItemCount { item_type: (*t).into(), quantity: *n as i32 })
                .collect(),
            build_secs: floors::build_secs(floor),
        }
    }

    fn pack(items: &[(&str, i32)]) -> Container {
        let slots: Vec<_> = items
            .iter()
            .enumerate()
            .map(|(i, (t, n))| capitol_protocol::rest::SlotItem::new(Uuid::from_u128(i as u128 + 1), (*t).into(), (*t).into(), 50, *n, i as i32))
            .collect();
        Container::new(Uuid::from_u128(77), "player_inventory", 24, 6, &slots)
    }

    /// The next floor's bill, against what you hold (V3.5)
    #[test]
    fn a_lot_of_yours_offers_the_next_floor_at_its_bill() {
        let catalog = Catalog::fallback();
        let now: DateTime<Utc> = "2026-10-25T12:00:00Z".parse().unwrap();
        let mut p = plot(Some(1000), Some(ME));
        p.indoor_room = Some(4);
        assert_eq!(upward(&p, Some(ME), now, Some(5000), None, &catalog), None, "no bill: no building");
        p.next_floor = Some(bill(1));
        let all = pack(&[("plank", 30), ("brick", 45), ("iron_nails", 15)]);
        let offer = upward(&p, Some(ME), now, Some(400), Some(&all), &catalog).unwrap();
        assert!(offer.can_build(), "{offer:?}");
        let Upward::Next { floor, permit, room, build_secs, .. } = &offer else { panic!("{offer:?}") };
        assert_eq!((*floor, *permit, *room, *build_secs), (1, 400, 4, 900));
        // A Strand short, and a plank
        let some = pack(&[("plank", 29), ("brick", 45), ("iron_nails", 15)]);
        let Some(Upward::Next { short, goods, .. }) = upward(&p, Some(ME), now, Some(399), Some(&some), &catalog) else {
            panic!()
        };
        assert_eq!(short, 1);
        assert_eq!(goods.iter().map(Need::short).collect::<Vec<_>>(), vec![1, 0, 0]);
        // Not yours: nothing
        assert_eq!(upward(&p, Some(OTHER), now, Some(5000), Some(&all), &catalog), None);
    }

    /// A floor going up says how long it has left, of the time it takes
    /// at the server's scale
    #[test]
    fn a_floor_going_up_says_its_time_left() {
        let catalog = Catalog::fallback();
        let now: DateTime<Utc> = "2026-10-25T12:00:00Z".parse().unwrap();
        let mut p = plot(Some(1000), Some(ME));
        p.floors = 2;
        p.next_floor = Some(bill(3));
        p.floor_ready_at = Some(now + chrono::Duration::seconds(600));
        let offer = upward(&p, Some(ME), now, Some(0), None, &catalog).unwrap();
        assert_eq!(offer, Upward::GoingUp { floor: 2, left: 600, secs: 1800 });
        assert!(!offer.can_build());
        // At ten times game speed it's up sooner than the bill's game time
        p.floor_ready_at = Some(now + chrono::Duration::seconds(180));
        assert_eq!(upward(&p, Some(ME), now, None, None, &catalog), Some(Upward::GoingUp { floor: 2, left: 180, secs: 1800 }));
        // Up: the next one's offered
        p.floor_ready_at = Some(now - chrono::Duration::seconds(1));
        assert!(matches!(upward(&p, Some(ME), now, None, None, &catalog), Some(Upward::Next { floor: 3, .. })));
    }

    #[test]
    fn floors_are_said_for_people() {
        assert_eq!(storeys_words(0), "Ground floor");
        assert_eq!(storeys_words(1), "Ground and 1 floor");
        assert_eq!(storeys_words(4), "Ground and 4 floors");
        assert_eq!(build_words(40), "40 s");
        assert_eq!(build_words(900), "15 min");
        assert_eq!(build_words(5400), "1 h 30 min");
        assert_eq!(build_words(7200), "2 h");
        assert_eq!(build_label(2), "Build the second floor");
        assert_eq!(
            going_up_words(1, 900, 4),
            ("The first floor is going up".into(), "Up in 15 min. Then it holds 4 more stations indoors".into())
        );
        assert_eq!(floor_up_words(2, 4).0, "The second floor is up");
    }

    #[test]
    fn words_are_for_people() {
        assert_eq!(words("medium"), "Medium");
        assert_eq!(words("guild_hall"), "Guild hall");
        assert_eq!(words(""), "");
    }
}
