//! `CapitolExtension`: everything the Capitol adds to Flint's player, as a
//! `PlayerExtension`. Today that's the script API, the game egui, the terrain
//! (fetched and cached, or read from a file offline), the world built from
//! it (`world`, so far the ground, the canals, whose water is the scene's
//! ocean, and the roads) and the spike's markers.

use crate::script_api::{self, Shared};
use crate::terrain::TerrainSource;
use crate::{spike_world, ui, world};
use flint_player::{ExtensionCtx, PlayerExtension};

pub struct CapitolExtension {
    shared: Shared,
    terrain: TerrainSource,
    /// Whether the spike's markers have been uploaded.
    world_built: bool,
}

impl CapitolExtension {
    pub fn new(shared: Shared, terrain: TerrainSource) -> Self {
        Self {
            shared,
            terrain,
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
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        if !self.world_built {
            spike_world::upload_static(gpu);
            self.world_built = true;
        }
        // Held until the renderer exists, so a terrain that arrives first
        // (offline, or from the cache) isn't consumed with nothing to draw on.
        let Some(terrain) = self.terrain.take_changed() else {
            return;
        };
        let rings = &terrain.water_rings;
        world::ground::upload(gpu, terrain.world_size, &world::water::holes(rings));
        world::water::upload(gpu, rings, terrain.world_size);
        let roads = world::roads::upload(gpu, &terrain.roads, rings);
        // One entity per sector mesh, made the first time that mesh has
        // anything in it. A rebuild replaces the meshes by name, so the
        // entities stay; one whose mesh is now empty draws nothing.
        for name in roads {
            if ctx.world.get_id(&name).is_some() {
                continue;
            }
            let attached = ctx
                .world
                .spawn(name.as_str())
                .and_then(|id| ctx.attach_mesh(id, &name));
            if let Err(e) = attached {
                tracing::warn!("roads: no entity for {name}: {e}");
            }
        }
    }

    fn ui(&mut self, ctx: &egui::Context) {
        ui::inscription(ctx, &self.shared, &self.terrain.status());
    }
}
