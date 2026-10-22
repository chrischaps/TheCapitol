//! How many stations fit on a lot, for the layouts' targets (plots plan §3),
//! and for Plots v2's farms, which must each take 12 (`RuralParams::room`,
//! V1.3, #457).
//!
//! A copy of the Flint client's packing rule, `capitol_core::sim::lot::takes`
//! (#305), which the backend can't call: stations at least `SPOT_SPACING`
//! apart inside the lot's quad, `EDGE` in from its sides, packed greedily in
//! whichever of a few orders packs the most. Keep the two in step: the
//! targets mean what a citizen finds when it builds.

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::quad_contains;

type P = (f64, f64);

/// `sim::lot::SPOT_SPACING`: just over `STATION_MIN_SPACING` (50).
const SPOT_SPACING: f64 = 55.0;
/// `sim::lot::EDGE`.
const EDGE: f64 = 10.0;
/// `sim::lot::FINE`.
const FINE: f64 = 5.0;

fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).stable_hypot(a.1 - b.1)
}

fn to_segment(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

fn at(c: &[P; 4], u: f64, v: f64) -> P {
    let lerp = |a: P, b: P, t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    lerp(lerp(c[0], c[1], u), lerp(c[3], c[2], u), v)
}

fn lattice(len: f64, step: f64, edge: f64) -> Vec<f64> {
    if len <= 0.0 {
        return vec![0.5];
    }
    let n = (((len - 2.0 * edge).max(0.0) / step).floor() as usize) + 1;
    let first = 0.5 - (n - 1) as f64 * step / 2.0 / len;
    (0..n).map(|k| first + k as f64 * step / len).collect()
}

fn pack(order: &[P], cap: usize) -> usize {
    let mut placed: Vec<P> = Vec::new();
    for &p in order {
        if placed.len() >= cap {
            break;
        }
        if placed.iter().all(|s| dist(*s, p) >= SPOT_SPACING - 1e-6) {
            placed.push(p);
        }
    }
    placed.len()
}

/// The most stations that could stand in a convex lot `edge` in from its
/// sides, by Groemer's packing bound: N points at least `d` apart in a convex
/// set of area A and perimeter P number at most 2A / (√3 d²) + P / (2d) + 1.
/// The set is the lot inset by `edge`, whose corners are where the inset
/// sides meet. `None` when the lot isn't convex or the inset doesn't stay a
/// quad, and the packing has to say. No order can pack past it, so it only
/// ever saves packing, and [`takes`] and [`most`] answer as the client does
/// (V1.3, #457).
fn most_that_fit(c: &[P; 4], edge: f64) -> Option<f64> {
    let cross = |o: P, a: P, b: P| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let turns: Vec<f64> = (0..4).map(|i| cross(c[i], c[(i + 1) % 4], c[(i + 2) % 4])).collect();
    let sign = turns[0].signum();
    if sign == 0.0 || turns.iter().any(|t| t.signum() != sign) {
        return None;
    }
    // Each side moved `edge` inward, as a point and a direction.
    let sides: Vec<(P, P)> = (0..4)
        .map(|i| {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            let len = dist(a, b);
            let d = ((b.0 - a.0) / len, (b.1 - a.1) / len);
            // Inward is to the left of a counter-clockwise walk.
            let n = (-d.1 * sign, d.0 * sign);
            ((a.0 + n.0 * edge, a.1 + n.1 * edge), d)
        })
        .collect();
    let meet = |(p, d): (P, P), (q, e): (P, P)| -> Option<P> {
        let denom = d.0 * e.1 - d.1 * e.0;
        if denom.abs() < 1e-12 {
            return None;
        }
        let s = ((q.0 - p.0) * e.1 - (q.1 - p.1) * e.0) / denom;
        Some((p.0 + d.0 * s, p.1 + d.1 * s))
    };
    let mut k = [(0.0, 0.0); 4];
    for i in 0..4 {
        k[i] = meet(sides[(i + 3) % 4], sides[i])?;
    }
    // Still a quad that runs the same way, every side forward of its own.
    for i in 0..4 {
        let (a, b) = (k[i], k[(i + 1) % 4]);
        let d = sides[i].1;
        if (b.0 - a.0) * d.0 + (b.1 - a.1) * d.1 <= 0.0 || cross(k[i], k[(i + 1) % 4], k[(i + 2) % 4]).signum() != sign {
            return None;
        }
    }
    let area = (0..4).map(|i| k[i].0 * k[(i + 1) % 4].1 - k[(i + 1) % 4].0 * k[i].1).sum::<f64>().abs() / 2.0;
    let perimeter: f64 = (0..4).map(|i| dist(k[i], k[(i + 1) % 4])).sum();
    let d = SPOT_SPACING - 1e-6;
    Some(2.0 * area / (3f64.sqrt() * d * d) + perimeter / (2.0 * d) + 1.0)
}

fn edge_of(c: &[P; 4]) -> f64 {
    let len_u = (dist(c[0], c[1]) + dist(c[3], c[2])) / 2.0;
    let len_v = (dist(c[0], c[3]) + dist(c[1], c[2])) / 2.0;
    EDGE.min(0.4 * len_u.min(len_v))
}

/// Whether an empty lot with these corners takes `wanted` stations, as
/// `sim::lot::takes` says, with its capacity out of the way. A lot the
/// packing bound rules out is answered without packing: the farms ask this
/// of every farm they try (V1.3, #457).
pub fn takes(c: &[P; 4], wanted: usize) -> bool {
    if most_that_fit(c, edge_of(c)).is_some_and(|most| most < wanted as f64) {
        return false;
    }
    most(c, wanted) >= wanted
}

/// The most stations, up to `cap`, an empty lot with these corners takes by
/// `takes`'s rule: `takes(c, n)` holds for every n up to it and none past
/// it, as each order packs greedily. One packing, not a scan of n (#468).
pub fn most(c: &[P; 4], cap: usize) -> usize {
    // No order packs past the bound, so it caps the packing too.
    let cap = match most_that_fit(c, edge_of(c)) {
        Some(bound) => cap.min(bound.max(0.0).floor() as usize),
        None => cap,
    };
    packed(c, cap)
}

/// [`most`] without the bound: the best of the client's orders, each packed
/// greedily up to `cap`. Each order is built only when the ones before it
/// fall short of `cap`, and the first starts with the coarse lattice, so a
/// lot that lattice fills needs no fine grid at all.
fn packed(c: &[P; 4], cap: usize) -> usize {
    let len_u = (dist(c[0], c[1]) + dist(c[3], c[2])) / 2.0;
    let len_v = (dist(c[0], c[3]) + dist(c[1], c[2])) / 2.0;
    let edge = edge_of(c);
    let middle = at(c, 0.5, 0.5);
    let fits = |p: P| quad_contains(c, p.0, p.1) && (0..4).all(|i| to_segment(p, c[i], c[(i + 1) % 4]) >= edge);
    let grid = |step: f64| -> Vec<P> {
        let vs = lattice(len_v, step, edge);
        lattice(len_u, step, edge)
            .into_iter()
            .flat_map(|u| vs.iter().map(move |&v| (u, v)))
            .map(|(u, v)| at(c, u, v))
            .filter(|p| fits(*p))
            .collect()
    };
    let by = |mut points: Vec<P>, key: &dyn Fn(P) -> f64| {
        points.sort_by(|a, b| key(*a).total_cmp(&key(*b)));
        points
    };
    // The centred order packs its coarse prefix first, so what that prefix
    // packs, the whole order packs at least.
    let mut centred = by(grid(SPOT_SPACING), &|p| dist(p, middle));
    let mut best = pack(&centred, cap);
    if best >= cap {
        return best;
    }
    let fine = grid(FINE);
    centred.extend(by(fine.clone(), &|p| dist(p, middle)));
    best = best.max(pack(&centred, cap));
    let (a, b) = if len_u >= len_v { (at(c, 0.0, 0.5), at(c, 1.0, 0.5)) } else { (at(c, 0.5, 0.0), at(c, 0.5, 1.0)) };
    let len = dist(a, b).max(1e-9);
    let axis = ((b.0 - a.0) / len, (b.1 - a.1) / len);
    let along = |p: P| (p.0 - a.0) * axis.0 + (p.1 - a.1) * axis.1;
    let keys: [&dyn Fn(P) -> f64; 6] = [
        &along,
        &|p| -along(p),
        &|p| dist(p, c[0]),
        &|p| dist(p, c[1]),
        &|p| dist(p, c[2]),
        &|p| dist(p, c[3]),
    ];
    for key in keys {
        if best >= cap {
            break;
        }
        best = best.max(pack(&by(fine.clone(), key), cap));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(w: f64, h: f64) -> [P; 4] {
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    /// Four need 55 × 55 of usable ground, about 75 × 75 with the edges, so
    /// 80 × 100 takes 4 and today's 64 × 106 suburban box only 3.
    #[test]
    fn a_box_takes_four_from_about_ninety_square() {
        assert!(takes(&boxed(80.0, 100.0), 4));
        assert!(takes(&boxed(64.0, 106.0), 3) && !takes(&boxed(64.0, 106.0), 4));
        assert!(!takes(&boxed(60.0, 60.0), 4));
        assert!(takes(&boxed(76.0, 76.0), 4));
        // The client's own cases (`sim::lot`'s tests): a lot from the soak
        // that's nearly a triangle (#305), and a 19 × 127 strip, take 3.
        let triangle = [(8128.44, 7845.58), (8144.65, 7831.81), (8262.94, 7862.64), (8190.01, 7956.66)];
        assert!(takes(&triangle, 3) && !takes(&triangle, 4));
        let strip = [(0.0, 0.0), (19.0, 0.0), (19.0, 127.0), (0.0, 127.0)];
        assert!(takes(&strip, 3) && !takes(&strip, 4));
    }

    /// `most` is the largest n that `takes` admits (#468).
    #[test]
    fn most_is_the_largest_n_taken() {
        let pie = [(10403.4, 9354.0), (10407.2, 9263.8), (10433.4, 9205.4), (10557.7, 9311.5)];
        for c in [boxed(80.0, 100.0), boxed(140.0, 130.0), boxed(150.0, 120.0), boxed(64.0, 106.0), pie] {
            let n = most(&c, 24);
            assert!(n > 0 && takes(&c, n) && !takes(&c, n + 1), "{c:?}: {n}");
            assert_eq!(most(&c, n - 1), n - 1);
        }
        // Three rows of three: 2 × 55 + 2 × 10 = 130 each way.
        assert_eq!(most(&boxed(140.0, 130.0), 24), 9);
    }

    /// The packing bound only ever rules out what the packing can't do: on
    /// boxes and skewed quads of every size round a farm's, no order packs
    /// past it, and `most` is what the packing alone finds.
    #[test]
    fn the_packing_bound_never_rules_out_a_lot_that_fits() {
        let mut rng = crate::models::geom::SplitMix64::stream(7, &[1]);
        let mut ruled_out = 0;
        for _ in 0..600 {
            let (w, h) = (rng.range(60.0, 200.0), rng.range(60.0, 320.0));
            let (skew, lean) = (rng.range(-30.0, 30.0), rng.range(-25.0, 25.0));
            let c = [(0.0, 0.0), (w, lean), (w + skew, h + lean), (skew, h)];
            let unbounded = packed(&c, 40);
            if let Some(bound) = most_that_fit(&c, edge_of(&c)) {
                assert!((unbounded as f64) <= bound, "{c:?} packs {unbounded}, past the bound {bound:.2}");
                ruled_out += [4, 8, 12, 15].iter().filter(|&&n| bound < n as f64).count();
            }
            assert_eq!(most(&c, 40), unbounded, "{c:?}");
            for n in [4, 8, 12, 15] {
                assert_eq!(takes(&c, n), unbounded >= n, "{c:?} takes {n}?");
            }
        }
        assert!(ruled_out > 200, "the bound ruled out only {ruled_out}");
        // A farm a hair short of 12, and one that takes them.
        assert!(most_that_fit(&boxed(140.0, 150.0), EDGE).unwrap() < 12.0);
        assert!(takes(&boxed(132.0, 190.0), 12));
    }
}
