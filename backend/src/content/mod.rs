//! The static game data, as data (E0.3).
//!
//! Items, resources, containers, stations and recipes live in
//! `backend/content/*.toml`, built into the binary. At startup [`sync`] writes
//! them into their tables in one transaction, before `main` loads the game
//! data from those tables. The migrations keep only the schema; the seed rows
//! they once inserted are now upserted from here.
//!
//! A row in a static table that the files no longer define stops startup,
//! unless `retired.toml` lists it. Items in players' hands name their type,
//! so a type can't just disappear. [`validate`] checks that the content holds
//! together: references resolve, everything can be had, nothing is its own
//! ingredient. Startup runs it before [`sync`] ([`load_and_sync`]), and so
//! does `cargo test`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use capitol_protocol::rules::resources::{self, Cluster, ResourceYield, Spawn};
use capitol_protocol::rules::{tools, Zone};
pub use crate::models::CraftingRequirements;
use serde::Deserialize;
use sqlx::{PgPool, Postgres, Transaction};

const ITEMS: &str = include_str!("../../content/items.toml");
const RESOURCES: &str = include_str!("../../content/resources.toml");
const CONTAINERS: &str = include_str!("../../content/containers.toml");
const STATIONS: &str = include_str!("../../content/stations.toml");
const RECIPES: &str = include_str!("../../content/recipes.toml");
const RETIRED: &str = include_str!("../../content/retired.toml");
const WILDS: &str = include_str!("../../content/wilds.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDef {
    pub id: String,
    pub name: String,
    pub category: String,
    pub stackable: bool,
    pub weight: f32,
    /// Strands a unit the economy is balanced to (D2.3, D5's value chain)
    pub target_price: Option<i64>,
    /// Set for a tool (E1.4)
    pub tool: Option<ToolDef>,
}

/// What makes an item a tool: `rules::tools` reads these.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDef {
    pub tool_type: String,
    pub tier: String,
    pub base_power: f64,
    pub max_durability: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceDef {
    pub id: String,
    pub name: String,
    pub yields_item: String,
    pub base_quality_min: i32,
    pub base_quality_max: i32,
    /// A tool type it can't be gathered without (E1.5). It's then `tool_type` too.
    pub required_tool: Option<String>,
    /// The tool type that works it: the best one held speeds the gather, reads
    /// its tier's yield row and wears (`rules::tools`). Defaults to `required_tool`.
    pub tool_type: Option<String>,
    /// How hard it is to work, on the tools' power scale (hands are 4).
    pub difficulty: f64,
    pub extraction_duration_ticks: i32,
    pub regeneration_ticks: i32,
    /// Units a gather gives, by `hand` or tool tier (E2.2)
    pub yields: BTreeMap<String, YieldDef>,
    /// How its nodes group (`rules::resources::Cluster`)
    pub cluster: String,
    /// The zones its nodes grow in
    pub spawns: Vec<SpawnDef>,
}

impl ResourceDef {
    /// The tool type that works it, given or implied by `required_tool`.
    pub fn tool(&self) -> Option<&str> {
        self.tool_type.as_deref().or(self.required_tool.as_deref())
    }

    /// Its yield table as the rules read it.
    pub fn yield_table(&self) -> resources::YieldTable {
        self.yields
            .iter()
            .map(|(k, y)| (k.clone(), ResourceYield { min: y.min, max: y.max, ticks: y.ticks }))
            .collect()
    }

    /// The row `resource_types.yield_quantity_min`/`_max` keep: by hand, or
    /// the least tool that works it.
    pub fn first_yield(&self) -> Option<ResourceYield> {
        resources::yield_for(&self.yield_table(), None).cloned()
    }

    pub fn spawn_table(&self) -> Vec<Spawn> {
        self.spawns
            .iter()
            .map(|s| Spawn { zone: s.zone.clone(), density: s.density, quality_floor: s.quality_floor })
            .collect()
    }
}

/// `id` from the built-in content as the engine holds it, for tests.
#[cfg(test)]
pub fn resource_type(id: &str) -> crate::models::ResourceType {
    load().unwrap().resources.iter().find(|r| r.id == id).expect("a resource in the content").to_type()
}

#[cfg(test)]
impl ResourceDef {
    /// This resource as `main` loads it from `resource_types` after a sync.
    pub fn to_type(&self) -> crate::models::ResourceType {
        let first = self.first_yield().unwrap_or(ResourceYield { min: 1, max: 1, ticks: None });
        crate::models::ResourceType {
            id: self.id.clone(),
            name: self.name.clone(),
            yields_item: self.yields_item.clone(),
            yield_quantity_min: first.min,
            yield_quantity_max: first.max,
            base_quality_min: self.base_quality_min,
            base_quality_max: self.base_quality_max,
            required_tool: self.required_tool.clone(),
            extraction_duration_ticks: self.extraction_duration_ticks,
            regeneration_ticks: self.regeneration_ticks,
            tool_type: self.tool().map(str::to_string),
            difficulty: self.difficulty,
            yields: sqlx::types::Json(self.yield_table()),
            cluster: self.cluster.clone(),
            spawns: sqlx::types::Json(self.spawn_table()),
        }
    }
}

/// A row of a resource's yield table (`rules::resources::ResourceYield`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YieldDef {
    pub min: i32,
    pub max: i32,
    pub ticks: Option<i32>,
}

/// A row of a resource's spawn table (`rules::resources::Spawn`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnDef {
    pub zone: String,
    pub density: f64,
    pub quality_floor: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerDef {
    pub id: String,
    pub name: String,
    pub slot_count: i32,
    pub layout_columns: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StationDef {
    pub id: String,
    pub name: String,
    pub category: String,
    pub kit: String,
    pub slot_count: i32,
    pub layout_columns: i32,
    pub interaction_range: f32,
    pub icon: Option<String>,
    pub description: Option<String>,
    /// May it stand off a plot, on dry ground no lot covers (#187)?
    #[serde(default)]
    pub off_plot: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputDef {
    pub item_type: String,
    pub quantity: i32,
    pub quality_weight: f64,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutputDef {
    pub item_type: String,
    pub quantity_min: i32,
    pub quantity_max: i32,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct VarianceDef {
    pub variance_min: i32,
    pub variance_max: i32,
    /// The station's share of the output's quality; 0.2 if the recipe needs
    /// a station and this is left out (`rules::quality::Weights::for_recipe`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<f64>,
    /// The tool's share; 0 if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeDef {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: String,
    pub required_station_type: Option<String>,
    pub base_duration_ticks: i32,
    pub inputs: Vec<InputDef>,
    pub outputs: Vec<OutputDef>,
    pub quality_weights: VarianceDef,
    #[serde(default)]
    pub crafting_requirements: CraftingRequirements,
}

impl ItemDef {
    /// As the engine holds it once it's loaded from `item_types`.
    #[cfg(test)]
    pub fn to_item_type(&self) -> crate::models::ItemType {
        crate::models::ItemType {
            id: self.id.clone(),
            name: self.name.clone(),
            category: self.category.clone(),
            stackable: self.stackable,
            weight: self.weight,
            tool_type: self.tool.as_ref().map(|t| t.tool_type.clone()),
            tier: self.tool.as_ref().map(|t| t.tier.clone()),
            base_power: self.tool.as_ref().map(|t| t.base_power),
            max_durability: self.tool.as_ref().map(|t| t.max_durability),
            target_price: self.target_price,
        }
    }
}

impl RecipeDef {
    /// As the engine holds it once it's loaded from `recipes`, weights filled in.
    #[cfg(test)]
    pub fn to_recipe(&self) -> crate::models::Recipe {
        use crate::models::{QualityWeights, RecipeInput, RecipeOutput, RecipeRow};
        let q = &self.quality_weights;
        crate::models::Recipe::from(RecipeRow {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            category: self.category.clone(),
            inputs: sqlx::types::Json(
                self.inputs
                    .iter()
                    .map(|i| RecipeInput {
                        item_type: i.item_type.clone(),
                        quantity: i.quantity,
                        quality_weight: i.quality_weight,
                    })
                    .collect(),
            ),
            outputs: sqlx::types::Json(
                self.outputs
                    .iter()
                    .map(|o| RecipeOutput {
                        item_type: o.item_type.clone(),
                        quantity_min: o.quantity_min,
                        quantity_max: o.quantity_max,
                    })
                    .collect(),
            ),
            base_duration_ticks: self.base_duration_ticks,
            quality_weights: sqlx::types::Json(QualityWeights {
                variance_min: q.variance_min,
                variance_max: q.variance_max,
                station: q.station,
                tool: q.tool,
            }),
            crafting_requirements: sqlx::types::Json(self.crafting_requirements.clone()),
            required_station_type: self.required_station_type.clone(),
        })
    }
}

/// Where the wilds' nodes grow, and how good they are there (W6, #199):
/// `wilds.toml`. Only the node planner reads it (`engine::node_spawn`); it
/// isn't synced to a table.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WildsDef {
    pub quality: WildQualityDef,
    #[serde(default)]
    pub habitat: Vec<HabitatDef>,
}

/// How a wild node's base-quality ceiling rises: points added to its
/// resource's `base_quality_max`, interpolated between `[at, add]` rows by
/// how far past the rural edge (`depth`, PV2 #454) and by the land's height
/// (`height`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WildQualityDef {
    pub depth: Vec<[f64; 2]>,
    pub height: Vec<[f64; 2]>,
}

impl WildQualityDef {
    /// The ceiling for a resource whose own is `max`, `r` past the rural
    /// edge and at height `h`: never over 100, nor under `floor`.
    pub fn ceiling(&self, max: i32, floor: i32, r: f64, h: f64) -> i32 {
        let add = interpolate(&self.depth, r) + interpolate(&self.height, h);
        ((max as f64 + add).round() as i32).min(100).max(floor)
    }
}

/// Piecewise-linear through `rows` (sorted by their first value), held flat
/// past either end.
fn interpolate(rows: &[[f64; 2]], x: f64) -> f64 {
    let Some(first) = rows.first() else { return 0.0 };
    if x <= first[0] {
        return first[1];
    }
    for w in rows.windows(2) {
        let ([x0, y0], [x1, y1]) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    rows[rows.len() - 1][1]
}

/// Where in the wilds one resource grows. Every bound is optional, and each
/// is `[lo, hi]`, inclusive.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HabitatDef {
    pub resource: String,
    /// The land's height (`land::height_at`)
    pub height: Option<[f64; 2]>,
    /// Rise over run (`land::slope_at`)
    pub slope: Option<[f64; 2]>,
    /// How far past the rural edge
    pub depth: Option<[f64; 2]>,
    /// For woods: the share of the habitat that's woods
    pub cover: Option<f64>,
    /// For woods: the resource whose woods' noise field these grow in, so
    /// they run on from its woods (old growth from the rural trees, #225).
    /// Left out, its own.
    pub woods_of: Option<String>,
    /// Only on the even or the odd massifs (`land::massif_at`), so copper
    /// and tin keep to different ranges (#225).
    pub massifs: Option<Massifs>,
}

/// Which of the ranges' massifs a habitat keeps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Massifs {
    Even,
    Odd,
}

impl HabitatDef {
    /// Whether the land `r` past the rural edge, at height `h` and slope
    /// `s`, suits it.
    pub fn suits(&self, r: f64, h: f64, s: f64) -> bool {
        let within = |b: Option<[f64; 2]>, v: f64| b.is_none_or(|[lo, hi]| (lo..=hi).contains(&v));
        within(self.depth, r) && within(self.height, h) && within(self.slope, s)
    }

    /// Whether massif `massif` (`land::massif_at`) is one it keeps to. With
    /// no massifs on the land, any is.
    pub fn on_massif(&self, massif: Option<u32>) -> bool {
        match (self.massifs, massif) {
            (Some(Massifs::Even), Some(m)) => m % 2 == 0,
            (Some(Massifs::Odd), Some(m)) => m % 2 == 1,
            _ => true,
        }
    }
}

/// The wilds' content, parsed once: `validate` has checked it at startup.
pub fn wilds() -> &'static WildsDef {
    static WILDS_DEF: std::sync::OnceLock<WildsDef> = std::sync::OnceLock::new();
    WILDS_DEF.get_or_init(|| parse("wilds.toml", WILDS).expect("content/wilds.toml parses"))
}

/// The ids `retired.toml` keeps in the tables, by table.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retired {
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub containers: Vec<String>,
    #[serde(default)]
    pub stations: Vec<String>,
    #[serde(default)]
    pub recipes: Vec<String>,
}

/// Everything in `backend/content/`.
#[derive(Debug, Clone)]
pub struct Content {
    pub items: Vec<ItemDef>,
    pub resources: Vec<ResourceDef>,
    pub containers: Vec<ContainerDef>,
    pub stations: Vec<StationDef>,
    pub recipes: Vec<RecipeDef>,
    pub retired: Retired,
    pub wilds: WildsDef,
}

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("content/{file} doesn't parse: {source}")]
    Parse {
        file: &'static str,
        source: toml::de::Error,
    },
    #[error("the content doesn't hold together:\n  {}", .0.join("\n  "))]
    Invalid(Vec<String>),
    #[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
    Removed(Vec<Removed>),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// A row in a static table that the content no longer defines or retires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub table: &'static str,
    pub id: String,
}

impl fmt::Display for Removed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (file, key) = table_files(self.table);
        write!(
            f,
            "{} has a row '{}' that content/{file} no longer defines. Put it back, \
             or list it under `{key}` in content/retired.toml so the game keeps it",
            self.table, self.id
        )
    }
}

/// The content file and `retired.toml` key for a static table.
fn table_files(table: &str) -> (&'static str, &'static str) {
    match table {
        "item_types" => ("items.toml", "items"),
        "resource_types" => ("resources.toml", "resources"),
        "container_types" => ("containers.toml", "containers"),
        "station_types" => ("stations.toml", "stations"),
        _ => ("recipes.toml", "recipes"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemsFile {
    #[serde(default)]
    item: Vec<ItemDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourcesFile {
    #[serde(default)]
    resource: Vec<ResourceDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainersFile {
    #[serde(default)]
    container: Vec<ContainerDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StationsFile {
    #[serde(default)]
    station: Vec<StationDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipesFile {
    #[serde(default)]
    recipe: Vec<RecipeDef>,
}

fn parse<T: serde::de::DeserializeOwned>(file: &'static str, text: &str) -> Result<T, ContentError> {
    toml::from_str(text).map_err(|source| ContentError::Parse { file, source })
}

/// The content built into this binary.
pub fn load() -> Result<Content, ContentError> {
    Ok(Content {
        items: parse::<ItemsFile>("items.toml", ITEMS)?.item,
        resources: parse::<ResourcesFile>("resources.toml", RESOURCES)?.resource,
        containers: parse::<ContainersFile>("containers.toml", CONTAINERS)?.container,
        stations: parse::<StationsFile>("stations.toml", STATIONS)?.station,
        recipes: parse::<RecipesFile>("recipes.toml", RECIPES)?.recipe,
        retired: parse("retired.toml", RETIRED)?,
        wilds: parse("wilds.toml", WILDS)?,
    })
}

/// Everything wrong with `content`, one line each. Empty means it holds together.
pub fn validate(content: &Content) -> Vec<String> {
    let mut problems = Vec::new();

    let mut ids = |kind: &str, file_ids: Vec<&str>, retired: &[String]| -> HashSet<String> {
        let mut seen = HashSet::new();
        for id in file_ids {
            if !seen.insert(id.to_string()) {
                problems.push(format!("{kind} '{id}' is defined twice"));
            }
            if retired.iter().any(|r| r == id) {
                problems.push(format!("{kind} '{id}' is both defined and retired"));
            }
        }
        seen
    };
    let items = ids("item", content.items.iter().map(|i| i.id.as_str()).collect(), &content.retired.items);
    let _ = ids(
        "resource",
        content.resources.iter().map(|r| r.id.as_str()).collect(),
        &content.retired.resources,
    );
    let _ = ids(
        "container",
        content.containers.iter().map(|c| c.id.as_str()).collect(),
        &content.retired.containers,
    );
    let stations = ids(
        "station",
        content.stations.iter().map(|s| s.id.as_str()).collect(),
        &content.retired.stations,
    );
    let _ = ids("recipe", content.recipes.iter().map(|r| r.id.as_str()).collect(), &content.retired.recipes);

    let item = |problems: &mut Vec<String>, whose: String, id: &str| {
        if !items.contains(id) {
            problems.push(format!("{whose} names item '{id}', which items.toml doesn't define"));
        }
    };
    for r in &content.resources {
        item(&mut problems, format!("resource '{}'", r.id), &r.yields_item);
        for tool in [&r.required_tool, &r.tool_type].into_iter().flatten() {
            if tools::ToolType::from_str(tool).is_none() {
                problems.push(format!("resource '{}' names unknown tool type '{tool}'", r.id));
            }
        }
        if let (Some(required), Some(works)) = (&r.required_tool, &r.tool_type) {
            if required != works {
                problems.push(format!("resource '{}' requires a {required} but is worked by a {works}", r.id));
            }
        }
        if !(r.difficulty.is_finite() && r.difficulty >= 0.0) {
            problems.push(format!("resource '{}' has a difficulty that is negative or not a number", r.id));
        }
        validate_yields(r, &mut problems);
        validate_spawns(r, &mut problems);
        validate_habitat(r, &content.wilds, &mut problems);
    }
    validate_wilds(&content.wilds, &content.resources, &mut problems);
    for s in &content.stations {
        item(&mut problems, format!("station '{}' (its kit)", s.id), &s.kit);
    }
    for i in &content.items {
        if i.target_price.is_some_and(|p| p < 1) {
            problems.push(format!("item '{}' has a target price under 1 Strand", i.id));
        }
    }
    for i in &content.items {
        let Some(tool) = &i.tool else { continue };
        if tools::ToolType::from_str(&tool.tool_type).is_none() {
            problems.push(format!("item '{}' is a tool of unknown type '{}'", i.id, tool.tool_type));
        }
        if tools::Tier::from_str(&tool.tier).is_none() {
            problems.push(format!("item '{}' is a tool of unknown tier '{}'", i.id, tool.tier));
        }
        if !(tool.base_power.is_finite() && tool.base_power > 0.0) || tool.max_durability <= 0 {
            problems.push(format!("item '{}' is a tool with no power or no durability", i.id));
        }
        if i.stackable {
            problems.push(format!("item '{}' is a tool, and tools don't stack", i.id));
        }
    }
    let is_tool: HashSet<&str> =
        content.items.iter().filter(|i| i.tool.is_some()).map(|i| i.id.as_str()).collect();
    for r in &content.recipes {
        if let Some(tool) = &r.crafting_requirements.tool_type {
            if tools::ToolType::from_str(tool).is_none() {
                problems.push(format!("recipe '{}' needs unknown tool type '{tool}'", r.id));
            }
        }
        // A tool that wears for nothing, or a share nothing fills
        let tool_share = r.quality_weights.tool.unwrap_or(0.0) > 0.0;
        if tool_share != r.crafting_requirements.tool_type.is_some() {
            problems.push(format!(
                "recipe '{}' must name a tool type exactly when it gives the tool a quality share",
                r.id
            ));
        }
        for i in &r.inputs {
            item(&mut problems, format!("recipe '{}'", r.id), &i.item_type);
        }
        for o in &r.outputs {
            item(&mut problems, format!("recipe '{}'", r.id), &o.item_type);
            if is_tool.contains(o.item_type.as_str()) && (o.quantity_min != 1 || o.quantity_max != 1) {
                problems.push(format!("recipe '{}' makes tool '{}' other than one at a time", r.id, o.item_type));
            }
        }
        if r.outputs.is_empty() {
            problems.push(format!("recipe '{}' makes nothing", r.id));
        }
        if r.inputs.is_empty() {
            problems.push(format!("recipe '{}' takes nothing", r.id));
        }
        let q = &r.quality_weights;
        if q.variance_min > q.variance_max {
            problems.push(format!("recipe '{}' has variance_min above variance_max", r.id));
        }
        let shares = r.inputs.iter().map(|i| i.quality_weight).chain(q.station).chain(q.tool);
        if shares.into_iter().any(|w| !w.is_finite() || w < 0.0) {
            problems.push(format!("recipe '{}' has a quality weight that is negative or not a number", r.id));
        }
        if q.station.unwrap_or(0.0) + q.tool.unwrap_or(0.0) > 1.0 {
            problems.push(format!("recipe '{}' gives its station and tool more than the whole", r.id));
        }
        if let Some(station) = &r.required_station_type {
            if !stations.contains(station) {
                problems.push(format!(
                    "recipe '{}' needs station '{station}', which stations.toml doesn't define",
                    r.id
                ));
            }
        }
    }

    // What a player can come to hold, starting empty-handed: what grows
    // (given its tool), then what recipes make from that (given their station
    // and tool).
    let kit_of: HashMap<&str, &str> =
        content.stations.iter().map(|s| (s.id.as_str(), s.kit.as_str())).collect();
    let tool_of: HashMap<&str, &str> = content
        .items
        .iter()
        .filter_map(|i| Some((i.id.as_str(), i.tool.as_ref()?.tool_type.as_str())))
        .collect();
    let held = |had: &HashSet<&str>, t: &str| had.iter().any(|i| tool_of.get(i) == Some(&t));
    let mut had: HashSet<&str> = HashSet::new();
    loop {
        let before = had.len();
        for r in &content.resources {
            if r.required_tool.as_deref().is_none_or(|t| held(&had, t)) {
                had.insert(&r.yields_item);
            }
        }
        for r in &content.recipes {
            let station = r
                .required_station_type
                .as_deref()
                .is_none_or(|s| kit_of.get(s).is_some_and(|kit| had.contains(kit)));
            let tool = r.crafting_requirements.tool_type.as_deref().is_none_or(|t| held(&had, t));
            if station && tool && r.inputs.iter().all(|i| had.contains(i.item_type.as_str())) {
                had.extend(r.outputs.iter().map(|o| o.item_type.as_str()));
            }
        }
        if had.len() == before {
            break;
        }
    }
    for i in &content.items {
        if !had.contains(i.id.as_str()) {
            problems.push(format!(
                "item '{}' can't be had: no resource yields it and no recipe a player can reach makes it",
                i.id
            ));
        }
    }

    if let Some(cycle) = recipe_cycle(&content.recipes) {
        problems.push(format!("recipes go round in a circle: {}", cycle.join(" -> ")));
    }

    for s in &content.stations {
        if !content.recipes.iter().any(|r| r.outputs.iter().any(|o| o.item_type == s.kit)) {
            problems.push(format!("station '{}': no recipe makes its kit '{}'", s.id, s.kit));
        }
        let needed = content.recipes.iter().any(|r| r.required_station_type.as_deref() == Some(&s.id));
        if s.category != "storage" && !needed {
            problems.push(format!("station '{}' has nothing to make: no recipe needs it", s.id));
        }
    }

    problems
}

/// A resource's yield table: known rows, sane ranges, and a hand row exactly
/// when it can be gathered by hand.
fn validate_yields(r: &ResourceDef, problems: &mut Vec<String>) {
    if r.yields.is_empty() {
        problems.push(format!("resource '{}' has no yields", r.id));
    }
    for (row, y) in &r.yields {
        let tier = tools::Tier::from_str(row);
        if row != resources::HAND && tier.is_none() {
            problems.push(format!("resource '{}' has a yield row '{row}', which is neither hand nor a tier", r.id));
        }
        if tier.is_some() && r.tool().is_none() {
            problems.push(format!("resource '{}' has a {row} yield row but no tool type to use it", r.id));
        }
        if y.min < 1 || y.max < y.min {
            problems.push(format!("resource '{}' yields {}..{} at {row}: not at least 1, low to high", r.id, y.min, y.max));
        }
        if y.ticks.is_some_and(|t| t < 1) {
            problems.push(format!("resource '{}' takes no time at {row}", r.id));
        }
    }
    let by_hand = r.yields.contains_key(resources::HAND);
    match (&r.required_tool, by_hand) {
        (Some(tool), true) => problems.push(format!("resource '{}' needs a {tool} but has a hand yield row", r.id)),
        (None, false) => problems.push(format!("resource '{}' needs no tool but has no hand yield row", r.id)),
        _ => {}
    }
}

/// A resource's spawn table and cluster: known zones, once each, at a density,
/// with a floor inside the quality range.
fn validate_spawns(r: &ResourceDef, problems: &mut Vec<String>) {
    if Cluster::from_str(&r.cluster).is_none() {
        problems.push(format!("resource '{}' has unknown cluster '{}'", r.id, r.cluster));
    }
    if r.base_quality_min > r.base_quality_max {
        problems.push(format!("resource '{}' has base_quality_min above base_quality_max", r.id));
    }
    let mut zones = HashSet::new();
    for s in &r.spawns {
        if Zone::from_str(&s.zone).is_none() {
            problems.push(format!("resource '{}' spawns in unknown zone '{}'", r.id, s.zone));
        }
        if !zones.insert(s.zone.as_str()) {
            problems.push(format!("resource '{}' lists zone '{}' twice", r.id, s.zone));
        }
        if !(s.density.is_finite() && s.density > 0.0) {
            problems.push(format!("resource '{}' has a density in '{}' that isn't above 0", r.id, s.zone));
        }
        if let Some(floor) = s.quality_floor {
            if !(r.base_quality_min..=r.base_quality_max).contains(&floor) {
                problems.push(format!(
                    "resource '{}' has a quality floor in '{}' outside {}..{}",
                    r.id, s.zone, r.base_quality_min, r.base_quality_max
                ));
            }
        }
    }
}

/// A resource that grows in the wilds has one habitat there, with sound bounds.
fn validate_habitat(r: &ResourceDef, wilds: &WildsDef, problems: &mut Vec<String>) {
    let wild = r.spawns.iter().any(|s| s.zone == Zone::Wilderness.as_str());
    let habitats: Vec<&HabitatDef> = wilds.habitat.iter().filter(|h| h.resource == r.id).collect();
    match (wild, habitats.len()) {
        (true, 0) => problems.push(format!("resource '{}' grows in the wilderness but wilds.toml gives it no habitat", r.id)),
        (false, n) if n > 0 => problems.push(format!("wilds.toml gives '{}' a habitat but it doesn't grow in the wilderness", r.id)),
        (_, n) if n > 1 => problems.push(format!("wilds.toml gives '{}' {n} habitats", r.id)),
        _ => {}
    }
    for h in habitats {
        for (name, bound) in [("height", h.height), ("slope", h.slope), ("depth", h.depth)] {
            if bound.is_some_and(|[lo, hi]| !lo.is_finite() || !hi.is_finite() || lo > hi) {
                problems.push(format!("the habitat of '{}' has a {name} that isn't low to high", r.id));
            }
        }
        if h.slope.is_some_and(|[_, hi]| hi > capitol_protocol::rules::ground::CLIFF_SLOPE) {
            problems.push(format!("the habitat of '{}' runs steeper than anyone can climb", r.id));
        }
        if h.cover.is_some_and(|c| !(c.is_finite() && c > 0.0) || c > 1.0) {
            problems.push(format!("the habitat of '{}' has a cover outside 0..1", r.id));
        }
        if h.cover.is_some() != (r.cluster == "woods") {
            problems.push(format!("the habitat of '{}' has a cover exactly when it's woods", r.id));
        }
        if h.woods_of.is_some() && r.cluster != "woods" {
            problems.push(format!("the habitat of '{}' grows in another's woods but isn't woods", r.id));
        }
    }
}

/// Every habitat names a resource, and the quality rows run in order.
fn validate_wilds(wilds: &WildsDef, resources: &[ResourceDef], problems: &mut Vec<String>) {
    for h in &wilds.habitat {
        if !resources.iter().any(|r| r.id == h.resource) {
            problems.push(format!("wilds.toml gives a habitat to unknown resource '{}'", h.resource));
        }
        if let Some(of) = &h.woods_of {
            if !resources.iter().any(|r| &r.id == of && r.cluster == "woods") {
                problems.push(format!("the habitat of '{}' grows in the woods of '{of}', which isn't a woods resource", h.resource));
            }
        }
    }
    for (name, rows) in [("depth", &wilds.quality.depth), ("height", &wilds.quality.height)] {
        if rows.is_empty() || rows.windows(2).any(|w| w[0][0] >= w[1][0]) || rows.iter().flatten().any(|v| !v.is_finite()) {
            problems.push(format!("wilds.toml's quality {name} rows aren't a rising list"));
        }
    }
}

/// A loop of items where each is an ingredient of the next, if there is one.
fn recipe_cycle(recipes: &[RecipeDef]) -> Option<Vec<String>> {
    let mut makes: HashMap<&str, Vec<&str>> = HashMap::new();
    for r in recipes {
        for i in &r.inputs {
            let next = makes.entry(i.item_type.as_str()).or_default();
            next.extend(r.outputs.iter().map(|o| o.item_type.as_str()));
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Done,
    }
    fn visit<'a>(
        item: &'a str,
        makes: &HashMap<&'a str, Vec<&'a str>>,
        marks: &mut HashMap<&'a str, Mark>,
        path: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        match marks.get(item) {
            Some(Mark::Done) => return None,
            Some(Mark::Open) => {
                let start = path.iter().position(|p| *p == item).unwrap_or(0);
                let mut cycle: Vec<String> = path[start..].iter().map(|s| s.to_string()).collect();
                cycle.push(item.to_string());
                return Some(cycle);
            }
            None => {}
        }
        marks.insert(item, Mark::Open);
        path.push(item);
        for next in makes.get(item).into_iter().flatten() {
            if let Some(cycle) = visit(next, makes, marks, path) {
                return Some(cycle);
            }
        }
        path.pop();
        marks.insert(item, Mark::Done);
        None
    }

    let mut marks = HashMap::new();
    let mut starts: Vec<&str> = makes.keys().copied().collect();
    starts.sort_unstable();
    starts
        .into_iter()
        .find_map(|item| visit(item, &makes, &mut marks, &mut Vec::new()))
}

/// [`load`], [`validate`] and [`sync`]: what `main` runs at startup.
pub async fn load_and_sync(pool: &PgPool) -> Result<SyncReport, ContentError> {
    let content = load()?;
    let problems = validate(&content);
    if !problems.is_empty() {
        return Err(ContentError::Invalid(problems));
    }
    sync(pool, &content).await
}

/// What [`sync`] did.
#[derive(Debug, Default)]
pub struct SyncReport {
    /// Rows written, inserted or updated alike, across all the tables.
    pub upserted: usize,
    /// Rows kept because `retired.toml` lists them.
    pub retired: usize,
}

/// Write `content` into the static tables, in one transaction. Refuses, and
/// writes nothing, when a table holds a row the content neither defines nor
/// retires.
pub async fn sync(pool: &PgPool, content: &Content) -> Result<SyncReport, ContentError> {
    let mut tx = pool.begin().await?;

    let mut removed = Vec::new();
    let mut retired = 0;
    let tables: [(&'static str, Vec<&str>, &[String]); 5] = [
        ("item_types", content.items.iter().map(|i| i.id.as_str()).collect(), &content.retired.items),
        (
            "resource_types",
            content.resources.iter().map(|r| r.id.as_str()).collect(),
            &content.retired.resources,
        ),
        (
            "container_types",
            content.containers.iter().map(|c| c.id.as_str()).collect(),
            &content.retired.containers,
        ),
        (
            "station_types",
            content.stations.iter().map(|s| s.id.as_str()).collect(),
            &content.retired.stations,
        ),
        ("recipes", content.recipes.iter().map(|r| r.id.as_str()).collect(), &content.retired.recipes),
    ];
    for (table, defined, kept) in &tables {
        let rows: Vec<String> = sqlx::query_scalar(&format!("SELECT id FROM {table} ORDER BY id"))
            .fetch_all(&mut *tx)
            .await?;
        for id in rows {
            if kept.contains(&id) {
                retired += 1;
            } else if !defined.contains(&id.as_str()) {
                removed.push(Removed { table, id });
            }
        }
    }
    if !removed.is_empty() {
        return Err(ContentError::Removed(removed));
    }

    // In foreign-key order: kits before the stations that name them, stations
    // before the recipes that need them.
    let upserted = upsert(&mut tx, content).await?;
    tx.commit().await?;
    Ok(SyncReport { upserted, retired })
}

async fn upsert(tx: &mut Transaction<'_, Postgres>, content: &Content) -> Result<usize, sqlx::Error> {
    let mut n = 0;
    for i in &content.items {
        sqlx::query(
            "INSERT INTO item_types (id, name, category, stackable, weight,
                 tool_type, tier, base_power, max_durability, target_price)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, category = EXCLUDED.category,
                 stackable = EXCLUDED.stackable, weight = EXCLUDED.weight,
                 tool_type = EXCLUDED.tool_type, tier = EXCLUDED.tier,
                 base_power = EXCLUDED.base_power, max_durability = EXCLUDED.max_durability,
                 target_price = EXCLUDED.target_price",
        )
        .bind(&i.id)
        .bind(&i.name)
        .bind(&i.category)
        .bind(i.stackable)
        .bind(i.weight)
        .bind(i.tool.as_ref().map(|t| &t.tool_type))
        .bind(i.tool.as_ref().map(|t| &t.tier))
        .bind(i.tool.as_ref().map(|t| t.base_power))
        .bind(i.tool.as_ref().map(|t| t.max_durability))
        .bind(i.target_price)
        .execute(&mut **tx)
        .await?;
        n += 1;
    }
    for r in &content.resources {
        // Validated: there's always a row
        let first = r.first_yield().unwrap_or(ResourceYield { min: 1, max: 1, ticks: None });
        sqlx::query(
            "INSERT INTO resource_types (id, name, yields_item, yield_quantity_min, yield_quantity_max,
                 base_quality_min, base_quality_max, required_tool, extraction_duration_ticks, regeneration_ticks,
                 tool_type, difficulty, yields, cluster, spawns)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, yields_item = EXCLUDED.yields_item,
                 yield_quantity_min = EXCLUDED.yield_quantity_min, yield_quantity_max = EXCLUDED.yield_quantity_max,
                 base_quality_min = EXCLUDED.base_quality_min, base_quality_max = EXCLUDED.base_quality_max,
                 required_tool = EXCLUDED.required_tool,
                 extraction_duration_ticks = EXCLUDED.extraction_duration_ticks,
                 regeneration_ticks = EXCLUDED.regeneration_ticks,
                 tool_type = EXCLUDED.tool_type, difficulty = EXCLUDED.difficulty,
                 yields = EXCLUDED.yields, cluster = EXCLUDED.cluster, spawns = EXCLUDED.spawns",
        )
        .bind(&r.id)
        .bind(&r.name)
        .bind(&r.yields_item)
        .bind(first.min)
        .bind(first.max)
        .bind(r.base_quality_min)
        .bind(r.base_quality_max)
        .bind(&r.required_tool)
        .bind(r.extraction_duration_ticks)
        .bind(r.regeneration_ticks)
        .bind(r.tool())
        .bind(r.difficulty)
        .bind(sqlx::types::Json(r.yield_table()))
        .bind(&r.cluster)
        .bind(sqlx::types::Json(r.spawn_table()))
        .execute(&mut **tx)
        .await?;
        n += 1;
    }
    for c in &content.containers {
        sqlx::query(
            "INSERT INTO container_types (id, name, slot_count, layout_columns)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, slot_count = EXCLUDED.slot_count,
                 layout_columns = EXCLUDED.layout_columns",
        )
        .bind(&c.id)
        .bind(&c.name)
        .bind(c.slot_count)
        .bind(c.layout_columns)
        .execute(&mut **tx)
        .await?;
        n += 1;
    }
    for s in &content.stations {
        sqlx::query(
            "INSERT INTO station_types (id, name, category, slot_count, layout_columns,
                 interaction_range, icon, description, kit_item, off_plot)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, category = EXCLUDED.category,
                 slot_count = EXCLUDED.slot_count, layout_columns = EXCLUDED.layout_columns,
                 interaction_range = EXCLUDED.interaction_range, icon = EXCLUDED.icon,
                 description = EXCLUDED.description, kit_item = EXCLUDED.kit_item,
                 off_plot = EXCLUDED.off_plot",
        )
        .bind(&s.id)
        .bind(&s.name)
        .bind(&s.category)
        .bind(s.slot_count)
        .bind(s.layout_columns)
        .bind(s.interaction_range)
        .bind(&s.icon)
        .bind(&s.description)
        .bind(&s.kit)
        .bind(s.off_plot)
        .execute(&mut **tx)
        .await?;
        n += 1;
    }
    for r in &content.recipes {
        sqlx::query(
            "INSERT INTO recipes (id, name, description, category, inputs, outputs, base_duration_ticks,
                 quality_weights, crafting_requirements, required_station_type)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, description = EXCLUDED.description,
                 category = EXCLUDED.category, inputs = EXCLUDED.inputs, outputs = EXCLUDED.outputs,
                 base_duration_ticks = EXCLUDED.base_duration_ticks,
                 quality_weights = EXCLUDED.quality_weights,
                 crafting_requirements = EXCLUDED.crafting_requirements,
                 required_station_type = EXCLUDED.required_station_type",
        )
        .bind(&r.id)
        .bind(&r.name)
        .bind(&r.description)
        .bind(&r.category)
        .bind(sqlx::types::Json(&r.inputs))
        .bind(sqlx::types::Json(&r.outputs))
        .bind(r.base_duration_ticks)
        .bind(sqlx::types::Json(&r.quality_weights))
        .bind(sqlx::types::Json(&r.crafting_requirements))
        .bind(&r.required_station_type)
        .execute(&mut **tx)
        .await?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_content_parses_and_holds_together() {
        let content = load().unwrap();
        let problems = validate(&content);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn the_validator_catches_what_it_should() {
        let mut content = load().unwrap();
        content.recipes[0].inputs[0].item_type = "moonbeam".into();
        content.items.push(content.items[0].clone());
        content.items.push(ItemDef { id: "lint".into(), ..content.items[0].clone() });
        content.retired.recipes.push(content.recipes[1].id.clone());
        let problems = validate(&content).join("\n");
        assert!(problems.contains("names item 'moonbeam'"), "{problems}");
        assert!(problems.contains("item 'fiber' is defined twice"), "{problems}");
        assert!(problems.contains("item 'lint' can't be had"), "{problems}");
        assert!(problems.contains("is both defined and retired"), "{problems}");
    }

    #[test]
    fn the_validator_checks_quality_weights() {
        let mut content = load().unwrap();
        content.recipes[0].quality_weights.variance_min = 5;
        content.recipes[0].quality_weights.variance_max = -5;
        content.recipes[1].inputs[0].quality_weight = -1.0;
        content.recipes[2].quality_weights.station = Some(0.8);
        content.recipes[2].quality_weights.tool = Some(0.4);
        let problems = validate(&content).join("\n");
        assert!(problems.contains("variance_min above variance_max"), "{problems}");
        assert!(problems.contains("negative or not a number"), "{problems}");
        assert!(problems.contains("more than the whole"), "{problems}");
    }

    #[test]
    fn the_validator_checks_tools() {
        let mut content = load().unwrap();
        let sickle = content.items.iter_mut().find(|i| i.id == "stone_sickle").unwrap();
        sickle.stackable = true;
        let tool = sickle.tool.as_mut().unwrap();
        tool.tool_type = "spoon".into();
        tool.tier = "mithril".into();
        tool.max_durability = 0;
        let recipe = content.recipes.iter_mut().find(|r| r.id == "craft_stone_sickle").unwrap();
        recipe.outputs[0].quantity_max = 2;
        let problems = validate(&content).join("
");
        assert!(problems.contains("unknown type 'spoon'"), "{problems}");
        assert!(problems.contains("unknown tier 'mithril'"), "{problems}");
        assert!(problems.contains("no power or no durability"), "{problems}");
        assert!(problems.contains("tools don't stack"), "{problems}");
        assert!(problems.contains("other than one at a time"), "{problems}");
    }

    #[test]
    fn the_validator_checks_tools_at_work() {
        let mut content = load().unwrap();
        let grass = content.resources.iter_mut().find(|r| r.id == "grass").unwrap();
        grass.difficulty = -1.0;
        let mut gated = grass.clone();
        gated.id = "reeds".into();
        gated.required_tool = Some("axe".into());
        gated.difficulty = 3.0;
        let mut unknown = gated.clone();
        unknown.id = "moss".into();
        unknown.required_tool = Some("spoon".into());
        unknown.tool_type = None;
        content.resources.extend([gated, unknown]);
        content.recipes[0].crafting_requirements.tool_type = Some("ladle".into());
        content.recipes[1].quality_weights.tool = Some(0.2);
        let problems = validate(&content).join("\n");
        assert!(problems.contains("'grass' has a difficulty that is negative"), "{problems}");
        assert!(problems.contains("'reeds' requires a axe but is worked by a sickle"), "{problems}");
        assert!(problems.contains("'moss' names unknown tool type 'spoon'"), "{problems}");
        assert!(problems.contains("needs unknown tool type 'ladle'"), "{problems}");
        let share = format!("recipe '{}' must name a tool type", content.recipes[0].id);
        assert!(problems.contains(&share), "a tool with no share: {problems}");
        let share = format!("recipe '{}' must name a tool type", content.recipes[1].id);
        assert!(problems.contains(&share), "a share with no tool: {problems}");
    }

    /// Every resource in D5, with its tool, yield rows and spawn table
    #[test]
    fn every_d5_resource_is_content() {
        let content = load().unwrap();
        let d5 = [
            ("grass", "fiber", None, Some("sickle"), 0.0),
            ("fallen_branches", "sticks", None, Some("axe"), 0.0),
            ("clay_bank", "clay", None, None, 2.0),
            ("rock_outcrop", "stone", None, Some("pickaxe"), 8.0),
            ("tree", "log", Some("axe"), Some("axe"), 12.0),
            ("iron_vein", "iron_ore", Some("pickaxe"), Some("pickaxe"), 20.0),
            // The wilds' own (#225)
            ("old_growth", "log", Some("axe"), Some("axe"), 16.0),
            ("copper_vein", "copper_ore", Some("pickaxe"), Some("pickaxe"), 14.0),
            ("tin_vein", "tin_ore", Some("pickaxe"), Some("pickaxe"), 14.0),
            ("reed_bed", "reeds", Some("sickle"), Some("sickle"), 6.0),
        ];
        assert_eq!(content.resources.len(), d5.len());
        for (id, item, required, tool, difficulty) in d5 {
            let r = content.resources.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no {id}"));
            assert_eq!(r.yields_item, item);
            assert_eq!((r.required_tool.as_deref(), r.tool(), r.difficulty), (required, tool, difficulty), "{id}");
            assert_eq!(r.yields.contains_key("hand"), required.is_none(), "{id}");
            assert!(!r.spawns.is_empty(), "{id} grows nowhere");
        }
        // D5's own numbers, spot-checked
        let outcrop = content.resources.iter().find(|r| r.id == "rock_outcrop").unwrap();
        assert_eq!(outcrop.yields["hand"].ticks, Some(80));
        let vein = content.resources.iter().find(|r| r.id == "iron_vein").unwrap();
        assert_eq!((vein.yields["iron"].min, vein.yields["iron"].max), (1, 2));
        assert_eq!(vein.spawns.iter().find(|s| s.zone == "wilderness").unwrap().quality_floor, Some(47));
    }

    #[test]
    fn the_validator_checks_yields_and_spawns() {
        let mut content = load().unwrap();
        let tree = content.resources.iter_mut().find(|r| r.id == "tree").unwrap();
        tree.yields.insert("hand".into(), YieldDef { min: 1, max: 1, ticks: None });
        tree.yields.insert("mithril".into(), YieldDef { min: 3, max: 2, ticks: Some(0) });
        tree.cluster = "heap".into();
        tree.spawns.push(SpawnDef { zone: "rural".into(), density: 0.0, quality_floor: Some(99) });
        tree.spawns.push(SpawnDef { zone: "moon".into(), density: 1.0, quality_floor: None });
        let clay = content.resources.iter_mut().find(|r| r.id == "clay_bank").unwrap();
        clay.yields.clear();
        clay.yields.insert("stone".into(), YieldDef { min: 1, max: 2, ticks: None });
        let problems = validate(&content).join("\n");
        for expected in [
            "'tree' needs a axe but has a hand yield row",
            "'tree' has a yield row 'mithril', which is neither hand nor a tier",
            "'tree' yields 3..2 at mithril",
            "'tree' takes no time at mithril",
            "'tree' has unknown cluster 'heap'",
            "'tree' lists zone 'rural' twice",
            "'tree' has a density in 'rural' that isn't above 0",
            "'tree' has a quality floor in 'rural' outside 35..80",
            "'tree' spawns in unknown zone 'moon'",
            "'clay_bank' has a stone yield row but no tool type",
            "'clay_bank' needs no tool but has no hand yield row",
        ] {
            assert!(problems.contains(expected), "missing {expected:?} in:\n{problems}");
        }
    }

    /// W6 (#199): every resource that grows in the wilds has one habitat
    /// there, with bounds low to high, never steeper than a walker climbs.
    #[test]
    fn the_validator_checks_the_wilds() {
        let mut content = load().unwrap();
        content.wilds.habitat.retain(|h| h.resource != "clay_bank");
        let vein = content.wilds.habitat.iter_mut().find(|h| h.resource == "iron_vein").unwrap();
        vein.height = Some([900.0, 380.0]);
        vein.slope = Some([0.5, 2.0]);
        let tree = content.wilds.habitat.iter_mut().find(|h| h.resource == "old_growth").unwrap();
        tree.cover = None;
        let copper = content.wilds.habitat.iter_mut().find(|h| h.resource == "copper_vein").unwrap();
        copper.woods_of = Some("rock_outcrop".into());
        content.wilds.habitat.push(HabitatDef {
            resource: "grass".into(),
            height: None,
            slope: None,
            depth: None,
            cover: None,
            woods_of: None,
            massifs: None,
        });
        content.wilds.habitat.push(HabitatDef {
            resource: "moss".into(),
            height: None,
            slope: None,
            depth: None,
            cover: None,
            woods_of: None,
            massifs: None,
        });
        content.wilds.quality.depth = vec![[4000.0, 0.0], [3800.0, 5.0]];
        let problems = validate(&content).join("\n");
        for expected in [
            "'clay_bank' grows in the wilderness but wilds.toml gives it no habitat",
            "the habitat of 'iron_vein' has a height that isn't low to high",
            "the habitat of 'iron_vein' runs steeper than anyone can climb",
            "the habitat of 'old_growth' has a cover exactly when it's woods",
            "the habitat of 'copper_vein' grows in another's woods but isn't woods",
            "the habitat of 'copper_vein' grows in the woods of 'rock_outcrop', which isn't a woods resource",
            "wilds.toml gives 'grass' a habitat but it doesn't grow in the wilderness",
            "wilds.toml gives a habitat to unknown resource 'moss'",
            "wilds.toml's quality depth rows aren't a rising list",
        ] {
            assert!(problems.contains(expected), "missing {expected:?} in:\n{problems}");
        }
        assert!(validate(&load().unwrap()).is_empty());
    }

    /// The ceiling follows the rows, holds flat past their ends, and stays
    /// between the floor and 100.
    #[test]
    fn a_wild_ceiling_rises_with_depth_and_height() {
        let q = WildQualityDef { depth: vec![[3800.0, -10.0], [4800.0, 10.0]], height: vec![[0.0, 0.0], [500.0, 5.0]] };
        assert_eq!(q.ceiling(80, 50, 3000.0, 0.0), 70);
        assert_eq!(q.ceiling(80, 50, 4300.0, 0.0), 80);
        assert_eq!(q.ceiling(80, 50, 4300.0, 250.0), 83);
        assert_eq!(q.ceiling(80, 50, 6000.0, 900.0), 95);
        assert_eq!(q.ceiling(95, 50, 6000.0, 900.0), 100);
        assert_eq!(q.ceiling(55, 50, 3800.0, 0.0), 50);
        assert!(q.ceiling(80, 50, 4000.0, 0.0) < q.ceiling(80, 50, 4600.0, 0.0));
    }

    /// A gated resource, or a recipe that needs a tool, can only be had once a
    /// tool of that type can.
    #[test]
    fn a_tool_gate_needs_a_tool_that_can_be_had() {
        let mut content = load().unwrap();
        let grass = content.resources.iter_mut().find(|r| r.id == "grass").unwrap();
        grass.required_tool = Some("sickle".into());
        let problems = validate(&content).join("\n");
        assert!(problems.contains("item 'fiber' can't be had"), "the sickle needs fiber: {problems}");

        let mut content = load().unwrap();
        let rope = content.recipes.iter_mut().find(|r| r.outputs[0].item_type == "rope").unwrap();
        rope.crafting_requirements.tool_type = Some("saw".into());
        rope.quality_weights.tool = Some(0.2);
        let problems = validate(&content).join("\n");
        assert!(problems.contains("item 'rope' can't be had"), "no saw anywhere: {problems}");
    }

    #[test]
    fn a_circle_of_recipes_is_found() {
        let mut content = load().unwrap();
        // Rope back into fiber: fiber -> rope -> fiber.
        let mut unrope = content.recipes[0].clone();
        unrope.id = "unrope".into();
        unrope.inputs[0].item_type = "rope".into();
        unrope.outputs[0].item_type = "fiber".into();
        content.recipes.push(unrope);
        let problems = validate(&content).join("\n");
        assert!(problems.contains("recipes go round in a circle"), "{problems}");
    }

    #[test]
    fn a_station_needs_a_kit_recipe_and_something_to_make() {
        let mut content = load().unwrap();
        content.recipes.retain(|r| r.id != "craft_chest" && r.required_station_type.as_deref() != Some("workbench"));
        let problems = validate(&content).join("\n");
        assert!(problems.contains("no recipe makes its kit 'chest_kit'"), "{problems}");
        assert!(problems.contains("station 'workbench' has nothing to make"), "{problems}");
        assert!(!problems.contains("station 'storage_chest' has nothing"), "storage makes nothing");
    }

    async fn static_tables(db: &PgPool) -> Vec<String> {
        let mut rows = Vec::new();
        for table in ["item_types", "resource_types", "container_types", "station_types", "recipes"] {
            let json: Vec<String> = sqlx::query_scalar(&format!(
                "SELECT row_to_json(t)::text FROM (SELECT * FROM {table} ORDER BY id) t"
            ))
            .fetch_all(db)
            .await
            .unwrap();
            rows.extend(json.into_iter().map(|row| format!("{table} {row}")));
        }
        rows
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_content_keeps_what_the_migrations_seeded(db: PgPool) {
        // E0.3 moved the seeds across unchanged. Content has grown since (the
        // stone sickle, E1.4; D5's tree, E2.4), so the sync may add rows, but
        // every seeded row comes through it as it was, and a second sync
        // changes nothing. Rows the content changes on purpose are named here.
        const CHANGED_ON_PURPOSE: &[&str] = &[
            // D5: the forge kit is bricks, stone and rope, under a hammer
            "recipes {\"id\":\"craft_forge\"",
        ];
        // D2.3 gives items target prices, which no seed had
        fn untargeted(row: &str) -> String {
            match row.split_once("\"target_price\":") {
                Some((head, tail)) => {
                    let rest = tail.trim_start_matches(|c: char| c.is_ascii_digit());
                    format!("{head}\"target_price\":null{rest}")
                }
                None => row.to_string(),
            }
        }
        let before = static_tables(&db).await;
        let report = sync(&db, &load().unwrap()).await.unwrap();
        assert_eq!(report.retired, 0);
        let after = static_tables(&db).await;
        let untargeted_after: Vec<_> = after.iter().map(|row| untargeted(row)).collect();
        let changed: Vec<_> = before
            .iter()
            .filter(|row| !untargeted_after.contains(row))
            .filter(|row| !CHANGED_ON_PURPOSE.iter().any(|p| row.starts_with(p)))
            .collect();
        assert!(changed.is_empty(), "the sync changed seeded rows:
{changed:#?}");
        sync(&db, &load().unwrap()).await.unwrap();
        assert_eq!(static_tables(&db).await, after);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_item_removed_without_retiring_it_stops_the_sync(db: PgPool) {
        let mut content = load().unwrap();
        content.items.retain(|i| i.id != "rope");
        content.items[0].name = "Changed".into();
        let err = sync(&db, &content).await.unwrap_err();
        let message = err.to_string();
        assert!(message.contains("item_types has a row 'rope'"), "{message}");
        assert!(message.contains("content/retired.toml"), "{message}");
        let name: String = sqlx::query_scalar("SELECT name FROM item_types WHERE id = 'fiber'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(name, "Plant Fiber", "nothing is written");

        // Retired, it stays in the table as it was.
        content.retired.items.push("rope".into());
        let report = sync(&db, &content).await.unwrap();
        assert_eq!(report.retired, 1);
        let rope: String = sqlx::query_scalar("SELECT name FROM item_types WHERE id = 'rope'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(rope, "Rope");
    }
}
