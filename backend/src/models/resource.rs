use capitol_protocol::rules::resources::{self, ResourceYield, Spawn, YieldTable};
use capitol_protocol::rules::tools::{Tier, ToolType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::FromRow;
use uuid::Uuid;

pub use capitol_protocol::rest::ResourceNodeInfo;

/// Static resource type definition (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResourceType {
    pub id: String,
    pub name: String,
    pub yields_item: String,
    pub yield_quantity_min: i32,
    pub yield_quantity_max: i32,
    pub base_quality_min: i32,
    pub base_quality_max: i32,
    /// A tool type it can't be gathered without (E1.5); then `tool_type` too
    pub required_tool: Option<String>,
    pub extraction_duration_ticks: i32,
    pub regeneration_ticks: i32,
    /// The tool type that works it: the best one held is used (E1.5)
    pub tool_type: Option<String>,
    /// How hard it is to work, on the tools' power scale (`rules::tools::leverage`)
    pub difficulty: f64,
    /// Units a gather gives, by `hand` or tool tier (E2.2)
    pub yields: Json<YieldTable>,
    /// How its nodes group (`rules::resources::Cluster`)
    pub cluster: String,
    /// The zones its nodes grow in (E2.2)
    pub spawns: Json<Vec<Spawn>>,
}

impl ResourceType {
    /// The tool type that works it, if any. The content validator rejects an
    /// unknown one, and the schema makes a required tool its tool type.
    pub fn tool(&self) -> Option<ToolType> {
        self.tool_type.as_deref().and_then(ToolType::from_str)
    }

    /// The tool type it can't be gathered without, if any.
    pub fn required(&self) -> Option<ToolType> {
        self.required_tool.as_deref().and_then(ToolType::from_str)
    }

    /// The row of its yield table a gather with a tool of `tier`, or by hand
    /// (`None`), reads (`rules::resources::yield_for`).
    pub fn yield_for(&self, tier: Option<Tier>) -> Option<&ResourceYield> {
        resources::yield_for(&self.yields, tier)
    }
}

/// Resource node in the world (from database)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResourceNode {
    pub id: Uuid,
    pub resource_type: String,
    pub position_x: f64,
    pub position_y: f64,
    pub base_quality: i32,
    pub state: String,
    pub harvested_by: Option<Uuid>,
    pub regenerates_at_tick: Option<i64>,
    pub created_at: DateTime<Utc>,
}

/// Node state for in-memory game state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    Available,
    BeingHarvested,
    Depleted,
}

impl From<&str> for NodeState {
    fn from(s: &str) -> Self {
        match s {
            "available" => NodeState::Available,
            "harvesting" => NodeState::BeingHarvested,
            "depleted" => NodeState::Depleted,
            _ => NodeState::Available,
        }
    }
}

impl From<NodeState> for String {
    fn from(state: NodeState) -> Self {
        match state {
            NodeState::Available => "available".to_string(),
            NodeState::BeingHarvested => "harvesting".to_string(),
            NodeState::Depleted => "depleted".to_string(),
        }
    }
}

/// In-memory resource node state for the game engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceNodeState {
    pub id: Uuid,
    pub resource_type: String,
    pub x: f64,
    pub y: f64,
    pub base_quality: i32,
    pub state: NodeState,
    pub harvested_by: Option<Uuid>,
    pub regenerates_at_tick: Option<u64>,
}

impl From<ResourceNode> for ResourceNodeState {
    fn from(node: ResourceNode) -> Self {
        Self {
            id: node.id,
            resource_type: node.resource_type,
            x: node.position_x,
            y: node.position_y,
            base_quality: node.base_quality,
            state: NodeState::from(node.state.as_str()),
            harvested_by: node.harvested_by,
            regenerates_at_tick: node.regenerates_at_tick.map(|t| t as u64),
        }
    }
}

impl From<&ResourceNodeState> for ResourceNodeInfo {
    fn from(node: &ResourceNodeState) -> Self {
        Self {
            id: node.id,
            resource_type: node.resource_type.clone(),
            x: node.x,
            y: node.y,
            state: String::from(node.state),
        }
    }
}

#[cfg(test)]
mod tests {
    use capitol_protocol::fmath::StableMath;
    use capitol_protocol::rules::{
        check_water_crossing, Zone, CENTER_X, CENTER_Y, NEARBY_NODE_RANGE, SPAWN_SCATTER,
        SPAWN_X, SPAWN_Y, WORLD_SIZE,
    };

    use capitol_protocol::terrain::quad_contains;
    use uuid::Uuid;

    use crate::engine::node_placement::{place_nodes, KEEP_CLEAR};
    use crate::engine::node_spawn::{self, NodeSeed, SpawnTable};
    use crate::models::terrain::{point_to_segment_distance, TerrainConfig, WorldGenParams};

    const WORLD_SCALE_MIGRATION: &str =
        include_str!("../../migrations/20260212_001_world_scale.sql");

    /// Moves for seeded nodes, in the order the migrations run: the ones a later
    /// street plan ran over (#62), the ones the deeper suburb moved (#117),
    /// then the ones the Neighbourhood suburb's streets ran over (#112). All
    /// three are on the 8000 world, before W1's shift; a move migration
    /// written after it (from `print_node_moves`) is on the 12000 world, and
    /// applies after `shifted`.
    const NODE_MOVES_MIGRATIONS: [&str; 3] = [
        include_str!("../../migrations/20260215_001_suburban_street_nodes.sql"),
        include_str!("../../migrations/20261018_001_deeper_suburb_nodes.sql"),
        include_str!("../../migrations/20261019_001_neighbourhood_suburb.sql"),
    ];

    /// How far W1's migration moved every stored position on both axes, when
    /// the world grew to 12000 (#194). The earlier migrations' rows are on the
    /// 8000 world.
    const WORLD_SHIFT: f64 = 2000.0;
    const WORLD_GROWS_MIGRATION: &str = include_str!("../../migrations/20261107_001_world_grows.sql");

    /// A row from before the world grew, where W1's migration put it.
    fn shifted(x: f64) -> f64 {
        ((x + WORLD_SHIFT) * 10.0).round() / 10.0
    }

    #[test]
    fn the_world_grows_by_the_shift() {
        assert_eq!(WORLD_SHIFT, CENTER_X - 4000.0);
        assert!(WORLD_GROWS_MIGRATION.contains("position_x = position_x + 2000"));
    }

    /// The numbers in each `(a, b, ...)` row of a migration's VALUES list.
    fn value_rows(sql: &str) -> Vec<Vec<f64>> {
        sql.lines()
            .filter_map(|line| {
                let row = line.trim().strip_prefix('(')?;
                let row = row.trim_end_matches(',').strip_suffix(')')?;
                row.split(',').map(|f| f.trim().parse().ok()).collect()
            })
            .collect()
    }

    /// The `(x, y, quality)` rows the world-scale migration seeds, where the
    /// later migrations leave them: moved by the node move migrations, then
    /// by the world's growth.
    fn seeded_nodes() -> Vec<(f64, f64, i32)> {
        let migrations: Vec<Vec<Vec<f64>>> = NODE_MOVES_MIGRATIONS.iter().map(|sql| value_rows(sql)).collect();
        value_rows(WORLD_SCALE_MIGRATION)
            .into_iter()
            .filter(|row| row.len() == 3)
            .map(|row| {
                let (x, y) = migrations.iter().fold((row[0], row[1]), |(x, y), moves| {
                    moves.iter().find(|m| m[0] == x && m[1] == y).map_or((x, y), |m| (m[2], m[3]))
                });
                (shifted(x), shifted(y), row[2] as i32)
            })
            .collect()
    }

    /// How far a point is from the edge of the nearest road; negative inside one.
    fn road_clearance(terrain: &TerrainConfig, x: f64, y: f64) -> f64 {
        terrain
            .roads
            .iter()
            .flat_map(|road| {
                road.points.windows(2).map(move |w| {
                    point_to_segment_distance(x, y, w[0].0, w[0].1, w[1].0, w[1].1)
                        - road.width / 2.0
                })
            })
            .fold(f64::MAX, f64::min)
    }

    /// The seeds were placed against the default world. A live world built
    /// with other params has its nodes moved by `node_placement` (#70).
    #[test]
    fn seeded_nodes_sit_clear_of_the_default_terrain() {
        let terrain = TerrainConfig::get_default();
        let nodes = seeded_nodes();
        assert_eq!(nodes.len(), 156);

        for &(x, y, quality) in &nodes {
            let clearance = road_clearance(&terrain, x, y);
            // The rule startup's `place_nodes` moves a node by. Before #112 the
            // seeds also sat 20 clear, but the Neighbourhood suburb's streets
            // pass some at 15-20, where nothing moves them.
            assert!(clearance >= KEEP_CLEAR, "node at ({x}, {y}) is {clearance:.1} from a road");
            assert!((40..=70).contains(&quality), "node at ({x}, {y}) has quality {quality}");
            let near_spawn = (x - SPAWN_X).stable_hypot(y - SPAWN_Y) <= NEARBY_NODE_RANGE;
            let zone = Zone::from_position(x, y);
            assert!(
                near_spawn || matches!(zone, Zone::Suburban | Zone::Rural),
                "node at ({x}, {y}) is in {zone}"
            );
        }
    }

    fn seeded_positions() -> Vec<(Uuid, (f64, f64))> {
        seeded_nodes()
            .iter()
            .enumerate()
            .map(|(i, &(x, y, _))| (Uuid::from_u128(i as u128), (x, y)))
            .collect()
    }

    /// The default world is the one the seeds were placed for, so fitting the
    /// nodes to it moves none of them. Startup changes nothing there.
    #[test]
    fn the_default_terrain_moves_no_seeded_node() {
        let report = place_nodes(&TerrainConfig::get_default(), &seeded_positions());
        assert!(report.moves.is_empty(), "{} seeded nodes moved: {:?}", report.moves.len(), report.moves);
        assert!(report.stuck.is_empty());
    }

    /// Prints the VALUES rows of a node move migration: the seeded nodes the
    /// default world's roads run over, from where the migrations leave them
    /// to where `place_nodes` puts them. Run it after a change that moves the
    /// default world (`default_terrain_version_is_pinned`), paste the rows into
    /// a new migration, and add that to `NODE_MOVES_MIGRATIONS` (#112):
    /// `cargo test print_node_moves -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_node_moves() {
        let report = place_nodes(&TerrainConfig::get_default(), &seeded_positions());
        for m in &report.moves {
            println!("    ({:.1}, {:.1}, {:.1}, {:.1}),", m.from.0, m.from.1, m.to.0, m.to.1);
        }
        println!("{} moved, {} stuck: {:?}", report.moves.len(), report.stuck.len(), report.stuck);
    }

    /// Another seed and wider streets run roads over seeded nodes. Every node
    /// ends clear of them, and fitting again moves nothing.
    #[test]
    fn a_regenerated_terrain_moves_the_nodes_it_runs_over() {
        let mut params = WorldGenParams { seed: 7, ..WorldGenParams::default() };
        params.suburban.neighbourhood.local_width *= 2.0;
        params.suburban.neighbourhood.collector_width *= 2.0;
        params.arterial_width *= 1.5;
        let terrain = TerrainConfig::from_params(&params);

        let report = place_nodes(&terrain, &seeded_positions());
        assert!(!report.moves.is_empty(), "the new streets miss every seeded node");
        assert!(report.stuck.is_empty(), "{} nodes found no clear spot", report.stuck.len());

        let mut fitted = seeded_positions();
        for m in &report.moves {
            fitted.iter_mut().find(|n| n.0 == m.id).unwrap().1 = m.to;
        }
        for &(_, (x, y)) in &fitted {
            let clearance = road_clearance(&terrain, x, y);
            assert!(clearance >= KEEP_CLEAR, "node at ({x}, {y}) is {clearance:.1} from a road");
        }
        for m in &report.moves {
            let (x, y) = m.to;
            assert!(!terrain.terrain_plots.iter().any(|lot| quad_contains(&lot.corners, x, y)), "node moved onto a lot at ({x}, {y})");
            assert_eq!(Zone::from_position(m.from.0, m.from.1), Zone::from_position(x, y));
        }
        assert!(place_nodes(&terrain, &fitted).moves.is_empty(), "a second fit moved nodes again");
    }

    #[test]
    fn a_new_player_arrives_on_the_road_in_with_grass_in_sight() {
        let terrain = TerrainConfig::get_default();
        assert_eq!(Zone::from_position(SPAWN_X, SPAWN_Y), Zone::Rural);

        // On the arterial, even at the edge of the scatter, with room for the
        // player's own radius.
        for dx in [-SPAWN_SCATTER, 0.0, SPAWN_SCATTER] {
            let clearance = road_clearance(&terrain, SPAWN_X + dx, SPAWN_Y);
            assert!(clearance <= -8.0, "spawn + {dx} is {clearance:.1} from the road edge");
        }

        // The walk straight in reaches the Capitol over bridges.
        let center = (CENTER_X, CENTER_Y);
        let blocked =
            check_water_crossing(center, &terrain.water_rings, (SPAWN_X, SPAWN_Y), center);
        assert!(blocked.is_none(), "the road in is cut by {blocked:?}");

        // `/world/nearby` from the spawn shows grass.
        let in_sight = seeded_nodes()
            .iter()
            .filter(|&&(x, y, _)| (x - SPAWN_X).stable_hypot(y - SPAWN_Y) <= NEARBY_NODE_RANGE)
            .count();
        assert!(in_sight >= 4, "only {in_sight} nodes in sight of the spawn");
    }

    /// E2.3's nodes on the default world, kept clear of the seeds as the
    /// migration's were.
    fn default_plan() -> Vec<NodeSeed> {
        let existing: Vec<(f64, f64)> = seeded_nodes().iter().map(|&(x, y, _)| (x, y)).collect();
        node_spawn::plan(&TerrainConfig::get_default(), &content_tables(), &existing, WorldGenParams::default().seed)
    }

    fn content_tables() -> Vec<SpawnTable> {
        crate::content::load().unwrap().resources.iter().map(|r| SpawnTable::from(&r.to_type())).collect()
    }

    const NEW_NODES_MIGRATION: &str = include_str!("../../migrations/20261106_001_seeded_nodes_by_zone.sql");

    /// W6's (#199): the wilds' nodes, on the 12000 world.
    const WILD_NODES_MIGRATION: &str = include_str!("../../migrations/20261113_001_wild_nodes.sql");

    /// The planned nodes the default world's water moved (#222).
    const PLANNED_MOVES_MIGRATION: &str = include_str!("../../migrations/20261112_001_wild_water_nodes.sql");

    /// The plan, where the migrations leave its nodes: as planned, then moved
    /// by [`PLANNED_MOVES_MIGRATION`] (its `('id', x, y)` rows).
    fn planned_nodes() -> Vec<NodeSeed> {
        let moves: std::collections::HashMap<Uuid, (f64, f64)> = PLANNED_MOVES_MIGRATION
            .lines()
            .filter_map(|line| {
                let row = line.trim().strip_prefix("('")?.trim_end_matches([',', ';']).strip_suffix(')')?;
                let f: Vec<&str> = row.split(',').map(|f| f.trim().trim_matches('\'')).collect();
                Some((f[0].parse().ok()?, (f[1].parse().ok()?, f[2].parse().ok()?)))
            })
            .collect();
        default_plan()
            .into_iter()
            .map(|s| match moves.get(&s.id) {
                Some(&(x, y)) => NodeSeed { x, y, ..s },
                None => s,
            })
            .collect()
    }

    /// The `('id', 'type', x, y, quality)` rows of a node seed insert, where
    /// W1's migration moved them.
    fn insert_rows(sql: &str) -> Vec<NodeSeed> {
        insert_rows_at(sql, shifted)
    }

    /// The rows of the node seed inserts the migrations make, in order: E2.3's
    /// (from before W1's shift), then W6's.
    fn migrated_plan() -> Vec<NodeSeed> {
        let mut rows = insert_rows(NEW_NODES_MIGRATION);
        rows.extend(insert_rows_at(WILD_NODES_MIGRATION, |v| v));
        rows
    }

    /// The `('id', 'type', x, y, quality)` rows of a node seed insert, each
    /// coordinate through `at`.
    fn insert_rows_at(sql: &str, at: fn(f64) -> f64) -> Vec<NodeSeed> {
        sql.lines()
            .filter_map(|line| {
                let row = line.trim().strip_prefix("('")?.trim_end_matches(',').strip_suffix(')')?;
                let f: Vec<&str> = row.split(',').map(|f| f.trim().trim_matches('\'')).collect();
                Some(NodeSeed {
                    id: f[0].parse().ok()?,
                    resource_type: f[1].to_string(),
                    x: at(f[2].parse().ok()?),
                    y: at(f[3].parse().ok()?),
                    base_quality: f[4].parse().ok()?,
                })
            })
            .collect()
    }

    /// D5: woods, branches, outcrops and veins in the rural ring, clay by the
    /// canal and in the suburb's parks, branches in the parks too. Each at
    /// about its density, and none on a road, the water or a lot.
    #[test]
    fn the_default_world_grows_what_d5_says() {
        let terrain = TerrainConfig::get_default();
        let plan = default_plan();
        let tables = content_tables();
        for table in &tables {
            for spawn in &table.spawns {
                let zone = Zone::from_str(&spawn.zone).unwrap();
                if !table.is_planned() || zone == Zone::Wilderness {
                    continue;
                }
                let (inner, outer) = zone.radii();
                let want = spawn.density * std::f64::consts::PI * (outer * outer - inner * inner) / 1e6;
                let got = plan.iter().filter(|s| s.resource_type == table.id && Zone::from_position(s.x, s.y) == zone).count();
                assert!(got as f64 >= 0.95 * want.round(), "{} in {zone}: {got} of {want:.0}", table.id);
            }
        }
        for s in &plan {
            let clearance = road_clearance(&terrain, s.x, s.y);
            assert!(clearance >= KEEP_CLEAR, "{} at ({}, {}) is {clearance:.1} from a road", s.resource_type, s.x, s.y);
            assert!(terrain.lot_at(s.x, s.y).is_none(), "{} on a lot at ({}, {})", s.resource_type, s.x, s.y);
            let r = (s.x - CENTER_X).stable_hypot(s.y - CENTER_Y);
            assert!(terrain.water_rings.iter().all(|w| r < w.inner_radius() - KEEP_CLEAR || r > w.outer_radius() + KEEP_CLEAR));
            let table = tables.iter().find(|t| t.id == s.resource_type).unwrap();
            // A wild node's ceiling rises past its type's (W6, `wilds.toml`).
            let top = if Zone::from_position(s.x, s.y) == Zone::Wilderness { 100 } else { table.quality.1 };
            assert!((table.quality.0..=top).contains(&s.base_quality));
        }
        let suburb_branches = plan.iter().filter(|s| s.resource_type == "fallen_branches" && Zone::from_position(s.x, s.y) == Zone::Suburban);
        for s in suburb_branches {
            assert!(terrain.parks.iter().any(|p| p.contains(s.x, s.y)), "suburban branches outside the parks at ({}, {})", s.x, s.y);
        }
    }

    /// The new nodes are where startup's fit leaves them, as the seeds are:
    /// where the plan put them, or where the water's migration moved them.
    #[test]
    fn the_default_terrain_moves_no_planned_node() {
        let mut nodes = seeded_positions();
        let n = nodes.len() as u128;
        nodes.extend(planned_nodes().iter().enumerate().map(|(i, s)| (Uuid::from_u128(n + i as u128), (s.x, s.y))));
        let report = place_nodes(&TerrainConfig::get_default(), &nodes);
        assert!(report.moves.is_empty(), "{} nodes moved: {:?}", report.moves.len(), report.moves);
    }

    /// Prints the `('id', x, y)` rows of a planned node move migration: E2.3's
    /// nodes that a change to the default world puts in the way, where
    /// startup's fit would move them (#222). Paste them into a new migration
    /// and point [`PLANNED_MOVES_MIGRATION`] at it:
    /// `cargo test print_planned_node_moves -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_planned_node_moves() {
        let planned = planned_nodes();
        let nodes: Vec<(Uuid, (f64, f64))> =
            seeded_positions().into_iter().chain(planned.iter().map(|s| (s.id, (s.x, s.y)))).collect();
        let report = place_nodes(&TerrainConfig::get_default(), &nodes);
        for m in &report.moves {
            println!("    ('{}', {:.1}, {:.1}),", m.id, m.to.0, m.to.1);
        }
        println!("{} moved, {} stuck: {:?}", report.moves.len(), report.stuck.len(), report.stuck);
    }

    /// The migrations write exactly the default world's plan. After a change to
    /// the placer, the content's spawn tables or the default world, write a new
    /// migration from `print_new_nodes`.
    #[test]
    fn the_migration_writes_the_default_plan() {
        let rows = migrated_plan();
        let plan = default_plan();
        assert_eq!(rows.len(), plan.len());
        for (row, seed) in rows.iter().zip(&plan) {
            assert_eq!(row, seed);
        }
    }

    /// No node stands in the wild water (W3, #196; on the rural plain since
    /// #222): not the seeded grass, nor E2.3's nodes where the migrations
    /// leave them.
    #[test]
    fn no_node_stands_in_the_wild_water() {
        let terrain = TerrainConfig::get_default();
        let planned = planned_nodes();
        let nodes = seeded_positions().into_iter().map(|(_, p)| p).chain(planned.iter().map(|s| (s.x, s.y)));
        let mut wet = Vec::new();
        for (x, y) in nodes {
            if terrain.lakes.iter().any(|l| l.contains(x, y)) {
                wet.push(format!("a lake at ({x}, {y})"));
            }
            for c in terrain.rivers.iter().chain(&terrain.streams) {
                for w in c.points.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    if point_to_segment_distance(x, y, a.x, a.y, b.x, b.y) <= a.width.max(b.width) / 2.0 {
                        wet.push(format!("{} at ({x}, {y})", c.id));
                    }
                }
            }
        }
        assert!(wet.is_empty(), "{} wet: {:?}", wet.len(), wet);
    }

    /// Prints the VALUES rows of a node insert for the default world's planned
    /// nodes that no migration writes yet:
    /// `cargo test print_new_nodes -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_new_nodes() {
        let migrated: std::collections::HashSet<Uuid> = migrated_plan().iter().map(|s| s.id).collect();
        let plan: Vec<NodeSeed> = default_plan().into_iter().filter(|s| !migrated.contains(&s.id)).collect();
        for s in &plan {
            println!("    ('{}', '{}', {:.1}, {:.1}, {}),", s.id, s.resource_type, s.x, s.y, s.base_quality);
        }
        let mut counts = std::collections::BTreeMap::new();
        for s in &plan {
            *counts.entry((s.resource_type.clone(), Zone::from_position(s.x, s.y).as_str())).or_insert(0) += 1;
        }
        for ((t, z), n) in counts {
            println!("-- {t} in {z}: {n}");
        }
    }

    /// W6 (#199): the wilds grow from the land. Each resource at about its
    /// density, where its habitat in `wilds.toml` says: veins high on the
    /// rock of the ranges, clay on the banks, old growth on the valley floors.
    /// None stands in the water, on a trail, on a cliff, or where a walker
    /// from the rural edge can't get to.
    #[test]
    fn the_wilds_grow_from_the_land() {
        use crate::engine::node_placement::MOVE_CLEAR;
        use crate::engine::node_spawn::{wilds::Land, BANK_CLEAR};
        use capitol_protocol::land::{height_at, slope_at};
        use capitol_protocol::rules::ground::CLIFF_SLOPE;

        let terrain = TerrainConfig::get_default();
        let land = Land::new(&terrain);
        let plan = default_plan();
        let wild: Vec<&NodeSeed> = plan.iter().filter(|s| Zone::from_position(s.x, s.y) == Zone::Wilderness).collect();
        let tables = content_tables();
        let (inner, outer) = Zone::Wilderness.radii();
        let km2 = std::f64::consts::PI * (outer * outer - inner * inner) / 1e6;
        for table in &tables {
            let Some(spawn) = table.spawns.iter().find(|s| s.zone == Zone::Wilderness.as_str()) else { continue };
            let want = (spawn.density * km2).round();
            let got = wild.iter().filter(|s| s.resource_type == table.id).count();
            assert!(got as f64 >= 0.95 * want, "{} in the wilds: {got} of {want}", table.id);
        }
        for s in &wild {
            let (h, slope) = (height_at(s.x, s.y, &terrain.land), slope_at(s.x, s.y, &terrain.land));
            let at = format!("{} at ({}, {}), height {h:.0}, slope {slope:.2}", s.resource_type, s.x, s.y);
            let habitat = tables.iter().find(|t| t.id == s.resource_type).unwrap().habitat.as_ref().unwrap();
            let r = (s.x - CENTER_X).stable_hypot(s.y - CENTER_Y);
            assert!(habitat.suits(r, h, slope), "{at} isn't in its habitat");
            assert!(slope <= CLIFF_SLOPE, "{at} is on a cliff");
            assert!(land.reached((s.x, s.y)), "{at} can't be reached");
            let road = road_clearance(&terrain, s.x, s.y);
            assert!(road >= MOVE_CLEAR, "{at} is {road:.1} from a road or trail");
            let water = land.water_clearance((s.x, s.y));
            let clear = if s.resource_type == "clay_bank" { BANK_CLEAR } else { MOVE_CLEAR };
            assert!(water >= clear, "{at} is {water:.1} from the water");
            match s.resource_type.as_str() {
                "iron_vein" => assert!(h >= 300.0 && slope >= 0.4, "{at} isn't in the ranges"),
                "tree" => assert!(h <= 250.0 && slope <= 0.4, "{at} isn't on a valley floor"),
                _ => {}
            }
        }
        // Clay on the banks: within the bank strip of some water.
        let on_a_bank = |x: f64, y: f64| {
            let course = terrain.rivers.iter().chain(&terrain.streams).any(|c| {
                c.points.windows(2).any(|w| {
                    let half = w[0].width.max(w[1].width) / 2.0;
                    point_to_segment_distance(x, y, w[0].x, w[0].y, w[1].x, w[1].y) <= half + 50.0
                })
            });
            let shore = terrain.lakes.iter().any(|l| {
                l.rings.iter().any(|ring| {
                    (0..ring.len()).any(|i| {
                        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                        point_to_segment_distance(x, y, a.0, a.1, b.0, b.1) <= 50.0
                    })
                })
            });
            course || shore
        };
        let clay: Vec<&&NodeSeed> = wild.iter().filter(|s| s.resource_type == "clay_bank").collect();
        assert!(!clay.is_empty());
        for s in clay {
            assert!(on_a_bank(s.x, s.y), "clay at ({}, {}) is off the banks", s.x, s.y);
        }
    }

    /// W6 (#199): the base quality rises from the rural edge to the ranges,
    /// for each wild resource found at both, and for the wilds as a whole.
    #[test]
    fn wild_quality_rises_into_the_ranges() {
        let plan = default_plan();
        let r = |s: &NodeSeed| (s.x - CENTER_X).stable_hypot(s.y - CENTER_Y);
        let mean = |v: &[&NodeSeed]| v.iter().map(|s| s.base_quality as f64).sum::<f64>() / v.len() as f64;
        let wild: Vec<&NodeSeed> = plan.iter().filter(|s| Zone::from_position(s.x, s.y) == Zone::Wilderness).collect();
        let edge: Vec<&NodeSeed> = wild.iter().copied().filter(|s| r(s) < 4300.0).collect();
        let ranges: Vec<&NodeSeed> = wild.iter().copied().filter(|s| r(s) > 5000.0).collect();
        let (near, far) = (mean(&edge), mean(&ranges));
        assert!(far > near + 5.0, "the wilds: {near:.1} by the rural edge, {far:.1} in the ranges");
        for kind in ["tree", "fallen_branches", "rock_outcrop", "clay_bank"] {
            let near: Vec<&NodeSeed> = edge.iter().copied().filter(|s| s.resource_type == kind).collect();
            let far: Vec<&NodeSeed> = ranges.iter().copied().filter(|s| s.resource_type == kind).collect();
            if near.len() < 20 || far.len() < 20 {
                continue;
            }
            assert!(mean(&far) > mean(&near), "{kind}: {:.1} by the rural edge, {:.1} in the ranges", mean(&near), mean(&far));
        }
        assert!(wild.iter().all(|s| (0..=100).contains(&s.base_quality)));
    }

    /// Draws the default world's nodes by type over its rings, roads, lots and
    /// parks, for review: the whole world, and a close look at the rural ring
    /// east of the road in. `backend/tools/node-plan.ps1` runs it and makes PNGs:
    /// `cargo test --release node_plan_svg -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn node_plan_svg() {
        use std::fmt::Write;
        let out = std::env::var("NODE_PLAN_OUT").unwrap_or_else(|_| "../target/node-plan".into());
        std::fs::create_dir_all(&out).unwrap();
        let terrain = TerrainConfig::get_default();
        let plan = default_plan();
        let seeds: Vec<(f64, f64)> = seeded_nodes().iter().map(|&(x, y, _)| (x, y)).collect();
        const KINDS: [(&str, &str, &str); 7] = [
            ("grass", "#9cc46a", "wild grass (seeded before)"),
            ("tree", "#2f6b3a", "tree"),
            ("fallen_branches", "#a0773f", "fallen branches"),
            ("rock_outcrop", "#7d7d86", "rock outcrop"),
            ("iron_vein", "#b5402a", "iron vein"),
            ("clay_bank", "#d9925b", "clay bank"),
            ("", "", ""),
        ];
        let views = [
            ("nodes-world", (CENTER_X - WORLD_SIZE / 2.0, CENTER_Y - WORLD_SIZE / 2.0, WORLD_SIZE), 2400.0, 3.0),
            ("nodes-rural-east", (CENTER_X + 2850.0, CENTER_Y - 600.0, 1100.0), 1100.0, 5.0),
            ("nodes-urban-north", (CENTER_X - 550.0, CENTER_Y - 1850.0, 1100.0), 1100.0, 5.0),
        ];
        for (name, (x0, y0, span), px, dot) in views {
            let k = px / span;
            let at = |x: f64, y: f64| ((x - x0) * k, (y - y0) * k);
            let mut svg = String::new();
            let legend_h = 40.0;
            writeln!(svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{px}" height="{}" viewBox="0 0 {px} {}">"#, px + legend_h, px + legend_h).unwrap();
            writeln!(svg, r##"<rect width="100%" height="100%" fill="#f3eee2"/>"##).unwrap();
            for zone in Zone::ALL {
                let (cx, cy) = at(CENTER_X, CENTER_Y);
                writeln!(svg, r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="{:.1}" fill="none" stroke="#c9bfa8" stroke-dasharray="6 4"/>"##, zone.radii().1 * k).unwrap();
            }
            for park in &terrain.parks {
                let d: String = park.rings.iter().map(|r| {
                    let pts: Vec<String> = r.iter().map(|&(x, y)| { let (a, b) = at(x, y); format!("{a:.1},{b:.1}") }).collect();
                    format!("M{}Z", pts.join("L"))
                }).collect();
                writeln!(svg, r##"<path d="{d}" fill="#d7e6c0" fill-rule="evenodd"/>"##).unwrap();
            }
            for lot in &terrain.terrain_plots {
                let pts: Vec<String> = lot.corners.iter().map(|&(x, y)| { let (a, b) = at(x, y); format!("{a:.1},{b:.1}") }).collect();
                writeln!(svg, r##"<polygon points="{}" fill="#e6dcc6" stroke="#d6cab0" stroke-width="0.5"/>"##, pts.join(" ")).unwrap();
            }
            for ring in &terrain.water_rings {
                let (cx, cy) = at(CENTER_X, CENTER_Y);
                writeln!(svg, r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="{:.1}" fill="none" stroke="#6f9fc4" stroke-width="{:.1}"/>"##, ring.radius * k, (ring.width * k).max(1.0)).unwrap();
            }
            for road in &terrain.roads {
                let pts: Vec<String> = road.points.iter().map(|&(x, y)| { let (a, b) = at(x, y); format!("{a:.1},{b:.1}") }).collect();
                writeln!(svg, r##"<polyline points="{}" fill="none" stroke="#a89f8f" stroke-width="{:.1}" stroke-linecap="round"/>"##, pts.join(" "), (road.width * k).max(0.8)).unwrap();
            }
            let dots = seeds.iter().map(|&p| ("grass", p)).chain(plan.iter().map(|s| (s.resource_type.as_str(), (s.x, s.y))));
            for (kind, (x, y)) in dots {
                let (a, b) = at(x, y);
                if a < -dot || b < -dot || a > px + dot || b > px + dot {
                    continue;
                }
                let colour = KINDS.iter().find(|c| c.0 == kind).map_or("#000", |c| c.1);
                writeln!(svg, r#"<circle cx="{a:.1}" cy="{b:.1}" r="{dot}" fill="{colour}" fill-opacity="0.9"/>"#).unwrap();
            }
            let mut lx = 12.0;
            for (kind, colour, label) in KINDS.iter().filter(|c| !c.0.is_empty()) {
                let n = if *kind == "grass" { seeds.len() } else { plan.iter().filter(|s| s.resource_type == *kind).count() };
                writeln!(svg, r##"<circle cx="{lx}" cy="{}" r="6" fill="{colour}"/><text x="{}" y="{}" font-family="Georgia, serif" font-size="14" fill="#2e2a24">{label} ({n})</text>"##, px + 20.0, lx + 10.0, px + 25.0).unwrap();
                lx += 12.0 + 8.0 * (label.len() as f64 + 6.0);
            }
            svg.push_str("</svg>\n");
            let path = format!("{out}/{name}.svg");
            std::fs::write(&path, svg).unwrap();
            println!("wrote {path}");
        }
    }
}
