//! The frame's instant, paced like the engine's dt (#406).
//!
//! With vsync on, frames can be let through in bursts: 6 ms, 6 ms, 6 ms, then
//! 45 ms apart at a 60 Hz mean. The engine paces its dt to the refresh, so
//! everything it moves steps evenly. But [`crate::motion::Motion`] samples
//! the avatars at an `Instant`, and sampled at `Instant::now()` they'd still
//! step 6, 6, 6, then 45 ms a shown frame, and so would a camera that follows
//! one. [`FrameInstant`] is that `Instant`, moved on by the frame's dt.
//!
//! It stays on the wall clock: past [`MAX_DRIFT`] either way (a stall the dt
//! was clamped through, a sleep), it snaps back to it.

use std::time::{Duration, Instant};

/// How far the frame's instant may drift from the wall clock before it
/// snaps back, in seconds.
pub const MAX_DRIFT: f64 = 0.25;

#[derive(Debug, Clone, Copy, Default)]
pub struct FrameInstant {
    at: Option<Instant>,
}

impl FrameInstant {
    pub fn new() -> Self {
        Self::default()
    }

    /// Once a frame: move on by `dt` seconds, the frame's step, and return
    /// the frame's instant. The first frame's is `wall`.
    pub fn advance(&mut self, wall: Instant, dt: f64) -> Instant {
        let next = match self.at {
            Some(at) => at + Duration::from_secs_f64(dt.max(0.0)),
            None => wall,
        };
        let drift = if next >= wall {
            next.duration_since(wall)
        } else {
            wall.duration_since(next)
        };
        let next = if drift.as_secs_f64() > MAX_DRIFT { wall } else { next };
        self.at = Some(next);
        next
    }

    /// This frame's instant, or `wall` before the first.
    pub fn now_or(&self, wall: Instant) -> Instant {
        self.at.unwrap_or(wall)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ60: f64 = 1.0 / 60.0;

    #[test]
    fn the_first_frame_is_the_wall_clock() {
        let wall = Instant::now();
        assert_eq!(FrameInstant::new().advance(wall, 0.5), wall);
    }

    #[test]
    fn bursts_on_the_wall_step_evenly() {
        let t0 = Instant::now();
        let mut frame = FrameInstant::new();
        let mut wall = t0;
        let mut last = frame.advance(wall, 0.0);
        for i in 1..240 {
            let gap = if i % 4 == 0 { 4.0 * HZ60 - 0.018 } else { 0.006 };
            wall += Duration::from_secs_f64(gap);
            let at = frame.advance(wall, HZ60);
            assert!((at.duration_since(last).as_secs_f64() - HZ60).abs() < 1e-6);
            last = at;
        }
    }

    #[test]
    fn a_long_drift_snaps_to_the_wall() {
        let t0 = Instant::now();
        let mut frame = FrameInstant::new();
        frame.advance(t0, 0.0);
        // A one-second stall, stepped by a clamped 250 ms dt.
        let wall = t0 + Duration::from_secs(1);
        assert_eq!(frame.advance(wall, 0.25), wall);
        assert_eq!(frame.now_or(t0), wall);
    }

    #[test]
    fn a_small_lead_is_kept() {
        let t0 = Instant::now();
        let mut frame = FrameInstant::new();
        frame.advance(t0, 0.0);
        let at = frame.advance(t0 + Duration::from_millis(6), HZ60);
        assert_eq!(at, t0 + Duration::from_secs_f64(HZ60));
    }
}
