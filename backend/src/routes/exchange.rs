use axum::{
    extract::State,
    routing::post,
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::engine::currency::{add_strands, subtract_strands, TransactionType};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

/// Exchange fee rate (0.5% = 0.005)
const EXCHANGE_FEE_RATE: f64 = 0.005;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/deposit", post(deposit_fiber))
        .route("/withdraw", post(withdraw_fiber))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DepositRequest {
    /// Item IDs to deposit (must be fiber items)
    pub item_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DepositResponse {
    /// Total fiber quantity deposited
    pub deposited_quantity: i32,
    /// Exchange fee (0.5%)
    pub fee: i64,
    /// Net Strands received after fee
    pub strands_received: i64,
    /// New Strand balance
    pub new_balance: i64,
}

/// Deposit fiber items to receive Strands
#[utoipa::path(
    post,
    path = "/exchange/deposit",
    request_body = DepositRequest,
    responses(
        (status = 200, description = "Deposit successful", body = DepositResponse),
        (status = 400, description = "Invalid items or not fiber")
    ),
    security(("bearer_auth" = [])),
    tag = "Exchange"
)]
pub async fn deposit_fiber(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<DepositRequest>,
) -> Result<Json<DepositResponse>, AppError> {
    if req.item_ids.is_empty() {
        return Err(AppError::BadRequest("No items specified".into()));
    }

    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Get the items and verify they are fiber type and owned by player
    let items: Vec<(Uuid, String, i32)> = sqlx::query_as(
        "SELECT id, item_type, quantity FROM items
         WHERE id = ANY($1) AND owner_id = $2"
    )
    .bind(&req.item_ids)
    .bind(player_id)
    .fetch_all(&state.db)
    .await?;

    // Verify all requested items were found
    if items.len() != req.item_ids.len() {
        return Err(AppError::BadRequest("Some items not found or not owned by you".into()));
    }

    // Calculate total fiber quantity
    let mut total_fiber: i32 = 0;
    for (_, item_type, quantity) in &items {
        // Only accept fiber items
        if item_type != "fiber" {
            return Err(AppError::BadRequest(format!("Item type '{}' cannot be deposited, only fiber", item_type)));
        }
        total_fiber += quantity;
    }

    if total_fiber <= 0 {
        return Err(AppError::BadRequest("No fiber to deposit".into()));
    }

    // Calculate strands after fee (0.5% fee)
    let gross_strands = total_fiber as i64;
    let fee = ((gross_strands as f64) * EXCHANGE_FEE_RATE).floor() as i64;
    let net_strands = gross_strands - fee;

    if net_strands <= 0 {
        return Err(AppError::BadRequest("Deposit amount too small after fee".into()));
    }

    // Start transaction
    let mut tx = state.db.begin().await?;

    // Delete the fiber items
    sqlx::query("DELETE FROM items WHERE id = ANY($1)")
        .bind(&req.item_ids)
        .execute(&mut *tx)
        .await?;

    // Commit item deletion
    tx.commit().await?;

    // Add strands to player balance (this has its own transaction)
    let new_balance = add_strands(
        &state.db,
        player_id,
        net_strands,
        TransactionType::ExchangeDeposit,
        Some(&format!("Deposited {} fiber", total_fiber)),
        None,
    )
    .await
    .map_err(|e| AppError::Internal(e))?;

    tracing::info!(
        "Player {} deposited {} fiber for {} strands (fee: {})",
        player_id, total_fiber, net_strands, fee
    );

    Ok(Json(DepositResponse {
        deposited_quantity: total_fiber,
        fee,
        strands_received: net_strands,
        new_balance,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WithdrawRequest {
    /// Amount of fiber to withdraw
    pub quantity: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WithdrawResponse {
    /// Total Strands spent (including fee)
    pub strands_spent: i64,
    /// Exchange fee (0.5%)
    pub fee: i64,
    /// Fiber items received
    pub fiber_received: i32,
    /// New Strand balance
    pub new_balance: i64,
}

/// Withdraw Strands to receive fiber items
#[utoipa::path(
    post,
    path = "/exchange/withdraw",
    request_body = WithdrawRequest,
    responses(
        (status = 200, description = "Withdrawal successful", body = WithdrawResponse),
        (status = 400, description = "Insufficient balance or inventory full")
    ),
    security(("bearer_auth" = [])),
    tag = "Exchange"
)]
pub async fn withdraw_fiber(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<WithdrawRequest>,
) -> Result<Json<WithdrawResponse>, AppError> {
    if req.quantity <= 0 {
        return Err(AppError::BadRequest("Quantity must be positive".into()));
    }

    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Calculate strands needed (fiber / (1 - fee_rate), rounded up)
    // If you want 100 fiber, you pay 100 / 0.995 = ~100.5 strands
    let strands_needed = ((req.quantity as f64) / (1.0 - EXCHANGE_FEE_RATE)).ceil() as i64;
    let fee = strands_needed - (req.quantity as i64);

    // Subtract strands from player balance (validates sufficient funds)
    let new_balance = subtract_strands(
        &state.db,
        player_id,
        strands_needed,
        TransactionType::ExchangeWithdraw,
        Some(&format!("Withdrew {} fiber", req.quantity)),
        None,
    )
    .await
    .map_err(|e| {
        if e.contains("Insufficient") {
            AppError::BadRequest(e)
        } else {
            AppError::Internal(e)
        }
    })?;

    // Get player's inventory container
    let container_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'player_inventory' LIMIT 1"
    )
    .bind(player_id)
    .fetch_one(&state.db)
    .await?;

    // Get container slot count
    let slot_count: i32 = sqlx::query_scalar(
        "SELECT ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.id = $1"
    )
    .bind(container_id)
    .fetch_one(&state.db)
    .await?;

    // Check if there's an existing fiber stack to merge with
    let existing_fiber: Option<(Uuid, i32, i32)> = sqlx::query_as(
        "SELECT id, quantity, quality FROM items
         WHERE container_id = $1 AND item_type = 'fiber' LIMIT 1"
    )
    .bind(container_id)
    .fetch_optional(&state.db)
    .await?;

    if let Some((existing_id, existing_qty, existing_quality)) = existing_fiber {
        // Merge with existing stack
        let new_qty = existing_qty + req.quantity;
        // Quality stays the same since we're adding standard quality (50)
        let new_quality = (existing_quality * existing_qty + 50 * req.quantity) / new_qty;

        sqlx::query("UPDATE items SET quantity = $1, quality = $2 WHERE id = $3")
            .bind(new_qty)
            .bind(new_quality)
            .bind(existing_id)
            .execute(&state.db)
            .await?;
    } else {
        // Find empty slot
        let used_slots: Vec<(i32,)> = sqlx::query_as(
            "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL"
        )
        .bind(container_id)
        .fetch_all(&state.db)
        .await?;

        let used: std::collections::HashSet<i32> = used_slots.into_iter().map(|(s,)| s).collect();
        let mut empty_slot: Option<i32> = None;
        for slot in 0..slot_count {
            if !used.contains(&slot) {
                empty_slot = Some(slot);
                break;
            }
        }

        let slot_index = match empty_slot {
            Some(s) => s,
            None => return Err(AppError::BadRequest("Inventory full".into())),
        };

        // Create new fiber item with standard quality (50)
        let item_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'fiber', 50, $2, $3, $4, $5)"
        )
        .bind(item_id)
        .bind(req.quantity)
        .bind(player_id)
        .bind(container_id)
        .bind(slot_index)
        .execute(&state.db)
        .await?;
    }

    tracing::info!(
        "Player {} withdrew {} fiber for {} strands (fee: {})",
        player_id, req.quantity, strands_needed, fee
    );

    Ok(Json(WithdrawResponse {
        strands_spent: strands_needed,
        fee,
        fiber_received: req.quantity,
        new_balance,
    }))
}
