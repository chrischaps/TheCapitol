//! `/account/api-keys`: an account mints, lists and revokes its bots' API
//! keys (E0.9, `api_keys`). Only the player's own access token reaches these:
//! the middleware refuses keys here (`api_keys::rest_scope`).

use axum::{
    extract::{Path, State},
    routing::{delete, get},
    Extension, Json, Router,
};
use uuid::Uuid;

use capitol_protocol::rest::{ApiKeyInfo, ApiKeyScope, CreateApiKeyRequest, CreatedApiKey, SuccessResponse};

use crate::api_keys::{self, MAX_KEYS};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;
use crate::subscriptions::Subscriber;

/// The longest name a key may have.
const MAX_NAME_LEN: usize = 64;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/{id}", delete(revoke_api_key))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Belt and braces: the middleware already keeps keys out of `/account`.
fn players_only(user: &AuthUser) -> Result<(), AppError> {
    match user.key {
        Some(_) => Err(AppError::Forbidden("An API key can't manage API keys".into())),
        None => Ok(()),
    }
}

/// The scopes to store: known, without repeats, in order.
fn checked_scopes(scopes: &[ApiKeyScope]) -> Result<Vec<ApiKeyScope>, AppError> {
    let mut scopes = scopes.to_vec();
    scopes.sort();
    scopes.dedup();
    if scopes.is_empty() {
        return Err(AppError::BadRequest("A key needs at least one scope: read, play or trade".into()));
    }
    if scopes.contains(&ApiKeyScope::Town) {
        return Err(AppError::BadRequest("The town scope is the town's report key, which an admin mints at /admin/town/key".into()));
    }
    Ok(scopes)
}

/// Mint an API key. The secret is in this answer only. Bots are for
/// subscribers (#217), so this takes a running subscription; listing and
/// revoking don't, so a lapsed player can still revoke a key.
#[utoipa::path(
    post,
    path = "/account/api-keys",
    request_body(content = CreateApiKeyRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "The new key, with its secret shown this once", body = CreatedApiKey),
        (status = 400, description = "No scopes, or a bad name"),
        (status = 403, description = "The account has no running subscription"),
        (status = 409, description = "The account already holds the most live keys it may")
    ),
    security(("bearer_auth" = [])),
    tag = "Account"
)]
pub async fn create_api_key(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    subscriber: Subscriber,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<CreatedApiKey>, AppError> {
    players_only(&user)?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
        return Err(AppError::BadRequest(format!("A key's name is 1 to {MAX_NAME_LEN} characters")));
    }
    let scopes = checked_scopes(&req.scopes)?;

    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_keys WHERE account_id = $1 AND revoked_at IS NULL",
    )
    .bind(subscriber.account_id)
    .fetch_one(&state.db)
    .await?;
    if live >= MAX_KEYS {
        return Err(AppError::Conflict(format!(
            "An account may hold {MAX_KEYS} live keys; revoke one first"
        )));
    }

    let (key, secret) = api_keys::mint(&state.db, subscriber.account_id, name, &scopes).await?;
    tracing::info!(
        "Account {} minted API key {} ({:?}), subscribed until {:?}",
        subscriber.account_id,
        key.id,
        key.scopes,
        subscriber.subscription.until
    );
    Ok(Json(CreatedApiKey { key, secret }))
}

/// The account's keys, revoked ones too, newest first. No secrets.
#[utoipa::path(
    get,
    path = "/account/api-keys",
    responses((status = 200, description = "The account's keys", body = Vec<ApiKeyInfo>)),
    security(("bearer_auth" = [])),
    tag = "Account"
)]
pub async fn list_api_keys(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<ApiKeyInfo>>, AppError> {
    players_only(&user)?;
    let rows: Vec<api_keys::InfoRow> = sqlx::query_as(&format!(
        "SELECT {} FROM api_keys WHERE account_id = $1 ORDER BY created_at DESC",
        api_keys::INFO_COLUMNS
    ))
    .bind(user.account_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(api_keys::info).collect()))
}

/// Revoke a key. It's refused from its next request, and its open sockets close.
#[utoipa::path(
    delete,
    path = "/account/api-keys/{id}",
    params(("id" = Uuid, Path, description = "The key's id")),
    responses(
        (status = 200, description = "Revoked: `{\"success\": true}`"),
        (status = 404, description = "No live key of this account's has that id")
    ),
    security(("bearer_auth" = [])),
    tag = "Account"
)]
pub async fn revoke_api_key(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<SuccessResponse>, AppError> {
    players_only(&user)?;
    let revoked = sqlx::query(
        "UPDATE api_keys SET revoked_at = NOW()
         WHERE id = $1 AND account_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(user.account_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if revoked == 0 {
        return Err(AppError::NotFound("No such API key".into()));
    }
    let closed = state.game.write().await.revoke_key(id);
    tracing::info!("Account {} revoked API key {id}, closing {closed} socket(s)", user.account_id);
    Ok(Json(SuccessResponse::ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ApiKeyScope::*;

    #[test]
    fn scopes_are_deduplicated_and_none_is_refused() {
        assert_eq!(checked_scopes(&[Trade, Read, Trade]).unwrap(), vec![Read, Trade]);
        assert!(checked_scopes(&[]).is_err());
        assert!(checked_scopes(&[Read, Town]).is_err(), "only an admin mints the town's key");
    }

    #[test]
    fn a_key_cant_manage_keys() {
        let mut user = AuthUser {
            account_id: Uuid::new_v4(),
            is_admin: false,
            key: None,
        };
        assert!(players_only(&user).is_ok());
        user.key = Some(api_keys::KeyGrant {
            id: Uuid::new_v4(),
            scopes: api_keys::Scopes::of(&[Play, Trade]),
        });
        assert!(players_only(&user).is_err());
    }
}
