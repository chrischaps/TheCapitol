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
    /// Whether players may grant themselves Strands and goods with
    /// `POST /player/grant`, to fund a market maker on a local stack (E3.8).
    /// `DEV_GRANTS=on`; off unless set, in every build, since it prints money.
    pub dev_grants: bool,
    /// How many times faster than real time the tick runs (E2.7), so the
    /// economy sim's hours take minutes. `SIM_TICK_SCALE`, 1 to 100; debug
    /// builds only, and 1 (real time) unless set. Everything counted in
    /// ticks speeds up with it: walking, gathering, crafting, regrowth and
    /// the hourly tax. What runs on the wall clock doesn't: token lifetimes,
    /// rate limits, trade request expiry and the market's matching task.
    pub sim_tick_scale: f64,
    /// How long a database statement can take before it's logged as slow
    /// and counted against the phase it ran in (D3.2). `SLOW_STATEMENT_MS`,
    /// 50 unless set.
    pub slow_statement: std::time::Duration,
    /// Seconds between the Observatory's samples (D0.2):
    /// `METRICS_SAMPLE_SECS`, `metrics::SAMPLE_SECS` unless set. 0 turns the
    /// sampler off, to measure what it costs the tick.
    pub sample_secs: u64,
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
            dev_grants: env::var("DEV_GRANTS").as_deref() == Ok("on"),
            sim_tick_scale: match tick_scale(
                env::var("SIM_TICK_SCALE").ok().as_deref(),
                cfg!(debug_assertions),
            ) {
                Ok(scale) => scale,
                Err(e) => {
                    tracing::warn!("SIM_TICK_SCALE ignored: {e}");
                    1.0
                }
            },
            slow_statement: std::time::Duration::from_millis(secs("SLOW_STATEMENT_MS", 50)),
            sample_secs: secs("METRICS_SAMPLE_SECS", capitol_protocol::metrics::SAMPLE_SECS),
        })
    }
}

fn secs(name: &str, default: u64) -> u64 {
    env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// `SIM_TICK_SCALE` as set (`raw`), or why it can't be used. Unset is 1.
fn tick_scale(raw: Option<&str>, debug_build: bool) -> Result<f64, String> {
    let Some(raw) = raw.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(1.0);
    };
    let scale: f64 = raw.parse().map_err(|_| format!("{raw:?} isn't a number"))?;
    if !(1.0..=100.0).contains(&scale) {
        return Err(format!("{scale} is outside 1 to 100"));
    }
    if scale != 1.0 && !debug_build {
        return Err("a release build always runs in real time".into());
    }
    Ok(scale)
}

#[cfg(test)]
mod tests {
    use super::tick_scale;

    #[test]
    fn the_tick_scale_is_real_time_unless_a_debug_build_asks() {
        assert_eq!(tick_scale(None, true), Ok(1.0));
        assert_eq!(tick_scale(Some(""), false), Ok(1.0));
        assert_eq!(tick_scale(Some("10"), true), Ok(10.0));
        assert_eq!(tick_scale(Some(" 2.5 "), true), Ok(2.5));
        assert!(tick_scale(Some("10"), false).is_err(), "release builds stay real time");
        assert_eq!(tick_scale(Some("1"), false), Ok(1.0));
        assert!(tick_scale(Some("0.5"), true).is_err());
        assert!(tick_scale(Some("1000"), true).is_err());
        assert!(tick_scale(Some("fast"), true).is_err());
    }
}
