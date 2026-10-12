//! `MeshBuf`: vertices and indices gathered on the CPU, then uploaded as a
//! runtime mesh through `ExtensionGpu` (H4).

use crate::coords::polar_to_world;
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;
use flint_render::Vertex;

/// Straight up, for everything that lies flat on the ground.
pub const UP: [f32; 3] = [0.0, 1.0, 0.0];

#[derive(Default)]
pub struct MeshBuf {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshBuf {
    /// Adds a flat vertex at world (X, `y`, Z) and returns its index.
    pub fn flat_vertex(&mut self, [x, z]: [f32; 2], y: f32, color: [f32; 4]) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(Vertex {
            position: [x, y, z],
            normal: UP,
            color,
            uv: [0.0, 0.0],
        });
        index
    }

    /// A flat ring at height `y` from `r_in` to `r_out` server units,
    /// skipping each `(centre_deg, half_width_deg)` gap. `r_in = 0` gives a
    /// disc. Triangles wind counter-clockwise seen from above (+Y).
    pub fn annulus(&mut self, r_in: f64, r_out: f64, y: f32, color: [f32; 4], gaps: &[(f64, f64)]) {
        const SEGMENTS: usize = 360;
        let in_gap = |deg: f64| {
            gaps.iter().any(|&(c, h)| {
                let d = (deg - c + 540.0).rem_euclid(360.0) - 180.0;
                d.abs() < h
            })
        };
        for i in 0..SEGMENTS {
            let a0 = i as f64 * 360.0 / SEGMENTS as f64;
            let a1 = (i + 1) as f64 * 360.0 / SEGMENTS as f64;
            if in_gap((a0 + a1) * 0.5) {
                continue;
            }
            let i0 = self.flat_vertex(polar_to_world(r_in, a0), y, color);
            let o0 = self.flat_vertex(polar_to_world(r_out, a0), y, color);
            let i1 = self.flat_vertex(polar_to_world(r_in, a1), y, color);
            let o1 = self.flat_vertex(polar_to_world(r_out, a1), y, color);
            self.indices.extend([i0, o1, o0, i0, i1, o1]);
        }
    }

    /// Uploads under `name`. An empty mesh is skipped (wgpu can't make an
    /// empty buffer), so nothing is drawn until there's something to draw.
    pub fn upload(&self, gpu: &mut ExtensionGpu, name: &str, roughness: f32) {
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
