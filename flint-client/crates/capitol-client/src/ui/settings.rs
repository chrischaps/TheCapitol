//! The settings panel (P8.4): the server, how the world is drawn, how loud
//! it is, and how the camera answers. F10 (the `settings` action) or the
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

use super::theme::{self, size, DIM, GOLD};
use crate::session::{normalize_server, server_label};
use crate::settings::{Settings, Shadows, SENSITIVITY};
use egui::{Align2, Id, RichText};

/// The panel's width inside its frame.
const WIDTH: f32 = 300.0;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// How far it slides as it opens, in points.
const RISE: f32 = 12.0;

/// What the panel asks for this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Talk to this server instead (already normalized).
    SetServer(String),
    Close,
}

/// What the panel draws from, besides the settings themselves.
pub struct View<'a> {
    /// The server the session talks to, or `None` offline.
    pub server: Option<&'a str>,
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
}

impl SettingsPanel {
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

        section(ui, "Server");
        if let Some(a) = self.server(ui, view) {
            action = Some(a);
        }

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

/// A section's name, small and spaced, over its rows.
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
        let ctx = egui::Context::default();
        let mut action = None;
        for raw in [egui::RawInput::default(), egui::RawInput::default(), input] {
            let _ = ctx.run(raw, |ctx| {
                if !theme::install(ctx) {
                    return;
                }
                let view = View {
                    server: Some("http://localhost:3000"),
                };
                action = panel.show(ctx, settings, &view);
            });
        }
        action
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
    fn a_closed_panel_draws_nothing() {
        let mut panel = SettingsPanel::default();
        let mut settings = Settings::default();
        frame(&mut panel, &mut settings, egui::RawInput::default());
        assert!(panel.rect.is_none());
    }
}
