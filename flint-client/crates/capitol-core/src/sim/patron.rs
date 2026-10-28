//! Patrons (Callings K2.3, plan D7): a rich citizen posts commissions for
//! what its home lacks, as a person with money and a home pays someone to
//! bring what they won't make themselves. Pure.
//!
//! - **Who.** A citizen with Strands over its line ([`patron_line`]: the
//!   town's `[patrons] line`, by thrift). The line is what it keeps back,
//!   so a post never takes it under it.
//! - **What** ([`home_wants`]), first things first, read from the home's
//!   own rules rather than a list of its own:
//!   - the kits of the stations its calling keeps its lot for
//!     ([`lot_for`](super::stint::lot_for), K1.3) that it doesn't own,
//!     while the lot has room for them (a fire only on the ground), and a
//!     storage chest in the room its calling doesn't want;
//!   - the next floor's goods it lacks, once those stations want more room
//!     than the lot has and a floor may go up ([`Holdings::floor`], #478);
//!   - otherwise a working stock ([`STOCK`]) of the goods its leanings use
//!     ([`surplus::wants`](super::surplus::wants)).
//! - **The price** ([`patron_menu`]) runs from the books' worth to twice
//!   it, from its target until it trades (a kit since K2.3, a good since #410)
//!   ([`worth_unbooked`](super::worth_unbooked)). Through the court's fee
//!   and escrow, at the board's grade ([`POST_GRADE`]) and under its limit
//!   ([`MAX_POSTS`]).
//!
//! A kit that comes goes down first ([`furnish`]), and builders answer
//! commissions for kits ([`Role::answers`](super::Role::answers)), as they
//! make them for their own stations.

use super::plan::{Goal, Holdings, FLOAT};
use super::surplus::STOCK;
use super::{postable, LOT, MAX_POSTS, POST_DAYS, POST_GRADE};
use crate::catalog::Catalog;
use crate::citizen::decide::{Choice, Span};

/// The Strands a middling citizen keeps back before it posts for its home:
/// past the hiring line (600) and the trading line (1200), so a citizen
/// with money hires and trades first, and pays for its home with what's
/// left over.
pub const PATRON_LINE: i64 = 1500;

/// The chest a home keeps its goods in, in the room its calling doesn't
/// want: the builder's last station.
const CHEST: &str = "storage_chest";

/// Takes one of the room `n` counts, if there's any.
fn take(n: &mut i32) -> bool {
    let free = *n > 0;
    if free {
        *n -= 1;
    }
    free
}

/// A citizen's line for posting as a patron: the town's `base` times 0.5
/// for the free-handed to 1.5 for the thriftiest, as the hiring line is.
pub fn patron_line(base: i64, thrift: f32) -> i64 {
    (base as f64 * (0.5 + f64::from(thrift.clamp(0.0, 1.0)))).round() as i64
}

/// What a patron's home lacks, first things first, with how many of each
/// (see the module's notes): the kits of the `lot_for` stations it doesn't
/// own that the lot has room for, a chest's kit in room to spare, the next
/// floor's goods when they want a floor, or else a [`STOCK`] of what it
/// `wants`. A kit in hand isn't wanted again; one a claim of its own has
/// promised isn't counted in hand (`SimBot::holdings`). Never an item it
/// has enough of.
pub fn home_wants(catalog: &Catalog, holdings: &Holdings, lot_for: &[&str], wants: &[&str]) -> Vec<(String, i32)> {
    let mut out: Vec<(String, i32)> = Vec::new();
    if let Some(room) = holdings.plot_room {
        let mut ground = (room - holdings.upstairs).max(0);
        let mut upstairs = holdings.upstairs.min(room).max(0);
        // A floor's room, if one may go up now: it never saves for one
        let floor = holdings.floor.as_ref().filter(|f| holdings.strands >= f.permit + FLOAT);
        let mut above = floor.map_or(0, |f| f.room);
        let mut going_up = false;
        let in_hand = |kit: &str| holdings.count(kit) > 0;
        for &station in lot_for {
            if holdings.stations.contains(station) {
                continue;
            }
            let Some(kind) = catalog.station_type(station) else { continue };
            let Some(kit) = kind.kit.as_deref() else { continue };
            // A fire has the ground alone; anything else goes up first
            let fits = if kind.ground_only {
                take(&mut ground)
            } else {
                take(&mut upstairs) || take(&mut ground) || {
                    let up = take(&mut above);
                    going_up |= up;
                    up
                }
            };
            if fits && !in_hand(kit) {
                out.push((kit.to_string(), 1));
            }
        }
        // A chest, in the room its calling doesn't want
        let chest = catalog.station_type(CHEST).and_then(|k| k.kit.as_deref());
        if let Some(kit) = chest {
            if !holdings.stations.contains(CHEST) && holdings.plot_spare > 0 && ground + upstairs > 0 && !in_hand(kit) {
                out.push((kit.to_string(), 1));
            }
        }
        if let (true, Some(up)) = (going_up, floor) {
            for (item, n) in &up.goods {
                let short = n - holdings.count(item);
                if short > 0 {
                    out.push((item.clone(), short));
                }
            }
        }
    }
    if out.is_empty() {
        for &item in wants {
            let short = STOCK - holdings.count(item);
            if short > 0 {
                out.push((item.to_string(), short));
            }
        }
    }
    out
}

/// The menu for a patron's post for `want` of `item` (Callings K2.3), if
/// there's a question to ask: it has fewer than [`MAX_POSTS`] open, the
/// books (or a kit's target) put a `worth` to it, and it can put the price
/// and the court's fee down on some with Strands over its `line`. Post
/// (from the worth to twice it, up to two lots), or pass.
pub fn patron_menu(holdings: &Holdings, item: &str, want: i32, worth: Option<i64>, line: i64, open: usize) -> Option<Vec<Choice>> {
    if open >= MAX_POSTS || want <= 0 {
        return None;
    }
    let worth = worth?.max(1);
    let top = worth * 2;
    let afford = postable(top, want.min(2 * LOT), holdings.strands - line);
    (afford > 0).then(|| {
        vec![
            Choice::Post {
                item: item.to_string(),
                grade: POST_GRADE.to_string(),
                price: Span::new(worth, top),
                quantity: Span::new(1, afford),
                due: POST_DAYS,
            },
            Choice::Pass,
        ]
    })
}

/// The stations a citizen holds the kit for and doesn't own, which its lot
/// takes now (Callings K2.3): what a patron's commissions brought, put down
/// before anything else. Of the stations that stand on a lot, the chest
/// among them, never a workbench, which stands where the work wants it. A
/// kit a claim of its own has promised isn't counted in hand
/// (`SimBot::holdings`), so a builder never puts down the kit it owes.
/// Only on a lot kept for a calling
/// ([`Holdings::lot_for`]): a swarm bot's is its role's, and it makes a
/// kit only for the station its ladder wants next anyway.
pub fn furnish(catalog: &Catalog, holdings: &Holdings) -> Vec<Goal> {
    if holdings.lot_for.is_none() || holdings.plot_room.is_none() {
        return Vec::new();
    }
    catalog
        .station_types()
        .iter()
        .filter(|kind| !kind.off_plot && !holdings.stations.contains(&kind.id))
        .filter(|kind| kind.kit.as_deref().is_some_and(|kit| holdings.count(kit) > 0))
        .map(|kind| Goal::Station(kind.id.clone()))
        .filter(|goal| matches!(super::plan::next(catalog, holdings, goal), Ok(Some(_))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::plan::{Step, Upward};
    use crate::sim::stint::lot_for;
    use crate::sim::Role;
    use std::collections::HashSet;

    fn home(room: i32, owns: &[&str]) -> Holdings {
        let mut h = Holdings {
            plot_room: Some(room),
            strands: 5000,
            stations: owns.iter().map(|s| s.to_string()).collect(),
            ..Holdings::default()
        };
        let kept: HashSet<String> = lot_for(&crate::citizen::calling::Calling::single(Role::Builder))
            .into_iter()
            .map(String::from)
            .collect();
        let missing = kept.iter().filter(|s| !h.stations.contains(*s)).count() as i32;
        h.plot_spare = (room - missing).max(0);
        h.lot_for = Some(kept);
        h
    }

    const BUILDER: [&str; 4] = ["kiln", "forge", "sawbench", "anvil"];

    #[test]
    fn a_patron_posts_from_a_line_its_thrift_weighs() {
        assert_eq!(patron_line(PATRON_LINE, 0.5), PATRON_LINE);
        assert_eq!(patron_line(1000, 0.0), 500);
        assert_eq!(patron_line(1000, 1.0), 1500);
        assert_eq!(patron_line(1000, 7.0), 1500, "thrift is 0 to 1");
    }

    #[test]
    fn a_home_wants_the_kits_its_calling_keeps_room_for_first() {
        let catalog = Catalog::fallback();
        let owns = home(4, &["kiln"]);
        let wants = home_wants(&catalog, &owns, &BUILDER, &["log"]);
        assert_eq!(
            wants,
            [
                ("forge_kit".to_string(), 1),
                ("sawbench_kit".into(), 1),
                ("anvil_kit".into(), 1),
                ("chest_kit".into(), 1)
            ],
            "the kiln it has, a chest in the room left; no logs while the home wants"
        );
        // A kit in hand isn't wanted again
        let mut holding = owns.clone();
        holding.items.insert("forge_kit".into(), 1);
        assert!(!home_wants(&catalog, &holding, &BUILDER, &[]).iter().any(|(i, _)| i == "forge_kit"));
        // Room for two: the first two in the calling's order
        let small = home(2, &[]);
        let wants: Vec<String> = home_wants(&catalog, &small, &BUILDER, &[]).into_iter().map(|(i, _)| i).collect();
        assert_eq!(wants, ["kiln_kit", "forge_kit"]);
    }

    #[test]
    fn a_fire_wants_the_ground() {
        let catalog = Catalog::fallback();
        // Room for two, both upstairs: only the sawbench and anvil fit
        let mut h = home(2, &[]);
        h.upstairs = 2;
        let wants: Vec<String> = home_wants(&catalog, &h, &BUILDER, &[]).into_iter().map(|(i, _)| i).collect();
        assert_eq!(wants, ["sawbench_kit", "anvil_kit"]);
    }

    #[test]
    fn a_chest_goes_in_the_room_its_calling_doesnt_want() {
        let catalog = Catalog::fallback();
        let full = home(1, &BUILDER);
        assert_eq!(home_wants(&catalog, &full, &BUILDER, &[]), [("chest_kit".to_string(), 1)]);
        let chest = home(0, &["kiln", "forge", "sawbench", "anvil", "storage_chest"]);
        assert_eq!(home_wants(&catalog, &chest, &BUILDER, &["log"]), [("log".to_string(), STOCK)], "nothing lacking");
    }

    #[test]
    fn a_full_lot_wants_a_floors_goods_once_a_floor_may_go_up() {
        let catalog = Catalog::fallback();
        let mut h = home(0, &["kiln", "forge"]);
        h.items.insert("plank".into(), 30);
        let up = Upward {
            permit: 400,
            goods: vec![("plank".into(), 40), ("brick".into(), 20)],
            room: 4,
        };
        // No floor may go up: nothing for the home it can't put anywhere
        assert_eq!(home_wants(&catalog, &h, &BUILDER, &["log"]), [("log".to_string(), STOCK)]);
        h.floor = Some(up.clone());
        assert_eq!(
            home_wants(&catalog, &h, &BUILDER, &["log"]),
            [
                ("sawbench_kit".to_string(), 1),
                ("anvil_kit".into(), 1),
                ("plank".into(), 10),
                ("brick".into(), 20)
            ]
        );
        // Without the permit and its float in hand, no floor
        h.strands = up.permit + FLOAT - 1;
        assert_eq!(home_wants(&catalog, &h, &BUILDER, &[]), []);
    }

    #[test]
    fn without_a_home_it_wants_what_its_leanings_use() {
        let catalog = Catalog::fallback();
        let mut h = Holdings::default();
        h.items.insert("log".into(), 5);
        h.items.insert("stone".into(), STOCK);
        let wants = home_wants(&catalog, &h, &[], &["log", "stone", "plank"]);
        assert_eq!(wants, [("log".to_string(), STOCK - 5), ("plank".into(), STOCK)]);
    }

    #[test]
    fn a_patron_posts_over_its_line_at_the_books_to_twice_them() {
        let h = Holdings {
            strands: 2000,
            ..Holdings::default()
        };
        let Some(menu) = patron_menu(&h, "kiln_kit", 1, Some(90), 1500, 0) else { panic!("a post") };
        assert_eq!(
            menu,
            [
                Choice::Post {
                    item: "kiln_kit".into(),
                    grade: POST_GRADE.into(),
                    price: Span::new(90, 180),
                    quantity: Span::new(1, 1),
                    due: POST_DAYS,
                },
                Choice::Pass
            ]
        );
        // Goods: up to what it wants, at most two lots, and what its line leaves
        let Some(Choice::Post { quantity, .. }) = patron_menu(&h, "log", 50, Some(5), 1500, 0).map(|m| m[0].clone()) else {
            panic!("a post")
        };
        assert_eq!(quantity, Span::new(1, 2 * LOT));
        let Some(Choice::Post { quantity, .. }) = patron_menu(&h, "log", 50, Some(20), 1800, 0).map(|m| m[0].clone()) else {
            panic!("a post")
        };
        assert!(quantity.max < 2 * LOT && quantity.max > 0, "{quantity:?}");
        assert_eq!(patron_menu(&h, "kiln_kit", 1, Some(90), 1990, 0), None, "under its line");
        assert_eq!(patron_menu(&h, "kiln_kit", 1, Some(90), 1500, MAX_POSTS), None, "the board's limit");
        assert_eq!(patron_menu(&h, "kiln_kit", 1, None, 1500, 0), None, "no worth to go by");
    }

    #[test]
    fn a_kit_that_came_goes_down_first() {
        let catalog = Catalog::fallback();
        let mut h = home(4, &[]);
        h.items.insert("anvil_kit".into(), 1);
        assert_eq!(furnish(&catalog, &h), [Goal::Station("anvil".into())]);
        let goals = Role::Gatherer(crate::sim::Gather::Fiber).goals(&catalog, &h);
        assert_eq!(goals[0], Goal::Station("anvil".into()));
        assert!(matches!(
            crate::sim::plan::next(&catalog, &h, &goals[0]),
            Ok(Some(Step::Place { .. }))
        ));
        // Not with no room, nor a workbench
        let mut full = h.clone();
        full.plot_room = Some(0);
        assert_eq!(furnish(&catalog, &full), []);
        let mut bench = home(4, &[]);
        bench.items.insert("workbench_kit".into(), 1);
        assert_eq!(furnish(&catalog, &bench), []);
        // Nor on a lot no calling keeps, as the planner's own tests have it
        let mut kept = h.clone();
        kept.lot_for = None;
        assert_eq!(furnish(&catalog, &kept), []);
    }
}
