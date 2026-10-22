//! The market's matching engine (E3.2, economy plan D7): every second, close
//! the orders past their expiry and write the fills [`Book`] proposes, one
//! transaction a fill (TDD §5.2, write-through).
//!
//! The book is in memory behind a plain mutex that's never held across an
//! await. The book proposes a batch of fills ([`Book::propose`], #237): the
//! ones it would make one after another, each applied before it's written,
//! for as long as no player is in two. The lock is dropped, and each fill is
//! written in a transaction of its own on a connection of its own, side by
//! side, up to its commit. Those before the first that failed are committed
//! together, so Postgres flushes them as one; those after it are rolled back,
//! and the book [`Book::rewind`]s to just before the failed one. So what's
//! written is exactly what fills one at a time would have written. A fill
//! that fails changes nothing, in the DB or the book:
//! - an order no longer as the book had it (cancelled under it, say) is
//!   reloaded from the DB, and matching goes on;
//! - a buyer whose market box is full sits out for `STALL_SECS`;
//! - a DB error ends the cycle, and the next one tries again. A commit that
//!   fails reloads its orders, since the book had applied it.
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
//! on them. A burst over what the DB takes makes a cycle run long rather than
//! lose anything.
//!
//! A fill is some eleven round trips on its connection (#208): lock the pair,
//! the escrow's units (#238), the buyer's box, deliver (three), both orders
//! and the trade in one statement, a statement per balance moved, and the
//! commit. One after another that's about 230 fills a second on the local
//! stack, and the commit's flush is nearly half of it; side by side, the
//! flushes are shared (#237).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tracing::Instrument;

use capitol_protocol::rest::{DepthLevel, MarketOrder, MarketTrade};
use capitol_protocol::rules::market::{self, GradeRule, OrderSide, OrderStatus};
use capitol_protocol::rules::quality_grade;
use capitol_protocol::rules::stacking::Units;
use chrono::{DateTime, Utc};
use futures::future::join_all;
use sqlx::pool::PoolConnection;
use sqlx::{Connection, PgConnection, PgPool, Postgres, Transaction};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::currency::{add_strands_in, TransactionType};
use super::events::GameEvent;
use super::market::{self as escrow, order_from_row, Cancelled, MarketError, OrderRow, ORDER_COLUMNS};
use super::market_book::{Book, BookOrder, Fill};
use super::units::{self, Marked, NewStack};

/// How often matching runs: the tick's 10-tick cadence (TDD §3.2).
pub const CYCLE_MS: u64 = 1000;

/// The most fills written side by side (#237), each on a connection of its
/// own: enough for their commits' flushes to share a few, and few enough to
/// leave most of the pool (16) to the routes and the tick.
pub const BATCH: usize = 6;

/// How long the matcher waits for a connection past its first: a batch is
/// only as wide as the connections it can have at once (#237).
const SPARE_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

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
    async fn reload(&self, conn: &mut PgConnection, order_id: Uuid) -> Result<(), sqlx::Error> {
        let order = escrow::load(conn, order_id).await?;
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

/// How many cycles apart the court moves delivered units waiting for room
/// in their posters' market boxes (Commons S2.1): every 10 s.
const WAITING_CYCLES: u64 = 10;

/// Run the matcher forever, a cycle a second. The court's commissions ride
/// the same loop (Commons S2.1): each cycle expires what's come due, and
/// every [`WAITING_CYCLES`] it moves delivered units on as boxes have room.
pub async fn run_market_loop(market: Arc<Market>, db: PgPool, events: broadcast::Sender<GameEvent>) {
    tracing::info!("Market matching every {CYCLE_MS}ms, {} orders on the book", market.with_book(|b| b.len()));
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(CYCLE_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut n: u64 = 0;
    loop {
        n = n.wrapping_add(1);
        interval.tick().await;
        let timed = super::load::Loop::Market.timed();
        let cycle = run_cycle(&market, &db, &events, Utc::now())
            .instrument(tracing::info_span!("loop.market"))
            .await;
        court_cycle(&db, &events, n.is_multiple_of(WAITING_CYCLES))
            .instrument(tracing::info_span!("loop.court"))
            .await;
        drop(timed);
        if cycle != Cycle::default() {
            tracing::debug!("Market: {cycle:?}");
        }
    }
}

/// The court's part of a cycle: expire the commissions due, and with
/// `waiting`, move delivered units into boxes that have room now.
async fn court_cycle(db: &PgPool, events: &broadcast::Sender<GameEvent>, waiting: bool) {
    match super::commissions::expire_due(db, Utc::now()).await {
        Ok(closed) => crate::routes::commissions::announce_closed(db, events, closed).await,
        Err(e) => tracing::warn!("Court: expiring commissions: {}", e.message()),
    }
    if waiting {
        match super::commissions::move_waiting(db).await {
            Ok(boxes) => {
                for (player_id, container_id) in boxes {
                    let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
                }
            }
            Err(e) => tracing::warn!("Court: moving delivered units on: {e}"),
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
                let reloaded = match db.acquire().await {
                    Ok(mut conn) => market.reload(&mut conn, order_id).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = reloaded {
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

    // The cycle's connections, one a fill in a batch, each fill beginning on
    // its own rather than acquiring (and checking) one (#208)
    let mut held: Vec<PoolConnection<Postgres>> = Vec::new();
    // As wide as the pool lets it be without waiting
    let mut width = BATCH;
    loop {
        let mut batch = market.book().propose(now, width);
        if batch.is_empty() {
            market.book().settle();
            break;
        }
        while held.len() < batch.len() {
            if held.is_empty() {
                match db.acquire().await {
                    Ok(conn) => held.push(conn),
                    Err(e) => {
                        tracing::warn!("Market: no connection to fill on: {e}");
                        break;
                    }
                }
                continue;
            }
            // Past the first, only a connection that's to hand: the routes
            // and the tick need the pool more than a batch needs to be wide
            match tokio::time::timeout(SPARE_WAIT, db.acquire()).await {
                Ok(Ok(conn)) => held.push(conn),
                _ => {
                    width = held.len();
                    break;
                }
            }
        }
        if held.len() < batch.len() {
            // Fewer connections than fills: the rest wait their turn
            let mut book = market.book();
            if let Some((_, mark)) = batch.get(held.len()) {
                book.rewind(*mark);
            }
            batch.truncate(held.len());
            if batch.is_empty() {
                book.settle();
                break;
            }
        }

        // Each fill up to its commit, side by side: they share no player, so
        // no row, and none waits on another
        let prepared = join_all(held.iter_mut().zip(&batch).map(|(conn, (fill, _))| prepare_fill(conn, fill))).await;
        // Those before the first that failed are what fills one at a time
        // would have written; those after it were matched as if it hadn't
        // failed, so they go back
        let mut ready = Vec::new();
        let mut failed = None;
        let mut undone = Vec::new();
        for (i, result) in prepared.into_iter().enumerate() {
            match result {
                Ok(prepared) if failed.is_none() => ready.push(prepared),
                Ok((tx, _)) => undone.push(tx),
                Err(e) if failed.is_none() => failed = Some((i, e)),
                Err(_) => {}
            }
        }
        // Their commits flush together
        let committed = join_all(ready.into_iter().map(|(tx, done)| async move { tx.commit().await.map(|()| done) })).await;
        for result in join_all(undone.into_iter().map(|tx| tx.rollback())).await {
            if let Err(e) = result {
                tracing::warn!("Market: rolling back a fill after one that failed: {e}");
            }
        }

        {
            let mut book = market.book();
            if let Some((k, _)) = &failed {
                book.rewind(batch[*k].1);
            }
            book.settle();
        }
        let mut lost = false;
        for ((fill, _), result) in batch.iter().zip(committed) {
            match result {
                Ok(done) => {
                    cycle.fills += 1;
                    announce_fill(events, fill, done);
                }
                // Applied to the book but maybe not written: the DB says
                Err(e) => {
                    tracing::warn!("Market: committing {} against {}: {e}", fill.buy, fill.sell);
                    lost = true;
                    for id in [fill.buy, fill.sell] {
                        if let Err(e) = market.reload(&mut held[0], id).await {
                            tracing::warn!("Market: reloading order {id}: {e}");
                        }
                    }
                }
            }
        }
        if lost {
            return cycle;
        }
        let Some((k, error)) = failed else { continue };
        let fill = &batch[k].0;
        match error {
            MarketError::MarketBoxFull => {
                cycle.refused += 1;
                market.book().stall(fill.buy, now);
            }
            MarketError::Changed => {
                cycle.refused += 1;
                for id in [fill.buy, fill.sell] {
                    if let Err(e) = market.reload(&mut held[0], id).await {
                        tracing::warn!("Market: reloading order {id}: {e}");
                        return cycle;
                    }
                }
            }
            e => {
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

/// Write `fill` in a transaction on `conn`, checking first that both orders
/// are as the book had them, and hand back the transaction uncommitted: the
/// matcher commits a batch's fills together, or rolls one back (#237).
/// Nothing changes unless all of it does, and an error rolls it back.
async fn prepare_fill<'c>(
    conn: &'c mut PgConnection,
    fill: &Fill,
) -> Result<(Transaction<'c, Postgres>, Executed), MarketError> {
    let mut tx = conn.begin().await?;
    let (buy, sell) = lock_pair(&mut tx, fill).await?;

    // What the sell's escrow keeps
    let mut left = sell.marked.units();
    if left.take_units(&fill.units).is_none() {
        return Err(MarketError::Changed);
    }
    let whole = left.is_empty();

    let market_box = escrow::market_box(&mut tx, fill.buyer).await?;
    let moved = deliver(&mut tx, fill, sell.escrow_item, &sell.marked, &left, market_box).await?;

    // The Strands: the escrow at the buy's price pays the seller at the
    // fill's, and the buyer gets the difference
    let paid = market::order_value(fill.price, fill.quantity);
    let escrowed = market::order_value(buy.price, fill.quantity);
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
    // Both orders and the trade in one statement, and the orders as they
    // stand after it. A filled buy's escrow goes to 0: what's left over its
    // price × quantity is refunded below, so no Strand stays in a closed order.
    let rows: Vec<OrderRow> = sqlx::query_as(&format!(
        "WITH trade AS (
             INSERT INTO market_trades
                 (id, buy_order_id, sell_order_id, item_type, grade, quality, min_quality, price, quantity, at, maker_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $13)
         )
         UPDATE market_orders SET
             quantity_remaining = quantity_remaining - $9,
             escrow_strands = CASE
                 WHEN id <> $2 THEN escrow_strands
                 WHEN quantity_remaining = $9 THEN 0
                 ELSE escrow_strands - $11 END,
             escrow_item_id = CASE WHEN id = $3 AND $12 THEN NULL ELSE escrow_item_id END,
             status = CASE WHEN quantity_remaining = $9 THEN 'filled' ELSE status END
         WHERE id IN ($2, $3)
         RETURNING {ORDER_COLUMNS}"
    ))
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
    .bind(escrowed)
    .bind(whole)
    // Whose work it was, for the Gazette (#231)
    .bind(moved.lead_maker())
    .fetch_all(&mut *tx)
    .await?;
    let (mut buy_now, mut sell_now) = (None, None);
    for row in rows {
        if row.0 == fill.buy {
            buy_now = Some(order_from_row(row, Units::new())?);
        } else {
            sell_now = Some(order_from_row(row, left.clone())?);
        }
    }
    let (Some(buy_now), Some(sell_now)) = (buy_now, sell_now) else {
        return Err(MarketError::Changed);
    };

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
        Some(balance)
    } else {
        None
    };
    Ok((
        tx,
        Executed {
            buy: buy_now,
            sell: sell_now,
            trade,
            market_box,
            seller_balance,
            buyer_balance,
        },
    ))
}

/// Move `fill`'s units from the sell's escrow stack `escrow_item` (locked,
/// leaving `left`) into the buyer's market box: onto the first stack of the
/// type there if it stacks and has room (D2: nothing is diluted), or into the
/// first free slot, as the escrow stack itself if it's all of them (a tool
/// keeps its wear) or a stack split off it. `MarketBoxFull`, and nothing
/// moved, if there's no room. Returns the units moved with their makers'
/// marks (#231); `escrowed` is everything the escrow stack holds.
///
/// This is `escrow::stow` for a fill, which knows the units already: some
/// four round trips where splitting a stack off into the escrow and stowing
/// it took a dozen (#208).
async fn deliver(
    conn: &mut PgConnection,
    fill: &Fill,
    escrow_item: Uuid,
    escrowed: &Marked,
    left: &Units,
    market_box: Uuid,
) -> Result<Marked, MarketError> {
    // The first stack of the type in the box, if the type stacks, with its
    // units, both locked
    let target: Vec<(Uuid, i16, i32)> = sqlx::query_as(
        "SELECT u.item_id, u.quality, u.count FROM item_units u
         WHERE u.item_id = (
             SELECT i.id FROM items i JOIN item_types t ON t.id = i.item_type
             WHERE i.container_id = $1 AND i.item_type = $2 AND t.stackable
             ORDER BY i.slot_index NULLS LAST, i.id
             LIMIT 1
             FOR UPDATE OF i
         )
         ORDER BY u.quality
         FOR UPDATE",
    )
    .bind(market_box)
    .bind(&fill.item_type)
    .fetch_all(&mut *conn)
    .await?;
    if let Some(&(stack, ..)) = target.first() {
        let mut held = Units::from_pairs(target.iter().map(|&(_, q, c)| (q as i32, c)));
        if held.merge(&fill.units) {
            let moved = units::write_take(conn, escrow_item, &fill.units, left).await?;
            units::add(conn, stack, &moved).await?;
            return Ok(moved);
        }
    }

    let slot = escrow::free_slot(conn, market_box).await?.ok_or(MarketError::MarketBoxFull)?;
    if left.is_empty() {
        sqlx::query("UPDATE items SET container_id = $2, slot_index = $3, owner_id = $4 WHERE id = $1")
            .bind(escrow_item)
            .bind(market_box)
            .bind(slot)
            .bind(fill.buyer)
            .execute(&mut *conn)
            .await?;
        return Ok(escrowed.clone());
    }
    let durability: Option<i32> = sqlx::query_scalar("SELECT durability FROM items WHERE id = $1")
        .bind(escrow_item)
        .fetch_one(&mut *conn)
        .await?;
    let moved = units::write_take(conn, escrow_item, &fill.units, left).await?;
    units::insert_stack(
        conn,
        NewStack {
            item_type: &fill.item_type,
            owner_id: Some(fill.buyer),
            container_id: Some(market_box),
            slot_index: Some(slot),
            durability,
        },
        &moved,
    )
    .await?;
    Ok(moved)
}

/// An order as a fill finds it, locked.
struct Locked {
    price: i64,
    remaining: i32,
    escrow_strands: i64,
    escrow_item: Uuid,
    /// What a sell's escrow stack holds, with its makers' marks
    marked: Marked,
}

/// Lock both of `fill`'s orders (by id, so two fills never lock in opposite
/// orders) and check each is open, as the book had it, and can carry it.
///
/// The sell's escrow units (and their marks) are read in a statement of their
/// own, after the lock (#238). Nothing writes an escrow stack without first
/// locking its order, so they're as the last holder of the lock left them.
/// Read in the locking statement they weren't: when `FOR UPDATE` waits,
/// Postgres re-reads the locked row, but the rest of the statement keeps the
/// snapshot from before the wait. A fill would then work from units that
/// were no longer there. That was harmless only while every writer of an
/// open order's escrow but the matcher closed the order. An amend or a
/// partial cancel would have let a fill hand the buyer units nobody sold.
async fn lock_pair(conn: &mut PgConnection, fill: &Fill) -> Result<(Locked, Locked), MarketError> {
    type Row = (
        Uuid,
        Uuid,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i32,
        Option<Uuid>,
        i64,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, player_id, side, status, item_type, min_grade, grade_rule, price,
                quantity_remaining, escrow_item_id, escrow_strands
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
    let marked = units::marked_of(conn, escrow_item).await?;
    let locked = |r: &Row, escrow_item: Uuid, marked: Marked| Locked {
        price: r.7,
        remaining: r.8,
        escrow_strands: r.10,
        escrow_item,
        marked,
    };
    Ok((locked(b, Uuid::nil(), Marked::default()), locked(s, escrow_item, marked)))
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
    use crate::engine::market::cancel;
    use crate::engine::market::tests::place;
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

    /// A fill hands on whose work the units were (#231), and records the
    /// maker of most of them for the Gazette
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_fill_keeps_the_makers_marks_and_records_the_maker(db: PgPool) {
        use crate::engine::units::test_units::{all_marked, marked_stack_of};
        use crate::engine::units::Marked;
        let (events, _heard) = broadcast::channel(64);
        let market = Market::default();
        let ada = player(&db, "ada", 100).await;
        let bea = player(&db, "bea", 1000).await;
        let cy = player(&db, "cy", 0).await;
        let rope = Marked::from_rows([(80, None, 2), (80, Some(cy.id), 4), (50, Some(ada.id), 3)]);
        let stack = marked_stack_of(&db, ada.id, ada.inventory, 0, "rope", &rope).await;
        placed(&db, &market, ada.id, &sell("rope", stack, 10, 9, None)).await;

        // The six 80s, most of them Cy's; then the rest, Ada's own, the escrow stack whole
        placed(&db, &market, bea.id, &every_unit_buy("rope", 12, 6, "B")).await;
        assert_eq!(run_cycle(&market, &db, &events, Utc::now()).await.fills, 1);
        assert_eq!(all_marked(&db, bea.id, "rope").await, Marked::from_rows([(80, None, 2), (80, Some(cy.id), 4)]));
        placed(&db, &market, bea.id, &buy("rope", 12, 3, Some("F"))).await;
        assert_eq!(run_cycle(&market, &db, &events, Utc::now()).await.fills, 1);
        assert_eq!(all_marked(&db, bea.id, "rope").await, rope);

        let makers: Vec<(i32, Option<Uuid>)> =
            sqlx::query_as("SELECT quantity, maker_id FROM market_trades ORDER BY quality DESC")
                .fetch_all(&db)
                .await
                .unwrap();
        assert_eq!(makers, vec![(6, Some(cy.id)), (3, Some(ada.id))]);
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

    /// The same with 30 sellers and 30 buyers, so fills go side by side in
    /// batches of [`BATCH`] (#237). Run it in release:
    /// `cargo test --release -p capitol-backend market_fill_throughput -- --ignored --nocapture`
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn market_fill_throughput_many_players(db: PgPool) {
        let (events, _heard) = broadcast::channel(8192);
        let market = Market::default();
        let mut sellers = Vec::new();
        let mut buyers = Vec::new();
        for i in 0..30 {
            let seller = player(&db, &format!("seller{i}"), 1000).await;
            let stack =
                stack_of(&db, seller.id, seller.inventory, 0, "rope", &Units::from_pairs([(40, 30), (85, 30)])).await;
            sellers.push((seller.id, stack));
            buyers.push(player(&db, &format!("buyer{i}"), 100_000).await.id);
        }
        for i in 0..300 {
            let (seller, stack) = sellers[i % 30];
            placed(&db, &market, seller, &sell("rope", stack, 10 + (i % 20) as i64, 6, None)).await;
        }
        for i in 0..300 {
            let grade = ["F", "C"][i % 2];
            placed(&db, &market, buyers[i % 30], &buy("rope", 10 + (i % 20) as i64, 6, Some(grade))).await;
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
        // Every unit is still somewhere, and what the trades say was sold is
        // what the sells say they sold
        let units: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(u.count), 0)::BIGINT FROM item_units u JOIN items i ON i.id = u.item_id
             WHERE i.item_type = 'rope'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(units, 30 * 60);
        let traded: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(quantity), 0)::BIGINT FROM market_trades")
            .fetch_one(&db)
            .await
            .unwrap();
        let sold: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(quantity_total - quantity_remaining), 0)::BIGINT FROM market_orders WHERE side = 'sell'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(traded, sold);
    }

    /// A batch whose second fill finds its buyer's box full writes the first,
    /// and rolls back those after it to match again without it (#237): the
    /// next batch trades them as fills one at a time would have.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_batch_writes_the_fills_before_one_that_fails_and_retries_those_after(db: PgPool) {
        let (events, mut heard) = broadcast::channel(1024);
        let market = Market::default();
        // Four asks at 10, 11, 12 and 13, each a seller's own
        let mut asks = Vec::new();
        for i in 0..4 {
            let seller = player(&db, &format!("seller{i}"), 100).await;
            let stack = stack_of(&db, seller.id, seller.inventory, 0, "rope", &Units::uniform(2, 60)).await;
            asks.push(placed(&db, &market, seller.id, &sell("rope", stack, 10 + i, 2, None)).await);
        }
        let mut bidders = Vec::new();
        for i in 0..4 {
            bidders.push(player(&db, &format!("bidder{i}"), 1000).await);
        }
        // The second bidder's box is full
        let full = box_of(&db, bidders[1].id).await;
        for slot in 0..48 {
            stack_of(&db, bidders[1].id, full, slot, "fiber", &Units::uniform(1, 10)).await;
        }
        let mut bids = Vec::new();
        for bidder in &bidders {
            bids.push(placed(&db, &market, bidder.id, &buy("rope", 20, 2, None)).await);
        }

        let cycle = run_cycle(&market, &db, &events, Utc::now()).await;
        assert_eq!((cycle.fills, cycle.refused), (3, 1), "{cycle:?}");
        // Bidder 0 took the 10s. Bidder 1 sat out, so bidder 2 took the 11s
        // bidder 1 had been matched to, and bidder 3 the 12s
        let filled: Vec<(Uuid, Uuid, i64)> =
            sqlx::query_as("SELECT buy_order_id, sell_order_id, price FROM market_trades ORDER BY price")
                .fetch_all(&db)
                .await
                .unwrap();
        assert_eq!(
            filled,
            vec![(bids[0].id, asks[0].id, 10), (bids[2].id, asks[1].id, 11), (bids[3].id, asks[2].id, 12)]
        );
        // The book is the DB's
        for o in bids.iter().chain(&asks) {
            let now = order(&db, o.id).await;
            let open = now.status == OrderStatus::Open;
            let on_book = market.with_book(|b| b.get(o.id).map(|b| b.remaining));
            assert_eq!(on_book, open.then_some(now.quantity_remaining), "order {}", o.id);
        }
        assert_eq!(order(&db, bids[1].id).await.quantity_remaining, 2);
        // Each written fill is heard once, and the undone ones not at all
        let mut trades = 0;
        while let Ok(event) = heard.try_recv() {
            if let GameEvent::MarketTrade { .. } = event {
                trades += 1;
            }
        }
        assert_eq!(trades, 3);
    }

    /// A fill that waits on a sell's lock sees its escrow as the holder left
    /// it (#238). Here the holder adds two 90s to an open sell, as an amend
    /// would. Read in the lock's statement, the escrow was the snapshot from
    /// before the wait, five 60s and nothing left after the fill's five, so
    /// the whole stack went to the buyer, the 90s with it.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_fill_that_waits_on_the_lock_sees_the_escrow_as_it_was_left(db: PgPool) {
        use crate::engine::units::Marked;
        let (events, _heard) = broadcast::channel(64);
        let market = Arc::new(Market::default());
        let ada = player(&db, "ada", 100).await;
        let bea = player(&db, "bea", 1000).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::uniform(5, 60)).await;
        let ask = placed(&db, &market, ada.id, &sell("rope", stack, 10, 5, None)).await;
        placed(&db, &market, bea.id, &buy("rope", 10, 5, None)).await;

        // Hold the sell's lock and add the 90s, uncommitted
        let mut holder = db.begin().await.unwrap();
        let escrow: Uuid = sqlx::query_scalar("SELECT escrow_item_id FROM market_orders WHERE id = $1 FOR UPDATE")
            .bind(ask.id)
            .fetch_one(&mut *holder)
            .await
            .unwrap();
        units::add(&mut holder, escrow, &Marked::by(None, &Units::uniform(2, 90))).await.unwrap();
        sqlx::query("UPDATE market_orders SET quantity_total = quantity_total + 2, quantity_remaining = quantity_remaining + 2 WHERE id = $1")
            .bind(ask.id)
            .execute(&mut *holder)
            .await
            .unwrap();

        // The fill starts, and waits on the lock
        let cycle = tokio::spawn({
            let (market, db) = (market.clone(), db.clone());
            async move { run_cycle(&market, &db, &events, Utc::now()).await }
        });
        let waiting = async {
            loop {
                let waits: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock'",
                )
                .fetch_one(&db)
                .await
                .unwrap();
                if waits > 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), waiting).await.expect("the fill never waited");
        holder.commit().await.unwrap();

        assert_eq!(cycle.await.unwrap().fills, 1);
        assert_eq!(all_in(&db, box_of(&db, bea.id).await, "rope").await, Units::uniform(5, 60));
        let ask_now = order(&db, ask.id).await;
        assert_eq!((ask_now.status, ask_now.quantity_remaining), (OrderStatus::Open, 2));
        assert_eq!(ask_now.units, Units::uniform(2, 90));
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
