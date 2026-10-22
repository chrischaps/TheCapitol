//! `/commissions` (Commons S2.1, plan D5): the hall's board. Posting,
//! delivering, cancelling and reading commissions.
//!
//! Each is `engine::commissions`'s, one transaction. Once it commits, the
//! route tells everyone the board changed (`GameEvent::Board`), sends the
//! poster their notice if they're on, and says which balances and containers
//! changed. Posting and delivering happen at the Exchange hall, from within
//! `HALL_RANGE` of its footprint; reading and cancelling work from anywhere.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use capitol_protocol::rest::{
    Commission, CommissionDelivery, CommissionsQuery, CommissionsResponse, DeliverCommission, PostCommission,
};
use capitol_protocol::rules::court::CommissionStatus;
use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use super::market::{at_the_exchange, player_of, tell_balance};
use crate::engine::bazaar::Notice;
use crate::engine::commissions::{self, Closed, CourtError};
use crate::engine::{BoardChange, GameEvent};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", post(post_commission).get(list_commissions))
        .route("/{id}", get(get_commission).delete(cancel_commission))
        .route("/{id}/deliver", post(deliver_commission))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Post a commission: its whole price is escrowed and the court's fee
/// charged, and it goes up on the board
#[utoipa::path(
    post,
    path = "/commissions",
    request_body = PostCommission,
    responses(
        (status = 200, description = "On the board", body = Commission),
        (status = 400, description = "Away from the Exchange hall, out of bounds, five open already, or not enough Strands")
    ),
    security(("bearer_auth" = [])),
    tag = "Commissions"
)]
pub async fn post_commission(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<PostCommission>,
) -> Result<Json<Commission>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    at_the_exchange(&state.game, &state.db, player_id, "post a commission").await?;
    let commission = commissions::post(&state.db, player_id, &req, Utc::now()).await.map_err(refused)?;

    let events = events(&state).await;
    let _ = events.send(GameEvent::Board { change: BoardChange::Posted, commission: commission.clone() });
    tell_balance(&state.db, &events, player_id).await;
    Ok(Json(commission))
}

/// Deliver units to a commission: they go to its poster's market box, and
/// you're paid for them out of its escrow
#[utoipa::path(
    post,
    path = "/commissions/{id}/deliver",
    params(("id" = Uuid, Path, description = "The commission")),
    request_body = DeliverCommission,
    responses(
        (status = 200, description = "Delivered and paid", body = CommissionDelivery),
        (status = 400, description = "Away from the Exchange hall, your own, not open, not your stack, or under its quality"),
        (status = 404, description = "No such commission")
    ),
    security(("bearer_auth" = [])),
    tag = "Commissions"
)]
pub async fn deliver_commission(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(commission_id): Path<Uuid>,
    Json(req): Json<DeliverCommission>,
) -> Result<Json<CommissionDelivery>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    at_the_exchange(&state.game, &state.db, player_id, "deliver to a commission").await?;
    let done = commissions::deliver(&state.db, player_id, commission_id, &req, Utc::now())
        .await
        .map_err(refused)?;

    let events = events(&state).await;
    let commission = done.delivery.commission.clone();
    let change = match commission.status {
        CommissionStatus::Open => BoardChange::Updated,
        _ => BoardChange::Closed,
    };
    let _ = events.send(GameEvent::Board { change, commission });
    let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id: done.source, slots: Vec::new() });
    if let Some(container_id) = done.market_box {
        let poster = done.notice.player_id;
        let _ = events.send(GameEvent::InventoryUpdated { player_id: poster, container_id, slots: Vec::new() });
    }
    tell(&events, done.notice);
    tell_balance(&state.db, &events, player_id).await;
    Ok(Json(done.delivery))
}

/// Cancel one of your open commissions: what's unspent of its escrow comes
/// back. The court's fee isn't refunded.
#[utoipa::path(
    delete,
    path = "/commissions/{id}",
    params(("id" = Uuid, Path, description = "The commission")),
    responses(
        (status = 200, description = "Cancelled, as it stands now", body = Commission),
        (status = 400, description = "Not open"),
        (status = 404, description = "No such commission of yours")
    ),
    security(("bearer_auth" = [])),
    tag = "Commissions"
)]
pub async fn cancel_commission(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(commission_id): Path<Uuid>,
) -> Result<Json<Commission>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let closed = commissions::cancel(&state.db, player_id, commission_id).await.map_err(refused)?;
    let commission = closed.commission.clone();
    announce_closed(&state.db, &events(&state).await, vec![closed]).await;
    Ok(Json(commission))
}

/// The board: open commissions newest first, or as the query narrows it
#[utoipa::path(
    get,
    path = "/commissions",
    params(CommissionsQuery),
    responses((status = 200, description = "At most 200, newest first", body = CommissionsResponse)),
    security(("bearer_auth" = [])),
    tag = "Commissions"
)]
pub async fn list_commissions(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<CommissionsQuery>,
) -> Result<Json<CommissionsResponse>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let commissions = commissions::list(&state.db, player_id, &query).await?;
    Ok(Json(CommissionsResponse { commissions }))
}

/// One commission, open or closed
#[utoipa::path(
    get,
    path = "/commissions/{id}",
    params(("id" = Uuid, Path, description = "The commission")),
    responses(
        (status = 200, description = "As it stands", body = Commission),
        (status = 404, description = "No such commission")
    ),
    security(("bearer_auth" = [])),
    tag = "Commissions"
)]
pub async fn get_commission(
    State(state): State<AppState>,
    Path(commission_id): Path<Uuid>,
) -> Result<Json<Commission>, AppError> {
    let mut conn = state.db.acquire().await?;
    commissions::load(&mut conn, commission_id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::NotFound(CourtError::NotFound.message()))
}

/// Tell everyone that `closed` came down, send each poster their notice if
/// they're on, and their balance where a refund moved it. The routes' cancel
/// and the market loop's expiry both end here.
pub async fn announce_closed(db: &PgPool, events: &broadcast::Sender<GameEvent>, closed: Vec<Closed>) {
    for Closed { commission, refunded, notice } in closed {
        let poster = commission.poster_id;
        let _ = events.send(GameEvent::Board { change: BoardChange::Closed, commission });
        if let Some(notice) = notice {
            tell(events, notice);
        }
        if refunded > 0 {
            tell_balance(db, events, poster).await;
        }
    }
}

/// Send a queued notice live, for the player's socket to take
fn tell(events: &broadcast::Sender<GameEvent>, notice: Notice) {
    let Notice { player_id, notice_id, message } = notice;
    let _ = events.send(GameEvent::Notice { player_id, notice_id, message });
}

/// What the court refused, as the API says it.
fn refused(e: CourtError) -> AppError {
    match e {
        CourtError::NotFound => AppError::NotFound(e.message()),
        CourtError::Db(e) => AppError::Database(e),
        e => AppError::BadRequest(e.message()),
    }
}

async fn events(state: &AppState) -> broadcast::Sender<GameEvent> {
    state.game.read().await.event_sender.clone()
}
