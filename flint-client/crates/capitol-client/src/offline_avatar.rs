//! `--offline-avatar [rtt_ms]` (controls T1.3): offline, you stand on the
//! offline terrain's ground and walk it, with a [`Loopback`] for a server
//! at that round trip. It walks you as the server would, on the same
//! [`Ground`], so Direct mode's walk (and a click's, in Click) can be
//! tried and captured with no server at all.
//!
//! You're placed where the camera looks half a second in, as the samples
//! are, once a `--view` or a one-key `--camera-path` has put it there.
//! The extension draws you through `entities::Players`, which sets the
//! follow point the camera rig takes up.
//!
//! On exit it prints `[motion] corrections N`, the snapshots that
//! disagreed with the prediction: on flat ground that should be 0. Then
//! `[motion] walked D from where you stood, throttles [..]`: how far off
//! you ended, in server units, and each throttle sent in turn, so a capture
//! can show a stick's half-speed walk, or drift that never moved you (T2.1).

use std::sync::Arc;
use std::time::{Duration, Instant};

use capitol_core::motion::{Loopback, Motion, Pose};
use capitol_core::protocol::rules::ground::Ground;
use capitol_core::protocol::ws::ClientMessage;
use capitol_core::state::GameState;
use capitol_core::steering::Steering;
use uuid::Uuid;

/// Your id offline, which `--citizen-sample` leaves out of its row (#515).
pub const ME: Uuid = Uuid::from_u128(1);

/// You, offline, and your server. See the module docs.
pub struct OfflineAvatar {
    round_trip: Duration,
    walking: Option<Walking>,
}

/// Once placed.
struct Walking {
    server: Loopback,
    state: GameState,
    motion: Motion,
    steering: Steering,
    /// Where you were stood, and where you're drawn now.
    start: (f64, f64),
    at: (f64, f64),
    /// Each throttle sent, when it changed.
    throttles: Vec<u8>,
}

impl OfflineAvatar {
    pub fn new(round_trip_ms: u64) -> Self {
        Self {
            round_trip: Duration::from_millis(round_trip_ms),
            walking: None,
        }
    }

    /// Whether you've been stood anywhere yet.
    pub fn placed(&self) -> bool {
        self.walking.is_some()
    }

    /// Stands you at `at`, in server units, on `ground`, connected at `now`.
    pub fn place(&mut self, now: Instant, at: (f64, f64), ground: Arc<Ground>) {
        let server = Loopback::new(now, ME, at, ground.clone())
            .named("You")
            .round_trip(self.round_trip);
        let mut motion = Motion::new();
        motion.set_ground(Some(ground));
        tracing::info!(
            "offline avatar: at ({:.0}, {:.0}), {} ms there and back",
            at.0,
            at.1,
            self.round_trip.as_millis()
        );
        self.walking = Some(Walking {
            server,
            state: GameState::new(),
            motion,
            steering: Steering::new(),
            start: at,
            at,
            throttles: Vec::new(),
        });
    }

    /// One frame: what the server has sent by `now`, then the keys' `way`
    /// ((0, 0) in Click mode, as for `Session::steer`) and a click's walk
    /// to `walk_to`, both in server units. Returns where to draw everyone,
    /// with the store they're drawn from. `None` until placed.
    pub fn update(&mut self, now: Instant, way: (f64, f64), walk_to: Option<(f64, f64)>) -> Option<(&[Pose], &GameState)> {
        let w = self.walking.as_mut()?;
        for event in w.server.run(now) {
            let changes = w.state.apply(&event);
            w.motion.observe(now, &changes, &w.state);
        }
        if let Some((x, y)) = walk_to {
            w.motion.move_to(now, x, y);
            w.server.send(now, ClientMessage::Move { x, y });
        }
        if let Some(message) = w.steering.drive(now, way, &mut w.motion) {
            if let ClientMessage::Steer { throttle, .. } = message {
                if w.throttles.last() != Some(&throttle) {
                    w.throttles.push(throttle);
                }
            }
            w.server.send(now, message);
        }
        let poses = w.motion.sample(now);
        if let Some(p) = poses.iter().find(|p| p.id == ME) {
            w.at = (p.x, p.y);
        }
        Some((poses, &w.state))
    }

    /// Where a click's walk is going, in server units, for the ring.
    pub fn destination(&self) -> Option<(f64, f64)> {
        self.walking.as_ref()?.motion.destination()
    }
}

impl Drop for OfflineAvatar {
    fn drop(&mut self) {
        if let Some(w) = &self.walking {
            eprintln!("[motion] corrections {}", w.motion.corrections());
            let walked = (w.at.0 - w.start.0).hypot(w.at.1 - w.start.1);
            eprintln!("[motion] walked {walked:.1} from where you stood, throttles {:?}", w.throttles);
        }
    }
}
