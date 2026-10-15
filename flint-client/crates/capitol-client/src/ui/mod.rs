//! The Capitol's egui UI, drawn through `PlayerExtension::ui` (plan §4.3).
//!
//! - [`theme`]: the Capitol's look, which every panel takes its colours,
//!   type and frames from (P5.1).
//! - [`login`]: the login form over the live world (P4.1).
//! - [`icons`]: item icons by `item_type`, and the `--icon-sheet` review (P5.2).
//! - [`slots`]: the slot grid, with drag and drop between containers (P5.3).
//! - [`inventory`]: the inventory panel, toggled with `I` (P5.4).
//! - [`containers`]: `--containers`, the containers no panel shows yet, as
//!   grids, until the crafting and station panels take them over.
//! - [`sample`]: a panel showing the theme's parts, for reviewing the theme
//!   against the React client (`--ui-sample`).
//! - [`hud`]: the header strip (zone, strands, who, Sign out) and the
//!   overlay while the connection is down (P4.5).
//! - [`inscription`]: the spike's window, whose text box writes the note the
//!   spike script draws over the Capitol. It exercises the game egui layer.
//!   Typing in it must not reach the script's P binding, and clicks outside
//!   it must still reach the script. Offline only now: the login form
//!   tests the same thing (P4.1).
//! - [`controls`]: once signed in, the inscription's place is a quiet
//!   reminder of the controls, bottom left, clear of the header, the zone
//!   banner and the toasts (P4.5).

pub mod containers;
pub mod hud;
pub mod icons;
pub mod inventory;
pub mod login;
pub mod sample;
pub mod slots;
pub mod theme;

use crate::script_api::Shared;

/// Longest inscription the box accepts.
const NOTE_MAX_CHARS: usize = 32;

/// The controls, and where the terrain came from (`status`), small and dim
/// at the bottom left. Not interactable, so clicks reach the world.
pub fn controls(ctx: &egui::Context, status: &str) {
    egui::Area::new(egui::Id::new("capitol_controls"))
        .anchor(egui::Align2::LEFT_BOTTOM, [16.0, -14.0])
        .interactable(false)
        .show(ctx, |ui| {
            theme::hint().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for line in [
                    "Click the ground to walk, or grass to gather",
                    "WASD pans · H home",
                    "Wheel or R/F zooms · Q/E or right-drag turns",
                    status,
                ] {
                    ui.label(theme::dim(line).size(theme::size::LABEL));
                }
            });
        });
}

/// The inscription window, top left, with where the terrain came from
/// (`status`) underneath. Offline only.
pub fn inscription(ctx: &egui::Context, shared: &Shared, status: &str) {
    egui::Window::new("inscription")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::LEFT_TOP, [16.0, 16.0])
        .frame(theme::panel())
        .show(ctx, |ui| {
            ui.label(theme::title("Inscribe the Capitol", theme::size::BODY));
            ui.add_space(8.0);
            let Ok(mut note) = shared.note.lock() else {
                return;
            };
            ui.add(
                egui::TextEdit::singleline(&mut *note)
                    .hint_text("a name, a motto…")
                    .char_limit(NOTE_MAX_CHARS)
                    .desired_width(240.0),
            );
            ui.add_space(6.0);
            ui.label(theme::dim(
                "WASD pans · wheel or R/F zooms · Q/E or right-drag turns · H home",
            ));
            ui.label(theme::dim(
                "P traces the plan · click the ground to walk there",
            ));
            ui.label(theme::dim(status));
        });
}
