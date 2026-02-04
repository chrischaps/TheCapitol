use axum::{
    extract::State,
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::engine::Command;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{InventoryItem, Item, ItemType};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/move", post(move_player))
        .route("/inventory", get(get_inventory))
        .route("/currency", get(get_currency))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MoveRequest {
    /// Destination X coordinate (0-1000)
    pub x: f64,
    /// Destination Y coordinate (0-1000)
    pub y: f64,
}

/// Set player movement destination
#[utoipa::path(
    post,
    path = "/player/move",
    request_body = MoveRequest,
    responses(
        (status = 200, description = "Movement queued"),
        (status = 400, description = "Coordinates out of bounds")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
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

#[derive(Debug, Serialize, ToSchema)]
pub struct InventoryResponse {
    /// All items owned by the player
    pub items: Vec<InventoryItem>,
}

/// Get player's inventory items
#[utoipa::path(
    get,
    path = "/player/inventory",
    responses(
        (status = 200, description = "Player inventory", body = InventoryResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_inventory(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<InventoryResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Get items with their type names
    let items: Vec<Item> = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE owner_id = $1"
    )
    .bind(player_id)
    .fetch_all(&state.db)
    .await?;

    // Get item type names
    let item_types: Vec<ItemType> = sqlx::query_as(
        "SELECT id, name, category, stackable, weight FROM item_types"
    )
    .fetch_all(&state.db)
    .await?;

    let type_names: std::collections::HashMap<String, String> = item_types
        .into_iter()
        .map(|t| (t.id, t.name))
        .collect();

    let inventory_items: Vec<InventoryItem> = items
        .iter()
        .map(|item| {
            let name = type_names
                .get(&item.item_type)
                .cloned()
                .unwrap_or_else(|| item.item_type.clone());
            InventoryItem::from_item(item, &name)
        })
        .collect();

    Ok(Json(InventoryResponse { items: inventory_items }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CurrencyResponse {
    /// Current Strand balance
    pub strand_balance: i64,
}

/// Get player's currency balance
#[utoipa::path(
    get,
    path = "/player/currency",
    responses(
        (status = 200, description = "Player currency balance", body = CurrencyResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_currency(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<CurrencyResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Get strand balance
    let balance = crate::engine::currency::get_balance(&state.db, player_id)
        .await
        .map_err(|e| AppError::NotFound(e))?;

    Ok(Json(CurrencyResponse { strand_balance: balance }))
}
