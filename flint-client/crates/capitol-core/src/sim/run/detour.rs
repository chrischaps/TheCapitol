//! A citizen's detours (Callings K1.5, plan D2): about a quarter hour off
//! its work for something unrelated, as people do. What's on offer is
//! [`citizen::detour`](crate::citizen::detour)'s, and the rules weigh it by
//! the calling's `wander`; this is the walk there, the thing itself, and
//! the time there.
//!
//! It's on the stint menu as a session starts and a stint ends
//! ([`SimBot::mind_stint`]), and on the free-time menu ([`SimBot::idle_time`]),
//! in working hours only, never with a claim on the board to keep or on a
//! wage's shift, and not again within [`DETOUR_AGAIN`] of the last. Its
//! time is filed under [`Activity::Detour`](crate::sim::activity::Activity::Detour).

use std::collections::HashSet;
use std::time::Instant;

use capitol_protocol::rules::talk;
use uuid::Uuid;

use super::super::plan::{Holdings, Step};
use super::SimBot;
use crate::bot::BotError;
use crate::citizen::decide::Choice;
use crate::citizen::detour::{self, Detour, DetourKind, Open, DETOUR_SECS, NEIGHBOUR_REACH, PARTNER_REACH, WANTED};
use crate::citizen::idle::{Place, Places};
use crate::citizen::phrases::Occasion;
use crate::citizen::schedule::Phase;
use crate::citizen::town::{Standing, Tether};
use crate::net::RestRequest;

type Point = (f64, f64);

/// Game hours after a detour before another is on its menus: a middling
/// stint's, so even a citizen whose stints keep getting stuck spends no
/// more than about a seventh of its time wandering.
pub(super) const DETOUR_AGAIN: f64 = 1.5;
/// The least time it spends where a detour took it, in game seconds, when
/// the walk there took most of the quarter hour.
const THERE_AT_LEAST: f64 = 60.0;
/// How far from a lot's middle to look for the road in front of it.
const GATE_REACH: f64 = 200.0;

impl SimBot {
    /// Whether a detour may go on its menus now (K1.5): a citizen's, with a
    /// taste for them, in its working hours (`phase`), with nothing
    /// promised on the board, on no shift, and none in the last
    /// [`DETOUR_AGAIN`].
    pub(super) fn detours_open(&self, phase: Option<Phase>) -> bool {
        self.persona.is_some()
            && self.calling.wander > 0.0
            && phase == Some(Phase::Work)
            && !self.night_shift
            && !self.wages.on_shift()
            && !self.board.owing_any()
            && self.last_detour.is_none_or(|t| self.now() - t >= DETOUR_AGAIN)
    }

    /// The detours open to it now, as menu entries
    /// ([`detour::menu`]).
    pub(super) fn detours(&mut self, holdings: &Holdings) -> Vec<Choice> {
        let Ok(walks) = self.bot.walks() else {
            return Vec::new();
        };
        let places = walks.places();
        let catalog = self.bot.state().catalog().clone();
        let open = Open {
            places,
            at: self.bot.position(),
            time: self.bot.state().time_of_day(),
            seen: &self.stints.seen,
            recipe: detour::untried(&catalog, holdings, &self.stints.made),
            call: self.call_at(places),
        };
        detour::menu(&open)
    }

    /// A lot to call at: one of someone it has asked to trade (#295), within
    /// [`PARTNER_REACH`] of where it stands, else a neighbour's within
    /// [`NEIGHBOUR_REACH`] of its home, the nearest. Its id, and whether
    /// it's a trading partner's.
    fn call_at(&self, places: &Places) -> Option<(String, bool)> {
        let state = self.bot.state();
        let me = state.me()?;
        let (here, home) = (self.bot.position(), self.home());
        let partners: HashSet<Uuid> = self.asked.keys().copied().collect();
        state
            .claimed_lots()
            .iter()
            .filter(|(_, lot)| lot.owner_id != me && lot.guild.is_none())
            .filter_map(|(id, lot)| {
                let &middle = places.lots.get(id)?;
                let partner = partners.contains(&lot.owner_id);
                let (from, reach) = if partner { (here, PARTNER_REACH) } else { (home, NEIGHBOUR_REACH) };
                let d = distance(middle, from);
                (d <= reach).then(|| ((!partner, d), id.clone(), partner))
            })
            .min_by(|a, b| a.0 .0.cmp(&b.0 .0).then(a.0 .1.total_cmp(&b.0 .1)).then_with(|| a.1.cmp(&b.1)))
            .map(|(_, id, partner)| (id, partner))
    }

    /// Takes `d`: walks there, does it, and stays about until a quarter
    /// hour ([`DETOUR_SECS`]) since it set off, at least a game minute
    /// there. Someone asking to trade cuts it short, as in free time.
    pub(super) fn take_detour(&mut self, d: &Detour, town: Option<&Tether>) -> Result<(), BotError> {
        let end = Instant::now() + self.wall(DETOUR_SECS);
        self.last_detour = Some(self.now());
        let result = self.detour_to(d, town, end);
        self.spot = None;
        self.back_from_detour();
        result
    }

    fn detour_to(&mut self, d: &Detour, town: Option<&Tether>, end: Instant) -> Result<(), BotError> {
        let places = self.bot.walks()?.places().clone();
        match d.kind {
            DetourKind::Try => {
                let catalog = self.bot.state().catalog().clone();
                let station = catalog
                    .recipes()
                    .iter()
                    .find(|r| r.id == d.to)
                    .ok_or_else(|| BotError::Failed(format!("no recipe {}", d.to)))?
                    .required_station_type
                    .clone();
                self.take(&Step::Craft {
                    recipe: d.to.clone(),
                    station,
                })?;
                let here = self.bot.position();
                self.there(d, town, end, here)
            }
            DetourKind::See => {
                let place = places
                    .named(&d.to)
                    .cloned()
                    .ok_or_else(|| BotError::Failed(format!("no place {} to see", d.to)))?;
                let spot = self.stand_at(&place)?;
                self.stints.seen.insert(d.to.clone());
                self.there(d, town, end, spot)
            }
            DetourKind::Browse => {
                self.bot.go_to_bazaar()?;
                // A stall to look in at: one someone keeps, or a bare one
                let _ = self.look_in_at_a_stall();
                let spot = match &places.lane {
                    Some(lane) => self.stand_at(lane)?,
                    None => self.bot.position(),
                };
                self.there(d, town, end, spot)
            }
            DetourKind::Read => {
                self.bot.go_to_exchange()?;
                if d.to == WANTED {
                    self.bot.commissions()?;
                } else {
                    self.bot.call("reading the Gazette", RestRequest::Gazette { day: None })?;
                }
                let spot = match &places.hall_steps {
                    Some(steps) => self.stand_at(steps)?,
                    None => self.bot.position(),
                };
                self.there(d, town, end, spot)
            }
            DetourKind::Call => self.call_on(d, &places, town, end),
        }
    }

    /// To a spot of its own in `place`, looking where the spot looks.
    fn stand_at(&mut self, place: &Place) -> Result<Point, BotError> {
        let (spot, look) = place.spot(self.random(), self.random());
        self.bot.walk_to(spot.0, spot.1)?;
        self.bot.face(look.0, look.1)?;
        self.spot = Some((spot, look));
        Ok(spot)
    }

    /// There, about `spot`: what the health line says, and the rest of the
    /// quarter hour, at least [`THERE_AT_LEAST`].
    fn there(&mut self, d: &Detour, town: Option<&Tether>, end: Instant, spot: Point) -> Result<(), BotError> {
        if let Some(town) = town {
            town.health(|h| h.doing(Standing::Working, &d.there()));
        }
        let until = end.max(Instant::now() + self.wall(THERE_AT_LEAST));
        self.stay(until, town, Some(spot))
    }

    /// At the road in front of the lot `d` goes to, facing it: a word to its
    /// owner if they're about, and a look at their card, as a person learns
    /// what their neighbours do (Callings D8).
    fn call_on(&mut self, d: &Detour, places: &Places, town: Option<&Tether>, end: Instant) -> Result<(), BotError> {
        let middle = *places
            .lots
            .get(&d.to)
            .ok_or_else(|| BotError::Failed(format!("no lot {} to call at", d.to)))?;
        let owner = self.bot.state().claimed_lots().get(&d.to).map(|l| l.owner_id);
        let walks = self.bot.walks()?;
        let gate = gate(&walks.edges_near(middle, GATE_REACH), middle).unwrap_or(middle);
        self.bot.walk_to(gate.0, gate.1)?;
        self.bot.face(middle.0, middle.1)?;
        if let Some(owner) = owner {
            let about = self
                .bot
                .state()
                .players()
                .get(&owner)
                .is_some_and(|p| (p.x - gate.0).hypot(p.y - gate.1) <= talk::HEARD);
            if about {
                self.bot.speak(Occasion::Greeting, owner);
            }
            // What they do, from their card; a card it can't read is no
            // reason to leave
            match self.bot.call("reading a card", RestRequest::PlayerCard { player_id: owner }) {
                Err(e) if e.is_final() => return Err(e),
                _ => {}
            }
        }
        self.there(d, town, end, gate)
    }
}

/// The nearest point on `edges` (the roads near a lot) to its `middle`:
/// where a caller stands, at the gate.
fn gate(edges: &[(Point, Point)], middle: Point) -> Option<Point> {
    edges
        .iter()
        .map(|&(a, b)| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 == 0.0 { 0.0 } else { (((middle.0 - a.0) * dx + (middle.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
            (a.0 + dx * t, a.1 + dy * t)
        })
        .min_by(|a, b| distance(*a, middle).total_cmp(&distance(*b, middle)))
}

fn distance(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caller_stands_on_the_road_nearest_the_lot() {
        let edges = [((0.0, 0.0), (100.0, 0.0)), ((0.0, 50.0), (0.0, 200.0))];
        assert_eq!(gate(&edges, (40.0, 30.0)), Some((40.0, 0.0)));
        assert_eq!(gate(&edges, (-20.0, 120.0)), Some((0.0, 120.0)));
        assert_eq!(gate(&[], (0.0, 0.0)), None);
    }
}
