//! The urban plan view (#421): the ring from the outer canal to the suburb's
//! edge, and a close-up of one sector, with the report test that writes them.
//! `backend/tools/urban-plan.ps1` runs it in release and turns the SVGs into
//! PNGs. Server y points down, as SVG's does, so world coordinates go in as
//! they are.
//!
//! Lots are shaded by area. One that doesn't take 4 stations is red, and one
//! `finish_lots` trimmed has an orange edge. The street edge is drawn darker.

use std::fmt::Write as _;

use capitol_protocol::terrain::{RoadType, TerrainPlot};

use super::metrics::Metrics;
use crate::models::station_room;
use crate::models::suburb::render::{esc, poly, INK, PAPER, TRIMMED, VIOLATION};
use crate::models::terrain::TerrainConfig;

type P = (f64, f64);

const OUT_PX: f64 = 2400.0;
const WATER: &str = "#6f9fc4";
const BANK: &str = "#dfe6c8";
const ARTERIAL: &str = "#5b544c";
const STREET: &str = "#9a8f7c";
const OTHER_LOT: &str = "#e6dfcf";
/// Lots shade from this area to the next.
const AREA_LO: f64 = 6_000.0;
const AREA_HI: f64 = 11_000.0;

fn lot_fill(area: f64) -> String {
    let t = ((area - AREA_LO) / (AREA_HI - AREA_LO)).clamp(0.0, 1.0);
    let mix = |a: f64, b: f64| (a + (b - a) * t).round() as u8;
    format!("#{:02x}{:02x}{:02x}", mix(214.0, 120.0), mix(196.0, 92.0), mix(160.0, 70.0))
}

/// The plan of `terrain` between `lo` and `hi`, in world units.
pub fn svg(title: &str, terrain: &TerrainConfig, metrics: &Metrics, laid: &[TerrainPlot], lo: P, hi: P) -> String {
    let (w, h) = (hi.0 - lo.0, hi.1 - lo.1);
    // World units per output pixel, so strokes read the same at any zoom.
    let k = w / OUT_PX;
    let in_view = |c: &[P]| c.iter().any(|p| p.0 > lo.0 - 100.0 && p.0 < hi.0 + 100.0 && p.1 > lo.1 - 100.0 && p.1 < hi.1 + 100.0);
    let center = (terrain.center_x, terrain.center_y);
    let mut s = String::new();
    let _ = writeln!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{OUT_PX}" height="{}" viewBox="{:.1} {:.1} {w:.1} {h:.1}" font-family="Georgia, 'Times New Roman', serif">"##,
        (OUT_PX * h / w).round(),
        lo.0,
        lo.1,
    );
    let _ = writeln!(s, "<title>{}</title>", esc(title));
    let _ = writeln!(s, r##"<rect x="{:.1}" y="{:.1}" width="{w:.1}" height="{h:.1}" fill="{PAPER}"/>"##, lo.0, lo.1);

    // The urban ring's ground, then the water rings over it.
    if let Some(canal) = terrain.water_rings.iter().find(|r| r.id == "canal_outer") {
        let r = (canal.outer_radius() + capitol_protocol::rules::URBAN_OUTER_RADIUS) / 2.0;
        let width = capitol_protocol::rules::URBAN_OUTER_RADIUS - canal.outer_radius();
        let _ = writeln!(s, r##"<circle cx="{:.1}" cy="{:.1}" r="{r:.1}" fill="none" stroke="{BANK}" stroke-width="{width:.1}"/>"##, center.0, center.1);
    }
    for ring in &terrain.water_rings {
        let _ = writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" fill="none" stroke="{WATER}" stroke-width="{:.1}"><title>{}</title></circle>"##,
            center.0,
            center.1,
            ring.radius,
            ring.width,
            esc(&ring.id)
        );
    }

    // Lots: other zones' quietly, the urban ones by area and room.
    let _ = writeln!(s, r##"<g stroke-linejoin="round">"##);
    for lot in terrain.terrain_plots.iter().filter(|l| in_view(&l.corners)) {
        if lot.zone != "urban" {
            let _ = writeln!(s, r##"<polygon points="{}" fill="{OTHER_LOT}"/>"##, poly(&lot.corners));
            continue;
        }
        let four = station_room::takes(&lot.corners, 4);
        let trimmed = laid.iter().find(|l| l.id == lot.id).is_none_or(|l| l.corners != lot.corners);
        let _ = writeln!(
            s,
            r##"<polygon points="{}" fill="{}" stroke="{}" stroke-width="{:.2}"><title>{} · {:.0}{}</title></polygon>"##,
            poly(&lot.corners),
            if four { lot_fill(lot.area) } else { VIOLATION.to_string() },
            if trimmed { TRIMMED } else { PAPER },
            if trimmed { 3.0 * k } else { 0.8 * k },
            esc(&lot.id),
            lot.area,
            if four { "" } else { " · takes under 4" }
        );
        let (a, b) = (lot.corners[lot.street_facing_edge.0], lot.corners[lot.street_facing_edge.1]);
        let _ = writeln!(s, r##"<polyline points="{}" fill="none" stroke="{INK}" stroke-width="{:.2}"/>"##, poly(&[a, b]), 1.6 * k);
    }
    let _ = writeln!(s, "</g>");

    // Roads at their width.
    let _ = writeln!(s, r##"<g fill="none" stroke-linecap="round" stroke-linejoin="round">"##);
    for road in terrain.roads.iter().filter(|r| in_view(&r.points)) {
        let colour = if road.road_type == RoadType::Arterial { ARTERIAL } else { STREET };
        let _ = writeln!(
            s,
            r##"<polyline points="{}" stroke="{colour}" stroke-width="{:.1}"><title>{}</title></polyline>"##,
            poly(&road.points),
            road.width,
            esc(&road.id)
        );
    }
    let _ = writeln!(s, "</g>");

    // The numbers, top left.
    let lines = [
        title.to_string(),
        format!("{} lots · area median {:.0} (p5 {:.0}, p95 {:.0})", metrics.lots, metrics.area_median, metrics.area_p5, metrics.area_p95),
        format!("width median {:.0} (least {:.0}) · depth median {:.0}", metrics.width_median, metrics.width_min, metrics.depth_median),
        format!("{:.0}% take 4 · {:.0}% of the ring built · trimmed {} · dropped {}", 100.0 * metrics.takes_four_share, 100.0 * metrics.cover, metrics.trimmed, metrics.dropped),
    ];
    for (i, line) in lines.iter().enumerate() {
        let _ = writeln!(
            s,
            r##"<text x="{:.1}" y="{:.1}" font-size="{:.1}" fill="{INK}" stroke="{PAPER}" stroke-width="{:.1}" paint-order="stroke">{}</text>"##,
            lo.0 + 24.0 * k,
            lo.1 + (44.0 + 34.0 * i as f64) * k,
            if i == 0 { 32.0 * k } else { 24.0 * k },
            5.0 * k,
            esc(line)
        );
    }
    let _ = writeln!(s, "</svg>");
    s
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::models::suburb::render::page;
    use crate::models::terrain::WorldGenParams;

    /// Writes the whole ring and a close-up of the sector from 0° to 45°,
    /// with the metrics. Run it with `backend/tools/urban-plan.ps1`, or
    /// `cargo test --release urban_plan_report -- --ignored --nocapture`.
    /// Env: `URBAN_SEED` (default 42; the grid has no seed, but the world
    /// round it does) and `URBAN_OUT` (default `target/urban-plan/`).
    #[test]
    #[ignore]
    fn urban_plan_report() {
        let seed: u64 = std::env::var("URBAN_SEED").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(42);
        let out = std::env::var("URBAN_OUT")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("{}/../target/urban-plan", env!("CARGO_MANIFEST_DIR"))));
        std::fs::create_dir_all(&out).expect("create URBAN_OUT");
        let params = WorldGenParams { seed, ..Default::default() };
        let (terrain, m) = super::super::metrics::measure(&params);
        let laid = super::super::grid(&params, (terrain.center_x, terrain.center_y), &terrain.roads).lots(&params.urban);
        let c = (terrain.center_x, terrain.center_y);
        let r = params.urban_outer_radius + 100.0;
        let views = [
            ("urban", format!("Urban · seed {seed}"), (c.0 - r, c.1 - r), (c.0 + r, c.1 + r)),
            ("urban-sector", format!("Urban · seed {seed} · 0° to 45°"), (c.0 + 900.0, c.1 - 150.0), (c.0 + 1900.0, c.1 + 1400.0)),
        ];
        for (name, title, lo, hi) in &views {
            std::fs::write(out.join(format!("{name}.svg")), svg(title, &terrain, &m, &laid, *lo, *hi)).unwrap();
        }
        std::fs::write(out.join("urban.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
        let table = format!(
            "| lots | area median | p5 | p95 | width median | least | depth median | take 4 | built | trimmed | dropped |\n\
             |---|---|---|---|---|---|---|---|---|---|---|\n\
             | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0}% | {:.0}% | {} | {} |\n",
            m.lots,
            m.area_median,
            m.area_p5,
            m.area_p95,
            m.width_median,
            m.width_min,
            m.depth_median,
            100.0 * m.takes_four_share,
            100.0 * m.cover,
            m.trimmed,
            m.dropped
        );
        std::fs::write(out.join("report.md"), &table).unwrap();
        std::fs::write(out.join("index.html"), page("Urban plan", &table, &["urban", "urban-sector"])).unwrap();
        eprintln!("\n{table}\nwrote {}", out.display());
    }
}
