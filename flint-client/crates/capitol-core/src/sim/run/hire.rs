//! A citizen hiring (Callings K2.4): piece-rate work offered at a station or
//! on a lot of its own that its stint leaves idle, as a person with a bench
//! they aren't using and Strands to spare does (GDD: veterans hire
//! newcomers). Through the court's routes and fees, as the Flint client's
//! Hire tab offers it (Commons S2.7), so it can't breach.
//!
//! - **When.** In its working hours, owing nothing on the board, not on a
//!   shift of its own, with fewer than [`MAX_HIRES`] offers up. Each place
//!   is asked about once a game hour at most ([`ASK_AGAIN`]).
//! - **What.** The work its [openings](wage::openings) give: a craft at a
//!   station whose trade isn't its stint's, from inputs its stint doesn't
//!   need, or a gather on its lot of what its stint doesn't gather.
//! - **Whether.** A question ([`Question::Hire`]), asked only with Strands
//!   over its line (the town's `[hiring] line`, by thrift: [`wage::hire_line`])
//!   enough to escrow the work at the rate its books say is fair. Below the
//!   line nobody hires, so a broke citizen never does.
//! - **Offering.** A craft's inputs go into the station's box first; then
//!   the offer, for a day. Its mark goes with it, as with everything it
//!   posts: the server says the employer is a citizen.
//! - **Ending.** It takes the offer down when it signs off, at dusk, when
//!   its money runs low (under half its line), and when its stint wants the
//!   place back ([`wage::wants_back`]). The server ends it at the cap and
//!   the term. What a craft's hire made, and what it left, comes home from
//!   the box after.
//! - **Taking.** Citizens take each other's work through the worker's side
//!   (`wages.rs`), as they take a person's.

use std::collections::HashMap;
use std::time::Instant;

use capitol_protocol::rest::WageOffer;
use capitol_protocol::rules::wages::{OfferStatus, Work};
use uuid::Uuid;

use super::super::plan::Holdings;
use super::super::report::Entry;
use super::super::wage::{self, Opening};
use super::SimBot;
use crate::bot::BotError;
use crate::citizen::decide::Choice;
use crate::citizen::schedule::Phase;

/// The most offers a citizen keeps up at once: a station and its lot, say.
const MAX_HIRES: usize = 2;
/// Game seconds before a place it asked about is asked about again.
const ASK_AGAIN: f64 = 3600.0;
/// How long an offer runs, in days: the least the court takes. It comes
/// down at dusk anyway.
const TERM_DAYS: i64 = 1;

/// A citizen's offers of work, and what it has done about hiring.
pub(super) struct Hiring {
    /// The Strands a middling citizen keeps back before it hires: the
    /// town's `[hiring] line`, [`wage::HIRE_LINE`] unless it says
    pub(super) base_line: i64,
    /// Its offers still up, as it last heard of them
    open: Vec<WageOffer>,
    /// Each place it asked about (a station or a lot), and when
    asked: HashMap<Uuid, Instant>,
    /// Stations whose boxes hold what a hire made or left, to bring home
    collect: Vec<Uuid>,
    /// It has looked on the board for offers of its own a session before
    /// left up (a crash, say), so they end as any of its offers does
    adopted: bool,
}

impl Hiring {
    pub(super) fn new() -> Hiring {
        Hiring {
            base_line: wage::HIRE_LINE,
            open: Vec::new(),
            asked: HashMap::new(),
            collect: Vec::new(),
            adopted: false,
        }
    }

    /// Takes up offers of `me`'s still open on the board, once a session.
    pub(super) fn adopt(&mut self, offers: &[WageOffer], me: Uuid) {
        if self.adopted {
            return;
        }
        self.adopted = true;
        for o in offers.iter().filter(|o| o.employer_id == me && o.status == OfferStatus::Open) {
            if !self.open.iter().any(|m| m.id == o.id) {
                self.open.push(o.clone());
            }
        }
    }
}

/// What a citizen does about hiring this round.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum HireAct {
    /// Offer `opening`'s work at `rate` a unit for up to `cap` units,
    /// stocking a station's box first. `work` in a few words.
    Offer {
        opening: Opening,
        rate: i64,
        cap: i32,
        work: String,
    },
    /// Bring home what a hire left in the box of station `station`
    Collect { station: Uuid },
}

impl HireAct {
    /// What the health line and the report call it: `hire: craft plank at
    /// a sawbench`.
    pub(super) fn label(&self) -> String {
        match self {
            HireAct::Offer { work, .. } => format!("hire: {work}"),
            HireAct::Collect { .. } => "hire: bring home the box".into(),
        }
    }
}

impl SimBot {
    /// Its line: the Strands it keeps back before it hires.
    fn hire_line(&self) -> i64 {
        wage::hire_line(self.hiring.base_line, self.mind.temperament.thrift)
    }

    /// What to do about hiring this round: a box to bring home, or the
    /// mind's answer about the first place it could hire for, in its
    /// working hours. `None`: nothing to do there.
    pub(super) fn choose_hire(&mut self, holdings: &Holdings, phase: Option<Phase>) -> Option<HireAct> {
        if !self.people || self.persona.is_none() {
            return None;
        }
        if phase != Some(Phase::Work) || self.night_shift || self.wages.on_shift() || self.board.owing_any() {
            return None;
        }
        // A box a hire left, once nobody works there
        let busy = |station: Uuid, open: &[WageOffer]| open.iter().any(|o| wage_place(&o.work) == station);
        self.hiring.collect.retain(|s| self.stations.contains_key(s));
        if let Some(&station) = self.hiring.collect.iter().find(|s| !busy(**s, &self.hiring.open)) {
            return Some(HireAct::Collect { station });
        }
        if self.hiring.open.len() >= MAX_HIRES {
            return None;
        }
        let line = self.hire_line();
        if holdings.strands <= line {
            return None;
        }
        let catalog = self.bot.state().catalog().clone();
        let job = self.job();
        let stations: Vec<(String, Uuid)> = self.stations.values().map(|s| (s.station_type.clone(), s.id)).collect();
        // What its stint neither needs nor makes: a fiber stint banks its
        // fiber
        let spare: HashMap<String, i32> = holdings
            .items
            .iter()
            .filter(|(item, _)| !job.needs().contains(&item.as_str()) && !job.answers().contains(&item.as_str()))
            .map(|(item, n)| (item.clone(), *n))
            .collect();
        let state = self.bot.state();
        // What grows on its lot, and how many nodes of each, of all it has
        // seen: the server's own test is the lot's bounding box (#435)
        let lot = state.plots().first().map(|plot| {
            let b = &plot.bounds;
            let mut seen: HashMap<uuid::Uuid, &str> = HashMap::new();
            for n in self.atlas.values().map(|s| &s.node).chain(state.nodes().values()) {
                if (b.min_x..=b.max_x).contains(&n.x) && (b.min_y..=b.max_y).contains(&n.y) {
                    seen.insert(n.id, &n.resource_type);
                }
            }
            let mut counts: HashMap<String, i32> = HashMap::new();
            for kind in seen.values() {
                *counts.entry(kind.to_string()).or_insert(0) += 1;
            }
            let mut counts: Vec<(String, i32)> = counts.into_iter().collect();
            counts.sort();
            (plot.id, counts)
        });
        let again = self.wall(ASK_AGAIN);
        self.hiring.asked.retain(|_, at| at.elapsed() < again);
        let openings: Vec<Opening> = wage::openings(&catalog, job, &stations, &spare, lot.as_ref().map(|(id, r)| (*id, r.as_slice())))
            .into_iter()
            .filter(|o| {
                let place = wage_place(&o.work);
                !busy(place, &self.hiring.open) && !self.hiring.asked.contains_key(&place)
            })
            .collect();
        for opening in openings {
            let Some((question, menu)) = wage::hire_menu(&opening, &catalog, holdings, line, |item| self.worth(item)) else {
                continue;
            };
            self.hiring.asked.insert(wage_place(&opening.work), Instant::now());
            let work = wage::words(&catalog, &opening.work);
            let pass = menu.len() - 1;
            return match self.decide(question, menu, pass, holdings) {
                Some((d, Choice::Hire { .. })) => match (d.price, d.quantity) {
                    (Some(rate), Some(cap)) => Some(HireAct::Offer { opening, rate, cap, work }),
                    _ => None,
                },
                _ => None,
            };
        }
        None
    }

    /// Does what it chose about hiring.
    pub(super) fn hire_act(&mut self, act: &HireAct) -> Result<(), BotError> {
        match act {
            HireAct::Offer { opening, rate, cap, work } => self.offer_work(opening, *rate, *cap, work),
            HireAct::Collect { station } => self.collect_box(*station),
        }
    }

    /// Offers `opening`'s work: for a craft, at the station with its box
    /// stocked for `cap` units, and no more than it stocked. Never more
    /// than its Strands over its line escrow, whatever a layer said.
    fn offer_work(&mut self, opening: &Opening, rate: i64, cap: i32, work: &str) -> Result<(), BotError> {
        let budget = self.bot.state().strands().unwrap_or(0) - self.hire_line();
        let mut cap = wage::affordable(rate, cap.min(opening.most), budget);
        if cap <= 0 {
            return Err(BotError::Failed("no longer over its line".into()));
        }
        if let Work::Craft { station_id, recipe_id } = &opening.work {
            let station = self
                .stations
                .values()
                .find(|s| s.id == *station_id)
                .cloned()
                .ok_or_else(|| BotError::Failed(format!("its station {station_id} is gone")))?;
            self.bot.walk_to(station.x, station.y + 10.0)?;
            let catalog = self.bot.state().catalog().clone();
            // Whatever goes in comes home after, the offer up or not
            self.hiring.collect.retain(|s| s != station_id);
            self.hiring.collect.push(*station_id);
            let mut crafts = i32::MAX;
            let recipe = catalog.recipes().iter().find(|r| &r.id == recipe_id).cloned();
            for (item, want) in opening.stock_for(&catalog, cap) {
                let put = self.bot.stock_station(*station_id, &item, want)?;
                let per = recipe
                    .as_ref()
                    .and_then(|r| r.inputs.iter().find(|i| i.item_type == item))
                    .map_or(1, |i| i.quantity.max(1));
                crafts = crafts.min(put / per);
            }
            let made = recipe.as_ref().and_then(|r| r.outputs.first()).map_or(1, |o| o.quantity_min.max(1));
            cap = cap.min(crafts.saturating_mul(made));
            if cap <= 0 {
                return Err(BotError::Failed(format!("nothing to stock the {} with", station.station_type)));
            }
        }
        let offer = self.bot.offer_wages(capitol_protocol::rest::OfferWages {
            work: opening.work.clone(),
            rate,
            cap,
            term_days: TERM_DAYS,
            worker_id: None,
        })?;
        self.entries.push(Entry::Hired {
            work: work.to_string(),
            rate,
            cap,
            fee: offer.fee,
        });
        self.hiring.open.push(offer);
        Ok(())
    }

    /// Brings home what's in station `station`'s box, a hire over.
    fn collect_box(&mut self, station: Uuid) -> Result<(), BotError> {
        self.hiring.collect.retain(|s| *s != station);
        let Some(s) = self.stations.get(&station).cloned() else {
            return Ok(());
        };
        self.bot.walk_to(s.x, s.y + 10.0)?;
        self.bot.empty_station(station).map(drop)
    }

    /// Takes down any offer whose place its stint wants back, and every one
    /// once its money runs low (under half its line). Before the round's
    /// business; a call that fails is tried again next round, and only a
    /// final error ends the round.
    pub(super) fn mind_hires(&mut self) -> Result<(), BotError> {
        if self.hiring.open.is_empty() {
            return Ok(());
        }
        let catalog = self.bot.state().catalog().clone();
        let job = self.job();
        let low = self.bot.state().strands().unwrap_or(0) < self.hire_line() / 2;
        let ending: Vec<(Uuid, &'static str)> = self
            .hiring
            .open
            .iter()
            .filter_map(|o| {
                if wage::wants_back(job, &catalog, &o.work) {
                    Some((o.id, "wanted"))
                } else {
                    low.then_some((o.id, "low"))
                }
            })
            .collect();
        for (id, why) in ending {
            self.take_down(id, why)?;
        }
        Ok(())
    }

    /// Takes every offer of its down, `why` in a word: `day` at dusk,
    /// `signed_off` as its session ends.
    pub(super) fn end_hires(&mut self, why: &str) -> Result<(), BotError> {
        let ids: Vec<Uuid> = self.hiring.open.iter().map(|o| o.id).collect();
        for id in ids {
            self.take_down(id, why)?;
        }
        Ok(())
    }

    /// Brings home every box a hire left, best done on the walk home: its
    /// stations are on its lot. A box it can't empty (a full pack) is left.
    pub(super) fn collect_boxes(&mut self) -> Result<(), BotError> {
        while let Some(&station) = self.hiring.collect.first() {
            match self.collect_box(station) {
                Ok(()) => {}
                Err(e) if e.is_final() || matches!(e, BotError::Offline(_)) => return Err(e),
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// Takes offer `id` down. One the server closed already (at the cap,
    /// its term) is left to its `wage_closed`.
    fn take_down(&mut self, id: Uuid, why: &str) -> Result<(), BotError> {
        match self.bot.withdraw_wages(id) {
            Ok(offer) => {
                self.hire_closed(&offer, why);
                Ok(())
            }
            // Closed already (at the cap, say), unheard: as it was last heard
            Err(BotError::Rest { source, .. }) if source.status().is_some_and(|s| s < 500) => {
                if let Some(known) = self.hiring.open.iter().find(|o| o.id == id).cloned() {
                    self.hire_closed(&known, why);
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// An offer as the board now has it (`wage_offered`, `wage_updated`,
    /// `wage_closed`): one of its own, kept up to date, and closed when the
    /// server closed it.
    pub(super) fn hire_heard(&mut self, offer: &WageOffer) {
        let Some(mine) = self.hiring.open.iter_mut().find(|o| o.id == offer.id) else {
            return;
        };
        *mine = offer.clone();
        if offer.status != OfferStatus::Open {
            self.hire_closed(offer, offer.status.as_str());
        }
    }

    /// Offer `offer` of its closed, `why` in a word: what was made on it,
    /// paid from its escrow, and the rest come back. A craft's box is
    /// brought home.
    fn hire_closed(&mut self, offer: &WageOffer, why: &str) {
        let Some(at) = self.hiring.open.iter().position(|o| o.id == offer.id) else {
            return;
        };
        self.hiring.open.remove(at);
        let made = offer.made.clamp(0, offer.cap);
        let catalog = self.bot.state().catalog().clone();
        self.entries.push(Entry::HireEnded {
            work: wage::words(&catalog, &offer.work),
            why: why.to_string(),
            made,
            paid: offer.rate * made as i64,
            refunded: offer.rate * (offer.cap - made) as i64,
        });
        if let Work::Craft { station_id, .. } = offer.work {
            if !self.hiring.collect.contains(&station_id) {
                self.hiring.collect.push(station_id);
            }
        }
    }
}

/// Where `work` is done: its station, or its lot.
fn wage_place(work: &Work) -> Uuid {
    crate::wages::place_of(work)
}
