//! A fake server for one player's walk (controls T0.4): `move` and `steer`
//! walked at 10 Hz on the shared [`Ground`], with a set round trip and tick
//! phase, and the snapshots it sends back as [`NetEvent`]s. The tests use it
//! to play a walk at a given latency with no server, and the client's
//! `--offline-avatar` (T1.3) to walk the offline world.
//!
//! It walks as the server does: a steer by [`steer::take`] and
//! [`steer::tick`], the very functions `engine/movement.rs` calls, and a
//! click's walk by [`Ground::step`]. Each tick takes the messages that have
//! arrived, then walks, then sends `positions`, as `engine/tick.rs` does. A
//! block is told at once on a click's walk, and once in
//! [`steer::BLOCKED_TICKS`] on a steer. Dropping the connection takes the
//! avatar out of the world, steer and all; reconnecting puts it back where
//! it stood, still walking to a click's destination (it's saved) but not
//! steering (a steer never is).
//!
//! ```
//! # use std::{sync::Arc, time::{Duration, Instant}};
//! # use capitol_core::{motion::{Loopback, Motion}, state::GameState, protocol::rules::ground::Ground};
//! # use uuid::Uuid;
//! let start = Instant::now();
//! let mut server = Loopback::new(start, Uuid::from_u128(1), (4000.0, 4000.0), Arc::new(Ground::default()))
//!     .round_trip(Duration::from_millis(80));
//! let (mut state, mut motion) = (GameState::new(), Motion::new());
//! // once per frame:
//! let now = start + Duration::from_millis(16);
//! for event in server.run(now) {
//!     let changes = state.apply(&event);
//!     motion.observe(now, &changes, &state);
//! }
//! // and each message the client sends: server.send(now, message);
//! ```

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use capitol_protocol::rules::ground::{Ground, Step};
use capitol_protocol::rules::steer::{self, Steering};
use capitol_protocol::rules::{ARRIVAL_EPSILON, PLAYER_SPEED, TICK_SECS};
use capitol_protocol::ws::{ClientMessage, PlayerPosition, ServerMessage};
use uuid::Uuid;

use super::TICK;
use crate::net::{ConnectionState, NetEvent};

/// One player's server. See the module docs.
#[derive(Debug, Clone)]
pub struct Loopback {
    ground: Arc<Ground>,
    id: Uuid,
    /// Half the round trip: each way.
    half_trip: Duration,
    next_tick: Instant,
    ticks: u64,
    at: (f64, f64),
    dest: Option<(f64, f64)>,
    steer: Option<Steering>,
    /// When a steer was last told it's blocked, by tick.
    blocked_at: Option<u64>,
    connected: bool,
    /// Messages on their way to the server, by when they land.
    inbound: VecDeque<(Instant, ClientMessage)>,
    /// Events on their way to the client, by when they land.
    outbound: VecDeque<(Instant, NetEvent)>,
}

impl Loopback {
    /// A server with player `id` standing at `at`, connected at `now`, with
    /// no round trip and its first tick at `now`.
    pub fn new(now: Instant, id: Uuid, at: (f64, f64), ground: Arc<Ground>) -> Self {
        let mut server = Self {
            ground,
            id,
            half_trip: Duration::ZERO,
            next_tick: now,
            ticks: 0,
            at,
            dest: None,
            steer: None,
            blocked_at: None,
            connected: false,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
        };
        server.connect(now, false);
        server
    }

    /// The time a message takes there and back. Set it before the first
    /// [`Loopback::run`].
    pub fn round_trip(mut self, round_trip: Duration) -> Self {
        let connected = self.outbound.iter_mut().map(|(at, _)| at);
        for at in connected {
            *at = *at - self.half_trip + round_trip / 2;
        }
        self.half_trip = round_trip / 2;
        self
    }

    /// Delays the ticks by `phase` (under a tick), so they don't fall on
    /// the client's.
    pub fn tick_phase(mut self, phase: Duration) -> Self {
        self.next_tick += phase;
        self
    }

    /// Where the server has the player.
    pub fn position(&self) -> (f64, f64) {
        self.at
    }

    /// Whether the server has a steer under way.
    pub fn steering(&self) -> bool {
        self.steer.is_some()
    }

    /// The client sends `message` at `now`. Lost while disconnected.
    pub fn send(&mut self, now: Instant, message: ClientMessage) {
        if self.connected {
            self.inbound.push_back((now + self.half_trip, message));
        }
    }

    /// Runs the server's ticks up to `now`, and returns what has reached
    /// the client by then.
    pub fn run(&mut self, now: Instant) -> Vec<NetEvent> {
        while self.next_tick <= now {
            let at = self.next_tick;
            self.tick(at);
            self.next_tick += TICK;
        }
        let mut arrived = Vec::new();
        while self.outbound.front().is_some_and(|(at, _)| *at <= now) {
            arrived.extend(self.outbound.pop_front().map(|(_, event)| event));
        }
        arrived
    }

    /// The connection drops at `now`: the avatar leaves the world, and what
    /// was on its way either way is lost.
    pub fn disconnect(&mut self, now: Instant) {
        self.connected = false;
        self.steer = None;
        self.inbound.clear();
        self.outbound.clear();
        self.outbound.push_back((
            now,
            NetEvent::Connection(ConnectionState::Reconnecting {
                attempt: 1,
                delay: Duration::ZERO,
                reason: "loopback".into(),
            }),
        ));
    }

    /// The client connects again at `now`: the avatar is back where it
    /// stood, and the client hears so, and to resync, on the way back.
    pub fn reconnect(&mut self, now: Instant) {
        self.connect(now, true);
    }

    fn connect(&mut self, now: Instant, resync: bool) {
        self.connected = true;
        let player_id = self.id;
        let at = now + self.half_trip;
        self.outbound
            .push_back((at, NetEvent::Connection(ConnectionState::Connected { player_id })));
        if resync {
            self.outbound.push_back((at, NetEvent::Resync));
        }
    }

    fn tick(&mut self, now: Instant) {
        self.ticks += 1;
        if !self.connected {
            return;
        }
        while self.inbound.front().is_some_and(|(at, _)| *at <= now) {
            if let Some((_, message)) = self.inbound.pop_front() {
                self.take(message);
            }
        }
        let walk = PLAYER_SPEED * TICK_SECS;
        let steering = self.steer.is_some();
        let step = match self.dest {
            _ if steering => Some(steer::tick(&mut self.steer, self.at, &self.ground, walk)),
            Some(dest) => Some(self.dest_step(dest, walk)),
            None => None,
        };
        if let Some(step) = step {
            self.at = (step.x, step.y);
            if let Some(blocked) = step.blocked {
                let quiet = steering
                    && self
                        .blocked_at
                        .is_some_and(|said| self.ticks < said + steer::BLOCKED_TICKS);
                if !quiet {
                    if steering {
                        self.blocked_at = Some(self.ticks);
                    }
                    self.tell(
                        now,
                        ServerMessage::MovementBlocked {
                            player_id: self.id,
                            reason: blocked.reason(),
                            water_id: blocked.water_id().to_string(),
                            stopped_x: self.at.0,
                            stopped_y: self.at.1,
                        },
                    );
                }
            }
        }
        let players = vec![PlayerPosition {
            id: self.id,
            name: "loopback".into(),
            x: self.at.0,
            y: self.at.1,
            citizen: false,
        }];
        self.tell(now, ServerMessage::Positions { players });
    }

    fn take(&mut self, message: ClientMessage) {
        match message {
            ClientMessage::Move { x, y } => {
                self.steer = None;
                self.dest = Some((x, y));
            }
            ClientMessage::Steer { heading, throttle, at } => {
                let at = at.map(|[x, y]| (x, y));
                if steer::take(&mut self.steer, self.at, heading, throttle, at) {
                    self.dest = None;
                }
            }
            _ => {}
        }
    }

    /// A tick toward a click's destination, as `dest_step` walks it.
    fn dest_step(&mut self, dest: (f64, f64), walk: f64) -> Step {
        let (dx, dy) = (dest.0 - self.at.0, dest.1 - self.at.1);
        if (dx * dx + dy * dy).sqrt() < ARRIVAL_EPSILON {
            self.dest = None;
            return Step {
                x: dest.0,
                y: dest.1,
                blocked: None,
            };
        }
        let step = self.ground.step(self.at, dest, walk);
        if step.blocked.is_some() || (step.x, step.y) == dest {
            self.dest = None;
        }
        step
    }

    fn tell(&mut self, now: Instant, message: ServerMessage) {
        self.outbound.push_back((now + self.half_trip, NetEvent::Ws(message)));
    }
}
