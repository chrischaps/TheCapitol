//! Guilds (Commons S4.4), as the client keeps them: your guild and your
//! place in it, the invitations and applications that name you, the
//! registry, the guild you're looking at, and the calls for them.
//!
//! [`Guilds`] is pure. The session makes the call [`Guilds::wants`] names,
//! one at a time, and hands back the answer. Your guild is read when you
//! sign in, and again whenever `guild_changed` or a `guild_notice` says it
//! moved; every action answers with it whole (`MyGuild`), which replaces
//! it, so the client never patches its own copy. The registry is read as
//! the panel opens on it. What you do waits in a queue, oldest first.
//!
//! Inviting is by name, since that's what you know of someone: the name is
//! looked up as a card first (`/players/by-name`), and the invitation goes
//! to whoever it finds. Names aren't unique, so that's the first to take it
//! ([#428](https://github.com/chrischaps/TheCapitol/issues/428)); a card's
//! *Invite* goes by id.
//!
//! Who may do what is `rules::guild::GuildRole`'s, the server's own rights:
//! [`roster_acts`] and [`waiting_acts`] offer only what it would allow. The
//! words are here too, so they're tested without egui.

use std::collections::VecDeque;

use capitol_protocol::rest::{
    FoundGuild, GuildDetail, GuildInvite, GuildInviteKind, GuildMember, GuildSummary, MyGuild, PlayerCard,
};
use capitol_protocol::rules::guild::{self, GuildRole};
use capitol_protocol::ws::GuildNoticeKind;
use uuid::Uuid;

/// A call [`Guilds`] wants made.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// `GET /player/guild`
    Mine,
    /// `GET /guilds`
    Registry,
    /// `GET /guilds/{id}`: one from the registry, opened
    Guild(Uuid),
    /// `POST /guilds`, at the Exchange hall
    Found(FoundGuild),
    /// `POST /guilds/{id}/apply`: ask to join, or accept their invitation
    Apply(Uuid),
    /// `GET /players/by-name/{name}`, to invite whoever it finds to `guild_id`
    Find { guild_id: Uuid, name: String },
    /// `POST /guilds/{id}/invites/{player}`: invite, or accept their application
    Invite { guild_id: Uuid, player_id: Uuid },
    /// `DELETE /guilds/{id}/invites/{player}`: decline or withdraw either
    Decline { guild_id: Uuid, player_id: Uuid },
    /// `POST /guilds/{id}/leave`
    Leave(Uuid),
    /// `DELETE /guilds/{id}/members/{player}`
    Expel { guild_id: Uuid, player_id: Uuid },
    /// `PUT /guilds/{id}/members/{player}`
    SetRole { guild_id: Uuid, player_id: Uuid, role: GuildRole },
}

impl Ask {
    /// What it did, once it's done, in a line for the panel.
    fn done_words(&self, mine: &MyGuild, name_of: impl Fn(Uuid) -> Option<String>) -> Option<String> {
        let guild = mine.guild.as_ref().map(|g| g.name.clone());
        let them = |id: &Uuid| name_of(*id).unwrap_or_else(|| "them".into());
        Some(match self {
            Ask::Mine | Ask::Registry | Ask::Guild(_) | Ask::Find { .. } => return None,
            Ask::Found(f) => format!("{} is founded, and you lead it.", f.name),
            Ask::Apply(_) => match guild {
                Some(g) => format!("You're in {g}."),
                None => "Asked. The officers will see it.".into(),
            },
            Ask::Invite { player_id, guild_id } => {
                let joined = mine.guild.as_ref().is_some_and(|g| g.id == *guild_id && g.members.iter().any(|m| m.id == *player_id));
                if joined {
                    format!("{} is in.", them(player_id))
                } else {
                    format!("Invited {}.", them(player_id))
                }
            }
            Ask::Decline { .. } => "Withdrawn.".into(),
            Ask::Leave(_) => "You've left the guild.".into(),
            Ask::Expel { player_id, .. } => format!("{} is out.", them(player_id)),
            Ask::SetRole { player_id, role, .. } => match role {
                GuildRole::Leader => format!("The lead is {}'s now.", them(player_id)),
                GuildRole::Officer => format!("{} is an officer.", them(player_id)),
                GuildRole::Member => format!("{} is a member.", them(player_id)),
            },
        })
    }
}

/// What can be done to someone on the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterAct {
    MakeOfficer,
    MakeMember,
    HandLead,
    Expel,
}

impl RosterAct {
    pub fn words(self) -> &'static str {
        match self {
            RosterAct::MakeOfficer => "Make officer",
            RosterAct::MakeMember => "Make member",
            RosterAct::HandLead => "Hand on the lead",
            RosterAct::Expel => "Expel",
        }
    }

    /// The call it makes, on `member` of `guild_id`.
    pub fn ask(self, guild_id: Uuid, member: Uuid) -> Ask {
        let role = |role| Ask::SetRole { guild_id, player_id: member, role };
        match self {
            RosterAct::MakeOfficer => role(GuildRole::Officer),
            RosterAct::MakeMember => role(GuildRole::Member),
            RosterAct::HandLead => role(GuildRole::Leader),
            RosterAct::Expel => Ask::Expel { guild_id, player_id: member },
        }
    }

    /// What it costs: a roster change is a Strand, paid by whoever makes it.
    pub fn hover(self) -> String {
        match self {
            RosterAct::HandLead => format!(
                "They lead, and you become an officer. They need a subscription. {} Strand.",
                guild::ROSTER_FEE
            ),
            _ => format!("{} Strand, from you", guild::ROSTER_FEE),
        }
    }
}

/// What `me`, as `mine`, may do to `member` on the roster: the server's
/// rights, so nothing offered is refused for want of them. The leader sets
/// roles and expels anyone; an officer expels members; a member, nothing.
pub fn roster_acts(mine: Option<GuildRole>, me: Uuid, member: &GuildMember) -> Vec<RosterAct> {
    let Some(mine) = mine else { return Vec::new() };
    if member.id == me {
        return Vec::new();
    }
    let mut acts = Vec::new();
    if mine.may_set_roles() {
        match member.role {
            GuildRole::Member => acts.push(RosterAct::MakeOfficer),
            GuildRole::Officer => acts.push(RosterAct::MakeMember),
            GuildRole::Leader => {}
        }
        if member.role != GuildRole::Leader {
            acts.push(RosterAct::HandLead);
        }
    }
    if mine.may_expel(member.role) {
        acts.push(RosterAct::Expel);
    }
    acts
}

/// What can be done about an invitation or an application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitAct {
    /// Take them in, or join
    Accept,
    /// Turn it down, or take it back
    Decline,
}

/// What may be done about `invite`, by `me` as `mine` in their guild (or
/// in none). Your own application, or an invitation an officer made, can
/// only be taken back; the other side accepts or declines.
pub fn waiting_acts(mine: Option<GuildRole>, me: Uuid, invite: &GuildInvite) -> Vec<WaitAct> {
    let yours = invite.player.id == me;
    match (invite.kind, yours) {
        // Invited: you join or decline
        (GuildInviteKind::Invite, true) => vec![WaitAct::Accept, WaitAct::Decline],
        // Your own application: you take it back
        (GuildInviteKind::Apply, true) => vec![WaitAct::Decline],
        // Someone applied: an officer or the leader takes them in or not
        (GuildInviteKind::Apply, false) if mine.is_some_and(GuildRole::may_invite) => {
            vec![WaitAct::Accept, WaitAct::Decline]
        }
        // An invitation out: an officer or the leader withdraws it
        (GuildInviteKind::Invite, false) if mine.is_some_and(GuildRole::may_invite) => vec![WaitAct::Decline],
        _ => Vec::new(),
    }
}

impl WaitAct {
    /// The call it makes on `invite`, for `me`.
    pub fn ask(self, me: Uuid, invite: &GuildInvite) -> Ask {
        let (guild_id, player_id) = (invite.guild_id, invite.player.id);
        match (self, player_id == me) {
            (WaitAct::Accept, true) => Ask::Apply(guild_id),
            (WaitAct::Accept, false) => Ask::Invite { guild_id, player_id },
            (WaitAct::Decline, _) => Ask::Decline { guild_id, player_id },
        }
    }

    /// Its button, on `invite` as `me` sees it.
    pub fn words(self, me: Uuid, invite: &GuildInvite) -> &'static str {
        let yours = invite.player.id == me;
        match (self, invite.kind, yours) {
            (WaitAct::Accept, GuildInviteKind::Invite, _) => "Join",
            (WaitAct::Accept, GuildInviteKind::Apply, _) => "Take in",
            (WaitAct::Decline, GuildInviteKind::Invite, true) => "Decline",
            (WaitAct::Decline, GuildInviteKind::Apply, false) => "Turn down",
            (WaitAct::Decline, _, _) => "Withdraw",
        }
    }
}

/// The founding form, as typed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    pub name: String,
    pub tag: String,
    pub charter: String,
}

impl Draft {
    /// The guild it founds, cleaned as the server cleans it, or what's
    /// wrong with it, in words.
    pub fn check(&self) -> Result<FoundGuild, String> {
        let name = guild::clean_name(&self.name).map_err(|e| e.message())?;
        let tag = guild::clean_tag(&self.tag).map_err(|e| e.message())?;
        let charter = guild::clean_charter(&self.charter).map_err(|e| e.message())?;
        Ok(FoundGuild { name, tag, charter })
    }

    /// The fee, and whether `balance` covers it.
    pub fn affordable(balance: i64) -> bool {
        balance >= guild::FOUNDING_FEE
    }
}

/// Your guilds, and the calls for them.
#[derive(Debug, Clone, Default)]
pub struct Guilds {
    mine: Option<MyGuild>,
    registry: Option<Vec<GuildSummary>>,
    /// The guild opened from the registry
    viewing: Option<GuildDetail>,
    stale: bool,
    registry_stale: bool,
    /// What you do, not yet asked, oldest first
    pending: VecDeque<Ask>,
    /// The call that's out
    asked: Option<Ask>,
    refused: Option<String>,
    done: Option<String>,
    online: bool,
}

impl Guilds {
    /// Signed in, or back after a drop: whatever was out won't be answered,
    /// and your guild may have changed meanwhile.
    pub fn connected(&mut self) {
        self.online = true;
        self.asked = None;
        self.stale = true;
        self.registry_stale = self.registry.is_some();
    }

    /// Signed out: nothing is kept.
    pub fn signed_out(&mut self) {
        *self = Guilds::default();
    }

    /// The call to make next, if any. Mark it sent with [`Guilds::sent`].
    pub fn wants(&self) -> Option<Ask> {
        if !self.online || self.asked.is_some() {
            return None;
        }
        if let Some(ask) = self.pending.front() {
            return Some(ask.clone());
        }
        if self.stale || self.mine.is_none() {
            return Some(Ask::Mine);
        }
        self.registry_stale.then_some(Ask::Registry)
    }

    pub fn sent(&mut self, ask: Ask) {
        if self.pending.front() == Some(&ask) {
            self.pending.pop_front();
        }
        match ask {
            Ask::Mine => self.stale = false,
            Ask::Registry => self.registry_stale = false,
            _ => {}
        }
        self.asked = Some(ask);
    }

    /// Do `ask` next, after what's already waiting.
    pub fn queue(&mut self, ask: Ask) {
        self.refused = None;
        self.done = None;
        if !self.pending.contains(&ask) && self.asked.as_ref() != Some(&ask) {
            self.pending.push_back(ask);
        }
    }

    /// `guild_changed`, or a `guild_notice`: your guild is read again, and
    /// so is the registry if it's been shown, since its counts moved.
    pub fn changed(&mut self) {
        self.stale = true;
        self.registry_stale |= self.registry.is_some();
    }

    /// The registry's wanted (the panel opened on it).
    pub fn browse(&mut self) {
        self.registry_stale = true;
    }

    /// Open `guild_id` from the registry.
    pub fn open(&mut self, guild_id: Uuid) {
        if self.viewing.as_ref().is_some_and(|g| g.id == guild_id) {
            return;
        }
        self.viewing = None;
        self.queue(Ask::Guild(guild_id));
    }

    pub fn close_viewing(&mut self) {
        self.viewing = None;
    }

    /// Your guild as it stands: from a read, or any action's answer.
    pub fn answered(&mut self, mine: MyGuild) {
        let joined_or_left = self.guild_id() != mine.guild.as_ref().map(|g| g.id);
        if let Some(ask) = self.asked.take() {
            let before = self.mine.clone().unwrap_or_default();
            let name_of = |id: Uuid| name_in(&before, id).or_else(|| name_in(&mine, id));
            if let Some(words) = ask.done_words(&mine, name_of) {
                self.refused = None;
                self.done = Some(words);
            }
            // An action moved the counts on the registry
            if !matches!(ask, Ask::Mine) {
                self.registry_stale |= self.registry.is_some();
            }
        }
        if joined_or_left {
            self.viewing = None;
        }
        self.mine = Some(mine);
    }

    /// `GET /guilds` came back.
    pub fn listed(&mut self, guilds: Vec<GuildSummary>) {
        if self.asked == Some(Ask::Registry) {
            self.asked = None;
        }
        self.registry = Some(guilds);
    }

    /// `GET /guilds/{id}` came back.
    pub fn shown(&mut self, guild: GuildDetail) {
        if matches!(self.asked, Some(Ask::Guild(id)) if id == guild.id) {
            self.asked = None;
        }
        self.viewing = Some(guild);
    }

    /// The card for a name to invite came back: the invitation goes next.
    pub fn found(&mut self, card: &PlayerCard) {
        if let Some(Ask::Find { guild_id, .. }) = self.asked {
            self.asked = None;
            if card.guild.is_some() && !card.mine {
                self.refused = Some(format!("{} is in a guild already.", card.name));
            } else if card.mine {
                self.refused = Some("That's you.".into());
            } else {
                self.pending.push_front(Ask::Invite { guild_id, player_id: card.id });
            }
        }
    }

    /// The call that was out failed, saying `why`. What you did says so;
    /// a read is asked again later.
    pub fn failed(&mut self, why: String, not_found: bool) {
        match self.asked.take() {
            Some(Ask::Mine) => {
                // Shown as in no guild rather than loading for ever
                self.mine.get_or_insert_with(MyGuild::default);
            }
            Some(Ask::Registry) => {
                self.registry.get_or_insert_with(Vec::new);
            }
            Some(Ask::Find { name, .. }) if not_found => {
                self.done = None;
                self.refused = Some(format!("Nobody in the city is called {name}."));
            }
            Some(Ask::Guild(_)) if not_found => {
                self.refused = Some("That guild has disbanded.".into());
                self.registry_stale = true;
            }
            Some(_) => {
                self.done = None;
                self.refused = Some(why);
            }
            None => {}
        }
    }

    /// Your guild, your role and what's waiting, once read.
    pub fn mine(&self) -> Option<&MyGuild> {
        self.mine.as_ref()
    }

    pub fn guild(&self) -> Option<&GuildDetail> {
        self.mine.as_ref()?.guild.as_ref()
    }

    pub fn guild_id(&self) -> Option<Uuid> {
        self.guild().map(|g| g.id)
    }

    pub fn role(&self) -> Option<GuildRole> {
        self.mine.as_ref()?.role
    }

    pub fn registry(&self) -> Option<&[GuildSummary]> {
        self.registry.as_deref()
    }

    pub fn viewing(&self) -> Option<&GuildDetail> {
        self.viewing.as_ref()
    }

    /// Your application to or invitation from `guild_id`, if one waits.
    pub fn waiting_with(&self, guild_id: Uuid) -> Option<&GuildInvite> {
        self.mine.as_ref()?.invites.iter().find(|i| i.guild_id == guild_id)
    }

    /// A call of yours is out or waiting: the buttons hold still.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || self.asked.as_ref().is_some_and(|a| !matches!(a, Ask::Mine | Ask::Registry))
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn done(&self) -> Option<&str> {
        self.done.as_deref()
    }

    /// Invitations to you, for the header's count.
    pub fn invitations(&self) -> usize {
        self.mine.as_ref().map_or(0, |m| m.invites.iter().filter(|i| i.kind == GuildInviteKind::Invite).count())
    }
}

/// `id`'s name, if `mine` mentions them.
fn name_in(mine: &MyGuild, id: Uuid) -> Option<String> {
    let guild = mine.guild.as_ref()?;
    guild
        .members
        .iter()
        .find(|m| m.id == id)
        .map(|m| m.name.clone())
        .or_else(|| guild.waiting.iter().find(|w| w.player.id == id).map(|w| w.player.name.clone()))
}

/// A role, in a word.
pub fn role_words(role: GuildRole) -> &'static str {
    match role {
        GuildRole::Leader => "Leader",
        GuildRole::Officer => "Officer",
        GuildRole::Member => "Member",
    }
}

/// "1 member", "12 members".
pub fn members_words(n: usize) -> String {
    if n == 1 {
        "1 member".into()
    } else {
        format!("{n} members")
    }
}

/// A tag as it hangs after a name: "[ROPE]".
pub fn bracketed(tag: &str) -> String {
    format!("[{tag}]")
}

/// A `guild_notice`'s toast: its title and its line.
pub fn notice_words(kind: GuildNoticeKind, guild: &str, name: &str) -> (String, String) {
    match kind {
        GuildNoticeKind::Invited => (format!("Invited to {guild}"), format!("{name} asks you to join. Open the guild panel (U) to answer.")),
        GuildNoticeKind::Applied => (format!("{name} applied"), format!("They'd like to join {guild}. Open the guild panel (U) to answer.")),
        GuildNoticeKind::Expelled => (format!("Out of {guild}"), format!("{name} expelled you.")),
        GuildNoticeKind::Lead => (format!("You lead {guild}"), format!("The lead came to you from {name}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::Correspondent;
    use chrono::{DateTime, Utc};

    const ME: Uuid = Uuid::from_u128(1);
    const BRAM: Uuid = Uuid::from_u128(2);
    const CASS: Uuid = Uuid::from_u128(3);
    const ROPE: Uuid = Uuid::from_u128(10);

    fn when() -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap()
    }

    fn who(id: Uuid, name: &str) -> Correspondent {
        Correspondent { id, name: name.into(), citizen: false, office: false }
    }

    fn member(id: Uuid, name: &str, role: GuildRole) -> GuildMember {
        GuildMember { id, name: name.into(), citizen: false, role, joined_at: when(), standing: "New in town".into() }
    }

    fn invite(kind: GuildInviteKind, player: Uuid, by: Uuid) -> GuildInvite {
        GuildInvite {
            guild_id: ROPE,
            guild: "The Rope Walkers".into(),
            tag: "ROPE".into(),
            kind,
            player: who(player, "P"),
            by: who(by, "B"),
            created_at: when(),
        }
    }

    fn rope(members: Vec<GuildMember>) -> GuildDetail {
        GuildDetail {
            hall: None,
            id: ROPE,
            name: "The Rope Walkers".into(),
            tag: "ROPE".into(),
            charter: String::new(),
            founded_at: when(),
            founder: who(ME, "Wren"),
            leader: who(ME, "Wren"),
            members,
            waiting: Vec::new(),
            treasury: None,
        }
    }

    fn mine(role: GuildRole) -> MyGuild {
        MyGuild {
            guild: Some(rope(vec![member(ME, "Wren", role), member(BRAM, "Bram", GuildRole::Member)])),
            role: Some(role),
            invites: Vec::new(),
        }
    }

    #[test]
    fn the_leader_sets_roles_and_expels_officers_and_members() {
        let leader = Some(GuildRole::Leader);
        let officer = member(BRAM, "Bram", GuildRole::Officer);
        let plain = member(CASS, "Cass", GuildRole::Member);
        assert_eq!(roster_acts(leader, ME, &officer), [RosterAct::MakeMember, RosterAct::HandLead, RosterAct::Expel]);
        assert_eq!(roster_acts(leader, ME, &plain), [RosterAct::MakeOfficer, RosterAct::HandLead, RosterAct::Expel]);
        assert!(roster_acts(leader, ME, &member(ME, "Wren", GuildRole::Leader)).is_empty(), "nothing to yourself");
    }

    #[test]
    fn an_officer_expels_members_only_and_a_member_does_nothing() {
        let officer = Some(GuildRole::Officer);
        assert_eq!(roster_acts(officer, ME, &member(CASS, "Cass", GuildRole::Member)), [RosterAct::Expel]);
        assert!(roster_acts(officer, ME, &member(BRAM, "Bram", GuildRole::Officer)).is_empty());
        assert!(roster_acts(officer, ME, &member(CASS, "Cy", GuildRole::Leader)).is_empty());
        let plain = Some(GuildRole::Member);
        assert!(roster_acts(plain, ME, &member(CASS, "Cass", GuildRole::Member)).is_empty());
        assert!(roster_acts(None, ME, &member(CASS, "Cass", GuildRole::Member)).is_empty());
    }

    #[test]
    fn each_side_of_an_invitation_answers_its_own_way() {
        let to_me = invite(GuildInviteKind::Invite, ME, BRAM);
        assert_eq!(waiting_acts(None, ME, &to_me), [WaitAct::Accept, WaitAct::Decline]);
        assert_eq!(WaitAct::Accept.ask(ME, &to_me), Ask::Apply(ROPE), "joining is applying to the guild that asked");
        assert_eq!(WaitAct::Accept.words(ME, &to_me), "Join");
        let my_application = invite(GuildInviteKind::Apply, ME, ME);
        assert_eq!(waiting_acts(None, ME, &my_application), [WaitAct::Decline]);
        assert_eq!(WaitAct::Decline.words(ME, &my_application), "Withdraw");

        let theirs = invite(GuildInviteKind::Apply, CASS, CASS);
        assert_eq!(waiting_acts(Some(GuildRole::Officer), ME, &theirs), [WaitAct::Accept, WaitAct::Decline]);
        assert_eq!(WaitAct::Accept.ask(ME, &theirs), Ask::Invite { guild_id: ROPE, player_id: CASS });
        assert_eq!(WaitAct::Decline.words(ME, &theirs), "Turn down");
        assert!(waiting_acts(Some(GuildRole::Member), ME, &theirs).is_empty(), "a member can't take anyone in");
        let out = invite(GuildInviteKind::Invite, CASS, ME);
        assert_eq!(waiting_acts(Some(GuildRole::Leader), ME, &out), [WaitAct::Decline]);
    }

    #[test]
    fn your_guild_is_read_on_sign_in_and_after_it_changes() {
        let mut guilds = Guilds::default();
        assert_eq!(guilds.wants(), None, "nothing before signing in");
        guilds.connected();
        assert_eq!(guilds.wants(), Some(Ask::Mine));
        guilds.sent(Ask::Mine);
        assert_eq!(guilds.wants(), None, "one call at a time");
        guilds.answered(mine(GuildRole::Leader));
        assert_eq!(guilds.wants(), None);
        assert_eq!(guilds.guild_id(), Some(ROPE));
        assert_eq!(guilds.done(), None, "a read says nothing");
        guilds.changed();
        assert_eq!(guilds.wants(), Some(Ask::Mine));
    }

    #[test]
    fn an_action_goes_before_a_read_and_says_what_it_did() {
        let mut guilds = Guilds::default();
        guilds.connected();
        guilds.sent(Ask::Mine);
        guilds.answered(mine(GuildRole::Leader));
        guilds.queue(RosterAct::MakeOfficer.ask(ROPE, BRAM));
        guilds.changed();
        let ask = guilds.wants().unwrap();
        assert_eq!(ask, Ask::SetRole { guild_id: ROPE, player_id: BRAM, role: GuildRole::Officer });
        assert!(guilds.busy());
        guilds.sent(ask);
        let mut after = mine(GuildRole::Leader);
        after.guild.as_mut().unwrap().members[1].role = GuildRole::Officer;
        guilds.answered(after);
        assert_eq!(guilds.done(), Some("Bram is an officer."));
        assert!(!guilds.busy());
    }

    #[test]
    fn a_refusal_is_said_and_a_read_that_fails_shows_no_guild() {
        let mut guilds = Guilds::default();
        guilds.connected();
        guilds.sent(Ask::Mine);
        guilds.failed("down".into(), false);
        assert_eq!(guilds.mine(), Some(&MyGuild::default()));
        guilds.queue(Ask::Leave(ROPE));
        guilds.sent(Ask::Leave(ROPE));
        guilds.failed("The leader leaves last".into(), false);
        assert_eq!(guilds.refused(), Some("The leader leaves last"));
    }

    #[test]
    fn inviting_by_name_finds_the_card_then_invites_its_id() {
        let mut guilds = Guilds::default();
        guilds.connected();
        guilds.sent(Ask::Mine);
        guilds.answered(mine(GuildRole::Officer));
        let find = Ask::Find { guild_id: ROPE, name: "Cass".into() };
        guilds.queue(find.clone());
        guilds.sent(find);
        let card = PlayerCard {
            id: CASS,
            name: "Cass".into(),
            citizen: false,
            since: 3,
            home: None,
            standing: "New in town".into(),
            guild: None,
            mine: false,
            muted: false,
            own: None,
        };
        guilds.found(&card);
        assert_eq!(guilds.wants(), Some(Ask::Invite { guild_id: ROPE, player_id: CASS }));

        // Someone in a guild can't be asked
        let find = Ask::Find { guild_id: ROPE, name: "Dot".into() };
        guilds.sent(guilds.wants().unwrap());
        guilds.answered(mine(GuildRole::Officer));
        guilds.queue(find.clone());
        guilds.sent(find);
        guilds.found(&PlayerCard { guild: Some("Oak & Iron".into()), name: "Dot".into(), ..card.clone() });
        assert_eq!(guilds.refused(), Some("Dot is in a guild already."));

        let find = Ask::Find { guild_id: ROPE, name: "Nobody".into() };
        guilds.queue(find.clone());
        guilds.sent(find);
        guilds.failed("Not found".into(), true);
        assert_eq!(guilds.refused(), Some("Nobody in the city is called Nobody."));
    }

    #[test]
    fn the_founding_form_is_cleaned_as_the_server_cleans_it() {
        let draft = Draft { name: "  The Rope   Walkers ".into(), tag: "rope".into(), charter: "We make rope.".into() };
        let found = draft.check().unwrap();
        assert_eq!((found.name.as_str(), found.tag.as_str()), ("The Rope Walkers", "ROPE"));
        assert!(Draft { tag: "x".into(), ..draft.clone() }.check().is_err());
        assert!(Draft { name: "The Capitol Guild".into(), ..draft }.check().is_err(), "reserved");
        assert!(Draft::affordable(guild::FOUNDING_FEE) && !Draft::affordable(guild::FOUNDING_FEE - 1));
    }

    #[test]
    fn joining_or_leaving_closes_the_guild_you_were_looking_at() {
        let mut guilds = Guilds::default();
        guilds.connected();
        guilds.sent(Ask::Mine);
        guilds.answered(MyGuild::default());
        guilds.open(ROPE);
        guilds.sent(Ask::Guild(ROPE));
        guilds.shown(rope(vec![member(BRAM, "Bram", GuildRole::Leader)]));
        assert!(guilds.viewing().is_some());
        guilds.queue(Ask::Apply(ROPE));
        guilds.sent(Ask::Apply(ROPE));
        guilds.answered(mine(GuildRole::Member));
        assert!(guilds.viewing().is_none());
        assert_eq!(guilds.done(), Some("You're in The Rope Walkers."));
    }

    #[test]
    fn the_words() {
        assert_eq!(members_words(1), "1 member");
        assert_eq!(members_words(12), "12 members");
        assert_eq!(bracketed("ROPE"), "[ROPE]");
        let (title, body) = notice_words(GuildNoticeKind::Invited, "The Rope Walkers", "Bram");
        assert_eq!(title, "Invited to The Rope Walkers");
        assert!(body.starts_with("Bram asks you to join"));
    }
}
