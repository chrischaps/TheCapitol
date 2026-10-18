//! Hourly property tax (M7), and foreclosure for unpaid tax (#131).
//!
//! Each hour a plot owes 1/168th of its weekly tax (2% of `assessed_value`).
//! An hour its owner can't pay goes on `plots.tax_owed`. The next hour they
//! can, the debt comes out of their strands, as much as their balance covers.
//! The owner hears about the debt (`tax_delinquent`) as it passes each whole
//! week of tax, and again an hour before it reaches [`FORECLOSE_AFTER_WEEKS`]
//! weeks. Then the plot is foreclosed. It's released the same way a forced
//! terrain commit releases one: its stations go, and their contents with
//! them. The deed is refunded, less the debt.

use std::collections::HashMap;

use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use super::currency::{add_strands_in, TransactionType};
use super::events::GameEvent;
use super::plot_sync::release_plot;
use crate::models::StationState;

/// Weekly tax rate as a fraction (2% = 0.02)
const TAX_RATE: f64 = 0.02;

/// Hours per week
const HOURS_PER_WEEK: f64 = 168.0;

/// How many weeks of tax a plot may owe before it's foreclosed (#131)
pub const FORECLOSE_AFTER_WEEKS: i64 = 4;

/// Property tax transaction type for currency system
pub enum PropertyTaxType {
    Collection,
    Delinquent,
}

/// A week's tax on a plot assessed at `assessed_value`
pub fn weekly_tax(assessed_value: i32) -> i64 {
    (assessed_value as f64 * TAX_RATE).round() as i64
}

/// An hour's tax: a week's over 168, rounded up
pub fn hourly_tax(assessed_value: i32) -> i64 {
    ((weekly_tax(assessed_value) as f64) / HOURS_PER_WEEK).ceil() as i64
}

/// A week of tax as it's charged: 168 hours of [`hourly_tax`]. Rounding each
/// hour up makes this more than [`weekly_tax`] on a cheap plot (an urban deed
/// of 1000 is charged 168 a week, not 20). Notices and foreclosure count
/// in these weeks, so a plot goes after about four weeks unpaid whatever
/// its value. Charging the exact fraction instead is #136.
pub fn week_charged(assessed_value: i32) -> i64 {
    hourly_tax(assessed_value) * HOURS_PER_WEEK as i64
}

/// The debt at which a plot is foreclosed: four weeks of tax as charged
pub fn foreclosure_threshold(assessed_value: i32) -> i64 {
    week_charged(assessed_value) * FORECLOSE_AFTER_WEEKS
}

/// Whether debt going from `before` to `after` in one unpaid hour earns the
/// owner a notice: it passed a whole week of tax, or one more hour forecloses.
/// A debt that has reached the threshold is foreclosed, not warned.
pub fn notice_due(before: i64, after: i64, assessed_value: i32) -> bool {
    let weekly = week_charged(assessed_value);
    let hourly = hourly_tax(assessed_value);
    let threshold = foreclosure_threshold(assessed_value);
    if after >= threshold || after <= before || weekly <= 0 {
        return false;
    }
    let passed_a_week = (1..FORECLOSE_AFTER_WEEKS).any(|k| before < k * weekly && after >= k * weekly);
    let last_hour = after + hourly >= threshold && before + hourly < threshold;
    passed_a_week || last_hour
}

/// A plot's owner should hear about its debt (`tax_delinquent`).
#[derive(Debug, Clone, PartialEq)]
pub struct TaxNotice {
    pub owner_id: Uuid,
    pub terrain_id: String,
    pub tax_owed: i64,
    pub forecloses_at: i64,
}

/// A plot taken for unpaid tax.
#[derive(Debug, Clone, PartialEq)]
pub struct Foreclosure {
    pub plot_id: Uuid,
    pub owner_id: Uuid,
    /// `None` for a plot from before plots came from the terrain. No client
    /// knows it, so there's nothing to announce.
    pub terrain_id: Option<String>,
    pub tax_owed: i64,
    pub refund: i64,
    /// The owner's strands after the refund
    pub balance: i64,
    pub removed_stations: Vec<Uuid>,
}

/// What an hour's tax run did that the game and the clients need to hear.
#[derive(Debug, Default)]
pub struct TaxReport {
    pub notices: Vec<TaxNotice>,
    pub foreclosures: Vec<Foreclosure>,
}

/// Process hourly property taxes for all owned plots. The tick loop passes
/// the report to [`announce`].
pub async fn process_property_taxes(db: &PgPool) -> TaxReport {
    tracing::debug!("Processing property taxes...");
    let mut report = TaxReport::default();

    // Get all owned plots with their owners
    let owned_plots: Vec<(Uuid, Uuid, i32, i32, Option<String>)> = match sqlx::query_as(
        "SELECT id, owner_id, assessed_value, tax_owed, terrain_id FROM plots WHERE owner_id IS NOT NULL",
    )
    .fetch_all(db)
    .await
    {
        Ok(plots) => plots,
        Err(e) => {
            tracing::error!("Failed to fetch owned plots for tax: {:?}", e);
            return report;
        }
    };

    if owned_plots.is_empty() {
        return report;
    }

    tracing::info!("Processing taxes for {} owned plots", owned_plots.len());

    for (plot_id, owner_id, assessed_value, tax_owed, terrain_id) in owned_plots {
        let hourly_tax = hourly_tax(assessed_value);

        if hourly_tax <= 0 {
            continue;
        }

        // Try to collect from player's balance
        let collected = collect_tax_from_player(db, owner_id, hourly_tax).await;

        let owed = if collected {
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

            if tax_owed > 0 {
                collect_debt(db, plot_id, owner_id).await
            } else {
                0
            }
        } else {
            // Add to tax debt
            let after: i32 = match sqlx::query_scalar(
                "UPDATE plots SET tax_owed = tax_owed + $1 WHERE id = $2 RETURNING tax_owed"
            )
            .bind(hourly_tax as i32)
            .bind(plot_id)
            .fetch_one(db)
            .await
            {
                Ok(after) => after,
                Err(e) => {
                    tracing::error!("Failed to add tax debt for plot {}: {:?}", plot_id, e);
                    continue;
                }
            };

            // Log delinquent tax
            log_tax_transaction(db, plot_id, owner_id, hourly_tax, false).await;

            tracing::warn!(
                "Player {} could not pay {} strands tax for plot {} - added to debt ({} owed)",
                owner_id, hourly_tax, plot_id, after
            );

            let after = after as i64;
            if notice_due(after - hourly_tax, after, assessed_value) {
                if let Some(terrain_id) = &terrain_id {
                    report.notices.push(TaxNotice {
                        owner_id,
                        terrain_id: terrain_id.clone(),
                        tax_owed: after,
                        forecloses_at: foreclosure_threshold(assessed_value),
                    });
                }
            }
            after
        };

        if owed >= foreclosure_threshold(assessed_value) {
            match foreclose(db, plot_id).await {
                Ok(Some(f)) => {
                    tracing::warn!(
                        "Plot {} foreclosed from player {} for {} strands unpaid tax; refunded {}",
                        plot_id, owner_id, f.tax_owed, f.refund
                    );
                    report.foreclosures.push(f);
                }
                Ok(None) => {}
                Err(e) => tracing::error!("Failed to foreclose plot {}: {}", plot_id, e),
            }
        }
    }
    report
}

/// Take a plot's tax debt from its owner's strands, as far as they reach.
/// Returns what's still owed.
async fn collect_debt(db: &PgPool, plot_id: Uuid, owner_id: Uuid) -> i64 {
    let result: Result<i32, sqlx::Error> = async {
        let mut tx = db.begin().await?;
        let owed: i32 = sqlx::query_scalar("SELECT tax_owed FROM plots WHERE id = $1 FOR UPDATE")
            .bind(plot_id)
            .fetch_one(&mut *tx)
            .await?;
        let balance: i64 = sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1 FOR UPDATE")
            .bind(owner_id)
            .fetch_one(&mut *tx)
            .await?;
        let take = (owed as i64).min(balance.max(0));
        if take <= 0 {
            return Ok(owed);
        }
        sqlx::query("UPDATE players SET strand_balance = strand_balance - $1 WHERE id = $2")
            .bind(take)
            .bind(owner_id)
            .execute(&mut *tx)
            .await?;
        let left: i32 = sqlx::query_scalar("UPDATE plots SET tax_owed = tax_owed - $1 WHERE id = $2 RETURNING tax_owed")
            .bind(take as i32)
            .bind(plot_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!(
            "Collected {} strands of tax debt from player {} for plot {} ({} left)",
            take, owner_id, plot_id, left
        );
        Ok(left)
    }
    .await;
    match result {
        Ok(left) => left as i64,
        Err(e) => {
            tracing::error!("Failed to collect tax debt for plot {}: {:?}", plot_id, e);
            0
        }
    }
}

/// Foreclose a plot whose debt has reached the threshold. Releasing it and
/// refunding the deed less the debt happen in one transaction. Its stations
/// and their contents are destroyed; giving them back to the owner is #137. `None` if,
/// once the row is locked, the plot is no longer owned or owes less.
pub async fn foreclose(db: &PgPool, plot_id: Uuid) -> Result<Option<Foreclosure>, String> {
    let mut tx = db.begin().await.map_err(|e| e.to_string())?;
    let row: Option<(Option<Uuid>, i32, i32, Option<String>)> = sqlx::query_as(
        "SELECT owner_id, assessed_value, tax_owed, terrain_id FROM plots WHERE id = $1 FOR UPDATE",
    )
    .bind(plot_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let Some((Some(owner_id), assessed_value, tax_owed, terrain_id)) = row else {
        return Ok(None);
    };
    let tax_owed = tax_owed as i64;
    if tax_owed <= 0 || tax_owed < foreclosure_threshold(assessed_value) {
        return Ok(None);
    }
    let removed_stations = release_plot(&mut tx, plot_id).await.map_err(|e| e.to_string())?;
    let refund = (assessed_value as i64 - tax_owed).max(0);
    let balance = if refund > 0 {
        add_strands_in(
            &mut tx,
            owner_id,
            refund,
            TransactionType::ForeclosureRefund,
            Some(&format!("Plot foreclosed for {tax_owed} strands of unpaid tax: deed refunded less the debt")),
            Some(plot_id),
        )
        .await?
    } else {
        sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
            .bind(owner_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
    };
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(Some(Foreclosure { plot_id, owner_id, terrain_id, tax_owed, refund, balance, removed_stations }))
}

/// Apply a tax run to the game and tell the clients. The owner gets the
/// notices, `plot_foreclosed` and their balance after the refund, but only
/// if they're online; keeping those for an offline owner is #138. Everyone
/// hears about each removed station, and gets `plot_released`.
pub fn announce(report: TaxReport, stations: &mut HashMap<Uuid, StationState>, events: &broadcast::Sender<GameEvent>) {
    for n in report.notices {
        let _ = events.send(GameEvent::TaxDelinquent {
            owner_id: n.owner_id,
            terrain_id: n.terrain_id,
            tax_owed: n.tax_owed,
            forecloses_at: n.forecloses_at,
        });
    }
    for f in report.foreclosures {
        for station_id in f.removed_stations {
            stations.remove(&station_id);
            let _ = events.send(GameEvent::StationRemoved { station_id });
        }
        if f.refund > 0 {
            let _ = events.send(GameEvent::CurrencyChanged { player_id: f.owner_id, new_balance: f.balance });
        }
        if let Some(terrain_id) = f.terrain_id {
            let _ = events.send(GameEvent::PlotForeclosed {
                owner_id: f.owner_id,
                terrain_id: terrain_id.clone(),
                tax_owed: f.tax_owed,
                refund: f.refund,
            });
            let _ = events.send(GameEvent::PlotReleased { terrain_id });
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

/// The `#[ignore]`d ones need the local Postgres (`docker-compose up -d`). Run
/// them with `cargo test property_tax -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{balance, ledger, owner, player, plot};

    /// 6000 assessed: 120 a week at 2%, but 1 an hour is 168 a week as
    /// charged, so it's foreclosed at 672 owed
    const VALUE: i32 = 6000;

    #[test]
    fn the_threshold_is_four_weeks_of_tax_as_charged() {
        assert_eq!(weekly_tax(VALUE), 120);
        assert_eq!(hourly_tax(VALUE), 1);
        assert_eq!(week_charged(VALUE), 168);
        assert_eq!(foreclosure_threshold(VALUE), 672);
        // The cheapest deed still takes four weeks unpaid to lose.
        assert_eq!(foreclosure_threshold(100) / hourly_tax(100), 4 * 168);
    }

    #[test]
    fn notices_come_at_each_week_and_an_hour_before() {
        let due: Vec<i64> = (1..=672).filter(|&after| notice_due(after - 1, after, VALUE)).collect();
        assert_eq!(due, vec![168, 336, 504, 671]);
    }

    #[test]
    fn a_big_hour_still_warns_once() {
        // 168000 assessed: 3360 a week, 20 an hour, foreclosed at 13440
        let v = 168_000;
        assert_eq!(hourly_tax(v), 20);
        let due: Vec<i64> = (1..=13_440 / 20).map(|h| h * 20).filter(|&a| notice_due(a - 20, a, v)).collect();
        assert_eq!(due, vec![3360, 6720, 10080, 13420]);
    }

    async fn owe(db: &PgPool, lot: Uuid, owed: i32) {
        sqlx::query("UPDATE plots SET tax_owed = $1 WHERE id = $2")
            .bind(owed)
            .bind(lot)
            .execute(db)
            .await
            .unwrap();
    }

    async fn owed(db: &PgPool, lot: Uuid) -> i32 {
        sqlx::query_scalar("SELECT tax_owed FROM plots WHERE id = $1").bind(lot).fetch_one(db).await.unwrap()
    }

    /// A storage chest on `lot`, with a container
    async fn station(db: &PgPool, owner: Uuid, lot: Uuid) -> (Uuid, Uuid) {
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality)
             VALUES ($1, 'storage_chest', $2, 1200, 0, $3, 50)",
        )
        .bind(station)
        .bind(owner)
        .bind(lot)
        .execute(db)
        .await
        .unwrap();
        let container = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2)")
            .bind(container)
            .bind(station)
            .execute(db)
            .await
            .unwrap();
        (station, container)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_last_unpaid_hour_forecloses_and_says_so(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 671).await;
        let (chest, chest_box) = station(&db, ada, lot).await;

        let report = process_property_taxes(&db).await;

        assert!(report.notices.is_empty(), "foreclosed, not warned");
        assert_eq!(report.foreclosures.len(), 1);
        let f = &report.foreclosures[0];
        assert_eq!((f.owner_id, f.tax_owed, f.refund, f.balance), (ada, 672, 5328, 5328));
        assert_eq!(f.removed_stations, vec![chest]);
        assert_eq!(owner(&db, lot).await, Some(None), "the lot is unclaimed again");
        assert_eq!(owed(&db, lot).await, 0);
        assert_eq!(balance(&db, ada).await, 5328);
        assert_eq!(ledger(&db, ada).await, vec![(5328, "foreclosure_refund".to_string())]);
        let boxes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM containers WHERE id = $1")
            .bind(chest_box)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(boxes, 0, "the chest's contents went with it");

        // The game drops the chest, the owner hears why, and everyone hears the lot is free.
        let (events, mut rx) = broadcast::channel(16);
        let mut stations = HashMap::new();
        announce(report, &mut stations, &events);
        let mut sent = Vec::new();
        while let Ok(e) = rx.try_recv() {
            sent.push(e);
        }
        let terrain_id = format!("test-{lot}");
        assert!(matches!(&sent[0], GameEvent::StationRemoved { station_id } if *station_id == chest));
        assert!(matches!(&sent[1], GameEvent::CurrencyChanged { player_id, new_balance: 5328 } if *player_id == ada));
        assert!(matches!(&sent[2], GameEvent::PlotForeclosed { owner_id, terrain_id: t, tax_owed: 672, refund: 5328 }
            if *owner_id == ada && *t == terrain_id));
        assert!(matches!(&sent[3], GameEvent::PlotReleased { terrain_id: t } if *t == terrain_id));
        assert_eq!(sent.len(), 4);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_debt_past_the_deed_refunds_nothing(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 7000).await;

        let report = process_property_taxes(&db).await;

        assert_eq!(report.foreclosures[0].refund, 0);
        assert_eq!(balance(&db, ada).await, 0);
        assert!(ledger(&db, ada).await.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn passing_a_week_owed_warns_the_owner(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 167).await;

        let report = process_property_taxes(&db).await;

        assert!(report.foreclosures.is_empty());
        assert_eq!(
            report.notices,
            vec![TaxNotice { owner_id: ada, terrain_id: format!("test-{lot}"), tax_owed: 168, forecloses_at: 672 }]
        );
        assert_eq!(owner(&db, lot).await, Some(Some(ada)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn strands_pay_off_the_debt_as_far_as_they_reach(db: PgPool) {
        let ada = player(&db, "ada", 101).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 671).await;

        let report = process_property_taxes(&db).await;

        // 1 for the hour, then the other 100 off the debt
        assert!(report.foreclosures.is_empty() && report.notices.is_empty());
        assert_eq!(balance(&db, ada).await, 0);
        assert_eq!(owed(&db, lot).await, 571);
        assert_eq!(owner(&db, lot).await, Some(Some(ada)));
    }
}
