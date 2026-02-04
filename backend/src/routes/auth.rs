use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use chrono::Utc;
use jsonwebtoken::{encode, Header, EncodingKey};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::AccountInfo;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterRequest {
    /// Email address for the account
    pub email: String,
    /// Password (will be hashed with Argon2)
    pub password: String,
    /// Display name for the player character
    pub player_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthResponse {
    /// JWT token for authentication
    pub token: String,
    /// Account information
    pub account: AccountInfo,
    /// Player character ID
    pub player_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub tier: String,
    pub is_admin: bool,
    pub exp: usize,
}

/// Register a new account
#[utoipa::path(
    post,
    path = "/auth/register",
    request_body(content = RegisterRequest, description = "Registration details", content_type = "application/json"),
    responses(
        (status = 200, description = "Registration successful", body = AuthResponse),
        (status = 409, description = "Email already registered")
    ),
    tag = "Authentication"
)]
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    // Check if email already exists
    let existing = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts WHERE email = $1")
        .bind(&req.email)
        .fetch_one(&state.db)
        .await?;

    if existing > 0 {
        return Err(AppError::Conflict("Email already registered".into()));
    }

    // Hash password with Argon2id
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Password hashing failed: {}", e)))?
        .to_string();

    // Create account
    let account_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO accounts (id, email, password_hash, tier, created_at, last_login)
         VALUES ($1, $2, $3, $4, $5, $5)"
    )
    .bind(account_id)
    .bind(&req.email)
    .bind(&password_hash)
    .bind("free")
    .bind(now)
    .execute(&state.db)
    .await?;

    // Create player
    let player_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO players (id, account_id, name, position_x, position_y, speed, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(player_id)
    .bind(account_id)
    .bind(&req.player_name)
    .bind(500.0)
    .bind(500.0)
    .bind(50.0f32)
    .bind(now)
    .execute(&state.db)
    .await?;

    // Create containers for the new player
    // Player inventory (24 slots, 6 columns)
    sqlx::query(
        "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, $2, $3)"
    )
    .bind(Uuid::new_v4())
    .bind("player_inventory")
    .bind(player_id)
    .execute(&state.db)
    .await?;

    // Crafting input (4 slots, 2 columns)
    sqlx::query(
        "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, $2, $3)"
    )
    .bind(Uuid::new_v4())
    .bind("crafting_input")
    .bind(player_id)
    .execute(&state.db)
    .await?;

    // Crafting output (1 slot)
    sqlx::query(
        "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, $2, $3)"
    )
    .bind(Uuid::new_v4())
    .bind("crafting_output")
    .bind(player_id)
    .execute(&state.db)
    .await?;

    // Generate JWT (new accounts are not admin by default)
    let token = generate_token(&state, account_id, "free", false)?;

    // Store session in Redis
    let mut redis = state.redis.clone();
    let session_key = format!("session:{}", account_id);
    redis.set_ex::<_, _, ()>(&session_key, &token, 86400).await?;

    Ok(Json(AuthResponse {
        token,
        account: AccountInfo {
            id: account_id,
            email: req.email,
            tier: "free".into(),
            is_admin: false,
        },
        player_id,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    /// Account email address
    pub email: String,
    /// Account password
    pub password: String,
}

/// Login to an existing account
#[utoipa::path(
    post,
    path = "/auth/login",
    request_body(content = LoginRequest, description = "Login credentials", content_type = "application/json"),
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 401, description = "Invalid credentials")
    ),
    tag = "Authentication"
)]
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    // Find account
    let account = sqlx::query_as::<_, crate::models::Account>(
        "SELECT id, email, password_hash, tier, is_admin, created_at, last_login FROM accounts WHERE email = $1"
    )
    .bind(&req.email)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;

    // Verify password
    let parsed_hash = PasswordHash::new(&account.password_hash)
        .map_err(|e| AppError::Internal(format!("Password hash parse error: {}", e)))?;

    Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Invalid credentials".into()))?;

    // Update last login
    sqlx::query("UPDATE accounts SET last_login = $1 WHERE id = $2")
        .bind(Utc::now())
        .bind(account.id)
        .execute(&state.db)
        .await?;

    // Get player
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(account.id)
    .fetch_one(&state.db)
    .await?;

    // Generate JWT
    let token = generate_token(&state, account.id, &account.tier, account.is_admin)?;

    // Store session in Redis
    let mut redis = state.redis.clone();
    let session_key = format!("session:{}", account.id);
    redis.set_ex::<_, _, ()>(&session_key, &token, 86400).await?;

    Ok(Json(AuthResponse {
        token,
        account: account.into(),
        player_id,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LogoutRequest {
    /// JWT token to invalidate
    pub token: String,
}

/// Logout and invalidate the session
#[utoipa::path(
    post,
    path = "/auth/logout",
    request_body(content = LogoutRequest, description = "Token to invalidate", content_type = "application/json"),
    responses(
        (status = 200, description = "Logout successful")
    ),
    tag = "Authentication"
)]
pub async fn logout(
    State(state): State<AppState>,
    Json(req): Json<LogoutRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Decode token to get account_id
    let token_data = jsonwebtoken::decode::<Claims>(
        &req.token,
        &jsonwebtoken::DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    )?;

    // Remove session from Redis
    let mut redis = state.redis.clone();
    let session_key = format!("session:{}", token_data.claims.sub);
    redis.del::<_, ()>(&session_key).await?;

    Ok(Json(serde_json::json!({ "success": true })))
}

fn generate_token(state: &AppState, account_id: Uuid, tier: &str, is_admin: bool) -> Result<String, AppError> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: account_id,
        tier: tier.to_string(),
        is_admin,
        exp: expiration,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.jwt_secret.as_bytes()),
    )
    .map_err(AppError::from)
}
