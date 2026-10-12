//! The Capitol client. For now this is the P0.4 embedding spike: it builds a
//! `flint_player::PlayerApp`, runs it inside its own `ApplicationHandler`
//! (forwarding every callback, as `flint-android` does), and installs the
//! spike's Rhai function, meshes and egui window. P1's `PlayerExtension`
//! replaces the `spike_*` hooks.

mod coords;
mod spike;

use anyhow::{Context, Result};
use clap::Parser;
use flint_player::PlayerApp;
use std::path::{Path, PathBuf};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

#[derive(Parser)]
#[command(name = "capitol", about = "The Capitol, as a Flint game")]
struct Args {
    /// Scene file, relative to flint-client/
    #[arg(long, default_value = "scenes/spike.scene.toml")]
    scene: String,

    /// Text to start the inscription box with
    #[arg(long, default_value = "")]
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

/// Owns the player and forwards every winit callback to it. This is where
/// the Capitol's own handling (network drain, UI routing) will sit.
struct CapitolApp {
    player: PlayerApp,
}

impl ApplicationHandler for CapitolApp {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.player.new_events(event_loop, cause);
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.player.resumed(event_loop);
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ()) {
        self.player.user_event(event_loop, event);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.player.window_event(event_loop, id, event);
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.player.device_event(event_loop, id, event);
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.player.about_to_wait(event_loop);
    }
    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.player.suspended(event_loop);
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.player.exiting(event_loop);
    }
    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.player.memory_warning(event_loop);
    }
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
                env.ambient_sky.unwrap_or(flint_render::LightUniforms::DEFAULT_AMBIENT_SKY),
                env.ambient_ground.unwrap_or(flint_render::LightUniforms::DEFAULT_AMBIENT_GROUND),
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

    // The spike's three integration points.
    let shared = spike::Shared::default();
    *shared.note.lock().unwrap() = args.note;
    spike::register_script_api(player.script.rhai_engine_mut(), &shared);
    player.spike_on_init = Some(Box::new(spike::upload_meshes));
    player.spike_ui = Some(Box::new(spike::ui(shared)));

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut CapitolApp { player })?;
    Ok(())
}
