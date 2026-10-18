//! Sessions and refresh tokens, in Redis (E0.6).
//!
//! An account has at most one session (#78), named by a random session id
//! (`sid`) at `session:{account}`. Every access token carries its session's
//! id, and a token is good while its id is still the account's session. So a
//! new login, a logout or a revoked family ends every token of the old
//! session at once, while a refresh, which keeps the id, leaves tokens
//! already in flight working until they expire.
//!
//! A session started with a refresh token is a refresh **family**:
//! - `refresh:{sha256(token)}` holds `{account}:{sid}`. Only the hash is kept.
//! - Using a refresh token marks it used (`refresh_used:{hash}`, set with
//!   `NX`, so of two concurrent uses only one wins) and issues the next.
//! - A refresh token used twice was copied. The second use ends the family:
//!   the session goes, and with it every access and refresh token it issued.
//!   There's no grace, so a client that never got a refresh's answer is
//!   signed out by its next one (#184).

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// A legacy session's lifetime, and its access token's: the React and Love2D
/// clients log in without asking for a refresh token and keep the 24 h
/// tokens they always had (until #185).
pub const LEGACY_TTL_SECS: u64 = 24 * 60 * 60;

/// A session just started.
#[derive(Debug)]
pub struct Started {
    pub sid: String,
    /// `None` for a legacy session.
    pub refresh_token: Option<String>,
}

/// A refresh token swapped for the next.
#[derive(Debug, PartialEq)]
pub struct Rotated {
    pub account_id: Uuid,
    pub sid: String,
    pub refresh_token: String,
}

/// Why a refresh token was refused.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum Refused {
    /// Never issued, or older than its TTL.
    #[error("Invalid refresh token")]
    Unknown,
    /// Its session has ended: a logout, a newer login, or a reuse.
    #[error("Session expired")]
    Ended,
    /// Already used once. Its family has now been revoked.
    #[error("Refresh token reused")]
    Reused { account_id: Uuid },
}

#[derive(Debug, thiserror::Error)]
pub enum RotateError {
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error(transparent)]
    Redis(#[from] redis::RedisError),
}

fn session_key(account_id: Uuid) -> String {
    format!("session:{account_id}")
}

fn refresh_key(hash: &str) -> String {
    format!("refresh:{hash}")
}

fn used_key(hash: &str) -> String {
    format!("refresh_used:{hash}")
}

fn hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 32 random bytes, as hex: an opaque token or a session id.
fn random_token() -> String {
    hex(&rand::random::<[u8; 32]>())
}

/// Starts the account's session, replacing any it had. With `refresh_ttl`,
/// it's a refresh family whose first token is returned, and the session
/// lives as long as its refresh tokens do; without, it's a legacy session.
pub async fn start(
    redis: &mut ConnectionManager,
    account_id: Uuid,
    refresh_ttl: Option<u64>,
) -> redis::RedisResult<Started> {
    let sid = random_token();
    let ttl = refresh_ttl.unwrap_or(LEGACY_TTL_SECS);
    redis.set_ex::<_, _, ()>(session_key(account_id), &sid, ttl).await?;
    let refresh_token = match refresh_ttl {
        Some(ttl) => Some(issue(redis, account_id, &sid, ttl).await?),
        None => None,
    };
    Ok(Started { sid, refresh_token })
}

async fn issue(
    redis: &mut ConnectionManager,
    account_id: Uuid,
    sid: &str,
    ttl: u64,
) -> redis::RedisResult<String> {
    let token = random_token();
    redis
        .set_ex::<_, _, ()>(refresh_key(&hash(&token)), format!("{account_id}:{sid}"), ttl)
        .await?;
    Ok(token)
}

/// The account and session a refresh token was issued to, used or not.
async fn lookup(redis: &mut ConnectionManager, token: &str) -> redis::RedisResult<Option<(Uuid, String)>> {
    let entry: Option<String> = redis.get(refresh_key(&hash(token))).await?;
    Ok(entry.and_then(|entry| {
        let (account, sid) = entry.split_once(':')?;
        Some((account.parse().ok()?, sid.to_owned()))
    }))
}

/// Whether `sid` is still the account's session.
pub async fn is_current(redis: &mut ConnectionManager, account_id: Uuid, sid: &str) -> redis::RedisResult<bool> {
    let current: Option<String> = redis.get(session_key(account_id)).await?;
    Ok(!sid.is_empty() && current.as_deref() == Some(sid))
}

/// Swaps a refresh token for the next one in its family, and keeps the
/// session alive for another `ttl`.
pub async fn rotate(redis: &mut ConnectionManager, token: &str, ttl: u64) -> Result<Rotated, RotateError> {
    let Some((account_id, sid)) = lookup(redis, token).await? else {
        return Err(Refused::Unknown.into());
    };
    if !is_current(redis, account_id, &sid).await? {
        return Err(Refused::Ended.into());
    }
    // SET NX answers OK for the first use only.
    let first: Option<String> = redis::cmd("SET")
        .arg(used_key(&hash(token)))
        .arg(1)
        .arg("NX")
        .arg("EX")
        .arg(ttl)
        .query_async(redis)
        .await?;
    if first.is_none() {
        end(redis, account_id, &sid).await?;
        return Err(Refused::Reused { account_id }.into());
    }
    redis.expire::<_, ()>(session_key(account_id), ttl as i64).await?;
    let refresh_token = issue(redis, account_id, &sid, ttl).await?;
    Ok(Rotated {
        account_id,
        sid,
        refresh_token,
    })
}

/// Ends the account's session if it's still `sid`, so an old client's logout
/// can't end a newer login's session. Returns whether it ended one.
pub async fn end(redis: &mut ConnectionManager, account_id: Uuid, sid: &str) -> redis::RedisResult<bool> {
    if !is_current(redis, account_id, sid).await? {
        return Ok(false);
    }
    redis.del::<_, ()>(session_key(account_id)).await?;
    Ok(true)
}

/// Ends the session a refresh token belongs to, used or not.
pub async fn end_by_refresh(redis: &mut ConnectionManager, token: &str) -> redis::RedisResult<Option<Uuid>> {
    let Some((account_id, sid)) = lookup(redis, token).await? else {
        return Ok(None);
    };
    Ok(end(redis, account_id, &sid).await?.then_some(account_id))
}

/// The `#[ignore]`d tests need the local Redis (`docker-compose up -d`), or
/// `REDIS_URL`. Run them with `cargo test sessions -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;

    const TTL: u64 = 60;

    async fn redis() -> ConnectionManager {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
        crate::state::connect_redis(&url).await.expect("Redis is up")
    }

    fn refused(result: Result<Rotated, RotateError>) -> Refused {
        match result {
            Err(RotateError::Refused(why)) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn tokens_are_random_and_only_their_hash_is_kept() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(hash(&a), hash(&a));
        assert_ne!(hash(&a), a);
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_refresh_rotates_the_token_and_keeps_the_session() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let started = start(&mut redis, account, Some(TTL)).await.unwrap();
        let first = started.refresh_token.unwrap();

        let rotated = rotate(&mut redis, &first, TTL).await.unwrap();
        assert_eq!(rotated.account_id, account);
        assert_eq!(rotated.sid, started.sid, "the session lives on");
        assert_ne!(rotated.refresh_token, first);
        assert!(is_current(&mut redis, account, &started.sid).await.unwrap());

        // And the next one rotates in turn.
        let again = rotate(&mut redis, &rotated.refresh_token, TTL).await.unwrap();
        assert_eq!(again.sid, started.sid);
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_replayed_token_is_refused_and_revokes_the_family() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let started = start(&mut redis, account, Some(TTL)).await.unwrap();
        let first = started.refresh_token.unwrap();
        let second = rotate(&mut redis, &first, TTL).await.unwrap().refresh_token;

        assert_eq!(
            refused(rotate(&mut redis, &first, TTL).await),
            Refused::Reused { account_id: account }
        );
        assert!(!is_current(&mut redis, account, &started.sid).await.unwrap());
        // The thief's token is gone with the family, and so is the owner's.
        assert_eq!(refused(rotate(&mut redis, &second, TTL).await), Refused::Ended);
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn two_uses_at_once_let_only_one_through() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let token = start(&mut redis, account, Some(TTL)).await.unwrap().refresh_token.unwrap();
        let (mut a, mut b) = (redis.clone(), redis.clone());
        let (x, y) = tokio::join!(rotate(&mut a, &token, TTL), rotate(&mut b, &token, TTL));
        assert_eq!(x.is_ok() as u8 + y.is_ok() as u8, 1, "{x:?} {y:?}");
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_new_login_or_a_logout_ends_the_family() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let old = start(&mut redis, account, Some(TTL)).await.unwrap();
        let new = start(&mut redis, account, Some(TTL)).await.unwrap();
        assert_eq!(
            refused(rotate(&mut redis, old.refresh_token.as_ref().unwrap(), TTL).await),
            Refused::Ended
        );
        // The old session's logout leaves the new one alone.
        assert!(!end(&mut redis, account, &old.sid).await.unwrap());
        assert!(is_current(&mut redis, account, &new.sid).await.unwrap());

        let token = new.refresh_token.unwrap();
        assert_eq!(end_by_refresh(&mut redis, &token).await.unwrap(), Some(account));
        assert_eq!(refused(rotate(&mut redis, &token, TTL).await), Refused::Ended);
        assert_eq!(refused(rotate(&mut redis, "nonsense", TTL).await), Refused::Unknown);
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_legacy_session_has_no_refresh_token() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let started = start(&mut redis, account, None).await.unwrap();
        assert!(started.refresh_token.is_none());
        assert!(is_current(&mut redis, account, &started.sid).await.unwrap());
        assert!(!is_current(&mut redis, account, "").await.unwrap());
    }
}
