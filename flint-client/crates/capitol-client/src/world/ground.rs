//! The ground (P3.2): one flat disc under the whole world, coloured per
//! vertex. Each zone is its §4.7 colour pulled most of the way toward the
//! background, so the ground reads as tinted dark rather than as a flat
//! fill, and neighbouring zones blend softly across their boundary. Past
//! the rural ring the colour thins into the wilderness and fades to the
//! background at the world's edge (radius 4000), and the disc runs on far
//! past the world's corners so its rim is lost in the fog, never seen
//! against the sky.
//!
//! A faint, slow mottling breaks up the flat tints, the way real ground is
//! never one colour.

use crate::coords::{layer, polar_to_world};
use crate::world::mesh::MeshBuf;
use crate::world::palette::{self, hex, mix};
use capitol_protocol::rules::Zone;
use flint_player::ExtensionGpu;

/// The runtime mesh's name, as the scene's ground entity names it.
pub const MESH: &str = "ground";

/// Half-width, in server units, of the blend at a zone boundary.
const BLEND: f64 = 40.0;
/// Rural into wilderness is a wider, slower change: the city thinning out.
const WILD_BLEND: f64 = 200.0;
/// Where the disc ends, in server units from the centre: about the camera's
/// far plane (6 km) from anywhere in the world, and fogged nearly to the
/// sky's colour by then, so the sea under the land (the canals' water,
/// `world::water`) never shows at the horizon.
const SKIRT: f64 = 50_000.0;
/// Angular segments. At the world's edge one is about 52 units (5 m) wide.
const SEGMENTS: usize = 480;
/// How far the mottling moves a colour's brightness, either way.
const MOTTLE: f32 = 0.10;
/// The mottling's broadest wavelength in metres.
const MOTTLE_SCALE: f32 = 30.0;

/// How far a zone's colour is pulled from the background toward its token.
/// Bright tokens (the greens) get less, so every ring sits at about the
/// same darkness and only the hue changes; the Capitol gets the most,
/// because gold at the core is the point.
fn strength(zone: Zone) -> f32 {
    match zone {
        Zone::Capitol => 0.40,
        Zone::TradeDistrict => 0.17,
        Zone::GuildDistrict => 0.16,
        Zone::Urban => 0.20,
        Zone::Suburban => 0.10,
        Zone::Rural => 0.085,
        Zone::Wilderness => 1.0,
    }
}

/// A zone's ground colour (linear).
fn tint(zone: Zone) -> [f32; 4] {
    mix(
        hex(palette::BACKGROUND),
        hex(palette::zone(zone)),
        strength(zone),
    )
}

/// The half-width of the blend between `ALL[i]` and `ALL[i + 1]`.
fn blend_after(i: usize) -> f64 {
    if Zone::ALL[i + 1] == Zone::Wilderness {
        WILD_BLEND
    } else {
        BLEND
    }
}

fn smoothstep(lo: f64, hi: f64, x: f64) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

/// The ground's colour at `r` server units from the centre, before
/// mottling, with the world's edge at `edge`.
fn shade(r: f64, edge: f64) -> [f32; 4] {
    let zones = Zone::ALL;
    let i = zones
        .iter()
        .position(|z| r < z.radii().1)
        .unwrap_or(zones.len() - 1);
    let (lo, hi) = zones[i].radii();
    // The bands are narrower than any zone, so at most one boundary is near.
    let zoned = if i + 1 < zones.len() && r > hi - blend_after(i) {
        let h = blend_after(i);
        mix(
            tint(zones[i]),
            tint(zones[i + 1]),
            smoothstep(hi - h, hi + h, r),
        )
    } else if i > 0 && r < lo + blend_after(i - 1) {
        let h = blend_after(i - 1);
        mix(
            tint(zones[i - 1]),
            tint(zones[i]),
            smoothstep(lo - h, lo + h, r),
        )
    } else {
        tint(zones[i])
    };
    // The wilderness fades out to the world's edge.
    let wild_from = Zone::Rural.radii().1;
    mix(
        zoned,
        hex(palette::BACKGROUND),
        smoothstep(wild_from, edge, r),
    )
}

/// A hash of a lattice point to [0, 1).
fn lattice(ix: i32, iz: i32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x9e37_79b1) ^ (iz as u32).wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65536.0
}

/// Smooth value noise in [0, 1) with a lattice spacing of one unit.
fn value_noise(x: f32, z: f32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (ix, iz) = (fx as i32, fz as i32);
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, tz) = (ease(x - fx), ease(z - fz));
    let top = lattice(ix, iz) + (lattice(ix + 1, iz) - lattice(ix, iz)) * tx;
    let bottom = lattice(ix, iz + 1) + (lattice(ix + 1, iz + 1) - lattice(ix, iz + 1)) * tx;
    top + (bottom - top) * tz
}

/// The mottling at world (X, Z): a brightness factor within ±`MOTTLE`.
fn mottle([x, z]: [f32; 2]) -> f32 {
    let broad = value_noise(x / MOTTLE_SCALE, z / MOTTLE_SCALE);
    let fine = value_noise(
        x / (MOTTLE_SCALE * 0.37) + 17.0,
        z / (MOTTLE_SCALE * 0.37) - 9.0,
    );
    1.0 + MOTTLE * ((broad * 0.7 + fine * 0.3) * 2.0 - 1.0)
}

/// The radii, in server units, the disc's rings are drawn at: even steps
/// (finer in the inner city, where the camera spends its time), extra
/// rings across every blend so the curve is drawn rather than sampled
/// twice, then a few wide steps out to the skirt.
fn ring_radii(edge: f64, holes: &[(f64, f64)]) -> Vec<f64> {
    let mut radii = Vec::new();
    let mut r = 0.0;
    while r < edge {
        r += if r < 1000.0 { 20.0 } else { 40.0 };
        radii.push(r.min(edge));
    }
    for i in 0..Zone::ALL.len() - 1 {
        let (b, h) = (Zone::ALL[i].radii().1, blend_after(i));
        radii.extend((-4..=4).map(|k| b + h * k as f64 / 4.0));
    }
    for &(lo, hi) in holes {
        radii.extend([lo, hi]);
    }
    let skirt = SKIRT.max(edge * 3.0);
    radii.extend([
        edge * 1.15,
        edge * 1.4,
        edge * 1.8,
        edge * 2.4,
        edge * 4.0,
        skirt,
    ]);
    radii.retain(|&r| r > 0.0);
    radii.sort_by(f64::total_cmp);
    radii.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    radii
}

/// The ground for a world `world_size` server units across, open over
/// each `(inner, outer)` band of radii in `holes`: the canals, where the
/// sea shows through.
pub fn build(world_size: f64, holes: &[(f64, f64)]) -> MeshBuf {
    let edge = world_size * 0.5;
    let mut mesh = MeshBuf::default();
    let vertex = |mesh: &mut MeshBuf, r: f64, deg: f64| {
        let p = polar_to_world(r, deg);
        let m = mottle(p);
        let c = shade(r, edge);
        mesh.flat_vertex(p, layer::GROUND, [c[0] * m, c[1] * m, c[2] * m, 1.0])
    };
    let angle = |j: usize| j as f64 * 360.0 / SEGMENTS as f64;
    let next = |j: usize| (j + 1) % SEGMENTS;

    // A fan for the innermost ring, then a strip between each pair.
    let centre = vertex(&mut mesh, 0.0, 0.0);
    let mut inner: Option<u32> = None;
    let mut last = 0.0;
    for r in ring_radii(edge, holes) {
        let mid = (last + r) * 0.5;
        last = r;
        let open = holes.iter().any(|&(lo, hi)| lo < mid && mid < hi);
        let first = mesh.vertices.len() as u32;
        for j in 0..SEGMENTS {
            vertex(&mut mesh, r, angle(j));
        }
        let outer = |j: usize| first + j as u32;
        for j in (0..SEGMENTS).filter(|_| !open) {
            let (o0, o1) = (outer(j), outer(next(j)));
            match inner {
                None => mesh.indices.extend([centre, o1, o0]),
                Some(base) => {
                    let (i0, i1) = (base + j as u32, base + next(j) as u32);
                    mesh.indices.extend([i0, o1, o0, i0, i1, o1]);
                }
            }
        }
        inner = Some(first);
    }
    mesh
}

/// Builds and uploads the ground.
pub fn upload(gpu: &mut ExtensionGpu, world_size: f64, holes: &[(f64, f64)]) {
    build(world_size, holes).upload(gpu, MESH, 0.92);
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::WORLD_SIZE;

    const EDGE: f64 = WORLD_SIZE * 0.5;

    fn near(a: [f32; 4], b: [f32; 4], eps: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= eps)
    }

    #[test]
    fn each_zone_wears_its_own_tint_away_from_its_edges() {
        for zone in Zone::ALL.into_iter().filter(|z| *z != Zone::Wilderness) {
            let (lo, hi) = zone.radii();
            let mid = (lo + hi) * 0.5;
            assert!(near(shade(mid, EDGE), tint(zone), 1e-6), "{zone}");
        }
    }

    #[test]
    fn boundaries_blend_without_a_seam() {
        // One server unit apart, no channel moves more than a sliver. The
        // steepest blend (gold into blue) moves about 0.004 a unit; a hard
        // edge would jump by 0.2.
        let mut prev = shade(0.0, EDGE);
        let mut r = 1.0;
        while r < EDGE * 1.5 {
            let c = shade(r, EDGE);
            assert!(near(c, prev, 0.005), "jump at r = {r}: {prev:?} -> {c:?}");
            prev = c;
            r += 1.0;
        }
    }

    #[test]
    fn a_boundary_is_halfway_between_its_zones() {
        let b = Zone::Urban.radii().1;
        let half = mix(tint(Zone::Urban), tint(Zone::Suburban), 0.5);
        assert!(near(shade(b, EDGE), half, 1e-6));
    }

    #[test]
    fn the_world_fades_to_the_background_at_its_edge() {
        let bg = hex(palette::BACKGROUND);
        assert!(near(shade(EDGE, EDGE), bg, 1e-6));
        assert!(near(shade(EDGE * 2.0, EDGE), bg, 1e-6));
        assert!(!near(shade(Zone::Rural.radii().0, EDGE), bg, 1e-3));
    }

    #[test]
    fn mottling_stays_within_bounds() {
        for i in 0..2000 {
            let p = [i as f32 * 3.7 - 3000.0, i as f32 * -2.3 + 1500.0];
            let m = mottle(p);
            assert!((1.0 - MOTTLE..=1.0 + MOTTLE).contains(&m), "{m}");
        }
    }

    #[test]
    fn the_disc_lies_flat_faces_up_and_reaches_past_the_corners() {
        let mesh = build(WORLD_SIZE, &[]);
        assert!(mesh.vertices.iter().all(|v| v.position[1] == layer::GROUND));
        assert!(mesh.vertices.iter().all(|v| v.normal == [0.0, 1.0, 0.0]));
        let reach = mesh
            .vertices
            .iter()
            .map(|v| v.position[0].hypot(v.position[2]))
            .fold(0.0, f32::max);
        let corner = (EDGE * std::f64::consts::SQRT_2) as f32 * crate::coords::S;
        assert!(reach > corner * 2.0, "reach {reach} m");
        assert_eq!(mesh.indices.len() % 3, 0);
        assert!(mesh
            .indices
            .iter()
            .all(|&i| (i as usize) < mesh.vertices.len()));
    }

    #[test]
    fn every_triangle_winds_counter_clockwise_from_above() {
        let mesh = build(WORLD_SIZE, &[]);
        for tri in mesh.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[tri[k] as usize].position);
            let (e1, e2) = ([b[0] - a[0], b[2] - a[2]], [c[0] - a[0], c[2] - a[2]]);
            // The y of (b - a) × (c - a): positive is counter-clockwise
            // seen from +Y with north (-Z) up the screen.
            let cross_y = e1[1] * e2[0] - e1[0] * e2[1];
            assert!(cross_y > 0.0, "{tri:?}");
        }
    }
}
