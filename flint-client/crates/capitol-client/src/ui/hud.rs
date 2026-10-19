//! The parts of the HUD that are egui (P4.5): the header strip across the
//! top, and the overlay while the connection is down. The zone banner is
//! the script HUD's (`scripts/hud.rhai`), as the plan's world HUD is.
//!
//! - **The header** is React's (`pages/Game.tsx`): the game's name on the
//!   left; on the right the zone, the strand balance, who's signed in with
//!   a connection light, and Sign out. The balance comes from the store,
//!   which the poller keeps fresh (`Fetch::POLLED`). When it changes, the
//!   figure counts to the new one ([`StrandTicker`], P7.1): a gain throws
//!   bright gold motes up out of the chip, a loss lets dim ones fall, and
//!   the difference shows under it and fades. A subscriber has a small
//!   gold Patron chip beside their name (E4.1).
//! - **The overlay** dims the world while a dropped connection is retried,
//!   and says which attempt it is, why the last one ended, and counts down
//!   to the next. A drop shorter than [`GRACE`] shows nothing, so a blip
//!   doesn't flash. On the way back it says so as it fades. A token refused
//!   while away isn't an outage: the login form takes over, with its notice.

use std::time::{Duration, Instant};

use capitol_core::net::ConnectionState;
use capitol_core::protocol::rules::Zone;
use egui::{Align, Align2, Color32, Id, Layout, Margin, Order, RichText, Stroke};

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
    /// A subscriber's Patron chip, with its hover text ("Patron until …").
    /// `None` hides it.
    pub patron: Option<String>,
    pub connection: Option<&'a ConnectionState>,
    /// The connection dropped and hasn't come back, whichever step of an
    /// attempt `connection` is at.
    pub down: bool,
    /// Whether the inventory panel is open, to light its button.
    pub inventory_open: bool,
    /// The same for the crafting panel.
    pub crafting_open: bool,
    /// The same for the exchange panel.
    pub exchange_open: bool,
    /// The same for the market panel.
    pub market_open: bool,
    /// The same for the Gazette (E3.9).
    pub gazette_open: bool,
    /// An edition is out you haven't read: its headline, for the hint.
    pub gazette_news: Option<String>,
    /// The same for the shortcut overlay (P8.3).
    pub help_open: bool,
    /// The same for the settings panel.
    pub settings_open: bool,
    /// How brightly the Inventory button glows, 0 to 1, as a pickup lands
    /// on it (P8.2).
    pub inventory_pulse: f32,
    /// Goods wait in your market box and you're away from the Exchange
    /// hall (E3.7): the hint, its arrow pointing this way on screen
    /// ([`bearing`]). `None` hides it.
    pub hall_hint: Option<f32>,
}

/// What was clicked in the header.
#[derive(Default)]
pub struct HeaderClicks {
    pub sign_out: bool,
    pub inventory: bool,
    pub crafting: bool,
    pub exchange: bool,
    pub market: bool,
    pub gazette: bool,
    /// The hall hint: go to the hall.
    pub hall: bool,
    pub help: bool,
    pub settings: bool,
    /// Where the Inventory button was drawn, for pickups to fly to.
    pub inventory_rect: Option<egui::Rect>,
}

/// The header strip. Draw it before any other panel or window, so theirs
/// are placed below it.
pub fn header(
    ctx: &egui::Context,
    view: &HeaderView,
    ticker: &mut StrandTicker,
    now: Instant,
) -> HeaderClicks {
    ticker.update(view.strands, now);
    let mut clicks = HeaderClicks::default();
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
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    clicks.sign_out = theme::link(ui, "Sign out", size::SMALL).clicked();
                    let ink = if view.settings_open {
                        theme::GOLD
                    } else {
                        theme::GOLD_DIM
                    };
                    clicks.settings = ui
                        .add(
                            egui::Label::new(
                                RichText::new("Settings")
                                    .color(ink)
                                    .size(size::SMALL)
                                    .underline(),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Settings (F10)")
                        .clicked();
                    ui.label(
                        RichText::new(view.who)
                            .font(theme::strong(size::BODY))
                            .color(theme::GOLD),
                    );
                    if let Some(hover) = &view.patron {
                        patron_chip(ui, hover);
                    }
                    let (colour, meaning) = match view.connection {
                        _ if view.down => (theme::ERROR, "reconnecting…"),
                        Some(ConnectionState::Connected { .. }) => (theme::SUCCESS, "connected"),
                        _ => (theme::DIM, "connecting…"),
                    };
                    theme::status_dot(ui, colour, meaning);
                    ui.add_space(4.0);
                    strands(ui, ticker, now);
                    if let Some(zone) = view.zone {
                        zone_chip(ui, zone);
                    }
                    ui.add_space(4.0);
                    if let Some(angle) = view.hall_hint {
                        clicks.hall = hall_chip(ui, angle).clicked();
                    }
                    clicks.gazette = match &view.gazette_news {
                        Some(headline) => news_chip(ui, headline).clicked(),
                        None => theme::toggle_chip(ui, "Gazette", view.gazette_open)
                            .on_hover_text("The Capitol Gazette (G)")
                            .clicked(),
                    };
                    clicks.market = theme::toggle_chip(ui, "Market", view.market_open)
                        .on_hover_text("The market (M)")
                        .clicked();
                    clicks.exchange = theme::toggle_chip(ui, "Exchange", view.exchange_open)
                        .on_hover_text("Fiber exchange (X)")
                        .clicked();
                    let chip = theme::toggle_chip(ui, "Inventory", view.inventory_open)
                        .on_hover_text("Inventory (I)");
                    theme::landed_glow(ui.painter(), chip.rect, 5.0, view.inventory_pulse);
                    clicks.inventory = chip.clicked();
                    clicks.inventory_rect = Some(chip.rect);
                    clicks.crafting = theme::toggle_chip(ui, "Crafting", view.crafting_open)
                        .on_hover_text("Crafting (C)")
                        .clicked();
                    clicks.help = theme::toggle_chip(ui, "?", view.help_open)
                        .on_hover_text("Every control (? or F1)")
                        .clicked();
                    // The name takes what's left, and gives way in a
                    // narrow window rather than sit under the chips
                    let title = theme::title("The Capitol", size::LARGE + 3.0);
                    let wide = egui::WidgetText::from(title.clone())
                        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body)
                        .size()
                        .x;
                    if ui.available_width() >= wide + 16.0 {
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.label(title);
                        });
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
    clicks
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

/// A subscriber's mark: a gold lozenge and "Patron", on the strand chip's
/// wash, so it reads as belonging with the name, not as a button.
fn patron_chip(ui: &mut egui::Ui, hover: &str) {
    theme::chip(theme::GOLD_DIM, theme::GOLD_WASH)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(
                    RichText::new("Patron")
                        .font(theme::strong(size::SMALL))
                        .color(theme::GOLD_LIGHT),
                );
                let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 10.0), egui::Sense::hover());
                let c = rect.center();
                let lozenge = vec![
                    c + egui::vec2(0.0, -4.5),
                    c + egui::vec2(3.5, 0.0),
                    c + egui::vec2(0.0, 4.5),
                    c + egui::vec2(-3.5, 0.0),
                ];
                ui.painter().add(egui::Shape::convex_polygon(
                    lozenge,
                    theme::GOLD,
                    Stroke::new(0.8, theme::GOLD_LIGHT),
                ));
            });
        })
        .response
        .on_hover_text(hover);
}

/// Goods waiting at the Exchange hall (E3.7): a gold chip with an arrow
/// that points to the hall on screen, its edge breathing. A click walks you
/// there.
fn hall_chip(ui: &mut egui::Ui, angle: f32) -> egui::Response {
    let t = ui.input(|i| i.time) as f32;
    let breath = 0.5 + 0.5 * (t * 2.4).sin();
    let shown = theme::chip(lerp(theme::GOLD_DIM, theme::GOLD_LIGHT, breath), theme::GOLD_WASH).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            let c = rect.center();
            let (s, k) = angle.sin_cos();
            let at = |along: f32, across: f32| c + egui::vec2(k * along - s * across, s * along + k * across);
            ui.painter().add(egui::Shape::convex_polygon(
                vec![at(5.5, 0.0), at(-4.0, 4.0), at(-1.5, 0.0), at(-4.0, -4.0)],
                theme::GOLD,
                Stroke::NONE,
            ));
            ui.label(
                RichText::new("Goods at the hall")
                    .font(theme::strong(size::SMALL))
                    .color(theme::GOLD_LIGHT),
            );
        });
    });
    ui.ctx().request_repaint();
    shown
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Your market box holds goods. Collect them at the Exchange hall: click to walk there")
}

/// A new edition of the Gazette is out (E3.9): a gold-edged chip that
/// breathes, with the headline on hover. A click opens it.
fn news_chip(ui: &mut egui::Ui, headline: &str) -> egui::Response {
    let t = ui.input(|i| i.time) as f32;
    let k = 0.5 + 0.5 * (t * 2.2).sin();
    let shown = theme::chip(theme::GOLD.gamma_multiply(0.55 + 0.45 * k), theme::GOLD_WASH)
        .show(ui, |ui| {
            ui.label(
                RichText::new("The Gazette is out")
                    .font(theme::strong(size::SMALL))
                    .color(theme::GOLD_LIGHT),
            );
        });
    ui.ctx().request_repaint();
    shown
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("“{headline}”: read it (G)"))
}

/// Which way `at` lies on screen for a camera at `eye` looking at `look`
/// (world metres), seen from above: radians, 0 to the right and clockwise,
/// as egui's y runs down. Straight ahead is up the screen.
pub fn bearing(eye: [f32; 3], look: [f32; 3], at: [f32; 3]) -> f32 {
    let mut f = [look[0] - eye[0], look[2] - eye[2]];
    let len = f[0].hypot(f[1]);
    if len < 1e-6 {
        // Looking straight down: the screen's up is the camera's -Z.
        f = [0.0, -1.0];
    } else {
        f = [f[0] / len, f[1] / len];
    }
    // Flint's right is forward × up: (-f.z, f.x) on the ground.
    let right = [-f[1], f[0]];
    let d = [at[0] - eye[0], at[2] - eye[2]];
    let x = d[0] * right[0] + d[1] * right[1];
    let ahead = d[0] * f[0] + d[1] * f[1];
    (-ahead).atan2(x)
}

/// The strand balance, gold on a gold wash. A dash until it's known.
fn strands(ui: &mut egui::Ui, ticker: &StrandTicker, now: Instant) {
    let tick = ticker.tick(now);
    let glow = tick.as_ref().map_or(0.0, |t| t.glow);
    let shown = theme::chip(lerp(theme::GOLD_DIM, theme::GOLD, glow), theme::GOLD_WASH)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let font = theme::strong(size::BODY);
                let amount = ticker.shown(now).map_or_else(|| "—".to_owned(), grouped);
                // The chip eases to the new figure's width, rather than
                // jumping, so the chips beside it slide instead of lurch.
                let settled = ticker
                    .target()
                    .map_or(0.0, |to| text_width(ui, &grouped(to), &font));
                let width = ui
                    .ctx()
                    .animate_value_with_time(Id::new("capitol_strand_width"), settled, COUNT)
                    .max(text_width(ui, &amount, &font));
                ui.allocate_ui_with_layout(
                    egui::vec2(width, ui.spacing().interact_size.y),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        ui.set_min_width(width);
                        ui.label(RichText::new(amount).font(font).color(theme::GOLD));
                    },
                );
                ui.label(
                    RichText::new("S")
                        .font(theme::display(size::LARGE))
                        .color(lerp(theme::GOLD, theme::GOLD_LIGHT, glow)),
                );
            });
        })
        .response
        .on_hover_text("Strands");
    if let Some(tick) = tick {
        sparkle(ui.ctx(), shown.rect, &tick);
        ui.ctx().request_repaint();
    }
}

fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font.clone(), theme::GOLD))
        .size()
        .x
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

/// The motes and the difference, in front of the strip.
fn sparkle(ctx: &egui::Context, chip: egui::Rect, tick: &Tick) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        Order::Foreground,
        Id::new("capitol_strand_sparkle"),
    ));
    let gain = tick.to > tick.from;
    let colour = if gain {
        theme::GOLD_LIGHT
    } else {
        theme::GOLD_DIM
    };
    // A gain flies up off the chip's top edge, a loss drops off its bottom,
    // so neither crosses the figure.
    let from = if gain {
        chip.center_top()
    } else {
        chip.center_bottom()
    };
    for mote in &tick.motes {
        let at = from + mote.offset;
        painter.circle_filled(at, mote.radius, colour.gamma_multiply(mote.alpha));
        // The bigger ones glint, four-pointed.
        if mote.radius > 1.6 {
            let arm = mote.radius * 2.6;
            let stroke = Stroke::new(0.8, colour.gamma_multiply(mote.alpha * 0.7));
            painter.hline(at.x - arm..=at.x + arm, at.y, stroke);
            painter.vline(at.x, at.y - arm..=at.y + arm, stroke);
        }
    }
    if tick.label_alpha > 0.0 {
        let diff = tick.to - tick.from;
        let (text, ink) = if gain {
            (format!("+{}", grouped(diff)), theme::GOLD_LIGHT)
        } else {
            (format!("\u{2212}{}", grouped(-diff)), theme::SHORT)
        };
        painter.text(
            egui::pos2(chip.center().x, chip.bottom() + 3.0 + tick.label_drift),
            Align2::CENTER_TOP,
            text,
            theme::strong(size::SMALL),
            ink.gamma_multiply(tick.label_alpha),
        );
    }
}

/// How long the figure takes to count to a new balance, in seconds.
const COUNT: f32 = 0.9;
/// How long the motes and the difference last, in seconds.
const SPARKLE: f32 = 1.4;
/// Motes per change.
const MOTES: u32 = 9;

/// The balance the header shows, counting from the last one to the newest
/// (P7.1). The first balance after sign-in shows as it is, because
/// arriving isn't a change. No balance (signed out) starts it over.
#[derive(Default)]
pub struct StrandTicker {
    /// The balance it's counting to.
    to: Option<i64>,
    /// Where the count started, and when. No `since`: nothing is moving.
    from: i64,
    since: Option<Instant>,
    /// Changes so far, so each scatters its motes differently.
    changes: u32,
}

/// One frame of a change under way.
pub struct Tick {
    pub from: i64,
    pub to: i64,
    /// 1 at the change, fading to 0: the chip's edge and its S brighten.
    pub glow: f32,
    pub motes: Vec<Mote>,
    pub label_alpha: f32,
    /// Points down from under the chip: a gain's difference lifts toward
    /// the balance, a loss's sinks away from it.
    pub label_drift: f32,
}

/// A spark, placed from the chip's centre.
pub struct Mote {
    pub offset: egui::Vec2,
    pub radius: f32,
    pub alpha: f32,
}

impl StrandTicker {
    /// Takes the store's balance. A change counts from wherever the figure
    /// stands now, so a second change mid-count doesn't jump.
    pub fn update(&mut self, balance: Option<i64>, now: Instant) {
        match (self.to, balance) {
            (_, None) => *self = Self::default(),
            (None, Some(b)) => {
                self.to = Some(b);
                self.from = b;
                self.since = None;
            }
            (Some(to), Some(b)) if to != b => {
                self.from = self.shown(now).unwrap_or(to);
                self.to = Some(b);
                self.since = Some(now);
                self.changes = self.changes.wrapping_add(1);
            }
            _ => {}
        }
    }

    /// The balance it's showing or counting to.
    pub fn target(&self) -> Option<i64> {
        self.to
    }

    /// The figure to draw now.
    pub fn shown(&self, now: Instant) -> Option<i64> {
        let to = self.to?;
        let Some(since) = self.since else {
            return Some(to);
        };
        let t = (now.saturating_duration_since(since).as_secs_f32() / COUNT).min(1.0);
        let eased = 1.0 - (1.0 - t as f64).powi(3);
        Some(self.from + ((to - self.from) as f64 * eased).round() as i64)
    }

    /// The change under way, if one is.
    pub fn tick(&self, now: Instant) -> Option<Tick> {
        let (to, since) = (self.to?, self.since?);
        let age = now.saturating_duration_since(since).as_secs_f32();
        if age >= SPARKLE || to == self.from {
            return None;
        }
        let life = age / SPARKLE;
        let gain = to > self.from;
        let motes = (0..MOTES)
            .map(|i| {
                // A fixed scatter per change, not a random one, so captures repeat.
                let h = hash(self.changes, i);
                let byte = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
                // Alternate sides, clear of the middle, where the difference shows.
                let side = if i % 2 == 0 { -1.0 } else { 1.0 };
                let spread = side * (0.45 + byte(0) * 0.55);
                let speed = 0.6 + byte(8) * 0.6;
                let delay = byte(16) * 0.25;
                let t = ((life - delay) / (1.0 - delay)).clamp(0.0, 1.0);
                let travel = 22.0 * speed * (1.0 - (1.0 - t).powi(2));
                let dy = if gain { -travel * 0.55 } else { travel * 0.7 };
                Mote {
                    offset: egui::vec2(spread * (22.0 + 10.0 * t), dy),
                    radius: 1.0 + byte(24) * 1.4,
                    alpha: if t <= 0.0 { 0.0 } else { (1.0 - t).powf(1.5) },
                }
            })
            .collect();
        Some(Tick {
            from: self.from,
            to,
            glow: (1.0 - age / 0.8).max(0.0).powi(2),
            motes,
            label_alpha: (1.0 - life).powf(0.8) * (age / 0.12).min(1.0),
            label_drift: if gain { -6.0 * life } else { 6.0 * life },
        })
    }
}

fn hash(a: u32, b: u32) -> u32 {
    let mut x = a.wrapping_mul(0x9e37_79b9) ^ b.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
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
    fn the_first_balance_shows_as_it_is() {
        let now = Instant::now();
        let mut t = StrandTicker::default();
        assert_eq!(t.shown(now), None);
        t.update(Some(500), now);
        assert_eq!(t.shown(now), Some(500));
        assert!(t.tick(now).is_none());
    }

    #[test]
    fn a_change_counts_to_the_new_balance() {
        let start = Instant::now();
        let mut t = StrandTicker::default();
        t.update(Some(100), start);
        t.update(Some(200), start);
        assert_eq!(t.shown(start), Some(100));
        let mid = t
            .shown(start + Duration::from_secs_f32(COUNT / 2.0))
            .unwrap();
        assert!(mid > 150 && mid < 200, "eases out: {mid}");
        assert_eq!(t.shown(start + Duration::from_secs_f32(COUNT)), Some(200));
        assert!(t.tick(start + Duration::from_millis(100)).is_some());
        assert!(t.tick(start + Duration::from_secs_f32(SPARKLE)).is_none());
    }

    #[test]
    fn a_second_change_counts_on_from_where_the_figure_stands() {
        let start = Instant::now();
        let mut t = StrandTicker::default();
        t.update(Some(1000), start);
        t.update(Some(0), start);
        let later = start + Duration::from_secs_f32(COUNT / 2.0);
        let at = t.shown(later).unwrap();
        t.update(Some(400), later);
        assert_eq!(t.shown(later), Some(at));
        assert_eq!(t.tick(later).unwrap().from, at);
    }

    #[test]
    fn signing_out_starts_over() {
        let now = Instant::now();
        let mut t = StrandTicker::default();
        t.update(Some(100), now);
        t.update(None, now);
        t.update(Some(9000), now);
        assert!(
            t.tick(now).is_none(),
            "another account's balance isn't a change"
        );
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

    #[test]
    fn the_hall_hint_points_where_the_hall_is_on_screen() {
        use std::f32::consts::FRAC_PI_2;
        // Looking north (-Z) from above and behind: ahead is up the screen,
        // east (+X) to the right, south down.
        let (eye, look) = ([0.0, 30.0, 20.0], [0.0, 0.0, 0.0]);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(close(bearing(eye, look, [0.0, 0.0, -50.0]), -FRAC_PI_2));
        assert!(close(bearing(eye, look, [50.0, 0.0, 20.0]), 0.0));
        assert!(close(bearing(eye, look, [0.0, 0.0, 80.0]), FRAC_PI_2));
        // Turned to look east, north is to the left.
        let look_east = [10.0, 0.0, 20.0];
        assert!(close(bearing(eye, look_east, [0.0, 0.0, -50.0]).abs(), std::f32::consts::PI));
    }
}
