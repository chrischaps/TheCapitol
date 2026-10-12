//! `CapitolExtension`: everything the Capitol adds to Flint's player, as a
//! `PlayerExtension`. Today that's the script API, the game egui and the
//! spike's runtime meshes; the network drain joins `pre_update` in P2–P3.

use crate::script_api::{self, Shared};
use crate::{spike_world, ui};
use flint_player::{ExtensionCtx, PlayerExtension};

pub struct CapitolExtension {
    shared: Shared,
    /// Whether the ground and markers have been uploaded.
    world_built: bool,
}

impl CapitolExtension {
    pub fn new(shared: Shared) -> Self {
        Self {
            shared,
            world_built: false,
        }
    }
}

impl PlayerExtension for CapitolExtension {
    fn register_script_api(&mut self, engine: &mut rhai::Engine) {
        script_api::register(engine, &self.shared);
    }

    fn pre_update(&mut self, ctx: &mut ExtensionCtx) {
        let t = ctx.clock.total_time;
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        if !self.world_built {
            spike_world::upload_static(gpu);
            self.world_built = true;
        }
        spike_world::upload_water(gpu, t);
    }

    fn ui(&mut self, ctx: &egui::Context) {
        ui::inscription(ctx, &self.shared);
    }
}
