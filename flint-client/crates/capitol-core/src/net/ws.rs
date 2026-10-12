//! One WebSocket session: connect, authenticate, subscribe, then pump
//! messages both ways until the socket closes.
//!
//! The server's side of the handshake is in `backend/src/ws/connection.rs`
//! (`handle_socket`): it waits 10 s for `auth`, answers `auth_ok` or
//! `auth_error` (and hangs up), and drops every game event until the client
//! has sent `subscribe{channel:"position"}`. Messages the game sends before
//! that are held here and flushed once `subscribed` arrives.

use std::collections::VecDeque;
use std::sync::mpsc;

use capitol_protocol::ws::{ClientMessage, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc as tmpsc;
use tokio::task::JoinHandle;
use tokio::time::{self, Instant, MissedTickBehavior};
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

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
        token: String,
        config: NetConfig,
        events: mpsc::Sender<NetEvent>,
    ) -> Session {
        let (outbound, rx) = tmpsc::unbounded_channel();
        let task = tokio::spawn(run(url, token, config, rx, events));
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

/// How the session ended.
enum End {
    Disconnected(String),
    AuthFailed(String),
}

async fn run(
    url: String,
    token: String,
    config: NetConfig,
    outbound: tmpsc::UnboundedReceiver<Outbound>,
    events: mpsc::Sender<NetEvent>,
) {
    let emit = |state| {
        let _ = events.send(NetEvent::Connection(state));
    };
    emit(ConnectionState::Connecting);
    let end = session(&url, token, &config, outbound, &events, &emit).await;
    match end {
        End::Disconnected(reason) => {
            tracing::info!("websocket disconnected: {reason}");
            emit(ConnectionState::Disconnected { reason });
        }
        End::AuthFailed(message) => {
            tracing::warn!("websocket auth failed: {message}");
            emit(ConnectionState::AuthFailed { message });
        }
    }
}

async fn session(
    url: &str,
    token: String,
    config: &NetConfig,
    mut outbound: tmpsc::UnboundedReceiver<Outbound>,
    events: &mpsc::Sender<NetEvent>,
    emit: &impl Fn(ConnectionState),
) -> End {
    let connect = async {
        let tcp = connect_tcp(url).await?;
        tokio_tungstenite::client_async_tls(url, tcp)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))
    };
    let socket = match time::timeout(config.connect_timeout, connect).await {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(e)) => return End::Disconnected(format!("could not connect to {url}: {e}")),
        Err(_) => return End::Disconnected(format!("could not connect to {url}: timed out")),
    };
    let (mut sink, mut stream) = socket.split();

    emit(ConnectionState::Authenticating);
    if let Err(e) = sink.send(encode(&ClientMessage::Auth { token })).await {
        return End::Disconnected(format!("send failed: {e}"));
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
                    Some(Err(e)) => return End::Disconnected(format!("connection lost: {e}")),
                    None => return End::Disconnected("connection closed by server".into()),
                };
                last_heard = Instant::now();
                let text = match frame {
                    Message::Text(text) => text,
                    Message::Close(frame) => {
                        let reason = frame
                            .map(|f| f.reason.to_string())
                            .filter(|r| !r.is_empty())
                            .unwrap_or_else(|| "connection closed by server".into());
                        return End::Disconnected(reason);
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
                    (Phase::Authenticating, ServerMessage::AuthOk { player_id }) => {
                        let subscribe = ClientMessage::Subscribe { channel: POSITION_CHANNEL.into() };
                        if let Err(e) = sink.send(encode(&subscribe)).await {
                            return End::Disconnected(format!("send failed: {e}"));
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
                    return End::AuthFailed(message);
                }
                if let Some(player_id) = became_live {
                    phase = Phase::Live;
                    emit(ConnectionState::Connected { player_id });
                    while let Some(message) = pending.pop_front() {
                        if let Err(e) = sink.send(encode(&message)).await {
                            return End::Disconnected(format!("send failed: {e}"));
                        }
                    }
                }
            }

            out = outbound.recv() => match out {
                Some(Outbound::Send(message)) => {
                    if matches!(phase, Phase::Live) {
                        if let Err(e) = sink.send(encode(&message)).await {
                            return End::Disconnected(format!("send failed: {e}"));
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
                    return End::Disconnected("closed by client".into());
                }
            },

            _ = ping.tick() => {
                // The server has no idle timeout of its own, so a half-open
                // socket would otherwise look connected forever.
                if last_heard.elapsed() > config.idle_timeout {
                    return End::Disconnected("server stopped responding".into());
                }
                if let Err(e) = sink.send(Message::Ping(Default::default())).await {
                    return End::Disconnected(format!("send failed: {e}"));
                }
            }

            _ = &mut handshake_deadline, if !matches!(phase, Phase::Live) => {
                return End::Disconnected("handshake timed out".into());
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
