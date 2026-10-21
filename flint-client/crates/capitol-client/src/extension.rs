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
//! settings and their panel (`settings`, `ui::settings`), and the spike's
//! markers.

use crate::atmosphere::Atmosphere;
use crate::daylight;
use crate::entities::Players;
use crate::juice_sample::JuiceSample;
use crate::nodes::{self, Nodes};
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
use capitol_core::protocol::rules::{self, water_name, STATION_INTERACTION_RANGE, STATION_MIN_SPACING};
use capitol_core::protocol::terrain::{LandmarkKind, TerrainResponse};
use capitol_core::state::Activity;
use capitol_core::station::{Note as PlaceNote, Phase as PlacePhase};
use flint_player::{ExtensionCtx, KeyInput, PlayerExtension};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::keyboard::KeyCode;

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
    /// Where each of the terrain's plots is in the plot meshes, by its
    /// `TerrainPlot::id`, which a claimable plot names as
    /// `PlotInfo::terrain_id` (B7).
    lots: HashMap<String, Lot>,
    /// The lots `world::plots::OWNED_MESH` was last built over, or `None`
    /// to build it again.
    owned_drawn: Option<Vec<String>>,
    /// The houses on everyone's claimed lots (P8.6b).
    houses: world::houses::Houses,
    /// `--claim-sample`: offline, claims to build houses on.
    claim_sample: Option<world::houses::Sample>,
    /// Offline, one of every station, placed once the camera settles
    /// (`--station-sample`).
    station_sample: Option<Option<capitol_core::state::GameState>>,
    /// The station sample is `--automation-sample`'s (E4.4), and how long
    /// it has run.
    automation_sample: Option<f32>,
    /// `--node-sample`: offline, every kind of node, laid out round where
    /// the camera first looks once it's there.
    node_sample: bool,
    node_sampled: Option<nodes::sample::Sample>,
    /// `--citizen-sample` (C0.2): offline, people and citizens in a row
    /// where the camera looks, stood there once it's settled.
    citizen_sample: Option<Option<(capitol_core::state::GameState, Vec<capitol_core::motion::Pose>)>>,
    /// The grass fields over the rural ring and the wilderness (P8.6c).
    grass: world::grass::Grass,
    /// The wilds' height field, sampled off the main thread (W9), its
    /// meshes, and whether it's in (the flat disc is open over the wilds
    /// once it is).
    land: world::land::Builder,
    land_meshes: world::land_mesh::LandMeshes,
    /// The mist's opacity as last written (`mist_in_place`).
    mist_shown: Option<f32>,
    land_in: bool,
    plot_panel: ui::plot::PlotPanel,
    atmosphere: Atmosphere,
    /// `--time-of-day`: the hour pinned, 0..1 from midnight, in place of
    /// the server's clock (P8.6a).
    time_of_day: Option<f64>,
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
    market: ui::market::MarketPanel,
    /// `--market-sample`: a market to show offline (E3.5).
    market_sample: Option<ui::market::Sample>,
    /// The Capitol Gazette (E3.9), and `--gazette-sample`: an edition to
    /// read offline.
    gazette: ui::gazette::GazettePanel,
    gazette_sample: Option<capitol_core::gazette::Reader>,
    /// What a foreclosure kept for you (#141), and `--impound-sample`: an
    /// impound and a pack to show offline.
    impound: ui::impound::ImpoundPanel,
    impound_sample: Option<(capitol_core::state::Container, capitol_core::state::Container)>,
    /// `--station-panel-sample`: a workbench of yours open, offline (#235).
    station_panel_sample: Option<capitol_core::state::GameState>,
    /// Its tab, set once the panel has opened (opening picks its own).
    station_panel_tab: Option<ui::station::Tab>,
    /// The Exchange hall (E3.7), and `--hall-sample`: offline, goods waiting
    /// and you where the camera looks.
    hall: world::hall::HallLayer,
    hall_sample: bool,
    /// Where the hall is on screen, as the header's hint points (radians,
    /// 0 to the right, clockwise), while goods wait and you're not there.
    hall_hint: Option<f32>,
    /// The bazaar (E3.11): its building, the open stall's shop panel, and
    /// `--bazaar-sample`'s shops offline.
    bazaar: world::bazaar::BazaarLayer,
    shop: ui::shop::ShopPanel,
    bazaar_sample: Option<ui::shop::Sample>,
    /// A layer built again, so the camera's solids want gathering (E3.13).
    solids_stale: bool,
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
            lots: HashMap::new(),
            owned_drawn: None,
            houses: world::houses::Houses::default(),
            claim_sample: None,
            station_sample: None,
            automation_sample: None,
            node_sample: false,
            node_sampled: None,
            citizen_sample: None,
            grass: world::grass::Grass::default(),
            land: world::land::Builder::default(),
            land_meshes: world::land_mesh::LandMeshes::default(),
            mist_shown: None,
            land_in: false,
            plot_panel: ui::plot::PlotPanel::default(),
            atmosphere: Atmosphere::default(),
            time_of_day: None,
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
            market: ui::market::MarketPanel::default(),
            market_sample: None,
            gazette: ui::gazette::GazettePanel::default(),
            gazette_sample: None,
            impound: ui::impound::ImpoundPanel::default(),
            impound_sample: None,
            station_panel_sample: None,
            station_panel_tab: None,
            hall: world::hall::HallLayer::default(),
            hall_sample: false,
            hall_hint: None,
            bazaar: world::bazaar::BazaarLayer::default(),
            shop: ui::shop::ShopPanel::default(),
            bazaar_sample: None,
            solids_stale: false,
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

    /// Pin the time of day (`--time-of-day`), 0..1 from midnight.
    pub fn with_time_of_day(mut self, phase: Option<f64>) -> Self {
        self.time_of_day = phase;
        self
    }

    /// The time of day to light the city for: `--time-of-day` if given,
    /// else the server's clock (B12), else, offline or before the first
    /// `AuthOk`, the evening, which is the scene as authored.
    fn phase(&self) -> f64 {
        self.time_of_day
            .or_else(|| self.session.as_ref()?.state().time_of_day())
            .unwrap_or(daylight::EVENING)
    }

    /// Offline, claim `share` of the lots, and with `grow_at`, the lots
    /// nearest the camera at that second (`--claim-sample`, `--grow-at`).
    pub fn with_claim_sample(mut self, share: Option<f32>, grow_at: Option<f32>) -> Self {
        if self.session.is_none() && (share.is_some() || grow_at.is_some()) {
            self.claim_sample = Some(world::houses::Sample::new(share.unwrap_or(0.0), grow_at));
        }
        self
    }

    /// Offline, one of every station in a row before the camera
    /// (`--station-sample`).
    pub fn with_station_sample(mut self, on: bool) -> Self {
        if self.session.is_none() && on {
            self.station_sample = Some(None);
        }
        self
    }

    /// Offline, stations at work and waiting in a row before the camera
    /// (`--automation-sample`).
    pub fn with_automation_sample(mut self, on: bool) -> Self {
        if self.session.is_none() && on {
            self.station_sample = Some(None);
            self.automation_sample = Some(0.0);
        }
        self
    }

    /// Offline, scatter every kind of node round the camera and take some
    /// through a day's work (`--node-sample`).
    /// Offline, people and citizens side by side before the camera
    /// (`--citizen-sample`, C0.2).
    pub fn with_citizen_sample(mut self, on: bool) -> Self {
        if self.session.is_none() && on {
            self.citizen_sample = Some(None);
        }
        self
    }

    pub fn with_node_sample(mut self, on: bool) -> Self {
        self.node_sample = on && self.session.is_none();
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

    /// Offline, open the Gazette on the protocol's sample edition
    /// (`--gazette-sample`).
    pub fn with_gazette_sample(mut self, on: bool) -> Self {
        if on && self.session.is_none() {
            self.gazette_sample = Some(ui::gazette::sample());
            self.gazette.open = true;
        }
        self
    }

    /// Offline, open the impound on a sample (`--impound-sample`, #141).
    pub fn with_impound_sample(mut self, on: bool) -> Self {
        if on && self.session.is_none() {
            self.impound_sample = Some(ui::impound::sample());
            self.impound.open = true;
        }
        self
    }

    /// Offline, open the station panel on a sample workbench, on `tab`
    /// (`craft` or `automation`), with the inventory and crafting panels
    /// open beside it (`--station-panel-sample`, #235).
    pub fn with_station_panel_sample(mut self, tab: Option<&str>) -> Self {
        if let (Some(tab), None) = (tab, &self.session) {
            self.station_panel_sample = Some(ui::station::sample());
            self.station_panel_tab = Some(match tab {
                "automation" => ui::station::Tab::Automation,
                _ => ui::station::Tab::Craft,
            });
            self.inventory.open = true;
        }
        self
    }

    /// Offline, show the hall as if goods waited (`--hall-sample`).
    /// Offline, sample shops in the bazaar's stalls and the shop panel open
    /// on stall `stall` (a number, or "mine"), as if you stood at it
    /// (`--bazaar-sample`).
    pub fn with_bazaar_sample(mut self, stall: Option<&str>) -> Self {
        if let (Some(stall), None) = (stall, &self.session) {
            let sample = ui::shop::Sample::new();
            let id = match stall {
                "mine" => sample.mine().map_or_else(String::new, |s| s.stall_id.clone()),
                n => format!("bazaar_stall_{n}"),
            };
            if !id.is_empty() && !stall.is_empty() {
                self.shop.open_on(&id);
            }
            self.bazaar_sample = Some(sample);
        }
        self
    }

    pub fn with_hall_sample(mut self, on: bool) -> Self {
        self.hall_sample = on;
        self
    }

    /// The land (W9). Once a field is sampled, everything stands on it:
    /// the disc opens over the wilds, and the chunks, the click's and the
    /// rig's ground, the grass, the trails and the nodes take it up, and
    /// the water lies in its beds with bridges over it (W10). Then,
    /// every frame, the chunks the camera has come nearer to or gone
    /// further from are built again.
    fn land_in_place(&mut self, ctx: &mut ExtensionCtx) {
        let eye = ctx.camera.position.to_array();
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let mut names = Vec::new();
        if let Some(sampled) = self.land.take() {
            let field = std::sync::Arc::new(sampled.field);
            world::land::set(Some(field.clone()));
            if let Some(projector) = &self.shared.projector {
                projector.set_ground(Some(field.clone()));
            }
            let fords = self.built.iter().flat_map(|t| &t.fords).map(|f| {
                (crate::coords::to_world(f.x, f.y), (0.5 * f.width) as f32 * crate::coords::S)
            });
            let shore = world::land_mesh::Shore {
                cut: sampled.cut.clone(),
                wet: sampled.wet.clone(),
                fords: fords.collect(),
            };
            self.land_meshes.set_field(field.clone(), std::sync::Arc::new(shore));
            // The valleys' mist (W11), shown by the hour.
            world::mist::upload(gpu, &world::mist::build(&field));
            names.push(world::mist::MESH.to_string());
            self.mist_shown = None;
            // The wilds' water lies in its beds (W10), with bridges where
            // a trail crosses a river with no ford.
            world::wild_water::upload(gpu, &sampled.water);
            // Heard as drawn: the courses past the edge run on (#229).
            self.sound.set_wild_water(crate::sound::WildWater::of(sampled.drawn.wild()));
            if let Some(terrain) = &self.built {
                let ground = |at: [f32; 2]| crate::coords::layer::GROUND + world::land::rise_on(&field, at);
                let spans = world::bridges::trail_spans(terrain, ground);
                names.push(world::bridges::upload_trails(gpu, &spans));
                self.players.set_trail_decks(spans);
            }
            if let Some(terrain) = &self.built {
                let rings = &terrain.water_rings;
                world::ground::upload(gpu, terrain.world_size, &world::water::holes(rings), true);
                // The wild trails lie on the land.
                names.extend(world::roads::upload(gpu, &terrain.roads, rings));
                self.grass.land(field.clone(), sampled.cut, sampled.wet, terrain);
            }
            self.land_in = true;
            self.nodes.land_changed();
        }
        let land = self.land_meshes.update(gpu, eye);
        names.extend(land.iter().map(|(name, _)| name.clone()));
        attach_each(ctx, names.iter());
        // The plain's chunks have nothing on them tall enough to throw a
        // shadow, and the low sun's cascades take in a hundred of them.
        for (name, _) in land.iter().filter(|(_, casts)| !casts) {
            let Some(id) = ctx.world.get_id(name) else { continue };
            let mut table = toml::Table::new();
            table.insert("cast_shadows".into(), false.into());
            if let Err(e) = ctx.world.merge_component(id, "material", table.into()) {
                tracing::warn!("land: can't keep {name}'s shadow off: {e}");
            }
        }
    }

    /// The valleys' mist as thick as the hour and the camera's height say
    /// (`Atmosphere::mist`), written to its entity's opacity when it moves
    /// a step. It casts no shadow, however thick (#232).
    fn mist_in_place(&mut self, ctx: &mut ExtensionCtx) {
        let mist = (self.atmosphere.mist() * 100.0).round() / 100.0;
        if self.mist_shown == Some(mist) {
            return;
        }
        let Some(id) = ctx.world.get_id(world::mist::MESH) else {
            return;
        };
        let mut table = toml::Table::new();
        table.insert("opacity".into(), f64::from(mist).into());
        table.insert("cast_shadows".into(), false.into());
        match ctx.world.merge_component(id, "material", table.into()) {
            Ok(()) => self.mist_shown = Some(mist),
            Err(e) => tracing::warn!("mist: can't write its opacity: {e}"),
        }
    }

    /// The Exchange hall as scripts see it. Offline with `--hall-sample`,
    /// you stand at `camera_target` (world metres). `None` until the terrain
    /// brings a hall.
    fn hall_mark(&self, camera_target: [f32; 2]) -> Option<script_api::HallMark> {
        let site = self.hall.site()?;
        let hall = self.terrain.landmark(LandmarkKind::ExchangeHall)?;
        let (you, goods) = match &self.session {
            Some(session) => {
                let state = session.state();
                (
                    state.local_player().map(|p| [p.x, p.y]),
                    capitol_core::hall::goods_waiting(state.market_box()),
                )
            }
            None if self.hall_sample => (
                Some(crate::coords::to_server(camera_target[0], camera_target[1])),
                true,
            ),
            None => (None, false),
        };
        let world = |p: (f64, f64)| crate::coords::to_world(p.0, p.1);
        let (board, board_top) = site.board();
        let news = match (&self.session, &self.gazette_sample) {
            (Some(session), _) => session.gazette().unread(),
            (None, Some(reader)) => reader.unread(),
            _ => false,
        };
        Some(script_api::HallMark {
            corners: hall.corners.map(world),
            door: site.door(),
            roof: site.roof(),
            near: you.is_some_and(|[x, y]| rules::market::at_the_hall(hall, x, y)),
            goods,
            open: self.market.open,
            board,
            board_top,
            news,
            reading: self.gazette.open,
        })
    }

    /// The bazaar's stalls as scripts see them (E3.11): each one's footprint,
    /// sign and shop, and whether you're in its range. Offline with
    /// `--bazaar-sample`, you stand at `camera_target` (world metres).
    /// `None` until the terrain brings a bazaar.
    fn bazaar_mark(&self, camera_target: [f32; 2]) -> Option<script_api::BazaarMark> {
        let landmarks = &self.built.as_ref()?.landmarks;
        landmarks.iter().find(|l| l.kind == LandmarkKind::Bazaar)?;
        let (you, directory, mine, walking) = match (&self.session, &self.bazaar_sample) {
            (Some(session), _) => {
                let state = session.state();
                let store = state.bazaar();
                (
                    state.local_player().map(|p| [p.x, p.y]),
                    store.directory(),
                    store.my_shop().map(|s| s.stall_id.clone()),
                    session.walking_to_stall().map(str::to_owned),
                )
            }
            (None, Some(sample)) => (
                Some(crate::coords::to_server(camera_target[0], camera_target[1])),
                Some(&sample.directory),
                sample.mine().map(|s| s.stall_id.clone()),
                None,
            ),
            (None, None) => return None,
        };
        let world = |p: (f64, f64)| crate::coords::to_world(p.0, p.1);
        let stalls = landmarks
            .iter()
            .filter(|l| l.kind == LandmarkKind::Stall)
            .map(|stall| {
                let shop = directory
                    .and_then(|d| d.stalls.iter().find(|s| s.stall_id == stall.id))
                    .and_then(|s| s.shop.as_ref());
                let sign = self.bazaar.signs().iter().find(|s| s.stall_id == stall.id);
                script_api::StallMark {
                    id: stall.id.clone(),
                    corners: stall.corners.map(world),
                    sign: sign.map_or_else(|| { let [x, z] = world(stall.position); [x, 1.5, z] }, |s| s.at),
                    colour: sign.map_or([0.8, 0.78, 0.72], |s| s.colour),
                    sign_facing: sign.map_or([0.0, 0.0, 1.0], |s| s.facing),
                    sign_width: sign.map_or(0.0, |s| s.width),
                    sign_depth: sign.map_or(0.0, |s| s.depth),
                    name: shop.map(|s| s.name.clone()),
                    keeper: shop.map(|s| s.owner_name.clone()),
                    citizen: shop.is_some_and(|s| s.owner_citizen),
                    mine: mine.as_deref() == Some(stall.id.as_str()),
                    near: you.is_some_and(|[x, y]| rules::bazaar::at_the_stall(stall, x, y)),
                    open: self.shop.stall.as_deref() == Some(stall.id.as_str()),
                }
            })
            .collect();
        Some(script_api::BazaarMark { stalls, walking })
    }

    /// Open the market panel preset (`--market ITEM[:SIDE:N@PRICE[:GRADE]]`),
    /// and offline, on a sample market (`--market-sample`).
    pub fn with_market(mut self, spec: Option<&str>, sample: bool) -> Self {
        if let Some(spec) = spec {
            self.market.preset(spec);
        }
        if sample && self.session.is_none() {
            self.market_sample = Some(ui::market::Sample::new());
            if spec.is_none() {
                self.market.preset("plank:buy:12@42:C");
            }
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
        if !self.at_login() && ctx.input.is_action_just_pressed("market") {
            self.market.toggle();
        }
        // The Gazette (E3.9): G, or a click on its notice board
        if !self.at_login() && ctx.input.is_action_just_pressed("gazette") {
            self.gazette.toggle();
        }
        if self.shared.read_gazette.swap(false, Ordering::Relaxed) {
            self.gazette.open = true;
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
            // The Exchange hall (E3.7): a click on it opens the market there,
            // or walks you to its door and opens it as you come into range.
            if let Some(hall) = self.terrain.landmark(LandmarkKind::ExchangeHall) {
                let clicked = self.shared.visit_hall.swap(false, Ordering::Relaxed);
                if (clicked && session.visit_hall(now, hall)) || session.at_hall(hall) {
                    self.market.open = true;
                }
            }
            // The bazaar (E3.11): a click on a stall walks you there, in at
            // the gate and up the lane, and its shop opens as you arrive.
            let clicked = self.shared.visit_stall.lock().ok().and_then(|mut v| v.take());
            if let Some(id) = clicked {
                visit_stall(session, &self.terrain, &mut self.shop, &id, now);
            }
            if let Some(id) = session.walking_to_stall().map(str::to_owned) {
                if let Some(stall) = self.terrain.landmark_by_id(&id) {
                    if session.at_stall(now, stall) {
                        self.shop.open_on(&id);
                    }
                }
            }
            // The Gazette follows whoever is signed in, and counts what's
            // opened as read
            let (server, me) = (session.server().to_owned(), session.state().me());
            self.gazette.follow(&server, me, session.gazette_mut());
            self.gazette.reading(session.gazette_mut());
            let made = crafted(ctx, session, &self.shared, &mut self.sound);
            // The hints follow whoever is signed in (nobody, signed out).
            let state = session.state();
            self.hints.follow(
                session.server(),
                state.me(),
                state.inventory(),
                state.catalog(),
            );
            if session.destination().is_some() {
                self.hints.saw(Did::Walked);
            }
            if gathered {
                self.hints.saw(Did::Gathered);
            }
            if made {
                self.hints.saw(Did::Crafted);
            }
            plots(ctx, session, &self.shared, &self.lots, &mut self.sound);
            // Signed out, the store is empty, so everyone fades away.
            let (poses, state) = session.sample(now);
            let dt = ctx.clock.delta_time.min(0.1) as f32;
            self.players.update(ctx, poses, state, dt, &self.shared);
            trade(ctx, session, &self.shared, now);
            for note in session.take_market_notes() {
                let mut data = toml::Table::new();
                data.insert("title".into(), note.title.into());
                data.insert("body".into(), note.body.into());
                ctx.fire_event("market_note", data);
            }
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
            let state = session.state();
            let work = |id| {
                let player = state.harvester(id)?;
                match state.activity(player) {
                    Some(Activity::Extracting {
                        progress, duration, ..
                    }) => Some(nodes::Work {
                        player,
                        progress: *progress,
                        duration: *duration,
                    }),
                    _ => None,
                }
            };
            let changes = self.nodes.update(
                ctx,
                state.nodes(),
                &work,
                state.me(),
                target,
                dt,
                &self.shared,
            );
            fire_node_changes(ctx, changes);
            let placed = session.take_placed();
            let me = self.shared.follow.lock().ok().and_then(|f| *f);
            let produced = session.take_produced();
            self.stations
                .update(ctx, session.state(), me, &placed, &produced, dt, &self.shared);
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
            placing(
                ctx,
                session,
                &self.shared,
                &mut self.stations,
                self.built.as_ref(),
                now,
                dt,
            );
        } else if let Some(sample) = &mut self.station_sample {
            // Placed once the camera has settled on its shot.
            if sample.is_none() && ctx.clock.total_time > 0.5 {
                let (t, c) = (ctx.camera.target, ctx.camera.position);
                *sample = Some(match self.automation_sample {
                    Some(_) => crate::stations::automation_sample([t.x, t.z], [c.x, c.z]),
                    None => crate::stations::sample([t.x, t.z], [c.x, c.z]),
                });
            }
            if let Some(state) = sample {
                let none = std::collections::HashSet::new();
                let dt = ctx.clock.delta_time.min(0.1) as f32;
                let produced = match &mut self.automation_sample {
                    Some(t) => {
                        *t += dt;
                        crate::stations::automation_tick(state, *t, dt)
                    }
                    None => Vec::new(),
                };
                self.stations
                    .update(ctx, state, None, &none, &produced, dt, &self.shared);
            }
        }
        if let Some(sample) = &mut self.citizen_sample {
            // Stood once the camera has settled on its shot.
            if sample.is_none() && ctx.clock.total_time > 0.5 {
                let (t, c) = (ctx.camera.target, ctx.camera.position);
                *sample = Some(crate::entities::sample([t.x, t.z], [c.x, c.z]));
            }
            if let Some((state, poses)) = sample {
                let dt = ctx.clock.delta_time.min(0.1) as f32;
                self.players.update(ctx, poses, state, dt, &self.shared);
            }
        }
        if self.session.is_none() && self.shared.visit_hall.swap(false, Ordering::Relaxed) {
            self.market.open = true;
        }
        if self.session.is_none() {
            let clicked = self.shared.visit_stall.lock().ok().and_then(|mut v| v.take());
            if let (Some(id), Some(_)) = (clicked, &self.bazaar_sample) {
                self.shop.open_on(&id);
            }
        }
        if self.session.is_none() && self.node_sample && ctx.clock.total_time > 0.5 {
            // Offline, every kind of node round where the camera looks
            // half a second in (`--node-sample`), once a `--view` or a
            // `--camera-path` has put it there.
            let dt = ctx.clock.delta_time.min(0.1) as f32;
            let t = ctx.camera.target;
            let sample = self
                .node_sampled
                .get_or_insert_with(|| nodes::sample::Sample::new(crate::coords::to_server(t.x, t.z)));
            sample.update(dt);
            let sample = &*sample;
            let changes = self.nodes.update(
                ctx,
                sample.nodes(),
                &|id| sample.work(id),
                None,
                None,
                dt,
                &self.shared,
            );
            fire_node_changes(ctx, changes);
        }
        self.shared
            .at_login
            .store(self.at_login(), Ordering::Relaxed);
        listen(ctx, &self.shared, &mut self.sound);
        let Some(gpu) = ctx.gpu.as_mut() else {
            return;
        };
        let phase = self.phase();
        self.atmosphere.update(
            gpu.renderer,
            ctx.world,
            crate::atmosphere::height_over_land(ctx.camera.position.to_array()),
            phase,
            ctx.clock.delta_time.min(0.1),
        );
        if !self.world_built {
            spike_world::upload_static(gpu);
            self.world_built = true;
        }
        // Your plots in gold, rebuilt when they change. The light on them
        // follows the day (P8.6a) on their entity's material, sliding the
        // curb from lit gold to light (#139, Flint ADR 0096). Offline, none.
        let owned = self
            .session
            .as_ref()
            .map_or_else(Vec::new, |s| owned_lots(s.state().plots(), &self.lots));
        let glow = self.atmosphere.plots();
        if self.owned_drawn.as_ref() != Some(&owned) {
            let lit = owned.iter().filter_map(|id| self.lots.get(id));
            lots::upload_owned(gpu, lit);
            let any = !owned.is_empty();
            self.owned_drawn = Some(owned);
            if any {
                attach_each(ctx, std::iter::once(&lots::OWNED_MESH.to_owned()));
            }
        }
        set_material(ctx.world, lots::OWNED_MESH, &lots::owned_glow(glow));
        // Houses on everyone's claimed lots (P8.6b): online the store's set,
        // offline `--claim-sample`'s.
        let dt = ctx.clock.delta_time.min(0.1) as f32;
        if let Some(session) = &self.session {
            let state = session.state();
            let claims = state.claimed_lots();
            let now = state.server_time_ms();
            let fresh = |id: &str| {
                claims
                    .get(id)
                    .is_some_and(|c| world::houses::fresh(c.claimed_at.timestamp_millis(), now))
            };
            self.houses
                .sync(state.claimed_version(), claims.keys(), fresh, &self.lots);
        } else if let Some(sample) = &mut self.claim_sample {
            let t = ctx.camera.target;
            sample.update(dt, crate::coords::to_server(t.x, t.z), &self.lots);
            let growing = &sample.growing;
            self.houses.sync(
                sample.version,
                &sample.claimed,
                |id| growing.contains(id),
                &self.lots,
            );
        }
        self.houses.advance(dt, &self.lots);
        if let Some(gpu) = ctx.gpu.as_mut() {
            let uploaded = self.houses.upload(gpu, &self.lots);
            self.solids_stale |= !uploaded.is_empty();
            attach_each(ctx, uploaded.iter());
        }
        // The lamps in the windows glow and change colour with the hour,
        // set on their entities, so no mesh is rebuilt (H23, #269; Flint
        // ADRs 0090, 0096 and 0102).
        let lamps = world::houses::lamp_glow(glow);
        for name in world::houses::lamp_meshes() {
            set_material(ctx.world, &name, &lamps);
        }
        // So do the fires in the forges and kilns.
        self.stations
            .fires(ctx.world, crate::stations::fire_light(glow));
        // And the Exchange hall's glass (E3.7). Its clock keeps the hour,
        // and the scripts hear where it is, for the prompt and the mote.
        set_material(
            ctx.world,
            world::hall::LAMPS_MESH,
            &[("emissive_scale", world::hall::glass_light(glow))],
        );
        if let Some(gpu) = ctx.gpu.as_mut() {
            self.hall.tick(gpu, phase);
        }
        let camera = ctx.camera;
        let mark = self.hall_mark([camera.target.x, camera.target.z]);
        self.hall_hint = mark.as_ref().filter(|h| h.goods && !h.near).map(|h| {
            ui::hud::bearing(
                camera.position.to_array(),
                camera.target.to_array(),
                h.roof,
            )
        });
        if let Ok(mut shared) = self.shared.hall.lock() {
            *shared = mark;
        }
        // The bazaar (E3.11), across the avenue: built from the terrain and
        // the directory (which stalls are kept, and what they sell), and its
        // lanterns lit with the hour
        if let (Some(gpu), Some(terrain)) = (ctx.gpu.as_mut(), self.built.as_ref()) {
            let directory = match (&self.session, &self.bazaar_sample) {
                (Some(session), _) => session.state().bazaar().directory(),
                (None, Some(sample)) => Some(&sample.directory),
                (None, None) => None,
            };
            let shop = |id: &str| {
                directory
                    .and_then(|d| d.stalls.iter().find(|s| s.stall_id == id))
                    .and_then(|s| s.shop.as_ref())
            };
            if self.bazaar.sync(gpu, &terrain.landmarks, shop) {
                attach_each(ctx, world::bazaar::meshes().iter());
                self.solids_stale = true;
                if let Ok(mut feet) = self.shared.stall_feet.lock() {
                    *feet = terrain
                        .landmarks
                        .iter()
                        .filter(|l| l.kind == LandmarkKind::Stall)
                        .map(|l| l.corners.map(|p| crate::coords::to_world(p.0, p.1)))
                        .collect();
                }
            }
        }
        if std::mem::take(&mut self.solids_stale) {
            // What the camera keeps out of (E3.13): every building's bulk
            let mut solids: Vec<world::solids::Solid> = self.bazaar.solids().collect();
            solids.extend(self.hall.site().map(|s| s.solid()));
            solids.extend(self.houses.solids().copied());
            if let Ok(mut shared) = self.shared.solids.lock() {
                *shared = solids;
            }
        }
        set_material(
            ctx.world,
            world::bazaar::LAMPS_MESH,
            &[("emissive_scale", world::bazaar::lantern_light(glow))],
        );
        let mark = self.bazaar_mark([camera.target.x, camera.target.z]);
        if let Ok(mut shared) = self.shared.bazaar.lock() {
            *shared = mark;
        }
        // The grass, cleared round every node the server has shown us, as
        // wide as its kind needs.
        let shown = match (&self.session, &self.node_sampled) {
            (Some(session), _) => Some(session.state().nodes()),
            (None, Some(sample)) => Some(sample.nodes()),
            (None, None) => None,
        };
        if let Some(shown) = shown {
            self.grass.nodes(shown.values().map(|n| {
                let kind = nodes::models::Kind::of(&n.resource_type);
                ([n.x, n.y], kind.clearing())
            }));
        }
        self.land_in_place(ctx);
        self.mist_in_place(ctx);
        if let Some(gpu) = ctx.gpu.as_mut() {
            self.grass.flush(gpu, ctx.world);
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
            let holes = world::water::holes(rings);
            world::ground::upload(gpu, terrain.world_size, &holes, self.land_in);
        }
        // The land is sampled off this thread, and comes in on a later
        // frame (`land_in_place`); until then the world stands as it was.
        self.land.ask(terrain);
        self.sound.set_wild_water(crate::sound::WildWater::new(terrain));
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
            let (plots, lots) = world::plots::upload(gpu, &terrain.terrain_plots);
            sectors.extend(plots);
            self.lots = lots;
            // The lots moved, so the gold and the houses are built again.
            self.owned_drawn = None;
            self.houses.lots_changed();
        }
        if rebuild.hall {
            let hall = terrain
                .landmarks
                .iter()
                .find(|l| l.kind == LandmarkKind::ExchangeHall);
            self.hall.terrain(gpu, hall, phase);
            sectors.extend(world::hall::meshes());
            self.solids_stale = true;
        }
        if rebuild.ground || rebuild.roads || rebuild.plots || rebuild.parks {
            self.grass.terrain(terrain);
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

    /// `?` opens and closes the shortcut overlay (P8.3), read off the
    /// character so it's `?` on any layout. While it's open, Esc closes it.
    /// While a station is being placed (P6.2), Esc stops it rather than
    /// quitting. Keys come here rather than to `on_window_event` so an input
    /// script can press them too (#273, Flint ADR 0103).
    fn on_key(&mut self, key: &KeyInput) -> bool {
        if self.in_world() && key.character == Some("?") {
            if key.pressed() {
                self.shortcuts.toggle();
            }
            return true;
        }
        if key.key != KeyCode::Escape {
            return false;
        }
        if self.shortcuts.open {
            if key.pressed() {
                self.shortcuts.open = false;
            }
            return true;
        }
        if self.session.as_ref().is_some_and(|s| s.placing().is_some()) {
            if key.state == ElementState::Pressed {
                self.cancel_placing = true;
            }
            return true;
        }
        false
    }

    /// While a station is being placed (P6.2), a right click on the world
    /// stops it (not a right-drag, which orbits), as Esc does. egui has
    /// already kept any click over its panels.
    fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        let placing = self.session.as_ref().is_some_and(|s| s.placing().is_some());
        match event {
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
        false
    }

    fn ui(&mut self, ctx: &egui::Context) {
        ui::theme::install(ctx);
        // What a tool's tooltip reads its power from (E1.7)
        match &self.session {
            Some(session) => ui::slots::know_catalog(ctx, session.state().catalog()),
            None => ui::slots::know_catalog(ctx, &capitol_core::catalog::Catalog::fallback()),
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
                    patron: state.is_patron().then(|| {
                        match state.subscription().and_then(|s| s.until) {
                            Some(until) => format!("Patron until {}", until.format("%-d %B %Y")),
                            None => "Patron".to_owned(),
                        }
                    }),
                    connection: session.auth().connection(),
                    down: session.link().outage().is_some(),
                    inventory_open: self.inventory.open,
                    crafting_open: self.crafting.open,
                    exchange_open: self.exchange.open,
                    market_open: self.market.open,
                    gazette_open: self.gazette.open,
                    gazette_news: session
                        .gazette()
                        .unread()
                        .then(|| session.gazette().latest_headline().unwrap_or_default().to_owned()),
                    help_open: self.shortcuts.open,
                    settings_open: self.settings_panel.open,
                    inventory_pulse: pulse,
                    hall_hint: self.hall_hint,
                    impound: state.impound().map(|c| c.used_slots()),
                    impound_open: self.impound.open,
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
                if clicks.market {
                    self.market.toggle();
                }
                if clicks.gazette {
                    self.gazette.toggle();
                }
                if clicks.hall {
                    self.shared.visit_hall.store(true, Ordering::Relaxed);
                }
                if clicks.impound {
                    self.impound.toggle();
                    // Somewhere to drag it to
                    self.inventory.open |= self.impound.open;
                }
                if clicks.help {
                    self.shortcuts.toggle();
                }
                if clicks.settings {
                    self.settings_panel.toggle();
                }
                // Another device or a bot plays the avatar (E0.8)
                if ui::hud::presence_banner(ctx, session.state().presence()) {
                    session.claim_presence();
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
                // The market (E3.5): watch the item it shows while it's open
                session.set_market_open(self.market.open);
                session.watch_market(self.market.watching());
                session.chart_market(self.market.charting());
                let state = session.state();
                let market = state.market();
                // D6: placing and collecting need the Exchange hall
                let away_from_hall = self
                    .terrain
                    .landmark(LandmarkKind::ExchangeHall)
                    .zip(state.local_player())
                    .filter(|(hall, me)| !rules::market::at_the_hall(hall, me.x, me.y))
                    .map(|(hall, me)| hall.distance_to(me.x, me.y));
                let view = ui::market::View {
                    catalog: state.catalog(),
                    summary: session.market_summary().map(|s| s.items.as_slice()),
                    book: self.market.selected.as_deref().and_then(|i| market.book(i)),
                    history: market.history(),
                    orders: market.open_orders(),
                    inventory: state.inventory(),
                    market_box: state.market_box(),
                    strands: state.strands(),
                    busy: session.market_busy(),
                    refused: session.market_refused(now),
                    away_from_hall,
                    bazaar: state.bazaar().directory(),
                    my_stall: state.bazaar().my_shop().map(|s| s.stall_id.as_str()),
                    walking_to: session.walking_to_stall(),
                };
                match self.market.show(ctx, &mut self.icons, &view) {
                    Some(ui::market::Action::Place(order)) => session.place_order(order),
                    Some(ui::market::Action::Cancel(id)) => session.cancel_order(id),
                    Some(ui::market::Action::Collect) => session.collect_market_box(),
                    Some(ui::market::Action::VisitStall(id)) => {
                        // Out of the way, so you see the walk
                        self.market.open = false;
                        visit_stall(session, &self.terrain, &mut self.shop, &id, now);
                    }
                    Some(ui::market::Action::Close) => self.market.open = false,
                    None => {}
                }
                // The shop at the stall you're at (E3.11)
                session.watch_shop(self.shop.stall.as_deref());
                let state = session.state();
                let store = state.bazaar();
                let action = match self.shop.stall.clone() {
                    Some(stall_id) => {
                        let away = self
                            .terrain
                            .landmark_by_id(&stall_id)
                            .zip(state.local_player())
                            .filter(|(stall, me)| !rules::bazaar::at_the_stall(stall, me.x, me.y))
                            .map(|(stall, me)| stall.distance_to(me.x, me.y));
                        let view = ui::shop::View {
                            stall_id: &stall_id,
                            shop: store.shop(&stall_id),
                            sign: store
                                .directory()
                                .and_then(|d| d.stalls.iter().find(|s| s.stall_id == stall_id))
                                .and_then(|s| s.shop.as_ref()),
                            listed: store.directory().is_some(),
                            mine: store.my_shop(),
                            me: state.me(),
                            away,
                            inventory: state.inventory(),
                            strands: state.strands(),
                            catalog: state.catalog(),
                            busy: session.bazaar_busy(),
                            refused: session.bazaar_refused(now),
                            now: state
                                .server_time_ms()
                                .and_then(capitol_core::chrono::DateTime::from_timestamp_millis)
                                .unwrap_or_else(capitol_core::chrono::Utc::now),
                        };
                        self.shop.show(ctx, &mut self.icons, Some(&view), self.crafting.rect)
                    }
                    None => self.shop.show(ctx, &mut self.icons, None, self.crafting.rect),
                };
                match action {
                    Some(ui::shop::Action::Buy { listing_id, request }) => session.buy_listing(listing_id, request),
                    Some(ui::shop::Action::Rent { stall_id, request }) => session.rent_stall(&stall_id, request),
                    Some(ui::shop::Action::Stock(request)) => session.stock_shop(request),
                    Some(ui::shop::Action::Unlist(id)) => session.unlist(id),
                    Some(ui::shop::Action::GoTo(id)) => visit_stall(session, &self.terrain, &mut self.shop, &id, now),
                    Some(ui::shop::Action::Close) => self.shop.close(),
                    None => {}
                }
                // What a foreclosure kept for you (#141)
                let state = session.state();
                let view = ui::impound::View {
                    impound: state.impound(),
                    inventory: state.inventory(),
                    busy: session.impound_busy(),
                    refused: session.impound_refused(now),
                };
                match self.impound.show(ctx, &mut self.icons, &view, self.inventory.rect) {
                    Some(ui::impound::Action::TakeAll) => session.collect_impound(),
                    Some(ui::impound::Action::Close) => self.impound.open = false,
                    None => {}
                }
                // The Gazette (E3.9)
                let view = ui::gazette::View { reader: session.gazette(), online: true };
                match self.gazette.show(ctx, &view) {
                    Some(ui::gazette::Action::Turn(by)) => session.gazette_mut().turn(by),
                    Some(ui::gazette::Action::Byline(named)) => session.set_byline(named),
                    None => {}
                }
                // The settings panel takes the hint card's corner, and the
                // Gazette the middle of the screen; the card waits behind
                // them rather than showing through.
                if self.settings_panel.open || self.gazette.open {
                    self.hints.rect = None;
                } else {
                    // The inventory's and the station's rects are last frame's: they're drawn after.
                    let panels: Vec<_> =
                        [self.inventory.rect, self.crafting.rect, self.exchange.rect, self.market.rect, self.impound.rect, self.shop.rect, self.station.rect]
                            .into_iter()
                            .flatten()
                            .collect();
                    self.hints.show(ctx, now, &panels);
                }
                let state = session.state();
                let picked = state.picked_plot();
                let view = ui::plot::View {
                    plot: picked,
                    offer: picked.map_or(Offer::NotForSale, |p| {
                        Offer::new(p, state.me(), state.strands())
                    }),
                    strands: state.strands(),
                    claiming: session.claiming(),
                    paying: session.paying(),
                    refused: session
                        .claim_refused(now)
                        .or_else(|| session.pay_refused(now)),
                };
                match self.plot_panel.show(ctx, &view, self.crafting.rect) {
                    Some(ui::plot::Action::Claim(id)) => session.claim_plot(id),
                    Some(ui::plot::Action::PayTax(id, amount)) => session.pay_tax(id, amount),
                    Some(ui::plot::Action::Close) => session.close_plot(),
                    None => {}
                }
                let view = ui::station::View {
                    state: session.state(),
                    phase: session.crafting().phase(),
                    refused: session
                        .craft_refused(now)
                        .or_else(|| session.move_refused(now)),
                    beside: self.inventory.rect,
                    automation_refused: session.automation_refused(now),
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
                    Some(StationAction::InstallTool {
                        station_id,
                        item_id,
                    }) => session.install_tool(station_id, item_id),
                    Some(StationAction::RemoveTool(id)) => session.remove_tool(id),
                    Some(StationAction::StartAutomation { station_id, draft }) => {
                        session.start_automation(station_id, &draft)
                    }
                    Some(StationAction::StopAutomation(id)) => session.stop_automation(id),
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
                let state = session.state();
                let clicks = self.inventory.show(
                    ctx,
                    &mut self.icons,
                    state.catalog(),
                    state.inventory(),
                    refused,
                    pulse,
                );
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
            let clear_of = self.plot_panel.rect.or(self.market.rect).or(self.crafting.rect);
            ui::controls(ctx, &self.terrain.status(), clear_of);
        }
        if self.icon_sheet {
            ui::icons::sheet(ctx, &mut self.icons);
        }
        if let Some(sample) = &mut self.sample {
            sample.show(ctx, &mut self.icons);
        }
        if let Some(sample) = &self.market_sample {
            let history = self.market.charting().map(|key| sample.history(&key));
            let mut view = sample.view_with(self.market.selected.as_deref(), history.as_ref());
            view.bazaar = self.bazaar_sample.as_ref().map(|b| &b.directory);
            view.my_stall = self.bazaar_sample.as_ref().and_then(|b| b.mine()).map(|s| s.stall_id.as_str());
            match self.market.show(ctx, &mut self.icons, &view) {
                Some(ui::market::Action::Close) => self.market.open = false,
                Some(ui::market::Action::VisitStall(id)) if self.bazaar_sample.is_some() => {
                    self.market.open = false;
                    self.shop.open_on(&id);
                }
                _ => {}
            }
        }
        if let (Some(sample), None) = (&self.bazaar_sample, &self.session) {
            let catalog = capitol_core::catalog::Catalog::fallback();
            let stall = self.shop.stall.clone();
            let view = stall.as_deref().map(|id| sample.view(id, &catalog));
            if let Some(ui::shop::Action::Close) = self.shop.show(ctx, &mut self.icons, view.as_ref(), None) {
                self.shop.close();
            }
        }
        if let Some((impound, inventory)) = &self.impound_sample {
            let view = ui::impound::View {
                impound: Some(impound),
                inventory: Some(inventory),
                busy: false,
                refused: None,
            };
            if let Some(ui::impound::Action::Close) = self.impound.show(ctx, &mut self.icons, &view, None) {
                self.impound.open = false;
            }
        }
        if let Some(state) = &self.station_panel_sample {
            let phase = capitol_core::craft::Phase::Idle;
            let view = ui::crafting::View { state, phase: &phase, refused: None };
            self.crafting.show(ctx, &mut self.icons, &view, 0.0);
            // The inventory's rect is last frame's: it's drawn after.
            let view = ui::station::View {
                state,
                phase: &phase,
                refused: None,
                beside: self.inventory.rect,
                automation_refused: None,
            };
            self.station.show(ctx, &mut self.icons, &view, 0.0);
            if let Some(tab) = self.station_panel_tab.take() {
                self.station.tab = tab;
            }
            self.inventory.show(ctx, &mut self.icons, state.catalog(), state.inventory(), None, 0.0);
        }
        if let Some(reader) = &mut self.gazette_sample {
            self.gazette.reading(reader);
            if let Some(ui::gazette::Action::Turn(by)) =
                self.gazette.show(ctx, &ui::gazette::View { reader, online: false })
            {
                reader.turn(by);
            }
        }
        if let Some(juice) = &mut self.juice {
            juice.ui(ctx, &mut self.icons, &self.shared);
        }
        ui::slots::paint_shatters(ctx);
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

/// Goes to `stall_id` in the bazaar (E3.11): its shop opens now if you're at
/// it, or once the walk through the gate brings you there.
fn visit_stall(
    session: &mut Session,
    terrain: &TerrainSource,
    shop: &mut ui::shop::ShopPanel,
    stall_id: &str,
    now: Instant,
) {
    let Some(stall) = terrain.landmark_by_id(stall_id) else {
        return;
    };
    if session.visit_stall(now, &terrain.bazaar_streets(), stall) {
        shop.open_on(stall_id);
    }
}

/// What you hear (P8.1): ease the beds toward where you are, and play
/// each one-shot due. The ears are on you, or offline on the point the
/// camera looks at.
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
        // Over the land under the camera, so a hillside isn't a height.
        height: cam.position.y - crate::world::land::ground_y([cam.position.x, cam.position.z]),
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
    let cues = sound.update(ears, &working, dt);
    sound.apply(ctx, &cues);
}

/// Click to move (P4.3): send the walk a script asked for (`cap_move`),
/// tell scripts where you're walking to (`cap_destination`) and by which
/// corners round the water (`cap_route`, #267), and fire
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
    if let Ok(mut r) = shared.route.lock() {
        r.clear();
        r.extend(session.route().into_iter().map(|(x, y)| crate::coords::to_world(x, y)));
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
        data.insert("water".into(), water_name(&blocked.water).into());
        // The slope stopped you, not water (W4): no splash, no bridge to find.
        data.insert("steep".into(), (blocked.reason == rules::ground::TOO_STEEP).into());
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
        Action::Qualities { item, picked } => {
            session.draft_mut().set_qualities(&item, picked, now)
        }
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

/// Tells the scripts a node was taken (`node_depleted`) or grew back
/// (`node_regenerated`), with its place (`x`, `z`), its model (`kind`) and
/// the colour of the bits it throws (`r`, `g`, `b`, sRGB 0 to 1).
fn fire_node_changes(ctx: &mut ExtensionCtx, changes: Vec<nodes::Changed>) {
    for c in changes {
        let [x, z] = c.at;
        let [r, g, b] = c.kind.bits();
        let mut data = toml::Table::new();
        data.insert("x".into(), (x as f64).into());
        data.insert("z".into(), (z as f64).into());
        data.insert("kind".into(), c.kind.name().into());
        data.insert("r".into(), (r as f64).into());
        data.insert("g".into(), (g as f64).into());
        data.insert("b".into(), (b as f64).into());
        ctx.fire_event(c.name, data);
    }
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
    let any = yields.iter().any(|(_, item)| item.quantity > 0);
    for (node, item) in yields {
        if item.quantity == 0 {
            // A failed pull (D5, #206): a note, not a "+0"
            let (title, body) = missed_words(session.state(), node, &item);
            tracing::info!("extract: {body}");
            let mut data = toml::Table::new();
            data.insert("title".into(), title.into());
            data.insert("body".into(), body.into());
            ctx.fire_event("extraction_note", data);
            continue;
        }
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
    lots: &HashMap<String, Lot>,
    sound: &mut Soundscape,
) {
    if let Some([x, y]) = shared.pick_plot.lock().ok().and_then(|mut p| p.take()) {
        session.pick_plot(x, y);
    }
    let me = session.state().me();
    let mark = session.state().picked_plot().map(|p| {
        let lot = p.terrain_id.as_ref().and_then(|id| lots.get(id));
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
        use crate::session::PlotNote;
        let (title, body): (String, String) = match note {
            PlotNote::Displaced { station_type } => {
                let name = session
                    .state()
                    .catalog()
                    .station_type(&station_type)
                    .map_or_else(|| station_type.replace('_', " "), |t| t.name.to_lowercase());
                (
                    format!("Your {name} was taken down"),
                    "A new lot was laid where it stood. Its kit and what it held are back in your pack, or your impound if it was full".into(),
                )
            }
            PlotNote::NoneThere => ("No plot here".into(), "Alt-click a lot to see its deed".into()),
            PlotNote::Unanswered => ("Couldn't look".into(), "The server didn't say what's here".into()),
            PlotNote::TaxOwed {
                tax_owed,
                forecloses_at,
            } => (
                "Property tax overdue".into(),
                format!("Your plot owes {tax_owed} Strands. At {forecloses_at} it's foreclosed"),
            ),
            PlotNote::SubscriptionLapsed { until } => (
                "Subscription lapsed".into(),
                format!(
                    "It ran out on {}. Stations stop working while you're away",
                    until.format("%-d %B")
                ),
            ),
            PlotNote::Foreclosed {
                tax_owed,
                refund,
                impounded,
                lost,
            } => {
                let catalog = session.state().catalog();
                let mut body = if refund > 0 {
                    format!("{tax_owed} Strands of tax unpaid. {refund} of the deed came back")
                } else {
                    format!("{tax_owed} Strands of tax unpaid took the whole deed")
                };
                if !impounded.is_empty() {
                    let held = ui::impound::counted(catalog, &impounded);
                    body.push_str(&format!(". Your pack was full: {held} wait in your impound"));
                }
                if !lost.is_empty() {
                    let gone = ui::impound::counted(catalog, &lost);
                    body.push_str(&format!(". No room even there for {gone}"));
                }
                ("Plot foreclosed".into(), body)
            }
        };
        let mut data = toml::Table::new();
        data.insert("title".into(), title.into());
        data.insert("body".into(), body.into());
        ctx.fire_event("plot_note", data);
    }
    let egui = shared.egui.lock().ok().and_then(|e| e.clone());
    for (item_id, name) in session.take_broken_tools() {
        if let Some(egui) = &egui {
            ui::slots::shatter(egui, item_id);
        }
        let mut data = toml::Table::new();
        data.insert("title".into(), format!("Your {name} broke").into());
        data.insert("body".into(), "Worn out. Make or buy another".into());
        ctx.fire_event("tool_note", data);
    }
    // A station of yours waiting (E4.4), live or from the notices that
    // waited for you to come back (#138)
    for (station_id, station_type, reason) in session.take_automation_notes() {
        let state = session.state();
        let recipe = state
            .station(station_id)
            .and_then(|s| s.automation.as_ref())
            .and_then(|a| state.recipe(&a.recipe_id));
        let words = capitol_core::automation::pause_words(&reason, state.catalog(), recipe);
        let name = state
            .catalog()
            .station_type(&station_type)
            .map_or_else(|| station_type.replace('_', " "), |t| t.name.to_lowercase());
        let mut data = toml::Table::new();
        data.insert("title".into(), format!("Your {name} paused").into());
        data.insert("body".into(), format!("{}. {}", words.what, words.fix).into());
        tracing::info!("automation: your {name} paused: {}", words.what);
        ctx.fire_event("automation_note", data);
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

/// The ids of your plots' lots, sorted: each joined to its lot by
/// `terrain_id`. A plot with no lot (pre-B7, or a terrain that no longer
/// has it) isn't drawn.
fn owned_lots(
    plots: &[capitol_core::protocol::rest::PlotInfo],
    lots: &HashMap<String, Lot>,
) -> Vec<String> {
    let mut owned: Vec<String> = plots
        .iter()
        .filter_map(|p| p.terrain_id.clone())
        .filter(|id| lots.contains_key(id))
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
    terrain: Option<&TerrainResponse>,
    now: Instant,
    dt: f32,
) {
    use crate::coords::{to_server, to_world, S};
    let click = shared.place_at.lock().ok().and_then(|mut p| p.take());
    if let Some([x, z]) = click {
        let [x, y] = to_server(x, z);
        session.place(now, x, y, terrain);
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
        let check = target.map(|at| p.check(at, state, terrain));
        let ok = matches!(check, Some(Ok(_)));
        let station_type = p.kit().station_type.as_str();
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
            kind: crate::stations::kind_of(station_type),
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
/// The note for a pull that came away with nothing: it fails only when hands
/// or the tool fall short of the resource, so a stronger tool is the answer.
fn missed_words(state: &capitol_core::state::GameState, node: uuid::Uuid, item: &capitol_core::state::Yield) -> (&'static str, String) {
    let what = state
        .node(node)
        .and_then(|n| state.catalog().resource_type(&n.resource_type))
        .map_or_else(|| item.item_name.clone(), |r| r.name.clone());
    ("Nothing this time", format!("The {what} gave nothing. A stronger tool fails less"))
}

fn note_words(note: &Note) -> (&'static str, String) {
    match note {
        Note::Lost(_) => ("Taken", "Someone else got to that grass first".into()),
        Note::Refused(reason) => ("Couldn't gather", reason.clone()),
        Note::Unanswered(_) => ("Couldn't gather", "The server didn't answer".into()),
        Note::Busy => ("Busy", "Finish what you're doing first".into()),
    }
}

/// Sets numbers in `name`'s `material`, such as `emissive_scale` (Flint
/// ADR 0090) and `glow_surface` (ADR 0096), if it has an entity and one of
/// them has moved, so its glow changes with no upload.
fn set_material(world: &mut flint_ecs::FlintWorld, name: &str, fields: &[(&str, f32)]) {
    let Some(id) = world.get_id(name) else {
        return;
    };
    let material = world.get_components(id).and_then(|c| c.get("material"));
    let unmoved = fields.iter().all(|&(key, value)| {
        material.and_then(|m| m.get(key)).and_then(|v| v.as_float()) == Some(value as f64)
    });
    if unmoved {
        return;
    }
    let mut table = material
        .and_then(|m| m.as_table())
        .cloned()
        .unwrap_or_default();
    for &(key, value) in fields {
        table.insert(key.into(), (value as f64).into());
    }
    if let Err(e) = world.set_component(id, "material", table.into()) {
        tracing::warn!("world: no material on {name}: {e}");
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
