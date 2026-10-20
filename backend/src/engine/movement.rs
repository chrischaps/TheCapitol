use crate::models::{PlayerState, TerrainConfig, Zone, ZoneTransition};

use capitol_protocol::rules::ground::Ground;
use capitol_protocol::rules::{water_name, ARRIVAL_EPSILON, TICK_SECS as TICK_DURATION_SECS};

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
    /// a river or a lake (`river_3`, `lake_1`). Empty when it was too steep.
    pub water_id: String,
    pub stopped_x: f64,
    pub stopped_y: f64,
}

/// One tick of a player's walk: slowed by the slope and the water, stopped
/// by a cliff, a river or a lake (`Ground::step`, W4), and by the moat and
/// canals away from their bridges.
pub fn process_movement(player: &mut PlayerState, terrain: &TerrainConfig, ground: &Ground) -> MovementResult {
    let (dest_x, dest_y) = match (player.dest_x, player.dest_y) {
        (Some(x), Some(y)) => (x, y),
        _ => return MovementResult { moved: false, zone_transition: None, blocked: None },
    };

    let dx = dest_x - player.x;
    let dy = dest_y - player.y;
    let distance = (dx * dx + dy * dy).sqrt();

    let old_zone = player.current_zone.clone();
    let mut blocked_result: Option<MovementBlocked> = None;

    if distance < ARRIVAL_EPSILON {
        // Arrived at destination
        player.x = dest_x;
        player.y = dest_y;
        player.dest_x = None;
        player.dest_y = None;
    } else {
        let walk = player.speed as f64 * TICK_DURATION_SECS;
        let step = ground.step((player.x, player.y), (dest_x, dest_y), walk);

        // The moat and canals, over the step the land allows
        let validation = terrain.validate_movement(player.x, player.y, step.x, step.y);
        let blocked = if validation.allowed {
            player.x = step.x;
            player.y = step.y;
            step.blocked.map(|b| (b.reason(), b.water_id().to_string()))
        } else {
            if let Some((block_x, block_y)) = validation.blocked_at {
                player.x = block_x;
                player.y = block_y;
            }
            let water_id = validation.blocked_by.unwrap_or_default();
            Some((format!("Cannot cross {} without a bridge", water_name(&water_id)), water_id))
        };

        match blocked {
            Some((reason, water_id)) => {
                // Clear destination - player can't reach it
                player.dest_x = None;
                player.dest_y = None;
                blocked_result = Some(MovementBlocked {
                    reason,
                    water_id,
                    stopped_x: player.x,
                    stopped_y: player.y,
                });
            }
            None if (step.x, step.y) == (dest_x, dest_y) => {
                player.dest_x = None;
                player.dest_y = None;
            }
            None => {}
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ActionState;
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

    fn get_terrain() -> TerrainConfig {
        world().0.clone()
    }

    #[test]
    fn test_movement_toward_destination() {
        let terrain = get_terrain();
        // Start inside Capitol, move within Capitol
        let mut player = make_player(6000.0, 6000.0, Some(6050.0), Some(6000.0));
        let result = process_movement(&mut player, &terrain, &world().1);

        assert!(result.moved);
        assert!(player.x > 6000.0);
        assert_eq!(player.y, 6000.0);
    }

    #[test]
    fn test_arrival_at_destination() {
        let terrain = get_terrain();
        let mut player = make_player(6049.9, 6000.0, Some(6050.0), Some(6000.0));
        let result = process_movement(&mut player, &terrain, &world().1);

        assert!(result.moved);
        assert!((player.x - 6050.0).abs() < 1.0);
        assert!(player.dest_x.is_none());
    }

    #[test]
    fn test_zone_transition() {
        let terrain = get_terrain();
        // Start at center (Capitol)
        let mut player = make_player(6000.0, 6000.0, Some(6000.0), Some(6150.0));
        player.current_zone = Some("capitol".to_string());

        // Move south toward the moat bridge at 270° (south)
        // Keep moving until we cross the moat using the bridge
        for _ in 0..200 {
            let result = process_movement(&mut player, &terrain, &world().1);
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
        let terrain = get_terrain();
        let mut player = make_player(6000.0, 6000.0, None, None);
        let result = process_movement(&mut player, &terrain, &world().1);

        assert!(!result.moved);
        assert!(result.zone_transition.is_none());
    }

    #[test]
    fn test_movement_blocked_by_water() {
        let terrain = get_terrain();
        // Try to cross moat going east (no bridge at 0° for moat)
        let mut player = make_player(6180.0, 6000.0, Some(6250.0), Some(6000.0));

        // Move until blocked
        for _ in 0..50 {
            let result = process_movement(&mut player, &terrain, &world().1);
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
        let (terrain, ground) = world();
        for _ in 0..2000 {
            let result = process_movement(player, terrain, ground);
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
        let mut flat = make_player(6000.0, 9300.0, Some(6000.0), Some(9400.0));
        process_movement(&mut flat, &world().0, &world().1);
        assert!((flat.y - 9305.0).abs() < 1e-9);

        // Somewhere in the wilds that rises 1 in 2, dry, heading straight up it.
        let (x, y) = (0..3600)
            .map(|i| {
                let a = (i as f64 * 0.1).to_radians();
                (6000.0 + 4600.0 * a.stable_cos(), 6000.0 + 4600.0 * a.stable_sin())
            })
            .find(|&(x, y)| {
                let s = slope_at(x, y, land);
                (0.4..0.6).contains(&s) && world().1.water_at(x, y).is_none()
            })
            .expect("a hillside in the wilds");
        let (gx, gy) = gradient_at(x, y, land);
        let len = (gx * gx + gy * gy).sqrt();
        let mut climber = make_player(x, y, Some(x + gx / len * 100.0), Some(y + gy / len * 100.0));
        let result = process_movement(&mut climber, &world().0, &world().1);
        assert!(result.blocked.is_none());
        let up = apart((climber.x, climber.y), (x, y));
        assert!(up > 2.0 && up < 4.0, "a climb at {len:.2} went {up:.2} in a tick");

        // Down the same slope is quicker than up, and slower than the flat.
        let mut descender = make_player(x, y, Some(x - gx / len * 100.0), Some(y - gy / len * 100.0));
        process_movement(&mut descender, &world().0, &world().1);
        let down = apart((descender.x, descender.y), (x, y));
        assert!(down > up && down < 5.0, "down {down:.2}, up {up:.2}");
    }

    #[test]
    fn a_cliff_stops_you() {
        let land = world().1.land();
        let crag = (0..3600)
            .map(|i| {
                let a = (i as f64 * 0.1).to_radians();
                (6000.0 + 5000.0 * a.stable_cos(), 6000.0 + 5000.0 * a.stable_sin())
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
}
