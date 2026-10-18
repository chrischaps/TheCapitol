use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use tower_http::compression::CompressionLayer;

use capitol_protocol::terrain::TerrainResponse;

use crate::models::TerrainConfig;
use crate::state::AppState;

/// The generator builds the shared terrain types directly, so the response is a copy.
/// `version` is left empty: the caller has it cached (`GameState::terrain_version`).
impl From<&TerrainConfig> for TerrainResponse {
    fn from(config: &TerrainConfig) -> Self {
        TerrainResponse {
            world_size: config.world_size,
            center_x: config.center_x,
            center_y: config.center_y,
            water_rings: config.water_rings.clone(),
            roads: config.roads.clone(),
            terrain_plots: config.terrain_plots.clone(),
            parks: config.parks.clone(),
            version: None,
        }
    }
}

/// Get terrain configuration (roads, water, bridges, plots)
///
/// The response carries `version`, a hash of the terrain, which is also its (weak) ETag.
/// Send it back in `If-None-Match` and an unchanged terrain answers 304 with no body.
/// Gzipped when the request accepts it.
#[utoipa::path(
    get,
    path = "/terrain",
    params(
        ("If-None-Match" = Option<String>, Header, description = "A version from an earlier response")
    ),
    responses(
        (status = 200, description = "Terrain data", body = TerrainResponse,
            headers(("ETag" = String, description = "W/\"<version>\""))),
        (status = 304, description = "The terrain still matches If-None-Match")
    ),
    tag = "Terrain"
)]
pub async fn get_terrain(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let game = state.game.read().await;
    let version = game.terrain_version.clone();
    let cache_headers = [
        (header::ETAG, format!("W/\"{version}\"")),
        // Stored, but always revalidated, so a regenerated terrain is seen at once
        (header::CACHE_CONTROL, "no-cache".to_string()),
    ];

    if matches_version(&headers, &version) {
        return (StatusCode::NOT_MODIFIED, cache_headers).into_response();
    }

    let mut response: TerrainResponse = (&game.terrain).into();
    drop(game);
    response.version = Some(version);
    (cache_headers, Json(response)).into_response()
}

/// Whether `If-None-Match` names this version. It's a comma-separated list of
/// ETags, quoted and maybe weak, or `*`. A bare version is accepted too.
fn matches_version(headers: &HeaderMap, version: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|tag| tag.trim())
        .any(|tag| {
            let tag = tag.strip_prefix("W/").unwrap_or(tag);
            tag == "*" || tag.trim_matches('"') == version
        })
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(get_terrain))
        .layer(CompressionLayer::new())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn if_none_match(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(header::IF_NONE_MATCH, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn test_if_none_match_forms() {
        let v = "0123456789abcdef";
        assert!(!matches_version(&HeaderMap::new(), v));
        assert!(matches_version(&if_none_match(&["W/\"0123456789abcdef\""]), v));
        assert!(matches_version(&if_none_match(&["\"0123456789abcdef\""]), v));
        assert!(matches_version(&if_none_match(&["0123456789abcdef"]), v));
        assert!(matches_version(&if_none_match(&["*"]), v));
        assert!(matches_version(&if_none_match(&["\"old\", W/\"0123456789abcdef\""]), v));
        assert!(matches_version(&if_none_match(&["\"old\"", "W/\"0123456789abcdef\""]), v));
        assert!(!matches_version(&if_none_match(&["W/\"fedcba9876543210\""]), v));
    }
}
