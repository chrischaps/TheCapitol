//! The water (P3.3): the moat and the two canals, each a channel of
//! translucent water inside a low stone kerb, broken where a bridge
//! crosses.
//!
//! Each ring is cut into *reaches*, the open water between one bridge and
//! the next. A gap has straight, parallel sides exactly the bridge's
//! nominal width apart, so the deck P3.5 lays across it fits. The server
//! is more generous than that: it lets a move cross anywhere within twice
//! the width (`Bridge::angular_half_width`), so everywhere drawn dry is
//! walkable, and a little drawn wet near a bridge is too.
//!
//! Two meshes. The banks (the dark bed under the water and the kerb that
//! rims every reach) are opaque and built once per terrain. The surface is
//! translucent and recoloured every frame: slow swells drift around each
//! ring, their crests catching the highlight colour, and tilt the normals
//! so the low sun glints off them. Toward the kerbs the water darkens and
//! thins, as if shaded by the walls. The ocean shader (H6, P8) replaces
//! the surface later.

use crate::coords::{layer, polar_to_world, S};
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{self, hex, mix};
use capitol_protocol::terrain::WaterRing;
use flint_import::{AlphaMode, ImportedMaterial};
use flint_player::ExtensionGpu;

/// The runtime meshes' names, as the scene's water entities name them.
pub const SURFACE_MESH: &str = "water";
pub const BANKS_MESH: &str = "water_banks";

/// Vertex spacing along a ring, in server units (1 m): a few vertices to
/// the shortest swell.
const STEP: f64 = 10.0;
/// Rows of vertices across the channel, so the water can darken toward
/// its kerbs.
const ROWS: usize = 6;
/// The kerb's width in metres.
const KERB: f32 = 1.0;
/// The swells' wavelengths in metres, rounded per ring to a whole number of
/// waves so a ring with no bridge has no seam.
const SWELLS: [f32; 2] = [9.0, 5.5];
/// How fast each swell's phase turns, in radians a second. They drift in
/// opposite directions, at about 0.7 and 0.3 m/s.
const DRIFT: [f32; 2] = [0.5, -0.35];
/// How far a swell tilts the surface normal, at its steepest.
const TILT: f32 = 0.12;

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
        let side = |h: f64| (h / r).clamp(-1.0, 1.0).asin().to_degrees();
        (self.from.0 + side(self.from.1), self.to.0 - side(self.to.1))
    }

    /// The server polar point (radius, degrees) at `t` along the reach
    /// (0 = start, 1 = end) and `u` across it (0 = inner edge, 1 = outer).
    fn polar(&self, t: f64, u: f64) -> (f64, f64) {
        let r = self.r_in + (self.r_out - self.r_in) * u;
        let (a, b) = self.span(r);
        (r, a + (b - a) * t)
    }

    fn world(&self, t: f64, u: f64) -> [f32; 2] {
        let (r, deg) = self.polar(t, u);
        polar_to_world(r, deg)
    }

    /// Columns along the reach: one per `STEP` at its outer edge.
    fn columns(&self) -> usize {
        let (a, b) = self.span(self.r_out);
        ((b - a).to_radians() * self.r_out / STEP).ceil().max(1.0) as usize
    }
}

/// A ring's reaches, in order round from its first bridge.
fn reaches(ring: &WaterRing) -> Vec<Reach> {
    let (r_in, r_out) = (ring.inner_radius(), ring.outer_radius());
    let mut bridges: Vec<(f64, f64)> = ring
        .bridges
        .iter()
        .map(|b| (b.angle_degrees.rem_euclid(360.0), b.width * 0.5))
        .collect();
    bridges.sort_by(|a, b| a.0.total_cmp(&b.0));
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

/// Where a surface vertex sits in the water, for shading.
#[derive(Debug, Clone, Copy)]
struct Sample {
    /// Each swell's phase here, in radians.
    phase: [f32; 2],
    /// 0 at the inner kerb, 1 at the outer.
    across: f32,
    /// Unit direction along the ring (increasing angle), world XZ.
    tangent: [f32; 2],
}

/// The water's surface: built once per terrain, recoloured every frame.
#[derive(Default)]
pub struct Water {
    mesh: MeshBuf,
    samples: Vec<Sample>,
}

impl Water {
    pub fn new(rings: &[WaterRing]) -> Self {
        let mut water = Water::default();
        for ring in rings {
            let waves = SWELLS.map(|l| {
                let metres = std::f64::consts::TAU * ring.radius * S as f64;
                (metres / l as f64).round().max(1.0) as f32
            });
            for reach in reaches(ring) {
                water.add_reach(&reach, waves);
            }
        }
        water
    }

    fn add_reach(&mut self, reach: &Reach, waves: [f32; 2]) {
        let cols = reach.columns();
        let base = self.mesh.vertices.len() as u32;
        for i in 0..=cols {
            let t = i as f64 / cols as f64;
            for k in 0..=ROWS {
                let u = k as f64 / ROWS as f64;
                let (r, deg) = reach.polar(t, u);
                self.mesh
                    .flat_vertex(polar_to_world(r, deg), layer::WATER, [0.0; 4]);
                let theta = deg.to_radians();
                self.samples.push(Sample {
                    phase: waves.map(|n| (theta * n as f64) as f32),
                    across: u as f32,
                    tangent: [-theta.sin() as f32, theta.cos() as f32],
                });
            }
        }
        let stride = (ROWS + 1) as u32;
        for i in 0..cols as u32 {
            for k in 0..ROWS as u32 {
                let inner = base + i * stride + k;
                let outer = inner + 1;
                let (inner_next, outer_next) = (inner + stride, outer + stride);
                self.mesh
                    .indices
                    .extend([inner, outer_next, outer, inner, inner_next, outer_next]);
            }
        }
    }

    /// Recolour the surface for game time `t` (seconds) and upload it.
    pub fn upload(&mut self, gpu: &mut ExtensionGpu, t: f64) {
        self.animate(t as f32);
        self.mesh.upload_material(
            gpu,
            ImportedMaterial {
                roughness: 0.15,
                alpha_mode: AlphaMode::Blend,
                ..ImportedMaterial::vertex_colored(SURFACE_MESH)
            },
        );
    }

    fn animate(&mut self, t: f32) {
        for (v, s) in self.mesh.vertices.iter_mut().zip(&self.samples) {
            (v.color, v.normal) = shade(s, t);
        }
    }
}

/// A surface vertex's colour and normal at time `t`.
fn shade(s: &Sample, t: f32) -> ([f32; 4], [f32; 3]) {
    let a = s.phase[0] + DRIFT[0] * t + s.across * 1.7;
    let b = s.phase[1] + DRIFT[1] * t - s.across * 2.3;
    let swell = 0.6 * a.sin() + 0.4 * b.sin();
    let crest = swell.max(0.0).powi(4);
    let slope = 0.6 * a.cos() + 0.4 * b.cos();

    // 0 against a kerb, 1 out in the channel.
    let open = smoothstep(0.0, 0.35, s.across.min(1.0 - s.across));
    let deep = mix(hex(palette::BACKGROUND), hex(palette::WATER), 0.25);
    let body = mix(deep, hex(palette::WATER), 0.25 + 0.45 * open);
    let mut colour = mix(body, hex(palette::WATER_HIGHLIGHT), 0.4 * crest);
    colour[3] = 0.62 + 0.2 * open + 0.12 * crest;

    let [tx, tz] = s.tangent;
    let n = [-TILT * slope * tx, 1.0, -TILT * slope * tz];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (colour, n.map(|c| c / len))
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The opaque part of the water: a dark bed under each reach and a stone
/// kerb around it. The moat's kerb is the Capitol's dim gold; the canals'
/// are a warm grey.
fn banks(rings: &[WaterRing]) -> MeshBuf {
    let mut m = MeshBuf::default();
    let bed = mix(hex(palette::BACKGROUND), hex(palette::WATER), 0.2);
    for ring in rings {
        let stone = if ring.id == "moat" {
            hex(palette::GOLD_DIM)
        } else {
            let warm = mix(hex(palette::DIM_TEXT), hex(palette::GOLD_DIM), 0.25);
            mix(warm, hex(palette::BACKGROUND), 0.35)
        };
        for reach in reaches(ring) {
            let cols = reach.columns();
            let at = |i: usize, u: f64| reach.world(i as f64 / cols as f64, u);
            for i in 0..cols {
                let corners = [at(i, 0.0), at(i, 1.0), at(i + 1, 1.0), at(i + 1, 0.0)];
                m.quad(corners.map(|p| lift(p, layer::WATER_BED)), UP, bed);
            }
            let inner: Vec<_> = (0..=cols).map(|i| at(i, 0.0)).collect();
            let outer: Vec<_> = (0..=cols).rev().map(|i| at(i, 1.0)).collect();
            if reach.closed {
                // The first and last columns meet; drop the repeat.
                kerb(&mut m, &inner[..cols], false, stone);
                kerb(&mut m, &outer[..cols], true, stone);
            } else {
                // Round the water: in along the inner edge, across the gap
                // line, back along the outer edge and across the other.
                let outline: Vec<_> = inner.into_iter().chain(outer).collect();
                kerb(&mut m, &outline, true, stone);
            }
        }
    }
    m
}

/// Upload the banks for these rings (once per terrain).
pub fn upload_banks(gpu: &mut ExtensionGpu, rings: &[WaterRing]) {
    banks(rings).upload(gpu, BANKS_MESH, 0.85);
}

fn lift([x, z]: [f32; 2], y: f32) -> [f32; 3] {
    [x, y, z]
}

/// A kerb `KERB` wide and up to `layer::BANK` round the closed loop `pts`
/// (world XZ), on its outside when `water_inside`, else on its inside.
/// Corners are mitred. The wall facing the water runs down to the bed, so
/// it shows through the water; the one facing the land runs to the ground.
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
    for i in 0..n {
        let j = (i + 1) % n;
        let [nx, nz] = normals[i];
        let (p, q, po, qo) = (pts[i], pts[j], offset[i], offset[j]);
        let top = layer::BANK;
        m.quad([p, q, qo, po].map(|c| lift(c, top)), UP, colour);
        m.quad(
            [lift(po, 0.0), lift(qo, 0.0), lift(qo, top), lift(po, top)],
            [nx, 0.0, nz],
            colour,
        );
        m.quad(
            [
                lift(p, layer::WATER_BED),
                lift(q, layer::WATER_BED),
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
    use capitol_protocol::rules::{CENTER_X, CENTER_Y};
    use capitol_protocol::terrain::Bridge;
    use std::path::Path;

    fn rings() -> Vec<WaterRing> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
        assert_eq!(terrain.water_rings.len(), 3);
        terrain.water_rings
    }

    /// Whether world (x, z) is under some triangle of `m`, seen from above.
    fn covered(m: &MeshBuf, [x, z]: [f32; 2]) -> bool {
        m.indices
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]])
            .any(|tri| {
                let [a, b, c] = tri.map(|i| {
                    let p = m.vertices[i as usize].position;
                    [p[0], p[2]]
                });
                if x < a[0].min(b[0]).min(c[0]) || x > a[0].max(b[0]).max(c[0]) {
                    return false;
                }
                let side = |p: [f32; 2], q: [f32; 2]| {
                    (q[0] - p[0]) * (z - p[1]) - (q[1] - p[1]) * (x - p[0])
                };
                let (d0, d1, d2) = (side(a, b), side(b, c), side(c, a));
                (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
            })
    }

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
            let water = Water::new(std::slice::from_ref(&ring));
            for bridge in &ring.bridges {
                let h = bridge.width * 0.5;
                for u in [0.05, 0.25, 0.5, 0.75, 0.95] {
                    let r = ring.inner_radius() + ring.width * u;
                    for side in [-1.0, 1.0] {
                        for (offset, wet) in [(h - 0.5, false), (h + 0.5, true)] {
                            let along = (r * r - offset * offset).sqrt();
                            let p = beside_bridge(bridge, along, side * offset);
                            assert_eq!(
                                covered(&water.mesh, p),
                                wet,
                                "{} bridge {}: {offset} units {} of the axis at r {r}",
                                ring.id,
                                bridge.angle_degrees,
                                if side < 0.0 {
                                    "anticlockwise"
                                } else {
                                    "clockwise"
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_only_gaps_round_each_ring_are_its_bridges() {
        for ring in rings() {
            let water = Water::new(std::slice::from_ref(&ring));
            for step in 0..1440 {
                let deg = step as f64 * 0.25;
                let a = deg.to_radians();
                let (x, y) = (ring.radius * a.cos(), ring.radius * a.sin());
                // Distance from each bridge's axis, on its side of the centre.
                let gap_edge = ring
                    .bridges
                    .iter()
                    .map(|b| {
                        let (bx, by) = (
                            b.angle_degrees.to_radians().cos(),
                            b.angle_degrees.to_radians().sin(),
                        );
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
                assert_eq!(
                    covered(&water.mesh, p),
                    gap_edge > 0.0,
                    "{} at {deg}°",
                    ring.id
                );
            }
        }
    }

    #[test]
    fn the_server_lets_you_cross_wherever_a_gap_is_drawn() {
        for ring in rings() {
            for reach in reaches(&ring) {
                for r in [ring.inner_radius(), ring.radius, ring.outer_radius()] {
                    let (start, end) = reach.span(r);
                    let after = ring
                        .bridges
                        .iter()
                        .find(|b| b.angle_degrees.rem_euclid(360.0) == reach.from.0)
                        .unwrap();
                    let before = ring
                        .bridges
                        .iter()
                        .find(|b| b.angle_degrees.rem_euclid(360.0) == reach.to.0.rem_euclid(360.0))
                        .unwrap();
                    assert!(
                        after.contains_angle(start, ring.radius),
                        "{} {start}",
                        ring.id
                    );
                    assert!(before.contains_angle(end, ring.radius), "{} {end}", ring.id);
                }
            }
        }
    }

    #[test]
    fn a_ring_with_no_bridges_is_water_all_the_way_round() {
        let ring = WaterRing {
            id: "test".into(),
            radius: 600.0,
            width: 50.0,
            bridges: vec![],
        };
        let water = Water::new(std::slice::from_ref(&ring));
        for step in 0..360 {
            let a = (step as f64 + 0.5).to_radians();
            let p = to_world(CENTER_X + 600.0 * a.cos(), CENTER_Y + 600.0 * a.sin());
            assert!(covered(&water.mesh, p), "{step}°");
        }
        // Two kerbs, inner and outer, both on land.
        let banks = banks(std::slice::from_ref(&ring));
        assert_kerbs_on_land(&banks, &water.mesh);
    }

    fn assert_kerbs_on_land(banks: &MeshBuf, water: &MeshBuf) {
        let mut tops = 0;
        for tri in banks.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]) {
            let v = tri.map(|i| banks.vertices[i as usize]);
            if v.iter().all(|v| v.position[1] == layer::BANK) {
                tops += 1;
                let c = [0, 2].map(|k| v.iter().map(|v| v.position[k]).sum::<f32>() / 3.0);
                assert!(!covered(water, c), "a kerb over the water at {c:?}");
            }
        }
        assert!(tops > 0);
    }

    #[test]
    fn kerbs_rim_the_water_on_land() {
        for ring in rings() {
            let one = std::slice::from_ref(&ring);
            assert_kerbs_on_land(&banks(one), &Water::new(one).mesh);
        }
    }

    #[test]
    fn every_bank_face_faces_its_normal() {
        let banks = banks(&rings());
        for tri in banks.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]) {
            let v = tri.map(|i| banks.vertices[i as usize]);
            assert!(facing(
                v[0].position,
                v[1].position,
                v[2].position,
                v[0].normal
            ));
        }
    }

    #[test]
    fn the_surface_faces_up_and_stays_translucent() {
        let mut water = Water::new(&rings());
        for t in [0.0, 1.7, 40.0] {
            water.animate(t);
            for tri in water
                .mesh
                .indices
                .chunks_exact(3)
                .map(|t| [t[0], t[1], t[2]])
            {
                let p = tri.map(|i| water.mesh.vertices[i as usize].position);
                assert!(facing(p[0], p[1], p[2], UP));
            }
            for v in &water.mesh.vertices {
                assert!((0.5..0.98).contains(&v.color[3]), "alpha {}", v.color[3]);
                let len = v.normal.iter().map(|c| c * c).sum::<f32>().sqrt();
                assert!((len - 1.0).abs() < 1e-4 && v.normal[1] > 0.98);
            }
        }
    }
}
