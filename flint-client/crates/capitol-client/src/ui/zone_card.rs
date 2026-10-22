//! The zone card (#444, the road in plan D9): what living in each home zone
//! is like, honestly, so the choice of a home is an informed one.
//!
//! It opens from the plot panel ("About the Urban zone") on one zone, and as
//! a three-column comparison, the city out, for the Deeds Office (#448).
//! One zone's card offers the comparison too. Each column carries its zone's
//! colour, its play style in a line, and a plan of a typical lot with its
//! stations as gold dots, every plan at one scale, so a farm's room and a
//! yard's read at a glance. Under it the live deed price and a week's tax,
//! the stations a lot fits (`capitol_core::zone_card` owns up to fewer than
//! a workshop or the deed's capacity), the free lots, the walk to the hall
//! and what grows there.
//!
//! The facts are `/bureaucracy/zone-facts`, asked each time it opens. It's
//! in the bureaucracy's voice (plan D1): plain, a little dry, never a
//! character. Opening it is the road in's client-reported step
//! (`Did::ReadZoneCard`), which the extension reports as `tutorial_seen`
//! where it opens the card (#441).

use std::collections::HashMap;

use capitol_core::protocol::rest::{ZoneFacts, ZoneGrowth};
use capitol_core::protocol::rules::Zone;
use capitol_core::sim::lot;
use capitol_core::zone_card as words;
use egui::{Align2, Color32, Id, Pos2, Rect, RichText, Sense, Stroke};

use super::theme::{self, size, DIM, GOLD, TEXT};

/// The open and close fade, in seconds, and how far it rises as it comes.
const FADE: f32 = 0.22;
const RISE: f32 = 12.0;
/// One column's width inside the panel, and the gap between columns.
const COLUMN: f32 = 216.0;
const GAP: f32 = 14.0;
/// Room at the right for the scroll bar, so it never sits on a column.
const GUTTER: f32 = 10.0;
/// The plan's height; its width is the column's.
const PLAN: f32 = 92.0;
/// The room kept for the style line, two lines of it, so the columns'
/// rows stay level.
const STYLE: f32 = 34.0;
/// The length of the plan's scale bar, in world units (metres).
const SCALE_BAR: f64 = 50.0;

/// A typical lot's corners, and where its stations stand on it.
type Dots = ([(f64, f64); 4], Vec<(f64, f64)>);

/// What the card shows: one zone, or all three side by side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    One(String),
    Compare,
}

/// What the reader did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Close,
}

/// The card and what it keeps between frames.
#[derive(Default)]
pub struct ZoneCard {
    shown: Option<Shown>,
    /// What was shown last, so it can fade out after it closes
    last: Option<Shown>,
    /// Each zone's stations on its typical lot, packed once per lot
    dots: HashMap<String, Dots>,
    /// Where it was drawn this frame, `None` while it's closed
    pub rect: Option<Rect>,
}

impl ZoneCard {
    /// Opens it on one zone. The caller asks for the facts.
    pub fn open_one(&mut self, zone: &str) {
        self.open(Shown::One(zone.to_owned()));
    }

    /// Opens it on the three home zones side by side.
    pub fn open_compare(&mut self) {
        self.open(Shown::Compare);
    }

    fn open(&mut self, shown: Shown) {
        self.shown = Some(shown);
    }

    pub fn close(&mut self) {
        self.shown = None;
    }

    pub fn is_open(&self) -> bool {
        self.shown.is_some()
    }

    /// Draws it in the middle of the screen. `facts` is `None` until the
    /// server has answered.
    pub fn show(&mut self, ctx: &egui::Context, facts: Option<&[ZoneFacts]>) -> Option<Action> {
        if self.shown.is_some() {
            self.last = self.shown.clone();
        }
        let open = self.shown.is_some();
        let shown = ctx.animate_bool_with_time(Id::new("capitol_zone_card_fade"), open, FADE);
        self.rect = None;
        if shown <= 0.0 {
            self.last = None;
            return None;
        }
        let which = self.last.clone()?;
        let ease = 1.0 - (1.0 - shown).powi(3);
        let screen = ctx.available_rect();
        let mut action = None;
        let area = egui::Area::new(Id::new("capitol_zone_card"))
            .order(egui::Order::Middle)
            .anchor(Align2::CENTER_CENTER, [0.0, RISE * (1.0 - ease)])
            .interactable(open)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                let edge = match &which {
                    Shown::One(zone) => Zone::from_str(zone).map_or(GOLD, theme::zone),
                    Shown::Compare => GOLD,
                };
                theme::panel_edged(edge).show(ui, |ui| {
                    action = self.body(ui, &which, facts, screen.height() - 48.0);
                });
            });
        self.rect = open.then_some(area.response.rect);
        if action == Some(Action::Close) {
            self.close();
        }
        action
    }

    fn body(&mut self, ui: &mut egui::Ui, which: &Shown, facts: Option<&[ZoneFacts]>, room: f32) -> Option<Action> {
        let columns: Vec<&ZoneFacts> = match (which, facts) {
            (Shown::One(zone), Some(all)) => words::find(all, zone).into_iter().collect(),
            (Shown::Compare, Some(all)) => all.iter().collect(),
            (_, None) => Vec::new(),
        };
        let count = match which {
            Shown::One(_) => 1,
            Shown::Compare => 3,
        };
        let width = COLUMN * count as f32 + GAP * (count - 1) as f32 + GUTTER;
        ui.set_width(width);
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(theme::label("The Land Office").color(theme::GOLD_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.add(egui::Button::new(RichText::new("×").size(size::HEADING).color(DIM)).frame(false));
                if close.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    action = Some(Action::Close);
                }
            });
        });
        let lede = match which {
            Shown::One(_) => "What living here is like, as the records have it.",
            Shown::Compare => "The three zones a home may be had in, the city out.",
        };
        ui.label(theme::dim(lede));
        ui.add_space(4.0);
        theme::divider(ui);
        ui.add_space(8.0);

        if columns.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label(theme::dim(if facts.is_some() { "No records for this zone." } else { "Fetching the records…" })));
            ui.add_space(24.0);
            return action;
        }
        // Every plan at one scale: the biggest lot fills its box.
        let scale = plan_scale(facts.unwrap_or_default());
        let foot = 26.0;
        theme::scrolled(ui, "capitol_zone_card_scroll", (room - 90.0 - foot).max(160.0), |ui| {
            // The full width, so the scroll bar stands in the gutter
            ui.set_min_width(width);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for f in &columns {
                    ui.vertical(|ui| {
                        ui.set_width(COLUMN);
                        let dots = self.dots_for(f);
                        column(ui, f, dots, scale);
                    });
                }
            });
        });
        ui.add_space(6.0);
        theme::divider(ui);
        ui.add_space(4.0);
        let rate = columns.first().map(|f| f.tax_per_week as f64 / f.deed_cost.max(1) as f64 * 100.0).unwrap_or(0.0);
        ui.add(egui::Label::new(theme::dim(format!("Prices are today's. Tax is {rate:.0}% of the deed a week.")).size(size::LABEL)).wrap());
        if let Shown::One(_) = which {
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                if theme::link(ui, "Compare the zones", size::LABEL).clicked() {
                    self.shown = Some(Shown::Compare);
                }
            });
        }
        action
    }

    /// The stations on `f`'s typical lot, as `sim::lot` places them.
    fn dots_for(&mut self, f: &ZoneFacts) -> &[(f64, f64)] {
        let want = f.stations_median.max(0) as usize;
        let stale = self
            .dots
            .get(&f.zone_id)
            .is_none_or(|(corners, dots)| *corners != f.typical_lot || dots.len() > want);
        if stale {
            let dots = lot::layout(f.typical_lot, want);
            self.dots.insert(f.zone_id.clone(), (f.typical_lot, dots));
        }
        &self.dots[&f.zone_id].1
    }
}

/// Points of the plan per world unit, so the biggest typical lot of `all`,
/// laid long side across, fills the box.
fn plan_scale(all: &[ZoneFacts]) -> f32 {
    let (mut w, mut h) = (1.0f64, 1.0f64);
    for f in all {
        let (lw, lh) = extent(&laid(&f.typical_lot, &f.typical_lot));
        w = w.max(lw);
        h = h.max(lh);
    }
    ((COLUMN - 24.0) as f64 / w).min((PLAN - 30.0) as f64 / h) as f32
}

/// `points`, turned so `corners`' longest side runs across.
fn laid(corners: &[(f64, f64); 4], points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let side = |i: usize| {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        (b.0 - a.0, b.1 - a.1)
    };
    let longest = (0..4).map(side).max_by(|a, b| a.0.hypot(a.1).total_cmp(&b.0.hypot(b.1))).unwrap_or((1.0, 0.0));
    let angle = longest.1.atan2(longest.0);
    let (sin, cos) = (-angle).sin_cos();
    points.iter().map(|&(x, y)| (x * cos - y * sin, x * sin + y * cos)).collect()
}

fn extent(points: &[(f64, f64)]) -> (f64, f64) {
    let xs = points.iter().map(|p| p.0);
    let ys = points.iter().map(|p| p.1);
    let w = xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min);
    let h = ys.clone().fold(f64::MIN, f64::max) - ys.fold(f64::MAX, f64::min);
    (w, h)
}

/// One zone's column: the band, the plan, the facts and what it owns up to.
fn column(ui: &mut egui::Ui, f: &ZoneFacts, dots: &[(f64, f64)], scale: f32) {
    let ink = Zone::from_str(&f.zone_id).map_or(TEXT, theme::zone);
    let (band, _) = ui.allocate_exact_size(egui::vec2(COLUMN, 3.0), Sense::hover());
    ui.painter().rect_filled(band, 1.5, ink);
    ui.add_space(4.0);
    ui.label(RichText::new(&f.zone_name).font(theme::strong(size::LARGE)).color(ink));
    let style = ui.add(egui::Label::new(RichText::new(&f.style).italics().color(DIM).size(size::SMALL)).wrap());
    ui.add_space((STYLE - style.rect.height()).max(0.0) + 6.0);
    plan(ui, f, dots, scale, ink);
    ui.add_space(6.0);
    theme::list_spacing(ui);
    row(ui, "Deed", RichText::new(format!("{} Strands", f.deed_cost)).color(GOLD));
    row(ui, "Tax", theme::text(format!("{} a week", f.tax_per_week)));
    row(ui, "Stations", theme::text(words::stations(f)));
    row(ui, "Lot", theme::text(words::area(f)));
    row(ui, "Free", theme::text(words::free(f)));
    row(ui, "To the hall", theme::text(words::walk_range(f)));
    grows(ui, &f.grows);
    let notes = words::room_notes(f);
    if !notes.is_empty() {
        ui.add_space(4.0);
        for note in notes {
            ui.add(egui::Label::new(RichText::new(note).color(theme::EMBER_HOT.gamma_multiply(0.85)).size(size::LABEL)).wrap());
        }
    }
}

/// The typical lot, long side across, its stations as gold dots, and a
/// scale bar, at the card's one scale.
fn plan(ui: &mut egui::Ui, f: &ZoneFacts, dots: &[(f64, f64)], scale: f32, ink: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(COLUMN, PLAN), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, theme::WELL);
    let lot = laid(&f.typical_lot, &f.typical_lot);
    let stations = laid(&f.typical_lot, dots);
    let (cx, cy) = {
        let n = lot.len() as f64;
        (lot.iter().map(|p| p.0).sum::<f64>() / n, lot.iter().map(|p| p.1).sum::<f64>() / n)
    };
    let middle = Pos2::new(rect.center().x, rect.top() + (PLAN - 16.0) / 2.0);
    let to_screen = |(x, y): (f64, f64)| middle + egui::vec2((x - cx) as f32 * scale, (y - cy) as f32 * scale);
    let outline: Vec<Pos2> = lot.iter().map(|&p| to_screen(p)).collect();
    painter.add(egui::Shape::convex_polygon(outline.clone(), ink.gamma_multiply(0.14), Stroke::NONE));
    painter.add(egui::Shape::closed_line(outline, Stroke::new(1.25_f32, ink.gamma_multiply(0.8))));
    // A station's reach, faint, then the station
    let reach = (lot::SPOT_SPACING as f32 / 2.0 * scale).max(2.0);
    for &p in &stations {
        let at = to_screen(p);
        painter.circle_filled(at, reach, theme::GOLD.gamma_multiply(0.10));
        painter.circle_filled(at, 2.6, GOLD);
    }
    // The scale bar, bottom left
    let bar = SCALE_BAR as f32 * scale;
    let y = rect.bottom() - 8.0;
    let x0 = rect.left() + 8.0;
    let tick = Stroke::new(1.0_f32, DIM);
    painter.line_segment([Pos2::new(x0, y), Pos2::new(x0 + bar, y)], tick);
    for x in [x0, x0 + bar] {
        painter.line_segment([Pos2::new(x, y - 3.0), Pos2::new(x, y + 1.0)], tick);
    }
    painter.text(Pos2::new(x0 + bar + 4.0, y), Align2::LEFT_CENTER, format!("{SCALE_BAR:.0} m"), theme::body(size::TINY), DIM);
    painter.text(
        Pos2::new(rect.right() - 8.0, y),
        Align2::RIGHT_CENTER,
        format!("{} stations", f.stations_median),
        theme::body(size::TINY),
        GOLD.gamma_multiply(0.8),
    );
}

/// What grows there, under its label, wrapped.
fn grows(ui: &mut egui::Ui, grows: &[ZoneGrowth]) {
    ui.horizontal_top(|ui| {
        ui.label(theme::dim("Grows").size(size::BODY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let names: Vec<&str> = grows.iter().take(words::GROWS_SHOWN).map(|g| g.name.as_str()).collect();
            let text = if names.is_empty() { "Nothing wild".to_owned() } else { names.join("\n") };
            ui.label(theme::text(text).size(size::BODY));
        });
    });
}

/// One label and value, spread across the column.
fn row(ui: &mut egui::Ui, label: &str, value: RichText) {
    ui.horizontal(|ui| {
        ui.label(theme::dim(label).size(size::BODY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value.size(size::BODY));
        });
    });
}

/// The three zones as the default world has them (Plots v2, #459), for
/// `--zone-card-sample` and the tests.
pub fn sample() -> Vec<ZoneFacts> {
    let grow = |id: &str, name: &str, density: f64| ZoneGrowth { resource_id: id.into(), name: name.into(), density };
    let quad = |w: f64, h: f64, skew: f64| [(-w / 2.0, -h / 2.0), (w / 2.0, -h / 2.0), (w / 2.0 + skew, h / 2.0), (-w / 2.0 + skew, h / 2.0)];
    vec![
        ZoneFacts {
            zone_id: "urban".into(),
            zone_name: "Urban".into(),
            style: "A workshop yard a step from the market.".into(),
            deed_cost: 1000,
            tax_per_week: 20,
            stations_median: 4,
            stations_least: 4,
            station_capacity: 16,
            lots_free: 1015,
            lots_total: 1016,
            walk_nearest_secs: 17.0,
            walk_median_secs: 34.0,
            grows: vec![grow("clay_bank", "Clay Bank", 6.0)],
            typical_lot: quad(86.0, 100.0, 0.0),
            typical_area: 8600.0,
        },
        ZoneFacts {
            zone_id: "suburban".into(),
            zone_name: "Suburban".into(),
            style: "A house and a garden on a quiet street, with room for a trade.".into(),
            deed_cost: 500,
            tax_per_week: 10,
            stations_median: 8,
            stations_least: 3,
            station_capacity: 25,
            lots_free: 977,
            lots_total: 977,
            walk_nearest_secs: 39.0,
            walk_median_secs: 59.0,
            grows: vec![grow("grass", "Wild Grass", 5.0), grow("fallen_branches", "Fallen Branches", 4.0), grow("clay_bank", "Clay Bank", 2.0)],
            typical_lot: quad(139.0, 134.0, 6.0),
            typical_area: 18_600.0,
        },
        ZoneFacts {
            zone_id: "rural".into(),
            zone_name: "Rural".into(),
            style: "A farm on a lane, with room to spread out.".into(),
            deed_cost: 250,
            tax_per_week: 5,
            stations_median: 15,
            stations_least: 12,
            station_capacity: 25,
            lots_free: 901,
            lots_total: 901,
            walk_nearest_secs: 68.0,
            walk_median_secs: 98.0,
            grows: vec![
                grow("tree", "Tree", 25.0),
                grow("fallen_branches", "Fallen Branches", 15.0),
                grow("rock_outcrop", "Rock Outcrop", 6.0),
                grow("grass", "Wild Grass", 2.5),
            ],
            typical_lot: quad(130.0, 268.0, 0.0),
            typical_area: 34_900.0,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(card: &mut ZoneCard, facts: Option<&[ZoneFacts]>, size: egui::Vec2, click: Option<Pos2>) -> (Option<Action>, Option<Rect>) {
        let ctx = theme::test_context();
        let mut out = (None, None);
        for frame in 0..10 {
            let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
            if let (Some(p), 7) = (click, frame) {
                raw.events.push(egui::Event::PointerMoved(p));
                for pressed in [true, false] {
                    raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
                }
            }
            let _ = ctx.run(raw, |ctx| {
                theme::install(ctx);
                if let Some(a) = card.show(ctx, facts) {
                    out.0 = Some(a);
                }
                out.1 = card.rect;
            });
        }
        out
    }

    #[test]
    fn the_comparison_fits_the_smallest_screen() {
        let facts = sample();
        let mut card = ZoneCard::default();
        card.open_compare();
        // 1280 × 720 at 150%
        let screen = egui::vec2(853.0, 480.0);
        let (_, rect) = run(&mut card, Some(&facts), screen, None);
        let rect = rect.expect("drawn");
        assert!(rect.left() >= 0.0 && rect.right() <= screen.x, "{rect:?}");
        assert!(rect.top() >= 0.0 && rect.bottom() <= screen.y, "{rect:?}");
    }

    #[test]
    fn one_zone_closes_with_its_cross() {
        let facts = sample();
        let mut card = ZoneCard::default();
        card.open_one("rural");
        let screen = egui::vec2(1280.0, 720.0);
        let (_, rect) = run(&mut card, Some(&facts), screen, None);
        let rect = rect.expect("drawn");
        assert!(rect.width() < COLUMN + GUTTER + 60.0 && rect.height() < 600.0, "one column: {rect:?}");
        let cross = Pos2::new(rect.right() - 16.0, rect.top() + 20.0);
        let (action, _) = run(&mut card, Some(&facts), screen, Some(cross));
        assert_eq!(action, Some(Action::Close));
        assert!(!card.is_open());
    }

    #[test]
    fn every_plan_shares_one_scale_and_fits_its_box() {
        let facts = sample();
        let scale = plan_scale(&facts);
        for f in &facts {
            let (w, h) = extent(&laid(&f.typical_lot, &f.typical_lot));
            assert!(w as f32 * scale <= COLUMN - 23.9 && h as f32 * scale <= PLAN - 29.9, "{}", f.zone_id);
            assert!(w >= h, "{} is laid long side across", f.zone_id);
        }
    }

    #[test]
    fn the_dots_are_the_stations_a_typical_lot_fits() {
        let mut card = ZoneCard::default();
        for f in sample() {
            let dots = card.dots_for(&f).len();
            assert_eq!(dots as i32, f.stations_median, "{}", f.zone_id);
        }
    }
}
