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
//!
//! The treasury (S4.2) comes with your guild, to members only. Its books
//! are read as the panel opens on your guild and after anything moves
//! them, newest first. Anyone deposits; [`withdrawable`] is what you may
//! take, by the server's rule: the leader anything the treasury holds, an
//! officer what's left of the leader's weekly limit. [`Purse`] checks an
//! amount as typed against both.
//!
//! The hall (S4.3) is the leader's to buy, from the treasury, at the
//! Exchange hall: [`hall_offer`] says what a free guild-district lot's
//! panel offers whoever's looking, and why not.

use std::collections::VecDeque;

use capitol_protocol::rest::{
    FoundGuild, GuildDetail, GuildEntry, GuildEntryKind, GuildHall, GuildInvite, GuildInviteKind, GuildMember, GuildSummary,
    GuildTreasury, MyGuild, PlayerCard, PlotInfo,
};
use capitol_protocol::rules::guild::{self, GuildRole};
use capitol_protocol::rules::Zone;
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
    /// `GET /guilds/{id}/books`: the treasury's moves
    Books(Uuid),
    /// `POST /guilds/{id}/deposit`
    Deposit { guild_id: Uuid, strands: i64 },
    /// `POST /guilds/{id}/withdraw`, with what it's for
    Withdraw { guild_id: Uuid, strands: i64, note: String },
    /// `PUT /guilds/{id}/limit`: what an officer may take in a week
    SetLimit { guild_id: Uuid, strands: i64 },
    /// `POST /guilds/{id}/hall`, at the Exchange hall
    ClaimHall { guild_id: Uuid, plot_id: Uuid },
}

impl Ask {
    /// What it did, once it's done, in a line for the panel.
    fn done_words(&self, mine: &MyGuild, name_of: impl Fn(Uuid) -> Option<String>) -> Option<String> {
        let guild = mine.guild.as_ref().map(|g| g.name.clone());
        let them = |id: &Uuid| name_of(*id).unwrap_or_else(|| "them".into());
        Some(match self {
            Ask::Mine | Ask::Registry | Ask::Guild(_) | Ask::Find { .. } | Ask::Books(_) => return None,
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
            Ask::Deposit { strands, .. } => format!("You put {} in the treasury.", strands_words(*strands)),
            Ask::Withdraw { strands, .. } => format!("You took {} from the treasury.", strands_words(*strands)),
            Ask::SetLimit { strands: 0, .. } => "Officers may take nothing from the treasury now.".into(),
            Ask::SetLimit { strands, .. } => format!("Officers may take {} a week.", strands_words(*strands)),
            Ask::ClaimHall { .. } => match guild {
                Some(g) => format!("The hall is {}.", possessive(&g)),
                None => "The hall is the guild's.".into(),
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

/// What you may take from the treasury now, as `role`: the leader all of
/// it, an officer what's left of the week's limit, a member nothing.
pub fn withdrawable(role: Option<GuildRole>, treasury: &GuildTreasury) -> i64 {
    match role {
        Some(GuildRole::Leader) => treasury.strands.max(0),
        Some(r) if r.may_withdraw() => {
            (treasury.withdrawal_limit - treasury.withdrawn_this_week).clamp(0, treasury.strands.max(0))
        }
        _ => 0,
    }
}

/// The treasury's fields, as typed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Purse {
    pub deposit: String,
    pub withdraw: String,
    /// What a withdrawal is for, kept in the books
    pub note: String,
    pub limit: String,
}

/// A whole number of Strands as typed, or what's wrong with it. Commas and
/// spaces are allowed, as the panel writes them.
pub fn amount(typed: &str) -> Result<i64, String> {
    let digits: String = typed.chars().filter(|c| !matches!(c, ',' | ' ' | '_')).collect();
    if digits.is_empty() {
        return Err("How many Strands?".into());
    }
    match digits.parse::<i64>() {
        Ok(n) if n > 0 => Ok(n),
        Ok(_) => Err("A whole number of Strands, more than none.".into()),
        Err(_) => Err("A whole number of Strands.".into()),
    }
}

impl Purse {
    /// The deposit as typed, against what you hold.
    pub fn check_deposit(&self, balance: Option<i64>) -> Result<i64, String> {
        let n = amount(&self.deposit)?;
        match balance {
            Some(held) if held < n => Err(format!("You hold {}.", strands_words(held))),
            _ => Ok(n),
        }
    }

    /// The withdrawal as typed, against what `role` may take.
    pub fn check_withdraw(&self, role: Option<GuildRole>, treasury: &GuildTreasury) -> Result<i64, String> {
        let n = amount(&self.withdraw)?;
        let most = withdrawable(role, treasury);
        if n <= most {
            return Ok(n);
        }
        Err(match role {
            Some(GuildRole::Leader) => format!("The treasury holds {}.", strands_words(treasury.strands)),
            _ if most == 0 => "You've taken your limit this week.".into(),
            _ => format!("You may take {} more this week.", strands_words(most)),
        })
    }

    /// The officers' weekly limit as typed: none is allowed, to stop them.
    pub fn check_limit(&self) -> Result<i64, String> {
        if self.limit.trim() == "0" {
            return Ok(0);
        }
        amount(&self.limit)
    }
}

/// What a free lot's panel offers about a hall (S4.3), for a lot in the
/// guild district.
#[derive(Debug, Clone, PartialEq)]
pub enum HallOffer {
    /// You lead a guild with no hall, it can pay, and you're at the hall
    Claim { guild_id: Uuid, guild: String, cost: i64 },
    /// As above, away from the Exchange hall, where it's done
    AwayFromHall { guild: String, cost: i64 },
    /// The treasury is `short` of the deed
    Short { guild: String, cost: i64, short: i64 },
    /// Your guild has a hall already
    HasAHall { guild: String },
    /// You're in a guild and don't lead it
    NotLeader { guild: String },
    /// You're in no guild
    NoGuild,
}

/// What `plot`'s panel says of halls to `mine`, standing `at_hall` or not:
/// `None` for a lot outside the guild district, or one that's held. The
/// deed is the plot's `deed_cost`, and the treasury pays it.
pub fn hall_offer(plot: &PlotInfo, mine: Option<&MyGuild>, at_hall: bool) -> Option<HallOffer> {
    if plot.zone_id != Zone::GuildDistrict.as_str() || plot.owner_id.is_some() {
        return None;
    }
    let cost = plot.deed_cost? as i64;
    let Some(guild) = mine.and_then(|m| m.guild.as_ref()) else {
        return Some(HallOffer::NoGuild);
    };
    let name = guild.name.clone();
    if mine.and_then(|m| m.role) != Some(GuildRole::Leader) {
        return Some(HallOffer::NotLeader { guild: name });
    }
    if guild.hall.is_some() {
        return Some(HallOffer::HasAHall { guild: name });
    }
    let held = guild.treasury.as_ref().map_or(0, |t| t.strands);
    Some(if held < cost {
        HallOffer::Short { guild: name, cost, short: cost - held }
    } else if !at_hall {
        HallOffer::AwayFromHall { guild: name, cost }
    } else {
        HallOffer::Claim { guild_id: guild.id, guild: name, cost }
    })
}

impl HallOffer {
    /// The line under the price.
    pub fn words(&self) -> String {
        match self {
            HallOffer::Claim { guild, .. } => format!("{} treasury pays the deed, and the hall's tax after.", possessive(guild)),
            HallOffer::AwayFromHall { .. } => "A hall is bought at the Exchange hall, in the Capitol.".into(),
            HallOffer::Short { guild, short, .. } => format!("{} treasury is {} short.", possessive(guild), strands_words(*short)),
            HallOffer::HasAHall { guild } => format!("{guild} holds a hall already. A guild holds one."),
            HallOffer::NotLeader { guild } => format!("A guild hall. {} leader buys one for the guild.", possessive(guild)),
            HallOffer::NoGuild => "A guild hall: a guild's leader buys it from the treasury.".into(),
        }
    }

    /// The deed, where there's one to show.
    pub fn cost(&self) -> Option<i64> {
        match self {
            HallOffer::Claim { cost, .. } | HallOffer::AwayFromHall { cost, .. } | HallOffer::Short { cost, .. } => Some(*cost),
            _ => None,
        }
    }
}

/// Where a free lot in the guild district stands, for the leader choosing a
/// hall (#581): its side of the guild ring, by its terrain id
/// (`guild_plot_in_*`, `guild_plot_out_*`), and its bearing from the tower.
/// "Inside the guild ring, to the north-east".
pub fn hall_lot_words(terrain_id: &str, x: f64, y: f64) -> String {
    let side = if terrain_id.starts_with("guild_plot_in_") {
        "Inside the guild ring"
    } else if terrain_id.starts_with("guild_plot_out_") {
        "Outside the guild ring"
    } else {
        "In the guild district"
    };
    format!("{side}, to the {}", crate::deeds::bearing(x, y))
}

/// What the leader can do with a free hall's lot in the list, with
/// `treasury` Strands against a deed of `cost`, standing `at_hall` or not:
/// the plot panel's [`hall_offer`], for every lot at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HallChoice {
    Claim,
    /// The treasury is this short of the deed
    Short(i64),
    /// It's bought at the Exchange hall
    AwayFromHall,
}

pub fn hall_choice(treasury: i64, cost: i64, at_hall: bool) -> HallChoice {
    if treasury < cost {
        HallChoice::Short(cost - treasury)
    } else if !at_hall {
        HallChoice::AwayFromHall
    } else {
        HallChoice::Claim
    }
}

/// "The Kiln Yard's", "The Rope Walkers'".
pub fn possessive(name: &str) -> String {
    if name.ends_with('s') {
        format!("{name}'")
    } else {
        format!("{name}'s")
    }
}

/// "1 Strand", "1,240 Strands".
pub fn strands_words(n: i64) -> String {
    if n.abs() == 1 {
        format!("{n} Strand")
    } else {
        format!("{} Strands", thousands(n))
    }
}

/// "1,240", "-80".
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// How the week's dues are made up: "50, and 5 for each of 6 members,
/// charged by the hour".
pub fn dues_words(members: usize, hall: Option<&GuildHall>) -> String {
    let dues = format!(
        "{}, and {} for each of {}, charged by the hour",
        guild::DUES_BASE,
        guild::DUES_PER_MEMBER,
        members_words(members),
    );
    match hall {
        Some(hall) if hall.weekly_tax > 0 => format!("{dues}, and the hall's {} a week", thousands(hall.weekly_tax)),
        _ => dues,
    }
}

/// Where a guild's hall stands, for the line over its door: its address,
/// or the district for a plot with none (#582).
pub fn hall_where(hall: &GuildHall) -> String {
    match &hall.address {
        Some(address) => format!("over the door at {address}"),
        None => "over the door, in the guild district".into(),
    }
}

/// A guild's hall's tax, as its members plan for it (#582).
pub fn hall_tax_words(hall: &GuildHall) -> String {
    if hall.weekly_tax > 0 {
        format!("Its tax is {} a week, from the treasury by the hour.", strands_words(hall.weekly_tax))
    } else {
        "Its tax comes from the treasury by the hour.".into()
    }
}

/// How far the debt has gone toward dissolving the guild, 0 to 1.
pub fn debt_share(treasury: &GuildTreasury) -> f32 {
    if treasury.dissolves_at <= 0 {
        return 0.0;
    }
    (treasury.owed as f32 / treasury.dissolves_at as f32).clamp(0.0, 1.0)
}

/// The debt, in a line, or `None` with none owed.
pub fn debt_words(treasury: &GuildTreasury) -> Option<String> {
    (treasury.owed > 0).then(|| {
        format!(
            "Owes {} in dues. Deposits pay it off first; at {} owed the guild is dissolved, and what's left shared.",
            strands_words(treasury.owed),
            thousands(treasury.dissolves_at),
        )
    })
}

/// What moved the treasury, in a word or two.
pub fn entry_kind_words(kind: GuildEntryKind) -> &'static str {
    match kind {
        GuildEntryKind::Deposit => "Deposit",
        GuildEntryKind::Withdrawal => "Withdrawal",
        GuildEntryKind::Dues => "Dues",
        GuildEntryKind::Fee => "Roster fee",
        GuildEntryKind::Share => "Share",
        GuildEntryKind::Hall => "The hall's deed",
        GuildEntryKind::Tax => "The hall's tax",
        GuildEntryKind::Refund => "The deed refunded",
    }
}

/// A line of the books: what, who, and the note, as "Withdrawal · Ada
/// Reeder · rope for the sheds". A note that only says what it is ("Dues")
/// isn't said twice.
pub fn entry_words(entry: &GuildEntry) -> String {
    let kind = entry_kind_words(entry.kind);
    let mut line = kind.to_string();
    if let Some(by) = &entry.by {
        line.push_str(" · ");
        line.push_str(&by.name);
    }
    let note = entry.note.trim();
    if !note.is_empty() && !note.eq_ignore_ascii_case(kind) {
        line.push_str(" · ");
        line.push_str(note);
    }
    line
}

/// A line of the books as the panel shows it: an entry, or a run of the
/// hourly ones folded together.
#[derive(Debug, Clone, PartialEq)]
pub struct BookLine {
    /// The newest of them
    pub at: chrono::DateTime<chrono::Utc>,
    pub kind: GuildEntryKind,
    pub words: String,
    /// What the treasury gained, negative for what it lost, all told
    pub strands: i64,
    /// The treasury after the newest
    pub treasury: i64,
}

/// `entries`, newest first, as lines: a run of hours' dues, or of the
/// hall's tax, one line ("Dues, 12 hours"), since the server books each
/// hour and a page of them would hide everything else.
pub fn book_lines(entries: &[GuildEntry]) -> Vec<BookLine> {
    let hourly = |k: GuildEntryKind| matches!(k, GuildEntryKind::Dues | GuildEntryKind::Tax);
    let mut lines: Vec<(BookLine, usize)> = Vec::new();
    for entry in entries {
        if let Some((last, n)) = lines.last_mut() {
            if hourly(entry.kind) && last.kind == entry.kind && entry.note.trim().eq_ignore_ascii_case(entry_kind_words(entry.kind)) {
                last.strands += entry.strands;
                *n += 1;
                continue;
            }
        }
        let line = BookLine {
            at: entry.at,
            kind: entry.kind,
            words: entry_words(entry),
            strands: entry.strands,
            treasury: entry.treasury,
        };
        lines.push((line, 1));
    }
    lines
        .into_iter()
        .map(|(mut line, n)| {
            if n > 1 {
                line.words = format!("{}, {n} hours", entry_kind_words(line.kind));
            }
            line
        })
        .collect()
}

/// "+50", "-8".
pub fn signed(n: i64) -> String {
    if n > 0 {
        format!("+{}", thousands(n))
    } else {
        thousands(n)
    }
}

/// Your guilds, and the calls for them.
#[derive(Debug, Clone, Default)]
pub struct Guilds {
    mine: Option<MyGuild>,
    /// Your guild's books, by its id, newest first
    books: Option<(Uuid, Vec<GuildEntry>)>,
    books_stale: bool,
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
        if self.books_stale {
            if let Some(id) = self.guild().filter(|g| g.treasury.is_some()).map(|g| g.id) {
                return Some(Ask::Books(id));
            }
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
            Ask::Books(_) => self.books_stale = false,
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

    /// The panel opened on your guild: it and its books are read again,
    /// since dues move the treasury by the hour and say nothing.
    pub fn look_in(&mut self) {
        self.stale = true;
        self.books_stale = true;
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
        // Whatever moved the guild may have moved its treasury: books once
        // read are read again
        if self.books.is_some() && mine.guild.as_ref().is_some_and(|g| g.treasury.is_some()) {
            self.books_stale = true;
        }
        if self.books.as_ref().map(|b| b.0) != mine.guild.as_ref().map(|g| g.id) {
            self.books = None;
        }
        self.mine = Some(mine);
    }

    /// `GET /guilds/{id}/books` came back.
    pub fn booked(&mut self, entries: Vec<GuildEntry>) {
        if let Some(Ask::Books(id)) = self.asked {
            self.asked = None;
            if self.guild_id() == Some(id) {
                self.books = Some((id, entries));
            }
        }
    }

    /// Your guild's books, newest first, once read.
    pub fn books(&self) -> Option<&[GuildEntry]> {
        let (id, entries) = self.books.as_ref()?;
        (self.guild_id() == Some(*id)).then_some(entries.as_slice())
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
            Some(Ask::Books(id)) => {
                // Shown empty rather than reading for ever
                self.books.get_or_insert((id, Vec::new()));
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
        !self.pending.is_empty()
            || self.asked.as_ref().is_some_and(|a| !matches!(a, Ask::Mine | Ask::Registry | Ask::Books(_)))
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
            home_address: None,
            standing: "New in town".into(),
            guild: None,
            lately: None,
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

    fn treasury(strands: i64, limit: i64, withdrawn: i64) -> GuildTreasury {
        GuildTreasury {
            strands,
            owed: 0,
            weekly_dues: guild::weekly_dues(6),
            dissolves_at: 4 * guild::weekly_dues(6),
            withdrawal_limit: limit,
            withdrawn_this_week: withdrawn,
        }
    }

    #[test]
    fn the_leader_takes_anything_an_officer_their_limit_a_member_nothing() {
        let t = treasury(900, 200, 150);
        assert_eq!(withdrawable(Some(GuildRole::Leader), &t), 900);
        assert_eq!(withdrawable(Some(GuildRole::Officer), &t), 50, "what's left of the week");
        assert_eq!(withdrawable(Some(GuildRole::Member), &t), 0);
        assert_eq!(withdrawable(None, &t), 0);
        assert_eq!(withdrawable(Some(GuildRole::Officer), &treasury(30, 200, 0)), 30, "no more than it holds");
        assert_eq!(withdrawable(Some(GuildRole::Officer), &treasury(900, 200, 260)), 0, "never below none");
    }

    #[test]
    fn the_purse_checks_what_is_typed() {
        assert_eq!(amount("1,240"), Ok(1240));
        assert!(amount("").is_err() && amount("0").is_err() && amount("-5").is_err() && amount("ten").is_err());
        let purse = Purse { deposit: "500".into(), withdraw: "80".into(), note: String::new(), limit: "0".into() };
        assert_eq!(purse.check_deposit(Some(600)), Ok(500));
        assert_eq!(purse.check_deposit(Some(400)), Err("You hold 400 Strands.".into()));
        assert_eq!(purse.check_deposit(None), Ok(500), "the server decides while your balance isn't known");
        let t = treasury(900, 200, 150);
        assert_eq!(purse.check_withdraw(Some(GuildRole::Leader), &t), Ok(80));
        assert_eq!(purse.check_withdraw(Some(GuildRole::Officer), &t), Err("You may take 50 Strands more this week.".into()));
        assert_eq!(purse.check_withdraw(Some(GuildRole::Officer), &treasury(900, 200, 200)), Err("You've taken your limit this week.".into()));
        assert_eq!(purse.check_limit(), Ok(0), "a limit of none stops the officers");
    }

    fn lot(zone: &str, owner: Option<Uuid>) -> PlotInfo {
        use capitol_protocol::rest::Bounds;
        PlotInfo {
            guild: None,
            id: Uuid::from_u128(9),
            zone_id: zone.into(),
            world_x: 0.0,
            world_y: 0.0,
            bounds: Bounds::new(0.0, 0.0, 10.0, 10.0),
            size_category: "medium".into(),
            plot_type: "standard".into(),
            owner_id: owner,
            owner_name: None,
            claimed_at: None,
            assessed_value: 0,
            tax_owed: 0,
            forecloses_at: None,
            deed_cost: Some(5000),
            station_count: 0,
            station_capacity: 9,
            terrain_id: None,
            address: None,
            corners: None,
            indoor_room: None,
            yard_room: None,
            owner_citizen: false,
            floor_ready_at: None,
            floors: 0,
            next_floor: None,
        }
    }

    #[test]
    fn only_a_leader_with_the_strands_claims_a_hall_and_at_the_exchange() {
        let free = lot("guild_district", None);
        assert_eq!(hall_offer(&lot("urban", None), None, true), None, "a home lot isn't a hall");
        assert_eq!(hall_offer(&lot("guild_district", Some(BRAM)), None, true), None, "a held one isn't offered");
        assert_eq!(hall_offer(&free, None, true), Some(HallOffer::NoGuild));
        let mut rich = mine(GuildRole::Leader);
        rich.guild.as_mut().unwrap().treasury = Some(treasury(6000, 200, 0));
        let name = "The Rope Walkers".to_string();
        assert_eq!(hall_offer(&free, Some(&rich), true), Some(HallOffer::Claim { guild_id: ROPE, guild: name.clone(), cost: 5000 }));
        assert_eq!(hall_offer(&free, Some(&rich), false), Some(HallOffer::AwayFromHall { guild: name.clone(), cost: 5000 }));
        let mut poor = rich.clone();
        poor.guild.as_mut().unwrap().treasury = Some(treasury(1200, 200, 0));
        assert_eq!(hall_offer(&free, Some(&poor), true), Some(HallOffer::Short { guild: name.clone(), cost: 5000, short: 3800 }));
        let mut officer = rich.clone();
        officer.role = Some(GuildRole::Officer);
        assert_eq!(hall_offer(&free, Some(&officer), true), Some(HallOffer::NotLeader { guild: name.clone() }));
        let mut housed = rich.clone();
        housed.guild.as_mut().unwrap().hall = Some(capitol_protocol::rest::GuildHall {
            plot_id: Uuid::from_u128(40),
            terrain_id: None,
            address: None,
            x: 0.0,
            y: 0.0,
            claimed_at: when(),
            weekly_tax: 0,
            tax_owed: 0,
        });
        assert_eq!(hall_offer(&free, Some(&housed), true), Some(HallOffer::HasAHall { guild: name }));
    }

    /// The list of free halls (#581) offers what the plot panel would
    #[test]
    fn a_free_hall_is_placed_and_claimed_as_the_plot_panel_would() {
        use capitol_protocol::rules::{CENTER_X, CENTER_Y};
        assert_eq!(hall_lot_words("guild_plot_in_1_2", CENTER_X + 500.0, CENTER_Y - 500.0), "Inside the guild ring, to the north-east");
        assert_eq!(hall_lot_words("guild_plot_out_4_0", CENTER_X, CENTER_Y + 800.0), "Outside the guild ring, to the south");
        assert_eq!(hall_lot_words("guild_hall_x", CENTER_X - 800.0, CENTER_Y), "In the guild district, to the west");
        assert_eq!(hall_choice(6240, 5000, true), HallChoice::Claim);
        assert_eq!(hall_choice(6240, 5000, false), HallChoice::AwayFromHall);
        assert_eq!(hall_choice(1200, 5000, true), HallChoice::Short(3800), "short, wherever you stand");
    }

    #[test]
    fn the_books_are_read_with_the_guild_and_after_it_moves() {
        let mut guilds = Guilds::default();
        guilds.connected();
        let mut rich = mine(GuildRole::Officer);
        rich.guild.as_mut().unwrap().treasury = Some(treasury(900, 200, 0));
        guilds.sent(Ask::Mine);
        guilds.answered(rich.clone());
        assert_eq!(guilds.wants(), None, "books wait for the panel");
        guilds.look_in();
        assert_eq!(guilds.wants(), Some(Ask::Mine), "your guild first");
        guilds.sent(Ask::Mine);
        guilds.answered(rich.clone());
        assert_eq!(guilds.wants(), Some(Ask::Books(ROPE)));
        guilds.sent(Ask::Books(ROPE));
        assert!(!guilds.busy(), "a read holds no button still");
        let entry = GuildEntry {
            at: when(),
            kind: GuildEntryKind::Withdrawal,
            strands: -80,
            treasury: 900,
            by: Some(who(BRAM, "Bram")),
            note: "rope for the sheds".into(),
        };
        guilds.booked(vec![entry.clone()]);
        assert_eq!(guilds.books().map(<[_]>::len), Some(1));
        assert_eq!(entry_words(&entry), "Withdrawal · Bram · rope for the sheds");

        // A deposit answers with the guild, and the books are read again
        let deposit = Ask::Deposit { guild_id: ROPE, strands: 50 };
        guilds.queue(deposit.clone());
        guilds.sent(deposit);
        guilds.answered(rich);
        assert_eq!(guilds.done(), Some("You put 50 Strands in the treasury."));
        assert_eq!(guilds.wants(), Some(Ask::Books(ROPE)));

        // Out of the guild, its books go
        guilds.sent(Ask::Books(ROPE));
        guilds.answered(MyGuild::default());
        assert_eq!(guilds.books(), None);
    }

    #[test]
    fn a_run_of_hourly_dues_is_one_line() {
        let entry = |kind, strands, treasury, note: &str| GuildEntry {
            at: when(),
            kind,
            strands,
            treasury,
            by: (kind == GuildEntryKind::Deposit).then(|| who(BRAM, "Bram")),
            note: note.into(),
        };
        let entries = vec![
            entry(GuildEntryKind::Dues, -1, 897, "Dues"),
            entry(GuildEntryKind::Dues, -1, 898, "Dues"),
            entry(GuildEntryKind::Dues, 0, 899, "Dues"),
            entry(GuildEntryKind::Deposit, 100, 899, ""),
            entry(GuildEntryKind::Dues, -5, 799, "Dues, and 4 owed"),
            entry(GuildEntryKind::Tax, -2, 804, "Tax"),
        ];
        let lines = book_lines(&entries);
        let words: Vec<_> = lines.iter().map(|l| (l.words.as_str(), l.strands, l.treasury)).collect();
        assert_eq!(
            words,
            [
                ("Dues, 3 hours", -2, 897),
                ("Deposit · Bram", 100, 899),
                ("Dues · Dues, and 4 owed", -5, 799),
                ("The hall's tax · Tax", -2, 804),
            ]
        );
    }

    #[test]
    fn the_treasury_words() {
        assert_eq!(thousands(1_240), "1,240");
        assert_eq!(thousands(-1_000_000), "-1,000,000");
        assert_eq!(signed(50), "+50");
        assert_eq!(signed(-8), "-8");
        assert_eq!(strands_words(1), "1 Strand");
        assert_eq!(possessive("The Rope Walkers"), "The Rope Walkers'");
        assert_eq!(possessive("Oak & Iron"), "Oak & Iron's");
        let mut t = treasury(0, 200, 0);
        assert_eq!(t.weekly_dues, 80);
        assert_eq!(dues_words(6, None), "50, and 5 for each of 6 members, charged by the hour");
        let mut hall = capitol_protocol::rest::GuildHall {
            plot_id: Uuid::from_u128(40),
            terrain_id: Some("guild_3_12".into()),
            address: Some("3 Guild Ring".into()),
            x: 0.0,
            y: 0.0,
            claimed_at: chrono::DateTime::UNIX_EPOCH,
            weekly_tax: 1_200,
            tax_owed: 0,
        };
        assert_eq!(dues_words(6, Some(&hall)), "50, and 5 for each of 6 members, charged by the hour, and the hall's 1,200 a week");
        assert_eq!(hall_where(&hall), "over the door at 3 Guild Ring");
        assert_eq!(hall_tax_words(&hall), "Its tax is 1,200 Strands a week, from the treasury by the hour.");
        hall.address = None;
        hall.weekly_tax = 0;
        assert_eq!(dues_words(1, Some(&hall)), "50, and 5 for each of 1 member, charged by the hour");
        assert_eq!(hall_where(&hall), "over the door, in the guild district");
        assert_eq!(debt_words(&t), None);
        t.owed = 80;
        assert!((debt_share(&t) - 0.25).abs() < 1e-6, "a week's dues of four");
        assert!(debt_words(&t).unwrap().starts_with("Owes 80 Strands in dues."));
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
