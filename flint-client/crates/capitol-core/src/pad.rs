//! The pad over the shoulder (controls plan D6, T2.1): the sticks as a walk
//! and a look. Pure, so the feel is tested without a pad.
//!
//! - **The walk.** [`walk`] takes the summed `walk_x`/`walk_y` actions (the
//!   keys at ±1 and the left stick, bound with no deadzone of their own) and
//!   gives `(forward, right)` for [`crate::steering::camera_way`]. A radial
//!   [`DEADZONE`], rescaled to 0..1, so stick drift never steers and so never
//!   cancels a gather; then cut to length 1, so W+D is no faster than W. The
//!   throttle steps are [`crate::steering::throttle_of`]'s, after the rescale.
//! - **The look.** [`look`] turns the right stick into a turn rate: the same
//!   radial deadzone, then a gentle curve, so a small push turns slowly and a
//!   full one at [`TURN_RATE`]. It's a rate, never added to the mouse's
//!   counts.
//! - **Recentring.** [`Recentre`] eases the view round behind the walk once
//!   the look stick has rested [`REST`], and only while the walk is within
//!   [`AHEAD`] of forward, so a strafe or a back-step never turns the view.
//!   Standing still it never moves, so a look round at a stall stays put.

use std::f64::consts::FRAC_PI_4;

/// The radial deadzone of both sticks, as a share of full deflection.
pub const DEADZONE: f64 = 0.2;

/// The fastest the look stick turns the view, in radians a second (about
/// 140°), at a look sensitivity of 1.
pub const TURN_RATE: f64 = 2.4;

/// The fastest it tilts the view, in degrees a second.
pub const TILT_RATE: f64 = 60.0;

/// How long the look stick rests before the view recentres, in seconds.
pub const REST: f64 = 1.5;

/// How far off forward the walk may be for the view to recentre.
pub const AHEAD: f64 = FRAC_PI_4;

/// How fast the view eases round behind you: the share of the way left
/// covered each second is `1 - e^-RECENTRE_K`.
pub const RECENTRE_K: f64 = 0.6;

/// A stick at `(x, y)` past the radial deadzone, rescaled so the deadzone's
/// edge is 0 and full deflection 1, and cut to length 1. Its direction is
/// kept. Inside the deadzone, `(0, 0)`.
pub fn stick(x: f64, y: f64) -> (f64, f64) {
    let len = x.hypot(y);
    if !len.is_finite() || len <= DEADZONE {
        return (0.0, 0.0);
    }
    let to = ((len - DEADZONE) / (1.0 - DEADZONE)).min(1.0);
    (x / len * to, y / len * to)
}

/// The walk for the summed `walk_x` (right) and `walk_y` (forward, up the
/// stick) actions: `(forward, right)`, as [`crate::steering::camera_way`]
/// takes them. Keys come through at full length, held one or two at once.
pub fn walk(x: f64, y: f64) -> (f64, f64) {
    let (right, forward) = stick(x, y);
    (forward, right)
}

/// This frame's turn from the look stick at `(x, y)` (right and up are
/// positive) over `dt` seconds: `(yaw, tilt)`, the yaw in radians (right
/// turns it down, as the camera rig counts it) and the tilt in degrees
/// (down is positive, as a mouse moving down tilts it). `sensitivity`
/// scales both; `invert_y` makes up look down.
pub fn look(x: f64, y: f64, dt: f64, sensitivity: f64, invert_y: bool) -> (f64, f64) {
    let (sx, sy) = stick(x, y);
    let len = sx.hypot(sy);
    if len == 0.0 {
        return (0.0, 0.0);
    }
    // Gentle: half the push turns at a quarter of the rate
    let (ux, uy, rate) = (sx / len, sy / len, len * len * sensitivity * dt);
    let up = if invert_y { -1.0 } else { 1.0 };
    (-ux * rate * TURN_RATE, -uy * up * rate * TILT_RATE)
}

/// Whether the view should ease round behind the walk this frame, and by
/// how much. See the module docs.
#[derive(Debug, Clone, Default)]
pub struct Recentre {
    /// How long the look stick has rested, in seconds.
    rested: f64,
}

/// What [`Recentre::update`] asks of the rig this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ease {
    /// Turn the yaw by this, in radians (right is negative, as for
    /// [`look`]).
    pub yaw: f64,
    /// Bring the tilt this share of the way back to level.
    pub level: f64,
}

impl Recentre {
    pub fn new() -> Self {
        Self::default()
    }

    /// One frame of `dt` seconds. `pad_only` is whether the pad was the last
    /// device used; `looking` whether the look stick is past its deadzone;
    /// `walk` the `(forward, right)` from [`walk`].
    pub fn update(&mut self, dt: f64, pad_only: bool, looking: bool, walk: (f64, f64)) -> Ease {
        if looking || !pad_only {
            self.rested = 0.0;
            return Ease::default();
        }
        self.rested += dt;
        let (forward, right) = walk;
        if self.rested < REST || forward.hypot(right) == 0.0 {
            return Ease::default();
        }
        let off = right.atan2(forward);
        if off.abs() > AHEAD {
            return Ease::default();
        }
        let share = 1.0 - (-RECENTRE_K * dt).exp();
        Ease {
            yaw: -off * share,
            level: share,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steering::{camera_way, throttle_of};

    fn throttle(x: f64, y: f64) -> u8 {
        let (forward, right) = walk(x, y);
        let (wx, wy) = camera_way(forward, right, 0.0);
        throttle_of(wx.hypot(wy))
    }

    #[test]
    fn drift_never_steers() {
        for (x, y) in [(0.15, 0.0), (0.0, -0.15), (0.12, 0.12), (0.2, 0.0)] {
            assert_eq!(stick(x, y), (0.0, 0.0), "({x}, {y})");
            assert_eq!(throttle(x, y), 0);
        }
    }

    #[test]
    fn a_push_walks_at_half_then_full() {
        // Past the deadzone, rescaled: a quarter is still neutral
        assert_eq!(throttle(0.35, 0.0), 0);
        assert_eq!(throttle(0.5, 0.0), 5);
        assert_eq!(throttle(0.0, -0.6), 5);
        assert_eq!(throttle(0.9, 0.0), 10);
        assert_eq!(throttle(1.0, 1.0), 10);
    }

    #[test]
    fn keys_walk_at_full_and_two_are_no_faster_than_one() {
        let (f, r) = walk(0.0, 1.0);
        assert!((f - 1.0).abs() < 1e-12 && r == 0.0);
        let (f, r) = walk(1.0, 1.0);
        assert!((f.hypot(r) - 1.0).abs() < 1e-12, "W+D is 1, not 1.4");
        // A key and the stick on the same way, or W and Up: still 1
        let (f, r) = walk(0.0, 2.0);
        assert!((f - 1.0).abs() < 1e-12 && r == 0.0);
    }

    #[test]
    fn the_stick_keeps_its_direction() {
        let (f, r) = walk(0.6, 0.8);
        assert!((r / f - 0.75).abs() < 1e-12);
        // Up the stick walks the way the view faces: north at yaw 0
        let (wx, wy) = camera_way(walk(0.0, 1.0).0, walk(0.0, 1.0).1, 0.0);
        assert!(wx.abs() < 1e-12 && wy < 0.0);
    }

    #[test]
    fn the_look_turns_as_a_rate_on_a_gentle_curve() {
        assert_eq!(look(0.15, 0.1, 1.0 / 60.0, 1.0, false), (0.0, 0.0));
        let (full, _) = look(1.0, 0.0, 1.0, 1.0, false);
        assert!((full + TURN_RATE).abs() < 1e-12, "right turns the yaw down");
        let (half, _) = look(0.6, 0.0, 1.0, 1.0, false);
        assert!((half + TURN_RATE * 0.25).abs() < 1e-12, "half the push, a quarter of the rate");
        // A rate: twice the time, twice the turn
        let (a, _) = look(1.0, 0.0, 0.01, 1.0, false);
        let (b, _) = look(1.0, 0.0, 0.02, 1.0, false);
        assert!((b - 2.0 * a).abs() < 1e-12);
        // Sensitivity scales it
        let (s, _) = look(1.0, 0.0, 1.0, 0.5, false);
        assert!((s - full * 0.5).abs() < 1e-12);
    }

    #[test]
    fn up_looks_up_unless_inverted() {
        let (_, tilt) = look(0.0, 1.0, 1.0, 1.0, false);
        assert!((tilt + TILT_RATE).abs() < 1e-12, "up lifts the view");
        let (_, tilt) = look(0.0, 1.0, 1.0, 1.0, true);
        assert!((tilt - TILT_RATE).abs() < 1e-12);
    }

    #[test]
    fn the_view_recentres_after_the_look_rests_while_walking_ahead() {
        let dt = 0.25;
        let ahead = walk(0.3, 0.9);
        let mut r = Recentre::new();
        // Still looking: nothing, and the rest starts again
        assert_eq!(r.update(dt, true, true, ahead), Ease::default());
        for _ in 0..5 {
            assert_eq!(r.update(dt, true, false, ahead), Ease::default(), "rested under 1.5 s");
        }
        let e = r.update(dt, true, false, ahead);
        assert!(e.yaw < 0.0, "the walk is right of the view, so it turns right: {e:?}");
        assert!(e.level > 0.0 && e.level < 0.2, "gently: {e:?}");
        // A look starts the wait again
        r.update(dt, true, true, ahead);
        assert_eq!(r.update(dt, true, false, ahead), Ease::default());
    }

    #[test]
    fn never_while_strafing_backing_or_standing_or_on_the_mouse() {
        let rested = || {
            let mut r = Recentre::new();
            r.update(REST, true, false, (0.0, 0.0));
            r
        };
        // Standing: a look round at a stall stays where it was left
        assert_eq!(rested().update(0.1, true, false, (0.0, 0.0)), Ease::default());
        // Strafing, past 45°
        assert_eq!(rested().update(0.1, true, false, walk(1.0, 0.5)), Ease::default());
        // Backing up
        assert_eq!(rested().update(0.1, true, false, walk(0.0, -1.0)), Ease::default());
        // Keys and the mouse last: never
        assert_eq!(rested().update(0.1, false, false, walk(0.0, 1.0)), Ease::default());
        // Within 45° it does
        assert_ne!(rested().update(0.1, true, false, walk(0.9, 1.0)), Ease::default());
    }
}
