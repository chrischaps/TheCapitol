//! Selling and bidding by grade (#429). Pure.
//!
//! Every unit keeps its own quality (D2), and the hall's book splits by
//! grade (E3.3): a sell counts at the grade of each of its units, a bid at
//! the grade it asks. A sell that names no units takes the lowest, so a
//! citizen used to sell its worst first, at the good's one price: the wilds'
//! ore, Q40 to Q89 in the pack, went to the book at Q12 (#429). So:
//!
//! - **A bid for any grade gets the lowest units**, at the price the mind
//!   chose ([`plan_sale`]), as before: that's what such a bid is for, and
//!   it keeps the goods moving.
//! - **A grade someone bids for sells into that bid**, at the best bid that
//!   takes it ([`taker`]).
//! - **The best grade stands as an ask at its premium** beside them, so the
//!   book shows it ([`premium`]): the mind's price carried by what quality
//!   is worth ([`worth_factor`]) from the units it was for, to the grade's
//!   middle, no dearer than the cheapest ask at that grade and never under a
//!   bid that takes it. Only a gatherer stands its best: a citizen whose
//!   work uses the good keeps its best for its own crafts, as crafting takes
//!   the best first, and sells its lowest.
//! - **A crafter's standing bid asks a grade** ([`bid_grade`]) for the main
//!   ingredient of what it makes, by its thrift, at that grade's price.
//!
//! Units are never averaged away: a sale names exactly the units it takes,
//! and what's left in the stack is each unit as it was.
//!
//! The mind still chooses per item: its quotes have no grade, so the price
//! is carried to the grade after it decides, and the bid's grade is a rule
//! of thrift, not a choice (#579).
//!
//! A stall stocks a grade to a listing too (#578, [`plan_stall`]), every
//! unit named and each grade at its own price ([`graded_price`]): a buyer
//! who names no units gets a listing's best, so a listing of mixed grades
//! sold its A units at the F's price.

use capitol_protocol::rest::{DepthLevel, MarketTrade};
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

/// `price`, for units at `at`, carried to `grade`'s middle: what a grade
/// stands at with no book to hold it to. `price` itself if `grade` isn't one.
pub fn graded_price(price: i64, at: i32, grade: &str) -> i64 {
    grade_mid(grade).map_or(price.max(1), |mid| carried(price, at, mid))
}

/// The grade of the book's best bid.
pub fn bid_grade_of(book: Option<&Book>) -> Option<&str> {
    book?.side(OrderSide::Buy).next().map(|l| l.grade.as_str())
}

/// The quality a bid's worth is for: the typical trade's, else the best
/// bid's grade, or Q50 for a target price.
pub fn worth_quality(book: Option<&Book>, typical: Option<i32>) -> i32 {
    typical.or_else(|| bid_grade_of(book).and_then(grade_mid)).unwrap_or(50)
}

/// How many of the latest trades [`typical_last`] reads.
pub const TYPICAL_OF: usize = 9;

/// The quality the latest `trades` were, by the unit: what a price for any
/// grade buys.
pub fn typical_quality<'a>(trades: impl IntoIterator<Item = &'a MarketTrade>) -> Option<i32> {
    let (units, total) = trades
        .into_iter()
        .fold((0i64, 0i64), |(n, q), t| (n + i64::from(t.quantity), q + i64::from(t.quantity) * i64::from(t.quality)));
    (units > 0).then(|| (total / units) as i32)
}

/// The price of a good's latest trades (newest first), for any grade: the
/// median of the last [`TYPICAL_OF`]. What the mind's quotes and a market
/// maker take for its last trade, so a lot sold at a grade's premium doesn't
/// make every price for any grade a B's, and good units sold at a bid for
/// any grade don't sink it either. Soaks of #429 had each: the last trade
/// alone ratcheted ore up to +52% of its target, and read at its quality
/// down to 1 Strand.
pub fn typical_last(trades: &[&MarketTrade]) -> Option<i64> {
    let mut prices: Vec<i64> = trades.iter().take(TYPICAL_OF).map(|t| t.price).collect();
    prices.sort_unstable();
    prices.get(prices.len() / 2).copied()
}

fn levels(book: Option<&Book>, side: OrderSide) -> impl Iterator<Item = &DepthLevel> {
    book.into_iter().flat_map(move |b| b.side(side))
}

/// Whether someone bids for `grade` itself, not for any.
fn bid_for(book: Option<&Book>, grade: &str) -> bool {
    grade != "F" && levels(book, OrderSide::Buy).any(|l| l.grade == grade)
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

/// What units of `grade` stand at, when the mind chose `price` for units at
/// `at`: `price` carried to the grade's middle, but no dearer than the
/// cheapest ask at that grade, and never under the best bid that takes it
/// (a bid for any grade at 6 takes B units too).
pub fn premium(price: i64, at: i32, grade: &str, book: Option<&Book>) -> i64 {
    let carried = graded_price(price, at, grade);
    let asked = ask_at(book, grade).map_or(carried, |ask| carried.min(ask));
    asked.max(taker(book, grade).unwrap_or(1)).max(1)
}

/// The units of `held` at `grade`.
pub fn at_grade(held: &Units, grade: &str) -> Units {
    Units::from_pairs(held.pairs().filter(|&(q, _)| quality_grade(q) == grade))
}

/// One order at the hall: the units it names, and its price.
#[derive(Debug, Clone, PartialEq)]
pub struct Sale {
    pub units: Units,
    pub price: i64,
}

/// What a citizen puts on the book at one visit: a sale into the bids,
/// waited on, and an ask that stands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub sell: Option<Sale>,
    pub stand: Option<Sale>,
}

/// What a citizen does at the hall with `held`, the mind having chosen to
/// sell up to `quantity` at `price` on the item's book. `typical` is the
/// quality its trades typically are ([`typical_quality`]).
///
/// - **The sale**, up to `quantity`: of a grade someone bids for, the best
///   such grade (the lowest, if it `keeps_best`), at the best bid that
///   takes it; else, if a bid for any grade pays its best grade's
///   [`premium`], those; else its lowest units into a bid for any grade, at
///   `price`. A grade it stands is kept out of that sale.
/// - **The stand**, up to a `lot` of its best grade at its premium, if it
///   doesn't keep its best, has no ask up (`asking`), promised none
///   (`free`), holds another grade or finds no bid at all, and isn't selling
///   that grade. With no bid at all, a citizen that keeps its best stands its
///   lowest (#501's ask to open the book). The premium is carried from the
///   quality `price` is for: the grade the best bid asks, or for a bid for
///   any grade the mean of the units it gets, or with no bid `typical`, else
///   Q50.
#[allow(clippy::too_many_arguments)]
pub fn plan_sale(
    held: &Units,
    quantity: i32,
    lot: i32,
    price: i64,
    book: Option<&Book>,
    typical: Option<i32>,
    keeps_best: bool,
    asking: bool,
    free: bool,
) -> Plan {
    let grades: Vec<&'static str> = GRADES.iter().copied().filter(|g| !at_grade(held, g).is_empty()).collect();
    let (Some(&top), Some(&lowest)) = (grades.first(), grades.last()) else {
        return Plan::default();
    };
    if quantity < 1 {
        return Plan::default();
    }
    let bids = taker(book, "A").is_some();
    // The grade it would stand: its best, or keeping its best, its lowest
    // to open an empty book
    let standing = match (keeps_best, bids) {
        (false, _) if grades.len() > 1 || !bids => Some(top),
        (true, false) => Some(lowest),
        _ => None,
    }
    .filter(|_| free && !asking);
    // What a bid for any grade gets: the lowest, but for a grade it stands
    let rest = match standing {
        Some(g) if bids => Units::from_pairs(held.pairs().filter(|&(q, _)| quality_grade(q) != g)),
        _ => held.clone(),
    };
    let cheap = rest.pick(quantity.min(rest.count() as i32), Pick::Lowest);
    let at = match bid_grade_of(book) {
        Some("F") => cheap.as_ref().and_then(Units::mean).unwrap_or(17),
        Some(g) => grade_mid(g).unwrap_or(50),
        None => typical.unwrap_or(50),
    };
    let pick = if keeps_best { Pick::Lowest } else { Pick::Best };
    let of_grade = |g: &str, most: i32| {
        let units = at_grade(held, g);
        units.pick((units.count() as i32).min(most), pick)
    };
    let mut order = grades.clone();
    if keeps_best {
        order.reverse();
    }
    let graded = order.iter().copied().find(|g| bid_for(book, g));
    let pays_top = !keeps_best && taker(book, top).is_some_and(|bid| bid >= premium(price, at, top, book));
    let sell = match graded {
        Some(g) => of_grade(g, quantity).zip(taker(book, g)),
        None if pays_top => of_grade(top, quantity).zip(taker(book, top)),
        None if bids => cheap.map(|units| (units, price.max(1))),
        None => None,
    }
    .map(|(units, price)| Sale { units, price });
    let sold = |g: &str| sell.as_ref().is_some_and(|s| s.units.pairs().any(|(q, _)| quality_grade(q) == g));
    let stand = standing
        .filter(|g| !sold(g))
        .and_then(|g| of_grade(g, quantity.min(lot)).map(|units| Sale { units, price: premium(price, at, g, book) }));
    Plan { sell, stand }
}

/// What a citizen stocks its stall with (#578), the mind having chosen to
/// stock up to `quantity` of `held` at `price` for units at `at` (the
/// quality the worth it priced from was for, [`worth_quality`]): one listing
/// a grade, its units named, at the grade's [`graded_price`], in no more
/// than `room` listings. A gatherer stocks its best first; a citizen whose
/// work uses the good (`keeps_best`) its lowest, as at the hall
/// ([`plan_sale`]), and the shelf's room goes to the grades it stocks first.
/// Best grade first either way, so the shelf reads down.
pub fn plan_stall(held: &Units, quantity: i32, price: i64, at: i32, keeps_best: bool, room: usize) -> Vec<Sale> {
    let pick = if keeps_best { Pick::Lowest } else { Pick::Best };
    let Some(chosen) = held.pick(quantity.min(held.count() as i32), pick) else {
        return Vec::new();
    };
    let mut grades: Vec<&str> = GRADES.iter().copied().filter(|g| !at_grade(&chosen, g).is_empty()).collect();
    if keeps_best {
        grades.reverse();
    }
    grades.truncate(room);
    let mut sales: Vec<Sale> = grades
        .into_iter()
        .map(|g| Sale {
            units: at_grade(&chosen, g),
            price: graded_price(price, at, g),
        })
        .collect();
    sales.sort_by_key(|s| std::cmp::Reverse(s.units.max()));
    sales
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

    /// `plan_sale` for a gatherer with no ask up and nothing promised.
    fn plan(held: &Units, quantity: i32, price: i64, book: Option<&Book>) -> Plan {
        plan_sale(held, quantity, 20, price, book, None, false, false, true)
    }

    #[test]
    fn a_gatherer_sells_its_lowest_into_a_bid_for_any_grade_and_stands_its_best_at_a_premium() {
        // #429: the book bids 10 for any ore, and before this the sale took
        // the lowest units at that price and nothing showed the better
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40)]);
        let p = plan(&wild_ore(), 20, 10, Some(&b));
        let sell = p.sell.unwrap();
        assert_eq!(sell.units, Units::from_pairs([(12, 4), (40, 6), (62, 10)]), "the units named, the lowest");
        assert_eq!(sell.price, 10, "at the mind's price, as before");
        let stand = p.stand.unwrap();
        assert_eq!(stand.units, Units::uniform(3, 93), "its best grade, apart");
        // Carried from the mean of what the bid gets (Q45) to A's middle
        assert_eq!(stand.price, carried(10, 45, 95));
        assert!(stand.price > 10, "at a premium: {}", stand.price);
    }

    #[test]
    fn with_its_ask_up_or_its_units_promised_nothing_more_stands() {
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40)]);
        for (asking, free) in [(true, true), (false, false)] {
            let p = plan_sale(&wild_ore(), 20, 20, 10, Some(&b), None, false, asking, free);
            assert_eq!(p.stand, None);
            assert_eq!(p.sell.unwrap().units, Units::from_pairs([(12, 4), (40, 6), (62, 10)]));
        }
    }

    #[test]
    fn a_bid_for_a_grade_buys_that_grade_at_its_price() {
        // Bids best first, as the server sends them
        let b = book(vec![level(OrderSide::Buy, 18, "B", 10), level(OrderSide::Buy, 10, "F", 40)]);
        let p = plan(&wild_ore(), 10, 18, Some(&b));
        assert_eq!(p.sell, Some(Sale { units: Units::uniform(10, 80), price: 18 }));
        // A stands, carried from B, the grade the mind's price was for
        assert_eq!(p.stand, Some(Sale { units: Units::uniform(3, 93), price: carried(18, 82, 95) }));
    }

    #[test]
    fn a_bid_for_any_grade_that_pays_the_premium_takes_the_best() {
        let b = book(vec![level(OrderSide::Buy, 30, "F", 40)]);
        let p = plan(&wild_ore(), 20, 10, Some(&b));
        assert_eq!(p.sell, Some(Sale { units: Units::uniform(3, 93), price: 30 }));
        assert_eq!(p.stand, None, "its best is sold");
    }

    #[test]
    fn a_crafter_keeps_its_best_and_sells_its_lowest() {
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40)]);
        let p = plan_sale(&wild_ore(), 5, 20, 10, Some(&b), None, true, false, true);
        assert_eq!(p.sell, Some(Sale { units: Units::from_pairs([(12, 4), (40, 1)]), price: 10 }));
        assert_eq!(p.stand, None);
    }

    #[test]
    fn an_empty_book_opens_at_a_grade_and_a_standing_ask_is_not_doubled() {
        // #501: nobody bids or asks, the price is a worth, for Q50 or the last trade
        let p = plan(&wild_ore(), 40, 10, None);
        assert_eq!(p.sell, None);
        assert_eq!(p.stand, Some(Sale { units: Units::uniform(3, 93), price: carried(10, 50, 95) }));
        let p = plan_sale(&wild_ore(), 40, 20, 10, None, Some(70), false, false, true);
        assert_eq!(p.stand.unwrap().price, carried(10, 70, 95));
        assert_eq!(plan_sale(&wild_ore(), 40, 20, 10, None, None, false, true, true), Plan::default());
        // One that keeps its best opens with its lowest
        let p = plan_sale(&wild_ore(), 40, 20, 10, None, None, true, false, true);
        assert_eq!(p.stand, Some(Sale { units: Units::uniform(4, 12), price: carried(10, 50, 17) }));
    }

    #[test]
    fn a_premium_is_no_dearer_than_the_cheapest_ask_and_never_under_a_bid_that_takes_it() {
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40), level(OrderSide::Sell, 15, "B", 5)]);
        assert_eq!(premium(10, 17, "B", Some(&b)), 15);
        assert_eq!(premium(10, 17, "C", Some(&b)), carried(10, 17, 64));
        // A bid for any grade at 30 takes B units too
        let b = book(vec![level(OrderSide::Buy, 30, "F", 40)]);
        assert_eq!(premium(10, 17, "B", Some(&b)), 30);
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

    fn shelf(sales: &[Sale]) -> Vec<(Units, i64)> {
        sales.iter().map(|s| (s.units.clone(), s.price)).collect()
    }

    #[test]
    fn a_stall_stocks_a_grade_to_a_listing_each_at_its_price() {
        // #578: 30 of the wild ore, the mind's price 10 for Q50
        let p = plan_stall(&wild_ore(), 30, 10, 50, false, 24);
        assert_eq!(
            shelf(&p),
            vec![(Units::uniform(3, 93), carried(10, 50, 95)), (Units::uniform(27, 80), carried(10, 50, 82))],
            "its best first, each grade apart at its own price"
        );
        assert_eq!(p.iter().map(|s| s.price).collect::<Vec<_>>(), vec![15, 13], "A and B over the mind's 10");
        // One whose work uses ore stocks its lowest, and the F goes under
        // the mind's price, not at it
        let p = plan_stall(&wild_ore(), 12, 10, 50, true, 24);
        assert_eq!(
            shelf(&p),
            vec![
                (Units::uniform(2, 62), carried(10, 50, 64)),
                (Units::uniform(6, 40), carried(10, 50, 44)),
                (Units::uniform(4, 12), carried(10, 50, 17)),
            ]
        );
        assert_eq!(p.iter().map(|s| s.price).collect::<Vec<_>>(), vec![11, 9, 7]);
    }

    #[test]
    fn a_stall_of_several_grades_keeps_to_the_shelf() {
        // Room for one: the grade it stocks first, and the rest stays in the pack
        let p = plan_stall(&wild_ore(), 63, 10, 50, false, 1);
        assert_eq!(p, vec![Sale { units: Units::uniform(3, 93), price: carried(10, 50, 95) }]);
        let p = plan_stall(&wild_ore(), 63, 10, 50, true, 1);
        assert_eq!(p, vec![Sale { units: Units::uniform(4, 12), price: carried(10, 50, 17) }]);
        assert!(plan_stall(&wild_ore(), 5, 10, 50, false, 0).is_empty());
        assert!(plan_stall(&Units::new(), 5, 10, 50, false, 24).is_empty());
        // All it holds: every unit in one listing, a grade to each
        let all = plan_stall(&wild_ore(), 999, 10, 50, false, 24);
        assert_eq!(all.len(), 5);
        assert_eq!(all.iter().map(|s| s.units.count()).sum::<i64>(), wild_ore().count());
        assert_eq!(graded_price(10, 50, "Z"), 10);
    }

    fn trade(quality: i32, price: i64, quantity: i32) -> MarketTrade {
        MarketTrade {
            id: uuid::Uuid::nil(),
            buy_order_id: uuid::Uuid::nil(),
            sell_order_id: uuid::Uuid::nil(),
            item_type: "iron_ore".into(),
            grade: quality_grade(quality).into(),
            quality,
            min_quality: quality,
            price,
            quantity,
            at: chrono::DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_price_for_any_grade_is_the_median_of_the_latest_trades() {
        // One B lot at its premium among trades for any grade
        let b = trade(80, 39, 5);
        let others = [trade(40, 20, 10), trade(38, 19, 15), trade(60, 21, 2)];
        let newest_first: Vec<&MarketTrade> = std::iter::once(&b).chain(others.iter()).collect();
        assert_eq!(typical_last(&newest_first), Some(21), "not the B's 39");
        assert_eq!(typical_quality(newest_first.iter().copied()), Some(46), "by the unit: 1490 / 32");
        // Good units sold cheap don't sink it either
        let cheap = trade(72, 2, 6);
        let newest_first: Vec<&MarketTrade> = std::iter::once(&cheap).chain(others.iter()).collect();
        assert_eq!(typical_last(&newest_first), Some(20));
        assert_eq!(typical_last(&[]), None);
        // Only the latest few count
        let old: Vec<MarketTrade> = (0..20).map(|_| trade(40, 50, 1)).collect();
        let newest_first: Vec<&MarketTrade> = others.iter().cycle().take(TYPICAL_OF).chain(old.iter()).collect();
        assert_eq!(typical_last(&newest_first), Some(20));
    }

    #[test]
    fn the_worth_a_bid_is_carried_from_is_the_typical_trades() {
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40)]);
        assert_eq!(worth_quality(Some(&b), Some(70)), 70);
        assert_eq!(worth_quality(Some(&b), None), 17);
        assert_eq!(worth_quality(None, None), 50);
    }
}
