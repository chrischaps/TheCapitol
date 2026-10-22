//! The deed grant (O0.3, #440, `the-capitol-road-in.md` §5 D4 to D6): the
//! road in's reward. Walking every chapter before the Deeds Office opens one
//! grant for the account (`Steps::deed_earned`), a free claim on any rural,
//! suburban or urban lot the player chooses. It isn't an item, so it can't
//! be traded, sold or given.
//!
//! - [`open`] is called by `tutorial::saw` the moment the road is walked. A
//!   claim that finds no grant opens it then, if the steps say it's earned,
//!   so a lost open never costs the player their deed ([`take_in`]).
//! - A granted plot is assessed at the zone's live deed price and taxed as a
//!   bought one (D4). Nothing was paid, so a foreclosure refunds nothing
//!   (`property_tax::foreclose`), and a retired lot hands the grant back
//!   unused rather than paying out ([`hand_back_in`], from `plot_sync`) (D5).
//! - The books count grants as land given, outside the Strands' ([`books`]).

use capitol_protocol::ledger::DeedGrants;
use capitol_protocol::rules::Zone;
use capitol_protocol::tutorial::Steps;
use sqlx::PgConnection;
use uuid::Uuid;

/// Why a claim with the grant was refused
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The road in isn't walked yet
    NotEarned,
    /// Spent already, on a plot still held or foreclosed
    Used,
    /// Not a home zone: the guild district, say (D6)
    NotAHomeZone,
}

impl Refused {
    pub fn message(self) -> &'static str {
        match self {
            Refused::NotEarned => "You haven't earned your deed yet: walk the road in to the Deeds Office",
            Refused::Used => "You've already claimed a lot with your deed",
            Refused::NotAHomeZone => "Your deed claims a rural, suburban or urban lot",
        }
    }
}

/// The zones a grant can claim in (D6)
pub fn home_zone(zone: Zone) -> bool {
    matches!(zone, Zone::Rural | Zone::Suburban | Zone::Urban)
}

/// Open `player`'s account's grant, if it hasn't one. True if this opened it.
pub async fn open(conn: &mut PgConnection, player: Uuid) -> Result<bool, sqlx::Error> {
    let opened = sqlx::query(
        "INSERT INTO deed_grants (account_id, player_id)
         SELECT account_id, id FROM players WHERE id = $1
         ON CONFLICT (account_id) DO NOTHING",
    )
    .bind(player)
    .execute(conn)
    .await?;
    Ok(opened.rows_affected() == 1)
}

/// Lock `player`'s grant for a claim, inside the claim's transaction, and
/// check it can be spent. A grant the steps have earned but that was never
/// opened is opened here.
pub async fn take_in(conn: &mut PgConnection, player: Uuid) -> Result<Result<(), Refused>, sqlx::Error> {
    let steps: Option<i64> = sqlx::query_scalar("SELECT steps FROM tutorial_progress WHERE player_id = $1")
        .bind(player)
        .fetch_optional(&mut *conn)
        .await?;
    if steps.is_some_and(|steps| Steps::from_bits(steps as u64).deed_earned()) {
        open(conn, player).await?;
    }
    let used: Option<Option<chrono::DateTime<chrono::Utc>>> = sqlx::query_scalar(
        "SELECT g.used_at FROM deed_grants g JOIN players p ON p.account_id = g.account_id
         WHERE p.id = $1
         FOR UPDATE OF g",
    )
    .bind(player)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match used {
        None => Err(Refused::NotEarned),
        Some(Some(_)) => Err(Refused::Used),
        Some(None) => Ok(()),
    })
}

/// Spend `player`'s grant, locked by [`take_in`], on `plot`
pub async fn use_in(conn: &mut PgConnection, player: Uuid, plot: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE deed_grants SET used_at = NOW(), plot_id = $2
         WHERE account_id = (SELECT account_id FROM players WHERE id = $1)",
    )
    .bind(player)
    .bind(plot)
    .execute(conn)
    .await?;
    Ok(())
}

/// A granted `plot` of `owner`'s is being released because its lot is
/// retired, or an admin forced a commit over it: hand the grant back unused,
/// in the release's transaction, so it can be spent again. True if one was.
pub async fn hand_back_in(conn: &mut PgConnection, owner: Uuid, plot: Uuid) -> Result<bool, sqlx::Error> {
    let returned = sqlx::query(
        "UPDATE deed_grants SET used_at = NULL, plot_id = NULL, returned_at = NOW()
         WHERE player_id = $1 AND plot_id = $2 AND used_at IS NOT NULL",
    )
    .bind(owner)
    .bind(plot)
    .execute(conn)
    .await?;
    Ok(returned.rows_affected() == 1)
}

/// The grants as they stand, for the ledger. Read in its snapshot.
pub async fn books(conn: &mut PgConnection) -> Result<DeedGrants, sqlx::Error> {
    let (opened, used, returned): (i64, i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COUNT(used_at), COUNT(returned_at) FROM deed_grants")
            .fetch_one(&mut *conn)
            .await?;
    let granted_value: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(assessed_value), 0)::BIGINT FROM plots WHERE granted")
            .fetch_one(&mut *conn)
            .await?;
    Ok(DeedGrants { opened: opened as u64, used: used as u64, returned: returned as u64, granted_value })
}


/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test deed_grants -- --ignored`. The helpers are shared with the
/// claim's, foreclosure's and the terrain sync's tests.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{player, plot};
    use crate::engine::tutorial::{self, Did};
    use capitol_protocol::tutorial::{Chapter, Step};
    use sqlx::PgPool;

    /// Every step of the chapters before the Deeds Office: the road walked
    pub(crate) fn the_road() -> Steps {
        Chapter::ALL.into_iter().take_while(|c| *c != Chapter::DeedsOffice).flat_map(|c| c.steps()).collect()
    }

    /// Set `player`'s steps, as if they'd walked them
    pub(crate) async fn walked(db: &PgPool, player: Uuid, steps: Steps) {
        sqlx::query(
            "INSERT INTO tutorial_progress (player_id, steps) VALUES ($1, $2)
             ON CONFLICT (player_id) DO UPDATE SET steps = $2",
        )
        .bind(player)
        .bind(steps.bits() as i64)
        .execute(db)
        .await
        .unwrap();
    }

    /// `player`'s grant as (used, plot, returned), or `None` if they have none
    pub(crate) async fn grant_of(db: &PgPool, player: Uuid) -> Option<(bool, Option<Uuid>, bool)> {
        sqlx::query_as(
            "SELECT used_at IS NOT NULL, plot_id, returned_at IS NOT NULL FROM deed_grants WHERE player_id = $1",
        )
        .bind(player)
        .fetch_optional(db)
        .await
        .unwrap()
    }

    /// `owner`'s grant, spent on their `plot`, as a claim would leave it
    pub(crate) async fn granted(db: &PgPool, owner: Uuid, plot: Uuid) {
        let mut conn = db.acquire().await.unwrap();
        assert!(open(&mut conn, owner).await.unwrap());
        use_in(&mut conn, owner, plot).await.unwrap();
        sqlx::query("UPDATE plots SET granted = TRUE WHERE id = $1").bind(plot).execute(&mut *conn).await.unwrap();
    }

    #[test]
    fn a_grant_claims_only_in_the_home_zones() {
        let homes: Vec<Zone> = Zone::ALL.into_iter().filter(|z| home_zone(*z)).collect();
        assert_eq!(homes, [Zone::Urban, Zone::Suburban, Zone::Rural]);
        assert!(!home_zone(Zone::GuildDistrict));
        assert!(the_road().deed_earned() && !Steps::NONE.deed_earned());
    }

    /// The step that finishes the road opens the grant, once; the steps
    /// before it don't
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn walking_the_road_in_opens_the_grant(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let almost = Steps::from_bits(the_road().bits() & !Step::Walk.bit());
        walked(&db, ada, Steps::from_bits(almost.bits() & !Step::GatherGrass.bit())).await;

        tutorial::saw(&db, &events, ada, Did::Gathered("fiber")).await;
        assert_eq!(grant_of(&db, ada).await, None, "a step short");
        assert_eq!(tutorial::progress(&db, ada).await.unwrap().grant, capitol_protocol::tutorial::Grant::Unearned);
        tutorial::saw(&db, &events, ada, Did::Walked).await;
        assert_eq!(grant_of(&db, ada).await, Some((false, None, false)), "open, and not spent");
        // And the client hears it open (#448)
        assert_eq!(tutorial::progress(&db, ada).await.unwrap().grant, capitol_protocol::tutorial::Grant::Open);
        tutorial::saw(&db, &events, ada, Did::Walked).await;
        let grants: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM deed_grants").fetch_one(&db).await.unwrap();
        assert_eq!(grants, 1);
    }

    /// The books count the grants as they stand, and the land they hold at
    /// its assessed value, with no Strand among them
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_books_count_the_grants(db: PgPool) {
        let (ada, bo, cy) = (player(&db, "ada", 0).await, player(&db, "bo", 0).await, player(&db, "cy", 0).await);
        let lot = plot(&db, Some(ada), 8600).await;
        granted(&db, ada, lot).await;
        let mut conn = db.acquire().await.unwrap();
        open(&mut conn, bo).await.unwrap();
        let back = plot(&db, Some(cy), 17000).await;
        granted(&db, cy, back).await;
        assert!(hand_back_in(&mut conn, cy, back).await.unwrap());
        sqlx::query("UPDATE plots SET granted = FALSE, owner_id = NULL WHERE id = $1").bind(back).execute(&mut *conn).await.unwrap();

        let books = books(&mut conn).await.unwrap();
        assert_eq!(books, DeedGrants { opened: 3, used: 1, returned: 1, granted_value: 8600 });
        let ledger = crate::engine::ledger::books(&db).await.unwrap();
        assert_eq!(ledger.grants, books, "and /admin/ledger shows them");
    }
}
