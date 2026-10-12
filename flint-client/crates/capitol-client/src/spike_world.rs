//! The spike's city, built through the runtime mesh hook (H4): zone-tinted
//! ground, the moat and both canals with their real bridge gaps, and two
//! compass markers. The water is rebuilt every frame, breathing slowly
//! like a tide, which exercises mesh replacement in the live client.
//! The water comes from the terrain (P3.1); P3.2–P3.7 replace the rest
//! with the world built from `/terrain`.

use crate::coords::{polar_to_world, S};
use capitol_protocol::rules::Zone;
use capitol_protocol::terrain::WaterRing;
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;
use flint_render::{srgb_to_linear, Vertex};

/// A hex colour token (sRGB, as in the React client) as linear RGBA.
fn hex(h: u32) -> [f32; 4] {
    let c = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
    srgb_to_linear([c(16), c(8), c(0), 1.0])
}

/// A zone colour pulled most of the way toward the background, so the
/// ground reads as tinted dark rather than as a flat fill.
fn ground_tint(c: [f32; 4], amount: f32) -> [f32; 4] {
    let bg = hex(0x0a0a0f);
    let m = |i: usize| bg[i] + (c[i] - bg[i]) * amount;
    [m(0), m(1), m(2), 1.0]
}

#[derive(Default)]
struct MeshBuf {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl MeshBuf {
    /// A flat ring at height `y` from `r_in` to `r_out` server units,
    /// skipping each `(centre_deg, half_width_deg)` gap. `r_in = 0` gives a
    /// disc. Triangles wind counter-clockwise seen from above (+Y).
    fn annulus(&mut self, r_in: f64, r_out: f64, y: f32, color: [f32; 4], gaps: &[(f64, f64)]) {
        const SEGMENTS: usize = 360;
        let in_gap = |deg: f64| {
            gaps.iter().any(|&(c, h)| {
                let d = (deg - c + 540.0).rem_euclid(360.0) - 180.0;
                d.abs() < h
            })
        };
        let vertex = |r: f64, deg: f64| {
            let [x, z] = polar_to_world(r, deg);
            Vertex {
                position: [x, y, z],
                normal: [0.0, 1.0, 0.0],
                color,
                uv: [0.0, 0.0],
            }
        };
        for i in 0..SEGMENTS {
            let a0 = i as f64 * 360.0 / SEGMENTS as f64;
            let a1 = (i + 1) as f64 * 360.0 / SEGMENTS as f64;
            if in_gap((a0 + a1) * 0.5) {
                continue;
            }
            let base = self.vertices.len() as u32;
            self.vertices.extend([
                vertex(r_in, a0),
                vertex(r_out, a0),
                vertex(r_in, a1),
                vertex(r_out, a1),
            ]);
            let (i0, o0, i1, o1) = (base, base + 1, base + 2, base + 3);
            self.indices.extend([i0, o1, o0, i0, i1, o1]);
        }
    }

    /// Uploads under `name`. An empty mesh is skipped (wgpu can't make an
    /// empty buffer), so nothing is drawn until there's something to draw.
    fn upload(&self, gpu: &mut ExtensionGpu, name: &str, roughness: f32) {
        if self.indices.is_empty() {
            return;
        }
        let material = ImportedMaterial {
            roughness,
            ..ImportedMaterial::vertex_colored(name)
        };
        gpu.upload_mesh(name, &self.vertices, &self.indices, material);
    }
}

/// Upload the ground and the compass markers. Called once, on the first
/// frame the renderer exists.
pub fn upload_static(gpu: &mut ExtensionGpu) {
    // Ground: each zone tinted toward the dark, fading out to the edge.
    let mut ground = MeshBuf::default();
    for (zone, colour, amount) in [
        (Zone::Capitol, 0xc9a227, 0.40),
        (Zone::TradeDistrict, 0x3b82f6, 0.10),
        (Zone::GuildDistrict, 0x8b5cf6, 0.09),
        (Zone::Urban, 0x64748b, 0.10),
        (Zone::Suburban, 0x22c55e, 0.05),
        (Zone::Rural, 0x84cc16, 0.03),
        (Zone::Wilderness, 0x1a1a24, 1.0),
    ] {
        let (r_in, r_out) = zone.radii();
        ground.annulus(r_in, r_out, 0.0, ground_tint(hex(colour), amount), &[]);
    }
    ground.upload(gpu, "spike_ground", 0.9);

    // Compass markers for checking handedness: east (angle 0) in the
    // in-range orange, north (angle 270) in the text colour.
    for (name, angle, color) in [
        ("spike_marker_east", 0.0, hex(0xf97316)),
        ("spike_marker_north", 270.0, hex(0xe0e0e0)),
    ] {
        let mut marker = MeshBuf::default();
        let [cx, cz] = polar_to_world(400.0, angle);
        marker.annulus(0.0, 30.0, 0.04, color, &[]);
        for v in &mut marker.vertices {
            v.position[0] += cx;
            v.position[2] += cz;
        }
        marker.upload(gpu, name, 0.6);
    }
}

/// Rebuild the terrain's water for game time `t`: the rings swell and ease
/// back by about a metre over a slow twelve-second tide, a hair above the
/// ground, open at each bridge.
pub fn upload_water(gpu: &mut ExtensionGpu, t: f64, rings: &[WaterRing]) {
    let tide = (t * std::f64::consts::TAU / 12.0).sin();
    // The React water colour reads too close to the Trade tint once lit,
    // so this sits between it and the highlight rgba(100, 180, 220).
    let colour = hex(0x4696c8);
    let mut water = MeshBuf::default();
    for ring in rings {
        let width = ring.width + tide * 0.5 / S as f64;
        let gaps: Vec<(f64, f64)> = ring
            .bridges
            .iter()
            .map(|b| {
                let half = ((b.width * 0.5) / ring.radius).asin().to_degrees();
                (b.angle_degrees, half)
            })
            .collect();
        water.annulus(
            ring.radius - width * 0.5,
            ring.radius + width * 0.5,
            0.02,
            colour,
            &gaps,
        );
    }
    water.upload(gpu, "spike_water", 0.25);
}
