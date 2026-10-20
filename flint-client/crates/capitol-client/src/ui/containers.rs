//! `--containers`: any container the player holds that no panel shows, as
//! slot grids (P5.3). The inventory (P5.4), crafting (P5.5) and station
//! (P6.3) panels have taken every type the server sends today, so this is
//! empty unless a new type appears. It sits bottom left, above the controls
//! hint, clear of the panels.

use super::icons::ItemIcons;
use super::slots::{Move, SlotGrid};
use super::theme::{self, size, CRAFT, DIM};
use capitol_core::catalog::container_type;
use capitol_core::state::{Container, GameState};

/// Draws the bench, bottom left.
pub fn bench(
    ctx: &egui::Context,
    icons: &mut ItemIcons,
    state: &GameState,
    refused: Option<&str>,
) -> Option<Move> {
    let mut action = None;
    egui::Area::new(egui::Id::new("capitol_containers"))
        .anchor(egui::Align2::LEFT_BOTTOM, [20.0, -150.0])
        .show(ctx, |ui| {
            theme::panel_edged(CRAFT).show(ui, |ui| {
                // The inventory panel's width, so the two line up.
                ui.set_width(6.0 * theme::SLOT_SIZE + 5.0 * theme::SLOT_GAP + 16.0);
                theme::header(ui, "Containers", None, None);
                let mut moved = None;
                let mut grid = |ui: &mut egui::Ui, name: &str, c: Option<&Container>| {
                    ui.label(theme::label(name));
                    match c {
                        Some(c) => {
                            if let Some(m) = SlotGrid::new(c).show(ui, icons) {
                                moved = Some(m);
                            }
                        }
                        None => {
                            ui.label(theme::dim("not loaded"));
                        }
                    }
                };
                // Anything else the server hands us, so nothing is hidden.
                let known = [
                    container_type::PLAYER_INVENTORY,
                    container_type::CRAFTING_INPUT,
                    container_type::CRAFTING_OUTPUT,
                    container_type::IMPOUND,
                ];
                let mut others: Vec<_> = state
                    .containers()
                    .values()
                    .filter(|c| !known.contains(&c.container_type.as_str()))
                    .collect();
                others.sort_by_key(|c| c.id);
                if others.is_empty() {
                    ui.label(theme::dim("Every container has its panel"));
                }
                for c in others {
                    grid(ui, &c.container_type, Some(c));
                }
                if moved.is_some() {
                    action = moved;
                }
                if let Some(why) = refused {
                    ui.add_space(4.0);
                    theme::error_box(ui, why);
                }
                ui.add_space(2.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "drag to move · onto a stack to merge · onto another to swap",
                        )
                        .color(DIM)
                        .size(size::LABEL),
                    );
                });
            });
        });
    action
}
