//! Walking on a heading (controls plan D3, T0.4): the movement input each
//! frame, turned into `steer`s for the server. [`Steering`] is pure; it
//! decides what to send and when, and [`Motion::steer`] predicts it.
//!
//! ```no_run
//! # use std::time::Instant;
//! # use capitol_core::{motion::Motion, net::*, steering::Steering};
//! # let net = Net::spawn(NetConfig::default()).unwrap();
//! # let mut motion = Motion::new();
//! let mut steering = Steering::new();
//! // once per frame, with the way the keys or the stick point, in server
//! // units (y south), as long as their deflection:
//! # let way = (1.0, 0.0);
//! if let Some(message) = steering.drive(Instant::now(), way, &mut motion) {
//!     net.ws(message);
//! }
//! ```
//!
//! - **Quantised.** The heading is a `u16` ([`steer::heading_of`]) and the
//!   throttle tenths: keys give 10, a stick 0, 5 or 10, so feet don't slide
//!   on the walk's yes/no blend. A turn under [`MIN_TURN`] isn't sent.
//! - **Sent on change**, at most [`RATE`] a second: a bucket of [`BURST`].
//!   A turn leaves a token in it, so a stop or a start is never held up.
//!   It's renewed every [`RENEW`] while held, inside the server's lease, and
//!   on release a stop goes with `at` where the prediction stopped.
//! - **The latch.** After an interaction ([`Steering::latch`]) it stops the
//!   walk and sends nothing more until the input goes back to neutral, so a
//!   held W doesn't cancel the gather that E just started.
//! - **A reconnect** forgets what was last sent, so a held walk is sent
//!   again: the server never keeps a steer across one.

use std::time::{Duration, Instant};

use capitol_protocol::rules::steer::{self, FULL};
use capitol_protocol::ws::ClientMessage;

use crate::motion::Motion;

/// The most steers a second, held steady.
pub const RATE: f64 = 8.0;

/// The bucket's size. A turn needs two tokens and spends one, so one is
/// always left for a stop or a start.
pub const BURST: f64 = 2.0;

/// How often a held walk is sent again, well inside the lease
/// ([`steer::LEASE_TICKS`], 3 s).
pub const RENEW: Duration = Duration::from_secs(1);

/// The least turn worth sending: half a degree, in heading units.
pub const MIN_TURN: u16 = 91;

/// Below this deflection the input is neutral.
pub const NEUTRAL: f64 = 0.25;

/// Below this deflection (and above [`NEUTRAL`]), a half-speed walk.
pub const HALF: f64 = 0.75;

/// One steer to send: a heading, and a throttle in tenths (0 is a stop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Steer {
    pub heading: u16,
    pub throttle: u8,
}

impl Steer {
    /// The wire message, with `at` from [`Motion::steer`].
    pub fn message(self, at: Option<(f64, f64)>) -> ClientMessage {
        ClientMessage::Steer {
            heading: self.heading,
            throttle: self.throttle,
            at: at.map(|(x, y)| [x, y]),
        }
    }
}

/// Turns the movement input into steers. See the module docs.
#[derive(Debug, Clone)]
pub struct Steering {
    /// The walk last sent, or `None` standing.
    walking: Option<Steer>,
    /// When anything was last sent, for the renewal.
    sent_at: Option<Instant>,
    tokens: f64,
    refilled: Option<Instant>,
    latched: bool,
    /// The [`Motion::placed`] session this was last driven in.
    session: Option<u64>,
}

impl Default for Steering {
    fn default() -> Self {
        Self::new()
    }
}

impl Steering {
    pub fn new() -> Self {
        Self {
            walking: None,
            sent_at: None,
            tokens: BURST,
            refilled: None,
            latched: false,
            session: None,
        }
    }

    /// The walk last sent, or `None` standing.
    pub fn walking(&self) -> Option<Steer> {
        self.walking
    }

    /// Latched after an interaction, until the input is neutral.
    pub fn latched(&self) -> bool {
        self.latched
    }

    /// One frame of input: `way` is the way to walk in server units (y
    /// south), as long as the input's deflection, 1 for keys. Returns the
    /// steer to send now, if any; predict it with [`Motion::steer`] for
    /// the `at` to send with it. [`Steering::drive`] does both.
    pub fn update(&mut self, now: Instant, way: (f64, f64)) -> Option<Steer> {
        self.refill(now);
        let throttle = throttle_of(way.0.hypot(way.1));
        if self.latched {
            if throttle == 0 {
                self.latched = false;
            }
            return self.stop(now);
        }
        if throttle == 0 {
            return self.stop(now);
        }
        let want = Steer {
            heading: steer::heading_of(way.0, way.1),
            throttle,
        };
        let Some(walking) = self.walking else {
            return self.send(now, want, 1.0);
        };
        let turned = turn(walking.heading, want.heading) >= MIN_TURN;
        if turned || want.throttle != walking.throttle {
            return self.send(now, want, 2.0);
        }
        let due = self.sent_at.is_none_or(|at| now.duration_since(at) >= RENEW);
        if due {
            return self.send(now, walking, 1.0);
        }
        None
    }

    /// An interaction has begun: stop the walk, and send nothing more until
    /// the input is neutral.
    pub fn latch(&mut self) {
        self.latched = true;
    }

    /// The session reconnected: the server has no steer, so a held walk
    /// must be sent again. [`Steering::drive`] notices on its own.
    pub fn reconnected(&mut self) {
        self.walking = None;
        self.sent_at = None;
    }

    /// [`Steering::update`] and [`Motion::steer`] together: the message to
    /// send this frame, if any. Nothing goes before the first snapshot
    /// places the player, and a new session (a reconnect) sends a held walk
    /// again.
    pub fn drive(&mut self, now: Instant, way: (f64, f64), motion: &mut Motion) -> Option<ClientMessage> {
        let session = motion.placed()?;
        if self.session != Some(session) {
            self.session = Some(session);
            self.reconnected();
        }
        let steer = self.update(now, way)?;
        let at = motion.steer(now, steer.heading, steer.throttle);
        Some(steer.message(at))
    }

    /// A stop, if walking and a token allows.
    fn stop(&mut self, now: Instant) -> Option<Steer> {
        let walking = self.walking?;
        self.send(
            now,
            Steer {
                heading: walking.heading,
                throttle: 0,
            },
            1.0,
        )
    }

    /// Sends `steer` if the bucket holds `needs` tokens, spending one.
    fn send(&mut self, now: Instant, steer: Steer, needs: f64) -> Option<Steer> {
        if self.tokens < needs {
            return None;
        }
        self.tokens -= 1.0;
        self.sent_at = Some(now);
        self.walking = (steer.throttle > 0).then_some(steer);
        Some(steer)
    }

    fn refill(&mut self, now: Instant) {
        if let Some(then) = self.refilled {
            let secs = now.saturating_duration_since(then).as_secs_f64();
            self.tokens = (self.tokens + secs * RATE).min(BURST);
        }
        self.refilled = Some(now);
    }
}

/// The throttle for a deflection: 0, 5 or 10.
pub fn throttle_of(deflection: f64) -> u8 {
    if deflection < NEUTRAL {
        0
    } else if deflection < HALF {
        FULL / 2
    } else {
        FULL
    }
}

/// The way to walk for movement input held against the view (T1.3):
/// `forward` and `right` from -1 to 1, and the camera rig's `yaw` (radians
/// round the point it looks at; 0 stands it south, looking north). In
/// server units (y south), as [`Steering::update`] takes it: never longer
/// than 1, so two keys held walk no faster than one.
pub fn camera_way(forward: f64, right: f64, yaw: f64) -> (f64, f64) {
    // Forward is from the camera to the point, (-sin, -cos) in x and y;
    // right is (cos, -sin). Server y is world z, so the rig's are these.
    let (s, c) = (yaw.sin(), yaw.cos());
    let x = -s * forward + c * right;
    let y = -c * forward - s * right;
    let len = x.hypot(y);
    if len > 1.0 {
        (x / len, y / len)
    } else {
        (x, y)
    }
}

/// How far apart two headings are, either way round.
fn turn(a: u16, b: u16) -> u16 {
    let d = a.wrapping_sub(b);
    d.min(d.wrapping_neg())
}

#[cfg(test)]
mod tests;
