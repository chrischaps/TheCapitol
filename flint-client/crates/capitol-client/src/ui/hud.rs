//! The parts of the HUD that are egui (P4.5): the header strip across the
//! top, and the overlay while the connection is down. The zone banner is
//! the script HUD's (`scripts/hud.rhai`), as the plan's world HUD is.
//!
//! - **The header** is React's (`pages/Game.tsx`): the game's name on the
//!   left; on the right the zone, the strand balance, who's signed in with
//!   a connection light, and Sign out. The balance comes from the store,
//!   which the poller keeps fresh (`Fetch::POLLED`).
//! - **The overlay** dims the world while a dropped connection is retried,
//!   and says which attempt it is, why the last one ended, and counts down
//!   to the next. A drop shorter than [`GRACE`] shows nothing, so a blip
//!   doesn't flash. On the way back it says so as it fades. A token refused
//!   while away isn't an outage: the login form takes over, with its notice.

use std::time::{Duration, Instant};

use capitol_core::net::ConnectionState;
use capitol_core::protocol::rules::Zone;
use egui::{Align, Align2, Id, Layout, Margin, Order, RichText, Stroke};

use super::theme::{self, size};
use crate::session::Outage;

/// How long a drop lasts before the overlay starts to show.
pub const GRACE: Duration = Duration::from_millis(400);
/// The overlay's fade in and out, in seconds.
const FADE: f32 = 0.35;
/// The overlay panel's width, in points.
const OVERLAY_WIDTH: f32 = 320.0;

/// What the header shows.
pub struct HeaderView<'a> {
    pub zone: Option<Zone>,
    pub strands: Option<i64>,
    pub who: &'a str,
    pub connection: Option<&'a ConnectionState>,
    /// The connection dropped and hasn't come back, whichever step of an
    /// attempt `connection` is at.
    pub down: bool,
}

/// The header strip. Draw it before any other panel or window, so theirs
/// are placed below it. Returns whether Sign out was clicked.
pub fn header(ctx: &egui::Context, view: &HeaderView) -> bool {
    let mut sign_out = false;
    let shown = egui::TopBottomPanel::top("capitol_header")
        .frame(
            egui::Frame::none()
                .fill(theme::STRIP)
                .inner_margin(Margin::symmetric(20.0, 7.0)),
        )
        .show_separator_line(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::title("The Capitol", size::LARGE + 3.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    sign_out = theme::link(ui, "Sign out", size::SMALL).clicked();
                    ui.label(
                        RichText::new(view.who)
                            .font(theme::strong(size::BODY))
                            .color(theme::GOLD),
                    );
                    let (colour, meaning) = match view.connection {
                        _ if view.down => (theme::ERROR, "reconnecting…"),
                        Some(ConnectionState::Connected { .. }) => (theme::SUCCESS, "connected"),
                        _ => (theme::DIM, "connecting…"),
                    };
                    theme::status_dot(ui, colour, meaning);
                    ui.add_space(4.0);
                    strands(ui, view.strands);
                    if let Some(zone) = view.zone {
                        zone_chip(ui, zone);
                    }
                });
            });
        });
    // React's gold rule under the header, fading out at both ends.
    let rect = shown.response.rect;
    let painter = ctx.layer_painter(shown.response.layer_id);
    let steps = 32;
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let fade = (1.0 - (2.0 * ((t0 + t1) / 2.0) - 1.0).abs()).powf(0.5);
        painter.hline(
            rect.left() + rect.width() * t0..=rect.left() + rect.width() * t1,
            rect.bottom() - 0.5,
            Stroke::new(1.0, theme::GOLD_DIM.gamma_multiply(fade)),
        );
    }
    sign_out
}

// The chips sit in the header's right-to-left row, and `ui.horizontal`
// keeps that direction, so each adds its parts last first.

/// React's zone indicator: a dot and the name, edged in the zone's colour.
fn zone_chip(ui: &mut egui::Ui, zone: Zone) {
    let colour = theme::zone(zone);
    theme::chip(colour, theme::CHIP).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            ui.label(theme::text(zone.display_name()).size(size::BODY));
            let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 5.0, colour);
        });
    });
}

/// The strand balance, gold on a gold wash. A dash until it's known.
fn strands(ui: &mut egui::Ui, balance: Option<i64>) {
    theme::chip(theme::GOLD_DIM, theme::GOLD_WASH)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let amount = balance.map_or_else(|| "—".to_owned(), grouped);
                ui.label(
                    RichText::new(amount)
                        .font(theme::strong(size::BODY))
                        .color(theme::GOLD),
                );
                ui.label(
                    RichText::new("S")
                        .font(theme::display(size::LARGE))
                        .color(theme::GOLD),
                );
            });
        })
        .response
        .on_hover_text("Strands");
}

/// A whole number with thousands separators, as React's `toLocaleString`.
pub fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out
}

/// The overlay while the connection is down. It keeps the last outage it
/// showed, so it can fade out with something to say.
#[derive(Default)]
pub struct ConnectionOverlay {
    last: Option<Outage>,
}

impl ConnectionOverlay {
    /// `outage` is the session's, `None` while the connection is up.
    /// `connected` tells a fade out on the way back from one at sign-out.
    /// Returns whether Sign out was clicked.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        outage: Option<&Outage>,
        connected: bool,
        now: Instant,
    ) -> bool {
        let showing = outage.is_some_and(|o| now >= o.since + GRACE);
        if showing {
            self.last = outage.cloned();
        }
        let fade = ctx.animate_bool_with_time(Id::new("capitol_outage"), showing, FADE);
        if fade <= 0.0 {
            self.last = None;
            return false;
        }
        let Some(o) = &self.last else {
            return false;
        };
        let back = !showing && connected;

        ctx.layer_painter(egui::LayerId::new(
            Order::Background,
            Id::new("capitol_outage_scrim"),
        ))
        .rect_filled(ctx.screen_rect(), 0.0, theme::SCRIM.gamma_multiply(fade));

        let mut sign_out = false;
        egui::Area::new(Id::new("capitol_outage_panel"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_CENTER, [0.0, -40.0])
            .interactable(showing && fade > 0.5)
            .show(ctx, |ui| {
                ui.multiply_opacity(fade);
                theme::panel_edged(theme::GOLD)
                    .inner_margin(Margin::symmetric(24.0, 20.0))
                    .show(ui, |ui| {
                        ui.set_width(OVERLAY_WIDTH);
                        ui.vertical_centered(|ui| {
                            sign_out = overlay_body(ui, o, back, now);
                        });
                    });
            });
        sign_out
    }
}

fn overlay_body(ui: &mut egui::Ui, o: &Outage, back: bool, now: Instant) -> bool {
    if back {
        ui.label(theme::title("Reconnected", size::HEADING + 6.0));
        ui.add_space(10.0);
        ui.label(theme::dim("The city is with you again."));
        return false;
    }
    ui.add(egui::Spinner::new().size(size::SPINNER).color(theme::GOLD));
    ui.add_space(8.0);
    ui.label(theme::title("Reconnecting", size::HEADING + 6.0));
    ui.add_space(8.0);
    ui.label(theme::dim(plain_reason(&o.reason)).size(size::SMALL))
        .on_hover_text(&o.reason);
    ui.add_space(14.0);

    let wait = o.next_try.saturating_duration_since(now);
    let (line, fraction) = if wait > Duration::from_millis(50) {
        let fraction = 1.0 - wait.as_secs_f32() / o.delay.as_secs_f32().max(0.001);
        (
            format!("Attempt {} in {:.1} s", o.attempt, wait.as_secs_f32()),
            fraction,
        )
    } else {
        (format!("Attempt {}: knocking at the gate…", o.attempt), 1.0)
    };
    ui.label(theme::text(line).size(size::BODY));
    ui.add_space(6.0);
    theme::progress(ui, fraction, theme::GOLD_DIM);
    ui.add_space(14.0);
    ui.label(theme::dim("Your goods are kept on the server.").size(size::SMALL));
    ui.add_space(10.0);
    theme::link(ui, "Sign out", size::SMALL).clicked()
}

/// The net layer's reason in a few plain words. The whole of it is the
/// line's hover.
fn plain_reason(reason: &str) -> String {
    let lower = reason.to_lowercase();
    // The reasons are `capitol_core::net::ws`'s `End::Dropped`s.
    let plain = if lower.starts_with("could not connect") {
        "The server isn't answering."
    } else if lower.contains("stopped responding") || lower.contains("timed out") {
        "The server stopped answering."
    } else if lower.trim().is_empty()
        || lower.starts_with("connection lost")
        || lower.starts_with("connection closed")
        || lower.starts_with("send failed")
    {
        "The connection was lost."
    } else {
        return reason.trim().to_owned();
    };
    plain.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balances_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1234567), "1,234,567");
        assert_eq!(grouped(-45000), "-45,000");
    }

    #[test]
    fn reasons_are_said_plainly() {
        // Both seen live, killing the local backend.
        assert_eq!(
            plain_reason("connection lost: IO error: An existing connection was forcibly closed by the remote host. (os error 10054)"),
            "The connection was lost."
        );
        assert_eq!(
            plain_reason("could not connect to ws://127.0.0.1:3035/ws: No connection could be made because the target machine actively refused it. (os error 10061)"),
            "The server isn't answering."
        );
        assert_eq!(plain_reason(""), "The connection was lost.");
        assert_eq!(plain_reason("something new"), "something new");
    }
}
