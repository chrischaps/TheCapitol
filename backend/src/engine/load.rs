//! How hard the server works (C1.7): each tick's time, and the sockets, as
//! running totals since it started. `GET /admin/ledger` reads them
//! ([`read`]); a soak takes two readings and works out the window between
//! (`capitol_protocol::ledger::LoadWindow`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use capitol_protocol::ledger::{Load, TICK_BUCKETS_MS};

static TICKS: AtomicU64 = AtomicU64::new(0);
static TICK_NANOS: AtomicU64 = AtomicU64::new(0);
static TICK_MAX_NANOS: AtomicU64 = AtomicU64::new(0);
/// Ticks under each of [`TICK_BUCKETS_MS`], and past the last
static TICK_BUCKETS: [AtomicU64; TICK_BUCKETS_MS.len() + 1] = [const { AtomicU64::new(0) }; TICK_BUCKETS_MS.len() + 1];

static SOCKETS_OPENED: AtomicU64 = AtomicU64::new(0);
static SOCKETS_ENDED: [AtomicU64; SocketEnd::ALL.len()] = [const { AtomicU64::new(0) }; SocketEnd::ALL.len()];
static SOCKET_RESYNCS: AtomicU64 = AtomicU64::new(0);

/// A tick's work took `took`: the world's tick and the writes after it,
/// not the wait for the next.
pub fn tick_took(took: Duration) {
    let nanos = took.as_nanos().min(u64::MAX as u128) as u64;
    TICKS.fetch_add(1, Ordering::Relaxed);
    TICK_NANOS.fetch_add(nanos, Ordering::Relaxed);
    TICK_MAX_NANOS.fetch_max(nanos, Ordering::Relaxed);
    let ms = nanos as f64 / 1e6;
    let bucket = TICK_BUCKETS_MS.iter().position(|bound| ms <= *bound).unwrap_or(TICK_BUCKETS_MS.len());
    TICK_BUCKETS[bucket].fetch_add(1, Ordering::Relaxed);
}

/// How a WebSocket ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketEnd {
    /// It never signed in: no auth, a bad token or key, no player
    AuthFailed,
    /// The client sent a close
    Closed,
    /// It went without a close: the connection dropped, or the client
    /// just stopped
    Vanished,
    /// The server couldn't write to it
    SendFailed,
    /// Its session ended: logged out, or the account's avatar went to
    /// another client and it was told so (#78)
    SessionEnded,
    /// The API key it opened with was revoked
    KeyRevoked,
    /// The server's event stream closed under it: it's stopping
    ServerStopping,
}

impl SocketEnd {
    const ALL: [SocketEnd; 7] = [
        SocketEnd::AuthFailed,
        SocketEnd::Closed,
        SocketEnd::Vanished,
        SocketEnd::SendFailed,
        SocketEnd::SessionEnded,
        SocketEnd::KeyRevoked,
        SocketEnd::ServerStopping,
    ];

    fn as_str(&self) -> &'static str {
        match self {
            SocketEnd::AuthFailed => "auth_failed",
            SocketEnd::Closed => "closed",
            SocketEnd::Vanished => "vanished",
            SocketEnd::SendFailed => "send_failed",
            SocketEnd::SessionEnded => "session_ended",
            SocketEnd::KeyRevoked => "key_revoked",
            SocketEnd::ServerStopping => "server_stopping",
        }
    }

    fn index(&self) -> usize {
        SocketEnd::ALL.iter().position(|e| e == self).expect("every end is in ALL")
    }
}

/// One WebSocket, counted open from [`Socket::opened`] until it's dropped,
/// and counted ended by however it was last said to end.
pub struct Socket {
    end: SocketEnd,
}

impl Socket {
    /// It ends as [`SocketEnd::AuthFailed`] unless it's told otherwise.
    pub fn opened() -> Socket {
        SOCKETS_OPENED.fetch_add(1, Ordering::Relaxed);
        Socket { end: SocketEnd::AuthFailed }
    }

    /// How it will have ended, when it does.
    pub fn ends(&mut self, how: SocketEnd) {
        self.end = how;
    }

    /// It fell behind the event stream and was resynced (C0.3).
    pub fn resynced(&self) {
        SOCKET_RESYNCS.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        SOCKETS_ENDED[self.end.index()].fetch_add(1, Ordering::Relaxed);
    }
}

/// The running totals. The database's numbers and the avatars in the world
/// are left for the caller, which has them.
pub fn read(tick_period: Duration) -> Load {
    let ended: BTreeMap<String, u64> = SocketEnd::ALL
        .iter()
        .map(|e| (e.as_str().to_string(), SOCKETS_ENDED[e.index()].load(Ordering::Relaxed)))
        .collect();
    let opened = SOCKETS_OPENED.load(Ordering::Relaxed);
    Load {
        tick_period_ms: tick_period.as_secs_f64() * 1000.0,
        ticks: TICKS.load(Ordering::Relaxed),
        tick_secs: TICK_NANOS.load(Ordering::Relaxed) as f64 / 1e9,
        tick_max_ms: TICK_MAX_NANOS.load(Ordering::Relaxed) as f64 / 1e6,
        tick_buckets: TICK_BUCKETS.iter().map(|b| b.load(Ordering::Relaxed)).collect(),
        sockets_open: opened.saturating_sub(ended.values().sum()),
        sockets_opened: opened,
        sockets_ended: ended,
        socket_resyncs: SOCKET_RESYNCS.load(Ordering::Relaxed),
        ..Load::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The totals are the process's, and other tests open sockets too, so
    /// this only looks at what it changed.
    #[test]
    fn ticks_and_sockets_add_up() {
        let before = read(Duration::from_millis(100));
        tick_took(Duration::from_micros(1500));
        tick_took(Duration::from_secs(2));
        {
            let mut socket = Socket::opened();
            socket.ends(SocketEnd::Vanished);
            socket.resynced();
        }
        let _never_signed_in = Socket::opened();
        let after = read(Duration::from_millis(100));
        assert!(after.ticks >= before.ticks + 2);
        assert!(after.tick_max_ms >= 2000.0);
        // 1.5 ms is under the 2 ms bound; 2 s is past the last
        assert!(after.tick_buckets[2] > before.tick_buckets[2]);
        assert!(after.tick_buckets[TICK_BUCKETS_MS.len()] > before.tick_buckets[TICK_BUCKETS_MS.len()]);
        assert!(after.sockets_ended["vanished"] > before.sockets_ended["vanished"]);
        assert!(after.socket_resyncs > before.socket_resyncs);
        assert!(after.sockets_opened >= before.sockets_opened + 2);
        assert_eq!(after.tick_period_ms, 100.0);
    }
}
