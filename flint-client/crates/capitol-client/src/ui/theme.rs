//! The Capitol's look for egui (plan §4.3, P5.1): the colour tokens, the
//! fonts, egui's `Style` and `Visuals`, and the frames, text and small
//! widgets every panel is made of. Panels take their colours and sizes from
//! here and nowhere else, so the city looks like one place.
//!
//! The hex tokens are `world::palette`'s (plan §4.7, from the React client's
//! `index.css`); this turns them into egui colours. The panel values are the
//! React panels' CSS (`InventoryPanel.css`, `CraftingPanel.css`).
//!
//! The fonts are embedded, so a panel can't fall back to egui's default:
//! Source Sans 3 for text (the nearest open face to the React client's
//! Segoe UI, and clear at 11 px) and Cinzel, Roman inscriptional capitals,
//! for titles. The same files sit in `fonts/` for the script HUD, where
//! Flint registers them by name (`fonts/fonts.toml`).

use crate::world::palette;
use capitol_protocol::rules::Zone;
use egui::epaint::Shadow;
use egui::{
    Color32, FontData, FontFamily, FontId, Margin, Response, RichText, Rounding, Stroke, TextStyle,
    Ui,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// An sRGB hex token as an egui colour.
const fn rgb(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// CSS `rgba(r, g, b, a)` as an egui colour (egui's are premultiplied).
const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    const fn m(c: u8, a: f32) -> u8 {
        (c as f32 * a + 0.5) as u8
    }
    Color32::from_rgba_premultiplied(m(r, a), m(g, a), m(b, a), m(255, a))
}

/// The page behind everything, and the ink on gold buttons.
pub const BG_DARK: Color32 = rgb(palette::BACKGROUND);
/// A panel: `rgba(15,15,25,0.95)`.
pub const PANEL: Color32 = rgba(15, 15, 25, 0.95);
/// A panel's edge, and the rules inside one: `rgba(80,80,100,0.3)`.
pub const BORDER: Color32 = rgba(80, 80, 100, 0.3);
/// A recessed row or section in a panel: `rgba(30,30,45,0.6)`.
pub const WELL: Color32 = rgba(30, 30, 45, 0.6);
/// A field or a list entry, more solid than a well: `rgba(30,30,45,0.9)`.
pub const FIELD: Color32 = rgba(30, 30, 45, 0.9);
/// A count badge's ground: `rgba(40,40,55,0.8)`.
pub const PILL: Color32 = rgba(40, 40, 55, 0.8);

pub const TEXT: Color32 = rgb(0xe0e0e0);
pub const DIM: Color32 = rgb(palette::DIM_TEXT);
pub const GOLD: Color32 = rgb(palette::GOLD);
pub const GOLD_DIM: Color32 = rgb(palette::GOLD_DIM);
/// Crafting, and the crafting ring on the avatar.
pub const CRAFT: Color32 = rgb(0x8b5cf6);
/// In range of the player.
pub const IN_RANGE: Color32 = rgb(0xf97316);
/// Stations: the station panel's accent, the orange of the in-range ring.
pub const STATION: Color32 = IN_RANGE;
/// Trading (P7.3): the Trade District's blue, where the market is.
pub const TRADE: Color32 = rgb(0x3b82f6);
pub const ERROR: Color32 = rgb(0xdc3545);
/// The ground of an error box: the error colour at 10%.
pub const ERROR_WASH: Color32 = rgba(220, 53, 69, 0.1);
pub const SUCCESS: Color32 = rgb(0x28a745);
/// Enough of an ingredient.
pub const ENOUGH: Color32 = rgb(0x22c55e);
/// Not enough of an ingredient.
pub const SHORT: Color32 = rgb(0xef4444);
/// An inventory slot: `rgba(30,30,40,0.8)`, edged `rgba(80,80,100,0.5)`.
pub const SLOT: Color32 = rgba(30, 30, 40, 0.8);
pub const SLOT_EDGE: Color32 = rgba(80, 80, 100, 0.5);
/// A slot's edge when something's in it: `rgba(100,100,120,0.7)`.
pub const SLOT_EDGE_FULL: Color32 = rgba(100, 100, 120, 0.7);
/// The dark ring round a grade pip: black at 30%.
pub const PIP_EDGE: Color32 = rgba(0, 0, 0, 0.3);
/// The tray a grid of slots sits in: `rgba(20,20,30,0.6)`.
pub const TRAY: Color32 = rgba(20, 20, 30, 0.6);
/// A full slot under the pointer: `rgba(40,40,55,0.9)`, edged in the
/// inventory's purple at 60%.
pub const SLOT_HOVER: Color32 = rgba(40, 40, 55, 0.9);
/// A slot's quantity: white, on a dark shadow so it reads over any icon.
pub const QUANTITY: Color32 = Color32::WHITE;
pub const INK_SHADOW: Color32 = rgba(0, 0, 0, 0.8);
/// A slot's side, the icon's inside it, and the gap between slots, in
/// points (P5.2: the icons keep the pip's and the quantity's corners clear
/// at 36 in 48).
pub const SLOT_SIZE: f32 = 48.0;
pub const SLOT_ICON: f32 = 36.0;
pub const SLOT_GAP: f32 = 4.0;

/// A quality grade's pip colour (plan §4.7), for a letter from
/// `capitol_protocol::rules::quality_grade`.
pub fn grade(letter: &str) -> Color32 {
    match letter {
        "A" => rgb(0xffd700),
        "B" => rgb(0x22c55e),
        "C" => rgb(0x3b82f6),
        "D" => rgb(0xa855f7),
        _ => rgb(0x6b7280),
    }
}

/// The light end of a grade pip's gradient (React's `.slot-quality-pip`).
pub fn grade_light(letter: &str) -> Color32 {
    match letter {
        "A" => rgb(0xffed4e),
        "B" => rgb(0x4ade80),
        "C" => rgb(0x60a5fa),
        "D" => rgb(0xc084fc),
        _ => rgb(0x9ca3af),
    }
}

/// A zone's colour where it's named: the header's zone chip, as React's
/// zone indicator. The banner (`hud.rhai`) gets the same colour.
pub fn zone(zone: Zone) -> Color32 {
    rgb(palette::zone_mark(zone))
}

/// The header strip's ground: a panel, a touch less solid, so the dusk
/// shows through at the top of the screen.
pub const STRIP: Color32 = rgba(15, 15, 25, 0.88);
/// The zone chip's ground: `rgba(26,26,36,0.9)` (`ZoneIndicator.css`).
pub const CHIP: Color32 = rgba(26, 26, 36, 0.9);
/// The strand balance's ground: gold at 10% (`.currency-display`).
pub const GOLD_WASH: Color32 = rgba(201, 162, 39, 0.1);
/// Gold catching the light: a gain to the balance, and its sparks (P7.1).
pub const GOLD_LIGHT: Color32 = rgb(0xf2d675);
/// The glass under a hint: a panel at half strength.
pub const HINT: Color32 = rgba(15, 15, 25, 0.55);
/// What dims the world while the connection is down.
pub const SCRIM: Color32 = rgba(5, 5, 12, 0.45);

// ---------------------------------------------------------------------------
// Type
// ---------------------------------------------------------------------------

/// Text sizes in points, from the React panels.
pub mod size {
    /// The game's name on the login form.
    pub const TITLE: f32 = 28.0;
    /// A panel's heading.
    pub const HEADING: f32 = 14.0;
    pub const BODY: f32 = 13.0;
    /// A field's text, and the primary button's label.
    pub const LARGE: f32 = 15.0;
    /// An icon-sized glyph, such as the crafting arrow.
    pub const GLYPH: f32 = 24.0;
    /// A spinner.
    pub const SPINNER: f32 = 16.0;
    /// Secondary lines and badges.
    pub const SMALL: f32 = 12.0;
    /// Small-caps labels and footers.
    pub const LABEL: f32 = 11.0;
    /// Identifiers under a name, in monospace.
    pub const TINY: f32 = 10.0;
}

const BODY_FONT: &str = "capitol-body";
const STRONG_FONT: &str = "capitol-strong";
const DISPLAY_FONT: &str = "capitol-display";

/// Text in the body face.
pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Text in the semibold face: headings and buttons. (egui's `strong()`
/// only brightens the colour; this is the real weight.)
pub fn strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(STRONG_FONT.into()))
}

/// Text in the inscriptional capitals: titles only.
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(DISPLAY_FONT.into()))
}

/// The game's name, or a panel's name set large: gold, in inscriptional
/// capitals, spaced like letters cut in stone.
pub fn title(text: &str, size: f32) -> RichText {
    RichText::new(text)
        .font(display(size))
        .color(GOLD)
        .extra_letter_spacing(size * 0.18)
}

/// A panel's heading.
pub fn heading(text: &str) -> RichText {
    RichText::new(text).font(strong(size::HEADING)).color(TEXT)
}

/// A small-caps label over a field or a section.
pub fn label(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .font(strong(size::LABEL))
        .color(DIM)
        .extra_letter_spacing(1.2)
}

pub fn text(text: impl Into<String>) -> RichText {
    RichText::new(text).color(TEXT)
}

pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text).color(DIM).size(size::SMALL)
}

// ---------------------------------------------------------------------------
// Installing it
// ---------------------------------------------------------------------------

/// Makes `ctx` look like the Capitol. Call it every frame before drawing,
/// and draw nothing when it returns false: the fonts arrive at the start of
/// the next frame, and text in a family egui doesn't have yet panics. It's
/// cheap once applied, and it puts the fonts back if Flint reloads its own
/// (on a scene change, which replaces egui's fonts).
///
/// A workaround: Flint can't make a project font egui's default, and its
/// font reload drops a game's own. Issue #72 removes the embedded copies,
/// the per-frame check and the skipped frame.
pub fn install(ctx: &egui::Context) -> bool {
    let id = egui::Id::new("capitol_theme");
    if ctx.data(|d| d.get_temp::<bool>(id)).is_none() {
        ctx.all_styles_mut(style);
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
    let ready = ctx.fonts(|f| {
        f.lock()
            .fonts
            .definitions()
            .font_data
            .contains_key(BODY_FONT)
    });
    if !ready {
        let mut defs = ctx.fonts(|f| f.lock().fonts.definitions().clone());
        with_theme_fonts(&mut defs);
        ctx.set_fonts(defs);
        ctx.request_repaint();
    }
    ready
}

/// `defs` with the theme's faces added: the body face first in
/// `Proportional`, and a family each for the semibold and the capitals.
/// Every family keeps what `Proportional` had after it, for the glyphs our
/// faces lack. Flint's own families (one per file in `fonts/`) stay.
fn with_theme_fonts(defs: &mut egui::FontDefinitions) {
    let faces: [(&str, &'static [u8]); 3] = [
        (
            BODY_FONT,
            include_bytes!("../../../../fonts/SourceSans3-Regular.ttf"),
        ),
        (
            STRONG_FONT,
            include_bytes!("../../../../fonts/SourceSans3-Semibold.ttf"),
        ),
        (DISPLAY_FONT, include_bytes!("../../../../fonts/Cinzel.ttf")),
    ];
    for (name, bytes) in faces {
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let fallbacks = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, face) in [
        (FontFamily::Proportional, BODY_FONT),
        (FontFamily::Name(STRONG_FONT.into()), STRONG_FONT),
        (FontFamily::Name(DISPLAY_FONT.into()), DISPLAY_FONT),
    ] {
        let mut list = vec![face.to_owned()];
        list.extend(
            fallbacks
                .iter()
                .filter(|f| f.as_str() != BODY_FONT)
                .cloned(),
        );
        defs.families.insert(family, list);
    }
}

/// egui's `Style` and `Visuals`, from the tokens.
fn style(style: &mut egui::Style) {
    style.text_styles = [
        (TextStyle::Small, body(size::LABEL)),
        (TextStyle::Body, body(size::BODY)),
        (TextStyle::Button, strong(size::BODY)),
        (TextStyle::Heading, strong(size::HEADING)),
        (TextStyle::Monospace, FontId::monospace(size::SMALL)),
    ]
    .into();

    // The React panels: 12 px padding, 8 px gaps, 4 px between list rows.
    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(8.0, 6.0);
    spacing.button_padding = egui::vec2(10.0, 5.0);
    spacing.window_margin = Margin::same(12.0);
    spacing.menu_margin = Margin::same(8.0);
    spacing.interact_size = egui::vec2(32.0, 26.0);
    spacing.icon_width = 14.0;
    spacing.icon_spacing = 6.0;
    spacing.combo_width = 140.0;
    spacing.text_edit_width = 220.0;
    spacing.scroll.bar_width = 6.0;
    spacing.scroll.floating_allocated_width = 0.0;

    style.visuals = visuals();
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.override_text_color = None;
    v.hyperlink_color = GOLD;
    v.faint_bg_color = WELL;
    v.extreme_bg_color = BG_DARK;
    v.code_bg_color = FIELD;
    v.warn_fg_color = IN_RANGE;
    v.error_fg_color = ERROR;

    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(8.0);
    v.window_highlight_topmost = false;
    // The React panels' `box-shadow: 0 4px 20px rgba(0,0,0,0.5)`.
    v.window_shadow = Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 20.0,
        spread: 0.0,
        color: rgba(0, 0, 0, 0.5),
    };
    v.popup_shadow = Shadow {
        offset: egui::vec2(0.0, 3.0),
        blur: 12.0,
        spread: 0.0,
        color: rgba(0, 0, 0, 0.5),
    };

    // Selection and focus are gold. A focused widget draws as `active`, so
    // its gold edge is the focus ring.
    v.selection.bg_fill = GOLD.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, GOLD);
    v.text_cursor.stroke = Stroke::new(1.5, GOLD);

    let round = Rounding::same(4.0);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.rounding = round;

    w.inactive.bg_fill = FIELD;
    w.inactive.weak_bg_fill = FIELD;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.rounding = round;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = PILL;
    w.hovered.weak_bg_fill = PILL;
    w.hovered.bg_stroke = Stroke::new(1.0, GOLD_DIM);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.rounding = round;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = PILL;
    w.active.weak_bg_fill = PILL;
    w.active.bg_stroke = Stroke::new(1.0, GOLD);
    w.active.fg_stroke = Stroke::new(1.0, GOLD);
    w.active.rounding = round;
    w.active.expansion = 0.0;

    w.open = w.active;
    v
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// A panel: the dark glass, its faint edge and 12 px corners.
pub fn panel() -> egui::Frame {
    egui::Frame::none()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .rounding(12.0)
        .inner_margin(Margin::same(12.0))
        .shadow(Shadow {
            offset: egui::vec2(0.0, 4.0),
            blur: 20.0,
            spread: 0.0,
            color: rgba(0, 0, 0, 0.5),
        })
}

/// A panel edged in its own colour, as the React panels are: crafting and
/// the inventory in purple, the login form in gold.
pub fn panel_edged(edge: Color32) -> egui::Frame {
    panel().stroke(Stroke::new(1.0, edge.gamma_multiply(0.3)))
}

/// A recessed section inside a panel.
pub fn well() -> egui::Frame {
    egui::Frame::none()
        .fill(WELL)
        .rounding(6.0)
        .inner_margin(Margin::same(8.0))
}

/// The tray under a grid of slots (React's `.inventory-grid`).
pub fn tray() -> egui::Frame {
    egui::Frame::none()
        .fill(TRAY)
        .rounding(8.0)
        .inner_margin(Margin::same(8.0))
}

/// A small bordered chip in the header strip: the zone (edged in its
/// colour) and the strand balance (gold on a gold wash).
pub fn chip(edge: Color32, fill: Color32) -> egui::Frame {
    egui::Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0, edge))
        .rounding(5.0)
        .inner_margin(Margin::symmetric(10.0, 4.0))
}

/// A header chip that opens and closes a panel: lit in the panel's purple
/// while it's open, a quiet chip while it's closed.
pub fn toggle_chip(ui: &mut Ui, text: &str, on: bool) -> Response {
    let (edge, fill, ink) = if on {
        (CRAFT.gamma_multiply(0.6), CRAFT.gamma_multiply(0.18), TEXT)
    } else {
        (BORDER, CHIP, DIM)
    };
    ui.add(
        egui::Button::new(RichText::new(text).size(size::BODY).color(ink))
            .fill(fill)
            .stroke(Stroke::new(1.0, edge))
            .rounding(5.0)
            .min_size(egui::vec2(0.0, 26.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Something just landed here (P8.2): a gold edge round `rect`, and a
/// fainter one just outside it, at strength `k` (0 to 1). Painted over
/// what's there, so it lights the edge, not the insides.
pub fn landed_glow(painter: &egui::Painter, rect: egui::Rect, rounding: f32, k: f32) {
    if k <= 0.0 {
        return;
    }
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.5, GOLD.gamma_multiply(0.9 * k)),
    );
    painter.rect_stroke(
        rect.expand(2.5 + 2.0 * (1.0 - k)),
        rounding + 2.5,
        Stroke::new(2.0, GOLD.gamma_multiply(0.3 * k)),
    );
}

/// A faint glass under a few lines of hint text over the world.
pub fn hint() -> egui::Frame {
    egui::Frame::none()
        .fill(HINT)
        .rounding(6.0)
        .inner_margin(Margin::symmetric(10.0, 6.0))
}

/// The form's message, in the React client's error box.
pub fn error_box(ui: &mut Ui, text: &str) {
    egui::Frame::none()
        .fill(ERROR_WASH)
        .stroke(Stroke::new(1.0, ERROR))
        .rounding(4.0)
        .inner_margin(Margin::symmetric(12.0, 9.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(ERROR).size(size::BODY));
        });
}

// ---------------------------------------------------------------------------
// Small widgets
// ---------------------------------------------------------------------------

/// A panel's header: the heading, an optional badge on the right, and the
/// rule under them (React's `.inventory-header`). `accent` colours the
/// heading and the rule, as the crafting panel's are purple; `None` is
/// plain.
pub fn header(ui: &mut Ui, title: &str, accent: Option<Color32>, badge: Option<(&str, Color32)>) {
    ui.horizontal(|ui| {
        ui.label(heading(title).color(accent.unwrap_or(TEXT)));
        if let Some((text, fill)) = badge {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pill(ui, text, fill);
            });
        }
    });
    ui.add_space(2.0);
    match accent {
        Some(accent) => rule_in(ui, accent.gamma_multiply(0.3)),
        None => divider(ui),
    }
    ui.add_space(2.0);
}

/// A count or state badge, rounded at the ends.
pub fn pill(ui: &mut Ui, text: &str, fill: Color32) -> Response {
    let ink = if fill == PILL { DIM } else { TEXT };
    egui::Frame::none()
        .fill(fill)
        .rounding(10.0)
        .inner_margin(Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(ink).size(size::SMALL))
        })
        .response
}

/// A plain 1 px rule across the space, in the border colour.
pub fn divider(ui: &mut Ui) {
    rule_in(ui, BORDER);
}

fn rule_in(ui: &mut Ui, colour: Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, colour));
}

/// Rows of a list: closer together than the rest of a panel, and no
/// taller than their text.
pub fn list_spacing(ui: &mut Ui) {
    let spacing = ui.spacing_mut();
    spacing.item_spacing.y = 4.0;
    spacing.interact_size.y = 16.0;
}

/// A thin gold rule across the space, fading at both ends.
pub fn gold_rule(ui: &mut Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    let painter = ui.painter();
    let steps = 24;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let fade = 1.0 - (2.0 * ((t0 + t1) / 2.0) - 1.0).abs();
        let a = rect.left() + rect.width() * t0;
        let b = rect.left() + rect.width() * t1;
        painter.hline(
            a..=b,
            rect.center().y,
            Stroke::new(1.0, GOLD_DIM.gamma_multiply(fade)),
        );
    }
}

/// The gold call to action, spanning its column. Dimmed while `busy`.
pub fn primary_button(ui: &mut Ui, text: &str, busy: bool) -> Response {
    let button = egui::Button::new(RichText::new(text).font(strong(size::LARGE)).color(BG_DARK))
        .fill(if busy { GOLD_DIM } else { GOLD })
        .stroke(Stroke::NONE)
        .rounding(4.0)
        .min_size(egui::vec2(0.0, 40.0));
    ui.vertical_centered_justified(|ui| ui.add_enabled(!busy, button))
        .inner
}

/// An everyday button: a dark field with a gold edge on hover and focus.
/// `gold` sets its label in gold, for the one a row is for.
pub fn button(ui: &mut Ui, text: &str, gold: bool) -> Response {
    let ink = if gold { GOLD } else { TEXT };
    ui.button(RichText::new(text).color(ink))
}

/// A panel's own call to action, in its accent: a tinted fill, a 2 px
/// edge and the label in the accent (React's `.craft-button`).
pub fn accent_button(ui: &mut Ui, text: &str, accent: Color32, enabled: bool) -> Response {
    let hovered_fill = accent.gamma_multiply(0.4);
    let button = egui::Button::new(
        RichText::new(text)
            .font(strong(size::HEADING))
            .color(accent),
    )
    .fill(accent.gamma_multiply(0.2))
    .stroke(Stroke::new(2.0, accent))
    .rounding(6.0)
    .min_size(egui::vec2(0.0, 38.0));
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        w.hovered.weak_bg_fill = hovered_fill;
        w.active.weak_bg_fill = hovered_fill;
        if !enabled {
            ui.multiply_opacity(0.5);
        }
        ui.vertical_centered_justified(|ui| ui.add_enabled(enabled, button))
            .inner
    })
    .inner
}

/// How far along something is, as a bar in `accent` (0 to 1).
pub fn progress(ui: &mut Ui, fraction: f32, accent: Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, accent.gamma_multiply(0.2));
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    painter.rect_filled(fill, 4.0, accent);
}

/// A predicted quality and its grade, tinted by the grade: "~82 B".
pub fn grade_badge(ui: &mut Ui, quality: i32) -> Response {
    grade_tag(ui, &format!("~{quality}"), quality)
}

/// A known quality and its grade, tinted by the grade: "82 B".
pub fn quality_badge(ui: &mut Ui, quality: i32) -> Response {
    grade_tag(ui, &quality.to_string(), quality)
}

fn grade_tag(ui: &mut Ui, number: &str, quality: i32) -> Response {
    let letter = capitol_protocol::rules::quality_grade(quality);
    let colour = grade(letter);
    egui::Frame::none()
        .fill(colour.gamma_multiply(0.2))
        .rounding(4.0)
        .inner_margin(Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("{number} {letter}"))
                    .font(strong(size::SMALL))
                    .color(colour),
            )
        })
        .response
}

/// Underlined words that act, in dim gold.
pub fn link(ui: &mut Ui, text: &str, size: f32) -> Response {
    ui.add(
        egui::Label::new(RichText::new(text).color(GOLD_DIM).size(size).underline())
            .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small status light; the hover says what it means.
pub fn status_dot(ui: &mut Ui, colour: Color32, meaning: &str) {
    let (dot, response) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(dot.center(), 3.5, colour);
    response.on_hover_text(meaning);
}

/// Text fields as dark wells with a dim gold edge that turns gold on focus,
/// for a form that's mostly fields (the login form).
pub fn gold_fields(ui: &mut Ui) {
    let v = ui.visuals_mut();
    v.extreme_bg_color = BG_DARK;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, GOLD_DIM.gamma_multiply(0.7));
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, GOLD_DIM);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_rgba_is_premultiplied() {
        assert_eq!(PANEL, Color32::from_rgba_premultiplied(14, 14, 24, 242));
        assert_eq!(BORDER, Color32::from_rgba_premultiplied(24, 24, 30, 77));
        assert_eq!(PANEL.to_srgba_unmultiplied()[..3], [15, 15, 25]);
    }

    #[test]
    fn tokens_are_the_palette() {
        assert_eq!(GOLD, Color32::from_rgb(0xc9, 0xa2, 0x27));
        assert_eq!(DIM, Color32::from_rgb(0x80, 0x80, 0x80));
        assert_eq!(
            zone(Zone::TradeDistrict),
            Color32::from_rgb(0x3b, 0x82, 0xf6)
        );
        assert_eq!(grade("A"), Color32::from_rgb(0xff, 0xd7, 0x00));
        assert_eq!(grade("?"), grade("F"));
    }

    /// Every family the theme names exists once it's installed: an unknown
    /// family panics when egui lays text out in it.
    #[test]
    fn installed_fonts_lay_out() {
        let ctx = egui::Context::default();
        assert!(!run(&ctx), "the fonts can't be ready on the first frame");
        assert!(run(&ctx));
        fn run(ctx: &egui::Context) -> bool {
            let mut ready = false;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                ready = install(ctx);
                if !ready {
                    return;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label(title("The Capitol", size::TITLE));
                    ui.label(heading("Inventory"));
                    ui.label(label("Email"));
                    ui.label(text("Fiber ×12"));
                });
            });
            ready
        }
        ctx.fonts(|f| {
            let fonts = f.lock();
            let families = &fonts.fonts.definitions().families;
            assert_eq!(families[&FontFamily::Proportional][0], BODY_FONT);
            assert_eq!(
                families[&FontFamily::Name(STRONG_FONT.into())][0],
                STRONG_FONT
            );
            assert_eq!(
                families[&FontFamily::Name(DISPLAY_FONT.into())][0],
                DISPLAY_FONT
            );
        });
    }
}
