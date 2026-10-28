//! Keeps the resource nodes off the live terrain's roads and water (#70), and off the landmarks (E3.4).
//! The water is the moat and canals, and the wild water: rivers, streams and
//! lakes, which since #222 run on into the rural ring.
//!
//! The nodes are seeded as fixed positions in migrations, chosen against the
//! default world. The admin terrain tools can regenerate the world with other
//! params, and a node the new roads run over can't be told from a node on the
//! verge. So [`fit_to_terrain`] runs whenever the terrain is set: on startup,
//! on an admin's regenerate, and on the commit that saves it.
//!
//! A node is **in the way** if it stands within [`KEEP_CLEAR`] of a road's
//! edge or of the water's kerb. One that is gets moved to the
//! nearest spot in the same zone that is [`MOVE_CLEAR`] from every road and
//! the water, [`LOT_MARGIN`] off every lot and [`NODE_SPACING`] from every
//! other node. The gap between the two clearances is hysteresis: a moved node
//! isn't left at the limit, where the next small terrain tweak would move it
//! again. A node that isn't in the way never moves, so on an unchanged
//! terrain this does nothing.
//!
//! Nodes that grow in the water (the `shallows` cluster: reeds, #225) aren't
//! fitted: they stand in it on purpose, and an admin's regenerate re-plans
//! them with the rest of the wilds.
//!
//! Standing on a lot doesn't put a node in the way. Some seeds stand on lots
//! in the default world (grass in a rural field, mostly), and grass on a lot
//! belongs to the lot: once claimed, only those the plot's `station_use_level`
//! admits may gather it (`container_access::may_gather`, #102). A moved node
//! does keep off lots, as the #62 migration's moves did.

use capitol_protocol::fmath::StableMath;
use std::collections::HashMap;

use capitol_protocol::rules::resources::Cluster;
use capitol_protocol::rules::{Zone, CENTER_X, CENTER_Y};
use capitol_protocol::terrain::{quad_contains, Lake, Landmark, TerrainPlot};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::lot_clearance::WATER_KERB;
use crate::models::terrain::{point_to_segment_distance, TerrainConfig};
use crate::models::{ResourceNodeState, ResourceType};

/// A node closer than this to a road's edge or the water's kerb is in the way.
pub const KEEP_CLEAR: f64 = 15.0;

/// How far a moved node stands from a road's edge and the water's kerb. The
/// same 30 the #62 migration used.
pub const MOVE_CLEAR: f64 = 30.0;

/// How far a moved node stands off the edge of any lot.
pub const LOT_MARGIN: f64 = 10.0;

/// How far a moved node stands from every other node.
pub const NODE_SPACING: f64 = 30.0;

/// How far from where it stood a node is looked for a new spot. 400 until
/// Plots v2 (#459), whose suburb, of lots about 1.4 times as wide, left a
/// grass node in a regenerated world with doubled streets no spot within it.
const SEARCH_RADIUS: f64 = 600.0;

/// The step between the rings searched, and between spots along each ring.
const SEARCH_STEP: f64 = 5.0;

#[derive(Debug, Clone, PartialEq)]
pub struct NodeMove {
    pub id: Uuid,
    pub from: (f64, f64),
    pub to: (f64, f64),
}

#[derive(Debug, Default)]
pub struct PlacementReport {
    pub moves: Vec<NodeMove>,
    /// Nodes in the way with no clear spot within [`SEARCH_RADIUS`]. They stay put.
    pub stuck: Vec<Uuid>,
}

/// The roads, water and lots near one node, which are all its search can reach.
struct Surroundings<'a> {
    /// Each road segment with its half width.
    segments: Vec<((f64, f64), (f64, f64), f64)>,
    /// Each water ring as (inner, outer) radius, widened by the kerb.
    bands: Vec<(f64, f64)>,
    /// Each stretch of river or stream with its half width.
    courses: Vec<((f64, f64), (f64, f64), f64)>,
    lakes: Vec<&'a Lake>,
    lots: Vec<&'a TerrainPlot>,
    /// The landmarks' footprints (the Exchange hall): a node stays off them
    /// as it does off a road.
    landmarks: Vec<&'a Landmark>,
}

impl<'a> Surroundings<'a> {
    fn around(terrain: &'a TerrainConfig, at: (f64, f64)) -> Self {
        let reach = SEARCH_RADIUS + MOVE_CLEAR;
        let near_box = |min: (f64, f64), max: (f64, f64), pad: f64| {
            at.0 >= min.0 - pad && at.0 <= max.0 + pad && at.1 >= min.1 - pad && at.1 <= max.1 + pad
        };
        let segments = terrain
            .roads
            .iter()
            .flat_map(|road| {
                let half = road.width / 2.0;
                road.points.windows(2).map(move |w| (w[0], w[1], half))
            })
            .filter(|&(a, b, half)| {
                near_box((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)), reach + half)
            })
            .collect();
        let bands = terrain
            .water_rings
            .iter()
            .map(|ring| (ring.inner_radius() - WATER_KERB, ring.outer_radius() + WATER_KERB))
            .collect();
        let lots = terrain
            .terrain_plots
            .iter()
            .filter(|lot| {
                let xs = lot.corners.map(|c| c.0);
                let ys = lot.corners.map(|c| c.1);
                let min = (xs.iter().copied().fold(f64::MAX, f64::min), ys.iter().copied().fold(f64::MAX, f64::min));
                let max = (xs.iter().copied().fold(f64::MIN, f64::max), ys.iter().copied().fold(f64::MIN, f64::max));
                near_box(min, max, reach + LOT_MARGIN)
            })
            .collect();
        let courses = terrain
            .rivers
            .iter()
            .chain(&terrain.streams)
            .flat_map(|c| c.points.windows(2).map(|w| ((w[0].x, w[0].y), (w[1].x, w[1].y), w[0].width.max(w[1].width) / 2.0)))
            .filter(|&(a, b, half)| {
                near_box((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)), reach + half)
            })
            .collect();
        let lakes = terrain
            .lakes
            .iter()
            .filter(|lake| {
                let points = lake.rings.iter().flatten();
                let min = points.clone().fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
                let max = points.fold((f64::MIN, f64::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
                near_box(min, max, reach)
            })
            .collect();
        let landmarks = terrain.landmarks.iter().collect();
        Self { segments, bands, courses, lakes, lots, landmarks }
    }

    /// How far a point is from the nearest road edge, water kerb, bank, shore
    /// or landmark; negative inside a road or the water, zero on a landmark.
    fn clearance(&self, (x, y): (f64, f64)) -> f64 {
        let r = (x - CENTER_X).stable_hypot(y - CENTER_Y);
        let water = self
            .bands
            .iter()
            .map(|&(inner, outer)| (inner - r).max(r - outer))
            .fold(f64::MAX, f64::min);
        let lakes = self.lakes.iter().map(|lake| {
            let shore = lake
                .rings
                .iter()
                .flat_map(|ring| (0..ring.len()).map(move |i| (ring[i], ring[(i + 1) % ring.len()])))
                .map(|(a, b)| point_to_segment_distance(x, y, a.0, a.1, b.0, b.1))
                .fold(f64::MAX, f64::min);
            if lake.contains(x, y) { -shore } else { shore }
        });
        self.segments
            .iter()
            .chain(&self.courses)
            .map(|&(a, b, half)| point_to_segment_distance(x, y, a.0, a.1, b.0, b.1) - half)
            .chain(lakes)
            .fold(water, f64::min)
            .min(self.landmarks.iter().map(|l| l.distance_to(x, y)).fold(f64::MAX, f64::min))
    }

    /// How far a point is outside the nearest lot; zero or less on one.
    fn lot_distance(&self, (x, y): (f64, f64)) -> f64 {
        self.lots
            .iter()
            .map(|lot| {
                if quad_contains(&lot.corners, x, y) {
                    return 0.0;
                }
                (0..4)
                    .map(|i| {
                        let (a, b) = (lot.corners[i], lot.corners[(i + 1) % 4]);
                        point_to_segment_distance(x, y, a.0, a.1, b.0, b.1)
                    })
                    .fold(f64::MAX, f64::min)
            })
            .fold(f64::MAX, f64::min)
    }

    fn in_the_way(&self, at: (f64, f64)) -> bool {
        self.clearance(at) < KEEP_CLEAR
    }
}

/// Where each node in `nodes` should stand on `terrain`. Only nodes in the way
/// get a move, and nodes are taken in id order, so the result is the same
/// every time for the same terrain and nodes.
pub fn place_nodes(terrain: &TerrainConfig, nodes: &[(Uuid, (f64, f64))]) -> PlacementReport {
    let mut positions: Vec<(Uuid, (f64, f64))> = nodes.to_vec();
    positions.sort_by_key(|&(id, _)| id);
    let mut report = PlacementReport::default();

    for i in 0..positions.len() {
        let (id, from) = positions[i];
        let around = Surroundings::around(terrain, from);
        if !around.in_the_way(from) {
            continue;
        }
        let zone = Zone::from_position(from.0, from.1);
        let spaced = |at: (f64, f64)| {
            positions
                .iter()
                .enumerate()
                .all(|(j, &(_, p))| j == i || (p.0 - at.0).stable_hypot(p.1 - at.1) >= NODE_SPACING)
        };
        let fits = |at: (f64, f64)| {
            Zone::from_position(at.0, at.1) == zone
                && around.clearance(at) >= MOVE_CLEAR
                && around.lot_distance(at) >= LOT_MARGIN
                && spaced(at)
        };
        match nearest_spot(from, fits) {
            Some(to) => {
                positions[i].1 = to;
                report.moves.push(NodeMove { id, from, to });
            }
            None => report.stuck.push(id),
        }
    }
    report
}

/// The first spot that fits, searching rings outward from `from`.
fn nearest_spot(from: (f64, f64), fits: impl Fn((f64, f64)) -> bool) -> Option<(f64, f64)> {
    let rings = (SEARCH_RADIUS / SEARCH_STEP) as usize;
    (1..=rings).find_map(|k| {
        let r = k as f64 * SEARCH_STEP;
        let spots = ((std::f64::consts::TAU * r) / SEARCH_STEP).ceil() as usize;
        (0..spots).find_map(|s| {
            let angle = std::f64::consts::TAU * s as f64 / spots as f64;
            let at = (round_tenth(from.0 + r * angle.stable_cos()), round_tenth(from.1 + r * angle.stable_sin()));
            fits(at).then_some(at)
        })
    })
}

/// Positions are kept to a tenth, like the seeds.
fn round_tenth(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Moves the game's nodes off `terrain`'s roads and water, in memory, all
/// but those that grow in the water.
/// The caller persists the moves with [`save_moves`] when the terrain is saved.
pub fn fit_to_terrain(
    terrain: &TerrainConfig,
    nodes: &mut HashMap<Uuid, ResourceNodeState>,
    resource_types: &HashMap<String, ResourceType>,
) -> PlacementReport {
    let wet = |t: &str| resource_types.get(t).is_some_and(|r| r.cluster == Cluster::Shallows.as_str());
    let positions: Vec<(Uuid, (f64, f64))> =
        nodes.values().filter(|n| !wet(&n.resource_type)).map(|n| (n.id, (n.x, n.y))).collect();
    let report = place_nodes(terrain, &positions);
    for m in &report.moves {
        if let Some(node) = nodes.get_mut(&m.id) {
            node.x = m.to.0;
            node.y = m.to.1;
        }
    }
    for m in &report.moves {
        tracing::info!(
            "Resource node {} moved off the terrain from ({:.1}, {:.1}) to ({:.1}, {:.1})",
            m.id, m.from.0, m.from.1, m.to.0, m.to.1
        );
    }
    for id in &report.stuck {
        tracing::warn!("Resource node {} is in the terrain's way and has no clear spot nearby; left where it is", id);
    }
    report
}

/// Writes these node positions to `resource_nodes`.
pub async fn save_positions(db: &PgPool, positions: &[(Uuid, (f64, f64))]) -> Result<u64, sqlx::Error> {
    if positions.is_empty() {
        return Ok(0);
    }
    let ids: Vec<Uuid> = positions.iter().map(|p| p.0).collect();
    let xs: Vec<f64> = positions.iter().map(|p| p.1 .0).collect();
    let ys: Vec<f64> = positions.iter().map(|p| p.1 .1).collect();
    let result = sqlx::query(
        "UPDATE resource_nodes AS n SET position_x = v.x, position_y = v.y
         FROM UNNEST($1::uuid[], $2::float8[], $3::float8[]) AS v(id, x, y)
         WHERE n.id = v.id AND (n.position_x <> v.x OR n.position_y <> v.y)",
    )
    .bind(&ids)
    .bind(&xs)
    .bind(&ys)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

/// Writes a placement's moves to `resource_nodes`.
pub async fn save_moves(db: &PgPool, moves: &[NodeMove]) -> Result<u64, sqlx::Error> {
    let positions: Vec<(Uuid, (f64, f64))> = moves.iter().map(|m| (m.id, m.to)).collect();
    save_positions(db, &positions).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::terrain::{RoadSegment, RoadType};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// A rural spot, well clear of the default terrain's water.
    const AT: (f64, f64) = (CENTER_X + 3000.0, CENTER_Y + 500.0);

    fn empty_terrain() -> TerrainConfig {
        let mut terrain = TerrainConfig::get_default();
        terrain.roads.clear();
        terrain.terrain_plots.clear();
        terrain
    }

    fn road_through(at: (f64, f64), width: f64) -> RoadSegment {
        RoadSegment {
            id: "test_road".into(),
            road_type: RoadType::Suburban,
            points: vec![(at.0 - 500.0, at.1), (at.0 + 500.0, at.1)],
            width,
            name: None,
        }
    }

    #[test]
    fn a_node_clear_of_everything_stays_put() {
        let report = place_nodes(&empty_terrain(), &[(id(1), AT)]);
        assert!(report.moves.is_empty() && report.stuck.is_empty());
    }

    #[test]
    fn a_node_on_a_road_moves_just_far_enough_off_it() {
        let mut terrain = empty_terrain();
        terrain.roads.push(road_through(AT, 20.0));
        let report = place_nodes(&terrain, &[(id(1), AT)]);
        assert_eq!(report.moves.len(), 1);
        let to = report.moves[0].to;
        let off = (to.1 - AT.1).abs() - 10.0;
        assert!(off >= MOVE_CLEAR, "{off:.1} from the road edge");
        assert!(off < MOVE_CLEAR + SEARCH_STEP, "moved {off:.1} off the road, further than it needed");
    }

    #[test]
    fn a_node_beside_a_road_but_clear_of_it_stays_put() {
        let mut terrain = empty_terrain();
        terrain.roads.push(road_through((AT.0, AT.1 - 10.0 - KEEP_CLEAR - 1.0), 20.0));
        assert!(place_nodes(&terrain, &[(id(1), AT)]).moves.is_empty());
    }

    fn lot_around(at: (f64, f64), half: f64) -> TerrainPlot {
        let (x, y) = at;
        TerrainPlot {
            id: "lot".into(),
            corners: [(x - half, y - half), (x + half, y - half), (x + half, y + half), (x - half, y + half)],
            street_facing_edge: (0, 1),
            area: 4.0 * half * half,
            adjacent_street_id: String::new(),
            zone: "rural".into(),
        }
    }

    #[test]
    fn a_node_on_a_lot_stays_put() {
        let mut terrain = empty_terrain();
        terrain.terrain_plots.push(lot_around(AT, 50.0));
        assert!(place_nodes(&terrain, &[(id(1), AT)]).moves.is_empty());
    }

    #[test]
    fn a_moved_node_keeps_off_lots_and_in_its_zone() {
        let mut terrain = empty_terrain();
        terrain.roads.push(road_through(AT, 20.0));
        // A lot just across each verge, where the nearest clear spots would be.
        terrain.terrain_plots.push(lot_around((AT.0, AT.1 + 60.0), 30.0));
        terrain.terrain_plots.push(lot_around((AT.0, AT.1 - 60.0), 30.0));
        let report = place_nodes(&terrain, &[(id(1), AT)]);
        let to = report.moves[0].to;
        for lot in &terrain.terrain_plots {
            assert!(!quad_contains(&lot.corners, to.0, to.1), "moved onto a lot at {to:?}");
        }
        assert_eq!(Zone::from_position(to.0, to.1), Zone::from_position(AT.0, AT.1));
    }

    #[test]
    fn moved_nodes_keep_their_spacing() {
        let mut terrain = empty_terrain();
        terrain.roads.push(road_through(AT, 20.0));
        let nodes: Vec<_> = (0..4).map(|i| (id(i), (AT.0 + i as f64, AT.1))).collect();
        let report = place_nodes(&terrain, &nodes);
        assert_eq!(report.moves.len(), 4);
        for a in &report.moves {
            for b in report.moves.iter().filter(|b| b.id != a.id) {
                let gap = (a.to.0 - b.to.0).stable_hypot(a.to.1 - b.to.1);
                assert!(gap >= NODE_SPACING, "{} and {} are {gap:.1} apart", a.id, b.id);
            }
        }
    }

    #[test]
    fn a_node_in_the_water_moves_out_of_it() {
        let terrain = empty_terrain();
        let ring = &terrain.water_rings[0];
        let at = (CENTER_X + ring.radius, CENTER_Y);
        let report = place_nodes(&terrain, &[(id(1), at)]);
        if let Some(m) = report.moves.first() {
            let r = (m.to.0 - CENTER_X).stable_hypot(m.to.1 - CENTER_Y);
            assert!(r < ring.inner_radius() - WATER_KERB || r > ring.outer_radius() + WATER_KERB);
        } else {
            // A narrow zone ring can hold no clear spot at all.
            assert_eq!(report.stuck, vec![id(1)]);
        }
    }

    #[test]
    fn placement_is_the_same_every_time() {
        let mut terrain = empty_terrain();
        terrain.roads.push(road_through(AT, 20.0));
        let nodes: Vec<_> = (0..4).map(|i| (id(i), (AT.0 + 3.0 * i as f64, AT.1))).collect();
        let mut reversed = nodes.clone();
        reversed.reverse();
        assert_eq!(place_nodes(&terrain, &nodes).moves, place_nodes(&terrain, &reversed).moves);
    }
}
