//! `CapitolExtension`: everything the Capitol adds to Flint's player, as a
//! `PlayerExtension`. Today that's the script API, the session (the login
//! form, and once signed in the store and poller), the game egui, the
//! terrain (fetched and cached, or read from a file offline), the world
//! built from it (`world`, so far the ground, the canals, whose water is
//! the scene's ocean, the roads, the bridges, the plots and the Capitol
//! tower), the players walking it (`entities`), the fog and the light as
//! the camera climbs (`atmosphere`), the wheel for the camera rig, and the
//! spike's markers.

use crate::atmosphere::Atmosphere;
use crate::entities::Players;
use crate::script_api::{self, Shared, ZoneBanner};
use crate::session::Session;
use crate::terrain::TerrainSource;
use crate::ui::containers::BenchAction;
use crate::ui::hud::{ConnectionOverlay, HeaderView};
use crate::ui::icons::ItemIcons;
use crate::ui::login::{FormAction, LoginForm, LoginView};
use crate::world::palette;
use crate::world::plots::Lot;
use crate::world::Rebuild;
use crate::{spike_world, ui, world};
use capitol_core::auth::{Notice, Phase};
use capitol_core::net::ConnectionState;
use capitol_core::protocol::terrain::TerrainResponse;
use capitol_core::protocol::ws::ClientMessage;
use flint_player::{ExtensionCtx, PlayerExtension};
use std::sync::atomic::Ordering;
use std::time::Instant;
use winit::event::{MouseScrollDelta, WindowEvent};

/// Pixels of a touchpad's smooth scroll that make one wheel notch.
const PIXELS_PER_NOTCH: f32 = 50.0;

/// How long the login form takes to fade out once the player is in, in
/// seconds. The camera comes down from its circle over about the same time.
const FORM_FADE: f32 = 0.8;

pub struct CapitolExtension {
    shared: Shared,
    terrain: TerrainSource,
    /// `None` offline: no server, so no login, just the world.
    session: Option<Session>,
    form: LoginForm,
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
    icons: ItemIcons,
    /// `--icon-sheet`: every item icon on a panel, for review.
    icon_sheet: bool,
    /// The theme's sample panels, with `--ui-sample`.
    sample: Option<ui::sample::Sample>,
    /// `--containers`: every container as a slot grid (P5.3).
    containers: bool,
    players: Players,
    overlay: ConnectionOverlay,
}

impl CapitolExtension {
    pub fn new(shared: Shared, terrain: TerrainSource, session: Option<Session>) -> Self {
        Self {
            shared,
            terrain,
            session,
            form: LoginForm::default(),
            world_built: false,
            built: None,
            lots: Vec::new(),
            stats_logged: 0.0,
            atmosphere: Atmosphere::default(),
            icons: ItemIcons::load(std::path::Path::new(ItemIcons::DIR)),
            icon_sheet: false,
            sample: None,
            containers: false,
            players: Players::default(),
            overlay: ConnectionOverlay::default(),
        }
    }

    /// Show every item icon on a panel (`--icon-sheet`).
    pub fn with_icon_sheet(mut self, on: bool) -> Self {
        self.icon_sheet = on;
        self
    }

    /// Also show the theme's sample panels (`--ui-sample`).
    pub fn with_ui_sample(mut self) -> Self {
        self.sample = Some(ui::sample::Sample::default());
        self
    }

    /// Show every container as a slot grid once signed in (`--containers`).
    pub fn with_containers(mut self, on: bool) -> Self {
        self.containers = on;
        self
    }

    /// The login form is up: online, and not yet in the world.
    fn at_login(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.auth().phase() != Phase::InWorld)
    }

    fn act(&mut self, action: FormAction) {
        let Some(session) = &mut self.session else {
            return;
        };
        match action {
            FormAction::LogIn { email, password } => session.log_in(&email, &password),
            FormAction::Register {
                name,
                email,
                password,
            } => session.register(&name, &email, &password),
            FormAction::SignInAsSomeoneElse => session.log_out(),
            FormAction::SetServer(server) => self.switch_server(&server),
        }
    }

    /// A new session on `server`, and its terrain: the world is rebuilt from
    /// that server's cache, or once its `/terrain` arrives.
    fn switch_server(&mut self, server: &str) {
        let Some(old) = self.session.take() else {
            return;
        };
        tracing::info!("session: switching from {} to {server}", old.server());
        let mut file = old.into_file();
        file.set_server(server);
        match Session::start(server, file) {
            Ok(session) => {
                self.terrain = TerrainSource::online(server);
                self.session = Some(session);
            }
            Err(e) => tracing::error!("session: couldn't start for {server}: {e:#}"),
        }
    }
}

impl PlayerExtension for CapitolExtension {
    fn register_script_api(&mut self, engine: &mut rhai::Engine) {
        script_api::register(engine, &self.shared);
    }

    fn pre_update(&mut self, ctx: &mut ExtensionCtx) {
        let now = Instant::now();
        if let Some(session) = &mut self.session {
            session.update(now, &mut self.terrain);
            walk(ctx, session, &self.shared, now);
            // Signed out, the store is empty, so everyone fades away.
            let (poses, state) = session.sample(now);
            let dt = ctx.clock.delta_time.min(0.1) as f32;
            self.players.update(ctx, poses, state, dt, &self.shared);
            let banner = session.arrival().map(|a| ZoneBanner {
                name: a.zone.display_name().to_owned(),
                colour: palette::srgb(palette::zone_mark(a.zone)),
                at: a.at,
                first: a.first,
            });
            if let Ok(mut shared) = self.shared.banner.lock() {
                *shared = banner;
            }
        }
        self.shared
            .at_login
            .store(self.at_login(), Ordering::Relaxed);
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
            self.players.set_rings(rings);
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
        if !ui::theme::install(ctx) {
            return; // one frame, while the fonts go in
        }
        let at_login = self.at_login();
        let mut action = None;
        if let Some(session) = &mut self.session {
            // First, so every other panel is placed below it.
            let mut sign_out = false;
            if !at_login {
                let state = session.state();
                let view = HeaderView {
                    zone: state.zone(),
                    strands: state.strands(),
                    who: &session.who(),
                    connection: session.auth().connection(),
                    down: session.link().outage().is_some(),
                };
                sign_out |= ui::hud::header(ctx, &view);
            }
            let outage = if at_login {
                None
            } else {
                session.link().outage()
            };
            let connected = matches!(
                session.auth().connection(),
                Some(ConnectionState::Connected { .. })
            );
            sign_out |= self.overlay.show(ctx, outage, connected, Instant::now());
            if sign_out {
                session.log_out();
            }
            // Starts at its target, so a first frame in the world doesn't fade.
            let opacity = ctx.animate_bool_with_time(
                egui::Id::new("capitol_login_fade"),
                at_login,
                FORM_FADE,
            );
            if opacity > 0.0 {
                let auth = session.auth();
                let view = LoginView {
                    phase: auth.phase(),
                    notice: auth.notice().map(ToString::to_string),
                    wrong_password: auth.notice() == Some(&Notice::WrongPassword),
                    reachable: auth.reachable(),
                    server: session.server(),
                    remembered_email: auth.email(),
                    who: session.who(),
                    connection: auth.connection(),
                };
                action = self.form.show(ctx, &view, opacity);
            }
            if !at_login && self.containers {
                let refused = session.move_refused(Instant::now());
                match ui::containers::bench(ctx, &mut self.icons, session.state(), refused) {
                    Some(BenchAction::Move(m)) => session.move_item(m),
                    Some(BenchAction::OpenStation(id)) => session.ws(ClientMessage::OpenStation {
                        station_id: id.to_string(),
                    }),
                    Some(BenchAction::CloseStation) => session.ws(ClientMessage::CloseStation),
                    None => {}
                }
            }
        }
        if self.session.is_none() {
            ui::inscription(ctx, &self.shared, &self.terrain.status());
        } else if !at_login {
            ui::controls(ctx, &self.terrain.status());
        }
        if self.icon_sheet {
            ui::icons::sheet(ctx, &mut self.icons);
        }
        if let Some(sample) = &mut self.sample {
            sample.show(ctx, &mut self.icons);
        }
        if let Some(action) = action {
            self.act(action);
        }
    }
}

/// Click to move (P4.3): send the walk a script asked for (`cap_move`),
/// tell scripts where you're walking to (`cap_destination`), and fire
/// `movement_blocked` at them for each time the water stopped you, with
/// where you stand (`x`, `z`, world metres), which way you were going
/// (`dir_x`, `dir_z`, a unit vector, or 0 if unknown), the server's
/// `reason`, and the `water` it named, in words ("the outer canal").
fn walk(ctx: &mut ExtensionCtx, session: &mut Session, shared: &script_api::Shared, now: Instant) {
    if let Some([x, y]) = shared.move_to.lock().ok().and_then(|mut m| m.take()) {
        session.move_to(now, x, y);
    }
    if let Ok(mut d) = shared.destination.lock() {
        *d = session
            .destination()
            .map(|(x, y)| crate::coords::to_world(x, y));
    }
    for blocked in session.take_blocked() {
        let [x, z] = crate::coords::to_world(blocked.x, blocked.y);
        let (mut dx, mut dz) = match blocked.toward {
            Some((tx, ty)) => ((tx - blocked.x) as f32, (ty - blocked.y) as f32),
            None => (0.0, 0.0),
        };
        let len = dx.hypot(dz);
        if len > 1e-3 {
            (dx, dz) = (dx / len, dz / len);
        } else {
            (dx, dz) = (0.0, 0.0);
        }
        tracing::info!(
            "move: blocked at ({:.0}, {:.0}): {}",
            blocked.x,
            blocked.y,
            blocked.reason
        );
        let mut data = toml::Table::new();
        data.insert("x".into(), (x as f64).into());
        data.insert("z".into(), (z as f64).into());
        data.insert("dir_x".into(), (dx as f64).into());
        data.insert("dir_z".into(), (dz as f64).into());
        data.insert("water".into(), water_name(&blocked.reason).into());
        data.insert("reason".into(), blocked.reason.into());
        ctx.fire_event("movement_blocked", data);
    }
}

/// The water a `movement_blocked` reason names, for people: the server says
/// "Cannot cross canal_outer without a bridge", with the ring's id. A
/// workaround: the message has no field for it (issue #74).
fn water_name(reason: &str) -> String {
    let id = reason
        .strip_prefix("Cannot cross ")
        .and_then(|r| r.strip_suffix(" without a bridge"))
        .unwrap_or("water");
    match id {
        "moat" => "the moat".into(),
        "canal_inner" => "the inner canal".into(),
        "canal_outer" => "the outer canal".into(),
        "water" => "the water".into(),
        other => format!("the {}", other.replace('_', " ")),
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

#[cfg(test)]
mod tests {
    use super::water_name;

    #[test]
    fn the_water_is_named_for_people() {
        let name = |id: &str| water_name(&format!("Cannot cross {id} without a bridge"));
        assert_eq!(name("canal_outer"), "the outer canal");
        assert_eq!(name("moat"), "the moat");
        assert_eq!(name("canal_far"), "the canal far");
        assert_eq!(water_name("something else"), "the water");
    }
}
