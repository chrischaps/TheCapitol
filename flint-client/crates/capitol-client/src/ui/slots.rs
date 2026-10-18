//! The slot grid (P5.3): any container drawn as React draws one
//! (`InventorySlot.tsx`, `InventoryGrid.tsx`), with drag and drop between
//! any two grids on screen.
//!
//! A slot shows its item's icon (36 in 48, P5.2), the quantity bottom right
//! and a quality pip top right in the grade's colour (plan §4.7). A blend,
//! a stack whose min grade isn't its grade, rings its pip in the min's
//! colour (E1.2). Hovering one shows the name, the quality and the grade,
//! and what a blend was blended from.
//!
//! Dragging uses egui's drag-and-drop payload, so a drag can start in one
//! panel and end in another without either knowing about the other. A drop
//! on another slot comes back from [`SlotGrid::show`] as a [`Move`]. The
//! caller sends it (`Session::move_item`), and the server decides whether
//! it moves, merges or swaps. A drop onto a stack of the same type blends
//! the two whatever their grades: that's the player's call (plan D2).
//! Nothing changes here until the refetch that follows (`poll.rs`), so the
//! grid never shows a state the server didn't.
//!
//! **Splitting (E1.2).** A shift-drag onto an empty slot of the same grid
//! opens a count field under the slot, set to half the stack. Enter or
//! Split sends the move with [`Move::split`]; Escape or ✕ lets it go.
//! Anywhere else, a shift-drag is a plain drag: the server splits only
//! within a container ([#190](https://github.com/chrischaps/TheCapitol/issues/190)).
//! A capture can't hold Shift for egui (#189), so the rig below checks it.
//!
//! A grid made [`SlotGrid::placing`] also reports a right click on a
//! `*_kit` ([`SlotGrid::show_clicks`]), and its tooltip says so. That's
//! how a station is placed (P6.2), as React's `InventorySlot.tsx` does it.

use super::icons::ItemIcons;
use super::theme::{self, size, CRAFT, SLOT, SLOT_EDGE, SLOT_EDGE_FULL, SLOT_GAP, SLOT_SIZE};
use capitol_core::catalog::Catalog;
use capitol_core::protocol::rest::SlotItem;
use capitol_core::protocol::rules::quality_grade;
use capitol_core::state::Container;
use egui::epaint::Shadow;
use egui::{Color32, CursorIcon, Id, Painter, Pos2, Rect, Sense, Stroke, Ui};
use uuid::Uuid;

/// A drop: put `item_id` in `target_slot` of `target_container_id`. The
/// body of `move_item`, REST or WebSocket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move {
    pub item_id: Uuid,
    pub target_container_id: Uuid,
    pub target_slot: i32,
    /// Split this many off into the slot, an empty one of the same
    /// container, rather than moving the whole stack (`split_stack`, E1.2).
    pub split: Option<i32>,
}

impl Move {
    /// The whole stack `item_id` into `target_slot` of `target_container_id`.
    pub fn whole(item_id: Uuid, target_container_id: Uuid, target_slot: i32) -> Self {
        Self {
            item_id,
            target_container_id,
            target_slot,
            split: None,
        }
    }
}

/// What a drag carries: the item, where it came from, and whether shift
/// was down as it started (a split).
#[derive(Debug, Clone)]
struct Held {
    item: SlotItem,
    container_id: Uuid,
    slot: usize,
    split: bool,
}

/// A split waiting on its count: the stack, where it goes, and how many.
/// One at a time, in egui's memory, drawn by the grid it's for.
#[derive(Debug, Clone)]
struct PendingSplit {
    item: SlotItem,
    container_id: Uuid,
    slot: usize,
    quantity: i32,
    /// The target slot, to stand the field under
    at: Rect,
}

impl PendingSplit {
    fn id() -> Id {
        Id::new("capitol_pending_split")
    }

    fn get(ctx: &egui::Context) -> Option<Self> {
        ctx.data(|d| d.get_temp(Self::id()))
    }

    fn set(ctx: &egui::Context, pending: Option<Self>) {
        ctx.data_mut(|d| match pending {
            Some(p) => d.insert_temp(Self::id(), p),
            None => d.remove::<Self>(Self::id()),
        });
    }
}

/// What a split starts at: half, rounded down, and at least one.
pub fn half(quantity: i32) -> i32 {
    (quantity / 2).max(1)
}

/// What a grid's slots were asked this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clicks {
    /// A drop onto one of them.
    pub moved: Option<Move>,
    /// A right click on a kit, with [`SlotGrid::placing`].
    pub place: Option<SlotItem>,
}

/// One container as a grid of slots, `layout_columns` wide.
pub struct SlotGrid<'a> {
    container: &'a Container,
    enabled: bool,
    /// Set by [`SlotGrid::placing`]: the catalog that says what's a kit.
    placing: Option<&'a Catalog>,
}

impl<'a> SlotGrid<'a> {
    pub fn new(container: &'a Container) -> Self {
        Self {
            container,
            enabled: true,
            placing: None,
        }
    }

    /// A right click on a kit asks to place its station (the inventory's
    /// grid). A kit is what `catalog` pairs with a station type (#181). See
    /// [`SlotGrid::show_clicks`].
    pub fn placing(mut self, catalog: &'a Catalog) -> Self {
        self.placing = Some(catalog);
        self
    }

    /// A disabled grid is dimmed, and nothing can be dragged out of it or
    /// dropped into it: crafting's inputs while a craft runs, say.
    #[allow(dead_code)] // P5.5 is the first panel to disable one
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draws the grid in its tray. Returns the move a drop onto it asks for.
    pub fn show(self, ui: &mut Ui, icons: &mut ItemIcons) -> Option<Move> {
        self.show_clicks(ui, icons).moved
    }

    /// Draws the grid in its tray. Returns the move a drop onto it asks
    /// for, and the kit a right click asks to place.
    pub fn show_clicks(self, ui: &mut Ui, icons: &mut ItemIcons) -> Clicks {
        let mut clicks = Clicks::default();
        theme::tray().show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(SLOT_GAP, SLOT_GAP);
            let columns = self.container.layout_columns.max(1);
            for row in self.container.slots.chunks(columns).enumerate() {
                let (r, items) = row;
                ui.horizontal(|ui| {
                    for (c, item) in items.iter().enumerate() {
                        let index = r * columns + c;
                        let slot = Slot {
                            container_id: self.container.id,
                            index,
                            item: item.as_ref(),
                            enabled: self.enabled,
                        };
                        let (moved, right) = slot.show_inner(ui, icons, self.placing);
                        clicks.moved = clicks.moved.or(moved);
                        if let Some(catalog) = self.placing.filter(|_| right) {
                            clicks.place = item.clone().filter(|i| is_kit(catalog, i));
                        }
                    }
                });
            }
        });
        if self.enabled {
            clicks.moved = clicks.moved.or(split_field(ui.ctx(), self.container.id));
        }
        clicks
    }
}

/// The count field of a split waiting on this container, if there is one.
/// Returns the split once it's confirmed.
fn split_field(ctx: &egui::Context, container_id: Uuid) -> Option<Move> {
    let mut pending = PendingSplit::get(ctx).filter(|p| p.container_id == container_id)?;
    let most = pending.item.quantity - 1;
    let (mut confirmed, mut dropped) = (false, false);
    egui::Area::new(Id::new(("capitol_split_field", container_id)))
        .order(egui::Order::Foreground)
        .fixed_pos(pending.at.left_bottom() + egui::vec2(0.0, 4.0))
        .show(ctx, |ui| {
            theme::panel().inner_margin(6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::dim("Split"));
                    let field = ui.add(
                        egui::DragValue::new(&mut pending.quantity)
                            .range(1..=most)
                            .speed(0.1),
                    );
                    if !field.has_focus() && !field.lost_focus() && !field.dragged() {
                        field.request_focus();
                    }
                    ui.label(theme::dim(format!("of {}", pending.item.quantity)));
                    confirmed = theme::button(ui, "Split", true).clicked();
                    dropped = ui.small_button("✕").clicked();
                });
            });
        });
    let (enter, escape) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    pending.quantity = pending.quantity.clamp(1, most);
    if confirmed || enter {
        PendingSplit::set(ctx, None);
        return Some(Move {
            item_id: pending.item.item_id,
            target_container_id: container_id,
            target_slot: pending.slot as i32,
            split: Some(pending.quantity),
        });
    }
    PendingSplit::set(ctx, (!(dropped || escape)).then_some(pending));
    None
}

/// The item a slot's drag let go of over `response` this frame, if any:
/// how a panel that isn't a container takes a drop (the trade window, P7.3).
pub fn dropped_item(response: &egui::Response) -> Option<SlotItem> {
    response
        .dnd_release_payload::<Held>()
        .map(|held| held.item.clone())
}

/// A slot's item is being dragged over `response`.
pub fn item_over(response: &egui::Response) -> bool {
    response.dnd_hover_payload::<Held>().is_some()
}

/// A slot's item is being dragged anywhere.
pub fn dragging_item(ctx: &egui::Context) -> bool {
    egui::DragAndDrop::has_payload_of_type::<Held>(ctx)
}

/// A station kit: a right click on one places its station.
fn is_kit(catalog: &Catalog, item: &SlotItem) -> bool {
    catalog.station_for_kit(&item.item_type).is_some()
}

/// One slot of a container. Grids are made of these; a panel with a lone
/// slot (crafting's output) can use one directly.
pub struct Slot<'a> {
    pub container_id: Uuid,
    pub index: usize,
    pub item: Option<&'a SlotItem>,
    pub enabled: bool,
}

impl Slot<'_> {
    pub fn show(self, ui: &mut Ui, icons: &mut ItemIcons) -> Option<Move> {
        self.show_inner(ui, icons, None).0
    }

    /// The drop onto this slot, and whether it was right-clicked. `placing`
    /// adds the kit's hint to the tooltip.
    fn show_inner(
        self,
        ui: &mut Ui,
        icons: &mut ItemIcons,
        placing: Option<&Catalog>,
    ) -> (Option<Move>, bool) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(SLOT_SIZE, SLOT_SIZE), Sense::hover());
        let id = Id::new(("capitol_slot", self.container_id, self.index));
        let draggable = self.enabled && self.item.is_some();
        let sense = if draggable {
            Sense::click_and_drag()
        } else {
            Sense::click()
        };
        let response = ui.interact(rect, id, sense);

        if let (true, Some(item)) = (draggable, self.item) {
            // Read as the drag starts: egui keeps the payload it's given then.
            let shift = ui.input(|i| i.modifiers.shift);
            response.dnd_set_drag_payload(Held {
                item: item.clone(),
                container_id: self.container_id,
                slot: self.index,
                split: shift && item.quantity > 1,
            });
        }
        let held = egui::DragAndDrop::payload::<Held>(ui.ctx());
        let is_source = held
            .as_ref()
            .is_some_and(|h| h.container_id == self.container_id && h.slot == self.index);
        let accepts = self.enabled && !is_source;
        let over = accepts && response.dnd_hover_payload::<Held>().is_some();

        let look = if !self.enabled {
            Look::Disabled
        } else if is_source {
            Look::Source
        } else if over {
            Look::Target
        } else if response.hovered() && self.item.is_some() {
            Look::Hovered
        } else {
            Look::Plain
        };
        paint_slot(ui.painter(), icons, rect, self.item, look);
        let waiting = PendingSplit::get(ui.ctx())
            .is_some_and(|p| p.container_id == self.container_id && p.slot == self.index);
        if waiting {
            dashed_edge(
                ui.painter(),
                rect.shrink(1.0),
                Stroke::new(2.0, CRAFT.gamma_multiply(0.8)),
            );
        }

        if let Some(item) = self.item {
            if held.is_none() {
                let response = if draggable {
                    response.clone().on_hover_cursor(CursorIcon::Grab)
                } else {
                    response.clone()
                };
                response.on_hover_ui(|ui| {
                    tooltip(ui, item);
                    if placing.is_some_and(|c| is_kit(c, item)) {
                        ui.label(theme::dim("Right-click to place"));
                    }
                    if draggable && item.quantity > 1 {
                        ui.label(theme::dim("Shift-drag to an empty slot to split"));
                    }
                });
            }
        }
        let right = self.item.is_some() && response.secondary_clicked();
        if is_source && response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            if let (Some(item), Some(at)) = (self.item, ui.ctx().pointer_interact_pos()) {
                let splitting = held.as_ref().is_some_and(|h| h.split);
                let shown = if splitting { half(item.quantity) } else { item.quantity };
                paint_held(ui.ctx(), icons, item, shown, at);
            }
        }

        if !accepts {
            return (None, right);
        }
        let moved = response.dnd_release_payload::<Held>().and_then(|held| {
            if held.split && held.container_id == self.container_id && self.item.is_none() {
                // Ask how many first: the grid draws the field (`split_field`)
                PendingSplit::set(
                    ui.ctx(),
                    Some(PendingSplit {
                        quantity: half(held.item.quantity),
                        item: held.item.clone(),
                        container_id: self.container_id,
                        slot: self.index,
                        at: rect,
                    }),
                );
                return None;
            }
            Some(Move::whole(
                held.item.item_id,
                self.container_id,
                self.index as i32,
            ))
        });
        (moved, right)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Plain,
    Hovered,
    /// Being dragged from: faded, with a dashed edge.
    Source,
    /// A drag is over it: lit in purple.
    Target,
    Disabled,
}

fn paint_slot(
    painter: &Painter,
    icons: &mut ItemIcons,
    rect: Rect,
    item: Option<&SlotItem>,
    look: Look,
) {
    let fade = match look {
        Look::Source => 0.4,
        Look::Disabled => 0.5,
        _ => 1.0,
    };
    let inner = rect.shrink(1.0); // a 2 px edge, drawn centred on this
    let (fill, edge) = match (look, item.is_some()) {
        (Look::Target, _) => (CRAFT.gamma_multiply(0.15), CRAFT.gamma_multiply(0.8)),
        (Look::Hovered, _) => (theme::SLOT_HOVER, CRAFT.gamma_multiply(0.6)),
        (_, true) => (SLOT, SLOT_EDGE_FULL),
        (_, false) => (SLOT, SLOT_EDGE),
    };
    if look == Look::Target {
        // React's `box-shadow: 0 0 8px rgba(139,92,246,0.4)`.
        let glow = Shadow {
            offset: egui::Vec2::ZERO,
            blur: 8.0,
            spread: 0.0,
            color: CRAFT.gamma_multiply(0.4),
        };
        painter.add(glow.as_shape(inner, 6.0));
    }
    if look == Look::Source {
        painter.rect_filled(inner, 6.0, SLOT.gamma_multiply(fade));
        dashed_edge(painter, inner, Stroke::new(2.0, edge.gamma_multiply(fade)));
    } else {
        painter.rect(
            inner,
            6.0,
            fill.gamma_multiply(fade),
            Stroke::new(2.0, edge.gamma_multiply(fade)),
        );
    }
    let Some(item) = item else {
        return;
    };
    let icon = Rect::from_center_size(
        rect.center(),
        egui::vec2(theme::SLOT_ICON, theme::SLOT_ICON),
    );
    icons.paint_on(painter, icon, &item.item_type, fade);
    if quality_grade(item.quality) == "A" && look != Look::Source {
        let time = painter.ctx().input(|i| i.time);
        glint(painter, icon, item.item_id.as_u128(), time, fade);
    }
    // React's pip: 8 px, 3 px in from the inside of the 2 px edge.
    pip(painter, rect.right_top() + egui::vec2(-9.0, 9.0), item, fade);
    quantity(
        painter,
        rect.right_bottom() + egui::vec2(-6.0, -3.0),
        item.quantity,
        fade,
    );
}

/// The pip's colour for a quality: its grade's (plan §4.7). Worked out from
/// the quality, not the server's `quality_grade`, so a merge that the
/// client hasn't refetched yet can't show a stale letter.
pub fn pip_colour(quality: i32) -> Color32 {
    theme::grade(quality_grade(quality))
}

/// A grade pip: a 4-point disc, its grade's colour lightening towards the
/// bottom right (React's 135° gradient), ringed in dark. An A glows. A
/// blend is ringed again, in its min grade's colour (E1.2).
fn pip(painter: &Painter, centre: Pos2, item: &SlotItem, fade: f32) {
    let quality = item.quality;
    let letter = quality_grade(quality);
    let (dark, light) = (pip_colour(quality), theme::grade_light(letter));
    const R: f32 = 4.0;
    if letter == "A" {
        painter.circle_filled(centre, R + 2.0, dark.gamma_multiply(0.3 * fade));
    }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, lerp(dark, light, 0.5).gamma_multiply(fade));
    const N: u32 = 20;
    for i in 0..N {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        let d = egui::vec2(a.cos(), a.sin());
        // Projected onto the gradient's axis (down and right), 0 to 1.
        let t = ((d.x + d.y) / std::f32::consts::SQRT_2 + 1.0) / 2.0;
        mesh.colored_vertex(centre + d * R, lerp(dark, light, t).gamma_multiply(fade));
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % N);
    }
    painter.add(mesh);
    painter.circle_stroke(
        centre,
        R,
        Stroke::new(1.0, theme::PIP_EDGE.gamma_multiply(fade)),
    );
    if item.lot().is_blended() {
        painter.circle_stroke(
            centre,
            R + 1.75,
            Stroke::new(1.5, pip_colour(item.min_quality).gamma_multiply(fade)),
        );
    }
}

/// How often an A's slot glints, and how long a glint lasts (s).
const GLINT_EVERY: f64 = 4.5;
const GLINT_LIFE: f64 = 0.55;

/// An A-grade item's glint (P8.2): every few seconds a four-pointed star
/// swells and turns on the icon's upper left and is gone. Each item keeps
/// its own time, from its id, so a row of A's doesn't flash at once.
fn glint(painter: &Painter, icon: Rect, id: u128, time: f64, fade: f32) {
    let offset = (id % 997) as f64 / 997.0 * GLINT_EVERY;
    let age = (time + offset) % GLINT_EVERY;
    if age > GLINT_LIFE {
        return;
    }
    let k = (age / GLINT_LIFE) as f32;
    let swell = 1.0 - (2.0 * k - 1.0).abs();
    let centre = icon.left_top() + icon.size() * egui::vec2(0.3, 0.28);
    let r = 7.0 * swell;
    let w = r * 0.22;
    let colour = theme::grade_light("A").gamma_multiply(0.95 * fade * swell);
    let turn = k * 0.6;
    for i in 0..2 {
        let a = turn + i as f32 * std::f32::consts::FRAC_PI_2;
        let u = egui::vec2(a.cos(), a.sin());
        let n = egui::vec2(-u.y, u.x);
        painter.add(egui::Shape::convex_polygon(
            vec![
                centre + u * r,
                centre + n * w,
                centre - u * r,
                centre - n * w,
            ],
            colour,
            Stroke::NONE,
        ));
    }
    painter.circle_filled(centre, w * 1.2, Color32::WHITE.gamma_multiply(fade * swell));
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

/// The stack size, bottom right, in white over a dark shadow.
fn quantity(painter: &Painter, corner: Pos2, quantity: i32, fade: f32) {
    let font = theme::strong(size::LABEL);
    let text = quantity.to_string();
    painter.text(
        corner + egui::vec2(1.0, 1.0),
        egui::Align2::RIGHT_BOTTOM,
        &text,
        font.clone(),
        theme::INK_SHADOW.gamma_multiply(fade),
    );
    painter.text(
        corner,
        egui::Align2::RIGHT_BOTTOM,
        text,
        font,
        theme::QUANTITY.gamma_multiply(fade),
    );
}

/// A slot's edge as dashes, for the slot a drag came from.
fn dashed_edge(painter: &Painter, rect: Rect, stroke: Stroke) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(&corners, stroke, 5.0, 3.0));
}

/// The item under the pointer while it's dragged, over everything.
fn paint_held(ctx: &egui::Context, icons: &mut ItemIcons, item: &SlotItem, shown: i32, at: Pos2) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::new("capitol_slot_held"),
    ));
    let rect = Rect::from_center_size(at, egui::vec2(SLOT_SIZE, SLOT_SIZE));
    let icon = Rect::from_center_size(at, egui::vec2(theme::SLOT_ICON, theme::SLOT_ICON));
    icons.paint_on(&painter, icon, &item.item_type, 0.9);
    pip(&painter, rect.right_top() + egui::vec2(-9.0, 9.0), item, 0.9);
    quantity(
        &painter,
        rect.right_bottom() + egui::vec2(-6.0, -3.0),
        shown,
        0.9,
    );
}

/// What hovering a full slot says: the name, the quality and its grade,
/// how many, and for a blend what it was blended from.
fn tooltip(ui: &mut Ui, item: &SlotItem) {
    ui.label(
        egui::RichText::new(&item.item_name)
            .font(theme::strong(size::BODY))
            .color(theme::TEXT),
    );
    ui.horizontal(|ui| {
        ui.label(theme::dim("Quality"));
        theme::quality_badge(ui, item.quality);
        if item.quantity > 1 {
            ui.label(theme::dim(format!("× {}", item.quantity)));
        }
    });
    if item.lot().is_blended() {
        ui.horizontal(|ui| {
            ui.label(theme::dim("Blended from"));
            theme::quality_badge(ui, item.min_quality);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pips_change_colour_at_the_grade_boundaries() {
        let (a, b, c, d, f) = (
            theme::grade("A"),
            theme::grade("B"),
            theme::grade("C"),
            theme::grade("D"),
            theme::grade("F"),
        );
        for (quality, colour) in [
            (100, a),
            (90, a),
            (89, b),
            (75, b),
            (74, c),
            (55, c),
            (54, d),
            (35, d),
            (34, f),
            (0, f),
        ] {
            assert_eq!(pip_colour(quality), colour, "quality {quality}");
        }
        // The plan's palette (§4.7).
        assert_eq!(a, Color32::from_rgb(0xff, 0xd7, 0x00));
        assert_eq!(b, Color32::from_rgb(0x22, 0xc5, 0x5e));
        assert_eq!(c, Color32::from_rgb(0x3b, 0x82, 0xf6));
        assert_eq!(d, Color32::from_rgb(0xa8, 0x55, 0xf7));
        assert_eq!(f, Color32::from_rgb(0x6b, 0x72, 0x80));
    }

    // ---- drag and drop, driven through a headless egui ----
    //
    // A workaround: a capture can't play the mouse (#76), so the drag is
    // tested here, not in a screenshot.

    /// A fresh ID (the client's uuid has no `v4` feature).
    fn id() -> Uuid {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Uuid::from_u128(NEXT.fetch_add(1, Ordering::Relaxed) as u128)
    }

    fn item(slot: i32, item_type: &str, quantity: i32) -> SlotItem {
        SlotItem::new(id(), item_type.into(), item_type.into(), 80, quantity, slot)
    }

    /// An inventory (6 × 2) holding fiber in slot 0 and rope in slot 3, and
    /// a crafting input (2 × 2) holding fiber in slot 1.
    fn containers() -> (Container, Container) {
        let inventory = Container::new(
            id(),
            "player_inventory",
            12,
            6,
            &[
                item(0, "fiber", 12),
                item(3, "rope", 1),
                item(10, "forge_kit", 1),
            ],
        );
        let input = Container::new(id(), "crafting_input", 4, 2, &[item(1, "fiber", 3)]);
        (inventory, input)
    }

    struct Rig {
        ctx: egui::Context,
        icons: ItemIcons,
        inventory: Container,
        input: Container,
        /// Every slot's rect, by (container, index), from the last frame.
        rects: std::collections::HashMap<(Uuid, usize), Rect>,
        moves: Vec<Move>,
        /// Kits a right click asked to place.
        places: Vec<SlotItem>,
        /// Inventory disabled: nothing goes in or out.
        inventory_enabled: bool,
        /// Shift held, for every frame until it's let go.
        shift: bool,
    }

    impl Rig {
        fn new() -> Self {
            let (inventory, input) = containers();
            let mut rig = Rig {
                ctx: theme::test_context(),
                icons: ItemIcons::load(std::path::Path::new("no/icons/needed")),
                inventory,
                input,
                rects: Default::default(),
                moves: Vec::new(),
                places: Vec::new(),
                inventory_enabled: true,
                shift: false,
            };
            rig.frame(vec![]);
            rig.frame(vec![]); // settle the layout before the first click
            rig
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0))),
                events,
                modifiers: egui::Modifiers {
                    shift: self.shift,
                    ..Default::default()
                },
                ..Default::default()
            };
            let Rig {
                ctx,
                icons,
                inventory,
                input: crafting,
                rects,
                moves,
                places,
                inventory_enabled,
                ..
            } = self;
            let _ = ctx.run(input, |ctx| {
                theme::install(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    // The inventory places kits, as its panel's grid does.
                    let catalog = Catalog::fallback();
                    let grids = [
                        (&*inventory, *inventory_enabled, true),
                        (&*crafting, true, false),
                    ];
                    for (c, enabled, placing) in grids {
                        let mut grid = SlotGrid::new(c).enabled(enabled);
                        if placing {
                            grid = grid.placing(&catalog);
                        }
                        let clicks = grid.show_clicks(ui, icons);
                        moves.extend(clicks.moved);
                        places.extend(clicks.place);
                    }
                });
                // Where each slot landed, read back from its interact rect.
                for c in [&*inventory, &*crafting] {
                    for i in 0..c.slot_count() {
                        let id = Id::new(("capitol_slot", c.id, i));
                        if let Some(r) = ctx.read_response(id) {
                            rects.insert((c.id, i), r.rect);
                        }
                    }
                }
            });
        }

        fn centre(&self, container: Uuid, slot: usize) -> Pos2 {
            self.rects[&(container, slot)].center()
        }

        /// Press on one slot, move across in steps, release on another.
        fn drag(&mut self, from: (Uuid, usize), to: (Uuid, usize)) {
            let (a, b) = (self.centre(from.0, from.1), self.centre(to.0, to.1));
            self.frame(vec![egui::Event::PointerMoved(a)]);
            self.frame(vec![button(a, true)]);
            for i in 1..=6 {
                let p = a + (b - a) * (i as f32 / 6.0);
                self.frame(vec![egui::Event::PointerMoved(p)]);
            }
            self.frame(vec![button(b, false)]);
            self.frame(vec![]);
        }

        /// A right click on a slot.
        fn right_click(&mut self, container: Uuid, slot: usize) {
            let at = self.centre(container, slot);
            self.frame(vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                self.frame(vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
            self.frame(vec![]);
        }
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn a_right_click_on_a_kit_asks_to_place_it() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        rig.right_click(inv, 3); // rope: not a kit
        rig.right_click(inv, 1); // empty
        assert!(rig.places.is_empty());
        rig.right_click(inv, 10);
        let kit = rig.inventory.slot(10).unwrap().clone();
        assert_eq!(rig.places, [kit]);
        assert!(rig.moves.is_empty(), "a right click moves nothing");
    }

    #[test]
    fn only_a_placing_grid_places() {
        let mut rig = Rig::new();
        let kit = item(2, "chest_kit", 1);
        rig.input = Container::new(rig.input.id, "crafting_input", 4, 2, &[kit]);
        rig.frame(vec![]);
        rig.right_click(rig.input.id, 2);
        assert!(rig.places.is_empty());
    }

    #[test]
    fn dragging_within_a_grid_asks_to_move_there() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        let fiber = rig.inventory.slot(0).unwrap().item_id;
        rig.drag((inv, 0), (inv, 7));
        assert_eq!(rig.moves, [Move::whole(fiber, inv, 7)]);
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// A shift-drag to an empty slot asks how many, starting at half, and
    /// Enter sends the split (E1.2)
    #[test]
    fn a_shift_drag_to_an_empty_slot_splits_half() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        let fiber = rig.inventory.slot(0).unwrap().item_id;
        rig.shift = true;
        rig.drag((inv, 0), (inv, 7));
        rig.shift = false;
        assert!(rig.moves.is_empty(), "it waits on the count");
        assert!(PendingSplit::get(&rig.ctx).is_some());

        rig.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(
            rig.moves,
            [Move {
                split: Some(6),
                ..Move::whole(fiber, inv, 7)
            }]
        );
        assert!(PendingSplit::get(&rig.ctx).is_none());
    }

    #[test]
    fn escape_lets_a_split_go() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        rig.shift = true;
        rig.drag((inv, 0), (inv, 7));
        rig.shift = false;
        rig.frame(vec![key(egui::Key::Escape)]);
        rig.frame(vec![key(egui::Key::Enter)]);
        assert!(rig.moves.is_empty());
    }

    /// Onto a stack, into another grid, or of a single item: a plain drag
    #[test]
    fn a_shift_drag_that_cant_split_moves() {
        let mut rig = Rig::new();
        let (inv, input) = (rig.inventory.id, rig.input.id);
        let fiber = rig.inventory.slot(0).unwrap().item_id;
        let rope = rig.inventory.slot(3).unwrap().item_id;
        rig.shift = true;
        rig.drag((inv, 0), (input, 0)); // another grid
        rig.drag((inv, 3), (inv, 8)); // one rope
        rig.shift = false;
        assert_eq!(
            rig.moves,
            [Move::whole(fiber, input, 0), Move::whole(rope, inv, 8)]
        );
        assert!(PendingSplit::get(&rig.ctx).is_none());
    }

    #[test]
    fn dragging_between_grids_names_the_other_container() {
        let mut rig = Rig::new();
        let (inv, input) = (rig.inventory.id, rig.input.id);
        let rope = rig.inventory.slot(3).unwrap().item_id;
        rig.drag((inv, 3), (input, 1)); // onto the fiber: the server swaps
        let fiber = rig.input.slot(1).unwrap().item_id;
        rig.drag((input, 1), (inv, 0)); // fiber onto fiber: the server merges
        assert_eq!(
            rig.moves,
            [Move::whole(rope, input, 1), Move::whole(fiber, inv, 0)]
        );
    }

    #[test]
    fn dropping_where_it_started_or_off_a_slot_does_nothing() {
        let mut rig = Rig::new();
        let inv = rig.inventory.id;
        rig.drag((inv, 0), (inv, 0));
        assert!(rig.moves.is_empty(), "back where it started");

        let a = rig.centre(inv, 0);
        rig.frame(vec![egui::Event::PointerMoved(a)]);
        rig.frame(vec![button(a, true)]);
        let away = Pos2::new(900.0, 650.0);
        for i in 1..=4 {
            rig.frame(vec![egui::Event::PointerMoved(
                a + (away - a) * (i as f32 / 4.0),
            )]);
        }
        rig.frame(vec![button(away, false)]);
        rig.frame(vec![]);
        assert!(rig.moves.is_empty(), "dropped on nothing");
        assert!(
            !egui::DragAndDrop::has_any_payload(&rig.ctx),
            "the drag is over"
        );
    }

    #[test]
    fn empty_slots_and_disabled_grids_give_nothing_to_drag() {
        let mut rig = Rig::new();
        let (inv, input) = (rig.inventory.id, rig.input.id);
        rig.drag((inv, 5), (input, 0)); // slot 5 is empty
        assert!(rig.moves.is_empty());

        rig.inventory_enabled = false;
        rig.frame(vec![]);
        rig.drag((inv, 0), (input, 0)); // out of a disabled grid
        rig.drag((input, 1), (inv, 4)); // into one
        assert!(rig.moves.is_empty());
    }
}
