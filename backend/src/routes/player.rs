use axum::{
    extract::State,
    routing::{get, post},
    Extension, Json, Router,
};
use uuid::Uuid;

use capitol_protocol::rest::{
    CurrencyResponse, Destination, InventoryResponse, MoveRequest, MoveResponse,
    PlayerMeResponse,
};
use capitol_protocol::rules::{Zone, WORLD_SIZE};

use crate::engine::Command;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{InventoryItem, Item, ItemType};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/me", get(get_me))
        .route("/move", post(move_player))
        .route("/inventory", get(get_inventory))
        .route("/currency", get(get_currency))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Get the player's name, live position, zone and balance
///
/// While the player is in the game, the name and position come from the tick
/// loop's state, not the `players` row, which is only written every 10 s. The
/// balance always comes from the row: trades, the exchange and property tax
/// write it there, and the in-memory copy is never updated after login.
#[utoipa::path(
    get,
    path = "/player/me",
    responses(
        (status = 200, description = "The player, as the server sees them now", body = PlayerMeResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_me(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<PlayerMeResponse>, AppError> {
    let (player_id, name, x, y, strand_balance) =
        sqlx::query_as::<_, (Uuid, String, f64, f64, i64)>(
            "SELECT id, name, position_x, position_y, strand_balance
             FROM players WHERE account_id = $1 LIMIT 1"
        )
        .bind(auth_user.account_id)
        .fetch_one(&state.db)
        .await?;

    let live = {
        let game = state.game.read().await;
        game.players.get(&player_id).map(|p| (p.name.clone(), p.x, p.y))
    };
    let online = live.is_some();
    let (name, x, y) = live.unwrap_or((name, x, y));

    Ok(Json(PlayerMeResponse {
        player_id,
        name,
        x,
        y,
        zone: Zone::from_position(x, y).as_str().to_string(),
        strand_balance,
        online,
    }))
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
) -> Result<Json<MoveResponse>, AppError> {
    // Validate coordinates
    if !(0.0..=WORLD_SIZE).contains(&req.x) || !(0.0..=WORLD_SIZE).contains(&req.y) {
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

    Ok(Json(MoveResponse {
        success: true,
        destination: Destination { x: req.x, y: req.y },
    }))
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
            item.to_inventory_item(&name)
        })
        .collect();

    Ok(Json(InventoryResponse { items: inventory_items }))
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
        .map_err(AppError::NotFound)?;

    Ok(Json(CurrencyResponse { strand_balance: balance }))
}
