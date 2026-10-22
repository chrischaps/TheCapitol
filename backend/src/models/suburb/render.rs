//! The plan view: one SVG per layout and seed, with its metrics, and the
//! report test that writes them (#104).
//!
//! It's a test rather than a bin because the backend has no lib.rs, so a bin
//! can't reach `models`. `backend/tools/suburb-plan.ps1` runs it in release
//! and turns the SVGs into PNGs.
//!
//! Server y points down, as SVG's does, so world coordinates go in as they are.

use std::fmt::Write as _;

use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot, WaterRing};

use super::metrics::{self, fmt_num, Marker, Metrics, Scope, Target, Violation};
use crate::models::station_room;
use super::neighbourhood::{BufferKind, Neighbourhood};
use super::parks::{Ground, Greens, CELL};

type P = (f64, f64);

/// Output width of every SVG, in pixels. A sector view keeps its aspect.
const OUT_PX: f64 = 2400.0;
/// The key takes a column this wide on the left, so it never covers the plan.
const LEGEND_PX: f64 = 480.0;
/// Lots are shaded from this area...
const AREA_LO: f64 = 6000.0;
/// ...to this one, the p5 and p95 targets.
const AREA_HI: f64 = 15000.0;
/// A lot that doesn't take `View::takes` stations is this colour.
const SHORT: &str = "#d8c9a3";

pub(crate) const PAPER: &str = "#f3eee2";
pub(crate) const INK: &str = "#2e2a24";
pub(crate) const VIOLATION: &str = "#d7263d";
pub(crate) const TRIMMED: &str = "#e8820c";
pub(crate) const DROPPED: &str = "#c2189b";
/// The Neighbourhood layout's scaffolding: frames, green buffers, entrances.
const FRAME: &str = "#8a6d3b";
const GREEN: &str = "#cfe0b4";
/// Parks: a pale meadow with trees dotted over it.
const PARK: &str = "#dcebb8";
const TREE: &str = "#6f9a4e";
const ENTRANCE: &str = "#1f6f8b";

/// What a road is for, as the plan view colours it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Role {
    Arterial,
    Grid,
    Trail,
    Collector,
    CulDeSac,
    Loop,
    PStreet,
    Connector,
    Local,
}

impl Role {
    /// Streets are named by kind (`collector_`, `culdesac_`, ...).
    fn of(road: &RoadSegment) -> Role {
        match road.road_type {
            RoadType::Arterial => Role::Arterial,
            RoadType::Grid => Role::Grid,
            RoadType::Trail => Role::Trail,
            RoadType::Suburban => {
                let id = road.id.as_str();
                if id.starts_with("collector_") {
                    Role::Collector
                } else if id.starts_with("culdesac_") {
                    Role::CulDeSac
                } else if id.starts_with("loop_") {
                    Role::Loop
                } else if id.starts_with("pshaped_") {
                    Role::PStreet
                } else if id.contains("connector_") {
                    Role::Connector
                } else {
                    Role::Local
                }
            }
        }
    }

    fn colour(self) -> &'static str {
        match self {
            Role::Arterial => "#5b544c",
            Role::Grid => "#b9b1a3",
            Role::Trail => "#b79a6a",
            Role::Collector => "#d98a2b",
            Role::CulDeSac => "#3f78b0",
            Role::Loop => "#7a58b0",
            Role::PStreet => "#2e9a8a",
            Role::Connector => "#9a9a9a",
            Role::Local => "#6e8fb0",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Role::Arterial => "arterial",
            Role::Grid => "urban grid",
            Role::Trail => "trail",
            Role::Collector => "collector",
            Role::CulDeSac => "cul-de-sac",
            Role::Loop => "loop",
            Role::PStreet => "P-street",
            Role::Connector => "connector",
            Role::Local => "local",
        }
    }

    const LEGEND: [Role; 7] =
        [Role::Arterial, Role::Collector, Role::CulDeSac, Role::Loop, Role::PStreet, Role::Connector, Role::Local];
}

/// Everything one plan view draws.
pub struct View<'a> {
    pub title: String,
    pub roads: &'a [RoadSegment],
    pub rings: &'a [WaterRing],
    pub lots: &'a [TerrainPlot],
    pub before: &'a [TerrainPlot],
    pub cleared: &'a [Option<[P; 4]>],
    pub scope: Scope,
    pub metrics: &'a Metrics,
    pub markers: &'a [Marker],
    pub violations: &'a [Violation],
    /// The neighbourhoods, for the Neighbourhood layout's overlays.
    pub neighbourhoods: &'a [Neighbourhood],
    /// Each neighbourhood's ground, for its parks (#111).
    pub greens: &'a [Greens],
    /// Lots are shaded by area over this range: the p5 and p95 targets.
    pub area: (f64, f64),
    /// Lots that don't take this many stations are drawn in `SHORT`, so
    /// the plan shows where they are (8 for Plots v2's lots, #456).
    pub takes: Option<usize>,
}

fn hex(c: &str) -> (f64, f64, f64) {
    let v = |i: usize| u8::from_str_radix(&c[i..i + 2], 16).unwrap() as f64;
    (v(1), v(3), v(5))
}

pub(crate) fn mix(a: &str, b: &str, t: f64) -> String {
    let (a, b) = (hex(a), hex(b));
    let t = t.clamp(0.0, 1.0);
    let m = |x: f64, y: f64| (x + (y - x) * t).round() as u8;
    format!("#{:02x}{:02x}{:02x}", m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

fn lot_fill(area: f64, (lo, hi): (f64, f64)) -> String {
    mix("#e4ebcb", "#4b7a43", (area - lo) / (hi - lo))
}

pub(crate) fn pt(p: P) -> String {
    format!("{:.1},{:.1}", p.0, p.1)
}

pub(crate) fn poly(c: &[P]) -> String {
    c.iter().map(|&p| pt(p)).collect::<Vec<_>>().join(" ")
}

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The street edge of a lot, from its `street_facing_edge`.
fn street_edge(lot: &TerrainPlot, corners: &[P; 4]) -> (P, P) {
    (corners[lot.street_facing_edge.0], corners[lot.street_facing_edge.1])
}

pub fn svg(v: &View) -> String {
    let scope = v.scope;
    let pad = 60.0;
    let (lo, hi) = scope.bounds();
    let (lo, hi) = ((lo.0 - pad, lo.1 - pad), (hi.0 + pad, hi.1 + pad));
    let in_view = |p: P| p.0 > lo.0 - 200.0 && p.0 < hi.0 + 200.0 && p.1 > lo.1 - 200.0 && p.1 < hi.1 + 200.0;
    // World units per output pixel, for strokes and type that read the same at any zoom.
    let k = (hi.0 - lo.0) / (OUT_PX - LEGEND_PX);
    let lo = (lo.0 - LEGEND_PX * k, lo.1);
    let (w, h) = (hi.0 - lo.0, hi.1 - lo.1);
    let near_suburb = |p: P| scope.radius_of(p) < scope.outer + 300.0 && in_view(p);

    let mut s = String::new();
    let _ = writeln!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{:.1} {:.1} {:.1} {:.1}" font-family="Georgia, 'Times New Roman', serif">"##,
        OUT_PX,
        (OUT_PX * h / w).round(),
        lo.0,
        lo.1,
        w,
        h
    );
    let _ = writeln!(s, "<title>{}</title>", esc(&v.title));
    let _ = writeln!(s, r##"<rect x="{:.1}" y="{:.1}" width="{w:.1}" height="{h:.1}" fill="{PAPER}"/>"##, lo.0, lo.1);

    // Zones: the suburb a little lighter than the paper, its edges dashed.
    let (c, inner, outer) = (scope.center, scope.inner, scope.outer);
    let _ = writeln!(
        s,
        r##"<path d="M {x1:.1} {cy:.1} A {outer} {outer} 0 1 0 {x2:.1} {cy:.1} A {outer} {outer} 0 1 0 {x1:.1} {cy:.1} Z M {x3:.1} {cy:.1} A {inner} {inner} 0 1 1 {x4:.1} {cy:.1} A {inner} {inner} 0 1 1 {x3:.1} {cy:.1} Z" fill="#faf7ef" fill-rule="evenodd"/>"##,
        x1 = c.0 - outer,
        x2 = c.0 + outer,
        x3 = c.0 - inner,
        x4 = c.0 + inner,
        cy = c.1
    );
    for r in [inner, outer] {
        let _ = writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="{r}" fill="none" stroke="#b5a88c" stroke-width="{:.2}" stroke-dasharray="{:.1} {:.1}"/>"##,
            c.0,
            c.1,
            1.5 * k,
            10.0 * k,
            6.0 * k
        );
    }
    for ring in v.rings {
        let _ = writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="{}" fill="none" stroke="#9cc0d4" stroke-width="{}"/>"##,
            c.0, c.1, ring.radius, ring.width
        );
    }

    neighbourhood_ground(&mut s, v, k);

    // Lots: suburban ones shaded by area with the street edge darker, the
    // rest a quiet grey. Trimmed lots get an orange edge.
    let _ = writeln!(s, r##"<g stroke-linejoin="round">"##);
    let trimmed: std::collections::HashSet<[u64; 8]> = v
        .before
        .iter()
        .zip(v.cleared)
        .filter_map(|(b, c)| c.filter(|c| *c != b.corners))
        .map(|c| key(&c))
        .collect();
    for lot in v.lots.iter().filter(|l| near_suburb(metrics::centroid(&l.corners))) {
        if lot.zone != "suburban" {
            let _ = writeln!(
                s,
                r##"<polygon points="{}" fill="#e2dccd" stroke="#cfc6b2" stroke-width="{:.2}"/>"##,
                poly(&lot.corners),
                0.8 * k
            );
            continue;
        }
        let is_trimmed = trimmed.contains(&key(&lot.corners));
        let (stroke, sw) = if is_trimmed { (TRIMMED, 2.4 * k) } else { ("#87986b", 0.8 * k) };
        let short = v.takes.filter(|&n| !station_room::takes(&lot.corners, n));
        let _ = writeln!(
            s,
            r##"<polygon points="{}" fill="{}" stroke="{stroke}" stroke-width="{sw:.2}"><title>{} · area {:.0} · {:.0}×{:.0}{}{}</title></polygon>"##,
            poly(&lot.corners),
            if short.is_some() { SHORT.to_string() } else { lot_fill(lot.area, v.area) },
            esc(&lot.id),
            lot.area,
            metrics::width_at_mid_depth(lot),
            metrics::depth(lot),
            if is_trimmed { " · trimmed" } else { "" },
            short.map(|n| format!(" · takes under {n}")).unwrap_or_default()
        );
        let (a, b) = street_edge(lot, &lot.corners);
        let _ = writeln!(
            s,
            r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="#2f4a2a" stroke-width="{:.2}" stroke-linecap="round"/>"##,
            a.0,
            a.1,
            b.0,
            b.1,
            2.6 * k
        );
    }
    for (b, _) in v.before.iter().zip(v.cleared).filter(|(_, c)| c.is_none()) {
        if !near_suburb(metrics::centroid(&b.corners)) {
            continue;
        }
        let _ = writeln!(
            s,
            r##"<polygon points="{}" fill="{DROPPED}" fill-opacity="0.22" stroke="{DROPPED}" stroke-width="{:.2}" stroke-dasharray="{:.1} {:.1}"><title>{} · dropped</title></polygon>"##,
            poly(&b.corners),
            2.0 * k,
            6.0 * k,
            4.0 * k,
            esc(&b.id)
        );
    }
    let _ = writeln!(s, "</g>");

    // Roads at their true width, the minor ones under the major ones.
    let order = |r: &RoadSegment| match Role::of(r) {
        Role::Grid | Role::Trail => 0,
        Role::Arterial => 2,
        _ => 1,
    };
    // By extent, not by point: an arterial is two points, both outside a sector's view.
    let box_in_view = |r: &&RoadSegment| {
        let (x0, x1) = r.points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.0), b.max(p.0)));
        let (y0, y1) = r.points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.1), b.max(p.1)));
        x1 > lo.0 - 200.0 && x0 < hi.0 + 200.0 && y1 > lo.1 - 200.0 && y0 < hi.1 + 200.0
    };
    let mut roads: Vec<&RoadSegment> = v.roads.iter().filter(box_in_view).collect();
    roads.sort_by_key(|r| order(r));
    let _ = writeln!(s, r##"<g fill="none" stroke-linecap="round" stroke-linejoin="round">"##);
    for r in roads {
        let role = Role::of(r);
        let d: String = r
            .points
            .iter()
            .enumerate()
            .map(|(i, &p)| format!("{}{}", if i == 0 { "M" } else { " L" }, pt(p)))
            .collect();
        let dash = if role == Role::Trail { format!(r##" stroke-dasharray="{:.1} {:.1}""##, 8.0 * k, 6.0 * k) } else { String::new() };
        let _ = writeln!(
            s,
            r##"<path d="{d}" stroke="{}" stroke-width="{}"{dash}><title>{} · {} · w {}</title></path>"##,
            role.colour(),
            r.width,
            esc(&r.id),
            role.label(),
            r.width
        );
    }
    let _ = writeln!(s, "</g>");

    // Violations: a red ring at each place that breaks a target.
    let _ = writeln!(s, r##"<g fill="none" stroke="{VIOLATION}">"##);
    for m in v.markers {
        let _ = writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" stroke-width="{:.2}"><title>{}: {}</title></circle>"##,
            m.at.0,
            m.at.1,
            9.0 * k,
            2.6 * k,
            m.kind,
            esc(&m.note)
        );
    }
    let _ = writeln!(s, "</g>");

    neighbourhood_marks(&mut s, v, k);
    legend(&mut s, v, lo, k, h);
    s.push_str("</svg>\n");
    s
}

/// Under the lots: the green buffers, each frame's outline dashed, and its
/// rows dotted where the streets will run.
fn neighbourhood_ground(s: &mut String, v: &View, k: f64) {
    let in_scope = |n: &&Neighbourhood| v.scope.sector.is_none_or(|x| x == n.sector);
    let _ = writeln!(s, r##"<g stroke-linejoin="round">"##);
    for n in v.neighbourhoods.iter().filter(in_scope) {
        for b in &n.buffers {
            let label = match b.kind {
                BufferKind::Arterial => "arterial buffer",
                BufferKind::Greenway => "greenway",
                BufferKind::Edge => "edge strip",
            };
            let _ = writeln!(
                s,
                r##"<polygon points="{}" fill="{GREEN}" fill-opacity="{}"><title>s{}·{} {label}</title></polygon>"##,
                poly(&b.polygon),
                if b.kind == BufferKind::Edge { 0.45 } else { 0.85 },
                n.sector,
                n.index
            );
        }
    }
    // Parks, as runs of squares along each raster row, filled with trees.
    let _ = writeln!(
        s,
        r##"<defs><pattern id="trees" patternUnits="userSpaceOnUse" width="28" height="24"><rect width="28" height="24" fill="{PARK}"/><circle cx="7" cy="6" r="3.6" fill="{TREE}" fill-opacity="0.55"/><circle cx="21" cy="18" r="4.2" fill="{TREE}" fill-opacity="0.45"/></pattern></defs>"##
    );
    for g in v.greens.iter().filter(|g| v.scope.sector.is_none_or(|x| x == g.sector)) {
        let mut d = String::new();
        for r in 0..g.rows {
            let mut c = 0;
            while c < g.cols {
                if g.cells[r * g.cols + c] != Ground::Park {
                    c += 1;
                    continue;
                }
                let start = c;
                while c < g.cols && g.cells[r * g.cols + c] == Ground::Park {
                    c += 1;
                }
                let (x, y) = g.corner(start, r);
                let _ = write!(d, "M{x:.1} {y:.1}h{:.1}v{CELL}h{:.1}z", (c - start) as f64 * CELL, -((c - start) as f64) * CELL);
            }
        }
        if !d.is_empty() {
            let _ = writeln!(
                s,
                r##"<path d="{d}" fill="url(#trees)" shape-rendering="crispEdges"><title>s{}·{} parks: {}</title></path>"##,
                g.sector,
                g.nbhd,
                g.parks.len()
            );
        }
    }
    for n in v.neighbourhoods.iter().filter(in_scope) {
        let f = &n.frame;
        let _ = writeln!(
            s,
            r##"<polygon points="{}" fill="none" stroke="{FRAME}" stroke-width="{:.2}" stroke-dasharray="{:.1} {:.1}"/>"##,
            poly(&f.outline()),
            1.8 * k,
            12.0 * k,
            7.0 * k
        );
        for &row in f.fit.iter().flat_map(|fit| &fit.rows) {
            let _ = writeln!(
                s,
                r##"<polyline points="{}" fill="none" stroke="{FRAME}" stroke-opacity="0.55" stroke-width="{:.2}" stroke-dasharray="{:.1} {:.1}"/>"##,
                poly(&f.iso_v(row, 0.0, f.width)),
                1.2 * k,
                2.0 * k,
                5.0 * k
            );
        }
    }
    let _ = writeln!(s, "</g>");
}

/// Over the roads: entrances, and a label in the middle of each frame.
fn neighbourhood_marks(s: &mut String, v: &View, k: f64) {
    let in_scope = |n: &&Neighbourhood| v.scope.sector.is_none_or(|x| x == n.sector);
    for n in v.neighbourhoods.iter().filter(in_scope) {
        let f = &n.frame;
        for e in &n.entrances {
            let _ = writeln!(
                s,
                r##"<g stroke="{ENTRANCE}"><line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke-width="{:.2}"/><circle cx="{:.1}" cy="{:.1}" r="{:.1}" fill="{ENTRANCE}" stroke="none"><title>s{}·{} entrance, row {} · arterial {} at {:.0}</title></circle></g>"##,
                e.gate.0,
                e.gate.1,
                e.at.0,
                e.at.1,
                3.0 * k,
                e.at.0,
                e.at.1,
                7.0 * k,
                n.sector,
                n.index,
                e.row,
                e.arterial,
                e.along
            );
        }
        let c = f.to_world(f.width / 2.0, f.depth / 2.0);
        let fit = match &f.fit {
            Some(fit) => format!("{} rows · lots {:.0} deep", fit.rows.len(), fit.lot_depth),
            None => "no rows fit".to_string(),
        };
        for (dy, size, weight, body) in
            [(-10.0, 22.0, "bold", format!("s{}·{}", n.sector, n.index)), (16.0, 16.0, "normal", fit)]
        {
            let _ = writeln!(
                s,
                r##"<text x="{:.1}" y="{:.1}" font-size="{:.1}" font-weight="{weight}" fill="{FRAME}" text-anchor="middle" stroke="{PAPER}" stroke-width="{:.1}" paint-order="stroke">{}</text>"##,
                c.0,
                c.1 + dy * k,
                size * k,
                4.0 * k,
                esc(&body)
            );
        }
    }
}

/// A stable key for a lot's corners, to match a finished lot to its trimmed shape.
fn key(c: &[P; 4]) -> [u64; 8] {
    std::array::from_fn(|i| if i % 2 == 0 { c[i / 2].0.to_bits() } else { c[i / 2].1.to_bits() })
}

/// The key and the headline numbers, in a column on the left.
fn legend(s: &mut String, v: &View, lo: P, k: f64, h: f64) {
    let m = v.metrics;
    let x = lo.0;
    let mut y = lo.1 + 24.0 * k;
    let line = 26.0 * k;
    let panel_w = LEGEND_PX * k;
    let _ = writeln!(
        s,
        r##"<g><rect x="{x:.1}" y="{:.1}" width="{panel_w:.1}" height="{h:.1}" fill="{PAPER}"/><line x1="{x2:.1}" y1="{:.1}" x2="{x2:.1}" y2="{:.1}" stroke="#c9bea5" stroke-width="{:.2}"/>"##,
        lo.1,
        lo.1,
        lo.1 + h,
        1.2 * k,
        x2 = x + panel_w
    );
    let tx = x + 28.0 * k;
    y += 44.0 * k;
    let text = |s: &mut String, y: f64, size: f64, weight: &str, fill: &str, body: &str| {
        let _ = writeln!(
            s,
            r##"<text x="{tx:.1}" y="{y:.1}" font-size="{:.1}" font-weight="{weight}" fill="{fill}">{}</text>"##,
            size * k,
            esc(body)
        );
    };
    text(s, y, 30.0, "bold", INK, &v.title);
    y += line;
    text(
        s,
        y,
        17.0,
        "normal",
        "#6b6253",
        &format!(
            "{} lots · {} streets · {} T · {} 4-way · {} cul-de-sacs",
            m.lots, m.suburban_roads, m.tees, m.four_ways, m.culdesacs
        ),
    );
    y += line * 1.4;

    for role in Role::LEGEND {
        let _ = writeln!(
            s,
            r##"<line x1="{tx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="{:.1}" stroke-linecap="round"/>"##,
            y - 6.0 * k,
            tx + 34.0 * k,
            y - 6.0 * k,
            role.colour(),
            9.0 * k
        );
        let _ = writeln!(
            s,
            r##"<text x="{:.1}" y="{y:.1}" font-size="{:.1}" fill="{INK}">{}</text>"##,
            tx + 48.0 * k,
            17.0 * k,
            role.label()
        );
        y += line;
    }

    // Lot area ramp.
    let steps = 8;
    for i in 0..steps {
        let t = i as f64 / (steps - 1) as f64;
        let _ = writeln!(
            s,
            r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{}"/>"##,
            tx + i as f64 * 26.0 * k,
            y - 18.0 * k,
            26.0 * k,
            16.0 * k,
            lot_fill(v.area.0 + t * (v.area.1 - v.area.0), v.area)
        );
    }
    let _ = writeln!(
        s,
        r##"<text x="{:.1}" y="{y:.1}" font-size="{:.1}" fill="{INK}">lot area {}–{}</text>"##,
        tx + (steps as f64 * 26.0 + 14.0) * k,
        17.0 * k,
        fmt_num(v.area.0),
        fmt_num(v.area.1)
    );
    y += line;
    if let Some(n) = v.takes {
        let _ = writeln!(
            s,
            r##"<rect x="{tx:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{SHORT}"/><text x="{:.1}" y="{y:.1}" font-size="{:.1}" fill="{INK}">takes under {n} stations</text>"##,
            y - 18.0 * k,
            26.0 * k,
            16.0 * k,
            tx + 40.0 * k,
            17.0 * k,
        );
        y += line;
    }
    for (colour, label, dashed) in [(TRIMMED, "trimmed by finish_lots", false), (DROPPED, "dropped by finish_lots", true)] {
        let dash = if dashed { format!(r##" stroke-dasharray="{:.1} {:.1}""##, 5.0 * k, 3.0 * k) } else { String::new() };
        let _ = writeln!(
            s,
            r##"<rect x="{tx:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{colour}" fill-opacity="0.2" stroke="{colour}" stroke-width="{:.1}"{dash}/>"##,
            y - 18.0 * k,
            34.0 * k,
            16.0 * k,
            2.4 * k
        );
        let _ = writeln!(
            s,
            r##"<text x="{:.1}" y="{y:.1}" font-size="{:.1}" fill="{INK}">{label}</text>"##,
            tx + 48.0 * k,
            17.0 * k
        );
        y += line;
    }
    if !v.neighbourhoods.is_empty() {
        let swatch = |s: &mut String, y: f64, body: &str| {
            let _ = writeln!(
                s,
                r##"<text x="{:.1}" y="{y:.1}" font-size="{:.1}" fill="{INK}">{}</text>"##,
                tx + 48.0 * k,
                17.0 * k,
                esc(body)
            );
        };
        let _ = writeln!(
            s,
            r##"<line x1="{tx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{FRAME}" stroke-width="{:.1}" stroke-dasharray="{:.1} {:.1}"/>"##,
            y - 10.0 * k,
            tx + 34.0 * k,
            y - 10.0 * k,
            1.8 * k,
            6.0 * k,
            4.0 * k
        );
        swatch(s, y, "neighbourhood frame, rows dotted");
        y += line;
        let _ = writeln!(
            s,
            r##"<rect x="{tx:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{GREEN}"/>"##,
            y - 18.0 * k,
            34.0 * k,
            16.0 * k
        );
        swatch(s, y, "buffer, greenway, edge strip");
        y += line;
        let _ = writeln!(
            s,
            r##"<rect x="{tx:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="url(#trees)"/>"##,
            y - 18.0 * k,
            34.0 * k,
            16.0 * k
        );
        let parks: usize = v.greens.iter().map(|g| g.parks.len()).sum();
        swatch(s, y, &format!("park ({parks})"));
        y += line;
        let _ = writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" fill="{ENTRANCE}"/>"##,
            tx + 17.0 * k,
            y - 10.0 * k,
            7.0 * k
        );
        let count: usize = v
            .neighbourhoods
            .iter()
            .filter(|n| v.scope.sector.is_none_or(|x| x == n.sector))
            .map(|n| n.entrances.len())
            .sum();
        swatch(s, y, &format!("entrance ({count})"));
        y += line;
    }
    let _ = writeln!(
        s,
        r##"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" fill="none" stroke="{VIOLATION}" stroke-width="{:.1}"/>"##,
        tx + 17.0 * k,
        y - 10.0 * k,
        9.0 * k,
        2.6 * k
    );
    let _ = writeln!(
        s,
        r##"<text x="{:.1}" y="{y:.1}" font-size="{:.1}" fill="{INK}">breaks a target ({} places)</text>"##,
        tx + 48.0 * k,
        17.0 * k,
        v.markers.len()
    );
    y += line * 1.4;

    if v.violations.is_empty() {
        text(s, y, 17.0, "normal", "#3d7a3a", "every target met");
    }
    for viol in v.violations {
        let fill = if viol.hard { VIOLATION } else { "#8a4b2a" };
        let weight = if viol.hard { "bold" } else { "normal" };
        let body = format!("✗ {}: {} (target {})", viol.label, fmt_num(viol.value), viol.target);
        text(s, y, 15.0, weight, fill, &body);
        y += line * 0.85;
    }
    let _ = writeln!(s, "</g>");
}

/// The markdown table: a row per target, a column per plan.
pub fn table(plans: &[(String, Metrics, Vec<Violation>)], targets: &[&Target]) -> String {
    let mut s = String::new();
    let _ = write!(s, "| metric | target |");
    for (name, ..) in plans {
        let _ = write!(s, " {name} |");
    }
    let _ = write!(s, "\n|---|---|");
    for _ in plans {
        s.push_str("---:|");
    }
    s.push('\n');
    let info = [
        ("lots", "suburban lots"),
        ("lots_before", "lots before finish_lots"),
        ("suburban_roads", "suburban streets"),
        ("junctions", "junctions"),
        ("tees", "T junctions"),
        ("culdesacs", "cul-de-sacs"),
        ("parallel_samples", "parallel spacing samples"),
    ];
    for (metric, label) in info {
        let _ = write!(s, "| {label} | |");
        for (_, m, _) in plans {
            let _ = write!(s, " {} |", m.value(metric).map(fmt_num).unwrap_or_else(|| "–".into()));
        }
        s.push('\n');
    }
    for t in targets {
        let label = if t.hard { format!("**{}**", t.label) } else { t.label.to_string() };
        let _ = write!(s, "| {label} | {} |", t.check.describe());
        for (_, m, viol) in plans {
            let cell = match m.value(t.metric) {
                None => "–".to_string(),
                Some(v) if viol.iter().any(|x| x.metric == t.metric) => format!("{} ✗", fmt_num(v)),
                Some(v) => fmt_num(v),
            };
            let _ = write!(s, " {cell} |");
        }
        s.push('\n');
    }
    let _ = write!(s, "| entrances by neighbourhood | |");
    for (_, m, _) in plans {
        let _ = write!(s, " {} |", m.entrances.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(" "));
    }
    s.push('\n');
    s
}

/// A page with the table and every plan view, in a grid.
pub fn index_html(plans: &[(String, Metrics, Vec<Violation>)], targets: &[&Target]) -> String {
    let names: Vec<&str> = plans.iter().map(|(name, ..)| name.as_str()).collect();
    page("Suburb plan", &table(plans, targets), &names)
}

/// A page titled `title`, with the markdown table `md` and the plan views
/// `names` (`{name}.png`, or the SVG where there's no PNG), in a grid. The
/// rural report (#284) shares it.
pub(crate) fn page(title: &str, md: &str, names: &[&str]) -> String {
    let mut rows = String::new();
    for line in md.lines().filter(|l| !l.starts_with("|---")) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let tag = if rows.is_empty() { "th" } else { "td" };
        rows.push_str("<tr>");
        for c in cells {
            let hard = c.starts_with("**");
            let c = esc(c.trim_matches('*'));
            let class = if c.ends_with('✗') { r##" class="miss""## } else { "" };
            let c = if hard { format!("<b>{c}</b>") } else { c };
            let _ = write!(rows, "<{tag}{class}>{c}</{tag}>");
        }
        rows.push_str("</tr>\n");
    }
    let mut figures = String::new();
    for name in names {
        let _ = write!(
            figures,
            r##"<figure><a href="{name}.svg"><img src="{name}.png" onerror="this.onerror=null;this.src='{name}.svg'" alt="{name}"></a><figcaption>{name}</figcaption></figure>"##
        );
    }
    format!(
        r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ --paper: {PAPER}; --ink: {INK}; --rule: #d8cfbb; --miss: {VIOLATION}; }}
body {{ margin: 0; padding: 24px 16px 48px; background: var(--paper); color: var(--ink); font: 15px/1.45 Georgia, serif; }}
h1 {{ font-weight: normal; letter-spacing: .02em; margin: 0 0 16px; }}
.wrap {{ overflow-x: auto; }}
table {{ border-collapse: collapse; margin-bottom: 32px; font-variant-numeric: tabular-nums; }}
th, td {{ padding: 4px 12px; border-bottom: 1px solid var(--rule); text-align: right; }}
th:first-child, td:first-child, td:nth-child(2) {{ text-align: left; }}
td.miss {{ color: var(--miss); }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 560px), 1fr)); gap: 24px; }}
figure {{ margin: 0; }}
img {{ width: 100%; height: auto; display: block; border: 1px solid var(--rule); }}
figcaption {{ margin-top: 6px; color: #6b6253; }}
</style></head>
<body>
<h1>{title}</h1>
<div class="wrap"><table>
{rows}</table></div>
<div class="grid">{figures}</div>
</body></html>
"##
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::models::suburb::metrics::{Measurement, TARGETS};
    use crate::models::suburb::tests::grown_world;
    use crate::models::terrain::{NeighbourhoodParams, NeighbourhoodPattern, SuburbanGenParams, WorldGenParams};
    use capitol_protocol::rules::{SUBURBAN_OUTER_RADIUS, URBAN_OUTER_RADIUS};

    fn env_or(name: &str, default: &str) -> String {
        std::env::var(name).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default.to_string())
    }

    /// Writes the plan view, metrics and tables for each seed. Run it with
    /// `backend/tools/suburb-plan.ps1`, or:
    /// `cargo test --release suburb_plan_report -- --ignored --nocapture`.
    ///
    /// Env: `SUBURB_SEEDS` (comma-separated, default 42,7,1234,99999),
    /// `SUBURB_SECTOR` (0–7, one sector only),
    /// `SUBURB_INNER` and `SUBURB_OUTER` (the suburb's radii, to try other depths; #117),
    /// `SUBURB_PARAMS` (`today`, the default, or `grown`: Plots v2's lots,
    /// `NeighbourhoodParams::grown`, at r2160 to r3600 unless the radii say otherwise; #456),
    /// `SUBURB_PATTERN` (`warped_parallels`, `loops_and_lollipops` or `central_loop`: force one
    /// wherever it fits; #109) and `SUBURB_OUT` (default `target/suburb-plan/` at the workspace root).
    #[test]
    #[ignore]
    fn suburb_plan_report() {
        let seeds: Vec<u64> = env_or("SUBURB_SEEDS", "42,7,1234,99999")
            .split(',')
            .map(|s| s.trim().parse().expect("SUBURB_SEEDS is a list of numbers"))
            .collect();
        // The plan's name in files and titles. `neighbourhood` is the one
        // layout since #113 removed Legacy, and keeps old runs comparable.
        let layout = "neighbourhood".to_string();
        let pattern: Option<NeighbourhoodPattern> = std::env::var("SUBURB_PATTERN")
            .ok()
            .map(|v| v.trim().to_lowercase())
            .filter(|v| !v.is_empty())
            .map(|v| {
                serde_json::from_value(serde_json::Value::String(v.clone()))
                    .unwrap_or_else(|_| panic!("SUBURB_PATTERN={v}: warped_parallels, loops_and_lollipops or central_loop"))
            });
        // The layout's name in files and titles, with the forced pattern.
        let layout = match pattern {
            Some(p) => format!("{layout}-{}", serde_json::to_value(p).unwrap().as_str().unwrap().replace('_', "-")),
            None => layout,
        };
        let sector: Option<u8> = std::env::var("SUBURB_SECTOR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|v| v.trim().parse().ok().filter(|&s: &u8| s < 8).expect("SUBURB_SECTOR is 0 to 7"));
        let out = PathBuf::from(env_or(
            "SUBURB_OUT",
            &format!("{}/../target/suburb-plan", env!("CARGO_MANIFEST_DIR")),
        ));
        std::fs::create_dir_all(&out).expect("create SUBURB_OUT");
        let radius = |name: &str, default: f64| -> f64 {
            env_or(name, &default.to_string()).trim().parse().unwrap_or_else(|_| panic!("{name} is a number"))
        };
        let grown = match env_or("SUBURB_PARAMS", "today").trim().to_lowercase().as_str() {
            "today" => false,
            "grown" => true,
            v => panic!("SUBURB_PARAMS={v}: today or grown"),
        };
        let defaults = if grown { grown_world(0) } else { WorldGenParams::default() };
        let inner = radius("SUBURB_INNER", defaults.urban_outer_radius);
        let outer = radius("SUBURB_OUTER", defaults.suburban_outer_radius);
        let layout = if grown { format!("{layout}-grown") } else { layout };
        let targets = metrics::targets(grown);
        assert!(
            defaults.guild_outer_radius < inner && inner < outer && outer < defaults.rural_outer_radius,
            "SUBURB_INNER and SUBURB_OUTER must sit between the guild district and the rural edge"
        );

        let mut plans = Vec::new();
        for seed in seeds {
            let params = WorldGenParams {
                seed,
                urban_outer_radius: inner,
                suburban_outer_radius: outer,
                suburban: SuburbanGenParams {
                    neighbourhood: NeighbourhoodParams { force_pattern: pattern, ..defaults.suburban.neighbourhood.clone() },
                    ..Default::default()
                },
                ..defaults.clone()
            };
            let Measurement { terrain: t, before, cleared, neighbourhoods, greens, scope, measured } =
                metrics::measure_world(&params, sector);
            let kept = t.terrain_plots.iter().filter(|l| l.zone == "suburban").count();
            assert_eq!(cleared.iter().flatten().count(), kept, "seed {seed}: the lots before finish_lots don't match");
            let mut m = measured.metrics;
            m.layout = layout.clone();
            let gen_ms = m.gen_ms;
            let viol = metrics::violations_of(&m, &targets);

            let name = match sector {
                Some(s) => format!("{layout}-{seed}-s{s}"),
                None => format!("{layout}-{seed}"),
            };
            let title = match sector {
                Some(s) => format!("{} · seed {seed} · sector {s}", capitalise(&layout)),
                None => format!("{} · seed {seed}", capitalise(&layout)),
            };
            let view = View {
                title,
                roads: &t.roads,
                rings: &t.water_rings,
                lots: &t.terrain_plots,
                before: &before,
                cleared: &cleared,
                scope,
                metrics: &m,
                markers: &measured.markers,
                violations: &viol,
                neighbourhoods: &neighbourhoods,
                greens: &greens,
                area: if grown { (10000.0, 28000.0) } else { (AREA_LO, AREA_HI) },
                takes: grown.then_some(6),
            };
            std::fs::write(out.join(format!("{name}.svg")), svg(&view)).unwrap();
            let json = serde_json::json!({ "metrics": m, "violations": viol, "markers": measured.markers });
            std::fs::write(out.join(format!("{name}.json")), serde_json::to_string_pretty(&json).unwrap()).unwrap();
            eprintln!(
                "{name}: {} lots, {} violations ({} hard), {} markers, {gen_ms:.0} ms",
                m.lots,
                viol.len(),
                viol.iter().filter(|v| v.hard).count(),
                measured.markers.len()
            );
            plans.push((name, m, viol));
        }

        let table = table(&plans, &targets);
        std::fs::write(out.join("report.md"), &table).unwrap();
        std::fs::write(out.join("index.html"), index_html(&plans, &targets)).unwrap();
        eprintln!("\n{table}\nwrote {}", out.display());
    }

    fn capitalise(s: &str) -> String {
        let mut c = s.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    }

    #[test]
    fn the_plan_view_draws_a_small_plan() {
        let road = |id: &str, road_type, points: Vec<P>, width| RoadSegment { id: id.into(), road_type, points, width };
        let roads = vec![
            road("arterial_0", RoadType::Arterial, vec![(URBAN_OUTER_RADIUS, 0.0), (SUBURBAN_OUTER_RADIUS, 0.0)], 60.0),
            road("culdesac_1", RoadType::Suburban, vec![(2000.0, 0.0), (2000.0, 100.0)], 16.0),
        ];
        let lot = TerrainPlot {
            id: "l".into(),
            corners: [(2008.0, 40.0), (2008.0, 100.0), (2118.0, 100.0), (2118.0, 40.0)],
            street_facing_edge: (0, 1),
            area: 6600.0,
            adjacent_street_id: "culdesac_1".into(),
            zone: "suburban".into(),
        };
        let lots = vec![lot];
        let cleared = vec![Some(lots[0].corners)];
        let scope = Scope { center: (0.0, 0.0), inner: URBAN_OUTER_RADIUS, outer: SUBURBAN_OUTER_RADIUS, sector: Some(0) };
        let measured = metrics::measure(&metrics::Plan { roads: &roads, lots: &lots, before: &lots, cleared: &cleared, scope, coverage: true, greens: &[], streets: &[], culdesac_max: 300.0 });
        let viol = metrics::violations(&measured.metrics);
        let view = View {
            title: "Test".into(),
            roads: &roads,
            rings: &[],
            lots: &lots,
            before: &lots,
            cleared: &cleared,
            scope,
            metrics: &measured.metrics,
            markers: &measured.markers,
            violations: &viol,
            neighbourhoods: &[],
            greens: &[],
            area: (AREA_LO, AREA_HI),
            takes: None,
        };
        let doc = svg(&view);
        assert!(doc.starts_with("<svg") && doc.trim_end().ends_with("</svg>"));
        assert!(doc.contains("culdesac_1 · cul-de-sac"));
        assert!(doc.contains(Role::Arterial.colour()));
        let md = table(&[("t".into(), measured.metrics, viol)], &metrics::targets(false));
        assert_eq!(md.lines().count(), 2 + 7 + TARGETS.len() + 1);
    }
}
