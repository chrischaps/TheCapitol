//! A citizen in the guilds (Commons S4.5): it asks to join a guild for its
//! trade, pays into its treasury, and leaves one that's going under. It
//! never founds one or takes the lead: both need a subscription, and a
//! citizen has none (citizens plan D2).
//!
//! - **Reading.** Its guild, its place and the invitations that name it
//!   (`GET /player/guild`) are read every [`READ_EVERY`], and again after a
//!   guild message names it: a roster change, an invitation, debt, the end.
//! - **Joining.** A citizen in no guild is asked once a game day
//!   ([`Question::Guild`]) about the guilds whose name or charter names its
//!   trade ([`guild::names_trade`]), and at once when a guild invites it,
//!   with that guild on the menu. It asks to join one, or stays out and
//!   declines the invitation. An officer or the leader takes it in, a
//!   person; an invitation it accepts takes it in at once. It says so in the
//!   guild's channel as it joins.
//! - **Paying in.** On the guild day, once a game week, after its working
//!   hours or as it signs off, it pays in a tenth of what it gained over the
//!   day, from what it doesn't keep back ([`guild::deposit`]).
//! - **Leaving.** It leaves a guild whose debt for dues passes two weeks of
//!   them ([`guild::too_deep`]).
//!
//! Each is a call from wherever it stands, between its other business: no
//! walk, and no round of its day. A guild's hall, its stations and its
//! steps as a place to be wait on S4.3
//! ([#510](https://github.com/chrischaps/TheCapitol/issues/510)).

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use capitol_protocol::rest::{GuildDetail, GuildInviteKind, GuildSummary, MyGuild};
use capitol_protocol::rules::day_number;
use uuid::Uuid;

use super::super::guild;
use super::super::plan::Holdings;
use super::super::report::Entry;
use super::super::kept;
use super::SimBot;
use crate::bot::BotError;
use crate::citizen::decide::{Choice, Question};
use crate::citizen::phrases::Occasion;
use crate::citizen::schedule::Phase;
use crate::protocol::ws::GuildNoticeKind;
use crate::state::StateEvent;

/// How often a citizen reads its guild, at most, in game seconds.
const READ_EVERY: f64 = 3600.0;

/// A citizen's guild as it last read it, and what it has done about guilds.
pub(super) struct Guilds {
    /// Its guild as last read: `None` until the first read
    mine: Option<MyGuild>,
    /// When it last read it; `None` makes the next round read again
    read_at: Option<Instant>,
    /// The game day it was last asked about joining
    asked_day: Option<i64>,
    /// Invitations it has been asked about, by guild
    answered: HashSet<Uuid>,
    /// Charters it has read, by guild
    charters: HashMap<Uuid, String>,
    /// The game day it's in, and its Strands as it began, or as it signed in
    day: Option<(i64, i64)>,
    /// The game week it last paid in
    paid_week: Option<i64>,
    /// The game day it last tried to leave: a leader can't while others
    /// are in, so it tries once a day, not every round
    leave_day: Option<i64>,
    /// Guilds it left itself, so their closing isn't counted twice
    left: HashSet<Uuid>,
}

impl Guilds {
    pub(super) fn new() -> Guilds {
        Guilds {
            mine: None,
            read_at: None,
            asked_day: None,
            answered: HashSet::new(),
            charters: HashMap::new(),
            day: None,
            paid_week: None,
            leave_day: None,
            left: HashSet::new(),
        }
    }

    /// Its guild, if it's in one.
    fn guild(&self) -> Option<&GuildDetail> {
        self.mine.as_ref()?.guild.as_ref()
    }
}

/// What a citizen does about guilds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuildAct {
    /// Ask to join guild `id`, `name`, or accept its invitation
    Apply { id: Uuid, name: String },
    /// Decline the invitations of these guilds: it stays out of them
    Decline { ids: Vec<Uuid> },
    /// Pay `strands` into its guild's treasury
    Deposit { id: Uuid, name: String, strands: i64 },
    /// Leave its guild, deep in debt
    Leave { id: Uuid, name: String },
}

impl GuildAct {
    /// What the health line calls it: `join a guild: The Ore Guild`.
    pub(super) fn label(&self) -> String {
        match self {
            GuildAct::Apply { name, .. } => format!("join a guild: {name}"),
            GuildAct::Decline { .. } => "decline a guild's invitation".into(),
            GuildAct::Deposit { name, strands, .. } => format!("pay {strands} into {name}"),
            GuildAct::Leave { name, .. } => format!("leave {name}"),
        }
    }
}

impl SimBot {
    /// Reads its guild, if a read is due. A guild it can't read is as it
    /// was until the next read; only a final error ends the round.
    pub(super) fn read_guild(&mut self) -> Result<(), BotError> {
        if !self.people || self.guilds.read_at.is_some_and(|t| t.elapsed() < self.wall(READ_EVERY)) {
            return Ok(());
        }
        self.guilds.read_at = Some(Instant::now());
        match self.bot.my_guild() {
            Ok(mine) => {
                self.guild_read(mine);
                Ok(())
            }
            Err(e) if e.is_final() => Err(e),
            Err(_) => Ok(()),
        }
    }

    /// Its guild as the server now has it. Joining one is said in its
    /// channel; the first read of a life only learns where it stands.
    fn guild_read(&mut self, mine: MyGuild) {
        let had = self.guilds.mine.as_ref().map(|m| m.guild.as_ref().map(|g| g.id));
        let joined = match (had, &mine.guild) {
            (Some(None), Some(g)) => Some((g.id, g.name.clone())),
            (Some(Some(was)), Some(g)) if was != g.id => Some((g.id, g.name.clone())),
            _ => None,
        };
        self.guilds.mine = Some(mine);
        if let Some((id, name)) = joined {
            self.guilds.left.remove(&id);
            self.entries.push(Entry::Joined { guild: name.clone() });
            self.bot.speak_to_guild(Occasion::Joined, &name, id);
        }
    }

    /// What a guild message tells it: read again, and count a guild it's
    /// out of.
    pub(super) fn guild_event(&mut self, change: &StateEvent) {
        match change {
            StateEvent::GuildChanged { .. } | StateEvent::GuildDues { .. } => self.guilds.read_at = None,
            StateEvent::GuildNotice { kind, guild, guild_id, .. } => {
                self.guilds.read_at = None;
                if *kind == GuildNoticeKind::Expelled {
                    self.guilds.left.insert(*guild_id);
                    self.entries.push(Entry::LeftGuild {
                        guild: guild.clone(),
                        why: "expelled".into(),
                    });
                }
            }
            StateEvent::GuildDissolved { guild_id, guild, share, .. } => {
                self.guilds.read_at = None;
                if self.guilds.left.insert(*guild_id) {
                    self.entries.push(Entry::LeftGuild {
                        guild: guild.clone(),
                        why: "dissolved".into(),
                    });
                }
                if *share > 0 {
                    self.entries.push(Entry::GuildShare {
                        guild: guild.clone(),
                        strands: *share,
                    });
                }
            }
            _ => {}
        }
    }

    /// What to do about guilds now: leave one going under, pay in on the
    /// guild day, or the mind's answer to whether to join one. `None`:
    /// nothing. `Err` only for a read that failed for good.
    pub(super) fn choose_guild(&mut self, holdings: &Holdings, phase: Option<Phase>) -> Result<Option<GuildAct>, BotError> {
        if !self.people || self.guilds.mine.is_none() {
            return Ok(None);
        }
        let Some(today) = self.bot.state().server_time_ms().map(day_number) else {
            return Ok(None);
        };
        if self.guilds.day.is_none_or(|(day, _)| day != today) {
            self.guilds.day = Some((today, holdings.strands));
        }
        if let Some(g) = self.guilds.guild() {
            if g.treasury.as_ref().is_some_and(guild::too_deep) && self.guilds.leave_day != Some(today) {
                let act = GuildAct::Leave {
                    id: g.id,
                    name: g.name.clone(),
                };
                self.guilds.leave_day = Some(today);
                return Ok(Some(act));
            }
            // After its working hours on the guild day
            let evening = matches!(phase, Some(Phase::Home | Phase::Night));
            return Ok(if evening { self.pay_in(holdings, today) } else { None });
        }
        self.ask_to_join(holdings, today)
    }

    /// Its deposit on the guild day, once a game week, if it gained enough
    /// to pay one; marks the week paid either way.
    fn pay_in(&mut self, holdings: &Holdings, today: i64) -> Option<GuildAct> {
        let g = self.guilds.guild()?;
        let week = guild::week_of(today);
        if !guild::guild_day(today) || self.guilds.paid_week == Some(week) {
            return None;
        }
        let (id, name) = (g.id, g.name.clone());
        self.guilds.paid_week = Some(week);
        let began = self.guilds.day.filter(|(day, _)| *day == today).map_or(holdings.strands, |(_, s)| s);
        let strands = guild::deposit(holdings.strands - began, holdings.strands, kept(holdings));
        (strands > 0).then_some(GuildAct::Deposit { id, name, strands })
    }

    /// Whether to join a guild: once a game day, and at once on an
    /// invitation it hasn't been asked about.
    fn ask_to_join(&mut self, holdings: &Holdings, today: i64) -> Result<Option<GuildAct>, BotError> {
        let Some(mine) = self.guilds.mine.clone() else { return Ok(None) };
        let invited: Vec<Uuid> = mine
            .invites
            .iter()
            .filter(|i| i.kind == GuildInviteKind::Invite && !self.guilds.answered.contains(&i.guild_id))
            .map(|i| i.guild_id)
            .collect();
        if invited.is_empty() && self.guilds.asked_day == Some(today) {
            return Ok(None);
        }
        self.guilds.asked_day = Some(today);
        self.guilds.answered.extend(invited.iter().copied());
        // Guilds it has asked to join already wait on their officers
        let applied: HashSet<Uuid> = mine.invites.iter().filter(|i| i.kind == GuildInviteKind::Apply).map(|i| i.guild_id).collect();
        let list = match self.bot.guilds() {
            Ok(list) => list,
            Err(e) if e.is_final() => return Err(e),
            Err(_) => return Ok(None),
        };
        // Each guild's charter, read once a life
        let unread: Vec<Uuid> = list
            .iter()
            .map(|g| g.id)
            .filter(|id| !applied.contains(id) && !self.guilds.charters.contains_key(id))
            .collect();
        for id in unread {
            match self.bot.guild(id) {
                Ok(detail) => {
                    self.guilds.charters.insert(id, detail.charter);
                }
                Err(e) if e.is_final() => return Err(e),
                Err(_) => {}
            }
        }
        let listed: Vec<(&GuildSummary, &str)> = list
            .iter()
            .filter_map(|g| Some((g, self.guilds.charters.get(&g.id)?.as_str())))
            .collect();
        // The guilds of every trade it's willing to work at, its favourite's
        // first (Callings K1.2)
        let seen = guild::seen(&self.willing(), &listed, &invited, &applied);
        if seen.is_empty() {
            return Ok(None);
        }
        let menu = guild::menu(&seen);
        let stay = menu.len() - 1;
        let question = Question::Guild {
            guilds: seen.iter().map(|(_, g)| g.clone()).collect(),
        };
        Ok(match self.decide(question, menu, stay, holdings) {
            Some((_, Choice::Apply { guild: name })) => {
                seen.iter().find(|(_, g)| g.name == name).map(|(id, _)| GuildAct::Apply { id: *id, name })
            }
            _ if !invited.is_empty() => Some(GuildAct::Decline { ids: invited }),
            _ => None,
        })
    }

    /// Its deposit as it signs off on the guild day, if it hasn't paid in
    /// this week (S4.5): a citizen whose evening isn't on the server pays
    /// in before it goes.
    pub(super) fn pay_in_on_leaving(&mut self) -> Result<(), BotError> {
        let Some(today) = self.bot.state().server_time_ms().map(day_number) else {
            return Ok(());
        };
        let holdings = self.holdings();
        match self.pay_in(&holdings, today) {
            Some(act) => self.guild_act(&act),
            None => Ok(()),
        }
    }

    /// Does what it chose about guilds.
    pub(super) fn guild_act(&mut self, act: &GuildAct) -> Result<(), BotError> {
        match act {
            GuildAct::Apply { id, name } => {
                let mine = self.bot.apply_to_guild(*id)?;
                self.entries.push(Entry::Applied { guild: name.clone() });
                self.guild_read(mine);
            }
            GuildAct::Decline { ids } => {
                for id in ids {
                    let mine = self.bot.decline_guild(*id)?;
                    self.guild_read(mine);
                }
            }
            GuildAct::Deposit { id, name, strands } => {
                let mine = self.bot.deposit_to_guild(*id, *strands)?;
                self.entries.push(Entry::PaidIn {
                    guild: name.clone(),
                    strands: *strands,
                });
                self.guild_read(mine);
            }
            GuildAct::Leave { id, name } => {
                let mine = self.bot.leave_guild(*id)?;
                self.guilds.left.insert(*id);
                self.entries.push(Entry::LeftGuild {
                    guild: name.clone(),
                    why: "debt".into(),
                });
                self.guild_read(mine);
            }
        }
        Ok(())
    }
}
