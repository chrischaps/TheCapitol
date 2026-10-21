//! Grants (E3.8): Strands and goods from nowhere, to fund a market maker on a
//! local stack. `POST /player/grant` serves it only when the server has
//! `DEV_GRANTS=on`, which no build turns on by itself: a grant prints money,
//! so a shared server never has it without someone choosing to.
//!
//! The Strands go through the ledger as an `admin_adjustment`, so the audit
//! still explains every balance. The goods go to the market box, where a sell
//! order can take them straight from.

use capitol_protocol::rest::{GrantItem, GrantRequest, MAX_GRANT_LINES, MAX_GRANT_STRANDS};
use capitol_protocol::rules::market::MAX_ORDER_QUANTITY;
use capitol_protocol::rules::stacking::{Units, MAX_QUALITY, MIN_QUALITY};
use sqlx::PgPool;
use uuid::Uuid;

use super::currency::{add_strands_in, TransactionType};
use super::market::{market_box, stow};
use super::units::{insert_stack, Marked, NewStack};
use crate::models::{ItemType, ITEM_TYPE_COLUMNS};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GrantError {
    #[error("Strands must be 0 to {MAX_GRANT_STRANDS}")]
    BadStrands,
    #[error("At most {MAX_GRANT_LINES} lines a grant")]
    TooManyLines,
    #[error("{0}: quantity must be 1 to {MAX_ORDER_QUANTITY} and quality {MIN_QUALITY} to {MAX_QUALITY}")]
    BadLine(String),
    #[error("No such item type: {0}")]
    UnknownItem(String),
    #[error("The market box is full")]
    MarketBoxFull,
    #[error("{0}")]
    Db(String),
}

impl From<sqlx::Error> for GrantError {
    fn from(e: sqlx::Error) -> Self {
        GrantError::Db(e.to_string())
    }
}

/// What a grant left: the balance, and the market box the goods went to.
#[derive(Debug, Clone, PartialEq)]
pub struct Granted {
    pub strands: i64,
    pub market_box: Uuid,
}

/// Give `player_id` the request's Strands and goods, all or nothing.
pub async fn grant(db: &PgPool, player_id: Uuid, request: &GrantRequest) -> Result<Granted, GrantError> {
    if !(0..=MAX_GRANT_STRANDS).contains(&request.strands) {
        return Err(GrantError::BadStrands);
    }
    if request.items.len() > MAX_GRANT_LINES {
        return Err(GrantError::TooManyLines);
    }
    for GrantItem { item_type, quantity, quality } in &request.items {
        if !(1..=MAX_ORDER_QUANTITY).contains(quantity) || !(MIN_QUALITY..=MAX_QUALITY).contains(quality) {
            return Err(GrantError::BadLine(item_type.clone()));
        }
    }

    let mut tx = db.begin().await?;
    let strands = if request.strands > 0 {
        add_strands_in(
            &mut tx,
            player_id,
            request.strands,
            TransactionType::AdminAdjustment,
            Some("dev grant"),
            None,
        )
        .await
        .map_err(GrantError::Db)?
    } else {
        sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(&mut *tx)
            .await?
    };
    let into = market_box(&mut tx, player_id).await?;
    let mut flows = super::ledger::Flows::new();
    for line in &request.items {
        let item_type: Option<ItemType> =
            sqlx::query_as(&format!("SELECT {ITEM_TYPE_COLUMNS} FROM item_types WHERE id = $1"))
                .bind(&line.item_type)
                .fetch_optional(&mut *tx)
                .await?;
        let item_type = item_type.ok_or_else(|| GrantError::UnknownItem(line.item_type.clone()))?;
        // A tool doesn't stack: one of its own per unit, each with its wear
        let (stacks, per_stack) = if item_type.stackable {
            (1, line.quantity)
        } else {
            (line.quantity, 1)
        };
        for _ in 0..stacks {
            let stack = NewStack {
                item_type: &item_type.id,
                owner_id: Some(player_id),
                container_id: None,
                slot_index: None,
                durability: item_type.durability_at(line.quality),
            };
            // Granted goods carry no mark (#231): no one made them
            let id = insert_stack(&mut tx, stack, &Marked::by(None, &Units::uniform(per_stack, line.quality))).await?;
            // Returning drops `tx`, which rolls the whole grant back
            if stow(&mut tx, player_id, id, into).await?.is_none() {
                return Err(GrantError::MarketBoxFull);
            }
        }
        flows.add(&item_type.id, capitol_protocol::ledger::UnitFlow::Granted, line.quantity as i64);
    }
    super::ledger::commit(tx, flows).await?;
    tracing::info!("Player {player_id} granted {} Strands and {} lines", request.strands, request.items.len());
    Ok(Granted { strands, market_box: into })
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test grants -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::market::tests::{balance, player};
    use crate::engine::units::test_units::all_in;

    fn line(item_type: &str, quantity: i32, quality: i32) -> GrantItem {
        GrantItem {
            item_type: item_type.into(),
            quantity,
            quality,
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_grant_pays_through_the_ledger_and_fills_the_market_box(db: PgPool) {
        crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
        let ada = player(&db, "ada", 10).await;
        let request = GrantRequest {
            strands: 500,
            items: vec![line("plank", 40, 60), line("plank", 5, 80), line("stone_axe", 2, 50)],
        };
        let granted = grant(&db, ada.id, &request).await.unwrap();
        assert_eq!(granted.strands, 510);
        assert_eq!(balance(&db, ada.id).await, 510);
        let ledger: Vec<(i64, String)> =
            sqlx::query_as("SELECT amount, transaction_type FROM currency_transactions WHERE player_id = $1")
                .bind(ada.id)
                .fetch_all(&db)
                .await
                .unwrap();
        assert_eq!(ledger, [(500, "admin_adjustment".to_string())]);
        // The second plank line joins the first stack, keeping its units (D2)
        assert_eq!(all_in(&db, granted.market_box, "plank").await, Units::from_pairs([(60, 40), (80, 5)]));
        let axes: Vec<Option<i32>> = sqlx::query_scalar(
            "SELECT durability FROM items WHERE container_id = $1 AND item_type = 'stone_axe'",
        )
        .bind(granted.market_box)
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(axes.len(), 2);
        assert!(axes.iter().all(|d| d.is_some_and(|d| d > 0)), "{axes:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_bad_grant_gives_nothing(db: PgPool) {
        crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
        let ada = player(&db, "ada", 10).await;
        let unknown = GrantRequest {
            strands: 500,
            items: vec![line("plank", 4, 60), line("unobtainium", 1, 50)],
        };
        assert_eq!(
            grant(&db, ada.id, &unknown).await,
            Err(GrantError::UnknownItem("unobtainium".into()))
        );
        assert_eq!(balance(&db, ada.id).await, 10);
        let held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE owner_id = $1")
            .bind(ada.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(held, 0);

        let too_rich = GrantRequest {
            strands: MAX_GRANT_STRANDS + 1,
            items: Vec::new(),
        };
        assert_eq!(grant(&db, ada.id, &too_rich).await, Err(GrantError::BadStrands));
        let bad_quality = GrantRequest {
            strands: 0,
            items: vec![line("plank", 1, 101)],
        };
        assert_eq!(grant(&db, ada.id, &bad_quality).await, Err(GrantError::BadLine("plank".into())));
        // Too many stacks for the box's 48 slots
        let too_many = GrantRequest {
            strands: 0,
            items: vec![line("stone_axe", 49, 50)],
        };
        assert_eq!(grant(&db, ada.id, &too_many).await, Err(GrantError::MarketBoxFull));
    }
}
