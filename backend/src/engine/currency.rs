use sqlx::PgPool;
use uuid::Uuid;

/// Transaction types for currency audit log
pub enum TransactionType {
    ExchangeDeposit,
    ExchangeWithdraw,
    TradeReceived,
    TradeSent,
    AdminAdjustment,
}

impl TransactionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransactionType::ExchangeDeposit => "exchange_deposit",
            TransactionType::ExchangeWithdraw => "exchange_withdraw",
            TransactionType::TradeReceived => "trade_received",
            TransactionType::TradeSent => "trade_sent",
            TransactionType::AdminAdjustment => "admin_adjustment",
        }
    }
}

/// Add strands to a player's balance with audit logging.
/// Uses SELECT FOR UPDATE to ensure atomic operations.
/// Returns the new balance.
pub async fn add_strands(
    pool: &PgPool,
    player_id: Uuid,
    amount: i64,
    tx_type: TransactionType,
    description: Option<&str>,
    reference_id: Option<Uuid>,
) -> Result<i64, String> {
    if amount <= 0 {
        return Err("Amount must be positive".to_string());
    }

    // Start transaction
    let mut tx = pool.begin().await.map_err(|e| format!("Failed to start transaction: {}", e))?;

    // Lock row and get current balance
    let current_balance: (i64,) = sqlx::query_as(
        "SELECT strand_balance FROM players WHERE id = $1 FOR UPDATE"
    )
    .bind(player_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| format!("Player not found: {}", e))?;

    let balance_before = current_balance.0;
    let balance_after = balance_before + amount;

    // Update balance
    sqlx::query("UPDATE players SET strand_balance = $1 WHERE id = $2")
        .bind(balance_after)
        .bind(player_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to update balance: {}", e))?;

    // Insert audit record
    sqlx::query(
        "INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, reference_id, description)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(player_id)
    .bind(amount)
    .bind(balance_before)
    .bind(balance_after)
    .bind(tx_type.as_str())
    .bind(reference_id)
    .bind(description)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Failed to insert audit record: {}", e))?;

    // Commit transaction
    tx.commit().await.map_err(|e| format!("Failed to commit: {}", e))?;

    tracing::info!(
        "Added {} strands to player {} ({}): {} -> {}",
        amount, player_id, tx_type.as_str(), balance_before, balance_after
    );

    Ok(balance_after)
}

/// Subtract strands from a player's balance with audit logging.
/// Uses SELECT FOR UPDATE to ensure atomic operations.
/// Returns the new balance, or error if insufficient funds.
pub async fn subtract_strands(
    pool: &PgPool,
    player_id: Uuid,
    amount: i64,
    tx_type: TransactionType,
    description: Option<&str>,
    reference_id: Option<Uuid>,
) -> Result<i64, String> {
    if amount <= 0 {
        return Err("Amount must be positive".to_string());
    }

    // Start transaction
    let mut tx = pool.begin().await.map_err(|e| format!("Failed to start transaction: {}", e))?;

    // Lock row and get current balance
    let current_balance: (i64,) = sqlx::query_as(
        "SELECT strand_balance FROM players WHERE id = $1 FOR UPDATE"
    )
    .bind(player_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| format!("Player not found: {}", e))?;

    let balance_before = current_balance.0;

    if balance_before < amount {
        return Err(format!("Insufficient strands: have {}, need {}", balance_before, amount));
    }

    let balance_after = balance_before - amount;

    // Update balance
    sqlx::query("UPDATE players SET strand_balance = $1 WHERE id = $2")
        .bind(balance_after)
        .bind(player_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to update balance: {}", e))?;

    // Insert audit record (negative amount for subtraction)
    sqlx::query(
        "INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, reference_id, description)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(player_id)
    .bind(-amount)
    .bind(balance_before)
    .bind(balance_after)
    .bind(tx_type.as_str())
    .bind(reference_id)
    .bind(description)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Failed to insert audit record: {}", e))?;

    // Commit transaction
    tx.commit().await.map_err(|e| format!("Failed to commit: {}", e))?;

    tracing::info!(
        "Subtracted {} strands from player {} ({}): {} -> {}",
        amount, player_id, tx_type.as_str(), balance_before, balance_after
    );

    Ok(balance_after)
}

/// Transfer strands from one player to another atomically.
/// Both players are locked in consistent order to prevent deadlocks.
pub async fn transfer_strands(
    pool: &PgPool,
    from_id: Uuid,
    to_id: Uuid,
    amount: i64,
    description: Option<&str>,
    reference_id: Option<Uuid>,
) -> Result<(i64, i64), String> {
    if amount <= 0 {
        return Err("Amount must be positive".to_string());
    }

    if from_id == to_id {
        return Err("Cannot transfer to self".to_string());
    }

    // Start transaction
    let mut tx = pool.begin().await.map_err(|e| format!("Failed to start transaction: {}", e))?;

    // Lock both rows in consistent order (by UUID) to prevent deadlocks
    let (first_id, second_id) = if from_id < to_id {
        (from_id, to_id)
    } else {
        (to_id, from_id)
    };

    // Lock first player
    let first_balance: (i64,) = sqlx::query_as(
        "SELECT strand_balance FROM players WHERE id = $1 FOR UPDATE"
    )
    .bind(first_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| format!("First player not found: {}", e))?;

    // Lock second player
    let second_balance: (i64,) = sqlx::query_as(
        "SELECT strand_balance FROM players WHERE id = $1 FOR UPDATE"
    )
    .bind(second_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| format!("Second player not found: {}", e))?;

    // Determine actual balances based on who is from/to
    let (from_balance_before, to_balance_before) = if from_id < to_id {
        (first_balance.0, second_balance.0)
    } else {
        (second_balance.0, first_balance.0)
    };

    // Check sender has sufficient funds
    if from_balance_before < amount {
        return Err(format!("Insufficient strands: have {}, need {}", from_balance_before, amount));
    }

    let from_balance_after = from_balance_before - amount;
    let to_balance_after = to_balance_before + amount;

    // Update sender
    sqlx::query("UPDATE players SET strand_balance = $1 WHERE id = $2")
        .bind(from_balance_after)
        .bind(from_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to update sender balance: {}", e))?;

    // Update recipient
    sqlx::query("UPDATE players SET strand_balance = $1 WHERE id = $2")
        .bind(to_balance_after)
        .bind(to_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to update recipient balance: {}", e))?;

    // Audit record for sender (negative)
    sqlx::query(
        "INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, reference_id, description)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(from_id)
    .bind(-amount)
    .bind(from_balance_before)
    .bind(from_balance_after)
    .bind(TransactionType::TradeSent.as_str())
    .bind(reference_id)
    .bind(description)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Failed to insert sender audit: {}", e))?;

    // Audit record for recipient (positive)
    sqlx::query(
        "INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, reference_id, description)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(to_id)
    .bind(amount)
    .bind(to_balance_before)
    .bind(to_balance_after)
    .bind(TransactionType::TradeReceived.as_str())
    .bind(reference_id)
    .bind(description)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Failed to insert recipient audit: {}", e))?;

    // Commit
    tx.commit().await.map_err(|e| format!("Failed to commit: {}", e))?;

    tracing::info!(
        "Transferred {} strands from {} to {}: {} -> {}, {} -> {}",
        amount, from_id, to_id,
        from_balance_before, from_balance_after,
        to_balance_before, to_balance_after
    );

    Ok((from_balance_after, to_balance_after))
}

/// Get player's current strand balance
pub async fn get_balance(pool: &PgPool, player_id: Uuid) -> Result<i64, String> {
    let balance: (i64,) = sqlx::query_as(
        "SELECT strand_balance FROM players WHERE id = $1"
    )
    .bind(player_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Player not found: {}", e))?;

    Ok(balance.0)
}
