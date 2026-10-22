//! Commissions (commons plan D5, S2.1): standing requests on the hall's board
//! with the money already down, and claims on them (D6, S2.2). The routes
//! that call these are `routes::commissions`.
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
//! **Claims** (S2.2) hold a share for a deliverer:
//! - [`claim`] promises k units by `rules::court::claim_until`. While a claim
//!   stands, its undelivered share is the claimant's alone to deliver, and the
//!   claimant's deliveries count against it first. Delivered in full, it's
//!   kept.
//! - [`release`] gives one up: no breach with more than an hour left, a lapse
//!   after. [`claims_due`] lapses those that run out, which reopens their
//!   share, and tells each claimant when an hour is left.
//! - A lapsed claim is a breach on its claimant's record: standing (S3.1)
//!   reads `commission_claims`.
//! - The poster can't cancel a claimed share. A [`cancel`] while claims stand
//!   withdraws the commission instead: the unclaimed part is refunded, each
//!   claim's share is refunded as it ends short, and the commission closes
//!   when the last one ends.
//!
//! A claim "stands" until it ends; it holds its share only until its `until`.
//! One past it that the sweep hasn't reached yet holds nothing, and lapses
//! when it's reached, or when its commission closes.
//!
//! So for any run of these, the Strands held plus those in escrow plus the
//! fees are what there were at the start, and every unit is somewhere (the
//! conservation test). The bounds are `rules::court`, shared with the clients.

use capitol_protocol::rest::{
    ClaimCommission, ClaimResponse, Commission, CommissionClaim, CommissionDelivery, CommissionsQuery, DeliverCommission,
    PostCommission,
};
use capitol_protocol::rules::court::{
    self, commission_value, court_fee, validate_commission, ClaimError, ClaimStatus, CommissionStatus, DeliveryError,
    CLAIM_GRACE_SECS, MAX_OPEN_COMMISSIONS,
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
/// The most claims `GET /commissions/{id}/claims` answers with.
pub const MAX_CLAIMS_LISTED: i64 = 200;

/// Why the court refused.
#[derive(Debug)]
pub enum CourtError {
    /// The price, quantity, quality or due date is out of bounds
    Rules(court::CommissionError),
    /// The units named, or held, can't be delivered
    Delivery(DeliveryError),
    /// A claim on nothing, or on more than is unclaimed
    Claim(ClaimError),
    /// No such item type
    UnknownItemType,
    /// [`MAX_OPEN_COMMISSIONS`] are open already
    TooMany,
    /// Not enough Strands for the escrow and the fee
    Funds(String),
    /// No such commission (of yours, for a cancel)
    NotFound,
    /// It's already filled, cancelled or expired, or past due, or withdrawn
    NotOpen,
    /// A poster delivering to, or claiming, their own commission
    OwnCommission,
    /// The stack isn't the deliverer's, in their inventory or market box
    NotYourStack,
    /// The stack isn't of the commission's item type
    WrongItemType,
    /// What's still wanted is all held by others' claims
    Claimed,
    /// A second claim on the same commission while the first stands
    AlreadyClaimed,
    /// A release with no claim standing
    NoClaim,
    /// A cancel of a commission already withdrawn: it closes as its claims end
    ClaimsStand,
    Db(sqlx::Error),
}

impl CourtError {
    pub fn message(&self) -> String {
        match self {
            CourtError::Rules(e) => e.message().to_string(),
            CourtError::Delivery(e) => e.message().to_string(),
            CourtError::Claim(e) => e.message(),
            CourtError::UnknownItemType => "There's no such item".to_string(),
            CourtError::TooMany => format!("You have {MAX_OPEN_COMMISSIONS} commissions open already"),
            CourtError::Funds(m) => m.clone(),
            CourtError::NotFound => "No such commission".to_string(),
            CourtError::NotOpen => "That commission is no longer open".to_string(),
            CourtError::OwnCommission => "That's your own commission".to_string(),
            CourtError::NotYourStack => "That stack isn't in your inventory or market box".to_string(),
            CourtError::WrongItemType => "That stack isn't the item the commission asks for".to_string(),
            CourtError::Claimed => "What that commission still wants is claimed by others".to_string(),
            CourtError::AlreadyClaimed => "You have a claim on that commission already".to_string(),
            CourtError::NoClaim => "You have no claim on that commission".to_string(),
            CourtError::ClaimsStand => "It's taken down already, and closes as its claims end".to_string(),
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

/// A commission's row, locked for the rest of the transaction by [`lock`].
/// Every operation takes this lock before its claims', so they queue in one
/// order.
#[derive(sqlx::FromRow)]
struct Head {
    poster_id: Uuid,
    item_type: String,
    min_quality: i16,
    quantity: i32,
    delivered: i32,
    price_per_unit: i64,
    status: String,
    due_at: DateTime<Utc>,
    withdrawn: bool,
}

impl Head {
    fn is_open(&self) -> bool {
        self.status == CommissionStatus::Open.as_str()
    }
}

/// Lock the commission `commission_id`, if there is one.
async fn lock(conn: &mut PgConnection, commission_id: Uuid) -> Result<Option<Head>, sqlx::Error> {
    sqlx::query_as(
        "SELECT poster_id, item_type, min_quality, quantity, delivered, price_per_unit, status, due_at, withdrawn
         FROM commissions WHERE id = $1 FOR UPDATE",
    )
    .bind(commission_id)
    .fetch_optional(conn)
    .await
}

/// A claim that hasn't ended, as [`standing`] locks it.
#[derive(sqlx::FromRow, Clone)]
struct Standing {
    id: Uuid,
    claimant_id: Uuid,
    quantity: i32,
    delivered: i32,
    until: DateTime<Utc>,
}

impl Standing {
    /// What it still owes
    fn owed(&self) -> i32 {
        self.quantity - self.delivered
    }

    /// Whether it holds its share at `now`
    fn holds(&self, now: DateTime<Utc>) -> bool {
        self.until > now
    }
}

/// The claims standing on `commission_id` (locked by the caller), oldest
/// first, locked too.
async fn standing(conn: &mut PgConnection, commission_id: Uuid) -> Result<Vec<Standing>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, claimant_id, quantity, delivered, until FROM commission_claims
         WHERE commission_id = $1 AND status = 'open' ORDER BY claimed_at, id FOR UPDATE",
    )
    .bind(commission_id)
    .fetch_all(conn)
    .await
}

/// What the claims holding at `now` still owe: the share nobody else may
/// claim or deliver.
fn held_by(claims: &[Standing], now: DateTime<Utc>) -> i32 {
    claims.iter().filter(|k| k.holds(now)).map(Standing::owed).sum()
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
/// the deliverer is paid for them out of the escrow. Only the unclaimed part
/// and the deliverer's own claim may be delivered, and the units count
/// against their claim first. Nothing changes unless all of it does.
pub async fn deliver(
    db: &PgPool,
    deliverer_id: Uuid,
    commission_id: Uuid,
    req: &DeliverCommission,
    now: DateTime<Utc>,
) -> Result<Delivered, CourtError> {
    let mut tx = db.begin().await?;
    let Some(head) = lock(&mut tx, commission_id).await? else {
        return Err(CourtError::NotFound);
    };
    if !head.is_open() || head.due_at <= now {
        return Err(CourtError::NotOpen);
    }
    if head.poster_id == deliverer_id {
        return Err(CourtError::OwnCommission);
    }
    let claims = standing(&mut tx, commission_id).await?;
    let own = claims.iter().find(|k| k.claimant_id == deliverer_id && k.holds(now)).cloned();
    let left = head.quantity - head.delivered;
    let unclaimed = court::unclaimed(left, held_by(&claims, now), head.withdrawn);
    let open_to = court::deliverable(unclaimed, own.as_ref().map_or(0, Standing::owed));
    if open_to < 1 {
        return Err(CourtError::Claimed);
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
    if stack_type != head.item_type {
        return Err(CourtError::WrongItemType);
    }
    let (item_type, price, poster_id) = (head.item_type.as_str(), head.price_per_unit, head.poster_id);

    let held = units::units_of(&mut tx, req.item_id).await?;
    let wanted = court::delivery(&held, head.min_quality as i32, open_to, req.qualities.as_ref(), req.quantity)?;
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
            item_type,
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
    let filled = head.delivered + n == head.quantity;
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

    // Against their own claim first: delivered in full, it's kept
    if let Some(own) = own {
        let toward = n.min(own.owed());
        sqlx::query("UPDATE commission_claims SET delivered = delivered + $2 WHERE id = $1")
            .bind(own.id)
            .bind(toward)
            .execute(&mut *tx)
            .await?;
        if toward == own.owed() {
            finish(&mut tx, &head, commission_id, &own, ClaimStatus::Kept, now).await?;
        }
    }
    if !filled {
        // A withdrawn commission closes with its last claim
        settle(&mut tx, commission_id, now).await?;
    }

    let (deliverer_name, deliverer_citizen) = name_of(&mut tx, deliverer_id).await?;
    let message = ServerMessage::CommissionDelivered {
        commission_id,
        item_type: head.item_type.clone(),
        deliverer_id,
        deliverer_name,
        deliverer_citizen,
        units: taken.units.clone(),
        paid,
        left: left - n,
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

/// A commission taken down: by its poster, or by its due date. Or, if its
/// poster cancelled it while claims stood, withdrawn: still open, waiting on
/// its claims, with the unclaimed part refunded.
#[derive(Debug, Clone, PartialEq)]
pub struct Closed {
    /// As it stands now: `cancelled`, `expired`, or `open` and `withdrawn`
    pub commission: Commission,
    /// The escrow refunded, now back on the poster's balance
    pub refunded: i64,
    /// An expiry's `commission_expired` to the poster, and a `claim_lapsed`
    /// to each side of each claim it ended
    pub notices: Vec<Notice>,
}

/// Cancel `poster_id`'s open commission `commission_id` at `now`: the unspent
/// escrow comes back. The fee isn't refunded. While claims hold their
/// shares it's withdrawn instead: only the unclaimed part comes back now, and
/// the rest as the claims end short. Works from anywhere.
pub async fn cancel(db: &PgPool, poster_id: Uuid, commission_id: Uuid, now: DateTime<Utc>) -> Result<Closed, CourtError> {
    let mut tx = db.begin().await?;
    let head = match lock(&mut tx, commission_id).await? {
        Some(head) if head.poster_id == poster_id => head,
        _ => return Err(CourtError::NotFound),
    };
    if !head.is_open() {
        return Err(CourtError::NotOpen);
    }
    let claims = standing(&mut tx, commission_id).await?;
    let closed = if !claims.iter().any(|k| k.holds(now)) {
        close(&mut tx, commission_id, CommissionStatus::Cancelled, now).await?
    } else if head.withdrawn {
        return Err(CourtError::ClaimsStand);
    } else {
        let owed: i32 = claims.iter().map(Standing::owed).sum();
        let refunded = commission_value(head.price_per_unit, head.quantity - head.delivered - owed);
        if refunded > 0 {
            add_strands_in(
                &mut tx,
                poster_id,
                refunded,
                TransactionType::CourtRefund,
                Some(&format!("Commission withdrawn {commission_id}")),
                Some(commission_id),
            )
            .await
            .map_err(CourtError::Funds)?;
        }
        sqlx::query("UPDATE commissions SET withdrawn = TRUE, escrow_strands = escrow_strands - $2 WHERE id = $1")
            .bind(commission_id)
            .bind(refunded)
            .execute(&mut *tx)
            .await?;
        let commission = load(&mut tx, commission_id).await?.ok_or(CourtError::NotFound)?;
        Closed { commission, refunded, notices: Vec::new() }
    };
    tx.commit().await?;
    Ok(closed)
}

/// Take the open commission `commission_id` (locked by the caller) down as
/// `status` at `now`, refunding what's unspent of its escrow. Any claim still
/// standing lapses. An expiry queues the poster's notice.
async fn close(
    conn: &mut PgConnection,
    commission_id: Uuid,
    status: CommissionStatus,
    now: DateTime<Utc>,
) -> Result<Closed, CourtError> {
    let head = lock(conn, commission_id).await?.ok_or(CourtError::NotFound)?;
    let mut notices = Vec::new();
    for claim in standing(conn, commission_id).await? {
        notices.extend(finish(conn, &head, commission_id, &claim, ClaimStatus::Lapsed, now).await?);
    }
    let escrow_strands: i64 = sqlx::query_scalar("SELECT escrow_strands FROM commissions WHERE id = $1")
        .bind(commission_id)
        .fetch_one(&mut *conn)
        .await?;
    let poster_id = head.poster_id;
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
    if status == CommissionStatus::Expired {
        let message = ServerMessage::CommissionExpired {
            commission_id,
            item_type: commission.item_type.clone(),
            delivered: commission.delivered,
            quantity: commission.quantity,
            refunded: escrow_strands,
        };
        let notice_id = notices::queue(&mut *conn, poster_id, &commission_id.to_string(), &message).await?;
        notices.push(Notice { player_id: poster_id, notice_id, message });
    }
    Ok(Closed { commission, refunded: escrow_strands, notices })
}

/// Close the withdrawn commission `commission_id` (locked by the caller) as
/// cancelled once no claim stands on it. What its claims delivered stays
/// delivered; its escrow is spent or refunded by then.
async fn settle(conn: &mut PgConnection, commission_id: Uuid, now: DateTime<Utc>) -> Result<Option<Closed>, CourtError> {
    let waiting: bool = sqlx::query_scalar(
        "SELECT c.status = 'open' AND c.withdrawn
             AND NOT EXISTS (SELECT 1 FROM commission_claims k WHERE k.commission_id = c.id AND k.status = 'open')
         FROM commissions c WHERE c.id = $1",
    )
    .bind(commission_id)
    .fetch_one(&mut *conn)
    .await?;
    if !waiting {
        return Ok(None);
    }
    Ok(Some(close(conn, commission_id, CommissionStatus::Cancelled, now).await?))
}

/// Expire every open commission due by `now`, each in a transaction of its
/// own. Called every market cycle (a second), after [`claims_due`].
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
            expired.push(close(&mut tx, id, CommissionStatus::Expired, now).await?);
        }
        tx.commit().await?;
    }
    Ok(expired)
}

/// A claim made: the answer, and the poster's `commission_claimed`.
#[derive(Debug, Clone, PartialEq)]
pub struct Claimed {
    pub response: ClaimResponse,
    pub notice: Notice,
}

/// Claim `req.quantity` of the open commission `commission_id` for
/// `claimant_id` at `now`: that share is held for them until
/// `rules::court::claim_until`. One standing claim each, never on your own,
/// and never more than is unclaimed. Works from anywhere.
pub async fn claim(
    db: &PgPool,
    claimant_id: Uuid,
    commission_id: Uuid,
    req: &ClaimCommission,
    now: DateTime<Utc>,
) -> Result<Claimed, CourtError> {
    let mut tx = db.begin().await?;
    let Some(head) = lock(&mut tx, commission_id).await? else {
        return Err(CourtError::NotFound);
    };
    if !head.is_open() || head.withdrawn || head.due_at <= now {
        return Err(CourtError::NotOpen);
    }
    if head.poster_id == claimant_id {
        return Err(CourtError::OwnCommission);
    }
    let claims = standing(&mut tx, commission_id).await?;
    if claims.iter().any(|k| k.claimant_id == claimant_id) {
        return Err(CourtError::AlreadyClaimed);
    }
    let unclaimed = court::unclaimed(head.quantity - head.delivered, held_by(&claims, now), head.withdrawn);
    court::validate_claim(req.quantity, unclaimed).map_err(CourtError::Claim)?;

    let id = Uuid::new_v4();
    // As stored, to the microsecond, so the notice and the claim agree
    let until: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO commission_claims (id, commission_id, claimant_id, quantity, claimed_at, until)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING until",
    )
    .bind(id)
    .bind(commission_id)
    .bind(claimant_id)
    .bind(req.quantity)
    .bind(now)
    .bind(court::claim_until(now, head.due_at))
    .fetch_one(&mut *tx)
    .await?;

    let (claimant_name, claimant_citizen) = name_of(&mut tx, claimant_id).await?;
    let message = ServerMessage::CommissionClaimed {
        commission_id,
        claim_id: id,
        item_type: head.item_type.clone(),
        claimant_id,
        claimant_name,
        claimant_citizen,
        quantity: req.quantity,
        until,
    };
    let notice_id = notices::queue(&mut *tx, head.poster_id, &id.to_string(), &message).await?;
    let claim = load_claim(&mut tx, id).await?.ok_or(CourtError::NoClaim)?;
    let commission = load(&mut tx, commission_id).await?.ok_or(CourtError::NotFound)?;
    tx.commit().await?;
    Ok(Claimed {
        response: ClaimResponse { claim, commission },
        notice: Notice { player_id: head.poster_id, notice_id, message },
    })
}

/// A claim ended short, by its claimant or by its time.
#[derive(Debug, Clone, PartialEq)]
pub struct Ended {
    /// The claim as it ended, and its commission after it: open with the
    /// share back on the board, or, withdrawn, perhaps closed by it
    pub response: ClaimResponse,
    /// Escrow back on the poster's balance: a withdrawn commission's share
    pub refunded: i64,
    /// A lapse's `claim_lapsed`, to the poster and the claimant
    pub notices: Vec<Notice>,
}

/// Give up `claimant_id`'s claim on `commission_id` at `now`: released with
/// more than an hour left, lapsed (a breach) after. Its share goes back to
/// the board. Works from anywhere.
pub async fn release(db: &PgPool, claimant_id: Uuid, commission_id: Uuid, now: DateTime<Utc>) -> Result<Ended, CourtError> {
    let mut tx = db.begin().await?;
    let Some(head) = lock(&mut tx, commission_id).await? else {
        return Err(CourtError::NotFound);
    };
    let claims = standing(&mut tx, commission_id).await?;
    let claim = claims.into_iter().find(|k| k.claimant_id == claimant_id).ok_or(CourtError::NoClaim)?;
    let ended = end(&mut tx, &head, commission_id, &claim, court::release_status(now, claim.until), now).await?;
    tx.commit().await?;
    Ok(ended)
}

/// End the standing `claim` on the commission `head` (both locked) as
/// `status` at `now`, short of what it promised. A withdrawn commission
/// refunds the share, and closes if it was the last claim.
async fn end(
    conn: &mut PgConnection,
    head: &Head,
    commission_id: Uuid,
    claim: &Standing,
    status: ClaimStatus,
    now: DateTime<Utc>,
) -> Result<Ended, CourtError> {
    let notices = finish(conn, head, commission_id, claim, status, now).await?;
    let mut refunded = 0;
    if head.withdrawn && claim.owed() > 0 {
        refunded = commission_value(head.price_per_unit, claim.owed());
        add_strands_in(
            conn,
            head.poster_id,
            refunded,
            TransactionType::CourtRefund,
            Some(&format!("Claim on commission {commission_id} {}", status.as_str())),
            Some(commission_id),
        )
        .await
        .map_err(CourtError::Funds)?;
        sqlx::query("UPDATE commissions SET escrow_strands = escrow_strands - $2 WHERE id = $1")
            .bind(commission_id)
            .bind(refunded)
            .execute(&mut *conn)
            .await?;
    }
    if let Some(closed) = settle(conn, commission_id, now).await? {
        refunded += closed.refunded;
    }
    let claim = load_claim(conn, claim.id).await?.ok_or(CourtError::NoClaim)?;
    let commission = load(conn, commission_id).await?.ok_or(CourtError::NotFound)?;
    Ok(Ended { response: ClaimResponse { claim, commission }, refunded, notices })
}

/// Mark the standing `claim` on the commission `head` ended as `status` at
/// `now`. A lapse queues `claim_lapsed` for the poster and the claimant, and
/// returns them.
async fn finish(
    conn: &mut PgConnection,
    head: &Head,
    commission_id: Uuid,
    claim: &Standing,
    status: ClaimStatus,
    now: DateTime<Utc>,
) -> Result<Vec<Notice>, sqlx::Error> {
    sqlx::query("UPDATE commission_claims SET status = $2, ended_at = $3 WHERE id = $1")
        .bind(claim.id)
        .bind(status.as_str())
        .bind(now)
        .execute(&mut *conn)
        .await?;
    if status != ClaimStatus::Lapsed {
        return Ok(Vec::new());
    }
    let (claimant_name, claimant_citizen) = name_of(conn, claim.claimant_id).await?;
    let message = ServerMessage::ClaimLapsed {
        commission_id,
        claim_id: claim.id,
        item_type: head.item_type.clone(),
        claimant_id: claim.claimant_id,
        claimant_name,
        claimant_citizen,
        quantity: claim.quantity,
        delivered: claim.delivered,
    };
    let mut told = Vec::new();
    for player_id in [head.poster_id, claim.claimant_id] {
        let notice_id = notices::queue(&mut *conn, player_id, &claim.id.to_string(), &message).await?;
        told.push(Notice { player_id, notice_id, message: message.clone() });
    }
    Ok(told)
}

/// What [`claims_due`] did.
#[derive(Debug, Default)]
pub struct ClaimSweep {
    /// Claims that ran out, each lapsed
    pub lapsed: Vec<Ended>,
    /// `claim_due_soon` to each claimant with an hour or less left
    pub warned: Vec<Notice>,
}

/// Lapse every claim that ran out by `now`, each in a transaction of its
/// own, and tell each claimant with [`CLAIM_GRACE_SECS`] or less left, once.
/// A claim made with no more than that to run isn't told. Called every
/// market cycle (a second).
pub async fn claims_due(db: &PgPool, now: DateTime<Utc>) -> Result<ClaimSweep, CourtError> {
    let mut sweep = ClaimSweep::default();
    let due: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, commission_id FROM commission_claims WHERE status = 'open' AND until <= $1 ORDER BY until, id",
    )
    .bind(now)
    .fetch_all(db)
    .await?;
    for (claim_id, commission_id) in due {
        let mut tx = db.begin().await?;
        let Some(head) = lock(&mut tx, commission_id).await? else {
            continue;
        };
        // Its claimant may have delivered or released it meanwhile
        let claim = standing(&mut tx, commission_id)
            .await?
            .into_iter()
            .find(|k| k.id == claim_id && !k.holds(now));
        if let Some(claim) = claim {
            sweep.lapsed.push(end(&mut tx, &head, commission_id, &claim, ClaimStatus::Lapsed, now).await?);
        }
        tx.commit().await?;
    }

    let grace = Duration::seconds(CLAIM_GRACE_SECS);
    let mut tx = db.begin().await?;
    let warn: Vec<(Uuid, Uuid, Uuid, String, i32, i32, DateTime<Utc>)> = sqlx::query_as(
        "UPDATE commission_claims k SET warned = TRUE
         FROM commissions c
         WHERE c.id = k.commission_id AND k.status = 'open' AND NOT k.warned
             AND k.until > $1 AND k.until <= $2 AND k.claimed_at < k.until - ($2 - $1)
         RETURNING k.id, k.commission_id, k.claimant_id, c.item_type, k.quantity, k.delivered, k.until",
    )
    .bind(now)
    .bind(now + grace)
    .fetch_all(&mut *tx)
    .await?;
    for (claim_id, commission_id, claimant_id, item_type, quantity, delivered, until) in warn {
        let message = ServerMessage::ClaimDueSoon { commission_id, claim_id, item_type, quantity, delivered, until };
        let notice_id = notices::queue(&mut *tx, claimant_id, &claim_id.to_string(), &message).await?;
        sweep.warned.push(Notice { player_id: claimant_id, notice_id, message });
    }
    tx.commit().await?;
    Ok(sweep)
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
type Row = (
    Uuid,
    Uuid,
    String,
    bool,
    String,
    i16,
    i32,
    i32,
    i64,
    i64,
    i64,
    DateTime<Utc>,
    DateTime<Utc>,
    String,
    i32,
    bool,
);

/// The columns of a [`Row`], over `commissions c` joined to its poster.
/// `claimed` is what the standing claims still owe.
const COLUMNS: &str = "c.id, c.poster_id, p.name, a.kind = 'citizen', c.item_type, c.min_quality, c.quantity,
     c.delivered, c.price_per_unit, c.escrow_strands, c.fee, c.posted_at, c.due_at, c.status,
     (SELECT COALESCE(SUM(k.quantity - k.delivered), 0) FROM commission_claims k
      WHERE k.commission_id = c.id AND k.status = 'open')::INT,
     c.withdrawn
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
        claimed,
        withdrawn,
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
        claimed,
        withdrawn,
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

/// A `commission_claims` row with its claimant, as [`CLAIM_COLUMNS`] selects it
type ClaimRow = (Uuid, Uuid, Uuid, String, bool, i32, i32, DateTime<Utc>, DateTime<Utc>, String);

/// The columns of a [`ClaimRow`], over `commission_claims k` joined to its claimant
const CLAIM_COLUMNS: &str = "k.id, k.commission_id, k.claimant_id, p.name, a.kind = 'citizen', k.quantity, k.delivered,
     k.claimed_at, k.until, k.status
     FROM commission_claims k JOIN players p ON p.id = k.claimant_id JOIN accounts a ON a.id = p.account_id";

fn claim_from_row(row: ClaimRow) -> Result<CommissionClaim, sqlx::Error> {
    let (id, commission_id, claimant_id, claimant_name, claimant_citizen, quantity, delivered, claimed_at, until, status) =
        row;
    Ok(CommissionClaim {
        id,
        commission_id,
        claimant_id,
        claimant_name,
        claimant_citizen,
        quantity,
        delivered,
        claimed_at,
        until,
        status: ClaimStatus::parse(&status)
            .ok_or_else(|| sqlx::Error::Decode(format!("commission_claims.status is {status:?}").into()))?,
    })
}

/// The claim `claim_id`.
pub async fn load_claim(conn: &mut PgConnection, claim_id: Uuid) -> Result<Option<CommissionClaim>, sqlx::Error> {
    let row: Option<ClaimRow> = sqlx::query_as(&format!("SELECT {CLAIM_COLUMNS} WHERE k.id = $1"))
        .bind(claim_id)
        .fetch_optional(conn)
        .await?;
    row.map(claim_from_row).transpose()
}

/// Every claim on `commission_id`: standing first, then newest first, at
/// most [`MAX_CLAIMS_LISTED`].
pub async fn claims_on(db: &PgPool, commission_id: Uuid) -> Result<Vec<CommissionClaim>, sqlx::Error> {
    let rows: Vec<ClaimRow> = sqlx::query_as(&format!(
        "SELECT {CLAIM_COLUMNS} WHERE k.commission_id = $1
         ORDER BY k.status = 'open' DESC, k.claimed_at DESC, k.id LIMIT $2"
    ))
    .bind(commission_id)
    .bind(MAX_CLAIMS_LISTED)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(claim_from_row).collect()
}

/// `claimant_id`'s standing claims, the soonest to run out first.
pub async fn claims_of(db: &PgPool, claimant_id: Uuid) -> Result<Vec<CommissionClaim>, sqlx::Error> {
    let rows: Vec<ClaimRow> = sqlx::query_as(&format!(
        "SELECT {CLAIM_COLUMNS} WHERE k.claimant_id = $1 AND k.status = 'open' ORDER BY k.until, k.id"
    ))
    .bind(claimant_id)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(claim_from_row).collect()
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
        assert!(matches!(cancel(&db, ada.id, posted.id, now).await, Err(CourtError::NotOpen)));
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
        cancel(&db, ada.id, open[0].id, now).await.unwrap();
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
        assert!(matches!(cancel(&db, bo.id, c.id, now).await, Err(CourtError::NotFound)));
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
        let cancelled = cancel(&db, ada.id, first.id, now).await.unwrap();
        assert_eq!((cancelled.refunded, cancelled.notices.len()), (70, 0));
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
        let notice = expired[0].notices[0].clone();
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

        cancel(&db, ada.id, dropped.id, now).await.unwrap();
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

    /// For any run of posts, part deliveries, claims, releases, cancels,
    /// withdrawals, lapses and expiries, Strands held + in escrow + fees, and
    /// every unit of every type, are what there were at the start. Each open
    /// commission's escrow is its price on what's still wanted (on what its
    /// claims still owe, once withdrawn), and every balance is its start plus
    /// its audit entries.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn posts_deliveries_cancels_and_expiries_conserve_strands_and_units(db: PgPool) {
        let mut total = Run::default();
        for seed in 0..6 {
            let run = conservation_run(&db, seed, 160).await;
            total.deliveries += run.deliveries;
            total.waited += run.waited;
            total.claims += run.claims;
            total.ended_short += run.ended_short;
            total.withdrawn += run.withdrawn;
        }
        assert!(total.deliveries > 30, "only {} deliveries in six runs", total.deliveries);
        assert!(total.waited > 3, "only {} deliveries waited in the court", total.waited);
        assert!(total.claims > 15, "only {} claims in six runs", total.claims);
        assert!(total.ended_short > 5, "only {} claims released or lapsed in six runs", total.ended_short);
        assert!(total.withdrawn > 1, "only {} commissions withdrawn in six runs", total.withdrawn);
    }

    /// What one seeded run did.
    #[derive(Default)]
    struct Run {
        deliveries: usize,
        /// Deliveries that waited in the court for room in the poster's box
        waited: usize,
        claims: usize,
        /// Claims released or lapsed
        ended_short: usize,
        /// Cancels while claims stood
        withdrawn: usize,
    }

    /// One seeded run.
    async fn conservation_run(db: &PgPool, seed: u64, steps: usize) -> Run {
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
        let (mut posted, mut refused, mut expired) = (Vec::new(), 0, 0);
        let mut run = Run::default();
        for step in 0..steps {
            let p = &players[rng.gen_range(0..players.len())];
            let item_type = TYPES[rng.gen_range(0..TYPES.len())];
            let outcome = match rng.gen_range(0..13) {
                0..=2 => {
                    let quality = [0, 0, 20, 40, 55, 75, 90][rng.gen_range(0..7)];
                    let req = ask(item_type, quality, rng.gen_range(1..12), rng.gen_range(1..40), rng.gen_range(1..=3));
                    post(db, p.id, &req, now).await.map(|c| posted.push((c.id, item_type, p.id)))
                }
                _ if posted.is_empty() => continue,
                3..=7 => {
                    let (c, wanted, _) = posted[rng.gen_range(0..posted.len())];
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
                        run.deliveries += 1;
                        run.waited += usize::from(d.market_box.is_none());
                    })
                }
                8 => {
                    // Mostly one of their own, so some are withdrawn under claims
                    let own: Vec<Uuid> = posted.iter().filter(|(_, _, by)| *by == p.id).map(|(c, _, _)| *c).collect();
                    let c = match own.get(rng.gen_range(0..own.len().max(1))) {
                        Some(&c) if rng.gen_bool(0.8) => c,
                        _ => posted[rng.gen_range(0..posted.len())].0,
                    };
                    cancel(db, p.id, c, now).await.map(|closed| {
                        run.withdrawn += usize::from(closed.commission.status == CommissionStatus::Open);
                    })
                }
                9 | 10 => {
                    let (c, _, _) = posted[rng.gen_range(0..posted.len())];
                    claim(db, p.id, c, &promise(rng.gen_range(0..8)), now).await.map(|_| run.claims += 1)
                }
                11 => {
                    let (c, _, _) = posted[rng.gen_range(0..posted.len())];
                    release(db, p.id, c, now).await.map(|_| run.ended_short += 1)
                }
                _ => {
                    // Time passes: some claims run out, some commissions come due
                    now += Duration::minutes(rng.gen_range(10..30 * 60));
                    run.ended_short += claims_due(db, now).await.unwrap().lapsed.len();
                    expired += expire_due(db, now).await.unwrap().len();
                    if rng.gen_bool(0.15) {
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
        for (id, _, _) in &posted {
            let c = load(&mut conn, *id).await.unwrap().unwrap();
            match c.status {
                // Withdrawn, it holds only what its claims still owe
                CommissionStatus::Open if c.withdrawn => {
                    assert_eq!(c.escrow_strands, commission_value(c.price, c.claimed));
                    assert!(c.claimed > 0, "a withdrawn commission with no claims closes");
                }
                CommissionStatus::Open => {
                    assert_eq!(c.escrow_strands, commission_value(c.price, c.quantity - c.delivered));
                    assert!(c.claimed <= c.quantity - c.delivered, "claims hold more than is wanted");
                }
                CommissionStatus::Filled => assert_eq!((c.delivered, c.escrow_strands), (c.quantity, 0)),
                _ => assert_eq!((c.escrow_strands, c.claimed), (0, 0)),
            }
            // Kept is delivered in full; nothing delivers more than it promised
            for k in claims_on(db, *id).await.unwrap() {
                assert!(k.delivered <= k.quantity);
                if k.status == ClaimStatus::Kept {
                    assert_eq!(k.delivered, k.quantity);
                }
            }
        }
        assert_audited(db, &start_strands).await;
        run
    }

    fn promise(quantity: i32) -> ClaimCommission {
        ClaimCommission { quantity }
    }

    /// Bo's claim holds 4 of 10: Cy can deliver the other 6 and no more, and
    /// can't claim the held share either.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_holds_its_share_against_another_deliverer(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let cy = player(&db, "cy", 0).await;
        let cy_rope = stack_of(&db, cy.id, cy.inventory, 0, "rope", &Units::uniform(10, 70)).await;
        let c = post(&db, ada.id, &ask("rope", 0, 10, 10, 3), now).await.unwrap();

        let made = claim(&db, bo.id, c.id, &promise(4), now).await.unwrap();
        let k = &made.response.claim;
        assert_eq!((k.quantity, k.delivered, k.status), (4, 0, ClaimStatus::Open));
        // A day, as the commission runs three
        assert_eq!((k.until - now - Duration::days(1)).num_milliseconds(), 0);
        assert_eq!(made.response.commission.claimed, 4);
        // The poster hears who promised what
        assert_eq!(made.notice.player_id, ada.id);
        match &made.notice.message {
            ServerMessage::CommissionClaimed { claimant_name, quantity, until, .. } => {
                assert_eq!((claimant_name.as_str(), *quantity, *until), ("bo", 4, k.until));
            }
            other => panic!("the notice is {other:?}"),
        }

        // Cy brings all ten: only the six unclaimed are taken
        let done = deliver(&db, cy.id, c.id, &hand(cy_rope, None, None), now).await.unwrap();
        assert_eq!(done.delivery.units.count(), 6);
        assert_eq!((done.delivery.commission.delivered, done.delivery.commission.claimed), (6, 4));
        assert_eq!(done.delivery.commission.status, CommissionStatus::Open);
        // Nothing more for Cy, to deliver or to claim
        assert!(matches!(deliver(&db, cy.id, c.id, &hand(cy_rope, None, None), now).await, Err(CourtError::Claimed)));
        assert!(matches!(
            claim(&db, cy.id, c.id, &promise(1), now).await,
            Err(CourtError::Claim(ClaimError::TooMany(0)))
        ));
        assert_eq!(units(&db, cy_rope).await, Units::uniform(4, 70));
        assert_eq!(balance(&db, cy.id).await, 60);
    }

    /// Bo delivers his claim in two parts; it's kept once all four are in,
    /// and his units count against it before the unclaimed part.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_delivered_in_full_is_kept(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let rope = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::uniform(10, 70)).await;
        let c = post(&db, ada.id, &ask("rope", 0, 10, 10, 3), now).await.unwrap();
        let k = claim(&db, bo.id, c.id, &promise(4), now).await.unwrap().response.claim;

        let first = deliver(&db, bo.id, c.id, &hand(rope, None, Some(3)), now).await.unwrap();
        assert_eq!(first.delivery.commission.claimed, 1);
        let mut conn = db.acquire().await.unwrap();
        let open = load_claim(&mut conn, k.id).await.unwrap().unwrap();
        assert_eq!((open.delivered, open.status), (3, ClaimStatus::Open));
        // Three more: one finishes the claim, two come off the unclaimed part
        let second = deliver(&db, bo.id, c.id, &hand(rope, None, Some(3)), now + Duration::hours(2)).await.unwrap();
        assert_eq!((second.delivery.commission.delivered, second.delivery.commission.claimed), (6, 0));
        let kept = load_claim(&mut conn, k.id).await.unwrap().unwrap();
        assert_eq!((kept.delivered, kept.status), (4, ClaimStatus::Kept));
        assert!(claims_of(&db, bo.id).await.unwrap().is_empty());
        // Kept, it's no claim to release, and a lapse sweep passes it by
        assert!(matches!(release(&db, bo.id, c.id, now).await, Err(CourtError::NoClaim)));
        assert!(claims_due(&db, now + Duration::days(2)).await.unwrap().lapsed.is_empty());
        // And Bo may promise again
        claim(&db, bo.id, c.id, &promise(2), now + Duration::hours(3)).await.unwrap();
    }

    /// Bo promises 4, delivers 1 and runs out of time: the claim lapses,
    /// both sides hear, the 3 he owed go back to the board, and the lapse
    /// stays on his record.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_lapsed_claim_reopens_its_share_and_records_a_breach(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let cy = player(&db, "cy", 0).await;
        let rope = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::uniform(1, 70)).await;
        let cy_rope = stack_of(&db, cy.id, cy.inventory, 0, "rope", &Units::uniform(10, 70)).await;
        let c = post(&db, ada.id, &ask("rope", 0, 4, 10, 3), now).await.unwrap();
        let k = claim(&db, bo.id, c.id, &promise(4), now).await.unwrap().response.claim;
        deliver(&db, bo.id, c.id, &hand(rope, None, None), now).await.unwrap();

        // Not yet
        assert!(claims_due(&db, k.until - Duration::seconds(1)).await.unwrap().lapsed.is_empty());
        let sweep = claims_due(&db, k.until).await.unwrap();
        assert_eq!(sweep.lapsed.len(), 1);
        let lapsed = &sweep.lapsed[0];
        assert_eq!((lapsed.response.claim.status, lapsed.response.claim.delivered), (ClaimStatus::Lapsed, 1));
        assert_eq!((lapsed.response.commission.claimed, lapsed.refunded), (0, 0));
        let told: Vec<Uuid> = lapsed.notices.iter().map(|n| n.player_id).collect();
        assert_eq!(told, [ada.id, bo.id]);
        assert!(matches!(
            &lapsed.notices[0].message,
            ServerMessage::ClaimLapsed { quantity: 4, delivered: 1, claimant_name, .. } if claimant_name == "bo"
        ));
        // Once only
        assert!(claims_due(&db, k.until + Duration::hours(1)).await.unwrap().lapsed.is_empty());

        // The share is open: Cy fills the rest
        let done = deliver(&db, cy.id, c.id, &hand(cy_rope, None, None), k.until).await.unwrap();
        assert_eq!((done.delivery.units.count(), done.delivery.commission.status), (3, CommissionStatus::Filled));
        // The breach, where standing (S3.1) will read it
        let breaches: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM commission_claims WHERE claimant_id = $1 AND status = 'lapsed'",
        )
        .bind(bo.id)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(breaches, 1);
        // Away, Bo hears of it as he comes back: Ada heard of the claim too
        let bo_heard: Vec<&str> = notices::take_all(&db, bo.id, 0.0).await.unwrap().iter().map(|m| m.type_name()).collect();
        assert_eq!(bo_heard, ["claim_due_soon", "claim_lapsed"]);
        let ada_heard: Vec<String> =
            notices::take_all(&db, ada.id, 0.0).await.unwrap().iter().map(|m| m.type_name().to_string()).collect();
        assert_eq!(
            ada_heard,
            ["commission_claimed", "commission_delivered", "claim_lapsed", "commission_delivered"]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_early_release_is_no_breach_and_a_late_one_is(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let c = post(&db, ada.id, &ask("rope", 0, 10, 10, 3), now).await.unwrap();

        let early = claim(&db, bo.id, c.id, &promise(5), now).await.unwrap().response.claim;
        let released = release(&db, bo.id, c.id, early.until - Duration::minutes(61)).await.unwrap();
        assert_eq!(released.response.claim.status, ClaimStatus::Released);
        assert!(released.notices.is_empty());
        assert_eq!(released.response.commission.claimed, 0);

        let late = claim(&db, bo.id, c.id, &promise(5), now + Duration::hours(1)).await.unwrap().response.claim;
        let lapsed = release(&db, bo.id, c.id, late.until - Duration::minutes(59)).await.unwrap();
        assert_eq!(lapsed.response.claim.status, ClaimStatus::Lapsed);
        assert_eq!(lapsed.notices.len(), 2);
        // Nothing left to release
        assert!(matches!(release(&db, bo.id, c.id, now).await, Err(CourtError::NoClaim)));
    }

    /// Ada cancels with Bo's claim standing: the six unclaimed come back,
    /// the four Bo holds stay his to deliver, and it closes when he's done.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_poster_cannot_cancel_a_claimed_share(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let cy = player(&db, "cy", 0).await;
        let rope = stack_of(&db, bo.id, bo.inventory, 0, "rope", &Units::uniform(10, 70)).await;
        let cy_rope = stack_of(&db, cy.id, cy.inventory, 0, "rope", &Units::uniform(10, 70)).await;
        let c = post(&db, ada.id, &ask("rope", 0, 10, 10, 3), now).await.unwrap();
        claim(&db, bo.id, c.id, &promise(4), now).await.unwrap();
        claim(&db, cy.id, c.id, &promise(2), now).await.unwrap();
        deliver(&db, bo.id, c.id, &hand(rope, None, Some(1)), now).await.unwrap();
        let before = balance(&db, ada.id).await;

        let withdrawn = cancel(&db, ada.id, c.id, now).await.unwrap();
        // 9 still wanted, 5 of them claimed: 4 come back
        assert_eq!(withdrawn.refunded, 40);
        let w = &withdrawn.commission;
        assert_eq!((w.status, w.withdrawn, w.claimed, w.escrow_strands), (CommissionStatus::Open, true, 5, 50));
        assert_eq!(balance(&db, ada.id).await, before + 40);
        // Not twice, and nothing new is claimed or delivered off a claim
        assert!(matches!(cancel(&db, ada.id, c.id, now).await, Err(CourtError::ClaimsStand)));
        let dee = player(&db, "dee", 0).await;
        assert!(matches!(claim(&db, dee.id, c.id, &promise(1), now).await, Err(CourtError::NotOpen)));
        // Bo brings his other three, and can't bring a fourth
        let done = deliver(&db, bo.id, c.id, &hand(rope, None, None), now).await.unwrap();
        assert_eq!(done.delivery.units.count(), 3);
        assert!(matches!(deliver(&db, bo.id, c.id, &hand(rope, None, None), now).await, Err(CourtError::Claimed)));
        // Cy lets his go: the 2 come back, and with no claim left it's down
        let ended = release(&db, cy.id, c.id, now).await.unwrap();
        assert_eq!(ended.refunded, 20);
        let after = &ended.response.commission;
        assert_eq!((after.status, after.delivered, after.escrow_strands), (CommissionStatus::Cancelled, 4, 0));
        assert!(matches!(deliver(&db, cy.id, c.id, &hand(cy_rope, None, None), now).await, Err(CourtError::NotOpen)));
        // 100 down and a fee of 6; 40 paid to Bo, 60 back
        assert_eq!(balance(&db, ada.id).await, 1000 - 106 + 60);
        assert_eq!(balance(&db, bo.id).await, 40);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn claims_never_add_up_to_more_than_is_wanted(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let cy = player(&db, "cy", 0).await;
        let c = post(&db, ada.id, &ask("rope", 0, 10, 10, 3), now).await.unwrap();

        let refused = [
            claim(&db, ada.id, c.id, &promise(1), now).await.map(|_| ()),
            claim(&db, bo.id, c.id, &promise(0), now).await.map(|_| ()),
            claim(&db, bo.id, c.id, &promise(11), now).await.map(|_| ()),
            claim(&db, bo.id, Uuid::new_v4(), &promise(1), now).await.map(|_| ()),
            claim(&db, bo.id, c.id, &promise(1), c.due_at).await.map(|_| ()),
        ];
        let kinds: Vec<String> = refused.iter().map(|r| format!("{:?}", r.as_ref().unwrap_err())).collect();
        assert_eq!(kinds, ["OwnCommission", "Claim(Nothing)", "Claim(TooMany(10))", "NotFound", "NotOpen"]);

        claim(&db, bo.id, c.id, &promise(7), now).await.unwrap();
        assert!(matches!(claim(&db, bo.id, c.id, &promise(1), now).await, Err(CourtError::AlreadyClaimed)));
        assert!(matches!(
            claim(&db, cy.id, c.id, &promise(4), now).await,
            Err(CourtError::Claim(ClaimError::TooMany(3)))
        ));
        let cy_claim = claim(&db, cy.id, c.id, &promise(3), now).await.unwrap();
        assert_eq!(cy_claim.response.commission.claimed, 10);

        // A claim never runs past the due date
        let soon = post(&db, ada.id, &ask("rope", 0, 2, 10, 1), now).await.unwrap();
        let late = claim(&db, bo.id, soon.id, &promise(1), now + Duration::hours(20)).await.unwrap();
        assert_eq!(late.response.claim.until, soon.due_at);
        // It lapses as its commission comes due, then the commission expires
        let sweep = claims_due(&db, soon.due_at).await.unwrap();
        assert!(sweep.lapsed.iter().any(|e| e.response.claim.id == late.response.claim.id));
        let expired: Vec<Uuid> = expire_due(&db, soon.due_at).await.unwrap().iter().map(|e| e.commission.id).collect();
        assert_eq!(expired, [soon.id]);
    }

    /// An expiry lapses a claim the sweep hasn't reached
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_expiry_lapses_the_claims_standing(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let c = post(&db, ada.id, &ask("rope", 0, 4, 10, 1), now).await.unwrap();
        claim(&db, bo.id, c.id, &promise(4), now + Duration::hours(12)).await.unwrap();
        let expired = expire_due(&db, c.due_at).await.unwrap();
        let kinds: Vec<&str> = expired[0].notices.iter().map(|n| n.message.type_name()).collect();
        assert_eq!(kinds, ["claim_lapsed", "claim_lapsed", "commission_expired"]);
        assert_eq!((expired[0].refunded, expired[0].commission.claimed), (40, 0));
        let statuses: Vec<ClaimStatus> = claims_on(&db, c.id).await.unwrap().iter().map(|k| k.status).collect();
        assert_eq!(statuses, [ClaimStatus::Lapsed]);
    }

    /// The claimant is told once, as the last hour starts; a claim made with
    /// less than that to run isn't.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claimant_hears_an_hour_ahead_once(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let cy = player(&db, "cy", 0).await;
        let c = post(&db, ada.id, &ask("rope", 0, 10, 10, 1), now).await.unwrap();
        let k = claim(&db, bo.id, c.id, &promise(4), now).await.unwrap().response.claim;
        // Cy's runs to the due date, under an hour away by then
        claim(&db, cy.id, c.id, &promise(2), c.due_at - Duration::minutes(30)).await.unwrap();

        let grace = Duration::seconds(CLAIM_GRACE_SECS);
        assert!(claims_due(&db, k.until - grace - Duration::seconds(1)).await.unwrap().warned.is_empty());
        let sweep = claims_due(&db, k.until - grace).await.unwrap();
        assert_eq!(sweep.warned.len(), 1);
        assert_eq!(sweep.warned[0].player_id, bo.id);
        assert!(matches!(sweep.warned[0].message, ServerMessage::ClaimDueSoon { quantity: 4, delivered: 0, .. }));
        assert!(claims_due(&db, k.until - Duration::minutes(10)).await.unwrap().warned.is_empty());
    }
}
