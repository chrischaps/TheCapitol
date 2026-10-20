//! The valleys' mist (W11, #204): at dawn and dusk a pale mist lies in the
//! wilds' hollows, on the valley floors and over the lakes, and the ridges
//! stand clear of it.
//!
//! - **Where.** The field is read every [`CELL`] samples. A cell is a
//!   hollow by how far it lies under the ground round it (the mean over
//!   [`REACH`] cells each way), and the mist lies thickest in the deepest,
//!   thins on the slopes and the high ground, and keeps to the wilds.
//! - **How.** Three sheets draped over the ground, each higher and fainter
//!   than the one under it ([`LAYERS`]), so from the side it has depth.
//!   Their vertex alpha is how thick the mist lies there, broken into
//!   drifts by a noise of its own on each sheet ([`drift`]). One mesh,
//!   [`MESH`], blended over the land and the water.
//! - **When.** The entity's opacity is the hour's (`daylight::Palette::
//!   mist`: thickest at dawn, some at dusk and through the night, none by
//!   day), cleared with the fog as the camera climbs.
//!
//! The mist is lit like any surface, so the low sun warms it and the night
//! leaves it grey.
//!
//! The mist casts no shadow (`material.cast_shadows = false`, #232), so it
//! leaves no shade on the ground at noon when it can't be seen.
//!
//! Sheets are a workaround: Flint's height fog does nothing (#233), and
//! with it the mist would be the hour's `fog_height_*` and fill the air.

use crate::coords::{layer, to_world};
use crate::world::ground::value_noise;
use crate::world::land::{sample_at, SAMPLES};
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{self, hex};
use capitol_protocol::rules::{CENTER_X, CENTER_Y, WILDERNESS_OUTER_RADIUS};
use flint_import::{AlphaMode, ImportedMaterial};
use flint_player::{ExtensionGpu, GroundField};

/// The mesh, and its entity.
pub const MESH: &str = "wild_mist";
/// Field samples (m) between the mist's vertices.
pub const CELL: usize = 6;
/// Cells each way the ground round a cell is averaged over: about 40 m.
const REACH: usize = 7;
/// How far (m) under the ground round it a cell must lie for mist to start,
/// and where it lies thickest.
const HOLLOW: (f32, f32) = (0.4, 4.0);
/// The heights (m over the plain) over which it thins out to nothing.
const HIGH: (f32, f32) = (16.0, 32.0);
/// The slopes (rise over run) over which it thins out, measured across
/// [`SLOPE_REACH`] cells each way, so a river's banks don't count as steep
/// and the mist lies over the water as well as beside it.
const STEEP: (f32, f32) = (0.3, 0.7);
const SLOPE_REACH: isize = 2;
/// The server radii over which it comes in at the ranges' foot, and goes
/// again past the world's edge.
const FROM: (f32, f32) = (3850.0, 4150.0);
const OUT: (f32, f32) = (WILDERNESS_OUTER_RADIUS as f32 - 200.0, WILDERNESS_OUTER_RADIUS as f32 + 300.0);
/// Each sheet's height (m) over the ground and how opaque it is where the
/// mist lies thickest.
pub const LAYERS: [(f32, f32); 3] = [(0.5, 0.42), (1.4, 0.28), (2.6, 0.16)];
/// The drifts' size (m): broad banks, broken finer.
const DRIFT_SCALE: (f32, f32) = (38.0, 13.0);
/// The noise (0 to 1) over which a drift goes from clear to whole.
const DRIFT_EDGE: (f32, f32) = (0.38, 0.68);

/// How much of the mist sheet `layer` holds at world (X, Z), 0 to 1: banks
/// and gaps, the banks shared by every sheet so a gap goes all the way
/// down, their edges broken differently on each, so the mist isn't one
/// flat veil.
pub fn drift(layer: usize, [x, z]: [f32; 2]) -> f32 {
    let o = 31.7 * layer as f32;
    let broad = value_noise(x / DRIFT_SCALE.0, z / DRIFT_SCALE.0);
    let fine = value_noise(x / DRIFT_SCALE.1 - o, z / DRIFT_SCALE.1 + 2.0 * o);
    smoothstep(DRIFT_EDGE, 0.7 * broad + 0.3 * fine)
}

fn smoothstep((lo, hi): (f32, f32), x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How thick the mist lies (0 to 1) on ground `rise` m over the plain, on
/// a `slope`, `hollow` m under the ground round it, at server radius `r`.
pub fn thickness(rise: f32, slope: f32, hollow: f32, r: f32) -> f32 {
    smoothstep(HOLLOW, hollow)
        * (1.0 - smoothstep(HIGH, rise))
        * (1.0 - smoothstep(STEEP, slope))
        * smoothstep(FROM, r)
        * (1.0 - smoothstep(OUT, r))
}

/// The mist's sheets over `field`.
pub fn build(field: &GroundField) -> MeshBuf {
    let heights = field.heightmap().heights();
    let n = (SAMPLES - 1) / CELL + 1;
    let at = |a: usize, b: usize| heights[(b * CELL) * SAMPLES + a * CELL];
    let ground: Vec<f32> = (0..n * n).map(|k| at(k % n, k / n)).collect();
    let mean = box_mean(&ground, n, REACH);
    let cell = CELL as f32 * (crate::world::land::TEXEL as f32) * crate::coords::S;
    let thick: Vec<f32> = (0..n * n)
        .map(|k| {
            let (a, b) = (k % n, k / n);
            let h = ground[k];
            let g = |da: isize, db: isize| {
                let aa = (a as isize + da).clamp(0, n as isize - 1) as usize;
                let bb = (b as isize + db).clamp(0, n as isize - 1) as usize;
                ground[bb * n + aa]
            };
            let s = SLOPE_REACH;
            let slope = ((g(s, 0) - g(-s, 0)).hypot(g(0, s) - g(0, -s))) / (2.0 * s as f32 * cell);
            let [x, y] = sample_at(a * CELL, b * CELL);
            let r = (x - CENTER_X).hypot(y - CENTER_Y) as f32;
            thickness(h - layer::GROUND, slope, mean[k] - h, r)
        })
        .collect();
    let colour = hex(palette::WILD_MIST);
    let mut mesh = MeshBuf::default();
    for (sheet, (over, opacity)) in LAYERS.into_iter().enumerate() {
        let base = mesh.vertices.len() as u32;
        let mut index = vec![u32::MAX; n * n];
        let mut vertex = |mesh: &mut MeshBuf, k: usize| {
            if index[k] == u32::MAX {
                let (a, b) = (k % n, k / n);
                let [x, y] = sample_at(a * CELL, b * CELL);
                let [wx, wz] = to_world(x, y);
                let alpha = thick[k] * opacity * drift(sheet, [wx, wz]);
                index[k] = mesh.vertex([wx, ground[k] + over, wz], UP, [colour[0], colour[1], colour[2], alpha]) - base;
            }
            base + index[k]
        };
        for b in 0..n - 1 {
            for a in 0..n - 1 {
                let corners = [b * n + a, b * n + a + 1, (b + 1) * n + a, (b + 1) * n + a + 1];
                if corners.iter().all(|&k| thick[k] <= 0.0) {
                    continue;
                }
                let [p00, p10, p01, p11] = corners.map(|k| vertex(&mut mesh, k));
                mesh.indices.extend([p00, p01, p10, p10, p01, p11]);
            }
        }
    }
    mesh
}

/// The mean of `values` (`n` × `n`) over a square `reach` cells each way,
/// clipped at the edges.
fn box_mean(values: &[f32], n: usize, reach: usize) -> Vec<f32> {
    let pass = |src: &[f32], along_rows: bool| -> Vec<f32> {
        let mut out = vec![0.0; n * n];
        for line in 0..n {
            let get = |t: usize| if along_rows { src[line * n + t] } else { src[t * n + line] };
            let mut sum = vec![0.0f64; n + 1];
            for t in 0..n {
                sum[t + 1] = sum[t] + get(t) as f64;
            }
            for t in 0..n {
                let (lo, hi) = (t.saturating_sub(reach), (t + reach + 1).min(n));
                let mean = ((sum[hi] - sum[lo]) / (hi - lo) as f64) as f32;
                if along_rows {
                    out[line * n + t] = mean;
                } else {
                    out[t * n + line] = mean;
                }
            }
        }
        out
    };
    pass(&pass(values, true), false)
}

/// Uploads the mist, blended: its vertex alpha is how thick it lies.
pub fn upload(gpu: &mut ExtensionGpu, mesh: &MeshBuf) {
    if mesh.indices.is_empty() {
        gpu.remove_mesh(MESH);
        return;
    }
    let material = ImportedMaterial {
        roughness: 1.0,
        alpha_mode: AlphaMode::Blend,
        ..ImportedMaterial::vertex_colored(MESH)
    };
    gpu.upload_mesh(MESH, &mesh.vertices, &mesh.indices, material);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::land::tests::fixture_field;

    #[test]
    fn the_mist_lies_in_the_wild_valleys_and_not_on_the_tops() {
        let mesh = build(fixture_field());
        let layer = mesh.vertices.len() / LAYERS.len();
        assert!(layer > 1000, "{layer} vertices a sheet");
        let mut thick = 0;
        for v in &mesh.vertices {
            let [x, y, z] = v.position;
            let [sx, sy] = crate::coords::to_server(x, z);
            let r = (sx - CENTER_X).hypot(sy - CENTER_Y);
            if v.color[3] > 0.0 {
                assert!(r > FROM.0 as f64, "mist at r {r:.0}, in the city");
                assert!(y < layer::GROUND + HIGH.1 + LAYERS[2].0 + 0.1, "mist {y:.1} m up, on the tops");
            }
            if v.color[3] > 0.2 {
                thick += 1;
            }
        }
        assert!(thick > 500, "only {thick} vertices where it lies thick");
    }

    #[test]
    fn each_sheet_is_fainter_than_the_one_under_it() {
        for w in LAYERS.windows(2) {
            assert!(w[1].0 > w[0].0 && w[1].1 < w[0].1, "{w:?}");
        }
        assert_eq!(thickness(0.0, 0.0, 5.0, 5000.0), 1.0);
        assert_eq!(thickness(0.0, 0.0, 0.2, 5000.0), 0.0, "no hollow");
        assert_eq!(thickness(40.0, 0.0, 5.0, 5000.0), 0.0, "on the tops");
        assert_eq!(thickness(0.0, 0.0, 5.0, 3000.0), 0.0, "in the city");
    }

    #[test]
    fn the_box_mean_of_a_flat_field_is_flat() {
        let n = 9;
        let flat = vec![2.5; n * n];
        assert!(box_mean(&flat, n, 3).iter().all(|&v| (v - 2.5).abs() < 1e-5));
        // A pit is shallower once averaged.
        let mut pit = flat.clone();
        pit[4 * n + 4] = 0.0;
        let mean = box_mean(&pit, n, 1);
        assert!(mean[4 * n + 4] > 2.0 && mean[4 * n + 4] < 2.5);
    }
}
