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
//! and places stations on its plot. It doesn't trade (there's no market yet,
//! E3), so each bot climbs the whole tree its role needs by itself. That
//! makes the sim a measure of one player's path through the content, run
//! twenty times at once on shared nodes.
//!
//! **Time.** A backend started with `SIM_TICK_SCALE=10` runs ten ticks for
//! every real one, so two game hours take twelve minutes. `/health` says the
//! scale, and the report counts in game hours.

pub mod plan;
pub mod report;
pub mod run;

use std::fmt;

use plan::{Goal, Holdings};

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
