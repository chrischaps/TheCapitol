use axum::{extract::State, Json};
use capitol_protocol::rest::CatalogResponse;

use crate::state::AppState;

/// The static game data: item, station, resource and container types, and
/// every recipe, as the server loaded them at startup (E0.2). `version`
/// changes exactly when the content does; `/health` reports it too.
#[utoipa::path(
    get,
    path = "/catalog",
    responses(
        (status = 200, description = "The static game data", body = CatalogResponse)
    ),
    tag = "Catalog"
)]
pub async fn get_catalog(State(state): State<AppState>) -> Json<CatalogResponse> {
    let catalog = state.game.read().await.catalog.clone();
    Json(CatalogResponse::clone(&catalog))
}
