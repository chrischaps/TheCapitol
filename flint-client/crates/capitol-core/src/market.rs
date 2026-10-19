//! The market (E3.3), as the client holds it: the books of the items it
//! watches, your orders, and the latest trades. No UI yet: the panel is E3.5.
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

use capitol_protocol::rest::{DepthLevel, MarketDepthResponse, MarketOrder, MarketTrade};
use capitol_protocol::rules::market::{apply_depth_diff, OrderSide, OrderStatus};
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

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::market::{depth_diff, GradeRule};

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

    #[test]
    fn items_not_watched_are_ignored() {
        let mut market = MarketStore::new();
        assert!(market.apply_ws(&depth(1, true, Vec::new())).is_empty());
        assert!(market.book("plank").is_none());
    }
}
