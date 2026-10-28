//! Layout metrics for the rural ring, and the targets they're held to (#284).
//!
//! `measure` turns a generated ring into [`Metrics`], the numbers in the
//! targets table of `the-capitol-rural.md` §5, and [`Marker`]s, the places
//! that break a target, which the plan view draws in red. [`violations`]
//! holds the numbers against [`TARGETS`].
//!
//! The words, as the plan uses them:
//! - **half-sector:** the ring between an arterial and the bisector of its
//!   sector, 22.5° wide. Half-sector `h` runs from `22.5h°`, so sector `s`
//!   is half-sectors `2s` and `2s + 1`;
//! - **built share:** the part of the ground in the ring (or a half-sector)
//!   that a rural lot stands on, sampled every [`COVER_STEP`];
//! - **gap:** the shortest distance between two rural lots, edge to edge;
//!   0 where they touch or overlap;
//! - **on water:** a lot that a river or stream (a capsule as wide as the
//!   water at each end of the stretch) or a lake (its even-odd rings) runs into;
//! - **farm gate:** where a lane's side road or spur ends (RD5), the one place
//!   a loose end is allowed. Today's trails have none.
//!
//! The scaffolding (the scope, the checks, the violations and the number
//! format) is the suburb's (`suburb::metrics`, #104).

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{Lake, RoadSegment, RoadType, TerrainPlot, WaterCourse};
use serde::Serialize;

use crate::models::suburb::metrics::{percentile, Check, Marker, Scope, Target, Violation};
use crate::models::{lot_clearance, station_room};
use crate::models::geom::{quads_overlap_strict, SpatialGrid};
use crate::models::street_plan;
use crate::models::terrain::{point_to_segment_distance, TerrainConfig, WorldGenParams};
use crate::models::rural::sectors::{Character, Sector};
use crate::models::rural;
use crate::models::suburb::metrics::centroid;

type P = (f64, f64);

/// Built share is sampled on a grid this fine.
pub const COVER_STEP: f64 = 10.0;
/// The ring has this many half-sectors.
pub const HALF_SECTORS: usize = 16;
/// Each is this wide, in degrees.
pub const HALF_SECTOR_DEG: f64 = 360.0 / HALF_SECTORS as f64;
/// The smallest gap between farms (plan RD6), for the markers.
pub const MIN_GAP: f64 = 40.0;
/// Audit issues this far outside the ring still count: the ring's edges are
/// a little fuzzy where the roads leave it.
const EDGE_PAD: f64 = 40.0;
/// Grid cell for the sampling lookups.
const CELL: f64 = 100.0;
/// Lots further apart than this, box to box, aren't measured against each
/// other: no farm's nearest neighbour is that far.
const NEAR: f64 = 200.0;

/// Whether `road` is one of the ring's own, a `lane_*` (R3). The wild trails
/// (`wild_*`) are `Trail` too, but they're the wilds'.
pub fn is_rural_road(road: &RoadSegment) -> bool {
    road.road_type == RoadType::Trail && road.id.starts_with("lane_")
}

/// The half-sector `p` is in, 0 to 15.
pub fn half_sector_of(scope: &Scope, p: P) -> usize {
    let deg = (p.1 - scope.center.1).stable_atan2(p.0 - scope.center.0).to_degrees().rem_euclid(360.0);
    ((deg / HALF_SECTOR_DEG) as usize).min(HALF_SECTORS - 1)
}

/// The half-sectors a scope covers, in order.
pub fn half_sectors(scope: &Scope) -> Vec<usize> {
    match scope.sector {
        Some(s) => vec![2 * s as usize, 2 * s as usize + 1],
        None => (0..HALF_SECTORS).collect(),
    }
}

/// The ring's water, as the metrics see it.
#[derive(Clone, Copy)]
pub struct Waters<'a> {
    pub rivers: &'a [WaterCourse],
    pub streams: &'a [WaterCourse],
    pub lakes: &'a [Lake],
}

/// A generated ring, as the metrics see it.
pub struct Plan<'a> {
    pub roads: &'a [RoadSegment],
    /// Every lot in the world, after `finish_lots`.
    pub lots: &'a [TerrainPlot],
    /// The rural lots as generated, before `finish_lots`...
    pub before: &'a [TerrainPlot],
    /// ...and what `finish_lots` makes of each: `None` where it's dropped.
    pub cleared: &'a [Option<[P; 4]>],
    pub water: Waters<'a>,
    pub scope: Scope,
    /// The farm gates, where a loose end is allowed (RD5)...
    pub gates: &'a [P],
    /// ...and the sectors with their characters (RD2). Both empty for the
    /// trails.
    pub sectors: &'a [Sector],
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Metrics {
    pub layout: String,
    pub seed: u64,
    pub sector: Option<u8>,
    pub gen_ms: f64,

    pub rural_roads: usize,
    /// Their length inside the ring.
    pub road_length: f64,
    /// `street_plan::audit` on the rural roads, but for loose ends...
    pub audit_issues: usize,
    /// ...which are counted apart...
    pub loose_ends: usize,
    /// ...and these are the ones that aren't at a farm gate (RD5).
    pub loose_ends_off_gate: usize,
    /// How many sectors have the rarer of the two characters (plan RD2).
    /// Today's trails have no deck, so it's `None` for them.
    pub sector_character_min: Option<f64>,
    /// The fewest main roads, arterial to arterial, in a sector (RD10)...
    pub sector_mains_min: Option<f64>,
    /// ...and the fewest roads joining two of them. Both `None` for the
    /// trails, which have no sectors.
    pub sector_links_min: Option<f64>,

    pub lots: usize,
    pub lots_before: usize,
    pub trimmed_share: Option<f64>,
    pub dropped_share: Option<f64>,
    pub lot_area_mean: Option<f64>,
    pub lot_area_min: Option<f64>,
    pub lot_area_max: Option<f64>,
    /// The share of the farms that take 12 stations at 50 spacing
    /// (`station_room::takes`), Plots v2's room for a farm (§3).
    pub take_12_share: Option<f64>,

    /// The share of the ring a rural lot stands on.
    pub built_share: Option<f64>,
    /// The same, for each half-sector measured, in [`half_sectors`] order.
    pub half_sector_shares: Vec<f64>,
    pub half_sector_min: Option<f64>,
    pub half_sector_max: Option<f64>,
    /// Their coefficient of variation: standard deviation over mean.
    pub half_sector_cv: Option<f64>,

    /// The smallest gap between two lots...
    pub gap_min: Option<f64>,
    /// ...and the median of each lot's gap to its nearest neighbour.
    pub gap_nearest_median: Option<f64>,
    pub overlaps: usize,

    pub lots_on_water: usize,
    /// The closest any dry lot comes to the water.
    pub water_clearance_min: Option<f64>,
    /// Water in the ring: courses with a point in it, and lakes.
    pub courses: usize,
    pub lakes: usize,
}

impl Metrics {
    /// A metric by its field name, as a number.
    pub fn value(&self, name: &str) -> Option<f64> {
        serde_json::to_value(self).ok()?.get(name)?.as_f64()
    }
}

const fn hard(metric: &'static str, label: &'static str, check: Check) -> Target {
    Target { metric, label, check, hard: true }
}

const fn soft(metric: &'static str, label: &'static str, check: Check) -> Target {
    Target { metric, label, check, hard: false }
}

/// The targets table in `the-capitol-rural.md` §5: Plots v2's
/// (`the-capitol-plots-v2.md` §3, V1.3, #457), the only ones since the flip
/// (#459), for the ring of r3600 to r6100 and four main roads a sector:
/// about 900 farms of about 35k, built 40 to 50%, nearly every one taking
/// 12 stations. The farms were 900 to 1100 until #472 let the water half
/// way across them, as the 1600-deep ring's came, at a cost of about 35.
/// The rows that hold the ring together are RW1's, with a road joining
/// each pair of main roads. Before the flip the targets were 350 to 450
/// farms of 32k to 42k, built 30 to 40%, with two mains and a link.
pub const TARGETS: &[Target] = &[
    hard("lots", "farms", Check::Range(850.0, 1050.0)),
    hard("lot_area_mean", "mean farm area", Check::Range(31_000.0, 38_000.0)),
    hard("built_share", "ring built share", Check::Range(0.40, 0.50)),
    hard("take_12_share", "farms that take 12 stations", Check::Min(0.99)),
    hard("half_sector_min", "half-sector built share, least", Check::Min(0.20)),
    hard("half_sector_max", "half-sector built share, most", Check::Max(0.60)),
    soft("half_sector_cv", "CV of half-sector built shares", Check::Max(0.30)),
    hard("sector_character_min", "sectors of each character, fewest", Check::Min(3.0)),
    hard("sector_mains_min", "main roads in a sector, fewest", Check::Min(4.0)),
    hard("sector_links_min", "roads joining them in a sector, fewest", Check::Min(3.0)),
    hard("audit_issues", "street_plan::audit issues, but for loose ends", Check::Max(0.0)),
    hard("loose_ends_off_gate", "loose road ends off a farm gate", Check::Max(0.0)),
    hard("lots_on_water", "farms on water", Check::Max(0.0)),
    hard("gap_min", "gap between farms, least", Check::Min(MIN_GAP)),
];

/// The targets `m` misses. A metric with nothing to measure misses nothing.
pub fn violations(m: &Metrics) -> Vec<Violation> {
    violations_against(m, TARGETS)
}

/// The targets of `targets` that `m` misses.
pub fn violations_against(m: &Metrics, targets: &[Target]) -> Vec<Violation> {
    targets
        .iter()
        .filter_map(|t| {
            let v = m.value(t.metric)?;
            (!t.check.passes(v)).then(|| Violation {
                metric: t.metric,
                label: t.label,
                value: v,
                target: t.check.describe(),
                hard: t.hard,
            })
        })
        .collect()
}

pub struct Measured {
    pub metrics: Metrics,
    pub markers: Vec<Marker>,
}

/// A world, generated and measured over the rural ring, or one `sector` of it.
pub struct Measurement {
    pub terrain: TerrainConfig,
    /// The rural lots before `finish_lots`, and what it made of each.
    pub before: Vec<TerrainPlot>,
    pub cleared: Vec<Option<[P; 4]>>,
    pub scope: Scope,
    /// Under `Farms`, the ring's plan: its sectors and farm gates.
    pub rural: rural::Plan,
    pub measured: Measured,
}

/// Generates the world `params` describes and measures its rural ring.
/// `gen_ms` is how long the terrain took.
pub fn measure_world(params: &WorldGenParams, sector: Option<u8>) -> Measurement {
    let started = std::time::Instant::now();
    let terrain = TerrainConfig::from_params(params);
    let gen_ms = started.elapsed().as_secs_f64() * 1000.0;
    let center = (terrain.center_x, terrain.center_y);
    // The plan again, for its gates (the balance pass's too) and deck, and
    // the roads the farms were laid against.
    let (roads, ring) = terrain.generate_roads_and_rural_plan(params);
    let (before, laid_on) = terrain.rural_plots_before(params, roads);
    let cleared = lot_clearance::clear_each(&before, &laid_on, &terrain.water_rings, center);
    let scope = Scope { center, inner: params.suburban_outer_radius, outer: params.rural_outer_radius, sector };
    let plan = Plan {
        roads: &terrain.roads,
        lots: &terrain.terrain_plots,
        before: &before,
        cleared: &cleared,
        water: Waters { rivers: &terrain.rivers, streams: &terrain.streams, lakes: &terrain.lakes },
        scope,
        gates: &ring.gates,
        sectors: &ring.sectors,
    };
    let mut measured = measure(&plan);
    (measured.metrics.seed, measured.metrics.gen_ms) = (params.seed, gen_ms);
    Measurement { terrain, before, cleared, scope, rural: ring, measured }
}

pub fn measure(plan: &Plan) -> Measured {
    let scope = plan.scope;
    let roads = plan.roads;
    let mut m = Metrics { sector: scope.sector, ..Default::default() };
    let mut markers = Vec::new();

    // Roads.
    let mut length = 0.0;
    for road in roads.iter().filter(|r| is_rural_road(r)) {
        let inside: f64 = road
            .points
            .windows(2)
            .filter(|w| scope.contains(((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0)))
            .map(|w| dist(w[0], w[1]))
            .sum();
        if inside > 0.0 {
            m.rural_roads += 1;
            length += inside;
        }
    }
    m.road_length = length.round();

    let audit = street_plan::audit(roads, is_rural_road);
    let point_of = |id: &str, i: usize| {
        roads.iter().find(|r| r.id == id).and_then(|r| r.points.get(i).copied()).unwrap_or_default()
    };
    let mut issues: Vec<(P, String)> = Vec::new();
    issues.extend(audit.short_segments.iter().map(|(id, i)| (point_of(id, *i), format!("short segment on {id}"))));
    issues.extend(audit.sharp_turns.iter().map(|(id, i, d)| (point_of(id, *i), format!("{d:.0}° turn on {id}"))));
    issues.extend(audit.self_crossings.iter().map(|id| (point_of(id, 0), format!("{id} crosses itself"))));
    issues.extend(audit.unjoined_crossings.iter().map(|(a, b, x)| (*x, format!("{a} crosses {b} unjoined"))));
    for (at, note) in issues {
        if scope.contains_padded(at, EDGE_PAD) {
            m.audit_issues += 1;
            markers.push(Marker { kind: "audit", at, note });
        }
    }
    let loose: Vec<&(String, P)> = audit.loose_ends.iter().filter(|(_, p)| scope.contains_padded(*p, EDGE_PAD)).collect();
    m.loose_ends = loose.len();
    for (id, at) in loose {
        if !plan.gates.iter().any(|g| dist(*g, *at) < 1e-6) {
            m.loose_ends_off_gate += 1;
            markers.push(Marker { kind: "audit", at: *at, note: format!("loose end on {id}") });
        }
    }
    // The deck is the whole ring's, whatever the scope.
    if !plan.sectors.is_empty() {
        let count = |c: Character| plan.sectors.iter().filter(|s| s.character == c).count();
        m.sector_character_min = Some(count(Character::Concession).min(count(Character::Winding)) as f64);
        let counted: Vec<SectorRoads> = plan
            .sectors
            .iter()
            .filter(|s| scope.sector.is_none_or(|k| k as usize == s.index))
            .map(|s| sector_roads(roads, s.index))
            .collect();
        m.sector_mains_min = counted.iter().map(|c| c.mains as f64).reduce(f64::min);
        m.sector_links_min = counted.iter().map(|c| c.links as f64).reduce(f64::min);
    }

    // Lots.
    let lots: Vec<&TerrainPlot> =
        plan.lots.iter().filter(|l| l.zone == "rural" && scope.contains(centroid(&l.corners))).collect();
    m.lots = lots.len();
    let areas: Vec<f64> = lots.iter().map(|l| l.area).collect();
    if !areas.is_empty() {
        m.lot_area_mean = Some(areas.iter().sum::<f64>() / areas.len() as f64);
    }
    m.lot_area_min = areas.iter().copied().reduce(f64::min);
    m.lot_area_max = areas.iter().copied().reduce(f64::max);
    if !lots.is_empty() {
        let twelve = lots.iter().filter(|l| station_room::takes(&l.corners, 12)).count();
        m.take_12_share = Some(twelve as f64 / lots.len() as f64);
    }

    let before: Vec<usize> = (0..plan.before.len())
        .filter(|&i| plan.before[i].zone == "rural" && scope.contains(centroid(&plan.before[i].corners)))
        .collect();
    m.lots_before = before.len();
    if !before.is_empty() {
        let dropped = before.iter().filter(|&&i| plan.cleared[i].is_none()).count();
        let trimmed = before.iter().filter(|&&i| plan.cleared[i].is_some_and(|c| c != plan.before[i].corners)).count();
        m.dropped_share = Some(dropped as f64 / before.len() as f64);
        m.trimmed_share = Some(trimmed as f64 / before.len() as f64);
    }

    // Gaps, between every pair of lots that come within [`NEAR`] of each
    // other: a farm's nearest neighbour is a field away, and a ring of Plots
    // v2's has about 1000 (V1.3, #457).
    let mut nearest = vec![f64::INFINITY; lots.len()];
    let mut gap_min: Option<f64> = None;
    let boxes: Vec<(P, P)> = lots.iter().map(|l| bounds(&l.corners)).collect();
    let mut grid = SpatialGrid::new(scope.center.0, scope.center.1, scope.outer + CELL, CELL);
    for (i, l) in lots.iter().enumerate() {
        grid.insert(i, &l.corners);
    }
    for i in 0..lots.len() {
        let (lo, hi) = boxes[i];
        let near = grid.query_aabb(lo.0 - NEAR, lo.1 - NEAR, hi.0 + NEAR, hi.1 + NEAR);
        for k in near.into_iter().filter(|&k| k > i) {
            let (a, b) = (&lots[i].corners, &lots[k].corners);
            let overlap = quads_overlap_strict(a, b);
            let gap = if overlap { 0.0 } else { quad_gap(a, b) };
            nearest[i] = nearest[i].min(gap);
            nearest[k] = nearest[k].min(gap);
            gap_min = Some(gap_min.map_or(gap, |g| g.min(gap)));
            let at = mid(centroid(a), centroid(b));
            if overlap {
                m.overlaps += 1;
                markers.push(Marker { kind: "overlap", at, note: format!("{} overlaps {}", lots[i].id, lots[k].id) });
            } else if gap < MIN_GAP {
                markers.push(Marker { kind: "gap", at, note: format!("{} and {} {gap:.0} apart", lots[i].id, lots[k].id) });
            }
        }
    }
    m.gap_min = gap_min;
    let nearest: Vec<f64> = nearest.into_iter().filter(|g| g.is_finite()).collect();
    m.gap_nearest_median = percentile(&nearest, 0.5);

    // Water.
    let courses: Vec<&WaterCourse> = plan.water.rivers.iter().chain(plan.water.streams).collect();
    m.courses = courses.iter().filter(|c| c.points.iter().any(|p| scope.contains((p.x, p.y)))).count();
    m.lakes = plan
        .water
        .lakes
        .iter()
        .filter(|l| l.rings.iter().flatten().any(|&p| scope.contains(p)))
        .count();
    let mut clearance: Option<f64> = None;
    for lot in &lots {
        let (lo, hi) = bounds(&lot.corners);
        let mut wet: Option<&str> = None;
        let mut nearest = f64::INFINITY;
        for course in &courses {
            for w in course.points.windows(2) {
                let (p, q) = ((w[0].x, w[0].y), (w[1].x, w[1].y));
                let half = w[0].width.max(w[1].width) / 2.0;
                if p.0.max(q.0) + half < lo.0 - CELL
                    || p.0.min(q.0) - half > hi.0 + CELL
                    || p.1.max(q.1) + half < lo.1 - CELL
                    || p.1.min(q.1) - half > hi.1 + CELL
                {
                    continue;
                }
                let d = seg_quad_distance(p, q, &lot.corners) - half;
                if d < 0.0 {
                    wet = wet.or(Some(&course.id));
                }
                nearest = nearest.min(d);
            }
        }
        for lake in plan.water.lakes {
            for ring in &lake.rings {
                for i in 0..ring.len() {
                    let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
                    if p.0.max(q.0) < lo.0 - CELL || p.0.min(q.0) > hi.0 + CELL || p.1.max(q.1) < lo.1 - CELL || p.1.min(q.1) > hi.1 + CELL {
                        continue;
                    }
                    let d = seg_quad_distance(p, q, &lot.corners);
                    if d <= 0.0 {
                        wet = wet.or(Some(&lake.id));
                    }
                    nearest = nearest.min(d);
                }
            }
            // A lot wholly inside a lake crosses none of its shore.
            if lot.corners.iter().any(|&(x, y)| lake.contains(x, y)) {
                wet = wet.or(Some(&lake.id));
            }
        }
        match wet {
            Some(water) => {
                m.lots_on_water += 1;
                markers.push(Marker { kind: "water", at: centroid(&lot.corners), note: format!("{water} runs into {}", lot.id) });
            }
            None if nearest.is_finite() => clearance = Some(clearance.map_or(nearest, |c| c.min(nearest))),
            None => {}
        }
    }
    m.water_clearance_min = clearance.map(|c| c.max(0.0));

    // Built share.
    let wedges = half_sectors(&scope);
    let [built, ground] = built_ground(&lots, &scope);
    let shares: Vec<f64> = wedges.iter().map(|&h| if ground[h] > 0 { built[h] as f64 / ground[h] as f64 } else { 0.0 }).collect();
    let (b, g): (usize, usize) = wedges.iter().map(|&h| (built[h], ground[h])).fold((0, 0), |a, x| (a.0 + x.0, a.1 + x.1));
    if g > 0 {
        m.built_share = Some(b as f64 / g as f64);
    }
    m.half_sector_min = shares.iter().copied().reduce(f64::min);
    m.half_sector_max = shares.iter().copied().reduce(f64::max);
    if !shares.is_empty() {
        let mean = shares.iter().sum::<f64>() / shares.len() as f64;
        let var = shares.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / shares.len() as f64;
        m.half_sector_cv = (mean > 0.0).then(|| var.sqrt() / mean);
    }
    for (&h, &share) in wedges.iter().zip(&shares) {
        if !(0.20..=0.55).contains(&share) {
            let a = ((h as f64 + 0.5) * HALF_SECTOR_DEG).to_radians();
            let r = (scope.inner + scope.outer) / 2.0;
            let at = (scope.center.0 + r * a.stable_cos(), scope.center.1 + r * a.stable_sin());
            markers.push(Marker { kind: "share", at, note: format!("half-sector {h} is {:.0}% built", share * 100.0) });
        }
    }
    m.half_sector_shares = shares;

    Measured { metrics: m, markers }
}

/// A sector's roads, as the weld left them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SectorRoads {
    /// Lanes with both ends on an arterial's vertex.
    pub mains: usize,
    /// Lanes from a vertex of one main road to a vertex of another.
    pub links: usize,
}

/// Counts sector `s`'s roads from where their ends are, not from their ids,
/// so the weld is checked too.
pub fn sector_roads(roads: &[RoadSegment], s: usize) -> SectorRoads {
    let prefix = format!("lane_{s}_");
    let lanes: Vec<&RoadSegment> = roads.iter().filter(|r| is_rural_road(r) && r.id.starts_with(&prefix)).collect();
    let arterials: Vec<&RoadSegment> = roads.iter().filter(|r| r.road_type == RoadType::Arterial).collect();
    let ends = |r: &RoadSegment| [r.points[0], *r.points.last().expect("a road has ends")];
    let on = |road: &RoadSegment, p: P| road.points.iter().any(|&q| dist(p, q) < 1e-6);
    let mains: Vec<&RoadSegment> =
        lanes.iter().copied().filter(|r| ends(r).iter().all(|&e| arterials.iter().any(|a| on(a, e)))).collect();
    let links = lanes
        .iter()
        .filter(|r| {
            let [a, b] = ends(r);
            mains.iter().any(|m| m.id != r.id && on(m, a))
                && mains.iter().any(|m| m.id != r.id && on(m, b) && !on(m, a))
        })
        .count();
    SectorRoads { mains: mains.len(), links }
}

/// Samples the ring every [`COVER_STEP`] and counts, for each half-sector,
/// the samples on a lot and all of them.
fn built_ground(lots: &[&TerrainPlot], scope: &Scope) -> [[usize; HALF_SECTORS]; 2] {
    let mut grid = SpatialGrid::new(scope.center.0, scope.center.1, scope.outer + CELL, CELL);
    for (i, l) in lots.iter().enumerate() {
        grid.insert(i, &l.corners);
    }
    let (mut built, mut ground) = ([0; HALF_SECTORS], [0; HALF_SECTORS]);
    let (lo, hi) = scope.bounds();
    let mut y = lo.1 + COVER_STEP / 2.0;
    while y < hi.1 {
        let mut x = lo.0 + COVER_STEP / 2.0;
        while x < hi.0 {
            let p = (x, y);
            x += COVER_STEP;
            if !scope.contains(p) {
                continue;
            }
            let h = half_sector_of(scope, p);
            ground[h] += 1;
            if grid.query_point(p.0, p.1, 0.0).into_iter().any(|i| inside(&lots[i].corners, p)) {
                built[h] += 1;
            }
        }
        y += COVER_STEP;
    }
    [built, ground]
}

fn dist(a: P, b: P) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn mid(a: P, b: P) -> P {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn bounds(c: &[P; 4]) -> (P, P) {
    let (mut lo, mut hi) = (c[0], c[0]);
    for p in &c[1..] {
        lo = (lo.0.min(p.0), lo.1.min(p.1));
        hi = (hi.0.max(p.0), hi.1.max(p.1));
    }
    (lo, hi)
}

fn inside(c: &[P; 4], p: P) -> bool {
    let mut odd = false;
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 3) % 4]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            odd = !odd;
        }
    }
    odd
}

fn cross(o: P, a: P, b: P) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// Whether segments `ab` and `cd` cross or touch. Segments on one line
/// (within rounding) only meet if they overlap, which their ends' distances
/// tell: two farms' fronts along opposite arterials are on one line, far
/// apart (#287).
fn segments_meet(a: P, b: P, c: P, d: P) -> bool {
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    let scale = dist(a, b).powi(2) + dist(c, d).powi(2);
    if [d1, d2, d3, d4].iter().all(|x| x.abs() <= 1e-9 * scale) {
        return false;
    }
    d1 * d2 <= 0.0 && d3 * d4 <= 0.0 && (d1 != 0.0 || d2 != 0.0 || d3 != 0.0 || d4 != 0.0)
}

fn seg_seg_distance(a: P, b: P, c: P, d: P) -> f64 {
    if segments_meet(a, b, c, d) {
        return 0.0;
    }
    let pd = |p: P, s: P, e: P| point_to_segment_distance(p.0, p.1, s.0, s.1, e.0, e.1);
    pd(a, c, d).min(pd(b, c, d)).min(pd(c, a, b)).min(pd(d, a, b))
}

/// How far segment `pq` is from a lot: 0 where it crosses or is inside it.
fn seg_quad_distance(p: P, q: P, c: &[P; 4]) -> f64 {
    if inside(c, p) || inside(c, q) {
        return 0.0;
    }
    (0..4).map(|i| seg_seg_distance(p, q, c[i], c[(i + 1) % 4])).fold(f64::INFINITY, f64::min)
}

/// Edge to edge between two lots that don't overlap.
fn quad_gap(a: &[P; 4], b: &[P; 4]) -> f64 {
    (0..4).map(|i| seg_quad_distance(a[i], a[(i + 1) % 4], b)).fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::terrain::{Mouth, WaterPoint};

    fn lot(id: &str, corners: [P; 4]) -> TerrainPlot {
        TerrainPlot {
            id: id.into(),
            corners,
            street_facing_edge: (0, 1),
            area: TerrainPlot::calculate_area(&corners),
            adjacent_street_id: "lane_0_0".into(),
            zone: "rural".into(),
        }
    }

    /// A lot 150 across and 200 down, its top left corner at (`x`, `y`).
    fn lot_at(id: &str, x: f64, y: f64) -> TerrainPlot {
        lot(id, [(x, y), (x + 150.0, y), (x + 150.0, y + 200.0), (x, y + 200.0)])
    }

    fn ring() -> Scope {
        Scope { center: (0.0, 0.0), inner: 3000.0, outer: 3800.0, sector: Some(0) }
    }

    fn course(id: &str, points: &[(f64, f64)], width: f64) -> WaterCourse {
        WaterCourse {
            id: id.into(),
            source: None,
            mouth: Mouth::PetersOut,
            points: points.iter().map(|&(x, y)| WaterPoint { x, y, width, depth: 1.0, surface: 0.0 }).collect(),
        }
    }

    fn plan<'a>(lots: &'a [TerrainPlot], water: Waters<'a>, cleared: &'a [Option<[P; 4]>]) -> Plan<'a> {
        Plan { roads: &[], lots, before: lots, cleared, water, scope: ring(), gates: &[], sectors: &[] }
    }

    const DRY: Waters<'static> = Waters { rivers: &[], streams: &[], lakes: &[] };

    #[test]
    fn segments_on_one_line_meet_only_where_they_overlap() {
        // Far apart on one line, with the rounding of a world 13600 across.
        let y = 6860.000000000001;
        assert!(!segments_meet((2300.0, y), (2660.0, 6860.0), (11000.0, 6860.0), (11300.0, y)));
        assert!((seg_seg_distance((2300.0, y), (2660.0, 6860.0), (11000.0, 6860.0), (11300.0, y)) - 8340.0).abs() < 1e-6);
        assert_eq!(seg_seg_distance((0.0, 0.0), (10.0, 0.0), (5.0, 0.0), (20.0, 0.0)), 0.0);
        assert!(segments_meet((0.0, 0.0), (10.0, 10.0), (0.0, 10.0), (10.0, 0.0)));
    }

    #[test]
    fn gaps_are_edge_to_edge() {
        let lots = [lot_at("a", 3200.0, 10.0), lot_at("b", 3400.0, 10.0), lot_at("c", 3200.0, 260.0)];
        let cleared: Vec<_> = lots.iter().map(|l| Some(l.corners)).collect();
        let m = measure(&plan(&lots, DRY, &cleared)).metrics;
        // a's far side is at 3350, b's near one at 3400; a's bottom at 210, c's top at 260.
        assert_eq!(m.gap_min, Some(50.0));
        assert_eq!(m.gap_nearest_median, Some(50.0));
        assert_eq!((m.overlaps, m.lots), (0, 3));
        assert_eq!((m.trimmed_share, m.dropped_share), (Some(0.0), Some(0.0)));
    }

    #[test]
    fn built_share_counts_each_half_sector_apart() {
        // Sector 0 is half-sectors 0 (0° to 22.5°) and 1; a lot just south of
        // the x axis is in 0.
        let lots = [lot_at("a", 3200.0, 10.0)];
        let cleared = [Some(lots[0].corners)];
        let m = measure(&plan(&lots, DRY, &cleared)).metrics;
        let [first, second] = m.half_sector_shares[..] else { panic!("{:?}", m.half_sector_shares) };
        assert!(second == 0.0 && first > 0.0, "{first} {second}");
        // The half-sector is 22.5° of the ring: π(3800² − 3000²)/16 ≈ 1.07M.
        let expected = 30_000.0 / (std::f64::consts::PI * (3800.0f64.powi(2) - 3000.0f64.powi(2)) / 16.0);
        assert!((first - expected).abs() < 0.002, "{first} vs {expected}");
        assert_eq!(m.half_sector_min, Some(0.0));
        assert!(m.half_sector_cv.is_some_and(|cv| (cv - 1.0).abs() < 1e-9), "two shares, one empty: CV 1");
        assert!(violations(&m).iter().any(|v| v.metric == "half_sector_min" && v.hard));
    }

    #[test]
    fn a_stream_or_a_lake_across_a_lot_puts_it_on_water() {
        let lots = [lot_at("a", 3200.0, 10.0), lot_at("b", 3500.0, 10.0), lot_at("c", 3200.0, 400.0)];
        let cleared: Vec<_> = lots.iter().map(|l| Some(l.corners)).collect();
        // A stream 8 wide down the middle of a, and one 6 short of b's side.
        let streams = [course("stream_0", &[(3275.0, -100.0), (3275.0, 300.0)], 8.0), course("stream_1", &[(3480.0, -100.0), (3480.0, 300.0)], 28.0)];
        let pond = Lake {
            id: "lake_0".into(),
            level: 0.0,
            rings: vec![vec![(3250.0, 450.0), (3300.0, 450.0), (3300.0, 500.0), (3250.0, 500.0)]],
            area: 2500.0,
        };
        let lakes = [pond];
        let water = Waters { rivers: &[], streams: &streams, lakes: &lakes };
        let measured = measure(&plan(&lots, water, &cleared));
        let m = measured.metrics;
        assert_eq!(m.lots_on_water, 2, "a has a stream across it, c a pond inside it");
        assert_eq!(m.water_clearance_min, Some(6.0));
        assert_eq!(m.courses, 2);
        assert_eq!(m.lakes, 1);
        let notes: Vec<&str> = measured.markers.iter().filter(|k| k.kind == "water").map(|k| k.note.as_str()).collect();
        assert_eq!(notes, ["stream_0 runs into a", "lake_0 runs into c"]);
    }

    #[test]
    fn only_the_rings_own_lanes_are_rural_roads() {
        let road = |id: &str, road_type| RoadSegment { id: id.into(), road_type, points: vec![], width: 6.0, name: None };
        assert!(!is_rural_road(&road("trail_3", RoadType::Trail)), "the old trails are gone (#291)");
        assert!(is_rural_road(&road("lane_2_0", RoadType::Trail)));
        assert!(!is_rural_road(&road("wild_0_1", RoadType::Trail)));
        assert!(!is_rural_road(&road("arterial_0", RoadType::Arterial)));
    }

    #[test]
    fn targets_name_real_metrics() {
        let m = Metrics::default();
        let json = serde_json::to_value(&m).unwrap();
        for t in TARGETS {
            assert!(json.get(t.metric).is_some(), "{} is not a metric", t.metric);
        }
        assert!(violations(&m).iter().all(|v| v.metric != "sector_character_min"), "nothing measured, nothing missed");
    }
}
