//! The conservation harness (commons plan §5, S0.2): the checks every flow
//! that holds Strands or units for someone runs its seeded lives against.
//! Lifted from the market's test, so commissions (S2.1), wages (S2.6) and
//! guild treasuries (S4.2) each add their own flows here, not a copy:
//!
//! - [`strands_accounted`]: Strands in balances, held for someone and kept in
//!   fees are what there were at the start, after every step
//! - [`units_of_type`] and [`every_unit`]: every unit of a type, wherever it
//!   is, and stack by stack with each one's summaries checked
//! - [`balances_of`] and [`assert_audited`]: every balance is its start plus
//!   its audit rows
//!
//! Test-only. These need the local Postgres (`docker-compose up -d`).

use std::collections::BTreeMap;

use capitol_protocol::rules::stacking::Units;
use sqlx::PgPool;
use uuid::Uuid;

use super::ledger::held_strands;
use super::units::test_units::units;

/// Strands in players' balances, held for someone (the ledger's
/// [`held_strands`]), and kept in fees: the total there is. Every flow
/// conserves it, so it's the same after any step as before.
pub(crate) async fn strands_accounted(db: &PgPool) -> i64 {
    let mut conn = db.acquire().await.unwrap();
    let balances: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(strand_balance), 0)::BIGINT FROM players")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    balances + held_strands(&mut conn).await.unwrap() + fees_kept(db).await
}

/// The fees each flow's own rows say it kept: the market's listing fees and
/// the court's on commissions (S2.1). A flow with a fee adds its sum here
/// (wages', S2.6), so a fee charged twice, or a fee with no row, shows as
/// Strands made or lost.
async fn fees_kept(db: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT (SELECT COALESCE(SUM(fee), 0) FROM market_orders)::BIGINT
              + (SELECT COALESCE(SUM(fee), 0) FROM commissions)::BIGINT",
    )
    .fetch_one(db)
    .await
    .unwrap()
}

/// Every unit of `item_type` there is, held or in escrow, in one query
pub(crate) async fn units_of_type(db: &PgPool, item_type: &str) -> Units {
    let rows: Vec<(i16, i64)> = sqlx::query_as(
        "SELECT u.quality, SUM(u.count)::BIGINT FROM item_units u JOIN items i ON i.id = u.item_id
         WHERE i.item_type = $1 GROUP BY u.quality",
    )
    .bind(item_type)
    .fetch_all(db)
    .await
    .unwrap();
    Units::from_pairs(rows.into_iter().map(|(q, c)| (q as i32, c as i32)))
}

/// [`units_of_type`] stack by stack, checking each one's summaries match its units
pub(crate) async fn every_unit(db: &PgPool, item_type: &str) -> Units {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM items WHERE item_type = $1")
        .bind(item_type)
        .fetch_all(db)
        .await
        .unwrap();
    let mut all = Units::new();
    for id in ids {
        all.merge(&units(db, id).await);
    }
    all
}

/// The balances of `players` now, for [`assert_audited`] at the end
pub(crate) async fn balances_of(db: &PgPool, players: impl IntoIterator<Item = Uuid>) -> BTreeMap<Uuid, i64> {
    let mut balances = BTreeMap::new();
    for player in players {
        let balance: i64 = sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
            .bind(player)
            .fetch_one(db)
            .await
            .unwrap();
        balances.insert(player, balance);
    }
    balances
}

/// Every balance in `start` is now its start plus its audit rows: nothing
/// moved a player's Strands without writing it down.
pub(crate) async fn assert_audited(db: &PgPool, start: &BTreeMap<Uuid, i64>) {
    let now = balances_of(db, start.keys().copied()).await;
    for (player, start) in start {
        let audited: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0)::BIGINT FROM currency_transactions WHERE player_id = $1")
                .bind(player)
                .fetch_one(db)
                .await
                .unwrap();
        assert_eq!(now[player], start + audited, "player {player}'s balance isn't its audit trail");
    }
}
