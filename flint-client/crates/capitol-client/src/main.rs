//! The Capitol client. It builds a `flint_player::PlayerApp` for the scene
//! and installs `CapitolExtension`, through which everything the Capitol
//! adds reaches the engine. It runs `scenes/capitol.scene.toml`: the city at
//! dusk, as runtime meshes built from the terrain, which is fetched from the
//! server (and cached) or read from `--offline <file>`. Online it opens on
//! the login form, with the camera circling the tower behind it, unless a
//! stored session lets it straight in. The camera is the rig in
//! `scripts/camera.rhai`, and it follows your avatar once you're in; the
//! names over the players are `scripts/hud.rhai`. The P0.4 spike's script, markers and inscription
//! window (game egui) stay until the living world (P4) replaces them.

mod atmosphere;
mod building_sample;
mod camera_path;
#[cfg(test)]
mod camera_rig;
mod coords;
mod daylight;
mod debug_layers;
mod entities;
mod extension;
mod juice_sample;
mod labels;
mod nodes;
mod script_api;
mod session;
mod settings;
mod sound;
mod spike_world;
mod stations;
mod terrain;
mod ui;
mod world;

use anyhow::{Context, Result};
use clap::Parser;
use flint_player::PlayerApp;
use std::path::{Path, PathBuf};
use winit::event_loop::{ControlFlow, EventLoop};

#[derive(Parser)]
#[command(name = "capitol", about = "The Capitol, as a Flint game")]
struct Args {
    /// Scene file, relative to flint-client/
    #[arg(long, default_value = "scenes/capitol.scene.toml")]
    scene: String,

    /// Text written over the Capitol
    #[arg(long, default_value = "The Capitol")]
    note: String,

    /// Server root, for this run. Without it, the server last chosen on the
    /// login form, or http://localhost:3000
    #[arg(long, env = "CAPITOL_SERVER")]
    server: Option<String>,

    /// Build the world from this /terrain JSON and never touch the network:
    /// no server, so no login (fixtures/terrain.json, or a file from cache/)
    #[arg(long, value_name = "TERRAIN_JSON", conflicts_with = "server")]
    offline: Option<PathBuf>,

    /// Where the camera rig starts. street, district and map are its own
    /// zooms, and bazaar puts its point in the bazaar's lane (E3.13). The
    /// rest are fixed shots, held until the first camera input: bridge
    /// (close, over the inner canal's south crossing), suburb (low over the
    /// southern suburbs' curving streets), urban (close over the urban lots
    /// by the south arterial), capitol (up at the tower from beyond the
    /// moat), crown (level with its lantern), from-urban, from-suburban and
    /// from-rural (the tower on the horizon from each outer zone),
    /// from-wilderness (down from a ridge to the tower) or range (out along the ranges from a summit)
    #[arg(long, default_value = "street", value_parser = [
        "street", "district", "map", "bridge", "suburb", "urban", "capitol", "crown",
        "from-urban", "from-suburban", "from-rural", "from-wilderness", "range", "bazaar",
    ])]
    view: String,
    /// Offline, film a camera move: a file of timed keys, each where the
    /// camera stands and what it looks at (`t px py pz tx ty tz`, world
    /// metres; see camera_path.rs). It takes the place of --view, holds at
    /// its last key, and the first camera input lets go of it into the rig
    #[arg(long, value_name = "FILE", requires = "offline")]
    camera_path: Option<PathBuf>,

    #[arg(long, default_value_t = 1280)]
    width: u32,
    #[arg(long, default_value_t = 720)]
    height: u32,

    /// Write a PNG of the presented frame (scene and egui). Defaults to
    /// renders/capitol.png when only --screenshot-at is given.
    #[arg(long, value_name = "PATH")]
    screenshot: Option<PathBuf>,
    /// Game time in seconds for the screenshot (default: the first frame)
    #[arg(long, value_name = "SECS")]
    screenshot_at: Option<f64>,
    /// Also write a numbered PNG this often (game seconds), from
    /// --screenshot-at on: renders/capitol_0001.png and on
    #[arg(long, value_name = "SECS")]
    screenshot_every: Option<f64>,
    /// Advance game time exactly this much per frame, whatever the wall
    /// clock does, so a scripted capture plays the same every time
    #[arg(long, value_name = "SECS")]
    frame_step: Option<f64>,
    /// Exit once game time reaches this many seconds
    #[arg(long, value_name = "SECS")]
    exit_after: Option<f64>,
    /// Timed key and mouse input (flint-player's input-script format). It can tap
    /// the debug keys too: `2.0 tap F2` opens the stats overlay
    #[arg(long, value_name = "FILE")]
    input_script: Option<String>,
    /// Open the F2 rendering stats overlay from the first frame
    #[arg(long)]
    stats: bool,

    /// Show every item icon (assets/icons/) on a panel, for review
    #[arg(long)]
    icon_sheet: bool,
    /// Show two sample panels made of the UI theme's parts, for comparing
    /// the theme with the React client's panels (tools/theme-sample.html)
    #[arg(long)]
    ui_sample: bool,
    /// Once signed in, show every container you hold as a slot grid, with
    /// drag and drop between them (P5.3, until the panels take over)
    #[arg(long)]
    containers: bool,
    /// Once signed in, start placing the first kit of this item type
    /// ("workbench_kit"), as a right click on it in the inventory does.
    /// For captures: they can't click egui (issue #76)
    #[arg(long, value_name = "KIT")]
    place: Option<String>,
    /// Once signed in, open the exchange panel: "deposit" ticks every fiber
    /// stack, "withdraw:200" types 200. For captures: they can't click or
    /// type into egui (issue #76)
    #[arg(long, value_name = "TAB[:N]")]
    exchange: Option<String>,
    /// Open the market panel on an item, with the form filled in:
    /// "plank", or "plank:buy:10@41:B" for a buy of 10 at 41, B or better.
    /// For captures, which can't type into egui
    #[arg(long, value_name = "ITEM[:SIDE:N@PRICE[:GRADE]]")]
    market: Option<String>,
    /// Offline, open the market panel on a sample market: a plank book with
    /// blended lots and both grade rules, and a few orders of yours
    #[arg(long)]
    market_sample: bool,
    /// Offline, open the Capitol Gazette on a sample edition (E3.9)
    #[arg(long)]
    gazette_sample: bool,
    /// Offline, open the zone card (#444) on sample facts: "urban",
    /// "suburban", "rural", or "compare" (the default) for all three
    #[arg(long, value_name = "ZONE", num_args = 0..=1, default_missing_value = "compare")]
    zone_card_sample: Option<String>,
    /// Offline, open the Deeds Office (#448) on sample facts and a sample
    /// register: "compare" (the default), "lots" (the farms, your deed
    /// open), "shown" (the first farm shown on the ground), "confirm" (its
    /// claim asked once), "unearned" or "spent"
    #[arg(long, value_name = "PAGE", num_args = 0..=1, default_missing_value = "compare")]
    deeds_sample: Option<String>,
    /// Offline, open a person's card (S0.3) on a sample: "citizen" (the
    /// default), "person", "muted" (the person, muted, S1.4) or "mine"
    #[arg(long, value_name = "WHO", num_args = 0..=1, default_missing_value = "citizen")]
    card_sample: Option<String>,
    /// Offline, open the mailbox (S1.5) on sample letters: "in" (the
    /// default), "letter", "write" or "sent"
    #[arg(long, value_name = "VIEW", num_args = 0..=1, default_missing_value = "in")]
    mail_sample: Option<String>,
    /// Offline, the Exchange hall as if you stood where the camera looks
    /// with goods in your market box: the gold mote, the header's hint and,
    /// in range, the prompt (E3.7)
    #[arg(long)]
    hall_sample: bool,
    /// Offline, the dev overlay (F7) open on sample net, prediction and
    /// world numbers, to see its online layout (D5.2)
    #[arg(long)]
    dev_sample: bool,
    /// Offline, the bazaar with sample shops in its stalls (E3.11), you
    /// where the camera looks, and the shop panel open on stall N ("mine"
    /// for your own, stall 6). "street" keeps most of the stalls and opens
    /// nothing, for filming the street (#345)
    #[arg(long, value_name = "STALL")]
    bazaar_sample: Option<String>,
    /// Offline, open the chat (S1.3) on a sample of both channels, with the
    /// crafting and inventory panels open and the hint card up
    #[arg(long)]
    chat_sample: bool,
    /// Offline, the road in's card (#441) on a sample tour: "new" (the
    /// default), "half" (half way up the road), "hidden" (its card hidden:
    /// the header's chip instead) or "help" (hidden, with the `?` overlay
    /// open on its line). A step's name or a chapter's stands it there, and
    /// "collect_box-empty" is the market box's card with the box empty (#446)
    #[arg(long, value_name = "TOUR", num_args = 0..=1, default_missing_value = "new")]
    hints_sample: Option<String>,
    /// Offline, open the impound (#141) on a sample: what a foreclosed
    /// workshop leaves when your pack is full
    #[arg(long)]
    impound_sample: bool,
    /// Offline, open the station panel (#235) on a sample workbench of
    /// yours, beside the inventory and crafting panels, on its `craft` or
    /// `automation` tab, or `empty` (the craft tab, its box empty) or `chest`
    /// (an empty chest of yours), so its recycling bin may be pressed (#488). Check it at --width 1280 --height 720 at 150%
    #[arg(long, value_name = "TAB", requires = "offline")]
    station_panel_sample: Option<String>,
    /// No sound. Captures (--exit-after) are silent anyway
    #[arg(long)]
    mute: bool,
    /// Write every sound the game makes into DIR as WAVs, and exit. For
    /// listening to a recipe, or drawing its spectrogram
    #[arg(long, value_name = "DIR")]
    write_sounds: Option<PathBuf>,
    /// Offline, draw only the city: no inscription panel, no free-point
    /// mark. For showcase captures (tools/showcase.ps1)
    #[arg(long, requires = "offline")]
    clean: bool,
    /// Offline, play the juice on a timetable (P8.2): pickups to an
    /// inventory panel, an A grade's sparkle, the toast stack, a zone's
    /// banner and the camera's ease into it. For captures
    #[arg(long)]
    juice_sample: bool,
    /// Once a trade opens, offer every stack of this item type, and with
    /// ":N" N Strands too ("rope:25", or ":25" for Strands alone). For
    /// captures: they can't drag into egui (issue #76)
    #[arg(long, value_name = "ITEM[:N]")]
    trade_offer: Option<String>,
    /// Keep the settings (graphics, volumes, camera) in this file instead
    /// of save/settings.toml (P8.4). For captures that mustn't take the
    /// player's own
    #[arg(long, value_name = "PATH")]
    settings_file: Option<String>,
    /// Open the settings panel at start (F10 toggles it). For captures
    #[arg(long)]
    settings_panel: bool,
    /// Point and walk (click) or over the shoulder (direct) for this run,
    /// over the settings. Without it, offline runs and captures
    /// (--exit-after, --input-script) are click, whatever is saved, so
    /// every capture recipe stays as it was. F8 switches either way
    #[arg(long, value_enum, value_name = "MODE")]
    controls: Option<settings::ControlMode>,
    /// Pin the time of day, 0 to 1 from midnight (0.25 dawn, 0.5 noon, 0.8
    /// evening), in place of the server's clock. Offline it's evening
    /// unless this says otherwise. For captures
    #[arg(long, value_name = "0..1", value_parser = phase)]
    time_of_day: Option<f64>,
    /// Offline, claim this share of the lots (0 to 1, picked by id), each
    /// with its house already built (P8.6b). For captures
    #[arg(long, value_name = "0..1", requires = "offline")]
    claim_sample: Option<f32>,
    /// Offline, at this second claim the lots nearest the camera, so their
    /// houses grow. For captures
    #[arg(long, value_name = "SECS", requires = "offline")]
    grow_at: Option<f32>,
    /// Offline, stand one of every station the catalog knows in a row
    /// where the camera looks, and a crate for a type it doesn't (E2.6).
    /// For captures
    #[arg(long, requires = "offline")]
    station_sample: bool,
    /// Offline, stand stations at work and waiting in a row where the
    /// camera looks: forges running, waiting and idle, a workbench and an
    /// anvil running, a kiln with its box full (E4.4). For captures
    #[arg(long, requires = "offline")]
    automation_sample: bool,
    /// Offline, claim the home lots nearest where the camera first looks,
    /// with floors, one going up in scaffolding, and stations indoors and
    /// out in the weather, yours the nearest: to see the buildings open as
    /// you come close (V3.4). For captures
    #[arg(long, requires = "offline")]
    building_sample: bool,
    /// Offline, scatter every kind of resource node round where the camera
    /// first looks, and take every other one through being worked, taken
    /// and growing back on a loop (E2.5). For captures
    #[arg(long, requires = "offline")]
    node_sample: bool,
    /// Offline, stand people and citizens side by side where the camera
    /// first looks, so their lanterns and names can be compared (C0.2).
    /// "stroll" walks them instead, up and down the way the camera looks,
    /// as if along a street (#346). For captures
    #[arg(long, requires = "offline", value_name = "stroll", num_args = 0..=1,
        default_missing_value = "row", value_parser = ["row", "stroll"])]
    citizen_sample: Option<String>,
    /// With `--citizen-sample`, the row talks: lines over their heads at
    /// 1 s, 3 s and 5 s, a long one among them (Commons S1.2). For captures
    #[arg(long, requires = "citizen_sample")]
    talk_sample: bool,
}

/// A time of day for `--time-of-day`: 0 to 1, 1 being midnight again.
fn phase(s: &str) -> Result<f64, String> {
    match s.parse::<f64>() {
        Ok(t) if (0.0..=1.0).contains(&t) => Ok(t),
        _ => Err(format!("{s} isn't a time of day from 0 to 1")),
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,capitol=info,capitol_core=info")
            }),
        )
        .init();
    let mut args = Args::parse();
    if let Some(dir) = &args.write_sounds {
        sound::library::write_all(dir)
            .with_context(|| format!("writing the sounds into {}", dir.display()))?;
        println!("wrote {} sounds into {}", sound::library::RECIPES.len(), dir.display());
        return Ok(());
    }
    enter_game_dir(&mut args)?;
    let (terrain, session) = match &args.offline {
        Some(path) => (terrain::TerrainSource::offline(path)?, None),
        None => {
            let file = session::SessionFile::load(session::SessionFile::PATH);
            let server = match &args.server {
                Some(s) => session::normalize_server(s)
                    .with_context(|| format!("--server {s:?} isn't a server URL"))?,
                None => file
                    .server()
                    .unwrap_or_else(|| session::DEFAULT_SERVER.to_owned()),
            };
            let session = session::Session::start(&server, file)?;
            (terrain::TerrainSource::online(&server), Some(session))
        }
    };

    // Engine schemas first, then the game's own layered over them.
    let schema_dirs: Vec<String> = ["engine/schemas", "schemas"]
        .into_iter()
        .filter(|d| Path::new(d).is_dir())
        .map(String::from)
        .collect();
    let dir_refs: Vec<&str> = schema_dirs.iter().map(String::as_str).collect();
    let registry = flint_schema::SchemaRegistry::load_from_directories(&dir_refs)
        .context("loading schemas (run from flint-client/)")?;
    let (world, scene_file) = flint_scene::load_scene(&args.scene, &registry)
        .with_context(|| format!("loading scene {}", args.scene))?;

    let mut player = PlayerApp::new(
        world,
        args.scene.clone(),
        false,
        None,
        scene_file.scene.input_config.clone(),
    );
    player.window_size = (args.width, args.height);
    player.scene_camera = scene_file.camera.clone();
    player.scene_post_process = scene_file.post_process.clone();
    if let Some(env) = &scene_file.environment {
        player.skybox_path = env.skybox.clone();
        if env.ambient_sky.is_some() || env.ambient_ground.is_some() {
            player.scene_ambient = Some((
                env.ambient_sky
                    .unwrap_or(flint_render::LightUniforms::DEFAULT_AMBIENT_SKY),
                env.ambient_ground
                    .unwrap_or(flint_render::LightUniforms::DEFAULT_AMBIENT_GROUND),
            ));
        }
    }
    player.set_schema_paths(schema_dirs);
    let screenshot = args.screenshot.or_else(|| {
        (args.screenshot_at.is_some() || args.screenshot_every.is_some())
            .then(|| PathBuf::from("renders/capitol.png"))
    });
    player.capture = flint_player::CaptureConfig::new(
        screenshot,
        args.screenshot_at.unwrap_or(0.0),
        args.screenshot_every,
        args.exit_after,
    );
    player.clock.frame_step = args.frame_step.filter(|s| *s > 0.0);
    if let Some(path) = &args.input_script {
        player.capture.input_script = Some(flint_player::InputScript::load(path)?);
    }
    player.capture.show_stats = args.stats;

    let shared = script_api::Shared {
        projector: Some(player.projector()),
        ..Default::default()
    };
    *shared.note.lock().unwrap() = args.note;
    *shared.view.lock().unwrap() = args.view;
    if let Some(path) = &args.camera_path {
        *shared.camera_path.lock().unwrap() = Some(camera_path::CameraPath::load(path)?);
    }
    shared
        .clean
        .store(args.clean, std::sync::atomic::Ordering::Relaxed);
    let mut extension = extension::CapitolExtension::new(shared, terrain, session)
        .with_icon_sheet(args.icon_sheet)
        .with_containers(args.containers)
        .with_place(args.place)
        .with_exchange(args.exchange.as_deref())
        .with_market(args.market.as_deref(), args.market_sample)
        .with_hall_sample(args.hall_sample && args.offline.is_some())
        .with_dev_sample(args.dev_sample && args.offline.is_some())
        .with_bazaar_sample(args.bazaar_sample.as_deref().filter(|_| args.offline.is_some()))
        .with_gazette_sample(args.gazette_sample && args.offline.is_some())
        .with_zone_card_sample(args.zone_card_sample.as_deref().filter(|_| args.offline.is_some()))
        .with_deeds_sample(args.deeds_sample.as_deref().filter(|_| args.offline.is_some()))
        .with_card_sample(args.card_sample.as_deref().filter(|_| args.offline.is_some()))
        .with_mail_sample(args.mail_sample.as_deref().filter(|_| args.offline.is_some()))
        .with_impound_sample(args.impound_sample && args.offline.is_some())
        .with_chat_sample(args.chat_sample && args.offline.is_some())
        .with_hints_sample(args.hints_sample.as_deref().filter(|_| args.offline.is_some()))
        .with_station_panel_sample(args.station_panel_sample.as_deref())
        .with_mute(args.mute || args.exit_after.is_some())
        .with_juice_sample(args.juice_sample && args.offline.is_some())
        .with_time_of_day(args.time_of_day)
        .with_claim_sample(args.claim_sample, args.grow_at)
        .with_station_sample(args.station_sample)
        .with_automation_sample(args.automation_sample)
        .with_building_sample(args.building_sample)
        .with_node_sample(args.node_sample)
        .with_citizen_sample(args.citizen_sample.as_deref())
        .with_talk_sample(args.talk_sample)
        .with_trade_offer(args.trade_offer.as_deref())
        .with_controls(args.controls.or_else(|| {
            let capture = args.offline.is_some()
                || args.exit_after.is_some()
                || args.input_script.is_some();
            capture.then_some(settings::ControlMode::Click)
        }))
        .with_settings(
            settings::SettingsFile::load(
                args.settings_file
                    .as_deref()
                    .unwrap_or(settings::Settings::PATH),
            ),
            args.settings_panel,
        );
    if args.ui_sample {
        extension = extension.with_ui_sample();
    }
    player.add_extension(Box::new(extension));

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut player)?;
    Ok(())
}

/// Every path the game and the engine read (scenes/, scripts/, save/, ...)
/// is relative to the working directory. In development that's
/// flint-client/. The packaged game (package.ps1, P8.7) has its data beside
/// the exe, and may be started from anywhere: a shortcut, a terminal in
/// another folder. So when the scene isn't found here but is beside the
/// exe, move there, taking the paths given on the command line along.
fn enter_game_dir(args: &mut Args) -> Result<()> {
    if Path::new(&args.scene).exists() {
        return Ok(());
    }
    let exe = std::env::current_exe().context("finding the exe")?;
    let Some(dir) = exe.parent().filter(|d| d.join(&args.scene).exists()) else {
        return Ok(()); // loading the scene will say what's missing
    };
    fn absolute(p: &mut PathBuf) {
        if let Ok(a) = std::path::absolute(&*p) {
            *p = a;
        }
    }
    fn absolute_str(s: &mut String) {
        if let Ok(a) = std::path::absolute(&*s) {
            *s = a.to_string_lossy().into_owned();
        }
    }
    args.offline.iter_mut().for_each(absolute);
    args.screenshot.iter_mut().for_each(absolute);
    args.camera_path.iter_mut().for_each(absolute);
    args.input_script.iter_mut().for_each(absolute_str);
    args.settings_file.iter_mut().for_each(absolute_str);
    std::env::set_current_dir(dir)
        .with_context(|| format!("moving to the game's folder {}", dir.display()))?;
    tracing::info!("running from {}", dir.display());
    Ok(())
}
