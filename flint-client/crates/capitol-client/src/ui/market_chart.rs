//! The market panel's chart (E3.6): the selected item's price over time,
//! with its volume under it and the latest trades beside it.
//!
//! - **Price** as candles (open, high, low, close a bucket: green where it
//!   closed up, red where down) or as a line through the closes. An hour a
//!   candle over the last week, or a day a candle over four months, of every
//!   grade or of one: a grade's chart is trades whose units averaged it.
//! - **Your fills** are gold, the market panel's colour for what's yours: a
//!   triangle up where you bought, down where you sold, at the price and the
//!   moment of the fill.
//! - **Hover** a bucket for its numbers in the line over the chart.
//! - **The tape**: the latest trades, newest first, your own in gold. Click
//!   one to take its price to the order form.
//!
//! The chart is `egui_plot`'s, styled from the theme. It stays still: no
//! drag or zoom, so the price and volume plots always line up and the panel
//! keeps the wheel.

use super::pad_nav::FollowFocus;
use capitol_core::market::{Bucket, History};
use capitol_core::protocol::rest::{Candle, MarketTrade};
use capitol_core::protocol::rules::market::OrderSide;
use chrono::{DateTime, Datelike, Local, TimeZone, Timelike, Utc};
use egui::{Align2, Id, RichText, Sense, Stroke};
use egui_plot::{
    Bar, BarChart, BoxElem, BoxPlot, BoxSpread, GridInput, GridMark, HLine, Line, LineStyle,
    MarkerShape, Plot, PlotBounds, PlotPoints, Points, VLine,
};
use uuid::Uuid;

use super::hud::grouped;
use super::theme::{self, size, DIM, ENOUGH, GOLD, GOLD_DIM, GOLD_WASH, INK_SHADOW, SHORT, TEXT};

/// The plots' heights: the price, then the volume with the time axis.
const PRICE_HEIGHT: f32 = 150.0;
const VOLUME_HEIGHT: f32 = 50.0;
/// The price axis, on the right as a ticker's is; the volume plot keeps
/// the same width there so the two line up.
const AXIS_WIDTH: f32 = 30.0;
/// How much of a bucket a candle or a volume bar fills.
const BODY: f64 = 0.7;
/// The fewest buckets a chart spans, so a new item's first trades don't
/// fill it edge to edge.
const MIN_BUCKETS: i64 = 24;
/// The tape's rows, and where its columns sit: price and quantity by their
/// right edges, the grade by its left.
const TAPE_ROW: f32 = 17.0;
const PRICE_AT: f32 = 72.0;
const QTY_AT: f32 = 100.0;
const GRADE_AT: f32 = 110.0;
/// How far apart time labels must be, in points.
const LABEL_SPACING: f32 = 58.0;

/// How the price is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    #[default]
    Candles,
    Line,
}

/// What the chart shows. The panel keeps it between items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settings {
    pub bucket: Bucket,
    /// One grade, or every grade
    pub grade: Option<&'static str>,
    pub style: Style,
}

/// The grade filter's choices.
const GRADES: [(Option<&str>, &str); 6] = [
    (None, "All"),
    (Some("A"), "A"),
    (Some("B"), "B"),
    (Some("C"), "C"),
    (Some("D"), "D"),
    (Some("F"), "F"),
];

/// The bucket, the grade and the style, in a row.
pub fn controls(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.scope(|ui| {
            theme::segmented(
                ui,
                &mut settings.bucket,
                &[(Bucket::Hour, "1h"), (Bucket::Day, "1d")],
            )
            .on_hover_text("An hour a candle over the week, or a day a candle over four months");
        });
        ui.scope(|ui| {
            theme::segmented(ui, &mut settings.grade, &GRADES)
                .on_hover_text("Only trades whose units averaged this grade");
        });
        ui.scope(|ui| {
            theme::segmented(
                ui,
                &mut settings.style,
                &[(Style::Candles, "Candles"), (Style::Line, "Line")],
            );
        });
    });
}

/// What the chart keeps between frames.
#[derive(Debug, Default)]
pub struct ChartState {
    /// The time under the pointer, over either plot (seconds since the epoch)
    pub hover: Option<f64>,
    /// The tape's rows as drawn this frame, for the tests to click
    pub tape_rows: Vec<(Uuid, egui::Rect)>,
}

/// The time span a chart of `history` covers, in seconds since the epoch:
/// from its first candle, or [`MIN_BUCKETS`] back, to the bucket after now.
fn span(history: &History, now: DateTime<Utc>) -> (f64, f64) {
    let width = history.key.bucket.secs();
    let now = now.timestamp();
    let end = now - now.rem_euclid(width) + width;
    let first = history
        .candles
        .first()
        .map_or(end, |c| c.at.timestamp().min(end));
    let start = first.min(end - MIN_BUCKETS * width) - width / 2;
    (start as f64, (end + width / 2) as f64)
}

/// The price range shown: the candles' lows to highs, and your fills, with
/// room above and below.
fn price_range(history: &History) -> (f64, f64) {
    let lows = history.candles.iter().map(|c| c.low);
    let highs = history.candles.iter().map(|c| c.high);
    let fills = history.mine.iter().map(|f| f.trade.price);
    let low = lows.chain(fills.clone()).min().unwrap_or(0) as f64;
    let high = highs.chain(fills).max().unwrap_or(1) as f64;
    let pad = ((high - low) * 0.12).max(1.0);
    ((low - pad).max(0.0), high + pad)
}

/// The middle of `candle`'s bucket.
fn centre(candle: &Candle, bucket: Bucket) -> f64 {
    (candle.at.timestamp() + bucket.secs() / 2) as f64
}

fn rising(candle: &Candle) -> bool {
    candle.close >= candle.open
}

/// The price plot over the volume plot. `now` places the right edge.
pub fn plots(
    ui: &mut egui::Ui,
    state: &mut ChartState,
    history: &History,
    settings: &Settings,
    now: DateTime<Utc>,
) {
    let bucket = history.key.bucket;
    let (x0, x1) = span(history, now);
    let (y0, y1) = price_range(history);
    let width = ui.available_width();
    let secs_per_point = (x1 - x0) / (width - AXIS_WIDTH).max(1.0) as f64;
    let hovered = state.hover.and_then(|x| history.candle_at(x)).cloned();
    readout(ui, history, hovered.as_ref());
    // The grid and the axes' labels take the text colour: dim them
    ui.visuals_mut().override_text_color = Some(DIM);

    let mut hover = None;
    let price_step = price_step((y1 - y0) / PRICE_HEIGHT as f64);
    let price = base_plot("capitol_market_price", width, PRICE_HEIGHT, secs_per_point)
        .show_axes([false, true])
        .y_grid_spacer(move |input| price_marks(input, price_step))
        .y_axis_formatter(move |mark, _| {
            if mark.step_size < price_step as f64 {
                String::new()
            } else {
                grouped(mark.value as i64)
            }
        })
        .show(ui, |plot| {
            plot.set_plot_bounds(PlotBounds::from_min_max([x0, y0], [x1, y1]));
            if let Some(c) = &hovered {
                plot.vline(highlight(centre(c, bucket)));
            }
            if let Some(last) = history.candles.last() {
                plot.hline(
                    HLine::new(last.close as f64)
                        .color(DIM.gamma_multiply(0.5))
                        .style(LineStyle::Dashed { length: 4.0 })
                        .width(1.0_f32)
                        .allow_hover(false),
                );
            }
            match settings.style {
                Style::Candles => {
                    let boxes = history
                        .candles
                        .iter()
                        .map(|c| {
                            let colour = if rising(c) { ENOUGH } else { SHORT };
                            let (bottom, top) = (c.open.min(c.close) as f64, c.open.max(c.close) as f64);
                            BoxElem::new(
                                centre(c, bucket),
                                BoxSpread::new(c.low as f64, bottom, c.close as f64, top, c.high as f64),
                            )
                            .box_width(bucket.secs() as f64 * BODY)
                            .whisker_width(0.0)
                            .fill(colour.gamma_multiply(0.55))
                            .stroke(Stroke::new(1.0_f32, colour))
                        })
                        .collect();
                    plot.box_plot(BoxPlot::new(boxes).allow_hover(false));
                }
                Style::Line => {
                    let closes: PlotPoints = history
                        .candles
                        .iter()
                        .map(|c| [centre(c, bucket), c.close as f64])
                        .collect();
                    plot.line(
                        Line::new(closes)
                            .color(TEXT.gamma_multiply(0.85))
                            .width(1.5_f32)
                            .fill(y0 as f32)
                            // egui_plot scales the fill in linear light, so
                            // a little goes a long way
                            .fill_alpha(0.012_f32)
                            .allow_hover(false),
                    );
                }
            }
            fills(plot, history);
            plot.pointer_coordinate().map(|p| p.x)
        });
    hover = hover.or(price.response.hovered().then_some(price.inner).flatten());

    let deepest = history.candles.iter().map(|c| c.volume).max().unwrap_or(1).max(1) as f64;
    let volume = base_plot("capitol_market_volume", width, VOLUME_HEIGHT, secs_per_point)
        .show_axes([true, true])
        .y_axis_formatter(|_, _| String::new())
        .x_grid_spacer(move |input| time_marks(input, secs_per_point))
        .x_axis_formatter(move |mark, _| time_label(mark, secs_per_point))
        .show(ui, |plot| {
            plot.set_plot_bounds(PlotBounds::from_min_max([x0, 0.0], [x1, deepest * 1.15]));
            if let Some(c) = &hovered {
                plot.vline(highlight(centre(c, bucket)));
            }
            let bars = history
                .candles
                .iter()
                .map(|c| {
                    let colour = if rising(c) { ENOUGH } else { SHORT };
                    Bar::new(centre(c, bucket), c.volume as f64)
                        .width(bucket.secs() as f64 * BODY)
                        .fill(colour.gamma_multiply(0.35))
                        .stroke(Stroke::NONE)
                })
                .collect();
            plot.bar_chart(BarChart::new(bars).allow_hover(false));
            plot.pointer_coordinate().map(|p| p.x)
        });
    hover = hover.or(volume.response.hovered().then_some(volume.inner).flatten());
    state.hover = hover;
}

/// A plot as both are: still, no hover labels of its own, the price axis on
/// the right and the theme's grid.
fn base_plot<'a>(id: &str, width: f32, height: f32, secs_per_point: f64) -> Plot<'a> {
    Plot::new(Id::new(id))
        .width(width)
        .height(height)
        .show_background(false)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_x(false)
        .show_y(false)
        .y_axis_position(egui_plot::HPlacement::Right)
        .y_axis_min_width(AXIS_WIDTH)
        .x_grid_spacer(move |input| time_marks(input, secs_per_point))
        .set_margin_fraction(egui::Vec2::ZERO)
}

fn highlight(x: f64) -> VLine {
    VLine::new(x)
        .color(TEXT.gamma_multiply(0.18))
        .width(1.0_f32)
        .allow_hover(false)
}

/// Your fills, gold over a dark rim: up for a buy, down for a sell.
fn fills(plot: &mut egui_plot::PlotUi, history: &History) {
    for (side, shape) in [(OrderSide::Buy, MarkerShape::Up), (OrderSide::Sell, MarkerShape::Down)] {
        let at: Vec<[f64; 2]> = history
            .mine
            .iter()
            .filter(|f| f.side == side)
            .map(|f| [f.trade.at.timestamp() as f64, f.trade.price as f64])
            .collect();
        if at.is_empty() {
            continue;
        }
        plot.points(
            Points::new(at.clone())
                .shape(shape)
                .radius(6.5_f32)
                .filled(true)
                .color(INK_SHADOW)
                .allow_hover(false),
        );
        plot.points(
            Points::new(at)
                .shape(shape)
                .radius(4.5_f32)
                .filled(true)
                .color(GOLD)
                .allow_hover(false),
        );
    }
}

/// How far apart price labels must be, in points.
const PRICE_SPACING: f64 = 24.0;

/// The whole-Strand step, 1, 2 or 5 times a power of ten, whose labels sit
/// at least [`PRICE_SPACING`] apart.
fn price_step(strands_per_point: f64) -> i64 {
    let least = strands_per_point * PRICE_SPACING;
    let mut step = 1i64;
    loop {
        for k in [1, 2, 5] {
            if (step * k) as f64 >= least {
                return step * k;
            }
        }
        step *= 10;
    }
}

/// Grid lines on whole Strands: every `step`, labelled, and the fifth or
/// half of it between, fainter.
fn price_marks(input: GridInput, step: i64) -> Vec<GridMark> {
    let finer = if step % 5 == 0 { step / 5 } else { step / 2 }.max(1);
    let (from, to) = (input.bounds.0.ceil() as i64, input.bounds.1.floor() as i64);
    let passes: &[i64] = if finer < step { &[finer, step] } else { &[step] };
    let mut marks = Vec::new();
    for &s in passes {
        let mut v = from.div_euclid(s) * s;
        while v <= to {
            if v >= from && (s == step || v % step != 0) {
                marks.push(GridMark {
                    value: v as f64,
                    step_size: s as f64,
                });
            }
            v += s;
        }
    }
    marks
}

/// The steps a time axis may mark, in seconds, finest first.
const STEPS: [i64; 10] = [
    3600,
    3 * 3600,
    6 * 3600,
    12 * 3600,
    86_400,
    2 * 86_400,
    7 * 86_400,
    14 * 86_400,
    28 * 86_400,
    56 * 86_400,
];

/// The step whose labels sit at least [`LABEL_SPACING`] apart.
fn label_step(secs_per_point: f64) -> i64 {
    let least = secs_per_point * LABEL_SPACING as f64;
    STEPS
        .into_iter()
        .find(|s| *s as f64 >= least)
        .unwrap_or(STEPS[STEPS.len() - 1])
}

/// Seconds east of UTC here, now.
fn local_offset() -> i64 {
    Local::now().offset().local_minus_utc() as i64
}

/// Grid lines on local hours and midnights: the labelled step, and the one
/// under it as fainter lines.
fn time_marks(input: GridInput, secs_per_point: f64) -> Vec<GridMark> {
    let label = label_step(secs_per_point);
    let finer = STEPS
        .into_iter()
        .rev()
        .find(|s| *s < label && *s as f64 / secs_per_point >= 14.0);
    let offset = local_offset();
    let (from, to) = (input.bounds.0 as i64, input.bounds.1 as i64);
    let mut marks = Vec::new();
    for step in finer.into_iter().chain([label]) {
        let mut t = (from + offset).div_euclid(step) * step - offset;
        while t <= to {
            if t >= from && (step == label || (t + offset).rem_euclid(label) != 0) {
                marks.push(GridMark {
                    value: t as f64,
                    step_size: step as f64,
                });
            }
            t += step;
        }
    }
    marks
}

/// A labelled mark's time: the hour within a day, the weekday at midnight,
/// the date when days are far apart. Fainter marks go unlabelled.
fn time_label(mark: GridMark, secs_per_point: f64) -> String {
    let label = label_step(secs_per_point);
    if (mark.step_size as i64) < label {
        return String::new();
    }
    let t = local(mark.value as i64);
    if label < 86_400 && (t.hour() != 0 || t.minute() != 0) {
        format!("{:02}:00", t.hour())
    } else if label <= 2 * 86_400 {
        format!("{} {}", weekday(t.weekday()), t.day())
    } else {
        format!("{} {}", t.day(), month(t.month()))
    }
}

fn local(secs: i64) -> DateTime<Local> {
    Local
        .timestamp_opt(secs, 0)
        .single()
        .unwrap_or_else(|| Local.timestamp_opt(0, 0).unwrap())
}

fn weekday(day: chrono::Weekday) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][day.num_days_from_monday() as usize]
}

fn month(m: u32) -> &'static str {
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m as usize + 11) % 12]
}

/// When a bucket starts, as the readout says it.
fn bucket_name(candle: &Candle, bucket: Bucket) -> String {
    let t = local(candle.at.timestamp());
    match bucket {
        Bucket::Hour => format!("{} {} {:02}:00", weekday(t.weekday()), t.day(), t.hour()),
        Bucket::Day => format!("{} {} {}", weekday(t.weekday()), t.day(), month(t.month())),
    }
}

/// The line over the chart: the hovered bucket's numbers, or the latest's.
fn readout(ui: &mut egui::Ui, history: &History, hovered: Option<&Candle>) {
    let bucket = history.key.bucket;
    let (candle, when) = match (hovered, history.candles.last()) {
        (Some(c), _) => (c, bucket_name(c, bucket)),
        (None, Some(c)) => (c, "latest".to_owned()),
        (None, None) => {
            ui.label(theme::dim(" ").size(size::SMALL));
            return;
        }
    };
    let from = candle.at.timestamp();
    let yours: i64 = history
        .mine
        .iter()
        .filter(|f| (from..from + bucket.secs()).contains(&f.trade.at.timestamp()))
        .map(|f| f.trade.quantity as i64)
        .sum();
    let colour = if rising(candle) { ENOUGH } else { SHORT };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let small = |s: String| RichText::new(s).size(size::SMALL);
        ui.label(small(when).color(DIM));
        for (name, v) in [
            ("O", candle.open),
            ("H", candle.high),
            ("L", candle.low),
            ("C", candle.close),
        ] {
            ui.label(small(name.to_owned()).color(DIM));
            ui.add_space(-4.0);
            ui.label(small(grouped(v)).color(if name == "C" { colour } else { TEXT }));
        }
        ui.label(small(format!(
            "· {} in {}",
            grouped(candle.volume),
            if candle.trades == 1 { "1 trade".to_owned() } else { format!("{} trades", candle.trades) }
        )).color(DIM));
        if yours > 0 {
            ui.label(small(format!("· {} yours", grouped(yours))).color(GOLD));
        }
    });
}

/// The latest trades, newest first. Returns the one clicked.
pub fn tape(
    ui: &mut egui::Ui,
    state: &mut ChartState,
    history: &History,
    height: f32,
) -> Option<MarketTrade> {
    state.tape_rows.clear();
    let mut clicked = None;
    let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::hover());
    let painter = ui.painter();
    let font = theme::body(size::TINY);
    for (x, align, text) in [
        (head.left() + 8.0, Align2::LEFT_CENTER, "TIME"),
        (head.left() + PRICE_AT, Align2::RIGHT_CENTER, "PRICE"),
        (head.left() + QTY_AT, Align2::RIGHT_CENTER, "QTY"),
        (head.left() + GRADE_AT, Align2::LEFT_CENTER, "GRADE"),
    ] {
        painter.text(egui::pos2(x, head.center().y), align, text, font.clone(), DIM);
    }
    if history.recent.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| ui.label(theme::dim("No trades yet").size(size::SMALL)));
        return None;
    }
    let today = Local::now().date_naive();
    egui::ScrollArea::vertical()
        .id_salt("capitol_market_tape")
        .max_height(height - 16.0)
        .auto_shrink([false, false])
        .show_following(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let trades: Vec<&MarketTrade> = history.recent.iter().collect();
            for (i, trade) in trades.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), TAPE_ROW),
                    Sense::click(),
                );
                state.tape_rows.push((trade.id, rect));
                let mine = history.is_mine(trade);
                let painter = ui.painter();
                if mine.is_some() {
                    painter.rect_filled(rect, 2.0, GOLD_WASH);
                }
                if response.hovered() {
                    painter.rect_filled(rect, 2.0, theme::SLOT_HOVER.gamma_multiply(0.7));
                }
                let y = rect.center().y;
                if mine.is_some() {
                    painter.circle_filled(egui::pos2(rect.left() + 3.0, y), 2.0, GOLD);
                }
                let t = local(trade.at.timestamp());
                let when = if t.date_naive() == today {
                    format!("{:02}:{:02}", t.hour(), t.minute())
                } else {
                    format!("{} {}", t.day(), month(t.month()))
                };
                painter.text(
                    egui::pos2(rect.left() + 8.0, y),
                    Align2::LEFT_CENTER,
                    when,
                    theme::body(size::LABEL),
                    DIM,
                );
                // Up or down on the trade before it
                let tick = match trades.get(i + 1) {
                    Some(prev) if trade.price > prev.price => ENOUGH,
                    Some(prev) if trade.price < prev.price => SHORT,
                    _ => TEXT,
                };
                painter.text(
                    egui::pos2(rect.left() + PRICE_AT, y),
                    Align2::RIGHT_CENTER,
                    grouped(trade.price),
                    theme::strong(size::SMALL),
                    tick,
                );
                painter.text(
                    egui::pos2(rect.left() + QTY_AT, y),
                    Align2::RIGHT_CENTER,
                    grouped(trade.quantity as i64),
                    theme::body(size::SMALL),
                    if mine.is_some() { GOLD } else { TEXT },
                );
                painter.text(
                    egui::pos2(rect.left() + GRADE_AT, y),
                    Align2::LEFT_CENTER,
                    format!("{} {}", trade.grade, trade.quality),
                    theme::body(size::LABEL),
                    theme::grade(&trade.grade),
                );
                let who = match mine {
                    Some(OrderSide::Buy) => "You bought these. ",
                    Some(OrderSide::Sell) => "You sold these. ",
                    None => "",
                };
                let response = response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!(
                        "{who}{} at {} each, quality {} (lowest {}). Click to use this price",
                        grouped(trade.quantity as i64),
                        grouped(trade.price),
                        trade.quality,
                        trade.min_quality
                    ));
                if response.clicked() {
                    clicked = Some((*trade).clone());
                }
            }
        });
    clicked
}

/// Where a chart has nothing to draw yet, or nothing at all.
pub fn empty(ui: &mut egui::Ui, text: &str, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::body(size::SMALL),
        DIM,
    );
    ui.painter().hline(
        rect.shrink2(egui::vec2(24.0, 0.0)).x_range(),
        rect.center().y + 14.0,
        Stroke::new(1.0_f32, GOLD_DIM.gamma_multiply(0.25)),
    );
}

/// The plots' height together, with the readout over them.
pub const HEIGHT: f32 = PRICE_HEIGHT + VOLUME_HEIGHT + 22.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_never_crowd_and_land_on_local_hours() {
        // A week over 300 points: about 2000 s a point, so a label each day
        let week = 7.0 * 86_400.0 / 300.0;
        assert_eq!(label_step(week), 2 * 86_400);
        let marks = time_marks(GridInput { bounds: (1.0e9, 1.0e9 + 7.0 * 86_400.0), base_step_size: 1.0 }, week);
        let offset = local_offset();
        assert!(marks.iter().all(|m| (m.value as i64 + offset) % 3600 == 0));
        let labelled: Vec<f64> = marks
            .iter()
            .filter(|m| !time_label(**m, week).is_empty())
            .map(|m| m.value)
            .collect();
        assert!(labelled.windows(2).all(|w| (w[1] - w[0]) / week >= LABEL_SPACING as f64));
        assert!((3..=4).contains(&labelled.len()), "{labelled:?}");
    }
}
