//! The ring's roads, a sector at a time (plan RD3 to RD5).
//!
//! - **Concession** sectors get a straight main road from arterial to
//!   arterial, two legs kinked at the bisector, sometimes with a
//!   correction-line jog there, and one or two straight radial side roads.
//! - **Winding** sectors get a main lane on two low sines through a
//!   centripetal spline, and one spur that wanders off it.
//!
//! Every side road and spur ends at a farm gate: the one place a loose end is
//! allowed. `TerrainConfig::generate_all_roads_with_params` welds the lanes
//! to the arterials afterwards, so each main road ends on one at a shared
//! vertex, and each side road and spur on its main road.

use std::f64::consts::{PI, TAU};

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{RoadSegment, RoadType};

use super::sectors::{polar, Character, Sector};
use crate::models::geom::SplitMix64;
use crate::models::street_plan::{centripetal_spline, resample};
use crate::models::terrain::point_to_segment_distance;

type P = (f64, f64);

/// Lanes are sampled about this often.
pub const STEP: f64 = 20.0;
/// The main road runs this far into the ring from its inner edge, at least...
const MAIN_IN: f64 = 330.0;
/// ...and at most (RD3: ρ ∈ [3330, 3470] on today's ring).
const MAIN_OUT: f64 = 470.0;
/// How far a concession road's ends may sit off its kink's radius.
const END_DRIFT: f64 = 25.0;
/// The chance of a correction-line jog at the kink.
const JOG_CHANCE: f64 = 0.5;
/// The jog crosses at this many degrees off the road's mean line. Its two
/// bends are this plus or minus the legs' half-kink (11.25° in a 45° sector),
/// so 40° or less (RD3).
const JOG_DEG: (f64, f64) = (16.0, 24.0);
/// The jog's length along its own line.
const JOG_LEN: (f64, f64) = (110.0, 170.0);
/// A farm gate stands this far in from the ring's edge it faces.
const GATE_IN: (f64, f64) = (80.0, 140.0);
/// The winding lane's two sines, in total amplitude (RD4: 150 or less)...
const SWAY: (f64, f64) = (90.0, 150.0);
/// ...and its control points across the sector.
const CONTROLS: usize = 12;
/// The spur turns at most this much a step (RD4)...
const SPUR_TURN_DEG: f64 = 8.0;
/// ...leaves the lane within this of square...
const SPUR_LEAVE_DEG: f64 = 25.0;
/// ...and runs this long, unless something stops it first...
const SPUR_LEN: (f64, f64) = (260.0, 380.0);
/// ...though never shorter than this.
const SPUR_MIN: f64 = 160.0;
/// A spur keeps this far from its own lane once it's left it...
const SPUR_CLEAR: f64 = 40.0;
/// ...and this far past an arterial's kerb.
const ARTERIAL_CLEAR: f64 = 60.0;

/// What one sector lays.
#[derive(Debug, Default)]
pub struct Laid {
    pub roads: Vec<RoadSegment>,
    /// Where the side roads and spurs end, at a farm's gate.
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
}

/// The roads of `sector`, from its own stream.
pub fn lay(sector: &Sector, ring: &Ring, rng: &mut SplitMix64) -> Laid {
    let mut out = Laid::default();
    let road = |k: usize, points: Vec<P>| RoadSegment {
        id: format!("lane_{}_{k}", sector.index),
        road_type: RoadType::Trail,
        points,
        width: ring.lane_width,
    };
    match sector.character {
        Character::Concession => {
            let main = concession_main(sector, ring, rng);
            let sides = side_roads(sector, ring, rng, &main);
            out.roads.push(road(0, main));
            for (k, side) in sides.into_iter().enumerate() {
                out.gates.push(*side.last().expect("a side road has two ends"));
                out.roads.push(road(k + 1, side));
            }
        }
        Character::Winding => {
            let main = winding_main(sector, ring, rng);
            let spur = spur(sector, ring, rng, &main);
            out.roads.push(road(0, main));
            if let Some(spur) = spur {
                out.gates.push(*spur.last().expect("a spur has two ends"));
                out.roads.push(road(1, spur));
            }
        }
    }
    out
}

/// Two straight legs from arterial to arterial, kinked at the bisector, or
/// jogged there (RD3).
fn concession_main(sector: &Sector, ring: &Ring, rng: &mut SplitMix64) -> Vec<P> {
    let c = ring.center;
    let (lo, hi) = (ring.inner + MAIN_IN, ring.inner + MAIN_OUT);
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

/// One or two straight radial roads off the main road at T junctions, each
/// to a farm gate. Two go opposite ways, so neither half of the sector is
/// left with nothing.
fn side_roads(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, main: &[P]) -> Vec<Vec<P>> {
    let spots = if rng.next_f64() < 0.5 {
        // Clear of the kink and its jog.
        let u = rng.range(0.30, 0.58);
        vec![if u < 0.44 { u } else { u + 0.12 }]
    } else {
        vec![rng.range(0.22, 0.40), rng.range(0.60, 0.78)]
    };
    let first_out = rng.next_f64() < 0.5;
    spots
        .into_iter()
        .enumerate()
        .filter_map(|(k, t)| {
            let outward = first_out == (k == 0);
            let a = sector.bearing(t);
            let start = ray_hit(main, ring.center, a)?;
            let r = if outward { ring.outer - rng.range(GATE_IN.0, GATE_IN.1) } else { ring.inner + rng.range(GATE_IN.0, GATE_IN.1) };
            Some(vec![start, polar(ring.center, r, a)])
        })
        .collect()
}

/// The main lane: ρ(θ) on two low sines, giving two or three bends across
/// the sector, with a gentle wander in bearing, through a centripetal spline
/// and resampled (RD4).
fn winding_main(sector: &Sector, ring: &Ring, rng: &mut SplitMix64) -> Vec<P> {
    let rho0 = rng.range(ring.inner + MAIN_IN, ring.inner + MAIN_OUT);
    let sway = rng.range(SWAY.0, SWAY.1);
    let share = rng.range(0.55, 0.8);
    let (a1, a2) = (sway * share, sway * (1.0 - share));
    let (f1, f2) = (rng.range(0.9, 1.4), rng.range(1.6, 2.4));
    let (p1, p2) = (rng.range(0.0, TAU), rng.range(0.0, TAU));
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
    resample(&centripetal_spline(&controls, 8), STEP)
}

/// One spur off the lane, turning a little each step, to a farm gate (RD4).
/// It stops early rather than run near the ring's edges, an arterial or its
/// own lane, and a draw that stops too soon is drawn again. `None` if none of
/// the draws gets far enough.
fn spur(sector: &Sector, ring: &Ring, rng: &mut SplitMix64, lane: &[P]) -> Option<Vec<P>> {
    let c = ring.center;
    let max_turn = SPUR_TURN_DEG.to_radians();
    let kerb = ring.arterial_width / 2.0 + ARTERIAL_CLEAR;
    let near_lane = |p: P| lane.windows(2).any(|w| point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1) < SPUR_CLEAR);
    for _ in 0..8 {
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
            let r = ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt();
            let a = (p.1 - c.1).stable_atan2(p.0 - c.0);
            let from_edge = wrap(a - sector.from).abs().min(wrap(sector.to - a).abs());
            if r < ring.inner + GATE_IN.0
                || r > ring.outer - GATE_IN.0
                || r * from_edge.stable_sin() < kerb
                || (length > 3.0 * STEP && near_lane(p))
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

/// An angle brought into (-π, π].
fn wrap(a: f64) -> f64 {
    let a = a.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

/// Where the ray from `center` at bearing `a` first meets `road`.
fn ray_hit(road: &[P], center: P, a: f64) -> Option<P> {
    let d = (a.stable_cos(), a.stable_sin());
    road.windows(2).find_map(|w| {
        let (p, q) = (w[0], w[1]);
        let e = (q.0 - p.0, q.1 - p.1);
        let denom = d.0 * e.1 - d.1 * e.0;
        if denom.abs() < 1e-12 {
            return None;
        }
        let o = (p.0 - center.0, p.1 - center.1);
        let s = (o.0 * e.1 - o.1 * e.0) / denom;
        let u = (o.0 * d.1 - o.1 * d.0) / denom;
        (s > 0.0 && (0.0..=1.0).contains(&u)).then_some((p.0 + e.0 * u, p.1 + e.1 * u))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::street_plan::{sharpest_turn, turns};

    const C: P = (6000.0, 6000.0);

    fn ring() -> Ring {
        Ring { center: C, inner: 3000.0, outer: 3800.0, arterial_width: 60.0, lane_width: 8.0 }
    }

    fn sector(character: Character) -> Sector {
        Sector { index: 2, from: 90f64.to_radians(), to: 135f64.to_radians(), character }
    }

    fn radius(p: P) -> f64 {
        ((p.0 - C.0).powi(2) + (p.1 - C.1).powi(2)).sqrt()
    }

    #[test]
    fn a_concession_road_is_two_straight_legs_and_its_side_roads_are_radial() {
        for seed in 0..100 {
            let mut rng = SplitMix64::stream(seed, &[1]);
            let laid = lay(&sector(Character::Concession), &ring(), &mut rng);
            let main = &laid.roads[0].points;
            assert!(main.len() == 3 || main.len() == 4, "seed {seed}: {} points", main.len());
            assert!(main.iter().all(|&p| (3330.0..=3470.0 + 1e-9).contains(&radius(p))), "seed {seed}: off the band");
            assert!(sharpest_turn(main) <= 40.0, "seed {seed}: a {:.1}° bend", sharpest_turn(main));
            assert!((1..=2).contains(&(laid.roads.len() - 1)), "seed {seed}: {} side roads", laid.roads.len() - 1);
            assert_eq!(laid.gates.len(), laid.roads.len() - 1);
            for side in &laid.roads[1..] {
                let (a, b) = (side.points[0], side.points[1]);
                let (ba, bb) = ((a.1 - C.1).stable_atan2(a.0 - C.0), (b.1 - C.1).stable_atan2(b.0 - C.0));
                assert!((ba - bb).abs() < 1e-9, "seed {seed}: {} isn't radial", side.id);
                assert!(main.windows(2).any(|w| point_to_segment_distance(a.0, a.1, w[0].0, w[0].1, w[1].0, w[1].1) < 1e-6));
            }
        }
    }

    #[test]
    fn a_winding_lane_sways_gently_and_its_spur_turns_slowly() {
        let mut spurs = 0;
        for seed in 0..100 {
            let mut rng = SplitMix64::stream(seed, &[1]);
            let laid = lay(&sector(Character::Winding), &ring(), &mut rng);
            let lane = &laid.roads[0].points;
            let (lo, hi) = lane.iter().map(|&p| radius(p)).fold((f64::MAX, f64::MIN), |(l, h), r| (l.min(r), h.max(r)));
            assert!(lo > 3150.0 && hi < 3650.0, "seed {seed}: the lane runs {lo:.0} to {hi:.0}");
            assert!(sharpest_turn(lane) < 10.0, "seed {seed}: a {:.1}° bend in the lane", sharpest_turn(lane));
            if let Some(spur) = laid.roads.get(1) {
                spurs += 1;
                let worst = turns(&spur.points).map(|(_, d)| d).fold(0.0, f64::max);
                assert!(worst <= SPUR_TURN_DEG + 1e-9, "seed {seed}: the spur turns {worst:.1}°");
                assert!(spur.points.iter().all(|&p| (3080.0..=3720.0).contains(&radius(p))), "seed {seed}");
            }
        }
        assert!(spurs >= 95, "only {spurs} of 100 winding sectors got a spur");
    }

    #[test]
    fn a_ray_meets_the_road_it_crosses() {
        let road = [(0.0, 10.0), (10.0, 10.0), (20.0, 0.0)];
        let hit = ray_hit(&road, (5.0, 0.0), PI / 2.0).expect("it crosses");
        assert!((hit.0 - 5.0).abs() < 1e-9 && (hit.1 - 10.0).abs() < 1e-9, "{hit:?}");
        assert_eq!(ray_hit(&road, (5.0, 0.0), -PI / 2.0), None);
    }
}
