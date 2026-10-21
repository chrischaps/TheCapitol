//! The air over the city (P3.8), and the light it lets through. The scene
//! sets the dusk: the sky, the low sun, and a fog the colour of the sky's
//! horizon, so the land runs out into the sky with no line where they meet.
//! The day goes round from there (`daylight`, P8.6a): the dusk is its
//! evening, and the sky, the sun and moon, the fill, the fog's colour and
//! the light from above move with the server's clock. Here, as the camera
//! climbs, the fog thins and is gone by the district view, and then the
//! light flattens into a map (P3.9), whatever the hour.
//!
//! Fog works on distance from the camera, and from high up all of the city
//! is far away: at the map view (880 m up) the scene's fog would cover it in
//! haze. So the density eases to nothing as the camera climbs. Fog stays on
//! the whole way: the canals' water takes the scene's fog like the land does
//! (Flint ADR 0078, H10), so at density zero it's clear too.
//!
//! The map is the React client's world: flat zone colours, no sun and no
//! shadow. Over the top of the zoom the sun and fill fade out, the sky's
//! light from above rises to `FLAT_SKY`, and the ambient occlusion goes, so
//! each surface shows its own colour, the §4.7 tokens. The ground's ambient
//! is left alone. The lights (`palette::glow`) give off their own light,
//! whatever the ambient, so the crown still glows gold at the centre.
//! The light is written as the scene's own components (the `sun` and
//! `fill` lights, the sky), which the renderer reads every frame.
//!
//! The heights are the camera's over the land under it (W11), so on a
//! ridge 50 m up the street view keeps its fog as it does in the city.
//!
//! The air also turns far things blue (the fog's aerial tint, Flint ADR
//! 0101, #233: the ranges stand blue round the city, and from a ridge the
//! far side of the ring does) and lays the valleys' mist (`world::mist`),
//! both clearing with the fog as the camera climbs. The tint's colour goes
//! with the hour (`Palette::aerial`); the scene authors the dusk's, and how
//! far off it comes in.
//!
//! The camera rig (`scripts/camera.rhai`) sets the heights: above
//! `CLEAR_FROM` it always looks down past the horizon (its pitch is over
//! 50° by then), so the thinning fog never shows the dark ground skirt
//! against the sky.

use flint_core::EntityId;
use flint_ecs::FlintWorld;
use flint_render::SceneRenderer;

use crate::daylight::{self, Daylight, Light, Palette, PlotGlow, Sky};

/// Up to this height (metres) the fog is the scene's own: every street,
/// quay and tower view.
pub const CLEAR_FROM: f32 = 100.0;
/// Above this the fog's density is zero. The district view (about 230 m) and
/// the map (880 m) see the city clear.
pub const CLEAR_BY: f32 = 250.0;
/// From this height the light starts to flatten: above the district view,
/// so the city keeps its dusk until it's well on the way to a map.
pub const FLAT_FROM: f32 = 280.0;
/// By this height the light is flat. The rig's map is 880 m up.
pub const FLAT_BY: f32 = 600.0;
// The fog is gone before the light starts to change.
const _: () = assert!(CLEAR_BY <= FLAT_FROM);
/// The light from above on a flat map, linear. Lit by this and nothing
/// else, a surface shows about its own colour after tone mapping.
pub const FLAT_SKY: [f32; 3] = [1.3, 1.3, 1.3];

/// The scene's key light (the sun, and by night the moon) and its fill.
const SUN: &str = "sun";
const FILL: &str = "fill";

/// The phase is written in steps of a day over this: under a real second.
const PHASE_STEPS: f64 = 8192.0;

#[derive(Default)]
pub struct Atmosphere {
    /// The scene as authored, read the first frame there is a renderer.
    authored: Option<Authored>,
    /// The time of day shown. It eases toward the clock's after a jump
    /// (`daylight::approach`); `None` until the first frame.
    shown: Option<f64>,
    /// The (fog, flat, phase step) last written, so a still camera at a
    /// pinned hour writes nothing, and a value tuned live in F3 holds until
    /// the camera climbs or falls or the day moves on.
    written: Option<(f32, f32, i64)>,
    /// How the gold on your plots shows at the hour shown, in steps.
    plots: Option<PlotGlow>,
    /// How thick the valleys' mist shows, the hour's times the fog's.
    mist: f32,
}

/// What the scene set, before any of it was thinned or flattened: the
/// day's evening.
struct Authored {
    fog_density: f32,
    fog_tint_strength: f32,
    ssao_intensity: f32,
    key: Light,
    evening: Palette,
    sun: Option<EntityId>,
    fill: Option<EntityId>,
    sky: Option<EntityId>,
}

impl Atmosphere {
    /// Light the scene for `phase` (the time of day, 0..1 from midnight),
    /// and thin the fog and flatten the light for a camera `height` metres
    /// up. `dt` eases a jump in the phase.
    pub fn update(
        &mut self,
        renderer: &mut SceneRenderer,
        world: &mut FlintWorld,
        height: f32,
        phase: f64,
        dt: f64,
    ) {
        let authored = self
            .authored
            .get_or_insert_with(|| Authored::read(renderer, world));
        let shown = match self.shown {
            Some(shown) => daylight::approach(shown, phase, dt),
            None => phase.rem_euclid(1.0),
        };
        self.shown = Some(shown);
        let state = (
            fog_scale(height),
            flatness(height),
            (shown * PHASE_STEPS).round() as i64,
        );
        if self.written == Some(state) {
            return;
        }
        self.written = Some(state);
        let (fog, flat, _) = state;
        let Daylight { key, palette } = daylight::at(shown, &authored.key, &authored.evening);
        self.plots = Some(palette.plots.stepped());
        self.mist = palette.mist * fog;

        let mut config = renderer.post_process_config().clone();
        config.fog_density = authored.fog_density * fog;
        config.fog_color = palette.fog_color;
        config.fog_tint_color = palette.aerial;
        config.fog_tint_strength = authored.fog_tint_strength * fog;
        config.exposure = palette.exposure;
        config.ssao_intensity = authored.ssao_intensity * (1.0 - flat);
        renderer.set_post_process_config(config);

        for (id, light) in [(authored.sun, key), (authored.fill, palette.fill)] {
            if let Some(id) = id {
                write_light(world, id, &light, 1.0 - flat);
            }
        }
        if let Some(id) = authored.sky {
            write_sky(world, id, &palette.sky, flat);
        }
    }

    /// How the gold on your own plots shows at the hour shown, in steps
    /// (`PlotGlow::stepped`): rebuild it when this changes. Evening until
    /// the first frame.
    pub fn plots(&self) -> PlotGlow {
        self.plots.unwrap_or(PlotGlow::EVENING)
    }

    /// How thick the valleys' mist shows (`world::mist`), 0 to 1: the
    /// hour's, cleared with the fog. None until the first frame.
    pub fn mist(&self) -> f32 {
        self.mist
    }
}

/// How high (m) a camera at world `eye` stands over the land under it:
/// what the fog and the light go by.
pub fn height_over_land(eye: [f32; 3]) -> f32 {
    eye[1] - crate::world::land::ground_y([eye[0], eye[2]])
}

/// `light`, its intensity times `dim`.
fn write_light(world: &mut FlintWorld, id: EntityId, light: &Light, dim: f32) {
    let mut table = toml::Table::new();
    table.insert("direction".into(), array(&light.direction));
    table.insert("color".into(), array(&light.color));
    table.insert("intensity".into(), f64::from(light.intensity * dim).into());
    merge(world, id, "light", table);
}

/// `sky`, its light from above eased toward `FLAT_SKY` by `flat`.
fn write_sky(world: &mut FlintWorld, id: EntityId, sky: &Sky, flat: f32) {
    let mut ambient = sky.ambient_sky;
    for (c, to) in ambient.iter_mut().zip(FLAT_SKY) {
        *c += (to - *c) * flat;
    }
    let mut table = toml::Table::new();
    for (field, value) in [
        ("zenith_color", sky.zenith),
        ("horizon_color", sky.horizon),
        ("haze_color", sky.haze),
        ("cloud_tint", sky.cloud_tint),
        ("ambient_sky", ambient),
    ] {
        table.insert(field.into(), array(&value));
    }
    for (field, value) in [
        ("cloud_coverage", sky.cloud_coverage),
        ("star_opacity", sky.star_opacity),
        ("sun_disc_size", sky.sun_disc_size),
        ("sun_glow", sky.sun_glow),
    ] {
        table.insert(field.into(), f64::from(value).into());
    }
    merge(world, id, "sky", table);
}

impl Authored {
    fn read(renderer: &SceneRenderer, world: &FlintWorld) -> Self {
        let config = renderer.post_process_config();
        let light = |name: &str| -> (Option<EntityId>, Light) {
            let id = world.get_id(name);
            if id.is_none() {
                tracing::warn!("atmosphere: the scene has no `{name}` light to drive");
            }
            let component = id.and_then(|id| world.get_component(id, "light"));
            let field = |f: &str| component.and_then(|c| c.get(f));
            let light = Light {
                direction: field("direction")
                    .and_then(floats::<3>)
                    .unwrap_or([0.0, 1.0, 0.0]),
                color: field("color").and_then(floats::<3>).unwrap_or([1.0; 3]),
                intensity: number(field("intensity")).unwrap_or(1.0),
            };
            (id, light)
        };
        let (sun, key) = light(SUN);
        let (fill, fill_light) = light(FILL);
        let sky = world.entities_with_component("sky").iter().min().copied();
        let params = sky
            .and_then(|id| world.get_component(id, "sky"))
            .map(flint_render::sky_pipeline::SkyParams::from_component)
            .unwrap_or_default();
        let ambient_sky = params.ambient_sky.unwrap_or_else(|| {
            let [r, g, b] = flint_render::LightUniforms::DEFAULT_AMBIENT_SKY;
            [r, g, b, 1.0]
        });
        let evening = Palette {
            sky: Sky {
                zenith: params.zenith_color,
                horizon: params.horizon_color,
                haze: params.haze_color,
                cloud_tint: params.cloud_tint,
                cloud_coverage: params.cloud_coverage,
                star_opacity: params.star_opacity,
                sun_disc_size: params.sun_disc_size,
                sun_glow: params.sun_glow,
                ambient_sky,
            },
            fill: fill_light,
            fog_color: config.fog_color,
            aerial: config.fog_tint_color,
            exposure: config.exposure,
            plots: PlotGlow::EVENING,
            mist: daylight::EVENING_MIST,
        };
        Self {
            fog_density: config.fog_density,
            fog_tint_strength: config.fog_tint_strength,
            ssao_intensity: config.ssao_intensity,
            key,
            evening,
            sun,
            fill,
            sky,
        }
    }
}

fn array(values: &[f32]) -> toml::Value {
    let values: Vec<toml::Value> = values.iter().map(|&c| f64::from(c).into()).collect();
    values.into()
}

fn floats<const N: usize>(v: &toml::Value) -> Option<[f32; N]> {
    let values: Vec<f32> = v
        .as_array()?
        .iter()
        .filter_map(|v| number(Some(v)))
        .collect();
    values.get(..N)?.try_into().ok()
}

pub(crate) fn number(v: Option<&toml::Value>) -> Option<f32> {
    match v? {
        toml::Value::Float(f) => Some(*f as f32),
        toml::Value::Integer(i) => Some(*i as f32),
        _ => None,
    }
}

fn merge(world: &mut FlintWorld, id: EntityId, component: &str, table: toml::Table) {
    if let Err(e) = world.merge_component(id, component, table.into()) {
        tracing::warn!("atmosphere: can't write {component}: {e}");
    }
}

/// How much of the scene's fog a camera `height` metres up sees: all of it
/// to `CLEAR_FROM`, easing to none at `CLEAR_BY`.
pub fn fog_scale(height: f32) -> f32 {
    1.0 - smoothstep(CLEAR_FROM, CLEAR_BY, height)
}

/// How flat the light is for a camera `height` metres up: 0 (the scene's
/// dusk) to `FLAT_FROM`, easing to 1 (a map) at `FLAT_BY`.
pub fn flatness(height: f32) -> f32 {
    smoothstep(FLAT_FROM, FLAT_BY, height)
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_street_has_all_the_fog_and_the_map_none() {
        // --view street, crown, from-wilderness: the scene's fog.
        for height in [3.0, 26.0, 64.0, 70.0] {
            assert_eq!(fog_scale(height), 1.0, "{height} m");
        }
        // --view district and map: clear.
        for height in [260.0, 880.0] {
            assert_eq!(fog_scale(height), 0.0, "{height} m");
        }
        let mut last = 1.0;
        for step in 0..=60 {
            let scale = fog_scale(step as f32 * 5.0);
            assert!(scale <= last, "the fog thickens as the camera climbs");
            last = scale;
        }
    }

    #[test]
    fn the_light_flattens_only_on_the_way_to_the_map() {
        // The street and the district keep their dusk.
        for height in [5.0, 25.0, 210.0, 260.0] {
            assert_eq!(flatness(height), 0.0, "{height} m");
        }
        // The rig's map (880 m) is flat.
        assert_eq!(flatness(880.0), 1.0);
    }
}
