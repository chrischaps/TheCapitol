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
use crate::settings::{ControlMode, Controls, Graphics, SettingsFile};
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
use capitol_core::onboarding::{self, Did, Target};
use capitol_core::plot::Offer;
use capitol_core::protocol::rules::{self, water_name, STATION_INTERACTION_RANGE, STATION_MIN_SPACING};
use capitol_core::protocol::terrain::{LandmarkKind, TerrainResponse};
use capitol_core::protocol::tutorial::stops;
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

/// How far in from the screen's edges a lead's place must be to count as
/// on screen, in points (#442). Nearer the edge, the card's arrow points.
const LEAD_MARGIN: f32 = 32.0;

/// How far you may walk from the workbench's clear spot before it's looked
/// for again near you (#443), server units: well past the 75 a station goes
/// down from, so it holds while you walk over to it.
const SPOT_KEEP: f64 = 120.0;

/// How long, with no clear spot near, before it's looked for again (#443).
const SPOT_AGAIN: std::time::Duration = std::time::Duration::from_millis(500);

/// `--deeds-sample` (#448): the Deeds Office offline, on the zone card's
/// sample facts, a sample register and a deed.
struct DeedsSample {
    facts: Vec<capitol_core::protocol::rest::ZoneFacts>,
    lots: capitol_core::protocol::rest::FreeLotsResponse,
    grant: capitol_core::protocol::tutorial::Grant,
}

/// `--board-sample` (S2.3): the notice board offline, on sample
/// commissions, a pack, you, and the server's clock, with the Gazette's
/// sample edition on its other page.
struct BoardSample {
    board: capitol_core::commissions::Board,
    pack: Option<capitol_core::state::Container>,
    me: uuid::Uuid,
    now: capitol_core::chrono::DateTime<capitol_core::chrono::Utc>,
    catalog: capitol_core::catalog::Catalog,
    reader: capitol_core::gazette::Reader,
    /// The Work page's offers (S2.7)
    wages: capitol_core::wages::Wages,
}


/// `--citizen-sample stroll` (#346), once the camera settled: where it
/// looked, where it stood, the street there (#349) and when.
#[derive(Clone, Copy)]
struct Stroll {
    target: [f32; 2],
    camera: [f32; 2],
    street: Option<crate::entities::Street>,
    from: f64,
}

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
    /// `--building-sample` (V3.4): offline, the lots round where the camera
    /// first looks, claimed and worked in, once it's there.
    building_sample: Option<Option<crate::building_sample::Buildings>>,
    /// `--node-sample`: offline, every kind of node, laid out round where
    /// the camera first looks once it's there.
    node_sample: bool,
    node_sampled: Option<nodes::sample::Sample>,
    /// `--offline-avatar` (controls T1.3): offline, you on the ground,
    /// walked by a loopback server, once the camera has settled.
    offline_avatar: Option<crate::offline_avatar::OfflineAvatar>,
    /// `--citizen-sample` (C0.2): offline, people and citizens in a row
    /// where the camera looks, stood there once it's settled.
    citizen_sample: Option<Option<(capitol_core::state::GameState, Vec<capitol_core::motion::Pose>)>>,
    /// `--citizen-sample stroll` (#346): walk them, from where the camera
    /// looked, where it stood, the street there (#349) and when, once it
    /// settled
    citizen_strolls: bool,
    citizen_stroll: Option<Stroll>,
    /// `--talk-sample` (S1.2): the citizen sample's row talks.
    talk_sample: Option<crate::entities::TalkSample>,
    /// The grass fields over the rural ring and the wilderness (P8.6c).
    grass: world::grass::Grass,
    /// The wilds' height field, sampled off the main thread (W9), its
    /// meshes, and whether it's in (the flat disc is open over the wilds
    /// once it is).
    land: world::land::Builder,
    land_meshes: world::land_mesh::LandMeshes,
    land_in: bool,
    plot_panel: ui::plot::PlotPanel,
    /// The zone card (#444), and `--zone-card-sample`'s facts offline
    zone_card: ui::zone_card::ZoneCard,
    zone_card_sample: Option<Vec<capitol_core::protocol::rest::ZoneFacts>>,
    /// The Deeds Office's panel (#448), the lot it showed you (ringed on the
    /// ground until you claim it or stop), and that lot's header chip this
    /// frame while the road in's card isn't pointing to it
    deeds: ui::deeds::DeedsOffice,
    shown_lot: Option<ui::deeds::Shown>,
    shown_chip: Option<ui::hud::ShownLot>,
    /// `--deeds-sample`: the facts, a register and the deed, offline
    deeds_sample: Option<DeedsSample>,
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
    /// `--board-sample`: the board's Wanted page (S2.3) on sample
    /// commissions and a pack, offline.
    board_sample: Option<BoardSample>,
    /// A person's card (S0.3), and `--card-sample`: a card, and today's
    /// game day, to show offline.
    card: ui::card::CardPanel,
    card_sample: Option<(capitol_core::card::Cards, i64, capitol_core::catalog::Catalog)>,
    /// Letters (S1.5), and `--mail-sample`: sample boxes to show offline.
    mail: ui::mail::MailPanel,
    mail_sample: Option<(capitol_core::mail::Mail, ui::mail::View)>,
    /// Guilds (S4.4), and `--guild-sample`: a sample to show offline.
    guild: ui::guild::GuildPanel,
    guild_sample: Option<(capitol_core::guilds::Guilds, ui::guild::View)>,
    /// What a foreclosure kept for you (#141), and `--impound-sample`: an
    /// impound and a pack to show offline.
    impound: ui::impound::ImpoundPanel,
    impound_sample: Option<(capitol_core::state::Container, capitol_core::state::Container)>,
    /// `--station-panel-sample`: a workbench of yours open, offline (#235).
    station_panel_sample: Option<capitol_core::state::GameState>,
    /// Its offers, for the Hire tab and a shift (S2.7)
    station_panel_wages: Option<capitol_core::wages::Wages>,
    /// Its tab, set once the panel has opened (opening picks its own).
    station_panel_tab: Option<ui::station::Tab>,
    /// The Exchange hall (E3.7), and `--hall-sample`: offline, goods waiting
    /// and you where the camera looks.
    hall: world::hall::HallLayer,
    hall_sample: bool,
    /// Where the hall is on screen, as the header's hint points (radians,
    /// 0 to the right, clockwise), while goods wait and you're not there.
    hall_hint: Option<f32>,
    /// Where the road in's card leads (#442): how far, and which way while
    /// it's off screen. Found in `pre_update`, said on the card.
    lead: Option<ui::hints::Lead>,
    /// The workbench's clear spot (#443), server units, and when it was
    /// last looked for: kept while it stays clear and near, so the ring
    /// doesn't follow you (`keep_clear_spot`). `None` until the step asks.
    clear_spot: Option<(Option<[f64; 2]>, Instant)>,
    /// The bazaar (E3.11): its building, the open stall's shop panel, and
    /// `--bazaar-sample`'s shops offline.
    bazaar: world::bazaar::BazaarLayer,
    shop: ui::shop::ShopPanel,
    bazaar_sample: Option<ui::shop::Sample>,
    station: ui::station::StationPanel,
    /// Enter (the `craft` action) was pressed since `ui` last looked.
    craft_key: bool,
    /// The chat panel (Commons S1.3), and `--chat-sample`: talk on both
    /// channels, with the crafting and inventory panels open, offline.
    chat: ui::chat::ChatPanel,
    chat_sample: Option<(capitol_core::talk::Log, capitol_core::chat::Chat, uuid::Uuid, capitol_core::state::GameState, ui::hints::Hints)>,
    /// Enter (the `chat` action) was pressed since `ui` last looked: chat,
    /// unless it's for crafting (`ui::chat::enter_means`).
    chat_key: bool,
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
    /// The road in's card (P8.3, #441), and the shortcut overlay.
    hints: ui::hints::Hints,
    /// `--hints-sample`: the card offline, on a sample tour in `hints`.
    hints_sample: bool,
    shortcuts: ui::shortcuts::Shortcuts,
    /// The dev overlay on F7 (D5.2), and `--dev-sample`: offline, open on
    /// sample numbers.
    dev: ui::dev_overlay::DevOverlay,
    dev_sample: bool,
    /// The admin's world layer on F6 (D5.3).
    layer: crate::debug_layers::Layer,
    /// The player's settings, and the file they're kept in (P8.4).
    settings: SettingsFile,
    settings_panel: ui::settings::SettingsPanel,
    /// The graphics last applied to the renderer, or `None` before the
    /// first frame with one.
    graphics: Option<Graphics>,
    /// The control mode, when this run holds it apart from the settings
    /// file (T1.1): `--controls`, or Click for captures and offline runs,
    /// so every capture recipe stays reproducible. F8 and the panel change
    /// it for the run alone, and the file keeps the player's own.
    controls_pinned: Option<ControlMode>,
    /// The mode last applied, to tell a switch. `None` before the first frame.
    control_mode: Option<ControlMode>,
    /// Alt is held this frame: in Direct it frees the cursor (T1.4).
    alt_held: bool,
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
            building_sample: None,
            node_sample: false,
            offline_avatar: None,
            node_sampled: None,
            citizen_sample: None,
            citizen_strolls: false,
            citizen_stroll: None,
            talk_sample: None,
            grass: world::grass::Grass::default(),
            land: world::land::Builder::default(),
            land_meshes: world::land_mesh::LandMeshes::default(),
            land_in: false,
            plot_panel: ui::plot::PlotPanel::default(),
            zone_card: ui::zone_card::ZoneCard::default(),
            zone_card_sample: None,
            deeds: ui::deeds::DeedsOffice::default(),
            shown_lot: None,
            shown_chip: None,
            deeds_sample: None,
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
            board_sample: None,
            card: ui::card::CardPanel::default(),
            card_sample: None,
            mail: ui::mail::MailPanel::default(),
            mail_sample: None,
            guild: ui::guild::GuildPanel::default(),
            guild_sample: None,
            impound: ui::impound::ImpoundPanel::default(),
            impound_sample: None,
            station_panel_sample: None,
            station_panel_wages: None,
            station_panel_tab: None,
            hall: world::hall::HallLayer::default(),
            hall_sample: false,
            hall_hint: None,
            lead: None,
            clear_spot: None,
            bazaar: world::bazaar::BazaarLayer::default(),
            shop: ui::shop::ShopPanel::default(),
            bazaar_sample: None,
            station: ui::station::StationPanel::default(),
            craft_key: false,
            chat: ui::chat::ChatPanel::default(),
            chat_sample: None,
            chat_key: false,
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
            hints_sample: false,
            shortcuts: ui::shortcuts::Shortcuts::default(),
            dev: ui::dev_overlay::DevOverlay::default(),
            dev_sample: false,
            layer: Default::default(),
            settings: SettingsFile::load(crate::settings::Settings::PATH),
            settings_panel: ui::settings::SettingsPanel::default(),
            graphics: None,
            controls_pinned: None,
            control_mode: None,
            alt_held: false,
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

    /// Hold the control mode at `mode` for this run, apart from the
    /// settings file (`--controls`, and Click for captures and offline).
    pub fn with_controls(mut self, mode: Option<ControlMode>) -> Self {
        self.controls_pinned = mode;
        self
    }

    /// The control mode in play: the pinned one, or the settings'.
    fn control_mode(&self) -> ControlMode {
        self.controls_pinned
            .unwrap_or(self.settings.settings.controls.mode)
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

    /// Offline, the lots round where the camera looks claimed, with floors,
    /// scaffolding and stations indoors and out (`--building-sample`).
    pub fn with_building_sample(mut self, on: bool) -> Self {
        if self.session.is_none() && on {
            self.building_sample = Some(None);
        }
        self
    }

    /// Offline, scatter every kind of node round the camera and take some
    /// through a day's work (`--node-sample`).
    /// Offline, people and citizens side by side before the camera
    /// (`--citizen-sample`, C0.2), or with "stroll" walking up and down the
    /// way it looks (#346).
    pub fn with_citizen_sample(mut self, mode: Option<&str>) -> Self {
        if let (Some(mode), None) = (mode, &self.session) {
            self.citizen_sample = Some(None);
            self.citizen_strolls = mode == "stroll";
        }
        self
    }

    /// The citizen sample's row talks (`--talk-sample`, Commons S1.2).
    pub fn with_talk_sample(mut self, on: bool) -> Self {
        if on && self.citizen_sample.is_some() {
            self.talk_sample = Some(Default::default());
        }
        self
    }

    pub fn with_node_sample(mut self, on: bool) -> Self {
        self.node_sample = on && self.session.is_none();
        self
    }

    /// Offline, stand you where the camera looks and walk you by a
    /// loopback server that answers in `round_trip_ms` (`--offline-avatar`).
    pub fn with_offline_avatar(mut self, round_trip_ms: Option<u64>) -> Self {
        if self.session.is_none() {
            self.offline_avatar = round_trip_ms.map(crate::offline_avatar::OfflineAvatar::new);
        }
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

    /// Offline, open a person's card on a sample (`--card-sample`): a
    /// citizen, a person, or your own.
    pub fn with_card_sample(mut self, which: Option<&str>) -> Self {
        if let (Some(which), None) = (which, &self.session) {
            self.card_sample = Some(ui::card::sample(which));
        }
        self
    }

    /// Offline, open the mailbox on sample letters (`--mail-sample`): the
    /// boxes, a letter, or one being written.
    pub fn with_mail_sample(mut self, which: Option<&str>) -> Self {
        if let (Some(which), None) = (which, &self.session) {
            self.mail_sample = Some(ui::mail::sample(which));
            self.mail.open = true;
        }
        self
    }

    /// Offline, open the guild panel (S4.4) on a sample (`--guild-sample`):
    /// your guild as its leader or an officer, the registry, or the form.
    pub fn with_guild_sample(mut self, which: Option<&str>) -> Self {
        if let (Some(which), None) = (which, &self.session) {
            let (guilds, view, page, draft) = ui::guild::sample(which);
            self.guild.open_on(page);
            self.guild.draft = draft;
            self.guild_sample = Some((guilds, view));
        }
        self
    }

    /// Offline, open the zone card (#444) on sample facts: one zone by its
    /// id, or "compare" for all three (`--zone-card-sample`).
    pub fn with_zone_card_sample(mut self, which: Option<&str>) -> Self {
        if let (Some(which), None) = (which, &self.session) {
            self.zone_card_sample = Some(ui::zone_card::sample());
            match which {
                "compare" => self.zone_card.open_compare(),
                zone => self.zone_card.open_one(zone),
            }
        }
        self
    }

    /// Offline, open the Deeds Office (#448) on sample facts and a sample
    /// register (`--deeds-sample`): `compare` (the default), `lots` (the
    /// farms, the deed open), `shown` (the first farm shown on the ground),
    /// `unearned`, `spent` or `confirm` (the claim asked once).
    pub fn with_deeds_sample(mut self, which: Option<&str>) -> Self {
        use capitol_core::protocol::tutorial::Grant;
        if let (Some(which), None) = (which, &self.session) {
            let grant = match which {
                "unearned" => Grant::Unearned,
                "spent" => Grant::Spent,
                _ => Grant::Open,
            };
            let lots = ui::deeds::sample_lots(rules::Zone::Rural);
            // Shown, you've left the office to walk out and look: its chip
            match which {
                "shown" => {}
                "lots" | "spent" | "confirm" => self.deeds.open_on(rules::Zone::Rural),
                _ => self.deeds.open(),
            }
            if which == "confirm" {
                self.deeds.ask_to_claim(lots.lots[0].plot_id);
            }
            if which == "shown" {
                let first = &lots.lots[0];
                self.shown_lot = Some(ui::deeds::Shown {
                    terrain_id: first.terrain_id.clone(),
                    plot_id: first.plot_id,
                    zone: rules::Zone::Rural,
                    x: first.x,
                    y: first.y,
                });
            }
            self.deeds_sample = Some(DeedsSample { facts: ui::zone_card::sample(), lots, grant });
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

    /// Offline, open the notice board's Wanted page (S2.3) on sample
    /// commissions (`--board-sample`): "list" (the default), "post" or
    /// "deliver" for a form, "work" for the Work page's wage offers (S2.7),
    /// or "glint" to leave it closed, so the board in the forecourt shows
    /// that your pack could fill some.
    pub fn with_board_sample(mut self, which: Option<&str>) -> Self {
        if let (Some(which), None) = (which, &self.session) {
            let catalog = capitol_core::catalog::Catalog::fallback();
            let (board, pack, me, now) = ui::board::sample(which, &catalog);
            let wages = ui::work::sample(me, now);
            self.board_sample = Some(BoardSample { board, pack, me, now, catalog, reader: ui::gazette::sample(), wages });
            match which {
                "glint" => {}
                "work" => self.gazette.open_work(),
                _ => self.gazette.open_board(false),
            }
        }
        self
    }

    /// Offline, open the chat on a sample of both channels, with the
    /// crafting and inventory panels open and the hint card up
    /// (`--chat-sample`, S1.3).
    pub fn with_chat_sample(mut self, on: bool) -> Self {
        if on && self.session.is_none() {
            let state = ui::station::sample();
            // A fresh tour, so the first card shows
            let mut hints = ui::hints::Hints::default();
            hints.follow(Some(ui::hints::sample("new")));
            let (log, chat, me) = ui::chat::sample(Instant::now(), 0.0);
            self.chat_sample = Some((log, chat, me, state, hints));
            self.chat.open_typing();
            self.crafting.open = true;
            self.inventory.open = true;
        }
        self
    }

    /// Offline, the road in's card on a sample tour (`--hints-sample`,
    /// #441): `new`, `half` way up the road, or `hidden`, which shows the
    /// header's chip instead, or `help`, hidden with the `?` overlay open
    /// on its line. A step's or a chapter's name stands the tour there, and
    /// `<chapter>-done` shows that chapter's close (#442, `hints::sample`).
    /// With `--chat-sample`, it's the card in front of that sample's panels.
    pub fn with_hints_sample(mut self, which: Option<&str>) -> Self {
        if let Some(which) = which.filter(|_| self.session.is_none()) {
            if let Some((.., hints)) = &mut self.chat_sample {
                hints.with_sample(which);
                return self;
            }
            self.hints.with_sample(which);
            self.hints_sample = true;
            self.shortcuts.open = which == "help";
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
    /// (`craft` or `automation`; `empty`, craft with the box empty, or
    /// `chest`, an empty chest of yours, for the recycling bin, #488;
    /// `hire`, the Hire tab with an offer of yours worked, or `shift`, on a
    /// shift at Ada's workbench, S2.7),
    /// with the inventory and crafting panels open beside it
    /// (`--station-panel-sample`, #235).
    pub fn with_station_panel_sample(mut self, tab: Option<&str>) -> Self {
        if let (Some(tab), None) = (tab, &self.session) {
            let mut state = ui::station::sample();
            match tab {
                "empty" => ui::station::empty_the_box(&mut state),
                "chest" => ui::station::chest_of_yours(&mut state),
                "shift" => ui::station::at_adas_bench(&mut state),
                _ => {}
            }
            self.station_panel_sample = Some(state);
            self.station_panel_wages = Some(ui::station::sample_wages(tab));
            self.station_panel_tab = Some(match tab {
                "automation" => ui::station::Tab::Automation,
                "hire" => ui::station::Tab::Hire,
                _ => ui::station::Tab::Craft,
            });
            self.inventory.open = true;
        }
        self
    }

    /// Offline, show the hall as if goods waited (`--hall-sample`).
    /// Offline, sample shops in the bazaar's stalls and the shop panel open
    /// on stall `stall` (a number, or "mine"), as if you stood at it
    /// (`--bazaar-sample`). "street" keeps most of the stalls, stands you at
    /// none and opens nothing, for filming (#345).
    pub fn with_bazaar_sample(mut self, stall: Option<&str>) -> Self {
        if let (Some("street"), None) = (stall, &self.session) {
            self.bazaar_sample = Some(ui::shop::Sample::street());
            return self;
        }
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

    pub fn with_dev_sample(mut self, on: bool) -> Self {
        self.dev_sample = on;
        self.dev.open |= on;
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
            // The valleys' mist (W11), Flint's ground fog on the ground
            // (#297), as thick as the hour says (`Atmosphere`).
            if let Err(e) = gpu.set_ground_fog(&world::mist::build(&field).ground_fog()) {
                tracing::warn!("mist: can't lay it: {e}");
            }
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
        // The Wanted page (S2.3): commissions pinned there your pack could fill
        let fill = match (&self.session, &self.board_sample) {
            (Some(session), _) => {
                let state = session.state();
                session.board().can_fill(state.inventory(), state.me()).len()
            }
            (None, Some(s)) => s.board.can_fill(s.pack.as_ref(), Some(s.me)).len(),
            _ => 0,
        };
        let (office, office_top) = site.office();
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
            fill,
            reading: self.gazette.open,
            office,
            office_top,
            deed_waits: self.grant().is_open(),
            office_open: self.deeds.is_open(),
        })
    }

    /// Where the deed the road in earns stands (#448): the server's, or a
    /// sample's offline.
    fn grant(&self) -> capitol_core::protocol::tutorial::Grant {
        match (&self.session, &self.deeds_sample) {
            (Some(session), _) => session.state().tutorial().map(|t| t.grant).unwrap_or_default(),
            (None, Some(sample)) => sample.grant,
            // `--hints-sample`'s tour, earned as the server would have it
            (None, None) => self.hints.progress().map(|t| t.grant).unwrap_or_default(),
        }
    }

    /// Look in at the Deeds Office (#448): its panel on the zones side by
    /// side, with the facts asked for afresh. Opening it is the road in's
    /// office step, and the comparison is the zone card's.
    fn open_office(&mut self) {
        self.deeds.open();
        if let Some(session) = &mut self.session {
            session.ask_zone_facts();
            for did in [Did::OpenedOffice, Did::ReadZoneCard] {
                if let Some(step) = self.hints.saw(did) {
                    session.tutorial_seen(step);
                }
            }
        }
    }

    /// The lot the Deeds Office showed you, as the road in's ring marks it:
    /// its edge traced, or a ring at its middle if it isn't drawn.
    fn shown_mark(&self) -> Option<script_api::LeadMark> {
        let shown = self.shown_lot.as_ref()?;
        Some(match self.lots.get(&shown.terrain_id).filter(|l| !l.outline.is_empty()) {
            Some(lot) => lot_mark(lot),
            None => script_api::LeadMark {
                at: crate::coords::to_world(shown.x, shown.y),
                radius: 2.5,
                outline: Vec::new(),
            },
        })
    }

    /// Where you stand for the road in's card (world metres): your avatar,
    /// or offline with `--hints-sample` or `--deeds-sample`, `camera_target`.
    fn lead_from(&self, camera_target: [f32; 2]) -> Option<[f32; 2]> {
        match &self.session {
            Some(session) => {
                let p = session.state().local_player()?;
                Some(crate::coords::to_world(p.x, p.y))
            }
            None if self.hints_sample || self.deeds_sample.is_some() => Some(camera_target),
            None => None,
        }
    }

    /// Finds the workbench's clear spot while the card asks for one (#443,
    /// `Target::ClearSpot`), and keeps it while it stays clear (no bench
    /// put down there since) and you stay near it: the ring stays put
    /// rather than following you. When there's none near, it looks again
    /// after [`SPOT_AGAIN`], not every frame.
    fn keep_clear_spot(&mut self, camera_target: [f32; 2], now: Instant) {
        if self.hints.target() != Some(Target::ClearSpot) {
            self.clear_spot = None;
            return;
        }
        let (Some(terrain), Some(you)) = (&self.built, self.lead_from(camera_target)) else {
            return;
        };
        let you = crate::coords::to_server(you[0], you[1]);
        let none = HashMap::new();
        let stations = self.session.as_ref().map_or(&none, |s| s.state().stations()).values();
        let kept = match self.clear_spot {
            Some((Some(at), _)) => {
                (at[0] - you[0]).hypot(at[1] - you[1]) <= SPOT_KEEP
                    && onboarding::Ground::near(at, 0.0, terrain, stations.clone()).clear(at)
            }
            Some((None, looked)) => now.saturating_duration_since(looked) < SPOT_AGAIN,
            None => false,
        };
        if !kept {
            self.clear_spot = Some((onboarding::clear_spot(you, terrain, stations), now));
        }
    }

    /// Where the road in's card leads, if it names a place this client can
    /// find (#442), and where you stand, both world metres. A node is the
    /// nearest live one of its kinds, a lot its zone's stop on the road in
    /// ([`Self::road_stop`]), a stall the counter of the nearest someone
    /// keeps (or the nearest), the board the Gazette's notice board, the
    /// office the paving before its desk. Where the card leads nowhere of
    /// its own, it's the lot the Deeds Office showed you (#448), if any.
    /// Offline with `--hints-sample` or `--deeds-sample`, you stand at
    /// `camera_target`. Call after the hall's and the bazaar's marks are
    /// set this frame.
    fn lead_mark(&self, camera_target: [f32; 2]) -> Option<(script_api::LeadMark, [f32; 2])> {
        let you = self.lead_from(camera_target)?;
        // The lot the Deeds Office showed you (#448), wherever the card
        // leads nowhere of its own: the claim's step, or no card at all
        let target = match self.hints.target() {
            Some(target) => target,
            None => return self.shown_mark().map(|mark| (mark, you)),
        };
        let from = crate::coords::to_server(you[0], you[1]);
        let ring = |at, radius| script_api::LeadMark {
            at,
            radius,
            outline: Vec::new(),
        };
        let mark = match target {
            Target::Point([x, y]) => ring(crate::coords::to_world(x, y), 1.2),
            // As wide as the bench's own ring, so it reads as its footprint
            Target::ClearSpot => {
                let [x, y] = self.clear_spot.and_then(|(spot, _)| spot)?;
                ring(crate::coords::to_world(x, y), crate::stations::ring_radius("workbench"))
            }
            Target::Node(kinds) => {
                let nodes = match (&self.session, &self.node_sampled) {
                    (Some(session), _) => session.state().nodes(),
                    (None, Some(sample)) => sample.nodes(),
                    (None, None) => return None,
                };
                let node = onboarding::nearest_node(nodes.values(), kinds, from)?;
                let id = node.id.to_string();
                // Its ring as the node's own are drawn, if it's shown
                let shown = self
                    .shared
                    .nodes
                    .lock()
                    .ok()
                    .and_then(|n| n.iter().find(|m| m.id == id).map(|m| (m.at, m.r)));
                let (at, r) = shown.unwrap_or((crate::coords::to_world(node.x, node.y), 1.5));
                ring(at, r + 0.7)
            }
            Target::Lot(zone) => {
                // The road in's stop (#445): its drawn edge, or its quad if
                // it's too thin to draw
                let stop = self.road_stop(zone)?;
                let outline: Vec<[f64; 2]> = match self.lots.get(&stop.lot.id) {
                    Some(l) if !l.outline.is_empty() => l.outline.clone(),
                    _ => stop.lot.corners.iter().map(|c| [c.0, c.1]).collect(),
                };
                let n = outline.len() as f64;
                let [x, y] = outline
                    .iter()
                    .fold([0.0, 0.0], |m, p| [m[0] + p[0] / n, m[1] + p[1] / n]);
                script_api::LeadMark {
                    at: crate::coords::to_world(x, y),
                    radius: 2.5,
                    outline: outline
                        .iter()
                        .map(|p| crate::coords::to_world(p[0], p[1]))
                        .collect(),
                }
            }
            Target::Hall => {
                let hall = self.shared.hall.lock().ok()?.clone()?;
                ring(hall.door, 2.5)
            }
            Target::Stall => {
                let bazaar = self.shared.bazaar.lock().ok()?.clone()?;
                // Out on the lane before its counter, where its sign faces
                let counter = |s: &script_api::StallMark| {
                    [
                        s.sign[0] + s.sign_facing[0] * 1.2,
                        s.sign[2] + s.sign_facing[2] * 1.2,
                    ]
                };
                let near = |p: [f32; 2]| (p[0] - you[0]).powi(2) + (p[1] - you[1]).powi(2);
                // A shop someone else keeps, if there is one (#446); an
                // empty counter counts too, so an empty bazaar never blocks
                let bare = |s: &script_api::StallMark| s.name.is_none() || s.mine;
                let stall = bazaar.stalls.iter().min_by(|a, b| {
                    bare(a)
                        .cmp(&bare(b))
                        .then(near(counter(a)).total_cmp(&near(counter(b))))
                })?;
                ring(counter(stall), 1.4)
            }
            Target::Board => {
                // The Gazette's notice board, under its prompt (#446)
                let hall = self.shared.hall.lock().ok()?.clone()?;
                ring([hall.board_top[0], hall.board_top[2]], 1.6)
            }
            Target::Person => {
                let state = self.session.as_ref()?.state();
                let person = onboarding::nearest_person(state.players().values(), state.me(), from)?;
                // At their feet as they're drawn, so the ring walks with them
                let drawn = self
                    .shared
                    .players
                    .lock()
                    .ok()
                    .and_then(|p| p.iter().find(|l| l.id == person.id).map(|l| [l.at[0], l.at[2]]));
                ring(drawn.unwrap_or(crate::coords::to_world(person.x, person.y)), 0.9)
            }
            // Out on the paving before the desk, where you'd be served
            Target::Office => ring(self.hall.site()?.office_front(), 1.1),
            // Your own land (#449), its edge traced as the stops' are
            Target::Home => self.home_mark()?,
            // A station of yours that can be set to work, at home first,
            // ringed as wide as its own ring
            Target::MyStation => {
                let state = self.session.as_ref()?.state();
                let home = onboarding::home_plot(state.plots());
                let station = onboarding::my_station(state.stations().values(), state.me(), home, from)?;
                ring(
                    crate::coords::to_world(station.x, station.y),
                    crate::stations::ring_radius(&station.station_type),
                )
            }
        };
        Some((mark, you))
    }

    /// Your home lot as the road in's ring marks it (#449): its drawn edge,
    /// else its claimed quad. Offline with `--hints-sample`, the farm the
    /// road in stops at stands in for it.
    fn home_mark(&self) -> Option<script_api::LeadMark> {
        let Some(session) = &self.session else {
            let stop = self.hints_sample.then(|| self.road_stop(rules::Zone::Rural)).flatten()?;
            return self.lots.get(&stop.lot.id).filter(|l| !l.outline.is_empty()).map(lot_mark);
        };
        let plot = onboarding::home_plot(session.state().plots())?;
        let lot = plot.terrain_id.as_ref().and_then(|id| self.lots.get(id));
        let outline = plot_outline(plot, lot);
        let n = outline.len().max(1) as f32;
        let at = outline.iter().fold([0.0, 0.0], |m, p| [m[0] + p[0] / n, m[1] + p[1] / n]);
        Some(script_api::LeadMark { at, radius: 2.5, outline })
    }

    /// The road in's stop in `zone` (#445): the lot
    /// `protocol::tutorial::stops` picks by the south arterial, from the
    /// terrain the world was built from and everyone's claims. Offline,
    /// `--claim-sample`'s.
    fn road_stop(&self, zone: rules::Zone) -> Option<stops::Stop<'_>> {
        let terrain = self.built.as_ref()?;
        let taken = |id: &str| match (&self.session, &self.claim_sample) {
            (Some(session), _) => session.state().claimed_lots().contains_key(id),
            (None, Some(sample)) => sample.claimed.iter().any(|c| c == id),
            (None, None) => false,
        };
        stops::stop(&terrain.terrain_plots, &rules::Frame::TABLE, zone, taken)
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
                (!sample.street).then(|| crate::coords::to_server(camera_target[0], camera_target[1])),
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
        self.seen(Did::UsedInventory);
    }

    fn toggle_crafting(&mut self) {
        self.crafting.toggle();
        if self.crafting.open {
            self.seen(Did::OpenedCrafting);
        }
    }

    /// The screen showed `did`: tell the server, if it's worth telling
    /// (`tutorial_seen`, #441).
    fn seen(&mut self, did: Did) {
        if let (Some(step), Some(session)) = (self.hints.saw(did), &mut self.session) {
            session.tutorial_seen(step);
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
    /// camera's feel and the controls to `cap_setting`, and the graphics to
    /// the renderer whenever they change. Then write the file once they
    /// settle.
    fn apply_settings(&mut self, ctx: &mut ExtensionCtx, now: Instant) {
        let mode = self.control_mode();
        let was = self.control_mode.replace(mode);
        if was.is_some_and(|was| was != mode) {
            self.switch_controls(ctx, mode, now);
        }
        if was != Some(mode) && mode == ControlMode::Direct {
            self.enter_direct();
        }
        let settings = &self.settings.settings;
        self.sound.set_volumes(settings.audio);
        if let Ok(mut feel) = self.shared.camera_feel.lock() {
            *feel = settings.camera;
        }
        if let Ok(mut controls) = self.shared.controls.lock() {
            *controls = Controls {
                mode,
                ..settings.controls
            };
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

    /// The control mode just switched (T1.1, F8 or the panel). Any walk
    /// stops where you are, so neither mode inherits the other's: a click
    /// walk's ring goes, a steer stops as a steer does (T1.3), and
    /// camera.rhai drops its pan when it sees Click back. A toast names
    /// the new mode.
    fn switch_controls(&mut self, ctx: &mut ExtensionCtx, mode: ControlMode, now: Instant) {
        tracing::info!("controls: {}", mode.name());
        // Esc's letting go is Direct's alone (T1.4)
        self.shared.cursor_let_go.store(false, Ordering::Relaxed);
        if let Some(session) = &mut self.session {
            session.stop_walk(now);
        }
        let body = match mode {
            ControlMode::Click => "Click the ground to walk there. F8 to switch",
            ControlMode::Direct => "WASD walks the way you face and the mouse looks; Alt or Esc frees the cursor. F8 to switch",
        };
        let mut data = toml::Table::new();
        data.insert("title".into(), mode.label().into());
        data.insert("body".into(), body.into());
        ctx.fire_event("controls_note", data);
    }

    /// Into Direct, the first frame too (T1.4): the inventory and crafting
    /// panels, open from the start in Click, close, or an open panel would
    /// keep the cursor free from the first frame. I and C bring them back.
    fn enter_direct(&mut self) {
        self.inventory.open = false;
        self.crafting.open = false;
    }

    /// A panel is up that takes clicks (T1.4): those that keep a rect are
    /// read by it, as drawn this frame.
    fn panel_open(&self) -> bool {
        let rects = [
            self.inventory.rect,
            self.crafting.rect,
            self.exchange.rect,
            self.market.rect,
            self.impound.rect,
            self.shop.rect,
            self.station.rect,
            self.card.rect,
            self.plot_panel.rect,
            self.trade.rect,
        ];
        rects.iter().any(Option::is_some)
            || self.chat.open
            || self.mail.open
            || self.gazette.open
            || self.guild.open
            || self.settings_panel.open
            || self.shortcuts.open
            || self.deeds.is_open()
            || self.zone_card.is_open()
    }

    /// Whether something wants the cursor free (T1.4), for `Shared`: a
    /// panel, the login, the "Playing on another device" banner, a station
    /// being placed, Alt, or Escape having let it go. A panel open clears
    /// Escape's, so closing it captures the cursor again.
    ///
    /// The road in's card doesn't count: it's up for the whole tour, and
    /// Direct would never capture. Alt frees the cursor for its buttons.
    fn wants_cursor(&self) -> bool {
        let let_go = &self.shared.cursor_let_go;
        let panel = self.panel_open();
        if panel {
            let_go.store(false, Ordering::Relaxed);
        }
        let session = self.session.as_ref();
        let elsewhere = session.is_some_and(|s| s.state().presence().is_some_and(|p| !p.yours));
        let placing = session.is_some_and(|s| s.placing().is_some());
        panel || self.at_login() || elsewhere || placing || self.alt_held || let_go.load(Ordering::Relaxed)
    }

    /// Esc over the shoulder (T1.4): closes the panel on top, by a fixed
    /// order from the ones that open over others down to the inventory.
    /// The trade window keeps its own buttons, so Esc never walks out of a
    /// deal. Returns whether it closed one.
    fn close_top_panel(&mut self) -> bool {
        if let Some(session) = &mut self.session {
            if session.cards().is_open() {
                session.close_card();
                return true;
            }
        }
        if self.zone_card.is_open() {
            self.zone_card.close();
            return true;
        }
        if self.deeds.is_open() {
            self.deeds.close();
            return true;
        }
        for open in [
            &mut self.settings_panel.open,
            &mut self.mail.open,
            &mut self.guild.open,
            &mut self.gazette.open,
        ] {
            if *open {
                *open = false;
                return true;
            }
        }
        if self.shop.rect.is_some() {
            self.shop.close();
            return true;
        }
        for open in [&mut self.market.open, &mut self.exchange.open, &mut self.impound.open] {
            if *open {
                *open = false;
                return true;
            }
        }
        if let Some(session) = &mut self.session {
            if session.state().open_station().is_some() {
                session.close_station();
                return true;
            }
            if session.state().picked_plot().is_some() {
                session.close_plot();
                return true;
            }
        }
        for open in [&mut self.crafting.open, &mut self.inventory.open] {
            if *open {
                *open = false;
                return true;
            }
        }
        false
    }
}

impl PlayerExtension for CapitolExtension {
    fn register_script_api(&mut self, engine: &mut rhai::Engine) {
        script_api::register(engine, &self.shared);
    }

    fn pre_update(&mut self, ctx: &mut ExtensionCtx) {
        let now = Instant::now();
        self.dev.record(ctx.clock.delta_time, ctx.world.entity_count());
        if ctx.input.is_action_just_pressed("dev_overlay") {
            self.dev.toggle();
        }
        // The world layers (D5.3) are an admin's: a person's F6 says so
        if !self.at_login() && ctx.input.is_action_just_pressed("debug_layer") {
            let admin = self.session.as_ref().is_some_and(|s| s.admin().is_some());
            let mut data = toml::Table::new();
            if admin {
                self.layer = self.layer.next();
                data.insert("title".into(), "World layer".into());
                data.insert("body".into(), format!("{}. F6 for the next", ui::slots::capitalised(self.layer.name())).into());
            } else {
                data.insert("title".into(), "World layers".into());
                data.insert("body".into(), "Only for an admin's account".into());
            }
            ctx.fire_event("debug_note", data);
        }
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
        // A click on the notice board: Wanted, or the Gazette while there's news
        if self.shared.read_gazette.swap(false, Ordering::Relaxed) {
            let news = self.session.as_ref().is_some_and(|s| s.gazette().unread());
            self.gazette.open_board(news);
        }
        // The Deeds Office (#448): a click on its desk, from anywhere (D8)
        if self.shared.open_office.swap(false, Ordering::Relaxed) {
            self.open_office();
        }
        // Letters (S1.5): L
        if !self.at_login() && ctx.input.is_action_just_pressed("mail") {
            self.mail.toggle();
        }
        // Guilds (S4.4): U
        if !self.at_login() && ctx.input.is_action_just_pressed("guild") {
            self.guild.toggle();
        }
        if ctx.input.is_action_just_pressed("settings") {
            self.settings_panel.toggle();
        }
        // The control mode (T1.1): read here, so input scripts can press it.
        if !self.at_login() && ctx.input.is_action_just_pressed("control_mode") {
            match &mut self.controls_pinned {
                Some(mode) => *mode = mode.other(),
                None => {
                    let c = &mut self.settings.settings.controls;
                    c.mode = c.mode.other();
                }
            }
        }
        self.apply_settings(ctx, now);
        self.alt_held = ctx.input.is_action_pressed("with_alt");
        // Taken in `ui`, where the panel knows its recipe.
        self.craft_key |= !self.at_login() && ctx.input.is_action_just_pressed("craft");
        self.chat_key |= !self.at_login() && ctx.input.is_action_just_pressed("chat");
        let station_key = !self.at_login() && ctx.input.is_action_just_pressed("station");
        let trade_key = !self.at_login() && ctx.input.is_action_just_pressed("trade");
        // The movement keys walk you in Direct (T1.3); in Click they pan
        // the view (camera.rhai), and the steer sees none.
        let way = match self.control_mode() {
            ControlMode::Direct if !self.at_login() => keys_way(ctx, &self.shared),
            _ => (0.0, 0.0),
        };
        if let Some(session) = &mut self.session {
            session.update(now, &mut self.terrain);
            session.steer(now, way);
            if station_key {
                session.open_nearest_station(now);
            }
            if trade_key {
                session.trade_key(now);
            }
            walk(ctx, session, &self.shared, now);
            gather(ctx, session, &self.shared, &mut self.sound, now);
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
            crafted(ctx, session, &self.shared, &mut self.sound);
            // The road in's card follows the server's tour (#441), which
            // judges the walking, gathering and making itself
            self.hints.follow(session.state().tutorial());
            // The bid step opens the market on an item to bid for (#494)
            if self.market.open {
                if let Some(item) = self.hints.bid_lead() {
                    self.market.lead_bid(item);
                }
            }
            // The market box is waived with words when nothing's in it (#446, D10)
            let goods = session.state().market_box().map(|b| capitol_core::hall::goods_waiting(Some(b)));
            self.hints.market_box(goods, now);
            if let Some(step) = self.hints.waive(now) {
                session.tutorial_seen(step);
            }
            // The quality card's numbers, from your own pack (#443)
            let state = session.state();
            let pack = [state.inventory(), state.crafting_input(), state.crafting_output()];
            let held = pack.into_iter().flatten().flat_map(|c| c.slots.iter().flatten());
            self.hints.set_lesson(onboarding::QualityLesson::of(held, state.recipe("rope_basic")));
            // Settle in's words, from your own lot (#449)
            self.hints.set_home(onboarding::home_plot(state.plots()));
            if plots(ctx, session, &self.shared, &self.lots, &mut self.sound) {
                // The deed is spent (#448): the office steps aside for the
                // gold rings, and the lot it showed is yours now or let go
                self.deeds.close();
                self.shown_lot = None;
            }
            // Signed out, the store is empty, so everyone fades away.
            let (poses, state) = session.sample(now);
            let dt = ctx.clock.delta_time.min(0.1) as f32;
            self.players.update(ctx, poses, state, dt, &self.shared);
            let poses = (self.layer != crate::debug_layers::Layer::Off).then(|| poses.to_vec());
            debug_layers(session, self.layer, poses.as_deref(), self.dev.open, &self.shared);
            speak(&self.shared, |id| session.bubble(id, now));
            murmurs(&mut self.sound, &session.take_heard());
            trade(ctx, session, &self.shared, now);
            open_card(session, &self.shared);
            for note in session.take_market_notes() {
                let mut data = toml::Table::new();
                data.insert("title".into(), note.title.into());
                data.insert("body".into(), note.body.into());
                ctx.fire_event("market_note", data);
            }
            for (title, body) in session.take_letter_notes() {
                let mut data = toml::Table::new();
                data.insert("title".into(), title.into());
                data.insert("body".into(), body.into());
                ctx.fire_event("letter_note", data);
            }
            for (title, body) in session.take_guild_notes() {
                let mut data = toml::Table::new();
                data.insert("title".into(), title.into());
                data.insert("body".into(), body.into());
                ctx.fire_event("guild_note", data);
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
        } else if let Some(sample) = &mut self.building_sample {
            // Claimed once the camera has settled on its shot.
            if sample.is_none() && ctx.clock.total_time > 0.5 && !self.lots.is_empty() {
                let t = ctx.camera.target;
                let focus = crate::coords::to_server(t.x, t.z);
                *sample = Some(crate::building_sample::Buildings::new((focus[0], focus[1]), &self.lots));
            }
            if let Some(b) = sample {
                let none = std::collections::HashSet::new();
                let dt = ctx.clock.delta_time.min(0.1) as f32;
                b.tick(dt);
                self.stations
                    .update(ctx, &b.state, None, &none, &[], dt, &self.shared);
            }
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
                let (t, c) = ([t.x, t.z], [c.x, c.z]);
                if self.citizen_strolls {
                    // On the street under where it looks, if there is one (#349)
                    let street = self.built.as_ref().and_then(|b| crate::entities::Street::at(&b.roads, t));
                    self.citizen_stroll = Some(Stroll { target: t, camera: c, street, from: ctx.clock.total_time });
                }
                *sample = Some(crate::entities::sample(t, c));
            }
            if let Some(s) = self.citizen_stroll {
                *sample = Some(crate::entities::stroll(s.target, s.camera, s.street, (ctx.clock.total_time - s.from) as f32));
            }
            if let Some((state, poses)) = sample {
                let dt = ctx.clock.delta_time.min(0.1) as f32;
                self.players.update(ctx, poses, state, dt, &self.shared);
                if let Some(talk) = &mut self.talk_sample {
                    let now = ctx.clock.total_time;
                    murmurs(&mut self.sound, &talk.tick(now, state, poses));
                    speak(&self.shared, |id| talk.log.bubble(id, now).map(script_api::Speech::of));
                }
            }
        }
        if let (None, Some(avatar)) = (&self.session, &mut self.offline_avatar) {
            // Stood once the camera has settled on its shot, on the ground
            // the offline terrain gives (T1.3).
            if !avatar.placed() && ctx.clock.total_time > 0.5 {
                if let Some(ground) = self.terrain.ground() {
                    let t = ctx.camera.target;
                    let [x, y] = crate::coords::to_server(t.x, t.z);
                    avatar.place(now, (x, y), ground.clone());
                }
            }
            let walk_to = self.shared.move_to.lock().ok().and_then(|mut m| m.take());
            if let Some((poses, state)) = avatar.update(now, way, walk_to.map(|[x, y]| (x, y))) {
                let dt = ctx.clock.delta_time.min(0.1) as f32;
                self.players.update(ctx, poses, state, dt, &self.shared);
            }
            if let Ok(mut d) = self.shared.destination.lock() {
                *d = avatar.destination().map(|(x, y)| crate::coords::to_world(x, y));
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
        // offline `--claim-sample`'s and `--building-sample`'s.
        let dt = ctx.clock.delta_time.min(0.1) as f32;
        let buildings = self.building_sample.as_ref().and_then(|b| b.as_ref());
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
        } else if self.claim_sample.is_some() || buildings.is_some() {
            if let Some(sample) = &mut self.claim_sample {
                let t = ctx.camera.target;
                sample.update(dt, crate::coords::to_server(t.x, t.z), &self.lots);
            }
            let sample = self.claim_sample.as_ref();
            let growing = sample.map(|s| &s.growing);
            let version = sample.map_or(0, |s| s.version) + ((buildings.is_some() as u64) << 32);
            let claimed = sample.into_iter().flat_map(|s| &s.claimed);
            self.houses.sync(
                version,
                claimed.chain(buildings.into_iter().flat_map(|b| &b.claimed)),
                |id| growing.is_some_and(|g| g.contains(id)),
                &self.lots,
            );
        }
        // Floors (V3.3): offline the sample's. Online they come with #475.
        for (id, floors, under_way) in buildings.into_iter().flat_map(|b| &b.floors) {
            self.houses.set_floors(id, *floors, *under_way, &self.lots);
        }
        // A chimney over every kiln and forge indoors, smoking from the top
        // (V3.4).
        let stations = match (&self.session, buildings) {
            (Some(session), _) => Some(session.state().stations()),
            (None, Some(b)) => Some(b.state.stations()),
            _ => None,
        };
        let mut fires: Vec<(uuid::Uuid, [f64; 2])> = stations
            .into_iter()
            .flatten()
            .filter(|(_, s)| s.indoors && matches!(crate::stations::kind_of(&s.station_type), "kiln" | "forge"))
            .map(|(id, s)| (*id, [s.x, s.y]))
            .collect();
        fires.sort_by_key(|(id, _)| *id);
        let at: Vec<[f64; 2]> = fires.iter().map(|(_, p)| *p).collect();
        self.houses.fires(&at, &self.lots);
        let chimneys = self
            .houses
            .chimneys()
            .filter_map(|(p, top)| fires.iter().find(|(_, q)| *q == p).map(|(id, _)| (*id, top)))
            .collect();
        self.stations.set_chimneys(chimneys);
        // The buildings the camera comes close to open (V3.4): yours from
        // further off.
        let mine: std::collections::HashSet<String> = match buildings {
            Some(b) => b.mine.clone(),
            None => self.owned_drawn.iter().flatten().cloned().collect(),
        };
        self.houses.cutaway(dt, ctx.camera.position.to_array(), &mine);
        self.houses.advance(dt, &self.lots);
        if let Some(gpu) = ctx.gpu.as_mut() {
            let uploaded = self.houses.upload(gpu, &self.lots);
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
            if self.bazaar.sync(gpu, terrain, shop) {
                attach_each(ctx, world::bazaar::meshes().iter());
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
        set_material(
            ctx.world,
            world::bazaar::LAMPS_MESH,
            &[("emissive_scale", world::bazaar::lantern_light(glow))],
        );
        let mark = self.bazaar_mark([camera.target.x, camera.target.z]);
        if let Ok(mut shared) = self.shared.bazaar.lock() {
            *shared = mark;
        }
        // Where the road in's card leads (#442): ringed on the ground by
        // `input.rhai` while it's on screen, pointed to from the card while
        // it isn't.
        self.keep_clear_spot([camera.target.x, camera.target.z], Instant::now());
        let found = self.lead_mark([camera.target.x, camera.target.z]);
        // The card speaks of its own step's place, and of the shown lot
        // only on the claim's step (#448): any other step's card would
        // send you there for the wrong reason
        let card_leads = self.hints.target().is_some()
            || self.hints.leading() == Some(capitol_core::protocol::tutorial::Step::ClaimHome);
        self.lead = found.as_ref().filter(|_| card_leads).map(|(mark, you)| {
            let [x, z] = mark.at;
            let metres = (x - you[0]).hypot(z - you[1]);
            let on_screen = self.shared.projector.as_ref().is_some_and(|p| {
                let view = p.view();
                let y = p.ground_height(x, z).unwrap_or(0.0);
                view.world_to_screen([x, y + 0.5, z]).is_some_and(|s| {
                    s.x > LEAD_MARGIN
                        && s.x < view.width - LEAD_MARGIN
                        && s.y > LEAD_MARGIN
                        && s.y < view.height - LEAD_MARGIN
                })
            });
            let off_screen = (!on_screen).then(|| {
                ui::hud::bearing(camera.position.to_array(), camera.target.to_array(), [x, 0.0, z])
            });
            // A stop someone owns teaches as well, and the card says so (#445)
            let taken = matches!(self.hints.target(), Some(Target::Lot(zone))
                if self.road_stop(zone).is_some_and(|s| s.taken));
            ui::hints::Lead { metres, off_screen, taken }
        });
        // The shown lot's chip (#448), unless the card's claim step is
        // pointing to it already
        self.shown_chip = self.shown_lot.as_ref().filter(|_| self.hints.leading() != Some(capitol_core::protocol::tutorial::Step::ClaimHome)).and_then(|shown| {
            let you = found.as_ref().map(|(_, you)| *you).or_else(|| {
                let p = self.session.as_ref()?.state().local_player()?;
                Some(crate::coords::to_world(p.x, p.y))
            })?;
            let [x, z] = crate::coords::to_world(shown.x, shown.y);
            let noun = capitol_core::deeds::home_noun(shown.zone);
            Some(ui::hud::ShownLot {
                what: format!("A {noun}"),
                far: Some((x - you[0]).hypot(z - you[1])).filter(|m| *m >= 6.0).map_or_else(|| "here".to_owned(), ui::hints::far),
                angle: ui::hud::bearing(camera.position.to_array(), camera.target.to_array(), [x, 0.0, z]),
            })
        });
        if let Ok(mut shared) = self.shared.lead.lock() {
            *shared = found.map(|(mark, _)| mark);
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
        // While you type, egui has Esc; this is the open chat after a click
        // on the world
        if self.chat.open {
            if key.pressed() {
                self.chat.close();
            }
            return true;
        }
        // Over the shoulder (T1.4) Esc closes the top panel, or with none
        // open lets the cursor go, and never quits
        if self.control_mode() == ControlMode::Direct {
            if key.pressed() && !self.close_top_panel() {
                self.shared.cursor_let_go.store(true, Ordering::Relaxed);
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
                    chat_open: self.chat.open,
                    chat_unread: {
                        use capitol_core::protocol::ws::Channel;
                        [Channel::Local, Channel::Capitol, Channel::Guild].map(|c| session.talk().unread(c)).iter().sum()
                    },
                    gazette_news: session
                        .gazette()
                        .unread()
                        .then(|| session.gazette().latest_headline().unwrap_or_default().to_owned()),
                    mail_open: self.mail.open,
                    mail_unread: session.mail().unread(),
                    help_open: self.shortcuts.open,
                    road_in: self.hints.hidden(),
                    shown_lot: self.shown_chip.clone(),
                    working: session.wages().working().map(|(o, s)| capitol_core::wages::chip_words(o, s)),
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
                    if let Some(step) = self.hints.saw(Did::UsedInventory) {
                        session.tutorial_seen(step);
                    }
                }
                if clicks.crafting {
                    self.crafting.toggle();
                    if let Some(step) = self.crafting.open.then(|| self.hints.saw(Did::OpenedCrafting)).flatten() {
                        session.tutorial_seen(step);
                    }
                }
                if clicks.road_in {
                    self.hints.bring_back();
                    session.set_tutorial_dismissed(false);
                }
                if let (true, Some(shown)) = (clicks.shown_lot, &self.shown_lot) {
                    session.move_to(Instant::now(), shown.x, shown.y);
                }
                if clicks.work {
                    self.gazette.open_work();
                }
                if let (true, Some(shift)) = (clicks.stop_work, session.wages().shift()) {
                    let ask = capitol_core::wages::Ask::Stop(shift.offer_id);
                    session.wage_act(ask);
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
                if clicks.chat {
                    self.chat.toggle();
                }
                if clicks.mail {
                    self.mail.toggle();
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
                // Enter crafts at the open station, else on the crafting
                // panel, when either is set to a recipe; otherwise it talks.
                let enter = std::mem::take(&mut self.craft_key);
                let at_station = self.station.recipe(session.state()).is_some();
                let ready = self.crafting.open && self.crafting.recipe(session.state()).is_some();
                if std::mem::take(&mut self.chat_key) && ui::chat::enter_means(ready, at_station) == ui::chat::Enter::Chat {
                    self.chat.open_typing();
                }
                let key = (enter && !at_station && ready).then_some(CraftAction::Craft);
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
                // Talk (S1.3): beside the crafting panel if it reaches down
                // The inventory's rect is last frame's: it's drawn after.
                let before = self.station.rect.or(self.inventory.rect);
                let (clock, me) = (session.talk_time(now), session.state().me());
                let guild = session.guilds().guild().map(|g| g.name.clone());
                let view = ui::chat::View { now, clock, me, beside: self.crafting.rect, before, cards: true, guild };
                let (log, chat) = session.talk_and_chat();
                match self.chat.show(ctx, log, chat, &view) {
                    Some(ui::chat::Action::Say(channel)) => session.say(channel),
                    Some(ui::chat::Action::Card { id, name, citizen }) => {
                        session.open_card(capitol_core::card::Who::Id(id), Some(capitol_core::card::Hint { name, citizen }))
                    }
                    None => {}
                }
                let state = session.state();
                let view = ui::exchange::View {
                    inventory: state.inventory(),
                    strands: state.strands(),
                    fees: state.fees(),
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
                    listings: session.market_listings().map(|l| l.listings.as_slice()),
                    book: self.market.selected.as_deref().and_then(|i| market.book(i)),
                    history: market.history(),
                    orders: market.open_orders(),
                    inventory: state.inventory(),
                    market_box: state.market_box(),
                    fees: state.fees(),
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
                            fees: state.fees(),
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
                // The notice board: Wanted (S2.3) and the Gazette (E3.9). The
                // board is taken out to be drawn beside the state it reads.
                let mut board = std::mem::take(session.board_mut());
                let hall = self.terrain.landmark(LandmarkKind::ExchangeHall);
                let action = {
                    let state = session.state();
                    let at_hall = match (hall, state.local_player()) {
                        (Some(hall), Some(me)) => rules::market::at_the_hall(hall, me.x, me.y),
                        _ => false,
                    };
                    let wanted = ui::gazette::Wanted {
                        board: &mut board,
                        view: ui::board::View {
                            pack: state.inventory(),
                            me: state.me(),
                            strands: state.strands(),
                            now: state.utc_now(),
                            catalog: state.catalog(),
                            at_hall,
                            walking: session.walking_to_board(),
                            online: true,
                        },
                        icons: &mut self.icons,
                        work: Some(ui::gazette::WorkPage {
                            wages: session.wages(),
                            view: ui::work::View {
                                me: state.me(),
                                at: state.local_player().map(|me| [me.x, me.y]),
                                now: state.utc_now(),
                                catalog: state.catalog(),
                                walking: session.walking_to_work(),
                                online: true,
                            },
                        }),
                    };
                    let view = ui::gazette::View { reader: session.gazette(), online: true };
                    self.gazette.show(ctx, &view, Some(wanted))
                };
                *session.board_mut() = board;
                match action {
                    Some(ui::gazette::Action::Turn(by)) => session.gazette_mut().turn(by),
                    Some(ui::gazette::Action::Byline(named)) => session.set_byline(named),
                    Some(ui::gazette::Action::Card(name)) => {
                        session.open_card(capitol_core::card::Who::Name(name), None)
                    }
                    Some(ui::gazette::Action::Board(ui::board::Action::Act(ask))) => {
                        session.board_act(now, hall, ask)
                    }
                    Some(ui::gazette::Action::Board(ui::board::Action::Card(id))) => {
                        session.open_card(capitol_core::card::Who::Id(id), None)
                    }
                    Some(ui::gazette::Action::Work(ui::work::Action::Take(id))) => session.take_work(now, id),
                    Some(ui::gazette::Action::Work(ui::work::Action::Act(ask))) => session.wage_act(ask),
                    Some(ui::gazette::Action::Work(ui::work::Action::Card(id))) => {
                        session.open_card(capitol_core::card::Who::Id(id), None)
                    }
                    None => {}
                }
                // Letters (S1.5)
                session.set_mail_open(self.mail.open);
                let view = ui::mail::View {
                    strands: session.state().strands(),
                    now: session
                        .state()
                        .server_time_ms()
                        .and_then(capitol_core::chrono::DateTime::from_timestamp_millis)
                        .unwrap_or_else(capitol_core::chrono::Utc::now),
                    online: true,
                };
                match self.mail.show(ctx, session.mail_mut(), &view) {
                    Some(ui::mail::Action::Card(who)) => {
                        let hint = capitol_core::card::Hint { name: who.name, citizen: who.citizen };
                        session.open_card(capitol_core::card::Who::Id(who.id), Some(hint));
                    }
                    Some(ui::mail::Action::Close) | None => {}
                }
                // Guilds (S4.4)
                let hall = self.terrain.landmark(LandmarkKind::ExchangeHall);
                let view = {
                    let state = session.state();
                    let me = state.local_player();
                    ui::guild::View {
                        me: state.me(),
                        my_name: me.map(|p| p.name.clone()).unwrap_or_default(),
                        strands: state.strands(),
                        subscribed: state.subscription().is_some_and(|s| s.is_active_at(state.utc_now())),
                        at_hall: match (hall, me) {
                            (Some(hall), Some(me)) => rules::market::at_the_hall(hall, me.x, me.y),
                            _ => false,
                        },
                        online: true,
                        now: state.utc_now(),
                    }
                };
                match self.guild.show(ctx, session.guilds_mut(), &view) {
                    Some(ui::guild::Action::Card { id, name, citizen }) => {
                        session.open_card(capitol_core::card::Who::Id(id), Some(capitol_core::card::Hint { name, citizen }))
                    }
                    Some(ui::guild::Action::ToHall) => {
                        if let Some(hall) = hall {
                            session.walk_to_hall(now, hall);
                        }
                    }
                    Some(ui::guild::Action::Close) | None => {}
                }
                // A person's card (S0.3), over the Gazette when it's opened from it
                let today = rules::day_number(
                    session.state().server_time_ms().unwrap_or_else(|| capitol_core::chrono::Utc::now().timestamp_millis()),
                );
                let view = ui::card::View { shown: session.cards().shown(), today, online: true, catalog: session.state().catalog() };
                match self.card.show(ctx, &view) {
                    Some(ui::card::Action::Trade(id)) => {
                        session.trade_with(now, id);
                    }
                    Some(ui::card::Action::Mute(id)) => session.mute(id, true),
                    Some(ui::card::Action::Unmute(id)) => session.mute(id, false),
                    Some(ui::card::Action::Write(to)) => {
                        session.mail_mut().write_to(to);
                        self.mail.open = true;
                    }
                    Some(ui::card::Action::Close) => session.close_card(),
                    None => {}
                }
                // The settings panel takes the hint card's corner, and the
                // Gazette and the letters the middle of the screen; the card
                // waits behind them rather than showing through.
                if self.settings_panel.open || self.gazette.open || self.mail.open || self.guild.open || self.zone_card.is_open() || self.deeds.is_open() {
                    self.hints.rect = None;
                } else {
                    // The inventory's and the station's rects are last frame's: they're drawn after.
                    let panels: Vec<_> =
                        [self.inventory.rect, self.crafting.rect, self.exchange.rect, self.market.rect, self.impound.rect, self.shop.rect, self.station.rect, self.card.rect, self.chat.rect, self.mail.rect]
                            .into_iter()
                            .flatten()
                            .collect();
                    if self.hints.show(ctx, now, &panels, self.lead) {
                        session.set_tutorial_dismissed(true);
                    }
                    // The tax card read (#449): its "Noted" is the step
                    if let Some(step) = self.hints.noted() {
                        session.tutorial_seen(step);
                    }
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
                    deed: picked.map_or(capitol_core::deeds::DeedOffer::None, |p| {
                        capitol_core::deeds::offer(p, state.tutorial())
                    }),
                    // Gathering work on a plot of yours (S2.7)
                    hire: picked.filter(|p| p.owner_id.is_some() && p.owner_id == state.me()).map(|p| ui::plot::Hire {
                        wages: session.wages(),
                        me: state.me(),
                        resources: capitol_core::wages::resources_on(&p.bounds, state.nodes().values())
                            .into_iter()
                            .map(|id| {
                                let name = state.catalog().resource_type(&id).map_or_else(|| id.replace('_', " "), |r| r.name.clone());
                                (id, name)
                            })
                            .collect(),
                        catalog: state.catalog(),
                        now: state.utc_now(),
                        online: true,
                    }),
                };
                let beside = self.chat.rect.filter(|_| self.chat.open).or(self.crafting.rect);
                match self.plot_panel.show(ctx, &view, beside) {
                    Some(ui::plot::Action::Claim(id)) => session.claim_plot(id),
                    Some(ui::plot::Action::ClaimWithDeed(id)) => session.claim_with_grant(id),
                    Some(ui::plot::Action::PayTax(id, amount)) => session.pay_tax(id, amount),
                    Some(ui::plot::Action::AboutZone(zone)) => {
                        self.zone_card.open_one(&zone);
                        session.ask_zone_facts();
                        if let Some(step) = self.hints.saw(Did::ReadZoneCard) {
                            session.tutorial_seen(step);
                        }
                    }
                    Some(ui::plot::Action::Wages(ui::hire::Act::Ask(ask))) => session.wage_act(ask),
                    Some(ui::plot::Action::Wages(ui::hire::Act::LookUp(draft))) => session.wages_mut().look_up(&draft),
                    Some(ui::plot::Action::Close) => session.close_plot(),
                    None => {}
                }
                // The zone card (#444), over the plot panel that opened it
                let facts = session.state().zone_facts().map(|f| f.zones.as_slice());
                let _ = self.zone_card.show(ctx, facts);
                // The Deeds Office (#448)
                let state = session.state();
                let view = ui::deeds::View {
                    facts: state.zone_facts().map(|f| f.zones.as_slice()),
                    lots: state.free_lots(),
                    grant: state.tutorial().map(|t| t.grant).unwrap_or_default(),
                    shown: self.shown_lot.as_ref().map(|s| s.terrain_id.as_str()),
                    claiming: session.claiming(),
                    refused: session.claim_refused(now),
                };
                match self.deeds.show(ctx, &view) {
                    Some(ui::deeds::Action::List(zone)) => session.ask_free_lots(zone.as_str()),
                    Some(ui::deeds::Action::ShowMe(shown)) => self.shown_lot = Some(shown),
                    Some(ui::deeds::Action::StopShowing) => self.shown_lot = None,
                    Some(ui::deeds::Action::Claim(id)) => session.claim_with_grant(id),
                    Some(ui::deeds::Action::Close) | None => {}
                }
                let view = ui::station::View {
                    state: session.state(),
                    phase: session.crafting().phase(),
                    refused: session
                        .craft_refused(now)
                        .or_else(|| session.move_refused(now)),
                    beside: self.inventory.rect,
                    automation_refused: session.automation_refused(now),
                    wages: Some(session.wages()),
                    now: session.state().utc_now(),
                    online: true,
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
                    Some(StationAction::Wages(ui::hire::Act::Ask(ask))) => session.wage_act(ask),
                    Some(StationAction::Wages(ui::hire::Act::LookUp(draft))) => session.wages_mut().look_up(&draft),
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
                    if let Some(step) = self.hints.saw(Did::UsedInventory) {
                        session.tutorial_seen(step);
                    }
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
        // Muted on another device since: ask again as it opens (S1.4)
        if self.settings_panel.just_opened() {
            if let Some(session) = self.session.as_mut() {
                session.refresh_mutes();
            }
        }
        let view = ui::settings::View {
            server: self.session.as_ref().map(|s| s.server()),
            mutes: self.session.as_ref().map(|s| s.mutes()),
        };
        // A pinned mode is what the panel shows and changes, for this run
        // alone: swapped in for the panel and back out, so the file keeps
        // the player's own.
        let settings = &mut self.settings.settings;
        if let Some(mode) = &mut self.controls_pinned {
            std::mem::swap(mode, &mut settings.controls.mode);
        }
        let asked = self.settings_panel.show(ctx, settings, &view);
        if let Some(mode) = &mut self.controls_pinned {
            std::mem::swap(mode, &mut settings.controls.mode);
        }
        match asked {
            Some(ui::settings::Action::SetServer(server)) => self.switch_server(&server),
            Some(ui::settings::Action::Unmute(id)) => {
                if let Some(session) = self.session.as_mut() {
                    session.mute(id, false);
                }
            }
            Some(ui::settings::Action::Close) => self.settings_panel.toggle(),
            None => {}
        }
        if self.session.is_none() {
            if !self.shared.clean.load(Ordering::Relaxed) {
                ui::inscription(ctx, &self.shared, &self.terrain.status());
            }
        } else if !at_login {
            let chat = self.chat.rect.filter(|_| self.chat.open);
            let clear_of = self.plot_panel.rect.or(self.market.rect).or(chat).or(self.crafting.rect);
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
            // On a shift, the header's chip says so (S2.7). Drawn first, as
            // online, so the panels are placed under it
            if let Some((o, sh)) = self.station_panel_wages.as_ref().and_then(|w| w.working()) {
                let view = HeaderView {
                    zone: None,
                    strands: Some(148),
                    who: "You",
                    patron: None,
                    connection: None,
                    down: false,
                    inventory_open: true,
                    crafting_open: true,
                    exchange_open: false,
                    market_open: false,
                    gazette_open: false,
                    gazette_news: None,
                    chat_open: false,
                    chat_unread: 0,
                    mail_open: false,
                    mail_unread: 0,
                    help_open: false,
                    settings_open: false,
                    inventory_pulse: 0.0,
                    hall_hint: None,
                    impound: None,
                    impound_open: false,
                    road_in: None,
                    shown_lot: None,
                    working: Some(capitol_core::wages::chip_words(o, sh)),
                };
                ui::hud::header(ctx, &view, &mut self.strands, Instant::now());
            }
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
                wages: self.station_panel_wages.as_ref(),
                now: capitol_core::chrono::Utc::now(),
                online: false,
            };
            self.station.show(ctx, &mut self.icons, &view, 0.0);
            if let Some(tab) = self.station_panel_tab.take() {
                self.station.tab = tab;
            }

            self.inventory.show(ctx, &mut self.icons, state.catalog(), state.inventory(), None, 0.0);
        }
        if let Some((log, chat, me, state, hints)) = &mut self.chat_sample {
            let phase = capitol_core::craft::Phase::Idle;
            let view = ui::crafting::View { state, phase: &phase, refused: None };
            self.crafting.show(ctx, &mut self.icons, &view, 0.0);
            // The sample's clock started at 0 as the run did
            let clock = ctx.input(|i| i.time);
            let (me, before) = (Some(*me), self.inventory.rect);
            let view = ui::chat::View { now: Instant::now(), clock, me, beside: self.crafting.rect, before, cards: false, guild: Some("The Rope Walkers".into()) };
            if let Some(ui::chat::Action::Say(channel)) = self.chat.show(ctx, log, chat, &view) {
                chat.send(channel);
            }
            self.inventory.show(ctx, &mut self.icons, state.catalog(), state.inventory(), None, 0.0);
            let panels: Vec<_> = [self.inventory.rect, self.crafting.rect, self.chat.rect].into_iter().flatten().collect();
            hints.show(ctx, Instant::now(), &panels, None);
        }
        if self.hints_sample {
            if let Some(hidden) = self.hints.hidden() {
                let view = HeaderView {
                    zone: None,
                    strands: Some(100),
                    who: "Ada Reeder",
                    patron: None,
                    connection: None,
                    down: false,
                    inventory_open: false,
                    crafting_open: false,
                    exchange_open: false,
                    market_open: false,
                    gazette_open: false,
                    gazette_news: None,
                    chat_open: false,
                    chat_unread: 0,
                    mail_open: false,
                    mail_unread: 0,
                    help_open: self.shortcuts.open,
                    settings_open: false,
                    inventory_pulse: 0.0,
                    hall_hint: None,
                    impound: None,
                    impound_open: false,
                    road_in: Some(hidden),
                    shown_lot: None,
                    working: None,
                };
                ui::hud::header(ctx, &view, &mut self.strands, Instant::now());
            }
            self.hints.show(ctx, Instant::now(), &[], self.lead);
        }
        if let Some(reader) = &mut self.gazette_sample {
            self.gazette.reading(reader);
            if let Some(ui::gazette::Action::Turn(by)) =
                self.gazette.show(ctx, &ui::gazette::View { reader, online: false }, None)
            {
                reader.turn(by);
            }
        }
        if let Some(s) = &mut self.board_sample {
            let wanted = ui::gazette::Wanted {
                board: &mut s.board,
                view: ui::board::View {
                    pack: s.pack.as_ref(),
                    me: Some(s.me),
                    strands: Some(1200),
                    now: s.now,
                    catalog: &s.catalog,
                    at_hall: false,
                    walking: false,
                    online: false,
                },
                icons: &mut self.icons,
                work: Some(ui::gazette::WorkPage {
                    wages: &s.wages,
                    view: ui::work::View {
                        me: Some(s.me),
                        at: Some([capitol_core::protocol::rules::CENTER_X, capitol_core::protocol::rules::CENTER_Y + 400.0]),
                        now: s.now,
                        catalog: &s.catalog,
                        walking: false,
                        online: false,
                    },
                }),
            };
            self.gazette.reading(&mut s.reader);
            if let Some(ui::gazette::Action::Turn(by)) =
                self.gazette.show(ctx, &ui::gazette::View { reader: &s.reader, online: false }, Some(wanted))
            {
                s.reader.turn(by);
            }
        }
        if let Some(facts) = &self.zone_card_sample {
            let _ = self.zone_card.show(ctx, Some(facts));
        }
        if let Some(sample) = &self.deeds_sample {
            let view = ui::deeds::View {
                facts: Some(&sample.facts),
                lots: Some(&sample.lots),
                grant: sample.grant,
                shown: self.shown_lot.as_ref().map(|s| s.terrain_id.as_str()),
                claiming: false,
                refused: None,
            };
            match self.deeds.show(ctx, &view) {
                Some(ui::deeds::Action::ShowMe(shown)) => self.shown_lot = Some(shown),
                Some(ui::deeds::Action::StopShowing) => self.shown_lot = None,
                _ => {}
            }
            if let Some(lot) = &self.shown_chip {
                let view = HeaderView {
                    zone: None,
                    strands: Some(100),
                    who: "Ada Reeder",
                    patron: None,
                    connection: None,
                    down: false,
                    inventory_open: false,
                    crafting_open: false,
                    exchange_open: false,
                    market_open: false,
                    gazette_open: false,
                    gazette_news: None,
                    chat_open: false,
                    chat_unread: 0,
                    mail_open: false,
                    mail_unread: 0,
                    help_open: false,
                    settings_open: false,
                    inventory_pulse: 0.0,
                    hall_hint: None,
                    impound: None,
                    impound_open: false,
                    road_in: None,
                    shown_lot: Some(lot.clone()),
                    working: None,
                };
                ui::hud::header(ctx, &view, &mut self.strands, Instant::now());
            }
        }
        if let Some((mail, view)) = &mut self.mail_sample {
            let _ = self.mail.show(ctx, mail, view);
        }
        if let Some((guilds, view)) = &mut self.guild_sample {
            let _ = self.guild.show(ctx, guilds, view);
        }
        if let Some((cards, today, catalog)) = &mut self.card_sample {
            // Trade is drawn as it would be signed in; offline it asks no one
            let view = ui::card::View { shown: cards.shown(), today: *today, online: true, catalog };
            if let Some(ui::card::Action::Close) = self.card.show(ctx, &view) {
                cards.close();
            }
        }
        if let Some(juice) = &mut self.juice {
            juice.ui(ctx, &mut self.icons, &self.shared);
        }
        ui::slots::paint_shatters(ctx);
        // The dev overlay (D5.2): the store's counts and the net once there's
        // a server, the frames and the engine's entities always
        let (session, sample, layer) = (self.session.as_ref(), self.dev_sample, self.layer.name());
        self.dev.show(ctx, || {
            let Some(session) = session else {
                return match sample {
                    true => ui::dev_overlay::sample(),
                    false => ui::dev_overlay::View::default(),
                };
            };
            let state = session.state();
            let players = state.players();
            ui::dev_overlay::View {
                net: Some(session.net_stats(Instant::now())),
                world: Some(ui::dev_overlay::World {
                    people: players.len(),
                    citizens: players.values().filter(|p| p.citizen).count(),
                    nodes: state.nodes().len(),
                    stations: state.stations().len(),
                    lots: state.claimed_lots().len(),
                }),
                interest: state
                    .local_player()
                    .map(|me| ui::dev_overlay::Interest::at(me.x, me.y)),
                // An admin's token adds the server's column (D5.3)
                server: session
                    .admin()
                    .map(|watch| ui::dev_overlay::Server::from_watch(watch, state.server_time_ms(), layer)),
            }
        });
        if !self.in_world() && !self.hints_sample {
            self.shortcuts.open = false;
        }
        let asked = self.shortcuts.show(ctx, self.hints.hidden().is_some());
        if asked.close {
            self.shortcuts.open = false;
        }
        if asked.road_in {
            self.hints.bring_back();
            if let Some(session) = &mut self.session {
                session.set_tutorial_dismissed(false);
            }
        }
        if let Some(action) = action {
            self.act(action);
        }
        // Over the shoulder the camera rig captures the cursor whenever
        // nothing here wants it (T1.4)
        let wants = self.wants_cursor();
        self.shared.wants_cursor.store(wants, Ordering::Relaxed);
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
/// Hang what each player is saying over their name (Commons S1.2), once
/// `entities::Players` has placed the names this frame.
fn speak(shared: &script_api::Shared, bubble: impl Fn(uuid::Uuid) -> Option<script_api::Speech>) {
    if let Ok(mut labels) = shared.players.lock() {
        for label in labels.iter_mut() {
            label.line = bubble(label.id);
        }
    }
}

/// A murmur for each line just heard: a local one where it was said, a
/// Capitol one from nowhere.
fn murmurs(sound: &mut Soundscape, lines: &[capitol_core::talk::Line]) {
    use capitol_core::protocol::ws::Channel;
    for line in lines {
        let at = (line.channel == Channel::Local).then(|| {
            let [x, z] = crate::coords::to_world(line.x, line.y);
            [x, crate::world::land::ground_y([x, z]) + 1.6, z]
        });
        sound.murmur(at);
    }
}

fn listen(ctx: &mut ExtensionCtx, shared: &script_api::Shared, sound: &mut Soundscape) {
    let cam = ctx.camera;
    let focus = shared
        .follow
        .lock()
        .ok()
        .and_then(|f| *f)
        .map_or([cam.target.x, cam.target.z], |f| [f[0], f[2]]);
    // The bazaar's crowd (E3.14): full among its stalls, gone 24 m off
    let market = shared.stall_feet.lock().map_or(0.0, |feet| {
        let near = feet
            .iter()
            .map(|q| {
                let c = [(q[0][0] + q[2][0]) * 0.5, (q[0][1] + q[2][1]) * 0.5];
                (c[0] - focus[0]).hypot(c[1] - focus[1])
            })
            .fold(f32::MAX, f32::min);
        1.0 - crate::sound::dsp::smoothstep(4.0, 24.0, near)
    });
    let ears = Ears {
        focus,
        camera: [cam.position.x, cam.position.y, cam.position.z],
        // Over the land under the camera, so a hillside isn't a height.
        height: cam.position.y - crate::world::land::ground_y([cam.position.x, cam.position.z]),
        market,
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

/// The movement keys as the way to walk (Direct mode, T1.3), in server
/// units: against the yaw the camera rig means to have (`cap_view_yaw`),
/// two keys held no faster than one.
fn keys_way(ctx: &ExtensionCtx, shared: &script_api::Shared) -> (f64, f64) {
    let held = |action| if ctx.input.is_action_pressed(action) { 1.0 } else { 0.0 };
    let forward = held("move_forward") - held("move_backward");
    let right = held("move_right") - held("move_left");
    let yaw = shared.view_yaw.lock().ok().and_then(|y| *y).unwrap_or(0.0);
    capitol_core::steering::camera_way(forward, right, yaw)
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

/// The admin's world layers (D5.3): what `layer` shows round you, for
/// `debug.rhai`, and the walkers asked for while it shows them. Off, or for
/// anyone but an admin, nothing. Also tells the session whether the dev
/// overlay is open, so it watches the server only then.
fn debug_layers(session: &mut Session, layer: crate::debug_layers::Layer, poses: Option<&[capitol_core::motion::Pose]>, overlay: bool, shared: &Shared) {
    use crate::debug_layers::Layer;
    session.set_admin_open(overlay);
    let me = session.state().local_player().map(|me| (me.x, me.y));
    let on = layer != Layer::Off && session.admin().is_some();
    let (mark, round) = match (on, me, poses) {
        (true, Some(me), Some(poses)) => {
            let walkers = session.admin().map_or(&[][..], |w| w.walkers());
            let mark = crate::debug_layers::gather(layer, me, session.state(), session.walks(), walkers, poses);
            (Some(mark), layer.wants_walkers().then_some(me))
        }
        _ => (None, None),
    };
    session.watch_walkers(round);
    if let Ok(mut debug) = shared.debug.lock() {
        *debug = mark;
    }
}

/// A person's card (S0.3): open the one a script asked for (`cap_open_card`),
/// with the name over their head and their mark to show until it comes.
fn open_card(session: &mut Session, shared: &Shared) {
    let Some(id) = shared.open_card.lock().ok().and_then(|mut c| c.take()) else { return };
    match uuid::Uuid::parse_str(&id) {
        Ok(player) => {
            let hint = shared.players.lock().ok().and_then(|players| {
                players
                    .iter()
                    .find(|p| p.id == player)
                    .map(|p| capitol_core::card::Hint { name: p.name.clone(), citizen: p.citizen })
            });
            session.open_card(capitol_core::card::Who::Id(player), hint);
        }
        Err(e) => tracing::warn!("cap_open_card: {id:?} isn't a player id: {e}"),
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
/// extraction didn't happen.
fn gather(
    ctx: &mut ExtensionCtx,
    session: &mut Session,
    shared: &script_api::Shared,
    sound: &mut Soundscape,
    now: Instant,
) {
    if let Some(id) = shared.extract.lock().ok().and_then(|mut e| e.take()) {
        match uuid::Uuid::parse_str(&id) {
            Ok(node) => session.extract(now, node),
            Err(e) => tracing::warn!("cap_extract: {id:?} isn't a node id: {e}"),
        }
    }
    for (node, item) in session.take_yields() {
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
/// `extraction_completed` has.
fn crafted(ctx: &mut ExtensionCtx, session: &mut Session, shared: &script_api::Shared, sound: &mut Soundscape) {
    for item in session.take_crafted() {
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
}

/// Plots (P6.4): pick the plot a script asked for (`cap_pick_plot`), tell
/// scripts which is picked (`cap_picked_plot`), and fire at them
/// `plot_claimed` with the claimed plot's middle (`x`, `z`, world metres)
/// and about how far it reaches from there (`r`, metres), and `plot_note`
/// with a `title` and `body` when a pick found nothing.
/// The plot panel's pick, its outline, and the claims just made: their
/// rings and toasts. True if the road in's deed paid for one (#448).
fn plots(
    ctx: &mut ExtensionCtx,
    session: &mut Session,
    shared: &script_api::Shared,
    lots: &HashMap<String, Lot>,
    sound: &mut Soundscape,
) -> bool {
    let mut granted_any = false;
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
    for (plot_id, granted) in session.take_claimed() {
        granted_any |= granted;
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
        data.insert("granted".into(), granted.into());
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
            PlotNote::Collapsed { station_type, impounded } => {
                let name = session
                    .state()
                    .catalog()
                    .station_type(&station_type)
                    .map_or_else(|| station_type.replace('_', " "), |t| t.name.to_lowercase());
                let body = if impounded.is_empty() {
                    "It stood out on public land until the weather wore it out. Its kit is lost".to_string()
                } else {
                    "It stood out on public land until the weather wore it out. Its kit is lost, and what it held is in your impound".to_string()
                };
                (format!("Your {name} fell down"), body)
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
    granted_any
}

/// A lot as the road in's ring marks it (#442): its edge traced, and a ring
/// at its middle.
fn lot_mark(lot: &Lot) -> script_api::LeadMark {
    let n = lot.outline.len().max(1) as f64;
    let [x, y] = lot.outline.iter().fold([0.0, 0.0], |m, p| [m[0] + p[0] / n, m[1] + p[1] / n]);
    script_api::LeadMark {
        at: crate::coords::to_world(x, y),
        radius: 2.5,
        outline: lot.outline.iter().map(|p| crate::coords::to_world(p[0], p[1])).collect(),
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

#[cfg(test)]
mod escape_tests {
    //! T1.4: over the shoulder Esc closes the top panel, or lets the cursor
    //! go, and never reaches the engine's quit; in Click it does as before.
    use super::*;
    use std::path::Path;
    use winit::keyboard::ModifiersState;

    fn extension(mode: ControlMode) -> CapitolExtension {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = TerrainSource::offline(&path).expect("fixture");
        let mut ext = CapitolExtension::new(Shared::default(), terrain, None);
        ext.controls_pinned = Some(mode);
        ext
    }

    fn escape(ext: &mut CapitolExtension) -> bool {
        let key = KeyInput {
            key: KeyCode::Escape,
            state: ElementState::Pressed,
            repeat: false,
            character: None,
            modifiers: ModifiersState::empty(),
            scripted: true,
        };
        ext.on_key(&key)
    }

    #[test]
    fn escape_closes_the_top_panel_then_lets_the_cursor_go_and_never_quits() {
        let mut ext = extension(ControlMode::Direct);
        let let_go = ext.shared.cursor_let_go.clone();
        ext.enter_direct();
        assert!(!ext.inventory.open && !ext.crafting.open, "Direct starts with the view clear");
        ext.inventory.open = true;
        ext.mail.open = true;
        assert!(escape(&mut ext), "kept from the engine");
        assert!(!ext.mail.open && ext.inventory.open, "the letters, over the inventory, first");
        assert!(escape(&mut ext));
        assert!(!ext.inventory.open);
        assert!(!let_go.load(Ordering::Relaxed), "closing a panel isn't letting go");
        assert!(escape(&mut ext), "with no panel open, still kept from the engine");
        assert!(let_go.load(Ordering::Relaxed), "and the cursor is let go");
        assert!(ext.wants_cursor(), "so it stays free");
        // A panel opening clears it, so closing that captures again
        ext.inventory.rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0)));
        assert!(ext.wants_cursor());
        assert!(!let_go.load(Ordering::Relaxed));
        ext.inventory.rect = None;
        assert!(!ext.wants_cursor(), "nothing wants it now: Direct captures it");
        ext.alt_held = true;
        assert!(ext.wants_cursor(), "Alt frees it");
    }

    #[test]
    fn in_click_escape_still_reaches_the_engine() {
        let mut ext = extension(ControlMode::Click);
        ext.inventory.open = true;
        assert!(!escape(&mut ext));
        assert!(ext.inventory.open);
    }
}
