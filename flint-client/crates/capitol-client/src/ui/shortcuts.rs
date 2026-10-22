//! The shortcut overlay (P8.3): every control on one card, over a dimmed
//! world. `?` opens and closes it (read off the typed character in
//! `CapitolExtension::on_window_event`, so it's `?` on any layout), and so
//! does F1 (the `help` action, which `--input-script` can press) and the
//! header's `?` chip. Esc or a click outside closes it.
//!
//! A row is written as words with keys in brackets: `"[Alt] + [Click] a
//! plot"`. Each bracket is drawn as a key cap.

use super::theme::{self, size};
use egui::{Align2, Id, Order, RichText};

/// How long the overlay takes to fade in or out, in seconds.
const FADE: f32 = 0.16;
/// The keys' width in the left column and the right, then what they do.
const KEYS_LEFT: f32 = 192.0;
const KEYS_RIGHT: f32 = 160.0;
const WORDS_WIDTH: f32 = 150.0;

/// Every control, by what it's for. When a control changes, change it here
/// and in `config/input.toml`.
const GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Getting about",
        &[
            ("[Click] the ground", "Walk there"),
            ("[W] [A] [S] [D] or arrows", "Pan the view"),
            ("[Wheel], [R] [F] or [+] [-]", "Zoom in and out"),
            ("[Q] [E] or right-drag", "Turn the view"),
            ("[H] or [Home]", "Back to you"),
        ],
    ),
    (
        "Making a living",
        &[
            ("[Click] grass", "Gather fiber"),
            ("[Click] a station, or [O]", "Open it, or the nearest"),
            ("[Enter], crafting", "Craft what's set"),
            ("Right-click a kit", "Place its station"),
            ("[Esc] or right-click", "Stop placing"),
            ("[Alt] + [Click] a plot", "See its deed"),
            ("[Shift] + [Click] a player", "Ask them to trade"),
            ("[T]", "Accept, or ask the nearest"),
        ],
    ),
    (
        "Panels",
        &[
            ("[I]", "Inventory"),
            ("[C]", "Crafting"),
            ("[X]", "Fiber exchange"),
            ("[M]", "The market"),
            ("[G]", "The Gazette"),
            ("[Enter]", "Talk: Enter sends, Esc closes"),
            ("[L]", "Your letters"),
            ("[F10]", "Settings"),
            ("[F8]", "Point and walk, or over the shoulder"),
            ("[F7]", "Dev overlay"),
            ("[F6]", "World layers (admins)"),
            ("[?] or [F1]", "This list"),
        ],
    ),
];

#[derive(Default)]
pub struct Shortcuts {
    pub open: bool,
}

impl Shortcuts {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Draw it while it's open or fading. True if a click outside asked to
    /// close it.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let k = ctx.animate_bool_with_time(Id::new("capitol_shortcuts"), self.open, FADE);
        if k <= 0.0 {
            return false;
        }
        let screen = ctx.screen_rect();
        // The scrim takes clicks, so one outside the card closes it rather
        // than walking you somewhere.
        let scrim = egui::Area::new(Id::new("capitol_shortcuts_scrim"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(self.open)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, theme::SCRIM.gamma_multiply(k));
                response
            })
            .inner;
        let card = egui::Area::new(Id::new("capitol_shortcuts"))
            .order(Order::Tooltip)
            .anchor(Align2::CENTER_CENTER, [0.0, 8.0 * (1.0 - k)])
            .interactable(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(k);
                theme::panel_edged(theme::GOLD).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(theme::title("Controls", size::TITLE * 0.75));
                    });
                    ui.add_space(4.0);
                    theme::gold_rule(ui);
                    ui.add_space(10.0);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 28.0;
                        // Getting about and the panels on the left, the
                        // work on the right: two columns of about equal height.
                        ui.vertical(|ui| {
                            group(ui, GROUPS[0], KEYS_LEFT);
                            ui.add_space(12.0);
                            group(ui, GROUPS[2], KEYS_LEFT);
                        });
                        ui.vertical(|ui| group(ui, GROUPS[1], KEYS_RIGHT));
                    });
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        ui.label(theme::dim("? or Esc closes this"));
                    });
                });
            })
            .response
            .rect;
        // The card isn't interactable, so a click on it reaches the scrim.
        let on_card = ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| card.contains(p));
        self.open && scrim.clicked() && !on_card
    }
}

/// A group's name, then its rows: the keys in a column `keys_width` wide,
/// so what they do lines up.
fn group(ui: &mut egui::Ui, (name, rows): (&str, &[(&str, &str)]), keys_width: f32) {
    ui.label(theme::label(name));
    ui.add_space(4.0);
    for (keys, words) in rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.allocate_ui_with_layout(
                egui::vec2(keys_width, 22.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(keys_width);
                    keys_row(ui, keys);
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(WORDS_WIDTH, 22.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(WORDS_WIDTH);
                    ui.label(theme::text(*words).size(size::BODY));
                },
            );
        });
        ui.add_space(2.0);
    }
}

/// Words with `[Key]`s in them, the keys as caps and the words dim.
pub fn keys_row(ui: &mut egui::Ui, spec: &str) {
    for (cap, part) in parse(spec) {
        if cap {
            theme::keycap(ui, part);
        } else {
            ui.label(RichText::new(part).color(theme::DIM).size(size::SMALL));
        }
    }
}

/// `spec` split into keys (`true`) and the words between them, trimmed,
/// with empty pieces dropped.
fn parse(spec: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut rest = spec;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|c| open + c) else {
            break;
        };
        out.push((false, rest[..open].trim()));
        out.push((true, &rest[open + 1..close]));
        rest = &rest[close + 1..];
    }
    out.push((false, rest.trim()));
    out.retain(|(_, s)| !s.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn a_row_splits_into_keys_and_words() {
        assert_eq!(
            parse("[Alt] + [Click] a plot"),
            vec![
                (true, "Alt"),
                (false, "+"),
                (true, "Click"),
                (false, "a plot")
            ]
        );
        assert_eq!(
            parse("Right-click a kit"),
            vec![(false, "Right-click a kit")]
        );
        assert_eq!(parse("[H]"), vec![(true, "H")]);
    }
}
