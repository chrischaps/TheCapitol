//! Plane geometry in server space, shared by the world's builders.

use capitol_protocol::rules::{CENTER_X, CENTER_Y};

/// A point in server space.
pub type P = [f64; 2];

pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
pub fn scale(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k]
}
pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
pub fn unit(a: P) -> P {
    scale(a, 1.0 / a[0].hypot(a[1]))
}
/// `d` turned a quarter anticlockwise (in maths axes): its left.
pub fn left(d: P) -> P {
    [-d[1], d[0]]
}
pub fn rot(a: P, radians: f64) -> P {
    let (s, c) = radians.sin_cos();
    [a[0] * c - a[1] * s, a[0] * s + a[1] * c]
}
/// `t` of the way from `a` to `b`, landing exactly on each end.
pub fn lerp(a: P, b: P, t: f64) -> P {
    if t == 1.0 {
        b
    } else {
        add(a, scale(sub(b, a), t))
    }
}
pub fn radius(p: P) -> f64 {
    (p[0] - CENTER_X).hypot(p[1] - CENTER_Y)
}

/// Server distance from `p` to the segment `a`→`b`.
pub fn distance_to_segment(p: P, a: P, b: P) -> f64 {
    let ab = sub(b, a);
    let t = (dot(sub(p, a), ab) / dot(ab, ab).max(1e-12)).clamp(0.0, 1.0);
    dist(p, lerp(a, b, t))
}
