//! The air over the city (P3.8), and the light it lets through. The scene
//! sets the dusk: the sky, the low sun, and a fog the colour of the sky's
//! horizon, so the land runs out into the sky with no line where they meet.
//! Here, as the camera climbs, the fog thins and is gone by the district
//! view, and then the light flattens into a map (P3.9).
//!
//! Fog works on distance from the camera, and from high up all of the city
//! is far away: at the map view (640 m up) the scene's fog would cover it in
//! haze. Climbing out of the fog also turns it off, not just thins it,
//! because Flint's ocean fogs itself over a fixed 380–640 m whenever fog is
//! on, whatever its density, and that would wash the canals out of the map.
//! A workaround: H10 (issue #66) lets the fog reach everything it should.
//!
//! The map is the React client's world: flat zone colours, no sun and no
//! shadow. Over the top of the zoom the sun and fill fade out, the sky's
//! light from above rises to `FLAT_SKY`, and the ambient occlusion goes, so
//! each surface shows its own colour, the §4.7 tokens. The ground's ambient
//! is left alone. The lights (`palette::glow`) give off their own light,
//! whatever the ambient, so the crown still glows gold at the centre.
//! The light is written as the scene's own components (each light's
//! `intensity`, the sky's `ambient_sky`), which the renderer reads every
//! frame.
//!
//! The camera rig (`scripts/camera.rhai`) sets the heights: above
//! `CLEAR_FROM` it always looks down past the horizon (its pitch is over
//! 50° by then), so the thinning fog never shows the dark ground skirt
//! against the sky.

use flint_core::EntityId;
use flint_ecs::FlintWorld;
use flint_render::SceneRenderer;

/// Up to this height (metres) the fog is the scene's own: every street,
/// quay and tower view.
pub const CLEAR_FROM: f32 = 100.0;
/// Above this the fog is off. The district view (about 230 m) and the map
/// (640 m) see the city clear.
pub const CLEAR_BY: f32 = 250.0;
/// From this height the light starts to flatten: above the district view,
/// so the city keeps its dusk until it's well on the way to a map.
pub const FLAT_FROM: f32 = 280.0;
/// By this height the light is flat. The rig's map is 640 m up.
pub const FLAT_BY: f32 = 600.0;
// The fog is gone before the light starts to change.
const _: () = assert!(CLEAR_BY <= FLAT_FROM);
/// The light from above on a flat map, linear. Lit by this and nothing
/// else, a surface shows about its own colour after tone mapping.
pub const FLAT_SKY: [f32; 3] = [1.3, 1.3, 1.3];

#[derive(Default)]
pub struct Atmosphere {
    /// The scene as authored, read the first frame there is a renderer.
    authored: Option<Authored>,
    /// The (fog, flat) pair last written, so a still camera writes nothing,
    /// and a value tuned live in F3 holds until the camera climbs or falls.
    written: Option<(f32, f32)>,
}

/// What the scene set, before any of it was thinned or flattened.
struct Authored {
    fog_enabled: bool,
    fog_density: f32,
    ssao_intensity: f32,
    /// Each light and its `intensity`.
    lights: Vec<(EntityId, f32)>,
    /// The sky entity and its `ambient_sky` (RGBA), if it sets one.
    sky: Option<(EntityId, [f32; 4])>,
}

impl Atmosphere {
    /// Thin the fog and flatten the light for a camera `height` metres up.
    pub fn update(&mut self, renderer: &mut SceneRenderer, world: &mut FlintWorld, height: f32) {
        let authored = self
            .authored
            .get_or_insert_with(|| Authored::read(renderer, world));
        let state = (fog_scale(height), flatness(height));
        if self.written == Some(state) {
            return;
        }
        self.written = Some(state);
        let (fog, flat) = state;

        let mut config = renderer.post_process_config().clone();
        config.fog_enabled = authored.fog_enabled && fog > 0.0;
        config.fog_density = authored.fog_density * fog;
        config.ssao_intensity = authored.ssao_intensity * (1.0 - flat);
        renderer.set_post_process_config(config);

        for &(id, intensity) in &authored.lights {
            let dimmed = f64::from(intensity * (1.0 - flat));
            write(world, id, "light", "intensity", dimmed.into());
        }
        if let Some((id, sky)) = authored.sky {
            let mut lit = sky;
            for (c, to) in lit.iter_mut().zip(FLAT_SKY) {
                *c += (to - *c) * flat;
            }
            let lit: Vec<toml::Value> = lit.iter().map(|&c| f64::from(c).into()).collect();
            write(world, id, "sky", "ambient_sky", lit.into());
        }
    }
}

impl Authored {
    fn read(renderer: &SceneRenderer, world: &FlintWorld) -> Self {
        let config = renderer.post_process_config();
        let mut lights: Vec<(EntityId, f32)> = world
            .entities_with_component("light")
            .iter()
            .filter_map(|&id| {
                let light = world.get_component(id, "light")?;
                Some((id, number(light.get("intensity")).unwrap_or(1.0)))
            })
            .collect();
        lights.sort_by_key(|(id, _)| *id);
        let sky = world.entities_with_component("sky").iter().find_map(|&id| {
            let field = world.get_component(id, "sky")?.get("ambient_sky")?;
            let rgba: Vec<f32> = field
                .as_array()?
                .iter()
                .filter_map(|v| number(Some(v)))
                .collect();
            match rgba[..] {
                [r, g, b] => Some((id, [r, g, b, 1.0])),
                [r, g, b, a, ..] => Some((id, [r, g, b, a])),
                _ => None,
            }
        });
        Self {
            fog_enabled: config.fog_enabled,
            fog_density: config.fog_density,
            ssao_intensity: config.ssao_intensity,
            lights,
            sky,
        }
    }
}

fn number(v: Option<&toml::Value>) -> Option<f32> {
    match v? {
        toml::Value::Float(f) => Some(*f as f32),
        toml::Value::Integer(i) => Some(*i as f32),
        _ => None,
    }
}

fn write(world: &mut FlintWorld, id: EntityId, component: &str, field: &str, value: toml::Value) {
    let mut table = toml::Table::new();
    table.insert(field.into(), value);
    if let Err(e) = world.merge_component(id, component, table.into()) {
        tracing::warn!("atmosphere: can't write {component}.{field}: {e}");
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
        for height in [260.0, 640.0] {
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
        // The rig's map (640 m) is flat.
        assert_eq!(flatness(640.0), 1.0);
    }
}
