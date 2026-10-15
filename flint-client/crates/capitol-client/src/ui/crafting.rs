//! The crafting panel (P5.5), bottom left as in React (`CraftingPanel.tsx`),
//! toggled with C (the `crafting` action) or the header's Crafting chip;
//! Enter (`craft`) crafts while it's open. The rules are
//! `capitol_core::craft`; this draws them.
//!
//! - **A recipe** picked from the list, or the one the inputs make.
//! - **What it needs**, against what the input slots hold, with **Fill**
//!   to bring it from the inventory (best stacks first) and **Clear** to
//!   put the inputs back. React has neither; with them you can craft
//!   without dragging at all.
//! - **The slots**, `crafting_input` (4) → `crafting_output` (1), shut to
//!   drags while a craft is out, so nothing it uses can be moved away
//!   (B10, #61, may make the server refuse that itself).
//! - **The estimate**, labelled as one: the server's sums will change.
//! - **Output with nowhere to go** stops the craft, says what's in the way
//!   and offers to move it, because the server would destroy the output.
//! - **Under way**: the bar, the time left and Cancel. Walking cancels too
//!   (the server stops a craft on `move`), and the panel says so.

use super::icons::ItemIcons;
use super::slots::{Move, SlotGrid};
use super::theme::{self, size, CRAFT, DIM, ENOUGH, SHORT, SLOT_GAP, TEXT};
use capitol_core::craft::{self, Check, Phase};
use capitol_core::protocol::rest::RecipeInfo;
use capitol_core::protocol::rules::TICK_MS;
use capitol_core::state::{Activity, GameState};
use egui::RichText;

/// The panel's width inside its frame: React's 300 less the padding.
const WIDTH: f32 = 276.0;

/// What the panel asks for this frame.
pub enum Action {
    Move(Move),
    /// Several moves at once (Fill, Clear, or moving what blocks the output).
    Moves(Vec<craft::Shift>),
    Craft,
    Cancel,
}

/// What the panel keeps between frames.
pub struct CraftingPanel {
    pub open: bool,
    /// The recipe picked by hand; `None` lets the inputs choose.
    picked: Option<String>,
    /// The bar, eased between the server's half-second updates.
    bar: Fill,
    /// Where the panel was drawn this frame, for the controls hint to
    /// keep clear of. `None` while it's closed.
    pub rect: Option<egui::Rect>,
}

impl Default for CraftingPanel {
    fn default() -> Self {
        Self {
            open: true, // React's is always there
            picked: None,
            bar: Fill::default(),
            rect: None,
        }
    }
}

/// What the panel draws from.
pub struct View<'a> {
    pub state: &'a GameState,
    pub phase: &'a Phase,
    /// Why the last craft or move was refused, if it was just now.
    pub refused: Option<&'a str>,
}

impl CraftingPanel {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// The recipe the panel is set to: the one picked, else the one the
    /// inputs make.
    pub fn recipe<'a>(&self, state: &'a GameState) -> Option<&'a RecipeInfo> {
        let hand = || craft::hand_recipes(state.recipes());
        match &self.picked {
            Some(id) => hand().find(|r| &r.id == id),
            None => craft::detect(hand(), state.crafting_input()?),
        }
    }

    /// The recipe weighed against the boxes, for the panel and for Craft.
    pub fn check<'a>(&self, state: &'a GameState) -> Option<Check<'a>> {
        let recipe = self.recipe(state)?;
        Some(craft::check(
            recipe,
            state.crafting_input()?,
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
        self.rect = None;
        if !self.open {
            return None;
        }
        let mut action = None;
        // Top left, under the header strip (anchors are screen-relative, so
        // its height comes off here), growing down toward the controls
        // hint. React's sits bottom left, but here the hint is there.
        let top = ctx.available_rect().top();
        let shown = egui::Area::new(egui::Id::new("capitol_crafting"))
            .anchor(egui::Align2::LEFT_TOP, [16.0, top + 12.0])
            .show(ctx, |ui| {
                theme::panel_edged(CRAFT).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    action = self.body(ui, icons, view, crafting);
                });
            });
        self.rect = Some(shown.response.rect);
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
        let mut action = None;
        let running = crafting.is_some();
        let badge = running.then(|| {
            // React's pulse: the pill breathes while the craft runs.
            let t = ui.input(|i| i.time) as f32;
            ui.ctx().request_repaint();
            (
                "In progress",
                CRAFT.gamma_multiply(0.45 + 0.15 * (t * 4.2).sin()),
            )
        });
        theme::header(ui, "Crafting", Some(CRAFT), badge);
        ui.add_space(2.0);

        if let Some(Activity::Crafting {
            recipe_name,
            duration,
            ..
        }) = crafting
        {
            self.under_way(ui, recipe_name, *duration, &mut action);
            ui.add_space(6.0);
        } else {
            self.picker(ui, state);
            ui.add_space(6.0);
        }

        let check = self.check(state);
        if !running {
            if let Some(check) = &check {
                needs(ui, check);
                ui.add_space(6.0);
            }
        }

        // The slots: open to drags only while nothing is out.
        let open = *view.phase == Phase::Idle && !running;
        ui.horizontal(|ui| {
            let width = 3.0 * theme::SLOT_SIZE + SLOT_GAP + 2.0 * 16.0 + 24.0 + 4.0 * 8.0;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            ui.vertical(|ui| {
                ui.label(theme::label("Inputs"));
                match state.crafting_input() {
                    Some(c) => {
                        if let Some(m) = SlotGrid::new(c).enabled(open).show(ui, icons) {
                            action = Some(Action::Move(m));
                        }
                    }
                    None => {
                        ui.label(theme::dim("Loading…"));
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("→").color(CRAFT).size(size::GLYPH));
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(theme::label("Output"));
                match state.crafting_output() {
                    Some(c) => {
                        if let Some(m) = SlotGrid::new(c).enabled(open).show(ui, icons) {
                            action = Some(Action::Move(m));
                        }
                    }
                    None => {
                        ui.label(theme::dim("Loading…"));
                    }
                }
            });
        });

        if running {
            if let Some(why) = view.refused {
                ui.add_space(6.0);
                theme::error_box(ui, why);
            }
            return action;
        }

        // Fill and Clear, under the slots they work on.
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(open, |ui| {
                let fill = match (&check, state.inventory(), state.crafting_input()) {
                    (Some(c), Some(inv), Some(inp)) if !c.satisfied() => {
                        craft::fill(c.recipe, inv, inp)
                    }
                    _ => Vec::new(),
                };
                let clear = match (state.crafting_input(), state.inventory()) {
                    (Some(inp), Some(inv)) => craft::empty_into(inp, inv),
                    _ => Vec::new(),
                };
                ui.add_space(((ui.available_width() - 150.0) / 2.0).max(0.0));
                let tip = "Bring what the recipe needs from your inventory, best first";
                if ui
                    .add_enabled(!fill.is_empty(), egui::Button::new(theme::text("Fill")))
                    .on_hover_text(tip)
                    .clicked()
                {
                    action = Some(Action::Moves(fill));
                }
                if ui
                    .add_enabled(!clear.is_empty(), egui::Button::new(theme::text("Clear")))
                    .on_hover_text("Put the inputs back in your inventory")
                    .clicked()
                {
                    action = Some(Action::Moves(clear));
                }
            });
        });

        if let Some(check) = &check {
            if let Some(blocked) = &check.blocked {
                ui.add_space(6.0);
                if let Some(m) = blocked_box(ui, state, check, blocked) {
                    action = Some(m);
                }
            } else if let Some(q) = check.estimate {
                ui.add_space(6.0);
                preview(ui, check, q);
            }
        }
        ui.add_space(6.0);

        let (label, enabled) = craft_label(check.as_ref(), view.phase, state);
        let button = theme::accent_button(ui, &label, CRAFT, enabled);
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

    /// The recipe list: "Auto: …" first, then every hand recipe.
    fn picker(&mut self, ui: &mut egui::Ui, state: &GameState) {
        let detected = state
            .crafting_input()
            .and_then(|c| craft::detect(craft::hand_recipes(state.recipes()), c));
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
            egui::ComboBox::from_id_salt("capitol_recipe")
                .width(ui.available_width())
                .selected_text(theme::text(shown))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.picked, None, auto);
                    for r in craft::hand_recipes(state.recipes()) {
                        ui.selectable_value(&mut self.picked, Some(r.id.clone()), &r.name);
                    }
                });
        });
    }

    /// A craft under way: what, the bar, how long, and Cancel.
    fn under_way(
        &self,
        ui: &mut egui::Ui,
        recipe_name: &str,
        duration: u32,
        action: &mut Option<Action>,
    ) {
        ui.ctx().request_repaint();
        let f = self.bar.shown;
        ui.vertical_centered(|ui| {
            ui.label(theme::text(format!("Crafting {recipe_name}…")));
        });
        ui.add_space(4.0);
        theme::progress(ui, f, CRAFT);
        ui.add_space(2.0);
        let left = (1.0 - f) * duration as f32 * TICK_MS as f32 / 1000.0;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{:.0}%", f * 100.0))
                    .color(TEXT)
                    .monospace()
                    .size(size::LABEL),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{left:.1}s left"))
                        .color(DIM)
                        .monospace()
                        .size(size::LABEL),
                );
            });
        });
        ui.add_space(4.0);
        ui.vertical_centered_justified(|ui| {
            if ui.button(theme::text("Cancel")).clicked() {
                *action = Some(Action::Cancel);
            }
        });
        ui.vertical_centered(|ui| {
            ui.label(theme::dim("Walking away cancels it too").size(size::LABEL));
        });
    }
}

/// What the recipe needs, against what the inputs hold.
fn needs(ui: &mut egui::Ui, check: &Check) {
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::list_spacing(ui);
        ui.label(theme::label("Required"));
        for (i, need) in check.needs.iter().enumerate() {
            if i > 0 {
                theme::divider(ui);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(&need.item_name).color(TEXT).size(size::SMALL));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let colour = if need.met() { ENOUGH } else { SHORT };
                    ui.label(RichText::new(if need.met() { "✓" } else { "✗" }).color(colour));
                    ui.label(
                        RichText::new(format!("/{}", need.required))
                            .color(DIM)
                            .monospace()
                            .size(size::LABEL),
                    );
                    ui.label(
                        RichText::new(need.have.to_string())
                            .color(colour)
                            .monospace()
                            .size(size::LABEL),
                    );
                });
            });
        }
    });
}

/// The estimate: what comes out and about how good, said as an estimate.
fn preview(ui: &mut egui::Ui, check: &Check, quality: i32) {
    let output = check
        .recipe
        .outputs
        .first()
        .map_or("Output", |o| o.item_name.as_str());
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - 190.0) / 2.0).max(0.0));
            ui.label(theme::dim("Preview:"));
            ui.label(theme::text(output));
            theme::grade_badge(ui, quality).on_hover_text(
                "An estimate: the mean of the inputs it uses, give or take a few points. \
                 How the server weighs quality will change.",
            );
            ui.label(theme::dim("est.").size(size::LABEL));
        });
    });
}

/// The output slot is taken by something else: say so, and offer to move
/// it to the inventory.
fn blocked_box(
    ui: &mut egui::Ui,
    state: &GameState,
    check: &Check,
    blocked: &craft::Blocked,
) -> Option<Action> {
    let output = check
        .recipe
        .outputs
        .first()
        .map_or("the output", |o| o.item_name.as_str());
    theme::error_box(
        ui,
        &format!(
            "{} fills the output, so the {output} would be destroyed.",
            blocked.item_name
        ),
    );
    let inventory = state.inventory()?;
    let slot = inventory.first_empty()?;
    ui.add_space(2.0);
    let mut action = None;
    ui.vertical_centered(|ui| {
        if theme::link(ui, "Move it to the inventory", size::SMALL).clicked() {
            action = Some(Action::Moves(vec![(
                blocked.item_id,
                inventory.id,
                slot as i32,
            )]));
        }
    });
    action
}

/// The Craft button's words, and whether it can be pressed.
fn craft_label(check: Option<&Check>, phase: &Phase, state: &GameState) -> (String, bool) {
    match phase {
        Phase::Asking { .. } => return ("Starting…".into(), false),
        Phase::Settling { .. } => return ("Finishing…".into(), false),
        Phase::Idle | Phase::Running => {}
    }
    let Some(check) = check else {
        return ("Select a recipe".into(), false);
    };
    let name = &check.recipe.name;
    if state.is_extracting() {
        ("Finish gathering first".into(), false)
    } else if !check.satisfied() {
        (format!("Need materials for {name}"), false)
    } else if check.blocked.is_some() {
        ("Clear the output first".into(), false)
    } else {
        let time = seconds(check.recipe.duration_ticks);
        (format!("Craft {name} · {time}"), true)
    }
}

/// A duration in ticks, for people: "5s", "2.5s".
fn seconds(ticks: i32) -> String {
    let s = ticks.max(0) as f32 * TICK_MS as f32 / 1000.0;
    if s.fract() == 0.0 {
        format!("{s:.0}s")
    } else {
        format!("{s:.1}s")
    }
}

/// A craft's progress eased between the server's updates, which come every
/// five ticks. It runs on at the tick rate, but never more than one update
/// ahead of the server, nor behind it.
#[derive(Debug, Default, Clone, Copy)]
pub struct Fill {
    /// 0 to 1, as drawn.
    pub shown: f32,
}

impl Fill {
    /// Ticks between the server's `crafting_progress` messages.
    const UPDATE_TICKS: f32 = 5.0;

    pub fn update(&mut self, activity: Option<&Activity>, dt: f32) {
        let Some(Activity::Crafting {
            progress, duration, ..
        }) = activity
        else {
            self.shown = 0.0;
            return;
        };
        let duration = (*duration).max(1) as f32;
        let at = *progress as f32 / duration;
        let per_second = 1000.0 / (TICK_MS as f32 * duration);
        let ahead = Self::UPDATE_TICKS / duration;
        self.shown = (self.shown + dt * per_second)
            .clamp(at, (at + ahead).min(1.0))
            .min(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn crafting(progress: u32) -> Activity {
        Activity::Crafting {
            operation_id: Uuid::nil(),
            recipe_id: "rope_basic".into(),
            recipe_name: "Basic Rope".into(),
            progress,
            duration: 50,
        }
    }

    #[test]
    fn the_bar_runs_smoothly_between_updates_and_never_strays() {
        let mut bar = Fill::default();
        bar.update(Some(&crafting(0)), 0.0);
        assert_eq!(bar.shown, 0.0);
        // A quarter second: 2.5 ticks of 50.
        bar.update(Some(&crafting(0)), 0.25);
        assert!((bar.shown - 0.05).abs() < 1e-4, "{}", bar.shown);
        // A stalled server: no more than one update ahead.
        bar.update(Some(&crafting(0)), 5.0);
        assert!((bar.shown - 0.1).abs() < 1e-4, "{}", bar.shown);
        // An update ahead of the bar pulls it up.
        bar.update(Some(&crafting(30)), 0.0);
        assert!((bar.shown - 0.6).abs() < 1e-4, "{}", bar.shown);
        bar.update(Some(&crafting(50)), 1.0);
        assert_eq!(bar.shown, 1.0);
        bar.update(None, 0.1);
        assert_eq!(bar.shown, 0.0);
    }

    #[test]
    fn durations_read_as_seconds() {
        assert_eq!(seconds(50), "5s");
        assert_eq!(seconds(25), "2.5s");
        assert_eq!(seconds(-1), "0s");
    }
}
