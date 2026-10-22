//! The valleys' mist (W11, #204): at dawn and dusk a pale mist lies in the
//! wilds' hollows, on the valley floors and over the lakes, and the ridges
//! stand clear of it.
//!
//! - **Where.** The field is read every [`CELL`] samples. A cell is a
//!   hollow by how far it lies under the ground round it (the mean over
//!   [`REACH`] cells each way), and the mist lies thickest in the deepest,
//!   thins on the slopes and the high ground, and keeps to the wilds. Its
//!   banks and gaps are a noise of their own ([`drift`]).
//! - **How.** It's air, not a surface: Flint's ground fog (ADR 0106, #297)
//!   on a map ([`build`]) whose floor is the ground and whose amount is
//!   how thick the mist lies there. Over its floor it thins by e every
//!   `1 / FALLOFF` metres, so it lies deep on the valley floor, a ridge
//!   stands up out of it, and you walk into it.
//! - **When.** Its density is the hour's (`daylight::Palette::mist`:
//!   thickest at dawn, some at dusk and through the night, none by day),
//!   cleared with the fog as the camera climbs (`Atmosphere`). It isn't
//!   lit, so it takes the colour of the hour's air ([`colour`]).

use crate::coords::{layer, to_world};
use crate::world::ground::value_noise;
use crate::world::land::{sample_at, SAMPLES};
use crate::world::palette::{self, hex};
use capitol_protocol::rules::{CENTER_X, CENTER_Y, RURAL_OUTER_RADIUS, WILDERNESS_OUTER_RADIUS};
use flint_player::{GroundField, GroundFog};

/// Field samples (m) between the map's texels: fine enough for the
/// smaller drifts.
pub const CELL: usize = 3;
/// Cells each way the ground round a cell is averaged over: about 40 m.
const REACH: usize = 14;
/// How far (m) under the ground round it a cell must lie for mist to start,
/// and where it lies thickest.
const HOLLOW: (f32, f32) = (0.4, 4.0);
/// The heights (m over the plain) over which it thins out to nothing.
const HIGH: (f32, f32) = (16.0, 32.0);
/// The slopes (rise over run) over which it thins out, measured across
/// [`SLOPE_REACH`] cells each way, so a river's banks don't count as steep
/// and the mist lies over the water as well as beside it.
const STEEP: (f32, f32) = (0.3, 0.7);
const SLOPE_REACH: isize = 4;
/// The server radii over which it comes in at the ranges' foot, and goes
/// again past the world's edge.
const FROM: (f32, f32) = (RURAL_OUTER_RADIUS as f32 + 50.0, RURAL_OUTER_RADIUS as f32 + 350.0);
const OUT: (f32, f32) = (WILDERNESS_OUTER_RADIUS as f32 - 200.0, WILDERNESS_OUTER_RADIUS as f32 + 300.0);
/// How dense (per metre) the mist is on its floor where it lies thickest,
/// at dawn.
pub const DENSITY: f32 = 0.45;
/// It thins by e every `1 / FALLOFF` metres over the ground.
pub const FALLOFF: f32 = 1.1;
/// Steps each view ray takes through it.
pub const STEPS: u32 = 24;
/// The drifts' size (m): broad banks, broken finer.
const DRIFT_SCALE: (f32, f32) = (38.0, 13.0);
/// The noise (0 to 1) over which a drift goes from clear to whole.
const DRIFT_EDGE: (f32, f32) = (0.32, 0.66);

/// How much of the mist lies at world (X, Z), 0 to 1: banks and gaps,
/// their edges broken finer, so it isn't one flat veil.
pub fn drift([x, z]: [f32; 2]) -> f32 {
    let broad = value_noise(x / DRIFT_SCALE.0, z / DRIFT_SCALE.0);
    let fine = value_noise(x / DRIFT_SCALE.1, z / DRIFT_SCALE.1 + 31.7);
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

/// The colour (linear) the mist takes in air of `fog` colour: the fog's
/// own light, paled toward the mist's grey-white at the same brightness,
/// and a little brighter, so it reads as mist against the land at every
/// hour and goes dark with the night.
pub fn colour(fog: [f32; 3]) -> [f32; 3] {
    let pale = hex(palette::WILD_MIST);
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let scale = 1.25 * luma(fog) / luma([pale[0], pale[1], pale[2]]);
    std::array::from_fn(|i| fog[i] + (pale[i] * scale - fog[i]) * 0.55)
}

/// The mist's map over the field: Flint's ground fog lies on it.
pub struct MistMap {
    pub origin: [f32; 2],
    pub size: [f32; 2],
    pub width: u32,
    /// `[floor, amount]` a texel: the ground (world Y) and how thick the
    /// mist lies there.
    pub texels: Vec<[f32; 2]>,
}

impl MistMap {
    pub fn ground_fog(&self) -> GroundFog<'_> {
        GroundFog {
            origin: self.origin,
            size: self.size,
            width: self.width,
            depth: self.width,
            texels: &self.texels,
        }
    }

    /// The texel round world (X, Z), for tests.
    #[cfg(test)]
    fn at(&self, [x, z]: [f32; 2]) -> [f32; 2] {
        let n = self.width as usize;
        let cell = self.size[0] / n as f32;
        let a = (((x - self.origin[0]) / cell) as usize).min(n - 1);
        let b = (((z - self.origin[1]) / cell) as usize).min(n - 1);
        self.texels[b * n + a]
    }
}

/// The mist's map over `field`.
pub fn build(field: &GroundField) -> MistMap {
    let heights = field.heightmap().heights();
    let n = (SAMPLES - 1) / CELL + 1;
    let at = |a: usize, b: usize| heights[(b * CELL) * SAMPLES + a * CELL];
    let ground: Vec<f32> = (0..n * n).map(|k| at(k % n, k / n)).collect();
    let mean = box_mean(&ground, n, REACH);
    let cell = CELL as f32 * (crate::world::land::TEXEL as f32) * crate::coords::S;
    let texels = (0..n * n)
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
            let thick = thickness(h - layer::GROUND, slope, mean[k] - h, r);
            let amount = if thick > 0.0 { thick * drift(to_world(x, y)) } else { 0.0 };
            [h, amount]
        })
        .collect();
    // A texel's value is at the middle of its cell, and a sample is at a
    // texel's middle.
    let [x, y] = sample_at(0, 0);
    let [x0, z0] = to_world(x, y);
    MistMap {
        origin: [x0 - 0.5 * cell, z0 - 0.5 * cell],
        size: [n as f32 * cell; 2],
        width: n as u32,
        texels,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::land::tests::fixture_field;

    #[test]
    fn the_mist_lies_in_the_wild_valleys_and_not_on_the_tops() {
        let map = build(fixture_field());
        map.ground_fog().validate().unwrap();
        let n = map.width as usize;
        let cell = map.size[0] / n as f32;
        let mut thick = 0;
        for (k, &[floor, amount]) in map.texels.iter().enumerate() {
            let x = map.origin[0] + ((k % n) as f32 + 0.5) * cell;
            let z = map.origin[1] + ((k / n) as f32 + 0.5) * cell;
            let [sx, sy] = crate::coords::to_server(x, z);
            let r = (sx - CENTER_X).hypot(sy - CENTER_Y);
            if amount > 0.0 {
                assert!(r > FROM.0 as f64, "mist at r {r:.0}, in the city");
                assert!(floor < layer::GROUND + HIGH.1, "mist on a floor {floor:.1} m up, on the tops");
            }
            if amount > 0.5 {
                thick += 1;
            }
        }
        assert!(thick > 2000, "only {thick} texels where it lies thick");
        // Its floor is the ground under it.
        let field = fixture_field();
        for at in [[CENTER_X + 6500.0, CENTER_Y], [CENTER_X, CENTER_Y - 7100.0]] {
            let [x, z] = to_world(at[0], at[1]);
            let [floor, _] = map.at([x, z]);
            let ground = crate::world::land::rise_on(field, [x, z]) + layer::GROUND;
            assert!((floor - ground).abs() < 1.0, "floor {floor} over ground {ground}");
        }
    }

    #[test]
    fn it_lies_where_hollows_are_and_takes_the_airs_colour() {
        assert_eq!(thickness(0.0, 0.0, 5.0, 6500.0), 1.0);
        assert_eq!(thickness(0.0, 0.0, 0.2, 6500.0), 0.0, "no hollow");
        assert_eq!(thickness(40.0, 0.0, 5.0, 6500.0), 0.0, "on the tops");
        assert_eq!(thickness(0.0, 0.0, 5.0, 3000.0), 0.0, "in the city");
        // Paler than the air, a little brighter, and dark at night.
        let dusk = colour([0.22, 0.11, 0.14]);
        assert!(dusk[1] / dusk[0] > 0.11 / 0.22, "{dusk:?}");
        assert!(dusk.iter().sum::<f32>() > 0.47, "{dusk:?}");
        let night = colour([0.01, 0.012, 0.02]);
        assert!(night.iter().all(|&c| c < 0.05), "{night:?}");
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
