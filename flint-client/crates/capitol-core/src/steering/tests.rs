//! `Steering` on its own, on a fake clock in milliseconds. Its walks against
//! a server are in `motion::tests::steering`.

use capitol_protocol::rules::steer::QUARTER;

use super::*;

const EAST: (f64, f64) = (1.0, 0.0);
const SOUTH: (f64, f64) = (0.0, 1.0);
const NEUTRAL_WAY: (f64, f64) = (0.0, 0.0);

struct Clock(Instant);

impl Clock {
    fn at(&self, ms: u64) -> Instant {
        self.0 + Duration::from_millis(ms)
    }
}

fn walk(heading: u16) -> Option<Steer> {
    Some(Steer { heading, throttle: FULL })
}

fn stop(heading: u16) -> Option<Steer> {
    Some(Steer { heading, throttle: 0 })
}

#[test]
fn a_held_key_is_sent_once_then_renewed_each_second_and_a_release_stops() {
    let clock = Clock(Instant::now());
    let mut steering = Steering::new();
    assert_eq!(steering.update(clock.at(0), NEUTRAL_WAY), None, "standing, nothing to say");
    assert_eq!(steering.update(clock.at(10), EAST), walk(0));
    let sent: Vec<u64> = (20..=2500)
        .step_by(10)
        .filter(|&ms| steering.update(clock.at(ms), EAST).is_some())
        .collect();
    assert_eq!(sent, vec![1010, 2010], "renewed every second, inside the 3 s lease");
    assert_eq!(steering.update(clock.at(2510), NEUTRAL_WAY), stop(0));
    assert_eq!(steering.update(clock.at(2520), NEUTRAL_WAY), None);
    assert_eq!(steering.walking(), None);
}

#[test]
fn the_heading_and_throttle_are_quantised() {
    let clock = Clock(Instant::now());
    let mut steering = Steering::new();
    // South-east, as two keys: a unit diagonal.
    let d = std::f64::consts::FRAC_1_SQRT_2;
    assert_eq!(steering.update(clock.at(0), (d, d)), walk(QUARTER / 2));
    // A stick half over walks at half speed; a nudge is neutral.
    assert_eq!(throttle_of(1.0), 10);
    assert_eq!(throttle_of(0.8), 10);
    assert_eq!(throttle_of(0.5), 5);
    assert_eq!(throttle_of(0.2), 0);
    let mut stick = Steering::new();
    assert_eq!(
        stick.update(clock.at(0), (0.0, 0.5)),
        Some(Steer { heading: QUARTER, throttle: 5 })
    );
    // From half to full is a change, sent.
    assert_eq!(stick.update(clock.at(200), (0.0, 0.9)), walk(QUARTER));
}

#[test]
fn a_turn_under_half_a_degree_isnt_sent() {
    let clock = Clock(Instant::now());
    let mut steering = Steering::new();
    steering.update(clock.at(0), EAST);
    let a_little = (0.25_f64.to_radians().cos(), 0.25_f64.to_radians().sin());
    assert_eq!(steering.update(clock.at(500), a_little), None);
    let more = (1.0_f64.to_radians().cos(), 1.0_f64.to_radians().sin());
    assert!(steering.update(clock.at(510), more).is_some());
}

#[test]
fn a_steady_turn_is_sent_at_eight_a_second_and_a_stop_is_never_held_up() {
    let clock = Clock(Instant::now());
    let mut steering = Steering::new();
    steering.update(clock.at(0), EAST);
    // The mouse sweeps round a degree a frame, for two seconds.
    let mut sent = 0;
    for frame in 1..=200 {
        let angle = (frame as f64).to_radians();
        if steering.update(clock.at(frame * 10), (angle.cos(), angle.sin())).is_some() {
            sent += 1;
        }
    }
    assert!((15..=17).contains(&sent), "{sent} turns in 2 s");
    // Let go in the middle of it: the stop goes at once.
    let last = steering.walking().unwrap().heading;
    assert_eq!(steering.update(clock.at(2005), NEUTRAL_WAY), stop(last));
}

#[test]
fn the_latch_stops_the_walk_and_holds_until_neutral() {
    let clock = Clock(Instant::now());
    let mut steering = Steering::new();
    steering.update(clock.at(0), EAST);
    // E starts a gather while W is held.
    steering.latch();
    assert_eq!(steering.update(clock.at(100), EAST), stop(0));
    for ms in (110..2000).step_by(10) {
        assert_eq!(steering.update(clock.at(ms), EAST), None, "{ms} ms: still held");
        assert_eq!(steering.update(clock.at(ms + 5), SOUTH), None, "{ms} ms: any way");
    }
    assert!(steering.latched());
    // Let go, and press again: walking.
    assert_eq!(steering.update(clock.at(2000), NEUTRAL_WAY), None);
    assert!(!steering.latched());
    assert_eq!(steering.update(clock.at(2010), EAST), walk(0));
}

#[test]
fn a_reconnect_sends_a_held_walk_again() {
    let clock = Clock(Instant::now());
    let mut steering = Steering::new();
    steering.update(clock.at(0), EAST);
    assert_eq!(steering.update(clock.at(300), EAST), None);
    steering.reconnected();
    assert_eq!(steering.update(clock.at(310), EAST), walk(0));
}

#[test]
fn turn_is_the_short_way_round() {
    assert_eq!(turn(10, 65530), 16);
    assert_eq!(turn(65530, 10), 16);
    assert_eq!(turn(0, 2 * QUARTER), 2 * QUARTER);
}
