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
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

mod api_keys;
mod config;
mod content;
mod engine;
mod error;
mod middleware;
mod migrations;
mod models;
mod openapi;
mod rate_limit;
mod routes;
mod sessions;
mod state;
mod ws;

use config::Config;
use models::{ContainerType, ItemType, Recipe, RecipeRow, ResourceNode, ResourceNodeState, ResourceType, Station, StationState, StationType};
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
    migrations::run(&db_pool).await?;
    tracing::info!("Migrations complete");

    // The zone radii live in capitol_protocol::rules. The zones table follows
    // them, so /bureaucracy/zones serves the radii the game uses (#116).
    let zones = engine::zone_sync::sync_zone_radii(&db_pool).await?;
    if zones.updated > 0 {
        tracing::info!("Zone radii synced from the protocol: {} rows updated", zones.updated);
    }
    for id in &zones.missing {
        tracing::warn!("Zone {id} has no row in the zones table");
    }

    // The static game data comes from backend/content/*.toml (E0.3). Write it
    // into its tables before anything below loads them.
    let synced = match content::load_and_sync(&db_pool).await {
        Ok(synced) => synced,
        Err(e) => {
            tracing::error!("Content not loaded: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!(
        "Content synced: {} rows upserted, {} retired rows kept",
        synced.upserted, synced.retired
    );

    // Redis connection
    let redis_conn = state::connect_redis(&config.redis_url).await?;
    tracing::info!("Connected to Redis");

    // Load world gen params from DB (if previously saved by admin), else use defaults
    let saved_params: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT params FROM world_config WHERE id = 1"
    )
    .fetch_optional(&db_pool)
    .await?;

    let world_gen_params: models::WorldGenParams = if let Some((json,)) = saved_params {
        match serde_json::from_value(json) {
            Ok(params) => {
                tracing::info!("Loaded world gen params from database");
                params
            }
            Err(e) => {
                tracing::warn!("Failed to deserialize saved world params, using defaults: {}", e);
                models::WorldGenParams::default()
            }
        }
    } else {
        tracing::info!("No saved world params, using defaults");
        models::WorldGenParams::default()
    };

    // Initialize game state with loaded params
    let game_state = Arc::new(RwLock::new(engine::GameState::with_params(world_gen_params)));

    // The claimable plots are the terrain's lots (B7). Bring the table up to
    // date with them. An owned plot the terrain no longer matches is left as it
    // is: only an admin's forced commit releases one.
    let lots = game_state.read().await.terrain.terrain_plots.clone();
    let report = match engine::plot_sync::sync_plots(&db_pool, &lots, engine::plot_sync::OnOwned::Keep).await {
        Ok(report) => report,
        Err(engine::plot_sync::SyncError::Db(e)) => return Err(e.into()),
        Err(engine::plot_sync::SyncError::OwnedConflicts(_) | engine::plot_sync::SyncError::Refund(_)) => {
            unreachable!("Keep never refuses or refunds")
        }
    };
    tracing::info!(
        "Plots synced with the terrain's {} lots: {} added, {} updated, {} removed",
        lots.len(), report.inserted, report.updated, report.deleted
    );
    for c in &report.conflicts {
        tracing::warn!(
            "Owned plot {} ({}) no longer matches the terrain ({}); kept as it is",
            c.plot_id, c.terrain_id, if c.removed { "lot gone" } else { "lot reshaped" }
        );
    }

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

    // The nodes are seeded against the default world. Move any the live
    // terrain's roads or water run over, and save where they went (#70).
    let placement = {
        let mut game = game_state.write().await;
        let game = &mut *game;
        engine::node_placement::fit_to_terrain(&game.terrain, &mut game.resource_nodes)
    };
    engine::node_placement::save_moves(&db_pool, &placement.moves).await?;
    tracing::info!(
        "Resource nodes fitted to the terrain: {} moved, {} with no clear spot",
        placement.moves.len(), placement.stuck.len()
    );

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
        "SELECT id, name, category, slot_count, layout_columns, interaction_range, icon, description, kit_item
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

    // Load container types into game state
    let container_types: Vec<ContainerType> = sqlx::query_as(
        "SELECT id, name, slot_count, layout_columns FROM container_types"
    )
    .fetch_all(&db_pool)
    .await?;

    {
        let mut game = game_state.write().await;
        for ct in container_types {
            game.container_types.insert(ct.id.clone(), ct);
        }
        // Every static table is in: build what `/catalog` serves (E0.2)
        game.refresh_catalog();
        tracing::info!("Catalog version {}", game.catalog.version);
    }

    // Load stations into game state
    let stations: Vec<Station> = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y, plot_id, placed_at FROM stations"
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
    if config.rate_limits.enabled {
        tracing::info!("Rate limits: {:?}", config.rate_limits);
    } else {
        tracing::warn!("Rate limits are off (RATE_LIMITS=off)");
    }

    // Start tick engine
    let tick_state = game_state.clone();
    let tick_app_state = app_state.clone();
    tokio::spawn(async move {
        engine::run_tick_loop(tick_state, tick_app_state).await;
    });

    // Build router
    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        // Auth is limited by IP; the authenticated routes by account, in auth_middleware (E0.7)
        .nest("/auth", routes::auth::router().route_layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            rate_limit::by_ip,
        )))
        .nest("/account", routes::account::router(app_state.clone()))
        .nest("/player", routes::player::router(app_state.clone()))
        .nest("/world", routes::world::router(app_state.clone()))
        .nest("/recipes", routes::recipes::router(app_state.clone()))
        .route("/catalog", get(routes::catalog::get_catalog))
        .nest("/inventory", routes::inventory::router(app_state.clone()))
        .nest("/stations", routes::stations::router(app_state.clone()))
        .nest("/exchange", routes::exchange::router(app_state.clone()))
        .nest("/plots", routes::plots::router(app_state.clone()))
        .nest("/bureaucracy", routes::bureaucracy::router(app_state.clone()))
        .nest("/terrain", routes::terrain::router(app_state.clone()))
        .nest("/admin", routes::admin::router(app_state.clone()))
        .route("/ws", get(ws::ws_handler))
        .merge(SwaggerUi::new("/swagger-ui")
            .url("/api-doc/openapi.json", openapi::ApiDoc::openapi()))
        .layer(CorsLayer::new().allow_origin(Any).allow_headers(Any).allow_methods(Any))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    // With the peer address, for the per-IP auth limit
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;

    Ok(())
}
