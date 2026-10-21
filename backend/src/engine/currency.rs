use sqlx::{PgConnection, PgPool};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::events::GameEvent;

/// Transaction types for currency audit log
#[allow(dead_code)] // the ledger API; the market's escrow and fills (#167, #168) post through it
pub enum TransactionType {
    ExchangeDeposit,
    ExchangeWithdraw,
    TradeReceived,
    TradeSent,
    AdminAdjustment,
    DeedPurchase,
    DeedRefund,
    /// The deed, less the tax owed, back to a foreclosed plot's owner (#131)
    ForeclosureRefund,
    /// Property tax owed, paid off by the owner (#138)
    TaxPayment,
    /// The hourly property tax, and debt it takes back when the owner has
    /// Strands again (C1.7: before, these wrote no row)
    PropertyTax,
    /// A buy order's `price × quantity`, into its escrow (E3.1)
    MarketEscrow,
    /// The 1% listing fee on placing an order: a sink, never refunded (E3.1)
    MarketFee,
    /// What's left of a buy's escrow, back on a cancel or expiry (E3.1), or
    /// what it paid over a fill's price (E3.2)
    MarketRefund,
    /// A fill's proceeds to the seller: the fill's price × its quantity (E3.2)
    MarketSale,
    /// A bazaar stall's rent, paid ahead: the bazaar's sink (E3.10)
    BazaarRent,
    /// A bazaar purchase, from the buyer (E3.10)
    BazaarPurchase,
    /// A bazaar purchase, to the merchant (E3.10)
    BazaarSale,
    /// Rent given back for days a stall didn't stand (E3.12's migration)
    BazaarRefund,
}

impl TransactionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransactionType::ExchangeDeposit => "exchange_deposit",
            TransactionType::ExchangeWithdraw => "exchange_withdraw",
            TransactionType::TradeReceived => "trade_received",
            TransactionType::TradeSent => "trade_sent",
            TransactionType::AdminAdjustment => "admin_adjustment",
            TransactionType::DeedPurchase => "deed_purchase",
            TransactionType::DeedRefund => "deed_refund",
            TransactionType::ForeclosureRefund => "foreclosure_refund",
            TransactionType::TaxPayment => "tax_payment",
            TransactionType::PropertyTax => "property_tax",
            TransactionType::MarketEscrow => "market_escrow",
            TransactionType::MarketFee => "market_fee",
            TransactionType::MarketRefund => "market_refund",
            TransactionType::MarketSale => "market_sale",
            TransactionType::BazaarRent => "bazaar_rent",
            TransactionType::BazaarPurchase => "bazaar_purchase",
            TransactionType::BazaarSale => "bazaar_sale",
            TransactionType::BazaarRefund => "bazaar_refund",
        }
    }
}

/// Add strands to a player's balance with audit logging, in its own transaction.
/// Returns the new balance.
#[allow(dead_code)] // the ledger API; the market's escrow and fills (#167, #168) post through it
pub async fn add_strands(
    pool: &PgPool,
    player_id: Uuid,
    amount: i64,
    tx_type: TransactionType,
    description: Option<&str>,
    reference_id: Option<Uuid>,
) -> Result<i64, String> {
    let mut tx = pool.begin().await.map_err(|e| format!("Failed to start transaction: {}", e))?;
    let (balance_before, balance_after) =
        apply(&mut tx, player_id, amount, false, &tx_type, description, reference_id).await?;
    tx.commit().await.map_err(|e| format!("Failed to commit: {}", e))?;

    tracing::info!(
        "Added {} strands to player {} ({}): {} -> {}",
        amount, player_id, tx_type.as_str(), balance_before, balance_after
    );

    Ok(balance_after)
}

/// Subtract strands from a player's balance with audit logging, in its own transaction.
/// Returns the new balance, or error if insufficient funds.
#[allow(dead_code)] // the ledger API; the market's escrow and fills (#167, #168) post through it
pub async fn subtract_strands(
    pool: &PgPool,
    player_id: Uuid,
    amount: i64,
    tx_type: TransactionType,
    description: Option<&str>,
    reference_id: Option<Uuid>,
) -> Result<i64, String> {
    let mut tx = pool.begin().await.map_err(|e| format!("Failed to start transaction: {}", e))?;
    let (balance_before, balance_after) =
        apply(&mut tx, player_id, amount, true, &tx_type, description, reference_id).await?;
    tx.commit().await.map_err(|e| format!("Failed to commit: {}", e))?;

    tracing::info!(
        "Subtracted {} strands from player {} ({}): {} -> {}",
        amount, player_id, tx_type.as_str(), balance_before, balance_after
    );

    Ok(balance_after)
}

/// [`add_strands`] inside the caller's transaction, so the money moves only if
/// the caller's other writes commit. The caller commits, and an error here
/// should roll the whole transaction back.
pub async fn add_strands_in(
    conn: &mut PgConnection,
    player_id: Uuid,
    amount: i64,
    tx_type: TransactionType,
    description: Option<&str>,
    reference_id: Option<Uuid>,
) -> Result<i64, String> {
    apply(conn, player_id, amount, false, &tx_type, description, reference_id)
        .await
        .map(|(_, after)| after)
}

/// [`subtract_strands`] inside the caller's transaction. See [`add_strands_in`].
pub async fn subtract_strands_in(
    conn: &mut PgConnection,
    player_id: Uuid,
    amount: i64,
    tx_type: TransactionType,
    description: Option<&str>,
    reference_id: Option<Uuid>,
) -> Result<i64, String> {
    apply(conn, player_id, amount, true, &tx_type, description, reference_id)
        .await
        .map(|(_, after)| after)
}

/// Add `amount` to the player's balance, or take it off if `subtract`, and
/// write the audit record, in one statement (#208). The update locks the
/// player row until the transaction ends. Returns the balance before and after.
async fn apply(
    conn: &mut PgConnection,
    player_id: Uuid,
    amount: i64,
    subtract: bool,
    tx_type: &TransactionType,
    description: Option<&str>,
    reference_id: Option<Uuid>,
) -> Result<(i64, i64), String> {
    if amount <= 0 {
        return Err("Amount must be positive".to_string());
    }
    let amount = if subtract { -amount } else { amount };
    // The balances the move works from: it neither goes below 0 nor
    // overflows. The bounds are in the WHERE, so a refusal changes nothing
    // and leaves the caller's transaction usable.
    let (lowest, highest) = (-amount, if subtract { i64::MAX } else { i64::MAX - amount });

    // Update balance and insert audit record (negative amount for subtraction)
    let moved: Option<i64> = sqlx::query_scalar(
        "WITH moved AS (
             UPDATE players SET strand_balance = strand_balance + $2
             WHERE id = $1 AND strand_balance BETWEEN $3 AND $4
             RETURNING strand_balance
         )
         INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, reference_id, description)
         SELECT $1, $2, strand_balance - $2, strand_balance, $5, $6, $7 FROM moved
         RETURNING balance_after",
    )
    .bind(player_id)
    .bind(amount)
    .bind(lowest)
    .bind(highest)
    .bind(tx_type.as_str())
    .bind(reference_id)
    .bind(description)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| format!("Failed to update balance: {}", e))?;
    if let Some(balance_after) = moved {
        return Ok((balance_after - amount, balance_after));
    }

    // Refused: say why
    let balance_before: i64 = sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
        .bind(player_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| format!("Player not found: {}", e))?
        .ok_or_else(|| "Player not found".to_string())?;
    if balance_before < lowest {
        return Err(format!("Insufficient strands: have {}, need {}", balance_before, -amount));
    }
    Err(format!("Balance would overflow: {} + {}", balance_before, amount))
}

/// Transfer strands from one player to another atomically.
/// Both players are locked in consistent order to prevent deadlocks.
#[allow(dead_code)] // the ledger API; the market's escrow and fills (#167, #168) post through it
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

/// The strand balances of `players`, for telling them after a change made
/// outside a request of theirs (the tax run, a forced terrain commit).
/// Read after the change commits, so each is at least as new as it.
pub async fn balances_of(pool: &PgPool, players: &[Uuid]) -> Result<Vec<(Uuid, i64)>, sqlx::Error> {
    if players.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as("SELECT id, strand_balance FROM players WHERE id = ANY($1) ORDER BY id")
        .bind(players)
        .fetch_all(pool)
        .await
}

/// Tell each player their new balance (`currency_changed`, #148). The
/// connection sends it only to that player.
pub fn announce_balances(events: &broadcast::Sender<GameEvent>, balances: impl IntoIterator<Item = (Uuid, i64)>) {
    for (player_id, new_balance) in balances {
        let _ = events.send(GameEvent::CurrencyChanged { player_id, new_balance });
    }
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test currency -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::market::tests::{balance, player};

    /// A move is one statement (#208): the balance and its audit row together,
    /// and a refusal writes neither and leaves the caller's transaction usable.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_move_writes_balance_and_audit_and_a_refusal_writes_nothing(db: PgPool) {
        let ada = player(&db, "ada", 100).await;
        let mut tx = db.begin().await.unwrap();
        let refused = subtract_strands_in(&mut tx, ada.id, 101, TransactionType::MarketEscrow, None, None).await;
        assert_eq!(refused, Err("Insufficient strands: have 100, need 101".to_string()));
        let overflow = add_strands_in(&mut tx, ada.id, i64::MAX, TransactionType::AdminAdjustment, None, None).await;
        assert!(overflow.unwrap_err().starts_with("Balance would overflow"));
        let missing = add_strands_in(&mut tx, Uuid::new_v4(), 5, TransactionType::AdminAdjustment, None, None).await;
        assert_eq!(missing, Err("Player not found".to_string()));
        // The transaction goes on
        let after = subtract_strands_in(&mut tx, ada.id, 30, TransactionType::MarketEscrow, Some("bid"), None).await;
        assert_eq!(after, Ok(70));
        assert_eq!(add_strands_in(&mut tx, ada.id, 5, TransactionType::MarketRefund, None, None).await, Ok(75));
        tx.commit().await.unwrap();

        assert_eq!(balance(&db, ada.id).await, 75);
        let audit: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "SELECT amount, balance_before, balance_after, transaction_type
             FROM currency_transactions WHERE player_id = $1 ORDER BY balance_after",
        )
        .bind(ada.id)
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(audit, vec![(-30, 100, 70, "market_escrow".into()), (5, 70, 75, "market_refund".into())]);
    }
}
