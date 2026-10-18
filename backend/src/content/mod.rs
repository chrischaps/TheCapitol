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

use std::collections::{HashMap, HashSet};
use std::fmt;

use capitol_protocol::rules::tools;
use serde::Deserialize;
use sqlx::{PgPool, Postgres, Transaction};

/// Crafting stations with no recipe that needs them yet. Each one is a gap
/// in the content, not a rule: E2.4 (#163) gives the forge something to make.
const NOTHING_TO_MAKE_YET: &[&str] = &["forge"];

const ITEMS: &str = include_str!("../../content/items.toml");
const RESOURCES: &str = include_str!("../../content/resources.toml");
const CONTAINERS: &str = include_str!("../../content/containers.toml");
const STATIONS: &str = include_str!("../../content/stations.toml");
const RECIPES: &str = include_str!("../../content/recipes.toml");
const RETIRED: &str = include_str!("../../content/retired.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDef {
    pub id: String,
    pub name: String,
    pub category: String,
    pub stackable: bool,
    pub weight: f32,
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
    pub yield_quantity_min: i32,
    pub yield_quantity_max: i32,
    pub base_quality_min: i32,
    pub base_quality_max: i32,
    pub required_tool: Option<String>,
    pub extraction_duration_ticks: i32,
    pub regeneration_ticks: i32,
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
    #[serde(default = "empty_table")]
    pub crafting_requirements: serde_json::Value,
}

fn empty_table() -> serde_json::Value {
    serde_json::json!({})
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
        if let Some(tool) = &r.required_tool {
            item(&mut problems, format!("resource '{}' (its tool)", r.id), tool);
        }
    }
    for s in &content.stations {
        item(&mut problems, format!("station '{}' (its kit)", s.id), &s.kit);
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
    // (given its tool), then what recipes make from that (given their station).
    let kit_of: HashMap<&str, &str> =
        content.stations.iter().map(|s| (s.id.as_str(), s.kit.as_str())).collect();
    let mut had: HashSet<&str> = HashSet::new();
    loop {
        let before = had.len();
        for r in &content.resources {
            if r.required_tool.as_deref().is_none_or(|t| had.contains(t)) {
                had.insert(&r.yields_item);
            }
        }
        for r in &content.recipes {
            let station = r
                .required_station_type
                .as_deref()
                .is_none_or(|s| kit_of.get(s).is_some_and(|kit| had.contains(kit)));
            if station && r.inputs.iter().all(|i| had.contains(i.item_type.as_str())) {
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
        if s.category != "storage" && !needed && !NOTHING_TO_MAKE_YET.contains(&s.id.as_str()) {
            problems.push(format!("station '{}' has nothing to make: no recipe needs it", s.id));
        }
    }

    problems
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
                 tool_type, tier, base_power, max_durability)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, category = EXCLUDED.category,
                 stackable = EXCLUDED.stackable, weight = EXCLUDED.weight,
                 tool_type = EXCLUDED.tool_type, tier = EXCLUDED.tier,
                 base_power = EXCLUDED.base_power, max_durability = EXCLUDED.max_durability",
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
        .execute(&mut **tx)
        .await?;
        n += 1;
    }
    for r in &content.resources {
        sqlx::query(
            "INSERT INTO resource_types (id, name, yields_item, yield_quantity_min, yield_quantity_max,
                 base_quality_min, base_quality_max, required_tool, extraction_duration_ticks, regeneration_ticks)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, yields_item = EXCLUDED.yields_item,
                 yield_quantity_min = EXCLUDED.yield_quantity_min, yield_quantity_max = EXCLUDED.yield_quantity_max,
                 base_quality_min = EXCLUDED.base_quality_min, base_quality_max = EXCLUDED.base_quality_max,
                 required_tool = EXCLUDED.required_tool,
                 extraction_duration_ticks = EXCLUDED.extraction_duration_ticks,
                 regeneration_ticks = EXCLUDED.regeneration_ticks",
        )
        .bind(&r.id)
        .bind(&r.name)
        .bind(&r.yields_item)
        .bind(r.yield_quantity_min)
        .bind(r.yield_quantity_max)
        .bind(r.base_quality_min)
        .bind(r.base_quality_max)
        .bind(&r.required_tool)
        .bind(r.extraction_duration_ticks)
        .bind(r.regeneration_ticks)
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
                 interaction_range, icon, description, kit_item)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, category = EXCLUDED.category,
                 slot_count = EXCLUDED.slot_count, layout_columns = EXCLUDED.layout_columns,
                 interaction_range = EXCLUDED.interaction_range, icon = EXCLUDED.icon,
                 description = EXCLUDED.description, kit_item = EXCLUDED.kit_item",
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
    fn kits_agree_with_the_protocol() {
        // The Flint client still pairs kits and stations through
        // `rules::station_kit_item` and `station_type_for_kit`, not the
        // catalog's `kit`. Until it doesn't (#181), the two must agree.
        use capitol_protocol::rules::{station_kit_item, station_type_for_kit};
        for s in load().unwrap().stations {
            assert_eq!(station_kit_item(&s.id), Some(s.kit.as_str()), "{}", s.id);
            assert_eq!(station_type_for_kit(&s.kit), Some(s.id.as_str()), "{}", s.kit);
        }
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
        content.recipes.retain(|r| !["craft_chest", "craft_forge", "craft_stone_sickle"].contains(&r.id.as_str()));
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
        // stone sickle, E1.4), so the sync may add rows, but every seeded row
        // comes through it as it was, and a second sync changes nothing.
        let before = static_tables(&db).await;
        let report = sync(&db, &load().unwrap()).await.unwrap();
        assert_eq!(report.retired, 0);
        let after = static_tables(&db).await;
        let changed: Vec<_> = before.iter().filter(|row| !after.contains(row)).collect();
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
