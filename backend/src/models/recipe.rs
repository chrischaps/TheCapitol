use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

/// Recipe input requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeInput {
    pub item_type: String,
    pub quantity: i32,
    #[serde(default = "default_quality_weight")]
    pub quality_weight: f32,
}

fn default_quality_weight() -> f32 {
    1.0
}

/// Recipe output definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeOutput {
    pub item_type: String,
    pub quantity_min: i32,
    pub quantity_max: i32,
}

/// Quality variance weights for crafting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityWeights {
    pub variance_min: i32,
    pub variance_max: i32,
}

impl Default for QualityWeights {
    fn default() -> Self {
        Self {
            variance_min: -3,
            variance_max: 3,
        }
    }
}

/// Recipe definition from database
#[derive(Debug, Clone, FromRow)]
pub struct RecipeRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: String,
    pub inputs: sqlx::types::Json<Vec<RecipeInput>>,
    pub outputs: sqlx::types::Json<Vec<RecipeOutput>>,
    pub base_duration_ticks: i32,
    pub quality_weights: sqlx::types::Json<QualityWeights>,
    pub crafting_requirements: sqlx::types::Json<serde_json::Value>,
    pub required_station_type: Option<String>,
}

/// Parsed recipe for game engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: String,
    pub inputs: Vec<RecipeInput>,
    pub outputs: Vec<RecipeOutput>,
    pub base_duration_ticks: i32,
    pub quality_weights: QualityWeights,
    pub required_station_type: Option<String>,
}

impl From<RecipeRow> for Recipe {
    fn from(row: RecipeRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            category: row.category,
            inputs: row.inputs.0,
            outputs: row.outputs.0,
            base_duration_ticks: row.base_duration_ticks,
            quality_weights: row.quality_weights.0,
            required_station_type: row.required_station_type,
        }
    }
}

/// Client-facing recipe info for UI
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RecipeInfo {
    /// Unique recipe identifier
    pub id: String,
    /// Display name of the recipe
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Recipe category (e.g., "hand_crafting")
    pub category: String,
    /// Required input items
    pub inputs: Vec<RecipeInputInfo>,
    /// Produced output items
    pub outputs: Vec<RecipeOutputInfo>,
    /// Base crafting duration in ticks
    pub duration_ticks: i32,
    /// Station type required (null for hand crafting)
    pub required_station_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RecipeInputInfo {
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Required quantity
    pub quantity: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RecipeOutputInfo {
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Minimum output quantity
    pub quantity_min: i32,
    /// Maximum output quantity
    pub quantity_max: i32,
}

impl Recipe {
    /// Convert to client-facing info with item names resolved
    pub fn to_info(&self, item_names: &std::collections::HashMap<String, String>) -> RecipeInfo {
        RecipeInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            category: self.category.clone(),
            inputs: self.inputs.iter().map(|i| RecipeInputInfo {
                item_type: i.item_type.clone(),
                item_name: item_names.get(&i.item_type).cloned().unwrap_or_else(|| i.item_type.clone()),
                quantity: i.quantity,
            }).collect(),
            outputs: self.outputs.iter().map(|o| RecipeOutputInfo {
                item_type: o.item_type.clone(),
                item_name: item_names.get(&o.item_type).cloned().unwrap_or_else(|| o.item_type.clone()),
                quantity_min: o.quantity_min,
                quantity_max: o.quantity_max,
            }).collect(),
            duration_ticks: self.base_duration_ticks,
            required_station_type: self.required_station_type.clone(),
        }
    }
}
