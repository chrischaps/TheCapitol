//! The market's escrow (E3.1, economy plan D7): placing and cancelling orders.
//! Matching them is `market_book` and `market_matcher` (E3.2), and the routes
//! that call these are E3.3.
//!
//! Each operation is one transaction, and every Strand it moves goes through
//! the currency audit log:
//! - [`place`] a sell: its units leave the seller's stack for a stack of their
//!   own in the shared escrow (no slot, no owner, so nobody can move them), and
//!   the listing fee is charged (`market_fee`).
//! - [`place`] a buy: `price × quantity` goes into the order's escrow
//!   (`market_escrow`), and the fee is charged on top.
//! - [`cancel`]: what's left comes back, a sell's units to the seller's market
//!   box and a buy's Strands to their balance (`market_refund`). The fee stays
//!   spent: it's the market's currency sink.
//!
//! So for any run of these, the Strands and units held plus those in escrow
//! plus the fees are what there were at the start (the conservation test).
//! The fee and bounds are `rules::market`, shared with the clients.
#![allow(dead_code)] // the market's model; the routes (#169) call place and cancel

use capitol_protocol::rest::{MarketOrder, PlaceOrder};
use capitol_protocol::rules::market::{self, GradeRule, OrderError, OrderSide, OrderStatus};
use capitol_protocol::rules::stacking::{Pick, Units};
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::currency::{add_strands_in, subtract_strands_in, TransactionType};
use super::units::{self, NewStack};

/// Why the market refused.
#[derive(Debug)]
pub enum MarketError {
    /// The price, quantity or grade is out of bounds
    Order(OrderError),
    /// No such item type
    UnknownItemType,
    /// A sell that names no stack
    NoStack,
    /// The stack isn't the seller's, in their inventory or market box
    NotYourStack,
    /// The stack isn't of the order's item type
    WrongItemType,
    /// Named units that don't add up to the quantity
    BadQualities,
    /// The stack doesn't hold the units asked for
    NotHeld,
    /// Not enough Strands for the fee, or the escrow
    Funds(String),
    /// No such order of the player's
    NotFound,
    /// It's already filled, cancelled or expired
    NotOpen,
    /// The market box has no room for what's coming back
    MarketBoxFull,
    /// A fill found an order not as the book had it (E3.2): the book reloads it
    Changed,
    Db(sqlx::Error),
}

impl MarketError {
    pub fn message(&self) -> String {
        match self {
            MarketError::Order(e) => e.message().to_string(),
            MarketError::UnknownItemType => "There's no such item".to_string(),
            MarketError::NoStack => "Say which stack to sell from".to_string(),
            MarketError::NotYourStack => "That stack isn't in your inventory or market box".to_string(),
            MarketError::WrongItemType => "That stack isn't the item the order is for".to_string(),
            MarketError::BadQualities => "The qualities named don't add up to the quantity".to_string(),
            MarketError::NotHeld => "That stack doesn't hold those units".to_string(),
            MarketError::Funds(m) => m.clone(),
            MarketError::NotFound => "No such order".to_string(),
            MarketError::NotOpen => "That order is no longer open".to_string(),
            MarketError::MarketBoxFull => "Your market box is full: collect from it first".to_string(),
            MarketError::Changed => "The order changed before it could trade".to_string(),
            MarketError::Db(e) => format!("Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for MarketError {
    fn from(e: sqlx::Error) -> Self {
        MarketError::Db(e)
    }
}

impl From<OrderError> for MarketError {
    fn from(e: OrderError) -> Self {
        MarketError::Order(e)
    }
}

/// Place `order` for `player_id`: take its escrow and fee, and put it on the
/// book. Returns it as placed. Nothing changes unless all of it does.
pub async fn place(db: &PgPool, player_id: Uuid, order: &PlaceOrder) -> Result<MarketOrder, MarketError> {
    market::validate(order.price, order.quantity)?;
    let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM item_types WHERE id = $1)")
        .bind(&order.item_type)
        .fetch_one(db)
        .await?;
    if !known {
        return Err(MarketError::UnknownItemType);
    }

    let id = Uuid::new_v4();
    let created_at = Utc::now();
    let expires_at = created_at + Duration::seconds(market::ORDER_LIFETIME_SECS);
    let value = market::order_value(order.price, order.quantity);
    let fee = market::listing_fee(value);

    let mut tx = db.begin().await?;
    let (escrow_item_id, escrow_strands, grade) = match order.side {
        OrderSide::Sell => {
            let item_id = order.item_id.ok_or(MarketError::NoStack)?;
            let escrowed = escrow_units(&mut tx, player_id, item_id, order).await?;
            (Some(escrowed), 0, None)
        }
        OrderSide::Buy => {
            let min_grade = order.min_grade.clone().unwrap_or_else(|| "F".to_string());
            if market::grade_floor(&min_grade).is_none() {
                return Err(OrderError::BadGrade.into());
            }
            subtract_strands_in(
                &mut tx,
                player_id,
                value,
                TransactionType::MarketEscrow,
                Some(&format!("Buy {} {} at {} each", order.quantity, order.item_type, order.price)),
                Some(id),
            )
            .await
            .map_err(MarketError::Funds)?;
            (None, value, Some((min_grade, order.grade_rule.unwrap_or_default())))
        }
    };
    subtract_strands_in(
        &mut tx,
        player_id,
        fee,
        TransactionType::MarketFee,
        Some(&format!("Listing fee: {} {} {}", order.side.as_str(), order.quantity, order.item_type)),
        Some(id),
    )
    .await
    .map_err(MarketError::Funds)?;

    sqlx::query(
        "INSERT INTO market_orders
             (id, player_id, side, item_type, min_grade, grade_rule, price, quantity_total,
              quantity_remaining, status, escrow_item_id, escrow_strands, fee, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, 'open', $9, $10, $11, $12, $13)",
    )
    .bind(id)
    .bind(player_id)
    .bind(order.side.as_str())
    .bind(&order.item_type)
    .bind(grade.as_ref().map(|(g, _)| g.as_str()))
    .bind(grade.as_ref().map(|(_, r)| r.as_str()))
    .bind(order.price)
    .bind(order.quantity)
    .bind(escrow_item_id)
    .bind(escrow_strands)
    .bind(fee)
    .bind(created_at)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    let placed = load(&mut tx, id).await?.ok_or(MarketError::NotFound)?;
    tx.commit().await?;
    Ok(placed)
}

/// Move a sell's units from the seller's stack `item_id` into a new stack in
/// the escrow. Returns the escrow stack.
async fn escrow_units(
    conn: &mut PgConnection,
    player_id: Uuid,
    item_id: Uuid,
    order: &PlaceOrder,
) -> Result<Uuid, MarketError> {
    let stack: Option<(String, Option<Uuid>, Option<String>, Option<i32>)> = sqlx::query_as(
        "SELECT i.item_type, i.owner_id, c.container_type, i.durability
         FROM items i LEFT JOIN containers c ON c.id = i.container_id
         WHERE i.id = $1 FOR UPDATE OF i",
    )
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((item_type, owner, container_type, durability)) = stack else {
        return Err(MarketError::NotYourStack);
    };
    let sellable_from = matches!(container_type.as_deref(), Some("player_inventory" | "market_box"));
    if owner != Some(player_id) || !sellable_from {
        return Err(MarketError::NotYourStack);
    }
    if item_type != order.item_type {
        return Err(MarketError::WrongItemType);
    }

    let held = units::units_of(conn, item_id).await?;
    let wanted = match &order.qualities {
        Some(named) if named.count() != order.quantity as i64 => return Err(MarketError::BadQualities),
        Some(named) => named.clone(),
        None => held.pick(order.quantity, Pick::default()).ok_or(MarketError::NotHeld)?,
    };
    let taken = units::take_units(conn, item_id, &wanted).await?.ok_or(MarketError::NotHeld)?;

    let escrow: Uuid = sqlx::query_scalar("SELECT id FROM containers WHERE container_type = 'market_escrow'")
        .fetch_one(&mut *conn)
        .await?;
    let escrowed = units::insert_stack(
        conn,
        NewStack {
            item_type: &item_type,
            owner_id: None,
            container_id: Some(escrow),
            slot_index: None,
            durability,
        },
        &taken.units,
    )
    .await?;
    Ok(escrowed)
}

/// What a cancel gave back.
#[derive(Debug, Clone, PartialEq)]
pub struct Cancelled {
    /// The order as it stands now, `cancelled`
    pub order: MarketOrder,
    /// A sell's units, now in `market_box`
    pub returned: Units,
    /// A buy's Strands, now back on the balance
    pub refunded: i64,
    /// The seller's market box
    pub market_box: Option<Uuid>,
}

/// Cancel `player_id`'s open order `order_id`: what's left of its escrow goes
/// back, units to their market box and Strands to their balance. The fee isn't
/// refunded.
pub async fn cancel(db: &PgPool, player_id: Uuid, order_id: Uuid) -> Result<Cancelled, MarketError> {
    let mut tx = db.begin().await?;
    let owner: Option<(Uuid, String)> =
        sqlx::query_as("SELECT player_id, status FROM market_orders WHERE id = $1 FOR UPDATE")
            .bind(order_id)
            .fetch_optional(&mut *tx)
            .await?;
    match owner {
        Some((owner, _)) if owner != player_id => return Err(MarketError::NotFound),
        None => return Err(MarketError::NotFound),
        Some((_, status)) if status != OrderStatus::Open.as_str() => return Err(MarketError::NotOpen),
        Some(_) => {}
    }
    let cancelled = close(&mut tx, order_id, OrderStatus::Cancelled).await?;
    tx.commit().await?;
    Ok(cancelled)
}

/// Take the open order `order_id` (locked by the caller) off the book as
/// `status`, giving back what's left of its escrow. Expiry (E3.2) is this with
/// `Expired`.
pub(super) async fn close(conn: &mut PgConnection, order_id: Uuid, status: OrderStatus) -> Result<Cancelled, MarketError> {
    let (player_id, side, escrow_item_id, escrow_strands): (Uuid, String, Option<Uuid>, i64) = sqlx::query_as(
        "SELECT player_id, side, escrow_item_id, escrow_strands FROM market_orders WHERE id = $1",
    )
    .bind(order_id)
    .fetch_one(&mut *conn)
    .await?;

    let (mut returned, mut refunded, mut market_box_id) = (Units::new(), 0, None);
    if let Some(stack) = escrow_item_id.filter(|_| side == OrderSide::Sell.as_str()) {
        let into = market_box(conn, player_id).await?;
        returned = return_to_box(conn, player_id, stack, into).await?;
        market_box_id = Some(into);
    }
    if escrow_strands > 0 {
        add_strands_in(
            conn,
            player_id,
            escrow_strands,
            TransactionType::MarketRefund,
            Some(&format!("Order {} {}", status.as_str(), order_id)),
            Some(order_id),
        )
        .await
        .map_err(MarketError::Funds)?;
        refunded = escrow_strands;
    }

    sqlx::query(
        "UPDATE market_orders SET status = $2, escrow_item_id = NULL, escrow_strands = 0 WHERE id = $1",
    )
    .bind(order_id)
    .bind(status.as_str())
    .execute(&mut *conn)
    .await?;

    let order = load(conn, order_id).await?.ok_or(MarketError::NotFound)?;
    Ok(Cancelled {
        order,
        returned,
        refunded,
        market_box: market_box_id,
    })
}

/// Move the escrow stack `stack` into `player_id`'s market box `into`: onto
/// the first stack of its type there, if it stacks (D2: nothing is diluted),
/// or into the first free slot. Returns its units.
pub(super) async fn return_to_box(conn: &mut PgConnection, player_id: Uuid, stack: Uuid, into: Uuid) -> Result<Units, MarketError> {
    let units = units::units_of(conn, stack).await?;
    let (item_type, stackable): (String, bool) = sqlx::query_as(
        "SELECT i.item_type, t.stackable FROM items i JOIN item_types t ON t.id = i.item_type WHERE i.id = $1",
    )
    .bind(stack)
    .fetch_one(&mut *conn)
    .await?;
    if stackable {
        if let Some(target) = units::first_stack(conn, into, &item_type).await? {
            let mut held = units::units_of(conn, target).await?;
            if held.merge(&units) {
                units::merge_into(conn, stack, target).await?;
                return Ok(units);
            }
        }
    }
    let (slot_count,): (i32,) = sqlx::query_as("SELECT slot_count FROM container_types WHERE id = 'market_box'")
        .fetch_one(&mut *conn)
        .await?;
    let used: Vec<i32> =
        sqlx::query_scalar("SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL")
            .bind(into)
            .fetch_all(&mut *conn)
            .await?;
    let slot = (0..slot_count).find(|s| !used.contains(s)).ok_or(MarketError::MarketBoxFull)?;
    sqlx::query("UPDATE items SET container_id = $2, slot_index = $3, owner_id = $4 WHERE id = $1")
        .bind(stack)
        .bind(into)
        .bind(slot)
        .bind(player_id)
        .execute(&mut *conn)
        .await?;
    Ok(units)
}

/// `player_id`'s market box, made the first time it's needed, and locked for
/// the caller's transaction.
pub async fn market_box(conn: &mut PgConnection, player_id: Uuid) -> Result<Uuid, sqlx::Error> {
    sqlx::query(
        "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'market_box', $2)
         ON CONFLICT (owner_id) WHERE container_type = 'market_box' DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(player_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query_scalar("SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'market_box' FOR UPDATE")
        .bind(player_id)
        .fetch_one(&mut *conn)
        .await
}

type OrderRow = (
    Uuid,
    Uuid,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i32,
    i32,
    String,
    Option<Uuid>,
    i64,
    i64,
    DateTime<Utc>,
    DateTime<Utc>,
);

/// The order `order_id`, with the units still in its escrow.
pub async fn load(conn: &mut PgConnection, order_id: Uuid) -> Result<Option<MarketOrder>, sqlx::Error> {
    let row: Option<OrderRow> = sqlx::query_as(
        "SELECT id, player_id, side, item_type, min_grade, grade_rule, price, quantity_total,
                quantity_remaining, status, escrow_item_id, escrow_strands, fee, created_at, expires_at
         FROM market_orders WHERE id = $1",
    )
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((
        id,
        player_id,
        side,
        item_type,
        min_grade,
        grade_rule,
        price,
        quantity_total,
        quantity_remaining,
        status,
        escrow_item_id,
        escrow_strands,
        fee,
        created_at,
        expires_at,
    )) = row
    else {
        return Ok(None);
    };
    let units = match escrow_item_id {
        Some(stack) => units::units_of(conn, stack).await?,
        None => Units::new(),
    };
    let bad = |what: &str, value: &str| sqlx::Error::Decode(format!("market_orders.{what} is {value:?}").into());
    Ok(Some(MarketOrder {
        id,
        player_id,
        side: OrderSide::parse(&side).ok_or_else(|| bad("side", &side))?,
        item_type,
        price,
        quantity_total,
        quantity_remaining,
        status: OrderStatus::parse(&status).ok_or_else(|| bad("status", &status))?,
        min_grade,
        grade_rule: grade_rule.as_deref().map(|r| GradeRule::parse(r).ok_or_else(|| bad("grade_rule", r))).transpose()?,
        units,
        escrow_strands,
        fee,
        created_at,
        expires_at,
    }))
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test market -- --ignored`.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::market_matcher::{run_cycle, Market};
    use crate::engine::units::test_units::{all_in, stack_of, units};
    use rand::rngs::StdRng;
    use tokio::sync::broadcast;
    use rand::{Rng, SeedableRng};
    use std::collections::BTreeMap;

    pub(crate) struct Player {
        pub(crate) id: Uuid,
        pub(crate) inventory: Uuid,
    }

    pub(crate) async fn player(db: &PgPool, name: &str, strands: i64) -> Player {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(account)
            .bind(format!("{name}@test"))
            .execute(db)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name, strand_balance) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(account)
            .bind(name)
            .bind(strands)
            .execute(db)
            .await
            .unwrap();
        let inventory = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(inventory)
            .bind(id)
            .execute(db)
            .await
            .unwrap();
        Player { id, inventory }
    }

    pub(crate) async fn balance(db: &PgPool, player: Uuid) -> i64 {
        sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
            .bind(player)
            .fetch_one(db)
            .await
            .unwrap()
    }

    pub(crate) async fn box_of(db: &PgPool, player: Uuid) -> Uuid {
        let mut conn = db.acquire().await.unwrap();
        market_box(&mut conn, player).await.unwrap()
    }

    pub(crate) fn sell(item_type: &str, item_id: Uuid, price: i64, quantity: i32, qualities: Option<Units>) -> PlaceOrder {
        PlaceOrder {
            side: OrderSide::Sell,
            item_type: item_type.into(),
            price,
            quantity,
            item_id: Some(item_id),
            qualities,
            min_grade: None,
            grade_rule: None,
        }
    }

    pub(crate) fn buy(item_type: &str, price: i64, quantity: i32, min_grade: Option<&str>) -> PlaceOrder {
        PlaceOrder {
            side: OrderSide::Buy,
            item_type: item_type.into(),
            price,
            quantity,
            item_id: None,
            qualities: None,
            min_grade: min_grade.map(Into::into),
            grade_rule: None,
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_sell_escrows_the_named_units_and_a_cancel_sends_them_to_the_market_box(db: PgPool) {
        let ada = player(&db, "ada", 1000).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::from_pairs([(50, 2), (81, 6), (92, 4)])).await;

        let named = Units::from_pairs([(81, 6), (92, 4)]);
        let order = place(&db, ada.id, &sell("rope", stack, 11, 10, Some(named.clone()))).await.unwrap();
        assert_eq!((order.status, order.quantity_remaining, order.fee), (OrderStatus::Open, 10, 2));
        assert_eq!(order.units, named);
        assert_eq!(order.expires_at - order.created_at, Duration::days(7));
        assert_eq!(balance(&db, ada.id).await, 998);
        assert_eq!(units(&db, stack).await, Units::uniform(2, 50));

        let done = cancel(&db, ada.id, order.id).await.unwrap();
        assert_eq!(done.order.status, OrderStatus::Cancelled);
        assert!(done.order.units.is_empty());
        assert_eq!((done.returned.clone(), done.refunded), (named.clone(), 0));
        let market_box = box_of(&db, ada.id).await;
        assert_eq!(done.market_box, Some(market_box));
        assert_eq!(all_in(&db, market_box, "rope").await, named);
        // The fee stays spent
        assert_eq!(balance(&db, ada.id).await, 998);
        // A second cancel finds nothing open
        assert!(matches!(cancel(&db, ada.id, order.id).await, Err(MarketError::NotOpen)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_sell_without_named_units_takes_the_lowest_and_joins_the_box_on_return(db: PgPool) {
        let ada = player(&db, "ada", 100).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::from_pairs([(40, 3), (95, 3)])).await;
        let market_box = box_of(&db, ada.id).await;
        let waiting = stack_of(&db, ada.id, market_box, 5, "rope", &Units::uniform(1, 70)).await;

        let order = place(&db, ada.id, &sell("rope", stack, 3, 4, None)).await.unwrap();
        assert_eq!(order.units, Units::from_pairs([(40, 3), (95, 1)]));
        cancel(&db, ada.id, order.id).await.unwrap();
        // Back onto the stack already waiting there, every unit at its quality
        assert_eq!(units(&db, waiting).await, Units::from_pairs([(40, 3), (70, 1), (95, 1)]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_whole_unstackable_stack_comes_back_to_a_slot_of_its_own(db: PgPool) {
        let ada = player(&db, "ada", 100).await;
        let kit = stack_of(&db, ada.id, ada.inventory, 0, "workbench_kit", &Units::uniform(1, 66)).await;
        let market_box = box_of(&db, ada.id).await;
        let other = stack_of(&db, ada.id, market_box, 0, "workbench_kit", &Units::uniform(1, 30)).await;

        let order = place(&db, ada.id, &sell("workbench_kit", kit, 50, 1, None)).await.unwrap();
        let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE id = $1")
            .bind(kit)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(gone, 0, "selling every unit uses the stack up");
        cancel(&db, ada.id, order.id).await.unwrap();
        let slots: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT slot_index, quality FROM items WHERE container_id = $1 ORDER BY slot_index",
        )
        .bind(market_box)
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(slots, vec![(0, 30), (1, 66)]);
        assert_eq!(units(&db, other).await, Units::uniform(1, 30));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_buy_escrows_its_value_and_a_cancel_refunds_all_but_the_fee(db: PgPool) {
        let bea = player(&db, "bea", 1000).await;
        let order = place(&db, bea.id, &buy("rope", 12, 20, Some("B"))).await.unwrap();
        assert_eq!((order.escrow_strands, order.fee), (240, 3));
        assert_eq!((order.min_grade.as_deref(), order.grade_rule), (Some("B"), Some(GradeRule::Average)));
        assert_eq!(balance(&db, bea.id).await, 1000 - 243);

        let done = cancel(&db, bea.id, order.id).await.unwrap();
        assert_eq!((done.refunded, done.order.escrow_strands, done.market_box), (240, 0, None));
        assert_eq!(balance(&db, bea.id).await, 997);

        let audit: Vec<(String, i64)> = sqlx::query_as(
            "SELECT transaction_type, amount FROM currency_transactions WHERE player_id = $1",
        )
        .bind(bea.id)
        .fetch_all(&db)
        .await
        .unwrap();
        let mut audit: Vec<_> = audit.iter().map(|(t, a)| (t.as_str(), *a)).collect();
        audit.sort();
        assert_eq!(audit, vec![("market_escrow", -240), ("market_fee", -3), ("market_refund", 240)]);
        // A buy with no grade takes any
        let any = place(&db, bea.id, &buy("rope", 1, 1, None)).await.unwrap();
        assert_eq!(any.min_grade.as_deref(), Some("F"));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn refusals_change_nothing(db: PgPool) {
        let ada = player(&db, "ada", 1).await;
        let bea = player(&db, "bea", 50).await;
        let held = Units::from_pairs([(60, 5), (88, 5)]);
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &held).await;

        let refused = [
            // The fee is 1 on 100 and Ada has 1, so this one's fine; 101 needs 2
            place(&db, ada.id, &sell("rope", stack, 101, 1, None)).await,
            place(&db, bea.id, &sell("rope", stack, 1, 1, None)).await,
            place(&db, ada.id, &sell("fiber", stack, 1, 1, None)).await,
            place(&db, ada.id, &sell("rope", stack, 1, 11, None)).await,
            place(&db, ada.id, &sell("rope", stack, 1, 2, Some(Units::uniform(2, 70)))).await,
            place(&db, ada.id, &sell("rope", stack, 1, 3, Some(Units::uniform(2, 60)))).await,
            place(&db, ada.id, &sell("nonsense", stack, 1, 1, None)).await,
            place(&db, ada.id, &sell("rope", stack, 0, 1, None)).await,
            place(&db, bea.id, &buy("rope", 10, 5, None)).await,
            place(&db, bea.id, &buy("rope", 1, 1, Some("E"))).await,
            place(&db, bea.id, &PlaceOrder { item_id: None, ..sell("rope", stack, 1, 1, None) }).await,
        ];
        let kinds: Vec<String> = refused.iter().map(|r| format!("{:?}", r.as_ref().unwrap_err())).collect();
        let kinds: Vec<&str> = kinds.iter().map(|k| k.split('(').next().unwrap()).collect();
        assert_eq!(
            kinds,
            [
                "Funds", "NotYourStack", "WrongItemType", "NotHeld", "NotHeld", "BadQualities",
                "UnknownItemType", "Order", "Funds", "Order", "NoStack"
            ]
        );
        assert_eq!(units(&db, stack).await, held);
        assert_eq!((balance(&db, ada.id).await, balance(&db, bea.id).await), (1, 50));
        let orders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM market_orders").fetch_one(&db).await.unwrap();
        assert_eq!(orders, 0);

        // Someone else's order is no order of yours
        let order = place(&db, ada.id, &sell("rope", stack, 100, 1, None)).await.unwrap();
        assert!(matches!(cancel(&db, bea.id, order.id).await, Err(MarketError::NotFound)));
        assert!(matches!(cancel(&db, bea.id, Uuid::new_v4()).await, Err(MarketError::NotFound)));
        // Escrowed units can't be sold again
        let escrowed: Uuid = sqlx::query_scalar("SELECT escrow_item_id FROM market_orders WHERE id = $1")
            .bind(order.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert!(matches!(
            place(&db, ada.id, &sell("rope", escrowed, 1, 1, None)).await,
            Err(MarketError::NotYourStack)
        ));
    }

    /// For any run of places, cancels and fills (E3.2), Strands held + in
    /// escrow + fees, and
    /// every unit of every type, are what there were at the start. Each open
    /// sell's escrow holds exactly its remaining units, and every balance is
    /// its start plus its audit entries.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn places_cancels_and_fills_conserve_strands_and_units(db: PgPool) {
        // Each run adds its own players to the same world, so the totals
        // checked are everything's, across the runs
        let mut fills = 0;
        for seed in 0..6 {
            fills += conservation_run(&db, seed, 120).await;
        }
        assert!(fills > 30, "only {fills} fills in six runs");
    }

    /// Strands held, in open escrow and paid in fees: the total there is
    async fn strands_accounted(db: &PgPool) -> i64 {
        let (held, escrowed, fees): (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COALESCE(SUM(strand_balance), 0)::BIGINT FROM players),
                    (SELECT COALESCE(SUM(escrow_strands), 0)::BIGINT FROM market_orders WHERE status = 'open'),
                    (SELECT COALESCE(SUM(fee), 0)::BIGINT FROM market_orders)",
        )
        .fetch_one(db)
        .await
        .unwrap();
        held + escrowed + fees
    }

    async fn conservation_run(pool: &PgPool, seed: u64, steps: usize) -> usize {
        const TYPES: [&str; 2] = ["rope", "fiber"];
        let db = pool.clone();
        let mut rng = StdRng::seed_from_u64(seed);
        let mut players = Vec::new();
        for name in ["ada", "bea", "cy"] {
            let p = player(&db, &format!("{name}{seed}"), rng.gen_range(0..2000)).await;
            for (slot, item_type) in TYPES.iter().enumerate() {
                let mut held = Units::new();
                for _ in 0..rng.gen_range(1..5) {
                    held.add(rng.gen_range(0..=100), rng.gen_range(1..8));
                }
                stack_of(&db, p.id, p.inventory, slot as i32, item_type, &held).await;
            }
            players.push(p);
        }
        let start_strands: BTreeMap<Uuid, i64> = {
            let mut m = BTreeMap::new();
            for p in &players {
                m.insert(p.id, balance(&db, p.id).await);
            }
            m
        };
        let total_strands = strands_accounted(&db).await;
        let mut start_units = BTreeMap::new();
        for item_type in TYPES {
            start_units.insert(item_type, every_unit(&db, item_type).await);
        }

        let (mut placed, mut refused, mut fills, mut untold) = (Vec::new(), 0, 0, Vec::new());
        let market = Market::default();
        let (events, _heard) = broadcast::channel(4096);
        for step in 0..steps {
            let p = &players[rng.gen_range(0..players.len())];
            let item_type = TYPES[rng.gen_range(0..TYPES.len())];
            let price = rng.gen_range(1..60);
            let outcome = match rng.gen_range(0..10) {
                0..=3 => {
                    let stacks: Vec<(Uuid, i32)> = sqlx::query_as(
                        "SELECT i.id, i.quantity FROM items i JOIN containers c ON c.id = i.container_id
                         WHERE i.owner_id = $1 AND i.item_type = $2 ORDER BY i.id",
                    )
                    .bind(p.id)
                    .bind(item_type)
                    .fetch_all(&db)
                    .await
                    .unwrap();
                    let Some(&(stack, quantity)) = stacks.get(rng.gen_range(0..stacks.len().max(1))) else {
                        continue;
                    };
                    // Now and then more than it holds
                    let n = rng.gen_range(1..=quantity + 1);
                    let named = if rng.gen_bool(0.5) {
                        let mut conn = db.acquire().await.unwrap();
                        units::units_of(&mut conn, stack).await.unwrap().pick(n, Pick::Best)
                    } else {
                        None
                    };
                    place(&db, p.id, &sell(item_type, stack, price, n, named)).await.map(|o| {
                        market.placed(&o);
                        placed.push((o.id, o.player_id))
                    })
                }
                4..=6 => {
                    let grade = ["A", "B", "C", "D", "F"][rng.gen_range(0..5)];
                    place(&db, p.id, &buy(item_type, price, rng.gen_range(1..12), Some(grade)))
                        .await
                        .map(|o| {
                            market.placed(&o);
                            placed.push((o.id, o.player_id))
                        })
                }
                _ if placed.is_empty() => continue,
                _ => {
                    // Mostly their own, open or not; now and then anyone's
                    let own: Vec<Uuid> = placed.iter().filter(|(_, by)| *by == p.id).map(|(o, _)| *o).collect();
                    let order = if !own.is_empty() && rng.gen_bool(0.75) {
                        own[rng.gen_range(0..own.len())]
                    } else {
                        placed[rng.gen_range(0..placed.len())].0
                    };
                    // Now and then the book isn't told, and a fill finds it gone
                    let told = rng.gen_bool(0.75);
                    cancel(&db, p.id, order).await.map(|_| {
                        if told {
                            market.cancelled(order)
                        } else {
                            untold.push(order);
                        }
                    })
                }
            };
            if outcome.is_err() {
                refused += 1;
            }
            // The matcher runs every few steps, so orders cross in batches
            if rng.gen_bool(0.3) {
                fills += run_cycle(&market, &db, &events, Utc::now()).await.fills;
            }

            assert_eq!(strands_accounted(&db).await, total_strands, "Strands after step {step} of run {seed}");
            for item_type in TYPES {
                assert_eq!(units_of_type(&db, item_type).await, start_units[item_type], "{item_type} after step {step} of run {seed}");
            }
        }
        assert!(
            placed.len() > 30 && refused > 5,
            "run {seed}: {} placed, {refused} refused; it should do plenty of both",
            placed.len()
        );

        // Every stack's summaries still match its units
        for item_type in TYPES {
            assert_eq!(every_unit(&db, item_type).await, start_units[item_type]);
        }
        let mut conn = db.acquire().await.unwrap();
        for (id, _) in &placed {
            let order = load(&mut conn, *id).await.unwrap().unwrap();
            match (order.status, order.side) {
                (OrderStatus::Open, OrderSide::Sell) => assert_eq!(order.units.count(), order.quantity_remaining as i64),
                (OrderStatus::Open, OrderSide::Buy) => assert_eq!(
                    order.escrow_strands,
                    market::order_value(order.price, order.quantity_remaining)
                ),
                _ => assert!(order.units.is_empty() && order.escrow_strands == 0),
            }
        }
        for (player, start) in start_strands {
            let audited: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount), 0)::BIGINT FROM currency_transactions WHERE player_id = $1",
            )
            .bind(player)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
            assert_eq!(balance(&db, player).await, start + audited);
        }

        // The book is the DB's open orders, each with what's left of it
        fills += run_cycle(&market, &db, &events, Utc::now()).await.fills;
        for (id, _) in &placed {
            let order = load(&mut conn, *id).await.unwrap().unwrap();
            let booked = market.with_book(|b| b.get(*id).map(|o| (o.remaining, o.units.clone())));
            match order.status {
                OrderStatus::Open => assert_eq!(booked, Some((order.quantity_remaining, order.units))),
                // A cancel the book wasn't told of waits there for a fill to find it
                _ if untold.contains(id) => {}
                _ => assert_eq!(booked, None, "{:?} order {id} is still on the book", order.status),
            }
        }
        fills
    }

    /// Every unit of `item_type` there is, held or in escrow, in one query
    async fn units_of_type(db: &PgPool, item_type: &str) -> Units {
        let rows: Vec<(i16, i64)> = sqlx::query_as(
            "SELECT u.quality, SUM(u.count)::BIGINT FROM item_units u JOIN items i ON i.id = u.item_id
             WHERE i.item_type = $1 GROUP BY u.quality",
        )
        .bind(item_type)
        .fetch_all(db)
        .await
        .unwrap();
        Units::from_pairs(rows.into_iter().map(|(q, c)| (q as i32, c as i32)))
    }

    /// [`units_of_type`] stack by stack, checking each one's summaries match its units
    async fn every_unit(db: &PgPool, item_type: &str) -> Units {
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM items WHERE item_type = $1")
            .bind(item_type)
            .fetch_all(db)
            .await
            .unwrap();
        let mut all = Units::new();
        for id in ids {
            all.merge(&units(db, id).await);
        }
        all
    }
}
