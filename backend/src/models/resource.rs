use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
    pub required_tool: Option<String>,
    pub extraction_duration_ticks: i32,
    pub regeneration_ticks: i32,
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
        SPAWN_X, SPAWN_Y,
    };

    use capitol_protocol::terrain::quad_contains;
    use uuid::Uuid;

    use crate::engine::node_placement::{place_nodes, KEEP_CLEAR};
    use crate::models::terrain::{point_to_segment_distance, TerrainConfig, WorldGenParams};

    const WORLD_SCALE_MIGRATION: &str =
        include_str!("../../migrations/20260212_001_world_scale.sql");

    /// Moves for seeded nodes, in the order the migrations run: the ones a later
    /// street plan ran over (#62), the ones the deeper suburb moved (#117),
    /// then the ones the Neighbourhood suburb's streets ran over (#112).
    const NODE_MOVES_MIGRATIONS: [&str; 3] = [
        include_str!("../../migrations/20260215_001_suburban_street_nodes.sql"),
        include_str!("../../migrations/20261018_001_deeper_suburb_nodes.sql"),
        include_str!("../../migrations/20261019_001_neighbourhood_suburb.sql"),
    ];

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
    /// later migrations leave them.
    fn seeded_nodes() -> Vec<(f64, f64, i32)> {
        let migrations: Vec<Vec<Vec<f64>>> = NODE_MOVES_MIGRATIONS.iter().map(|sql| value_rows(sql)).collect();
        value_rows(WORLD_SCALE_MIGRATION)
            .into_iter()
            .filter(|row| row.len() == 3)
            .map(|row| {
                let (x, y) = migrations.iter().fold((row[0], row[1]), |(x, y), moves| {
                    moves.iter().find(|m| m[0] == x && m[1] == y).map_or((x, y), |m| (m[2], m[3]))
                });
                (x, y, row[2] as i32)
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
}
