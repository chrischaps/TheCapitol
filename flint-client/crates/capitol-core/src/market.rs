//! The market (E3.3), as the client holds it: the books of the items it
//! watches, your orders, and the latest trades; and the order form's rules
//! (E3.5): [`quote`] says what an order costs, what of it would fill now
//! against the visible book, and what you'd hold afterwards.
//!
//! - **Watching an item.** [`MarketStore::watch`] queues `subscribe_market`.
//!   The server answers with the whole book (`market_depth`, `full`), then
//!   sends its diffs and its `market_trade`s. Each diff's `seq` is one more
//!   than the last: one at or below the book's is old news and skipped, and
//!   one further on means some were lost, so the book is dropped and asked for
//!   again. A new session (every `Connected`) asks again for every item.
//! - **Your orders** come from `/market/orders/mine`, the answers to placing
//!   and cancelling, and `order_placed`, `order_filled` and `order_cancelled`,
//!   which arrive whether or not you watch the item.
//!
//! The game sends what [`MarketStore::take_outgoing`] gives it each frame:
//! the store never touches the network itself.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use capitol_protocol::rest::{DepthLevel, MarketDepthResponse, MarketOrder, MarketTrade, PlaceOrder};
use capitol_protocol::rules::market::{
    apply_depth_diff, buy_debit, fill_units, grade_floor, listing_fee, order_value, validate, GradeRule, OrderError,
    OrderSide, OrderStatus,
};
use capitol_protocol::rules::quality_grade;
use capitol_protocol::rules::stacking::{Pick, Units};
use capitol_protocol::ws::{ClientMessage, ServerMessage};
use uuid::Uuid;

/// How many of an item's latest trades the store keeps.
pub const RECENT_TRADES: usize = 50;

/// One item's book, as the server last said it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Book {
    /// The server's count of the book's changes
    pub seq: u64,
    /// Bids best first, then asks best first
    pub levels: Vec<DepthLevel>,
}

impl Book {
    /// The levels on `side`, best first.
    pub fn side(&self, side: OrderSide) -> impl Iterator<Item = &DepthLevel> {
        self.levels.iter().filter(move |l| l.side == side)
    }

    /// The highest bid's price.
    pub fn best_bid(&self) -> Option<i64> {
        self.side(OrderSide::Buy).next().map(|l| l.price)
    }

    /// The lowest ask's price.
    pub fn best_ask(&self) -> Option<i64> {
        self.side(OrderSide::Sell).next().map(|l| l.price)
    }
}

/// What changed in the market, for the UI to react to.
#[derive(Debug, Clone, PartialEq)]
pub enum MarketEvent {
    /// An item's book changed (or arrived). See [`MarketStore::book`].
    DepthChanged { item_type: String },
    /// A trade in an item you watch, anyone's
    Traded(MarketTrade),
    /// Your order is on the book
    OrderPlaced(MarketOrder),
    /// Some or all of your order traded
    OrderFilled { order: MarketOrder, trade: MarketTrade },
    /// Your order left the book: cancelled or expired (its status says which)
    OrderClosed(MarketOrder),
    /// Your orders were refetched
    OrdersUpdated,
}

/// The market as the client holds it.
#[derive(Debug, Clone, Default)]
pub struct MarketStore {
    watched: BTreeSet<String>,
    /// Only items whose whole book has arrived
    books: HashMap<String, Book>,
    /// Yours, open and closed, by id
    orders: BTreeMap<Uuid, MarketOrder>,
    trades: HashMap<String, VecDeque<MarketTrade>>,
    outgoing: Vec<ClientMessage>,
}

impl MarketStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Watch `item_type`'s book and trades.
    pub fn watch(&mut self, item_type: &str) {
        self.watched.insert(item_type.to_string());
        self.ask_for(item_type);
    }

    /// Stop watching `item_type`.
    pub fn unwatch(&mut self, item_type: &str) {
        if self.watched.remove(item_type) {
            self.books.remove(item_type);
            self.trades.remove(item_type);
            self.outgoing.push(ClientMessage::UnsubscribeMarket {
                item_type: Some(item_type.to_string()),
            });
        }
    }

    /// The messages to send, oldest first. The game sends them each frame.
    pub fn take_outgoing(&mut self) -> Vec<ClientMessage> {
        std::mem::take(&mut self.outgoing)
    }

    fn ask_for(&mut self, item_type: &str) {
        let ask = ClientMessage::SubscribeMarket {
            item_type: item_type.to_string(),
        };
        if !self.outgoing.contains(&ask) {
            self.outgoing.push(ask);
        }
    }

    /// A new session: the server watches nothing for it yet, so ask again
    /// for every item.
    pub fn connected(&mut self) {
        self.books.clear();
        let items: Vec<String> = self.watched.iter().cloned().collect();
        for item in items {
            self.ask_for(&item);
        }
    }

    /// Apply a server message. Those that aren't the market's change nothing.
    pub fn apply_ws(&mut self, message: &ServerMessage) -> Vec<MarketEvent> {
        let mut out = Vec::new();
        match message {
            ServerMessage::MarketDepth {
                item_type,
                seq,
                full,
                levels,
            } => {
                if !self.watched.contains(item_type) {
                    return out;
                }
                if *full {
                    self.books.insert(
                        item_type.clone(),
                        Book {
                            seq: *seq,
                            levels: levels.clone(),
                        },
                    );
                    out.push(MarketEvent::DepthChanged {
                        item_type: item_type.clone(),
                    });
                    return out;
                }
                // Before the whole book arrives, diffs are already in it
                let Some(book) = self.books.get_mut(item_type) else {
                    return out;
                };
                if *seq <= book.seq {
                    return out;
                }
                if *seq == book.seq + 1 {
                    apply_depth_diff(&mut book.levels, levels);
                    book.seq = *seq;
                    out.push(MarketEvent::DepthChanged {
                        item_type: item_type.clone(),
                    });
                } else {
                    tracing::debug!("{item_type}'s book skipped from {} to {seq}; asking again", book.seq);
                    self.books.remove(item_type);
                    self.ask_for(item_type);
                }
            }
            ServerMessage::MarketTrade { trade } => {
                if self.watched.contains(&trade.item_type) {
                    let recent = self.trades.entry(trade.item_type.clone()).or_default();
                    recent.push_front(trade.clone());
                    recent.truncate(RECENT_TRADES);
                    out.push(MarketEvent::Traded(trade.clone()));
                }
            }
            ServerMessage::OrderPlaced { order } => {
                self.orders.insert(order.id, order.clone());
                out.push(MarketEvent::OrderPlaced(order.clone()));
            }
            ServerMessage::OrderFilled { order, trade } => {
                self.orders.insert(order.id, order.clone());
                out.push(MarketEvent::OrderFilled {
                    order: order.clone(),
                    trade: trade.clone(),
                });
            }
            ServerMessage::OrderCancelled { order } => {
                self.orders.insert(order.id, order.clone());
                out.push(MarketEvent::OrderClosed(order.clone()));
            }
            _ => {}
        }
        out
    }

    /// `/market/orders/mine`: every order it lists replaces the store's, and
    /// an open one it doesn't list has closed meanwhile, so it goes.
    pub fn set_orders(&mut self, orders: &[MarketOrder]) -> Vec<MarketEvent> {
        self.orders.retain(|_, o| o.status != OrderStatus::Open);
        for order in orders {
            self.orders.insert(order.id, order.clone());
        }
        vec![MarketEvent::OrdersUpdated]
    }

    /// An order as the server answered placing or cancelling it. A later
    /// event may already have moved it on, and a smaller remainder or a
    /// closed status is later.
    pub fn answered(&mut self, order: &MarketOrder) -> Vec<MarketEvent> {
        let newer = match self.orders.get(&order.id) {
            Some(held) => {
                held.status == OrderStatus::Open
                    && (order.status != OrderStatus::Open || order.quantity_remaining < held.quantity_remaining)
            }
            None => true,
        };
        if newer {
            self.orders.insert(order.id, order.clone());
        }
        vec![MarketEvent::OrdersUpdated]
    }

    /// `GET /market/orders` for a watched item: taken if it's no older than
    /// the book held.
    pub fn set_depth(&mut self, depth: &MarketDepthResponse) -> Vec<MarketEvent> {
        if !self.watched.contains(&depth.item_type) {
            return Vec::new();
        }
        if let Some(book) = self.books.get(&depth.item_type) {
            if book.seq > depth.seq {
                return Vec::new();
            }
        }
        self.books.insert(
            depth.item_type.clone(),
            Book {
                seq: depth.seq,
                levels: depth.levels.clone(),
            },
        );
        vec![MarketEvent::DepthChanged {
            item_type: depth.item_type.clone(),
        }]
    }

    pub fn watched(&self) -> impl Iterator<Item = &str> {
        self.watched.iter().map(String::as_str)
    }

    pub fn is_watching(&self, item_type: &str) -> bool {
        self.watched.contains(item_type)
    }

    /// `item_type`'s book, once the whole of it has arrived.
    pub fn book(&self, item_type: &str) -> Option<&Book> {
        self.books.get(item_type)
    }

    /// Your open orders, newest first.
    pub fn open_orders(&self) -> Vec<&MarketOrder> {
        let mut open: Vec<&MarketOrder> = self.orders.values().filter(|o| o.status == OrderStatus::Open).collect();
        open.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        open
    }

    pub fn order(&self, id: Uuid) -> Option<&MarketOrder> {
        self.orders.get(&id)
    }

    /// `item_type`'s latest trades, newest first.
    pub fn trades(&self, item_type: &str) -> impl Iterator<Item = &MarketTrade> {
        self.trades.get(item_type).into_iter().flatten()
    }
}

// ---------------------------------------------------------------------------
// The order form (E3.5): what an order costs, what of it would fill now
// against the visible book, and what you'd hold afterwards
// ---------------------------------------------------------------------------

/// An order as the form has it, before it's sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub side: OrderSide,
    pub item_type: String,
    /// Strands per unit
    pub price: i64,
    pub quantity: i32,
    /// Buy: the lowest grade it takes, A to F
    pub min_grade: String,
    /// Buy: how `min_grade` reads the units sold
    pub grade_rule: GradeRule,
}

impl Draft {
    /// What to send for it. A sell names `stack`, the stack its units come
    /// from; the server takes the lowest of them, as [`quote`] does.
    pub fn to_place(&self, stack: Option<Uuid>) -> PlaceOrder {
        let buy = self.side == OrderSide::Buy;
        PlaceOrder {
            side: self.side,
            item_type: self.item_type.clone(),
            price: self.price,
            quantity: self.quantity,
            item_id: if buy { None } else { stack },
            qualities: None,
            min_grade: buy.then(|| self.min_grade.clone()),
            grade_rule: buy.then_some(self.grade_rule),
        }
    }
}

/// What of an order would trade the moment it reached the book.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fills {
    /// Units that would trade
    pub quantity: i64,
    /// Strands they'd trade for, each at the resting order's price
    pub value: i64,
    /// A buy on `average`: lower-grade units at a crossing price might ride
    /// along in a blend, and the book can't say which orders hold them. So
    /// more may fill than `quantity`.
    pub maybe_more: bool,
}

impl Fills {
    /// The mean price of what would fill, rounded to the nearest Strand.
    pub fn mean_price(&self) -> Option<i64> {
        (self.quantity > 0).then(|| (self.value + self.quantity / 2) / self.quantity)
    }
}

/// An order, previewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderQuote {
    /// `price × quantity`
    pub value: i64,
    /// The listing fee: [`listing_fee`], what the server charges
    pub fee: i64,
    /// What placing takes from your balance: a buy's value and fee, a
    /// sell's fee
    pub debit: i64,
    /// What would trade at once
    pub fills: Fills,
    /// Your balance once it's placed and those fills are done, once your
    /// balance is known. A buy filled under its price gets the difference
    /// back; a sell is paid for what fills.
    pub balance_after: Option<i64>,
    /// Sell: the units that go, the lowest of the stack
    pub units: Option<Units>,
    /// Sell: what's left in the stack afterwards
    pub left_in_stack: Option<i64>,
}

/// Why the form can't place its order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormRefusal {
    /// No price or no quantity yet: nothing to say
    Incomplete,
    /// Out of the server's bounds
    Bad(OrderError),
    /// A sell with no stack of the item to sell from
    NoStack,
    /// A sell for more than the stack holds
    NotHeld { held: i64 },
    /// Not Strands enough for the debit
    Short { debit: i64, short: i64 },
}

/// Previews `draft`. `stack` is a sell's stack's units, `book` the item's
/// book as the client holds it, `mine` your open orders (the matcher walks
/// past them, so they don't fill yours), `strands` your balance.
pub fn quote(
    draft: &Draft,
    stack: Option<&Units>,
    book: Option<&Book>,
    mine: &[&MarketOrder],
    strands: Option<i64>,
) -> Result<OrderQuote, FormRefusal> {
    if draft.price < 1 || draft.quantity < 1 {
        return Err(FormRefusal::Incomplete);
    }
    validate(draft.price, draft.quantity).map_err(FormRefusal::Bad)?;
    let value = order_value(draft.price, draft.quantity);
    let fee = listing_fee(value);
    let others = book.map(|b| others_levels(&draft.item_type, b, mine)).unwrap_or_default();
    match draft.side {
        OrderSide::Buy => {
            if grade_floor(&draft.min_grade).is_none() {
                return Err(FormRefusal::Bad(OrderError::BadGrade));
            }
            let debit = buy_debit(draft.price, draft.quantity);
            short_of(debit, strands)?;
            let fills = buy_fills(draft, &others);
            // Escrow is the price for every unit; a fill under it refunds the rest
            let refund = draft.price * fills.quantity - fills.value;
            Ok(OrderQuote {
                value,
                fee,
                debit,
                balance_after: strands.map(|s| s - debit + refund),
                fills,
                units: None,
                left_in_stack: None,
            })
        }
        OrderSide::Sell => {
            let held = stack.ok_or(FormRefusal::NoStack)?;
            let units = held
                .pick(draft.quantity, Pick::Lowest)
                .ok_or(FormRefusal::NotHeld { held: held.count() })?;
            short_of(fee, strands)?;
            let fills = sell_fills(draft, &units, &others);
            Ok(OrderQuote {
                value,
                fee,
                debit: fee,
                balance_after: strands.map(|s| s - fee + fills.value),
                fills,
                left_in_stack: Some(held.count() - units.count()),
                units: Some(units),
            })
        }
    }
}

fn short_of(debit: i64, strands: Option<i64>) -> Result<(), FormRefusal> {
    match strands {
        Some(s) if s < debit => Err(FormRefusal::Short { debit, short: debit - s }),
        _ => Ok(()),
    }
}

/// The book less your own orders, which yours can't trade with.
fn others_levels(item_type: &str, book: &Book, mine: &[&MarketOrder]) -> Vec<DepthLevel> {
    let mut levels = book.levels.clone();
    for order in mine.iter().filter(|o| o.item_type == item_type && o.status == OrderStatus::Open) {
        let mut less = |side: OrderSide, grade: &str, rule: Option<GradeRule>, n: i64| {
            if let Some(l) = levels
                .iter_mut()
                .find(|l| l.side == side && l.price == order.price && l.grade == grade && l.grade_rule == rule)
            {
                l.quantity -= n;
            }
        };
        match order.side {
            OrderSide::Buy => less(
                OrderSide::Buy,
                order.min_grade.as_deref().unwrap_or("F"),
                Some(order.grade_rule.unwrap_or_default()),
                order.quantity_remaining as i64,
            ),
            OrderSide::Sell => {
                for (q, c) in order.units.pairs() {
                    less(OrderSide::Sell, quality_grade(q), None, c as i64);
                }
            }
        }
    }
    levels.retain(|l| l.quantity > 0);
    levels
}

/// A buy walks the asks at or under its price, cheapest first, taking the
/// units whose grade meets its own.
fn buy_fills(draft: &Draft, levels: &[DepthLevel]) -> Fills {
    let floor = grade_floor(&draft.min_grade).unwrap_or(0);
    let mut fills = Fills::default();
    let mut left = draft.quantity as i64;
    for level in levels.iter().filter(|l| l.side == OrderSide::Sell && l.price <= draft.price) {
        if grade_floor(&level.grade).unwrap_or(0) < floor {
            fills.maybe_more |= draft.grade_rule == GradeRule::Average;
            continue;
        }
        let n = left.min(level.quantity);
        fills.quantity += n;
        fills.value += n * level.price;
        left -= n;
        if left == 0 {
            break;
        }
    }
    if left == 0 {
        fills.maybe_more = false;
    }
    fills
}

/// A sell walks the bids at or over its price, best first; each takes what
/// of the units meets its grade, as the matcher's [`fill_units`] does.
fn sell_fills(draft: &Draft, units: &Units, levels: &[DepthLevel]) -> Fills {
    let mut fills = Fills::default();
    let mut left = units.clone();
    for level in levels.iter().filter(|l| l.side == OrderSide::Buy && l.price >= draft.price) {
        let want = level.quantity.min(i32::MAX as i64) as i32;
        let rule = level.grade_rule.unwrap_or_default();
        let Some(taken) = fill_units(&left, want, &level.grade, rule) else {
            continue;
        };
        left.take_units(&taken);
        fills.quantity += taken.count();
        fills.value += taken.count() * level.price;
        if left.is_empty() {
            break;
        }
    }
    fills
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::market::depth_diff;

    fn level(side: OrderSide, price: i64, grade: &str, quantity: i64) -> DepthLevel {
        DepthLevel {
            side,
            price,
            grade: grade.into(),
            grade_rule: (side == OrderSide::Buy).then_some(GradeRule::Average),
            quantity,
            orders: 1,
        }
    }

    fn depth(seq: u64, full: bool, levels: Vec<DepthLevel>) -> ServerMessage {
        ServerMessage::MarketDepth {
            item_type: "plank".into(),
            seq,
            full,
            levels,
        }
    }

    #[test]
    fn watching_asks_once_and_a_new_session_asks_again() {
        let mut market = MarketStore::new();
        market.watch("plank");
        market.watch("plank");
        let ask = ClientMessage::SubscribeMarket {
            item_type: "plank".into(),
        };
        assert_eq!(market.take_outgoing(), std::slice::from_ref(&ask));
        assert!(market.take_outgoing().is_empty());
        market.apply_ws(&depth(3, true, vec![level(OrderSide::Buy, 9, "B", 4)]));
        assert!(market.book("plank").is_some());
        market.connected();
        assert!(market.book("plank").is_none());
        assert_eq!(market.take_outgoing(), [ask]);
        market.unwatch("plank");
        assert_eq!(
            market.take_outgoing(),
            [ClientMessage::UnsubscribeMarket {
                item_type: Some("plank".into())
            }]
        );
    }

    #[test]
    fn diffs_apply_in_order_old_ones_are_skipped_and_a_gap_asks_again() {
        let mut market = MarketStore::new();
        market.watch("plank");
        market.take_outgoing();
        let first = vec![level(OrderSide::Buy, 9, "B", 4), level(OrderSide::Sell, 11, "A", 2)];
        // A diff before the whole book is ignored
        assert!(market.apply_ws(&depth(1, false, Vec::new())).is_empty());
        market.apply_ws(&depth(5, true, first.clone()));

        let second = vec![level(OrderSide::Buy, 10, "C", 1), level(OrderSide::Sell, 11, "A", 2)];
        // An old diff, already in the book, changes nothing
        assert!(market.apply_ws(&depth(5, false, depth_diff(&first, &second))).is_empty());
        let changed = market.apply_ws(&depth(6, false, depth_diff(&first, &second)));
        assert_eq!(changed, [MarketEvent::DepthChanged { item_type: "plank".into() }]);
        let book = market.book("plank").unwrap();
        assert_eq!((book.seq, &book.levels), (6, &second));
        assert_eq!((book.best_bid(), book.best_ask()), (Some(10), Some(11)));

        // 8 after 6: one was lost
        market.apply_ws(&depth(8, false, Vec::new()));
        assert!(market.book("plank").is_none());
        assert_eq!(
            market.take_outgoing(),
            [ClientMessage::SubscribeMarket {
                item_type: "plank".into()
            }]
        );
    }

    fn draft(side: OrderSide, price: i64, quantity: i32, min_grade: &str, rule: GradeRule) -> Draft {
        Draft {
            side,
            item_type: "plank".into(),
            price,
            quantity,
            min_grade: min_grade.into(),
            grade_rule: rule,
        }
    }

    fn my_order(side: OrderSide, price: i64, remaining: i32, units: Units) -> MarketOrder {
        let at = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
        MarketOrder {
            id: Uuid::from_u128(price as u128),
            player_id: Uuid::nil(),
            side,
            item_type: "plank".into(),
            price,
            quantity_total: remaining,
            quantity_remaining: remaining,
            status: OrderStatus::Open,
            min_grade: (side == OrderSide::Buy).then(|| "F".into()),
            grade_rule: (side == OrderSide::Buy).then_some(GradeRule::Average),
            units,
            escrow_strands: 0,
            fee: 1,
            created_at: at,
            expires_at: at,
        }
    }

    fn book(levels: Vec<DepthLevel>) -> Book {
        Book { seq: 1, levels }
    }

    #[test]
    fn the_fee_and_debit_are_the_servers() {
        for (price, quantity) in [(1, 1), (7, 3), (41, 10), (250, 40), (1_000_000_000, 10_000)] {
            let buy = quote(&draft(OrderSide::Buy, price, quantity, "F", GradeRule::Average), None, None, &[], None)
                .unwrap();
            assert_eq!(buy.fee, listing_fee(price * quantity as i64));
            assert_eq!(buy.debit, buy_debit(price, quantity));
            let stack = Units::uniform(quantity, 60);
            let sell = quote(&draft(OrderSide::Sell, price, quantity, "F", GradeRule::Average), Some(&stack), None, &[], None)
                .unwrap();
            assert_eq!((sell.fee, sell.debit), (buy.fee, buy.fee));
        }
        let incomplete = draft(OrderSide::Buy, 0, 3, "F", GradeRule::Average);
        assert_eq!(quote(&incomplete, None, None, &[], None), Err(FormRefusal::Incomplete));
        let huge = draft(OrderSide::Buy, 5, 10_001, "F", GradeRule::Average);
        assert_eq!(quote(&huge, None, None, &[], None), Err(FormRefusal::Bad(OrderError::BadQuantity)));
        // 10 at 41 is 410, and its fee is 5
        let poor = draft(OrderSide::Buy, 41, 10, "F", GradeRule::Average);
        assert_eq!(quote(&poor, None, None, &[], Some(400)), Err(FormRefusal::Short { debit: 415, short: 15 }));
    }

    #[test]
    fn a_buy_fills_against_cheaper_asks_of_its_grade_and_gets_the_difference_back() {
        let asks = book(vec![
            level(OrderSide::Buy, 38, "C", 9),
            level(OrderSide::Sell, 40, "B", 3),
            level(OrderSide::Sell, 40, "D", 5),
            level(OrderSide::Sell, 42, "A", 4),
            level(OrderSide::Sell, 45, "B", 8),
        ]);
        // C or better at up to 42: 3 at 40, 4 at 42; the D's might ride in a blend
        let q = quote(&draft(OrderSide::Buy, 42, 10, "C", GradeRule::Average), None, Some(&asks), &[], Some(1000)).unwrap();
        assert_eq!(q.fills, Fills { quantity: 7, value: 288, maybe_more: true });
        assert_eq!(q.fills.mean_price(), Some(41));
        // 420 + 5 fee out, then 2 back on each of the three filled at 40
        assert_eq!(q.balance_after, Some(1000 - 425 + 6));
        // Every unit: the D's never come
        let strict = quote(&draft(OrderSide::Buy, 42, 10, "C", GradeRule::EveryUnit), None, Some(&asks), &[], None).unwrap();
        assert!(!strict.fills.maybe_more);
        // Your own ask doesn't fill your bid
        let mine = my_order(OrderSide::Sell, 40, 3, Units::uniform(3, 80));
        let q = quote(&draft(OrderSide::Buy, 40, 5, "F", GradeRule::Average), None, Some(&asks), &[&mine], None).unwrap();
        assert_eq!(q.fills, Fills { quantity: 5, value: 200, maybe_more: false });
        let q = quote(&draft(OrderSide::Buy, 40, 9, "F", GradeRule::Average), None, Some(&asks), &[&mine], None).unwrap();
        assert_eq!(q.fills.quantity, 5);
        // Under every ask, nothing fills
        let q = quote(&draft(OrderSide::Buy, 39, 9, "F", GradeRule::Average), None, Some(&asks), &[], None).unwrap();
        assert_eq!(q.fills, Fills::default());
    }

    #[test]
    fn a_sell_fills_the_bids_its_units_meet_and_says_whats_left() {
        let bids = book(vec![
            DepthLevel { grade_rule: Some(GradeRule::EveryUnit), ..level(OrderSide::Buy, 50, "A", 2) },
            level(OrderSide::Buy, 47, "C", 4),
            level(OrderSide::Buy, 44, "F", 10),
            level(OrderSide::Sell, 52, "B", 1),
        ]);
        let stack = Units::from_pairs([(40, 3), (70, 3), (92, 2)]);
        // Six go, the lowest: three 40s and three 70s. None meets A at every
        // unit; the C at 47 takes four that average 55; the two left go at 44
        let q = quote(&draft(OrderSide::Sell, 44, 6, "F", GradeRule::Average), Some(&stack), Some(&bids), &[], Some(100)).unwrap();
        assert_eq!(q.units, Some(Units::from_pairs([(40, 3), (70, 3)])));
        assert_eq!(q.left_in_stack, Some(2));
        assert_eq!(q.fills, Fills { quantity: 6, value: 4 * 47 + 2 * 44, maybe_more: false });
        assert_eq!(q.balance_after, Some(100 - q.fee + 276));
        // Priced over every bid, it rests
        let q = quote(&draft(OrderSide::Sell, 51, 2, "F", GradeRule::Average), Some(&stack), Some(&bids), &[], None).unwrap();
        assert_eq!(q.fills.quantity, 0);
        assert_eq!(
            quote(&draft(OrderSide::Sell, 44, 9, "F", GradeRule::Average), Some(&stack), None, &[], None),
            Err(FormRefusal::NotHeld { held: 8 })
        );
        assert_eq!(quote(&draft(OrderSide::Sell, 44, 1, "F", GradeRule::Average), None, None, &[], None), Err(FormRefusal::NoStack));
        // A buy and a sell ask for what they need
        let buy = draft(OrderSide::Buy, 44, 3, "B", GradeRule::EveryUnit).to_place(Some(Uuid::nil()));
        assert_eq!((buy.item_id, buy.min_grade.as_deref(), buy.grade_rule), (None, Some("B"), Some(GradeRule::EveryUnit)));
        let sell = draft(OrderSide::Sell, 44, 3, "B", GradeRule::EveryUnit).to_place(Some(Uuid::nil()));
        assert_eq!((sell.item_id, sell.min_grade, sell.grade_rule), (Some(Uuid::nil()), None, None));
    }

    #[test]
    fn items_not_watched_are_ignored() {
        let mut market = MarketStore::new();
        assert!(market.apply_ws(&depth(1, true, Vec::new())).is_empty());
        assert!(market.book("plank").is_none());
    }
}
