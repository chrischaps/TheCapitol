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
//! - **Where the output goes.** It stays at the station, in its box, as
//!   yours (#489), and you drag it out to your inventory as from any slot.
//!   It once landed in your own crafting output, which surprised people who
//!   had made it at the bench. A line under Fill and Clear says so.
//! - **The tool slot (E1.6, E1.7)**, under a crafting station's box: drag a
//!   tool in to install it (`install_tool`), drag it out to any slot of the
//!   inventory, or press Take out (`remove_tool`; the server puts it in your
//!   inventory). Taking it out is for whoever put it in and the station's
//!   owner. While it's there, a recipe of its type uses it before yours, and
//!   it wears here.
//! - **Automation (E4.4)**, for the owner of a crafting station: a second
//!   tab beside Craft (`ui::automation`), set to make one recipe from the
//!   box, a count of times or forever. The header says when it's running
//!   or paused, for anyone who opens it.
//! - **Hire (Commons S2.7)**, for the owner of a crafting station: a third
//!   tab (`ui::hire`), to offer piece-rate work at one of its recipes, and
//!   your offers there with who's working them, Stop and Take it down.
//! - **On a shift** at someone else's station, the panel opens on their
//!   recipe and keeps to it. The box is theirs, so it can't be dragged
//!   from and Fill and Clear are gone; the line under the recipe says the
//!   work goes into their box and what you're paid for each.
//! - **Its condition (V3.5, #477)**, under the box and its tool, or over
//!   the box once it's under 25, with a badge in the header: a bar of what the
//!   weather has left of it, where it stands (indoors, upstairs, out on a
//!   plot or on open ground) and, below 25 or at 0, what that stops. Worn
//!   at all, Repair, with the bill (`capitol_core::station::repair`, the
//!   server's) against your pack, for anyone who has it open.
//! - **Return to inventory**, for the owner: a recycling bin in gold
//!   (`theme::recycle_bin`, #488). The server puts the whole kit back in
//!   your inventory (#88), and what's in the box with it (#489): your things
//!   to you, a guest's back to the guest. It refuses if your inventory can't
//!   hold it all. So it's how you move a station, and it doesn't ask twice.
//!   It was a dim Remove link, and nobody saw it gives everything back.
//!
//! **On a small screen (#235)** the panel goes where there's room (`place`):
//! two columns left of the inventory if they fit there, else one column,
//! the box above the work. It's never taller than the screen under the
//! header strip: the header, the tabs and the footer stay put, and the
//! columns scroll. 1280×720 at 150% (853×480 points) is the case to check.

use super::automation::{self, Ask};
use super::crafting::{blocked_box, craft_label, needs, picker, preview, under_way, Fill};
use super::hire;
use super::icons::ItemIcons;
use super::slots::{Move, Slot, SlotGrid};
use super::theme::{self, size, EMBER, ENOUGH, SHORT, STATION};
use capitol_core::automation::{Draft, Status};
use capitol_core::chrono::{DateTime, Utc};
use capitol_core::craft::{self, Check, Phase};
use capitol_core::protocol::rules::quality;
use capitol_core::protocol::rules::wages::Work;
use capitol_core::protocol::rest::{RecipeInfo, SlotItem, WageOffer, WageShift};
use capitol_core::wages::{self as wg, Wages};
use capitol_core::state::{Activity, GameState};
use capitol_core::protocol::rules::weather;
use capitol_core::station::{self, repair, repair::Shelter, repair::State as RepairState};
use egui::{Align2, Id};
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
/// The space between the panel and the screen's right and bottom edges.
const EDGE: f32 = 20.0;
/// The nearest the panel comes to the screen's left edge: the crafting
/// panel's inset.
const LEFT: f32 = 16.0;
/// The space under the header strip, as the panels at the top keep.
const UNDER_HEADER: f32 = 12.0;
/// The least the columns get to scroll in, however short the screen.
const LEAST_ROOM: f32 = 120.0;
/// The footer's height until it's drawn: its divider and the Close row.
const FOOTER: f32 = 40.0;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// The recycling bin's button, and the bin on it (#488).
const RECYCLE_BUTTON: egui::Vec2 = egui::vec2(36.0, 32.0);
const RECYCLE_ICON: egui::Vec2 = egui::vec2(19.0, 22.0);

/// How a crafting station's panel lays out its box and its work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// The box on the left, the recipe's column on the right.
    Columns,
    /// One column, the box above the work, where two don't fit (#235).
    Stacked,
}

/// What the panel asks for this frame.
pub enum Action {
    Move(Move),
    /// Several moves at once (Fill, Clear, or clearing the output).
    Moves(Vec<craft::Shift>),
    Craft,
    Cancel,
    Close,
    Remove(Uuid),
    /// Put this tool in the station's slot.
    InstallTool { station_id: Uuid, item_id: Uuid },
    /// Take the station's tool out, into your inventory.
    RemoveTool(Uuid),
    /// Set the station to make this on its own, and start it (E4.4).
    StartAutomation { station_id: Uuid, draft: Draft },
    StopAutomation(Uuid),
    /// Something on the Hire tab, or Stop on your shift (S2.7)
    Wages(hire::Act),
    /// Mend the station to full, for goods from your pack (V3.5)
    Repair(Uuid),
}

/// The right-hand column's two tabs at a station of yours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Craft,
    Automation,
    /// Piece-rate work offered here (S2.7)
    Hire,
}

/// The tool slot's stand-in container id: a drop onto it is an install.
/// Only this panel ever sees it.
pub fn tool_slot_id(station_id: Uuid) -> Uuid {
    Uuid::from_u128(station_id.as_u128() ^ 0x7001_7001)
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct StationPanel {
    /// The station the panel last showed. Opening another resets the pick.
    showing: Option<Uuid>,
    /// The recipe picked by hand. `None` lets the box choose.
    picked: Option<String>,
    bar: Fill,
    /// The tab shown, at a crafting station of yours.
    pub tab: Tab,
    /// The Automation tab's form.
    draft: Draft,
    /// The Hire tab's form, for the station it was written at.
    hire: Option<wg::Draft>,
    /// The footer's height as last drawn, so the columns know their room.
    footer_height: Option<f32>,
    /// The recipe of the shift you're on here, if you are (S2.7).
    shift_recipe: Option<String>,
    /// Where the panel was drawn this frame, if it was, so the hint card
    /// can keep clear of it.
    pub rect: Option<egui::Rect>,
}

/// What the panel draws from.
pub struct View<'a> {
    pub state: &'a GameState,
    pub phase: &'a Phase,
    /// Why the last craft or move was refused, if it was just now.
    pub refused: Option<&'a str>,
    /// Where the inventory panel is, if it's open: the panel sits to its left.
    pub beside: Option<egui::Rect>,
    /// Why the last automation command was refused, if it was just now.
    pub automation_refused: Option<&'a str>,
    /// The offers and your shift (S2.7). `None`: no Hire tab, no shift.
    pub wages: Option<&'a Wages>,
    /// The server's clock, for how long an offer runs.
    pub now: DateTime<Utc>,
    /// Signed in. Offline it's a sample, and nothing is sent.
    pub online: bool,
    /// A repair is out (V3.5).
    pub repairing: bool,
    /// Why the last repair was refused, if it was just now.
    pub repair_refused: Option<&'a str>,
}

impl View<'_> {
    /// The shift you're on at the open station, if you are: the offer and
    /// the shift.
    pub fn shift_here(&self) -> Option<(&WageOffer, &WageShift)> {
        let open = self.state.open_station()?.station_id;
        self.wages?.working().filter(|(o, _)| matches!(&o.work, Work::Craft { station_id, .. } if *station_id == open))
    }
}

impl StationPanel {
    /// The recipe the panel is set to: the one picked, else the one the
    /// station's box makes.
    pub fn recipe<'a>(&self, state: &'a GameState) -> Option<&'a RecipeInfo> {
        let open = state.open_station()?;
        // On a shift, the employer's recipe and no other (S2.7)
        if let Some(id) = &self.shift_recipe {
            return station::recipes_at(state.recipes(), &open.station_type).find(|r| &r.id == id);
        }
        let here = || station::recipes_at(state.recipes(), &open.station_type);
        match &self.picked {
            Some(id) => here().find(|r| &r.id == id),
            None => craft::detect(here(), state.station_container()?),
        }
    }

    /// The recipe weighed against the station's box, which takes its inputs
    /// and its output (#489), at the station's quality.
    pub fn check<'a>(&self, state: &'a GameState) -> Option<Check<'a>> {
        let recipe = self.recipe(state)?;
        let bench = state.station_container()?;
        Some(craft::check_at(
            recipe,
            bench,
            Some(bench),
            state.open_station_info()?.quality,
            craft::tool_in(state, recipe, true),
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
        self.shift_recipe = view.shift_here().and_then(|(o, _)| match &o.work {
            Work::Craft { recipe_id, .. } => Some(recipe_id.clone()),
            Work::Gather { .. } => None,
        });
        let open = view.state.open_station().map(|o| o.station_id);
        if open != self.showing {
            self.showing = open;
            self.picked = None;
            let info = view.state.open_station_info();
            let detected = self.recipe(view.state).map(|r| r.id.clone());
            self.draft = Draft::from(info.and_then(|i| i.automation.as_ref()), detected.as_deref());
            // A station of yours at work opens on that work.
            let working = info.is_some_and(|i| {
                matches!(
                    Status::of(i.automation.as_ref()),
                    Status::Running | Status::Paused(_)
                )
            });
            self.tab = if working && mine(view.state) {
                Tab::Automation
            } else {
                Tab::Craft
            };
            // The panel just opened goes in front of any it lands on.
            if open.is_some() {
                ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, Id::new("capitol_station")));
            }
        }
        self.rect = None;
        let shown =
            ctx.animate_bool_with_time(Id::new("capitol_station_fade"), open.is_some(), FADE);
        if open.is_none() || shown <= 0.0 {
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let storage = view
            .state
            .open_station()
            .is_some_and(|o| station::is_storage(view.state.catalog(), &o.station_type));
        let gaps = ctx.style().spacing.item_spacing.x;
        let frame = theme::panel_edged(STATION);
        let margins = frame.total_margin().sum();
        let inner = |layout| inner_width(layout, storage, gaps);
        // Bottom right, to the left of the inventory when it's open, and
        // never taller than the screen under the header strip.
        let screen = ctx.screen_rect();
        let (layout, right) = place(
            screen,
            view.beside,
            inner(Layout::Columns) + margins.x,
            inner(Layout::Stacked) + margins.x,
        );
        let top = ctx.available_rect().top() + UNDER_HEADER;
        let tallest = screen.bottom() - EDGE - top - margins.y;
        let mut action = None;
        let area = egui::Area::new(Id::new("capitol_station"))
            .anchor(Align2::RIGHT_BOTTOM, [-right, -EDGE])
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                frame.show(ui, |ui| {
                    ui.set_width(inner(layout));
                    action = self.body(ui, icons, view, crafting, layout, tallest);
                });
            });
        self.rect = Some(area.response.rect);
        action
    }

    /// The panel's insides, laid out as `layout`, in at most `tallest`
    /// points: the columns scroll in what the header and footer leave.
    fn body(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        crafting: Option<&Activity>,
        layout: Layout,
        tallest: f32,
    ) -> Option<Action> {
        let state = view.state;
        let open = state.open_station()?;
        let storage = station::is_storage(state.catalog(), &open.station_type);
        let running = crafting.is_some();
        let mut action = None;
        let start = ui.cursor().top();

        let status = Status::of(state.open_station_info().and_then(|i| i.automation.as_ref()));
        let badge = if running {
            let t = ui.input(|i| i.time) as f32;
            ui.ctx().request_repaint();
            Some((
                "In progress",
                STATION.gamma_multiply(0.45 + 0.15 * (t * 4.2).sin()),
            ))
        } else if status == Status::Running {
            let t = ui.input(|i| i.time) as f32;
            ui.ctx().request_repaint();
            Some(("Running", STATION.gamma_multiply(0.4 + 0.12 * (t * 2.4).sin())))
        } else if view.shift_here().is_some() {
            Some(("On shift", theme::GOLD.gamma_multiply(0.35)))
        } else if matches!(status, Status::Paused(_)) {
            Some(("Paused", EMBER.gamma_multiply(0.35)))
        } else if let Some(worn) = state
            .open_station_info()
            .map(|i| RepairState::of(i.condition))
            .filter(|w| *w != RepairState::Sound)
        {
            // V3.5: worn by the weather
            let words = if worn == RepairState::WornOut { "Worn out" } else { "Needs repair" };
            Some((words, EMBER.gamma_multiply(0.35)))
        } else if storage {
            Some(("Storage", theme::PILL))
        } else {
            None
        };
        theme::header(ui, &open.name, Some(STATION), badge);
        ui.add_space(2.0);

        // The box, open to drags only while nothing is out, and at a
        // crafting station, the recipe's column: side by side or stacked.
        // Each scrolls in the room the header and the footer leave.
        // On a shift the box is the employer's: nothing to drag (S2.7)
        let idle = *view.phase == Phase::Idle && !running && view.shift_here().is_none();
        let footer = self.footer_height.unwrap_or(FOOTER);
        let room = |ui: &egui::Ui| (tallest - (ui.cursor().top() - start) - footer).max(LEAST_ROOM);
        let mut put = |a: Option<Action>| {
            if a.is_some() {
                action = a;
            }
        };
        if storage {
            let room = room(ui);
            theme::scrolled(ui, "capitol_station_box", room, |ui| {
                put(station_box(ui, icons, view, true, idle));
            });
        } else if layout == Layout::Columns {
            let room = room(ui);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(BOX_WIDTH);
                    theme::scrolled(ui, "capitol_station_box", room, |ui| {
                        ui.set_width(BOX_WIDTH);
                        put(station_box(ui, icons, view, false, idle));
                    });
                });
                ui.add_space(COLUMN_GAP);
                ui.vertical(|ui| {
                    ui.set_width(WIDTH);
                    let top = ui.cursor().top();
                    let tab = self.tabs(ui, view);
                    let room = (room - (ui.cursor().top() - top)).max(LEAST_ROOM);
                    theme::scrolled(ui, "capitol_station_work", room, |ui| {
                        ui.set_width(WIDTH);
                        put(self.work(ui, icons, view, crafting, tab));
                    });
                });
            });
        } else {
            // The tabs stay put over the box; the box and the work scroll.
            let tab = self.tabs(ui, view);
            let room = room(ui);
            theme::scrolled(ui, "capitol_station_stack", room, |ui| {
                ui.set_width(WIDTH);
                put(station_box(ui, icons, view, false, idle));
                ui.add_space(10.0);
                put(self.work(ui, icons, view, crafting, tab));
            });
        }

        // From the columns' foot, so the spacing after them counts too.
        let before = ui.min_rect().bottom();
        ui.add_space(6.0);
        theme::divider(ui);
        ui.add_space(2.0);
        put(self.footer(ui, view));
        self.footer_height = Some(ui.min_rect().bottom() - before);
        action
    }

    /// The Craft, Automation and Hire tabs, at a crafting station of yours,
    /// and the tab to show: always Craft at someone else's.
    fn tabs(&mut self, ui: &mut egui::Ui, view: &View) -> Tab {
        if !mine(view.state) {
            return Tab::Craft;
        }
        if view.wages.is_none() && self.tab == Tab::Hire {
            self.tab = Tab::Craft;
        }
        ui.horizontal(|ui| {
            if view.wages.is_some() {
                theme::segmented(
                    ui,
                    &mut self.tab,
                    &[(Tab::Craft, "Craft"), (Tab::Automation, "Automation"), (Tab::Hire, "Hire")],
                );
            } else {
                theme::segmented(ui, &mut self.tab, &[(Tab::Craft, "Craft"), (Tab::Automation, "Automation")]);
            }
        });
        ui.add_space(6.0);
        self.tab
    }

    /// A crafting station's work: the recipe's column, or its automation.
    fn work(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
        crafting: Option<&Activity>,
        tab: Tab,
    ) -> Option<Action> {
        match tab {
            Tab::Craft => self.crafting(ui, view, crafting),
            Tab::Automation => self.automation(ui, icons, view),
            Tab::Hire => self.hire(ui, view),
        }
    }

    /// A crafting station's half: the recipe, its needs, Fill and Clear,
    /// where the output goes, the estimate and Craft. Or the craft under way.
    fn crafting(
        &mut self,
        ui: &mut egui::Ui,
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
        let shift = view.shift_here();
        if shift.is_none() {
            self.picker(ui, state, &open.station_type);
        } else if let Some(recipe) = self.recipe(state) {
            ui.horizontal(|ui| {
                ui.label(theme::dim("Working on:"));
                ui.label(theme::text(&recipe.name));
            });
        }
        ui.add_space(6.0);

        let check = self.check(state);
        if let Some(info) = state.open_station_info() {
            station_quality(ui, info.quality, check.as_ref());
            ui.add_space(4.0);
        }
        if let Some(check) = &check {
            needs(ui, check);
            ui.add_space(4.0);
        }
        let idle = *view.phase == Phase::Idle;
        if let Some((offer, shift)) = shift {
            // Their box, their inputs: what's made goes to them (S2.7)
            ui.add_space(2.0);
            theme::well().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    theme::text(format!("What you make goes into {}'s box", offer.employer_name)).size(size::SMALL),
                );
                ui.label(
                    theme::dim(format!(
                        "You're paid {} Strands each as it's made: {} so far this shift, {} of {} in all",
                        offer.rate,
                        offer.rate.saturating_mul(shift.made as i64),
                        offer.made,
                        offer.cap
                    ))
                    .size(size::LABEL),
                );
                if theme::link(ui, "Stop the shift", size::LABEL)
                    .on_hover_text("End your shift. What you made is paid already")
                    .clicked()
                    && view.online
                {
                    action = Some(Action::Wages(hire::Act::Ask(wg::Ask::Stop(offer.id))));
                }
            });
        } else {
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

        // Where the output lands: here, in the box (#489)
        ui.add_space(4.0);
        ui.vertical_centered(|ui| {
            ui.label(theme::dim("What's made here waits in the station's box").size(size::LABEL));
        });
        }

        if let Some(check) = &check {
            if let Some(blocked) = &check.blocked {
                ui.add_space(6.0);
                if let Some(shifts) = blocked_box(ui, state, check, blocked, true) {
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

    /// The Automation tab, at a station of yours.
    fn automation(
        &mut self,
        ui: &mut egui::Ui,
        icons: &mut ItemIcons,
        view: &View,
    ) -> Option<Action> {
        let info = view.state.open_station_info()?;
        let station_id = info.id;
        match automation::show(
            ui,
            icons,
            view.state,
            info,
            &mut self.draft,
            view.automation_refused,
        )? {
            Ask::Start => Some(Action::StartAutomation {
                station_id,
                draft: self.draft.clone(),
            }),
            Ask::Stop => Some(Action::StopAutomation(station_id)),
        }
    }

    /// The Hire tab, at a station of yours (S2.7): your offers here, and
    /// the form for one at any of its recipes.
    fn hire(&mut self, ui: &mut egui::Ui, view: &View) -> Option<Action> {
        let wages = view.wages?;
        let open = view.state.open_station()?;
        let station_id = open.station_id;
        let choices: Vec<(String, String)> = station::recipes_at(view.state.recipes(), &open.station_type)
            .map(|r| (r.id.clone(), r.name.clone()))
            .collect();
        // The recipe the box is set for, as the Craft tab picks it
        let set = self.recipe(view.state).map(|r| r.id.clone());
        let fresh = || {
            let work = Work::Craft {
                station_id,
                recipe_id: set.or_else(|| choices.first().map(|c| c.0.clone())).unwrap_or_default(),
            };
            let rate = wg::rate_for(&work, view.state.catalog());
            wg::Draft::new(work, rate)
        };
        let draft = match &mut self.hire {
            Some(d) if wg::place_of(&d.work) == station_id => d,
            slot => slot.insert(fresh()),
        };
        let hire_view = hire::View {
            wages,
            me: view.state.me(),
            strands: view.state.strands(),
            catalog: view.state.catalog(),
            now: view.now,
            online: view.online,
            accent: STATION,
        };
        hire::show(ui, station_id, draft, &choices, &hire_view).map(Action::Wages)
    }

    /// The station's recipes, narrowed by what's in its box (#490).
    fn picker(&mut self, ui: &mut egui::Ui, state: &GameState, station_type: &str) {
        if let Some(input) = state.station_container() {
            let recipes = station::recipes_at(state.recipes(), station_type);
            picker(ui, "capitol_station_recipe", &mut self.picked, recipes, input);
        }
    }

    /// Return to inventory (the owner's) and Close. It's a recycling bin in
    /// the gold of "yours", which says what it does on hover: a dim Remove
    /// link read as losing the bench, so benches were left standing by the
    /// road rather than taken home (#488). It packs up the box too (#489).
    fn footer(&mut self, ui: &mut egui::Ui, view: &View) -> Option<Action> {
        let state = view.state;
        let open = state.open_station()?;
        let mine = state
            .station(open.station_id)
            .is_some_and(|s| Some(s.owner_id) == state.me());
        let empty = state
            .station_container()
            .is_none_or(|c| c.used_slots() == 0);
        let busy = state.local_activity().is_some();
        let mut action = None;
        ui.horizontal(|ui| {
            if mine {
                let enabled = !busy;
                let button = ui
                    .add_enabled(enabled, egui::Button::new("").min_size(RECYCLE_BUTTON))
                    .on_hover_text(return_hover(empty))
                    .on_disabled_hover_text(RETURN_BUSY);
                let ink = if enabled { theme::GOLD } else { theme::GOLD.gamma_multiply(0.4) };
                let bin = egui::Rect::from_center_size(button.rect.center(), RECYCLE_ICON);
                theme::recycle_bin(ui.painter(), bin, ink);
                if button.clicked() {
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

/// The recycling bin's hover (#488), and with things in the box, where they
/// go (#489): yours with you, a guest's back to them
fn return_hover(empty: bool) -> &'static str {
    if empty {
        "Return to inventory"
    } else {
        "Return to inventory, with what's in its box"
    }
}

/// Its hover while a craft is under way.
const RETURN_BUSY: &str = "Return to inventory, once the craft is done";

/// The panel's width inside its frame: one column for a chest or when
/// stacked, else the box and the recipe's column with the gap between them
/// (`gaps` is egui's item spacing, which comes either side of the gap).
fn inner_width(layout: Layout, storage: bool, gaps: f32) -> f32 {
    match (storage, layout) {
        (true, _) => BOX_WIDTH,
        (false, Layout::Columns) => BOX_WIDTH + gaps + COLUMN_GAP + gaps + WIDTH,
        (false, Layout::Stacked) => WIDTH,
    }
}

/// Where the panel goes on `screen`, given its width in `columns` and
/// stacked: the layout, and the panel's inset from the right edge. Left of
/// the inventory when it's open (`beside`), so a drag across is short: in
/// columns if they fit there, else stacked. If even that doesn't fit, it's
/// stacked as far left as it may go, over the inventory's edge.
fn place(screen: egui::Rect, beside: Option<egui::Rect>, columns: f32, stacked: f32) -> (Layout, f32) {
    let right = beside.map_or(EDGE, |inventory| screen.right() - inventory.left() + GAP);
    let fits = |width: f32| screen.right() - right - width >= screen.left() + LEFT;
    if fits(columns) {
        (Layout::Columns, right)
    } else if fits(stacked) {
        (Layout::Stacked, right)
    } else {
        (Layout::Stacked, (screen.width() - stacked - LEFT).max(EDGE))
    }
}

/// The station's box, as a slot grid, and under it a chest's note or a
/// crafting station's tool slot.
fn station_box(
    ui: &mut egui::Ui,
    icons: &mut ItemIcons,
    view: &View,
    storage: bool,
    idle: bool,
) -> Option<Action> {
    let state = view.state;
    let mut action = None;
    // Worn past working on its own, its condition and Repair come first
    let worn = state.open_station_info().is_some_and(|i| RepairState::of(i.condition) != RepairState::Sound);
    if worn {
        action = condition(ui, view);
        ui.add_space(4.0);
    }
    ui.label(theme::label(if storage { "Storage" } else { "Station box" }));
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
        ui.label(theme::dim("Drag items in to store them, and out to take them").size(size::LABEL));
        if let Some(why) = view.refused {
            ui.add_space(6.0);
            theme::error_box(ui, why);
        }
    } else if let Some(a) = tool_slot(ui, icons, state, idle) {
        action = Some(a);
    }
    if !worn {
        ui.add_space(8.0);
        if let Some(a) = condition(ui, view) {
            action = Some(a);
        }
    }
    action
}

/// The station's condition under the header (V3.5): a bar of it, where it
/// stands, what wear stops, and Repair with its bill while it's worn.
fn condition(ui: &mut egui::Ui, view: &View) -> Option<Action> {
    let state = view.state;
    let info = state.open_station_info()?;
    let me = state.me();
    // Only your own plots are known: a station of yours off them is on
    // open ground, and anyone else's is just outdoors
    let on_a_plot = (me.is_some() && Some(info.owner_id) == me)
        .then(|| state.plots().iter().any(|p| p.contains(info.x, info.y)));
    let shelter = Shelter::of(info, on_a_plot);
    let worn = RepairState::of(info.condition);
    let mut action = None;
    ui.label(theme::label("Condition"));
    ui.horizontal(|ui| {
        condition_bar(ui, info.condition);
        ui.label(theme::text(format!("{}%", info.condition.clamp(0, 100))).size(size::SMALL));
    });
    ui.label(theme::dim(shelter.words()).size(size::LABEL));
    if let Some(words) = worn.words() {
        ui.label(egui::RichText::new(words).color(if worn == RepairState::WornOut { SHORT } else { EMBER }).size(size::LABEL));
    }
    if info.condition >= weather::FULL as i32 {
        ui.add_space(4.0);
        return None;
    }
    ui.add_space(4.0);
    let needs = repair::needs(state.catalog(), state.recipes(), info, state.inventory());
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::dim("Repair takes").size(size::LABEL));
            if needs.is_empty() {
                ui.label(theme::text("nothing").size(size::LABEL));
            }
            for (i, need) in needs.iter().enumerate() {
                let ink = if need.short() == 0 { ENOUGH } else { SHORT };
                let sep = if i + 1 < needs.len() { "," } else { "" };
                ui.label(egui::RichText::new(format!("{} {}", need.need, need.name)).color(ink).size(size::LABEL))
                    .on_hover_text(format!("You hold {}", need.held));
                ui.label(theme::dim(format!("(you hold {}){sep}", need.held)).size(size::LABEL));
            }
        });
        let short = needs.iter().any(|n| n.short() > 0);
        let (label, enabled) = match () {
            _ if view.repairing => ("Repairing…".to_string(), false),
            _ if short => ("Not enough to repair it".to_string(), false),
            _ => ("Repair".to_string(), true),
        };
        ui.add_space(2.0);
        if theme::accent_button(ui, &label, STATION, enabled && view.online)
            .on_hover_text("Back to 100%, for a share of what its kit took, the lowest grade first")
            .clicked()
            && enabled
        {
            action = Some(Action::Repair(info.id));
        }
        if let Some(why) = view.repair_refused {
            ui.add_space(4.0);
            theme::error_box(ui, why);
        }
    });
    ui.add_space(4.0);
    action
}

/// A short bar of `condition`, green going to gold and then to red as it
/// falls, with a tick at the 25 under which it stops working on its own.
fn condition_bar(ui: &mut egui::Ui, condition: i32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(CONDITION_BAR, 6.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, theme::BAR_TRACK);
    let k = (condition.clamp(0, 100) as f32) / 100.0;
    let ink = if k >= 0.5 {
        theme::GOLD.lerp_to_gamma(ENOUGH, (k - 0.5) * 2.0)
    } else {
        SHORT.lerp_to_gamma(theme::GOLD, k * 2.0)
    };
    let mut filled = rect;
    filled.set_width(rect.width() * k);
    painter.rect_filled(filled, 3.0, ink);
    let x = rect.left() + rect.width() * weather::NEEDS_REPAIR as f32 / 100.0;
    painter.line_segment([egui::pos2(x, rect.top() - 1.0), egui::pos2(x, rect.bottom() + 1.0)], (1.0, theme::GRADE_TICK));
}

/// The condition bar's width, in points.
const CONDITION_BAR: f32 = 96.0;

/// The open station is yours.
fn mine(state: &GameState) -> bool {
    state
        .open_station_info()
        .is_some_and(|s| state.me().is_some() && Some(s.owner_id) == state.me())
}

/// A crafting station's tool slot: the tool in it, as a slot you can drag
/// out of or into, with its uses, and Take out for whoever may.
fn tool_slot(
    ui: &mut egui::Ui,
    icons: &mut ItemIcons,
    state: &GameState,
    idle: bool,
) -> Option<Action> {
    let info = state.open_station_info()?;
    let station_id = info.id;
    let installed = info.tool.as_ref();
    let item = installed.map(|t| {
        SlotItem::new(
            t.item_id,
            t.item_type.clone(),
            state.catalog().item_name(&t.item_type),
            t.quality,
            1,
            0,
        )
        .with_durability(t.durability, t.max_durability)
    });
    let me = state.me();
    let may_take = installed
        .is_some_and(|t| me.is_some() && (t.owner_id == me || Some(info.owner_id) == me));
    let mut action = None;
    ui.add_space(8.0);
    ui.label(theme::label("Tool slot"));
    ui.horizontal(|ui| {
        theme::tray().show(ui, |ui| {
            let slot = Slot {
                container_id: tool_slot_id(station_id),
                index: 0,
                item: item.as_ref(),
                // A tool you may not take out can't be dragged out either
                enabled: idle && (installed.is_none() || may_take),
            };
            if let Some(m) = slot.show(ui, icons) {
                action = Some(Action::InstallTool {
                    station_id,
                    item_id: m.item_id,
                });
            }
        });
        ui.vertical(|ui| {
            match installed {
                Some(t) => {
                    ui.label(theme::text(state.catalog().item_name(&t.item_type)).size(size::SMALL));
                    ui.label(
                        theme::dim(format!("{} of {} uses", t.durability, t.max_durability))
                            .size(size::LABEL),
                    );
                    if may_take && idle && theme::link(ui, "Take out", size::LABEL).clicked() {
                        action = Some(Action::RemoveTool(station_id));
                    }
                }
                None => {
                    ui.label(theme::dim("Drag a tool here").size(size::SMALL));
                    ui.label(
                        theme::dim("Recipes of its type use it first, and it wears here")
                            .size(size::LABEL),
                    );
                }
            }
        });
    });
    action
}

/// The station's grade, and what it puts into the recipe picked: its share
/// of the output and the points that share is worth.
fn station_quality(ui: &mut egui::Ui, station: i32, check: Option<&Check>) {
    ui.horizontal(|ui| {
        ui.label(theme::dim("Station quality:"));
        theme::quality_badge(ui, station)
            .on_hover_text("The quality of the kit that placed it. A better station makes better work");
        let share = check.and_then(|c| {
            c.breakdown()
                .into_iter()
                .find(|t| t.source == quality::Source::Station)
        });
        if let Some(term) = share.filter(|t| t.weight > 0.0) {
            ui.label(
                theme::dim(format!(
                    "{:.0}% of this recipe, +{:.1}",
                    term.weight * 100.0,
                    term.points()
                ))
                .size(size::LABEL),
            );
        }
    });
}

/// `--station-panel-sample` (#235), and the tests': you at a workbench of
/// yours, its box holding a tool handle's sticks and rope so the Craft tab
/// draws all it has, with your inventory and crafting boxes stocked.
pub fn sample() -> GameState {
    use capitol_core::net::{ConnectionState, RestResponse};
    use capitol_core::protocol::rest::{
        ContainerResponse, ContainersResponse, RecipesResponse, StationContainerResponse,
    };
    use capitol_core::protocol::ws::ServerMessage;
    const ME: Uuid = Uuid::from_u128(0xca9);
    const BENCH: Uuid = Uuid::from_u128(0x235);
    const BOX: Uuid = Uuid::from_u128(0x2350);
    let mut state = GameState::new();
    state.apply_connection(&ConnectionState::Connected { player_id: ME });
    let recipes = state.catalog().recipes().to_vec();
    state.apply_rest(&RestResponse::Recipes(RecipesResponse { recipes }));
    state.apply_ws(&ServerMessage::StationPlaced {
        station_id: BENCH,
        station_type: "workbench".into(),
        name: "Workbench".into(),
        owner_id: ME,
        x: 0.0,
        y: 0.0,
        container_id: BOX,
        quality: 60,
        floor: 0,
        indoors: false,
    });
    state.apply_ws(&ServerMessage::StationOpened {
        player_id: ME,
        station_id: BENCH,
        station_type: "workbench".into(),
        name: "Workbench".into(),
        container_id: BOX,
    });
    let catalog = capitol_core::catalog::Catalog::fallback();
    let item = |n: u128, slot, kind: &str, quality, quantity| {
        let name = catalog.item_name(kind);
        SlotItem::new(Uuid::from_u128(n), kind.into(), name, quality, quantity, slot)
    };
    let container = |n: u128, kind: &str, slot_count, layout_columns, slots| ContainerResponse {
        id: Uuid::from_u128(n),
        container_type: kind.into(),
        slot_count,
        layout_columns,
        slots,
    };
    let containers = vec![
        container(
            0x2351,
            "player_inventory",
            24,
            6,
            vec![
                item(1, 0, "fiber", 52, 18),
                item(2, 1, "sticks", 61, 6),
                item(3, 2, "rope", 58, 3),
                item(4, 3, "stone", 47, 9),
                item(5, 5, "log", 66, 4),
            ],
        ),
        container(0x2352, "crafting_input", 4, 2, vec![]),
        container(0x2353, "crafting_output", 1, 1, vec![]),
    ];
    state.apply_rest(&RestResponse::Containers(ContainersResponse { containers }));
    let slots = vec![item(10, 0, "sticks", 64, 4), item(11, 1, "rope", 70, 2)];
    state.apply_rest(&RestResponse::StationContainer(StationContainerResponse {
        id: BOX,
        container_type: "station_inventory".into(),
        slot_count: 16,
        layout_columns: 4,
        slots,
    }));
    state
}

/// Empties [`sample`]'s station box, so its owner may return it to their
/// inventory (#488).
pub fn empty_the_box(state: &mut GameState) {
    use capitol_core::net::RestResponse;
    use capitol_core::protocol::rest::StationContainerResponse;
    state.apply_rest(&RestResponse::StationContainer(StationContainerResponse {
        id: Uuid::from_u128(0x2350),
        container_type: "station_inventory".into(),
        slot_count: 16,
        layout_columns: 4,
        slots: Vec::new(),
    }));
}

/// `--station-panel-sample repair` and `worn` (V3.5): [`sample`]'s
/// workbench out on open ground, worn by the weather to 18 (`repair`), with
/// the fiber its repair takes in your pack, or to 0 with too little (`worn`).
pub fn worn_out_there(state: &mut GameState, which: &str) {
    use capitol_core::net::RestResponse;
    use capitol_core::protocol::rest::{ContainerResponse, ContainersResponse};
    use capitol_core::protocol::ws::ServerMessage;
    let condition = if which == "worn" { 0 } else { 18 };
    state.apply_ws(&ServerMessage::StationCondition { station_id: Uuid::from_u128(0x235), condition });
    if which == "worn" {
        let catalog = capitol_core::catalog::Catalog::fallback();
        let item = |n: u128, slot, kind: &str, quality, quantity| {
            SlotItem::new(Uuid::from_u128(n), kind.into(), catalog.item_name(kind), quality, quantity, slot)
        };
        state.apply_rest(&RestResponse::Containers(ContainersResponse {
            containers: vec![ContainerResponse {
                id: Uuid::from_u128(0x2351),
                container_type: "player_inventory".into(),
                slot_count: 24,
                layout_columns: 6,
                slots: vec![item(1, 0, "fiber", 52, 2), item(4, 3, "stone", 47, 9)],
            }],
        }));
    }
}

/// [`sample`] at an empty storage chest of yours instead, whose panel is
/// one box wide, for its footer's recycling bin (#488).
pub fn chest_of_yours(state: &mut GameState) {
    use capitol_core::protocol::ws::ServerMessage;
    const ME: Uuid = Uuid::from_u128(0xca9);
    const CHEST: Uuid = Uuid::from_u128(0xc4e5);
    const BOX: Uuid = Uuid::from_u128(0x2350);
    state.apply_ws(&ServerMessage::StationPlaced {
        station_id: CHEST,
        station_type: "storage_chest".into(),
        name: "Storage Chest".into(),
        owner_id: ME,
        x: 20.0,
        y: 0.0,
        container_id: BOX,
        quality: 50,
        floor: 0,
        indoors: false,
    });
    state.apply_ws(&ServerMessage::StationOpened {
        player_id: ME,
        station_id: CHEST,
        station_type: "storage_chest".into(),
        name: "Storage Chest".into(),
        container_id: BOX,
    });
    // Opening it asks for its box afresh
    empty_the_box(state);
}

/// `--station-panel-sample hire` and `shift` (S2.7): the offers on the
/// Work page. `hire`: one of yours at [`sample`]'s workbench, with Brin on
/// it. `shift`: one of Ada's at her workbench ([`at_adas_bench`]), with you
/// on it.
pub fn sample_wages(which: &str) -> Wages {
    use capitol_core::protocol::rest::WagesResponse;
    use capitol_core::protocol::rules::{court, wages::OfferStatus};
    let now = Utc::now();
    let hours = capitol_core::chrono::Duration::hours;
    let shift = |worker: u128, name: &str, made: i32| WageShift {
        id: Uuid::from_u128(0x5393),
        offer_id: Uuid::from_u128(0x393),
        worker_id: Uuid::from_u128(worker),
        worker_name: name.into(),
        worker_citizen: false,
        started_at: now - hours(1),
        ended_at: None,
        made,
        end: None,
    };
    let (employer, name, station, worker) = match which {
        "shift" => (0xada_u128, "Ada Reeder", ADAS_BENCH, shift(0xca9, "You", 3)),
        _ => (0xca9, "You", 0x235, shift(0xb21, "Brin", 7)),
    };
    let offer = WageOffer {
        id: Uuid::from_u128(0x393),
        employer_id: Uuid::from_u128(employer),
        employer_name: name.into(),
        employer_citizen: false,
        worker_id: None,
        worker_name: None,
        work: Work::Craft { station_id: Uuid::from_u128(station), recipe_id: "tool_handle".into() },
        x: 0.0,
        y: 0.0,
        rate: 4,
        cap: 40,
        made: 19,
        escrow_strands: 4 * 21,
        fee: court::court_fee(160),
        posted_at: now - hours(5),
        ends_at: now + hours(67),
        status: OfferStatus::Open,
        shift: Some(worker.clone()),
        employer_standing: String::new(),
    };
    let mut wages = Wages::default();
    wages.connected();
    if let Some(ask) = wages.wants() {
        wages.sent(ask);
    }
    let mine = (which == "shift").then_some(worker);
    wages.listed(WagesResponse { offers: vec![offer], shift: mine });
    wages
}

/// Ada's workbench, beside [`sample`]'s.
const ADAS_BENCH: u128 = 0xada0;

/// [`sample`] at Ada's workbench instead, where you're on a shift: its box
/// holds the same sticks and rope, hers.
pub fn at_adas_bench(state: &mut GameState) {
    use capitol_core::net::RestResponse;
    use capitol_core::protocol::rest::StationContainerResponse;
    use capitol_core::protocol::ws::ServerMessage;
    const ME: Uuid = Uuid::from_u128(0xca9);
    let (bench, boxed) = (Uuid::from_u128(ADAS_BENCH), Uuid::from_u128(0xada1));
    state.apply_ws(&ServerMessage::StationPlaced {
        station_id: bench,
        station_type: "workbench".into(),
        name: "Workbench".into(),
        owner_id: Uuid::from_u128(0xada),
        x: 20.0,
        y: 0.0,
        container_id: boxed,
        quality: 70,
        floor: 0,
        indoors: false,
    });
    state.apply_ws(&ServerMessage::StationOpened {
        player_id: ME,
        station_id: bench,
        station_type: "workbench".into(),
        name: "Workbench".into(),
        container_id: boxed,
    });
    let catalog = capitol_core::catalog::Catalog::fallback();
    let item = |n: u128, slot, kind: &str, quality, quantity| {
        SlotItem::new(Uuid::from_u128(n), kind.into(), catalog.item_name(kind), quality, quantity, slot)
    };
    state.apply_rest(&RestResponse::StationContainer(StationContainerResponse {
        id: boxed,
        container_type: "station_inventory".into(),
        slot_count: 16,
        layout_columns: 4,
        slots: vec![item(20, 0, "sticks", 66, 14), item(21, 1, "rope", 61, 7), item(22, 4, "tool_handle", 63, 3)],
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws the panel for a few frames on a `screen` of points, beside an
    /// inventory at `beside`, and returns where it went.
    fn run(panel: &mut StationPanel, state: &GameState, screen: egui::Vec2, beside: Option<egui::Rect>) -> egui::Rect {
        run_with(panel, state, None, screen, beside)
    }

    fn run_with(
        panel: &mut StationPanel,
        state: &GameState,
        wages: Option<&Wages>,
        screen: egui::Vec2,
        beside: Option<egui::Rect>,
    ) -> egui::Rect {
        let ctx = theme::test_context();
        let mut icons = ItemIcons::load(std::path::Path::new("no/icons/needed"));
        let view = View {
            state,
            phase: &Phase::Idle,
            refused: None,
            beside,
            automation_refused: None,
            wages,
            now: Utc::now(),
            online: true,
            repairing: false,
            repair_refused: None,
        };
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
            ..Default::default()
        };
        for _ in 0..30 {
            let _ = ctx.run(raw.clone(), |ctx| {
                theme::install(ctx);
                panel.show(ctx, &mut icons, &view, 0.05);
            });
        }
        ctx.memory(|m| m.area_rect(Id::new("capitol_station"))).expect("drawn")
    }

    fn rect(l: f32, t: f32, r: f32, b: f32) -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(l, t), egui::pos2(r, b))
    }

    /// 1280×720 at 150%, in points, and the inventory on it as measured
    /// from a live capture (as `hints`' tests have it).
    const SMALL: egui::Vec2 = egui::vec2(853.0, 480.0);
    fn small_inventory() -> egui::Rect {
        rect(485.0, 173.0, 834.0, 460.0)
    }

    /// The panel is on the screen, under the header strip's gap, and off
    /// its bottom and right edges.
    fn on_screen(panel: egui::Rect, screen: egui::Vec2) {
        assert!(panel.top() >= UNDER_HEADER - 0.5, "{panel:?}");
        assert!(panel.left() >= LEFT - 0.5, "{panel:?}");
        assert!(panel.bottom() <= screen.y - EDGE + 0.5, "{panel:?}");
        assert!(panel.right() <= screen.x - EDGE + 0.5, "{panel:?}");
    }

    fn craft_tab() -> StationPanel {
        StationPanel {
            picked: Some("tool_handle".into()),
            ..Default::default()
        }
    }

    #[test]
    fn on_a_small_screen_it_fits_on_both_tabs() {
        let state = sample();
        let mut panel = craft_tab();
        let alone = run(&mut panel, &state, SMALL, None);
        on_screen(alone, SMALL);
        // Two columns fit with the inventory closed.
        let gaps = theme::test_context().style().spacing.item_spacing.x;
        assert!(alone.width() > inner_width(Layout::Columns, false, gaps), "{alone:?}");
        panel.tab = Tab::Automation;
        on_screen(run(&mut panel, &state, SMALL, None), SMALL);
    }

    #[test]
    fn on_a_small_screen_it_stacks_beside_the_inventory() {
        let state = sample();
        let inventory = small_inventory();
        for tab in [Tab::Craft, Tab::Automation] {
            let mut panel = StationPanel { tab, ..craft_tab() };
            let beside = run(&mut panel, &state, SMALL, Some(inventory));
            on_screen(beside, SMALL);
            assert!(beside.width() < BOX_WIDTH + WIDTH, "stacked: {beside:?}");
            assert!((inventory.left() - GAP - beside.right()).abs() < 1.0, "{beside:?}");
        }
    }

    #[test]
    fn on_a_large_screen_it_keeps_its_columns_and_its_height() {
        let state = sample();
        let big = egui::vec2(1920.0, 1080.0);
        let inventory = small_inventory().translate(big - SMALL);
        let mut panel = craft_tab();
        let beside = run(&mut panel, &state, big, Some(inventory));
        on_screen(beside, big);
        assert!((inventory.left() - GAP - beside.right()).abs() < 1.0, "{beside:?}");
        // Drawn as wide as `place` reckoned it, so it chose by the truth.
        let gaps = theme::test_context().style().spacing.item_spacing.x;
        let margins = theme::panel_edged(STATION).total_margin().sum();
        let columns = inner_width(Layout::Columns, false, gaps) + margins.x;
        assert!((beside.width() - columns).abs() < 1.5, "{beside:?} against {columns}");
        // Nothing scrolls: the Craft tab is taller than the small screen.
        assert!(beside.height() > SMALL.y, "{beside:?}");
    }

    /// The Hire tab (S2.7) fits the small screen beside the inventory, with
    /// a running offer over the form.
    #[test]
    fn the_hire_tab_fits_beside_the_inventory() {
        let state = sample();
        let wages = sample_wages("hire");
        assert_eq!(wages.mine_at(state.me(), Uuid::from_u128(0x235)).len(), 1);
        for beside in [None, Some(small_inventory())] {
            // Opening sets the tab, so turn to Hire after, as a click would
            let mut panel = craft_tab();
            run_with(&mut panel, &state, Some(&wages), SMALL, beside);
            panel.tab = Tab::Hire;
            let drawn = run_with(&mut panel, &state, Some(&wages), SMALL, beside);
            on_screen(drawn, SMALL);
            assert_eq!(panel.tab, Tab::Hire);
            let draft = panel.hire.as_ref().expect("the form");
            assert!(matches!(&draft.work, Work::Craft { station_id, .. } if station_id.as_u128() == 0x235));
        }
        // No wages to hand (offline, no sample): no Hire tab
        let mut panel = craft_tab();
        run(&mut panel, &state, SMALL, None);
        panel.tab = Tab::Hire;
        run(&mut panel, &state, SMALL, None);
        assert_eq!(panel.tab, Tab::Craft);
    }

    /// On a shift at Ada's bench, the panel keeps to her recipe.
    #[test]
    fn on_a_shift_the_panel_keeps_to_the_employers_recipe() {
        let mut state = sample();
        at_adas_bench(&mut state);
        let wages = sample_wages("shift");
        let mut panel = StationPanel::default();
        let drawn = run_with(&mut panel, &state, Some(&wages), SMALL, Some(small_inventory()));
        on_screen(drawn, SMALL);
        assert_eq!(panel.recipe(&state).map(|r| r.id.as_str()), Some("tool_handle"));
        assert_eq!(panel.tab, Tab::Craft, "not yours: no other tab");
        // Off the shift, the box chooses again
        let mut panel = StationPanel::default();
        run_with(&mut panel, &state, None, SMALL, None);
        assert_eq!(panel.shift_recipe, None);
    }

    /// A worn station (V3.5) shows its condition and Repair, and the panel
    /// still fits the small screen beside the inventory
    #[test]
    fn a_worn_station_shows_repair_and_still_fits() {
        for which in ["repair", "worn"] {
            let mut state = sample();
            worn_out_there(&mut state, which);
            let info = state.open_station_info().unwrap();
            assert!(info.condition < 25, "{which}");
            let needs = repair::needs(state.catalog(), state.recipes(), info, state.inventory());
            assert_eq!(needs.iter().map(|n| (n.item_type.as_str(), n.need)).collect::<Vec<_>>(), vec![("fiber", 3)]);
            assert_eq!(needs[0].short() > 0, which == "worn", "{which}: {needs:?}");
            for beside in [None, Some(small_inventory())] {
                let mut panel = craft_tab();
                on_screen(run(&mut panel, &state, SMALL, beside), SMALL);
            }
        }
    }

    #[test]
    fn it_goes_where_there_is_room() {
        let screen = rect(0.0, 0.0, 853.0, 480.0);
        let (columns, stacked) = (544.0, 300.0);
        // Alone: bottom right, in columns.
        assert_eq!(place(screen, None, columns, stacked), (Layout::Columns, EDGE));
        // Beside the inventory: stacked, just left of it.
        let inventory = small_inventory();
        let right = 853.0 - inventory.left() + GAP;
        assert_eq!(place(screen, Some(inventory), columns, stacked), (Layout::Stacked, right));
        // On a wide screen, in columns beside it.
        let wide = rect(0.0, 0.0, 1280.0, 480.0);
        let far = inventory.translate(egui::vec2(427.0, 0.0));
        assert_eq!(place(wide, Some(far), columns, stacked), (Layout::Columns, right));
        // No room even stacked: as far left as it may go.
        let narrow = rect(0.0, 0.0, 500.0, 480.0);
        let squeezed = rect(250.0, 173.0, 480.0, 460.0);
        assert_eq!(
            place(narrow, Some(squeezed), columns, stacked),
            (Layout::Stacked, 500.0 - stacked - LEFT)
        );
    }
}
