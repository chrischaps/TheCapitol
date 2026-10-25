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
        check_water_crossing, Zone, CENTER_X, CENTER_Y, NEARBY_NODE_RANGE, RURAL_OUTER_RADIUS, SPAWN_SCATTER,
        SPAWN_X, SPAWN_Y, WORLD_SIZE,
    };

    use capitol_protocol::terrain::quad_contains;
    use uuid::Uuid;

    use crate::engine::node_placement::{place_nodes, KEEP_CLEAR};
    use crate::engine::node_spawn::{self, arrival, NodeSeed, SpawnTable};
    use crate::models::terrain::{point_to_segment_distance, TerrainConfig, WorldGenParams};

    /// A short walk from the spawn, for Arrival's nodes (#443): what
    /// `/world/nearby` shows from it.
    const ARRIVAL_WALK: f64 = NEARBY_NODE_RANGE;

    const WORLD_SCALE_MIGRATION: &str =
        include_str!("../../migrations/20260212_001_world_scale.sql");

    /// Moves for seeded nodes, in the order the migrations run: the ones a later
    /// street plan ran over (#62), the ones the deeper suburb moved (#117),
    /// then the ones the Neighbourhood suburb's streets ran over (#112). All
    /// three are on the 8000 world, before W1's shift. A move migration
    /// written since goes in [`SHIFTED_NODE_MOVES_MIGRATIONS`].
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

    /// How far RW1's migration moved them again, when the rural ring widened
    /// and the world grew to 13600 (#308). Rows written between the two are
    /// on the 12000 world.
    const RURAL_WIDENS_SHIFT: f64 = 800.0;
    const RURAL_WIDENS_MIGRATION: &str = include_str!("../../migrations/20261122_001_rural_widens.sql");

    /// Moves for seeded nodes written on the 13600 world, after both shifts,
    /// in the order they run: the farms' lanes ran over two (R7, #290). From
    /// `print_node_moves`.
    const SHIFTED_NODE_MOVES_MIGRATIONS: [&str; 1] =
        [include_str!("../../migrations/20261126_001_rural_farms.sql")];

    /// How far Plots v2's flip moved them again, when every zone took its v2
    /// rings and the world grew to 16600 (#459).
    const FLIP_SHIFT: f64 = 1500.0;
    /// The flip, which shifts them and then moves the seeded grass out with
    /// the rings (`print_grass_remap`'s rows).
    const FLIP_MIGRATION: &str = include_str!("../../migrations/20261220_001_plots_v2_flip.sql");

    /// Moves for seeded nodes written on the 16600 world, after the flip's
    /// own, in the order they run. From `print_node_moves`.
    const FLIPPED_NODE_MOVES_MIGRATIONS: [&str; 0] = [];

    /// A row from the 8000 world, where W1's and RW1's shifts put it.
    fn shifted(x: f64) -> f64 {
        ((x + WORLD_SHIFT + RURAL_WIDENS_SHIFT) * 10.0).round() / 10.0
    }

    /// A row from the 13600 world, where the flip's shift puts it on the
    /// 16600 one.
    fn flipped(x: f64) -> f64 {
        ((x + FLIP_SHIFT) * 10.0).round() / 10.0
    }

    #[test]
    fn the_world_grows_by_the_shifts() {
        assert_eq!(WORLD_SHIFT + RURAL_WIDENS_SHIFT + FLIP_SHIFT, CENTER_X - 4000.0);
        assert!(WORLD_GROWS_MIGRATION.contains("position_x = position_x + 2000"));
        assert!(RURAL_WIDENS_MIGRATION.contains("position_x = position_x + 800"));
        assert!(FLIP_MIGRATION.contains("position_x = position_x + 1500"));
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

    /// Each move migration's `(x, y, new x, new y)` rows.
    fn move_rows(migrations: &[&str]) -> Vec<Vec<Vec<f64>>> {
        migrations.iter().map(|sql| value_rows(sql).into_iter().filter(|row| row.len() == 4).collect()).collect()
    }

    /// Where `migrations` leave a node at `(x, y)`, each matching on where the
    /// one before left it.
    fn moved(migrations: &[Vec<Vec<f64>>], at: (f64, f64)) -> (f64, f64) {
        migrations.iter().fold(at, |(x, y), moves| {
            moves.iter().find(|m| m[0] == x && m[1] == y).map_or((x, y), |m| (m[2], m[3]))
        })
    }

    /// The `(x, y, quality)` rows the world-scale migration seeds, where the
    /// migrations before the flip's grass leave them: moved by the node move
    /// migrations, then by the world's growth, then by the moves written
    /// since, then by the flip's shift.
    fn flipped_nodes() -> Vec<(f64, f64, i32)> {
        let before = move_rows(&NODE_MOVES_MIGRATIONS);
        let after = move_rows(&SHIFTED_NODE_MOVES_MIGRATIONS);
        value_rows(WORLD_SCALE_MIGRATION)
            .into_iter()
            .filter(|row| row.len() == 3)
            .map(|row| {
                let (x, y) = moved(&before, (row[0], row[1]));
                let (x, y) = moved(&after, (shifted(x), shifted(y)));
                (flipped(x), flipped(y), row[2] as i32)
            })
            .collect()
    }

    /// The seeded rows where every migration leaves them: the flip's grass
    /// moved out with the rings (`FLIP_MIGRATION`), then the moves since.
    fn seeded_nodes() -> Vec<(f64, f64, i32)> {
        let flip = move_rows(&[FLIP_MIGRATION]);
        let since = move_rows(&FLIPPED_NODE_MOVES_MIGRATIONS);
        flipped_nodes()
            .into_iter()
            .map(|(x, y, q)| {
                let (x, y) = moved(&flip, (x, y));
                let (x, y) = moved(&since, (x, y));
                (x, y, q)
            })
            .collect()
    }

    /// Where Plots v2's rings put a seeded node from RW1's (#459): its
    /// radius remapped zone by zone, the suburb's 1800 to 3000 onto 2160 to
    /// 3600 and the rural ring's 3000 to 4600 onto 3600 to 6100, on the same
    /// bearing from the centre. So a node keeps its zone and its place
    /// across it. The grass in sight of the spawn moves with it instead,
    /// 600 out along the south arterial (from r3350 to r3950), so a new
    /// player still arrives with grass in sight: remapped, only 2 of its 6
    /// would be.
    fn remapped(x: f64, y: f64) -> (f64, f64) {
        const PIECES: [((f64, f64), (f64, f64)); 2] = [((1800.0, 3000.0), (2160.0, 3600.0)), ((3000.0, 4600.0), (3600.0, 6100.0))];
        let spawn_before = (SPAWN_X, SPAWN_Y - 600.0);
        if (x - spawn_before.0).stable_hypot(y - spawn_before.1) <= NEARBY_NODE_RANGE {
            return (x, ((y + 600.0) * 10.0).round() / 10.0);
        }
        let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
        let r = dx.stable_hypot(dy);
        let Some(((a, b), (c, d))) = PIECES.into_iter().find(|((a, b), _)| (*a..=*b).contains(&r)) else { return (x, y) };
        let k = (c + (r - a) * (d - c) / (b - a)) / r;
        (((CENTER_X + dx * k) * 10.0).round() / 10.0, ((CENTER_Y + dy * k) * 10.0).round() / 10.0)
    }

    /// Prints the VALUES rows of the flip's grass move (#459): each seeded
    /// node from where the shift leaves it to where Plots v2's rings put it
    /// (`remapped`), then fitted to the default world by `place_nodes`, clear
    /// of its roads, lots and water and of each other:
    /// `cargo test print_grass_remap -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_grass_remap() {
        let nodes = flipped_nodes();
        let at: Vec<(Uuid, (f64, f64))> =
            nodes.iter().enumerate().map(|(i, &(x, y, _))| (Uuid::from_u128(i as u128), remapped(x, y))).collect();
        let report = place_nodes(TerrainConfig::default_world(), &at);
        let mut stayed = 0;
        for (i, &(x, y, _)) in nodes.iter().enumerate() {
            let id = Uuid::from_u128(i as u128);
            let to = report.moves.iter().find(|m| m.id == id).map_or(at[i].1, |m| m.to);
            if to == (x, y) {
                stayed += 1;
                continue;
            }
            println!("    ({x:.1}, {y:.1}, {:.1}, {:.1}),", to.0, to.1);
        }
        println!("-- {} remapped, {} fitted, {stayed} stayed, {} stuck: {:?}", nodes.len() - stayed, report.moves.len(), report.stuck.len(), report.stuck);
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
        let terrain = TerrainConfig::default_world();
        let nodes = seeded_nodes();
        assert_eq!(nodes.len(), 156);

        for &(x, y, quality) in &nodes {
            let clearance = road_clearance(terrain, x, y);
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
        let report = place_nodes(TerrainConfig::default_world(), &seeded_positions());
        assert!(report.moves.is_empty(), "{} seeded nodes moved: {:?}", report.moves.len(), report.moves);
        assert!(report.stuck.is_empty());
    }

    /// Prints the VALUES rows of a node move migration: the seeded nodes the
    /// default world's roads run over, from where the migrations leave them
    /// to where `place_nodes` puts them. Run it after a change that moves the
    /// default world (`default_terrain_version_is_pinned`), paste the rows into
    /// a new migration, and add that to `FLIPPED_NODE_MOVES_MIGRATIONS` (#290):
    /// `cargo test print_node_moves -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_node_moves() {
        let report = place_nodes(TerrainConfig::default_world(), &seeded_positions());
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
        let stuck: Vec<(f64, f64)> = report.stuck.iter().map(|id| seeded_positions().iter().find(|n| n.0 == *id).unwrap().1).collect();
        assert!(stuck.is_empty(), "{} nodes found no clear spot: {stuck:?}", stuck.len());

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
        let terrain = TerrainConfig::default_world();
        assert_eq!(Zone::from_position(SPAWN_X, SPAWN_Y), Zone::Rural);

        // On the arterial, even at the edge of the scatter, with room for the
        // player's own radius.
        for dx in [-SPAWN_SCATTER, 0.0, SPAWN_SCATTER] {
            let clearance = road_clearance(terrain, SPAWN_X + dx, SPAWN_Y);
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

        // Arrival (#443) gathers grass, fallen branches and stone by hand and
        // fells a tree: every kind within a short walk, off every lot (so a
        // farmer's claim can't take the chapter from a newcomer), and enough
        // of each for a crowd to walk it at once with none waiting on a
        // regrowth. Before the commons (`node_spawn::arrival`) the nearest
        // tree stood 300 off and the nearest outcrop 466.
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        let nodes = seeded_nodes()
            .into_iter()
            .map(|(x, y, _)| ("grass".to_string(), x, y))
            .chain(planned_nodes().into_iter().map(|s| (s.resource_type, s.x, s.y)));
        for (kind, x, y) in nodes {
            let near = (x - SPAWN_X).stable_hypot(y - SPAWN_Y) <= ARRIVAL_WALK;
            if near && !terrain.terrain_plots.iter().any(|lot| quad_contains(&lot.corners, x, y)) {
                *counts.entry(kind).or_default() += 1;
            }
        }
        for (kind, pulls) in arrival::PULLS {
            let wanted = arrival::CROWD * pulls;
            let got = counts.get(kind).copied().unwrap_or(0);
            assert!(got >= wanted, "{got} {kind} within {ARRIVAL_WALK} of the spawn, off the lots, for {wanted}: {counts:?}");
        }
        assert!(counts.get("tree").copied().unwrap_or(0) >= arrival::COPSE_TREES, "a copse: {counts:?}");
    }

    /// E2.3's nodes on the default world, kept clear of the seeds as the
    /// migration's were.
    fn default_plan() -> Vec<NodeSeed> {
        let existing: Vec<(f64, f64)> = seeded_nodes().iter().map(|&(x, y, _)| (x, y)).collect();
        node_spawn::plan(TerrainConfig::default_world(), &content_tables(), &existing, WorldGenParams::default().seed)
    }

    /// Whether a planned node stands in the water on purpose (the reeds).
    fn grows_in_the_water(s: &NodeSeed) -> bool {
        static WET: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        let wet = WET.get_or_init(|| {
            crate::content::load().unwrap().resources.into_iter().filter(|r| r.cluster == "shallows").map(|r| r.id).collect()
        });
        wet.contains(&s.resource_type)
    }

    fn content_tables() -> Vec<SpawnTable> {
        crate::content::load().unwrap().resources.iter().map(|r| SpawnTable::from(&r.to_type())).collect()
    }

    /// The whole plan, E2.3's nodes and the wilds', on the 16600 world.
    /// Plots v2's flip re-planned it when every zone took its v2 rings
    /// (#459), and deleted every planned row before it: RW1's whole re-plan
    /// (`20261122_001_rural_widens_nodes`, which had deleted E2.3's, W6's,
    /// #225's and #222's before it) and the partial re-plans since.
    const PLAN_MIGRATION: &str = include_str!("../../migrations/post-content/20261220_001_plots_v2_flip_nodes.sql");

    /// Moves of planned nodes written since the re-plan, in the order they run:
    /// each a migration of `('id', x, y)` rows on the 16600 world, from
    /// [`print_planned_node_moves`], as #222's were before it.
    const PLANNED_MOVES_MIGRATIONS: [&str; 0] = [];

    /// The plan, where the migrations leave its nodes: as planned, then moved
    /// by [`PLANNED_MOVES_MIGRATIONS`].
    fn planned_nodes() -> Vec<NodeSeed> {
        let moves: std::collections::HashMap<Uuid, (f64, f64)> = PLANNED_MOVES_MIGRATIONS
            .iter()
            .flat_map(|sql| sql.lines())
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

    /// Re-plans of some of the nodes since the flip, in the order they run:
    /// each deletes the planned rows it lists by id (a `'id',` line), if any,
    /// and inserts its own (a `('id', 'type', x, y, quality)` line), from
    /// `print_node_replan`, or from `print_new_nodes` for one that only adds.
    const REPLAN_MIGRATIONS: [&str; 3] = [
        // Arrival's commons round the spawn (#443): only adds
        include_str!("../../migrations/post-content/20261222_001_arrival_commons.sql"),
        // The wilds on a spacing grid of their own (#340)
        include_str!("../../migrations/post-content/20261223_001_wilds_own_grid_nodes.sql"),
        // The wilds' nodes only where a walker gets to them (#347)
        include_str!("../../migrations/post-content/20261224_001_wilds_reached_nodes.sql"),
    ];

    /// The `('id', 'type', x, y, quality)` rows of the plan's insert: every
    /// planned node, since the flip re-planned them whole (#459), less what
    /// [`REPLAN_MIGRATIONS`] deleted and with what they inserted, by id.
    fn migrated_plan() -> Vec<NodeSeed> {
        let inserts = |sql: &'static str| {
            sql.lines().filter_map(|line| {
                let row = line.trim().strip_prefix("('")?.trim_end_matches([',', ';']).strip_suffix(')')?;
                let f: Vec<&str> = row.split(',').map(|f| f.trim().trim_matches('\'')).collect();
                Some(NodeSeed {
                    id: f[0].parse().ok()?,
                    resource_type: f[1].to_string(),
                    x: f[2].parse().ok()?,
                    y: f[3].parse().ok()?,
                    base_quality: f[4].parse().ok()?,
                })
            })
        };
        let mut rows: Vec<NodeSeed> = inserts(PLAN_MIGRATION).collect();
        for sql in REPLAN_MIGRATIONS {
            let deleted: std::collections::HashSet<Uuid> = sql
                .lines()
                .filter_map(|line| line.trim().trim_end_matches(',').strip_prefix('\'')?.strip_suffix('\'')?.parse().ok())
                .collect();
            rows.retain(|s| !deleted.contains(&s.id));
            rows.extend(inserts(sql));
        }
        rows.sort_by_key(|s| s.id);
        rows
    }

    /// D5: woods, branches, outcrops and veins in the rural ring, clay by the
    /// canal and in the suburb's parks, branches in the parks too. Each at
    /// about its density, and none on a road, the water or a lot.
    #[test]
    fn the_default_world_grows_what_d5_says() {
        let terrain = TerrainConfig::default_world();
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
            let clearance = road_clearance(terrain, s.x, s.y);
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
        let dry = planned_nodes().into_iter().filter(|s| !grows_in_the_water(s));
        nodes.extend(dry.enumerate().map(|(i, s)| (Uuid::from_u128(n + i as u128), (s.x, s.y))));
        let report = place_nodes(TerrainConfig::default_world(), &nodes);
        assert!(report.moves.is_empty(), "{} nodes moved: {:?}", report.moves.len(), report.moves);
    }

    /// Prints the `('id', x, y)` rows of a planned node move migration: the
    /// planned nodes that a change to the default world puts in the way, where
    /// startup's fit would move them (#222). Paste them into a new
    /// post-content migration and add it to [`PLANNED_MOVES_MIGRATIONS`]:
    /// `cargo test print_planned_node_moves -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_planned_node_moves() {
        let planned = planned_nodes();
        let nodes: Vec<(Uuid, (f64, f64))> =
            seeded_positions().into_iter().chain(planned.iter().map(|s| (s.id, (s.x, s.y)))).collect();
        let report = place_nodes(TerrainConfig::default_world(), &nodes);
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
        let mut plan = default_plan();
        plan.sort_by_key(|s| s.id);
        assert_eq!(rows.len(), plan.len());
        for (row, seed) in rows.iter().zip(&plan) {
            assert_eq!(row, seed);
        }
    }

    /// No node stands in the wild water (W3, #196; on the rural plain since
    /// #222): not the seeded grass, nor E2.3's nodes where the migrations
    /// leave them. Only the reeds, which grow there (#225).
    #[test]
    fn no_node_stands_in_the_wild_water() {
        let terrain = TerrainConfig::default_world();
        let planned: Vec<NodeSeed> = planned_nodes().into_iter().filter(|s| !grows_in_the_water(s)).collect();
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
    /// `cargo test print_new_nodes -- --ignored --nocapture`. They go in a new
    /// post-content migration (`migrations/post-content/`, #207) as a plain
    /// `INSERT INTO resource_nodes`, and in [`migrated_plan`]. With
    /// `PRINT_ALL_NODES=1` it prints the whole plan, for a re-plan that
    /// replaces every planned row, as RW1's (#308) and the flip's (#459) did.
    #[test]
    #[ignore]
    fn print_new_nodes() {
        let all = std::env::var_os("PRINT_ALL_NODES").is_some();
        let migrated: std::collections::HashSet<Uuid> =
            if all { Default::default() } else { migrated_plan().iter().map(|s| s.id).collect() };
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

    /// Prints a re-plan migration's body for the planned nodes the default
    /// world's plan no longer matches: the ids to delete (a `'id',` line),
    /// then the rows to insert (`('id', 'type', x, y, quality),`), both by id,
    /// for [`REPLAN_MIGRATIONS`]. After a change to the default world that
    /// moves part of the plan, as the flip to the farms did (#290):
    /// `cargo test print_node_replan -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_node_replan() {
        let migrated: std::collections::HashMap<Uuid, NodeSeed> = migrated_plan().into_iter().map(|s| (s.id, s)).collect();
        let mut plan = default_plan();
        plan.sort_by_key(|s| s.id);
        let wanted: std::collections::HashMap<Uuid, &NodeSeed> = plan.iter().map(|s| (s.id, s)).collect();
        let mut stale: Vec<&NodeSeed> = migrated.values().filter(|s| wanted.get(&s.id) != Some(s)).collect();
        stale.sort_by_key(|s| s.id);
        let fresh: Vec<&NodeSeed> = plan.iter().filter(|s| migrated.get(&s.id) != Some(s)).collect();
        for s in &stale {
            println!("    '{}',", s.id);
        }
        for s in &fresh {
            println!("    ('{}', '{}', {:.1}, {:.1}, {}),", s.id, s.resource_type, s.x, s.y, s.base_quality);
        }
        let count = |nodes: &[&NodeSeed]| {
            let mut counts = std::collections::BTreeMap::new();
            for s in nodes {
                *counts.entry((s.resource_type.clone(), Zone::from_position(s.x, s.y).as_str())).or_insert(0) += 1;
            }
            counts
        };
        println!("-- {} deleted: {:?}", stale.len(), count(&stale));
        println!("-- {} inserted: {:?}", fresh.len(), count(&fresh));
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

        let terrain = TerrainConfig::default_world();
        let land = Land::new(terrain);
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
        for s in wild.iter().filter(|s| s.resource_type != "reed_bed") {
            let (h, slope) = (height_at(s.x, s.y, &terrain.land), slope_at(s.x, s.y, &terrain.land));
            let at = format!("{} at ({}, {}), height {h:.0}, slope {slope:.2}", s.resource_type, s.x, s.y);
            let habitat = tables.iter().find(|t| t.id == s.resource_type).unwrap().habitat.as_ref().unwrap();
            let r = (s.x - CENTER_X).stable_hypot(s.y - CENTER_Y) - RURAL_OUTER_RADIUS;
            assert!(habitat.suits(r, h, slope), "{at} isn't in its habitat");
            assert!(slope <= CLIFF_SLOPE, "{at} is on a cliff");
            assert!(land.reached((s.x, s.y)), "{at} can't be reached");
            let road = road_clearance(terrain, s.x, s.y);
            assert!(road >= MOVE_CLEAR, "{at} is {road:.1} from a road or trail");
            let water = land.water_clearance((s.x, s.y));
            let clear = if s.resource_type == "clay_bank" { BANK_CLEAR } else { MOVE_CLEAR };
            assert!(water >= clear, "{at} is {water:.1} from the water");
            match s.resource_type.as_str() {
                "iron_vein" => assert!(h >= 300.0 && slope >= 0.4, "{at} isn't in the ranges"),
                "old_growth" => assert!(h <= 250.0 && slope <= 0.4, "{at} isn't on a valley floor"),
                "copper_vein" | "tin_vein" => assert!((150.0..=500.0).contains(&h), "{at} isn't on the flanks"),
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
        // Reeds (#225): in the water, with dry ground a walker can reach
        // within gathering range of them.
        let reeds: Vec<&&NodeSeed> = wild.iter().filter(|s| s.resource_type == "reed_bed").collect();
        assert!(!reeds.is_empty());
        for s in reeds {
            let at = format!("reeds at ({}, {})", s.x, s.y);
            assert!(land.in_the_water((s.x, s.y)), "{at} aren't in the water");
            assert!(road_clearance(terrain, s.x, s.y) >= MOVE_CLEAR, "{at} are by a road or trail");
            let reach = capitol_protocol::rules::EXTRACTION_RANGE - 1.0;
            let bank = (0..72).any(|k| {
                let a = k as f64 * std::f64::consts::TAU / 72.0;
                (2..=(reach as i32)).any(|d| {
                    let p = (s.x + d as f64 * a.stable_cos(), s.y + d as f64 * a.stable_sin());
                    !land.in_the_water(p) && land.reached(p)
                })
            });
            assert!(bank, "{at} have no bank to gather them from");
        }
    }

    /// #225: copper and tin keep to different massifs, so bronze needs both
    /// ranges, or a trade.
    #[test]
    fn copper_and_tin_grow_on_different_massifs() {
        use capitol_protocol::land::massif_at;
        let terrain = TerrainConfig::default_world();
        let plan = default_plan();
        let massifs = |ore: &str| -> std::collections::BTreeSet<u32> {
            plan.iter().filter(|s| s.resource_type == ore).filter_map(|s| massif_at(s.x, s.y, &terrain.land)).collect()
        };
        let (copper, tin) = (massifs("copper_vein"), massifs("tin_vein"));
        assert!(copper.len() >= 3 && tin.len() >= 3, "copper on {copper:?}, tin on {tin:?}");
        assert!(copper.iter().all(|m| m % 2 == 0), "copper on {copper:?}");
        assert!(tin.iter().all(|m| m % 2 == 1), "tin on {tin:?}");
    }

    /// W6 (#199): the base quality rises from the rural edge to the ranges,
    /// for each wild resource found at both, and for the wilds as a whole.
    #[test]
    fn wild_quality_rises_into_the_ranges() {
        let plan = default_plan();
        let r = |s: &NodeSeed| (s.x - CENTER_X).stable_hypot(s.y - CENTER_Y);
        let mean = |v: &[&NodeSeed]| v.iter().map(|s| s.base_quality as f64).sum::<f64>() / v.len() as f64;
        let wild: Vec<&NodeSeed> = plan.iter().filter(|s| Zone::from_position(s.x, s.y) == Zone::Wilderness).collect();
        let edge: Vec<&NodeSeed> = wild.iter().copied().filter(|s| r(s) < RURAL_OUTER_RADIUS + 500.0).collect();
        let ranges: Vec<&NodeSeed> = wild.iter().copied().filter(|s| r(s) > RURAL_OUTER_RADIUS + 1200.0).collect();
        let (near, far) = (mean(&edge), mean(&ranges));
        assert!(far > near + 5.0, "the wilds: {near:.1} by the rural edge, {far:.1} in the ranges");
        for kind in ["old_growth", "fallen_branches", "rock_outcrop", "clay_bank"] {
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
        let terrain = TerrainConfig::default_world();
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
