//! The urban ring, from the outer canal to r2160: a planned grid of
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
//! Plots v2 (#455) grew it to five rings of blocks, with the urban edge at
//! r2160 (r1800 and three rings before the flip, #459): about 1000 lots.
//! Out there a block between spokes every 15° is
//! 540 long, so the rings of blocks past r1800 can take bearings of their
//! own (`outer_spokes`, 32 in v2). The ring street where the two sets meet
//! has a vertex at every bearing of both: the inner spokes end on it and
//! the outer ones start from it, each a T, and the arterials, in both sets,
//! run on through.
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
//! and the weld puts the same point into the arterial (#436). The lots laid
//! along an arterial follow its shape, not its vertices, so none moved.

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
    /// Bearings round the outer rings of blocks, those whose inner ring
    /// street stands past `outer_past`, so their blocks aren't as long as
    /// 24 bearings make them out there (#455). 0, the default, keeps
    /// `spokes` all the way out. Where the two sets meet, on the ring
    /// street between, the inner spokes end and the outer ones start, each
    /// a T onto it. Like `spokes`, a multiple of 8, so the arterials are
    /// among its bearings: a lot laid across an arterial is trimmed.
    pub outer_spokes: u32,
    pub outer_past: f64,
    /// The narrowest a lot is. A row is split into as many lots as fit at
    /// this width along its inner edge, the narrow end of every lot in it.
    pub lot_frontage: f64,
}

impl Default for UrbanParams {
    fn default() -> Self {
        Self {
            bank: 80.0,
            street_width: 16.0,
            lot_depth: 95.0,
            // Five rings of blocks, the outer ring street at r2125.5 under the
            // edge at r2160 (Plots v2, #455, #459; three before).
            rings: 5,
            spokes: 24,
            // 32 bearings round the rings of blocks past r1800, so no block
            // is much over 500 long: at 24 they'd be 540 out there. Their
            // cross streets meet the ring street at r1919.5 as T-junctions,
            // offset from the inner spokes (Chris, 2026-10-22).
            outer_spokes: 32,
            outer_past: 1800.0,
            lot_frontage: 80.0,
        }
    }
}

/// Whether `road` is one of the grid's streets.
pub fn is_grid_street(road: &RoadSegment) -> bool {
    road.id.starts_with("urb_")
}

/// One set of bearings round the ring.
#[derive(Debug, Clone)]
pub struct Bearings {
    /// Every 360°/n, ascending in `[0, TAU)`, as `(bearing, half width of
    /// the road there)`.
    pub at: Vec<(f64, f64)>,
    /// The bearings that are arterials: no spoke is laid on them.
    pub arterial: Vec<bool>,
}

/// The grid's layout: the ring streets' radii, inside out, and the
/// bearings of its spokes and arterials.
#[derive(Debug, Clone)]
pub struct Grid {
    pub center: P,
    pub streets: Vec<f64>,
    /// The bearings of the rings of blocks inside `outer_from`.
    pub inner: Bearings,
    /// The bearings of the rings of blocks from `outer_from` out (#455):
    /// `inner`'s, with `outer_from` past the last, when the params name no
    /// outer spokes.
    pub outer: Bearings,
    pub outer_from: usize,
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
    let bearings = |n: u32| {
        let n = n.max(1);
        let (at, arterial) = (0..n)
            .map(|j| {
                let a = j as f64 * TAU / n as f64;
                let art = on_arterial(a);
                ((a, if art { params.arterial_width / 2.0 } else { half }), art)
            })
            .unzip();
        Bearings { at, arterial }
    };
    let inner = bearings(u.spokes);
    let (outer, outer_from) = if u.outer_spokes > 0 && u.outer_spokes != u.spokes {
        (bearings(u.outer_spokes), streets.iter().position(|&r| r > u.outer_past).unwrap_or(streets.len()))
    } else {
        (inner.clone(), streets.len())
    };
    Grid { center, streets, inner, outer, outer_from, street_width: u.street_width }
}

impl Grid {
    fn polar(&self, r: f64, a: f64) -> P {
        (self.center.0 + r * a.stable_cos(), self.center.1 + r * a.stable_sin())
    }

    /// The bearings of ring of blocks `ring`, between ring streets `ring`
    /// and `ring + 1`.
    pub fn bearings(&self, ring: usize) -> &Bearings {
        if ring >= self.outer_from {
            &self.outer
        } else {
            &self.inner
        }
    }

    /// Ring street `k`'s points, closed: the last point is the first.
    /// Every bearing of the blocks on either side of it is a vertex.
    fn ring_street(&self, k: usize) -> Vec<P> {
        let r = self.streets[k];
        let sides: Vec<&Bearings> = [k.checked_sub(1), (k + 1 < self.streets.len()).then_some(k)]
            .into_iter()
            .flatten()
            .map(|ring| self.bearings(ring))
            .collect();
        if sides.windows(2).all(|w| std::ptr::eq(w[0], w[1])) {
            // One set: evenly round, so the vertices at its bearings are exact.
            let n = sides.first().map_or(&self.inner, |b| *b).at.len() as u32 * SEGMENTS_PER_BLOCK;
            return (0..=n).map(|i| self.polar(r, (i % n) as f64 * TAU / n as f64)).collect();
        }
        // Where two sets meet: both sets' bearings, exactly, and between
        // each two as many points as the finer set's blocks have.
        let mut angles: Vec<f64> = sides.iter().flat_map(|b| b.at.iter().map(|&(a, _)| a)).collect();
        angles.sort_by(f64::total_cmp);
        angles.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let most = sides.iter().map(|b| b.at.len()).max().unwrap_or(1) as u32;
        let step = TAU / (most * SEGMENTS_PER_BLOCK) as f64;
        let mut points = Vec::new();
        for (i, &a) in angles.iter().enumerate() {
            let b = angles.get(i + 1).copied().unwrap_or(angles[0] + TAU);
            let pieces = (((b - a) / step - 1e-9).ceil() as usize).max(1);
            points.extend((0..pieces).map(|p| self.polar(r, a + (b - a) * p as f64 / pieces as f64)));
        }
        points.push(points[0]);
        points
    }

    /// The ring streets and the spokes.
    pub fn roads(&self) -> Vec<RoadSegment> {
        let mut roads = Vec::new();
        if self.streets.is_empty() {
            return roads;
        }
        for k in 0..self.streets.len() {
            let points = self.ring_street(k);
            roads.push(RoadSegment { id: format!("urb_ring_{k}"), road_type: RoadType::Grid, points, width: self.street_width });
        }
        // Each spoke runs across the rings of blocks whose set has its
        // bearing: the inner set's out to ring street `outer_from`, the
        // outer set's on from there. One in both sets runs on through.
        let last = self.streets.len() - 1;
        let shared = |a: f64, set: &Bearings| set.at.iter().any(|&(b, _)| (a - b).abs() < 1e-9);
        let mut spokes: Vec<(String, f64, usize, usize)> = Vec::new();
        for (j, &(a, _)) in self.inner.at.iter().enumerate() {
            if !self.inner.arterial[j] {
                let through = self.outer_from >= last || shared(a, &self.outer);
                spokes.push((format!("urb_spoke_{j}"), a, 0, if through { last } else { self.outer_from }));
            }
        }
        if self.outer_from < last {
            for (j, &(a, _)) in self.outer.at.iter().enumerate() {
                if !self.outer.arterial[j] && !shared(a, &self.inner) {
                    spokes.push((format!("urb_spoke_out_{j}"), a, self.outer_from, last));
                }
            }
        }
        for (id, a, from, to) in spokes {
            if to > from {
                let points = self.streets[from..=to].iter().map(|&r| self.polar(r, a)).collect();
                roads.push(RoadSegment { id, road_type: RoadType::Grid, points, width: self.street_width });
            }
        }
        roads
    }

    /// The lots: `urb_{ring}_{block}_{i|o}_{k}`, the inner row (`i`) fronting
    /// ring street `ring`, the outer (`o`) fronting `ring + 1`, `k` along
    /// the row anticlockwise. `block` counts round the ring's own bearings.
    /// Each lot's street edge is its first two corners.
    pub fn lots(&self, params: &UrbanParams) -> Vec<TerrainPlot> {
        let half = self.street_width / 2.0;
        let mut lots = Vec::new();
        for ring in 0..self.streets.len().saturating_sub(1) {
            let bearings = &self.bearings(ring).at;
            let m = bearings.len();
            let (front_in, front_out) = (self.streets[ring] + half + CLEAR, self.streets[ring + 1] - half - CLEAR);
            let mid = (self.streets[ring] + self.streets[ring + 1]) / 2.0;
            for block in 0..m {
                let (a1, h1) = bearings[block];
                let (mut a2, h2) = bearings[(block + 1) % m];
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

    /// The seeds the targets hold over: the default and three more. The grid
    /// has no seed, but the world round it does.
    const SEEDS: [u64; 4] = [42, 7, 1234, 99_999];

    fn worlds() -> Vec<WorldGenParams> {
        SEEDS.map(|seed| WorldGenParams { seed, ..WorldGenParams::default() }).into()
    }

    /// Plots v2's §3 targets for the urban ring (#455; the default since the
    /// flip, #459): with the urban edge at r2160 and five rings of blocks,
    /// 900 to 1100 lots with a median of 7.5k to 9.5k, at least 95% taking 4
    /// stations at 50 spacing and none taking 5, laid clear of every road so
    /// `finish_lots` trims and drops none, on streets that meet at shared
    /// vertices. 32 bearings round the outer rings keep every block about 500
    /// long or less. Before the flip the targets were #421's: 500 to 700
    /// lots of 7k to 9k, nine in ten taking 4.
    #[test]
    fn the_urban_grid_meets_its_targets() {
        for params in worlds() {
            let (_, m) = metrics::measure(&params);
            eprintln!("seed {}: {m:?}", params.seed);
            let why = format!("seed {}: {m:#?}", params.seed);
            assert!((900..=1100).contains(&m.lots), "{why}");
            assert!((7500.0..=9500.0).contains(&m.area_median), "{why}");
            assert!(m.takes_four_share >= 0.95, "{why}");
            assert_eq!(m.takes_five, 0, "{why}");
            assert_eq!((m.trimmed, m.dropped), (0, 0), "{why}");
            assert_eq!(m.audit_issues, 0, "{why}");
            assert!(m.longest_block <= 500.0, "{why}");
        }
    }

    /// The grid alone measures the default world as the whole world does:
    /// nothing but the city's roads and the suburb's touch the urban ring.
    #[test]
    fn the_grid_alone_measures_the_default_world() {
        let params = WorldGenParams::default();
        let (whole, grid) = (metrics::measure(&params).1, metrics::measure_grid(&params).1);
        assert_eq!(format!("{whole:?}"), format!("{grid:?}"));
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
