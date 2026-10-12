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
//!   moat and a warm grey at the canals;
//! - a causeway at land height across each bridge's gap, for P3.5's deck.
//!
//! Each ring is cut into *reaches*, the open water between one bridge and
//! the next. A gap has straight, parallel sides exactly the bridge's
//! nominal width apart, so the deck fits it. The server is more generous:
//! it lets a move cross anywhere within twice the width
//! (`Bridge::angular_half_width`), so everywhere drawn dry is walkable,
//! and a little drawn wet near a bridge is too.

use crate::coords::{layer, polar_to_world, S};
use crate::world::ground;
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
/// Rows of quads across a causeway, so its ground colour follows the
/// zone blend.
const CAUSEWAY_ROWS: usize = 4;

/// Open water between two bridges, or a whole ring when it has none.
#[derive(Debug, Clone, Copy)]
struct Reach {
    r_in: f64,
    r_out: f64,
    /// The bridge it starts after: its axis (degrees) and half its width.
    from: (f64, f64),
    /// The bridge it ends at. The angle is past `from`'s, by up to 360.
    to: (f64, f64),
    /// No bridges: the reach runs all the way round and meets itself.
    closed: bool,
}

impl Reach {
    /// Where the reach starts and ends at radius `r`, in degrees: on the
    /// lines half a bridge's width either side of its axis.
    fn span(&self, r: f64) -> (f64, f64) {
        (
            self.from.0 + gap_side(self.from.1, r),
            self.to.0 - gap_side(self.to.1, r),
        )
    }

    /// World (X, Z) at `t` along the reach (0 = start, 1 = end) and `u`
    /// across it (0 = inner edge, 1 = outer).
    fn world(&self, t: f64, u: f64) -> [f32; 2] {
        let r = self.r_in + (self.r_out - self.r_in) * u;
        let (a, b) = self.span(r);
        polar_to_world(r, a + (b - a) * t)
    }

    /// Columns along the reach: one per `STEP` at its outer edge.
    fn columns(&self) -> usize {
        let (a, b) = self.span(self.r_out);
        ((b - a).to_radians() * self.r_out / STEP).ceil().max(1.0) as usize
    }
}

/// The angle, in degrees, between a bridge's axis and the side of its gap
/// at radius `r`, for a gap `half` server units either side of the axis.
fn gap_side(half: f64, r: f64) -> f64 {
    (half / r).clamp(-1.0, 1.0).asin().to_degrees()
}

/// A ring's bridges as (axis degrees in 0..360, half width), in order.
fn bridges(ring: &WaterRing) -> Vec<(f64, f64)> {
    let mut bridges: Vec<(f64, f64)> = ring
        .bridges
        .iter()
        .map(|b| (b.angle_degrees.rem_euclid(360.0), b.width * 0.5))
        .collect();
    bridges.sort_by(|a, b| a.0.total_cmp(&b.0));
    bridges
}

/// A ring's reaches, in order round from its first bridge.
fn reaches(ring: &WaterRing) -> Vec<Reach> {
    let (r_in, r_out) = (ring.inner_radius(), ring.outer_radius());
    let bridges = bridges(ring);
    if bridges.is_empty() {
        return vec![Reach {
            r_in,
            r_out,
            from: (0.0, 0.0),
            to: (360.0, 0.0),
            closed: true,
        }];
    }
    let n = bridges.len();
    (0..n)
        .map(|i| {
            let (angle, half) = bridges[(i + 1) % n];
            let to = if i + 1 == n { angle + 360.0 } else { angle };
            Reach {
                r_in,
                r_out,
                from: bridges[i],
                to: (to, half),
                closed: false,
            }
        })
        // Two bridges close enough to touch leave no water between them.
        .filter(|reach| {
            let (a, b) = reach.span(reach.r_in);
            b > a
        })
        .collect()
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

/// The channel floor under each reach, down at `layer::CHANNEL_BED`.
fn beds(ring: &WaterRing) -> MeshBuf {
    let mut m = MeshBuf::default();
    let bed = mix(hex(palette::BACKGROUND), hex(palette::WATER), 0.3);
    for reach in reaches(ring) {
        let cols = reach.columns();
        let at = |i: usize, u: f64| reach.world(i as f64 / cols as f64, u);
        for i in 0..cols {
            let corners = [at(i, 0.0), at(i, 1.0), at(i + 1, 1.0), at(i + 1, 0.0)];
            m.quad(corners.map(|p| lift(p, layer::CHANNEL_BED)), UP, bed);
        }
    }
    m
}

/// The walls and kerbs round each reach.
fn walls(ring: &WaterRing) -> MeshBuf {
    let mut m = MeshBuf::default();
    let stone = stone(ring);
    for reach in reaches(ring) {
        let cols = reach.columns();
        let at = |i: usize, u: f64| reach.world(i as f64 / cols as f64, u);
        let inner: Vec<_> = (0..=cols).map(|i| at(i, 0.0)).collect();
        let outer: Vec<_> = (0..=cols).rev().map(|i| at(i, 1.0)).collect();
        if reach.closed {
            // The first and last columns meet; drop the repeat.
            kerb(&mut m, &inner[..cols], false, stone);
            kerb(&mut m, &outer[..cols], true, stone);
        } else {
            // Round the water: along the inner edge, across the gap line,
            // back along the outer edge and across the other.
            let outline: Vec<_> = inner.into_iter().chain(outer).collect();
            kerb(&mut m, &outline, true, stone);
        }
    }
    m
}

/// Land across each bridge's gap, coloured like the ground around it, in
/// a world whose edge is `edge` server units out.
fn causeways(ring: &WaterRing, edge: f64) -> MeshBuf {
    let mut m = MeshBuf::default();
    let (r_in, r_out) = (ring.inner_radius(), ring.outer_radius());
    for (axis, half) in bridges(ring) {
        let corner = |r: f64, side: f64| polar_to_world(r, axis + side * gap_side(half, r));
        for k in 0..CAUSEWAY_ROWS {
            let r0 = r_in + (r_out - r_in) * k as f64 / CAUSEWAY_ROWS as f64;
            let r1 = r_in + (r_out - r_in) * (k + 1) as f64 / CAUSEWAY_ROWS as f64;
            let shade = ground::shade((r0 + r1) * 0.5, edge);
            let mottle = ground::mottle(polar_to_world((r0 + r1) * 0.5, axis));
            let colour = [shade[0] * mottle, shade[1] * mottle, shade[2] * mottle, 1.0];
            let corners = [
                corner(r0, -1.0),
                corner(r1, -1.0),
                corner(r1, 1.0),
                corner(r0, 1.0),
            ];
            m.quad(corners.map(|p| lift(p, layer::GROUND)), UP, colour);
        }
    }
    m
}

/// Everything solid about the canals, for a world `world_size` across.
fn build(rings: &[WaterRing], world_size: f64) -> MeshBuf {
    let mut m = MeshBuf::default();
    for ring in rings {
        m.append(beds(ring));
        m.append(walls(ring));
        m.append(causeways(ring, world_size * 0.5));
    }
    m
}

/// Upload the canals for these rings (once per terrain).
pub fn upload(gpu: &mut ExtensionGpu, rings: &[WaterRing], world_size: f64) {
    build(rings, world_size).upload(gpu, MESH, 0.85);
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
    use crate::coords::to_world;
    use crate::world::mesh::facing;
    use capitol_protocol::rules::{CENTER_X, CENTER_Y, WORLD_SIZE};
    use capitol_protocol::terrain::Bridge;
    use std::path::Path;

    fn rings() -> Vec<WaterRing> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
        assert_eq!(terrain.water_rings.len(), 3);
        terrain.water_rings
    }

    use crate::world::mesh::probe::{covered, triangles};

    /// The server point `along` units out from the centre along a bridge's
    /// axis and `beside` units to one side of it.
    fn beside_bridge(bridge: &Bridge, along: f64, beside: f64) -> [f32; 2] {
        let a = bridge.angle_degrees.to_radians();
        let (ax, ay) = (a.cos(), a.sin());
        let (px, py) = (-ay, ax);
        to_world(
            CENTER_X + ax * along + px * beside,
            CENTER_Y + ay * along + py * beside,
        )
    }

    #[test]
    fn each_gap_is_exactly_its_bridge_wide_with_straight_sides() {
        for ring in rings() {
            let (water, land) = (beds(&ring), causeways(&ring, WORLD_SIZE * 0.5));
            for bridge in &ring.bridges {
                let h = bridge.width * 0.5;
                for u in [0.05, 0.25, 0.5, 0.75, 0.95] {
                    let r = ring.inner_radius() + ring.width * u;
                    for side in [-1.0, 1.0] {
                        for (offset, wet) in [(h - 0.5, false), (h + 0.5, true)] {
                            let along = (r * r - offset * offset).sqrt();
                            let p = beside_bridge(bridge, along, side * offset);
                            let what = format!(
                                "{} bridge {}: {offset} units {} of the axis at r {r}",
                                ring.id,
                                bridge.angle_degrees,
                                if side < 0.0 {
                                    "anticlockwise"
                                } else {
                                    "clockwise"
                                },
                            );
                            assert_eq!(covered(&water, p), wet, "water at {what}");
                            assert_eq!(covered(&land, p), !wet, "causeway at {what}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_only_gaps_round_each_ring_are_its_bridges() {
        for ring in rings() {
            let water = beds(&ring);
            for step in 0..1440 {
                let deg = step as f64 * 0.25;
                let a = deg.to_radians();
                let (x, y) = (ring.radius * a.cos(), ring.radius * a.sin());
                // Distance outside the nearest gap (negative inside one).
                let gap_edge = ring
                    .bridges
                    .iter()
                    .map(|b| {
                        let (by, bx) = b.angle_degrees.to_radians().sin_cos();
                        let ahead = x * bx + y * by;
                        let off = (x * by - y * bx).abs();
                        if ahead > 0.0 {
                            off - b.width * 0.5
                        } else {
                            f64::INFINITY
                        }
                    })
                    .fold(f64::INFINITY, f64::min);
                if gap_edge.abs() < 1.0 {
                    continue; // on a gap's edge
                }
                let p = to_world(CENTER_X + x, CENTER_Y + y);
                assert_eq!(covered(&water, p), gap_edge > 0.0, "{} at {deg}°", ring.id);
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
            let water = beds(ring);
            let land = causeways(ring, WORLD_SIZE * 0.5);
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
                    if in_band {
                        // Either water, or a causeway over a bridge's gap.
                        assert!(covered(&water, p) ^ covered(&land, p), "{at}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_server_lets_you_cross_wherever_a_gap_is_drawn() {
        for ring in rings() {
            let find = |angle: f64| {
                ring.bridges
                    .iter()
                    .find(|b| b.angle_degrees.rem_euclid(360.0) == angle.rem_euclid(360.0))
                    .unwrap()
            };
            for reach in reaches(&ring) {
                for r in [ring.inner_radius(), ring.radius, ring.outer_radius()] {
                    let (start, end) = reach.span(r);
                    assert!(
                        find(reach.from.0).contains_angle(start, ring.radius),
                        "{} {start}",
                        ring.id
                    );
                    assert!(
                        find(reach.to.0).contains_angle(end, ring.radius),
                        "{} {end}",
                        ring.id
                    );
                }
            }
        }
    }

    fn assert_kerbs_on_land(walls: &MeshBuf, water: &MeshBuf) {
        let mut tops = 0;
        for v in triangles(walls) {
            if v.iter().all(|v| v.position[1] == layer::BANK) {
                tops += 1;
                let c = [0, 2].map(|k| v.iter().map(|v| v.position[k]).sum::<f32>() / 3.0);
                assert!(!covered(water, c), "a kerb over the water at {c:?}");
            }
        }
        assert!(tops > 0);
    }

    #[test]
    fn a_ring_with_no_bridges_is_water_all_the_way_round() {
        let ring = WaterRing {
            id: "test".into(),
            radius: 600.0,
            width: 50.0,
            bridges: vec![],
        };
        let water = beds(&ring);
        for step in 0..360 {
            let p = polar_to_world(600.0, step as f64 + 0.5);
            assert!(covered(&water, p), "{step}°");
        }
        assert!(causeways(&ring, WORLD_SIZE * 0.5).indices.is_empty());
        assert_kerbs_on_land(&walls(&ring), &water);
    }

    #[test]
    fn kerbs_rim_the_water_on_land() {
        for ring in rings() {
            assert_kerbs_on_land(&walls(&ring), &beds(&ring));
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
        let m = build(&rings(), WORLD_SIZE);
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
