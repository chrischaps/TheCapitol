//! Pattern (c), central loop (plan §4.1.6; `street_layouts.jpg` PW2).
//!
//! - **Loop.** A collector round the middle of the frame, `band` in from
//!   every edge: its rows `band` from the back lines and its legs `band`
//!   from the side lines, in the world. Its corners are tight (`CORNER`), so
//!   the rows and legs run straight and their stems are parallel combs. The
//!   inside is one block of lots back to back, with open ground down the
//!   middle where it's wider than two lots (a park, #111).
//! - **Cul-de-sacs.** Straight stems off the loop along its outward normal,
//!   a street spacing apart at mid-stem, so neighbouring stems' lots back
//!   onto each other. At the corners they fan out, one or two pointing into
//!   the frame's corner. Each runs as far as leaves room for its bulb and a
//!   ring of pies before the frame's edge, `STEM` where the loop runs along
//!   an edge, and no more than `cul_de_sac_max`.
//! - **Frontage.** Only the loop's inside has lots. Its outside is all
//!   stems' and entrance roads' lots, sideways on, as the post-war ideal had
//!   it: no house on the collector. So the stems' lots run right up to the
//!   loop.
//! - **Entrances.** Each is a road from the loop, leaving it square on and
//!   curving to cross the side line square on (a cubic Hermite), then a
//!   straight stub to the arterial. It meets the loop at the vertex nearest
//!   its gate, spread to a street spacing from the next, and the stems are
//!   laid evenly between. It has lots both sides, like a stem.
//!
//! **Roads.** The loop is one closed road, `collector_{s}_{n}_0`, starting
//! and ending at an entrance's junction. Stems are `culdesac_{s}_{n}_{j}`
//! and entrance roads `entrance_{s}_{n}_{k}`.
//!
//! **Joins.** Every stem and entrance road starts exactly on a loop vertex.
//! The loop is resampled at `STEP` in the world, so no other vertex is
//! within `SNAP` of a junction.
//!
//! Tried and dropped (#110): a stadium on a straight axis, whose round ends
//! fanned the stems so far apart that a quarter of the ground was left; and
//! stems curving from the loop to bulbs spaced along the frame's edge, whose
//! skewed, converging stems the lots walker couldn't tile.

use capitol_protocol::fmath::StableMath;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

use super::frame::{Frame, STEP};
use super::neighbourhood::{Edge, Neighbourhood};
use super::patterns::{road, Laid, Role, StreetMeta, BULB_SEGMENTS};
use crate::models::street_plan;
use crate::models::terrain::NeighbourhoodParams;

type P = (f64, f64);

/// The stem wanted from the loop to a bulb, where the loop runs along an
/// edge. It sets the band.
const STEM: f64 = 170.0;
/// A stem shorter than this isn't laid.
const STEM_MIN: f64 = 100.0;
/// (c) suits frames about as wide as they are deep: split half sectors.
/// A whole sector is about 1.6 times wider than deep, which would leave a
/// loop round a park the size of a block of (a).
const SQUARE: f64 = 1.3;
/// The corners take at most this share of the inside's half width, so the
/// two at one end never quite meet...
const ROUND: f64 = 0.98;
/// ...and are no rounder than this, so the rows and legs run straight for
/// most of their length. Round ends fan the stems out, and the wedges
/// between them are too shallow for lots.
const CORNER: f64 = 180.0;
/// Stems are spaced a street spacing apart this share of `STEM` out from
/// the loop. Out at the tips, so the fans at the corners get enough of
/// them for their lots to back onto each other.
const FAN: f64 = 1.0;
/// Neighbouring teeth are at least this share of a street spacing apart.
const MIN_SPACING: f64 = 0.9;
/// A stem that passes nearer an entrance road than this share of a street
/// spacing isn't laid.
const ENTRANCE_CLEAR: f64 = 0.4;

fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}

fn add(a: P, b: P, k: f64) -> P {
    (a.0 + b.0 * k, a.1 + b.1 * k)
}

fn len(a: P) -> f64 {
    a.0.stable_hypot(a.1)
}

fn unit(a: P) -> P {
    let l = len(a);
    (a.0 / l, a.1 / l)
}

/// The loop in a frame: its rows `band` in from the back lines, its legs
/// `band` in from the side lines, and its corners of `radius` at the inner
/// end and the outer.
#[derive(Debug, Clone, Copy)]
struct Racetrack {
    band: f64,
    radius: [f64; 2],
}

impl Racetrack {
    fn of(f: &Frame, p: &NeighbourhoodParams) -> Option<Racetrack> {
        let fit = f.fit.as_ref()?;
        let band = p.collector_width / 2.0 + STEM + 2.0 * p.bulb_radius + fit.lot_depth;
        // Half the inside at depth v, between the legs' centre lines.
        let half = |v: f64| {
            let (a, b) = (f.to_world(f.side_u(0, band, v), v), f.to_world(f.side_u(1, band, v), v));
            len(sub(a, b)) / 2.0
        };
        // The inside widens outward, so the inner corners are measured at
        // the inner row and the outer at their centres: a few rounds
        // settle that.
        let inner = (ROUND * half(band)).min(CORNER);
        let mut outer = inner;
        for _ in 0..4 {
            outer = (ROUND * half(f.depth - band - outer)).min(CORNER);
        }
        let least = p.lot_depth_min / 2.0 + p.collector_width / 2.0;
        (half(band) >= least && f.depth - 2.0 * band - inner - outer >= 0.0).then_some(Racetrack { band, radius: [inner, outer] })
    }

    /// The middle of each corner, where a stem points into the frame's corner.
    fn corner_middles(&self, f: &Frame) -> Vec<P> {
        let (b, d) = (self.band, f.depth);
        let mut out = Vec::new();
        for end in 0..2 {
            let r = self.radius[end];
            let (x, dv) = (b + r - r * FRAC_PI_4.stable_cos(), r * FRAC_PI_4.stable_sin());
            let v = if end == 0 { b + r - dv } else { d - b - r + dv };
            for side in 0..2 {
                out.push(f.to_world(f.side_u(side, x, v), v));
            }
        }
        out
    }

    /// The loop's centre line, anticlockwise, closed, at `STEP` apart.
    fn ring(&self, f: &Frame) -> Vec<P> {
        let (b, d) = (self.band, f.depth);
        // A point `x` in from side `side`, at depth v.
        let at = |side: usize, x: f64, v: f64| f.to_world(f.side_u(side, x, v), v);
        // The corner at `side` of end `end`, from its row to its leg.
        let corner = |side: usize, end: usize| -> Vec<P> {
            let r = self.radius[end];
            let vc = if end == 0 { b + r } else { d - b - r };
            let sign = if end == 0 { -1.0 } else { 1.0 };
            let m = (FRAC_PI_2 * r / STEP).ceil().max(4.0) as usize;
            (0..=m)
                .map(|i| {
                    let phi = FRAC_PI_2 * (1.0 - i as f64 / m as f64);
                    at(side, b + r - r * phi.stable_cos(), vc + sign * r * phi.stable_sin())
                })
                .collect()
        };
        let row = |end: usize, from: usize| -> Vec<P> {
            let v = if end == 0 { b } else { d - b };
            let r = self.radius[end];
            f.iso_v(v, f.side_u(from, b + r, v), f.side_u(1 - from, b + r, v))
        };
        let leg = |side: usize, up: bool| -> Vec<P> {
            let (v0, v1) = (b + self.radius[0], d - b - self.radius[1]);
            let n = ((v1 - v0) / STEP).ceil().max(1.0) as usize;
            let mut pts: Vec<P> = (0..=n).map(|i| at(side, b, v0 + (v1 - v0) * i as f64 / n as f64)).collect();
            if !up {
                pts.reverse();
            }
            pts
        };
        let rev = |mut v: Vec<P>| {
            v.reverse();
            v
        };
        // The inner row lo to hi, up the hi leg, the outer row back, down the lo leg.
        let mut pts = row(0, 0);
        pts.extend(corner(1, 0));
        pts.extend(leg(1, true));
        pts.extend(rev(corner(1, 1)));
        pts.extend(row(1, 1));
        pts.extend(corner(0, 1));
        pts.extend(leg(0, false));
        pts.extend(rev(corner(0, 0)));
        pts.push(pts[0]);
        let mut ring = street_plan::resample(&street_plan::dedupe(&pts), STEP);
        let area: f64 = ring.windows(2).map(|w| w[0].0 * w[1].1 - w[1].0 * w[0].1).sum();
        if area < 0.0 {
            ring.reverse();
        }
        ring
    }
}

/// Pattern (c) fits a frame with rows, about as wide as deep, with room
/// inside the loop for a lot each side and for the loop's legs.
pub(crate) fn fits(frame: &Frame, p: &NeighbourhoodParams) -> bool {
    frame.width <= SQUARE * frame.depth && Racetrack::of(frame, p).is_some()
}

/// The outward unit normal at each vertex of an anticlockwise closed ring.
fn normals(ring: &[P]) -> Vec<P> {
    let n = ring.len() - 1;
    (0..n)
        .map(|i| {
            let (prev, next) = (ring[(i + n - 1) % n], ring[i + 1]);
            let t = unit(sub(next, prev));
            (t.1, -t.0)
        })
        .collect()
}

/// Each vertex's place round a closed ring, measured on the line `out`
/// outside it: its length plus `out` times the angle turned. Stems spaced
/// evenly by this are evenly spaced `out` along them, at the corners as on
/// the straights. With the ring's whole length.
fn offset_lengths(ring: &[P], out: f64) -> (Vec<f64>, f64) {
    let nrm = normals(ring);
    let n = nrm.len();
    let turn = |a: P, b: P| (a.0 * b.1 - a.1 * b.0).stable_atan2(a.0 * b.0 + a.1 * b.1).abs();
    let mut acc = vec![0.0];
    for i in 1..n {
        acc.push(acc[i - 1] + len(sub(ring[i], ring[i - 1])) + out * turn(nrm[i - 1], nrm[i]));
    }
    let total = acc[n - 1] + len(sub(ring[n], ring[n - 1])) + out * turn(nrm[n - 1], nrm[0]);
    (acc, total)
}

/// The ring vertex whose place is nearest `at`, round the ring.
fn vertex_at(places: &[f64], total: f64, at: f64) -> usize {
    let d = |k: usize| {
        let x = (places[k] - at).rem_euclid(total);
        x.min(total - x)
    };
    (0..places.len()).min_by(|&i, &j| d(i).total_cmp(&d(j))).expect("a ring")
}

/// How far the ray from `o` along `d` runs before it leaves the frame.
fn to_edge(f: &Frame, o: P, d: P) -> f64 {
    let inside = |q: P| {
        let (u, v) = f.from_world(q);
        (0.0..=f.width).contains(&u) && (0.0..=f.depth).contains(&v)
    };
    let mut t = 0.0;
    while t < 2.0 * f.depth && inside(add(o, d, t + 5.0)) {
        t += 5.0;
    }
    t
}

/// A curve from `p0` heading `d0` to `p1` heading `d1`, both unit, at
/// `STEP` apart: a cubic Hermite with tangents as long as the chord.
fn hermite(p0: P, d0: P, p1: P, d1: P) -> Vec<P> {
    let k = len(sub(p1, p0));
    let m = (k / 2.0).ceil().max(8.0) as usize;
    let pts: Vec<P> = (0..=m)
        .map(|i| {
            let t = i as f64 / m as f64;
            let (t2, t3) = (t * t, t * t * t);
            let (h00, h10, h01, h11) = (2.0 * t3 - 3.0 * t2 + 1.0, t3 - 2.0 * t2 + t, -2.0 * t3 + 3.0 * t2, t3 - t2);
            (
                h00 * p0.0 + h10 * k * d0.0 + h01 * p1.0 + h11 * k * d1.0,
                h00 * p0.1 + h10 * k * d0.1 + h01 * p1.1 + h11 * k * d1.1,
            )
        })
        .collect();
    street_plan::resample(&pts, STEP)
}

/// Spread places round a ring of length `total`, given in order round it,
/// until neighbours are at least `gap` apart (or the ring's share each),
/// each pair pushed apart evenly. A place behind the one before it first
/// moves up to it, so the order holds.
fn spread(places: &mut [f64], total: f64, gap: f64) {
    let n = places.len();
    if n < 2 {
        return;
    }
    let gap = gap.min(total / n as f64);
    for i in 1..n {
        let d = (places[i] - places[i - 1]).rem_euclid(total);
        places[i] = places[i - 1] + if d > total / 2.0 { 0.0 } else { d };
    }
    for _ in 0..200 {
        let mut moved = false;
        for i in 0..n {
            let (j, wrap) = if i + 1 < n { (i + 1, 0.0) } else { (0, total) };
            let d = places[j] + wrap - places[i];
            if d < gap - 1e-6 {
                let push = (gap - d) / 2.0;
                places[i] -= push;
                places[j] += push;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    for x in places.iter_mut() {
        *x = x.rem_euclid(total);
    }
}

/// Pattern (c): a central loop with cul-de-sacs all round it.
pub(crate) fn lay(n: &mut Neighbourhood, p: &NeighbourhoodParams) -> Laid {
    let f = n.frame.clone();
    let (Some(fit), Some(track)) = (f.fit.clone(), Racetrack::of(&f, p)) else { return Laid::default() };
    let s = 2.0 * fit.lot_depth + p.local_width;
    let ring = track.ring(&f);
    let nrm = normals(&ring);
    let (places, total) = offset_lengths(&ring, p.collector_width / 2.0 + STEM * FAN);

    // Entrances: each gate, and the loop vertex nearest it, spread apart.
    let mut gates: Vec<(f64, P, usize)> = n
        .entrances
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (inset, u_side) = if e.edge == Edge::Lo { (f.lo.inset, 0.0) } else { (f.hi.inset, f.width) };
            // The depth at the side line whose foot on the arterial is `at`.
            let v = (e.along * e.along + inset * inset).sqrt() - f.base - f.warp.at(u_side);
            let gate = f.to_world(u_side, v);
            let k = (0..nrm.len()).min_by(|&a, &b| len(sub(ring[a], gate)).total_cmp(&len(sub(ring[b], gate)))).expect("a ring");
            (places[k], gate, i)
        })
        .collect();
    gates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut at: Vec<f64> = gates.iter().map(|g| g.0).collect();
    spread(&mut at, total, MIN_SPACING * s);
    let joins: Vec<usize> = at.iter().map(|&x| vertex_at(&places, total, x)).collect();

    // A stem into each of the frame's corners, the longest ground a stem
    // reaches, unless an entrance is already there.
    let nearest = |q: P| (0..nrm.len()).min_by(|&a, &b| len(sub(ring[a], q)).total_cmp(&len(sub(ring[b], q)))).expect("a ring");
    let mut stems: Vec<usize> = Vec::new();
    for q in track.corner_middles(&f) {
        let k = nearest(q);
        let near = |x: f64| {
            let d = (places[k] - x).rem_euclid(total);
            d.min(total - d) < MIN_SPACING * s
        };
        if !at.iter().copied().any(near) {
            stems.push(k);
        }
    }
    // The rest evenly between those, a spacing apart or a little more. A
    // gap splits into `parts` spans, and a stem ends each but the last,
    // which ends on the next fixed tooth.
    let mut fixed: Vec<f64> = at.iter().copied().chain(stems.iter().map(|&k| places[k])).collect();
    fixed.sort_by(f64::total_cmp);
    let mut split = |from: f64, gap: f64, last: bool| {
        let parts = ((gap / s).round() as usize).min((gap / (MIN_SPACING * s)).floor() as usize).max(1);
        let k = if last { parts } else { parts - 1 };
        for j in 1..=k {
            stems.push(vertex_at(&places, total, (from + gap * j as f64 / parts as f64).rem_euclid(total)));
        }
    };
    if fixed.is_empty() {
        split(0.0, total, true);
    } else {
        for i in 0..fixed.len() {
            let j = (i + 1) % fixed.len();
            let gap = if fixed.len() == 1 { total } else { (fixed[j] - fixed[i]).rem_euclid(total) };
            split(fixed[i], gap, false);
        }
    }

    let tag = format!("{}_{}", n.sector, n.index);
    let mut out = Laid::default();
    let meta = |id: &str, role, fronted, bulb| StreetMeta { id: id.to_string(), role, fronted, bulb, sector: n.sector, nbhd: n.index };

    // The loop, closed, from the first entrance's junction, its inside fronted.
    let mut closed = ring[..ring.len() - 1].to_vec();
    if let Some(&k) = joins.first() {
        closed.rotate_left(k);
    }
    closed.push(closed[0]);
    let id = format!("collector_{tag}_0");
    out.meta.push(meta(&id, Role::Row, (true, false), None));
    out.roads.push(road(id, closed, p.collector_width));

    // Entrance roads: out of the loop square on, curving to cross the side
    // line square on, then straight to the arterial.
    let mut entrances: Vec<Vec<P>> = Vec::new();
    for (k, (&j, &(_, gate, e))) in joins.iter().zip(&gates).enumerate() {
        let to = n.entrances[e].at;
        let mut pts = hermite(ring[j], nrm[j], gate, unit(sub(to, gate)));
        pts.extend(street_plan::resample(&[gate, to], STEP).into_iter().skip(1));
        pts[0] = ring[j];
        let id = format!("entrance_{tag}_{k}");
        out.meta.push(meta(&id, Role::Row, (true, true), None));
        out.roads.push(road(id, pts.clone(), p.local_width));
        entrances.push(pts);
    }

    // The cul-de-sacs.
    let longest = p.cul_de_sac_max - STEP;
    let reach = 2.0 * p.bulb_radius + fit.lot_depth;
    let mut j = 0;
    for &k in &stems {
        let (root, d) = (ring[k], nrm[k]);
        let stem = (to_edge(&f, root, d) - reach).min(longest);
        if stem < STEM_MIN {
            continue;
        }
        let tip = add(root, d, stem);
        let mut pts = street_plan::resample(&[root, tip], STEP);
        let clear = entrances.iter().all(|e| pts.iter().skip(3).all(|&q| e.iter().all(|&x| len(sub(q, x)) >= ENTRANCE_CLEAR * s)));
        if !clear {
            continue;
        }
        let centre = add(tip, d, p.bulb_radius);
        pts.extend(street_plan::bulb(tip, d, p.bulb_radius, BULB_SEGMENTS));
        let id = format!("culdesac_{tag}_{j}");
        out.meta.push(meta(&id, Role::CulDeSac, (true, true), Some(centre)));
        out.roads.push(road(id, pts, p.local_width));
        j += 1;
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::suburb::metrics;
    use crate::models::terrain::{NeighbourhoodPattern, TerrainConfig, WorldGenParams};

    pub(crate) fn central_world(seed: u64) -> WorldGenParams {
        let mut params = WorldGenParams { seed, ..Default::default() };
        params.suburban.neighbourhood.force_pattern = Some(NeighbourhoodPattern::CentralLoop);
        // The whole sectors, which (c) doesn't fit, get (a), so the numbers
        // compare with (a)'s own.
        params.suburban.neighbourhood.pattern_weights.loops_and_lollipops = 0.0;
        params
    }

    #[test]
    fn split_halves_fit_and_whole_sectors_do_not() {
        let params = central_world(42).suburban_params();
        let p = &params.neighbourhood;
        let all = crate::models::suburb::neighbourhood::neighbourhoods(&params, (capitol_protocol::rules::CENTER_X, capitol_protocol::rules::CENTER_Y));
        let whole = |n: &Neighbourhood| all.iter().filter(|m| m.sector == n.sector).count() == 1;
        assert!(all.iter().any(|n| !whole(n)), "seed 42 splits a sector");
        for n in &all {
            assert_eq!(fits(&n.frame, p), !whole(n), "sector {} nbhd {}", n.sector, n.index);
        }
    }

    #[test]
    fn the_ring_is_closed_anticlockwise_and_keeps_its_band() {
        let params = central_world(42).suburban_params();
        let all = crate::models::suburb::neighbourhood::neighbourhoods(&params, (capitol_protocol::rules::CENTER_X, capitol_protocol::rules::CENTER_Y));
        for n in all.iter().filter(|n| fits(&n.frame, &params.neighbourhood)) {
            let f = &n.frame;
            let track = Racetrack::of(f, &params.neighbourhood).unwrap();
            let ring = track.ring(f);
            assert_eq!(ring.first(), ring.last());
            let area: f64 = ring.windows(2).map(|w| w[0].0 * w[1].1 - w[1].0 * w[0].1).sum();
            assert!(area > 0.0, "anticlockwise");
            assert!(!street_plan::self_crosses(&ring));
            assert!(street_plan::sharpest_turn(&ring) < 20.0, "s{}·{}: a kink", n.sector, n.index);
            for w in ring.windows(2) {
                assert!(len(sub(w[1], w[0])) > street_plan::SNAP, "vertices at least SNAP apart");
            }
            // Nowhere nearer the frame's edge than the band, in v.
            for &q in &ring {
                let (_, v) = f.from_world(q);
                assert!(v >= track.band - 1.0 && v <= f.depth - track.band + 1.0, "v {v}");
            }
        }
    }

    /// Acceptance for #110: the WP4 street invariants and the WP5 lot
    /// checks, with the pattern forced to CentralLoop, at the four report seeds.
    #[test]
    fn central_loop_meets_its_targets() {
        for seed in [42, 7, 1234, 99999] {
            let params = central_world(seed);
            let t = TerrainConfig::from_params(&params);
            let sp = params.suburban_params();
            let plan = crate::models::suburb::plan(&sp, (t.center_x, t.center_y));
            let halves = plan.neighbourhoods.iter().filter(|n| fits(&n.frame, &sp.neighbourhood)).count();
            let stems = plan.roads.iter().filter(|r| r.id.starts_with("culdesac_")).count();
            assert!(halves == 0 || stems >= 5 * halves, "seed {seed}: {stems} stems in {halves} loops");

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
            // The open ground left is parks (#111): what's too thin to use is the gap.
            assert!(m.cover_gap.unwrap() <= 0.05, "{why}");
        }
    }

    #[test]
    fn every_entrance_is_reached() {
        for seed in 0..30 {
            let params = central_world(seed).suburban_params();
            let plan = crate::models::suburb::plan(&params, (capitol_protocol::rules::CENTER_X, capitol_protocol::rules::CENTER_Y));
            assert_eq!(plan.roads.len(), plan.meta.len());
            for n in &plan.neighbourhoods {
                for e in &n.entrances {
                    let ends = plan.roads.iter().flat_map(|r| [r.points[0], *r.points.last().unwrap()]);
                    assert!(ends.into_iter().any(|q| q == e.at), "seed {seed} s{}·{}: entrance not reached", n.sector, n.index);
                }
            }
        }
    }
}
