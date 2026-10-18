//! The station panel (P6.3), bottom right beside the inventory while a
//! station is open, as React's `StationPanel.tsx`. Side by side, a drag
//! between the station's box and the inventory is a short move across. Clicking a station opens it (`input.rhai`, click
//! priority 3, and `capitol_core::station::Visit`). The panel closes with
//! its Close button, and when you walk out of reach.
//!
//! - **The station's box**, a slot grid like every other, so a drag to or
//!   from the inventory (bottom right) just works. A chest is only this.
//! - **At a crafting station:** its recipes (`required_station_type`), what
//!   the one picked or detected needs against the box, Fill and Clear as on
//!   the crafting panel, the quality range, and Craft, which sends
//!   `craft_at_station`. While it runs: the bar, the time left and Cancel.
//!   The rules are `capitol_core::craft`, as for crafting by hand.
//! - **Where the output goes.** The server puts a station's output in your
//!   own crafting output, not the station's box. React doesn't say so; this
//!   shows that slot under the box, so you can see it arrive and drag it
//!   out.
//! - **Remove**, for the owner, once the box is empty. The server refuses
//!   while it holds anything and puts the kit back in your inventory (#88),
//!   so taking a station down is how you move it, and it doesn't ask twice.

use super::crafting::{blocked_box, craft_label, needs, preview, under_way, Fill};
use super::icons::ItemIcons;
use super::slots::{Move, SlotGrid};
use super::theme::{self, size, DIM, STATION};
use capitol_core::craft::{self, Check, Phase};
use capitol_core::protocol::rest::RecipeInfo;
use capitol_core::state::{Activity, GameState};
use capitol_core::station;
use egui::{Align2, Id, RichText};
use uuid::Uuid;

/// The recipe column's width: the crafting panel's.
const WIDTH: f32 = 276.0;
/// The box column's width: four slots, their gaps and the tray's margin.
/// Every seeded station lays its box out in four columns.
const BOX_WIDTH: f32 = 4.0 * theme::SLOT_SIZE + 3.0 * theme::SLOT_GAP + 16.0;
/// The space between the two columns.
const COLUMN_GAP: f32 = 16.0;
/// The space between the panel and the inventory beside it, in points.
const GAP: f32 = 12.0;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;

/// What the panel asks for this frame.
pub enum Action {
    Move(Move),
    /// Several moves at once (Fill, Clear, or clearing the output).
    Moves(Vec<craft::Shift>),
    Craft,
    Cancel,
    Close,
    Remove(Uuid),
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct StationPanel {
    /// The station the panel last showed. Opening another resets the pick.
    showing: Option<Uuid>,
    /// The recipe picked by hand. `None` lets the box choose.
    picked: Option<String>,
    bar: Fill,
}

/// What the panel draws from.
pub struct View<'a> {
    pub state: &'a GameState,
    pub phase: &'a Phase,
    /// Why the last craft or move was refused, if it was just now.
    pub refused: Option<&'a str>,
    /// Where the inventory panel is, if it's open: the panel sits to its left.
    pub beside: Option<egui::Rect>,
}

impl StationPanel {
    /// The recipe the panel is set to: the one picked, else the one the
    /// station's box makes.
    pub fn recipe<'a>(&self, state: &'a GameState) -> Option<&'a RecipeInfo> {
        let open = state.open_station()?;
        let here = || station::recipes_at(state.recipes(), &open.station_type);
        match &self.picked {
            Some(id) => here().find(|r| &r.id == id),
            None => craft::detect(here(), state.station_container()?),
        }
    }

    /// The recipe weighed against the station's box and your output slot.
    pub fn check<'a>(&self, state: &'a GameState) -> Option<Check<'a>> {
        Some(craft::check(
            self.recipe(state)?,
            state.station_container()?,
            state.crafting_output(),
        ))
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        icons: &mut ItemIcons,
        view: &View,
        dt: f32,
    ) -> Option<Action> {
        let crafting = match view.state.local_activity() {
            Some(a @ Activity::Crafting { .. }) => Some(a),
            _ => None,
        };
        self.bar.update(crafting, dt);
        let open = view.state.open_station().map(|o| o.station_id);
        if open != self.showing {
            self.showing = open;
            self.picked = None;
        }
        let shown =
            ctx.animate_bool_with_time(Id::new("capitol_station_fade"), open.is_some(), FADE);
        if open.is_none() || shown <= 0.0 {
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        // Bottom right, to the left of the inventory when it's open.
        let right = match view.beside {
            Some(inventory) => ctx.screen_rect().right() - inventory.left() + GAP,
            None => 20.0,
        };
        let mut action = None;
        egui::Area::new(Id::new("capitol_station"))
            .anchor(Align2::RIGHT_BOTTOM, [-right, -20.0])
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(STATION).show(ui, |ui| {
                    let storage = view
                        .state
                        .open_station()
                        .is_some_and(|o| station::is_storage(view.state.catalog(), &o.station_type));
                    ui.set_width(if storage {
                        BOX_WIDTH
                    } else {
                        BOX_WIDTH + COLUMN_GAP + WIDTH
                    });
                    action = self.body(ui, icons, view, crafting);
                });
            });
        action
    }

    fn body(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        crafting: Option<&Activity>,
    ) -> Option<Action> {
        let state = view.state;
        let open = state.open_station()?;
        let storage = station::is_storage(state.catalog(), &open.station_type);
        let running = crafting.is_some();
        let mut action = None;

        let badge = if running {
            let t = ui.input(|i| i.time) as f32;
            ui.ctx().request_repaint();
            Some((
                "In progress",
                STATION.gamma_multiply(0.45 + 0.15 * (t * 4.2).sin()),
            ))
        } else if storage {
            Some(("Storage", theme::PILL))
        } else {
            None
        };
        theme::header(ui, &open.name, Some(STATION), badge);
        ui.add_space(2.0);

        // The box on the left, open to drags only while nothing is out, and
        // at a crafting station, the recipe's column on the right.
        let idle = *view.phase == Phase::Idle && !running;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(BOX_WIDTH);
                ui.label(theme::label(if storage {
                    "Storage"
                } else {
                    "Station box"
                }));
                match state.station_container() {
                    Some(c) => {
                        if let Some(m) = SlotGrid::new(c).enabled(idle).show(ui, icons) {
                            action = Some(Action::Move(m));
                        }
                    }
                    None => {
                        ui.label(theme::dim("Loading…"));
                    }
                }
                if storage {
                    ui.add_space(4.0);
                    ui.label(
                        theme::dim("Drag items in to store them, and out to take them")
                            .size(size::LABEL),
                    );
                    if let Some(why) = view.refused {
                        ui.add_space(6.0);
                        theme::error_box(ui, why);
                    }
                }
            });
            if !storage {
                ui.add_space(COLUMN_GAP);
                ui.vertical(|ui| {
                    ui.set_width(WIDTH);
                    if let Some(a) = self.crafting(ui, icons, view, crafting) {
                        action = Some(a);
                    }
                });
            }
        });

        ui.add_space(6.0);
        theme::divider(ui);
        ui.add_space(2.0);
        if let Some(a) = self.footer(ui, view) {
            action = Some(a);
        }
        action
    }

    /// A crafting station's half: the recipe, its needs, Fill and Clear,
    /// where the output goes, the estimate and Craft. Or the craft under way.
    fn crafting(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        crafting: Option<&Activity>,
    ) -> Option<Action> {
        let state = view.state;
        let open = state.open_station()?;
        let mut action = None;

        if let Some(Activity::Crafting {
            recipe_name,
            duration,
            ..
        }) = crafting
        {
            if under_way(ui, self.bar.shown, recipe_name, *duration, STATION) {
                action = Some(Action::Cancel);
            }
            if let Some(why) = view.refused {
                ui.add_space(6.0);
                theme::error_box(ui, why);
            }
            return action;
        }

        if station::recipes_at(state.recipes(), &open.station_type)
            .next()
            .is_none()
        {
            ui.vertical_centered(|ui| {
                ui.label(theme::dim(format!(
                    "Nothing is made at a {} yet",
                    open.name.to_lowercase()
                )));
            });
            return None;
        }
        self.picker(ui, state, &open.station_type);
        ui.add_space(6.0);

        let check = self.check(state);
        if let Some(check) = &check {
            needs(ui, check);
            ui.add_space(4.0);
        }
        let idle = *view.phase == Phase::Idle;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(idle, |ui| {
                let fill = match (&check, state.inventory(), state.station_container()) {
                    (Some(c), Some(inv), Some(bench)) if !c.satisfied() => {
                        craft::fill(c.recipe, inv, bench)
                    }
                    _ => Vec::new(),
                };
                let clear = match (state.station_container(), state.inventory()) {
                    (Some(bench), Some(inv)) => craft::empty_into(bench, inv),
                    _ => Vec::new(),
                };
                ui.add_space(((ui.available_width() - 150.0) / 2.0).max(0.0));
                if ui
                    .add_enabled(!fill.is_empty(), egui::Button::new(theme::text("Fill")))
                    .on_hover_text("Bring what the recipe needs from your inventory, best first")
                    .clicked()
                {
                    action = Some(Action::Moves(fill));
                }
                if ui
                    .add_enabled(!clear.is_empty(), egui::Button::new(theme::text("Clear")))
                    .on_hover_text("Put everything in the station's box back in your inventory")
                    .clicked()
                {
                    action = Some(Action::Moves(clear));
                }
            });
        });

        // Where the output lands: your own output slot.
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(theme::label("Output"));
                ui.label(
                    theme::dim("Made here, it lands in your crafting output").size(size::LABEL),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(out) = state.crafting_output() {
                    if let Some(m) = SlotGrid::new(out).enabled(idle).show(ui, icons) {
                        action = Some(Action::Move(m));
                    }
                }
            });
        });

        if let Some(check) = &check {
            if let Some(blocked) = &check.blocked {
                ui.add_space(6.0);
                if let Some(shifts) = blocked_box(ui, state, check, blocked) {
                    action = Some(Action::Moves(shifts));
                }
            } else if let Some(range) = check.range {
                ui.add_space(6.0);
                preview(ui, check, range);
            }
        }
        ui.add_space(6.0);
        let (label, enabled) = craft_label(check.as_ref(), view.phase, state);
        let button = theme::accent_button(ui, &label, STATION, enabled);
        let button = match check.as_ref().and_then(|c| c.recipe.description.as_deref()) {
            Some(d) => button.on_hover_text(d),
            None => button,
        };
        if button.clicked() {
            action = Some(Action::Craft);
        }
        if let Some(why) = view.refused {
            ui.add_space(6.0);
            theme::error_box(ui, why);
        }
        action
    }

    /// The recipe list: "Auto: …" first, then the station's recipes.
    fn picker(&mut self, ui: &mut egui::Ui, state: &GameState, station_type: &str) {
        let detected = state
            .station_container()
            .and_then(|c| craft::detect(station::recipes_at(state.recipes(), station_type), c));
        let auto = match detected {
            Some(r) => format!("Auto: {}", r.name),
            None => "Select or auto-detect".to_owned(),
        };
        let shown = match &self.picked {
            Some(id) => state
                .recipe(id)
                .map_or_else(|| auto.clone(), |r| r.name.clone()),
            None => auto.clone(),
        };
        ui.horizontal(|ui| {
            ui.label(theme::dim("Recipe:"));
            egui::ComboBox::from_id_salt("capitol_station_recipe")
                .width(ui.available_width())
                .selected_text(theme::text(shown))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.picked, None, auto);
                    for r in station::recipes_at(state.recipes(), station_type) {
                        ui.selectable_value(&mut self.picked, Some(r.id.clone()), &r.name);
                    }
                });
        });
    }

    /// Remove (the owner's, once the box is empty) and Close.
    fn footer(&mut self, ui: &mut egui::Ui, view: &View) -> Option<Action> {
        let state = view.state;
        let open = state.open_station()?;
        let mine = state
            .station(open.station_id)
            .is_some_and(|s| Some(s.owner_id) == state.me());
        let empty = state
            .station_container()
            .is_some_and(|c| c.used_slots() == 0);
        let mut action = None;
        ui.horizontal(|ui| {
            if mine {
                let tip = if empty {
                    "Take the station down. Its kit goes back in your inventory"
                } else {
                    "Empty its box first"
                };
                let link = ui
                    .add_enabled(
                        empty && view.state.local_activity().is_none(),
                        egui::Label::new(RichText::new("Remove").color(DIM).size(size::SMALL))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(tip)
                    .on_disabled_hover_text(tip);
                if link.clicked() {
                    action = Some(Action::Remove(open.station_id));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::button(ui, "Close", false).clicked() {
                    action = Some(Action::Close);
                }
            });
        });
        action
    }
}
