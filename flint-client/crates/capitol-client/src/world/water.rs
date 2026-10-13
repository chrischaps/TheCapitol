//! The canals (P3.3): the moat and the two canals, cut through the land
//! down to the sea.
//!
//! The water itself is Flint's Gerstner ocean, the one Float drifts on,
//! configured in the scene at canal scale. It's an endless sheet at y = 0
//! (`layer::SEA`) that follows the camera. The land stands above it
//! (`layer::GROUND`), and the ground leaves a hole over each ring
//! ([`holes`]), so the sea shows only in the channels. This module builds
//! everything solid around that water, as one opaque mesh:
//!
//! - a stone wall down each side of every channel, through the water to a
//!   dark bed that the ocean's refraction shows;
//! - a low kerb along the top of each wall, the Capitol's dim gold at the
//!   moat and a warm grey at the canals.
//!
//! The water runs right round each ring. Its bridges (P3.5, `bridges`)
//! stand on arches over it, so the canal carries on beneath them.

use crate::coords::{layer, polar_to_world, S};
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{self, hex, mix};
use capitol_protocol::terrain::WaterRing;
use flint_player::ExtensionGpu;

/// The runtime mesh's name, as the scene's canals entity names it.
pub const MESH: &str = "canals";

/// Vertex spacing along a ring, in server units (1 m).
const STEP: f64 = 10.0;
/// The kerb's width in metres.
pub const KERB: f32 = 1.0;

/// Columns round a ring: one per `STEP` at its outer edge.
fn columns(ring: &WaterRing) -> usize {
    (std::f64::consts::TAU * ring.outer_radius() / STEP).ceil() as usize
}

/// World (X, Z) at column `i` round the ring and `u` across it (0 = inner
/// edge, 1 = outer).
fn at(ring: &WaterRing, i: usize, u: f64) -> [f32; 2] {
    let r = ring.inner_radius() + ring.width * u;
    polar_to_world(r, 360.0 * i as f64 / columns(ring) as f64)
}

/// The bands of radii the ground leaves open, one per ring, so the sea
/// shows through: `(inner edge, outer edge)` in server units.
pub fn holes(rings: &[WaterRing]) -> Vec<(f64, f64)> {
    rings
        .iter()
        .map(|r| (r.inner_radius(), r.outer_radius()))
        .collect()
}

/// The bands of radii a road keeps out of, one per ring: the channel and
/// the kerb along each side of it, `(inner, outer)` in server units. A road
/// stops at the kerb's land face, at a bridge too, where P3.5's deck takes
/// over.
pub fn kerbed(rings: &[WaterRing]) -> Vec<(f64, f64)> {
    let kerb = (KERB / S) as f64;
    rings
        .iter()
        .map(|r| (r.inner_radius() - kerb, r.outer_radius() + kerb))
        .collect()
}

/// The kerb stone for a ring: dim gold round the Capitol, warm grey
/// elsewhere.
fn stone(ring: &WaterRing) -> [f32; 4] {
    if ring.id == "moat" {
        hex(palette::GOLD_DIM)
    } else {
        let warm = mix(hex(palette::DIM_TEXT), hex(palette::GOLD_DIM), 0.25);
        mix(warm, hex(palette::BACKGROUND), 0.35)
    }
}

/// The channel floor, down at `layer::CHANNEL_BED`.
fn bed(ring: &WaterRing) -> MeshBuf {
    let mut m = MeshBuf::default();
    let bed = mix(hex(palette::BACKGROUND), hex(palette::WATER), 0.3);
    for i in 0..columns(ring) {
        let corners = [
            at(ring, i, 0.0),
            at(ring, i, 1.0),
            at(ring, i + 1, 1.0),
            at(ring, i + 1, 0.0),
        ];
        m.quad(corners.map(|p| lift(p, layer::CHANNEL_BED)), UP, bed);
    }
    m
}

/// The walls and kerbs along both edges of the channel.
fn walls(ring: &WaterRing) -> MeshBuf {
    let mut m = MeshBuf::default();
    let stone = stone(ring);
    let edge = |u: f64| -> Vec<_> { (0..columns(ring)).map(|i| at(ring, i, u)).collect() };
    // The water lies outside the inner edge and inside the outer one.
    kerb(&mut m, &edge(0.0), false, stone);
    kerb(&mut m, &edge(1.0), true, stone);
    m
}

/// Everything solid about the canals.
fn build(rings: &[WaterRing]) -> MeshBuf {
    let mut m = MeshBuf::default();
    for ring in rings {
        m.append(bed(ring));
        m.append(walls(ring));
    }
    m
}

/// Upload the canals for these rings (once per terrain).
pub fn upload(gpu: &mut ExtensionGpu, rings: &[WaterRing]) {
    build(rings).upload(gpu, MESH, 0.85);
}

fn lift([x, z]: [f32; 2], y: f32) -> [f32; 3] {
    [x, y, z]
}

/// A kerb `KERB` wide round the closed loop `pts` (world XZ), on its
/// outside when `water_inside`, else on its inside. Corners are mitred.
/// Its top is `layer::BANK`. The wall facing the land drops to the ground;
/// the one facing the water drops through it to the channel bed, so the
/// water is held between stone.
fn kerb(m: &mut MeshBuf, pts: &[[f32; 2]], water_inside: bool, colour: [f32; 4]) {
    let n = pts.len();
    if n < 3 {
        return;
    }
    let twice_area: f32 = (0..n)
        .map(|i| {
            let (p, q) = (pts[i], pts[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum();
    let sign = twice_area.signum() * if water_inside { 1.0 } else { -1.0 };
    // Each edge's unit normal pointing away from the water.
    let normals: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let (p, q) = (pts[i], pts[(i + 1) % n]);
            let (dx, dz) = (q[0] - p[0], q[1] - p[1]);
            let len = dx.hypot(dz).max(1e-6);
            [sign * dz / len, -sign * dx / len]
        })
        .collect();
    let offset: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let (a, b) = (normals[(i + n - 1) % n], normals[i]);
            let mid = [a[0] + b[0], a[1] + b[1]];
            let len = mid[0].hypot(mid[1]).max(1e-6);
            let dir = [mid[0] / len, mid[1] / len];
            let reach = (KERB / (dir[0] * b[0] + dir[1] * b[1]).max(1e-3)).min(3.0 * KERB);
            [pts[i][0] + dir[0] * reach, pts[i][1] + dir[1] * reach]
        })
        .collect();
    let top = layer::BANK;
    for i in 0..n {
        let j = (i + 1) % n;
        let [nx, nz] = normals[i];
        let (p, q, po, qo) = (pts[i], pts[j], offset[i], offset[j]);
        m.quad([p, q, qo, po].map(|c| lift(c, top)), UP, colour);
        m.quad(
            [
                lift(po, layer::GROUND),
                lift(qo, layer::GROUND),
                lift(qo, top),
                lift(po, top),
            ],
            [nx, 0.0, nz],
            colour,
        );
        m.quad(
            [
                lift(p, layer::CHANNEL_BED),
                lift(q, layer::CHANNEL_BED),
                lift(q, top),
                lift(p, top),
            ],
            [-nx, 0.0, -nz],
            colour,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::ground;
    use crate::world::mesh::facing;
    use crate::world::mesh::probe::{covered, triangles};
    use capitol_protocol::rules::WORLD_SIZE;
    use std::path::Path;

    fn rings() -> Vec<WaterRing> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
        assert_eq!(terrain.water_rings.len(), 3);
        terrain.water_rings
    }

    #[test]
    fn the_water_runs_all_the_way_round_each_ring_bridges_or_not() {
        for ring in rings() {
            let water = bed(&ring);
            for step in 0..1440 {
                let deg = step as f64 * 0.25 + 0.1;
                for r in [
                    ring.inner_radius() + 0.5,
                    ring.radius,
                    ring.outer_radius() - 0.5,
                ] {
                    assert!(
                        covered(&water, polar_to_world(r, deg)),
                        "{} r {r} at {deg}°",
                        ring.id
                    );
                }
            }
        }
    }

    #[test]
    fn the_sea_shows_only_in_the_channels() {
        let rings = rings();
        let ground = ground::build(WORLD_SIZE, &holes(&rings));
        for ring in &rings {
            // Only the ground near this ring, to keep the test quick.
            let (lo, hi) = (
                ring.inner_radius() * 0.1 - 6.0,
                ring.outer_radius() * 0.1 + 6.0,
            );
            let mut ground_here = MeshBuf::default();
            for tri in triangles(&ground) {
                let r = |v: &flint_render::Vertex| v.position[0].hypot(v.position[2]) as f64;
                if tri.iter().any(|v| (lo..hi).contains(&r(v))) {
                    let i = ground_here.vertices.len() as u32;
                    ground_here.vertices.extend(tri);
                    ground_here.indices.extend([i, i + 1, i + 2]);
                }
            }
            let ground = ground_here;
            let water = bed(ring);
            for step in 0..90 {
                let deg = step as f64 * 4.0 + 1.3;
                for (r, in_band) in [
                    (ring.inner_radius() - 3.0, false),
                    (ring.inner_radius() + 2.0, true),
                    (ring.radius, true),
                    (ring.outer_radius() - 2.0, true),
                    (ring.outer_radius() + 3.0, false),
                ] {
                    let p = polar_to_world(r, deg);
                    let at = format!("{} r {r} at {deg}°", ring.id);
                    assert_eq!(covered(&ground, p), !in_band, "ground, {at}");
                    assert_eq!(covered(&water, p), in_band, "water, {at}");
                }
            }
        }
    }

    #[test]
    fn kerbs_rim_the_water_on_land() {
        for ring in rings() {
            let (walls, water) = (walls(&ring), bed(&ring));
            let mut tops = 0;
            for v in triangles(&walls) {
                if v.iter().all(|v| v.position[1] == layer::BANK) {
                    tops += 1;
                    let c = [0, 2].map(|k| v.iter().map(|v| v.position[k]).sum::<f32>() / 3.0);
                    assert!(!covered(&water, c), "a kerb over the water at {c:?}");
                }
            }
            assert!(tops > 0);
        }
    }

    #[test]
    fn walls_run_from_the_bed_through_the_sea_to_the_kerb() {
        let m = walls(&rings()[1]);
        let lowest = m
            .vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::MAX, f32::min);
        let highest = m
            .vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::MIN, f32::max);
        assert_eq!((lowest, highest), (layer::CHANNEL_BED, layer::BANK));
    }

    #[test]
    fn every_face_faces_its_normal() {
        let m = build(&rings());
        for v in triangles(&m) {
            assert!(facing(
                v[0].position,
                v[1].position,
                v[2].position,
                v[0].normal
            ));
        }
    }
}
