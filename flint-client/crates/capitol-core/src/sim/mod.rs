//! The economy sim (E2.7): a swarm of bots that gathers, crafts and builds
//! against a local stack for a few game hours, then reports what the economy
//! produced. `capitol-bot --sim <bots> --hours <h>` runs it.
//!
//! - [`plan`]: what a bot does next toward its goal, from the catalog and
//!   what it holds. Pure.
//! - [`report`]: the numbers, as markdown and CSV. Pure.
//! - [`run`]: the swarm itself, one thread a bot.
//!
//! Every bot plays as a player would, with no help from the server: it walks,
//! extracts, crafts at its own stations, buys a deed with fiber it deposited,
//! and places stations on its plot.
//!
//! **The market (E3.8).** Market makers (`--makers`, two by default) join the
//! swarm, funded by a grant, and quote [`MARKET_ITEMS`]. The swarm trades
//! through them ([`Role::trade`]): a bot sells what its role makes once it
//! holds [`SELL_AT`], and buys what a step would have it gather, if gathering
//! that isn't its job. Orders are placed at the Exchange hall (D6), so each
//! trade is a walk in and out, and a seller sells all it holds at once. With no makers (`--makers 0`) nobody trades and each
//! bot climbs the whole tree its role needs by itself, as in E2.7.
//!
//! **Time.** A backend started with `SIM_TICK_SCALE=10` runs ten ticks for
//! every real one, so two game hours take twelve minutes. `/health` says the
//! scale, and the report counts in game hours.

pub mod plan;
pub mod report;
pub mod run;

use std::fmt;

use capitol_protocol::rules::market::{buy_debit, OrderSide};
use plan::{Goal, Holdings, Step};

use crate::catalog::Catalog;

/// What a bot is in the swarm for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Gathers one raw material for the whole run, making the tools it needs.
    Gatherer(Gather),
    /// Makes one kind of processed good or tool, building what it takes.
    Crafter(Craft),
    /// Buys a deed and puts every station on it.
    Builder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gather {
    Fiber,
    Wood,
    Ore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Craft {
    Planks,
    Ingots,
    Tools,
}

/// The swarm's mix: the issue's three roles, each split into lanes.
pub const MIX: [(Role, f64); 7] = [
    (Role::Gatherer(Gather::Fiber), 0.20),
    (Role::Gatherer(Gather::Wood), 0.15),
    (Role::Gatherer(Gather::Ore), 0.15),
    (Role::Crafter(Craft::Planks), 0.10),
    (Role::Crafter(Craft::Ingots), 0.15),
    (Role::Crafter(Craft::Tools), 0.10),
    (Role::Builder, 0.15),
];

/// A fiber gatherer banks its fiber at the exchange once it holds this much.
const BANK_FIBER_AT: i32 = 200;

/// What the sim's market makers quote, and the swarm trades through them:
/// the raw and processed goods the chains pass along.
pub const MARKET_ITEMS: [&str; 7] = ["log", "iron_ore", "stone", "charcoal", "plank", "iron_ingot", "brick"];
/// A bot sells what its role makes once it holds this many, all of it.
pub const SELL_AT: i32 = 20;
/// Units a bot buys at a time.
pub const LOT: i32 = 10;

/// A trade a bot makes instead of the planner's step, at the best price on
/// the other side of the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trade {
    Sell { item: String, quantity: i32, price: i64 },
    Buy { item: String, quantity: i32, price: i64 },
}

impl Trade {
    /// What the report files the trade's time under: `sell log`, `buy plank`.
    pub fn label(&self) -> String {
        match self {
            Trade::Sell { item, .. } => format!("sell {item}"),
            Trade::Buy { item, .. } => format!("buy {item}"),
        }
    }
}

/// The stations a builder puts down, in order.
const BUILD: [&str; 6] = ["workbench", "kiln", "forge", "sawbench", "anvil", "storage_chest"];

/// The tools a tool crafter makes, in order: the stone tier, then iron.
const TOOLS: [&str; 8] = [
    "stone_axe",
    "stone_pickaxe",
    "stone_hammer",
    "stone_sickle",
    "iron_pickaxe",
    "iron_axe",
    "iron_hammer",
    "iron_sickle",
];

impl Role {
    /// `n` roles in [`MIX`]'s proportions, by largest remainder, dealt out
    /// so that any first few bots are already a mix.
    pub fn assign(n: usize) -> Vec<Role> {
        let total: f64 = MIX.iter().map(|(_, w)| w).sum();
        let exact: Vec<f64> = MIX.iter().map(|(_, w)| w / total * n as f64).collect();
        let mut counts: Vec<usize> = exact.iter().map(|e| e.floor() as usize).collect();
        let mut order: Vec<usize> = (0..MIX.len()).collect();
        order.sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())));
        let short = n - counts.iter().sum::<usize>();
        for &i in order.iter().take(short) {
            counts[i] += 1;
        }
        // Round-robin over the lanes, so bot 1 to 7 cover all seven.
        let mut roles = Vec::with_capacity(n);
        while roles.len() < n {
            for (i, (role, _)) in MIX.iter().enumerate() {
                if counts[i] > 0 {
                    counts[i] -= 1;
                    roles.push(*role);
                }
            }
        }
        roles
    }

    /// What the bot is after now.
    pub fn goal(&self, catalog: &Catalog, holdings: &Holdings) -> Goal {
        let more = |item: &str| Goal::hold(item, holdings.count(item) + 1);
        let unmet = |goal: &Goal| plan::next(catalog, holdings, goal) != Ok(None);
        match self {
            Role::Gatherer(Gather::Fiber) => more("fiber"),
            Role::Gatherer(Gather::Wood) => more("log"),
            Role::Gatherer(Gather::Ore) => more("iron_ore"),
            Role::Crafter(Craft::Planks) => more("plank"),
            Role::Crafter(Craft::Ingots) => more("iron_ingot"),
            Role::Crafter(Craft::Tools) => TOOLS
                .iter()
                .map(|t| Goal::hold(t, 1))
                .find(unmet)
                .unwrap_or_else(|| more("tool_handle")),
            Role::Builder => BUILD
                .iter()
                .map(|s| Goal::Station(s.to_string()))
                .find(unmet)
                .unwrap_or_else(|| more("brick")),
        }
    }

    /// What the role makes for others, if it sells anything.
    pub fn product(&self) -> Option<&'static str> {
        match self {
            Role::Gatherer(Gather::Wood) => Some("log"),
            Role::Gatherer(Gather::Ore) => Some("iron_ore"),
            Role::Crafter(Craft::Planks) => Some("plank"),
            Role::Crafter(Craft::Ingots) => Some("iron_ingot"),
            Role::Builder => Some("brick"),
            Role::Gatherer(Gather::Fiber) | Role::Crafter(Craft::Tools) => None,
        }
    }

    /// A trade to make before `step`, if the market offers one. `best` is
    /// the best price on a side of an item's book.
    ///
    /// - **Sell** all the role's product once it holds [`SELL_AT`] and is
    ///   only making more of it (a builder still building keeps its bricks),
    ///   to the best bid. Whatever the bid doesn't take comes back.
    /// - **Buy** a lot of what `step` would gather, at the best ask, if it's
    ///   on the market and gathering it isn't the role's job. A bot with no
    ///   plot keeps a deed's price back.
    pub fn trade(
        &self,
        catalog: &Catalog,
        holdings: &Holdings,
        step: &Step,
        best: impl Fn(&str, OrderSide) -> Option<i64>,
    ) -> Option<Trade> {
        if let Some(product) = self.product() {
            let held = holdings.count(product);
            if held >= SELL_AT && self.goal(catalog, holdings) == Goal::hold(product, held + 1) {
                if let Some(price) = best(product, OrderSide::Buy) {
                    return Some(Trade::Sell {
                        item: product.to_string(),
                        quantity: held,
                        price,
                    });
                }
            }
        }
        let Step::Gather { item, .. } = step else {
            return None;
        };
        if self.product() == Some(item.as_str()) || !MARKET_ITEMS.contains(&item.as_str()) {
            return None;
        }
        let price = best(item, OrderSide::Sell)?;
        let keep = if holdings.plot_room.is_none() { holdings.deed_cost } else { 0 };
        let budget = holdings.strands - keep;
        let mut quantity = LOT;
        while quantity > 0 && buy_debit(price, quantity) > budget {
            quantity -= 1;
        }
        (quantity > 0).then(|| Trade::Buy {
            item: item.clone(),
            quantity,
            price,
        })
    }

    /// Whether it's time to bank fiber at the exchange.
    pub fn banks(&self, holdings: &Holdings) -> bool {
        *self == Role::Gatherer(Gather::Fiber) && holdings.count("fiber") >= BANK_FIBER_AT
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::Gatherer(Gather::Fiber) => write!(f, "gatherer (fiber)"),
            Role::Gatherer(Gather::Wood) => write!(f, "gatherer (wood)"),
            Role::Gatherer(Gather::Ore) => write!(f, "gatherer (ore)"),
            Role::Crafter(Craft::Planks) => write!(f, "crafter (planks)"),
            Role::Crafter(Craft::Ingots) => write!(f, "crafter (ingots)"),
            Role::Crafter(Craft::Tools) => write!(f, "crafter (tools)"),
            Role::Builder => write!(f, "builder"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_bots_follow_the_mix_and_the_first_seven_cover_it() {
        let roles = Role::assign(20);
        assert_eq!(roles.len(), 20);
        let count = |r: Role| roles.iter().filter(|&&x| x == r).count();
        assert_eq!(count(Role::Gatherer(Gather::Fiber)), 4);
        assert_eq!(count(Role::Gatherer(Gather::Wood)), 3);
        assert_eq!(count(Role::Crafter(Craft::Planks)), 2);
        assert_eq!(count(Role::Builder), 3);
        let first: std::collections::HashSet<Role> = roles[..7].iter().copied().collect();
        assert_eq!(first.len(), 7);
        assert_eq!(Role::assign(1).len(), 1);
        assert_eq!(Role::assign(3).len(), 3);
    }

    #[test]
    fn a_builder_builds_in_order_then_makes_bricks() {
        let catalog = Catalog::fallback();
        let mut h = Holdings {
            deed_cost: 250,
            ..Holdings::default()
        };
        assert_eq!(Role::Builder.goal(&catalog, &h), Goal::Station("workbench".into()));
        h.stations.extend(BUILD.iter().take(2).map(|s| s.to_string()));
        assert_eq!(Role::Builder.goal(&catalog, &h), Goal::Station("forge".into()));
        h.stations.extend(BUILD.iter().map(|s| s.to_string()));
        h.items.insert("brick".into(), 4);
        assert_eq!(Role::Builder.goal(&catalog, &h), Goal::hold("brick", 5));
    }

    fn gather(item: &str) -> Step {
        Step::Gather {
            resource: "x".into(),
            item: item.into(),
        }
    }

    #[test]
    fn a_maker_of_something_sells_its_surplus_to_the_best_bid() {
        let catalog = Catalog::fallback();
        let mut h = Holdings::default();
        h.items.insert("log".into(), SELL_AT);
        let wood = Role::Gatherer(Gather::Wood);
        let bid = |item: &str, side| (item == "log" && side == OrderSide::Buy).then_some(7);
        let step = gather("log");
        assert_eq!(
            wood.trade(&catalog, &h, &step, bid),
            Some(Trade::Sell {
                item: "log".into(),
                quantity: SELL_AT,
                price: 7
            })
        );
        // No bid, no sale; and under SELL_AT it keeps gathering
        assert_eq!(wood.trade(&catalog, &h, &step, |_, _| None), None);
        h.items.insert("log".into(), SELL_AT - 1);
        assert_eq!(wood.trade(&catalog, &h, &step, bid), None);
        // A builder still building keeps its bricks
        h.items.insert("brick".into(), 40);
        let brick_bid = |_: &str, side| (side == OrderSide::Buy).then_some(9);
        assert_eq!(Role::Builder.trade(&catalog, &h, &gather("stone"), brick_bid), None);
    }

    #[test]
    fn a_bot_buys_what_it_would_gather_unless_gathering_it_is_its_job() {
        let catalog = Catalog::fallback();
        let mut h = Holdings {
            strands: 1000,
            plot_room: Some(3),
            deed_cost: 250,
            ..Holdings::default()
        };
        let ask = |_: &str, side| (side == OrderSide::Sell).then_some(8);
        let ingots = Role::Crafter(Craft::Ingots);
        assert_eq!(
            ingots.trade(&catalog, &h, &gather("log"), ask),
            Some(Trade::Buy {
                item: "log".into(),
                quantity: LOT,
                price: 8
            })
        );
        // The wood gatherer gathers its own logs
        assert_eq!(Role::Gatherer(Gather::Wood).trade(&catalog, &h, &gather("log"), ask), None);
        // Not on the market: gather it
        assert_eq!(ingots.trade(&catalog, &h, &gather("fiber"), ask), None);
        // No plot yet: the deed's price is kept back, which leaves 3 at 8
        h.plot_room = None;
        h.strands = 280;
        assert_eq!(
            ingots.trade(&catalog, &h, &gather("log"), ask),
            Some(Trade::Buy {
                item: "log".into(),
                quantity: 3,
                price: 8
            })
        );
        h.strands = 255;
        assert_eq!(ingots.trade(&catalog, &h, &gather("log"), ask), None);
    }

    #[test]
    fn only_a_fiber_gatherer_banks() {
        let mut h = Holdings::default();
        h.items.insert("fiber".into(), BANK_FIBER_AT);
        assert!(Role::Gatherer(Gather::Fiber).banks(&h));
        assert!(!Role::Builder.banks(&h));
        h.items.insert("fiber".into(), BANK_FIBER_AT - 1);
        assert!(!Role::Gatherer(Gather::Fiber).banks(&h));
    }
}
