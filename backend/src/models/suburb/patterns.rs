//! Street patterns (plan §4.1.5, §4.1.6): what a neighbourhood's frame is
//! filled with.
//!
//! **Building blocks**, all drawn in frame coordinates and mapped to the world:
//! - **row**: an iso-v street. Near a side where an entrance was moved along
//!   the arterial, clear of one across it, every row bends together to meet
//!   it square on (`Row::ramps`).
//! - **rail**: an iso-u link between two rows.
//! - **U-turn**: a half circle of radius S/2 joining two neighbouring rows.
//! - **bulb**: `street_plan::bulb`, r 13, 12 segments, so no turn passes 45°.
//! - **entrance**: a straight stub from a row's gate to the arterial.
//!
//! **Pattern (a), warped parallels** (`reference/aerial-suburban-layout-stockcake.jpg`):
//! - the frame's fitted rows;
//! - rails every L or so, half a spacing out of step between neighbouring
//!   row gaps, so two rails never meet a row at one point (no 4-ways);
//! - each row end is an entrance, a U-turn or a bulb.
//!
//! **Pattern (b), loops and lollipops**, is in `loops.rs` (#109); it
//! builds on the row, ramp and bulb pieces here.
//!
//! **Roads.** Rows joined by U-turns are one road, a *strand*, with id
//! `collector_{sector}_{nbhd}_{k}`. A bulb end is a road of its own,
//! `culdesac_{sector}_{nbhd}_{row}{l|h}`, from the row's last rail junction,
//! so its stem is measured from the network. Rails are
//! `link_{sector}_{nbhd}_{gap}_{j}`.
//!
//! **Joins.** Every end lands exactly on a vertex of its target: rows carry a
//! vertex at each rail, and both are mapped from the same (u, v). Entrance
//! stubs end on the arterial's centre line, which `weld_junctions` then puts
//! into the arterial.

use std::f64::consts::PI;

use capitol_protocol::terrain::{RoadSegment, RoadType};

use super::frame::{Frame, STEP};
use super::loops;
use super::neighbourhood::{Edge, Entrance, Neighbourhood};
use crate::models::geom::polyline_length;
use crate::models::street_plan;
use crate::models::terrain::{NeighbourhoodParams, NeighbourhoodPattern};

type P = (f64, f64);

/// Segments in a cul-de-sac's bulb: 30° a vertex, under the 45° limit.
pub(crate) const BULB_SEGMENTS: usize = 12;
/// No street end comes nearer than this to a road it doesn't join (plan §4.1.8).
pub(crate) const END_CLEARANCE: f64 = 40.0;
/// Where an entrance was moved along the arterial, the rows bend to meet it
/// over at least this much of their length, so the stub needn't slant...
pub(super) const RAMP_MIN: f64 = 200.0;
/// ...and over this many times the shift, so they tilt by under 11° and the
/// rails still cross them at 70° or more.
pub(super) const RAMP_RUN: f64 = 8.0;

/// What a street is for, for lots v2 (#108).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// A strand of rows and U-turns, entrance stubs included.
    Row,
    /// A rail between two rows. Lots don't front it; the rows own the corners.
    Link,
    CulDeSac,
}

/// What the lots walker needs to know about a street (plan §4.2).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StreetMeta {
    pub id: String,
    pub role: Role,
    /// Whether lots front the street's left and right, walking along its points.
    pub fronted: (bool, bool),
    /// A cul-de-sac's bulb centre.
    pub bulb: Option<P>,
    pub sector: u8,
    pub nbhd: u8,
}

/// A neighbourhood's streets.
#[derive(Debug, Default)]
pub(crate) struct Laid {
    pub roads: Vec<RoadSegment>,
    pub meta: Vec<StreetMeta>,
}

/// Lay out a neighbourhood's streets in the pattern its stream picks.
pub(crate) fn lay(n: &mut Neighbourhood, p: &NeighbourhoodParams) -> Laid {
    match choose(n, p) {
        Some(NeighbourhoodPattern::WarpedParallels) => warped_parallels(n, p),
        Some(NeighbourhoodPattern::LoopsAndLollipops) => loops::lay(n, p),
        // (c) comes with #110; until then it never fits.
        Some(NeighbourhoodPattern::CentralLoop) | None => Laid::default(),
    }
}

/// The pattern for a neighbourhood: `force_pattern` if it fits, otherwise a
/// weighted draw among those that fit. One draw is always taken, so the
/// stream moves on the same whatever the outcome.
pub(crate) fn choose(n: &mut Neighbourhood, p: &NeighbourhoodParams) -> Option<NeighbourhoodPattern> {
    let draw = n.rng.next_f64();
    let w = &p.pattern_weights;
    let fitting: Vec<(NeighbourhoodPattern, f64)> = [
        (NeighbourhoodPattern::WarpedParallels, w.warped_parallels, parallels_fit(&n.frame, p)),
        (NeighbourhoodPattern::LoopsAndLollipops, w.loops_and_lollipops, loops::fits(&n.frame, p)),
        (NeighbourhoodPattern::CentralLoop, w.central_loop, false),
    ]
    .into_iter()
    .filter(|&(_, weight, fits)| fits && weight > 0.0)
    .map(|(pattern, weight, _)| (pattern, weight))
    .collect();
    if let Some(forced) = p.force_pattern.filter(|f| fitting.iter().any(|(x, _)| x == f)) {
        return Some(forced);
    }
    pick(&fitting, draw)
}

/// The item a draw in [0, 1) lands on, by weight.
pub(super) fn pick<T: Copy>(weighted: &[(T, f64)], draw: f64) -> Option<T> {
    let total: f64 = weighted.iter().map(|w| w.1).sum();
    let mut at = draw * total;
    for &(item, weight) in weighted {
        if at < weight {
            return Some(item);
        }
        at -= weight;
    }
    weighted.last().map(|w| w.0)
}

/// Sizes every pattern shares, from the params and the frame's row fit.
#[derive(Debug, Clone, Copy)]
struct Sizes {
    /// From a side line to the far edge of a bulb or U-turn: a lot's depth
    /// and half a street.
    clear: f64,
    /// U-turn radius: half the row spacing.
    turn: f64,
    bulb: f64,
}

impl Sizes {
    fn of(frame: &Frame, p: &NeighbourhoodParams) -> Option<Sizes> {
        let fit = frame.fit.as_ref()?;
        let pitch = frame.depth / fit.rows.len() as f64;
        Some(Sizes {
            clear: fit.lot_depth + p.local_width / 2.0,
            turn: pitch / 2.0,
            bulb: p.bulb_radius,
        })
    }

    /// From the side line to a bulb's entry, where it closes on its stem.
    fn bulb_entry(&self) -> f64 {
        self.clear + 2.0 * self.bulb
    }

    /// From the side line to a U-turn's centre, where its rows go straight.
    fn turn_centre(&self) -> f64 {
        self.clear + self.turn
    }
}

/// How far a row's other vertices keep from a rail's, in u. At the inner
/// edge a unit of u is about 0.8 in the world, so this is over `SNAP`.
pub(super) const MARK_CLEAR: f64 = 0.6 * STEP;

/// The least room between the rails' margins for a pattern (a) layout.
const MIN_RAIL_SPAN: f64 = 100.0;

/// Pattern (a) fits a frame with two rows or more, wide enough for a bulb
/// at each end and rails between.
pub(crate) fn parallels_fit(frame: &Frame, p: &NeighbourhoodParams) -> bool {
    let Some(s) = Sizes::of(frame, p) else { return false };
    let rows = frame.fit.as_ref().map_or(0, |f| f.rows.len());
    rows >= 2 && frame.width >= 2.0 * (s.bulb_entry() + END_CLEARANCE) + MIN_RAIL_SPAN
}

/// A row street: iso-v at `v`, plus any ramps.
#[derive(Debug, Clone)]
pub(super) struct Row {
    pub v: f64,
    pub ramps: Vec<Ramp>,
}

/// A smooth shift of a row's v by `dv` at u = `at`, easing to nothing `len` away.
#[derive(Debug, Clone, Copy)]
pub(super) struct Ramp {
    pub at: f64,
    pub len: f64,
    pub dv: f64,
}

impl Row {
    pub(super) fn v_at(&self, u: f64) -> f64 {
        self.v
            + self
                .ramps
                .iter()
                .map(|r| {
                    let t = (1.0 - (u - r.at).abs() / r.len).clamp(0.0, 1.0);
                    r.dv * t * t * (3.0 - 2.0 * t)
                })
                .sum::<f64>()
    }
}

/// How a row ends at one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// Through the side line to the arterial, as `n.entrances[i]`.
    Entrance(usize),
    /// A U-turn to the next row up...
    Up,
    /// ...or down.
    Down,
    Bulb,
}

/// Every way a side's row ends can go: entrances where the neighbourhood has
/// them, and each free row a bulb or half of a U-turn with a free neighbour.
fn side_options(entrances: &[Option<usize>]) -> Vec<Vec<End>> {
    fn from(k: usize, entrances: &[Option<usize>], so_far: &mut Vec<End>, out: &mut Vec<Vec<End>>) {
        if k == entrances.len() {
            out.push(so_far.clone());
            return;
        }
        if let Some(e) = entrances[k] {
            so_far.push(End::Entrance(e));
            from(k + 1, entrances, so_far, out);
            so_far.pop();
            return;
        }
        so_far.push(End::Bulb);
        from(k + 1, entrances, so_far, out);
        so_far.pop();
        if k + 1 < entrances.len() && entrances[k + 1].is_none() {
            so_far.extend([End::Up, End::Down]);
            from(k + 2, entrances, so_far, out);
            so_far.truncate(so_far.len() - 2);
        }
    }
    let mut out = Vec::new();
    from(0, entrances, &mut Vec::new(), &mut out);
    out
}

/// Steps from `a` to `b`, both included, at about `STEP` apart, with a step
/// at each of `marks` between them and no other within `MARK_CLEAR` of one.
pub(super) fn marked_steps(a: f64, b: f64, marks: &[f64]) -> Vec<f64> {
    let (lo, hi) = (a.min(b), a.max(b));
    let n = ((hi - lo) / STEP).ceil().max(1.0) as usize;
    let inner: Vec<f64> = marks.iter().copied().filter(|&m| m > lo && m < hi).collect();
    let mut steps: Vec<f64> = (1..n)
        .map(|i| lo + (hi - lo) * i as f64 / n as f64)
        .filter(|&x| inner.iter().all(|&m| (x - m).abs() > MARK_CLEAR))
        .chain(inner.iter().copied())
        .collect();
    steps.sort_by(f64::total_cmp);
    let mut all = vec![lo];
    all.extend(steps);
    all.push(hi);
    if a > b {
        all.reverse();
    }
    all
}

/// A row street from `u0` to `u1`, with a vertex at each of `marks`.
pub(super) fn row_points(f: &Frame, row: &Row, u0: f64, u1: f64, marks: &[f64]) -> Vec<P> {
    marked_steps(u0, u1, marks).into_iter().map(|u| f.to_world(u, row.v_at(u))).collect()
}

/// Side 0 is the frame's lo side (u = 0), side 1 its hi side (u = width).
const SIDES: [Edge; 2] = [Edge::Lo, Edge::Hi];

/// One way of laying pattern (a) out: the row ends at each side and where
/// the rails go.
#[derive(Debug, Clone)]
struct Parallels {
    ends: [Vec<End>; 2],
    /// Rail u values in each gap between rows, in order.
    rails: Vec<Vec<f64>>,
    /// Each row's bulb entry at each side, as u, where it ends in a bulb.
    entries: [Vec<f64>; 2],
    /// The longest cul-de-sac stem, in world units, before any is pulled in.
    worst_stem: f64,
}

struct Ctx<'a> {
    f: &'a Frame,
    s: Sizes,
    rows: Vec<Row>,
}

impl Ctx<'_> {
    /// u at `x` in from a side line.
    fn u(&self, side: usize, x: f64) -> f64 {
        if side == 0 {
            x
        } else {
            self.f.width - x
        }
    }

    fn at(&self, k: usize, u: f64) -> P {
        self.f.to_world(u, self.rows[k].v_at(u))
    }

    /// Row k from `u0` to `u1`, at about `STEP` apart, with a vertex at each
    /// of `marks` between them, so rails meet it on a vertex. No other vertex
    /// comes within `MARK_CLEAR` of a mark: `weld_junctions` snaps an end to
    /// the first vertex of a segment within `street_plan::SNAP`, even when the
    /// next one is exactly on it (#114).
    fn row(&self, k: usize, u0: f64, u1: f64, marks: &[f64]) -> Vec<P> {
        row_points(self.f, &self.rows[k], u0, u1, marks)
    }

    /// The rails that meet row k, in order of u.
    fn rails_on(&self, k: usize, rails: &[Vec<f64>]) -> Vec<f64> {
        let mut out: Vec<f64> = rails.get(k.wrapping_sub(1)).into_iter().chain(rails.get(k)).flatten().copied().collect();
        out.sort_by(f64::total_cmp);
        out
    }

    /// The rail on row k nearest a side.
    fn nearest_rail(&self, k: usize, side: usize, rails: &[Vec<f64>]) -> Option<f64> {
        let on = self.rails_on(k, rails);
        if side == 0 {
            on.first().copied()
        } else {
            on.last().copied()
        }
    }

    /// The U-turn's points between rows `from` and `to` at a side, both
    /// ends left out: the rows carry them.
    fn u_turn(&self, side: usize, from: usize, to: usize) -> Vec<P> {
        let (a, b) = (self.rows[from].v, self.rows[to].v);
        let (mid, r) = ((a + b) / 2.0, (b - a).abs() / 2.0);
        let c = self.u(side, self.s.turn_centre());
        let toward = if side == 0 { -1.0 } else { 1.0 };
        let m = (PI * r / STEP).ceil().max(4.0) as usize;
        (1..m)
            .map(|i| {
                // From `from` (φ = −90°) round the apex (0°) to `to` (90°).
                let phi = -PI / 2.0 + PI * i as f64 / m as f64;
                let u = c + toward * r * phi.cos();
                // The rows' ramps, blended from one to the other.
                let t = (1.0 + phi.sin()) / 2.0;
                let bend = (1.0 - t) * (self.rows[from].v_at(u) - a) + t * (self.rows[to].v_at(u) - b);
                self.f.to_world(u, mid + (b - a).signum() * r * phi.sin() + bend)
            })
            .collect()
    }

    /// Pattern (a) with these ends and rail phase, or None if the rails have
    /// no room.
    fn parallels(&self, ends: [Vec<End>; 2], phase: usize, p: &NeighbourhoodParams) -> Option<Parallels> {
        let n = self.rows.len();
        let start = |e: End| match e {
            End::Entrance(_) => self.s.clear,
            End::Up | End::Down => self.s.turn_centre(),
            End::Bulb => self.s.bulb_entry() + END_CLEARANCE,
        };
        let margin = |side: usize| ends[side].iter().map(|&e| start(e)).fold(0.0, f64::max);
        let (lo, hi) = (margin(0), self.f.width - margin(1));
        let span = hi - lo;
        if span < MIN_RAIL_SPAN {
            return None;
        }
        let count = (span / p.link_spacing).round().max(1.0);
        let step = span / count;
        let rails: Vec<Vec<f64>> = (0..n - 1)
            .map(|gap| {
                let offset = if (gap + phase) % 2 == 0 { 0.25 } else { 0.75 };
                (0..count as usize).map(|j| lo + (j as f64 + offset) * step).collect()
            })
            .collect();
        let mut entries = [vec![f64::NAN; n], vec![f64::NAN; n]];
        let mut worst_stem: f64 = 0.0;
        for side in 0..2 {
            for k in 0..n {
                if ends[side][k] != End::Bulb {
                    continue;
                }
                let entry = self.u(side, self.s.bulb_entry());
                let rail = self.nearest_rail(k, side, &rails)?;
                worst_stem = worst_stem.max(polyline_length(&self.row(k, rail, entry, &[])));
                entries[side][k] = entry;
            }
        }
        Some(Parallels { ends, rails, entries, worst_stem })
    }
}

/// Pattern (a): warped parallels.
fn warped_parallels(n: &mut Neighbourhood, p: &NeighbourhoodParams) -> Laid {
    let f = n.frame.clone();
    let Some(s) = Sizes::of(&f, p) else { return Laid::default() };
    let Some(fit) = f.fit.clone() else { return Laid::default() };
    let rows = fit.rows.iter().map(|&v| Row { v, ramps: Vec::new() }).collect();
    let mut cx = Ctx { f: &f, s, rows };
    let count = fit.rows.len();

    // Entrances at each side, by row, and the ramps to the moved ones.
    let mut at_side = [vec![None; count], vec![None; count]];
    for (i, e) in n.entrances.iter().enumerate() {
        let side = SIDES.iter().position(|&x| x == e.edge).expect("an edge");
        at_side[side][e.row] = Some(i);
        if let Some(ramp) = ramp_for(&cx, side, e) {
            cx.rows[e.row].ramps.push(ramp);
        }
    }
    // The other rows bend with them, so the band keeps its spacing. Across an
    // arterial every row sits off by the same warp difference, so a side's
    // ramps nearly always agree. `allocate_entrances` moves a side's
    // entrances together; one it couldn't move keeps its row straight.
    for side in 0..2 {
        let at = cx.u(side, 0.0);
        let first = cx.rows.iter().flat_map(|r| &r.ramps).find(|r| r.at == at).copied();
        let Some(ramp) = first else { continue };
        for (k, row) in cx.rows.iter_mut().enumerate() {
            if at_side[side][k].is_none() && !row.ramps.iter().any(|r| r.at == at) {
                row.ramps.push(ramp);
            }
        }
    }

    // Every layout the ends allow, both rail phases; a seeded pick among
    // those whose cul-de-sacs are short enough, U-turns favoured.
    let longest = p.cul_de_sac_max - STEP;
    let mut options = Vec::new();
    for phase in 0..2 {
        for lo in side_options(&at_side[0]) {
            for hi in side_options(&at_side[1]) {
                options.extend(cx.parallels([lo.clone(), hi], phase, p));
            }
        }
    }
    let draw = n.rng.next_f64();
    let weighted: Vec<(usize, f64)> = options
        .iter()
        .enumerate()
        .filter(|(_, o)| o.worst_stem <= longest)
        .map(|(i, o)| (i, 1.0 + o.ends.iter().flatten().filter(|&&e| e == End::Up).count() as f64))
        .collect();
    let chosen = pick(&weighted, draw)
        .or_else(|| (0..options.len()).min_by(|&a, &b| options[a].worst_stem.total_cmp(&options[b].worst_stem)));
    let Some(mut layout) = chosen.map(|i| options.swap_remove(i)) else { return Laid::default() };
    pull_in_long_stems(&cx, &mut layout, longest);
    build(&cx, n, &layout, p)
}

/// The ramp that bends a row so its entrance stub meets the arterial square
/// on, where the entrance was moved along the arterial to keep clear of one
/// across it, and its `at` isn't the gate's foot.
fn ramp_for(cx: &Ctx, side: usize, e: &Entrance) -> Option<Ramp> {
    let f = cx.f;
    let inset = if side == 0 { f.lo.inset } else { f.hi.inset };
    let u = cx.u(side, 0.0);
    // On the side line, a point at radius r is sqrt(r² − inset²) along the arterial.
    let r = (e.along * e.along + inset * inset).sqrt();
    let dv = r - f.base - f.warp.at(u) - cx.rows[e.row].v;
    (dv.abs() > 1e-6).then(|| Ramp { at: u, len: RAMP_MIN.max(RAMP_RUN * dv.abs()), dv })
}

/// Pull any bulb whose stem is too long in towards its rail.
fn pull_in_long_stems(cx: &Ctx, layout: &mut Parallels, longest: f64) {
    for side in 0..2 {
        for k in 0..cx.rows.len() {
            let entry = layout.entries[side][k];
            if entry.is_nan() {
                continue;
            }
            let Some(rail) = cx.nearest_rail(k, side, &layout.rails) else { continue };
            let stem = polyline_length(&cx.row(k, rail, entry, &[]));
            if stem > longest {
                layout.entries[side][k] = rail + (entry - rail) * longest / stem;
            }
        }
    }
}

pub(super) fn road(id: String, points: Vec<P>, width: f64) -> RoadSegment {
    RoadSegment { id, road_type: RoadType::Suburban, points, width }
}

/// The roads of a chosen pattern (a) layout.
fn build(cx: &Ctx, n: &Neighbourhood, layout: &Parallels, p: &NeighbourhoodParams) -> Laid {
    let rows = cx.rows.len();
    let tag = format!("{}_{}", n.sector, n.index);
    let width = p.local_width;
    let mut out = Laid::default();
    let meta = |id: &str, role, fronted, bulb| StreetMeta {
        id: id.to_string(),
        role,
        fronted,
        bulb,
        sector: n.sector,
        nbhd: n.index,
    };

    // Rails.
    for (gap, rails) in layout.rails.iter().enumerate() {
        for (j, &u) in rails.iter().enumerate() {
            let (a, b) = (cx.rows[gap].v_at(u), cx.rows[gap + 1].v_at(u));
            let m = ((b - a) / STEP).ceil().max(1.0) as usize;
            let mut pts: Vec<P> = vec![cx.at(gap, u)];
            pts.extend((1..m).map(|i| cx.f.to_world(u, a + (b - a) * i as f64 / m as f64)));
            pts.push(cx.at(gap + 1, u));
            let id = format!("link_{tag}_{gap}_{j}");
            out.meta.push(meta(&id, Role::Link, (false, false), None));
            out.roads.push(road(id, pts, width));
        }
    }

    // Cul-de-sacs: from the row's rail nearest the bulb to the bulb.
    for side in 0..2 {
        for k in 0..rows {
            let entry = layout.entries[side][k];
            if entry.is_nan() {
                continue;
            }
            let rail = cx.nearest_rail(k, side, &layout.rails).expect("every row has a rail");
            let mut pts = cx.row(k, rail, entry, &[]);
            let dir = street_plan::end_direction(&pts).expect("a stem");
            let stem_end = *pts.last().expect("a stem");
            let centre = (stem_end.0 + p.bulb_radius * dir.0, stem_end.1 + p.bulb_radius * dir.1);
            pts.extend(street_plan::bulb(stem_end, dir, p.bulb_radius, BULB_SEGMENTS));
            let id = format!("culdesac_{tag}_{k}{}", ["l", "h"][side]);
            out.meta.push(meta(&id, Role::CulDeSac, (true, true), Some(centre)));
            out.roads.push(road(id, pts, width));
        }
    }

    // Strands: rows chained through their U-turns.
    let mut done = vec![false; rows];
    let mut strands = Vec::new();
    // Strands that end somewhere first, then the closed ones.
    let starts: Vec<(usize, usize)> = (0..rows)
        .flat_map(|k| (0..2).map(move |side| (k, side)))
        .filter(|&(k, side)| !matches!(layout.ends[side][k], End::Up | End::Down))
        .chain((0..rows).map(|k| (k, 0)))
        .collect();
    for (k, side) in starts {
        if done[k] {
            continue;
        }
        let pts = strand(cx, n, layout, k, side, &mut done);
        if pts.len() >= 2 {
            strands.push(pts);
        }
    }
    for (i, pts) in strands.into_iter().enumerate() {
        let id = format!("collector_{tag}_{i}");
        out.meta.push(meta(&id, Role::Row, (true, true), None));
        out.roads.push(road(id, pts, width));
    }
    out
}

/// The strand that starts at row k's end at `side` and runs until it ends,
/// or comes back round to where it began.
fn strand(cx: &Ctx, n: &Neighbourhood, layout: &Parallels, k0: usize, side0: usize, done: &mut [bool]) -> Vec<P> {
    let rails = &layout.rails;
    // Where a row's street stops at a side: an entrance's gate, a U-turn's
    // centre, or the rail a cul-de-sac leaves from.
    let stop = |k: usize, side: usize| match layout.ends[side][k] {
        End::Entrance(_) => cx.u(side, 0.0),
        End::Up | End::Down => cx.u(side, cx.s.turn_centre()),
        End::Bulb => cx.nearest_rail(k, side, rails).expect("every row has a rail"),
    };
    let stub = |k: usize, side: usize, e: usize| -> Vec<P> {
        street_plan::resample(&[cx.at(k, cx.u(side, 0.0)), n.entrances[e].at], STEP)
    };
    let closed = matches!(layout.ends[side0][k0], End::Up | End::Down);

    // Pieces meet on a shared point; `dedupe` drops the repeat.
    let mut pts: Vec<P> = Vec::new();
    // A closed strand starts and ends at the row's first rail.
    let first = if closed { cx.rails_on(k0, rails)[0] } else { stop(k0, side0) };
    if let End::Entrance(e) = layout.ends[side0][k0] {
        pts.extend(stub(k0, side0, e).into_iter().rev());
    }
    let (mut k, mut from_u, mut toward) = (k0, first, 1 - side0);
    loop {
        done[k] = true;
        pts.extend(cx.row(k, from_u, stop(k, toward), &cx.rails_on(k, rails)));
        match layout.ends[toward][k] {
            End::Entrance(e) => {
                pts.extend(stub(k, toward, e));
                break;
            }
            End::Bulb => break,
            end @ (End::Up | End::Down) => {
                let next = if end == End::Up { k + 1 } else { k - 1 };
                pts.extend(cx.u_turn(toward, k, next));
                if closed && next == k0 {
                    // Back round: the last stretch to where it began.
                    pts.extend(cx.row(k0, stop(k0, toward), first, &cx.rails_on(k0, rails)));
                    break;
                }
                (k, from_u, toward) = (next, stop(next, toward), 1 - toward);
            }
        }
    }
    street_plan::dedupe(&pts)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::suburb::graph::{Graph, JunctionKind};
    use crate::models::suburb::metrics::{self, Plan, Scope};
    use crate::models::terrain::{SuburbLayout, TerrainConfig, WorldGenParams};

    pub(crate) fn parallels_world(seed: u64) -> WorldGenParams {
        let mut params = WorldGenParams { seed, ..Default::default() };
        params.suburban.layout = SuburbLayout::Neighbourhood;
        params.suburban.neighbourhood.force_pattern = Some(NeighbourhoodPattern::WarpedParallels);
        params
    }

    #[test]
    fn side_options_pair_only_free_neighbours() {
        let all = side_options(&[None, None, None]);
        // Three bulbs, or a U-turn and a bulb either way round.
        assert_eq!(all.len(), 3);
        assert!(all.contains(&vec![End::Up, End::Down, End::Bulb]));
        assert!(all.contains(&vec![End::Bulb, End::Up, End::Down]));
        // An entrance in the middle leaves the outer two as bulbs.
        assert_eq!(side_options(&[None, Some(0), None]), vec![vec![End::Bulb, End::Entrance(0), End::Bulb]]);
    }

    #[test]
    fn pick_follows_the_weights() {
        let w = [('a', 1.0), ('b', 3.0)];
        assert_eq!(pick(&w, 0.0), Some('a'));
        assert_eq!(pick(&w, 0.24), Some('a'));
        assert_eq!(pick(&w, 0.26), Some('b'));
        assert_eq!(pick(&w, 0.999), Some('b'));
        assert_eq!(pick::<char>(&[], 0.5), None);
    }

    /// Acceptance for #107: pattern (a) everywhere, at the four report seeds.
    #[test]
    fn warped_parallels_meet_their_targets() {
        for seed in [42, 7, 1234, 99999] {
            let params = parallels_world(seed);
            let t = TerrainConfig::from_params(&params);
            let scope = Scope {
                center: (t.center_x, t.center_y),
                inner: params.urban_outer_radius,
                outer: params.suburban_outer_radius,
                sector: None,
            };
            let plan = Plan { roads: &t.roads, lots: &[], before: &[], cleared: &[], scope, coverage: false };
            let m = metrics::measure(&plan).metrics;
            let why = format!("seed {seed}: {m:#?}");
            assert!(m.suburban_roads > 50, "{why}");
            assert_eq!(m.audit_issues, 0, "{why}");
            assert_eq!(m.four_ways, 0, "{why}");
            assert!(m.min_junction_angle.unwrap() >= 70.0, "{why}");
            assert!(m.t_spacing_min.unwrap() >= 40.0, "{why}");
            assert!(m.culdesac_max_len.unwrap() <= 300.0, "{why}");
            assert_eq!(m.connected_share, Some(1.0), "{why}");
            assert!(m.parallel_spacing_p5.unwrap() >= 200.0, "{why}");
        }
    }

    #[test]
    fn every_neighbourhood_gets_rows_rails_and_its_entrances() {
        let params = parallels_world(42).suburban_params();
        let plan = crate::models::suburb::plan(&params, (4000.0, 4000.0));
        assert_eq!(plan.roads.len(), plan.meta.len());
        for n in &plan.neighbourhoods {
            let mine: Vec<_> = plan.meta.iter().filter(|m| (m.sector, m.nbhd) == (n.sector, n.index)).collect();
            assert!(mine.iter().any(|m| m.role == Role::Row), "s{}·{}: no rows", n.sector, n.index);
            assert!(mine.iter().any(|m| m.role == Role::Link), "s{}·{}: no rails", n.sector, n.index);
            // Every entrance's `at` is the end of a road.
            for e in &n.entrances {
                let ends = plan.roads.iter().flat_map(|r| [r.points[0], *r.points.last().unwrap()]);
                assert!(ends.into_iter().any(|q| q == e.at), "s{}·{}: entrance not reached", n.sector, n.index);
            }
        }
        for (r, m) in plan.roads.iter().zip(&plan.meta) {
            assert_eq!(r.id, m.id);
            assert!(r.points.len() >= 10, "{} has {} points", r.id, r.points.len());
            assert_eq!(m.bulb.is_some(), m.role == Role::CulDeSac);
        }
    }

    #[test]
    fn rails_meet_rows_on_their_vertices_and_never_line_up() {
        let params = parallels_world(7).suburban_params();
        let plan = crate::models::suburb::plan(&params, (4000.0, 4000.0));
        let g = Graph::build(&plan.roads);
        for r in plan.roads.iter().filter(|r| r.id.starts_with("link_")) {
            for end in [r.points[0], *r.points.last().unwrap()] {
                let shared = plan.roads.iter().filter(|o| o.id != r.id && o.points.contains(&end)).count();
                assert!(shared >= 1, "{} ends off a row at {end:?}", r.id);
            }
        }
        assert!(g.junctions().iter().all(|j| j.kind == JunctionKind::Tee), "a 4-way before welding");
    }

    #[test]
    fn a_u_turn_is_smooth_and_meets_both_rows() {
        let params = parallels_world(1234).suburban_params();
        let mut ns = crate::models::suburb::neighbourhood::neighbourhoods(&params, (4000.0, 4000.0));
        let n = &mut ns[0];
        let f = n.frame.clone();
        let p = &params.neighbourhood;
        let s = Sizes::of(&f, p).unwrap();
        let rows = f.fit.as_ref().unwrap().rows.iter().map(|&v| Row { v, ramps: Vec::new() }).collect();
        let cx = Ctx { f: &f, s, rows };
        for side in 0..2 {
            let c = cx.u(side, s.turn_centre());
            let mut pts = vec![cx.at(0, c)];
            pts.extend(cx.u_turn(side, 0, 1));
            pts.push(cx.at(1, c));
            assert!(street_plan::sharpest_turn(&pts) < 10.0, "{}", street_plan::sharpest_turn(&pts));
            assert!(!street_plan::self_crosses(&pts));
            // The apex sits a lot's depth and half a street in from the side line.
            let apex = pts.iter().map(|&q| cx.f.from_world(q).0).fold(f64::NAN, |a, u| if side == 0 { a.min(u) } else { a.max(u) });
            assert!((cx.u(side, s.clear) - apex).abs() < 1.0, "apex at u {apex}");
        }
    }
}
