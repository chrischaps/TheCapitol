use axum::{extract::State, routing::get, Json, Router};

use capitol_protocol::terrain::TerrainResponse;

use crate::models::TerrainConfig;
use crate::state::AppState;

/// The generator builds the shared terrain types directly, so the response is a copy.
impl From<&TerrainConfig> for TerrainResponse {
    fn from(config: &TerrainConfig) -> Self {
        TerrainResponse {
            world_size: config.world_size,
            center_x: config.center_x,
            center_y: config.center_y,
            water_rings: config.water_rings.clone(),
            roads: config.roads.clone(),
            terrain_plots: config.terrain_plots.clone(),
        }
    }
}

/// Get terrain configuration (roads, water, bridges, plots)
#[utoipa::path(
    get,
    path = "/terrain",
    responses(
        (status = 200, description = "Terrain data", body = TerrainResponse)
    ),
    tag = "Terrain"
)]
pub async fn get_terrain(State(state): State<AppState>) -> Json<TerrainResponse> {
    let game = state.game.read().await;
    let response: TerrainResponse = (&game.terrain).into();
    Json(response)
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(get_terrain))
        .with_state(state)
}
