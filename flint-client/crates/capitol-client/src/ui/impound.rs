//! The impound (#141): what a foreclosure, or a lot laid under your
//! workbench, gave back when your pack had no room. The header's Impound
//! chip shows while it holds anything and opens this panel.
//!
//! It reads as a clerk's docket: a parchment edge and a red-brown HELD
//! stamp, since what's here was taken with the land and kept, not lost.
//! Things only come out. Drag a stack to an empty slot in your pack, or
//! take everything with the button, which joins stacks of a kind as the
//! market box's Collect does. A drop onto the impound does nothing: the
//! server refuses anything going in (`container_access::impound_refusal`).

use super::hud::grouped;
use super::icons::ItemIcons;
use super::slots::Slot;
use super::theme::{self, size, DIM, INK_DOWN, PAPER, PAPER_EDGE, SLOT_GAP, SLOT_SIZE};
use capitol_core::protocol::ws::ItemCount;
use capitol_core::state::Container;
use egui::{Align2, Id, RichText, Stroke};

/// Stacks a row, as in the inventory, so the two line up.
const COLUMNS: usize = 6;
/// Rows shown before the stacks scroll: the inventory's four.
const ROWS: usize = 4;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// How far it slides as it opens, in points.
const RISE: f32 = 12.0;
/// The stamp's tilt, in radians.
const STAMP_TILT: f32 = -0.12;

/// What the panel asks for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Everything to the pack, as far as it fits
    TakeAll,
    Close,
}

/// What the panel draws from.
pub struct View<'a> {
    /// Your impound, if you have one.
    pub impound: Option<&'a Container>,
    /// Your inventory, for the room left in it.
    pub inventory: Option<&'a Container>,
    /// A take is out.
    pub busy: bool,
    /// Why the last take or move was refused, while it's fresh.
    pub refused: Option<&'a str>,
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct ImpoundPanel {
    pub open: bool,
    /// Where the panel was drawn this frame. `None` while it's closed.
    pub rect: Option<egui::Rect>,
}

impl ImpoundPanel {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Draws the panel just left of `beside` (the inventory panel, bottom
    /// right) while that's open, so stacks drag straight across, or else
    /// top right under the header.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        icons: &mut ItemIcons,
        view: &View,
        beside: Option<egui::Rect>,
    ) -> Option<Action> {
        // Emptied, it has nothing to say: it closes, and the chip goes
        if self.open && view.impound.is_some_and(|c| c.used_slots() == 0) {
            self.open = false;
        }
        let shown = ctx.animate_bool_with_time(Id::new("capitol_impound_fade"), self.open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let rise = RISE * (1.0 - ease);
        let (align, at) = match beside {
            Some(pack) => {
                let screen = ctx.screen_rect();
                (Align2::RIGHT_BOTTOM, [pack.left() - screen.right() - 12.0, pack.bottom() - screen.bottom() + rise])
            }
            None => (Align2::RIGHT_TOP, [-24.0, ctx.available_rect().top() + 12.0 - rise]),
        };
        let area = egui::Area::new(Id::new("capitol_impound"))
            .anchor(align, at)
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(PAPER_EDGE).show(ui, |ui| body(ui, icons, view)).inner
            });
        self.rect = self.open.then_some(area.response.rect);
        area.inner
    }
}

fn body(ui: &mut egui::Ui, icons: &mut ItemIcons, view: &View) -> Option<Action> {
    let width = COLUMNS as f32 * SLOT_SIZE + (COLUMNS - 1) as f32 * SLOT_GAP + 16.0;
    ui.set_width(width);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(theme::heading("Impound").color(PAPER));
        ui.add_space(6.0);
        stamp(ui, "HELD");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui.add(egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM)).frame(false));
            if close.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Close").clicked() {
                action = Some(Action::Close);
            }
        });
    });
    ui.label(
        RichText::new("Taken down with your foreclosed land, and kept for you. Nothing goes back in.")
            .italics()
            .color(DIM)
            .size(size::SMALL),
    );
    ui.add_space(8.0);

    let held: Vec<_> = view.impound.map_or_else(Vec::new, |c| c.items().collect());
    if held.is_empty() {
        ui.label(theme::dim("Nothing is held for you."));
        return action;
    }
    let impound = view.impound.map(|c| c.id).unwrap_or_default();
    let tall = ROWS as f32 * (SLOT_SIZE + SLOT_GAP) + 12.0;
    egui::ScrollArea::vertical().max_height(tall).show(ui, |ui| {
        theme::tray().show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(SLOT_GAP, SLOT_GAP);
            for row in held.chunks(COLUMNS) {
                ui.horizontal(|ui| {
                    for item in row {
                        // A drop here would be a deposit, which the server
                        // refuses: it's dropped
                        let _ = Slot {
                            container_id: impound,
                            index: item.slot_index.max(0) as usize,
                            item: Some(item),
                            enabled: !view.busy,
                        }
                        .show(ui, icons);
                    }
                });
            }
        });
    });

    ui.add_space(6.0);
    let units: i64 = held.iter().map(|i| i64::from(i.quantity)).sum();
    let room = view.inventory.map(|c| c.slot_count() - c.used_slots());
    let stacks = format!(
        "{} {}, {} in all",
        held.len(),
        if held.len() == 1 { "stack" } else { "stacks" },
        grouped(units)
    );
    let tally = match room {
        Some(1) => format!("{stacks} · 1 slot free in your pack"),
        Some(room) => format!("{stacks} · {room} slots free in your pack"),
        None => stacks,
    };
    ui.label(theme::dim(tally).size(size::SMALL));
    if let Some(why) = view.refused {
        ui.add_space(4.0);
        theme::error_box(ui, why);
    }
    ui.add_space(6.0);
    let can = !view.busy;
    let take = theme::accent_button(ui, "Take all to your pack", PAPER_EDGE, can)
        .on_hover_text("As far as it fits. A stack joins one of its kind in your pack");
    if take.clicked() && can {
        action = Some(Action::TakeAll);
    }
    ui.add_space(2.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("or drag a stack to an empty slot").color(DIM).size(size::LABEL));
    });
    action
}

/// A clerk's rubber stamp: `text` in red-brown capitals in a ruled box,
/// tilted a little, as if pressed by hand.
fn stamp(ui: &mut egui::Ui, text: &str) {
    let font = theme::strong(size::SMALL);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, INK_DOWN);
    let pad = egui::vec2(6.0, 2.0);
    let (rect, _) = ui.allocate_exact_size(galley.size() + pad * 2.0 + egui::vec2(4.0, 4.0), egui::Sense::hover());
    let c = rect.center();
    let (s, k) = STAMP_TILT.sin_cos();
    let turn = |v: egui::Vec2| c + egui::vec2(k * v.x - s * v.y, s * v.x + k * v.y);
    let half = galley.size() / 2.0 + pad;
    let corners = [
        turn(egui::vec2(-half.x, -half.y)),
        turn(egui::vec2(half.x, -half.y)),
        turn(egui::vec2(half.x, half.y)),
        turn(egui::vec2(-half.x, half.y)),
    ];
    let ink = INK_DOWN.gamma_multiply(1.6);
    ui.painter().add(egui::Shape::closed_line(corners.to_vec(), Stroke::new(1.5, ink)));
    let at = turn(-galley.size() / 2.0);
    ui.painter()
        .add(egui::epaint::TextShape::new(at, galley, ink).with_angle(STAMP_TILT));
}

/// An impound for captures (`--impound-sample`): planks, rope, a chest's
/// kit and a sickle, as a foreclosed workshop leaves them.
pub fn sample() -> (Container, Container) {
    use capitol_core::protocol::rest::SlotItem;
    use uuid::Uuid;
    let item = |n: u128, kind: &str, name: &str, quality, quantity, slot| {
        SlotItem::new(Uuid::from_u128(n), kind.into(), name.into(), quality, quantity, slot)
    };
    let impound = Container::new(
        Uuid::from_u128(300),
        "impound",
        480,
        8,
        &[
            item(301, "plank", "Plank", 71, 140, 0),
            item(302, "chest_kit", "Storage Chest Kit", 64, 1, 1),
            item(303, "rope", "Rope", 58, 12, 2),
            item(304, "iron_ingot", "Iron Ingot", 82, 9, 3),
            item(305, "stone_sickle", "Stone Sickle", 50, 1, 4),
            item(306, "fiber", "Plant Fiber", 44, 60, 5),
            item(307, "workbench_kit", "Workbench Kit", 55, 1, 6),
        ],
    );
    let inventory = Container::new(
        Uuid::from_u128(310),
        "player_inventory",
        24,
        6,
        &[item(311, "fiber", "Plant Fiber", 40, 20, 0)],
    );
    (impound, inventory)
}

/// `counts` in words for a toast: the first two by name, then how many
/// more kinds ("140 Plank, 12 Rope and 3 more").
pub fn counted(catalog: &capitol_core::catalog::Catalog, counts: &[ItemCount]) -> String {
    let named: Vec<String> = counts
        .iter()
        .take(2)
        .map(|c| format!("{} {}", grouped(i64::from(c.quantity)), catalog.item_name(&c.item_type)))
        .collect();
    match counts.len() {
        0 => String::new(),
        1 => named[0].clone(),
        2 => format!("{} and {}", named[0], named[1]),
        n => format!("{}, {} and {} more", named[0], named[1], n - 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::catalog::Catalog;

    fn count(item_type: &str, quantity: i32) -> ItemCount {
        ItemCount {
            item_type: item_type.into(),
            quantity,
        }
    }

    #[test]
    fn a_toast_names_two_and_counts_the_rest() {
        let catalog = Catalog::fallback();
        let plank = catalog.item_name("plank");
        let rope = catalog.item_name("rope");
        assert_eq!(counted(&catalog, &[count("plank", 1400)]), format!("1,400 {plank}"));
        assert_eq!(
            counted(&catalog, &[count("plank", 140), count("rope", 12)]),
            format!("140 {plank} and 12 {rope}")
        );
        assert_eq!(
            counted(&catalog, &[count("plank", 140), count("rope", 12), count("fiber", 3), count("iron_ingot", 9)]),
            format!("140 {plank}, 12 {rope} and 2 more")
        );
    }
}
