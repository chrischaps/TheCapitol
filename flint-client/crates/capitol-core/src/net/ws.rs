//! One WebSocket session: connect, authenticate, subscribe, then pump
//! messages both ways. If the socket drops, wait and do it all again, until
//! the game closes the session or the server rejects the token.
//!
//! The server's side of the handshake is in `backend/src/ws/connection.rs`
//! (`handle_socket`): it waits 10 s for `auth`, answers `auth_ok` or
//! `auth_error` (and hangs up), and drops every game event until the client
//! has sent `subscribe{channel:"position"}`. Messages the game sends before
//! that are held here and flushed once `subscribed` arrives.

use std::collections::hash_map::RandomState;
use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use capitol_protocol::ws::{ClientMessage, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc as tmpsc;
use tokio::task::JoinHandle;
use tokio::time::{self, Instant, MissedTickBehavior};
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use super::keyring::Keyring;
use super::{ConnectionState, NetConfig, NetEvent};

/// The only channel the server knows.
const POSITION_CHANNEL: &str = "position";

/// Messages held while the handshake runs. A game that queues more than this
/// before it's connected has a bug; the oldest are dropped.
const MAX_PENDING: usize = 256;

/// How long to wait for the server's reply to our Close frame.
const CLOSE_GRACE: time::Duration = time::Duration::from_secs(1);

pub(crate) enum Outbound {
    Send(ClientMessage),
    Close,
}

/// The IO thread's handle on a running session.
pub(crate) struct Session {
    outbound: tmpsc::UnboundedSender<Outbound>,
    task: JoinHandle<()>,
}

impl Session {
    pub(crate) fn spawn(
        url: String,
        keyring: Arc<Keyring>,
        config: NetConfig,
        events: mpsc::Sender<NetEvent>,
    ) -> Session {
        let (outbound, rx) = tmpsc::unbounded_channel();
        let task = tokio::spawn(run(url, keyring, config, rx, events));
        Session { outbound, task }
    }

    pub(crate) fn send(&self, message: ClientMessage) {
        if self.outbound.send(Outbound::Send(message)).is_err() {
            tracing::warn!("websocket session has ended; message dropped");
        }
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Asks the session to close and waits for it, so its final
    /// `Connection` event lands before anything a new session emits.
    pub(crate) async fn close(self) {
        let _ = self.outbound.send(Outbound::Close);
        let mut task = self.task;
        if time::timeout(CLOSE_GRACE * 2, &mut task).await.is_err() {
            task.abort();
        }
    }
}

enum Phase {
    /// `auth` sent, waiting for `auth_ok`.
    Authenticating,
    /// `subscribe` sent, waiting for `subscribed`.
    Subscribing {
        player_id: Uuid,
    },
    Live,
}

/// How one connection ended.
enum End {
    /// The game asked to close.
    Closed,
    /// Lost, refused or timed out. Worth retrying.
    Dropped(String),
    /// The server rejected the token.
    AuthFailed(String),
}

const CLOSED_BY_CLIENT: &str = "closed by client";

async fn run(
    url: String,
    keyring: Arc<Keyring>,
    config: NetConfig,
    mut outbound: tmpsc::UnboundedReceiver<Outbound>,
    events: mpsc::Sender<NetEvent>,
) {
    let emit = |state| {
        let _ = events.send(NetEvent::Connection(state));
    };
    let closed = || {
        tracing::info!("websocket {CLOSED_BY_CLIENT}");
        emit(ConnectionState::Disconnected {
            reason: CLOSED_BY_CLIENT.into(),
        });
    };
    let mut attempt = 0;
    // Once a connection has gone live, the game holds state that the next
    // one must refresh.
    let mut was_live = false;
    // The last connection's token was refused and a fresh one got: if that
    // one is refused too, it's final.
    let mut renewed = false;
    let auth_failed = |message: String| {
        tracing::warn!("websocket auth failed: {message}");
        emit(ConnectionState::AuthFailed { message });
    };

    loop {
        emit(ConnectionState::Connecting);
        // Each attempt takes the keyring's token, refreshed if it's due.
        let Some(token) = keyring.token().await else {
            let refused = keyring.refused().await;
            return auth_failed(refused.unwrap_or_else(|| "Session expired".into()));
        };
        let mut live_since = None;
        let end = connection(
            &url,
            &token,
            &config,
            &mut outbound,
            &events,
            &emit,
            was_live,
            &mut live_since,
        )
        .await;
        if live_since.is_some() {
            renewed = false;
        }
        let reason = match end {
            End::Closed => return closed(),
            End::AuthFailed(message) => {
                // An expired token, say after the machine slept through the
                // refresh: refresh and go again at once.
                if !renewed && keyring.renew(Some(&token)).await.is_some() {
                    tracing::info!("websocket token refused ({message}); refreshed it");
                    renewed = true;
                    continue;
                }
                return auth_failed(keyring.refused().await.unwrap_or(message));
            }
            End::Dropped(reason) => reason,
        };

        was_live |= live_since.is_some();
        if live_since.is_some_and(|t: Instant| t.elapsed() >= config.reconnect_max) {
            attempt = 0;
        }
        attempt += 1;
        let delay = backoff(
            attempt,
            config.reconnect_initial,
            config.reconnect_max,
            jitter(),
        );
        tracing::info!("websocket dropped ({reason}); attempt {attempt} in {delay:?}");
        emit(ConnectionState::Reconnecting {
            attempt,
            delay,
            reason,
        });

        // Wait, but stay responsive to the game closing the session.
        let wait = time::sleep(delay);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                out = outbound.recv() => match out {
                    Some(Outbound::Send(message)) => {
                        tracing::warn!("reconnecting; dropping {}", message.type_name());
                    }
                    Some(Outbound::Close) | None => return closed(),
                },
            }
        }
    }
}

/// The wait before reconnect attempt `attempt` (counting from 1): `initial`,
/// doubled for each attempt after the first, capped at `max`. `jitter`, in
/// [0, 1), then trims up to a fifth off, so clients the server dropped
/// together don't all come back on the same tick.
pub(crate) fn backoff(attempt: u32, initial: Duration, max: Duration, jitter: f64) -> Duration {
    let doublings = attempt.saturating_sub(1).min(31);
    let nominal = initial.saturating_mul(1 << doublings).min(max);
    nominal.mul_f64(1.0 - 0.2 * jitter.clamp(0.0, 1.0))
}

/// A number in [0, 1). `RandomState` is seeded randomly, which is plenty for
/// spreading out reconnects and saves a dependency.
pub(crate) fn jitter() -> f64 {
    let bits = RandomState::new().build_hasher().finish();
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

/// Whether an `auth_error` means the token is no good. The server also sends
/// `auth_error` when `auth` didn't arrive within 10 s, or when the database
/// failed while loading the player (`backend/src/ws/connection.rs`,
/// `handle_socket`), or when it couldn't reach Redis to check the session
/// (`SESSIONS_UNAVAILABLE` in `backend/src/error.rs`, #75). None of these
/// says anything about the token, so they're retried.
pub(crate) fn auth_error_is_final(message: &str) -> bool {
    !(message == "Auth timeout"
        || message.starts_with("Database error")
        || message == "Session store unavailable")
}

/// One connection: connect, handshake, then pump until it ends. Sets
/// `live_since` when `subscribed` arrives, and sends [`NetEvent::Resync`]
/// straight after `Connected` if `resync` is set.
#[allow(clippy::too_many_arguments)]
async fn connection(
    url: &str,
    token: &str,
    config: &NetConfig,
    outbound: &mut tmpsc::UnboundedReceiver<Outbound>,
    events: &mpsc::Sender<NetEvent>,
    emit: &impl Fn(ConnectionState),
    resync: bool,
    live_since: &mut Option<Instant>,
) -> End {
    let connect = async {
        let tcp = connect_tcp(url).await?;
        tokio_tungstenite::client_async_tls(url, tcp)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))
    };
    let socket = match time::timeout(config.connect_timeout, connect).await {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(e)) => return End::Dropped(format!("could not connect to {url}: {e}")),
        Err(_) => return End::Dropped(format!("could not connect to {url}: timed out")),
    };
    let (mut sink, mut stream) = socket.split();

    emit(ConnectionState::Authenticating);
    let auth = ClientMessage::Auth {
        token: token.into(),
    };
    if let Err(e) = sink.send(encode(&auth)).await {
        return End::Dropped(format!("send failed: {e}"));
    }

    let mut phase = Phase::Authenticating;
    let mut pending: VecDeque<ClientMessage> = VecDeque::new();
    let handshake_deadline = time::sleep(config.handshake_timeout);
    tokio::pin!(handshake_deadline);
    let mut ping = time::interval_at(Instant::now() + config.ping_interval, config.ping_interval);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_heard = Instant::now();

    loop {
        tokio::select! {
            frame = stream.next() => {
                let frame = match frame {
                    Some(Ok(frame)) => frame,
                    Some(Err(e)) => return End::Dropped(format!("connection lost: {e}")),
                    None => return End::Dropped("connection closed by server".into()),
                };
                last_heard = Instant::now();
                let text = match frame {
                    Message::Text(text) => text,
                    Message::Close(frame) => {
                        let reason = frame
                            .map(|f| f.reason.to_string())
                            .filter(|r| !r.is_empty())
                            .unwrap_or_else(|| "connection closed by server".into());
                        return End::Dropped(reason);
                    }
                    // Pongs only refresh `last_heard`; tungstenite answers pings itself.
                    _ => continue,
                };
                let message = match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(message) => message,
                    Err(e) => {
                        let excerpt: String = text.chars().take(120).collect();
                        tracing::warn!("ignoring malformed server message ({e}): {excerpt}");
                        continue;
                    }
                };

                // Advance the handshake. The message itself is forwarded either way.
                let mut became_live = None;
                let mut auth_failed = None;
                match (&phase, &message) {
                    (Phase::Authenticating, ServerMessage::AuthOk { player_id, .. }) => {
                        let subscribe = ClientMessage::Subscribe { channel: POSITION_CHANNEL.into() };
                        if let Err(e) = sink.send(encode(&subscribe)).await {
                            return End::Dropped(format!("send failed: {e}"));
                        }
                        phase = Phase::Subscribing { player_id: *player_id };
                    }
                    (Phase::Authenticating, ServerMessage::AuthError { message }) => {
                        auth_failed = Some(message.clone());
                    }
                    (Phase::Subscribing { player_id }, ServerMessage::Subscribed { channel })
                        if channel == POSITION_CHANNEL =>
                    {
                        became_live = Some(*player_id);
                    }
                    _ => {}
                }
                let _ = events.send(NetEvent::Ws(message));

                if let Some(message) = auth_failed {
                    return if auth_error_is_final(&message) {
                        End::AuthFailed(message)
                    } else {
                        End::Dropped(format!("auth_error: {message}"))
                    };
                }
                if let Some(player_id) = became_live {
                    phase = Phase::Live;
                    *live_since = Some(Instant::now());
                    emit(ConnectionState::Connected { player_id });
                    if resync {
                        let _ = events.send(NetEvent::Resync);
                    }
                    while let Some(message) = pending.pop_front() {
                        if let Err(e) = sink.send(encode(&message)).await {
                            return End::Dropped(format!("send failed: {e}"));
                        }
                    }
                }
            }

            out = outbound.recv() => match out {
                Some(Outbound::Send(message)) => {
                    if matches!(phase, Phase::Live) {
                        if let Err(e) = sink.send(encode(&message)).await {
                            return End::Dropped(format!("send failed: {e}"));
                        }
                    } else {
                        if pending.len() == MAX_PENDING {
                            tracing::warn!("too many messages queued before the handshake finished; dropping the oldest");
                            pending.pop_front();
                        }
                        pending.push_back(message);
                    }
                }
                Some(Outbound::Close) | None => {
                    let _ = sink.send(Message::Close(None)).await;
                    // Finish the closing handshake before dropping the socket. On
                    // Windows, closing a socket with unread data sends a reset, and
                    // the server can lose our Close frame with it.
                    let drain = async { while let Some(Ok(_)) = stream.next().await {} };
                    let _ = time::timeout(CLOSE_GRACE, drain).await;
                    return End::Closed;
                }
            },

            _ = ping.tick() => {
                // The server has no idle timeout of its own, so a half-open
                // socket would otherwise look connected forever.
                if last_heard.elapsed() > config.idle_timeout {
                    return End::Dropped("server stopped responding".into());
                }
                if let Err(e) = sink.send(Message::Ping(Default::default())).await {
                    return End::Dropped(format!("send failed: {e}"));
                }
            }

            _ = &mut handshake_deadline, if !matches!(phase, Phase::Live) => {
                return End::Dropped("handshake timed out".into());
            }
        }
    }
}

/// Connects to every address the host resolves to at once and keeps the first
/// to answer. `localhost` resolves to `::1` first, the backend listens on IPv4
/// only, and Windows takes about 2 s to give up on a refused address, so
/// trying them in order made every connect 2 s slower.
async fn connect_tcp(url: &str) -> std::io::Result<TcpStream> {
    let uri: Uri = url.parse().map_err(std::io::Error::other)?;
    let host = uri
        .host()
        .ok_or_else(|| std::io::Error::other("URL has no host"))?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("wss") {
            443
        } else {
            80
        });

    let attempts: Vec<_> = tokio::net::lookup_host((host, port))
        .await?
        .map(|addr| Box::pin(TcpStream::connect(addr)))
        .collect();
    if attempts.is_empty() {
        return Err(std::io::Error::other(format!("{host} did not resolve")));
    }
    let (stream, _) = futures_util::future::select_ok(attempts).await?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

fn encode(message: &ClientMessage) -> Message {
    // ClientMessage is plain data; serialising it can't fail.
    Message::text(serde_json::to_string(message).expect("ClientMessage serialises"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_half_a_second_to_ten() {
        let config = NetConfig::default();
        let at = |attempt| backoff(attempt, config.reconnect_initial, config.reconnect_max, 0.0);
        let secs: Vec<f64> = (1..=8).map(|n| at(n).as_secs_f64()).collect();
        assert_eq!(secs, [0.5, 1.0, 2.0, 4.0, 8.0, 10.0, 10.0, 10.0]);
        // Far past the cap, nothing overflows.
        assert_eq!(at(u32::MAX), config.reconnect_max);
    }

    #[test]
    fn jitter_only_shortens_the_wait_by_up_to_a_fifth() {
        let (initial, max) = (Duration::from_millis(500), Duration::from_secs(10));
        assert_eq!(backoff(1, initial, max, 1.0), Duration::from_millis(400));
        for _ in 0..1000 {
            let j = jitter();
            assert!((0.0..1.0).contains(&j));
            let delay = backoff(6, initial, max, j);
            assert!(delay > Duration::from_secs(8) && delay <= max, "{delay:?}");
        }
    }

    #[test]
    fn only_token_problems_are_final() {
        assert!(auth_error_is_final("Invalid token"));
        assert!(auth_error_is_final("Session expired"));
        assert!(auth_error_is_final("Player not found"));
        assert!(!auth_error_is_final("Auth timeout"));
        assert!(!auth_error_is_final("Session store unavailable"));
        assert!(!auth_error_is_final(
            "Database error: pool timed out while waiting for an open connection"
        ));
    }
}
