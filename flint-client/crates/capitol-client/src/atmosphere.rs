//! The air over the city (P3.8). The scene sets the dusk: the sky, the low
//! sun, and a fog the colour of the sky's horizon, so the land runs out into
//! the sky with no line where they meet. Here the fog thins as the camera
//! climbs, and it's gone by map height.
//!
//! Fog works on distance from the camera, and from high up all of the city
//! is far away: at the map view (620 m up) the scene's fog would cover it in
//! haze. Climbing out of the fog also turns it off, not just thins it,
//! because Flint's ocean fogs itself over a fixed 380–640 m whenever fog is
//! on, whatever its density, and that would wash the canals out of the map.
//! A workaround: H10 (issue #66) lets the fog reach everything it should.
//!
//! The camera rig (P3.9) owns zoom, and it can retune the heights here.
//! Until fog thins on its own, keep the horizon out of frame above
//! `CLEAR_FROM`: thin fog leaves the dark ground skirt showing against the
//! sky.

use flint_render::SceneRenderer;

/// Up to this height (metres) the fog is the scene's own: every street,
/// quay and tower view.
pub const CLEAR_FROM: f32 = 100.0;
/// Above this the fog is off. The district view (260 m) and the map (620 m)
/// see the city clear.
pub const CLEAR_BY: f32 = 250.0;

#[derive(Default)]
pub struct Atmosphere {
    /// The scene's fog (`[post_process]`), read once the renderer has it.
    authored: Option<Fog>,
    /// The scale last written, so a still camera writes nothing, and a
    /// density tuned live in F3 holds until the camera climbs or falls.
    written: Option<f32>,
}

#[derive(Clone, Copy)]
struct Fog {
    enabled: bool,
    density: f32,
}

impl Atmosphere {
    /// Thin the fog for a camera `height` metres up.
    pub fn update(&mut self, renderer: &mut SceneRenderer, height: f32) {
        let mut config = renderer.post_process_config().clone();
        let authored = *self.authored.get_or_insert(Fog {
            enabled: config.fog_enabled,
            density: config.fog_density,
        });
        let scale = fog_scale(height);
        if self.written == Some(scale) {
            return;
        }
        self.written = Some(scale);
        config.fog_enabled = authored.enabled && scale > 0.0;
        config.fog_density = authored.density * scale;
        renderer.set_post_process_config(config);
    }
}

/// How much of the scene's fog a camera `height` metres up sees: all of it
/// to `CLEAR_FROM`, easing to none at `CLEAR_BY`.
pub fn fog_scale(height: f32) -> f32 {
    let t = ((height - CLEAR_FROM) / (CLEAR_BY - CLEAR_FROM)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
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
        for height in [260.0, 620.0] {
            assert_eq!(fog_scale(height), 0.0, "{height} m");
        }
        let mut last = 1.0;
        for step in 0..=60 {
            let scale = fog_scale(step as f32 * 5.0);
            assert!(scale <= last, "the fog thickens as the camera climbs");
            last = scale;
        }
    }
}
