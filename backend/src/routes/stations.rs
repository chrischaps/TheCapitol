use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use std::collections::HashMap;
use uuid::Uuid;

use capitol_protocol::rest::{
    NearbyStationsResponse, PlaceStationRequest, PlaceStationResponse, StationContainerResponse,
    SuccessResponse,
};

use crate::engine::container_access::{check_station_access, live_position};
use crate::engine::GameEvent;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{Container, ContainerType, Item, ItemType, SlotItem, Station, StationInfo, StationType, STATION_COLUMNS};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/nearby", get(get_nearby_stations))
        .route("/place", post(place_station))
        .route("/{id}", delete(remove_station))
        .route("/{id}/container", get(get_station_container))
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
    let station_types: Vec<StationType> = sqlx::query_as(
        "SELECT id, name, category, slot_count, layout_columns, interaction_range, icon, description, kit_item
         FROM station_types"
    )
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
            })
        })
        .collect();

    Ok(Json(NearbyStationsResponse { stations: station_infos }))
}

/// Place a new station on a plot you own
#[utoipa::path(
    post,
    path = "/stations/place",
    request_body = PlaceStationRequest,
    responses(
        (status = 200, description = "Station placed", body = PlaceStationResponse),
        (status = 400, description = "Invalid placement (out of range, wrong item, plot full)"),
        (status = 403, description = "Don't own the plot")
    ),
    security(("bearer_auth" = [])),
    tag = "Stations"
)]
pub async fn place_station(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<PlaceStationRequest>,
) -> Result<Json<PlaceStationResponse>, AppError> {
    // Get player
    let (player_id, player_x, player_y): (Uuid, f64, f64) = sqlx::query_as(
        "SELECT id, position_x, position_y FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Verify station type exists
    let station_type: StationType = sqlx::query_as(
        "SELECT id, name, category, slot_count, layout_columns, interaction_range, icon, description, kit_item
         FROM station_types WHERE id = $1"
    )
    .bind(&req.station_type)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::BadRequest(format!("Unknown station type: {}", req.station_type)))?;

    // Check range from player
    let dx = req.x - player_x;
    let dy = req.y - player_y;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance > station_type.interaction_range as f64 {
        return Err(AppError::BadRequest("Too far to place station".into()));
    }

    // M7: Find plot at placement position
    let plot = crate::routes::plots::find_plot_at_position(&state.db, req.x, req.y)
        .await?
        .ok_or_else(|| AppError::BadRequest("Must place station on a plot".into()))?;

    // M7: Verify player owns the plot
    if plot.owner_id != Some(player_id) {
        return Err(AppError::Forbidden("You don't own this plot".into()));
    }

    // M7: Check plot capacity
    let (station_count, station_capacity): (i32, i32) = sqlx::query_as(
        "SELECT p.station_count, psc.station_capacity
         FROM plots p
         JOIN plot_size_categories psc ON p.size_category = psc.id
         WHERE p.id = $1"
    )
    .bind(plot.id)
    .fetch_one(&state.db)
    .await?;

    if station_count >= station_capacity {
        return Err(AppError::BadRequest(format!(
            "Plot is at capacity ({}/{})", station_count, station_capacity
        )));
    }

    // Verify player owns the kit item and it's the right type
    let expected_kit = station_type
        .kit_item
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("Invalid station type".into()))?;

    let kit_item: Item = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE id = $1 AND owner_id = $2"
    )
    .bind(req.kit_item_id)
    .bind(player_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Kit item not found".into()))?;

    if kit_item.item_type != expected_kit {
        return Err(AppError::BadRequest(format!(
            "Wrong item type: expected {}, got {}",
            expected_kit, kit_item.item_type
        )));
    }

    // Check minimum distance from other stations (50 units)
    let nearby: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM stations
         WHERE (position_x - $1)^2 + (position_y - $2)^2 < $3"
    )
    .bind(req.x)
    .bind(req.y)
    .bind(capitol_protocol::rules::STATION_MIN_SPACING * capitol_protocol::rules::STATION_MIN_SPACING)
    .fetch_optional(&state.db)
    .await?;

    if nearby.is_some() {
        return Err(AppError::BadRequest("Too close to another station".into()));
    }

    // Create station with plot_id
    let station_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(station_id)
    .bind(&req.station_type)
    .bind(player_id)
    .bind(req.x as f32)
    .bind(req.y as f32)
    .bind(plot.id)
    .bind(kit_item.quality)
    .execute(&state.db)
    .await?;

    // Increment plot station count
    sqlx::query("UPDATE plots SET station_count = station_count + 1 WHERE id = $1")
        .bind(plot.id)
        .execute(&state.db)
        .await?;

    // Create container for station
    let container_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO containers (id, container_type, station_id)
         VALUES ($1, 'station_inventory', $2)"
    )
    .bind(container_id)
    .bind(station_id)
    .execute(&state.db)
    .await?;

    // Consume the kit item
    if kit_item.quantity <= 1 {
        sqlx::query("DELETE FROM items WHERE id = $1")
            .bind(req.kit_item_id)
            .execute(&state.db)
            .await?;
    } else {
        sqlx::query("UPDATE items SET quantity = quantity - 1 WHERE id = $1")
            .bind(req.kit_item_id)
            .execute(&state.db)
            .await?;
    }

    // Return station info
    let info = StationInfo {
        id: station_id,
        station_type: req.station_type,
        name: station_type.name,
        category: station_type.category,
        icon: station_type.icon,
        x: req.x,
        y: req.y,
        owner_id: player_id,
        container_id: Some(container_id),
        interaction_range: station_type.interaction_range,
        quality: kit_item.quality,
    };

    Ok(Json(PlaceStationResponse { station: info }))
}

/// Take down a station you own. Its box must be empty, and its kit goes back into your inventory.
#[utoipa::path(
    delete,
    path = "/stations/{id}",
    params(
        ("id" = Uuid, Path, description = "Station ID")
    ),
    responses(
        (status = 200, description = "Station removed, kit returned"),
        (status = 400, description = "Its box isn't empty, or your inventory has no room for the kit"),
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
    let _ = game.event_sender.send(GameEvent::InventoryUpdated {
        player_id,
        container_id: removed.inventory_id,
        slots: vec![],
    });
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
    // station_use_level admits, within range (#80)
    let at = live_position(&state.game, player_id).await;
    check_station_access(&state.db, player_id, station_id, at).await?;

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

    // Get item types for names
    let item_types: Vec<ItemType> = sqlx::query_as(
        "SELECT id, name, category, stackable, weight FROM item_types"
    )
    .fetch_all(&state.db)
    .await?;

    let item_type_names: HashMap<String, String> = item_types
        .into_iter()
        .map(|it| (it.id, it.name))
        .collect();

    // Get items in container
    let items: Vec<Item> = sqlx::query_as(
        "SELECT id, item_type, quality, quantity, owner_id, container_id, slot_index, created_at
         FROM items WHERE container_id = $1"
    )
    .bind(container.id)
    .fetch_all(&state.db)
    .await?;

    let slots: Vec<SlotItem> = items
        .into_iter()
        .filter_map(|item| {
            let slot_index = item.slot_index?;
            let item_name = item_type_names
                .get(&item.item_type)
                .cloned()
                .unwrap_or_else(|| item.item_type.clone());

            Some(SlotItem::new(
                item.id,
                item.item_type,
                item_name,
                item.quality,
                item.quantity,
                slot_index,
            ))
        })
        .collect();

    Ok(Json(StationContainerResponse {
        id: container.id,
        container_type: container.container_type,
        slot_count: container_type.slot_count,
        layout_columns: container_type.layout_columns,
        slots,
    }))
}
