use axum::{
    extract::State,
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{quality_to_grade, Container, ContainerType, Item, ItemType, SlotItem};
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

/// Container state for API response
#[derive(Debug, Serialize, ToSchema)]
pub struct ContainerResponse {
    /// Container instance ID
    pub id: Uuid,
    /// Container type identifier
    pub container_type: String,
    /// Total number of slots
    pub slot_count: i32,
    /// Number of columns for UI layout
    pub layout_columns: i32,
    /// Items in each slot
    pub slots: Vec<SlotItem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContainersResponse {
    /// All containers owned by the player
    pub containers: Vec<ContainerResponse>,
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

    // Get item types for names
    let item_types: Vec<ItemType> = sqlx::query_as(
        "SELECT id, name, category, stackable, weight FROM item_types"
    )
    .fetch_all(&state.db)
    .await?;

    let item_type_names: HashMap<String, String> = item_types
        .into_iter()
        .map(|it| (it.id, it.name))
        .collect();

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
        let items: Vec<Item> = sqlx::query_as(
            "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
             FROM items WHERE container_id = $1"
        )
        .bind(container.id)
        .fetch_all(&state.db)
        .await?;

        let slots: Vec<SlotItem> = items
            .into_iter()
            .filter_map(|item| {
                let slot_index = item.slot_index?;
                let item_name = item_type_names
                    .get(&item.item_type)
                    .cloned()
                    .unwrap_or_else(|| item.item_type.clone());

                Some(SlotItem::new(
                    item.id,
                    item.item_type,
                    item_name,
                    item.quality,
                    item.quantity,
                    slot_index,
                ))
            })
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

#[derive(Debug, Deserialize, ToSchema)]
pub struct MoveItemRequest {
    /// ID of the item to move
    pub item_id: Uuid,
    /// Target container ID
    pub target_container_id: Uuid,
    /// Target slot index
    pub target_slot: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MoveItemResponse {
    /// Whether the operation succeeded
    pub success: bool,
    /// Action taken: "moved", "merged", or "swapped"
    pub action: String,
    /// New quantity if items were merged
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_quantity: Option<i32>,
    /// New quality if items were merged
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_quality: Option<i32>,
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

    // Verify the item belongs to player and get its current location
    let source_item: Item = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE id = $1 AND owner_id = $2"
    )
    .bind(req.item_id)
    .bind(player_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Item not found".into()))?;

    // Verify target container belongs to player
    let target_container: Container = sqlx::query_as(
        "SELECT id, container_type, owner_id, station_id, created_at
         FROM containers WHERE id = $1 AND owner_id = $2"
    )
    .bind(req.target_container_id)
    .bind(player_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Container not found".into()))?;

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
    let target_item: Option<Item> = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE container_id = $1 AND slot_index = $2"
    )
    .bind(req.target_container_id)
    .bind(req.target_slot)
    .fetch_optional(&state.db)
    .await?;

    let response = match target_item {
        None => {
            // Target empty: simple move
            sqlx::query(
                "UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3"
            )
            .bind(req.target_container_id)
            .bind(req.target_slot)
            .bind(req.item_id)
            .execute(&state.db)
            .await?;

            MoveItemResponse {
                success: true,
                action: "moved".to_string(),
                merged_quantity: None,
                merged_quality: None,
            }
        }
        Some(target) if target.item_type == source_item.item_type => {
            // Same type: merge stacks
            let new_qty = source_item.quantity + target.quantity;
            let new_quality = (source_item.quality * source_item.quantity
                + target.quality * target.quantity)
                / new_qty;

            // Update target with merged values
            sqlx::query("UPDATE items SET quantity = $1, quality = $2 WHERE id = $3")
                .bind(new_qty)
                .bind(new_quality)
                .bind(target.id)
                .execute(&state.db)
                .await?;

            // Delete source item
            sqlx::query("DELETE FROM items WHERE id = $1")
                .bind(req.item_id)
                .execute(&state.db)
                .await?;

            MoveItemResponse {
                success: true,
                action: "merged".to_string(),
                merged_quantity: Some(new_qty),
                merged_quality: Some(new_quality),
            }
        }
        Some(target) => {
            // Different type: swap
            let source_container_id = source_item.container_id;
            let source_slot = source_item.slot_index;

            // Move source to target's location
            sqlx::query("UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3")
                .bind(req.target_container_id)
                .bind(req.target_slot)
                .bind(req.item_id)
                .execute(&state.db)
                .await?;

            // Move target to source's location
            sqlx::query("UPDATE items SET container_id = $1, slot_index = $2 WHERE id = $3")
                .bind(source_container_id)
                .bind(source_slot)
                .bind(target.id)
                .execute(&state.db)
                .await?;

            MoveItemResponse {
                success: true,
                action: "swapped".to_string(),
                merged_quantity: None,
                merged_quality: None,
            }
        }
    };

    Ok(Json(response))
}
