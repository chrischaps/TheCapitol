//! The station panel's Automation tab (E4.4), for the station's owner:
//! what it makes on its own and how far it has got, why it waits and what
//! fixes that, the form that sets it up, Start and Stop, and the
//! production log. The rules are `capitol_core::automation`; the server's
//! side is E4.2 and E4.3.
//!
//! It works from the station's own box, into the same box, so the box on
//! the panel's left is where you load it and take its work out. While it
//! runs or waits, the form shows what it's set to and only Stop is offered:
//! the server stops a station to set it up again anyway.

use super::pad_nav::FollowFocus;
use super::icons::ItemIcons;
use super::slots::grade_pip;
use super::theme::{self, size, EMBER, EMBER_HOT, PILL, STATION};
use capitol_core::automation::{pause_words, progress, Draft, Status, MAX_COUNT};
use capitol_core::chrono::Utc;
use capitol_core::protocol::rest::{Produced, StationInfo};
use capitol_core::state::GameState;
use capitol_core::station;
use egui::{Id, RichText};

/// The production log's rows: an icon this tall, and the most shown before
/// it scrolls: four, or two on a short screen (1280×720 at 150% is 480
/// points high). The panel is still taller than that screen, as it was
/// before this tab: #235.
const ROW_ICON: f32 = 20.0;
const LOG_HEIGHT: f32 = 132.0;
const LOG_HEIGHT_SHORT: f32 = 64.0;
const SHORT_SCREEN: f32 = 640.0;

/// What the tab asks for this frame.
pub enum Ask {
    Start,
    Stop,
}

/// The tab, for `info`, whose owner you are. `draft` is the form, kept by
/// the panel.
pub fn show(
    ui: &mut egui::Ui,
    icons: &mut ItemIcons,
    state: &GameState,
    info: &StationInfo,
    draft: &mut Draft,
    refused: Option<&str>,
) -> Option<Ask> {
    let set = info.automation.as_ref();
    let status = Status::of(set);
    let working = matches!(status, Status::Running | Status::Paused(_));
    // While it works, the form is what it works at.
    if working {
        *draft = Draft::from(set, None);
    }
    let mut ask = None;

    status_well(ui, state, info, &status);
    ui.add_space(8.0);

    ui.add_enabled_ui(!working, |ui| {
        recipe_picker(ui, state, &info.station_type, draft);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(theme::dim("Run:"));
            theme::segmented(ui, &mut draft.forever, &[(false, "Count"), (true, "Forever")]);
            if !draft.forever {
                ui.add_space(8.0);
                if ui.small_button("−").clicked() {
                    draft.count = draft.count.saturating_sub(step(draft.count)).max(1);
                }
                ui.add(
                    egui::DragValue::new(&mut draft.count)
                        .range(1..=MAX_COUNT)
                        .speed(0.25),
                );
                if ui.small_button("+").clicked() {
                    draft.count = (draft.count + step(draft.count)).min(MAX_COUNT);
                }
            }
        });
    });
    ui.add_space(6.0);

    if working {
        if theme::accent_button(ui, "Stop", STATION, true)
            .on_hover_text("It keeps its setting and its count")
            .clicked()
        {
            ask = Some(Ask::Stop);
        }
    } else {
        let label = match (&status, draft.differs(set)) {
            (Status::Stopped, false) => "Carry on",
            (Status::Finished, false) => "Run again",
            _ => "Start",
        };
        let ready = draft.recipe_id.is_some();
        let button = theme::accent_button(ui, label, STATION, ready);
        let button = if ready {
            button
        } else {
            button.on_disabled_hover_text("Pick what it should make")
        };
        if button.clicked() {
            ask = Some(Ask::Start);
        }
    }
    if let Some(why) = refused {
        ui.add_space(6.0);
        theme::error_box(ui, why);
    }

    ui.add_space(8.0);
    log(ui, icons, state, &info.production);
    ask
}

/// Steps for − and +: by ones to ten, then tens, then hundreds.
fn step(count: u32) -> u32 {
    match count {
        0..10 => 1,
        10..100 => 10,
        _ => 100,
    }
}

/// Where it stands: a state pill, the recipe and the count, and a pause's
/// reason and remedy.
fn status_well(ui: &mut egui::Ui, state: &GameState, info: &StationInfo, status: &Status) {
    let set = info.automation.as_ref();
    let recipe = set.and_then(|a| state.recipe(&a.recipe_id));
    theme::well().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let t = ui.input(|i| i.time) as f32;
            let (text, fill) = match status {
                Status::Unset => ("Not set up", PILL),
                Status::Stopped => ("Stopped", PILL),
                Status::Finished => ("Finished", PILL),
                Status::Running => {
                    ui.ctx().request_repaint();
                    ("Running", STATION.gamma_multiply(0.45 + 0.15 * (t * 2.4).sin()))
                }
                Status::Paused(_) => ("Paused", EMBER.gamma_multiply(0.35)),
            };
            theme::pill(ui, text, fill);
            if let (Some(a), Some(r)) = (set, recipe) {
                ui.label(theme::text(&r.name).size(size::SMALL));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::dim(progress(a)));
                });
            }
        });
        match status {
            Status::Paused(reason) => {
                let words = pause_words(reason, state.catalog(), recipe);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(words.what)
                        .font(theme::strong(size::BODY))
                        .color(EMBER_HOT),
                );
                ui.label(theme::dim(words.fix).size(size::LABEL));
            }
            Status::Unset => {
                ui.add_space(2.0);
                ui.label(
                    theme::dim("It makes one recipe again and again, from what's in its box into its box")
                        .size(size::LABEL),
                );
            }
            Status::Finished => {
                ui.add_space(2.0);
                ui.label(theme::dim("It made all it was asked for").size(size::LABEL));
            }
            _ => {}
        }
    });
}

/// The station's recipes, the one it's set to or the form's picked.
fn recipe_picker(ui: &mut egui::Ui, state: &GameState, station_type: &str, draft: &mut Draft) {
    let shown = draft
        .recipe_id
        .as_deref()
        .and_then(|id| state.recipe(id))
        .map_or_else(|| "Pick a recipe".to_owned(), |r| r.name.clone());
    ui.horizontal(|ui| {
        ui.label(theme::dim("Makes:"));
        egui::ComboBox::from_id_salt("capitol_automation_recipe")
            .width(ui.available_width())
            .selected_text(theme::text(shown))
            .show_ui(ui, |ui| {
                for r in station::recipes_at(state.recipes(), station_type) {
                    ui.selectable_value(&mut draft.recipe_id, Some(r.id.clone()), &r.name);
                }
            });
    });
}

/// The production log, newest first: icon, item and count, its quality as a
/// pip and a number, and how long ago.
fn log(ui: &mut egui::Ui, icons: &mut ItemIcons, state: &GameState, production: &[Produced]) {
    ui.horizontal(|ui| {
        ui.label(theme::label("Made here"));
        if !production.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::dim(format!("last {}", production.len())).size(size::LABEL));
            });
        }
    });
    if production.is_empty() {
        ui.label(theme::dim("Nothing yet").size(size::LABEL));
        return;
    }
    let now = Utc::now();
    egui::ScrollArea::vertical()
        .id_salt(Id::new("capitol_automation_log"))
        .max_height(if ui.ctx().screen_rect().height() < SHORT_SCREEN {
            LOG_HEIGHT_SHORT
        } else {
            LOG_HEIGHT
        })
        .auto_shrink([false, true])
        .show_following(ui, |ui| {
            for made in production.iter().rev() {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(ROW_ICON, ROW_ICON), egui::Sense::hover());
                    icons.paint(ui, rect, &made.item_type);
                    let name = state.catalog().item_name(&made.item_type);
                    let name = if made.quantity > 1 {
                        format!("{name} ×{}", made.quantity)
                    } else {
                        name
                    };
                    ui.label(theme::text(name).size(size::SMALL));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::dim(ago((now - made.at).num_seconds())).size(size::LABEL));
                        ui.add_space(6.0);
                        ui.label(theme::dim(made.quality.to_string()));
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        grade_pip(ui.painter(), rect.center(), made.quality, 1.0);
                    });
                });
            }
        });
}

/// "just now", "40s ago", "12m ago", "3h ago", "2d ago".
fn ago(seconds: i64) -> String {
    match seconds {
        ..5 => "just now".to_owned(),
        5..60 => format!("{seconds}s ago"),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86_400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_reads_like_speech() {
        assert_eq!(ago(-3), "just now");
        assert_eq!(ago(42), "42s ago");
        assert_eq!(ago(61), "1m ago");
        assert_eq!(ago(7300), "2h ago");
        assert_eq!(ago(200_000), "2d ago");
        assert_eq!((step(9), step(10), step(250)), (1, 10, 100));
    }
}
