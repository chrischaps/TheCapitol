//! The bazaar quarter (E3.12, economy plan D6): the south-west quarter of the
//! Trade District, from the Capitol avenue round to the west arterial,
//! between the moat and the inner canal, given over to merchants' stalls.
//!
//! - **The market street** is the trade ring road's quarter. Stalls stand on
//!   both sides of it, facing it.
//! - **The lane** runs straight in from the avenue, from opposite the
//!   Exchange hall's door to the ring, so the hall and the bazaar still face
//!   each other across the avenue. Its mouth on the avenue is the `bazaar`
//!   landmark.
//! - **The alley** runs in from the ring toward the moat and opens into a
//!   small **square** (`bazaar_square`) at its end.
//!
//! The lane and the alley are `grid` roads (`bazaar_lane`, `bazaar_alley`):
//! a new road type would break the older clients, which read every road.
//!
//! Stalls are laid generously along each street, each with a frontage and a
//! depth drawn from the quarter's own stream, and any that would overlap a
//! stall already placed or stand on a road is dropped. So the junctions carve
//! themselves, and the street keeps a little of the irregularity of a market
//! that grew. Every stall's door is on its street's edge, in range of the
//! crown in front of it (`rules::bazaar::STALL_RANGE`).

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{Landmark, LandmarkKind, RoadSegment, RoadType};

use super::geom::SplitMix64;
use super::terrain::WorldGenParams;

/// The quarter's stream: change it and every stall moves.
const SEED: u64 = 0xba_2aa2_e312;
/// The lane's width, and the alley's.
pub const LANE_WIDTH: f64 = 24.0;
pub const ALLEY_WIDTH: f64 = 20.0;
/// How far back from a street's edge a stall's face stands.
const GAP: f64 = 3.0;
/// A stall's frontage and depth, each drawn from these.
const FRONTAGE: (f64, f64) = (26.0, 32.0);
const DEPTH: (f64, f64) = (26.0, 32.0);
/// Where the alley leaves the ring (degrees), and the square at its end:
/// how far it reaches in toward the moat, how deep and how wide it is.
const ALLEY_BEARING: f64 = 155.0;
const SQUARE_IN: f64 = 222.0;
const SQUARE_OUT: f64 = 290.0;
const SQUARE_HALF: f64 = 35.0;
/// The mouth on the avenue: how far it reaches in from the avenue's kerb.
const MOUTH: f64 = 24.0;

/// The quarter: its landmarks (the mouth, the square, then the stalls,
/// `bazaar_stall_1` and up along the streets from the avenue) and its two
/// roads.
pub struct Quarter {
    pub landmarks: Vec<Landmark>,
    pub roads: Vec<RoadSegment>,
}

/// A street a stall may stand on, and the roads it must keep off.
struct Corridor {
    points: Vec<(f64, f64)>,
    half: f64,
}

pub fn quarter(center: (f64, f64), params: &WorldGenParams) -> Quarter {
    let (cx, cy) = center;
    let polar = |r: f64, deg: f64| {
        let t = deg.to_radians();
        (cx + r * t.stable_cos(), cy + r * t.stable_sin())
    };
    let ring_r = params.trade_ring_radius;
    let ring_half = params.trade_ring_width / 2.0;
    let avenue_half = params.arterial_width / 2.0;
    // The lane leaves the avenue level with the hall's door: midway between
    // the moat's edge and the ring's (as `exchange_hall_site` sets it).
    let moat_edge = params.capitol_radius + params.moat_width / 2.0;
    let along = (moat_edge + (ring_r - ring_half)) / 2.0;
    // West of the avenue, in the avenue's frame: `lateral` out from its axis
    let lane_at = |lateral: f64, a: f64| (cx - lateral, cy + a);
    let lane_end = (ring_r * ring_r - along * along).sqrt();

    let lane = RoadSegment {
        id: "bazaar_lane".into(),
        road_type: RoadType::Grid,
        points: vec![lane_at(0.0, along), lane_at(lane_end, along)],
        width: LANE_WIDTH,
    };
    let alley = RoadSegment {
        id: "bazaar_alley".into(),
        road_type: RoadType::Grid,
        points: vec![polar(ring_r, ALLEY_BEARING), polar(SQUARE_OUT, ALLEY_BEARING)],
        width: ALLEY_WIDTH,
    };

    // Everything a stall keeps off: the streets it faces, the avenue and the
    // west arterial, and the square
    let arc = |r: f64| (0..=90).map(|k| polar(r, 90.0 + k as f64)).collect::<Vec<_>>();
    let keep_off = [
        Corridor { points: arc(ring_r), half: ring_half },
        Corridor { points: lane.points.clone(), half: LANE_WIDTH / 2.0 },
        Corridor { points: alley.points.clone(), half: ALLEY_WIDTH / 2.0 },
        Corridor { points: vec![polar(moat_edge, 90.0), polar(params.trade_outer_radius, 90.0)], half: avenue_half },
        Corridor { points: vec![polar(ring_r, 180.0), polar(params.trade_outer_radius, 180.0)], half: avenue_half },
    ];

    let mouth = {
        let (r0, r1) = (along - LANE_WIDTH / 2.0, along + LANE_WIDTH / 2.0);
        let (l0, l1) = (avenue_half, avenue_half + MOUTH);
        Landmark {
            id: "bazaar".into(),
            kind: LandmarkKind::Bazaar,
            position: lane_at((l0 + l1) / 2.0, along),
            corners: [lane_at(l0, r1), lane_at(l0, r0), lane_at(l1, r0), lane_at(l1, r1)],
            entrance_edge: (0, 1),
            door: lane_at(l0, along),
        }
    };
    let square = {
        let axis = (ALLEY_BEARING.to_radians().stable_cos(), ALLEY_BEARING.to_radians().stable_sin());
        let across = (-axis.1, axis.0);
        let at = |r: f64, s: f64| (cx + axis.0 * r + across.0 * s, cy + axis.1 * r + across.1 * s);
        Landmark {
            id: "bazaar_square".into(),
            kind: LandmarkKind::Square,
            position: at((SQUARE_IN + SQUARE_OUT) / 2.0, 0.0),
            corners: [at(SQUARE_OUT, SQUARE_HALF), at(SQUARE_OUT, -SQUARE_HALF), at(SQUARE_IN, -SQUARE_HALF), at(SQUARE_IN, SQUARE_HALF)],
            entrance_edge: (0, 1),
            door: at(SQUARE_OUT, 0.0),
        }
    };

    let mut rng = SplitMix64::stream(SEED, &[]);
    let mut draw = |(lo, hi): (f64, f64)| lo + (hi - lo) * rng.next_f64();
    let mut placed: Vec<[(f64, f64); 4]> = vec![mouth.corners, square.corners];
    let mut stalls: Vec<(u8, f64, Landmark)> = Vec::new();
    let try_place = |placed: &mut Vec<[(f64, f64); 4]>, corners: [(f64, f64); 4]| {
        let clear = keep_off.iter().all(|c| clear_of(&corners, c)) && placed.iter().all(|q| !overlap(q, &corners));
        if clear {
            placed.push(corners);
        }
        clear
    };
    // A stall from its face (`a` to `b`, on the street's side) back `depth`
    // along `inward`
    let quad = |a: (f64, f64), b: (f64, f64), inward: (f64, f64), depth: f64| {
        [b, a, (a.0 + inward.0 * depth, a.1 + inward.1 * depth), (b.0 + inward.0 * depth, b.1 + inward.1 * depth)]
    };

    // The lane, both sides, in from the mouth
    for side in [1.0f64, -1.0] {
        let face = along + side * (LANE_WIDTH / 2.0 + GAP);
        let mut lateral = avenue_half + MOUTH + 2.0;
        while lateral < lane_end {
            let (w, d) = (draw(FRONTAGE), draw(DEPTH));
            let corners = quad(lane_at(lateral, face), lane_at(lateral + w, face), (0.0, side), d);
            if try_place(&mut placed, corners) {
                stalls.push(stall(corners, 0, lateral));
                lateral += w;
            } else {
                lateral += 4.0;
            }
        }
    }
    // The alley, both sides, in from the ring
    let alley_axis = (ALLEY_BEARING.to_radians().stable_cos(), ALLEY_BEARING.to_radians().stable_sin());
    let alley_across = (-alley_axis.1, alley_axis.0);
    for side in [1.0f64, -1.0] {
        let off = side * (ALLEY_WIDTH / 2.0 + GAP);
        let at = |r: f64| (cx + alley_axis.0 * r + alley_across.0 * off, cy + alley_axis.1 * r + alley_across.1 * off);
        let mut r = ring_r - ring_half - GAP - 2.0;
        while r > SQUARE_OUT {
            let (w, d) = (draw(FRONTAGE), draw(DEPTH));
            let corners = quad(at(r), at(r - w), (alley_across.0 * side, alley_across.1 * side), d);
            if try_place(&mut placed, corners) {
                stalls.push(stall(corners, 2, ring_r - r));
                r -= w;
            } else {
                r -= 4.0;
            }
        }
    }
    // The market street: the ring's outer side, then its inner, from the
    // avenue round to the west arterial
    for (outer, key) in [(true, 1), (false, 1)] {
        let face = if outer { ring_r + ring_half + GAP } else { ring_r - ring_half - GAP };
        let mut deg = 90.0;
        while deg < 180.0 {
            let (w, d) = (draw(FRONTAGE), draw(DEPTH));
            let step = (w / face).to_degrees();
            let (a, b) = (polar(face, deg), polar(face, deg + step));
            let out = |deg: f64| {
                let t = deg.to_radians();
                (t.stable_cos(), t.stable_sin())
            };
            let n = out(deg + step / 2.0);
            let inward = if outer { n } else { (-n.0, -n.1) };
            let corners = quad(a, b, inward, d);
            if try_place(&mut placed, corners) {
                stalls.push(stall(corners, key, deg));
                deg += step;
            } else {
                deg += (4.0 / face).to_degrees();
            }
        }
    }

    // Along the streets from the avenue: the lane, the ring, the alley
    stalls.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut landmarks = vec![mouth, square];
    landmarks.extend(stalls.into_iter().enumerate().map(|(k, (_, _, mut l))| {
        l.id = format!("bazaar_stall_{}", k + 1);
        l
    }));
    Quarter { landmarks, roads: vec![lane, alley] }
}

/// A stall landmark from its corners (face first), with the street it's on
/// and how far along it, for the ids' order.
fn stall(corners: [(f64, f64); 4], street: u8, along: f64) -> (u8, f64, Landmark) {
    let mid = |p: (f64, f64), q: (f64, f64)| ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
    let centre = mid(mid(corners[0], corners[2]), mid(corners[1], corners[3]));
    (
        street,
        along,
        Landmark {
            id: String::new(),
            kind: LandmarkKind::Stall,
            position: centre,
            corners,
            entrance_edge: (0, 1),
            door: mid(corners[0], corners[1]),
        },
    )
}

/// Whether the quad keeps off the corridor: no point of its outline within
/// the corridor's half-width plus a hand of its edge.
fn clear_of(q: &[(f64, f64); 4], c: &Corridor) -> bool {
    let reach = c.half + GAP - 0.5;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        for k in 0..=8 {
            let t = k as f64 / 8.0;
            let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            if c.points.windows(2).any(|w| segment_distance(p, w[0], w[1]) < reach) {
                return false;
            }
        }
    }
    true
}

/// Whether two convex quads overlap by more than a hair (neighbours share an
/// edge): no edge normal of either separates them.
fn overlap(a: &[(f64, f64); 4], b: &[(f64, f64); 4]) -> bool {
    const TOUCH: f64 = 0.5;
    for q in [a, b] {
        for i in 0..4 {
            let (p, r) = (q[i], q[(i + 1) % 4]);
            let n = (r.1 - p.1, p.0 - r.0);
            let len = (n.0 * n.0 + n.1 * n.1).sqrt().max(1e-9);
            let n = (n.0 / len, n.1 / len);
            let span = |s: &[(f64, f64); 4]| {
                s.iter().map(|v| v.0 * n.0 + v.1 * n.1).fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)))
            };
            let ((a0, a1), (b0, b1)) = (span(a), span(b));
            if a1 - TOUCH <= b0 || b1 - TOUCH <= a0 {
                return false;
            }
        }
    }
    true
}

fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + dx * t, a.1 + dy * t);
    ((p.0 - qx) * (p.0 - qx) + (p.1 - qy) * (p.1 - qy)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::bazaar::{at_the_stall, STALL_RANGE};
    use capitol_protocol::terrain::quad_contains;

    fn built() -> (Quarter, (f64, f64), WorldGenParams) {
        let params = WorldGenParams::default();
        let center = (6000.0, 6000.0);
        (quarter(center, &params), center, params)
    }

    fn stalls(q: &Quarter) -> Vec<&Landmark> {
        q.landmarks.iter().filter(|l| l.kind == LandmarkKind::Stall).collect()
    }

    #[test]
    fn the_quarter_has_about_forty_stalls_numbered_from_the_avenue() {
        let (q, _, _) = built();
        let s = stalls(&q);
        println!("{} stalls", s.len());
        assert!((34..=52).contains(&s.len()), "{} stalls", s.len());
        for (k, l) in s.iter().enumerate() {
            assert_eq!(l.id, format!("bazaar_stall_{}", k + 1));
        }
        assert_eq!(q.landmarks[0].kind, LandmarkKind::Bazaar);
        assert_eq!(q.landmarks[1].id, "bazaar_square");
        // The first stall is the lane's, near the mouth
        let mouth = &q.landmarks[0];
        assert!(s[0].distance_to(mouth.door.0, mouth.door.1) < 80.0);
    }

    #[test]
    fn no_stall_overlaps_another_or_stands_on_a_road() {
        let (q, center, params) = built();
        let s = stalls(&q);
        for (i, a) in s.iter().enumerate() {
            for b in &s[i + 1..] {
                assert!(!overlap(&a.corners, &b.corners), "{} overlaps {}", a.id, b.id);
            }
            assert!(!quad_contains(&q.landmarks[1].corners, a.position.0, a.position.1), "{} in the square", a.id);
            let r = ((a.position.0 - center.0).powi(2) + (a.position.1 - center.1).powi(2)).sqrt();
            assert!(r > params.capitol_radius + params.moat_width && r < params.trade_outer_radius - 10.0, "{} at r {r}", a.id);
            // In the south-west quarter: west of the avenue, south of the west arterial
            assert!(a.position.0 < center.0 && a.position.1 > center.1, "{} at {:?}", a.id, a.position);
        }
    }

    /// Every stall's door is on a street's edge, and the crown in front of it
    /// is in its range; and the stall stands back from that street.
    #[test]
    fn every_stall_faces_a_street_within_reach() {
        let (q, center, params) = built();
        let ring: Vec<_> = (0..=90)
            .map(|k| {
                let t = (90.0 + k as f64).to_radians();
                (center.0 + params.trade_ring_radius * t.stable_cos(), center.1 + params.trade_ring_radius * t.stable_sin())
            })
            .collect();
        let streets = [ring, q.roads[0].points.clone(), q.roads[1].points.clone()];
        for s in stalls(&q) {
            let (crown, d) = streets
                .iter()
                .flat_map(|st| st.windows(2).map(|w| nearest(s.door, w[0], w[1])))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert!(d <= STALL_RANGE, "{}'s door is {d} from its street's crown", s.id);
            assert!(at_the_stall(s, crown.0, crown.1), "{}", s.id);
            // The door is the side nearest the street
            assert!(s.distance_to(crown.0, crown.1) > GAP - 0.01, "{} stands on its street", s.id);
            let back = ((s.corners[2].0 + s.corners[3].0) / 2.0, (s.corners[2].1 + s.corners[3].1) / 2.0);
            let far = ((back.0 - crown.0).powi(2) + (back.1 - crown.1).powi(2)).sqrt();
            assert!(far > d, "{} faces away from its street", s.id);
        }
    }

    #[test]
    fn the_mouth_faces_the_hall_across_the_avenue() {
        let (q, center, params) = built();
        let mouth = &q.landmarks[0];
        let moat_edge = params.capitol_radius + params.moat_width / 2.0;
        let mid_r = (moat_edge + params.trade_ring_radius - params.trade_ring_width / 2.0) / 2.0;
        assert!((mouth.door.1 - (center.1 + mid_r)).abs() < 1e-9, "level with the hall's door");
        assert!((mouth.door.0 - (center.0 - params.arterial_width / 2.0)).abs() < 1e-9, "on the avenue's kerb");
    }

    fn nearest(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> ((f64, f64), f64) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        let q = (a.0 + dx * t, a.1 + dy * t);
        (q, segment_distance(p, a, b))
    }
}
