//! Guilds (commons plan D10, S4.1): the registry, roles and the roster.
//!
//! A subscriber founds a guild for [`guild::FOUNDING_FEE`] (the route checks
//! they stand at the Exchange hall), and leads it. An officer or the leader
//! invites, or a player applies, and the other side's same action accepts:
//! inviting someone who applied takes them in, and so does applying to a
//! guild that invited you. Anyone can leave, but a leader only once they're
//! the last, which disbands the guild. Officers expel members, the leader
//! officers too, and only the leader sets roles and hands on the lead.
//!
//! Each change to the roster costs [`guild::ROSTER_FEE`] (a `guild_fee`, a
//! sink): from the treasury when it has a Strand, else from whoever makes the
//! change. Leaving is free.
//!
//! **The treasury** (S4.2) is held for the guild, outside every balance, and
//! counted in the ledger's held Strands ([`held`]). Any member deposits, from
//! anywhere; an officer withdraws up to the weekly limit the leader sets, the
//! leader without one. Every move is a `guild_ledger` row its members read
//! ([`books`]). [`collect_dues`] charges the week's dues
//! ([`guild::weekly_dues`]) hourly with the property tax, in carried
//! fractions through [`charge_hour`]. What the treasury can't pay is debt,
//! paid off first as Strands come in. Officers hear as the debt passes each
//! week, and at four weeks' worth the guild is dissolved: the treasury pays
//! the debt, what's left is shared among the members by tenure, and the
//! roster is cleared ([`close`]). Dues and fees leave the holding for no
//! one, so they're written only in the treasury's books, and the ledger reads
//! them back as `held_flows` ([`held_flows`]).
//!
//! **A hall** (S4.3) is a guild-district lot the leader claims for the
//! guild at the Exchange hall ([`claim_hall`]), its deed paid from the
//! treasury. The guild owns it (`plots.guild_id`) and its leader acts for it
//! as its owner, so the hall passes with the lead. Its officers put stations
//! there, and its `guild` level opens them to the members. Its tax comes from
//! the treasury hourly ([`hall_tax_hour`]), debt first, with the officers
//! warned; at four weeks' debt it's foreclosed and the deed, less the debt,
//! goes back into the treasury. A guild that closes gives its hall up first
//! ([`close`]). The deed and the tax leave the treasury for no one and the
//! refund comes in from no one, so they're read as `held_flows` too.
//!
//! A leader whose subscription has lapsed keeps the guild but can't invite.
//! After [`guild::LEAD_LAPSE_DAYS`] lapsed, [`pass_lapsed_leads`] gives the
//! lead to the longest-serving officer, or else the longest-serving member,
//! a subscriber before one who isn't. A leader who never subscribed (the
//! lead passed to them) counts as lapsed from when it came to them.
//!
//! Every write is one transaction holding the guild's row, so two officers
//! can't cross. One guild per player is the members table's key. Each one
//! returns a [`Told`]: who hears `guild_changed`, and the notices queued for
//! those named, which wait for anyone away ([`notices::queue`]).

use std::collections::HashMap;

use capitol_protocol::rest::{
    Correspondent, GuildDetail, GuildInvite, GuildInviteKind, GuildMember, GuildSummary, MyGuild, Subscription,
    SubscriptionStatus,
};
use capitol_protocol::rules::guild::{self, GuildRole, NameError};
use capitol_protocol::rules::standing;
use capitol_protocol::ws::{GuildNoticeKind, ServerMessage};
use chrono::{DateTime, Duration, SubsecRound, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::currency::{add_strands_in, announce_balances, balances_of, subtract_strands_in, TransactionType};
use super::gazette::{self, DeedEvent};
use super::ledger::Flows;
use super::notices;
use super::plot_sync::release_plot;
use super::property_tax::{charge_hour, foreclosure_threshold, notice_due, weekly_tax, TaxNotice};
use super::station_removal::{return_plot_stations, ReturnedStations};
use super::GameEvent;
use crate::models::StationState;
use capitol_protocol::rest::{GuildBooks, GuildEntry, GuildEntryKind, GuildHall, GuildTreasury, PlotGuild};
use capitol_protocol::rules::Zone;
use capitol_protocol::tunables::Tunables;

/// Why something asked of a guild was refused.
#[derive(Debug)]
pub enum GuildError {
    Name(NameError),
    NameTaken,
    TagTaken,
    /// Founding, or leading, needs a subscription: the refusal's words
    Unsubscribed(String),
    /// You're in a guild already
    InAGuild,
    /// They're in a guild already
    TheyreInAGuild,
    /// No guild standing by that id
    NoGuild,
    /// Nobody by that id
    NoOne,
    /// They aren't in this guild
    NotAMember,
    /// You aren't in that guild
    NotInAGuild,
    /// Your role can't do this: the words
    NotAllowed(&'static str),
    /// Not enough Strands to found one
    Founding,
    /// Not enough Strands for a roster change
    RosterFee,
    /// No such invitation or application
    NoInvite,
    /// Already asked, and waiting
    AlreadyWaiting,
    /// A leader with members left must hand on the lead first
    LeaderLeaving,
    /// A deposit, withdrawal or limit that isn't a number of Strands
    Strands,
    /// You haven't the Strands to deposit
    Short,
    /// The treasury holds only so many
    TreasuryShort(i64),
    /// Over an officer's weekly limit: what's left of it
    OverLimit(i64),
    /// The guild holds a hall already (S4.3)
    HasAHall,
    /// No such plot
    NoLot,
    /// Not a lot in the guild district
    NotAHallLot,
    /// Someone holds that lot
    LotTaken,
    /// The treasury can't pay for the hall
    HallShort { cost: i64, has: i64 },
    Db(sqlx::Error),
}

impl GuildError {
    /// Words a client can show.
    pub fn message(&self) -> String {
        match self {
            GuildError::Name(e) => e.message(),
            GuildError::NameTaken => "A guild goes by that name already".into(),
            GuildError::TagTaken => "A guild wears that tag already".into(),
            GuildError::Unsubscribed(why) => why.clone(),
            GuildError::InAGuild => "You're in a guild already".into(),
            GuildError::TheyreInAGuild => "They're in a guild already".into(),
            GuildError::NoGuild => "There's no such guild".into(),
            GuildError::NoOne => "No one goes by that".into(),
            GuildError::NotAMember => "They aren't in this guild".into(),
            GuildError::NotInAGuild => "You aren't in that guild".into(),
            GuildError::NotAllowed(why) => (*why).into(),
            GuildError::Founding => format!("Founding a guild costs {} Strands", guild::FOUNDING_FEE),
            GuildError::RosterFee => format!(
                "A change to the roster costs {} Strand, and you haven't one",
                guild::ROSTER_FEE
            ),
            GuildError::NoInvite => "There's no such invitation or application".into(),
            GuildError::AlreadyWaiting => "That's asked already, and waiting".into(),
            GuildError::LeaderLeaving => "Hand on the lead before you leave".into(),
            GuildError::Strands => "Name a number of Strands".into(),
            GuildError::Short => "You haven't that many Strands".into(),
            GuildError::TreasuryShort(has) => format!("The treasury holds only {has} Strands"),
            GuildError::OverLimit(left) => format!("That's over your weekly limit: {left} Strands left of it"),
            GuildError::HasAHall => "The guild holds a hall already".into(),
            GuildError::NoLot => "There's no such lot".into(),
            GuildError::NotAHallLot => "A guild's hall stands in the guild district".into(),
            GuildError::LotTaken => "Someone holds that lot already".into(),
            GuildError::HallShort { cost, has } => {
                format!("A hall costs {cost} Strands from the treasury, and it holds {has}")
            }
            GuildError::Db(e) => format!("Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for GuildError {
    fn from(e: sqlx::Error) -> Self {
        GuildError::Db(e)
    }
}

/// What a change means to tell people, once it's committed.
#[derive(Debug, Default)]
pub struct Told {
    pub guild_id: Uuid,
    /// Who hears `guild_changed`: the members, and anyone who just left
    pub changed: Vec<Uuid>,
    /// Each queued notice: who for, its id and the message to send live
    pub notices: Vec<(Uuid, Uuid, ServerMessage)>,
    /// Whose balance changed, to be told it: who paid a fee, deposited,
    /// withdrew or was given a share
    pub paid: Vec<Uuid>,
    /// Events for everyone (S4.3): a hall claimed, or its leader changed
    pub broadcast: Vec<GameEvent>,
    /// A hall given up as the guild closed (S4.3): the game drops its
    /// stations with [`Told::release_halls`] before it's sent
    pub halls: Vec<HallReleased>,
    /// The kits its stations gave back, counted as the change commits
    pub flows: Flows,
}

/// A guild's hall given up as the guild closed (S4.3): its stations came
/// down, their kits and contents went home, and the lot is free.
#[derive(Debug, Clone, PartialEq)]
pub struct HallReleased {
    pub terrain_id: Option<String>,
    pub removed_stations: Vec<Uuid>,
    /// `(player, container)` for each inventory or impound that took
    /// something from them
    pub returned_to: Vec<(Uuid, Uuid)>,
}

impl Told {
    /// Drops a closed guild's hall's stations from the game, and tells
    /// everyone the lot is free and each player whose inventory took
    /// something. Call it before [`Told::send`], with the game held.
    pub fn release_halls(&mut self, stations: &mut HashMap<Uuid, StationState>, events: &broadcast::Sender<GameEvent>) {
        for hall in self.halls.drain(..) {
            for station_id in hall.removed_stations {
                stations.remove(&station_id);
                let _ = events.send(GameEvent::StationRemoved { station_id });
            }
            for (player_id, container_id) in hall.returned_to {
                let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
            }
            if let Some(terrain_id) = hall.terrain_id {
                let _ = events.send(GameEvent::PlotReleased { terrain_id });
            }
        }
    }

    /// Sends what a committed change tells: `guild_changed`, its notices
    /// (each taken as it's sent live) and the payer's balance.
    pub async fn send(self, db: &PgPool, events: &broadcast::Sender<GameEvent>) {
        for event in self.broadcast {
            let _ = events.send(event);
        }
        if !self.changed.is_empty() {
            let _ = events.send(GameEvent::GuildChanged { guild_id: self.guild_id, players: self.changed });
        }
        for (player_id, notice_id, message) in self.notices {
            let _ = events.send(GameEvent::BazaarNotice { player_id, notice_id, message });
        }
        if let Ok(balances) = balances_of(db, &self.paid).await {
            announce_balances(events, balances);
        }
    }
}

/// A guild's row, as the writes read it.
struct Held {
    name: String,
    tag: String,
}

/// `founder` founds a guild at `now`, paying [`guild::FOUNDING_FEE`]. The
/// route has checked they stand at the hall.
pub async fn found(
    db: &PgPool,
    founder: Uuid,
    name: &str,
    tag: &str,
    charter: &str,
    now: DateTime<Utc>,
) -> Result<(Uuid, Told), GuildError> {
    let name = guild::clean_name(name).map_err(GuildError::Name)?;
    let tag = guild::clean_tag(tag).map_err(GuildError::Name)?;
    let charter = guild::clean_charter(charter).map_err(GuildError::Name)?;
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let subscription = subscription_of(&mut tx, founder).await?.ok_or(GuildError::NoOne)?;
    if !subscription.is_active_at(now) {
        return Err(GuildError::Unsubscribed(crate::subscriptions::refusal(&subscription.as_of(now))));
    }
    if guild_of(&mut tx, founder).await?.is_some() {
        return Err(GuildError::InAGuild);
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO guilds (id, name, tag, charter, founded_at, founder_id, leader_id, leader_since)
         VALUES ($1, $2, $3, $4, $5, $6, $6, $5)",
    )
    .bind(id)
    .bind(&name)
    .bind(&tag)
    .bind(&charter)
    .bind(now)
    .bind(founder)
    .execute(&mut *tx)
    .await
    .map_err(|e| match unique_violation(&e) {
        Some("idx_guilds_name") => GuildError::NameTaken,
        Some("idx_guilds_tag") => GuildError::TagTaken,
        _ => GuildError::Db(e),
    })?;
    subtract_strands_in(
        &mut tx,
        founder,
        guild::FOUNDING_FEE,
        TransactionType::GuildFee,
        Some(&format!("Founding {name}")),
        Some(id),
    )
    .await
    .map_err(|e| fee_error(e, GuildError::Founding))?;
    join(&mut tx, id, founder, GuildRole::Leader, now).await?;
    tx.commit().await?;
    Ok((id, Told { guild_id: id, changed: vec![founder], notices: Vec::new(), paid: vec![founder], ..Default::default() }))
}

/// `actor`, an officer or the leader of `guild_id`, invites `player`. If
/// they'd applied, they're in.
pub async fn invite(db: &PgPool, actor: Uuid, guild_id: Uuid, player: Uuid, now: DateTime<Utc>) -> Result<Told, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    let role = role_in(&mut tx, guild_id, actor).await?.ok_or(GuildError::NotAllowed("You aren't in this guild"))?;
    if !role.may_invite() {
        return Err(GuildError::NotAllowed("Only an officer or the leader can invite"));
    }
    if role == GuildRole::Leader {
        let subscription = subscription_of(&mut tx, actor).await?.ok_or(GuildError::NoOne)?;
        if !subscription.is_active_at(now) {
            return Err(GuildError::Unsubscribed(format!(
                "A leader needs a subscription to invite. {}",
                crate::subscriptions::refusal(&subscription.as_of(now))
            )));
        }
    }
    let them = correspondent(&mut tx, player).await?.ok_or(GuildError::NoOne)?;
    if guild_of(&mut tx, player).await?.is_some() {
        return Err(GuildError::TheyreInAGuild);
    }
    match waiting(&mut tx, guild_id, player).await? {
        Some(GuildInviteKind::Apply) => {
            let paid = charge(&mut tx, actor, guild_id, &format!("{} joins {}", them.name, held.name), now).await?;
            join(&mut tx, guild_id, player, GuildRole::Member, now).await?;
            let changed = members_of(&mut tx, guild_id).await?;
            tx.commit().await?;
            Ok(Told { guild_id, changed, notices: Vec::new(), paid, ..Default::default() })
        }
        Some(GuildInviteKind::Invite) => Err(GuildError::AlreadyWaiting),
        None => {
            ask(&mut tx, guild_id, player, actor, GuildInviteKind::Invite, now).await?;
            let by = correspondent(&mut tx, actor).await?.ok_or(GuildError::NoOne)?;
            let notice = notify(&mut tx, player, guild_id, &held, GuildNoticeKind::Invited, &by).await?;
            let changed = officers_of(&mut tx, guild_id).await?;
            tx.commit().await?;
            Ok(Told { guild_id, changed, notices: vec![notice], paid: Vec::new(), ..Default::default() })
        }
    }
}

/// `player` applies to `guild_id`. If it had invited them, they're in.
pub async fn apply(db: &PgPool, player: Uuid, guild_id: Uuid, now: DateTime<Utc>) -> Result<Told, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    if guild_of(&mut tx, player).await?.is_some() {
        return Err(GuildError::InAGuild);
    }
    let me = correspondent(&mut tx, player).await?.ok_or(GuildError::NoOne)?;
    match waiting(&mut tx, guild_id, player).await? {
        Some(GuildInviteKind::Invite) => {
            let paid = charge(&mut tx, player, guild_id, &format!("Joining {}", held.name), now).await?;
            join(&mut tx, guild_id, player, GuildRole::Member, now).await?;
            let changed = members_of(&mut tx, guild_id).await?;
            tx.commit().await?;
            Ok(Told { guild_id, changed, notices: Vec::new(), paid, ..Default::default() })
        }
        Some(GuildInviteKind::Apply) => Err(GuildError::AlreadyWaiting),
        None => {
            ask(&mut tx, guild_id, player, player, GuildInviteKind::Apply, now).await?;
            let officers = officers_of(&mut tx, guild_id).await?;
            let mut notices = Vec::new();
            for officer in &officers {
                notices.push(notify(&mut tx, *officer, guild_id, &held, GuildNoticeKind::Applied, &me).await?);
            }
            tx.commit().await?;
            let mut changed = officers;
            changed.push(player);
            Ok(Told { guild_id, changed, notices, paid: Vec::new(), ..Default::default() })
        }
    }
}

/// Drop the invitation or application between `guild_id` and `player`:
/// declined or withdrawn by the player, or by an officer or the leader.
pub async fn withdraw(db: &PgPool, actor: Uuid, guild_id: Uuid, player: Uuid) -> Result<Told, GuildError> {
    let mut tx = db.begin().await?;
    hold(&mut tx, guild_id).await?;
    if actor != player {
        let role = role_in(&mut tx, guild_id, actor).await?;
        if !role.is_some_and(GuildRole::may_invite) {
            return Err(GuildError::NotAllowed("Only an officer or the leader can turn someone down"));
        }
    }
    let dropped = sqlx::query("DELETE FROM guild_invites WHERE guild_id = $1 AND player_id = $2")
        .bind(guild_id)
        .bind(player)
        .execute(&mut *tx)
        .await?;
    if dropped.rows_affected() == 0 {
        return Err(GuildError::NoInvite);
    }
    let mut changed = officers_of(&mut tx, guild_id).await?;
    changed.push(player);
    tx.commit().await?;
    Ok(Told { guild_id, changed, notices: Vec::new(), paid: Vec::new(), ..Default::default() })
}

/// `player` leaves their guild, `guild_id`, for nothing. A leader can leave
/// only as the last member, and the guild is disbanded: its treasury pays
/// what it owes, and what's left is theirs.
pub async fn leave(db: &PgPool, player: Uuid, guild_id: Uuid, now: DateTime<Utc>) -> Result<Told, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    if guild_of(&mut tx, player).await? != Some(guild_id) {
        return Err(GuildError::NotInAGuild);
    }
    let held = hold(&mut tx, guild_id).await?;
    let role = role_in(&mut tx, guild_id, player).await?.ok_or(GuildError::NotInAGuild)?;
    let others: Vec<Uuid> = members_of(&mut tx, guild_id).await?.into_iter().filter(|m| *m != player).collect();
    if role == GuildRole::Leader && !others.is_empty() {
        return Err(GuildError::LeaderLeaving);
    }
    if others.is_empty() {
        let mut told = close(&mut tx, guild_id, &held, Closing::Disbanded, now).await?;
        super::ledger::commit(tx, std::mem::take(&mut told.flows)).await?;
        return Ok(told);
    }
    sqlx::query("DELETE FROM guild_members WHERE player_id = $1")
        .bind(player)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut changed = others;
    changed.push(player);
    Ok(Told { guild_id, changed, notices: Vec::new(), paid: Vec::new(), ..Default::default() })
}

/// `actor` expels `player` from `guild_id`: an officer a member, the leader
/// an officer or a member.
pub async fn expel(db: &PgPool, actor: Uuid, guild_id: Uuid, player: Uuid, now: DateTime<Utc>) -> Result<Told, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    let role = role_in(&mut tx, guild_id, actor).await?.ok_or(GuildError::NotAllowed("You aren't in this guild"))?;
    let theirs = role_in(&mut tx, guild_id, player).await?.ok_or(GuildError::NotAMember)?;
    if actor == player {
        return Err(GuildError::NotAllowed("Leave, rather than expel yourself"));
    }
    if !role.may_expel(theirs) {
        return Err(GuildError::NotAllowed(match role {
            GuildRole::Member => "Only an officer or the leader can expel",
            _ => "Only the leader can expel an officer",
        }));
    }
    let them = correspondent(&mut tx, player).await?.ok_or(GuildError::NoOne)?;
    let paid = charge(&mut tx, actor, guild_id, &format!("{} expelled from {}", them.name, held.name), now).await?;
    sqlx::query("DELETE FROM guild_members WHERE player_id = $1")
        .bind(player)
        .execute(&mut *tx)
        .await?;
    let by = correspondent(&mut tx, actor).await?.ok_or(GuildError::NoOne)?;
    let notice = notify(&mut tx, player, guild_id, &held, GuildNoticeKind::Expelled, &by).await?;
    let mut changed = members_of(&mut tx, guild_id).await?;
    changed.push(player);
    tx.commit().await?;
    Ok(Told { guild_id, changed, notices: vec![notice], paid, ..Default::default() })
}

/// The leader `actor` makes `player` an officer or a member, or hands them
/// the lead and becomes an officer. The new leader needs a subscription.
/// Setting a role someone has already costs nothing.
pub async fn set_role(
    db: &PgPool,
    actor: Uuid,
    guild_id: Uuid,
    player: Uuid,
    to: GuildRole,
    now: DateTime<Utc>,
) -> Result<Told, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    let role = role_in(&mut tx, guild_id, actor).await?.ok_or(GuildError::NotAllowed("You aren't in this guild"))?;
    if !role.may_set_roles() {
        return Err(GuildError::NotAllowed("Only the leader sets roles"));
    }
    let theirs = role_in(&mut tx, guild_id, player).await?.ok_or(GuildError::NotAMember)?;
    if theirs == to {
        return Ok(Told { guild_id, changed: Vec::new(), notices: Vec::new(), paid: Vec::new(), ..Default::default() });
    }
    let them = correspondent(&mut tx, player).await?.ok_or(GuildError::NoOne)?;
    let mut notices = Vec::new();
    let mut broadcast = Vec::new();
    let paid;
    if to == GuildRole::Leader {
        let subscription = subscription_of(&mut tx, player).await?.ok_or(GuildError::NoOne)?;
        if !subscription.is_active_at(now) {
            return Err(GuildError::Unsubscribed("The lead goes only to someone with a subscription".into()));
        }
        paid = charge(&mut tx, actor, guild_id, &format!("The lead of {} to {}", held.name, them.name), now).await?;
        broadcast.extend(hand_on(&mut tx, guild_id, actor, player, now).await?);
        let by = correspondent(&mut tx, actor).await?.ok_or(GuildError::NoOne)?;
        notices.push(notify(&mut tx, player, guild_id, &held, GuildNoticeKind::Lead, &by).await?);
    } else {
        paid = charge(&mut tx, actor, guild_id, &format!("{} made {} in {}", them.name, to.as_str(), held.name), now).await?;
        sqlx::query("UPDATE guild_members SET role = $2 WHERE player_id = $1")
            .bind(player)
            .bind(to.as_str())
            .execute(&mut *tx)
            .await?;
    }
    let changed = members_of(&mut tx, guild_id).await?;
    tx.commit().await?;
    Ok(Told { guild_id, changed, notices, paid, broadcast, ..Default::default() })
}

/// Pass the lead of every guild whose leader has been lapsed
/// [`guild::LEAD_LAPSE_DAYS`] at `now`: to the longest-serving officer, or
/// else the longest-serving member, a subscriber first in each. A guild of
/// one keeps its leader. Run hourly beside the subscriptions' check. Errors
/// are logged, and that guild waits for the next run.
pub async fn pass_lapsed_leads(db: &PgPool, now: DateTime<Utc>) -> Vec<Told> {
    let cutoff = now - Duration::days(guild::LEAD_LAPSE_DAYS);
    let due: Vec<Uuid> = match sqlx::query_scalar(
        "SELECT g.id FROM guilds g
         JOIN players p ON p.id = g.leader_id JOIN accounts a ON a.id = p.account_id
         WHERE g.status = 'active'
           AND NOT (a.subscription_status = 'active' AND a.subscription_until > $1)
           AND GREATEST(COALESCE(a.subscription_until, '-infinity'), g.leader_since) <= $2",
    )
    .bind(now)
    .bind(cutoff)
    .fetch_all(db)
    .await
    {
        Ok(due) => due,
        Err(e) => {
            tracing::error!("Failed to find guilds whose leader lapsed: {:?}", e);
            return Vec::new();
        }
    };
    let mut told = Vec::new();
    for guild_id in due {
        match pass_lead(db, guild_id, now).await {
            Ok(Some(t)) => told.push(t),
            Ok(None) => {}
            Err(e) => tracing::error!("Failed to pass on the lead of guild {guild_id}: {:?}", e),
        }
    }
    told
}

async fn pass_lead(db: &PgPool, guild_id: Uuid, now: DateTime<Utc>) -> Result<Option<Told>, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    let leader: Uuid = sqlx::query_scalar("SELECT leader_id FROM guilds WHERE id = $1")
        .bind(guild_id)
        .fetch_one(&mut *tx)
        .await?;
    let next: Option<Uuid> = sqlx::query_scalar(
        "SELECT m.player_id FROM guild_members m
         JOIN players p ON p.id = m.player_id JOIN accounts a ON a.id = p.account_id
         WHERE m.guild_id = $1 AND m.player_id <> $2
         ORDER BY m.role = 'officer' DESC,
                  (a.subscription_status = 'active' AND a.subscription_until > $3) DESC,
                  m.joined_at, m.player_id
         LIMIT 1",
    )
    .bind(guild_id)
    .bind(leader)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(next) = next else {
        return Ok(None);
    };
    let broadcast = hand_on(&mut tx, guild_id, leader, next, now).await?.into_iter().collect();
    let by = correspondent(&mut tx, leader).await?.ok_or(GuildError::NoOne)?;
    let notice = notify(&mut tx, next, guild_id, &held, GuildNoticeKind::Lead, &by).await?;
    let changed = members_of(&mut tx, guild_id).await?;
    tx.commit().await?;
    tracing::info!("The lead of {} passed to {next}: its leader {leader} lapsed", held.name);
    Ok(Some(Told { guild_id, changed, notices: vec![notice], broadcast, ..Default::default() }))
}

/// `player`, a member of `guild_id`, puts `strands` of their own into its
/// treasury, from anywhere.
pub async fn deposit(db: &PgPool, player: Uuid, guild_id: Uuid, strands: i64, now: DateTime<Utc>) -> Result<Told, GuildError> {
    if strands <= 0 {
        return Err(GuildError::Strands);
    }
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    if role_in(&mut tx, guild_id, player).await?.is_none() {
        return Err(GuildError::NotInAGuild);
    }
    subtract_strands_in(
        &mut tx,
        player,
        strands,
        TransactionType::GuildDeposit,
        Some(&format!("Into the treasury of {}", held.name)),
        Some(guild_id),
    )
    .await
    .map_err(|e| fee_error(e, GuildError::Short))?;
    let after = move_treasury(&mut tx, guild_id, strands).await?;
    book(&mut tx, guild_id, Some(player), GuildEntryKind::Deposit, strands, after, "", now).await?;
    let changed = members_of(&mut tx, guild_id).await?;
    tx.commit().await?;
    Ok(Told { guild_id, changed, notices: Vec::new(), paid: vec![player], ..Default::default() })
}

/// `actor` takes `strands` out of `guild_id`'s treasury for themselves: the
/// leader as they like, an officer within the leader's limit over a rolling
/// week ([`guild::WITHDRAWAL_WINDOW_DAYS`]). `note` says what for.
pub async fn withdraw_strands(
    db: &PgPool,
    actor: Uuid,
    guild_id: Uuid,
    strands: i64,
    note: &str,
    now: DateTime<Utc>,
) -> Result<Told, GuildError> {
    if strands <= 0 {
        return Err(GuildError::Strands);
    }
    let note = capitol_protocol::rules::talk::clean(note).unwrap_or_default();
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    let role = role_in(&mut tx, guild_id, actor).await?.ok_or(GuildError::NotInAGuild)?;
    if !role.may_withdraw() {
        return Err(GuildError::NotAllowed("Only an officer or the leader can draw on the treasury"));
    }
    let treasury = treasury_of(&mut tx, guild_id).await?;
    if treasury < strands {
        return Err(GuildError::TreasuryShort(treasury));
    }
    if role == GuildRole::Officer {
        let left = limit_of(&mut tx, guild_id).await? - withdrawn_in_week(&mut tx, guild_id, actor, now).await?;
        if strands > left {
            return Err(GuildError::OverLimit(left.max(0)));
        }
    }
    let after = move_treasury(&mut tx, guild_id, -strands).await?;
    book(&mut tx, guild_id, Some(actor), GuildEntryKind::Withdrawal, -strands, after, &note, now).await?;
    add_strands_in(
        &mut tx,
        actor,
        strands,
        TransactionType::GuildWithdrawal,
        Some(&format!("From the treasury of {}", held.name)),
        Some(guild_id),
    )
    .await
    .map_err(|e| GuildError::Db(sqlx::Error::Protocol(e)))?;
    let changed = members_of(&mut tx, guild_id).await?;
    tx.commit().await?;
    Ok(Told { guild_id, changed, notices: Vec::new(), paid: vec![actor], ..Default::default() })
}

/// The leader `actor` sets what each officer of `guild_id` may withdraw in a
/// rolling week.
pub async fn set_limit(db: &PgPool, actor: Uuid, guild_id: Uuid, strands: i64) -> Result<Told, GuildError> {
    if strands < 0 {
        return Err(GuildError::Strands);
    }
    let mut tx = db.begin().await?;
    hold(&mut tx, guild_id).await?;
    let role = role_in(&mut tx, guild_id, actor).await?.ok_or(GuildError::NotInAGuild)?;
    if role != GuildRole::Leader {
        return Err(GuildError::NotAllowed("Only the leader sets the officers' limit"));
    }
    sqlx::query("UPDATE guilds SET withdrawal_limit = $2 WHERE id = $1")
        .bind(guild_id)
        .bind(strands)
        .execute(&mut *tx)
        .await?;
    let changed = officers_of(&mut tx, guild_id).await?;
    tx.commit().await?;
    Ok(Told { guild_id, changed, notices: Vec::new(), paid: Vec::new(), ..Default::default() })
}

/// `guild_id`'s treasury's moves, newest first, for `viewer`, who must be a
/// member. The latest [`BOOKS_SHOWN`].
pub async fn books(db: &PgPool, guild_id: Uuid, viewer: Uuid) -> Result<GuildBooks, GuildError> {
    let mut conn = db.acquire().await?;
    if guild_of(&mut conn, viewer).await? != Some(guild_id) {
        return Err(GuildError::NotInAGuild);
    }
    type Row = (DateTime<Utc>, String, i64, i64, Option<Uuid>, Option<String>, Option<bool>, String);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT l.occurred_at, l.kind, l.amount, l.treasury_after, p.id, p.name, a.kind = 'citizen', l.note
         FROM guild_ledger l
         LEFT JOIN players p ON p.id = l.player_id LEFT JOIN accounts a ON a.id = p.account_id
         WHERE l.guild_id = $1
         ORDER BY l.id DESC
         LIMIT $2",
    )
    .bind(guild_id)
    .bind(BOOKS_SHOWN)
    .fetch_all(&mut *conn)
    .await?;
    let entries = rows
        .into_iter()
        .filter_map(|(at, kind, strands, treasury, id, name, citizen, note)| {
            Some(GuildEntry {
                at,
                kind: GuildEntryKind::parse(&kind)?,
                strands,
                treasury,
                by: id.zip(name).map(|(id, name)| Correspondent {
                    id,
                    name,
                    citizen: citizen.unwrap_or(false),
                    office: false,
                }),
                note,
            })
        })
        .collect();
    Ok(GuildBooks { entries })
}

/// How many of a treasury's moves [`books`] shows.
pub const BOOKS_SHOWN: i64 = 200;

/// An hour's dues for every guild standing, from its treasury, at `now`:
/// what it can't pay is debt, paid off first as Strands come in. Its
/// officers hear as the debt passes each week of dues, and at four weeks'
/// the guild is dissolved. Run hourly with the property tax. Errors are
/// logged, and that guild waits for the next hour.
pub async fn collect_dues(db: &PgPool, now: DateTime<Utc>) -> Vec<Told> {
    let due: Vec<Uuid> = match sqlx::query_scalar("SELECT id FROM guilds WHERE status = 'active' ORDER BY founded_at, id")
        .fetch_all(db)
        .await
    {
        Ok(due) => due,
        Err(e) => {
            tracing::error!("Failed to find the guilds for their dues: {:?}", e);
            return Vec::new();
        }
    };
    let mut told = Vec::new();
    for guild_id in due {
        match dues_hour(db, guild_id, now).await {
            Ok(Some(t)) => told.push(t),
            Ok(None) => {}
            Err(GuildError::NoGuild) => {}
            Err(e) => tracing::error!("Failed to charge guild {guild_id} its dues: {:?}", e),
        }
    }
    told
}

/// One guild's hour of dues: [`collect_dues`].
async fn dues_hour(db: &PgPool, guild_id: Uuid, now: DateTime<Utc>) -> Result<Option<Told>, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    let (treasury, carry, owed): (i64, i32, i64) =
        sqlx::query_as("SELECT treasury, dues_carry, dues_owed FROM guilds WHERE id = $1")
            .bind(guild_id)
            .fetch_one(&mut *tx)
            .await?;
    let members = members_of(&mut tx, guild_id).await?.len() as u32;
    let (due, carry) = charge_hour(guild::weekly_dues(members), carry);
    let pay = treasury.min(owed + due);
    let owed_after = owed + due - pay;
    let after: i64 = sqlx::query_scalar(
        "UPDATE guilds SET treasury = treasury - $2, dues_carry = $3, dues_owed = $4 WHERE id = $1 RETURNING treasury",
    )
    .bind(guild_id)
    .bind(pay)
    .bind(carry)
    .bind(owed_after)
    .fetch_one(&mut *tx)
    .await?;
    if pay > 0 {
        let note = if pay > due { format!("Dues, and {} owed", pay - due) } else { "Dues".into() };
        book(&mut tx, guild_id, None, GuildEntryKind::Dues, -pay, after, &note, now).await?;
    }
    let mut told = if owed_after >= guild::dissolves_at(members) {
        tracing::warn!("{} is dissolved, owing {owed_after} Strands of dues", held.name);
        Some(close(&mut tx, guild_id, &held, Closing::Dissolved, now).await?)
    } else if guild::debt_warning_due(owed, owed_after, members) {
        let message = ServerMessage::GuildDues {
            guild_id,
            guild: held.name.clone(),
            tag: held.tag.clone(),
            owed: owed_after,
            dissolves_at: guild::dissolves_at(members),
        };
        let officers = officers_of(&mut tx, guild_id).await?;
        let mut notices = Vec::new();
        for officer in &officers {
            let id = notices::queue(&mut *tx, *officer, &guild_id.to_string(), &message).await?;
            notices.push((*officer, id, message.clone()));
        }
        Some(Told { guild_id, changed: officers, notices, paid: Vec::new(), ..Default::default() })
    } else {
        None
    };
    let flows = told.as_mut().map(|t| std::mem::take(&mut t.flows)).unwrap_or_default();
    super::ledger::commit(tx, flows).await?;
    Ok(told)
}

/// Why a guild closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    /// Its last member left
    Disbanded,
    /// Four weeks of dues unpaid
    Dissolved,
}

/// Closes `guild_id`, held in `conn`'s transaction. Its hall is given up
/// first (S4.3): its stations come down, their kits and contents going
/// home, and its deed is refunded into the treasury, less the tax it owes.
/// Then the treasury pays the dues it owes, what's left is shared among its
/// members by tenure ([`guild::share_by_tenure`]), the longest-serving first
/// on a tie, and its roster and invitations are cleared. On a dissolution
/// each member hears what they were given (`guild_dissolved`), queued for
/// anyone away, and anyone whose things from the hall went to their impound
/// or were lost hears that in theirs, on a disbanding too (#512). Commit with [`super::ledger::commit`] and the `Told`'s
/// flows, and give the game its halls ([`Told::release_halls`]).
async fn close(
    conn: &mut PgConnection,
    guild_id: Uuid,
    held: &Held,
    closing: Closing,
    now: DateTime<Utc>,
) -> Result<Told, GuildError> {
    let mut halls = Vec::new();
    let mut flows = Flows::new();
    let hall: Option<(Uuid, i32, i32, Option<String>)> =
        sqlx::query_as("SELECT id, assessed_value, tax_owed, terrain_id FROM plots WHERE guild_id = $1 FOR UPDATE")
            .bind(guild_id)
            .fetch_optional(&mut *conn)
            .await?;
    // What found no room went to its owner's impound, or was lost: each
    // owner hears it in their `guild_dissolved` (#512)
    let mut returned = ReturnedStations::default();
    if let Some((plot_id, assessed_value, tax_owed, terrain_id)) = hall {
        returned = return_plot_stations(conn, plot_id).await?;
        returned.count_into(&mut flows);
        let removed_stations = release_plot(conn, plot_id).await?;
        let refund = (assessed_value as i64 - tax_owed as i64).max(0);
        refund_hall(conn, guild_id, refund, "The hall's deed as the guild closed, less the tax it owed", now).await?;
        halls.push(HallReleased { terrain_id, removed_stations, returned_to: returned.containers.clone() });
    }
    let (treasury, owed): (i64, i64) = sqlx::query_as("SELECT treasury, dues_owed FROM guilds WHERE id = $1")
        .bind(guild_id)
        .fetch_one(&mut *conn)
        .await?;
    let pay = treasury.min(owed);
    let mut after = treasury;
    if pay > 0 {
        after = move_treasury(conn, guild_id, -pay).await?;
        book(conn, guild_id, None, GuildEntryKind::Dues, -pay, after, "Dues owed, as it closed", now).await?;
    }
    let tenure: Vec<(Uuid, DateTime<Utc>)> =
        sqlx::query_as("SELECT player_id, joined_at FROM guild_members WHERE guild_id = $1 ORDER BY joined_at, player_id")
            .bind(guild_id)
            .fetch_all(&mut *conn)
            .await?;
    let members: Vec<Uuid> = tenure.iter().map(|(m, _)| *m).collect();
    let tenures: Vec<i64> = tenure.iter().map(|(_, joined)| (now - *joined).num_seconds()).collect();
    let shares = guild::share_by_tenure(after, &tenures);
    let mut paid = Vec::new();
    for (member, share) in members.iter().zip(&shares) {
        if *share <= 0 {
            continue;
        }
        after = move_treasury(conn, guild_id, -share).await?;
        book(conn, guild_id, Some(*member), GuildEntryKind::Share, -share, after, "A share as it closed", now).await?;
        add_strands_in(
            conn,
            *member,
            *share,
            TransactionType::GuildWithdrawal,
            Some(&format!("A share of {}'s treasury, as it closed", held.name)),
            Some(guild_id),
        )
        .await
        .map_err(|e| GuildError::Db(sqlx::Error::Protocol(e)))?;
        paid.push(*member);
    }
    // On a dissolution every member hears it. Whoever had something of
    // theirs impounded or lost from the hall hears it too, member or not,
    // and on a disbanding as well (#512)
    let mut told: Vec<(Uuid, i64)> = Vec::new();
    if closing == Closing::Dissolved {
        told.extend(members.iter().copied().zip(shares.iter().copied()));
    }
    for (player, ..) in returned.impounded.iter().chain(&returned.lost) {
        if !told.iter().any(|(p, _)| p == player) {
            let share = members.iter().position(|m| m == player).map_or(0, |i| shares[i]);
            told.push((*player, share));
        }
    }
    let mut notices = Vec::new();
    for (player, share) in told {
        let message = ServerMessage::GuildDissolved {
            guild_id,
            guild: held.name.clone(),
            tag: held.tag.clone(),
            owed,
            share,
            impounded: returned.impounded_for(player),
            lost: returned.lost_for(player),
        };
        let id = notices::queue(&mut *conn, player, &guild_id.to_string(), &message).await?;
        notices.push((player, id, message));
    }
    sqlx::query("DELETE FROM guild_members WHERE guild_id = $1")
        .bind(guild_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM guild_invites WHERE guild_id = $1")
        .bind(guild_id)
        .execute(&mut *conn)
        .await?;
    let status = match closing {
        Closing::Disbanded => "disbanded",
        Closing::Dissolved => "dissolved",
    };
    sqlx::query("UPDATE guilds SET status = $2, disbanded_at = $3, dues_owed = dues_owed - $4 WHERE id = $1")
        .bind(guild_id)
        .bind(status)
        .bind(now)
        .bind(pay)
        .execute(&mut *conn)
        .await?;
    Ok(Told { guild_id, changed: members, notices, paid, halls, flows, ..Default::default() })
}

/// The leader `actor` claims the guild-district lot `plot_id` as `guild_id`'s
/// hall (S4.3), paying its deed (the zone's, at its live price) from the
/// treasury. The route has checked they stand at the Exchange hall. The
/// guild owns the lot, and its leader acts for it as its owner: the lead
/// takes the hall with it. A guild holds one hall. Its `station_use_level`
/// starts at `guild`, so the members use what the officers put there.
pub async fn claim_hall(
    db: &PgPool,
    actor: Uuid,
    guild_id: Uuid,
    plot_id: Uuid,
    tunables: &Tunables,
    now: DateTime<Utc>,
) -> Result<Told, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    let held = hold(&mut tx, guild_id).await?;
    let role = role_in(&mut tx, guild_id, actor).await?.ok_or(GuildError::NotAllowed("You aren't in this guild"))?;
    if role != GuildRole::Leader {
        return Err(GuildError::NotAllowed("Only the leader claims a hall for the guild"));
    }
    if hall_of(&mut tx, guild_id).await?.is_some() {
        return Err(GuildError::HasAHall);
    }
    let row: Option<(String, Option<Uuid>, Option<i32>, f32, f32)> = sqlx::query_as(
        "SELECT p.zone_id, p.owner_id, z.deed_cost, p.world_x, p.world_y
         FROM plots p JOIN zones z ON z.id = p.zone_id
         WHERE p.id = $1
         FOR UPDATE OF p",
    )
    .bind(plot_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (zone_id, owner, deed_cost, x, y) = row.ok_or(GuildError::NoLot)?;
    let in_the_district = zone_id == Zone::GuildDistrict.as_str() && Zone::from_position(x as f64, y as f64) == Zone::GuildDistrict;
    let deed_cost = deed_cost.filter(|_| in_the_district).ok_or(GuildError::NotAHallLot)?;
    if owner.is_some() {
        return Err(GuildError::LotTaken);
    }
    let cost = tunables.deed_cost(&zone_id, deed_cost);
    let treasury = treasury_of(&mut tx, guild_id).await?;
    if treasury < cost as i64 {
        return Err(GuildError::HallShort { cost: cost as i64, has: treasury });
    }
    let after = move_treasury(&mut tx, guild_id, -(cost as i64)).await?;
    book(&mut tx, guild_id, Some(actor), GuildEntryKind::Hall, -(cost as i64), after, "A hall in the guild district", now).await?;
    sqlx::query(
        "UPDATE plots SET owner_id = $2, guild_id = $3, claimed_at = $4, assessed_value = $5, granted = FALSE,
                tax_owed = 0, tax_carry = 0
         WHERE id = $1",
    )
    .bind(plot_id)
    .bind(actor)
    .bind(guild_id)
    .bind(now)
    .bind(cost)
    .execute(&mut *tx)
    .await?;
    gazette::record_deed(&mut tx, plot_id, &zone_id, DeedEvent::Hall(guild_id), actor).await?;
    sqlx::query(
        "INSERT INTO plot_permissions (plot_id, station_use_level) VALUES ($1, 'guild')
         ON CONFLICT (plot_id) DO UPDATE SET station_use_level = 'guild'",
    )
    .bind(plot_id)
    .execute(&mut *tx)
    .await?;
    let broadcast = hall_claimed(&mut tx, guild_id).await?.into_iter().collect();
    let changed = members_of(&mut tx, guild_id).await?;
    tx.commit().await?;
    tracing::info!("{} took a hall, {plot_id}, for {cost} Strands", held.name);
    Ok(Told { guild_id, changed, broadcast, ..Default::default() })
}

/// What an hour's tax on a hall did ([`hall_tax_hour`]).
#[derive(Debug, Default)]
pub struct HallHour {
    /// `tax_delinquent` for each officer and the leader, queued
    pub notices: Vec<TaxNotice>,
    /// Its debt has reached the threshold: foreclose it
    pub forecloses: bool,
}

/// An hour's tax on `guild_id`'s hall `plot_id` at `rate`, from its treasury
/// (S4.3), the way [`collect_dues`] takes the dues: the hour's tax and any
/// debt as far as the treasury reaches, so debt is paid off first as Strands
/// come in. What it can't pay is the plot's `tax_owed`. The officers and the
/// leader are warned as a plain plot's owner is
/// ([`notice_due`]), and at the threshold the caller forecloses it.
pub(crate) async fn hall_tax_hour(
    db: &PgPool,
    plot_id: Uuid,
    guild_id: Uuid,
    rate: f64,
    now: DateTime<Utc>,
) -> Result<HallHour, GuildError> {
    let now = now.trunc_subsecs(6);
    let mut tx = db.begin().await?;
    lock(&mut tx, guild_id).await?;
    let row: Option<(i32, i32, i32, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT assessed_value, tax_owed, tax_carry, terrain_id, guild_id FROM plots WHERE id = $1 FOR UPDATE",
    )
    .bind(plot_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((assessed_value, owed, carry, terrain_id, Some(hall_of))) = row else {
        return Ok(HallHour::default());
    };
    let weekly = weekly_tax(assessed_value, rate);
    if hall_of != guild_id || weekly <= 0 {
        return Ok(HallHour::default());
    }
    let (due, carry) = charge_hour(weekly, carry);
    let owed = owed as i64;
    let pay = treasury_of(&mut tx, guild_id).await?.min(owed + due);
    let owed_after = owed + due - pay;
    sqlx::query(
        "UPDATE plots SET tax_carry = $2, tax_owed = $3,
                last_tax_paid = CASE WHEN $4 THEN $5 ELSE last_tax_paid END
         WHERE id = $1",
    )
    .bind(plot_id)
    .bind(carry)
    .bind(owed_after as i32)
    .bind(pay > 0)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if pay > 0 {
        let after = move_treasury(&mut tx, guild_id, -pay).await?;
        let note = if pay > due { format!("Tax on the hall, and {} owed", pay - due) } else { "Tax on the hall".into() };
        book(&mut tx, guild_id, None, GuildEntryKind::Tax, -pay, after, &note, now).await?;
    }
    let mut notices = Vec::new();
    if let Some(terrain_id) = terrain_id.filter(|_| notice_due(owed, owed_after, assessed_value, rate)) {
        let forecloses_at = foreclosure_threshold(assessed_value, rate);
        let message = ServerMessage::TaxDelinquent { terrain_id: terrain_id.clone(), tax_owed: owed_after, forecloses_at };
        for officer in officers_of(&mut tx, guild_id).await? {
            let notice_id = Some(notices::queue(&mut *tx, officer, &terrain_id, &message).await?);
            notices.push(TaxNotice { owner_id: officer, terrain_id: terrain_id.clone(), tax_owed: owed_after, forecloses_at, notice_id });
        }
    }
    tx.commit().await?;
    Ok(HallHour { notices, forecloses: owed_after >= foreclosure_threshold(assessed_value, rate) })
}

/// Puts a hall's refunded deed, `strands`, into `guild_id`'s treasury, in
/// `conn`'s transaction (S4.3): as it's foreclosed, released by a terrain
/// change, or given up as the guild closes. Nothing for nothing.
pub(crate) async fn refund_hall(
    conn: &mut PgConnection,
    guild_id: Uuid,
    strands: i64,
    note: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    if strands <= 0 {
        return Ok(());
    }
    let after = move_treasury(conn, guild_id, strands).await?;
    book(conn, guild_id, None, GuildEntryKind::Refund, strands, after, note, now.trunc_subsecs(6)).await
}

/// Locks `guild_id`'s row for this transaction, whatever its status. A
/// hall's guild is always locked before its plot (S4.3).
pub(crate) async fn lock(conn: &mut PgConnection, guild_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1 FROM guilds WHERE id = $1 FOR UPDATE")
        .bind(guild_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// `guild_id`'s officers and leader, who hear of its hall's tax (S4.3).
pub(crate) async fn officers(conn: &mut PgConnection, guild_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    officers_of(conn, guild_id).await
}

/// `guild_id`'s hall, as its members see it.
async fn hall_of(conn: &mut PgConnection, guild_id: Uuid) -> Result<Option<GuildHall>, sqlx::Error> {
    let row: Option<(Uuid, Option<String>, f32, f32, Option<DateTime<Utc>>, i32)> = sqlx::query_as(
        "SELECT id, terrain_id, world_x, world_y, claimed_at, tax_owed FROM plots WHERE guild_id = $1",
    )
    .bind(guild_id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(plot_id, terrain_id, x, y, claimed_at, tax_owed)| GuildHall {
        plot_id,
        terrain_id,
        x: x as f64,
        y: y as f64,
        claimed_at: claimed_at.unwrap_or(DateTime::UNIX_EPOCH),
        tax_owed: tax_owed as i64,
    }))
}

/// Every guild's treasury: Strands held for guilds, outside every balance,
/// for the ledger's [`held_strands`](super::ledger::held_strands).
pub(crate) async fn held(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(SUM(treasury), 0)::BIGINT FROM guilds")
        .fetch_one(conn)
        .await
}

/// What's left the treasuries for no one, or come into them from no one,
/// ever, by transaction type, for the ledger's `held_flows`: dues
/// (`guild_dues`) and roster fees (`guild_fee`), and since S4.3 a hall's
/// deed (`deed_purchase`) and tax (`property_tax`), all negative, and its
/// deed refunded (`foreclosure_refund`, whether it was foreclosed, released
/// or given up), positive.
pub(crate) async fn held_flows(conn: &mut PgConnection) -> Result<std::collections::BTreeMap<String, i64>, sqlx::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT kind, SUM(amount)::BIGINT FROM guild_ledger WHERE kind IN ('dues', 'fee', 'hall', 'tax', 'refund') GROUP BY kind",
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(kind, sum)| {
            let kind = match kind.as_str() {
                "dues" => TransactionType::GuildDues,
                "fee" => TransactionType::GuildFee,
                "hall" => TransactionType::DeedPurchase,
                "tax" => TransactionType::PropertyTax,
                "refund" => TransactionType::ForeclosureRefund,
                _ => return None,
            };
            Some((kind.as_str().to_string(), sum))
        })
        .collect())
}

/// What `guild_id`'s officers may each withdraw in a rolling week.
async fn limit_of(conn: &mut PgConnection, guild_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT withdrawal_limit FROM guilds WHERE id = $1")
        .bind(guild_id)
        .fetch_one(conn)
        .await
}

/// What `player` has withdrawn from `guild_id` in the week to `now`.
async fn withdrawn_in_week(conn: &mut PgConnection, guild_id: Uuid, player: Uuid, now: DateTime<Utc>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(-SUM(amount), 0)::BIGINT FROM guild_ledger
         WHERE guild_id = $1 AND player_id = $2 AND kind = 'withdrawal' AND occurred_at > $3",
    )
    .bind(guild_id)
    .bind(player)
    .bind(now - Duration::days(guild::WITHDRAWAL_WINDOW_DAYS))
    .fetch_one(conn)
    .await
}

/// Every guild standing, the largest first.
pub async fn list(db: &PgPool) -> Result<Vec<GuildSummary>, sqlx::Error> {
    type Row = (Uuid, String, String, i64, DateTime<Utc>, Uuid, String, bool);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT g.id, g.name, g.tag, (SELECT COUNT(*) FROM guild_members m WHERE m.guild_id = g.id),
                g.founded_at, l.id, l.name, la.kind = 'citizen'
         FROM guilds g JOIN players l ON l.id = g.leader_id JOIN accounts la ON la.id = l.account_id
         WHERE g.status = 'active'
         ORDER BY 4 DESC, g.founded_at, g.id",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, tag, members, founded_at, leader, leader_name, citizen)| GuildSummary {
            id,
            name,
            tag,
            members: members as u32,
            leader: Correspondent { id: leader, name: leader_name, citizen, office: false },
            founded_at,
        })
        .collect())
}

/// The guild `guild_id` as `viewer` sees it: who's waiting only for its
/// officers and leader.
pub async fn detail(db: &PgPool, guild_id: Uuid, viewer: Option<Uuid>) -> Result<GuildDetail, GuildError> {
    detail_at(db, guild_id, viewer, Utc::now()).await
}

/// [`detail`] as of `now`, for the week an officer's withdrawals count in.
async fn detail_at(db: &PgPool, guild_id: Uuid, viewer: Option<Uuid>, now: DateTime<Utc>) -> Result<GuildDetail, GuildError> {
    let mut conn = db.acquire().await?;
    type Row = (String, String, String, DateTime<Utc>, Uuid, String, bool, Uuid, String, bool);
    let row: Option<Row> = sqlx::query_as(
        "SELECT g.name, g.tag, g.charter, g.founded_at,
                f.id, f.name, fa.kind = 'citizen', l.id, l.name, la.kind = 'citizen'
         FROM guilds g
         JOIN players f ON f.id = g.founder_id JOIN accounts fa ON fa.id = f.account_id
         JOIN players l ON l.id = g.leader_id JOIN accounts la ON la.id = l.account_id
         WHERE g.id = $1 AND g.status = 'active'",
    )
    .bind(guild_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (name, tag, charter, founded_at, f, f_name, f_citizen, l, l_name, l_citizen) = row.ok_or(GuildError::NoGuild)?;

    type MemberRow = (Uuid, String, bool, String, DateTime<Utc>);
    let rows: Vec<MemberRow> = sqlx::query_as(
        "SELECT p.id, p.name, a.kind = 'citizen', m.role, m.joined_at
         FROM guild_members m JOIN players p ON p.id = m.player_id JOIN accounts a ON a.id = p.account_id
         WHERE m.guild_id = $1
         ORDER BY CASE m.role WHEN 'leader' THEN 0 WHEN 'officer' THEN 1 ELSE 2 END, m.joined_at, p.id",
    )
    .bind(guild_id)
    .fetch_all(&mut *conn)
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.0).collect();
    let mut words = super::standing::words_of(&mut conn, &ids, Utc::now()).await?;
    let members: Vec<GuildMember> = rows
        .into_iter()
        .map(|(id, name, citizen, role, joined_at)| GuildMember {
            id,
            name,
            citizen,
            role: GuildRole::parse(&role).unwrap_or(GuildRole::Member),
            joined_at,
            standing: words.remove(&id).unwrap_or_else(|| standing::NEW_IN_TOWN.into()),
        })
        .collect();
    let sees_waiting = viewer.is_some_and(|v| members.iter().any(|m| m.id == v && m.role.may_invite()));
    let waiting = if sees_waiting { invites(&mut conn, "i.guild_id", guild_id).await? } else { Vec::new() };
    let member = viewer.and_then(|v| members.iter().find(|m| m.id == v));
    let treasury = match member {
        Some(m) => {
            let (strands, owed, withdrawal_limit): (i64, i64, i64) =
                sqlx::query_as("SELECT treasury, dues_owed, withdrawal_limit FROM guilds WHERE id = $1")
                    .bind(guild_id)
                    .fetch_one(&mut *conn)
                    .await?;
            let withdrawn_this_week = if m.role == GuildRole::Officer {
                withdrawn_in_week(&mut conn, guild_id, m.id, now).await?
            } else {
                0
            };
            let size = members.len() as u32;
            Some(GuildTreasury {
                strands,
                owed,
                weekly_dues: guild::weekly_dues(size),
                dissolves_at: guild::dissolves_at(size),
                withdrawal_limit,
                withdrawn_this_week,
            })
        }
        None => None,
    };
    // Where its hall stands is anyone's to see; what it owes, its members'
    let hall = hall_of(&mut conn, guild_id).await?.map(|h| GuildHall {
        tax_owed: if member.is_some() { h.tax_owed } else { 0 },
        ..h
    });
    Ok(GuildDetail {
        id: guild_id,
        name,
        tag,
        charter,
        founded_at,
        founder: Correspondent { id: f, name: f_name, citizen: f_citizen, office: false },
        leader: Correspondent { id: l, name: l_name, citizen: l_citizen, office: false },
        members,
        waiting,
        treasury,
        hall,
    })
}

/// `player`'s guild, their role in it, and the invitations and
/// applications naming them.
pub async fn mine(db: &PgPool, player: Uuid) -> Result<MyGuild, GuildError> {
    let mut conn = db.acquire().await?;
    let invites = invites(&mut conn, "i.player_id", player).await?;
    let Some(guild_id) = guild_of(&mut conn, player).await? else {
        return Ok(MyGuild { guild: None, role: None, invites });
    };
    let role = role_in(&mut conn, guild_id, player).await?;
    drop(conn);
    Ok(MyGuild { guild: Some(detail(db, guild_id, Some(player)).await?), role, invites })
}

/// The guild `player` is in now (S4.4): the guild channel's reach.
pub async fn guild_of_player(db: &PgPool, player: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    guild_of(&mut *db.acquire().await?, player).await
}

/// Each member's guild tag (S4.4), for the names over their heads: of
/// everyone in a guild, or only of `players`. Someone in no guild isn't in
/// it.
pub async fn tags(db: &PgPool, players: Option<&[Uuid]>) -> Result<HashMap<Uuid, String>, sqlx::Error> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT m.player_id, g.tag FROM guild_members m JOIN guilds g ON g.id = m.guild_id
         WHERE $1::uuid[] IS NULL OR m.player_id = ANY($1)",
    )
    .bind(players.map(<[Uuid]>::to_vec))
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Reads `players`' tags again into the game, after a change to their
/// guild, so the next `positions` carries them. A failed read leaves the
/// tags as they were.
pub async fn retag(db: &PgPool, game: &tokio::sync::RwLock<super::GameState>, players: &[Uuid]) {
    match tags(db, Some(players)).await {
        Ok(now) => {
            let mut game = game.write().await;
            for player in players {
                match now.get(player) {
                    Some(tag) => game.guild_tags.insert(*player, tag.clone()),
                    None => game.guild_tags.remove(player),
                };
            }
        }
        Err(e) => tracing::warn!("Couldn't read the guild tags of {} players: {e}", players.len()),
    }
}

/// The guild standing at `guild_id`, locked for this transaction.
async fn hold(conn: &mut PgConnection, guild_id: Uuid) -> Result<Held, GuildError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT name, tag FROM guilds WHERE id = $1 AND status = 'active' FOR UPDATE")
            .bind(guild_id)
            .fetch_optional(conn)
            .await?;
    let (name, tag) = row.ok_or(GuildError::NoGuild)?;
    Ok(Held { name, tag })
}

/// The guild `player` is in.
async fn guild_of(conn: &mut PgConnection, player: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE player_id = $1")
        .bind(player)
        .fetch_optional(conn)
        .await
}

/// `player`'s role in `guild_id`, if they're in it.
async fn role_in(conn: &mut PgConnection, guild_id: Uuid, player: Uuid) -> Result<Option<GuildRole>, sqlx::Error> {
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM guild_members WHERE guild_id = $1 AND player_id = $2")
        .bind(guild_id)
        .bind(player)
        .fetch_optional(conn)
        .await?;
    Ok(role.as_deref().and_then(GuildRole::parse))
}

/// Everyone in `guild_id`.
async fn members_of(conn: &mut PgConnection, guild_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT player_id FROM guild_members WHERE guild_id = $1 ORDER BY joined_at, player_id")
        .bind(guild_id)
        .fetch_all(conn)
        .await
}

/// `guild_id`'s officers and leader: those who hear of applications.
async fn officers_of(conn: &mut PgConnection, guild_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT player_id FROM guild_members WHERE guild_id = $1 AND role IN ('leader', 'officer')
         ORDER BY joined_at, player_id",
    )
    .bind(guild_id)
    .fetch_all(conn)
    .await
}

/// `player` joins `guild_id` as `role`: anything else they'd asked or been
/// asked lapses. The members table's key keeps them to one guild.
async fn join(conn: &mut PgConnection, guild_id: Uuid, player: Uuid, role: GuildRole, now: DateTime<Utc>) -> Result<(), GuildError> {
    sqlx::query("INSERT INTO guild_members (player_id, guild_id, role, joined_at) VALUES ($1, $2, $3, $4)")
        .bind(player)
        .bind(guild_id)
        .bind(role.as_str())
        .bind(now)
        .execute(&mut *conn)
        .await
        .map_err(|e| match unique_violation(&e) {
            Some("guild_members_pkey") => GuildError::TheyreInAGuild,
            _ => GuildError::Db(e),
        })?;
    sqlx::query("DELETE FROM guild_invites WHERE player_id = $1")
        .bind(player)
        .execute(conn)
        .await?;
    Ok(())
}

/// The lead of `guild_id` goes from `from` to `to`; `from` becomes an
/// officer. The guild's hall goes with it, since its leader acts for it
/// (S4.3): its `plot_claimed`, naming the new leader, is returned to tell
/// everyone.
async fn hand_on(
    conn: &mut PgConnection,
    guild_id: Uuid,
    from: Uuid,
    to: Uuid,
    now: DateTime<Utc>,
) -> Result<Option<GameEvent>, sqlx::Error> {
    // The old leader steps down first: one leader a guild, by an index
    sqlx::query("UPDATE guild_members SET role = 'officer' WHERE player_id = $1 AND guild_id = $2")
        .bind(from)
        .bind(guild_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE guild_members SET role = 'leader' WHERE player_id = $1 AND guild_id = $2")
        .bind(to)
        .bind(guild_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE guilds SET leader_id = $2, leader_since = $3 WHERE id = $1")
        .bind(guild_id)
        .bind(to)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE plots SET owner_id = $2 WHERE guild_id = $1")
        .bind(guild_id)
        .bind(to)
        .execute(&mut *conn)
        .await?;
    hall_claimed(conn, guild_id).await
}

/// `plot_claimed` for `guild_id`'s hall as it stands, naming the guild, if
/// it holds one on a lot (S4.3).
async fn hall_claimed(conn: &mut PgConnection, guild_id: Uuid) -> Result<Option<GameEvent>, sqlx::Error> {
    let row: Option<(String, Uuid, DateTime<Utc>, String, String)> = sqlx::query_as(
        "SELECT p.terrain_id, p.owner_id, p.claimed_at, g.name, g.tag
         FROM plots p JOIN guilds g ON g.id = p.guild_id
         WHERE p.guild_id = $1 AND p.terrain_id IS NOT NULL AND p.owner_id IS NOT NULL AND p.claimed_at IS NOT NULL",
    )
    .bind(guild_id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(terrain_id, owner_id, claimed_at, name, tag)| GameEvent::PlotClaimed {
        terrain_id,
        owner_id,
        claimed_at,
        guild: Some(PlotGuild { id: guild_id, name, tag }),
    }))
}

/// What waits between `guild_id` and `player`, if anything.
async fn waiting(conn: &mut PgConnection, guild_id: Uuid, player: Uuid) -> Result<Option<GuildInviteKind>, sqlx::Error> {
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM guild_invites WHERE guild_id = $1 AND player_id = $2")
        .bind(guild_id)
        .bind(player)
        .fetch_optional(conn)
        .await?;
    Ok(kind.as_deref().and_then(GuildInviteKind::parse))
}

async fn ask(
    conn: &mut PgConnection,
    guild_id: Uuid,
    player: Uuid,
    by: Uuid,
    kind: GuildInviteKind,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO guild_invites (guild_id, player_id, by_id, kind, created_at) VALUES ($1, $2, $3, $4, $5)")
        .bind(guild_id)
        .bind(player)
        .bind(by)
        .bind(kind.as_str())
        .bind(now)
        .execute(conn)
        .await?;
    Ok(())
}

/// The invitations and applications whose `column` (`i.guild_id` or
/// `i.player_id`) is `id`, oldest first.
async fn invites(conn: &mut PgConnection, column: &str, id: Uuid) -> Result<Vec<GuildInvite>, sqlx::Error> {
    type Row = (Uuid, String, String, String, Uuid, String, bool, Uuid, String, bool, DateTime<Utc>);
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT g.id, g.name, g.tag, i.kind, p.id, p.name, pa.kind = 'citizen', b.id, b.name, ba.kind = 'citizen', i.created_at
         FROM guild_invites i JOIN guilds g ON g.id = i.guild_id
         JOIN players p ON p.id = i.player_id JOIN accounts pa ON pa.id = p.account_id
         JOIN players b ON b.id = i.by_id JOIN accounts ba ON ba.id = b.account_id
         WHERE {column} = $1 AND g.status = 'active'
         ORDER BY i.created_at, p.id"
    ))
    .bind(id)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(guild_id, guild, tag, kind, p, p_name, p_citizen, b, b_name, b_citizen, created_at)| {
            Some(GuildInvite {
                guild_id,
                guild,
                tag,
                kind: GuildInviteKind::parse(&kind)?,
                player: Correspondent { id: p, name: p_name, citizen: p_citizen, office: false },
                by: Correspondent { id: b, name: b_name, citizen: b_citizen, office: false },
                created_at,
            })
        })
        .collect())
}

/// Takes [`guild::ROSTER_FEE`] for a change to `guild_id`'s roster, which
/// `actor` makes: from the treasury if it has it, else from them. Whose
/// balance it changed: theirs, or no one's.
async fn charge(
    conn: &mut PgConnection,
    actor: Uuid,
    guild_id: Uuid,
    what: &str,
    now: DateTime<Utc>,
) -> Result<Vec<Uuid>, GuildError> {
    if treasury_of(conn, guild_id).await? >= guild::ROSTER_FEE {
        let after = move_treasury(conn, guild_id, -guild::ROSTER_FEE).await?;
        book(conn, guild_id, Some(actor), GuildEntryKind::Fee, -guild::ROSTER_FEE, after, what, now).await?;
        return Ok(Vec::new());
    }
    subtract_strands_in(conn, actor, guild::ROSTER_FEE, TransactionType::GuildFee, Some(what), Some(guild_id))
        .await
        .map_err(|e| fee_error(e, GuildError::RosterFee))?;
    Ok(vec![actor])
}

/// The Strands in `guild_id`'s treasury.
async fn treasury_of(conn: &mut PgConnection, guild_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT treasury FROM guilds WHERE id = $1")
        .bind(guild_id)
        .fetch_one(conn)
        .await
}

/// Moves `guild_id`'s treasury by `strands`, and returns it after.
async fn move_treasury(conn: &mut PgConnection, guild_id: Uuid, strands: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("UPDATE guilds SET treasury = treasury + $2 WHERE id = $1 RETURNING treasury")
        .bind(guild_id)
        .bind(strands)
        .fetch_one(conn)
        .await
}

/// Writes a move of `guild_id`'s treasury in its books.
#[allow(clippy::too_many_arguments)]
async fn book(
    conn: &mut PgConnection,
    guild_id: Uuid,
    by: Option<Uuid>,
    kind: GuildEntryKind,
    strands: i64,
    after: i64,
    note: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO guild_ledger (guild_id, player_id, kind, amount, treasury_after, note, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(guild_id)
    .bind(by)
    .bind(kind.as_str())
    .bind(strands)
    .bind(after)
    .bind(note)
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}

/// Queues `to` a notice of `kind` about `held`, done by `who`.
async fn notify(
    conn: &mut PgConnection,
    to: Uuid,
    guild_id: Uuid,
    held: &Held,
    kind: GuildNoticeKind,
    who: &Correspondent,
) -> Result<(Uuid, Uuid, ServerMessage), sqlx::Error> {
    let message = ServerMessage::GuildNotice {
        guild_id,
        guild: held.name.clone(),
        tag: held.tag.clone(),
        kind,
        who: who.id,
        name: who.name.clone(),
        citizen: who.citizen,
    };
    let id = notices::queue(conn, to, &guild_id.to_string(), &message).await?;
    Ok((to, id, message))
}

/// Who `player` is, or `None` if there's no such player.
async fn correspondent(conn: &mut PgConnection, player: Uuid) -> Result<Option<Correspondent>, sqlx::Error> {
    let row: Option<(String, bool)> = sqlx::query_as(
        "SELECT p.name, a.kind = 'citizen' FROM players p JOIN accounts a ON a.id = p.account_id WHERE p.id = $1",
    )
    .bind(player)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(name, citizen)| Correspondent { id: player, name, citizen, office: false }))
}

/// `player`'s account's subscription, as its row has it.
async fn subscription_of(conn: &mut PgConnection, player: Uuid) -> Result<Option<Subscription>, sqlx::Error> {
    let row: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT a.subscription_status, a.subscription_until
         FROM players p JOIN accounts a ON a.id = p.account_id WHERE p.id = $1",
    )
    .bind(player)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(status, until)| Subscription { status: SubscriptionStatus::parse(&status), until }))
}

/// The unique index a write broke, if that's why it failed.
fn unique_violation(e: &sqlx::Error) -> Option<&str> {
    match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => db.constraint(),
        _ => None,
    }
}

/// A fee `subtract_strands_in` refused: `short` if it was the balance.
fn fee_error(e: String, short: GuildError) -> GuildError {
    if e.starts_with("Insufficient") {
        short
    } else {
        GuildError::Db(sqlx::Error::Protocol(e))
    }
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test guilds -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::conservation::{assert_audited, balances_of, strands_accounted};
    use crate::engine::plot_sync::tests::player;
    use capitol_protocol::ledger::StrandFlow;

    fn at(days: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap() + Duration::days(days)
    }

    /// A player with `balance` Strands and a subscription running from
    /// `at(0)` for `days`
    async fn subscriber(db: &PgPool, name: &str, balance: i64, days: i64) -> Uuid {
        let id = player(db, name, balance).await;
        sqlx::query(
            "UPDATE accounts SET subscription_status = 'active', subscription_until = $2
             WHERE id = (SELECT account_id FROM players WHERE id = $1)",
        )
        .bind(id)
        .bind(at(days))
        .execute(db)
        .await
        .unwrap();
        id
    }

    /// A citizen: an account's kind is fixed from the first
    async fn citizen(db: &PgPool, name: &str, balance: i64) -> Uuid {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash, kind) VALUES ($1, $2, 'x', 'citizen')")
            .bind(account)
            .bind(format!("{name}@town"))
            .execute(db)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name, strand_balance) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(account)
            .bind(name)
            .bind(balance)
            .execute(db)
            .await
            .unwrap();
        id
    }

    async fn fees_paid(db: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0)::BIGINT FROM currency_transactions WHERE transaction_type = 'guild_fee'")
            .fetch_one(db)
            .await
            .unwrap()
    }

    async fn rope_walkers(db: &PgPool, leader: Uuid) -> Uuid {
        found(db, leader, "The Rope Walkers", "rope", "We make rope.", at(0)).await.unwrap().0
    }

    async fn role(db: &PgPool, guild_id: Uuid, player: Uuid) -> Option<GuildRole> {
        role_in(&mut db.acquire().await.unwrap(), guild_id, player).await.unwrap()
    }

    /// `leader`'s guild takes `player` in as a member: invited, then accepting
    async fn take_in(db: &PgPool, guild_id: Uuid, leader: Uuid, player: Uuid, day: i64) {
        invite(db, leader, guild_id, player, at(day)).await.unwrap();
        apply(db, player, guild_id, at(day)).await.unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_tags_and_the_channel_follow_the_roster(db: PgPool) {
        let ada = subscriber(&db, "Ada", 1000, 30).await;
        let bram = player(&db, "Bram", 10).await;
        let id = rope_walkers(&db, ada).await;
        take_in(&db, id, ada, bram, 1).await;
        let all = tags(&db, None).await.unwrap();
        assert_eq!((all.get(&ada).map(String::as_str), all.get(&bram).map(String::as_str)), (Some("ROPE"), Some("ROPE")));
        assert_eq!(guild_of_player(&db, bram).await.unwrap(), Some(id));

        // Into the game: Bram leaves, and only his tag goes
        let terrain = crate::models::TerrainConfig::get_default();
        let game = tokio::sync::RwLock::new(crate::engine::GameState::with_terrain(crate::models::WorldGenParams::default(), terrain));
        game.write().await.guild_tags = all;
        let told = leave(&db, bram, id, at(2)).await.unwrap();
        retag(&db, &game, &told.changed).await;
        let now = game.read().await.guild_tags.clone();
        assert_eq!(now.get(&ada).map(String::as_str), Some("ROPE"));
        assert!(!now.contains_key(&bram));
        assert_eq!(guild_of_player(&db, bram).await.unwrap(), None);
        assert!(tags(&db, Some(&[bram])).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_free_player_or_a_citizen_cant_found(db: PgPool) {
        let ada = player(&db, "Ada", 1000).await;
        let refused = found(&db, ada, "The Rope Walkers", "ROPE", "", at(0)).await.unwrap_err();
        assert!(matches!(&refused, GuildError::Unsubscribed(m) if m == "This needs a subscription."), "{refused:?}");
        let brin = citizen(&db, "Brin", 1000).await;
        assert!(matches!(found(&db, brin, "Brin's Own", "BRIN", "", at(0)).await, Err(GuildError::Unsubscribed(_))));
        // Lapsed, it's refused with when
        let cy = subscriber(&db, "Cy", 1000, 1).await;
        let refused = found(&db, cy, "Cy's Own", "CY", "", at(2)).await.unwrap_err();
        assert!(matches!(&refused, GuildError::Unsubscribed(m) if m.contains("lapsed on")), "{refused:?}");
        assert_eq!(fees_paid(&db).await, 0);
        assert!(list(&db).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn founding_charges_500_sunk_and_audited(db: PgPool) {
        let ada = subscriber(&db, "Ada", 600, 30).await;
        let bea = subscriber(&db, "Bea", guild::FOUNDING_FEE - 1, 30).await;
        let start = balances_of(&db, [ada, bea]).await;
        let total = strands_accounted(&db).await;

        let (id, told) = found(&db, ada, "  The Rope   Walkers ", "rope", "We make rope.", at(0)).await.unwrap();
        assert_eq!((told.changed, told.paid), (vec![ada], vec![ada]));
        assert!(matches!(found(&db, bea, "Bea's Own", "BEA", "", at(0)).await, Err(GuildError::Founding)));

        assert_eq!(balances_of(&db, [ada]).await[&ada], 100);
        assert_eq!(fees_paid(&db).await, -guild::FOUNDING_FEE);
        assert_eq!(StrandFlow::of(TransactionType::GuildFee.as_str()), StrandFlow::Sink);
        assert_eq!(strands_accounted(&db).await, total - guild::FOUNDING_FEE, "sunk, not held");
        assert_audited(&db, &start).await;

        let detail = detail(&db, id, None).await.unwrap();
        assert_eq!((detail.name.as_str(), detail.tag.as_str(), detail.charter.as_str()), ("The Rope Walkers", "ROPE", "We make rope."));
        assert_eq!((detail.founder.id, detail.leader.id), (ada, ada));
        assert_eq!(detail.members.len(), 1);
        assert_eq!(detail.members[0].role, GuildRole::Leader);
        assert_eq!(list(&db).await.unwrap().len(), 1, "Bea's never stood");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn names_and_tags_are_unique_and_the_citys_are_reserved(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 30).await;
        let bea = subscriber(&db, "Bea", 5000, 30).await;
        let ropers = rope_walkers(&db, ada).await;
        assert!(matches!(found(&db, bea, "the rope walkers", "RW", "", at(0)).await, Err(GuildError::NameTaken)));
        assert!(matches!(found(&db, bea, "Rope Makers", "Rope", "", at(0)).await, Err(GuildError::TagTaken)));
        assert!(matches!(
            found(&db, bea, "Capitol Weavers", "WEAVE", "", at(0)).await,
            Err(GuildError::Name(NameError::Reserved(w))) if w == "capitol"
        ));
        assert!(matches!(found(&db, bea, "Weavers", "GAZ", "", at(0)).await, Err(GuildError::Name(NameError::ReservedTag))));
        assert_eq!(fees_paid(&db).await, -guild::FOUNDING_FEE, "refusals cost nothing");

        // Disbanded, its name and tag are free again
        leave(&db, ada, ropers, at(1)).await.unwrap();
        assert!(list(&db).await.unwrap().is_empty());
        found(&db, bea, "The Rope Walkers", "ROPE", "", at(2)).await.unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn one_guild_per_player(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 30).await;
        let bea = subscriber(&db, "Bea", 5000, 30).await;
        let cy = player(&db, "Cy", 10).await;
        let ropers = rope_walkers(&db, ada).await;
        let weavers = found(&db, bea, "The Weavers", "WEAVE", "", at(0)).await.unwrap().0;
        assert!(matches!(found(&db, ada, "Another", "ANOT", "", at(0)).await, Err(GuildError::InAGuild)));
        assert!(matches!(apply(&db, ada, weavers, at(0)).await, Err(GuildError::InAGuild)));
        assert!(matches!(invite(&db, bea, weavers, ada, at(0)).await, Err(GuildError::TheyreInAGuild)));

        // Cy is asked by both, and joins one: the other's invitation lapses
        invite(&db, ada, ropers, cy, at(0)).await.unwrap();
        invite(&db, bea, weavers, cy, at(0)).await.unwrap();
        assert_eq!(mine(&db, cy).await.unwrap().invites.len(), 2);
        apply(&db, cy, ropers, at(1)).await.unwrap();
        let cys = mine(&db, cy).await.unwrap();
        assert_eq!((cys.guild.map(|g| g.id), cys.role, cys.invites.len()), (Some(ropers), Some(GuildRole::Member), 0));
        assert!(matches!(apply(&db, cy, weavers, at(1)).await, Err(GuildError::InAGuild)));
        // Their card says so (S0.3)
        use crate::routes::players::{card, Who};
        assert_eq!(card(&db, &Default::default(), Uuid::nil(), Who::Id(cy)).await.unwrap().guild.as_deref(), Some("The Rope Walkers"));
        assert_eq!(card(&db, &Default::default(), Uuid::nil(), Who::Id(ada)).await.unwrap().guild.as_deref(), Some("The Rope Walkers"));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn invitations_and_applications_meet_and_tell_people(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 30).await;
        let bea = player(&db, "Bea", 10).await;
        let brin = citizen(&db, "Brin", 10).await;
        let ropers = rope_walkers(&db, ada).await;

        // Bea is invited, and hears of it while away
        let told = invite(&db, ada, ropers, bea, at(1)).await.unwrap();
        assert_eq!((told.paid, told.changed.clone()), (Vec::new(), vec![ada]), "an invitation costs nothing");
        let waiting = notices::take_all(&db, bea, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap();
        assert!(matches!(&waiting[..], [ServerMessage::GuildNotice { kind: GuildNoticeKind::Invited, who, tag, .. }] if *who == ada && tag == "ROPE"));
        assert!(matches!(invite(&db, ada, ropers, bea, at(1)).await, Err(GuildError::AlreadyWaiting)));
        let as_leader = detail(&db, ropers, Some(ada)).await.unwrap();
        assert_eq!(as_leader.waiting.len(), 1);
        assert!(detail(&db, ropers, Some(bea)).await.unwrap().waiting.is_empty(), "only officers see who's waiting");

        // She accepts by applying, and pays the roster's Strand
        let told = apply(&db, bea, ropers, at(2)).await.unwrap();
        assert_eq!((told.paid, told.changed), (vec![bea], vec![ada, bea]));
        assert_eq!(balances_of(&db, [bea]).await[&bea], 10 - guild::ROSTER_FEE);

        // Brin applies; the leader hears, and inviting him takes him in
        let told = apply(&db, brin, ropers, at(3)).await.unwrap();
        assert!(matches!(&told.notices[..], [(to, _, ServerMessage::GuildNotice { kind: GuildNoticeKind::Applied, citizen: true, .. })] if *to == ada));
        invite(&db, ada, ropers, brin, at(4)).await.unwrap();
        assert_eq!(role(&db, ropers, brin).await, Some(GuildRole::Member));
        let members: Vec<Uuid> = detail(&db, ropers, None).await.unwrap().members.iter().map(|m| m.id).collect();
        assert_eq!(members, vec![ada, bea, brin], "the leader, then by tenure");

        // Withdrawn and declined
        let cy = player(&db, "Cy", 10).await;
        invite(&db, ada, ropers, cy, at(5)).await.unwrap();
        withdraw(&db, cy, ropers, cy).await.unwrap();
        assert!(matches!(withdraw(&db, cy, ropers, cy).await, Err(GuildError::NoInvite)));
        apply(&db, cy, ropers, at(5)).await.unwrap();
        assert!(matches!(withdraw(&db, bea, ropers, cy).await, Err(GuildError::NotAllowed(_))), "a member can't turn someone down");
        withdraw(&db, ada, ropers, cy).await.unwrap();
        assert!(mine(&db, cy).await.unwrap().invites.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn each_role_has_its_rights(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 30).await;
        let bea = subscriber(&db, "Bea", 10, 30).await;
        let cy = player(&db, "Cy", 10).await;
        let dee = player(&db, "Dee", 10).await;
        let eve = player(&db, "Eve", 0).await;
        let ropers = rope_walkers(&db, ada).await;
        for (n, p) in [bea, cy, dee].into_iter().enumerate() {
            take_in(&db, ropers, ada, p, n as i64 + 1).await;
        }

        // A member can't invite, expel or set roles
        assert!(matches!(invite(&db, cy, ropers, eve, at(5)).await, Err(GuildError::NotAllowed(_))));
        assert!(matches!(expel(&db, cy, ropers, dee, at(6)).await, Err(GuildError::NotAllowed(_))));
        assert!(matches!(set_role(&db, cy, ropers, dee, GuildRole::Officer, at(5)).await, Err(GuildError::NotAllowed(_))));

        // The leader makes Bea an officer, for a Strand
        let before = balances_of(&db, [ada]).await[&ada];
        set_role(&db, ada, ropers, bea, GuildRole::Officer, at(5)).await.unwrap();
        assert_eq!(balances_of(&db, [ada]).await[&ada], before - guild::ROSTER_FEE);
        assert!(set_role(&db, ada, ropers, bea, GuildRole::Officer, at(5)).await.unwrap().paid.is_empty(), "no change, no fee");

        // An officer invites and expels members, but not officers or the leader, nor sets roles
        invite(&db, bea, ropers, eve, at(5)).await.unwrap();
        assert!(matches!(set_role(&db, bea, ropers, cy, GuildRole::Officer, at(5)).await, Err(GuildError::NotAllowed(_))));
        assert!(matches!(expel(&db, bea, ropers, ada, at(6)).await, Err(GuildError::NotAllowed(_))));
        let told = expel(&db, bea, ropers, dee, at(6)).await.unwrap();
        assert!(told.changed.contains(&dee), "the expelled hear it too");
        assert!(matches!(&told.notices[..], [(to, _, ServerMessage::GuildNotice { kind: GuildNoticeKind::Expelled, .. })] if *to == dee));
        assert_eq!(role(&db, ropers, dee).await, None);

        // A roster change needs the Strand: Eve has none to join with
        assert!(matches!(apply(&db, eve, ropers, at(6)).await, Err(GuildError::RosterFee)));
        assert_eq!(role(&db, ropers, eve).await, None);

        // The leader expels an officer, after a promotion
        set_role(&db, ada, ropers, cy, GuildRole::Officer, at(6)).await.unwrap();
        assert!(matches!(expel(&db, bea, ropers, cy, at(6)).await, Err(GuildError::NotAllowed(m)) if m.contains("leader")));
        expel(&db, ada, ropers, cy, at(6)).await.unwrap();

        // The lead goes only to a subscriber; then Ada is an officer
        take_in(&db, ropers, ada, dee, 7).await;
        assert!(matches!(set_role(&db, ada, ropers, dee, GuildRole::Leader, at(7)).await, Err(GuildError::Unsubscribed(_))));
        assert!(matches!(leave(&db, ada, ropers, at(7)).await, Err(GuildError::LeaderLeaving)));
        let told = set_role(&db, ada, ropers, bea, GuildRole::Leader, at(7)).await.unwrap();
        assert!(matches!(&told.notices[..], [(to, _, ServerMessage::GuildNotice { kind: GuildNoticeKind::Lead, .. })] if *to == bea));
        assert_eq!((role(&db, ropers, bea).await, role(&db, ropers, ada).await), (Some(GuildRole::Leader), Some(GuildRole::Officer)));
        assert_eq!(detail(&db, ropers, None).await.unwrap().leader.id, bea);

        // Anyone else leaves, for nothing
        let before = balances_of(&db, [dee]).await[&dee];
        leave(&db, dee, ropers, at(8)).await.unwrap();
        assert_eq!(balances_of(&db, [dee]).await[&dee], before);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_lapsed_leader_cant_invite_and_after_four_weeks_the_lead_passes(db: PgPool) {
        // Ada's subscription ends on day 10
        let ada = subscriber(&db, "Ada", 5000, 10).await;
        let bea = player(&db, "Bea", 10).await;
        let cy = player(&db, "Cy", 10).await;
        let dee = player(&db, "Dee", 10).await;
        let ropers = rope_walkers(&db, ada).await;
        take_in(&db, ropers, ada, bea, 1).await;
        take_in(&db, ropers, ada, cy, 2).await;
        set_role(&db, ada, ropers, cy, GuildRole::Officer, at(3)).await.unwrap();

        // Lapsed, Ada can't invite; her officer still can
        let refused = invite(&db, ada, ropers, dee, at(11)).await.unwrap_err();
        assert!(matches!(&refused, GuildError::Unsubscribed(m) if m.starts_with("A leader needs a subscription")), "{refused:?}");
        invite(&db, cy, ropers, dee, at(11)).await.unwrap();

        // Not yet at 27 days lapsed
        assert!(pass_lapsed_leads(&db, at(10 + 27)).await.is_empty());
        // At 28 it goes to the officer, Cy, though Bea has served longer
        let told = pass_lapsed_leads(&db, at(10 + 28)).await;
        assert_eq!(told.len(), 1);
        assert!(matches!(&told[0].notices[..], [(to, _, ServerMessage::GuildNotice { kind: GuildNoticeKind::Lead, who, .. })] if *to == cy && *who == ada));
        assert_eq!((role(&db, ropers, cy).await, role(&db, ropers, ada).await), (Some(GuildRole::Leader), Some(GuildRole::Officer)));
        assert!(pass_lapsed_leads(&db, at(10 + 29)).await.is_empty(), "Cy's four weeks start now");

        // Cy never subscribed: four weeks on, it passes to the longest-serving
        // officer, Ada, who still hasn't
        let told = pass_lapsed_leads(&db, at(10 + 28 + 28)).await;
        assert_eq!(told.len(), 1);
        assert_eq!(role(&db, ropers, ada).await, Some(GuildRole::Leader));
    }

    /// `hours` of dues from `from`, an hour apart
    async fn dues_for(db: &PgPool, from: DateTime<Utc>, hours: i64) -> Vec<Told> {
        let mut told = Vec::new();
        for h in 0..hours {
            told.extend(collect_dues(db, from + Duration::hours(h)).await);
        }
        told
    }

    /// `guild_id`'s treasury and what it owes
    async fn treasury(db: &PgPool, guild_id: Uuid) -> (i64, i64) {
        sqlx::query_as("SELECT treasury, dues_owed FROM guilds WHERE id = $1")
            .bind(guild_id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_week_of_dues_costs_exactly_fifty_and_five_a_member(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 30).await;
        let bea = player(&db, "Bea", 10).await;
        let cy = player(&db, "Cy", 10).await;
        let ropers = rope_walkers(&db, ada).await;
        take_in(&db, ropers, ada, bea, 0).await;
        take_in(&db, ropers, ada, cy, 0).await;
        let told = deposit(&db, ada, ropers, 1000, at(1)).await.unwrap();
        assert_eq!((told.paid, told.changed.len()), (vec![ada], 3));
        let before = balances_of(&db, [ada, bea, cy]).await;
        let total = strands_accounted(&db).await;

        // An hour charges a Strand of 65 now and then, carrying the rest
        assert!(dues_for(&db, at(1), 2).await.is_empty(), "nothing to tell");
        assert_eq!(treasury(&db, ropers).await, (1000, 0), "65/168 of a Strand twice is less than one");
        dues_for(&db, at(1) + Duration::hours(2), 166).await;
        assert_eq!(treasury(&db, ropers).await, (1000 - guild::weekly_dues(3), 0), "a week is exactly 65");
        dues_for(&db, at(8), 168).await;
        assert_eq!(treasury(&db, ropers).await.0, 1000 - 2 * guild::weekly_dues(3));
        // Sunk from the holding, written in its books, the members untouched
        assert_eq!(strands_accounted(&db).await, total);
        assert_eq!(balances_of(&db, [ada, bea, cy]).await, before);
        let entries = books(&db, ropers, cy).await.unwrap().entries;
        let dues: Vec<&GuildEntry> = entries.iter().filter(|e| e.kind == GuildEntryKind::Dues).collect();
        assert_eq!(dues.len(), 130);
        assert!(dues.iter().all(|e| e.strands == -1 && e.by.is_none()));
        let outsider = player(&db, "Outsider", 0).await;
        assert!(matches!(books(&db, ropers, outsider).await, Err(GuildError::NotInAGuild)), "only members read them");
        let flows = held_flows(&mut db.acquire().await.unwrap()).await.unwrap();
        assert_eq!(flows["guild_dues"], -130);
        let seen = detail(&db, ropers, Some(bea)).await.unwrap().treasury.unwrap();
        assert_eq!((seen.strands, seen.weekly_dues, seen.dissolves_at), (870, 65, 260));
        assert!(detail(&db, ropers, None).await.unwrap().treasury.is_none(), "only members see it");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn officers_withdraw_within_the_leaders_weekly_limit(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 30).await;
        let bea = player(&db, "Bea", 10).await;
        let cy = player(&db, "Cy", 301).await;
        let start = balances_of(&db, [ada, bea, cy]).await;
        let ropers = rope_walkers(&db, ada).await;
        take_in(&db, ropers, ada, bea, 0).await;
        take_in(&db, ropers, ada, cy, 0).await;
        set_role(&db, ada, ropers, bea, GuildRole::Officer, at(0)).await.unwrap();
        let total = strands_accounted(&db).await;

        // Any member deposits, no more than they have
        assert!(matches!(deposit(&db, cy, ropers, 301, at(1)).await, Err(GuildError::Short)));
        assert!(matches!(deposit(&db, cy, ropers, 0, at(1)).await, Err(GuildError::Strands)));
        deposit(&db, cy, ropers, 300, at(1)).await.unwrap();
        deposit(&db, ada, ropers, 700, at(1)).await.unwrap();

        // An officer's limit starts at nothing, and only the leader sets it
        assert!(matches!(withdraw_strands(&db, bea, ropers, 1, "", at(1)).await, Err(GuildError::OverLimit(0))));
        assert!(matches!(set_limit(&db, bea, ropers, 100).await, Err(GuildError::NotAllowed(_))));
        set_limit(&db, ada, ropers, 100).await.unwrap();
        let told = withdraw_strands(&db, bea, ropers, 60, "Rope for the west road", at(1)).await.unwrap();
        assert_eq!(told.paid, vec![bea]);
        assert!(matches!(withdraw_strands(&db, bea, ropers, 50, "", at(2)).await, Err(GuildError::OverLimit(40))));
        withdraw_strands(&db, bea, ropers, 40, "", at(2)).await.unwrap();
        let seen = detail_at(&db, ropers, Some(bea), at(2)).await.unwrap().treasury.unwrap();
        assert_eq!((seen.strands, seen.withdrawal_limit, seen.withdrawn_this_week), (900, 100, 100));
        // A member can't at all; the leader has no limit, only the treasury's
        assert!(matches!(withdraw_strands(&db, cy, ropers, 1, "", at(2)).await, Err(GuildError::NotAllowed(_))));
        assert!(matches!(withdraw_strands(&db, ada, ropers, 901, "", at(2)).await, Err(GuildError::TreasuryShort(900))));
        withdraw_strands(&db, ada, ropers, 500, "", at(2)).await.unwrap();
        // The week rolls on from each draw: day 2's 40 still counts on day 8
        assert!(matches!(withdraw_strands(&db, bea, ropers, 61, "", at(8)).await, Err(GuildError::OverLimit(60))));
        withdraw_strands(&db, bea, ropers, 100, "", at(9)).await.unwrap();

        let entries = books(&db, ropers, cy).await.unwrap().entries;
        assert!(entries.iter().all(|e| e.kind != GuildEntryKind::Fee), "the roster changed before it had a Strand");
        let draw = entries.iter().find(|e| e.note == "Rope for the west road").unwrap();
        assert_eq!((draw.kind, draw.strands, draw.by.as_ref().map(|b| b.id)), (GuildEntryKind::Withdrawal, -60, Some(bea)));
        assert_eq!(treasury(&db, ropers).await.0, 1000 - 60 - 40 - 500 - 100);
        assert_eq!(entries[0].treasury, 300, "the newest first, with the treasury after");
        assert_eq!(strands_accounted(&db).await, total, "held, not sunk");
        assert_audited(&db, &start).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_treasury_pays_the_roster_fee_when_it_can(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 30).await;
        let bea = player(&db, "Bea", 0).await;
        let cy = player(&db, "Cy", 0).await;
        let ropers = rope_walkers(&db, ada).await;
        // Empty, so Bea can't join with no Strand of her own
        invite(&db, ada, ropers, bea, at(0)).await.unwrap();
        assert!(matches!(apply(&db, bea, ropers, at(0)).await, Err(GuildError::RosterFee)));
        deposit(&db, ada, ropers, 2, at(0)).await.unwrap();
        let told = apply(&db, bea, ropers, at(0)).await.unwrap();
        assert!(told.paid.is_empty(), "the treasury paid");
        let told = set_role(&db, ada, ropers, bea, GuildRole::Officer, at(0)).await.unwrap();
        assert!(told.paid.is_empty());
        // Spent, it falls to whoever makes the change
        let before = balances_of(&db, [ada]).await[&ada];
        apply(&db, cy, ropers, at(0)).await.unwrap();
        assert_eq!(invite(&db, ada, ropers, cy, at(0)).await.unwrap().paid, vec![ada]);
        assert_eq!(balances_of(&db, [ada]).await[&ada], before - guild::ROSTER_FEE);
        let entries = books(&db, ropers, bea).await.unwrap().entries;
        assert_eq!(entries.iter().filter(|e| e.kind == GuildEntryKind::Fee).count(), 2);
        assert_eq!(held_flows(&mut db.acquire().await.unwrap()).await.unwrap()["guild_fee"], -2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn four_weeks_of_debt_warn_the_officers_then_dissolve_the_guild(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 60).await;
        let bea = player(&db, "Bea", 10).await;
        let cy = player(&db, "Cy", 20).await;
        let start = balances_of(&db, [ada, bea, cy]).await;
        let ropers = rope_walkers(&db, ada).await;
        take_in(&db, ropers, ada, bea, 0).await;
        take_in(&db, ropers, ada, cy, 0).await;
        set_role(&db, ada, ropers, bea, GuildRole::Officer, at(0)).await.unwrap();
        // 30 in, against the first week's 65: 35 owed
        deposit(&db, cy, ropers, 10, at(0)).await.unwrap();
        deposit(&db, ada, ropers, 20, at(0)).await.unwrap();
        let total = strands_accounted(&db).await;

        let told = dues_for(&db, at(1), 168).await;
        assert_eq!(treasury(&db, ropers).await, (0, 35));
        assert!(told.is_empty(), "under a week owed");
        // Passing a week of debt warns the leader and the officer, queued for them
        let told = dues_for(&db, at(8), 168).await;
        assert_eq!(treasury(&db, ropers).await, (0, 100));
        assert_eq!(told.len(), 1);
        // Ada and Bea joined in the same moment, so in either order
        let mut changed = told[0].changed.clone();
        changed.sort();
        let mut officers = vec![ada, bea];
        officers.sort();
        assert_eq!(changed, officers);
        assert!(matches!(&told[0].notices[..], [(_, _, ServerMessage::GuildDues { owed: 65, dissolves_at: 260, .. }), _]));
        let rate = capitol_protocol::tunables::PROPERTY_TAX_RATE;
        let dues_notices = |waiting: Vec<ServerMessage>| -> Vec<ServerMessage> {
            waiting.into_iter().filter(|m| matches!(m, ServerMessage::GuildDues { .. })).collect()
        };
        let waiting = dues_notices(notices::take_all(&db, bea, rate).await.unwrap());
        assert!(matches!(&waiting[..], [ServerMessage::GuildDues { tag, .. }] if tag == "ROPE"));
        assert!(dues_notices(notices::take_all(&db, cy, rate).await.unwrap()).is_empty(), "a member isn't warned");

        // A deposit pays the debt first, in the next hour
        deposit(&db, cy, ropers, 5, at(15)).await.unwrap();
        dues_for(&db, at(15), 1).await;
        assert_eq!(treasury(&db, ropers).await, (0, 95));
        // Two more weeks warned, then at 260 it's dissolved
        let told = dues_for(&db, at(15) + Duration::hours(1), 168 * 3).await;
        let kinds: Vec<&str> = told.iter().flat_map(|t| t.notices.iter().map(|(_, _, m)| m.type_name())).collect();
        assert_eq!(kinds, ["guild_dues", "guild_dues", "guild_dues", "guild_dues", "guild_dissolved", "guild_dissolved", "guild_dissolved"]);
        assert_eq!(treasury(&db, ropers).await, (0, guild::dissolves_at(3)));
        let status: String = sqlx::query_scalar("SELECT status FROM guilds WHERE id = $1").bind(ropers).fetch_one(&db).await.unwrap();
        assert_eq!(status, "dissolved");
        assert!(mine(&db, cy).await.unwrap().guild.is_none(), "the roster's cleared");
        assert!(collect_dues(&db, at(40)).await.is_empty(), "and no more dues");
        // Its name is free, and nothing was made or lost
        found(&db, ada, "The Rope Walkers", "ROPE", "", at(41)).await.unwrap();
        assert_eq!(strands_accounted(&db).await, total - guild::FOUNDING_FEE);
        assert_audited(&db, &start).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_closing_guild_pays_its_debt_and_shares_the_rest_by_tenure(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 30).await;
        let bea = player(&db, "Bea", 0).await;
        let cy = player(&db, "Cy", 0).await;
        let start = balances_of(&db, [ada, bea, cy]).await;
        let ropers = rope_walkers(&db, ada).await;
        deposit(&db, ada, ropers, 1000, at(0)).await.unwrap();
        // As of day 6, Ada has served 6 days, Bea 4 and Cy 2; the treasury
        // paid both their roster fees
        take_in(&db, ropers, ada, bea, 2).await;
        take_in(&db, ropers, ada, cy, 4).await;
        // As S4.3's foreclosed hall would leave it: 98 owed
        sqlx::query("UPDATE guilds SET dues_owed = 98 WHERE id = $1").bind(ropers).execute(&db).await.unwrap();
        let before = balances_of(&db, [ada, bea, cy]).await;
        let total = strands_accounted(&db).await;

        let mut tx = db.begin().await.unwrap();
        let held = hold(&mut tx, ropers).await.unwrap();
        let told = close(&mut tx, ropers, &held, Closing::Dissolved, at(6)).await.unwrap();
        tx.commit().await.unwrap();
        // 900 left, shared 6:4:2
        let after = balances_of(&db, [ada, bea, cy]).await;
        assert_eq!((after[&ada] - before[&ada], after[&bea], after[&cy]), (450, 300, 150));
        assert_eq!(told.paid, vec![ada, bea, cy]);
        assert!(matches!(&told.notices[..], [(_, _, ServerMessage::GuildDissolved { owed: 98, share: 450, .. }), _, _]));
        assert_eq!(treasury(&db, ropers).await, (0, 0));
        assert_eq!(strands_accounted(&db).await, total, "the debt is a sink the books keep, the rest shared");
        assert_audited(&db, &start).await;

        // A leader alone who leaves takes what's left, less what's owed
        let dee = subscriber(&db, "Dee", 1000, 30).await;
        let alone = found(&db, dee, "Alone", "ALONE", "", at(6)).await.unwrap().0;
        deposit(&db, dee, alone, 300, at(6)).await.unwrap();
        sqlx::query("UPDATE guilds SET dues_owed = 7 WHERE id = $1").bind(alone).execute(&db).await.unwrap();
        let told = leave(&db, dee, alone, at(7)).await.unwrap();
        assert_eq!((told.paid, told.notices.len()), (vec![dee], 0));
        assert_eq!(balances_of(&db, [dee]).await[&dee], 1000 - guild::FOUNDING_FEE - 7);
    }

    /// A seeded month of a guild's life: members come and go, deposit and
    /// draw, the dues run hourly, the roster fees fall on the treasury or the
    /// actor, and at the end it's dissolved. Every Strand is accounted for
    /// after each day, every balance is its trail, and the ledger shows no
    /// drift.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_seeded_month_of_a_guild_conserves_every_strand(db: PgPool) {
        use crate::engine::ledger;
        use capitol_protocol::ledger::reconcile;

        let ada = subscriber(&db, "Ada", 3000, 60).await;
        let mut people = vec![ada];
        for name in ["Bea", "Cy", "Dee", "Eve"] {
            people.push(player(&db, name, 400).await);
        }
        let start = balances_of(&db, people.clone()).await;
        let ropers = rope_walkers(&db, ada).await;
        let books_start = ledger::books(&db).await.unwrap();
        // A roster fee a player pays is a sink from their balance, which the
        // harness doesn't add back, so it's counted beside
        let total = strands_accounted(&db).await - fees_paid(&db).await;

        // SplitMix64, so the month is the same every run
        let mut seed = 0x5eed_0398_u64;
        let mut next = move |n: u64| {
            seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            (z ^ (z >> 31)) % n
        };
        set_limit(&db, ada, ropers, 80).await.unwrap();
        let (mut joined, mut drawn) = (0, 0);
        for day in 0..30 {
            let now = at(day);
            let who = people[next(people.len() as u64) as usize];
            match next(5) {
                0 | 1 => {
                    let _ = invite(&db, ada, ropers, who, now).await;
                    joined += usize::from(apply(&db, who, ropers, now).await.is_ok());
                }
                2 if who != ada => {
                    let _ = leave(&db, who, ropers, now).await;
                }
                3 if who != ada => {
                    let _ = set_role(&db, ada, ropers, who, GuildRole::Officer, now).await;
                }
                _ => {}
            }
            let _ = deposit(&db, who, ropers, next(120) as i64 + 1, now).await;
            drawn += usize::from(withdraw_strands(&db, who, ropers, next(60) as i64 + 1, "", now).await.is_ok());
            dues_for(&db, now, 24).await;
            assert_eq!(strands_accounted(&db).await - fees_paid(&db).await, total, "day {day}");
        }
        assert!(joined > 0 && drawn > 0, "the month did something: {joined} joined, {drawn} draws");
        // Then more owed than it holds, past four weeks: it's dissolved
        sqlx::query("UPDATE guilds SET dues_owed = treasury + 400 WHERE id = $1").bind(ropers).execute(&db).await.unwrap();
        let told = collect_dues(&db, at(30)).await;
        assert!(told.iter().any(|t| t.notices.iter().any(|(_, _, m)| matches!(m, ServerMessage::GuildDissolved { .. }))));
        assert!(held_flows(&mut db.acquire().await.unwrap()).await.unwrap()["guild_dues"] < 0, "dues were paid");
        assert_eq!(strands_accounted(&db).await - fees_paid(&db).await, total);
        assert_audited(&db, &start).await;
        let books_end = ledger::books(&db).await.unwrap();
        let books = reconcile(&books_start, &books_end);
        assert!(books.strands.holds(), "{:#?}", books.strands);
        assert_eq!(books_end.escrow_strands, 0, "nothing's held for a guild that's gone");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_lead_passes_to_a_member_with_no_officer_and_a_guild_of_one_keeps_it(db: PgPool) {
        let ada = subscriber(&db, "Ada", 5000, 10).await;
        let bea = player(&db, "Bea", 10).await;
        let cy = subscriber(&db, "Cy", 10, 100).await;
        let ropers = rope_walkers(&db, ada).await;
        let lone = subscriber(&db, "Lone", 5000, 10).await;
        found(&db, lone, "Alone", "ALONE", "", at(0)).await.unwrap();
        take_in(&db, ropers, ada, bea, 1).await;
        take_in(&db, ropers, ada, cy, 2).await;

        let told = pass_lapsed_leads(&db, at(10 + 28)).await;
        assert_eq!(told.len(), 1, "the guild of one keeps its leader");
        assert_eq!(role(&db, ropers, cy).await, Some(GuildRole::Leader), "a subscribing member before one who's served longer");
        assert_eq!(role(&db, ropers, bea).await, Some(GuildRole::Member));
    }

    /// An unclaimed lot in the guild district, the `n`th, for a hall (S4.3)
    async fn hall_lot(db: &PgPool, n: i32) -> Uuid {
        use capitol_protocol::rules::{CENTER_X, CENTER_Y};
        let (x, y) = (CENTER_X + 800.0, CENTER_Y + 40.0 * n as f64);
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO plots (id, terrain_id, zone_id, grid_x, grid_y, world_x, world_y,
                                bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y, size_category)
             VALUES ($1, $2, 'guild_district', 9000, $3, $4, $5, $4 - 10, $5 - 10, $4 + 10, $5 + 10, 'guild_hall')",
        )
        .bind(id)
        .bind(format!("hall-{n}"))
        .bind(n)
        .bind(x as f32)
        .bind(y as f32)
        .execute(db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO plot_permissions (plot_id) VALUES ($1)").bind(id).execute(db).await.unwrap();
        id
    }

    /// The hall's owner, guild, deed and station use level
    async fn hall_row(db: &PgPool, plot: Uuid) -> (Option<Uuid>, Option<Uuid>, i32, String) {
        sqlx::query_as(
            "SELECT p.owner_id, p.guild_id, p.assessed_value, pp.station_use_level
             FROM plots p JOIN plot_permissions pp ON pp.plot_id = p.id WHERE p.id = $1",
        )
        .bind(plot)
        .fetch_one(db)
        .await
        .unwrap()
    }

    /// The deed of a guild-district lot, at the default tunables
    const HALL: i64 = 5000;

    /// The leader claims a hall from the treasury, which must hold its deed;
    /// a guild holds one, a lot one guild, and only in the guild district.
    /// The deed is a sink from the treasury, the Gazette hears the guild's
    /// name, the hall opens to the guild, and its owner follows the lead.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_guild_claims_a_hall_from_its_treasury(db: PgPool) {
        use crate::engine::plot_sync::tests::plot;
        let tunables = Tunables::default();
        let ada = subscriber(&db, "Ada", 20_000, 30).await;
        let bea = player(&db, "Bea", 10).await;
        let cy = subscriber(&db, "Cy", 10, 30).await;
        let dee = subscriber(&db, "Dee", 10_000, 30).await;
        let start = balances_of(&db, [ada, bea, cy, dee]).await;
        let ropers = rope_walkers(&db, ada).await;
        take_in(&db, ropers, ada, bea, 1).await;
        take_in(&db, ropers, ada, cy, 1).await;
        let (lot, other, urban) = (hall_lot(&db, 1).await, hall_lot(&db, 2).await, plot(&db, None, 0).await);
        deposit(&db, ada, ropers, HALL - 1, at(1)).await.unwrap();
        let claim = |who, lot| claim_hall(&db, who, ropers, lot, &tunables, at(2));

        assert!(matches!(claim(bea, lot).await, Err(GuildError::NotAllowed(_))), "the leader's to do");
        assert!(matches!(claim(ada, lot).await, Err(GuildError::HallShort { cost: HALL, has }) if has == HALL - 1));
        deposit(&db, ada, ropers, 101, at(1)).await.unwrap();
        assert!(matches!(claim(ada, urban).await, Err(GuildError::NotAHallLot)));
        assert!(matches!(claim(ada, Uuid::new_v4()).await, Err(GuildError::NoLot)));
        let total = strands_accounted(&db).await;

        let told = claim(ada, lot).await.unwrap();
        assert_eq!(hall_row(&db, lot).await, (Some(ada), Some(ropers), HALL as i32, "guild".into()));
        assert_eq!(treasury(&db, ropers).await, (100, 0));
        let hall_named = |t: &Told, owner: Uuid| {
            t.broadcast.iter().any(|e| {
                matches!(e, GameEvent::PlotClaimed { owner_id, guild: Some(g), .. } if *owner_id == owner && g.id == ropers && g.tag == "ROPE")
            })
        };
        assert!(hall_named(&told, ada), "everyone hears whose hall it is: {:?}", told.broadcast);
        let entry = books(&db, ropers, bea).await.unwrap().entries.remove(0);
        assert_eq!((entry.kind, entry.strands, entry.treasury), (GuildEntryKind::Hall, -HALL, 100));
        let guild: Option<Uuid> = sqlx::query_scalar("SELECT guild_id FROM deed_events WHERE plot_id = $1 AND kind = 'hall'")
            .bind(lot)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(guild, Some(ropers), "for the Gazette's deeds column");
        let hall = detail(&db, ropers, None).await.unwrap().hall.expect("anyone sees where its hall stands");
        assert_eq!((hall.plot_id, hall.terrain_id.as_deref()), (lot, Some("hall-1")));

        // One hall a guild, and one guild a lot
        deposit(&db, ada, ropers, HALL, at(2)).await.unwrap();
        assert!(matches!(claim(ada, other).await, Err(GuildError::HasAHall)));
        let weavers = found(&db, dee, "The Weavers", "WEAVE", "", at(2)).await.unwrap().0;
        deposit(&db, dee, weavers, HALL, at(2)).await.unwrap();
        let taken = claim_hall(&db, dee, weavers, lot, &tunables, at(2)).await;
        assert!(matches!(taken, Err(GuildError::LotTaken)));

        // The deed is a sink the treasury's books keep, so nothing's lost
        // but Dee's founding fee, a sink from her balance
        assert_eq!(strands_accounted(&db).await, total - guild::FOUNDING_FEE);
        let held = held_flows(&mut db.acquire().await.unwrap()).await.unwrap();
        assert_eq!(held["deed_purchase"], -HALL);

        // The hall goes with the lead
        let told = set_role(&db, ada, ropers, cy, GuildRole::Leader, at(3)).await.unwrap();
        assert_eq!(hall_row(&db, lot).await.0, Some(cy));
        assert!(hall_named(&told, cy));
        assert_audited(&db, &start).await;
    }

    /// The hall's tax comes from the treasury, debt first as Strands come
    /// in. Its officers are warned as the debt passes each week, and at four
    /// weeks it's foreclosed: the deed less the debt goes into the treasury,
    /// and the officers hear. Every Strand is accounted for, and the books
    /// balance.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_halls_tax_comes_from_the_treasury_and_forecloses_on_debt(db: PgPool) {
        use crate::engine::ledger;
        use crate::engine::property_tax::process_property_taxes;
        use capitol_protocol::ledger::reconcile;
        use capitol_protocol::tunables::PROPERTY_TAX_RATE as RATE;
        let ada = subscriber(&db, "Ada", 10_000, 30).await;
        let bea = player(&db, "Bea", 10).await;
        let start = balances_of(&db, [ada, bea]).await;
        let books_start = ledger::books(&db).await.unwrap();
        let ropers = rope_walkers(&db, ada).await;
        take_in(&db, ropers, ada, bea, 1).await;
        set_role(&db, ada, ropers, bea, GuildRole::Officer, at(1)).await.unwrap();
        deposit(&db, ada, ropers, HALL + 300, at(1)).await.unwrap();
        let lot = hall_lot(&db, 1).await;
        claim_hall(&db, ada, ropers, lot, &Tunables::default(), at(2)).await.unwrap();
        let total = strands_accounted(&db).await;
        let (before, _) = treasury(&db, ropers).await;
        let owed = || async {
            sqlx::query_scalar::<_, i32>("SELECT tax_owed FROM plots WHERE id = $1").bind(lot).fetch_one(&db).await.unwrap()
        };

        // A week's tax, 2% of 5000, from the treasury: Ada pays nothing
        for _ in 0..168 {
            process_property_taxes(&db, RATE).await;
        }
        assert_eq!((treasury(&db, ropers).await.0, owed().await), (before - 100, 0));
        assert_eq!(balances_of(&db, [ada]).await[&ada], 10_000 - guild::FOUNDING_FEE - guild::ROSTER_FEE - HALL - 300, "and making Bea an officer");

        // An empty treasury runs up debt; passing three weeks' worth warns
        // the officers and the leader
        withdraw_strands(&db, ada, ropers, before - 100, "", at(9)).await.unwrap();
        sqlx::query("UPDATE plots SET tax_owed = 299 WHERE id = $1").bind(lot).execute(&db).await.unwrap();
        let mut warned = Vec::new();
        for _ in 0..3 {
            warned.extend(process_property_taxes(&db, RATE).await.notices);
        }
        let mut warned: Vec<Uuid> = warned.into_iter().map(|n| n.owner_id).collect();
        warned.sort();
        let mut officers = vec![ada, bea];
        officers.sort();
        assert_eq!(warned, officers);

        // A deposit pays the debt off first
        deposit(&db, bea, ropers, 5, at(9)).await.unwrap();
        process_property_taxes(&db, RATE).await;
        assert!(owed().await <= 300 - 4, "{}", owed().await);

        // At four weeks it's foreclosed into the treasury
        sqlx::query("UPDATE plots SET tax_owed = 399 WHERE id = $1").bind(lot).execute(&db).await.unwrap();
        let mut foreclosed = Vec::new();
        for _ in 0..3 {
            foreclosed.extend(process_property_taxes(&db, RATE).await.foreclosures);
        }
        let [f] = &foreclosed[..] else { panic!("one foreclosure: {foreclosed:?}") };
        assert_eq!((f.owner_id, f.guild, f.tax_owed, f.refund), (ada, Some(ropers), 400, HALL - 400));
        assert_eq!(f.officers.iter().map(|o| o.officer).collect::<Vec<_>>(), vec![bea], "the other officers hear too");
        assert_eq!(hall_row(&db, lot).await.0, None);
        assert_eq!(hall_row(&db, lot).await.1, None, "the guild's no longer");
        assert_eq!(treasury(&db, ropers).await.0, HALL - 400);
        assert!(detail(&db, ropers, None).await.unwrap().hall.is_none());
        let entry = books(&db, ropers, ada).await.unwrap().entries.remove(0);
        assert_eq!((entry.kind, entry.strands), (GuildEntryKind::Refund, HALL - 400));

        // The tax and the refund are the treasury's own, so nothing's lost or made
        assert_eq!(strands_accounted(&db).await, total);
        assert_audited(&db, &start).await;
        let books_end = ledger::books(&db).await.unwrap();
        let books = reconcile(&books_start, &books_end);
        assert!(books.strands.holds(), "{:#?}", books.strands);
        assert!(books.strands.flows["property_tax"] < 0 && books.strands.flows["foreclosure_refund"] == HALL - 400);
    }

    /// A guild that closes gives its hall up first: its stations come down
    /// and go home, and the deed, less the tax owed, is in the treasury it
    /// shares
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_closing_guild_gives_up_its_hall(db: PgPool) {
        let ada = subscriber(&db, "Ada", 10_000, 30).await;
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(Uuid::new_v4())
            .bind(ada)
            .execute(&db)
            .await
            .unwrap();
        let start = balances_of(&db, [ada]).await;
        let ropers = rope_walkers(&db, ada).await;
        deposit(&db, ada, ropers, HALL + 1000, at(1)).await.unwrap();
        let lot = hall_lot(&db, 1).await;
        claim_hall(&db, ada, ropers, lot, &Tunables::default(), at(1)).await.unwrap();
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality)
             SELECT $1, 'storage_chest', $2, world_x, world_y, id, 50 FROM plots WHERE id = $3",
        )
        .bind(station)
        .bind(ada)
        .bind(lot)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("UPDATE plots SET tax_owed = 50, station_count = 1 WHERE id = $1").bind(lot).execute(&db).await.unwrap();

        let told = leave(&db, ada, ropers, at(2)).await.unwrap();
        let [hall] = &told.halls[..] else { panic!("its hall: {:?}", told.halls) };
        assert_eq!((hall.terrain_id.as_deref(), &hall.removed_stations[..]), (Some("hall-1"), &[station][..]));
        assert_eq!(hall_row(&db, lot).await.0, None);
        assert_eq!(balances_of(&db, [ada]).await[&ada], 10_000 - guild::FOUNDING_FEE - 50, "all but the founding and the tax owed");
        assert!(told.notices.is_empty(), "the kit found room, so a disbanding tells nobody");
        assert_audited(&db, &start).await;
    }

    /// What of a closing hall's finds no room goes to its owner's impound,
    /// and each owner hears what in their `guild_dissolved`, member or not,
    /// on a dissolution or a disbanding (#512)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_closing_halls_owners_hear_what_went_to_their_impound(db: PgPool) {
        use capitol_protocol::ws::ItemCount;
        let count = |item_type: &str, quantity| ItemCount { item_type: item_type.into(), quantity };
        // A hall with a chest in it, its box holding someone else's rope
        async fn chest_in(db: &PgPool, lot: Uuid, placer: Uuid, rope_owner: Uuid) {
            let station = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality)
                 SELECT $1, 'storage_chest', $2, world_x, world_y, id, 50 FROM plots WHERE id = $3",
            )
            .bind(station)
            .bind(placer)
            .bind(lot)
            .execute(db)
            .await
            .unwrap();
            let chest_box = Uuid::new_v4();
            sqlx::query("INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2)")
                .bind(chest_box)
                .bind(station)
                .execute(db)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
                 VALUES ($1, 'rope', 60, 12, $2, $3, 0)",
            )
            .bind(Uuid::new_v4())
            .bind(rope_owner)
            .bind(chest_box)
            .execute(db)
            .await
            .unwrap();
            sqlx::query("UPDATE plots SET station_count = 1 WHERE id = $1").bind(lot).execute(db).await.unwrap();
        }

        // Dissolved: Ada and Bea are members, Cy isn't. None has an
        // inventory, so Bea's kit and Cy's rope go to their impounds
        let ada = subscriber(&db, "Ada", 10_000, 30).await;
        let bea = player(&db, "Bea", 0).await;
        let cy = player(&db, "Cy", 0).await;
        let ropers = rope_walkers(&db, ada).await;
        // The treasury pays Bea's roster fee
        deposit(&db, ada, ropers, HALL + 600, at(0)).await.unwrap();
        take_in(&db, ropers, ada, bea, 0).await;
        let lot = hall_lot(&db, 1).await;
        claim_hall(&db, ada, ropers, lot, &Tunables::default(), at(1)).await.unwrap();
        chest_in(&db, lot, bea, cy).await;

        let mut tx = db.begin().await.unwrap();
        let held = hold(&mut tx, ropers).await.unwrap();
        let mut told = close(&mut tx, ropers, &held, Closing::Dissolved, at(6)).await.unwrap();
        super::super::ledger::commit(tx, std::mem::take(&mut told.flows)).await.unwrap();
        let heard: Vec<(Uuid, i64, Vec<ItemCount>, Vec<ItemCount>)> = told
            .notices
            .iter()
            .map(|(p, _, m)| match m {
                ServerMessage::GuildDissolved { share, impounded, lost, .. } => (*p, *share, impounded.clone(), lost.clone()),
                other => panic!("not a guild_dissolved: {other:?}"),
            })
            .collect();
        // By who it's for: Ada and Bea joined in the same instant, so the
        // members' order falls to their ids (#564)
        assert_eq!(heard.len(), 3, "Ada, Bea and Cy: {heard:?}");        let to = |who: Uuid| heard.iter().find(|(p, ..)| *p == who).unwrap_or_else(|| panic!("{who}'s: {heard:?}"));
        let ((_, a_share, a_pound, a_lost), (_, b_share, b_pound, b_lost), (_, c_share, c_pound, c_lost)) = (to(ada), to(bea), to(cy));
        assert!(*a_share > 0 && *b_share > 0, "the members' shares: {a_share}, {b_share}");
        assert_eq!(*c_share, 0, "Cy had no share");
        assert!(a_pound.is_empty() && a_lost.is_empty(), "nothing of Ada's was there");
        assert_eq!((&b_pound[..], b_lost.is_empty()), (&[count("chest_kit", 1)][..], true));
        assert_eq!((&c_pound[..], c_lost.is_empty()), (&[count("rope", 12)][..], true));
        // Queued for them, so they hear it as they come back
        let rate = capitol_protocol::tunables::PROPERTY_TAX_RATE;
        let waiting = notices::take_all(&db, cy, rate).await.unwrap();
        assert!(
            matches!(&waiting[..], [ServerMessage::GuildDissolved { impounded, .. }] if impounded == &[count("rope", 12)]),
            "Cy's: {waiting:?}"
        );

        // Disbanded: Dee leaves her guild of one, and hears where her kit went
        let dee = subscriber(&db, "Dee", 10_000, 30).await;
        let alone = found(&db, dee, "Alone", "ALONE", "", at(6)).await.unwrap().0;
        deposit(&db, dee, alone, HALL + 100, at(6)).await.unwrap();
        let lot = hall_lot(&db, 2).await;
        claim_hall(&db, dee, alone, lot, &Tunables::default(), at(6)).await.unwrap();
        chest_in(&db, lot, dee, dee).await;
        let told = leave(&db, dee, alone, at(7)).await.unwrap();
        let [(p, _, ServerMessage::GuildDissolved { owed: 0, share, impounded, lost, .. })] = &told.notices[..] else {
            panic!("Dee's: {:?}", told.notices)
        };
        assert_eq!((*p, *share > 0), (dee, true));
        assert_eq!((&impounded[..], lost.is_empty()), (&[count("chest_kit", 1), count("rope", 12)][..], true));
    }
}
