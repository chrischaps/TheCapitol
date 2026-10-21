//! The market's rules (economy plan D7, E3.1): what an order costs to place,
//! which orders are allowed, and when a sell meets a buy's grade.
//!
//! - Orders are priced per unit in Strands. A sell escrows its units; a buy
//!   escrows `price × quantity` Strands ([`order_value`]).
//! - Placing either charges the listing fee, 1% of the order value rounded up
//!   by default. It's a live knob since D6.1, so the fee charged is the
//!   server's [`crate::tunables::Fees::listing_fee`], and a buy's debit
//!   [`crate::tunables::Fees::buy_debit`]. It's a currency sink and isn't
//!   refunded on cancel.
//! - A buy names a `min_grade` and a [`GradeRule`]: **average** is met by the
//!   mean of the units sold, so a blend can carry surplus quality over a grade
//!   line; **every unit** is met only by their min ([`meets_grade`]).
//!
//! The server charges exactly these numbers, so a client's preview is its answer.

use serde::{Deserialize, Serialize};

use super::stacking::{Pick, Units};
use crate::rest::DepthLevel;

/// The listing fee by default, in hundredths of the order value (1%). The
/// live one is [`crate::tunables::Fees`]'s.
pub const LISTING_FEE_PERCENT: i64 = 1;
/// The lowest price per unit, in Strands.
pub const MIN_PRICE: i64 = 1;
/// The highest price per unit, in Strands.
pub const MAX_PRICE: i64 = 1_000_000_000;
/// The most units one order may trade.
pub const MAX_ORDER_QUANTITY: i32 = 10_000;
/// How long an order rests before it expires and its escrow comes back: 7 days.
pub const ORDER_LIFETIME_SECS: i64 = 7 * 24 * 60 * 60;
/// How close to the Exchange hall's footprint you must stand to place an
/// order or collect from your market box (D6, E3.4). Browsing, the chart and
/// cancelling work from anywhere. It's the station range, so standing on the
/// avenue outside is enough.
pub const HALL_RANGE: f64 = super::STATION_INTERACTION_RANGE;

/// Whether `(x, y)` is close enough to `hall` to place or collect.
pub fn at_the_hall(hall: &crate::terrain::Landmark, x: f64, y: f64) -> bool {
    hall.distance_to(x, y) <= HALL_RANGE
}

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

/// Which units of a sell's `units` a buy for up to `want` of them, at
/// `min_grade` under `rule`, takes in one fill (E3.2). `None` if no unit can
/// meet it, and then no fill can.
///
/// It's as many as can meet the grade, up to `want`, leaning low: the buyer
/// gets what they asked for and the seller keeps the quality left over for
/// buyers who ask for more.
/// - **every unit**: the lowest units at the grade's floor or better.
/// - **average**: the most units whose best mean still meets the floor, then
///   as many of the lowest as that mean can carry, the rest from the top.
///   (Not always the lowest total that would do: a pair from the middle can
///   beat a low and a high. It's simple, and never takes fewer units.)
///
/// Whether there's a fill at all doesn't depend on `want`, and taking units
/// never makes a lot meet a grade it didn't. So once a book has no fill left,
/// only a new order can make one.
pub fn fill_units(units: &Units, want: i32, min_grade: &str, rule: GradeRule) -> Option<Units> {
    let floor = grade_floor(min_grade)? as i64;
    let n = (want as i64).min(units.count());
    if n < 1 {
        return None;
    }
    match rule {
        GradeRule::EveryUnit => {
            let eligible = Units::from_pairs(units.pairs().filter(|&(q, _)| q as i64 >= floor));
            let m = n.min(eligible.count());
            eligible.pick(m as i32, Pick::Lowest)
        }
        GradeRule::Average => {
            // The best `m` units' mean falls as `m` grows: the most that meet it
            let meets = |total: i64, m: i64| total >= floor * m;
            let m = largest(1, n, |m| meets(sum_of(units, m, Pick::Best), m))?;
            // Swapping a top unit for a bottom one only lowers the total: the
            // most bottom units the floor allows
            let k = largest(0, m, |k| {
                meets(sum_of(units, k, Pick::Lowest) + sum_of(units, m - k, Pick::Best), m)
            })?;
            let mut rest = units.clone();
            let mut taken = if k > 0 { rest.take(k as i32, Pick::Lowest)? } else { Units::new() };
            if m > k {
                taken.merge(&rest.take((m - k) as i32, Pick::Best)?);
            }
            Some(taken)
        }
    }
}

/// The summed quality of the `n` units `pick` would take.
fn sum_of(units: &Units, n: i64, pick: Pick) -> i64 {
    let order: Box<dyn Iterator<Item = (i32, i32)>> = match pick {
        Pick::Lowest => Box::new(units.pairs()),
        Pick::Best => Box::new(units.pairs().rev()),
    };
    let (mut left, mut sum) = (n, 0i64);
    for (q, c) in order {
        if left == 0 {
            break;
        }
        let used = (c as i64).min(left);
        sum += q as i64 * used;
        left -= used;
    }
    sum
}

/// The largest `x` in `lo..=hi` for which `ok` holds, where `ok` holds up to
/// some point and not after. `None` if it fails at `lo`.
fn largest(lo: i64, hi: i64, ok: impl Fn(i64) -> bool) -> Option<i64> {
    if !ok(lo) {
        return None;
    }
    let (mut good, mut bad) = (lo, hi + 1);
    while bad - good > 1 {
        let mid = good + (bad - good) / 2;
        if ok(mid) {
            good = mid;
        } else {
            bad = mid;
        }
    }
    Some(good)
}

// ---------------------------------------------------------------------------
// Depth (E3.3): an item's book, aggregated, and the diffs that keep a copy of
// it the same as the server's
// ---------------------------------------------------------------------------

/// Where a level sits in a book: bids best first (price high to low), then
/// asks best first (low to high). At one price the better grade comes first,
/// then `average` before `every_unit`.
fn level_key(level: &DepthLevel) -> (u8, i64, &str, u8) {
    let (side, price) = match level.side {
        OrderSide::Buy => (0, -level.price),
        OrderSide::Sell => (1, level.price),
    };
    let rule = match level.grade_rule {
        None => 0,
        Some(GradeRule::Average) => 1,
        Some(GradeRule::EveryUnit) => 2,
    };
    (side, price, level.grade.as_str(), rule)
}

/// Put `levels` in book order.
pub fn sort_levels(levels: &mut [DepthLevel]) {
    levels.sort_by(|a, b| level_key(a).cmp(&level_key(b)));
}

/// The changes that take the book `old` to `new` (both in book order): every
/// level that's new or changed, as it is in `new`, and every level that's
/// gone, with `quantity` and `orders` 0. Empty when nothing changed.
pub fn depth_diff(old: &[DepthLevel], new: &[DepthLevel]) -> Vec<DepthLevel> {
    let mut diff = Vec::new();
    let (mut o, mut n) = (old.iter().peekable(), new.iter().peekable());
    loop {
        match (o.peek(), n.peek()) {
            (None, None) => break,
            (Some(&gone), None) => {
                diff.push(DepthLevel { quantity: 0, orders: 0, ..gone.clone() });
                o.next();
            }
            (None, Some(&added)) => {
                diff.push(added.clone());
                n.next();
            }
            (Some(&a), Some(&b)) => match level_key(a).cmp(&level_key(b)) {
                std::cmp::Ordering::Less => {
                    diff.push(DepthLevel { quantity: 0, orders: 0, ..a.clone() });
                    o.next();
                }
                std::cmp::Ordering::Greater => {
                    diff.push(b.clone());
                    n.next();
                }
                std::cmp::Ordering::Equal => {
                    if a != b {
                        diff.push(b.clone());
                    }
                    o.next();
                    n.next();
                }
            },
        }
    }
    diff
}

/// Apply a [`depth_diff`] to `levels`, keeping them in book order: a level
/// with `quantity` 0 goes, any other takes its place or joins.
pub fn apply_depth_diff(levels: &mut Vec<DepthLevel>, diff: &[DepthLevel]) {
    for change in diff {
        let at = levels.binary_search_by(|l| level_key(l).cmp(&level_key(change)));
        match (at, change.quantity > 0) {
            (Ok(i), true) => levels[i] = change.clone(),
            (Ok(i), false) => {
                levels.remove(i);
            }
            (Err(i), true) => levels.insert(i, change.clone()),
            (Err(_), false) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::quality_grade;
    use super::*;

    #[test]
    fn the_fee_is_one_percent_rounded_up() {
        let fees = crate::tunables::Fees::default();
        assert_eq!(fees.listing_fee(1), 1);
        assert_eq!(fees.listing_fee(100), 1);
        assert_eq!(fees.listing_fee(101), 2);
        assert_eq!(fees.listing_fee(250), 3);
        assert_eq!(fees.listing_fee(10_000), 100);
        assert_eq!(fees.listing_fee(0), 0);
        for value in 1..=100_000i64 {
            let fee = fees.listing_fee(value);
            assert!(fee * 100 >= value && (fee - 1) * 100 < value, "fee on {value}");
        }
        // The biggest order there is fits
        let most = order_value(MAX_PRICE, MAX_ORDER_QUANTITY);
        assert_eq!(most, 10_000_000_000_000);
        assert_eq!(fees.buy_debit(MAX_PRICE, MAX_ORDER_QUANTITY), most + most / 100);
        assert_eq!(fees.buy_debit(7, 3), 22);
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
    fn a_fill_takes_the_lowest_units_that_meet_the_grade() {
        let lot = Units::from_pairs([(40, 3), (60, 2), (92, 2)]);
        // Every unit at B or better: only the 92s can go
        assert_eq!(fill_units(&lot, 5, "B", GradeRule::EveryUnit), Some(Units::uniform(2, 92)));
        // At D every unit is fine, so the lowest first
        assert_eq!(fill_units(&lot, 4, "D", GradeRule::EveryUnit), Some(Units::from_pairs([(40, 3), (60, 1)])));
        // C on average for 4: two 40s ride on the 92s (40+40+92+92 = 264 ≥ 220)
        assert_eq!(
            fill_units(&lot, 4, "C", GradeRule::Average),
            Some(Units::from_pairs([(40, 2), (92, 2)]))
        );
        // A on average: only the 92s, however many are wanted
        assert_eq!(fill_units(&lot, 7, "A", GradeRule::Average), Some(Units::uniform(2, 92)));
        // F takes the lowest, as any trade does
        assert_eq!(fill_units(&lot, 2, "F", GradeRule::Average), Some(Units::uniform(2, 40)));
        // Nothing meets A at every unit in a lot topping out at 89; nothing meets a non-grade
        assert_eq!(fill_units(&Units::uniform(9, 89), 3, "A", GradeRule::EveryUnit), None);
        assert_eq!(fill_units(&Units::uniform(9, 89), 3, "A", GradeRule::Average), None);
        assert_eq!(fill_units(&lot, 3, "Z", GradeRule::Average), None);
        assert_eq!(fill_units(&lot, 0, "F", GradeRule::Average), None);
        assert_eq!(fill_units(&Units::new(), 1, "F", GradeRule::Average), None);
    }

    /// Against every sub-lot of small random lots: a fill meets the grade,
    /// takes as many units as any sub-lot could, and there's a fill whenever
    /// one unit alone could meet it.
    #[test]
    fn a_fill_is_as_big_as_any_that_meets_the_grade() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..3000 {
            let mut lot = Units::new();
            for _ in 0..=next(4) {
                lot.add(next(101) as i32, 1 + next(2) as i32);
            }
            let want = 1 + next(lot.count() as u64 + 1) as i32;
            let grade = ["A", "B", "C", "D", "F"][next(5) as usize];
            for rule in [GradeRule::Average, GradeRule::EveryUnit] {
                let best = biggest_meeting(&lot, want, grade, rule);
                match fill_units(&lot, want, grade, rule) {
                    None => assert_eq!(best, 0, "{lot:?} {want} {grade} {rule:?}"),
                    Some(taken) => {
                        assert!(lot.contains(&taken));
                        assert!(meets_grade(&taken, grade, rule), "{lot:?} gave {taken:?}");
                        assert_eq!(taken.count(), best, "{lot:?} {want} {grade} {rule:?} gave {taken:?}");
                    }
                }
            }
        }
    }

    /// The most units of `lot`, up to `want`, that some sub-lot of meets the grade
    fn biggest_meeting(lot: &Units, want: i32, grade: &str, rule: GradeRule) -> i64 {
        let flat: Vec<i32> = lot.pairs().flat_map(|(q, c)| std::iter::repeat_n(q, c as usize)).collect();
        let mut best = 0;
        for mask in 1u32..(1 << flat.len()) {
            let picked = Units::from_pairs(
                flat.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, &q)| (q, 1)),
            );
            if picked.count() <= want as i64 && meets_grade(&picked, grade, rule) {
                best = best.max(picked.count());
            }
        }
        best
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

    fn level(side: OrderSide, price: i64, grade: &str, rule: Option<GradeRule>, quantity: i64) -> DepthLevel {
        DepthLevel { side, price, grade: grade.into(), grade_rule: rule, quantity, orders: 1 }
    }

    #[test]
    fn levels_sort_bids_then_asks_best_first() {
        let mut levels = vec![
            level(OrderSide::Sell, 12, "A", None, 1),
            level(OrderSide::Buy, 9, "B", Some(GradeRule::Average), 1),
            level(OrderSide::Sell, 11, "C", None, 1),
            level(OrderSide::Buy, 10, "C", Some(GradeRule::EveryUnit), 1),
            level(OrderSide::Buy, 10, "C", Some(GradeRule::Average), 1),
            level(OrderSide::Sell, 11, "A", None, 1),
        ];
        sort_levels(&mut levels);
        let order: Vec<_> = levels.iter().map(|l| (l.side, l.price, l.grade.as_str(), l.grade_rule)).collect();
        assert_eq!(
            order,
            [
                (OrderSide::Buy, 10, "C", Some(GradeRule::Average)),
                (OrderSide::Buy, 10, "C", Some(GradeRule::EveryUnit)),
                (OrderSide::Buy, 9, "B", Some(GradeRule::Average)),
                (OrderSide::Sell, 11, "A", None),
                (OrderSide::Sell, 11, "C", None),
                (OrderSide::Sell, 12, "A", None),
            ]
        );
    }

    #[test]
    fn a_diff_applied_to_the_old_book_makes_the_new() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let random_book = |next: &mut dyn FnMut(u64) -> u64| {
            let mut levels: Vec<DepthLevel> = Vec::new();
            for _ in 0..next(12) {
                let side = if next(2) == 0 { OrderSide::Buy } else { OrderSide::Sell };
                let rule = match side {
                    OrderSide::Buy => Some(if next(2) == 0 { GradeRule::Average } else { GradeRule::EveryUnit }),
                    OrderSide::Sell => None,
                };
                let l = level(side, 8 + next(5) as i64, ["A", "B", "F"][next(3) as usize], rule, 1 + next(4) as i64);
                if !levels.iter().any(|x| level_key(x) == level_key(&l)) {
                    levels.push(l);
                }
            }
            sort_levels(&mut levels);
            levels
        };
        for _ in 0..2000 {
            let old = random_book(&mut next);
            let new = random_book(&mut next);
            let diff = depth_diff(&old, &new);
            let mut copy = old.clone();
            apply_depth_diff(&mut copy, &diff);
            assert_eq!(copy, new);
            assert_eq!(depth_diff(&new, &new), Vec::new());
        }
    }
}
