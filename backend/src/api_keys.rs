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
//! can't mint or revoke keys) or `/admin`.
//!
//! A key isn't a session: it doesn't replace the account's login (#78), so a
//! bot on a key leaves its owner signed in over REST. Over the WebSocket a
//! `play` or `trade` key does take the player over, as a second login's
//! socket would, until #151 decides how many presences an account may have.
//!
//! The secret is `cap_` and 64 hex digits, shown once. Only its SHA-256 is
//! kept. A revoked key is refused from its next request, and its sockets are
//! closed (`GameState::revoke_key`).

use axum::http::Method;
use capitol_protocol::rest::ApiKeyScope;
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
    play: bool,
    trade: bool,
}

impl Scopes {
    pub fn of(scopes: &[ApiKeyScope]) -> Scopes {
        Scopes {
            play: scopes.contains(&ApiKeyScope::Play),
            trade: scopes.contains(&ApiKeyScope::Trade),
        }
    }

    pub fn allows(self, needed: ApiKeyScope) -> bool {
        match needed {
            ApiKeyScope::Read => true,
            ApiKeyScope::Play => self.play,
            ApiKeyScope::Trade => self.trade,
        }
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
        | ("DELETE", "stations", [_]) => Some(ApiKeyScope::Play),
        ("POST", "exchange", ["deposit" | "withdraw"])
        | ("POST", "plots", [_, "claim" | "pay-tax"])
        | ("PUT", "plots", [_, "permissions"]) => Some(ApiKeyScope::Trade),
        _ => None,
    }
}

/// The scope a WebSocket command needs.
pub fn ws_scope(message: &ClientMessage) -> ApiKeyScope {
    match message {
        ClientMessage::Auth { .. } | ClientMessage::Subscribe { .. } => ApiKeyScope::Read,
        ClientMessage::Move { .. }
        | ClientMessage::Extract { .. }
        | ClientMessage::CancelExtraction
        | ClientMessage::Craft { .. }
        | ClientMessage::CancelCrafting
        | ClientMessage::MoveItem { .. }
        | ClientMessage::PlaceStation { .. }
        | ClientMessage::RemoveStation { .. }
        | ClientMessage::OpenStation { .. }
        | ClientMessage::CloseStation
        | ClientMessage::CraftAtStation { .. }
        | ClientMessage::InstallTool { .. }
        | ClientMessage::RemoveTool { .. } => ApiKeyScope::Play,
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

/// The account a secret belongs to, if it's a live key. Notes the use, at
/// most once a minute.
pub async fn authenticate(db: &PgPool, secret: &str) -> Result<Option<KeyUser>, sqlx::Error> {
    let row: Option<(Uuid, Uuid, Vec<String>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT k.id, k.account_id, k.scopes, k.last_used_at
         FROM api_keys k
         WHERE k.secret_hash = $1 AND k.revoked_at IS NULL",
    )
    .bind(crate::sessions::hash(secret))
    .fetch_optional(db)
    .await?;
    let Some((id, account_id, scopes, last_used_at)) = row else {
        return Ok(None);
    };
    let now = Utc::now();
    if last_used_at.is_none_or(|at| now - at > chrono::Duration::minutes(1)) {
        sqlx::query("UPDATE api_keys SET last_used_at = $2 WHERE id = $1")
            .bind(id)
            .bind(now)
            .execute(db)
            .await?;
    }
    let scopes: Vec<ApiKeyScope> = scopes.iter().filter_map(|s| ApiKeyScope::parse(s)).collect();
    Ok(Some(KeyUser {
        account_id,
        grant: KeyGrant {
            id,
            scopes: Scopes::of(&scopes),
        },
    }))
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
        assert_eq!(rest(Method::POST, "/exchange/deposit"), Some(Trade));
        assert_eq!(rest(Method::POST, "/exchange/withdraw"), Some(Trade));
        assert_eq!(rest(Method::POST, "/plots/abc/claim"), Some(Trade));
        assert_eq!(rest(Method::POST, "/plots/abc/pay-tax"), Some(Trade));
        assert_eq!(rest(Method::PUT, "/plots/abc/permissions"), Some(Trade));
    }

    #[test]
    fn keys_never_reach_accounts_admin_or_unlisted_routes() {
        assert_eq!(rest(Method::GET, "/account/api-keys"), None);
        assert_eq!(rest(Method::POST, "/account/api-keys"), None);
        assert_eq!(rest(Method::DELETE, "/account/api-keys/abc"), None);
        assert_eq!(rest(Method::GET, "/admin/stats"), None);
        assert_eq!(rest(Method::PUT, "/admin/world-params"), None);
        assert_eq!(rest(Method::POST, "/auth/logout"), None);
        assert_eq!(rest(Method::POST, "/player/teleport"), None);
        // Subscribing is the account's own business, as paying will be
        assert_eq!(rest(Method::POST, "/player/subscription"), None);
        assert_eq!(rest(Method::PATCH, "/player/move"), None);
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
        assert!(!play.allows(ws_scope(&ClientMessage::TradeAccept)));
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
        let (id, secret) = mint(&db, ada, &["read"]).await;

        let user = authenticate(&db, &secret).await.unwrap().expect("a live key");
        assert_eq!(user.account_id, ada);
        assert_eq!(user.grant, KeyGrant { id, scopes: Scopes::of(&[Read]) });
        let used: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT last_used_at FROM api_keys WHERE id = $1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert!(used.is_some(), "the use is noted");
        assert!(is_live(&db, id).await.unwrap());

        assert!(authenticate(&db, &new_secret()).await.unwrap().is_none(), "an unknown secret");
        sqlx::query("UPDATE api_keys SET revoked_at = NOW() WHERE id = $1").bind(id).execute(&db).await.unwrap();
        assert!(authenticate(&db, &secret).await.unwrap().is_none(), "revoked");
        assert!(!is_live(&db, id).await.unwrap());
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
