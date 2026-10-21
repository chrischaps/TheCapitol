//! The Capitol's egui UI, drawn through `PlayerExtension::ui` (plan §4.3).
//!
//! - [`theme`]: the Capitol's look, which every panel takes its colours,
//!   type and frames from (P5.1).
//! - [`login`]: the login form over the live world (P4.1).
//! - [`icons`]: item icons by `item_type`, and the `--icon-sheet` review (P5.2).
//! - [`slots`]: the slot grid, with drag and drop between containers (P5.3).
//! - [`inventory`]: the inventory panel, toggled with `I` (P5.4).
//! - [`crafting`]: the crafting panel, toggled with C (P5.5).
//! - [`plot`]: the plot panel, opened by alt-clicking a plot (P6.4).
//! - [`settings`]: the settings panel, toggled with F10 (P8.4).
//! - [`trade`]: a trade request's card, and the two-sided trade window (P7.3).
//! - [`gazette`]: the Capitol Gazette, toggled with G or its notice board
//!   (E3.9): the day's edition as a page of newsprint.
//! - [`impound`]: what a foreclosure kept for you when your pack was
//!   full, opened from the header's Impound chip (#141).
//! - [`market`]: the market panel, toggled with M: items, the ladder, the
//!   order form and your orders (E3.5).
//! - [`shop`]: a bazaar stall's shop, opened at its stall: its listings
//!   to buy from, or yours to rent, stock and unlist (E3.11).
//! - [`station`]: the open station's panel: its box, and its recipes at a
//!   crafting station (P6.3).
//! - [`automation`]: the station panel's Automation tab, for its owner
//!   (E4.4).
//! - [`containers`]: `--containers`, any container no panel shows, as grids.
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
//!   pointer to the shortcut overlay, bottom left, clear of the header, the
//!   zone banner and the toasts (P4.5; one line since P8.3).
//! - [`hints`]: the first-session hints, once per account (P8.3).
//! - [`shortcuts`]: every control on one card, on `?` or F1 (P8.3).
//! - [`dev_overlay`]: the dev overlay on F7 (D5.2): frame times, the net,
//!   the prediction, the store's counts and the interest cell.

pub mod automation;
pub mod containers;
pub mod crafting;
pub mod dev_overlay;
pub mod exchange;
pub mod gazette;
pub mod hints;
pub mod hud;
pub mod icons;
pub mod impound;
pub mod inventory;
pub mod login;
pub mod market;
pub mod market_chart;
pub mod plot;
pub mod sample;
pub mod settings;
pub mod shop;
pub mod shortcuts;
pub mod slots;
pub mod station;
pub mod theme;
pub mod trade;

use crate::script_api::Shared;

/// Longest inscription the box accepts.
const NOTE_MAX_CHARS: usize = 32;

/// How tall the controls hint stands, in points, for a panel above it.
const CONTROLS_HEIGHT: f32 = 62.0;

/// Where every control is (the `?` overlay lists them all), and where the
/// terrain came from (`status`), small and dim at the bottom left. Not interactable, so clicks reach the world. It
/// steps aside while a panel (`clear_of`) reaches down into its place.
pub fn controls(ctx: &egui::Context, status: &str, clear_of: Option<egui::Rect>) {
    let floor = ctx.screen_rect().bottom() - CONTROLS_HEIGHT;
    if clear_of.is_some_and(|r| r.bottom() > floor) {
        return;
    }
    egui::Area::new(egui::Id::new("capitol_controls"))
        .anchor(egui::Align2::LEFT_BOTTOM, [16.0, -14.0])
        .interactable(false)
        .show(ctx, |ui| {
            theme::hint().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    theme::keycap(ui, "?");
                    ui.label(theme::dim("every control").size(theme::size::LABEL));
                });
                ui.label(theme::dim(status).size(theme::size::LABEL));
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
            ui.label(theme::dim("WASD pans · wheel, R/F or +/- zooms"));
            ui.label(theme::dim("Q/E or right-drag turns · H or Home comes home"));
            ui.label(theme::dim("Click the ground to walk there"));
            ui.label(theme::dim(status));
        });
}
