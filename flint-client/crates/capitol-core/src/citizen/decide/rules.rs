//! The rules layer: the [`Role`](crate::sim::Role) goals and trade rules the
//! sim's bots play by, as a utility pick weighted by [`Temperament`](super::Temperament).
//!
//! Each entry on the menu gets a utility, about 0.9 for "what the rules
//! would do" and less for the rest, and the best is picked (the earlier
//! entry, on a tie). The confidence is how well the best fits (its utility,
//! up to 1) times how clearly it beats the runner-up: a close call, or a
//! menu where nothing fits, comes back unsure, for a layer above to settle.
//! With no layer above, the pick stands all the same.
//!
//! A middling temperament (0.5 in everything) plays exactly as C1.1's
//! citizens did. Away from the middle:
//! - **patience** sells and banks in bigger lots: at `SELL_AT` and
//!   `BANK_FIBER_AT` times 0.5 to 1.5;
//! - **thrift** weighs a price against the last trade, more the thriftier
//!   above the middle (a bid under it puts a seller off, an ask over it a
//!   buyer, and a good one draws them on), gathers rather than buy, and
//!   buys smaller lots. A middling citizen ignores the last trade, as C1.1's
//!   did: the sim showed that weighing it at all sent wood gatherers to the
//!   hall half as often again, on bids a Strand over a 1-Strand trade;
//! - **risk** asks above the bid, up to halfway to the ask;
//! - **sociability** passes free time at the hall rather than resting.

use super::{Choice, Decider, Decision, Lot, Question, Quote, Situation, Verdict};
use crate::sim::{BANK_FIBER_AT, LOT, SELL_AT};

/// What "what the rules would do" is worth.
const SURE: f64 = 0.9;
/// Each later goal on a work menu is worth this much less than the one
/// before: the role's order is its priority.
const GOAL_STEP: f64 = 0.15;
/// A lead this big over the runner-up is a clear choice; less is a close one.
const CLEAR: f64 = 0.2;
/// How far a price's fairness can raise or lower a utility.
const FAIRNESS: (f64, f64) = (0.0, 1.2);

/// The rules layer. It keeps no state: the same situation gets the same
/// answer, which is what lets C3.1 replay a log exactly.
#[derive(Debug, Clone, Copy, Default)]
pub struct RulesDecider;

impl Decider for RulesDecider {
    fn name(&self) -> &str {
        "rules"
    }

    fn decide(&mut self, s: &Situation) -> Verdict {
        let t = &s.who.temperament;
        let mut goals = 0;
        let mut scored: Vec<(usize, f64, Decision)> = Vec::with_capacity(s.menu.len());
        for (i, choice) in s.menu.iter().enumerate() {
            let (u, d) = match choice {
                Choice::Work { .. } => {
                    goals += 1;
                    let u = (SURE - GOAL_STEP * (goals - 1) as f64).max(0.1);
                    (u, Decision::plain(i))
                }
                Choice::Bank => {
                    let fiber = s.holdings.count(crate::exchange::FIBER);
                    (SURE * fiber as f64 / lot(BANK_FIBER_AT, t.patience), Decision::plain(i))
                }
                Choice::Sell { item, price, quantity } => {
                    let book = quote(s, item);
                    let bid = book.and_then(|q| q.bid).unwrap_or(price.min);
                    let fair = book.and_then(|q| q.last).map_or(1.0, |last| fairness(bid, last, t.thrift));
                    let held = s.holdings.count(item);
                    let u = SURE * held as f64 / lot(SELL_AT, t.patience) * fair;
                    // A taker of risks asks up toward the ask
                    let reach = (t.risk as f64 - 0.5).max(0.0);
                    let ask = book.and_then(|q| q.ask).unwrap_or(bid).max(bid);
                    let p = bid + ((ask - bid) as f64 * reach) as i64;
                    let d = Decision {
                        pick: i,
                        price: Some(price.clamp(p)),
                        quantity: Some(quantity.clamp(held)),
                    };
                    (u, d)
                }
                Choice::Keep => (SURE, Decision::plain(i)),
                Choice::Buy { item, price, quantity } => {
                    let book = quote(s, item);
                    let ask = book.and_then(|q| q.ask).unwrap_or(price.max);
                    // Paying over the last trade is unfair to the buyer
                    let fair = book.and_then(|q| q.last).map_or(1.0, |last| fairness(last, ask, t.thrift));
                    let want = (LOT as f64 * (1.5 - t.thrift as f64)).round() as i32;
                    let d = Decision {
                        pick: i,
                        price: Some(price.clamp(ask)),
                        quantity: Some(quantity.clamp(want)),
                    };
                    (SURE * fair, d)
                }
                Choice::Gather { .. } => (0.5 + 0.4 * t.thrift as f64, Decision::plain(i)),
                Choice::Accept | Choice::Counter { .. } | Choice::Decline => trade(s, i, choice),
                Choice::Rest => (0.8 - 0.2 * t.sociability as f64, Decision::plain(i)),
                Choice::Hall => (0.4 + 0.4 * t.sociability as f64, Decision::plain(i)),
            };
            scored.push((i, u, d));
        }
        pick(scored)
    }
}

/// The best entry, and how sure of it to be.
fn pick(scored: Vec<(usize, f64, Decision)>) -> Verdict {
    let mut order: Vec<&(usize, f64, Decision)> = scored.iter().collect();
    // Stable, so the earlier entry wins a tie
    order.sort_by(|a, b| b.1.total_cmp(&a.1));
    let Some(&&(_, top, choice)) = order.first() else {
        return Verdict::Escalate {
            reason: "the menu is empty".into(),
        };
    };
    let lead = order.get(1).map_or(1.0, |second| ((top - second.1) / CLEAR).min(1.0));
    let confidence = top.clamp(0.0, 1.0) * (0.5 + 0.5 * lead);
    Verdict::Decided {
        choice,
        confidence: confidence as f32,
    }
}

/// `base` scaled by patience: half as many for the hasty, half again for
/// the patient.
fn lot(base: i32, patience: f32) -> f64 {
    (base as f64 * (0.5 + patience as f64)).max(1.0)
}

fn quote<'a>(s: &'a Situation, item: &str) -> Option<&'a Quote> {
    s.market.iter().find(|q| q.item == item)
}

/// How a price looks beside the last trade, as thrift sees it: `got` over
/// `paid` (a sale's bid over the last trade, the last trade over a
/// purchase's ask), to a power that's 0 at middling thrift and 2 at full.
fn fairness(got: i64, paid: i64, thrift: f32) -> f64 {
    let weight = 4.0 * (thrift as f64 - 0.5).max(0.0);
    if paid <= 0 || weight == 0.0 {
        return 1.0;
    }
    (got as f64 / paid as f64).powf(weight).clamp(FAIRNESS.0, FAIRNESS.1)
}

/// What an item is worth by the citizen's books: the last trade, else the
/// middle of the book, else either side.
fn worth(s: &Situation, item: &str) -> Option<i64> {
    let q = quote(s, item)?;
    q.last.or(match (q.bid, q.ask) {
        (Some(b), Some(a)) => Some((a + b) / 2),
        (b, a) => b.or(a),
    })
}

fn value(s: &Situation, lots: &[Lot]) -> f64 {
    lots.iter()
        .map(|l| l.quantity as f64 * worth(s, &l.item).unwrap_or(0) as f64)
        .sum()
}

/// A P2P trade, valued at the citizen's books: what it gets over what it
/// gives, against the share a fair deal should be. Thrift wants more,
/// sociability will take a little less. A counter asks for the shortfall.
fn trade(s: &Situation, i: usize, choice: &Choice) -> (f64, Decision) {
    let Question::Trade {
        theirs,
        their_strands,
        ours,
        our_strands,
        ..
    } = &s.question
    else {
        // A trade's answers on another question's menu fit nothing
        return (0.0, Decision::plain(i));
    };
    let t = &s.who.temperament;
    let get = value(s, theirs) + *their_strands as f64;
    let give = value(s, ours) + *our_strands as f64;
    let wants = 0.9 + 0.2 * t.thrift as f64 - 0.1 * (t.sociability as f64 - 0.5);
    let q = if give <= 0.0 { 2.0 } else { get / (give * wants) };
    match choice {
        Choice::Accept => (SURE * q.min(1.2).powi(4), Decision::plain(i)),
        Choice::Counter { strands } => {
            let short = (give * wants - get).ceil().max(1.0) as i64;
            let u = if (0.6..1.0).contains(&q) { 0.75 } else { 0.1 };
            let d = Decision {
                pick: i,
                price: Some(strands.clamp(short)),
                quantity: None,
            };
            (u, d)
        }
        _ => (if q < 0.6 { SURE } else { 0.3 }, Decision::plain(i)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Held, Place, Span, Temperament, When, Who};
    use super::*;

    fn situation(question: Question, menu: Vec<Choice>, t: Temperament) -> Situation {
        Situation {
            who: Who {
                name: "Percy Sawyer".into(),
                role: "gatherer-wood".into(),
                temperament: t,
            },
            at: Place {
                x: 0.0,
                y: 0.0,
                zone: "rural".into(),
            },
            when: When {
                time_of_day: None,
                hours: 0.0,
            },
            holdings: Held::default(),
            market: Vec::new(),
            question,
            menu,
            carry_on: None,
        }
    }

    fn decide(s: &Situation) -> (Decision, f32) {
        match RulesDecider.decide(s) {
            Verdict::Decided { choice, confidence } => (choice, confidence),
            other => panic!("{other:?}"),
        }
    }

    fn work(n: usize) -> Vec<Choice> {
        (0..n)
            .map(|i| Choice::Work {
                goal: format!("g{i}"),
                step: format!("s{i}"),
            })
            .collect()
    }

    fn with(f: impl FnOnce(&mut Temperament)) -> Temperament {
        let mut t = Temperament::default();
        f(&mut t);
        t
    }

    #[test]
    fn the_roles_own_goal_comes_first_and_surely() {
        let (d, conf) = decide(&situation(Question::Work, work(3), Temperament::default()));
        assert_eq!(d, Decision::plain(0));
        assert!(conf >= super::super::RULES_THRESHOLD, "{conf}");
        let (_, alone) = decide(&situation(Question::Work, work(1), Temperament::default()));
        assert!(alone > conf);
    }

    fn fiber(n: i32, t: Temperament) -> Situation {
        let mut menu = vec![Choice::Bank];
        menu.extend(work(1));
        let mut s = situation(Question::Work, menu, t);
        s.holdings.items.insert("fiber".into(), n);
        s
    }

    #[test]
    fn a_middling_fiber_gatherer_banks_at_200_as_the_sim_does() {
        let t = Temperament::default();
        assert_eq!(decide(&fiber(BANK_FIBER_AT - 1, t)).0.pick, 1);
        assert_eq!(decide(&fiber(BANK_FIBER_AT, t)).0.pick, 0);
        // Right at the line it's a close call; well past it, a clear one
        assert!(decide(&fiber(BANK_FIBER_AT, t)).1 < super::super::RULES_THRESHOLD);
        assert!(decide(&fiber(2 * BANK_FIBER_AT, t)).1 >= 0.9);
        // A patient one waits for more, a hasty one less
        assert_eq!(decide(&fiber(BANK_FIBER_AT, with(|t| t.patience = 1.0))).0.pick, 1);
        assert_eq!(decide(&fiber(BANK_FIBER_AT / 2, with(|t| t.patience = 0.0))).0.pick, 0);
    }

    fn sell(held: i32, bid: i64, ask: Option<i64>, last: Option<i64>, t: Temperament) -> Situation {
        let menu = vec![
            Choice::Sell {
                item: "log".into(),
                price: Span::new(bid / 2, bid * 2),
                quantity: Span::new(1, held),
            },
            Choice::Keep,
        ];
        let mut s = situation(Question::Sell { item: "log".into() }, menu, t);
        s.holdings.items.insert("log".into(), held);
        s.market.push(Quote {
            item: "log".into(),
            bid: Some(bid),
            ask,
            last,
        });
        s
    }

    #[test]
    fn a_middling_seller_sells_all_at_the_bid_from_sell_at() {
        let t = Temperament::default();
        let (d, _) = decide(&sell(SELL_AT, 7, Some(9), None, t));
        assert_eq!(d, Decision { pick: 0, price: Some(7), quantity: Some(SELL_AT) });
        assert_eq!(decide(&sell(SELL_AT - 1, 7, Some(9), None, t)).0.pick, 1);
        // The patient hold on
        assert_eq!(decide(&sell(SELL_AT, 7, None, None, with(|t| t.patience = 0.9))).0.pick, 1);
    }

    #[test]
    fn thrift_holds_out_against_a_low_bid_and_risk_asks_above_it() {
        // A bid well under the last trade: the thrifty keep, the careless sell
        let low = |thrift| sell(2 * SELL_AT, 5, Some(9), Some(10), with(|t| t.thrift = thrift));
        assert_eq!(decide(&low(1.0)).0.pick, 1);
        assert_eq!(decide(&low(0.0)).0.pick, 0);
        // Middling, the last trade counts for nothing either way, as in C1.1
        let middling = Temperament::default();
        assert_eq!(decide(&sell(2 * SELL_AT, 5, Some(9), Some(10), middling)).0.pick, 0);
        assert_eq!(decide(&sell(SELL_AT - 2, 2, Some(3), Some(1), middling)).0.pick, 1);
        // The bold ask halfway to the ask
        let (d, _) = decide(&sell(SELL_AT, 6, Some(10), None, with(|t| t.risk = 1.0)));
        assert_eq!(d.price, Some(8));
        let (d, _) = decide(&sell(SELL_AT, 6, Some(10), None, with(|t| t.risk = 0.2)));
        assert_eq!(d.price, Some(6));
    }

    fn buy(ask: i64, last: Option<i64>, afford: i32, t: Temperament) -> Situation {
        let menu = vec![
            Choice::Buy {
                item: "log".into(),
                price: Span::new((ask / 2).max(1), ask),
                quantity: Span::new(1, afford),
            },
            Choice::Gather { item: "log".into() },
        ];
        let mut s = situation(Question::Buy { item: "log".into() }, menu, t);
        s.market.push(Quote {
            item: "log".into(),
            bid: None,
            ask: Some(ask),
            last,
        });
        s
    }

    #[test]
    fn a_middling_buyer_buys_a_lot_at_the_ask() {
        let (d, conf) = decide(&buy(8, None, 2 * LOT, Temperament::default()));
        assert_eq!(d, Decision { pick: 0, price: Some(8), quantity: Some(LOT) });
        assert!(conf >= super::super::RULES_THRESHOLD, "{conf}");
        // As much as it can afford, if that's less
        assert_eq!(decide(&buy(8, None, 3, Temperament::default())).0.quantity, Some(3));
        // The thrifty buy less, and gather rather than pay over the odds
        assert_eq!(decide(&buy(8, None, 2 * LOT, with(|t| t.thrift = 0.8))).0.quantity, Some(7));
        assert_eq!(decide(&buy(12, Some(8), 2 * LOT, with(|t| t.thrift = 0.8))).0.pick, 1);
        assert_eq!(decide(&buy(12, Some(8), 2 * LOT, with(|t| t.thrift = 0.1))).0.pick, 0);
    }

    #[test]
    fn free_time_is_a_rest_unless_the_citizen_likes_company() {
        let menu = || vec![Choice::Rest, Choice::Hall];
        assert_eq!(decide(&situation(Question::FreeTime, menu(), Temperament::default())).0.pick, 0);
        let social = with(|t| t.sociability = 0.9);
        assert_eq!(decide(&situation(Question::FreeTime, menu(), social)).0.pick, 1);
    }

    fn offer(theirs: i64, ours: i32, t: Temperament) -> Situation {
        let q = Question::Trade {
            partner: "Someone".into(),
            partner_citizen: false,
            theirs: Vec::new(),
            their_strands: theirs,
            ours: vec![Lot {
                item: "log".into(),
                quantity: ours,
            }],
            our_strands: 0,
        };
        let menu = vec![
            Choice::Accept,
            Choice::Counter {
                strands: Span::new(1, 1000),
            },
            Choice::Decline,
        ];
        let mut s = situation(q, menu, t);
        s.market.push(Quote {
            item: "log".into(),
            bid: Some(9),
            ask: Some(11),
            last: Some(10),
        });
        s
    }

    #[test]
    fn a_fair_offer_is_taken_a_near_one_countered_a_poor_one_declined() {
        let t = Temperament::default();
        // Ten logs at the last trade's 10
        assert_eq!(decide(&offer(100, 10, t)).0.pick, 0);
        assert_eq!(decide(&offer(130, 10, t)).0.pick, 0);
        let (d, _) = decide(&offer(80, 10, t));
        assert_eq!(d, Decision { pick: 1, price: Some(20), quantity: None });
        assert_eq!(decide(&offer(30, 10, t)).0.pick, 2);
        // A gift is always welcome
        assert_eq!(decide(&offer(50, 0, t)).0.pick, 0);
    }

    #[test]
    fn a_close_call_comes_back_unsure() {
        // Gathering and buying at a slightly dear price, thrifty: close
        let s = buy(9, Some(8), 2 * LOT, with(|t| t.thrift = 0.7));
        let (_, conf) = decide(&s);
        assert!(conf < super::super::RULES_THRESHOLD, "{conf}");
    }
}
