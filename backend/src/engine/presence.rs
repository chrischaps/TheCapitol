//! Presence (E0.8, D9): which of a player's sockets drives the avatar.
//!
//! An account can have many sessions (`sessions`) and many sockets open at
//! once, but only one socket **holds** the avatar. The others watch: they get
//! every event, and may still use the market and automation over REST and
//! WS, but nothing that drives the avatar ([`drives_avatar_ws`],
//! [`drives_avatar_rest`]).
//!
//! - The player's own clients (signed in with a session) take the avatar as
//!   they connect: the newest one holds it. An older one is told
//!   `presence {yours: false, held_by: client}` and can take it back with
//!   `claim_presence`.
//! - A bot (a `play` or `trade` API key) holds it only while none of the
//!   player's own clients is connected. A client connecting always takes it
//!   from the bot, and a bot's claim is refused while a client is connected.
//!   When the holder leaves, everyone left is told `held_by: None`, so a bot
//!   can claim it again.
//! - A `read` key only ever watches.
//!
//! [`Sockets`] is the bookkeeping, with no world in it. `GameState` adds the
//! player to the world when a socket picks the avatar up, and removes them
//! when its holder leaves.

use axum::http::Method;
use capitol_protocol::ws::{ClientMessage, PresenceHolder};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Who a socket, or a REST request, signed in as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// One of the player's own clients, by its session id
    Session(String),
    /// A bot, by its API key's id
    Key(Uuid),
}

impl Holder {
    pub fn kind(&self) -> PresenceHolder {
        match self {
            Holder::Session(_) => PresenceHolder::Client,
            Holder::Key(_) => PresenceHolder::Bot,
        }
    }
}

/// What a socket is told about its presence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceEvent {
    /// `presence`: whether it holds the avatar now, and who does if not.
    Changed {
        yours: bool,
        held_by: Option<PresenceHolder>,
    },
    /// Its session ended (a logout, or a revoked refresh family): say
    /// `auth_error` and hang up.
    Ended,
}

/// Why a claim was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("Your own client is connected, so a bot can't take the avatar")]
    ClientConnected,
    #[error("A read-only API key can only watch")]
    WatchOnly,
    #[error("This connection has closed")]
    Closed,
}

/// Refuses a command that drives the avatar from a socket that doesn't hold it.
pub const NOT_HOLDING: &str = "You're playing on another device. Take over here to do that";

struct Socket {
    id: u64,
    holder: Holder,
    /// Whether it may hold the avatar at all: a `read` key may not.
    may_hold: bool,
    tx: mpsc::UnboundedSender<PresenceEvent>,
}

impl Socket {
    fn tell(&self, event: PresenceEvent) {
        let _ = self.tx.send(event);
    }
}

/// One player's open sockets, and which of them holds the avatar.
#[derive(Default)]
pub struct Sockets {
    sockets: Vec<Socket>,
    holding: Option<u64>,
}

impl Sockets {
    /// Adds socket `id`, holding nothing yet.
    pub fn open(
        &mut self,
        id: u64,
        holder: Holder,
        may_hold: bool,
    ) -> mpsc::UnboundedReceiver<PresenceEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.sockets.push(Socket { id, holder, may_hold, tx });
        rx
    }

    pub fn is_empty(&self) -> bool {
        self.sockets.is_empty()
    }

    /// Whether socket `id` holds the avatar.
    pub fn holds(&self, id: u64) -> bool {
        self.holding == Some(id)
    }

    /// Who holds the avatar, if anyone does.
    pub fn holder(&self) -> Option<&Holder> {
        let id = self.holding?;
        self.sockets.iter().find(|s| s.id == id).map(|s| &s.holder)
    }

    fn get(&self, id: u64) -> Option<&Socket> {
        self.sockets.iter().find(|s| s.id == id)
    }

    /// Hands the avatar to socket `id`, telling it and whoever held it
    /// before. Returns whether nobody held it before, so the avatar has to
    /// come into the world.
    pub fn claim(&mut self, id: u64) -> Result<bool, Refused> {
        let socket = self.get(id).ok_or(Refused::Closed)?;
        if !socket.may_hold {
            return Err(Refused::WatchOnly);
        }
        if self.holding == Some(id) {
            return Ok(false);
        }
        let kind = socket.holder.kind();
        if kind == PresenceHolder::Bot
            && self
                .sockets
                .iter()
                .any(|s| s.id != id && s.holder.kind() == PresenceHolder::Client)
        {
            return Err(Refused::ClientConnected);
        }
        let arriving = match self.holding.and_then(|old| self.get(old)) {
            Some(old) => {
                old.tell(PresenceEvent::Changed { yours: false, held_by: Some(kind) });
                false
            }
            None => true,
        };
        self.holding = Some(id);
        self.get(id)
            .expect("checked above")
            .tell(PresenceEvent::Changed { yours: true, held_by: None });
        Ok(arriving)
    }

    /// Tells socket `id` who holds the avatar, when it isn't itself.
    pub fn tell_watcher(&self, id: u64) {
        if let Some(socket) = self.get(id) {
            let held_by = self.holder().map(Holder::kind);
            socket.tell(PresenceEvent::Changed { yours: false, held_by });
        }
    }

    /// Socket `id` closed. Returns whether it held the avatar, so the avatar
    /// leaves the world. The sockets left are told nobody holds it.
    pub fn close(&mut self, id: u64) -> bool {
        self.sockets.retain(|s| s.id != id);
        if self.holding != Some(id) {
            return false;
        }
        self.holding = None;
        for socket in &self.sockets {
            socket.tell(PresenceEvent::Changed { yours: false, held_by: None });
        }
        true
    }

    /// The sessions `sids` ended: their sockets are told to hang up. Returns
    /// how many were.
    pub fn end_sessions(&self, sids: &[String]) -> usize {
        self.sockets
            .iter()
            .filter(|s| matches!(&s.holder, Holder::Session(sid) if sids.contains(sid)))
            .map(|s| s.tell(PresenceEvent::Ended))
            .count()
    }
}

/// Whether a WebSocket command drives the avatar, so only the socket holding
/// it may send it. The market, automation and watching don't.
pub fn drives_avatar_ws(message: &ClientMessage) -> bool {
    !matches!(
        message,
        ClientMessage::Auth { .. }
            | ClientMessage::Subscribe { .. }
            | ClientMessage::SubscribeMarket { .. }
            | ClientMessage::UnsubscribeMarket { .. }
            | ClientMessage::ConfigureAutomation { .. }
            | ClientMessage::StartAutomation { .. }
            | ClientMessage::StopAutomation { .. }
            | ClientMessage::ClaimPresence
            // A watcher may talk on the Capitol channel; `talk` refuses it a local line
            | ClientMessage::Say { .. }
            // What a screen showed: the pack opened on any device is opened (#441)
            | ClientMessage::TutorialSeen { .. }
    )
}

/// Whether a REST call drives the avatar, so only the session or key holding
/// it may make it while someone does. Everything else (reading, the market,
/// the exchange, automation, plots, the account) works from any session.
pub fn drives_avatar_rest(method: &Method, path: &str) -> bool {
    let path = path.trim_end_matches('/');
    let mut parts = path.trim_start_matches('/').split('/');
    let group = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    matches!(
        (method.as_str(), group, rest.as_slice()),
        ("POST", "player", ["move"])
            | ("POST", "world", ["extract"])
            | ("POST", "inventory", ["move"])
            | ("POST", "stations", ["place"])
            | ("DELETE", "stations", [_])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(sid: &str) -> Holder {
        Holder::Session(sid.into())
    }

    fn told(rx: &mut mpsc::UnboundedReceiver<PresenceEvent>) -> Vec<PresenceEvent> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn changed(yours: bool, held_by: Option<PresenceHolder>) -> PresenceEvent {
        PresenceEvent::Changed { yours, held_by }
    }

    #[test]
    fn the_newest_client_takes_the_avatar_and_the_older_can_take_it_back() {
        let mut sockets = Sockets::default();
        let mut desk = sockets.open(1, session("desk"), true);
        assert_eq!(sockets.claim(1), Ok(true), "nobody held it, so it comes into the world");
        let mut phone = sockets.open(2, session("phone"), true);
        assert_eq!(sockets.claim(2), Ok(false));
        assert_eq!(told(&mut desk), [changed(true, None), changed(false, Some(PresenceHolder::Client))]);
        assert_eq!(told(&mut phone), [changed(true, None)]);
        assert_eq!(sockets.holder(), Some(&session("phone")));

        assert_eq!(sockets.claim(1), Ok(false));
        assert!(sockets.holds(1) && !sockets.holds(2));
        assert_eq!(told(&mut phone), [changed(false, Some(PresenceHolder::Client))]);
        assert_eq!(sockets.claim(1), Ok(false), "claiming what you hold is a no-op");
        assert_eq!(told(&mut desk), [changed(true, None)], "and says nothing more");
    }

    #[test]
    fn a_bot_holds_only_while_no_client_is_connected() {
        let mut sockets = Sockets::default();
        let key = Uuid::new_v4();
        let mut bot = sockets.open(1, Holder::Key(key), true);
        assert_eq!(sockets.claim(1), Ok(true));

        // A client arrives and takes it.
        let mut desk = sockets.open(2, session("desk"), true);
        assert_eq!(sockets.claim(2), Ok(false));
        assert_eq!(told(&mut bot), [changed(true, None), changed(false, Some(PresenceHolder::Client))]);
        // The bot can't take it back while the client is connected, even watching.
        assert_eq!(sockets.claim(1), Err(Refused::ClientConnected));

        // The client leaves: the bot hears nobody holds it, and may claim.
        assert!(sockets.close(2), "the client held it");
        assert_eq!(told(&mut bot), [changed(false, None)]);
        assert_eq!(sockets.claim(1), Ok(true), "the avatar comes back into the world");
        assert_eq!(told(&mut desk), [changed(true, None)], "a closed socket hears nothing more");
    }

    #[test]
    fn a_bot_cant_claim_past_a_watching_client() {
        let mut sockets = Sockets::default();
        sockets.open(1, session("desk"), true);
        sockets.claim(1).unwrap();
        let mut phone = sockets.open(2, session("phone"), true);
        sockets.tell_watcher(2);
        assert_eq!(told(&mut phone), [changed(false, Some(PresenceHolder::Client))]);
        let bot = sockets.open(3, Holder::Key(Uuid::new_v4()), true);
        drop(bot);

        assert!(sockets.close(1));
        assert_eq!(sockets.claim(3), Err(Refused::ClientConnected), "the phone is still there");
        assert_eq!(sockets.claim(2), Ok(true));
    }

    #[test]
    fn a_read_key_only_watches() {
        let mut sockets = Sockets::default();
        let mut watcher = sockets.open(1, Holder::Key(Uuid::new_v4()), false);
        assert_eq!(sockets.claim(1), Err(Refused::WatchOnly));
        sockets.tell_watcher(1);
        assert_eq!(told(&mut watcher), [changed(false, None)]);
        assert!(!sockets.close(1), "it held nothing");
        assert!(sockets.is_empty());
    }

    #[test]
    fn a_watcher_closing_changes_nothing() {
        let mut sockets = Sockets::default();
        let mut desk = sockets.open(1, session("desk"), true);
        sockets.claim(1).unwrap();
        sockets.open(2, session("phone"), true);
        told(&mut desk);
        assert!(!sockets.close(2));
        assert!(sockets.holds(1));
        assert!(told(&mut desk).is_empty());
    }

    #[test]
    fn an_ended_session_hangs_up_only_its_own_sockets() {
        let mut sockets = Sockets::default();
        let mut desk = sockets.open(1, session("desk"), true);
        let mut phone = sockets.open(2, session("phone"), true);
        let mut bot = sockets.open(3, Holder::Key(Uuid::new_v4()), true);
        assert_eq!(sockets.end_sessions(&["phone".into()]), 1);
        assert_eq!(told(&mut phone), [PresenceEvent::Ended]);
        assert!(told(&mut desk).is_empty() && told(&mut bot).is_empty());
        assert_eq!(sockets.claim(9), Err(Refused::Closed));
    }

    #[test]
    fn the_market_and_automation_work_from_a_watcher_but_the_avatar_doesnt() {
        for watching in [
            ClientMessage::SubscribeMarket { item_type: "logs".into() },
            ClientMessage::StartAutomation { station_id: "s".into() },
            ClientMessage::ClaimPresence,
            ClientMessage::Say { channel: capitol_protocol::ws::Channel::Capitol, text: "Hello".into() },
            ClientMessage::TutorialSeen { step: capitol_protocol::tutorial::Step::OpenPack },
        ] {
            assert!(!drives_avatar_ws(&watching), "{watching:?}");
        }
        for driving in [
            ClientMessage::Move { x: 1.0, y: 2.0 },
            ClientMessage::Steer { heading: 0, throttle: 10, at: None },
            ClientMessage::CancelCrafting,
            ClientMessage::TradeCancel,
            ClientMessage::CloseStation,
        ] {
            assert!(drives_avatar_ws(&driving), "{driving:?}");
        }

        assert!(drives_avatar_rest(&Method::POST, "/player/move"));
        assert!(drives_avatar_rest(&Method::POST, "/world/extract/"));
        assert!(drives_avatar_rest(&Method::DELETE, "/stations/abc"));
        assert!(!drives_avatar_rest(&Method::POST, "/market/orders"));
        assert!(!drives_avatar_rest(&Method::POST, "/market/box/collect"));
        assert!(!drives_avatar_rest(&Method::PUT, "/stations/abc/automation"));
        assert!(!drives_avatar_rest(&Method::POST, "/stations/abc/automation/start"));
        assert!(!drives_avatar_rest(&Method::POST, "/exchange/deposit"));
        assert!(!drives_avatar_rest(&Method::GET, "/player/me"));
    }
}
