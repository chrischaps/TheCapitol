//! `/tutorial` (#438, `the-capitol-road-in.md`): where you are on the road
//! in, and hiding its card for now. The steps themselves are judged by the
//! server where each action succeeds (#439), never asked for here.

use axum::{
    extract::State,
    routing::{get, put},
    Extension, Json, Router,
};
use capitol_protocol::tutorial::{SetDismissed, TutorialProgress};

use crate::engine::tutorial;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(get_tutorial))
        .route("/dismissed", put(set_dismissed))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Where you are on the road in: the steps done, and whether the card is
/// hidden
#[utoipa::path(
    get,
    path = "/tutorial",
    responses(
        (status = 200, description = "Your place on the road in", body = TutorialProgress)
    ),
    security(("bearer_auth" = [])),
    tag = "Tutorial"
)]
pub async fn get_tutorial(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<TutorialProgress>, AppError> {
    let player = super::players::player_of(&state.db, auth_user.account_id).await?;
    Ok(Json(tutorial::progress(&state.db, player).await?))
}

/// Hide the road in's card for now, or bring it back, on every device. The
/// tour waits: it can't be skipped (plan D2).
#[utoipa::path(
    put,
    path = "/tutorial/dismissed",
    request_body = SetDismissed,
    responses(
        (status = 200, description = "Your place on the road in, after it", body = TutorialProgress)
    ),
    security(("bearer_auth" = [])),
    tag = "Tutorial"
)]
pub async fn set_dismissed(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<SetDismissed>,
) -> Result<Json<TutorialProgress>, AppError> {
    let player = super::players::player_of(&state.db, auth_user.account_id).await?;
    let events = state.game.read().await.event_sender.clone();
    Ok(Json(tutorial::set_dismissed(&state.db, &events, player, req.dismissed).await?))
}
