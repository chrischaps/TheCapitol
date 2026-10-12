//! The Capitol client. It builds a `flint_player::PlayerApp` for the scene
//! and installs `CapitolExtension`, through which everything the Capitol
//! adds reaches the engine. For now it runs the P0.4 spike scene with its
//! native Rhai function and the inscription window (game egui). The spike's
//! meshes come back through the runtime mesh hook in P1.4 (they're on
//! branch `spike/p0.4` until then).

#[allow(dead_code)] // used once the world is built (P3)
mod coords;
mod extension;
mod script_api;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use flint_player::PlayerApp;
use std::path::{Path, PathBuf};
use winit::event_loop::{ControlFlow, EventLoop};

#[derive(Parser)]
#[command(name = "capitol", about = "The Capitol, as a Flint game")]
struct Args {
    /// Scene file, relative to flint-client/
    #[arg(long, default_value = "scenes/spike.scene.toml")]
    scene: String,

    /// Text written over the Capitol
    #[arg(long, default_value = "The Capitol")]
    note: String,

    #[arg(long, default_value_t = 1280)]
    width: u32,
    #[arg(long, default_value_t = 720)]
    height: u32,

    /// Write a PNG of the presented frame (scene and egui)
    #[arg(long, value_name = "PATH")]
    screenshot: Option<PathBuf>,
    /// Game time in seconds for the screenshot
    #[arg(long, default_value_t = 0.0, value_name = "SECS")]
    screenshot_at: f64,
    /// Exit once game time reaches this many seconds
    #[arg(long, value_name = "SECS")]
    exit_after: Option<f64>,
    /// Timed key input (flint-player's input-script format)
    #[arg(long, value_name = "FILE")]
    input_script: Option<String>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let args = Args::parse();

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
    player.capture = flint_player::CaptureConfig::new(
        args.screenshot,
        args.screenshot_at,
        None,
        args.exit_after,
    );
    if let Some(path) = &args.input_script {
        player.capture.input_script = Some(flint_player::InputScript::load(path)?);
    }

    let shared = script_api::Shared::default();
    *shared.note.lock().unwrap() = args.note;
    player.add_extension(Box::new(extension::CapitolExtension::new(shared)));

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut player)?;
    Ok(())
}
