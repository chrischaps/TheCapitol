//! A member and its guild's hall (#510, the hall parts of Commons S4.5):
//! the hall as a reason to join, a bench when its own is busy, and a place
//! to be.
//!
//! - **Near.** A hall is within a citizen's usual walk when it's no further
//!   from home than the Exchange hall ([`guild::near`]): the guild district
//!   rings the Exchange, so a hall on its side of town. The
//!   walk is the walk graph's, as a wage's is: by the route round the water
//!   by the bridges, slowed by the slope and the streams (#507, #586), each
//!   kept for the session. Which lots are halls the world says (`ClaimedPlot.guild`, kept
//!   current by `plot_claimed`), so knowing costs no call.
//! - **Benches.** A member whose own station of a kind is busy, lent out to
//!   someone it hired or worn out ([`guild::busy`]), works at its hall's,
//!   which the Guild permission lets it use (S4.3). It knows what's there
//!   from having stood at the hall: in its free time, or going to look, once
//!   a game day, when its own is busy and it doesn't know of one. A bench
//!   that refused it (a leader who set the hall to owner-only, say) isn't
//!   tried again this session. One its hire is at it lets the hire finish
//!   rather than take the offer down ([`SimBot::hall_takes`]).
//! - **Being there.** Its hall is on its free-time menu beside the
//!   Exchange's (`idle::menu`): out front, on the street, among the others.

use std::collections::HashSet;

use capitol_protocol::rest::{GuildHall, StationInfo};
use uuid::Uuid;

use super::super::guild;
use super::SimBot;
use crate::bot::BotError;
use crate::citizen::idle::Place;
use crate::paths::RouteTimes;
use crate::poll::Fetch;

type Point = (f64, f64);

/// Whether a craft's failure `e` says the station won't do for it, rather
/// than that it was short of something: the hall's permission no longer
/// admits it, the station's gone, or it's worn out (the server's words).
pub(super) fn refusal(e: &BotError) -> bool {
    let why = e.to_string();
    ["Not your station", "Station not found", "worn this station out"].iter().any(|w| why.contains(w))
}

/// A member's hall, as it last looked round it.
#[derive(Default)]
pub(super) struct Hall {
    /// The hall's lot it looked round
    lot: Option<String>,
    /// The crafting stations it saw on it
    benches: Vec<StationInfo>,
    /// The game day it last went to look for a bench
    looked: Option<i64>,
    /// The hall's stations that refused it, for the session
    refused: HashSet<Uuid>,
    /// The walks from home to the halls and the Exchange, by the route
    /// round the water (#586). Home seldom moves and halls are few, so
    /// they're kept for the session
    routes: RouteTimes,
}

impl SimBot {
    /// Its guild's hall, and the guild's name, if it's in a guild that holds
    /// one.
    fn my_hall(&self) -> Option<(GuildHall, String)> {
        let g = self.guilds.guild()?;
        Some((g.hall.clone()?, g.name.clone()))
    }

    /// Whether `to` is within its usual walk from home ([`guild::near`]),
    /// both walks by the route round the water (#586). Somewhere with no
    /// way there isn't.
    fn within_walk(&mut self, to: Point) -> bool {
        let Ok(walks) = self.bot.walks() else { return false };
        let Some(exchange) = walks.places().hall_steps.as_ref().map(|s| s.at) else {
            return false;
        };
        let home = self.home();
        let to_hall = self.hall.routes.secs(&walks, home, to);
        let to_exchange = self.hall.routes.secs(&walks, home, exchange);
        to_hall.zip(to_exchange).is_some_and(|(h, e)| guild::near(h, e))
    }

    /// The guilds whose halls are within its usual walk, for its guild
    /// question.
    pub(super) fn halls_near(&mut self) -> HashSet<Uuid> {
        let Ok(walks) = self.bot.walks() else { return HashSet::new() };
        let halls: Vec<(Uuid, Point)> = self
            .bot
            .state()
            .claimed_lots()
            .values()
            .filter_map(|c| Some((c.guild.as_ref()?.id, *walks.places().lots.get(&c.terrain_id)?)))
            .collect();
        halls.into_iter().filter(|&(_, at)| self.within_walk(at)).map(|(id, _)| id).collect()
    }

    /// Its guild's hall as a place to spend free time, if it's within its
    /// usual walk: the guild's name, and out front of it.
    pub(super) fn hall_to_be_at(&mut self) -> Option<(String, Place)> {
        let (hall, name) = self.my_hall()?;
        let lot = hall.terrain_id.clone()?;
        if !self.within_walk((hall.x, hall.y)) {
            return None;
        }
        let place = self.bot.walks().ok()?.places().guild_hall(&name, &lot)?;
        Some((name, place))
    }

    /// Notes the crafting stations on its hall's lot, standing at it: the
    /// nearby view, fetched again, holds them.
    pub(super) fn look_round_hall(&mut self) -> Result<(), BotError> {
        let Some(lot_id) = self.my_hall().and_then(|(h, _)| h.terrain_id) else {
            return Ok(());
        };
        self.bot.refetch(Fetch::NearbyStations);
        self.bot.settle()?;
        let walks = self.bot.walks()?;
        let Some(lot) = walks.places().hall_lots.get(&lot_id) else {
            return Ok(());
        };
        self.hall.benches = self
            .bot
            .state()
            .stations()
            .values()
            .filter(|s| s.category == "crafting" && lot.contains((s.x, s.y)))
            .cloned()
            .collect();
        self.hall.lot = Some(lot_id);
        Ok(())
    }

    /// Its hall's bench of `kind` it knows of, if the hall is its guild's
    /// still and within its usual walk.
    fn hall_bench(&mut self, kind: &str) -> Option<StationInfo> {
        let (hall, _) = self.my_hall()?;
        if hall.terrain_id.is_none() || self.hall.lot != hall.terrain_id || !self.within_walk((hall.x, hall.y)) {
            return None;
        }
        guild::hall_bench(kind, &self.hall.benches, &self.hall.refused).cloned()
    }

    /// The stations its open offers of work are at: lent out.
    fn lent(&self) -> Vec<Uuid> {
        self.hiring.lent()
    }

    /// Where it works at `kind`: its own station ([`SimBot::own`]), unless
    /// that's busy ([`guild::busy`]) and its hall, within its usual walk,
    /// has one. A member that doesn't know of one goes to look, once a game
    /// day. `None` only when it has none of its own and the hall has none.
    pub(super) fn bench(&mut self, kind: &str) -> Result<Option<StationInfo>, BotError> {
        let own = self.own(kind).cloned();
        let lent = self.lent();
        let busy = own.as_ref().is_none_or(|s| {
            let condition = self.bot.state().station(s.id).map_or(s.condition, |s| s.condition);
            guild::busy(s.id, condition, &lent)
        });
        if !busy {
            return Ok(own);
        }
        if let Some(bench) = self.hall_bench(kind) {
            return Ok(Some(bench));
        }
        let Some(place) = self.hall_to_be_at().map(|(_, p)| p) else {
            return Ok(own);
        };
        let today = self.day();
        if self.hall.looked == Some(today) {
            return Ok(own);
        }
        self.hall.looked = Some(today);
        let (spot, _) = place.spot(0.5, 0.5);
        self.bot.walk_to(spot.0, spot.1)?;
        self.look_round_hall()?;
        Ok(self.hall_bench(kind).or(own))
    }

    /// If `station` is one of its hall's, it refused it ([`refusal`]), and
    /// isn't tried again this session.
    pub(super) fn hall_refused(&mut self, station: Uuid) {
        if self.hall.benches.iter().any(|s| s.id == station) {
            self.hall.refused.insert(station);
        }
    }

    /// Whether its hall would take the work at a station of `kind` of its
    /// own that it has lent out, so its stint can have it back without
    /// taking the hire's offer down.
    pub(super) fn hall_takes(&mut self, kind: &str) -> bool {
        self.hall_bench(kind).is_some()
    }
}
