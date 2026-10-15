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
#[cfg(test)]
mod camera_rig;
mod coords;
mod entities;
mod extension;
mod nodes;
mod script_api;
mod session;
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
    /// zooms. The rest are fixed shots, held until the first camera input:
    /// bridge (close, over the inner canal's south crossing), suburb (low
    /// over the southern suburbs' curving streets), urban (close over the
    /// urban lots by the south arterial), capitol (up at the tower from
    /// beyond the moat), crown (level with its lantern), or from-urban,
    /// from-suburban, from-rural and from-wilderness (the tower on the
    /// horizon from each outer zone)
    #[arg(long, default_value = "street", value_parser = [
        "street", "district", "map", "bridge", "suburb", "urban", "capitol", "crown",
        "from-urban", "from-suburban", "from-rural", "from-wilderness",
    ])]
    view: String,

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
    /// Timed key input (flint-player's input-script format)
    #[arg(long, value_name = "FILE")]
    input_script: Option<String>,

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
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,capitol=info,capitol_core=info")
            }),
        )
        .init();
    let args = Args::parse();
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

    let shared = script_api::Shared::default();
    *shared.note.lock().unwrap() = args.note;
    *shared.view.lock().unwrap() = args.view;
    let mut extension = extension::CapitolExtension::new(shared, terrain, session)
        .with_icon_sheet(args.icon_sheet)
        .with_containers(args.containers);
    if args.ui_sample {
        extension = extension.with_ui_sample();
    }
    player.add_extension(Box::new(extension));

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut player)?;
    Ok(())
}
