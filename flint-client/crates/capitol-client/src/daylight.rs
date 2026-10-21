//! The day going round (P8.6a). One day is two real hours on the server's
//! clock (`rules::time_of_day`, B12), so every client sees the same sky.
//! The phase runs from 0 at midnight through 0.5 at noon.
//!
//! The light is two keyframe tracks, each eased (smoothstep) from key to
//! key and wrapping at midnight:
//!
//! - **The palette**: the sky's colours and stars, the ambient from above,
//!   the fill light, the fog's colour and the far land's blue (#233), the
//!   exposure, how the gold on your plots shows, and how thick the
//!   valleys' mist lies (W11). Night holds
//!   round midnight, then dawn, a held day, evening, and night again.
//! - **The key light**: the sun by day and the moon by night, as a compass
//!   bearing, a height and a colour. The engine draws the sky's disc where
//!   the first directional light is, so the same light is both. It's put
//!   out below the horizon, and crosses back under it in the dark between
//!   sunset and moonrise, and moonset and sunrise.
//!
//! **Evening is the scene as authored**: the dusk of P3.8, read from
//! `scenes/capitol.scene.toml` the first frame. So the dusk is one stop on
//! the wheel, tuned in one place, and an offline capture (which defaults
//! to evening) looks as it always has.
//!
//! The scene's compass: north is -z and east is +x, so the usual camera
//! looks north and the evening sun is in the south-west.

use crate::world::palette::mix;

/// The evening key: the scene as authored. Offline captures default here.
pub const EVENING: f64 = 0.80;
/// How thick the valleys' mist lies at evening (`world::mist`), as the dusk
/// cools. The scene doesn't author it.
pub const EVENING_MIST: f32 = 0.6;

/// How the gold over your own plots shows: `surface` is how much of it is
/// a lit gold surface (1 by day), and `radiance` how much light it gives
/// off (`palette::glow`). Evening is today's look, light at 0.5 and no
/// surface; night is brighter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlotGlow {
    pub surface: f32,
    pub radiance: f32,
}

impl PlotGlow {
    /// The glow at evening: P4's gold curb, all light at 0.5.
    pub const EVENING: Self = Self {
        surface: 0.0,
        radiance: 0.5,
    };

    /// Rounded to steps of 1/40, so the mesh is only rebuilt when the
    /// change could show.
    pub fn stepped(self) -> Self {
        let r = |v: f32| (v * 40.0).round() / 40.0;
        Self {
            surface: r(self.surface),
            radiance: r(self.radiance),
        }
    }
}

/// A directional light: `direction` points toward the light, as the
/// engine's `light.direction` does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
}

/// The sky component's fields that move with the day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    pub zenith: [f32; 4],
    pub horizon: [f32; 4],
    pub haze: [f32; 4],
    pub cloud_tint: [f32; 4],
    pub cloud_coverage: f32,
    pub star_opacity: f32,
    pub sun_disc_size: f32,
    pub sun_glow: f32,
    pub ambient_sky: [f32; 4],
}

/// Everything on the palette track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub sky: Sky,
    pub fill: Light,
    pub fog_color: [f32; 3],
    /// The air's blue on far things (linear): the fog's aerial tint (Flint
    /// ADR 0101, #233). It's light in the air, not a surface, so it goes
    /// dark with the sky.
    pub aerial: [f32; 3],
    pub exposure: f32,
    pub plots: PlotGlow,
    /// How thick the valleys' mist lies, 0 to 1 (`world::mist`, W11):
    /// thickest at dawn, some through the dusk and the night, none by day.
    pub mist: f32,
}

/// The key light on its track: a compass bearing (degrees from north,
/// clockwise), a height over the horizon (degrees), and its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Luminary {
    pub bearing: f32,
    pub height: f32,
    pub color: [f32; 3],
    pub intensity: f32,
}

impl Luminary {
    /// The light this puts in the scene.
    pub fn light(&self) -> Light {
        let (b, h) = (self.bearing.to_radians(), self.height.to_radians());
        Light {
            direction: [h.cos() * b.sin(), h.sin(), -h.cos() * b.cos()],
            color: self.color,
            intensity: self.intensity,
        }
    }

    /// The same light read back as a bearing and a height.
    pub fn from_light(light: &Light) -> Self {
        let [x, y, z] = light.direction;
        let flat = x.hypot(z);
        Self {
            bearing: x.atan2(-z).to_degrees().rem_euclid(360.0),
            height: y.atan2(flat).to_degrees(),
            color: light.color,
            intensity: light.intensity,
        }
    }
}

/// The light at one moment: what the atmosphere writes into the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Daylight {
    pub key: Light,
    pub palette: Palette,
}

/// The fog is the sky's colour on the horizon: `horizon` mixed this far
/// toward `haze`, so the land runs out into the sky with no line (P3.8).
const FOG_TOWARD_HAZE: f32 = 0.45;

fn fog_of(sky: &Sky) -> [f32; 3] {
    let [r, g, b, _] = mix(sky.horizon, sky.haze, FOG_TOWARD_HAZE);
    [r, g, b]
}

/// Deep night: a navy-black sky full of stars, a cool sky light just
/// strong enough to read the streets by, and the gold at its brightest.
fn night() -> Palette {
    let sky = Sky {
        zenith: [0.002, 0.003, 0.010, 1.0],
        horizon: [0.018, 0.024, 0.055, 1.0],
        haze: [0.035, 0.045, 0.085, 0.30],
        cloud_tint: [0.05, 0.06, 0.09, 0.35],
        cloud_coverage: 0.2,
        star_opacity: 1.0,
        sun_disc_size: 0.010,
        sun_glow: 0.4,
        ambient_sky: [0.07, 0.085, 0.15, 1.0],
    };
    Palette {
        fog_color: fog_of(&sky),
        sky,
        fill: Light {
            direction: [0.0, 1.0, 0.0],
            color: [0.35, 0.42, 0.80],
            intensity: 0.12,
        },
        aerial: [0.012, 0.017, 0.038],
        exposure: 1.0,
        plots: PlotGlow {
            surface: 0.0,
            radiance: 1.6,
        },
        mist: 0.35,
    }
}

/// Dawn: the east goes rose and gold under a sky still blue-black
/// overhead, the last stars going out.
fn dawn() -> Palette {
    let sky = Sky {
        zenith: [0.030, 0.050, 0.130, 1.0],
        horizon: [0.30, 0.20, 0.25, 1.0],
        haze: [0.75, 0.42, 0.28, 0.50],
        cloud_tint: [0.75, 0.48, 0.45, 0.45],
        cloud_coverage: 0.25,
        star_opacity: 0.15,
        sun_disc_size: 0.014,
        sun_glow: 0.5,
        ambient_sky: [0.22, 0.22, 0.30, 1.0],
    };
    Palette {
        fog_color: fog_of(&sky),
        sky,
        fill: Light {
            direction: [-0.6, 0.5, 0.3],
            color: [0.55, 0.60, 0.92],
            intensity: 0.35,
        },
        aerial: [0.15, 0.15, 0.24],
        exposure: 1.0,
        plots: PlotGlow {
            surface: 0.6,
            radiance: 0.25,
        },
        mist: 1.0,
    }
}

/// Day: a clear blue sky, white sun, soft blue light from above. The gold
/// on your plots is plain gold in the sun.
fn day() -> Palette {
    let sky = Sky {
        zenith: [0.07, 0.20, 0.52, 1.0],
        horizon: [0.42, 0.56, 0.72, 1.0],
        haze: [0.70, 0.75, 0.80, 0.40],
        cloud_tint: [0.95, 0.95, 0.97, 0.80],
        cloud_coverage: 0.30,
        star_opacity: 0.0,
        sun_disc_size: 0.014,
        sun_glow: 0.6,
        ambient_sky: [0.40, 0.46, 0.60, 1.0],
    };
    Palette {
        fog_color: fog_of(&sky),
        sky,
        fill: Light {
            direction: [0.2, 0.8, -0.4],
            color: [0.60, 0.70, 0.95],
            intensity: 0.35,
        },
        aerial: [0.22, 0.33, 0.55],
        exposure: 0.9,
        plots: PlotGlow {
            surface: 1.0,
            radiance: 0.0,
        },
        mist: 0.0,
    }
}

/// The moon: a small pale disc, and a cool light that still throws soft
/// shadows.
const MOON: [f32; 3] = [0.55, 0.65, 1.0];
const MOON_LIGHT: f32 = 0.45;
/// The sun at dawn, low and warm, and at noon, high and white.
const DAWN_SUN: [f32; 3] = [1.0, 0.70, 0.45];
const NOON_SUN: [f32; 3] = [1.0, 0.95, 0.88];
/// The sun's light as it stands on the horizon, rising or setting.
const ON_THE_HORIZON: f32 = 0.3;

/// The palette's keys. Night holds across midnight, day across noon.
fn palette_keys(evening: &Palette) -> [(f64, Palette); 6] {
    [
        (0.08, night()),
        (0.25, dawn()),
        (0.42, day()),
        (0.62, day()),
        (EVENING, *evening),
        (0.92, night()),
    ]
}

/// The key light's keys: the sun rises in the east, crosses the south and
/// sets beyond the evening's south-west; the moon rises in the east after
/// it, stands in the south at midnight and is down before dawn. Below the
/// horizon a light gives none, and on it only `ON_THE_HORIZON`.
fn key_keys(evening: &Luminary) -> [(f64, Luminary); 10] {
    let lum = |bearing, height, color, intensity| Luminary {
        bearing,
        height,
        color,
        intensity,
    };
    [
        (0.0, lum(175.0, 40.0, MOON, MOON_LIGHT)),
        (0.16, lum(255.0, -4.0, MOON, 0.0)),
        (0.19, lum(75.0, -4.0, DAWN_SUN, 0.0)),
        (0.21, lum(79.0, 0.0, DAWN_SUN, ON_THE_HORIZON)),
        (0.25, lum(85.0, 7.0, DAWN_SUN, 2.6)),
        (0.5, lum(165.0, 56.0, NOON_SUN, 3.0)),
        (EVENING, *evening),
        (0.84, lum(238.0, 0.0, evening.color, ON_THE_HORIZON)),
        (0.86, lum(245.0, -4.0, evening.color, 0.0)),
        (0.89, lum(105.0, -4.0, MOON, 0.0)),
    ]
}

/// The light at `phase` (0..1, midnight to midnight), given the evening
/// as authored.
pub fn at(phase: f64, evening_key: &Light, evening: &Palette) -> Daylight {
    let phase = phase.rem_euclid(1.0);
    let evening_lum = Luminary::from_light(evening_key);
    let key = sample(&key_keys(&evening_lum), phase, lerp_luminary);
    let mut key = key.light();
    if phase == EVENING {
        // Exactly as authored, not as it comes back from a bearing.
        key = *evening_key;
    }
    Daylight {
        key,
        palette: sample(&palette_keys(evening), phase, lerp_palette),
    }
}

/// `keys` (sorted by phase, wrapping at 1) at `phase`, eased from key to
/// key.
fn sample<T: Copy>(keys: &[(f64, T)], phase: f64, lerp: fn(&T, &T, f32) -> T) -> T {
    let n = keys.len();
    let i = keys
        .iter()
        .rposition(|(at, _)| *at <= phase)
        .unwrap_or(n - 1);
    let (a_at, a) = keys[i];
    let (b_at, b) = keys[(i + 1) % n];
    let span = (b_at - a_at).rem_euclid(1.0);
    let into = (phase - a_at).rem_euclid(1.0);
    if into == 0.0 || span <= 0.0 {
        // On the key: exactly the key.
        return a;
    }
    let t = into / span;
    let t = t as f32;
    lerp(&a, &b, t * t * (3.0 - 2.0 * t))
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| lerp(a[i], b[i], t))
}

fn lerp_light(a: &Light, b: &Light, t: f32) -> Light {
    let d = lerp3(a.direction, b.direction, t);
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    Light {
        direction: d.map(|c| c / len),
        color: lerp3(a.color, b.color, t),
        intensity: lerp(a.intensity, b.intensity, t),
    }
}

/// The bearing goes the short way round.
fn lerp_luminary(a: &Luminary, b: &Luminary, t: f32) -> Luminary {
    let turn = (b.bearing - a.bearing + 540.0).rem_euclid(360.0) - 180.0;
    Luminary {
        bearing: (a.bearing + turn * t).rem_euclid(360.0),
        height: lerp(a.height, b.height, t),
        color: lerp3(a.color, b.color, t),
        intensity: lerp(a.intensity, b.intensity, t),
    }
}

fn lerp_sky(a: &Sky, b: &Sky, t: f32) -> Sky {
    Sky {
        zenith: mix(a.zenith, b.zenith, t),
        horizon: mix(a.horizon, b.horizon, t),
        haze: mix(a.haze, b.haze, t),
        cloud_tint: mix(a.cloud_tint, b.cloud_tint, t),
        cloud_coverage: lerp(a.cloud_coverage, b.cloud_coverage, t),
        star_opacity: lerp(a.star_opacity, b.star_opacity, t),
        sun_disc_size: lerp(a.sun_disc_size, b.sun_disc_size, t),
        sun_glow: lerp(a.sun_glow, b.sun_glow, t),
        ambient_sky: mix(a.ambient_sky, b.ambient_sky, t),
    }
}

fn lerp_palette(a: &Palette, b: &Palette, t: f32) -> Palette {
    Palette {
        sky: lerp_sky(&a.sky, &b.sky, t),
        fill: lerp_light(&a.fill, &b.fill, t),
        fog_color: lerp3(a.fog_color, b.fog_color, t),
        aerial: lerp3(a.aerial, b.aerial, t),
        exposure: lerp(a.exposure, b.exposure, t),
        plots: PlotGlow {
            surface: lerp(a.plots.surface, b.plots.surface, t),
            radiance: lerp(a.plots.radiance, b.plots.radiance, t),
        },
        mist: lerp(a.mist, b.mist, t),
    }
}

/// How far `shown` moves toward `target` this frame, both phases. A small
/// gap (the clock ticking on) is taken at once; a large one (the first
/// `AuthOk`, a `--time-of-day` jump) is eased, the short way round.
pub fn approach(shown: f64, target: f64, dt: f64) -> f64 {
    let gap = (target - shown + 1.5).rem_euclid(1.0) - 0.5;
    if gap.abs() < 1e-3 {
        return target.rem_euclid(1.0);
    }
    (shown + gap * (1.0 - (-dt * EASE_RATE).exp())).rem_euclid(1.0)
}

/// 1/e of a jump in the phase is taken in this many seconds' inverse.
const EASE_RATE: f64 = 1.2;

#[cfg(test)]
mod tests {
    use super::*;

    fn evening() -> (Light, Palette) {
        // The scene's dusk (capitol.scene.toml).
        let sky = Sky {
            zenith: [0.010, 0.012, 0.032, 1.0],
            horizon: [0.110, 0.065, 0.120, 1.0],
            haze: [0.36, 0.17, 0.16, 0.45],
            cloud_tint: [0.35, 0.22, 0.30, 0.4],
            cloud_coverage: 0.2,
            star_opacity: 0.6,
            sun_disc_size: 0.014,
            sun_glow: 0.3,
            ambient_sky: [0.16, 0.17, 0.24, 1.0],
        };
        let key = Light {
            direction: [-0.68, 0.14, 0.68],
            color: [1.0, 0.60, 0.32],
            intensity: 3.2,
        };
        let palette = Palette {
            sky,
            fill: Light {
                direction: [0.6, 0.5, -0.4],
                color: [0.50, 0.58, 0.95],
                intensity: 0.4,
            },
            fog_color: [0.2225, 0.1123, 0.1380],
            aerial: [0.10, 0.105, 0.20],
            exposure: 1.0,
            plots: PlotGlow::EVENING,
            mist: EVENING_MIST,
        };
        (key, palette)
    }

    #[test]
    fn the_mist_lies_thickest_at_dawn_and_is_gone_by_day() {
        let (key, palette) = evening();
        let mist = |phase: f64| at(phase, &key, &palette).palette.mist;
        assert_eq!(mist(0.25), 1.0);
        assert_eq!(mist(0.5), 0.0);
        assert_eq!(mist(EVENING), EVENING_MIST);
        assert!(mist(0.0) > 0.0 && mist(0.0) < EVENING_MIST, "{}", mist(0.0));
    }

    /// The far land's blue (#233) is light in the air, not a surface: it's
    /// blue at every hour, and as dark as the sky is at night.
    #[test]
    fn the_far_blue_is_blue_and_goes_dark_with_the_sky() {
        let (key, palette) = evening();
        let aerial = |phase: f64| at(phase, &key, &palette).palette.aerial;
        let light = |[r, g, b]: [f32; 3]| 0.2126 * r + 0.7152 * g + 0.0722 * b;
        for step in 0..48 {
            let [r, g, b] = aerial(step as f64 / 48.0);
            assert!(b > g && g >= r, "{step}: {:?}", [r, g, b]);
        }
        let (night, dawn, day) = (light(aerial(0.0)), light(aerial(0.25)), light(aerial(0.5)));
        let dusk = light(aerial(EVENING));
        assert!(night < dusk && dusk < dawn && dawn < day, "{night} {dusk} {dawn} {day}");
        // Never brighter than the fog it lies under on the horizon by day.
        let fog = at(0.5, &key, &palette).palette.fog_color;
        assert!(day < light(fog), "{day} vs {}", light(fog));
    }

    #[test]
    fn evening_is_the_scene_as_authored() {
        let (key, palette) = evening();
        let d = at(EVENING, &key, &palette);
        assert_eq!(d.key, key);
        assert_eq!(d.palette, palette);
    }

    #[test]
    fn the_fog_rule_holds_for_the_authored_dusk() {
        let (_, palette) = evening();
        let fog = fog_of(&palette.sky);
        for (a, b) in fog.iter().zip(palette.fog_color) {
            assert!((a - b).abs() < 1e-3, "{fog:?}");
        }
    }

    #[test]
    fn the_evening_sun_is_low_in_the_south_west() {
        let (key, _) = evening();
        let lum = Luminary::from_light(&key);
        assert!((lum.bearing - 225.0).abs() < 0.01, "{lum:?}");
        assert!((lum.height - 8.28).abs() < 0.05, "{lum:?}");
        // And back again, parallel to the authored direction.
        let back = lum.light().direction;
        let len = (0.68f32 * 0.68 * 2.0 + 0.14 * 0.14).sqrt();
        for (a, b) in back.iter().zip(key.direction) {
            assert!((a - b / len).abs() < 1e-4);
        }
    }

    #[test]
    fn the_sun_is_up_by_day_and_the_moon_by_night() {
        let (key, palette) = evening();
        let noon = at(0.5, &key, &palette).key;
        assert!(noon.direction[1] > 0.75, "high at noon");
        assert!(noon.direction[2] > 0.0, "in the south at noon");
        let dawn = at(0.25, &key, &palette).key;
        assert!(dawn.direction[0] > 0.9, "in the east at dawn");
        let midnight = at(0.0, &key, &palette).key;
        assert!(midnight.direction[1] > 0.5 && midnight.intensity < 1.0);
        assert!(midnight.color[2] > midnight.color[0], "the moon is cool");
    }

    #[test]
    fn a_light_below_the_horizon_gives_none() {
        let (key, palette) = evening();
        for step in 0..=2000 {
            let phase = step as f64 / 2000.0;
            let light = at(phase, &key, &palette).key;
            // More than a degree under the horizon.
            if light.direction[1] < -0.0175 {
                assert!(light.intensity < ON_THE_HORIZON, "{phase}: {light:?}");
            }
        }
    }

    #[test]
    fn the_day_wraps_without_a_jump() {
        let (key, palette) = evening();
        let before = at(0.99999, &key, &palette);
        let after = at(0.0, &key, &palette);
        assert!((before.palette.sky.star_opacity - after.palette.sky.star_opacity).abs() < 1e-3);
        for (a, b) in before.key.direction.iter().zip(after.key.direction) {
            assert!((a - b).abs() < 1e-3);
        }
        // And nowhere else does a frame (a second of game time) jump.
        let step = 1.0 / rules_day_secs();
        let mut last = at(0.0, &key, &palette);
        for i in 1..=(1.0 / step) as usize {
            let now = at(i as f64 * step, &key, &palette);
            assert!(
                (now.palette.sky.ambient_sky[0] - last.palette.sky.ambient_sky[0]).abs() < 0.01
            );
            assert!((now.key.intensity - last.key.intensity).abs() < 0.05);
            last = now;
        }
    }

    fn rules_day_secs() -> f64 {
        capitol_protocol::rules::DAY_LENGTH_SECS as f64
    }

    #[test]
    fn the_plots_glow_at_night_and_not_by_day() {
        let (key, palette) = evening();
        let night = at(0.0, &key, &palette).palette.plots;
        let noon = at(0.5, &key, &palette).palette.plots;
        assert!(night.radiance > PlotGlow::EVENING.radiance);
        assert_eq!(noon.radiance, 0.0);
        assert_eq!(noon.surface, 1.0);
    }

    #[test]
    fn a_jump_is_eased_the_short_way_and_a_tick_taken_at_once() {
        assert_eq!(approach(0.5, 0.5004, 0.016), 0.5004);
        let next = approach(0.8, 0.1, 0.1);
        assert!(next > 0.8, "forward over midnight: {next}");
        let mut shown = 0.8;
        for _ in 0..600 {
            shown = approach(shown, 0.1, 1.0 / 60.0);
        }
        assert!((shown - 0.1).abs() < 1e-3, "settled within 10 s: {shown}");
    }
}
