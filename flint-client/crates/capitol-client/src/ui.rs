//! The Capitol's egui UI, drawn through `PlayerExtension::ui` (plan §4.3).
//! For now that's one window: the inscription, whose text box writes the
//! note the spike script draws over the Capitol. It exercises the game
//! egui layer. Typing in it must not reach the script's E binding, and
//! clicks outside it must still reach the script.

use crate::script_api::Shared;
use egui::{Color32, RichText};

// The Capitol tokens (plan §4.3), shared with the React client.
const PANEL_FILL: Color32 = Color32::from_rgba_premultiplied(14, 14, 24, 242); // rgba(15,15,25,0.95)
const PANEL_BORDER: Color32 = Color32::from_rgba_premultiplied(24, 24, 30, 77); // rgba(80,80,100,0.3)
const GOLD: Color32 = Color32::from_rgb(0xc9, 0xa2, 0x27);
const TEXT: Color32 = Color32::from_rgb(0xe0, 0xe0, 0xe0);
const DIM: Color32 = Color32::from_rgb(0x80, 0x80, 0x90);

/// Longest inscription the box accepts.
const NOTE_MAX_CHARS: usize = 32;

fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(PANEL_FILL)
        .stroke(egui::Stroke::new(1.0, PANEL_BORDER))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(14.0))
}

/// The inscription window, top left.
pub fn inscription(ctx: &egui::Context, shared: &Shared) {
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
                RichText::new("E traces the plan · click the city to mark it")
                    .color(DIM)
                    .size(11.0),
            );
        });
}
