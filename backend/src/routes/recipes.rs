use axum::{
    extract::State,
    routing::get,
    Extension, Json, Router,
};
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::RecipeInfo;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(list_recipes))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecipesResponse {
    /// Available crafting recipes
    pub recipes: Vec<RecipeInfo>,
}

/// Get available crafting recipes
#[utoipa::path(
    get,
    path = "/recipes",
    responses(
        (status = 200, description = "List of recipes", body = RecipesResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Recipes"
)]
pub async fn list_recipes(
    State(state): State<AppState>,
    Extension(_auth_user): Extension<AuthUser>,
) -> Result<Json<RecipesResponse>, AppError> {
    let game = state.game.read().await;

    // Build item name map
    let item_names: HashMap<String, String> = game.item_types
        .iter()
        .map(|(id, t)| (id.clone(), t.name.clone()))
        .collect();

    // Convert recipes to client-facing info
    let recipes: Vec<RecipeInfo> = game.recipes
        .values()
        .filter(|r| r.category == "hand_crafting")
        .map(|r| r.to_info(&item_names))
        .collect();

    Ok(Json(RecipesResponse { recipes }))
}
