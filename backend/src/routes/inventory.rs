use axum::{
    extract::{Path, State},
    routing::{get, post},
    Extension, Json, Router,
};
use std::collections::HashMap;
use uuid::Uuid;

use capitol_protocol::rest::{
    ContainerResponse, ContainersResponse, MoveItemRequest, MoveItemResponse, SplitStackRequest,
    SplitStackResponse,
};

use crate::engine::container_access::{check_container_access, live_position, movable_item};
use crate::engine::slot_moves::{self, apply_move, SlotMove};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{Container, ContainerType, Item, ItemType, SlotItem, ITEM_COLUMNS, ITEM_TYPE_COLUMNS};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/containers", get(get_containers))
        .route("/move", post(move_item))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// `/items`: what can be done to one stack.
pub fn items_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{id}/split", post(split_stack))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Get all player containers with their contents
#[utoipa::path(
    get,
    path = "/inventory/containers",
    responses(
        (status = 200, description = "Player containers", body = ContainersResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Inventory"
)]
pub async fn get_containers(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<ContainersResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Get container types (for slot_count and layout)
    let container_types: Vec<ContainerType> = sqlx::query_as(
        "SELECT id, name, slot_count, layout_columns FROM container_types"
    )
    .fetch_all(&state.db)
    .await?;

    let type_map: HashMap<String, ContainerType> = container_types
        .into_iter()
        .map(|ct| (ct.id.clone(), ct))
        .collect();

    // Get player's containers
    let containers: Vec<Container> = sqlx::query_as(
        "SELECT id, container_type, owner_id, station_id, created_at
         FROM containers WHERE owner_id = $1"
    )
    .bind(player_id)
    .fetch_all(&state.db)
    .await?;

    // Item types, for names and tools' durability
    let item_types: Vec<ItemType> = sqlx::query_as(&format!("SELECT {ITEM_TYPE_COLUMNS} FROM item_types"))
        .fetch_all(&state.db)
        .await?;
    let item_types: HashMap<String, ItemType> =
        item_types.into_iter().map(|it| (it.id.clone(), it)).collect();

    // Build response for each container
    let mut response_containers = Vec::new();

    for container in containers {
        let ct = type_map.get(&container.container_type).ok_or_else(|| {
            AppError::Internal(format!(
                "Unknown container type: {}",
                container.container_type
            ))
        })?;

        // Get items in this container
        let items: Vec<Item> = sqlx::query_as(&format!("SELECT {ITEM_COLUMNS} FROM items WHERE container_id = $1"))
            .bind(container.id)
            .fetch_all(&state.db)
            .await?;

        let slots: Vec<SlotItem> = items
            .into_iter()
            .filter_map(|item| item.to_slot_item(item_types.get(&item.item_type)))
            .collect();

        response_containers.push(ContainerResponse {
            id: container.id,
            container_type: container.container_type,
            slot_count: ct.slot_count,
            layout_columns: ct.layout_columns,
            slots,
        });
    }

    Ok(Json(ContainersResponse {
        containers: response_containers,
    }))
}

/// Move an item to a different slot or container
#[utoipa::path(
    post,
    path = "/inventory/move",
    request_body = MoveItemRequest,
    responses(
        (status = 200, description = "Item moved/merged/swapped", body = MoveItemResponse),
        (status = 400, description = "Invalid slot or container"),
        (status = 404, description = "Item or container not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Inventory"
)]
pub async fn move_item(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<MoveItemRequest>,
) -> Result<Json<MoveItemResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // The player's own item, or a deposit in their own station's box (#80)
    let source_item: Item = movable_item(&state.db, player_id, req.item_id).await?;

    // Both ends must be usable: the player's own containers, or a station's
    // within reach (B4). The source matters too, since a swap fills it.
    let at = live_position(&state.game, player_id).await;
    if let Some(source_container_id) = source_item.container_id {
        check_container_access(&state.db, player_id, source_container_id, at).await?;
    }
    let target_container =
        check_container_access(&state.db, player_id, req.target_container_id, at).await?;

    // Get container type for slot validation
    let container_type: ContainerType = sqlx::query_as(
        "SELECT id, name, slot_count, layout_columns FROM container_types WHERE id = $1"
    )
    .bind(&target_container.container_type)
    .fetch_one(&state.db)
    .await?;

    // Validate slot bounds
    if req.target_slot < 0 || req.target_slot >= container_type.slot_count {
        return Err(AppError::BadRequest(format!(
            "Slot {} out of bounds (0-{})",
            req.target_slot,
            container_type.slot_count - 1
        )));
    }

    // Check if target slot is occupied
    let target_item: Option<Item> = sqlx::query_as(&format!("SELECT {ITEM_COLUMNS} FROM items WHERE container_id = $1 AND slot_index = $2"))
        .bind(req.target_container_id)
        .bind(req.target_slot)
        .fetch_optional(&state.db)
        .await?;

    // A shared station can hold other players' items. Merging into or swapping
    // with one would hand it over, so only empty slots or your own items.
    if target_item.as_ref().is_some_and(|t| t.owner_id != Some(player_id)) {
        return Err(AppError::Forbidden("That slot holds someone else's item".into()));
    }

    let done = apply_move(
        &state.db,
        player_id,
        &source_item,
        req.target_container_id,
        req.target_slot,
        target_item.as_ref(),
    )
    .await?;
    let (action, merged_quantity, merged_quality) = match done {
        SlotMove::Moved => ("moved", None, None),
        SlotMove::Merged { units } => ("merged", Some(units.count() as i32), units.mean()),
        SlotMove::Swapped => ("swapped", None, None),
    };
    let response = MoveItemResponse {
        success: true,
        action: action.to_string(),
        merged_quantity,
        merged_quality,
    };

    Ok(Json(response))
}

/// Split part of a stack into an empty slot of its own container, or of
/// `to_container_id` (E1.2, #190). Both are checked as a move's are.
/// `qualities` names which units, as `[quality, count]` pairs adding up to
/// `quantity`; without it the lowest go. Every unit keeps its quality (E1.8).
#[utoipa::path(
    post,
    path = "/items/{id}/split",
    params(("id" = Uuid, Path, description = "The stack to split")),
    request_body = SplitStackRequest,
    responses(
        (status = 200, description = "The new stack", body = SplitStackResponse),
        (status = 400, description = "Bad quantity, qualities the stack doesn't hold, or the slot is taken or doesn't exist"),
        (status = 404, description = "Item not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Inventory"
)]
pub async fn split_stack(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<SplitStackRequest>,
) -> Result<Json<SplitStackResponse>, AppError> {
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    let source = movable_item(&state.db, player_id, item_id).await?;
    let container_id = source
        .container_id
        .ok_or_else(|| AppError::BadRequest("That item isn't in a container".into()))?;
    let at = live_position(&state.game, player_id).await;
    check_container_access(&state.db, player_id, container_id, at).await?;
    let to_container = req.to_container_id.unwrap_or(container_id);
    if to_container != container_id {
        check_container_access(&state.db, player_id, to_container, at).await?;
    }

    let new = slot_moves::split(
        &state.db,
        player_id,
        &source,
        req.quantity,
        to_container,
        req.to_slot,
        req.qualities.as_ref(),
    )
        .await?
        .map_err(|e| AppError::BadRequest(e.message().into()))?;
    let item_type: Option<ItemType> =
        sqlx::query_as(&format!("SELECT {ITEM_TYPE_COLUMNS} FROM item_types WHERE id = $1"))
            .bind(&new.item_type)
            .fetch_optional(&state.db)
            .await?;
    let item = new
        .to_slot_item(item_type.as_ref())
        .ok_or_else(|| AppError::Internal("A split stack has no slot".into()))?;
    Ok(Json(SplitStackResponse { item, container_id: to_container }))
}
