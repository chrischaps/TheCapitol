//! A neighbourhood's frame (plan §4.1.2): smooth coordinates `F(u, v) → world`
//! that the streets are laid out in.
//!
//! - **v** is radial depth, 0 at the inner back line to `depth` at the outer.
//! - **u** runs across, 0 on the `lo` side line to `width` on the `hi` one. It
//!   is in world units at mid-depth, so the warp's wavelength means the same
//!   in every neighbourhood.
//! - The side lines are straight, parallel to the arterial (or greenway)
//!   they face and inset from it by `e`. At radius r a side line sits at
//!   angle `θ ± asin(e/r)`.
//! - The warp `r += A·sin(2πu/λ + φ)` moves every iso-v line by the same
//!   amount at the same u, so rows stay exactly `depth / n` apart radially.
//!
//! The base band is inset from the zone by A at each edge, so the warped
//! back lines never leave the annulus. They touch it at the warp's crests.
//!
//! Both F and its inverse are closed forms, so the round trip is exact to
//! rounding.

use capitol_protocol::fmath::StableMath;
use std::f64::consts::TAU;

use crate::models::geom::SplitMix64;
use crate::models::terrain::NeighbourhoodParams;

type P = (f64, f64);

/// The warp's limits (plan §4.1.2). Past them, rows stop looking like
/// gently curved streets.
pub(crate) const MAX_WARP_AMPLITUDE: f64 = 40.0;
pub(crate) const MIN_WARP_WAVELENGTH: f64 = 1200.0;

/// Spacing of the points on the polylines a frame draws.
pub(crate) const STEP: f64 = 10.0;

/// A straight line parallel to the ray from the centre at `angle`, offset
/// by `inset` towards the inside of the neighbourhood.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Side {
    pub angle: f64,
    pub inset: f64,
}

/// The shared radial warp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Warp {
    pub amplitude: f64,
    pub wavelength: f64,
    pub phase: f64,
}

impl Warp {
    pub(crate) fn at(&self, u: f64) -> f64 {
        self.amplitude * (TAU * u / self.wavelength + self.phase).stable_sin()
    }
}

/// The street rows a band of depth W fits: n rows of width w, each with a lot
/// row either side of depth `d = (W − n·w) / 2n` (plan §4.1.4).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowFit {
    pub lot_depth: f64,
    /// Each row's centre line, as v.
    pub rows: Vec<f64>,
}

/// The row count whose lot depth falls in `[min, max]` and comes nearest
/// `want`, or None if no count does.
pub(crate) fn fit_rows(depth: f64, street: f64, min: f64, max: f64, want: f64) -> Option<RowFit> {
    let most = (depth / (street + 2.0 * min)).floor() as usize;
    let (n, d) = (1..=most)
        .map(|n| (n, (depth - n as f64 * street) / (2.0 * n as f64)))
        .filter(|&(_, d)| (min..=max).contains(&d))
        .min_by(|a, b| (a.1 - want).abs().total_cmp(&(b.1 - want).abs()))?;
    let pitch = depth / n as f64;
    Some(RowFit { lot_depth: d, rows: (0..n).map(|k| (k as f64 + 0.5) * pitch).collect() })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Frame {
    pub center: P,
    pub lo: Side,
    /// Unwrapped: always `lo.angle` plus less than a turn.
    pub hi: Side,
    /// Base radius of v = 0, before the warp.
    pub base: f64,
    pub depth: f64,
    pub width: f64,
    pub warp: Warp,
    pub fit: Option<RowFit>,
}

impl Frame {
    /// The frame between two sides, over the annulus `inner..outer`, with
    /// its warp phase drawn from `rng`. None when the band left inside the
    /// warp is empty, or the sides meet.
    pub(crate) fn new(
        center: P,
        lo: Side,
        hi: Side,
        (inner, outer): (f64, f64),
        p: &NeighbourhoodParams,
        rng: &mut SplitMix64,
    ) -> Option<Frame> {
        let amplitude = p.warp_amplitude.clamp(0.0, MAX_WARP_AMPLITUDE);
        let wavelength = p.warp_wavelength.max(MIN_WARP_WAVELENGTH);
        let phase = rng.range(0.0, TAU);
        let (base, depth) = (inner + amplitude, outer - inner - 2.0 * amplitude);
        let mut frame = Frame {
            center,
            lo,
            hi,
            base,
            depth,
            width: 0.0,
            warp: Warp { amplitude, wavelength, phase },
            fit: None,
        };
        // Narrowest at the inner edge, where the warp can dip to `inner`.
        if depth <= 0.0 || frame.span(inner) <= 0.0 {
            return None;
        }
        let mid = base + depth / 2.0;
        frame.width = mid * frame.span(mid);
        frame.fit = fit_rows(depth, p.local_width, p.lot_depth_min, p.lot_depth_max, p.lot_depth);
        Some(frame)
    }

    pub(crate) fn theta_lo(&self, r: f64) -> f64 {
        self.lo.angle + (self.lo.inset / r).stable_asin()
    }

    pub(crate) fn theta_hi(&self, r: f64) -> f64 {
        self.hi.angle - (self.hi.inset / r).stable_asin()
    }

    /// The angle between the side lines at radius r.
    pub(crate) fn span(&self, r: f64) -> f64 {
        self.theta_hi(r) - self.theta_lo(r)
    }

    pub(crate) fn to_world(&self, u: f64, v: f64) -> P {
        let r = self.base + v + self.warp.at(u);
        let t = u / self.width;
        let a = self.theta_lo(r) + t * self.span(r);
        (self.center.0 + r * a.stable_cos(), self.center.1 + r * a.stable_sin())
    }

    #[allow(clippy::wrong_self_convention)] // the inverse of `to_world`, in this frame
    pub(crate) fn from_world(&self, p: P) -> (f64, f64) {
        let (dx, dy) = (p.0 - self.center.0, p.1 - self.center.1);
        let r = dx.stable_hypot(dy);
        // The angle past `lo`'s ray, in (−π, π].
        let a = (dy.stable_atan2(dx) - self.lo.angle + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
        let t = (a - (self.lo.inset / r).stable_asin()) / self.span(r);
        let u = t * self.width;
        (u, r - self.warp.at(u) - self.base)
    }

    /// u at `x` in from a side line (0 lo, 1 hi), in the world, at depth v.
    /// Use it wherever something must keep a real distance from the side
    /// lines: u units shrink to about 0.8 at the inner edge.
    pub(crate) fn side_u(&self, side: usize, x: f64, v: f64) -> f64 {
        let mut u = if side == 0 { 0.0 } else { self.width };
        // The warp moves r with u, but slowly: a few rounds settle it.
        for _ in 0..4 {
            let r = self.base + v + self.warp.at(u);
            let a = if side == 0 {
                self.lo.angle + ((self.lo.inset + x) / r).stable_asin()
            } else {
                self.hi.angle - ((self.hi.inset + x) / r).stable_asin()
            };
            u = (a - self.theta_lo(r)) / self.span(r) * self.width;
        }
        u
    }

    /// The iso-v line from u0 to u1: a row street's centre line, or a back line.
    pub(crate) fn iso_v(&self, v: f64, u0: f64, u1: f64) -> Vec<P> {
        let n = ((u1 - u0).abs() / STEP).ceil().max(1.0) as usize;
        (0..=n).map(|i| self.to_world(u0 + (u1 - u0) * i as f64 / n as f64, v)).collect()
    }

    /// The iso-u line from v0 to v1: a rail between rows, or a side line.
    pub(crate) fn iso_u(&self, u: f64, v0: f64, v1: f64) -> Vec<P> {
        let n = ((v1 - v0).abs() / STEP).ceil().max(1.0) as usize;
        (0..=n).map(|i| self.to_world(u, v0 + (v1 - v0) * i as f64 / n as f64)).collect()
    }

    /// The frame's edge, anticlockwise in angle: the inner back line, the hi
    /// side, the outer back line back, the lo side down.
    pub(crate) fn outline(&self) -> Vec<P> {
        let (w, d) = (self.width, self.depth);
        let mut ring = self.iso_v(0.0, 0.0, w);
        ring.extend(self.iso_u(w, 0.0, d).into_iter().skip(1));
        ring.extend(self.iso_v(d, w, 0.0).into_iter().skip(1));
        ring.extend(self.iso_u(0.0, d, 0.0).into_iter().skip(1));
        ring.pop(); // the first point again
        ring
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_4;

    use capitol_protocol::rules::{SUBURBAN_OUTER_RADIUS as OUTER, URBAN_OUTER_RADIUS as INNER};

    use super::*;

    const C: P = (capitol_protocol::rules::CENTER_X, capitol_protocol::rules::CENTER_Y);

    fn frame(sector: f64, phase_seed: u64) -> Frame {
        let lo = Side { angle: sector * FRAC_PI_4, inset: 50.0 };
        let hi = Side { angle: (sector + 1.0) * FRAC_PI_4, inset: 50.0 };
        let p = NeighbourhoodParams::default();
        Frame::new(C, lo, hi, (INNER, OUTER), &p, &mut SplitMix64::new(phase_seed)).unwrap()
    }

    /// Distance from p to the infinite line through the centre at `angle`.
    fn off_line(p: P, angle: f64) -> f64 {
        ((p.0 - C.0) * -angle.stable_sin() + (p.1 - C.1) * angle.stable_cos()).abs()
    }

    #[test]
    fn the_inverse_undoes_the_frame() {
        // Sector 7 crosses angle 0, where atan2 wraps.
        for sector in [0.0, 3.0, 7.0] {
            let f = frame(sector, 11);
            for i in 0..=20 {
                for j in 0..=20 {
                    let (u, v) = (f.width * i as f64 / 20.0, f.depth * j as f64 / 20.0);
                    let (u2, v2) = f.from_world(f.to_world(u, v));
                    assert!((u - u2).abs() < 1e-6 && (v - v2).abs() < 1e-6, "sector {sector}: ({u}, {v}) came back ({u2}, {v2})");
                }
            }
        }
    }

    #[test]
    fn side_lines_run_parallel_to_the_arterials() {
        let f = frame(7.0, 3);
        for v in (0..=20).map(|j| f.depth * j as f64 / 20.0) {
            assert!((off_line(f.to_world(0.0, v), f.lo.angle) - 50.0).abs() < 0.5);
            assert!((off_line(f.to_world(f.width, v), f.hi.angle) - 50.0).abs() < 0.5);
        }
    }

    #[test]
    fn rows_keep_their_spacing_and_the_band_stays_in_the_zone() {
        let f = frame(2.0, 5);
        let fit = f.fit.as_ref().expect("a 1440 band fits rows");
        // 1440 less a 10 warp each side is 1420: five rows, lots 134 deep
        // (Plots v2, #456, #459; 1200 less 30 each side made them 106, #117).
        assert_eq!(fit.rows.len(), 5);
        assert!((fit.lot_depth - (1420.0 - 80.0) / 10.0).abs() < 1e-9);
        let r = |p: P| (p.0 - C.0).stable_hypot(p.1 - C.1);
        for i in 0..=40 {
            let u = f.width * i as f64 / 40.0;
            let gap = r(f.to_world(u, fit.rows[1])) - r(f.to_world(u, fit.rows[0]));
            assert!((gap - 1420.0 / 5.0).abs() < 1e-6, "radial spacing at u {u} is {gap}");
            assert!(r(f.to_world(u, 0.0)) >= INNER - 1e-9 && r(f.to_world(u, f.depth)) <= OUTER + 1e-9);
        }
    }

    #[test]
    fn row_fit_matches_the_plan() {
        // Plan §4.1.4: W = 800, n = 3 gives d ≈ 125.
        let fit = fit_rows(800.0, 16.0, 95.0, 130.0, 110.0).unwrap();
        assert_eq!(fit.rows.len(), 3);
        assert!((fit.lot_depth - 125.33).abs() < 0.01);
        for (row, want) in fit.rows.iter().zip([800.0 / 6.0, 400.0, 800.0 * 5.0 / 6.0]) {
            assert!((row - want).abs() < 1e-9, "row at {row}, want {want}");
        }
        assert!(fit_rows(150.0, 16.0, 95.0, 130.0, 110.0).is_none(), "too shallow for one row");
        // 236 is one row at exactly 110; 2.0·236 two rows at 110.
        assert_eq!(fit_rows(472.0, 16.0, 95.0, 130.0, 110.0).unwrap().rows.len(), 2);
    }

    #[test]
    fn the_outline_closes_round_the_frame() {
        let f = frame(1.0, 9);
        let ring = f.outline();
        assert!(ring.len() > 100);
        assert_ne!(ring.first(), ring.last());
        for &p in &ring {
            let (u, v) = f.from_world(p);
            let on_edge = u.abs() < 1e-6 || (u - f.width).abs() < 1e-6 || v.abs() < 1e-6 || (v - f.depth).abs() < 1e-6;
            assert!(on_edge, "({u}, {v}) is inside the frame");
        }
    }
}
