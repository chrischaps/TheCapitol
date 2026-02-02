use crate::config::Config;
use crate::engine::GameState;
use redis::aio::ConnectionManager;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::RwLock;

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
