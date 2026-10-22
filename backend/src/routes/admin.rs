use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Extension, Query, State},
    http::header,
    response::IntoResponse,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post, put},
    Json, Router,
};
use capitol_protocol::annotation::{AnnotationKind, AnnotationsQuery};
use capitol_protocol::metrics::{Feed, SeriesQuery, TownLatest, TownReport};
use futures::{Stream, StreamExt};
use tokio::sync::broadcast::error::RecvError;
use capitol_protocol::rest::{
    AdminBooks, AdminMap, ApiKeyScope, CitizenInfo, CreateCitizenRequest, CreatedApiKey, CreatedCitizen, Doing, FillHeat, MapPlot,
    MapQuery, MapZone, MapStation, PlayerDetail, PlayerQuery, StrandMove, StrandPoint, SuccessResponse, Walker,
    WalkersQuery, MAP_HEAT_MAX_HOURS, PLAYER_MOVES, PLAYER_POINTS, WALKERS_MAX_RADIUS,
};
use capitol_protocol::metrics::Trades;
use capitol_protocol::rules::Zone;
use tower_http::compression::CompressionLayer;
use capitol_protocol::tunables::{TunablesResponse, TunablesUpdate};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::api_keys;
use crate::citizens;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{ActionState, TerrainConfig, WorldGenParams};
use crate::state::AppState;
use crate::engine::currency::{announce_balances, balances_of};
use crate::engine::{admin_feed, annotations, ledger, load, node_placement, node_spawn, sampler, tunables};
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
        .route("/ledger", get(get_ledger))
        .route("/annotations", get(get_annotations))
        .route("/stream", get(stream))
        .route("/series", get(get_series))
        .route("/tunables", get(get_tunables).put(put_tunables))
        .route("/tunables/preset", get(get_tunables_preset).put(put_tunables_preset))
        .route("/walkers", get(get_walkers))
        .route("/positions", get(get_positions).layer(CompressionLayer::new()))
        .route("/map", get(get_map).layer(CompressionLayer::new()))
        .route("/player", get(get_player).layer(CompressionLayer::new()))
        .route("/books", get(get_books))
        .route("/accounts/{id}/subscription", post(subscribe_account))
        .route("/citizens", get(list_citizens).post(create_citizen))
        .route("/town", get(get_town))
        .route("/town/key", post(mint_town_key))
        .route("/town/report", post(post_town_report))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
        // After the auth layer, so the console's page loads without a token
        .merge(super::console::router())
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

/// Make a citizen (C0.1, `citizens`): an account of kind `citizen`, its
/// player at the spawn as any new player's, and a `play`+`trade` key, whose
/// secret is in this answer only.
pub async fn create_citizen(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<CreateCitizenRequest>,
) -> Result<Json<CreatedCitizen>, AppError> {
    require_admin(&user)?;
    let made = citizens::mint(&state.db, &req.name).await?;
    tracing::info!(
        "Admin {} made citizen {} ({})",
        user.account_id, made.citizen.name, made.citizen.account_id
    );
    Ok(Json(made))
}

/// Every citizen, oldest first, each with its newest live key (no secrets).
pub async fn list_citizens(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<CitizenInfo>>, AppError> {
    require_admin(&user)?;
    Ok(Json(citizens::list(&state.db).await?))
}

/// Mint the town's report key (D4.1) on the admin's own account, revoking
/// the one before. `capitol-town` mints it once and keeps it beside its
/// roster. The secret is in this answer only.
pub async fn mint_town_key(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<CreatedApiKey>, AppError> {
    require_admin(&user)?;
    let (key, secret) = api_keys::mint_town_key(&state.db, user.account_id).await?;
    tracing::info!("Admin {} minted the town's report key {}", user.account_id, key.id);
    Ok(Json(CreatedApiKey { key, secret }))
}

/// The town's report (D4.1), each game minute, by its key (the only route
/// that key reaches) or an admin. It's kept as the newest and added to the
/// samples' windows. One from a town coming on, or stopping, marks the
/// charts.
pub async fn post_town_report(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(report): Json<TownReport>,
) -> Result<Json<SuccessResponse>, AppError> {
    let town_key = user.key.is_some_and(|key| key.scopes.allows(ApiKeyScope::Town));
    if !town_key {
        require_admin(&user)?;
    }
    if let Some(mark) = state.town.receive(report, Utc::now().timestamp_millis()) {
        tracing::info!("{}", mark.text);
        annotations::note(&state.db, mark.kind, &mark.text).await;
    }
    Ok(Json(SuccessResponse::ok()))
}

/// The town's newest report (D4.1), each citizen with its last decisions,
/// and whether it's on; `null` if no town has reported since the server
/// started.
pub async fn get_town(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Option<TownLatest>>, AppError> {
    require_admin(&user)?;
    Ok(Json(state.town.latest()))
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
/// client keeps no copy of them, so they stay in one place (#116). The zone
/// radii aren't among them: they aren't tweakable (#118).
pub async fn get_default_world_params(
    Extension(user): Extension<AuthUser>,
) -> Result<Json<WorldGenParams>, AppError> {
    require_admin(&user)?;
    Ok(Json(WorldGenParams::default()))
}

/// Regenerate terrain in memory (does NOT persist to database)
///
/// The zone radii aren't tweakable (#118), so params that move one are
/// refused with 400 rather than quietly built at the table's radii.
pub async fn update_world_params(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(json): Json<serde_json::Value>,
) -> Result<Json<WorldGenParams>, AppError> {
    require_admin(&user)?;
    let moved = WorldGenParams::untunable_radii_in(&json);
    if !moved.is_empty() {
        let list: Vec<String> = moved.iter().map(|(key, v)| format!("{key} {v}")).collect();
        return Err(AppError::BadRequest(format!(
            "The zone radii aren't tweakable: they're the protocol's table (capitol_protocol::rules), which zone classification reads. Asked for {}.",
            list.join(", ")
        )));
    }
    let params: WorldGenParams =
        serde_json::from_value(json).map_err(|e| AppError::BadRequest(format!("Bad world params: {e}")))?;

    let mut game = state.game.write().await;

    game.set_terrain(TerrainConfig::from_params(&params));
    game.world_gen_params = params.clone();
    // In memory only, like the terrain. The commit saves where the nodes are (#70).
    let game = &mut *game;
    node_placement::fit_to_terrain(&game.terrain, &mut game.resource_nodes, &game.resource_types);
    // The woods, outcrops, veins and clay grow from the terrain, so they're
    // planned afresh (E2.3). Also in memory only until the commit.
    let replan = node_spawn::replan(&game.terrain, &game.resource_types, &mut game.resource_nodes, params.seed);
    tracing::info!(
        "Re-planned {} resource nodes: {} removed, {} added",
        replan.nodes.len(), replan.removed, replan.added
    );

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
    // And re-planned the woods, outcrops, veins and clay (E2.3).
    let planned = node_spawn::planned_in(&game.resource_types, &game.resource_nodes);
    drop(game); // release lock before DB call

    let on_owned = if query.force { OnOwned::Release } else { OnOwned::Refuse };
    let report = match sync_plots(&state.db, &lots, on_owned).await {
        Ok(report) => report,
        Err(SyncError::Db(e)) => return Err(e.into()),
        Err(SyncError::Refund(e)) => return Err(AppError::Internal(format!("{e}. The commit changed nothing."))),
        Err(SyncError::Conflicts { plots, stations }) => {
            let mut list: Vec<String> = plots
                .iter()
                .map(|c| format!("plot {} ({}, {})", c.terrain_id, c.plot_id, if c.removed { "gone" } else { "reshaped" }))
                .collect();
            list.extend(stations.iter().map(|c| format!("{} {} (under lot {})", c.station_type, c.station_id, c.terrain_id)));
            return Err(AppError::Conflict(format!(
                "The new terrain moves or removes {} owned plot(s) and lays a lot under {} station(s) off a plot: {}. Commit with ?force=true to release the plots, refund their deeds and take their stations and those down, returning what they hold.",
                plots.len(),
                stations.len(),
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
        for &(player_id, container_id) in &report.returned_to {
            let _ = game.event_sender.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
        }
        for (c, notice_id) in &report.displaced {
            let _ = game.event_sender.send(GameEvent::StationDisplaced {
                owner_id: c.owner_id,
                station_id: c.station_id,
                station_type: c.station_type.clone(),
                terrain_id: c.terrain_id.clone(),
                notice_id: Some(*notice_id),
            });
        }
    }
    let released = report.released.len();
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
        "Admin {} commit synced plots: {} added, {} updated, {} removed, {} owned released, {} stations off a plot taken down",
        user.account_id, report.inserted, report.updated, report.deleted,
        if query.force { report.conflicts.len() } else { 0 },
        report.displaced.len()
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

    let replanned = node_spawn::save_replan(&state.db, &planned).await?;
    let moved = node_placement::save_positions(&state.db, &node_positions).await?;

    tracing::info!(
        "Admin {} committed world params to database ({} resource nodes saved where the terrain moved them, {} planned ones gone)",
        user.account_id, moved, replanned
    );
    let text = format!(
        "Terrain committed: {} lots added, {} reshaped, {} gone, {} owned released; {} nodes moved",
        report.inserted, report.updated, report.deleted, released, moved
    );
    annotations::note(&state.db, AnnotationKind::TerrainCommitted, &text).await;

    Ok(Json(params))
}

/// The world's books (C1.7): every account's Strands and what its audit trail
/// says moved them, escrow, every unit and the units made and destroyed since
/// the server started, and its load. `capitol-town soak` reads it at each end
/// and reconciles the two (`capitol_protocol::ledger::reconcile`).
pub async fn get_ledger(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<capitol_protocol::ledger::Ledger>, AppError> {
    require_admin(&user)?;
    Ok(Json(ledger::snapshot(&state).await?))
}

/// The annotations (D0.4): what explains a bend in a chart, oldest first.
/// `/admin/series` and the stream (D0.3) serve them with the samples too.
pub async fn get_annotations(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<AnnotationsQuery>,
) -> Result<Json<Vec<capitol_protocol::annotation::Annotation>>, AppError> {
    require_admin(&user)?;
    let since_ms = query.since_ms.unwrap_or_else(|| Utc::now().timestamp_millis() - 24 * 3_600_000);
    let limit = query.limit.unwrap_or(1000).clamp(1, 10_000);
    Ok(Json(annotations::since(&state.db, since_ms, limit).await?))
}

/// The Observatory's live feed (D0.3), as server-sent events: each sample as
/// it's taken, each annotation as it's written, and the game's fills, deeds,
/// Gazettes and warnings (`engine::admin_feed`). Each event is named for its
/// `type` and carries a `Feed` as JSON. It opens with the newest sample, so a
/// console has something to draw before the next. A stream that falls behind
/// gets a `lagged` event saying how much went past.
///
/// It takes the bearer header like every admin route, so a browser reads it
/// with `fetch`, not `EventSource`, which can't send one.
pub async fn stream(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, AppError> {
    require_admin(&user)?;
    let feed = admin_feed::subscribe();
    let first = state.samples.latest().map(|s| Arc::new(Feed::Sample(Box::new(s))));
    let live = futures::stream::unfold(feed, |mut feed| async move {
        let next = match feed.recv().await {
            Ok(next) => next,
            Err(RecvError::Lagged(missed)) => {
                load::Receiver::AdminStream.lagged(missed);
                Arc::new(Feed::Lagged { missed })
            }
            Err(RecvError::Closed) => return None,
        };
        Some((next, feed))
    });
    let events = futures::stream::iter(first).chain(live).map(|feed| {
        Ok(SseEvent::default().event(feed.type_name()).json_data(&*feed).unwrap_or_else(|e| {
            tracing::error!("Couldn't write a {} to an admin stream: {e}", feed.type_name());
            SseEvent::default().comment("unwritable")
        }))
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// The samples and annotations between two times (D0.3), oldest first, as
/// `capitol_protocol::metrics::Series`: the newest day from the ring, ten
/// seconds apart, and before it a minute apart from `metrics_samples`.
pub async fn get_series(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<SeriesQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&user)?;
    let to_ms = query.to_ms.unwrap_or_else(|| Utc::now().timestamp_millis());
    let from_ms = query.from_ms.unwrap_or(to_ms - 3_600_000);
    if from_ms >= to_ms {
        return Err(AppError::BadRequest("from_ms must be before to_ms".into()));
    }
    let fields: Option<Vec<String>> = query.fields.as_deref().map(|f| {
        f.split(',').map(str::trim).filter(|f| !f.is_empty()).map(str::to_string).collect()
    });
    let samples = sampler::series(&state.samples, &state.db, from_ms, to_ms, fields.as_deref()).await?;
    // `since` counts from its time inclusive; the series' start is exclusive
    let annotations: Vec<_> = annotations::since(&state.db, from_ms + 1, 10_000)
        .await?
        .into_iter()
        .filter(|a| a.at_ms <= to_ms)
        .collect();
    Ok(Json(serde_json::json!({ "samples": samples, "annotations": annotations })))
}

/// The zones and resources the tunables' families may name, as the world has them now
async fn known_ids(state: &AppState) -> Result<tunables::Known, AppError> {
    let resources: Vec<String> = state.game.read().await.resource_types.keys().cloned().collect();
    Ok(tunables::Known::read(&state.db, resources).await?)
}

/// Every live tunable (D6.1) with its unit, range, default and value, one per
/// zone with deeds and per resource for the families.
pub async fn get_tunables(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<TunablesResponse>, AppError> {
    require_admin(&user)?;
    let known = known_ids(&state).await?;
    Ok(Json(known.list(&state.tunables.get())))
}

/// Change tunables, all of them or none: an unknown key or a value out of its
/// range is a 400 that changes nothing. Each change is saved and marked on
/// the console's charts, and a fee's is sent to every socket. Answers with
/// every knob, as `GET` does.
pub async fn put_tunables(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(update): Json<TunablesUpdate>,
) -> Result<Json<TunablesResponse>, AppError> {
    require_admin(&user)?;
    let known = known_ids(&state).await?;
    set_tunables(&state, &user, &update, &known).await
}

/// Every knob as a TOML preset to save and edit (D6.2)
pub async fn get_tunables_preset(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_admin(&user)?;
    let known = known_ids(&state).await?;
    let knobs = known.list(&state.tunables.get()).knobs;
    let text = tunables::preset::write(&knobs, &Utc::now().format("%Y-%m-%d %H:%M UTC").to_string());
    Ok(([(header::CONTENT_TYPE, "application/toml; charset=utf-8")], text))
}

/// Run the world on a preset: every knob it names to its value, and the rest
/// to their defaults, all of them or none, as `PUT /admin/tunables`. The body
/// is the preset's TOML. Answers with every knob.
pub async fn put_tunables_preset(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    text: String,
) -> Result<Json<TunablesResponse>, AppError> {
    require_admin(&user)?;
    let known = known_ids(&state).await?;
    let knobs = known.list(&state.tunables.get()).knobs;
    let update = tunables::preset::read(&text, &knobs).map_err(AppError::BadRequest)?;
    set_tunables(&state, &user, &update, &known).await
}

async fn set_tunables(
    state: &AppState,
    user: &AuthUser,
    update: &TunablesUpdate,
    known: &tunables::Known,
) -> Result<Json<TunablesResponse>, AppError> {
    let events = state.game.read().await.event_sender.clone();
    match tunables::update(&state.db, &state.tunables, update, known, &events).await {
        Ok(changes) => {
            for change in &changes {
                tracing::info!("Admin {} set {} to {} (was {})", user.account_id, change.key, change.to, change.from);
            }
        }
        Err(tunables::UpdateError::Db(e)) => return Err(e.into()),
        Err(e) => return Err(AppError::BadRequest(e.message(update))),
    }
    Ok(Json(known.list(&state.tunables.get())))
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

/// The players near a point as the server is moving them (D5.3): where each
/// stands, where it's walking to and what it's doing, for the Flint client's
/// admin layers. A citizen's destination is its next leg.
pub async fn get_walkers(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<WalkersQuery>,
) -> Result<Json<Vec<Walker>>, AppError> {
    require_admin(&user)?;
    Ok(Json(walkers_near(&state.game.read().await.players, query)))
}

fn walkers_near(
    players: &std::collections::HashMap<uuid::Uuid, crate::models::PlayerState>,
    query: WalkersQuery,
) -> Vec<Walker> {
    let radius = query.radius.clamp(0.0, WALKERS_MAX_RADIUS);
    walkers(players.values().filter(|p| (p.x - query.x).powi(2) + (p.y - query.y).powi(2) <= radius * radius))
}

fn walkers<'a>(players: impl Iterator<Item = &'a crate::models::PlayerState>) -> Vec<Walker> {
    let mut out: Vec<Walker> = players
        .map(|p| Walker {
            player_id: p.id,
            name: p.name.clone(),
            citizen: p.citizen,
            x: p.x,
            y: p.y,
            dest: p.dest_x.zip(p.dest_y).map(|(x, y)| [x, y]),
            doing: match &p.action_state {
                ActionState::Idle => Doing::Idle,
                ActionState::Extracting { node_id, .. } => Doing::Extracting { node_id: *node_id },
                ActionState::Crafting { recipe_id, .. } => Doing::Crafting { recipe_id: recipe_id.clone() },
            },
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Every player in the world as the server is moving them (D1.3), for the
/// console's map, which reads it every two seconds while it's open. Gzipped:
/// a walker is about 175 bytes, so 200 are 35 KB, and gzip takes over half.
pub async fn get_positions(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<Walker>>, AppError> {
    require_admin(&user)?;
    Ok(Json(walkers(state.game.read().await.players.values())))
}

/// One player by name (D4.3), for the console's page of one citizen: their
/// Strands over the last `hours`, what moved them, and where they are.
pub async fn get_player(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<PlayerQuery>,
) -> Result<Json<PlayerDetail>, AppError> {
    require_admin(&user)?;
    let from = Utc::now() - chrono::Duration::hours(i64::from(query.hours.clamp(1, MAP_HEAT_MAX_HOURS)));
    let mut detail = player_detail(&state.db, &query.name, from)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No player is called {}", query.name)))?;
    detail.walker = walkers(state.game.read().await.players.get(&detail.player_id).into_iter()).pop();
    Ok(Json(detail))
}

/// [`get_player`] from the database, all but the walker; `None` if nobody is
/// called `name`. A citizen comes first, since no two share a name.
async fn player_detail(
    db: &sqlx::PgPool,
    name: &str,
    from: chrono::DateTime<Utc>,
) -> Result<Option<PlayerDetail>, sqlx::Error> {
    let found: Option<(uuid::Uuid, String, bool, i64)> = sqlx::query_as(
        "SELECT p.id, p.name, COALESCE(acc.kind = 'citizen', FALSE), p.strand_balance
         FROM players p
         JOIN accounts acc ON acc.id = p.account_id
         WHERE lower(p.name) = lower($1)
         ORDER BY COALESCE(acc.kind = 'citizen', FALSE) DESC, p.created_at
         LIMIT 1",
    )
    .bind(name.trim())
    .fetch_optional(db)
    .await?;
    let Some((player_id, name, citizen, strands)) = found else { return Ok(None) };

    let escrow: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(escrow_strands), 0)::int8 FROM market_orders WHERE player_id = $1")
            .bind(player_id)
            .fetch_one(db)
            .await?;

    // A move's amount is its balance after less before, whatever sign its
    // `amount` was written with
    let by_kind: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT transaction_type, SUM(balance_after - balance_before)::int8, COUNT(*)
         FROM currency_transactions
         WHERE player_id = $1 AND occurred_at >= $2
         GROUP BY transaction_type",
    )
    .bind(player_id)
    .bind(from)
    .fetch_all(db)
    .await?;
    let moves = by_kind.iter().map(|(_, _, n)| *n as u64).sum::<u64>();

    let opening: Option<i64> = sqlx::query_scalar(
        "SELECT balance_before FROM currency_transactions
         WHERE player_id = $1 AND occurred_at >= $2
         ORDER BY occurred_at, id
         LIMIT 1",
    )
    .bind(player_id)
    .bind(from)
    .fetch_optional(db)
    .await?;

    // Past PLAYER_POINTS moves, the range in even slices, each its last
    let slice_ms = ((Utc::now() - from).num_milliseconds() / PLAYER_POINTS as i64).max(1);
    let points: Vec<(chrono::DateTime<Utc>, i64)> = if moves as usize <= PLAYER_POINTS {
        sqlx::query_as(
            "SELECT occurred_at, balance_after FROM currency_transactions
             WHERE player_id = $1 AND occurred_at >= $2
             ORDER BY occurred_at, id",
        )
        .bind(player_id)
        .bind(from)
        .fetch_all(db)
        .await?
    } else {
        sqlx::query_as(
            "SELECT DISTINCT ON (date_bin($3 * interval '1 millisecond', occurred_at, $2))
                    occurred_at, balance_after
             FROM currency_transactions
             WHERE player_id = $1 AND occurred_at >= $2
             ORDER BY date_bin($3 * interval '1 millisecond', occurred_at, $2), occurred_at DESC, id DESC",
        )
        .bind(player_id)
        .bind(from)
        .bind(slice_ms as f64)
        .fetch_all(db)
        .await?
    };

    let recent: Vec<(chrono::DateTime<Utc>, String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT occurred_at, transaction_type, balance_after - balance_before, balance_after, description
         FROM currency_transactions
         WHERE player_id = $1
         ORDER BY occurred_at DESC, id DESC
         LIMIT $2",
    )
    .bind(player_id)
    .bind(PLAYER_MOVES as i64)
    .fetch_all(db)
    .await?;

    Ok(Some(PlayerDetail {
        player_id,
        name,
        citizen,
        strands,
        escrow,
        from_ms: from.timestamp_millis(),
        opening: opening.unwrap_or(strands),
        balance: points
            .into_iter()
            .map(|(at, strands)| StrandPoint { at_ms: at.timestamp_millis(), strands })
            .collect(),
        moves,
        flows: by_kind.into_iter().map(|(kind, sum, _)| (kind, sum)).collect(),
        recent: recent
            .into_iter()
            .map(|(at, kind, amount, strands, description)| StrandMove {
                at_ms: at.timestamp_millis(),
                kind,
                amount,
                strands,
                description,
            })
            .collect(),
        walker: None,
    }))
}

/// How the console reads a sample's Strand flows (D2.1): which transaction
/// types are faucets and which sinks, what a new account starts with, and
/// how long a game day is.
pub async fn get_books(Extension(user): Extension<AuthUser>) -> Result<Json<AdminBooks>, AppError> {
    require_admin(&user)?;
    Ok(Json(AdminBooks::current()))
}

/// The console map's layers that change slowly (D1.3): the owned lots, the
/// stations, and where the last `hours` of trade happened.
pub async fn get_map(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<MapQuery>,
) -> Result<Json<AdminMap>, AppError> {
    require_admin(&user)?;
    let hours = query.hours.clamp(1, MAP_HEAT_MAX_HOURS);
    let from = Utc::now() - chrono::Duration::hours(i64::from(hours));

    let plots: Vec<(String, bool)> = sqlx::query_as(
        "SELECT p.terrain_id, COALESCE(acc.kind = 'citizen', FALSE)
         FROM plots p
         JOIN players pl ON pl.id = p.owner_id
         JOIN accounts acc ON acc.id = pl.account_id
         WHERE p.terrain_id IS NOT NULL
         ORDER BY p.terrain_id",
    )
    .fetch_all(&state.db)
    .await?;

    let stations: Vec<(uuid::Uuid, String, f64, f64, bool)> = sqlx::query_as(
        "SELECT s.id, s.station_type, s.position_x::float8, s.position_y::float8,
                COALESCE(acc.kind = 'citizen', FALSE)
         FROM stations s
         LEFT JOIN players pl ON pl.id = s.owner_id
         LEFT JOIN accounts acc ON acc.id = pl.account_id
         ORDER BY s.id",
    )
    .fetch_all(&state.db)
    .await?;

    let market: (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(quantity), 0)::int8, COALESCE(SUM(price * quantity), 0)::int8
         FROM market_trades WHERE at > $1",
    )
    .bind(from)
    .fetch_one(&state.db)
    .await?;

    let stalls: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT stall_id, COUNT(*), SUM(quantity)::int8, SUM(price * quantity)::int8
         FROM bazaar_sales WHERE at > $1
         GROUP BY stall_id ORDER BY stall_id",
    )
    .bind(from)
    .fetch_all(&state.db)
    .await?;

    let trades = |(trades, units, strands): (i64, i64, i64)| Trades { trades: trades as u64, units, strands };
    Ok(Json(AdminMap {
        zones: Zone::ALL
            .iter()
            .map(|z| {
                let (inner, outer) = z.radii();
                MapZone { zone: z.as_str().to_string(), inner, outer }
            })
            .collect(),
        plots: plots.into_iter().map(|(terrain_id, citizen)| MapPlot { terrain_id, citizen }).collect(),
        stations: stations
            .into_iter()
            .map(|(id, station_type, x, y, citizen)| MapStation { id, station_type, x, y, citizen })
            .collect(),
        heat: FillHeat {
            from_ms: from.timestamp_millis(),
            market: trades(market),
            stalls: stalls.into_iter().map(|(id, n, units, strands)| (id, trades((n, units, strands)))).collect(),
        },
    }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PlayerState;
    use uuid::Uuid;

    fn player(name: &str, x: f64, y: f64, citizen: bool) -> PlayerState {
        PlayerState {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            name: name.into(),
            x,
            y,
            dest_x: None,
            dest_y: None,
            speed: 50.0,
            strand_balance: 0,
            current_zone: None,
            action_state: ActionState::Idle,
            citizen,
            steer: None,
        }
    }

    #[test]
    fn walkers_are_the_players_near_with_their_destinations() {
        let mut ada = player("Ada", 6800.0, 6800.0, true);
        ada.dest_x = Some(6900.0);
        ada.dest_y = Some(6750.0);
        let mut bram = player("Bram", 6900.0, 6800.0, false);
        bram.action_state = ActionState::Crafting {
            operation_id: Uuid::new_v4(),
            recipe_id: "rope".into(),
            progress: 0,
            duration: 10,
        };
        let far = player("Cora", 6800.0 + WALKERS_MAX_RADIUS + 1.0, 6800.0, true);
        let players: std::collections::HashMap<Uuid, PlayerState> =
            [ada, bram, far].into_iter().map(|p| (p.id, p)).collect();

        let near = walkers_near(&players, WalkersQuery { x: 6800.0, y: 6800.0, radius: 150.0 });
        let names: Vec<&str> = near.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Ada", "Bram"]);
        assert_eq!(near[0].dest, Some([6900.0, 6750.0]));
        assert!(near[0].citizen);
        assert_eq!(near[1].doing, Doing::Crafting { recipe_id: "rope".into() });
        assert_eq!(near[1].dest, None);
        // The radius is held to the most it looks
        let all = walkers_near(&players, WalkersQuery { x: 6800.0, y: 6800.0, radius: 1e9 });
        assert_eq!(all.len(), 2);
    }

    /// These need the local Postgres (`docker-compose up -d`).
    mod db {
        use super::super::player_detail;
        use crate::engine::currency::{add_strands, subtract_strands, TransactionType};
        use crate::routes::auth::create_account;
        use capitol_protocol::rest::{AccountKind, PLAYER_POINTS};
        use chrono::Utc;
        use sqlx::PgPool;
        use uuid::Uuid;

        async fn make(db: &PgPool, name: &str, kind: AccountKind) -> Uuid {
            let mut tx = db.begin().await.unwrap();
            let (_, player) = create_account(&mut tx, &format!("{name}@test.local"), "x", name, kind).await.unwrap();
            tx.commit().await.unwrap();
            player
        }

        async fn balance(db: &PgPool, player: Uuid) -> i64 {
            sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1").bind(player).fetch_one(db).await.unwrap()
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_players_strands_over_the_range_and_their_newest_moves(db: PgPool) {
            let ada = make(&db, "Ada Reeder", AccountKind::Citizen).await;
            let start = balance(&db, ada).await;
            // Before the range: a move an hour and a half ago
            add_strands(&db, ada, 40, TransactionType::ExchangeDeposit, Some("labour"), None).await.unwrap();
            sqlx::query("UPDATE currency_transactions SET occurred_at = NOW() - interval '90 minutes' WHERE player_id = $1")
                .bind(ada)
                .execute(&db)
                .await
                .unwrap();
            let opening = balance(&db, ada).await;
            add_strands(&db, ada, 25, TransactionType::MarketSale, None, None).await.unwrap();
            subtract_strands(&db, ada, 10, TransactionType::MarketFee, None, None).await.unwrap();
            subtract_strands(&db, ada, 5, TransactionType::MarketFee, None, None).await.unwrap();

            let from = Utc::now() - chrono::Duration::hours(1);
            let d = player_detail(&db, "ada reeder", from).await.unwrap().expect("found by name, any case");
            assert_eq!((d.player_id, d.name.as_str(), d.citizen), (ada, "Ada Reeder", true));
            assert_eq!((d.opening, d.strands, d.escrow), (opening, start + 50, 0));
            assert_eq!(d.moves, 3, "the deposit is before the range");
            let strands: Vec<i64> = d.balance.iter().map(|p| p.strands).collect();
            assert_eq!(strands, [opening + 25, opening + 15, opening + 10]);
            assert_eq!(d.flows["market_sale"], 25);
            assert_eq!(d.flows["market_fee"], -15, "a sink reads negative");
            assert!(!d.flows.contains_key("exchange_deposit"));
            let recent: Vec<(&str, i64)> = d.recent.iter().map(|m| (m.kind.as_str(), m.amount)).collect();
            assert_eq!(
                recent,
                [("market_fee", -5), ("market_fee", -10), ("market_sale", 25), ("exchange_deposit", 40)],
                "newest first, before the range too"
            );
            assert_eq!(d.recent[3].description.as_deref(), Some("labour"));

            assert!(player_detail(&db, "Nobody", from).await.unwrap().is_none());
            // Nothing in the range: the line is flat at what they hold
            let quiet = player_detail(&db, "Ada Reeder", Utc::now() + chrono::Duration::seconds(5)).await.unwrap().unwrap();
            assert_eq!((quiet.moves, quiet.opening, quiet.balance.len()), (0, quiet.strands, 0));
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_busy_players_moves_are_thinned_to_the_last_in_each_slice(db: PgPool) {
            let bo = make(&db, "Bo", AccountKind::Human).await;
            let start = balance(&db, bo).await;
            // Three moves a minute over the hour, each one Strand up
            let n = 3 * 60;
            let many = PLAYER_POINTS + n;
            sqlx::query(
                "INSERT INTO currency_transactions (player_id, amount, balance_before, balance_after, transaction_type, occurred_at)
                 SELECT $1, 1, $2 + k - 1, $2 + k, 'market_sale', NOW() - interval '59 minutes' + (k * interval '1 second' * 3540 / $3)
                 FROM generate_series(1, $3) AS k",
            )
            .bind(bo)
            .bind(start)
            .bind(many as i64)
            .execute(&db)
            .await
            .unwrap();
            sqlx::query("UPDATE players SET strand_balance = $2 WHERE id = $1")
                .bind(bo)
                .bind(start + many as i64)
                .execute(&db)
                .await
                .unwrap();

            let d = player_detail(&db, "Bo", Utc::now() - chrono::Duration::hours(1)).await.unwrap().unwrap();
            assert_eq!(d.moves, many as u64);
            assert!(d.balance.len() <= PLAYER_POINTS && d.balance.len() > PLAYER_POINTS / 2, "{}", d.balance.len());
            assert!(d.balance.windows(2).all(|w| w[0].at_ms <= w[1].at_ms && w[0].strands < w[1].strands));
            assert_eq!(d.balance.last().unwrap().strands, start + many as i64, "the last slice keeps the newest");
            assert_eq!((d.opening, d.flows["market_sale"]), (start, many as i64));
        }
    }
}
