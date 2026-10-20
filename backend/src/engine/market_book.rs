//! The market's order books in memory (E3.2, economy plan D7): one per item
//! type, loaded at startup from the open orders and kept in step as orders are
//! placed, cancelled, filled and expired. It holds no lock across a DB await,
//! and never touches the DB itself: it proposes fills, and
//! [`super::market_matcher`] writes each one before the book [`Book::apply`]s it.
//!
//! - **Priority.** Bids by price high to low, then time; asks low to high,
//!   then time. Time is the order's `created_at`, then its id.
//! - **Matching.** Each new order, in the order they came to the book, walks
//!   the other side for as long as prices cross, meeting only orders that came
//!   before it: those resting when it arrived. So a batch of orders arriving
//!   in one second trades as if each came alone. It fills against the first
//!   order whose units meet the buy's grade (`rules::market::fill_units`),
//!   walking past those that don't without changing their place. A fill
//!   trades at the resting order's price.
//! - **Self-trade is refused.** An order walks past its own player's orders on
//!   the other side, and both rest. A wash trade would only pay fees, but it
//!   would paint the chart and the Gazette (E3.9) with trades nobody made.
//! - **Expired and stalled orders don't trade.** Expired ones wait for the
//!   matcher to close them. A buy whose market box was full is stalled for
//!   [`STALL_SECS`], then tried again.
//!
//! - **Batches.** [`Book::propose`] applies fills before they're written, so
//!   several can be written side by side, and [`Book::rewind`] takes back
//!   those after one that fails (#237). Orders placed and cancelled meanwhile
//!   wait for [`Book::settle`].
//!
//! A fill's eligibility doesn't depend on how much is wanted, and taking units
//! never makes a lot meet a grade it didn't (`fill_units`). So once the queue
//! is empty no two resting orders can fill, and only new ones are matched.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use capitol_protocol::rest::{DepthLevel, MarketOrder};
use capitol_protocol::rules::market::{self, fill_units, GradeRule, OrderSide, OrderStatus};
use capitol_protocol::rules::quality_grade;
use capitol_protocol::rules::stacking::Units;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// How long a stalled order sits out before it's tried again.
pub const STALL_SECS: i64 = 30;

/// An open order as the book holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct BookOrder {
    pub id: Uuid,
    pub player_id: Uuid,
    pub side: OrderSide,
    pub item_type: String,
    pub price: i64,
    pub remaining: i32,
    /// A sell's units in escrow; empty on a buy
    pub units: Units,
    /// A buy's grade and rule; `None` on a sell
    pub grade: Option<(String, GradeRule)>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl BookOrder {
    /// The book's copy of `order`, or `None` if it isn't open.
    pub fn from_order(order: &MarketOrder) -> Option<BookOrder> {
        if order.status != OrderStatus::Open || order.quantity_remaining < 1 {
            return None;
        }
        let grade = match order.side {
            OrderSide::Buy => Some((order.min_grade.clone()?, order.grade_rule?)),
            OrderSide::Sell => None,
        };
        Some(BookOrder {
            id: order.id,
            player_id: order.player_id,
            side: order.side,
            item_type: order.item_type.clone(),
            price: order.price,
            remaining: order.quantity_remaining,
            units: order.units.clone(),
            grade,
            created_at: order.created_at,
            expires_at: order.expires_at,
        })
    }

    fn key(&self) -> Key {
        Key {
            price: match self.side {
                OrderSide::Buy => -self.price,
                OrderSide::Sell => self.price,
            },
            at: self.created_at,
            id: self.id,
        }
    }

    fn time(&self) -> (DateTime<Utc>, Uuid) {
        (self.created_at, self.id)
    }
}

/// Where an order stands on its side: the best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    /// The price, negated on a bid so the highest sorts first
    price: i64,
    at: DateTime<Utc>,
    id: Uuid,
}

#[derive(Debug, Default)]
struct Sides {
    bids: BTreeSet<Key>,
    asks: BTreeSet<Key>,
}

/// A trade the book proposes: `quantity` units, exactly `units`, from `sell`
/// to `buy` at `price` each.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub buy: Uuid,
    pub sell: Uuid,
    pub buyer: Uuid,
    pub seller: Uuid,
    pub item_type: String,
    /// The resting order's price
    pub price: i64,
    /// What the buy escrowed per unit; over `price`, the buyer gets the rest back
    pub buy_price: i64,
    pub quantity: i32,
    pub units: Units,
}

/// What one change to the book overwrote, so [`Book::rewind`] can put it back.
#[derive(Debug)]
enum Was {
    /// The queue's front, popped
    Front(Uuid),
    /// The queue's back was pushed
    Back,
    Queued(Uuid, bool),
    Woken(Uuid, bool),
    Stalled(Uuid, Option<DateTime<Utc>>),
    Arrived(Uuid, Option<u64>),
    /// The order as it stood, or `None` if it wasn't on the book
    Order(Uuid, Option<BookOrder>),
}

/// Every open order, by item type.
#[derive(Debug, Default)]
pub struct Book {
    orders: HashMap<Uuid, BookOrder>,
    sides: HashMap<String, Sides>,
    /// New orders still to be matched, in the order they came
    queue: VecDeque<Uuid>,
    queued: HashSet<Uuid>,
    /// Orders sitting out until then
    stalled: HashMap<Uuid, DateTime<Utc>>,
    /// When each order came to the book: an order matches only those that
    /// came before it, the ones resting when it arrived
    arrived: HashMap<Uuid, u64>,
    next_arrival: u64,
    /// Stalled orders back in the queue: they match every order, since newer
    /// ones walked past them while they sat out
    woken: HashSet<Uuid>,
    /// Item types whose depth may have changed since [`Book::take_dirty`]
    dirty: HashSet<String>,
    /// While the matcher speculates (#237), what each change overwrote
    journal: Option<Vec<Was>>,
    /// Orders placed and cancelled while it speculates, for [`Book::settle`]:
    /// a rewind mustn't undo them, nor theirs a rewind
    later: Vec<Later>,
}

/// A change from outside the matcher, held while it speculates.
#[derive(Debug)]
enum Later {
    Insert(BookOrder),
    Remove(Uuid),
}

impl Book {
    pub fn new() -> Self {
        Self::default()
    }

    /// A book of `orders`, every one queued to match in time order, so a
    /// crossing left by a crash is matched on startup.
    pub fn load(mut orders: Vec<BookOrder>) -> Self {
        orders.sort_by_key(BookOrder::time);
        let mut book = Book::new();
        for order in orders {
            book.insert(order);
        }
        book
    }

    pub fn len(&self) -> usize {
        self.orders.len()
    }

    pub fn get(&self, id: Uuid) -> Option<&BookOrder> {
        self.orders.get(&id)
    }

    /// Put `order` on the book and queue it to match. One already there is
    /// replaced, keeping its place in the queue. While speculating it's done
    /// at [`Book::settle`].
    pub fn insert(&mut self, order: BookOrder) {
        if self.journal.is_some() {
            self.later.push(Later::Insert(order));
            return;
        }
        if let Some(old) = self.orders.remove(&order.id) {
            if let Some(sides) = self.sides.get_mut(&old.item_type) {
                sides.bids.remove(&old.key());
                sides.asks.remove(&old.key());
            }
            self.dirty.insert(old.item_type);
        }
        self.dirty.insert(order.item_type.clone());
        let sides = self.sides.entry(order.item_type.clone()).or_default();
        match order.side {
            OrderSide::Buy => sides.bids.insert(order.key()),
            OrderSide::Sell => sides.asks.insert(order.key()),
        };
        if let Entry::Vacant(e) = self.arrived.entry(order.id) {
            e.insert(self.next_arrival);
            self.next_arrival += 1;
        }
        if self.queued.insert(order.id) {
            self.queue.push_back(order.id);
        }
        self.orders.insert(order.id, order);
    }

    /// Take order `id` off the book (cancelled, expired, or found closed).
    /// While speculating it's done at [`Book::settle`], and this returns the
    /// order as it stands.
    pub fn remove(&mut self, id: Uuid) -> Option<BookOrder> {
        if self.journal.is_some() {
            self.later.push(Later::Remove(id));
            return self.orders.get(&id).cloned();
        }
        self.take(id)
    }

    fn take(&mut self, id: Uuid) -> Option<BookOrder> {
        self.note(|b| Was::Order(id, b.orders.get(&id).cloned()));
        let order = self.orders.remove(&id)?;
        if let Some(sides) = self.sides.get_mut(&order.item_type) {
            match order.side {
                OrderSide::Buy => sides.bids.remove(&order.key()),
                OrderSide::Sell => sides.asks.remove(&order.key()),
            };
        }
        self.dirty.insert(order.item_type.clone());
        self.set_stalled(id, None);
        self.note(|b| Was::Arrived(id, b.arrived.get(&id).copied()));
        self.arrived.remove(&id);
        self.set_woken(id, false);
        // Its queue entry is skipped when it comes up
        self.set_queued(id, false);
        Some(order)
    }

    /// Sit order `id` out until `now` + [`STALL_SECS`]. A buy whose market box
    /// is full stays on the book, but doesn't trade until then.
    pub fn stall(&mut self, id: Uuid, now: DateTime<Utc>) {
        if self.orders.contains_key(&id) {
            self.set_stalled(id, Some(now + Duration::seconds(STALL_SECS)));
        }
    }

    /// Start remembering every change, so the book can be put back as it was
    /// at any [`Book::mark`] since (#237). The matcher applies a batch of fills
    /// before they're written, and rewinds past one that fails.
    pub fn speculate(&mut self) {
        self.journal = Some(Vec::new());
    }

    /// Where the book stands now, to [`Book::rewind`] to.
    pub fn mark(&self) -> usize {
        self.journal.as_ref().map_or(0, Vec::len)
    }

    /// Put the book back as it was at `mark`, undoing every change since.
    /// Depth may be marked changed when it isn't, which costs an empty diff.
    pub fn rewind(&mut self, mark: usize) {
        let Some(mut journal) = self.journal.take() else { return };
        while journal.len() > mark {
            let Some(was) = journal.pop() else { break };
            match was {
                Was::Front(id) => self.queue.push_front(id),
                Was::Back => {
                    self.queue.pop_back();
                }
                Was::Queued(id, on) => toggle(&mut self.queued, id, on),
                Was::Woken(id, on) => toggle(&mut self.woken, id, on),
                Was::Stalled(id, until) => {
                    match until {
                        Some(until) => self.stalled.insert(id, until),
                        None => self.stalled.remove(&id),
                    };
                }
                Was::Arrived(id, at) => {
                    match at {
                        Some(at) => self.arrived.insert(id, at),
                        None => self.arrived.remove(&id),
                    };
                }
                Was::Order(id, order) => {
                    if let Some(now) = self.orders.remove(&id) {
                        if let Some(sides) = self.sides.get_mut(&now.item_type) {
                            sides.bids.remove(&now.key());
                            sides.asks.remove(&now.key());
                        }
                        self.dirty.insert(now.item_type);
                    }
                    if let Some(order) = order {
                        self.dirty.insert(order.item_type.clone());
                        let sides = self.sides.entry(order.item_type.clone()).or_default();
                        match order.side {
                            OrderSide::Buy => sides.bids.insert(order.key()),
                            OrderSide::Sell => sides.asks.insert(order.key()),
                        };
                        self.orders.insert(id, order);
                    }
                }
            }
        }
        self.journal = Some(journal);
    }

    /// Stop remembering: what's been applied stays, and the orders placed and
    /// cancelled meanwhile go on and off the book.
    pub fn settle(&mut self) {
        self.journal = None;
        for later in std::mem::take(&mut self.later) {
            match later {
                Later::Insert(order) => self.insert(order),
                Later::Remove(id) => {
                    self.take(id);
                }
            }
        }
    }

    /// Remember what a change is about to overwrite, if speculating.
    fn note(&mut self, was: impl FnOnce(&Self) -> Was) {
        if self.journal.is_some() {
            let was = was(self);
            if let Some(journal) = &mut self.journal {
                journal.push(was);
            }
        }
    }

    /// Put `id` in the queued set or take it out; whether that changed it.
    fn set_queued(&mut self, id: Uuid, on: bool) -> bool {
        self.note(|b| Was::Queued(id, b.queued.contains(&id)));
        if on {
            self.queued.insert(id)
        } else {
            self.queued.remove(&id)
        }
    }

    fn set_woken(&mut self, id: Uuid, on: bool) {
        self.note(|b| Was::Woken(id, b.woken.contains(&id)));
        toggle(&mut self.woken, id, on);
    }

    fn set_stalled(&mut self, id: Uuid, until: Option<DateTime<Utc>>) {
        self.note(|b| Was::Stalled(id, b.stalled.get(&id).copied()));
        match until {
            Some(until) => self.stalled.insert(id, until),
            None => self.stalled.remove(&id),
        };
    }

    fn push_queue(&mut self, id: Uuid) {
        self.note(|_| Was::Back);
        self.queue.push_back(id);
    }

    fn pop_queue(&mut self) {
        if let Some(id) = self.queue.pop_front() {
            self.note(|_| Was::Front(id));
        }
    }

    /// The orders past their expiry, oldest first.
    pub fn expired(&self, now: DateTime<Utc>) -> Vec<Uuid> {
        let mut gone: Vec<&BookOrder> = self.orders.values().filter(|o| o.expires_at <= now).collect();
        gone.sort_by_key(|o| o.time());
        gone.into_iter().map(|o| o.id).collect()
    }

    /// The stalled orders whose time is up: queue them to match again.
    fn wake(&mut self, now: DateTime<Utc>) {
        let due: Vec<Uuid> = self.stalled.iter().filter(|(_, until)| **until <= now).map(|(id, _)| *id).collect();
        for &id in &due {
            self.set_stalled(id, None);
        }
        let mut woken: Vec<&BookOrder> = due.iter().filter_map(|id| self.orders.get(id)).collect();
        woken.sort_by_key(|o| o.time());
        let ids: Vec<Uuid> = woken.into_iter().map(|o| o.id).collect();
        for id in ids {
            self.set_woken(id, true);
            if self.set_queued(id, true) {
                self.push_queue(id);
            }
        }
    }

    /// The next fill, or `None` when nothing more can trade. The order being
    /// matched stays at the front of the queue until it has no fill left, so
    /// call [`Book::apply`] (or [`Book::remove`], [`Book::stall`]) on what
    /// this returns before asking again.
    pub fn next_fill(&mut self, now: DateTime<Utc>) -> Option<Fill> {
        self.wake(now);
        while let Some(&id) = self.queue.front() {
            if self.queued.contains(&id) {
                if let Some(fill) = self.fill_for(id, now) {
                    return Some(fill);
                }
            }
            self.pop_queue();
            self.set_queued(id, false);
            self.set_woken(id, false);
        }
        None
    }

    /// Speculate (see [`Book::speculate`]) and take the next fills, up to
    /// `width`, that can be written side by side (#237): those
    /// [`Book::next_fill`] makes one after another, each applied, for as long
    /// as no player is in two of them. A player's box and balance are theirs
    /// alone, so fills that share none share no row, and none waits on
    /// another's lock. Each comes with the mark to rewind to if it isn't
    /// written: the book as it stood just before it was applied. So a batch is
    /// exactly what fills one at a time would be, up to the first that fails.
    ///
    /// Call [`Book::settle`] once the batch is written, or rewound, before
    /// asking again. An empty batch leaves the book speculating too.
    pub fn propose(&mut self, now: DateTime<Utc>, width: usize) -> Vec<(Fill, usize)> {
        self.speculate();
        let mut busy = HashSet::new();
        let mut batch = Vec::new();
        while batch.len() < width {
            let mark = self.mark();
            let Some(fill) = self.next_fill(now) else { break };
            if busy.contains(&fill.buyer) || busy.contains(&fill.seller) {
                // Its turn comes in the next batch
                self.rewind(mark);
                break;
            }
            busy.extend([fill.buyer, fill.seller]);
            let applied = self.mark();
            self.apply(&fill);
            batch.push((fill, applied));
        }
        batch
    }

    fn can_trade(&self, order: &BookOrder, now: DateTime<Utc>) -> bool {
        order.remaining > 0 && order.expires_at > now && !self.stalled.contains_key(&order.id)
    }

    /// The first fill order `id` makes, walking the other side in priority.
    fn fill_for(&self, id: Uuid, now: DateTime<Utc>) -> Option<Fill> {
        let taker = self.orders.get(&id)?;
        if !self.can_trade(taker, now) {
            return None;
        }
        let sides = self.sides.get(&taker.item_type)?;
        let arrived = |o: &BookOrder| self.arrived.get(&o.id).copied().unwrap_or(u64::MAX);
        let against_all = self.woken.contains(&id);
        let other = match taker.side {
            OrderSide::Buy => &sides.asks,
            OrderSide::Sell => &sides.bids,
        };
        for key in other {
            let maker = &self.orders[&key.id];
            let (buy, sell) = match taker.side {
                OrderSide::Buy => (taker, maker),
                OrderSide::Sell => (maker, taker),
            };
            if buy.price < sell.price {
                // Everything further on is further from crossing
                break;
            }
            // A newer order meets this one when its own turn comes
            let resting = against_all || arrived(maker) < arrived(taker);
            if !resting || maker.player_id == taker.player_id || !self.can_trade(maker, now) {
                continue;
            }
            let Some((grade, rule)) = &buy.grade else { continue };
            let Some(units) = fill_units(&sell.units, buy.remaining.min(sell.remaining), grade, *rule) else {
                continue;
            };
            let first = if arrived(buy) < arrived(sell) { buy } else { sell };
            return Some(Fill {
                buy: buy.id,
                sell: sell.id,
                buyer: buy.player_id,
                seller: sell.player_id,
                item_type: taker.item_type.clone(),
                price: first.price,
                buy_price: buy.price,
                quantity: units.count() as i32,
                units,
            });
        }
        None
    }

    /// Record `fill`, now that it's written: both orders lose its quantity,
    /// the sell its units, and an order with none left leaves the book.
    pub fn apply(&mut self, fill: &Fill) {
        self.dirty.insert(fill.item_type.clone());
        for id in [fill.buy, fill.sell] {
            self.note(|b| Was::Order(id, b.orders.get(&id).cloned()));
            let Some(order) = self.orders.get_mut(&id) else { continue };
            order.remaining -= fill.quantity;
            if order.side == OrderSide::Sell {
                order.units.take_units(&fill.units);
            }
            if order.remaining <= 0 {
                self.take(id);
            }
        }
    }

    /// Every order of `item_type` on `side`, best first: the depth E3.3 serves.
    pub fn side(&self, item_type: &str, side: OrderSide) -> Vec<&BookOrder> {
        let Some(sides) = self.sides.get(item_type) else { return Vec::new() };
        let keys = match side {
            OrderSide::Buy => &sides.bids,
            OrderSide::Sell => &sides.asks,
        };
        keys.iter().map(|k| &self.orders[&k.id]).collect()
    }

    /// The item types whose depth may have changed since the last call.
    pub fn take_dirty(&mut self) -> Vec<String> {
        let mut items: Vec<String> = self.dirty.drain().collect();
        items.sort();
        items
    }

    /// Every item type with an order on the book.
    pub fn item_types(&self) -> Vec<String> {
        let mut items: Vec<String> = self
            .sides
            .iter()
            .filter(|(_, s)| !s.bids.is_empty() || !s.asks.is_empty())
            .map(|(item, _)| item.clone())
            .collect();
        items.sort();
        items
    }

    /// `item_type`'s book, aggregated by price and grade, in book order
    /// (`rules::market::sort_levels`): a buy at its `min_grade` and rule, a
    /// sell at the grade of each of its units. Expired and stalled orders
    /// count until they leave the book.
    pub fn depth(&self, item_type: &str) -> Vec<DepthLevel> {
        type LevelKey = (OrderSide, i64, &'static str, Option<GradeRule>);
        let mut levels: HashMap<LevelKey, (i64, i32)> = HashMap::new();
        for side in [OrderSide::Buy, OrderSide::Sell] {
            for order in self.side(item_type, side) {
                match &order.grade {
                    Some((grade, rule)) => {
                        let Some(grade) = market::grade_floor(grade).map(quality_grade) else { continue };
                        let level = levels.entry((side, order.price, grade, Some(*rule))).or_default();
                        level.0 += order.remaining as i64;
                        level.1 += 1;
                    }
                    None => {
                        let mut grades: HashMap<&'static str, i64> = HashMap::new();
                        for (quality, count) in order.units.pairs() {
                            *grades.entry(quality_grade(quality)).or_default() += count as i64;
                        }
                        for (grade, count) in grades {
                            let level = levels.entry((side, order.price, grade, None)).or_default();
                            level.0 += count;
                            level.1 += 1;
                        }
                    }
                }
            }
        }
        let mut out: Vec<DepthLevel> = levels
            .into_iter()
            .map(|((side, price, grade, grade_rule), (quantity, orders))| DepthLevel {
                side,
                price,
                grade: grade.to_string(),
                grade_rule,
                quantity,
                orders,
            })
            .collect();
        market::sort_levels(&mut out);
        out
    }
}

fn toggle(set: &mut HashSet<Uuid>, id: Uuid, on: bool) {
    if on {
        set.insert(id);
    } else {
        set.remove(&id);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    pub(crate) fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    /// Builds orders one second apart, so each is later than the last.
    pub(crate) struct Orders {
        at: i64,
    }

    impl Orders {
        pub(crate) fn new() -> Self {
            Orders { at: 0 }
        }

        fn next(&mut self, player: u128, side: OrderSide, price: i64, quantity: i32) -> BookOrder {
            self.at += 1;
            let created_at = t0() + Duration::seconds(self.at);
            BookOrder {
                id: Uuid::new_v4(),
                player_id: Uuid::from_u128(player),
                side,
                item_type: "rope".into(),
                price,
                remaining: quantity,
                units: Units::new(),
                grade: None,
                created_at,
                expires_at: created_at + Duration::days(7),
            }
        }

        pub(crate) fn sell(&mut self, player: u128, price: i64, units: Units) -> BookOrder {
            let quantity = units.count() as i32;
            BookOrder { units, ..self.next(player, OrderSide::Sell, price, quantity) }
        }

        pub(crate) fn buy(&mut self, player: u128, price: i64, quantity: i32, grade: &str, rule: GradeRule) -> BookOrder {
            BookOrder { grade: Some((grade.into(), rule)), ..self.next(player, OrderSide::Buy, price, quantity) }
        }
    }

    /// Match everything, applying each fill as if it were written
    pub(crate) fn run(book: &mut Book, now: DateTime<Utc>) -> Vec<Fill> {
        let mut fills = Vec::new();
        while let Some(fill) = book.next_fill(now) {
            book.apply(&fill);
            fills.push(fill);
        }
        fills
    }

    fn now() -> DateTime<Utc> {
        t0() + Duration::hours(1)
    }

    #[test]
    fn a_buy_at_or_over_the_ask_trades_at_the_asks_price() {
        let mut o = Orders::new();
        let mut book = Book::new();
        let ask = o.sell(1, 10, Units::uniform(5, 60));
        book.insert(ask.clone());
        assert!(run(&mut book, now()).is_empty(), "one order alone doesn't trade");

        // Under the ask: nothing
        book.insert(o.buy(2, 9, 5, "F", GradeRule::Average));
        assert!(run(&mut book, now()).is_empty());
        // At 12, over the ask: fills at the ask's 10
        let bid = o.buy(3, 12, 5, "F", GradeRule::Average);
        book.insert(bid.clone());
        let fills = run(&mut book, now());
        assert_eq!(fills.len(), 1);
        assert_eq!((fills[0].price, fills[0].buy_price, fills[0].quantity), (10, 12, 5));
        assert_eq!((fills[0].buy, fills[0].sell), (bid.id, ask.id));
        // Both done; the 9 still rests
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn a_sell_into_a_resting_bid_trades_at_the_bids_price() {
        let mut o = Orders::new();
        let mut book = Book::new();
        book.insert(o.buy(1, 15, 3, "F", GradeRule::Average));
        book.insert(o.sell(2, 11, Units::uniform(3, 50)));
        let fills = run(&mut book, now());
        assert_eq!(fills.iter().map(|f| f.price).collect::<Vec<_>>(), vec![15]);
    }

    #[test]
    fn sell_100_against_buy_50_leaves_50() {
        let mut o = Orders::new();
        let mut book = Book::new();
        let bid = o.buy(1, 20, 50, "F", GradeRule::Average);
        let ask = o.sell(2, 20, Units::from_pairs([(30, 40), (70, 60)]));
        book.insert(bid.clone());
        book.insert(ask.clone());
        let fills = run(&mut book, now());
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].quantity, 50);
        // The lowest 50 go
        assert_eq!(fills[0].units, Units::from_pairs([(30, 40), (70, 10)]));
        assert!(book.get(bid.id).is_none());
        let left = book.get(ask.id).unwrap();
        assert_eq!((left.remaining, left.units.clone()), (50, Units::uniform(50, 70)));
    }

    #[test]
    fn a_big_buy_sweeps_the_asks_best_price_first() {
        let mut o = Orders::new();
        let mut book = Book::new();
        for price in [14, 10, 12, 10, 30] {
            book.insert(o.sell(1, price, Units::uniform(4, 50)));
        }
        book.insert(o.buy(2, 14, 15, "F", GradeRule::Average));
        let fills = run(&mut book, now());
        let traded: Vec<(i64, i32)> = fills.iter().map(|f| (f.price, f.quantity)).collect();
        assert_eq!(traded, vec![(10, 4), (10, 4), (12, 4), (14, 3)]);
        // One unit of the 14 and all of the 30 still rest
        let asks: Vec<(i64, i32)> = book.side("rope", OrderSide::Sell).iter().map(|o| (o.price, o.remaining)).collect();
        assert_eq!(asks, vec![(14, 1), (30, 4)]);
    }

    #[test]
    fn at_one_price_the_earlier_order_trades_first() {
        let mut o = Orders::new();
        let mut book = Book::new();
        let first = o.buy(1, 10, 2, "F", GradeRule::Average);
        let second = o.buy(2, 10, 2, "F", GradeRule::Average);
        let better = o.buy(3, 11, 2, "F", GradeRule::Average);
        // Inserted out of time order; time, not arrival, sets the priority
        book.insert(second.clone());
        book.insert(first.clone());
        book.insert(better.clone());
        assert!(run(&mut book, now()).is_empty());
        book.insert(o.sell(4, 10, Units::uniform(5, 50)));
        let fills = run(&mut book, now());
        let who: Vec<(Uuid, i32)> = fills.iter().map(|f| (f.buy, f.quantity)).collect();
        assert_eq!(who, vec![(better.id, 2), (first.id, 2), (second.id, 1)]);
        // All at the resting bids' prices
        assert_eq!(fills.iter().map(|f| f.price).collect::<Vec<_>>(), vec![11, 10, 10]);
    }

    #[test]
    fn a_buy_walks_past_asks_below_its_grade_and_they_keep_their_place() {
        let mut o = Orders::new();
        let mut book = Book::new();
        let cheap_low = o.sell(1, 5, Units::uniform(3, 40));
        let dear_high = o.sell(2, 8, Units::uniform(3, 91));
        book.insert(cheap_low.clone());
        book.insert(dear_high.clone());
        book.insert(o.buy(3, 10, 3, "A", GradeRule::EveryUnit));
        let fills = run(&mut book, now());
        assert_eq!(fills.iter().map(|f| (f.sell, f.price)).collect::<Vec<_>>(), vec![(dear_high.id, 8)]);
        // The low lot is still first in line for the next buyer who'll take it
        book.insert(o.buy(4, 10, 1, "F", GradeRule::Average));
        let fills = run(&mut book, now());
        assert_eq!(fills.iter().map(|f| f.sell).collect::<Vec<_>>(), vec![cheap_low.id]);
    }

    #[test]
    fn a_blended_lot_sells_its_best_to_a_strict_buyer_and_its_rest_to_anyone() {
        let mut o = Orders::new();
        let mut book = Book::new();
        let lot = o.sell(1, 10, Units::from_pairs([(45, 4), (93, 2)]));
        book.insert(lot.clone());
        // An A buyer wants 5, every unit: only the two 93s are A
        book.insert(o.buy(2, 10, 5, "A", GradeRule::EveryUnit));
        let fills = run(&mut book, now());
        assert_eq!((fills[0].quantity, fills[0].units.clone()), (2, Units::uniform(2, 93)));
        // The A buy rests with 3 to go; the lot's 45s go to a D buyer
        book.insert(o.buy(3, 10, 9, "D", GradeRule::Average));
        let fills = run(&mut book, now());
        assert_eq!(fills[0].units, Units::uniform(4, 45));
        assert!(book.get(lot.id).is_none());
        assert_eq!(book.side("rope", OrderSide::Buy).iter().map(|b| b.remaining).collect::<Vec<_>>(), vec![3, 5]);
    }

    #[test]
    fn nobody_trades_with_themselves() {
        let mut o = Orders::new();
        let mut book = Book::new();
        let own = o.sell(1, 10, Units::uniform(2, 50));
        let theirs = o.sell(2, 11, Units::uniform(2, 50));
        book.insert(own.clone());
        book.insert(theirs.clone());
        let bid = o.buy(1, 12, 4, "F", GradeRule::Average);
        book.insert(bid.clone());
        let fills = run(&mut book, now());
        // Past their own cheaper ask to the other seller's
        assert_eq!(fills.iter().map(|f| f.sell).collect::<Vec<_>>(), vec![theirs.id]);
        // Their own bid and ask both rest, crossed, and stay so
        assert!(book.get(own.id).is_some() && book.get(bid.id).is_some());
        assert!(run(&mut book, now()).is_empty());
    }

    #[test]
    fn expired_and_stalled_orders_dont_trade() {
        let mut o = Orders::new();
        let mut book = Book::new();
        let ask = o.sell(1, 10, Units::uniform(2, 50));
        book.insert(ask.clone());
        let later = ask.expires_at;
        book.insert(o.buy(2, 10, 1, "F", GradeRule::Average));
        assert!(run(&mut book, later).is_empty());
        assert_eq!(book.expired(later), vec![ask.id]);

        let mut book = Book::new();
        book.insert(ask.clone());
        let bid = o.buy(2, 10, 1, "F", GradeRule::Average);
        book.insert(bid.clone());
        let fill = book.next_fill(now()).unwrap();
        // The buyer's box was full: they sit out, and the ask rests
        book.stall(fill.buy, now());
        assert!(run(&mut book, now()).is_empty());
        assert!(run(&mut book, now() + Duration::seconds(STALL_SECS - 1)).is_empty());
        // Until their time's up
        let fills = run(&mut book, now() + Duration::seconds(STALL_SECS));
        assert_eq!(fills.iter().map(|f| f.buy).collect::<Vec<_>>(), vec![bid.id]);
    }

    #[test]
    fn a_crossed_book_loaded_at_startup_trades_at_the_earlier_price() {
        let mut o = Orders::new();
        let bid = o.buy(1, 20, 2, "F", GradeRule::Average);
        let ask = o.sell(2, 12, Units::uniform(2, 50));
        let mut book = Book::load(vec![ask, bid]);
        let fills = run(&mut book, now());
        assert_eq!(fills.iter().map(|f| f.price).collect::<Vec<_>>(), vec![20]);
    }

    /// Many random orders crossing at once, applied in random batches: every
    /// fill is within both prices at the earlier one's, meets the grade, and
    /// isn't a self-trade; units and quantities are conserved; and at the end
    /// no two orders that could trade are left resting.
    #[test]
    fn random_books_conserve_and_leave_nothing_that_could_trade() {
        for seed in 0..40 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut o = Orders::new();
            let mut book = Book::new();
            let mut placed: HashMap<Uuid, BookOrder> = HashMap::new();
            let mut fills = Vec::new();
            for _ in 0..rng.gen_range(5..15) {
                for _ in 0..rng.gen_range(1..30) {
                    let player = rng.gen_range(0..5);
                    let price = rng.gen_range(5..25);
                    let order = if rng.gen_bool(0.5) {
                        let mut units = Units::new();
                        for _ in 0..rng.gen_range(1..4) {
                            units.add(rng.gen_range(0..=100), rng.gen_range(1..6));
                        }
                        o.sell(player, price, units)
                    } else {
                        let grade = ["A", "B", "C", "D", "F"][rng.gen_range(0..5)];
                        let rule = if rng.gen_bool(0.5) { GradeRule::Average } else { GradeRule::EveryUnit };
                        o.buy(player, price, rng.gen_range(1..15), grade, rule)
                    };
                    placed.insert(order.id, order.clone());
                    book.insert(order);
                }
                fills.extend(run(&mut book, now()));
            }
            assert!(!fills.is_empty(), "seed {seed} traded nothing");

            let mut traded: HashMap<Uuid, i32> = HashMap::new();
            let mut sold: HashMap<Uuid, Units> = HashMap::new();
            for f in &fills {
                let (buy, sell) = (&placed[&f.buy], &placed[&f.sell]);
                assert!(sell.price <= f.price && f.price <= buy.price, "seed {seed}: {f:?}");
                let first = if buy.time() < sell.time() { buy } else { sell };
                assert_eq!(f.price, first.price);
                assert_ne!(f.buyer, f.seller);
                let (grade, rule) = buy.grade.clone().unwrap();
                assert!(capitol_protocol::rules::market::meets_grade(&f.units, &grade, rule));
                assert_eq!(f.units.count(), f.quantity as i64);
                *traded.entry(f.buy).or_default() += f.quantity;
                *traded.entry(f.sell).or_default() += f.quantity;
                assert!(sold.entry(f.sell).or_default().merge(&f.units));
            }
            for (id, order) in &placed {
                let done = traded.get(id).copied().unwrap_or(0);
                match book.get(*id) {
                    Some(left) => {
                        assert_eq!(left.remaining + done, order.remaining, "seed {seed}");
                        if order.side == OrderSide::Sell {
                            let mut all = left.units.clone();
                            all.merge(sold.get(id).unwrap_or(&Units::new()));
                            assert_eq!(all, order.units);
                        }
                    }
                    None => assert_eq!(done, order.remaining, "seed {seed}: a gone order traded it all"),
                }
            }
            // Nothing left that could trade
            let bids = book.side("rope", OrderSide::Buy);
            for ask in book.side("rope", OrderSide::Sell) {
                for bid in &bids {
                    let (grade, rule) = bid.grade.clone().unwrap();
                    let could = bid.price >= ask.price
                        && bid.player_id != ask.player_id
                        && fill_units(&ask.units, bid.remaining, &grade, rule).is_some();
                    assert!(!could, "seed {seed}: {bid:?} and {ask:?} could still trade");
                }
            }
        }
    }

    /// What a fill meets when it's written, the same whichever way it's run:
    /// the buyer's box is full (they're stalled), or the sell was found
    /// cancelled (it leaves the book), or it's written.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Fate {
        Written,
        BoxFull,
        Cancelled,
    }

    fn fate(fill: &Fill, full: &HashSet<Uuid>, cancelled: &HashSet<Uuid>) -> Fate {
        if full.contains(&fill.buyer) {
            Fate::BoxFull
        } else if cancelled.contains(&fill.sell) {
            Fate::Cancelled
        } else {
            Fate::Written
        }
    }

    fn meet(book: &mut Book, fill: &Fill, fate: Fate, now: DateTime<Utc>) {
        match fate {
            Fate::Written => book.apply(fill),
            Fate::BoxFull => book.stall(fill.buy, now),
            Fate::Cancelled => {
                book.remove(fill.sell);
            }
        }
    }

    /// Everything on the book, best first, by item and side
    fn snapshot(book: &Book) -> Vec<BookOrder> {
        book.item_types()
            .iter()
            .flat_map(|item| [book.side(item, OrderSide::Buy), book.side(item, OrderSide::Sell)])
            .flatten()
            .cloned()
            .collect()
    }

    /// Fills proposed in batches, applied before they're written and rewound
    /// past the first that fails, trade exactly as fills one at a time do
    /// (#237): the same fills in the same order, and the same book after.
    #[test]
    fn batches_rewound_at_a_failure_trade_as_fills_one_at_a_time() {
        for seed in 0..60 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut o = Orders::new();
            let players = rng.gen_range(2..12u128);
            let mut orders = Vec::new();
            for _ in 0..rng.gen_range(10..60) {
                let player = rng.gen_range(0..players);
                let price = rng.gen_range(5..15);
                orders.push(if rng.gen_bool(0.5) {
                    o.sell(player, price, Units::from_pairs([(rng.gen_range(0..=100), rng.gen_range(1..6))]))
                } else {
                    let grade = ["A", "C", "F"][rng.gen_range(0..3)];
                    o.buy(player, price, rng.gen_range(1..10), grade, GradeRule::Average)
                });
            }
            let full: HashSet<Uuid> =
                (0..players).filter(|_| rng.gen_bool(0.15)).map(Uuid::from_u128).collect();
            let cancelled: HashSet<Uuid> = orders.iter().filter(|_| rng.gen_bool(0.1)).map(|o| o.id).collect();
            let width = rng.gen_range(1..8);

            let mut one = Book::load(orders.clone());
            let mut singly = Vec::new();
            while let Some(fill) = one.next_fill(now()) {
                let fate = fate(&fill, &full, &cancelled);
                meet(&mut one, &fill, fate, now());
                singly.push((fill, fate));
            }

            let mut batched = Book::load(orders);
            let mut together = Vec::new();
            loop {
                let batch = batched.propose(now(), width);
                assert!(batch.len() <= width);
                if batch.is_empty() {
                    batched.settle();
                    break;
                }
                let mut players = HashSet::new();
                for (fill, _) in &batch {
                    assert!(players.insert(fill.buyer) && players.insert(fill.seller), "seed {seed}: a player twice");
                }
                let failed = batch.iter().position(|(f, _)| fate(f, &full, &cancelled) != Fate::Written);
                let written = failed.unwrap_or(batch.len());
                together.extend(batch[..written].iter().map(|(f, _)| (f.clone(), Fate::Written)));
                if let Some(k) = failed {
                    batched.rewind(batch[k].1);
                    batched.settle();
                    let fill = &batch[k].0;
                    let fate = fate(fill, &full, &cancelled);
                    meet(&mut batched, fill, fate, now());
                    together.push((fill.clone(), fate));
                } else {
                    batched.settle();
                }
            }
            assert_eq!(together, singly, "seed {seed} width {width}");
            assert_eq!(snapshot(&batched), snapshot(&one), "seed {seed} width {width}");
        }
    }

    /// An order placed or cancelled while the matcher speculates waits for
    /// the settle: a rewind doesn't undo it, and it doesn't upset a rewind.
    #[test]
    fn orders_placed_and_cancelled_while_speculating_survive_a_rewind() {
        let mut o = Orders::new();
        let mut book = Book::new();
        let ask = o.sell(1, 10, Units::uniform(4, 50));
        let gone = o.sell(2, 11, Units::uniform(4, 50));
        book.insert(ask.clone());
        book.insert(gone.clone());
        book.insert(o.buy(3, 12, 2, "F", GradeRule::Average));

        let batch = book.propose(now(), 4);
        assert_eq!(batch.len(), 1);
        let late = o.buy(4, 10, 2, "F", GradeRule::Average);
        book.insert(late.clone());
        book.remove(gone.id);
        // Not yet
        assert!(book.get(late.id).is_none() && book.get(gone.id).is_some());
        // The fill wasn't written
        book.rewind(batch[0].1);
        book.settle();
        assert_eq!(book.get(ask.id).map(|a| a.remaining), Some(4));
        assert!(book.get(late.id).is_some() && book.get(gone.id).is_none());

        // The first buy fills, then the late one
        let fills = run(&mut book, now());
        assert_eq!(fills.iter().map(|f| (f.sell, f.quantity)).collect::<Vec<_>>(), vec![(ask.id, 2), (ask.id, 2)]);
        assert_eq!(fills[1].buy, late.id);
    }

    /// The benchmark: 10,000 resting orders that don't cross, then 1,000 new
    /// ones a second that do. Run it in release:
    /// `cargo test --release -p capitol-backend market_book_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore = "a benchmark; run it in release"]
    fn market_book_benchmark() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut o = Orders::new();
        let resting: Vec<BookOrder> = (0..10_000)
            .map(|i| {
                let player = rng.gen_range(0..200);
                let grade = ["A", "B", "C", "D", "F"][rng.gen_range(0..5)];
                if i % 2 == 0 {
                    let mut units = Units::new();
                    for _ in 0..rng.gen_range(1..5) {
                        units.add(rng.gen_range(0..=100), rng.gen_range(1..20));
                    }
                    o.sell(player, rng.gen_range(101..200), units)
                } else {
                    o.buy(player, rng.gen_range(1..100), rng.gen_range(1..50), grade, GradeRule::Average)
                }
            })
            .collect();
        let started = std::time::Instant::now();
        let mut book = Book::load(resting);
        assert!(run(&mut book, now()).is_empty());
        println!("load and first pass over 10,000: {:?}", started.elapsed());

        let mut worst = std::time::Duration::ZERO;
        let mut total = std::time::Duration::ZERO;
        let mut fills = 0;
        for _ in 0..10 {
            for _ in 0..1000 {
                let player = rng.gen_range(0..200);
                let order = if rng.gen_bool(0.5) {
                    let mut units = Units::new();
                    units.add(rng.gen_range(0..=100), rng.gen_range(1..20));
                    o.sell(player, rng.gen_range(50..150), units)
                } else {
                    let grade = ["A", "B", "C", "D", "F"][rng.gen_range(0..5)];
                    let rule = if rng.gen_bool(0.5) { GradeRule::Average } else { GradeRule::EveryUnit };
                    o.buy(player, rng.gen_range(50..150), rng.gen_range(1..50), grade, rule)
                };
                book.insert(order);
            }
            let tick = std::time::Instant::now();
            fills += run(&mut book, now()).len();
            let took = tick.elapsed();
            worst = worst.max(took);
            total += took;
        }
        println!(
            "10 ticks of 1,000 new orders each against ~10,000 resting: {fills} fills, {:?} a tick on average, {worst:?} at worst; {} orders rest",
            total / 10,
            book.len()
        );
        // A debug build is ten times slower; the budget is for release
        if !cfg!(debug_assertions) {
            assert!(worst < std::time::Duration::from_millis(100), "a tick's matching took {worst:?}");
        }
    }
}
