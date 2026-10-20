//! The IO thread. A tokio runtime runs on one background thread with
//! `reqwest` for REST and `tokio-tungstenite` for the WebSocket. The game
//! talks to it through [`Net`]: [`Command`]s go in, [`NetEvent`]s come out,
//! and neither side ever blocks the other.
//!
//! ```no_run
//! # use capitol_core::net::*;
//! # use capitol_core::protocol::rest::LoginRequest;
//! let net = Net::spawn(NetConfig::default()).unwrap();
//! net.rest(RestRequest::Login(LoginRequest { email: "a@b.c".into(), password: "pw".into(), refresh: true }), 1);
//! // once per frame:
//! for event in net.drain() {
//!     if let NetEvent::Rest(1, Ok(RestResponse::Auth(auth))) = event {
//!         net.connect(&auth);
//!     }
//! }
//! ```

mod keyring;
mod rest;
mod ws;

use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use capitol_protocol::rest::AuthResponse;
use capitol_protocol::ws::{ClientMessage, ServerMessage};
use tokio::sync::mpsc as tmpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use keyring::Keyring;

pub use rest::{RestError, RestRequest, RestResponse};
pub(crate) use ws::auth_error_is_final;

/// Matches a [`NetEvent::Rest`] result to the request that caused it.
/// The game chooses the values.
pub type Tag = u64;

/// Where the server is and how patient to be with it.
#[derive(Debug, Clone)]
pub struct NetConfig {
    /// Server root, e.g. `http://localhost:3000`. The WebSocket URL is derived
    /// from it: `http` becomes `ws`, `https` becomes `wss`, plus `/ws`.
    pub base_url: String,
    /// How long a WebSocket connect may take.
    pub connect_timeout: Duration,
    /// From connected to `subscribed`. The server allows 10 s for `auth`.
    pub handshake_timeout: Duration,
    /// How often to send a WebSocket Ping.
    pub ping_interval: Duration,
    /// With nothing heard from the server for this long, the session is dead.
    /// `positions` arrive every 100 ms once subscribed, so this is generous.
    pub idle_timeout: Duration,
    /// Whole-request timeout for REST calls.
    pub rest_timeout: Duration,
    /// The wait before the first reconnect attempt. It doubles with each
    /// failed attempt, up to `reconnect_max`.
    pub reconnect_initial: Duration,
    /// The longest wait between reconnect attempts. A session that stays
    /// connected this long resets the backoff.
    pub reconnect_max: Duration,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:3000".into(),
            connect_timeout: Duration::from_secs(10),
            handshake_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(45),
            rest_timeout: Duration::from_secs(30),
            reconnect_initial: Duration::from_millis(500),
            reconnect_max: Duration::from_secs(10),
        }
    }
}

impl NetConfig {
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            ..Self::default()
        }
    }

    fn rest_base(&self) -> String {
        self.base_url.trim_end_matches('/').to_string()
    }

    /// `http://host:3000` → `ws://host:3000/ws`; `https` → `wss`.
    pub fn ws_url(&self) -> String {
        let base = self.rest_base();
        let rest = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            base
        };
        format!("{rest}/ws")
    }
}

/// What a session signs in with. With a refresh token, the IO thread keeps
/// the access token fresh by itself (E0.6): before it expires, after a 401
/// (and the request goes again), and before each WebSocket reconnect. Each
/// refresh rotates the refresh token, and [`NetEvent::Refreshed`] carries the
/// new one to keep.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Credentials {
    /// The access token. `None` to refresh before anything is sent, as when
    /// resuming from a stored refresh token.
    pub token: Option<String>,
    pub refresh_token: Option<String>,
    /// How long `token` has left; `None` when that isn't known.
    pub expires_in: Option<Duration>,
}

impl Credentials {
    /// A session resumed from a stored refresh token.
    pub fn resume(refresh_token: impl Into<String>) -> Credentials {
        Credentials {
            refresh_token: Some(refresh_token.into()),
            ..Credentials::default()
        }
    }
}

/// An access token alone, which works until it expires.
impl From<String> for Credentials {
    fn from(token: String) -> Credentials {
        Credentials {
            token: Some(token),
            ..Credentials::default()
        }
    }
}

impl From<&str> for Credentials {
    fn from(token: &str) -> Credentials {
        Credentials::from(token.to_owned())
    }
}

impl From<&AuthResponse> for Credentials {
    fn from(auth: &AuthResponse) -> Credentials {
        Credentials {
            token: Some(auth.token.clone()),
            refresh_token: auth.refresh_token.clone(),
            // An older server sends no expiry.
            expires_in: (auth.expires_in > 0).then(|| Duration::from_secs(auth.expires_in)),
        }
    }
}

/// Game thread → IO thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Use these credentials for REST calls and open the WebSocket with them:
    /// connect, `auth`, then `subscribe` to positions. Replaces any open
    /// session. If the session drops, it reconnects with backoff until the
    /// server rejects the token or the game sends `Disconnect`.
    Connect(Credentials),
    /// Close the WebSocket and forget the credentials.
    Disconnect,
    /// Send over the WebSocket. Held until the handshake finishes if one is
    /// running; dropped with a warning if there's no session or it's waiting
    /// to reconnect.
    Ws(ClientMessage),
    /// Make a REST call. The result comes back as [`NetEvent::Rest`] with `tag`.
    /// Calls run concurrently, so results can arrive out of order.
    Rest(RestRequest, Tag),
}

/// IO thread → game thread.
#[derive(Debug, Clone, PartialEq)]
pub enum NetEvent {
    /// Every message the server sends, handshake replies included.
    Ws(ServerMessage),
    Rest(Tag, Result<RestResponse, RestError>),
    Connection(ConnectionState),
    /// Sent right after `Connected` when the session has reconnected. Events
    /// were lost while it was down, so refetch everything the server doesn't
    /// push: containers, nearby nodes and stations, currency, plots, recipes.
    Resync,
    /// The session's token was refreshed, and this is the refresh token now.
    /// The old one is spent: keep this one instead.
    Refreshed {
        refresh_token: String,
    },
}

/// The WebSocket's state. A session moves
/// `Connecting → Authenticating → Connected`. If it drops, from any of those,
/// it goes to `Reconnecting` and starts over with `Connecting`. It ends only
/// in `Disconnected` (the game closed it) or `AuthFailed`.
///
/// The server drops a client that falls 1024 events behind
/// (`backend/src/ws/connection.rs`, the `Err(_)` arm of the event loop)
/// without a Close frame. That arrives here as an ordinary drop.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Connecting,
    /// Connected; `auth` sent, waiting for `auth_ok` and `subscribed`.
    Authenticating,
    /// Authenticated and subscribed: game events are flowing.
    Connected {
        player_id: Uuid,
    },
    /// The session dropped, and the next attempt starts after `delay`.
    /// `attempt` counts from 1, and resets once a session stays up for
    /// [`NetConfig::reconnect_max`].
    Reconnecting {
        attempt: u32,
        delay: Duration,
        reason: String,
    },
    /// The game closed the session (`disconnect`, a new `connect`, or
    /// dropping `Net`). Nothing is retried.
    Disconnected {
        reason: String,
    },
    /// The server rejected the token: it expired, or its session was signed
    /// out from elsewhere. Nothing is retried: log in again.
    AuthFailed {
        message: String,
    },
}

enum Control {
    Command(Command),
    Shutdown,
}

/// The game's handle on the IO thread. Dropping it closes the WebSocket and
/// joins the thread.
pub struct Net {
    commands: tmpsc::UnboundedSender<Control>,
    events: mpsc::Receiver<NetEvent>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Net {
    /// Starts the IO thread.
    pub fn spawn(config: NetConfig) -> std::io::Result<Net> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let http = reqwest::Client::builder()
            .timeout(config.rest_timeout)
            .user_agent(concat!("capitol-client/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(std::io::Error::other)?;
        // The game sends on a tokio channel because its `send` never blocks and the
        // IO loop can await it; events go back on std mpsc for a plain `try_recv`.
        let (commands, command_rx) = tmpsc::unbounded_channel();
        let (event_tx, events) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("capitol-net".into())
            .spawn(move || runtime.block_on(io_loop(config, http, command_rx, event_tx)))?;
        Ok(Net {
            commands,
            events,
            thread: Some(thread),
        })
    }

    pub fn send(&self, command: Command) {
        if self.commands.send(Control::Command(command)).is_err() {
            tracing::error!("net thread has stopped; command dropped");
        }
    }

    /// [`Command::Connect`], with an access token, an [`AuthResponse`] or
    /// [`Credentials`]
    pub fn connect(&self, credentials: impl Into<Credentials>) {
        self.send(Command::Connect(credentials.into()));
    }

    /// [`Command::Disconnect`]
    pub fn disconnect(&self) {
        self.send(Command::Disconnect);
    }

    /// [`Command::Ws`]
    pub fn ws(&self, message: ClientMessage) {
        self.send(Command::Ws(message));
    }

    /// [`Command::Rest`]
    pub fn rest(&self, request: RestRequest, tag: Tag) {
        self.send(Command::Rest(request, tag));
    }

    /// The next event, if one is waiting.
    pub fn try_recv(&self) -> Option<NetEvent> {
        self.events.try_recv().ok()
    }

    /// Every event waiting now. Call once per frame.
    pub fn drain(&self) -> mpsc::TryIter<'_, NetEvent> {
        self.events.try_iter()
    }

    /// Blocks for up to `timeout`. For tests and headless tools, not the game loop.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<NetEvent> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        let _ = self.commands.send(Control::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn io_loop(
    config: NetConfig,
    http: reqwest::Client,
    mut commands: tmpsc::UnboundedReceiver<Control>,
    events: mpsc::Sender<NetEvent>,
) {
    let base = config.rest_base();
    let mut keyring: Option<Arc<Keyring>> = None;
    // Refreshes the keyring on time; one per session.
    let mut keeper: Option<JoinHandle<()>> = None;
    let mut session: Option<ws::Session> = None;

    while let Some(control) = commands.recv().await {
        let command = match control {
            Control::Command(command) => command,
            Control::Shutdown => break,
        };
        match command {
            Command::Connect(credentials) => {
                if let Some(old) = session.take() {
                    old.close().await;
                }
                if let Some(old) = keeper.take() {
                    old.abort();
                }
                let new = Keyring::new(credentials, http.clone(), base.clone(), events.clone());
                keeper = Some(tokio::spawn(keyring::keep_fresh(new.clone())));
                session = Some(ws::Session::spawn(
                    config.ws_url(),
                    new.clone(),
                    config.clone(),
                    events.clone(),
                ));
                keyring = Some(new);
            }
            Command::Disconnect => {
                keyring = None;
                if let Some(old) = keeper.take() {
                    old.abort();
                }
                if let Some(old) = session.take() {
                    old.close().await;
                }
            }
            Command::Ws(message) => match &session {
                Some(s) if !s.is_finished() => s.send(message),
                _ => tracing::warn!("not connected; dropping {}", message.type_name()),
            },
            Command::Rest(request, tag) => {
                let http = http.clone();
                let base = base.clone();
                let keyring = keyring.clone();
                let events = events.clone();
                tokio::spawn(async move {
                    let result = execute(&http, &base, keyring.as_deref(), &request).await;
                    if let Err(e) = &result {
                        tracing::debug!("{} {} failed: {e}", request.method(), request.path());
                    }
                    let _ = events.send(NetEvent::Rest(tag, result));
                });
            }
        }
    }

    if let Some(old) = keeper.take() {
        old.abort();
    }
    if let Some(old) = session.take() {
        old.close().await;
    }
}

/// Runs a request with the session's token. A 401 refreshes the token and
/// sends it once more: the server refuses before doing anything, so even a
/// POST is safe to repeat.
async fn execute(
    http: &reqwest::Client,
    base: &str,
    keyring: Option<&Keyring>,
    request: &RestRequest,
) -> Result<RestResponse, RestError> {
    let Some(keyring) = keyring else {
        return rest::execute(http, base, None, request).await;
    };
    let token = keyring.token().await;
    let result = rest::execute(http, base, token.as_deref(), request).await;
    match result {
        Err(e) if e.is_unauthorized() && !request.is_auth() => {
            match keyring.renew(token.as_deref()).await {
                Some(fresh) => rest::execute(http, base, Some(&fresh), request).await,
                None => Err(e),
            }
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_follows_the_scheme() {
        assert_eq!(NetConfig::default().ws_url(), "ws://localhost:3000/ws");
        assert_eq!(
            NetConfig::with_base_url("https://capitol.example/").ws_url(),
            "wss://capitol.example/ws"
        );
    }
}
