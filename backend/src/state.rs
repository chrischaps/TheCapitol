use crate::config::Config;
use crate::engine::GameState;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use sqlx::PgPool;
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

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: ConnectionManager,
    pub config: Config,
    pub game: Arc<RwLock<GameState>>,
}

impl AppState {
    pub fn new(
        db: PgPool,
        redis: ConnectionManager,
        config: Config,
        game: Arc<RwLock<GameState>>,
    ) -> Self {
        Self {
            db,
            redis,
            config,
            game,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis::AsyncCommands;
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
}
