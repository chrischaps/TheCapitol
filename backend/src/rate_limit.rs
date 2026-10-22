//! Rate limits (E0.7, TDD §2.6).
//!
//! Each key gets a token bucket that holds a window's allowance (a minute,
//! or 10 s for chat) and refills at that rate, so a client may burst up to
//! its limit and is then held to the steady rate. Four places draw from it:
//!
//! - `/auth/*`, keyed by the caller's IP ([`by_ip`])
//! - every authenticated REST route, keyed by account and [`Category`]
//!   ([`crate::middleware::auth::auth_middleware`] calls [`RateLimiter::check`])
//! - WebSocket commands, counted per connection (`ws/connection.rs`)
//! - talk: `say` (S1.1) and `POST /letters` (S1.5), keyed by account
//!   ([`Key::chat`]), apart from the connection's bucket, so talking never
//!   costs a step, and an account's second session can't double it
//!   (commons plan D2). Over it, a line is refused with [`chat_message`].
//!
//! Over the limit, REST answers `429 Too Many Requests` with `Retry-After`,
//! and the WebSocket answers the command with an `error`. capitol-core waits
//! out a `429` and sends the request again.
//!
//! The limits come from the environment ([`RateLimitConfig::from_env`]), with
//! the TDD's numbers as defaults. `RATE_LIMITS=off` turns them all off, for a
//! local `capitol-bot` swarm whose bots all log in from one address.
//!
//! A debug build lets loopback sign in [`LOOPBACK_AUTH_PER_MIN_DEBUG`] times a
//! minute (`RATE_LIMIT_LOOPBACK_AUTH_PER_MIN` sets it), so a local test suite
//! or a few bots don't trip the 10 a minute meant for strangers (#239). A
//! release build counts loopback like any other address.
//!
//! The buckets live in this process. One backend is all there is; a second
//! would need them in Redis.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{ConnectInfo, OriginalUri, State},
    http::{header, HeaderValue, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use crate::state::AppState;

/// What a request is counted against. Each has its own per-minute limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// `/auth/*`: register, login, logout. Keyed by IP.
    Auth,
    /// Authenticated reads (`GET`) outside the inventory: the client's polls.
    Read,
    /// Authenticated writes outside the inventory: moving, extracting,
    /// placing, claiming, exchanging.
    World,
    /// `/inventory/*` and `/player/inventory`, read or write.
    Inventory,
    /// Market writes (E3.3): placing and cancelling orders, collecting from
    /// the market box; and the bazaar's (E3.10): renting, stocking, buying.
    /// Their reads are [`Category::Read`].
    Market,
    /// WebSocket commands, per connection.
    Ws,
    /// Lines of talk and letters, per account, counted over 10 s rather than
    /// a minute (S0.1). Never a REST route's category: the routes that send
    /// text check it themselves, as well as their own.
    Chat,
}

impl Category {
    /// The category of an authenticated REST request, by its full path.
    pub fn of(method: &Method, path: &str) -> Category {
        if path.starts_with("/inventory") || path.starts_with("/player/inventory") {
            Category::Inventory
        } else if method == Method::GET || method == Method::HEAD {
            Category::Read
        } else if path.starts_with("/market") || path.starts_with("/bazaar") {
            Category::Market
        } else {
            Category::World
        }
    }
}

/// Who is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Ip(IpAddr),
    Account(Uuid, Category),
    /// One WebSocket connection. Connections are numbered by [`RateLimiter::connection`].
    Connection(u64),
}

impl Key {
    /// The talk bucket of the account `account_id`: one for all its
    /// sessions, whether they talk over the socket or write letters.
    #[cfg_attr(not(test), allow(dead_code))] // `say` (S1.1, #381) and `POST /letters` (S1.5, #385) check it
    pub fn chat(account_id: Uuid) -> Key {
        Key::Account(account_id, Category::Chat)
    }

    fn category(&self) -> Category {
        match self {
            Key::Ip(_) => Category::Auth,
            Key::Account(_, category) => *category,
            Key::Connection(_) => Category::Ws,
        }
    }
}

/// Requests per minute, per category (lines per 10 s for chat), and whether
/// to count at all.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub auth_per_min: u32,
    /// `/auth/*` from 127.0.0.1 or ::1. The same as `auth_per_min` unless a
    /// debug build or `RATE_LIMIT_LOOPBACK_AUTH_PER_MIN` raises it.
    pub loopback_auth_per_min: u32,
    pub read_per_min: u32,
    pub world_per_min: u32,
    pub inventory_per_min: u32,
    pub market_per_min: u32,
    pub ws_per_min: u32,
    /// Lines of talk and letters, per account, in [`CHAT_WINDOW_SECS`].
    pub chat_per_10s: u32,
}

/// The window chat is counted over: 8 lines in it, by default, and then one
/// every 1.25 s.
pub const CHAT_WINDOW_SECS: f64 = 10.0;

impl Default for RateLimitConfig {
    /// TDD §2.6. Reads aren't in its table: they're the client's polls
    /// (three every 5 s, `capitol_core::poll`), with room for a bot scouting
    /// `/world/nearby` every second. Market reads share `read`; market writes
    /// are the table's 30. Chat is the commons plan's (D2): 8 lines in 10 s.
    fn default() -> Self {
        Self {
            enabled: true,
            auth_per_min: 10,
            loopback_auth_per_min: 10,
            read_per_min: 120,
            world_per_min: 60,
            inventory_per_min: 120,
            market_per_min: 30,
            ws_per_min: 60,
            chat_per_10s: 8,
        }
    }
}

/// Sign-ins a minute from loopback in a debug build, unless
/// `RATE_LIMIT_LOOPBACK_AUTH_PER_MIN` says otherwise: room for capitol-core's
/// live suites run whole, and for a small local bot swarm.
pub const LOOPBACK_AUTH_PER_MIN_DEBUG: u32 = 1000;

impl RateLimitConfig {
    /// `RATE_LIMITS=off` (or `0`, `false`) turns limiting off. Each limit
    /// can be set on its own: `RATE_LIMIT_AUTH_PER_MIN`, `_LOOPBACK_AUTH_`,
    /// `_READ_`, `_WORLD_`, `_INVENTORY_`, `_MARKET_` and `_WS_PER_MIN`, and
    /// `RATE_LIMIT_CHAT_PER_10S`.
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var(name).ok(), cfg!(debug_assertions))
    }

    fn from_vars(var: impl Fn(&str) -> Option<String>, debug_build: bool) -> Self {
        let d = Self::default();
        let limit = |name: &str, default: u32| {
            var(name)
                .and_then(|v| v.trim().parse().ok())
                .filter(|&n: &u32| n > 0)
                .unwrap_or(default)
        };
        let enabled = !matches!(
            var("RATE_LIMITS")
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref(),
            Some("off" | "0" | "false" | "no")
        );
        let auth_per_min = limit("RATE_LIMIT_AUTH_PER_MIN", d.auth_per_min);
        // A release build counts loopback as a stranger unless told otherwise.
        let loopback_default = if debug_build {
            auth_per_min.max(LOOPBACK_AUTH_PER_MIN_DEBUG)
        } else {
            auth_per_min
        };
        Self {
            enabled,
            auth_per_min,
            loopback_auth_per_min: limit("RATE_LIMIT_LOOPBACK_AUTH_PER_MIN", loopback_default),
            read_per_min: limit("RATE_LIMIT_READ_PER_MIN", d.read_per_min),
            world_per_min: limit("RATE_LIMIT_WORLD_PER_MIN", d.world_per_min),
            inventory_per_min: limit("RATE_LIMIT_INVENTORY_PER_MIN", d.inventory_per_min),
            market_per_min: limit("RATE_LIMIT_MARKET_PER_MIN", d.market_per_min),
            ws_per_min: limit("RATE_LIMIT_WS_PER_MIN", d.ws_per_min),
            chat_per_10s: limit("RATE_LIMIT_CHAT_PER_10S", d.chat_per_10s),
        }
    }

    /// The limit for `key`, in its [`window`](Self::window).
    fn capacity(&self, key: &Key) -> u32 {
        match key {
            Key::Ip(ip) if ip.to_canonical().is_loopback() => self.loopback_auth_per_min,
            key => self.limit(key.category()),
        }
    }

    /// How many seconds `key`'s allowance is counted over.
    fn window(key: &Key) -> f64 {
        match key.category() {
            Category::Chat => CHAT_WINDOW_SECS,
            _ => 60.0,
        }
    }

    /// The bucket's refill, in requests a second.
    fn per_sec(&self, key: &Key) -> f64 {
        self.capacity(key) as f64 / Self::window(key)
    }

    fn limit(&self, category: Category) -> u32 {
        match category {
            Category::Auth => self.auth_per_min,
            Category::Read => self.read_per_min,
            Category::World => self.world_per_min,
            Category::Inventory => self.inventory_per_min,
            Category::Market => self.market_per_min,
            Category::Ws => self.ws_per_min,
            Category::Chat => self.chat_per_10s,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

/// Prune full buckets once there are this many keys.
const PRUNE_AT: usize = 4096;

/// The token buckets, one per [`Key`].
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<HashMap<Key, Bucket>>,
    next_connection: std::sync::atomic::AtomicU64,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
            next_connection: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// A key for a new WebSocket connection.
    pub fn connection(&self) -> Key {
        Key::Connection(
            self.next_connection
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Takes one request from `key`'s allowance. `Err` holds how long until
    /// the next one would be let through.
    pub fn check(&self, key: Key) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: Key, now: Instant) -> Result<(), Duration> {
        if !self.config.enabled {
            return Ok(());
        }
        let capacity = self.config.capacity(&key) as f64;
        let per_sec = self.config.per_sec(&key);
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if buckets.len() >= PRUNE_AT {
            // A bucket that has refilled is the same as no bucket.
            let config = &self.config;
            buckets.retain(|k, b| {
                let cap = config.capacity(k) as f64;
                b.tokens + now.saturating_duration_since(b.at).as_secs_f64() * config.per_sec(k) < cap
            });
        }
        let bucket = buckets.entry(key).or_insert(Bucket {
            tokens: capacity,
            at: now,
        });
        let refilled = now.saturating_duration_since(bucket.at).as_secs_f64() * per_sec;
        bucket.tokens = (bucket.tokens + refilled).min(capacity);
        bucket.at = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - bucket.tokens) / per_sec))
        }
    }
}

/// The `429` REST answers when a limit is reached.
pub fn too_many(retry_after: Duration) -> Response {
    // Whole seconds, rounded up, as `Retry-After` wants.
    let secs = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!({ "error": "Too many requests", "retry_after": secs })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(secs.max(1)));
    response
}

/// The WebSocket `error` message for a command over the limit.
pub fn ws_message(retry_after: Duration) -> String {
    format!(
        "Too many commands; wait {:.1} s",
        retry_after.as_secs_f64().max(0.1)
    )
}

/// The words for a line of talk or a letter over [`Category::Chat`]'s limit:
/// `say`'s `error` and `POST /letters`' refusal.
#[cfg_attr(not(test), allow(dead_code))] // `say` (S1.1, #381) and `POST /letters` (S1.5, #385) answer with it
pub fn chat_message(retry_after: Duration) -> String {
    format!(
        "You're talking too fast; wait {:.1} s",
        retry_after.as_secs_f64().max(0.1)
    )
}

/// Middleware for routes keyed by the caller's IP (`/auth/*`). A request
/// with no peer address (only in tests that skip `into_make_service_with_connect_info`)
/// isn't counted.
pub async fn by_ip(State(state): State<AppState>, request: Request<Body>, next: Next) -> Response {
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    if let Some(ip) = ip {
        if let Err(wait) = state.limiter.check(Key::Ip(ip)) {
            // Nested under /auth, so the full path is in OriginalUri
            let path = request
                .extensions()
                .get::<OriginalUri>()
                .map_or(request.uri().path(), |OriginalUri(uri)| uri.path());
            tracing::info!("Rate limited {} {path} from {ip}", request.method());
            return too_many(wait);
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(config: RateLimitConfig) -> RateLimiter {
        RateLimiter::new(config)
    }

    fn ip(last: u8) -> Key {
        Key::Ip(IpAddr::from([10, 0, 0, last]))
    }

    #[test]
    fn a_burst_up_to_the_limit_passes_and_the_next_waits() {
        let l = limiter(RateLimitConfig::default());
        let t = Instant::now();
        for n in 0..10 {
            assert!(l.check_at(ip(1), t).is_ok(), "login {n} refused");
        }
        let wait = l.check_at(ip(1), t).unwrap_err();
        // 10 a minute: one every 6 s
        assert!((wait.as_secs_f64() - 6.0).abs() < 0.01, "{wait:?}");
        assert!(l.check_at(ip(1), t + Duration::from_secs(6)).is_ok());
        assert!(l.check_at(ip(1), t + Duration::from_secs(6)).is_err());
    }

    #[test]
    fn ips_are_counted_apart() {
        let l = limiter(RateLimitConfig::default());
        let t = Instant::now();
        for _ in 0..10 {
            l.check_at(ip(1), t).unwrap();
        }
        assert!(l.check_at(ip(1), t).is_err());
        assert!(l.check_at(ip(2), t).is_ok());
    }

    #[test]
    fn accounts_and_categories_are_counted_apart() {
        let l = limiter(RateLimitConfig::default());
        let t = Instant::now();
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        for _ in 0..60 {
            l.check_at(Key::Account(a, Category::World), t).unwrap();
        }
        assert!(l.check_at(Key::Account(a, Category::World), t).is_err());
        // The same account's reads and inventory have their own allowance,
        assert!(l.check_at(Key::Account(a, Category::Read), t).is_ok());
        assert!(l.check_at(Key::Account(a, Category::Inventory), t).is_ok());
        // and another account isn't touched.
        assert!(l.check_at(Key::Account(b, Category::World), t).is_ok());
    }

    #[test]
    fn connections_are_counted_apart() {
        let l = limiter(RateLimitConfig::default());
        let t = Instant::now();
        let (one, two) = (l.connection(), l.connection());
        assert_ne!(one, two);
        for _ in 0..60 {
            l.check_at(one, t).unwrap();
        }
        assert!(l.check_at(one, t).is_err());
        assert!(l.check_at(two, t).is_ok());
    }

    #[test]
    fn chat_allows_eight_lines_in_ten_seconds() {
        let l = limiter(RateLimitConfig::default());
        let t = Instant::now();
        let me = Key::chat(Uuid::from_u128(1));
        for n in 0..8 {
            assert!(l.check_at(me, t).is_ok(), "line {n} refused");
        }
        let wait = l.check_at(me, t).unwrap_err();
        // Then one every 1.25 s, not one every 7.5 s as a per-minute 8 would.
        assert!((wait.as_secs_f64() - 1.25).abs() < 0.01, "{wait:?}");
        assert!(l.check_at(me, t + Duration::from_millis(1250)).is_ok());
        assert!(l.check_at(me, t + Duration::from_millis(1250)).is_err());
        // Ten quiet seconds and the whole burst is back.
        let later = t + Duration::from_secs(20);
        for _ in 0..8 {
            l.check_at(me, later).unwrap();
        }
        assert!(l.check_at(me, later).is_err());
        assert_eq!(
            chat_message(Duration::from_millis(1250)),
            "You're talking too fast; wait 1.2 s"
        );
    }

    #[test]
    fn an_accounts_sessions_share_one_chat_bucket() {
        let l = limiter(RateLimitConfig::default());
        let t = Instant::now();
        let (ada, bea) = (Uuid::from_u128(1), Uuid::from_u128(2));
        // Two sockets of one account: their commands are counted apart,
        let (desk, laptop) = (l.connection(), l.connection());
        assert_ne!(desk, laptop);
        // but what they say is counted together, with the letters she writes.
        for _ in 0..4 {
            l.check_at(Key::chat(ada), t).unwrap(); // `say` on the desk
            l.check_at(Key::chat(ada), t).unwrap(); // `say` on the laptop, or a letter
        }
        assert!(l.check_at(Key::chat(ada), t).is_err());
        assert!(l.check_at(Key::chat(bea), t).is_ok(), "another account isn't touched");
    }

    #[test]
    fn chat_never_touches_the_connection_bucket() {
        let l = limiter(RateLimitConfig::default());
        let t = Instant::now();
        let me = Uuid::from_u128(1);
        let socket = l.connection();
        for _ in 0..100 {
            let _ = l.check_at(Key::chat(me), t);
        }
        // Talked out, she still has every one of her 60 commands,
        for n in 0..60 {
            assert!(l.check_at(socket, t).is_ok(), "command {n} refused");
        }
        // and her account's REST allowance.
        assert!(l.check_at(Key::Account(me, Category::World), t).is_ok());
        // Walked out, she can still talk.
        assert!(l.check_at(socket, t).is_err());
        let fresh = Key::chat(Uuid::from_u128(2));
        assert!(l.check_at(fresh, t).is_ok());
    }

    #[test]
    fn off_lets_chat_through_too() {
        let l = limiter(RateLimitConfig {
            enabled: false,
            ..Default::default()
        });
        let t = Instant::now();
        for _ in 0..100 {
            l.check_at(Key::chat(Uuid::from_u128(1)), t).unwrap();
        }
    }

    #[test]
    fn a_normal_client_never_waits() {
        // Three polls every 5 s and a containers refetch each second, for ten minutes.
        let l = limiter(RateLimitConfig::default());
        let me = Uuid::from_u128(7);
        let t = Instant::now();
        for s in 0..600u64 {
            let now = t + Duration::from_secs(s);
            if s % 5 == 0 {
                for _ in 0..3 {
                    l.check_at(Key::Account(me, Category::Read), now).unwrap();
                }
            }
            l.check_at(Key::Account(me, Category::Inventory), now)
                .unwrap();
        }
    }

    #[test]
    fn off_lets_everything_through() {
        let l = limiter(RateLimitConfig {
            enabled: false,
            ..Default::default()
        });
        let t = Instant::now();
        for _ in 0..1000 {
            l.check_at(ip(1), t).unwrap();
        }
    }

    #[test]
    fn categories_follow_the_path() {
        use Category::*;
        assert_eq!(
            Category::of(&Method::GET, "/inventory/containers"),
            Inventory
        );
        assert_eq!(Category::of(&Method::POST, "/inventory/move"), Inventory);
        assert_eq!(Category::of(&Method::GET, "/player/inventory"), Inventory);
        assert_eq!(Category::of(&Method::GET, "/world/nearby"), Read);
        assert_eq!(Category::of(&Method::POST, "/world/extract"), World);
        assert_eq!(Category::of(&Method::DELETE, "/stations/x"), World);
        assert_eq!(Category::of(&Method::GET, "/market/orders"), Read);
        assert_eq!(Category::of(&Method::POST, "/market/orders"), Market);
        assert_eq!(Category::of(&Method::DELETE, "/market/orders/x"), Market);
        assert_eq!(Category::of(&Method::POST, "/bazaar/listings/x/buy"), Market);
        assert_eq!(Category::of(&Method::GET, "/bazaar"), Read);
    }

    #[test]
    fn config_reads_the_environment_and_ignores_nonsense() {
        let vars: HashMap<&str, &str> = [
            ("RATE_LIMIT_AUTH_PER_MIN", "100"),
            ("RATE_LIMIT_WS_PER_MIN", "zero"),
            ("RATE_LIMIT_READ_PER_MIN", "0"),
            ("RATE_LIMIT_CHAT_PER_10S", "20"),
        ]
        .into();
        let c = RateLimitConfig::from_vars(|k| vars.get(k).map(|v| v.to_string()), false);
        assert!(c.enabled);
        assert_eq!(c.chat_per_10s, 20);
        assert_eq!(c.auth_per_min, 100);
        assert_eq!(c.loopback_auth_per_min, 100);
        assert_eq!(c.ws_per_min, 60);
        assert_eq!(c.read_per_min, 120);
        let off =
            RateLimitConfig::from_vars(|k| (k == "RATE_LIMITS").then(|| "Off".into()), false);
        assert!(!off.enabled);
    }

    #[test]
    fn a_debug_build_lets_loopback_sign_in_freely() {
        let none = |_: &str| None;
        let debug = RateLimitConfig::from_vars(none, true);
        assert_eq!(debug.auth_per_min, 10);
        assert_eq!(debug.loopback_auth_per_min, LOOPBACK_AUTH_PER_MIN_DEBUG);
        let release = RateLimitConfig::from_vars(none, false);
        assert_eq!(release.loopback_auth_per_min, 10, "release counts loopback as anyone");
        let set = RateLimitConfig::from_vars(
            |k| (k == "RATE_LIMIT_LOOPBACK_AUTH_PER_MIN").then(|| "50".into()),
            false,
        );
        assert_eq!(set.loopback_auth_per_min, 50);

        let l = limiter(debug);
        let t = Instant::now();
        let v4 = Key::Ip(IpAddr::from([127, 0, 0, 1]));
        let v6 = Key::Ip(IpAddr::from(std::net::Ipv6Addr::LOCALHOST));
        let mapped = Key::Ip(IpAddr::from(std::net::Ipv4Addr::LOCALHOST.to_ipv6_mapped()));
        for _ in 0..100 {
            l.check_at(v4, t).unwrap();
            l.check_at(v6, t).unwrap();
            l.check_at(mapped, t).unwrap();
        }
        // Everyone else still gets ten.
        for _ in 0..10 {
            l.check_at(ip(1), t).unwrap();
        }
        assert!(l.check_at(ip(1), t).is_err());
    }

    #[test]
    fn full_buckets_are_pruned() {
        let l = limiter(RateLimitConfig::default());
        let t = Instant::now();
        for n in 0..PRUNE_AT as u128 {
            l.check_at(Key::Account(Uuid::from_u128(n), Category::Read), t)
                .unwrap();
        }
        // A minute on, every bucket has refilled and goes.
        l.check_at(ip(1), t + Duration::from_secs(60)).unwrap();
        assert_eq!(l.buckets.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_429_says_when_to_come_back() {
        let r = too_many(Duration::from_millis(5200));
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(r.headers()[header::RETRY_AFTER], "6");
    }
}
