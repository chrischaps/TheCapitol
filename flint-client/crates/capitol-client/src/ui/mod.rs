//! The Capitol's egui UI, drawn through `PlayerExtension::ui` (plan §4.3).
//!
//! - [`login`]: the login form over the live world (P4.1).
//! - [`account`]: who's signed in, and Sign out, top right.
//! - [`inscription`]: the spike's window, whose text box writes the note the
//!   spike script draws over the Capitol. It exercises the game egui layer.
//!   Typing in it must not reach the script's P binding, and clicks outside
//!   it must still reach the script. P4 replaces it.

pub mod login;

use crate::script_api::Shared;
use capitol_core::net::ConnectionState;
use egui::{Color32, RichText};

// The Capitol tokens (plan §4.3), shared with the React client.
const PANEL_FILL: Color32 = Color32::from_rgba_premultiplied(14, 14, 24, 242); // rgba(15,15,25,0.95)
const PANEL_BORDER: Color32 = Color32::from_rgba_premultiplied(24, 24, 30, 77); // rgba(80,80,100,0.3)
const GOLD: Color32 = Color32::from_rgb(0xc9, 0xa2, 0x27);
const TEXT: Color32 = Color32::from_rgb(0xe0, 0xe0, 0xe0);
const DIM: Color32 = Color32::from_rgb(0x80, 0x80, 0x90);
const GOLD_DIM: Color32 = Color32::from_rgb(0x8a, 0x70, 0x19);
const BG_DARK: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0f);
const ERROR: Color32 = Color32::from_rgb(0xdc, 0x35, 0x45);
const ONLINE: Color32 = Color32::from_rgb(0x28, 0xa7, 0x45);

/// Longest inscription the box accepts.
const NOTE_MAX_CHARS: usize = 32;

fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(PANEL_FILL)
        .stroke(egui::Stroke::new(1.0, PANEL_BORDER))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(14.0))
}

/// The inscription window, top left, with where the terrain came from
/// (`status`) underneath.
pub fn inscription(ctx: &egui::Context, shared: &Shared, status: &str) {
    egui::Window::new("inscription")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::LEFT_TOP, [16.0, 16.0])
        .frame(panel_frame())
        .show(ctx, |ui| {
            ui.label(
                RichText::new("INSCRIBE THE CAPITOL")
                    .color(GOLD)
                    .size(12.0)
                    .extra_letter_spacing(1.5)
                    .strong(),
            );
            ui.add_space(8.0);
            let Ok(mut note) = shared.note.lock() else {
                return;
            };
            ui.add(
                egui::TextEdit::singleline(&mut *note)
                    .hint_text("a name, a motto…")
                    .text_color(TEXT)
                    .char_limit(NOTE_MAX_CHARS)
                    .desired_width(240.0),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("WASD moves · wheel or R/F zooms · Q/E or right-drag turns · H home")
                    .color(DIM)
                    .size(11.0),
            );
            ui.label(
                RichText::new("P traces the plan · click the city to mark it")
                    .color(DIM)
                    .size(11.0),
            );
            ui.label(RichText::new(status).color(DIM).size(11.0));
        });
}

/// Who's signed in, and the way out, top right. The connection overlay and
/// header strip (P4.5) take this over. Returns whether Sign out was clicked.
pub fn account(ctx: &egui::Context, who: &str, connection: Option<&ConnectionState>) -> bool {
    let mut sign_out = false;
    egui::Area::new(egui::Id::new("capitol_account"))
        .anchor(egui::Align2::RIGHT_TOP, [-16.0, 16.0])
        .show(ctx, |ui| {
            panel_frame()
                .inner_margin(egui::Margin::symmetric(14.0, 8.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (colour, hover) = match connection {
                            Some(ConnectionState::Connected { .. }) => (ONLINE, "connected"),
                            Some(ConnectionState::Reconnecting { .. }) => (ERROR, "reconnecting…"),
                            _ => (DIM, "connecting…"),
                        };
                        let (dot, response) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.5, colour);
                        response.on_hover_text(hover);
                        ui.label(RichText::new(who).color(GOLD).size(13.0).strong());
                        ui.add_space(8.0);
                        sign_out = ui
                            .add(
                                egui::Label::new(RichText::new("Sign out").color(DIM).size(12.0))
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked();
                    });
                });
        });
    sign_out
}
