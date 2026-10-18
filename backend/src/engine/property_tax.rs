//! Hourly property tax (M7), and foreclosure for unpaid tax (#131).
//!
//! Each hour a plot owes 1/168th of its weekly tax (2% of `assessed_value`).
//! The part that isn't a whole strand carries to the next hour on
//! `plots.tax_carry` (#136), so a week charges exactly the weekly tax: a deed
//! of 1000 pays 1 strand in about every eighth hour, 20 a week.
//! An hour its owner can't pay goes on `plots.tax_owed`. The next hour they
//! can, the debt comes out of their strands, as much as their balance covers.
//! The owner hears about the debt (`tax_delinquent`) as it passes each whole
//! week of tax, and again when one more hour's charge would bring it to
//! [`FORECLOSE_AFTER_WEEKS`] weeks. Then the plot is foreclosed. Its stations
//! come down: each kit goes back to the station's owner, and what was in the
//! boxes to whoever owns it (#137). What doesn't fit an inventory is lost
//! (#141). Then it's released the same way a forced terrain commit releases
//! one, and the deed is refunded, less the debt.
//!
//! Both kinds of word to the owner are queued as notices (#138), so an owner
//! who's offline hears on their return. The owner can pay the debt off
//! themselves with [`pay_tax_debt`] (`POST /plots/{id}/pay-tax`).

use std::collections::HashMap;

use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use super::currency::{add_strands_in, announce_balances, balances_of, subtract_strands_in, TransactionType};
use super::events::GameEvent;
use super::notices;
use super::plot_sync::release_plot;
use super::station_removal::return_plot_stations;
use crate::error::AppError;
use crate::models::StationState;
use capitol_protocol::ws::ServerMessage;

/// Weekly tax rate as a fraction (2% = 0.02)
const TAX_RATE: f64 = 0.02;

/// Hours per week. `tax_carry` counts in 168ths of a strand.
const HOURS_PER_WEEK: i64 = 168;

/// How many weeks of tax a plot may owe before it's foreclosed (#131)
pub const FORECLOSE_AFTER_WEEKS: i64 = 4;

/// A week's tax on a plot assessed at `assessed_value`
pub fn weekly_tax(assessed_value: i32) -> i64 {
    (assessed_value as f64 * TAX_RATE).round() as i64
}

/// An hour's tax on a plot with `weekly` tax and `carry` 168ths of a strand
/// left from the hours before: the whole strands due, and the new carry
pub fn charge_hour(weekly: i64, carry: i32) -> (i64, i32) {
    let due = carry as i64 + weekly;
    (due / HOURS_PER_WEEK, (due % HOURS_PER_WEEK) as i32)
}

/// The most one hour can charge: a week's tax over 168, rounded up
pub fn largest_hourly_tax(assessed_value: i32) -> i64 {
    (weekly_tax(assessed_value) + HOURS_PER_WEEK - 1) / HOURS_PER_WEEK
}

/// The debt at which a plot is foreclosed: four weeks of tax
pub fn foreclosure_threshold(assessed_value: i32) -> i64 {
    weekly_tax(assessed_value) * FORECLOSE_AFTER_WEEKS
}

/// Whether debt going from `before` to `after` in one unpaid hour earns the
/// owner a notice: it passed a whole week of tax, or one more hour's charge
/// could foreclose. On a cheap plot that last warning comes days ahead: a
/// wilderness deed is charged 1 strand every 84 hours. A debt that has
/// reached the threshold is foreclosed, not warned.
pub fn notice_due(before: i64, after: i64, assessed_value: i32) -> bool {
    let weekly = weekly_tax(assessed_value);
    let hourly = largest_hourly_tax(assessed_value);
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
    /// Its queued notice (#138). `None` if queuing failed: it's sent live only.
    pub notice_id: Option<Uuid>,
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
    /// `(player, inventory)` for each inventory the stations were emptied into
    pub returned_to: Vec<(Uuid, Uuid)>,
    /// What didn't fit, as `(item_type, quantity)`
    pub lost: Vec<(String, i32)>,
    /// The queued `plot_foreclosed` notice (#138), if there's a lot to name
    pub notice_id: Option<Uuid>,
}

/// What an hour's tax run did that the game and the clients need to hear.
#[derive(Debug, Default)]
pub struct TaxReport {
    pub notices: Vec<TaxNotice>,
    pub foreclosures: Vec<Foreclosure>,
    /// The new balance of each owner the run charged or refunded, read once
    /// it's done (`currency_changed`, #148)
    pub balances: Vec<(Uuid, i64)>,
}

/// Process hourly property taxes for all owned plots. The tick loop passes
/// the report to [`announce`].
pub async fn process_property_taxes(db: &PgPool) -> TaxReport {
    tracing::debug!("Processing property taxes...");
    let mut report = TaxReport::default();

    // Get all owned plots with their owners
    let owned_plots: Vec<(Uuid, Uuid, i32, i32, i32, Option<String>)> = match sqlx::query_as(
        "SELECT id, owner_id, assessed_value, tax_owed, tax_carry, terrain_id FROM plots WHERE owner_id IS NOT NULL",
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
    let mut charged: Vec<Uuid> = Vec::new();

    for (plot_id, owner_id, assessed_value, tax_owed, carry, terrain_id) in owned_plots {
        let weekly = weekly_tax(assessed_value);
        if weekly <= 0 {
            continue;
        }

        let (hourly_tax, carry) = charge_hour(weekly, carry);
        if let Err(e) = sqlx::query("UPDATE plots SET tax_carry = $1 WHERE id = $2")
            .bind(carry)
            .bind(plot_id)
            .execute(db)
            .await
        {
            tracing::error!("Failed to carry tax for plot {}: {:?}", plot_id, e);
            continue;
        }

        // An hour that charges nothing still takes what it can of the debt
        let owed = if hourly_tax == 0 {
            if tax_owed > 0 {
                charged.push(owner_id);
                collect_debt(db, plot_id, owner_id).await
            } else {
                0
            }
        } else if collect_tax_from_player(db, owner_id, hourly_tax).await {
            charged.push(owner_id);
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
                    let forecloses_at = foreclosure_threshold(assessed_value);
                    let message = ServerMessage::TaxDelinquent {
                        terrain_id: terrain_id.clone(),
                        tax_owed: after,
                        forecloses_at,
                    };
                    let notice_id = notices::queue(db, owner_id, terrain_id, &message)
                        .await
                        .map_err(|e| tracing::error!("Failed to queue a tax notice for plot {}: {:?}", plot_id, e))
                        .ok();
                    report.notices.push(TaxNotice {
                        owner_id,
                        terrain_id: terrain_id.clone(),
                        tax_owed: after,
                        forecloses_at,
                        notice_id,
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
                    if f.refund > 0 {
                        charged.push(owner_id);
                    }
                    report.foreclosures.push(f);
                }
                Ok(None) => {}
                Err(e) => tracing::error!("Failed to foreclose plot {}: {}", plot_id, e),
            }
        }
    }
    charged.sort();
    charged.dedup();
    match balances_of(db, &charged).await {
        Ok(balances) => report.balances = balances,
        Err(e) => tracing::error!("Failed to read balances after the tax run: {:?}", e),
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

/// Foreclose a plot whose debt has reached the threshold. Emptying its
/// stations into inventories, releasing it and refunding the deed less the
/// debt happen in one transaction. `None` if,
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
    let returned = return_plot_stations(&mut tx, plot_id).await.map_err(|e| e.to_string())?;
    if !returned.lost.is_empty() {
        tracing::warn!("Foreclosing plot {}: no room for {:?}, lost with its stations", plot_id, returned.lost);
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
    let notice_id = match &terrain_id {
        Some(terrain_id) => {
            let message = ServerMessage::PlotForeclosed { terrain_id: terrain_id.clone(), tax_owed, refund };
            Some(notices::queue(&mut *tx, owner_id, terrain_id, &message).await.map_err(|e| e.to_string())?)
        }
        None => None,
    };
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(Some(Foreclosure {
        plot_id,
        owner_id,
        terrain_id,
        tax_owed,
        refund,
        balance,
        removed_stations,
        returned_to: returned.inventories,
        lost: returned.lost,
        notice_id,
    }))
}

/// Apply a tax run to the game and tell the clients. The owner gets the
/// notices and `plot_foreclosed` if they're online, and last, each owner the
/// run charged or refunded gets their balance (#148). The first two were
/// queued, so an offline owner gets them when they're back (#138); their
/// balance they'll load anyway. Everyone
/// hears about each removed station, and gets `plot_released`. Each player
/// whose inventory took something from the stations gets `inventory_updated`.
pub fn announce(report: TaxReport, stations: &mut HashMap<Uuid, StationState>, events: &broadcast::Sender<GameEvent>) {
    for n in report.notices {
        let _ = events.send(GameEvent::TaxDelinquent {
            owner_id: n.owner_id,
            terrain_id: n.terrain_id,
            tax_owed: n.tax_owed,
            forecloses_at: n.forecloses_at,
            notice_id: n.notice_id,
        });
    }
    for f in report.foreclosures {
        for station_id in f.removed_stations {
            stations.remove(&station_id);
            let _ = events.send(GameEvent::StationRemoved { station_id });
        }
        for (player_id, container_id) in f.returned_to {
            let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
        }
        if let Some(terrain_id) = f.terrain_id {
            let _ = events.send(GameEvent::PlotForeclosed {
                owner_id: f.owner_id,
                terrain_id: terrain_id.clone(),
                tax_owed: f.tax_owed,
                refund: f.refund,
                notice_id: f.notice_id,
            });
            let _ = events.send(GameEvent::PlotReleased { terrain_id });
        }
    }
    announce_balances(events, report.balances);
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
            tx.commit().await.is_ok()
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

/// Pay `amount` strands (all of it if `None`) off the tax `player_id` owes
/// on `plot_id`, as `tax_payment` in the ledger. Paying more than is owed
/// pays what's owed. Returns what's still owed and the new balance.
pub async fn pay_tax_debt(
    db: &PgPool,
    player_id: Uuid,
    plot_id: Uuid,
    amount: Option<i64>,
) -> Result<(i64, i64), AppError> {
    let mut tx = db.begin().await?;
    // The plot before the player, as the hourly run locks them
    let row: Option<(Option<Uuid>, i32)> =
        sqlx::query_as("SELECT owner_id, tax_owed FROM plots WHERE id = $1 FOR UPDATE")
            .bind(plot_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (owner_id, tax_owed) = row.ok_or_else(|| AppError::NotFound("Plot not found".into()))?;
    if owner_id != Some(player_id) {
        return Err(AppError::Forbidden("You don't own this plot".into()));
    }
    let payment = amount.unwrap_or(tax_owed as i64).min(tax_owed as i64);
    if tax_owed <= 0 {
        return Err(AppError::BadRequest("This plot owes no tax".into()));
    }
    if payment <= 0 {
        return Err(AppError::BadRequest("Amount must be positive".into()));
    }
    let new_balance = subtract_strands_in(
        &mut tx,
        player_id,
        payment,
        TransactionType::TaxPayment,
        Some("Paid off property tax owed"),
        Some(plot_id),
    )
    .await
    .map_err(AppError::BadRequest)?;
    let left: i32 = sqlx::query_scalar("UPDATE plots SET tax_owed = tax_owed - $1 WHERE id = $2 RETURNING tax_owed")
        .bind(payment as i32)
        .bind(plot_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!("Player {} paid {} strands of tax on plot {} ({} left)", player_id, payment, plot_id, left);
    Ok((left as i64, new_balance))
}

/// The `#[ignore]`d ones need the local Postgres (`docker-compose up -d`). Run
/// them with `cargo test property_tax -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{balance, ledger, owner, player, plot};

    /// 6000 assessed: 120 a week at 2%, charged 1 in 120 of every 168 hours,
    /// so it's foreclosed at 480 owed
    const VALUE: i32 = 6000;

    /// A carry from which this hour charges [`VALUE`]'s plot 1 strand
    const CARRY_DUE: i32 = 48;

    #[test]
    fn the_threshold_is_four_weeks_of_tax() {
        assert_eq!(weekly_tax(VALUE), 120);
        assert_eq!(largest_hourly_tax(VALUE), 1);
        assert_eq!(foreclosure_threshold(VALUE), 480);
        assert_eq!(foreclosure_threshold(100), 8);
        assert_eq!(charge_hour(weekly_tax(VALUE), CARRY_DUE), (1, 0));
    }

    #[test]
    fn a_week_charges_exactly_the_weekly_tax() {
        // wilderness and urban deeds, one just under the old 1-an-hour floor,
        // one a whole strand an hour, and one with a remainder
        for value in [100, 1000, 8350, 8400, 9000, 168_000] {
            let weekly = weekly_tax(value);
            let mut carry = 0;
            let mut paid = 0;
            for _ in 0..HOURS_PER_WEEK {
                let (charge, next) = charge_hour(weekly, carry);
                assert!(charge <= largest_hourly_tax(value));
                assert!((0..HOURS_PER_WEEK as i32).contains(&next));
                paid += charge;
                carry = next;
            }
            assert_eq!((paid, carry), (weekly, 0), "a deed of {value}");
        }
    }

    #[test]
    fn notices_come_at_each_week_and_a_charge_before() {
        let due: Vec<i64> = (1..=480).filter(|&after| notice_due(after - 1, after, VALUE)).collect();
        assert_eq!(due, vec![120, 240, 360, 479]);
    }

    #[test]
    fn a_big_hour_still_warns_once() {
        // 168000 assessed: 3360 a week, 20 an hour, foreclosed at 13440
        let v = 168_000;
        assert_eq!(largest_hourly_tax(v), 20);
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

    async fn carry(db: &PgPool, lot: Uuid, carry: i32) {
        sqlx::query("UPDATE plots SET tax_carry = $1 WHERE id = $2")
            .bind(carry)
            .bind(lot)
            .execute(db)
            .await
            .unwrap();
    }

    async fn carried(db: &PgPool, lot: Uuid) -> i32 {
        sqlx::query_scalar("SELECT tax_carry FROM plots WHERE id = $1").bind(lot).fetch_one(db).await.unwrap()
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

    /// `player`'s inventory
    async fn inventory(db: &PgPool, player: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(id)
            .bind(player)
            .execute(db)
            .await
            .unwrap();
        id
    }

    /// `quantity` of `kind` at `quality`, owned by `owner`, in `slot` of `container`
    async fn put(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, kind: &str, quality: i32, quantity: i32) {
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(kind)
        .bind(quality)
        .bind(quantity)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
    }

    /// What `container` holds, as `(slot, item_type, quality, quantity, owner)` in slot order
    async fn held(db: &PgPool, container: Uuid) -> Vec<(i32, String, i32, i32, Uuid)> {
        sqlx::query_as(
            "SELECT slot_index, item_type, quality, quantity, owner_id FROM items
             WHERE container_id = $1 ORDER BY slot_index",
        )
        .bind(container)
        .fetch_all(db)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_last_unpaid_hour_forecloses_and_says_so(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let ada_bag = inventory(&db, ada).await;
        let guest = player(&db, "guest", 0).await;
        let guest_bag = inventory(&db, guest).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 479).await;
        carry(&db, lot, CARRY_DUE).await;
        let (chest, chest_box) = station(&db, ada, lot).await;
        put(&db, ada, ada_bag, 0, "fiber", 20, 10).await;
        put(&db, ada, chest_box, 3, "fiber", 50, 30).await;
        put(&db, ada, chest_box, 7, "forge_kit", 64, 1).await;
        put(&db, guest, chest_box, 9, "fiber", 40, 5).await;

        let report = process_property_taxes(&db).await;

        assert!(report.notices.is_empty(), "foreclosed, not warned");
        assert_eq!(report.foreclosures.len(), 1);
        let f = &report.foreclosures[0];
        assert_eq!((f.owner_id, f.tax_owed, f.refund, f.balance), (ada, 480, 5520, 5520));
        assert_eq!(f.removed_stations, vec![chest]);
        assert_eq!(owner(&db, lot).await, Some(None), "the lot is unclaimed again");
        assert_eq!((owed(&db, lot).await, carried(&db, lot).await), (0, 0));
        assert_eq!(balance(&db, ada).await, 5520);
        assert_eq!(ledger(&db, ada).await, vec![(5520, "foreclosure_refund".to_string())]);
        let boxes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM containers WHERE id = $1")
            .bind(chest_box)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(boxes, 0, "the chest went");
        // Its kit and what ada kept in it are ada's again; the guest's fiber is the guest's.
        // The D fiber doesn't join ada's F: the server never blends grades (D2).
        assert_eq!(
            held(&db, ada_bag).await,
            vec![
                (0, "fiber".to_string(), 20, 10, ada),
                (1, "chest_kit".to_string(), 50, 1, ada),
                (2, "fiber".to_string(), 50, 30, ada),
                (3, "forge_kit".to_string(), 64, 1, ada),
            ]
        );
        assert_eq!(held(&db, guest_bag).await, vec![(0, "fiber".to_string(), 40, 5, guest)]);
        assert_eq!(f.returned_to, vec![(ada, ada_bag), (guest, guest_bag)]);
        assert!(f.lost.is_empty());

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
        assert!(matches!(&sent[1], GameEvent::InventoryUpdated { player_id, container_id, .. }
            if *player_id == ada && *container_id == ada_bag));
        assert!(matches!(&sent[2], GameEvent::InventoryUpdated { player_id, container_id, .. }
            if *player_id == guest && *container_id == guest_bag));
        assert!(matches!(&sent[3], GameEvent::PlotForeclosed { owner_id, terrain_id: t, tax_owed: 480, refund: 5520, notice_id: Some(_) }
            if *owner_id == ada && *t == terrain_id));
        assert!(matches!(&sent[4], GameEvent::PlotReleased { terrain_id: t } if *t == terrain_id));
        assert!(matches!(&sent[5], GameEvent::CurrencyChanged { player_id, new_balance: 5520 } if *player_id == ada));
        assert_eq!(sent.len(), 6);

        // Away when it happened, ada hears when she's back (#138)
        assert_eq!(
            notices::take_all(&db, ada).await.unwrap(),
            vec![ServerMessage::PlotForeclosed { terrain_id, tax_owed: 480, refund: 5520 }]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn what_finds_no_room_is_lost_and_the_plot_still_forecloses(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let bag = inventory(&db, ada).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 480).await;
        let (_, chest_box) = station(&db, ada, lot).await;
        // all but one slot full, and no fiber stack to join
        for slot in 0..23 {
            put(&db, ada, bag, slot, "forge_kit", 50, 1).await;
        }
        put(&db, ada, chest_box, 0, "fiber", 50, 30).await;

        let report = process_property_taxes(&db).await;

        // The kit takes the last slot; the fiber goes with the chest.
        let f = &report.foreclosures[0];
        assert_eq!(f.lost, vec![("fiber".to_string(), 30)]);
        assert_eq!(f.returned_to, vec![(ada, bag)]);
        assert_eq!(held(&db, bag).await[23], (23, "chest_kit".to_string(), 50, 1, ada));
        assert_eq!(owner(&db, lot).await, Some(None));
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE item_type = 'fiber'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    /// A tool in a station's tool slot goes back to whoever put it there (E1.6)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_foreclosed_stations_tool_goes_back_to_who_put_it_there(db: PgPool) {
        crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
        let ada = player(&db, "ada", 0).await;
        let ada_bag = inventory(&db, ada).await;
        let guest = player(&db, "guest", 0).await;
        let guest_bag = inventory(&db, guest).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 480).await;
        let (bench, _) = station(&db, ada, lot).await;
        sqlx::query("UPDATE stations SET station_type = 'workbench' WHERE id = $1")
            .bind(bench)
            .execute(&db)
            .await
            .unwrap();
        let sickle: Uuid = sqlx::query_scalar(
            "INSERT INTO items (id, item_type, quality, quantity, durability)
             VALUES ($1, 'stone_sickle', 70, 1, 12) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .fetch_one(&db)
        .await
        .unwrap();
        sqlx::query("UPDATE stations SET installed_tool = $2, tool_owner = $3 WHERE id = $1")
            .bind(bench)
            .bind(sickle)
            .bind(guest)
            .execute(&db)
            .await
            .unwrap();

        let report = process_property_taxes(&db).await;

        let f = &report.foreclosures[0];
        assert_eq!(f.removed_stations, vec![bench]);
        assert_eq!(held(&db, ada_bag).await, vec![(0, "workbench_kit".to_string(), 50, 1, ada)]);
        assert_eq!(held(&db, guest_bag).await, vec![(0, "stone_sickle".to_string(), 70, 1, guest)]);
        assert_eq!(f.returned_to, vec![(ada, ada_bag), (guest, guest_bag)]);
        assert!(f.lost.is_empty());
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
        owe(&db, lot, 119).await;
        carry(&db, lot, CARRY_DUE).await;

        let report = process_property_taxes(&db).await;

        assert!(report.foreclosures.is_empty());
        let terrain_id = format!("test-{lot}");
        assert_eq!(report.notices.len(), 1);
        let n = &report.notices[0];
        assert_eq!((n.owner_id, &n.terrain_id, n.tax_owed, n.forecloses_at), (ada, &terrain_id, 120, 480));
        assert!(n.notice_id.is_some(), "queued for her return");
        assert_eq!(owner(&db, lot).await, Some(Some(ada)));
        assert_eq!(
            notices::take_all(&db, ada).await.unwrap(),
            vec![ServerMessage::TaxDelinquent { terrain_id, tax_owed: 120, forecloses_at: 480 }]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_owner_can_pay_the_debt_off(db: PgPool) {
        let ada = player(&db, "ada", 200).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 150).await;

        assert_eq!(pay_tax_debt(&db, ada, lot, Some(50)).await.unwrap(), (100, 150));
        // All of it, by default; and no more than is owed
        assert_eq!(pay_tax_debt(&db, ada, lot, None).await.unwrap(), (0, 50));
        assert_eq!(owed(&db, lot).await, 0);
        assert_eq!(
            ledger(&db, ada).await,
            vec![(-50, "tax_payment".to_string()), (-100, "tax_payment".to_string())]
        );
        assert!(matches!(pay_tax_debt(&db, ada, lot, None).await, Err(AppError::BadRequest(_))), "nothing owed");

        owe(&db, lot, 30).await;
        assert_eq!(pay_tax_debt(&db, ada, lot, Some(1000)).await.unwrap(), (0, 20));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn paying_tax_takes_nothing_it_cant(db: PgPool) {
        let ada = player(&db, "ada", 40).await;
        let bram = player(&db, "bram", 1000).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 150).await;

        assert!(matches!(pay_tax_debt(&db, ada, lot, None).await, Err(AppError::BadRequest(_))), "can't afford it");
        assert!(matches!(pay_tax_debt(&db, bram, lot, None).await, Err(AppError::Forbidden(_))), "not his");
        assert!(matches!(pay_tax_debt(&db, ada, lot, Some(0)).await, Err(AppError::BadRequest(_))));
        assert!(matches!(pay_tax_debt(&db, ada, Uuid::new_v4(), None).await, Err(AppError::NotFound(_))));
        assert_eq!(owed(&db, lot).await, 150);
        assert_eq!((balance(&db, ada).await, balance(&db, bram).await), (40, 1000));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn strands_pay_off_the_debt_as_far_as_they_reach(db: PgPool) {
        let ada = player(&db, "ada", 101).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 479).await;
        carry(&db, lot, CARRY_DUE).await;

        let report = process_property_taxes(&db).await;

        // 1 for the hour, then the other 100 off the debt
        assert!(report.foreclosures.is_empty() && report.notices.is_empty());
        assert_eq!(balance(&db, ada).await, 0);
        assert_eq!(owed(&db, lot).await, 379);
        assert_eq!(owner(&db, lot).await, Some(Some(ada)));
        assert_eq!(report.balances, vec![(ada, 0)], "she hears her balance (#148)");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_week_of_hours_charges_a_weeks_tax(db: PgPool) {
        let ada = player(&db, "ada", 1000).await;
        let urban = plot(&db, Some(ada), 1000).await;
        let wild = plot(&db, Some(ada), 100).await;

        for _ in 0..HOURS_PER_WEEK {
            process_property_taxes(&db).await;
        }

        // 20 and 2, where rounding each hour up took 168 each
        assert_eq!(balance(&db, ada).await, 1000 - 20 - 2);
        assert_eq!((owed(&db, urban).await, carried(&db, urban).await), (0, 0));
        assert_eq!((owed(&db, wild).await, carried(&db, wild).await), (0, 0));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_hour_that_charges_nothing_still_pays_down_debt(db: PgPool) {
        let ada = player(&db, "ada", 50).await;
        let lot = plot(&db, Some(ada), VALUE).await;
        owe(&db, lot, 30).await;

        let report = process_property_taxes(&db).await;

        assert_eq!(carried(&db, lot).await, 120, "this hour's 120/168 carries");
        assert_eq!((balance(&db, ada).await, owed(&db, lot).await), (20, 0));
        assert_eq!(report.balances, vec![(ada, 20)]);
    }
}
