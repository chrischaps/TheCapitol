//! Two sample panels made only of `theme`'s parts, for reviewing the theme
//! against the React client (P5.1). `--ui-sample` shows them over the
//! world. They copy the React crafting and inventory panels' layout
//! (`CraftingPanel.tsx`, `InventoryPanel.tsx`) with made-up contents, so a
//! capture of each client can be laid side by side. They do nothing.
//!
//! `tools/theme-sample.html` is the same pair in the React client's own CSS.

use super::theme::{self, size, CRAFT, DIM, ENOUGH, SHORT, SLOT_GAP, TEXT};
use egui::RichText;

/// What the sample panels hold between frames.
pub struct Sample {
    recipe: usize,
    quantity: String,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            recipe: 1,
            quantity: "12".into(),
        }
    }
}

const RECIPES: [&str; 3] = ["Select or auto-detect", "Rope", "Workbench Kit"];

impl Sample {
    pub fn show(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("capitol_sample_crafting"))
            .anchor(egui::Align2::LEFT_BOTTOM, [20.0, -20.0])
            .show(ctx, |ui| {
                theme::panel_edged(CRAFT).show(ui, |ui| {
                    ui.set_width(276.0); // React's 300, less the padding
                    self.crafting(ui);
                });
            });
        egui::Area::new(egui::Id::new("capitol_sample_inventory"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0])
            .show(ctx, |ui| {
                theme::panel_edged(CRAFT).show(ui, |ui| {
                    inventory(ui);
                });
            });
    }

    fn crafting(&mut self, ui: &mut egui::Ui) {
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
                slots(ui, 2, 2);
            });
            ui.add_space(6.0);
            ui.label(RichText::new("→").color(CRAFT).size(size::GLYPH));
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(theme::label("Output"));
                slots(ui, 1, 1);
            });
        });
        ui.add_space(6.0);

        theme::well().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 130.0) / 2.0).max(0.0));
                ui.label(theme::dim("Preview:"));
                ui.label(theme::text("Rope"));
                theme::grade_badge(ui, 82);
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
    }
}

fn inventory(ui: &mut egui::Ui) {
    ui.set_width(6.0 * theme::SLOT_SIZE + 5.0 * SLOT_GAP + 16.0);
    theme::header(ui, "Inventory", None, Some(("7/24", theme::PILL)));
    slots(ui, 6, 4);
    ui.add_space(2.0);
    theme::divider(ui);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("31 items").color(DIM).size(size::LABEL));
    });
}

fn slots(ui: &mut egui::Ui, columns: usize, rows: usize) {
    theme::tray().show(ui, |ui| {
        egui::Grid::new(ui.next_auto_id())
            .spacing([SLOT_GAP, SLOT_GAP])
            .show(ui, |ui| {
                for _ in 0..rows {
                    for _ in 0..columns {
                        theme::empty_slot(ui);
                    }
                    ui.end_row();
                }
            });
    });
}
