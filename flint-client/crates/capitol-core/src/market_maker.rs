//! Market makers (E3.8): bots that quote both sides of an item's book around
//! a reference price, so a market with few players still has someone to trade
//! with. `capitol-bot --market plank,iron_ingot` runs one, and the economy
//! sim's makers run the same loop.
//!
//! - **The reference** is the item's last trade or, before it has traded, its
//!   target price, D5's labour value, from the catalog (`target_price` in
//!   `items.toml`, D2.3). It follows the trades, but only within
//!   [`DRIFT`]× of where it started, so a run of one-sided fills can't walk it
//!   off.
//! - **Quotes** ([`quotes`]): `depth` levels a side. The first is half the
//!   spread from the centre, and each further one another half spread out. The
//!   centre leans against the position: holding more than the target moves
//!   both sides down, so it sells more and buys less. Holding less moves them
//!   up.
//! - **Bounds.** Bids never take the position past twice the target, nor spend
//!   Strands below the reserve. Asks never offer more than it holds.
//! - **Re-quoting** ([`plan`]): an order stays while its side and price are
//!   still wanted. A level that's gone is cancelled, and a missing or mostly
//!   filled one is placed. So a fill re-quotes only what moved, and the fee
//!   (1%, at least 1 Strand an order) is paid only on a change.
//!
//! **Funding** ([`fund`]) tops the bot up with Strands and stock through
//! `POST /player/grant`, which a server serves only with `DEV_GRANTS=on`.
//! It's for `capitol-bot --market` and the sim's makers only. A town's
//! merchant (C1.6) is never funded: it earns its float, and its quotes are
//! held to its own wealth by a [`Limit`] an item ([`share`]).
//!
//! **Where.** Orders are placed at the Exchange hall (D6), so the bot must be
//! there ([`Bot::go_to_exchange`]) before [`Maker::start`], and stays.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use capitol_protocol::rest::{GrantItem, GrantRequest, GrantResponse, MarketOrder, PlaceOrder};
use capitol_protocol::rules::market::{OrderSide, OrderStatus, MIN_PRICE};
use capitol_protocol::tunables::Fees;
use uuid::Uuid;

use crate::bot::{Bot, BotError};
use crate::market::MarketEvent;
use crate::state::StateEvent;

/// How far the reference may follow the trades from where it started, as a
/// factor either way.
pub const DRIFT: f64 = 3.0;
/// The least an order is worth, in Strands, so the 1-Strand minimum fee
/// stays near 1%. A level's size grows to reach it.
pub const MIN_ORDER_VALUE: i64 = 100;
/// How often every item is re-quoted even when nothing traded, for expiry
/// and anything the events missed.
pub const REQUOTE_EVERY: Duration = Duration::from_secs(30);
/// How long the loop waits between looks at the events.
const POLL: Duration = Duration::from_millis(500);

/// How a maker quotes.
#[derive(Debug, Clone, PartialEq)]
pub struct MakerConfig {
    pub items: Vec<String>,
    /// The whole spread as a fraction of the price: 0.1 quotes 5% either side
    pub spread: f64,
    /// Levels a side
    pub depth: usize,
    /// Units a level, at least (more for a cheap item: [`MIN_ORDER_VALUE`])
    pub size: i32,
    /// Units of each item it aims to hold. It bids up to twice this.
    pub target: i32,
    /// Strands it never commits to bids
    pub reserve: i64,
}

impl Default for MakerConfig {
    fn default() -> Self {
        MakerConfig {
            items: Vec::new(),
            spread: 0.1,
            depth: 2,
            size: 10,
            target: 40,
            reserve: 100,
        }
    }
}

/// One order a maker wants on the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    pub side: OrderSide,
    pub price: i64,
    pub quantity: i32,
}

/// A level's size at `price`: `size`, or enough to be worth [`MIN_ORDER_VALUE`].
fn level_size(size: i32, price: i64) -> i32 {
    let floor = (MIN_ORDER_VALUE + price - 1) / price.max(1);
    size.max(floor.min(i32::MAX as i64) as i32)
}

/// What a merchant lets one item's quotes hold (C1.6), in place of the
/// config's target and every Strand it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    /// Units it aims to hold. It bids up to twice this.
    pub target: i32,
    /// Strands the item's bids may hold, fees included.
    pub budget: i64,
}

/// A merchant's limit for an item at `reference`, its `wealth` shared evenly
/// over `items`: the share is the bids' budget, and half of it, in units, the
/// target. So it holds about half its share in stock and half in Strands,
/// and both grow with its wealth.
pub fn share(wealth: i64, reference: i64, items: usize) -> Limit {
    let share = (wealth / items.max(1) as i64).max(0);
    Limit {
        target: (share / 2 / reference.max(1)).clamp(1, i32::MAX as i64) as i32,
        budget: share,
    }
}

/// What the maker wants on the book for one item: bids best first, then
/// asks best first. `position` is every unit it holds, those on its asks
/// included, and `strands` every Strand, those on its bids included. Its bids
/// pay the listing fee at `fees`, the server's (D6.1).
pub fn quotes(config: &MakerConfig, reference: i64, position: i32, strands: i64, fees: &Fees) -> Vec<Quote> {
    let target = config.target.max(1);
    let skew = ((position - target) as f64 / target as f64).clamp(-1.0, 1.0);
    let half = (reference as f64 * config.spread / 2.0).max(0.5);
    let centre = reference as f64 - skew * half;
    let mut out = Vec::new();

    let mut room = (2 * target - position).max(0);
    let mut budget = (strands - config.reserve).max(0);
    let mut last = None;
    for k in 0..config.depth {
        // At the floor, the deeper levels collapse into the first
        let price = ((centre - half * (k + 1) as f64).floor() as i64).max(MIN_PRICE);
        if last == Some(price) {
            break;
        }
        let mut quantity = level_size(config.size, price).min(room);
        quantity = quantity.min((budget / price).min(i32::MAX as i64) as i32);
        while quantity > 0 && fees.buy_debit(price, quantity) > budget {
            quantity -= 1;
        }
        if quantity <= 0 {
            break;
        }
        out.push(Quote {
            side: OrderSide::Buy,
            price,
            quantity,
        });
        room -= quantity;
        budget -= fees.buy_debit(price, quantity);
        last = Some(price);
    }

    let best_bid = out.first().map(|q| q.price);
    let mut stock = position.max(0);
    let mut last = None;
    for k in 0..config.depth {
        let mut price = (centre + half * (k + 1) as f64).ceil() as i64;
        if let Some(bid) = best_bid {
            price = price.max(bid + 1);
        }
        if last.is_some_and(|l| price <= l) {
            price = last.unwrap_or(price) + 1;
        }
        let quantity = level_size(config.size, price).min(stock);
        if quantity <= 0 {
            break;
        }
        out.push(Quote {
            side: OrderSide::Sell,
            price,
            quantity,
        });
        stock -= quantity;
        last = Some(price);
    }
    out
}

/// What to change on the book: the orders to cancel, and the quotes to place.
/// An open order stays while its side and price are wanted. A wanted level
/// gets a new order for what it lacks once it holds half or less.
pub fn plan(wanted: &[Quote], open: &[&MarketOrder]) -> (Vec<Uuid>, Vec<Quote>) {
    let cancel = open
        .iter()
        .filter(|o| !wanted.iter().any(|w| w.side == o.side && w.price == o.price))
        .map(|o| o.id)
        .collect();
    let place = wanted
        .iter()
        .filter_map(|w| {
            let resting: i32 = open
                .iter()
                .filter(|o| o.side == w.side && o.price == w.price)
                .map(|o| o.quantity_remaining)
                .sum();
            (resting * 2 <= w.quantity).then_some(Quote {
                quantity: w.quantity - resting,
                ..*w
            })
        })
        .collect();
    (cancel, place)
}

/// Where an item's price is, and where it started.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reference {
    price: i64,
    anchor: i64,
}

impl Reference {
    fn new(price: i64) -> Reference {
        Reference { price, anchor: price }
    }

    /// Follow a trade at `price`, within [`DRIFT`] of the anchor.
    fn follow(&mut self, price: i64) {
        let lo = ((self.anchor as f64 / DRIFT).floor() as i64).max(MIN_PRICE);
        let hi = (self.anchor as f64 * DRIFT).ceil() as i64;
        self.price = price.clamp(lo, hi);
    }
}

/// What a maker did, for its last words.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MakerStats {
    pub placed: usize,
    pub cancelled: usize,
    /// Units bought and sold
    pub bought: i64,
    pub sold: i64,
}

/// Tops the bot up to `strands` and to `config.target` units of each item, at
/// quality `quality`, with a grant. Needs a server with `DEV_GRANTS=on`.
/// Returns `None` when it already had enough.
pub fn fund(bot: &mut Bot, config: &MakerConfig, strands: i64, quality: i32) -> Result<Option<GrantResponse>, BotError> {
    bot.fetch_market_box()?;
    bot.my_orders()?;
    let have = bot.state().strands().unwrap_or(0);
    let items: Vec<GrantItem> = config
        .items
        .iter()
        .filter_map(|item| {
            let short = config.target - position(bot, item);
            (short > 0).then(|| GrantItem {
                item_type: item.clone(),
                quantity: short,
                quality,
            })
        })
        .collect();
    let request = GrantRequest {
        strands: (strands - have).max(0),
        items,
    };
    if request.strands == 0 && request.items.is_empty() {
        return Ok(None);
    }
    let granted = bot.grant(request)?;
    bot.fetch_market_box()?;
    Ok(Some(granted))
}

/// Units of `item` the bot holds free: in its inventory and its market box.
fn free_units(bot: &Bot, item: &str) -> i32 {
    let state = bot.state();
    [state.inventory(), state.market_box()]
        .into_iter()
        .flatten()
        .flat_map(|c| c.items())
        .filter(|i| i.item_type == item)
        .map(|i| i.quantity)
        .sum()
}

/// The bot's open orders in `item`.
fn open_orders<'a>(bot: &'a Bot, item: &str) -> Vec<&'a MarketOrder> {
    bot.state()
        .market()
        .open_orders()
        .into_iter()
        .filter(|o| o.item_type == item && o.status == OrderStatus::Open)
        .collect()
}

/// Units of `item` free and on the bot's asks.
pub fn position(bot: &Bot, item: &str) -> i32 {
    let asked: i32 = open_orders(bot, item)
        .iter()
        .filter(|o| o.side == OrderSide::Sell)
        .map(|o| o.quantity_remaining)
        .sum();
    free_units(bot, item) + asked
}

/// The bot's Strands, those on its bids included.
pub fn all_strands(bot: &Bot) -> i64 {
    let escrowed: i64 = bot
        .state()
        .market()
        .open_orders()
        .iter()
        .filter(|o| o.side == OrderSide::Buy)
        .map(|o| o.escrow_strands)
        .sum();
    bot.state().strands().unwrap_or(0) + escrowed
}

/// One market maker: its config and where it thinks each price is.
pub struct Maker {
    config: MakerConfig,
    references: HashMap<String, Reference>,
    /// Each item's [`Limit`], where one is set: a merchant's (C1.6).
    limits: HashMap<String, Limit>,
    pub stats: MakerStats,
}

impl Maker {
    pub fn new(config: MakerConfig) -> Maker {
        Maker {
            config,
            references: HashMap::new(),
            limits: HashMap::new(),
            stats: MakerStats::default(),
        }
    }

    pub fn config(&self) -> &MakerConfig {
        &self.config
    }

    /// The reference price for `item`, once [`start`](Self::start) has run.
    pub fn reference(&self, item: &str) -> Option<i64> {
        self.references.get(item).map(|r| r.price)
    }

    /// Holds `item`'s quotes to `limit` from the next requote on.
    pub fn limit(&mut self, item: &str, limit: Limit) {
        self.limits.insert(item.to_string(), limit);
    }

    /// Watches every item, learns the bot's orders and box, and sets each
    /// reference from the last trade, or the catalog's target price. An item
    /// with neither is an error.
    pub fn start(&mut self, bot: &mut Bot) -> Result<(), BotError> {
        let summary = bot.market_summary()?;
        for item in &self.config.items {
            bot.watch_market(item)?;
            let last = summary.items.iter().find(|s| &s.item_type == item).and_then(|s| s.last_price);
            let price = last.or_else(|| bot.state().catalog().target_price(item)).ok_or_else(|| {
                BotError::Failed(format!("{item} has never traded and the catalog gives it no target price"))
            })?;
            self.references.insert(item.clone(), Reference::new(price));
        }
        bot.my_orders()?;
        bot.fetch_market_box()?;
        Ok(())
    }

    /// Quotes until `deadline` (or for ever), re-quoting an item when it
    /// trades or one of the maker's orders in it fills, and every item every
    /// [`REQUOTE_EVERY`].
    pub fn run(&mut self, bot: &mut Bot, deadline: Option<Instant>) -> Result<(), BotError> {
        self.run_while(bot, deadline, |_| true)
    }

    /// As [`run`](Self::run), and stops too once `keep` says no; it's asked
    /// every half second or so. A merchant stops for a trade request, or
    /// its town stopping.
    pub fn run_while(&mut self, bot: &mut Bot, deadline: Option<Instant>, mut keep: impl FnMut(&Bot) -> bool) -> Result<(), BotError> {
        let mut dirty: BTreeSet<String> = self.config.items.iter().cloned().collect();
        let mut box_stale = false;
        let mut last_sweep = Instant::now();
        while deadline.is_none_or(|d| Instant::now() < d) && keep(bot) {
            if box_stale {
                bot.fetch_market_box()?;
                box_stale = false;
            }
            for item in std::mem::take(&mut dirty) {
                self.requote(bot, &item)?;
            }
            for event in bot.idle(POLL)? {
                let StateEvent::Market(event) = event else { continue };
                match *event {
                    MarketEvent::Traded(trade) => {
                        if let Some(r) = self.references.get_mut(&trade.item_type) {
                            r.follow(trade.price);
                            dirty.insert(trade.item_type);
                        }
                    }
                    MarketEvent::OrderFilled { order, trade } => {
                        match order.side {
                            OrderSide::Buy => self.stats.bought += trade.quantity as i64,
                            OrderSide::Sell => self.stats.sold += trade.quantity as i64,
                        }
                        box_stale = true;
                        dirty.insert(order.item_type);
                    }
                    MarketEvent::OrderClosed(order) => {
                        box_stale = true;
                        dirty.insert(order.item_type);
                    }
                    _ => {}
                }
            }
            if last_sweep.elapsed() >= REQUOTE_EVERY {
                last_sweep = Instant::now();
                dirty.extend(self.config.items.iter().cloned());
                // Fills the events missed, and expiries
                bot.my_orders()?;
                box_stale = true;
            }
        }
        Ok(())
    }

    /// Brings `item`'s orders in line with [`quotes`].
    pub fn requote(&mut self, bot: &mut Bot, item: &str) -> Result<(), BotError> {
        let Some(reference) = self.reference(item) else {
            return Ok(());
        };
        let fees = bot.state().fees();
        let wanted = match self.limits.get(item) {
            None => quotes(&self.config, reference, position(bot, item), all_strands(bot), &fees),
            Some(limit) => {
                // Its own bids' Strands, and as many free as the budget allows
                let escrowed: i64 = open_orders(bot, item)
                    .iter()
                    .filter(|o| o.side == OrderSide::Buy)
                    .map(|o| o.escrow_strands)
                    .sum();
                let free = bot.state().strands().unwrap_or(0);
                let strands = (free + escrowed).min(limit.budget + self.config.reserve);
                let config = MakerConfig {
                    target: limit.target,
                    ..self.config.clone()
                };
                quotes(&config, reference, position(bot, item), strands, &fees)
            }
        };
        let (cancel, place) = plan(&wanted, &open_orders(bot, item));
        let mut returned = false;
        for id in cancel {
            let side = bot.state().market().order(id).map(|o| o.side);
            match bot.cancel_order(id) {
                Ok(_) => {
                    self.stats.cancelled += 1;
                    returned |= side == Some(OrderSide::Sell);
                }
                // Filled or closed meanwhile: the next look sees it
                Err(BotError::Rest { source, .. }) if source.status().is_some_and(|s| s < 500) => {}
                Err(e) => return Err(e),
            }
        }
        // A cancelled sell's units went back to the market box
        if returned {
            bot.fetch_market_box()?;
        }
        for quote in place {
            let order = match quote.side {
                OrderSide::Buy => PlaceOrder {
                    side: OrderSide::Buy,
                    item_type: item.to_string(),
                    price: quote.price,
                    quantity: quote.quantity,
                    item_id: None,
                    qualities: None,
                    min_grade: None,
                    grade_rule: None,
                },
                OrderSide::Sell => {
                    // From the biggest free stack, as much as it holds
                    let stack = [bot.state().market_box(), bot.state().inventory()]
                        .into_iter()
                        .flatten()
                        .flat_map(|c| c.items())
                        .filter(|i| i.item_type == item)
                        .max_by_key(|i| i.quantity)
                        .map(|i| (i.item_id, i.quantity));
                    let Some((item_id, held)) = stack else { continue };
                    PlaceOrder {
                        side: OrderSide::Sell,
                        item_type: item.to_string(),
                        price: quote.price,
                        quantity: quote.quantity.min(held),
                        item_id: Some(item_id),
                        qualities: None,
                        min_grade: None,
                        grade_rule: None,
                    }
                }
            };
            match bot.place_order(order) {
                Ok(_) => self.stats.placed += 1,
                // Short of Strands or units for now: the next look tries again
                Err(BotError::Rest { source, .. }) if source.status().is_some_and(|s| s < 500) => {
                    bot.log(format!("{item}: {source}"));
                }
                Err(e) => return Err(e),
            }
            if quote.side == OrderSide::Sell {
                // The stack it sold from changed
                bot.fetch_market_box()?;
            }
        }
        Ok(())
    }

    /// Cancels every open order in the maker's items, so its Strands and
    /// stock come home.
    pub fn withdraw(&mut self, bot: &mut Bot) -> Result<(), BotError> {
        bot.my_orders()?;
        for item in self.config.items.clone() {
            for id in open_orders(bot, &item).iter().map(|o| o.id).collect::<Vec<_>>() {
                match bot.cancel_order(id) {
                    Ok(_) => self.stats.cancelled += 1,
                    Err(BotError::Rest { source, .. }) if source.status().is_some_and(|s| s < 500) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::stacking::Units;
    use chrono::DateTime;

    fn config() -> MakerConfig {
        MakerConfig {
            items: vec!["plank".into()],
            spread: 0.2,
            depth: 2,
            size: 10,
            target: 40,
            reserve: 0,
        }
    }

    fn order(side: OrderSide, price: i64, remaining: i32) -> MarketOrder {
        let at = DateTime::from_timestamp(0, 0).unwrap();
        MarketOrder {
            // Distinct enough for a test
            id: Uuid::from_u128(((price as u128) << 8) | remaining as u128),
            player_id: Uuid::nil(),
            side,
            item_type: "plank".into(),
            price,
            quantity_total: 10,
            quantity_remaining: remaining,
            status: OrderStatus::Open,
            min_grade: None,
            grade_rule: None,
            units: Units::default(),
            escrow_strands: 0,
            fee: 1,
            created_at: at,
            expires_at: at,
        }
    }

    fn prices(quotes: &[Quote], side: OrderSide) -> Vec<i64> {
        quotes.iter().filter(|q| q.side == side).map(|q| q.price).collect()
    }

    #[test]
    fn every_item_the_sim_trades_has_a_target() {
        let catalog = crate::catalog::Catalog::fallback();
        for item in ["fiber", "log", "plank", "iron_ore", "iron_ingot", "charcoal", "brick", "stone", "rope"] {
            assert!(catalog.target_price(item).is_some(), "{item}");
        }
        assert_eq!(catalog.target_price("iron_ingot"), Some(50));
        assert_eq!(catalog.target_price("unobtainium"), None);
    }

    #[test]
    fn at_the_target_it_quotes_evenly_round_the_reference() {
        let q = quotes(&config(), 50, 40, 100_000, &Fees::default());
        // 20% spread: 5 either side, then another 5
        assert_eq!(prices(&q, OrderSide::Buy), [45, 40]);
        assert_eq!(prices(&q, OrderSide::Sell), [55, 60]);
        assert!(q.iter().all(|q| q.quantity == 10));
    }

    #[test]
    fn holding_too_much_leans_down_and_too_little_leans_up() {
        let long = quotes(&config(), 50, 80, 100_000, &Fees::default());
        let short = quotes(&config(), 50, 0, 100_000, &Fees::default());
        // At twice the target there's no room to bid at all
        assert!(prices(&long, OrderSide::Buy).is_empty());
        assert_eq!(prices(&long, OrderSide::Sell), [50, 55]);
        // Nothing to sell, and the bids lean up
        assert!(prices(&short, OrderSide::Sell).is_empty());
        assert_eq!(prices(&short, OrderSide::Buy), [50, 45]);
    }

    #[test]
    fn bids_stay_within_the_room_and_the_budget() {
        let mut c = config();
        c.reserve = 100;
        // 35 units of room and 400 spendable Strands at 44: 9 units and the fee
        // spend it all, so the second level gets nothing
        let q = quotes(&c, 50, 45, 500, &Fees::default());
        let bids: Vec<&Quote> = q.iter().filter(|q| q.side == OrderSide::Buy).collect();
        assert_eq!(bids.len(), 1);
        assert_eq!((bids[0].price, bids[0].quantity), (44, 9));
        assert!(Fees::default().buy_debit(bids[0].price, bids[0].quantity) <= 400);
        // Asks offer no more than it holds
        let asked: i32 = q.iter().filter(|q| q.side == OrderSide::Sell).map(|q| q.quantity).sum();
        assert_eq!(asked, 20);
        let thin = quotes(&c, 50, 15, 100_000, &Fees::default());
        assert_eq!(thin.iter().filter(|q| q.side == OrderSide::Sell).map(|q| q.quantity).sum::<i32>(), 15);
    }

    #[test]
    fn a_cheap_item_quotes_at_the_floor_with_lots_worth_the_fee() {
        let mut c = config();
        c.target = 200;
        let q = quotes(&c, 1, 200, 100_000, &Fees::default());
        // Fiber at 1: one bid at the floor, asks above it
        assert_eq!(prices(&q, OrderSide::Buy), [1]);
        assert_eq!(prices(&q, OrderSide::Sell), [2, 3]);
        assert_eq!(q[0].quantity, 100);
        // Each ask is worth at least the minimum order value
        assert!(q.iter().filter(|q| q.side == OrderSide::Sell).all(|q| q.price * q.quantity as i64 >= MIN_ORDER_VALUE));
    }

    #[test]
    fn a_requote_keeps_what_still_fits_and_tops_up_what_filled() {
        let wanted = quotes(&config(), 50, 40, 100_000, &Fees::default());
        let kept = order(OrderSide::Buy, 45, 10);
        let filled = order(OrderSide::Sell, 55, 3);
        let stale = order(OrderSide::Sell, 70, 10);
        let (cancel, place) = plan(&wanted, &[&kept, &filled, &stale]);
        assert_eq!(cancel, [stale.id]);
        assert_eq!(
            place,
            [
                Quote {
                    side: OrderSide::Buy,
                    price: 40,
                    quantity: 10
                },
                Quote {
                    side: OrderSide::Sell,
                    price: 55,
                    quantity: 7
                },
                Quote {
                    side: OrderSide::Sell,
                    price: 60,
                    quantity: 10
                },
            ]
        );
        // Over half left: kept as is
        let mostly = order(OrderSide::Sell, 55, 6);
        let (_, place) = plan(&wanted, &[&kept, &mostly]);
        assert!(!place.iter().any(|q| q.side == OrderSide::Sell && q.price == 55));
    }

    #[test]
    fn a_merchants_share_grows_with_its_wealth() {
        // 600 over three goods: 200 a good, half of it in logs at 8
        assert_eq!(share(600, 8, 3), Limit { target: 12, budget: 200 });
        assert_eq!(share(1200, 8, 3), Limit { target: 25, budget: 400 });
        // Dear goods still get a unit to aim for
        assert_eq!(share(600, 500, 3).target, 1);
        assert_eq!(share(-5, 8, 3), Limit { target: 1, budget: 0 });
        // Quoted at the limit, its bids stay within the budget
        let mut c = config();
        c.reserve = 0;
        let limit = share(600, 8, 3);
        c.target = limit.target;
        let spent: i64 = quotes(&c, 8, 0, limit.budget, &Fees::default())
            .iter()
            .filter(|q| q.side == OrderSide::Buy)
            .map(|q| Fees::default().buy_debit(q.price, q.quantity))
            .sum();
        assert!(spent > 0 && spent <= limit.budget, "{spent}");
    }

    #[test]
    fn the_reference_follows_trades_only_so_far() {
        let mut r = Reference::new(50);
        r.follow(60);
        assert_eq!(r.price, 60);
        r.follow(1000);
        assert_eq!(r.price, 150);
        r.follow(1);
        assert_eq!(r.price, 16);
        let mut floor = Reference::new(1);
        floor.follow(0);
        assert_eq!(floor.price, 1);
    }
}
