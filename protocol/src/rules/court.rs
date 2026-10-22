//! The Contract Court's fee (commons plan D5 and D7, S0.2): what posting a
//! commission or offering wages costs on top of the escrow.
//!
//! - The **filing fee** is 1% of the value, rounded up, and never under
//!   [`MIN_FILING_FEE`] (GDD §8.4).
//! - The **escrow fee** is 0.5% of the value, rounded up.
//! - The court charges both together as one fee, [`court_fee`]. It's a
//!   currency sink (`court_fee`), kept on a cancel or an expiry.
//!
//! The value is a commission's `price × quantity`, or a wage offer's
//! `rate × cap`. The server charges exactly these numbers, so a client's
//! preview is its answer.
//!
//! **Commissions** (D5, S2.1): a standing request for `quantity` units of an
//! item at or over `min_quality`, at `price` each, due in 1 to 7 days, with
//! the whole price escrowed. [`validate_commission`] holds the bounds (the
//! market's price and quantity, [`validate`]), and [`delivery`] is which
//! units a delivery takes: the deliverer's named units, or the lowest that
//! meet the quality, so the better ones stay theirs.
//!
//! **Claims** (D6, S2.2): a deliverer's promise to bring k units, which holds
//! that share for them until [`claim_until`]: a day, never past the due date.
//! Nobody else may deliver or claim a held share ([`unclaimed`],
//! [`deliverable`]). A claim delivered in full is kept; one that runs out is
//! lapsed, a breach on its claimant's record, and so is one released with
//! [`CLAIM_GRACE_SECS`] or less left ([`release_status`]).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::market::{order_value, validate, OrderError};
use super::stacking::{Pick, Units, MAX_QUALITY, MIN_QUALITY};

/// The least a filing costs, in Strands.
pub const MIN_FILING_FEE: i64 = 5;
/// The filing fee, in hundredths of a percent of the value (1%).
pub const FILING_FEE_BASIS_POINTS: i64 = 100;
/// The escrow fee, in hundredths of a percent of the value (0.5%).
pub const ESCROW_FEE_BASIS_POINTS: i64 = 50;

/// The fewest days a commission may run.
pub const MIN_DUE_DAYS: i64 = 1;
/// The most days a commission may run.
pub const MAX_DUE_DAYS: i64 = 7;
/// A day of a commission's term, in seconds: a game day is a real one, as
/// the bazaar's rent day and the market's order lifetime are.
pub const DAY_SECS: i64 = 24 * 60 * 60;
/// The most commissions one person may have open at once (D5): the same for
/// everyone, citizens and subscribers alike.
pub const MAX_OPEN_COMMISSIONS: i64 = 5;

/// `basis_points` hundredths of a percent of `value`, rounded up. A value
/// under 0 counts as 0.
fn share(value: i64, basis_points: i64) -> i64 {
    value.max(0).saturating_mul(basis_points).saturating_add(9_999) / 10_000
}

/// The filing fee on `value`: 1% rounded up, at least [`MIN_FILING_FEE`].
pub fn filing_fee(value: i64) -> i64 {
    share(value, FILING_FEE_BASIS_POINTS).max(MIN_FILING_FEE)
}

/// The escrow fee on `value`: 0.5% rounded up.
pub fn escrow_fee(value: i64) -> i64 {
    share(value, ESCROW_FEE_BASIS_POINTS)
}

/// What the court charges to hold `value` in escrow: the filing fee plus the
/// escrow fee.
pub fn court_fee(value: i64) -> i64 {
    filing_fee(value).saturating_add(escrow_fee(value))
}

/// Where a commission is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum CommissionStatus {
    /// On the board, wanting more
    Open,
    /// Every unit delivered
    Filled,
    /// Taken down by its poster; what was unspent went back
    Cancelled,
    /// Ran past its due date; what was unspent went back
    Expired,
}

impl CommissionStatus {
    /// The column's value, and the wire's.
    pub fn as_str(self) -> &'static str {
        match self {
            CommissionStatus::Open => "open",
            CommissionStatus::Filled => "filled",
            CommissionStatus::Cancelled => "cancelled",
            CommissionStatus::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Option<CommissionStatus> {
        match s {
            "open" => Some(CommissionStatus::Open),
            "filled" => Some(CommissionStatus::Filled),
            "cancelled" => Some(CommissionStatus::Cancelled),
            "expired" => Some(CommissionStatus::Expired),
            _ => None,
        }
    }
}

/// Why a commission can't be posted as asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommissionError {
    /// The price or quantity is out of the market's bounds
    Order(OrderError),
    /// A quality that isn't 0 to 100
    BadQuality,
    /// Due in fewer than [`MIN_DUE_DAYS`] or more than [`MAX_DUE_DAYS`]
    BadDue,
}

impl CommissionError {
    pub fn message(&self) -> &'static str {
        match self {
            CommissionError::Order(OrderError::BadQuantity) => "A commission is for 1 to 10,000 units",
            CommissionError::Order(e) => e.message(),
            CommissionError::BadQuality => "The lowest quality taken is from 0 to 100",
            CommissionError::BadDue => "A commission is due in 1 to 7 days",
        }
    }
}

/// Whether a commission for `quantity` units at or over `min_quality`, at
/// `price` each, due in `due_days`, may be posted.
pub fn validate_commission(price: i64, quantity: i32, min_quality: i32, due_days: i64) -> Result<(), CommissionError> {
    validate(price, quantity).map_err(CommissionError::Order)?;
    if !(MIN_QUALITY..=MAX_QUALITY).contains(&min_quality) {
        return Err(CommissionError::BadQuality);
    }
    if !(MIN_DUE_DAYS..=MAX_DUE_DAYS).contains(&due_days) {
        return Err(CommissionError::BadDue);
    }
    Ok(())
}

/// The value a commission escrows: `price × quantity`.
pub fn commission_value(price: i64, quantity: i32) -> i64 {
    order_value(price, quantity)
}

/// Why a delivery can't take the units asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryError {
    /// A named unit, or every unit held, is under the commission's quality
    BelowGrade,
    /// The named units aren't all in the stack
    NotHeld,
    /// Nothing to deliver: a quantity under 1, or no units named
    Nothing,
}

impl DeliveryError {
    pub fn message(&self) -> &'static str {
        match self {
            DeliveryError::BelowGrade => "Those units are under the quality the commission asks for",
            DeliveryError::NotHeld => "That stack doesn't hold those units",
            DeliveryError::Nothing => "Deliver at least one unit",
        }
    }
}

/// Which units a delivery from `held` takes, toward a commission wanting
/// `wanted` more at or over `min_quality`:
/// - with `named`, those units, every one at the quality and in the stack,
///   and past `wanted` (or `at_most`) only the lowest of them;
/// - without, the lowest units that meet the quality (the best-fitting: the
///   better ones stay the deliverer's), as many as are held, `wanted` and
///   `at_most` allow.
pub fn delivery(
    held: &Units,
    min_quality: i32,
    wanted: i32,
    named: Option<&Units>,
    at_most: Option<i32>,
) -> Result<Units, DeliveryError> {
    let cap = at_most.map_or(wanted, |n| n.min(wanted));
    if cap < 1 {
        return Err(DeliveryError::Nothing);
    }
    let (from, short) = match named {
        Some(named) if named.is_empty() => return Err(DeliveryError::Nothing),
        Some(named) if named.min().is_some_and(|q| q < min_quality) => return Err(DeliveryError::BelowGrade),
        Some(named) if !held.contains(named) => return Err(DeliveryError::NotHeld),
        Some(named) => (named.clone(), DeliveryError::Nothing),
        None => (Units::from_pairs(held.pairs().filter(|(q, _)| *q >= min_quality)), DeliveryError::BelowGrade),
    };
    let n = from.count().min(cap as i64) as i32;
    from.pick(n, Pick::Lowest).ok_or(short)
}

/// The longest a claim holds its share, in seconds: a game day (D6).
pub const CLAIM_SECS: i64 = DAY_SECS;
/// A claim released with more than this left, in seconds, is no breach; a
/// later release is a lapse. The claimant is told when this much is left.
pub const CLAIM_GRACE_SECS: i64 = 60 * 60;

/// Where a claim is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ClaimStatus {
    /// Holding its share
    Open,
    /// Delivered in full in time
    Kept,
    /// Ran out short, or released late: a breach. Its share went back to the
    /// board.
    Lapsed,
    /// Given up in good time: no breach
    Released,
}

impl ClaimStatus {
    /// The column's value, and the wire's.
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimStatus::Open => "open",
            ClaimStatus::Kept => "kept",
            ClaimStatus::Lapsed => "lapsed",
            ClaimStatus::Released => "released",
        }
    }

    pub fn parse(s: &str) -> Option<ClaimStatus> {
        match s {
            "open" => Some(ClaimStatus::Open),
            "kept" => Some(ClaimStatus::Kept),
            "lapsed" => Some(ClaimStatus::Lapsed),
            "released" => Some(ClaimStatus::Released),
            _ => None,
        }
    }
}

/// Why a claim can't be made as asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimError {
    /// A quantity under 1
    Nothing,
    /// More than the unclaimed part, which is this many
    TooMany(i32),
}

impl ClaimError {
    pub fn message(&self) -> String {
        match self {
            ClaimError::Nothing => "Claim at least one unit".to_string(),
            ClaimError::TooMany(0) => "Everything still wanted is claimed".to_string(),
            ClaimError::TooMany(n) => format!("Only {n} still wanted are unclaimed"),
        }
    }
}

/// What a commission still wants that no claim holds: anyone may claim or
/// deliver it. `left` is what's still wanted, `claimed` what the claims
/// standing hold of it. Nothing, once its poster has withdrawn it: then only
/// the claims are delivered.
pub fn unclaimed(left: i32, claimed: i32, withdrawn: bool) -> i32 {
    if withdrawn {
        0
    } else {
        (left - claimed).max(0)
    }
}

/// The most a deliverer may bring: the unclaimed part, and what's still
/// owed on their own claim (`own`, 0 without one). Their deliveries count
/// against their claim first.
pub fn deliverable(unclaimed: i32, own: i32) -> i32 {
    unclaimed + own.max(0)
}

/// Whether a claim on `quantity` units may be made when `unclaimed` are.
pub fn validate_claim(quantity: i32, unclaimed: i32) -> Result<(), ClaimError> {
    if quantity < 1 {
        Err(ClaimError::Nothing)
    } else if quantity > unclaimed {
        Err(ClaimError::TooMany(unclaimed.max(0)))
    } else {
        Ok(())
    }
}

/// When a claim made at `now` on a commission due at `due_at` runs out: a
/// day on, or the due date if that's sooner.
pub fn claim_until(now: DateTime<Utc>, due_at: DateTime<Utc>) -> DateTime<Utc> {
    (now + Duration::seconds(CLAIM_SECS)).min(due_at)
}

/// How a claim running until `until` ends if its claimant releases it at
/// `now`: released with more than [`CLAIM_GRACE_SECS`] left, lapsed after.
pub fn release_status(now: DateTime<Utc>, until: DateTime<Utc>) -> ClaimStatus {
    if until - now > Duration::seconds(CLAIM_GRACE_SECS) {
        ClaimStatus::Released
    } else {
        ClaimStatus::Lapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filing_costs_at_least_five() {
        assert_eq!(filing_fee(0), 5);
        assert_eq!(filing_fee(100), 5);
        assert_eq!(filing_fee(500), 5);
        // 1% passes the minimum at 501
        assert_eq!(filing_fee(501), 6);
        assert_eq!(filing_fee(10_000), 100);
    }

    #[test]
    fn each_fee_rounds_up() {
        assert_eq!(escrow_fee(1), 1);
        assert_eq!(escrow_fee(200), 1);
        assert_eq!(escrow_fee(201), 2);
        assert_eq!(filing_fee(1_001), 11);
        // 1% of 1234 is 12.34 and 0.5% is 6.17
        assert_eq!(court_fee(1_234), 13 + 7);
    }

    #[test]
    fn value_zero_costs_only_the_minimum() {
        assert_eq!(escrow_fee(0), 0);
        assert_eq!(court_fee(0), MIN_FILING_FEE);
        // Nothing under 0
        assert_eq!(court_fee(-50), MIN_FILING_FEE);
    }

    #[test]
    fn a_large_value_saturates_rather_than_overflowing() {
        assert!(court_fee(i64::MAX) > 0);
    }

    #[test]
    fn a_commission_keeps_to_its_bounds() {
        assert_eq!(validate_commission(6, 20, 55, 4), Ok(()));
        assert_eq!(validate_commission(0, 20, 55, 4), Err(CommissionError::Order(OrderError::BadPrice)));
        assert_eq!(validate_commission(6, 0, 55, 4), Err(CommissionError::Order(OrderError::BadQuantity)));
        assert_eq!(validate_commission(6, 10_001, 55, 4), Err(CommissionError::Order(OrderError::BadQuantity)));
        assert_eq!(validate_commission(6, 20, -1, 4), Err(CommissionError::BadQuality));
        assert_eq!(validate_commission(6, 20, 101, 4), Err(CommissionError::BadQuality));
        assert_eq!(validate_commission(6, 20, 55, 0), Err(CommissionError::BadDue));
        assert_eq!(validate_commission(6, 20, 55, 8), Err(CommissionError::BadDue));
        assert_eq!(validate_commission(6, 20, 0, 1), Ok(()));
        assert_eq!(validate_commission(6, 20, 100, 7), Ok(()));
        assert_eq!(commission_value(6, 20), 120);
    }

    #[test]
    fn a_status_reads_back_as_written() {
        for status in [
            CommissionStatus::Open,
            CommissionStatus::Filled,
            CommissionStatus::Cancelled,
            CommissionStatus::Expired,
        ] {
            assert_eq!(CommissionStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(CommissionStatus::parse("lapsed"), None);
    }

    #[test]
    fn a_delivery_takes_the_lowest_that_meet_the_quality() {
        let held = Units::from_pairs([(40, 5), (60, 3), (80, 2), (95, 1)]);
        // The 40s are under 55; the 60s go first, then the 80s
        assert_eq!(delivery(&held, 55, 20, None, None), Ok(Units::from_pairs([(60, 3), (80, 2), (95, 1)])));
        assert_eq!(delivery(&held, 55, 4, None, None), Ok(Units::from_pairs([(60, 3), (80, 1)])));
        assert_eq!(delivery(&held, 55, 20, None, Some(2)), Ok(Units::uniform(2, 60)));
        // Nothing held meets it
        assert_eq!(delivery(&held, 96, 20, None, None), Err(DeliveryError::BelowGrade));
        assert_eq!(delivery(&held, 55, 20, None, Some(0)), Err(DeliveryError::Nothing));
        assert_eq!(delivery(&held, 55, 0, None, None), Err(DeliveryError::Nothing));
    }

    #[test]
    fn named_units_must_all_meet_the_quality_and_be_held() {
        let held = Units::from_pairs([(40, 5), (60, 3), (95, 1)]);
        let best = Units::from_pairs([(60, 1), (95, 1)]);
        assert_eq!(delivery(&held, 55, 20, Some(&best), None), Ok(best.clone()));
        // Past what's wanted, the lowest named go
        assert_eq!(delivery(&held, 55, 1, Some(&best), None), Ok(Units::uniform(1, 60)));
        // One of them under the quality refuses all of it
        let mixed = Units::from_pairs([(40, 1), (95, 1)]);
        assert_eq!(delivery(&held, 55, 20, Some(&mixed), None), Err(DeliveryError::BelowGrade));
        assert_eq!(delivery(&held, 55, 20, Some(&Units::uniform(2, 95)), None), Err(DeliveryError::NotHeld));
        assert_eq!(delivery(&held, 55, 20, Some(&Units::new()), None), Err(DeliveryError::Nothing));
    }

    #[test]
    fn a_claim_status_reads_back_as_written() {
        for status in [ClaimStatus::Open, ClaimStatus::Kept, ClaimStatus::Lapsed, ClaimStatus::Released] {
            assert_eq!(ClaimStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(ClaimStatus::parse("expired"), None);
    }

    #[test]
    fn claims_never_hold_more_than_is_wanted() {
        // 12 still wanted, 7 of them claimed
        assert_eq!(unclaimed(12, 7, false), 5);
        assert_eq!(validate_claim(5, unclaimed(12, 7, false)), Ok(()));
        assert_eq!(validate_claim(6, unclaimed(12, 7, false)), Err(ClaimError::TooMany(5)));
        assert_eq!(validate_claim(1, unclaimed(7, 7, false)), Err(ClaimError::TooMany(0)));
        assert_eq!(validate_claim(0, 5), Err(ClaimError::Nothing));
        // Withdrawn, nothing is open to claim
        assert_eq!(unclaimed(12, 7, true), 0);
        // A claimant brings the unclaimed part and their own share; others only
        // the unclaimed part
        assert_eq!(deliverable(unclaimed(12, 7, false), 4), 9);
        assert_eq!(deliverable(unclaimed(12, 7, false), 0), 5);
        assert_eq!(deliverable(unclaimed(12, 7, true), 4), 4);
    }

    #[test]
    fn a_claim_runs_a_day_but_never_past_the_due_date() {
        let now = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(claim_until(now, now + Duration::days(3)), now + Duration::days(1));
        assert_eq!(claim_until(now, now + Duration::hours(5)), now + Duration::hours(5));
    }

    #[test]
    fn a_release_in_the_last_hour_is_a_lapse() {
        let until = DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(release_status(until - Duration::hours(5), until), ClaimStatus::Released);
        assert_eq!(release_status(until - Duration::minutes(61), until), ClaimStatus::Released);
        assert_eq!(release_status(until - Duration::minutes(60), until), ClaimStatus::Lapsed);
        assert_eq!(release_status(until - Duration::minutes(5), until), ClaimStatus::Lapsed);
        assert_eq!(release_status(until + Duration::minutes(5), until), ClaimStatus::Lapsed);
    }
}
