//! The IO thread. A tokio runtime runs on one background thread with
//! `reqwest` for REST and `tokio-tungstenite` for the WebSocket. The game
//! talks to it through [`Net`]: [`Command`]s go in, [`NetEvent`]s come out,
//! and neither side ever blocks the other.
//!
//! ```no_run
//! # use capitol_core::net::*;
//! # use capitol_core::protocol::rest::LoginRequest;
//! let net = Net::spawn(NetConfig::default()).unwrap();
//! net.rest(RestRequest::Login(LoginRequest { email: "a@b.c".into(), password: "pw".into() }), 1);
//! // once per frame:
//! for event in net.drain() {
//!     if let NetEvent::Rest(1, Ok(RestResponse::Auth(auth))) = event {
//!         net.connect(auth.token);
//!     }
//! }
//! ```

mod rest;
mod ws;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use capitol_protocol::ws::{ClientMessage, ServerMessage};
use tokio::sync::mpsc as tmpsc;
use uuid::Uuid;

pub use rest::{RestError, RestRequest, RestResponse};

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

/// Game thread → IO thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Use `token` for REST calls and open the WebSocket with it: connect,
    /// `auth`, then `subscribe` to positions. Replaces any open session.
    Connect { token: String },
    /// Close the WebSocket and forget the token.
    Disconnect,
    /// Send over the WebSocket. Held until the handshake finishes if one is
    /// running; dropped with a warning if there's no session.
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
}

/// The WebSocket's state. A session moves
/// `Connecting → Authenticating → Connected`, and ends in `Disconnected`
/// or `AuthFailed`.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Connecting,
    /// Connected; `auth` sent, waiting for `auth_ok` and `subscribed`.
    Authenticating,
    /// Authenticated and subscribed: game events are flowing.
    Connected {
        player_id: Uuid,
    },
    /// The session ended. Retrying may help.
    Disconnected {
        reason: String,
    },
    /// The server rejected the token. Retrying won't help: log in again.
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

    /// [`Command::Connect`]
    pub fn connect(&self, token: impl Into<String>) {
        self.send(Command::Connect {
            token: token.into(),
        });
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
    let mut token: Option<String> = None;
    let mut session: Option<ws::Session> = None;

    while let Some(control) = commands.recv().await {
        let command = match control {
            Control::Command(command) => command,
            Control::Shutdown => break,
        };
        match command {
            Command::Connect { token: new_token } => {
                if let Some(old) = session.take() {
                    old.close().await;
                }
                token = Some(new_token.clone());
                session = Some(ws::Session::spawn(
                    config.ws_url(),
                    new_token,
                    config.clone(),
                    events.clone(),
                ));
            }
            Command::Disconnect => {
                token = None;
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
                let token = token.clone();
                let events = events.clone();
                tokio::spawn(async move {
                    let result = rest::execute(&http, &base, token.as_deref(), &request).await;
                    if let Err(e) = &result {
                        tracing::debug!("{} {} failed: {e}", request.method(), request.path());
                    }
                    let _ = events.send(NetEvent::Rest(tag, result));
                });
            }
        }
    }

    if let Some(old) = session.take() {
        old.close().await;
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
