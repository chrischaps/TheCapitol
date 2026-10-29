//! The dev overlay (D5.2, devtools plan §6 and D6): F7 opens a card, bottom
//! left over the controls hint, that says how the client is doing. There it
//! keeps clear of the panels that open top left (crafting, the exchange)
//! and of the offline inscription. It has five
//! parts:
//! - the frame: a graph of the last few seconds' frame times, from
//!   `pre_update`'s dt, with the 60 and 30 fps lines;
//! - the net: the round trip, messages and bytes a second each way and
//!   which types are busiest, and reconnects (`NetStats`, D5.1);
//! - the prediction: corrections, the server's lag, the interpolation's
//!   depth and the others it tracks (`MotionStats`);
//! - the world: the engine's entities and what the store holds;
//! - interest: your cell of the server's interest grid (C0.4), who's in
//!   sight from it and how far its edge is;
//! - for an admin's token (D5.3), a third column, the server: its tick from
//!   the newest of the Observatory's samples, what's been marked lately
//!   (the watches firing among them), the tunables off their defaults, and
//!   which world layer F6 has on.
//!
//! Each part opens with a plain sentence, as the Observatory's console
//! does, and its numbers follow. The frames are recorded whether it's open
//! or not, so the graph is full the moment it opens. It never takes a
//! click: the world and the panels under it work as before.

use std::collections::VecDeque;
use std::time::Duration;

use super::theme::{self, size};
use capitol_core::net::NetStats;
use capitol_core::protocol::annotation::{Annotation, AnnotationKind};
use capitol_core::protocol::ledger::LoadWindow;
use capitol_core::protocol::metrics::Sample;
use capitol_core::protocol::rules::{interest_cell, INTEREST_CELL};
use egui::{Align, Align2, Color32, Id, Layout, Order, Stroke};

/// Frames the graph keeps: four seconds at 60 fps.
pub const FRAMES: usize = 240;
/// A frame's budget at 60 fps, in ms, and the 30 fps line past which it's
/// a hitch.
pub const BUDGET_MS: f32 = 1000.0 / 60.0;
pub const HITCH_MS: f32 = 1000.0 / 30.0;
/// The graph's height, and the frame time at its top: never under 50 ms,
/// so the guides stay put, and never over 15 fps, so one long load doesn't
/// flatten the rest (a frame past the top is cut off there).
const GRAPH_HEIGHT: f32 = 64.0;
const GRAPH_FLOOR_MS: f32 = 50.0;
const GRAPH_CEILING_MS: f32 = 1000.0 / 15.0;
/// The card's two columns, and the gap between them: the frames, the world
/// and interest on the left, the net and the prediction on the right, so
/// it stays short enough for 853×480 points (1280×720 at 150%).
const COLUMN: f32 = 236.0;
const GAP: f32 = 24.0;
/// How long it takes to fade in or out, in seconds.
const FADE: f32 = 0.12;
/// Message types named as the busiest each way.
const BUSIEST: usize = 3;
/// A row's height.
const ROW: f32 = 17.0;
/// The annotations and the tuned knobs the server's column lists.
const MARKS_SHOWN: usize = 4;
const TUNED_SHOWN: usize = 4;

/// The overlay: open or not, and what `pre_update` recorded for it.
#[derive(Default)]
pub struct DevOverlay {
    pub open: bool,
    frames: Frames,
    /// The engine's entities, last frame.
    entities: usize,
}

impl DevOverlay {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Once a frame, from `pre_update`, open or not: the frame's dt in
    /// seconds and the engine's entity count.
    pub fn record(&mut self, dt: f64, entities: usize) {
        self.frames.push(dt);
        self.entities = entities;
    }

    /// Draws it while it's open or fading, asking `view` for what to show
    /// only then.
    pub fn show(&self, ctx: &egui::Context, view: impl FnOnce() -> View) {
        let k = ctx.animate_bool_with_time(Id::new("capitol_dev_overlay"), self.open, FADE);
        if k <= 0.0 {
            return;
        }
        let view = view();
        egui::Area::new(Id::new("capitol_dev_overlay_card"))
            .order(Order::Foreground)
            .anchor(Align2::LEFT_BOTTOM, [16.0, -(super::CONTROLS_HEIGHT + 8.0)])
            .interactable(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(k);
                theme::panel_edged(theme::GOLD).show(ui, |ui| {
                    let columns = if view.server.is_some() { 3.0 } else { 2.0 };
                    ui.set_width(COLUMN * columns + GAP * (columns - 1.0));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.label(theme::title("Observatory", size::HEADING));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            theme::keycap(ui, "F7");
                            if let Some(server) = &view.server {
                                ui.add_space(8.0);
                                ui.label(theme::dim(format!("World layer: {}", server.layer)).size(size::SMALL));
                                theme::keycap(ui, "F6");
                            }
                        });
                    });
                    ui.add_space(2.0);
                    theme::gold_rule(ui);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = GAP;
                        ui.vertical(|ui| {
                            ui.set_width(COLUMN);
                            self.frame_part(ui);
                            self.world_part(ui, view.world.as_ref());
                            if let Some(interest) = &view.interest {
                                interest_part(ui, interest);
                            }
                        });
                        ui.vertical(|ui| {
                            ui.set_width(COLUMN);
                            net_part(ui, view.net.as_ref());
                            if let Some(motion) = view.net.as_ref().and_then(|n| n.motion.as_ref()) {
                                prediction_part(ui, motion);
                            }
                        });
                        if let Some(server) = &view.server {
                            ui.vertical(|ui| {
                                ui.set_width(COLUMN);
                                server_parts(ui, server);
                            });
                        }
                    });
                });
            });
    }

    fn frame_part(&self, ui: &mut egui::Ui) {
        part(ui, "Frame");
        let Some(summary) = self.frames.summary() else {
            sentence(ui, "No frames yet.");
            return;
        };
        sentence(ui, &summary.sentence());
        ui.add_space(2.0);
        self.frames.graph(ui, summary.worst);
        ui.label(
            theme::dim(format!(
                "now {:.1} · mean {:.1} · p99 {:.1} · worst {:.1} ms",
                summary.last, summary.mean, summary.p99, summary.worst
            ))
            .size(size::TINY),
        );
    }

    fn world_part(&self, ui: &mut egui::Ui, world: Option<&World>) {
        part(ui, "World");
        // Who's in sight and the nodes are in the sentence
        let mut rows = vec![("Entities", count(self.entities))];
        match world {
            Some(w) => {
                sentence(ui, &w.sentence());
                rows.push(("Stations", count(w.stations)));
                rows.push(("Claimed lots", count(w.lots)));
            }
            None => sentence(ui, "Offline: the engine's entities only."),
        }
        table(ui, &rows);
    }
}

/// What the overlay shows besides the frames, gathered by the extension
/// each frame it's open.
#[derive(Default)]
pub struct View {
    /// The connection and the prediction; `None` offline.
    pub net: Option<NetStats>,
    /// What the store holds; `None` offline.
    pub world: Option<World>,
    /// Your interest cell, once the server has placed you.
    pub interest: Option<Interest>,
    /// The server, for an admin's token (D5.3).
    pub server: Option<Server>,
}

/// The server as an admin's client sees it (D5.3), from
/// `capitol_core::admin::Watch`.
#[derive(Debug, Clone, Default)]
pub struct Server {
    /// The newest sample, once one has come.
    pub sample: Option<Sample>,
    /// The newest annotations, newest first.
    pub marks: Vec<Annotation>,
    /// How many knobs there are, once read.
    pub knobs: Option<usize>,
    /// The knobs off their defaults: what each is called, its value and its
    /// default, as the console says them.
    pub tuned: Vec<(String, String, String)>,
    /// The server's clock now (Unix ms), for how long ago things were.
    pub now_ms: Option<i64>,
    /// The world layer F6 has on.
    pub layer: &'static str,
}

impl Server {
    pub fn from_watch(watch: &capitol_core::admin::Watch, now_ms: Option<i64>, layer: &'static str) -> Server {
        Server {
            sample: watch.sample().cloned(),
            marks: watch.annotations().rev().take(MARKS_SHOWN).cloned().collect(),
            knobs: watch.tunables().map(<[_]>::len),
            tuned: watch
                .tuned()
                .into_iter()
                .map(|k| (k.label.clone(), k.unit.format(k.value), k.unit.format(k.default)))
                .collect(),
            now_ms,
            layer,
        }
    }
}

/// The tick over a sample's window in a sentence: its mean against its
/// budget, and how many ran over.
pub fn tick_sentence(load: &LoadWindow) -> String {
    if load.ticks == 0 {
        return "No ticks in the newest sample.".to_owned();
    }
    let budget = load.tick_period_ms;
    match load.ticks_over {
        0 => format!("The tick takes {:.1} ms of its {budget:.0}, and none ran over.", load.tick_mean_ms),
        n => format!(
            "The tick takes {:.1} ms of its {budget:.0}; {} of {} ran over.",
            load.tick_mean_ms,
            count(n as usize),
            count(load.ticks as usize)
        ),
    }
}

/// How long before `now_ms` `at_ms` was: `40 s ago`, `12 min ago`, `3 h ago`.
pub fn ago(now_ms: i64, at_ms: i64) -> String {
    let secs = (now_ms - at_ms).max(0) / 1000;
    match secs {
        0..=4 => "just now".to_owned(),
        5..=59 => format!("{secs} s ago"),
        60..=3599 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3600),
    }
}

/// The server's column: the tick, what's been marked, and the tunables.
fn server_parts(ui: &mut egui::Ui, server: &Server) {
    part(ui, "Server");
    match &server.sample {
        None => sentence(ui, "Waiting on the first sample."),
        Some(sample) => {
            let load = &sample.load;
            sentence(ui, &tick_sentence(load));
            // The phase that takes the most of the tick, per tick
            let busiest = load
                .phases
                .iter()
                .max_by(|a, b| a.1.secs.total_cmp(&b.1.secs))
                .filter(|(_, p)| p.secs > 0.0)
                .map(|(name, p)| format!("{name} · {:.2} ms", p.secs * 1000.0 / load.ticks.max(1) as f64));
            let p95 = load
                .tick_p95_ms
                .map_or_else(|| "past the last bucket".to_owned(), |ms| format!("{ms:.2} ms"));
            let mut rows = vec![("p95", p95)];
            if let Some(busiest) = busiest {
                rows.push(("Busiest phase", busiest));
            }
            rows.push((
                "Playing",
                format!(
                    "{} · {}",
                    plural(sample.players.people as usize, "person", "people"),
                    plural(sample.players.citizens as usize, "citizen", "citizens")
                ),
            ));
            rows.push(("Sockets", count(load.sockets_open as usize)));
            if let Some(now) = server.now_ms {
                rows.push(("Sampled", ago(now, sample.at_ms)));
            }
            table(ui, &rows);
        }
    }

    part(ui, "Marked");
    if server.marks.is_empty() {
        sentence(ui, "Nothing marked in the last hour.");
    }
    for mark in &server.marks {
        let colour = match mark.kind {
            AnnotationKind::WatchFired => theme::FRAME_HITCH,
            AnnotationKind::TunableChanged => theme::GOLD,
            _ => theme::TEXT,
        };
        if let Some(now) = server.now_ms {
            ui.label(theme::dim(ago(now, mark.at_ms)).size(size::TINY));
        }
        ui.add(egui::Label::new(theme::text(&mark.text).size(size::SMALL).color(colour)).wrap());
    }

    part(ui, "Tunables");
    match server.knobs {
        None => sentence(ui, "Not read yet."),
        Some(n) if server.tuned.is_empty() => sentence(ui, &format!("All {n} knobs at their defaults.")),
        Some(n) => {
            sentence(
                ui,
                &format!("{} of {n} off their defaults. The console sets them.", server.tuned.len()),
            );
            for (label, value, default) in server.tuned.iter().take(TUNED_SHOWN) {
                row(ui, label, &format!("{value} (default {default})"), theme::GOLD);
            }
            if server.tuned.len() > TUNED_SHOWN {
                ui.label(theme::dim(format!("and {} more", server.tuned.len() - TUNED_SHOWN)).size(size::TINY));
            }
        }
    }
}

/// `--dev-sample`: numbers like a client's in a busy town, to see the
/// online layout offline.
pub fn sample() -> View {
    use capitol_core::motion::MotionStats;
    use capitol_core::net::{Traffic, TypeRate};
    let rate = |kind, messages, bytes| TypeRate { kind, messages, bytes };
    let net = NetStats {
        rtt: Some(Duration::from_millis(6)),
        rtt_avg: Some(Duration::from_millis(5)),
        inbound: Traffic {
            per_second: vec![rate("positions", 10, 4_210), rate("extraction_progress", 3, 390), rate("pong", 1, 24)],
            messages: 18_342,
            bytes: 6_410_000,
        },
        outbound: Traffic { per_second: vec![rate("move", 1, 64)], messages: 412, bytes: 30_100 },
        reconnects: 1,
        motion: Some(MotionStats {
            corrections: 4,
            corrected: 9.6,
            last_correction: 0.42,
            lag: Some(Duration::from_millis(58)),
            interp_depth: Some(0.046),
            tracks: 11,
        }),
    };
    let now_ms = 1_792_620_000_000;
    let sample: Sample = capitol_core::serde_json::from_str(SAMPLE).expect("the sample fixture decodes");
    let mark = |id, ago_s: i64, kind, text: &str| Annotation { id, at_ms: now_ms - ago_s * 1000, kind, text: text.into() };
    View {
        net: Some(net),
        world: Some(World { people: 12, citizens: 9, nodes: 14_906, stations: 37, lots: 102 }),
        interest: Some(Interest::at(6812.0, 6745.0)),
        server: Some(Server {
            sample: Some(Sample { at_ms: now_ms - 4000, ..sample }),
            marks: vec![
                mark(44, 50, AnnotationKind::WatchFired, "Fiber gatherers hold 81% of the Strands"),
                mark(43, 400, AnnotationKind::TunableChanged, "Property tax a week: 1% → 2.5%"),
                mark(42, 1900, AnnotationKind::TownUp, "The town came on: 18 citizens"),
            ],
            knobs: Some(31),
            tuned: vec![("Property tax a week".into(), "2.5%".into(), "1%".into())],
            now_ms: Some(now_ms),
            layer: "interest cells",
        }),
    }
}

/// The protocol's sample fixture: ten seconds of a 20-citizen town, for
/// `--dev-sample` and the tests.
const SAMPLE: &str = include_str!("../../../../../protocol/tests/fixtures/rest/sample.json");

/// What the store holds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct World {
    /// Players in sight, you among them, and how many are citizens.
    pub people: usize,
    pub citizens: usize,
    pub nodes: usize,
    pub stations: usize,
    /// Everyone's claimed lots.
    pub lots: usize,
}

impl World {
    pub fn sentence(&self) -> String {
        let others = self.people.saturating_sub(1);
        let people = match (others, self.citizens) {
            (0, _) => "Nobody else in sight".to_owned(),
            (n, 0) => format!("{} in sight", plural(n, "other", "others")),
            (n, c) if c == n => format!("{} in sight, all citizens", plural(n, "other", "others")),
            (n, c) => format!("{} in sight, {c} of them citizens", plural(n, "other", "others")),
        };
        format!("{people}; the store knows {} nodes.", count(self.nodes))
    }
}

/// Your cell of the server's interest grid (C0.4): the server sends you
/// the players in it and the eight round it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interest {
    pub cell: (i32, i32),
    /// How far you are from the nearest side of your cell, in server
    /// units: walk that far and who's in sight changes.
    pub edge: f64,
}

impl Interest {
    /// At `(x, y)` in server units.
    pub fn at(x: f64, y: f64) -> Interest {
        let cell = interest_cell(x, y);
        let (left, top) = (cell.0 as f64 * INTEREST_CELL, cell.1 as f64 * INTEREST_CELL);
        let edge = [x - left, left + INTEREST_CELL - x, y - top, top + INTEREST_CELL - y]
            .into_iter()
            .fold(f64::INFINITY, f64::min);
        Interest { cell, edge }
    }

    pub fn sentence(&self) -> String {
        let side = INTEREST_CELL * 3.0;
        format!(
            "You see the {side:.0} u square round cell ({}, {}); its edge is {:.0} u away.",
            self.cell.0, self.cell.1, self.edge
        )
    }
}

/// The last [`FRAMES`] frame times, in ms.
#[derive(Debug, Default)]
pub struct Frames {
    ms: VecDeque<f32>,
}

/// The frames over the graph's window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub last: f32,
    pub mean: f32,
    /// The time 99 frames in 100 come in under.
    pub p99: f32,
    pub worst: f32,
    /// Frames over [`HITCH_MS`].
    pub hitches: usize,
    /// How long the window is, in seconds.
    pub seconds: f32,
}

impl Frames {
    /// A frame's dt, in seconds.
    pub fn push(&mut self, dt: f64) {
        if self.ms.len() == FRAMES {
            self.ms.pop_front();
        }
        self.ms.push_back((dt * 1000.0) as f32);
    }

    pub fn summary(&self) -> Option<Summary> {
        let last = *self.ms.back()?;
        let total: f32 = self.ms.iter().sum();
        let mut sorted: Vec<f32> = self.ms.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        let p99 = sorted[(sorted.len() * 99 / 100).min(sorted.len() - 1)];
        Some(Summary {
            last,
            mean: total / self.ms.len() as f32,
            p99,
            worst: sorted[sorted.len() - 1],
            hitches: self.ms.iter().filter(|&&ms| ms > HITCH_MS).count(),
            seconds: total / 1000.0,
        })
    }

    /// The frames as bars, oldest at the left, over the 60 and 30 fps lines.
    fn graph(&self, ui: &mut egui::Ui, worst: f32) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), GRAPH_HEIGHT),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, theme::WELL);
        let top_ms = (worst * 1.15).clamp(GRAPH_FLOOR_MS, GRAPH_CEILING_MS);
        let y = |ms: f32| rect.bottom() - rect.height() * (ms / top_ms).min(1.0);
        let step = rect.width() / FRAMES as f32;
        // Right-aligned, so a graph still filling grows from the right
        let start = rect.right() - step * self.ms.len() as f32;
        for (i, &ms) in self.ms.iter().enumerate() {
            let x = start + step * i as f32;
            let colour = frame_colour(ms);
            let bar = egui::Rect::from_min_max(egui::pos2(x, y(ms)), egui::pos2(x + step.max(1.0), rect.bottom()));
            painter.rect_filled(bar, 0.0, colour);
        }
        for (ms, fps) in [(BUDGET_MS, "60"), (HITCH_MS, "30")] {
            let at = y(ms);
            painter.hline(rect.x_range(), at, Stroke::new(1.0_f32, theme::GRAPH_GUIDE));
            // On a scrap of the well, so the bars don't swallow it
            let galley = painter.layout_no_wrap(fps.to_owned(), theme::body(size::TINY), theme::DIM);
            let at = egui::pos2(rect.left() + 3.0, at - galley.size().y / 2.0);
            painter.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand(1.5), 2.0, theme::WELL_SOLID);
            painter.galley(at, galley, theme::DIM);
        }
    }
}

impl Summary {
    pub fn sentence(&self) -> String {
        let fps = 1000.0 / self.mean.max(0.001);
        if self.hitches > 0 {
            format!(
                "{fps:.0} fps, with {} over {HITCH_MS:.0} ms in {:.0} s; the worst took {:.0} ms.",
                plural(self.hitches, "hitch", "hitches"),
                self.seconds,
                self.worst
            )
        } else if self.p99 <= BUDGET_MS * 1.2 {
            format!("Steady at {fps:.0} fps.")
        } else {
            format!("{fps:.0} fps; the worst frame took {:.0} ms.", self.worst)
        }
    }
}

fn frame_colour(ms: f32) -> Color32 {
    if ms > HITCH_MS {
        theme::FRAME_HITCH
    } else if ms > BUDGET_MS * 1.2 {
        theme::FRAME_LATE
    } else {
        theme::FRAME_OK
    }
}

fn net_part(ui: &mut egui::Ui, net: Option<&NetStats>) {
    part(ui, "Net");
    let Some(net) = net else {
        sentence(ui, "Offline: no server, so no net.");
        return;
    };
    sentence(ui, &net_sentence(net));
    let ms = |d: Duration| format!("{:.0} ms", d.as_secs_f64() * 1000.0);
    let trip = match (net.rtt, net.rtt_avg) {
        (Some(rtt), Some(avg)) => format!("{} · avg {}", ms(rtt), ms(avg)),
        _ => "not yet".to_owned(),
    };
    let way = |t: &capitol_core::net::Traffic| {
        format!("{} msg/s · {:.1} kB/s", t.messages_per_second(), t.bytes_per_second() as f64 / 1000.0)
    };
    table(ui, &[("Round trip", trip), ("In", way(&net.inbound))]);
    ui.add(egui::Label::new(theme::dim(busiest(&net.inbound)).size(size::TINY)).wrap());
    table(
        ui,
        &[("Out", way(&net.outbound)), ("Reconnects", count(net.reconnects as usize))],
    );
}

/// The connection in a sentence: the round trip, and what most of the
/// traffic in is.
pub fn net_sentence(net: &NetStats) -> String {
    let mut out = match net.rtt_avg {
        Some(avg) => format!("A {:.0} ms round trip", avg.as_secs_f64() * 1000.0),
        None => "Waiting on the first round trip".to_owned(),
    };
    let per_second = net.inbound.messages_per_second();
    match net.inbound.per_second.first() {
        Some(top) if per_second > 0 => out.push_str(&format!(
            "; {per_second} messages a second in, mostly {}.",
            top.kind.replace('_', " ")
        )),
        _ => out.push_str("; nothing in this second."),
    }
    if net.reconnects > 0 {
        out.push_str(&format!(" It has reconnected {}.", plural(net.reconnects as usize, "time", "times")));
    }
    out
}

/// The busiest types in the last second: `positions 10 · pong 1`.
fn busiest(traffic: &capitol_core::net::Traffic) -> String {
    if traffic.per_second.is_empty() {
        return "nothing".to_owned();
    }
    traffic
        .per_second
        .iter()
        .take(BUSIEST)
        .map(|r| format!("{} {}", r.kind, r.messages))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn prediction_part(ui: &mut egui::Ui, motion: &capitol_core::motion::MotionStats) {
    part(ui, "Prediction");
    sentence(ui, &prediction_sentence(motion));
    let ms = |s: f64| format!("{:.0} ms", s * 1000.0);
    let depth = match motion.interp_depth {
        Some(d) if d < 0.0 => (format!("{} behind", ms(-d)), theme::FRAME_HITCH),
        Some(d) => (ms(d), theme::TEXT),
        None => ("no snapshot yet".to_owned(), theme::DIM),
    };
    table(
        ui,
        &[
            ("Corrections", format!("{} · {:.1} u, last {:.2}", motion.corrections, motion.corrected, motion.last_correction)),
            ("Server lag", motion.lag.map_or_else(|| "not yet".to_owned(), |l| ms(l.as_secs_f64()))),
            ("Tracking", plural(motion.tracks, "other", "others")),
        ],
    );
    row(ui, "Interp. depth", &depth.0, depth.1);
}

/// The prediction in a sentence: how often the server put it right, and
/// whether others are drawn from snapshots or guessed past them.
pub fn prediction_sentence(motion: &capitol_core::motion::MotionStats) -> String {
    let corrected = match motion.corrections {
        0 => "The server hasn't corrected the prediction".to_owned(),
        n => format!(
            "Corrected {}, the last by {:.1} u",
            plural(n as usize, "time", "times"),
            motion.last_correction
        ),
    };
    match motion.interp_depth {
        Some(d) if d < 0.0 => format!("{corrected}; others are extrapolated {:.0} ms past the last snapshot.", -d * 1000.0),
        _ => format!("{corrected}."),
    }
}

/// The sentence names the cell and how far its edge is, which is all of it.
fn interest_part(ui: &mut egui::Ui, interest: &Interest) {
    part(ui, "Interest");
    sentence(ui, &interest.sentence());
}

/// A part's label, with a little air above it.
fn part(ui: &mut egui::Ui, name: &str) {
    ui.add_space(8.0);
    ui.label(theme::label(name));
}

/// A part's plain sentence, wrapped to the card.
fn sentence(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(theme::text(text).size(size::SMALL)).wrap());
}

fn table(ui: &mut egui::Ui, rows: &[(&str, String)]) {
    for (name, value) in rows {
        row(ui, name, value, theme::TEXT);
    }
}

/// A name on the left and its value on the right, so the numbers line up,
/// painted as one line rather than laid out, so a row is no taller than
/// its text.
fn row(ui: &mut egui::Ui, name: &str, value: &str, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::hover());
    let painter = ui.painter();
    let font = theme::body(size::SMALL);
    painter.text(rect.left_center(), Align2::LEFT_CENTER, name, font.clone(), theme::DIM);
    painter.text(rect.right_center(), Align2::RIGHT_CENTER, value, font, colour);
}

fn count(n: usize) -> String {
    super::hud::grouped(n as i64)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", count(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::motion::MotionStats;
    use capitol_core::net::{Traffic, TypeRate};

    fn frames(ms: &[f32]) -> Frames {
        let mut f = Frames::default();
        for &m in ms {
            f.push(m as f64 / 1000.0);
        }
        f
    }

    #[test]
    fn the_graph_keeps_the_last_four_seconds() {
        let mut f = Frames::default();
        for _ in 0..FRAMES + 50 {
            f.push(0.016);
        }
        f.push(0.040);
        assert_eq!(f.ms.len(), FRAMES);
        let s = f.summary().unwrap();
        assert_eq!(s.last, 40.0);
        assert_eq!(s.worst, 40.0);
        assert_eq!(s.hitches, 1);
    }

    #[test]
    fn a_steady_run_says_so() {
        let s = frames(&[16.6; 120]).summary().unwrap();
        assert_eq!(s.hitches, 0);
        assert_eq!(s.sentence(), "Steady at 60 fps.");
        assert!(Frames::default().summary().is_none());
    }

    #[test]
    fn hitches_are_counted_and_named() {
        let mut ms = vec![16.0; 98];
        ms.extend([45.0, 80.0]);
        let s = frames(&ms).summary().unwrap();
        assert_eq!(s.hitches, 2);
        assert_eq!(s.worst, 80.0);
        assert_eq!(s.p99, 80.0);
        assert!(s.sentence().contains("2 hitches over 33 ms"), "{}", s.sentence());
        assert!(s.sentence().ends_with("the worst took 80 ms."), "{}", s.sentence());
    }

    #[test]
    fn late_but_not_hitching_names_the_worst() {
        let mut ms = vec![16.0; 90];
        ms.extend([25.0; 10]);
        let s = frames(&ms).summary().unwrap();
        assert_eq!(s.hitches, 0);
        assert_eq!(s.sentence(), "59 fps; the worst frame took 25 ms.");
    }

    #[test]
    fn the_interest_cell_and_its_edge() {
        let c = INTEREST_CELL;
        let i = Interest::at(22.0 * c + 37.0, 31.0 * c + 150.0);
        assert_eq!(i.cell, (22, 31));
        assert!((i.edge - 37.0).abs() < 1e-9);
        // The server's own cell, from the same rule
        assert_eq!(Interest::at(6800.0, 6800.0).cell, interest_cell(6800.0, 6800.0));
        let i = Interest::at(-1.0, 10.0);
        assert_eq!(i.cell, (-1, 0));
        assert!((i.edge - 1.0).abs() < 1e-9);
        assert_eq!(
            Interest::at(150.0, 150.0).sentence(),
            "You see the 900 u square round cell (0, 0); its edge is 150 u away."
        );
    }

    #[test]
    fn the_net_in_a_sentence() {
        let mut net = NetStats::default();
        assert_eq!(net_sentence(&net), "Waiting on the first round trip; nothing in this second.");
        net.rtt_avg = Some(Duration::from_millis(4));
        net.inbound = Traffic {
            per_second: vec![
                TypeRate { kind: "positions", messages: 10, bytes: 1200 },
                TypeRate { kind: "pong", messages: 1, bytes: 20 },
            ],
            messages: 400,
            bytes: 50_000,
        };
        net.reconnects = 1;
        assert_eq!(
            net_sentence(&net),
            "A 4 ms round trip; 11 messages a second in, mostly positions. It has reconnected 1 time."
        );
        assert_eq!(busiest(&net.inbound), "positions 10 · pong 1");
    }

    #[test]
    fn the_prediction_in_a_sentence() {
        let mut m = MotionStats { corrections: 0, corrected: 0.0, last_correction: 0.0, lag: None, interp_depth: Some(0.05), tracks: 2 };
        assert_eq!(prediction_sentence(&m), "The server hasn't corrected the prediction.");
        m.corrections = 3;
        m.last_correction = 0.84;
        m.interp_depth = Some(-0.02);
        assert_eq!(
            prediction_sentence(&m),
            "Corrected 3 times, the last by 0.8 u; others are extrapolated 20 ms past the last snapshot."
        );
    }

    #[test]
    fn the_world_in_a_sentence() {
        let mut w = World { people: 1, citizens: 0, nodes: 12_480, stations: 3, lots: 40 };
        assert_eq!(w.sentence(), "Nobody else in sight; the store knows 12,480 nodes.");
        w.people = 5;
        w.citizens = 4;
        assert_eq!(w.sentence(), "4 others in sight, all citizens; the store knows 12,480 nodes.");
        w.citizens = 2;
        assert_eq!(w.sentence(), "4 others in sight, 2 of them citizens; the store knows 12,480 nodes.");
    }

    #[test]
    fn the_server_in_sentences() {
        let sample: Sample = capitol_core::serde_json::from_str(SAMPLE).unwrap();
        let mut load = sample.load.clone();
        load.ticks = 100;
        load.ticks_over = 0;
        load.tick_mean_ms = 0.47;
        load.tick_period_ms = 100.0;
        assert_eq!(tick_sentence(&load), "The tick takes 0.5 ms of its 100, and none ran over.");
        load.ticks_over = 3;
        assert_eq!(tick_sentence(&load), "The tick takes 0.5 ms of its 100; 3 of 100 ran over.");
        load.ticks = 0;
        assert_eq!(tick_sentence(&load), "No ticks in the newest sample.");
        assert_eq!(ago(100_000, 98_000), "just now");
        assert_eq!(ago(100_000, 60_000), "40 s ago");
        assert_eq!(ago(800_000, 20_000), "13 min ago");
        assert_eq!(ago(3_700_000, 100_000), "1 h ago");
    }

    #[test]
    fn the_server_column_comes_from_the_watch() {
        use capitol_core::admin::Watch;
        use capitol_core::net::RestResponse;
        use capitol_core::protocol::tunables::{TunableValue, TunablesResponse, Unit};
        let mut watch = Watch::default();
        let knob = |key: &str, value| TunableValue {
            key: key.into(),
            label: format!("Knob {key}"),
            unit: Unit::Fraction,
            min: 0.0,
            max: 1.0,
            step: 0.001,
            default: 0.01,
            value,
            chart: "money".into(),
            base_strands: None,
        };
        watch.observe(&RestResponse::Tunables(TunablesResponse { knobs: vec![knob("a", 0.01), knob("b", 0.025)] }));
        let marks = (1..=6).map(|i| Annotation { id: i, at_ms: i * 1000, kind: AnnotationKind::Boot, text: format!("#{i}") });
        watch.observe(&RestResponse::Annotations(marks.collect()));
        let server = Server::from_watch(&watch, Some(10_000), "off");
        assert_eq!(server.knobs, Some(2));
        assert_eq!(server.tuned, vec![("Knob b".to_owned(), "2.5%".to_owned(), "1%".to_owned())]);
        assert_eq!(server.marks.iter().map(|m| m.id).collect::<Vec<_>>(), [6, 5, 4, 3], "newest first");
    }

    #[test]
    fn it_draws_open_and_offline() {
        let ctx = theme::test_context();
        let mut overlay = DevOverlay { open: true, ..Default::default() };
        for i in 0..FRAMES {
            overlay.record(if i % 50 == 0 { 0.045 } else { 0.016 }, 900);
        }
        let net = NetStats::default().with_motion(MotionStats {
            corrections: 1,
            corrected: 0.5,
            last_correction: 0.5,
            lag: Some(Duration::from_millis(110)),
            interp_depth: Some(0.05),
            tracks: 3,
        });
        let world = World { people: 4, citizens: 2, nodes: 100, stations: 2, lots: 9 };
        let online = || View {
            net: Some(net.clone()),
            world: Some(world),
            interest: Some(Interest::at(6810.0, 6790.0)),
            server: None,
        };
        let _ = ctx.run(Default::default(), |ctx| overlay.show(ctx, online));
        // An admin's, with the server's column, and one still waiting on it all
        let _ = ctx.run(Default::default(), |ctx| overlay.show(ctx, sample));
        let waiting = || View { server: Some(Server::default()), ..View::default() };
        let _ = ctx.run(Default::default(), |ctx| overlay.show(ctx, waiting));
        let _ = ctx.run(Default::default(), |ctx| overlay.show(ctx, View::default));
        // Never opened, it never asks
        let closed = DevOverlay::default();
        let _ = theme::test_context().run(Default::default(), |ctx| closed.show(ctx, || unreachable!()));
    }
}
