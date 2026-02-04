use sqlx::PgPool;
use uuid::Uuid;

/// Weekly tax rate as a fraction (2% = 0.02)
const TAX_RATE: f64 = 0.02;

/// Hours per week
const HOURS_PER_WEEK: f64 = 168.0;

/// Property tax transaction type for currency system
pub enum PropertyTaxType {
    Collection,
    Delinquent,
}

/// Process hourly property taxes for all owned plots
pub async fn process_property_taxes(db: &PgPool) {
    tracing::debug!("Processing property taxes...");

    // Get all owned plots with their owners
    let owned_plots: Vec<(Uuid, Uuid, i32)> = match sqlx::query_as(
        "SELECT id, owner_id, assessed_value FROM plots WHERE owner_id IS NOT NULL"
    )
    .fetch_all(db)
    .await
    {
        Ok(plots) => plots,
        Err(e) => {
            tracing::error!("Failed to fetch owned plots for tax: {:?}", e);
            return;
        }
    };

    if owned_plots.is_empty() {
        return;
    }

    tracing::info!("Processing taxes for {} owned plots", owned_plots.len());

    for (plot_id, owner_id, assessed_value) in owned_plots {
        // Calculate hourly tax (1/168th of weekly rate)
        let weekly_tax = (assessed_value as f64 * TAX_RATE).round() as i64;
        let hourly_tax = ((weekly_tax as f64) / HOURS_PER_WEEK).ceil() as i64;

        if hourly_tax <= 0 {
            continue;
        }

        // Try to collect from player's balance
        let collected = collect_tax_from_player(db, owner_id, hourly_tax).await;

        if collected {
            // Update last_tax_paid timestamp
            if let Err(e) = sqlx::query(
                "UPDATE plots SET last_tax_paid = NOW() WHERE id = $1"
            )
            .bind(plot_id)
            .execute(db)
            .await
            {
                tracing::error!("Failed to update last_tax_paid for plot {}: {:?}", plot_id, e);
            }

            // Log successful collection
            log_tax_transaction(db, plot_id, owner_id, hourly_tax, true).await;

            tracing::debug!(
                "Collected {} strands tax from player {} for plot {}",
                hourly_tax, owner_id, plot_id
            );
        } else {
            // Add to tax debt
            if let Err(e) = sqlx::query(
                "UPDATE plots SET tax_owed = tax_owed + $1 WHERE id = $2"
            )
            .bind(hourly_tax as i32)
            .bind(plot_id)
            .execute(db)
            .await
            {
                tracing::error!("Failed to add tax debt for plot {}: {:?}", plot_id, e);
            }

            // Log delinquent tax
            log_tax_transaction(db, plot_id, owner_id, hourly_tax, false).await;

            tracing::warn!(
                "Player {} could not pay {} strands tax for plot {} - added to debt",
                owner_id, hourly_tax, plot_id
            );
        }
    }
}

/// Attempt to collect tax from a player's strand balance
async fn collect_tax_from_player(db: &PgPool, player_id: Uuid, amount: i64) -> bool {
    // Start transaction for atomic operation
    let mut tx = match db.begin().await {
        Ok(tx) => tx,
        Err(_) => return false,
    };

    // Check and deduct balance atomically
    let result = sqlx::query(
        "UPDATE players SET strand_balance = strand_balance - $1
         WHERE id = $2 AND strand_balance >= $1"
    )
    .bind(amount)
    .bind(player_id)
    .execute(&mut *tx)
    .await;

    match result {
        Ok(result) if result.rows_affected() > 0 => {
            if tx.commit().await.is_ok() {
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Log a tax transaction for auditing
async fn log_tax_transaction(
    db: &PgPool,
    plot_id: Uuid,
    player_id: Uuid,
    amount: i64,
    paid: bool,
) {
    // Calculate current tax week (start of week)
    let tax_week = chrono::Utc::now().date_naive();

    if paid {
        let result = sqlx::query(
            "INSERT INTO property_tax_transactions (id, plot_id, player_id, amount, tax_week)
             VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(Uuid::new_v4())
        .bind(plot_id)
        .bind(player_id)
        .bind(amount as i32)
        .bind(tax_week)
        .execute(db)
        .await;

        if let Err(e) = result {
            tracing::error!("Failed to log tax transaction: {:?}", e);
        }
    }
    // For delinquent taxes, we just update tax_owed on the plot, no separate log entry
}

/// Get a player's total outstanding tax debt across all plots
pub async fn get_player_tax_debt(db: &PgPool, player_id: Uuid) -> Result<i64, sqlx::Error> {
    let debt: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(tax_owed), 0) FROM plots WHERE owner_id = $1"
    )
    .bind(player_id)
    .fetch_one(db)
    .await?;

    Ok(debt.0)
}

/// Pay off tax debt for a specific plot
pub async fn pay_tax_debt(
    db: &PgPool,
    player_id: Uuid,
    plot_id: Uuid,
    amount: i64,
) -> Result<i64, String> {
    // Verify player owns the plot
    let plot_owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT owner_id FROM plots WHERE id = $1"
    )
    .bind(plot_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("Database error: {}", e))?
    .flatten();

    if plot_owner != Some(player_id) {
        return Err("You don't own this plot".to_string());
    }

    // Get current tax owed
    let tax_owed: i32 = sqlx::query_scalar(
        "SELECT tax_owed FROM plots WHERE id = $1"
    )
    .bind(plot_id)
    .fetch_one(db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    let payment = amount.min(tax_owed as i64);
    if payment <= 0 {
        return Err("No tax debt to pay".to_string());
    }

    // Start transaction
    let mut tx = db.begin().await.map_err(|e| format!("Transaction error: {}", e))?;

    // Deduct from player balance
    let result = sqlx::query(
        "UPDATE players SET strand_balance = strand_balance - $1
         WHERE id = $2 AND strand_balance >= $1"
    )
    .bind(payment)
    .bind(player_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Payment error: {}", e))?;

    if result.rows_affected() == 0 {
        return Err("Insufficient strands".to_string());
    }

    // Reduce tax debt
    sqlx::query(
        "UPDATE plots SET tax_owed = tax_owed - $1 WHERE id = $2"
    )
    .bind(payment as i32)
    .bind(plot_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("Debt update error: {}", e))?;

    tx.commit().await.map_err(|e| format!("Commit error: {}", e))?;

    // Get remaining debt
    let remaining: i32 = sqlx::query_scalar(
        "SELECT tax_owed FROM plots WHERE id = $1"
    )
    .bind(plot_id)
    .fetch_one(db)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    Ok(remaining as i64)
}
