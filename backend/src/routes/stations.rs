use axum::{
    extract::{Path, State},
    routing::{delete, get, post, put},
    Extension, Json, Router,
};
use std::collections::HashMap;
use uuid::Uuid;

use capitol_protocol::rest::{
    ConfigureAutomation, NearbyStationsResponse, RepairStationResponse, StationAutomation, PlaceStationRequest, PlaceStationResponse, StationContainerResponse,
    SuccessResponse,
};

use capitol_protocol::rules::weather;

use crate::engine::automation::AutomationError;
use crate::engine::container_access::{check_station_use, live_position, AccessError};
use crate::engine::GameEvent;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{Container, ContainerType, Item, ItemType, SlotItem, Station, StationInfo, StationType, ITEM_COLUMNS, ITEM_TYPE_COLUMNS, STATION_COLUMNS, STATION_TYPE_COLUMNS};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/nearby", get(get_nearby_stations))
        .route("/place", post(place_station))
        .route("/{id}", delete(remove_station))
        .route("/{id}/container", get(get_station_container))
        .route("/{id}/automation", put(configure_automation))
        .route("/{id}/automation/start", post(start_automation))
        .route("/{id}/automation/stop", post(stop_automation))
        .route("/{id}/repair", post(repair_station))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Get all stations near the player
#[utoipa::path(
    get,
    path = "/stations/nearby",
    responses(
        (status = 200, description = "Nearby stations", body = NearbyStationsResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn get_nearby_stations(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<NearbyStationsResponse>, AppError> {
    // Get player position
    let player: (Uuid, f64, f64) = sqlx::query_as(
        "SELECT id, position_x, position_y FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    let (_player_id, player_x, player_y) = player;

    // Get station types for metadata
    let station_types: Vec<StationType> = sqlx::query_as(&format!("SELECT {STATION_TYPE_COLUMNS} FROM station_types"))
    .fetch_all(&state.db)
    .await?;

    let type_map: HashMap<String, StationType> = station_types
        .into_iter()
        .map(|st| (st.id.clone(), st))
        .collect();

    // Get all stations within a generous range (500 units for visibility)
    let stations: Vec<Station> = sqlx::query_as(&format!(
        "SELECT {STATION_COLUMNS}
         FROM stations
         WHERE position_x BETWEEN $1 - 500 AND $1 + 500
           AND position_y BETWEEN $2 - 500 AND $2 + 500"
    ))
    .bind(player_x)
    .bind(player_y)
    .fetch_all(&state.db)
    .await?;

    // Get containers for these stations
    let station_ids: Vec<Uuid> = stations.iter().map(|s| s.id).collect();
    let containers: Vec<(Uuid, Uuid)> = if !station_ids.is_empty() {
        sqlx::query_as(
            "SELECT station_id, id FROM containers WHERE station_id = ANY($1)"
        )
        .bind(&station_ids)
        .fetch_all(&state.db)
        .await?
    } else {
        Vec::new()
    };

    let container_map: HashMap<Uuid, Uuid> = containers.into_iter().collect();
    let tools = crate::engine::station_tools::tools_at(&state.db, &station_ids).await?;
    let automation = crate::engine::automation::automation_at(&state.db, &station_ids).await?;
    let mut production = state.game.read().await.works.production(&station_ids);

    // Build response
    let station_infos: Vec<StationInfo> = stations
        .iter()
        .filter_map(|station| {
            let st = type_map.get(&station.station_type)?;
            Some(StationInfo {
                id: station.id,
                station_type: station.station_type.clone(),
                name: st.name.clone(),
                category: st.category.clone(),
                icon: st.icon.clone(),
                x: station.position_x as f64,
                y: station.position_y as f64,
                owner_id: station.owner_id,
                container_id: container_map.get(&station.id).copied(),
                interaction_range: st.interaction_range,
                quality: station.quality,
                tool: tools.get(&station.id).cloned(),
                automation: automation.get(&station.id).cloned(),
                production: production.remove(&station.id).unwrap_or_default(),
                indoors: station.indoors,
                condition: weather::shown(station.condition),
                floor: station.floor,
            })
        })
        .collect();

    Ok(Json(NearbyStationsResponse { stations: station_infos }))
}

/// Place a new station: on a plot you own, or, for a workbench, on dry ground no plot covers (#187)
#[utoipa::path(
    post,
    path = "/stations/place",
    request_body = PlaceStationRequest,
    responses(
        (status = 200, description = "Station placed", body = PlaceStationResponse),
        (status = 400, description = "Invalid placement (out of range, wrong item, plot full, off a plot, in water)"),
        (status = 403, description = "On a plot you don't own")
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn place_station(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<PlaceStationRequest>,
) -> Result<Json<PlaceStationResponse>, AppError> {
    let player_id = sqlx::query_scalar::<_, Uuid>("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(auth_user.account_id)
        .fetch_one(&state.db)
        .await?;

    let placed = crate::engine::station_placement::place_station(
        &state,
        player_id,
        &req.station_type,
        req.x,
        req.y,
        req.floor,
        req.kit_item_id,
    )
    .await?;
    placed.remember(&state).await;
    Ok(Json(PlaceStationResponse { station: placed.info }))
}

/// Take down a station you own. Its kit, its tool and your things in its box go back into your
/// inventory, and a guest's deposit to the guest (#489).
#[utoipa::path(
    delete,
    path = "/stations/{id}",
    params(
        ("id" = Uuid, Path, description = "Station ID")
    ),
    responses(
        (status = 200, description = "Station removed, kit and box returned"),
        (status = 400, description = "Your inventory has no room for the kit, its tool and what's in the box"),
        (status = 403, description = "Not your station"),
        (status = 404, description = "Station not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn remove_station(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(station_id): Path<Uuid>,
) -> Result<Json<SuccessResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    let removed = crate::engine::station_removal::remove_station(&state.db, player_id, station_id).await?;

    let mut game = state.game.write().await;
    game.stations.remove(&station_id);
    let _ = game.event_sender.send(GameEvent::StationRemoved { station_id });
    // The kit and the box's contents, to whoever they went to (#489)
    for (player_id, container_id) in removed.returned_to {
        let _ = game.event_sender.send(GameEvent::InventoryUpdated { player_id, container_id, slots: vec![] });
    }
    drop(game);

    Ok(Json(SuccessResponse::ok()))
}

/// Get a station's inventory container
#[utoipa::path(
    get,
    path = "/stations/{id}/container",
    params(
        ("id" = Uuid, Path, description = "Station ID")
    ),
    responses(
        (status = 200, description = "Station container", body = StationContainerResponse),
        (status = 403, description = "Not your station"),
        (status = 404, description = "Station not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn get_station_container(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(station_id): Path<Uuid>,
) -> Result<Json<StationContainerResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // The one station rule: its owner, the plot's owner, or anyone the plot's
    // station_use_level admits, within range (#80), or a worker on a wage
    // shift there (Commons D7)
    let at = live_position(&state.game, player_id).await;
    check_station_use(&state.db, player_id, station_id, at).await?;

    // Get container
    let container: Container = sqlx::query_as(
        "SELECT id, container_type, owner_id, station_id, created_at
         FROM containers WHERE station_id = $1"
    )
    .bind(station_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Internal("Station has no container".into()))?;

    // Get container type
    let container_type: ContainerType = sqlx::query_as(
        "SELECT id, name, slot_count, layout_columns FROM container_types WHERE id = $1"
    )
    .bind(&container.container_type)
    .fetch_one(&state.db)
    .await?;

    // Item types, for names and tools' durability
    let item_types: Vec<ItemType> = sqlx::query_as(&format!("SELECT {ITEM_TYPE_COLUMNS} FROM item_types"))
        .fetch_all(&state.db)
        .await?;
    let item_types: HashMap<String, ItemType> =
        item_types.into_iter().map(|it| (it.id.clone(), it)).collect();

    // Get items in container
    let items: Vec<Item> = sqlx::query_as(&format!("SELECT {ITEM_COLUMNS} FROM items WHERE container_id = $1"))
        .bind(container.id)
        .fetch_all(&state.db)
        .await?;

    let slots: Vec<SlotItem> = items
        .into_iter()
        .filter_map(|item| item.to_slot_item(item_types.get(&item.item_type)))
        .collect();

    Ok(Json(StationContainerResponse {
        id: container.id,
        container_type: container.container_type,
        slot_count: container_type.slot_count,
        layout_columns: container_type.layout_columns,
        slots,
    }))
}

/// Set what your station makes on its own (E4.2): a recipe at this station's
/// type, a count or forever, from and into the station's own box. It stops the
/// station if it was running. Everyone hears `station_automation_changed`.
#[utoipa::path(
    put,
    path = "/stations/{id}/automation",
    params(
        ("id" = Uuid, Path, description = "Station ID")
    ),
    request_body = ConfigureAutomation,
    responses(
        (status = 200, description = "Set, and stopped", body = StationAutomation),
        (status = 400, description = "The recipe isn't made here, needs a tool the slot doesn't hold, or the count is out of range"),
        (status = 403, description = "Not your station"),
        (status = 404, description = "Station not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn configure_automation(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(station_id): Path<Uuid>,
    Json(req): Json<ConfigureAutomation>,
) -> Result<Json<StationAutomation>, AppError> {
    let player_id = player_of(&state, &auth_user).await?;
    let (recipes, events) = {
        let game = state.game.read().await;
        (game.recipes.clone(), game.event_sender.clone())
    };
    let result =
        crate::engine::automation::configure(&state.db, &events, &recipes, player_id, station_id, &req.recipe_id, req.mode)
            .await;
    announce_automation(&state, station_id, result).await
}

/// Set your configured station running (E4.2)
#[utoipa::path(
    post,
    path = "/stations/{id}/automation/start",
    params(
        ("id" = Uuid, Path, description = "Station ID")
    ),
    responses(
        (status = 200, description = "Running", body = StationAutomation),
        (status = 400, description = "Not configured, or its setting no longer holds (a tool taken out)"),
        (status = 403, description = "Not your station"),
        (status = 404, description = "Station not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn start_automation(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(station_id): Path<Uuid>,
) -> Result<Json<StationAutomation>, AppError> {
    let player_id = player_of(&state, &auth_user).await?;
    let recipes = state.game.read().await.recipes.clone();
    let result = crate::engine::automation::start(&state.db, &recipes, player_id, station_id).await;
    announce_automation(&state, station_id, result).await
}

/// Repair a station the weather has worn (V3.2): yours, or one its plot lets
/// you use, from within its range. It's restored to 100 for a share of what
/// its kit took, more the further it has fallen, from your pack, lowest
/// grade first.
#[utoipa::path(
    post,
    path = "/stations/{id}/repair",
    params(
        ("id" = Uuid, Path, description = "Station ID")
    ),
    responses(
        (status = 200, description = "Repaired", body = RepairStationResponse),
        (status = 400, description = "It's in good repair, you're too far, or your pack is short of the goods"),
        (status = 403, description = "Not a station you may use"),
        (status = 404, description = "Station not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn repair_station(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(station_id): Path<Uuid>,
) -> Result<Json<RepairStationResponse>, AppError> {
    let player_id = player_of(&state, &auth_user).await?;
    let at = live_position(&state.game, player_id).await;
    crate::engine::container_access::check_station_access(&state.db, player_id, station_id, at).await?;
    let recipes = state.game.read().await.recipes.clone();
    let repaired = crate::engine::weather::repair(&state.db, &recipes, player_id, station_id, chrono::Utc::now()).await?;
    let condition = weather::FULL as i32;
    let game = state.game.read().await;
    let _ = game.event_sender.send(GameEvent::StationCondition { station_id, condition });
    let _ = game.event_sender.send(GameEvent::InventoryUpdated {
        player_id,
        container_id: repaired.inventory_id,
        slots: vec![],
    });
    Ok(Json(RepairStationResponse { station_id, condition, used: repaired.used }))
}

/// Stop your station (E4.2). It keeps its setting and its count.
#[utoipa::path(
    post,
    path = "/stations/{id}/automation/stop",
    params(
        ("id" = Uuid, Path, description = "Station ID")
    ),
    responses(
        (status = 200, description = "Stopped", body = StationAutomation),
        (status = 400, description = "Not configured"),
        (status = 403, description = "Not your station"),
        (status = 404, description = "Station not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn stop_automation(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(station_id): Path<Uuid>,
) -> Result<Json<StationAutomation>, AppError> {
    let player_id = player_of(&state, &auth_user).await?;
    let result = crate::engine::automation::stop(&state.db, player_id, station_id).await;
    announce_automation(&state, station_id, result).await
}

async fn player_of(state: &AppState, auth_user: &AuthUser) -> Result<Uuid, AppError> {
    Ok(sqlx::query_scalar::<_, Uuid>("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(auth_user.account_id)
        .fetch_one(&state.db)
        .await?)
}

/// Everyone hears what the station is set to now, as over WS
async fn announce_automation(
    state: &AppState,
    station_id: Uuid,
    result: Result<StationAutomation, AutomationError>,
) -> Result<Json<StationAutomation>, AppError> {
    let automation = result.map_err(automation_refusal)?;
    let game = state.game.read().await;
    crate::engine::automation::announce(&game, station_id, automation.clone());
    Ok(Json(automation))
}

fn automation_refusal(e: AutomationError) -> AppError {
    match e {
        AutomationError::Access(AccessError::StationNotFound) => AppError::NotFound(e.message()),
        AutomationError::Access(AccessError::Database(db)) => AppError::Database(db),
        AutomationError::NotYourStation => AppError::Forbidden(e.message()),
        _ => AppError::BadRequest(e.message()),
    }
}
