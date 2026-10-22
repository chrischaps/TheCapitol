//! How hard the server works (C1.7): each tick's time, and the sockets, as
//! running totals since it started. `GET /admin/ledger` reads them
//! ([`read`]); a soak takes two readings and works out the window between
//! (`capitol_protocol::ledger::LoadWindow`).
//!
//! Since D0.1 the tick's time is also kept by [`Phase`], and the loops
//! beside it by [`Loop`], each with a histogram like the tick's. Since D3.2
//! each also counts its slow statements ([`slow_statement`]).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use capitol_protocol::ledger::{Load, PhaseLoad, TICK_BUCKETS_MS};

/// Running totals of something timed: how often, how long, the slowest,
/// and a histogram over [`TICK_BUCKETS_MS`].
struct Timings {
    runs: AtomicU64,
    nanos: AtomicU64,
    max_nanos: AtomicU64,
    /// Runs under each of [`TICK_BUCKETS_MS`], and past the last
    buckets: [AtomicU64; TICK_BUCKETS_MS.len() + 1],
    /// Its statements over the slow threshold (D3.2)
    slow_statements: AtomicU64,
}

impl Timings {
    const fn new() -> Timings {
        Timings {
            runs: AtomicU64::new(0),
            nanos: AtomicU64::new(0),
            max_nanos: AtomicU64::new(0),
            buckets: [const { AtomicU64::new(0) }; TICK_BUCKETS_MS.len() + 1],
            slow_statements: AtomicU64::new(0),
        }
    }

    fn took(&self, took: Duration) {
        let nanos = took.as_nanos().min(u64::MAX as u128) as u64;
        self.runs.fetch_add(1, Ordering::Relaxed);
        self.nanos.fetch_add(nanos, Ordering::Relaxed);
        self.max_nanos.fetch_max(nanos, Ordering::Relaxed);
        let ms = nanos as f64 / 1e6;
        let bucket = TICK_BUCKETS_MS.iter().position(|bound| ms <= *bound).unwrap_or(TICK_BUCKETS_MS.len());
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
    }

    fn read(&self) -> PhaseLoad {
        PhaseLoad {
            runs: self.runs.load(Ordering::Relaxed),
            secs: self.nanos.load(Ordering::Relaxed) as f64 / 1e9,
            max_ms: self.max_nanos.load(Ordering::Relaxed) as f64 / 1e6,
            buckets: self.buckets.iter().map(|b| b.load(Ordering::Relaxed)).collect(),
            slow_statements: self.slow_statements.load(Ordering::Relaxed),
        }
    }
}

static TICK: Timings = Timings::new();
static PHASES: [Timings; Phase::ALL.len()] = [const { Timings::new() }; Phase::ALL.len()];
static LOOPS: [Timings; Loop::ALL.len()] = [const { Timings::new() }; Loop::ALL.len()];

static SOCKETS_OPENED: AtomicU64 = AtomicU64::new(0);
static SOCKETS_ENDED: [AtomicU64; SocketEnd::ALL.len()] = [const { AtomicU64::new(0) }; SocketEnd::ALL.len()];
static SOCKET_RESYNCS: AtomicU64 = AtomicU64::new(0);

static SLOW_ELSEWHERE: AtomicU64 = AtomicU64::new(0);
/// `SLOW_STATEMENT_MS`, as the pool was told it, in µs
static SLOW_THRESHOLD_MICROS: AtomicU64 = AtomicU64::new(0);

/// A tick's work took `took`: the world's tick and the writes after it,
/// not the wait for the next.
pub fn tick_took(took: Duration) {
    TICK.took(took);
}

/// A part of the tick's work (D0.1). Between them they cover all of it, so
/// their times add up to the tick's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the game's lock, which the automation loop and the
    /// routes take too
    Lock,
    Commands,
    Movement,
    Extraction,
    Crafting,
    Regeneration,
    /// Expiring trade requests nobody answered
    Trades,
    /// The positions that changed, to every socket
    Broadcast,
    /// Handing the gathers and crafts the tick finished to the writer
    /// ([`Loop::Writer`]), which saves them off the tick (#304)
    Writes,
    /// Reading every player's position, every 100 ticks, for
    /// [`Loop::Positions`] to save
    Positions,
    /// Starting the hourly property tax ([`Loop::Tax`])
    Tax,
    /// Starting the hourly lapse of subscriptions that ran out
    /// ([`Loop::Subscriptions`])
    Subscriptions,
}

impl Phase {
    pub const ALL: [Phase; 12] = [
        Phase::Lock,
        Phase::Commands,
        Phase::Movement,
        Phase::Extraction,
        Phase::Crafting,
        Phase::Regeneration,
        Phase::Trades,
        Phase::Broadcast,
        Phase::Writes,
        Phase::Positions,
        Phase::Tax,
        Phase::Subscriptions,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Lock => "lock",
            Phase::Commands => "commands",
            Phase::Movement => "movement",
            Phase::Extraction => "extraction",
            Phase::Crafting => "crafting",
            Phase::Regeneration => "regeneration",
            Phase::Trades => "trades",
            Phase::Broadcast => "broadcast",
            Phase::Writes => "writes",
            Phase::Positions => "positions",
            Phase::Tax => "tax",
            Phase::Subscriptions => "subscriptions",
        }
    }

    fn index(&self) -> usize {
        Phase::ALL.iter().position(|p| p == self).expect("every phase is in ALL")
    }

    /// The phase whose span is `name` (`tick.<phase>`).
    pub fn of_span(name: &str) -> Option<Phase> {
        let name = name.strip_prefix("tick.")?;
        Phase::ALL.into_iter().find(|p| p.as_str() == name)
    }

    /// Times the phase until the returned guard drops.
    pub fn timed(self) -> Timed {
        Timed {
            timings: &PHASES[self.index()],
            began: Instant::now(),
        }
    }
}

/// Work that runs beside the tick: a loop on its own interval, or a task
/// the tick hands its writes to so it never awaits them (#304).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loop {
    Automation,
    Market,
    Bazaar,
    /// Saving one tick's finished gathers and crafts (`persist_gather`,
    /// `finish_craft`), in order
    Writer,
    /// Saving every player's position in one statement, every 100 ticks
    Positions,
    /// The hourly property tax
    Tax,
    /// Lapsing subscriptions that ran out, hourly
    Subscriptions,
    /// The Observatory's sampler (D0.2): a ledger and the window's trades
    Sampler,
}

impl Loop {
    pub const ALL: [Loop; 8] = [
        Loop::Automation,
        Loop::Market,
        Loop::Bazaar,
        Loop::Writer,
        Loop::Positions,
        Loop::Tax,
        Loop::Subscriptions,
        Loop::Sampler,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Loop::Automation => "automation",
            Loop::Market => "market",
            Loop::Bazaar => "bazaar",
            Loop::Writer => "writer",
            Loop::Positions => "positions",
            Loop::Tax => "tax",
            Loop::Subscriptions => "subscriptions",
            Loop::Sampler => "sampler",
        }
    }

    fn index(&self) -> usize {
        Loop::ALL.iter().position(|l| l == self).expect("every loop is in ALL")
    }

    /// The loop whose span is `name` (`loop.<name>`).
    pub fn of_span(name: &str) -> Option<Loop> {
        let name = name.strip_prefix("loop.")?;
        Loop::ALL.into_iter().find(|l| l.as_str() == name)
    }

    /// Times one cycle until the returned guard drops.
    pub fn timed(self) -> Timed {
        Timed {
            timings: &LOOPS[self.index()],
            began: Instant::now(),
        }
    }
}

/// A phase or a cycle being timed: it counts when it's dropped. It holds
/// no span, so it can be held across an `.await`.
#[must_use = "it times until it's dropped"]
pub struct Timed {
    timings: &'static Timings,
    began: Instant,
}

impl Drop for Timed {
    fn drop(&mut self) {
        self.timings.took(self.began.elapsed());
    }
}

/// Where a statement ran: in a phase of the tick, in a loop beside it, or
/// anywhere else (a route, a socket, startup).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Phase(Phase),
    Loop(Loop),
    Elsewhere,
}

/// What counts as a slow statement, for the ledger to say (D3.2).
pub fn slow_threshold(threshold: Duration) {
    SLOW_THRESHOLD_MICROS.store(threshold.as_micros().min(u64::MAX as u128) as u64, Ordering::Relaxed);
}

/// A statement took the slow threshold or longer, in `work` (D3.2).
pub fn slow_statement(work: Work) {
    let counter = match work {
        Work::Phase(p) => &PHASES[p.index()].slow_statements,
        Work::Loop(l) => &LOOPS[l.index()].slow_statements,
        Work::Elsewhere => &SLOW_ELSEWHERE,
    };
    counter.fetch_add(1, Ordering::Relaxed);
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
    let tick = TICK.read();
    Load {
        tick_period_ms: tick_period.as_secs_f64() * 1000.0,
        ticks: tick.runs,
        tick_secs: tick.secs,
        tick_max_ms: tick.max_ms,
        tick_buckets: tick.buckets,
        phases: Phase::ALL.iter().map(|p| (p.as_str().to_string(), PHASES[p.index()].read())).collect(),
        loops: Loop::ALL.iter().map(|l| (l.as_str().to_string(), LOOPS[l.index()].read())).collect(),
        sockets_open: opened.saturating_sub(ended.values().sum()),
        sockets_opened: opened,
        sockets_ended: ended,
        socket_resyncs: SOCKET_RESYNCS.load(Ordering::Relaxed),
        slow_statements_elsewhere: SLOW_ELSEWHERE.load(Ordering::Relaxed),
        slow_statement_ms: SLOW_THRESHOLD_MICROS.load(Ordering::Relaxed) as f64 / 1000.0,
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

    #[test]
    fn phases_and_loops_count_when_their_guard_drops() {
        let before = read(Duration::from_millis(100));
        assert_eq!(before.phases.len(), Phase::ALL.len());
        assert_eq!(before.loops.len(), Loop::ALL.len());
        {
            let _tax = Phase::Tax.timed();
            std::thread::sleep(Duration::from_millis(3));
        }
        drop(Loop::Bazaar.timed());
        let after = read(Duration::from_millis(100));
        let (was, is) = (&before.phases["tax"], &after.phases["tax"]);
        assert!(is.runs > was.runs);
        assert!(is.secs - was.secs >= 0.003);
        assert!(is.max_ms >= 3.0);
        assert_eq!(is.buckets.len(), TICK_BUCKETS_MS.len() + 1);
        // 3 ms and more: past the 2 ms bound
        assert!(is.buckets[2..].iter().sum::<u64>() > was.buckets[2..].iter().sum::<u64>());
        assert!(after.loops["bazaar"].runs > before.loops["bazaar"].runs);
    }
}
