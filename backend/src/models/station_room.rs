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
use capitol_protocol::rules::building::{self, footprint};
use capitol_protocol::rules::Zone;
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

/// The most stations that could stand in a convex lot `edges` in from its
/// sides, by Groemer's packing bound: N points at least `d` apart in a convex
/// set of area A and perimeter P number at most 2A / (√3 d²) + P / (2d) + 1.
/// The set is the lot inset by `edges`, whose corners are where the inset
/// sides meet. `None` when the lot isn't convex or the inset doesn't stay a
/// quad, and the packing has to say. No order can pack past it, so it only
/// ever saves packing, and [`takes`] and [`most`] answer as the client does
/// (V1.3, #457).
fn most_that_fit(c: &[P; 4], edges: &[f64; 4]) -> Option<f64> {
    let cross = |o: P, a: P, b: P| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let turns: Vec<f64> = (0..4).map(|i| cross(c[i], c[(i + 1) % 4], c[(i + 2) % 4])).collect();
    let sign = turns[0].signum();
    if sign == 0.0 || turns.iter().any(|t| t.signum() != sign) {
        return None;
    }
    // Each side moved its edge inward, as a point and a direction.
    let sides: Vec<(P, P)> = (0..4)
        .map(|i| {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            let len = dist(a, b);
            let d = ((b.0 - a.0) / len, (b.1 - a.1) / len);
            // Inward is to the left of a counter-clockwise walk.
            let n = (-d.1 * sign, d.0 * sign);
            ((a.0 + n.0 * edges[i], a.1 + n.1 * edges[i]), d)
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

/// How far in from each side a station stands: [`EDGE`], or less across a
/// lot too narrow for it, 0.4 of its extent across that side. Each axis
/// keeps its own, so a lot's room along it grows as the lot widens (#563):
/// with one edge for all four, from the shorter axis, a 22 × 127 strip took
/// fewer than a 19 × 127 one.
fn edges(c: &[P; 4]) -> [f64; 4] {
    let len_u = (dist(c[0], c[1]) + dist(c[3], c[2])) / 2.0;
    let len_v = (dist(c[0], c[3]) + dist(c[1], c[2])) / 2.0;
    // Sides 0 and 2 run along u, so a station keeps from them across v
    let (across_v, across_u) = (EDGE.min(0.4 * len_v), EDGE.min(0.4 * len_u));
    [across_v, across_u, across_v, across_u]
}

/// Whether an empty lot with these corners takes `wanted` stations, as
/// `sim::lot::takes` says, with its capacity out of the way. A lot the
/// packing bound rules out is answered without packing: the farms ask this
/// of every farm they try (V1.3, #457).
pub fn takes(c: &[P; 4], wanted: usize) -> bool {
    if most_that_fit(c, &edges(c)).is_some_and(|most| most < wanted as f64) {
        return false;
    }
    most(c, wanted) >= wanted
}

/// The most stations, up to `cap`, an empty lot with these corners takes by
/// `takes`'s rule: `takes(c, n)` holds for every n up to it and none past
/// it, as each order packs greedily. One packing, not a scan of n (#468).
pub fn most(c: &[P; 4], cap: usize) -> usize {
    packed(c, None, bounded(c, cap))
}

/// `cap`, or less where the packing bound says no order packs that many: it
/// caps the packing too.
fn bounded(c: &[P; 4], cap: usize) -> usize {
    match most_that_fit(c, &edges(c)) {
        Some(bound) => cap.min(bound.max(0.0).floor() as usize),
        None => cap,
    }
}

/// More than any lot the world lays holds, so a room is never cut short.
const ROOM_CAP: usize = 64;

/// A lot's room for stations with its building on it (Plots v2 §5.6, V3.1,
/// #473): how many stand indoors on each storey, and how many in the yard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Room {
    pub indoor: usize,
    pub yard: usize,
}

impl Room {
    /// What the lot holds with `floors` bought: indoors on every storey,
    /// and the yard.
    pub fn capacity(&self, floors: usize) -> usize {
        self.indoor * (1 + floors) + self.yard
    }
}

/// The room on a lot of `zone` with these corners: inside its building's
/// footprint (`rules::building::footprint`), packed as a lot is, and in the
/// yard round it, with the footprint and [`EDGE`] round it taken out. A wall
/// stands between the two, so the spacing holds within each, not across.
/// `None` for a zone whose lots have no building.
pub fn room(c: &[P; 4], zone: Zone) -> Option<Room> {
    let Some(f) = footprint(c, zone) else {
        // A home lot too small for a building is all yard
        return building::shape(zone).map(|_| Room { indoor: 0, yard: most(c, ROOM_CAP) });
    };
    Some(Room { indoor: most(&f, ROOM_CAP), yard: packed(c, Some(&f), bounded(c, ROOM_CAP)) })
}

/// [`most`] without the bound: the best of the client's orders, each packed
/// greedily up to `cap`. Each order is built only when the ones before it
/// fall short of `cap`, and the first starts with the coarse lattice, so a
/// lot that lattice fills needs no fine grid at all.
fn packed(c: &[P; 4], without: Option<&[P; 4]>, cap: usize) -> usize {
    let len_u = (dist(c[0], c[1]) + dist(c[3], c[2])) / 2.0;
    let len_v = (dist(c[0], c[3]) + dist(c[1], c[2])) / 2.0;
    let edges = edges(c);
    // Off the building's walls by the narrower
    let edge = edges[0].min(edges[1]);
    let middle = at(c, 0.5, 0.5);
    // Give or take rounding: a row of `staggered`'s stands exactly its edge in
    let fits = |p: P| quad_contains(c, p.0, p.1) && (0..4).all(|i| to_segment(p, c[i], c[(i + 1) % 4]) >= edges[i] - 1e-6);
    // Out in the yard: not in the building, nor against its walls
    let fits = |p: P| {
        fits(p)
            && without.is_none_or(|f| !quad_contains(f, p.0, p.1) && (0..4).all(|i| to_segment(p, f[i], f[(i + 1) % 4]) >= edge - 1e-6))
    };
    let grid = |step: f64| -> Vec<P> {
        let vs = lattice(len_v, step, edges[0]);
        lattice(len_u, step, edges[1])
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
    if best < cap {
        staggered(c, &edges, &fits, best, &mut |rows| {
            best = best.max(rows.len().min(cap));
            best >= cap
        });
    }
    best
}

/// `sim::lot::staggered`: rows of stations along each side of the lot, of
/// three kinds. Even rows are spread over the lot's depth, each shifted from
/// the last as far as its spacing needs (a square grid once they're
/// `SPOT_SPACING` apart), started at a few phases along the side. Or full
/// rows mixed with shifted ones, each shifted row as close to the full rows
/// either side as the spacing lets it: full rows spread over the width with
/// the shifted ones between their stations (a hex packing at the closest),
/// or full rows the spacing apart from the end with the shifted ones set a
/// few offsets along them, with a column down the far end. Each is spaced
/// by construction. `visit` is given those that might beat `best`, kept to the points that `fits`,
/// and stops them by returning true. `edges` is how far in from each side.
///
/// On a box, the rows along its first two sides (at phase 0) pack no fewer
/// on a box at least as large each way, so what a lot takes never falls as
/// it grows (#467): the greedy orders alone left cliffs, a 150 × 120 box
/// taking 6 where a 140 × 127 one takes 8. The rows set from the end, with
/// their column, take the odd packings the greedy orders found by luck, as 10
/// in 127 × 180 (#563).
fn staggered(
    c: &[P; 4],
    edges: &[f64; 4],
    fits: &dyn Fn(P) -> bool,
    best: usize,
    visit: &mut dyn FnMut(Vec<P>) -> bool,
) {
    let d = SPOT_SPACING;
    let best = std::cell::Cell::new(best);
    let middle = ((c[0].0 + c[1].0 + c[2].0 + c[3].0) / 4.0, (c[0].1 + c[1].1 + c[2].1 + c[3].1) / 4.0);
    let mut offer = |points: Vec<P>| -> bool {
        best.set(best.get().max(points.len()));
        visit(points)
    };
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        let len = dist(a, b);
        if len < 1e-9 {
            continue;
        }
        let dir = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        // Across the lot from this side
        let mut n = (-dir.1, dir.0);
        if (middle.0 - a.0) * n.0 + (middle.1 - a.1) * n.1 < 0.0 {
            n = (-n.0, -n.1);
        }
        let x = |p: P| (p.0 - a.0) * dir.0 + (p.1 - a.1) * dir.1;
        let y = |p: P| (p.0 - a.0) * n.0 + (p.1 - a.1) * n.1;
        let (x0, x1) = c.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &p| (lo.min(x(p)), hi.max(x(p))));
        let (y0, y1) = c.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &p| (lo.min(y(p)), hi.max(y(p))));
        // In from the ends by the sides either side of this one, and from
        // this side and the one across by its own
        let (ex, ey) = (edges[(i + 1) % 4], edges[i]);
        let (wide, deep) = ((x1 - x0 - 2.0 * ex).max(0.0), (y1 - y0 - 2.0 * ey).max(0.0));
        let at = |u: f64, v: f64| (a.0 + dir.0 * (x0 + ex + u) + n.0 * (y0 + ey + v), a.1 + dir.1 * (x0 + ex + u) + n.1 * (y0 + ey + v));
        // Even rows spread over the depth, each shifted from the last as far
        // as its spacing needs: a square grid once they're the spacing apart
        let most_rows = (2.0 * deep / d + 1e-9).floor() as usize + 1;
        for rows in 1..=most_rows {
            let step_y = if rows > 1 { deep / (rows - 1) as f64 } else { 0.0 };
            let shift = if rows > 1 && step_y < d { (d * d - step_y * step_y).sqrt() } else { 0.0 };
            let step_x = d.max(2.0 * shift);
            // No phase of these rows packs more than a full row each
            if rows * ((wide / step_x + 1e-9).floor() as usize + 1) + 2 <= best.get() {
                continue;
            }
            for parity in 0..if shift > 0.0 { 2 } else { 1 } {
                let mut phase = 0.0;
                while phase < step_x - 1e-9 {
                    let mut points = Vec::new();
                    for k in 0..rows {
                        let mut u = phase + ((k + parity) % 2) as f64 * shift - step_x;
                        while u <= wide + 1e-6 {
                            let p = at(u, k as f64 * step_y);
                            if fits(p) {
                                points.push(p);
                            }
                            u += step_x;
                        }
                    }
                    if offer(points) {
                        return;
                    }
                    phase += 2.0 * FINE;
                }
            }
        }
        // Full rows and shifted rows: `full` across, and `between` in a
        // shifted row, `near` from the full rows either side of it. Each
        // shifted row has full rows either side, and there are as many
        // changes and full rows after full ones as the depth takes, the full
        // ones first or last.
        let mut mixed = |full: &[f64], between: &[f64], near: f64, column: bool| -> bool {
            if most_rows * full.len().max(between.len()) + 2 <= best.get() {
                return false;
            }
            for switches in 0..=(deep / near + 1e-9).floor() as usize {
                let alike = ((deep - switches as f64 * near) / d + 1e-9).floor() as usize;
                for alike_first in [true, false] {
                    let mut gaps = vec![d; alike];
                    let changes = (0..switches).map(|_| near);
                    if alike_first {
                        gaps.extend(changes);
                    } else {
                        gaps.splice(0..0, changes);
                    }
                    let mut points = Vec::new();
                    let (mut v, mut shifted) = (0.0, false);
                    for k in 0..=gaps.len() {
                        if k > 0 {
                            v += gaps[k - 1];
                            shifted = gaps[k - 1] < d && !shifted;
                        }
                        for &u in if shifted { between } else { full } {
                            let p = at(u, v);
                            if fits(p) {
                                points.push(p);
                            }
                        }
                    }
                    // A column down the far end, where rows set from the
                    // near one leave room: the most a line there takes,
                    // packed from this side (#563)
                    if column {
                        let mut v = 0.0;
                        while v <= deep + 1e-6 {
                            let p = at(wide, v);
                            if fits(p) && points.iter().all(|s| dist(*s, p) >= d - 1e-6) {
                                points.push(p);
                            }
                            v += FINE;
                        }
                    }
                    if offer(points) {
                        return true;
                    }
                }
            }
            false
        };
        let near = |shift: f64| (d * d - shift * shift).max(0.0).sqrt().max(d / 2.0);
        // Full rows spread over the width, the shifted ones between their
        // stations; with one across, the shifted row is at the far side
        for across in 1..=(wide / d + 1e-9).floor() as usize + 1 {
            let (full, between, shift): (Vec<f64>, Vec<f64>, f64) = match across {
                1 => (vec![0.0], vec![wide], wide),
                k => {
                    let step = wide / (k - 1) as f64;
                    ((0..k).map(|j| j as f64 * step).collect(), (0..k - 1).map(|j| (j as f64 + 0.5) * step).collect(), step / 2.0)
                }
            };
            if mixed(&full, &between, near(shift), false) {
                return;
            }
        }
        // Full rows the spacing apart from the near end, and shifted rows
        // set `shift` along them, as many as fit, and a column at the far end
        let full: Vec<f64> = (0..(wide / d + 1e-9).floor() as usize + 1).map(|j| j as f64 * d).collect();
        for shift in [27.5, 25.0, 20.0, 15.0, 10.0, 5.0] {
            if shift > wide {
                continue;
            }
            let between: Vec<f64> = (0..((wide - shift) / d + 1e-9).floor() as usize + 1).map(|j| shift + j as f64 * d).collect();
            if mixed(&full, &between, near(shift), true) {
                return;
            }
        }
    }
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
        // that's nearly a triangle (#305) takes 3, and a 19 × 127 strip 2:
        // 3 until its ends kept the full 10 (#563).
        let triangle = [(8128.44, 7845.58), (8144.65, 7831.81), (8262.94, 7862.64), (8190.01, 7956.66)];
        assert!(takes(&triangle, 3) && !takes(&triangle, 4));
        let strip = [(0.0, 0.0), (19.0, 0.0), (19.0, 127.0), (0.0, 127.0)];
        assert!(takes(&strip, 2) && !takes(&strip, 3));
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

    /// A box at least as large each way never takes fewer (#467, #563), on
    /// every box from 20 to 200 each way in steps of 2. At steps of 1 it
    /// holds too, on all 32,761 (about 12 s in release, too slow for here).
    /// The cliffs #467 found are gone: 150 × 120 takes the 8 that 140 × 127
    /// does, and the hex rows fit 8 in 130 × 116. So are #563's: the odd 10
    /// in 127 × 180 holds a unit larger, and a strip's room along it doesn't
    /// shrink as it widens.
    #[test]
    fn a_larger_box_never_takes_fewer() {
        let sizes: Vec<f64> = (0..91).map(|k| 20.0 + 2.0 * k as f64).collect();
        let most_of: Vec<Vec<usize>> = sizes.iter().map(|&w| sizes.iter().map(|&h| most(&boxed(w, h), 40)).collect()).collect();
        for (i, row) in most_of.iter().enumerate() {
            for (j, &n) in row.iter().enumerate() {
                if i > 0 {
                    assert!(n >= most_of[i - 1][j], "{} × {} takes {n}, {} × {} {}", sizes[i], sizes[j], sizes[i - 1], sizes[j], most_of[i - 1][j]);
                }
                if j > 0 {
                    assert!(n >= row[j - 1], "{} × {} takes {n}, {} × {} {}", sizes[i], sizes[j], sizes[i], sizes[j - 1], row[j - 1]);
                }
            }
        }
        assert!(takes(&boxed(150.0, 120.0), 8) && takes(&boxed(140.0, 127.0), 8) && takes(&boxed(160.0, 115.0), 8));
        assert!(takes(&boxed(130.0, 116.0), 8));
        assert_eq!(takes(&boxed(143.0, 128.6), 8), takes(&boxed(143.0, 128.9), 8));
        for h in 180..=184 {
            assert!(takes(&boxed(127.0, h as f64), 10), "127 × {h} takes 10");
        }
        let strip = |w: f64| most(&boxed(w, 127.0), 40);
        assert!(strip(19.0) <= strip(22.0) && strip(22.0) <= strip(25.0));
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
            let unbounded = packed(&c, None, 40);
            if let Some(bound) = most_that_fit(&c, &edges(&c)) {
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
        assert!(most_that_fit(&boxed(140.0, 150.0), &[EDGE; 4]).unwrap() < 12.0);
        assert!(takes(&boxed(132.0, 190.0), 12));
    }
}

#[cfg(test)]
mod world_tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Every home lot of the default world (Plots v2's, the grown one since
    /// the flip, #459) has its building and its room (V3.1, #473): every
    /// urban lot 4 indoors and none in the yard, the suburb about 5 and 3 with
    /// nine in ten taking a whole workshop, as the lots alone did, and every
    /// farm the 12 it was laid for (V1.3, #457). The rows along a side
    /// (#467) put a hex row of 3 and 2 in many a suburban house and farm
    /// house, where the greedy orders found 4.
    #[test]
    fn every_home_lot_has_its_room() {
        let world = crate::models::TerrainConfig::default_world();
        let mut by: BTreeMap<&str, Vec<Room>> = BTreeMap::new();
        for lot in &world.terrain_plots {
            let Some(zone) = Zone::from_str(&lot.zone) else { continue };
            let Some(room) = room(&lot.corners, zone) else { continue };
            by.entry(lot.zone.as_str()).or_default().push(room);
        }
        let median = |mut xs: Vec<usize>| {
            xs.sort_unstable();
            xs[xs.len() / 2]
        };
        let urban = &by["urban"];
        assert!(urban.len() > 1000 && urban.iter().all(|r| *r == Room { indoor: 4, yard: 0 }));
        let suburb = &by["suburban"];
        assert_eq!(median(suburb.iter().map(|r| r.indoor).collect()), 5);
        assert_eq!(median(suburb.iter().map(|r| r.capacity(0)).collect()), 8);
        let workshops = suburb.iter().filter(|r| r.capacity(0) >= 6).count();
        assert!(workshops * 100 >= suburb.len() * 90, "{workshops} of {} take a workshop", suburb.len());
        let farms = &by["rural"];
        assert_eq!(median(farms.iter().map(|r| r.indoor).collect()), 5);
        assert!(farms.iter().all(|r| r.capacity(0) >= 12), "every farm takes 12");
    }
}
