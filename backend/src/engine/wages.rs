//! Piece-rate wages (commons plan D7, S2.6): work at someone else's station
//! or on their lot, paid unit by unit as it's made. The routes that call
//! these are `routes::wages`.
//!
//! Each operation is one transaction, and every Strand it moves goes through
//! the currency audit log:
//! - [`offer`]: `rate × cap` goes into the offer's escrow (`court_escrow`),
//!   and the court's fee is charged on top (`court_fee`, a sink,
//!   `rules::court::court_fee`).
//! - [`take`]: a shift starts, for one worker at a time on an offer, and one
//!   offer at a time for a worker.
//! - While it runs, the access checks admit the worker to that station's
//!   recipe (`container_access::check_station_use`), or to that resource on
//!   that lot (`container_access::gathering_terms`), and nothing else. A
//!   craft takes its inputs from the station's box and puts its output back
//!   there; a gather goes into the employer's market box. Either way the units
//!   are the employer's, marked as the worker's make (#231), and [`pay`] pays
//!   for them out of the escrow (`court_payout`) in the transaction that
//!   writes them.
//! - [`stop`] ends a shift, by either side, and [`withdraw`] ends the offer;
//!   [`sweep`] ends the rest: at the end of the term, when the station or the
//!   lot goes, and when a worker has been away [`AWAY_SECS`]. What was made is
//!   always paid, and closing refunds what's unspent (`court_refund`).
//!
//! Automation at the station pauses for a shift (`PauseReason::Shift`, the
//! automation tick's `owners_of`) and resumes after it.
//!
//! So for any run of these, the Strands held plus those in escrow plus the
//! fees are what there were at the start, and every unit is somewhere (the
//! conservation test).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use capitol_protocol::rest::{OfferWages, WageOffer, WageShift, WagesQuery};
use capitol_protocol::rules::court::court_fee;
use capitol_protocol::rules::stacking::Units;
use capitol_protocol::rules::wages::{
    offer_value, payable, validate_offer, OfferError, OfferStatus, ShiftEnd, Work, AWAY_SECS, DAY_SECS,
    MAX_OPEN_OFFERS,
};
use capitol_protocol::terrain::quad_contains;
use capitol_protocol::ws::ServerMessage;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::sync::{broadcast, RwLock};
use tracing::Instrument;
use uuid::Uuid;

use super::bazaar::Notice;
use super::currency::{add_strands_in, announce_balances, subtract_strands_in, TransactionType};
use super::events::GameEvent;
use super::{notices, GameState};

/// The most offers `GET /wages` answers with: the newest.
pub const MAX_LISTED: i64 = 200;
/// How often the court looks for shifts and offers to end (the term, a
/// station or lot gone, a worker away).
pub const SWEEP_SECS: u64 = 5;
/// Why a craft or a gather on a shift that has ended writes nothing.
pub const SHIFT_OVER: &str = "Your shift ended before the work was done";

/// A running shift, as the work it admits: what the access checks hand on to
/// the craft or the gather, which pays on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftWork {
    pub offer_id: Uuid,
    pub shift_id: Uuid,
    pub employer_id: Uuid,
    pub worker_id: Uuid,
    pub work: Work,
}

/// Why the court refused.
#[derive(Debug)]
pub enum WageError {
    /// The rate, cap or term is out of bounds
    Rules(OfferError),
    /// No such station
    NoSuchStation,
    /// The station is someone else's
    NotYourStation,
    /// No such recipe
    UnknownRecipe,
    /// The recipe is made at another kind of station
    WrongStation(String),
    /// No such lot
    NoSuchLot,
    /// The lot isn't the employer's
    NotYourLot,
    /// No such resource type
    UnknownResource,
    /// Nothing of the resource grows on the lot
    NothingToGather,
    /// The named worker isn't a player
    NoSuchWorker,
    /// [`MAX_OPEN_OFFERS`] are open already
    TooMany,
    /// Not enough Strands for the escrow and the fee
    Funds(String),
    /// No such offer (of yours, for a withdrawal)
    NotFound,
    /// It's filled, withdrawn, expired or gone, or past its end
    NotOpen,
    /// Offering yourself work, or taking your own
    OwnOffer,
    /// It's named for someone else
    NotForYou,
    /// Someone is working it already
    Taken,
    /// You're working another offer
    OnAShift,
    /// No shift is running on it that's yours to stop
    NoShift,
    Db(sqlx::Error),
}

impl WageError {
    pub fn message(&self) -> String {
        match self {
            WageError::Rules(e) => e.message().to_string(),
            WageError::NoSuchStation => "There's no such station".to_string(),
            WageError::NotYourStation => "You can only offer work at your own station".to_string(),
            WageError::UnknownRecipe => "There's no such recipe".to_string(),
            WageError::WrongStation(needs) => format!("That recipe is made at a {needs}"),
            WageError::NoSuchLot => "There's no such lot".to_string(),
            WageError::NotYourLot => "You can only offer work on your own lot".to_string(),
            WageError::UnknownResource => "There's no such resource".to_string(),
            WageError::NothingToGather => "None of that grows on the lot".to_string(),
            WageError::NoSuchWorker => "There's no such player".to_string(),
            WageError::TooMany => format!("You have {MAX_OPEN_OFFERS} offers of work open already"),
            WageError::Funds(m) => m.clone(),
            WageError::NotFound => "No such offer of work".to_string(),
            WageError::NotOpen => "That offer of work is no longer open".to_string(),
            WageError::OwnOffer => "You can't work for yourself".to_string(),
            WageError::NotForYou => "That offer of work is for someone else".to_string(),
            WageError::Taken => "Someone is working that offer already".to_string(),
            WageError::OnAShift => "You're on a shift already: stop it first".to_string(),
            WageError::NoShift => "No shift of yours is running on that offer".to_string(),
            WageError::Db(e) => format!("Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for WageError {
    fn from(e: sqlx::Error) -> Self {
        WageError::Db(e)
    }
}

/// What a wage action has to tell people, once it's committed: [`announce`]
/// sends it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct News {
    /// To everyone, for the board's Work page
    pub board: Vec<ServerMessage>,
    /// Queued for a player (`notices::queue`), and sent live if they're on
    pub notices: Vec<Notice>,
    /// To one player, live only: a worker's pay as it's made
    pub quiet: Vec<(Uuid, ServerMessage)>,
    /// Balances that changed, as they stand now
    pub balances: Vec<(Uuid, i64)>,
}

impl News {
    fn and(mut self, more: News) -> News {
        self.board.extend(more.board);
        self.notices.extend(more.notices);
        self.quiet.extend(more.quiet);
        self.balances.extend(more.balances);
        self
    }
}

/// Send `news`, once what it reports is committed.
pub fn announce(events: &broadcast::Sender<GameEvent>, news: News) {
    for message in news.board {
        let _ = events.send(GameEvent::Wages { to: None, message });
    }
    for Notice { player_id, notice_id, message } in news.notices {
        let _ = events.send(GameEvent::Notice { player_id, notice_id, message });
    }
    for (player_id, message) in news.quiet {
        let _ = events.send(GameEvent::Wages { to: Some(player_id), message });
    }
    announce_balances(events, news.balances);
}

/// Offer `req` for `employer_id` at `now`: check the work is theirs to offer,
/// take its escrow and the court's fee, and put it on the board. Nothing
/// changes unless all of it does. `grows` is [`grows`]' answer for its work:
/// a gather of something that doesn't grow on the lot is refused.
pub async fn offer(
    db: &PgPool,
    employer_id: Uuid,
    req: &OfferWages,
    grows: bool,
    now: DateTime<Utc>,
) -> Result<(WageOffer, News), WageError> {
    validate_offer(req.rate, req.cap, req.term_days).map_err(WageError::Rules)?;
    if req.worker_id == Some(employer_id) {
        return Err(WageError::OwnOffer);
    }

    let mut tx = db.begin().await?;
    // The employer's row, held so two offers at once can't both be the sixth
    sqlx::query("SELECT 1 FROM players WHERE id = $1 FOR UPDATE")
        .bind(employer_id)
        .fetch_one(&mut *tx)
        .await?;
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wage_offers WHERE employer_id = $1 AND status = 'open'")
        .bind(employer_id)
        .fetch_one(&mut *tx)
        .await?;
    if open >= MAX_OPEN_OFFERS {
        return Err(WageError::TooMany);
    }
    let what = check_work(&mut tx, employer_id, &req.work, grows).await?;
    if let Some(worker) = req.worker_id {
        let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM players WHERE id = $1)")
            .bind(worker)
            .fetch_one(&mut *tx)
            .await?;
        if !known {
            return Err(WageError::NoSuchWorker);
        }
    }

    let id = Uuid::new_v4();
    let value = offer_value(req.rate, req.cap);
    let fee = court_fee(value);
    let what = format!("{} {what} at {} a unit", req.cap, req.rate);
    subtract_strands_in(&mut tx, employer_id, value, TransactionType::CourtEscrow, Some(&format!("Wages: {what}")), Some(id))
        .await
        .map_err(WageError::Funds)?;
    let balance = subtract_strands_in(
        &mut tx,
        employer_id,
        fee,
        TransactionType::CourtFee,
        Some(&format!("Court fee: wages, {what}")),
        Some(id),
    )
    .await
    .map_err(WageError::Funds)?;

    let (station_id, recipe_id, plot_id, resource) = match &req.work {
        Work::Craft { station_id, recipe_id } => (Some(*station_id), Some(recipe_id.as_str()), None, None),
        Work::Gather { plot_id, resource } => (None, None, Some(*plot_id), Some(resource.as_str())),
    };
    sqlx::query(
        "INSERT INTO wage_offers
             (id, employer_id, worker_id, work_kind, station_id, recipe_id, plot_id, resource_type,
              rate, cap, escrow_strands, fee, posted_at, ends_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(id)
    .bind(employer_id)
    .bind(req.worker_id)
    .bind(req.work.kind())
    .bind(station_id)
    .bind(recipe_id)
    .bind(plot_id)
    .bind(resource)
    .bind(req.rate)
    .bind(req.cap)
    .bind(value)
    .bind(fee)
    .bind(now)
    .bind(now + Duration::seconds(req.term_days * DAY_SECS))
    .execute(&mut *tx)
    .await?;

    let mut offered = load(&mut tx, id).await?.ok_or(WageError::NotFound)?;
    // The board hears what's said of a new employer with the offer (S3.1)
    offered.employer_standing = super::standing::words_for(&mut tx, employer_id, now).await?;
    tx.commit().await?;
    let news = News {
        board: vec![ServerMessage::WageOffered { offer: offered.clone() }],
        balances: vec![(employer_id, balance)],
        ..News::default()
    };
    Ok((offered, news))
}

/// Whether `work` can be done where it's offered, for [`offer`]: for a gather,
/// whether a node of its resource stands on the lot, by the lot's outline, as
/// `container_access::gathering_terms` finds a worker's lot. The outline is
/// the terrain's, in game state, not the database's (#435), so the route asks
/// this before the offer's transaction. Work at a station always can.
pub async fn grows(db: &PgPool, game: &RwLock<GameState>, work: &Work) -> Result<bool, sqlx::Error> {
    let Work::Gather { plot_id, resource } = work else {
        return Ok(true);
    };
    let lot: Option<Option<String>> = sqlx::query_scalar("SELECT terrain_id FROM plots WHERE id = $1")
        .bind(plot_id)
        .fetch_optional(db)
        .await?;
    // No such lot, or one with no outline, which nobody can gather on
    let Some(Some(lot)) = lot else {
        return Ok(false);
    };
    Ok(grows_on(&*game.read().await, &lot, resource))
}

/// Whether a node of `resource` stands on the lot `lot_id`: inside its
/// outline, and on no other lot first, as `TerrainConfig::lot_at` finds it.
fn grows_on(game: &GameState, lot_id: &str, resource: &str) -> bool {
    let Some(lot) = game.terrain.terrain_plots.iter().find(|lot| lot.id == lot_id) else {
        return false;
    };
    game.resource_nodes.values().any(|node| {
        node.resource_type == resource
            && quad_contains(&lot.corners, node.x, node.y)
            && game.terrain.lot_at(node.x, node.y).is_some_and(|at| at.id == lot_id)
    })
}

/// Check `work` is `employer_id`'s to offer, and say what it is in a few
/// words. `grows` is [`grows`]' answer for it.
async fn check_work(conn: &mut PgConnection, employer_id: Uuid, work: &Work, grows: bool) -> Result<String, WageError> {
    match work {
        Work::Craft { station_id, recipe_id } => {
            let station: Option<(Uuid, String)> = sqlx::query_as("SELECT owner_id, station_type FROM stations WHERE id = $1")
                .bind(station_id)
                .fetch_optional(&mut *conn)
                .await?;
            let Some((owner, station_type)) = station else {
                return Err(WageError::NoSuchStation);
            };
            if owner != employer_id {
                return Err(WageError::NotYourStation);
            }
            let needs: Option<Option<String>> = sqlx::query_scalar("SELECT required_station_type FROM recipes WHERE id = $1")
                .bind(recipe_id)
                .fetch_optional(&mut *conn)
                .await?;
            match needs {
                None => Err(WageError::UnknownRecipe),
                // A recipe for another station; one by hand may be made at any
                Some(Some(needs)) if needs != station_type => Err(WageError::WrongStation(needs)),
                Some(_) => Ok(format!("{recipe_id} at a {station_type}")),
            }
        }
        Work::Gather { plot_id, resource } => {
            let owner: Option<Option<Uuid>> = sqlx::query_scalar("SELECT owner_id FROM plots WHERE id = $1")
                .bind(plot_id)
                .fetch_optional(&mut *conn)
                .await?;
            match owner {
                None => return Err(WageError::NoSuchLot),
                Some(owner) if owner != Some(employer_id) => return Err(WageError::NotYourLot),
                Some(_) => {}
            }
            let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM resource_types WHERE id = $1)")
                .bind(resource)
                .fetch_one(&mut *conn)
                .await?;
            if !known {
                return Err(WageError::UnknownResource);
            }
            if !grows {
                return Err(WageError::NothingToGather);
            }
            Ok(format!("{resource} gathered"))
        }
    }
}

/// `worker_id` takes the open offer `offer_id` at `now`, and their shift on
/// it starts. The employer hears of it, now or when they're back.
pub async fn take(db: &PgPool, worker_id: Uuid, offer_id: Uuid, now: DateTime<Utc>) -> Result<(WageShift, News), WageError> {
    let mut tx = db.begin().await?;
    // The worker's row first, so two takes of theirs at once can't both start
    sqlx::query("SELECT 1 FROM players WHERE id = $1 FOR UPDATE")
        .bind(worker_id)
        .fetch_one(&mut *tx)
        .await?;
    let row: Option<(Uuid, Option<Uuid>, String, DateTime<Utc>)> =
        sqlx::query_as("SELECT employer_id, worker_id, status, ends_at FROM wage_offers WHERE id = $1 FOR UPDATE")
            .bind(offer_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((employer_id, named, status, ends_at)) = row else {
        return Err(WageError::NotFound);
    };
    if status != OfferStatus::Open.as_str() || ends_at <= now {
        return Err(WageError::NotOpen);
    }
    if employer_id == worker_id {
        return Err(WageError::OwnOffer);
    }
    if named.is_some_and(|named| named != worker_id) {
        return Err(WageError::NotForYou);
    }
    if running_on(&mut tx, offer_id).await?.is_some() {
        return Err(WageError::Taken);
    }
    let busy: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM wage_shifts WHERE worker_id = $1 AND ended_at IS NULL)")
            .bind(worker_id)
            .fetch_one(&mut *tx)
            .await?;
    if busy {
        return Err(WageError::OnAShift);
    }

    let shift_id = Uuid::new_v4();
    sqlx::query("INSERT INTO wage_shifts (id, offer_id, worker_id, started_at) VALUES ($1, $2, $3, $4)")
        .bind(shift_id)
        .bind(offer_id)
        .bind(worker_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    let shift = load_shift(&mut tx, shift_id).await?;
    let message = ServerMessage::WageTaken { offer_id, shift: shift.clone() };
    let notice_id = notices::queue(&mut *tx, employer_id, &offer_id.to_string(), &message).await?;
    let offer = load(&mut tx, offer_id).await?.ok_or(WageError::NotFound)?;
    tx.commit().await?;
    let news = News {
        board: vec![ServerMessage::WageUpdated { offer }],
        notices: vec![Notice { player_id: employer_id, notice_id, message }],
        ..News::default()
    };
    Ok((shift, news))
}

/// The shift running on `offer_id`, if one is: (its id, its worker).
async fn running_on(conn: &mut PgConnection, offer_id: Uuid) -> Result<Option<(Uuid, Uuid)>, sqlx::Error> {
    sqlx::query_as("SELECT id, worker_id FROM wage_shifts WHERE offer_id = $1 AND ended_at IS NULL FOR UPDATE")
        .bind(offer_id)
        .fetch_optional(conn)
        .await
}

/// A shift that ended: as it stands now, and what it was paid.
#[derive(Debug, Clone, PartialEq)]
pub struct Ended {
    pub shift: WageShift,
    pub paid: i64,
}

/// End the shift running on `offer_id` (locked by the caller), if one is,
/// for `reason`. The employer and the worker each get a `wage_shift_ended`,
/// now or when they're back. What was made was paid as it was made.
async fn end_running(
    conn: &mut PgConnection,
    offer_id: Uuid,
    reason: ShiftEnd,
    now: DateTime<Utc>,
) -> Result<Option<(Ended, News)>, sqlx::Error> {
    let Some((shift_id, worker_id)) = running_on(conn, offer_id).await? else {
        return Ok(None);
    };
    sqlx::query("UPDATE wage_shifts SET ended_at = $2, end_reason = $3 WHERE id = $1")
        .bind(shift_id)
        .bind(now)
        .bind(reason.as_str())
        .execute(&mut *conn)
        .await?;
    let (employer_id, rate): (Uuid, i64) = sqlx::query_as("SELECT employer_id, rate FROM wage_offers WHERE id = $1")
        .bind(offer_id)
        .fetch_one(&mut *conn)
        .await?;
    let shift = load_shift(conn, shift_id).await?;
    let paid = rate.saturating_mul(shift.made as i64);
    let message = ServerMessage::WageShiftEnded { offer_id, shift: shift.clone(), paid };
    let mut news = News::default();
    for player_id in [employer_id, worker_id] {
        let notice_id = notices::queue(&mut *conn, player_id, &offer_id.to_string(), &message).await?;
        news.notices.push(Notice { player_id, notice_id, message: message.clone() });
    }
    Ok(Some((Ended { shift, paid }, news)))
}

/// Take the open offer `offer_id` (locked by the caller, its shift ended)
/// down as `status`, refunding what's unspent of its escrow.
async fn close(conn: &mut PgConnection, offer_id: Uuid, status: OfferStatus) -> Result<(WageOffer, News), WageError> {
    let (employer_id, escrow_strands): (Uuid, i64) =
        sqlx::query_as("SELECT employer_id, escrow_strands FROM wage_offers WHERE id = $1")
            .bind(offer_id)
            .fetch_one(&mut *conn)
            .await?;
    let mut news = News::default();
    if escrow_strands > 0 {
        let balance = add_strands_in(
            conn,
            employer_id,
            escrow_strands,
            TransactionType::CourtRefund,
            Some(&format!("Wages {}: the unspent pay back", status.as_str())),
            Some(offer_id),
        )
        .await
        .map_err(WageError::Funds)?;
        news.balances.push((employer_id, balance));
    }
    sqlx::query("UPDATE wage_offers SET status = $2, escrow_strands = 0 WHERE id = $1")
        .bind(offer_id)
        .bind(status.as_str())
        .execute(&mut *conn)
        .await?;
    let offer = load(conn, offer_id).await?.ok_or(WageError::NotFound)?;
    news.board.push(ServerMessage::WageClosed { offer: offer.clone() });
    Ok((offer, news))
}

/// `employer_id` takes their open offer `offer_id` down at `now`: a shift on
/// it ends, having been paid for what it made, and the rest of the escrow
/// comes back. The fee isn't refunded. From anywhere.
pub async fn withdraw(db: &PgPool, employer_id: Uuid, offer_id: Uuid, now: DateTime<Utc>) -> Result<(WageOffer, News), WageError> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, String)> = sqlx::query_as("SELECT employer_id, status FROM wage_offers WHERE id = $1 FOR UPDATE")
        .bind(offer_id)
        .fetch_optional(&mut *tx)
        .await?;
    match row {
        Some((owner, _)) if owner != employer_id => return Err(WageError::NotFound),
        None => return Err(WageError::NotFound),
        Some((_, status)) if status != OfferStatus::Open.as_str() => return Err(WageError::NotOpen),
        Some(_) => {}
    }
    let ended = end_running(&mut tx, offer_id, ShiftEnd::Employer, now).await?;
    let (offer, closed) = close(&mut tx, offer_id, OfferStatus::Withdrawn).await?;
    tx.commit().await?;
    let news = ended.map(|(_, news)| news).unwrap_or_default().and(closed);
    Ok((offer, news))
}

/// End the shift on `offer_id` at `now`, as its worker or the offer's
/// employer. The offer stays open for someone else. From anywhere.
pub async fn stop(db: &PgPool, player_id: Uuid, offer_id: Uuid, now: DateTime<Utc>) -> Result<(Ended, News), WageError> {
    let mut tx = db.begin().await?;
    let employer: Option<Uuid> = sqlx::query_scalar("SELECT employer_id FROM wage_offers WHERE id = $1 FOR UPDATE")
        .bind(offer_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(employer_id) = employer else {
        return Err(WageError::NotFound);
    };
    let Some((_, worker_id)) = running_on(&mut tx, offer_id).await? else {
        return Err(WageError::NoShift);
    };
    let reason = if player_id == worker_id {
        ShiftEnd::Worker
    } else if player_id == employer_id {
        ShiftEnd::Employer
    } else {
        return Err(WageError::NoShift);
    };
    let (ended, news) = end_running(&mut tx, offer_id, reason, now).await?.ok_or(WageError::NoShift)?;
    let offer = load(&mut tx, offer_id).await?.ok_or(WageError::NotFound)?;
    tx.commit().await?;
    let news = news.and(News { board: vec![ServerMessage::WageUpdated { offer }], ..News::default() });
    Ok((ended, news))
}

/// Pay for `units` of `item_type` made on the shift `work`, in the caller's
/// transaction, which is writing them: each at the offer's rate out of its
/// escrow, up to the cap ([`payable`]). At the cap the shift ends and the
/// offer is filled. `Err` with [`SHIFT_OVER`] if the shift isn't running any
/// more: the caller writes nothing, so nothing is made unpaid.
pub async fn pay(
    conn: &mut PgConnection,
    work: &ShiftWork,
    item_type: &str,
    units: &Units,
    now: DateTime<Utc>,
) -> Result<Result<News, &'static str>, WageError> {
    let row: Option<(i64, i32, i32, String, DateTime<Utc>)> =
        sqlx::query_as("SELECT rate, cap, made, status, ends_at FROM wage_offers WHERE id = $1 FOR UPDATE")
            .bind(work.offer_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((rate, cap, made, status, ends_at)) = row else {
        return Ok(Err(SHIFT_OVER));
    };
    let running = running_on(conn, work.offer_id).await?;
    if status != OfferStatus::Open.as_str() || ends_at <= now || running != Some((work.shift_id, work.worker_id)) {
        return Ok(Err(SHIFT_OVER));
    }

    let n = payable(cap, made, units.count() as i32);
    let paid = rate.saturating_mul(n as i64);
    let mut news = News::default();
    if paid > 0 {
        let balance = add_strands_in(
            conn,
            work.worker_id,
            paid,
            TransactionType::CourtPayout,
            Some(&format!("Wages: {n} {item_type} at {rate} a unit")),
            Some(work.offer_id),
        )
        .await
        .map_err(WageError::Funds)?;
        news.balances.push((work.worker_id, balance));
    }
    sqlx::query("UPDATE wage_offers SET made = made + $2, escrow_strands = escrow_strands - $3 WHERE id = $1")
        .bind(work.offer_id)
        .bind(n)
        .bind(paid)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE wage_shifts SET made = made + $2 WHERE id = $1")
        .bind(work.shift_id)
        .bind(n)
        .execute(&mut *conn)
        .await?;
    news.quiet.push((
        work.worker_id,
        ServerMessage::WagePaid {
            offer_id: work.offer_id,
            item_type: item_type.to_string(),
            units: units.clone(),
            paid,
            made: made + n,
            cap,
        },
    ));

    if made + n >= cap {
        if let Some((_, ended)) = end_running(conn, work.offer_id, ShiftEnd::Cap, now).await? {
            news = news.and(ended);
        }
        let (_, closed) = close(conn, work.offer_id, OfferStatus::Filled).await?;
        news = news.and(closed);
    } else {
        let offer = load(conn, work.offer_id).await?.ok_or(WageError::NotFound)?;
        news.board.push(ServerMessage::WageUpdated { offer });
    }
    Ok(Ok(news))
}

/// End what's run its course at `now`, each offer in a transaction of its
/// own:
/// - offers at the end of their term (`expired`), with their shifts (`term`);
/// - offers whose station went or changed hands, or whose lot did (`gone`);
/// - shifts whose worker has been away [`AWAY_SECS`] (`away`): `online` is
///   who's on now, and `away` when each shift's worker was first seen gone,
///   kept from one sweep to the next.
pub async fn sweep(
    db: &PgPool,
    now: DateTime<Utc>,
    online: &HashSet<Uuid>,
    away: &mut HashMap<Uuid, DateTime<Utc>>,
) -> Result<News, WageError> {
    let mut news = News::default();

    let due: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM wage_offers WHERE status = 'open' AND ends_at <= $1 ORDER BY ends_at, id")
            .bind(now)
            .fetch_all(db)
            .await?;
    for id in due {
        news = news.and(close_if(db, id, "ends_at <= $2", now, ShiftEnd::Term, OfferStatus::Expired).await?);
    }

    // The station gone (its id set NULL) or someone else's now, or the lot
    // released or sold
    const GONE: &str = "((work_kind = 'craft'
           AND NOT EXISTS (SELECT 1 FROM stations s WHERE s.id = station_id AND s.owner_id = employer_id))
       OR (work_kind = 'gather'
           AND NOT EXISTS (SELECT 1 FROM plots p WHERE p.id = plot_id AND p.owner_id = employer_id)))";
    let gone: Vec<Uuid> = sqlx::query_scalar(&format!("SELECT id FROM wage_offers WHERE status = 'open' AND {GONE}"))
        .fetch_all(db)
        .await?;
    for id in gone {
        news = news.and(close_if(db, id, GONE, now, ShiftEnd::Gone, OfferStatus::Gone).await?);
    }

    let running: Vec<(Uuid, Uuid, Uuid)> =
        sqlx::query_as("SELECT id, offer_id, worker_id FROM wage_shifts WHERE ended_at IS NULL")
            .fetch_all(db)
            .await?;
    away.retain(|shift, _| running.iter().any(|(id, _, _)| id == shift));
    for (shift_id, offer_id, worker_id) in running {
        if online.contains(&worker_id) {
            away.remove(&shift_id);
            continue;
        }
        let since = *away.entry(shift_id).or_insert(now);
        if now - since < Duration::seconds(AWAY_SECS) {
            continue;
        }
        let mut tx = db.begin().await?;
        news = news.and(end_if_running(&mut tx, offer_id, (shift_id, worker_id), ShiftEnd::Away, now).await?);
        tx.commit().await?;
        away.remove(&shift_id);
    }
    Ok(news)
}

/// End the shift `work`, in the caller's transaction, for `reason`, if it's
/// still running: it may have ended meanwhile. The offer stays open.
pub async fn end_shift(conn: &mut PgConnection, work: &ShiftWork, reason: ShiftEnd, now: DateTime<Utc>) -> Result<News, WageError> {
    end_if_running(conn, work.offer_id, (work.shift_id, work.worker_id), reason, now).await
}

/// [`end_shift`] by ids: the shift `(shift_id, worker_id)` on `offer_id`.
async fn end_if_running(
    conn: &mut PgConnection,
    offer_id: Uuid,
    shift: (Uuid, Uuid),
    reason: ShiftEnd,
    now: DateTime<Utc>,
) -> Result<News, WageError> {
    sqlx::query("SELECT 1 FROM wage_offers WHERE id = $1 FOR UPDATE")
        .bind(offer_id)
        .execute(&mut *conn)
        .await?;
    if running_on(conn, offer_id).await? != Some(shift) {
        return Ok(News::default());
    }
    let Some((_, ended)) = end_running(conn, offer_id, reason, now).await? else {
        return Ok(News::default());
    };
    let offer = load(conn, offer_id).await?.ok_or(WageError::NotFound)?;
    Ok(ended.and(News { board: vec![ServerMessage::WageUpdated { offer }], ..News::default() }))
}

/// Close the offer `id` as `status`, its shift ending for `reason`, if it's
/// still open and `condition` (over `wage_offers`, `$2` being `now`) still holds.
async fn close_if(
    db: &PgPool,
    id: Uuid,
    condition: &str,
    now: DateTime<Utc>,
    reason: ShiftEnd,
    status: OfferStatus,
) -> Result<News, WageError> {
    let mut tx = db.begin().await?;
    let still: Option<Uuid> = sqlx::query_scalar(&format!(
        "SELECT id FROM wage_offers WHERE id = $1 AND status = 'open' AND {condition} AND $2::TIMESTAMPTZ IS NOT NULL
         FOR UPDATE SKIP LOCKED"
    ))
    .bind(id)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    let mut news = News::default();
    if still.is_some() {
        if let Some((_, ended)) = end_running(&mut tx, id, reason, now).await? {
            news = news.and(ended);
        }
        let (_, closed) = close(&mut tx, id, status).await?;
        news = news.and(closed);
    }
    tx.commit().await?;
    Ok(news)
}

/// Run the court's [`sweep`] forever, every [`SWEEP_SECS`].
pub async fn run_wage_loop(game: Arc<RwLock<GameState>>, db: PgPool) {
    let events = game.read().await.event_sender.clone();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(SWEEP_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut away = HashMap::new();
    loop {
        interval.tick().await;
        let online: HashSet<Uuid> = game.read().await.players.keys().copied().collect();
        match sweep(&db, Utc::now(), &online, &mut away).instrument(tracing::info_span!("loop.wages")).await {
            Ok(news) => announce(&events, news),
            Err(e) => tracing::warn!("Wages: sweeping: {}", e.message()),
        }
    }
}

/// The craft `worker_id` may do at `station_id` on a running shift, if
/// they're on one there.
pub async fn shift_at_station(db: &PgPool, worker_id: Uuid, station_id: Uuid) -> Result<Option<ShiftWork>, sqlx::Error> {
    let row: Option<(Uuid, Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT o.id, s.id, o.employer_id, o.recipe_id
         FROM wage_shifts s JOIN wage_offers o ON o.id = s.offer_id
         WHERE s.worker_id = $1 AND s.ended_at IS NULL AND o.status = 'open'
           AND o.work_kind = 'craft' AND o.station_id = $2",
    )
    .bind(worker_id)
    .bind(station_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(offer_id, shift_id, employer_id, recipe_id)| ShiftWork {
        offer_id,
        shift_id,
        employer_id,
        worker_id,
        work: Work::Craft { station_id, recipe_id },
    }))
}

/// The gather `worker_id` may do of `resource` on the lot `terrain_id` on a
/// running shift, if they're on one there.
pub async fn shift_on_lot(
    db: &PgPool,
    worker_id: Uuid,
    terrain_id: &str,
    resource: &str,
) -> Result<Option<ShiftWork>, sqlx::Error> {
    let row: Option<(Uuid, Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT o.id, s.id, o.employer_id, o.plot_id
         FROM wage_shifts s JOIN wage_offers o ON o.id = s.offer_id JOIN plots p ON p.id = o.plot_id
         WHERE s.worker_id = $1 AND s.ended_at IS NULL AND o.status = 'open'
           AND o.work_kind = 'gather' AND p.terrain_id = $2 AND o.resource_type = $3",
    )
    .bind(worker_id)
    .bind(terrain_id)
    .bind(resource)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(offer_id, shift_id, employer_id, plot_id)| ShiftWork {
        offer_id,
        shift_id,
        employer_id,
        worker_id,
        work: Work::Gather { plot_id, resource: resource.to_string() },
    }))
}

/// Why `worker_id`, on the shift `work` at a station, can't make
/// `recipe_id` there: a shift is for its offer's recipe alone.
pub fn recipe_refusal(work: &ShiftWork, recipe_id: &str) -> Option<String> {
    match &work.work {
        Work::Craft { recipe_id: offered, .. } if offered != recipe_id => {
            Some(format!("Your shift here is for {offered}, nothing else"))
        }
        _ => None,
    }
}

/// An offer as [`COLUMNS`] selects it, over `wage_offers o` and its people
#[derive(sqlx::FromRow)]
struct OfferRow {
    id: Uuid,
    employer_id: Uuid,
    employer_name: String,
    employer_citizen: bool,
    worker_id: Option<Uuid>,
    worker_name: Option<String>,
    work_kind: String,
    station_id: Option<Uuid>,
    recipe_id: Option<String>,
    plot_id: Option<Uuid>,
    resource_type: Option<String>,
    x: f64,
    y: f64,
    rate: i64,
    cap: i32,
    made: i32,
    escrow_strands: i64,
    fee: i64,
    posted_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    status: String,
}

/// The columns of an [`OfferRow`]. The work's place is its station's, or
/// its lot's middle.
const COLUMNS: &str = "o.id, o.employer_id, ep.name AS employer_name, ea.kind = 'citizen' AS employer_citizen,
     o.worker_id, wp.name AS worker_name, o.work_kind, o.station_id, o.recipe_id, o.plot_id, o.resource_type,
     COALESCE(s.position_x, pl.world_x, 0)::FLOAT8 AS x, COALESCE(s.position_y, pl.world_y, 0)::FLOAT8 AS y,
     o.rate, o.cap, o.made, o.escrow_strands, o.fee, o.posted_at, o.ends_at, o.status
     FROM wage_offers o
     JOIN players ep ON ep.id = o.employer_id JOIN accounts ea ON ea.id = ep.account_id
     LEFT JOIN players wp ON wp.id = o.worker_id
     LEFT JOIN stations s ON s.id = o.station_id
     LEFT JOIN plots pl ON pl.id = o.plot_id";

fn from_row(row: OfferRow, shift: Option<WageShift>) -> Result<WageOffer, sqlx::Error> {
    let decode = |what: &str| sqlx::Error::Decode(format!("wage_offers.{what} is missing or unknown").into());
    let work = match row.work_kind.as_str() {
        // A station taken down leaves no id: the offer is gone, or soon will be
        "craft" => Work::Craft {
            station_id: row.station_id.unwrap_or_default(),
            recipe_id: row.recipe_id.ok_or_else(|| decode("recipe_id"))?,
        },
        "gather" => Work::Gather {
            plot_id: row.plot_id.unwrap_or_default(),
            resource: row.resource_type.ok_or_else(|| decode("resource_type"))?,
        },
        _ => return Err(decode("work_kind")),
    };
    Ok(WageOffer {
        id: row.id,
        employer_id: row.employer_id,
        employer_name: row.employer_name,
        employer_citizen: row.employer_citizen,
        worker_id: row.worker_id,
        worker_name: row.worker_name,
        work,
        x: row.x,
        y: row.y,
        rate: row.rate,
        cap: row.cap,
        made: row.made,
        escrow_strands: row.escrow_strands,
        fee: row.fee,
        posted_at: row.posted_at,
        ends_at: row.ends_at,
        status: OfferStatus::parse(&row.status).ok_or_else(|| decode("status"))?,
        shift,
        employer_standing: String::new(),
    })
}

/// A shift as [`SHIFT_COLUMNS`] selects it
#[derive(sqlx::FromRow)]
struct ShiftRow {
    id: Uuid,
    offer_id: Uuid,
    worker_id: Uuid,
    worker_name: String,
    worker_citizen: bool,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    made: i32,
    end_reason: Option<String>,
}

const SHIFT_COLUMNS: &str = "s.id, s.offer_id, s.worker_id, p.name AS worker_name, a.kind = 'citizen' AS worker_citizen,
     s.started_at, s.ended_at, s.made, s.end_reason
     FROM wage_shifts s JOIN players p ON p.id = s.worker_id JOIN accounts a ON a.id = p.account_id";

impl From<ShiftRow> for WageShift {
    fn from(row: ShiftRow) -> WageShift {
        WageShift {
            id: row.id,
            offer_id: row.offer_id,
            worker_id: row.worker_id,
            worker_name: row.worker_name,
            worker_citizen: row.worker_citizen,
            started_at: row.started_at,
            ended_at: row.ended_at,
            made: row.made,
            end: row.end_reason.as_deref().and_then(ShiftEnd::parse),
        }
    }
}

async fn load_shift(conn: &mut PgConnection, shift_id: Uuid) -> Result<WageShift, sqlx::Error> {
    let row: ShiftRow = sqlx::query_as(&format!("SELECT {SHIFT_COLUMNS} WHERE s.id = $1"))
        .bind(shift_id)
        .fetch_one(conn)
        .await?;
    Ok(row.into())
}

/// The shifts running on `offer_ids`, by offer.
async fn running_shifts(conn: &mut PgConnection, offer_ids: &[Uuid]) -> Result<HashMap<Uuid, WageShift>, sqlx::Error> {
    let rows: Vec<ShiftRow> =
        sqlx::query_as(&format!("SELECT {SHIFT_COLUMNS} WHERE s.offer_id = ANY($1) AND s.ended_at IS NULL"))
            .bind(offer_ids)
            .fetch_all(conn)
            .await?;
    Ok(rows.into_iter().map(|row| (row.offer_id, row.into())).collect())
}

/// The offer `offer_id`, with the shift running on it.
pub async fn load(conn: &mut PgConnection, offer_id: Uuid) -> Result<Option<WageOffer>, sqlx::Error> {
    let row: Option<OfferRow> = sqlx::query_as(&format!("SELECT {COLUMNS} WHERE o.id = $1"))
        .bind(offer_id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let mut shifts = running_shifts(conn, &[offer_id]).await?;
    from_row(row, shifts.remove(&offer_id)).map(Some)
}

/// The board as `query` asks for it, for `player_id`: newest first, at most
/// [`MAX_LISTED`]. `mine` is the offers they made or are named on.
pub async fn list(db: &PgPool, player_id: Uuid, query: &WagesQuery) -> Result<Vec<WageOffer>, sqlx::Error> {
    let status = query.status.unwrap_or(OfferStatus::Open);
    let rows: Vec<OfferRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS}
         WHERE o.status = $1 AND (NOT $2 OR o.employer_id = $3 OR o.worker_id = $3)
         ORDER BY o.posted_at DESC, o.id
         LIMIT $4"
    ))
    .bind(status.as_str())
    .bind(query.mine)
    .bind(player_id)
    .bind(MAX_LISTED)
    .fetch_all(db)
    .await?;
    let mut conn = db.acquire().await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let mut shifts = running_shifts(&mut conn, &ids).await?;
    let mut offers: Vec<WageOffer> = rows.into_iter().map(|row| {
        let shift = shifts.remove(&row.id);
        from_row(row, shift)
    }).collect::<Result<_, _>>()?;
    // What's said of each employer, for the Work page's cards (S3.1)
    let employers: Vec<Uuid> = offers.iter().map(|o| o.employer_id).collect();
    let words = super::standing::words_of(&mut conn, &employers, Utc::now()).await?;
    for offer in &mut offers {
        offer.employer_standing = words.get(&offer.employer_id).cloned().unwrap_or_default();
    }
    Ok(offers)
}

/// The shift `worker_id` is working, if they are.
pub async fn shift_of(db: &PgPool, worker_id: Uuid) -> Result<Option<WageShift>, sqlx::Error> {
    let row: Option<ShiftRow> = sqlx::query_as(&format!("SELECT {SHIFT_COLUMNS} WHERE s.worker_id = $1 AND s.ended_at IS NULL"))
        .bind(worker_id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(Into::into))
}

/// Strands the open offers hold: their unspent escrow, for the ledger's
/// `court_held`.
pub(crate) async fn held(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(SUM(escrow_strands), 0)::BIGINT FROM wage_offers")
        .fetch_one(conn)
        .await
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test wage -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::conservation::{assert_audited, balances_of, every_unit, strands_accounted, units_of_type};
    use crate::engine::container_access::{check_container_access, check_station_use, AccessError, StationUse};
    use crate::engine::craft_completion::{finish_craft, CraftOutput, FinishedCraft};
    use crate::engine::extraction::PendingItem;
    use crate::engine::market::tests::{balance, box_of, player, Player};
    use crate::engine::units::test_units::{all_in, marked, stack_of};
    use crate::engine::units::Marked;
    use capitol_protocol::rules::stacking::Pick;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    const HERE: Option<(f64, f64)> = Some((0.0, 0.0));
    /// The lot the gathering tests work on
    const LOT: &str = "wage_lot";

    async fn content(db: &PgPool) {
        crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
    }

    /// [`super::offer`] of work that grows where it's offered, as the route
    /// finds it for these tests' lot (`grows_on` has its own test)
    async fn offer(db: &PgPool, employer_id: Uuid, req: &OfferWages, now: DateTime<Utc>) -> Result<(WageOffer, News), WageError> {
        super::offer(db, employer_id, req, true, now).await
    }

    /// A lot that isn't a rectangle: a node inside its bounding box but over
    /// its edge doesn't grow on it, on a neighbour's ground or on none (#435)
    #[test]
    fn what_grows_on_a_lot_is_inside_its_outline() {
        use crate::models::{NodeState, ResourceNodeState, TerrainConfig, TerrainPlot, WorldGenParams};
        let plot = |id: &str, corners| TerrainPlot {
            id: id.into(),
            corners,
            street_facing_edge: (0, 1),
            area: TerrainPlot::calculate_area(&corners),
            adjacent_street_id: String::new(),
            zone: "suburban".into(),
        };
        // A diamond round the origin, its bounding box ±10, and a neighbour
        // across its north-east edge, in that corner of the box
        let diamond = plot(LOT, [(0.0, -10.0), (10.0, 0.0), (0.0, 10.0), (-10.0, 0.0)]);
        let neighbour = plot("next_door", [(5.0, 5.0), (10.0, 0.0), (20.0, 10.0), (10.0, 20.0)]);
        let terrain = TerrainConfig {
            world_size: capitol_protocol::rules::WORLD_SIZE,
            center_x: capitol_protocol::rules::CENTER_X,
            center_y: capitol_protocol::rules::CENTER_Y,
            water_rings: Vec::new(),
            roads: Vec::new(),
            terrain_plots: vec![neighbour, diamond],
            parks: Vec::new(),
            landmarks: Vec::new(),
            land: Default::default(),
            rivers: Vec::new(),
            streams: Vec::new(),
            lakes: Vec::new(),
            fords: Vec::new(),
        };
        let with = |nodes: &[(&str, f64, f64)]| {
            let mut game = GameState::with_terrain(WorldGenParams::default(), terrain.clone());
            for &(resource_type, x, y) in nodes {
                let id = Uuid::new_v4();
                game.resource_nodes.insert(id, ResourceNodeState {
                    id,
                    resource_type: resource_type.into(),
                    x,
                    y,
                    base_quality: 50,
                    state: NodeState::Available,
                    harvested_by: None,
                    regenerates_at_tick: None,
                });
            }
            game
        };

        assert!(grows_on(&with(&[("grass", 2.0, 3.0)]), LOT, "grass"), "inside the diamond");
        // In the box's corners, off the diamond: on the neighbour's lot, and on none
        assert!(!grows_on(&with(&[("clay", 9.0, 9.0)]), LOT, "clay"), "next door");
        assert!(!grows_on(&with(&[("clay", -9.0, -9.0)]), LOT, "clay"), "on no lot");
        // On the corner they share, a node is the lot's that `lot_at` finds first
        assert!(!grows_on(&with(&[("clay", 10.0, 0.0)]), LOT, "clay"), "the neighbour's, as a worker finds it");
        assert!(grows_on(&with(&[("clay", 9.0, 9.0), ("clay", -1.0, 0.0)]), LOT, "clay"), "one on it is enough");
        assert!(!grows_on(&with(&[("grass", 2.0, 3.0)]), LOT, "clay"), "another resource");
        assert!(!grows_on(&with(&[("grass", 2.0, 3.0)]), "no_such_lot", "grass"));
    }

    /// A workbench of `owner`'s at `x`, quality 50: (station, its box)
    async fn bench(db: &PgPool, owner: Uuid, x: f64) -> (Uuid, Uuid) {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
             VALUES ($1, 'workbench', $2, $3, 0, 50)",
        )
        .bind(id)
        .bind(owner)
        .bind(x as f32)
        .execute(db)
        .await
        .unwrap();
        let container = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2)")
            .bind(container)
            .bind(id)
            .execute(db)
            .await
            .unwrap();
        (id, container)
    }

    /// Plenty to make tool handles from, `owner`'s, in `the_box`
    async fn load_handles(db: &PgPool, owner: Uuid, the_box: Uuid) {
        stack_of(db, owner, the_box, 0, "sticks", &Units::uniform(40, 70)).await;
        stack_of(db, owner, the_box, 1, "rope", &Units::uniform(20, 50)).await;
    }

    /// `owner`'s lot [`LOT`], round the origin, with grass on it
    async fn lot(db: &PgPool, owner: Uuid) -> Uuid {
        let plot: Uuid = sqlx::query_scalar(
            "INSERT INTO plots (zone_id, grid_x, grid_y, world_x, world_y, bounds_min_x,
                                bounds_min_y, bounds_max_x, bounds_max_y, size_category, terrain_id, owner_id)
             VALUES ((SELECT id FROM zones LIMIT 1), 9997, 9997, 0, 0, -10, -10, 10, 10, 'small', $1, $2)
             RETURNING id",
        )
        .bind(LOT)
        .bind(owner)
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO resource_nodes (id, resource_type, position_x, position_y, base_quality)
             VALUES ($1, 'grass', 2, 3, 50)",
        )
        .bind(Uuid::new_v4())
        .execute(db)
        .await
        .unwrap();
        plot
    }

    fn handles_at(station_id: Uuid, rate: i64, cap: i32) -> OfferWages {
        OfferWages {
            work: Work::Craft { station_id, recipe_id: "tool_handle".into() },
            rate,
            cap,
            term_days: 1,
            worker_id: None,
        }
    }

    fn grass_on(plot_id: Uuid, rate: i64, cap: i32) -> OfferWages {
        OfferWages { work: Work::Gather { plot_id, resource: "grass".into() }, rate, cap, term_days: 1, worker_id: None }
    }

    /// `n` tool handles made by `worker` from what's in `the_box`, as the tick
    /// would finish the craft: two sticks and a rope, the best first
    async fn handle_craft(db: &PgPool, worker: Uuid, the_box: Uuid, n: i32, wage: Option<ShiftWork>) -> FinishedCraft {
        let mut inputs = Vec::new();
        for (item_type, quantity) in [("sticks", 2), ("rope", 1)] {
            let stack: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM items WHERE container_id = $1 AND item_type = $2 ORDER BY slot_index LIMIT 1",
            )
            .bind(the_box)
            .bind(item_type)
            .fetch_optional(db)
            .await
            .unwrap();
            let Some(stack) = stack else { continue };
            let held = crate::engine::units::test_units::units(db, stack).await;
            if let Some(units) = held.pick(quantity, Pick::Best) {
                inputs.push((stack, units));
            }
        }
        FinishedCraft {
            player_id: worker,
            operation_id: Uuid::new_v4(),
            recipe_id: "tool_handle".into(),
            inputs,
            outputs: vec![CraftOutput {
                item_type: "tool_handle".into(),
                item_name: "Tool Handle".into(),
                quantity: n,
                quality: 60,
                stackable: true,
                durability: None,
            }],
            tool: None,
            station_id: None,
            wage,
        }
    }

    fn gathered(worker: Uuid, item_type: &str, quantity: i32, wage: Option<ShiftWork>) -> PendingItem {
        PendingItem {
            player_id: worker,
            node_id: Uuid::new_v4(),
            item_type: item_type.into(),
            item_name: item_type.into(),
            quantity,
            quality: 55,
            tool: None,
            wage,
        }
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

    async fn offer_of(db: &PgPool, id: Uuid) -> WageOffer {
        load(&mut db.acquire().await.unwrap(), id).await.unwrap().unwrap()
    }

    /// The `wage_shift_ended` in `news` for `player_id`: the shift and its pay
    fn ended_for(news: &News, player_id: Uuid) -> Option<(WageShift, i64)> {
        news.notices.iter().find(|n| n.player_id == player_id).and_then(|n| match &n.message {
            ServerMessage::WageShiftEnded { shift, paid, .. } => Some((shift.clone(), *paid)),
            _ => None,
        })
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_offer_escrows_its_pay_and_the_fee_and_refuses_what_isnt_yours(db: PgPool) {
        content(&db).await;
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 3).await;
        let (station, _) = bench(&db, ada.id, 0.0).await;
        let (bos_bench, _) = bench(&db, bo.id, 0.0).await;
        let plot = lot(&db, ada.id).await;

        // 10 handles at 4: 40 down and a fee of 5 + 1
        let (offered, news) = offer(&db, ada.id, &handles_at(station, 4, 10), now).await.unwrap();
        assert_eq!((offered.escrow_strands, offered.fee, offered.made, offered.status), (40, 6, 0, OfferStatus::Open));
        assert_eq!(offered.ends_at - offered.posted_at, Duration::days(1));
        assert_eq!(offered.shift, None);
        assert_eq!(balance(&db, ada.id).await, 1000 - 46);
        assert_eq!(audit_of(&db, ada.id).await, vec![("court_escrow".into(), -40), ("court_fee".into(), -6)]);
        assert!(matches!(&news.board[..], [ServerMessage::WageOffered { .. }]));
        assert_eq!(news.balances, vec![(ada.id, 954)]);
        // The ledger counts the offer's escrow as hers (#553)
        let books = crate::engine::ledger::books(&db).await.unwrap();
        let hers = books.accounts.iter().find(|a| a.player_id == ada.id).unwrap();
        assert_eq!((hers.escrow.wages, hers.escrow.bids, hers.holding()), (40, 0, 994));

        // Grass on her lot, placed at the lot's middle
        let (field, _) = offer(&db, ada.id, &grass_on(plot, 2, 5), now).await.unwrap();
        assert_eq!((field.x, field.y), (0.0, 0.0));

        let kiln_recipe: String = sqlx::query_scalar("SELECT id FROM recipes WHERE required_station_type = 'kiln' LIMIT 1")
            .fetch_one(&db)
            .await
            .unwrap();
        let wrong = OfferWages { work: Work::Craft { station_id: station, recipe_id: kiln_recipe }, ..handles_at(station, 4, 10) };
        assert_eq!(offer(&db, ada.id, &wrong, now).await.unwrap_err().message(), "That recipe is made at a kiln");
        let unknown = OfferWages { work: Work::Craft { station_id: station, recipe_id: "nope".into() }, ..handles_at(station, 4, 10) };
        assert!(matches!(offer(&db, ada.id, &unknown, now).await, Err(WageError::UnknownRecipe)));
        assert!(matches!(offer(&db, bo.id, &handles_at(station, 1, 1), now).await, Err(WageError::NotYourStation)));
        assert!(matches!(offer(&db, bo.id, &grass_on(plot, 1, 1), now).await, Err(WageError::NotYourLot)));
        let clay = OfferWages { work: Work::Gather { plot_id: plot, resource: "clay_bank".into() }, ..grass_on(plot, 1, 1) };
        assert!(matches!(super::offer(&db, ada.id, &clay, false, now).await, Err(WageError::NothingToGather)));
        let mud = OfferWages { work: Work::Gather { plot_id: plot, resource: "mud".into() }, ..grass_on(plot, 1, 1) };
        assert!(matches!(offer(&db, ada.id, &mud, now).await, Err(WageError::UnknownResource)));
        assert!(matches!(offer(&db, ada.id, &handles_at(station, 0, 1), now).await, Err(WageError::Rules(_))));
        let long = OfferWages { term_days: 8, ..handles_at(station, 1, 1) };
        assert!(matches!(offer(&db, ada.id, &long, now).await, Err(WageError::Rules(OfferError::BadTerm))));
        let herself = OfferWages { worker_id: Some(ada.id), ..handles_at(station, 1, 1) };
        assert!(matches!(offer(&db, ada.id, &herself, now).await, Err(WageError::OwnOffer)));
        // Bo has 3 Strands: not the 5 and the fee of a 5 at 1
        assert!(matches!(offer(&db, bo.id, &handles_at(bos_bench, 1, 5), now).await, Err(WageError::Funds(_))));
        assert_eq!(balance(&db, bo.id).await, 3, "a refusal takes nothing");

        // Five open at once
        for _ in 0..3 {
            offer(&db, ada.id, &handles_at(station, 1, 1), now).await.unwrap();
        }
        assert!(matches!(offer(&db, ada.id, &handles_at(station, 1, 1), now).await, Err(WageError::TooMany)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_worker_may_use_only_that_station_and_recipe_and_only_during_the_shift(db: PgPool) {
        content(&db).await;
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let cy = player(&db, "cy", 0).await;
        let (station, the_box) = bench(&db, ada.id, 0.0).await;
        let (other, _) = bench(&db, ada.id, 5.0).await;

        // Before the shift, nothing
        assert!(matches!(check_station_use(&db, bo.id, station, HERE).await, Err(AccessError::Forbidden)));
        let (offered, _) = offer(&db, ada.id, &handles_at(station, 4, 10), now).await.unwrap();
        let (shift, news) = take(&db, bo.id, offered.id, now).await.unwrap();
        assert_eq!((shift.worker_id, shift.made, shift.ended_at), (bo.id, 0, None));
        // Ada hears, now or when she's back, and the board sees it taken
        let [Notice { player_id, message: ServerMessage::WageTaken { .. }, .. }] = &news.notices[..] else {
            panic!("Ada's notice: {:?}", news.notices);
        };
        assert_eq!(*player_id, ada.id);
        assert!(matches!(&news.board[..], [ServerMessage::WageUpdated { offer }] if offer.shift.is_some()));

        // During it: that station, within range, to open and craft its recipe
        let StationUse::Shift(work) = check_station_use(&db, bo.id, station, HERE).await.unwrap() else {
            panic!("on a shift there");
        };
        assert_eq!((work.offer_id, work.employer_id, work.worker_id), (offered.id, ada.id, bo.id));
        assert!(matches!(check_station_use(&db, bo.id, station, Some((500.0, 0.0))).await, Err(AccessError::TooFar)));
        assert_eq!(recipe_refusal(&work, "tool_handle"), None);
        assert!(recipe_refusal(&work, "rope_basic").is_some(), "nothing else");
        // Not another station of hers, nor her box's contents by hand
        assert!(matches!(check_station_use(&db, bo.id, other, HERE).await, Err(AccessError::Forbidden)));
        assert!(matches!(check_container_access(&db, bo.id, the_box, HERE).await, Err(AccessError::Forbidden)));
        // Ada still uses it as hers
        assert_eq!(check_station_use(&db, ada.id, station, HERE).await.unwrap(), StationUse::Free { owner: ada.id });

        // One worker at a time, one shift at a time, and never your own
        assert!(matches!(take(&db, cy.id, offered.id, now).await, Err(WageError::Taken)));
        let (second, _) = offer(&db, ada.id, &handles_at(other, 4, 10), now).await.unwrap();
        assert!(matches!(take(&db, bo.id, second.id, now).await, Err(WageError::OnAShift)));
        assert!(matches!(take(&db, ada.id, second.id, now).await, Err(WageError::OwnOffer)));
        let for_cy = OfferWages { worker_id: Some(cy.id), ..handles_at(other, 4, 10) };
        let (named, _) = offer(&db, ada.id, &for_cy, now).await.unwrap();
        let dee = player(&db, "dee", 0).await;
        assert!(matches!(take(&db, dee.id, named.id, now).await, Err(WageError::NotForYou)));
        take(&db, cy.id, named.id, now).await.unwrap();

        // Stopped, it's over: nothing again
        assert!(matches!(stop(&db, dee.id, offered.id, now).await, Err(WageError::NoShift)));
        let (ended, news) = stop(&db, bo.id, offered.id, now).await.unwrap();
        assert_eq!((ended.shift.end, ended.paid), (Some(ShiftEnd::Worker), 0));
        assert!(ended_for(&news, ada.id).is_some() && ended_for(&news, bo.id).is_some(), "both hear");
        assert!(matches!(check_station_use(&db, bo.id, station, HERE).await, Err(AccessError::Forbidden)));
        assert_eq!(offer_of(&db, offered.id).await.status, OfferStatus::Open, "open for someone else");
        assert!(matches!(stop(&db, bo.id, offered.id, now).await, Err(WageError::NoShift)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_shift_craft_goes_to_the_employer_and_is_paid_in_the_transaction_that_made_it(db: PgPool) {
        content(&db).await;
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let (station, the_box) = bench(&db, ada.id, 0.0).await;
        load_handles(&db, ada.id, the_box).await;
        let (offered, _) = offer(&db, ada.id, &handles_at(station, 4, 10), now).await.unwrap();
        take(&db, bo.id, offered.id, now).await.unwrap();
        let work = shift_at_station(&db, bo.id, station).await.unwrap().unwrap();

        // From her box, into her box, as hers, made by Bo
        let done = finish_craft(&db, &handle_craft(&db, bo.id, the_box, 1, Some(work.clone())).await).await.unwrap();
        assert_eq!(done.changed, vec![the_box]);
        assert_eq!(all_in(&db, the_box, "sticks").await, Units::uniform(38, 70));
        assert_eq!(all_in(&db, the_box, "rope").await, Units::uniform(19, 50));
        let made: (Uuid, Uuid) =
            sqlx::query_as("SELECT id, owner_id FROM items WHERE container_id = $1 AND item_type = 'tool_handle'")
                .bind(the_box)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(made.1, ada.id);
        assert_eq!(marked(&db, made.0).await, Marked::by(Some(bo.id), &Units::uniform(1, 60)));
        // Paid there and then
        assert_eq!(balance(&db, bo.id).await, 4);
        assert_eq!(audit_of(&db, bo.id).await, vec![("court_payout".into(), 4)]);
        let after = offer_of(&db, offered.id).await;
        assert_eq!((after.made, after.escrow_strands, after.shift.map(|s| s.made)), (1, 36, Some(1)));
        let news = done.wage.unwrap();
        assert!(matches!(&news.quiet[..], [(to, ServerMessage::WagePaid { paid: 4, made: 1, cap: 10, .. })] if *to == bo.id));
        assert_eq!(news.balances, vec![(bo.id, 4)]);

        // A craft whose shift ended while it ran writes nothing at all
        stop(&db, ada.id, offered.id, now).await.unwrap();
        let late = handle_craft(&db, bo.id, the_box, 1, Some(work)).await;
        assert_eq!(finish_craft(&db, &late).await, Err(SHIFT_OVER.to_string()));
        assert_eq!(all_in(&db, the_box, "sticks").await, Units::uniform(38, 70), "the inputs stay");
        assert_eq!(all_in(&db, the_box, "tool_handle").await.count(), 1);
        assert_eq!(balance(&db, bo.id).await, 4);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_employer_withdrawing_mid_shift_pays_what_was_made_and_the_cap_fills_it(db: PgPool) {
        content(&db).await;
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let (station, the_box) = bench(&db, ada.id, 0.0).await;
        load_handles(&db, ada.id, the_box).await;

        // Two made of ten at 4, then withdrawn: 8 paid, 32 back, the fee kept
        let (offered, _) = offer(&db, ada.id, &handles_at(station, 4, 10), now).await.unwrap();
        take(&db, bo.id, offered.id, now).await.unwrap();
        let work = shift_at_station(&db, bo.id, station).await.unwrap().unwrap();
        for _ in 0..2 {
            finish_craft(&db, &handle_craft(&db, bo.id, the_box, 1, Some(work.clone())).await).await.unwrap();
        }
        let (withdrawn, news) = withdraw(&db, ada.id, offered.id, now).await.unwrap();
        assert_eq!((withdrawn.status, withdrawn.escrow_strands, withdrawn.made), (OfferStatus::Withdrawn, 0, 2));
        assert_eq!(balance(&db, bo.id).await, 8);
        assert_eq!(balance(&db, ada.id).await, 1000 - 46 + 32);
        let (shift, paid) = ended_for(&news, bo.id).expect("Bo hears");
        assert_eq!((shift.made, shift.end, paid), (2, Some(ShiftEnd::Employer), 8));
        assert!(ended_for(&news, ada.id).is_some());
        assert!(matches!(news.board.last(), Some(ServerMessage::WageClosed { .. })));
        assert!(matches!(withdraw(&db, ada.id, offered.id, now).await, Err(WageError::NotOpen)));
        assert!(matches!(take(&db, bo.id, offered.id, now).await, Err(WageError::NotOpen)));

        // Two of two: the second craft makes 2, so one is paid and the offer fills
        let (capped, _) = offer(&db, ada.id, &handles_at(station, 5, 2), now).await.unwrap();
        take(&db, bo.id, capped.id, now).await.unwrap();
        let work = shift_at_station(&db, bo.id, station).await.unwrap().unwrap();
        finish_craft(&db, &handle_craft(&db, bo.id, the_box, 1, Some(work.clone())).await).await.unwrap();
        let done = finish_craft(&db, &handle_craft(&db, bo.id, the_box, 2, Some(work.clone())).await).await.unwrap();
        assert_eq!(balance(&db, bo.id).await, 8 + 5 + 5, "the unit over the cap is unpaid");
        let filled = offer_of(&db, capped.id).await;
        assert_eq!((filled.status, filled.made, filled.escrow_strands, filled.shift), (OfferStatus::Filled, 2, 0, None));
        let news = done.wage.unwrap();
        assert_eq!(ended_for(&news, bo.id).map(|(s, paid)| (s.end, paid)), Some((Some(ShiftEnd::Cap), 10)));
        assert_eq!(all_in(&db, the_box, "tool_handle").await.count(), 2 + 3, "every unit is hers");
        let over = handle_craft(&db, bo.id, the_box, 1, Some(work)).await;
        assert_eq!(finish_craft(&db, &over).await, Err(SHIFT_OVER.to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_gather_on_a_shift_goes_to_the_employers_market_box_paid(db: PgPool) {
        content(&db).await;
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let plot = lot(&db, ada.id).await;
        let (events, _rx) = broadcast::channel(64);
        let (offered, _) = offer(&db, ada.id, &grass_on(plot, 2, 10), now).await.unwrap();
        take(&db, bo.id, offered.id, now).await.unwrap();
        assert_eq!(shift_on_lot(&db, bo.id, LOT, "clay").await.unwrap(), None, "that resource alone");
        assert_eq!(shift_on_lot(&db, bo.id, "elsewhere", "grass").await.unwrap(), None, "that lot alone");
        let work = shift_on_lot(&db, bo.id, LOT, "grass").await.unwrap().unwrap();

        crate::engine::tick::persist_gather(&db, gathered(bo.id, "fiber", 3, Some(work.clone())), &events).await;
        let her_box = box_of(&db, ada.id).await;
        assert_eq!(all_in(&db, her_box, "fiber").await, Units::uniform(3, 55));
        let stack: (Uuid, Uuid) = sqlx::query_as("SELECT id, owner_id FROM items WHERE container_id = $1")
            .bind(her_box)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(stack.1, ada.id);
        assert_eq!(marked(&db, stack.0).await, Marked::by(Some(bo.id), &Units::uniform(3, 55)));
        assert_eq!(all_in(&db, bo.inventory, "fiber").await.count(), 0);
        assert_eq!(balance(&db, bo.id).await, 6);
        assert_eq!(offer_of(&db, offered.id).await.made, 3);

        // Her box full: Bo keeps what he gathered, unpaid, and the shift ends
        for slot in 1..48 {
            stack_of(&db, ada.id, her_box, slot, "rope", &Units::uniform(1, 50)).await;
        }
        crate::engine::tick::persist_gather(&db, gathered(bo.id, "sticks", 2, Some(work.clone())), &events).await;
        assert_eq!(all_in(&db, bo.inventory, "sticks").await, Units::uniform(2, 55));
        assert_eq!(balance(&db, bo.id).await, 6);
        let after = offer_of(&db, offered.id).await;
        assert_eq!((after.made, after.status, after.shift), (3, OfferStatus::Open, None));
        let end: String = sqlx::query_scalar("SELECT end_reason FROM wage_shifts WHERE offer_id = $1")
            .bind(offered.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(end, "box_full");

        // A gather after the shift ended writes nothing
        crate::engine::tick::persist_gather(&db, gathered(bo.id, "fiber", 4, Some(work)), &events).await;
        assert_eq!(all_in(&db, bo.inventory, "fiber").await.count(), 0);
        assert_eq!(all_in(&db, her_box, "fiber").await.count(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_disconnected_workers_shift_ends_after_five_minutes(db: PgPool) {
        content(&db).await;
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let (station, _) = bench(&db, ada.id, 0.0).await;
        let (offered, _) = offer(&db, ada.id, &handles_at(station, 4, 10), now).await.unwrap();
        take(&db, bo.id, offered.id, now).await.unwrap();
        let (on, off) = (HashSet::from([bo.id]), HashSet::new());
        let mut away = HashMap::new();
        let at = |secs: i64| now + Duration::seconds(secs);

        // On, then away four minutes, then back: the clock starts again
        assert_eq!(sweep(&db, at(0), &on, &mut away).await.unwrap(), News::default());
        sweep(&db, at(5), &off, &mut away).await.unwrap();
        sweep(&db, at(5 + 240), &off, &mut away).await.unwrap();
        assert!(offer_of(&db, offered.id).await.shift.is_some());
        sweep(&db, at(250), &on, &mut away).await.unwrap();
        sweep(&db, at(255), &off, &mut away).await.unwrap();
        sweep(&db, at(255 + AWAY_SECS - 1), &off, &mut away).await.unwrap();
        assert!(offer_of(&db, offered.id).await.shift.is_some(), "under five minutes");

        let news = sweep(&db, at(255 + AWAY_SECS), &off, &mut away).await.unwrap();
        let after = offer_of(&db, offered.id).await;
        assert_eq!((after.shift, after.status), (None, OfferStatus::Open), "the offer stays up");
        assert_eq!(ended_for(&news, bo.id).and_then(|(s, _)| s.end), Some(ShiftEnd::Away));
        assert!(away.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_term_a_station_gone_and_a_lot_let_go_close_offers(db: PgPool) {
        content(&db).await;
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bo = player(&db, "bo", 0).await;
        let cy = player(&db, "cy", 0).await;
        let (station, _) = bench(&db, ada.id, 0.0).await;
        let (sold, _) = bench(&db, ada.id, 5.0).await;
        let plot = lot(&db, ada.id).await;
        let (due, _) = offer(&db, ada.id, &handles_at(station, 4, 10), now).await.unwrap();
        let (moved, _) = offer(&db, ada.id, &handles_at(sold, 1, 10), now).await.unwrap();
        let (field, _) = offer(&db, ada.id, &grass_on(plot, 1, 10), now).await.unwrap();
        take(&db, bo.id, due.id, now).await.unwrap();
        let start = balance(&db, ada.id).await;
        let online = HashSet::from([bo.id]);
        let mut away = HashMap::new();

        // Nothing yet
        assert_eq!(sweep(&db, now, &online, &mut away).await.unwrap(), News::default());

        // The station someone else's now, the lot let go
        sqlx::query("UPDATE stations SET owner_id = $1 WHERE id = $2").bind(cy.id).bind(sold).execute(&db).await.unwrap();
        sqlx::query("UPDATE plots SET owner_id = NULL WHERE id = $1").bind(plot).execute(&db).await.unwrap();
        sweep(&db, now, &online, &mut away).await.unwrap();
        assert_eq!(offer_of(&db, moved.id).await.status, OfferStatus::Gone);
        assert_eq!(offer_of(&db, field.id).await.status, OfferStatus::Gone);
        assert_eq!(balance(&db, ada.id).await, start + 10 + 10);

        // A day on, the term's out: the shift ends and the rest comes back
        let news = sweep(&db, now + Duration::seconds(DAY_SECS), &online, &mut away).await.unwrap();
        let after = offer_of(&db, due.id).await;
        assert_eq!((after.status, after.escrow_strands), (OfferStatus::Expired, 0));
        assert_eq!(ended_for(&news, bo.id).and_then(|(s, _)| s.end), Some(ShiftEnd::Term));
        assert_eq!(balance(&db, ada.id).await, start + 20 + 40);
    }

    /// Strands and units through seeded lives of offers, shifts, crafts,
    /// gathers, stops, withdrawals and sweeps: every Strand is in a balance,
    /// an escrow or a fee, every balance is its audit trail, and every unit
    /// made, used or gathered is where it was written.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn conservation_through_seeded_wage_lives(db: PgPool) {
        content(&db).await;
        let ada = player(&db, "ada", 5000).await;
        let dee = player(&db, "dee", 5000).await;
        let workers: Vec<Player> = vec![player(&db, "bo", 0).await, player(&db, "cy", 0).await, player(&db, "eve", 50).await];
        let (station, ada_box) = bench(&db, ada.id, 0.0).await;
        let (dee_bench, dee_box) = bench(&db, dee.id, 5.0).await;
        load_handles(&db, ada.id, ada_box).await;
        load_handles(&db, dee.id, dee_box).await;
        let plot = lot(&db, ada.id).await;
        let (events, _rx) = broadcast::channel(4096);
        let everyone: Vec<Uuid> = [ada.id, dee.id].into_iter().chain(workers.iter().map(|w| w.id)).collect();
        let start = balances_of(&db, everyone).await;
        let total = strands_accounted(&db).await;
        let kinds = ["sticks", "rope", "tool_handle", "fiber"];
        let mut expected: HashMap<&str, i64> = HashMap::new();
        for kind in kinds {
            expected.insert(kind, units_of_type(&db, kind).await.count());
        }

        let mut rng = StdRng::seed_from_u64(0x0526);
        let mut now = Utc::now();
        let mut away = HashMap::new();
        for step in 0..160 {
            let worker = &workers[rng.gen_range(0..workers.len())];
            match rng.gen_range(0..8) {
                0 => {
                    let (who, req) = match rng.gen_range(0..3) {
                        0 => (ada.id, handles_at(station, rng.gen_range(1..8), rng.gen_range(1..6))),
                        1 => (dee.id, handles_at(dee_bench, rng.gen_range(1..8), rng.gen_range(1..6))),
                        _ => (ada.id, grass_on(plot, rng.gen_range(1..5), rng.gen_range(1..8))),
                    };
                    let _ = offer(&db, who, &req, now).await;
                }
                1 => {
                    let open = list(&db, worker.id, &WagesQuery::default()).await.unwrap();
                    if !open.is_empty() {
                        let _ = take(&db, worker.id, open[rng.gen_range(0..open.len())].id, now).await;
                    }
                }
                2..=4 => {
                    // Work the shift they're on, if they're on one
                    let Some(shift) = shift_of(&db, worker.id).await.unwrap() else { continue };
                    match offer_of(&db, shift.offer_id).await.work {
                        Work::Craft { station_id, .. } => {
                            let the_box = if station_id == station { ada_box } else { dee_box };
                            let work = shift_at_station(&db, worker.id, station_id).await.unwrap().unwrap();
                            let n = rng.gen_range(1..3);
                            let craft = handle_craft(&db, worker.id, the_box, n, Some(work)).await;
                            if finish_craft(&db, &craft).await.is_ok() {
                                *expected.get_mut("sticks").unwrap() -= 2;
                                *expected.get_mut("rope").unwrap() -= 1;
                                *expected.get_mut("tool_handle").unwrap() += n as i64;
                            }
                        }
                        Work::Gather { .. } => {
                            let work = shift_on_lot(&db, worker.id, LOT, "grass").await.unwrap().unwrap();
                            let n = rng.gen_range(1..4);
                            crate::engine::tick::persist_gather(&db, gathered(worker.id, "fiber", n, Some(work)), &events).await;
                            *expected.get_mut("fiber").unwrap() += n as i64;
                        }
                    }
                }
                5 => {
                    let mine = list(&db, ada.id, &WagesQuery { mine: true, status: None }).await.unwrap();
                    if let Some(o) = mine.first() {
                        if rng.gen_bool(0.5) {
                            let _ = withdraw(&db, ada.id, o.id, now).await;
                        } else {
                            let _ = stop(&db, ada.id, o.id, now).await;
                        }
                    }
                }
                6 => {
                    if let Some(shift) = shift_of(&db, worker.id).await.unwrap() {
                        let _ = stop(&db, worker.id, shift.offer_id, now).await;
                    }
                }
                _ => {
                    // Time passes, and some of them are away
                    now += Duration::seconds(rng.gen_range(60..(DAY_SECS / 3)));
                    let online: HashSet<Uuid> = workers.iter().map(|w| w.id).filter(|_| rng.gen_bool(0.5)).collect();
                    sweep(&db, now, &online, &mut away).await.unwrap();
                }
            }

            assert_eq!(strands_accounted(&db).await, total, "step {step}: Strands made or lost");
            for kind in kinds {
                assert_eq!(every_unit(&db, kind).await.count(), expected[kind], "step {step}: {kind}");
            }
            // Each open offer holds the rest of its pay, and no more
            let held: Vec<(i64, i32, i32, i64)> =
                sqlx::query_as("SELECT rate, cap, made, escrow_strands FROM wage_offers WHERE status = 'open'")
                    .fetch_all(&db)
                    .await
                    .unwrap();
            for (rate, cap, made, escrow) in held {
                assert_eq!(escrow, offer_value(rate, cap - made), "step {step}");
            }
        }
        for kind in kinds {
            assert_eq!(units_of_type(&db, kind).await, every_unit(&db, kind).await, "{kind}'s summaries");
        }
        assert_audited(&db, &start).await;
        // The seed saw both kinds of work paid, and offers end more than one way
        let worked: Vec<(String, i64)> = sqlx::query_as(
            "SELECT o.work_kind, SUM(s.made)::BIGINT FROM wage_shifts s JOIN wage_offers o ON o.id = s.offer_id
             GROUP BY o.work_kind ORDER BY o.work_kind",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        let paid_for = |kind: &str| worked.iter().any(|(k, made)| k == kind && *made > 0);
        assert!(paid_for("craft") && paid_for("gather"), "{worked:?}");
        let ends: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT status FROM wage_offers WHERE status <> 'open' ORDER BY status")
                .fetch_all(&db)
                .await
                .unwrap();
        assert!(ends.len() >= 2, "{ends:?}");
    }
}
