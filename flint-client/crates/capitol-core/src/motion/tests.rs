//! Motion tests. Snapshots go through a real `GameState`, on a fake clock
//! that counts milliseconds from the start of the rig.

use capitol_protocol::ws::{PlayerPosition, ServerMessage as M};

use super::*;
use crate::net::{ConnectionState, NetEvent};

const ME: Uuid = Uuid::from_u128(1);
const OTHER: Uuid = Uuid::from_u128(2);
const THIRD: Uuid = Uuid::from_u128(3);

/// A store and a `Motion` wired the way the game wires them.
struct Rig {
    state: GameState,
    motion: Motion,
    start: Instant,
}

impl Rig {
    /// Connected as `ME`, before any snapshot.
    fn new() -> Self {
        let mut rig = Self {
            state: GameState::new(),
            motion: Motion::new(),
            start: Instant::now(),
        };
        rig.feed(
            0,
            NetEvent::Connection(ConnectionState::Connected { player_id: ME }),
        );
        rig
    }

    fn at(&self, ms: u64) -> Instant {
        self.start + Duration::from_millis(ms)
    }

    fn feed(&mut self, ms: u64, event: NetEvent) {
        let changes = self.state.apply(&event);
        let now = self.at(ms);
        self.motion.observe(now, &changes, &self.state);
    }

    fn snapshot(&mut self, ms: u64, players: &[(Uuid, f64, f64)]) {
        let players = players.iter().map(|&(id, x, y)| pos(id, x, y)).collect();
        self.feed(ms, NetEvent::Ws(M::Positions { players }));
    }

    fn move_to(&mut self, ms: u64, x: f64, y: f64) {
        let now = self.at(ms);
        self.motion.move_to(now, x, y);
    }

    fn sample(&mut self, ms: u64) -> Vec<Pose> {
        let now = self.at(ms);
        self.motion.sample(now).to_vec()
    }

    fn pose(&mut self, ms: u64, id: Uuid) -> Pose {
        self.sample(ms)
            .into_iter()
            .find(|pose| pose.id == id)
            .unwrap_or_else(|| panic!("no pose for {id} at {ms} ms"))
    }
}

fn pos(id: Uuid, x: f64, y: f64) -> PlayerPosition {
    PlayerPosition {
        id,
        name: format!("player {}", id.as_u128()),
        x,
        y,
        citizen: false,
    }
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

/// Where a player walking east from 4000 at 50 u/s is after `ms`.
fn walked(ms: f64) -> f64 {
    4000.0 + 50.0 * ms.max(0.0) / 1000.0
}

/// The x of snapshot `k` for a player walking east, one tick per snapshot.
fn tick_x(k: u64) -> f64 {
    4000.0 + 5.0 * k as f64
}

/// Plays `OTHER` walking east, snapshot `k` arriving at `arrival(k)` (or
/// never, for `None`), sampling every 10 ms. Returns `(ms, x)` per frame.
fn play_other(rig: &mut Rig, until: u64, arrival: impl Fn(u64) -> Option<u64>) -> Vec<(u64, f64)> {
    let mut due: Vec<(u64, u64)> = (0..=until / 100 + 5)
        .filter_map(|k| arrival(k).map(|at| (at, k)))
        .collect();
    due.sort();
    let mut frames = Vec::new();
    let mut next = 0;
    for ms in (0..=until).step_by(10) {
        while next < due.len() && due[next].0 <= ms {
            let k = due[next].1;
            rig.snapshot(ms, &[(ME, 4000.0, 4100.0), (OTHER, tick_x(k), 4000.0)]);
            next += 1;
        }
        frames.push((ms, rig.pose(ms, OTHER).x));
    }
    frames
}

/// The largest step forward and backward between consecutive frames.
fn steps(frames: &[(u64, f64)]) -> (f64, f64) {
    frames
        .windows(2)
        .fold((0.0_f64, 0.0_f64), |(fwd, back), pair| {
            let step = pair[1].1 - pair[0].1;
            (fwd.max(step), back.max(-step))
        })
}

// ---------------------------------------------------------------------------
// Other players
// ---------------------------------------------------------------------------

#[test]
fn steady_motion_is_drawn_150ms_behind_at_walking_speed() {
    let mut rig = Rig::new();
    let frames = play_other(&mut rig, 2000, |k| Some(k * 100));
    for &(ms, x) in &frames {
        if ms >= 150 {
            assert!(
                close(x, walked(ms as f64 - 150.0), 1e-6),
                "{ms} ms: x = {x}"
            );
        }
    }
    let pose = rig.pose(2000, OTHER);
    assert!(pose.moving);
    assert!(close(pose.facing, 0.0, 1e-9), "walking east faces 0");
}

#[test]
fn facing_follows_the_walk() {
    let mut rig = Rig::new();
    // Walking south (y grows): facing π/2.
    for k in 0..5 {
        rig.snapshot(k * 100, &[(OTHER, 4000.0, 4000.0 + 5.0 * k as f64)]);
    }
    let pose = rig.pose(400, OTHER);
    assert!(close(pose.facing, std::f64::consts::FRAC_PI_2, 1e-9));
    // It keeps facing south once it stops.
    for k in 5..12 {
        rig.snapshot(k * 100, &[(OTHER, 4000.0, 4020.0)]);
    }
    let pose = rig.pose(1100, OTHER);
    assert!(!pose.moving);
    assert!(close(pose.facing, std::f64::consts::FRAC_PI_2, 1e-9));
}

#[test]
fn a_burst_after_a_stall_is_spread_back_over_its_ticks() {
    let mut rig = Rig::new();
    // Snapshots 6, 7 and 8 are held up and arrive together at 800 ms.
    let frames = play_other(&mut rig, 2000, |k| {
        Some(if (6..=8).contains(&k) { 800 } else { k * 100 })
    });
    let (forward, back) = steps(&frames);
    // A walk is 0.5 units a frame. Stamped on arrival, the burst would jump
    // 10 units in one frame.
    assert!(forward <= 1.0, "largest step forward {forward}");
    // The drift while extrapolating is taken back gently.
    assert!(back <= 0.5, "largest step back {back}");
    // And it's back on the steady track afterwards.
    let (_, x) = frames.last().copied().unwrap();
    assert!(close(x, walked(2000.0 - 150.0), 1e-6), "x = {x}");
}

#[test]
fn late_snapshots_carry_on_briefly_then_hold() {
    let mut rig = Rig::new();
    let frames = play_other(&mut rig, 1200, |k| (k <= 5).then_some(k * 100));
    let x_at = |ms: u64| frames.iter().find(|f| f.0 == ms).unwrap().1;
    // The last snapshot (x 4025, stamped 500) is drawn at 650 ms.
    assert!(close(x_at(650), 4025.0, 1e-6));
    // Then the last velocity carries it on for 150 ms...
    assert!(close(x_at(700), 4027.5, 1e-6));
    assert!(close(x_at(800), 4032.5, 1e-6));
    // ...and it holds there.
    assert!(close(x_at(1200), 4032.5, 1e-6));
    let pose = rig.pose(1200, OTHER);
    assert!(!pose.moving, "a held player stands still");
}

#[test]
fn a_snapshot_that_never_comes_is_caught_up() {
    let mut rig = Rig::new();
    // This can't happen on the real socket, but if it did...
    let frames = play_other(&mut rig, 4000, |k| (k != 5).then_some(k * 100));
    let (forward, back) = steps(&frames);
    // ...it costs one hitch. The extrapolation is 2.8 units short when the
    // next snapshot comes, then 1.2 over at the faster pace it implies.
    assert!(forward <= 3.0, "largest step forward {forward}");
    assert!(back <= 1.5, "largest step back {back}");
    // The timeline slews back, so it's on the steady track again.
    let (_, x) = frames.last().copied().unwrap();
    assert!(close(x, walked(4000.0 - 150.0), 1e-6), "x = {x}");
}

#[test]
fn a_teleport_snaps_instead_of_sliding() {
    let mut rig = Rig::new();
    for k in 0..=5 {
        rig.snapshot(k * 100, &[(OTHER, 4000.0, 4000.0)]);
    }
    rig.snapshot(600, &[(OTHER, 4500.0, 4000.0)]);
    for ms in (600..=1000).step_by(10) {
        let x = rig.pose(ms, OTHER).x;
        assert_eq!(x, 4500.0, "{ms} ms: x = {x}");
    }
}

#[test]
fn players_come_and_go_with_the_snapshots() {
    let mut rig = Rig::new();
    rig.snapshot(0, &[(ME, 4000.0, 4000.0), (OTHER, 4100.0, 4000.0)]);
    let ids: Vec<Uuid> = rig.sample(10).iter().map(|pose| pose.id).collect();
    assert_eq!(ids, vec![ME, OTHER], "the local player comes first");

    rig.snapshot(
        100,
        &[
            (ME, 4000.0, 4000.0),
            (OTHER, 4100.0, 4000.0),
            (THIRD, 4200.0, 4000.0),
        ],
    );
    // A newcomer is drawn where they are straight away.
    assert_eq!(rig.pose(110, THIRD).x, 4200.0);
    assert_eq!(rig.motion.pose(THIRD).unwrap().x, 4200.0);

    rig.feed(150, NetEvent::Ws(M::PlayerLeft { player_id: OTHER }));
    assert!(rig.sample(160).iter().all(|pose| pose.id != OTHER));
    // Missing from a snapshot is gone too.
    rig.snapshot(200, &[(ME, 4000.0, 4000.0)]);
    assert_eq!(rig.sample(210).len(), 1);
}

// ---------------------------------------------------------------------------
// The local player
// ---------------------------------------------------------------------------

#[test]
fn nothing_is_drawn_or_predicted_before_the_first_snapshot() {
    let mut rig = Rig::new();
    rig.move_to(0, 4100.0, 4000.0);
    assert!(rig.sample(100).is_empty());
    assert_eq!(rig.motion.destination(), None);

    rig.snapshot(200, &[(ME, 4000.0, 4000.0)]);
    let me = rig.pose(300, ME);
    assert_eq!((me.x, me.y), (4000.0, 4000.0));
    assert!(!me.moving);
}

#[test]
fn the_local_player_walks_at_server_speed_and_snaps_on_arrival() {
    let mut rig = Rig::new();
    rig.snapshot(0, &[(ME, 4000.0, 4000.0)]);
    rig.move_to(0, 4010.2, 4000.0);
    assert_eq!(rig.motion.destination(), Some((4010.2, 4000.0)));

    let me = rig.pose(100, ME);
    assert!(close(me.x, 4005.0, 1e-9));
    assert!(me.moving);
    assert!(close(me.facing, 0.0, 1e-9));
    assert!(close(rig.pose(200, ME).x, 4010.0, 1e-9));
    // 0.2 short: inside ARRIVAL_EPSILON, so it snaps on and stops.
    let me = rig.pose(201, ME);
    assert_eq!(me.x, 4010.2);
    assert!(!me.moving);
    assert_eq!(rig.motion.destination(), None);
}

#[test]
fn a_new_click_turns_from_where_the_player_is() {
    let mut rig = Rig::new();
    rig.snapshot(0, &[(ME, 4000.0, 4000.0)]);
    rig.move_to(0, 4100.0, 4000.0);
    rig.sample(50);
    // Turn north (y shrinks) at 200 ms, without a frame in between.
    rig.move_to(200, 4010.0, 3900.0);
    let me = rig.pose(300, ME);
    assert!(close(me.x, 4010.0, 1e-9));
    assert!(close(me.y, 3995.0, 1e-9));
    assert!(close(me.facing, -std::f64::consts::FRAC_PI_2, 1e-9));
}

/// The local player walks east from 4000 toward 4100 from 0 ms, sampled
/// every 10 ms. The server reports `server(ms)` each tick. Returns the
/// drawn x per frame.
fn play_me(rig: &mut Rig, until: u64, server: impl Fn(u64) -> f64) -> Vec<(u64, f64)> {
    rig.snapshot(0, &[(ME, 4000.0, 4000.0)]);
    rig.move_to(0, 4100.0, 4000.0);
    let mut frames = Vec::new();
    for ms in (0..=until).step_by(10) {
        if ms > 0 && ms % 100 == 0 {
            rig.snapshot(ms, &[(ME, server(ms), 4000.0)]);
        }
        let me = rig.pose(ms, ME);
        assert_eq!(me.y, 4000.0);
        frames.push((ms, me.x));
    }
    frames
}

#[test]
fn a_server_a_round_trip_behind_on_the_same_path_is_left_alone() {
    let mut rig = Rig::new();
    // The server starts 120 ms late and moves in 5-unit ticks.
    let frames = play_me(&mut rig, 2500, |ms| {
        walked((ms as f64 - 120.0).max(0.0)).min(4100.0)
    });
    for &(ms, x) in &frames {
        assert!(
            close(x, walked(ms as f64).min(4100.0), 1e-9),
            "{ms} ms: x = {x}"
        );
    }
    assert_eq!(rig.motion.corrections(), 0);
    // It learned the lag: 120 ms, plus up to a tick for the 5-unit steps.
    let lag = rig.motion.lag().unwrap().as_millis();
    assert!((100..=230).contains(&lag), "lag {lag} ms");
}

#[test]
fn a_lag_spike_is_not_a_disagreement() {
    let mut rig = Rig::new();
    // A round trip of 100 ms, then 400 ms for a second, then 100 ms again.
    let lag = |ms: u64| {
        if (600..1600).contains(&ms) {
            400.0
        } else {
            100.0
        }
    };
    let frames = play_me(&mut rig, 2500, |ms| walked(ms as f64 - lag(ms)).min(4100.0));
    for &(ms, x) in &frames {
        assert!(
            close(x, walked(ms as f64).min(4100.0), 1e-9),
            "{ms} ms: x = {x}"
        );
    }
}

#[test]
fn a_server_ahead_on_the_same_path_is_left_alone() {
    let mut rig = Rig::new();
    // The server ticks the moment the move lands, then catches up on late
    // ticks in a burst: 150 ms (7.5 units) ahead of the prediction, as seen
    // live under load (#98).
    let frames = play_me(&mut rig, 2500, |ms| walked(ms as f64 + 150.0).min(4100.0));
    for &(ms, x) in &frames {
        assert!(
            close(x, walked(ms as f64).min(4100.0), 1e-9),
            "{ms} ms: x = {x}"
        );
    }
    assert_eq!(rig.motion.corrections(), 0);
}

#[test]
fn a_server_far_ahead_on_the_same_path_is_caught_up() {
    let mut rig = Rig::new();
    // A second (50 units) ahead: more than the window allows.
    play_me(&mut rig, 300, |ms| walked(ms as f64 + 1000.0).min(4100.0));
    assert!(rig.motion.corrections() > 0);
    let me = rig.pose(300, ME);
    assert!(me.x > walked(300.0) + 10.0, "pulled ahead: {}", me.x);
}

#[test]
fn a_walk_the_server_resumed_is_followed() {
    let mut rig = Rig::new();
    // After a reconnect the server walks on to a stored destination: east
    // from 4000 for a second, then it stops at 4050.
    let server = |ms: u64| walked((ms as f64 - 100.0).clamp(0.0, 1000.0));
    rig.snapshot(0, &[(ME, 4000.0, 4000.0)]);
    let mut frames = Vec::new();
    for ms in (0..=2500).step_by(10) {
        if ms > 0 && ms % 100 == 0 {
            rig.snapshot(ms, &[(ME, server(ms), 4000.0)]);
        }
        let me = rig.pose(ms, ME);
        if (300..1100).contains(&ms) {
            assert!(me.moving, "{ms} ms: walking with the server");
            // A tick behind the server's report at worst, a tick ahead at best.
            assert!(
                close(me.x, server(ms), 6.0),
                "{ms} ms: drawn {}, server {}",
                me.x,
                server(ms)
            );
        }
        frames.push((ms, me.x));
    }
    assert_eq!(
        rig.motion.destination(),
        None,
        "no marker for a walk we didn't click"
    );
    let (forward, back) = steps(&frames);
    assert!(forward <= 1.5 && back <= 1.0, "steps {forward} / {back}");
    let me = rig.pose(2500, ME);
    assert!(close(me.x, 4050.0, 0.01), "stops with the server: {}", me.x);
    assert!(!me.moving);
}

#[test]
fn a_server_that_stopped_pulls_the_prediction_back_smoothly() {
    let mut rig = Rig::new();
    // The move never reached the server: it stays put.
    let frames = play_me(&mut rig, 2000, |_| 4000.0);
    let peak = frames.iter().map(|f| f.1).fold(f64::MIN, f64::max);
    let peak_ms = frames.iter().find(|f| f.1 == peak).unwrap().0;
    // It walks until the start of the walk falls out of the window, then
    // turns back.
    assert!(
        (500..=800).contains(&peak_ms),
        "turned back at {peak_ms} ms"
    );
    // Back without a jump: the largest correction per frame is a tenth of the
    // distance (1 - e^-0.1), well short of the whole way.
    let (_, back) = steps(&frames);
    assert!(
        back < 0.1 * (peak - 4000.0),
        "largest step back {back} from {peak}"
    );
    let (_, x) = frames.last().copied().unwrap();
    assert!(close(x, 4000.0, 0.01), "settles on the server: {x}");
    assert_eq!(rig.motion.destination(), None, "and stops");
    assert_eq!(
        rig.motion.corrections(),
        1,
        "one correction, then agreement"
    );
}

#[test]
fn a_slower_server_is_followed_without_jumps() {
    let mut rig = Rig::new();
    // Only a hand-edited `players.speed` does this. The server walks at
    // 40 u/s: it falls behind along the path, which looks like lag until it
    // leaves the window, so the prediction runs ahead by up to the window's
    // worth of walking (25 units) before it's pulled back.
    let server = |ms: u64| (4000.0 + 40.0 * (ms as f64 - 100.0).max(0.0) / 1000.0).min(4100.0);
    let frames = play_me(&mut rig, 3500, server);
    for &(ms, x) in &frames {
        let truth = (4000.0 + 40.0 * ms as f64 / 1000.0).min(4100.0);
        assert!(close(x, truth, 30.0), "{ms} ms: drawn {x}, server {truth}");
    }
    let (forward, back) = steps(&frames);
    assert!(forward <= 1.0 && back <= 1.0, "steps {forward} / {back}");
    let (_, x) = frames.last().copied().unwrap();
    assert!(close(x, 4100.0, 0.01), "arrives: {x}");
}

#[test]
fn a_far_disagreement_snaps() {
    let mut rig = Rig::new();
    rig.snapshot(0, &[(ME, 4000.0, 4000.0)]);
    rig.sample(50);
    rig.snapshot(100, &[(ME, 4300.0, 4000.0)]);
    let me = rig.pose(110, ME);
    assert_eq!((me.x, me.y), (4300.0, 4000.0));
}

#[test]
fn movement_blocked_snaps_and_stops() {
    let mut rig = Rig::new();
    rig.snapshot(0, &[(ME, 4000.0, 4000.0)]);
    rig.move_to(0, 4100.0, 4000.0);
    assert!(close(rig.pose(300, ME).x, 4015.0, 1e-9));
    rig.feed(
        300,
        NetEvent::Ws(M::MovementBlocked {
            player_id: ME,
            reason: "Cannot cross the moat without a bridge".into(),
            water_id: "moat".into(),
            stopped_x: 4012.0,
            stopped_y: 4000.0,
        }),
    );
    let me = rig.pose(300, ME);
    assert_eq!(me.x, 4012.0, "no blend: straight there");
    assert!(!me.moving);
    assert_eq!(rig.pose(600, ME).x, 4012.0);
    assert_eq!(rig.motion.destination(), None);
}

#[test]
fn starting_to_extract_stops_the_prediction() {
    let mut rig = Rig::new();
    rig.snapshot(0, &[(ME, 4000.0, 4000.0)]);
    rig.move_to(0, 4100.0, 4000.0);
    rig.sample(100);
    rig.feed(
        200,
        NetEvent::Ws(M::ExtractionStarted {
            player_id: ME,
            node_id: Uuid::from_u128(10),
            duration_ticks: 30,
        }),
    );
    assert_eq!(rig.motion.destination(), None);
    // Stopped where the prediction was when it heard.
    assert!(close(rig.pose(500, ME).x, 4010.0, 1e-9));
}

#[test]
fn someone_elses_extraction_doesnt_stop_me() {
    let mut rig = Rig::new();
    rig.snapshot(0, &[(ME, 4000.0, 4000.0), (OTHER, 4100.0, 4000.0)]);
    rig.move_to(0, 4100.0, 4000.0);
    rig.feed(
        100,
        NetEvent::Ws(M::ExtractionStarted {
            player_id: OTHER,
            node_id: Uuid::from_u128(10),
            duration_ticks: 30,
        }),
    );
    assert!(rig.motion.destination().is_some());
}

#[test]
fn a_reconnect_snaps_everyone_to_the_next_snapshot() {
    let mut rig = Rig::new();
    for k in 0..=5 {
        rig.snapshot(k * 100, &[(ME, 4000.0, 4000.0), (OTHER, tick_x(k), 4000.0)]);
    }
    rig.move_to(500, 4100.0, 4000.0);
    rig.sample(550);

    rig.feed(
        2000,
        NetEvent::Connection(ConnectionState::Connected { player_id: ME }),
    );
    rig.feed(2000, NetEvent::Resync);
    assert_eq!(
        rig.motion.destination(),
        None,
        "the server re-added us idle"
    );
    // Small differences snap too: nothing is blended across a reconnect.
    rig.snapshot(2100, &[(ME, 4010.0, 4000.0), (OTHER, 4040.0, 4000.0)]);
    let poses = rig.sample(2100);
    assert_eq!((poses[0].x, poses[1].x), (4010.0, 4040.0));
}

// ---------------------------------------------------------------------------
// On the land (W4)
// ---------------------------------------------------------------------------

mod on_the_land {
    use super::*;
    use capitol_protocol::fmath::StableMath;
    use capitol_protocol::land::{self, LandParams};
    use capitol_protocol::rules::{CENTER_X, CENTER_Y};
    use capitol_protocol::terrain::{Bridge, Mouth, WaterCourse, WaterPoint, WaterRing};

    /// A river 20 wide running north to south at x = 4050, on flat land.
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

    #[test]
    fn a_river_stops_the_prediction_at_its_bank() {
        let mut rig = Rig::new();
        rig.motion.set_ground(Some(river()));
        rig.snapshot(0, &[(ME, 4000.0, 4000.0)]);
        rig.move_to(0, 4100.0, 4000.0);
        let me = rig.pose(2000, ME);
        assert!((4039.0..=4040.0).contains(&me.x), "stopped at {}", me.x);
        assert!(!me.moving);
        assert_eq!(rig.motion.destination(), None);
    }

    /// Somewhere in the wilds that rises about 1 in 2, and the way up it.
    fn hillside(land: &LandParams) -> ((f64, f64), (f64, f64)) {
        let at = (0..3600)
            .map(|i| {
                let a = (i as f64 * 0.1).to_radians();
                (6800.0 + 5400.0 * a.stable_cos(), 6800.0 + 5400.0 * a.stable_sin())
            })
            .find(|&(x, y)| (0.45..0.55).contains(&land::slope_at(x, y, land)))
            .expect("a hillside");
        let (gx, gy) = land::gradient_at(at.0, at.1, land);
        let len = gx.stable_hypot(gy);
        (at, (at.0 + gx / len * 60.0, at.1 + gy / len * 60.0))
    }

    /// The local player walks up a hill for 2 s while a server a round trip
    /// behind walks the same `Ground`. How many times the prediction was
    /// corrected.
    fn climb(predict: bool) -> u32 {
        let land = LandParams::default();
        let ground = Arc::new(Ground::new(&land, &[], &[], &[], &[], &[]));
        let (from, to) = hillside(&land);
        let mut rig = Rig::new();
        if predict {
            rig.motion.set_ground(Some(ground.clone()));
        }
        rig.snapshot(0, &[(ME, from.0, from.1)]);
        rig.move_to(0, to.0, to.1);
        // The server: 5 units of walking per tick, starting 120 ms late.
        let mut server = from;
        for ms in (10..=2000).step_by(10) {
            if ms % 100 == 0 {
                if ms > 120 {
                    let step = ground.step(server, to, 5.0);
                    server = (step.x, step.y);
                }
                rig.snapshot(ms, &[(ME, server.0, server.1)]);
            }
            rig.sample(ms);
        }
        rig.motion.corrections()
    }

    /// Flat land with a canal 600 round the centre, 30 wide, bridged only
    /// due south.
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

    /// Click mode, square into a canal (T0.1, #315): the prediction stops
    /// at the bank on the tick the server does, where the server does, and
    /// is never pulled back.
    #[test]
    fn a_canal_stops_the_prediction_where_it_stops_the_server() {
        let ground = canal();
        let mut rig = Rig::new();
        rig.motion.set_ground(Some(ground.clone()));
        // East of the centre, outside the canal, walking west across it.
        let from = (CENTER_X + 660.0, CENTER_Y);
        let to = (CENTER_X + 500.0, CENTER_Y);
        rig.snapshot(0, &[(ME, from.0, from.1)]);
        rig.move_to(0, to.0, to.1);
        // The server, a round trip behind: 5 a tick, from 120 ms.
        let mut server = from;
        let mut server_stopped = None;
        let mut predicted_stop = None;
        for ms in (10..=3000).step_by(10) {
            if ms % 100 == 0 {
                if ms > 120 && server_stopped.is_none() {
                    let step = ground.step(server, to, 5.0);
                    server = (step.x, step.y);
                    if step.blocked.is_some() {
                        server_stopped = Some(ms);
                    }
                }
                rig.snapshot(ms, &[(ME, server.0, server.1)]);
            }
            let me = rig.pose(ms, ME);
            if predicted_stop.is_none() && rig.motion.destination().is_none() {
                predicted_stop = Some((ms, me.x, me.y));
            }
        }
        let server_ms = server_stopped.expect("the canal stopped the server");
        let (ms, x, y) = predicted_stop.expect("the prediction stopped");
        // The canal's outer bank is at 615: both stop 2 short of it.
        assert!((server.0 - (CENTER_X + 617.0)).abs() < 1e-6, "the server stopped at {}", server.0);
        assert!((x - server.0).abs() < 1e-6 && (y - server.1).abs() < 1e-6, "predicted ({x}, {y}), server {server:?}");
        // The prediction runs a round trip ahead, so it stops that much sooner.
        assert!(ms <= server_ms && server_ms - ms <= 200, "predicted at {ms} ms, the server at {server_ms} ms");
        assert_eq!(rig.motion.corrections(), 0);
    }

    #[test]
    fn a_climb_predicted_on_the_ground_matches_the_server() {
        assert_eq!(climb(true), 0, "the prediction rubber-banded on the hill");
        // Without the ground, the flat prediction runs ahead and is pulled back.
        assert!(climb(false) > 0);
    }
}

#[test]
fn stats_show_the_corrections_and_the_interpolation_depth() {
    let mut rig = Rig::new();
    assert_eq!(rig.motion.stats(rig.at(0)), MotionStats::default());

    rig.snapshot(0, &[(ME, 4000.0, 4100.0), (OTHER, 4000.0, 4000.0)]);
    rig.snapshot(100, &[(ME, 4000.0, 4100.0), (OTHER, 4005.0, 4000.0)]);
    let stats = rig.motion.stats(rig.at(100));
    assert_eq!(stats.tracks, 1);
    // On time, the drawn moment is the delay behind the newest snapshot.
    assert!(close(stats.interp_depth.unwrap(), 0.150, 1e-6), "{stats:?}");
    // Late: the drawn moment has passed the newest, and it extrapolates.
    let late = rig.motion.stats(rig.at(400)).interp_depth.unwrap();
    assert!(close(late, -0.150, 1e-6), "{late}");
    assert_eq!(stats.corrections, 0);

    // The move is lost: the server stands while the prediction walks 40
    // units, and is pulled back.
    rig.move_to(100, 4100.0, 4100.0);
    for ms in (110..900).step_by(10) {
        rig.sample(ms);
    }
    rig.snapshot(900, &[(ME, 4000.0, 4100.0), (OTHER, 4005.0, 4000.0)]);
    let stats = rig.motion.stats(rig.at(900));
    assert_eq!(stats.corrections, 1);
    assert!(close(stats.last_correction, 40.0, 1e-6), "{stats:?}");
    assert_eq!(stats.corrected, stats.last_correction);
    assert!(stats.lag.is_some());
}
