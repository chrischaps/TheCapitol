//! The market's matching engine (E3.2, economy plan D7): every second, close
//! the orders past their expiry and write the fills [`Book`] proposes, one
//! transaction a fill (TDD §5.2, write-through).
//!
//! The book is in memory behind a plain mutex that's never held across an
//! await: take a fill, drop the lock, write it, and only once it's committed
//! [`Book::apply`] it. A fill that fails changes nothing, in the DB or the book:
//! - an order no longer as the book had it (cancelled under it, say) is
//!   reloaded from the DB, and matching goes on;
//! - a buyer whose market box is full sits out for `STALL_SECS`;
//! - a DB error ends the cycle, and the next one tries again.
//!
//! A fill moves:
//! - the units the book named from the sell's escrow stack to the buyer's
//!   market box (onto the first stack of the type there, or a free slot);
//! - `price × quantity` from the buy's escrow to the seller's balance
//!   (`market_sale`), with what the buy escrowed over the fill's price back to
//!   the buyer (`market_refund`). So a buy's escrow is always its price × what's
//!   left of it. Proceeds go straight to the balance: D6 holds goods at the
//!   hall, and Strands aren't goods.
//!
//! It runs in a task of its own on a 1 s interval rather than in the tick
//! loop: a burst of fills is a burst of DB writes, and movement shouldn't wait
//! on them. A fill is about twenty round trips, some 130 fills a second on the
//! local stack, so a burst over that makes a cycle run long rather than lose
//! anything (#208: fewer round trips a fill).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use capitol_protocol::rest::{DepthLevel, MarketOrder, MarketTrade};
use capitol_protocol::rules::market::{self, GradeRule, OrderSide, OrderStatus};
use capitol_protocol::rules::quality_grade;
use capitol_protocol::rules::stacking::Units;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::currency::{add_strands_in, TransactionType};
use super::events::GameEvent;
use super::market::{self as escrow, Cancelled, MarketError};
use super::market_book::{Book, BookOrder, Fill};
use super::units::{self, NewStack};

/// How often matching runs: the tick's 10-tick cadence (TDD §3.2).
pub const CYCLE_MS: u64 = 1000;

/// The market: the book, shared by the matcher and the routes that place and
/// cancel (E3.3), and each item's depth as last published.
#[derive(Debug, Default)]
pub struct Market {
    book: Mutex<Book>,
    /// What each item's `market:<item>` channel was last told. The routes and
    /// subscribers read depth from here, not the book, so a snapshot and the
    /// diffs after it always line up by `seq`.
    published: Mutex<HashMap<String, Published>>,
}

/// One item's depth as last published.
#[derive(Debug, Default, Clone)]
pub struct Published {
    /// Counts the item's published changes
    pub seq: u64,
    pub levels: Vec<DepthLevel>,
}

/// What one cycle did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cycle {
    pub fills: usize,
    pub expired: usize,
    /// Fills that found an order changed under them, or a box full
    pub refused: usize,
}

impl Market {
    /// The market with every open order on its book.
    pub async fn load(db: &PgPool) -> Result<Market, sqlx::Error> {
        let orders = open_orders(db).await?;
        Ok(Market::from_book(Book::load(orders)))
    }

    /// The market over `book`, its depth published (to nobody yet) at seq 0.
    pub fn from_book(mut book: Book) -> Market {
        let published = book
            .take_dirty()
            .into_iter()
            .map(|item| {
                let levels = book.depth(&item);
                (item, Published { seq: 0, levels })
            })
            .collect();
        Market {
            book: Mutex::new(book),
            published: Mutex::new(published),
        }
    }

    fn published(&self) -> MutexGuard<'_, HashMap<String, Published>> {
        self.published.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Tell each changed item's channel what changed: a `MarketDepth` diff
    /// per item whose depth moved, its `seq` one up. The routes call it after
    /// placing or cancelling, and the matcher after each cycle.
    ///
    /// The published map is locked first and held while the book is read and
    /// the diffs are sent, so two publishers can't send an older depth after
    /// a newer one, and the channel carries each item's diffs in `seq` order.
    pub fn publish(&self, events: &broadcast::Sender<GameEvent>) {
        let mut published = self.published();
        let changed: Vec<(String, Vec<DepthLevel>)> = {
            let mut book = self.book();
            book.take_dirty()
                .into_iter()
                .map(|item| {
                    let levels = book.depth(&item);
                    (item, levels)
                })
                .collect()
        };
        for (item_type, levels) in changed {
            let entry = published.entry(item_type.clone()).or_default();
            let diff = market::depth_diff(&entry.levels, &levels);
            if diff.is_empty() {
                continue;
            }
            entry.seq += 1;
            entry.levels = levels;
            let _ = events.send(GameEvent::MarketDepth {
                item_type,
                seq: entry.seq,
                levels: diff,
            });
        }
    }

    /// `item_type`'s depth as last published: what a new subscriber starts from.
    pub fn depth(&self, item_type: &str) -> Published {
        self.published().get(item_type).cloned().unwrap_or_default()
    }

    /// Every item on the book as last published, with its best bid and ask.
    pub fn best_prices(&self) -> Vec<(String, Option<i64>, Option<i64>)> {
        let published = self.published();
        let mut out: Vec<_> = published
            .iter()
            .filter(|(_, p)| !p.levels.is_empty())
            .map(|(item, p)| {
                let best = |side: OrderSide| p.levels.iter().find(|l| l.side == side).map(|l| l.price);
                (item.clone(), best(OrderSide::Buy), best(OrderSide::Sell))
            })
            .collect();
        out.sort();
        out
    }

    fn book(&self) -> MutexGuard<'_, Book> {
        // A panic mid-update can't leave the book half-written: each change
        // is one call. So a poisoned lock is still a good book.
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `order` was placed: put it on the book to match next cycle.
    pub fn placed(&self, order: &MarketOrder) {
        if let Some(order) = BookOrder::from_order(order) {
            self.book().insert(order);
        }
    }

    /// Order `order_id` was cancelled: take it off the book.
    pub fn cancelled(&self, order_id: Uuid) {
        self.book().remove(order_id);
    }

    /// Read the book.
    pub fn with_book<R>(&self, f: impl FnOnce(&Book) -> R) -> R {
        f(&self.book())
    }

    /// Put the DB's view of `order_id` on the book: as it is if it's open,
    /// gone if it isn't.
    async fn reload(&self, db: &PgPool, order_id: Uuid) -> Result<(), sqlx::Error> {
        let mut conn = db.acquire().await?;
        let order = escrow::load(&mut conn, order_id).await?;
        drop(conn);
        let mut book = self.book();
        match order.as_ref().and_then(BookOrder::from_order) {
            Some(order) => book.insert(order),
            None => {
                book.remove(order_id);
            }
        }
        Ok(())
    }
}

/// Every open order, as the book holds it, in one query for the orders and
/// one for the sells' units.
async fn open_orders(db: &PgPool) -> Result<Vec<BookOrder>, sqlx::Error> {
    type Row = (
        Uuid,
        Uuid,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i32,
        Option<Uuid>,
        DateTime<Utc>,
        DateTime<Utc>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, player_id, side, item_type, min_grade, grade_rule, price, quantity_remaining,
                escrow_item_id, created_at, expires_at
         FROM market_orders WHERE status = 'open' AND quantity_remaining > 0",
    )
    .fetch_all(db)
    .await?;
    let unit_rows: Vec<(Uuid, i16, i32)> = sqlx::query_as(
        "SELECT u.item_id, u.quality, u.count FROM item_units u
         JOIN market_orders o ON o.escrow_item_id = u.item_id WHERE o.status = 'open'",
    )
    .fetch_all(db)
    .await?;
    let mut held: HashMap<Uuid, Units> = HashMap::new();
    for (item, quality, count) in unit_rows {
        held.entry(item).or_default().add(quality as i32, count);
    }

    let mut orders = Vec::with_capacity(rows.len());
    for (id, player_id, side, item_type, min_grade, grade_rule, price, remaining, escrow_item, created_at, expires_at) in
        rows
    {
        let Some(side) = OrderSide::parse(&side) else {
            tracing::warn!("Market order {id} has side {side:?}; left off the book");
            continue;
        };
        let grade = match (side, min_grade, grade_rule.as_deref().and_then(GradeRule::parse)) {
            (OrderSide::Buy, Some(grade), Some(rule)) => Some((grade, rule)),
            (OrderSide::Buy, ..) => {
                tracing::warn!("Buy order {id} has no grade; left off the book");
                continue;
            }
            (OrderSide::Sell, ..) => None,
        };
        let units = escrow_item.and_then(|item| held.remove(&item)).unwrap_or_default();
        orders.push(BookOrder {
            id,
            player_id,
            side,
            item_type,
            price,
            remaining,
            units,
            grade,
            created_at,
            expires_at,
        });
    }
    Ok(orders)
}

/// Run the matcher forever, a cycle a second.
pub async fn run_market_loop(market: Arc<Market>, db: PgPool, events: broadcast::Sender<GameEvent>) {
    tracing::info!("Market matching every {CYCLE_MS}ms, {} orders on the book", market.with_book(|b| b.len()));
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(CYCLE_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let cycle = run_cycle(&market, &db, &events, Utc::now()).await;
        if cycle != Cycle::default() {
            tracing::debug!("Market: {cycle:?}");
        }
    }
}

/// One cycle at `now`: expire what's due, fill until nothing crosses, then
/// publish what changed to the items' channels.
pub async fn run_cycle(market: &Market, db: &PgPool, events: &broadcast::Sender<GameEvent>, now: DateTime<Utc>) -> Cycle {
    let cycle = match_cycle(market, db, events, now).await;
    market.publish(events);
    cycle
}

async fn match_cycle(market: &Market, db: &PgPool, events: &broadcast::Sender<GameEvent>, now: DateTime<Utc>) -> Cycle {
    let mut cycle = Cycle::default();

    let due = market.book().expired(now);
    for order_id in due {
        match expire(db, order_id, now).await {
            Ok(Some(closed)) => {
                market.book().remove(order_id);
                cycle.expired += 1;
                announce_closed(db, events, closed).await;
            }
            Ok(None) => {
                if let Err(e) = market.reload(db, order_id).await {
                    tracing::warn!("Market: reloading order {order_id}: {e}");
                    return cycle;
                }
            }
            // Its seller's box is full: try again after a while
            Err(MarketError::MarketBoxFull) => market.book().stall(order_id, now),
            Err(e) => {
                tracing::warn!("Market: expiring order {order_id}: {}", e.message());
                return cycle;
            }
        }
    }

    loop {
        let Some(fill) = market.book().next_fill(now) else { break };
        match execute_fill(db, &fill).await {
            Ok(done) => {
                market.book().apply(&fill);
                cycle.fills += 1;
                announce_fill(events, &fill, done);
            }
            Err(MarketError::MarketBoxFull) => {
                cycle.refused += 1;
                market.book().stall(fill.buy, now);
            }
            Err(MarketError::Changed) => {
                cycle.refused += 1;
                for id in [fill.buy, fill.sell] {
                    if let Err(e) = market.reload(db, id).await {
                        tracing::warn!("Market: reloading order {id}: {e}");
                        return cycle;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Market: filling {} against {}: {}", fill.buy, fill.sell, e.message());
                break;
            }
        }
    }
    cycle
}

/// A written fill: both orders as they stand after it, and the trade.
#[derive(Debug, Clone)]
pub struct Executed {
    pub buy: MarketOrder,
    pub sell: MarketOrder,
    pub trade: MarketTrade,
    /// The buyer's market box, where the units went
    pub market_box: Uuid,
    pub seller_balance: i64,
    /// The buyer's balance, if the fill refunded them some
    pub buyer_balance: Option<i64>,
}

/// Write `fill` in one transaction, checking first that both orders are as
/// the book had them. Nothing changes unless all of it does.
pub async fn execute_fill(db: &PgPool, fill: &Fill) -> Result<Executed, MarketError> {
    let mut tx = db.begin().await?;
    let (buy, sell) = lock_pair(&mut tx, fill).await?;

    let taken = units::units_of(&mut tx, sell.escrow_item).await?;
    if !taken.contains(&fill.units) {
        return Err(MarketError::Changed);
    }
    let whole = taken == fill.units;

    // The units, to the buyer's box: the escrow stack itself if it's all of
    // them (a tool keeps its wear), otherwise a stack split off it
    let market_box = escrow::market_box(&mut tx, fill.buyer).await?;
    let moving = if whole {
        sell.escrow_item
    } else {
        let durability: Option<i32> = sqlx::query_scalar("SELECT durability FROM items WHERE id = $1")
            .bind(sell.escrow_item)
            .fetch_one(&mut *tx)
            .await?;
        units::take_units(&mut tx, sell.escrow_item, &fill.units).await?.ok_or(MarketError::Changed)?;
        let escrow_container: Uuid =
            sqlx::query_scalar("SELECT id FROM containers WHERE container_type = 'market_escrow'")
                .fetch_one(&mut *tx)
                .await?;
        units::insert_stack(
            &mut tx,
            NewStack {
                item_type: &fill.item_type,
                owner_id: None,
                container_id: Some(escrow_container),
                slot_index: None,
                durability,
            },
            &fill.units,
        )
        .await?
    };
    escrow::return_to_box(&mut tx, fill.buyer, moving, market_box).await?;

    // The Strands: the escrow at the buy's price pays the seller at the
    // fill's, and the buyer gets the difference
    let paid = market::order_value(fill.price, fill.quantity);
    let escrowed = market::order_value(buy.price, fill.quantity);
    sqlx::query(
        "UPDATE market_orders SET
             quantity_remaining = quantity_remaining - $2,
             escrow_strands = escrow_strands - $3,
             status = CASE WHEN quantity_remaining = $2 THEN 'filled' ELSE status END
         WHERE id = $1",
    )
    .bind(fill.buy)
    .bind(fill.quantity)
    .bind(escrowed)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE market_orders SET
             quantity_remaining = quantity_remaining - $2,
             escrow_item_id = CASE WHEN $3 THEN NULL ELSE escrow_item_id END,
             status = CASE WHEN quantity_remaining = $2 THEN 'filled' ELSE status END
         WHERE id = $1",
    )
    .bind(fill.sell)
    .bind(fill.quantity)
    .bind(whole)
    .execute(&mut *tx)
    .await?;

    let seller_balance = add_strands_in(
        &mut tx,
        fill.seller,
        paid,
        TransactionType::MarketSale,
        Some(&format!("Sold {} {} at {} each", fill.quantity, fill.item_type, fill.price)),
        Some(fill.sell),
    )
    .await
    .map_err(MarketError::Funds)?;
    // A filled buy whose escrow somehow wasn't its price × quantity gets back
    // what's left too, so no Strand stays in a closed order
    let leftover = if buy.remaining == fill.quantity { buy.escrow_strands - escrowed } else { 0 };
    let refund = escrowed - paid + leftover;
    let buyer_balance = if refund > 0 {
        let balance = add_strands_in(
            &mut tx,
            fill.buyer,
            refund,
            TransactionType::MarketRefund,
            Some(&format!("Bought {} {} at {} each, under the {} bid", fill.quantity, fill.item_type, fill.price, buy.price)),
            Some(fill.buy),
        )
        .await
        .map_err(MarketError::Funds)?;
        if leftover != 0 {
            sqlx::query("UPDATE market_orders SET escrow_strands = 0 WHERE id = $1")
                .bind(fill.buy)
                .execute(&mut *tx)
                .await?;
        }
        Some(balance)
    } else {
        None
    };

    let trade = MarketTrade {
        id: Uuid::new_v4(),
        buy_order_id: fill.buy,
        sell_order_id: fill.sell,
        item_type: fill.item_type.clone(),
        grade: quality_grade(fill.units.mean().unwrap_or(0)).to_string(),
        quality: fill.units.mean().unwrap_or(0),
        min_quality: fill.units.min().unwrap_or(0),
        price: fill.price,
        quantity: fill.quantity,
        at: Utc::now(),
    };
    sqlx::query(
        "INSERT INTO market_trades
             (id, buy_order_id, sell_order_id, item_type, grade, quality, min_quality, price, quantity, at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(trade.id)
    .bind(trade.buy_order_id)
    .bind(trade.sell_order_id)
    .bind(&trade.item_type)
    .bind(&trade.grade)
    .bind(trade.quality)
    .bind(trade.min_quality)
    .bind(trade.price)
    .bind(trade.quantity)
    .bind(trade.at)
    .execute(&mut *tx)
    .await?;

    let buy_now = escrow::load(&mut tx, fill.buy).await?.ok_or(MarketError::Changed)?;
    let sell_now = escrow::load(&mut tx, fill.sell).await?.ok_or(MarketError::Changed)?;
    tx.commit().await?;
    Ok(Executed {
        buy: buy_now,
        sell: sell_now,
        trade,
        market_box,
        seller_balance,
        buyer_balance,
    })
}

/// An order as a fill finds it, locked.
struct Locked {
    price: i64,
    remaining: i32,
    escrow_strands: i64,
    escrow_item: Uuid,
}

/// Lock both of `fill`'s orders (by id, so two fills never lock in opposite
/// orders) and check each is open, as the book had it, and can carry it.
async fn lock_pair(conn: &mut PgConnection, fill: &Fill) -> Result<(Locked, Locked), MarketError> {
    type Row = (Uuid, Uuid, String, String, String, Option<String>, Option<String>, i64, i32, Option<Uuid>, i64);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, player_id, side, status, item_type, min_grade, grade_rule, price, quantity_remaining,
                escrow_item_id, escrow_strands
         FROM market_orders WHERE id = ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind(vec![fill.buy, fill.sell])
    .fetch_all(&mut *conn)
    .await?;
    let find = |id: Uuid| rows.iter().find(|r| r.0 == id).ok_or(MarketError::Changed);
    let (b, s) = (find(fill.buy)?, find(fill.sell)?);
    let open = |r: &Row, side: OrderSide, player: Uuid| {
        r.1 == player
            && r.2 == side.as_str()
            && r.3 == OrderStatus::Open.as_str()
            && r.4 == fill.item_type
            && r.8 >= fill.quantity
    };
    if !open(b, OrderSide::Buy, fill.buyer) || !open(s, OrderSide::Sell, fill.seller) || fill.buyer == fill.seller {
        return Err(MarketError::Changed);
    }
    // The price is the resting order's, between the two
    if b.7 != fill.buy_price || fill.price < s.7 || fill.price > b.7 {
        return Err(MarketError::Changed);
    }
    if b.10 < market::order_value(b.7, fill.quantity) {
        return Err(MarketError::Changed);
    }
    let (Some(grade), Some(rule)) = (b.5.as_deref(), b.6.as_deref().and_then(GradeRule::parse)) else {
        return Err(MarketError::Changed);
    };
    if fill.units.count() != fill.quantity as i64 || !market::meets_grade(&fill.units, grade, rule) {
        return Err(MarketError::Changed);
    }
    let Some(escrow_item) = s.9 else { return Err(MarketError::Changed) };
    let locked = |r: &Row, escrow_item: Uuid| Locked {
        price: r.7,
        remaining: r.8,
        escrow_strands: r.10,
        escrow_item,
    };
    Ok((locked(b, Uuid::nil()), locked(s, escrow_item)))
}

/// Close order `order_id` as expired if it's still open and due at `now`.
/// `None` if it isn't (cancelled or filled meanwhile).
pub async fn expire(db: &PgPool, order_id: Uuid, now: DateTime<Utc>) -> Result<Option<Cancelled>, MarketError> {
    let mut tx = db.begin().await?;
    let due: Option<bool> = sqlx::query_scalar(
        "SELECT status = 'open' AND expires_at <= $2 FROM market_orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    if due != Some(true) {
        return Ok(None);
    }
    let closed = escrow::close(&mut tx, order_id, OrderStatus::Expired).await?;
    tx.commit().await?;
    Ok(Some(closed))
}

fn announce_fill(events: &broadcast::Sender<GameEvent>, fill: &Fill, done: Executed) {
    let _ = events.send(GameEvent::InventoryUpdated {
        player_id: fill.buyer,
        container_id: done.market_box,
        slots: Vec::new(),
    });
    let _ = events.send(GameEvent::CurrencyChanged {
        player_id: fill.seller,
        new_balance: done.seller_balance,
    });
    if let Some(balance) = done.buyer_balance {
        let _ = events.send(GameEvent::CurrencyChanged {
            player_id: fill.buyer,
            new_balance: balance,
        });
    }
    let _ = events.send(GameEvent::OrderFilled {
        player_id: fill.buyer,
        order: done.buy,
        trade: done.trade.clone(),
    });
    let _ = events.send(GameEvent::OrderFilled {
        player_id: fill.seller,
        order: done.sell,
        trade: done.trade.clone(),
    });
    let _ = events.send(GameEvent::MarketTrade { trade: done.trade });
}

async fn announce_closed(db: &PgPool, events: &broadcast::Sender<GameEvent>, closed: Cancelled) {
    let player_id = closed.order.player_id;
    if let Some(container_id) = closed.market_box {
        let _ = events.send(GameEvent::InventoryUpdated {
            player_id,
            container_id,
            slots: Vec::new(),
        });
    }
    if closed.refunded > 0 {
        let balance: Result<i64, _> = sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(db)
            .await;
        if let Ok(new_balance) = balance {
            let _ = events.send(GameEvent::CurrencyChanged { player_id, new_balance });
        }
    }
    let _ = events.send(GameEvent::OrderClosed {
        player_id,
        order: closed.order,
    });
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test market -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::market::tests::{balance, box_of, buy, player, sell};
    use crate::engine::market::{cancel, place};
    use crate::engine::units::test_units::{all_in, stack_of};
    use capitol_protocol::rest::PlaceOrder;

    async fn placed(db: &PgPool, market: &Market, player_id: Uuid, order: &PlaceOrder) -> MarketOrder {
        let order = place(db, player_id, order).await.unwrap();
        market.placed(&order);
        order
    }

    async fn order(db: &PgPool, id: Uuid) -> MarketOrder {
        let mut conn = db.acquire().await.unwrap();
        escrow::load(&mut conn, id).await.unwrap().unwrap()
    }

    async fn audit(db: &PgPool, player_id: Uuid) -> Vec<(String, i64)> {
        let mut rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT transaction_type, amount FROM currency_transactions WHERE player_id = $1")
                .bind(player_id)
                .fetch_all(db)
                .await
                .unwrap();
        rows.sort();
        rows
    }

    fn every_unit_buy(item_type: &str, price: i64, quantity: i32, grade: &str) -> PlaceOrder {
        PlaceOrder { grade_rule: Some(GradeRule::EveryUnit), ..buy(item_type, price, quantity, Some(grade)) }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_fill_moves_the_units_and_strands_and_writes_the_trade(db: PgPool) {
        let (events, mut heard) = broadcast::channel(64);
        let market = Market::default();
        let ada = player(&db, "ada", 100).await;
        let bea = player(&db, "bea", 1000).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::from_pairs([(50, 4), (80, 6)])).await;

        let ask = placed(&db, &market, ada.id, &sell("rope", stack, 10, 10, None)).await;
        // Every unit B or better, up to 12 each: only the six 80s qualify
        let bid = placed(&db, &market, bea.id, &every_unit_buy("rope", 12, 8, "B")).await;
        let cycle = run_cycle(&market, &db, &events, Utc::now()).await;
        assert_eq!(cycle, Cycle { fills: 1, expired: 0, refused: 0 });

        // The units are in Bea's box; she paid the resting 10, not her 12
        assert_eq!(all_in(&db, box_of(&db, bea.id).await, "rope").await, Units::uniform(6, 80));
        assert_eq!(balance(&db, ada.id).await, 100 - 1 + 60);
        assert_eq!(balance(&db, bea.id).await, 1000 - 96 - 1 + 12);
        assert_eq!(
            audit(&db, bea.id).await,
            vec![("market_escrow".into(), -96), ("market_fee".into(), -1), ("market_refund".into(), 12)]
        );
        assert_eq!(audit(&db, ada.id).await, vec![("market_fee".into(), -1), ("market_sale".into(), 60)]);

        // The ask rests with its 50s; the bid with 2 to go, escrowed at its own price
        let (ask_now, bid_now) = (order(&db, ask.id).await, order(&db, bid.id).await);
        assert_eq!((ask_now.status, ask_now.quantity_remaining), (OrderStatus::Open, 4));
        assert_eq!(ask_now.units, Units::uniform(4, 50));
        assert_eq!((bid_now.status, bid_now.quantity_remaining, bid_now.escrow_strands), (OrderStatus::Open, 2, 24));
        assert_eq!(market.with_book(|b| b.get(ask.id).map(|o| o.units.clone())), Some(Units::uniform(4, 50)));

        let trades: Vec<(String, i32, i32, i64, i32)> =
            sqlx::query_as("SELECT grade, quality, min_quality, price, quantity FROM market_trades")
                .fetch_all(&db)
                .await
                .unwrap();
        assert_eq!(trades, vec![("B".into(), 80, 80, 10, 6)]);

        // Each party hears of it, and the item's channel does
        let mut filled = Vec::new();
        let mut channel = 0;
        while let Ok(event) = heard.try_recv() {
            match event {
                GameEvent::OrderFilled { player_id, order, trade } => {
                    assert_eq!((trade.price, trade.quantity), (10, 6));
                    filled.push((player_id, order.id));
                }
                GameEvent::MarketTrade { .. } => channel += 1,
                _ => {}
            }
        }
        filled.sort();
        let mut expected = vec![(ada.id, ask.id), (bea.id, bid.id)];
        expected.sort();
        assert_eq!((filled, channel), (expected, 1));

        // The rest of the lot goes with its stack, and the order is filled
        let cy = player(&db, "cy", 1000).await;
        placed(&db, &market, cy.id, &buy("rope", 10, 4, Some("F"))).await;
        run_cycle(&market, &db, &events, Utc::now()).await;
        let ask_now = order(&db, ask.id).await;
        assert_eq!((ask_now.status, ask_now.quantity_remaining), (OrderStatus::Filled, 0));
        assert!(ask_now.units.is_empty());
        assert!(market.with_book(|b| b.get(ask.id).is_none()));
        assert_eq!(all_in(&db, box_of(&db, cy.id).await, "rope").await, Units::uniform(4, 50));
    }

    /// How many fills a second the DB takes: 300 sells, then 300 buys that
    /// each cross one. Run it in release for numbers worth quoting:
    /// `cargo test --release -p capitol-backend market_fill_throughput -- --ignored --nocapture`
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn market_fill_throughput(db: PgPool) {
        let (events, _heard) = broadcast::channel(4096);
        let market = Market::default();
        let ada = player(&db, "ada", 1_000_000).await;
        let bea = player(&db, "bea", 1_000_000).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::from_pairs([(40, 900), (85, 900)])).await;
        for i in 0..300 {
            placed(&db, &market, ada.id, &sell("rope", stack, 10 + i % 20, 6, None)).await;
        }
        for i in 0..300 {
            placed(&db, &market, bea.id, &buy("rope", 10 + i % 20, 6, Some(["F", "C"][i as usize % 2]))).await;
        }
        let started = std::time::Instant::now();
        let cycle = run_cycle(&market, &db, &events, Utc::now()).await;
        let took = started.elapsed();
        println!(
            "{} fills in {took:?}: {:.0} fills a second, {:?} each",
            cycle.fills,
            cycle.fills as f64 / took.as_secs_f64(),
            took / cycle.fills.max(1) as u32
        );
        assert!(cycle.fills > 150, "{cycle:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_fill_that_finds_the_order_cancelled_changes_nothing(db: PgPool) {
        let (events, _heard) = broadcast::channel(64);
        let market = Market::default();
        let ada = player(&db, "ada", 100).await;
        let bea = player(&db, "bea", 1000).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::uniform(5, 60)).await;
        let ask = placed(&db, &market, ada.id, &sell("rope", stack, 10, 5, None)).await;
        // Cancelled, and the book not told
        cancel(&db, ada.id, ask.id).await.unwrap();
        let bid = placed(&db, &market, bea.id, &buy("rope", 10, 5, None)).await;

        let before = balance(&db, bea.id).await;
        let cycle = run_cycle(&market, &db, &events, Utc::now()).await;
        assert_eq!((cycle.fills, cycle.refused), (0, 1));
        assert_eq!(balance(&db, bea.id).await, before);
        assert!(market.with_book(|b| b.get(ask.id).is_none()), "the book let the cancelled order go");
        assert!(market.with_book(|b| b.get(bid.id).is_some()));
        let trades: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM market_trades").fetch_one(&db).await.unwrap();
        assert_eq!(trades, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_buyer_with_a_full_box_sits_out_and_trades_once_theres_room(db: PgPool) {
        let (events, _heard) = broadcast::channel(256);
        let market = Market::default();
        let ada = player(&db, "ada", 100).await;
        let bea = player(&db, "bea", 1000).await;
        let full = box_of(&db, bea.id).await;
        let mut stacks = Vec::new();
        for slot in 0..48 {
            stacks.push(stack_of(&db, bea.id, full, slot, "fiber", &Units::uniform(1, 10)).await);
        }
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::uniform(3, 60)).await;
        let ask = placed(&db, &market, ada.id, &sell("rope", stack, 10, 3, None)).await;
        let bid = placed(&db, &market, bea.id, &buy("rope", 10, 3, None)).await;

        let now = Utc::now();
        let cycle = run_cycle(&market, &db, &events, now).await;
        assert_eq!((cycle.fills, cycle.refused), (0, 1));
        assert_eq!(order(&db, ask.id).await.quantity_remaining, 3);
        // Still sitting out a second later, with room made
        sqlx::query("DELETE FROM items WHERE id = $1").bind(stacks[7]).execute(&db).await.unwrap();
        assert_eq!(run_cycle(&market, &db, &events, now + chrono::Duration::seconds(1)).await, Cycle::default());
        let later = now + chrono::Duration::seconds(crate::engine::market_book::STALL_SECS);
        assert_eq!(run_cycle(&market, &db, &events, later).await.fills, 1);
        assert_eq!(order(&db, bid.id).await.status, OrderStatus::Filled);
        let slot: i32 = sqlx::query_scalar("SELECT slot_index FROM items WHERE container_id = $1 AND item_type = 'rope'")
            .bind(full)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(slot, 7);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn orders_past_their_expiry_come_back_and_dont_trade(db: PgPool) {
        let (events, mut heard) = broadcast::channel(64);
        let ada = player(&db, "ada", 100).await;
        let bea = player(&db, "bea", 1000).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::from_pairs([(20, 2), (70, 3)])).await;
        let ask = place(&db, ada.id, &sell("rope", stack, 10, 5, None)).await.unwrap();
        let bid = place(&db, bea.id, &buy("rope", 5, 4, None)).await.unwrap();

        // Loaded from the DB, as on startup, with the sell's units
        let market = Market::load(&db).await.unwrap();
        assert_eq!(market.with_book(|b| b.get(ask.id).map(|o| o.units.clone())), Some(ask.units.clone()));
        assert_eq!(market.with_book(|b| b.len()), 2);

        // A week on, a crossing buy placed then doesn't meet the expired ask
        let week = Utc::now() + chrono::Duration::seconds(market::ORDER_LIFETIME_SECS + 1);
        let cy = player(&db, "cy", 1000).await;
        let late = place(&db, cy.id, &buy("rope", 10, 5, None)).await.unwrap();
        sqlx::query("UPDATE market_orders SET expires_at = $2 WHERE id = $1")
            .bind(late.id)
            .bind(week + chrono::Duration::days(7))
            .execute(&db)
            .await
            .unwrap();
        market.placed(&order(&db, late.id).await);

        let cycle = run_cycle(&market, &db, &events, week).await;
        assert_eq!(cycle, Cycle { fills: 0, expired: 2, refused: 0 });
        let (ask_now, bid_now) = (order(&db, ask.id).await, order(&db, bid.id).await);
        assert_eq!((ask_now.status, bid_now.status), (OrderStatus::Expired, OrderStatus::Expired));
        assert_eq!(all_in(&db, box_of(&db, ada.id).await, "rope").await, Units::from_pairs([(20, 2), (70, 3)]));
        // The bid's escrow is back; the fee isn't
        assert_eq!(balance(&db, bea.id).await, 1000 - 1);
        assert_eq!(market.with_book(|b| b.len()), 1);

        let mut closed = Vec::new();
        while let Ok(event) = heard.try_recv() {
            if let GameEvent::OrderClosed { player_id, order } = event {
                assert_eq!(order.status, OrderStatus::Expired);
                closed.push(player_id);
            }
        }
        closed.sort();
        let mut expected = vec![ada.id, bea.id];
        expected.sort();
        assert_eq!(closed, expected);
    }
}

/// The depth channels (E3.3), without the DB: a subscriber that starts from a
/// snapshot and applies the diffs holds the server's book.
#[cfg(test)]
mod depth_tests {
    use super::*;
    use crate::engine::market_book::tests::{run, t0, Orders};
    use capitol_protocol::rules::market::apply_depth_diff;
    use chrono::Duration;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use std::collections::BTreeMap;

    const ITEMS: [&str; 3] = ["rope", "plank", "ingot"];

    /// A client watching some items: what it holds of each, by the protocol's
    /// rules (the doc of `ServerMessage::MarketDepth`).
    #[derive(Default)]
    struct Subscriber {
        books: BTreeMap<String, (u64, Vec<DepthLevel>)>,
    }

    impl Subscriber {
        fn subscribe(&mut self, market: &Market, item: &str) {
            let snapshot = market.depth(item);
            self.books.insert(item.to_string(), (snapshot.seq, snapshot.levels));
        }

        fn hear(&mut self, event: GameEvent) {
            let GameEvent::MarketDepth { item_type, seq, levels } = event else { return };
            let Some((held, book)) = self.books.get_mut(&item_type) else { return };
            if seq <= *held {
                return;
            }
            assert_eq!(seq, *held + 1, "{item_type}: a gap in the diffs");
            apply_depth_diff(book, &levels);
            *held = seq;
        }
    }

    fn deliver(heard: &mut broadcast::Receiver<GameEvent>, subscribers: &mut [Subscriber]) {
        while let Ok(event) = heard.try_recv() {
            for s in subscribers.iter_mut() {
                s.hear(event.clone());
            }
        }
    }

    #[test]
    fn diffs_keep_a_subscribers_book_identical_to_the_servers() {
        for seed in 0..30 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut orders = Orders::new();
            let market = Market::default();
            let (events, mut heard) = broadcast::channel(4096);
            let mut subscribers: Vec<Subscriber> = (0..3).map(|_| Subscriber::default()).collect();
            let now = t0() + Duration::days(1);

            for step in 0..200 {
                match rng.gen_range(0..10) {
                    // Someone watches an item: from now on they get its diffs
                    0 => {
                        let item = ITEMS[rng.gen_range(0..ITEMS.len())];
                        let who = rng.gen_range(0..subscribers.len());
                        subscribers[who].subscribe(&market, item);
                    }
                    // A cancel
                    1 | 2 => {
                        let ids: Vec<Uuid> = market.with_book(|b| {
                            ITEMS
                                .iter()
                                .flat_map(|i| [b.side(i, OrderSide::Buy), b.side(i, OrderSide::Sell)])
                                .flatten()
                                .map(|o| o.id)
                                .collect()
                        });
                        if !ids.is_empty() {
                            market.cancelled(ids[rng.gen_range(0..ids.len())]);
                        }
                    }
                    // A matching cycle
                    3 => {
                        run(&mut market.book(), now);
                    }
                    // A new order
                    _ => {
                        let item = ITEMS[rng.gen_range(0..ITEMS.len())].to_string();
                        let player = rng.gen_range(0..4u128);
                        let price = rng.gen_range(8..14);
                        let order = if rng.gen_bool(0.5) {
                            let lots = rng.gen_range(1..4);
                            let units = Units::from_pairs(
                                (0..lots).map(|_| (rng.gen_range(30..100), rng.gen_range(1..5))),
                            );
                            orders.sell(player, price, units)
                        } else {
                            let grade = ["A", "B", "C", "F"][rng.gen_range(0..4)];
                            let rule = if rng.gen_bool(0.5) { GradeRule::Average } else { GradeRule::EveryUnit };
                            orders.buy(player, price, rng.gen_range(1..8), grade, rule)
                        };
                        market.book().insert(BookOrder { item_type: item, ..order });
                    }
                }
                // Not every change is published at once: a route publishes
                // after its order, the matcher after a cycle
                if rng.gen_bool(0.6) {
                    market.publish(&events);
                }
                deliver(&mut heard, &mut subscribers);
                for (n, s) in subscribers.iter().enumerate() {
                    for (item, (seq, book)) in &s.books {
                        let published = market.depth(item);
                        assert_eq!(*seq, published.seq, "seed {seed} step {step}: subscriber {n} behind on {item}");
                        assert_eq!(book, &published.levels, "seed {seed} step {step}: subscriber {n}'s {item}");
                    }
                }
            }
            // Once published, what they hold is the book itself
            market.publish(&events);
            deliver(&mut heard, &mut subscribers);
            for s in &subscribers {
                for (item, (_, book)) in &s.books {
                    assert_eq!(book, &market.with_book(|b| b.depth(item)), "seed {seed}: {item}");
                }
            }
        }
    }

    #[test]
    fn depth_counts_a_sell_at_each_units_grade_and_a_buy_at_its_own() {
        let mut orders = Orders::new();
        let mut book = Book::new();
        book.insert(orders.sell(1, 11, Units::from_pairs([(92, 6), (70, 4)])));
        book.insert(orders.sell(2, 11, Units::from_pairs([(95, 1)])));
        book.insert(orders.buy(3, 9, 5, "B", GradeRule::EveryUnit));
        book.insert(orders.buy(4, 9, 2, "B", GradeRule::EveryUnit));
        book.insert(orders.buy(5, 10, 3, "F", GradeRule::Average));
        let levels: Vec<_> = book
            .depth("rope")
            .into_iter()
            .map(|l| (l.side, l.price, l.grade, l.grade_rule, l.quantity, l.orders))
            .collect();
        assert_eq!(
            levels,
            [
                (OrderSide::Buy, 10, "F".to_string(), Some(GradeRule::Average), 3, 1),
                (OrderSide::Buy, 9, "B".to_string(), Some(GradeRule::EveryUnit), 7, 2),
                (OrderSide::Sell, 11, "A".to_string(), None, 7, 2),
                (OrderSide::Sell, 11, "C".to_string(), None, 4, 1),
            ]
        );
    }

    #[test]
    fn a_new_market_publishes_its_book_at_seq_zero_and_counts_from_there() {
        let mut orders = Orders::new();
        let mut book = Book::new();
        book.insert(orders.buy(1, 9, 5, "C", GradeRule::Average));
        let market = Market::from_book(book);
        let (events, mut heard) = broadcast::channel(16);
        assert_eq!(market.depth("rope").seq, 0);
        assert_eq!(market.depth("rope").levels.len(), 1);
        // Nothing changed, so nothing is said
        market.publish(&events);
        assert!(heard.try_recv().is_err());
        market.placed(&MarketOrder {
            id: Uuid::new_v4(),
            player_id: Uuid::from_u128(2),
            side: OrderSide::Buy,
            item_type: "rope".into(),
            price: 9,
            quantity_total: 2,
            quantity_remaining: 2,
            status: OrderStatus::Open,
            min_grade: Some("C".into()),
            grade_rule: Some(GradeRule::Average),
            units: Units::new(),
            escrow_strands: 18,
            fee: 1,
            created_at: t0(),
            expires_at: t0() + Duration::days(7),
        });
        market.publish(&events);
        match heard.try_recv() {
            Ok(GameEvent::MarketDepth { item_type, seq, levels }) => {
                assert_eq!((item_type.as_str(), seq), ("rope", 1));
                assert_eq!((levels.len(), levels[0].quantity, levels[0].orders), (1, 7, 2));
            }
            other => panic!("expected a depth diff, got {other:?}"),
        }
        assert_eq!(market.best_prices(), [("rope".to_string(), Some(9), None)]);
    }
}
