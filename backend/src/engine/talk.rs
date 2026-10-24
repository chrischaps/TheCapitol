//! Talk (Commons S1.1, plan D1): a local line is heard within
//! [`HEARD`] of whoever said it, a Capitol line by everyone, and a guild
//! line by the speaker's guild (S4.4).
//!
//! Nothing said is stored. The last [`CAPITOL_BACKLOG`] Capitol lines are
//! kept here, in memory, for a socket that's just subscribed, and they're
//! gone when the server stops.
//!
//! A line goes out as [`GameEvent::Said`]. Each socket decides for itself
//! whether its player hears it ([`hears`]), as it does for gathering: the
//! interest cell first, which is cheap, then the distance. A line from
//! someone the listener has muted (S1.4, `crate::mutes`) never reaches them.

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

use capitol_protocol::rules::talk::{self, CAPITOL_BACKLOG, HEARD};
use capitol_protocol::ws::{Channel, ServerMessage};
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use super::interest::{cell_at, near, Cell, Positions};
use super::GameEvent;
use crate::rate_limit::{self, Key, RateLimiter};

/// Why a watcher (E0.8) can't say a local line: you have to be somewhere to
/// be overheard
pub const WATCHER_LOCAL: &str = "Play here to talk to people nearby";

/// Why someone in no guild can't say a guild line (S4.4)
pub const NO_GUILD: &str = "Join a guild to talk to one";

/// One line said, as each socket that hears it is sent it
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    pub from: Uuid,
    pub name: String,
    pub citizen: bool,
    pub channel: Channel,
    pub text: String,
    pub x: f64,
    pub y: f64,
    /// The speaker's guild, which alone hears a guild line (S4.4)
    #[serde(skip)]
    pub guild: Option<Uuid>,
}

impl Line {
    pub fn message(&self) -> ServerMessage {
        ServerMessage::Said {
            from: self.from,
            name: self.name.clone(),
            citizen: self.citizen,
            channel: self.channel,
            text: self.text.clone(),
            x: self.x,
            y: self.y,
        }
    }
}

/// Who's talking: the player, their account (whose chat bucket the line
/// comes out of), and where they stand
#[derive(Debug, Clone)]
pub struct Speaker {
    pub player_id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub citizen: bool,
    pub x: f64,
    pub y: f64,
    /// This socket holds the avatar (E0.8). A watcher may not say `local`.
    pub holds_avatar: bool,
    /// The guild they're in (S4.4), whose members alone hear a guild line
    pub guild: Option<Uuid>,
}

/// The Capitol channel's last lines, each numbered, so a socket that took
/// the backlog can skip the copy of a line still on its way to it
#[derive(Debug, Default)]
struct Backlog {
    /// The newest line's number; the first is 1
    seq: u64,
    lines: VecDeque<(u64, Line)>,
}

/// Talk's state: only the Capitol backlog
#[derive(Debug, Default)]
pub struct Talk {
    capitol: Mutex<Backlog>,
}

impl Talk {
    pub fn new() -> Talk {
        Talk::default()
    }

    /// `speaker` says `text` on `channel`: cleaned
    /// ([`talk::clean`]), refused from a watcher if it's local, counted in
    /// the account's chat bucket, then sent to `events` as
    /// [`GameEvent::Said`]. A Capitol line joins the backlog. `Err` is the
    /// refusal, in words for the `error`.
    pub fn say(
        &self,
        events: &broadcast::Sender<GameEvent>,
        limiter: &RateLimiter,
        speaker: Speaker,
        channel: Channel,
        text: &str,
    ) -> Result<(), String> {
        let text = talk::clean(text).map_err(|e| e.message())?;
        if channel == Channel::Local && !speaker.holds_avatar {
            return Err(WATCHER_LOCAL.into());
        }
        if channel == Channel::Guild && speaker.guild.is_none() {
            return Err(NO_GUILD.into());
        }
        limiter.check(Key::chat(speaker.account_id)).map_err(rate_limit::chat_message)?;
        let line = Line {
            from: speaker.player_id,
            name: speaker.name,
            citizen: speaker.citizen,
            channel,
            text,
            x: speaker.x,
            y: speaker.y,
            guild: speaker.guild,
        };
        match channel {
            // Not kept: a guild's talk is for those on to hear it
            Channel::Local | Channel::Guild => {
                let _ = events.send(GameEvent::Said { seq: 0, line });
            }
            // Numbered and sent under the one lock, so the lines go out in
            // the order of their numbers
            Channel::Capitol => {
                let mut backlog = self.capitol.lock().expect("talk's backlog lock");
                backlog.seq += 1;
                let seq = backlog.seq;
                if backlog.lines.len() == CAPITOL_BACKLOG {
                    backlog.lines.pop_front();
                }
                backlog.lines.push_back((seq, line.clone()));
                let _ = events.send(GameEvent::Said { seq, line });
            }
        }
        Ok(())
    }

    /// The last Capitol lines, oldest first, but those from anyone in
    /// `muted`, and the newest one's number: a socket sends these, then
    /// skips any Capitol line numbered at or below it
    pub fn backlog(&self, muted: &HashSet<Uuid>) -> (u64, Vec<ServerMessage>) {
        let backlog = self.capitol.lock().expect("talk's backlog lock");
        let heard = backlog.lines.iter().filter(|(_, line)| !muted.contains(&line.from));
        (backlog.seq, heard.map(|(_, line)| line.message()).collect())
    }
}

/// Whether the socket of `listener`, standing in `cell` on the positions
/// `seen`, in `guild`, hears `line`. Nothing from anyone in `muted` does.
/// Capitol lines reach everyone else, and a guild line every socket of its
/// guild's members, watchers too. A local line reaches only a socket that
/// holds the avatar: the speaker's own, or one within [`HEARD`] of where it
/// was said.
pub fn hears(
    line: &Line,
    listener: Uuid,
    holds_avatar: bool,
    seen: Option<&Positions>,
    cell: Option<Cell>,
    muted: &HashSet<Uuid>,
    guild: Option<Uuid>,
) -> bool {
    if muted.contains(&line.from) {
        return false;
    }
    match line.channel {
        Channel::Capitol => true,
        Channel::Guild => line.guild.is_some() && line.guild == guild,
        Channel::Local => {
            holds_avatar
                && (line.from == listener
                    || cell.is_some_and(|mine| near(mine, cell_at(line.x, line.y)))
                        && seen
                            .and_then(|p| p.position_of(listener))
                            .is_some_and(|(x, y)| (x - line.x).powi(2) + (y - line.y).powi(2) <= HEARD * HEARD))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActionState, PlayerState};
    use crate::rate_limit::RateLimitConfig;
    use std::collections::HashMap;

    const AT: (f64, f64) = (6800.0, 6800.0);

    fn player(name: &str, x: f64, y: f64) -> PlayerState {
        PlayerState {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            name: name.into(),
            x,
            y,
            dest_x: None,
            dest_y: None,
            speed: 50.0,
            strand_balance: 0,
            current_zone: None,
            action_state: ActionState::Idle,
            citizen: false,
            steer: None,
        }
    }

    fn speaker(p: &PlayerState) -> Speaker {
        Speaker {
            player_id: p.id,
            account_id: p.account_id,
            name: p.name.clone(),
            citizen: p.citizen,
            x: p.x,
            y: p.y,
            holds_avatar: true,
            guild: None,
        }
    }

    fn unlimited() -> RateLimiter {
        RateLimiter::new(RateLimitConfig { enabled: false, ..Default::default() })
    }

    fn said(rx: &mut broadcast::Receiver<GameEvent>) -> Vec<(u64, Line)> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|e| match e {
                GameEvent::Said { seq, line } => Some((seq, line)),
                _ => None,
            })
            .collect()
    }

    /// Whether `listener` hears `line` on `positions`, holding the avatar
    fn heard_by(line: &Line, listener: &PlayerState, positions: &Positions) -> bool {
        hears(line, listener.id, true, Some(positions), positions.cell_of(listener.id), &HashSet::new(), None)
    }

    #[test]
    fn a_local_line_reaches_29_units_and_not_31() {
        let ada = player("Ada", AT.0, AT.1);
        let near_by = player("Bram", AT.0 + 29.0, AT.1);
        let too_far = player("Cass", AT.0, AT.1 - 31.0);
        let positions = Positions::of([&ada, &near_by, &too_far], &HashMap::new(), &HashMap::new());
        let (events, mut rx) = broadcast::channel(16);
        Talk::new().say(&events, &unlimited(), speaker(&ada), Channel::Local, "Rope, anyone?").unwrap();
        let [(seq, line)] = said(&mut rx).try_into().unwrap();
        assert_eq!(seq, 0, "a local line isn't numbered");
        assert!(heard_by(&line, &ada, &positions), "the speaker hears herself");
        assert!(heard_by(&line, &near_by, &positions));
        assert!(!heard_by(&line, &too_far, &positions));
        assert!(
            !hears(&line, near_by.id, false, Some(&positions), positions.cell_of(near_by.id), &HashSet::new(), None),
            "a watcher hears no local line"
        );
    }

    /// Two players either side of a cell's edge are in sight of each other,
    /// so the distance decides; two cells off, the cell does
    #[test]
    fn the_cell_is_checked_and_then_the_distance() {
        let edge = capitol_protocol::rules::INTEREST_CELL * 23.0;
        let ada = player("Ada", edge - 10.0, edge - 10.0);
        let over = player("Bram", edge + 10.0, edge + 10.0);
        let far = player("Cass", edge + 700.0, edge);
        let positions = Positions::of([&ada, &over, &far], &HashMap::new(), &HashMap::new());
        assert_ne!(positions.cell_of(ada.id), positions.cell_of(over.id));
        let (events, mut rx) = broadcast::channel(16);
        Talk::new().say(&events, &unlimited(), speaker(&ada), Channel::Local, "Over here").unwrap();
        let [(_, line)] = said(&mut rx).try_into().unwrap();
        assert!(heard_by(&line, &over, &positions), "28 units off, across the edge");
        assert!(!heard_by(&line, &far, &positions));
        assert!(!hears(&line, Uuid::new_v4(), true, Some(&positions), None, &HashSet::new(), None), "nor one out of the world");
    }

    #[test]
    fn a_capitol_line_reaches_everyone() {
        let ada = player("Ada", AT.0, AT.1);
        let far = player("Bram", AT.0 + 4000.0, AT.1);
        let positions = Positions::of([&ada, &far], &HashMap::new(), &HashMap::new());
        let (events, mut rx) = broadcast::channel(16);
        Talk::new().say(&events, &unlimited(), speaker(&ada), Channel::Capitol, "Planks at the hall").unwrap();
        let [(seq, line)] = said(&mut rx).try_into().unwrap();
        assert_eq!(seq, 1);
        assert!(heard_by(&line, &far, &positions));
        assert!(hears(&line, far.id, false, None, None, &HashSet::new(), None), "a watcher with nobody in sight hears it too");
    }

    #[test]
    fn the_backlog_holds_the_last_40_lines_in_order() {
        let ada = player("Ada", AT.0, AT.1);
        let talk = Talk::new();
        let (events, _rx) = broadcast::channel(64);
        for n in 1..=45 {
            talk.say(&events, &unlimited(), speaker(&ada), Channel::Capitol, &format!("line {n}")).unwrap();
        }
        talk.say(&events, &unlimited(), speaker(&ada), Channel::Local, "not kept").unwrap();
        let (seq, lines) = talk.backlog(&HashSet::new());
        assert_eq!(seq, 45);
        let texts: Vec<String> = lines
            .iter()
            .map(|m| match m {
                ServerMessage::Said { text, channel: Channel::Capitol, .. } => text.clone(),
                other => panic!("{other:?} in the backlog"),
            })
            .collect();
        let want: Vec<String> = (6..=45).map(|n| format!("line {n}")).collect();
        assert_eq!(texts, want);
    }

    /// S1.4: neither channel reaches the muter, nor the backlog, and both
    /// still reach everyone else
    #[test]
    fn a_muted_speakers_lines_reach_everyone_but_the_muter() {
        let ada = player("Ada", AT.0, AT.1);
        let bram = player("Bram", AT.0 + 10.0, AT.1);
        let cass = player("Cass", AT.0, AT.1 + 10.0);
        let positions = Positions::of([&ada, &bram, &cass], &HashMap::new(), &HashMap::new());
        let bram_muted = HashSet::from([ada.id]);
        let talk = Talk::new();
        let (events, mut rx) = broadcast::channel(16);
        talk.say(&events, &unlimited(), speaker(&ada), Channel::Local, "Rope, anyone?").unwrap();
        talk.say(&events, &unlimited(), speaker(&ada), Channel::Capitol, "Rope at the hall").unwrap();
        talk.say(&events, &unlimited(), speaker(&cass), Channel::Capitol, "Planks too").unwrap();
        let lines = said(&mut rx);
        let reaches = |line: &Line, who: &PlayerState, muted: &HashSet<Uuid>| {
            hears(line, who.id, true, Some(&positions), positions.cell_of(who.id), muted, None)
        };
        let to_bram: Vec<&str> = lines.iter().filter(|(_, l)| reaches(l, &bram, &bram_muted)).map(|(_, l)| l.text.as_str()).collect();
        assert_eq!(to_bram, ["Planks too"]);
        assert!(lines.iter().all(|(_, l)| reaches(l, &cass, &HashSet::new())), "Cass hears them all");
        let (_, backlog) = talk.backlog(&bram_muted);
        assert!(matches!(&backlog[..], [ServerMessage::Said { text, .. }] if text == "Planks too"), "{backlog:?}");
        assert_eq!(talk.backlog(&HashSet::new()).1.len(), 2);
    }

    #[test]
    fn a_watcher_cant_talk_locally() {
        let ada = player("Ada", AT.0, AT.1);
        let watcher = Speaker { holds_avatar: false, ..speaker(&ada) };
        let talk = Talk::new();
        let (events, mut rx) = broadcast::channel(16);
        assert_eq!(talk.say(&events, &unlimited(), watcher.clone(), Channel::Local, "Hello"), Err(WATCHER_LOCAL.into()));
        assert!(said(&mut rx).is_empty());
        talk.say(&events, &unlimited(), watcher, Channel::Capitol, "Hello, Capitol").unwrap();
        assert_eq!(said(&mut rx).len(), 1, "but may on the Capitol channel");
    }

    #[test]
    fn the_9th_line_in_10_s_is_refused() {
        let ada = player("Ada", AT.0, AT.1);
        let limiter = RateLimiter::new(RateLimitConfig::default());
        let talk = Talk::new();
        let (events, mut rx) = broadcast::channel(16);
        for n in 0..8 {
            let channel = if n % 2 == 0 { Channel::Local } else { Channel::Capitol };
            talk.say(&events, &limiter, speaker(&ada), channel, "again").unwrap();
        }
        let refused = talk.say(&events, &limiter, speaker(&ada), Channel::Capitol, "and again").unwrap_err();
        assert!(refused.starts_with("You're talking too fast"), "{refused}");
        assert_eq!(said(&mut rx).len(), 8);
        assert_eq!(talk.backlog(&HashSet::new()).1.len(), 4, "the refused line isn't kept");
        let bram = player("Bram", AT.0, AT.1);
        talk.say(&events, &limiter, speaker(&bram), Channel::Local, "me too").unwrap();
    }

    #[test]
    fn a_line_is_cleaned_and_an_empty_one_refused_without_counting() {
        let ada = player("Ada", AT.0, AT.1);
        let limiter = RateLimiter::new(RateLimitConfig::default());
        let talk = Talk::new();
        let (events, mut rx) = broadcast::channel(16);
        for _ in 0..20 {
            assert!(talk.say(&events, &limiter, speaker(&ada), Channel::Local, " \n ").is_err());
        }
        talk.say(&events, &limiter, speaker(&ada), Channel::Local, "  one\ntwo  ").unwrap();
        let [(_, line)] = said(&mut rx).try_into().unwrap();
        assert_eq!(line.text, "one two");
    }

    #[test]
    fn a_citizens_line_carries_citizen_true() {
        let brin = PlayerState { citizen: true, ..player("Brin", AT.0, AT.1) };
        let (events, mut rx) = broadcast::channel(16);
        Talk::new().say(&events, &unlimited(), speaker(&brin), Channel::Capitol, "Planks, fair price").unwrap();
        let [(_, line)] = said(&mut rx).try_into().unwrap();
        assert!(line.citizen);
        let json = serde_json::to_string(&line.message()).unwrap();
        assert!(json.contains(r#""citizen":true"#), "{json}");
        let ada = player("Ada", AT.0, AT.1);
        Talk::new().say(&events, &unlimited(), speaker(&ada), Channel::Capitol, "Thanks").unwrap();
        let [(_, line)] = said(&mut rx).try_into().unwrap();
        assert!(!serde_json::to_string(&line.message()).unwrap().contains("citizen"), "a person's line leaves it out");
    }

    #[test]
    fn a_guild_line_reaches_its_members_only_wherever_they_are() {
        let weavers_id = Uuid::new_v4();
        let weavers = Some(weavers_id);
        let ada = player("Ada", AT.0, AT.1);
        let far_member = player("Bram", AT.0 + 4000.0, AT.1);
        let near_outsider = player("Cass", AT.0 + 5.0, AT.1);
        let other_guild = player("Dot", AT.0 + 6.0, AT.1);
        let positions = Positions::of([&ada, &far_member, &near_outsider, &other_guild], &HashMap::new(), &HashMap::new());
        let (events, mut rx) = broadcast::channel(16);
        let talk = Talk::new();
        talk.say(&events, &unlimited(), Speaker { guild: weavers, ..speaker(&ada) }, Channel::Guild, "Hall at noon").unwrap();
        let [(seq, line)] = said(&mut rx).try_into().unwrap();
        assert_eq!(seq, 0, "not numbered: it's not the Capitol's");
        let hears_in = |who: &PlayerState, avatar: bool, guild: Option<Uuid>| {
            hears(&line, who.id, avatar, Some(&positions), positions.cell_of(who.id), &HashSet::new(), guild)
        };
        assert!(hears_in(&ada, true, weavers), "the speaker hears their own");
        assert!(hears_in(&far_member, true, weavers), "a member across the world");
        assert!(hears_in(&far_member, false, weavers), "a member watching");
        assert!(!hears_in(&near_outsider, true, None), "not someone beside them in no guild");
        assert!(!hears_in(&other_guild, true, Some(Uuid::new_v4())), "nor another guild's member");
        let mut muted = HashSet::new();
        muted.insert(ada.id);
        assert!(!hears(&line, far_member.id, true, Some(&positions), None, &muted, weavers), "nor one who muted them");
        assert!(talk.backlog(&HashSet::new()).1.is_empty(), "the Capitol's backlog doesn't keep it");
        let json = serde_json::to_string(&line.message()).unwrap();
        assert!(json.contains(r#""channel":"guild""#) && !json.contains(&weavers_id.to_string()), "{json}");
    }

    #[test]
    fn someone_in_no_guild_cant_say_a_guild_line() {
        let ada = player("Ada", AT.0, AT.1);
        let (events, mut rx) = broadcast::channel(16);
        let refused = Talk::new().say(&events, &unlimited(), speaker(&ada), Channel::Guild, "Anyone?");
        assert_eq!(refused, Err(NO_GUILD.into()));
        assert!(said(&mut rx).is_empty());
        let watcher = Speaker { holds_avatar: false, guild: Some(Uuid::new_v4()), ..speaker(&ada) };
        Talk::new().say(&events, &unlimited(), watcher, Channel::Guild, "From afar").unwrap();
    }
}
