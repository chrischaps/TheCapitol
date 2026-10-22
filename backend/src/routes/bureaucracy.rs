use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use capitol_protocol::rest::{DeedPrice, DeedPricesResponse, ZoneFactsResponse, ZonesResponse};

use crate::error::AppError;
use crate::models::ZoneInfo;
use crate::state::AppState;

pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/zones", get(get_zones))
        .route("/deed-prices", get(get_deed_prices))
        .route("/zone-facts", get(get_zone_facts))
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
    let tunables = state.tunables.get();
    let zones = zones
        .into_iter()
        .map(|zone| ZoneInfo { deed_cost: zone.deed_cost.map(|base| tunables.deed_cost(&zone.id, base)), ..zone })
        .collect();

    Ok(Json(ZonesResponse { zones }))
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

    let tunables = state.tunables.get();
    let prices: Vec<DeedPrice> = rows
        .into_iter()
        .map(|(zone_id, zone_name, deed_cost, plot_size, station_capacity)| DeedPrice {
            deed_cost: tunables.deed_cost(&zone_id, deed_cost),
            zone_id,
            zone_name,
            plot_size,
            station_capacity,
        })
        .collect();

    Ok(Json(DeedPricesResponse { prices }))
}

/// What living in each home zone is like (#444)
///
/// One entry per zone a player can claim in: the live deed price and a
/// week's tax on it, the stations a typical lot fits (not its capacity),
/// lots free, the walk to the hall, what grows there, and the play style in
/// a line. The room and the walk are worked out once for each world, so the
/// first ask after a terrain commit can take a few seconds.
#[utoipa::path(
    get,
    path = "/bureaucracy/zone-facts",
    responses(
        (status = 200, description = "Each home zone's facts, from the city out", body = ZoneFactsResponse)
    ),
    tag = "Bureaucracy"
)]
pub async fn get_zone_facts(State(state): State<AppState>) -> Result<Json<ZoneFactsResponse>, AppError> {
    Ok(Json(ZoneFactsResponse { zones: crate::engine::zone_facts::facts(&state).await? }))
}
