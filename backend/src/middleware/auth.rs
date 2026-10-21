use axum::{
    body::Body,
    extract::{OriginalUri, State},
    http::{header, Method, Request, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use uuid::Uuid;

use crate::api_keys::{self, KeyAuth, KeyGrant};
use crate::engine::presence::{self, Holder};
use crate::error::SESSIONS_UNAVAILABLE;
use crate::routes::auth::Claims;
use crate::state::AppState;

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub account_id: Uuid,
    pub is_admin: bool,
    /// The API key the request came with (E0.9), or `None` for a player's
    /// access token. The middleware has already checked the key's scope.
    pub key: Option<KeyGrant>,
}

type Refusal = (StatusCode, Json<serde_json::Value>);

fn refuse(status: StatusCode, message: impl Into<String>) -> Refusal {
    (status, Json(serde_json::json!({ "error": message.into() })))
}

/// Accepts `Authorization: Bearer <access token>`, or an API key's secret as
/// `Authorization: ApiKey <secret>` (or `Bearer <secret>`), which reaches
/// only the routes its scopes allow (`api_keys::rest_scope`).
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, Refusal> {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| {
            header
                .strip_prefix("Bearer ")
                .or_else(|| header.strip_prefix("ApiKey "))
        })
        .map(|token| token.trim().to_owned());

    let Some(token) = token else {
        return Err(refuse(StatusCode::UNAUTHORIZED, "Missing authorization header"));
    };

    // The full path: inside a `nest`, the request's own URI has lost its prefix.
    let path = request
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| request.uri().path().to_owned(), |uri| uri.path().to_owned());
    let (user, holder) = if api_keys::is_secret(&token) {
        let user = key_user(&state, request.method().clone(), &path, &token).await?;
        let key = user.key.expect("a key's user carries its key").id;
        (user, Holder::Key(key))
    } else {
        token_user(&state, &token).await?
    };

    // Each account's allowance, by what it's asking for (E0.7). A key
    // shares its account's.
    let category = crate::rate_limit::Category::of(request.method(), &path);
    let key = crate::rate_limit::Key::Account(user.account_id, category);
    if let Err(wait) = state.limiter.check(key) {
        tracing::info!("Rate limited {} {path} for account {}", request.method(), user.account_id);
        return Ok(crate::rate_limit::too_many(wait));
    }

    // Driving the avatar is for whoever holds it, while someone does (E0.8).
    // Everything else works from any of the account's sessions.
    if presence::drives_avatar_rest(request.method(), &path) {
        let game = state.game.read().await;
        if game.presence_of_account(user.account_id).is_some_and(|held| *held != holder) {
            return Err(refuse(StatusCode::CONFLICT, presence::NOT_HOLDING));
        }
    }

    // Insert auth user into request extensions
    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}

async fn token_user(state: &AppState, token: &str) -> Result<(AuthUser, Holder), Refusal> {
    // Decode and validate JWT
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| refuse(StatusCode::UNAUTHORIZED, "Invalid token"))?;

    // Verify the token's session is still the account's (sessions.rs)
    let mut redis = state.redis.clone();
    let current = match crate::sessions::is_current(&mut redis, token_data.claims.sub, &token_data.claims.sid).await {
        Ok(current) => current,
        Err(e) => {
            // Not a 401: the token may be fine, and a 401 signs the client out.
            tracing::error!("Redis error checking a session: {:?}", e);
            return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, SESSIONS_UNAVAILABLE));
        }
    };

    if !current {
        return Err(refuse(StatusCode::UNAUTHORIZED, "Session expired"));
    }

    let user = AuthUser {
        account_id: token_data.claims.sub,
        is_admin: token_data.claims.is_admin,
        key: None,
    };
    Ok((user, Holder::Session(token_data.claims.sid)))
}

async fn key_user(state: &AppState, method: Method, path: &str, secret: &str) -> Result<AuthUser, Refusal> {
    let user = match api_keys::authenticate(&state.db, secret).await {
        Ok(KeyAuth::Live(user)) => user,
        Ok(KeyAuth::Unknown) => return Err(refuse(StatusCode::UNAUTHORIZED, "Invalid or revoked API key")),
        Ok(KeyAuth::Unsubscribed(subscription)) => {
            return Err(refuse(StatusCode::FORBIDDEN, api_keys::unsubscribed(&subscription)))
        }
        Err(e) => {
            // Not a 401, for the same reason as a Redis error above.
            tracing::error!("Database error checking an API key: {:?}", e);
            return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "Database error"));
        }
    };

    let needed = api_keys::rest_scope(&method, path);
    if !needed.is_some_and(|scope| user.grant.scopes.allows(scope)) {
        let what = format!("{method} {path}");
        return Err(refuse(StatusCode::FORBIDDEN, api_keys::refusal(&what, needed)));
    }

    Ok(AuthUser {
        account_id: user.account_id,
        // `/admin` is closed to keys anyway.
        is_admin: false,
        key: Some(user.grant),
    })
}
