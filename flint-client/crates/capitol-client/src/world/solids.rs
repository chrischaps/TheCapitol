//! What the camera keeps out of (E3.13): the city's buildings as simple
//! solids, a footprint and a height, so that at the shoulder the camera can
//! pull in rather than look through a wall.
//!
//! Each layer says what it stands: the bazaar's stalls (`BazaarLayer`), the
//! Exchange hall (`hall::Site::solid`) and the houses on claimed lots
//! (`houses::Houses::solids`). The tower keeps the camera out itself
//! (`camera.rhai`, `keep_radius`). The script asks [`clear_along`] through
//! `cap_clear_line`.
//!
//! A workaround, as #274's is: the engine can't ray-test the meshes it
//! draws, so the buildings are described twice, once to draw and once to
//! keep out of (#293).

use crate::world::tower::V2;

/// A building as the camera sees it: a convex footprint (world X, Z, in
/// order round it) standing from `bottom` to `top` (world Y).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Solid {
    pub corners: [V2; 4],
    pub bottom: f32,
    pub top: f32,
}

impl Solid {
    /// Whether the world point `p` is inside.
    pub fn contains(&self, p: [f32; 3]) -> bool {
        if p[1] < self.bottom || p[1] > self.top {
            return false;
        }
        let c = &self.corners;
        let side = |i: usize| {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            (b[0] - a[0]) * (p[2] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        };
        let s = [side(0), side(1), side(2), side(3)];
        s.iter().all(|&x| x >= 0.0) || s.iter().all(|&x| x <= 0.0)
    }

    /// The footprint's bounds, for skipping the far ones cheaply.
    fn bounds(&self) -> [f32; 4] {
        let xs = self.corners.map(|c| c[0]);
        let zs = self.corners.map(|c| c[1]);
        let lo = |v: [f32; 4]| v.iter().copied().fold(f32::MAX, f32::min);
        let hi = |v: [f32; 4]| v.iter().copied().fold(f32::MIN, f32::max);
        [lo(xs), lo(zs), hi(xs), hi(zs)]
    }
}

/// How far, as a share of the way from `from` to `to`, the line runs before
/// it enters one of `solids`: 1 when it's clear all the way. Sampled every
/// 5 cm, which is finer than any wall.
pub fn clear_along(solids: &[Solid], from: [f32; 3], to: [f32; 3]) -> f32 {
    let (x0, x1) = (from[0].min(to[0]), from[0].max(to[0]));
    let (z0, z1) = (from[2].min(to[2]), from[2].max(to[2]));
    let near: Vec<&Solid> = solids
        .iter()
        .filter(|s| {
            let [a, b, c, d] = s.bounds();
            a <= x1 && c >= x0 && b <= z1 && d >= z0
        })
        .collect();
    if near.is_empty() {
        return 1.0;
    }
    let len = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2) + (to[2] - from[2]).powi(2)).sqrt();
    let steps = ((len / 0.05).ceil() as usize).max(2);
    for k in 1..=steps {
        let t = k as f32 / steps as f32;
        let p = std::array::from_fn(|i| from[i] + (to[i] - from[i]) * t);
        if near.iter().any(|s| s.contains(p)) {
            return (k - 1) as f32 / steps as f32;
        }
    }
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> Solid {
        Solid { corners: [[1.0, -5.0], [2.0, -5.0], [2.0, 5.0], [1.0, 5.0]], bottom: 0.0, top: 3.0 }
    }

    #[test]
    fn a_wall_in_the_way_stops_the_line_at_its_face() {
        let t = clear_along(&[wall()], [0.0, 1.5, 0.0], [4.0, 1.5, 0.0]);
        assert!((t - 0.25).abs() < 0.02, "{t}");
    }

    #[test]
    fn over_it_or_beside_it_the_line_is_clear() {
        assert_eq!(clear_along(&[wall()], [0.0, 3.5, 0.0], [4.0, 3.6, 0.0]), 1.0);
        assert_eq!(clear_along(&[wall()], [0.0, 1.5, 6.0], [4.0, 1.5, 6.0]), 1.0);
        assert_eq!(clear_along(&[], [0.0, 1.5, 0.0], [4.0, 1.5, 0.0]), 1.0);
    }
}
