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
//!
//! A home lot has its building (Plots v2 §5.6, V3.1, #473), on
//! `rules::building::footprint`. Its room indoors and in the yard are
//! separate places: the server counts each against its own room
//! (`PlotInfo::indoor_room` and `yard_room`), and keeps the spacing within
//! each, not through the wall. So the spots go indoors first, packed in the
//! footprint round the stations inside, then in the yard, packed round the
//! footprint and the stations outside ([`Building`]).
//!
//! A building may have floors (V3.3, #475), each another place with the
//! indoor room again. [`spots`] and [`room`] are the ground's, and their
//! `taken` the stations on the ground: one upstairs takes none of the room
//! below it. [`spots_for`] and [`room_upstairs`] take the floors that are up
//! too, each storey packed round its own stations, so a citizen puts its
//! stations indoors on the ground, then upstairs, then in the yard, and
//! never a fire upstairs (#478).

use capitol_protocol::rest::PlotInfo;
use capitol_protocol::rules::{building, Zone};
use capitol_protocol::terrain::quad_contains;

/// A point in server units.
type P = (f64, f64);

/// Where a station stands on a lot, and on which storey: 0, the ground, or
/// the kth floor (V3.3).
pub type Spot = (P, i32);

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
///
/// On a lot with a building, the spots indoors come first while there's
/// room inside, then the yard's.
pub fn spots(plot: &PlotInfo, taken: &[(f64, f64)]) -> Vec<(f64, f64)> {
    match Building::of(plot) {
        None => ordered(corners(plot), taken, left(plot), None),
        Some(b) => {
            let split = b.split(plot, taken);
            let mut spots = Vec::new();
            if split.indoor_left > 0 {
                spots = ordered(b.footprint, &split.inside, split.indoor_left, None);
            }
            if split.yard_left > 0 {
                spots.extend(ordered(corners(plot), &split.outside, split.yard_left, Some(&b.footprint)));
            }
            spots
        }
    }
}

/// What the lot's capacity has left.
fn left(plot: &PlotInfo) -> usize {
    (plot.station_capacity - plot.station_count).max(0) as usize
}

/// A home lot's building, and its room indoors and in the yard, as the
/// server worked them out (V3.1, #473).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Building {
    pub footprint: [(f64, f64); 4],
    pub indoor_room: usize,
    pub yard_room: usize,
}

/// The stations round a lot with a building, by place, and the room each
/// place has left.
struct Split {
    inside: Vec<(f64, f64)>,
    outside: Vec<(f64, f64)>,
    indoor_left: usize,
    yard_left: usize,
}

impl Building {
    /// `plot`'s building, or `None` for a lot with none (a guild hall's), or
    /// whose room the server hasn't sent.
    pub fn of(plot: &PlotInfo) -> Option<Building> {
        let zone = Zone::from_str(&plot.zone_id)?;
        Some(Building {
            footprint: building::footprint(&corners(plot), zone)?,
            indoor_room: plot.indoor_room?.max(0) as usize,
            yard_room: plot.yard_room?.max(0) as usize,
        })
    }

    /// The spots on its `open` floors, storey by storey and best first on
    /// each, round the stations at `upstairs`: as many as each storey's
    /// indoor room has left, at most `left`.
    fn upstairs(&self, upstairs: &[Spot], open: i32, left: usize) -> Vec<Spot> {
        let mut out = Vec::new();
        for floor in 1..=open.max(0) {
            let on: Vec<P> = upstairs.iter().filter(|s| s.1 == floor).map(|s| s.0).collect();
            let free = self.indoor_room.saturating_sub(on.len()).min(left);
            if free > 0 {
                out.extend(ordered(self.footprint, &on, free, None).into_iter().map(|p| (p, floor)));
            }
        }
        out
    }

    /// Whether `p` is inside it.
    pub fn contains(&self, p: (f64, f64)) -> bool {
        quad_contains(&self.footprint, p.0, p.1)
    }

    /// `taken` indoors and out, and what's left of each place's room: the
    /// stations inside the footprint are this lot's indoors, and the ones
    /// on the lot outside it its yard's.
    fn split(&self, plot: &PlotInfo, taken: &[(f64, f64)]) -> Split {
        let (inside, outside): (Vec<P>, Vec<P>) = taken.iter().copied().partition(|&p| self.contains(p));
        let in_yard = outside.iter().filter(|p| plot.contains(p.0, p.1)).count();
        let left = left(plot);
        Split {
            indoor_left: self.indoor_room.saturating_sub(inside.len()).min(left),
            yard_left: self.yard_room.saturating_sub(in_yard).min(left),
            inside,
            outside,
        }
    }
}

/// [`spots`], in the first order that packs `enough`, or else the one that
/// packs the most. A spot inside `without`, or within the edge of it, is
/// left out: that's the yard round a building.
fn ordered(
    c: [(f64, f64); 4],
    taken: &[(f64, f64)],
    enough: usize,
    without: Option<&[(f64, f64); 4]>,
) -> Vec<(f64, f64)> {
    let len_u = (dist(c[0], c[1]) + dist(c[3], c[2])) / 2.0;
    let len_v = (dist(c[0], c[3]) + dist(c[1], c[2])) / 2.0;
    let edge = EDGE.min(0.4 * len_u.min(len_v));
    let middle = at(&c, 0.5, 0.5);
    let fits = |p: (f64, f64)| {
        quad_contains(&c, p.0, p.1)
            && (0..4).all(|i| to_segment(p, c[i], c[(i + 1) % 4]) >= edge)
            && without.is_none_or(|f| {
                !quad_contains(f, p.0, p.1) && (0..4).all(|i| to_segment(p, f[i], f[(i + 1) % 4]) >= edge)
            })
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
    let Some(b) = Building::of(plot) else {
        return pack(&spots(plot, taken), taken, left(plot)).len() as i32;
    };
    let s = b.split(plot, taken);
    let indoors = pack(&ordered(b.footprint, &s.inside, s.indoor_left, None), &s.inside, s.indoor_left).len();
    let yard = pack(&ordered(corners(plot), &s.outside, s.yard_left, Some(&b.footprint)), &s.outside, s.yard_left).len();
    (indoors + yard).min(left(plot)) as i32
}

/// How many more stations `plot`'s `open` floors take, round the stations
/// at `upstairs` (those on the ground are left out): each storey's indoor
/// room less what stands on it, packed as [`room`] packs the ground, all
/// within what the lot's capacity has left. None on a lot with no building.
pub fn room_upstairs(plot: &PlotInfo, upstairs: &[Spot], open: i32) -> i32 {
    let Some(b) = Building::of(plot) else {
        return 0;
    };
    let left = left(plot);
    let mut n = 0;
    for floor in 1..=open.max(0) {
        let on: Vec<P> = upstairs.iter().filter(|s| s.1 == floor).map(|s| s.0).collect();
        let free = b.indoor_room.saturating_sub(on.len()).min(left);
        n += pack(&ordered(b.footprint, &on, free, None), &on, free).len();
    }
    n.min(left) as i32
}

/// Every spot on `plot` a station could go, best first, round the stations
/// at `taken` on their storeys, with `open` floors up (#478): indoors on the
/// ground, then upstairs, then in the yard. A `fire` (`ground_only`) never
/// goes upstairs. Any other station goes upstairs before it takes the
/// ground's last `keep` spots indoors, which are left for the fires still
/// to come: an urban lot has no yard to put a kiln in. Like [`spots`], they
/// aren't spaced from each other.
pub fn spots_for(plot: &PlotInfo, taken: &[Spot], open: i32, fire: bool, keep: usize) -> Vec<Spot> {
    let ground: Vec<P> = taken.iter().filter(|s| s.1 == 0).map(|s| s.0).collect();
    let on_ground = |spots: Vec<P>| spots.into_iter().map(|p| (p, 0)).collect::<Vec<Spot>>();
    let Some(b) = Building::of(plot) else {
        return on_ground(spots(plot, &ground));
    };
    let split = b.split(plot, &ground);
    let indoors = match split.indoor_left {
        0 => Vec::new(),
        n => on_ground(ordered(b.footprint, &split.inside, n, None)),
    };
    let yard = match split.yard_left {
        0 => Vec::new(),
        n => on_ground(ordered(corners(plot), &split.outside, n, Some(&b.footprint))),
    };
    let up = if fire { Vec::new() } else { b.upstairs(taken, open, left(plot)) };
    let mut out = Vec::new();
    if fire || split.indoor_left > keep {
        out.extend(indoors);
        out.extend(up);
    } else {
        out.extend(up);
        out.extend(indoors);
    }
    out.extend(yard);
    out
}

/// Whether taking down the `movable` stations on `plot` and putting them back
/// where [`spots`] packs them leaves room for `still` more, round the
/// stations at `fixed` (#405). A kiln placed in the middle of a narrow lot
/// before #305 can leave no room for the forge, on a lot that takes both.
pub fn repack_frees(plot: &PlotInfo, fixed: &[(f64, f64)], movable: usize, still: usize) -> bool {
    let mut emptied = plot.clone();
    emptied.station_count = (plot.station_count - movable as i32).max(0);
    room(&emptied, fixed) as usize >= movable + still
}

/// Whether an empty `plot` takes `wanted` stations.
pub fn takes(plot: &PlotInfo, wanted: i32) -> bool {
    let wanted = (wanted.max(0) as usize).min(left(plot));
    match Building::of(plot) {
        None => pack(&ordered(corners(plot), &[], wanted, None), &[], wanted).len() >= wanted,
        Some(_) => room(plot, &[]) as usize >= wanted,
    }
}

/// Where `most` stations would stand on an empty lot with these corners, as
/// [`room`] places them: for drawing a lot's room, as the zone card does
/// (#444). On a lot of a zone with buildings, indoors first, then the yard
/// round the building ([`layout_in`]).
pub fn layout(corners: [(f64, f64); 4], most: usize) -> Vec<(f64, f64)> {
    pack(&ordered(corners, &[], most, None), &[], most)
}

/// [`layout`] on a lot of `zone`: its building's footprint, if it has one,
/// and `most` stations packed inside it first, then in the yard round it.
pub fn layout_in(corners: [P; 4], zone: Zone, most: usize) -> (Option<[P; 4]>, Vec<P>) {
    let Some(f) = building::footprint(&corners, zone) else {
        return (None, layout(corners, most));
    };
    let mut dots = pack(&ordered(f, &[], most, None), &[], most);
    let rest = most - dots.len();
    dots.extend(pack(&ordered(corners, &[], rest, Some(&f)), &[], rest));
    (Some(f), dots)
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
            guild: None,
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
            indoor_room: None,
            yard_room: None,
            floor_ready_at: None,
            floors: 0,
            next_floor: None,
        }
    }

    /// A `zone` lot `w` wide along its street (y = 1000) and `h` deep, with
    /// its building's room as the server would send it
    fn home(zone: &str, w: f64, h: f64, indoor: i32, yard: i32) -> PlotInfo {
        let corners = [(1000.0, 1000.0), (1000.0 + w, 1000.0), (1000.0 + w, 1000.0 + h), (1000.0, 1000.0 + h)];
        PlotInfo {
            zone_id: zone.into(),
            station_capacity: indoor + yard,
            indoor_room: Some(indoor),
            yard_room: Some(yard),
            ..lot(corners, 0)
        }
    }

    /// An urban lot: 4 indoors and no yard, so every spot is inside, and a
    /// full building leaves none (V3.1, #473)
    #[test]
    fn an_urban_lots_spots_are_indoors() {
        let mut plot = home("urban", 90.0, 93.0, 4, 0);
        let b = Building::of(&plot).unwrap();
        let all = spots(&plot, &[]);
        assert!(!all.is_empty() && all.iter().all(|p| b.contains(*p)));
        assert_eq!(room(&plot, &[]), 4);
        assert!(takes(&plot, 4));
        let four: Vec<_> = pack(&all, &[], 4);
        plot.station_count = 4;
        assert_eq!(room(&plot, &four), 0);
        assert!(spots(&plot, &four).is_empty());
    }

    /// A full urban workshop with a floor up: the room upstairs is the
    /// building's again, packed round what's on that storey, and only a
    /// floor that's up counts (#478)
    #[test]
    fn a_floor_up_has_the_buildings_room_again() {
        let mut plot = home("urban", 90.0, 93.0, 4, 0);
        plot.floors = 1;
        plot.station_capacity = 8;
        let ground: Vec<Spot> = pack(&spots(&plot, &[]), &[], 4).into_iter().map(|p| (p, 0)).collect();
        plot.station_count = 4;
        assert_eq!(room_upstairs(&plot, &ground, 1), 4);
        assert_eq!(room_upstairs(&plot, &ground, 0), 0, "the floor is still going up");
        // One upstairs takes its storey's room, not the ground's
        let up = (ground[0].0, 1);
        let mut taken = ground.clone();
        taken.push(up);
        plot.station_count = 5;
        assert_eq!(room_upstairs(&plot, &taken, 1), 3);
        let all = spots_for(&plot, &taken, 1, false, 0);
        assert!(!all.is_empty() && all.iter().all(|s| s.1 == 1), "{all:?}");
        assert!(all.iter().all(|s| clear(s.0, up.0)), "spaced round the one upstairs");
        // A fire has nowhere to go
        assert!(spots_for(&plot, &taken, 1, true, 0).is_empty());
        // A lot with no building has no upstairs
        let mut lot_only = lot(turned(200.0, 145.0, 0.3), 0);
        lot_only.floors = 1;
        assert_eq!(room_upstairs(&lot_only, &[], 1), 0);
    }

    /// Indoors on the ground first, then upstairs, then the yard; but the
    /// ground's last spots indoors are kept for the fires still to come
    #[test]
    fn upstairs_before_the_ground_the_fires_want() {
        let mut plot = home("suburban", 130.0, 130.0, 4, 3);
        plot.floors = 1;
        plot.station_capacity = 11;
        let b = Building::of(&plot).unwrap();
        let place = |s: &Spot| match (s.1, b.contains(s.0)) {
            (0, true) => "ground",
            (0, false) => "yard",
            _ => "upstairs",
        };
        let order = |spots: Vec<Spot>| {
            let mut out: Vec<&str> = spots.iter().map(place).collect();
            out.dedup();
            out
        };
        assert_eq!(order(spots_for(&plot, &[], 1, false, 0)), ["ground", "upstairs", "yard"]);
        assert_eq!(order(spots_for(&plot, &[], 1, true, 0)), ["ground", "yard"]);
        // Two down indoors, two fires to come: the bench goes upstairs
        let two: Vec<Spot> = pack(&ordered(b.footprint, &[], 2, None), &[], 2).into_iter().map(|p| (p, 0)).collect();
        plot.station_count = 2;
        assert_eq!(order(spots_for(&plot, &two, 1, false, 2)), ["upstairs", "ground", "yard"]);
        assert_eq!(order(spots_for(&plot, &two, 1, false, 1)), ["ground", "upstairs", "yard"]);
        assert_eq!(order(spots_for(&plot, &two, 1, true, 2)), ["ground", "yard"]);
    }

    /// Indoors first, then the yard, each spaced only within its own place:
    /// a yard spot may stand just the other side of the wall from a station
    /// inside
    #[test]
    fn indoors_first_then_the_yard_through_the_wall() {
        let mut plot = home("suburban", 130.0, 130.0, 4, 3);
        let b = Building::of(&plot).unwrap();
        let all = spots(&plot, &[]);
        assert!(b.contains(all[0]), "indoors first");
        assert!(all.iter().any(|p| !b.contains(*p)), "then the yard");
        assert_eq!(room(&plot, &[]), 7);
        // The house full: its back wall is at y = 1084, the yard behind
        let inside: Vec<_> = pack(&ordered(b.footprint, &[], 4, None), &[], 4);
        assert_eq!(inside.len(), 4);
        plot.station_count = 4;
        let next = spots(&plot, &inside);
        assert!(next.iter().all(|p| !b.contains(*p)), "only the yard is left");
        assert_eq!(room(&plot, &inside), 3);
        assert!(
            next.iter().any(|p| inside.iter().any(|s| !clear(*s, *p))),
            "a yard spot within 55 of a station inside"
        );
    }

    /// The card's plan: the building's footprint, and the stations inside it first
    #[test]
    fn a_layout_fills_the_building_then_the_yard() {
        let corners = [(0.0, 0.0), (130.0, 0.0), (130.0, 130.0), (0.0, 130.0)];
        let (f, dots) = layout_in(corners, Zone::Suburban, 6);
        let f = f.expect("a house");
        assert_eq!(dots.len(), 6);
        let indoors = dots.iter().filter(|p| quad_contains(&f, p.0, p.1)).count();
        assert_eq!(indoors, 4, "{dots:?}");
        let (none, _) = layout_in(corners, Zone::GuildDistrict, 4);
        assert_eq!(none, None);
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

    /// A kiln in the middle of a lot 120 by 60 leaves no room, on a lot
    /// that takes three zigzagged along it: taken down and put back, it
    /// leaves room for two more, but not for three (#405).
    #[test]
    fn a_kiln_in_the_middle_repacks_to_make_room() {
        let plot = |count| lot([(0.0, 0.0), (120.0, 0.0), (120.0, 60.0), (0.0, 60.0)], count);
        assert_eq!(room(&plot(0), &[]), 3);
        assert_eq!(room(&plot(1), &[(60.0, 30.0)]), 0);
        assert!(repack_frees(&plot(1), &[], 1, 2));
        assert!(!repack_frees(&plot(1), &[], 1, 3));
        // A station that stays, at the lot's middle, holds it as the kiln did
        assert!(!repack_frees(&plot(2), &[(60.0, 30.0)], 1, 1));
    }
}

