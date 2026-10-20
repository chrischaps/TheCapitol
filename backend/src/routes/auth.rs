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
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use capitol_protocol::rest::{
    AuthResponse, LoginRequest, LogoutRequest, RefreshRequest, RegisterRequest, SuccessResponse,
};
use capitol_protocol::rules::{SPAWN_SCATTER, SPAWN_X, SPAWN_Y};

use crate::error::AppError;
use crate::models::AccountInfo;
use crate::sessions::{self, Refused, RotateError, LEGACY_TTL_SECS};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/logout", post(logout))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub tier: String,
    pub is_admin: bool,
    pub exp: usize,
    /// The session this token belongs to (`sessions`). Tokens from before
    /// E0.6 have none, and no session matches them.
    #[serde(default)]
    pub sid: String,
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
        "INSERT INTO accounts (id, email, password_hash, subscription_status, created_at, last_login)
         VALUES ($1, $2, $3, $4, $5, $5)"
    )
    .bind(account_id)
    .bind(&req.email)
    .bind(&password_hash)
    .bind("free")
    .bind(now)
    .execute(&state.db)
    .await?;

    // Create player at the spawn, scattered across the road so arrivals don't stack
    let player_id = Uuid::new_v4();
    let spawn_x = SPAWN_X + (rand::random::<f64>() * 2.0 - 1.0) * SPAWN_SCATTER;
    sqlx::query(
        "INSERT INTO players (id, account_id, name, position_x, position_y, speed, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(player_id)
    .bind(account_id)
    .bind(&req.player_name)
    .bind(spawn_x)
    .bind(SPAWN_Y)
    .bind(capitol_protocol::rules::PLAYER_SPEED as f32)
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

    // New accounts are not admin by default
    let account = AccountInfo {
        id: account_id,
        email: req.email,
        tier: "free".into(),
        is_admin: false,
    };
    let auth = start_session(&state, account, player_id, req.refresh).await?;
    Ok(Json(auth))
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
        "SELECT id, email, password_hash, subscription_status AS tier, is_admin, created_at, last_login FROM accounts WHERE email = $1"
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

    let auth = start_session(&state, account.into(), player_id, req.refresh).await?;

    // This login replaced any older session, so close that session's socket (#78)
    state.game.write().await.end_session(player_id);

    Ok(Json(auth))
}

/// Starts the account's one session, replacing any it had, and issues its
/// tokens: a short access token and a refresh token when `refresh`, else a
/// legacy 24 h access token.
async fn start_session(
    state: &AppState,
    account: AccountInfo,
    player_id: Uuid,
    refresh: bool,
) -> Result<AuthResponse, AppError> {
    let mut redis = state.redis.clone();
    let refresh_ttl = refresh.then_some(state.config.refresh_ttl_secs);
    let started = sessions::start(&mut redis, account.id, refresh_ttl).await?;
    let expires_in = if refresh { state.config.access_ttl_secs } else { LEGACY_TTL_SECS };
    let token = generate_token(state, &account, &started.sid, expires_in)?;
    Ok(AuthResponse {
        token,
        expires_in,
        refresh_token: started.refresh_token,
        account,
        player_id,
    })
}

/// Swap a refresh token for a new access token and the next refresh token
#[utoipa::path(
    post,
    path = "/auth/refresh",
    request_body(content = RefreshRequest, description = "The session's current refresh token", content_type = "application/json"),
    responses(
        (status = 200, description = "A new token pair", body = AuthResponse),
        (status = 401, description = "Unknown or expired refresh token, an ended session, or a token reused after its 30 s grace (which revokes the session)")
    ),
    tag = "Authentication"
)]
pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let mut redis = state.redis.clone();
    let rotated = match sessions::rotate(&mut redis, &req.refresh_token, state.config.refresh_ttl_secs).await {
        Ok(rotated) => rotated,
        Err(RotateError::Redis(e)) => return Err(e.into()),
        Err(RotateError::Refused(why)) => {
            if let Refused::Reused { account_id } = why {
                // Someone else holds a copy, so the socket goes too.
                tracing::warn!("Refresh token reused for account {}; revoking its session", account_id);
                end_player_session(&state, account_id).await?;
            }
            return Err(AppError::Unauthorized(why.to_string()));
        }
    };

    // Read the account again, so a changed tier or admin flag reaches the token.
    let account = sqlx::query_as::<_, crate::models::Account>(
        "SELECT id, email, password_hash, subscription_status AS tier, is_admin, created_at, last_login FROM accounts WHERE id = $1"
    )
    .bind(rotated.account_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Account not found".into()))?;
    let player_id = sqlx::query_scalar::<_, Uuid>("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account.id)
        .fetch_one(&state.db)
        .await?;

    let account: AccountInfo = account.into();
    let expires_in = state.config.access_ttl_secs;
    let token = generate_token(&state, &account, &rotated.sid, expires_in)?;
    Ok(Json(AuthResponse {
        token,
        expires_in,
        refresh_token: Some(rotated.refresh_token),
        account,
        player_id,
    }))
}

/// Logout and invalidate the session
#[utoipa::path(
    post,
    path = "/auth/logout",
    request_body(content = LogoutRequest, description = "The access or refresh token of the session to end", content_type = "application/json"),
    responses(
        (status = 200, description = "Logout successful")
    ),
    tag = "Authentication"
)]
pub async fn logout(
    State(state): State<AppState>,
    Json(req): Json<LogoutRequest>,
) -> Result<Json<SuccessResponse>, AppError> {
    let mut redis = state.redis.clone();
    let ended = match (&req.token, &req.refresh_token) {
        (Some(token), _) => {
            // An expired token can still log out; its signature must hold.
            let mut validation = jsonwebtoken::Validation::default();
            validation.validate_exp = false;
            let claims = jsonwebtoken::decode::<Claims>(
                token,
                &jsonwebtoken::DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
                &validation,
            )?
            .claims;
            sessions::end(&mut redis, claims.sub, &claims.sid)
                .await?
                .then_some(claims.sub)
        }
        (None, Some(refresh_token)) => sessions::end_by_refresh(&mut redis, refresh_token).await?,
        (None, None) => return Err(AppError::BadRequest("Give a token or a refresh_token".into())),
    };

    // The session is gone, so its socket goes too (#78)
    if let Some(account_id) = ended {
        end_player_session(&state, account_id).await?;
    }

    Ok(Json(SuccessResponse::ok()))
}

/// Closes the account's player's socket, its session having ended.
async fn end_player_session(state: &AppState, account_id: Uuid) -> Result<(), AppError> {
    let player_id = sqlx::query_scalar::<_, Uuid>("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account_id)
        .fetch_optional(&state.db)
        .await?;
    if let Some(player_id) = player_id {
        state.game.write().await.end_session(player_id);
    }
    Ok(())
}

fn generate_token(state: &AppState, account: &AccountInfo, sid: &str, ttl_secs: u64) -> Result<String, AppError> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::seconds(ttl_secs as i64))
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: account.id,
        tier: account.tier.clone(),
        is_admin: account.is_admin,
        exp: expiration,
        sid: sid.to_owned(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.jwt_secret.as_bytes()),
    )
    .map_err(AppError::from)
}
