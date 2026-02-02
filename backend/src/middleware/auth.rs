use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use redis::AsyncCommands;
use uuid::Uuid;

use crate::routes::auth::Claims;
use crate::state::AppState;

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub account_id: Uuid,
    pub tier: String,
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

    // Verify session exists in Redis
    let mut redis = state.redis.clone();
    let session_key = format!("session:{}", token_data.claims.sub);
    let stored_token: Option<String> = redis.get(&session_key).await.unwrap_or(None);

    if stored_token.as_deref() != Some(token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Session expired" })),
        ));
    }

    // Insert auth user into request extensions
    request.extensions_mut().insert(AuthUser {
        account_id: token_data.claims.sub,
        tier: token_data.claims.tier,
    });

    Ok(next.run(request).await)
}
