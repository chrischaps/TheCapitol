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
use capitol_core::protocol::rules::stacking::{Pick, Units};
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
                        // A B that spans A to D: its pip is ringed, and its
                        // tooltip lists its units (E1.8)
                        item(2, 1, "fiber", "Plant Fiber", 77, 9)
                            .with_units(Units::from_pairs([(44, 2), (81, 3), (92, 4)])),
                        item(3, 2, "rope", "Rope", 64, 3),
                        item(4, 6, "fiber", "Plant Fiber", 41, 4),
                        item(5, 7, "workbench_kit", "Workbench Kit", 70, 1),
                        item(6, 8, "chest_kit", "Storage Chest Kit", 22, 1),
                        item(7, 13, "forge_kit", "Forge Kit", 88, 1),
                        // Six qualities: its tooltip draws a bar
                        item(9, 9, "fiber", "Plant Fiber", 70, 20).with_units(Units::from_pairs([
                            (38, 2),
                            (52, 3),
                            (66, 4),
                            (79, 5),
                            (88, 3),
                            (96, 3),
                        ])),
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
                    moved = moved.take().or(self.crafting(ui, icons));
                });
            });
        egui::Area::new(egui::Id::new("capitol_sample_inventory"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0])
            .show(ctx, |ui| {
                theme::panel_edged(CRAFT).show(ui, |ui| {
                    moved = moved.take().or(super::inventory::body(
                        ui,
                        icons,
                        &capitol_core::catalog::Catalog::fallback(),
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
                moved = moved.take().or(SlotGrid::new(&self.containers[1]).show(ui, icons));
            });
            ui.add_space(6.0);
            ui.label(RichText::new("→").color(CRAFT).size(size::GLYPH));
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(theme::label("Output"));
                moved = moved.take().or(SlotGrid::new(&self.containers[2]).show(ui, icons));
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

/// A move as the server makes it (`backend/src/engine/slot_moves.rs`): into
/// an empty slot it moves, onto the same type it merges, every unit keeping
/// its quality (E1.8), and onto anything else the two swap. A split takes
/// part of a stack, the units named or the lowest, into an empty slot of
/// its own container.
fn apply(containers: &mut [Container; 3], m: Move) {
    if let Some(quantity) = m.split {
        split(containers, &m, quantity);
        return;
    }
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
        Some(target) if target.item_type == source.item_type => {
            let mut units = target.units.clone();
            if !units.merge(&source.units) {
                containers[fc].slots[fs] = Some(source);
                containers[tc].slots[ts] = Some(target);
                return;
            }
            containers[tc].slots[ts] = Some(target.with_units(units));
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

/// `quantity` of `m.item_id` into the empty `m.target_slot` of its own
/// container: `m.qualities`, or the lowest.
fn split(containers: &mut [Container; 3], m: &Move, quantity: i32) {
    let Some(c) = containers
        .iter_mut()
        .find(|c| c.id == m.target_container_id)
    else {
        return;
    };
    let ts = m.target_slot as usize;
    if c.slots.get(ts).is_none_or(|s| s.is_some()) {
        return;
    }
    let Some(source) = c.slots.iter_mut().flatten().find(|i| i.item_id == m.item_id) else {
        return;
    };
    if quantity < 1 || quantity >= source.quantity {
        return;
    }
    let mut left = source.units.clone();
    let taken = match &m.qualities {
        Some(named) if named.count() == quantity as i64 => left.take_units(named),
        Some(_) => None,
        None => left.take(quantity, Pick::default()),
    };
    let Some(taken) = taken else {
        return;
    };
    let mut new = source.clone().with_units(taken);
    *source = source.clone().with_units(left);
    // A new id the sample won't have used: the source's, marked with the slot
    new.item_id = Uuid::from_u128(source.item_id.as_u128() | ((ts as u128 + 1) << 64));
    new.slot_index = ts as i32;
    c.slots[ts] = Some(new);
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
            split: None,
            qualities: None,
        }
    }

    #[test]
    fn the_sample_moves_merges_and_swaps_like_the_server() {
        let mut s = Sample::default();
        let m = mv(&s, (0, 2), (0, 3));
        apply(&mut s.containers, m);
        assert_eq!(at(&s, 0, 2), None);
        assert_eq!(at(&s, 0, 3), Some(("rope", 64, 3)));

        // 12 at 92 onto 2 × 44, 3 × 81, 4 × 92: 21 at (88 + 243 + 1472) / 21 = 85,
        // and every unit keeps its quality
        let m = mv(&s, (0, 0), (0, 1));
        apply(&mut s.containers, m);
        assert_eq!(at(&s, 0, 0), None);
        assert_eq!(at(&s, 0, 1), Some(("fiber", 85, 21)));
        let merged = s.containers[0].slot(1).unwrap();
        assert_eq!(merged.quality_grade, "B");
        assert_eq!(merged.units, Units::from_pairs([(44, 2), (81, 3), (92, 16)]));
        assert_eq!(merged.min_quality, 44);

        let m = mv(&s, (0, 3), (1, 0));
        apply(&mut s.containers, m);
        assert_eq!(at(&s, 1, 0), Some(("rope", 64, 3)));
        assert_eq!(at(&s, 0, 3), Some(("fiber", 81, 3)));
        assert_eq!(s.containers[0].slot(3).unwrap().slot_index, 3);
    }

    #[test]
    fn the_sample_splits_like_the_server() {
        let mut s = Sample::default();
        // The lowest four of 2 × 44, 3 × 81, 4 × 92: (88 + 162) / 4 = 62
        let m = Move {
            split: Some(4),
            ..mv(&s, (0, 1), (0, 3))
        };
        apply(&mut s.containers, m);
        assert_eq!(at(&s, 0, 3), Some(("fiber", 62, 4)));
        assert_eq!(at(&s, 0, 1), Some(("fiber", 89, 5)));
        let new = s.containers[0].slot(3).unwrap();
        assert_eq!((new.units.clone(), new.slot_index), (Units::from_pairs([(44, 2), (81, 2)]), 3));
        assert_ne!(new.item_id, s.containers[0].slot(1).unwrap().item_id);

        // Named qualities: two of the 92s
        let m = Move {
            split: Some(2),
            qualities: Some(Units::uniform(2, 92)),
            ..mv(&s, (0, 1), (0, 4))
        };
        apply(&mut s.containers, m);
        assert_eq!(at(&s, 0, 4), Some(("fiber", 92, 2)));
        assert_eq!(s.containers[0].slot(1).unwrap().units, Units::from_pairs([(81, 1), (92, 2)]));

        // Not into a full slot, never the whole stack, and not units it lacks
        let before = at(&s, 0, 1).map(|(k, q, n)| (k.to_string(), q, n));
        for (to, quantity, qualities) in [
            ((0, 2), 1, None),
            ((0, 5), 3, None),
            ((0, 5), 1, Some(Units::uniform(1, 44))),
        ] {
            let m = Move {
                split: Some(quantity),
                qualities,
                ..mv(&s, (0, 1), to)
            };
            apply(&mut s.containers, m);
        }
        assert_eq!(at(&s, 0, 1).map(|(k, q, n)| (k.to_string(), q, n)), before);
        assert_eq!(at(&s, 0, 5), None);
    }
}
