//! The session's credentials, kept fresh (E0.6). The server issues a short
//! access token with a single-use refresh token, and the [`Keyring`] swaps
//! the refresh token for a new pair:
//! - **before expiry**, at four fifths of the access token's life (at most a
//!   minute early), from a timer and before any request that finds it due;
//! - **on a 401**, once, then the request is sent again;
//! - **for a WebSocket (re)connect**, which takes [`Keyring::token`] each time.
//!
//! Refreshes are one at a time behind a lock. Two at once would spend the
//! same refresh token twice, and the server takes a second use as theft and
//! ends the session.

use std::sync::{mpsc, Arc};
use std::time::Duration;

use capitol_protocol::rest::RefreshRequest;
use tokio::sync::Mutex;
use tokio::time::Instant;

use super::rest::{self, RestError, RestRequest, RestResponse};
use super::{Credentials, NetEvent};

/// The wait before trying again after a refresh the server didn't answer.
const RETRY_AFTER: Duration = Duration::from_secs(5);

pub(crate) struct Keyring {
    held: Mutex<Held>,
    http: reqwest::Client,
    base: String,
    events: mpsc::Sender<NetEvent>,
}

struct Held {
    access: Option<String>,
    /// `None` without one (a legacy login), or once the server refused it.
    refresh: Option<String>,
    /// When to refresh; `None` when the expiry isn't known.
    refresh_at: Option<Instant>,
    /// After a failed refresh, none before this.
    retry_at: Option<Instant>,
    /// The server's reason, when it refused the refresh token.
    refused: Option<String>,
}

impl Held {
    fn due(&self, now: Instant) -> bool {
        self.refresh.is_some()
            && (self.access.is_none() || self.refresh_at.is_some_and(|at| now >= at))
            && self.retry_at.is_none_or(|at| now >= at)
    }
}

/// When to refresh a token good for `ttl`: at four fifths of its life, but
/// no more than a minute before it expires.
fn refresh_after(ttl: Duration) -> Duration {
    (ttl * 4 / 5).max(ttl.saturating_sub(Duration::from_secs(60)))
}

impl Keyring {
    pub(crate) fn new(
        credentials: Credentials,
        http: reqwest::Client,
        base: String,
        events: mpsc::Sender<NetEvent>,
    ) -> Arc<Keyring> {
        let now = Instant::now();
        Arc::new(Keyring {
            held: Mutex::new(Held {
                access: credentials.token,
                refresh: credentials.refresh_token,
                refresh_at: credentials.expires_in.map(|ttl| now + refresh_after(ttl)),
                retry_at: None,
                refused: None,
            }),
            http,
            base,
            events,
        })
    }

    /// The access token to send, refreshed first if it's due.
    pub(crate) async fn token(&self) -> Option<String> {
        let mut held = self.held.lock().await;
        if held.due(Instant::now()) {
            self.refresh(&mut held).await;
        }
        held.access.clone()
    }

    /// `used` was refused: a fresh token to try once more with, or `None`
    /// when there's no getting one. If another request has refreshed since
    /// `used` was handed out, that token is the answer.
    pub(crate) async fn renew(&self, used: Option<&str>) -> Option<String> {
        let mut held = self.held.lock().await;
        if held.access.is_some() && held.access.as_deref() != used {
            return held.access.clone();
        }
        held.retry_at = None;
        if held.refresh.is_some() && self.refresh(&mut held).await {
            return held.access.clone();
        }
        None
    }

    /// Why the server refused the refresh token, once it has.
    pub(crate) async fn refused(&self) -> Option<String> {
        self.held.lock().await.refused.clone()
    }

    /// When the timer should next refresh, or `None` for never.
    pub(crate) async fn next_refresh(&self) -> Option<Instant> {
        let held = self.held.lock().await;
        held.refresh.as_ref()?;
        // With no access token yet (a session resumed from disk), now.
        let at = match (&held.access, held.refresh_at) {
            (None, _) => Instant::now(),
            (Some(_), at) => at?,
        };
        Some(held.retry_at.map_or(at, |retry| retry.max(at)))
    }

    async fn refresh(&self, held: &mut Held) -> bool {
        let Some(refresh_token) = held.refresh.clone() else {
            return false;
        };
        let request = RestRequest::Refresh(RefreshRequest { refresh_token });
        match rest::execute(&self.http, &self.base, None, &request).await {
            Ok(RestResponse::Auth(auth)) => {
                let now = Instant::now();
                held.access = Some(auth.token);
                held.refresh = auth.refresh_token.clone();
                held.refresh_at = Some(now + refresh_after(Duration::from_secs(auth.expires_in)));
                held.retry_at = None;
                tracing::debug!("net: refreshed the session's token");
                if let Some(refresh_token) = auth.refresh_token {
                    let _ = self.events.send(NetEvent::Refreshed { refresh_token });
                }
                true
            }
            // Refused: the token expired, or the session ended. It won't
            // work again, and requests made with the old access token will
            // get their own 401s.
            Err(RestError::Http { status, message }) if (400..500).contains(&status) => {
                tracing::info!("net: the server refused the refresh token: {message}");
                held.refresh = None;
                held.refused = Some(message);
                false
            }
            Err(e) => {
                tracing::warn!("net: refreshing the session's token failed: {e}");
                held.retry_at = Some(Instant::now() + RETRY_AFTER);
                false
            }
            Ok(other) => {
                tracing::warn!("net: unexpected answer to a refresh: {other:?}");
                held.retry_at = Some(Instant::now() + RETRY_AFTER);
                false
            }
        }
    }
}

/// Refreshes on time even when nothing else is asking. Ends once there's
/// nothing left to refresh.
pub(crate) async fn keep_fresh(keyring: Arc<Keyring>) {
    while let Some(at) = keyring.next_refresh().await {
        tokio::time::sleep_until(at).await;
        keyring.token().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_at_four_fifths_or_a_minute_early() {
        let s = Duration::from_secs;
        assert_eq!(refresh_after(s(120)), s(96));
        assert_eq!(refresh_after(s(900)), s(840));
        assert_eq!(refresh_after(s(0)), s(0));
    }
}
