//! A citizen's idle life (C2.1): its free time spent somewhere worth being,
//! a break at noon, the company it drifts toward while it lingers, and a
//! while on a bridge on the way home at dusk. The places and the menus are
//! [`citizen::idle`](crate::citizen::idle); this is the walking and the
//! standing about. A member's guild hall is among the places (#510,
//! [`hall`](super::hall)).
//!
//! The swarm's bots have no persona, so none of this: their free time is
//! still a rest or the hall, as C1.1's was.

use std::time::{Duration, Instant};

use super::{SimBot, STOP_CHECK};
use crate::bot::BotError;
use crate::citizen::decide::{Choice, Question};
use crate::citizen::detour::Detour;
use crate::sim::activity::Activity;
use crate::sim::report::Entry;
use crate::citizen::idle::{self, Place};
use crate::citizen::schedule;
use crate::citizen::town::{Standing, Tether};
use crate::sim::plan::Holdings;

type Point = (f64, f64);

/// How often a lingering citizen looks round for company, in game seconds.
const DRIFT_EVERY: f64 = 90.0;
/// How far from its spot it will drift to join someone: about eight metres.
const DRIFT_REACH: f64 = 80.0;
/// How close it stands to them: about a metre and a quarter.
const CHAT: f64 = 12.0;
/// Within this of home is home, for the night (#276): about five metres.
const HOME_NEAR: f64 = 50.0;

impl SimBot {
    /// Free time for a citizen: where to spend it, from the places near
    /// enough (`idle::menu`) and in its working hours the detours open to
    /// it (K1.5), and then there until `until`, or for a while by its
    /// patience. A walk that doesn't get there stays where it got to; a
    /// lost connection shows at the next step. The game hours a detour
    /// took, which has an entry of its own.
    pub(super) fn idle_time(&mut self, town: Option<&Tether>, holdings: &Holdings, until: Option<Instant>) -> f64 {
        let places = match self.bot.walks() {
            Ok(walks) => walks.places().clone(),
            Err(_) => {
                self.rest(town, "resting");
                return 0.0;
            }
        };
        let at = self.bot.position();
        let stall = self.bot.state().bazaar().my_shop().map(|s| s.stall_id.clone());
        let time = self.bot.state().time_of_day();
        // Its guild's hall, for a member, within its usual walk (#510)
        let hall = self.hall_to_be_at();
        let mut menu = idle::menu(&places, at, time, stall.as_deref(), hall.as_ref().map(|(name, _)| name.as_str()));
        if self.detours_open(self.phase()) {
            menu.extend(self.detours(holdings));
        }
        let choice = self.decide(Question::FreeTime, menu, 0, holdings).map(|(_, c)| c);
        if let Some(d) = choice.as_ref().and_then(Detour::of) {
            return self.detour_in_free_time(&d, town);
        }
        let end = until.unwrap_or_else(|| Instant::now() + self.while_there());
        let result = match choice {
            Some(Choice::Hall) => self.bot.go_to_exchange().and_then(|()| {
                // At the hall anyway (C1.5)
                let _ = self.bot.collect_market_box().and_then(|_| self.post_demand());
                match places.hall_steps.clone() {
                    Some(steps) => self.linger(&steps, end, town),
                    None => self.stay(end, town, None),
                }
            }),
            // Out front of its guild's hall, and a look at what stands
            // there, for a day its own bench is busy (#510)
            Some(Choice::GuildHall { .. }) => match hall {
                Some((_, place)) => self.linger(&place, end, town).and_then(|()| self.look_round_hall()),
                None => Ok(()),
            },
            Some(Choice::Lane) => match places.lane.clone() {
                Some(lane) => self.bot.go_to_bazaar().and_then(|()| self.linger(&lane, end, town)),
                None => Ok(()),
            },
            Some(Choice::Park { park } | Choice::Bridge { bridge: park }) => match places.named(&park).cloned() {
                Some(place) => self.linger(&place, end, town),
                None => Ok(()),
            },
            Some(Choice::Stall { stall }) => self.mind_stall(&stall, end, town),
            _ => {
                if let Some(town) = town {
                    town.health(|h| h.doing(Standing::Resting, "resting"));
                }
                self.stay(end, town, None)
            }
        };
        if let (Some(town), Err(e)) = (town, &result) {
            if !matches!(e, BotError::Interrupted(_)) {
                town.health(|h| h.failed(format!("free time: {e}")));
            }
        }
        0.0
    }

    /// A detour taken in free time, with its entry, as a round's is: filed
    /// under [`Activity::Detour`], and a failure as a failed step. The game
    /// hours it took.
    fn detour_in_free_time(&mut self, d: &Detour, town: Option<&Tether>) -> f64 {
        let label = format!("a detour: {}", d.does);
        if let Some(town) = town {
            town.health(|h| h.doing(Standing::Working, &label));
        }
        let (began, day) = (self.now(), self.day());
        let result = self.take_detour(d, town);
        let hours = self.now() - began;
        self.entries.push(Entry::Spent {
            label: label.clone(),
            hours,
            ok: result.is_ok(),
            activity: Activity::Detour,
            setup: false,
            day,
        });
        if let Err(e) = result {
            if !matches!(e, BotError::Interrupted(_)) {
                if let Some(town) = town {
                    town.health(|h| h.failed(format!("{label}: {e}")));
                }
                self.entries.push(Entry::Failed { label, why: e.to_string() });
            }
        }
        hours
    }

    /// Free time in the night's work (#276): home, if it isn't, and a game
    /// minute there with the lantern lit. No hall or park at night.
    pub(super) fn night_in(&mut self, town: Option<&Tether>) {
        let (x, y) = self.home();
        let (px, py) = self.bot.position();
        if (x - px).hypot(y - py) > HOME_NEAR {
            if let Some(town) = town {
                town.health(|h| h.doing(Standing::Working, "going home"));
            }
            if let (Some(town), Err(e)) = (town, self.bot.walk_to(x, y)) {
                if !matches!(e, BotError::Interrupted(_)) {
                    town.health(|h| h.failed(format!("going home: {e}")));
                }
            }
        }
        self.rest(town, "at home");
    }

    /// The midday break (`Schedule::noon`), once a day, if it's due now:
    /// the free time until it ends. `true` if it was taken.
    pub(super) fn noon_break(&mut self, town: Option<&Tether>) -> bool {
        let (Some(persona), Some(now), Some(ms)) = (
            self.persona.as_ref(),
            self.bot.state().time_of_day(),
            self.bot.state().server_time_ms(),
        ) else {
            return false;
        };
        let Some((start, end)) = persona.schedule.noon() else {
            return false;
        };
        let today = capitol_protocol::rules::day_number(ms);
        if self.noon_day == Some(today) || now < start || now >= end {
            return false;
        }
        self.noon_day = Some(today);
        let until = Instant::now() + schedule::real(end - now);
        let holdings = self.holdings();
        let _ = self.idle_time(town, &holdings, Some(until));
        true
    }

    /// At dusk, before the walk home: a while on a bridge near the way, if
    /// there is one and the mind would sooner stop there.
    pub(super) fn by_the_bridge(&mut self, town: &Tether, home: Point) {
        let Ok(walks) = self.bot.walks() else { return };
        let at = self.bot.position();
        let Some(menu) = idle::homeward(walks.places(), at, home) else {
            return;
        };
        let holdings = self.holdings();
        if let Some((_, Choice::Bridge { bridge })) = self.decide(Question::FreeTime, menu, 0, &holdings) {
            if let Some(place) = walks.places().named(&bridge).cloned() {
                let end = Instant::now() + self.while_there();
                if let Err(e) = self.linger(&place, end, Some(town)) {
                    if !matches!(e, BotError::Interrupted(_)) {
                        town.health(|h| h.failed(format!("on the bridge: {e}")));
                    }
                }
            }
        }
    }

    /// Market hours at the hall or the bazaar with nothing to trade: a spot
    /// on the hall's steps or along the lane, and a game minute there, back
    /// to the spot if a trade took it to the door.
    pub(super) fn market_spot(&mut self, town: Option<&Tether>, bazaar: bool) -> Result<(), BotError> {
        let places = self.bot.walks()?.places().clone();
        let place = if bazaar { places.lane } else { places.hall_steps };
        let end = Instant::now() + self.wall(60.0);
        let Some(place) = place else {
            return self.stay(end, town, None);
        };
        if let (Some(town), Some(_)) = (town, self.spot) {
            town.health(|h| h.doing(Standing::Resting, place.activity()));
        }
        let here = self.bot.position();
        match self.spot {
            Some((spot, look)) if (spot.0 - here.0).hypot(spot.1 - here.1) > 3.0 => {
                self.bot.walk_to(spot.0, spot.1)?;
                self.bot.face(look.0, look.1)?;
                self.stay(end, town, Some(spot))
            }
            Some((spot, _)) => self.stay(end, town, Some(spot)),
            None => self.linger(&place, end, town),
        }
    }

    /// How long a citizen stays where it went in its free time: two game
    /// minutes, up to four for the patient.
    fn while_there(&self) -> Duration {
        self.wall(120.0 + 120.0 * self.mind.temperament.patience as f64)
    }

    /// Walks to a spot of its own in `place`, looks where the spot looks,
    /// and stays until `end`.
    fn linger(&mut self, place: &Place, end: Instant, town: Option<&Tether>) -> Result<(), BotError> {
        let (spot, look) = place.spot(self.random(), self.random());
        if let Some(town) = town {
            town.health(|h| h.doing(Standing::Working, place.going()));
        }
        self.bot.walk_to(spot.0, spot.1)?;
        self.bot.face(look.0, look.1)?;
        self.spot = Some((spot, look));
        if let Some(town) = town {
            town.health(|h| h.doing(Standing::Resting, place.activity()));
        }
        self.stay(end, town, Some(spot))
    }

    /// Its own stall: on the street in front of it, facing its counter.
    fn mind_stall(&mut self, stall: &str, end: Instant, town: Option<&Tether>) -> Result<(), BotError> {
        if let Some(town) = town {
            town.health(|h| h.doing(Standing::Working, "to its stall"));
        }
        self.bot.go_to_stall(stall)?;
        let door = self.bot.walks()?.landmarks().iter().find(|l| l.id == stall).map(|l| l.door);
        if let Some((x, y)) = door {
            self.bot.face(x, y)?;
        }
        if let Some(town) = town {
            town.health(|h| h.doing(Standing::Resting, "minding its stall"));
        }
        let here = self.bot.position();
        self.stay(end, town, Some(here))
    }

    /// Stands until `end`, or until someone asks to trade or the town says
    /// stop. Every [`DRIFT_EVERY`], a citizen standing `about` a spot may
    /// ask a citizen near it to trade (#295), or if it doesn't, and it's
    /// sociable, drift over to someone near it.
    pub(super) fn stay(&mut self, end: Instant, town: Option<&Tether>, about: Option<Point>) -> Result<(), BotError> {
        let first = DRIFT_EVERY * self.random();
        let mut drift_at = Instant::now() + self.wall(first);
        loop {
            if town.is_some_and(Tether::stopped) || self.bot.asked_to_trade() {
                return Ok(());
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            if let Some(about) = about.filter(|_| Instant::now() >= drift_at) {
                drift_at = Instant::now() + self.wall(DRIFT_EVERY);
                if !self.ask_around(about, town)? {
                    self.drift(about)?;
                }
            }
            self.bot.idle(left.min(STOP_CHECK))?;
        }
    }

    /// Over to the nearest other player standing near `about`, if the
    /// citizen's sociability says so: to beside them, turned their way.
    fn drift(&mut self, about: Point) -> Result<(), BotError> {
        if self.random() >= self.mind.temperament.sociability as f64 {
            return Ok(());
        }
        let me = self.bot.state().me();
        let (x, y) = self.bot.position();
        let company = self
            .bot
            .state()
            .players()
            .values()
            .filter(|p| Some(p.id) != me)
            .filter(|p| (p.x - about.0).hypot(p.y - about.1) <= DRIFT_REACH)
            .map(|p| (p.x, p.y))
            .min_by(|a, b| (a.0 - x).hypot(a.1 - y).total_cmp(&(b.0 - x).hypot(b.1 - y)));
        let Some((ox, oy)) = company else {
            return Ok(());
        };
        let (dx, dy) = (x - ox, y - oy);
        let d = dx.hypot(dy);
        // Beside them already
        if d <= CHAT * 1.5 {
            return self.bot.face(ox, oy);
        }
        self.bot.walk_to(ox + dx / d * CHAT, oy + dy / d * CHAT)?;
        self.bot.face(ox, oy)
    }
}
