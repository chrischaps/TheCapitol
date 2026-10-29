//! A citizen at the hall's board (Commons S2.5): it answers commissions and
//! posts its own, as people do, through the court's normal routes and fees.
//!
//! - **Reading.** The board and its own claims are read every
//!   [`READ_EVERY`], and again after a notice says something changed.
//! - **Keeping its word.** A claim of its own comes first: once it holds
//!   all it owes at the grade (or the claim is nearly out), it walks to the
//!   hall and delivers, before anything else that day. Until then its work
//!   makes what it owes ([`Holdings::owed`]), and it sells none of it.
//! - **Letting go in time** (#505). A claim it won't keep, at its pace
//!   ([`pace`]) in the hours it's on before the claim runs out (its
//!   habit's, #276), is a question while the court's hour of grace is
//!   still ahead: it brings what it holds, then releases the rest, which is
//!   no breach, and the share goes back to the board. It asks again as it
//!   signs off ([`SimBot::release_on_leaving`]), so a night away never
//!   lapses a claim it could have let go.
//! - **Answering.** A commission for something one of its willing jobs
//!   makes ([`Role::answers`](crate::sim::Role::answers), Callings K1.2:
//!   its favourite, the job it's at, and those its calling leans to) is a
//!   question ([`Question::Commission`]): claim up to a day's work of it,
//!   deliver what it holds, or pass. One a round, the best paid first, and
//!   one it passed isn't asked about again for [`ASK_AGAIN`]. A claim it
//!   takes for another job's goods pulls it toward that job's stint.
//! - **Making it in time.** A claim is offered only for what it can make at
//!   the grade where it goes ([`ground`]) in the hours it's on while a
//!   claim runs. Anyone
//!   willing to mine counts the wild veins: a grade the rural ring can't
//!   give, or a price that pays the walk ([`WILD_PAY`]), sends it to the
//!   ranges, and it gathers only there until the claim is kept.
//! - **Posting.** A citizen whose bid for something it wants (the job at
//!   hand's inputs and its nearest jobs', Callings K1.4) has stood a game
//!   day without a fill posts a commission for it at the bid's price and a
//!   premium, never with more than [`MAX_POSTS`](crate::sim::MAX_POSTS)
//!   open ([`SimBot::post_wants`]).
//! - **A patron's posts** (Callings K2.3, [`patron`]). With Strands over
//!   its line it posts, at the hall, for what its home lacks: the kits of
//!   its calling's stations, a floor's goods, or the goods its leanings use
//!   ([`SimBot::post_patronage`]). A builder answers a kit's commission,
//!   one kit a claim.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use capitol_protocol::rest::{Commission, CommissionClaim, PostCommission};
use capitol_protocol::rules::court::{self, ClaimStatus, CommissionStatus};
use capitol_protocol::rules::market::grade_floor;
use capitol_protocol::rules::{Zone, DAY_LENGTH_SECS};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::super::patron::{self, PATRON_LINE};
use super::super::plan::Holdings;
use super::super::report::Entry;
use super::super::{stint::lot_for, Behind, Gather, Role, Trade, Wanted, LOT, POST_DAYS, POST_GRADE};
use super::{stint, Action, SimBot};
use crate::bot::BotError;
use crate::catalog::Catalog;
use crate::citizen::decide::{Choice, Question};
use crate::citizen::habit::Habit;
use crate::commissions::{meeting, stacks_for, GRADES};

/// How often a citizen reads the board, at most, in game seconds.
const READ_EVERY: f64 = 300.0;
/// Game seconds before a commission it passed is asked about again.
const ASK_AGAIN: f64 = 3600.0;
/// Seconds past the court's hour of grace a citizen lets a claim go by at
/// the latest (#505), so a slow round can't carry a release into the hour.
const RELEASE_MARGIN_SECS: i64 = 15 * 60;
/// Game seconds at a claim before its pace is what it made since, not a
/// day's work in a game day (#505): long enough to walk out to the ranges.
const TRIAL: f64 = DAY_LENGTH_SECS as f64;
/// A commission paying this many times the books pays an ore gatherer's
/// walk to the ranges, where the veins are thicker and better (#336).
const WILD_PAY: f64 = 1.5;
/// What the wilds' quality table adds at its deepest and highest: the last
/// rows of `backend/content/wilds.toml`'s `[quality]` (depth 10, height 8).
/// Only the server's planner reads that file, so it's kept here and a test
/// holds the two together. It decides only whether a claim is worth the walk
/// out; which node to gather is by the node's own quality (#504).
pub(crate) const WILD_QUALITY_ADD: i32 = 18;
/// The grade a citizen trusts itself to craft a good at: C. Its inputs'
/// quality decides, and a citizen doesn't plan for them.
const MADE_REACH: i32 = 55;

/// Where a citizen would go for a commission's goods at its grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ground {
    /// Where it works anyway: its own zone's nodes, or its own craft
    Home,
    /// Out to the ranges: an ore gatherer's wild veins
    Wilds,
}

/// Whether `role` can gather or make `item` at `min_quality`, and where
/// (S2.5). A node's quality is drawn across its spawn's range (in the
/// wilds from the row's floor up to the resource's top and
/// [`WILD_QUALITY_ADD`]); a citizen counts on the top third of it before it
/// sets out. Once there it gathers only from nodes at the grade, by each
/// node's own quality (#504). The
/// wild veins count for mining only: `role` is the job that answers for
/// `item`, so they count for anyone willing to mine (Callings K1.2). A
/// good no node yields is crafted, and made up to [`MADE_REACH`].
pub(crate) fn ground(role: Role, catalog: &Catalog, item: &str, min_quality: i32) -> Option<Ground> {
    let sources: Vec<_> = catalog.resource_types().iter().filter(|r| r.yields_item == item).collect();
    if sources.is_empty() {
        return (min_quality <= MADE_REACH).then_some(Ground::Home);
    }
    let reach = |floor: i32, top: i32| floor + (top - floor) * 2 / 3;
    let mut wilds = false;
    for r in sources {
        for spawn in r.spawns.iter().filter(|s| s.density > 0.0) {
            if spawn.zone == Zone::Wilderness.as_str() {
                let top = (r.base_quality_max + WILD_QUALITY_ADD).min(100);
                let floor = spawn.quality_floor.unwrap_or(r.base_quality_min).max(r.base_quality_min);
                wilds |= reach(floor, top) >= min_quality;
            } else if reach(r.base_quality_min, r.base_quality_max) >= min_quality {
                return Some(Ground::Home);
            }
        }
    }
    (wilds && role == Role::Gatherer(Gather::Ore)).then_some(Ground::Wilds)
}

/// A commission's grade as a letter, `A` to `F`: the best whose floor it
/// asks for.
pub(crate) fn grade_of(min_quality: i32) -> &'static str {
    GRADES
        .iter()
        .find(|g| grade_floor(g).is_some_and(|f| f <= min_quality))
        .copied()
        .unwrap_or("F")
}

/// What a claim may promise of `item`, by what the citizen made of it in
/// `made` over the last game day. Until it has worked a day, or for
/// something it hasn't made in one (a fiber gatherer's first rope), it
/// counts on a lot.
fn day_work(made: &VecDeque<(Instant, String, i32)>, item: &str, worked: Duration) -> i32 {
    let day = Duration::from_secs(DAY_LENGTH_SECS as u64);
    let n: i32 = made.iter().filter(|(at, i, _)| i == item && at.elapsed() < day).map(|(_, _, q)| q).sum();
    if worked >= day && n > 0 {
        n
    } else {
        n.max(LOT)
    }
}

/// Its pace on `item` (#505), in units a second: what it made of it in
/// `made` since `since` (the claim, or the session's start if that's
/// later), once it has been at it `trial`; until then, `day_work` in a game
/// day. A day back at most, as `made` keeps. `made` is this session's only,
/// so a citizen back from a night away is judged by a day's work again
/// until it has been at it a game day (#584).
fn pace(made: &VecDeque<(Instant, String, i32)>, item: &str, since: Instant, trial: Duration, day_work: i32) -> f64 {
    let day = Duration::from_secs(DAY_LENGTH_SECS as u64);
    let at = since.elapsed().min(day);
    if at < trial.min(day) {
        return day_work as f64 / DAY_LENGTH_SECS as f64;
    }
    let n: i32 = made.iter().filter(|(t, i, _)| i == item && t.elapsed() <= at).map(|(_, _, q)| q).sum();
    n as f64 / at.as_secs_f64()
}

/// The seconds a citizen keeping `hours` is on between `from` and `to`
/// (#505): by its habit's sessions (#276), or all of them without one.
fn hours_on(hours: Option<&Habit>, from: DateTime<Utc>, to: DateTime<Utc>) -> f64 {
    let (from, to) = (from.timestamp_millis(), to.timestamp_millis());
    let ms = match hours {
        Some(habit) => habit.online_ms(from, to),
        None => to - from,
    };
    ms.max(0) as f64 / 1000.0
}

/// Whether a claim still owing `short` beyond what's in hand is behind
/// (#505): at `pace` units a second, over the `on` seconds it's on before
/// the claim runs out, it makes less than that.
fn behind(short: i32, pace: f64, on: f64) -> Option<Behind> {
    let makes = (pace * on).floor().min(i32::MAX as f64) as i32;
    (short > 0 && makes < short).then_some(Behind { short, makes })
}

/// A citizen's view of the board, and its own part in it.
pub(super) struct Board {
    commissions: Vec<Commission>,
    claims: Vec<CommissionClaim>,
    /// When it last read them; `None` makes the next round read again
    read_at: Option<Instant>,
    /// Commissions it passed, and when
    passed: HashMap<Uuid, Instant>,
    /// What it made, and when, for a day's work
    made: VecDeque<(Instant, String, i32)>,
    began: Instant,
    /// Inputs its bids have wanted since, unfilled, and the bid's price
    wanting: HashMap<String, (Instant, i64)>,
    /// Commissions it claimed whose goods come from the wilds
    wild: HashSet<Uuid>,
    /// The Strands a middling citizen keeps back before it posts for its
    /// home (Callings K2.3): the town's `[patrons] line`, [`PATRON_LINE`]
    /// unless it says
    pub(super) patron_line: i64,
    /// What it passed on posting for as a patron, and when
    considered: HashMap<String, Instant>,
}

impl Board {
    pub(super) fn new() -> Board {
        Board {
            commissions: Vec::new(),
            claims: Vec::new(),
            read_at: None,
            passed: HashMap::new(),
            made: VecDeque::new(),
            began: Instant::now(),
            wanting: HashMap::new(),
            wild: HashSet::new(),
            patron_line: PATRON_LINE,
            considered: HashMap::new(),
        }
    }

    /// Something changed on the board: read it again next round.
    pub(super) fn stale(&mut self) {
        self.read_at = None;
    }

    /// `quantity` of `item` came into its hands.
    pub(super) fn made(&mut self, item: &str, quantity: i32) {
        let day = Duration::from_secs(DAY_LENGTH_SECS as u64);
        while self.made.front().is_some_and(|(at, _, _)| at.elapsed() >= day) {
            self.made.pop_front();
        }
        self.made.push_back((Instant::now(), item.to_string(), quantity));
    }

    /// A bid for `item` at `price` went up: it's wanted since the first.
    pub(super) fn bid(&mut self, item: &str, price: i64) {
        self.wanting.entry(item.to_string()).or_insert((Instant::now(), price)).1 = price;
    }

    /// A bid for `item` filled: it isn't wanting any more.
    pub(super) fn filled(&mut self, item: &str) {
        self.wanting.remove(item);
    }

    /// Its standing claims with something still to bring, and the
    /// commission of each.
    fn owing(&self) -> impl Iterator<Item = (&CommissionClaim, &Commission)> {
        self.claims
            .iter()
            .filter(|c| c.status == ClaimStatus::Open && c.quantity > c.delivered)
            .filter_map(|c| self.commissions.iter().find(|m| m.id == c.commission_id).map(|m| (c, m)))
    }

    /// Whether a claim of its own still wants anything (S2.8: it takes no
    /// work for a wage while it does).
    pub(super) fn owing_any(&self) -> bool {
        self.owing().next().is_some()
    }

    /// What its standing claims still want delivered, by item, whether it
    /// holds it or not: none of it is spare (Callings K1.4).
    pub(super) fn promised(&self) -> Vec<(String, i32)> {
        let mut promised: Vec<(String, i32)> = Vec::new();
        for (claim, c) in self.owing() {
            let left = claim.quantity - claim.delivered;
            match promised.iter_mut().find(|(item, _)| *item == c.item_type) {
                Some((_, n)) => *n += left,
                None => promised.push((c.item_type.clone(), left)),
            }
        }
        promised
    }

    /// Whether a claim of its own on `item` is to come from the wilds.
    pub(super) fn wild(&self, item: &str) -> bool {
        self.owing().any(|(c, m)| m.item_type == item && self.wild.contains(&c.commission_id))
    }

    /// The least quality its claims of its own on `item` ask a unit for: the
    /// most demanding claim's, so a node that gives it serves them all (#504).
    /// None when none asks for a grade.
    pub(super) fn grade_wanted(&self, item: &str) -> Option<i32> {
        self.owing().filter(|(_, m)| m.item_type == item && m.min_quality > 0).map(|(_, m)| m.min_quality).max()
    }
}

/// What a citizen does at the board this round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BoardAct {
    /// Claim `quantity` of commission `id`'s `item`; from the wilds, with
    /// `wild`. It `pays` Strands a game hour over the books, which pulls
    /// on the job that makes it (Callings K1.2)
    Claim { id: Uuid, item: String, quantity: i32, wild: bool, pays: i64 },
    /// Walk to the hall and deliver `quantity` of `item` to commission `id`
    Deliver { id: Uuid, item: String, quantity: i32 },
    /// Give up its claim on commission `id`'s `item` in good time (#505).
    /// From anywhere
    Release { id: Uuid, item: String },
}

impl BoardAct {
    /// What the health line and the report call it: `claim log`.
    pub(super) fn label(&self) -> String {
        match self {
            BoardAct::Claim { item, .. } => format!("claim {item}"),
            BoardAct::Deliver { item, .. } => format!("deliver {item}"),
            BoardAct::Release { item, .. } => format!("release a claim on {item}"),
        }
    }
}

impl SimBot {
    /// Reads the board and its own claims if a read is due. A board it can't
    /// read is an empty one until the next read; only a final error ends
    /// the round.
    pub(super) fn read_board(&mut self) -> Result<(), BotError> {
        if !self.people || self.board.read_at.is_some_and(|t| t.elapsed() < self.wall(READ_EVERY)) {
            return Ok(());
        }
        self.board.read_at = Some(Instant::now());
        match self.bot.commissions().and_then(|c| Ok((c, self.bot.my_claims()?))) {
            Ok((commissions, claims)) => {
                self.board.commissions = commissions;
                self.board.claims = claims;
                let standing: HashSet<Uuid> = self.board.claims.iter().map(|c| c.commission_id).collect();
                self.board.wild.retain(|id| standing.contains(id));
                Ok(())
            }
            Err(e) if e.is_final() => Err(e),
            Err(_) => Ok(()),
        }
    }

    /// What its claims want of it that it doesn't hold at their grade, by
    /// item, for [`Holdings::owed`].
    pub(super) fn owed(&self) -> Vec<(String, i32)> {
        let pack = self.bot.state().inventory();
        let mut owed: Vec<(String, i32)> = Vec::new();
        for (claim, c) in self.board.owing() {
            let held = held_at_grade(c, pack);
            let short = claim.quantity - claim.delivered - held;
            match owed.iter_mut().find(|(item, _)| *item == c.item_type) {
                Some((_, n)) => *n += short.max(0),
                None => owed.push((c.item_type.clone(), short.max(0))),
            }
        }
        owed
    }

    /// Asks the mind about the board, if there's a question: first its own
    /// claims, ready to deliver or behind (#505), then the best paid
    /// commission it could answer. `None`: nothing to do there this round.
    pub(super) fn choose_board(&mut self, holdings: &Holdings) -> Option<BoardAct> {
        let me = self.bot.state().me()?;
        let now = self.bot.state().utc_now();
        let grace = chrono::Duration::seconds(court::CLAIM_GRACE_SECS);
        // At night it works from home: claims, but no walk to the hall
        let walks = !self.night_shift;
        let pack = self.bot.state().inventory().cloned();
        let worked = self.board.began.elapsed();
        let catalog = self.bot.state().catalog().clone();
        // Its own claims first, once it holds what they want, or once it
        // won't keep one
        let owing: Vec<(CommissionClaim, Commission)> =
            self.board.owing().map(|(claim, c)| (claim.clone(), c.clone())).collect();
        for (claim, c) in owing {
            let due_soon = claim.until - now <= grace;
            let held = held_at_grade(&c, pack.as_ref());
            let wanted = Wanted {
                item: &c.item_type,
                unclaimed: court::unclaimed(c.quantity - c.delivered, c.claimed, c.withdrawn),
                owed: claim.quantity - claim.delivered,
                meeting: if walks || due_soon { held } else { 0 },
                day_work: 0,
                can_make: false,
                due_soon,
                behind: self.claim_behind(&catalog, &claim, &c, held, now),
            };
            if let Some(act) = self.ask_commission(&c, &wanted, holdings, false, 0) {
                return Some(act);
            }
        }
        // Then the board: what it answers, the best paid first. Not on a
        // shift, which keeps it from its own trade (S2.8)
        if self.wages.on_shift() {
            return None;
        }
        let again = self.wall(ASK_AGAIN);
        self.board.passed.retain(|_, at| at.elapsed() < again);
        let mine: HashSet<Uuid> = self.board.claims.iter().filter(|c| c.status == ClaimStatus::Open).map(|c| c.commission_id).collect();
        let mut open: Vec<Commission> = self
            .board
            .commissions
            .iter()
            .filter(|c| c.status == CommissionStatus::Open && c.poster_id != me && !c.withdrawn)
            .filter(|c| !mine.contains(&c.id) && !self.board.passed.contains_key(&c.id))
            .filter(|c| self.answerer(&c.item_type).is_some())
            .cloned()
            .collect();
        open.sort_by_key(|c| std::cmp::Reverse(c.price));
        for c in open {
            // The willing job that answers for it, whose menu it is
            let Some(job) = self.answerer(&c.item_type) else { continue };
            let unclaimed = court::unclaimed(c.quantity - c.delivered, c.claimed, c.withdrawn);
            let ground = ground(job, &catalog, &c.item_type, c.min_quality);
            let day_work = self.day_work(&catalog, &c.item_type, worked);
            // In time: a day's work is a game day's, made in the hours it's
            // on while a claim runs (#505), a court day at most and never
            // past the due date
            let on = self.on_between(now, court::claim_until(now, c.due_at));
            let in_time = (day_work as f64 * on / DAY_LENGTH_SECS as f64).floor() as i32;
            let promise = day_work.min(in_time);
            let wanted = Wanted {
                item: &c.item_type,
                unclaimed,
                owed: 0,
                meeting: if walks { held_at_grade(&c, pack.as_ref()) } else { 0 },
                day_work: promise,
                can_make: ground.is_some() && promise > 0,
                due_soon: false,
                behind: None,
            };
            // Better ore pays the walk to the ranges
            let worth = self.worth(&c.item_type);
            let pays_walk = worth.is_some_and(|w| c.price as f64 >= w as f64 * WILD_PAY);
            let wild = ground == Some(Ground::Wilds) || (pays_walk && job == Role::Gatherer(Gather::Ore));
            if job.commission_menu(&wanted).is_none() {
                continue;
            }
            let pays = stint::claim_pays(c.price, worth, day_work);
            return self.ask_commission(&c, &wanted, holdings, wild, pays);
        }
        None
    }

    /// The mind's answer about commission `c`, as `wanted` sees it: what
    /// to do, or `None` to pass (and not be asked again for a while). A
    /// claim of its own is any willing job's to bring in, and one it may
    /// take, the job's that answers for it.
    fn ask_commission(&mut self, c: &Commission, wanted: &Wanted, holdings: &Holdings, wild: bool, pays: i64) -> Option<BoardAct> {
        let job = self.answerer(&c.item_type).unwrap_or(self.job());
        let menu = job.commission_menu(wanted)?;
        let now = self.bot.state().utc_now();
        let question = Question::Commission {
            id: c.id,
            item: c.item_type.clone(),
            grade: grade_of(c.min_quality).to_string(),
            wanted: court::deliverable(wanted.unclaimed, wanted.owed),
            claimed: wanted.owed,
            price: c.price,
            due: (c.due_at - now).num_seconds().max(0) as f64 * self.scale / 3600.0,
        };
        let pass = menu.len() - 1;
        let act = match self.decide(question, menu, pass, holdings) {
            Some((d, Choice::Claim { .. })) => BoardAct::Claim {
                id: c.id,
                item: c.item_type.clone(),
                quantity: d.quantity?,
                wild,
                pays,
            },
            Some((d, Choice::Deliver { .. })) => BoardAct::Deliver {
                id: c.id,
                item: c.item_type.clone(),
                quantity: d.quantity?,
            },
            Some((_, Choice::Release { .. })) => BoardAct::Release {
                id: c.id,
                item: c.item_type.clone(),
            },
            _ => {
                if wanted.owed == 0 {
                    self.board.passed.insert(c.id, Instant::now());
                }
                return None;
            }
        };
        Some(act)
    }

    /// Does what it chose at the board.
    pub(super) fn board_act(&mut self, act: &BoardAct) -> Result<(), BotError> {
        match act {
            BoardAct::Claim { id, item, quantity, wild, pays } => {
                let claimed = self.bot.claim_commission(*id, *quantity);
                self.board.stale();
                // Refused (someone claimed it first, say): not asked again
                // straight away
                if claimed.is_err() {
                    self.board.passed.insert(*id, Instant::now());
                }
                let claimed = claimed?;
                if *wild {
                    self.board.wild.insert(*id);
                }
                self.entries.push(Entry::Promised {
                    item: item.clone(),
                    quantity: claimed.claim.quantity,
                });
                // A promise of another job's goods pulls it to that job
                self.pulled(item, *pays);
                // What it owes counts from now, not at the next read
                self.board.claims.retain(|c| c.commission_id != *id);
                self.board.claims.push(claimed.claim);
                if let Some(c) = self.board.commissions.iter_mut().find(|c| c.id == *id) {
                    *c = claimed.commission;
                }
                Ok(())
            }
            BoardAct::Deliver { id, item, quantity } => {
                self.bot.go_to_exchange()?;
                let result = self.deliver(*id, item, *quantity);
                self.board.stale();
                if result.is_err() {
                    self.board.passed.insert(*id, Instant::now());
                }
                result?;
                // At the hall anyway: what its trade wants
                self.bot.collect_market_box()?;
                self.post_demand()
            }
            BoardAct::Release { id, item } => {
                let ended = self.bot.release_claim(*id);
                self.board.stale();
                // It couldn't keep it: not claimed again straight away
                self.board.passed.insert(*id, Instant::now());
                let ended = ended?;
                self.board.wild.remove(id);
                // Released in good time; a late one is heard as a lapse
                if ended.claim.status == ClaimStatus::Released {
                    self.entries.push(Entry::Released {
                        item: item.clone(),
                        quantity: ended.claim.quantity - ended.claim.delivered,
                    });
                }
                self.board.claims.retain(|c| c.commission_id != *id);
                self.board.claims.push(ended.claim);
                if let Some(c) = self.board.commissions.iter_mut().find(|c| c.id == *id) {
                    *c = ended.commission;
                }
                Ok(())
            }
        }
    }

    /// As it signs off (#505): a claim it won't keep in the hours it's back
    /// before the claim runs out is a question now, while letting it go is
    /// free, rather than one it lapses on overnight. It's going home, so it
    /// brings nothing in.
    pub(super) fn release_on_leaving(&mut self) -> Result<(), BotError> {
        if !self.people {
            return Ok(());
        }
        let now = self.bot.state().utc_now();
        let pack = self.bot.state().inventory().cloned();
        let catalog = self.bot.state().catalog().clone();
        let holdings = self.holdings();
        let owing: Vec<(CommissionClaim, Commission)> =
            self.board.owing().map(|(claim, c)| (claim.clone(), c.clone())).collect();
        for (claim, c) in owing {
            let held = held_at_grade(&c, pack.as_ref());
            let Some(behind) = self.claim_behind(&catalog, &claim, &c, held, now) else { continue };
            let wanted = Wanted {
                item: &c.item_type,
                unclaimed: court::unclaimed(c.quantity - c.delivered, c.claimed, c.withdrawn),
                owed: claim.quantity - claim.delivered,
                meeting: 0,
                day_work: 0,
                can_make: false,
                due_soon: false,
                behind: Some(behind),
            };
            if let Some(act @ BoardAct::Release { .. }) = self.ask_commission(&c, &wanted, &holdings, false, 0) {
                self.board_act(&act)?;
            }
        }
        Ok(())
    }

    /// What a claim of its may promise of `item` (S2.5): a day's work
    /// ([`day_work`]), or a kit at a time, since they don't stack and a pack
    /// has 24 slots.
    fn day_work(&self, catalog: &Catalog, item: &str, worked: Duration) -> i32 {
        match catalog.station_for_kit(item) {
            Some(_) => 1,
            None => day_work(&self.board.made, item, worked),
        }
    }

    /// The seconds it's on between `from` and `to` (#505).
    pub(super) fn on_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> f64 {
        hours_on(self.hours.as_ref(), from, to)
    }

    /// Whether its `claim` on `c` won't be kept, holding `held` at the
    /// grade, while it may still be let go freely (#505): what's still to
    /// make, at its [`pace`] since it claimed (this session's part of it),
    /// over the hours it's on before the claim runs out. `None` when it's on
    /// time, or with the court's hour of grace and [`RELEASE_MARGIN_SECS`]
    /// or less left, when a release would be a lapse anyway.
    fn claim_behind(&self, catalog: &Catalog, claim: &CommissionClaim, c: &Commission, held: i32, now: DateTime<Utc>) -> Option<Behind> {
        let last = chrono::Duration::seconds(court::CLAIM_GRACE_SECS + RELEASE_MARGIN_SECS);
        if claim.until - now <= last {
            return None;
        }
        let short = claim.quantity - claim.delivered - held;
        let ago = (now - claim.claimed_at).to_std().unwrap_or_default();
        let since = Instant::now().checked_sub(ago).map_or(self.board.began, |at| at.max(self.board.began));
        let day_work = self.day_work(catalog, &c.item_type, self.board.began.elapsed());
        let pace = pace(&self.board.made, &c.item_type, since, self.wall(TRIAL), day_work);
        behind(short, pace, self.on_between(now, claim.until))
    }

    /// Delivers up to `quantity` of `item` to commission `id` from its pack,
    /// stack by stack, the fullest at the grade first. At the hall.
    fn deliver(&mut self, id: Uuid, item: &str, quantity: i32) -> Result<(), BotError> {
        let Some(c) = self.board.commissions.iter().find(|c| c.id == id).cloned() else {
            return Err(BotError::Failed(format!("commission {id} is off the board")));
        };
        let stacks: Vec<(Uuid, i32)> = stacks_for(&c, self.bot.state().inventory())
            .into_iter()
            .map(|s| (s.item_id, meeting(&s.units, c.min_quality) as i32))
            .collect();
        let (mut left, mut paid, mut filled) = (quantity, 0, false);
        for (stack, n) in stacks {
            if left <= 0 || filled {
                break;
            }
            let delivery = self.bot.deliver_commission(id, stack, left.min(n))?;
            left -= delivery.units.count() as i32;
            paid += delivery.paid;
            filled = delivery.commission.status == CommissionStatus::Filled;
        }
        if left == quantity {
            return Err(BotError::Failed(format!("no {item} at the commission's grade to deliver")));
        }
        self.entries.push(Entry::Delivered {
            item: item.to_string(),
            quantity: quantity - left,
            paid,
            filled,
        });
        Ok(())
    }

    /// At the hall: for each thing it wants ([`surplus::wants`](crate::sim::surplus::wants))
    /// that its bids have wanted a game day without a fill, asks the mind
    /// whether to post a commission for it.
    pub(super) fn post_wants(&mut self) -> Result<(), BotError> {
        let Some(me) = self.bot.state().me() else { return Ok(()) };
        let day = Duration::from_secs(DAY_LENGTH_SECS as u64);
        let wants = self.wants();
        for &item in &wants {
            let Some(&(since, bid)) = self.board.wanting.get(item) else { continue };
            let mine: Vec<&Commission> = self
                .board
                .commissions
                .iter()
                .filter(|c| c.poster_id == me && c.status == CommissionStatus::Open)
                .collect();
            if since.elapsed() < day || mine.iter().any(|c| c.item_type == item) {
                continue;
            }
            let open = mine.len();
            let holdings = self.holdings();
            let Some(menu) = super::super::post_menu(&holdings, item, &wants, bid, open) else { continue };
            let mark = self.mind.mark();
            let pass = menu.len() - 1;
            let trade = match self.decide(Question::Buy { item: item.to_string() }, menu, pass, &holdings) {
                Some((d, Choice::Post { item, .. })) => match (d.price, d.quantity) {
                    (Some(price), Some(quantity)) => Trade::Post { item, quantity, price },
                    _ => continue,
                },
                _ => {
                    self.settle(mark, None, &Ok(()));
                    // Asked again after another day of wanting
                    self.board.wanting.insert(item.to_string(), (Instant::now(), bid));
                    continue;
                }
            };
            let Trade::Post { item, quantity, price } = &trade else { unreachable!() };
            let result = self.post(item, *quantity, *price);
            self.settle(mark, Some(&Action::Trade(trade.clone())), &result);
            result?;
        }
        Ok(())
    }

    /// At the hall, a patron's posts (Callings K2.3): with Strands over its
    /// line, for each thing its home lacks ([`patron::home_wants`]) that it
    /// has no post of its own up for, asks the mind whether to post a
    /// commission for it. One it passed isn't asked about again for
    /// [`ASK_AGAIN`]. What its posts brought is collected from its market
    /// box first, so it never posts for a kit that came: a market spell
    /// doesn't collect the box.
    pub(super) fn post_patronage(&mut self) -> Result<(), BotError> {
        let Some(me) = self.bot.state().me() else { return Ok(()) };
        let line = self.patron_line();
        if self.persona.is_none() || self.bot.state().strands().unwrap_or(0) <= line {
            return Ok(());
        }
        self.bot.collect_market_box()?;
        let again = self.wall(ASK_AGAIN);
        self.board.considered.retain(|_, at| at.elapsed() < again);
        let catalog = self.bot.state().catalog().clone();
        let lacks = patron::home_wants(&catalog, &self.holdings(), &lot_for(&self.calling), &self.wants());
        for (item, want) in lacks {
            let mine: Vec<&Commission> = self
                .board
                .commissions
                .iter()
                .filter(|c| c.poster_id == me && c.status == CommissionStatus::Open)
                .collect();
            if mine.iter().any(|c| c.item_type == item) || self.board.considered.contains_key(&item) {
                continue;
            }
            let open = mine.len();
            let holdings = self.holdings();
            let Some(menu) = patron::patron_menu(&holdings, &item, want, self.worth(&item), line, open) else { continue };
            let mark = self.mind.mark();
            let pass = menu.len() - 1;
            let trade = match self.decide(Question::Buy { item: item.clone() }, menu, pass, &holdings) {
                Some((d, Choice::Post { item, .. })) => match (d.price, d.quantity) {
                    (Some(price), Some(quantity)) => Trade::Post { item, quantity, price },
                    _ => continue,
                },
                _ => {
                    self.settle(mark, None, &Ok(()));
                    self.board.considered.insert(item, Instant::now());
                    continue;
                }
            };
            let Trade::Post { item, quantity, price } = &trade else { unreachable!() };
            let result = self.post(item, *quantity, *price);
            self.settle(mark, Some(&Action::Trade(trade.clone())), &result);
            result?;
        }
        Ok(())
    }

    /// Its line as a patron: the Strands it keeps back before it posts for
    /// its home, by thrift ([`patron::patron_line`]).
    fn patron_line(&self) -> i64 {
        patron::patron_line(self.board.patron_line, self.mind.temperament.thrift)
    }

    /// Posts a commission for `quantity` of `item` at `price` each. At the
    /// hall.
    pub(super) fn post(&mut self, item: &str, quantity: i32, price: i64) -> Result<(), BotError> {
        let posted = self.bot.post_commission(PostCommission {
            item_type: item.to_string(),
            min_quality: grade_floor(POST_GRADE).unwrap_or(0),
            quantity,
            price,
            due_days: POST_DAYS,
        })?;
        self.entries.push(Entry::Posted {
            item: item.to_string(),
            quantity,
            price,
            fee: posted.fee,
        });
        // Wanting starts again from now: another day before another post
        if let Some(w) = self.board.wanting.get_mut(item) {
            w.0 = Instant::now();
        }
        self.board.commissions.insert(0, posted);
        Ok(())
    }
}

/// How many units its pack holds at commission `c`'s grade.
fn held_at_grade(c: &Commission, pack: Option<&crate::state::Container>) -> i32 {
    stacks_for(c, pack).iter().map(|s| meeting(&s.units, c.min_quality)).sum::<i64>() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Craft;

    #[test]
    fn ore_at_a_grade_the_rural_ring_cant_give_is_found_in_the_wilds() {
        let catalog = Catalog::fallback();
        let ore = Role::Gatherer(Gather::Ore);
        assert_eq!(ground(ore, &catalog, "iron_ore", 0), Some(Ground::Home));
        assert_eq!(ground(ore, &catalog, "iron_ore", 55), Some(Ground::Home));
        // B: past the rural veins' top third, inside the wild ones'
        assert_eq!(ground(ore, &catalog, "iron_ore", 75), Some(Ground::Wilds));
        // Nobody else counts on the ranges
        assert_eq!(ground(Role::Builder, &catalog, "iron_ore", 75), None);
        // Nothing gives A
        assert_eq!(ground(ore, &catalog, "iron_ore", 95), None);
        // A crafted good, up to C
        assert_eq!(ground(Role::Crafter(Craft::Planks), &catalog, "plank", 55), Some(Ground::Home));
        assert_eq!(ground(Role::Crafter(Craft::Planks), &catalog, "plank", 75), None);
    }

    /// A citizen holding claims on ore at C and at B gathers for the B: a
    /// node that gives it serves both (#504). One met in full, or on another
    /// good, or at no grade, asks nothing.
    #[test]
    fn the_grade_wanted_is_the_most_demanding_claims() {
        use crate::commissions::tests::{claim, commission};
        let mut board = Board::new();
        assert_eq!(board.grade_wanted("iron_ore"), None);
        board.commissions = vec![
            commission(1, "iron_ore", 60, 10, 0),
            commission(2, "iron_ore", 75, 10, 0),
            commission(3, "iron_ore", 90, 10, 0),
            commission(4, "wood", 85, 10, 0),
            commission(5, "iron_ore", 0, 10, 0),
        ];
        board.claims = vec![claim(11, 1, 5, 0), claim(12, 2, 5, 2), claim(13, 3, 5, 5), claim(14, 4, 5, 0), claim(15, 5, 5, 0)];
        assert_eq!(board.grade_wanted("iron_ore"), Some(75), "the A claim is met");
        assert_eq!(board.grade_wanted("wood"), Some(85));
        assert_eq!(board.grade_wanted("stone"), None);
    }

    #[test]
    fn the_wilds_top_add_is_wilds_tomls() {
        let text = include_str!("../../../../../../backend/content/wilds.toml");
        let wilds: toml::Value = toml::from_str(text).unwrap();
        let last = |key: &str| {
            wilds["quality"][key].as_array().unwrap().last().unwrap().as_array().unwrap()[1].as_integer().unwrap()
        };
        assert_eq!(WILD_QUALITY_ADD as i64, last("depth") + last("height"));
    }

    #[test]
    fn a_grade_is_the_best_whose_floor_is_asked_for() {
        assert_eq!(grade_of(0), "F");
        assert_eq!(grade_of(34), "F");
        assert_eq!(grade_of(35), "D");
        assert_eq!(grade_of(60), "C");
        assert_eq!(grade_of(75), "B");
        assert_eq!(grade_of(100), "A");
    }

    /// Its pace is a day's work in a game day until it has been at a claim
    /// a while, then what it made since (#505): a citizen stuck on a claim
    /// shows it.
    #[test]
    fn a_pace_is_what_it_made_since_once_it_has_been_at_it() {
        let ago = |secs: u64| Instant::now().checked_sub(Duration::from_secs(secs)).unwrap();
        let day = DAY_LENGTH_SECS as f64;
        let trial = Duration::from_secs(DAY_LENGTH_SECS as u64);
        let mut made = VecDeque::new();
        // Just claimed: a day's work in a day, whatever it made
        assert_eq!(pace(&made, "log", ago(600), trial, LOT), LOT as f64 / day);
        // A day at it with nothing to show: stuck
        assert_eq!(pace(&made, "log", ago(DAY_LENGTH_SECS as u64 + 60), trial, LOT), 0.0);
        // What it made since, of the claim's good, over the day
        made.push_back((ago(5000), "log".to_string(), 12));
        made.push_back((ago(100), "log".to_string(), 6));
        made.push_back((ago(100), "plank".to_string(), 30));
        let p = pace(&made, "log", ago(DAY_LENGTH_SECS as u64), trial, LOT);
        assert!((p - 18.0 / day).abs() < 1e-3 / day, "{p}");
        // Since the claim only: what came before was for something else
        let p = pace(&made, "log", ago(3000), Duration::from_secs(1800), LOT);
        assert!((p - 6.0 / 3000.0).abs() < 1e-4, "{p}");
    }

    /// A citizen near the end of its session whose next one comes after its
    /// claim runs out is on for the rest of this one only, and is behind
    /// on a day's work it would keep with all its hours (#505).
    #[test]
    fn a_claim_is_behind_when_the_hours_its_on_wont_make_it() {
        use crate::citizen::habit::Kind;
        let habit = Habit::new(Kind::Casual, 0, 7);
        let day_ms = 24 * 3600 * 1000;
        // A session whose next comes more than a day after its end
        let today = (20_000..20_400)
            .find(|&d| {
                habit.session(d).is_some_and(|s| {
                    (d + 1..d + 3).all(|n| habit.session(n).is_none_or(|next| next.start > s.end + day_ms))
                })
            })
            .expect("a casual citizen has a quiet day or two");
        let session = habit.session(today).unwrap();
        let at = |ms: i64| DateTime::from_timestamp_millis(ms).unwrap();
        let now = at(session.end - 30 * 60 * 1000);
        let until = now + chrono::Duration::seconds(court::CLAIM_SECS);
        let on = hours_on(Some(&habit), now, until);
        assert_eq!(on, 30.0 * 60.0, "the rest of this session, and none after");
        assert_eq!(hours_on(None, now, until), court::CLAIM_SECS as f64);
        // A day's work at a day's work a game day: a quarter of it in the
        // half hour it's on
        let pace = LOT as f64 / DAY_LENGTH_SECS as f64;
        assert_eq!(behind(LOT, pace, on), Some(Behind { short: LOT, makes: LOT / 4 }));
        // On all day, it's on time; with all of it in hand, nothing's short
        assert_eq!(behind(LOT, pace, hours_on(None, now, until)), None);
        assert_eq!(behind(0, pace, on), None);
        // Nothing made at all: all of it's short
        assert_eq!(behind(LOT, 0.0, on), Some(Behind { short: LOT, makes: 0 }));
    }

    #[test]
    fn a_days_work_is_what_it_made_in_a_day_or_a_lot_until_it_has() {
        let mut made = VecDeque::new();
        let day = Duration::from_secs(DAY_LENGTH_SECS as u64);
        assert_eq!(day_work(&made, "log", Duration::ZERO), LOT);
        made.push_back((Instant::now(), "log".to_string(), 25));
        made.push_back((Instant::now(), "plank".to_string(), 7));
        assert_eq!(day_work(&made, "log", Duration::ZERO), 25);
        assert_eq!(day_work(&made, "plank", Duration::ZERO), LOT);
        assert_eq!(day_work(&made, "plank", day), 7);
        assert_eq!(day_work(&made, "stone", day), LOT);
    }
}
