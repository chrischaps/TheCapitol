//! Scoped API keys for bots (E0.9).
//!
//! An account mints keys at `/account/api-keys` (`routes::account`). A key
//! acts for its account with only its scopes:
//! - `read`: GETs, and the WebSocket feed. A connection whose key has only
//!   `read` watches: it doesn't take the player over, so the player's own
//!   client stays connected, and it takes none of their waiting notices.
//! - `play`: movement, gathering, crafting, inventory and stations.
//! - `trade`: trades, the exchange and plots (and, from E3, the market).
//!
//! `play` and `trade` imply `read`. A key never reaches `/account` (a key
//! can't mint or revoke keys) or `/admin`, but for one route:
//! - `town`: `POST /admin/town/report` and nothing else, not even a GET or
//!   the WebSocket. It's the town's key for its reports (D4.1), which only
//!   an admin mints (`POST /admin/town/key`), on their own account. It
//!   needs no subscription: it isn't a bot.
//!
//! A key isn't a session: it doesn't replace the account's login (#78), so a
//! bot on a key leaves its owner signed in over REST. Over the WebSocket a
//! `play` or `trade` key does take the player over, as a second login's
//! socket would, until #151 decides how many presences an account may have.
//!
//! The secret is `cap_` and 64 hex digits, shown once. Only its SHA-256 is
//! kept. A revoked key is refused from its next request, and its sockets are
//! closed (`GameState::revoke_key`).
//!
//! Bots are for subscribers (#217, GDD §5). Minting a key takes a running
//! subscription (`subscriptions::Subscriber`), and a person's key signs in
//! only while its account's subscription runs: when it lapses, the hourly
//! check hangs up the key's sockets, and the next sign-in is refused. A
//! citizen's key needs none: citizens are the server's own town, and a
//! subscription isn't Strands, so they still earn everything (Citizens D2).
//! It's friction, not a wall: a bot can still sign in with its owner's
//! password, and nothing hunts for that.

use axum::http::Method;
use capitol_protocol::rest::{AccountKind, ApiKeyInfo, ApiKeyScope, Subscription, SubscriptionStatus};
use capitol_protocol::ws::ClientMessage;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Every secret starts with this, which is how the middleware and the
/// WebSocket tell one from an access token.
pub use capitol_protocol::rest::API_KEY_PREFIX as SECRET_PREFIX;

/// How much of the secret the owner's list shows.
const SHOWN_PREFIX_LEN: usize = 12;

/// Live (unrevoked) keys an account may hold at once.
pub const MAX_KEYS: i64 = 10;

/// What a key was minted with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scopes {
    read: bool,
    play: bool,
    trade: bool,
    town: bool,
}

impl Scopes {
    pub fn of(scopes: &[ApiKeyScope]) -> Scopes {
        let play = scopes.contains(&ApiKeyScope::Play);
        let trade = scopes.contains(&ApiKeyScope::Trade);
        Scopes {
            read: play || trade || scopes.contains(&ApiKeyScope::Read),
            play,
            trade,
            town: scopes.contains(&ApiKeyScope::Town),
        }
    }

    pub fn allows(self, needed: ApiKeyScope) -> bool {
        match needed {
            ApiKeyScope::Read => self.read,
            ApiKeyScope::Play => self.play,
            ApiKeyScope::Trade => self.trade,
            ApiKeyScope::Town => self.town,
        }
    }

    /// Whether it's the town's report key alone (D4.1), which needs no
    /// subscription.
    pub fn town_only(self) -> bool {
        self.town && !self.read
    }

    /// Whether a WebSocket on this key takes the player over. A read-only
    /// key only watches.
    pub fn takes_presence(self) -> bool {
        self.play || self.trade
    }
}

/// A key the request or socket was authenticated with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyGrant {
    pub id: Uuid,
    pub scopes: Scopes,
}

/// The account behind a good key.
#[derive(Debug, Clone)]
pub struct KeyUser {
    pub account_id: Uuid,
    pub grant: KeyGrant,
}

/// What a secret signs in as.
#[derive(Debug, Clone)]
pub enum KeyAuth {
    /// A live key, for an account that may run a bot
    Live(KeyUser),
    /// No live key has this secret: it's unknown or revoked
    Unknown,
    /// A person's key whose subscription isn't running (#217)
    Unsubscribed(Subscription),
}

#[cfg(test)]
impl KeyAuth {
    pub fn live(self) -> Option<KeyUser> {
        match self {
            KeyAuth::Live(user) => Some(user),
            _ => None,
        }
    }
}

/// Why a key whose owner isn't subscribed was refused, for the bot to show.
pub fn unsubscribed(subscription: &Subscription) -> String {
    match subscription.status {
        SubscriptionStatus::Lapsed => {
            format!("Bots need a subscription. {}", crate::subscriptions::refusal(subscription))
        }
        _ => "Bots need a subscription.".into(),
    }
}

/// A new secret.
pub fn new_secret() -> String {
    format!("{SECRET_PREFIX}{}", crate::sessions::random_token())
}

pub fn shown_prefix(secret: &str) -> String {
    secret.chars().take(SHOWN_PREFIX_LEN).collect()
}

pub fn is_secret(token: &str) -> bool {
    token.starts_with(SECRET_PREFIX)
}

/// The scope a REST request needs, or `None` when no key may make it.
/// `path` is the full path, before any `nest` stripped it. Anything not
/// listed here is refused to keys, so a new route stays closed to them until
/// it's given a scope.
pub fn rest_scope(method: &Method, path: &str) -> Option<ApiKeyScope> {
    let path = path.trim_end_matches('/');
    if method == Method::POST && path == "/admin/town/report" {
        return Some(ApiKeyScope::Town);
    }
    let mut parts = path.trim_start_matches('/').split('/');
    let group = parts.next().unwrap_or("");
    if matches!(group, "account" | "admin" | "auth") {
        return None;
    }
    if method == Method::GET {
        return Some(ApiKeyScope::Read);
    }
    let rest: Vec<&str> = parts.collect();
    match (method.as_str(), group, rest.as_slice()) {
        ("POST", "player", ["move"])
        | ("POST", "world", ["extract"])
        | ("POST", "inventory", ["move"])
        | ("POST", "stations", ["place"])
        | ("DELETE", "stations", [_])
        | ("PUT", "stations", [_, "automation"])
        | ("POST", "stations", [_, "automation", "start" | "stop"]) => Some(ApiKeyScope::Play),
        ("POST", "exchange", ["deposit" | "withdraw"])
        | ("POST", "market", ["orders"] | ["box", "collect"])
        | ("DELETE", "market", ["orders", _])
        | ("POST", "bazaar", ["stalls", _, "rent"] | ["mine", "listings"] | ["listings", _, "buy"])
        | ("DELETE", "bazaar", ["listings", _])
        | ("POST", "plots", [_, "claim" | "pay-tax"])
        | ("PUT", "plots", [_, "permissions"]) => Some(ApiKeyScope::Trade),
        _ => None,
    }
}

/// The scope a WebSocket command needs.
pub fn ws_scope(message: &ClientMessage) -> ApiKeyScope {
    match message {
        ClientMessage::Auth { .. }
        | ClientMessage::Subscribe { .. }
        | ClientMessage::SubscribeMarket { .. }
        | ClientMessage::UnsubscribeMarket { .. } => ApiKeyScope::Read,
        // A `read` key may ask, but `engine::presence` refuses it: it only watches.
        ClientMessage::ClaimPresence => ApiKeyScope::Read,
        ClientMessage::Move { .. }
        | ClientMessage::Steer { .. }
        | ClientMessage::Extract { .. }
        | ClientMessage::CancelExtraction
        | ClientMessage::Craft { .. }
        | ClientMessage::CancelCrafting
        | ClientMessage::MoveItem { .. }
        | ClientMessage::SplitStack { .. }
        | ClientMessage::PlaceStation { .. }
        | ClientMessage::RemoveStation { .. }
        | ClientMessage::OpenStation { .. }
        | ClientMessage::CloseStation
        | ClientMessage::CraftAtStation { .. }
        | ClientMessage::InstallTool { .. }
        | ClientMessage::RemoveTool { .. }
        | ClientMessage::ConfigureAutomation { .. }
        | ClientMessage::StartAutomation { .. }
        | ClientMessage::StopAutomation { .. }
        // A citizen talks on its Play key (Commons S1.1)
        | ClientMessage::Say { .. } => ApiKeyScope::Play,
        ClientMessage::TradeInitiate { .. }
        | ClientMessage::TradeAcceptRequest
        | ClientMessage::TradeDeclineRequest
        | ClientMessage::TradeOffer { .. }
        | ClientMessage::TradeAccept
        | ClientMessage::TradeCancel => ApiKeyScope::Trade,
    }
}

/// Why a key was refused something, for the `error` a client shows.
pub fn refusal(what: &str, needed: Option<ApiKeyScope>) -> String {
    match needed {
        Some(scope) => format!("This API key can't {what}: it needs the `{}` scope", scope.as_str()),
        None => format!("An API key can't {what}"),
    }
}

/// The columns [`info`] reads, in its order.
pub const INFO_COLUMNS: &str = "id, name, scopes, prefix, created_at, last_used_at, revoked_at";

pub type InfoRow = (Uuid, String, Vec<String>, String, DateTime<Utc>, Option<DateTime<Utc>>, Option<DateTime<Utc>>);

/// A key as its owner's list shows it, from [`INFO_COLUMNS`].
pub fn info((id, name, scopes, prefix, created_at, last_used_at, revoked_at): InfoRow) -> ApiKeyInfo {
    ApiKeyInfo {
        id,
        name,
        scopes: scopes.iter().filter_map(|s| ApiKeyScope::parse(s)).collect(),
        prefix,
        created_at,
        last_used_at,
        revoked_at,
    }
}

/// Mint a key for `account_id` with `scopes` (checked by the caller). Returns
/// it and its secret, which nothing keeps.
pub async fn mint(
    db: impl sqlx::PgExecutor<'_>,
    account_id: Uuid,
    name: &str,
    scopes: &[ApiKeyScope],
) -> Result<(ApiKeyInfo, String), sqlx::Error> {
    let secret = new_secret();
    let row: InfoRow = sqlx::query_as(&format!(
        "INSERT INTO api_keys (id, account_id, name, scopes, secret_hash, prefix)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING {INFO_COLUMNS}"
    ))
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(name)
    .bind(scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>())
    .bind(crate::sessions::hash(&secret))
    .bind(shown_prefix(&secret))
    .fetch_one(db)
    .await?;
    Ok((info(row), secret))
}

/// The account a secret belongs to, if it's a live key and the account may
/// run a bot. Notes the use, at most once a minute.
pub async fn authenticate(db: &PgPool, secret: &str) -> Result<KeyAuth, sqlx::Error> {
    type Row = (Uuid, Uuid, Vec<String>, Option<DateTime<Utc>>, String, String, Option<DateTime<Utc>>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT k.id, k.account_id, k.scopes, k.last_used_at,
                a.kind, a.subscription_status, a.subscription_until
         FROM api_keys k
         JOIN accounts a ON a.id = k.account_id
         WHERE k.secret_hash = $1 AND k.revoked_at IS NULL",
    )
    .bind(crate::sessions::hash(secret))
    .fetch_optional(db)
    .await?;
    let Some((id, account_id, scopes, last_used_at, kind, status, until)) = row else {
        return Ok(KeyAuth::Unknown);
    };
    let now = Utc::now();
    let scopes = Scopes::of(&scopes.iter().filter_map(|s| ApiKeyScope::parse(s)).collect::<Vec<_>>());
    let subscription = Subscription { status: SubscriptionStatus::parse(&status), until }.as_of(now);
    let needs_subscription = AccountKind::parse(&kind) != Some(AccountKind::Citizen) && !scopes.town_only();
    if needs_subscription && !subscription.is_active_at(now) {
        return Ok(KeyAuth::Unsubscribed(subscription));
    }
    if last_used_at.is_none_or(|at| now - at > chrono::Duration::minutes(1)) {
        sqlx::query("UPDATE api_keys SET last_used_at = $2 WHERE id = $1")
            .bind(id)
            .bind(now)
            .execute(db)
            .await?;
    }
    Ok(KeyAuth::Live(KeyUser {
        account_id,
        grant: KeyGrant { id, scopes },
    }))
}

/// The name every town key is minted with
pub const TOWN_KEY_NAME: &str = "town reports";

/// Mint the town's report key (D4.1) for `account_id`, an admin's, revoking
/// any town key it held before: a town has one key, and a new one is how a
/// lost one is replaced. The secret is in the answer only.
pub async fn mint_town_key(db: &PgPool, account_id: Uuid) -> Result<(ApiKeyInfo, String), sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "UPDATE api_keys SET revoked_at = NOW()
         WHERE account_id = $1 AND scopes = ARRAY['town'] AND revoked_at IS NULL",
    )
    .bind(account_id)
    .execute(&mut *tx)
    .await?;
    let minted = mint(&mut *tx, account_id, TOWN_KEY_NAME, &[ApiKeyScope::Town]).await?;
    tx.commit().await?;
    Ok(minted)
}

/// The account's unrevoked keys, to hang up when its subscription lapses.
pub async fn live_keys(db: impl sqlx::PgExecutor<'_>, account_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM api_keys WHERE account_id = $1 AND revoked_at IS NULL")
        .bind(account_id)
        .fetch_all(db)
        .await
}

/// Whether key `id` is still unrevoked.
pub async fn is_live(db: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM api_keys WHERE id = $1 AND revoked_at IS NULL)")
        .bind(id)
        .fetch_one(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ApiKeyScope::*;

    fn rest(method: Method, path: &str) -> Option<ApiKeyScope> {
        rest_scope(&method, path)
    }

    #[test]
    fn reads_need_read_and_commands_their_scope() {
        assert_eq!(rest(Method::GET, "/player/me"), Some(Read));
        assert_eq!(rest(Method::GET, "/plots/"), Some(Read));
        assert_eq!(rest(Method::GET, "/stations/abc/container"), Some(Read));
        assert_eq!(rest(Method::POST, "/player/move"), Some(Play));
        assert_eq!(rest(Method::POST, "/world/extract"), Some(Play));
        assert_eq!(rest(Method::POST, "/inventory/move"), Some(Play));
        assert_eq!(rest(Method::POST, "/stations/place"), Some(Play));
        assert_eq!(rest(Method::DELETE, "/stations/abc"), Some(Play));
        assert_eq!(rest(Method::PUT, "/stations/abc/automation"), Some(Play));
        assert_eq!(rest(Method::POST, "/stations/abc/automation/start"), Some(Play));
        assert_eq!(rest(Method::POST, "/stations/abc/automation/stop"), Some(Play));
        assert_eq!(rest(Method::POST, "/stations/abc/automation/pause"), None);
        assert_eq!(rest(Method::POST, "/exchange/deposit"), Some(Trade));
        assert_eq!(rest(Method::POST, "/exchange/withdraw"), Some(Trade));
        assert_eq!(rest(Method::POST, "/plots/abc/claim"), Some(Trade));
        assert_eq!(rest(Method::POST, "/plots/abc/pay-tax"), Some(Trade));
        assert_eq!(rest(Method::PUT, "/plots/abc/permissions"), Some(Trade));
        assert_eq!(rest(Method::GET, "/market/orders"), Some(Read));
        assert_eq!(rest(Method::POST, "/market/orders"), Some(Trade));
        assert_eq!(rest(Method::DELETE, "/market/orders/abc"), Some(Trade));
        assert_eq!(rest(Method::POST, "/market/box/collect"), Some(Trade));
        assert_eq!(rest(Method::POST, "/market/box"), None);
        assert_eq!(rest(Method::GET, "/bazaar"), Some(Read));
        assert_eq!(rest(Method::POST, "/bazaar/stalls/bazaar_stall_1/rent"), Some(Trade));
        assert_eq!(rest(Method::POST, "/bazaar/mine/listings"), Some(Trade));
        assert_eq!(rest(Method::POST, "/bazaar/listings/abc/buy"), Some(Trade));
        assert_eq!(rest(Method::DELETE, "/bazaar/listings/abc"), Some(Trade));
    }

    #[test]
    fn keys_never_reach_accounts_admin_or_unlisted_routes() {
        assert_eq!(rest(Method::GET, "/account/api-keys"), None);
        assert_eq!(rest(Method::POST, "/account/api-keys"), None);
        assert_eq!(rest(Method::DELETE, "/account/api-keys/abc"), None);
        assert_eq!(rest(Method::GET, "/admin/stats"), None);
        assert_eq!(rest(Method::PUT, "/admin/world-params"), None);
        // A key can't mint citizens, nor so much as list them (C0.1)
        assert_eq!(rest(Method::POST, "/admin/citizens"), None);
        assert_eq!(rest(Method::GET, "/admin/citizens"), None);
        assert_eq!(rest(Method::POST, "/auth/logout"), None);
        assert_eq!(rest(Method::POST, "/player/teleport"), None);
        // Subscribing is the account's own business, as paying will be
        assert_eq!(rest(Method::POST, "/player/subscription"), None);
        assert_eq!(rest(Method::PATCH, "/player/move"), None);
    }

    #[test]
    fn the_town_key_reaches_its_report_and_nothing_else() {
        assert_eq!(rest(Method::POST, "/admin/town/report"), Some(Town));
        assert_eq!(rest(Method::POST, "/admin/town/report/"), Some(Town));
        assert_eq!(rest(Method::GET, "/admin/town"), None);
        assert_eq!(rest(Method::POST, "/admin/town/key"), None);
        let town = Scopes::of(&[Town]);
        assert!(town.allows(Town) && town.town_only());
        assert!(!town.allows(Read) && !town.allows(Play) && !town.allows(Trade));
        assert!(!town.takes_presence());
        // Nor does any other scope reach it
        let all = Scopes::of(&[Read, Play, Trade]);
        assert!(!all.allows(Town) && !all.town_only());
    }

    #[test]
    fn play_and_trade_imply_read_but_not_each_other() {
        let read = Scopes::of(&[Read]);
        assert!(read.allows(Read) && !read.allows(Play) && !read.allows(Trade));
        assert!(!read.takes_presence());
        let play = Scopes::of(&[Play]);
        assert!(play.allows(Read) && play.allows(Play) && !play.allows(Trade));
        assert!(play.takes_presence());
        let trade = Scopes::of(&[Trade]);
        assert!(trade.allows(Read) && !trade.allows(Play) && trade.allows(Trade));
    }

    #[test]
    fn a_read_key_can_watch_but_not_move() {
        let read = Scopes::of(&[Read]);
        let subscribe = ClientMessage::Subscribe { channel: "position".into() };
        assert!(read.allows(ws_scope(&subscribe)));
        assert!(!read.allows(ws_scope(&ClientMessage::Move { x: 1.0, y: 2.0 })));
        assert!(!read.allows(ws_scope(&ClientMessage::TradeAccept)));
        let play = Scopes::of(&[Play]);
        assert!(play.allows(ws_scope(&ClientMessage::Move { x: 1.0, y: 2.0 })));
        assert!(play.allows(ws_scope(&ClientMessage::Steer { heading: 0, throttle: 10, at: None })));
        assert!(!read.allows(ws_scope(&ClientMessage::Steer { heading: 0, throttle: 0, at: None })));
        assert!(!play.allows(ws_scope(&ClientMessage::TradeAccept)));
        // A citizen talks on its Play key (Commons S1.1)
        let say = ClientMessage::Say { channel: capitol_protocol::ws::Channel::Local, text: "Hello".into() };
        assert!(play.allows(ws_scope(&say)) && !read.allows(ws_scope(&say)));
    }

    #[test]
    fn secrets_are_prefixed_random_and_shown_by_their_start() {
        let (a, b) = (new_secret(), new_secret());
        assert!(is_secret(&a) && a.len() == SECRET_PREFIX.len() + 64);
        assert_ne!(a, b);
        assert_eq!(shown_prefix(&a).len(), SHOWN_PREFIX_LEN);
        assert!(a.starts_with(&shown_prefix(&a)));
        assert!(!is_secret("eyJhbGciOiJIUzI1NiJ9.e30.x"), "a JWT isn't a key");
    }

    async fn account(db: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(id)
            .bind(format!("{id}@test.local"))
            .execute(db)
            .await
            .unwrap();
        id
    }

    async fn mint(db: &PgPool, account: Uuid, scopes: &[&str]) -> (Uuid, String) {
        let (id, secret) = (Uuid::new_v4(), new_secret());
        sqlx::query(
            "INSERT INTO api_keys (id, account_id, name, scopes, secret_hash, prefix)
             VALUES ($1, $2, 'bot', $3, $4, $5)",
        )
        .bind(id)
        .bind(account)
        .bind(scopes)
        .bind(crate::sessions::hash(&secret))
        .bind(shown_prefix(&secret))
        .execute(db)
        .await
        .unwrap();
        (id, secret)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_live_key_signs_in_as_its_account_until_revoked(db: PgPool) {
        let ada = account(&db).await;
        crate::subscriptions::extend(&db, ada, 30).await.unwrap();
        let (id, secret) = mint(&db, ada, &["read"]).await;

        let user = authenticate(&db, &secret).await.unwrap().live().expect("a live key");
        assert_eq!(user.account_id, ada);
        assert_eq!(user.grant, KeyGrant { id, scopes: Scopes::of(&[Read]) });
        let used: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT last_used_at FROM api_keys WHERE id = $1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert!(used.is_some(), "the use is noted");
        assert!(is_live(&db, id).await.unwrap());

        assert!(matches!(authenticate(&db, &new_secret()).await.unwrap(), KeyAuth::Unknown), "an unknown secret");
        sqlx::query("UPDATE api_keys SET revoked_at = NOW() WHERE id = $1").bind(id).execute(&db).await.unwrap();
        assert!(matches!(authenticate(&db, &secret).await.unwrap(), KeyAuth::Unknown), "revoked");
        assert!(!is_live(&db, id).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_persons_key_signs_in_only_while_they_subscribe(db: PgPool) {
        let ada = account(&db).await;
        let (_, secret) = mint(&db, ada, &["play"]).await;
        let refused = |auth: KeyAuth| match auth {
            KeyAuth::Unsubscribed(subscription) => unsubscribed(&subscription),
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert_eq!(refused(authenticate(&db, &secret).await.unwrap()), "Bots need a subscription.");

        crate::subscriptions::extend(&db, ada, 30).await.unwrap();
        assert!(authenticate(&db, &secret).await.unwrap().live().is_some(), "subscribed");

        // Past its end it's refused at once, before the hourly check marks it
        sqlx::query("UPDATE accounts SET subscription_until = NOW() - interval '1 hour' WHERE id = $1")
            .bind(ada)
            .execute(&db)
            .await
            .unwrap();
        let reason = refused(authenticate(&db, &secret).await.unwrap());
        assert!(reason.starts_with("Bots need a subscription. Your subscription lapsed on "), "{reason}");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_citizens_key_needs_no_subscription(db: PgPool) {
        let made = crate::citizens::mint(&db, "Wren Cooper").await.unwrap();
        let user = authenticate(&db, &made.secret).await.unwrap().live().expect("a citizen plays unsubscribed");
        assert_eq!(user.account_id, made.citizen.account_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_town_key_needs_no_subscription_and_replaces_the_last(db: PgPool) {
        let clerk = account(&db).await;
        let (first, first_secret) = mint_town_key(&db, clerk).await.unwrap();
        assert_eq!(first.scopes, vec![Town]);
        let user = authenticate(&db, &first_secret).await.unwrap().live().expect("a town key needs no subscription");
        assert!(user.grant.scopes.town_only());
        // A bot's key on the same account still does
        let (_, bot) = mint(&db, clerk, &["read"]).await;
        assert!(matches!(authenticate(&db, &bot).await.unwrap(), KeyAuth::Unsubscribed(_)));

        let (_, second_secret) = mint_town_key(&db, clerk).await.unwrap();
        assert!(matches!(authenticate(&db, &first_secret).await.unwrap(), KeyAuth::Unknown), "the old one is revoked");
        assert!(authenticate(&db, &second_secret).await.unwrap().live().is_some());
    }

    #[test]
    fn a_lapsed_key_says_when() {
        let until = "2026-11-17T12:00:00Z".parse().unwrap();
        let lapsed = Subscription { status: SubscriptionStatus::Lapsed, until: Some(until) };
        assert_eq!(
            unsubscribed(&lapsed),
            "Bots need a subscription. Your subscription lapsed on 17 November 2026. Renew it to use this."
        );
        assert_eq!(unsubscribed(&Subscription::default()), "Bots need a subscription.");
    }

    #[test]
    fn refusals_name_the_scope() {
        assert_eq!(
            refusal("move", Some(Play)),
            "This API key can't move: it needs the `play` scope"
        );
        assert_eq!(refusal("GET /account/api-keys", None), "An API key can't GET /account/api-keys");
    }
}
