//! `CapitolExtension`: everything the Capitol adds to Flint's player, as a
//! `PlayerExtension`. Today that's the script API and the game egui; the
//! network drain (`pre_update`) and runtime meshes join it in P1–P3.

use crate::script_api::{self, Shared};
use crate::ui;
use flint_player::PlayerExtension;

pub struct CapitolExtension {
    shared: Shared,
}

impl CapitolExtension {
    pub fn new(shared: Shared) -> Self {
        Self { shared }
    }
}

impl PlayerExtension for CapitolExtension {
    fn register_script_api(&mut self, engine: &mut rhai::Engine) {
        script_api::register(engine, &self.shared);
    }

    fn ui(&mut self, ctx: &egui::Context) {
        ui::inscription(ctx, &self.shared);
    }
}
