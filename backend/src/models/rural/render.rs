//! The rural plan view: one SVG per seed, with its metrics, and the report
//! test that writes them (#284). `backend/tools/rural-plan.ps1` runs it in
//! release and turns the SVGs into PNGs.
//!
//! It draws the ring r3000 to r3800 with its half-sectors shaded by built
//! share, the plain's water, the lots and the roads, and rings the places
//! that break a target. Server y points down, as SVG's does, so world
//! coordinates go in as they are.

use std::fmt::Write as _;

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot};

use super::metrics::{self, half_sectors, is_rural_road, Metrics, Waters, HALF_SECTOR_DEG, TARGETS};
use crate::models::suburb::metrics::{centroid, fmt_num, Marker, Scope, Violation};
use crate::models::suburb::render::{esc, mix, poly, pt, DROPPED, INK, PAPER, TRIMMED, VIOLATION};

type P = (f64, f64);

/// Output width of every SVG, in pixels. A sector view keeps its aspect.
const OUT_PX: f64 = 2400.0;
/// The key takes a column this wide on the left, so it never covers the plan.
const LEGEND_PX: f64 = 520.0;
/// Room round the ring for the half-sectors' labels, in world units.
const MARGIN: f64 = 260.0;
/// Lots are shaded from this area (today's lot is 200×150, about 29k)...
const AREA_LO: f64 = 20_000.0;
/// ...to this one, the top of the plan's farm.
const AREA_HI: f64 = 140_000.0;
/// Half-sectors are shaded from empty to this share, the plan's most.
const SHARE_HI: f64 = 0.55;
/// The plan's least share for a half-sector.
const SHARE_LO: f64 = 0.20;

const MEADOW: &str = "#e4ebcb";
const FARM: &str = "#4b7a43";
const STUBBLE: &str = "#f7f2e4";
const WHEAT: &str = "#d9bf7f";
const WATER: &str = "#6f9fc4";
const LAKE: &str = "#a9cbe0";
const ARTERIAL: &str = "#5b544c";
const LANE: &str = "#8b5e2f";
const WILD: &str = "#a59a86";
const STREET: &str = "#d4cbb8";
const LOT_ELSEWHERE: &str = "#e6dfcf";

/// Everything one plan view draws.
pub struct View<'a> {
    pub title: String,
    pub roads: &'a [RoadSegment],
    pub lots: &'a [TerrainPlot],
    pub before: &'a [TerrainPlot],
    pub cleared: &'a [Option<[P; 4]>],
    pub water: Waters<'a>,
    pub scope: Scope,
    pub metrics: &'a Metrics,
    pub markers: &'a [Marker],
    pub violations: &'a [Violation],
}

fn lot_fill(area: f64) -> String {
    mix(MEADOW, FARM, (area - AREA_LO) / (AREA_HI - AREA_LO))
}

fn share_fill(share: f64) -> String {
    mix(STUBBLE, WHEAT, share / SHARE_HI)
}

fn polar(scope: &Scope, r: f64, deg: f64) -> P {
    let a = deg.to_radians();
    (scope.center.0 + r * a.stable_cos(), scope.center.1 + r * a.stable_sin())
}

/// An annular wedge from `a0` to `a1` degrees, as an SVG path.
fn wedge(scope: &Scope, a0: f64, a1: f64) -> String {
    let (i0, i1) = (polar(scope, scope.inner, a0), polar(scope, scope.inner, a1));
    let (o0, o1) = (polar(scope, scope.outer, a0), polar(scope, scope.outer, a1));
    let (ri, ro) = (scope.inner, scope.outer);
    format!("M{} L{} A{ro} {ro} 0 0 1 {} L{} A{ri} {ri} 0 0 0 {} Z", pt(i0), pt(o0), pt(o1), pt(i1), pt(i0))
}

/// A stable key for a lot's corners, to match a finished lot to its trimmed shape.
fn key(c: &[P; 4]) -> [u64; 8] {
    std::array::from_fn(|i| if i % 2 == 0 { c[i / 2].0.to_bits() } else { c[i / 2].1.to_bits() })
}

pub fn svg(v: &View) -> String {
    let scope = v.scope;
    let (lo, hi) = scope.bounds();
    let (lo, hi) = ((lo.0 - MARGIN, lo.1 - MARGIN), (hi.0 + MARGIN, hi.1 + MARGIN));
    let in_view = |p: P| p.0 > lo.0 - 200.0 && p.0 < hi.0 + 200.0 && p.1 > lo.1 - 200.0 && p.1 < hi.1 + 200.0;
    // World units per output pixel, for strokes and type that read the same at any zoom.
    let k = (hi.0 - lo.0) / (OUT_PX - LEGEND_PX);
    let lo = (lo.0 - LEGEND_PX * k, lo.1);
    let (w, h) = (hi.0 - lo.0, hi.1 - lo.1);

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

    // The half-sectors, shaded by how much of each is built on. One outside
    // the plan's range gets a red edge.
    let wedges = half_sectors(&scope);
    let mut labels = String::new();
    let _ = writeln!(s, r##"<g stroke-linejoin="round">"##);
    for (&hs, &share) in wedges.iter().zip(&v.metrics.half_sector_shares) {
        let (a0, a1) = (hs as f64 * HALF_SECTOR_DEG, (hs + 1) as f64 * HALF_SECTOR_DEG);
        let miss = !(SHARE_LO..=SHARE_HI).contains(&share);
        let _ = writeln!(
            s,
            r##"<path d="{}" fill="{}" stroke="{}" stroke-width="{:.2}"><title>half-sector {hs} · {:.1}% built</title></path>"##,
            wedge(&scope, a0, a1),
            share_fill(share),
            if hs % 2 == 0 { "#b5a88c" } else { "#d3c8b0" },
            if hs % 2 == 0 { 1.6 * k } else { 0.9 * k },
            share * 100.0
        );
        if miss {
            let _ = writeln!(
                s,
                r##"<path d="{}" fill="none" stroke="{VIOLATION}" stroke-opacity="0.55" stroke-width="{:.2}" stroke-dasharray="{:.1} {:.1}"/>"##,
                wedge(&scope, a0 + 0.25, a1 - 0.25),
                2.0 * k,
                10.0 * k,
                6.0 * k
            );
        }
        // Written after the water and the roads, with a halo, so they read.
        let at = polar(&scope, scope.outer + 120.0, (a0 + a1) / 2.0);
        let _ = writeln!(
            labels,
            r##"<text x="{:.1}" y="{:.1}" font-size="{:.1}" text-anchor="middle" dominant-baseline="middle" fill="{}" stroke="{PAPER}" stroke-width="{:.1}" paint-order="stroke"{}>{:.0}%</text>"##,
            at.0,
            at.1,
            22.0 * k,
            if miss { VIOLATION } else { INK },
            5.0 * k,
            if miss { r#" font-weight="bold""# } else { "" },
            share * 100.0
        );
    }
    let _ = writeln!(s, "</g>");

    // Lots outside the ring, quietly, so the suburb's edge reads.
    let _ = writeln!(s, r##"<g fill="{LOT_ELSEWHERE}" stroke="none">"##);
    for lot in v.lots.iter().filter(|l| l.zone != "rural" && in_view(centroid(&l.corners))) {
        let _ = writeln!(s, r##"<polygon points="{}"/>"##, poly(&lot.corners));
    }
    let _ = writeln!(s, "</g>");

    // The water: lakes, then rivers and streams at their width.
    let _ = writeln!(s, r##"<g fill="{LAKE}" stroke="{WATER}" stroke-width="{:.2}" fill-rule="evenodd">"##, 1.0 * k);
    for lake in v.water.lakes.iter().filter(|l| l.rings.iter().flatten().any(|&p| in_view(p))) {
        let d: String = lake.rings.iter().map(|r| format!("M{}Z", r.iter().map(|&p| pt(p)).collect::<Vec<_>>().join("L"))).collect();
        let _ = writeln!(s, r##"<path d="{d}"><title>{} · area {:.0}</title></path>"##, esc(&lake.id), lake.area);
    }
    let _ = writeln!(s, "</g>");
    let _ = writeln!(s, r##"<g fill="none" stroke="{WATER}" stroke-linecap="round" stroke-linejoin="round">"##);
    for course in v.water.rivers.iter().chain(v.water.streams) {
        // In runs of points in view, each as wide as its water on average.
        let mut run: Vec<P> = Vec::new();
        let mut widths = 0.0;
        let flush = |s: &mut String, run: &mut Vec<P>, widths: &mut f64| {
            if run.len() > 1 {
                let wide = (*widths / run.len() as f64).max(1.4 * k);
                let _ = writeln!(s, r##"<polyline points="{}" stroke-width="{wide:.1}"><title>{}</title></polyline>"##, poly(run), esc(&course.id));
            }
            run.clear();
            *widths = 0.0;
        };
        for p in &course.points {
            if in_view((p.x, p.y)) {
                run.push((p.x, p.y));
                widths += p.width;
            } else {
                flush(&mut s, &mut run, &mut widths);
            }
        }
        flush(&mut s, &mut run, &mut widths);
    }
    let _ = writeln!(s, "</g>");

    // Rural lots shaded by area with the street edge darker. Trimmed ones
    // get an orange edge, and dropped ones are dashed where they were.
    let trimmed: std::collections::HashSet<[u64; 8]> = v
        .before
        .iter()
        .zip(v.cleared)
        .filter_map(|(b, c)| c.filter(|c| *c != b.corners))
        .map(|c| key(&c))
        .collect();
    let _ = writeln!(s, r##"<g stroke-linejoin="round">"##);
    for lot in v.lots.iter().filter(|l| l.zone == "rural" && in_view(centroid(&l.corners))) {
        let is_trimmed = trimmed.contains(&key(&lot.corners));
        let (stroke, sw) = if is_trimmed { (TRIMMED, 2.6 * k) } else { ("#5d7a4a", 1.0 * k) };
        let _ = writeln!(
            s,
            r##"<polygon points="{}" fill="{}" stroke="{stroke}" stroke-width="{sw:.2}"><title>{} · area {:.0} · on {}{}</title></polygon>"##,
            poly(&lot.corners),
            lot_fill(lot.area),
            esc(&lot.id),
            lot.area,
            esc(&lot.adjacent_street_id),
            if is_trimmed { " · trimmed" } else { "" }
        );
        let (a, b) = (lot.corners[lot.street_facing_edge.0], lot.corners[lot.street_facing_edge.1]);
        let _ = writeln!(
            s,
            r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="#2f4a2a" stroke-width="{:.2}" stroke-linecap="round"/>"##,
            a.0,
            a.1,
            b.0,
            b.1,
            3.0 * k
        );
    }
    for (b, _) in v.before.iter().zip(v.cleared).filter(|(b, c)| c.is_none() && in_view(centroid(&b.corners))) {
        let _ = writeln!(
            s,
            r##"<polygon points="{}" fill="{DROPPED}" fill-opacity="0.18" stroke="{DROPPED}" stroke-width="{:.2}" stroke-dasharray="{:.1} {:.1}"><title>{} · dropped</title></polygon>"##,
            poly(&b.corners),
            2.0 * k,
            6.0 * k,
            4.0 * k,
            esc(&b.id)
        );
    }
    let _ = writeln!(s, "</g>");

    // Roads: the suburb's and the bazaar's faint, the wild trails dashed, the
    // ring's own trails strong, and the arterials over all.
    let box_in_view = |r: &&RoadSegment| {
        let (x0, x1) = r.points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.0), b.max(p.0)));
        let (y0, y1) = r.points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.1), b.max(p.1)));
        x1 > lo.0 - 200.0 && x0 < hi.0 + 200.0 && y1 > lo.1 - 200.0 && y0 < hi.1 + 200.0
    };
    let order = |r: &RoadSegment| match r.road_type {
        RoadType::Arterial => 3,
        _ if is_rural_road(r) => 2,
        RoadType::Trail => 1,
        _ => 0,
    };
    let mut roads: Vec<&RoadSegment> = v.roads.iter().filter(box_in_view).collect();
    roads.sort_by_key(|r| order(r));
    let _ = writeln!(s, r##"<g fill="none" stroke-linecap="round" stroke-linejoin="round">"##);
    for r in roads {
        let d: String =
            r.points.iter().enumerate().map(|(i, &p)| format!("{}{}", if i == 0 { "M" } else { " L" }, pt(p))).collect();
        let (colour, width, dash) = match order(r) {
            3 => (ARTERIAL, r.width, String::new()),
            2 => (LANE, r.width.max(3.0 * k), String::new()),
            1 => (WILD, r.width.max(1.6 * k), format!(r##" stroke-dasharray="{:.1} {:.1}""##, 7.0 * k, 5.0 * k)),
            _ => (STREET, r.width, String::new()),
        };
        let _ = writeln!(
            s,
            r##"<path d="{d}" stroke="{colour}" stroke-width="{width:.1}"{dash}><title>{} · w {}</title></path>"##,
            esc(&r.id),
            r.width
        );
    }
    let _ = writeln!(s, "</g>");

    // The ring's edges, dashed over everything but the marks.
    for r in [scope.inner, scope.outer] {
        let _ = writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="{r}" fill="none" stroke="#8f8370" stroke-width="{:.2}" stroke-dasharray="{:.1} {:.1}"/>"##,
            scope.center.0,
            scope.center.1,
            1.4 * k,
            10.0 * k,
            6.0 * k
        );
    }

    s.push_str(&labels);

    // Violations: a red ring at each place that breaks a target. Lots too
    // close together get a smaller one, so the water and the audit stand out.
    let _ = writeln!(s, r##"<g fill="none" stroke="{VIOLATION}">"##);
    for m in v.markers.iter().filter(|m| m.kind != "share") {
        let small = m.kind == "gap";
        let _ = writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" stroke-width="{:.2}"{}><title>{}: {}</title></circle>"##,
            m.at.0,
            m.at.1,
            if small { 6.0 * k } else { 13.0 * k },
            if small { 1.6 * k } else { 2.8 * k },
            if small { r#" stroke-opacity="0.7""# } else { "" },
            m.kind,
            esc(&m.note)
        );
    }
    let _ = writeln!(s, "</g>");

    legend(&mut s, v, lo, k, h);
    s.push_str("</svg>\n");
    s
}

/// The key and the headline numbers, in a column on the left.
fn legend(s: &mut String, v: &View, lo: P, k: f64, h: f64) {
    let m = v.metrics;
    let x = lo.0;
    let line = 28.0 * k;
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
    let mut y = lo.1 + 68.0 * k;
    let text = |s: &mut String, y: f64, size: f64, weight: &str, fill: &str, body: &str| {
        let _ = writeln!(
            s,
            r##"<text x="{tx:.1}" y="{y:.1}" font-size="{:.1}" font-weight="{weight}" fill="{fill}">{}</text>"##,
            size * k,
            esc(body)
        );
    };
    let label = |s: &mut String, y: f64, body: &str| {
        let _ = writeln!(
            s,
            r##"<text x="{:.1}" y="{y:.1}" font-size="{:.1}" fill="{INK}">{}</text>"##,
            tx + 50.0 * k,
            18.0 * k,
            esc(body)
        );
    };
    text(s, y, 32.0, "bold", INK, &v.title);
    y += line;
    let per = |v: Option<f64>| v.map_or("–".to_string(), |v| format!("{:.0}%", v * 100.0));
    text(
        s,
        y,
        18.0,
        "normal",
        "#6b6253",
        &format!("{} lots · {} rural roads · {} built", m.lots, m.rural_roads, per(m.built_share)),
    );
    y += line * 0.85;
    text(s, y, 18.0, "normal", "#6b6253", &format!("{} courses and {} lakes in the ring", m.courses, m.lakes));
    y += line * 1.5;

    for (colour, width, dash, body) in [
        (ARTERIAL, 10.0, false, "arterial"),
        (LANE, 4.0, false, "rural road (trail, lane)"),
        (WILD, 2.5, true, "wild trail"),
        (STREET, 5.0, false, "suburban street"),
        (WATER, 5.0, false, "river or stream"),
    ] {
        let dash = if dash { format!(r##" stroke-dasharray="{:.1} {:.1}""##, 7.0 * k, 5.0 * k) } else { String::new() };
        let _ = writeln!(
            s,
            r##"<line x1="{tx:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{colour}" stroke-width="{:.1}" stroke-linecap="round"{dash}/>"##,
            y - 6.0 * k,
            tx + 36.0 * k,
            y - 6.0 * k,
            width * k
        );
        label(s, y, body);
        y += line;
    }
    let swatch = |s: &mut String, y: f64, fill: &str, stroke: &str, dashed: bool| {
        let dash = if dashed { format!(r##" stroke-dasharray="{:.1} {:.1}""##, 5.0 * k, 3.0 * k) } else { String::new() };
        let _ = writeln!(
            s,
            r##"<rect x="{tx:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{:.1}"{dash}/>"##,
            y - 18.0 * k,
            36.0 * k,
            17.0 * k,
            1.6 * k
        );
    };
    swatch(s, y, LAKE, WATER, false);
    label(s, y, "lake or pond");
    y += line * 1.3;

    // The two ramps.
    let ramp = |s: &mut String, y: f64, fill: &dyn Fn(f64) -> String, body: &str| {
        let steps = 8;
        for i in 0..steps {
            let _ = writeln!(
                s,
                r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{}"/>"##,
                tx + i as f64 * 24.0 * k,
                y - 18.0 * k,
                24.0 * k,
                17.0 * k,
                fill(i as f64 / (steps - 1) as f64)
            );
        }
        let _ = writeln!(
            s,
            r##"<text x="{:.1}" y="{y:.1}" font-size="{:.1}" fill="{INK}">{}</text>"##,
            tx + (steps as f64 * 24.0 + 14.0) * k,
            18.0 * k,
            esc(body)
        );
    };
    ramp(s, y, &|t| lot_fill(AREA_LO + t * (AREA_HI - AREA_LO)), &format!("lot area {}–{}", fmt_num(AREA_LO), fmt_num(AREA_HI)));
    y += line;
    ramp(s, y, &|t| share_fill(t * SHARE_HI), "half-sector built 0–55%");
    y += line;
    swatch(s, y, "none", VIOLATION, true);
    label(s, y, "half-sector outside 20–55%");
    y += line;
    for (colour, body, dashed) in [(TRIMMED, "trimmed by finish_lots", false), (DROPPED, "dropped by finish_lots", true)] {
        swatch(s, y, &format!("{colour}33"), colour, dashed);
        label(s, y, body);
        y += line;
    }
    let _ = writeln!(
        s,
        r##"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" fill="none" stroke="{VIOLATION}" stroke-width="{:.1}"/>"##,
        tx + 18.0 * k,
        y - 10.0 * k,
        10.0 * k,
        2.6 * k
    );
    let places = v.markers.iter().filter(|m| m.kind != "share").count();
    label(s, y, &format!("breaks a target ({places} places)"));
    y += line * 1.5;

    if v.violations.is_empty() {
        text(s, y, 18.0, "normal", "#3d7a3a", "every target met");
    }
    for viol in v.violations {
        let fill = if viol.hard { VIOLATION } else { "#8a4b2a" };
        let weight = if viol.hard { "bold" } else { "normal" };
        let body = format!("✗ {}: {} (target {})", viol.label, fmt_num(viol.value), viol.target);
        text(s, y, 16.0, weight, fill, &body);
        y += line * 0.85;
    }
    let _ = writeln!(s, "</g>");
}

/// The markdown table: a row per metric, a column per plan.
pub fn table(plans: &[(String, Metrics, Vec<Violation>)]) -> String {
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
        ("rural_roads", "rural roads"),
        ("road_length", "rural road length in the ring"),
        ("loose_ends", "loose road ends"),
        ("lots_before", "lots before finish_lots"),
        ("trimmed_share", "lots trimmed by finish_lots"),
        ("dropped_share", "lots dropped by finish_lots"),
        ("lot_area_min", "lot area, least"),
        ("lot_area_max", "lot area, most"),
        ("gap_nearest_median", "gap to the nearest lot, median"),
        ("overlaps", "lot-lot overlaps"),
        ("water_clearance_min", "closest a dry lot comes to water"),
        ("courses", "rivers and streams in the ring"),
        ("lakes", "lakes in the ring"),
        ("gen_ms", "generation (ms)"),
    ];
    for (metric, label) in info {
        let _ = write!(s, "| {label} | |");
        for (_, m, _) in plans {
            let _ = write!(s, " {} |", m.value(metric).map(fmt_num).unwrap_or_else(|| "–".into()));
        }
        s.push('\n');
    }
    for t in TARGETS {
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
    let _ = write!(s, "| built share by half-sector (%) | |");
    for (_, m, _) in plans {
        let shares: Vec<String> = m.half_sector_shares.iter().map(|v| format!("{:.0}", v * 100.0)).collect();
        let _ = write!(s, " {} |", shares.join(" "));
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::models::suburb::render::page;
    use crate::models::terrain::WorldGenParams;

    fn env_or(name: &str, default: &str) -> String {
        std::env::var(name).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default.to_string())
    }

    /// Writes the plan view, metrics and tables for each seed. Run it with
    /// `backend/tools/rural-plan.ps1`, or:
    /// `cargo test --release rural_plan_report -- --ignored --nocapture`.
    ///
    /// Env: `RURAL_SEEDS` (comma-separated, default 42,7,1234,99999),
    /// `RURAL_SECTOR` (0–7, one sector only) and `RURAL_OUT` (default
    /// `target/rural-plan/` at the workspace root).
    #[test]
    #[ignore]
    fn rural_plan_report() {
        let seeds: Vec<u64> = env_or("RURAL_SEEDS", "42,7,1234,99999")
            .split(',')
            .map(|s| s.trim().parse().expect("RURAL_SEEDS is a list of numbers"))
            .collect();
        // The layout's name in files and titles. Today's radial trails are the
        // only one until R3 (#286) adds the farms.
        let layout = "trails";
        let sector: Option<u8> = std::env::var("RURAL_SECTOR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|v| v.trim().parse().ok().filter(|&s: &u8| s < 8).expect("RURAL_SECTOR is 0 to 7"));
        let out = PathBuf::from(env_or("RURAL_OUT", &format!("{}/../target/rural-plan", env!("CARGO_MANIFEST_DIR"))));
        std::fs::create_dir_all(&out).expect("create RURAL_OUT");

        let mut plans = Vec::new();
        for seed in seeds {
            let params = WorldGenParams { seed, ..Default::default() };
            let measured = metrics::measure_world(&params, sector);
            let t = &measured.terrain;
            let kept = t.terrain_plots.iter().filter(|l| l.zone == "rural").count();
            assert_eq!(measured.cleared.iter().flatten().count(), kept, "seed {seed}: the lots before finish_lots don't match");
            let m = Metrics { layout: layout.into(), ..measured.measured.metrics };
            let viol = metrics::violations(&m);
            let (name, title) = match sector {
                Some(s) => (format!("{layout}-{seed}-s{s}"), format!("Rural {layout} · seed {seed} · sector {s}")),
                None => (format!("{layout}-{seed}"), format!("Rural {layout} · seed {seed}")),
            };
            let view = View {
                title,
                roads: &t.roads,
                lots: &t.terrain_plots,
                before: &measured.before,
                cleared: &measured.cleared,
                water: Waters { rivers: &t.rivers, streams: &t.streams, lakes: &t.lakes },
                scope: measured.scope,
                metrics: &m,
                markers: &measured.measured.markers,
                violations: &viol,
            };
            std::fs::write(out.join(format!("{name}.svg")), svg(&view)).unwrap();
            let json = serde_json::json!({ "metrics": m, "violations": viol, "markers": measured.measured.markers });
            std::fs::write(out.join(format!("{name}.json")), serde_json::to_string_pretty(&json).unwrap()).unwrap();
            eprintln!(
                "{name}: {} lots, {} violations ({} hard), {} markers, {:.0} ms",
                m.lots,
                viol.len(),
                viol.iter().filter(|v| v.hard).count(),
                measured.measured.markers.len(),
                m.gen_ms
            );
            plans.push((name, m, viol));
        }

        let table = table(&plans);
        std::fs::write(out.join("report.md"), &table).unwrap();
        let names: Vec<&str> = plans.iter().map(|(name, ..)| name.as_str()).collect();
        std::fs::write(out.join("index.html"), page("Rural plan", &table, &names)).unwrap();
        eprintln!("\n{table}\nwrote {}", out.display());
    }

    #[test]
    fn the_rural_plan_view_draws_a_small_ring() {
        let road = |id: &str, road_type, points: Vec<P>, width| RoadSegment { id: id.into(), road_type, points, width };
        let roads = vec![
            road("arterial_0", RoadType::Arterial, vec![(3000.0, 0.0), (3800.0, 0.0)], 60.0),
            road("trail_0", RoadType::Trail, vec![(3000.0, 300.0), (3780.0, 330.0)], 6.0),
        ];
        let corners = [(3200.0, 310.0), (3350.0, 315.0), (3345.0, 465.0), (3195.0, 460.0)];
        let lots = vec![TerrainPlot {
            id: "rural_plot_trail_0_R_0".into(),
            corners,
            street_facing_edge: (0, 1),
            area: TerrainPlot::calculate_area(&corners),
            adjacent_street_id: "trail_0".into(),
            zone: "rural".into(),
        }];
        let cleared = vec![Some(corners)];
        let scope = Scope { center: (0.0, 0.0), inner: 3000.0, outer: 3800.0, sector: Some(0) };
        let water = Waters { rivers: &[], streams: &[], lakes: &[] };
        let plan = metrics::Plan { roads: &roads, lots: &lots, before: &lots, cleared: &cleared, water, scope };
        let measured = metrics::measure(&plan);
        let viol = metrics::violations(&measured.metrics);
        let view = View {
            title: "Test".into(),
            roads: &roads,
            lots: &lots,
            before: &lots,
            cleared: &cleared,
            water,
            scope,
            metrics: &measured.metrics,
            markers: &measured.markers,
            violations: &viol,
        };
        let doc = svg(&view);
        assert!(doc.starts_with("<svg") && doc.trim_end().ends_with("</svg>"));
        assert!(doc.contains("trail_0 · w 6"));
        assert!(doc.contains("half-sector 1 · 0.0% built"));
        assert!(doc.contains(ARTERIAL));
        let md = table(&[("t".into(), measured.metrics, viol)]);
        assert_eq!(md.lines().count(), 2 + 14 + TARGETS.len() + 1);
    }
}
