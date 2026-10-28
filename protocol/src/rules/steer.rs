//! Walking on a heading (controls plan D3, T0.2, #316): what a `steer`
//! means, so the server and the client's prediction read it alike.
//!
//! - **The heading** is a fraction of a turn as a `u16`, in the server's
//!   frame: 0 is east and a quarter turn ([`QUARTER`]) is south, since y
//!   points south. [`direction`] turns it into a unit vector through
//!   [`StableMath`], so both ends get the same direction to the bit.
//! - **The throttle** is in tenths of the walking speed, [`FULL`] at most.
//! - **`at`** is a waypoint reached before heading off: a turn's corner, or
//!   where a stop ends. One more than [`AT_REACH`] from where the server
//!   has you, or off the world, is ignored. At most [`MAX_WAYPOINTS`] wait;
//!   past that the oldest goes.
//! - **The lease.** A steer lapses after [`LEASE_TICKS`] ticks unless
//!   another renews it, so a client that vanishes doesn't walk on forever.
//! - **The first tick stands** (#480). A steer taken from standing walks
//!   from the tick after, so the server's copy of the walk runs a whole
//!   tick behind the prediction. A stop, made part way through a step,
//!   then lands before the server walks that step: it walks on to `at` and
//!   stops there, and never steps past it and back.
//!
//! [`Steering`] is a walker's steer under way, [`take`] takes a steer into
//! it and [`tick`] walks a tick of it. The server walks its players with
//! them, and the Flint client's loopback (`capitol_core::motion::Loopback`,
//! T0.4) walks its fake server's, so the two can't drift apart.

use std::collections::VecDeque;

use crate::fmath::StableMath;
use crate::rules::ground::{Ground, Step};
use crate::rules::{in_world, ARRIVAL_EPSILON};

/// A quarter turn: south.
pub const QUARTER: u16 = 1 << 14;

/// The throttle at full speed: tenths, so 10.
pub const FULL: u8 = 10;

/// How many ticks a steer lasts unless renewed: 3 s.
pub const LEASE_TICKS: u16 = 30;

/// The most waypoints a steering player has queued.
pub const MAX_WAYPOINTS: usize = 8;

/// How far from the server's position an `at` may be.
pub const AT_REACH: f64 = 100.0;

/// A steering walker held against something hears `movement_blocked` at
/// most once in this many ticks (2 s); they keep the steer meanwhile.
pub const BLOCKED_TICKS: u64 = 20;

/// A walker on a heading, from `steer`s. The waypoints are walked first,
/// then the heading; a throttle of 0 is a stop, which clears once they're
/// walked.
#[derive(Debug, Clone, PartialEq)]
pub struct Steering {
    /// The heading's unit vector ([`direction`])
    pub dir: (f64, f64),
    /// Tenths of the walking speed, [`FULL`] at most
    pub throttle: u8,
    /// Corners and the stop point still to reach, oldest first:
    /// [`MAX_WAYPOINTS`] at most
    pub waypoints: VecDeque<Waypoint>,
    /// Ticks until it lapses unless a steer renews it
    pub lease_ticks: u16,
    /// Taken from standing and not yet ticked: its first tick stands (#480)
    pub held: bool,
}

/// A point a steering walker walks to before going on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waypoint {
    pub x: f64,
    pub y: f64,
    /// The throttle it's walked at: the one the client was walking when it
    /// marked the point, so the server's copy of the walk keeps its pace
    pub throttle: u8,
}

/// Takes a `steer` for a walker standing at `from`: walk on `heading` at
/// `throttle` tenths of the speed, after walking to `at`. Returns whether
/// it took, so the caller can clear a `move`'s destination.
///
/// - **A stop** is throttle 0: the walker walks to `at`, if it has one, and
///   stands. With no steer under way it does nothing, so letting go of W
///   doesn't stop a walk to a node that E just started.
/// - **`at`** is walked at the throttle the client had when it marked the
///   point, so the server's copy of the walk keeps its pace. One over
///   [`AT_REACH`] from `from`, or off the world, is ignored. Past
///   [`MAX_WAYPOINTS`], the oldest goes.
/// - **Any steer renews the lease** to [`LEASE_TICKS`].
pub fn take(steering: &mut Option<Steering>, from: (f64, f64), heading: u16, throttle: u8, at: Option<(f64, f64)>) -> bool {
    let throttle = throttle.min(FULL);
    if steering.is_none() && throttle == 0 {
        return false;
    }
    let at = at.filter(|&(ax, ay)| in_world(ax, ay) && (ax - from.0).stable_hypot(ay - from.1) <= AT_REACH);
    let steering = steering.get_or_insert_with(|| Steering {
        dir: direction(heading),
        throttle: 0,
        waypoints: VecDeque::new(),
        lease_ticks: LEASE_TICKS,
        held: true,
    });
    if let Some((x, y)) = at {
        let pace = [steering.throttle, throttle].into_iter().find(|&t| t > 0).unwrap_or(FULL);
        if steering.waypoints.len() >= MAX_WAYPOINTS {
            steering.waypoints.pop_front();
        }
        steering.waypoints.push_back(Waypoint { x, y, throttle: pace });
    }
    if throttle > 0 {
        steering.dir = direction(heading);
    }
    steering.throttle = throttle;
    steering.lease_ticks = LEASE_TICKS;
    true
}

/// A tick of a steer for a walker at `from`, worth `walk` units at full
/// throttle: to the next waypoint, or else along the heading by
/// [`Ground::walk_step`], at the throttle's share. A tick that reaches a
/// waypoint ends there: the client's prediction walks the same fixed steps,
/// so its corners fall on a step's end.
///
/// - A waypoint that can't be reached is dropped where the walk stopped.
/// - Blocked on the heading, the walker stands and keeps the steer.
/// - A steer's first tick stands, so the walk runs a tick behind the
///   prediction and a stop lands before the step it cut short (#480).
/// - The lease runs down a tick at a time, and the steer clears when it
///   runs out, or when a stop has walked its waypoints.
///
/// With no steer, the walker stands.
pub fn tick(steering: &mut Option<Steering>, from: (f64, f64), ground: &Ground, walk: f64) -> Step {
    let Some(steer) = steering.as_mut() else {
        return Step { x: from.0, y: from.1, blocked: None };
    };
    let step = match steer.waypoints.front().copied() {
        _ if steer.held => {
            steer.held = false;
            Step { x: from.0, y: from.1, blocked: None }
        }
        Some(wp) => {
            let to = (wp.x, wp.y);
            let near = (to.0 - from.0).stable_hypot(to.1 - from.1) < ARRIVAL_EPSILON;
            let step = if near {
                Step { x: to.0, y: to.1, blocked: None }
            } else {
                ground.step(from, to, walk * pace(wp.throttle))
            };
            if step.blocked.is_some() || (step.x, step.y) == to {
                steer.waypoints.pop_front();
            }
            step
        }
        None if steer.throttle > 0 => ground.walk_step(from, steer.dir, walk * pace(steer.throttle)),
        None => Step { x: from.0, y: from.1, blocked: None },
    };
    steer.lease_ticks = steer.lease_ticks.saturating_sub(1);
    if steer.lease_ticks == 0 || (steer.throttle == 0 && steer.waypoints.is_empty()) {
        *steering = None;
    }
    step
}

/// The unit vector of `heading`, in server units (y south).
pub fn direction(heading: u16) -> (f64, f64) {
    let angle = f64::from(heading) / 65536.0 * std::f64::consts::TAU;
    (angle.stable_cos(), angle.stable_sin())
}

/// The heading nearest the way `(dx, dy)` points, for sending; east for no
/// way at all.
pub fn heading_of(dx: f64, dy: f64) -> u16 {
    let turn = dy.stable_atan2(dx) / std::f64::consts::TAU;
    // Round to the nearest step; -0.5 and 0.5 of a turn are both west.
    (turn * 65536.0).round().rem_euclid(65536.0) as u16
}

/// What `throttle` walks of the full speed, clamped to [`FULL`].
pub fn pace(throttle: u8) -> f64 {
    f64::from(throttle.min(FULL)) / f64::from(FULL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12
    }

    #[test]
    fn the_quarters_point_east_south_west_and_north() {
        assert!(near(direction(0), (1.0, 0.0)));
        assert!(near(direction(QUARTER), (0.0, 1.0)), "a quarter turn is south: y points south");
        assert!(near(direction(2 * QUARTER), (-1.0, 0.0)));
        assert!(near(direction(3 * QUARTER), (0.0, -1.0)));
    }

    #[test]
    fn every_heading_is_a_unit_vector_and_comes_back_as_itself() {
        for heading in (0..=u16::MAX).step_by(7) {
            let (x, y) = direction(heading);
            assert!((x.stable_hypot(y) - 1.0).abs() < 1e-12);
            assert_eq!(heading_of(x, y), heading, "heading {heading}");
        }
        assert_eq!(heading_of(1.0, -1e-9), 0, "just north of east rounds back to east, not to 65536");
        assert_eq!(heading_of(-1.0, 0.0), 2 * QUARTER);
        assert_eq!(heading_of(0.0, 0.0), 0);
    }

    #[test]
    fn the_throttle_is_tenths_clamped_to_full() {
        assert_eq!(pace(0), 0.0);
        assert_eq!(pace(5), 0.5);
        assert_eq!(pace(10), 1.0);
        assert_eq!(pace(200), 1.0);
    }
}
