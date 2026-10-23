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
//! - **sociability** passes free time at the hall rather than resting;
//! - **patience**, again, earns a bigger float before a merchant makes a
//!   market (C1.6): `MERCHANT_FLOAT` times 0.5 to 1.5. One already making a
//!   market keeps at it down to half that, so it doesn't flit between the
//!   hall and the fields at the line.
//!
//! Idle life (C2.1): free time's other places are weighed by temperament
//! and a whim drawn from the situation ([`whim`]): the bazaar's lane for
//! the sociable, a park for the rest, and a bridge at dusk or its own stall
//! for the patient. Going straight home at dusk is a steady 0.75.
//!
//! People (C1.5): a standing bid is at the books' price (under it for the
//! thrifty, who'd sooner gather), a stall's price above it (more for the
//! bold), and a P2P trade is weighed whole at the books ([`trade`]).
//!
//! Asking (#295): a citizen lingering among others asks one who makes what
//! its trade needs to sell it some, or one whose trade needs what it makes
//! to buy, the more readily the more sociable (a thrifty buyer a little
//! less), against resting where it is. It asks for what it would bid for at
//! the books, and offers all it holds.
//!
//! The road in (#450): its next step is worth more than anything a citizen
//! was doing ([`ROAD`]), since at its end is a deed, its home if it has
//! none. On the road's question everything else is putting it off, and
//! worth [`PUT_OFF`] of what it would be, so the rules settle it clearly.

use super::{Choice, Decider, Decision, Lot, Question, Quote, Situation, Verdict};
use crate::sim::{BANK_FIBER_AT, LOT, MERCHANT_FLOAT, SELL_AT, STALL_AT};

/// What "what the rules would do" is worth.
const SURE: f64 = 0.9;
/// Each later goal on a work menu is worth this much less than the one
/// before: the role's order is its priority.
const GOAL_STEP: f64 = 0.15;
/// A lead this big over the runner-up is a clear choice; less is a close one.
const CLEAR: f64 = 0.2;
/// How far a price's fairness can raise or lower a utility.
const FAIRNESS: (f64, f64) = (0.0, 1.2);
/// How far a whim moves a place's appeal (C2.1).
const WHIM: f64 = 0.12;
/// What the road in's next step is worth (#450).
const ROAD: f64 = 1.0;
/// What anything else on the road's menu is worth, of what it would be.
const PUT_OFF: f64 = 0.6;

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
                Choice::Bid { item, price, quantity } => {
                    // A standing bid at the books' price, a little under it
                    // for the thrifty; they'd sooner gather it themselves.
                    let w = worth(s, item).unwrap_or(price.min) as f64;
                    let p = (w * (1.15 - 0.3 * t.thrift as f64)).round() as i64;
                    let want = (LOT as f64 * (1.5 - t.thrift as f64)).round() as i32;
                    let d = Decision {
                        pick: i,
                        price: Some(price.clamp(p.max(1))),
                        quantity: Some(quantity.clamp(want)),
                    };
                    (0.95 - 0.2 * t.thrift as f64, d)
                }
                Choice::Stock { item, price, quantity } => {
                    // A stall's price is the books' and a little more, more
                    // for the bold: someone who wants it now pays for that.
                    let w = worth(s, item).unwrap_or(price.max) as f64;
                    let p = (w * (1.05 + 0.3 * t.risk as f64)).round() as i64;
                    let held = s.holdings.count(item);
                    let d = Decision {
                        pick: i,
                        price: Some(price.clamp(p.max(1))),
                        quantity: Some(quantity.clamp(held)),
                    };
                    (SURE * held as f64 / lot(STALL_AT, t.patience), d)
                }
                Choice::Gather { .. } => (0.5 + 0.4 * t.thrift as f64, Decision::plain(i)),
                Choice::Accept | Choice::Counter { .. } | Choice::Pay { .. } | Choice::Decline => trade(s, i, choice),
                Choice::Rest => (0.8 - 0.2 * t.sociability as f64, Decision::plain(i)),
                Choice::Hall => (0.4 + 0.4 * t.sociability as f64, Decision::plain(i)),
                Choice::MakeMarket { float, making, .. } => {
                    let enough = lot(MERCHANT_FLOAT as i32, t.patience) * if *making { 0.5 } else { 1.0 };
                    (SURE * *float as f64 / enough, Decision::plain(i))
                }
                // Idle life (C2.1): the company of the lane for the sociable,
                // a park's quiet for the rest, a bridge at dusk and its own
                // stall for the patient, each as the whim takes it
                Choice::Lane => (0.5 + 0.35 * t.sociability as f64 + whim(s, i), Decision::plain(i)),
                Choice::Park { .. } => (0.6 + 0.25 * (1.0 - t.sociability as f64) + whim(s, i), Decision::plain(i)),
                Choice::Bridge { .. } => (0.65 + 0.2 * t.patience as f64 + whim(s, i), Decision::plain(i)),
                Choice::Stall { .. } => (0.55 + 0.3 * t.patience as f64 + whim(s, i), Decision::plain(i)),
                Choice::Home => (0.75, Decision::plain(i)),
                // Asking a neighbour (#295): for a lot at the books' price,
                // or to take all it holds; more readily the more sociable
                Choice::BuyFrom { item, price, quantity, .. } => {
                    let w = worth(s, item).unwrap_or(price.min);
                    let want = (LOT as f64 * (1.5 - t.thrift as f64)).round() as i32;
                    let d = Decision {
                        pick: i,
                        price: Some(price.clamp(w.max(1))),
                        quantity: Some(quantity.clamp(want)),
                    };
                    (0.8 + 0.25 * t.sociability as f64 - 0.2 * t.thrift as f64, d)
                }
                Choice::SellTo { item, quantity, .. } => {
                    let d = Decision {
                        pick: i,
                        price: None,
                        quantity: Some(quantity.clamp(s.holdings.count(item))),
                    };
                    (0.65 + 0.25 * t.sociability as f64, d)
                }
                Choice::Road { .. } => (ROAD, Decision::plain(i)),
            };
            // On the road's question, anything else puts the road off (#450)
            let road = matches!(s.question, Question::Road { .. }) && !matches!(choice, Choice::Road { .. });
            let u = if road { u * PUT_OFF } else { u };
            scored.push((i, u, d));
        }
        pick(scored)
    }
}

/// How much more or less a place appeals today: up to [`WHIM`] either
/// way, from who's asking, the quarter-hour and the entry. It's drawn from
/// the situation alone, so a replay of it answers the same (C3.1), but the
/// same citizen goes somewhere else an hour later.
fn whim(s: &Situation, entry: usize) -> f64 {
    let quarter = s.when.time_of_day.map_or((s.when.hours * 4.0) as i64, |t| (t * 96.0) as i64);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.who.name.bytes().chain(quarter.to_le_bytes()).chain((entry as u64).to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    // SplitMix's finish, so neighbouring quarters differ
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    let unit = (h >> 11) as f64 / (1u64 << 53) as f64;
    (unit * 2.0 - 1.0) * WHIM
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
/// middle of the book, else either side. `None`: it can't say, and won't
/// sell it in a P2P trade ([`meet`](crate::citizen::meet)).
pub(crate) fn worth(s: &Situation, item: &str) -> Option<i64> {
    worth_in(&s.market, item)
}

/// [`worth`] from the books alone.
pub(crate) fn worth_in(market: &[Quote], item: &str) -> Option<i64> {
    let q = market.iter().find(|q| q.item == item)?;
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

/// What a citizen wants back for every Strand's worth it gives: about 1,
/// more if thrifty, a little less if sociable.
pub(crate) fn wants(t: &super::Temperament) -> f64 {
    0.9 + 0.2 * t.thrift as f64 - 0.1 * (t.sociability as f64 - 0.5)
}

/// A P2P trade, valued at the citizen's books: what it gets over what it
/// gives, against the share a fair deal should be ([`wants`]).
///
/// - **Accept** a fair deal, or better.
/// - **Counter** a short one, asking for the shortfall, when the counter
///   can close it: its span's top is the most the citizen can take off its
///   side and still give something. So "30 for my ten logs at 10" is
///   answered "30 gets you three", however far short it is.
/// - **Decline** a short one it can't counter: it has countered already,
///   or the offer buys less than the least it would give.
/// - **Pay** for goods offered: their worth at its books over what it wants.
///   Goods it can't put a worth to, it declines.
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
    let wants = wants(&s.who.temperament);
    let get = value(s, theirs) + *their_strands as f64;
    let give = value(s, ours) + *our_strands as f64;
    let q = if give <= 0.0 { 2.0 } else { get / (give * wants) };
    // Less a hair, so 1.1 × 100 is 110 and not 111
    let short = (give * wants - get - 1e-9).ceil().max(1.0) as i64;
    // Whether a counter on the menu could close the gap
    let closes = s.menu.iter().any(|c| matches!(c, Choice::Counter { strands } if short <= strands.max));
    match choice {
        Choice::Accept => (SURE * q.min(1.2).powi(4), Decision::plain(i)),
        Choice::Counter { strands } => {
            let u = if q < 1.0 && short <= strands.max { 0.75 } else { 0.1 };
            let d = Decision {
                pick: i,
                price: Some(strands.clamp(short)),
                quantity: None,
            };
            (u, d)
        }
        Choice::Pay { strands } => {
            let fair = (value(s, theirs) / wants).floor() as i64;
            let u = if fair >= strands.min { SURE } else { 0.1 };
            let d = Decision {
                pick: i,
                price: Some(strands.clamp(fair)),
                quantity: None,
            };
            (u, d)
        }
        _ => (if q < 0.6 && !closes { SURE } else { 0.3 }, Decision::plain(i)),
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
    fn the_road_ins_next_step_is_taken_clearly_over_the_errand() {
        let road = Choice::Road {
            step: "twist_rope".into(),
            chapter: "arrival".into(),
            does: "twist a rope".into(),
        };
        let question = Question::Road { step: "twist_rope".into() };
        for t in [Temperament::default(), with(|t| t.patience = 1.0), with(|t| t.sociability = 1.0)] {
            for other in [work(1).remove(0), Choice::Rest, Choice::Bank] {
                let (d, confidence) = decide(&situation(question.clone(), vec![other.clone(), road.clone()], t));
                assert_eq!(d, Decision::plain(1), "{other:?}");
                // Sure enough that the rules settle it, under Jev's 0.9
                assert!(confidence >= 0.9, "{other:?}: {confidence}");
            }
        }
        // On any other question the road's entries change nothing else
        let (d, _) = decide(&situation(Question::FreeTime, vec![Choice::Rest, Choice::Hall], Temperament::default()));
        assert_eq!(d, Decision::plain(0));
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

    #[test]
    fn a_merchant_makes_a_market_once_its_float_is_earned_and_keeps_at_it() {
        let menu = |float: i64, making: bool| {
            vec![
                Choice::MakeMarket {
                    items: vec!["log".into()],
                    float,
                    making,
                },
                Choice::Work {
                    goal: "more fiber".into(),
                    step: "gather fiber".into(),
                },
            ]
        };
        let pick = |float, making, t| decide(&situation(Question::Market, menu(float, making), t)).0.pick;
        let middling = Temperament::default();
        assert_eq!(pick(MERCHANT_FLOAT - 1, false, middling), 1, "still earning");
        assert_eq!(pick(MERCHANT_FLOAT, false, middling), 0);
        // At it already, it keeps on down to half the float
        assert_eq!(pick(MERCHANT_FLOAT / 2 + 1, true, middling), 0);
        assert_eq!(pick(MERCHANT_FLOAT / 2 - 10, true, middling), 1);
        // The impatient start sooner, the patient later
        assert_eq!(pick(MERCHANT_FLOAT * 6 / 10, false, with(|t| t.patience = 0.0)), 0);
        assert_eq!(pick(MERCHANT_FLOAT * 12 / 10, false, with(|t| t.patience = 1.0)), 1);
    }

    #[test]
    fn free_time_goes_where_the_temperament_and_the_whim_say() {
        let menu = || {
            vec![
                Choice::Rest,
                Choice::Hall,
                Choice::Lane,
                Choice::Park { park: "park_0_0_2".into() },
                Choice::Stall { stall: "stall_3".into() },
            ]
        };
        let picks = |t: Temperament| {
            let mut picks = [0; 5];
            for quarter in 0..96 {
                let mut s = situation(Question::FreeTime, menu(), t);
                s.when.time_of_day = Some(quarter as f64 / 96.0);
                picks[decide(&s).0.pick] += 1;
            }
            picks
        };
        // A middling citizen goes about: never always the same place
        let middling = picks(Temperament::default());
        assert!(middling.iter().filter(|&&n| n > 0).count() >= 3, "{middling:?}");
        // The sociable to the lane, the shy to the park, the patient to their stall
        let lane = picks(with(|t| t.sociability = 1.0));
        assert!(lane[2] > lane[3] && lane[2] > lane[0], "{lane:?}");
        let park = picks(with(|t| t.sociability = 0.0));
        assert!(park[3] > park[2] && park[3] > park[1], "{park:?}");
        let stall = picks(with(|t| {
            t.patience = 1.0;
            t.sociability = 0.3;
        }));
        assert!(stall[4] > middling[4], "{stall:?} against {middling:?}");
        // The same situation always answers the same, for C3.1's replay
        let s = situation(Question::FreeTime, menu(), Temperament::default());
        assert_eq!(decide(&s), decide(&s));
    }

    #[test]
    fn at_dusk_the_patient_stop_on_the_bridge() {
        let menu = || vec![Choice::Home, Choice::Bridge { bridge: "canal_inner@90".into() }];
        let stops = |t: Temperament| {
            (0..96)
                .filter(|&q| {
                    let mut s = situation(Question::FreeTime, menu(), t);
                    s.when.time_of_day = Some(q as f64 / 96.0);
                    decide(&s).0.pick == 1
                })
                .count()
        };
        let (hasty, patient) = (stops(with(|t| t.patience = 0.0)), stops(with(|t| t.patience = 1.0)));
        assert!(hasty < 48 && patient > 48 && patient > hasty, "{hasty} against {patient}");
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
        // A counter may take off all but one log's worth
        let menu = vec![
            Choice::Accept,
            Choice::Counter {
                strands: Span::new(1, (ours as i64 - 1).max(1) * 10),
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
    fn a_fair_offer_is_taken_a_short_one_countered_a_poor_one_declined() {
        let t = Temperament::default();
        // Ten logs at the last trade's 10
        assert_eq!(decide(&offer(100, 10, t)).0.pick, 0);
        assert_eq!(decide(&offer(130, 10, t)).0.pick, 0);
        let (d, _) = decide(&offer(80, 10, t));
        assert_eq!(d, Decision { pick: 1, price: Some(20), quantity: None });
        // However short, a counter that leaves a log to give is made
        let (d, _) = decide(&offer(30, 10, t));
        assert_eq!(d, Decision { pick: 1, price: Some(70), quantity: None });
        // Less than a log's worth can't be countered
        assert_eq!(decide(&offer(5, 10, t)).0.pick, 2);
        // A gift is always welcome
        assert_eq!(decide(&offer(50, 0, t)).0.pick, 0);
    }

    #[test]
    fn with_its_counter_spent_a_short_offer_is_declined_or_taken_if_near() {
        let t = Temperament::default();
        let spent = |theirs| {
            let mut s = offer(theirs, 10, t);
            s.menu.remove(1);
            s
        };
        assert_eq!(decide(&spent(30)).0.pick, 1, "declined");
        assert_eq!(decide(&spent(90)).0.pick, 0, "near enough");
        // The thrifty want more for the same logs
        let thrifty = with(|t| t.thrift = 1.0);
        let (d, _) = decide(&offer(100, 10, thrifty));
        assert_eq!(d, Decision { pick: 1, price: Some(10), quantity: None });
    }

    fn goods(logs: i32, afford: i64, t: Temperament) -> Situation {
        let q = Question::Trade {
            partner: "Someone".into(),
            partner_citizen: false,
            theirs: vec![Lot {
                item: "log".into(),
                quantity: logs,
            }],
            their_strands: 0,
            ours: Vec::new(),
            our_strands: 0,
        };
        let menu = vec![
            Choice::Pay {
                strands: Span::new(1, afford),
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
    fn goods_offered_are_paid_for_at_the_books() {
        let (d, conf) = decide(&goods(6, 1000, Temperament::default()));
        assert_eq!(d, Decision { pick: 0, price: Some(60), quantity: None });
        assert!(conf >= super::super::RULES_THRESHOLD, "{conf}");
        // As much as it has; the thrifty pay less
        assert_eq!(decide(&goods(6, 40, Temperament::default())).0.price, Some(40));
        assert_eq!(decide(&goods(6, 1000, with(|t| t.thrift = 1.0))).0.price, Some(54));
        // Goods it can put no worth to, it declines
        let mut s = goods(6, 1000, Temperament::default());
        s.market.clear();
        assert_eq!(decide(&s).0.pick, 1);
    }

    #[test]
    fn a_standing_bid_is_at_the_books_and_a_stall_above_them() {
        let bid = |t| {
            let menu = vec![
                Choice::Bid {
                    item: "log".into(),
                    price: Span::new(1, 40),
                    quantity: Span::new(1, 30),
                },
                Choice::Gather { item: "log".into() },
            ];
            let mut s = situation(Question::Buy { item: "log".into() }, menu, t);
            s.market.push(Quote {
                item: "log".into(),
                bid: None,
                ask: None,
                last: Some(10),
            });
            s
        };
        let (d, conf) = decide(&bid(Temperament::default()));
        assert_eq!(d, Decision { pick: 0, price: Some(10), quantity: Some(LOT) });
        assert!(conf >= super::super::RULES_THRESHOLD, "{conf}");
        // The thrifty bid lower, and the thriftiest gather instead
        assert_eq!(decide(&bid(with(|t| t.thrift = 0.7))).0.price, Some(9));
        assert_eq!(decide(&bid(with(|t| t.thrift = 1.0))).0.pick, 1);

        let stall = |held, t| {
            let menu = vec![
                Choice::Stock {
                    item: "log".into(),
                    price: Span::new(1, 100),
                    quantity: Span::new(1, held),
                },
                Choice::Keep,
            ];
            let mut s = situation(Question::Sell { item: "log".into() }, menu, t);
            s.holdings.items.insert("log".into(), held);
            s.market.push(Quote {
                item: "log".into(),
                bid: None,
                ask: None,
                last: Some(10),
            });
            s
        };
        let (d, _) = decide(&stall(2 * STALL_AT, Temperament::default()));
        assert_eq!(d, Decision { pick: 0, price: Some(12), quantity: Some(2 * STALL_AT) });
        assert_eq!(decide(&stall(STALL_AT - 1, Temperament::default())).0.pick, 1);
        assert_eq!(decide(&stall(2 * STALL_AT, with(|t| t.risk = 0.0))).0.price, Some(11));
    }

    #[test]
    fn a_sociable_citizen_asks_its_neighbour_and_a_shy_one_rests() {
        let ask = |t, selling: bool| {
            let mut menu = vec![
                Choice::BuyFrom {
                    partner: "Ada Woodward".into(),
                    item: "log".into(),
                    price: Span::new(5, 15),
                    quantity: Span::new(1, 20),
                },
                Choice::Rest,
            ];
            if selling {
                menu.insert(
                    1,
                    Choice::SellTo {
                        partner: "Ivor Mason".into(),
                        item: "plank".into(),
                        quantity: Span::new(1, 14),
                    },
                );
            }
            let mut s = situation(Question::Ask, menu, t);
            s.holdings.items.insert("plank".into(), 14);
            s.market.push(Quote {
                item: "log".into(),
                bid: None,
                ask: None,
                last: Some(10),
            });
            s
        };
        // Middling: a lot of logs at the books, surely enough
        let (d, conf) = decide(&ask(Temperament::default(), false));
        assert_eq!(d, Decision { pick: 0, price: Some(10), quantity: Some(LOT) });
        assert!(conf >= super::super::RULES_THRESHOLD, "{conf}");
        assert_eq!(decide(&ask(Temperament::default(), true)).0.pick, 0);
        // Thrifty: sells before it buys, and offers all it holds
        let thrifty = with(|t| t.thrift = 1.0);
        assert_eq!(decide(&ask(thrifty, true)).0, Decision { pick: 1, price: None, quantity: Some(14) });
        // Shy: leaves them be
        assert_eq!(decide(&ask(with(|t| t.sociability = 0.0), true)).0.pick, 2);
    }

    #[test]
    fn a_close_call_comes_back_unsure() {
        // Gathering and buying at a slightly dear price, thrifty: close
        let s = buy(9, Some(8), 2 * LOT, with(|t| t.thrift = 0.7));
        let (_, conf) = decide(&s);
        assert!(conf < super::super::RULES_THRESHOLD, "{conf}");
    }
}
