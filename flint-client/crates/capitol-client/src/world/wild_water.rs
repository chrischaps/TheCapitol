//! The wilds' water (W10, #203): W3's rivers, streams and lakes drawn as
//! Flint water bodies (W7, ADR 0094), each at its own level, running
//! downhill in its carved bed.
//!
//! - **Courses** are ribbons from bank to bank (`WaterBodyMesh::river`),
//!   resting on their `surface`. They flow from source to mouth, faster
//!   where the surface falls (so a steep stream runs white), and fastest
//!   over a ford, where the shallows riffle and foam over pale gravel
//!   (`land_mesh::Shore`).
//! - **Lakes and ponds** are flat sheets at their level, cut into a grid
//!   fine enough to carry the waves, over every cell that touches the
//!   water. Their islands are left out, and the cells just past the shore
//!   tuck under the rising bank.
//! - **Where water meets water,** a tributary stops just inside the course
//!   it joins, and a course just inside the lake it runs into or out of,
//!   a little lower, so the two surfaces never fight.
//! - **On the land.** A surface sits [`SINK`] under its nominal level, so on
//!   the rural plain, where the bank is level with the water, the ground
//!   hides the edge. But the land is sampled every metre, and a stream
//!   narrower than that has no bed in it: wherever the drawn ground is
//!   higher than the water, the water rides [`LIFT`] over it instead.
//! - **Three bodies**, one per kind, because bodies aren't culled (ADR
//!   0094). Their colours are left to the ocean's, so they follow the hour.

use crate::coords::{layer, to_world, S};
use capitol_protocol::terrain::{Ford, Lake, Mouth, WaterCourse, WaterPoint};
use flint_player::{ExtensionGpu, GroundField, RiverPoint, WaterBodyLook, WaterBodyMesh, WaterVertex};
use std::collections::HashMap;

/// The bodies' ids.
pub const RIVERS: &str = "wild_rivers";
pub const STREAMS: &str = "wild_streams";
pub const LAKES: &str = "wild_lakes";

/// How far (m) a course's surface sits under its nominal level, and a
/// lake's: enough that the ground at a level bank hides the edge, from as
/// far off as the depth buffer tells them apart.
pub const SINK: f32 = 0.02;
pub const LAKE_SINK: f32 = 0.04;
/// How far (m) the water rides over ground that stands above it.
pub const LIFT: f32 = 0.03;
/// Where water runs into other water, how much lower (m) its tip is.
const TIP: f32 = 0.015;
/// How far (server units) a course runs on into the lake it meets.
const INTO_LAKE: f64 = 6.0;
/// How far into the course it joins a tributary stops: this share of the
/// way from that one's bank to its middle.
const INTO_JOIN: f64 = 0.15;
/// The longest stretch (server units) between a course's points once it's
/// been filled in, so the lift, the trims and the fords are fine enough.
const STEP: f64 = 10.0;
/// A lake's grid cell (server units).
const CELL: f64 = 20.0;

/// How fast (m/s) water runs on the flat, and how much faster per unit of
/// fall (surface drop over run), up to [`MAX_SPEED`].
const RIVER_SPEED: f32 = 0.7;
const STREAM_SPEED: f32 = 0.5;
const FALL_SPEEDUP: f32 = 8.0;
const MAX_SPEED: f32 = 3.5;
/// How much faster (m/s) the water runs over a ford's shallows.
const FORD_RUSH: f32 = 1.8;

/// How each kind looks where it isn't the sea. The waves are the canals'
/// scaled down: past about 0.3 the crests foam all over a wide river.
pub fn look(id: &str) -> WaterBodyLook {
    match id {
        RIVERS => WaterBodyLook {
            wave_scale: 0.2,
            flow_foam: 1.0,
            flow_period: 2.0,
            ..WaterBodyLook::default()
        },
        STREAMS => WaterBodyLook {
            wave_scale: 0.15,
            flow_foam: 1.2,
            flow_period: 1.5,
            ..WaterBodyLook::default()
        },
        _ => WaterBodyLook {
            wave_scale: 0.12,
            flow_foam: 0.0,
            ..WaterBodyLook::default()
        },
    }
}

/// The three bodies for a terrain on its land.
pub struct Bodies {
    pub rivers: WaterBodyMesh,
    pub streams: WaterBodyMesh,
    pub lakes: WaterBodyMesh,
}

impl Bodies {
    pub fn each(&self) -> [(&'static str, &WaterBodyMesh); 3] {
        [(RIVERS, &self.rivers), (STREAMS, &self.streams), (LAKES, &self.lakes)]
    }
}

/// The water on a terrain, as `/terrain` has it.
#[derive(Debug, Clone, Copy)]
pub struct Water<'a> {
    pub rivers: &'a [WaterCourse],
    pub streams: &'a [WaterCourse],
    pub lakes: &'a [Lake],
    pub fords: &'a [Ford],
}

/// Builds every body. `field` is the land (`land::Sampled::field`).
pub fn build(water: Water, field: &GroundField) -> Bodies {
    let ground = |[x, z]: [f32; 2]| field.height_at(x, z);
    let all: Vec<&WaterCourse> = water.rivers.iter().chain(water.streams).collect();
    let courses = |list: &[WaterCourse], base: f32| {
        let mut mesh = WaterBodyMesh::default();
        for course in list {
            if let Some(m) = course_mesh(course, &all, water, base, &ground) {
                append(&mut mesh, m);
            }
        }
        mesh
    };
    let mut lakes = WaterBodyMesh::default();
    for lake in water.lakes {
        append(&mut lakes, lake_mesh(lake));
    }
    Bodies {
        rivers: courses(water.rivers, RIVER_SPEED),
        streams: courses(water.streams, STREAM_SPEED),
        lakes,
    }
}

/// Uploads the bodies, replacing any from an earlier terrain.
pub fn upload(gpu: &mut ExtensionGpu, bodies: &Bodies) {
    for (id, mesh) in bodies.each() {
        if mesh.indices.is_empty() {
            gpu.remove_water_body(id);
        } else if let Err(e) = gpu.set_water_body(id, mesh, look(id)) {
            tracing::warn!("wild water: {id}: {e}");
        }
    }
}

fn append(into: &mut WaterBodyMesh, m: WaterBodyMesh) {
    let base = into.vertices.len() as u32;
    into.vertices.extend(m.vertices);
    into.indices.extend(m.indices.into_iter().map(|i| i + base));
}

fn lerp_point(a: &WaterPoint, b: &WaterPoint, t: f64) -> WaterPoint {
    let l = |p: f64, q: f64| p + (q - p) * t;
    WaterPoint {
        x: l(a.x, b.x),
        y: l(a.y, b.y),
        width: l(a.width, b.width),
        depth: l(a.depth, b.depth),
        surface: l(a.surface, b.surface),
    }
}

fn dist(a: &WaterPoint, x: f64, y: f64) -> f64 {
    (a.x - x).hypot(a.y - y)
}

/// The course's points filled in to at most [`STEP`] apart.
fn filled(points: &[WaterPoint]) -> Vec<WaterPoint> {
    let mut out = Vec::with_capacity(points.len() * 4);
    for w in points.windows(2) {
        let n = (dist(&w[0], w[1].x, w[1].y) / STEP).ceil().max(1.0) as usize;
        out.extend((0..n).map(|k| lerp_point(&w[0], &w[1], k as f64 / n as f64)));
    }
    out.extend(points.last().copied());
    out
}

/// Where along `a`→`b` the test first turns true, by bisection, given
/// it's false at `a` and true at `b`.
fn crossing(a: &WaterPoint, b: &WaterPoint, inside: impl Fn(f64, f64) -> bool) -> WaterPoint {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..16 {
        let mid = 0.5 * (lo + hi);
        let p = lerp_point(a, b, mid);
        if inside(p.x, p.y) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    lerp_point(a, b, hi)
}

/// `points` cut where they run into the water `inside` names, running on
/// `into` past the crossing. Unchanged if they never do.
fn cut_into(points: &mut Vec<WaterPoint>, inside: impl Fn(f64, f64) -> bool, into: f64) -> bool {
    let Some(first_in) = points.iter().position(|p| inside(p.x, p.y)) else {
        return false;
    };
    if first_in == 0 {
        return false;
    }
    let (a, b) = (points[first_in - 1], points[first_in]);
    let at = crossing(&a, &b, &inside);
    let len = dist(&a, b.x, b.y).max(1e-6);
    let t = (dist(&a, at.x, at.y) + into).min(len) / len;
    points.truncate(first_in);
    points.push(lerp_point(&a, &b, t));
    true
}

/// The width of `course` nearest server (x, y).
fn width_near(course: &WaterCourse, x: f64, y: f64) -> f64 {
    course
        .points
        .iter()
        .min_by(|p, q| dist(p, x, y).total_cmp(&dist(q, x, y)))
        .map_or(0.0, |p| p.width)
}

/// One course's ribbon, trimmed where it meets other water, or `None` if
/// nothing of it is left.
fn course_mesh(
    course: &WaterCourse,
    all: &[&WaterCourse],
    water: Water,
    base_speed: f32,
    ground: &impl Fn([f32; 2]) -> Option<f32>,
) -> Option<WaterBodyMesh> {
    let lake = |id: &str| water.lakes.iter().find(|l| l.id == id);
    let mut points = filled(&course.points);
    // Server (x, y) of each end that runs into other water, and its width.
    let mut tips: Vec<([f64; 2], f64)> = Vec::new();
    if let Some(source) = course.source.as_deref().and_then(lake) {
        points.reverse();
        if cut_into(&mut points, |x, y| source.contains(x, y), INTO_LAKE) {
            tips.extend(points.last().map(|p| ([p.x, p.y], p.width)));
        }
        points.reverse();
    }
    match &course.mouth {
        Mouth::Lake(id) => {
            if let Some(mouth) = lake(id) {
                if cut_into(&mut points, |x, y| mouth.contains(x, y), INTO_LAKE) {
                    tips.extend(points.last().map(|p| ([p.x, p.y], p.width)));
                }
            }
        }
        Mouth::Joins(id) => {
            let end = *course.points.last()?;
            if let Some(main) = all.iter().find(|c| &c.id == id) {
                // A course cut in parts runs straight on into the next.
                let runs_on = main.points.first().is_some_and(|p| dist(p, end.x, end.y) < 1e-3);
                if !runs_on {
                    let reach = 0.5 * width_near(main, end.x, end.y) * (1.0 - INTO_JOIN);
                    if cut_into(&mut points, |x, y| (x - end.x).hypot(y - end.y) < reach, 0.0) {
                        tips.extend(points.last().map(|p| ([p.x, p.y], p.width)));
                    }
                }
            }
        }
        // A course to the edge stops dead on r6000, though the land runs
        // on past it (#229).
        Mouth::Edge | Mouth::PetersOut => {}
    }
    if points.len() < 2 {
        return None;
    }
    let fords: Vec<_> = water.fords.iter().filter(|f| f.river_id == course.id).collect();
    let river: Vec<RiverPoint> = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let fall = if i > 0 {
                let q = &points[i - 1];
                ((q.surface - p.surface) / dist(q, p.x, p.y).max(1e-3)).max(0.0) as f32
            } else {
                0.0
            };
            let rush = fords
                .iter()
                .map(|f| {
                    let r = (0.5 * f.width).max(1.0);
                    1.0 - smoothstep(0.4 * r, r, dist(p, f.x, f.y))
                })
                .fold(0.0, f32::max);
            let [x, z] = to_world(p.x, p.y);
            RiverPoint {
                position: [x, layer::GROUND + p.surface as f32 * S - SINK, z],
                width: p.width as f32 * S,
                speed: (base_speed * (1.0 + FALL_SPEEDUP * fall)).min(MAX_SPEED) + FORD_RUSH * rush,
            }
        })
        .collect();
    let mut mesh = WaterBodyMesh::river(&river).ok()?;
    // World (X, Z) of each tip, and how near it a vertex is at the tip.
    let tips: Vec<([f32; 2], f32)> = tips
        .iter()
        .map(|&([x, y], w)| (to_world(x, y), (INTO_LAKE + 0.5 * w) as f32 * S + 0.5))
        .collect();
    for v in &mut mesh.vertices {
        let [x, y, z] = v.position;
        if let Some(g) = ground([x, z]) {
            v.position[1] = y.max(g + LIFT);
        }
        if tips.iter().any(|&([tx, tz], reach)| (tx - x).hypot(tz - z) < reach) {
            v.position[1] -= TIP;
        }
    }
    Some(mesh)
}

/// A lake's sheet: every cell of a [`CELL`] grid that touches the water,
/// at its level.
fn lake_mesh(lake: &Lake) -> WaterBodyMesh {
    let mut mesh = WaterBodyMesh::default();
    let points = lake.rings.iter().flatten();
    let (mut min, mut max) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for &(x, y) in points {
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    }
    if min.0 > max.0 {
        return mesh;
    }
    let (x0, y0) = (min.0 - CELL, min.1 - CELL);
    let cols = ((max.0 - x0) / CELL).ceil() as i64 + 1;
    let rows = ((max.1 - y0) / CELL).ceil() as i64 + 1;
    let level = layer::GROUND + lake.level as f32 * S - LAKE_SINK;
    let near = CELL * 0.75;
    let mut index: HashMap<(i64, i64), u32> = HashMap::new();
    let mut vertex = |mesh: &mut WaterBodyMesh, i: i64, j: i64| -> u32 {
        *index.entry((i, j)).or_insert_with(|| {
            let [x, z] = to_world(x0 + i as f64 * CELL, y0 + j as f64 * CELL);
            mesh.vertices.push(WaterVertex {
                position: [x, level, z],
                normal: [0.0, 1.0, 0.0],
                flow: [0.0, 0.0],
            });
            (mesh.vertices.len() - 1) as u32
        })
    };
    for j in 0..rows {
        for i in 0..cols {
            let (cx, cy) = (x0 + (i as f64 + 0.5) * CELL, y0 + (j as f64 + 0.5) * CELL);
            if !lake.contains(cx, cy) && shore_distance(lake, cx, cy) > near {
                continue;
            }
            let [a, b, c, d] = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)].map(|(i, j)| vertex(&mut mesh, i, j));
            mesh.indices.extend([a, c, b, b, c, d]);
        }
    }
    mesh
}

/// How far server (x, y) is from the nearest shore of `lake`.
fn shore_distance(lake: &Lake, x: f64, y: f64) -> f64 {
    let mut best = f64::INFINITY;
    for ring in &lake.rings {
        let n = ring.len();
        for k in 0..n {
            let (a, b) = (ring[k], ring[(k + 1) % n]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 {
                (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            best = best.min((a.0 + t * dx - x).hypot(a.1 + t * dy - y));
        }
    }
    best
}

fn smoothstep(lo: f64, hi: f64, x: f64) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::land::tests::{fixture, fixture_field};
    use std::sync::OnceLock;

    fn bodies() -> &'static Bodies {
        static B: OnceLock<Bodies> = OnceLock::new();
        B.get_or_init(|| {
            let started = std::time::Instant::now();
            let t = fixture();
            let water = Water { rivers: &t.rivers, streams: &t.streams, lakes: &t.lakes, fords: &t.fords };
            let b = build(water, fixture_field());
            eprintln!("wild water: built in {} ms", started.elapsed().as_millis());
            b
        })
    }

    /// Whether world (X, Z) is under one of `mesh`'s triangles.
    fn covered(mesh: &WaterBodyMesh, [x, z]: [f32; 2]) -> bool {
        mesh.indices.chunks_exact(3).any(|t| {
            let [a, b, c] = [0, 1, 2].map(|k| {
                let p = mesh.vertices[t[k] as usize].position;
                [p[0], p[2]]
            });
            let side = |p: [f32; 2], q: [f32; 2]| (q[0] - p[0]) * (z - p[1]) - (q[1] - p[1]) * (x - p[0]);
            let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
            !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
        })
    }

    #[test]
    fn every_body_is_whole() {
        for (id, mesh) in bodies().each() {
            mesh.validate().unwrap();
            assert!(!mesh.indices.is_empty(), "{id}");
            assert!(mesh.vertices.iter().all(|v| v.position.iter().all(|c| c.is_finite())), "{id}");
            eprintln!("{id}: {} vertices, {} triangles", mesh.vertices.len(), mesh.indices.len() / 3);
        }
    }

    #[test]
    fn every_course_and_lake_is_drawn() {
        let t = fixture();
        let b = bodies();
        // The middle of every course, and the middle of every lake's
        // first ring's bounding box nudged onto the water.
        for (list, mesh) in [(&t.rivers, &b.rivers), (&t.streams, &b.streams)] {
            for course in list.iter() {
                let mid = &course.points[course.points.len() / 2];
                if course.points.len() < 3 || mid.width < 1.0 {
                    continue;
                }
                // A short course's middle may be in the lake it runs into.
                let at = to_world(mid.x, mid.y);
                assert!(covered(mesh, at) || covered(&b.lakes, at), "{} at its middle", course.id);
            }
        }
        for lake in &t.lakes {
            let (x, y) = lake.rings[0][0];
            // A point just inside the shore.
            let inside = (0..64)
                .map(|k| {
                    let a = k as f64 * std::f64::consts::TAU / 64.0;
                    (x + 15.0 * a.cos(), y + 15.0 * a.sin())
                })
                .find(|&(x, y)| lake.contains(x, y));
            if let Some((x, y)) = inside {
                assert!(covered(&b.lakes, to_world(x, y)), "{}", lake.id);
            }
        }
    }

    #[test]
    fn the_water_stands_in_its_bed_and_never_under_the_ground_it_shows_over() {
        let field = fixture_field();
        let t = fixture();
        let b = bodies();
        // Mid-stream, the water is at its surface, or just over the ground
        // where the field can't hold the bed.
        let mut checked = 0;
        for course in t.rivers.iter() {
            for p in course.points.iter().step_by(5) {
                let [x, z] = to_world(p.x, p.y);
                let Some(v) = b
                    .rivers
                    .vertices
                    .iter()
                    .filter(|v| (v.position[0] - x).hypot(v.position[2] - z) < 0.4)
                    .min_by(|a, b| {
                        let d = |v: &WaterVertex| (v.position[0] - x).hypot(v.position[2] - z);
                        d(a).total_cmp(&d(b))
                    })
                else {
                    continue;
                };
                let want = layer::GROUND + p.surface as f32 * S - SINK;
                let g = field.height_at(v.position[0], v.position[2]).unwrap();
                assert!(v.position[1] >= g + LIFT - TIP - 1e-4, "{}: under the ground", course.id);
                assert!(
                    (v.position[1] - want).abs() < 0.05 || (v.position[1] - g - LIFT).abs() < TIP + 1e-3,
                    "{} at ({}, {}): {} against its surface {want}",
                    course.id,
                    p.x,
                    p.y,
                    v.position[1]
                );
                checked += 1;
            }
        }
        assert!(checked > 100, "{checked}");
    }

    #[test]
    fn water_runs_downstream_and_rushes_over_a_ford() {
        let t = fixture();
        let b = bodies();
        let ford = &t.fords[0];
        let [fx, fz] = to_world(ford.x, ford.y);
        let speed = |v: &WaterVertex| v.flow[0].hypot(v.flow[1]);
        let at_ford = b
            .rivers
            .vertices
            .iter()
            .filter(|v| (v.position[0] - fx).hypot(v.position[2] - fz) < 0.5)
            .map(speed)
            .fold(0.0, f32::max);
        assert!(at_ford > 2.0, "{at_ford} m/s at {}", ford.id);
        // Downstream: each course's flow points from its first point on.
        let river = &t.rivers[0];
        let (p, q) = (&river.points[0], &river.points[1]);
        let [px, pz] = to_world(p.x, p.y);
        let [qx, qz] = to_world(q.x, q.y);
        let v = b
            .rivers
            .vertices
            .iter()
            .find(|v| (v.position[0] - px).hypot(v.position[2] - pz) < 0.6 * p.width as f32 * S + 0.1)
            .unwrap();
        assert!(v.flow[0] * (qx - px) + v.flow[1] * (qz - pz) > 0.0);
    }

    #[test]
    fn a_tributary_stops_inside_the_course_it_joins() {
        let t = fixture();
        let all: Vec<&WaterCourse> = t.rivers.iter().chain(&t.streams).collect();
        let mut seen = 0;
        for course in &t.streams {
            let Mouth::Joins(id) = &course.mouth else { continue };
            let main = all.iter().find(|c| &c.id == id).unwrap();
            let end = course.points.last().unwrap();
            if main.points[0].x == end.x && main.points[0].y == end.y {
                continue;
            }
            let mut points = filled(&course.points);
            let reach = 0.5 * width_near(main, end.x, end.y) * (1.0 - INTO_JOIN);
            assert!(cut_into(&mut points, |x, y| (x - end.x).hypot(y - end.y) < reach, 0.0) || points.len() < 2);
            let tip = points.last().unwrap();
            let d = dist(tip, end.x, end.y);
            assert!(d <= reach + 1e-6 && d >= reach * 0.9 - 1e-3, "{}: {d} against {reach}", course.id);
            seen += 1;
        }
        assert!(seen > 20, "{seen}");
    }

    #[test]
    fn a_lake_leaves_its_islands_out() {
        let t = fixture();
        let Some(lake) = t.lakes.iter().find(|l| l.rings.len() > 1) else {
            return;
        };
        let mesh = lake_mesh(lake);
        // The island's middle (its ring's centroid), if it's dry.
        let ring = &lake.rings[1];
        let n = ring.len() as f64;
        let (cx, cy) = ring.iter().fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
        if !lake.contains(cx, cy) && shore_distance(lake, cx, cy) > CELL * 1.5 {
            assert!(!covered(&mesh, to_world(cx, cy)));
        }
    }
}
