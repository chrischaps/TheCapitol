//! Piece-rate wages (commons plan D7, S2.6): work at someone else's station,
//! or on their lot, paid unit by unit as it's made.
//!
//! - An **offer** is a station of the employer's and a recipe at it
//!   ([`Work::Craft`]), or a lot of theirs and a resource on it
//!   ([`Work::Gather`]), at a `rate` per unit, up to a `cap`, for 1 to 7 days.
//! - **The escrow** is `rate × cap` ([`offer_value`]), plus the court's fee
//!   on it (`rules::court::court_fee`), all paid when it's offered.
//! - **A shift** is one worker at a time on an offer, and one shift at a time
//!   for a worker. Each unit made is paid the rate out of the escrow, in the
//!   transaction that made it, up to the cap ([`payable`]).
//! - **Stopping.** Either side can stop at any time, and what was made is
//!   always paid. The offer closes at the cap or its end, or when the
//!   employer withdraws it, and what's left of the escrow comes back.
//!
//! The server charges exactly these numbers, so a client's preview is its answer.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::market::{order_value, validate, OrderError};

/// The fewest days an offer may run.
pub const MIN_TERM_DAYS: i64 = 1;
/// The most days an offer may run.
pub const MAX_TERM_DAYS: i64 = 7;
/// A day of an offer's term, in seconds: a game day is a real one, as the
/// bazaar's rent day and the market's order lifetime are.
pub const DAY_SECS: i64 = 24 * 60 * 60;
/// The most offers one employer may have open at once: the court's limit on
/// commissions (D5), for everyone alike.
pub const MAX_OPEN_OFFERS: i64 = 5;
/// How long a worker may be away before their shift ends (D7): 5 minutes.
pub const AWAY_SECS: i64 = 5 * 60;

/// What an offer is for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Work {
    /// Crafting `recipe_id` at the employer's station `station_id`, from the
    /// inputs in its box. The output goes into the same box.
    Craft { station_id: Uuid, recipe_id: String },
    /// Gathering `resource` (a resource type's id) on the employer's lot
    /// `plot_id`. What's gathered goes into the employer's market box.
    Gather { plot_id: Uuid, resource: String },
}

impl Work {
    /// The column's value for the kind
    pub fn kind(&self) -> &'static str {
        match self {
            Work::Craft { .. } => "craft",
            Work::Gather { .. } => "gather",
        }
    }
}

/// Where an offer is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum OfferStatus {
    /// Up on the board, to be taken or being worked
    Open,
    /// Every unit up to the cap made and paid
    Filled,
    /// Taken down by the employer; what was unspent went back
    Withdrawn,
    /// Ran to the end of its term; what was unspent went back
    Expired,
    /// Its station went, or its lot changed hands; what was unspent went back
    Gone,
}

impl OfferStatus {
    /// The column's value, and the wire's.
    pub fn as_str(self) -> &'static str {
        match self {
            OfferStatus::Open => "open",
            OfferStatus::Filled => "filled",
            OfferStatus::Withdrawn => "withdrawn",
            OfferStatus::Expired => "expired",
            OfferStatus::Gone => "gone",
        }
    }

    pub fn parse(s: &str) -> Option<OfferStatus> {
        match s {
            "open" => Some(OfferStatus::Open),
            "filled" => Some(OfferStatus::Filled),
            "withdrawn" => Some(OfferStatus::Withdrawn),
            "expired" => Some(OfferStatus::Expired),
            "gone" => Some(OfferStatus::Gone),
            _ => None,
        }
    }
}

/// Why a shift ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ShiftEnd {
    /// The cap was reached
    Cap,
    /// The offer's term ran out
    Term,
    /// The worker stopped
    Worker,
    /// The employer stopped the shift, or withdrew the offer
    Employer,
    /// The worker was away [`AWAY_SECS`]
    Away,
    /// The station went, or the lot changed hands
    Gone,
    /// The employer's market box had no room for a gather
    BoxFull,
}

impl ShiftEnd {
    /// The column's value, and the wire's.
    pub fn as_str(self) -> &'static str {
        match self {
            ShiftEnd::Cap => "cap",
            ShiftEnd::Term => "term",
            ShiftEnd::Worker => "worker",
            ShiftEnd::Employer => "employer",
            ShiftEnd::Away => "away",
            ShiftEnd::Gone => "gone",
            ShiftEnd::BoxFull => "box_full",
        }
    }

    pub fn parse(s: &str) -> Option<ShiftEnd> {
        match s {
            "cap" => Some(ShiftEnd::Cap),
            "term" => Some(ShiftEnd::Term),
            "worker" => Some(ShiftEnd::Worker),
            "employer" => Some(ShiftEnd::Employer),
            "away" => Some(ShiftEnd::Away),
            "gone" => Some(ShiftEnd::Gone),
            "box_full" => Some(ShiftEnd::BoxFull),
            _ => None,
        }
    }

    /// In words, after "The shift ended: "
    pub fn words(self) -> &'static str {
        match self {
            ShiftEnd::Cap => "all the work offered is done",
            ShiftEnd::Term => "the offer ran out",
            ShiftEnd::Worker => "the worker stopped",
            ShiftEnd::Employer => "the employer stopped it",
            ShiftEnd::Away => "the worker was away too long",
            ShiftEnd::Gone => "the station or the lot is gone",
            ShiftEnd::BoxFull => "the employer's market box is full",
        }
    }
}

/// Why an offer can't be made as asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferError {
    /// The rate or cap is out of the market's bounds
    Order(OrderError),
    /// A term of fewer than [`MIN_TERM_DAYS`] or more than [`MAX_TERM_DAYS`]
    BadTerm,
}

impl OfferError {
    pub fn message(&self) -> &'static str {
        match self {
            OfferError::Order(OrderError::BadPrice) => "The rate is from 1 to 1,000,000,000 Strands a unit",
            OfferError::Order(OrderError::BadQuantity) => "An offer is for 1 to 10,000 units",
            OfferError::Order(e) => e.message(),
            OfferError::BadTerm => "An offer runs 1 to 7 days",
        }
    }
}

/// Whether an offer of `rate` a unit, up to `cap` units, for `term_days`, may be made.
pub fn validate_offer(rate: i64, cap: i32, term_days: i64) -> Result<(), OfferError> {
    validate(rate, cap).map_err(OfferError::Order)?;
    if !(MIN_TERM_DAYS..=MAX_TERM_DAYS).contains(&term_days) {
        return Err(OfferError::BadTerm);
    }
    Ok(())
}

/// The value an offer escrows: `rate × cap`.
pub fn offer_value(rate: i64, cap: i32) -> i64 {
    order_value(rate, cap)
}

/// How many of `made` new units are paid, on an offer of `cap` with `so_far`
/// made already: all of them up to the cap. A last craft or gather that
/// makes more than the cap leaves goes to the employer whole, and the units
/// over it are unpaid, so the escrow is never short.
pub fn payable(cap: i32, so_far: i32, made: i32) -> i32 {
    made.clamp(0, (cap - so_far).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offer_runs_one_to_seven_days() {
        assert_eq!(validate_offer(4, 40, 0), Err(OfferError::BadTerm));
        assert_eq!(validate_offer(4, 40, 1), Ok(()));
        assert_eq!(validate_offer(4, 40, 7), Ok(()));
        assert_eq!(validate_offer(4, 40, 8), Err(OfferError::BadTerm));
    }

    #[test]
    fn rate_and_cap_keep_the_markets_bounds() {
        assert_eq!(validate_offer(0, 40, 1), Err(OfferError::Order(OrderError::BadPrice)));
        assert_eq!(validate_offer(4, 0, 1), Err(OfferError::Order(OrderError::BadQuantity)));
        assert_eq!(validate_offer(4, 10_001, 1), Err(OfferError::Order(OrderError::BadQuantity)));
        assert_eq!(offer_value(4, 40), 160);
    }

    #[test]
    fn pay_stops_at_the_cap() {
        assert_eq!(payable(40, 0, 1), 1);
        assert_eq!(payable(40, 38, 3), 2, "the third is over the cap");
        assert_eq!(payable(40, 40, 2), 0);
        assert_eq!(payable(40, 0, 0), 0, "a failed pull pays nothing");
        assert_eq!(payable(40, 45, 1), 0, "never under 0");
    }

    #[test]
    fn statuses_and_ends_round_trip() {
        for s in [OfferStatus::Open, OfferStatus::Filled, OfferStatus::Withdrawn, OfferStatus::Expired, OfferStatus::Gone] {
            assert_eq!(OfferStatus::parse(s.as_str()), Some(s));
        }
        for e in [
            ShiftEnd::Cap,
            ShiftEnd::Term,
            ShiftEnd::Worker,
            ShiftEnd::Employer,
            ShiftEnd::Away,
            ShiftEnd::Gone,
            ShiftEnd::BoxFull,
        ] {
            assert_eq!(ShiftEnd::parse(e.as_str()), Some(e));
            assert_eq!(serde_json::to_value(e).unwrap(), e.as_str());
        }
    }

    #[test]
    fn work_is_tagged_by_kind() {
        let craft = Work::Craft { station_id: Uuid::nil(), recipe_id: "iron_ingot".into() };
        let json = serde_json::to_value(&craft).unwrap();
        assert_eq!(json["kind"], "craft");
        assert_eq!(serde_json::from_value::<Work>(json).unwrap(), craft);
        assert_eq!(Work::Gather { plot_id: Uuid::nil(), resource: "grass".into() }.kind(), "gather");
    }
}
