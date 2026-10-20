//! Sessions and refresh tokens, in Redis (E0.6).
//!
//! An account can have several sessions at once (E0.8, D9): one per login,
//! each named by a random session id (`sid`) and alive while
//! `session:{account}:{sid}` is. `sessions:{account}` indexes them by when
//! they started, so a login past [`MAX_SESSIONS`] ends the oldest and a "log
//! out everywhere" finds them all. Every access token carries its session's
//! id, and a token is good while its session is. So a logout or a revoked
//! family ends every token of that session at once and leaves the account's
//! others alone, while a refresh, which keeps the id, leaves tokens already
//! in flight working until they expire.
//!
//! Which session drives the avatar isn't decided here: that's presence,
//! which one socket holds (`engine::presence`).
//!
//! A session started with a refresh token is a refresh **family**:
//! - `refresh:{sha256(token)}` holds `{account}:{sid}`. Only the hash is kept.
//! - Using a refresh token marks it used (`refresh_used:{hash}`, set with
//!   `NX`, so of two concurrent uses only one wins) and issues the next.
//! - A refresh token used twice was copied. The second use ends the family:
//!   the session goes, and with it every access and refresh token it issued.
//! - Except within a short grace (#184): for `REUSE_GRACE_MS` after a token's
//!   first use, presenting it again returns the same successor, so a client
//!   whose refresh answer was lost (or that sent two at once) isn't signed
//!   out. The successor is kept in plain at `refresh_grace:{hash}` for that
//!   window only. Once the successor has been used itself, the client plainly
//!   got it, and the old token's grace is over.

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// A legacy session's lifetime, and its access token's: the React and Love2D
/// clients log in without asking for a refresh token and keep the 24 h
/// tokens they always had (until #185).
pub const LEGACY_TTL_SECS: u64 = 24 * 60 * 60;

/// How long after a refresh token's first use presenting it again returns
/// the same successor instead of revoking the family (#184).
pub const REUSE_GRACE_MS: u64 = 30_000;

/// The most sessions an account keeps. A login past it ends the oldest.
pub const MAX_SESSIONS: usize = 10;

/// A session just started.
#[derive(Debug)]
pub struct Started {
    pub sid: String,
    /// `None` for a legacy session.
    pub refresh_token: Option<String>,
    /// Older sessions this one pushed past [`MAX_SESSIONS`], now ended.
    pub evicted: Vec<String>,
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
    Reused { account_id: Uuid, sid: String },
}

#[derive(Debug, thiserror::Error)]
pub enum RotateError {
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error(transparent)]
    Redis(#[from] redis::RedisError),
}

fn session_key(account_id: Uuid, sid: &str) -> String {
    format!("session:{account_id}:{sid}")
}

/// The account's sessions, each scored by when it started (Unix ms).
fn index_key(account_id: Uuid) -> String {
    format!("sessions:{account_id}")
}

fn refresh_key(hash: &str) -> String {
    format!("refresh:{hash}")
}

fn used_key(hash: &str) -> String {
    format!("refresh_used:{hash}")
}

fn grace_key(hash: &str) -> String {
    format!("refresh_grace:{hash}")
}

pub(crate) fn hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 32 random bytes, as hex: an opaque token or a session id.
pub(crate) fn random_token() -> String {
    hex(&rand::random::<[u8; 32]>())
}

/// Starts a new session for the account, beside any it has, and ends the
/// oldest past [`MAX_SESSIONS`]. With `refresh_ttl`, it's a refresh family
/// whose first token is returned, and the session lives as long as its
/// refresh tokens do; without, it's a legacy session.
pub async fn start(
    redis: &mut ConnectionManager,
    account_id: Uuid,
    refresh_ttl: Option<u64>,
) -> redis::RedisResult<Started> {
    let sid = random_token();
    let ttl = refresh_ttl.unwrap_or(LEGACY_TTL_SECS);
    let index = index_key(account_id);
    redis.set_ex::<_, _, ()>(session_key(account_id, &sid), 1, ttl).await?;
    redis
        .zadd::<_, _, _, ()>(&index, &sid, chrono::Utc::now().timestamp_millis())
        .await?;
    keep_index(redis, account_id, ttl).await?;

    // Sessions that expired on their own still sit in the index. Drop them
    // before counting, so they don't push out a live one.
    let mut live = Vec::new();
    for other in redis.zrange::<_, Vec<String>>(&index, 0, -1).await? {
        if redis.exists(session_key(account_id, &other)).await? {
            live.push(other);
        } else {
            redis.zrem::<_, _, ()>(&index, &other).await?;
        }
    }
    let mut evicted = Vec::new();
    for old in &live[..live.len().saturating_sub(MAX_SESSIONS)] {
        if end(redis, account_id, old).await? {
            evicted.push(old.clone());
        }
    }

    let refresh_token = match refresh_ttl {
        Some(ttl) => Some(issue(redis, account_id, &sid, ttl).await?),
        None => None,
    };
    Ok(Started { sid, refresh_token, evicted })
}

/// Keeps the account's index alive at least as long as a session just
/// started or renewed.
async fn keep_index(redis: &mut ConnectionManager, account_id: Uuid, ttl: u64) -> redis::RedisResult<()> {
    let index = index_key(account_id);
    let left: i64 = redis.ttl(&index).await?;
    if left < ttl as i64 {
        redis.expire::<_, ()>(&index, ttl as i64).await?;
    }
    Ok(())
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

/// Whether `sid` is still one of the account's sessions.
pub async fn is_current(redis: &mut ConnectionManager, account_id: Uuid, sid: &str) -> redis::RedisResult<bool> {
    if sid.is_empty() {
        return Ok(false);
    }
    redis.exists(session_key(account_id, sid)).await
}

/// Swaps a refresh token for the next one in its family, and keeps the
/// session alive for another `ttl`.
pub async fn rotate(redis: &mut ConnectionManager, token: &str, ttl: u64) -> Result<Rotated, RotateError> {
    rotate_with_grace(redis, token, ttl, REUSE_GRACE_MS).await
}

/// Marks a token used and issues its successor, or, for a token already
/// used, answers the successor it got while the grace lasts. All in one
/// script, so of two uses at once the second always finds the first's
/// successor. A nil answer is a reuse.
///
/// KEYS: the token's used marker, its grace entry, the successor's
/// `refresh:` key. ARGV: ttl (s), grace (ms), the successor's hash, the
/// successor, `{account}:{sid}`.
const ROTATE: &str = r"
if redis.call('SET', KEYS[1], 1, 'NX', 'EX', ARGV[1]) then
    redis.call('SET', KEYS[3], ARGV[5], 'EX', ARGV[1])
    redis.call('SET', KEYS[2], ARGV[3] .. ':' .. ARGV[4], 'PX', ARGV[2])
    return ARGV[4]
end
local grace = redis.call('GET', KEYS[2])
if not grace then
    return false
end
local hash, successor = string.match(grace, '^(%x+):(%x+)$')
if redis.call('EXISTS', 'refresh_used:' .. hash) == 1 then
    return false
end
return successor
";

async fn rotate_with_grace(
    redis: &mut ConnectionManager,
    token: &str,
    ttl: u64,
    grace_ms: u64,
) -> Result<Rotated, RotateError> {
    let Some((account_id, sid)) = lookup(redis, token).await? else {
        return Err(Refused::Unknown.into());
    };
    if !is_current(redis, account_id, &sid).await? {
        return Err(Refused::Ended.into());
    }
    let (token_hash, next) = (hash(token), random_token());
    let next_hash = hash(&next);
    let successor: Option<String> = redis::Script::new(ROTATE)
        .key(used_key(&token_hash))
        .key(grace_key(&token_hash))
        .key(refresh_key(&next_hash))
        .arg(ttl)
        .arg(grace_ms)
        .arg(&next_hash)
        .arg(&next)
        .arg(format!("{account_id}:{sid}"))
        .invoke_async(redis)
        .await?;
    let Some(refresh_token) = successor else {
        end(redis, account_id, &sid).await?;
        return Err(Refused::Reused { account_id, sid }.into());
    };
    redis.expire::<_, ()>(session_key(account_id, &sid), ttl as i64).await?;
    keep_index(redis, account_id, ttl).await?;
    Ok(Rotated {
        account_id,
        sid,
        refresh_token,
    })
}

/// Ends the account's session `sid`, leaving its others alone. Returns
/// whether it was still live.
pub async fn end(redis: &mut ConnectionManager, account_id: Uuid, sid: &str) -> redis::RedisResult<bool> {
    if sid.is_empty() {
        return Ok(false);
    }
    let ended: i64 = redis.del(session_key(account_id, sid)).await?;
    redis.zrem::<_, _, ()>(index_key(account_id), sid).await?;
    Ok(ended > 0)
}

/// Ends every session the account has: "log out everywhere". Returns the
/// ones that were live.
pub async fn end_all(redis: &mut ConnectionManager, account_id: Uuid) -> redis::RedisResult<Vec<String>> {
    let mut ended = Vec::new();
    for sid in redis.zrange::<_, Vec<String>>(index_key(account_id), 0, -1).await? {
        if end(redis, account_id, &sid).await? {
            ended.push(sid);
        }
    }
    redis.del::<_, ()>(index_key(account_id)).await?;
    Ok(ended)
}

/// Ends the session a refresh token belongs to, used or not. Returns its
/// account and id, if it was live.
pub async fn end_by_refresh(redis: &mut ConnectionManager, token: &str) -> redis::RedisResult<Option<(Uuid, String)>> {
    let Some((account_id, sid)) = lookup(redis, token).await? else {
        return Ok(None);
    };
    Ok(end(redis, account_id, &sid).await?.then_some((account_id, sid)))
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
    async fn a_replay_within_the_grace_gets_the_same_successor() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let started = start(&mut redis, account, Some(TTL)).await.unwrap();
        let first = started.refresh_token.unwrap();
        let rotated = rotate(&mut redis, &first, TTL).await.unwrap();

        // The answer was lost, so the client sends the old token again.
        let replayed = rotate(&mut redis, &first, TTL).await.unwrap();
        assert_eq!(replayed, rotated, "the same successor, the same session");
        assert!(is_current(&mut redis, account, &started.sid).await.unwrap());
        // And the successor rotates as usual.
        rotate(&mut redis, &rotated.refresh_token, TTL).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_replay_after_the_grace_is_refused_and_revokes_the_family() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let started = start(&mut redis, account, Some(TTL)).await.unwrap();
        let first = started.refresh_token.unwrap();
        let second = rotate_with_grace(&mut redis, &first, TTL, 200).await.unwrap().refresh_token;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        assert_eq!(
            refused(rotate(&mut redis, &first, TTL).await),
            Refused::Reused { account_id: account, sid: started.sid.clone() }
        );
        assert!(!is_current(&mut redis, account, &started.sid).await.unwrap());
        // The thief's token is gone with the family, and so is the owner's.
        assert_eq!(refused(rotate(&mut redis, &second, TTL).await), Refused::Ended);
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_replay_once_the_successor_is_used_revokes_the_family() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let started = start(&mut redis, account, Some(TTL)).await.unwrap();
        let first = started.refresh_token.unwrap();
        let second = rotate(&mut redis, &first, TTL).await.unwrap().refresh_token;
        rotate(&mut redis, &second, TTL).await.unwrap();

        // Still within the grace, but the client got its answer: this is a copy.
        assert_eq!(
            refused(rotate(&mut redis, &first, TTL).await),
            Refused::Reused { account_id: account, sid: started.sid.clone() }
        );
        assert!(!is_current(&mut redis, account, &started.sid).await.unwrap());
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn two_uses_at_once_get_the_same_successor() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let token = start(&mut redis, account, Some(TTL)).await.unwrap().refresh_token.unwrap();
        let (mut a, mut b) = (redis.clone(), redis.clone());
        let (x, y) = tokio::join!(rotate(&mut a, &token, TTL), rotate(&mut b, &token, TTL));
        assert_eq!(x.unwrap(), y.unwrap(), "one rotation, answered twice");
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_second_login_leaves_the_first_alone_and_a_logout_ends_only_its_own() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let old = start(&mut redis, account, Some(TTL)).await.unwrap();
        let new = start(&mut redis, account, Some(TTL)).await.unwrap();
        assert!(new.evicted.is_empty());
        // Both are live, and both refresh (E0.8).
        let old_token = rotate(&mut redis, old.refresh_token.as_ref().unwrap(), TTL)
            .await
            .unwrap()
            .refresh_token;
        assert!(is_current(&mut redis, account, &new.sid).await.unwrap());

        // The old session's logout leaves the new one alone.
        assert!(end(&mut redis, account, &old.sid).await.unwrap());
        assert!(!end(&mut redis, account, &old.sid).await.unwrap(), "ended once only");
        assert_eq!(refused(rotate(&mut redis, &old_token, TTL).await), Refused::Ended);
        assert!(is_current(&mut redis, account, &new.sid).await.unwrap());

        let token = new.refresh_token.unwrap();
        assert_eq!(end_by_refresh(&mut redis, &token).await.unwrap(), Some((account, new.sid)));
        assert_eq!(refused(rotate(&mut redis, &token, TTL).await), Refused::Ended);
        assert_eq!(refused(rotate(&mut redis, "nonsense", TTL).await), Refused::Unknown);
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_reuse_revokes_only_its_own_family() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let phone = start(&mut redis, account, Some(TTL)).await.unwrap();
        let desk = start(&mut redis, account, Some(TTL)).await.unwrap();
        let first = phone.refresh_token.unwrap();
        let second = rotate(&mut redis, &first, TTL).await.unwrap().refresh_token;
        rotate(&mut redis, &second, TTL).await.unwrap();
        assert_eq!(
            refused(rotate(&mut redis, &first, TTL).await),
            Refused::Reused { account_id: account, sid: phone.sid.clone() }
        );
        assert!(!is_current(&mut redis, account, &phone.sid).await.unwrap());
        assert!(
            is_current(&mut redis, account, &desk.sid).await.unwrap(),
            "the other device stays signed in"
        );
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn a_login_past_the_cap_ends_the_oldest() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let mut sids = Vec::new();
        for _ in 0..MAX_SESSIONS {
            let started = start(&mut redis, account, Some(TTL)).await.unwrap();
            assert!(started.evicted.is_empty());
            sids.push(started.sid);
            // Distinct start times, so the oldest is plain.
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        let newest = start(&mut redis, account, Some(TTL)).await.unwrap();
        assert_eq!(newest.evicted, vec![sids[0].clone()]);
        assert!(!is_current(&mut redis, account, &sids[0]).await.unwrap());
        assert!(is_current(&mut redis, account, &sids[1]).await.unwrap());
        assert!(is_current(&mut redis, account, &newest.sid).await.unwrap());
    }

    #[tokio::test]
    #[ignore = "needs the local Redis (docker-compose up -d)"]
    async fn logging_out_everywhere_ends_every_session() {
        let mut redis = redis().await;
        let account = Uuid::new_v4();
        let a = start(&mut redis, account, Some(TTL)).await.unwrap();
        let b = start(&mut redis, account, None).await.unwrap();
        let mut ended = end_all(&mut redis, account).await.unwrap();
        ended.sort();
        let mut expected = vec![a.sid.clone(), b.sid.clone()];
        expected.sort();
        assert_eq!(ended, expected);
        assert!(!is_current(&mut redis, account, &a.sid).await.unwrap());
        assert!(!is_current(&mut redis, account, &b.sid).await.unwrap());
        assert_eq!(
            refused(rotate(&mut redis, a.refresh_token.as_ref().unwrap(), TTL).await),
            Refused::Ended
        );
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
