//! Pattern (b), loops and lollipops (plan §4.1.6;
//! `reference/dafb5b4c5614ca76a703f53754f4b452.jpg`, `street_layouts.jpg` PW1).
//!
//! - **Racetrack.** A collector inset one lot row from every edge of the
//!   frame. It has two rows, on the frame's first and last, and two *legs*,
//!   each a lot row in from its side line, joined by corners of radius
//!   `clear`. The lots between it and the edge back onto the buffers and the
//!   edge strips.
//! - **Combs.** Cul-de-sacs off the racetrack's two rows, pointing in, a
//!   street spacing apart, so neighbouring stems' lots back onto each other.
//!   The outer comb is half a spacing out of step with the inner, so each
//!   bulb points between two across the middle of the band, and the pies
//!   fill it.
//! - **Loops.** A seeded few neighbouring pairs of stems are joined at their
//!   tips into one loop street instead.
//! - **Entrances.** One on a middle frame row is a stub from the leg out
//!   through the lot row. One on the first or last row squares off that
//!   corner: the row runs on through the side line, and the leg ends on it in
//!   a T.
//!
//! **Roads.** The racetrack is `collector_{s}_{n}_{k}`: one closed road, or
//! one per stretch between squared corners, split at a stem junction where a
//! stretch would end on itself. Stems are
//! `culdesac_{s}_{n}_{i|o}{j}`, loops `loop_{s}_{n}_{i|o}{j}` (i for the comb
//! off the inner row, o the outer) and stubs `entrance_{s}_{n}_{k}`.
//!
//! **Joins.** As in pattern (a), every end lands exactly on a vertex of its
//! target, both mapped from the same (u, v).

use capitol_protocol::fmath::StableMath;
use std::f64::consts::{FRAC_PI_2, PI};

use super::frame::{Frame, STEP};
use super::neighbourhood::{Edge, Neighbourhood};
use super::patterns::{
    marked_steps, road, row_points, Laid, Ramp, Role, Row, StreetMeta, BULB_SEGMENTS, RAMP_MIN, RAMP_RUN,
};
use crate::models::geom::{polyline_length, SplitMix64};
use crate::models::street_plan;
use crate::models::terrain::NeighbourhoodParams;

type P = (f64, f64);

/// The chance that a stem and the next one are joined into a loop street.
const LOOP_CHANCE: f64 = 0.25;
/// Neighbouring stems are at least this share of a street spacing apart.
const MIN_SPACING: f64 = 0.9;
/// The racetrack's corners, in ring order, as (side, row): 0 is the lo side
/// or the inner row, 1 the hi side or the outer row. Piece k of the ring runs
/// from corner k to corner k + 1: the inner row, the hi leg, the outer row
/// back, the lo leg down.
const CORNERS: [(usize, usize); 4] = [(0, 0), (1, 0), (1, 1), (0, 1)];

/// Pattern (b) fits a frame of three rows or more, at least 4S deep and 3S
/// across at its first row, where S is a nominal street spacing. That leaves
/// each comb room for a stem.
pub(crate) fn fits(frame: &Frame, p: &NeighbourhoodParams) -> bool {
    let Some(fit) = &frame.fit else { return false };
    let s = 2.0 * p.lot_depth + p.local_width;
    let r = frame.base + fit.rows[0];
    fit.rows.len() >= 3 && frame.depth >= 4.0 * s && r * frame.span(r) >= 3.0 * s
}

/// World units per unit of u at depth v, the warp aside.
fn scale(f: &Frame, v: f64) -> f64 {
    let r = f.base + v;
    r * f.span(r) / f.width
}

/// A stem off a racetrack row, or two joined into a loop, at these u.
#[derive(Debug, Clone, Copy)]
enum Tooth {
    CulDeSac(f64),
    Loop(f64, f64),
}

struct Track<'a> {
    f: &'a Frame,
    p: &'a NeighbourhoodParams,
    /// The row fit's lot depth.
    lot_depth: f64,
    /// From a side line to a leg's centre line, in the world, and the corner
    /// radius: half the row pitch, like the rows' own inset.
    clear: f64,
    /// The inner and outer rows.
    rows: [Row; 2],
}

impl Track<'_> {
    /// u at `x` in from a side line (0 lo, 1 hi), in the world, at depth v.
    fn side_u(&self, side: usize, x: f64, v: f64) -> f64 {
        self.f.side_u(side, x, v)
    }

    fn leg_u(&self, side: usize, v: f64) -> f64 {
        self.side_u(side, self.clear, v)
    }

    fn leg_at(&self, side: usize, v: f64) -> P {
        self.f.to_world(self.leg_u(side, v), v)
    }

    /// Where the leg at `side` meets row `top` in a T, as u on the row.
    fn tee_u(&self, side: usize, top: usize) -> f64 {
        let row = &self.rows[top];
        let mut u = self.leg_u(side, row.v);
        for _ in 0..4 {
            u = self.leg_u(side, row.v_at(u));
        }
        u
    }

    fn tee(&self, side: usize, top: usize) -> P {
        let u = self.tee_u(side, top);
        self.f.to_world(u, self.rows[top].v_at(u))
    }

    /// Where row `top` stops at `side`: the side line at a squared corner,
    /// otherwise where its rounded corner starts.
    fn row_end(&self, side: usize, top: usize, squared: bool) -> f64 {
        match (squared, side) {
            (true, 0) => 0.0,
            (true, _) => self.f.width,
            (false, 0) => self.leg_u(0, self.rows[top].v) + self.clear,
            (false, _) => self.leg_u(1, self.rows[top].v) - self.clear,
        }
    }

    /// Where the leg at a side stops at row `top`: on the row at a squared
    /// corner, otherwise where the rounded corner ends.
    fn leg_end(&self, side: usize, top: usize, squared: bool) -> f64 {
        match (squared, top) {
            (true, _) => self.rows[top].v_at(self.tee_u(side, top)),
            (false, 0) => self.rows[0].v + self.clear,
            (false, _) => self.rows[1].v - self.clear,
        }
    }

    /// The rounded corner at `side` of row `top`, from the row's end to the
    /// leg's, both left out. It's a quarter circle in coordinates measured
    /// from the leg, and takes the row's bend at its row end.
    fn corner(&self, side: usize, top: usize) -> Vec<P> {
        let (row, r) = (&self.rows[top], self.clear);
        let m = (FRAC_PI_2 * r / STEP).ceil().max(4.0) as usize;
        (1..m)
            .map(|i| {
                let phi = FRAC_PI_2 * i as f64 / m as f64;
                let x = r - r * phi.stable_sin();
                let v = if top == 0 { row.v + r - r * phi.stable_cos() } else { row.v - r + r * phi.stable_cos() };
                let u = if side == 0 { self.leg_u(0, v) + x } else { self.leg_u(1, v) - x };
                let bend = (row.v_at(u) - row.v) * phi.stable_cos();
                self.f.to_world(u, v + bend)
            })
            .collect()
    }

    /// The leg at `side` from v0 to v1, with a vertex at each of `marks`.
    fn leg(&self, side: usize, v0: f64, v1: f64, marks: &[f64]) -> Vec<P> {
        marked_steps(v0, v1, marks).into_iter().map(|v| self.leg_at(side, v)).collect()
    }

    /// The comb off row `top`: stems a street spacing apart, kept a leg's
    /// lots and a stem's lots in from each leg, at the comb's mid-depth.
    fn comb(&self, top: usize, rng: &mut SplitMix64) -> Vec<Tooth> {
        let (f, p) = (self.f, self.p);
        let mid = (self.rows[top].v + f.depth / 2.0) / 2.0;
        let x = self.clear + p.collector_width / 2.0 + 2.0 * self.lot_depth + p.local_width / 2.0;
        let (lo, hi) = (self.side_u(0, x, mid), self.side_u(1, x, mid));
        if hi < lo {
            return Vec::new();
        }
        let s = 2.0 * self.lot_depth + p.local_width;
        let across = (hi - lo) * scale(f, mid);
        let mut gaps = (across / s).round() as usize;
        while gaps > 0 && across / (gaps as f64) < MIN_SPACING * s {
            gaps -= 1;
        }
        // The outer comb sits half a spacing out of step with the inner, so
        // each bulb points between two across the band, and their pies fill
        // the ground that four bulbs facing in pairs would leave.
        let us: Vec<f64> = match (gaps, top) {
            (0, _) => vec![(lo + hi) / 2.0],
            (g, 0) => (0..=g).map(|i| lo + (hi - lo) * i as f64 / g as f64).collect(),
            (g, _) => (0..g).map(|i| lo + (hi - lo) * (i as f64 + 0.5) / g as f64).collect(),
        };
        let mut teeth = Vec::new();
        let mut j = 0;
        while j < us.len() {
            if j + 1 < us.len() && rng.next_f64() < LOOP_CHANCE && self.loop_street(top, us[j], us[j + 1]).is_some() {
                teeth.push(Tooth::Loop(us[j], us[j + 1]));
                j += 2;
            } else {
                teeth.push(Tooth::CulDeSac(us[j]));
                j += 1;
            }
        }
        teeth
    }

    /// Which way row `top`'s comb points, in v.
    fn inward(top: usize) -> f64 {
        if top == 0 {
            1.0
        } else {
            -1.0
        }
    }

    /// A cul-de-sac off row `top` at u: its points, bulb included, and the
    /// bulb's centre. The pies want to reach the middle of the band, where
    /// the other comb's meet them; the stem stops at `longest`.
    fn cul_de_sac(&self, top: usize, u: f64, longest: f64) -> (Vec<P>, P) {
        let (f, p) = (self.f, self.p);
        let dir = Self::inward(top);
        let v0 = self.rows[top].v_at(u);
        let entry = f.depth / 2.0 - dir * (self.lot_depth + p.local_width / 2.0 + 2.0 * p.bulb_radius);
        let mut pts = f.iso_u(u, v0, entry);
        let len = polyline_length(&pts);
        if len > longest {
            pts = f.iso_u(u, v0, v0 + (entry - v0) * longest / len);
        }
        let d = street_plan::end_direction(&pts).expect("a stem");
        let tip = *pts.last().expect("a stem");
        let centre = (tip.0 + p.bulb_radius * d.0, tip.1 + p.bulb_radius * d.1);
        pts.extend(street_plan::bulb(tip, d, p.bulb_radius, BULB_SEGMENTS));
        (pts, centre)
    }

    /// A loop street off row `top`, out at `ua` and back at `ub`: two stems
    /// joined by a half circle, its apex a lot's depth short of the middle
    /// of the band. None if the stems would have no length.
    fn loop_street(&self, top: usize, ua: f64, ub: f64) -> Option<Vec<P>> {
        let (f, p) = (self.f, self.p);
        let dir = Self::inward(top);
        let apex = f.depth / 2.0 - dir * (self.lot_depth + p.local_width / 2.0);
        let (uc, ru) = ((ua + ub) / 2.0, (ub - ua) / 2.0);
        let rv = ru * scale(f, apex);
        let vc = apex - dir * rv;
        let (va, vb) = (self.rows[top].v_at(ua), self.rows[top].v_at(ub));
        if (vc - va) * dir < 2.0 * STEP || (vc - vb) * dir < 2.0 * STEP {
            return None;
        }
        let mut pts = f.iso_u(ua, va, vc);
        let m = (PI * ru.max(rv) / STEP).ceil().max(4.0) as usize;
        pts.extend((1..m).map(|i| {
            let phi = PI * i as f64 / m as f64;
            f.to_world(uc - ru * phi.stable_cos(), vc + dir * rv * phi.stable_sin())
        }));
        pts.extend(f.iso_u(ub, vc, vb));
        Some(street_plan::dedupe(&pts))
    }
}

impl Tooth {
    fn marks(self) -> Vec<f64> {
        match self {
            Tooth::CulDeSac(u) => vec![u],
            Tooth::Loop(a, b) => vec![a, b],
        }
    }
}

/// Pattern (b): loops and lollipops.
pub(crate) fn lay(n: &mut Neighbourhood, p: &NeighbourhoodParams) -> Laid {
    let f = n.frame.clone();
    let Some(fit) = f.fit.clone() else { return Laid::default() };
    let last = fit.rows.len() - 1;
    let mut t = Track {
        f: &f,
        p,
        lot_depth: fit.lot_depth,
        clear: f.depth / fit.rows.len() as f64 / 2.0,
        rows: [fit.rows[0], fit.rows[last]].map(|v| Row { v, ramps: Vec::new() }),
    };

    // Entrances: a squared corner on the first or last row, which bends to
    // meet a moved one square on; a stub from the leg on any other.
    let mut squared: [[Option<usize>; 2]; 2] = [[None; 2]; 2];
    let mut stubs: Vec<(usize, f64, usize)> = Vec::new();
    for (i, e) in n.entrances.iter().enumerate() {
        let side = usize::from(e.edge == Edge::Hi);
        let (inset, u_side) = if side == 0 { (f.lo.inset, 0.0) } else { (f.hi.inset, f.width) };
        // The depth at the side line whose foot on the arterial is `at`.
        let v = (e.along * e.along + inset * inset).sqrt() - f.base - f.warp.at(u_side);
        if e.row == 0 || e.row == last {
            let top = usize::from(e.row != 0);
            let dv = v - t.rows[top].v;
            if dv.abs() > 1e-6 {
                t.rows[top].ramps.push(Ramp { at: u_side, len: RAMP_MIN.max(RAMP_RUN * dv.abs()), dv });
            }
            squared[side][top] = Some(i);
        } else {
            let v = v.clamp(t.rows[0].v + t.clear + STEP, t.rows[1].v - t.clear - STEP);
            stubs.push((side, v, i));
        }
    }
    let combs = [t.comb(0, &mut n.rng), t.comb(1, &mut n.rng)];

    let tag = format!("{}_{}", n.sector, n.index);
    let mut out = Laid::default();
    let meta = |id: &str, role, fronted, bulb| StreetMeta {
        id: id.to_string(),
        role,
        fronted,
        bulb,
        sector: n.sector,
        nbhd: n.index,
    };
    let sq = |c: usize| {
        let (side, top) = CORNERS[c % 4];
        squared[side][top]
    };
    let stub = |row: &Row, side: usize, e: usize| -> Vec<P> {
        let u = if side == 0 { 0.0 } else { f.width };
        street_plan::resample(&[f.to_world(u, row.v_at(u)), n.entrances[e].at], STEP)
    };

    // Piece k of the ring, from corner k to corner k + 1.
    let piece = |k: usize| -> Vec<P> {
        let ((sa, ta), (sb, tb)) = (CORNERS[k], CORNERS[(k + 1) % 4]);
        let (a, b) = (sq(k), sq(k + 1));
        if ta == tb {
            // A row, from side sa to side sb.
            let row = &t.rows[ta];
            let mut marks: Vec<f64> = combs[ta].iter().flat_map(|x| x.marks()).collect();
            marks.extend((0..2).filter(|&s| squared[s][ta].is_some()).map(|s| t.tee_u(s, ta)));
            let mut pts = Vec::new();
            if let Some(e) = a {
                pts.extend(stub(row, sa, e).into_iter().rev());
            }
            pts.extend(row_points(&f, row, t.row_end(sa, ta, a.is_some()), t.row_end(sb, ta, b.is_some()), &marks));
            if let Some(e) = b {
                pts.extend(stub(row, sb, e));
            }
            pts
        } else {
            // A leg, from row ta to row tb.
            let marks: Vec<f64> = stubs.iter().filter(|s| s.0 == sa).map(|s| s.1).collect();
            let mut pts = t.leg(sa, t.leg_end(sa, ta, a.is_some()), t.leg_end(sa, tb, b.is_some()), &marks);
            if a.is_some() {
                pts[0] = t.tee(sa, ta);
            }
            if b.is_some() {
                *pts.last_mut().expect("a leg") = t.tee(sb, tb);
            }
            pts
        }
    };
    // The rounded corner c, in ring order: row to leg at odd corners.
    let corner = |c: usize| {
        let (side, top) = CORNERS[c % 4];
        let mut pts = t.corner(side, top);
        if c.is_multiple_of(2) {
            pts.reverse();
        }
        pts
    };

    let mut tracks = Vec::new();
    match (0..4).find(|&c| sq(c).is_some()) {
        None => {
            let mut ring = Vec::new();
            for k in 0..4 {
                ring.extend(piece(k));
                ring.extend(corner(k + 1));
            }
            ring.push(ring[0]);
            let mut ring = street_plan::dedupe(&ring);
            // Closed, so start and end at a junction: the inner comb's first stem.
            let first = combs[0].first().map(|x| x.marks()[0]);
            if let Some(i) = first.and_then(|u| ring.iter().position(|&q| q == f.to_world(u, t.rows[0].v_at(u)))) {
                ring.pop();
                ring.rotate_left(i);
                ring.push(ring[0]);
            }
            tracks.push(ring);
        }
        Some(c0) => {
            let mut chain = Vec::new();
            for i in 0..4 {
                let k = (c0 + i) % 4;
                chain.extend(piece(k));
                if sq(k + 1).is_some() {
                    tracks.push(street_plan::dedupe(&chain));
                    chain.clear();
                } else {
                    chain.extend(corner(k + 1));
                }
            }
        }
    }
    // A stretch that starts or ends on itself, in a T at its one squared
    // corner, would read as a lollipop. It splits at the stem junction nearest its middle.
    let junctions: Vec<P> = (0..2)
        .flat_map(|top| combs[top].iter().flat_map(|x| x.marks()).map(move |u| (top, u)))
        .map(|(top, u)| f.to_world(u, t.rows[top].v_at(u)))
        .collect();
    let tracks: Vec<Vec<P>> = tracks
        .into_iter()
        .flat_map(|pts| {
            let (start, end) = (pts[0], *pts.last().expect("a track"));
            let inner = &pts[1..pts.len() - 1];
            let ends_on_itself = start != end && (inner.contains(&start) || inner.contains(&end));
            let split = ends_on_itself
                .then(|| {
                    (1..pts.len() - 1)
                        .filter(|&i| junctions.contains(&pts[i]))
                        .min_by_key(|&i| i.abs_diff(pts.len() / 2))
                })
                .flatten();
            match split {
                Some(i) => vec![pts[..=i].to_vec(), pts[i..].to_vec()],
                None => vec![pts],
            }
        })
        .collect();
    for (k, pts) in tracks.into_iter().enumerate() {
        let id = format!("collector_{tag}_{k}");
        out.meta.push(meta(&id, Role::Row, (true, true), None));
        out.roads.push(road(id, pts, p.collector_width));
    }

    // Stubs from a leg out to the arterial.
    for (k, &(side, v, e)) in stubs.iter().enumerate() {
        let u_side = if side == 0 { 0.0 } else { f.width };
        let flat = Row { v, ramps: Vec::new() };
        let mut pts = row_points(&f, &flat, t.leg_u(side, v), u_side, &[]);
        let gate = *pts.last().expect("a stub");
        pts.extend(street_plan::resample(&[gate, n.entrances[e].at], STEP).into_iter().skip(1));
        let id = format!("entrance_{tag}_{k}");
        out.meta.push(meta(&id, Role::Link, (false, false), None));
        out.roads.push(road(id, pts, p.local_width));
    }

    // The combs.
    let longest = p.cul_de_sac_max - STEP;
    for (top, comb) in combs.iter().enumerate() {
        let c = ["i", "o"][top];
        for (j, &tooth) in comb.iter().enumerate() {
            match tooth {
                Tooth::CulDeSac(u) => {
                    let (pts, centre) = t.cul_de_sac(top, u, longest);
                    let id = format!("culdesac_{tag}_{c}{j}");
                    out.meta.push(meta(&id, Role::CulDeSac, (true, true), Some(centre)));
                    out.roads.push(road(id, pts, p.local_width));
                }
                Tooth::Loop(a, b) => {
                    let pts = t.loop_street(top, a, b).expect("checked when the comb was laid");
                    let id = format!("loop_{tag}_{c}{j}");
                    out.meta.push(meta(&id, Role::Row, (true, true), None));
                    out.roads.push(road(id, pts, p.local_width));
                }
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::suburb::metrics;
    use crate::models::terrain::{NeighbourhoodPattern, TerrainConfig, WorldGenParams};

    pub(crate) fn loops_world(seed: u64) -> WorldGenParams {
        let mut params = WorldGenParams { seed, ..Default::default() };
        params.suburban.neighbourhood.force_pattern = Some(NeighbourhoodPattern::LoopsAndLollipops);
        // The split halves, which (b) doesn't fit, get (a) as they did
        // before (c) could take them (#110).
        params.suburban.neighbourhood.pattern_weights.central_loop = 0.0;
        params
    }

    #[test]
    fn whole_sectors_fit_and_a_shallow_frame_does_not() {
        let params = loops_world(42).suburban_params();
        let p = &params.neighbourhood;
        let all = crate::models::suburb::neighbourhood::neighbourhoods(&params, (capitol_protocol::rules::CENTER_X, capitol_protocol::rules::CENTER_Y));
        for n in all.iter().filter(|n| n.index == 0 && all.iter().filter(|m| m.sector == n.sector).count() == 1) {
            assert!(fits(&n.frame, p), "whole sector {} doesn't fit (b)", n.sector);
        }
        let mut shallow = all[0].frame.clone();
        shallow.depth = 3.0 * (2.0 * p.lot_depth + p.local_width);
        assert!(!fits(&shallow, p));
    }

    /// Acceptance for #109: the WP4 street invariants and the WP5 lot
    /// checks, with the pattern forced to Loops, at the four report seeds.
    #[test]
    fn loops_and_lollipops_meet_their_targets() {
        for seed in [42, 7, 1234, 99999] {
            let params = loops_world(seed);
            let t = TerrainConfig::from_params(&params);
            let sp = params.suburban_params();
            let plan = crate::models::suburb::plan(&sp, (t.center_x, t.center_y));
            let loops = plan.roads.iter().filter(|r| r.id.starts_with("loop_")).count();
            let stems = plan.roads.iter().filter(|r| r.id.starts_with("culdesac_") && r.id.contains("_i")).count();
            assert!(loops >= 1 && stems >= 5, "seed {seed}: {loops} loops, {stems} inner-comb stems");

            let m = metrics::measure_world(&params, None).measured.metrics;
            let why = format!("seed {seed}: {m:#?}");
            // Streets (WP4).
            assert_eq!(m.audit_issues, 0, "{why}");
            assert_eq!(m.four_ways, 0, "{why}");
            assert!(m.min_junction_angle.unwrap() >= 70.0, "{why}");
            assert!(m.t_spacing_min.unwrap() >= 40.0, "{why}");
            assert!(m.culdesac_max_len.unwrap() <= 300.0, "{why}");
            assert_eq!(m.connected_share, Some(1.0), "{why}");
            // Lots (WP5).
            assert!(m.culdesac_max_lots.unwrap() <= 20.0, "{why}");
            assert_eq!(m.overlaps, 0, "{why}");
            assert_eq!(m.convex_share, Some(1.0), "{why}");
            assert_eq!(m.arterial_fronting, 0, "{why}");
            // About 100 × 105 since #422 (plots plan §3): most take 4 stations.
            assert!((9000.0..=12000.0).contains(&m.lot_area_median.unwrap()), "{why}");
            assert!(m.lot_area_p5.unwrap() >= 6000.0 && m.lot_area_p95.unwrap() <= 15000.0, "{why}");
            assert!((90.0..=110.0).contains(&m.lot_width_median.unwrap()), "{why}");
            assert!(m.takes_four_share.unwrap() >= 0.70, "{why}");
            assert!((100.0..=125.0).contains(&m.lot_depth_median.unwrap()), "{why}");
            assert!(m.trimmed_share.unwrap() <= 0.10, "{why}");
            assert!(m.dropped_share.unwrap() <= 0.03, "{why}");
            // The band between the two combs and the corners between bulbs
            // are a green spine and pocket parks (#111): what's too thin to
            // use is the gap.
            assert!(m.cover_gap.unwrap() <= 0.05, "{why}");
        }
    }

    #[test]
    fn every_entrance_is_reached_and_no_track_ends_on_itself() {
        for seed in 0..30 {
            let params = loops_world(seed).suburban_params();
            let plan = crate::models::suburb::plan(&params, (capitol_protocol::rules::CENTER_X, capitol_protocol::rules::CENTER_Y));
            assert_eq!(plan.roads.len(), plan.meta.len());
            for n in &plan.neighbourhoods {
                for e in &n.entrances {
                    let ends = plan.roads.iter().flat_map(|r| [r.points[0], *r.points.last().unwrap()]);
                    assert!(ends.into_iter().any(|q| q == e.at), "seed {seed} s{}·{}: entrance not reached", n.sector, n.index);
                }
            }
            for r in plan.roads.iter().filter(|r| r.id.starts_with("collector_")) {
                let (a, b) = (r.points[0], *r.points.last().unwrap());
                let inner = &r.points[1..r.points.len() - 1];
                assert!(a == b || !(inner.contains(&a) || inner.contains(&b)), "seed {seed}: {} ends on itself", r.id);
            }
        }
    }
}
