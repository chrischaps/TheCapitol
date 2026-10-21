//! `MeshBuf`: vertices and indices gathered on the CPU, then uploaded as a
//! runtime mesh through `ExtensionGpu` (H4).

use crate::coords::polar_to_world;
use flint_import::ImportedMaterial;
use flint_player::{ExtensionGpu, MeshOptions};
use flint_render::Vertex;

/// For a building's walls and roofs: rays meet it, so the camera keeps
/// out of it and a click can pick it (#293, Flint ADR 0105).
pub const RAY_TARGET: MeshOptions = MeshOptions { ray_target: true };

/// Straight up, for everything that lies flat on the ground.
pub const UP: [f32; 3] = [0.0, 1.0, 0.0];

#[derive(Default, Clone)]
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

    /// Adds a vertex with its own normal and returns its index.
    pub fn vertex(&mut self, position: [f32; 3], normal: [f32; 3], color: [f32; 4]) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(Vertex {
            position,
            normal,
            color,
            uv: [0.0, 0.0],
        });
        index
    }

    /// A flat-shaded quad `a b c d` (in order around its edge, either way)
    /// facing `normal`. The winding is chosen from the normal, so callers
    /// don't have to track it: the front face is counter-clockwise, and
    /// back faces are culled.
    pub fn quad(&mut self, corners: [[f32; 3]; 4], normal: [f32; 3], color: [f32; 4]) {
        let [a, b, c, d] = corners.map(|p| self.vertex(p, normal, color));
        if facing(corners[0], corners[1], corners[2], normal) {
            self.indices.extend([a, b, c, a, c, d]);
        } else {
            self.indices.extend([a, c, b, a, d, c]);
        }
    }

    /// A flat-shaded triangle fan from `centre` through each point of `rim`
    /// in turn, facing `normal`. Each triangle winds itself, as in `quad`.
    pub fn fan(&mut self, centre: [f32; 3], rim: &[[f32; 3]], normal: [f32; 3], color: [f32; 4]) {
        let c = self.vertex(centre, normal, color);
        let first = self.vertices.len() as u32;
        for &p in rim {
            self.vertex(p, normal, color);
        }
        for (k, w) in rim.windows(2).enumerate() {
            let (a, b) = (first + k as u32, first + k as u32 + 1);
            if facing(centre, w[0], w[1], normal) {
                self.indices.extend([c, a, b]);
            } else {
                self.indices.extend([c, b, a]);
            }
        }
    }

    /// Adds all of `other`'s triangles to this mesh.
    pub fn append(&mut self, other: MeshBuf) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
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

    /// Uploads under `name`, opaque and coloured per vertex. An empty mesh
    /// removes any earlier one of that name instead (wgpu can't make an
    /// empty buffer), so nothing is drawn until there's something to draw.
    pub fn upload(&self, gpu: &mut ExtensionGpu, name: &str, roughness: f32) {
        self.upload_with(gpu, name, roughness, MeshOptions::default());
    }

    /// [`Self::upload`], with options: [`RAY_TARGET`] for a building.
    pub fn upload_with(&self, gpu: &mut ExtensionGpu, name: &str, roughness: f32, options: MeshOptions) {
        self.upload_material_with(
            gpu,
            ImportedMaterial {
                roughness,
                ..ImportedMaterial::vertex_colored(name)
            },
            options,
        );
    }

    /// Uploads under the material's name, with that material. Every mesh
    /// can carry light: per-vertex emission is turned on, so a vertex with
    /// alpha 0 (`palette::glow`) gives off its colour, and the rest (alpha
    /// 1) are lit as usual.
    pub fn upload_material(&self, gpu: &mut ExtensionGpu, material: ImportedMaterial) {
        self.upload_material_with(gpu, material, MeshOptions::default());
    }

    /// [`Self::upload_material`], with options: [`RAY_TARGET`] for a building.
    pub fn upload_material_with(&self, gpu: &mut ExtensionGpu, material: ImportedMaterial, options: MeshOptions) {
        let material = ImportedMaterial {
            vertex_emission: true,
            ..material
        };
        let name = material.name.clone();
        if self.indices.is_empty() {
            gpu.remove_mesh(&name);
            return;
        }
        gpu.upload_mesh_with(&name, &self.vertices, &self.indices, material, options);
    }
}

/// Whether `v` is light (`palette::glow`) rather than a lit surface.
#[cfg(test)]
pub fn is_light(v: &Vertex) -> bool {
    v.color[3] < 1.0
}

/// Whether the triangle `a b c` is front-facing (counter-clockwise) when
/// seen from the side `normal` points to.
pub fn facing(a: [f32; 3], b: [f32; 3], c: [f32; 3], normal: [f32; 3]) -> bool {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    cross[0] * normal[0] + cross[1] * normal[1] + cross[2] * normal[2] > 0.0
}

/// Looking at a built mesh from above, for tests.
#[cfg(test)]
pub mod probe {
    use super::MeshBuf;
    use flint_render::Vertex;

    pub fn triangles(m: &MeshBuf) -> impl Iterator<Item = [Vertex; 3]> + '_ {
        m.indices
            .as_chunks::<3>().0.iter()
            .map(|t| [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]))
    }

    /// Whether world (x, z) is under some triangle of `m`, seen from above.
    pub fn covered(m: &MeshBuf, [x, z]: [f32; 2]) -> bool {
        triangles(m).any(|tri| {
            let [a, b, c] = tri.map(|v| [v.position[0], v.position[2]]);
            if x < a[0].min(b[0]).min(c[0]) || x > a[0].max(b[0]).max(c[0]) {
                return false;
            }
            let side =
                |p: [f32; 2], q: [f32; 2]| (q[0] - p[0]) * (z - p[1]) - (q[1] - p[1]) * (x - p[0]);
            let (d0, d1, d2) = (side(a, b), side(b, c), side(c, a));
            (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_faces_its_normal_whichever_way_its_corners_run() {
        let square = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ];
        let mut reversed = square;
        reversed.reverse();
        for corners in [square, reversed] {
            for normal in [UP, [0.0, -1.0, 0.0]] {
                let mut m = MeshBuf::default();
                m.quad(corners, normal, [1.0; 4]);
                for tri in m.indices.as_chunks::<3>().0.iter().map(|t| [t[0], t[1], t[2]]) {
                    let p = tri.map(|i| m.vertices[i as usize].position);
                    assert!(facing(p[0], p[1], p[2], normal));
                }
            }
        }
    }

    #[test]
    fn the_annulus_faces_up() {
        let mut m = MeshBuf::default();
        m.annulus(10.0, 20.0, 0.0, [1.0; 4], &[]);
        for tri in m.indices.as_chunks::<3>().0.iter().map(|t| [t[0], t[1], t[2]]) {
            let p = tri.map(|i| m.vertices[i as usize].position);
            assert!(facing(p[0], p[1], p[2], UP));
        }
    }
}
