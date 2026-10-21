//! The bazaar's rules (economy plan D6, E3.10): what a stall costs, how close
//! you must stand to it, and which shops and listings are allowed.
//!
//! - A merchant rents one stall at a time, at [`RENT_PER_DAY`] Strands a day
//!   by default (a live knob since D6.1, so what's charged is the server's
//!   [`crate::tunables::Fees::rent`]), paid in advance for 1 to
//!   [`MAX_RENT_DAYS`] days. Paying again
//!   extends it, up to [`MAX_RENT_DAYS`] ahead. The rent is the bazaar's sink:
//!   there's no listing fee.
//! - A listing is a stack at a fixed price per unit. Its units keep their own
//!   quality (D2), and a buyer may name which they take.
//! - Stocking, buying and seeing the prices happen at the stall
//!   ([`at_the_stall`]); the directory of shops works from anywhere.
//!
//! The server charges exactly these numbers, so a client's preview is its answer.

use super::market::{MAX_ORDER_QUANTITY, MAX_PRICE, MIN_PRICE};
use crate::terrain::Landmark;

/// A stall's rent by default, in Strands a day (GDD §8.4). The live one is
/// [`crate::tunables::Fees`]'s.
pub const RENT_PER_DAY: i64 = 20;
/// The most days a rent may be paid ahead.
pub const MAX_RENT_DAYS: i64 = 7;
/// A day of rent, in seconds.
pub const RENT_DAY_SECS: i64 = 24 * 60 * 60;
/// How close to a stall's footprint you must stand to stock it, buy from it or
/// see its prices: 2 m, so from the lane's crown you reach the stalls either
/// side of you and those facing them, and no further.
pub const STALL_RANGE: f64 = 20.0;
/// The longest a shop's name may be, in characters.
pub const MAX_SHOP_NAME: usize = 40;
/// The most listings one shop may hold.
pub const MAX_LISTINGS: usize = 24;

/// Whether `(x, y)` is close enough to `stall` to stock, buy or see prices.
pub fn at_the_stall(stall: &Landmark, x: f64, y: f64) -> bool {
    stall.distance_to(x, y) <= STALL_RANGE
}

/// Why a rent, a name or a listing was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BazaarError {
    /// Days outside 1 to [`MAX_RENT_DAYS`], or paid further ahead than that
    BadDays,
    /// An empty name, or longer than [`MAX_SHOP_NAME`]
    BadName,
    /// A price outside the market's bounds
    BadPrice,
    /// A quantity below 1 or over the market's most
    BadQuantity,
}

impl BazaarError {
    pub fn message(&self) -> String {
        match self {
            BazaarError::BadDays => format!("Rent is paid 1 to {MAX_RENT_DAYS} days ahead"),
            BazaarError::BadName => format!("A shop's name is 1 to {MAX_SHOP_NAME} characters"),
            BazaarError::BadPrice => format!("A price is {MIN_PRICE} to {MAX_PRICE} Strands a unit"),
            BazaarError::BadQuantity => format!("A quantity is 1 to {MAX_ORDER_QUANTITY}"),
        }
    }
}

/// `days` of rent, if they're allowed.
pub fn validate_days(days: i64) -> Result<(), BazaarError> {
    if (1..=MAX_RENT_DAYS).contains(&days) {
        Ok(())
    } else {
        Err(BazaarError::BadDays)
    }
}

/// A shop's name as it's kept: trimmed, and refused if that leaves it empty
/// or too long.
pub fn shop_name(name: &str) -> Result<String, BazaarError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_SHOP_NAME || name.chars().any(char::is_control) {
        return Err(BazaarError::BadName);
    }
    Ok(name.to_string())
}

/// A listing's price, and a quantity to stock or buy, if they're allowed.
pub fn validate_listing(price: i64, quantity: i32) -> Result<(), BazaarError> {
    if !(MIN_PRICE..=MAX_PRICE).contains(&price) {
        return Err(BazaarError::BadPrice);
    }
    validate_quantity(quantity)
}

/// A quantity to stock or buy, if it's allowed.
pub fn validate_quantity(quantity: i32) -> Result<(), BazaarError> {
    if (1..=MAX_ORDER_QUANTITY).contains(&quantity) {
        Ok(())
    } else {
        Err(BazaarError::BadQuantity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::LandmarkKind;

    #[test]
    fn a_week_is_the_most_and_a_day_the_least() {
        let fees = crate::tunables::Fees::default();
        assert_eq!(fees.rent(1), 20);
        assert_eq!(fees.rent(7), 140);
        assert!(validate_days(0).is_err());
        assert!(validate_days(1).is_ok());
        assert!(validate_days(7).is_ok());
        assert!(validate_days(8).is_err());
    }

    #[test]
    fn a_name_is_trimmed_and_bounded() {
        assert_eq!(shop_name("  Ada's Ropes ").unwrap(), "Ada's Ropes");
        assert!(shop_name("   ").is_err());
        assert!(shop_name(&"x".repeat(MAX_SHOP_NAME)).is_ok());
        assert!(shop_name(&"x".repeat(MAX_SHOP_NAME + 1)).is_err());
        assert!(shop_name("a\nb").is_err());
    }

    #[test]
    fn listings_keep_the_markets_bounds() {
        assert!(validate_listing(1, 1).is_ok());
        assert_eq!(validate_listing(0, 1), Err(BazaarError::BadPrice));
        assert_eq!(validate_listing(5, 0), Err(BazaarError::BadQuantity));
        assert_eq!(validate_listing(5, MAX_ORDER_QUANTITY + 1), Err(BazaarError::BadQuantity));
    }

    #[test]
    fn the_stall_is_reached_from_just_in_front() {
        let stall = Landmark {
            id: "bazaar_stall_1".into(),
            kind: LandmarkKind::Stall,
            position: (5.0, 10.0),
            corners: [(0.0, 20.0), (0.0, 0.0), (10.0, 0.0), (10.0, 20.0)],
            entrance_edge: (0, 1),
            door: (0.0, 10.0),
        };
        assert!(at_the_stall(&stall, 5.0, 10.0));
        assert!(at_the_stall(&stall, -STALL_RANGE, 10.0));
        assert!(!at_the_stall(&stall, -STALL_RANGE - 1.0, 10.0));
    }
}
