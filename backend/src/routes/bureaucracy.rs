use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use capitol_protocol::rest::{
    DeedPrice, DeedPricesResponse, FreeLotsQuery, FreeLotsResponse, ZoneFactsResponse, ZonesResponse,
};
use capitol_protocol::rules::Zone;

use crate::error::AppError;
use crate::models::ZoneInfo;
use crate::state::AppState;

pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/zones", get(get_zones))
        .route("/deed-prices", get(get_deed_prices))
        .route("/zone-facts", get(get_zone_facts))
        .route("/free-lots", get(get_free_lots))
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

/// The lots free in one home zone or the guild district (the Deeds Office,
/// #448; a guild's hall, #581)
///
/// Every unclaimed lot in `zone`, nearest the Exchange hall first, with the
/// stations it fits (capped by its capacity, as the zone card counts them)
/// and its straight walk to the hall's door, and the zone's deed at its live
/// price. Which lots are free is read live; the room and the walk are worked
/// out once for each world.
#[utoipa::path(
    get,
    path = "/bureaucracy/free-lots",
    params(FreeLotsQuery),
    responses(
        (status = 200, description = "The zone's free lots, nearest the hall first", body = FreeLotsResponse),
        (status = 400, description = "Not a home zone or the guild district")
    ),
    tag = "Bureaucracy"
)]
pub async fn get_free_lots(
    State(state): State<AppState>,
    Query(query): Query<FreeLotsQuery>,
) -> Result<Json<FreeLotsResponse>, AppError> {
    let zone = Zone::from_str(&query.zone).filter(|z| crate::engine::zone_facts::lists(*z)).ok_or_else(|| {
        AppError::BadRequest("Lots are listed for the urban, suburban and rural zones and the guild district".into())
    })?;
    let lots = crate::engine::zone_facts::free_lots(&state, zone).await?;
    let base: Option<i32> = sqlx::query_scalar("SELECT deed_cost FROM zones WHERE id = $1")
        .bind(zone.as_str())
        .fetch_optional(&state.db)
        .await?
        .flatten();
    let deed_cost = base.map(|base| state.tunables.get().deed_cost(zone.as_str(), base));
    Ok(Json(FreeLotsResponse { zone_id: zone.as_str().to_owned(), lots, deed_cost }))
}
