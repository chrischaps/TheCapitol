//! `--containers`: the containers the player holds that no panel shows yet,
//! as slot grids, so drag and drop can be played against the server before
//! the panels exist (P5.3). The inventory panel (P5.4) has taken its own;
//! crafting (P5.5) and the station panel (P6.3) take theirs next, and then
//! this goes. Until P6.3 it also lists the stations nearby, to open one and
//! try its box. It sits top right, clear of the inventory panel, so a drag
//! between the two works.

use super::icons::ItemIcons;
use super::slots::{Move, SlotGrid};
use super::theme::{self, size, CRAFT, DIM};
use capitol_core::catalog::container_type;
use capitol_core::state::{Container, GameState};
use uuid::Uuid;

/// What the bench asks for.
pub enum BenchAction {
    Move(Move),
    OpenStation(Uuid),
    CloseStation,
}

/// Draws the bench, bottom right.
pub fn bench(
    ctx: &egui::Context,
    icons: &mut ItemIcons,
    state: &GameState,
    refused: Option<&str>,
) -> Option<BenchAction> {
    let mut action = None;
    egui::Area::new(egui::Id::new("capitol_containers"))
        .anchor(egui::Align2::RIGHT_TOP, [-20.0, 60.0])
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
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| grid(ui, "Crafting input", state.crafting_input()));
                    ui.vertical(|ui| grid(ui, "Output", state.crafting_output()));
                });
                match state.open_station() {
                    Some(open) => {
                        grid(ui, &open.name, state.station_container());
                        if theme::button(ui, "Close station", false).clicked() {
                            action = Some(BenchAction::CloseStation);
                        }
                    }
                    None => {
                        let mut stations: Vec<_> = state.stations().values().collect();
                        stations.sort_by_key(|s| s.id);
                        if !stations.is_empty() {
                            ui.label(theme::label("Stations nearby"));
                        }
                        for s in stations.into_iter().take(4) {
                            ui.horizontal(|ui| {
                                ui.label(theme::text(&s.name).size(size::SMALL));
                                if theme::link(ui, "open", size::SMALL).clicked() {
                                    action = Some(BenchAction::OpenStation(s.id));
                                }
                            });
                        }
                    }
                }
                // Anything else the server hands us, so nothing is hidden.
                let known = [
                    container_type::PLAYER_INVENTORY,
                    container_type::CRAFTING_INPUT,
                    container_type::CRAFTING_OUTPUT,
                ];
                let mut others: Vec<_> = state
                    .containers()
                    .values()
                    .filter(|c| !known.contains(&c.container_type.as_str()))
                    .collect();
                others.sort_by_key(|c| c.id);
                for c in others {
                    grid(ui, &c.container_type, Some(c));
                }
                if let Some(m) = moved {
                    action = Some(BenchAction::Move(m));
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
