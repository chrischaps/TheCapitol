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
//! Each change to the roster costs [`guild::ROSTER_FEE`] from whoever makes
//! it (a `guild_fee`, a sink), until S4.2's treasury pays. Leaving is free.
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

use super::currency::{announce_balances, subtract_strands_in, TransactionType};
use super::notices;
use super::GameEvent;

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
    /// Who paid a fee, to be told their balance
    pub paid: Option<Uuid>,
}

impl Told {
    /// Sends what a committed change tells: `guild_changed`, its notices
    /// (each taken as it's sent live) and the payer's balance.
    pub async fn send(self, db: &PgPool, events: &broadcast::Sender<GameEvent>) {
        if !self.changed.is_empty() {
            let _ = events.send(GameEvent::GuildChanged { guild_id: self.guild_id, players: self.changed });
        }
        for (player_id, notice_id, message) in self.notices {
            let _ = events.send(GameEvent::BazaarNotice { player_id, notice_id, message });
        }
        if let Some(payer) = self.paid {
            let balance: Result<i64, _> = sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
                .bind(payer)
                .fetch_one(db)
                .await;
            if let Ok(balance) = balance {
                announce_balances(events, [(payer, balance)]);
            }
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
    Ok((id, Told { guild_id: id, changed: vec![founder], notices: Vec::new(), paid: Some(founder) }))
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
            charge(&mut tx, actor, guild_id, &format!("{} joins {}", them.name, held.name)).await?;
            join(&mut tx, guild_id, player, GuildRole::Member, now).await?;
            let changed = members_of(&mut tx, guild_id).await?;
            tx.commit().await?;
            Ok(Told { guild_id, changed, notices: Vec::new(), paid: Some(actor) })
        }
        Some(GuildInviteKind::Invite) => Err(GuildError::AlreadyWaiting),
        None => {
            ask(&mut tx, guild_id, player, actor, GuildInviteKind::Invite, now).await?;
            let by = correspondent(&mut tx, actor).await?.ok_or(GuildError::NoOne)?;
            let notice = notify(&mut tx, player, guild_id, &held, GuildNoticeKind::Invited, &by).await?;
            let changed = officers_of(&mut tx, guild_id).await?;
            tx.commit().await?;
            Ok(Told { guild_id, changed, notices: vec![notice], paid: None })
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
            charge(&mut tx, player, guild_id, &format!("Joining {}", held.name)).await?;
            join(&mut tx, guild_id, player, GuildRole::Member, now).await?;
            let changed = members_of(&mut tx, guild_id).await?;
            tx.commit().await?;
            Ok(Told { guild_id, changed, notices: Vec::new(), paid: Some(player) })
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
            Ok(Told { guild_id, changed, notices, paid: None })
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
    Ok(Told { guild_id, changed, notices: Vec::new(), paid: None })
}

/// `player` leaves their guild, `guild_id`, for nothing. A leader can leave
/// only as the last member, and the guild is disbanded.
pub async fn leave(db: &PgPool, player: Uuid, guild_id: Uuid, now: DateTime<Utc>) -> Result<Told, GuildError> {
    let mut tx = db.begin().await?;
    if guild_of(&mut tx, player).await? != Some(guild_id) {
        return Err(GuildError::NotInAGuild);
    }
    hold(&mut tx, guild_id).await?;
    let role = role_in(&mut tx, guild_id, player).await?.ok_or(GuildError::NotInAGuild)?;
    let others: Vec<Uuid> = members_of(&mut tx, guild_id).await?.into_iter().filter(|m| *m != player).collect();
    if role == GuildRole::Leader && !others.is_empty() {
        return Err(GuildError::LeaderLeaving);
    }
    sqlx::query("DELETE FROM guild_members WHERE player_id = $1")
        .bind(player)
        .execute(&mut *tx)
        .await?;
    if others.is_empty() {
        sqlx::query("UPDATE guilds SET status = 'disbanded', disbanded_at = $2 WHERE id = $1")
            .bind(guild_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM guild_invites WHERE guild_id = $1")
            .bind(guild_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let mut changed = others;
    changed.push(player);
    Ok(Told { guild_id, changed, notices: Vec::new(), paid: None })
}

/// `actor` expels `player` from `guild_id`: an officer a member, the leader
/// an officer or a member.
pub async fn expel(db: &PgPool, actor: Uuid, guild_id: Uuid, player: Uuid) -> Result<Told, GuildError> {
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
    charge(&mut tx, actor, guild_id, &format!("{} expelled from {}", them.name, held.name)).await?;
    sqlx::query("DELETE FROM guild_members WHERE player_id = $1")
        .bind(player)
        .execute(&mut *tx)
        .await?;
    let by = correspondent(&mut tx, actor).await?.ok_or(GuildError::NoOne)?;
    let notice = notify(&mut tx, player, guild_id, &held, GuildNoticeKind::Expelled, &by).await?;
    let mut changed = members_of(&mut tx, guild_id).await?;
    changed.push(player);
    tx.commit().await?;
    Ok(Told { guild_id, changed, notices: vec![notice], paid: Some(actor) })
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
        return Ok(Told { guild_id, changed: Vec::new(), notices: Vec::new(), paid: None });
    }
    let them = correspondent(&mut tx, player).await?.ok_or(GuildError::NoOne)?;
    let mut notices = Vec::new();
    if to == GuildRole::Leader {
        let subscription = subscription_of(&mut tx, player).await?.ok_or(GuildError::NoOne)?;
        if !subscription.is_active_at(now) {
            return Err(GuildError::Unsubscribed("The lead goes only to someone with a subscription".into()));
        }
        charge(&mut tx, actor, guild_id, &format!("The lead of {} to {}", held.name, them.name)).await?;
        hand_on(&mut tx, guild_id, actor, player, now).await?;
        let by = correspondent(&mut tx, actor).await?.ok_or(GuildError::NoOne)?;
        notices.push(notify(&mut tx, player, guild_id, &held, GuildNoticeKind::Lead, &by).await?);
    } else {
        charge(&mut tx, actor, guild_id, &format!("{} made {} in {}", them.name, to.as_str(), held.name)).await?;
        sqlx::query("UPDATE guild_members SET role = $2 WHERE player_id = $1")
            .bind(player)
            .bind(to.as_str())
            .execute(&mut *tx)
            .await?;
    }
    let changed = members_of(&mut tx, guild_id).await?;
    tx.commit().await?;
    Ok(Told { guild_id, changed, notices, paid: Some(actor) })
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
    hand_on(&mut tx, guild_id, leader, next, now).await?;
    let by = correspondent(&mut tx, leader).await?.ok_or(GuildError::NoOne)?;
    let notice = notify(&mut tx, next, guild_id, &held, GuildNoticeKind::Lead, &by).await?;
    let changed = members_of(&mut tx, guild_id).await?;
    tx.commit().await?;
    tracing::info!("The lead of {} passed to {next}: its leader {leader} lapsed", held.name);
    Ok(Some(Told { guild_id, changed, notices: vec![notice], paid: None }))
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
    let members: Vec<GuildMember> = rows
        .into_iter()
        .map(|(id, name, citizen, role, joined_at)| GuildMember {
            id,
            name,
            citizen,
            role: GuildRole::parse(&role).unwrap_or(GuildRole::Member),
            joined_at,
            // S3.1 (#395) brings the record and the words
            standing: standing::NEW_IN_TOWN.into(),
        })
        .collect();
    let sees_waiting = viewer.is_some_and(|v| members.iter().any(|m| m.id == v && m.role.may_invite()));
    let waiting = if sees_waiting { invites(&mut conn, "i.guild_id", guild_id).await? } else { Vec::new() };
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

/// The lead of `guild_id` goes from `from` to `to`; `from` becomes an officer.
async fn hand_on(conn: &mut PgConnection, guild_id: Uuid, from: Uuid, to: Uuid, now: DateTime<Utc>) -> Result<(), sqlx::Error> {
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
        .execute(conn)
        .await?;
    Ok(())
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

/// Takes [`guild::ROSTER_FEE`] from `payer` for a change to `guild_id`'s roster.
async fn charge(conn: &mut PgConnection, payer: Uuid, guild_id: Uuid, what: &str) -> Result<(), GuildError> {
    subtract_strands_in(conn, payer, guild::ROSTER_FEE, TransactionType::GuildFee, Some(what), Some(guild_id))
        .await
        .map(|_| ())
        .map_err(|e| fee_error(e, GuildError::RosterFee))
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
        assert_eq!((told.changed, told.paid), (vec![ada], Some(ada)));
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
        assert_eq!(card(&db, Uuid::nil(), Who::Id(cy)).await.unwrap().guild.as_deref(), Some("The Rope Walkers"));
        assert_eq!(card(&db, Uuid::nil(), Who::Id(ada)).await.unwrap().guild.as_deref(), Some("The Rope Walkers"));
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
        assert_eq!((told.paid, told.changed.clone()), (None, vec![ada]), "an invitation costs nothing");
        let waiting = notices::take_all(&db, bea, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap();
        assert!(matches!(&waiting[..], [ServerMessage::GuildNotice { kind: GuildNoticeKind::Invited, who, tag, .. }] if *who == ada && tag == "ROPE"));
        assert!(matches!(invite(&db, ada, ropers, bea, at(1)).await, Err(GuildError::AlreadyWaiting)));
        let as_leader = detail(&db, ropers, Some(ada)).await.unwrap();
        assert_eq!(as_leader.waiting.len(), 1);
        assert!(detail(&db, ropers, Some(bea)).await.unwrap().waiting.is_empty(), "only officers see who's waiting");

        // She accepts by applying, and pays the roster's Strand
        let told = apply(&db, bea, ropers, at(2)).await.unwrap();
        assert_eq!((told.paid, told.changed), (Some(bea), vec![ada, bea]));
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
        assert!(matches!(expel(&db, cy, ropers, dee).await, Err(GuildError::NotAllowed(_))));
        assert!(matches!(set_role(&db, cy, ropers, dee, GuildRole::Officer, at(5)).await, Err(GuildError::NotAllowed(_))));

        // The leader makes Bea an officer, for a Strand
        let before = balances_of(&db, [ada]).await[&ada];
        set_role(&db, ada, ropers, bea, GuildRole::Officer, at(5)).await.unwrap();
        assert_eq!(balances_of(&db, [ada]).await[&ada], before - guild::ROSTER_FEE);
        assert!(set_role(&db, ada, ropers, bea, GuildRole::Officer, at(5)).await.unwrap().paid.is_none(), "no change, no fee");

        // An officer invites and expels members, but not officers or the leader, nor sets roles
        invite(&db, bea, ropers, eve, at(5)).await.unwrap();
        assert!(matches!(set_role(&db, bea, ropers, cy, GuildRole::Officer, at(5)).await, Err(GuildError::NotAllowed(_))));
        assert!(matches!(expel(&db, bea, ropers, ada).await, Err(GuildError::NotAllowed(_))));
        let told = expel(&db, bea, ropers, dee).await.unwrap();
        assert!(told.changed.contains(&dee), "the expelled hear it too");
        assert!(matches!(&told.notices[..], [(to, _, ServerMessage::GuildNotice { kind: GuildNoticeKind::Expelled, .. })] if *to == dee));
        assert_eq!(role(&db, ropers, dee).await, None);

        // A roster change needs the Strand: Eve has none to join with
        assert!(matches!(apply(&db, eve, ropers, at(6)).await, Err(GuildError::RosterFee)));
        assert_eq!(role(&db, ropers, eve).await, None);

        // The leader expels an officer, after a promotion
        set_role(&db, ada, ropers, cy, GuildRole::Officer, at(6)).await.unwrap();
        assert!(matches!(expel(&db, bea, ropers, cy).await, Err(GuildError::NotAllowed(m)) if m.contains("leader")));
        expel(&db, ada, ropers, cy).await.unwrap();

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
}
