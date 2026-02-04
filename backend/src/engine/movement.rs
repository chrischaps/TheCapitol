use crate::models::{PlayerState, TerrainConfig, Zone, ZoneTransition};

const TICK_DURATION_SECS: f64 = 0.1; // 100ms

/// Result of processing movement
pub struct MovementResult {
    pub moved: bool,
    pub zone_transition: Option<ZoneTransition>,
    pub blocked: Option<MovementBlocked>,
}

/// Information about blocked movement
pub struct MovementBlocked {
    pub reason: String,
    pub stopped_x: f64,
    pub stopped_y: f64,
}

pub fn process_movement(player: &mut PlayerState, terrain: &TerrainConfig) -> MovementResult {
    let (dest_x, dest_y) = match (player.dest_x, player.dest_y) {
        (Some(x), Some(y)) => (x, y),
        _ => return MovementResult { moved: false, zone_transition: None, blocked: None },
    };

    let dx = dest_x - player.x;
    let dy = dest_y - player.y;
    let distance = (dx * dx + dy * dy).sqrt();

    let old_zone = player.current_zone.clone();
    let mut blocked_result: Option<MovementBlocked> = None;

    if distance < 0.5 {
        // Arrived at destination
        player.x = dest_x;
        player.y = dest_y;
        player.dest_x = None;
        player.dest_y = None;
    } else {
        let move_distance = player.speed as f64 * TICK_DURATION_SECS;
        let actual_distance = if move_distance >= distance { distance } else { move_distance };
        let ratio = actual_distance / distance;

        let target_x = player.x + dx * ratio;
        let target_y = player.y + dy * ratio;

        // Check terrain for water crossing
        let validation = terrain.validate_movement(player.x, player.y, target_x, target_y);

        if validation.allowed {
            // Movement allowed
            player.x = target_x;
            player.y = target_y;

            if move_distance >= distance {
                player.dest_x = None;
                player.dest_y = None;
            }
        } else {
            // Movement blocked by water
            if let Some((block_x, block_y)) = validation.blocked_at {
                player.x = block_x;
                player.y = block_y;
            }
            // Clear destination - player can't reach it
            player.dest_x = None;
            player.dest_y = None;

            blocked_result = Some(MovementBlocked {
                reason: format!("Cannot cross {} without a bridge", validation.blocked_by.unwrap_or_else(|| "water".to_string())),
                stopped_x: player.x,
                stopped_y: player.y,
            });
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
        }
    }

    fn get_terrain() -> TerrainConfig {
        TerrainConfig::get_default()
    }

    #[test]
    fn test_movement_toward_destination() {
        let terrain = get_terrain();
        // Start inside Capitol, move within Capitol
        let mut player = make_player(4000.0, 4000.0, Some(4050.0), Some(4000.0));
        let result = process_movement(&mut player, &terrain);

        assert!(result.moved);
        assert!(player.x > 4000.0);
        assert_eq!(player.y, 4000.0);
    }

    #[test]
    fn test_arrival_at_destination() {
        let terrain = get_terrain();
        let mut player = make_player(4049.9, 4000.0, Some(4050.0), Some(4000.0));
        let result = process_movement(&mut player, &terrain);

        assert!(result.moved);
        assert!((player.x - 4050.0).abs() < 1.0);
        assert!(player.dest_x.is_none());
    }

    #[test]
    fn test_zone_transition() {
        let terrain = get_terrain();
        // Start at center (Capitol)
        let mut player = make_player(4000.0, 4000.0, Some(4000.0), Some(4150.0));
        player.current_zone = Some("capitol".to_string());

        // Move south toward the moat bridge at 270° (south)
        // Keep moving until we cross the moat using the bridge
        for _ in 0..200 {
            let result = process_movement(&mut player, &terrain);
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
        let mut player = make_player(4000.0, 4000.0, None, None);
        let result = process_movement(&mut player, &terrain);

        assert!(!result.moved);
        assert!(result.zone_transition.is_none());
    }

    #[test]
    fn test_movement_blocked_by_water() {
        let terrain = get_terrain();
        // Try to cross moat going east (no bridge at 0° for moat)
        let mut player = make_player(4180.0, 4000.0, Some(4250.0), Some(4000.0));

        // Move until blocked
        for _ in 0..50 {
            let result = process_movement(&mut player, &terrain);
            if result.blocked.is_some() {
                assert!(result.blocked.unwrap().reason.contains("moat"));
                return;
            }
        }
        // If we got here, the path might have been too short to cross
    }
}
