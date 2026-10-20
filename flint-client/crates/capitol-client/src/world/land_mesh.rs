//! The land's meshes (W9, #202): the wilds' height field drawn in chunks,
//! each at the detail its distance from the camera needs.
//!
//! - **Chunks.** The field (`land`) is cut into [`CHUNKS`]² squares of
//!   [`CHUNK`] quads (64 m). Those with no land (inside `land::FROM`, or
//!   out past where the ranges settle) have no mesh: the flat disc
//!   (`ground`) is there.
//! - **Detail.** Each chunk has one runtime mesh, `land_<row>_<col>`, built
//!   at one of [`LODS`] strides (1, 2, 4 or 8 samples a quad) from how far
//!   the camera is, and built again when that changes. A mesh is a copy out
//!   of the field plus a colour a vertex, so it takes well under a
//!   millisecond; a frame builds at most [`BUDGET`], nearest first.
//! - **Seams.** Where two chunks at different strides meet, their edges
//!   needn't agree. Each chunk hangs a skirt down from its edges, deeper
//!   at coarser strides, so no crack shows through. Where the land meets
//!   the flat disc, it dips a few centimetres under it, so the two never
//!   fight for the same pixels.
//! - **The tint** follows the slope and the height: the disc's hill meadow
//!   at the foot, bare rock on the steeps, scree on the high slopes and snow
//!   on the tops, all mottled as the disc is. By the water (W10) the banks
//!   are dark with moss, the beds silt, and the fords pale gravel
//!   ([`Shore`]).
//! - **The air** (W11): the further the land from the camera, the more it
//!   takes the haze's slate blue ([`View::haze`]), so from the city the
//!   ranges stand blue on the horizon in every direction, and from a ridge
//!   the far side of the ring does. The haze is baked into the colours, so
//!   a chunk is built again when the haze it was built with has drifted
//!   [`HAZE_STEP`] from the camera's. It clears with the fog as the camera
//!   climbs (`atmosphere::fog_scale`), so the map stays flat. A workaround:
//!   Flint's fog has one colour, the sky's horizon, so it can't turn far
//!   things blue itself. Only the land is hazed; what stands on it isn't.
//!   A distance tint in the fog removes this (#233).

use crate::coords::{layer, to_world, S};
use crate::world::ground::{self, mottle};
use crate::world::land::{sample_at, CHUNK, CHUNKS, FROM, OUTER, SAMPLES};
use crate::world::mesh::{facing, MeshBuf};
use crate::world::palette::{self, hex, mix};
use capitol_protocol::rules::{CENTER_X, CENTER_Y, WILDERNESS_OUTER_RADIUS};
use flint_player::{ExtensionGpu, GroundField};
use std::sync::Arc;

/// How many strides a chunk can be drawn at: 1, 2, 4 and 8 samples a quad.
pub const LODS: usize = 4;
/// Up to these distances (m, from the camera to the chunk's nearest point)
/// a chunk is drawn at each stride; past the last, at the coarsest.
const LOD_DISTANCE: [f32; LODS - 1] = [110.0, 230.0, 460.0];
/// How much nearer than its threshold the camera must come before a chunk
/// goes finer again, so one on the line doesn't flicker between two.
const HYSTERESIS: f32 = 0.9;
/// Chunks built a frame, after the first.
pub const BUDGET: usize = 10;
/// How far a vertex off the land (inside `land::FROM`, past the settling)
/// dips under the flat disc, in metres.
const DIP: f32 = 0.03;
/// A skirt's depth in metres at stride 1; it doubles with the stride.
const SKIRT: f32 = 0.6;
/// How much deeper than the carve the beds are drawn: this many times
/// its cut again. The carve is in server units, so a big river's bed is
/// 30 cm deep in metres and a stream's a few: the water's shore foam
/// (14 cm of depth) would cover all of it, and there'd be nothing under
/// the surface to see. Drawn three times as deep, the foam keeps to the
/// banks and the water has depth. Only the drawing: what stands in the
/// water stands on the carve (`land::rise`), a little higher. A
/// workaround: a per-body shore foam, or a deeper carve, removes it
/// (#230).
pub const DEEPEN: f32 = 2.0;

/// How strongly each wild colour shows, like `ground`'s zones.
const ROCK_STRENGTH: f32 = 0.2;
const SCREE_STRENGTH: f32 = 0.2;
const SNOW_STRENGTH: f32 = 0.5;
/// Slopes (rise over run) over which the ground turns to rock: well short
/// of the cliffs you can't climb (`rules::ground::CLIFF_SLOPE`, 1.2).
const ROCK_SLOPE: (f32, f32) = (0.55, 0.95);
/// Heights in metres over which scree comes in, then snow, and the slope
/// past which snow won't lie.
const SCREE_HEIGHT: (f32, f32) = (26.0, 40.0);
const SNOW_HEIGHT: (f32, f32) = (46.0, 58.0);
const SNOW_SLOPE: (f32, f32) = (0.6, 1.0);
/// How wet (m, `land::Sampled::wet`) the bank must be to read as moss,
/// and how deep a cut (m) reads as the bed's silt.
const MOSS_WET: (f32, f32) = (0.03, 0.45);
const SILT_CUT: (f32, f32) = (0.01, 0.08);
const MOSS_STRENGTH: f32 = 0.3;
const SILT_STRENGTH: f32 = 0.45;
const GRAVEL_STRENGTH: f32 = 0.3;
/// How much darker the wettest bank is than dry ground.
const WET_DARKEN: f32 = 0.22;
/// How much of the haze the farthest land takes, and the distances (m from
/// the camera) over which it comes in. The ranges are 320 to 600 m from
/// the quays, and the far side of the ring 900 m from a ridge.
const HAZE_MAX: f32 = 0.7;
const HAZE_DISTANCE: (f32, f32) = (120.0, 800.0);
/// The server radii over which the haze comes in at the ranges' foot, so
/// the rural plain's land meets the city's flat disc with no line, and
/// fades out again as the land settles past the world's edge to meet the
/// disc there.
const HAZE_FROM: (f32, f32) = (3800.0, 4200.0);
const HAZE_OUT: (f32, f32) = (WILDERNESS_OUTER_RADIUS as f32, OUTER as f32);
/// How far the haze at a chunk may drift from what it was built with
/// before it's built again: under a step the eye can see.
pub const HAZE_STEP: f32 = 0.03;

fn smoothstep((lo, hi): (f32, f32), x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn pulled(token: u32, strength: f32) -> [f32; 4] {
    mix(hex(palette::BACKGROUND), hex(token), strength)
}

/// The land's colour (linear) at server radius `r`, `rise` metres above the
/// flat ground on a `slope` (rise over run), at world (X, Z) `at`.
pub fn tint(r: f64, rise: f32, slope: f32, at: [f32; 2]) -> [f32; 4] {
    let base = ground::shade(r);
    let rock = smoothstep(ROCK_SLOPE, slope);
    let scree = smoothstep(SCREE_HEIGHT, rise);
    let snow = smoothstep(SNOW_HEIGHT, rise) * (1.0 - smoothstep(SNOW_SLOPE, slope));
    let mut c = mix(base, pulled(palette::WILD_SCREE, SCREE_STRENGTH), scree);
    c = mix(c, pulled(palette::WILD_ROCK, ROCK_STRENGTH), rock);
    c = mix(c, pulled(palette::WILD_SNOW, SNOW_STRENGTH), snow);
    let m = mottle(at);
    [c[0] * m, c[1] * m, c[2] * m, 1.0]
}

/// Where the land is seen from, for the haze: the camera's eye (world
/// metres) and how much of the air shows (`atmosphere::fog_scale`: 1 low
/// down, 0 at the map).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub eye: [f32; 3],
    pub air: f32,
}

impl View {
    /// No air: the land in its own colours.
    #[cfg(test)]
    pub const CLEAR: View = View {
        eye: [0.0; 3],
        air: 0.0,
    };

    /// How much of the haze the land at world `at`, server radius `r`,
    /// takes: 0 near the camera, up to [`HAZE_MAX`] far off.
    pub fn haze(&self, at: [f32; 3], r: f64) -> f32 {
        if self.air <= 0.0 {
            return 0.0;
        }
        let [dx, dy, dz] = [0, 1, 2].map(|k| at[k] - self.eye[k]);
        let d = (dx * dx + dy * dy + dz * dz).sqrt();
        let r = r as f32;
        HAZE_MAX
            * self.air
            * smoothstep(HAZE_DISTANCE, d)
            * smoothstep(HAZE_FROM, r)
            * (1.0 - smoothstep(HAZE_OUT, r))
    }
}

/// `colour` seen through `haze` of the air.
pub fn hazed(colour: [f32; 4], haze: f32) -> [f32; 4] {
    if haze <= 0.0 {
        return colour;
    }
    mix(colour, hex(palette::WILD_HAZE), haze)
}

/// The water's mark on the ground: how deep it cut each sample and how wet
/// each is (`land::Sampled`), and where the fords run shallow over gravel.
#[derive(Debug, Clone, Default)]
pub struct Shore {
    pub cut: Vec<f32>,
    pub wet: Vec<f32>,
    /// Each ford's centre (world X, Z) and radius in metres.
    pub fords: Vec<([f32; 2], f32)>,
}

impl Shore {
    /// The cut and the wetness at sample (i, j), in metres.
    fn at(&self, i: usize, j: usize) -> (f32, f32) {
        let k = j * SAMPLES + i;
        (
            self.cut.get(k).copied().unwrap_or(0.0),
            self.wet.get(k).copied().unwrap_or(0.0),
        )
    }

    /// How much of a ford's gravel shows at world (X, Z), 0 to 1.
    fn gravel(&self, [x, z]: [f32; 2]) -> f32 {
        self.fords
            .iter()
            .map(|&([fx, fz], r)| 1.0 - smoothstep((0.6 * r, 1.1 * r), (x - fx).hypot(z - fz)))
            .fold(0.0, f32::max)
    }
}

/// The tint `c` (before mottling) by the water: moss up the wet banks,
/// silt in the beds, gravel at a ford.
pub fn shore_tint(c: [f32; 4], cut: f32, wet: f32, gravel: f32) -> [f32; 4] {
    let moss = smoothstep(MOSS_WET, wet);
    let silt = smoothstep(SILT_CUT, cut);
    let mut c = mix(c, pulled(palette::WILD_MOSS, MOSS_STRENGTH), 0.6 * moss);
    c = mix(c, pulled(palette::WILD_SILT, SILT_STRENGTH), silt);
    c = mix(c, pulled(palette::WILD_GRAVEL, GRAVEL_STRENGTH), gravel * moss);
    let dark = 1.0 - WET_DARKEN * moss * (1.0 - gravel);
    [c[0] * dark, c[1] * dark, c[2] * dark, c[3]]
}

/// A chunk by its row (along +Z) and column (along +X).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chunk {
    pub row: usize,
    pub col: usize,
}

impl Chunk {
    pub fn mesh_name(self) -> String {
        format!("land_{:02}_{:02}", self.row, self.col)
    }

    /// Its middle sample, (i, j).
    fn middle(self) -> (usize, usize) {
        (self.col * CHUNK + CHUNK / 2, self.row * CHUNK + CHUNK / 2)
    }

    /// The server radius range its samples span.
    fn radii(self) -> (f64, f64) {
        let [x0, y0] = sample_at(self.col * CHUNK, self.row * CHUNK);
        let side = CHUNK as f64 * crate::world::land::TEXEL;
        let near = |lo: f64, c: f64| (c - lo).clamp(0.0, side) + lo;
        let nx = near(x0, CENTER_X) - CENTER_X;
        let ny = near(y0, CENTER_Y) - CENTER_Y;
        let fx = (x0 - CENTER_X).abs().max((x0 + side - CENTER_X).abs());
        let fy = (y0 - CENTER_Y).abs().max((y0 + side - CENTER_Y).abs());
        (nx.hypot(ny), fx.hypot(fy))
    }

    /// Whether any of its quads is on the land.
    pub fn has_land(self) -> bool {
        let (lo, hi) = self.radii();
        hi >= FROM && lo <= OUTER
    }

    /// Every chunk with land on it.
    pub fn all() -> impl Iterator<Item = Chunk> {
        (0..CHUNKS)
            .flat_map(|row| (0..CHUNKS).map(move |col| Chunk { row, col }))
            .filter(|c| c.has_land())
    }
}

/// The field's world y at sample (i, j).
fn y_at(field: &GroundField, i: usize, j: usize) -> f32 {
    field.heightmap().heights()[j * SAMPLES + i]
}

/// The server radius of sample (i, j).
fn radius_at(i: usize, j: usize) -> f64 {
    let [x, y] = sample_at(i, j);
    (x - CENTER_X).hypot(y - CENTER_Y)
}

/// `chunk` at stride `1 << lod`, seen from `view`: its quads on the land,
/// and a skirt down from every edge of them that's on the chunk's rim.
pub fn build(field: &GroundField, shore: &Shore, chunk: Chunk, lod: usize, view: &View) -> MeshBuf {
    let step = 1usize << lod;
    let n = CHUNK / step + 1;
    let texel = (crate::world::land::TEXEL as f32) * S;
    let (i0, j0) = (chunk.col * CHUNK, chunk.row * CHUNK);
    let last = SAMPLES - 1;
    let mut mesh = MeshBuf::default();
    // Every vertex of the grid, then the quads that are on the land.
    for b in 0..n {
        for a in 0..n {
            let (i, j) = (i0 + a * step, j0 + b * step);
            let [x, y] = sample_at(i, j);
            let at = to_world(x, y);
            let r = radius_at(i, j);
            let h = y_at(field, i, j);
            // The slope over this stride, so a coarse chunk's normals are
            // its own ground's, not its samples' alone.
            let d = |di: isize, dj: isize| {
                let ii = (i as isize + di).clamp(0, last as isize) as usize;
                let jj = (j as isize + dj).clamp(0, last as isize) as usize;
                y_at(field, ii, jj)
            };
            let s = step as isize;
            let gx = (d(s, 0) - d(-s, 0)) / (2.0 * step as f32 * texel);
            let gz = (d(0, s) - d(0, -s)) / (2.0 * step as f32 * texel);
            let len = (gx * gx + gz * gz + 1.0).sqrt();
            let normal = [-gx / len, 1.0 / len, -gz / len];
            let slope = gx.hypot(gz);
            let rise = h - layer::GROUND;
            let off = !(FROM..=OUTER).contains(&r);
            let (cut, wet) = shore.at(i, j);
            let y = if off { h - DIP } else { h - DEEPEN * cut };
            let dry = tint(r, rise, slope, at);
            let colour = if wet > 0.0 || cut > 0.0 {
                let m = mottle(at);
                let c = shore_tint([dry[0] / m, dry[1] / m, dry[2] / m, 1.0], cut, wet, shore.gravel(at));
                [c[0] * m, c[1] * m, c[2] * m, 1.0]
            } else {
                dry
            };
            let colour = hazed(colour, view.haze([at[0], h, at[1]], r));
            mesh.vertex([at[0], y, at[1]], normal, colour);
        }
    }
    let v = |a: usize, b: usize| (b * n + a) as u32;
    let on = |a: usize, b: usize| {
        let corners = [(a, b), (a + 1, b), (a, b + 1), (a + 1, b + 1)]
            .map(|(a, b)| radius_at(i0 + a * step, j0 + b * step));
        let hi = corners.iter().copied().fold(0.0, f64::max);
        let lo = corners.iter().copied().fold(f64::INFINITY, f64::min);
        hi >= FROM && lo <= OUTER
    };
    let quads = n - 1;
    let mut land = vec![false; quads * quads];
    for b in 0..quads {
        for a in 0..quads {
            if !on(a, b) {
                continue;
            }
            land[b * quads + a] = true;
            let (p00, p10, p01, p11) = (v(a, b), v(a + 1, b), v(a, b + 1), v(a + 1, b + 1));
            mesh.indices.extend([p00, p01, p10, p10, p01, p11]);
        }
    }
    // The skirt: each rim edge of a land quad, hung down and facing out.
    let depth = SKIRT * step as f32;
    let skirt = |mesh: &mut MeshBuf, p: u32, q: u32, out: [f32; 3]| {
        let (vp, vq) = (mesh.vertices[p as usize], mesh.vertices[q as usize]);
        let low = |v: flint_render::Vertex| {
            let mut v = v;
            v.position[1] -= depth;
            v
        };
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend([low(vp), low(vq)]);
        let (lp, lq) = (base, base + 1);
        let [a, b, c] = [vp.position, vq.position, mesh.vertices[lq as usize].position];
        if facing(a, b, c, out) {
            mesh.indices.extend([p, q, lq, p, lq, lp]);
        } else {
            mesh.indices.extend([p, lq, q, p, lp, lq]);
        }
    };
    for k in 0..quads {
        if land[k] {
            skirt(&mut mesh, v(k, 0), v(k + 1, 0), [0.0, 0.0, -1.0]);
        }
        if land[(quads - 1) * quads + k] {
            skirt(&mut mesh, v(k, quads), v(k + 1, quads), [0.0, 0.0, 1.0]);
        }
        if land[k * quads] {
            skirt(&mut mesh, v(0, k), v(0, k + 1), [-1.0, 0.0, 0.0]);
        }
        if land[k * quads + quads - 1] {
            skirt(&mut mesh, v(quads, k), v(quads, k + 1), [1.0, 0.0, 0.0]);
        }
    }
    mesh
}

/// The stride a chunk wants with the camera `distance` m from it, given the
/// one it has.
pub fn lod_for(distance: f32, had: Option<usize>) -> usize {
    let plain = LOD_DISTANCE
        .iter()
        .position(|&d| distance < d)
        .unwrap_or(LODS - 1);
    match had {
        // Going finer only once well inside the line.
        Some(had) if plain < had => {
            let line = LOD_DISTANCE[had - 1] * HYSTERESIS;
            if distance < line {
                plain
            } else {
                had
            }
        }
        _ => plain,
    }
}

/// A chunk as drawn: the stride (`None` before it's first built) and the
/// haze at its middle it was built with.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    chunk: Chunk,
    lod: Option<usize>,
    haze: f32,
}

/// The land's meshes as drawn: the field they're cut from, and each
/// chunk's stride and haze.
#[derive(Default)]
pub struct LandMeshes {
    field: Option<Arc<GroundField>>,
    shore: Arc<Shore>,
    chunks: Vec<Drawn>,
}

impl LandMeshes {
    /// A new field, and the water's mark on it: every chunk is built again.
    pub fn set_field(&mut self, field: Arc<GroundField>, shore: Arc<Shore>) {
        self.field = Some(field);
        self.shore = shore;
        self.chunks = Chunk::all()
            .map(|chunk| Drawn {
                chunk,
                lod: None,
                haze: 0.0,
            })
            .collect();
    }

    /// Builds the chunks whose stride the camera changes, or whose haze
    /// has drifted [`HAZE_STEP`] from what it was built with, nearest
    /// first: all of them the first time, then at most [`BUDGET`] a frame.
    /// Returns the mesh names first built, which need an entity.
    pub fn update(&mut self, gpu: &mut ExtensionGpu, view: View) -> Vec<String> {
        let Some(field) = self.field.clone() else {
            return Vec::new();
        };
        let first = self.chunks.iter().all(|d| d.lod.is_none());
        let mut wanted: Vec<(f32, usize, usize, f32)> = self
            .chunks
            .iter()
            .enumerate()
            .filter_map(|(k, drawn)| {
                let d = distance(&field, drawn.chunk, view.eye);
                let lod = lod_for(d, drawn.lod);
                let (i, j) = drawn.chunk.middle();
                let [x, z] = to_world_at(i, j);
                let haze = view.haze([x, y_at(&field, i, j), z], radius_at(i, j));
                let stale = (haze - drawn.haze).abs() > HAZE_STEP;
                (Some(lod) != drawn.lod || stale).then_some((d, k, lod, haze))
            })
            .collect();
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        if !first {
            wanted.truncate(BUDGET);
        }
        let mut new = Vec::new();
        for (_, k, lod, haze) in wanted {
            let drawn = &mut self.chunks[k];
            let name = drawn.chunk.mesh_name();
            build(&field, &self.shore, drawn.chunk, lod, &view).upload(gpu, &name, ground::ROUGHNESS);
            if drawn.lod.is_none() {
                new.push(name);
            }
            drawn.lod = Some(lod);
            drawn.haze = haze;
        }
        new
    }
}

/// World (X, Z) of sample (i, j).
fn to_world_at(i: usize, j: usize) -> [f32; 2] {
    let [x, y] = sample_at(i, j);
    to_world(x, y)
}

/// From `eye` (world metres) to the nearest point of `chunk`'s box.
fn distance(field: &GroundField, chunk: Chunk, eye: [f32; 3]) -> f32 {
    let [x0, y0] = sample_at(chunk.col * CHUNK, chunk.row * CHUNK);
    let [wx, wz] = to_world(x0, y0);
    let side = CHUNK as f32 * (crate::world::land::TEXEL as f32) * S;
    let [lo, hi] = field.y_range();
    let gap = |v: f32, a: f32, b: f32| (a - v).max(v - b).max(0.0);
    let dx = gap(eye[0], wx, wx + side);
    let dz = gap(eye[2], wz, wz + side);
    let dy = gap(eye[1], lo, hi);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::land::tests::fixture_field;
    use crate::world::land::{rise_on, TEXEL};

    fn polar(r: f64, deg: f64) -> [f32; 2] {
        crate::coords::polar_to_world(r, deg)
    }

    #[test]
    fn the_chunks_cover_the_wilds_and_leave_the_city_to_the_disc() {
        let all: Vec<_> = Chunk::all().collect();
        assert!(all.len() > 200 && all.len() < CHUNKS * CHUNKS, "{}", all.len());
        let centre = Chunk { row: CHUNKS / 2, col: CHUNKS / 2 };
        assert!(!centre.has_land());
        // Every point of the wilds is in a chunk with land.
        for deg in (0..360).step_by(5) {
            for r in [FROM + 1.0, 5000.0, 6000.0, OUTER - 1.0] {
                let [x, z] = polar(r, deg as f64);
                let [sx, sy] = crate::coords::to_server(x, z);
                let col = ((sx - sample_at(0, 0)[0]) / (CHUNK as f64 * TEXEL)) as usize;
                let row = ((sy - sample_at(0, 0)[1]) / (CHUNK as f64 * TEXEL)) as usize;
                assert!(Chunk { row, col }.has_land(), "r {r} at {deg}");
            }
        }
    }

    #[test]
    fn every_stride_faces_up_and_its_skirts_face_out() {
        let field = fixture_field();
        for chunk in [Chunk { row: 2, col: 12 }, Chunk { row: 12, col: 4 }, Chunk { row: 20, col: 19 }] {
            assert!(chunk.has_land());
            for lod in 0..LODS {
                let m = build(field, &Shore::default(), chunk, lod, &View::CLEAR);
                assert!(!m.indices.is_empty(), "{chunk:?} at {lod}");
                let n = CHUNK / (1 << lod) + 1;
                for tri in m.indices.chunks_exact(3) {
                    let [a, b, c] = [0, 1, 2].map(|k| m.vertices[tri[k] as usize].position);
                    let up = facing(a, b, c, [0.0, 1.0, 0.0]);
                    let ground = tri.iter().all(|&i| (i as usize) < n * n);
                    assert_eq!(up, ground, "{chunk:?} lod {lod}: {tri:?}");
                }
            }
        }
    }

    #[test]
    fn the_finest_stride_lies_on_the_field() {
        let field = fixture_field();
        let chunk = Chunk { row: 2, col: 12 };
        let m = build(field, &Shore::default(), chunk, 0, &View::CLEAR);
        for v in m.vertices.iter().step_by(37) {
            let [x, y, z] = v.position;
            let want = field.height_at(x, z).unwrap();
            assert!(y <= want + 1e-3 && y >= want - SKIRT - DIP - 1e-3, "{y} against {want}");
        }
    }

    #[test]
    fn the_land_meets_the_disc_at_the_rural_edge() {
        let field = fixture_field();
        for deg in (0..360).step_by(3) {
            let at = polar(FROM + 1.0, deg as f64);
            assert!(rise_on(field, at).abs() < 0.05, "{deg}");
            // And the colour carries on from the disc's.
            let c = tint(FROM, 0.0, 0.0, at);
            let d = ground::shade(FROM);
            let m = mottle(at);
            for k in 0..3 {
                assert!((c[k] - d[k] * m).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn steep_ground_is_rock_and_the_tops_are_snow() {
        let at = [0.0, 0.0];
        let meadow = tint(5000.0, 5.0, 0.1, at);
        let rock = tint(5000.0, 5.0, 1.1, at);
        let snow = tint(5000.0, 62.0, 0.2, at);
        let rock_t = pulled(palette::WILD_ROCK, ROCK_STRENGTH);
        let snow_t = pulled(palette::WILD_SNOW, SNOW_STRENGTH);
        let close = |a: [f32; 4], b: [f32; 4]| (0..3).all(|k| (a[k] - b[k] * mottle(at)).abs() < 1e-4);
        assert!(close(rock, rock_t));
        assert!(close(snow, snow_t));
        assert!(!close(meadow, rock_t));
        // Snow doesn't lie on a cliff.
        assert!(close(tint(5000.0, 62.0, 1.2, at), rock_t));
    }

    #[test]
    fn strides_follow_the_distance_and_hold_on_the_line() {
        assert_eq!(lod_for(10.0, None), 0);
        assert_eq!(lod_for(200.0, None), 1);
        assert_eq!(lod_for(2000.0, None), LODS - 1);
        // Just inside the line from a coarser stride: held.
        assert_eq!(lod_for(LOD_DISTANCE[0] - 1.0, Some(1)), 1);
        assert_eq!(lod_for(LOD_DISTANCE[0] * 0.8, Some(1)), 0);
        // Going coarser is at once.
        assert_eq!(lod_for(LOD_DISTANCE[0] + 1.0, Some(0)), 1);
    }

    #[test]
    fn a_chunk_builds_fast_enough_to_build_ten_a_frame() {
        let field = fixture_field();
        let chunk = Chunk { row: 2, col: 12 };
        let started = std::time::Instant::now();
        for _ in 0..4 {
            build(field, &Shore::default(), chunk, 0, &View::CLEAR);
        }
        let each = started.elapsed().as_secs_f64() * 1000.0 / 4.0;
        // Debug builds are several times slower than release.
        eprintln!("a chunk at stride 1: {each:.2} ms");
        assert!(each < 40.0, "{each} ms");
    }
}
