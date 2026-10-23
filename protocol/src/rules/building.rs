//! The building that comes with a deed (Plots v2 §5.1, V3.1, #473).
//!
//! Every claimed lot in a home zone has one building, on a footprint the
//! rules set: a rectangle square to the lot's street edge (corners 0 to 1),
//! set back by the zone's [`Shape::setback`], as wide as the lot allows less
//! the side [`Shape::margin`]s, and [`Shape::depth`] of the lot's depth deep.
//! The server tells indoors from the yard by it, and the clients draw the
//! building on exactly it, so what you see is what the rules count.
//!
//! The depth is a share of the lot's, so it follows the lot. The lot's depth
//! is the shallower of its back corners, measured square to the street edge.

use super::Zone;
use crate::terrain::quad_contains;

type P = (f64, f64);

/// How a zone's buildings sit on their lots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// From the street edge to the building's front, in world units
    pub setback: f64,
    /// The building's depth, as a share of the lot's
    pub depth: f64,
    /// Kept clear at each side, in world units
    pub margin: f64,
}

/// The narrowest a footprint may be, either way: any less and there's no
/// building, only a lot.
pub const MIN_SIDE: f64 = 20.0;

/// How `zone`'s buildings sit, or `None` for a zone whose lots have none:
/// only the three home zones do. Guild halls keep their own rules (S4).
///
/// - urban: a workshop house over the whole lot but a strip behind
///   (about 85 × 78 on a lot of 90 × 93)
/// - suburban: a house and workshop at the front, a garden behind
///   (about 120 × 78 on 130 × 130)
/// - rural: the farmhouse and barn standing back on the drive
///   (about 110 × 90 on 135 × 255)
pub fn shape(zone: Zone) -> Option<Shape> {
    match zone {
        Zone::Urban => Some(Shape { setback: 0.0, depth: 0.85, margin: 2.5 }),
        Zone::Suburban => Some(Shape { setback: 6.0, depth: 0.6, margin: 5.0 }),
        Zone::Rural => Some(Shape { setback: 40.0, depth: 0.35, margin: 12.0 }),
        _ => None,
    }
}

/// The building's footprint on a lot of `zone` with these `corners`, in the
/// lot's own order: its street side first (0 to 1), then its back (2, 3).
/// `None` where the zone has no buildings, or the lot is too small or
/// misshapen for one at least [`MIN_SIDE`] each way.
pub fn footprint(corners: &[P; 4], zone: Zone) -> Option<[P; 4]> {
    footprint_of(corners, shape(zone)?)
}

/// [`footprint`] for a building of this `shape`, whatever the zone.
pub fn footprint_of(corners: &[P; 4], shape: Shape) -> Option<[P; 4]> {
    let c = corners;
    let (dx, dy) = (c[1].0 - c[0].0, c[1].1 - c[0].1);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 0.0 {
        return None;
    }
    // Along the street, and square to it into the lot
    let e = (dx / len, dy / len);
    let mid = (c.iter().map(|p| p.0).sum::<f64>() / 4.0, c.iter().map(|p| p.1).sum::<f64>() / 4.0);
    let mut n = (-e.1, e.0);
    if (mid.0 - c[0].0) * n.0 + (mid.1 - c[0].1) * n.1 < 0.0 {
        n = (-n.0, -n.1);
    }
    let frame = |p: P| {
        let (x, y) = (p.0 - c[0].0, p.1 - c[0].1);
        (x * e.0 + y * e.1, x * n.0 + y * n.1)
    };
    let local: Vec<P> = c.iter().map(|&p| frame(p)).collect();
    let depth = local[2].1.min(local[3].1);
    let near = shape.setback;
    let far = (near + shape.depth * depth).min(depth);
    if far - near < MIN_SIDE {
        return None;
    }
    // A convex lot's width at a depth narrows to its ends, so the narrower of
    // the front's and the back's is the width all along.
    let (a, b) = across(&local, near)?;
    let (a2, b2) = across(&local, far)?;
    let (lo, hi) = (a.max(a2) + shape.margin, b.min(b2) - shape.margin);
    if hi - lo < MIN_SIDE {
        return None;
    }
    let world = |s: f64, t: f64| (c[0].0 + s * e.0 + t * n.0, c[0].1 + s * e.1 + t * n.1);
    Some([world(lo, near), world(hi, near), world(hi, far), world(lo, far)])
}

/// Where the line `t` deep crosses the lot (in its street frame), as the
/// span along the street, or `None` if it misses.
fn across(local: &[P], t: f64) -> Option<(f64, f64)> {
    let mut span: Option<(f64, f64)> = None;
    let mut take = |s: f64| span = Some(span.map_or((s, s), |(l, h)| (l.min(s), h.max(s))));
    for i in 0..local.len() {
        let (a, b) = (local[i], local[(i + 1) % local.len()]);
        let (lo, hi) = (a.1.min(b.1), a.1.max(b.1));
        if t < lo || t > hi {
            continue;
        }
        if hi - lo < 1e-12 {
            // A side along the line: both its ends are on it
            take(a.0);
            take(b.0);
        } else {
            take(a.0 + (b.0 - a.0) * (t - a.1) / (b.1 - a.1));
        }
    }
    span
}

/// Whether `(x, y)` is indoors on a lot of `zone` with these `corners`:
/// inside its building's footprint.
pub fn indoors(corners: &[P; 4], zone: Zone, x: f64, y: f64) -> bool {
    footprint(corners, zone).is_some_and(|f| quad_contains(&f, x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(w: f64, h: f64) -> [P; 4] {
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    fn size(f: &[P; 4]) -> (f64, f64) {
        let d = |a: P, b: P| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
        (d(f[0], f[1]), d(f[1], f[2]))
    }

    #[test]
    fn each_zone_builds_as_the_plan_says() {
        let urban = footprint(&boxed(90.0, 93.0), Zone::Urban).unwrap();
        let (w, d) = size(&urban);
        assert!((w - 85.0).abs() < 1e-9 && (d - 0.85 * 93.0).abs() < 1e-9, "{w} × {d}");
        assert_eq!(urban[0], (2.5, 0.0), "on the street");
        let (w, d) = size(&footprint(&boxed(130.0, 130.0), Zone::Suburban).unwrap());
        assert!((w - 120.0).abs() < 1e-9 && (d - 78.0).abs() < 1e-9, "{w} × {d}");
        let farm = footprint(&boxed(135.0, 255.0), Zone::Rural).unwrap();
        let (w, d) = size(&farm);
        assert!((w - 111.0).abs() < 1e-9 && (d - 89.25).abs() < 1e-9, "{w} × {d}");
        assert_eq!(farm[0].1, 40.0, "back on the drive");
        assert_eq!(footprint(&boxed(90.0, 93.0), Zone::GuildDistrict), None);
        assert_eq!(footprint(&boxed(30.0, 30.0), Zone::Suburban), None, "too small");
    }

    /// The street edge can run any way round, and the lot can lean: the
    /// footprint stays square to the street and inside the lot.
    #[test]
    fn it_stands_square_to_the_street_inside_any_lot() {
        let lots = [
            // turned the other way round
            [(90.0, 93.0), (0.0, 93.0), (0.0, 0.0), (90.0, 0.0)],
            // a trapezoid narrowing to the back, at an angle
            [(100.0, 100.0), (190.0, 140.0), (170.0, 220.0), (105.0, 190.0)],
            // a pie widening to the back
            [(0.0, 0.0), (40.0, 0.0), (90.0, 130.0), (-50.0, 130.0)],
        ];
        for lot in lots {
            let f = footprint(&lot, Zone::Suburban).unwrap();
            for p in f {
                assert!(quad_contains(&lot, p.0, p.1), "{p:?} outside {lot:?}");
            }
            let side = (f[1].0 - f[0].0, f[1].1 - f[0].1);
            let street = (lot[1].0 - lot[0].0, lot[1].1 - lot[0].1);
            let cross = side.0 * street.1 - side.1 * street.0;
            assert!(cross.abs() < 1e-6, "square to the street");
            let back = (f[2].0 - f[1].0, f[2].1 - f[1].1);
            assert!((side.0 * back.0 + side.1 * back.1).abs() < 1e-6, "a rectangle");
            let mid = ((f[0].0 + f[2].0) / 2.0, (f[0].1 + f[2].1) / 2.0);
            assert!(indoors(&lot, Zone::Suburban, mid.0, mid.1));
        }
    }
}
