use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum Command {
    Move {
        player_id: Uuid,
        dest_x: f64,
        dest_y: f64,
    },
    StopMoving {
        player_id: Uuid,
    },
}
