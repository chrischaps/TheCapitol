//! Notices that wait for an offline player (#138).
//!
//! A WebSocket event only reaches a player who's connected when it's sent, so
//! an owner who was away when their plot fell behind on tax, or was
//! foreclosed, would never hear why. Those messages are queued in
//! `player_notices` in the same transaction as what they report. The owner's
//! connection [`take`]s each one as it sends it live; whatever is left is
//! sent by [`take_all`] when they next subscribe. Taking deletes the row, so
//! a notice goes out once whichever way it goes.
//!
//! A `tax_delinquent` that waited is out of date by the time it's sent: the
//! debt may have grown or been paid, or the plot gone. So only the latest per
//! lot is kept, and it's sent as the plot stands now, or not at all.

use capitol_protocol::ws::ServerMessage;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use super::property_tax::foreclosure_threshold;

/// Queue `message` for `player_id`, about the lot `terrain_id` (or, for
/// `automation_paused`, the station's id). Any unsent `tax_delinquent` or
/// `automation_paused` about the same thing is dropped: this says more.
/// Returns the notice's id, for the live event to carry.
pub async fn queue<'e>(
    db: impl PgExecutor<'e>,
    player_id: Uuid,
    terrain_id: &str,
    message: &ServerMessage,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "WITH superseded AS (
             DELETE FROM player_notices
             WHERE player_id = $2 AND terrain_id = $3 AND kind IN ('tax_delinquent', 'automation_paused')
         )
         INSERT INTO player_notices (id, player_id, kind, terrain_id, message)
         VALUES ($1, $2, $4, $3, $5)",
    )
    .bind(id)
    .bind(player_id)
    .bind(terrain_id)
    .bind(message.type_name())
    .bind(serde_json::to_string(message).expect("a ServerMessage serialises"))
    .execute(db)
    .await?;
    Ok(id)
}

/// Take the notice `id` to send it live. `false` if it's gone: sent already,
/// or replaced by a newer one.
pub async fn take(db: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let taken = sqlx::query("DELETE FROM player_notices WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(taken.rows_affected() > 0)
}

/// Take every notice waiting for `player_id`, oldest first, as the messages
/// to send. A `tax_delinquent` is rebuilt from the plot as it is now, at the
/// tax `rate` now, and dropped if the plot isn't theirs any more or owes nothing.
pub async fn take_all(db: &PgPool, player_id: Uuid, rate: f64) -> Result<Vec<ServerMessage>, sqlx::Error> {
    let rows: Vec<(String, Option<String>, String, Option<i32>, Option<i32>)> = sqlx::query_as(
        "WITH taken AS (
             DELETE FROM player_notices WHERE player_id = $1
             RETURNING kind, terrain_id, message, created_at
         )
         SELECT t.kind, t.terrain_id, t.message, p.tax_owed, p.assessed_value
         FROM taken t
         LEFT JOIN plots p
             ON t.kind = 'tax_delinquent' AND p.terrain_id = t.terrain_id AND p.owner_id = $1
         ORDER BY t.created_at",
    )
    .bind(player_id)
    .fetch_all(db)
    .await?;

    let mut out = Vec::new();
    for (kind, terrain_id, message, tax_owed, assessed_value) in rows {
        if kind == "tax_delinquent" {
            if let (Some(terrain_id), Some(owed), Some(value)) = (terrain_id, tax_owed, assessed_value) {
                if owed > 0 {
                    out.push(ServerMessage::TaxDelinquent {
                        terrain_id,
                        tax_owed: owed as i64,
                        forecloses_at: foreclosure_threshold(value, rate),
                    });
                }
            }
            continue;
        }
        match serde_json::from_str(&message) {
            Ok(m) => out.push(m),
            Err(e) => tracing::error!("Dropping a {kind} notice for {player_id} that no longer parses: {e}"),
        }
    }
    Ok(out)
}

/// The `#[ignore]`d ones need the local Postgres (`docker-compose up -d`). Run
/// them with `cargo test notices -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{plot, player};

    fn delinquent(terrain_id: &str, tax_owed: i64) -> ServerMessage {
        ServerMessage::TaxDelinquent { terrain_id: terrain_id.into(), tax_owed, forecloses_at: 1 }
    }

    async fn owe(db: &PgPool, lot: Uuid, owed: i32) {
        sqlx::query("UPDATE plots SET tax_owed = $1 WHERE id = $2").bind(owed).bind(lot).execute(db).await.unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_notice_sent_live_isnt_sent_again(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let gone = ServerMessage::PlotForeclosed { terrain_id: "lot".into(), tax_owed: 80, refund: 20, impounded: vec![], lost: vec![] };
        let id = queue(&db, ada, "lot", &gone).await.unwrap();

        assert!(take(&db, id).await.unwrap());
        assert!(!take(&db, id).await.unwrap(), "taken once");
        assert!(take_all(&db, ada, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn what_waited_is_sent_on_return_and_only_once(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let bram = player(&db, "bram", 0).await;
        let gone = ServerMessage::PlotForeclosed { terrain_id: "lot".into(), tax_owed: 80, refund: 20, impounded: vec![], lost: vec![] };
        queue(&db, ada, "lot", &gone).await.unwrap();

        assert!(take_all(&db, bram, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap().is_empty(), "only to its player");
        assert_eq!(take_all(&db, ada, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap(), vec![gone]);
        assert!(take_all(&db, ada, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap().is_empty());
    }

    /// Two weeks' notices and then the foreclosure: only the foreclosure is left
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_newer_notice_replaces_an_unsent_warning(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let lot = plot(&db, Some(ada), 1000).await;
        let terrain_id = format!("test-{lot}");
        let first = queue(&db, ada, &terrain_id, &delinquent(&terrain_id, 20)).await.unwrap();
        queue(&db, ada, &terrain_id, &delinquent(&terrain_id, 40)).await.unwrap();
        queue(&db, ada, "elsewhere", &delinquent("elsewhere", 20)).await.unwrap();
        let gone = ServerMessage::PlotForeclosed { terrain_id: terrain_id.clone(), tax_owed: 80, refund: 920, impounded: vec![], lost: vec![] };
        queue(&db, ada, &terrain_id, &gone).await.unwrap();

        assert!(!take(&db, first).await.unwrap(), "replaced");
        // "elsewhere" is no plot of hers, so its warning is dropped
        assert_eq!(take_all(&db, ada, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap(), vec![gone]);
    }

    /// The warning says what's owed now, and nothing once it's paid
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_warning_that_waited_says_what_is_owed_now(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let lot = plot(&db, Some(ada), 1000).await;
        let terrain_id = format!("test-{lot}");

        queue(&db, ada, &terrain_id, &delinquent(&terrain_id, 20)).await.unwrap();
        owe(&db, lot, 33).await;
        assert_eq!(
            take_all(&db, ada, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap(),
            vec![ServerMessage::TaxDelinquent {
                terrain_id: terrain_id.clone(),
                tax_owed: 33,
                forecloses_at: foreclosure_threshold(1000, capitol_protocol::tunables::PROPERTY_TAX_RATE),
            }]
        );

        queue(&db, ada, &terrain_id, &delinquent(&terrain_id, 20)).await.unwrap();
        owe(&db, lot, 0).await;
        assert!(take_all(&db, ada, capitol_protocol::tunables::PROPERTY_TAX_RATE).await.unwrap().is_empty(), "paid off");
    }
}
