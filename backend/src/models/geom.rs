//! Plane geometry shared by the terrain generators.
//!
//! The suburb v2 harness started it (#104) with `quads_overlap_strict`. #105
//! moved `polyline_length` and `SpatialGrid` here from terrain.rs and added
//! `Polyline` and `SplitMix64` for the v2 generator.

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

/// Whether a quad is convex, either winding. A corner on a straight edge,
/// or two corners at one point, still counts.
pub(crate) fn quad_is_convex(c: &[P; 4]) -> bool {
    let turns: Vec<f64> = (0..4)
        .map(|i| {
            let (a, b, d) = (c[i], c[(i + 1) % 4], c[(i + 2) % 4]);
            (b.0 - a.0) * (d.1 - b.1) - (b.1 - a.1) * (d.0 - b.0)
        })
        .collect();
    turns.iter().all(|&t| t >= -1e-9) || turns.iter().all(|&t| t <= 1e-9)
}

/// Calculate total length of a polyline
pub(crate) fn polyline_length(points: &[(f64, f64)]) -> f64 {
    let mut total = 0.0;
    for i in 1..points.len() {
        let dx = points[i].0 - points[i - 1].0;
        let dy = points[i].1 - points[i - 1].1;
        total += (dx * dx + dy * dy).sqrt();
    }
    total
}


/// Spatial hash grid for accelerating collision queries during terrain generation.
/// Divides the world into fixed-size cells and tracks which segment indices overlap each cell.
pub(crate) struct SpatialGrid {
    cells: Vec<Vec<usize>>,
    grid_width: usize,
    grid_height: usize,
    cell_size: f64,
    origin_x: f64,
    origin_y: f64,
}

impl SpatialGrid {
    /// Create a new spatial grid covering a square region centered at (center_x, center_y)
    /// with half-extent `radius` and the given cell size.
    pub(crate) fn new(center_x: f64, center_y: f64, radius: f64, cell_size: f64) -> Self {
        let origin_x = center_x - radius;
        let origin_y = center_y - radius;
        let extent = radius * 2.0;
        let grid_width = (extent / cell_size).ceil() as usize + 1;
        let grid_height = grid_width;
        let cells = vec![Vec::new(); grid_width * grid_height];
        Self {
            cells,
            grid_width,
            grid_height,
            cell_size,
            origin_x,
            origin_y,
        }
    }

    /// Convert world coordinate to grid cell index (clamped to grid bounds)
    #[inline]
    fn cell_x(&self, x: f64) -> usize {
        let cx = ((x - self.origin_x) / self.cell_size) as isize;
        cx.clamp(0, self.grid_width as isize - 1) as usize
    }

    #[inline]
    fn cell_y(&self, y: f64) -> usize {
        let cy = ((y - self.origin_y) / self.cell_size) as isize;
        cy.clamp(0, self.grid_height as isize - 1) as usize
    }

    /// Insert a polyline's AABB into all overlapping cells with the given segment index.
    pub(crate) fn insert(&mut self, segment_idx: usize, points: &[(f64, f64)]) {
        if points.is_empty() {
            return;
        }
        let (mut min_x, mut min_y) = points[0];
        let (mut max_x, mut max_y) = points[0];
        for &(x, y) in &points[1..] {
            if x < min_x { min_x = x; }
            if x > max_x { max_x = x; }
            if y < min_y { min_y = y; }
            if y > max_y { max_y = y; }
        }
        let cx0 = self.cell_x(min_x);
        let cy0 = self.cell_y(min_y);
        let cx1 = self.cell_x(max_x);
        let cy1 = self.cell_y(max_y);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                self.cells[cy * self.grid_width + cx].push(segment_idx);
            }
        }
    }

    /// Insert a polyline segment by segment: each cell gets the index only if
    /// one of the segments' own boxes reaches it. `insert` floods the road's
    /// whole bounding box, which for a curved street or a ring is most of
    /// the cells round it.
    #[cfg_attr(not(test), allow(dead_code))] // the v2 generator (#106 on) uses it
    pub(crate) fn insert_segments(&mut self, segment_idx: usize, points: &[(f64, f64)]) {
        if points.len() == 1 {
            self.insert(segment_idx, points);
            return;
        }
        for w in points.windows(2) {
            let (cx0, cx1) = (self.cell_x(w[0].0.min(w[1].0)), self.cell_x(w[0].0.max(w[1].0)));
            let (cy0, cy1) = (self.cell_y(w[0].1.min(w[1].1)), self.cell_y(w[0].1.max(w[1].1)));
            for cy in cy0..=cy1 {
                for cx in cx0..=cx1 {
                    let cell = &mut self.cells[cy * self.grid_width + cx];
                    // Neighbouring segments share cells; one entry each is enough.
                    if cell.last() != Some(&segment_idx) {
                        cell.push(segment_idx);
                    }
                }
            }
        }
    }

    /// Query for all segment indices near a point within `radius`.
    /// Returns deduplicated indices.
    #[cfg(test)]
    pub(crate) fn query_point(&self, x: f64, y: f64, radius: f64) -> Vec<usize> {
        let cx0 = self.cell_x(x - radius);
        let cy0 = self.cell_y(y - radius);
        let cx1 = self.cell_x(x + radius);
        let cy1 = self.cell_y(y + radius);
        let mut result = Vec::new();
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                result.extend_from_slice(&self.cells[cy * self.grid_width + cx]);
            }
        }
        // Sorted rather than hashed: the sets are small and the probes many.
        result.sort_unstable();
        result.dedup();
        result
    }

    /// Query for all segment indices whose AABB overlaps the given AABB.
    /// Returns deduplicated indices.
    pub(crate) fn query_aabb(&self, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<usize> {
        let cx0 = self.cell_x(min_x);
        let cy0 = self.cell_y(min_y);
        let cx1 = self.cell_x(max_x);
        let cy1 = self.cell_y(max_y);
        let mut result = Vec::new();
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                result.extend_from_slice(&self.cells[cy * self.grid_width + cx]);
            }
        }
        // Sorted rather than hashed: the sets are small and the probes many.
        result.sort_unstable();
        result.dedup();
        result
    }
}

/// A polyline with its cumulative lengths, so a point at a distance along it
/// is a binary search rather than a walk from the start. `terrain.rs`'s
/// `point_at_distance` (the rural lots use it) walks, which makes stepping
/// along a road O(n²).
#[derive(Debug, Clone)]
pub(crate) struct Polyline {
    points: Vec<P>,
    /// `cum[i]` is the length from the start to `points[i]`.
    cum: Vec<f64>,
}

impl Polyline {
    pub(crate) fn new(points: Vec<P>) -> Self {
        let mut cum = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                let q = points[i - 1];
                total += ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
            }
            cum.push(total);
        }
        Self { points, cum }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn points(&self) -> &[P] {
        &self.points
    }

    pub(crate) fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// The point `d` along the line, clamped to its ends, and the direction
    /// of the segment it falls on (radians). None for fewer than two points.
    pub(crate) fn at(&self, d: f64) -> Option<(f64, f64, f64)> {
        let n = self.points.len();
        if n < 2 {
            return None;
        }
        let d = d.clamp(0.0, self.length());
        // The first segment ending at or past d. A zero-length segment only
        // ends at d where the one before it does, so it's never picked first.
        let i = self.cum.partition_point(|&c| c < d).clamp(1, n - 1);
        let (a, b) = (self.points[i - 1], self.points[i]);
        let len = self.cum[i] - self.cum[i - 1];
        let t = if len > 0.0 { (d - self.cum[i - 1]) / len } else { 0.0 };
        Some((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, (b.1 - a.1).atan2(b.0 - a.0)))
    }
}

/// SplitMix64 (Steele, Lea and Flood, 2014, with Vigna's constants): a small,
/// fast, well-mixed generator. Plan §4.1.7 gives each neighbourhood a stream
/// of its own, so changing one doesn't re-id the lots in the others.
#[cfg_attr(not(test), allow(dead_code))] // the v2 generator (#106 on) uses it
#[derive(Debug, Clone)]
pub(crate) struct SplitMix64(u64);

#[cfg_attr(not(test), allow(dead_code))]
impl SplitMix64 {
    const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A stream of its own for one part of the world. `keys` is its address,
    /// such as `[sector, nbhd]`. Each key goes in through a full output step,
    /// so streams with neighbouring keys don't track each other.
    pub(crate) fn stream(seed: u64, keys: &[u64]) -> Self {
        let h = keys.iter().fold(Self::new(seed).next_u64(), |h, &k| {
            Self::new(h ^ k.wrapping_mul(Self::GOLDEN)).next_u64()
        });
        Self(h)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(Self::GOLDEN);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1), from the top 53 bits.
    pub(crate) fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in [0, n), for n > 0. The modulo bias is under n / 2⁶⁴.
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// Uniform in [lo, hi).
    pub(crate) fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }
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

    #[test]
    fn polyline_points_at_distance() {
        let line = Polyline::new(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 0.0), (10.0, 5.0)]);
        assert_eq!(line.length(), 15.0);
        assert_eq!(line.length(), polyline_length(line.points()));
        assert_eq!(line.at(4.0), Some((4.0, 0.0, 0.0)));
        // Past the zero-length segment, on the upright.
        let (x, y, a) = line.at(12.0).unwrap();
        assert_eq!((x, y), (10.0, 2.0));
        assert!((a - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert_eq!(line.at(-3.0).map(|p| (p.0, p.1)), Some((0.0, 0.0)), "clamped to the start");
        assert_eq!(line.at(99.0).map(|p| (p.0, p.1)), Some((10.0, 5.0)), "clamped to the end");
        assert_eq!(Polyline::new(vec![(1.0, 1.0)]).at(0.0), None);
    }

    #[test]
    fn splitmix_matches_the_reference_and_streams_differ() {
        // Vigna's splitmix64.c, seeded with 0.
        let mut r = SplitMix64::new(0);
        assert_eq!(r.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert!((0.0..1.0).contains(&SplitMix64::new(7).next_f64()));
        let x = SplitMix64::new(7).range(95.0, 130.0);
        assert!((95.0..130.0).contains(&x));

        let first = |keys: &[u64]| SplitMix64::stream(42, keys).next_u64();
        assert_eq!(first(&[3, 1]), first(&[3, 1]), "deterministic");
        assert_ne!(first(&[3, 1]), first(&[3, 0]));
        assert_ne!(first(&[3, 1]), first(&[1, 3]), "order matters");
        assert_ne!(SplitMix64::stream(43, &[3, 1]).next_u64(), first(&[3, 1]));
    }

    #[test]
    fn insert_segments_leaves_the_middle_of_a_ring_empty() {
        let ring: Vec<P> = (0..=64)
            .map(|i| {
                let a = i as f64 / 64.0 * std::f64::consts::TAU;
                (500.0 + 400.0 * a.cos(), 500.0 + 400.0 * a.sin())
            })
            .collect();
        let mut flooded = SpatialGrid::new(500.0, 500.0, 500.0, 50.0);
        flooded.insert(0, &ring);
        let mut fine = SpatialGrid::new(500.0, 500.0, 500.0, 50.0);
        fine.insert_segments(0, &ring);
        assert_eq!(flooded.query_point(500.0, 500.0, 10.0), vec![0]);
        assert!(fine.query_point(500.0, 500.0, 10.0).is_empty());
        assert_eq!(fine.query_point(900.0, 500.0, 10.0), vec![0]);
        assert!(fine.cells.iter().all(|c| c.len() <= 1), "one entry per cell");
    }
}
