//! P0.4 embedding spike (throwaway). It proves three integration points
//! before P1 designs the real hooks:
//!
//! - a native Rhai function (`cap_spike_note`) called from `scripts/spike.rhai`
//! - procedural meshes uploaded after init: the moat and the two canals with
//!   their real bridge gaps, over zone-tinted ground
//! - a game egui window whose text box takes keyboard input
//!
//! The pieces are joined up: what you type in the egui box is Rust state,
//! the script reads it through the Rhai function, and the HUD writes it over
//! the Capitol. The patches this needs in the engine subtree are marked
//! `SPIKE(P0.4)`.

use crate::coords::polar_to_world;
use flint_render::Vertex;
use flint_render::SceneRenderer;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// State shared by the egui window (writer) and the Rhai function (reader).
#[derive(Clone, Default)]
pub struct Shared {
    pub note: Arc<Mutex<String>>,
    pub script_calls: Arc<AtomicU64>,
}

// ---- Rhai ----------------------------------------------------------------

/// `cap_spike_note() -> String`: the text in the egui box.
pub fn register_script_api(engine: &mut rhai::Engine, shared: &Shared) {
    let shared = shared.clone();
    engine.register_fn("cap_spike_note", move || -> String {
        shared.script_calls.fetch_add(1, Ordering::Relaxed);
        shared.note.lock().map(|s| s.clone()).unwrap_or_default()
    });
}

// ---- Meshes --------------------------------------------------------------

/// The backend's default rings (`TerrainConfig::from_params`): radius,
/// width and bridge angles, in server units. P3 reads these from `/terrain`.
const WATER_RINGS: &[(f64, f64, f64, &[f64])] = &[
    // (radius, width, bridge width, bridge angles)
    (200.0, 20.0, 30.0, &[90.0]),                                            // moat
    (600.0, 15.0, 25.0, &[270.0, 0.0, 90.0, 180.0]),                         // canal_inner
    (1000.0, 15.0, 25.0, &[270.0, 315.0, 0.0, 45.0, 90.0, 135.0, 180.0, 225.0]), // canal_outer
];

/// An sRGB hex colour (the React client's tokens) as the linear RGBA the
/// renderer expects: the surface is sRGB, so vertex colours are linear.
fn rgb(r: u8, g: u8, b: u8) -> [f32; 4] {
    let lin = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    [lin(r), lin(g), lin(b), 1.0]
}

fn hex(h: u32) -> [f32; 4] {
    rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// A zone colour pulled most of the way toward the background, so the
/// ground reads as tinted dark rather than as a flat fill.
fn ground_tint(c: [f32; 4], amount: f32) -> [f32; 4] {
    let bg = hex(0x0a0a0f);
    let m = |i: usize| bg[i] + (c[i] - bg[i]) * amount;
    [m(0), m(1), m(2), 1.0]
}

#[derive(Default)]
struct MeshBuf {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl MeshBuf {
    /// A flat ring at height `y`, from `r_in` to `r_out` server units,
    /// skipping each `(centre_deg, half_width_deg)` gap. `r_in = 0` gives a
    /// disc. Triangles wind counter-clockwise seen from above (+Y).
    fn annulus(&mut self, r_in: f64, r_out: f64, y: f32, color: [f32; 4], gaps: &[(f64, f64)]) {
        const SEGMENTS: usize = 360;
        let in_gap = |deg: f64| {
            gaps.iter().any(|&(c, h)| {
                let d = (deg - c + 540.0).rem_euclid(360.0) - 180.0;
                d.abs() < h
            })
        };
        let vertex = |r: f64, deg: f64| {
            let [x, z] = polar_to_world(r, deg);
            Vertex { position: [x, y, z], normal: [0.0, 1.0, 0.0], color, uv: [0.0, 0.0] }
        };
        for i in 0..SEGMENTS {
            let a0 = i as f64 * 360.0 / SEGMENTS as f64;
            let a1 = (i + 1) as f64 * 360.0 / SEGMENTS as f64;
            if in_gap((a0 + a1) * 0.5) {
                continue;
            }
            let base = self.vertices.len() as u32;
            self.vertices.extend([vertex(r_in, a0), vertex(r_out, a0), vertex(r_in, a1), vertex(r_out, a1)]);
            let (i0, o0, i1, o1) = (base, base + 1, base + 2, base + 3);
            self.indices.extend([i0, o1, o0, i0, i1, o1]);
        }
    }

    fn upload(self, renderer: &mut SceneRenderer, device: &wgpu::Device, name: &str, roughness: f32) {
        let material = flint_import::ImportedMaterial {
            name: name.to_string(),
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness,
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            use_vertex_color: true,
            alpha_mode: flint_import::AlphaMode::Opaque,
            alpha_cutoff: 0.5,
        };
        renderer.load_procedural_mesh(device, name, &self.vertices, &self.indices, material);
    }
}

/// Upload the spike meshes. The scene's entities reference them by name
/// through `model.asset`.
pub fn upload_meshes(renderer: &mut SceneRenderer, device: &wgpu::Device) {
    use capitol_protocol::rules::Zone;
    // Ground: each zone tinted toward the dark, fading out to the edge.
    let mut ground = MeshBuf::default();
    for (zone, colour, amount) in [
        (Zone::Capitol, 0xc9a227, 0.40),
        (Zone::TradeDistrict, 0x3b82f6, 0.10),
        (Zone::GuildDistrict, 0x8b5cf6, 0.09),
        (Zone::Urban, 0x64748b, 0.10),
        (Zone::Suburban, 0x22c55e, 0.05),
        (Zone::Rural, 0x84cc16, 0.03),
        (Zone::Wilderness, 0x1a1a24, 1.0),
    ] {
        let (r_in, r_out) = zone.radii();
        ground.annulus(r_in, r_out, 0.0, ground_tint(hex(colour), amount), &[]);
    }
    ground.upload(renderer, device, "spike_ground", 0.9);

    // Water, a hair above the ground, open at each bridge. The React water
    // colour reads too close to the Trade tint once lit, so this sits between
    // it and the highlight rgba(100,180,220).
    let mut water = MeshBuf::default();
    for &(radius, width, bridge_width, angles) in WATER_RINGS {
        let half_gap = ((bridge_width * 0.5) / radius).asin().to_degrees();
        let gaps: Vec<(f64, f64)> = angles.iter().map(|&a| (a, half_gap)).collect();
        water.annulus(radius - width * 0.5, radius + width * 0.5, 0.02, rgb(70, 150, 200), &gaps);
    }
    water.upload(renderer, device, "spike_water", 0.25);

    // Compass markers for checking handedness: east (angle 0) in the
    // in-range orange, north (angle 270) in the text colour.
    for (name, angle, color) in [
        ("spike_marker_east", 0.0, rgb(0xf9, 0x73, 0x16)),
        ("spike_marker_north", 270.0, rgb(0xe0, 0xe0, 0xe0)),
    ] {
        let mut marker = MeshBuf::default();
        let [cx, cz] = polar_to_world(400.0, angle);
        marker.annulus(0.0, 30.0, 0.04, color, &[]);
        for v in &mut marker.vertices {
            v.position[0] += cx;
            v.position[2] += cz;
        }
        marker.upload(renderer, device, name, 0.6);
    }
}

// ---- egui ----------------------------------------------------------------

/// The Capitol's panel style (plan §4.3), applied to one window.
fn capitol_frame() -> egui::Frame {
    egui::Frame::window(&egui::Style::default())
        .fill(egui::Color32::from_rgba_unmultiplied(15, 15, 25, 242))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(80, 80, 100, 77)))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(14.0))
}

const GOLD: egui::Color32 = egui::Color32::from_rgb(0xc9, 0xa2, 0x27);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe0, 0xe0, 0xe0);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x80, 0x80, 0x80);

/// The game egui callback: one window with a text box.
pub fn ui(shared: Shared) -> impl FnMut(&egui::Context) {
    let mut focused_once = false;
    move |ctx| {
        egui::Window::new("inscription")
            .title_bar(false)
            .resizable(false)
            .anchor(egui::Align2::LEFT_TOP, [16.0, 16.0])
            .frame(capitol_frame())
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("INSCRIBE THE CAPITOL").color(GOLD).size(13.0).strong());
                ui.add_space(6.0);
                let mut note = shared.note.lock().unwrap();
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut *note)
                        .hint_text("a name, a motto…")
                        .text_color(TEXT)
                        .desired_width(240.0),
                );
                if !focused_once {
                    edit.request_focus();
                    focused_once = true;
                }
                drop(note);
                ui.add_space(4.0);
                let calls = shared.script_calls.load(Ordering::Relaxed);
                ui.label(
                    egui::RichText::new(format!("read by Rhai via cap_spike_note() · {calls} calls"))
                        .color(DIM)
                        .size(11.0),
                );
            });
    }
}
