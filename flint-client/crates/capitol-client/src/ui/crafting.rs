//! The crafting panel (P5.5), bottom left as in React (`CraftingPanel.tsx`),
//! toggled with C (the `crafting` action) or the header's Crafting chip;
//! Enter (`craft`) crafts while it's open. The rules are
//! `capitol_core::craft`; this draws them.
//!
//! - **A recipe** picked from the list, or the one the inputs make. The
//!   inputs narrow the list: what they make first, marked ✓, then what
//!   takes something in them, then the rest (#490).
//! - **What it needs**, against what the input slots hold, with **Fill**
//!   to bring it from the inventory (best stacks first) and **Clear** to
//!   put the inputs back. React has neither; with them you can craft
//!   without dragging at all.
//! - **The slots**, `crafting_input` (4) → `crafting_output` (1), shut to
//!   drags while a craft is out, so nothing it uses can be moved away
//!   (B10, #61, may make the server refuse that itself).
//! - **The quality range**, exact: the server's own cascade over the units
//!   the craft takes (`rules::quality`), so only the roll is left. Under it
//!   a bar shows where the points come from (E1.7): each input, the
//!   station and the tool end to end on a 0–100 scale, the roll a pale band
//!   at the end, the grade lines ticked below. Hover either for the sums.
//!   Then the tool it would use and its wear, and what an A-grade station
//!   or tool would lift the floor to, where one would.
//! - **Output with nowhere to go** stops the craft, says what's in the way
//!   and offers to move it, because the server would destroy the output.
//! - **Under way**: the bar, the time left and Cancel. Walking cancels too
//!   (the server stops a craft on `move`), and the panel says so.

use super::icons::ItemIcons;
use super::slots::{Move, SlotGrid};
use super::theme::{self, size, CRAFT, DIM, ENOUGH, SHORT, SLOT_GAP, TEXT};
use capitol_core::craft::{self, Check, Fit, Phase};
use capitol_core::protocol::rest::RecipeInfo;
use capitol_core::protocol::rules::{quality, TICK_MS};
use capitol_core::state::{Activity, Container, GameState};
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
            craft::tool_in(state, recipe, false),
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
            if under_way(ui, self.bar.shown, recipe_name, *duration, CRAFT) {
                action = Some(Action::Cancel);
            }
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
                        craft::fill(c.recipe, inv, inp, craft::Whose::Any)
                    }
                    _ => Vec::new(),
                };
                let clear = match (state.crafting_input(), state.inventory()) {
                    (Some(inp), Some(inv)) => craft::empty_into(inp, inv, craft::Whose::Any),
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
                if let Some(shifts) = blocked_box(ui, state, check, blocked, false) {
                    action = Some(Action::Moves(shifts));
                }
            } else if let Some(range) = check.range {
                ui.add_space(6.0);
                preview(ui, check, range);
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

    /// The recipe list, narrowed by the inputs.
    fn picker(&mut self, ui: &mut egui::Ui, state: &GameState) {
        if let Some(input) = state.crafting_input() {
            let recipes = craft::hand_recipes(state.recipes());
            picker(ui, "capitol_recipe", &mut self.picked, recipes, input);
        }
    }
}

/// The recipe picker, here and at a station (#490). The automatic choice
/// first, in words, then the recipes as [`craft::shortlist`] orders them:
/// those the inputs make marked ✓, those that take something in them, and
/// under a rule the rest, dimmed.
pub(super) fn picker<'a>(
    ui: &mut egui::Ui,
    salt: &str,
    picked: &mut Option<String>,
    recipes: impl IntoIterator<Item = &'a RecipeInfo>,
    input: &Container,
) {
    let list = craft::shortlist(recipes, input);
    let auto = craft::auto_label(input, &list);
    let shown = picked
        .as_ref()
        .and_then(|id| list.iter().find(|l| &l.recipe.id == id))
        .map_or_else(|| auto.clone(), |l| l.recipe.name.clone());
    ui.horizontal(|ui| {
        ui.label(theme::dim("Recipe:"));
        egui::ComboBox::from_id_salt(salt)
            .width(ui.available_width())
            .selected_text(theme::text(shown))
            .show_ui(ui, |ui| {
                ui.selectable_value(picked, None, theme::text(auto)).on_hover_text(
                    "With nothing picked, what's in the inputs chooses: the recipe they \
                     make exactly, else the one that uses the most of them",
                );
                let narrowed = list.iter().any(|l| l.fit != Fit::Other);
                let mut ruled = false;
                for l in &list {
                    let name = match l.fit {
                        Fit::Ready => RichText::new(format!("✓ {}", l.recipe.name)).color(ENOUGH),
                        Fit::Uses => theme::text(&l.recipe.name),
                        Fit::Other if narrowed => {
                            if !ruled {
                                ruled = true;
                                ui.separator();
                            }
                            theme::dim(&l.recipe.name)
                        }
                        Fit::Other => theme::text(&l.recipe.name),
                    };
                    ui.selectable_value(picked, Some(l.recipe.id.clone()), name);
                }
            });
    });
}

/// A craft under way: what, the bar (`f`, 0 to 1) in `accent`, how long,
/// and Cancel.
/// True when Cancel was clicked.
pub(super) fn under_way(
    ui: &mut egui::Ui,
    f: f32,
    recipe_name: &str,
    duration: u32,
    accent: egui::Color32,
) -> bool {
    ui.ctx().request_repaint();
    let mut cancel = false;
    ui.vertical_centered(|ui| {
        ui.label(theme::text(format!("Crafting {recipe_name}…")));
    });
    ui.add_space(4.0);
    theme::progress(ui, f, accent);
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
        cancel = ui.button(theme::text("Cancel")).clicked();
    });
    ui.vertical_centered(|ui| {
        ui.label(theme::dim("Walking away cancels it too").size(size::LABEL));
    });
    cancel
}

/// What the recipe needs, against what the inputs hold.
pub(super) fn needs(ui: &mut egui::Ui, check: &Check) {
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

/// What comes out and the range its quality will land in, the bar of where
/// that comes from (E1.7), the tool it would use, and what a better station
/// or tool would do. The full breakdown is on hover.
pub(super) fn preview(ui: &mut egui::Ui, check: &Check, range: (i32, i32)) {
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
            theme::range_badge(ui, range).on_hover_ui(|ui| breakdown(ui, check));
        });
        ui.add_space(4.0);
        source_bar(ui, check).on_hover_ui(|ui| breakdown(ui, check));
        legend(ui, check);
        if let Some(tool) = &check.tool {
            tool_line(ui, tool);
        }
        for lift in check.lifts() {
            let what = match lift.source {
                quality::Source::Station => "An A-grade station",
                _ => "An A-grade tool in good repair",
            };
            ui.label(
                theme::dim(format!(
                    "{what} would lift the floor to {} (+{})",
                    lift.floor, lift.by
                ))
                .size(size::LABEL),
            );
        }
    });
}

/// A source's colour in the breakdown bar: the inputs in violet, each one
/// after the first a little lighter, then the station and the tool.
fn source_colour(source: &quality::Source, input_index: usize) -> egui::Color32 {
    match source {
        quality::Source::Input(_) => {
            theme::SOURCE_INPUT.lerp_to_gamma(egui::Color32::WHITE, 0.22 * input_index as f32)
        }
        quality::Source::Station => theme::SOURCE_STATION,
        quality::Source::Tool => theme::SOURCE_TOOL,
    }
}

/// The grade boundaries the bar marks: D, C, B and A begin here.
const GRADE_LINES: [i32; 4] = [35, 55, 75, 90];

/// Where the output's quality comes from, on a 0–100 bar (E1.7): each
/// source's points laid end to end in its colour, the roll a pale band over
/// the end, and the grade boundaries ticked underneath.
fn source_bar(ui: &mut egui::Ui, check: &Check) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
    let bar = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 9.0));
    let painter = ui.painter();
    let x = |q: f64| bar.left() + bar.width() * (q.clamp(0.0, 100.0) / 100.0) as f32;
    painter.rect_filled(bar, 3.0, theme::BAR_TRACK);
    let mut at = 0.0;
    let mut inputs = 0;
    for term in check.breakdown() {
        let colour = source_colour(&term.source, inputs);
        if matches!(term.source, quality::Source::Input(_)) {
            inputs += 1;
        }
        let end = at + term.points();
        let part = egui::Rect::from_x_y_ranges(x(at)..=x(end), bar.y_range());
        painter.rect_filled(part, 0.0, colour);
        at = end;
    }
    // The roll: from the low end of the range to the high, over the bar.
    let weights = &check.recipe.quality_weights;
    let base = at.round();
    let (low, high) = (base + weights.variance_min as f64, base + weights.variance_max as f64);
    let roll = egui::Rect::from_x_y_ranges(x(low)..=x(high), bar.y_range().expand(1.5));
    painter.rect_filled(roll, 1.0, theme::SOURCE_ROLL);
    for edge in [roll.left(), roll.right()] {
        painter.vline(edge, roll.y_range(), egui::Stroke::new(1.0_f32, theme::TEXT));
    }
    painter.rect_stroke(bar, 3.0, egui::Stroke::new(1.0_f32, theme::PIP_EDGE));
    for q in GRADE_LINES {
        let tick_x = x(q as f64);
        painter.vline(
            tick_x,
            (bar.bottom() + 1.0)..=(bar.bottom() + 5.0),
            egui::Stroke::new(1.0_f32, theme::GRADE_TICK),
        );
    }
    response
}

/// The bar's key: each source, its colour, and the points it puts in.
fn legend(ui: &mut egui::Ui, check: &Check) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
        let mut inputs = 0;
        for term in check.breakdown() {
            let colour = source_colour(&term.source, inputs);
            if matches!(term.source, quality::Source::Input(_)) {
                inputs += 1;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                ui.painter().rect_filled(dot, 1.5, colour);
                ui.label(
                    theme::dim(format!("{} {:.0}", source_name(check, &term.source), term.points()))
                        .size(size::LABEL),
                );
            });
        }
        let w = &check.recipe.quality_weights;
        ui.label(theme::dim(format!("roll {:+}…{:+}", w.variance_min, w.variance_max)).size(size::LABEL));
    });
}

/// The tool the craft would use: whose, its quality, and what its wear
/// leaves of it for the tool term.
fn tool_line(ui: &mut egui::Ui, tool: &craft::ToolPick) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let whose = if tool.installed { "The station's" } else { "Your" };
        ui.label(theme::dim(format!("{whose} {}", tool.item_name)).size(size::LABEL));
        theme::quality_badge(ui, tool.tool.quality);
        let left = format!("{} of {} uses", tool.tool.durability, tool.tool.max_durability);
        if tool.term() < tool.tool.quality {
            ui.label(
                RichText::new(format!("worn: counts {} · {left}", tool.term()))
                    .color(theme::EMBER)
                    .size(size::LABEL),
            );
        } else {
            ui.label(theme::dim(left).size(size::LABEL));
        }
    });
}

/// A breakdown term's name: the input's item name, "Station" or "Tool".
fn source_name<'a>(check: &'a Check, source: &'a quality::Source) -> &'a str {
    match source {
        quality::Source::Input(item) => check
            .recipe
            .inputs
            .iter()
            .find(|i| &i.item_type == item)
            .map_or(item.as_str(), |i| i.item_name.as_str()),
        quality::Source::Station => "Station",
        quality::Source::Tool => "Tool",
    }
}

/// Where the output's quality comes from: each source's weight and quality,
/// the points it puts in, and the roll on top.
fn breakdown(ui: &mut egui::Ui, check: &Check) {
    let weights = &check.recipe.quality_weights;
    egui::Grid::new("quality_breakdown")
        .num_columns(3)
        .spacing([12.0, 2.0])
        .show(ui, |ui| {
            let mut total = 0.0;
            for term in check.breakdown() {
                ui.label(theme::text(source_name(check, &term.source)));
                ui.label(theme::dim(format!(
                    "{:.0}% of {:.1}",
                    term.weight * 100.0,
                    term.quality
                )));
                ui.label(theme::text(format!("{:.1}", term.points())));
                ui.end_row();
                total += term.points();
            }
            ui.label(theme::text("Roll"));
            ui.label(theme::dim("at random"));
            ui.label(theme::text(format!(
                "{:+} to {:+}",
                weights.variance_min, weights.variance_max
            )));
            ui.end_row();
            ui.label(theme::dim("Before the roll"));
            ui.label("");
            ui.label(theme::text(format!("{total:.1}")));
            ui.end_row();
        });
}

/// The output slot is taken by something else, or `at_station` the
/// station's box is full (#489): say so, and offer to move it to the
/// inventory. Returns that move when it's asked for.
pub(super) fn blocked_box(
    ui: &mut egui::Ui,
    state: &GameState,
    check: &Check,
    blocked: &craft::Blocked,
    at_station: bool,
) -> Option<Vec<craft::Shift>> {
    let output = check
        .recipe
        .outputs
        .first()
        .map_or("the output", |o| o.item_name.as_str());
    let why = if at_station {
        format!("The station's box is full, so the {output} would have nowhere to go.")
    } else {
        format!("{} fills the output, so the {output} would be destroyed.", blocked.item_name)
    };
    theme::error_box(ui, &why);
    let inventory = state.inventory()?;
    let slot = inventory.first_empty()?;
    ui.add_space(2.0);
    let mut action = None;
    ui.vertical_centered(|ui| {
        if theme::link(ui, "Move it to the inventory", size::SMALL).clicked() {
            action = Some(vec![(blocked.item_id, inventory.id, slot as i32)]);
        }
    });
    action
}

/// The Craft button's words, and whether it can be pressed.
pub(super) fn craft_label(
    check: Option<&Check>,
    phase: &Phase,
    state: &GameState,
) -> (String, bool) {
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
    } else if let Some(tool) = check.missing_tool() {
        (format!("Need a {tool} for {name}"), false)
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
