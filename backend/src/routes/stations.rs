use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{quality_to_grade, Container, ContainerType, Item, ItemType, SlotItem, Station, StationInfo, StationType};
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

/// Station info for nearby query
#[derive(Debug, Serialize)]
pub struct NearbyStationsResponse {
    pub stations: Vec<StationInfo>,
}

/// Get all stations near the player
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

    let (player_id, player_x, player_y) = player;

    // Get station types for metadata
    let station_types: Vec<StationType> = sqlx::query_as(
        "SELECT id, name, category, slot_count, layout_columns, interaction_range, icon, description
         FROM station_types"
    )
    .fetch_all(&state.db)
    .await?;

    let type_map: HashMap<String, StationType> = station_types
        .into_iter()
        .map(|st| (st.id.clone(), st))
        .collect();

    // Get all stations within a generous range (500 units for visibility)
    let stations: Vec<Station> = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y, placed_at
         FROM stations
         WHERE position_x BETWEEN $1 - 500 AND $1 + 500
           AND position_y BETWEEN $2 - 500 AND $2 + 500"
    )
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
            })
        })
        .collect();

    Ok(Json(NearbyStationsResponse { stations: station_infos }))
}

#[derive(Debug, Deserialize)]
pub struct PlaceStationRequest {
    pub station_type: String,
    pub x: f64,
    pub y: f64,
    pub kit_item_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct PlaceStationResponse {
    pub station: StationInfo,
}

/// Place a new station in the world
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
        "SELECT id, name, category, slot_count, layout_columns, interaction_range, icon, description
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

    // Verify player owns the kit item and it's the right type
    let expected_kit = match req.station_type.as_str() {
        "workbench" => "workbench_kit",
        "forge" => "forge_kit",
        "storage_chest" => "chest_kit",
        _ => return Err(AppError::BadRequest("Invalid station type".into())),
    };

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
    .bind(50.0 * 50.0)
    .fetch_optional(&state.db)
    .await?;

    if nearby.is_some() {
        return Err(AppError::BadRequest("Too close to another station".into()));
    }

    // Create station
    let station_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stations (id, station_type, owner_id, position_x, position_y)
         VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(station_id)
    .bind(&req.station_type)
    .bind(player_id)
    .bind(req.x as f32)
    .bind(req.y as f32)
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
    };

    Ok(Json(PlaceStationResponse { station: info }))
}

/// Remove a station (owner only)
pub async fn remove_station(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(station_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Verify ownership
    let station: Station = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y, placed_at
         FROM stations WHERE id = $1"
    )
    .bind(station_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Station not found".into()))?;

    if station.owner_id != player_id {
        return Err(AppError::Forbidden("Not your station".into()));
    }

    // Delete station (cascade will delete container and items)
    sqlx::query("DELETE FROM stations WHERE id = $1")
        .bind(station_id)
        .execute(&state.db)
        .await?;

    Ok(Json(serde_json::json!({ "success": true })))
}

/// Container response with slot items
#[derive(Debug, Serialize)]
pub struct StationContainerResponse {
    pub id: Uuid,
    pub container_type: String,
    pub slot_count: i32,
    pub layout_columns: i32,
    pub slots: Vec<SlotItem>,
}

/// Get the inventory container of a station
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

    // Verify station exists and player can access (owner only for now)
    let station: Station = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y, placed_at
         FROM stations WHERE id = $1"
    )
    .bind(station_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Station not found".into()))?;

    if station.owner_id != player_id {
        return Err(AppError::Forbidden("Not your station".into()));
    }

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
