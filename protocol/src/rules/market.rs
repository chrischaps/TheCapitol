//! The market's rules (economy plan D7, E3.1): what an order costs to place,
//! which orders are allowed, and when a sell meets a buy's grade.
//!
//! - Orders are priced per unit in Strands. A sell escrows its units; a buy
//!   escrows `price × quantity` Strands ([`order_value`]).
//! - Placing either charges the listing fee, 1% of the order value rounded up
//!   ([`listing_fee`]). It's a currency sink and isn't refunded on cancel.
//! - A buy names a `min_grade` and a [`GradeRule`]: **average** is met by the
//!   mean of the units sold, so a blend can carry surplus quality over a grade
//!   line; **every unit** is met only by their min ([`meets_grade`]).
//!
//! The server charges exactly these numbers, so a client's preview is its answer.

use serde::{Deserialize, Serialize};

use super::stacking::Units;

/// The listing fee, in hundredths of the order value (1%).
pub const LISTING_FEE_PERCENT: i64 = 1;
/// The lowest price per unit, in Strands.
pub const MIN_PRICE: i64 = 1;
/// The highest price per unit, in Strands.
pub const MAX_PRICE: i64 = 1_000_000_000;
/// The most units one order may trade.
pub const MAX_ORDER_QUANTITY: i32 = 10_000;
/// How long an order rests before it expires and its escrow comes back: 7 days.
pub const ORDER_LIFETIME_SECS: i64 = 7 * 24 * 60 * 60;

/// Which side of the book an order is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    /// The column's value, and the wire's.
    pub fn as_str(self) -> &'static str {
        match self {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        }
    }

    pub fn parse(s: &str) -> Option<OrderSide> {
        match s {
            "buy" => Some(OrderSide::Buy),
            "sell" => Some(OrderSide::Sell),
            _ => None,
        }
    }
}

/// How a buy order's `min_grade` reads the units offered to it (D2, D7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum GradeRule {
    /// The mean of the units sold meets the grade: blends welcome
    #[default]
    Average,
    /// Their min does: every unit is at the grade or better
    EveryUnit,
}

impl GradeRule {
    /// The column's value, and the wire's.
    pub fn as_str(self) -> &'static str {
        match self {
            GradeRule::Average => "average",
            GradeRule::EveryUnit => "every_unit",
        }
    }

    pub fn parse(s: &str) -> Option<GradeRule> {
        match s {
            "average" => Some(GradeRule::Average),
            "every_unit" => Some(GradeRule::EveryUnit),
            _ => None,
        }
    }
}

/// Where an order is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum OrderStatus {
    /// On the book, with some quantity left
    Open,
    /// Every unit traded
    Filled,
    /// Taken off the book by its owner; what was left went back
    Cancelled,
    /// Ran past [`ORDER_LIFETIME_SECS`]; what was left went back
    Expired,
}

impl OrderStatus {
    /// The column's value, and the wire's.
    pub fn as_str(self) -> &'static str {
        match self {
            OrderStatus::Open => "open",
            OrderStatus::Filled => "filled",
            OrderStatus::Cancelled => "cancelled",
            OrderStatus::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Option<OrderStatus> {
        match s {
            "open" => Some(OrderStatus::Open),
            "filled" => Some(OrderStatus::Filled),
            "cancelled" => Some(OrderStatus::Cancelled),
            "expired" => Some(OrderStatus::Expired),
            _ => None,
        }
    }
}

/// Why an order can't be placed as asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderError {
    /// Under [`MIN_PRICE`] or over [`MAX_PRICE`]
    BadPrice,
    /// Under 1 or over [`MAX_ORDER_QUANTITY`]
    BadQuantity,
    /// A grade that isn't A to F
    BadGrade,
}

impl OrderError {
    pub fn message(&self) -> &'static str {
        match self {
            OrderError::BadPrice => "Price each must be from 1 to 1,000,000,000 Strands",
            OrderError::BadQuantity => "An order is for 1 to 10,000 units",
            OrderError::BadGrade => "A grade is A, B, C, D or F",
        }
    }
}

/// Whether an order for `quantity` at `price` each is allowed.
pub fn validate(price: i64, quantity: i32) -> Result<(), OrderError> {
    if !(MIN_PRICE..=MAX_PRICE).contains(&price) {
        return Err(OrderError::BadPrice);
    }
    if !(1..=MAX_ORDER_QUANTITY).contains(&quantity) {
        return Err(OrderError::BadQuantity);
    }
    Ok(())
}

/// What `quantity` units at `price` each are worth. Within [`validate`]'s
/// bounds it's at most 10¹³, so it never overflows.
pub fn order_value(price: i64, quantity: i32) -> i64 {
    price.saturating_mul(quantity as i64)
}

/// The listing fee on an order worth `value`: 1%, rounded up, so every order
/// pays at least 1 Strand.
pub fn listing_fee(value: i64) -> i64 {
    (value.max(0) * LISTING_FEE_PERCENT + 99) / 100
}

/// What placing a buy debits: the escrow, `price × quantity`, plus the fee.
pub fn buy_debit(price: i64, quantity: i32) -> i64 {
    let value = order_value(price, quantity);
    value + listing_fee(value)
}

/// The lowest quality of a grade, or `None` if `grade` isn't one. It's the
/// inverse of [`super::quality_grade`].
pub fn grade_floor(grade: &str) -> Option<i32> {
    match grade {
        "A" => Some(90),
        "B" => Some(75),
        "C" => Some(55),
        "D" => Some(35),
        "F" => Some(0),
        _ => None,
    }
}

/// Whether `units` offered to a buy meet its `min_grade` under `rule`. An
/// empty offer meets nothing; a grade that isn't one is never met.
pub fn meets_grade(units: &Units, min_grade: &str, rule: GradeRule) -> bool {
    let Some(floor) = grade_floor(min_grade) else {
        return false;
    };
    let shown = match rule {
        GradeRule::Average => units.mean(),
        GradeRule::EveryUnit => units.min(),
    };
    shown.is_some_and(|q| q >= floor)
}

#[cfg(test)]
mod tests {
    use super::super::quality_grade;
    use super::*;

    #[test]
    fn the_fee_is_one_percent_rounded_up() {
        assert_eq!(listing_fee(1), 1);
        assert_eq!(listing_fee(100), 1);
        assert_eq!(listing_fee(101), 2);
        assert_eq!(listing_fee(250), 3);
        assert_eq!(listing_fee(10_000), 100);
        assert_eq!(listing_fee(0), 0);
        for value in 1..=100_000i64 {
            let fee = listing_fee(value);
            assert!(fee * 100 >= value && (fee - 1) * 100 < value, "fee on {value}");
        }
        // The biggest order there is fits
        let most = order_value(MAX_PRICE, MAX_ORDER_QUANTITY);
        assert_eq!(most, 10_000_000_000_000);
        assert_eq!(buy_debit(MAX_PRICE, MAX_ORDER_QUANTITY), most + most / 100);
        assert_eq!(buy_debit(7, 3), 22);
    }

    #[test]
    fn orders_stay_in_bounds() {
        assert_eq!(validate(1, 1), Ok(()));
        assert_eq!(validate(MAX_PRICE, MAX_ORDER_QUANTITY), Ok(()));
        assert_eq!(validate(0, 1), Err(OrderError::BadPrice));
        assert_eq!(validate(MAX_PRICE + 1, 1), Err(OrderError::BadPrice));
        assert_eq!(validate(5, 0), Err(OrderError::BadQuantity));
        assert_eq!(validate(5, MAX_ORDER_QUANTITY + 1), Err(OrderError::BadQuantity));
    }

    #[test]
    fn grade_floors_invert_the_grades() {
        for q in 0..=100 {
            let grade = quality_grade(q);
            let floor = grade_floor(grade).unwrap();
            assert!(floor <= q, "{q} is a {grade}, from {floor}");
            if floor > 0 {
                assert_ne!(quality_grade(floor - 1), grade);
            }
        }
        assert_eq!(grade_floor("E"), None);
        assert_eq!(grade_floor("a"), None);
    }

    #[test]
    fn a_blend_meets_average_but_not_every_unit() {
        // Two at 50 and two at 90 show 70: a C on average, a D at worst
        let blend = Units::from_pairs([(50, 2), (90, 2)]);
        assert!(meets_grade(&blend, "C", GradeRule::Average));
        assert!(!meets_grade(&blend, "C", GradeRule::EveryUnit));
        assert!(meets_grade(&blend, "D", GradeRule::EveryUnit));
        assert!(!meets_grade(&blend, "B", GradeRule::Average));
        // Anything meets F; nothing meets a grade that isn't one, or no units
        assert!(meets_grade(&Units::uniform(1, 0), "F", GradeRule::EveryUnit));
        assert!(!meets_grade(&blend, "Z", GradeRule::Average));
        assert!(!meets_grade(&Units::new(), "F", GradeRule::Average));
    }

    #[test]
    fn the_names_round_trip() {
        for side in [OrderSide::Buy, OrderSide::Sell] {
            assert_eq!(OrderSide::parse(side.as_str()), Some(side));
        }
        for rule in [GradeRule::Average, GradeRule::EveryUnit] {
            assert_eq!(GradeRule::parse(rule.as_str()), Some(rule));
            assert_eq!(serde_json::to_string(&rule).unwrap(), format!("\"{}\"", rule.as_str()));
        }
        for status in [OrderStatus::Open, OrderStatus::Filled, OrderStatus::Cancelled, OrderStatus::Expired] {
            assert_eq!(OrderStatus::parse(status.as_str()), Some(status));
            assert_eq!(serde_json::to_string(&status).unwrap(), format!("\"{}\"", status.as_str()));
        }
    }
}
