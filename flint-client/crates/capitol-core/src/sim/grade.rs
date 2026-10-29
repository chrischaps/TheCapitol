//! Goods by grade (#429, #579): what a grade is worth, the book read by
//! grade, and the units a sale names. Pure.
//!
//! Every unit keeps its own quality (D2), and the hall's book splits by
//! grade (E3.3): a sell counts at the grade of each of its units, a bid at
//! the grade it asks. A sell that names no units takes the lowest, so a
//! citizen used to sell its worst first, at the good's one price: the wilds'
//! ore, Q40 to Q89 in the pack, went to the book at Q12 (#429).
//!
//! Since #579 the mind chooses the grade. Its quotes are split by grade
//! ([`quotes`]: the best bid a grade meets, the cheapest ask at it, and what
//! it's worth), and its sell, buy and bid menus offer a grade each
//! ([`graded_sell_menu`](super::graded_sell_menu), [`graded_bids`](super::graded_bids),
//! [`graded_buys`](super::graded_buys)), each priced at its grade. This
//! module keeps the arithmetic those menus price with and the unit picking:
//!
//! - **A sale for any grade gets the loose units** ([`loose`]): those no
//!   grade of their own prices over the bid, lowest first. It keeps the
//!   goods moving, and never throws a good grade in with the worst.
//! - **A sale of a grade names a lot of it alone** ([`lot`]), best first,
//!   or lowest first for a citizen that keeps its best for its own crafts,
//!   as crafting takes the best first.
//!
//! Units are never averaged away: a sale names exactly the units it takes,
//! and what's left in the stack is each unit as it was.
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
use crate::citizen::decide::GradeQuote;
use crate::market::Book;

/// The grades, best first.
pub const GRADES: [&str; 5] = ["A", "B", "C", "D", "F"];

/// The grades a crafter may ask of its main ingredient (#429), best first:
/// none asks for A, and F is any.
pub const BID_GRADES: [&str; 3] = ["B", "C", "D"];

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

/// The quality a price for any grade is for: the typical trade's, else the
/// best bid's grade, or Q50 for a target price.
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
pub fn bid_for(book: Option<&Book>, grade: &str) -> bool {
    grade != "F" && levels(book, OrderSide::Buy).any(|l| l.grade == grade)
}

/// The cheapest ask at `grade`.
pub fn ask_at(book: Option<&Book>, grade: &str) -> Option<i64> {
    levels(book, OrderSide::Sell).filter(|l| l.grade == grade).map(|l| l.price).min()
}

/// The cheapest ask at `grade` or better: what a buy asking that grade
/// pays.
pub fn ask_meeting(book: Option<&Book>, grade: &str) -> Option<i64> {
    let floor = grade_floor(grade)?;
    levels(book, OrderSide::Sell)
        .filter(|l| grade_floor(&l.grade).is_some_and(|f| f >= floor))
        .map(|l| l.price)
        .min()
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

/// What units of `grade` are worth asking for, when `price` is for units at
/// `at`: `price` carried to the grade's middle, but no dearer than the
/// cheapest ask at that grade, and never under the best bid that takes it
/// (a bid for any grade at 6 takes B units too).
pub fn premium(price: i64, at: i32, grade: &str, book: Option<&Book>) -> i64 {
    let carried = graded_price(price, at, grade);
    let asked = ask_at(book, grade).map_or(carried, |ask| carried.min(ask));
    asked.max(taker(book, grade).unwrap_or(1)).max(1)
}

/// A good's book split by grade, as the mind sees it (#579), best first:
/// for each grade, the best bid that takes it, the cheapest ask at it, and
/// `worth` (the price for any grade, for units at `at`) carried to it. A
/// grade the book and the books say nothing of is left out.
pub fn quotes(book: Option<&Book>, worth: Option<i64>, at: i32) -> Vec<GradeQuote> {
    GRADES
        .iter()
        .map(|&g| GradeQuote {
            grade: g.to_string(),
            bid: taker(book, g),
            ask: ask_at(book, g),
            worth: worth.zip(grade_mid(g)).map(|(w, mid)| carried(w, at, mid)),
        })
        .filter(|q| q.bid.is_some() || q.ask.is_some() || q.worth.is_some())
        .collect()
}

/// The units of `held` at `grade`.
pub fn at_grade(held: &Units, grade: &str) -> Units {
    Units::from_pairs(held.pairs().filter(|&(q, _)| quality_grade(q) == grade))
}

/// The grades `held` has units at, best first.
pub fn grades_held(held: &Units) -> Vec<&'static str> {
    GRADES.iter().copied().filter(|g| !at_grade(held, g).is_empty()).collect()
}

/// Whether a grade of `held` is kept apart from a sale for any grade at
/// `bid`, being worth more: someone bids for the grade itself, or its
/// [`premium`] (from `bid`, for units at `at`) is over `bid`. A citizen
/// that `keeps_best` sells its lowest as they come, and holds only its best
/// grade back.
pub fn apart(held: &Units, grade: &str, bid: i64, at: i32, book: Option<&Book>, keeps_best: bool) -> bool {
    if keeps_best {
        let grades = grades_held(held);
        return grades.len() > 1 && grades.first() == Some(&grade);
    }
    bid_for(book, grade) || premium(bid, at, grade, book) > bid
}

/// What a sale for any grade at `bid` may take of `held`: the units of
/// every grade not kept [`apart`].
pub fn loose(held: &Units, bid: i64, at: i32, book: Option<&Book>, keeps_best: bool) -> Units {
    let own: Vec<&str> = grades_held(held)
        .into_iter()
        .filter(|g| apart(held, g, bid, at, book, keeps_best))
        .collect();
    Units::from_pairs(held.pairs().filter(|&(q, _)| !own.contains(&quality_grade(q))))
}

/// A lot of `n` of `held`'s units at `grade`, best first, or the lowest
/// first if it `keeps_best`.
pub fn lot(held: &Units, grade: &str, n: i32, keeps_best: bool) -> Option<Units> {
    let units = at_grade(held, grade);
    units.pick(n.min(units.count() as i32), if keeps_best { Pick::Lowest } else { Pick::Best })
}

/// One listing on a stall: the units it names, and its price.
#[derive(Debug, Clone, PartialEq)]
pub struct Sale {
    pub units: Units,
    pub price: i64,
}

/// What a citizen stocks its stall with (#578), the mind having chosen to
/// stock up to `quantity` of `held` at `price` for units at `at` (the
/// quality the worth it priced from was for, [`worth_quality`]): one listing
/// a grade, its units named, at the grade's [`graded_price`], in no more
/// than `room` listings. A gatherer stocks its best first; a citizen whose
/// work uses the good (`keeps_best`) its lowest, as a [`lot`] does, and the
/// shelf's room goes to the grades it stocks first. Best grade first either
/// way, so the shelf reads down.
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

/// Whether `item` is the main ingredient (half or more of the inputs' share
/// of the quality cascade) of what one of `jobs` makes: what a crafter's
/// menus offer it by grade (#429), the grade being the mind's to choose by
/// its thrift (#579).
pub fn main_ingredient(catalog: &Catalog, item: &str, jobs: &[Role]) -> bool {
    let makes = |output: &str| jobs.iter().any(|j| j.product() == Some(output));
    catalog.recipes().iter().any(|r| {
        let w = &r.quality_weights.inputs;
        let all: f64 = w.values().sum();
        r.outputs.iter().any(|o| makes(&o.item_type)) && all > 0.0 && w.get(item).is_some_and(|share| share * 2.0 >= all)
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn level(side: OrderSide, price: i64, grade: &str, quantity: i64) -> DepthLevel {
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

    pub(crate) fn book(levels: Vec<DepthLevel>) -> Book {
        Book { seq: 1, levels }
    }

    /// An ore gatherer's pack after the wilds: a few low units from the
    /// ring, and most of it C and B.
    pub(crate) fn wild_ore() -> Units {
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
    fn the_book_is_quoted_grade_by_grade() {
        // #579: a bid for any ore at 10, one for C at 14, and a B asked at 25
        let b = book(vec![
            level(OrderSide::Buy, 14, "C", 5),
            level(OrderSide::Buy, 10, "F", 40),
            level(OrderSide::Sell, 25, "B", 5),
        ]);
        let q = quotes(Some(&b), Some(10), 44);
        let at = |g: &str| q.iter().find(|q| q.grade == g).unwrap();
        assert_eq!(q.len(), 5);
        assert_eq!((at("A").bid, at("B").bid, at("C").bid, at("D").bid, at("F").bid), (Some(14), Some(14), Some(14), Some(10), Some(10)));
        assert_eq!((at("B").ask, at("C").ask), (Some(25), None));
        assert_eq!(at("B").worth, Some(carried(10, 44, 82)));
        assert_eq!(at("D").worth, Some(10), "D's middle is the quality the price is for");
        // Nothing known of a grade: left out
        assert!(quotes(None, None, 50).is_empty());
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
    fn a_one_grade_lot_meets_a_bid_at_or_below_its_grade_and_a_buy_an_ask_at_or_above() {
        let b = book(vec![level(OrderSide::Buy, 30, "A", 1), level(OrderSide::Buy, 14, "C", 1), level(OrderSide::Buy, 9, "F", 1)]);
        assert_eq!(taker(Some(&b), "B"), Some(14));
        assert_eq!(taker(Some(&b), "A"), Some(30));
        assert_eq!(taker(Some(&b), "F"), Some(9));
        assert_eq!(taker(None, "A"), None);
        let b = book(vec![level(OrderSide::Sell, 8, "D", 1), level(OrderSide::Sell, 20, "A", 1), level(OrderSide::Sell, 16, "B", 1)]);
        assert_eq!(ask_meeting(Some(&b), "C"), Some(16));
        assert_eq!(ask_meeting(Some(&b), "D"), Some(8));
        assert_eq!(ask_meeting(Some(&b), "A"), Some(20));
    }

    #[test]
    fn a_sale_for_any_grade_takes_the_loose_units_and_never_a_better_grade() {
        // A bid for any ore at 10, for ore typically Q44: everything over D
        // is worth more than that, so only the F and D units are loose
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40)]);
        let held = wild_ore();
        assert_eq!(loose(&held, 10, 44, Some(&b), false), Units::from_pairs([(12, 4), (40, 6)]));
        assert!(apart(&held, "C", 10, 44, Some(&b), false));
        assert!(!apart(&held, "D", 10, 44, Some(&b), false));
        // A bid for D at 9 keeps the D units out of it too
        let b = book(vec![level(OrderSide::Buy, 10, "F", 40), level(OrderSide::Buy, 9, "D", 10)]);
        assert_eq!(loose(&held, 10, 44, Some(&b), false), Units::uniform(4, 12));
        // A crafter that keeps its best sells everything else
        assert_eq!(loose(&held, 10, 44, Some(&b), true), Units::from_pairs([(12, 4), (40, 6), (62, 20), (80, 30)]));
    }

    #[test]
    fn a_lot_is_one_grade_best_first_or_lowest_for_one_that_keeps_its_best() {
        let held = Units::from_pairs([(62, 5), (70, 5), (80, 2)]);
        assert_eq!(lot(&held, "C", 6, false), Some(Units::from_pairs([(62, 1), (70, 5)])));
        assert_eq!(lot(&held, "C", 6, true), Some(Units::from_pairs([(62, 5), (70, 1)])));
        assert_eq!(lot(&held, "C", 50, false), Some(Units::from_pairs([(62, 5), (70, 5)])), "no more than it holds");
        assert_eq!(lot(&held, "A", 1, false), None);
    }

    #[test]
    fn the_main_ingredient_of_what_its_jobs_make_is_offered_by_grade() {
        use crate::sim::Craft;
        let catalog = Catalog::fallback();
        let smith = [Role::Crafter(Craft::Ingots)];
        // Ore is 0.85 of an ingot's inputs; charcoal the rest
        assert!(main_ingredient(&catalog, "iron_ore", &smith));
        assert!(!main_ingredient(&catalog, "charcoal", &smith), "not its main ingredient");
        assert!(main_ingredient(&catalog, "log", &[Role::Crafter(Craft::Planks)]));
        assert!(!main_ingredient(&catalog, "iron_ore", &[Role::Crafter(Craft::Planks)]), "not what it makes");
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
