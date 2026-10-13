//! The login form (P4.1), over the live world: the camera circles the tower
//! behind it (`camera.rhai`, `cap_at_login`). Two tabs, Sign in and Create
//! account, as the React client's two pages. Tab moves between fields and
//! Enter submits. Under the form the server it talks to, whether that
//! server is answering, and a field to change it.
//!
//! It holds only what's typed. What happens is `capitol_core::auth`'s: the
//! form returns a [`FormAction`] and draws whatever [`LoginView`] says.
//! Only the basic theme for now; P5.1 themes all of the egui.

use capitol_core::auth::Phase;
use capitol_core::net::ConnectionState;
use egui::{Color32, Key, RichText, Stroke};

use super::{BG_DARK, DIM, ERROR, GOLD, GOLD_DIM, ONLINE, TEXT};
use crate::session::{normalize_server, server_label};

/// The form's width in points.
const WIDTH: f32 = 340.0;

/// What the form shows, from the session.
pub struct LoginView<'a> {
    pub phase: Phase,
    pub notice: Option<String>,
    /// The notice is a wrong password: clear the field for another try.
    pub wrong_password: bool,
    pub reachable: Option<bool>,
    pub server: &'a str,
    /// The email to fill the form with the first time.
    pub remembered_email: Option<&'a str>,
    /// The player's name, or the email, while entering.
    pub who: String,
    pub connection: Option<&'a ConnectionState>,
}

/// What the player asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum FormAction {
    LogIn {
        email: String,
        password: String,
    },
    Register {
        name: String,
        email: String,
        password: String,
    },
    /// Talk to this server instead (already normalized).
    SetServer(String),
    /// Stop entering with the stored token; show the form.
    SignInAsSomeoneElse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    SignIn,
    Register,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Email,
    Password,
}

#[derive(Default)]
pub struct LoginForm {
    tab: Tab,
    name: String,
    email: String,
    password: String,
    /// `Some` while the server field is open, holding what's typed.
    server_edit: Option<String>,
    server_error: Option<&'static str>,
    /// Focus the server field on the next frame it's drawn.
    server_focus: bool,
    /// Focus this field on the next frame.
    focus: Option<Field>,
    /// The remembered email has been filled in.
    filled: bool,
    last_phase: Option<Phase>,
}

impl LoginForm {
    /// Draws the form at `opacity` (it fades out as the player enters the
    /// world). Returns what the player asked for, if anything.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        view: &LoginView,
        opacity: f32,
    ) -> Option<FormAction> {
        self.follow(view);
        let mut action = None;
        egui::Area::new(egui::Id::new("capitol_login"))
            .anchor(egui::Align2::LEFT_CENTER, [72.0, 0.0])
            .interactable(opacity > 0.5)
            .show(ctx, |ui| {
                ui.multiply_opacity(opacity);
                super::panel_frame()
                    .inner_margin(egui::Margin::same(28.0))
                    .stroke(Stroke::new(1.0, GOLD_DIM.gamma_multiply(0.6)))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH);
                        style_fields(ui);
                        action = self.contents(ui, view);
                    });
            });
        action
    }

    /// Keeps the fields in step with the session.
    fn follow(&mut self, view: &LoginView) {
        if !self.filled {
            self.filled = true;
            if let Some(email) = view.remembered_email {
                self.email = email.to_owned();
            }
            self.focus = Some(self.first_empty());
        }
        if self.last_phase != Some(view.phase) {
            match view.phase {
                // Back at the form, after a failure or a sign-out.
                Phase::SignedOut if self.last_phase.is_some() => {
                    if view.wrong_password {
                        self.password.clear();
                    }
                    self.focus = Some(self.first_empty());
                }
                // Never keep a password longer than it's needed.
                Phase::Entering { .. } | Phase::InWorld => self.password.clear(),
                _ => {}
            }
            self.last_phase = Some(view.phase);
        }
    }

    fn first_empty(&self) -> Field {
        if self.tab == Tab::Register && self.name.trim().is_empty() {
            Field::Name
        } else if self.email.trim().is_empty() {
            Field::Email
        } else {
            Field::Password
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui, view: &LoginView) -> Option<FormAction> {
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("THE CAPITOL")
                    .color(GOLD)
                    .size(28.0)
                    .extra_letter_spacing(5.0)
                    .strong(),
            );
            ui.add_space(4.0);
            let subtitle = match self.tab {
                Tab::SignIn => "Enter the world of commerce and power",
                Tab::Register => "Begin your journey to power",
            };
            ui.label(RichText::new(subtitle).color(DIM).size(13.0).italics());
        });
        ui.add_space(14.0);
        rule(ui);
        ui.add_space(14.0);

        let mut action = match view.phase {
            Phase::SignedOut | Phase::SigningIn { .. } => self.fields(ui, view),
            Phase::Entering { .. } | Phase::InWorld => entering(ui, view),
        };

        // Not under the welcome: it's said its piece.
        if view.phase != Phase::InWorld {
            ui.add_space(18.0);
            if let Some(server) = self.server_footer(ui, view) {
                action = Some(server);
            }
        }
        action
    }

    fn fields(&mut self, ui: &mut egui::Ui, view: &LoginView) -> Option<FormAction> {
        let busy = matches!(view.phase, Phase::SigningIn { .. });
        ui.horizontal(|ui| {
            for (tab, text) in [(Tab::SignIn, "Sign in"), (Tab::Register, "Create account")] {
                if tab_button(ui, text, self.tab == tab).clicked() && !busy && self.tab != tab {
                    self.tab = tab;
                    self.focus = Some(self.first_empty());
                }
                ui.add_space(14.0);
            }
        });
        ui.add_space(14.0);

        let mut submit = false;
        if self.tab == Tab::Register {
            submit |= self.field(ui, Field::Name, busy);
        }
        submit |= self.field(ui, Field::Email, busy);
        submit |= self.field(ui, Field::Password, busy);

        if let Some(notice) = &view.notice {
            ui.add_space(2.0);
            notice_box(ui, notice);
            ui.add_space(10.0);
        } else {
            ui.add_space(6.0);
        }

        let label = match (self.tab, busy) {
            (Tab::SignIn, false) => "Sign in",
            (Tab::SignIn, true) => "Signing in…",
            (Tab::Register, false) => "Create account",
            (Tab::Register, true) => "Creating account…",
        };
        let button = egui::Button::new(RichText::new(label).color(BG_DARK).size(15.0).strong())
            .fill(if busy { GOLD_DIM } else { GOLD })
            .stroke(Stroke::NONE)
            .rounding(4.0)
            .min_size(egui::vec2(0.0, 40.0));
        // Justified, so the button spans the form with its label centred.
        submit |= ui
            .vertical_centered_justified(|ui| ui.add_enabled(!busy, button))
            .inner
            .clicked();

        if !submit || busy {
            return None;
        }
        Some(match self.tab {
            Tab::SignIn => FormAction::LogIn {
                email: self.email.clone(),
                password: self.password.clone(),
            },
            Tab::Register => FormAction::Register {
                name: self.name.clone(),
                email: self.email.clone(),
                password: self.password.clone(),
            },
        })
    }

    /// One labelled field. Returns whether Enter was pressed in it.
    fn field(&mut self, ui: &mut egui::Ui, field: Field, busy: bool) -> bool {
        let (label, hint) = match field {
            Field::Name => ("PLAYER NAME", "how the city will know you"),
            Field::Email => ("EMAIL", "you@example.com"),
            Field::Password => (
                "PASSWORD",
                match self.tab {
                    Tab::SignIn => "",
                    Tab::Register => "at least 8 characters",
                },
            ),
        };
        ui.label(
            RichText::new(label)
                .color(DIM)
                .size(11.0)
                .extra_letter_spacing(1.2),
        );
        ui.add_space(4.0);
        let text = match field {
            Field::Name => &mut self.name,
            Field::Email => &mut self.email,
            Field::Password => &mut self.password,
        };
        let edit = egui::TextEdit::singleline(text)
            .hint_text(RichText::new(hint).color(DIM.gamma_multiply(0.7)))
            .text_color(TEXT)
            .font(egui::FontId::proportional(15.0))
            .margin(egui::Margin::symmetric(10.0, 8.0))
            .desired_width(f32::INFINITY)
            .password(field == Field::Password)
            .char_limit(match field {
                Field::Name => 32,
                _ => 254,
            });
        let response = ui.add_enabled(!busy, edit);
        // Asked until it takes: a new Area's first frame only measures, and
        // a disabled field can't take focus until the sign-in comes back.
        if self.focus == Some(field) {
            response.request_focus();
            if response.has_focus() {
                self.focus = None;
            }
        }
        ui.add_space(12.0);
        response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
    }

    /// The server, whether it's answering, and the field to change it.
    fn server_footer(&mut self, ui: &mut egui::Ui, view: &LoginView) -> Option<FormAction> {
        let Some(edit) = &mut self.server_edit else {
            let mut open = false;
            ui.horizontal(|ui| {
                let (colour, state) = match view.reachable {
                    Some(true) => (ONLINE, "online"),
                    Some(false) => (ERROR, "can't reach it"),
                    None => (DIM, "checking…"),
                };
                let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, colour);
                ui.label(
                    RichText::new(format!("{} · {state}", server_label(view.server)))
                        .color(DIM)
                        .size(12.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    open = ui
                        .add(
                            egui::Label::new(
                                RichText::new("change")
                                    .color(GOLD_DIM)
                                    .size(12.0)
                                    .underline(),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked();
                });
            });
            if open {
                self.server_edit = Some(view.server.to_owned());
                self.server_error = None;
                self.server_focus = true;
            }
            return None;
        };

        ui.label(
            RichText::new("SERVER")
                .color(DIM)
                .size(11.0)
                .extra_letter_spacing(1.2),
        );
        ui.add_space(4.0);
        let mut apply = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(edit)
                    .text_color(TEXT)
                    .margin(egui::Margin::symmetric(8.0, 6.0))
                    .desired_width(WIDTH - 110.0),
            );
            if std::mem::take(&mut self.server_focus) {
                response.request_focus();
            }
            apply |= response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            apply |= ui.button(RichText::new("Use").color(GOLD)).clicked();
            cancel |= ui.button(RichText::new("Cancel").color(DIM)).clicked();
        });
        if let Some(why) = self.server_error {
            ui.label(RichText::new(why).color(ERROR).size(12.0));
        }
        if cancel {
            self.server_edit = None;
            return None;
        }
        if !apply {
            return None;
        }
        match normalize_server(edit) {
            Some(server) => {
                self.server_edit = None;
                (server != view.server).then_some(FormAction::SetServer(server))
            }
            None => {
                self.server_error = Some("Give a server like localhost:3000 or https://host.");
                None
            }
        }
    }
}

/// While the WebSocket opens: who's entering, how it's going, and a way out
/// if the server isn't answering.
fn entering(ui: &mut egui::Ui, view: &LoginView) -> Option<FormAction> {
    if view.phase == Phase::InWorld {
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(format!("Welcome, {}.", view.who))
                    .color(TEXT)
                    .size(16.0),
            );
        });
        return None;
    }
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(16.0).color(GOLD));
        ui.label(
            RichText::new(format!("Entering the city as {}", view.who))
                .color(TEXT)
                .size(15.0),
        );
    });
    ui.add_space(8.0);
    let detail = match view.connection {
        Some(ConnectionState::Reconnecting { attempt, .. }) => {
            format!("The server isn't answering. Trying again (attempt {attempt})…")
        }
        Some(ConnectionState::Authenticating) => "Showing your papers at the gate…".into(),
        _ => "Connecting…".into(),
    };
    ui.label(RichText::new(detail).color(DIM).size(13.0));
    ui.add_space(14.0);
    let other = ui
        .add(
            egui::Label::new(
                RichText::new("Sign in as someone else")
                    .color(GOLD_DIM)
                    .size(13.0)
                    .underline(),
            )
            .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    other.clicked().then_some(FormAction::SignInAsSomeoneElse)
}

/// The fields' look: dark wells, a dim gold edge that turns gold on focus.
fn style_fields(ui: &mut egui::Ui) {
    let visuals = ui.visuals_mut();
    visuals.extreme_bg_color = BG_DARK;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, GOLD_DIM.gamma_multiply(0.7));
    visuals.widgets.inactive.rounding = 4.0.into();
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, GOLD_DIM);
    visuals.widgets.hovered.rounding = 4.0.into();
    visuals.widgets.active.rounding = 4.0.into();
    visuals.selection.stroke = Stroke::new(1.0, GOLD);
    visuals.selection.bg_fill = GOLD.gamma_multiply(0.35);
    visuals.text_cursor.stroke = Stroke::new(1.5, GOLD);
}

/// A thin gold rule across the form, fading at both ends.
fn rule(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    let painter = ui.painter();
    let steps = 24;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let fade = 1.0 - (2.0 * ((t0 + t1) / 2.0) - 1.0).abs();
        let a = rect.left() + rect.width() * t0;
        let b = rect.left() + rect.width() * t1;
        painter.line_segment(
            [
                egui::pos2(a, rect.center().y),
                egui::pos2(b, rect.center().y),
            ],
            Stroke::new(1.0, GOLD_DIM.gamma_multiply(fade)),
        );
    }
}

/// A tab heading: gold and underlined when it's the open one.
fn tab_button(ui: &mut egui::Ui, text: &str, active: bool) -> egui::Response {
    let colour = if active { GOLD } else { DIM };
    let response = ui
        .add(
            egui::Label::new(RichText::new(text).color(colour).size(14.0).strong())
                .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if active {
        let r = response.rect;
        ui.painter().line_segment(
            [
                egui::pos2(r.left(), r.bottom() + 3.0),
                egui::pos2(r.right(), r.bottom() + 3.0),
            ],
            Stroke::new(2.0, GOLD),
        );
    }
    response
}

/// The form's message, in the React client's error box.
fn notice_box(ui: &mut egui::Ui, text: &str) {
    egui::Frame::none()
        .fill(Color32::from_rgba_unmultiplied(220, 53, 69, 26))
        .stroke(Stroke::new(1.0, ERROR))
        .rounding(4.0)
        .inner_margin(egui::Margin::symmetric(12.0, 9.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(ERROR).size(13.0));
        });
}
