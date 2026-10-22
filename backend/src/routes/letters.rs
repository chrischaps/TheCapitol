//! `/letters` (S1.5, commons plan D4): writing to someone, for a Strand's
//! postage, and your two boxes.
//!
//! Everything works from anywhere, from any of your sessions. A letter is
//! addressed by the reader's id, from a card's Write or a letter's Reply:
//! names aren't unique ([#428](https://github.com/chrischaps/TheCapitol/issues/428)).
//! Each write is one of `engine::letters`' transactions; once it commits,
//! the route tells the reader, live if they're on, and the writer their
//! balance.

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use capitol_protocol::rest::{Letter, LettersQuery, LettersUnread, Mailbox, SendLetter};
use capitol_protocol::rules::talk;
use chrono::Utc;
use uuid::Uuid;

use super::market::{player_of, tell_balance};
use crate::engine::letters::{self, LetterError};
use crate::engine::GameEvent;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::rate_limit::{self, Key};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(get_letters).post(send))
        .route("/{id}", axum::routing::delete(drop_letter))
        .route("/{id}/read", post(read))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Write to someone. Postage is 1 Strand, and it counts against the same
/// allowance as talk.
#[utoipa::path(
    post,
    path = "/letters",
    request_body = SendLetter,
    responses(
        (status = 200, description = "Sent", body = Letter),
        (status = 400, description = "Empty, too long, to yourself, or no Strand for the postage"),
        (status = 404, description = "No such player"),
        (status = 429, description = "Talking too fast")
    ),
    security(("bearer_auth" = [])),
    tag = "Letters"
)]
pub async fn send(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<SendLetter>,
) -> Response {
    // Cleaned first, so a slip costs no allowance
    if let Err(e) = talk::clean_letter(&req.body) {
        return AppError::BadRequest(e.message()).into_response();
    }
    if let Err(wait) = state.limiter.check(Key::chat(auth_user.account_id)) {
        return rate_limit::too_many_saying(wait, rate_limit::chat_message(wait));
    }
    match deliver(&state, auth_user.account_id, req).await {
        Ok(letter) => Json(letter).into_response(),
        Err(e) => e.into_response(),
    }
}

/// [`send`] past its checks: write it, then tell the reader and the writer.
async fn deliver(state: &AppState, account_id: Uuid, req: SendLetter) -> Result<Letter, AppError> {
    let from = player_of(&state.db, account_id).await?;
    // A reader who's muted the writer (S1.4) never gets it, and the writer
    // can't tell
    let muted = crate::mutes::of(&state.db, req.to).await?.contains(&from);
    let sent = letters::send(&state.db, from, req.to, &req.body, muted, Utc::now()).await.map_err(refused)?;

    let events = state.game.read().await.event_sender.clone();
    if let Some((player_id, notice_id, message)) = sent.notice {
        let _ = events.send(GameEvent::BazaarNotice { player_id, notice_id, message });
    }
    tell_balance(&state.db, &events, from).await;
    Ok(sent.letter)
}

/// A page of your in box or out box, newest first, and how many in your in
/// box are unread.
#[utoipa::path(
    get,
    path = "/letters",
    params(
        ("box" = Option<String>, Query, description = "`in` (the default) or `out`"),
        ("before" = Option<Uuid>, Query, description = "The letters sent before this one")
    ),
    responses((status = 200, description = "The page", body = Mailbox)),
    security(("bearer_auth" = [])),
    tag = "Letters"
)]
pub async fn get_letters(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<LettersQuery>,
) -> Result<Json<Mailbox>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    Ok(Json(letters::page(&state.db, player, query.which, query.before).await.map_err(refused)?))
}

/// You opened a letter in your in box.
#[utoipa::path(
    post,
    path = "/letters/{id}/read",
    params(("id" = Uuid, Path, description = "The letter")),
    responses(
        (status = 200, description = "Your unread letters now", body = LettersUnread),
        (status = 404, description = "No such letter in your in box")
    ),
    security(("bearer_auth" = [])),
    tag = "Letters"
)]
pub async fn read(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<LettersUnread>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    let unread = letters::read(&state.db, player, id, Utc::now()).await.map_err(refused)?;
    Ok(Json(LettersUnread { unread }))
}

/// Drop your copy of a letter, from your in box or out box. The other side
/// keeps theirs.
#[utoipa::path(
    delete,
    path = "/letters/{id}",
    params(("id" = Uuid, Path, description = "The letter")),
    responses(
        (status = 200, description = "Your unread letters now", body = LettersUnread),
        (status = 404, description = "No such letter in your boxes")
    ),
    security(("bearer_auth" = [])),
    tag = "Letters"
)]
pub async fn drop_letter(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<LettersUnread>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    let unread = letters::drop_letter(&state.db, player, id).await.map_err(refused)?;
    Ok(Json(LettersUnread { unread }))
}

/// What the post office refused, as the API says it.
fn refused(e: LetterError) -> AppError {
    match e {
        LetterError::NoOne | LetterError::NoLetter => AppError::NotFound(e.message()),
        LetterError::Db(e) => AppError::Database(e),
        e => AppError::BadRequest(e.message()),
    }
}
