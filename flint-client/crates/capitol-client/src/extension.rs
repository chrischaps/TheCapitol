//! `CapitolExtension`: everything the Capitol adds to Flint's player, as a
//! `PlayerExtension`. Today that's the script API, the session (the login
//! form, and once signed in the store and poller), the game egui, the
//! terrain (fetched and cached, or read from a file offline), the world
//! built from it (`world`, so far the ground, the canals, whose water is
//! the scene's ocean, the roads, the bridges, the plots and the Capitol
//! tower), the players walking it (`entities`), the resource nodes they
//! gather from (`nodes`), the stations they build (`stations`) and place
//! from a kit (`placing`), the crafting panel (`ui::crafting`), the plots you
//! pick, claim and own (`ui::plot`, and the gold of `world::plots::owned`),
//! the station panel (`ui::station`), the first-session hints (`ui::hints`)
//! and the shortcut overlay (`ui::shortcuts`), the fog and the light as
//! the camera climbs (`atmosphere`), what you hear (`sound`), the player's
//! settings and their panel (`settings`, `ui::settings`), the wheel for
//! the camera rig, and the spike's markers.

use crate::atmosphere::Atmosphere;
use crate::entities::Players;
use crate::juice_sample::JuiceSample;
use crate::nodes::Nodes;
use crate::script_api::{
    self, PlacementMark, PlotMark, Shared, TradeMark, UiAnchor, ZoneBanner,
};
use crate::session::Session;
use crate::settings::{Graphics, SettingsFile};
use crate::sound::{Ears, Rustling, Soundscape};
use crate::stations::{GhostView, Stations};
use crate::terrain::TerrainSource;
use crate::ui::crafting::Action as CraftAction;
use crate::ui::hud::{ConnectionOverlay, HeaderView, StrandTicker};
use crate::ui::icons::ItemIcons;
use crate::ui::inventory::InventoryPanel;
use crate::ui::login::{FormAction, LoginForm, LoginView};
use crate::ui::station::Action as StationAction;
use crate::world::palette;
use crate::world::plots::{self as lots, Lot};
use crate::world::Rebuild;
use crate::{spike_world, ui, world};
use capitol_core::auth::{Notice, Phase};
use capitol_core::harvest::Note;
use capitol_core::net::ConnectionState;
use capitol_core::onboarding::Did;
use capitol_core::plot::Offer;
use capitol_core::protocol::rules::{STATION_INTERACTION_RANGE, STATION_MIN_SPACING};
use capitol_core::protocol::terrain::TerrainResponse;
use capitol_core::station::{Note as PlaceNote, Phase as PlacePhase};
use flint_player::{ExtensionCtx, PlayerExtension};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key, KeyCode, PhysicalKey};

/// Pixels of a touchpad's smooth scroll that make one wheel notch.
const PIXELS_PER_NOTCH: f32 = 50.0;

/// How long the login form takes to fade out once the player is in, in
/// seconds. The camera comes down from its circle over about the same time.
const FORM_FADE: f32 = 0.8;

/// How far the cursor may move between a right press and its release, in
/// pixels, for it to be a click (cancel placing) rather than an orbit.
const RIGHT_CLICK_SLOP: f64 = 6.0;

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
    lots: Vec<Option<Lot>>,
    /// Each lot's place in `lots`, by its `TerrainPlot::id`, which a
    /// claimable plot names as `PlotInfo::terrain_id` (B7).
    lot_of: HashMap<String, usize>,
    /// The lots `world::plots::OWNED_MESH` was last built over, or `None`
    /// to build it again.
    owned_drawn: Option<Vec<usize>>,
    plot_panel: ui::plot::PlotPanel,
    /// Game time of the last render-stats line (debug log only).
    stats_logged: f64,
    atmosphere: Atmosphere,
    icons: ItemIcons,
    /// `--icon-sheet`: every item icon on a panel, for review.
    icon_sheet: bool,
    /// The theme's sample panels, with `--ui-sample`.
    sample: Option<ui::sample::Sample>,
    /// `--containers`: the containers no panel shows yet (P5.3).
    containers: bool,
    inventory: InventoryPanel,
    players: Players,
    overlay: ConnectionOverlay,
    /// The header's balance, counting to each new one (P7.1).
    strands: StrandTicker,
    nodes: Nodes,
    stations: Stations,
    crafting: ui::crafting::CraftingPanel,
    exchange: ui::exchange::ExchangePanel,
    station: ui::station::StationPanel,
    /// Enter (the `craft` action) was pressed since `ui` last looked.
    craft_key: bool,
    /// Esc, or a right click on the world, since the last frame: stop
    /// placing (P6.2).
    cancel_placing: bool,
    /// The cursor, physical pixels, and where the right button went down.
    cursor: (f64, f64),
    right_down: Option<(f64, f64)>,
    /// `--place`: the kit to start placing once the inventory holds one.
    place_on_arrival: Option<String>,
    /// The ambience and the one-shots (P8.1).
    sound: Soundscape,
    /// `--juice-sample`: the juice on a timetable, offline (P8.2).
    juice: Option<JuiceSample>,
    trade: ui::trade::TradePanel,
    /// `--trade-offer`: what to put in each trade as it opens, and the
    /// trade it was last put in.
    trade_offer: Option<(String, i64)>,
    trade_offered: Option<uuid::Uuid>,
    /// The first-session hints, and the shortcut overlay (P8.3).
    hints: ui::hints::Hints,
    shortcuts: ui::shortcuts::Shortcuts,
    /// The player's settings, and the file they're kept in (P8.4).
    settings: SettingsFile,
    settings_panel: ui::settings::SettingsPanel,
    /// The graphics last applied to the renderer, or `None` before the
    /// first frame with one.
    graphics: Option<Graphics>,
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
            lot_of: HashMap::new(),
            owned_drawn: None,
            plot_panel: ui::plot::PlotPanel::default(),
            stats_logged: 0.0,
            atmosphere: Atmosphere::default(),
            icons: ItemIcons::load(std::path::Path::new(ItemIcons::DIR)),
            icon_sheet: false,
            sample: None,
            containers: false,
            inventory: InventoryPanel::default(),
            players: Players::default(),
            overlay: ConnectionOverlay::default(),
            strands: StrandTicker::default(),
            nodes: Nodes::default(),
            stations: Stations::default(),
            crafting: ui::crafting::CraftingPanel::default(),
            exchange: ui::exchange::ExchangePanel::default(),
            station: ui::station::StationPanel::default(),
            craft_key: false,
            cancel_placing: false,
            cursor: (0.0, 0.0),
            right_down: None,
            place_on_arrival: None,
            sound: Soundscape::new(false),
            juice: None,
            trade: ui::trade::TradePanel::default(),
            trade_offer: None,
            trade_offered: None,
            hints: ui::hints::Hints::default(),
            shortcuts: ui::shortcuts::Shortcuts::default(),
            settings: SettingsFile::load(crate::settings::Settings::PATH),
            settings_panel: ui::settings::SettingsPanel::default(),
            graphics: None,
        }
    }

    /// Keep the settings in `file` rather than `save/settings.toml`
    /// (`--settings-file`), and open their panel at start if `open`
    /// (`--settings-panel`, for captures).
    pub fn with_settings(mut self, file: SettingsFile, open: bool) -> Self {
        self.settings = file;
        self.settings_panel.open = open;
        self
    }

    /// Play no sound (`--mute`, and every capture).
    pub fn with_mute(mut self, on: bool) -> Self {
        self.sound = Soundscape::new(on);
        self
    }

    /// Play the juice on a timetable, offline (`--juice-sample`).
    pub fn with_juice_sample(mut self, on: bool) -> Self {
        self.juice = on.then(JuiceSample::default);
        self
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

    /// Start placing the first `item_type` kit once signed in (`--place`).
    /// A workaround: captures can't right-click egui (issue #76).
    pub fn with_place(mut self, item_type: Option<String>) -> Self {
        self.place_on_arrival = item_type;
        self
    }

    /// Open the exchange panel preset (`--exchange deposit|withdraw:N`).
    /// A workaround: captures can't click or type into egui (issue #76).
    pub fn with_exchange(mut self, spec: Option<&str>) -> Self {
        if let Some(spec) = spec {
            self.exchange.preset(spec);
        }
        self
    }

    /// Offer this in each trade as it opens (`--trade-offer ITEM[:N]`).
    /// A workaround: captures can't drag into egui (issue #76).
    pub fn with_trade_offer(mut self, spec: Option<&str>) -> Self {
        self.trade_offer = spec.map(|s| {
            let (item, strands) = s.split_once(':').unwrap_or((s, "0"));
            (item.to_owned(), strands.parse().unwrap_or(0))
        });
        self
    }

    /// The login form is up: online, and not yet in the world.
    fn at_login(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.auth().phase() != Phase::InWorld)
    }

    /// Signed in and in the world, where the shortcuts and hints apply.
    fn in_world(&self) -> bool {
        self.session.is_some() && !self.at_login()
    }

    fn toggle_inventory(&mut self) {
        self.inventory.toggle();
        self.hints.saw(Did::UsedInventory);
    }

    fn toggle_crafting(&mut self) {
        self.crafting.toggle();
        if self.crafting.open {
            self.hints.saw(Did::OpenedCrafting);
        }
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

impl CapitolExtension {
    /// Apply the settings live (P8.4): the volumes to the soundscape, the
    /// camera's feel to `cap_setting`, and the graphics to the renderer
    /// whenever they change. Then write the file once they settle.
    fn apply_settings(&mut self, ctx: &mut ExtensionCtx, now: Instant) {
        let settings = &self.settings.settings;
        self.sound.set_volumes(settings.audio);
        if let Ok(mut feel) = self.shared.camera_feel.lock() {
            *feel = settings.camera;
        }
        if self.graphics != Some(settings.graphics) {
            if let Some(gpu) = ctx.gpu.as_mut() {
                settings.graphics.apply(self.graphics.as_ref(), gpu);
                tracing::info!("settings: graphics {:?}", settings.graphics);
                self.graphics = Some(settings.graphics);
            }
        }
        self.settings.update(now);
    }
}

impl PlayerExtension for CapitolExtension {
    fn register_script_api(&mut self, engine: &mut rhai::Engine) {
        script_api::register(engine, &self.shared);
    }

    fn pre_update(&mut self, ctx: &mut ExtensionCtx) {
        let now = Instant::now();
        if let Some(juice) = &mut self.juice {
            juice.update(ctx, &self.shared);
        }
        // Never while a text field has the keys: egui consumes those.
        if !self.at_login() && ctx.input.is_action_just_pressed("inventory") {
            self.toggle_inventory();
        }
        if !self.at_login() && ctx.input.is_action_just_pressed("crafting") {
            self.toggle_crafting();
        }
        if self.in_world() && ctx.input.is_action_just_pressed("help") {
            self.shortcuts.toggle();
        }
        if !self.at_login() && ctx.input.is_action_just_pressed("exchange") {
            self.exchange.toggle();
        }
        if ctx.input.is_action_just_pressed("settings") {
            self.settings_panel.toggle();
        }
        self.apply_settings(ctx, now);
        // Taken in `ui`, where the panel knows its recipe.
        self.craft_key |= !self.at_login() && ctx.input.is_action_just_pressed("craft");
        let station_key = !self.at_login() && ctx.input.is_action_just_pressed("station");
        let trade_key = !self.at_login() && ctx.input.is_action_just_pressed("trade");
        if let Some(session) = &mut self.session {
            session.update(now, &mut self.terrain);
            if station_key {
                session.open_nearest_station(now);
            }
            if trade_key {
                session.trade_key(now);
            }
            walk(ctx, session, &self.shared, now);
            let gathered = gather(ctx, session, &self.shared, &mut self.sound, now);
            visit(ctx, session, &self.shared, now);
            let made = crafted(ctx, session, &self.shared, &mut self.sound);
            // The hints follow whoever is signed in (nobody, signed out).
            let state = session.state();
            self.hints
                .follow(session.server(), state.me(), state.inventory());
            if session.destination().is_some() {
                self.hints.saw(Did::Walked);
            }
            if gathered {
                self.hints.saw(Did::Gathered);
            }
            if made {
                self.hints.saw(Did::Crafted);
            }
            plots(
                ctx,
                session,
                &self.shared,
                &self.lots,
                &self.lot_of,
                &mut self.sound,
            );
            // Signed out, the store is empty, so everyone fades away.
            let (poses, state) = session.sample(now);
            let dt = ctx.clock.delta_time.min(0.1) as f32;
            self.players.update(ctx, poses, state, dt, &self.shared);
            trade(ctx, session, &self.shared, now);
            if let Some((item, strands)) = &self.trade_offer {
                offer_preset(session, item, *strands, &mut self.trade_offered, now);
            }
            let banner = session.arrival().map(|a| ZoneBanner {
                name: a.zone.display_name().to_owned(),
                colour: palette::srgb(palette::zone_mark(a.zone)),
                at: a.at,
                first: a.first,
            });
            if let Ok(mut shared) = self.shared.banner.lock() {
                *shared = banner;
            }
            let target = session.harvest_target();
            for c in self
                .nodes
                .update(ctx, session.state(), target, dt, &self.shared)
            {
                let [x, z] = c.at;
                let mut data = toml::Table::new();
                data.insert("x".into(), (x as f64).into());
                data.insert("z".into(), (z as f64).into());
                ctx.fire_event(c.name, data);
            }
            let placed = session.take_placed();
            let me = self.shared.follow.lock().ok().and_then(|f| *f);
            self.stations
                .update(ctx, session.state(), me, &placed, dt, &self.shared);
            if let Some(kind) = &self.place_on_arrival {
                let kit = session
                    .state()
                    .inventory()
                    .and_then(|c| c.items().find(|i| &i.item_type == kind).cloned());
                if let Some(kit) = kit {
                    session.begin_placing(&kit);
                    self.place_on_arrival = None;
                }
            }
            if std::mem::take(&mut self.cancel_placing) {
                session.cancel_placing();
            }
            placing(ctx, session, &self.shared, &mut self.stations, now, dt);
        }
        self.shared
            .at_login
            .store(self.at_login(), Ordering::Relaxed);
        listen(ctx, &self.shared, &mut self.sound);
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
        // Your plots in gold, rebuilt when they change. Offline, none.
        let owned = self.session.as_ref().map_or_else(Vec::new, |s| {
            owned_lots(s.state().plots(), &self.lots, &self.lot_of)
        });
        if self.owned_drawn.as_ref() != Some(&owned) {
            lots::upload_owned(gpu, owned.iter().filter_map(|&i| self.lots[i].as_ref()));
            let any = !owned.is_empty();
            self.owned_drawn = Some(owned);
            if any {
                attach_each(ctx, std::iter::once(&lots::OWNED_MESH.to_owned()));
            }
        }
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
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
            self.sound.set_rings(rings);
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
            self.lot_of = terrain
                .terrain_plots
                .iter()
                .enumerate()
                .map(|(i, p)| (p.id.clone(), i))
                .collect();
            self.lots = lots;
            // The lots moved, so the gold is built again next frame.
            self.owned_drawn = None;
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
    ///
    /// While a station is being placed (P6.2), Esc stops it rather than
    /// quitting, and so does a right click on the world (not a right-drag,
    /// which orbits). egui has already kept any click over its panels.
    ///
    /// `?` opens and closes the shortcut overlay (P8.3), read off the typed
    /// character so it's `?` on any layout. While it's open, Esc closes it.
    fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        let placing = self.session.as_ref().is_some_and(|s| s.placing().is_some());
        if let WindowEvent::KeyboardInput { event: key, .. } = event {
            let pressed = key.state == ElementState::Pressed && !key.repeat;
            if self.in_world() && key.logical_key == Key::Character("?".into()) {
                if pressed {
                    self.shortcuts.toggle();
                }
                return true;
            }
            if self.shortcuts.open && key.physical_key == PhysicalKey::Code(KeyCode::Escape) {
                if pressed {
                    self.shortcuts.open = false;
                }
                return true;
            }
        }
        match event {
            WindowEvent::KeyboardInput { event: key, .. }
                if placing && key.physical_key == PhysicalKey::Code(KeyCode::Escape) =>
            {
                if key.state == ElementState::Pressed {
                    self.cancel_placing = true;
                }
                return true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => match state {
                ElementState::Pressed => self.right_down = Some(self.cursor),
                ElementState::Released => {
                    let (x, y) = self.cursor;
                    let click = self
                        .right_down
                        .take()
                        .is_some_and(|(px, py)| (x - px).hypot(y - py) <= RIGHT_CLICK_SLOP);
                    self.cancel_placing |= placing && click;
                }
            },
            _ => {}
        }
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
        if let Ok(mut egui) = self.shared.egui.lock() {
            // For measuring names (`cap_name_tags`), now the fonts are in.
            egui.get_or_insert_with(|| ctx.clone());
        }
        let at_login = self.at_login();
        let mut action = None;
        // Where pickups fly to (P8.2): the header's Inventory button, or the
        // inventory panel while it's open. Neither while signed out.
        let mut inventory_chip = None;
        let pulse = self.shared.pulse("inventory", Instant::now());
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
                    inventory_open: self.inventory.open,
                    crafting_open: self.crafting.open,
                    exchange_open: self.exchange.open,
                    help_open: self.shortcuts.open,
                    settings_open: self.settings_panel.open,
                    inventory_pulse: pulse,
                };
                let clicks = ui::hud::header(ctx, &view, &mut self.strands, Instant::now());
                sign_out |= clicks.sign_out;
                inventory_chip = clicks.inventory_rect;
                if clicks.inventory {
                    self.inventory.toggle();
                    self.hints.saw(Did::UsedInventory);
                }
                if clicks.crafting {
                    self.crafting.toggle();
                    if self.crafting.open {
                        self.hints.saw(Did::OpenedCrafting);
                    }
                }
                if clicks.exchange {
                    self.exchange.toggle();
                }
                if clicks.help {
                    self.shortcuts.toggle();
                }
                if clicks.settings {
                    self.settings_panel.toggle();
                }
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
            if !at_login {
                let now = Instant::now();
                // Enter crafts at the open station, else on the crafting panel.
                let enter = std::mem::take(&mut self.craft_key);
                let at_station = self.station.recipe(session.state()).is_some();
                let key =
                    (enter && !at_station && self.crafting.open).then_some(CraftAction::Craft);
                let station_key = (enter && at_station).then_some(StationAction::Craft);
                let view = ui::crafting::View {
                    state: session.state(),
                    phase: session.crafting().phase(),
                    // The inventory panel shows a refused move.
                    refused: session.craft_refused(now),
                };
                let dt = ctx.input(|i| i.stable_dt).min(0.1);
                match self.crafting.show(ctx, &mut self.icons, &view, dt).or(key) {
                    Some(CraftAction::Move(m)) => session.move_item(m),
                    Some(CraftAction::Moves(shifts)) => session.move_items(&shifts),
                    Some(CraftAction::Craft) => {
                        let recipe = self.crafting.recipe(session.state()).map(|r| r.id.clone());
                        if let Some(id) = recipe {
                            session.craft(now, &id);
                        }
                    }
                    Some(CraftAction::Cancel) => session.cancel_craft(),
                    None => {}
                }
                let state = session.state();
                let view = ui::exchange::View {
                    inventory: state.inventory(),
                    strands: state.strands(),
                    busy: session.exchanging(),
                    refused: session.exchange_refused(now),
                    receipt: session.receipt(now),
                };
                match self
                    .exchange
                    .show(ctx, &mut self.icons, &view, self.crafting.rect)
                {
                    Some(ui::exchange::Action::Deposit { item_ids, quote }) => {
                        session.deposit(item_ids, quote)
                    }
                    Some(ui::exchange::Action::Withdraw(quote)) => session.withdraw(quote),
                    Some(ui::exchange::Action::Close) => self.exchange.open = false,
                    None => {}
                }
                // The settings panel takes the hint card's corner; the card
                // waits behind it rather than showing through.
                if self.settings_panel.open {
                    self.hints.rect = None;
                } else {
                    self.hints.show(ctx, now);
                }
                let state = session.state();
                let picked = state.picked_plot();
                let view = ui::plot::View {
                    plot: picked,
                    offer: picked.map_or(Offer::Unpriced, |p| {
                        Offer::new(p, state.me(), state.deed_prices(), state.strands())
                    }),
                    strands: state.strands(),
                    claiming: session.claiming(),
                    refused: session.claim_refused(now),
                };
                match self.plot_panel.show(ctx, &view, self.crafting.rect) {
                    Some(ui::plot::Action::Claim(id)) => session.claim_plot(id),
                    Some(ui::plot::Action::Close) => session.close_plot(),
                    None => {}
                }
                let view = ui::station::View {
                    state: session.state(),
                    phase: session.crafting().phase(),
                    refused: session
                        .craft_refused(now)
                        .or_else(|| session.move_refused(now)),
                    now,
                    beside: self.inventory.rect,
                };
                match self
                    .station
                    .show(ctx, &mut self.icons, &view, dt)
                    .or(station_key)
                {
                    Some(StationAction::Move(m)) => session.move_item(m),
                    Some(StationAction::Moves(shifts)) => session.move_items(&shifts),
                    Some(StationAction::Craft) => {
                        let recipe = self.station.recipe(session.state()).map(|r| r.id.clone());
                        if let Some(id) = recipe {
                            session.craft_at_station(now, &id);
                        }
                    }
                    Some(StationAction::Cancel) => session.cancel_craft(),
                    Some(StationAction::Close) => session.close_station(),
                    Some(StationAction::Remove(id)) => session.remove_station(id),
                    None => {}
                }
                let state = session.state();
                let walking_to = session
                    .trade_target()
                    .and_then(|id| state.player(id))
                    .map(|p| p.name.clone());
                let view = ui::trade::View {
                    trade: state.trade(),
                    draft: session.draft(),
                    inventory: state.inventory(),
                    strands: state.strands(),
                    waiting: state.trade_waiting(),
                    offer_changed: session.offer_changed(now),
                    walking_to: walking_to.as_deref(),
                };
                let was_open = self.trade.is_open();
                let asked = self.trade.show(ctx, &mut self.icons, &view);
                // You'll want to drag from it.
                if self.trade.is_open() && !was_open {
                    self.inventory.open = true;
                }
                if let Some(action) = asked {
                    act_on_trade(session, action, now);
                }
                let refused = session.move_refused(Instant::now());
                let inventory = session.state().inventory();
                let clicks = self
                    .inventory
                    .show(ctx, &mut self.icons, inventory, refused, pulse);
                if let Some(m) = clicks.moved {
                    session.move_item(m);
                    self.hints.saw(Did::UsedInventory);
                }
                if let Some(kit) = clicks.place {
                    session.begin_placing(&kit);
                }
            }
            if !at_login && self.containers {
                // The inventory panel shows a refused move's reason.
                let refused = None;
                if let Some(m) =
                    ui::containers::bench(ctx, &mut self.icons, session.state(), refused)
                {
                    session.move_item(m);
                }
            }
        }
        let anchor = match (self.inventory.rect, inventory_chip) {
            _ if at_login || self.session.is_none() => None,
            (Some(r), _) => Some((r, true)),
            (None, Some(r)) => Some((r, false)),
            (None, None) => None,
        };
        if let Ok(mut anchors) = self.shared.anchors.lock() {
            match anchor {
                Some((r, panel)) => {
                    let rect = [r.left(), r.top(), r.width(), r.height()];
                    anchors.insert("inventory", UiAnchor { rect, panel });
                }
                None => {
                    anchors.remove("inventory");
                }
            }
        }
        let view = ui::settings::View {
            server: self.session.as_ref().map(|s| s.server()),
        };
        match self
            .settings_panel
            .show(ctx, &mut self.settings.settings, &view)
        {
            Some(ui::settings::Action::SetServer(server)) => self.switch_server(&server),
            Some(ui::settings::Action::Close) => self.settings_panel.toggle(),
            None => {}
        }
        if self.session.is_none() {
            if !self.shared.clean.load(Ordering::Relaxed) {
                ui::inscription(ctx, &self.shared, &self.terrain.status());
            }
        } else if !at_login {
            let clear_of = self.plot_panel.rect.or(self.crafting.rect);
            ui::controls(ctx, &self.terrain.status(), clear_of);
        }
        if self.icon_sheet {
            ui::icons::sheet(ctx, &mut self.icons);
        }
        if let Some(sample) = &mut self.sample {
            sample.show(ctx, &mut self.icons);
        }
        if let Some(juice) = &mut self.juice {
            juice.ui(ctx, &mut self.icons, &self.shared);
        }
        if !self.in_world() {
            self.shortcuts.open = false;
        }
        if self.shortcuts.show(ctx) {
            self.shortcuts.open = false;
        }
        if let Some(action) = action {
            self.act(action);
        }
        // Every widget clicked, and everything let go after a drag (a slot
        // filled), makes its sound next frame.
        let (clicked, dropped) =
            ctx.interaction_snapshot(|i| (i.clicked.is_some(), i.drag_stopped.is_some()));
        if clicked {
            self.sound.click();
        }
        if dropped {
            self.sound.drop_in_slot();
        }
    }
}

/// What you hear (P8.1): ease the beds toward where you are, and fire
/// `sound` at `audio.rhai` for each one-shot due. The ears are on you, or
/// offline on the point the camera looks at.
fn listen(ctx: &mut ExtensionCtx, shared: &script_api::Shared, sound: &mut Soundscape) {
    let cam = ctx.camera;
    let focus = shared
        .follow
        .lock()
        .ok()
        .and_then(|f| *f)
        .map_or([cam.target.x, cam.target.z], |f| [f[0], f[2]]);
    let ears = Ears {
        focus,
        camera: [cam.position.x, cam.position.y, cam.position.z],
        height: cam.position.y - crate::coords::layer::GROUND,
    };
    let working: Vec<Rustling> = shared
        .nodes
        .lock()
        .map(|nodes| {
            nodes
                .iter()
                .filter(|n| n.state == "harvesting")
                .map(|n| Rustling {
                    id: n.id.clone(),
                    at: n.at,
                    mine: n.mine,
                })
                .collect()
        })
        .unwrap_or_default();
    let dt = ctx.clock.delta_time.min(0.1) as f32;
    for cue in sound.update(ears, &working, dt) {
        tracing::debug!("sound: {} at {:.2}", cue.sound, cue.volume);
        ctx.fire_event("sound", cue.data());
    }
    sound.apply(ctx);
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

/// Trading (P7.3): ask the player a script asked for (`cap_trade_with`),
/// tell scripts where trading stands (`cap_trade`), and fire at them
/// `trade_note`, with a `title` and `body`, for what's worth a toast, and
/// `trade_executed` with where you and your partner stood (`x`, `z`,
/// `tx`, `tz`, world metres; the partner's are yours if unknown) when a
/// trade goes through.
fn trade(ctx: &mut ExtensionCtx, session: &mut Session, shared: &Shared, now: Instant) {
    use capitol_core::state::Trade;
    if let Some(id) = shared.trade_with.lock().ok().and_then(|mut t| t.take()) {
        match uuid::Uuid::parse_str(&id) {
            Ok(player) => {
                session.trade_with(now, player);
            }
            Err(e) => tracing::warn!("cap_trade_with: {id:?} isn't a player id: {e}"),
        }
    }
    for note in session.take_trade_notes() {
        let mut data = toml::Table::new();
        data.insert("title".into(), note.title.into());
        data.insert("body".into(), note.body.into());
        ctx.fire_event("trade_note", data);
    }
    for done in session.take_traded() {
        let Some((x, y)) = done.me else {
            continue;
        };
        let [x, z] = crate::coords::to_world(x, y);
        let [tx, tz] = done
            .them
            .map_or([x, z], |(x, y)| crate::coords::to_world(x, y));
        let mut data = toml::Table::new();
        data.insert("x".into(), (x as f64).into());
        data.insert("z".into(), (z as f64).into());
        data.insert("tx".into(), (tx as f64).into());
        data.insert("tz".into(), (tz as f64).into());
        data.insert("partner".into(), done.partner.into());
        ctx.fire_event("trade_executed", data);
    }
    let state = session.state();
    let (other, mark) = match state.trade() {
        Trade::Idle => match session.trade_target() {
            Some(id) => (Some(id), "walking"),
            None => (None, ""),
        },
        Trade::Requested { to, .. } => (Some(*to), "asking"),
        Trade::Incoming { from, .. } => (Some(*from), "incoming"),
        Trade::Open(open) => (Some(open.partner_id), "open"),
    };
    let open = state.open_trade();
    let mark = other.map(|id| {
        let them = shared.players.lock().ok().and_then(|labels| {
            labels
                .iter()
                .find(|l| l.id == id)
                .map(|l| [l.at[0], l.feet_y, l.at[2]])
        });
        TradeMark {
            state: mark,
            them,
            mine: open.is_some_and(|o| o.i_accepted),
            theirs: open.is_some_and(|o| o.they_accepted),
        }
    });
    if let Ok(mut shared) = shared.trade.lock() {
        *shared = mark;
    }
}

/// What the trade panel asked for.
fn act_on_trade(session: &mut Session, action: ui::trade::Action, now: Instant) {
    use ui::trade::Action;
    match action {
        Action::Answer(accept) => session.answer_trade(now, accept),
        Action::Withdraw => session.withdraw_trade(),
        Action::Add(item) => session.draft_mut().add(&item, now),
        Action::Quantity {
            item_id,
            quantity,
            stack,
        } => session
            .draft_mut()
            .set_quantity(item_id, quantity, stack, now),
        Action::Remove(id) => session.draft_mut().remove(id, now),
        Action::Strands(n) => session.draft_mut().set_strands(n, now),
        Action::Accept => session.accept_trade(now),
        Action::Cancel => session.cancel_trade(now),
    }
}

/// `--trade-offer`: puts every `item` stack and `strands` Strands into the
/// draft of each trade as it opens, once.
fn offer_preset(
    session: &mut Session,
    item: &str,
    strands: i64,
    offered: &mut Option<uuid::Uuid>,
    now: Instant,
) {
    let Some(id) = session.state().open_trade().map(|o| o.trade_id) else {
        return;
    };
    if *offered == Some(id) {
        return;
    }
    let stacks: Vec<_> = session
        .state()
        .inventory()
        .map(|c| c.items().filter(|i| i.item_type == item).cloned().collect())
        .unwrap_or_default();
    if session.draft().strands() != 0 || !session.draft().lines().is_empty() {
        return;
    }
    *offered = Some(id);
    // `Session::update` has already synced the draft to this trade.
    let draft = session.draft_mut();
    for stack in &stacks {
        draft.add(stack, now);
    }
    draft.set_strands(strands, now);
    tracing::info!(
        "trade: --trade-offer put {} stacks and {strands} Strands in",
        stacks.len()
    );
}

/// Resource nodes (P4.4): send the extraction a script asked for
/// (`cap_extract`), and fire at the scripts what came of it:
/// `extraction_completed` with the node's place (`x`, `z`), the yield
/// (`item_name`, `quantity`, `quality`) and its `grade` letter and `colour`
/// (sRGB, 0 to 1), and `extraction_note` with a `title` and `body` when an
/// extraction didn't happen. True if anything was gathered.
fn gather(
    ctx: &mut ExtensionCtx,
    session: &mut Session,
    shared: &script_api::Shared,
    sound: &mut Soundscape,
    now: Instant,
) -> bool {
    if let Some(id) = shared.extract.lock().ok().and_then(|mut e| e.take()) {
        match uuid::Uuid::parse_str(&id) {
            Ok(node) => session.extract(now, node),
            Err(e) => tracing::warn!("cap_extract: {id:?} isn't a node id: {e}"),
        }
    }
    let yields = session.take_yields();
    let any = !yields.is_empty();
    for (node, item) in yields {
        let at = session
            .state()
            .node(node)
            .map(|n| crate::coords::to_world(n.x, n.y))
            .or_else(|| {
                shared
                    .follow
                    .lock()
                    .ok()
                    .and_then(|f| *f)
                    .map(|f| [f[0], f[2]])
            });
        let [x, z] = at.unwrap_or_default();
        let mut data = yield_data(&item);
        data.insert("x".into(), (x as f64).into());
        data.insert("z".into(), (z as f64).into());
        ctx.fire_event("extraction_completed", data);
        sound.gathered();
    }
    for note in session.take_harvest_notes() {
        let (title, body) = note_words(&note);
        tracing::info!("extract: {note:?}");
        let mut data = toml::Table::new();
        data.insert("title".into(), title.into());
        data.insert("body".into(), body.into());
        ctx.fire_event("extraction_note", data);
    }
    any
}

/// Stations (P6.3): open the station a script asked for
/// (`cap_open_station`), and fire `station_note` at the scripts, with a
/// `title` and `body`, when one wouldn't open.
fn visit(ctx: &mut ExtensionCtx, session: &mut Session, shared: &script_api::Shared, now: Instant) {
    if let Some(id) = shared.open_station.lock().ok().and_then(|mut o| o.take()) {
        match uuid::Uuid::parse_str(&id) {
            Ok(station) => session.open_station(now, station),
            Err(e) => tracing::warn!("cap_open_station: {id:?} isn't a station id: {e}"),
        }
    }
    for note in session.take_station_notes() {
        tracing::info!("station: {note:?}");
        let (title, body) = match note {
            capitol_core::station::visit::Note::Refused(why) => ("Can't open it", why),
            capitol_core::station::visit::Note::Unanswered => (
                "Can't open it",
                "The server didn't answer. Try again".into(),
            ),
            capitol_core::station::visit::Note::Lost => {
                ("Gone", "That station was taken down".into())
            }
        };
        let mut data = toml::Table::new();
        data.insert("title".into(), title.into());
        data.insert("body".into(), body.into());
        ctx.fire_event("station_note", data);
    }
}

/// Crafting (P5.5): fire `crafting_completed` at the scripts for each craft
/// that finished, with where you stand (`x`, `z`), what it made
/// (`item_name`, `quantity`, `quality`) and its `grade` and `colour`, as
/// `extraction_completed` has. True if anything was made.
fn crafted(
    ctx: &mut ExtensionCtx,
    session: &mut Session,
    shared: &script_api::Shared,
    sound: &mut Soundscape,
) -> bool {
    let made = session.take_crafted();
    let any = !made.is_empty();
    for item in made {
        let [x, _, z] = shared
            .follow
            .lock()
            .ok()
            .and_then(|f| *f)
            .unwrap_or_default();
        tracing::info!(
            "craft: made {} x{} at {}",
            item.item_name,
            item.quantity,
            item.quality
        );
        let mut data = yield_data(&item);
        data.insert("x".into(), (x as f64).into());
        data.insert("z".into(), (z as f64).into());
        ctx.fire_event("crafting_completed", data);
        sound.crafted();
    }
    any
}

/// Plots (P6.4): pick the plot a script asked for (`cap_pick_plot`), tell
/// scripts which is picked (`cap_picked_plot`), and fire at them
/// `plot_claimed` with the claimed plot's middle (`x`, `z`, world metres)
/// and about how far it reaches from there (`r`, metres), and `plot_note`
/// with a `title` and `body` when a pick found nothing.
fn plots(
    ctx: &mut ExtensionCtx,
    session: &mut Session,
    shared: &script_api::Shared,
    lots: &[Option<Lot>],
    lot_of: &HashMap<String, usize>,
    sound: &mut Soundscape,
) {
    if let Some([x, y]) = shared.pick_plot.lock().ok().and_then(|mut p| p.take()) {
        session.pick_plot(x, y);
    }
    let me = session.state().me();
    let mark = session.state().picked_plot().map(|p| {
        let lot = p
            .terrain_id
            .as_ref()
            .and_then(|id| lot_of.get(id))
            .and_then(|&i| lots.get(i)?.as_ref());
        PlotMark {
            outline: plot_outline(p, lot),
            y: crate::coords::layer::PLOT + 0.06,
            mine: me.is_some() && p.owner_id == me,
            owned: p.owner_id.is_some(),
        }
    });
    if let Ok(mut picked) = shared.picked_plot.lock() {
        *picked = mark;
    }
    for plot_id in session.take_claimed() {
        let Some(plot) = session.state().plot(plot_id) else {
            continue;
        };
        let [x, z] = crate::coords::to_world(plot.world_x, plot.world_y);
        let (w, h) = (
            plot.bounds.max_x - plot.bounds.min_x,
            plot.bounds.max_y - plot.bounds.min_y,
        );
        let mut data = toml::Table::new();
        data.insert("x".into(), (x as f64).into());
        data.insert("z".into(), (z as f64).into());
        data.insert(
            "r".into(),
            (w.hypot(h) * 0.5 * crate::coords::S as f64).into(),
        );
        ctx.fire_event("plot_claimed", data);
        sound.claimed();
    }
    for note in session.take_plot_notes() {
        let (title, body) = match note {
            crate::session::PlotNote::NoneThere => {
                ("No plot here", "Alt-click a lot to see its deed")
            }
            crate::session::PlotNote::Unanswered => {
                ("Couldn't look", "The server didn't say what's here")
            }
        };
        let mut data = toml::Table::new();
        data.insert("title".into(), title.into());
        data.insert("body".into(), body.into());
        ctx.fire_event("plot_note", data);
    }
}

/// A plot's edge in world metres: the lot as drawn if the terrain has it,
/// else the claimed quad, else its box.
fn plot_outline(plot: &capitol_core::protocol::rest::PlotInfo, lot: Option<&Lot>) -> Vec<[f32; 2]> {
    let world = |&(x, y): &(f64, f64)| crate::coords::to_world(x, y);
    if let Some(lot) = lot {
        return lot.outline.iter().map(|p| world(&(p[0], p[1]))).collect();
    }
    if let Some(corners) = &plot.corners {
        return corners.iter().map(world).collect();
    }
    let b = &plot.bounds;
    [
        (b.min_x, b.min_y),
        (b.max_x, b.min_y),
        (b.max_x, b.max_y),
        (b.min_x, b.max_y),
    ]
    .iter()
    .map(world)
    .collect()
}

/// Where your plots are in `lots`, sorted: each joined to its lot by
/// `terrain_id`. A plot with no lot (pre-B7, or a terrain that no longer
/// has it) isn't drawn.
fn owned_lots(
    plots: &[capitol_core::protocol::rest::PlotInfo],
    lots: &[Option<Lot>],
    lot_of: &HashMap<String, usize>,
) -> Vec<usize> {
    let mut owned: Vec<usize> = plots
        .iter()
        .filter_map(|p| lot_of.get(p.terrain_id.as_ref()?).copied())
        .filter(|&i| lots.get(i).is_some_and(Option::is_some))
        .collect();
    owned.sort_unstable();
    owned.dedup();
    owned
}

/// Placing a station (P6.2): put it down where a script asked
/// (`cap_place`), show the ghost at the cursor (`cap_place_hover`), tell
/// scripts what they need to draw round it (`cap_placing`), and fire
/// `placement_note` at them, with a `title` and `body`, when a click is
/// refused or the server says no.
fn placing(
    ctx: &mut ExtensionCtx,
    session: &mut Session,
    shared: &Shared,
    stations: &mut Stations,
    now: Instant,
    dt: f32,
) {
    use crate::coords::{to_server, to_world, S};
    let click = shared.place_at.lock().ok().and_then(|mut p| p.take());
    if let Some([x, z]) = click {
        let [x, y] = to_server(x, z);
        session.place(now, x, y);
    }
    for note in session.take_placing_notes() {
        let (title, body) = match note {
            PlaceNote::Refused(why) => ("Can't build here".to_owned(), why.words()),
            PlaceNote::Failed(why) => ("Can't build here".to_owned(), why),
            PlaceNote::Unanswered => (
                "No answer".to_owned(),
                "The server didn't answer. Try again".to_owned(),
            ),
            PlaceNote::Placed { .. } => continue,
        };
        let mut data = toml::Table::new();
        data.insert("title".into(), title.into());
        data.insert("body".into(), body.into());
        ctx.fire_event("placement_note", data);
    }

    let hover = shared.place_hover.lock().ok().and_then(|h| *h);
    let state = session.state();
    let mark = session.placing().map(|p| {
        let asking = match p.phase() {
            PlacePhase::Asking { at, .. } => Some(*at),
            PlacePhase::Aiming => None,
        };
        // Asking, the ghost holds where the click was; otherwise it's the cursor.
        let target = asking.or_else(|| hover.map(|[x, z]| to_server(x, z)));
        let check = target.map(|at| p.check(at, state));
        let ok = matches!(check, Some(Ok(_)));
        let station_type = p.kit().station_type;
        let crowding = target.map_or_else(Vec::new, |[x, y]| {
            state
                .stations()
                .values()
                .filter(|s| (s.x - x).hypot(s.y - y) < STATION_MIN_SPACING * 2.0)
                .map(|s| to_world(s.x, s.y))
                .collect()
        });
        let me = state.me();
        let plots = state
            .plots()
            .iter()
            .filter(|plot| me.is_some() && plot.owner_id == me)
            .filter_map(|plot| plot.corners)
            .map(|c| c.map(|(x, y)| to_world(x, y)))
            .collect();
        let view = target.map(|[x, y]| GhostView {
            station_type,
            at: to_world(x, y),
            ok,
            asking: asking.is_some(),
        });
        let mark = PlacementMark {
            kind: crate::stations::kind_of(station_type),
            kit: p.kit().item_name.clone(),
            at: None,
            radius: crate::stations::ring_radius(station_type),
            ok,
            why: match &check {
                Some(Err(why)) => Some(why.words()),
                _ => None,
            },
            asking: asking.is_some(),
            you: state.local_player().map(|you| to_world(you.x, you.y)),
            reach: STATION_INTERACTION_RANGE as f32 * S,
            spacing: STATION_MIN_SPACING as f32 * S,
            crowding,
            plots,
        };
        (mark, view)
    });
    let view = mark.as_ref().and_then(|(_, v)| *v);
    stations.ghost(ctx, view, dt);
    let mark = mark.map(|(mut m, _)| {
        m.at = view.and(stations.ghost_at());
        m
    });
    if let Ok(mut shared) = shared.placement.lock() {
        *shared = mark;
    }
}

/// What a gather or a craft made, for the scripts: `item_name`,
/// `quantity`, `quality`, its `grade` letter and the grade's `colour`
/// (sRGB, 0 to 1).
pub(crate) fn yield_data(item: &capitol_core::state::Yield) -> toml::Table {
    let letter = capitol_core::protocol::rules::quality_grade(item.quality);
    let c = ui::theme::grade(letter);
    let mut data = toml::Table::new();
    data.insert("item_name".into(), item.item_name.clone().into());
    data.insert("quantity".into(), (item.quantity as i64).into());
    data.insert("quality".into(), (item.quality as i64).into());
    data.insert("grade".into(), letter.into());
    data.insert(
        "colour".into(),
        toml::Value::Array(
            [c.r(), c.g(), c.b()]
                .iter()
                .map(|v| toml::Value::Float(*v as f64 / 255.0))
                .collect(),
        ),
    );
    data
}

/// What to tell the player about an extraction that didn't happen. A
/// refusal carries the server's own reason (B9, #60).
fn note_words(note: &Note) -> (&'static str, String) {
    match note {
        Note::Lost(_) => ("Taken", "Someone else got to that grass first".into()),
        Note::Refused(reason) => ("Couldn't gather", reason.clone()),
        Note::Unanswered(_) => ("Couldn't gather", "The server didn't answer".into()),
        Note::Busy => ("Busy", "Finish what you're doing first".into()),
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
