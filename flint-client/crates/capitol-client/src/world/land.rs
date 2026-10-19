//! The land (W9, #202): the wilds' height field, from `capitol_protocol::land`
//! and W3's carve, as the one ground everything stands on.
//!
//! - **One field.** The heights are sampled once a terrain on a grid of
//!   [`TEXEL`] (1 m) over a square a little wider than the world, and held
//!   as a Flint [`GroundField`]. It's the click's ground (the projector's
//!   `screen_to_heightfield`, W8), the rig's, and the height every avatar,
//!   node, station and mark is stood at ([`rise`]).
//! - **Off the main thread.** [`Builder`] samples it on worker threads, so a
//!   new terrain never stalls a frame. Until it's in, the world is flat.
//! - **The city doesn't move.** The land is exactly flat inside the rural
//!   edge (r3800, `land::height_at` is 0 out to r3900), and the carve fades
//!   in over [`SEAM`] past it. The rivers that run on into the rural ring
//!   (#222) are W10's to draw: the flat disc there is uncut.
//! - **Past the world's edge** (r6000) the ranges run on and settle to flat
//!   by [`OUTER`], where the flat disc takes over again, so the horizon is
//!   hills, not a wall.
//!
//! The meshes are `land_mesh`.

use crate::coords::{layer, to_world, S};
use capitol_protocol::land::{self, LandParams, Water};
use capitol_protocol::rules::{CENTER_X, CENTER_Y, RURAL_OUTER_RADIUS, WILDERNESS_OUTER_RADIUS};
use capitol_protocol::terrain::TerrainResponse;
use flint_player::{GroundField, Heightmap};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, RwLock};

/// Server units between samples: 1 m.
pub const TEXEL: f64 = 10.0;
/// Quads along a chunk's side (`land_mesh`), and chunks along the field's.
pub const CHUNK: usize = 64;
pub const CHUNKS: usize = 24;
/// Samples along the field's side.
pub const SAMPLES: usize = CHUNK * CHUNKS + 1;
/// Half the field's side, in server units: 7680, past [`OUTER`].
pub const HALF: f64 = (CHUNK * CHUNKS) as f64 * TEXEL * 0.5;

/// Where the land starts: the rural edge.
pub const FROM: f64 = RURAL_OUTER_RADIUS;
/// How far past [`FROM`] the carve takes to come in.
pub const SEAM: f64 = 50.0;
/// Where the ranges start to settle, past the world's edge, and where
/// they're flat again and the flat disc takes over.
const SETTLE_FROM: f64 = WILDERNESS_OUTER_RADIUS + 400.0;
pub const OUTER: f64 = WILDERNESS_OUTER_RADIUS + 1400.0;

fn smoothstep(lo: f64, hi: f64, x: f64) -> f64 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The land's height at server (x, y), in server units above the flat
/// ground: the protocol's carved ground, flat inside [`FROM`] with the
/// carve faded in across the seam, and settled to flat by [`OUTER`].
#[cfg(test)]
pub fn height(x: f64, y: f64, params: &LandParams, water: &Water) -> f64 {
    carved(x, y, params, water).0
}

/// [`height`], and how deep the water cut into the land there.
fn carved(x: f64, y: f64, params: &LandParams, water: &Water) -> (f64, f64) {
    let r = (x - CENTER_X).hypot(y - CENTER_Y);
    if r <= FROM || r >= OUTER {
        return (0.0, 0.0);
    }
    let raw = land::height_at(x, y, params);
    let cut = (raw - land::carve(raw, x, y, water)) * smoothstep(FROM, FROM + SEAM, r);
    let settle = 1.0 - smoothstep(SETTLE_FROM, OUTER, r);
    (raw * settle - cut, cut)
}

/// Server (x, y) of sample (i, j): column i runs along +x, row j along +y.
pub fn sample_at(i: usize, j: usize) -> [f64; 2] {
    [
        CENTER_X - HALF + i as f64 * TEXEL,
        CENTER_Y - HALF + j as f64 * TEXEL,
    ]
}

/// The world y of every sample, row by row (`layer::GROUND` plus the
/// land's height in metres), and how deep in metres the water cut each
/// (the grass keeps off the beds until W10 fills them). Rows are shared
/// out over `threads`.
pub fn sample(params: &LandParams, water: &Water, threads: usize) -> (Vec<f32>, Vec<f32>) {
    let mut heights = vec![layer::GROUND; SAMPLES * SAMPLES];
    let mut cuts = vec![0.0; SAMPLES * SAMPLES];
    let rows_each = SAMPLES.div_ceil(threads.max(1));
    std::thread::scope(|scope| {
        let chunks = heights
            .chunks_mut(rows_each * SAMPLES)
            .zip(cuts.chunks_mut(rows_each * SAMPLES));
        for (k, (rows, cut_rows)) in chunks.enumerate() {
            scope.spawn(move || {
                for (dj, (row, cut_row)) in rows
                    .chunks_mut(SAMPLES)
                    .zip(cut_rows.chunks_mut(SAMPLES))
                    .enumerate()
                {
                    let j = k * rows_each + dj;
                    for (i, (h, c)) in row.iter_mut().zip(cut_row.iter_mut()).enumerate() {
                        let [x, y] = sample_at(i, j);
                        let (height, cut) = carved(x, y, params, water);
                        *h = layer::GROUND + (height as f32) * S;
                        *c = (cut as f32) * S;
                    }
                }
            });
        }
    });
    (heights, cuts)
}

/// A sampled land: the field, and the depth the water cut at each sample.
pub struct Sampled {
    pub field: GroundField,
    pub cut: Vec<f32>,
}

/// The cut, in metres, at the sample of `cut` (a [`Sampled::cut`]) nearest
/// world (X, Z).
pub fn cut_at(cut: &[f32], [x, z]: [f32; 2]) -> f32 {
    let [sx, sy] = crate::coords::to_server(x, z);
    let i = ((sx - (CENTER_X - HALF)) / TEXEL).round();
    let j = ((sy - (CENTER_Y - HALF)) / TEXEL).round();
    let n = SAMPLES as f64;
    if !(0.0..n).contains(&i) || !(0.0..n).contains(&j) {
        return 0.0;
    }
    cut[j as usize * SAMPLES + i as usize]
}

/// The field over `heights` (from [`sample`]), in world metres.
pub fn field(heights: Vec<f32>) -> GroundField {
    let [x0, z0] = to_world(CENTER_X - HALF, CENTER_Y - HALF);
    let side = (2.0 * HALF) as f32 * S;
    GroundField::new(
        Heightmap::from_raw(heights, SAMPLES as u32, SAMPLES as u32),
        [x0, z0],
        [side, side],
        1.0,
        0.0,
    )
}

/// What the land is made from on a terrain: when none of it changes, the
/// land doesn't either.
#[derive(Debug, Clone, PartialEq)]
pub struct Inputs {
    pub params: LandParams,
    pub water: (
        Vec<capitol_protocol::terrain::WaterCourse>,
        Vec<capitol_protocol::terrain::WaterCourse>,
        Vec<capitol_protocol::terrain::Lake>,
    ),
}

impl Inputs {
    pub fn of(terrain: &TerrainResponse) -> Self {
        Self {
            params: terrain.land.clone(),
            water: (
                terrain.rivers.clone(),
                terrain.streams.clone(),
                terrain.lakes.clone(),
            ),
        }
    }

    fn water(&self) -> Water {
        Water::new(self.water.0.iter().chain(&self.water.1), &self.water.2)
    }
}

/// The field being sampled on a worker thread, and the inputs it's for.
#[derive(Default)]
pub struct Builder {
    /// The inputs of the field last asked for.
    asked: Option<Inputs>,
    pending: Option<Receiver<Sampled>>,
}

impl Builder {
    /// Starts sampling the land for `terrain`, unless it's the land last
    /// asked for. Returns whether it started.
    pub fn ask(&mut self, terrain: &TerrainResponse) -> bool {
        let inputs = Inputs::of(terrain);
        if self.asked.as_ref() == Some(&inputs) {
            return false;
        }
        let (send, receive) = channel();
        let job = inputs.clone();
        let spawned = std::thread::Builder::new()
            .name("capitol-land".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
                let (heights, cut) = sample(&job.params, &job.water(), threads);
                tracing::info!(
                    "land: sampled {SAMPLES}x{SAMPLES} on {threads} threads in {} ms",
                    started.elapsed().as_millis()
                );
                let _ = send.send(Sampled {
                    field: field(heights),
                    cut,
                });
            });
        if let Err(e) = spawned {
            tracing::warn!("land: no thread to sample it on: {e}");
            return false;
        }
        self.asked = Some(inputs);
        self.pending = Some(receive);
        true
    }

    /// The land, once it's in. A newer ask drops an older one's result.
    pub fn take(&mut self) -> Option<Sampled> {
        let field = self.pending.as_ref()?.try_recv().ok()?;
        self.pending = None;
        Some(field)
    }
}

/// The land everything stands on, once it's in.
static CURRENT: RwLock<Option<Arc<GroundField>>> = RwLock::new(None);

/// Makes `field` the land everything stands on.
pub fn set(field: Option<Arc<GroundField>>) {
    if let Ok(mut current) = CURRENT.write() {
        *current = field;
    }
}

/// The land everything stands on, if it's in.
pub fn current() -> Option<Arc<GroundField>> {
    CURRENT.read().ok().and_then(|c| c.clone())
}

/// How far the land under world (X, Z) stands above the flat ground, in
/// metres: 0 in the city, before the land is in, and off the field.
pub fn rise([x, z]: [f32; 2]) -> f32 {
    current().map_or(0.0, |f| rise_on(&f, [x, z]))
}

/// [`rise`] on a given field. Exactly 0 where the field is flat: the
/// bilinear blend of four flat samples is off by a rounding, and the city
/// mustn't move by one.
pub fn rise_on(field: &GroundField, [x, z]: [f32; 2]) -> f32 {
    let rise = field.height_at(x, z).map_or(0.0, |y| y - layer::GROUND);
    if rise.abs() < 1e-3 {
        0.0
    } else {
        rise
    }
}

/// The ground's world y under world (X, Z).
pub fn ground_y(at: [f32; 2]) -> f32 {
    layer::GROUND + rise(at)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use capitol_core::terrain::read_terrain_file;
    use std::path::Path;
    use std::sync::OnceLock;

    pub(crate) fn fixture() -> &'static TerrainResponse {
        static T: OnceLock<TerrainResponse> = OnceLock::new();
        T.get_or_init(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
            read_terrain_file(&path).unwrap()
        })
    }

    /// The fixture's land, sampled once for every test that wants it.
    pub(crate) fn fixture_land() -> &'static Sampled {
        static F: OnceLock<Sampled> = OnceLock::new();
        F.get_or_init(|| {
            let inputs = Inputs::of(fixture());
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let started = std::time::Instant::now();
            let (heights, cut) = sample(&inputs.params, &inputs.water(), threads);
            eprintln!("land: sampled in {} ms", started.elapsed().as_millis());
            Sampled {
                field: field(heights),
                cut,
            }
        })
    }

    pub(crate) fn fixture_field() -> &'static GroundField {
        &fixture_land().field
    }

    fn polar(r: f64, deg: f64) -> [f64; 2] {
        let (s, c) = deg.to_radians().sin_cos();
        [CENTER_X + r * c, CENTER_Y + r * s]
    }

    #[test]
    fn the_city_is_flat_and_the_ranges_rise() {
        let f = fixture_field();
        for deg in (0..360).step_by(7) {
            for r in [0.0, 1000.0, 3000.0, FROM - 1.5 * TEXEL] {
                let [x, y] = polar(r, deg as f64);
                assert_eq!(rise_on(f, to_world(x, y)), 0.0, "r {r} at {deg}");
            }
        }
        let top = (0..360)
            .map(|deg| {
                let [x, y] = polar(5600.0, deg as f64 + 22.5);
                rise_on(f, to_world(x, y))
            })
            .fold(0.0, f32::max);
        assert!(top > 20.0, "the ranges top out at {top} m");
    }

    #[test]
    fn the_field_agrees_with_the_protocol_at_its_samples() {
        let f = fixture_field();
        let inputs = Inputs::of(fixture());
        let water = inputs.water();
        for (i, j) in [(1000, 300), (200, 768), (768, 1400), (1300, 1250), (40, 40)] {
            let [x, y] = sample_at(i, j);
            let want = height(x, y, &inputs.params, &water) as f32 * S;
            let got = rise_on(f, to_world(x, y));
            assert!((got - want).abs() < 1e-3, "({i}, {j}): {got} against {want}");
        }
    }

    #[test]
    fn the_land_is_continuous_across_the_seam_and_the_settling() {
        let inputs = Inputs::of(fixture());
        let water = inputs.water();
        for deg in 0..720 {
            let deg = deg as f64 * 0.5;
            for edge in [FROM, OUTER] {
                let [x0, y0] = polar(edge - 0.5, deg);
                let [x1, y1] = polar(edge + 0.5, deg);
                let a = height(x0, y0, &inputs.params, &water);
                let b = height(x1, y1, &inputs.params, &water);
                assert!((a - b).abs() < 0.5, "at {deg}° r{edge}: {a} to {b}");
            }
        }
    }

    #[test]
    fn the_rivers_are_cut_into_the_wilds() {
        let inputs = Inputs::of(fixture());
        let water = inputs.water();
        let mut cut = 0;
        let mut checked = 0;
        for river in &fixture().rivers {
            for p in &river.points {
                let r = (p.x - CENTER_X).hypot(p.y - CENTER_Y);
                if r < FROM + SEAM + 10.0 || r > WILDERNESS_OUTER_RADIUS {
                    continue;
                }
                checked += 1;
                let h = height(p.x, p.y, &inputs.params, &water);
                if h <= p.surface - 0.5 * p.depth {
                    cut += 1;
                }
            }
        }
        assert!(checked > 100, "{checked}");
        assert!(cut * 10 >= checked * 9, "{cut} of {checked} points sit in their beds");
    }
}
