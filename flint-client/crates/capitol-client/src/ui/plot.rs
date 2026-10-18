//! The plot panel (P6.4), bottom left as React's `PlotPanel.tsx` has it,
//! or beside the crafting panel while that's open.
//! Alt-click a plot to open it (`input.rhai`, click priority 4). The ×
//! closes it, and so does an alt-click on ground with no plot. Not Esc:
//! Flint's player quits on Esc.
//!
//! It has React's rows: zone, size, type, stations used of capacity,
//! owner, and the assessed value once owned. A plot for sale shows its
//! deed cost with your balance under it, and the Claim button. The deed
//! cost is the plot's `deed_cost` (`capitol_core::plot::Offer`), as React
//! shows it too.
//!
//! A plot of yours that owes tax shows the debt, a bar of how near it is to
//! foreclosure, and a button that pays what your strands cover (#138).
//!
//! The panel is edged in gold, the colour of what's yours (plan §1), and
//! the claimed plot's gold edge in the world answers it.

use super::theme::{self, size, DIM, ENOUGH, GOLD, SHORT, TEXT};
use capitol_core::plot::{words, Offer};
use capitol_core::protocol::rest::PlotInfo;
use capitol_core::protocol::rules::Zone;
use egui::{Align2, Id, RichText};

/// The panel's width inside its frame: React's 280 less the padding.
const WIDTH: f32 = 256.0;
/// How long the panel takes to open or close, in seconds.
const FADE: f32 = 0.18;
/// How far it slides as it opens, in points.
const RISE: f32 = 12.0;

/// What the panel asks for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Claim(uuid::Uuid),
    /// Pay this many strands of the plot's tax debt.
    PayTax(uuid::Uuid, i64),
    Close,
}

/// What the panel draws from.
pub struct View<'a> {
    /// The picked plot, or `None` while nothing is picked.
    pub plot: Option<&'a PlotInfo>,
    pub offer: Offer,
    /// Your balance, once known.
    pub strands: Option<i64>,
    /// A claim is out.
    pub claiming: bool,
    /// A tax payment is out.
    pub paying: bool,
    /// Why the last claim or payment was refused, while it's fresh.
    pub refused: Option<&'a str>,
}

/// What the panel keeps between frames.
#[derive(Default)]
pub struct PlotPanel {
    /// The plot last shown, so it can fade out after the store lets go.
    last: Option<PlotInfo>,
    /// Where the panel was drawn this frame, for the controls hint to keep
    /// clear of. `None` while it's closed.
    pub rect: Option<egui::Rect>,
}

impl PlotPanel {
    /// Draws the panel bottom left, or just right of `beside` (the crafting
    /// panel, which takes the top left) while that's open, so neither
    /// covers the other.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        view: &View,
        beside: Option<egui::Rect>,
    ) -> Option<Action> {
        if let Some(p) = view.plot {
            self.last = Some(p.clone());
        }
        let open = view.plot.is_some();
        let shown = ctx.animate_bool_with_time(Id::new("capitol_plot_fade"), open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            self.last = None;
            return None;
        }
        let plot = self.last.as_ref()?;
        let ease = 1.0 - (1.0 - shown).powi(3);
        let left = beside.map_or(16.0, |r| r.right() + 12.0);
        let area = egui::Area::new(Id::new("capitol_plot"))
            .anchor(Align2::LEFT_BOTTOM, [left, -20.0 + RISE * (1.0 - ease)])
            .interactable(open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                theme::panel_edged(GOLD)
                    .show(ui, |ui| body(ui, plot, view))
                    .inner
            });
        self.rect = open.then_some(area.response.rect);
        area.inner
    }
}

/// The panel's insides.
fn body(ui: &mut egui::Ui, plot: &PlotInfo, view: &View) -> Option<Action> {
    ui.set_width(WIDTH);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(theme::heading("Plot Details").color(GOLD));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui.add(
                egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM)).frame(false),
            );
            if close
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                action = Some(Action::Close);
            }
        });
    });
    ui.add_space(2.0);
    theme::divider(ui);
    ui.add_space(6.0);

    let zone = Zone::from_str(&plot.zone_id);
    let zone_name = zone.map_or_else(|| words(&plot.zone_id), |z| z.display_name().to_owned());
    let zone_ink = zone.map_or(TEXT, theme::zone);
    row(ui, "Zone", RichText::new(zone_name).color(zone_ink));
    row(ui, "Size", theme::text(words(&plot.size_category)));
    row(ui, "Type", theme::text(words(&plot.plot_type)));
    let full = plot.station_count >= plot.station_capacity;
    row(
        ui,
        "Stations",
        theme::text(format!(
            "{} / {}",
            plot.station_count, plot.station_capacity
        ))
        .color(if full { SHORT } else { TEXT }),
    );
    let owner = match &view.offer {
        Offer::Yours => RichText::new("You").color(ENOUGH),
        Offer::Taken { owner } => theme::text(owner.clone()),
        _ => theme::dim("Unclaimed"),
    };
    row(ui, "Owner", owner);
    if plot.owner_id.is_some() {
        row(
            ui,
            "Value",
            theme::text(format!("{} Strands", plot.assessed_value)),
        );
    }

    match &view.offer {
        Offer::Yours if plot.tax_owed > 0 => {
            section(ui);
            if let Some(a) = debt(ui, plot, view) {
                action = Some(a);
            }
        }
        Offer::Yours => {
            section(ui);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("You own this plot. Place stations here!")
                        .color(ENOUGH)
                        .size(size::BODY),
                );
            });
        }
        Offer::Taken { .. } => {}
        offer => {
            section(ui);
            if let Some(a) = claim(ui, plot, offer, view) {
                action = Some(a);
            }
        }
    }
    if let Some(why) = view.refused {
        ui.add_space(6.0);
        theme::error_box(ui, why);
    }
    action
}

/// The deed cost, your balance under it, and the Claim button.
fn claim(ui: &mut egui::Ui, plot: &PlotInfo, offer: &Offer, view: &View) -> Option<Action> {
    let cost = match offer {
        Offer::Claimable { cost } | Offer::TooDear { cost, .. } => Some(*cost),
        _ => None,
    };
    ui.vertical_centered(|ui| {
        let price = match cost {
            Some(c) => format!("{c} Strands"),
            None => "N/A".into(),
        };
        let mut job = egui::text::LayoutJob::default();
        let format = |font, color| egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        };
        job.append("Deed Cost: ", 0.0, format(theme::body(size::LARGE), DIM));
        job.append(&price, 0.0, format(theme::strong(size::LARGE), GOLD));
        ui.label(job);
        if let Some(held) = view.strands {
            let ink = if matches!(offer, Offer::TooDear { .. }) {
                SHORT
            } else {
                DIM
            };
            ui.label(
                RichText::new(format!("You hold {held} Strands"))
                    .color(ink)
                    .size(size::LABEL),
            );
        }
    });
    ui.add_space(8.0);
    let (text, enabled) = match offer {
        _ if view.claiming => ("Claiming…".to_owned(), false),
        Offer::Claimable { .. } => ("Claim Plot".to_owned(), true),
        Offer::TooDear { short, .. } => (format!("{short} Strands short"), false),
        Offer::NotForSale => ("Not for sale".to_owned(), false),
        _ => ("Claim Plot".to_owned(), false),
    };
    theme::accent_button(ui, &text, ENOUGH, enabled)
        .on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::NotAllowed
        })
        .clicked()
        .then_some(Action::Claim(plot.id))
}

/// The tax a plot of yours owes, how near that is to foreclosure, and the
/// button that pays it off as far as your strands reach.
fn debt(ui: &mut egui::Ui, plot: &PlotInfo, view: &View) -> Option<Action> {
    row(
        ui,
        "Tax owed",
        RichText::new(format!("{} Strands", plot.tax_owed)).color(SHORT),
    );
    if let Some(limit) = plot.forecloses_at.filter(|l| *l > 0) {
        let near = (plot.tax_owed as f32 / limit as f32).clamp(0.0, 1.0);
        ui.add_space(4.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(WIDTH, 4.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 2.0, theme::WELL);
        let mut filled = rect;
        filled.set_width(rect.width() * near);
        // Gold while there's time, red as the end nears
        painter.rect_filled(filled, 2.0, GOLD.lerp_to_gamma(SHORT, near));
        ui.add_space(3.0);
        ui.label(theme::dim(format!("Foreclosed at {limit} owed")).size(size::LABEL));
    }
    ui.add_space(8.0);
    let pay = view.strands.unwrap_or(0).clamp(0, plot.tax_owed);
    let (text, enabled) = match pay {
        _ if view.paying => ("Paying…".to_owned(), false),
        0 => ("No Strands to pay with".to_owned(), false),
        n if n == plot.tax_owed => (format!("Pay {n} Strands"), true),
        n => (format!("Pay {n} of it"), true),
    };
    theme::accent_button(ui, &text, GOLD, enabled)
        .on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::NotAllowed
        })
        .clicked()
        .then_some(Action::PayTax(plot.id, pay))
        .filter(|_| enabled)
}

/// One label and value, spread across the panel (React's `.plot-info-row`).
fn row(ui: &mut egui::Ui, label: &str, value: RichText) {
    ui.horizontal(|ui| {
        ui.label(theme::dim(label).size(size::BODY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value.size(size::BODY));
        });
    });
}

/// The rule above the claim or ownership section.
fn section(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    theme::divider(ui);
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::protocol::rest::Bounds;
    use uuid::Uuid;

    fn plot(owner: Option<Uuid>) -> PlotInfo {
        PlotInfo {
            id: Uuid::from_u128(9),
            zone_id: "suburban".into(),
            world_x: 0.0,
            world_y: 0.0,
            bounds: Bounds::new(0.0, 0.0, 10.0, 10.0),
            size_category: "large".into(),
            plot_type: "standard".into(),
            owner_id: owner,
            owner_name: owner.map(|_| "Ada".into()),
            claimed_at: None,
            assessed_value: 500,
            tax_owed: 0,
            forecloses_at: None,
            deed_cost: Some(500),
            station_count: 1,
            station_capacity: 9,
            terrain_id: None,
            corners: None,
        }
    }

    /// Runs a few frames of the panel and returns the last frame's action
    /// and the panel's rect.
    fn run(view: &View, click_at: Option<egui::Pos2>) -> (Option<Action>, Option<egui::Rect>) {
        let ctx = theme::test_context();
        let mut panel = PlotPanel::default();
        let mut out = (None, None);
        for frame in 0..8 {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 720.0),
                )),
                ..Default::default()
            };
            if let (Some(p), 5) = (click_at, frame) {
                raw.events.push(egui::Event::PointerMoved(p));
                raw.events.push(egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                });
                raw.events.push(egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                });
            }
            let _ = ctx.run(raw, |ctx| {
                theme::install(ctx);
                let a = panel.show(ctx, view, None);
                if a.is_some() {
                    out.0 = a;
                }
                out.1 = panel.rect;
            });
        }
        out
    }

    #[test]
    fn the_panel_sits_bottom_left_and_claims_with_the_button() {
        let p = plot(None);
        let view = View {
            plot: Some(&p),
            offer: Offer::Claimable { cost: 500 },
            strands: Some(800),
            claiming: false,
            paying: false,
            refused: None,
        };
        let (_, rect) = run(&view, None);
        let rect = rect.expect("drawn");
        assert!((rect.left() - 16.0).abs() < 1.0, "{rect:?}");
        assert!((720.0 - rect.bottom() - 20.0).abs() < 1.0, "{rect:?}");
        // The button spans the bottom of the panel, just above its margin.
        let button = egui::pos2(rect.center().x, rect.bottom() - 30.0);
        let (action, _) = run(&view, Some(button));
        assert_eq!(action, Some(Action::Claim(p.id)));
    }

    #[test]
    fn a_plot_you_cant_afford_doesnt_claim() {
        let p = plot(None);
        let view = View {
            plot: Some(&p),
            offer: Offer::TooDear {
                cost: 500,
                short: 100,
            },
            strands: Some(400),
            claiming: false,
            paying: false,
            refused: None,
        };
        let (_, rect) = run(&view, None);
        let rect = rect.unwrap();
        let (action, _) = run(
            &view,
            Some(egui::pos2(rect.center().x, rect.bottom() - 30.0)),
        );
        assert_eq!(action, None);
    }

    #[test]
    fn a_plot_that_owes_tax_pays_what_your_strands_cover() {
        let me = Uuid::from_u128(1);
        let mut p = plot(Some(me));
        p.tax_owed = 120;
        p.forecloses_at = Some(480);
        let view = View {
            plot: Some(&p),
            offer: Offer::Yours,
            strands: Some(90),
            claiming: false,
            paying: false,
            refused: None,
        };
        let (_, rect) = run(&view, None);
        let rect = rect.unwrap();
        let button = egui::pos2(rect.center().x, rect.bottom() - 30.0);
        assert_eq!(run(&view, Some(button)).0, Some(Action::PayTax(p.id, 90)));

        let broke = View {
            strands: Some(0),
            ..view
        };
        assert_eq!(run(&broke, Some(button)).0, None, "nothing to pay with");
    }

    #[test]
    fn nothing_picked_draws_nothing() {
        let view = View {
            plot: None,
            offer: Offer::NotForSale,
            strands: None,
            claiming: false,
            paying: false,
            refused: None,
        };
        assert_eq!(run(&view, None), (None, None));
    }
}
