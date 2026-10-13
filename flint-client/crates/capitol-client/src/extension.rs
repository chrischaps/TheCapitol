//! `CapitolExtension`: everything the Capitol adds to Flint's player, as a
//! `PlayerExtension`. Today that's the script API, the game egui, the terrain
//! (fetched and cached, or read from a file offline), the world built from
//! it (`world`, so far the ground, the canals, whose water is the scene's
//! ocean, the roads, the bridges, the plots and the Capitol tower), the
//! fog and the light as the camera climbs (`atmosphere`), the wheel for
//! the camera rig, and the spike's markers.

use crate::atmosphere::Atmosphere;
use crate::script_api::{self, Shared};
use crate::terrain::TerrainSource;
use crate::world::plots::Lot;
use crate::world::Rebuild;
use crate::{spike_world, ui, world};
use capitol_core::protocol::terrain::TerrainResponse;
use flint_player::{ExtensionCtx, PlayerExtension};
use std::time::Instant;
use winit::event::{MouseScrollDelta, WindowEvent};

/// Pixels of a touchpad's smooth scroll that make one wheel notch.
const PIXELS_PER_NOTCH: f32 = 50.0;

pub struct CapitolExtension {
    shared: Shared,
    terrain: TerrainSource,
    /// Whether the spike's markers have been uploaded.
    world_built: bool,
    /// The terrain the world was last built from, to tell what a new one
    /// changes (`world::Rebuild`).
    built: Option<TerrainResponse>,
    /// Where each of the terrain's plots is in the plot meshes, in order.
    #[allow(dead_code)] // P6.4 finds a lot here to highlight it
    lots: Vec<Option<Lot>>,
    /// Game time of the last render-stats line (debug log only).
    stats_logged: f64,
    atmosphere: Atmosphere,
}

impl CapitolExtension {
    pub fn new(shared: Shared, terrain: TerrainSource) -> Self {
        Self {
            shared,
            terrain,
            world_built: false,
            built: None,
            lots: Vec::new(),
            stats_logged: 0.0,
            atmosphere: Atmosphere::default(),
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
        self.atmosphere
            .update(gpu.renderer, ctx.world, ctx.camera.position.y);
        // How much the sectors cull: with RUST_LOG=capitol=debug, the last
        // frame's draws every 2 s. A workaround: captures can't open F2's
        // overlay until Flint lets them (H9, issue #65).
        let now = ctx.clock.total_time;
        if tracing::enabled!(tracing::Level::DEBUG) && now - self.stats_logged >= 2.0 {
            self.stats_logged = now;
            let stats = gpu.renderer.collect_stats();
            tracing::debug!(
                "render: {} entity draws, {} culled, {} triangles",
                stats.entity_draws,
                stats.culled_draws,
                stats.triangles
            );
        }
        if !self.world_built {
            spike_world::upload_static(gpu);
            self.world_built = true;
        }
        // Held until the renderer exists, so a terrain that arrives first
        // (offline, or from the cache) isn't consumed with nothing to draw on.
        let Some(terrain) = self.terrain.take_changed() else {
            return;
        };
        // A rebuild replaces each mesh by name, so the old buffers are freed
        // at the next draw, and empties the sectors the new terrain leaves bare.
        let rebuild = Rebuild::between(self.built.as_ref(), terrain);
        let started = Instant::now();
        let rings = &terrain.water_rings;
        if rebuild.ground {
            world::ground::upload(gpu, terrain.world_size, &world::water::holes(rings));
        }
        if rebuild.rings {
            world::water::upload(gpu, rings);
            world::bridges::upload(gpu, rings);
            world::tower::upload(gpu, rings);
        }
        let mut sectors = Vec::new();
        if rebuild.roads {
            sectors.extend(world::roads::upload(gpu, &terrain.roads, rings));
        }
        if rebuild.plots {
            let (plots, lots) =
                world::plots::upload(gpu, &terrain.terrain_plots, &terrain.roads, rings);
            sectors.extend(plots);
            self.lots = lots;
        }
        if self.built.is_some() {
            tracing::info!(
                "world: rebuilt {rebuild} in {} ms",
                started.elapsed().as_millis()
            );
        }
        self.built = Some(terrain.clone());
        attach_each(ctx, sectors.iter());
    }

    /// Collect the wheel for the camera rig (`cap_take_wheel`), because
    /// Flint doesn't feed it to actions. A workaround: issue #67. Never
    /// consumed; egui has already kept any wheel over its windows.
    fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        if let WindowEvent::MouseWheel { delta, .. } = event {
            let notches = match delta {
                MouseScrollDelta::LineDelta(_, y) => *y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / PIXELS_PER_NOTCH,
            };
            if let Ok(mut wheel) = self.shared.wheel.lock() {
                *wheel += notches;
            }
        }
        false
    }

    fn ui(&mut self, ctx: &egui::Context) {
        ui::inscription(ctx, &self.shared, &self.terrain.status());
    }
}

/// One entity per sector mesh, made the first time that mesh has anything
/// in it. A rebuild replaces the meshes by name, so the entities stay; one
/// whose mesh is now empty draws nothing.
fn attach_each<'a>(ctx: &mut ExtensionCtx, names: impl Iterator<Item = &'a String>) {
    for name in names {
        if ctx.world.get_id(name).is_some() {
            continue;
        }
        let attached = ctx
            .world
            .spawn(name.as_str())
            .and_then(|id| ctx.attach_mesh(id, name));
        if let Err(e) = attached {
            tracing::warn!("world: no entity for {name}: {e}");
        }
    }
}
