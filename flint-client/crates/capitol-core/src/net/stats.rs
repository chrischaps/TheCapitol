//! What the connection is doing, for a dev overlay or a bot's report (D5.1):
//! the round trip, messages and bytes per type per second each way, and how
//! often the session has reconnected. The IO thread keeps a [`Meter`] as
//! frames go by; [`Net::stats`](super::Net::stats) takes a [`NetStats`]
//! snapshot of it. Nothing here needs the engine, so bots and citizens carry
//! it too.
//!
//! The prediction's side, corrections and interpolation depth, lives with
//! [`Motion`](crate::motion::Motion), which a bot doesn't have. A game that
//! has one adds it with [`NetStats::with_motion`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::motion::MotionStats;

/// The weight of a new round trip in [`NetStats::rtt_avg`].
const RTT_WEIGHT: f64 = 0.2;

/// What a frame that isn't a message is counted as.
pub const PING: &str = "ping";
pub const PONG: &str = "pong";
/// A text frame that isn't a message the client knows.
pub const MALFORMED: &str = "malformed";

/// A snapshot of the connection. Rates are over the last whole second.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetStats {
    /// The last Ping's round trip. `None` until a Pong has come back.
    pub rtt: Option<Duration>,
    /// The round trip, smoothed over the last few Pings.
    pub rtt_avg: Option<Duration>,
    /// From the server.
    pub inbound: Traffic,
    /// To the server.
    pub outbound: Traffic,
    /// How many times the session has dropped and started over.
    pub reconnects: u32,
    /// The prediction's side, when [`NetStats::with_motion`] added it.
    pub motion: Option<MotionStats>,
}

impl NetStats {
    /// Adds the prediction's side from the game's [`Motion`](crate::motion::Motion).
    pub fn with_motion(mut self, motion: MotionStats) -> NetStats {
        self.motion = Some(motion);
        self
    }
}

/// One line for a log or a bot's report: `rtt 3 ms (avg 4 ms), in 11 msg/s
/// 1.4 kB/s, out 1 msg/s 0.1 kB/s, 0 reconnects`, and the corrections when
/// the prediction's side is in.
impl fmt::Display for NetStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        match (self.rtt, self.rtt_avg) {
            (Some(rtt), Some(avg)) => write!(f, "rtt {:.0} ms (avg {:.0} ms)", ms(rtt), ms(avg))?,
            _ => write!(f, "rtt unknown")?,
        }
        for (way, traffic) in [("in", &self.inbound), ("out", &self.outbound)] {
            write!(
                f,
                ", {way} {} msg/s {:.1} kB/s",
                traffic.messages_per_second(),
                traffic.bytes_per_second() as f64 / 1000.0
            )?;
        }
        let s = if self.reconnects == 1 { "" } else { "s" };
        write!(f, ", {} reconnect{s}", self.reconnects)?;
        if let Some(motion) = &self.motion {
            let s = if motion.corrections == 1 { "" } else { "s" };
            write!(f, ", {} correction{s} ({:.0} u)", motion.corrections, motion.corrected)?;
        }
        Ok(())
    }
}

/// One direction's traffic.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Traffic {
    /// Each type seen in the last whole second, busiest first.
    pub per_second: Vec<TypeRate>,
    /// Since the net thread started.
    pub messages: u64,
    pub bytes: u64,
}

impl Traffic {
    /// Messages in the last whole second, of every type.
    pub fn messages_per_second(&self) -> u32 {
        self.per_second.iter().map(|r| r.messages).sum()
    }

    /// Bytes in the last whole second, of every type.
    pub fn bytes_per_second(&self) -> u64 {
        self.per_second.iter().map(|r| r.bytes).sum()
    }

    /// One type's rate, if it was seen in the last whole second.
    pub fn rate(&self, kind: &str) -> Option<&TypeRate> {
        self.per_second.iter().find(|r| r.kind == kind)
    }
}

/// One message type's share of a second.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRate {
    /// The message's `type`, or [`PING`], [`PONG`] or [`MALFORMED`].
    pub kind: &'static str,
    pub messages: u32,
    pub bytes: u64,
}

/// The IO thread's running count, shared with [`Net`](super::Net).
pub(crate) type SharedMeter = Arc<Mutex<Meter>>;

#[derive(Debug)]
pub(crate) struct Meter {
    epoch: Instant,
    inbound: Direction,
    outbound: Direction,
    rtt: Option<Duration>,
    rtt_avg: Option<Duration>,
    reconnects: u32,
    /// The Ping waiting for its Pong: its number, and when it went.
    ping: Option<(u64, Instant)>,
    next_ping: u64,
}

impl Meter {
    pub(crate) fn new(epoch: Instant) -> Meter {
        Meter {
            epoch,
            inbound: Direction::default(),
            outbound: Direction::default(),
            rtt: None,
            rtt_avg: None,
            reconnects: 0,
            ping: None,
            next_ping: 0,
        }
    }

    pub(crate) fn shared() -> SharedMeter {
        Arc::new(Mutex::new(Meter::new(Instant::now())))
    }

    /// The meter, even if a thread panicked holding it: it's only counts.
    pub(crate) fn lock(meter: &SharedMeter) -> MutexGuard<'_, Meter> {
        meter.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn second(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.epoch).as_secs()
    }

    pub(crate) fn received(&mut self, at: Instant, kind: &'static str, bytes: usize) {
        let second = self.second(at);
        self.inbound.count(second, kind, bytes);
    }

    pub(crate) fn sent(&mut self, at: Instant, kind: &'static str, bytes: usize) {
        let second = self.second(at);
        self.outbound.count(second, kind, bytes);
    }

    /// A Ping going out now: the payload to send with it. A Ping still
    /// waiting is forgotten, so a lost Pong can't hold up the next one.
    pub(crate) fn ping(&mut self, at: Instant) -> [u8; 8] {
        let n = self.next_ping;
        self.next_ping += 1;
        self.ping = Some((n, at));
        n.to_be_bytes()
    }

    /// A Pong came back. Only the answer to the last Ping counts.
    pub(crate) fn pong(&mut self, at: Instant, payload: &[u8]) {
        let Some((n, sent)) = self.ping else { return };
        if payload != n.to_be_bytes() {
            return;
        }
        self.ping = None;
        let rtt = at.saturating_duration_since(sent);
        self.rtt = Some(rtt);
        self.rtt_avg = Some(match self.rtt_avg {
            None => rtt,
            Some(avg) => avg.mul_f64(1.0 - RTT_WEIGHT) + rtt.mul_f64(RTT_WEIGHT),
        });
    }

    /// The session dropped; a Ping in flight won't be answered.
    pub(crate) fn reconnecting(&mut self) {
        self.reconnects += 1;
        self.ping = None;
    }

    pub(crate) fn snapshot(&self, now: Instant) -> NetStats {
        let last = self.second(now).checked_sub(1);
        NetStats {
            rtt: self.rtt,
            rtt_avg: self.rtt_avg,
            inbound: self.inbound.traffic(last),
            outbound: self.outbound.traffic(last),
            reconnects: self.reconnects,
            motion: None,
        }
    }
}

/// One direction: totals, and the second being counted and the one before.
#[derive(Debug, Default)]
struct Direction {
    messages: u64,
    bytes: u64,
    current: Bucket,
    previous: Bucket,
}

#[derive(Debug, Default)]
struct Bucket {
    second: u64,
    kinds: BTreeMap<&'static str, (u32, u64)>,
}

impl Direction {
    fn count(&mut self, second: u64, kind: &'static str, bytes: usize) {
        let bytes = bytes as u64;
        self.messages += 1;
        self.bytes += bytes;
        if second != self.current.second {
            let current = std::mem::replace(
                &mut self.current,
                Bucket {
                    second,
                    kinds: BTreeMap::new(),
                },
            );
            // A gap leaves nothing worth keeping from before it.
            self.previous = if current.second + 1 == second {
                current
            } else {
                Bucket::default()
            };
        }
        let entry = self.current.kinds.entry(kind).or_default();
        entry.0 += 1;
        entry.1 += bytes;
    }

    /// The traffic, with the rates of second `last`.
    fn traffic(&self, last: Option<u64>) -> Traffic {
        let bucket = [&self.current, &self.previous]
            .into_iter()
            .find(|b| Some(b.second) == last && !b.kinds.is_empty());
        let mut per_second: Vec<TypeRate> = bucket
            .map(|b| {
                b.kinds
                    .iter()
                    .map(|(kind, (messages, bytes))| TypeRate {
                        kind,
                        messages: *messages,
                        bytes: *bytes,
                    })
                    .collect()
            })
            .unwrap_or_default();
        per_second.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.kind.cmp(b.kind)));
        Traffic {
            per_second,
            messages: self.messages,
            bytes: self.bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_says_it_all() {
        let t0 = Instant::now();
        let mut meter = Meter::new(t0);
        assert_eq!(
            meter.snapshot(t0).to_string(),
            "rtt unknown, in 0 msg/s 0.0 kB/s, out 0 msg/s 0.0 kB/s, 0 reconnects"
        );
        let ping = meter.ping(t0);
        meter.pong(t0 + ms(12), &ping);
        for i in 0..10 {
            meter.received(t0 + ms(100 * i), "positions", 150);
        }
        meter.received(t0 + ms(1000), "positions", 150);
        meter.reconnecting();
        let stats = meter.snapshot(t0 + ms(1000)).with_motion(MotionStats {
            corrections: 1,
            corrected: 7.6,
            ..MotionStats::default()
        });
        assert_eq!(
            stats.to_string(),
            "rtt 12 ms (avg 12 ms), in 10 msg/s 1.5 kB/s, out 0 msg/s 0.0 kB/s, \
             1 reconnect, 1 correction (8 u)"
        );
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn rates_are_the_last_whole_second() {
        let t0 = Instant::now();
        let mut meter = Meter::new(t0);
        for i in 0..10 {
            meter.received(t0 + ms(100 * i), "positions", 120);
        }
        meter.received(t0 + ms(500), "chat", 30);
        meter.sent(t0 + ms(200), "move", 40);

        // Still inside the first second: nothing whole to report yet.
        let stats = meter.snapshot(t0 + ms(900));
        assert!(stats.inbound.per_second.is_empty());
        assert_eq!((stats.inbound.messages, stats.inbound.bytes), (11, 1230));

        // Counting the second second moves the first into place.
        meter.received(t0 + ms(1100), "positions", 120);
        let stats = meter.snapshot(t0 + ms(1500));
        assert_eq!(
            stats.inbound.per_second,
            [
                TypeRate { kind: "positions", messages: 10, bytes: 1200 },
                TypeRate { kind: "chat", messages: 1, bytes: 30 },
            ]
        );
        assert_eq!(stats.inbound.messages_per_second(), 11);
        assert_eq!(stats.inbound.bytes_per_second(), 1230);
        assert_eq!(stats.outbound.rate("move").map(|r| r.messages), Some(1));

        // A quiet second reads as quiet, not as the last busy one.
        let stats = meter.snapshot(t0 + ms(3500));
        assert!(stats.inbound.per_second.is_empty());
        assert!(stats.outbound.per_second.is_empty());
        // After a gap, the old second isn't taken for the one before.
        meter.received(t0 + ms(5200), "positions", 120);
        assert!(meter.snapshot(t0 + ms(5300)).inbound.per_second.is_empty());
    }

    #[test]
    fn only_the_last_pings_pong_times_the_round_trip() {
        let t0 = Instant::now();
        let mut meter = Meter::new(t0);
        assert_eq!(meter.snapshot(t0).rtt, None);

        let first = meter.ping(t0);
        let second = meter.ping(t0 + ms(100));
        // The answer to a Ping that's been replaced, and a stranger's.
        meter.pong(t0 + ms(110), &first);
        meter.pong(t0 + ms(110), b"hello");
        assert_eq!(meter.snapshot(t0).rtt, None);

        meter.pong(t0 + ms(140), &second);
        let stats = meter.snapshot(t0);
        assert_eq!(stats.rtt, Some(ms(40)));
        assert_eq!(stats.rtt_avg, Some(ms(40)));
        // Answered once; a repeat doesn't count again.
        meter.pong(t0 + ms(400), &second);
        assert_eq!(meter.snapshot(t0).rtt, Some(ms(40)));

        let third = meter.ping(t0 + ms(1000));
        meter.pong(t0 + ms(1090), &third);
        let stats = meter.snapshot(t0);
        assert_eq!(stats.rtt, Some(ms(90)));
        let avg = stats.rtt_avg.unwrap().as_secs_f64() * 1000.0;
        assert!((avg - 50.0).abs() < 1e-3, "{avg} ms");
    }

    #[test]
    fn a_reconnect_is_counted_and_forgets_the_ping_in_flight() {
        let t0 = Instant::now();
        let mut meter = Meter::new(t0);
        let ping = meter.ping(t0);
        meter.reconnecting();
        meter.pong(t0 + ms(30), &ping);
        let stats = meter.snapshot(t0);
        assert_eq!(stats.reconnects, 1);
        assert_eq!(stats.rtt, None);
    }
}
