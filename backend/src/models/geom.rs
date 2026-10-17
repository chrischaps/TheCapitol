//! Plane geometry shared by the terrain generators.
//!
//! It starts with what the suburb v2 harness needs (#104). #105 moves
//! `polyline_length` and `SpatialGrid` here from terrain.rs and adds
//! `Polyline` and `SplitMix64`.

type P = (f64, f64);

/// How far two lots may reach into each other and still only touch. Lots
/// built back to back share a rear line exactly, but rounding can push one a
/// hair over it.
const TOUCH: f64 = 1e-6;

/// Whether two convex quads overlap with some area in common. Unlike
/// `terrain::polygons_overlap`, lots that only touch along an edge or at a
/// corner don't count, so back-to-back lots pass.
///
/// A separating-axis test: two convex shapes are apart exactly when some edge
/// normal of one of them has their projections meeting at most at a point.
/// A non-convex quad gets the test of its convex hull, which may report an
/// overlap that isn't there, never miss one.
#[cfg_attr(not(test), allow(dead_code))] // lots v2 (#108) uses it outside the harness
pub(crate) fn quads_overlap_strict(a: &[P; 4], b: &[P; 4]) -> bool {
    !(separated_on_edges_of(a, a, b) || separated_on_edges_of(b, a, b))
}

/// Whether one of `edges`' edge normals separates `a` from `b`.
fn separated_on_edges_of(edges: &[P; 4], a: &[P; 4], b: &[P; 4]) -> bool {
    (0..4).any(|i| {
        let (p, q) = (edges[i], edges[(i + 1) % 4]);
        let len = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
        if len == 0.0 {
            return false;
        }
        let n = (-(q.1 - p.1) / len, (q.0 - p.0) / len);
        let (a_lo, a_hi) = project(a, n);
        let (b_lo, b_hi) = project(b, n);
        a_hi <= b_lo + TOUCH || b_hi <= a_lo + TOUCH
    })
}

fn project(quad: &[P; 4], n: P) -> (f64, f64) {
    quad.iter()
        .map(|&(x, y)| x * n.0 + y * n.1)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| (lo.min(d), hi.max(d)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> [P; 4] {
        [(x, y), (x + s, y), (x + s, y + s), (x, y + s)]
    }

    #[test]
    fn touching_quads_do_not_overlap() {
        let a = square(0.0, 0.0, 10.0);
        assert!(!quads_overlap_strict(&a, &square(10.0, 0.0, 10.0)), "shared edge");
        assert!(!quads_overlap_strict(&a, &square(10.0, 10.0, 10.0)), "shared corner");
        assert!(!quads_overlap_strict(&a, &square(10.0, 3.0, 4.0)), "part of an edge");
        assert!(!quads_overlap_strict(&a, &square(30.0, 0.0, 10.0)), "apart");
    }

    #[test]
    fn crossing_and_nested_quads_overlap() {
        let a = square(0.0, 0.0, 10.0);
        assert!(quads_overlap_strict(&a, &square(9.0, 9.0, 10.0)));
        assert!(quads_overlap_strict(&a, &square(2.0, 2.0, 3.0)), "one inside the other");
        assert!(quads_overlap_strict(&a, &a));
        // A diamond just off an edge, then just over it.
        let diamond = [(10.5, 5.0), (15.0, 0.5), (19.5, 5.0), (15.0, 9.5)];
        assert!(!quads_overlap_strict(&a, &diamond));
        let diamond = [(9.5, 5.0), (15.0, -0.5), (20.5, 5.0), (15.0, 10.5)];
        assert!(quads_overlap_strict(&a, &diamond));
    }

    #[test]
    fn a_diagonal_gap_separates() {
        // Boxes overlap but the shapes don't: only a diagonal axis separates them.
        let a = [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (0.0, 5.0)];
        let b = [(9.0, 10.0), (9.0, 2.0), (11.0, 2.0), (11.0, 10.0)];
        assert!(!quads_overlap_strict(&a, &b));
    }
}
