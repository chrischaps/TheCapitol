use axum::{extract::State, Json};
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    /// Overall health status: "healthy" or "degraded"
    pub status: String,
    /// Database connection status
    pub database: String,
    /// Redis connection status
    pub redis: String,
}

/// Check server health
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Health check response", body = HealthResponse)
    ),
    tag = "Health"
)]
pub async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    let db_status = match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => "healthy".to_string(),
        Err(e) => format!("unhealthy: {}", e),
    };

    let redis_status = {
        let mut conn = state.redis.clone();
        match redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
        {
            Ok(_) => "healthy".to_string(),
            Err(e) => format!("unhealthy: {}", e),
        }
    };

    let overall = if db_status == "healthy" && redis_status == "healthy" {
        "healthy"
    } else {
        "degraded"
    };

    Json(HealthResponse {
        status: overall.to_string(),
        database: db_status,
        redis: redis_status,
    })
}
