//! Where a bot's stations go on its lot (#305). Pure, so it's tested without
//! a server.
//!
//! The server takes a station anywhere inside the lot's four corners
//! (`PlotInfo::contains`) and at least `STATION_MIN_SPACING` from every other
//! station. Lots aren't boxes: a farm is a quad at an angle to the axes, and
//! a suburban lot is often nearly a triangle, 20 across one end and 120
//! along the others. So the spots come from the quad itself: a lattice
//! [`SPOT_SPACING`] apart in its own frame, and a finer grid, in whichever
//! order packs the most stations round those already down ([`spots`]). Every
//! spot keeps [`EDGE`] in from the sides, or less on a lot too narrow for it.

use capitol_protocol::rest::PlotInfo;
use capitol_protocol::terrain::quad_contains;

/// How far apart stations go on a plot: just over `STATION_MIN_SPACING`.
pub const SPOT_SPACING: f64 = 55.0;
/// How far in from the lot's sides a station stands, where the lot is wide
/// enough: about half a forge's footprint (its ring in the Flint client is
/// about 20 in radius), so most of it stands on its own lot. More, and a
/// suburban lot that's nearly a triangle takes one station fewer.
const EDGE: f64 = 10.0;
/// The finer grid's step.
const FINE: f64 = 5.0;

/// Whether `a` and `b` are [`SPOT_SPACING`] apart, give or take rounding:
/// the lattice's own points are exactly that far.
fn clear(a: (f64, f64), b: (f64, f64)) -> bool {
    dist(a, b) >= SPOT_SPACING - 1e-6
}

/// The lot's corners, or its box for a lot from before B7.
pub fn corners(plot: &PlotInfo) -> [(f64, f64); 4] {
    plot.corners.unwrap_or_else(|| {
        let b = plot.bounds;
        [(b.min_x, b.min_y), (b.max_x, b.min_y), (b.max_x, b.max_y), (b.min_x, b.max_y)]
    })
}

fn dist((ax, ay): (f64, f64), (bx, by): (f64, f64)) -> f64 {
    (ax - bx).hypot(ay - by)
}

/// How far `p` is from the segment `a`–`b`.
fn to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// The point at `(u, v)` in the quad's own frame, each in [0, 1]: `u` along
/// its first side, `v` along its second.
fn at(c: &[(f64, f64); 4], u: f64, v: f64) -> (f64, f64) {
    let lerp = |a: (f64, f64), b: (f64, f64), t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    lerp(lerp(c[0], c[1], u), lerp(c[3], c[2], u), v)
}

/// Offsets in [0, 1] across a side `len` long: `n` points `step` apart,
/// centred, as many as fit with `edge` kept at each end.
fn lattice(len: f64, step: f64, edge: f64) -> Vec<f64> {
    if len <= 0.0 {
        return vec![0.5];
    }
    let n = (((len - 2.0 * edge).max(0.0) / step).floor() as usize) + 1;
    let first = 0.5 - (n - 1) as f64 * step / 2.0 / len;
    (0..n).map(|k| first + k as f64 * step / len).collect()
}

/// Every spot on `plot` a station could go, best first, each at least
/// [`SPOT_SPACING`] from the stations at `taken`. They aren't spaced from
/// each other: take the first, then the next clear of it, as [`room`] does.
///
/// Best first is whichever order packs the most: the lattice then the fine
/// grid, from the middle out, which suits a box; or the fine grid from either
/// end of the lot's longer axis, or out from one of its corners, which suit a
/// pie, a trapezoid or a suburban lot that's nearly a triangle, where the
/// lattice bunches up at the narrow end.
pub fn spots(plot: &PlotInfo, taken: &[(f64, f64)]) -> Vec<(f64, f64)> {
    ordered(corners(plot), taken, left(plot))
}

/// What the lot's capacity has left.
fn left(plot: &PlotInfo) -> usize {
    (plot.station_capacity - plot.station_count).max(0) as usize
}

/// [`spots`], in the first order that packs `enough`, or else the one that
/// packs the most.
fn ordered(c: [(f64, f64); 4], taken: &[(f64, f64)], enough: usize) -> Vec<(f64, f64)> {
    let len_u = (dist(c[0], c[1]) + dist(c[3], c[2])) / 2.0;
    let len_v = (dist(c[0], c[3]) + dist(c[1], c[2])) / 2.0;
    let edge = EDGE.min(0.4 * len_u.min(len_v));
    let middle = at(&c, 0.5, 0.5);
    let fits = |p: (f64, f64)| {
        quad_contains(&c, p.0, p.1)
            && (0..4).all(|i| to_segment(p, c[i], c[(i + 1) % 4]) >= edge)
            && taken.iter().all(|s| clear(*s, p))
    };
    let grid = |step: f64| -> Vec<(f64, f64)> {
        let vs = lattice(len_v, step, edge);
        lattice(len_u, step, edge)
            .into_iter()
            .flat_map(|u| vs.iter().map(move |&v| (u, v)))
            .map(|(u, v)| at(&c, u, v))
            .filter(|p| fits(*p))
            .collect()
    };
    let by = |mut points: Vec<(f64, f64)>, key: &dyn Fn((f64, f64)) -> f64| {
        points.sort_by(|a, b| key(*a).total_cmp(&key(*b)));
        points
    };
    let from_middle = |p: (f64, f64)| dist(p, middle);
    let fine = grid(FINE);
    let mut centred = by(grid(SPOT_SPACING), &from_middle);
    centred.extend(by(fine.clone(), &from_middle));
    // Along the longer axis, as a unit vector
    let (a, b) = if len_u >= len_v {
        (at(&c, 0.0, 0.5), at(&c, 1.0, 0.5))
    } else {
        (at(&c, 0.5, 0.0), at(&c, 0.5, 1.0))
    };
    let len = dist(a, b).max(1e-9);
    let axis = ((b.0 - a.0) / len, (b.1 - a.1) / len);
    let along = |p: (f64, f64)| (p.0 - a.0) * axis.0 + (p.1 - a.1) * axis.1;
    let mut orders = vec![centred, by(fine.clone(), &along), by(fine.clone(), &|p| -along(p))];
    orders.extend(c.iter().map(|&corner| by(fine.clone(), &|p| dist(p, corner))));
    let mut best = (0, Vec::new());
    for order in orders {
        let n = pack(&order, taken, enough).len();
        if n >= enough {
            return order;
        }
        if n > best.0 {
            best = (n, order);
        }
    }
    best.1
}

/// The spots of `order` a station goes on, one at a time, each clear of
/// `taken` and of those before it, `most` at most.
fn pack(order: &[(f64, f64)], taken: &[(f64, f64)], most: usize) -> Vec<(f64, f64)> {
    let mut placed: Vec<(f64, f64)> = Vec::new();
    for &p in order {
        if placed.len() >= most {
            break;
        }
        if taken.iter().chain(&placed).all(|s| clear(*s, p)) {
            placed.push(p);
        }
    }
    placed
}

/// How many more stations `plot` takes: what its capacity has left, and what
/// fits round the stations at `taken`, placed best spot first.
pub fn room(plot: &PlotInfo, taken: &[(f64, f64)]) -> i32 {
    pack(&spots(plot, taken), taken, left(plot)).len() as i32
}

/// Whether an empty `plot` takes `wanted` stations.
pub fn takes(plot: &PlotInfo, wanted: i32) -> bool {
    let wanted = (wanted.max(0) as usize).min(left(plot));
    pack(&ordered(corners(plot), &[], wanted), &[], wanted).len() >= wanted
}

/// Where `most` stations would stand on an empty lot with these corners, as
/// [`room`] places them: for drawing a lot's room, as the zone card does
/// (#444).
pub fn layout(corners: [(f64, f64); 4], most: usize) -> Vec<(f64, f64)> {
    pack(&ordered(corners, &[], most), &[], most)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::Bounds;
    use uuid::Uuid;

    fn lot(corners: [(f64, f64); 4], station_count: i32) -> PlotInfo {
        let xs = corners.map(|c| c.0);
        let ys = corners.map(|c| c.1);
        let min = |a: [f64; 4]| a.iter().cloned().fold(f64::MAX, f64::min);
        let max = |a: [f64; 4]| a.iter().cloned().fold(f64::MIN, f64::max);
        PlotInfo {
            id: Uuid::from_u128(1),
            zone_id: "suburban".into(),
            world_x: (min(xs) + max(xs)) / 2.0,
            world_y: (min(ys) + max(ys)) / 2.0,
            bounds: Bounds::new(min(xs), min(ys), max(xs), max(ys)),
            size_category: "medium".into(),
            plot_type: "residential".into(),
            owner_id: None,
            owner_name: None,
            owner_citizen: false,
            claimed_at: None,
            assessed_value: 0,
            tax_owed: 0,
            forecloses_at: None,
            deed_cost: Some(250),
            station_count,
            station_capacity: 9,
            terrain_id: None,
            corners: Some(corners),
        }
    }

    /// A `w` by `h` lot round (1000, 1000), turned `turn` radians.
    fn turned(w: f64, h: f64, turn: f64) -> [(f64, f64); 4] {
        let (s, c) = turn.sin_cos();
        [(-w, -h), (w, -h), (w, h), (-w, h)].map(|(x, y)| (1000.0 + (x * c - y * s) / 2.0, 1000.0 + (x * s + y * c) / 2.0))
    }

    fn spaced(spots: &[(f64, f64)]) -> bool {
        spots.iter().enumerate().all(|(i, a)| spots[i + 1..].iter().all(|b| clear(*a, *b)))
    }

    #[test]
    fn a_suburban_strip_takes_three_along_its_length() {
        // The soak's builders: 19 by 127, at an angle (#305)
        let plot = lot(turned(19.0, 127.0, 0.6), 0);
        assert_eq!(room(&plot, &[]), 3);
        let best: Vec<_> = spots(&plot, &[]).into_iter().take(3).collect();
        assert!(spaced(&best));
        assert!(best.iter().all(|p| plot.contains(p.0, p.1)));
    }

    #[test]
    fn a_suburban_lot_thats_nearly_a_triangle_takes_three() {
        // From the soak (#305): sides 21, 122, 119 and 127, whose builder got
        // a kiln, a forge and a sawbench onto it
        let plot = lot(
            [(8128.44, 7845.58), (8144.65, 7831.81), (8262.94, 7862.64), (8190.01, 7956.66)],
            0,
        );
        assert_eq!(room(&plot, &[]), 3);
    }

    #[test]
    fn every_spot_is_on_the_lot_not_just_in_its_box() {
        // A farm turned 45°: its box's corners are well off it
        let plot = lot(turned(200.0, 145.0, std::f64::consts::FRAC_PI_4), 0);
        let all = spots(&plot, &[]);
        assert!(!all.is_empty());
        assert!(all.iter().all(|p| plot.contains(p.0, p.1)));
        assert!(room(&plot, &[]) >= 6);
    }

    #[test]
    fn spots_keep_clear_of_the_stations_already_down() {
        let plot = lot(turned(200.0, 145.0, 0.3), 1);
        let kiln = (1000.0, 1000.0);
        let all = spots(&plot, &[kiln]);
        assert!(all.iter().all(|p| clear(*p, kiln)));
        assert!(room(&plot, &[kiln]) >= 4);
    }

    #[test]
    fn a_short_strip_with_a_station_in_its_middle_has_no_room() {
        // 20 by 76 holds two side by side, but not round one in the middle
        let plot = lot(turned(20.0, 76.0, 1.1), 0);
        assert_eq!(room(&plot, &[]), 2);
        let full = lot(turned(20.0, 76.0, 1.1), 1);
        assert_eq!(room(&full, &[(1000.0, 1000.0)]), 0);
    }

    #[test]
    fn room_stops_at_the_lots_capacity() {
        let mut plot = lot(turned(400.0, 400.0, 0.0), 0);
        plot.station_capacity = 4;
        assert_eq!(room(&plot, &[]), 4);
    }

    #[test]
    fn a_strip_doesnt_take_a_builders_four() {
        let strip = lot(turned(19.0, 127.0, 0.6), 0);
        assert!(strip.station_capacity >= 4);
        assert!(takes(&strip, 3));
        assert!(!takes(&strip, 4));
        assert!(takes(&lot(turned(200.0, 145.0, 0.3), 0), 4));
    }
}

