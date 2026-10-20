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
//! - **The city doesn't move.** `land::height_at` is 0 out to r3900, so
//!   the land rises only in the wilds. But the water runs on into the rural
//!   ring (#222), as far in as r3150, so the field starts at [`FROM`], just
//!   inside that, and the carve fades in over [`SEAM`] from there (W10). On
//!   the plain the field is flat but for the beds.
//! - **Past the world's edge** (r6000) the ranges run on and settle to flat
//!   by [`OUTER`], where the flat disc takes over again, so the horizon is
//!   hills, not a wall. The rivers and streams that run off the edge run on
//!   down them to ponds (`edge_water`, #229), and are carved like the rest.
//!
//! The meshes are `land_mesh`.

use crate::coords::{layer, to_world, S};
use crate::world::edge_water::Valleys;
use capitol_protocol::land::{self, LandParams, Water};
use capitol_protocol::rules::{CENTER_X, CENTER_Y, WILDERNESS_OUTER_RADIUS};
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

/// Where the land starts: [`SEAM`] inside r3150, where the wild water's
/// courses stop (the hydrology's inner rim), so the carve is fully in
/// before the first bed. Everything between here and the rural edge is
/// flat but for the water.
pub const FROM: f64 = 3100.0;
/// How far the water's banks read wet: the carve of the ground raised by
/// this much (server units) reaches about `WET_RISE / BANK_SLOPE` past
/// the bank.
const WET_RISE: f64 = 6.0;
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
/// ground, before the water's carve: the protocol's, settled to flat past
/// the world's edge by [`OUTER`]. It's exactly `land::height_at` inside
/// [`SETTLE_FROM`].
pub fn settled(x: f64, y: f64, params: &LandParams) -> f64 {
    let r = (x - CENTER_X).hypot(y - CENTER_Y);
    if r >= OUTER {
        return 0.0;
    }
    let raw = land::height_at(x, y, params);
    if r <= SETTLE_FROM {
        raw
    } else {
        raw * (1.0 - smoothstep(SETTLE_FROM, OUTER, r))
    }
}

/// The land's height at server (x, y), in server units above the flat
/// ground: the [`settled`] ground, lowered into the valleys past the edge,
/// with the water carved into it, flat inside [`FROM`] with the carve
/// faded in across the seam.
#[cfg(test)]
pub fn height(x: f64, y: f64, params: &LandParams, water: &Water, valleys: &Valleys) -> f64 {
    carved(x, y, params, water, valleys).0
}

/// [`height`], how deep the water cut into the land there, and how wet
/// the ground is (the cut of the ground raised by [`WET_RISE`]: deep in a
/// bed, fading out a metre or so up the banks).
fn carved(x: f64, y: f64, params: &LandParams, water: &Water, valleys: &Valleys) -> (f64, f64, f64) {
    let r = (x - CENTER_X).hypot(y - CENTER_Y);
    if r <= FROM || r >= OUTER {
        return (0.0, 0.0, 0.0);
    }
    let ground = valleys.lower(settled(x, y, params), x, y);
    let seam = smoothstep(FROM, FROM + SEAM, r);
    let cut = (ground - land::carve(ground, x, y, water)) * seam;
    let raised = ground + WET_RISE;
    let wet = (raised - land::carve(raised, x, y, water)) * seam;
    (ground - cut, cut, wet)
}

/// Server (x, y) of sample (i, j): column i runs along +x, row j along +y.
pub fn sample_at(i: usize, j: usize) -> [f64; 2] {
    [
        CENTER_X - HALF + i as f64 * TEXEL,
        CENTER_Y - HALF + j as f64 * TEXEL,
    ]
}

/// One sample's height, cut and wetness, in metres (`carved`'s, scaled).
#[derive(Debug, Clone, Copy, Default)]
struct Sample {
    y: f32,
    cut: f32,
    wet: f32,
}

/// The world y of every sample, row by row (`layer::GROUND` plus the
/// land's height in metres), how deep in metres the water cut each (the
/// grass keeps off the beds), and how wet each is (the banks' tint, W10).
/// Rows are shared out over `threads`.
pub fn sample(params: &LandParams, water: &Water, valleys: &Valleys, threads: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut samples = vec![Sample { y: layer::GROUND, ..Sample::default() }; SAMPLES * SAMPLES];
    let rows_each = SAMPLES.div_ceil(threads.max(1));
    std::thread::scope(|scope| {
        for (k, rows) in samples.chunks_mut(rows_each * SAMPLES).enumerate() {
            scope.spawn(move || {
                for (dj, row) in rows.chunks_mut(SAMPLES).enumerate() {
                    let j = k * rows_each + dj;
                    for (i, s) in row.iter_mut().enumerate() {
                        let [x, y] = sample_at(i, j);
                        let (height, cut, wet) = carved(x, y, params, water, valleys);
                        *s = Sample {
                            y: layer::GROUND + (height as f32) * S,
                            cut: (cut as f32) * S,
                            wet: (wet as f32) * S,
                        };
                    }
                }
            });
        }
    });
    (
        samples.iter().map(|s| s.y).collect(),
        samples.iter().map(|s| s.cut).collect(),
        samples.iter().map(|s| s.wet).collect(),
    )
}

/// A sampled land: the field, and the depth the water cut at each sample
/// and how wet it is there, both in metres; and the water on it (W10),
/// with the courses it was drawn from, run on past the edge (#229).
pub struct Sampled {
    pub field: GroundField,
    pub cut: Vec<f32>,
    pub wet: Vec<f32>,
    pub water: crate::world::wild_water::Bodies,
    pub drawn: Inputs,
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
    /// Only the water's look reads them (`wild_water`, `land_mesh::Shore`).
    pub fords: Vec<capitol_protocol::terrain::Ford>,
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
            fords: terrain.fords.clone(),
        }
    }

    fn water(&self) -> Water {
        Water::new(self.water.0.iter().chain(&self.water.1), &self.water.2)
    }

    /// The valleys of the courses past the edge (#229).
    fn valleys(&self) -> Valleys {
        Valleys::new(self.water.0.iter().chain(&self.water.1))
    }

    /// The water as `wild_water` and the sound take it.
    pub fn wild(&self) -> crate::world::wild_water::Water<'_> {
        crate::world::wild_water::Water {
            rivers: &self.water.0,
            streams: &self.water.1,
            lakes: &self.water.2,
            fords: &self.fords,
        }
    }

    /// These inputs with every course that runs off the world's edge run
    /// on down the land past it, to a pond or into another (#229).
    fn beyond_the_edge(mut self, threads: usize) -> Self {
        let (rivers, streams, lakes) = &mut self.water;
        crate::world::edge_water::run_on(rivers, streams, lakes, &self.params, threads);
        self
    }

    /// The water bodies on `field` (W10).
    fn bodies(&self, field: &GroundField) -> crate::world::wild_water::Bodies {
        crate::world::wild_water::build(self.wild(), field)
    }

    /// Samples the land on worker threads, with its water run on past the
    /// edge.
    fn sample(self, threads: usize) -> Sampled {
        let drawn = self.beyond_the_edge(threads);
        let (heights, cut, wet) = sample(&drawn.params, &drawn.water(), &drawn.valleys(), threads);
        let field = field(heights);
        let water = drawn.bodies(&field);
        Sampled {
            field,
            cut,
            wet,
            water,
            drawn,
        }
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
                let sampled = job.sample(threads);
                tracing::info!(
                    "land: sampled {SAMPLES}x{SAMPLES} with its water on {threads} threads in {} ms",
                    started.elapsed().as_millis()
                );
                let _ = send.send(sampled);
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
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let started = std::time::Instant::now();
            let sampled = Inputs::of(fixture()).sample(threads);
            eprintln!("land: sampled in {} ms", started.elapsed().as_millis());
            sampled
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
        let inputs = &fixture_land().drawn;
        let (water, valleys) = (inputs.water(), inputs.valleys());
        for (i, j) in [(1000, 300), (200, 768), (768, 1400), (1300, 1250), (40, 40)] {
            let [x, y] = sample_at(i, j);
            let want = height(x, y, &inputs.params, &water, &valleys) as f32 * S;
            let got = rise_on(f, to_world(x, y));
            assert!((got - want).abs() < 1e-3, "({i}, {j}): {got} against {want}");
        }
    }

    #[test]
    fn the_land_is_continuous_across_the_seam_and_the_settling() {
        let inputs = &fixture_land().drawn;
        let (water, valleys) = (inputs.water(), inputs.valleys());
        for deg in 0..720 {
            let deg = deg as f64 * 0.5;
            for edge in [FROM, OUTER] {
                let [x0, y0] = polar(edge - 0.5, deg);
                let [x1, y1] = polar(edge + 0.5, deg);
                let a = height(x0, y0, &inputs.params, &water, &valleys);
                let b = height(x1, y1, &inputs.params, &water, &valleys);
                assert!((a - b).abs() < 0.5, "at {deg}° r{edge}: {a} to {b}");
            }
        }
    }

    #[test]
    fn the_rivers_are_cut_into_the_wilds() {
        let inputs = &fixture_land().drawn;
        let (water, valleys) = (inputs.water(), inputs.valleys());
        let mut cut = 0;
        let mut checked = 0;
        for river in &fixture().rivers {
            for p in &river.points {
                let r = (p.x - CENTER_X).hypot(p.y - CENTER_Y);
                if r < FROM + SEAM + 10.0 || r > WILDERNESS_OUTER_RADIUS {
                    continue;
                }
                checked += 1;
                let h = height(p.x, p.y, &inputs.params, &water, &valleys);
                if h <= p.surface - 0.5 * p.depth {
                    cut += 1;
                }
            }
        }
        assert!(checked > 100, "{checked}");
        assert!(cut * 10 >= checked * 9, "{cut} of {checked} points sit in their beds");
    }
}
