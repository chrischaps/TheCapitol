//! A citizen working for a wage (Commons S2.8): it takes piece-rate work
//! someone offers at its own trade, works the shift, and is paid for each
//! unit as it's made, through the court's normal routes.
//!
//! - **Reading.** The open offers are read every [`READ_EVERY`], and kept
//!   between reads from the board's own messages (`wage_offered`,
//!   `wage_updated`, `wage_closed`).
//! - **Taking.** Work its trade does ([`Role::wage_menu`](crate::sim::Role::wage_menu))
//!   is a question ([`Question::Wage`]): what the work pays a game hour
//!   against what its own trade earns one at the books ([`wage`]). Only in
//!   its working hours, owing nothing on the board, and once a day for any
//!   one employer, so a single person doesn't take over a citizen's life.
//!   One a round, the best paid first; one it passed isn't asked about
//!   again for [`ASK_AGAIN`]. It takes the work, walks there, and says so.
//! - **Working.** Each round a unit: a craft at the employer's station from
//!   what's in its box, or a gather on the employer's lot. The shift comes
//!   before the road in, a trade and its market hours, after a promise on
//!   the board.
//! - **Stopping.** At the end of its day (the walk home), when the work
//!   fails [`FAILS_BEFORE_STOP`] times running, and when its session ends.
//!   The server ends it at the cap, at the term, or when the employer stops
//!   it, and says so.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use capitol_protocol::rest::{ResourceNodeInfo, WageOffer};
use capitol_protocol::rules::wages::{OfferStatus, Work};
use uuid::Uuid;

use super::super::plan::Holdings;
use super::super::report::Entry;
use super::super::wage::{self, Hire};
use super::SimBot;
use crate::bot::BotError;
use crate::citizen::decide::{Choice, Question};
use crate::citizen::phrases::Occasion;
use crate::citizen::schedule::Phase;
use crate::poll::Fetch;

/// How often a citizen reads the work offered, at most, in game seconds.
const READ_EVERY: f64 = 300.0;
/// Game seconds before work it passed is asked about again.
const ASK_AGAIN: f64 = 3600.0;
/// Units of work that fail in a row before it gives the shift up: inputs
/// the employer's box ran out of, say, or a lot gathered bare.
const FAILS_BEFORE_STOP: u32 = 3;
/// How far from a lot's middle it looks for what a gather shift is for. A
/// rural lot is about 34k, so its corners are within this.
const LOT_REACH: f64 = 250.0;

/// The work offered, as a citizen last read it, and its part in it.
pub(super) struct Wages {
    offers: Vec<WageOffer>,
    /// When it last read them; `None` makes the next round read again
    read_at: Option<Instant>,
    /// Offers it passed, and when
    passed: HashMap<Uuid, Instant>,
    /// The shift it's working
    shift: Option<Shift>,
    /// The employers it has worked for, by the day it last did
    /// ([`rules::day_number`](capitol_protocol::rules::day_number))
    worked: HashMap<Uuid, i64>,
}

/// A shift a citizen is working.
#[derive(Debug, Clone)]
struct Shift {
    offer: WageOffer,
    /// Units of work that failed in a row
    fails: u32,
    /// Nodes on the lot the server refused it
    shunned: HashSet<Uuid>,
}

impl Shift {
    fn on(offer: WageOffer) -> Shift {
        Shift {
            offer,
            fails: 0,
            shunned: HashSet::new(),
        }
    }
}

impl Wages {
    pub(super) fn new() -> Wages {
        Wages {
            offers: Vec::new(),
            read_at: None,
            passed: HashMap::new(),
            shift: None,
            worked: HashMap::new(),
        }
    }

    /// Whether it's working a shift.
    pub(super) fn on_shift(&self) -> bool {
        self.shift.is_some()
    }

    /// Whether it's working a shift on `offer_id`.
    fn on(&self, offer_id: Uuid) -> bool {
        self.shift.as_ref().is_some_and(|s| s.offer.id == offer_id)
    }

    /// An offer as the board now has it: up, taken, worked, or down.
    pub(super) fn update(&mut self, offer: WageOffer) {
        if let Some(shift) = self.shift.as_mut().filter(|s| s.offer.id == offer.id) {
            shift.offer = offer.clone();
        }
        self.offers.retain(|o| o.id != offer.id);
        if offer.status == OfferStatus::Open {
            self.offers.insert(0, offer);
        }
    }
}

/// What a citizen does about wages this round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WageAct {
    /// Take offer `id`'s work, `work` in a few words, and walk there
    Take { id: Uuid, work: String },
    /// A unit of the shift's work
    Work { work: String },
}

impl WageAct {
    /// What the health line and the report call it: `work: craft plank at
    /// a sawbench`.
    pub(super) fn label(&self) -> String {
        match self {
            WageAct::Take { work, .. } => format!("take work: {work}"),
            WageAct::Work { work } => format!("work: {work}"),
        }
    }
}

impl SimBot {
    /// Reads the work offered, and its own shift as the server has it, if a
    /// read is due. Work it can't read is none until the next read; only a
    /// final error ends the round.
    pub(super) fn read_wages(&mut self) -> Result<(), BotError> {
        if !self.people || self.wages.read_at.is_some_and(|t| t.elapsed() < self.wall(READ_EVERY)) {
            return Ok(());
        }
        self.wages.read_at = Some(Instant::now());
        let read = match self.bot.wages() {
            Ok(read) => read,
            Err(e) if e.is_final() => return Err(e),
            Err(_) => return Ok(()),
        };
        self.wages.offers = read.offers;
        let me = self.bot.state().me();
        // A shift the server says it's on: one from before a restart, or the
        // one it knows. None: any it thought it had ended unheard
        let running = read.shift.filter(|s| Some(s.worker_id) == me && s.ended_at.is_none());
        let kept = self.wages.shift.take();
        self.wages.shift = running.and_then(|s| {
            let offer = self.wages.offers.iter().find(|o| o.id == s.offer_id)?.clone();
            Some(match kept {
                Some(mut shift) if shift.offer.id == offer.id => {
                    shift.offer = offer;
                    shift
                }
                _ => Shift::on(offer),
            })
        });
        Ok(())
    }

    /// What to do about wages this round: a unit of its shift, or the mind's
    /// answer to the best paid work its trade does, in its working hours.
    /// `None`: nothing to do there.
    pub(super) fn choose_wage(&mut self, holdings: &Holdings, phase: Option<Phase>) -> Option<WageAct> {
        if !self.people {
            return None;
        }
        let catalog = self.bot.state().catalog().clone();
        if let Some(shift) = &self.wages.shift {
            return Some(WageAct::Work {
                work: wage::words(&catalog, &shift.offer.work),
            });
        }
        // In its working hours only, and never at a promise's cost
        if phase != Some(Phase::Work) || self.night_shift || self.board.owing_any() {
            return None;
        }
        let me = self.bot.state().me()?;
        let state = self.bot.state();
        let today = state.server_time_ms().map_or(0, capitol_protocol::rules::day_number);
        let now = state.utc_now();
        let again = self.wall(ASK_AGAIN);
        self.wages.passed.retain(|_, at| at.elapsed() < again);
        let mut open: Vec<(i64, WageOffer)> = self
            .wages
            .offers
            .iter()
            .filter(|o| o.status == OfferStatus::Open && o.shift.is_none() && o.employer_id != me)
            .filter(|o| o.worker_id.is_none_or(|w| w == me))
            .filter(|o| !self.wages.passed.contains_key(&o.id) && self.wages.worked.get(&o.employer_id) != Some(&today))
            .filter_map(|o| Some((wage::pays(&catalog, &o.work, o.rate)?, o.clone())))
            .collect();
        open.sort_by_key(|(pays, _)| std::cmp::Reverse(*pays));
        let earns = wage::earns(self.role, &catalog, |item| self.worth(item));
        for (pays, o) in open {
            let Some(job) = wage::job(&catalog, &o.work) else { continue };
            let tooled = wage::tool_needed(&catalog, &o.work).is_none_or(|t| holdings.tools.contains(t));
            let hire = Hire {
                job,
                left: o.cap - o.made,
                tooled,
            };
            let Some(menu) = self.role.wage_menu(&hire) else { continue };
            let work = wage::words(&catalog, &o.work);
            let question = Question::Wage {
                id: o.id,
                employer: o.employer_name.clone(),
                work: work.clone(),
                rate: o.rate,
                left: hire.left,
                ends: (o.ends_at - now).num_seconds().max(0) as f64 * self.scale / 3600.0,
                pays,
                earns,
            };
            let pass = menu.len() - 1;
            return match self.decide(question, menu, pass, holdings) {
                Some((_, Choice::TakeWork)) => Some(WageAct::Take { id: o.id, work }),
                _ => {
                    self.wages.passed.insert(o.id, Instant::now());
                    None
                }
            };
        }
        None
    }

    /// Does what it chose about wages.
    pub(super) fn wage_act(&mut self, act: &WageAct) -> Result<(), BotError> {
        match act {
            WageAct::Take { id, work } => self.take_work(*id, work),
            WageAct::Work { .. } => self.work_a_unit(),
        }
    }

    /// Takes offer `id`'s work, walks there and says so.
    fn take_work(&mut self, id: Uuid, work: &str) -> Result<(), BotError> {
        let taken = self.bot.take_wages(id);
        // Refused (someone took it first, say): not asked again straight away
        if taken.is_err() {
            self.wages.passed.insert(id, Instant::now());
        }
        let shift = taken?;
        let Some(mut offer) = self.wages.offers.iter().find(|o| o.id == id).cloned() else {
            return Err(BotError::Failed(format!("offer {id} is off the board")));
        };
        offer.shift = Some(shift);
        let today = self.bot.state().server_time_ms().map_or(0, capitol_protocol::rules::day_number);
        self.wages.worked.insert(offer.employer_id, today);
        self.entries.push(Entry::TookWork { work: work.to_string() });
        let (x, y) = to_work(&offer);
        let employer = offer.employer_id;
        self.wages.shift = Some(Shift::on(offer));
        self.bot.walk_to(x, y)?;
        self.bot.speak(Occasion::Hired, employer);
        Ok(())
    }

    /// A unit of its shift's work. Work that fails [`FAILS_BEFORE_STOP`]
    /// times running gives the shift up.
    fn work_a_unit(&mut self) -> Result<(), BotError> {
        let Some(shift) = self.wages.shift.clone() else { return Ok(()) };
        let (x, y) = to_work(&shift.offer);
        let result = match &shift.offer.work {
            Work::Craft { station_id, recipe_id } => self
                .bot
                .walk_to(x, y)
                .and_then(|()| self.bot.craft_on_shift(*station_id, recipe_id))
                .map(drop),
            Work::Gather { resource, .. } => self.gather_on_shift(&shift, resource),
        };
        match result {
            Ok(()) => {
                if let Some(shift) = self.wages.shift.as_mut() {
                    shift.fails = 0;
                }
                Ok(())
            }
            // Not the work's failing: the connection, or someone asking to
            // trade
            Err(e @ (BotError::Offline(_) | BotError::Interrupted(_))) => Err(e),
            Err(e) if e.is_final() => Err(e),
            Err(e) => {
                let fails = self.wages.shift.as_mut().map_or(0, |s| {
                    s.fails += 1;
                    s.fails
                });
                if fails >= FAILS_BEFORE_STOP {
                    self.end_shift("failed")?;
                }
                Err(e)
            }
        }
    }

    /// A gather of `resource` on the shift's lot: the nearest ready node of
    /// it near the lot's middle, walking there first if none is in sight.
    fn gather_on_shift(&mut self, shift: &Shift, resource: &str) -> Result<(), BotError> {
        let (x, y) = to_work(&shift.offer);
        let mut node = self.node_on_lot(shift, resource);
        if node.is_none() {
            self.bot.walk_to(x, y)?;
            self.bot.refetch(Fetch::NearbyNodes);
            self.bot.settle()?;
            node = self.node_on_lot(shift, resource);
        }
        let Some(node) = node else {
            return Err(BotError::Failed(format!("no {resource} ready on the lot")));
        };
        match self.bot.extract_on_shift(&node) {
            Ok(_) => Ok(()),
            // Off the lot after all, or someone else's to gather: another
            Err(e @ BotError::Refused { .. }) => {
                if let Some(shift) = self.wages.shift.as_mut() {
                    shift.shunned.insert(node.id);
                }
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// The nearest ready node of `resource` within [`LOT_REACH`] of the
    /// shift's lot, that nobody is gathering and the server hasn't refused.
    fn node_on_lot(&self, shift: &Shift, resource: &str) -> Option<ResourceNodeInfo> {
        let state = self.bot.state();
        let (lx, ly) = (shift.offer.x, shift.offer.y);
        let (x, y) = self.bot.position();
        let shunned = self.wages.shift.as_ref().map_or(&shift.shunned, |s| &s.shunned);
        state
            .nodes()
            .values()
            .filter(|n| n.resource_type == resource && n.state == "available")
            .filter(|n| (n.x - lx).hypot(n.y - ly) <= LOT_REACH && !shunned.contains(&n.id))
            .filter(|n| state.harvester(n.id).is_none())
            .min_by(|a, b| (a.x - x).hypot(a.y - y).total_cmp(&(b.x - x).hypot(b.y - y)))
            .cloned()
    }

    /// Stops its shift, if it's on one, `why` in a word for the report:
    /// `day`, `failed`, `signed_off`. What was made is paid either way.
    pub(super) fn end_shift(&mut self, why: &str) -> Result<(), BotError> {
        let Some(shift) = self.wages.shift.take() else { return Ok(()) };
        let ended = match self.bot.stop_shift(shift.offer.id) {
            Ok(ended) => ended,
            // Over already, at the cap or by the employer's say, unheard:
            // its end was the server's to say
            Err(BotError::Rest { source, .. }) if source.status().is_some_and(|s| s < 500) => return Ok(()),
            Err(e) => return Err(e),
        };
        self.entries.push(Entry::ShiftEnded {
            why: why.to_string(),
            made: ended.shift.made,
        });
        self.bot.speak(Occasion::ShiftDone, shift.offer.employer_id);
        Ok(())
    }

    /// The server ended a shift of its on `offer_id` (`wage_shift_ended`): at
    /// the cap, the term, or by the employer's say.
    pub(super) fn shift_ended(&mut self, offer_id: Uuid, why: &str, made: i32) {
        if !self.wages.on(offer_id) {
            return;
        }
        let employer = self.wages.shift.take().map(|s| s.offer.employer_id);
        self.entries.push(Entry::ShiftEnded {
            why: why.to_string(),
            made,
        });
        if let Some(employer) = employer {
            self.bot.speak(Occasion::ShiftDone, employer);
        }
    }
}

/// Where a shift's work is done: in reach of the station, or the lot's
/// middle.
fn to_work(offer: &WageOffer) -> (f64, f64) {
    match offer.work {
        Work::Craft { .. } => (offer.x, offer.y + 10.0),
        Work::Gather { .. } => (offer.x, offer.y),
    }
}
