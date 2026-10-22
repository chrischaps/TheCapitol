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

use crate::fmath::StableMath;

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
