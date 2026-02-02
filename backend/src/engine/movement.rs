use crate::models::PlayerState;

const TICK_DURATION_SECS: f64 = 0.1; // 100ms

pub fn process_movement(player: &mut PlayerState) -> bool {
    let (dest_x, dest_y) = match (player.dest_x, player.dest_y) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };

    let dx = dest_x - player.x;
    let dy = dest_y - player.y;
    let distance = (dx * dx + dy * dy).sqrt();

    if distance < 0.5 {
        // Arrived at destination
        player.x = dest_x;
        player.y = dest_y;
        player.dest_x = None;
        player.dest_y = None;
        return true;
    }

    let move_distance = player.speed as f64 * TICK_DURATION_SECS;

    if move_distance >= distance {
        // Will arrive this tick
        player.x = dest_x;
        player.y = dest_y;
        player.dest_x = None;
        player.dest_y = None;
    } else {
        // Move toward destination
        let ratio = move_distance / distance;
        player.x += dx * ratio;
        player.y += dy * ratio;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_movement_toward_destination() {
        let mut player = PlayerState {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            name: "Test".into(),
            x: 0.0,
            y: 0.0,
            dest_x: Some(100.0),
            dest_y: Some(0.0),
            speed: 50.0,
        };

        process_movement(&mut player);

        assert!(player.x > 0.0);
        assert_eq!(player.y, 0.0);
    }

    #[test]
    fn test_arrival_at_destination() {
        let mut player = PlayerState {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            name: "Test".into(),
            x: 99.9,
            y: 0.0,
            dest_x: Some(100.0),
            dest_y: Some(0.0),
            speed: 50.0,
        };

        process_movement(&mut player);

        assert_eq!(player.x, 100.0);
        assert!(player.dest_x.is_none());
    }
}
