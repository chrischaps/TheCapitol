use axum::{
    extract::{Extension, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{ActionState, TerrainConfig, WorldGenParams};
use crate::state::AppState;
use crate::engine::currency::{announce_balances, balances_of};
use crate::engine::node_placement;
use crate::engine::plot_sync::{sync_plots, OnOwned, SyncError};
use crate::engine::GameEvent;

fn require_admin(user: &AuthUser) -> Result<(), AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    Ok(())
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/world-params", get(get_world_params))
        .route("/world-params", put(update_world_params))
        .route("/world-params/default", get(get_default_world_params))
        .route("/world-params/commit", post(commit_world_params))
        .route("/stats", get(get_server_stats))
        .route("/accounts/{id}/subscription", post(subscribe_account))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Start or extend an account's subscription by `days` (E4.1). There are no
/// payments yet, so this is how one starts.
pub async fn subscribe_account(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    axum::extract::Path(account_id): axum::extract::Path<uuid::Uuid>,
    Json(req): Json<capitol_protocol::rest::SubscribeRequest>,
) -> Result<Json<capitol_protocol::rest::Subscription>, AppError> {
    require_admin(&user)?;
    let subscription = crate::subscriptions::extend(&state.db, account_id, req.days)
        .await?
        .ok_or_else(|| AppError::NotFound("Account not found".into()))?;
    tracing::info!("Admin {} subscribed account {} for {} days", user.account_id, account_id, req.days);
    Ok(Json(subscription))
}

/// Get current world generation parameters
pub async fn get_world_params(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<WorldGenParams>, AppError> {
    require_admin(&user)?;

    let game = state.game.read().await;
    Ok(Json(game.world_gen_params.clone()))
}

/// The default world generation parameters, for the admin form's reset. The
/// client keeps no copy of them, so the zone radii stay in one place (#116).
pub async fn get_default_world_params(
    Extension(user): Extension<AuthUser>,
) -> Result<Json<WorldGenParams>, AppError> {
    require_admin(&user)?;
    Ok(Json(WorldGenParams::default()))
}

/// Regenerate terrain in memory (does NOT persist to database)
pub async fn update_world_params(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(params): Json<WorldGenParams>,
) -> Result<Json<WorldGenParams>, AppError> {
    require_admin(&user)?;

    let mut game = state.game.write().await;

    game.set_terrain(TerrainConfig::from_params(&params));
    game.world_gen_params = params.clone();
    // In memory only, like the terrain. The commit saves where the nodes are (#70).
    let game = &mut *game;
    node_placement::fit_to_terrain(&game.terrain, &mut game.resource_nodes);

    let _ = game.event_sender.send(GameEvent::TerrainUpdated);

    tracing::info!("Admin {} regenerated terrain (uncommitted)", user.account_id);

    Ok(Json(params))
}

#[derive(Debug, Deserialize)]
pub struct CommitQuery {
    /// Release owned plots the new terrain moves or removes (refunding the
    /// deed and removing their stations) instead of refusing the commit.
    #[serde(default)]
    pub force: bool,
}

/// Commit current in-memory world params to the database.
///
/// The claimable plots follow the terrain's lots (B7), so this also syncs the
/// `plots` table. If that would move or remove an owned plot, the commit is
/// refused with 409 and changes nothing, unless `?force=true`.
pub async fn commit_world_params(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<CommitQuery>,
) -> Result<Json<WorldGenParams>, AppError> {
    require_admin(&user)?;

    let game = state.game.read().await;
    let params = game.world_gen_params.clone();
    let lots = game.terrain.terrain_plots.clone();
    // The regenerates since the last commit may have moved nodes (#70).
    let node_positions: Vec<_> = game.resource_nodes.values().map(|n| (n.id, (n.x, n.y))).collect();
    drop(game); // release lock before DB call

    let on_owned = if query.force { OnOwned::Release } else { OnOwned::Refuse };
    let report = match sync_plots(&state.db, &lots, on_owned).await {
        Ok(report) => report,
        Err(SyncError::Db(e)) => return Err(e.into()),
        Err(SyncError::Refund(e)) => return Err(AppError::Internal(format!("{e}. The commit changed nothing."))),
        Err(SyncError::OwnedConflicts(conflicts)) => {
            let list: Vec<String> = conflicts
                .iter()
                .map(|c| format!("{} ({}, {})", c.terrain_id, c.plot_id, if c.removed { "gone" } else { "reshaped" }))
                .collect();
            return Err(AppError::Conflict(format!(
                "The new terrain moves or removes {} owned plot(s): {}. Commit with ?force=true to release them, refund their deeds and remove their stations.",
                conflicts.len(),
                list.join(", ")
            )));
        }
    };
    if !report.removed_stations.is_empty() {
        let mut game = state.game.write().await;
        for station_id in &report.removed_stations {
            game.stations.remove(station_id);
            let _ = game.event_sender.send(GameEvent::StationRemoved { station_id: *station_id });
        }
    }
    if !report.released.is_empty() {
        let game = state.game.read().await;
        for terrain_id in report.released {
            let _ = game.event_sender.send(GameEvent::PlotReleased { terrain_id });
        }
    }
    if !report.refunded.is_empty() {
        match balances_of(&state.db, &report.refunded).await {
            Ok(balances) => announce_balances(&state.game.read().await.event_sender, balances),
            Err(e) => tracing::error!("Couldn't read the refunded owners' balances: {:?}", e),
        }
    }
    tracing::info!(
        "Admin {} commit synced plots: {} added, {} updated, {} removed, {} owned released",
        user.account_id, report.inserted, report.updated, report.deleted,
        if query.force { report.conflicts.len() } else { 0 }
    );

    let params_json = serde_json::to_value(&params)
        .map_err(|e| AppError::Internal(format!("Failed to serialize params: {}", e)))?;

    sqlx::query(
        "INSERT INTO world_config (id, params, updated_at) VALUES (1, $1, $2)
         ON CONFLICT (id) DO UPDATE SET params = $1, updated_at = $2"
    )
    .bind(&params_json)
    .bind(Utc::now())
    .execute(&state.db)
    .await?;

    let moved = node_placement::save_positions(&state.db, &node_positions).await?;

    tracing::info!(
        "Admin {} committed world params to database ({} resource nodes saved where the terrain moved them)",
        user.account_id, moved
    );

    Ok(Json(params))
}

#[derive(Serialize)]
pub struct ServerStats {
    pub tps: f64,
    pub tick_count: u64,
    pub uptime_seconds: f64,
    pub active_players: usize,
    pub active_crafting: usize,
    pub active_extracting: usize,
    pub active_trades: usize,
    pub resource_nodes: usize,
    pub station_count: usize,
    pub command_queue_size: usize,
    pub broadcast_subscribers: usize,
}

/// Get real-time server performance stats
pub async fn get_server_stats(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<ServerStats>, AppError> {
    require_admin(&user)?;

    let game = state.game.read().await;

    let active_extracting = game
        .players
        .values()
        .filter(|p| matches!(p.action_state, ActionState::Extracting { .. }))
        .count();

    let stats = ServerStats {
        tps: game.last_tps,
        tick_count: game.tick_count,
        uptime_seconds: game.uptime_seconds,
        active_players: game.players.len(),
        active_crafting: game.crafting_operations.len(),
        active_extracting,
        active_trades: game.trade_manager.active_trades.len(),
        resource_nodes: game.resource_nodes.len(),
        station_count: game.stations.len(),
        command_queue_size: game.command_queue.len(),
        broadcast_subscribers: game.event_sender.receiver_count(),
    };

    Ok(Json(stats))
}
