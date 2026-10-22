use crate::models::{PlayerState, Zone, ZoneTransition};

use capitol_protocol::rules::ground::{Blocked, Ground};
use capitol_protocol::rules::steer;
use capitol_protocol::rules::{ARRIVAL_EPSILON, TICK_SECS as TICK_DURATION_SECS};

/// Result of processing movement
pub struct MovementResult {
#[cfg_attr(not(test), allow(dead_code))] // the tests read it; the tick goes by the zone and the block
    pub moved: bool,
    pub zone_transition: Option<ZoneTransition>,
    pub blocked: Option<MovementBlocked>,
}

/// Information about blocked movement
pub struct MovementBlocked {
    pub reason: String,
    /// The water that stopped the move: a ring (`moat`, `canal_inner`, ...),
    /// a river or a lake (`river_3`, `lake_1`). Empty when it was too steep,
    /// or the edge of the world.
    pub water_id: String,
    pub stopped_x: f64,
    pub stopped_y: f64,
}

impl MovementBlocked {
    fn at(player: &PlayerState, blocked: Blocked) -> Self {
        MovementBlocked {
            reason: blocked.reason(),
            water_id: blocked.water_id().to_string(),
            stopped_x: player.x,
            stopped_y: player.y,
        }
    }
}

/// Take a `steer` (controls T0.2): walk on `heading` at `throttle` tenths
/// of the speed, after walking to `at`, by [`steer::take`]. It clears any
/// destination, and a `move` clears it in turn. The caller cancels a gather
/// or a craft first when it walks (`throttle` over 0), as for a `move`.
pub fn steer(player: &mut PlayerState, heading: u16, throttle: u8, at: Option<(f64, f64)>) {
    if steer::take(&mut player.steer, (player.x, player.y), heading, throttle, at) {
        player.dest_x = None;
        player.dest_y = None;
    }
}

/// One tick of a player's walk, by the shared ground step: slowed by the
/// slope and the water, stopped by a cliff, a river or a lake (W4), the
/// moat and the canals away from their bridges and the edge of the world,
/// and sliding along what stopped it where it can (T0.1, #315). The Flint
/// client predicts with the same step.
///
/// A steering player walks the steer ([`steer_step`]); anyone else walks to
/// their destination, if they have one.
pub fn process_movement(player: &mut PlayerState, ground: &Ground) -> MovementResult {
    let old_zone = player.current_zone.clone();
    let blocked_result = if player.steer.is_some() {
        steer_step(player, ground)
    } else if let (Some(x), Some(y)) = (player.dest_x, player.dest_y) {
        dest_step(player, ground, (x, y))
    } else {
        return MovementResult { moved: false, zone_transition: None, blocked: None };
    };

    // Check for zone transition
    let new_zone = Zone::from_position(player.x, player.y);
    let new_zone_str = new_zone.as_str().to_string();

    let zone_transition = if old_zone.as_ref() != Some(&new_zone_str) {
        player.current_zone = Some(new_zone_str.clone());
        Some(ZoneTransition {
            from_zone: old_zone.unwrap_or_else(|| "unknown".to_string()),
            to_zone: new_zone_str,
        })
    } else {
        None
    };

    MovementResult {
        moved: true,
        zone_transition,
        blocked: blocked_result,
    }
}

/// A tick toward the destination by `Ground::step`. A block ends the walk.
fn dest_step(player: &mut PlayerState, ground: &Ground, (dest_x, dest_y): (f64, f64)) -> Option<MovementBlocked> {
    let dx = dest_x - player.x;
    let dy = dest_y - player.y;
    let distance = (dx * dx + dy * dy).sqrt();

    if distance < ARRIVAL_EPSILON {
        // Arrived at destination
        player.x = dest_x;
        player.y = dest_y;
        player.dest_x = None;
        player.dest_y = None;
        return None;
    }
    let walk = player.speed as f64 * TICK_DURATION_SECS;
    let step = ground.step((player.x, player.y), (dest_x, dest_y), walk);
    player.x = step.x;
    player.y = step.y;

    match step.blocked {
        Some(blocked) => {
            // Clear destination - player can't reach it
            player.dest_x = None;
            player.dest_y = None;
            Some(MovementBlocked::at(player, blocked))
        }
        None => {
            if (step.x, step.y) == (dest_x, dest_y) {
                player.dest_x = None;
                player.dest_y = None;
            }
            None
        }
    }
}

/// A tick of a steer, by [`steer::tick`]: to the next waypoint, or else
/// along the heading, and the lease run down.
fn steer_step(player: &mut PlayerState, ground: &Ground) -> Option<MovementBlocked> {
    let walk = player.speed as f64 * TICK_DURATION_SECS;
    let step = steer::tick(&mut player.steer, (player.x, player.y), ground, walk);
    player.x = step.x;
    player.y = step.y;
    step.blocked.map(|blocked| MovementBlocked::at(player, blocked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActionState, TerrainConfig};
    use capitol_protocol::fmath::StableMath;
    use capitol_protocol::land::{gradient_at, slope_at};
    use capitol_protocol::rules::ground::{Footing, CLIFF_SLOPE, TOO_STEEP};
    use capitol_protocol::terrain::WaterCourse;
    use std::sync::OnceLock;
    use uuid::Uuid;

    fn make_player(x: f64, y: f64, dest_x: Option<f64>, dest_y: Option<f64>) -> PlayerState {
        PlayerState {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            name: "Test".into(),
            x,
            y,
            dest_x,
            dest_y,
            speed: 50.0,
            strand_balance: 0,
            current_zone: None,
            action_state: ActionState::Idle,
            citizen: false,
            steer: None,
        }
    }

    /// The default world and its ground, generated once for every test.
    fn world() -> &'static (TerrainConfig, Ground) {
        static WORLD: OnceLock<(TerrainConfig, Ground)> = OnceLock::new();
        WORLD.get_or_init(|| {
            let terrain = TerrainConfig::get_default();
            let ground = terrain.ground();
            (terrain, ground)
        })
    }

    #[test]
    fn test_movement_toward_destination() {
        // Start inside Capitol, move within Capitol
        let mut player = make_player(8300.0, 8300.0, Some(8350.0), Some(8300.0));
        let result = process_movement(&mut player, &world().1);

        assert!(result.moved);
        assert!(player.x > 8300.0);
        assert_eq!(player.y, 8300.0);
    }

    #[test]
    fn test_arrival_at_destination() {
        let mut player = make_player(8349.9, 8300.0, Some(8350.0), Some(8300.0));
        let result = process_movement(&mut player, &world().1);

        assert!(result.moved);
        assert!((player.x - 8350.0).abs() < 1.0);
        assert!(player.dest_x.is_none());
    }

    #[test]
    fn test_zone_transition() {
        // Start at center (Capitol)
        let mut player = make_player(8300.0, 8300.0, Some(8300.0), Some(8450.0));
        player.current_zone = Some("capitol".to_string());

        // Move south toward the moat bridge at 270° (south)
        // Keep moving until we cross the moat using the bridge
        for _ in 0..200 {
            let result = process_movement(&mut player, &world().1);
            if let Some(transition) = result.zone_transition {
                if transition.to_zone == "trade_district" {
                    return; // Success!
                }
            }
            // Reset destination if cleared (blocked by water)
            if player.dest_x.is_none() {
                break;
            }
        }
        // This path goes through the south bridge, so should reach trade district
    }

    #[test]
    fn test_no_movement_without_destination() {
        let mut player = make_player(8300.0, 8300.0, None, None);
        let result = process_movement(&mut player, &world().1);

        assert!(!result.moved);
        assert!(result.zone_transition.is_none());
    }

    #[test]
    fn test_movement_blocked_by_water() {
        // Try to cross moat going east (no bridge at 0° for moat)
        let mut player = make_player(8480.0, 8300.0, Some(8550.0), Some(8300.0));

        // Move until blocked
        for _ in 0..50 {
            let result = process_movement(&mut player, &world().1);
            if let Some(blocked) = result.blocked {
                assert_eq!(blocked.water_id, "moat");
                assert_eq!(blocked.reason, "Cannot cross the moat without a bridge");
                return;
            }
        }
        panic!("the moat never stopped the walk");
    }

    /// Walks `player` until it arrives or is stopped; what stopped it.
    fn walk(player: &mut PlayerState) -> Option<MovementBlocked> {
        let ground = &world().1;
        for _ in 0..2000 {
            let result = process_movement(player, ground);
            if result.blocked.is_some() {
                return result.blocked;
            }
            // Arrived: nothing stopped it
            player.dest_x?;
        }
        panic!("the walk never ended");
    }

    /// Whether a straight walk from `a` to `b` is gentle: nowhere near a cliff.
    fn gentle(a: (f64, f64), b: (f64, f64)) -> bool {
        let land = world().1.land();
        (0..=40).all(|i| {
            let t = i as f64 / 40.0;
            slope_at(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, land) < CLIFF_SLOPE * 0.8
        })
    }

    /// A crossing of a river or stream: the course, the point it's square
    /// to, and the two banks.
    type Crossing<'a> = (&'a WaterCourse, (f64, f64), (f64, f64), (f64, f64));

    /// Crossings of `courses` at their points: the two banks, `margin` past
    /// each, square to the water, where the walk between them is gentle and
    /// both banks are dry.
    fn crossings(courses: &[WaterCourse], margin: f64) -> impl Iterator<Item = Crossing<'_>> {
        let ground = &world().1;
        courses.iter().flat_map(move |c| {
            (1..c.points.len().saturating_sub(1)).filter_map(move |i| {
                let (p, prev, next) = (c.points[i], c.points[i - 1], c.points[i + 1]);
                let (tx, ty) = (next.x - prev.x, next.y - prev.y);
                let len = (tx * tx + ty * ty).sqrt();
                if len == 0.0 {
                    return None;
                }
                let off = p.width / 2.0 + margin;
                let (nx, ny) = (-ty / len * off, tx / len * off);
                let (a, b) = ((p.x - nx, p.y - ny), (p.x + nx, p.y + ny));
                let dry = |q: (f64, f64)| ground.water_at(q.0, q.1).is_none();
                (dry(a) && dry(b) && gentle(a, b)).then_some((c, (p.x, p.y), a, b))
            })
        })
    }

    fn apart(a: (f64, f64), b: (f64, f64)) -> f64 {
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    }

    #[test]
    fn a_climb_is_slower_than_the_flat() {
        let land = world().1.land();
        // On the flat in the city, a tick is a whole 5 units.
        let mut flat = make_player(8300.0, 11600.0, Some(8300.0), Some(11700.0));
        process_movement(&mut flat, &world().1);
        assert!((flat.y - 11605.0).abs() < 1e-9);

        // Somewhere in the wilds that rises 1 in 2, dry, heading straight up it.
        let (x, y) = (0..3600)
            .map(|i| {
                let a = (i as f64 * 0.1).to_radians();
                (8300.0 + 6900.0 * a.stable_cos(), 8300.0 + 6900.0 * a.stable_sin())
            })
            .find(|&(x, y)| {
                let s = slope_at(x, y, land);
                (0.4..0.6).contains(&s) && world().1.water_at(x, y).is_none()
            })
            .expect("a hillside in the wilds");
        let (gx, gy) = gradient_at(x, y, land);
        let len = (gx * gx + gy * gy).sqrt();
        let mut climber = make_player(x, y, Some(x + gx / len * 100.0), Some(y + gy / len * 100.0));
        let result = process_movement(&mut climber, &world().1);
        assert!(result.blocked.is_none());
        let up = apart((climber.x, climber.y), (x, y));
        assert!(up > 2.0 && up < 4.0, "a climb at {len:.2} went {up:.2} in a tick");

        // Down the same slope is quicker than up, and slower than the flat.
        let mut descender = make_player(x, y, Some(x - gx / len * 100.0), Some(y - gy / len * 100.0));
        process_movement(&mut descender, &world().1);
        let down = apart((descender.x, descender.y), (x, y));
        assert!(down > up && down < 5.0, "down {down:.2}, up {up:.2}");
    }

    #[test]
    fn a_cliff_stops_you() {
        let land = world().1.land();
        let crag = (0..3600)
            .map(|i| {
                let a = (i as f64 * 0.1).to_radians();
                (8300.0 + 7300.0 * a.stable_cos(), 8300.0 + 7300.0 * a.stable_sin())
            })
            .find(|&(x, y)| slope_at(x, y, land) > CLIFF_SLOPE + 0.3)
            .expect("a crag in the wilds");
        // Down from it to gentle ground, then back up the way we came, a
        // waypoint at a time, as a player clicking their way up would.
        let mut foot = crag;
        let mut path = vec![foot];
        while slope_at(foot.0, foot.1, land) > 0.6 {
            let (gx, gy) = gradient_at(foot.0, foot.1, land);
            let len = (gx * gx + gy * gy).sqrt();
            foot = (foot.0 - gx / len, foot.1 - gy / len);
            path.push(foot);
        }
        let mut player = make_player(foot.0, foot.1, None, None);
        for &(x, y) in path.iter().rev().step_by(5).chain(std::iter::once(&crag)) {
            player.dest_x = Some(x);
            player.dest_y = Some(y);
            if let Some(blocked) = walk(&mut player) {
                assert_eq!(blocked.reason, TOO_STEEP);
                assert_eq!(blocked.water_id, "");
                assert!(slope_at(player.x, player.y, land) <= CLIFF_SLOPE + 0.05);
                return;
            }
        }
        panic!("climbed a crag of {:.2}", slope_at(crag.0, crag.1, land));
    }

    #[test]
    fn a_river_stops_you() {
        let (terrain, ground) = world();
        let mut tried = 0;
        for (river, at, a, b) in crossings(&terrain.rivers, 15.0) {
            // Away from its fords and from roads.
            if ground.footing(at.0, at.1) != Footing::Blocked(&river.id) {
                continue;
            }
            tried += 1;
            let mut player = make_player(a.0, a.1, Some(b.0), Some(b.1));
            let blocked = walk(&mut player).expect("the river stops the walk");
            assert_eq!(blocked.water_id, river.id);
            assert_eq!(blocked.reason, "Cannot cross the river without a ford or a bridge");
            // On the near bank, short of the water.
            assert_eq!(ground.footing(player.x, player.y), Footing::Dry);
            assert!(apart((player.x, player.y), a) < 16.0);
            if tried == 20 {
                return;
            }
        }
        assert!(tried > 0, "no river to try");
    }

    #[test]
    fn a_ford_lets_you_over() {
        let (terrain, _) = world();
        let mut crossed = 0;
        for ford in &terrain.fords {
            let river = terrain.rivers.iter().find(|r| r.id == ford.river_id).expect("its river");
            let Some((_, _, a, b)) = crossings(std::slice::from_ref(river), 3.0)
                .find(|&(_, at, _, _)| apart(at, (ford.x, ford.y)) < 1.0)
            else {
                continue;
            };
            let mut player = make_player(a.0, a.1, Some(b.0), Some(b.1));
            assert!(walk(&mut player).is_none(), "{} at ({:.0}, {:.0})", ford.id, ford.x, ford.y);
            assert_eq!((player.x, player.y), b);
            crossed += 1;
        }
        assert!(crossed >= 5, "only {crossed} fords to cross");
    }

    #[test]
    fn a_stream_is_waded() {
        let (terrain, ground) = world();
        let mut crossed = 0;
        for (stream, at, a, b) in crossings(&terrain.streams, 5.0) {
            if ground.footing(at.0, at.1) != Footing::Wading(&stream.id) {
                continue;
            }
            // Only this stream between the banks.
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            if (1..40).any(|i| {
                let t = i as f64 / 40.0;
                matches!(ground.footing(a.0 + dx * t, a.1 + dy * t), Footing::Blocked(_))
            }) {
                continue;
            }
            let mut player = make_player(a.0, a.1, Some(b.0), Some(b.1));
            assert!(walk(&mut player).is_none(), "{} at ({:.0}, {:.0})", stream.id, at.0, at.1);
            assert_eq!((player.x, player.y), b);
            crossed += 1;
            if crossed == 20 {
                return;
            }
        }
        assert!(crossed > 0, "no stream to wade");
    }

    // Steering (controls T0.2, #316)

    use capitol_protocol::rules::steer::{QUARTER, LEASE_TICKS, MAX_WAYPOINTS};
    use capitol_protocol::rules::{CENTER_X, CENTER_Y, WILDERNESS_OUTER_RADIUS};

    const EAST: u16 = 0;
    const SOUTH: u16 = QUARTER;

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    fn at(player: &PlayerState) -> (f64, f64) {
        (player.x, player.y)
    }

    #[test]
    fn a_steer_walks_its_heading_at_a_moves_speed() {
        let ground = &world().1;
        let mut walker = make_player(CENTER_X, CENTER_Y, Some(CENTER_X + 1000.0), Some(CENTER_Y));
        let mut steerer = make_player(CENTER_X, CENTER_Y, None, None);
        steer(&mut steerer, EAST, 10, None);
        for _ in 0..10 {
            process_movement(&mut walker, ground);
            process_movement(&mut steerer, ground);
            assert!(near(at(&steerer), at(&walker)), "{:?} vs {:?}", at(&steerer), at(&walker));
        }
        assert!(near(at(&steerer), (CENTER_X + 50.0, CENTER_Y)), "a second at 50 a second");

        // Half throttle, half the pace; and south is +y
        let mut half = make_player(CENTER_X, CENTER_Y, None, None);
        steer(&mut half, SOUTH, 5, None);
        process_movement(&mut half, ground);
        assert!(near(at(&half), (CENTER_X, CENTER_Y + 2.5)), "{:?}", at(&half));

        // Over 10 is clamped to full
        let mut over = make_player(CENTER_X, CENTER_Y, None, None);
        steer(&mut over, EAST, 200, None);
        assert_eq!(over.steer.as_ref().unwrap().throttle, 10);
    }

    #[test]
    fn a_steer_clears_the_destination_and_a_zero_steer_with_none_does_nothing() {
        let dest = (Some(CENTER_X + 40.0), Some(CENTER_Y));
        let mut player = make_player(CENTER_X, CENTER_Y, dest.0, dest.1);
        // Letting go of W while a click walk (or E's walk to a node) is under way
        steer(&mut player, EAST, 0, Some((CENTER_X, CENTER_Y)));
        assert!(player.steer.is_none());
        assert_eq!((player.dest_x, player.dest_y), dest, "the walk carries on");

        steer(&mut player, SOUTH, 10, None);
        assert!(player.steer.is_some());
        assert_eq!((player.dest_x, player.dest_y), (None, None));
    }

    #[test]
    fn a_waypoint_is_walked_before_the_heading() {
        let ground = &world().1;
        let mut player = make_player(CENTER_X, CENTER_Y, None, None);
        // Heading south, from a corner 3 east
        steer(&mut player, SOUTH, 10, Some((CENTER_X + 3.0, CENTER_Y)));
        process_movement(&mut player, ground);
        assert_eq!(at(&player), (CENTER_X + 3.0, CENTER_Y), "the tick that reaches the corner ends on it");
        process_movement(&mut player, ground);
        assert!(near(at(&player), (CENTER_X + 3.0, CENTER_Y + 5.0)), "{:?}", at(&player));
    }

    #[test]
    fn a_waypoint_keeps_the_pace_it_was_marked_at() {
        let ground = &world().1;
        let mut player = make_player(CENTER_X, CENTER_Y, None, None);
        steer(&mut player, EAST, 5, None);
        // Speeding up at a corner 10 on: the server walks there at the old 5
        steer(&mut player, SOUTH, 10, Some((CENTER_X + 10.0, CENTER_Y)));
        process_movement(&mut player, ground);
        assert!(near(at(&player), (CENTER_X + 2.5, CENTER_Y)), "{:?}", at(&player));
    }

    #[test]
    fn a_stop_walks_to_its_at_and_clears() {
        let ground = &world().1;
        let mut player = make_player(CENTER_X, CENTER_Y, None, None);
        steer(&mut player, EAST, 10, None);
        process_movement(&mut player, ground);
        // The client stopped 12 further on than the server has it
        let stop = (CENTER_X + 17.0, CENTER_Y);
        steer(&mut player, EAST, 0, Some(stop));
        let mut ticks = 0;
        while player.steer.is_some() {
            process_movement(&mut player, ground);
            ticks += 1;
            assert!(ticks < 10, "the stop never ended");
        }
        assert_eq!(ticks, 3, "5, 5 and the last 2");
        assert_eq!(at(&player), stop, "it stands where the client stopped, not a step on");
        process_movement(&mut player, ground);
        assert_eq!(at(&player), stop);

        // A stop with no `at` stands at once
        steer(&mut player, EAST, 10, None);
        steer(&mut player, EAST, 0, None);
        process_movement(&mut player, ground);
        assert_eq!(at(&player), stop);
        assert!(player.steer.is_none());
    }

    #[test]
    fn an_at_too_far_or_off_the_world_is_ignored() {
        let mut player = make_player(CENTER_X, CENTER_Y, None, None);
        steer(&mut player, EAST, 10, Some((CENTER_X + 100.5, CENTER_Y)));
        assert!(player.steer.as_ref().unwrap().waypoints.is_empty(), "over 100 away");
        steer(&mut player, EAST, 10, Some((CENTER_X, CENTER_Y + 100.0)));
        assert_eq!(player.steer.as_ref().unwrap().waypoints.len(), 1, "100 is in reach");

        let edge = CENTER_X + WILDERNESS_OUTER_RADIUS;
        let mut out = make_player(edge - 1.0, CENTER_Y, None, None);
        steer(&mut out, EAST, 10, Some((edge + 5.0, CENTER_Y)));
        assert!(out.steer.as_ref().unwrap().waypoints.is_empty(), "off the world");
    }

    #[test]
    fn past_eight_waypoints_the_oldest_goes() {
        let mut player = make_player(CENTER_X, CENTER_Y, None, None);
        for k in 0..=MAX_WAYPOINTS {
            steer(&mut player, EAST, 10, Some((CENTER_X + k as f64, CENTER_Y)));
        }
        let waypoints = &player.steer.as_ref().unwrap().waypoints;
        assert_eq!(waypoints.len(), MAX_WAYPOINTS);
        assert_eq!(waypoints.front().unwrap().x, CENTER_X + 1.0);
    }

    #[test]
    fn the_lease_runs_out_after_30_ticks_unless_renewed() {
        let ground = &world().1;
        // At 2 tenths, a unit a tick: short of the moat
        let mut player = make_player(CENTER_X, CENTER_Y, None, None);
        steer(&mut player, EAST, 2, None);
        for _ in 1..LEASE_TICKS {
            process_movement(&mut player, ground);
        }
        assert!(player.steer.is_some(), "29 ticks in");
        // A renewal starts it over
        steer(&mut player, EAST, 2, None);
        for _ in 0..LEASE_TICKS {
            process_movement(&mut player, ground);
        }
        assert!(player.steer.is_none(), "30 ticks after the last steer");
        let walked = at(&player);
        assert!(near(walked, (CENTER_X + f64::from(2 * LEASE_TICKS - 1), CENTER_Y)), "{walked:?}");
        process_movement(&mut player, ground);
        assert_eq!(at(&player), walked, "and it stands");
    }

    #[test]
    fn blocked_on_its_heading_a_steer_stands_and_keeps_going() {
        let ground = &world().1;
        // Square into the world's edge
        let start = (CENTER_X + WILDERNESS_OUTER_RADIUS - 1.0, CENTER_Y);
        let mut player = make_player(start.0, start.1, None, None);
        steer(&mut player, EAST, 10, None);
        for _ in 0..3 {
            let result = process_movement(&mut player, ground);
            assert!(result.blocked.is_some());
            assert_eq!(at(&player), start);
            assert!(player.steer.is_some(), "the steer stays, for when it turns");
        }
        // Turning back walks
        steer(&mut player, 2 * QUARTER, 10, None);
        let result = process_movement(&mut player, ground);
        assert!(result.blocked.is_none());
        assert!(player.x < start.0);
    }

    #[test]
    fn a_steer_is_never_saved() {
        let mut player = make_player(CENTER_X, CENTER_Y, None, None);
        steer(&mut player, EAST, 10, Some((CENTER_X + 1.0, CENTER_Y)));
        let json = serde_json::to_string(&player).unwrap();
        assert!(!json.contains("steer"), "{json}");
        let back: PlayerState = serde_json::from_str(&json).unwrap();
        assert!(back.steer.is_none());
    }
}
