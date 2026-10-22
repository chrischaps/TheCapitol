//! Other people (S0.3): a person's card, who they are, as anyone may see it.
//!
//! `/player` is you; `/players` is everyone. A card says only what anyone
//! in the city could know: the name, whether they're a citizen, how long
//! they've been here, where they live and what's said of them. Nothing of
//! their Strands or goods.
//!
//! Mutes (S1.4) are here too: `PUT` and `DELETE /players/{id}/mute`, and
//! `GET /player/mutes` beside the rest of you. `crate::mutes` keeps them.

use axum::{
    extract::{Path, State},
    routing::{get, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use capitol_protocol::rest::{MutesResponse, PlayerCard};
use capitol_protocol::rules::{self, standing};

use crate::engine::tutorial::{self, Did};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{id}/card", get(get_card))
        .route("/by-name/{name}", get(get_card_by_name))
        .route("/{id}/mute", put(mute).delete(unmute))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Someone's card: who they are
#[utoipa::path(
    get,
    path = "/players/{id}/card",
    params(("id" = Uuid, Path, description = "The player's id")),
    responses(
        (status = 200, description = "Who they are", body = PlayerCard),
        (status = 404, description = "No such player")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_card(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<PlayerCard>, AppError> {
    let card = card(&state.db, auth_user.account_id, Who::Id(id)).await?;
    if !card.mine {
        tutorial::saw_by_account(&state, auth_user.account_id, Did::OpenedCard).await;
    }
    Ok(Json(card))
}

/// Someone's card, by their name, ignoring case: for a letter
///
/// Names aren't unique among people, so a name two share finds the one who
/// took it first ([#428](https://github.com/chrischaps/TheCapitol/issues/428)).
/// No two citizens share one, and a citizen comes before a person.
#[utoipa::path(
    get,
    path = "/players/by-name/{name}",
    params(("name" = String, Path, description = "The player's name, in any case")),
    responses(
        (status = 200, description = "Who they are", body = PlayerCard),
        (status = 404, description = "Nobody of that name")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_card_by_name(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> Result<Json<PlayerCard>, AppError> {
    let card = card(&state.db, auth_user.account_id, Who::Name(&name)).await?;
    // The Gazette's names open cards this way, and count as any other (#447)
    if !card.mine {
        tutorial::saw_by_account(&state, auth_user.account_id, Did::OpenedCard).await;
    }
    Ok(Json(card))
}

/// Stop hearing someone: nothing they say reaches you, on any device
#[utoipa::path(
    put,
    path = "/players/{id}/mute",
    params(("id" = Uuid, Path, description = "Who to mute")),
    responses(
        (status = 200, description = "The people you've muted now", body = MutesResponse),
        (status = 400, description = "You can't mute yourself"),
        (status = 404, description = "No such player")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn mute(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MutesResponse>, AppError> {
    set_mute(&state, auth_user.account_id, id, true).await
}

/// Hear someone again
#[utoipa::path(
    delete,
    path = "/players/{id}/mute",
    params(("id" = Uuid, Path, description = "Who to unmute")),
    responses(
        (status = 200, description = "The people you've muted now", body = MutesResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn unmute(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MutesResponse>, AppError> {
    set_mute(&state, auth_user.account_id, id, false).await
}

async fn set_mute(state: &AppState, account: Uuid, other: Uuid, on: bool) -> Result<Json<MutesResponse>, AppError> {
    let player = player_of(&state.db, account).await?;
    let events = state.game.read().await.event_sender.clone();
    crate::mutes::set(&state.db, &events, player, other, on).await?;
    Ok(Json(MutesResponse { mutes: crate::mutes::list(&state.db, player).await? }))
}

/// The account's player.
pub async fn player_of(db: &PgPool, account: Uuid) -> Result<Uuid, AppError> {
    sqlx::query_scalar("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("No player for this account".into()))
}

/// Whose card.
pub enum Who<'a> {
    Id(Uuid),
    Name(&'a str),
}

/// The card of `who`, as the account `viewer` sees it.
pub async fn card(db: &PgPool, viewer: Uuid, who: Who<'_>) -> Result<PlayerCard, AppError> {
    // Their oldest plot is their home; one claimed before claims were dated counts last
    const CARD: &str = "SELECT p.id, p.name, a.kind = 'citizen', a.created_at, p.account_id = $2,
                (SELECT pl.zone_id FROM plots pl WHERE pl.owner_id = p.id
                 ORDER BY pl.claimed_at NULLS LAST, pl.id LIMIT 1),
                EXISTS (SELECT 1 FROM player_mutes m JOIN players me ON me.id = m.player_id
                        WHERE me.account_id = $2 AND m.muted_id = p.id),
                (SELECT g.name FROM guild_members gm JOIN guilds g ON g.id = gm.guild_id
                 WHERE gm.player_id = p.id)
         FROM players p JOIN accounts a ON a.id = p.account_id";
    type Row = (Uuid, String, bool, DateTime<Utc>, bool, Option<String>, bool, Option<String>);
    let row: Option<Row> = match who {
        Who::Id(id) => {
            sqlx::query_as(&format!("{CARD} WHERE p.id = $1"))
                .bind(id)
                .bind(viewer)
                .fetch_optional(db)
                .await?
        }
        Who::Name(name) => {
            sqlx::query_as(&format!(
                "{CARD} WHERE lower(p.name) = lower($1)
                 ORDER BY a.kind = 'citizen' DESC, p.created_at, p.id LIMIT 1"
            ))
            .bind(name.trim())
            .bind(viewer)
            .fetch_optional(db)
            .await?
        }
    };
    let (id, name, citizen, created, mine, home, muted, guild) =
        row.ok_or_else(|| AppError::NotFound("No one goes by that".into()))?;
    Ok(PlayerCard {
        id,
        name,
        citizen,
        since: rules::day_number(created.timestamp_millis()),
        home,
        standing: standing::NEW_IN_TOWN.into(),
        guild,
        mine,
        muted,
    })
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh
/// database that `sqlx::test` makes from the migrations. Run them with
/// `cargo test card -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{player, plot};

    async fn account_of(db: &PgPool, player: Uuid) -> Uuid {
        sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1")
            .bind(player)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// A citizen called `name`, whose account was made at `at`. An account
    /// never changes kind, so it's a citizen from the first.
    async fn citizen_since(db: &PgPool, name: &str, at: DateTime<Utc>) -> Uuid {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash, kind, created_at) VALUES ($1, $2, 'x', 'citizen', $3)")
            .bind(account)
            .bind(format!("{account}@citizens.test"))
            .bind(at)
            .execute(db)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(account)
            .bind(name)
            .execute(db)
            .await
            .unwrap();
        id
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_persons_card_says_who_they_are_and_whose_it_is(db: PgPool) {
        let ada = player(&db, "Ada", 100).await;
        let bea = player(&db, "Bea", 100).await;
        let (ada_acc, bea_acc) = (account_of(&db, ada).await, account_of(&db, bea).await);

        let seen = card(&db, bea_acc, Who::Id(ada)).await.unwrap();
        assert_eq!((seen.id, seen.name.as_str(), seen.citizen, seen.mine), (ada, "Ada", false, false));
        assert_eq!(seen.since, rules::day_number(Utc::now().timestamp_millis()));
        assert_eq!((seen.home, seen.guild), (None, None), "no plot, and no guilds yet");
        assert_eq!(seen.standing, standing::NEW_IN_TOWN);

        let own = card(&db, ada_acc, Who::Id(ada)).await.unwrap();
        assert!(own.mine && !own.muted);

        // Muted by Bea (S1.4): her card of Ada says so, and no one else's
        let (events, _rx) = tokio::sync::broadcast::channel(4);
        crate::mutes::set(&db, &events, bea, ada, true).await.unwrap();
        assert!(card(&db, bea_acc, Who::Id(ada)).await.unwrap().muted);
        assert!(!card(&db, ada_acc, Who::Id(bea)).await.unwrap().muted, "a mute is one way");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_citizens_card_carries_its_mark_its_day_and_its_home(db: PgPool) {
        let at = DateTime::from_timestamp_millis(rules::day_start_ms(rules::GAZETTE_FIRST_DAY + 7) + 1000).unwrap();
        let cy = citizen_since(&db, "Citizen Cy", at).await;
        let ada = player(&db, "Ada", 100).await;
        plot(&db, Some(cy), 250).await;

        let seen = card(&db, account_of(&db, ada).await, Who::Id(cy)).await.unwrap();
        assert!(seen.citizen && !seen.mine);
        assert_eq!(seen.since, rules::GAZETTE_FIRST_DAY + 7);
        assert_eq!(seen.home.as_deref(), Some("urban"));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_card_for_an_unknown_id_or_name_is_not_found(db: PgPool) {
        let ada = player(&db, "Ada", 100).await;
        let me = account_of(&db, ada).await;
        let err = card(&db, me, Who::Id(Uuid::new_v4())).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
        let err = card(&db, me, Who::Name("Nobody")).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_card_by_name_ignores_case_and_finds_whoever_took_it_first(db: PgPool) {
        let ada = player(&db, "Ada Reeder", 100).await;
        let me = account_of(&db, ada).await;
        let found = card(&db, me, Who::Name("  ada REEDER ")).await.unwrap();
        assert_eq!((found.id, found.name.as_str(), found.mine), (ada, "Ada Reeder", true));

        // A second Ada, a day later, isn't who the name finds (#428)
        let later = player(&db, "ada reeder", 100).await;
        sqlx::query("UPDATE players SET created_at = NOW() + INTERVAL '1 day' WHERE id = $1")
            .bind(later)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(card(&db, me, Who::Name("Ada Reeder")).await.unwrap().id, ada);
    }
}
