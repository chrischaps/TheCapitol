//! `/leaderboards` (S3.2, commons plan D9): the week's boards, as
//! `engine::leaderboards` worked them out at the week's end, with your own
//! line on each.

use axum::{
    extract::{Query, State},
    routing::get,
    Extension, Json, Router,
};
use capitol_protocol::rest::{LeaderboardQuery, LeaderboardsResponse};

use super::market::player_of;
use crate::engine::leaderboards;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new().route("/", get(get_leaderboards)).route_layer(axum::middleware::from_fn_with_state(
        state,
        crate::middleware::auth::auth_middleware,
    ))
}

/// A week's leaderboards: each board's top 20 and your own line, whole-city
/// or for one home zone, citizens included or people only
#[utoipa::path(
    get,
    path = "/leaderboards",
    params(LeaderboardQuery),
    responses((status = 200, description = "The week's boards (none before the first week ends)", body = LeaderboardsResponse)),
    security(("bearer_auth" = [])),
    tag = "Leaderboards"
)]
pub async fn get_leaderboards(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<LeaderboardQuery>,
) -> Result<Json<LeaderboardsResponse>, AppError> {
    let me = player_of(&state.db, auth_user.account_id).await?;
    Ok(Json(leaderboards::read(&state.db, &query, Some(me)).await?))
}
