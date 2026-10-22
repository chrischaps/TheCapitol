//! The ring's roads, a sector at a time (plan RD3 to RD5, and RD10).
//!
//! The ring is 1600 deep, so every sector has two main roads across it from
//! arterial to arterial: an inner one about r3400 and an outer one about
//! r4200 (RD10, #310).
//! - **Concession** sectors' main roads are two straight legs each, kinked at
//!   the bisector, sometimes with a correction-line jog there. One or two
//!   straight radial *concession side roads* join them at T junctions, and
//!   at most one more runs from the outer road out to a farm gate.
//! - **Winding** sectors' main roads are lanes on two low sines through a
//!   centripetal spline, kept apart. One winding *cross-lane* joins them, and
//!   one spur wanders off either of them to a farm gate.
//!
//! Plots v2's ring (V1.3, #457) is 2500 deep and takes four main roads
//! (`RuralParams::mains`, bands as shares of the depth), with the roads
//! between each pair of them, and a winding sector's lanes there follow one
//! another on one set of sines (`RuralParams::lanes_follow`).
//!
//! A road that ends anywhere but on another road ends at a farm gate: the one
//! place a loose end is allowed. `TerrainConfig::generate_all_roads_with_params`
//! welds the lanes afterwards, so each main road ends on an arterial at a
//! shared vertex, and every other road on the main roads it leaves and meets.

use std::f64::consts::{PI, TAU};

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{RoadSegment, RoadType};

use super::sectors::{polar, Character, Sector};
use crate::models::geom::SplitMix64;
use crate::models::street_plan::{centripetal_spline, resample, MIN_STEP};
use crate::models::terrain::point_to_segment_distance;

type P = (f64, f64);

/// Lanes are sampled about this often.
pub const STEP: f64 = 20.0;
/// How far a concession road's ends may sit off its kink's radius.
const END_DRIFT: f64 = 25.0;
/// The chance of a correction-line jog at the kink, for each main road.
const JOG_CHANCE: f64 = 0.5;
/// The jog crosses at this many degrees off the road's mean line. Its two
/// bends are this plus or minus the legs' half-kink (11.25° in a 45° sector),
/// so 40° or less (RD3).
const JOG_DEG: (f64, f64) = (16.0, 24.0);
/// The jog's length along its own line.
const JOG_LEN: (f64, f64) = (110.0, 170.0);
/// The chance of a second concession side road...
const TWO_SIDE_ROADS: f64 = 0.5;
/// ...and of a road from the outer main road out to a gate.
const GATE_ROAD: f64 = 0.5;
/// A farm gate stands this far in from the ring's edge it faces.
const GATE_IN: (f64, f64) = (80.0, 140.0);
/// The winding lane's two sines, in total amplitude (RD4: 150 or less)...
const SWAY: (f64, f64) = (90.0, 150.0);
/// ...and its control points across the sector.
const CONTROLS: usize = 12;
/// The two lanes of a winding sector keep at least this far apart (RD10).
const LANES_APART: f64 = 500.0;
/// The cross-lane and the spur turn at most this much a step (RD4). They
/// draw their turns under it, so the last step, cut short where it meets a
/// road, keeps inside it too.
const WIND_TURN_DEG: f64 = 8.0;
const WIND_DRAW_DEG: f64 = WIND_TURN_DEG - 1.0;
/// The cross-lane leaves the inner lane this far across the sector...
const CROSS_AT: (f64, f64) = (0.3, 0.7);
/// ...and steers for the outer lane within this share of the sector of its
/// own bearing.
const CROSS_AIM: f64 = 0.08;
/// The spur leaves its lane within this of square...
const SPUR_LEAVE_DEG: f64 = 25.0;
/// ...and runs this long, unless something stops it first...
const SPUR_LEN: (f64, f64) = (260.0, 380.0);
/// ...though never shorter than this.
const SPUR_MIN: f64 = 160.0;
/// A spur keeps this far from the sector's other roads once it's left its
/// lane...
const SPUR_CLEAR: f64 = 40.0;
/// ...and this far past an arterial's kerb.
const ARTERIAL_CLEAR: f64 = 60.0;

/// What one sector lays.
#[derive(Debug, Default)]
pub struct Laid {
    /// `lane_{s}_0` is the inner main road, and the next main roads go on
    /// out: `lane_{s}_1` is the outer one of two, the middle one of three.
    /// Then come the roads that join them, and last the road to a gate, if
    /// there is one.
    pub roads: Vec<RoadSegment>,
    /// Where the roads to a gate end.
    pub gates: Vec<P>,
}

/// The ring the roads stay in, and the roads' widths.
#[derive(Debug, Clone, Copy)]
pub struct Ring {
    pub center: P,
    pub inner: f64,
    pub outer: f64,
    pub arterial_width: f64,
    pub lane_width: f64,
    /// The winding lanes' sway, as a share of [`SWAY`]'s...
    pub sway: f64,
    /// ...and whether a sector's lanes all take the first one's sines
    /// (`RuralParams::lanes_follow`).
    pub lanes_follow: bool,
}

impl Ring {
    /// The radii a main road keeps to, from a band given as shares of the
    /// ring's depth (`RuralParams::mains`). Today's shares give today's radii
    /// exactly: 1600 × (330 / 1600) is 330 in floating point, and so on.
    fn band(&self, band: (f64, f64)) -> (f64, f64) {
        let depth = self.outer - self.inner;
        (self.inner + band.0 * depth, self.inner + band.1 * depth)
    }
}

/// The roads of `sector`, from its own stream, with a main road in each of
/// `mains`' bands (`RuralParams::mains`: two or more, inner first).
///
/// The draws come in the order two main roads always drew them, so today's
/// ring is laid as it was: every main road, then the roads between each
/// pair, inner pair first, and the road to a gate off the outermost.
pub fn lay(sector: &Sector, ring: &Ring, mains: &[(f64, f64)], rng: &mut SplitMix64) -> Laid {
    assert!(mains.len() >= 2, "a sector has two main roads or more");
    let (mains, links, to_gate) = match sector.character {
        Character::Concession => {
            let roads: Vec<Vec<P>> = mains.iter().map(|&band| concession_main(sector, ring, rng, band)).collect();
            let mut links = Vec::new();
            let mut to_gate = None;
            let last = roads.len() - 2;
            for k in 0..=last {
                let (between, gate) = side_roads(sector, ring, rng, &roads[k], &roads[k + 1], k == last);
                links.extend(between);
                to_gate = to_gate.or(gate);
            }
            (roads, links, to_gate)
        }
        Character::Winding => {
            let (first, sines) = winding_lane(sector, ring, rng, mains[0], 1.0, None);
            let mut lanes = vec![first];
            for &band in &mains[1..] {
                let inner = lanes.last().expect("one at least");
                let follows = ring.lanes_follow.then(|| winding_lane(sector, ring, rng, band, 1.0, Some(sines)).0);
                // Two lanes on one set of sines keep their bands apart, near
                // enough: a wander that brings them closer draws again.
                let lane = match follows.filter(|lane| closest(lane, inner) >= LANES_APART) {
                    Some(lane) => lane,
                    None => lane_apart(sector, ring, rng, band, inner),
                };
                lanes.push(lane);
            }
            let mut crosses = Vec::new();
            for k in 0..lanes.len() - 1 {
                let (inner, outer) = lanes.split_at_mut(k + 1);
                crosses.push(cross_lane(sector, ring, rng, &inner[k], &mut outer[0]));
            }
            let spur = spur(sector, ring, rng, &lanes, &crosses);
            (lanes, crosses, spur)
        }
    };
    let mut out = Laid::default();
    if let Some(end) = to_gate.as_ref().and_then(|r| r.last()) {
        out.gates.push(*end);
    }
    out.roads = mains
        .into_iter()
        .chain(links)
        .chain(to_gate)
        .enumerate()
        .map(|(k, points)| RoadSegment {
            id: format!("lane_{}_{k}", sector.index),
            road_type: RoadType::Trail,
            points,
            width: ring.lane_width,
        })
        .collect();
    out
}

/// Two straight legs from arterial to arterial, kinked at the bisector, or
/// jogged there (RD3), within `band` of the ring's inner edge.
fn concession_main(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, band: (f64, f64)) -> Vec<P> {
    let c = ring.center;
    let (lo, hi) = ring.band(band);
    let rho = rng.range(lo + END_DRIFT, hi - END_DRIFT);
    let r_from = (rho + rng.range(-END_DRIFT, END_DRIFT)).clamp(lo, hi);
    let r_to = (rho + rng.range(-END_DRIFT, END_DRIFT)).clamp(lo, hi);
    let mid = sector.bearing(0.5);

    let mut main = vec![polar(c, r_from, sector.from)];
    if rng.next_f64() < JOG_CHANCE {
        // Across at `beta` off the tangent, outward or in, centred on the kink.
        let sign = if rng.next_f64() < 0.5 { 1.0 } else { -1.0 };
        let beta = sign * rng.range(JOG_DEG.0, JOG_DEG.1).to_radians();
        let half = rng.range(JOG_LEN.0, JOG_LEN.1) / 2.0;
        // The jog reaches up to half its length across, so its kink sits far
        // enough inside the band for both its bends to stay in.
        let room = half * JOG_DEG.1.to_radians().stable_sin() + 2.0;
        let k = polar(c, rho.clamp(lo + room, hi - room), mid);
        let n = (mid.stable_cos(), mid.stable_sin());
        let t = (-n.1, n.0);
        let (along, across) = (half * beta.stable_cos(), half * beta.stable_sin());
        main.push((k.0 - t.0 * along - n.0 * across, k.1 - t.1 * along - n.1 * across));
        main.push((k.0 + t.0 * along + n.0 * across, k.1 + t.1 * along + n.1 * across));
    } else {
        main.push(polar(c, rho, mid));
    }
    main.push(polar(c, r_to, sector.to));
    main
}

/// The concession side roads: one or two straight radial roads from the
/// inner main road to the outer one, at T junctions both ends. Then, half
/// the time and only if `gate` (off the outermost main road), one more from
/// the outer road out to a farm gate (RD10).
///
/// The sector's two halves sit either side of the kink and its jogs. Two
/// side roads take a half each; one takes a half and leaves the other to the
/// gate road. When both halves have a side road, the gate road shares one,
/// at the other end of it from the side road.
fn side_roads(
    sector: &Sector,
    ring: &Ring,
    rng: &mut SplitMix64,
    inner: &[P],
    outer: &[P],
    gate: bool,
) -> (Vec<Vec<P>>, Option<Vec<P>>) {
    // `u` runs across a half, 0 by the arterial and 1 by the kink.
    let at = |first: bool, u: f64| if first { 0.22 + 0.20 * u } else { 0.78 - 0.20 * u };
    let two = rng.next_f64() < TWO_SIDE_ROADS;
    let gate = rng.next_f64() < GATE_ROAD && gate;
    let first = rng.next_f64() < 0.5;
    let side_by_arterial = rng.next_f64() < 0.5;
    let mut u = |mid: f64, spread: f64| mid + rng.range(-spread, spread);
    let (sides, to_gate) = match (two, gate) {
        (true, false) => (vec![at(first, u(0.5, 0.1)), at(!first, u(0.5, 0.1))], None),
        (true, true) => {
            let (side, gate) = if side_by_arterial { (0.1, 0.9) } else { (0.9, 0.1) };
            (vec![at(first, u(side, 0.1)), at(!first, u(0.5, 0.1))], Some(at(first, u(gate, 0.1))))
        }
        (false, gate) => (vec![at(first, u(0.5, 0.2))], gate.then(|| at(!first, u(0.5, 0.2)))),
    };
    let c = ring.center;
    let links = sides
        .into_iter()
        .filter_map(|t| {
            let a = sector.bearing(t);
            Some(vec![ray_hit(inner, c, a)?.1, ray_hit(outer, c, a)?.1])
        })
        .collect();
    let to_gate = to_gate.and_then(|t| {
        let a = sector.bearing(t);
        let r = ring.outer - rng.range(GATE_IN.0, GATE_IN.1);
        Some(vec![ray_hit(outer, c, a)?.1, polar(c, r, a)])
    });
    (links, to_gate)
}

/// A winding lane: ρ(θ) on two low sines, giving two or three bends across
/// the sector, with a gentle wander in bearing, through a centripetal spline
/// and resampled (RD4). Its mean radius is within `band` of the ring's inner
/// edge, and `sway` scales its sines.
fn winding_main(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, band: (f64, f64), sway: f64) -> Vec<P> {
    winding_lane(sector, ring, rng, band, sway, None).0
}

/// A winding lane's two sines: their total amplitude, the first one's share
/// of it, and their frequencies across the sector and phases.
#[derive(Debug, Clone, Copy)]
struct Sines {
    amplitude: f64,
    share: f64,
    f: (f64, f64),
    p: (f64, f64),
}

/// [`winding_main`], on `sines` if given (`RuralParams::lanes_follow`) and on
/// sines of its own if not, and the sines it ran on. Its draws are
/// `winding_main`'s, in order, less the sines it was given.
fn winding_lane(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, band: (f64, f64), sway: f64, sines: Option<Sines>) -> (Vec<P>, Sines) {
    let (lo, hi) = ring.band(band);
    let rho0 = rng.range(lo, hi);
    let sines = sines.unwrap_or_else(|| {
        let amplitude = rng.range(SWAY.0, SWAY.1) * sway * ring.sway;
        let share = rng.range(0.55, 0.8);
        let f = (rng.range(0.9, 1.4), rng.range(1.6, 2.4));
        let p = (rng.range(0.0, TAU), rng.range(0.0, TAU));
        Sines { amplitude, share, f, p }
    });
    let Sines { amplitude, share, f: (f1, f2), p: (p1, p2) } = sines;
    let (a1, a2) = (amplitude * share, amplitude * (1.0 - share));
    let rho = |t: f64| rho0 + a1 * (TAU * f1 * t + p1).stable_sin() + a2 * (TAU * f2 * t + p2).stable_sin();

    let controls: Vec<P> = (0..=CONTROLS)
        .map(|j| {
            let t = j as f64 / CONTROLS as f64;
            // The ends stay on the arterials; between, the bearing wanders
            // a fifth of a step at most, so it never doubles back or bunches.
            let wander = if j == 0 || j == CONTROLS { 0.0 } else { rng.range(-0.2, 0.2) / CONTROLS as f64 };
            polar(ring.center, rho(t), sector.bearing(t + wander))
        })
        .collect();
    (resample(&centripetal_spline(&controls, 8), STEP), sines)
}

/// A winding sector's next lane out, in `band`, at least [`LANES_APART`]
/// from the lane inside it. A draw that comes too close is drawn again, and
/// after every eight the sines are let out less, down to none: the bands
/// alone keep two lanes without sway 660 apart in today's ring.
fn lane_apart(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, band: (f64, f64), inner: &[P]) -> Vec<P> {
    let mut sway: f64 = 1.0;
    loop {
        for _ in 0..8 {
            let outer = winding_main(sector, ring, rng, band, sway);
            if sway <= 0.0 || closest(&outer, inner) >= LANES_APART {
                return outer;
            }
        }
        sway = (sway - 0.25).max(0.0);
    }
}

/// The winding cross-lane, from a vertex of the inner lane to the outer lane,
/// at T junctions both ends (RD10). It leaves about square and steers for a
/// point on the outer lane, wandering as it goes and turning less than
/// [`WIND_TURN_DEG`] a step, and stops where it meets the outer lane.
///
/// Where it meets goes into the outer lane as a vertex. Left to the weld,
/// which snaps a junction to any vertex within `street_plan::SNAP`, a short
/// last step would swing through a sharp turn.
fn cross_lane(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, inner: &[P], outer: &mut Vec<P>) -> Vec<P> {
    let (mut points, (seg, x)) = walk_across(sector, ring, rng, inner, outer);
    match [seg, seg + 1].into_iter().find(|&k| dist(outer[k], x) < MIN_STEP) {
        Some(k) => outer[k] = x,
        None => outer.insert(seg + 1, x),
    }
    if dist(points[points.len() - 1], x) < 2.0 * MIN_STEP && points.len() > 1 {
        // A step that meets the lane a hair past the last vertex ends there
        // instead, rather than leave a sliver.
        points.pop();
    }
    points.push(x);
    points
}

/// The cross-lane's walk, up to the step that meets the outer lane, and
/// where it meets it: the lane's segment and the point.
fn walk_across(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, inner: &[P], outer: &[P]) -> (Vec<P>, (usize, P)) {
    let c = ring.center;
    let max_turn = WIND_DRAW_DEG.to_radians();
    let bearing = |p: P| (p.1 - c.1).stable_atan2(p.0 - c.0);
    for _ in 0..8 {
        let i = ((inner.len() - 1) as f64 * rng.range(CROSS_AT.0, CROSS_AT.1)).round() as usize;
        let start = inner[i];
        let aim = bearing(start) + rng.range(-CROSS_AIM, CROSS_AIM) * sector.span();
        let Some((_, target)) = ray_hit(outer, c, aim) else { continue };
        let mut heading = bearing(start) + rng.range(-15.0, 15.0).to_radians();
        let mut points = vec![start];
        for _ in 0..200 {
            let last = *points.last().expect("starts with one");
            if points.len() > 1 {
                // A random walk, steered for the target, harder as it nears.
                let off = wrap((target.1 - last.1).stable_atan2(target.0 - last.0) - heading);
                let pull = (3.0 * STEP / dist(last, target)).clamp(0.15, 1.0);
                let turn = rng.range(-6.0, 6.0).to_radians() * (1.0 - pull) + off * pull;
                heading += turn.clamp(-max_turn, max_turn);
            }
            let p = (last.0 + STEP * heading.stable_cos(), last.1 + STEP * heading.stable_sin());
            if let Some(hit) = first_hit(outer, last, p) {
                return (points, hit);
            }
            points.push(p);
        }
    }
    // Never seen: straight across at the bisector.
    let a = sector.bearing(0.5);
    let (_, from) = ray_hit(inner, c, a).expect("the inner lane spans the sector");
    (vec![from], ray_hit(outer, c, a).expect("so does the outer"))
}

/// One spur off any of the lanes, turning a little each step, to a farm gate
/// (RD4). It stops early rather than run near the ring's edges, an arterial
/// or the sector's other roads, and a draw that stops too soon is drawn
/// again. `None` if none of the draws gets far enough.
fn spur(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, lanes: &[Vec<P>], crosses: &[Vec<P>]) -> Option<Vec<P>> {
    let c = ring.center;
    let max_turn = WIND_DRAW_DEG.to_radians();
    let kerb = ring.arterial_width / 2.0 + ARTERIAL_CLEAR;
    let near = |p: P, road: &[P]| {
        road.windows(2).any(|w| point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1) < SPUR_CLEAR)
    };
    for _ in 0..8 {
        let from = rng.below(lanes.len() as u64) as usize;
        let lane = &lanes[from];
        let i = ((lane.len() - 1) as f64 * rng.range(0.3, 0.7)).round() as usize;
        let start = lane[i];
        let outward = rng.next_f64() < 0.5;
        // Square off the lane is radial, near enough: the lane runs round.
        let away = |p: P| (p.1 - c.1).stable_atan2(p.0 - c.0) + if outward { 0.0 } else { PI };
        let mut heading = away(start) + rng.range(-SPUR_LEAVE_DEG, SPUR_LEAVE_DEG).to_radians();
        let target = rng.range(SPUR_LEN.0, SPUR_LEN.1);
        let mut points = vec![start];
        let mut length = 0.0;
        while length < target {
            let last = *points.last().expect("starts with one");
            if points.len() > 1 {
                // A random walk, pulled gently back toward square.
                let off = wrap(heading - away(last));
                let turn = rng.range(-6.0, 6.0).to_radians() - 0.12 * off;
                heading += turn.clamp(-max_turn, max_turn);
            }
            let p = (last.0 + STEP * heading.stable_cos(), last.1 + STEP * heading.stable_sin());
            let r = dist(p, c);
            let a = (p.1 - c.1).stable_atan2(p.0 - c.0);
            let from_edge = wrap(a - sector.from).abs().min(wrap(sector.to - a).abs());
            // Its own lane, and a cross-lane that leaves the same place,
            // are near its start by right.
            let left = length > 3.0 * STEP;
            if r < ring.inner + GATE_IN.0
                || r > ring.outer - GATE_IN.0
                || r * from_edge.stable_sin() < kerb
                || lanes.iter().enumerate().any(|(k, other)| k != from && near(p, other))
                || (left && near(p, lane))
                || crosses.iter().any(|cross| (left || !near(start, cross)) && near(p, cross))
            {
                break;
            }
            points.push(p);
            length += STEP;
        }
        if length >= SPUR_MIN {
            return Some(points);
        }
    }
    None
}

/// A spur toward `hole`, the centre of a disc `radius` across that the farms
/// left open (RD7, #287). It leaves the nearest point of the sector's
/// `roads` and runs on past the centre by half the radius: straight in a
/// concession sector, and winding in a winding one, steered for the centre
/// until it's there and then wandering on. It stops early as [`spur`] does,
/// and a winding one that stops too soon is drawn again. `None` if none gets
/// [`SPUR_MIN`] long.
pub fn spur_to(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, roads: &[&[P]], hole: P, radius: f64) -> Option<Vec<P>> {
    let c = ring.center;
    let max_turn = WIND_DRAW_DEG.to_radians();
    let kerb = ring.arterial_width / 2.0 + ARTERIAL_CLEAR;
    let near = |p: P, road: &[P]| {
        road.windows(2).any(|w| point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1) < SPUR_CLEAR)
    };
    let clear_of_edges = |p: P| {
        let r = dist(p, c);
        let a = (p.1 - c.1).stable_atan2(p.0 - c.0);
        let from_edge = wrap(a - sector.from).abs().min(wrap(sector.to - a).abs());
        r >= ring.inner + GATE_IN.0 && r <= ring.outer - GATE_IN.0 && r * from_edge.stable_sin() >= kerb
    };
    let (from, start) = roads
        .iter()
        .enumerate()
        .flat_map(|(i, road)| road.windows(2).map(move |w| (i, nearest_on(w[0], w[1], hole))))
        .min_by(|a, b| dist(a.1, hole).total_cmp(&dist(b.1, hole)))?;
    if !clear_of_edges(start) {
        return None;
    }
    let to_hole = dist(start, hole);
    let target = to_hole + radius / 2.0;
    let winding = sector.character == Character::Winding;
    for _ in 0..if winding { 8 } else { 1 } {
        let toward = |p: P| (hole.1 - p.1).stable_atan2(hole.0 - p.0);
        let mut heading = toward(start) + if winding { rng.range(-15.0, 15.0).to_radians() } else { 0.0 };
        let mut points = vec![start];
        let mut length = 0.0;
        while length < target {
            let last = *points.last().expect("starts with one");
            if winding && points.len() > 1 {
                let turn = if length < to_hole {
                    // Steered for the centre, harder as it nears...
                    let off = wrap(toward(last) - heading);
                    let pull = (3.0 * STEP / dist(last, hole).max(STEP)).clamp(0.15, 1.0);
                    rng.range(-6.0, 6.0).to_radians() * (1.0 - pull) + off * pull
                } else {
                    // ...then wandering on.
                    rng.range(-6.0, 6.0).to_radians()
                };
                heading += turn.clamp(-max_turn, max_turn);
            }
            let p = (last.0 + STEP * heading.stable_cos(), last.1 + STEP * heading.stable_sin());
            // Its own road is near its start by right.
            let left = length > 3.0 * STEP;
            let blocked = roads.iter().enumerate().any(|(i, road)| (i != from || left) && near(p, road));
            if !clear_of_edges(p) || blocked {
                break;
            }
            points.push(p);
            length += STEP;
        }
        if length >= SPUR_MIN {
            return Some(if winding { points } else { vec![start, *points.last().expect("long enough")] });
        }
    }
    None
}

/// The point of the segment `a`..`b` nearest `q`.
fn nearest_on(a: P, b: P, q: P) -> P {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((q.0 - a.0) * dx + (q.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (a.0 + t * dx, a.1 + t * dy)
}

/// An angle brought into (-π, π].
fn wrap(a: f64) -> f64 {
    let a = a.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).stable_hypot(a.1 - b.1)
}

/// The least distance from a point of `a` to the line `b`.
fn closest(a: &[P], b: &[P]) -> f64 {
    a.iter()
        .flat_map(|p| b.windows(2).map(move |w| point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1)))
        .fold(f64::INFINITY, f64::min)
}

/// Where the ray from `center` at bearing `a` first meets `road`: the
/// road's segment and the point.
fn ray_hit(road: &[P], center: P, a: f64) -> Option<(usize, P)> {
    let d = (a.stable_cos(), a.stable_sin());
    road.windows(2).enumerate().find_map(|(i, w)| {
        let (p, q) = (w[0], w[1]);
        let e = (q.0 - p.0, q.1 - p.1);
        let denom = d.0 * e.1 - d.1 * e.0;
        if denom.abs() < 1e-12 {
            return None;
        }
        let o = (p.0 - center.0, p.1 - center.1);
        let s = (o.0 * e.1 - o.1 * e.0) / denom;
        let u = (o.0 * d.1 - o.1 * d.0) / denom;
        (s > 0.0 && (0.0..=1.0).contains(&u)).then_some((i, (p.0 + e.0 * u, p.1 + e.1 * u)))
    })
}

/// Where the segment from `p` to `q` first meets `road`, nearest `p`: the
/// road's segment and the point.
fn first_hit(road: &[P], p: P, q: P) -> Option<(usize, P)> {
    let d = (q.0 - p.0, q.1 - p.1);
    road.windows(2)
        .enumerate()
        .filter_map(|(i, w)| {
            let e = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let denom = d.0 * e.1 - d.1 * e.0;
            if denom.abs() < 1e-12 {
                return None;
            }
            let o = (w[0].0 - p.0, w[0].1 - p.1);
            let s = (o.0 * e.1 - o.1 * e.0) / denom;
            let u = (o.0 * d.1 - o.1 * d.0) / denom;
            ((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&u)).then_some((i, s))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, s)| (i, (p.0 + d.0 * s, p.1 + d.1 * s)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::rural::RuralParams;
    use crate::models::street_plan::{sharpest_turn, turns};

    const C: P = (6800.0, 6800.0);

    fn ring() -> Ring {
        Ring { center: C, inner: 3000.0, outer: 4600.0, arterial_width: 60.0, lane_width: 8.0, sway: 1.0, lanes_follow: false }
    }

    fn sector(character: Character) -> Sector {
        Sector { index: 2, from: 90f64.to_radians(), to: 135f64.to_radians(), character }
    }

    fn radius(p: P) -> f64 {
        dist(p, C)
    }

    fn bearing(p: P) -> f64 {
        (p.1 - C.1).stable_atan2(p.0 - C.0)
    }

    fn on(p: P, road: &[P]) -> bool {
        road.windows(2).any(|w| point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1) < 1e-6)
    }

    #[test]
    fn a_concession_sector_has_two_straight_roads_joined_by_radial_side_roads() {
        let (mut ones, mut twos, mut gates) = (0, 0, 0);
        for seed in 0..200 {
            let mut rng = SplitMix64::stream(seed, &[1]);
            let laid = lay(&sector(Character::Concession), &ring(), &RuralParams::default().mains, &mut rng);
            let (inner, outer) = (&laid.roads[0].points, &laid.roads[1].points);
            for (main, lo, hi) in [(inner, 3330.0, 3470.0), (outer, 4130.0, 4270.0)] {
                assert!(main.len() == 3 || main.len() == 4, "seed {seed}: {} points", main.len());
                assert!(main.iter().all(|&p| (lo..=hi + 1e-9).contains(&radius(p))), "seed {seed}: off the band");
                assert!(sharpest_turn(main) <= 40.0, "seed {seed}: a {:.1}° bend", sharpest_turn(main));
            }
            let rest = &laid.roads[2..];
            let links = rest.len() - laid.gates.len();
            assert!((1..=2).contains(&links), "seed {seed}: {links} side roads");
            assert!(laid.gates.len() <= 1, "seed {seed}: at most one road to a gate");
            (ones, twos, gates) = (ones + (links == 1) as u32, twos + (links == 2) as u32, gates + laid.gates.len());
            let mut bearings = Vec::new();
            for (k, road) in rest.iter().enumerate() {
                let (a, b) = (road.points[0], road.points[1]);
                assert_eq!(road.points.len(), 2, "seed {seed}: {} is straight", road.id);
                assert!((bearing(a) - bearing(b)).abs() < 1e-9, "seed {seed}: {} isn't radial", road.id);
                bearings.push(bearing(a));
                if k < links {
                    assert!(on(a, inner) && on(b, outer), "seed {seed}: {} doesn't join the main roads", road.id);
                } else {
                    assert!(on(a, outer), "seed {seed}: {} doesn't leave the outer road", road.id);
                    assert_eq!(b, laid.gates[0]);
                    assert!((4460.0 - 1e-9..=4520.0 + 1e-9).contains(&radius(b)), "seed {seed}: the gate at r{:.0}", radius(b));
                }
            }
            // No two of them share a bearing, or come within a farm of it.
            for i in 0..bearings.len() {
                for k in i + 1..bearings.len() {
                    let apart = (bearings[i] - bearings[k]).abs() * 4200.0;
                    assert!(apart > 300.0, "seed {seed}: two roads {apart:.0} apart at r4200");
                }
            }
        }
        assert!(ones > 60 && twos > 60 && gates > 60, "{ones} with one side road, {twos} with two, {gates} gates");
    }

    #[test]
    fn a_winding_sector_has_two_lanes_apart_and_a_cross_lane_between() {
        let mut spurs = 0;
        for seed in 0..200 {
            let mut rng = SplitMix64::stream(seed, &[1]);
            let laid = lay(&sector(Character::Winding), &ring(), &RuralParams::default().mains, &mut rng);
            let (inner, outer) = (&laid.roads[0].points, &laid.roads[1].points);
            for (lane, lo, hi) in [(inner, 3150.0, 3650.0), (outer, 3950.0, 4450.0)] {
                let (near, far) = lane.iter().map(|&p| radius(p)).fold((f64::MAX, f64::MIN), |(l, h), r| (l.min(r), h.max(r)));
                assert!(near > lo && far < hi, "seed {seed}: a lane runs {near:.0} to {far:.0}");
                assert!(sharpest_turn(lane) < 10.0, "seed {seed}: a {:.1}° bend in a lane", sharpest_turn(lane));
            }
            assert!(closest(outer, inner) >= LANES_APART, "seed {seed}: the lanes come {:.0} apart", closest(outer, inner));

            let cross = &laid.roads[2].points;
            assert!(inner.contains(&cross[0]), "seed {seed}: the cross-lane doesn't leave a vertex of the inner lane");
            assert!(on(*cross.last().unwrap(), outer), "seed {seed}: the cross-lane doesn't meet the outer lane");
            let worst = turns(cross).map(|(_, d)| d).fold(0.0, f64::max);
            assert!(worst <= WIND_TURN_DEG, "seed {seed}: the cross-lane turns {worst:.1}°");
            assert!(cross.windows(2).all(|w| dist(w[0], w[1]) >= 2.0 * MIN_STEP), "seed {seed}: a sliver on the cross-lane");

            if let Some(spur) = laid.roads.get(3) {
                spurs += 1;
                assert_eq!(*spur.points.last().unwrap(), laid.gates[0]);
                let worst = turns(&spur.points).map(|(_, d)| d).fold(0.0, f64::max);
                assert!(worst <= WIND_TURN_DEG, "seed {seed}: the spur turns {worst:.1}°");
                assert!(spur.points.iter().all(|&p| (3080.0..=4520.0).contains(&radius(p))), "seed {seed}");
            }
        }
        assert!(spurs >= 190, "only {spurs} of 200 winding sectors got a spur");
    }

    /// Plots v2's ring (V1.3, #457): each sector's four main roads keep to
    /// their bands down the 2500 of depth, a winding sector's lanes follow
    /// one another at least [`LANES_APART`] apart, and a road joins every
    /// pair of neighbouring main roads.
    #[test]
    fn four_main_roads_keep_their_bands_and_each_pair_is_joined() {
        let v2 = RuralParams::v2();
        let ring = Ring { center: C, inner: 3600.0, outer: 6100.0, arterial_width: 60.0, lane_width: 8.0, sway: v2.sway, lanes_follow: v2.lanes_follow };
        let n = v2.mains.len();
        assert_eq!(n, 4);
        for character in [Character::Concession, Character::Winding] {
            for seed in 0..100 {
                let mut rng = SplitMix64::stream(seed, &[1]);
                let laid = lay(&sector(character), &ring, &v2.mains, &mut rng);
                let mains: Vec<&Vec<P>> = laid.roads[..n].iter().map(|r| &r.points).collect();
                for (k, (main, &band)) in mains.iter().zip(&v2.mains).enumerate() {
                    let (lo, hi) = ring.band(band);
                    // A winding lane sways its sines' amplitude off its band.
                    let slack = if character == Character::Winding { SWAY.1 * v2.sway + 1.0 } else { 1e-9 };
                    let (near, far) = main.iter().map(|&p| radius(p)).fold((f64::MAX, f64::MIN), |(l, h), r| (l.min(r), h.max(r)));
                    assert!(near >= lo - slack && far <= hi + slack, "seed {seed}, {character:?} main {k}: r{near:.0} to r{far:.0}");
                }
                for k in 0..n - 1 {
                    if character == Character::Winding {
                        let apart = closest(mains[k + 1], mains[k]);
                        assert!(apart >= LANES_APART, "seed {seed}: lanes {k} and {} come {apart:.0} apart", k + 1);
                    }
                    let joined = laid.roads[n..].iter().any(|r| on(r.points[0], mains[k]) && on(*r.points.last().unwrap(), mains[k + 1]));
                    assert!(joined, "seed {seed}, {character:?}: nothing joins main roads {k} and {}", k + 1);
                }
            }
        }
    }

    #[test]
    fn a_ray_meets_the_road_it_crosses() {
        let road = [(0.0, 10.0), (10.0, 10.0), (20.0, 0.0)];
        let (seg, hit) = ray_hit(&road, (5.0, 0.0), PI / 2.0).expect("it crosses");
        assert_eq!(seg, 0);
        assert!((hit.0 - 5.0).abs() < 1e-9 && (hit.1 - 10.0).abs() < 1e-9, "{hit:?}");
        assert_eq!(ray_hit(&road, (5.0, 0.0), -PI / 2.0), None);
    }

    #[test]
    fn a_step_meets_the_road_nearest_its_start() {
        let road = [(0.0, 10.0), (20.0, 10.0), (20.0, 5.0), (0.0, 5.0)];
        assert_eq!(first_hit(&road, (10.0, 0.0), (10.0, 20.0)), Some((2, (10.0, 5.0))));
        assert_eq!(first_hit(&road, (10.0, 0.0), (10.0, 4.0)), None);
    }
}
