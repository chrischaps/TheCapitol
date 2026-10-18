use std::env;

use crate::rate_limit::RateLimitConfig;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub jwt_secret: String,
    pub host: String,
    pub port: u16,
    /// Lifetime of an access token issued with a refresh token (E0.6)
    pub access_ttl_secs: u64,
    /// Lifetime of a refresh token, and of the session it keeps alive
    pub refresh_ttl_secs: u64,
    pub rate_limits: RateLimitConfig,
    /// Whether players may start their own subscription with
    /// `POST /player/subscription`, standing in for payments (E4.1).
    /// `MOCK_SUBSCRIPTIONS=on` or `off`; on by default in debug builds only.
    pub mock_subscriptions: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, env::VarError> {
        Ok(Self {
            database_url: env::var("DATABASE_URL")?,
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into()),
            jwt_secret: env::var("JWT_SECRET")?,
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "3000".into())
                .parse()
                .unwrap_or(3000),
            access_ttl_secs: secs("ACCESS_TTL_SECS", 15 * 60),
            refresh_ttl_secs: secs("REFRESH_TTL_SECS", 30 * 24 * 60 * 60),
            rate_limits: RateLimitConfig::from_env(),
            mock_subscriptions: match env::var("MOCK_SUBSCRIPTIONS").as_deref() {
                Ok("on") => true,
                Ok("off") => false,
                _ => cfg!(debug_assertions),
            },
        })
    }
}

fn secs(name: &str, default: u64) -> u64 {
    env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
