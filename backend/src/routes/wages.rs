//! `/wages` (S2.6, commons plan D7): piece-rate work at someone else's
//! station or on their lot, the pay escrowed up front.
//!
//! Everything works from anywhere: offering, taking, stopping and
//! withdrawing. The work itself is done where it is, at the station or on the
//! lot, through the usual craft and gather. Each write is one of
//! `engine::wages`' transactions; once it commits, the route sends its news
//! (`wages::announce`): the board's Work page to everyone, notices to the
//! employer and the worker, and the balances that moved.

use axum::{
    extract::{Path, Query, State},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use capitol_protocol::rest::{OfferWages, ShiftEnded, WageOffer, WageShift, WagesQuery, WagesResponse};
use chrono::Utc;
use uuid::Uuid;

use super::market::player_of;
use crate::engine::wages::{self, News, WageError};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(get_wages).post(offer))
        .route("/{id}", get(get_offer).delete(withdraw))
        .route("/{id}/take", post(take))
        .route("/{id}/shift", delete(stop))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Offer work at a station of yours, or on a lot of yours. `rate × cap` is
/// escrowed, and the court's fee is charged on top.
#[utoipa::path(
    post,
    path = "/wages",
    request_body = OfferWages,
    responses(
        (status = 200, description = "On the board", body = WageOffer),
        (status = 400, description = "Out of bounds, nothing of it on the lot, five open already, or not enough Strands"),
        (status = 403, description = "Not your station or lot"),
        (status = 404, description = "No such station, lot, recipe, resource or player")
    ),
    security(("bearer_auth" = [])),
    tag = "Wages"
)]
pub async fn offer(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<OfferWages>,
) -> Result<Json<WageOffer>, AppError> {
    let employer = player_of(&state.db, auth_user.account_id).await?;
    // Whether it grows on the lot is the terrain's to say, from game state (#435)
    let grows = wages::grows(&state.db, &state.game, &req.work).await?;
    let (offered, news) = wages::offer(&state.db, employer, &req, grows, Utc::now()).await.map_err(refused)?;
    tell(&state, news).await;
    Ok(Json(offered))
}

/// The board's Work page: the open offers, newest first, and your own shift
/// if you're on one.
#[utoipa::path(
    get,
    path = "/wages",
    params(WagesQuery),
    responses((status = 200, description = "The offers", body = WagesResponse)),
    security(("bearer_auth" = [])),
    tag = "Wages"
)]
pub async fn get_wages(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<WagesQuery>,
) -> Result<Json<WagesResponse>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    let offers = wages::list(&state.db, player, &query).await?;
    let shift = wages::shift_of(&state.db, player).await?;
    Ok(Json(WagesResponse { offers, shift }))
}

/// One offer, open or not, with the shift running on it.
#[utoipa::path(
    get,
    path = "/wages/{id}",
    params(("id" = Uuid, Path, description = "The offer")),
    responses(
        (status = 200, description = "The offer", body = WageOffer),
        (status = 404, description = "No such offer")
    ),
    security(("bearer_auth" = [])),
    tag = "Wages"
)]
pub async fn get_offer(State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<WageOffer>, AppError> {
    let mut conn = state.db.acquire().await?;
    let offer = wages::load(&mut conn, id).await?.ok_or_else(|| refused(WageError::NotFound))?;
    Ok(Json(offer))
}

/// Take your offer down. A shift on it ends, paid for what it made, and the
/// rest of the escrow comes back. The fee isn't refunded.
#[utoipa::path(
    delete,
    path = "/wages/{id}",
    params(("id" = Uuid, Path, description = "The offer")),
    responses(
        (status = 200, description = "Withdrawn", body = WageOffer),
        (status = 400, description = "It's closed already"),
        (status = 404, description = "No such offer of yours")
    ),
    security(("bearer_auth" = [])),
    tag = "Wages"
)]
pub async fn withdraw(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<WageOffer>, AppError> {
    let employer = player_of(&state.db, auth_user.account_id).await?;
    let (offer, news) = wages::withdraw(&state.db, employer, id, Utc::now()).await.map_err(refused)?;
    tell(&state, news).await;
    Ok(Json(offer))
}

/// Take an offer and start a shift on it: one worker at a time, and one
/// shift at a time for you.
#[utoipa::path(
    post,
    path = "/wages/{id}/take",
    params(("id" = Uuid, Path, description = "The offer")),
    responses(
        (status = 200, description = "Your shift", body = WageShift),
        (status = 400, description = "It's closed, or yours"),
        (status = 403, description = "It's for someone else"),
        (status = 404, description = "No such offer"),
        (status = 409, description = "Someone is working it, or you're on a shift")
    ),
    security(("bearer_auth" = [])),
    tag = "Wages"
)]
pub async fn take(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<WageShift>, AppError> {
    let worker = player_of(&state.db, auth_user.account_id).await?;
    let (shift, news) = wages::take(&state.db, worker, id, Utc::now()).await.map_err(refused)?;
    tell(&state, news).await;
    Ok(Json(shift))
}

/// Stop the shift on an offer, as its worker or the offer's employer. What
/// was made is paid; the offer stays open.
#[utoipa::path(
    delete,
    path = "/wages/{id}/shift",
    params(("id" = Uuid, Path, description = "The offer")),
    responses(
        (status = 200, description = "What the shift came to", body = ShiftEnded),
        (status = 400, description = "No shift of yours is running on it"),
        (status = 404, description = "No such offer")
    ),
    security(("bearer_auth" = [])),
    tag = "Wages"
)]
pub async fn stop(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<ShiftEnded>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    let (ended, news) = wages::stop(&state.db, player, id, Utc::now()).await.map_err(refused)?;
    tell(&state, news).await;
    Ok(Json(ShiftEnded { shift: ended.shift, paid: ended.paid }))
}

async fn tell(state: &AppState, news: News) {
    let events = state.game.read().await.event_sender.clone();
    wages::announce(&events, news);
}

/// What the court refused, as the API says it.
fn refused(e: WageError) -> AppError {
    match e {
        WageError::NoSuchStation
        | WageError::NoSuchLot
        | WageError::UnknownRecipe
        | WageError::UnknownResource
        | WageError::NoSuchWorker
        | WageError::NotFound => AppError::NotFound(e.message()),
        WageError::NotYourStation | WageError::NotYourLot | WageError::NotForYou => AppError::Forbidden(e.message()),
        WageError::Taken | WageError::OnAShift => AppError::Conflict(e.message()),
        WageError::Db(e) => AppError::Database(e),
        e => AppError::BadRequest(e.message()),
    }
}
