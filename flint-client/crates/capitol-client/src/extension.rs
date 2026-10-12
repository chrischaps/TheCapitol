//! `CapitolExtension`: everything the Capitol adds to Flint's player, as a
//! `PlayerExtension`. Today that's the script API, the game egui, the terrain
//! (fetched and cached, or read from a file offline), the world built from
//! it (`world`, so far the ground and water) and the spike's markers.

use crate::script_api::{self, Shared};
use crate::terrain::TerrainSource;
use crate::world::water::Water;
use crate::{spike_world, ui, world};
use flint_player::{ExtensionCtx, PlayerExtension};

pub struct CapitolExtension {
    shared: Shared,
    terrain: TerrainSource,
    /// The water's surface, recoloured and uploaded every frame.
    water: Water,
    /// Whether the spike's markers have been uploaded.
    world_built: bool,
}

impl CapitolExtension {
    pub fn new(shared: Shared, terrain: TerrainSource) -> Self {
        Self {
            shared,
            terrain,
            water: Water::default(),
            world_built: false,
        }
    }
}

impl PlayerExtension for CapitolExtension {
    fn register_script_api(&mut self, engine: &mut rhai::Engine) {
        script_api::register(engine, &self.shared);
    }

    fn pre_update(&mut self, ctx: &mut ExtensionCtx) {
        self.terrain.update();
        let t = ctx.clock.total_time;
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        if !self.world_built {
            spike_world::upload_static(gpu);
            self.world_built = true;
        }
        // Held until the renderer exists, so a terrain that arrives first
        // (offline, or from the cache) isn't consumed with nothing to draw on.
        if let Some(terrain) = self.terrain.take_changed() {
            world::ground::upload(gpu, terrain.world_size);
            world::water::upload_banks(gpu, &terrain.water_rings);
            self.water = Water::new(&terrain.water_rings);
        }
        self.water.upload(gpu, t);
    }

    fn ui(&mut self, ctx: &egui::Context) {
        ui::inscription(ctx, &self.shared, &self.terrain.status());
    }
}
