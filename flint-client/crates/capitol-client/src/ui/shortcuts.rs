//! The shortcut overlay (P8.3): every control on one card, over a dimmed
//! world. `?` opens and closes it (read off the typed character in
//! `CapitolExtension::on_window_event`, so it's `?` on any layout), and so
//! does F1 (the `help` action, which `--input-script` can press) and the
//! header's `?` chip. Esc or a click outside closes it. While the road
//! in's card is hidden (#441), a line under the work brings it back.
//!
//! A row is written as words with keys in brackets: `"[Alt] + [Click] a
//! plot"`. Each bracket is drawn as a key cap.
//!
//! The card is the mode's (T1.6): point and walk, where a click walks and
//! WASD pans, or over the shoulder, where WASD walks, the mouse looks and E
//! acts. Only getting about and the work differ; the panels are the same.

use super::theme::{self, size};
use egui::{Align2, Id, Order, RichText};

/// How long the overlay takes to fade in or out, in seconds.
const FADE: f32 = 0.16;
/// The keys' width in the left column and the right, then what they do.
const KEYS_LEFT: f32 = 192.0;
const KEYS_RIGHT: f32 = 160.0;
const WORDS_WIDTH: f32 = 150.0;
/// The card's height less its columns', and a margin round it: what the
/// columns may not take of the screen.
const CHROME: f32 = 140.0;

/// A group of controls: its name, then each row's keys and what they do.
type Group = (&'static str, &'static [(&'static str, &'static str)]);

/// Every control, by what it's for, in point and walk: getting about, the
/// work and the panels. When a control changes, change it here (and in
/// [`DIRECT`] if it's there too) and in `config/input.toml`.
const CLICK: [Group; 3] = [
    (
        "Getting about",
        &[
            ("[Click] the ground", "Walk there"),
            ("[W] [A] [S] [D] or arrows", "Pan the view"),
            ("[Wheel], [R] [F] or [+] [-]", "Zoom in and out"),
            ("[Q] [E] or right-drag", "Turn the view"),
            ("[H] or [Home]", "Back to you"),
            ("[F8]", "Over the shoulder"),
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
    PANELS,
];

/// The same over the shoulder (Direct, T1.6). E, or a click while the
/// cursor is captured, acts on what's in front of you, as a click on it
/// does in point and walk.
const DIRECT: [Group; 3] = [
    (
        "Getting about",
        &[
            ("[W] [A] [S] [D] or arrows", "Walk"),
            ("Mouse", "Look"),
            ("[E] or [Click]", "Act on what's ahead"),
            ("[Alt]", "Free the cursor"),
            ("[Esc]", "Close a panel, or free the cursor"),
            ("[Wheel], [R] [F] or [+] [-]", "Zoom in and out"),
            ("[H] or [Home]", "Back over your shoulder"),
            ("[F8]", "Point and walk"),
        ],
    ),
    (
        "Making a living",
        &[
            ("[E] grass", "Gather fiber"),
            ("[E] a station, or [O]", "Open it, or the nearest"),
            ("[Enter], crafting", "Craft what's set"),
            ("Right-click a kit", "Place its station"),
            ("[Esc] or right-click", "Stop placing"),
            ("[Alt] + [Click] a plot", "See its deed"),
            ("[E] a player", "See their card"),
            ("[Shift] + [E] a player", "Ask them to trade"),
            ("[T]", "Accept, or ask the nearest"),
        ],
    ),
    PANELS,
];

/// The panels, the same in both modes.
const PANELS: Group = (
    "Panels",
        &[
            ("[I]", "Inventory"),
            ("[C]", "Crafting"),
            ("[X]", "Fiber exchange"),
            ("[M]", "The market"),
            ("[G]", "The Gazette"),
            ("[Enter]", "Talk: Enter sends, Esc closes"),
            ("[L]", "Your letters"),
            ("[U]", "Guilds"),
            ("[F10]", "Settings"),
            ("[F7]", "Dev overlay"),
            ("[F6]", "World layers (admins)"),
            ("[?] or [F1]", "This list"),
        ],
);

/// The mode's groups, and its name under the card's title.
fn groups(direct: bool) -> (&'static [Group; 3], &'static str) {
    if direct {
        (&DIRECT, "Over the shoulder")
    } else {
        (&CLICK, "Point and walk")
    }
}

#[derive(Default)]
pub struct Shortcuts {
    pub open: bool,
}

/// What was asked of the overlay this frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Asked {
    /// A click outside: close it.
    pub close: bool,
    /// Bring the road in's card back.
    pub road_in: bool,
}

impl Shortcuts {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Draw it while it's open or fading, for the mode in play (`direct`:
    /// over the shoulder). `road_in_hidden` adds the line that brings the
    /// road in's card back.
    pub fn show(&mut self, ctx: &egui::Context, direct: bool, road_in_hidden: bool) -> Asked {
        let (groups, mode) = groups(direct);
        let mut asked = Asked::default();
        let k = ctx.animate_bool_with_time(Id::new("capitol_shortcuts"), self.open, FADE);
        if k <= 0.0 {
            return asked;
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
            // For the road in's line and the scroll; a click elsewhere on
            // it is let be
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(k);
                theme::panel_edged(theme::GOLD).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(theme::title("Controls", size::TITLE * 0.75));
                        ui.label(theme::dim(mode));
                    });
                    ui.add_space(4.0);
                    theme::gold_rule(ui);
                    ui.add_space(10.0);
                    // A small window at a large scale (720p at 150%) has
                    // less height than the columns: they scroll then
                    let room = (screen.height() - CHROME).max(160.0);
                    egui::ScrollArea::vertical().max_height(room).show(ui, |ui| ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 28.0;
                        // Getting about and the panels on the left, the
                        // work on the right: two columns of about equal height.
                        ui.vertical(|ui| {
                            group(ui, groups[0], KEYS_LEFT);
                            ui.add_space(12.0);
                            group(ui, groups[2], KEYS_LEFT);
                        });
                        ui.vertical(|ui| {
                            group(ui, groups[1], KEYS_RIGHT);
                            if road_in_hidden {
                                ui.add_space(12.0);
                                ui.label(theme::label("The road in"));
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(theme::text("Its card is hidden for now.").size(size::BODY));
                                    asked.road_in = theme::link(ui, "Bring it back", size::BODY).clicked();
                                });
                            }
                        });
                    }));
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        ui.label(theme::dim("? or Esc closes this"));
                    });
                });
            })
            .response
            .rect;
        // A click on the card is the card's, not the scrim's.
        let on_card = ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| card.contains(p));
        asked.close = self.open && scrim.clicked() && !on_card;
        asked
    }
}

/// A group's name, then its rows: the keys in a column `keys_width` wide,
/// so what they do lines up.
fn group(ui: &mut egui::Ui, (name, rows): Group, keys_width: f32) {
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
    use super::{groups, parse};

    /// The caps a mode's card shows, as `config/input.toml` names its keys.
    fn caps(direct: bool) -> Vec<String> {
        let (groups, _) = groups(direct);
        groups
            .iter()
            .flat_map(|(_, rows)| rows.iter())
            .flat_map(|(keys, _)| parse(keys))
            .filter(|(cap, _)| *cap)
            .map(|(_, k)| k.to_owned())
            .collect()
    }

    /// A binding's key as a cap says it: `KeyW` is `W`, `ShiftLeft` is
    /// `Shift`. `None` for one a row names in words (the arrows) or that
    /// stands in for another (the numpad's).
    fn cap_of(code: &str) -> Option<String> {
        if code.starts_with("Arrow") || code.starts_with("Numpad") {
            return None;
        }
        let code = code.strip_suffix("Left").or_else(|| code.strip_suffix("Right")).unwrap_or(code);
        Some(match code {
            "Equal" => "+".into(),
            "Minus" => "-".into(),
            c => c.strip_prefix("Key").unwrap_or(c).into(),
        })
    }

    /// Every key a mode reads in `config/input.toml` is on its card, but
    /// for the actions only the other mode reads.
    fn every_key_on_the_card(direct: bool, not_read: &[&str]) {
        let config: toml::Table = include_str!("../../../../config/input.toml").parse().unwrap();
        let caps = caps(direct);
        let mode = if direct { "Direct" } else { "Click" };
        for (action, def) in config["actions"].as_table().unwrap() {
            if not_read.contains(&action.as_str()) {
                continue;
            }
            for binding in def["bindings"].as_array().unwrap() {
                if binding["type"].as_str() != Some("key") {
                    continue;
                }
                let Some(cap) = cap_of(binding["code"].as_str().unwrap()) else {
                    continue;
                };
                assert!(caps.contains(&cap), "{action}'s {cap} isn't on the {mode} card");
            }
        }
    }

    #[test]
    fn every_key_direct_uses_is_on_its_card() {
        // Q and E orbit only in point and walk
        every_key_on_the_card(true, &["orbit_left", "orbit_right"]);
        for key in ["W", "A", "S", "D", "E", "Alt", "F8", "Esc", "Shift"] {
            assert!(caps(true).iter().any(|c| c == key), "{key}");
        }
    }

    #[test]
    fn every_key_click_uses_is_on_its_card() {
        // E is interact only over the shoulder; in point and walk it orbits
        every_key_on_the_card(false, &["interact"]);
    }

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
