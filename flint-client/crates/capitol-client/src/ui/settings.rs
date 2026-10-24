//! The settings panel (P8.4): the server, how the world is drawn, how loud
//! it is, how the camera answers, and how you walk and look (T1.1). F10 (the `settings` action) or the
//! header's Settings link opens and closes it; offline, where there's no
//! header, F10 alone does.
//!
//! Every change applies the moment it's made, and `settings::SettingsFile`
//! writes it to `save/settings.toml` once it has held still for a moment.
//! The React client has no settings panel, so this one is made of the
//! theme's parts and sits top right, under the header, clear of the
//! inventory at the bottom and the toasts at the top.
//!
//! The server isn't one of the `Settings`: it's the session's, as on the
//! login form. Changing it here starts a session on the new server, which
//! signs you in there with that server's stored token, or shows the form.
//!
//! Signed in, the last section lists the people you've muted (S1.4), each
//! with Unmute. Muting is done from a person's card.

use super::theme::{self, size, DIM, GOLD};
use crate::session::{normalize_server, server_label};
use crate::controls_stats::Played;
use crate::settings::{ControlMode, Settings, Shadows, SENSITIVITY};
use capitol_core::mutes::Mutes;
use egui::{Align2, Id, RichText};
use uuid::Uuid;

/// The panel's width inside its frame.
const WIDTH: f32 = 300.0;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// How far it slides as it opens, in points.
const RISE: f32 = 12.0;
/// Under the sections: Restore defaults, its rule and the frame's edge.
const FOOTER: f32 = 64.0;
/// The least the sections scroll in, however short the window.
const LEAST_ROOM: f32 = 160.0;
/// Kept clear at the sections' right for the scroll bar, which floats
/// over what's under it.
const BAR: f32 = 12.0;

/// What the panel asks for this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Talk to this server instead (already normalized).
    SetServer(String),
    /// Hear them again (S1.4).
    Unmute(Uuid),
    Close,
}

/// What the panel draws from, besides the settings themselves.
pub struct View<'a> {
    /// The server the session talks to, or `None` offline.
    pub server: Option<&'a str>,
    /// Who you've muted, or `None` offline.
    pub mutes: Option<&'a Mutes>,
    /// How you've played each control mode (T3.1), or `None` before
    /// you've played, and offline.
    pub played: Option<&'a Played>,
}

#[derive(Default)]
pub struct SettingsPanel {
    pub open: bool,
    /// `Some` while the server field is open, holding what's typed.
    server_edit: Option<String>,
    server_error: Option<&'static str>,
    /// Focus the server field on the next frame it's drawn.
    server_focus: bool,
    /// Where the panel was drawn this frame. `None` while it's closed.
    pub rect: Option<egui::Rect>,
    /// Whether it was open when [`SettingsPanel::just_opened`] last asked
    seen_open: bool,
}

impl SettingsPanel {
    /// Whether it has opened since this was last asked: true once per
    /// opening, so the mutes are asked for afresh.
    pub fn just_opened(&mut self) -> bool {
        let opened = self.open && !self.seen_open;
        self.seen_open = self.open;
        opened
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.server_edit = None;
            self.server_error = None;
        }
    }

    /// Draws the panel top right. Changes `settings` in place.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        view: &View,
    ) -> Option<Action> {
        let shown = ctx.animate_bool_with_time(Id::new("capitol_settings_fade"), self.open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            return None;
        }
        let ease = 1.0 - (1.0 - shown).powi(3);
        let top = ctx.available_rect().top();
        let area = egui::Area::new(Id::new("capitol_settings"))
            .anchor(Align2::RIGHT_TOP, [-16.0, top + 12.0 - RISE * (1.0 - ease)])
            // Over the inventory panel, which it reaches down beside.
            .order(egui::Order::Foreground)
            .interactable(self.open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(GOLD)
                    .show(ui, |ui| self.body(ui, settings, view))
                    .inner
            });
        self.rect = self.open.then_some(area.response.rect);
        area.inner
    }

    fn body(&mut self, ui: &mut egui::Ui, settings: &mut Settings, view: &View) -> Option<Action> {
        ui.set_width(WIDTH);
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(theme::heading("Settings").color(GOLD));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.add(
                    egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM))
                        .frame(false),
                );
                if close
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Close (F10)")
                    .clicked()
                {
                    action = Some(Action::Close);
                }
            });
        });
        ui.add_space(2.0);
        theme::gold_rule(ui);
        // The sections scroll when the window is too short for them all,
        // under the title and over Restore defaults, which stay put.
        let room = (ui.ctx().screen_rect().bottom() - ui.cursor().top() - FOOTER).max(LEAST_ROOM);
        theme::scrolled(ui, "capitol_settings_sections", room, |ui| {
            let gutter = egui::Margin {
                right: BAR,
                ..Default::default()
            };
            egui::Frame::none().inner_margin(gutter).show(ui, |ui| {
                if let Some(a) = self.sections(ui, settings, view) {
                    action = Some(a);
                }
            });
        });

        ui.add_space(10.0);
        theme::divider(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if theme::link(ui, "Restore defaults", size::SMALL).clicked() {
                *settings = Settings::default();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::dim("Saved as you change them").size(size::LABEL));
            });
        });
        action
    }

    /// Every section, from the server down to the controls.
    fn sections(&mut self, ui: &mut egui::Ui, settings: &mut Settings, view: &View) -> Option<Action> {
        section(ui, "Server");
        let action = self.server(ui, view);

        section(ui, "Graphics");
        let g = &mut settings.graphics;
        theme::setting_row(ui, "Ambient occlusion", |ui| theme::switch(ui, &mut g.ssao))
            .on_hover_text("The soft shade where walls meet the ground");
        theme::setting_row(ui, "Bloom", |ui| theme::switch(ui, &mut g.bloom))
            .on_hover_text("The glow round the lantern and the forge");
        let shadows: Vec<(Shadows, &str)> = Shadows::ALL.iter().map(|s| (*s, s.label())).collect();
        theme::setting_row(ui, "Shadows", |ui| {
            theme::segmented(ui, &mut g.shadows, &shadows)
        });

        section(ui, "Sound");
        let a = &mut settings.audio;
        for (name, v) in [
            ("Master", &mut a.master),
            ("Ambience", &mut a.ambience),
            ("Effects", &mut a.effects),
            ("Interface", &mut a.interface),
        ] {
            theme::setting_row(ui, name, |ui| theme::level(ui, v, 0.0..=1.0, percent));
        }

        section(ui, "Camera");
        let c = &mut settings.camera;
        theme::setting_row(ui, "Sensitivity", |ui| {
            theme::level(ui, &mut c.sensitivity, SENSITIVITY, |v| format!("{v:.2}×"))
        })
        .on_hover_text("How fast the wheel, R/F, +/-, Q/E and a right-drag turn and zoom");
        theme::setting_row(ui, "Invert orbit", |ui| {
            theme::switch(ui, &mut c.invert_orbit)
        })
        .on_hover_text("Q/E and a right-drag turn the other way");

        section(ui, "Controls");
        let k = &mut settings.controls;
        let modes: Vec<(ControlMode, &str)> =
            ControlMode::ALL.iter().map(|m| (*m, m.label())).collect();
        // Both names don't fit beside a row's label: the switch has the row.
        ui.add_space(2.0);
        ui.horizontal(|ui| theme::segmented(ui, &mut k.mode, &modes)).inner.on_hover_text(
            "F8 switches live. Over the shoulder is only its name for now: clicks still walk",
        );
        ui.add_space(4.0);
        theme::setting_row(ui, "Mouse-look", |ui| {
            theme::level(ui, &mut k.look_sensitivity, SENSITIVITY, |v| format!("{v:.2}×"))
        })
        .on_hover_text("How fast the mouse turns the view over the shoulder");
        theme::setting_row(ui, "Invert look", |ui| {
            theme::switch(ui, &mut k.invert_look_y)
        })
        .on_hover_text("Over the shoulder, moving the mouse up looks down");
        theme::setting_row(ui, "Pad look", |ui| {
            theme::level(ui, &mut k.pad_look_sensitivity, SENSITIVITY, |v| format!("{v:.2}×"))
        })
        .on_hover_text("How fast the right stick turns the view over the shoulder");
        theme::setting_row(ui, "Invert pad look", |ui| {
            theme::switch(ui, &mut k.invert_pad_y)
        })
        .on_hover_text("Over the shoulder, pushing the right stick up looks down");
        if let Some(played) = view.played {
            how_youve_played(ui, played);
        }

        if let Some(mutes) = view.mutes {
            section(ui, "Muted");
            if let Some(id) = muted(ui, mutes) {
                return Some(Action::Unmute(id));
            }
        }
        action
    }

    /// The server, and the field to change it. Offline, none.
    fn server(&mut self, ui: &mut egui::Ui, view: &View) -> Option<Action> {
        let Some(server) = view.server else {
            theme::setting_row(ui, "Server", |ui| {
                ui.label(theme::dim("None, offline").size(size::BODY))
            });
            return None;
        };
        let Some(edit) = &mut self.server_edit else {
            theme::setting_row(ui, "Server", |ui| {
                ui.label(theme::text(server_label(server)).size(size::BODY));
                ui.add_space(6.0);
                if theme::link(ui, "Change", size::SMALL).clicked() {
                    self.server_edit = Some(server.to_owned());
                    self.server_error = None;
                    self.server_focus = true;
                }
            });
            return None;
        };
        let mut submit = false;
        let mut cancel = false;
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            theme::gold_fields(ui);
            let field = ui.add(
                egui::TextEdit::singleline(edit)
                    .hint_text("localhost:3000")
                    .desired_width(WIDTH - 130.0),
            );
            if std::mem::take(&mut self.server_focus) {
                field.request_focus();
            }
            submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            submit |= theme::button(ui, "Connect", true).clicked();
            cancel = theme::link(ui, "Cancel", size::SMALL).clicked();
        });
        ui.label(theme::dim("Changing it signs you in there").size(size::LABEL));
        if let Some(why) = self.server_error {
            ui.add_space(4.0);
            theme::error_box(ui, why);
        }
        if cancel {
            self.server_edit = None;
            self.server_error = None;
            return None;
        }
        if !submit {
            return None;
        }
        match normalize_server(edit) {
            Some(new) => {
                self.server_edit = None;
                self.server_error = None;
                (new != server).then_some(Action::SetServer(new))
            }
            None => {
                self.server_error = Some("Give a server like localhost:3000 or https://host.");
                None
            }
        }
    }
}

/// The people you've muted, newest first, each with Unmute.
fn muted(ui: &mut egui::Ui, mutes: &Mutes) -> Option<Uuid> {
    let Some(list) = mutes.list() else {
        ui.label(theme::dim("Asking the city…").size(size::SMALL).italics());
        return None;
    };
    if list.is_empty() {
        ui.label(theme::dim("No one. Mute someone from their card.").size(size::SMALL).italics());
        return None;
    }
    let mut unmute = None;
    for person in list {
        ui.horizontal(|ui| {
            if person.citizen {
                theme::citizen_mark(ui, size::BODY, None);
            }
            ui.label(theme::text(&person.name).size(size::BODY));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::link(ui, "Unmute", size::SMALL).on_hover_text("Hear them again").clicked() {
                    unmute = Some(person.id);
                }
            });
        });
    }
    unmute
}

/// A section's name, small and spaced, over its rows.
/// How you've played (T3.1): each mode's numbers side by side, small, for
/// the A/B. Kept on this machine alone.
fn how_youve_played(ui: &mut egui::Ui, played: &Played) {
    ui.add_space(8.0);
    ui.label(theme::dim("How you've played").size(size::SMALL).italics())
        .on_hover_text("Kept on this machine only, in save/controls_stats.toml");
    ui.add_space(2.0);
    egui::Grid::new("controls_stats").num_columns(3).spacing([12.0, 1.0]).show(ui, |ui| {
        // The modes' full names don't fit three columns in the panel
        ui.label("");
        for head in ["Point", "Shoulder"] {
            ui.label(theme::dim(head).size(size::SMALL));
        }
        ui.end_row();
        for (name, click, direct) in played.rows() {
            ui.label(theme::dim(name).size(size::SMALL));
            ui.label(theme::text(click).size(size::SMALL));
            ui.label(theme::text(direct).size(size::SMALL));
            ui.end_row();
        }
    });
}

fn section(ui: &mut egui::Ui, name: &str) {
    ui.add_space(10.0);
    ui.label(theme::label(name));
    ui.add_space(2.0);
}

fn percent(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame of the panel open, with the theme in.
    fn frame(
        panel: &mut SettingsPanel,
        settings: &mut Settings,
        input: egui::RawInput,
    ) -> Option<Action> {
        frame_with(panel, settings, input, None)
    }

    fn frame_with(
        panel: &mut SettingsPanel,
        settings: &mut Settings,
        input: egui::RawInput,
        played: Option<&Played>,
    ) -> Option<Action> {
        let ctx = theme::test_context();
        let mut action = None;
        for raw in [egui::RawInput::default(), egui::RawInput::default(), input] {
            let _ = ctx.run(raw, |ctx| {
                theme::install(ctx);
                let view = View {
                    server: Some("http://localhost:3000"),
                    mutes: None,
                    played,
                };
                action = panel.show(ctx, settings, &view);
            });
        }
        action
    }

    #[test]
    fn how_youve_played_fits_the_panel() {
        let mut settings = Settings::default();
        let mut plain = SettingsPanel::default();
        plain.toggle();
        frame(&mut plain, &mut settings, egui::RawInput::default());
        // Long figures in both columns
        let mut played = Played::default();
        for m in [&mut played.click, &mut played.direct] {
            m.seconds = 600_000.0;
            m.distance = 12_345_678.0;
            m.gathers = 99_999;
        }
        let mut panel = SettingsPanel::default();
        panel.toggle();
        frame_with(&mut panel, &mut settings, egui::RawInput::default(), Some(&played));
        let (plain, with) = (plain.rect.unwrap(), panel.rect.unwrap());
        assert!(with.width() <= plain.width() + 0.5, "{} against {}", with.width(), plain.width());
    }

    #[test]
    fn the_panel_draws_every_section_when_open() {
        let mut panel = SettingsPanel::default();
        panel.toggle();
        let mut settings = Settings::default();
        assert_eq!(
            frame(&mut panel, &mut settings, egui::RawInput::default()),
            None
        );
        let rect = panel.rect.expect("drawn");
        assert!(rect.width() >= WIDTH && rect.height() > 300.0, "{rect:?}");
        assert_eq!(settings, Settings::default(), "drawing changes nothing");
    }

    #[test]
    fn the_muted_are_listed_each_with_unmute() {
        use capitol_core::protocol::rest::MutedPlayer;
        let ada = Uuid::from_u128(0xada);
        let mut mutes = Mutes::default();
        mutes.answered(vec![MutedPlayer { id: ada, name: "Ada Reeder".into(), citizen: false }]);
        let mut panel = SettingsPanel::default();
        panel.toggle();
        let mut settings = Settings::default();
        let run = |panel: &mut SettingsPanel, settings: &mut Settings, click: Option<egui::Pos2>| {
            let ctx = theme::test_context();
            let mut action = None;
            // The sections' scroll area grows to its height over a few frames
            for frame in 0..14 {
                // Tall enough that the last section isn't scrolled out of sight
                let mut raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 1600.0))),
                    ..Default::default()
                };
                if let (Some(p), 12) = (click, frame) {
                    raw.events.push(egui::Event::PointerMoved(p));
                    for pressed in [true, false] {
                        raw.events.push(egui::Event::PointerButton {
                            pos: p,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        });
                    }
                }
                let _ = ctx.run(raw, |ctx| {
                    theme::install(ctx);
                    let view = View { server: Some("http://localhost:3000"), mutes: Some(&mutes), played: None };
                    if let Some(a) = panel.show(ctx, settings, &view) {
                        action = Some(a);
                    }
                });
            }
            action
        };
        run(&mut panel, &mut settings, None);
        let rect = panel.rect.expect("drawn");
        // Unmute sits at the right of the last section's row, over the footer
        let found = (0..(rect.height() as i32 / 4))
            .map(|k| egui::pos2(rect.right() - 40.0, rect.bottom() - 4.0 * k as f32))
            .find_map(|at| run(&mut panel, &mut settings, Some(at)));
        assert_eq!(found, Some(Action::Unmute(ada)));
        assert_eq!(settings, Settings::default(), "unmuting changes no setting");
    }

    #[test]
    fn a_closed_panel_draws_nothing() {
        let mut panel = SettingsPanel::default();
        let mut settings = Settings::default();
        frame(&mut panel, &mut settings, egui::RawInput::default());
        assert!(panel.rect.is_none());
    }
}
