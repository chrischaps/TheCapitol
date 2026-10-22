//! The economy sim (E2.7): a swarm of bots that gathers, crafts and builds
//! against a local stack for a few game hours, then reports what the economy
//! produced. `capitol-bot --sim <bots> --hours <h>` runs it.
//!
//! - [`lot`]: where its stations go on its lot. Pure.
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
//! through them ([`Role::sell_menu`], [`Role::buy_menu`], decided since C1.3 by
//! the rules layer, `citizen::decide`): a bot sells what its role makes once it
//! holds [`SELL_AT`], and buys what a step would have it gather, if gathering
//! that isn't its job. Orders are placed at the Exchange hall (D6), so each
//! trade is a walk in and out, and a seller sells all it holds at once. With no makers (`--makers 0`) nobody trades and each
//! bot climbs the whole tree its role needs by itself, as in E2.7.
//!
//! **Time.** A backend started with `SIM_TICK_SCALE=10` runs ten ticks for
//! every real one, so two game hours take twelve minutes. `/health` says the
//! scale, and the report counts in game hours.

pub mod lot;
pub mod plan;
pub mod report;
pub mod run;

use std::fmt;

use plan::{Goal, Holdings, Step, FLOAT, FLOOR};

use crate::catalog::Catalog;
use crate::citizen::decide::{Choice, Span};

/// An entry on a citizen's work menu: the goal it's for (none for banking)
/// and the step it starts with.
#[derive(Debug, Clone, PartialEq)]
pub struct Errand {
    pub goal: Option<Goal>,
    pub step: Step,
}

impl Errand {
    /// The menu's entry for it.
    pub fn choice(&self, catalog: &Catalog) -> Choice {
        match &self.goal {
            None => Choice::Bank,
            Some(goal) => Choice::Work {
                goal: goal.label(),
                step: self.step.label(catalog),
            },
        }
    }
}

/// What a bot is in the swarm for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Gathers one raw material for the whole run, making the tools it needs.
    Gatherer(Gather),
    /// Makes one kind of processed good or tool, building what it takes.
    Crafter(Craft),
    /// Buys a deed and puts every station on it.
    Builder,
    /// Earns a float by gathering fiber and banking it, then makes a market
    /// in a few goods with it (C1.6). Not in the swarm's [`MIX`]: the town
    /// mints merchants when asked (`mint --role merchant`).
    Merchant,
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

/// Every role: the swarm's [`MIX`], then the merchant.
pub const ROLES: [Role; 8] = [
    Role::Gatherer(Gather::Fiber),
    Role::Gatherer(Gather::Wood),
    Role::Gatherer(Gather::Ore),
    Role::Crafter(Craft::Planks),
    Role::Crafter(Craft::Ingots),
    Role::Crafter(Craft::Tools),
    Role::Builder,
    Role::Merchant,
];

/// A fiber gatherer banks its fiber at the exchange once it holds this much.
pub const BANK_FIBER_AT: i32 = 200;
/// The Strands a middling merchant earns before it makes a market, its
/// float (C1.6): two levels of [`MIN_ORDER_VALUE`](crate::market_maker::MIN_ORDER_VALUE)
/// a side in each of its [`MERCHANT_ITEMS`]' worth, about. Patience scales
/// it 0.5 to 1.5, as it does a lot.
pub const MERCHANT_FLOAT: i64 = 600;
/// How many goods a merchant deals in.
pub const MERCHANT_ITEMS: usize = 3;

/// What the sim's market makers quote, and the swarm trades through them:
/// the raw and processed goods the chains pass along.
pub const MARKET_ITEMS: [&str; 7] = ["log", "iron_ore", "stone", "charcoal", "plank", "iron_ingot", "brick"];
/// A bot sells what its role makes once it holds this many, all of it.
pub const SELL_AT: i32 = 20;
/// Units a bot buys at a time.
pub const LOT: i32 = 10;
/// A citizen keeps a stall once it holds this many of what it makes, with
/// nobody bidding for them at the hall (C1.5).
pub const STALL_AT: i32 = 2 * SELL_AT;
/// Days of rent a citizen pays at a time.
pub const STALL_DAYS: i64 = 2;

/// A trade a bot makes instead of the planner's step, as its mind decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trade {
    Sell { item: String, quantity: i32, price: i64 },
    Buy { item: String, quantity: i32, price: i64 },
    /// A standing bid, left at the hall (C1.5).
    Bid { item: String, quantity: i32, price: i64 },
    /// Units for its stall at the bazaar, renting one if need be (C1.5).
    Stock { item: String, quantity: i32, price: i64 },
    /// A merchant's spell of making a market in its goods (C1.6).
    MakeMarket,
}

impl Trade {
    /// What the report files the trade's time under: `sell log`, `buy plank`.
    pub fn label(&self) -> String {
        match self {
            Trade::Sell { item, .. } => format!("sell {item}"),
            Trade::Buy { item, .. } => format!("buy {item}"),
            Trade::Bid { item, .. } => format!("bid for {item}"),
            Trade::Stock { item, .. } => format!("stock {item}"),
            Trade::MakeMarket => "make a market".into(),
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

    /// What the bot is after now: the first of [`goals`](Self::goals).
    pub fn goal(&self, catalog: &Catalog, holdings: &Holdings) -> Goal {
        self.goals(catalog, holdings).remove(0)
    }

    /// What the role is after, first things first. A tool crafter's tools
    /// and a builder's stations come in order, those it lacks, then more of
    /// what it makes once it has them all. Never empty.
    ///
    /// A role that doesn't [`bank`](Self::banks) and is down to [`FLOOR`]
    /// earns its [`FLOAT`] back by fiber first (#294): with nothing in hand
    /// it can't pay a sale's fee, so it can't sell its way out, and it can't
    /// buy what it would otherwise gather.
    pub fn goals(&self, catalog: &Catalog, holdings: &Holdings) -> Vec<Goal> {
        let mut goals = self.work(catalog, holdings);
        if !self.banks() && holdings.strands < FLOOR {
            goals.insert(0, Goal::Strands(FLOAT));
        }
        goals
    }

    /// What the role works at: [`goals`](Self::goals) less its float.
    fn work(&self, catalog: &Catalog, holdings: &Holdings) -> Vec<Goal> {
        let more = |item: &str| Goal::hold(item, holdings.count(item) + 1);
        let unmet = |goal: &Goal| plan::next(catalog, holdings, goal) != Ok(None);
        let then = |lacking: Vec<Goal>, more: Goal| -> Vec<Goal> { lacking.into_iter().filter(unmet).chain([more]).collect() };
        match self {
            // A merchant earns its float as a fiber gatherer does
            Role::Gatherer(Gather::Fiber) | Role::Merchant => vec![more("fiber")],
            Role::Gatherer(Gather::Wood) => vec![more("log")],
            Role::Gatherer(Gather::Ore) => vec![more("iron_ore")],
            Role::Crafter(Craft::Planks) => vec![more("plank")],
            Role::Crafter(Craft::Ingots) => vec![more("iron_ingot")],
            Role::Crafter(Craft::Tools) => then(TOOLS.iter().map(|t| Goal::hold(t, 1)).collect(), more("tool_handle")),
            Role::Builder => then(BUILD.iter().map(|s| Goal::Station(s.to_string())).collect(), more("brick")),
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
            Role::Gatherer(Gather::Fiber) | Role::Crafter(Craft::Tools) | Role::Merchant => None,
        }
    }

    /// How many stations the role puts on its lot, so the lot it buys has
    /// the room (#419). The workbench stands off the plot. A plank crafter
    /// needs the sawbench; an ingot crafter the forge and the kiln for its
    /// charcoal; a tool crafter those and the anvil; a builder the kiln,
    /// forge, sawbench and anvil (#305), its chest last, where it fits. A
    /// role with no stations never claims, so its 1 is only a floor.
    pub fn room_wanted(&self) -> i32 {
        match self {
            Role::Crafter(Craft::Planks) => 1,
            Role::Crafter(Craft::Ingots) => 2,
            Role::Crafter(Craft::Tools) => 3,
            Role::Builder => 4,
            Role::Gatherer(_) | Role::Merchant => 1,
        }
    }

    /// Whether the role earns by banking fiber at the exchange.
    pub fn banks(&self) -> bool {
        matches!(self, Role::Gatherer(Gather::Fiber) | Role::Merchant)
    }

    /// A citizen's work menu (C1.3): banking, for a role that
    /// [`banks`](Self::banks) with fiber to bank, then each of [`goals`](Self::goals) in order with the
    /// step it starts with. Goals already met, and goals no step leads to,
    /// are left off. `Err` names why, when every goal is stuck.
    pub fn errands(&self, catalog: &Catalog, holdings: &Holdings) -> Result<Vec<Errand>, plan::Stuck> {
        let mut errands = Vec::new();
        if self.banks() && holdings.count(crate::exchange::FIBER) >= BANK_FIBER_AT / 4 {
            errands.push(Errand {
                goal: None,
                step: Step::Deposit,
            });
        }
        let mut stuck = None;
        for goal in self.goals(catalog, holdings) {
            match plan::next(catalog, holdings, &goal) {
                Ok(Some(step)) => errands.push(Errand { goal: Some(goal), step }),
                Ok(None) => {}
                Err(e) => {
                    stuck.get_or_insert(e);
                }
            }
        }
        match stuck {
            Some(e) if errands.is_empty() => Err(e),
            _ => Ok(errands),
        }
    }

    /// The menu for selling the role's product before working toward
    /// `goal`, if there's a question to ask: it holds at least half of
    /// [`SELL_AT`], is only making more of it (a builder still building
    /// keeps its bricks) or earning its float, someone bids, and it can pay
    /// the listing fee on some. Sell (a price round the bid, up to all it
    /// holds, or as many as it can pay the fee on at the top price), or keep.
    pub fn sell_menu(&self, holdings: &Holdings, goal: Option<&Goal>, bid: Option<i64>, ask: Option<i64>) -> Option<Vec<Choice>> {
        let product = self.product()?;
        let held = holdings.count(product);
        let selling = goal == Some(&Goal::hold(product, held + 1)) || matches!(goal, Some(Goal::Strands(_)));
        if held < SELL_AT / 2 || !selling {
            return None;
        }
        let bid = bid?;
        let price = Span::new((bid / 2).max(1), (bid * 2).max(ask.unwrap_or(0)));
        // A sell's fee is paid when it's placed (#294)
        let mut most = held;
        while most > 0 && holdings.fees.listing_fee(price.max * most as i64) > holdings.strands {
            most -= 1;
        }
        (most > 0).then(|| {
            vec![
                Choice::Sell {
                    item: product.to_string(),
                    price,
                    quantity: Span::new(1, most),
                },
                Choice::Keep,
            ]
        })
    }

    /// The menu for buying what `step` would gather, if there's a question
    /// to ask: it's on the market, gathering it isn't the role's job, it's
    /// for sale, and the bot can afford some at the ask, with what it
    /// [`kept`] back. Buy up to two lots, or gather it.
    pub fn buy_menu(&self, holdings: &Holdings, step: &Step, ask: Option<i64>) -> Option<Vec<Choice>> {
        let Step::Gather { item, .. } = step else {
            return None;
        };
        if self.product() == Some(item.as_str()) || !MARKET_ITEMS.contains(&item.as_str()) {
            return None;
        }
        let ask = ask?;
        let afford = affordable(holdings, ask);
        (afford > 0).then(|| {
            vec![
                Choice::Buy {
                    item: item.clone(),
                    price: Span::new((ask / 2).max(1), ask),
                    quantity: Span::new(1, afford),
                },
                Choice::Gather { item: item.clone() },
            ]
        })
    }

    /// What the role's work takes from the market that it doesn't make
    /// itself: what a citizen of it bids for (C1.5), so whoever gathers or
    /// makes it has someone to sell to. From the recipes: planks are sawn
    /// from logs, ingots smelted from ore with charcoal, tools knapped from
    /// stone with handles of plank and heads of iron, and a builder's
    /// stations take stone, logs, iron, planks and charcoal for its bricks.
    /// A merchant's buying is its market's bids.
    pub fn needs(&self) -> &'static [&'static str] {
        match self {
            Role::Gatherer(_) | Role::Merchant => &[],
            Role::Crafter(Craft::Planks) => &["log"],
            Role::Crafter(Craft::Ingots) => &["iron_ore", "charcoal"],
            Role::Crafter(Craft::Tools) => &["stone", "plank", "iron_ingot"],
            Role::Builder => &["stone", "log", "iron_ingot", "plank", "charcoal"],
        }
    }

    /// The menu for leaving a standing bid for `item` at the hall (C1.5),
    /// if there's a question to ask: the role needs it, it has no bid up for
    /// it already, the citizen can put a worth to it (`worth`, from its
    /// books), and it can afford some, with what it [`kept`] back. Bid (up
    /// to half again the worth, up to two lots), or gather it as it would
    /// have. A maker comes by its deed with its [`FLOAT`] still in hand, so
    /// it can bid once it has a plot (#294).
    pub fn bid_menu(&self, holdings: &Holdings, item: &str, worth: Option<i64>, bidding: bool) -> Option<Vec<Choice>> {
        if bidding || !self.needs().contains(&item) {
            return None;
        }
        let worth = worth?.max(1);
        let top = worth + (worth / 2).max(1);
        let afford = affordable(holdings, top);
        (afford > 0).then(|| {
            vec![
                Choice::Bid {
                    item: item.to_string(),
                    price: Span::new((worth / 2).max(1), top),
                    quantity: Span::new(1, afford),
                },
                Choice::Gather { item: item.to_string() },
            ]
        })
    }

    /// The menu for stocking a stall with what the role makes (C1.5), if
    /// there's a question to ask: it holds half of [`STALL_AT`] or more, can
    /// put a worth to it, and keeps a stall (`keeps`) or can pay
    /// [`STALL_DAYS`] of rent for one, with what it [`keeps`](kept) back.
    /// Stock (a price round its worth, up to all it holds), or keep it.
    pub fn stall_menu(&self, holdings: &Holdings, worth: Option<i64>, keeps: bool) -> Option<Vec<Choice>> {
        let product = self.product()?;
        let held = holdings.count(product);
        let worth = worth?.max(1);
        let rent = holdings.fees.rent(STALL_DAYS);
        if held < STALL_AT / 2 || (!keeps && holdings.strands - kept(holdings) < rent) {
            return None;
        }
        Some(vec![
            Choice::Stock {
                item: product.to_string(),
                price: Span::new((worth / 2).max(1), worth * 2 + 1),
                quantity: Span::new(1, held),
            },
            Choice::Keep,
        ])
    }

    /// The role as a roster names it (C1.1): `gatherer-fiber`, `builder`.
    pub fn slug(&self) -> &'static str {
        match self {
            Role::Gatherer(Gather::Fiber) => "gatherer-fiber",
            Role::Gatherer(Gather::Wood) => "gatherer-wood",
            Role::Gatherer(Gather::Ore) => "gatherer-ore",
            Role::Crafter(Craft::Planks) => "crafter-planks",
            Role::Crafter(Craft::Ingots) => "crafter-ingots",
            Role::Crafter(Craft::Tools) => "crafter-tools",
            Role::Builder => "builder",
            Role::Merchant => "merchant",
        }
    }

    /// The role a [`slug`](Self::slug) names.
    pub fn from_slug(slug: &str) -> Option<Role> {
        ROLES.into_iter().find(|role| role.slug() == slug)
    }
}

/// The goods a merchant deals in (C1.6): [`MERCHANT_ITEMS`] of
/// [`MARKET_ITEMS`], picked by `seed`, so a town's merchants spread over
/// the market and each keeps to its own goods from life to life.
pub fn merchant_items(seed: u64) -> Vec<&'static str> {
    let mut items = MARKET_ITEMS.to_vec();
    let mut h = seed;
    let mut picked = Vec::with_capacity(MERCHANT_ITEMS);
    while picked.len() < MERCHANT_ITEMS && !items.is_empty() {
        // SplitMix64's step
        h = h.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = h;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        picked.push(items.remove((z % items.len() as u64) as usize));
    }
    // In the market's own order, so a merchant's report reads the same way
    picked.sort_by_key(|i| MARKET_ITEMS.iter().position(|m| m == i));
    picked
}

/// The Strands a bot keeps back from goods and rent: [`FLOOR`] for its
/// fees, and a deed's price until it has a plot.
fn kept(holdings: &Holdings) -> i64 {
    FLOOR + if holdings.plot_room.is_none() { holdings.deed_cost } else { 0 }
}

/// How many, up to two lots, a bot can bid for at `price`, with what it
/// [`kept`] back.
fn affordable(holdings: &Holdings, price: i64) -> i32 {
    let budget = holdings.strands - kept(holdings);
    let mut afford = 2 * LOT;
    while afford > 0 && holdings.fees.buy_debit(price, afford) > budget {
        afford -= 1;
    }
    afford
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
            Role::Merchant => write!(f, "merchant"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citizen::decide::{Cascade, Place, Question, Quote, Situation, Temperament, When, Who};

    #[test]
    fn every_role_has_a_slug_that_names_it_again() {
        for role in ROLES {
            assert_eq!(Role::from_slug(role.slug()), Some(role));
        }
        assert_eq!(Role::from_slug("gatherer"), None);
        // Every role of the mix is among them, and the merchant isn't in the mix
        assert!(MIX.iter().all(|(r, _)| ROLES.contains(r)));
        assert!(!MIX.iter().any(|(r, _)| *r == Role::Merchant));
    }

    #[test]
    fn a_merchant_earns_by_fiber_and_deals_in_three_goods_of_its_own() {
        let catalog = Catalog::fallback();
        let mut h = Holdings::default();
        h.items.insert("fiber".into(), BANK_FIBER_AT / 4);
        let errands = Role::Merchant.errands(&catalog, &h).unwrap();
        assert_eq!(errands[0].choice(&catalog), Choice::Bank);
        assert_eq!(errands[1].goal, Some(Goal::hold("fiber", BANK_FIBER_AT / 4 + 1)));
        assert_eq!(Role::Merchant.product(), None);
        assert!(Role::Merchant.needs().is_empty());
        let items = merchant_items(7);
        assert_eq!(items, merchant_items(7));
        assert_eq!(items.len(), MERCHANT_ITEMS);
        assert!(items.iter().all(|i| MARKET_ITEMS.contains(i)));
        let distinct: std::collections::HashSet<_> = items.iter().collect();
        assert_eq!(distinct.len(), MERCHANT_ITEMS);
        // A town's merchants spread over the market
        let dealt: std::collections::HashSet<&str> = (0..8).flat_map(merchant_items).collect();
        assert_eq!(dealt.len(), MARKET_ITEMS.len());
    }

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
            strands: FLOAT,
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

    /// What a middling rules layer picks from `menu`, as the sim's bots
    /// decide since C1.3: the menu entry, or `None` for the other one.
    fn rules_pick(question: Question, menu: Vec<Choice>, h: &Holdings, bid: Option<i64>, ask: Option<i64>) -> Option<Choice> {
        let item = menu.iter().find_map(|c| match c {
            Choice::Sell { item, .. } | Choice::Buy { item, .. } => Some(item.clone()),
            _ => None,
        })?;
        let s = Situation {
            who: Who {
                name: "Bot".into(),
                role: "builder".into(),
                temperament: Temperament::default(),
            },
            at: Place { x: 0.0, y: 0.0, zone: "rural".into() },
            when: When { time_of_day: None, hours: 0.0 },
            holdings: h.into(),
            market: vec![Quote { item, bid, ask, last: None }],
            question,
            menu,
            carry_on: Some(1),
        };
        let ruling = Cascade::rules().ask(&s, std::time::Instant::now());
        let d = ruling.decision?;
        let mut choice = s.choice(&d).clone();
        // The parameters it set, in place of the spans
        match &mut choice {
            Choice::Sell { price, quantity, .. } | Choice::Buy { price, quantity, .. } => {
                *price = Span::new(d.price?, d.price?);
                *quantity = Span::new(d.quantity?, d.quantity?);
            }
            _ => return None,
        }
        Some(choice)
    }

    fn sold(item: &str, quantity: i32, price: i64) -> Option<Choice> {
        Some(Choice::Sell {
            item: item.into(),
            price: Span::new(price, price),
            quantity: Span::new(quantity, quantity),
        })
    }

    fn bought(item: &str, quantity: i32, price: i64) -> Option<Choice> {
        Some(Choice::Buy {
            item: item.into(),
            price: Span::new(price, price),
            quantity: Span::new(quantity, quantity),
        })
    }

    fn sell(role: Role, catalog: &Catalog, h: &Holdings, bid: Option<i64>) -> Option<Choice> {
        let product = role.product()?;
        let menu = role.sell_menu(h, Some(&role.goal(catalog, h)), bid, None)?;
        rules_pick(Question::Sell { item: product.into() }, menu, h, bid, None)
    }

    fn buy(role: Role, h: &Holdings, step: &Step, ask: Option<i64>) -> Option<Choice> {
        let menu = role.buy_menu(h, step, ask)?;
        let Step::Gather { item, .. } = step else { unreachable!() };
        rules_pick(Question::Buy { item: item.clone() }, menu, h, None, ask)
    }

    #[test]
    fn a_maker_of_something_sells_its_surplus_to_the_best_bid() {
        let catalog = Catalog::fallback();
        let mut h = Holdings {
            strands: FLOAT,
            ..Holdings::default()
        };
        h.items.insert("log".into(), SELL_AT);
        let wood = Role::Gatherer(Gather::Wood);
        assert_eq!(sell(wood, &catalog, &h, Some(7)), sold("log", SELL_AT, 7));
        // No bid, no sale; and under SELL_AT it keeps gathering
        assert_eq!(sell(wood, &catalog, &h, None), None);
        h.items.insert("log".into(), SELL_AT - 1);
        assert_eq!(sell(wood, &catalog, &h, Some(7)), None);
        // A builder still building keeps its bricks: there's no question
        h.items.insert("brick".into(), 40);
        assert_eq!(Role::Builder.sell_menu(&h, Some(&Role::Builder.goal(&catalog, &h)), Some(9), None), None);
    }

    #[test]
    fn a_bot_buys_what_it_would_gather_unless_gathering_it_is_its_job() {
        let mut h = Holdings {
            strands: 1000,
            plot_room: Some(3),
            deed_cost: 250,
            ..Holdings::default()
        };
        let ingots = Role::Crafter(Craft::Ingots);
        assert_eq!(buy(ingots, &h, &gather("log"), Some(8)), bought("log", LOT, 8));
        // The wood gatherer gathers its own logs
        assert_eq!(Role::Gatherer(Gather::Wood).buy_menu(&h, &gather("log"), Some(8)), None);
        // Not on the market: gather it
        assert_eq!(ingots.buy_menu(&h, &gather("fiber"), Some(8)), None);
        // No plot yet: the deed's price is kept back, and the floor, which
        // leaves 2 at 8
        h.plot_room = None;
        h.strands = 280;
        assert_eq!(buy(ingots, &h, &gather("log"), Some(8)), bought("log", 2, 8));
        h.strands = 255;
        assert_eq!(ingots.buy_menu(&h, &gather("log"), Some(8)), None);
    }

    #[test]
    fn a_maker_down_to_its_floor_earns_its_float_back_and_sells_what_it_can_pay_for() {
        let catalog = Catalog::fallback();
        let mut h = Holdings {
            strands: FLOOR - 1,
            plot_room: Some(3),
            deed_cost: 250,
            ..Holdings::default()
        };
        let wood = Role::Gatherer(Gather::Wood);
        let errands = wood.errands(&catalog, &h).unwrap();
        assert_eq!(errands[0].goal, Some(Goal::Strands(FLOAT)));
        assert_eq!(errands[0].step.label(&catalog), "gather fiber");
        assert_eq!(errands[1].goal, Some(Goal::hold("log", 1)));
        // At the floor it works as before; a fiber gatherer never tops up
        h.strands = FLOOR;
        assert_eq!(wood.goal(&catalog, &h), Goal::hold("log", 1));
        h.strands = 0;
        assert_eq!(Role::Gatherer(Gather::Fiber).goal(&catalog, &h), Goal::hold("fiber", 1));
        // Earning its float, it still sells to a bid, as many as it can pay
        // the fee on at the top of the span: 1 Strand covers 100 of value
        h.items.insert("log".into(), SELL_AT);
        let goal = Goal::Strands(FLOAT);
        assert_eq!(wood.sell_menu(&h, Some(&goal), Some(7), None), None, "nothing for the fee");
        h.strands = 1;
        let menu = wood.sell_menu(&h, Some(&goal), Some(7), None).unwrap();
        assert_eq!(
            menu[0],
            Choice::Sell {
                item: "log".into(),
                price: Span::new(3, 14),
                quantity: Span::new(1, 7),
            }
        );
        h.strands = FLOOR;
        assert!(matches!(&wood.sell_menu(&h, Some(&goal), Some(7), None).unwrap()[0], Choice::Sell { quantity, .. } if quantity.max == SELL_AT));
    }

    #[test]
    fn a_crafter_bids_for_what_its_work_takes_once() {
        let mut h = Holdings {
            strands: 1000,
            plot_room: Some(3),
            deed_cost: 250,
            ..Holdings::default()
        };
        let planks = Role::Crafter(Craft::Planks);
        let menu = planks.bid_menu(&h, "log", Some(8), false).unwrap();
        assert_eq!(
            menu[0],
            Choice::Bid {
                item: "log".into(),
                price: Span::new(4, 12),
                quantity: Span::new(1, 2 * LOT),
            }
        );
        // Not twice, not what it doesn't use, not without a worth
        assert_eq!(planks.bid_menu(&h, "log", Some(8), true), None);
        assert_eq!(planks.bid_menu(&h, "stone", Some(8), false), None);
        assert_eq!(planks.bid_menu(&h, "log", None, false), None);
        assert!(Role::Gatherer(Gather::Wood).needs().is_empty());
        // No plot yet: the deed's price is kept back
        h.plot_room = None;
        h.strands = 260;
        assert_eq!(planks.bid_menu(&h, "log", Some(8), false), None);
    }

    #[test]
    fn every_need_is_on_the_market_and_none_is_the_roles_own() {
        for (role, _) in MIX {
            for need in role.needs() {
                assert!(MARKET_ITEMS.contains(need), "{role} needs {need}");
                assert_ne!(role.product(), Some(*need), "{role}");
            }
        }
    }

    #[test]
    fn a_surplus_goes_to_a_stall_it_can_rent() {
        let mut h = Holdings {
            strands: 300,
            plot_room: None,
            deed_cost: 250,
            ..Holdings::default()
        };
        h.items.insert("log".into(), STALL_AT);
        let wood = Role::Gatherer(Gather::Wood);
        assert!(wood.stall_menu(&h, Some(10), false).is_some());
        // Not enough for the rent with a deed kept back, unless it keeps one
        h.strands = 280;
        assert_eq!(wood.stall_menu(&h, Some(10), false), None);
        assert!(wood.stall_menu(&h, Some(10), true).is_some());
        // Too few, or no worth to price them by
        h.items.insert("log".into(), STALL_AT / 2 - 1);
        assert_eq!(wood.stall_menu(&h, Some(10), true), None);
        h.items.insert("log".into(), STALL_AT);
        assert_eq!(wood.stall_menu(&h, None, true), None);
        assert_eq!(Role::Crafter(Craft::Tools).stall_menu(&h, Some(10), true), None);
    }

    /// C1.1's choice of step: bank at [`BANK_FIBER_AT`], else the first goal's.
    fn c1_1_step(role: Role, catalog: &Catalog, h: &Holdings) -> Option<Step> {
        if role == Role::Gatherer(Gather::Fiber) && h.count("fiber") >= BANK_FIBER_AT {
            return Some(Step::Deposit);
        }
        plan::next(catalog, h, &role.goal(catalog, h)).ok().flatten()
    }

    #[test]
    fn a_middling_rules_mind_works_on_what_c1_1_did() {
        let catalog = Catalog::fallback();
        let mut cases = 0;
        for (role, _) in MIX {
            for (fiber, strands, stations) in [(0, 0, 0), (49, 0, 0), (199, 0, 1), (200, 0, 2), (450, 300, 3), (60, 900, 6)] {
                let mut h = Holdings {
                    strands,
                    deed_cost: 250,
                    plot_room: (stations > 1).then_some(4),
                    ..Holdings::default()
                };
                h.items.insert("fiber".into(), fiber);
                h.stations.extend(BUILD.iter().take(stations).map(|s| s.to_string()));
                let Some(old) = c1_1_step(role, &catalog, &h) else { continue };
                let errands = role.errands(&catalog, &h).unwrap();
                let s = Situation {
                    who: Who {
                        name: "Bot".into(),
                        role: role.slug().into(),
                        temperament: Temperament::default(),
                    },
                    at: Place { x: 0.0, y: 0.0, zone: "rural".into() },
                    when: When { time_of_day: None, hours: 0.0 },
                    holdings: (&h).into(),
                    market: Vec::new(),
                    question: Question::Work,
                    menu: errands.iter().map(|e| e.choice(&catalog)).collect(),
                    carry_on: Some(0),
                };
                let pick = Cascade::rules().ask(&s, std::time::Instant::now()).decision.unwrap().pick;
                assert_eq!(errands[pick].step, old, "{role} with {fiber} fiber, {stations} stations");
                cases += 1;
            }
        }
        assert!(cases >= 30, "{cases}");
    }

    #[test]
    fn the_work_menu_is_banking_then_the_goals_in_order() {
        let catalog = Catalog::fallback();
        let mut h = Holdings::default();
        let fiber = Role::Gatherer(Gather::Fiber);
        let errands = fiber.errands(&catalog, &h).unwrap();
        assert_eq!(errands.len(), 1);
        assert_eq!(errands[0].goal, Some(Goal::hold("fiber", 1)));
        h.items.insert("fiber".into(), BANK_FIBER_AT / 4);
        let errands = fiber.errands(&catalog, &h).unwrap();
        assert_eq!(errands[0], Errand { goal: None, step: Step::Deposit });
        assert_eq!(errands[0].choice(&catalog), Choice::Bank);
        // Only a fiber gatherer banks
        assert!(Role::Builder.errands(&catalog, &h).unwrap().iter().all(|e| e.goal.is_some()));
        // A tool crafter lists every tool it lacks, then handles
        let h = Holdings {
            strands: FLOAT,
            ..Holdings::default()
        };
        let tools = Role::Crafter(Craft::Tools).errands(&catalog, &h).unwrap();
        assert_eq!(tools.first().and_then(|e| e.goal.clone()), Some(Goal::hold("stone_axe", 1)));
        assert_eq!(tools.last().and_then(|e| e.goal.clone()), Some(Goal::hold("tool_handle", 1)));
    }
}
