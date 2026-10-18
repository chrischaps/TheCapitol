use capitol_protocol::rules::quality::Weights;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub use capitol_protocol::rest::{RecipeInfo, RecipeInputInfo, RecipeOutputInfo};

/// Recipe input requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeInput {
    pub item_type: String,
    pub quantity: i32,
    #[serde(default = "default_quality_weight")]
    pub quality_weight: f64,
}

fn default_quality_weight() -> f64 {
    1.0
}

/// Recipe output definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeOutput {
    pub item_type: String,
    pub quantity_min: i32,
    pub quantity_max: i32,
}

/// `recipes.quality_weights`: the variance, and the station's and tool's
/// shares when the recipe sets them. The inputs' weights are on the inputs.
/// [`Weights::for_recipe`] fills the gaps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityWeights {
    pub variance_min: i32,
    pub variance_max: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<f64>,
}

impl Default for QualityWeights {
    fn default() -> Self {
        Self {
            variance_min: -3,
            variance_max: 3,
            station: None,
            tool: None,
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
#[allow(dead_code)] // tool gating in crafting (#157, E1.5) reads it
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
    /// The cascade's weights, defaults filled and normalised.
    pub quality_weights: Weights,
    pub required_station_type: Option<String>,
}

impl From<RecipeRow> for Recipe {
    fn from(row: RecipeRow) -> Self {
        let stored = row.quality_weights.0;
        let quality_weights = Weights::for_recipe(
            row.inputs.0.iter().map(|i| (i.item_type.as_str(), i.quality_weight)),
            stored.station,
            stored.tool,
            row.required_station_type.is_some(),
            stored.variance_min,
            stored.variance_max,
        );
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            category: row.category,
            inputs: row.inputs.0,
            outputs: row.outputs.0,
            base_duration_ticks: row.base_duration_ticks,
            quality_weights,
            required_station_type: row.required_station_type,
        }
    }
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
            quality_weights: self.quality_weights.clone(),
        }
    }
}
