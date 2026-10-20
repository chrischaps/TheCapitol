use axum::{
    extract::State,
    routing::post,
    Extension, Json, Router,
};
use capitol_protocol::rest::{DepositRequest, DepositResponse, WithdrawRequest, WithdrawResponse};
use capitol_protocol::rules::stacking::Units;
use capitol_protocol::rules::{deposit_quote, withdraw_quote};
use sqlx::PgPool;
use uuid::Uuid;

use crate::engine::units;
use crate::engine::currency::{add_strands_in, announce_balances, subtract_strands_in, TransactionType};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/deposit", post(deposit_fiber))
        .route("/withdraw", post(withdraw_fiber))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
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
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let response = deposit(&state.db, player_id, &req.item_ids).await?;
    tell_balance(&state, player_id, response.new_balance).await;
    Ok(Json(response))
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
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let response = withdraw(&state.db, player_id, req.quantity).await?;
    tell_balance(&state, player_id, response.new_balance).await;
    Ok(Json(response))
}

/// `currency_changed` to the player, so a client needn't refetch it (#148)
async fn tell_balance(state: &AppState, player_id: Uuid, new_balance: i64) {
    let events = state.game.read().await.event_sender.clone();
    announce_balances(&events, [(player_id, new_balance)]);
}

async fn player_of(db: &PgPool, account_id: Uuid) -> Result<Uuid, AppError> {
    Ok(sqlx::query_scalar("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account_id)
        .fetch_one(db)
        .await?)
}

/// Swap the fiber stacks `item_ids` for Strands, less the fee. The items go and
/// the Strands arrive in one transaction, so neither happens without the other.
pub async fn deposit(db: &PgPool, player_id: Uuid, item_ids: &[Uuid]) -> Result<DepositResponse, AppError> {
    if item_ids.is_empty() {
        return Err(AppError::BadRequest("No items specified".into()));
    }

    let mut tx = db.begin().await?;

    // Locked, so a second deposit of the same stacks waits, then finds them gone
    let items: Vec<(Uuid, String, i32)> = sqlx::query_as(
        "SELECT id, item_type, quantity FROM items
         WHERE id = ANY($1) AND owner_id = $2
         FOR UPDATE"
    )
    .bind(item_ids)
    .bind(player_id)
    .fetch_all(&mut *tx)
    .await?;

    if items.len() != item_ids.len() {
        return Err(AppError::BadRequest("Some items not found or not owned by you".into()));
    }

    let mut total_fiber: i64 = 0;
    for (_, item_type, quantity) in &items {
        // Only accept fiber items
        if item_type != "fiber" {
            return Err(AppError::BadRequest(format!("Item type '{}' cannot be deposited, only fiber", item_type)));
        }
        total_fiber += *quantity as i64;
    }

    if total_fiber <= 0 {
        return Err(AppError::BadRequest("No fiber to deposit".into()));
    }
    let deposited_quantity = i32::try_from(total_fiber)
        .map_err(|_| AppError::BadRequest("Too much fiber for one deposit".into()))?;

    // Strands after the 0.5% fee (the rule clients preview with)
    let (fee, net_strands) = deposit_quote(total_fiber);

    if net_strands <= 0 {
        return Err(AppError::BadRequest("Deposit amount too small after fee".into()));
    }

    sqlx::query("DELETE FROM items WHERE id = ANY($1)")
        .bind(item_ids)
        .execute(&mut *tx)
        .await?;

    let new_balance = add_strands_in(
        &mut tx,
        player_id,
        net_strands,
        TransactionType::ExchangeDeposit,
        Some(&format!("Deposited {} fiber", total_fiber)),
        None,
    )
    .await
    .map_err(AppError::Internal)?;

    tx.commit().await?;

    tracing::info!(
        "Player {} deposited {} fiber for {} strands (fee: {})",
        player_id, total_fiber, net_strands, fee
    );

    Ok(DepositResponse {
        deposited_quantity,
        fee,
        strands_received: net_strands,
        new_balance,
    })
}

/// The quality of fiber withdrawn from the exchange
const WITHDRAWN_FIBER_QUALITY: i32 = 50;

/// Swap Strands for `quantity` quality-50 fiber, the fee on top. The fiber
/// joins the inventory's lowest-slot fiber stack, else takes its first empty
/// slot. The debit and the fiber are one transaction, so with nowhere to put
/// the fiber the Strands stay.
pub async fn withdraw(db: &PgPool, player_id: Uuid, quantity: i32) -> Result<WithdrawResponse, AppError> {
    if quantity <= 0 {
        return Err(AppError::BadRequest("Quantity must be positive".into()));
    }

    // Calculate strands needed (fiber / (1 - fee_rate), rounded up)
    // If you want 100 fiber, you pay 100 / 0.995 = ~100.5 strands
    let (strands_needed, fee) = withdraw_quote(quantity as i64);

    let mut tx = db.begin().await?;

    // First, because it locks the player row: two withdrawals by one player
    // take turns, so they can't both pick the same empty slot.
    let new_balance = subtract_strands_in(
        &mut tx,
        player_id,
        strands_needed,
        TransactionType::ExchangeWithdraw,
        Some(&format!("Withdrew {} fiber", quantity)),
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

    let (container_id, slot_count): (Uuid, i32) = sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.owner_id = $1 AND c.container_type = 'player_inventory' LIMIT 1"
    )
    .bind(player_id)
    .fetch_one(&mut *tx)
    .await?;

    // Onto the lowest-slot fiber stack, so a client can tell which one it will
    // be. Its units keep their quality (plan D2, E1.8), so it may be any grade.
    // The Exchange's fiber carries no maker's mark (#231)
    let withdrawn = units::Marked::by(None, &Units::uniform(quantity, WITHDRAWN_FIBER_QUALITY));
    if !units::join_first_stack(&mut tx, container_id, "fiber", &withdrawn).await? {
        let used: std::collections::HashSet<i32> = sqlx::query_scalar(
            "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL"
        )
        .bind(container_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();

        // Returning drops `tx`, which rolls the debit back
        let Some(slot_index) = (0..slot_count).find(|s| !used.contains(s)) else {
            return Err(AppError::BadRequest("Inventory full".into()));
        };

        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'fiber', $6, $2, $3, $4, $5)"
        )
        .bind(Uuid::new_v4())
        .bind(quantity)
        .bind(player_id)
        .bind(container_id)
        .bind(slot_index)
        .bind(WITHDRAWN_FIBER_QUALITY)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    tracing::info!(
        "Player {} withdrew {} fiber for {} strands (fee: {})",
        player_id, quantity, strands_needed, fee
    );

    Ok(WithdrawResponse {
        strands_spent: strands_needed,
        fee,
        fiber_received: quantity,
        new_balance,
    })
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh
/// database that `sqlx::test` makes from the migrations. Run them with
/// `cargo test routes::exchange -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{balance, ledger, player};

    async fn inventory(db: &PgPool, owner: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(id)
            .bind(owner)
            .execute(db)
            .await
            .unwrap();
        id
    }

    async fn slot_count(db: &PgPool) -> i32 {
        sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'player_inventory'")
            .fetch_one(db)
            .await
            .unwrap()
    }

    async fn stack(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, item_type: &str, quality: i32, qty: i32) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(item_type)
        .bind(quality)
        .bind(qty)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
        id
    }

    /// Fill every slot of `container` with rope
    async fn fill(db: &PgPool, owner: Uuid, container: Uuid) {
        for slot in 0..slot_count(db).await {
            stack(db, owner, container, slot, "rope", 50, 1).await;
        }
    }

    /// (quantity, quality) of an item, or `None` if it's gone
    async fn item(db: &PgPool, id: Uuid) -> Option<(i32, i32)> {
        sqlx::query_as("SELECT quantity, quality FROM items WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await
            .unwrap()
    }

    async fn fiber_in(db: &PgPool, container: Uuid) -> Vec<(i32, i32, i32)> {
        sqlx::query_as(
            "SELECT slot_index, quantity, quality FROM items
             WHERE container_id = $1 AND item_type = 'fiber' ORDER BY slot_index",
        )
        .bind(container)
        .fetch_all(db)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_withdrawal_into_a_full_inventory_keeps_the_strands(db: PgPool) {
        let ada = player(&db, "ada", 1000).await;
        let inv = inventory(&db, ada).await;
        fill(&db, ada, inv).await;

        let err = withdraw(&db, ada, 100).await.unwrap_err();

        assert!(matches!(&err, AppError::BadRequest(m) if m == "Inventory full"), "{err:?}");
        assert_eq!(balance(&db, ada).await, 1000);
        assert_eq!(ledger(&db, ada).await, vec![], "no debit was recorded");
        assert_eq!(fiber_in(&db, inv).await, vec![]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_withdrawal_takes_the_first_empty_slot(db: PgPool) {
        let ada = player(&db, "ada", 1000).await;
        let inv = inventory(&db, ada).await;
        stack(&db, ada, inv, 0, "rope", 50, 1).await;

        let done = withdraw(&db, ada, 100).await.unwrap();

        let (strands, fee) = withdraw_quote(100);
        assert_eq!((done.strands_spent, done.fee, done.fiber_received), (strands, fee, 100));
        assert_eq!(done.new_balance, 1000 - strands);
        assert_eq!(balance(&db, ada).await, 1000 - strands);
        assert_eq!(fiber_in(&db, inv).await, vec![(1, 100, 50)]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_withdrawal_joins_the_lowest_slot_fiber_stack(db: PgPool) {
        let ada = player(&db, "ada", 1000).await;
        let inv = inventory(&db, ada).await;
        // Inserted high slot first, so insertion order can't be what picks.
        // Both are D, as withdrawn fiber is.
        let high = stack(&db, ada, inv, 7, "fiber", 52, 10).await;
        let low = stack(&db, ada, inv, 2, "fiber", 52, 10).await;

        withdraw(&db, ada, 10).await.unwrap();

        assert_eq!(item(&db, low).await, Some((20, 51)));
        assert_eq!(item(&db, high).await, Some((10, 52)));
    }

    /// Withdrawn fiber is a D, and joins an A stack as ten D units: the A
    /// units stay A (D2, E1.8)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_withdrawal_joins_any_grade_and_keeps_its_quality(db: PgPool) {
        use crate::engine::units::test_units::units;
        let ada = player(&db, "ada", 1000).await;
        let inv = inventory(&db, ada).await;
        let a = stack(&db, ada, inv, 0, "fiber", 95, 10).await;

        withdraw(&db, ada, 10).await.unwrap();

        assert_eq!(units(&db, a).await, Units::from_pairs([(WITHDRAWN_FIBER_QUALITY, 10), (95, 10)]));
        assert_eq!(item(&db, a).await, Some((20, 72)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_withdrawal_that_would_overflow_the_stack_takes_a_slot_of_its_own(db: PgPool) {
        let ada = player(&db, "ada", i64::MAX / 2).await;
        let inv = inventory(&db, ada).await;
        let full = stack(&db, ada, inv, 0, "fiber", 50, i32::MAX - 5).await;

        let done = withdraw(&db, ada, 10).await.unwrap();

        assert_eq!(item(&db, full).await, Some((i32::MAX - 5, 50)));
        assert_eq!(fiber_in(&db, inv).await, vec![(0, i32::MAX - 5, 50), (1, 10, 50)]);
        assert_eq!(balance(&db, ada).await, i64::MAX / 2 - done.strands_spent);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_withdrawal_short_of_strands_gives_nothing(db: PgPool) {
        let ada = player(&db, "ada", 50).await;
        let inv = inventory(&db, ada).await;

        let err = withdraw(&db, ada, 100).await.unwrap_err();

        assert!(matches!(&err, AppError::BadRequest(m) if m.contains("Insufficient")), "{err:?}");
        assert_eq!(balance(&db, ada).await, 50);
        assert_eq!(fiber_in(&db, inv).await, vec![]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_deposit_swaps_the_stacks_for_strands(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let inv = inventory(&db, ada).await;
        let a = stack(&db, ada, inv, 0, "fiber", 60, 150).await;
        let b = stack(&db, ada, inv, 1, "fiber", 40, 50).await;

        let done = deposit(&db, ada, &[a, b]).await.unwrap();

        let (fee, net) = deposit_quote(200);
        assert_eq!((done.deposited_quantity, done.fee, done.strands_received), (200, fee, net));
        assert_eq!(balance(&db, ada).await, net);
        assert_eq!(fiber_in(&db, inv).await, vec![]);
        assert_eq!(ledger(&db, ada).await, vec![(net, "exchange_deposit".to_string())]);
    }

    /// The items and the Strands are one transaction, so a credit that fails
    /// (here, a balance that would overflow) leaves the fiber where it was
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_deposit_whose_credit_fails_keeps_the_fiber(db: PgPool) {
        let ada = player(&db, "ada", i64::MAX - 10).await;
        let inv = inventory(&db, ada).await;
        let a = stack(&db, ada, inv, 0, "fiber", 60, 1000).await;

        let err = deposit(&db, ada, &[a]).await.unwrap_err();

        assert!(matches!(err, AppError::Internal(_)), "{err:?}");
        assert_eq!(item(&db, a).await, Some((1000, 60)));
        assert_eq!(balance(&db, ada).await, i64::MAX - 10);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_deposit_with_one_non_fiber_stack_takes_nothing(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let inv = inventory(&db, ada).await;
        let fiber = stack(&db, ada, inv, 0, "fiber", 60, 100).await;
        let rope = stack(&db, ada, inv, 1, "rope", 60, 1).await;

        let err = deposit(&db, ada, &[fiber, rope]).await.unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
        assert_eq!(item(&db, fiber).await, Some((100, 60)));
        assert_eq!(balance(&db, ada).await, 0);
    }
}
