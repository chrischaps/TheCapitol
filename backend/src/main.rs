use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod engine;
mod error;
mod middleware;
mod models;
mod routes;
mod state;
mod ws;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "capitol_backend=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;

    tracing::info!("Starting The Capitol backend server...");

    // Database connection
    let db_pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await?;

    tracing::info!("Connected to PostgreSQL");

    // Run migrations
    sqlx::migrate!("./migrations").run(&db_pool).await?;
    tracing::info!("Migrations complete");

    // Redis connection
    let redis_client = redis::Client::open(config.redis_url.clone())?;
    let redis_conn = redis::aio::ConnectionManager::new(redis_client).await?;
    tracing::info!("Connected to Redis");

    // Initialize game state
    let game_state = Arc::new(RwLock::new(engine::GameState::new()));

    // Create app state
    let app_state = AppState::new(db_pool, redis_conn, config.clone(), game_state.clone());

    // Start tick engine
    let tick_state = game_state.clone();
    let tick_app_state = app_state.clone();
    tokio::spawn(async move {
        engine::run_tick_loop(tick_state, tick_app_state).await;
    });

    // Build router
    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        .nest("/auth", routes::auth::router())
        .nest("/player", routes::player::router(app_state.clone()))
        .route("/ws", get(ws::ws_handler))
        .layer(CorsLayer::new().allow_origin(Any).allow_headers(Any).allow_methods(Any))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
