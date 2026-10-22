//! The urban ring, from the outer canal to r1800: a planned grid of
//! workshop yards a step from the market (#421, plots plan §5.1).
//!
//! From the canal outward:
//! - **the bank**, `bank` of open ground along the water, where the clay is;
//! - **the quay** (`urb_ring_0`), the first ring street;
//! - **three rings of blocks**, each two rows of lots back to back between
//!   ring streets (`urb_ring_1` to `urb_ring_3`), so every lot fronts a
//!   street and backs onto its neighbour's yard;
//! - **the spokes** (`urb_spoke_{j}`) cut the rings into blocks every 15°,
//!   from the quay to the outer ring street. Every third bearing is an
//!   arterial, which is the spoke there.
//!
//! Before #421 the grid was arcs every 100 and 48 spokes, with lots of 40 ×
//! 35 cut from the blocks: 3008 lots, each taking one station. Now about 560
//! lots of about 85 × 100 each take 4 (`station_room::takes`).
//!
//! The lots are laid exactly clear of the streets, so `finish_lots` has
//! nothing to trim:
//! - a lot's side on a spoke or an arterial runs parallel to it, `CLEAR`
//!   past its kerb;
//! - lots along a row share radial sides, the row's width split evenly;
//! - a lot's edge at the smaller radius is a chord, which sags toward the
//!   centre, so its corners stand out by `1 / cos(half the lot's angle)`.
//!   Its edge at the larger radius sags into the lot itself.
//!
//! The grid has no seed: it's the same in every world. Its streets meet at
//! shared vertices (`street_plan::audit`), by construction. Where a ring
//! street crosses an arterial it has a vertex on the arterial's centre line,
//! but the arterial isn't welded to it: the farms and suburban lots laid
//! along an arterial follow its vertices, and a weld moved 137 of them
//! (#436).

#[cfg(test)]
pub(crate) mod metrics;
#[cfg(test)]
mod render;

use std::f64::consts::TAU;

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot};
use serde::{Deserialize, Serialize};

use crate::models::terrain::WorldGenParams;

type P = (f64, f64);

/// How far a lot stays back from a street's kerb.
const CLEAR: f64 = 1.0;

/// Each ring street's segments, per block between spokes. 8 keeps a
/// segment under 60 and its sag under a quarter unit at r1750.
const SEGMENTS_PER_BLOCK: u32 = 8;

/// The urban grid's params. Every field has a default, so a saved
/// `world_config` from before them still loads. The old grid's
/// (`urban_radial_spacing`, `urban_spoke_count`, `urban_street_width`,
/// `urban_plot_frontage`, `urban_plot_depth`) are skipped by serde (#421).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UrbanParams {
    /// From the outer canal's water to the quay's kerb: the canal bank. 80
    /// leaves the clay its strip, 18 to 48 off the water, 30 clear of the quay.
    pub bank: f64,
    pub street_width: f64,
    /// Each lot's depth, from its street's kerb to the one it backs onto.
    pub lot_depth: f64,
    /// Rings of blocks. One that won't fit inside the zone isn't laid.
    pub rings: u32,
    /// Bearings round the ring, the arterials among them. Every 15° at 24.
    pub spokes: u32,
    /// The narrowest a lot is. A row is split into as many lots as fit at
    /// this width along its inner edge, the narrow end of every lot in it.
    pub lot_frontage: f64,
}

impl Default for UrbanParams {
    fn default() -> Self {
        Self { bank: 80.0, street_width: 16.0, lot_depth: 95.0, rings: 3, spokes: 24, lot_frontage: 80.0 }
    }
}

/// Whether `road` is one of the grid's streets.
#[cfg(test)]
pub fn is_grid_street(road: &RoadSegment) -> bool {
    road.id.starts_with("urb_")
}

/// The grid's layout: the ring streets' radii, inside out, and the
/// bearings of its spokes and arterials, ascending in `[0, TAU)`.
#[derive(Debug, Clone)]
pub struct Grid {
    pub center: P,
    pub streets: Vec<f64>,
    /// Every 360°/`spokes`, as `(bearing, half width of the road there)`.
    pub bearings: Vec<(f64, f64)>,
    /// The bearings that are arterials: no spoke is laid on them.
    pub arterial: Vec<bool>,
    pub street_width: f64,
}

/// Lays the grid out. `roads` needs only the arterials.
pub fn grid(params: &WorldGenParams, center: P, roads: &[RoadSegment]) -> Grid {
    let u = &params.urban;
    let canal_edge = params.guild_outer_radius + params.canal_outer_width / 2.0;
    let half = u.street_width / 2.0;
    let mut streets = vec![canal_edge + u.bank + half];
    for _ in 0..u.rings {
        let next = streets[streets.len() - 1] + u.street_width + 2.0 * u.lot_depth;
        if next + half > params.urban_outer_radius {
            break;
        }
        streets.push(next);
    }
    let arterials = super::rural::sectors::arterial_bearings(roads, center, canal_edge);
    let on_arterial = |a: f64| {
        arterials.iter().any(|&b| {
            let d = (a - b).rem_euclid(TAU);
            d.min(TAU - d) < 1e-6
        })
    };
    let n = u.spokes.max(1);
    let (bearings, arterial) = (0..n)
        .map(|j| {
            let a = j as f64 * TAU / n as f64;
            let art = on_arterial(a);
            ((a, if art { params.arterial_width / 2.0 } else { half }), art)
        })
        .unzip();
    Grid { center, streets, bearings, arterial, street_width: u.street_width }
}

impl Grid {
    fn polar(&self, r: f64, a: f64) -> P {
        (self.center.0 + r * a.stable_cos(), self.center.1 + r * a.stable_sin())
    }

    /// The ring streets and the spokes.
    pub fn roads(&self) -> Vec<RoadSegment> {
        let mut roads = Vec::new();
        if self.streets.is_empty() {
            return roads;
        }
        let n = self.bearings.len() as u32 * SEGMENTS_PER_BLOCK;
        for (k, &r) in self.streets.iter().enumerate() {
            // Closed: the last point is the first, so the vertices at the
            // spokes' bearings are exact.
            let points = (0..=n).map(|i| self.polar(r, (i % n) as f64 * TAU / n as f64)).collect();
            roads.push(RoadSegment { id: format!("urb_ring_{k}"), road_type: RoadType::Grid, points, width: self.street_width });
        }
        for (j, &(a, _)) in self.bearings.iter().enumerate() {
            if self.arterial[j] || self.streets.len() < 2 {
                continue;
            }
            let points = self.streets.iter().map(|&r| self.polar(r, a)).collect();
            roads.push(RoadSegment { id: format!("urb_spoke_{j}"), road_type: RoadType::Grid, points, width: self.street_width });
        }
        roads
    }

    /// The lots: `urb_{ring}_{block}_{i|o}_{k}`, the inner row (`i`) fronting
    /// ring street `ring`, the outer (`o`) fronting `ring + 1`, `k` along
    /// the row anticlockwise. Each lot's street edge is its first two corners.
    pub fn lots(&self, params: &UrbanParams) -> Vec<TerrainPlot> {
        let half = self.street_width / 2.0;
        let mut lots = Vec::new();
        let m = self.bearings.len();
        for ring in 0..self.streets.len().saturating_sub(1) {
            let (front_in, front_out) = (self.streets[ring] + half + CLEAR, self.streets[ring + 1] - half - CLEAR);
            let mid = (self.streets[ring] + self.streets[ring + 1]) / 2.0;
            for block in 0..m {
                let (a1, h1) = self.bearings[block];
                let (mut a2, h2) = self.bearings[(block + 1) % m];
                if a2 <= a1 {
                    a2 += TAU;
                }
                // The block's sides at radius r, each CLEAR past its road's kerb.
                let sides = |r: f64| (a1 + ((h1 + CLEAR) / r).stable_asin(), a2 - ((h2 + CLEAR) / r).stable_asin());
                for (row, inner, outer) in [('i', front_in, mid), ('o', mid, front_out)] {
                    // As many as fit at the row's narrow end, its inner edge.
                    let (n1, n2) = sides(inner);
                    let count = ((inner * (n2 - n1) / params.lot_frontage).floor() as usize).max(1);
                    // The inner edge's chord sags inward by 1 - cos(half its
                    // angle), so its corners stand out by that much.
                    let (i1, i2) = sides(inner);
                    let inner = inner / ((i2 - i1) / count as f64 / 2.0).stable_cos();
                    let (i1, i2) = sides(inner);
                    let (o1, o2) = sides(outer);
                    let at = |r: f64, lo: f64, hi: f64, f: f64| self.polar(r, lo + (hi - lo) * f);
                    for k in 0..count {
                        let (f0, f1) = (k as f64 / count as f64, (k + 1) as f64 / count as f64);
                        let (in0, in1) = (at(inner, i1, i2, f0), at(inner, i1, i2, f1));
                        let (out0, out1) = (at(outer, o1, o2, f0), at(outer, o1, o2, f1));
                        // The street edge first, the same way round in both rows.
                        let (corners, street) = match row {
                            'i' => ([in0, in1, out1, out0], ring),
                            _ => ([out1, out0, in0, in1], ring + 1),
                        };
                        lots.push(TerrainPlot {
                            id: format!("urb_{ring}_{block}_{row}_{k}"),
                            corners,
                            street_facing_edge: (0, 1),
                            area: TerrainPlot::calculate_area(&corners),
                            adjacent_street_id: format!("urb_ring_{street}"),
                            zone: "urban".to_string(),
                        });
                    }
                }
            }
        }
        lots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worlds the targets hold over: the default and three more seeds.
    /// The grid has no seed, but the world round it does.
    fn worlds() -> Vec<WorldGenParams> {
        [42, 7, 1234, 99_999].map(|seed| WorldGenParams { seed, ..WorldGenParams::default() }).into()
    }

    /// The plots plan's §3 targets for the urban ring (#421): 500 to 700
    /// lots of 7k to 9k, nine in ten taking 4 stations at 50 spacing, laid
    /// clear of every road so `finish_lots` trims and drops none, on
    /// streets that meet at shared vertices.
    #[test]
    fn the_urban_grid_meets_its_targets() {
        for params in worlds() {
            let (_, m) = metrics::measure(&params);
            eprintln!("seed {}: {m:?}", params.seed);
            let why = format!("seed {}: {m:#?}", params.seed);
            assert!((500..=700).contains(&m.lots), "{why}");
            assert!((7000.0..=9000.0).contains(&m.area_median), "{why}");
            assert!(m.takes_four_share >= 0.90, "{why}");
            assert_eq!((m.trimmed, m.dropped), (0, 0), "{why}");
            assert_eq!(m.audit_issues, 0, "{why}");
        }
    }

    /// A saved `world_config` from before #421 names the old grid's keys.
    /// They're skipped, and it builds the new grid.
    #[test]
    fn a_saved_config_with_the_old_grid_builds_the_new_one() {
        let mut json = serde_json::to_value(WorldGenParams::default()).unwrap();
        let fields = json.as_object_mut().unwrap();
        fields.remove("urban");
        for (key, value) in [
            ("urban_radial_spacing", 100.0),
            ("urban_spoke_count", 48.0),
            ("urban_street_width", 16.0),
            ("urban_plot_frontage", 40.0),
            ("urban_plot_depth", 50.0),
        ] {
            fields.insert(key.into(), value.into());
        }
        let old: WorldGenParams = serde_json::from_value(json).unwrap();
        assert_eq!(old.urban, UrbanParams::default());
    }
}
