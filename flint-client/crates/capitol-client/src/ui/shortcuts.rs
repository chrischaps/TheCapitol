//! The shortcut overlay (P8.3): every control on one card, over a dimmed
//! world. `?` opens and closes it (read off the typed character in
//! `CapitolExtension::on_window_event`, so it's `?` on any layout), and so
//! does F1 (the `help` action, which `--input-script` can press) and the
//! header's `?` chip. Esc or a click outside closes it. While the road
//! in's card is hidden (#441), a line under the work brings it back.
//!
//! A row is written as words with keys in brackets: `"[Alt] + [Click] a
//! plot"`. Each bracket is drawn as a key cap. The pad's controls go in
//! braces, `"{A}"`, and are drawn as its glyphs ([`theme::padcap`]).
//!
//! The card is the mode's (T1.6): point and walk, where a click walks and
//! WASD pans, or over the shoulder, where WASD walks, the mouse looks and E
//! acts. Only getting about and the work differ; the panels are the same.
//! Over the shoulder a column says what the pad does (T2.2), and whichever
//! you last used is drawn the brighter.

use super::pad_nav::FollowFocus;
use super::theme::{self, size};
use egui::{Align2, Id, Order, RichText};

/// How long the overlay takes to fade in or out, in seconds.
const FADE: f32 = 0.16;
/// The keys' width in the left column and the right, the pad's, then what
/// they do.
const KEYS_LEFT: f32 = 192.0;
const KEYS_RIGHT: f32 = 160.0;
const PAD_WIDTH: f32 = 68.0;
const WORDS_WIDTH: f32 = 150.0;
/// The card's height less its columns', and a margin round it: what the
/// columns may not take of the screen.
const CHROME: f32 = 140.0;
/// The room between the two columns.
const COLUMN_GAP: f32 = 28.0;
/// The card's frame and a margin round it, across: what two columns may
/// not take of the screen's width.
const CHROME_ACROSS: f32 = 72.0;
/// How bright the device you aren't using is drawn.
const OTHER_DEVICE: f32 = 0.45;

/// A control: its keys, the pad's (empty for none), and what it does.
type Row = (&'static str, &'static str, &'static str);

/// A group of controls: its name, then its rows.
type Group = (&'static str, &'static [Row]);

/// Every control, by what it's for, in point and walk: getting about, the
/// work and the panels. When a control changes, change it here (and in
/// [`DIRECT`] if it's there too) and in `config/input.toml`. Point and
/// walk shows no pad column: the sticks do nothing there.
const CLICK: [Group; 3] = [
    (
        "Getting about",
        &[
            ("[Click] the ground", "", "Walk there"),
            ("[W] [A] [S] [D] or arrows", "", "Pan the view"),
            ("[Wheel], [R] [F] or [+] [-]", "", "Zoom in and out"),
            ("[Q] [E] or right-drag", "", "Turn the view"),
            ("[H] or [Home]", "", "Back to you"),
            ("[F8]", "", "Over the shoulder"),
        ],
    ),
    (
        "Making a living",
        &[
            ("[Click] grass", "", "Gather fiber"),
            ("[Click] a station, or [O]", "", "Open it, or the nearest"),
            ("[Enter], crafting", "", "Craft what's set"),
            ("Right-click a kit", "", "Place its station"),
            ("[PgUp] [PgDn]", "", "Placing: the storey"),
            ("[Esc] or right-click", "", "Stop placing"),
            ("[Alt] + [Click] a plot", "", "See its deed"),
            ("[Shift] + [Click] a player", "", "Ask them to trade"),
            ("[T]", "", "Accept, or ask the nearest"),
        ],
    ),
    PANELS,
];

/// The same over the shoulder (Direct, T1.6). E, or a click while the
/// cursor is captured, acts on what's in front of you, as a click on it
/// does in point and walk; on the pad (T2.1) A does, and Y trades.
const DIRECT: [Group; 3] = [
    (
        "Getting about",
        &[
            ("[W] [A] [S] [D] or arrows", "{LS}", "Walk"),
            ("Mouse", "{RS}", "Look"),
            ("[E] or [Click]", "{A}", "Act on what's ahead"),
            ("[Alt]", "", "Free the cursor"),
            ("[Esc]", "{B}", "Close a panel, or free the cursor"),
            ("[Wheel], [R] [F] or [+] [-]", "{LT} {RT}", "Zoom in and out"),
            ("[H] or [Home]", "Press {RS}", "Back over your shoulder"),
            ("[F8]", "", "Point and walk"),
        ],
    ),
    (
        "Making a living",
        &[
            ("[E] grass", "{A}", "Gather fiber"),
            ("[E] a station, or [O]", "{A}", "Open it, or the nearest"),
            ("[Enter], crafting", "", "Craft what's set"),
            ("Right-click a kit", "", "Place its station"),
            ("[PgUp] [PgDn]", "", "Placing: the storey"),
            ("[Esc] or right-click", "{B}", "Stop placing"),
            ("[Alt] + [Click] a plot", "", "See its deed"),
            ("[E] a player", "{A}", "See their card"),
            ("[Shift] + [E] a player", "{Y}", "Ask them to trade"),
            ("[T]", "{Y}", "Accept, or ask who's near"),
        ],
    ),
    PANELS,
];

/// The panels, the same in both modes.
const PANELS: Group = (
    "Panels",
    &[
        ("[I]", "{Up}", "Inventory"),
        ("[C]", "{Left}", "Crafting"),
        ("[X]", "{Down}", "Fiber exchange"),
        ("[M]", "{Right}", "The market"),
        ("[G]", "", "The Gazette"),
        ("[Enter]", "", "Talk: Enter sends, Esc closes"),
        ("[L]", "", "Your letters"),
        ("[U]", "", "Guilds"),
        ("[F10]", "{Start}", "Settings"),
        ("[F7]", "", "Dev overlay"),
        ("[F6]", "", "World layers (admins)"),
        ("[?] or [F1]", "{Select}", "This list"),
        // Inside a panel, the pad's alone (#331): Click's card leaves these out
        ("", "{DPad}", "In a panel, move round it"),
        ("", "{A}", "Press, or pick up and put down"),
        ("", "{X}", "Pick up half a stack"),
        ("", "{LB} {RB}", "The next panel along"),
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

/// A row of keys as the pad would say it (T2.2), for the road in's cards
/// while a pad is in your hands: a click or E on a thing (`[Click] a
/// tuft`) is `{A}`, a shift on a person is `{Y}`, WASD is the left stick
/// and the panels' keys are the d-pad's. What the pad has no way to do
/// (`[Alt] + [Click]`, a kit's placing, `[O]`, `[Enter]`) keeps its keys.
pub fn on_the_pad(spec: &str) -> String {
    const KEEP: &str = "\u{1}";
    let mut s = spec.replace("[Alt] + [Click]", KEEP);
    for (keys, pad) in [
        ("[Shift] + [Click]", "{Y}"),
        ("[Shift] + [E]", "{Y}"),
        ("[W] [A] [S] [D]", "{LS}"),
        ("[I]", "{Up}"),
        ("[C]", "{Left}"),
        ("[X]", "{Down}"),
        ("[M]", "{Right}"),
        ("[F10]", "{Start}"),
        ("[F1]", "{Select}"),
    ] {
        s = s.replace(keys, pad);
    }
    // A click or E on a thing, not "[Click] to place"
    for act in ["[Click]", "[E]"] {
        for thing in [" a ", " an ", " the "] {
            s = s.replace(&format!("{act}{thing}"), &format!("{{A}}{thing}"));
        }
    }
    s.replace(KEEP, "[Alt] + [Click]")
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
    /// over the shoulder), with the pad's column the brighter while it's
    /// the pad you're using (`pad`). `road_in_hidden` adds the line that
    /// brings the road in's card back.
    pub fn show(&mut self, ctx: &egui::Context, direct: bool, pad: bool, road_in_hidden: bool) -> Asked {
        let (groups, mode) = groups(direct);
        // Only over the shoulder has a pad column
        let device = direct.then_some(pad);
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
                // A small window at a large scale (720p at 150%) has less
                // height than the columns: they scroll then. With the pad's
                // column it has less width than two, and the work goes under
                // getting about and the panels. How wide the two came out,
                // and the wider of them, when last drawn side by side, with
                // the pad's column or without:
                let wide_id = Id::new(("capitol_shortcuts_wide", device.is_some()));
                let wide: Option<(f32, f32)> = ctx.data(|d| d.get_temp(wide_id));
                let stacked = wide.filter(|(both, _)| both + CHROME_ACROSS > screen.width()).map(|(_, one)| one);
                theme::panel_edged(theme::GOLD).show(ui, |ui| {
                    if let Some(one) = stacked {
                        // Or the title's row keeps the card as wide as two
                        ui.set_max_width(one);
                    }
                    ui.vertical_centered(|ui| {
                        ui.label(theme::title("Controls", size::TITLE * 0.75));
                        ui.label(theme::dim(mode));
                    });
                    ui.add_space(4.0);
                    theme::gold_rule(ui);
                    ui.add_space(10.0);
                    let room = (screen.height() - CHROME).max(160.0);
                    egui::ScrollArea::vertical().max_height(room).show_following(ui, |ui| {
                        let mut road_in = |ui: &mut egui::Ui| {
                            if road_in_hidden {
                                ui.add_space(12.0);
                                ui.label(theme::label("The road in"));
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(theme::text("Its card is hidden for now.").size(size::BODY));
                                    asked.road_in = theme::link(ui, "Bring it back", size::BODY).clicked();
                                });
                            }
                        };
                        if stacked.is_some() {
                            ui.vertical(|ui| {
                                group(ui, groups[0], KEYS_LEFT, device);
                                ui.add_space(12.0);
                                group(ui, groups[1], KEYS_LEFT, device);
                                ui.add_space(12.0);
                                group(ui, groups[2], KEYS_LEFT, device);
                                road_in(ui);
                            });
                            return;
                        }
                        let drawn = ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                            // Getting about and the panels on the left, the
                            // work on the right: two columns of about equal height.
                            let left = ui.vertical(|ui| {
                                group(ui, groups[0], KEYS_LEFT, device);
                                ui.add_space(12.0);
                                group(ui, groups[2], KEYS_LEFT, device);
                            });
                            let right = ui.vertical(|ui| {
                                group(ui, groups[1], KEYS_RIGHT, device);
                                road_in(ui);
                            });
                            left.response.rect.width().max(right.response.rect.width())
                        });
                        let both = drawn.response.rect.width();
                        ui.ctx().data_mut(|d| d.insert_temp(wide_id, (both, drawn.inner)));
                    });
                    ui.add_space(10.0);
                    if device == Some(true) {
                        // The pad's own way to close it, as glyphs, centred
                        // by last frame's width: a row of several widgets
                        // doesn't centre itself
                        let id = Id::new("capitol_shortcuts_close");
                        let width: f32 = ctx.data(|d| d.get_temp(id)).unwrap_or(0.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                            let row = ui.horizontal(|ui| keys_row(ui, "{Select} or {B} closes this"));
                            ui.ctx().data_mut(|d| d.insert_temp(id, row.response.rect.width()));
                        });
                    } else {
                        ui.vertical_centered(|ui| {
                            ui.label(theme::dim("? or Esc closes this"));
                        });
                    }
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
/// then, with a `device` (over the shoulder), the pad's, so what they do
/// lines up. `device` says whether it's the pad you're using: the other
/// is drawn dimmer.
fn group(ui: &mut egui::Ui, (name, rows): Group, keys_width: f32, device: Option<bool>) {
    ui.label(theme::label(name));
    ui.add_space(4.0);
    let cell = |ui: &mut egui::Ui, width: f32, bright: bool, spec: &str| {
        ui.allocate_ui_with_layout(
            egui::vec2(width, 22.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(width);
                if !bright {
                    ui.multiply_opacity(OTHER_DEVICE);
                }
                keys_row(ui, spec);
            },
        );
    };
    for (keys, pad, words) in rows {
        // A row the pad alone has, where there's no pad column
        if keys.is_empty() && device.is_none() {
            continue;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            cell(ui, keys_width, device != Some(true), keys);
            if let Some(pad_held) = device {
                cell(ui, PAD_WIDTH, pad_held, pad);
            }
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

/// Words with `[Key]`s and the pad's `{Controls}` in them: the keys as
/// caps, the pad's as its glyphs, and the words dim.
pub fn keys_row(ui: &mut egui::Ui, spec: &str) {
    for piece in parse(spec) {
        match piece {
            Piece::Key(key) => {
                theme::keycap(ui, key);
            }
            Piece::Pad(control) => {
                theme::padcap(ui, control);
            }
            Piece::Words(words) => {
                ui.label(RichText::new(words).color(theme::DIM).size(size::SMALL));
            }
        }
    }
}

/// A piece of a row: a key, one of the pad's controls, or words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece<'a> {
    Key(&'a str),
    Pad(&'a str),
    Words(&'a str),
}

/// `spec` split into keys, the pad's controls and the words between them,
/// trimmed, with empty pieces dropped.
fn parse(spec: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = spec;
    while let Some(open) = rest.find(['[', '{']) {
        let shut = if rest[open..].starts_with('[') { ']' } else { '}' };
        let Some(close) = rest[open..].find(shut).map(|c| open + c) else {
            break;
        };
        out.push(Piece::Words(rest[..open].trim()));
        let inside = &rest[open + 1..close];
        out.push(if shut == ']' { Piece::Key(inside) } else { Piece::Pad(inside) });
        rest = &rest[close + 1..];
    }
    out.push(Piece::Words(rest.trim()));
    out.retain(|p| !matches!(p, Piece::Words(w) if w.is_empty()));
    out
}

#[cfg(test)]
mod tests {
    use super::{groups, on_the_pad, parse, Piece};

    /// The caps a mode's card shows, as `config/input.toml` names its keys
    /// (`pad`: the pad's column's, as `theme::padcap` names them).
    fn caps(direct: bool, pad: bool) -> Vec<String> {
        let (groups, _) = groups(direct);
        groups
            .iter()
            .flat_map(|(_, rows)| rows.iter())
            .flat_map(|(keys, pads, _)| parse(if pad { pads } else { keys }))
            .filter_map(|p| match p {
                Piece::Key(k) if !pad => Some(k.to_owned()),
                Piece::Pad(c) if pad => Some(c.to_owned()),
                _ => None,
            })
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
            "PageUp" => "PgUp".into(),
            "PageDown" => "PgDn".into(),
            c => c.strip_prefix("Key").unwrap_or(c).into(),
        })
    }

    /// A pad binding as its glyph names it: South is `A`, `DPadUp` is
    /// `Up`, a stick's axes and its press are the stick.
    fn glyph_of(binding: &toml::Value) -> String {
        let name = binding.get("button").or_else(|| binding.get("axis")).unwrap().as_str().unwrap();
        match name {
            "South" => "A",
            "East" => "B",
            "West" => "X",
            "North" => "Y",
            "LeftTrigger" => "LB",
            "RightTrigger" => "RB",
            "LeftTrigger2" => "LT",
            "RightTrigger2" => "RT",
            "LeftThumb" | "LeftStickX" | "LeftStickY" => "LS",
            "RightThumb" | "RightStickX" | "RightStickY" => "RS",
            d => d.strip_prefix("DPad").unwrap_or(d),
        }
        .to_owned()
    }

    fn input_toml() -> toml::Table {
        include_str!("../../../../config/input.toml").parse().unwrap()
    }

    /// Every key a mode reads in `config/input.toml` is on its card, but
    /// for the actions only the other mode reads.
    fn every_key_on_the_card(direct: bool, not_read: &[&str]) {
        let config = input_toml();
        let caps = caps(direct, false);
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
            assert!(caps(true, false).iter().any(|c| c == key), "{key}");
        }
    }

    #[test]
    fn every_key_click_uses_is_on_its_card() {
        // E is interact only over the shoulder; in point and walk it orbits
        every_key_on_the_card(false, &["interact"]);
    }

    /// Every pad control bound in `config/input.toml` is in Direct's pad
    /// column (T2.2), and the column names nothing that isn't bound.
    #[test]
    fn every_pad_control_is_in_the_pad_column() {
        let config = input_toml();
        let column = caps(true, true);
        let mut bound = Vec::new();
        for (action, def) in config["actions"].as_table().unwrap() {
            for binding in def["bindings"].as_array().unwrap() {
                if !binding["type"].as_str().unwrap().starts_with("gamepad") {
                    continue;
                }
                let glyph = glyph_of(binding);
                assert!(column.contains(&glyph), "{action}'s {glyph} isn't in the pad column");
                bound.push(glyph);
            }
        }
        // The whole d-pad stands for its four arms (#331)
        if ["Up", "Down", "Left", "Right"].iter().all(|arm| bound.iter().any(|g| g == arm)) {
            bound.push("DPad".to_owned());
        }
        for glyph in &column {
            assert!(bound.contains(glyph), "{glyph} is in the pad column but bound to nothing");
        }
        // Point and walk draws no pad column; only the panels it shares
        // have one
        let (click, _) = groups(false);
        assert!(click[..2].iter().all(|(_, rows)| rows.iter().all(|(_, pad, _)| pad.is_empty())));
    }

    #[test]
    fn a_row_splits_into_keys_the_pad_and_words() {
        assert_eq!(
            parse("[Alt] + [Click] a plot"),
            vec![Piece::Key("Alt"), Piece::Words("+"), Piece::Key("Click"), Piece::Words("a plot")]
        );
        assert_eq!(parse("Right-click a kit"), vec![Piece::Words("Right-click a kit")]);
        assert_eq!(parse("[H]"), vec![Piece::Key("H")]);
        assert_eq!(
            parse("Press {RS} or [Home]"),
            vec![Piece::Words("Press"), Piece::Pad("RS"), Piece::Words("or"), Piece::Key("Home")]
        );
    }

    #[test]
    fn keys_say_themselves_on_the_pad() {
        assert_eq!(on_the_pad("[Click] a tuft"), "{A} a tuft");
        assert_eq!(on_the_pad("[Click] a person  ·  [Shift] + [Click] trade"), "{A} a person  ·  {Y} trade");
        assert_eq!(on_the_pad("[C] crafting  ·  [Enter] craft"), "{Left} crafting  ·  [Enter] craft");
        assert_eq!(on_the_pad("[M] the market  ·  Sell"), "{Right} the market  ·  Sell");
        assert_eq!(on_the_pad("[I] opens and closes it"), "{Up} opens and closes it");
        // What the pad can't do keeps its keys
        assert_eq!(on_the_pad("[Alt] + [Click] the lot  ·  About the zone"), "[Alt] + [Click] the lot  ·  About the zone");
        assert_eq!(on_the_pad("Right-click the kit  ·  [Click] to place"), "Right-click the kit  ·  [Click] to place");
        assert_eq!(on_the_pad("[O] the nearest station"), "[O] the nearest station");
    }
}
