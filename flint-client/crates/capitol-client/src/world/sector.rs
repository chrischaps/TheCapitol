//! Sectors: the world cut into wedges and rings, so a layer with thousands
//! of pieces (the roads now, the plots in P3.6) is built as one mesh per
//! sector, and each mesh is culled on its own.
//!
//! There are 16 wedges round the centre, turned half a wedge so that the
//! eight arterials and the cardinal bridges run down the middle of one
//! rather than along an edge. The rings break at the zone boundaries where
//! the city changes character: inside the outer canal, urban, suburban,
//! and rural out to the edge.

use capitol_protocol::rules::{Zone, CENTER_X, CENTER_Y};

pub const WEDGES: u8 = 16;
/// The wedge's angular width in degrees.
const WEDGE_DEG: f64 = 360.0 / WEDGES as f64;
/// Where the rings break, in server radii.
pub fn ring_edges() -> [f64; 3] {
    [
        Zone::Urban.radii().0,
        Zone::Suburban.radii().0,
        Zone::Rural.radii().0,
    ]
}
pub const RINGS: u8 = 4;

/// The angle, in degrees, at which wedge `w` starts.
fn wedge_start(w: u8) -> f64 {
    w as f64 * WEDGE_DEG - WEDGE_DEG * 0.5
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Sector {
    pub ring: u8,
    pub wedge: u8,
}

impl Sector {
    /// The sector holding the server point (x, y).
    pub fn of([x, y]: [f64; 2]) -> Self {
        let (dx, dy) = (x - CENTER_X, y - CENTER_Y);
        let r = dx.hypot(dy);
        let ring = ring_edges().iter().filter(|&&edge| r >= edge).count() as u8;
        let deg = dy.atan2(dx).to_degrees() + WEDGE_DEG * 0.5;
        let wedge = (deg.rem_euclid(360.0) / WEDGE_DEG) as u8 % WEDGES;
        Sector { ring, wedge }
    }

    /// Every sector, ring by ring.
    pub fn all() -> impl Iterator<Item = Sector> {
        (0..RINGS).flat_map(|ring| (0..WEDGES).map(move |wedge| Sector { ring, wedge }))
    }

    /// A short name for mesh and entity names: `r1w07`.
    pub fn tag(self) -> String {
        format!("r{}w{:02}", self.ring, self.wedge)
    }
}

/// Where the segment `a`→`b` (server space) crosses a sector boundary, as
/// fractions of the way along it, in (0, 1) and unsorted.
pub fn crossings(a: [f64; 2], b: [f64; 2]) -> Vec<f64> {
    let mut ts = circle_crossings(a, b, &ring_edges());
    let (ax, ay) = (a[0] - CENTER_X, a[1] - CENTER_Y);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    for w in 0..WEDGES {
        // The ray from the centre along the wedge's starting edge.
        let (uy, ux) = wedge_start(w).to_radians().sin_cos();
        let denom = ux * dy - uy * dx;
        if denom.abs() < 1e-12 {
            continue; // parallel
        }
        let t = -(ux * ay - uy * ax) / denom;
        let ahead = ux * (ax + t * dx) + uy * (ay + t * dy);
        if t > 0.0 && t < 1.0 && ahead > 0.0 {
            ts.push(t);
        }
    }
    ts
}

/// Where the segment `a`→`b` (server space) crosses circles of the given
/// radii about the centre, as fractions along it in (0, 1).
pub fn circle_crossings(a: [f64; 2], b: [f64; 2], radii: &[f64]) -> Vec<f64> {
    let (ax, ay) = (a[0] - CENTER_X, a[1] - CENTER_Y);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let qa = dx * dx + dy * dy;
    if qa == 0.0 {
        return Vec::new();
    }
    let qb = 2.0 * (ax * dx + ay * dy);
    let mut ts = Vec::new();
    for &r in radii {
        let qc = ax * ax + ay * ay - r * r;
        let disc = qb * qb - 4.0 * qa * qc;
        if disc <= 0.0 {
            continue; // misses, or only grazes
        }
        let root = disc.sqrt();
        for t in [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)] {
            if t > 0.0 && t < 1.0 {
                ts.push(t);
            }
        }
    }
    ts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polar(r: f64, deg: f64) -> [f64; 2] {
        let (s, c) = deg.to_radians().sin_cos();
        [CENTER_X + r * c, CENTER_Y + r * s]
    }

    #[test]
    fn arterials_run_down_the_middle_of_a_wedge() {
        for k in 0..8 {
            let deg = k as f64 * 45.0;
            let a = Sector::of(polar(800.0, deg));
            assert_eq!(a.wedge, (k * 2) as u8);
            // Either side of the axis is the same wedge.
            assert_eq!(Sector::of(polar(800.0, deg - 5.0)), a);
            assert_eq!(Sector::of(polar(800.0, deg + 5.0)), a);
        }
    }

    #[test]
    fn rings_break_at_the_zone_edges() {
        let ring = |r| Sector::of(polar(r, 30.0)).ring;
        assert_eq!(
            [
                ring(0.0),
                ring(999.0),
                ring(1001.0),
                ring(1700.0),
                ring(2500.0),
                ring(3999.0)
            ],
            [0, 0, 1, 2, 3, 3]
        );
    }

    #[test]
    fn every_sector_is_listed_once() {
        let all: Vec<_> = Sector::all().collect();
        assert_eq!(all.len(), RINGS as usize * WEDGES as usize);
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len());
    }

    #[test]
    fn a_segment_split_at_its_crossings_has_one_sector_per_piece() {
        let (a, b) = (polar(300.0, 5.0), polar(3000.0, 75.0));
        let mut ts = crossings(a, b);
        ts.push(0.0);
        ts.push(1.0);
        ts.sort_by(f64::total_cmp);
        let at = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let sectors: Vec<_> = ts
            .windows(2)
            .map(|w| Sector::of(at((w[0] + w[1]) * 0.5)))
            .collect();
        // Neighbouring pieces differ, and each piece is one sector all along.
        assert!(sectors.windows(2).all(|s| s[0] != s[1]));
        for (w, s) in ts.windows(2).zip(&sectors) {
            for k in 1..10 {
                let t = w[0] + (w[1] - w[0]) * k as f64 / 10.0;
                assert_eq!(Sector::of(at(t)), *s);
            }
        }
        assert!(sectors.len() >= 5, "{sectors:?}");
    }
}
