//! Steering against the loopback (controls T0.4, #318): the input through
//! `Steering`, predicted by `Motion`, walked by a `Loopback` server at a
//! round trip of 0, 80 and 200 ms and at several tick phases. A frame is
//! 16 ms, so the client's frames never line up with the server's ticks.
//! The first steer goes at 516 ms, so it lands at 516, 556 or 616: ticks
//! at +20 and +60 come just after it, and the server walks each step almost
//! a step before the prediction draws it.

use capitol_protocol::land::LandParams;
use capitol_protocol::rules::{CENTER_X, CENTER_Y};
use capitol_protocol::terrain::{Bridge, Mouth, WaterCourse, WaterPoint, WaterRing};
use capitol_protocol::ws::ClientMessage;

use super::*;
use crate::steering::Steering;

const FRAME: u64 = 16;
const ROUND_TRIPS: [u64; 3] = [0, 80, 200];
const PHASES: [u64; 5] = [0, 20, 37, 60, 81];

const EAST: (f64, f64) = (1.0, 0.0);
const SOUTH: (f64, f64) = (0.0, 1.0);
const WEST: (f64, f64) = (-1.0, 0.0);
const LET_GO: (f64, f64) = (0.0, 0.0);

#[derive(Debug, Clone, Copy)]
struct Frame {
    drawn: (f64, f64),
    moving: bool,
    server: (f64, f64),
}

/// A client (store, `Motion`, `Steering`) and a loopback server, wired the
/// way the game wires them to the network.
struct Play {
    state: GameState,
    motion: Motion,
    steering: Steering,
    server: Loopback,
    start: Instant,
    ms: u64,
    frames: Vec<Frame>,
    /// The client sends nothing: frozen, or gone.
    gone: bool,
    label: String,
}

impl Play {
    fn new(from: (f64, f64), round_trip: u64, phase: u64, ground: Arc<Ground>) -> Self {
        let start = Instant::now();
        let server = Loopback::new(start, ME, from, ground.clone())
            .round_trip(Duration::from_millis(round_trip))
            .tick_phase(Duration::from_millis(phase));
        let mut motion = Motion::new();
        motion.set_ground(Some(ground));
        let mut play = Self {
            state: GameState::new(),
            motion,
            steering: Steering::new(),
            server,
            start,
            ms: 0,
            frames: Vec::new(),
            gone: false,
            label: format!("{round_trip} ms round trip, ticks at +{phase} ms"),
        };
        // Placed, and standing a moment, before anything is pressed.
        play.play(500, |_| LET_GO);
        assert!(play.motion.placed().is_some(), "{}: placed", play.label);
        play
    }

    fn at(&self, ms: u64) -> Instant {
        self.start + Duration::from_millis(ms)
    }

    /// Frames up to `until` ms, with the input `way(ms)` each frame.
    fn play(&mut self, until: u64, way: impl Fn(u64) -> (f64, f64)) {
        while self.ms + FRAME <= until {
            self.ms += FRAME;
            self.frame(way(self.ms));
        }
    }

    fn frame(&mut self, way: (f64, f64)) {
        let now = self.at(self.ms);
        for event in self.server.run(now) {
            let changes = self.state.apply(&event);
            self.motion.observe(now, &changes, &self.state);
        }
        if !self.gone {
            if let Some(message) = self.steering.drive(now, way, &mut self.motion) {
                self.server.send(now, message);
            }
        }
        if let Some(me) = self.motion.sample(now).first().copied() {
            self.frames.push(Frame {
                drawn: (me.x, me.y),
                moving: me.moving,
                server: self.server.position(),
            });
        }
    }

    fn last(&self) -> Frame {
        *self.frames.last().expect("a frame")
    }

    /// The prediction and the server agree, standing, and nothing was ever
    /// corrected.
    fn settled(&self) {
        let last = self.last();
        let off = distance(last.drawn.0, last.drawn.1, last.server.0, last.server.1);
        assert!(off <= 0.5, "{}: drawn {:?}, server {:?}", self.label, last.drawn, last.server);
        assert!(!last.moving, "{}: still moving", self.label);
        assert_eq!(self.motion.corrections(), 0, "{}: {:?}", self.label, self.motion.stats(self.at(self.ms)));
    }
}

/// Every round trip at every tick phase.
fn each(mut test: impl FnMut(u64, u64)) {
    for round_trip in ROUND_TRIPS {
        for phase in PHASES {
            test(round_trip, phase);
        }
    }
}

fn flat() -> Arc<Ground> {
    let flat = LandParams { foothill_radius: 1.0e9, ..LandParams::default() };
    Arc::new(Ground::new(&flat, &[], &[], &[], &[], &[]))
}

/// A river 20 wide running north to south at x = 4050, on flat land: its
/// west bank at 4040.
fn river() -> Arc<Ground> {
    let flat = LandParams { foothill_radius: 1.0e9, ..LandParams::default() };
    let point = |y| WaterPoint { x: 4050.0, y, width: 20.0, depth: 2.0, surface: 0.0 };
    let river = WaterCourse {
        id: "river_0".into(),
        source: None,
        mouth: Mouth::Edge,
        points: vec![point(3000.0), point(5000.0)],
    };
    Arc::new(Ground::new(&flat, &[river], &[], &[], &[], &[]))
}

/// A canal 600 round the centre, 30 wide, bridged only due south.
fn canal() -> Arc<Ground> {
    let flat = LandParams { foothill_radius: 1.0e9, ..LandParams::default() };
    let ring = WaterRing {
        id: "canal_inner".into(),
        radius: 600.0,
        width: 30.0,
        bridges: vec![Bridge { angle_degrees: 90.0, width: 40.0 }],
    };
    Arc::new(Ground::new(&flat, &[], &[], &[], &[], &[]).with_rings((CENTER_X, CENTER_Y), &[ring]))
}

#[test]
fn a_straight_walk_agrees_with_the_server() {
    each(|round_trip, phase| {
        let mut play = Play::new((4000.0, 4000.0), round_trip, phase, flat());
        play.play(2500, |_| EAST);
        play.play(4000, |_| LET_GO);
        play.settled();
        let walked = play.last().server.0 - 4000.0;
        assert!((95.0..=105.0).contains(&walked), "{}: walked {walked}", play.label);
        assert_eq!(play.last().server.1, 4000.0);
    });
}

#[test]
fn a_stop_ends_where_it_was_made_with_no_slide_on() {
    each(|round_trip, phase| {
        // Let go at every point of a step: early in one, a server that
        // ticks early has walked to its end before it hears the stop.
        for release in (1700..1800).step_by(FRAME as usize) {
            let mut play = Play::new((4000.0, 4000.0), round_trip, phase, flat());
            play.label = format!("{}, let go at {release} ms", play.label);
            play.play(release, |_| EAST);
            // The frame that sees it let go is where the walk ends.
            play.play(release + FRAME, |_| LET_GO);
            let released = play.last().drawn;
            play.play(3500, |_| LET_GO);
            play.settled();
            // The drawn walk only ever goes forward, and ends there.
            let back = play.frames.windows(2).map(|w| w[0].drawn.0 - w[1].drawn.0).fold(0.0, f64::max);
            assert!(back <= 1e-9, "{}: stepped back {back}", play.label);
            let on = play.last().drawn.0 - released.0;
            assert!(on.abs() <= 1e-9, "{}: slid on {on} after letting go", play.label);
            // And the server is there too, and never walked past it and
            // back, so nobody else saw a step back either (#480).
            assert!((play.last().server.0 - released.0).abs() <= 0.5, "{}", play.label);
            let back = play.frames.windows(2).map(|w| w[0].server.0 - w[1].server.0).fold(0.0, f64::max);
            assert!(back <= 1e-9, "{}: the server stepped back {back}", play.label);
        }
    });
}

#[test]
fn a_sharp_turn_is_taken_on_the_same_corner() {
    each(|round_trip, phase| {
        let mut play = Play::new((4000.0, 4000.0), round_trip, phase, flat());
        play.play(1500, |_| EAST);
        play.play(2500, |_| SOUTH);
        play.play(4000, |_| LET_GO);
        play.settled();
        // Both turned at the same x, to the bit.
        let corner = play.last().server.0;
        let drawn_corner = play.last().drawn.0;
        assert_eq!(corner, drawn_corner, "{}", play.label);
        assert!(play.last().server.1 > 4040.0, "{}: walked south", play.label);
    });
}

#[test]
fn a_sweeping_turn_never_disagrees() {
    each(|round_trip, phase| {
        let mut play = Play::new((4000.0, 4000.0), round_trip, phase, flat());
        // The mouse turns the view at 90° a second for four seconds.
        play.play(4500, |ms| {
            let angle = (ms as f64 - 500.0) / 1000.0 * std::f64::consts::FRAC_PI_2;
            (angle.cos(), angle.sin())
        });
        play.play(6000, |_| LET_GO);
        play.settled();
    });
}

#[test]
fn half_a_stick_walks_at_half_speed() {
    each(|round_trip, phase| {
        let mut play = Play::new((4000.0, 4000.0), round_trip, phase, flat());
        play.play(1000, |_| (0.0, 0.5));
        play.play(1500, |_| SOUTH);
        play.play(2500, |_| (0.0, 0.5));
        play.play(4000, |_| LET_GO);
        play.settled();
        // 0.5 s at 25, 0.5 s at 50 and 1 s at 25 a second.
        let walked = play.last().server.1 - 4000.0;
        assert!((57.5..=67.5).contains(&walked), "{}: walked {walked}", play.label);
    });
}

#[test]
fn a_walk_into_a_bank_at_a_slant_slides_along_it() {
    each(|round_trip, phase| {
        let mut play = Play::new((4000.0, 4000.0), round_trip, phase, river());
        // 30° off due south, toward the river: the bank comes after 80.
        let slant = (0.5, 3.0_f64.sqrt() / 2.0);
        play.play(5500, |_| slant);
        play.play(7000, |_| LET_GO);
        play.settled();
        let (x, y) = play.last().server;
        assert!((4030.0..=4040.0).contains(&x), "{}: at the bank: {x}", play.label);
        // 5 s of walking, 80 of it to the bank: most of the rest is along it.
        assert!(y > 4000.0 + 180.0, "{}: slid to {y}", play.label);
    });
}

#[test]
fn a_canal_stops_a_steer_where_it_stops_the_server() {
    each(|round_trip, phase| {
        let from = (CENTER_X + 660.0, CENTER_Y);
        let mut play = Play::new(from, round_trip, phase, canal());
        // Square into the canal from the east, held on well past the bank.
        play.play(3500, |_| WEST);
        let last = play.last();
        assert!(!last.moving, "{}: held against the bank, standing", play.label);
        assert!((last.server.0 - (CENTER_X + 617.0)).abs() < 1e-6, "{}: server at {:?}", play.label, last.server);
        assert!(distance(last.drawn.0, last.drawn.1, last.server.0, last.server.1) < 1e-6, "{}", play.label);
        play.play(4500, |_| LET_GO);
        play.settled();
    });
}

#[test]
fn a_reconnect_mid_steer_sends_the_walk_again() {
    each(|round_trip, phase| {
        let mut play = Play::new((4000.0, 4000.0), round_trip, phase, flat());
        play.play(1500, |_| EAST);
        let now = play.at(play.ms);
        play.server.disconnect(now);
        play.play(1900, |_| EAST);
        let dropped = play.server.position();
        let now = play.at(play.ms);
        play.server.reconnect(now);
        play.play(3500, |_| EAST);
        play.play(5000, |_| LET_GO);
        play.settled();
        let walked = play.last().server.0 - dropped.0;
        assert!(walked > 60.0, "{}: walked {walked} after the reconnect", play.label);
    });
}

#[test]
fn a_steer_lapses_when_the_client_is_gone() {
    each(|round_trip, phase| {
        let mut play = Play::new((4000.0, 4000.0), round_trip, phase, flat());
        play.play(1600, |_| EAST);
        assert!(play.server.steering());
        // The client freezes holding W: nothing more is sent, no renewal.
        play.gone = true;
        play.play(6000, |_| EAST);
        assert!(!play.server.steering(), "{}: lapsed", play.label);
        let stood = play.server.position();
        play.play(8000, |_| EAST);
        assert_eq!(play.server.position(), stood, "{}: and stands", play.label);
        // The last renewal went at about 1.5 s, and the lease ran 3 s from
        // there: about 4 s of walking from 0.5 s.
        let walked = stood.0 - 4000.0;
        assert!((190.0..=215.0).contains(&walked), "{}: walked {walked}", play.label);
    });
}

#[test]
fn a_click_ends_a_steer() {
    each(|round_trip, phase| {
        let mut play = Play::new((4000.0, 4000.0), round_trip, phase, flat());
        play.play(1500, |_| EAST);
        let now = play.at(play.ms);
        play.motion.move_to(now, 4100.0, 4100.0);
        play.server.send(now, ClientMessage::Move { x: 4100.0, y: 4100.0 });
        assert!(!play.motion.steering());
        play.play(6000, |_| LET_GO);
        play.settled();
        assert_eq!(play.last().server, (4100.0, 4100.0), "{}", play.label);
    });
}
