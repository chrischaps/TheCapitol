//! Commissions (commons plan D5, S2.1): standing requests on the hall's board
//! with the money already down. The routes that call these are
//! `routes::commissions`.
//!
//! Each operation is one transaction, and every Strand it moves goes through
//! the currency audit log:
//! - [`post`]: `price × quantity` goes into the commission's escrow
//!   (`court_escrow`), and the court's fee is charged on top (`court_fee`, a
//!   sink, `rules::court::court_fee`).
//! - [`deliver`]: units at or over the commission's quality leave the
//!   deliverer's stack for the poster's market box, each keeping its quality
//!   and maker's mark, and the deliverer is paid for them out of the escrow
//!   (`court_payout`). If the box has no room they wait in the court's escrow
//!   box ([`move_waiting`] moves them on as it has room), so a full box never
//!   turns a deliverer away.
//! - [`cancel`] and the expiry in [`expire_due`]: what's unspent comes back to
//!   the poster (`court_refund`). The fee stays spent.
//!
//! So for any run of these, the Strands held plus those in escrow plus the
//! fees are what there were at the start, and every unit is somewhere (the
//! conservation test). The bounds are `rules::court`, shared with the clients.

use capitol_protocol::rest::{Commission, CommissionDelivery, CommissionsQuery, DeliverCommission, PostCommission};
use capitol_protocol::rules::court::{
    self, commission_value, court_fee, validate_commission, CommissionStatus, DeliveryError, MAX_OPEN_COMMISSIONS,
};
use capitol_protocol::ws::ServerMessage;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::bazaar::Notice;
use super::currency::{add_strands_in, subtract_strands_in, TransactionType};
use super::market::{market_box, stow};
use super::notices;
use super::units::{self, NewStack};

/// The most commissions `GET /commissions` answers with: the newest.
pub const MAX_LISTED: i64 = 200;

/// Why the court refused.
#[derive(Debug)]
pub enum CourtError {
    /// The price, quantity, quality or due date is out of bounds
    Rules(court::CommissionError),
    /// The units named, or held, can't be delivered
    Delivery(DeliveryError),
    /// No such item type
    UnknownItemType,
    /// [`MAX_OPEN_COMMISSIONS`] are open already
    TooMany,
    /// Not enough Strands for the escrow and the fee
    Funds(String),
    /// No such commission (of yours, for a cancel)
    NotFound,
    /// It's already filled, cancelled or expired, or past due
    NotOpen,
    /// A poster delivering to their own commission
    OwnCommission,
    /// The stack isn't the deliverer's, in their inventory or market box
    NotYourStack,
    /// The stack isn't of the commission's item type
    WrongItemType,
    Db(sqlx::Error),
}

impl CourtError {
    pub fn message(&self) -> String {
        match self {
            CourtError::Rules(e) => e.message().to_string(),
            CourtError::Delivery(e) => e.message().to_string(),
            CourtError::UnknownItemType => "There's no such item".to_string(),
            CourtError::TooMany => format!("You have {MAX_OPEN_COMMISSIONS} commissions open already"),
            CourtError::Funds(m) => m.clone(),
            CourtError::NotFound => "No such commission".to_string(),
            CourtError::NotOpen => "That commission is no longer open".to_string(),
            CourtError::OwnCommission => "You can't deliver to your own commission".to_string(),
            CourtError::NotYourStack => "That stack isn't in your inventory or market box".to_string(),
            CourtError::WrongItemType => "That stack isn't the item the commission asks for".to_string(),
            CourtError::Db(e) => format!("Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for CourtError {
    fn from(e: sqlx::Error) -> Self {
        CourtError::Db(e)
    }
}

impl From<DeliveryError> for CourtError {
    fn from(e: DeliveryError) -> Self {
        CourtError::Delivery(e)
    }
}

/// Post `req` for `poster_id` at `now`: take its escrow and the court's fee,
/// and put it on the board. Returns it as posted. Nothing changes unless all
/// of it does.
pub async fn post(db: &PgPool, poster_id: Uuid, req: &PostCommission, now: DateTime<Utc>) -> Result<Commission, CourtError> {
    validate_commission(req.price, req.quantity, req.min_quality, req.due_days).map_err(CourtError::Rules)?;
    let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM item_types WHERE id = $1)")
        .bind(&req.item_type)
        .fetch_one(db)
        .await?;
    if !known {
        return Err(CourtError::UnknownItemType);
    }

    let id = Uuid::new_v4();
    let value = commission_value(req.price, req.quantity);
    let fee = court_fee(value);
    let due_at = now + Duration::seconds(req.due_days * court::DAY_SECS);

    let mut tx = db.begin().await?;
    // The poster's row, held so two posts at once can't both be the fifth
    sqlx::query("SELECT 1 FROM players WHERE id = $1 FOR UPDATE")
        .bind(poster_id)
        .fetch_one(&mut *tx)
        .await?;
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM commissions WHERE poster_id = $1 AND status = 'open'")
        .bind(poster_id)
        .fetch_one(&mut *tx)
        .await?;
    if open >= MAX_OPEN_COMMISSIONS {
        return Err(CourtError::TooMany);
    }
    let what = format!("{} {} at {} each", req.quantity, req.item_type, req.price);
    subtract_strands_in(
        &mut tx,
        poster_id,
        value,
        TransactionType::CourtEscrow,
        Some(&format!("Commission: {what}")),
        Some(id),
    )
    .await
    .map_err(CourtError::Funds)?;
    subtract_strands_in(&mut tx, poster_id, fee, TransactionType::CourtFee, Some(&format!("Court fee: {what}")), Some(id))
        .await
        .map_err(CourtError::Funds)?;

    sqlx::query(
        "INSERT INTO commissions
             (id, poster_id, item_type, min_quality, quantity, price_per_unit, escrow_strands, fee, posted_at, due_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(poster_id)
    .bind(&req.item_type)
    .bind(req.min_quality as i16)
    .bind(req.quantity)
    .bind(req.price)
    .bind(value)
    .bind(fee)
    .bind(now)
    .bind(due_at)
    .execute(&mut *tx)
    .await?;

    let posted = load(&mut tx, id).await?.ok_or(CourtError::NotFound)?;
    tx.commit().await?;
    Ok(posted)
}

/// What a delivery did.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivered {
    /// The units, the pay and the commission after them, as the deliverer hears it
    pub delivery: CommissionDelivery,
    /// The container the units came from
    pub source: Uuid,
    /// The poster's market box, if the units went into it; `None` if they wait
    /// in the court for room there
    pub market_box: Option<Uuid>,
    /// The poster's `commission_delivered`
    pub notice: Notice,
}

/// Deliver units from `deliverer_id`'s stack to the open commission
/// `commission_id` at `now`, as `req` names them: they go to the poster, and
/// the deliverer is paid for them out of the escrow. Nothing changes unless
/// all of it does.
pub async fn deliver(
    db: &PgPool,
    deliverer_id: Uuid,
    commission_id: Uuid,
    req: &DeliverCommission,
    now: DateTime<Utc>,
) -> Result<Delivered, CourtError> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, String, i16, i32, i32, i64, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT poster_id, item_type, min_quality, quantity, delivered, price_per_unit, status, due_at
         FROM commissions WHERE id = $1 FOR UPDATE",
    )
    .bind(commission_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((poster_id, item_type, min_quality, quantity, delivered, price, status, due_at)) = row else {
        return Err(CourtError::NotFound);
    };
    if status != CommissionStatus::Open.as_str() || due_at <= now {
        return Err(CourtError::NotOpen);
    }
    if poster_id == deliverer_id {
        return Err(CourtError::OwnCommission);
    }

    let stack: Option<(String, Option<Uuid>, Option<Uuid>, Option<String>, Option<i32>)> = sqlx::query_as(
        "SELECT i.item_type, i.owner_id, i.container_id, c.container_type, i.durability
         FROM items i LEFT JOIN containers c ON c.id = i.container_id
         WHERE i.id = $1 FOR UPDATE OF i",
    )
    .bind(req.item_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((stack_type, owner, Some(source), container_type, durability)) = stack else {
        return Err(CourtError::NotYourStack);
    };
    let deliverable_from = matches!(container_type.as_deref(), Some("player_inventory" | "market_box"));
    if owner != Some(deliverer_id) || !deliverable_from {
        return Err(CourtError::NotYourStack);
    }
    if stack_type != item_type {
        return Err(CourtError::WrongItemType);
    }

    let held = units::units_of(&mut tx, req.item_id).await?;
    let wanted = court::delivery(&held, min_quality as i32, quantity - delivered, req.qualities.as_ref(), req.quantity)?;
    let taken = units::take_units(&mut tx, req.item_id, &wanted)
        .await?
        .ok_or(CourtError::Delivery(DeliveryError::NotHeld))?;
    let n = taken.units.count() as i32;
    let paid = commission_value(price, n);

    let escrow: Uuid = sqlx::query_scalar("SELECT id FROM containers WHERE container_type = 'court_escrow'")
        .fetch_one(&mut *tx)
        .await?;
    let moved = units::insert_stack(
        &mut tx,
        NewStack {
            item_type: &item_type,
            owner_id: None,
            container_id: Some(escrow),
            slot_index: None,
            durability,
        },
        &taken.marked,
    )
    .await?;
    let into = market_box(&mut tx, poster_id).await?;
    let market_box = match stow(&mut tx, poster_id, moved, into).await? {
        Some(_) => Some(into),
        None => {
            sqlx::query("INSERT INTO commission_waiting (item_id, commission_id, poster_id, since) VALUES ($1, $2, $3, $4)")
                .bind(moved)
                .bind(commission_id)
                .bind(poster_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            None
        }
    };

    add_strands_in(
        &mut tx,
        deliverer_id,
        paid,
        TransactionType::CourtPayout,
        Some(&format!("Delivered {n} {item_type} at {price} each")),
        Some(commission_id),
    )
    .await
    .map_err(CourtError::Funds)?;
    let filled = delivered + n == quantity;
    sqlx::query(
        "UPDATE commissions SET delivered = delivered + $2, escrow_strands = escrow_strands - $3, status = $4
         WHERE id = $1",
    )
    .bind(commission_id)
    .bind(n)
    .bind(paid)
    .bind(if filled { CommissionStatus::Filled } else { CommissionStatus::Open }.as_str())
    .execute(&mut *tx)
    .await?;

    let (deliverer_name, deliverer_citizen) = name_of(&mut tx, deliverer_id).await?;
    let message = ServerMessage::CommissionDelivered {
        commission_id,
        item_type,
        deliverer_id,
        deliverer_name,
        deliverer_citizen,
        units: taken.units.clone(),
        paid,
        left: quantity - delivered - n,
    };
    let notice_id = notices::queue(&mut *tx, poster_id, &commission_id.to_string(), &message).await?;
    let commission = load(&mut tx, commission_id).await?.ok_or(CourtError::NotFound)?;
    tx.commit().await?;
    Ok(Delivered {
        delivery: CommissionDelivery { commission, units: taken.units, paid },
        source,
        market_box,
        notice: Notice { player_id: poster_id, notice_id, message },
    })
}

/// A commission taken down: by its poster, or by its due date.
#[derive(Debug, Clone, PartialEq)]
pub struct Closed {
    /// As it stands now, `cancelled` or `expired`
    pub commission: Commission,
    /// The unspent escrow, now back on the poster's balance
    pub refunded: i64,
    /// An expiry's `commission_expired` to the poster
    pub notice: Option<Notice>,
}

/// Cancel `poster_id`'s open commission `commission_id`: the unspent escrow
/// comes back. The fee isn't refunded. Works from anywhere.
pub async fn cancel(db: &PgPool, poster_id: Uuid, commission_id: Uuid) -> Result<Closed, CourtError> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, String)> = sqlx::query_as("SELECT poster_id, status FROM commissions WHERE id = $1 FOR UPDATE")
        .bind(commission_id)
        .fetch_optional(&mut *tx)
        .await?;
    match row {
        Some((owner, _)) if owner != poster_id => return Err(CourtError::NotFound),
        None => return Err(CourtError::NotFound),
        Some((_, status)) if status != CommissionStatus::Open.as_str() => return Err(CourtError::NotOpen),
        Some(_) => {}
    }
    let closed = close(&mut tx, commission_id, CommissionStatus::Cancelled).await?;
    tx.commit().await?;
    Ok(closed)
}

/// Take the open commission `commission_id` (locked by the caller) down as
/// `status`, refunding what's unspent of its escrow. An expiry queues the
/// poster's notice.
async fn close(conn: &mut PgConnection, commission_id: Uuid, status: CommissionStatus) -> Result<Closed, CourtError> {
    let (poster_id, escrow_strands): (Uuid, i64) =
        sqlx::query_as("SELECT poster_id, escrow_strands FROM commissions WHERE id = $1")
            .bind(commission_id)
            .fetch_one(&mut *conn)
            .await?;
    if escrow_strands > 0 {
        add_strands_in(
            conn,
            poster_id,
            escrow_strands,
            TransactionType::CourtRefund,
            Some(&format!("Commission {} {commission_id}", status.as_str())),
            Some(commission_id),
        )
        .await
        .map_err(CourtError::Funds)?;
    }
    sqlx::query("UPDATE commissions SET status = $2, escrow_strands = 0 WHERE id = $1")
        .bind(commission_id)
        .bind(status.as_str())
        .execute(&mut *conn)
        .await?;
    let commission = load(conn, commission_id).await?.ok_or(CourtError::NotFound)?;
    let notice = match status {
        CommissionStatus::Expired => {
            let message = ServerMessage::CommissionExpired {
                commission_id,
                item_type: commission.item_type.clone(),
                delivered: commission.delivered,
                quantity: commission.quantity,
                refunded: escrow_strands,
            };
            let notice_id = notices::queue(&mut *conn, poster_id, &commission_id.to_string(), &message).await?;
            Some(Notice { player_id: poster_id, notice_id, message })
        }
        _ => None,
    };
    Ok(Closed { commission, refunded: escrow_strands, notice })
}

/// Expire every open commission due by `now`, each in a transaction of its
/// own. Called every market cycle (a second).
pub async fn expire_due(db: &PgPool, now: DateTime<Utc>) -> Result<Vec<Closed>, CourtError> {
    let due: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM commissions WHERE status = 'open' AND due_at <= $1 ORDER BY due_at, id")
            .bind(now)
            .fetch_all(db)
            .await?;
    let mut expired = Vec::with_capacity(due.len());
    for id in due {
        let mut tx = db.begin().await?;
        // A delivery may have filled it, or its poster cancelled it, meanwhile
        let still: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM commissions WHERE id = $1 AND status = 'open' AND due_at <= $2 FOR UPDATE SKIP LOCKED",
        )
        .bind(id)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        if still.is_some() {
            expired.push(close(&mut tx, id, CommissionStatus::Expired).await?);
        }
        tx.commit().await?;
    }
    Ok(expired)
}

/// Move delivered units waiting in the court into their posters' market
/// boxes, oldest first, as far as each has room. Returns the boxes that
/// changed, with their owners.
pub async fn move_waiting(db: &PgPool) -> Result<Vec<(Uuid, Uuid)>, sqlx::Error> {
    let posters: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT poster_id FROM commission_waiting")
        .fetch_all(db)
        .await?;
    let mut boxes = Vec::new();
    for poster_id in posters {
        let mut tx = db.begin().await?;
        let into = market_box(&mut tx, poster_id).await?;
        let waiting: Vec<Uuid> = sqlx::query_scalar(
            "SELECT item_id FROM commission_waiting WHERE poster_id = $1 ORDER BY since, item_id FOR UPDATE",
        )
        .bind(poster_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut moved = false;
        for stack in waiting {
            if stow(&mut tx, poster_id, stack, into).await?.is_none() {
                break;
            }
            sqlx::query("DELETE FROM commission_waiting WHERE item_id = $1").bind(stack).execute(&mut *tx).await?;
            moved = true;
        }
        tx.commit().await?;
        if moved {
            boxes.push((poster_id, into));
        }
    }
    Ok(boxes)
}

/// A player's name and whether they're a citizen.
async fn name_of(conn: &mut PgConnection, player_id: Uuid) -> Result<(String, bool), sqlx::Error> {
    sqlx::query_as("SELECT p.name, a.kind = 'citizen' FROM players p JOIN accounts a ON a.id = p.account_id WHERE p.id = $1")
        .bind(player_id)
        .fetch_one(conn)
        .await
}

/// A `commissions` row with its poster, as [`COLUMNS`] selects it
type Row = (Uuid, Uuid, String, bool, String, i16, i32, i32, i64, i64, i64, DateTime<Utc>, DateTime<Utc>, String);

/// The columns of a [`Row`], over `commissions c` joined to its poster
const COLUMNS: &str = "c.id, c.poster_id, p.name, a.kind = 'citizen', c.item_type, c.min_quality, c.quantity,
     c.delivered, c.price_per_unit, c.escrow_strands, c.fee, c.posted_at, c.due_at, c.status
     FROM commissions c JOIN players p ON p.id = c.poster_id JOIN accounts a ON a.id = p.account_id";

fn from_row(row: Row) -> Result<Commission, sqlx::Error> {
    let (
        id,
        poster_id,
        poster_name,
        poster_citizen,
        item_type,
        min_quality,
        quantity,
        delivered,
        price,
        escrow_strands,
        fee,
        posted_at,
        due_at,
        status,
    ) = row;
    Ok(Commission {
        id,
        poster_id,
        poster_name,
        poster_citizen,
        item_type,
        min_quality: min_quality as i32,
        quantity,
        delivered,
        price,
        escrow_strands,
        fee,
        posted_at,
        due_at,
        status: CommissionStatus::parse(&status)
            .ok_or_else(|| sqlx::Error::Decode(format!("commissions.status is {status:?}").into()))?,
    })
}

/// The commission `commission_id`.
pub async fn load(conn: &mut PgConnection, commission_id: Uuid) -> Result<Option<Commission>, sqlx::Error> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLUMNS} WHERE c.id = $1"))
        .bind(commission_id)
        .fetch_optional(conn)
        .await?;
    row.map(from_row).transpose()
}

/// The board as `query` asks for it, for `player_id`: newest first, at most
/// [`MAX_LISTED`].
pub async fn list(db: &PgPool, player_id: Uuid, query: &CommissionsQuery) -> Result<Vec<Commission>, sqlx::Error> {
    let status = query.status.unwrap_or(CommissionStatus::Open);
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS}
         WHERE c.status = $1 AND ($2::TEXT IS NULL OR c.item_type = $2) AND (NOT $3 OR c.poster_id = $4)
         ORDER BY c.posted_at DESC, c.id
         LIMIT $5"
    ))
    .bind(status.as_str())
    .bind(query.item.as_deref())
    .bind(query.mine)
    .bind(player_id)
    .bind(MAX_LISTED)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(from_row).collect()
}

/// Strands the open commissions hold: their unspent escrow, for the ledger's
/// `court_held`.
pub(crate) async fn held(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(SUM(escrow_strands), 0)::BIGINT FROM commissions")
        .fetch_one(conn)
        .await
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test commission -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::stacking::Units;
    use crate::engine::conservation::{assert_audited, balances_of, every_unit, strands_accounted, units_of_type};
    use crate::engine::market::tests::{balance, box_of, player};
    use crate::engine::units::test_units::{all_in, marked, marked_stack_of, stack_of, units};
    use crate::engine::units::Marked;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    use std::collections::BTreeMap;

    /// Units delivered to `poster_id`'s commissions still waiting in the
    /// court for room in their market box
    async fn waiting_for(db: &PgPool, poster_id: Uuid) -> Units {
        let rows: Vec<(i16, i64)> = sqlx::query_as(
            "SELECT u.quality, SUM(u.count)::BIGINT FROM commission_waiting w JOIN item_units u ON u.item_id = w.item_id
             WHERE w.poster_id = $1 GROUP BY u.quality",
        )
        .bind(poster_id)
        .fetch_all(db)
        .await
        .unwrap();
        Units::from_pairs(rows.into_iter().map(|(q, c)| (q as i32, c as i32)))
    }

    fn ask(item_type: &str, min_quality: i32, quantity: i32, price: i64, due_days: i64) -> PostCommission {
        PostCommission { item_type: item_type.into(), min_quality, quantity, price, due_days }
    }

    fn hand(item_id: Uuid, qualities: Option<Units>, quantity: Option<i32>) -> DeliverCommission {
        DeliverCommission { item_id, qualities, quantity }
    }

    async fn audit_of(db: &PgPool, player: Uuid) -> Vec<(String, i64)> {
        let mut rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT transaction_type, amount FROM currency_transactions WHERE player_id = $1")
                .bind(player)
                .fetch_all(db)
                .await
                .unwrap();
        rows.sort();
        rows
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_delivery_pays_from_escrow_and_the_units_keep_their_quality_and_maker(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let cy = player(&db, "cy", 0).await;
        // 20 rope at 55 or better, 6 each: 120 down and a fee of 5 + 1
        let posted = post(&db, ada.id, &ask("rope", 55, 20, 6, 4), now).await.unwrap();
        assert_eq!((posted.escrow_strands, posted.fee, posted.status), (120, 6, CommissionStatus::Open));
        assert_eq!(posted.due_at - posted.posted_at, Duration::days(4));
        assert_eq!(balance(&db, ada.id).await, 1000 - 126);
        assert_eq!(audit_of(&db, ada.id).await, vec![("court_escrow".into(), -120), ("court_fee".into(), -6)]);

        // Bo's rope, part his own make and part Cy's
        let ore = Marked::from_rows([(40, None, 4), (60, Some(bo.id), 5), (72, Some(cy.id), 3), (90, Some(bo.id), 2)]);
        let stack = marked_stack_of(&db, bo.id, bo.inventory, 0, "rope", &ore).await;

        // Without naming, the lowest that meet 55 go, up to 8
        let done = deliver(&db, bo.id, posted.id, &hand(stack, None, Some(8)), now).await.unwrap();
        assert_eq!(done.delivery.units, Units::from_pairs([(60, 5), (72, 3)]));
        assert_eq!((done.delivery.paid, done.delivery.commission.delivered), (48, 8));
        assert_eq!(done.delivery.commission.escrow_strands, 72);
        assert_eq!(done.source, bo.inventory);
        let the_box = box_of(&db, ada.id).await;
        assert_eq!(done.market_box, Some(the_box));
        // In Ada's box, each unit with its own quality and maker
        let in_box: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM items WHERE container_id = $1")
            .bind(the_box)
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(in_box.len(), 1);
        assert_eq!(marked(&db, in_box[0]).await, Marked::from_rows([(60, Some(bo.id), 5), (72, Some(cy.id), 3)]));
        assert_eq!(units(&db, stack).await, Units::from_pairs([(40, 4), (90, 2)]));
        assert_eq!(balance(&db, bo.id).await, 48);
        assert_eq!(audit_of(&db, bo.id).await, vec![("court_payout".into(), 48)]);
        // The poster's notice says who, how many and which
        match &done.notice.message {
            ServerMessage::CommissionDelivered { deliverer_name, units, paid, left, .. } => {
                assert_eq!((deliverer_name.as_str(), units.count(), *paid, *left), ("bo", 8, 48, 12));
            }
            other => panic!("the notice is {other:?}"),
        }
        assert_eq!(done.notice.player_id, ada.id);

        // Naming the 90s, and more than are wanted is capped
        let more = stack_of(&db, bo.id, bo.inventory, 1, "rope", &Units::uniform(20, 80)).await;
        let rest = deliver(&db, bo.id, posted.id, &hand(more, Some(Units::uniform(20, 80)), None), now).await.unwrap();
        assert_eq!(rest.delivery.units, Units::uniform(12, 80));
        assert_eq!(rest.delivery.commission.status, CommissionStatus::Filled);
        assert_eq!((rest.delivery.commission.escrow_strands, rest.delivery.paid), (0, 72));
        assert_eq!(all_in(&db, the_box, "rope").await, Units::from_pairs([(60, 5), (72, 3), (80, 12)]));
        assert_eq!(units(&db, more).await, Units::uniform(8, 80));
        // Filled, it takes no more and refunds nothing
        assert!(matches!(deliver(&db, bo.id, posted.id, &hand(more, None, None), now).await, Err(CourtError::NotOpen)));
        assert!(matches!(cancel(&db, ada.id, posted.id).await, Err(CourtError::NotOpen)));
        assert_eq!(balance(&db, ada.id).await, 1000 - 126);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn refusals_change_nothing(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let held = Units::from_pairs([(40, 5), (60, 5)]);
        let ore = stack_of(&db, bo.id, bo.inventory, 0, "rope", &held).await;
        let ada_ore = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::uniform(5, 90)).await;
        let logs = stack_of(&db, bo.id, bo.inventory, 1, "fiber", &Units::uniform(5, 90)).await;

        let refused = [
            post(&db, ada.id, &ask("rope", 55, 20, 0, 4), now).await.map(|_| ()),
            post(&db, ada.id, &ask("rope", 101, 20, 6, 4), now).await.map(|_| ()),
            post(&db, ada.id, &ask("rope", 55, 20, 6, 8), now).await.map(|_| ()),
            post(&db, ada.id, &ask("nonsense", 55, 20, 6, 4), now).await.map(|_| ()),
            // 995 + a fee of 10 is more than Ada has
            post(&db, ada.id, &ask("rope", 55, 199, 5, 4), now).await.map(|_| ()),
            post(&db, bo.id, &ask("rope", 55, 1, 1, 1), now).await.map(|_| ()),
        ];
        let kinds: Vec<String> = refused.iter().map(|r| format!("{:?}", r.as_ref().unwrap_err())).collect();
        let kinds: Vec<&str> = kinds.iter().map(|k| k.split('(').next().unwrap()).collect();
        assert_eq!(kinds, ["Rules", "Rules", "Rules", "UnknownItemType", "Funds", "Funds"]);
        assert_eq!(balance(&db, ada.id).await, 1000);

        // Five open is the most, for everyone
        let mut open = Vec::new();
        for _ in 0..MAX_OPEN_COMMISSIONS {
            open.push(post(&db, ada.id, &ask("rope", 55, 2, 3, 2), now).await.unwrap());
        }
        assert!(matches!(post(&db, ada.id, &ask("rope", 55, 2, 3, 2), now).await, Err(CourtError::TooMany)));
        let spent = 1000 - balance(&db, ada.id).await;
        // A cancelled one makes room again
        cancel(&db, ada.id, open[0].id).await.unwrap();
        post(&db, ada.id, &ask("rope", 55, 2, 3, 2), now).await.unwrap();
        let c = &open[1];
        let refused = [
            deliver(&db, ada.id, c.id, &hand(ada_ore, None, None), now).await.map(|_| ()),
            deliver(&db, bo.id, c.id, &hand(ada_ore, None, None), now).await.map(|_| ()),
            deliver(&db, bo.id, c.id, &hand(logs, None, None), now).await.map(|_| ()),
            // Below the quality, named or not
            deliver(&db, bo.id, c.id, &hand(ore, Some(Units::uniform(1, 40)), None), now).await.map(|_| ()),
            deliver(&db, bo.id, c.id, &hand(ore, Some(Units::uniform(6, 60)), None), now).await.map(|_| ()),
            deliver(&db, bo.id, open[0].id, &hand(ore, None, None), now).await.map(|_| ()),
            deliver(&db, bo.id, Uuid::new_v4(), &hand(ore, None, None), now).await.map(|_| ()),
            // Past due, though the sweep hasn't been by yet
            deliver(&db, bo.id, c.id, &hand(ore, None, None), c.due_at).await.map(|_| ()),
        ];
        let kinds: Vec<String> = refused.iter().map(|r| format!("{:?}", r.as_ref().unwrap_err())).collect();
        let kinds: Vec<&str> = kinds.iter().map(|k| k.split('(').next().unwrap()).collect();
        assert_eq!(
            kinds,
            ["OwnCommission", "NotYourStack", "WrongItemType", "Delivery", "Delivery", "NotOpen", "NotFound", "NotOpen"]
        );
        assert_eq!(units(&db, ore).await, held);
        assert_eq!(balance(&db, bo.id).await, 0);
        // Someone else's commission is no commission of yours to cancel
        assert!(matches!(cancel(&db, bo.id, c.id).await, Err(CourtError::NotFound)));
        // The cancel gave back its 6, and the post after it cost 6 and a fee of
        // 6 (the least filing, and 0.5% of 6 rounded up)
        assert_eq!(1000 - balance(&db, ada.id).await, spent - 6 + 12);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_cancel_and_an_expiry_refund_what_is_unspent_but_not_the_fee(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let ore = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::uniform(10, 70)).await;

        let first = post(&db, ada.id, &ask("rope", 0, 10, 10, 1), now).await.unwrap();
        deliver(&db, bo.id, first.id, &hand(ore, None, Some(3)), now).await.unwrap();
        let cancelled = cancel(&db, ada.id, first.id).await.unwrap();
        assert_eq!((cancelled.refunded, cancelled.notice.clone()), (70, None));
        assert_eq!(cancelled.commission.status, CommissionStatus::Cancelled);
        // 100 down, a fee of 6, 70 back
        assert_eq!(balance(&db, ada.id).await, 1000 - 100 - 6 + 70);

        let second = post(&db, ada.id, &ask("rope", 0, 4, 10, 2), now).await.unwrap();
        deliver(&db, bo.id, second.id, &hand(ore, None, Some(1)), now).await.unwrap();
        // Not due yet
        assert!(expire_due(&db, now + Duration::days(1)).await.unwrap().is_empty());
        let expired = expire_due(&db, now + Duration::days(2)).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!((expired[0].refunded, expired[0].commission.status), (30, CommissionStatus::Expired));
        let notice = expired[0].notice.clone().unwrap();
        assert_eq!(
            notice.message,
            ServerMessage::CommissionExpired {
                commission_id: second.id,
                item_type: "rope".into(),
                delivered: 1,
                quantity: 4,
                refunded: 30,
            }
        );
        // Once only
        assert!(expire_due(&db, now + Duration::days(3)).await.unwrap().is_empty());
        assert_eq!(balance(&db, ada.id).await, 1000 - 100 - 6 + 70 - 40 - 6 + 30);
        assert_eq!(balance(&db, bo.id).await, 40);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_offline_poster_hears_of_each_delivery_and_the_expiry_once(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let ore = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::uniform(10, 70)).await;
        let c = post(&db, ada.id, &ask("rope", 0, 10, 10, 1), now).await.unwrap();
        deliver(&db, bo.id, c.id, &hand(ore, None, Some(2)), now).await.unwrap();
        deliver(&db, bo.id, c.id, &hand(ore, None, Some(3)), now).await.unwrap();
        expire_due(&db, now + Duration::days(1)).await.unwrap();

        let waiting = notices::take_all(&db, ada.id, 0.0).await.unwrap();
        let kinds: Vec<&str> = waiting.iter().map(|m| m.type_name()).collect();
        assert_eq!(kinds, ["commission_delivered", "commission_delivered", "commission_expired"]);
        assert!(notices::take_all(&db, ada.id, 0.0).await.unwrap().is_empty());
        // The deliverer was here and heard nothing queued
        assert!(notices::take_all(&db, bo.id, 0.0).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_box_never_turns_a_deliverer_away(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let full = box_of(&db, ada.id).await;
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'market_box'")
            .fetch_one(&db)
            .await
            .unwrap();
        let mut fillers = Vec::new();
        for slot in 0..slots {
            fillers.push(stack_of(&db, ada.id, full, slot, "workbench_kit", &Units::uniform(1, 10)).await);
        }
        let ore = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::uniform(6, 70)).await;
        let c = post(&db, ada.id, &ask("rope", 0, 6, 10, 1), now).await.unwrap();
        let first = deliver(&db, bo.id, c.id, &hand(ore, None, Some(2)), now).await.unwrap();
        let second = deliver(&db, bo.id, c.id, &hand(ore, None, Some(4)), now).await.unwrap();
        assert_eq!((first.market_box, second.market_box), (None, None));
        assert_eq!(balance(&db, bo.id).await, 60);
        assert_eq!(waiting_for(&db, ada.id).await, Units::uniform(6, 70));
        // Still no room: nothing moves
        assert!(move_waiting(&db).await.unwrap().is_empty());

        // Room for one stack: the older takes the slot, and the newer stacks
        // onto it
        sqlx::query("DELETE FROM items WHERE id = $1").bind(fillers[0]).execute(&db).await.unwrap();
        assert_eq!(move_waiting(&db).await.unwrap(), vec![(ada.id, full)]);
        assert_eq!(all_in(&db, full, "rope").await, Units::uniform(6, 70));
        assert!(waiting_for(&db, ada.id).await.is_empty());
        assert!(move_waiting(&db).await.unwrap().is_empty());
    }

    /// The ledger's books (`GET /admin/ledger`) across a commission's
    /// life: the open escrow is held, so no snapshot shows drift, and every
    /// flow is seen.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_ledger_holds_the_courts_escrow_without_drift(db: PgPool) {
        use crate::engine::ledger::books;
        use capitol_protocol::ledger::reconcile;

        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let rope = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::uniform(10, 70)).await;
        let start = books(&db).await.unwrap();

        let kept = post(&db, ada.id, &ask("rope", 50, 10, 10, 1), now).await.unwrap();
        let dropped = post(&db, ada.id, &ask("rope", 50, 5, 20, 3), now).await.unwrap();
        deliver(&db, bo.id, kept.id, &hand(rope, None, Some(4)), now).await.unwrap();
        let mid = books(&db).await.unwrap();
        // 60 still wanted on one, 100 on the other
        assert_eq!(mid.escrow_strands, 160);
        let books_mid = reconcile(&start, &mid);
        assert!(books_mid.strands.holds(), "{:#?}", books_mid.strands);
        assert!(books_mid.units.holds(), "{:#?}", books_mid.units);

        cancel(&db, ada.id, dropped.id).await.unwrap();
        expire_due(&db, now + Duration::days(1)).await.unwrap();
        let end = books(&db).await.unwrap();
        assert_eq!(end.escrow_strands, 0);
        let books = reconcile(&start, &end);
        assert!(books.strands.holds(), "{:#?}", books.strands);
        assert!(books.units.holds(), "{:#?}", books.units);
        let f = &books.strands.flows;
        assert_eq!((f["court_escrow"], f["court_payout"], f["court_refund"]), (-200, 40, 160));
        // Two fees of 5 + 1, a sink
        assert_eq!(f["court_fee"], -12);
    }

    /// For any run of posts, part deliveries, cancels and expiries, Strands
    /// held + in escrow + fees, and every unit of every type, are what there
    /// were at the start. Each open commission's escrow is its price on what's
    /// still wanted, and every balance is its start plus its audit entries.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn posts_deliveries_cancels_and_expiries_conserve_strands_and_units(db: PgPool) {
        let (mut delivered, mut waited) = (0, 0);
        for seed in 0..6 {
            let (d, w) = conservation_run(&db, seed, 120).await;
            delivered += d;
            waited += w;
        }
        assert!(delivered > 30, "only {delivered} deliveries in six runs");
        assert!(waited > 3, "only {waited} deliveries waited in the court");
    }

    /// One seeded run: how many deliveries there were, and how many of them
    /// waited in the court for room in the poster's box.
    async fn conservation_run(db: &PgPool, seed: u64, steps: usize) -> (usize, usize) {
        const TYPES: [&str; 2] = ["rope", "workbench_kit"];
        let mut rng = StdRng::seed_from_u64(seed);
        let mut players = Vec::new();
        for name in ["ada", "bea", "cy"] {
            let p = player(db, &format!("{name}{seed}"), rng.gen_range(0..1500)).await;
            let mut held = Units::new();
            for _ in 0..rng.gen_range(1..5) {
                held.add(rng.gen_range(0..=100), rng.gen_range(1..10));
            }
            stack_of(db, p.id, p.inventory, 0, "rope", &held).await;
            // Kits don't stack: one a slot
            for slot in 1..rng.gen_range(2..5) {
                stack_of(db, p.id, p.inventory, slot, "workbench_kit", &Units::uniform(1, rng.gen_range(0..=100))).await;
            }
            players.push(p);
        }
        // Now and then a box with no room, so units wait in the court
        let crowded = &players[(seed % 3) as usize];
        let crowded_box = box_of(db, crowded.id).await;
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'market_box'")
            .fetch_one(db)
            .await
            .unwrap();
        let mut fillers = Vec::new();
        for slot in 0..slots {
            fillers.push(stack_of(db, crowded.id, crowded_box, slot, "fiber", &Units::uniform(1, 5)).await);
        }

        let start_strands = balances_of(db, players.iter().map(|p| p.id)).await;
        let total_strands = strands_accounted(db).await;
        let mut start_units = BTreeMap::new();
        for item_type in TYPES {
            start_units.insert(item_type, every_unit(db, item_type).await);
        }

        let mut now = Utc::now();
        let (mut posted, mut refused, mut deliveries, mut expired, mut waited) = (Vec::new(), 0, 0, 0, 0);
        for step in 0..steps {
            let p = &players[rng.gen_range(0..players.len())];
            let item_type = TYPES[rng.gen_range(0..TYPES.len())];
            let outcome = match rng.gen_range(0..10) {
                0..=2 => {
                    let quality = [0, 0, 20, 40, 55, 75, 90][rng.gen_range(0..7)];
                    let req = ask(item_type, quality, rng.gen_range(1..12), rng.gen_range(1..40), rng.gen_range(1..=3));
                    post(db, p.id, &req, now).await.map(|c| posted.push((c.id, item_type)))
                }
                _ if posted.is_empty() => continue,
                3..=7 => {
                    let (c, wanted) = posted[rng.gen_range(0..posted.len())];
                    // Mostly what it asks for; now and then the wrong thing
                    let item_type = if rng.gen_bool(0.85) { wanted } else { item_type };
                    let stacks: Vec<Uuid> = sqlx::query_scalar(
                        "SELECT i.id FROM items i JOIN containers c ON c.id = i.container_id
                         WHERE i.owner_id = $1 AND i.item_type = $2 AND c.container_type IN ('player_inventory', 'market_box')
                         ORDER BY i.id",
                    )
                    .bind(p.id)
                    .bind(item_type)
                    .fetch_all(db)
                    .await
                    .unwrap();
                    let Some(&stack) = stacks.get(rng.gen_range(0..stacks.len().max(1))) else {
                        continue;
                    };
                    let named = if rng.gen_bool(0.3) {
                        Some(units(db, stack).await.pick(1, capitol_protocol::rules::stacking::Pick::Best).unwrap())
                    } else {
                        None
                    };
                    let at_most = rng.gen_bool(0.5).then(|| rng.gen_range(0..6));
                    deliver(db, p.id, c, &hand(stack, named, at_most), now).await.map(|d| {
                        deliveries += 1;
                        waited += usize::from(d.market_box.is_none());
                    })
                }
                8 => {
                    let (c, _) = posted[rng.gen_range(0..posted.len())];
                    cancel(db, p.id, c).await.map(|_| ())
                }
                _ => {
                    // Time passes: some come due
                    now += Duration::hours(rng.gen_range(1..30));
                    expired += expire_due(db, now).await.unwrap().len();
                    if rng.gen_bool(0.3) {
                        // The crowded box empties a slot now and then
                        if let Some(filler) = fillers.pop() {
                            sqlx::query("DELETE FROM items WHERE id = $1").bind(filler).execute(db).await.unwrap();
                            // A filler isn't one of the types counted
                        }
                    }
                    move_waiting(db).await.unwrap();
                    Ok(())
                }
            };
            if outcome.is_err() {
                refused += 1;
            }

            assert_eq!(strands_accounted(db).await, total_strands, "Strands after step {step} of run {seed}");
            for item_type in TYPES {
                assert_eq!(units_of_type(db, item_type).await, start_units[item_type], "{item_type} after step {step} of run {seed}");
            }
        }
        assert!(
            posted.len() > 10 && refused > 5,
            "run {seed}: {} posted, {refused} refused, {expired} expired; it should do plenty of each",
            posted.len()
        );

        for item_type in TYPES {
            assert_eq!(every_unit(db, item_type).await, start_units[item_type]);
        }
        let mut conn = db.acquire().await.unwrap();
        for (id, _) in &posted {
            let c = load(&mut conn, *id).await.unwrap().unwrap();
            match c.status {
                CommissionStatus::Open => assert_eq!(c.escrow_strands, commission_value(c.price, c.quantity - c.delivered)),
                CommissionStatus::Filled => assert_eq!((c.delivered, c.escrow_strands), (c.quantity, 0)),
                _ => assert_eq!(c.escrow_strands, 0),
            }
        }
        assert_audited(db, &start_strands).await;
        (deliveries, waited)
    }
}
