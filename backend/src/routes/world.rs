use axum::{
    extract::State,
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::engine::{Command, EXTRACTION_RANGE};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::ResourceNodeInfo;
use crate::state::AppState;

/// Maximum distance to return nearby nodes
const NEARBY_RANGE: f64 = 200.0;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/nearby", get(get_nearby))
        .route("/extract", post(start_extraction))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NearbyResponse {
    /// Resource nodes within range of the player
    pub nodes: Vec<ResourceNodeInfo>,
}

/// Get resource nodes near the player
#[utoipa::path(
    get,
    path = "/world/nearby",
    responses(
        (status = 200, description = "Nearby resource nodes", body = NearbyResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "World"
)]
pub async fn get_nearby(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<NearbyResponse>, AppError> {
    // Get player's current position from game state
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    let game = state.game.read().await;

    // Get player position
    let (player_x, player_y) = game
        .players
        .get(&player_id)
        .map(|p| (p.x, p.y))
        .unwrap_or((500.0, 500.0));

    // Filter nodes within range
    let nodes: Vec<ResourceNodeInfo> = game
        .resource_nodes
        .values()
        .filter(|node| {
            let dx = node.x - player_x;
            let dy = node.y - player_y;
            let distance = (dx * dx + dy * dy).sqrt();
            distance <= NEARBY_RANGE
        })
        .map(ResourceNodeInfo::from)
        .collect();

    Ok(Json(NearbyResponse { nodes }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExtractRequest {
    /// ID of the resource node to extract from
    pub node_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExtractResponse {
    /// Whether extraction was started successfully
    pub success: bool,
    /// Optional message (e.g., error reason)
    pub message: Option<String>,
}

/// Start extracting from a resource node
#[utoipa::path(
    post,
    path = "/world/extract",
    request_body = ExtractRequest,
    responses(
        (status = 200, description = "Extraction started", body = ExtractResponse),
        (status = 400, description = "Cannot extract (out of range, node unavailable)")
    ),
    security(("bearer_auth" = [])),
    tag = "World"
)]
pub async fn start_extraction(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<ExtractRequest>,
) -> Result<Json<ExtractResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Validate that the node exists and is in range
    {
        let game = state.game.read().await;

        let player = game.players.get(&player_id).ok_or_else(|| {
            AppError::BadRequest("Player not in game".into())
        })?;

        let node = game.resource_nodes.get(&req.node_id).ok_or_else(|| {
            AppError::BadRequest("Resource node not found".into())
        })?;

        // Check range
        let dx = player.x - node.x;
        let dy = player.y - node.y;
        let distance = (dx * dx + dy * dy).sqrt();
        if distance > EXTRACTION_RANGE {
            return Err(AppError::BadRequest("Too far from resource".into()));
        }

        // Check if node is available
        if node.state != crate::models::NodeState::Available {
            return Err(AppError::BadRequest("Resource is not available".into()));
        }
    }

    // Queue extraction command
    let mut game = state.game.write().await;
    game.queue_command(Command::StartExtraction {
        player_id,
        node_id: req.node_id,
    });

    Ok(Json(ExtractResponse {
        success: true,
        message: None,
    }))
}
