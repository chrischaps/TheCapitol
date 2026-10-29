//! Surplus and needs across jobs (Callings K1.4, plan D1, D7). Pure.
//!
//! A person sells what they have spare, whatever they were doing when they
//! came by it, and buys what they'll soon use. Before callings a citizen's
//! one role said both: it sold its product and bid for its role's inputs.
//! Once a citizen works in stints, ore left from an ore stint would stay in
//! its pack until it filled, and a sawyer leaning to building would never
//! bid for the stone its next stint takes. So:
//!
//! - **What it has spare** ([`spare`]) is what it holds of the goods the
//!   hall trades ([`MARKET_ITEMS`]) less what its claims still want of them
//!   ([`Holdings::promised`]), and less what its work keeps back:
//!   - the job at hand's inputs ([`Role::uses`]), all of them: it's working
//!     them now;
//!   - the job at hand's own product while that job is on the way to
//!     something else (a builder still building keeps its bricks), as
//!     before callings ([`selling`]);
//!   - a working stock ([`STOCK`]) of anything another job it's willing to
//!     do uses, for its next stint at it, but its favourite's own product
//!     (#583): an ingot crafter makes more ingots when its tools want them,
//!     so it sells what it has.
//!
//!   Everything else is spare, whatever its stint. The sell and stall
//!   menus offer it once there's enough to be worth the walk, and a trade
//!   window lays all of it out.
//! - **A tool** ([`TOOLS`]) is spare past one of each it holds (Callings
//!   K2.1): the server uses the best of a type, so a second sits in the
//!   pack. A tool crafter makes its few for sale ([`for_sale`](super::for_sale)).
//! - **What it wants** ([`wants`]) is the job at hand's inputs, then those
//!   of the jobs it leans to most ([`NEAR`]: its favourite and its biggest
//!   leanings), never what the job at hand makes, nor for those jobs what
//!   its favourite makes. Its bids, its posts and what it asks a neighbour
//!   for come from it, up to a working stock.
//!
//! A swarm bot's calling is its role alone, so it wants what its role
//! needs; what it sells stays its product, as in E3.8 (`SimBot::spare`).

use super::plan::{Goal, Holdings};
use super::stint::willing;
use super::{Role, LOT, MARKET_ITEMS, TOOLS};
use crate::citizen::calling::Calling;

/// What a citizen keeps back of a good another job it's willing to do
/// uses, for its next stint at it: two lots, which covers a forge's twelve
/// bricks, an anvil's six ingots and a kiln's ten stone. It bids for a good
/// it wants only up to this.
pub const STOCK: i32 = 2 * LOT;

/// How many jobs besides the one at hand a citizen bids for: its
/// favourite, then its biggest leanings.
pub const NEAR: usize = 2;

/// Whether the job at hand is only making more of its product, or earning
/// its float, so what it has of that product is for selling (E3.8): a
/// builder still building, or a gatherer making what a claim owes, keeps
/// it.
pub fn selling(job: Role, holdings: &Holdings, goal: Option<&Goal>) -> bool {
    let Some(product) = job.product() else { return false };
    goal == Some(&Goal::hold(product, holdings.count(product) + 1)) || matches!(goal, Some(Goal::Strands(_)))
}

/// What a citizen at `doing` has spare to sell, of the goods the hall
/// trades, the most first (ties in [`MARKET_ITEMS`]' order, then
/// [`TOOLS`]'): what it holds less what its claims still want, the job at
/// hand's inputs, its product unless it's `selling` it, a builder's next
/// floor's goods while it builds up ([`Holdings::building_up`]), and a
/// [`STOCK`] of what its other `willing` jobs use, unless its `favourite`
/// makes it. Of each tool, all but one (K2.1). Never a good with none
/// spare.
///
/// An ingot crafter leaning to tools and building kept twenty of its own
/// ingots back for them, a thousand Strands at their target, and sold
/// none under thirty. In a soak its four ingot crafters held 77 ingots
/// while 27 stood bid for at 50 to 76, and one traded (#583).
pub fn spare(holdings: &Holdings, doing: Role, selling: bool, willing: &[Role], favourite: Option<Role>) -> Vec<(&'static str, i32)> {
    let mut out: Vec<(&'static str, i32)> = MARKET_ITEMS
        .into_iter()
        .filter_map(|item| {
            let product = doing.product() == Some(item);
            if (product && !selling) || (!product && doing.uses().contains(&item)) {
                return None;
            }
            let own = favourite.is_some_and(|f| f.product() == Some(item));
            let others = !own && willing.iter().any(|&job| job != doing && job.uses().contains(&item));
            // A builder building up keeps the floor's goods (#478)
            let floor = if doing == Role::Builder { holdings.building_up(item) } else { 0 };
            let keep = holdings.promised(item) + floor + if others { STOCK } else { 0 };
            let n = holdings.count(item) - keep;
            (n > 0).then_some((item, n))
        })
        .chain(TOOLS.into_iter().filter_map(|tool| {
            let n = holdings.count(tool) - 1 - holdings.promised(tool);
            (n > 0).then_some((tool, n))
        }))
        .collect();
    // Stable, so a tie keeps the market's order
    out.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    out
}

/// What a citizen at `doing` wants of the hall's goods, soonest first: the
/// job at hand's needs, then its [`near`] jobs'. Never what `doing` makes,
/// which it's at, nor for its other jobs what its favourite makes, which
/// it would make for them (#583).
pub fn wants(calling: &Calling, doing: Role) -> Vec<&'static str> {
    let own = calling.favourite.and_then(|f| f.product());
    let mut out: Vec<&'static str> = Vec::new();
    for job in near(calling, doing) {
        for &item in job.needs() {
            // What its favourite makes it makes for its other jobs (#583)
            let made = job != doing && own == Some(item);
            if doing.product() != Some(item) && !made && !out.contains(&item) {
                out.push(item);
            }
        }
    }
    out
}

/// The job at hand, then the [`NEAR`] of its [willing](willing) jobs it
/// leans to most: its favourite, then its biggest leanings (ties in
/// [`ROLES`](super::ROLES)' order). Trading isn't among them: a market's
/// bids are its own.
pub fn near(calling: &Calling, doing: Role) -> Vec<Role> {
    let mut jobs = vec![doing];
    jobs.extend(willing(calling, doing).into_iter().filter(|&r| r != doing && r != Role::Merchant).take(NEAR));
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citizen::calling::{Mix, Style};
    use crate::citizen::persona::Rng;
    use crate::sim::{Craft, Gather, ROLES};

    const FIBER: Role = Role::Gatherer(Gather::Fiber);
    const WOOD: Role = Role::Gatherer(Gather::Wood);
    const ORE: Role = Role::Gatherer(Gather::Ore);
    const PLANKS: Role = Role::Crafter(Craft::Planks);
    const INGOTS: Role = Role::Crafter(Craft::Ingots);
    const TOOLS: Role = Role::Crafter(Craft::Tools);

    fn holding(items: &[(&str, i32)]) -> Holdings {
        let mut h = Holdings::default();
        for &(item, n) in items {
            h.items.insert(item.into(), n);
        }
        h
    }

    /// A calling: `favourite` at `focus`, and these leanings.
    fn calling(favourite: Option<Role>, focus: f32, leanings: &[(Role, f32)]) -> Calling {
        Calling {
            favourite,
            focus,
            leanings: leanings.to_vec(),
            wander: 0.0,
        }
    }

    #[test]
    fn ore_left_from_an_ore_stint_is_spare_on_the_next_unless_a_job_it_would_do_smelts_it() {
        let h = holding(&[("iron_ore", 30), ("log", 4), ("stone", 3), ("fiber", 80)]);
        // A wood stint, willing to fell trees and saw: the ore and stone are
        // spare, and fiber isn't the hall's
        assert_eq!(spare(&h, WOOD, true, &[WOOD, PLANKS], None), [("iron_ore", 30), ("stone", 3)]);
        // Willing to smelt too: a working stock of the ore stays
        assert_eq!(spare(&h, WOOD, true, &[WOOD, PLANKS, INGOTS], None), [("iron_ore", 30 - STOCK), ("stone", 3)]);
        // On an ingot stint it's working the ore, and the logs for its
        // charcoal: neither is spare
        assert_eq!(spare(&h, INGOTS, false, &[INGOTS, WOOD], None), [("stone", 3)]);
    }

    #[test]
    fn its_favourites_product_is_kept_for_no_other_job() {
        // #583: an ingot crafter leaning to tools and building, on a fiber
        // stint, held its ingots back for them and sold none under thirty
        let h = holding(&[("iron_ingot", 24), ("iron_ore", 30)]);
        let jobs = [INGOTS, FIBER, TOOLS, Role::Builder];
        assert_eq!(spare(&h, FIBER, true, &jobs, Some(INGOTS)), [("iron_ingot", 24), ("iron_ore", 30 - STOCK)]);
        // Its ore it still keeps a stock of, for its next stint at the forge;
        // and one whose favourite isn't the forge keeps the ingots too
        assert_eq!(spare(&h, FIBER, true, &jobs, Some(TOOLS)), [("iron_ore", 30 - STOCK), ("iron_ingot", 24 - STOCK)]);
        // On a tools stint it's working them: none spare
        assert_eq!(spare(&h, TOOLS, true, &jobs, Some(INGOTS)), [("iron_ore", 30 - STOCK)]);
        // Nor does it bid for its own ingots for its tools, off its tools
        // stint; on it, they're the job at hand's
        let c = calling(Some(INGOTS), 0.5, &[(TOOLS, 0.3), (Role::Builder, 0.2)]);
        assert!(!wants(&c, FIBER).contains(&"iron_ingot"), "{:?}", wants(&c, FIBER));
        assert!(wants(&c, TOOLS).contains(&"iron_ingot"));
        let smith = calling(Some(TOOLS), 0.5, &[(INGOTS, 0.3)]);
        assert!(wants(&smith, FIBER).contains(&"iron_ingot"), "a tool crafter still buys them");
    }

    #[test]
    fn nothing_a_claim_owes_is_spare() {
        let mut h = holding(&[("iron_ore", 30), ("plank", 25)]);
        h.promised = vec![("iron_ore".into(), 12), ("plank".into(), 40)];
        assert_eq!(spare(&h, WOOD, true, &[WOOD], None), [("iron_ore", 18)]);
        // Owed with some kept for a job besides
        assert!(spare(&h, WOOD, true, &[WOOD, INGOTS], None).is_empty());
    }

    /// A builder building up keeps the floor's bricks back from the
    /// market, and sells the rest (#478)
    #[test]
    fn a_builder_building_up_keeps_the_floors_bricks() {
        let mut h = holding(&[("brick", 60)]);
        h.plot_room = Some(0);
        h.strands = 600;
        h.floor = Some(crate::sim::plan::Upward {
            permit: 400,
            goods: vec![("plank".into(), 30), ("brick".into(), 45)],
            room: 4,
        });
        assert_eq!(spare(&h, Role::Builder, true, &[Role::Builder], None), [("brick", 15)]);
        // Not at the builder's job, nor without the permit, nor with room
        assert_eq!(spare(&h, WOOD, true, &[WOOD], None), [("brick", 60)]);
        h.strands = 300;
        assert_eq!(spare(&h, Role::Builder, true, &[Role::Builder], None), [("brick", 60)]);
        h.strands = 600;
        h.plot_room = Some(1);
        assert_eq!(spare(&h, Role::Builder, true, &[Role::Builder], None), [("brick", 60)]);
    }

    #[test]
    fn the_product_is_spare_while_the_job_only_makes_more_of_it() {
        let h = holding(&[("brick", 40), ("log", 30)]);
        // A builder still building keeps its bricks; once built, sells them
        assert!(spare(&h, Role::Builder, false, &[Role::Builder], None).iter().all(|(i, _)| *i != "brick"));
        assert_eq!(spare(&h, Role::Builder, true, &[Role::Builder], None)[0], ("brick", 40));
        // A builder at another job keeps its stock of bricks for the forge
        assert_eq!(spare(&h, WOOD, true, &[WOOD, Role::Builder], None), [("brick", 40 - STOCK), ("log", 30 - STOCK)]);
        // A woodcutter's logs, a working stock kept back for its sawing
        assert_eq!(spare(&h, WOOD, true, &[WOOD, PLANKS], None), [("brick", 40), ("log", 30 - STOCK)]);
        assert_eq!(spare(&h, WOOD, true, &[WOOD], None), [("brick", 40), ("log", 30)]);
        let goal = Goal::hold("log", 31);
        assert!(selling(WOOD, &h, Some(&goal)));
        assert!(selling(WOOD, &h, Some(&Goal::Strands(100))));
        assert!(!selling(Role::Builder, &h, Some(&Goal::Station("forge".into()))));
        assert!(!selling(FIBER, &h, Some(&Goal::hold("fiber", 1))), "fiber is banked, not sold");
    }

    #[test]
    fn a_tool_is_spare_past_one_of_each_whatever_the_job() {
        let h = holding(&[("stone_axe", 3), ("iron_pickaxe", 1), ("log", 12)]);
        for doing in [WOOD, TOOLS, Role::Builder] {
            let spare = spare(&h, doing, true, &[doing], None);
            assert!(spare.contains(&("stone_axe", 2)), "{doing}: {spare:?}");
            assert!(spare.iter().all(|(i, _)| *i != "iron_pickaxe"), "{doing}: the one it uses");
        }
        // A good outnumbers it, so the logs come first
        assert_eq!(spare(&h, ORE, true, &[ORE], None), [("log", 12), ("stone_axe", 2)]);
        // What a claim owes isn't spare
        let mut owing = h.clone();
        owing.promised = vec![("stone_axe".into(), 1)];
        assert_eq!(spare(&owing, ORE, true, &[ORE], None), [("log", 12), ("stone_axe", 1)]);
    }

    #[test]
    fn wants_are_the_job_at_hands_inputs_then_the_nearest_jobs() {
        // A sawyer who leans to building and tools
        let c = calling(Some(PLANKS), 0.7, &[(Role::Builder, 0.15), (TOOLS, 0.1), (ORE, 0.05)]);
        assert_eq!(near(&c, PLANKS), [PLANKS, Role::Builder, TOOLS]);
        assert_eq!(wants(&c, PLANKS), ["log", "stone", "iron_ingot", "charcoal"]);
        // On a wood stint: no bid for the logs it's felling, nor for the
        // planks its building takes, which it saws itself (#583)
        assert_eq!(near(&c, WOOD), [WOOD, PLANKS, Role::Builder]);
        assert_eq!(wants(&c, WOOD), ["stone", "iron_ingot", "charcoal"]);
        // A swarm bot wants what its role needs, as before callings
        for role in ROLES {
            assert_eq!(wants(&Calling::single(role), role), role.needs(), "{role}");
        }
    }

    #[test]
    fn surplus_per_job_mix_every_style_sells_what_its_jobs_dont_use_and_keeps_what_they_do() {
        // Every birth trade in every style, holding a bit of everything the
        // hall trades, at each job it might be on
        let h = holding(&MARKET_ITEMS.map(|i| (i, 50)));
        let mut rng = Rng::new(7);
        for style in Style::ALL {
            for role in ROLES.into_iter().filter(|&r| r != Role::Merchant) {
                let c = Calling::seeded(&mut rng, &Mix::only(style), role);
                for doing in ROLES.into_iter().filter(|&r| r != Role::Merchant && c.share(r) > 0.0) {
                    let jobs = willing(&c, doing);
                    let spare = spare(&h, doing, true, &jobs, c.favourite);
                    for item in MARKET_ITEMS {
                        let n = spare.iter().find(|(i, _)| *i == item).map_or(0, |(_, n)| *n);
                        let used_now = doing.uses().contains(&item) && doing.product() != Some(item);
                        let own = c.favourite.and_then(|f| f.product()) == Some(item);
                        let used_later = !own && jobs.iter().any(|&j| j != doing && j.uses().contains(&item));
                        let expect = if used_now {
                            0
                        } else if used_later {
                            50 - STOCK
                        } else {
                            50
                        };
                        assert_eq!(n, expect, "{style:?} {role} at {doing}: {item}");
                    }
                    // What it wants it doesn't sell all of
                    for item in wants(&c, doing) {
                        let n = spare.iter().find(|(i, _)| *i == item).map_or(0, |(_, n)| *n);
                        assert!(n <= 50 - STOCK, "{style:?} {role} at {doing} sells all its {item}");
                    }
                }
            }
        }
    }
}
