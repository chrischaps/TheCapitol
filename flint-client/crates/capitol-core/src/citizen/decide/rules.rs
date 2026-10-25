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
//!
//! The board (Commons S2.5, [`commission`]): a claim already made is
//! delivered before anything else ([`DELIVER`]). A commission's price is
//! weighed against the books: a citizen claims one that beats the last
//! trade by its margin ([`margin`]: more for the thrifty, less for the
//! bold), and delivers what it holds to one that pays the books' price,
//! sooner than promise more.
//! Passing is a steady [`PASS`]. A crafter posts a commission for an input
//! its bids haven't filled, at the bid's price and a premium
//! ([`premium`]: less for the thrifty), the more readily the less thrifty.
//!
//! Wages (Commons S2.8, [`wage`]): work offered for its trade is taken when
//! it pays a game hour at least what the citizen's own trade earns at the
//! books, by its [`wage_margin`]: more for the thrifty, and for the bold,
//! who'd sooner their own trade; less for the careful, since a wage is a
//! sure thing. With nothing at the books to go by, the careful take it.
//!
//! Hiring (Callings K2.4, [`hire`]): a citizen with Strands over its line
//! offers work at a station or on a lot its stint leaves idle, the more
//! readily the further over the line it is, and the less thrifty. It pays
//! what the work earns its trade at the books (or, with none, the form's
//! half the target), more for the free-handed and less for the thrifty
//! ([`wage::hire_rate`]), for as many units as its Strands over the line
//! escrow. Passing is a steady [`PASS`].
//!
//! Guilds (Commons S4.5, [`guild`]): a citizen asks to join a guild for its
//! trade the more readily the more sociable, and one that invited it more
//! readily still. A middling citizen joins its trade's guild; a reserved
//! one stays out unless it's asked. Staying out is a steady [`PASS`].
//!
//! Stints (Callings K1.1, D4, [`stint`]): a job is worth a steady
//! [`STINT`], a little more the more its calling leans to it ([`LEAN`]),
//! and more the further its share of the citizen's recent work falls short
//! of its calling's share, by [`CHASE`]. Chasing the shares
//! rather than drawing at random makes them converge to the calling. A job
//! with setup still to do has its shortfall charged by [`setup_cost`], a
//! pull on it adds up to [`PULL`], and the job it's at is worth
//! [`MOMENTUM`] more until its stint's least length ([`least_stint`]).
//!
//! Detours (Callings K1.5, [`detour`]): on the stint menu a draw from the
//! situation ([`unit`], whim's hash) says whether the end of a stint turns
//! into a quarter hour's detour first, with a chance of the calling's
//! `wander` ([`DETOUR`]), a quarter of that while the job it's at still
//! has its momentum ([`DETOUR_HELD`]). One taken is worth a little more
//! than the best job on the menu, so the rules' shares are untouched, and
//! which of them by its kind's taste ([`DetourKind::taste`]) and the whim.
//! On the free-time menu it's one place more, weighed by `wander` and
//! taste against the hall, the lane and the park.
//!
//! Trading's share rises with wealth (K2.2): past its line, trading takes
//! up to [`RICH_RISE`](crate::sim::stint::RICH_RISE) of the rest of its
//! calling ([`stint::target`](crate::sim::stint::target)), so a rich
//! citizen chases trading the more.

use super::{Choice, Decider, Decision, Lot, Question, Quote, Situation, Span, Temperament, Verdict};
use crate::citizen::detour::DetourKind;
use crate::sim::{stint as stints, wage};
use crate::sim::{Role, BANK_FIBER_AT, LOT, MERCHANT_FLOAT, SELL_AT, STALL_AT};

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
/// What delivering is worth, on a claim of its own or at a fair price: more
/// than anything, so a promise is kept before the day's other work, and
/// goods in hand go before a promise of more (S2.5).
const DELIVER: f64 = 1.0;
/// What leaving a commission be is worth.
const PASS: f64 = 0.5;
/// What any job on the stint menu is worth before its share is chased.
const STINT: f64 = 0.5;
/// What each point of a job's share in the calling adds, whatever it has
/// done lately: so with every share met, the calling's own order breaks the
/// tie, and a job it has no lean to never ties one it has. Small, so the
/// shares a citizen keeps still come within a point or so of its calling's.
const LEAN: f64 = 0.01;
/// What each point of a job's shortfall against its calling's share adds:
/// a specialist starting its day with nothing done lately is sure of its
/// favourite, and a job 10 points short beats one 5 points short clearly.
const CHASE: f64 = 2.0;
/// The setup steps that halve what a shortfall adds ([`setup_cost`]): a
/// wood gatherer's bench and stone axe from empty hands (28 steps) take a
/// sixth off it, a sawbench with its plot and the kiln's bricks (about 500)
/// three quarters.
const SETUP_HALF: f64 = 150.0;
/// The most a pull on a job adds.
const PULL: f64 = 0.6;
/// Strands a game hour over the books at which a pull adds half of
/// [`PULL`]: about what the town's faucet pays a citizen-hour (K0.1).
const PULL_HALF: f64 = 150.0;
/// What staying at the job it's at is worth, until its stint's least
/// length.
const MOMENTUM: f64 = 0.5;
/// A middling citizen's stint, in game hours.
const STINT_HOURS: f64 = 1.5;
/// The chance a stint's end turns into a detour first, a point of
/// `wander`: a middling wanderer takes one at about every other stint's
/// end, so most citizens take one in four game hours, and it comes to
/// under a tenth of their work (K1.5).
const DETOUR: f64 = 1.0;
/// What's left of that chance while the job it's at still has its
/// [`MOMENTUM`]: a session that starts mid-stint mostly gets on with it.
const DETOUR_HELD: f64 = 0.25;
/// What a detour taken is worth over the best job on the stint menu, and
/// what its kind's taste adds to that, to choose between them.
const DETOUR_OVER: f64 = 0.05;
const DETOUR_TASTE: f64 = 0.1;
/// The draw's own salt, apart from every entry's whim.
const DETOUR_SALT: u64 = u64::MAX;

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
                Choice::MakeMarket { float, making, .. } => (SURE * *float as f64 / enough_float(t, *making), Decision::plain(i)),
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
                Choice::Claim { .. } | Choice::Deliver { .. } => commission(s, i, choice),
                // Passing is a steady PASS, on the board and a wage alike
                Choice::Pass => (PASS, Decision::plain(i)),
                Choice::TakeWork => wage(s, i),
                Choice::Hire { rate, cap } => hire(s, i, rate, cap),
                Choice::Apply { guild: name } => guild(s, i, name),
                Choice::Stay => (PASS, Decision::plain(i)),
                Choice::Take { job, setup, pays } => (stint(s, job, *setup, *pays), Decision::plain(i)),
                Choice::Detour { detour: kind, .. } => (detour(s, i, *kind), Decision::plain(i)),
                Choice::Post { price, quantity, .. } => {
                    // At its bid's price and a premium, for what it would bid for
                    let bid = price.min as f64;
                    let p = ((bid * premium(t)).round() as i64).max(price.min + 1);
                    let want = (LOT as f64 * (1.5 - t.thrift as f64)).round() as i32;
                    let d = Decision {
                        pick: i,
                        price: Some(price.clamp(p)),
                        quantity: Some(quantity.clamp(want)),
                    };
                    (0.6 + 0.3 * (1.0 - t.thrift as f64), d)
                }
            };
            // On the road's question, anything else puts the road off (#450)
            let road = matches!(s.question, Question::Road { .. }) && !matches!(choice, Choice::Road { .. });
            let u = if road { u * PUT_OFF } else { u };
            scored.push((i, u, d));
        }
        pick(scored)
    }
}

/// How long a stint runs before the citizen asks what next (Callings D4),
/// in game hours: [`STINT_HOURS`] times 0.5 for the hasty to 1.5 for the
/// patient.
pub fn stint_hours(t: &Temperament) -> f64 {
    STINT_HOURS * (0.5 + t.patience as f64)
}

/// The least a stint runs: half its length. Until then the job it's at is
/// worth [`MOMENTUM`] more, so a pull has to be worth leaving it for.
pub fn least_stint(t: &Temperament) -> f64 {
    stint_hours(t) / 2.0
}

/// The float a citizen makes a market with (C1.6): [`MERCHANT_FLOAT`] times
/// 0.5 for the hasty to 1.5 for the patient, and half that for one already
/// making a market, so it doesn't flit between the hall and the fields at
/// the line. Trading is on the stint menu of anyone leaning to it who has
/// this much (Callings K2.2).
pub fn enough_float(t: &Temperament, making: bool) -> f64 {
    lot(MERCHANT_FLOAT as i32, t.patience) * if making { 0.5 } else { 1.0 }
}

/// What a shortfall is charged for `steps` of setup still to do: 1 when the
/// job's ready, half at [`SETUP_HALF`], saturating, so a long chain is dear
/// but never off the menu.
pub fn setup_cost(steps: u32) -> f64 {
    SETUP_HALF / (SETUP_HALF + steps as f64)
}

/// Taking up `job` for a stint (Callings K1.1, D4):
///
/// ```text
/// u = STINT + LEAN · target
///           + CHASE · max(0, target − recent) · setup_cost(setup)
///           + pull(pays) + momentum
/// ```
///
/// `target` is the calling's share of its work for the job, trading's
/// raised by its Strands over its `line` (K2.2, [`stints::target`]), and
/// `recent` its share of the work done lately, all from the situation, so a
/// replay answers the same. A situation with no calling chases its role
/// alone.
fn stint(s: &Situation, job: &str, setup: u32, pays: i64) -> f64 {
    let Question::Stint { doing, run, lately, line, .. } = &s.question else {
        // A stint's answers on another question's menu fit nothing
        return 0.0;
    };
    let target = match (s.who.calling(), Role::from_slug(job)) {
        (Some(calling), Some(role)) => stints::target(&calling, role, s.holdings.strands, *line),
        _ => 0.0,
    };
    let done: f64 = lately.values().map(|&v| v.max(0.0) as f64).sum();
    let recent = if done > 0.0 { lately.get(job).map_or(0.0, |&v| v.max(0.0) as f64) / done } else { 0.0 };
    let chase = CHASE * (target - recent).max(0.0) * setup_cost(setup);
    let pull = if pays > 0 { PULL * pays as f64 / (pays as f64 + PULL_HALF) } else { 0.0 };
    let at_it = doing.as_deref() == Some(job) && f64::from(*run) < least_stint(&s.who.temperament);
    STINT + LEAN * target + chase + pull + if at_it { MOMENTUM } else { 0.0 }
}

/// A wander off from work (Callings K1.5), of `kind`, as entry `entry`.
///
/// - **On the stint menu** the draw ([`unit`] with [`DETOUR_SALT`], one for
///   the whole menu) takes it when it falls under the chance: the
///   calling's `wander` times [`DETOUR`], times [`DETOUR_HELD`] while the
///   job it's at has its momentum. Taken, it's worth [`DETOUR_OVER`] more
///   than the best job, and up to [`DETOUR_TASTE`] more by its kind's
///   taste and the whim; not, it's worth less than any job.
/// - **On the free-time menu** it's a place like the rest: the more the
///   calling wanders, and the more the kind is to its taste, the better.
///
/// A situation with no calling wanders not at all.
fn detour(s: &Situation, entry: usize, kind: DetourKind) -> f64 {
    let wander = s.who.calling().map_or(0.0, |c| c.wander.clamp(0.0, 1.0) as f64);
    let taste = kind.taste(&s.who.temperament) + whim(s, entry);
    match &s.question {
        Question::Stint { doing, run, .. } => {
            let best = s
                .menu
                .iter()
                .filter_map(|c| match c {
                    Choice::Take { job, setup, pays } => Some(stint(s, job, *setup, *pays)),
                    _ => None,
                })
                .fold(STINT, f64::max);
            let held = doing.is_some() && f64::from(*run) < least_stint(&s.who.temperament);
            let chance = DETOUR * wander * if held { DETOUR_HELD } else { 1.0 };
            if unit(s, DETOUR_SALT) < chance {
                best + DETOUR_OVER + DETOUR_TASTE * taste.clamp(0.0, 1.0)
            } else {
                // Under any job, which is worth STINT at the least
                0.2 * wander + 0.1 * taste.clamp(0.0, 1.0)
            }
        }
        Question::FreeTime => 0.4 + 0.4 * wander + 0.15 * taste,
        // A detour's answers on another question's menu fit nothing
        _ => 0.0,
    }
}

/// How much more or less a place appeals today: up to [`WHIM`] either
/// way, from who's asking, the quarter-hour and the entry. It's drawn from
/// the situation alone, so a replay of it answers the same (C3.1), but the
/// same citizen goes somewhere else an hour later.
fn whim(s: &Situation, entry: usize) -> f64 {
    (unit(s, entry as u64) * 2.0 - 1.0) * WHIM
}

/// A number in [0, 1) from who's asking, the quarter-hour and `salt`: the
/// rules' only draw, from the situation alone.
fn unit(s: &Situation, salt: u64) -> f64 {
    let quarter = s.when.time_of_day.map_or((s.when.hours * 4.0) as i64, |t| (t * 96.0) as i64);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.who.name.bytes().chain(quarter.to_le_bytes()).chain(salt.to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    // SplitMix's finish, so neighbouring quarters differ
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    (h >> 11) as f64 / (1u64 << 53) as f64
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

/// What a citizen wants a commission's price to beat the last trade by
/// before it takes on a promise: about a tenth, more for the thrifty and
/// less for the bold.
pub(crate) fn margin(t: &Temperament) -> f64 {
    1.1 + 0.3 * (t.thrift as f64 - 0.5) - 0.2 * (t.risk as f64 - 0.5)
}

/// What a crafter offers over its standing bid when it posts a commission
/// for the input: a fifth, a twentieth for the thriftiest.
pub(crate) fn premium(t: &Temperament) -> f64 {
    1.35 - 0.3 * t.thrift as f64
}

/// A commission on the board (S2.5), valued at the citizen's books.
///
/// - **Deliver** on a claim of its own before anything else ([`DELIVER`]);
///   without one, deliver what it holds if the price is the books' or
///   better.
/// - **Claim** a commission whose price beats the last trade by its
///   [`margin`]. With no trade to go by, the bold take a chance on it.
/// - **Pass** otherwise: a steady [`PASS`] (in [`RulesDecider`]'s pick).
///
/// Each takes as many as its span allows: the menu caps a claim at a day's
/// work and a delivery at what it holds.
fn commission(s: &Situation, i: usize, choice: &Choice) -> (f64, Decision) {
    let Question::Commission { item, claimed, price, .. } = &s.question else {
        // A commission's answers on another question's menu fit nothing
        return (0.0, Decision::plain(i));
    };
    let t = &s.who.temperament;
    let worth = worth(s, item);
    let most = |quantity: &super::Span<i32>| Decision {
        pick: i,
        price: None,
        quantity: Some(quantity.max),
    };
    match choice {
        Choice::Deliver { quantity } => {
            let u = if *claimed > 0 || worth.is_none_or(|w| *price >= w) { DELIVER } else { 0.3 };
            (u, most(quantity))
        }
        Choice::Claim { quantity } => {
            let u = match worth {
                Some(w) if *price as f64 >= w as f64 * margin(t) => SURE,
                Some(_) => 0.2,
                // Nothing to go by: a chance, for those who take them
                None => 0.3 + 0.5 * t.risk as f64,
            };
            (u, most(quantity))
        }
        _ => (PASS, Decision::plain(i)),
    }
}

/// What a citizen wants a wage to pay a game hour, over what its own trade
/// earns, before it works for someone else: as much, more for the thrifty
/// and the bold, less for the careful.
pub(crate) fn wage_margin(t: &Temperament) -> f64 {
    1.0 + 0.3 * (t.thrift as f64 - 0.5) + 0.3 * (t.risk as f64 - 0.5)
}

/// Work offered for its trade (S2.8): taken when it `pays` a game hour at
/// least what the citizen's own trade `earns` there by its
/// [`wage_margin`], or when its trade earns nothing at the books. With no
/// books to go by, the careful take it and the bold pass.
fn wage(s: &Situation, i: usize) -> (f64, Decision) {
    let Question::Wage { pays, earns, .. } = &s.question else {
        return (0.0, Decision::plain(i));
    };
    let t = &s.who.temperament;
    let u = match earns {
        Some(e) if *e <= 0 => SURE,
        Some(e) if *pays as f64 >= *e as f64 * wage_margin(t) => SURE,
        Some(_) => 0.2,
        None => 0.35 + 0.5 * (1.0 - t.risk as f64),
    };
    (u, Decision::plain(i))
}

/// Offering work (K2.4): worth more the further its Strands are over its
/// line, up to as far again, and less the thriftier,
///
/// ```text
/// u = 0.45 + 0.45 · min(1, (strands − line) / line) − 0.3 · (thrift − 0.5)
/// ```
///
/// so a middling citizen hires once it's more than a tenth over its line,
/// and a thrifty one once it's nearly half over. The rate is
/// [`wage::hire_rate`], and the units as many as the Strands over the line
/// escrow with the court's fee, up to the most there are. One that can't
/// escrow a unit is worth nothing.
fn hire(s: &Situation, i: usize, rate: &Span<i64>, cap: &Span<i32>) -> (f64, Decision) {
    let none = Decision {
        pick: i,
        price: Some(rate.min),
        quantity: Some(cap.min),
    };
    let Question::Hire { rate_for, fair, line, .. } = &s.question else {
        return (0.0, none);
    };
    let t = &s.who.temperament;
    let r = rate.clamp(wage::hire_rate(*rate_for, *fair, t));
    let budget = s.holdings.strands - line;
    let n = wage::affordable(r, cap.max, budget);
    if n < cap.min {
        return (0.0, none);
    }
    let over = (budget as f64 / (*line).max(1) as f64).clamp(0.0, 1.0);
    let u = 0.45 + 0.45 * over - 0.3 * (t.thrift as f64 - 0.5);
    let d = Decision {
        pick: i,
        price: Some(r),
        quantity: Some(n),
    };
    (u, d)
}

/// Asking to join the guild named `name` (S4.5): the company of its
/// members appeals the more the more sociable the citizen, and an
/// invitation more again. A guild for its trade, and a bigger one, a little
/// more. A middling citizen asks to join its trade's guild, a reserved one
/// waits to be asked, and the most reserved stay out even then.
fn guild(s: &Situation, i: usize, name: &str) -> (f64, Decision) {
    let Question::Guild { guilds } = &s.question else {
        return (0.0, Decision::plain(i));
    };
    let Some(g) = guilds.iter().find(|g| g.name == name) else {
        return (0.0, Decision::plain(i));
    };
    let t = &s.who.temperament;
    let u = 0.15 + 0.7 * t.sociability as f64
        + if g.invited { 0.25 } else { 0.0 }
        + if g.trade { 0.05 } else { 0.0 }
        + 0.01 * g.members.min(10) as f64;
    (u, Decision::plain(i))
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
                calling: None,
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

    fn commission(price: i64, claimed: i32, menu: Vec<Choice>, last: Option<i64>, t: Temperament) -> Situation {
        let q = Question::Commission {
            id: uuid::Uuid::nil(),
            item: "log".into(),
            grade: "F".into(),
            wanted: 20,
            claimed,
            price,
            due: 30.0,
        };
        let mut s = situation(q, menu, t);
        if let Some(last) = last {
            s.market.push(Quote {
                item: "log".into(),
                bid: None,
                ask: None,
                last: Some(last),
            });
        }
        s
    }

    fn claim(most: i32) -> Choice {
        Choice::Claim {
            quantity: Span::new(1, most),
        }
    }

    fn deliver(most: i32) -> Choice {
        Choice::Deliver {
            quantity: Span::new(1, most),
        }
    }

    #[test]
    fn a_promise_is_kept_before_anything_else() {
        for t in [Temperament::default(), with(|t| t.thrift = 1.0), with(|t| t.risk = 0.0)] {
            // Even at a price under the books: it was promised
            let (d, confidence) = decide(&commission(6, 10, vec![deliver(10), Choice::Pass], Some(10), t));
            assert_eq!(d, Decision { pick: 0, price: None, quantity: Some(10) });
            assert!(confidence >= 0.9, "{confidence}");
        }
    }

    #[test]
    fn a_commission_is_claimed_when_it_beats_the_books_by_the_margin() {
        let menu = || vec![claim(10), Choice::Pass];
        let pick = |price, t| decide(&commission(price, 0, menu(), Some(10), t)).0;
        let middling = Temperament::default();
        // A tenth over the last trade, for a middling citizen
        assert_eq!(pick(11, middling), Decision { pick: 0, price: None, quantity: Some(10) });
        assert_eq!(pick(10, middling).pick, 1);
        // The thrifty want more for a promise, the bold less
        assert_eq!(pick(12, with(|t| t.thrift = 1.0)).pick, 1);
        assert_eq!(pick(13, with(|t| t.thrift = 1.0)).pick, 0);
        assert_eq!(pick(10, with(|t| t.risk = 1.0)).pick, 0);
        // A middling claim is settled by the rules, not passed up
        let (_, confidence) = decide(&commission(12, 0, menu(), Some(10), middling));
        assert!(confidence >= super::super::RULES_THRESHOLD, "{confidence}");
        // With no trade to go by, the bold take a chance and the careful pass
        assert_eq!(decide(&commission(5, 0, menu(), None, with(|t| t.risk = 1.0))).0.pick, 0);
        assert_eq!(decide(&commission(5, 0, menu(), None, with(|t| t.risk = 0.0))).0.pick, 1);
    }

    #[test]
    fn goods_in_hand_are_delivered_sooner_than_promised() {
        let menu = || vec![deliver(6), claim(10), Choice::Pass];
        let (d, confidence) = decide(&commission(12, 0, menu(), Some(10), Temperament::default()));
        assert_eq!(d, Decision { pick: 0, price: None, quantity: Some(6) });
        assert!(confidence >= super::super::RULES_THRESHOLD, "{confidence}");
        // Under the books, it neither sells them there nor promises more
        assert_eq!(decide(&commission(8, 0, menu(), Some(10), Temperament::default())).0.pick, 2);
    }

    #[test]
    fn a_crafter_posts_for_an_unfilled_input_at_its_bid_and_a_premium() {
        let post = |t| {
            let menu = vec![
                Choice::Post {
                    item: "log".into(),
                    grade: "F".into(),
                    price: Span::new(10, 20),
                    quantity: Span::new(1, 20),
                    due: 2,
                },
                Choice::Pass,
            ];
            decide(&situation(Question::Buy { item: "log".into() }, menu, t))
        };
        let (d, confidence) = post(Temperament::default());
        assert_eq!(d, Decision { pick: 0, price: Some(12), quantity: Some(LOT) });
        assert!(confidence >= super::super::RULES_THRESHOLD, "{confidence}");
        // The thriftiest offer a Strand over, for fewer, and still post
        let (d, _) = post(with(|t| t.thrift = 1.0));
        assert_eq!(d, Decision { pick: 0, price: Some(11), quantity: Some(5) });
    }

    fn wage(pays: i64, earns: Option<i64>, t: Temperament) -> Situation {
        let q = Question::Wage {
            id: uuid::Uuid::nil(),
            employer: "Ada Reeder".into(),
            work: "craft plank at a sawbench".into(),
            rate: 2,
            left: 40,
            ends: 20.0,
            pays,
            earns,
        };
        situation(q, vec![Choice::TakeWork, Choice::Pass], t)
    }

    #[test]
    fn work_is_taken_when_it_pays_what_its_own_trade_earns() {
        let middling = Temperament::default();
        let pick = |pays, earns, t| decide(&wage(pays, earns, t)).0.pick;
        // As much as its own trade, an hour: a sure thing is as good
        assert_eq!(pick(3600, Some(3600), middling), 0);
        assert_eq!(pick(3500, Some(3600), middling), 1);
        let (_, confidence) = decide(&wage(3600, Some(3600), middling));
        assert!(confidence >= super::super::RULES_THRESHOLD, "{confidence}");
        // The thrifty and the bold want more to leave their own trade
        assert_eq!(pick(3600, Some(3600), with(|t| t.thrift = 1.0)), 1);
        assert_eq!(pick(4200, Some(3600), with(|t| t.thrift = 1.0)), 0);
        assert_eq!(pick(3600, Some(3600), with(|t| t.risk = 1.0)), 1);
        // The careful take a little less
        assert_eq!(pick(3200, Some(3600), with(|t| t.risk = 0.0)), 0);
        // Its own trade earns nothing at the books: any wage beats it
        assert_eq!(pick(10, Some(0), with(|t| t.thrift = 1.0)), 0);
        assert_eq!(pick(10, Some(-50), middling), 0);
        // Nothing to go by: the careful take it, the bold pass
        assert_eq!(pick(100, None, with(|t| t.risk = 0.0)), 0);
        assert_eq!(pick(100, None, middling), 0);
        assert_eq!(pick(100, None, with(|t| t.risk = 1.0)), 1);
    }

    fn hiring(strands: i64, line: i64, fair: Option<i64>, t: Temperament) -> Situation {
        let q = Question::Hire {
            work: "craft plank at a sawbench".into(),
            rate_for: 2,
            fair,
            line,
        };
        let menu = vec![
            Choice::Hire {
                rate: Span::new(1, 6),
                cap: Span::new(1, 36),
            },
            Choice::Pass,
        ];
        let mut s = situation(q, menu, t);
        s.holdings.strands = strands;
        s
    }

    #[test]
    fn a_citizen_over_its_line_hires_at_the_fair_rate_for_what_it_can_escrow() {
        let middling = Temperament::default();
        // Well over its line: the fair rate, and every unit there is
        let (d, confidence) = decide(&hiring(1200, 600, Some(3), middling));
        assert_eq!((d.pick, d.price, d.quantity), (0, Some(3), Some(36)));
        assert!(confidence >= super::super::RULES_THRESHOLD, "{confidence}");
        // A little over: as many as the Strands over the line escrow
        let (d, _) = decide(&hiring(700, 600, Some(3), middling));
        let n = d.quantity.unwrap();
        assert_eq!(d.pick, 0);
        assert!(n < 36 && n == crate::sim::wage::affordable(3, 36, 100), "{n}");
        // A tenth over isn't enough
        assert_eq!(decide(&hiring(660, 600, Some(3), middling)).0.pick, 1);
        // At or under the line, never: nobody hires while broke
        assert_eq!(decide(&hiring(600, 600, Some(3), middling)).0.pick, 1);
        assert_eq!(decide(&hiring(0, 600, Some(3), middling)).0.pick, 1);
        // The thrifty wait to be further over, and pay less; the
        // free-handed pay more
        let thrifty = with(|t| t.thrift = 1.0);
        assert_eq!(decide(&hiring(700, 600, Some(3), thrifty)).0.pick, 1);
        let (d, _) = decide(&hiring(1500, 600, Some(3), thrifty));
        assert_eq!((d.pick, d.price), (0, Some(2)));
        assert_eq!(decide(&hiring(1500, 600, Some(3), with(|t| t.thrift = 0.0))).0.price, Some(4));
        // With no books, the form's rate
        assert_eq!(decide(&hiring(1500, 600, None, middling)).0.price, Some(2));
    }

    /// A middling employer's rate is one a middling worker takes: what the
    /// work pays it an hour is what its own trade earns there.
    #[test]
    fn a_middling_citizen_takes_a_middling_citizens_offer() {
        use crate::catalog::Catalog;
        use crate::sim::wage as w;
        let catalog = Catalog::fallback();
        let books = |item: &str| catalog.target_price(item);
        let work = capitol_protocol::rules::wages::Work::Craft {
            station_id: uuid::Uuid::nil(),
            recipe_id: "plank".into(),
        };
        let middling = Temperament::default();
        let fair = w::fair(&catalog, &work, books);
        let (d, _) = decide(&hiring(1500, 600, fair, middling));
        let rate = d.price.unwrap();
        let earns = w::earns(Role::Crafter(crate::sim::Craft::Planks), &catalog, books);
        let pays = w::pays(&catalog, &work, rate).unwrap();
        assert_eq!(decide(&wage(pays, earns, middling)).0.pick, 0, "{rate}: {pays} against {earns:?}");
        // A thrifty employer's isn't
        let (d, _) = decide(&hiring(1500, 600, fair, with(|t| t.thrift = 1.0)));
        let pays = w::pays(&catalog, &work, d.price.unwrap()).unwrap();
        assert_eq!(decide(&wage(pays, earns, middling)).0.pick, 1);
    }

    fn guilds(guilds: Vec<super::super::GuildSeen>, t: Temperament) -> Situation {
        let mut menu: Vec<Choice> = guilds.iter().map(|g| Choice::Apply { guild: g.name.clone() }).collect();
        menu.push(Choice::Stay);
        situation(Question::Guild { guilds }, menu, t)
    }

    fn seen(name: &str, members: u32, invited: bool, trade: bool) -> super::super::GuildSeen {
        super::super::GuildSeen {
            name: name.into(),
            tag: "ORE".into(),
            members,
            charter: "For the miners of the ranges.".into(),
            invited,
            trade,
        }
    }

    #[test]
    fn the_sociable_join_their_trades_guild_and_the_reserved_wait_to_be_asked() {
        let pick = |g: Vec<_>, t| decide(&guilds(g, t)).0.pick;
        let ore = || seen("The Ore Guild", 1, false, true);
        let asked = || seen("The Ore Guild", 1, true, true);
        // A middling citizen asks to join its trade's guild, clearly enough
        assert_eq!(pick(vec![ore()], Temperament::default()), 0);
        let (_, confidence) = decide(&guilds(vec![ore()], with(|t| t.sociability = 0.8)));
        assert!(confidence >= super::super::RULES_THRESHOLD, "{confidence}");
        // A reserved one stays out, unless it's invited
        assert_eq!(pick(vec![ore()], with(|t| t.sociability = 0.2)), 1);
        assert_eq!(pick(vec![asked()], with(|t| t.sociability = 0.2)), 0);
        // The most reserved stay out even then
        assert_eq!(pick(vec![asked()], with(|t| t.sociability = 0.0)), 1);
        // Of two, the one that asked it, then the bigger
        let both = vec![seen("Diggers", 9, false, true), seen("The Ore Guild", 1, true, true)];
        assert_eq!(pick(both, Temperament::default()), 1);
        let both = vec![seen("Diggers", 2, false, true), seen("The Ore Guild", 8, false, true)];
        assert_eq!(pick(both, Temperament::default()), 1);
        // A name not on the question fits nothing
        let mut s = guilds(vec![ore()], Temperament::default());
        s.menu[0] = Choice::Apply { guild: "Nobody's".into() };
        assert_eq!(decide(&s).0.pick, 1);
    }

    use crate::citizen::calling::{self, Calling, Style};
    use crate::citizen::persona::Persona;
    use crate::sim::ROLES;
    use std::collections::BTreeMap;

    /// A citizen born to `role`, of `style`.
    fn calling(role: Role, style: Style, seed: u64) -> Calling {
        Persona::seeded_in(role, seed, &Default::default(), &calling::Mix::only(style)).calling
    }

    fn take(job: Role, setup: u32, pays: i64) -> Choice {
        Choice::Take {
            job: job.slug().into(),
            setup,
            pays,
        }
    }

    /// The stint question for `calling`, at `doing` for `run` game hours,
    /// with `lately` done, over `menu`.
    fn stint(calling: Option<Calling>, doing: Option<Role>, run: f32, lately: &BTreeMap<String, f32>, menu: Vec<Choice>) -> Situation {
        let q = Question::Stint {
            why: super::super::Why::Done,
            doing: doing.map(|d| d.slug().into()),
            run,
            lately: lately.clone(),
            line: 0,
        };
        let mut s = situation(q, menu, Temperament::default());
        s.who.calling = calling;
        s.carry_on = doing.and_then(|d| s.menu.iter().position(|c| matches!(c, Choice::Take { job, .. } if job == d.slug())));
        s
    }

    /// What `calling` works at over `stints` stints, every job ready: each
    /// job's share of them.
    fn lived(calling: &Calling, stints: usize) -> BTreeMap<String, f32> {
        lived_with(calling, stints, 0, 0)
    }

    /// [`lived`], with `strands` in hand against a trading `line` (K2.2).
    fn lived_with(calling: &Calling, stints: usize, strands: i64, line: i64) -> BTreeMap<String, f32> {
        let mut lately: BTreeMap<String, f32> = BTreeMap::new();
        let menu: Vec<Choice> = ROLES.iter().map(|&job| take(job, 0, 0)).collect();
        for _ in 0..stints {
            let mut s = stint(Some(calling.clone()), None, 0.0, &lately, menu.clone());
            s.holdings.strands = strands;
            if let Question::Stint { line: l, .. } = &mut s.question {
                *l = line;
            }
            let Choice::Take { job, .. } = s.choice(&decide(&s).0) else { panic!() };
            *lately.entry(job.clone()).or_default() += 1.0;
        }
        lately.values_mut().for_each(|v| *v /= stints as f32);
        lately
    }

    #[test]
    fn stints_chase_the_callings_shares_in_every_style() {
        for (k, role) in ROLES.into_iter().enumerate() {
            for style in Style::ALL {
                let c = calling(role, style, 31 * k as u64 + 7);
                let shares = lived(&c, 400);
                for job in ROLES {
                    let (got, want) = (shares.get(job.slug()).copied().unwrap_or(0.0), c.share(job));
                    assert!((got - want).abs() < 0.02, "{style} {}: {job} {got} against {want}", role.slug());
                }
                // So a specialist keeps to its favourite, and an all-rounder
                // spreads itself
                let top = shares.values().fold(0.0f32, |a, &b| a.max(b));
                match style {
                    Style::Specialist => assert!((0.6..=0.8).contains(&shares[role.slug()]), "{shares:?}"),
                    Style::Leaner => assert!((0.35..=0.55).contains(&shares[role.slug()]), "{shares:?}"),
                    Style::AllRounder => assert!(top <= 0.25 && shares.len() == 8, "{shares:?}"),
                }
            }
        }
    }

    /// Callings K2.2: past its line, a citizen's Strands raise trading's
    /// share of its work, and the stints follow it, so a fiber-rich
    /// citizen's money goes into goods with no change of role.
    #[test]
    fn wealth_raises_trading_and_the_stints_follow() {
        use crate::sim::stint::{rich_rise, target, RICH_RISE};
        let fiber = Role::Gatherer(crate::sim::Gather::Fiber);
        let line = 1200;
        assert_eq!((rich_rise(line, line), rich_rise(0, line), rich_rise(9999, 0)), (0.0, 0.0, 0.0));
        assert!((rich_rise(line * 3 / 2, line) - RICH_RISE / 2.0).abs() < 1e-9);
        assert_eq!(rich_rise(line * 5, line), RICH_RISE);
        for (k, style) in Style::ALL.into_iter().enumerate() {
            let c = calling(fiber, style, 11 + k as u64);
            let trading = f64::from(c.share(Role::Merchant));
            assert!(trading > 0.0, "{c}");
            // The shares still sum to 1, and nothing but trading rises
            for strands in [0, line, line * 3 / 2, line * 4] {
                let sum: f64 = ROLES.iter().map(|&j| target(&c, j, strands, line)).sum();
                assert!((sum - 1.0).abs() < 1e-6, "{c}: {sum}");
                for job in ROLES.into_iter().filter(|&j| j != Role::Merchant) {
                    assert!(target(&c, job, strands, line) <= f64::from(c.share(job)) + 1e-9);
                }
            }
            let rich = target(&c, Role::Merchant, line * 4, line);
            assert!((rich - (trading + RICH_RISE * (1.0 - trading))).abs() < 1e-9);
            // Under the line, its calling's shares; well over it, trading's
            // raised, and the stints come out near it
            let poor = lived_with(&c, 400, line - 1, line);
            let got = |shares: &BTreeMap<String, f32>| f64::from(shares.get(Role::Merchant.slug()).copied().unwrap_or(0.0));
            assert!((got(&poor) - trading).abs() < 0.02, "{style}: {} against {trading}", got(&poor));
            let well = lived_with(&c, 400, line * 4, line);
            assert!((got(&well) - rich).abs() < 0.02, "{style}: {} against {rich}", got(&well));
            assert!(got(&well) > got(&poor) + 0.2, "{style}");
        }
        // A calling with no lean to trading keeps its shares, as a swarm
        // bot's does
        let one = Calling::single(fiber);
        assert_eq!(target(&one, fiber, line * 4, line), 1.0);
        assert_eq!(target(&one, Role::Merchant, line * 4, line), 0.0);
    }

    #[test]
    fn a_day_starts_at_the_favourite_surely() {
        let menu: Vec<Choice> = ROLES.iter().map(|&job| take(job, 0, 0)).collect();
        let fiber = Role::Gatherer(crate::sim::Gather::Fiber);
        for style in [Style::Specialist, Style::Leaner] {
            let s = stint(Some(calling(fiber, style, 3)), None, 0.0, &BTreeMap::new(), menu.clone());
            let (d, confidence) = decide(&s);
            assert_eq!(d, Decision::plain(0), "{style}");
            assert!(confidence >= super::super::RULES_THRESHOLD, "{style}: {confidence}");
        }
        // The same situation always answers the same, to the bit
        let s = stint(Some(calling(fiber, Style::AllRounder, 3)), None, 0.0, &BTreeMap::new(), menu);
        let (a, b) = (decide(&s), decide(&s));
        assert_eq!((a.0, a.1.to_bits()), (b.0, b.1.to_bits()));
    }

    #[test]
    fn setup_is_charged_momentum_holds_and_a_pull_draws() {
        use crate::sim::{Craft, Gather};
        let (wood, planks, ore) = (Role::Gatherer(Gather::Wood), Role::Crafter(Craft::Planks), Role::Gatherer(Gather::Ore));
        // Two jobs a tenth each, nothing done of either
        let c = Calling {
            favourite: None,
            focus: 0.0,
            leanings: vec![(wood, 0.1), (ore, 0.1), (planks, 0.1), (Role::Builder, 0.7)],
            wander: 0.5,
        };
        let lately = BTreeMap::from([(Role::Builder.slug().to_string(), 1.0)]);
        // The one ready to start wins over one with a sawbench to set up
        let s = stint(Some(c.clone()), None, 0.0, &lately, vec![take(planks, 500, 0), take(wood, 0, 0)]);
        assert_eq!(decide(&s).0.pick, 1);
        // Setup is charged, not barred: a big enough shortfall still takes it
        assert_eq!(setup_cost(0), 1.0);
        assert!(setup_cost(28) > 0.8 && setup_cost(500) < 0.25 && setup_cost(500) > 0.0);
        // At the builder's work a short while, it stays, though wood is short
        let menu = || vec![take(Role::Builder, 0, 0), take(wood, 0, 0)];
        let t = Temperament::default();
        let s = stint(Some(c.clone()), Some(Role::Builder), least_stint(&t) as f32 - 0.1, &lately, menu());
        assert_eq!(s.carry_on, Some(0));
        assert_eq!(decide(&s).0.pick, 0, "momentum");
        // Past its stint's least length, it goes where its share is short
        let s = stint(Some(c.clone()), Some(Role::Builder), least_stint(&t) as f32 + 0.1, &lately, menu());
        assert_eq!(decide(&s).0.pick, 1);
        // A patient citizen's stints run longer
        let (hasty, patient) = (stint_hours(&with(|t| t.patience = 0.0)), stint_hours(&with(|t| t.patience = 1.0)));
        assert!(hasty < stint_hours(&t) && stint_hours(&t) < patient, "{hasty} {patient}");
        // A pull draws it to a job it has done its share of already
        let even: BTreeMap<String, f32> = [(wood, 0.1), (ore, 0.1), (planks, 0.1), (Role::Builder, 0.6)]
            .into_iter()
            .map(|(job, share)| (job.slug().to_string(), share))
            .collect();
        let s = stint(Some(c.clone()), None, 0.0, &even, vec![take(Role::Builder, 0, 0), take(ore, 0, 400)]);
        assert_eq!(decide(&s).0.pick, 1, "pulled");
        let s = stint(Some(c), None, 0.0, &even, vec![take(Role::Builder, 0, 0), take(ore, 0, 0)]);
        assert_eq!(decide(&s).0.pick, 0, "not pulled");
    }

    #[test]
    fn with_no_calling_a_citizen_keeps_to_its_role() {
        // A swarm bot's, or a log's from before callings: its role alone
        let lately = BTreeMap::from([("gatherer-wood".to_string(), 1.0)]);
        let menu = vec![take(Role::Builder, 0, 0), take(Role::Gatherer(crate::sim::Gather::Wood), 0, 0)];
        let s = stint(None, None, 0.0, &lately, menu);
        assert_eq!(s.who.role, "gatherer-wood");
        assert_eq!(decide(&s).0.pick, 1);
        // A stint's answer on another question's menu fits nothing
        let s = situation(Question::Work, vec![take(Role::Builder, 0, 0), Choice::Rest], Temperament::default());
        assert_eq!(decide(&s).0.pick, 1);
    }

    fn detour(kind: DetourKind) -> Choice {
        Choice::Detour {
            detour: kind,
            to: kind.slug().into(),
            does: format!("{kind:?}"),
        }
    }

    /// Of `n` stint ends a game quarter hour apart, how many the rules turn
    /// into a detour, for `who` wandering `wander`, at `doing` for `run`.
    fn detours_in(n: usize, wander: f32, doing: Option<Role>, run: f32) -> usize {
        let c = Calling {
            wander,
            ..calling(Role::Builder, Style::Leaner, 5)
        };
        let lately = BTreeMap::from([(Role::Builder.slug().to_string(), 1.0)]);
        let mut menu: Vec<Choice> = ROLES.iter().map(|&job| take(job, 0, 0)).collect();
        menu.extend(DetourKind::ALL.map(detour));
        (0..n)
            .filter(|&q| {
                let mut s = stint(Some(c.clone()), doing, run, &lately, menu.clone());
                s.when.time_of_day = Some((q as f64 + 0.5) / 96.0 % 1.0);
                s.who.name = format!("Citizen {}", q / 96);
                matches!(s.choice(&decide(&s).0), Choice::Detour { .. })
            })
            .count()
    }

    #[test]
    fn a_stints_end_turns_into_a_detour_as_often_as_the_calling_wanders() {
        let n = 960;
        for wander in [0.0, 0.2, 0.5, 0.9] {
            let share = detours_in(n, wander, None, 0.0) as f64 / n as f64;
            assert!((share - DETOUR * f64::from(wander)).abs() < 0.06, "wander {wander}: {share}");
        }
        // While the job it's at has its momentum, a quarter as often
        let t = Temperament::default();
        let held = detours_in(n, 0.8, Some(Role::Builder), least_stint(&t) as f32 - 0.1) as f64 / n as f64;
        assert!((held - DETOUR * DETOUR_HELD * 0.8).abs() < 0.05, "{held}");
        // With no calling (a swarm bot, a log from before callings), never
        let mut menu = vec![take(Role::Builder, 0, 0), detour(DetourKind::See)];
        let s = stint(None, None, 0.0, &BTreeMap::new(), menu.clone());
        assert_eq!(decide(&s).0.pick, 0);
        // A detour's answer on another question's menu fits nothing
        menu[0] = Choice::Rest;
        let s = situation(Question::Work, menu, Temperament::default());
        assert_eq!(decide(&s).0.pick, 0);
    }

    #[test]
    fn a_detour_leaves_the_jobs_as_they_were_and_goes_by_taste() {
        let c = Calling {
            wander: 1.0,
            ..calling(Role::Builder, Style::Specialist, 9)
        };
        let lately = BTreeMap::from([(Role::Builder.slug().to_string(), 1.0)]);
        let jobs: Vec<Choice> = ROLES.iter().map(|&job| take(job, 0, 0)).collect();
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for q in 0..96 {
            let at = |menu: Vec<Choice>, t: Temperament| {
                let mut s = stint(Some(c.clone()), None, 0.0, &lately, menu);
                s.who.temperament = t;
                s.when.time_of_day = Some((q as f64 + 0.5) / 96.0);
                let d = decide(&s).0;
                s.choice(&d).clone()
            };
            let mut menu = jobs.clone();
            menu.extend(DetourKind::ALL.map(detour));
            // The sociable browse and call, the solitary and bold go out
            let social = at(menu.clone(), with(|t| t.sociability = 1.0));
            let solitary = at(menu.clone(), with(|t| {
                t.sociability = 0.0;
                t.risk = 1.0;
            }));
            for (who, choice) in [("social", social), ("solitary", solitary)] {
                if let Choice::Detour { detour, .. } = choice {
                    *seen.entry(format!("{who} {}", detour.slug())).or_default() += 1;
                } else {
                    // Not taken, the job is the one it would be with none on the menu
                    assert_eq!(choice, at(jobs.clone(), Temperament::default()), "quarter {q}");
                }
            }
        }
        let n = |k: &str| seen.get(k).copied().unwrap_or(0);
        assert!(n("social browse") + n("social call") > n("social see") * 3, "{seen:?}");
        assert!(n("solitary see") + n("solitary try") > n("solitary call") * 3, "{seen:?}");
    }

    #[test]
    fn in_free_time_a_wanderer_wanders_off_and_a_homebody_seldom() {
        let free = |wander: f32| {
            (0..96)
                .filter(|&q| {
                    let menu = vec![Choice::Rest, Choice::Hall, Choice::Lane, Choice::Park { park: "p".into() }, detour(DetourKind::Read)];
                    let mut s = situation(Question::FreeTime, menu, Temperament::default());
                    s.who.calling = Some(Calling {
                        wander,
                        ..calling(Role::Builder, Style::Leaner, 5)
                    });
                    s.when.time_of_day = Some((q as f64 + 0.5) / 96.0);
                    matches!(s.choice(&decide(&s).0), Choice::Detour { .. })
                })
                .count()
        };
        let (homebody, wanderer) = (free(0.1), free(0.9));
        assert!(homebody <= 10 && wanderer >= 30, "{homebody} {wanderer}");
    }

    /// A town of the default mix lives four game hours of stints, each
    /// asked as its session starts and as each ends, as K1.2's run asks
    /// them: most of its citizens take a detour (K1.5's Verify: at least
    /// 70%), and detours take under a tenth of the time.
    #[test]
    fn most_of_a_town_wanders_off_within_four_hours() {
        const HOURS: f64 = 4.0;
        const QUARTER: f64 = 0.25;
        let roles = Role::assign(200);
        let (mut wanderers, mut detour_hours, mut all_hours) = (0, 0.0, 0.0);
        for (i, &role) in roles.iter().enumerate() {
            let persona = Persona::seeded(role, 1000 + i as u64);
            let t = persona.temperament;
            let jobs: Vec<Role> = crate::sim::stint::menu_jobs(&persona.calling, false);
            let mut lately = crate::sim::stint::Lately::default();
            let (mut at, mut last, mut took) = (0.0, f64::NEG_INFINITY, false);
            // The day's clock at 10x: a game hour is a twentieth of a day
            let start = (i as f64 * 0.137) % 1.0;
            let mut doing: Option<Role> = None;
            while at < HOURS {
                let open = at - last >= 1.5;
                let mut menu: Vec<Choice> = jobs.iter().map(|&j| take(j, 0, 0)).collect();
                if open {
                    menu.extend(DetourKind::ALL.map(detour));
                }
                let run = if doing.is_some() { stint_hours(&t) as f32 } else { 0.0 };
                let mut s = stint(Some(persona.calling.clone()), doing, run, &lately.question(), menu);
                s.who.name = format!("Citizen {i}");
                s.who.temperament = t;
                s.when.time_of_day = Some((start + at * 0.05) % 1.0);
                match s.choice(&decide(&s).0).clone() {
                    Choice::Detour { .. } => {
                        (took, last) = (true, at);
                        at += QUARTER;
                        detour_hours += QUARTER;
                    }
                    Choice::Take { job, .. } => {
                        let job = Role::from_slug(&job).unwrap();
                        let hours = stint_hours(&t).min(HOURS - at);
                        lately.add(job, hours);
                        doing = Some(job);
                        at += hours;
                    }
                    other => panic!("{other:?}"),
                }
            }
            all_hours += at;
            wanderers += usize::from(took);
        }
        let share = wanderers as f64 / roles.len() as f64;
        assert!(share >= 0.7, "{wanderers} of {} took a detour", roles.len());
        let time = detour_hours / all_hours;
        assert!((0.02..0.12).contains(&time), "detours took {:.1}% of the time", time * 100.0);
    }

    #[test]
    fn a_close_call_comes_back_unsure() {
        // Gathering and buying at a slightly dear price, thrifty: close
        let s = buy(9, Some(8), 2 * LOT, with(|t| t.thrift = 0.7));
        let (_, conf) = decide(&s);
        assert!(conf < super::super::RULES_THRESHOLD, "{conf}");
    }
}
