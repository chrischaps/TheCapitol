use axum::{
    extract::State,
    routing::post,
    Extension, Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::engine::Command;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/move", post(move_player))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

#[derive(Debug, Deserialize)]
pub struct MoveRequest {
    pub x: f64,
    pub y: f64,
}

pub async fn move_player(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<MoveRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Validate coordinates
    if req.x < 0.0 || req.x > 1000.0 || req.y < 0.0 || req.y > 1000.0 {
        return Err(AppError::BadRequest("Coordinates out of bounds".into()));
    }

    // Get player for this account
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Queue movement command
    let mut game = state.game.write().await;
    game.queue_command(Command::Move {
        player_id,
        dest_x: req.x,
        dest_y: req.y,
    });

    Ok(Json(serde_json::json!({
        "success": true,
        "destination": { "x": req.x, "y": req.y }
    })))
}
