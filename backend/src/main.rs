use axum::{
    routing::get,
    Router,
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
use models::{ItemType, Recipe, RecipeRow, ResourceNode, ResourceNodeState, ResourceType, Station, StationState, StationType};
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

    // Load resource types into game state
    let resource_types: Vec<ResourceType> = sqlx::query_as(
        "SELECT id, name, yields_item, yield_quantity_min, yield_quantity_max,
         base_quality_min, base_quality_max, required_tool, extraction_duration_ticks, regeneration_ticks
         FROM resource_types"
    )
    .fetch_all(&db_pool)
    .await?;

    {
        let mut game = game_state.write().await;
        for rt in resource_types {
            tracing::info!("Loaded resource type: {}", rt.name);
            game.resource_types.insert(rt.id.clone(), rt);
        }
    }

    // Load resource nodes into game state
    let resource_nodes: Vec<ResourceNode> = sqlx::query_as(
        "SELECT id, resource_type, position_x, position_y, base_quality, state,
         harvested_by, regenerates_at_tick, created_at
         FROM resource_nodes"
    )
    .fetch_all(&db_pool)
    .await?;

    let node_count = resource_nodes.len();
    {
        let mut game = game_state.write().await;
        for node in resource_nodes {
            let node_state: ResourceNodeState = node.into();
            game.resource_nodes.insert(node_state.id, node_state);
        }
    }
    tracing::info!("Loaded {} resource nodes", node_count);

    // Load item types into game state
    let item_types: Vec<ItemType> = sqlx::query_as(
        "SELECT id, name, category, stackable, weight FROM item_types"
    )
    .fetch_all(&db_pool)
    .await?;

    {
        let mut game = game_state.write().await;
        for item_type in item_types {
            tracing::info!("Loaded item type: {}", item_type.name);
            game.item_types.insert(item_type.id.clone(), item_type);
        }
    }

    // Load recipes into game state
    let recipe_rows: Vec<RecipeRow> = sqlx::query_as(
        "SELECT id, name, description, category, inputs, outputs, base_duration_ticks, quality_weights, crafting_requirements, required_station_type
         FROM recipes"
    )
    .fetch_all(&db_pool)
    .await?;

    {
        let mut game = game_state.write().await;
        for row in recipe_rows {
            let recipe: Recipe = row.into();
            tracing::info!("Loaded recipe: {}", recipe.name);
            game.recipes.insert(recipe.id.clone(), recipe);
        }
    }

    // Load station types into game state
    let station_types: Vec<StationType> = sqlx::query_as(
        "SELECT id, name, category, slot_count, layout_columns, interaction_range, icon, description
         FROM station_types"
    )
    .fetch_all(&db_pool)
    .await?;

    {
        let mut game = game_state.write().await;
        for st in station_types {
            tracing::info!("Loaded station type: {}", st.name);
            game.station_types.insert(st.id.clone(), st);
        }
    }

    // Load stations into game state
    let stations: Vec<Station> = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y, placed_at FROM stations"
    )
    .fetch_all(&db_pool)
    .await?;

    // Get containers for stations
    let station_ids: Vec<uuid::Uuid> = stations.iter().map(|s| s.id).collect();
    let station_containers: Vec<(uuid::Uuid, uuid::Uuid)> = if !station_ids.is_empty() {
        sqlx::query_as(
            "SELECT station_id, id FROM containers WHERE station_id = ANY($1)"
        )
        .bind(&station_ids)
        .fetch_all(&db_pool)
        .await?
    } else {
        Vec::new()
    };

    let container_map: std::collections::HashMap<uuid::Uuid, uuid::Uuid> = station_containers.into_iter().collect();

    let station_count = stations.len();
    {
        let mut game = game_state.write().await;
        for station in stations {
            let container_id = container_map.get(&station.id).copied();
            let state = StationState::from_station(&station, container_id);
            game.stations.insert(state.id, state);
        }
    }
    tracing::info!("Loaded {} stations", station_count);

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
        .nest("/world", routes::world::router(app_state.clone()))
        .nest("/recipes", routes::recipes::router(app_state.clone()))
        .nest("/inventory", routes::inventory::router(app_state.clone()))
        .nest("/stations", routes::stations::router(app_state.clone()))
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
