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

/// The least a filing costs, in Strands.
pub const MIN_FILING_FEE: i64 = 5;
/// The filing fee, in hundredths of a percent of the value (1%).
pub const FILING_FEE_BASIS_POINTS: i64 = 100;
/// The escrow fee, in hundredths of a percent of the value (0.5%).
pub const ESCROW_FEE_BASIS_POINTS: i64 = 50;

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
}
