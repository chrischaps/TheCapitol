use axum::{
    extract::{Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use uuid::Uuid;

use capitol_protocol::rest::{ExtractRequest, ExtractResponse, NearbyQuery, NearbyResponse};
use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};

use crate::engine::container_access::gathering_terms;
use crate::engine::{extraction_refusal, Command};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::ResourceNodeInfo;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/nearby", get(get_nearby))
        .route("/nodes", get(get_all_nodes))
        .route("/extract", post(start_extraction))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Get resource nodes near the player
#[utoipa::path(
    get,
    path = "/world/nearby",
    params(NearbyQuery),
    responses(
        (status = 200, description = "Nearby resource nodes", body = NearbyResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "World"
)]
pub async fn get_nearby(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<NearbyQuery>,
) -> Result<Json<NearbyResponse>, AppError> {
    let range = query.effective_radius();

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
        .unwrap_or((SPAWN_X, SPAWN_Y));

    // Filter nodes within range
    let nodes: Vec<ResourceNodeInfo> = game
        .resource_nodes
        .values()
        .filter(|node| {
            let dx = node.x - player_x;
            let dy = node.y - player_y;
            let distance = (dx * dx + dy * dy).sqrt();
            distance <= range
        })
        .map(ResourceNodeInfo::from)
        .collect();

    Ok(Json(NearbyResponse { nodes }))
}

/// Every resource node in the world, wherever the player is (E2.5). The
/// Flint client draws the woods and outcrops from it, so they stand on the
/// horizon rather than appearing as you walk; the nearby list and the
/// WebSocket keep the states near you current. About a thousand nodes on the
/// default world, fetched once a session.
#[utoipa::path(
    get,
    path = "/world/nodes",
    responses(
        (status = 200, description = "Every resource node", body = NearbyResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "World"
)]
pub async fn get_all_nodes(State(state): State<AppState>) -> Json<NearbyResponse> {
    let game = state.game.read().await;
    let mut nodes: Vec<ResourceNodeInfo> =
        game.resource_nodes.values().map(ResourceNodeInfo::from).collect();
    nodes.sort_by_key(|n| n.id);
    Json(NearbyResponse { nodes })
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

    // The best tool for it, if it takes one (E1.5)
    let tool = crate::engine::tools::tool_for_node(&state.db, &state.game, player_id, req.node_id).await?;

    // Validate that the node exists and is in range
    {
        let game = state.game.read().await;

        let player = game.players.get(&player_id).ok_or_else(|| {
            AppError::BadRequest("Player not in game".into())
        })?;

        let node = game.resource_nodes.get(&req.node_id).ok_or_else(|| {
            AppError::BadRequest("Resource node not found".into())
        })?;

        let resource_type = game.resource_types.get(&node.resource_type).ok_or_else(|| {
            AppError::BadRequest(format!("Unknown resource type '{}'", node.resource_type))
        })?;

        // The same reasons the tick gives over the WebSocket as extraction_failed
        if let Some(reason) = extraction_refusal(player, node, resource_type, tool.as_ref()) {
            return Err(AppError::BadRequest(reason));
        }
    }

    // Grass on someone's lot is theirs (#102), unless they're on a wage shift
    // for it (Commons D7)
    let wage = gathering_terms(&state.db, &state.game, player_id, req.node_id).await?.map_err(AppError::Forbidden)?;

    // Queue extraction command
    let mut game = state.game.write().await;
    game.queue_command(Command::StartExtraction {
        player_id,
        node_id: req.node_id,
        tool,
        wage,
    });

    Ok(Json(ExtractResponse {
        success: true,
        message: None,
    }))
}
