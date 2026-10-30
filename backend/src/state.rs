use crate::config::Config;
use crate::engine::market_matcher::Market;
use crate::engine::tunables::Live;
use crate::engine::sampler::Samples;
use crate::engine::talk::Talk;
use crate::engine::town::Town;
use crate::engine::GameState;
use crate::rate_limit::RateLimiter;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, PgPool};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// How long one Redis command, or one connection attempt, may take.
///
/// The client's defaults have no limit at all. When Docker Desktop's Redis
/// container exits, its port proxy still accepts connections and never
/// answers, so without a limit every request that touches Redis hangs (#75).
pub const REDIS_TIMEOUT: Duration = Duration::from_secs(2);

/// Connect to Redis with timeouts and one quick reconnect retry.
///
/// The retry settings matter as much as the timeouts. redis 0.27 hands its
/// default `factor` (100) to backon as the growth factor, from a 1 s floor
/// with a 60 s cap and six retries, so one reconnect could wait about five
/// minutes before failing. Here a failed reconnect makes two attempts, 1 to 2 s
/// apart, so a request against a dead Redis fails in about 6 s at most. The
/// next request starts a fresh reconnect, so the backend recovers by itself
/// once Redis is back.
pub async fn connect_redis(url: &str) -> redis::RedisResult<ConnectionManager> {
    let client = redis::Client::open(url)?;
    let config = ConnectionManagerConfig::new()
        .set_response_timeout(REDIS_TIMEOUT)
        .set_connection_timeout(REDIS_TIMEOUT)
        .set_factor(2)
        .set_number_of_retries(1)
        .set_max_delay(1000);
    ConnectionManager::new_with_config(client, config).await
}

/// The database pool's connections. The automation tick holds
/// [`WORKERS`](crate::engine::automation_tick::WORKERS) connections through a
/// busy cycle and the market one through its fills (#212), so there are
/// enough besides for the tick, the writer's lanes and the routes.
pub const DB_POOL_SIZE: u32 = 16;

/// How long a connection may sit in the pool before it's pinged on its way
/// out (#622). sqlx pings every connection as it's released, so this only
/// catches one that died while it waited, which on a local database means a
/// restart. A restart usually takes longer than this, so a connection that
/// waited through one is pinged and replaced; after a quicker one, a
/// connection busy just before it fails its next statement, once (a route
/// answers 500, a loop logs it, a gather is tried again). In a busy pool (more than
/// [`DB_POOL_SIZE`] acquires a second, as the writer's lanes make at 200
/// citizens) none waits this long, so none is pinged.
pub const PING_AFTER_IDLE: Duration = Duration::from_secs(1);

/// The database pool's options. sqlx's default pings every connection as it
/// leaves the pool, a round trip on every gather and every route (#622);
/// here only one that has sat idle longer than `ping_after_idle` is pinged
/// ([`PING_AFTER_IDLE`] in the server). A dead one that slips through fails
/// on its first statement, and a gather tries again on another
/// ([`crate::engine::tick::persist_gather`]).
pub fn db_pool_options(ping_after_idle: Duration) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(DB_POOL_SIZE)
        .test_before_acquire(false)
        .before_acquire(move |conn, meta| {
            Box::pin(async move {
                if meta.idle_for > ping_after_idle {
                    conn.ping().await?;
                }
                Ok(true)
            })
        })
}

/// For extractors that only need the database, such as
/// [`crate::subscriptions::Subscriber`].
impl axum::extract::FromRef<AppState> for PgPool {
    fn from_ref(state: &AppState) -> PgPool {
        state.db.clone()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: ConnectionManager,
    pub config: Config,
    pub game: Arc<RwLock<GameState>>,
    pub limiter: Arc<RateLimiter>,
    /// The market's order book (E3.2), matched by its own task, and each
    /// item's published depth (E3.3)
    pub market: Arc<Market>,
    /// The economy's live knobs (D6.1), the same ones the tick reads
    pub tunables: Live,
    /// The Observatory's newest day of samples (D0.2)
    pub samples: Arc<Samples>,
    /// The town's newest report (D4.1)
    pub town: Arc<Town>,
    /// The Capitol channel's last lines, in memory only (Commons S1.1)
    pub talk: Arc<Talk>,
    /// What the day clock adds to the real one, 0 unless a dev server's was
    /// set (`PUT /admin/clock`, #427). In memory: a restart is real time.
    pub day_offset_ms: Arc<AtomicI64>,
}

impl AppState {
    /// The day clock's offset now, for `auth_ok` and `/health` (#427)
    pub fn day_offset_ms(&self) -> i64 {
        self.day_offset_ms.load(Ordering::Relaxed)
    }
}

impl AppState {
    pub fn new(
        db: PgPool,
        redis: ConnectionManager,
        config: Config,
        game: Arc<RwLock<GameState>>,
        market: Arc<Market>,
        tunables: Live,
    ) -> Self {
        Self {
            db,
            redis,
            limiter: Arc::new(RateLimiter::new(config.rate_limits.clone())),
            config,
            game,
            market,
            tunables,
            samples: Arc::new(Samples::new()),
            town: Arc::new(Town::new()),
            talk: Arc::new(Talk::from_clock()),
            day_offset_ms: Arc::default(),
        }
    }
}

/// An `AppState` for database tests that go through a route (#439): the
/// test's database, the local Redis (`REDIS_URL`, or the default), and a game
/// on the default terrain, with nobody in it. The terrain is generated once
/// for every test that asks.
#[cfg(test)]
pub async fn for_tests(db: PgPool) -> AppState {
    use crate::models::{TerrainConfig, WorldGenParams};
    static TERRAIN: std::sync::OnceLock<crate::models::TerrainConfig> = std::sync::OnceLock::new();
    let terrain = TERRAIN.get_or_init(TerrainConfig::get_default).clone();
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
    let redis = connect_redis(&redis_url).await.expect("needs the local Redis (docker-compose up -d)");
    let config = Config {
        database_url: String::new(),
        redis_url,
        jwt_secret: "test".into(),
        host: "127.0.0.1".into(),
        port: 0,
        access_ttl_secs: 15 * 60,
        refresh_ttl_secs: 24 * 60 * 60,
        rate_limits: Default::default(),
        mock_subscriptions: true,
        dev_grants: false,
        dev_clock: false,
        sim_tick_scale: 1.0,
        slow_statement: Duration::from_millis(50),
        sample_secs: 0,
    };
    let game = Arc::new(RwLock::new(GameState::with_terrain(WorldGenParams::default(), terrain)));
    let market = Arc::new(Market::load(&db).await.unwrap());
    // Capitol lines numbered from 1, as the tests count them
    AppState { talk: Arc::new(Talk::new()), ..AppState::new(db, redis, config, game, market, Live::default()) }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use redis::AsyncCommands;
    use sqlx::postgres::PgConnectOptions;
    use std::time::Instant;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    /// What Docker Desktop's port proxy does once the Redis container has
    /// exited: accept every connection, read what's sent, never answer.
    async fn silent_redis() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    while matches!(socket.read(&mut buf).await, Ok(n) if n > 0) {}
                });
            }
        });
        format!("redis://{addr}")
    }

    #[tokio::test]
    async fn a_redis_that_never_answers_fails_instead_of_hanging() {
        let url = silent_redis().await;
        let started = Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(15), connect_redis(&url)).await;
        let Ok(connected) = result else {
            panic!("connect_redis still waiting after 15 s");
        };
        // The handshake may or may not wait for a reply. Either it fails, or
        // the first command does.
        if let Ok(mut redis) = connected {
            let get = tokio::time::timeout(
                Duration::from_secs(15),
                redis.get::<_, Option<String>>("session:x"),
            )
            .await
            .expect("GET still waiting after 15 s");
            assert!(get.is_err(), "GET against a silent Redis succeeded");
        }
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_redis_that_is_gone_fails_quickly() {
        // Bind and drop, so the port refuses connections.
        let addr = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        let started = Instant::now();
        assert!(connect_redis(&format!("redis://{addr}")).await.is_err());
        assert!(started.elapsed() < Duration::from_secs(6), "took {:?}", started.elapsed());
    }

    /// A pool of one connection from [`db_pool_options`], used once and back
    /// in the pool, and that connection's server process
    pub(crate) async fn one_connection(connect: &PgConnectOptions, ping_after_idle: Duration) -> (PgPool, i32) {
        let pool = db_pool_options(ping_after_idle).max_connections(1).connect_with(connect.clone()).await.unwrap();
        let pid = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool).await.unwrap();
        idle(&pool).await;
        (pool, pid)
    }

    /// Wait for `pool`'s connection to come back from its release ping
    async fn idle(pool: &PgPool) {
        let began = Instant::now();
        while pool.num_idle() == 0 {
            assert!(began.elapsed() < Duration::from_secs(5), "the connection never came back to the pool");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// End `pid`'s session from `db`, as a restart does, and wait for it to go
    pub(crate) async fn kill(db: &PgPool, pid: i32) {
        let ended: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1, 5000)").bind(pid).fetch_one(db).await.unwrap();
        assert!(ended, "{pid} still running");
    }

    /// #622: a connection back from a statement a moment ago is handed out
    /// without a ping, so one that died while it waited fails its next
    /// statement, once: it's closed as it goes back, and the one after is
    /// new. One that has waited longer than `ping_after_idle` is pinged on
    /// its way out, and a dead one replaced before anything sees it.
    #[sqlx::test(migrations = false)]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn only_a_connection_that_waited_is_pinged(pool: PgPoolOptions, connect: PgConnectOptions) {
        let db = pool.connect_with(connect.clone()).await.unwrap();
        let select = |pool: PgPool| async move { sqlx::query("SELECT 1").execute(&pool).await };

        let (hot, pid) = one_connection(&connect, Duration::from_secs(60)).await;
        kill(&db, pid).await;
        let Err(e) = select(hot.clone()).await else { panic!("a dead connection was handed out and worked") };
        eprintln!("A dead connection's first statement: {e:?}");
        assert!(select(hot).await.is_ok(), "the dead one wasn't replaced");

        let (pinged, pid) = one_connection(&connect, Duration::ZERO).await;
        kill(&db, pid).await;
        assert!(select(pinged).await.is_ok(), "a dead connection that had waited wasn't pinged");
    }
}
