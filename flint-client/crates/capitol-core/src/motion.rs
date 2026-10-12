//! Smooth positions between snapshots. The server sends `positions` every tick
//! (100 ms) with no velocity (`broadcast_positions` in `engine/tick.rs`), so
//! drawing those as they come would step every avatar ten times a second.
//! [`Motion`] turns them into something to draw each frame.
//!
//! ```no_run
//! # use std::time::Instant;
//! # use capitol_core::{motion::*, net::*, protocol::ws::ClientMessage, state::*};
//! # let net = Net::spawn(NetConfig::default()).unwrap();
//! let mut state = GameState::new();
//! let mut motion = Motion::new();
//! // once per frame:
//! let now = Instant::now();
//! for event in net.drain() {
//!     let changes = state.apply(&event);
//!     motion.observe(now, &changes, &state);
//! }
//! // on a click: predict, and tell the server
//! # let (x, y) = (4100.0, 4000.0);
//! motion.move_to(now, x, y);
//! net.ws(ClientMessage::Move { x, y });
//! for pose in motion.sample(now) {
//!     // place pose.id's avatar at (pose.x, pose.y), facing pose.facing
//! }
//! ```
//!
//! **Other players** are drawn [`MotionConfig::interp_delay`] (150 ms) in the
//! past, lerped between the two snapshots either side of that moment. When
//! snapshots run late, the last velocity carries them on for at most
//! [`MotionConfig::max_extrapolation`], then they hold still. A jump no walk
//! could make between two snapshots is a teleport: it snaps.
//!
//! **Snapshot times.** A snapshot is stamped when the game drains it, but
//! network jitter bunches arrivals. Because the server ticks on a fixed
//! interval (`tokio::time::interval`, which bursts to catch up), each snapshot
//! is 100 ms of server time after the last. So a snapshot is stamped one tick
//! after the previous stamp, never later than its arrival, and never more than
//! [`MotionConfig::max_backdate`] before it. Three snapshots that arrive
//! together after a stall are spread back over 300 ms instead of piling onto one
//! instant. The timeline also slews toward arrivals by [`MotionConfig::slew`]
//! per snapshot, so a snapshot that never comes can't leave it behind for good.
//! That can't happen on this socket (a lagging client is disconnected, and a
//! late tick is still sent), so the slew is only a backstop: a lost snapshot
//! costs one small hitch.
//!
//! **The local player** is predicted. [`Motion::move_to`] walks toward the
//! destination at [`MotionConfig::speed`] (50 u/s, the server's default),
//! snapping on within `ARRIVAL_EPSILON` as `engine/movement.rs` does. Each
//! snapshot is checked against where the prediction has *been* lately: the
//! server lags the prediction by about one round trip, so its position should
//! lie on the trail the prediction left behind. If it's more than
//! [`MotionConfig::reconcile_distance`] (4 units) off that trail, the
//! prediction moves to where the server will be by now, and the drawn position
//! blends there over [`MotionConfig::blend_time`]. Further off than
//! [`MotionConfig::teleport_distance`], it snaps. So does `movement_blocked`.
//! Starting to extract or craft stops the prediction, because the server
//! clears the destination then. A server that stops while the prediction
//! walks (the move was lost) pulls it back. A server that walks while the
//! prediction stands (it resumed a stored destination on reconnect) is
//! followed, a step ahead on its heading, until it stops.
//!
//! [`Motion::sample`] fills a buffer the `Motion` owns and returns it, so it
//! doesn't allocate once the buffer has grown to the number of players.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use capitol_protocol::rules::{ARRIVAL_EPSILON, PLAYER_SPEED, TICK_MS};
use uuid::Uuid;

use crate::state::{GameState, StateEvent};

/// The server's tick: the time between two snapshots.
const TICK: Duration = Duration::from_millis(TICK_MS);

/// Snapshots kept per remote player. Interpolation needs two; the rest cover
/// a burst after a stall.
const TRACK_LEN: usize = 8;

/// Points kept on the local player's trail, and the least time between two.
/// 64 points at 15 ms cover a second, twice the default window.
const TRAIL_LEN: usize = 64;
const TRAIL_STEP: Duration = Duration::from_millis(15);

/// A player who moves less than this per second is standing still.
const STILL_SPEED: f64 = 0.5;

/// Tuning. The defaults follow plan §4.5.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionConfig {
    /// How far in the past other players are drawn.
    pub interp_delay: Duration,
    /// How long a late player keeps moving on its last velocity before it
    /// holds still.
    pub max_extrapolation: Duration,
    /// The furthest a snapshot is stamped before its arrival.
    pub max_backdate: Duration,
    /// How far the timeline catches up per snapshot when snapshots arrive
    /// later than it expects.
    pub slew: Duration,
    /// The local player's speed in units per second.
    pub speed: f64,
    /// How far the server may stray from the local player's trail before the
    /// prediction corrects.
    pub reconcile_distance: f64,
    /// A jump bigger than this snaps rather than slides or blends: another
    /// player's between two snapshots, or the local player's correction.
    pub teleport_distance: f64,
    /// Time constant of the local player's blend back after a correction.
    pub blend_time: Duration,
    /// How much of the local player's trail a snapshot is checked against.
    /// It must cover a round trip plus a tick.
    pub reconcile_window: Duration,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            interp_delay: Duration::from_millis(150),
            max_extrapolation: Duration::from_millis(150),
            max_backdate: Duration::from_millis(200),
            slew: Duration::from_millis(5),
            speed: PLAYER_SPEED,
            reconcile_distance: 4.0,
            // A walk covers 5 units a tick, so this is ten ticks of it. The
            // local player can run 35 units ahead of a server that stopped
            // before the window notices, and that should still blend.
            teleport_distance: 50.0,
            blend_time: Duration::from_millis(100),
            reconcile_window: Duration::from_millis(500),
        }
    }
}

/// Where to draw one player this frame, in server units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub id: Uuid,
    pub x: f64,
    pub y: f64,
    /// Heading in radians, in server space: 0 is east, π/2 is south (y
    /// points down). It keeps the last heading while standing still.
    pub facing: f64,
    /// Walking, as opposed to standing or held still while late.
    pub moving: bool,
}

/// Turns 10 Hz snapshots into per-frame poses. See the module docs.
#[derive(Debug, Clone)]
pub struct Motion {
    config: MotionConfig,
    /// The stamp of the last snapshot. `None` until one arrives, and again
    /// after a reconnect.
    last_stamp: Option<Instant>,
    tracks: HashMap<Uuid, Track>,
    local: Option<Local>,
    poses: Vec<Pose>,
}

impl Default for Motion {
    fn default() -> Self {
        Self::new()
    }
}

impl Motion {
    pub fn new() -> Self {
        Self::with_config(MotionConfig::default())
    }

    pub fn with_config(config: MotionConfig) -> Self {
        Self {
            config,
            last_stamp: None,
            tracks: HashMap::new(),
            local: None,
            poses: Vec::new(),
        }
    }

    pub fn config(&self) -> &MotionConfig {
        &self.config
    }

    /// Takes what [`GameState::apply`] reported for one net event. `now` is
    /// when the game drained it.
    pub fn observe(&mut self, now: Instant, changes: &[StateEvent], state: &GameState) {
        for change in changes {
            match change {
                StateEvent::SnapshotStale => self.reset(),
                StateEvent::PositionsUpdated => self.positions(now, state),
                StateEvent::PlayerLeft(id) => {
                    self.tracks.remove(id);
                }
                StateEvent::MovementBlocked { x, y, .. } => {
                    if let Some(local) = &mut self.local {
                        local.snap(now, *x, *y);
                    }
                }
                StateEvent::ExtractionStarted { .. } | StateEvent::CraftingStarted { .. } => {
                    if let Some(local) = &mut self.local {
                        local.advance(now, &self.config);
                        local.stop();
                    }
                }
                _ => {}
            }
        }
    }

    /// The local player clicked `(x, y)`. Send the server the matching
    /// `ClientMessage::Move` too. Does nothing before the first snapshot has
    /// placed the local player.
    pub fn move_to(&mut self, now: Instant, x: f64, y: f64) {
        if let Some(local) = &mut self.local {
            if local.synced {
                local.advance(now, &self.config);
                local.following = false;
                local.dest = Some((x, y));
            }
        }
    }

    /// Where the local player clicked to walk to, for a destination marker.
    /// `None` when standing, or when following a walk the server resumed.
    pub fn destination(&self) -> Option<(f64, f64)> {
        self.local
            .as_ref()
            .filter(|local| !local.following)
            .and_then(|local| local.dest)
    }

    /// Every player's pose at `now`, the local player's first. The slice is
    /// reused from call to call.
    pub fn sample(&mut self, now: Instant) -> &[Pose] {
        self.poses.clear();
        if let Some(local) = &mut self.local {
            if local.synced {
                local.advance(now, &self.config);
                self.poses.push(local.pose());
            }
        }
        let at = now.checked_sub(self.config.interp_delay).unwrap_or(now);
        for (id, track) in &mut self.tracks {
            self.poses
                .push(track.pose(*id, at, self.config.max_extrapolation));
        }
        &self.poses
    }

    /// One player's pose from the last [`Motion::sample`].
    pub fn pose(&self, id: Uuid) -> Option<&Pose> {
        self.poses.iter().find(|pose| pose.id == id)
    }

    /// How many snapshots have disagreed with the local player's prediction
    /// this session, for a debug overlay. A steady climb while walking means
    /// the prediction and the server don't agree on how players move.
    pub fn corrections(&self) -> u32 {
        self.local.as_ref().map_or(0, |local| local.corrections)
    }

    /// How far the server runs behind the local player's prediction, as
    /// learned from the snapshots: about a round trip plus half a tick.
    pub fn lag(&self) -> Option<Duration> {
        self.local
            .as_ref()
            .filter(|local| local.synced)
            .map(|local| Duration::from_secs_f64(local.lag))
    }

    /// The session reconnected, or connected for the first time. Positions may
    /// have jumped while it was away, so everything snaps to the next snapshot.
    fn reset(&mut self) {
        self.last_stamp = None;
        self.tracks.clear();
        if let Some(local) = &mut self.local {
            local.synced = false;
            local.stop();
        }
    }

    fn positions(&mut self, now: Instant, state: &GameState) {
        let stamp = self.stamp(now);
        let me = state.me();
        if self.local.as_ref().map(|local| local.id) != me {
            self.local = me.map(Local::new);
        }
        for (id, player) in state.players() {
            if Some(*id) == me {
                if let Some(local) = &mut self.local {
                    local.reconcile(now, player.x, player.y, &self.config);
                }
                continue;
            }
            let sample = Sample {
                at: stamp,
                x: player.x,
                y: player.y,
            };
            match self.tracks.get_mut(id) {
                Some(track) => track.push(sample, self.config.teleport_distance),
                None => {
                    self.tracks.insert(*id, Track::new(sample));
                }
            }
        }
        let players = state.players();
        self.tracks.retain(|id, _| players.contains_key(id));
    }

    /// The server time a snapshot that arrived at `arrival` stands for, on the
    /// client's clock. See "Snapshot times" in the module docs.
    fn stamp(&mut self, arrival: Instant) -> Instant {
        let stamp = match self.last_stamp {
            None => arrival,
            Some(prev) => {
                let mut ideal = prev + TICK;
                if arrival > ideal {
                    ideal += self.config.slew.min(arrival - ideal);
                }
                let floor = arrival
                    .checked_sub(self.config.max_backdate)
                    .unwrap_or(arrival);
                // Never before the last stamp, so stamps never go backwards.
                ideal.min(arrival).max(floor).max(prev)
            }
        };
        self.last_stamp = Some(stamp);
        stamp
    }
}

// ---------------------------------------------------------------------------
// Samples
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    at: Instant,
    x: f64,
    y: f64,
}

/// The last `N` samples, oldest first, in a fixed array.
#[derive(Debug, Clone)]
struct History<const N: usize> {
    samples: [Sample; N],
    len: usize,
}

impl<const N: usize> History<N> {
    fn new(first: Sample) -> Self {
        Self {
            samples: [first; N],
            len: 1,
        }
    }

    fn push(&mut self, sample: Sample) {
        if self.len == N {
            self.samples.copy_within(1.., 0);
            self.samples[N - 1] = sample;
        } else {
            self.samples[self.len] = sample;
            self.len += 1;
        }
    }

    fn reset(&mut self, sample: Sample) {
        self.samples[0] = sample;
        self.len = 1;
    }

    fn as_slice(&self) -> &[Sample] {
        &self.samples[..self.len]
    }

    fn last(&self) -> &Sample {
        &self.samples[self.len - 1]
    }
}

// ---------------------------------------------------------------------------
// Other players
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Track {
    history: History<TRACK_LEN>,
    facing: f64,
}

impl Track {
    fn new(first: Sample) -> Self {
        Self {
            history: History::new(first),
            facing: 0.0,
        }
    }

    /// Adds a snapshot. A jump too big to walk drops the history, so the
    /// player snaps there instead of sliding across the map.
    fn push(&mut self, sample: Sample, teleport_distance: f64) {
        let last = self.history.last();
        if distance(last.x, last.y, sample.x, sample.y) > teleport_distance {
            self.history.reset(sample);
        } else {
            self.history.push(sample);
        }
    }

    fn pose(&mut self, id: Uuid, at: Instant, max_extrapolation: Duration) -> Pose {
        let (x, y, vx, vy) = sample_at(self.history.as_slice(), at, max_extrapolation);
        let moving = vx.hypot(vy) > STILL_SPEED;
        if moving {
            self.facing = vy.atan2(vx);
        }
        Pose {
            id,
            x,
            y,
            facing: self.facing,
            moving,
        }
    }
}

/// Position and velocity at `at`: lerped between the samples either side of
/// it, carried on from the last pair for up to `max_extrapolation` past the
/// newest, then held. Velocity is zero while held.
fn sample_at(samples: &[Sample], at: Instant, max_extrapolation: Duration) -> (f64, f64, f64, f64) {
    let first = samples[0];
    if at <= first.at {
        return (first.x, first.y, 0.0, 0.0);
    }
    for pair in samples.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a.at <= at && at < b.at {
            let span = (b.at - a.at).as_secs_f64();
            let t = (at - a.at).as_secs_f64() / span;
            let (vx, vy) = ((b.x - a.x) / span, (b.y - a.y) / span);
            return (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, vx, vy);
        }
    }
    // Past the newest: carry on from the latest pair with distinct stamps.
    let last = samples[samples.len() - 1];
    let pair = samples
        .windows(2)
        .rev()
        .find(|pair| pair[1].at > pair[0].at);
    let Some(pair) = pair else {
        return (last.x, last.y, 0.0, 0.0);
    };
    let (a, b) = (pair[0], pair[1]);
    let span = (b.at - a.at).as_secs_f64();
    let (vx, vy) = ((b.x - a.x) / span, (b.y - a.y) / span);
    let late = at - last.at;
    let ahead = late.min(max_extrapolation).as_secs_f64();
    let (x, y) = (last.x + vx * ahead, last.y + vy * ahead);
    if late >= max_extrapolation {
        (x, y, 0.0, 0.0)
    } else {
        (x, y, vx, vy)
    }
}

// ---------------------------------------------------------------------------
// The local player
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Local {
    id: Uuid,
    /// False until a snapshot places the player, and again after a reconnect.
    synced: bool,
    /// The predicted position, without the blend offset.
    x: f64,
    y: f64,
    dest: Option<(f64, f64)>,
    /// The server is walking somewhere the player didn't click: it reloads
    /// the stored destination on login and reconnect (`load_player` in
    /// `ws/connection.rs`). `dest` is then a point a step ahead of the server
    /// on its heading, renewed by every snapshot.
    following: bool,
    /// Added to the prediction when drawing. It decays to zero, so a
    /// correction blends instead of jumping.
    offset: (f64, f64),
    facing: f64,
    /// When the prediction was last advanced.
    advanced: Instant,
    /// Where the prediction has been, for checking snapshots against.
    trail: History<TRAIL_LEN>,
    /// How far the server runs behind the prediction, learned from where its
    /// snapshots land on the trail. Starts at one tick.
    lag: f64,
    /// The server's last reported position.
    server: (f64, f64),
    /// How many snapshots have disagreed with the prediction.
    corrections: u32,
}

impl Local {
    fn new(id: Uuid) -> Self {
        // The placeholder instant is never read before `snap` replaces it.
        let now = Instant::now();
        Self {
            id,
            synced: false,
            x: 0.0,
            y: 0.0,
            dest: None,
            following: false,
            offset: (0.0, 0.0),
            facing: 0.0,
            advanced: now,
            trail: History::new(Sample {
                at: now,
                x: 0.0,
                y: 0.0,
            }),
            lag: TICK.as_secs_f64(),
            server: (0.0, 0.0),
            corrections: 0,
        }
    }

    fn pose(&self) -> Pose {
        Pose {
            id: self.id,
            x: self.x + self.offset.0,
            y: self.y + self.offset.1,
            facing: self.facing,
            moving: self.dest.is_some(),
        }
    }

    fn stop(&mut self) {
        self.dest = None;
        self.following = false;
    }

    /// Jumps straight to `(x, y)` and stops: no blend, no trail.
    fn snap(&mut self, now: Instant, x: f64, y: f64) {
        self.synced = true;
        self.x = x;
        self.y = y;
        self.stop();
        self.offset = (0.0, 0.0);
        self.advanced = now;
        self.trail.reset(Sample { at: now, x, y });
        self.server = (x, y);
    }

    /// Walks toward the destination up to `now`, decays the blend and records
    /// the trail.
    fn advance(&mut self, now: Instant, config: &MotionConfig) {
        if !self.synced || now <= self.advanced {
            return;
        }
        let dt = (now - self.advanced).as_secs_f64();
        self.advanced = now;

        if let Some((dx, dy)) = self.dest {
            let (ex, ey) = (dx - self.x, dy - self.y);
            let remaining = ex.hypot(ey);
            let step = config.speed * dt;
            if remaining < ARRIVAL_EPSILON || step >= remaining {
                self.x = dx;
                self.y = dy;
                self.dest = None;
            } else {
                self.x += ex / remaining * step;
                self.y += ey / remaining * step;
            }
            if remaining > 0.0 {
                self.facing = ey.atan2(ex);
            }
        }

        let keep = (-dt / config.blend_time.as_secs_f64()).exp();
        self.offset = (self.offset.0 * keep, self.offset.1 * keep);
        if self.offset.0.hypot(self.offset.1) < 1e-3 {
            self.offset = (0.0, 0.0);
        }

        if now - self.trail.last().at >= TRAIL_STEP {
            self.trail.push(Sample {
                at: now,
                x: self.x,
                y: self.y,
            });
        }
    }

    /// Checks the server's position against the trail and corrects the
    /// prediction if it has strayed. See the module docs.
    fn reconcile(&mut self, now: Instant, sx: f64, sy: f64, config: &MotionConfig) {
        if !self.synced {
            self.snap(now, sx, sy);
            return;
        }
        self.advance(now, config);
        let (mx, my) = (sx - self.server.0, sy - self.server.1);
        let moved = mx.hypot(my);
        let server_moved = moved > 1e-6;
        if moved > config.teleport_distance {
            // No walk does that in a tick.
            self.snap(now, sx, sy);
            return;
        }
        self.server = (sx, sy);
        // Where the server will be by now, if it walks on the way it went.
        let ahead = |lag: f64| {
            let step = config.speed * lag;
            (sx + mx / moved * step, sy + my / moved * step)
        };

        if self.following {
            if server_moved {
                // Stay a tick ahead of where it is by now.
                self.dest = Some(ahead(self.lag + TICK.as_secs_f64()));
            } else {
                // It stopped, right where it says.
                self.stop();
                self.correct(now, sx, sy, sx, sy, config);
                return;
            }
        }

        let since = now.checked_sub(config.reconcile_window).unwrap_or(now);
        let current = Sample {
            at: self.advanced,
            x: self.x,
            y: self.y,
        };
        let (off, lag, on_a_walk) =
            nearest_on_trail(self.trail.as_slice(), current, since, now, sx, sy);
        if off <= config.reconcile_distance {
            if on_a_walk {
                let window = config.reconcile_window.as_secs_f64();
                self.lag = (self.lag * 0.8 + lag * 0.2).clamp(0.0, window);
            }
            return;
        }

        // The server's position is a round trip old. If we're walking and so
        // is the server, it's walking to our destination: aim for where it is
        // by now. If it has stopped, it isn't following us: stop too. If it's
        // walking on its own, follow it.
        let target = match (self.dest, server_moved) {
            (Some((dx, dy)), true) => toward(sx, sy, dx, dy, config.speed * self.lag),
            (_, false) => {
                self.stop();
                (sx, sy)
            }
            (None, true) => {
                self.following = true;
                self.dest = Some(ahead(self.lag + TICK.as_secs_f64()));
                ahead(self.lag)
            }
        };
        self.correct(now, sx, sy, target.0, target.1, config);
    }

    /// Moves the prediction to `(tx, ty)`, blending the drawn position there,
    /// or snapping if it's too far. `(sx, sy)` is the server's report, which
    /// starts the new trail.
    fn correct(&mut self, now: Instant, sx: f64, sy: f64, tx: f64, ty: f64, config: &MotionConfig) {
        self.corrections += 1;
        if distance(tx, ty, self.x, self.y) > config.teleport_distance {
            self.snap(now, tx, ty);
            return;
        }
        let shown = (self.x + self.offset.0, self.y + self.offset.1);
        self.offset = (shown.0 - tx, shown.1 - ty);
        self.x = tx;
        self.y = ty;
        // The path the server is on, so the next snapshot lands on the trail.
        let lag = Duration::from_secs_f64(self.lag);
        self.trail.reset(Sample {
            at: now.checked_sub(lag).unwrap_or(now),
            x: sx,
            y: sy,
        });
        self.trail.push(Sample {
            at: now,
            x: tx,
            y: ty,
        });
    }
}

/// The point on the trail (from `since`, through `current`) nearest `(x, y)`.
/// Returns its distance, how long ago the prediction was there, and whether
/// that point is on a stretch where the prediction was walking.
fn nearest_on_trail(
    trail: &[Sample],
    current: Sample,
    since: Instant,
    now: Instant,
    x: f64,
    y: f64,
) -> (f64, f64, bool) {
    // Start at the last point before the window, so the stretch that crosses
    // into it counts.
    let start = trail
        .iter()
        .position(|sample| sample.at >= since)
        .unwrap_or(trail.len())
        .saturating_sub(1);
    let points = trail[start..]
        .iter()
        .copied()
        .chain(std::iter::once(current));

    let mut best = (f64::INFINITY, 0.0, false);
    let mut prev: Option<Sample> = None;
    for b in points {
        let a = prev.unwrap_or(b);
        prev = Some(b);
        let (sx, sy) = (b.x - a.x, b.y - a.y);
        let length2 = sx * sx + sy * sy;
        let t = if length2 > 0.0 {
            (((x - a.x) * sx + (y - a.y) * sy) / length2).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let (px, py) = (a.x + sx * t, a.y + sy * t);
        let off = distance(x, y, px, py);
        // On a tie the newest wins: a player parked at the destination
        // matches the standing points, not the end of the walk there.
        if off <= best.0 {
            let at = a.at + (b.at.saturating_duration_since(a.at)).mul_f64(t);
            let ago = now.saturating_duration_since(at).as_secs_f64();
            best = (off, ago, length2 > 1e-6);
        }
    }
    best
}

/// `step` units from `(x, y)` toward `(dx, dy)`, stopping there.
fn toward(x: f64, y: f64, dx: f64, dy: f64, step: f64) -> (f64, f64) {
    let remaining = distance(x, y, dx, dy);
    if remaining <= step || remaining == 0.0 {
        (dx, dy)
    } else {
        (
            x + (dx - x) / remaining * step,
            y + (dy - y) / remaining * step,
        )
    }
}

fn distance(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    (bx - ax).hypot(by - ay)
}

#[cfg(test)]
mod tests;
