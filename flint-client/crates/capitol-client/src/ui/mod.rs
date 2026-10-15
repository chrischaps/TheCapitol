//! The Capitol's egui UI, drawn through `PlayerExtension::ui` (plan §4.3).
//!
//! - [`theme`]: the Capitol's look, which every panel takes its colours,
//!   type and frames from (P5.1).
//! - [`login`]: the login form over the live world (P4.1).
//! - [`icons`]: item icons by `item_type`, and the `--icon-sheet` review (P5.2).
//! - [`sample`]: a panel showing the theme's parts, for reviewing the theme
//!   against the React client (`--ui-sample`).
//! - [`account`]: who's signed in, and Sign out, top right.
//! - [`inscription`]: the spike's window, whose text box writes the note the
//!   spike script draws over the Capitol. It exercises the game egui layer.
//!   Typing in it must not reach the script's P binding, and clicks outside
//!   it must still reach the script. P4 replaces it.

pub mod icons;
pub mod login;
pub mod sample;
pub mod theme;

use crate::script_api::Shared;
use capitol_core::net::ConnectionState;
use theme::{DIM, ERROR, SUCCESS};

/// Longest inscription the box accepts.
const NOTE_MAX_CHARS: usize = 32;

/// The inscription window, top left, with where the terrain came from
/// (`status`) underneath.
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

/// Who's signed in, and the way out, top right. The connection overlay and
/// header strip (P4.5) take this over. Returns whether Sign out was clicked.
pub fn account(ctx: &egui::Context, who: &str, connection: Option<&ConnectionState>) -> bool {
    let mut sign_out = false;
    egui::Area::new(egui::Id::new("capitol_account"))
        .anchor(egui::Align2::RIGHT_TOP, [-16.0, 16.0])
        .show(ctx, |ui| {
            theme::panel()
                .inner_margin(egui::Margin::symmetric(14.0, 8.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (colour, hover) = match connection {
                            Some(ConnectionState::Connected { .. }) => (SUCCESS, "connected"),
                            Some(ConnectionState::Reconnecting { .. }) => (ERROR, "reconnecting…"),
                            _ => (DIM, "connecting…"),
                        };
                        theme::status_dot(ui, colour, hover);
                        ui.label(
                            egui::RichText::new(who)
                                .font(theme::strong(theme::size::BODY))
                                .color(theme::GOLD),
                        );
                        ui.add_space(8.0);
                        sign_out = theme::link(ui, "Sign out", theme::size::SMALL).clicked();
                    });
                });
        });
    sign_out
}
