use axum::{extract::State, Json};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub database: String,
    pub redis: String,
}

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
