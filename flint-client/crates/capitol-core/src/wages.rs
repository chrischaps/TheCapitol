//! Piece-rate wages (Commons S2.7, plan D7), as the client keeps them:
//! every open offer for the board's Work page, the shift you're working,
//! the offer you're writing, and the calls for them.
//!
//! [`Wages`] is pure, as `commissions::Board` is. The offer being written
//! is a [`Draft`] the panel writing it keeps (the station's Hire tab, the
//! plot panel's), as the automation form is. The session makes the call
//! [`Wages::wants`] names, one at a time, and hands back the answer. The
//! offers are fetched when you sign in (`GET /wages`, which also says the
//! shift you're on), then kept live from `wage_offered`, `wage_updated` and
//! `wage_closed`, which go to everyone. `wage_paid` comes to the worker as
//! each unit is made, and `wage_shift_ended` to both sides.
//!
//! Offering, withdrawing and stopping work from anywhere. Taking an offer
//! walks you to the work first and takes it on arrival: the session
//! [`Wages::hold`]s the take while you walk, as the board holds a delivery.
//!
//! The numbers are the court's own: the escrow is `rules::wages::offer_value`
//! and the fee `rules::court::court_fee` on it, as the server charges. The
//! words are here too, so they're tested without egui.

use std::collections::VecDeque;

use capitol_protocol::rest::{
    Bounds, OfferWages, PlayerCard, ResourceNodeInfo, ShiftEnded, WageOffer, WageShift, WagesResponse,
};
use capitol_protocol::rules::court;
use capitol_protocol::rules::wages::{self, OfferStatus, ShiftEnd, Work};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::catalog::Catalog;

/// A call [`Wages`] wants made.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// `GET /wages`: every open offer, and your shift
    List,
    /// `POST /wages`
    Offer(OfferWages),
    /// `DELETE /wages/{id}`: take your offer down
    Withdraw(Uuid),
    /// `POST /wages/{id}/take`
    Take(Uuid),
    /// `DELETE /wages/{id}/shift`: stop the shift on it, as worker or employer
    Stop(Uuid),
    /// `GET /players/by-name/{name}`: who a named offer is for
    Lookup(String),
}

impl Ask {
    fn reads(&self) -> bool {
        matches!(self, Ask::List | Ask::Lookup(_))
    }
}

/// Who a named offer is for, as far as the server has said.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Named {
    /// Nothing asked yet
    #[default]
    Unasked,
    /// Asked after this name
    Asking(String),
    /// The player of that name
    Found { asked: String, id: Uuid, name: String, citizen: bool },
    /// No one is called this
    Unknown(String),
}

/// An offer being written, before it goes up.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub work: Work,
    /// Strands a unit
    pub rate: i64,
    /// The most units paid for
    pub cap: i32,
    /// It runs this many days, [`wages::MIN_TERM_DAYS`] to [`wages::MAX_TERM_DAYS`]
    pub term_days: i64,
    /// Only for the one named, or for anyone
    pub named: bool,
    /// The name typed, for a named offer
    pub name: String,
}

impl Draft {
    /// A fresh offer for `work` at `rate` a unit (the output's price, say).
    pub fn new(work: Work, rate: i64) -> Self {
        Draft { work, rate: rate.max(1), cap: 20, term_days: 3, named: false, name: String::new() }
    }

    /// What it escrows: `rate × cap`.
    pub fn value(&self) -> i64 {
        wages::offer_value(self.rate, self.cap)
    }

    /// The court's fee on top, the server's own formula.
    pub fn fee(&self) -> i64 {
        court::court_fee(self.value())
    }

    /// What offering it takes from your balance: the escrow and the fee.
    pub fn total(&self) -> i64 {
        self.value().saturating_add(self.fee())
    }

    pub fn request(&self, worker_id: Option<Uuid>) -> OfferWages {
        OfferWages {
            work: self.work.clone(),
            rate: self.rate,
            cap: self.cap,
            term_days: self.term_days,
            worker_id: if self.named { worker_id } else { None },
        }
    }

    /// The name as it's looked up: trimmed.
    pub fn name(&self) -> &str {
        self.name.trim()
    }
}

/// The offers as the client has them, your shift, and the calls for them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Wages {
    /// Every open offer, newest first
    open: Vec<WageOffer>,
    /// The shift you're working, if you are
    shift: Option<WageShift>,
    loaded: bool,
    stale: bool,
    /// Offers, takes and the rest not yet asked, oldest first
    pending: VecDeque<Ask>,
    /// A take held until you reach the work
    held: Option<Ask>,
    /// The call that's out
    asked: Option<Ask>,
    refused: Option<String>,
    done: Option<String>,
    online: bool,
    /// Who the draft's name is
    named: Named,
    /// A shift just taken, for the session to open its station
    started: Option<WageShift>,
}

impl Wages {
    /// Signed in, or back after a drop: what was out won't be answered, so
    /// the offers are asked again.
    pub fn connected(&mut self) {
        self.online = true;
        self.asked = None;
        self.stale = true;
    }

    /// Signed out: nothing is kept.
    pub fn signed_out(&mut self) {
        *self = Wages::default();
    }

    /// The call to make next, if any. Mark it sent with [`Wages::sent`].
    pub fn wants(&self) -> Option<Ask> {
        if !self.online || self.asked.is_some() {
            return None;
        }
        if let Some(ask) = self.pending.front() {
            return Some(ask.clone());
        }
        (self.stale || !self.loaded).then_some(Ask::List)
    }

    pub fn sent(&mut self, ask: Ask) {
        if self.pending.front() == Some(&ask) {
            self.pending.pop_front();
        }
        if ask == Ask::List {
            self.stale = false;
        }
        self.asked = Some(ask);
    }

    /// `GET /wages` came back: the offers as they stand, and your shift.
    pub fn listed(&mut self, response: WagesResponse) {
        if self.asked == Some(Ask::List) {
            self.asked = None;
        }
        self.open = response.offers.into_iter().filter(|o| o.status == OfferStatus::Open).collect();
        self.open.sort_by_key(|o| std::cmp::Reverse((o.posted_at, o.id)));
        self.shift = response.shift;
        self.loaded = true;
    }

    /// An offer as the server last said it (`wage_offered`, `wage_updated`,
    /// `wage_closed`, or an answer). An open one goes on the page, or
    /// replaces itself there; a closed one comes off. Your shift on it
    /// follows it.
    pub fn apply(&mut self, offer: WageOffer, me: Option<Uuid>) {
        if let Some(shift) = &self.shift {
            if shift.offer_id == offer.id {
                match &offer.shift {
                    Some(s) if Some(s.worker_id) == me => self.shift = Some(s.clone()),
                    _ => self.shift = None,
                }
            }
        } else if let Some(s) = offer.shift.as_ref().filter(|s| Some(s.worker_id) == me) {
            self.shift = Some(s.clone());
        }
        let at = self.open.iter().position(|o| o.id == offer.id);
        if offer.status != OfferStatus::Open {
            if let Some(i) = at {
                self.open.remove(i);
            }
            return;
        }
        match at {
            Some(i) => self.open[i] = offer,
            None => {
                let i = self.open.partition_point(|o| (o.posted_at, o.id) > (offer.posted_at, offer.id));
                self.open.insert(i, offer);
            }
        }
    }

    /// `wage_paid`: units made on your shift and paid. `made` is the
    /// offer's total so far.
    pub fn paid(&mut self, offer_id: Uuid, made: i32) {
        let Some(offer) = self.open.iter_mut().find(|o| o.id == offer_id) else {
            return;
        };
        let more = (made - offer.made).max(0);
        offer.made = made.max(offer.made);
        offer.escrow_strands = offer.escrow_strands.saturating_sub(offer.rate.saturating_mul(more as i64)).max(0);
        if let Some(shift) = self.shift.as_mut().filter(|s| s.offer_id == offer_id) {
            shift.made += more;
            offer.shift = Some(shift.clone());
        }
    }

    /// `wage_shift_ended`: yours, or one on an offer of yours.
    pub fn shift_ended(&mut self, offer_id: Uuid, shift: &WageShift) {
        if self.shift.as_ref().is_some_and(|s| s.id == shift.id) {
            self.shift = None;
        }
        if let Some(offer) = self.open.iter_mut().find(|o| o.id == offer_id) {
            if offer.shift.as_ref().is_some_and(|s| s.id == shift.id) {
                offer.shift = None;
            }
        }
    }

    /// Your offer went up, or came down at your word.
    pub fn offered(&mut self, offer: WageOffer, me: Option<Uuid>) {
        let offering = matches!(self.asked, Some(Ask::Offer(_)));
        if matches!(self.asked, Some(Ask::Offer(_) | Ask::Withdraw(_))) {
            self.asked = None;
        }
        self.refused = None;
        self.done = Some(if offering {
            "Offered. The pay is down, held by the court.".into()
        } else {
            "Taken down. What was unspent came back.".into()
        });
        if offering {
            self.named = Named::Unasked;
        }
        self.apply(offer, me);
    }

    /// Your take was answered: the shift is yours.
    pub fn took(&mut self, shift: WageShift) {
        if matches!(self.asked, Some(Ask::Take(_))) {
            self.asked = None;
        }
        self.refused = None;
        self.done = Some("Your shift has started.".into());
        if let Some(offer) = self.open.iter_mut().find(|o| o.id == shift.offer_id) {
            offer.shift = Some(shift.clone());
        }
        self.started = Some(shift.clone());
        self.shift = Some(shift);
    }

    /// A stop was answered: what the shift came to.
    pub fn stopped(&mut self, ended: ShiftEnded) {
        if matches!(self.asked, Some(Ask::Stop(_))) {
            self.asked = None;
        }
        self.refused = None;
        self.done = Some(format!("Stopped. {} made, {} Strands paid.", ended.shift.made, ended.paid));
        self.shift_ended(ended.shift.offer_id, &ended.shift);
    }

    /// The lookup for a named offer came back.
    pub fn card(&mut self, card: PlayerCard) {
        if let Some(Ask::Lookup(asked)) = self.asked.take() {
            self.named = Named::Found { asked, id: card.id, name: card.name, citizen: card.citizen };
        }
    }

    /// The call that was out failed, saying `why` (`not_found`: a 404). A
    /// read is asked again; a lookup that found no one says so.
    pub fn failed(&mut self, why: String, not_found: bool) {
        match self.asked.take() {
            Some(Ask::List) => self.stale = true,
            Some(Ask::Lookup(asked)) => {
                self.named = if not_found { Named::Unknown(asked) } else { Named::Unasked };
            }
            Some(_) => {
                self.done = None;
                self.refused = Some(why);
            }
            None => {}
        }
    }

    /// Do `ask` as soon as it can go.
    pub fn queue(&mut self, ask: Ask) {
        if !ask.reads() {
            self.refused = None;
            self.done = None;
        }
        if !self.pending.contains(&ask) && self.asked.as_ref() != Some(&ask) {
            self.pending.push_back(ask);
        }
    }

    /// Keep a take until you reach the work: the session is walking you there.
    pub fn hold(&mut self, ask: Ask) {
        self.refused = None;
        self.done = None;
        self.held = Some(ask);
    }

    pub fn held(&self) -> Option<&Ask> {
        self.held.as_ref()
    }

    /// You reached the work: the take goes.
    pub fn release(&mut self) {
        if let Some(ask) = self.held.take() {
            self.queue(ask);
        }
    }

    /// You walked off before reaching it: the take is dropped.
    pub fn drop_held(&mut self) {
        if self.held.take().is_some() {
            self.refused = Some("You walked off, so the work wasn't taken".into());
        }
    }

    /// A shift just taken, once: the session opens its station.
    pub fn take_started(&mut self) -> Option<WageShift> {
        self.started.take()
    }

    /// Something you did is on its way, or held until you arrive.
    pub fn busy(&self) -> bool {
        self.held.is_some()
            || self.pending.iter().any(|a| !a.reads())
            || self.asked.as_ref().is_some_and(|a| !a.reads())
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn done(&self) -> Option<&str> {
        self.done.as_deref()
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// Every open offer, newest first.
    pub fn open(&self) -> &[WageOffer] {
        &self.open
    }

    pub fn offer(&self, id: Uuid) -> Option<&WageOffer> {
        self.open.iter().find(|o| o.id == id)
    }

    /// The shift you're on, if you are.
    pub fn shift(&self) -> Option<&WageShift> {
        self.shift.as_ref()
    }

    /// The shift you're on and the offer it's on, once both are known.
    pub fn working(&self) -> Option<(&WageOffer, &WageShift)> {
        let shift = self.shift.as_ref()?;
        Some((self.offer(shift.offer_id)?, shift))
    }

    /// What your shift has earned: each unit is paid the rate as it's made.
    pub fn earned(&self) -> Option<i64> {
        let (offer, shift) = self.working()?;
        Some(offer.rate.saturating_mul(shift.made as i64))
    }

    /// Your offers that are open.
    pub fn mine(&self, me: Option<Uuid>) -> Vec<&WageOffer> {
        self.open.iter().filter(|o| Some(o.employer_id) == me).collect()
    }

    /// Your open offers for `work`'s place: that station, or that lot.
    pub fn mine_at(&self, me: Option<Uuid>, place: Uuid) -> Vec<&WageOffer> {
        self.open
            .iter()
            .filter(|o| Some(o.employer_id) == me && place_of(&o.work) == place)
            .collect()
    }

    /// The offers the Work page lists: everyone's open to anyone, those
    /// named for you, and your own. One named for someone else isn't yours
    /// to see taken.
    pub fn listed_now(&self, me: Option<Uuid>) -> Vec<&WageOffer> {
        self.open
            .iter()
            .filter(|o| Some(o.employer_id) == me || o.worker_id.is_none() || o.worker_id == me)
            .collect()
    }

    /// Whether you may take `offer` now, or why not, in words.
    pub fn can_take(&self, offer: &WageOffer, me: Option<Uuid>) -> Result<(), &'static str> {
        if Some(offer.employer_id) == me {
            return Err("It's your own offer");
        }
        if offer.worker_id.is_some() && offer.worker_id != me {
            return Err("It's for someone else");
        }
        if offer.shift.is_some() {
            return Err("Someone is working it");
        }
        if self.shift.is_some() {
            return Err("You're on a shift already");
        }
        Ok(())
    }

    /// Ask who `draft`'s name is, unless that's asked or known already.
    pub fn look_up(&mut self, draft: &Draft) {
        let name = draft.name().to_owned();
        if !draft.named || name.is_empty() || self.looked_up(&name) {
            return;
        }
        self.named = Named::Asking(name.clone());
        self.queue(Ask::Lookup(name));
    }

    fn looked_up(&self, name: &str) -> bool {
        match &self.named {
            Named::Asking(n) | Named::Unknown(n) | Named::Found { asked: n, .. } => n == name,
            Named::Unasked => false,
        }
    }

    /// Who the draft's name is, as far as it's been looked up.
    pub fn named(&self) -> &Named {
        &self.named
    }

    /// The worker `d` is for, if it's named and they were found.
    fn worker(&self, d: &Draft) -> Option<Uuid> {
        match &self.named {
            Named::Found { asked, id, .. } if d.named && asked == d.name() => Some(*id),
            _ => None,
        }
    }

    /// What stops `d` going up, with `strands` in hand (`None`: not known
    /// yet), in words; `None` if it can go.
    pub fn stopped_by(&self, d: &Draft, strands: Option<i64>, me: Option<Uuid>) -> Option<String> {
        if let Err(e) = wages::validate_offer(d.rate, d.cap, d.term_days) {
            return Some(e.message().into());
        }
        if self.mine(me).len() as i64 >= wages::MAX_OPEN_OFFERS {
            return Some(format!("You have {} offers up, the most anyone may", wages::MAX_OPEN_OFFERS));
        }
        if d.named {
            match &self.named {
                _ if d.name().is_empty() => return Some("Write the name of who it's for".into()),
                Named::Found { asked, id, .. } if asked == d.name() => {
                    if Some(*id) == me {
                        return Some("You can't hire yourself".into());
                    }
                }
                Named::Unknown(n) if n == d.name() => return Some(format!("No one in the city is called {n}")),
                _ => return Some("Checking the name…".into()),
            }
        }
        match strands {
            Some(s) if s < d.total() => Some(format!("It takes {} Strands, and you have {s}", d.total())),
            _ => None,
        }
    }

    /// Offer `d`, if it can go.
    pub fn post(&self, d: &Draft, strands: Option<i64>, me: Option<Uuid>) -> Option<Ask> {
        if self.busy() || self.stopped_by(d, strands, me).is_some() {
            return None;
        }
        Some(Ask::Offer(d.request(self.worker(d))))
    }
}

/// A walk to the work before taking it: to a station's reach, or onto the
/// lot's middle about as close.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Walk {
    to: Option<[f64; 2]>,
}

/// What a click on Take does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// You're there: take it now
    Here,
    /// Walk to this server point first
    WalkTo { x: f64, y: f64 },
}

impl Walk {
    /// Take work at `to`, standing at `at`.
    pub fn click(&mut self, to: [f64; 2], at: [f64; 2]) -> Step {
        let (dx, dy) = (to[0] - at[0], to[1] - at[1]);
        let d = dx.hypot(dy);
        if d <= ARRIVE {
            self.to = None;
            return Step::Here;
        }
        self.to = Some(to);
        // Stop short, well inside the reach: a station isn't stood on
        let short = ARRIVE * 0.6 / d;
        Step::WalkTo { x: to[0] - dx * short, y: to[1] - dy * short }
    }

    /// True once, as the walk comes within reach.
    pub fn arrived(&mut self, at: [f64; 2]) -> bool {
        match self.to {
            Some(to) if (to[0] - at[0]).hypot(to[1] - at[1]) <= ARRIVE => {
                self.to = None;
                true
            }
            _ => false,
        }
    }

    /// You walked somewhere else.
    pub fn walked(&mut self) {
        self.to = None;
    }

    pub fn walking(&self) -> bool {
        self.to.is_some()
    }
}

/// The station or lot an offer's work is at.
pub fn place_of(work: &Work) -> Uuid {
    match work {
        Work::Craft { station_id, .. } => *station_id,
        Work::Gather { plot_id, .. } => *plot_id,
    }
}

/// How far a worker must come to start: a station's reach, or onto the
/// lot's middle about as close.
pub const ARRIVE: f64 = capitol_protocol::rules::STATION_INTERACTION_RANGE;

/// What the work is, in words: "Making Tool Handles", "Gathering Grass".
pub fn work_words(work: &Work, catalog: &Catalog) -> String {
    match work {
        Work::Craft { recipe_id, .. } => {
            let name = catalog.recipes().iter().find(|r| &r.id == recipe_id).map_or_else(|| recipe_id.replace('_', " "), |r| r.name.clone());
            format!("Making {name}")
        }
        Work::Gather { resource, .. } => {
            let name = catalog.resource_type(resource).map_or_else(|| resource.replace('_', " "), |r| r.name.clone());
            format!("Gathering {name}")
        }
    }
}

/// The resources that grow on a lot of `bounds`, by id, each once: what
/// gathering work there may be for. The server checks the same box, not
/// the lot's outline (#435).
pub fn resources_on<'a>(bounds: &Bounds, nodes: impl IntoIterator<Item = &'a ResourceNodeInfo>) -> Vec<String> {
    let mut found: Vec<String> = nodes
        .into_iter()
        .filter(|n| (bounds.min_x..=bounds.max_x).contains(&n.x) && (bounds.min_y..=bounds.max_y).contains(&n.y))
        .map(|n| n.resource_type.clone())
        .collect();
    found.sort();
    found.dedup();
    found
}

/// A rate to start the form at: half what the work's yield fetches at the
/// Exchange (its target price), and at least a Strand.
pub fn rate_for(work: &Work, catalog: &Catalog) -> i64 {
    let item = match work {
        Work::Craft { recipe_id, .. } => {
            catalog.recipes().iter().find(|r| &r.id == recipe_id).and_then(|r| r.outputs.first()).map(|o| o.item_type.clone())
        }
        Work::Gather { resource, .. } => catalog.resource_type(resource).map(|r| r.yields_item.clone()),
    };
    item.and_then(|i| catalog.target_price(&i)).map_or(1, |p| (p / 2).max(1))
}

/// Where it's done, in words: "at their station", "on their lot".
pub fn place_words(work: &Work) -> &'static str {
    match work {
        Work::Craft { .. } => "at their station",
        Work::Gather { .. } => "on their lot",
    }
}

/// The header's chip while you work: "Working for Ada: 12 of 40, 48 Strands".
pub fn chip_words(offer: &WageOffer, shift: &WageShift) -> String {
    let earned = offer.rate.saturating_mul(shift.made as i64);
    format!("Working for {}: {} of {}, {earned} Strands", offer.employer_name, offer.made, offer.cap)
}

/// How far and which way `to` lies from `from`, in server units: "120 m
/// north-east", or "here".
pub fn whereabouts(from: [f64; 2], to: [f64; 2]) -> String {
    const POINTS: [&str; 8] = ["east", "north-east", "north", "north-west", "west", "south-west", "south", "south-east"];
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    // A server unit is a tenth of a metre (`coords::S`)
    let metres = dx.hypot(dy) * 0.1;
    if metres < 8.0 {
        return "here".into();
    }
    // Server y points south
    let turn = (-dy).atan2(dx) / std::f64::consts::TAU;
    let way = POINTS[((turn * 8.0).round() as i64).rem_euclid(8) as usize];
    let far = if metres >= 1000.0 {
        format!("{:.1} km", metres / 1000.0)
    } else {
        let to = if metres < 100.0 { 5.0 } else { 10.0 };
        format!("{} m", ((metres / to).round() * to) as i32)
    };
    format!("{far} {way}")
}

/// How long an offer has to run, in the court's days and hours.
pub fn time_left(ends_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    crate::commissions::time_left(ends_at, now)
}

fn who(name: &str, citizen: bool) -> String {
    if citizen {
        format!("{name}, a citizen,")
    } else {
        name.to_owned()
    }
}

/// The toast to an employer whose offer was taken.
pub fn taken_words(shift: &WageShift, work: &str) -> (String, String) {
    (
        "Your work was taken".into(),
        format!("{} started a shift: {}.", who(&shift.worker_name, shift.worker_citizen), work.to_lowercase()),
    )
}

/// The toast to a worker whose shift has started.
pub fn started_words(offer: &WageOffer, work: &str) -> (String, String) {
    (
        "Your shift has started".into(),
        format!(
            "{work} for {}, {} Strands each, up to {} more. It goes into their box, and you're paid as it's made.",
            offer.employer_name,
            offer.rate,
            (offer.cap - offer.made).max(0)
        ),
    )
}

/// The toast as a shift ends: to its worker, when `worker`, or its
/// employer. The cap reached has a title of its own.
pub fn ended_words(worker: bool, shift: &WageShift, paid: i64) -> (String, String) {
    let end = shift.end.unwrap_or(ShiftEnd::Worker);
    let title = match (end, worker) {
        (ShiftEnd::Cap, _) => "All the work is done",
        (_, true) => "Your shift ended",
        (_, false) => "A shift ended",
    };
    let body = if worker {
        format!("You made {} and were paid {paid} Strands: {}.", shift.made, end.words())
    } else {
        format!("{} made {} and was paid {paid} Strands: {}.", who(&shift.worker_name, shift.worker_citizen), shift.made, end.words())
    };
    (title.into(), body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::PlayerCard;

    const ME: u128 = 1;
    const ADA: u128 = 2;

    fn me() -> Option<Uuid> {
        Some(Uuid::from_u128(ME))
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap() + chrono::Duration::minutes(minutes)
    }

    fn bench() -> Work {
        Work::Craft { station_id: Uuid::from_u128(50), recipe_id: "tool_handle".into() }
    }

    fn offer(n: u128, employer: u128, made: i32, minutes: i64) -> WageOffer {
        WageOffer {
            id: Uuid::from_u128(n),
            employer_id: Uuid::from_u128(employer),
            employer_name: "Ada".into(),
            employer_citizen: false,
            worker_id: None,
            worker_name: None,
            work: bench(),
            x: 100.0,
            y: 100.0,
            rate: 4,
            cap: 40,
            made,
            escrow_strands: 4 * (40 - made) as i64,
            fee: court::court_fee(160),
            posted_at: at(minutes),
            ends_at: at(minutes + 3 * 24 * 60),
            status: OfferStatus::Open,
            shift: None,
        }
    }

    fn shift(n: u128, on: u128, worker: u128, made: i32) -> WageShift {
        WageShift {
            id: Uuid::from_u128(n),
            offer_id: Uuid::from_u128(on),
            worker_id: Uuid::from_u128(worker),
            worker_name: "Brin".into(),
            worker_citizen: false,
            started_at: at(0),
            ended_at: None,
            made,
            end: None,
        }
    }

    /// Signed in, with `open` up and `shift` yours
    fn wages(open: Vec<WageOffer>, shift: Option<WageShift>) -> Wages {
        let mut w = Wages::default();
        assert_eq!(w.wants(), None, "not before signing in");
        w.connected();
        assert_eq!(w.wants(), Some(Ask::List));
        w.sent(Ask::List);
        assert_eq!(w.wants(), None, "one call at a time");
        w.listed(WagesResponse { offers: open, shift });
        assert_eq!(w.wants(), None);
        w
    }

    #[test]
    fn the_wage_store_follows_the_boards_events() {
        let mut w = wages(vec![offer(1, ADA, 0, 0)], None);
        w.apply(offer(2, ADA, 0, 10), me());
        let ids: Vec<u128> = w.open().iter().map(|o| o.id.as_u128()).collect();
        assert_eq!(ids, [2, 1], "newest first");
        let mut one = offer(1, ADA, 5, 0);
        one.shift = Some(shift(7, 1, 9, 5));
        w.apply(one, me());
        assert_eq!(w.offer(Uuid::from_u128(1)).unwrap().made, 5);
        assert_eq!(w.shift(), None, "someone else's shift isn't yours");
        let mut closed = offer(1, ADA, 40, 0);
        closed.status = OfferStatus::Filled;
        w.apply(closed, me());
        assert_eq!(w.open().len(), 1);
        w.connected();
        assert_eq!(w.wants(), Some(Ask::List), "a reconnect asks for the lot");
    }

    /// The header's chip follows `wage_paid`, `wage_updated` and the end.
    #[test]
    fn the_wage_chip_follows_the_events() {
        let mut taken = offer(1, ADA, 10, 0);
        taken.shift = Some(shift(7, 1, ME, 0));
        let mut w = wages(vec![taken], Some(shift(7, 1, ME, 0)));
        let (o, s) = w.working().unwrap();
        assert_eq!(chip_words(o, s), "Working for Ada: 10 of 40, 0 Strands");
        // Two units, then one more
        w.paid(Uuid::from_u128(1), 12);
        let (o, s) = w.working().unwrap();
        assert_eq!(chip_words(o, s), "Working for Ada: 12 of 40, 8 Strands");
        w.paid(Uuid::from_u128(1), 13);
        assert_eq!(w.earned(), Some(12));
        // The board's update that follows agrees, and changes nothing
        let mut updated = offer(1, ADA, 13, 0);
        updated.shift = Some(shift(7, 1, ME, 3));
        w.apply(updated, me());
        let (o, s) = w.working().unwrap();
        assert_eq!(chip_words(o, s), "Working for Ada: 13 of 40, 12 Strands");
        assert_eq!(o.escrow_strands, 4 * 27);
        // Ended: no chip
        let mut ended = shift(7, 1, ME, 3);
        ended.end = Some(ShiftEnd::Employer);
        w.shift_ended(Uuid::from_u128(1), &ended);
        assert!(w.working().is_none());
        assert!(w.offer(Uuid::from_u128(1)).unwrap().shift.is_none());
        let (title, body) = ended_words(true, &ended, 12);
        assert_eq!(title, "Your shift ended");
        assert_eq!(body, "You made 3 and were paid 12 Strands: the employer stopped it.");
    }

    #[test]
    fn a_shift_seen_on_the_board_is_yours_if_its_yours() {
        let mut w = wages(vec![offer(1, ADA, 0, 0)], None);
        let mut taken = offer(1, ADA, 0, 0);
        taken.shift = Some(shift(7, 1, ME, 0));
        w.apply(taken, me());
        assert_eq!(w.shift().map(|s| s.id.as_u128()), Some(7));
        // Closed under you
        let mut gone = offer(1, ADA, 0, 0);
        gone.status = OfferStatus::Gone;
        w.apply(gone, me());
        assert!(w.shift().is_none());
    }

    /// The hire form's preview is the court's: `offer_value` and `court_fee`.
    #[test]
    fn the_wage_preview_is_the_courts() {
        let mut d = Draft::new(bench(), 4);
        d.cap = 40;
        assert_eq!(d.value(), wages::offer_value(4, 40));
        assert_eq!(d.value(), 160);
        assert_eq!(d.fee(), court::court_fee(160));
        assert_eq!(d.fee(), 5 + 1, "the filing's least, and 0.5% rounded up");
        assert_eq!(d.total(), 166);
        d.cap = 500;
        d.rate = 10;
        assert_eq!(d.fee(), court::court_fee(5000));
        assert_eq!(d.total(), 5000 + 50 + 25);
        let r = d.request(Some(Uuid::from_u128(9)));
        assert_eq!((r.rate, r.cap, r.term_days, r.worker_id), (10, 500, 3, None), "open to anyone unless named");
    }

    #[test]
    fn what_stops_a_wage_offer_is_said() {
        let w = wages(vec![], None);
        let mut d = Draft::new(bench(), 4);
        assert_eq!(w.stopped_by(&d, Some(1000), me()), None);
        assert_eq!(w.stopped_by(&d, Some(10), me()).as_deref(), Some("It takes 86 Strands, and you have 10"));
        d.term_days = 8;
        assert_eq!(w.stopped_by(&d, Some(1000), me()).as_deref(), Some("An offer runs 1 to 7 days"));
        d.term_days = 3;
        let five: Vec<WageOffer> = (0..5).map(|n| offer(10 + n, ME, 0, n as i64)).collect();
        let full = wages(five, None);
        assert!(full.stopped_by(&d, Some(1000), me()).unwrap().contains("the most anyone may"));
        assert_eq!(full.post(&d, Some(1000), me()), None);
    }

    #[test]
    fn a_named_offer_checks_the_name_first() {
        let mut w = wages(vec![], None);
        let mut d = Draft::new(bench(), 4);
        d.named = true;
        assert_eq!(w.stopped_by(&d, Some(1000), me()).as_deref(), Some("Write the name of who it's for"));
        d.name = " Brin ".into();
        w.look_up(&d);
        assert_eq!(w.wants(), Some(Ask::Lookup("Brin".into())));
        assert_eq!(w.stopped_by(&d, Some(1000), me()).as_deref(), Some("Checking the name…"));
        w.look_up(&d);
        w.sent(Ask::Lookup("Brin".into()));
        w.card(PlayerCard {
            id: Uuid::from_u128(9),
            name: "Brin".into(),
            citizen: false,
            since: 1,
            home: None,
            standing: "new in town".into(),
            guild: None,
            mine: false,
            muted: false,
        });
        assert!(matches!(w.named(), Named::Found { id, .. } if id.as_u128() == 9));
        let Some(Ask::Offer(o)) = w.post(&d, Some(1000), me()) else { panic!("it goes") };
        assert_eq!(o.worker_id, Some(Uuid::from_u128(9)));
        // A name changed since is checked again
        d.name = "Cy".into();
        assert_eq!(w.post(&d, Some(1000), me()), None);
        w.look_up(&d);
        w.sent(Ask::Lookup("Cy".into()));
        w.failed("No such player".into(), true);
        assert_eq!(w.stopped_by(&d, Some(1000), me()).as_deref(), Some("No one in the city is called Cy"));
    }

    #[test]
    fn an_offer_goes_up_and_comes_down() {
        let mut w = wages(vec![], None);
        let ask = w.post(&Draft::new(bench(), 4), Some(1000), me()).unwrap();
        w.queue(ask.clone());
        assert!(w.busy());
        assert_eq!(w.wants(), Some(ask.clone()));
        w.sent(ask);
        w.offered(offer(5, ME, 0, 0), me());
        assert_eq!(w.done(), Some("Offered. The pay is down, held by the court."));
        assert_eq!(w.mine_at(me(), Uuid::from_u128(50)).len(), 1);
        w.queue(Ask::Withdraw(Uuid::from_u128(5)));
        let ask = w.wants().unwrap();
        w.sent(ask);
        let mut down = offer(5, ME, 0, 0);
        down.status = OfferStatus::Withdrawn;
        w.offered(down, me());
        assert_eq!(w.done(), Some("Taken down. What was unspent came back."));
        assert!(w.mine(me()).is_empty());
    }

    #[test]
    fn a_take_waits_for_the_walk() {
        let mut w = wages(vec![offer(1, ADA, 0, 0)], None);
        let o = w.offer(Uuid::from_u128(1)).unwrap().clone();
        assert_eq!(w.can_take(&o, me()), Ok(()));
        w.hold(Ask::Take(o.id));
        assert!(w.busy());
        assert_eq!(w.wants(), None, "not until you arrive");
        w.release();
        assert_eq!(w.wants(), Some(Ask::Take(o.id)));
        w.sent(Ask::Take(o.id));
        w.took(shift(7, 1, ME, 0));
        assert_eq!(w.take_started().map(|s| s.id.as_u128()), Some(7));
        assert_eq!(w.take_started(), None, "once");
        assert!(w.working().is_some());
        assert_eq!(w.can_take(&o, me()), Err("You're on a shift already"));
        let now = w.offer(o.id).unwrap().clone();
        assert_eq!(w.can_take(&now, Some(Uuid::from_u128(9))), Err("Someone is working it"));
        // Walking off drops a held take
        let mut w = wages(vec![offer(1, ADA, 0, 0)], None);
        w.hold(Ask::Take(Uuid::from_u128(1)));
        w.drop_held();
        assert_eq!(w.refused(), Some("You walked off, so the work wasn't taken"));
        assert!(!w.busy());
    }

    #[test]
    fn the_work_page_lists_what_you_may_see() {
        let mut named_other = offer(2, ADA, 0, 1);
        named_other.worker_id = Some(Uuid::from_u128(9));
        let mut named_me = offer(3, ADA, 0, 2);
        named_me.worker_id = me();
        let w = wages(vec![offer(1, ADA, 0, 0), named_other.clone(), named_me, offer(4, ME, 0, 3)], None);
        let ids: Vec<u128> = w.listed_now(me()).iter().map(|o| o.id.as_u128()).collect();
        assert_eq!(ids, [4, 3, 1]);
        assert_eq!(w.can_take(&named_other, me()), Err("It's for someone else"));
        assert_eq!(w.can_take(w.offer(Uuid::from_u128(4)).unwrap(), me()), Err("It's your own offer"));
    }

    #[test]
    fn a_walk_to_the_work_stops_in_reach() {
        let mut walk = Walk::default();
        assert_eq!(walk.click([100.0, 100.0], [140.0, 100.0]), Step::Here);
        let Step::WalkTo { x, y } = walk.click([100.0, 100.0], [1100.0, 100.0]) else { panic!("a walk") };
        assert!((x - (100.0 + ARRIVE * 0.6)).abs() < 1e-9 && (y - 100.0).abs() < 1e-9, "{x} {y}");
        assert!(walk.walking());
        assert!(!walk.arrived([500.0, 100.0]));
        assert!(walk.arrived([x, y]));
        assert!(!walk.arrived([x, y]), "once");
        walk.click([100.0, 100.0], [1100.0, 100.0]);
        walk.walked();
        assert!(!walk.walking());
    }

    #[test]
    fn a_lot_offers_what_grows_on_it() {
        let node = |n: u128, kind: &str, x: f64, y: f64| ResourceNodeInfo {
            id: Uuid::from_u128(n),
            resource_type: kind.into(),
            x,
            y,
            state: "available".into(),
        };
        let bounds = Bounds { min_x: 0.0, min_y: 0.0, max_x: 100.0, max_y: 100.0 };
        let nodes = [node(1, "grass", 10.0, 10.0), node(2, "oak_tree", 50.0, 90.0), node(3, "grass", 60.0, 60.0), node(4, "iron_vein", 150.0, 10.0)];
        assert_eq!(resources_on(&bounds, &nodes), ["grass", "oak_tree"]);
        let catalog = Catalog::fallback();
        assert!(rate_for(&bench(), &catalog) >= 1);
        assert_eq!(rate_for(&Work::Gather { plot_id: Uuid::nil(), resource: "no_such".into() }, &catalog), 1);
    }

    #[test]
    fn the_wage_words() {
        let catalog = Catalog::fallback();
        assert_eq!(work_words(&bench(), &catalog), "Making Tool Handle");
        let grass = Work::Gather { plot_id: Uuid::nil(), resource: "grass".into() };
        assert!(work_words(&grass, &catalog).starts_with("Gathering "));
        assert_eq!(whereabouts([0.0, 0.0], [30.0, 0.0]), "here");
        assert_eq!(whereabouts([0.0, 0.0], [1200.0, -1200.0]), "170 m north-east");
        assert_eq!(whereabouts([0.0, 0.0], [0.0, 15_000.0]), "1.5 km south");
        let (title, body) = taken_words(&shift(7, 1, 9, 0), "Making Tool Handles");
        assert_eq!(title, "Your work was taken");
        assert_eq!(body, "Brin started a shift: making tool handles.");
        let mut done = shift(7, 1, 9, 40);
        done.end = Some(ShiftEnd::Cap);
        done.worker_citizen = true;
        let (title, body) = ended_words(false, &done, 160);
        assert_eq!(title, "All the work is done");
        assert_eq!(body, "Brin, a citizen, made 40 and was paid 160 Strands: all the work offered is done.");
    }
}
