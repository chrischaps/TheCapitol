//! The Capitol's look for egui (plan §4.3, P5.1): the colour tokens, the
//! fonts, egui's `Style` and `Visuals`, and the frames, text and small
//! widgets every panel is made of. Panels take their colours and sizes from
//! here and nowhere else, so the city looks like one place.
//!
//! The hex tokens are `world::palette`'s (plan §4.7, from the React client's
//! `index.css`); this turns them into egui colours. The panel values are the
//! React panels' CSS (`InventoryPanel.css`, `CraftingPanel.css`).
//!
//! The fonts are Source Sans 3 for text (the nearest open face to the React
//! client's Segoe UI, and clear at 11 px) and Cinzel, Roman inscriptional
//! capitals, for titles. They're in `fonts/`, where Flint registers them for
//! the script HUD and for egui alike, by their `fonts/fonts.toml` names, and
//! makes `body` egui's default face (Flint ADR 0081).

use crate::world::palette;
use capitol_protocol::rules::Zone;
use egui::epaint::Shadow;
use egui::{
    Color32, FontFamily, FontId, Margin, Response, RichText, Rounding, Stroke, TextStyle, Ui,
};

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
/// The bazaar (E3.11): its lanterns' amber, warm across the avenue from the
/// hall's blue. The shop panel's edge and its calls to action.
pub const BAZAAR: Color32 = rgb(0xe8a24a);
/// A person's lantern and a citizen's (`entities.rs`): a person's card (S0.3)
/// is edged in the light they carry.
pub const LANTERN: Color32 = rgb(crate::entities::LANTERN);
pub const CITIZEN_LANTERN: Color32 = rgb(crate::entities::CITIZEN_LANTERN);
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

/// A tool's wear bar (E1.7): pale tempered steel while it's sound, ember
/// below 20% (`rules::tools::PLATEAU_FLOOR`), where its work starts to
/// suffer, and breathing towards white heat below 5% (`CLIFF_EDGE`).
pub const WEAR: Color32 = rgb(0xc8d3df);
pub const EMBER: Color32 = rgb(0xf26b1d);
pub const EMBER_HOT: Color32 = rgb(0xffc56b);
/// The bar's empty track.
pub const WEAR_TRACK: Color32 = rgba(0, 0, 0, 0.55);

/// Where a craft's quality comes from, in the breakdown bar (E1.7): the
/// inputs in the crafting violet, lighter for each one after the first, the
/// station in its orange, the tool in steel, and the roll a pale band.
pub const SOURCE_INPUT: Color32 = CRAFT;
pub const SOURCE_STATION: Color32 = STATION;
pub const SOURCE_TOOL: Color32 = rgb(0x94a3b8);
pub const SOURCE_ROLL: Color32 = rgba(240, 240, 255, 0.4);
/// The breakdown bar's empty track, and a grade boundary's tick on it.
pub const BAR_TRACK: Color32 = rgba(0, 0, 0, 0.35);
pub const GRADE_TICK: Color32 = rgba(255, 255, 255, 0.22);

/// The dev overlay's frame graph (D5.2): a frame within its 60 fps budget
/// in dim gold, one over it in ember, one over the 30 fps line in red, and
/// the faint rules at those two lines.
pub const FRAME_OK: Color32 = GOLD_DIM;
pub const FRAME_LATE: Color32 = EMBER;
pub const FRAME_HITCH: Color32 = SHORT;
pub const GRAPH_GUIDE: Color32 = rgba(255, 255, 255, 0.16);
/// The well's colour made opaque, behind a guide's label.
pub const WELL_SOLID: Color32 = rgb(0x1a1a26);

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
/// A key cap's face, its edge and its side (`keycap`, P8.3).
pub const KEYCAP: Color32 = rgba(48, 48, 64, 0.95);
pub const KEYCAP_EDGE: Color32 = rgba(110, 110, 135, 0.55);
pub const KEYCAP_SIDE: Color32 = rgba(10, 10, 16, 0.9);
/// The narrowest cap, so a lone letter isn't a sliver.
pub const KEYCAP_MIN_WIDTH: f32 = 22.0;

/// The Gazette's newsprint (E3.9): a warm paper, its deckle edge, and the
/// inks. The one light page in a dark game, so it reads as a thing you hold.
pub const PAPER: Color32 = rgb(0xf0e6cf);
pub const PAPER_EDGE: Color32 = rgb(0xc9b893);
pub const INK: Color32 = rgb(0x231e17);
pub const INK_DIM: Color32 = rgb(0x6b6150);
/// The column rules and the masthead's lines
pub const INK_RULE: Color32 = rgba(35, 30, 23, 0.55);
/// A price that rose, and one that fell: the inks a printer has, green and red
/// gone dark enough to read on paper
pub const INK_UP: Color32 = rgb(0x2f6b3a);
pub const INK_DOWN: Color32 = rgb(0x9b2f25);

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

// Family names from `fonts/fonts.toml`.
#[cfg(test)]
const BODY_FONT: &str = "body";
const STRONG_FONT: &str = "strong";
const DISPLAY_FONT: &str = "display";

/// Text in the body face, egui's default.
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

/// Makes `ctx` look like the Capitol. Call it at the top of every frame's
/// UI; the style goes in once. The fonts are Flint's: it installs `fonts/`
/// before the first frame, with `body` as egui's default face (the `[egui]`
/// table in `fonts/fonts.toml`), and keeps them across its reloads.
pub fn install(ctx: &egui::Context) {
    let id = egui::Id::new("capitol_theme");
    if ctx.data(|d| d.get_temp::<bool>(id)).is_none() {
        ctx.all_styles_mut(style);
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
}

/// A context with the fonts Flint would give it, for tests that draw the
/// theme without a player.
#[cfg(test)]
pub fn test_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(flint_player::project_font_definitions(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/capitol.scene.toml"
    )));
    ctx
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
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
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
    v.selection.stroke = Stroke::new(1.0_f32, GOLD);
    v.text_cursor.stroke = Stroke::new(1.5_f32, GOLD);

    let round = Rounding::same(4.0);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.noninteractive.rounding = round;

    w.inactive.bg_fill = FIELD;
    w.inactive.weak_bg_fill = FIELD;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.rounding = round;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = PILL;
    w.hovered.weak_bg_fill = PILL;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, GOLD_DIM);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.rounding = round;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = PILL;
    w.active.weak_bg_fill = PILL;
    w.active.bg_stroke = Stroke::new(1.0_f32, GOLD);
    w.active.fg_stroke = Stroke::new(1.0_f32, GOLD);
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
        .stroke(Stroke::new(1.0_f32, BORDER))
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
    panel().stroke(Stroke::new(1.0_f32, edge.gamma_multiply(0.3)))
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
        .stroke(Stroke::new(1.0_f32, edge))
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
            .stroke(Stroke::new(1.0_f32, edge))
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
        Stroke::new(1.5_f32, GOLD.gamma_multiply(0.9 * k)),
    );
    painter.rect_stroke(
        rect.expand(2.5 + 2.0 * (1.0 - k)),
        rounding + 2.5,
        Stroke::new(2.0_f32, GOLD.gamma_multiply(0.3 * k)),
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
        .stroke(Stroke::new(1.0_f32, ERROR))
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

/// A scrolled column that runs on below its foot (#235): the foot fades
/// into the panel, and the scroll handle shows at rest, not only on hover.
/// `add` draws the column; it scrolls in at most `room` points.
pub fn scrolled<R>(
    ui: &mut Ui,
    id: &str,
    room: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    const FOOT: f32 = 28.0;
    ui.scope(|ui| {
        ui.spacing_mut().scroll.dormant_handle_opacity = 0.5;
        let out = egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(room)
            // A drag is for the slots; the wheel scrolls.
            .drag_to_scroll(false)
            .show(ui, add);
        let rect = out.inner_rect;
        if out.state.offset.y + rect.height() < out.content_size.y - 1.0 {
            let foot = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - FOOT),
                rect.right_bottom(),
            );
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(foot.left_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(foot.right_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(foot.left_bottom(), PANEL);
            mesh.colored_vertex(foot.right_bottom(), PANEL);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(1, 3, 2);
            ui.painter().add(mesh);
        }
        out.inner
    })
    .inner
}

/// A hairline across the space in `colour`: a header's rule in its accent.
pub fn rule_in(ui: &mut Ui, colour: Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0_f32, colour));
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
            Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(fade)),
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
    .stroke(Stroke::new(2.0_f32, accent))
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

/// The range a craft's quality will land in, and its grades, tinted by the
/// grade of the middle: "58–64 C", or "52–58 D–C" across a boundary.
pub fn range_badge(ui: &mut Ui, (low, high): (i32, i32)) -> Response {
    if low == high {
        return quality_badge(ui, low);
    }
    let (from, to) = (
        capitol_protocol::rules::quality_grade(low),
        capitol_protocol::rules::quality_grade(high),
    );
    let grades = if from == to { to.to_string() } else { format!("{from}–{to}") };
    let colour = grade(capitol_protocol::rules::quality_grade((low + high) / 2));
    tag(ui, &format!("{low}–{high} {grades}"), colour)
}

/// A known quality and its grade, tinted by the grade: "82 B".
pub fn quality_badge(ui: &mut Ui, quality: i32) -> Response {
    grade_tag(ui, &quality.to_string(), quality)
}

fn grade_tag(ui: &mut Ui, number: &str, quality: i32) -> Response {
    let letter = capitol_protocol::rules::quality_grade(quality);
    tag(ui, &format!("{number} {letter}"), grade(letter))
}

fn tag(ui: &mut Ui, text: &str, colour: Color32) -> Response {
    egui::Frame::none()
        .fill(colour.gamma_multiply(0.2))
        .rounding(4.0)
        .inner_margin(Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .font(strong(size::SMALL))
                    .color(colour),
            )
        })
        .response
}

/// A citizen's mark (C0.2) before their name: the slender lozenge, a lamp's
/// pane, that the HUD draws in their lantern's pale (`hud.rhai`). `text` is
/// the name's size. With `ink` it's an outline in that ink, for paper, and a
/// little fuller, as a thin line of ink is fainter than a lamp; without, it's
/// filled in the pale, as in the world.
pub fn citizen_mark(ui: &mut Ui, text: f32, ink: Option<Color32>) -> Response {
    let r = crate::labels::GLYPH_R * if ink.is_some() { 1.3 } else { 1.0 } * text;
    let w = r * 0.72;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(w * 2.0 + 2.0, text), egui::Sense::hover());
    // Level with the middle of the lower-case letters, as in the world
    let c = rect.center() + egui::vec2(0.0, text * 0.06);
    let points = vec![
        c + egui::vec2(0.0, -r),
        c + egui::vec2(w, 0.0),
        c + egui::vec2(0.0, r),
        c + egui::vec2(-w, 0.0),
    ];
    let shape = match ink {
        Some(ink) => egui::Shape::convex_polygon(points, Color32::TRANSPARENT, Stroke::new(1.2_f32, ink)),
        None => egui::Shape::convex_polygon(points, CITIZEN_LANTERN, Stroke::NONE),
    };
    ui.painter().add(shape);
    response.on_hover_text("A citizen: an AI who lives, works and trades in the city, by the same rules as anyone")
}

/// A wax seal of radius `r` at `c`: a red-brown disc, its rim pressed into
/// points, and the stamp's ring. The impound's chip and letters (S1.5) set it.
pub fn wax_seal(painter: &egui::Painter, c: egui::Pos2, r: f32) {
    let rim: Vec<egui::Pos2> = (0..16)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 16.0;
            let r = if i % 2 == 0 { r } else { r * 0.86 };
            c + egui::vec2(a.cos() * r, a.sin() * r)
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(rim, INK_DOWN, Stroke::NONE));
    painter.circle_stroke(c, r * 0.52, Stroke::new((r * 0.14).max(0.8), PAPER_EDGE.gamma_multiply(0.7)));
}

/// An envelope filling `rect`: sealed, a paper face with its flap down and a
/// seal on it; opened, an outline in `ink` with its flap up.
pub fn envelope(painter: &egui::Painter, rect: egui::Rect, sealed: bool, ink: Color32) {
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let mid = rect.center().x;
    let line = Stroke::new(1.0_f32, if sealed { PAPER_EDGE } else { ink });
    if sealed {
        painter.rect_filled(rect, 1.5, PAPER);
        painter.add(egui::Shape::line(
            vec![egui::pos2(l, t), egui::pos2(mid, t + rect.height() * 0.58), egui::pos2(r, t)],
            Stroke::new(1.0_f32, INK_DIM.gamma_multiply(0.6)),
        ));
        wax_seal(painter, egui::pos2(mid, t + rect.height() * 0.58), rect.height() * 0.3);
    } else {
        painter.rect_stroke(rect, 1.5, line);
        // The flap, folded back above it
        painter.add(egui::Shape::line(
            vec![egui::pos2(l, t), egui::pos2(mid, t - rect.height() * 0.45), egui::pos2(r, t)],
            line,
        ));
        painter.add(egui::Shape::line(
            vec![egui::pos2(l, b), egui::pos2(mid, t + rect.height() * 0.45), egui::pos2(r, b)],
            Stroke::new(1.0_f32, ink.gamma_multiply(0.5)),
        ));
    }
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

/// A key, or a mouse action, as a small raised cap: "Enter", "Click". The
/// shortcut overlay and the first-session hints (P8.3) set keys in these.
pub fn keycap(ui: &mut Ui, text: &str) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), strong(size::SMALL), TEXT);
    let pad = egui::vec2(7.0, 3.0);
    let width = (galley.size().x + pad.x * 2.0).max(KEYCAP_MIN_WIDTH);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, galley.size().y + pad.y * 2.0 + 2.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    // The cap's side, a shade darker, shows under its face.
    painter.rect_filled(rect, 4.0, KEYCAP_SIDE);
    let face = egui::Rect::from_min_max(rect.min, rect.max - egui::vec2(0.0, 2.0));
    painter.rect(face, 4.0, KEYCAP, Stroke::new(1.0_f32, KEYCAP_EDGE));
    painter.galley(face.center() - galley.size() * 0.5, galley, TEXT);
    response
}

/// How far along a run of steps is, as pips: gold for each done, a gold
/// ring for the one at hand, and dim dots for the rest.
pub fn step_pips(ui: &mut Ui, steps: usize, at: usize) {
    let r = 3.5;
    let gap = 7.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(steps as f32 * (r * 2.0 + gap) - gap, r * 2.0 + 2.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    for i in 0..steps {
        let c = egui::pos2(
            rect.left() + r + i as f32 * (r * 2.0 + gap),
            rect.center().y,
        );
        match i.cmp(&at) {
            std::cmp::Ordering::Less => {
                painter.circle_filled(c, r, GOLD);
            }
            std::cmp::Ordering::Equal => {
                painter.circle_filled(c, r, GOLD_WASH);
                painter.circle_stroke(c, r, Stroke::new(1.5_f32, GOLD));
            }
            std::cmp::Ordering::Greater => {
                painter.circle_filled(c, r - 1.0, BORDER);
            }
        }
    }
}

/// A tick, drawn rather than set in type, in `colour`, filling `rect`.
pub fn tick(painter: &egui::Painter, rect: egui::Rect, colour: Color32) {
    let p = |x: f32, y: f32| rect.min + egui::vec2(rect.width() * x, rect.height() * y);
    let stroke = Stroke::new(2.0_f32, colour);
    painter.line_segment([p(0.1, 0.55), p(0.4, 0.85)], stroke);
    painter.line_segment([p(0.4, 0.85), p(0.92, 0.18)], stroke);
}

/// Text fields as dark wells with a dim gold edge that turns gold on focus,
/// for a form that's mostly fields (the login form).
pub fn gold_fields(ui: &mut Ui) {
    let v = ui.visuals_mut();
    v.extreme_bg_color = BG_DARK;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(0.7));
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, GOLD_DIM);
}

// ---------------------------------------------------------------------------
// Settings controls (P8.4)
// ---------------------------------------------------------------------------

/// A settings row's label column, in points.
pub const ROW_LABEL: f32 = 112.0;
/// A settings row's height, in points.
const ROW_HEIGHT: f32 = 24.0;

/// A settings row: `name` in the label column, then the control.
pub fn setting_row<R>(ui: &mut Ui, name: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(ROW_LABEL, ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(ROW_LABEL);
                ui.label(text(name).size(size::BODY));
            },
        );
        add(ui)
    })
    .inner
}

/// An on/off switch: a gold track with the knob right when on, a dark
/// one with it left when off. The knob slides between them.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(36.0, 20.0), egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.12);
    let hovered = response.hovered();
    let painter = ui.painter();
    let track = lerp_colour(FIELD, GOLD.gamma_multiply(0.85), t);
    let edge = if hovered {
        GOLD
    } else {
        lerp_colour(GOLD_DIM.gamma_multiply(0.6), GOLD, t)
    };
    painter.rect(rect, 10.0, track, Stroke::new(1.0_f32, edge));
    let r = rect.height() / 2.0 - 3.0;
    let x = egui::lerp(rect.left() + r + 3.0..=rect.right() - r - 3.0, t);
    painter.circle_filled(
        egui::pos2(x, rect.center().y),
        r,
        lerp_colour(DIM, BG_DARK, t),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One of a few choices, side by side, the chosen one in gold.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    choices: &[(T, &str)],
) -> Response {
    ui.spacing_mut().item_spacing.x = 0.0;
    let mut joined: Option<Response> = None;
    for (i, &(choice, name)) in choices.iter().enumerate() {
        let chosen = *value == choice;
        let (first, last) = (i == 0, i + 1 == choices.len());
        let round = |end: bool| if end { 5.0 } else { 0.0 };
        let rounding = Rounding {
            nw: round(first),
            sw: round(first),
            ne: round(last),
            se: round(last),
        };
        let (fill, edge, ink) = if chosen {
            (GOLD_WASH, GOLD, GOLD)
        } else {
            (FIELD, GOLD_DIM.gamma_multiply(0.5), DIM)
        };
        let mut r = ui
            .add(
                egui::Button::new(RichText::new(name).size(size::SMALL).color(ink))
                    .fill(fill)
                    .stroke(Stroke::new(1.0_f32, edge))
                    .rounding(rounding)
                    .min_size(egui::vec2(36.0, 22.0)),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if r.clicked() && !chosen {
            *value = choice;
            r.mark_changed();
        }
        joined = Some(match joined {
            Some(j) => j | r,
            None => r,
        });
    }
    joined.expect("at least one choice")
}

/// A level in `range`: a dark rail filled in gold up to a gold knob, with
/// the value written after it by `show`. Drag or click anywhere on the
/// rail; the arrow keys step it while it has focus.
pub fn level(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    show: impl Fn(f32) -> String,
) -> Response {
    const READOUT: f32 = 40.0;
    let width = (ui.available_width() - READOUT - 8.0).max(80.0);
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let rail = rect.shrink2(egui::vec2(7.0, 0.0));
    if let Some(p) = response.interact_pointer_pos() {
        let t = ((p.x - rail.left()) / rail.width()).clamp(0.0, 1.0);
        let v = lo + (hi - lo) * t;
        if v != *value {
            *value = v;
            response.mark_changed();
        }
    }
    if response.has_focus() {
        let nudge = ui.input(|i| {
            i.num_presses(egui::Key::ArrowRight) as f32 - i.num_presses(egui::Key::ArrowLeft) as f32
        });
        if nudge != 0.0 {
            *value = (*value + nudge * (hi - lo) / 20.0).clamp(lo, hi);
            response.mark_changed();
        }
    }
    let t = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0);
    let hot = response.hovered() || response.dragged() || response.has_focus();
    let painter = ui.painter();
    let y = rect.center().y;
    let track = egui::Rect::from_min_max(
        egui::pos2(rail.left(), y - 2.5),
        egui::pos2(rail.right(), y + 2.5),
    );
    painter.rect(
        track,
        2.5,
        FIELD,
        Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(0.4)),
    );
    let x = egui::lerp(rail.left()..=rail.right(), t);
    let filled = egui::Rect::from_min_max(track.min, egui::pos2(x, track.max.y));
    painter.rect_filled(
        filled,
        2.5,
        GOLD_DIM.gamma_multiply(if hot { 1.0 } else { 0.8 }),
    );
    painter.circle(
        egui::pos2(x, y),
        if hot { 7.0 } else { 6.0 },
        if hot { GOLD_LIGHT } else { GOLD },
        Stroke::new(1.5_f32, BG_DARK),
    );
    let (readout, _) =
        ui.allocate_exact_size(egui::vec2(READOUT, ROW_HEIGHT), egui::Sense::hover());
    ui.painter().text(
        readout.right_center(),
        egui::Align2::RIGHT_CENTER,
        show(*value),
        body(size::SMALL),
        if t > 0.0 { TEXT } else { DIM },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn lerp_colour(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
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

    /// Draws `add` in a frame, then clicks at `at(rect)` of what it drew.
    fn click(add: &mut dyn FnMut(&mut Ui) -> Response, at: impl Fn(egui::Rect) -> egui::Pos2) {
        let ctx = test_context();
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut frame = |events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                install(ctx);
                egui::CentralPanel::default().show(ctx, |ui| rect.set(add(ui).rect));
            });
        };
        frame(vec![]);
        frame(vec![]);
        let pos = at(rect.get());
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos)]);
        frame(vec![button(true)]);
        frame(vec![button(false)]);
        frame(vec![]);
    }

    #[test]
    fn a_switch_flips_when_clicked() {
        let mut on = true;
        click(&mut |ui| switch(ui, &mut on), |r| r.center());
        assert!(!on);
    }

    #[test]
    fn a_level_goes_where_its_rail_is_clicked() {
        let mut v = 1.0;
        // A quarter of the way along the rail, which is inset 7 points.
        click(
            &mut |ui| level(ui, &mut v, 0.0..=1.0, |v| format!("{v}")),
            |r| egui::pos2(r.left() + 7.0 + (r.width() - 14.0) * 0.25, r.center().y),
        );
        assert!((v - 0.25).abs() < 0.02, "{v}");
    }

    #[test]
    fn a_segment_is_chosen_when_clicked() {
        let mut v = 1;
        // The last of three.
        click(
            &mut |ui| {
                let choices = [(0, "Off"), (1, "Low"), (2, "High")];
                ui.horizontal(|ui| segmented(ui, &mut v, &choices)).inner
            },
            |r| egui::pos2(r.right() - 4.0, r.center().y),
        );
        assert_eq!(v, 2);
    }

    /// Every family the theme names is one `fonts/fonts.toml` gives Flint,
    /// and `body` leads egui's default: an unknown family panics when egui
    /// lays text out in it. Text lays out on the first frame.
    #[test]
    fn the_theme_draws_in_flints_fonts_from_the_first_frame() {
        let ctx = test_context();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            install(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label(title("The Capitol", size::TITLE));
                ui.label(heading("Inventory"));
                ui.label(label("Email"));
                ui.label(text("Fiber ×12"));
            });
        });
        ctx.fonts(|f| {
            let fonts = f.lock();
            let families = &fonts.fonts.definitions().families;
            let face = |name: &str| families[&FontFamily::Name(name.into())][0].clone();
            assert_eq!(face(BODY_FONT), "SourceSans3-Regular");
            assert_eq!(families[&FontFamily::Proportional][0], face(BODY_FONT));
            assert_eq!(face(STRONG_FONT), "SourceSans3-Semibold");
            assert_eq!(face(DISPLAY_FONT), "Cinzel");
        });
    }
}
