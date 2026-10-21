//! Citizens (C0.1, Citizens D2 and D3): AI players who live in the world,
//! run by `capitol-town` over the same protocol any client uses.
//!
//! A citizen is an account of kind `citizen`, made only by an admin
//! (`POST /admin/citizens`). Registration only ever makes a person, and the
//! database refuses to change an account's kind
//! (`20261120_001_citizens.sql`), so the mark can't be put on or taken off.
//! Everyone else sees a citizen flagged: `citizen` on `positions` and
//! `player_joined`, and wherever else a player is named to others.
//!
//! A citizen starts as a new person does, with no grants and no special rules.
//! It has no password anyone knows; its `play`+`trade` key, shown once when
//! it's made, is the only way in.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use capitol_protocol::rest::{AccountKind, ApiKeyScope, CitizenInfo, CreatedCitizen};

use crate::api_keys;
use crate::error::AppError;
use crate::routes::auth::{create_account, hash_password};

/// The longest name a citizen may have.
pub const MAX_NAME_LEN: usize = 32;

/// What a citizen's key may do: everything a player does.
const SCOPES: [ApiKeyScope; 2] = [ApiKeyScope::Play, ApiKeyScope::Trade];

/// Make the citizen `name`: its account, its player at the spawn and its key.
pub async fn mint(db: &PgPool, name: &str) -> Result<CreatedCitizen, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
        return Err(AppError::BadRequest(format!("A citizen's name is 1 to {MAX_NAME_LEN} characters")));
    }
    // Everyone tells citizens apart by name, so a citizen takes no name a player has
    let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM players WHERE lower(name) = lower($1))")
        .bind(name)
        .fetch_one(db)
        .await?;
    if taken {
        return Err(AppError::Conflict(format!("A player is already called {name}")));
    }

    // Nobody signs in by password: it's random and never shown
    let password_hash = hash_password(&crate::sessions::random_token())?;
    let email = format!("citizen-{}@citizens.invalid", Uuid::new_v4().simple());
    let mut tx = db.begin().await?;
    let (account_id, player_id) = create_account(&mut tx, &email, &password_hash, name, AccountKind::Citizen).await?;
    let (key, secret) = api_keys::mint(&mut *tx, account_id, "citizen", &SCOPES).await?;
    let created_at: DateTime<Utc> = sqlx::query_scalar("SELECT created_at FROM accounts WHERE id = $1")
        .bind(account_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(CreatedCitizen {
        citizen: CitizenInfo {
            account_id,
            player_id,
            name: name.to_string(),
            created_at,
            key: Some(key),
        },
        secret,
    })
}

/// Every citizen, oldest first, each with its newest live key (no secrets).
pub async fn list(db: &PgPool) -> Result<Vec<CitizenInfo>, sqlx::Error> {
    type Key = (Option<Uuid>, Option<String>, Option<Vec<String>>, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>);
    type Row = (Uuid, Uuid, String, DateTime<Utc>, Option<Uuid>, Option<String>, Option<Vec<String>>, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT a.id, p.id, p.name, a.created_at,
                k.id, k.name, k.scopes, k.prefix, k.created_at, k.last_used_at
         FROM accounts a
         JOIN players p ON p.account_id = a.id
         LEFT JOIN LATERAL (
             SELECT * FROM api_keys
             WHERE account_id = a.id AND revoked_at IS NULL
             ORDER BY created_at DESC LIMIT 1
         ) k ON TRUE
         WHERE a.kind = 'citizen'
         ORDER BY a.created_at, a.id",
    )
    .fetch_all(db)
    .await?;
    let key = |(id, name, scopes, prefix, created_at, last_used_at): Key| {
        Some(api_keys::info((id?, name?, scopes?, prefix?, created_at?, last_used_at, None)))
    };
    Ok(rows
        .into_iter()
        .map(|(account_id, player_id, name, created_at, k0, k1, k2, k3, k4, k5)| CitizenInfo {
            account_id,
            player_id,
            name,
            created_at,
            key: key((k0, k1, k2, k3, k4, k5)),
        })
        .collect())
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test citizens -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_keys::Scopes;

    async fn person(db: &PgPool, name: &str) -> (Uuid, Uuid) {
        let mut tx = db.begin().await.unwrap();
        let ids = create_account(&mut tx, &format!("{name}@test.local"), "x", name, AccountKind::Human)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        ids
    }

    /// The player as the game sees it: its balance, its speed and its containers
    async fn start_of(db: &PgPool, player_id: Uuid) -> (i64, f32, Vec<String>) {
        let (strands, speed): (i64, f32) = sqlx::query_as("SELECT strand_balance, speed FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(db)
            .await
            .unwrap();
        let containers: Vec<String> =
            sqlx::query_scalar("SELECT container_type FROM containers WHERE owner_id = $1 ORDER BY container_type")
                .bind(player_id)
                .fetch_all(db)
                .await
                .unwrap();
        (strands, speed, containers)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_citizen_starts_as_a_person_does_and_signs_in_by_its_key(db: PgPool) {
        let (ada_account, ada) = person(&db, "ada").await;
        let made = mint(&db, "  Wren Cooper ").await.unwrap();
        let wren = &made.citizen;
        assert_eq!(wren.name, "Wren Cooper", "trimmed");

        let kind: String = sqlx::query_scalar("SELECT kind FROM accounts WHERE id = $1")
            .bind(wren.account_id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(kind, "citizen");
        assert_eq!(start_of(&db, wren.player_id).await, start_of(&db, ada).await, "no grants, nothing special");

        let user = api_keys::authenticate(&db, &made.secret).await.unwrap().live().expect("its key signs it in, with no subscription");
        assert_eq!(user.account_id, wren.account_id);
        assert_eq!(user.grant.scopes, Scopes::of(&SCOPES));

        let player = crate::ws::load_player(&db, wren.account_id).await.unwrap();
        assert!(player.citizen, "and the game marks it");
        assert!(!crate::ws::load_player(&db, ada_account).await.unwrap().citizen);

        let roster = list(&db).await.unwrap();
        assert_eq!(roster.len(), 1, "people aren't on it");
        assert_eq!(roster[0].player_id, wren.player_id);
        assert_eq!(roster[0].key.as_ref().map(|k| k.id), wren.key.as_ref().map(|k| k.id));

        sqlx::query("UPDATE api_keys SET revoked_at = NOW() WHERE account_id = $1")
            .bind(wren.account_id)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(list(&db).await.unwrap()[0].key, None, "a revoked key isn't shown");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_citizen_needs_a_name_nobody_has(db: PgPool) {
        person(&db, "Ada").await;
        assert!(matches!(mint(&db, "ada").await, Err(AppError::Conflict(_))), "any case");
        assert!(matches!(mint(&db, "   ").await, Err(AppError::BadRequest(_))));
        assert!(matches!(mint(&db, &"w".repeat(MAX_NAME_LEN + 1)).await, Err(AppError::BadRequest(_))));
        mint(&db, "Wren").await.unwrap();
        assert!(matches!(mint(&db, "Wren").await, Err(AppError::Conflict(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_account_never_changes_kind(db: PgPool) {
        let (ada, _) = person(&db, "ada").await;
        let wren = mint(&db, "Wren").await.unwrap().citizen;
        for (account, to) in [(ada, "citizen"), (wren.account_id, "human")] {
            let changed = sqlx::query("UPDATE accounts SET kind = $2 WHERE id = $1")
                .bind(account)
                .bind(to)
                .execute(&db)
                .await;
            assert!(changed.is_err(), "to {to}");
        }
        // Other changes to the account still go through
        sqlx::query("UPDATE accounts SET last_login = NOW() WHERE id = $1")
            .bind(wren.account_id)
            .execute(&db)
            .await
            .unwrap();
    }
}
