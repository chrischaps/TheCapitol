//! The roads (P3.4): a flat ribbon down each polyline in `/terrain`, as
//! wide as the data says, in the 2D clients' road colours.
//!
//! - **Joins are mitred**, so a curve stays one smooth band. Where a mitre
//!   would reach past `MITER_LIMIT` half-widths (a turn sharper than 120
//!   degrees), or its inside corner would run back past the middle of a
//!   short segment, the join is rounded instead, as the 2D clients draw
//!   every join. The inside of a rounded join is two segments overlapping,
//!   which is invisible in one flat colour.
//! - **Ends are round** where a road stops on its own, as the 2D clients
//!   draw them. Where a road runs into another, it ends flat, so a wide
//!   arterial meeting a ring road at a T doesn't bulge out past it.
//! - **Roads stop at the water.** Every road ends flat at the kerb's land
//!   face (`water::kerbed`), bridges included: the data's 60-unit
//!   arterials meet 25-unit bridges, and P3.5's deck spans the gap between
//!   the two road ends.
//! - **Layers.** Each type has its own height (`layer::road`): trail <
//!   suburban < grid < arterial, so where two meet, the greater road is on
//!   top. Two roads of one type overlap at the same height in the same
//!   colour, so where they cross nothing shows.
//! - **One mesh per type per sector** (`sector`), so each culls on its own.
//!   Segments are split where they cross a sector's edge, and every
//!   segment's quad goes to the sector it lies in. The split points are on
//!   straight lines, and the corners either side of every join are computed
//!   once and shared, so no crack opens between meshes.

use std::collections::HashMap;
use std::f64::consts::PI;

use crate::coords::{layer, to_world};
use crate::world::geom::*;
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{self, hex};
use crate::world::sector::{self, Sector};
use crate::world::water;
use capitol_protocol::terrain::{RoadSegment, RoadType, WaterRing};
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;

/// Every road type, lowest layer first.
pub const TYPES: [RoadType; 4] = [
    RoadType::Trail,
    RoadType::Suburban,
    RoadType::Grid,
    RoadType::Arterial,
];

/// Triangles in a round end cap, a half circle.
const CAP_SEGMENTS: usize = 10;
/// How far a mitred corner may reach, in half-widths, before the join is
/// rounded instead. 2 allows turns up to 120 degrees.
const MITER_LIMIT: f64 = 2.0;
/// Points closer than this, in server units (5 cm), are one point; the
/// suburban data repeats some. A cut closer than this to a point is moved
/// onto it.
const MIN_STEP: f64 = 0.5;

/// How a stretch of road ends.
#[derive(Clone, Copy, PartialEq, Debug)]
enum End {
    /// A half circle, as the 2D clients draw a road's end.
    Round,
    /// Square across, where the road was cut at the water.
    Flat,
}

/// A run of road to draw as one ribbon: a whole road, or the part of one
/// between its ends and the water.
#[derive(Debug)]
struct Stretch {
    points: Vec<P>,
    /// A loop: the last point joins the first, and there are no ends.
    closed: bool,
    start: End,
    end: End,
}

/// A road's polyline cleaned, cut where it meets the `keep_out` bands of
/// radii, and split at every sector edge. `ends` is how the road itself
/// starts and ends; a cut always ends flat.
fn stretches(points: &[(f64, f64)], keep_out: &[(f64, f64)], ends: [End; 2]) -> Vec<Stretch> {
    let mut pts: Vec<P> = Vec::with_capacity(points.len());
    for &(x, y) in points {
        if pts.last().is_none_or(|&q| dist(q, [x, y]) >= MIN_STEP) {
            pts.push([x, y]);
        }
    }
    let closed = pts.len() >= 4 && dist(pts[0], pts[pts.len() - 1]) < MIN_STEP;
    if closed {
        pts.pop();
    }
    let n = pts.len();
    if n < 2 {
        return Vec::new();
    }

    // Each segment cut into pieces at the water's edges and the sector
    // edges: (from, to, dry).
    let edges: Vec<f64> = keep_out.iter().flat_map(|&(lo, hi)| [lo, hi]).collect();
    let mut pieces: Vec<(P, P, bool)> = Vec::new();
    for i in 0..if closed { n } else { n - 1 } {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let len = dist(a, b);
        let mut ts = sector::circle_crossings(a, b, &edges);
        ts.extend(sector::crossings(a, b));
        ts.retain(|&t| t * len >= MIN_STEP && (1.0 - t) * len >= MIN_STEP);
        ts.extend([0.0, 1.0]);
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|later, kept| (*later - *kept) * len < MIN_STEP);
        for w in ts.windows(2) {
            let r = radius(lerp(a, b, (w[0] + w[1]) * 0.5));
            let dry = !keep_out.iter().any(|&(lo, hi)| r > lo && r < hi);
            pieces.push((lerp(a, b, w[0]), lerp(a, b, w[1]), dry));
        }
    }

    if closed && pieces.iter().all(|p| p.2) {
        return vec![Stretch {
            points: pieces.iter().map(|p| p.0).collect(),
            closed: true,
            start: End::Flat,
            end: End::Flat,
        }];
    }
    if closed {
        // A loop that dips into the water: start just after a wet piece,
        // so every stretch is open and ends flat at the water.
        let wet = pieces.iter().position(|p| !p.2).expect("some piece is wet");
        pieces.rotate_left(wet + 1);
    }
    let mut out = Vec::new();
    let mut run: Vec<P> = Vec::new();
    let mut start = if closed { End::Flat } else { ends[0] };
    for (p, q, dry) in pieces {
        if dry {
            if run.is_empty() {
                run.push(p);
            }
            run.push(q);
        } else {
            if run.len() >= 2 {
                out.push(Stretch {
                    points: std::mem::take(&mut run),
                    closed: false,
                    start,
                    end: End::Flat,
                });
            }
            run.clear();
            start = End::Flat;
        }
    }
    if run.len() >= 2 {
        out.push(Stretch {
            points: run,
            closed: false,
            start,
            end: ends[1],
        });
    }
    out
}

/// How road `i` should start and end: round where it stops on its own,
/// flat where it runs into another road, so a wide road meeting a narrower
/// one at a T doesn't bulge out past it.
fn natural_ends(roads: &[RoadSegment], i: usize) -> [End; 2] {
    let on_another = |&(x, y): &(f64, f64)| {
        roads.iter().enumerate().any(|(k, other)| {
            k != i
                && other.points.windows(2).any(|w| {
                    let (a, b) = ([w[0].0, w[0].1], [w[1].0, w[1].1]);
                    distance_to_segment([x, y], a, b) < other.width * 0.5
                })
        })
    };
    let points = &roads[i].points;
    let end = |p: Option<&(f64, f64)>| match p {
        Some(p) if on_another(p) => End::Flat,
        _ => End::Round,
    };
    [end(points.first()), end(points.last())]
}

/// The ribbon's (left, right) edge points at a place on the road.
type Pair = [P; 2];

/// Draws `s` as a ribbon `half` server units either side of its line, at
/// height `y`, into the mesh of each sector it passes through.
fn ribbon(out: &mut HashMap<Sector, MeshBuf>, s: &Stretch, half: f64, y: f32, colour: [f32; 4]) {
    let pts = &s.points;
    let n = pts.len();
    let segs = if s.closed { n } else { n - 1 };
    let seg = |j: usize| (pts[j], pts[(j + 1) % n]);
    let dir: Vec<P> = (0..segs).map(|j| unit(sub(seg(j).1, seg(j).0))).collect();
    let len: Vec<f64> = (0..segs).map(|j| dist(seg(j).0, seg(j).1)).collect();
    let home: Vec<Sector> = (0..segs)
        .map(|j| Sector::of(lerp(seg(j).0, seg(j).1, 0.5)))
        .collect();
    let lift = |p: P| {
        let [x, z] = to_world(p[0], p[1]);
        [x, y, z]
    };
    let pair = |p: P, off: P| -> Pair { [add(p, off), sub(p, off)] };
    // A fan round `p` from `p + from`, turning through `radians`.
    let mut fan = |sector: Sector, p: P, from: P, radians: f64, steps: usize| {
        let rim: Vec<[f32; 3]> = (0..=steps)
            .map(|k| lift(add(p, rot(from, radians * k as f64 / steps as f64))))
            .collect();
        out.entry(sector)
            .or_default()
            .fan(lift(p), &rim, UP, colour);
    };

    // Where each segment's quad starts and ends.
    let mut starts: Vec<Pair> = vec![[[0.0; 2]; 2]; segs];
    let mut ends: Vec<Pair> = vec![[[0.0; 2]; 2]; segs];
    for (i, &p) in pts.iter().enumerate() {
        let prev = match i {
            0 if s.closed => Some(segs - 1),
            0 => None,
            _ => Some(i - 1),
        };
        let next = (i < segs).then_some(i);
        match (prev, next) {
            (None, Some(j)) => {
                let off = scale(left(dir[j]), half);
                starts[j] = pair(p, off);
                if s.start == End::Round {
                    // From the left edge, round behind the start to the right.
                    fan(home[j], p, off, PI, CAP_SEGMENTS);
                }
            }
            (Some(j), None) => {
                let off = scale(left(dir[j]), half);
                ends[j] = pair(p, off);
                if s.end == End::Round {
                    // From the right edge, round past the end to the left.
                    fan(home[j], p, scale(off, -1.0), PI, CAP_SEGMENTS);
                }
            }
            (Some(a), Some(b)) => {
                let (na, nb) = (left(dir[a]), left(dir[b]));
                // Signed turn: positive toward the left.
                let turn =
                    (dir[a][0] * dir[b][1] - dir[a][1] * dir[b][0]).atan2(dot(dir[a], dir[b]));
                let inside = half * (turn.abs() * 0.5).tan();
                let mitred = (turn.abs() * 0.5).cos() >= 1.0 / MITER_LIMIT
                    && inside <= 0.5 * len[a].min(len[b]);
                if mitred {
                    let m = unit(add(na, nb));
                    let corner = pair(p, scale(m, half / dot(m, nb)));
                    ends[a] = corner;
                    starts[b] = corner;
                } else {
                    ends[a] = pair(p, scale(na, half));
                    starts[b] = pair(p, scale(nb, half));
                    // Round the outside of the corner, which is on the
                    // right when the road turns left.
                    let outside = scale(na, if turn > 0.0 { -half } else { half });
                    let steps = (turn.abs() * CAP_SEGMENTS as f64 / PI).ceil().max(1.0) as usize;
                    fan(home[b], p, outside, turn, steps);
                }
            }
            (None, None) => unreachable!("a stretch has two points or more"),
        }
    }
    for j in 0..segs {
        let ([l0, r0], [l1, r1]) = (starts[j], ends[j]);
        out.entry(home[j])
            .or_default()
            .quad([l0, r0, r1, l1].map(lift), UP, colour);
    }
}

/// The road meshes for this terrain, by type and sector. Only the ones
/// with something in them are present.
pub fn build(roads: &[RoadSegment], rings: &[WaterRing]) -> HashMap<(RoadType, Sector), MeshBuf> {
    let keep_out = water::kerbed(rings);
    let mut by_type: HashMap<RoadType, HashMap<Sector, MeshBuf>> = HashMap::new();
    for (i, road) in roads.iter().enumerate() {
        let sink = by_type.entry(road.road_type).or_default();
        let (y, colour) = (
            layer::road(road.road_type),
            hex(palette::road(road.road_type)),
        );
        for s in stretches(&road.points, &keep_out, natural_ends(roads, i)) {
            ribbon(sink, &s, road.width * 0.5, y, colour);
        }
    }
    by_type
        .into_iter()
        .flat_map(|(t, sectors)| sectors.into_iter().map(move |(s, m)| ((t, s), m)))
        .collect()
}

/// The runtime mesh (and entity) name for one type's roads in one sector:
/// `roads_suburban_r2w05`.
pub fn mesh_name(road_type: RoadType, sector: Sector) -> String {
    let t = match road_type {
        RoadType::Arterial => "arterial",
        RoadType::Grid => "grid",
        RoadType::Suburban => "suburban",
        RoadType::Trail => "trail",
    };
    format!("roads_{t}_{}", sector.tag())
}

/// Asphalt has a little sheen under the low sun; a trail has none.
fn roughness(road_type: RoadType) -> f32 {
    match road_type {
        RoadType::Arterial => 0.7,
        RoadType::Grid => 0.75,
        RoadType::Suburban => 0.82,
        RoadType::Trail => 0.95,
    }
}

/// Uploads every road mesh (once per terrain), clearing any that this
/// terrain leaves empty, and returns the names of those with something in
/// them, for the caller to give each an entity.
pub fn upload(gpu: &mut ExtensionGpu, roads: &[RoadSegment], rings: &[WaterRing]) -> Vec<String> {
    let mut meshes = build(roads, rings);
    let mut drawn = Vec::new();
    let mut vertices = 0;
    for t in TYPES {
        for s in Sector::all() {
            let name = mesh_name(t, s);
            let mesh = meshes.remove(&(t, s)).unwrap_or_default();
            if !mesh.indices.is_empty() {
                vertices += mesh.vertices.len();
                drawn.push(name.clone());
            }
            mesh.upload_material(
                gpu,
                ImportedMaterial {
                    roughness: roughness(t),
                    ..ImportedMaterial::vertex_colored(&name)
                },
            );
        }
    }
    tracing::info!(
        "roads: {} roads in {} meshes, {vertices} vertices",
        roads.len(),
        drawn.len()
    );
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::to_server;
    use crate::world::mesh::facing;
    use crate::world::mesh::probe::{covered, triangles};
    use std::path::Path;

    fn fixture() -> capitol_protocol::terrain::TerrainResponse {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        capitol_core::terrain::read_terrain_file(&path).expect("fixture")
    }

    /// One ribbon for `points`, `half` either side, every sector merged.
    fn draw(points: &[(f64, f64)], half: f64) -> MeshBuf {
        let mut sectors = HashMap::new();
        for s in stretches(points, &[], [End::Round; 2]) {
            ribbon(&mut sectors, &s, half, 0.0, [1.0; 4]);
        }
        let mut m = MeshBuf::default();
        for (_, part) in sectors {
            m.append(part);
        }
        m
    }

    /// Server distance from `p` to the polyline.
    fn distance_to(points: &[(f64, f64)], p: P) -> f64 {
        points
            .windows(2)
            .map(|w| distance_to_segment(p, [w[0].0, w[0].1], [w[1].0, w[1].1]))
            .fold(f64::INFINITY, f64::min)
    }

    fn assert_faces_up(m: &MeshBuf) {
        for v in &m.vertices {
            assert_eq!(v.normal, UP);
            assert!(v.position.iter().all(|c| c.is_finite()), "{:?}", v.position);
        }
        for t in triangles(m) {
            assert!(
                facing(t[0].position, t[1].position, t[2].position, UP),
                "{:?}",
                t.map(|v| v.position)
            );
        }
    }

    #[test]
    fn a_straight_road_is_one_quad_and_two_round_caps() {
        let m = draw(&[(6300.0, 6000.0), (6500.0, 6000.0)], 10.0);
        // A quad, then two half-circle fans, each a centre and a rim.
        assert_eq!(m.vertices.len(), 4 + 2 * (1 + CAP_SEGMENTS + 1));
        assert_eq!(m.indices.len() / 3, 2 + 2 * CAP_SEGMENTS);
        assert_faces_up(&m);
        // The caps reach a half-width past each end.
        assert!(covered(&m, to_world(6291.0, 6000.0)));
        assert!(!covered(&m, to_world(6289.0, 6000.0)));
        assert!(covered(&m, to_world(6509.0, 6000.0)));
    }

    #[test]
    fn a_gentle_curve_is_one_strip_of_mitred_quads() {
        let arc: Vec<(f64, f64)> = (0..=16)
            .map(|k| {
                let a = (k as f64 * 4.0).to_radians();
                (6000.0 + 500.0 * a.cos(), 6000.0 + 500.0 * a.sin())
            })
            .collect();
        let s = stretches(&arc, &[], [End::Round; 2]);
        let segments: usize = s.iter().map(|s| s.points.len() - 1).sum();
        let m = draw(&arc, 15.0);
        // Only quads and the two caps: no joins needed rounding.
        assert_eq!(m.indices.len() / 3, 2 * segments + 2 * CAP_SEGMENTS);
        assert_faces_up(&m);
    }

    #[test]
    fn no_cracks_at_any_join_and_nothing_past_the_mitre_limit() {
        let half = 12.0;
        let shapes: [&[(f64, f64)]; 6] = [
            // A right angle (mitred), crossing a sector edge.
            &[(6500.0, 6000.0), (6600.0, 6000.0), (6600.0, 6150.0)],
            // A hairpin (rounded).
            &[(6500.0, 6050.0), (6650.0, 6080.0), (6510.0, 6110.0)],
            // A dead reversal.
            &[(6500.0, 6050.0), (6620.0, 6050.0), (6540.0, 6050.0)],
            // A zigzag of short steps, each too short to mitre.
            &[
                (6500.0, 6000.0),
                (6506.0, 6012.0),
                (6512.0, 6000.0),
                (6518.0, 6012.0),
                (6524.0, 6000.0),
            ],
            // A curve that runs across a ring edge and two wedges.
            &[
                (6950.0, 6100.0),
                (6990.0, 6180.0),
                (7020.0, 6260.0),
                (7030.0, 6360.0),
                (7000.0, 6460.0),
            ],
            // Repeated points and a near repeat.
            &[
                (6500.0, 6000.0),
                (6500.0, 6000.0),
                (6560.0, 6030.0),
                (6560.1, 6030.0),
                (6600.0, 6100.0),
            ],
        ];
        for shape in shapes {
            let m = draw(shape, half);
            assert_faces_up(&m);
            let (lo_x, hi_x) = shape
                .iter()
                .fold((f64::MAX, f64::MIN), |(l, h), p| (l.min(p.0), h.max(p.0)));
            let (lo_y, hi_y) = shape
                .iter()
                .fold((f64::MAX, f64::MIN), |(l, h), p| (l.min(p.1), h.max(p.1)));
            let reach = half * MITER_LIMIT + 2.0;
            let steps = 60;
            for i in 0..=steps {
                for k in 0..=steps {
                    let p = [
                        lo_x - reach + (hi_x - lo_x + 2.0 * reach) * i as f64 / steps as f64,
                        lo_y - reach + (hi_y - lo_y + 2.0 * reach) * k as f64 / steps as f64,
                    ];
                    let d = distance_to(shape, p);
                    let here = covered(&m, to_world(p[0], p[1]));
                    if d < half * 0.97 {
                        assert!(here, "a crack at {p:?} in {shape:?}");
                    }
                    if d > half * MITER_LIMIT * 1.01 {
                        assert!(!here, "past the mitre at {p:?} in {shape:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_closed_ring_road_is_a_loop_with_no_ends() {
        let terrain = fixture();
        let ring = terrain.roads.iter().find(|r| r.id == "trade_ring").unwrap();
        let s = stretches(
            &ring.points,
            &water::kerbed(&terrain.water_rings),
            [End::Round; 2],
        );
        assert_eq!(s.len(), 1);
        assert!(s[0].closed);
        let mut sectors = HashMap::new();
        ribbon(&mut sectors, &s[0], ring.width * 0.5, 0.0, [1.0; 4]);
        let mut m = MeshBuf::default();
        sectors.into_values().for_each(|part| m.append(part));
        // Every segment a quad, every join mitred: no fans anywhere.
        assert_eq!(m.vertices.len(), 4 * s[0].points.len());
        for k in 0..360 {
            let a = (k as f64 + 0.5).to_radians();
            let p = to_world(6000.0 + 400.0 * a.cos(), 6000.0 + 400.0 * a.sin());
            assert!(covered(&m, p), "{k}°");
        }
    }

    #[test]
    fn a_road_that_runs_into_another_ends_flat_on_it() {
        let terrain = fixture();
        let meshes = build(&terrain.roads, &terrain.water_rings);
        let mut arterials = MeshBuf::default();
        for ((t, _), m) in meshes {
            if t == RoadType::Arterial {
                arterials.append(m);
            }
        }
        // East, arterial_inner_2 runs from the guild ring (r800) in to the
        // trade ring (r400, 30 wide). Its 60-wide end stops on the ring
        // rather than rounding out past the ring's inner edge.
        let ring_inside = 400.0 - 15.0;
        assert!(covered(
            &arterials,
            to_world(6000.0 + ring_inside + 2.0, 6000.0)
        ));
        assert!(!covered(
            &arterials,
            to_world(6000.0 + ring_inside - 2.0, 6000.0)
        ));
        // A road that stops on its own still gets a round end: the
        // arterials' far ends in the wilderness.
        let roads = &terrain.roads;
        let i = roads.iter().position(|r| r.id == "arterial_2").unwrap();
        assert_eq!(natural_ends(roads, i), [End::Flat, End::Round]);
    }

    #[test]
    fn every_road_in_the_fixture_is_drawn_flat_up_and_finite() {
        let terrain = fixture();
        let meshes = build(&terrain.roads, &terrain.water_rings);
        for t in TYPES {
            assert!(meshes.keys().any(|(mt, _)| *mt == t), "no {t:?} roads");
        }
        for ((t, _), m) in &meshes {
            assert_faces_up(m);
            assert!(m.vertices.iter().all(|v| v.position[1] == layer::road(*t)));
        }
    }

    #[test]
    fn no_road_runs_over_the_water() {
        let terrain = fixture();
        let meshes = build(&terrain.roads, &terrain.water_rings);
        let wet = |[x, z]: [f32; 2]| {
            let r = radius(to_server(x, z));
            terrain
                .water_rings
                .iter()
                .find(|w| r > w.inner_radius() && r < w.outer_radius())
        };
        for m in meshes.values() {
            for t in triangles(m) {
                let p = t.map(|v| [v.position[0], v.position[2]]);
                let mid = |a: [f32; 2], b: [f32; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                let probes = [
                    p[0],
                    p[1],
                    p[2],
                    mid(p[0], p[1]),
                    mid(p[1], p[2]),
                    mid(p[2], p[0]),
                    [
                        (p[0][0] + p[1][0] + p[2][0]) / 3.0,
                        (p[0][1] + p[1][1] + p[2][1]) / 3.0,
                    ],
                ];
                for q in probes {
                    assert!(
                        wet(q).is_none(),
                        "a road over {:?} at {q:?}",
                        wet(q).map(|w| &w.id)
                    );
                }
            }
        }
    }

    #[test]
    fn roads_come_up_to_every_bridge() {
        let terrain = fixture();
        let meshes = build(&terrain.roads, &terrain.water_rings);
        let mut all = MeshBuf::default();
        for (_, m) in meshes {
            all.append(m);
        }
        let kerb = (water::KERB / crate::coords::S) as f64;
        for ring in &terrain.water_rings {
            for bridge in &ring.bridges {
                let a = bridge.angle_degrees.to_radians();
                let at = |r: f64| to_world(6000.0 + r * a.cos(), 6000.0 + r * a.sin());
                let outside = ring.outer_radius() + kerb + 2.0;
                assert!(
                    covered(&all, at(outside)),
                    "{} {}° outside",
                    ring.id,
                    bridge.angle_degrees
                );
                // Nothing reaches the moat from the Capitol's side yet: that's the tower (P3.7).
                if ring.id != "moat" {
                    let inside = ring.inner_radius() - kerb - 2.0;
                    assert!(
                        covered(&all, at(inside)),
                        "{} {}° inside",
                        ring.id,
                        bridge.angle_degrees
                    );
                }
                // And stops at the kerb.
                assert!(!covered(&all, at(ring.outer_radius() + kerb - 2.0)));
            }
        }
    }

    #[test]
    fn each_mesh_keeps_to_its_sector() {
        let terrain = fixture();
        let widest = terrain.roads.iter().map(|r| r.width).fold(0.0, f64::max);
        let reach = widest * 0.5 * MITER_LIMIT + 1.0;
        for ((_, sector), m) in build(&terrain.roads, &terrain.water_rings) {
            for v in &m.vertices {
                let p = to_server(v.position[0], v.position[2]);
                let near = Sector::of(p) == sector
                    || (0..16).any(|k| {
                        let a = k as f64 * PI / 8.0;
                        Sector::of(add(p, [reach * a.cos(), reach * a.sin()])) == sector
                    });
                assert!(near, "{p:?} in {sector:?}");
            }
        }
    }

    #[test]
    fn degenerate_roads_draw_nothing_or_something_finite() {
        for points in [
            vec![],
            vec![(6500.0, 6000.0)],
            vec![(6500.0, 6000.0), (6500.0, 6000.0)],
            vec![(6500.0, 6000.0), (6500.2, 6000.1)],
        ] {
            assert!(draw(&points, 5.0).indices.is_empty(), "{points:?}");
        }
    }

    #[test]
    fn every_type_and_sector_has_its_own_mesh_name() {
        let mut names: Vec<_> = TYPES
            .iter()
            .flat_map(|&t| Sector::all().map(move |s| mesh_name(t, s)))
            .collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}
