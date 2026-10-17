//! A keyframed camera move for captures (`--camera-path`, issue #92).
//!
//! A path is a list of keys, each a game time and a camera: where it stands
//! and what it looks at, in world metres. `scripts/camera.rhai` plays it as
//! a moving shot (`cap_camera_path`): the camera is wherever the path is at
//! that moment, held at the last key once it ends, and the first camera
//! input lets go of it into the rig, like any fixed shot.
//!
//! The file is one key a line, `#` starts a comment:
//!
//! ```text
//! # t     px    py     pz      tx   ty   tz
//! 0.0   -348.0  26.0 -127.0   0.0 30.0  0.0
//! 6.0    -60.0  40.0  -25.0   0.0 20.0  0.0
//! ```
//!
//! Between keys the camera moves on a cubic through them, so it never stops
//! dead at a key it passes. It starts and ends at rest, and a key repeated
//! later is a hold: the camera settles there, waits, and sets off again.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// A camera at one moment: where it stands, then what it looks at.
pub type Pose = [f64; 6];

#[derive(Clone, Debug, PartialEq)]
pub struct CameraPath {
    keys: Vec<(f64, Pose)>,
}

impl CameraPath {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading the camera path {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in the camera path {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let mut keys: Vec<(f64, Pose)> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let n: Vec<f64> = line
                .split_whitespace()
                .map(str::parse)
                .collect::<Result<_, _>>()
                .with_context(|| format!("line {}: not a number", i + 1))?;
            let [t, px, py, pz, tx, ty, tz] = n[..] else {
                bail!(
                    "line {}: want 7 numbers (t px py pz tx ty tz), got {}",
                    i + 1,
                    n.len()
                );
            };
            if let Some(&(last, _)) = keys.last() {
                if t <= last {
                    bail!("line {}: time {t} isn't after {last}", i + 1);
                }
            }
            keys.push((t, [px, py, pz, tx, ty, tz]));
        }
        if keys.is_empty() {
            bail!("no keys");
        }
        Ok(Self { keys })
    }

    /// The camera at game time `t`: the first key before the path starts,
    /// the last after it ends.
    pub fn at(&self, t: f64) -> Pose {
        let k = &self.keys;
        let (first, last) = (k[0], k[k.len() - 1]);
        if t <= first.0 {
            return first.1;
        }
        if t >= last.0 {
            return last.1;
        }
        let i = k.partition_point(|&(kt, _)| kt <= t) - 1;
        let ((t0, p0), (t1, p1)) = (k[i], k[i + 1]);
        let h = t1 - t0;
        let s = (t - t0) / h;
        let (m0, m1) = (self.slope(i), self.slope(i + 1));
        // Cubic Hermite: the basis for the ends, then the slopes.
        let s2 = s * s;
        let s3 = s2 * s;
        let (h00, h10, h01, h11) = (
            2.0 * s3 - 3.0 * s2 + 1.0,
            s3 - 2.0 * s2 + s,
            -2.0 * s3 + 3.0 * s2,
            s3 - s2,
        );
        std::array::from_fn(|c| h00 * p0[c] + h10 * h * m0[c] + h01 * p1[c] + h11 * h * m1[c])
    }

    /// The time the path ends.
    pub fn end(&self) -> f64 {
        self.keys[self.keys.len() - 1].0
    }

    /// How fast the camera moves through key `i`, per component: at rest
    /// at the ends and at a hold, otherwise the two segments' speeds
    /// averaged, each weighted by the other's length (so an uneven spacing
    /// of keys doesn't lurch). Where the camera turns back along an axis
    /// it stops on that axis rather than swing past the key.
    fn slope(&self, i: usize) -> Pose {
        let k = &self.keys;
        if i == 0 || i == k.len() - 1 {
            return [0.0; 6];
        }
        let ((ta, pa), (tb, pb), (tc, pc)) = (k[i - 1], k[i], k[i + 1]);
        std::array::from_fn(|c| {
            let before = (pb[c] - pa[c]) / (tb - ta);
            let after = (pc[c] - pb[c]) / (tc - tb);
            if before * after <= 0.0 {
                return 0.0;
            }
            let m = (before * (tc - tb) + after * (tb - ta)) / (tc - ta);
            // No faster than three times the slower segment, or the cubic
            // swings past a key (Fritsch and Carlson).
            let most = 3.0 * before.abs().min(after.abs());
            m.signum() * m.abs().min(most)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "\
        # from the horizon in\n\
        1.0   -300 20 -100   0 30 0\n\
        5.0    -60 40  -20   0 20 0   # over the moat\n";

    fn close(a: Pose, b: Pose) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn a_path_parses_with_comments_and_blank_lines() {
        let p = CameraPath::parse(&format!("\n{TWO}\n")).unwrap();
        assert_eq!(p.keys.len(), 2);
        assert_eq!(p.end(), 5.0);
        assert!(close(p.at(5.0), [-60.0, 40.0, -20.0, 0.0, 20.0, 0.0]));
    }

    #[test]
    fn a_bad_path_says_where() {
        let short = CameraPath::parse("0 1 2 3").unwrap_err();
        assert!(format!("{short:#}").contains("line 1"), "{short:#}");
        let back = CameraPath::parse("2 0 0 0 0 0 0\n1 0 0 0 0 0 0").unwrap_err();
        assert!(format!("{back:#}").contains("isn't after"), "{back:#}");
        assert!(CameraPath::parse("# nothing\n").is_err());
        assert!(CameraPath::parse("0 a 0 0 0 0 0").is_err());
    }

    #[test]
    fn the_camera_holds_before_and_after_the_path() {
        let p = CameraPath::parse(TWO).unwrap();
        assert!(close(p.at(0.0), p.keys[0].1));
        assert!(close(p.at(99.0), p.keys[1].1));
    }

    #[test]
    fn it_passes_through_every_key_and_starts_and_ends_at_rest() {
        let p = CameraPath::parse("0 0 0 0 0 0 0\n2 10 5 0 0 0 0\n3 20 5 5 0 0 0\n6 30 0 0 0 0 0")
            .unwrap();
        for &(t, pose) in &p.keys {
            assert!(close(p.at(t), pose), "at {t}");
        }
        let speed = |t: f64| (p.at(t + 1e-4)[0] - p.at(t)[0]) / 1e-4;
        assert!(speed(0.0).abs() < 0.01);
        assert!(speed(6.0 - 1e-4).abs() < 0.01);
        // And doesn't stop at a key it passes through.
        assert!(speed(2.0) > 1.0);
    }

    #[test]
    fn a_repeated_key_is_a_hold() {
        let p = CameraPath::parse("0 0 0 0 0 0 0\n2 10 0 0 0 0 0\n4 10 0 0 0 0 0\n6 20 0 0 0 0 0")
            .unwrap();
        for t in [2.0, 2.5, 3.0, 3.9, 4.0] {
            assert!((p.at(t)[0] - 10.0).abs() < 1e-9, "at {t}: {}", p.at(t)[0]);
        }
    }

    #[test]
    fn it_never_swings_past_the_keys() {
        // Keys bunched unevenly, which is where a plain Catmull-Rom
        // overshoots.
        let p = CameraPath::parse(
            "0 0 0 0 0 0 0\n0.2 1 0 0 0 0 0\n4 100 0 0 0 0 0\n4.1 100.5 0 0 0 0 0",
        )
        .unwrap();
        let mut last = p.at(0.0)[0];
        for i in 1..=410 {
            let x = p.at(i as f64 * 0.01)[0];
            assert!(x >= last - 1e-9, "goes back at {}", i as f64 * 0.01);
            assert!(x <= 100.5 + 1e-9);
            last = x;
        }
    }
}
