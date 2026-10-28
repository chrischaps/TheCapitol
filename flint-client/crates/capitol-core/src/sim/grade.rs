//! Selling and bidding by grade (#429). Pure.
//!
//! Every unit keeps its own quality (D2), and the hall's book splits by
//! grade (E3.3): a sell counts at the grade of each of its units, a bid at
//! the grade it asks. A sell that names no units takes the lowest, so a
//! citizen used to sell its worst first, at the good's one price: the wilds'
//! ore, Q40 to Q89 in the pack, went to the book at Q12 (#429). So:
//!
//! - **A sale is one grade's units, named** ([`plan_sale`]), so its ask
//!   stands at that grade and at that grade's price. A gatherer sells its
//!   best first. A citizen whose work uses the good keeps its best for its
//!   own crafts, as crafting takes the best first, and sells its lowest.
//! - **Each grade has its price** ([`graded_price`]). The price the mind
//!   chose is for the grade the book's best bid asks: that grade sells at
//!   it. Another grade sells at a bid that asks for it, if there is one, and
//!   otherwise at the price carried to it by what quality is worth
//!   ([`worth_factor`]), but no dearer than someone already asks for it.
//! - **A grade no bid takes at its price stands** as an ask, the best of
//!   them, so the book shows it at its premium while a grade a bid takes
//!   sells into it on the next visit.
//! - **A crafter's standing bid asks a grade** ([`bid_grade`]) for the main
//!   ingredient of what it makes, by its thrift, at that grade's price.
//!
//! Units are never averaged away: a sale names exactly the units it takes,
//! and what's left in the stack is each unit as it was.
//!
//! The mind still chooses per item: its quotes have no grade, so the price
//! is carried to the grade after it decides, and the bid's grade is a rule
//! of thrift, not a choice (#579). A stall still stocks the lowest units at
//! one price (#578).

use capitol_protocol::rest::DepthLevel;
use capitol_protocol::rules::market::{grade_floor, OrderSide};
use capitol_protocol::rules::quality_grade;
use capitol_protocol::rules::stacking::{Pick, Units};

use super::Role;
use crate::catalog::Catalog;
use crate::market::Book;

/// The grades, best first.
pub const GRADES: [&str; 5] = ["A", "B", "C", "D", "F"];

/// What a unit at `quality` is worth against one at Q50: half at Q0, one and
/// a half at Q100, as quality scales a tool's yield and life (items.toml).
/// A good's quality flows on through the cascade into the tools and kits
/// made of it, so it's worth what it makes.
pub fn worth_factor(quality: i32) -> f64 {
    0.5 + f64::from(quality.clamp(0, 100)) / 100.0
}

/// The middle of a grade's qualities, the quality its price is for: A 95,
/// B 82, C 64, D 44, F 17. `None` if `grade` isn't one.
pub fn grade_mid(grade: &str) -> Option<i32> {
    let floor = grade_floor(grade)?;
    // The grade above's floor, less one
    let top = match grade {
        "A" => 100,
        "B" => 89,
        "C" => 74,
        "D" => 54,
        _ => 34,
    };
    Some((floor + top) / 2)
}

/// `price`, for units at `from`, carried to units at `to`.
pub fn carried(price: i64, from: i32, to: i32) -> i64 {
    ((price as f64 * worth_factor(to) / worth_factor(from)).round() as i64).max(1)
}

/// The grade of the book's best bid: what the mind's price for a sale is
/// for.
pub fn bid_grade_of(book: Option<&Book>) -> Option<&str> {
    book?.side(OrderSide::Buy).next().map(|l| l.grade.as_str())
}

/// The quality a sale's price is for: the best bid's grade, or with no bid,
/// the last trade's (the worth the ask opened at, #501), or Q50 for a target
/// price.
pub fn sale_quality(book: Option<&Book>, last: Option<i32>) -> i32 {
    bid_grade_of(book).and_then(grade_mid).or(last).unwrap_or(50)
}

/// The quality a bid's worth is for: the last trade's, the worth being its
/// price, else the best bid's grade, or Q50 for a target price.
pub fn worth_quality(book: Option<&Book>, last: Option<i32>) -> i32 {
    last.or_else(|| bid_grade_of(book).and_then(grade_mid)).unwrap_or(50)
}

fn levels(book: Option<&Book>, side: OrderSide) -> impl Iterator<Item = &DepthLevel> {
    book.into_iter().flat_map(move |b| b.side(side))
}

/// The best bid that asks for `grade` itself.
fn bid_at(book: Option<&Book>, grade: &str) -> Option<i64> {
    levels(book, OrderSide::Buy).filter(|l| l.grade == grade).map(|l| l.price).max()
}

/// The cheapest ask at `grade`.
fn ask_at(book: Option<&Book>, grade: &str) -> Option<i64> {
    levels(book, OrderSide::Sell).filter(|l| l.grade == grade).map(|l| l.price).min()
}

/// The best bid that takes units of `grade`: one asking that grade or a
/// lower one. Every unit of a one-grade lot is at the grade, so its mean is
/// too, and a bid of either rule takes it.
pub fn taker(book: Option<&Book>, grade: &str) -> Option<i64> {
    let floor = grade_floor(grade)?;
    levels(book, OrderSide::Buy)
        .filter(|l| grade_floor(&l.grade).is_some_and(|f| f <= floor))
        .map(|l| l.price)
        .max()
}

/// What units of `grade` sell for, when the mind chose `price` for units at
/// `at` ([`sale_quality`]): `price` itself for the best bid's grade; a bid's
/// own price for a grade someone bids for; else `price` carried to the
/// grade's middle, but no dearer than the cheapest ask at it. Never less
/// than the best bid that takes the grade ([`taker`]): a bid for any grade
/// at 6 takes D units too, so a D bid at 1 doesn't make them worth 1.
pub fn graded_price(price: i64, at: i32, grade: &str, book: Option<&Book>) -> i64 {
    if bid_grade_of(book) == Some(grade) {
        return price.max(1);
    }
    let Some(mid) = grade_mid(grade) else {
        return price.max(1);
    };
    let guess = bid_at(book, grade).unwrap_or_else(|| {
        let carried = carried(price, at, mid);
        ask_at(book, grade).map_or(carried, |ask| carried.min(ask))
    });
    guess.max(taker(book, grade).unwrap_or(1)).max(1)
}

/// The units of `held` at `grade`.
pub fn at_grade(held: &Units, grade: &str) -> Units {
    Units::from_pairs(held.pairs().filter(|&(q, _)| quality_grade(q) == grade))
}

/// One sale at the hall: the units it names, at one grade, and its price.
#[derive(Debug, Clone, PartialEq)]
pub struct Sale {
    pub grade: &'static str,
    pub units: Units,
    pub price: i64,
    /// Nobody bids its price for the grade: the ask stands on the book
    pub stands: bool,
}

/// What a citizen sells of `held`, up to `quantity`, when the mind chose
/// `price` for units at `at` ([`sale_quality`]).
///
/// It goes through the grades it holds, best first, or lowest first if it
/// `keeps_best` for its own work. The first grade a bid takes at its price
/// ([`graded_price`], [`taker`]) sells into it. But if the first grade it
/// would sell isn't that one and it has no ask up already (`asking`), that
/// grade stands as an ask instead, at most a `lot`, so the book shows it at
/// its price. With no ask up and no bid that takes any, the first stands.
/// `None`: its ask is up and no bid takes anything else.
#[allow(clippy::too_many_arguments)]
pub fn plan_sale(
    held: &Units,
    quantity: i32,
    lot: i32,
    price: i64,
    at: i32,
    book: Option<&Book>,
    keeps_best: bool,
    asking: bool,
) -> Option<Sale> {
    let mut grades: Vec<&'static str> = GRADES.iter().copied().filter(|g| !at_grade(held, g).is_empty()).collect();
    if keeps_best {
        grades.reverse();
    }
    let first = *grades.first()?;
    let pick = if keeps_best { Pick::Lowest } else { Pick::Best };
    let sale = |grade: &'static str, most: i32, stands: bool| {
        let units = at_grade(held, grade);
        let n = (units.count() as i32).min(most).max(1);
        Some(Sale {
            grade,
            units: units.pick(n, pick)?,
            price: graded_price(price, at, grade, book),
            stands,
        })
    };
    let fills = grades
        .iter()
        .copied()
        .find(|g| taker(book, g).is_some_and(|bid| bid >= graded_price(price, at, g, book)));
    match fills {
        _ if quantity < 1 => None,
        Some(g) if g == first || asking => sale(g, quantity, false),
        _ if !asking => sale(first, quantity.min(lot), true),
        _ => None,
    }
}

/// The grade a crafter's standing bid for `item` asks (#429), or `None` for
/// any: when `item` is the main ingredient (half or more of the inputs'
/// share of the quality cascade) of what one of `jobs` makes, by its
/// `thrift`. The least thrifty pay for B, then C, then D; the thriftiest
/// take any.
pub fn bid_grade(catalog: &Catalog, item: &str, jobs: &[Role], thrift: f32) -> Option<&'static str> {
    let makes = |output: &str| jobs.iter().any(|j| j.product() == Some(output));
    let main = catalog.recipes().iter().any(|r| {
        let w = &r.quality_weights.inputs;
        let all: f64 = w.values().sum();
        r.outputs.iter().any(|o| makes(&o.item_type)) && all > 0.0 && w.get(item).is_some_and(|share| share * 2.0 >= all)
    });
    if !main {
        return None;
    }
    match thrift {
        t if t < 0.25 => Some("B"),
        t if t < 0.5 => Some("C"),
        t if t < 0.75 => Some("D"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(side: OrderSide, price: i64, grade: &str, quantity: i64) -> DepthLevel {
        DepthLevel {
            side,
            price,
            grade: grade.into(),
            grade_rule: None,
            durability: None,
            max_durability: None,
            quantity,
            orders: 1,
        }
    }

    fn book(levels: Vec<DepthLevel>) -> Book {
        Book { seq: 1, levels }
    }

    /// An ore gatherer's pack after the wilds: a few low units from the
    /// ring, and most of it C and B.
    fn wild_ore() -> Units {
        Units::from_pairs([(12, 4), (40, 6), (62, 20), (80, 30), (93, 3)])
    }

    #[test]
    fn a_grade_is_worth_what_its_quality_makes() {
        assert_eq!([95, 82, 64, 44, 17], GRADES.map(|g| grade_mid(g).unwrap()));
        assert_eq!(grade_mid("Z"), None);
        assert_eq!((worth_factor(0), worth_factor(50), worth_factor(100)), (0.5, 1.0, 1.5));
        assert_eq!(carried(10, 17, 82), 20, "a B unit is worth about twice an F at the F's price");
        assert_eq!(carried(10, 50, 50), 10);
        assert_eq!(carried(1, 95, 0), 1, "never under a Strand");
    }

    #[test]
    fn a_gatherer_stands_its_best_grade_at_a_premium_rather_than_sell_it_to_an_any_grade_bid() {
        // #429: the book bids 10 for any ore, and before this the sale took
        // the lowest units at that price
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40)]);
        let at = sale_quality(Some(&b), None);
        assert_eq!(at, 17);
        let sale = plan_sale(&wild_ore(), 20, 20, 10, at, Some(&b), false, false).unwrap();
        assert_eq!(sale.grade, "A");
        assert_eq!(sale.units, Units::uniform(3, 93), "the units named, the best");
        assert!(sale.stands);
        assert_eq!(sale.price, carried(10, 17, 95));
        assert!(sale.price > 20, "at a premium: {}", sale.price);
    }

    #[test]
    fn with_its_best_up_a_gatherer_sells_the_grade_a_bid_takes() {
        // Its A ask stands; the any-grade bid takes only F at its price
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40), level(OrderSide::Sell, 22, "A", 3)]);
        let at = sale_quality(Some(&b), None);
        let sale = plan_sale(&wild_ore(), 20, 20, 10, at, Some(&b), false, true).unwrap();
        assert_eq!((sale.grade, sale.price, sale.stands), ("F", 10, false));
        assert_eq!(sale.units, Units::uniform(4, 12), "all four F, never a blend");
    }

    #[test]
    fn a_bid_for_a_grade_buys_that_grade_at_its_price() {
        // Bids best first, as the server sends them
        let b = book(vec![level(OrderSide::Buy, 18, "B", 10), level(OrderSide::Buy, 10, "F", 40)]);
        // The best bid asks B at 18: the mind's price is for B
        let at = sale_quality(Some(&b), None);
        assert_eq!(at, 82);
        // A, carried to a premium nobody bids, stands first
        let sale = plan_sale(&wild_ore(), 10, 20, 18, at, Some(&b), false, false).unwrap();
        assert_eq!((sale.grade, sale.stands), ("A", true));
        let next = plan_sale(&wild_ore(), 10, 20, 18, at, Some(&b), false, true).unwrap();
        assert_eq!((next.grade, next.price, next.stands), ("B", 18, false));
        assert_eq!(next.units, Units::uniform(10, 80));
    }

    #[test]
    fn a_crafter_keeps_its_best_and_sells_its_lowest() {
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40)]);
        let sale = plan_sale(&wild_ore(), 5, 20, 10, 17, Some(&b), true, false).unwrap();
        assert_eq!((sale.grade, sale.price, sale.stands), ("F", 10, false));
        assert_eq!(sale.units, Units::uniform(4, 12), "only the F, though five were asked");
    }

    #[test]
    fn an_empty_book_opens_at_the_best_grade_and_a_standing_ask_is_not_doubled() {
        // #501: nobody bids or asks, the price is a worth at Q50
        let sale = plan_sale(&wild_ore(), 40, 20, 10, 50, None, false, false).unwrap();
        assert_eq!((sale.grade, sale.price, sale.stands), ("A", carried(10, 50, 95), true));
        assert_eq!(plan_sale(&wild_ore(), 40, 20, 10, 50, None, false, true), None);
    }

    #[test]
    fn a_grade_is_no_dearer_than_its_cheapest_ask() {
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40), level(OrderSide::Sell, 15, "B", 5)]);
        assert_eq!(graded_price(10, 17, "B", Some(&b)), 15);
        assert_eq!(graded_price(10, 17, "C", Some(&b)), carried(10, 17, 64));
        assert_eq!(graded_price(12, 17, "F", Some(&b)), 12, "the bid's grade sells at the mind's price");
    }

    #[test]
    fn a_grade_never_sells_under_a_bid_that_takes_it() {
        // Seen in a soak (#429): a merchant bid 6 for any logs while a
        // citizen bid 1 for D
        let b = book(vec![level(OrderSide::Buy, 6, "F", 20), level(OrderSide::Buy, 1, "D", 6)]);
        assert_eq!(graded_price(6, 17, "D", Some(&b)), 6);
        let b = book(vec![level(OrderSide::Buy, 9, "F", 20), level(OrderSide::Buy, 12, "D", 6)].into_iter().rev().collect());
        assert_eq!(graded_price(12, 44, "C", Some(&b)), carried(12, 44, 64), "carried from the D bid");
        assert_eq!(graded_price(12, 44, "F", Some(&b)), 9);
    }

    #[test]
    fn a_one_grade_lot_meets_a_bid_at_or_below_its_grade() {
        let b = book(vec![level(OrderSide::Buy, 30, "A", 1), level(OrderSide::Buy, 14, "C", 1), level(OrderSide::Buy, 9, "F", 1)]);
        assert_eq!(taker(Some(&b), "B"), Some(14));
        assert_eq!(taker(Some(&b), "A"), Some(30));
        assert_eq!(taker(Some(&b), "F"), Some(9));
        assert_eq!(taker(None, "A"), None);
    }

    #[test]
    fn a_crafter_bids_for_a_grade_of_its_main_ingredient_by_its_thrift() {
        use crate::sim::Craft;
        let catalog = Catalog::fallback();
        let smith = [Role::Crafter(Craft::Ingots)];
        // Ore is 0.85 of an ingot's inputs; charcoal the rest
        assert_eq!(bid_grade(&catalog, "iron_ore", &smith, 0.1), Some("B"));
        assert_eq!(bid_grade(&catalog, "iron_ore", &smith, 0.4), Some("C"));
        assert_eq!(bid_grade(&catalog, "iron_ore", &smith, 0.6), Some("D"));
        assert_eq!(bid_grade(&catalog, "iron_ore", &smith, 0.9), None, "the thriftiest take any");
        assert_eq!(bid_grade(&catalog, "charcoal", &smith, 0.1), None, "not its main ingredient");
        assert_eq!(bid_grade(&catalog, "log", &[Role::Crafter(Craft::Planks)], 0.4), Some("C"));
        assert_eq!(bid_grade(&catalog, "iron_ore", &[Role::Crafter(Craft::Planks)], 0.1), None, "not what it makes");
    }

    #[test]
    fn the_worth_a_bid_is_carried_from_is_the_last_trades() {
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40)]);
        assert_eq!(worth_quality(Some(&b), Some(70)), 70);
        assert_eq!(worth_quality(Some(&b), None), 17);
        assert_eq!(worth_quality(None, None), 50);
        assert_eq!(sale_quality(None, Some(70)), 70);
    }
}
