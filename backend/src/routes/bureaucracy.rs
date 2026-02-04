use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::AppError;
use crate::models::ZoneInfo;
use crate::state::AppState;

pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/zones", get(get_zones))
        .route("/deed-prices", get(get_deed_prices))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ZonesResponse {
    /// All zones in the game world
    pub zones: Vec<ZoneInfo>,
}

/// Get all zone definitions
#[utoipa::path(
    get,
    path = "/bureaucracy/zones",
    responses(
        (status = 200, description = "Zone list", body = ZonesResponse)
    ),
    tag = "Bureaucracy"
)]
pub async fn get_zones(
    State(state): State<AppState>,
) -> Result<Json<ZonesResponse>, AppError> {
    let zones: Vec<ZoneInfo> = sqlx::query_as(
        "SELECT id, name, ring_order, inner_radius, outer_radius, has_plots,
                plot_size_category, deed_cost, description
         FROM zones ORDER BY ring_order"
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(ZonesResponse { zones }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeedPrice {
    /// Zone identifier
    pub zone_id: String,
    /// Zone display name
    pub zone_name: String,
    /// Cost in Strands to purchase a deed
    pub deed_cost: i32,
    /// Plot size category name
    pub plot_size: String,
    /// Maximum stations allowed on plots in this zone
    pub station_capacity: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeedPricesResponse {
    /// Deed prices for all zones with purchasable plots
    pub prices: Vec<DeedPrice>,
}

/// Get deed prices for all zones with purchasable plots
#[utoipa::path(
    get,
    path = "/bureaucracy/deed-prices",
    responses(
        (status = 200, description = "Deed prices", body = DeedPricesResponse)
    ),
    tag = "Bureaucracy"
)]
pub async fn get_deed_prices(
    State(state): State<AppState>,
) -> Result<Json<DeedPricesResponse>, AppError> {
    let rows: Vec<(String, String, i32, String, i32)> = sqlx::query_as(
        "SELECT z.id, z.name, z.deed_cost, psc.name, psc.station_capacity
         FROM zones z
         JOIN plot_size_categories psc ON z.plot_size_category = psc.id
         WHERE z.has_plots = true AND z.deed_cost IS NOT NULL
         ORDER BY z.ring_order"
    )
    .fetch_all(&state.db)
    .await?;

    let prices: Vec<DeedPrice> = rows
        .into_iter()
        .map(|(zone_id, zone_name, deed_cost, plot_size, station_capacity)| DeedPrice {
            zone_id,
            zone_name,
            deed_cost,
            plot_size,
            station_capacity,
        })
        .collect();

    Ok(Json(DeedPricesResponse { prices }))
}
