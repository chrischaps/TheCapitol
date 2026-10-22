//! `/gazette` (E3.9): the Capitol Gazette's editions, and your byline.
//!
//! An edition is composed at each in-game midnight by `engine::gazette` and
//! served as it was kept. `PUT /gazette/byline` says whether the Gazette may
//! print your name, from the next edition on.

use axum::{
    extract::{Query, State},
    routing::get,
    Extension, Json, Router,
};
use capitol_protocol::rest::{GazetteByline, GazetteQuery, GazetteResponse};
use uuid::Uuid;

use crate::engine::gazette;
use crate::engine::tutorial::{self, Did};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(by_date))
        .route("/latest", get(latest))
        .route("/byline", get(get_byline).put(set_byline))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// The latest edition (none before the first midnight), and your byline
#[utoipa::path(
    get,
    path = "/gazette/latest",
    responses((status = 200, description = "The latest edition", body = GazetteResponse)),
    security(("bearer_auth" = [])),
    tag = "Gazette"
)]
pub async fn latest(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<GazetteResponse>, AppError> {
    let edition = gazette::edition(&state.db, None).await?;
    tutorial::saw_by_account(&state, auth_user.account_id, Did::ReadGazette).await;
    Ok(Json(GazetteResponse { edition, named: named(&state, auth_user.account_id).await? }))
}

/// The edition about one day (`rules::day_number`), if there is one
#[utoipa::path(
    get,
    path = "/gazette",
    params(GazetteQuery),
    responses((status = 200, description = "That day's edition, if any", body = GazetteResponse)),
    security(("bearer_auth" = [])),
    tag = "Gazette"
)]
pub async fn by_date(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<GazetteQuery>,
) -> Result<Json<GazetteResponse>, AppError> {
    let edition = gazette::edition(&state.db, Some(query.date)).await?;
    tutorial::saw_by_account(&state, auth_user.account_id, Did::ReadGazette).await;
    Ok(Json(GazetteResponse { edition, named: named(&state, auth_user.account_id).await? }))
}

/// Whether the Gazette prints your name
#[utoipa::path(
    get,
    path = "/gazette/byline",
    responses((status = 200, description = "Your byline", body = GazetteByline)),
    security(("bearer_auth" = [])),
    tag = "Gazette"
)]
pub async fn get_byline(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<GazetteByline>, AppError> {
    Ok(Json(GazetteByline { named: named(&state, auth_user.account_id).await? }))
}

/// Let the Gazette print your name, or keep it out, from the next edition on
#[utoipa::path(
    put,
    path = "/gazette/byline",
    request_body = GazetteByline,
    responses((status = 200, description = "Your byline as it now stands", body = GazetteByline)),
    security(("bearer_auth" = [])),
    tag = "Gazette"
)]
pub async fn set_byline(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(body): Json<GazetteByline>,
) -> Result<Json<GazetteByline>, AppError> {
    let named: bool = sqlx::query_scalar("UPDATE players SET gazette_named = $1 WHERE account_id = $2 RETURNING gazette_named")
        .bind(body.named)
        .bind(auth_user.account_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(GazetteByline { named }))
}

async fn named(state: &AppState, account_id: Uuid) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar("SELECT gazette_named FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account_id)
        .fetch_one(&state.db)
        .await?)
}
