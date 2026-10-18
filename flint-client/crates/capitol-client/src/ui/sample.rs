//! Two sample panels made only of `theme`'s parts, for reviewing the theme
//! against the React client (P5.1). `--ui-sample` shows them over the
//! world. They copy the React crafting and inventory panels' layout
//! (`CraftingPanel.tsx`, `InventoryPanel.tsx`) with made-up contents, so a
//! capture of each client can be laid side by side. They do nothing, except
//! that the slots are the real slot grid (P5.3): drags between them move,
//! merge and swap here as the server would, so the widget can be tried
//! offline.
//!
//! `tools/theme-sample.html` is the same pair in the React client's own CSS.

use super::icons::ItemIcons;
use super::slots::{Move, SlotGrid};
use super::theme::{self, size, CRAFT, DIM, ENOUGH, SHORT, SLOT_GAP, TEXT};
use capitol_core::protocol::rest::SlotItem;
use capitol_core::protocol::rules::quality_grade;
use capitol_core::state::Container;
use egui::RichText;
use uuid::Uuid;

/// What the sample panels hold between frames.
pub struct Sample {
    recipe: usize,
    quantity: String,
    /// Inventory, crafting input, crafting output.
    containers: [Container; 3],
}

impl Default for Sample {
    fn default() -> Self {
        let item = |n: u128, slot: i32, kind: &str, name: &str, quality: i32, quantity: i32| {
            SlotItem::new(
                Uuid::from_u128(n),
                kind.into(),
                name.into(),
                quality,
                quantity,
                slot,
            )
        };
        let id = |n: u128| Uuid::from_u128(0x5a3d_0000 + n);
        Self {
            recipe: 1,
            quantity: "12".into(),
            containers: [
                Container::new(
                    id(1),
                    "player_inventory",
                    24,
                    6,
                    &[
                        item(1, 0, "fiber", "Plant Fiber", 92, 12),
                        item(2, 1, "fiber", "Plant Fiber", 78, 9),
                        item(3, 2, "rope", "Rope", 64, 3),
                        item(4, 6, "fiber", "Plant Fiber", 41, 4),
                        item(5, 7, "workbench_kit", "Workbench Kit", 70, 1),
                        item(6, 8, "chest_kit", "Storage Chest Kit", 22, 1),
                        item(7, 13, "forge_kit", "Forge Kit", 88, 1),
                    ],
                ),
                Container::new(
                    id(2),
                    "crafting_input",
                    4,
                    2,
                    &[item(8, 0, "fiber", "Plant Fiber", 81, 3)],
                ),
                Container::new(id(3), "crafting_output", 1, 1, &[]),
            ],
        }
    }
}

const RECIPES: [&str; 3] = ["Select or auto-detect", "Rope", "Workbench Kit"];

impl Sample {
    pub fn show(&mut self, ctx: &egui::Context, icons: &mut ItemIcons) {
        let mut moved = None;
        egui::Area::new(egui::Id::new("capitol_sample_crafting"))
            .anchor(egui::Align2::LEFT_BOTTOM, [20.0, -20.0])
            .show(ctx, |ui| {
                theme::panel_edged(CRAFT).show(ui, |ui| {
                    ui.set_width(276.0); // React's 300, less the padding
                    moved = moved.or(self.crafting(ui, icons));
                });
            });
        egui::Area::new(egui::Id::new("capitol_sample_inventory"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0])
            .show(ctx, |ui| {
                theme::panel_edged(CRAFT).show(ui, |ui| {
                    moved = moved.or(super::inventory::body(
                        ui,
                        icons,
                        Some(&self.containers[0]),
                        None,
                    )
                    .moved);
                });
            });
        if let Some(m) = moved {
            apply(&mut self.containers, m);
        }
    }

    fn crafting(&mut self, ui: &mut egui::Ui, icons: &mut ItemIcons) -> Option<Move> {
        let mut moved = None;
        theme::header(ui, "Crafting", Some(CRAFT), None);
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(theme::dim("Recipe:"));
            egui::ComboBox::from_id_salt("sample_recipe")
                .width(ui.available_width())
                .selected_text(theme::text(RECIPES[self.recipe]))
                .show_ui(ui, |ui| {
                    for (i, name) in RECIPES.iter().enumerate() {
                        ui.selectable_value(&mut self.recipe, i, *name);
                    }
                });
        });
        ui.add_space(6.0);

        theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::list_spacing(ui);
            ui.label(theme::label("Required"));
            for (i, (name, have, need)) in [("Fiber", 5, 3), ("Resin", 0, 1)].iter().enumerate() {
                if i > 0 {
                    theme::divider(ui);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(*name).color(TEXT).size(size::SMALL));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ok = have >= need;
                        let colour = if ok { ENOUGH } else { SHORT };
                        ui.label(RichText::new(if ok { "✓" } else { "✗" }).color(colour));
                        ui.label(
                            RichText::new(format!("/{need}"))
                                .color(DIM)
                                .monospace()
                                .size(size::LABEL),
                        );
                        ui.label(
                            RichText::new(have.to_string())
                                .color(colour)
                                .monospace()
                                .size(size::LABEL),
                        );
                    });
                });
            }
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            // Centred, as React's flex row is: two slots, the arrow, one slot.
            let width = 3.0 * theme::SLOT_SIZE + SLOT_GAP + 2.0 * 16.0 + 24.0 + 4.0 * 8.0;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            ui.vertical(|ui| {
                ui.label(theme::label("Inputs"));
                moved = moved.or(SlotGrid::new(&self.containers[1]).show(ui, icons));
            });
            ui.add_space(6.0);
            ui.label(RichText::new("→").color(CRAFT).size(size::GLYPH));
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(theme::label("Output"));
                moved = moved.or(SlotGrid::new(&self.containers[2]).show(ui, icons));
            });
        });
        ui.add_space(6.0);

        theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 130.0) / 2.0).max(0.0));
                ui.label(theme::dim("Preview:"));
                ui.label(theme::text("Rope"));
                theme::range_badge(ui, (79, 85));
            });
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label(theme::dim("Make"));
            ui.add(egui::TextEdit::singleline(&mut self.quantity).desired_width(48.0));
            theme::button(ui, "Max", false);
        });
        ui.add_space(6.0);
        theme::accent_button(ui, "Craft Rope", CRAFT, true);
        ui.add_space(8.0);
        theme::progress(ui, 0.62, CRAFT);
        ui.add_space(4.0);
        theme::divider(ui);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Rope")
                    .font(theme::strong(size::BODY))
                    .color(CRAFT),
            );
            ui.label(theme::dim("Time: 3s"));
        });
        moved
    }
}

/// A move as the server makes it (`backend/src/routes/inventory.rs`): into
/// an empty slot it moves, onto the same type it merges, with the quality
/// averaged by quantity, and onto anything else the two swap.
fn apply(containers: &mut [Container; 3], m: Move) {
    let from = containers.iter().enumerate().find_map(|(c, con)| {
        con.slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|i| i.item_id == m.item_id))
            .map(|s| (c, s))
    });
    let Some((fc, fs)) = from else {
        return;
    };
    let Some(tc) = containers
        .iter()
        .position(|c| c.id == m.target_container_id)
    else {
        return;
    };
    let ts = m.target_slot as usize;
    if ts >= containers[tc].slots.len() || (fc, fs) == (tc, ts) {
        return;
    }
    let mut source = containers[fc].slots[fs].take().expect("found above");
    match containers[tc].slots[ts].take() {
        None => {}
        Some(mut target) if target.item_type == source.item_type => {
            let quantity = source.quantity + target.quantity;
            target.quality =
                (source.quality * source.quantity + target.quality * target.quantity) / quantity;
            target.quality_grade = quality_grade(target.quality).into();
            target.quantity = quantity;
            containers[tc].slots[ts] = Some(target);
            return;
        }
        Some(mut target) => {
            target.slot_index = fs as i32;
            containers[fc].slots[fs] = Some(target);
        }
    }
    source.slot_index = ts as i32;
    containers[tc].slots[ts] = Some(source);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &Sample, c: usize, slot: usize) -> Option<(&str, i32, i32)> {
        s.containers[c]
            .slot(slot)
            .map(|i| (i.item_type.as_str(), i.quality, i.quantity))
    }

    fn mv(s: &Sample, from: (usize, usize), to: (usize, usize)) -> Move {
        Move {
            item_id: s.containers[from.0].slot(from.1).unwrap().item_id,
            target_container_id: s.containers[to.0].id,
            target_slot: to.1 as i32,
        }
    }

    #[test]
    fn the_sample_moves_merges_and_swaps_like_the_server() {
        let mut s = Sample::default();
        let m = mv(&s, (0, 2), (0, 3));
        apply(&mut s.containers, m);
        assert_eq!(at(&s, 0, 2), None);
        assert_eq!(at(&s, 0, 3), Some(("rope", 64, 3)));

        // 12 at 92 onto 9 at 78: 21 at (1104 + 702) / 21 = 86.
        let m = mv(&s, (0, 0), (0, 1));
        apply(&mut s.containers, m);
        assert_eq!(at(&s, 0, 0), None);
        assert_eq!(at(&s, 0, 1), Some(("fiber", 86, 21)));
        assert_eq!(s.containers[0].slot(1).unwrap().quality_grade, "B");

        let m = mv(&s, (0, 3), (1, 0));
        apply(&mut s.containers, m);
        assert_eq!(at(&s, 1, 0), Some(("rope", 64, 3)));
        assert_eq!(at(&s, 0, 3), Some(("fiber", 81, 3)));
        assert_eq!(s.containers[0].slot(3).unwrap().slot_index, 3);
    }
}
