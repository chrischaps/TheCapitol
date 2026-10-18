use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use uuid::Uuid;

use crate::error::SESSIONS_UNAVAILABLE;
use crate::routes::auth::Claims;
use crate::state::AppState;

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub account_id: Uuid,
#[allow(dead_code)] // the subscription tier; #176 (E4.1) gates on it
    pub tier: String,
    pub is_admin: bool,
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "));

    let token = match auth_header {
        Some(token) => token,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Missing authorization header" })),
            ));
        }
    };

    // Decode and validate JWT
    let token_data = match decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
        &Validation::default(),
    ) {
        Ok(data) => data,
        Err(_) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Invalid token" })),
            ));
        }
    };

    // Verify the token's session is still the account's (sessions.rs)
    let mut redis = state.redis.clone();
    let current = match crate::sessions::is_current(&mut redis, token_data.claims.sub, &token_data.claims.sid).await {
        Ok(current) => current,
        Err(e) => {
            // Not a 401: the token may be fine, and a 401 signs the client out.
            tracing::error!("Redis error checking a session: {:?}", e);
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": SESSIONS_UNAVAILABLE })),
            ));
        }
    };

    if !current {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Session expired" })),
        ));
    }

    // Insert auth user into request extensions
    request.extensions_mut().insert(AuthUser {
        account_id: token_data.claims.sub,
        tier: token_data.claims.tier,
        is_admin: token_data.claims.is_admin,
    });

    Ok(next.run(request).await)
}
