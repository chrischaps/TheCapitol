//! Layout metrics for a suburban plan, and the targets they're held to.
//!
//! `measure` turns a generated plan into [`Metrics`], the numbers in the
//! targets table of `the-capitol-suburb-v2.md` §4.4, and [`Marker`]s, the
//! places that break a target, which the plan view draws in red.
//! [`violations`] holds the numbers against [`TARGETS`].
//!
//! What counts as a neighbourhood, a cul-de-sac and green ground works from
//! the roads alone, so it also holds for the hand-built plans in the tests.
//! (It was first written for the Legacy plan, which #113 removed.)
//! - **neighbourhood:** the 45° sector between two arterials;
//! - **entrance:** a vertex shared by a suburban road and any other road,
//!   counted in the sector its suburban street comes from;
//! - **cul-de-sac:** a suburban road that ends in a bulb or a dead end;
//! - **green:** the neighbourhoods' buffers, greenways and parks (`parks`,
//!   #111). A plan without greens counts open ground within `GREEN_STRIP`
//!   of an arterial's edge instead.

use std::collections::HashSet;

use capitol_protocol::terrain::{RoadSegment, RoadType, TerrainPlot};
use serde::Serialize;

use super::graph::{Graph, JunctionKind};
use super::neighbourhood::Neighbourhood;
use super::parks::{self, Ground, Greens};
use crate::models::lot_clearance;
use crate::models::terrain::{TerrainConfig, WorldGenParams};
use crate::models::geom::{quads_overlap_strict, SpatialGrid};
use crate::models::street_plan;
use crate::models::terrain::point_to_segment_distance;

type P = (f64, f64);

/// Open ground this close to an arterial's edge is the green strip.
pub const GREEN_STRIP: f64 = 40.0;
/// Coverage is sampled on a grid this fine.
const COVER_STEP: f64 = 5.0;
/// Parallel spacing is sampled this often along each street...
const PARALLEL_STEP: f64 = 25.0;
/// ...but not this close to its ends, where it meets other streets.
const PARALLEL_END_SKIP: f64 = 40.0;
/// How far across a street to look for the next one.
const PARALLEL_REACH: f64 = 400.0;
/// A street counts as parallel within this many degrees.
const PARALLEL_DEG: f64 = 20.0;
/// Junctions and entrances this far outside the suburb still count: the
/// arterial ribbons and the zone edges are a little fuzzy.
const EDGE_PAD: f64 = 40.0;
/// Grid cell for the spatial lookups.
const CELL: f64 = 50.0;

/// The part of the world being measured: the suburban annulus, or one sector of it.
#[derive(Debug, Clone, Copy)]
pub struct Scope {
    pub center: P,
    pub inner: f64,
    pub outer: f64,
    /// Sector `n` runs from arterial `n` at 45n° to the next, at 45(n+1)°.
    pub sector: Option<u8>,
}

impl Scope {
    pub fn sector_of(&self, p: P) -> u8 {
        let deg = (p.1 - self.center.1).atan2(p.0 - self.center.0).to_degrees().rem_euclid(360.0);
        ((deg / 45.0) as u8).min(7)
    }

    pub fn radius_of(&self, p: P) -> f64 {
        ((p.0 - self.center.0).powi(2) + (p.1 - self.center.1).powi(2)).sqrt()
    }

    pub fn contains(&self, p: P) -> bool {
        self.contains_padded(p, 0.0)
    }

    pub fn contains_padded(&self, p: P, pad: f64) -> bool {
        let r = self.radius_of(p);
        r >= self.inner - pad && r <= self.outer + pad && self.sector.is_none_or(|s| self.sector_of(p) == s)
    }

    /// The world box the scope sits in.
    pub fn bounds(&self) -> (P, P) {
        let Some(s) = self.sector else {
            let r = self.outer;
            return ((self.center.0 - r, self.center.1 - r), (self.center.0 + r, self.center.1 + r));
        };
        let (mut lo, mut hi) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
        for k in 0..=16 {
            let a = (45.0 * s as f64 + 45.0 * k as f64 / 16.0).to_radians();
            for r in [self.inner, self.outer] {
                let p = (self.center.0 + r * a.cos(), self.center.1 + r * a.sin());
                lo = (lo.0.min(p.0), lo.1.min(p.1));
                hi = (hi.0.max(p.0), hi.1.max(p.1));
            }
        }
        (lo, hi)
    }
}

/// A generated plan, as the metrics see it.
pub struct Plan<'a> {
    pub roads: &'a [RoadSegment],
    /// Every lot in the world, after `finish_lots`.
    pub lots: &'a [TerrainPlot],
    /// The suburban lots as generated, before `finish_lots`...
    pub before: &'a [TerrainPlot],
    /// ...and what `finish_lots` makes of each: `None` where it's dropped.
    pub cleared: &'a [Option<[P; 4]>],
    pub scope: Scope,
    /// Whether to sample coverage. It's the slow part in a debug build, so
    /// the street tests leave it out.
    pub coverage: bool,
    /// Each neighbourhood's ground, with its parks. A plan without them (the
    /// hand-built test plans) counts the strip along the arterials as green
    /// instead.
    pub greens: &'a [Greens],
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Metrics {
    pub layout: String,
    pub seed: u64,
    pub sector: Option<u8>,
    pub gen_ms: f64,

    pub suburban_roads: usize,
    pub audit_issues: usize,
    pub junctions: usize,
    pub tees: usize,
    pub four_ways: usize,
    pub min_junction_angle: Option<f64>,
    pub t_spacing_min: Option<f64>,
    pub t_spacing_p10: Option<f64>,
    /// Entrances in each sector measured, in sector order.
    pub entrances: Vec<usize>,
    pub entrances_min: Option<f64>,
    pub entrances_max: Option<f64>,
    pub culdesacs: usize,
    pub culdesac_max_len: Option<f64>,
    pub culdesac_max_lots: Option<f64>,
    pub parallel_samples: usize,
    pub parallel_spacing_median: Option<f64>,
    pub parallel_spacing_p5: Option<f64>,
    pub connected_share: Option<f64>,

    pub lots: usize,
    pub lot_area_median: Option<f64>,
    pub lot_area_p5: Option<f64>,
    pub lot_area_p95: Option<f64>,
    pub lot_width_median: Option<f64>,
    pub lot_depth_median: Option<f64>,
    pub convex_share: Option<f64>,
    pub overlaps: usize,
    pub lots_before: usize,
    pub trimmed_share: Option<f64>,
    pub dropped_share: Option<f64>,
    pub arterial_fronting: usize,

    pub cover_lots: Option<f64>,
    pub cover_roads: Option<f64>,
    pub cover_green: Option<f64>,
    pub cover_gap: Option<f64>,
    pub parks: usize,
    /// The smallest park away from the buffers.
    pub park_area_min: Option<f64>,
}

impl Metrics {
    /// A metric by its field name, as a number.
    pub fn value(&self, name: &str) -> Option<f64> {
        serde_json::to_value(self).ok()?.get(name)?.as_f64()
    }
}

/// A place in the plan that breaks a target.
#[derive(Debug, Clone, Serialize)]
pub struct Marker {
    pub kind: &'static str,
    pub at: P,
    pub note: String,
}

#[derive(Debug, Clone, Copy)]
pub enum Check {
    Max(f64),
    Min(f64),
    Range(f64, f64),
}

impl Check {
    pub fn passes(self, v: f64) -> bool {
        match self {
            Check::Max(m) => v <= m,
            Check::Min(m) => v >= m,
            Check::Range(lo, hi) => (lo..=hi).contains(&v),
        }
    }

    pub fn describe(self) -> String {
        match self {
            Check::Max(m) => format!("≤ {}", fmt_num(m)),
            Check::Min(m) => format!("≥ {}", fmt_num(m)),
            Check::Range(lo, hi) => format!("{}–{}", fmt_num(lo), fmt_num(hi)),
        }
    }
}

pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else if v.abs() < 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.1}")
    }
}

pub struct Target {
    pub metric: &'static str,
    pub label: &'static str,
    pub check: Check,
    /// A hard target holds for every plan; the rest are what tuning aims at.
    pub hard: bool,
}

const fn soft(metric: &'static str, label: &'static str, check: Check) -> Target {
    Target { metric, label, check, hard: false }
}

/// The targets table in `the-capitol-suburb-v2.md` §4.4.
pub const TARGETS: &[Target] = &[
    Target { metric: "audit_issues", label: "street_plan::audit issues", check: Check::Max(0.0), hard: true },
    soft("four_ways", "suburban 4-ways", Check::Max(0.0)),
    soft("min_junction_angle", "narrowest junction angle (°)", Check::Min(70.0)),
    soft("t_spacing_min", "T spacing along a road, min", Check::Min(40.0)),
    soft("t_spacing_p10", "T spacing along a road, p10", Check::Min(150.0)),
    soft("entrances_min", "entrances per neighbourhood, fewest", Check::Min(2.0)),
    soft("entrances_max", "entrances per neighbourhood, most", Check::Max(4.0)),
    soft("culdesac_max_len", "longest cul-de-sac stem", Check::Max(300.0)),
    soft("culdesac_max_lots", "most lots on a cul-de-sac", Check::Max(20.0)),
    soft("parallel_spacing_median", "parallel spacing, median", Check::Range(216.0, 256.0)),
    soft("parallel_spacing_p5", "parallel spacing, p5", Check::Min(200.0)),
    soft("connected_share", "streets connected to an arterial", Check::Min(1.0)),
    soft("lot_area_median", "lot area, median", Check::Range(6600.0, 7500.0)),
    soft("lot_area_p5", "lot area, p5", Check::Min(4000.0)),
    soft("lot_area_p95", "lot area, p95", Check::Max(11000.0)),
    soft("lot_width_median", "lot width at mid-depth, median", Check::Range(55.0, 65.0)),
    soft("lot_depth_median", "lot depth, median", Check::Range(100.0, 125.0)),
    soft("convex_share", "convex lots", Check::Min(1.0)),
    Target { metric: "overlaps", label: "lot-lot overlaps", check: Check::Max(0.0), hard: true },
    soft("trimmed_share", "lots trimmed by finish_lots", Check::Max(0.10)),
    soft("dropped_share", "lots dropped by finish_lots", Check::Max(0.03)),
    soft("cover_lots", "coverage: lots", Check::Min(0.55)),
    soft("cover_roads", "coverage: roads", Check::Range(0.15, 0.22)),
    soft("cover_green", "coverage: green", Check::Range(0.08, 0.20)),
    soft("cover_gap", "coverage: gap", Check::Max(0.05)),
    soft("arterial_fronting", "lots fronting arterials", Check::Max(0.0)),
    // "Well under the 5 s test" in a debug build; the report runs in release.
    soft("gen_ms", "generation (ms)", Check::Max(2500.0)),
];

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub metric: &'static str,
    pub label: &'static str,
    pub value: f64,
    pub target: String,
    pub hard: bool,
}

/// The targets `m` misses. A metric with nothing to measure (no parallel
/// streets, say) misses nothing.
pub fn violations(m: &Metrics) -> Vec<Violation> {
    TARGETS
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

/// Nearest-rank percentile of `values`, `p` in 0..=1.
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    Some(v[((v.len() - 1) as f64 * p).round() as usize])
}

fn dist(a: P, b: P) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn mid(a: P, b: P) -> P {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

/// A lot's corners from its street edge round: street-left, street-right,
/// rear-right, rear-left.
fn from_street(lot: &TerrainPlot) -> [P; 4] {
    let (i, j) = lot.street_facing_edge;
    let s = if (i + 1) % 4 == j { i } else { j };
    std::array::from_fn(|k| lot.corners[(s + k) % 4])
}

/// The street edge's midpoint.
pub fn frontage_mid(lot: &TerrainPlot) -> P {
    let c = from_street(lot);
    mid(c[0], c[1])
}

/// Width halfway back: between the midpoints of the two side edges.
pub fn width_at_mid_depth(lot: &TerrainPlot) -> f64 {
    let c = from_street(lot);
    dist(mid(c[1], c[2]), mid(c[3], c[0]))
}

/// From the middle of the street edge to the middle of the rear edge.
pub fn depth(lot: &TerrainPlot) -> f64 {
    let c = from_street(lot);
    dist(mid(c[0], c[1]), mid(c[2], c[3]))
}

pub fn centroid(c: &[P; 4]) -> P {
    (c.iter().map(|p| p.0).sum::<f64>() / 4.0, c.iter().map(|p| p.1).sum::<f64>() / 4.0)
}

pub(crate) use crate::models::geom::quad_is_convex as is_convex;

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

/// One stretch of road centreline.
struct Seg {
    a: P,
    b: P,
    half: f64,
    road: usize,
}

/// Every road stretch, in a grid.
struct Segs {
    segs: Vec<Seg>,
    grid: SpatialGrid,
}

impl Segs {
    fn new(roads: &[RoadSegment], scope: &Scope, keep: impl Fn(&RoadSegment) -> bool) -> Self {
        let mut grid = SpatialGrid::new(scope.center.0, scope.center.1, scope.outer + 2.0 * PARALLEL_REACH, CELL);
        let mut segs = Vec::new();
        for (r, road) in roads.iter().enumerate().filter(|(_, r)| keep(r)) {
            for w in road.points.windows(2) {
                grid.insert(segs.len(), w);
                segs.push(Seg { a: w[0], b: w[1], half: road.width / 2.0, road: r });
            }
        }
        Self { segs, grid }
    }

    fn near(&self, p: P, radius: f64) -> impl Iterator<Item = &Seg> {
        self.grid.query_point(p.0, p.1, radius).into_iter().map(|i| &self.segs[i])
    }

    fn distance(s: &Seg, p: P) -> f64 {
        point_to_segment_distance(p.0, p.1, s.a.0, s.a.1, s.b.0, s.b.1)
    }
}

/// Where the ray from `o` along unit `d` first meets segment `a`-`b`, as a distance.
fn ray_hit(o: P, d: P, a: P, b: P) -> Option<f64> {
    let e = (b.0 - a.0, b.1 - a.1);
    let denom = d.0 * e.1 - d.1 * e.0;
    if denom.abs() < 1e-12 {
        return None;
    }
    let w = (a.0 - o.0, a.1 - o.1);
    let t = (w.0 * e.1 - w.1 * e.0) / denom;
    let u = (w.0 * d.1 - w.1 * d.0) / denom;
    (t > 0.0 && (0.0..=1.0).contains(&u)).then_some(t)
}

pub struct Measured {
    pub metrics: Metrics,
    pub markers: Vec<Marker>,
}

/// A Neighbourhood world, generated and measured over the whole suburb, or
/// one `sector` of it.
pub struct Measurement {
    pub terrain: TerrainConfig,
    /// The suburban lots before `finish_lots`, and what it made of each.
    pub before: Vec<TerrainPlot>,
    pub cleared: Vec<Option<[P; 4]>>,
    pub neighbourhoods: Vec<Neighbourhood>,
    pub greens: Vec<Greens>,
    pub scope: Scope,
    pub measured: Measured,
}

/// Generates the world `params` describes and measures its suburb, parks
/// and all. `gen_ms` is how long the terrain took.
pub fn measure_world(params: &WorldGenParams, sector: Option<u8>) -> Measurement {
    let started = std::time::Instant::now();
    let terrain = TerrainConfig::from_params(params);
    let gen_ms = started.elapsed().as_secs_f64() * 1000.0;
    let sp = params.suburban_params();
    let center = (terrain.center_x, terrain.center_y);
    let before = terrain.generate_suburban_plots_internal(&sp);
    let cleared = lot_clearance::clear_each(&before, &terrain.roads, &terrain.water_rings, center);
    let neighbourhoods = super::plan(&sp, center).neighbourhoods;
    let greens = parks::lay(&terrain.roads, &terrain.terrain_plots, &neighbourhoods, &sp, center);
    let scope = Scope { center, inner: params.urban_outer_radius, outer: params.suburban_outer_radius, sector };
    let plan = Plan {
        roads: &terrain.roads,
        lots: &terrain.terrain_plots,
        before: &before,
        cleared: &cleared,
        scope,
        coverage: true,
        greens: &greens,
    };
    let mut measured = measure(&plan);
    (measured.metrics.seed, measured.metrics.gen_ms) = (params.seed, gen_ms);
    Measurement { terrain, before, cleared, neighbourhoods, greens, scope, measured }
}

pub fn measure(plan: &Plan) -> Measured {
    let scope = plan.scope;
    let roads = plan.roads;
    let suburban = |r: usize| roads[r].road_type == RoadType::Suburban;
    let mut m = Metrics { sector: scope.sector, ..Default::default() };
    let mut markers = Vec::new();
    let mut mark = |kind: &'static str, at: P, note: String| {
        if scope.contains_padded(at, EDGE_PAD) {
            markers.push(Marker { kind, at, note });
        }
    };

    // Streets.
    let g = Graph::build(roads);
    let in_scope_road = |r: usize| {
        let pts = &roads[r].points;
        suburban(r) && !pts.is_empty() && scope.contains_padded(pts[pts.len() / 2], EDGE_PAD)
    };
    let local: Vec<usize> = (0..roads.len()).filter(|&r| in_scope_road(r)).collect();
    m.suburban_roads = local.len();

    let audit = street_plan::audit(roads, |r| r.road_type == RoadType::Suburban);
    let point_of = |id: &str, i: usize| {
        roads.iter().find(|r| r.id == id).and_then(|r| r.points.get(i).copied()).unwrap_or_default()
    };
    let mut issues: Vec<(P, String)> = Vec::new();
    issues.extend(audit.short_segments.iter().map(|(id, i)| (point_of(id, *i), format!("short segment on {id}"))));
    issues.extend(audit.sharp_turns.iter().map(|(id, i, d)| (point_of(id, *i), format!("{d:.0}° turn on {id}"))));
    issues.extend(audit.self_crossings.iter().map(|id| (point_of(id, 0), format!("{id} crosses itself"))));
    issues.extend(audit.unjoined_crossings.iter().map(|(a, b, x)| (*x, format!("{a} crosses {b} unjoined"))));
    issues.extend(audit.loose_ends.iter().map(|(id, p)| (*p, format!("loose end of {id}"))));
    for (at, note) in issues {
        if scope.contains_padded(at, EDGE_PAD) {
            m.audit_issues += 1;
            mark("audit", at, note);
        }
    }

    let mut angles = Vec::new();
    for j in g.junctions() {
        let at = g.nodes[j.node].at;
        if !scope.contains_padded(at, EDGE_PAD) || !g.roads_at(j.node).into_iter().any(suburban) {
            continue;
        }
        m.junctions += 1;
        match j.kind {
            JunctionKind::Tee => m.tees += 1,
            JunctionKind::FourWay => {
                m.four_ways += 1;
                mark("four_way", at, format!("{}-way junction", g.degree(j.node)));
            }
        }
        if j.min_angle < 70.0 {
            mark("angle", at, format!("{:.0}° junction", j.min_angle));
        }
        angles.push(j.min_angle);
    }
    m.min_junction_angle = angles.iter().copied().reduce(f64::min);

    let mut spacing = Vec::new();
    for &r in &local {
        for (gap, at) in g.junction_spacing(r) {
            if gap < 40.0 {
                mark("t_spacing", at, format!("junctions {gap:.0} apart on {}", roads[r].id));
            }
            spacing.push(gap);
        }
    }
    m.t_spacing_min = spacing.iter().copied().reduce(f64::min);
    m.t_spacing_p10 = percentile(&spacing, 0.10);

    let sectors: Vec<u8> = match scope.sector {
        Some(s) => vec![s],
        None => (0..8).collect(),
    };
    m.entrances = vec![0; sectors.len()];
    for (n, node) in g.nodes.iter().enumerate() {
        let roads_here = g.roads_at(n);
        if !roads_here.iter().any(|&r| suburban(r)) || roads_here.iter().all(|&r| suburban(r)) {
            continue;
        }
        if scope.contains_padded(node.at, EDGE_PAD) {
            // An entrance sits on the arterial's centre line, between two
            // sectors; it belongs to the one its suburban street comes from.
            let from = node.links.iter().find(|l| suburban(l.road)).map_or(node.at, |l| g.nodes[l.to].at);
            let s = scope.sector_of(from);
            if let Some(k) = sectors.iter().position(|&x| x == s) {
                m.entrances[k] += 1;
            }
        }
    }
    m.entrances_min = m.entrances.iter().min().map(|&v| v as f64);
    m.entrances_max = m.entrances.iter().max().map(|&v| v as f64);

    let reached = g.reached_from_arterials();
    let connected = local.iter().filter(|&&r| g.walks[r].nodes.iter().any(|&n| reached[n])).count();
    for &r in &local {
        if !g.walks[r].nodes.iter().any(|&n| reached[n]) {
            let pts = &roads[r].points;
            mark("disconnected", pts[pts.len() / 2], format!("{} can't reach an arterial", roads[r].id));
        }
    }
    m.connected_share = (!local.is_empty()).then(|| connected as f64 / local.len() as f64);

    let mut lots_on: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for lot in plan.lots.iter().filter(|l| l.zone == "suburban") {
        *lots_on.entry(lot.adjacent_street_id.as_str()).or_default() += 1;
    }
    let mut stems = Vec::new();
    let mut counts = Vec::new();
    for &r in &local {
        let Some(c) = g.cul_de_sac(r) else { continue };
        let id = roads[c.road].id.as_str();
        let lots = lots_on.get(id).copied().unwrap_or(0);
        if c.stem > 300.0 || lots > 20 {
            mark("culdesac", c.end, format!("{id}: stem {:.0}, {lots} lots", c.stem));
        }
        stems.push(c.stem);
        counts.push(lots as f64);
    }
    m.culdesacs = stems.len();
    m.culdesac_max_len = stems.iter().copied().reduce(f64::max);
    m.culdesac_max_lots = counts.iter().copied().reduce(f64::max);

    let streets = Segs::new(roads, &scope, |r| r.road_type == RoadType::Suburban);
    let mut parallel = Vec::new();
    let cos_par = PARALLEL_DEG.to_radians().cos();
    for &r in &local {
        let pts = &roads[r].points;
        let total: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
        let mut along = 0.0;
        let mut next = PARALLEL_END_SKIP;
        for w in pts.windows(2) {
            let len = dist(w[0], w[1]);
            while len > 0.0 && next <= along + len && next <= total - PARALLEL_END_SKIP {
                let t = (next - along) / len;
                let o = (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
                let tan = ((w[1].0 - w[0].0) / len, (w[1].1 - w[0].1) / len);
                for side in [1.0, -1.0] {
                    let d = (-tan.1 * side, tan.0 * side);
                    let far = (o.0 + d.0 * PARALLEL_REACH, o.1 + d.1 * PARALLEL_REACH);
                    let hits = streets.grid.query_aabb(o.0.min(far.0), o.1.min(far.1), o.0.max(far.0), o.1.max(far.1));
                    let nearest = hits
                        .into_iter()
                        .map(|i| &streets.segs[i])
                        .filter_map(|s| ray_hit(o, d, s.a, s.b).filter(|&h| h > 1.0 && h <= PARALLEL_REACH).map(|h| (h, s)))
                        .min_by(|a, b| a.0.total_cmp(&b.0));
                    if let Some((h, s)) = nearest {
                        let l = dist(s.a, s.b);
                        let cos = ((s.b.0 - s.a.0) * tan.0 + (s.b.1 - s.a.1) * tan.1).abs() / l;
                        if cos >= cos_par {
                            parallel.push(h);
                        }
                    }
                }
                next += PARALLEL_STEP;
            }
            along += len;
        }
    }
    m.parallel_samples = parallel.len();
    m.parallel_spacing_median = percentile(&parallel, 0.5);
    m.parallel_spacing_p5 = percentile(&parallel, 0.05);

    // Lots.
    let lots: Vec<&TerrainPlot> = plan
        .lots
        .iter()
        .filter(|l| l.zone == "suburban" && scope.contains(centroid(&l.corners)))
        .collect();
    m.lots = lots.len();
    let areas: Vec<f64> = lots.iter().map(|l| l.area).collect();
    m.lot_area_median = percentile(&areas, 0.5);
    m.lot_area_p5 = percentile(&areas, 0.05);
    m.lot_area_p95 = percentile(&areas, 0.95);
    let widths: Vec<f64> = lots.iter().map(|l| width_at_mid_depth(l)).collect();
    m.lot_width_median = percentile(&widths, 0.5);
    let depths: Vec<f64> = lots.iter().map(|l| depth(l)).collect();
    m.lot_depth_median = percentile(&depths, 0.5);
    let convex = lots.iter().filter(|l| is_convex(&l.corners)).count();
    for l in lots.iter().filter(|l| !is_convex(&l.corners)) {
        mark("concave", centroid(&l.corners), format!("{} is not convex", l.id));
    }
    m.convex_share = (!lots.is_empty()).then(|| convex as f64 / lots.len() as f64);

    let mut lot_grid = SpatialGrid::new(scope.center.0, scope.center.1, scope.outer + 2.0 * PARALLEL_REACH, CELL);
    for (i, l) in lots.iter().enumerate() {
        lot_grid.insert(i, &l.corners);
    }
    let mut pairs = HashSet::new();
    for (i, l) in lots.iter().enumerate() {
        let (lo, hi) = bounds(&l.corners);
        for k in lot_grid.query_aabb(lo.0, lo.1, hi.0, hi.1) {
            if k > i && quads_overlap_strict(&l.corners, &lots[k].corners) && pairs.insert((i, k)) {
                let at = mid(centroid(&l.corners), centroid(&lots[k].corners));
                mark("overlap", at, format!("{} overlaps {}", l.id, lots[k].id));
            }
        }
    }
    m.overlaps = pairs.len();

    let all_roads = Segs::new(roads, &scope, |_| true);
    for l in &lots {
        let f = frontage_mid(l);
        let nearest = all_roads
            .near(f, 2.0 * CELL)
            .map(|s| (Segs::distance(s, f) - s.half, s.road))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, r)) = nearest {
            if roads[r].road_type == RoadType::Arterial {
                m.arterial_fronting += 1;
                mark("arterial_front", f, format!("{} fronts {}", l.id, roads[r].id));
            }
        }
    }

    let before: Vec<usize> = (0..plan.before.len())
        .filter(|&i| plan.before[i].zone == "suburban" && scope.contains(centroid(&plan.before[i].corners)))
        .collect();
    m.lots_before = before.len();
    let dropped = before.iter().filter(|&&i| plan.cleared[i].is_none()).count();
    let trimmed = before
        .iter()
        .filter(|&&i| plan.cleared[i].is_some_and(|c| c != plan.before[i].corners))
        .count();
    if !before.is_empty() {
        m.dropped_share = Some(dropped as f64 / before.len() as f64);
        m.trimmed_share = Some(trimmed as f64 / before.len() as f64);
    }

    // Coverage. Every lot counts here, so urban and rural lots over the line
    // aren't read as gaps.
    let (lo, hi) = scope.bounds();
    let cover = if plan.coverage {
        coverage(plan.lots, roads, &all_roads, plan.greens, &scope, (lo, hi), |p| scope.contains(p))
    } else {
        [0; 4]
    };
    let total = cover.iter().sum::<usize>();
    if total > 0 {
        let share = |k: usize| Some(cover[k] as f64 / total as f64);
        (m.cover_lots, m.cover_roads, m.cover_green, m.cover_gap) = (share(0), share(1), share(2), share(3));
    }

    // Parks, in the neighbourhoods measured.
    let parks: Vec<_> = plan
        .greens
        .iter()
        .filter(|g| scope.sector.is_none_or(|x| x == g.sector))
        .flat_map(|g| &g.parks)
        .collect();
    m.parks = parks.len();
    m.park_area_min = parks.iter().filter(|k| !k.on_buffer).map(|k| k.area).reduce(f64::min);

    Measured { metrics: m, markers }
}

/// Samples the box `lo`..`hi` where `keep` holds, every `COVER_STEP`, and
/// counts the samples on lots, on roads, on green ground and on nothing.
fn coverage(
    lots: &[TerrainPlot],
    roads: &[RoadSegment],
    segs: &Segs,
    greens: &[Greens],
    scope: &Scope,
    (lo, hi): (P, P),
    keep: impl Fn(P) -> bool,
) -> [usize; 4] {
    let mut grid = SpatialGrid::new(scope.center.0, scope.center.1, scope.outer + 2.0 * PARALLEL_REACH, CELL);
    for (i, l) in lots.iter().enumerate() {
        grid.insert(i, &l.corners);
    }
    let reach = roads.iter().map(|r| r.width / 2.0).fold(0.0, f64::max) + GREEN_STRIP;
    let mut cover = [0usize; 4];
    let mut y = lo.1 + COVER_STEP / 2.0;
    while y < hi.1 {
        let mut x = lo.0 + COVER_STEP / 2.0;
        while x < hi.0 {
            let p = (x, y);
            x += COVER_STEP;
            if !keep(p) {
                continue;
            }
            let mut on_road = false;
            let mut near_arterial = false;
            for s in segs.near(p, reach) {
                let d = Segs::distance(s, p);
                if d < s.half {
                    on_road = true;
                    break;
                }
                if roads[s.road].road_type == RoadType::Arterial && d < s.half + GREEN_STRIP {
                    near_arterial = true;
                }
            }
            let k = if on_road {
                1
            } else if grid.query_point(p.0, p.1, 0.0).into_iter().any(|i| inside(&lots[i].corners, p)) {
                0
            } else if greens.is_empty() && near_arterial
                || greens.iter().any(|g| matches!(g.at(p), Some(Ground::Buffer | Ground::Park)))
            {
                2
            } else {
                3
            };
            cover[k] += 1;
        }
        y += COVER_STEP;
    }
    cover
}

fn bounds(c: &[P; 4]) -> (P, P) {
    let (mut lo, mut hi) = (c[0], c[0]);
    for p in &c[1..] {
        lo = (lo.0.min(p.0), lo.1.min(p.1));
        hi = (hi.0.max(p.0), hi.1.max(p.1));
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(id: &str, corners: [P; 4]) -> TerrainPlot {
        TerrainPlot {
            id: id.into(),
            corners,
            street_facing_edge: (0, 1),
            area: TerrainPlot::calculate_area(&corners),
            adjacent_street_id: "s".into(),
            zone: "suburban".into(),
        }
    }

    /// A block two lots deep and three wide, east of the centre: lots fill
    /// x 1700..1880 and y 0..220, and front streets 16 wide above and below.
    fn block() -> (Vec<RoadSegment>, Vec<TerrainPlot>) {
        let (x0, w, d) = (1700.0, 60.0, 110.0);
        let mut lots = Vec::new();
        for k in 0..3 {
            let (xa, xb) = (x0 + k as f64 * w, x0 + (k + 1) as f64 * w);
            // Fronting the street at y = 0: street edge first.
            lots.push(lot(&format!("n{k}"), [(xb, 0.0), (xa, 0.0), (xa, d), (xb, d)]));
            // Fronting the street at y = 2d, backing onto the first row.
            lots.push(lot(&format!("s{k}"), [(xa, 2.0 * d), (xb, 2.0 * d), (xb, d), (xa, d)]));
        }
        // Centred 8 outside each row, so the lots sit on the kerb.
        let street = |id: &str, y: f64| RoadSegment {
            id: id.into(),
            road_type: RoadType::Suburban,
            points: vec![(x0 - 30.0, y), (x0 + 90.0, y), (x0 + 210.0, y)],
            width: 16.0,
        };
        (vec![street("a", -8.0), street("b", 2.0 * d + 8.0)], lots)
    }

    fn block_scope() -> Scope {
        // One sector, so coverage samples a small box: the whole annulus is
        // seconds of work in a debug build.
        Scope { center: (0.0, 0.0), inner: 1700.0, outer: 1880.0, sector: Some(0) }
    }

    #[test]
    fn a_two_by_three_block_has_no_overlap() {
        let (roads, lots) = block();
        let cleared: Vec<_> = lots.iter().map(|l| Some(l.corners)).collect();
        let plan = Plan { roads: &roads, lots: &lots, before: &lots, cleared: &cleared, scope: block_scope(), coverage: true, greens: &[] };
        let measured = measure(&plan);
        let m = measured.metrics;
        assert_eq!(m.overlaps, 0);
        assert!(measured.markers.iter().all(|k| k.kind != "overlap"));
        assert_eq!(m.lots, 6);
        assert_eq!(m.convex_share, Some(1.0));
        assert_eq!(m.lot_width_median, Some(60.0));
        assert_eq!(m.lot_depth_median, Some(110.0));
        assert_eq!(m.lot_area_median, Some(6600.0));
        assert_eq!((m.trimmed_share, m.dropped_share), (Some(0.0), Some(0.0)));
        assert_eq!(m.arterial_fronting, 0);
        // Centre to centre, the streets are 2·110 + 16 apart.
        let spacing = m.parallel_spacing_median.expect("parallel streets");
        assert!((spacing - 236.0).abs() < 1e-9, "{spacing}");
    }

    #[test]
    fn a_two_by_three_block_has_no_gap() {
        let (roads, lots) = block();
        let scope = block_scope();
        let segs = Segs::new(&roads, &scope, |_| true);
        // The block and its two streets, kerb to kerb.
        let (lo, hi) = ((1700.0, -16.0), (1880.0, 236.0));
        let [on_lots, on_roads, green, gap] = coverage(&lots, &roads, &segs, &[], &scope, (lo, hi), |_| true);
        assert_eq!((green, gap), (0, 0));
        // 5-unit samples: 36 across, 44 down the lots and 2 × 3 down each street.
        assert_eq!((on_lots, on_roads), (36 * 44, 36 * 6));
    }

    #[test]
    fn back_to_back_lots_touch_but_shifted_ones_overlap() {
        let (_, mut lots) = block();
        let a = &lots[0].corners;
        let b = &lots[1].corners;
        assert!(!quads_overlap_strict(a, b));
        lots[1].corners.iter_mut().for_each(|p| p.1 -= 1.0);
        assert!(quads_overlap_strict(&lots[0].corners, &lots[1].corners));
    }

    #[test]
    fn targets_name_real_metrics() {
        let m = Metrics { cover_gap: Some(0.0), ..Default::default() };
        let json = serde_json::to_value(&m).unwrap();
        for t in TARGETS {
            assert!(json.get(t.metric).is_some(), "{} is not a metric", t.metric);
        }
        let bad = Metrics { overlaps: 2, four_ways: 0, ..Default::default() };
        let v = violations(&bad);
        assert!(v.iter().any(|v| v.metric == "overlaps" && v.hard));
        assert!(!v.iter().any(|v| v.metric == "four_ways"));
        assert!(!v.iter().any(|v| v.metric == "lot_area_median"), "nothing measured, nothing missed");
    }
}
