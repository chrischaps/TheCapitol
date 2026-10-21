//! The street's life (E3.14): the arch at the mouth, the strings across the
//! street between stalls that face each other (bunting, lanterns, and
//! washing between the upper floors), and the square's fountain with its
//! lamps.

use super::{
    both, cord, lantern, rnd, tri, Bazaar, Frame, Ground, Lot, AWNINGS, CLOTH, G, GOLD, IRON, PALE_STONE, PAVE,
    PAVING, STONE,
};
use crate::coords::to_world;
use crate::world::palette::{hex, mix};
use crate::world::tower::{hash, lift, loft, prism, V2};
use std::f32::consts::{PI, TAU};

/// The mouth on the avenue: a round arch between two piers with gold
/// finials, an entablature over it with a gold band facing the hall, and a
/// lantern hanging from the keystone. The passage is the lane's own width,
/// less the piers.
pub(super) fn mouth(h: &mut Bazaar, b: &Frame) {
    let (pale, stone) = (hex(PALE_STONE), hex(STONE));
    let half = b.width / 2.0;
    let u = (b.depth * 0.2).min(0.32);
    let (spring, entab) = (G + 2.75, (G + 3.85, G + 4.25));
    let pier = half - 0.25;
    let inner = pier - 0.22;
    let (r0, r1) = (inner, inner + 0.26);
    let deep = 0.3;
    for v in [-pier, pier] {
        prism(&mut h.stone, &b.square(u, v, 0.28, 0.22), PAVE, entab.0, pale, true);
        prism(&mut h.stone, &b.square(u, v, 0.31, 0.25), PAVE, PAVE + 0.3, stone, true);
        prism(&mut h.stone, &b.square(u, v, 0.3, 0.25), spring - 0.12, spring, stone, true);
        loft(&mut h.gold, &b.square(u, v, 0.1, 0.1), entab.1, &b.square(u, v, 0.015, 0.015), entab.1 + 0.4, hex(GOLD), false);
    }
    // The arch's stones, pier to pier, the keystone gold-faced
    let at = |r: f32, t: f32| (r * t.cos(), spring + r * t.sin());
    const STONES: usize = 9;
    for i in 0..STONES {
        let (t0, t1) = (PI * i as f32 / STONES as f32, PI * (i + 1) as f32 / STONES as f32);
        let (a0, a1, c0, c1) = (at(r0, t0), at(r0, t1), at(r1, t0), at(r1, t1));
        let key = i == STONES / 2;
        let colour = if i % 2 == 0 { pale } else { stone };
        for (uu, n) in [(u - deep, b.out()), (u + deep, b.back())] {
            let m = if key && uu < u { &mut h.gold } else { &mut h.stone };
            m.quad([b.p(uu, a0.0, a0.1), b.p(uu, a1.0, a1.1), b.p(uu, c1.0, c1.1), b.p(uu, c0.0, c0.1)], n, if key { hex(GOLD) } else { colour });
        }
        let tm = (t0 + t1) * 0.5;
        let radial = [b.a[0] * tm.cos(), tm.sin(), b.a[1] * tm.cos()];
        for (p, q, n) in [(a0, a1, radial.map(|x| -x)), (c0, c1, radial)] {
            h.stone.quad([b.p(u - deep, p.0, p.1), b.p(u - deep, q.0, q.1), b.p(u + deep, q.0, q.1), b.p(u + deep, p.0, p.1)], n, colour);
        }
    }
    // The spandrels, between the arch's back, the piers and the entablature
    for s in [-1.0f32, 1.0] {
        let from = (inner / r1).clamp(-1.0, 1.0).acos();
        let steps = 6;
        let curve: Vec<(f32, f32)> = (0..=steps)
            .map(|k| {
                let t = from + (PI / 2.0 - from) * k as f32 / steps as f32;
                (s * r1 * t.cos(), spring + r1 * t.sin())
            })
            .collect();
        let corner = (s * inner, entab.0);
        for (uu, n) in [(u - deep, b.out()), (u + deep, b.back())] {
            let c = b.p(uu, corner.0, corner.1);
            for w in curve.windows(2) {
                tri(&mut h.stone, [c, b.p(uu, w[0].0, w[0].1), b.p(uu, w[1].0, w[1].1)], n, pale);
            }
            tri(&mut h.stone, [c, b.p(uu, 0.0, spring + r1), b.p(uu, 0.0, entab.0)], n, pale);
        }
    }
    prism(&mut h.stone, &b.rect(u - deep - 0.03, u + deep + 0.03, -half + 0.02, half - 0.02), entab.0, entab.1, stone, true);
    // The gold band on its face, toward the avenue and the hall, as the
    // hall's rules run round its ticker
    b.face_u(&mut h.gold, u - deep - 0.035, (-half + 0.08, half - 0.08), (entab.0 + 0.14, entab.0 + 0.26), b.out(), hex(GOLD));
    // A threshold of pale stone, and the lantern on a short chain
    prism(&mut h.stone, &b.rect(u - deep, u + deep, -inner, inner), PAVE, PAVE + 0.04, pale, true);
    let c = b.at(u, 0.0);
    let hook = spring + r0 - 0.01;
    prism(&mut h.gold, &[[c[0] - 0.01, c[1] - 0.01], [c[0] + 0.01, c[1] - 0.01], [c[0] + 0.01, c[1] + 0.01], [c[0] - 0.01, c[1] + 0.01]], hook - 0.3, hook, hex(IRON), false);
    lantern(h, c, hook - 0.3, 0.1);
}

/// The street's setts, warm where the Trade District's roads are blue,
/// and the joints between their courses.
const SETTS: u32 = 0x8c7a62;
const JOINTS: u32 = 0x6b5d4c;
/// The setts' level: over the roads (an arterial's is 0.1 over the
/// ground), under the stalls' floors.
const STREET: f32 = PAVE - 0.005;
/// The quarter's ground between the streets: paler setts just over the
/// district's ground, under every road.
const GROUND_SETTS: u32 = 0x9a8c78;
const UNDER: f32 = G + 0.03;

/// The quarter's streets paved in setts, out to the stalls' edges: the
/// lane, the alley, and the market street, the trade ring's quarter from
/// the avenue round to the west arterial. Their courses run across them.
pub(super) fn paving(h: &mut Bazaar, ground: &Ground) {
    let in_quarter = |p: [f32; 2]| p[0] <= 0.5 && p[1] >= -0.5;
    // Under it all, the quarter's ground paved from the moat's bank out
    // past the houses, under the roads, so no blue shows between stalls
    let (r0, r1) = (21.6, 50.5);
    const ARC: usize = 45;
    for i in 0..ARC {
        let (t0, t1) = (PI * (0.5 + 0.5 * i as f32 / ARC as f32), PI * (0.5 + 0.5 * (i + 1) as f32 / ARC as f32));
        let p = |r: f32, t: f32| lift([r * t.cos(), r * t.sin()], UNDER);
        h.stone.quad([p(r0, t0), p(r1, t0), p(r1, t1), p(r0, t1)], [0.0, 1.0, 0.0], hex(GROUND_SETTS));
    }
    // The market street's crown, where the lane and the alley stop their
    // courses so the joints don't cross
    let ring: Vec<([f32; 2], [f32; 2], f32)> = ground
        .roads
        .iter()
        .filter(|r| r.id == "trade_ring")
        .flat_map(|r| r.points.windows(2).map(|s| (to_world(s[0].0, s[0].1), to_world(s[1].0, s[1].1), r.width as f32 * 0.05 + 0.3)).collect::<Vec<_>>())
        .collect();
    for road in ground.roads {
        let whole = matches!(road.id.as_str(), "bazaar_lane" | "bazaar_alley");
        if !whole && road.id != "trade_ring" {
            continue;
        }
        let half = road.width as f32 * 0.05 + 0.28;
        let pts: Vec<[f32; 2]> = road.points.iter().map(|p| to_world(p.0, p.1)).collect();
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if !whole && !(in_quarter(a) && in_quarter(b)) {
                continue;
            }
            let d = [b[0] - a[0], b[1] - a[1]];
            let len = d[0].hypot(d[1]);
            if len < 1e-3 {
                continue;
            }
            let (t, n) = ([d[0] / len, d[1] / len], [-d[1] / len, d[0] / len]);
            let at = |s: f32, c: f32| [a[0] + t[0] * s + n[0] * c, a[1] + t[1] * s + n[1] * c];
            let q = |s0: f32, s1: f32, c0: f32, c1: f32| vec![at(s0, c0), at(s1, c0), at(s1, c1), at(s0, c1)];
            prism(&mut h.stone, &q(0.0, len, -half, half), STREET - 0.01, STREET, hex(SETTS), true);
            // Round each bend, so the next stretch meets it with no gap
            let corner: Vec<[f32; 2]> = (0..8)
                .map(|i| {
                    let a = TAU * (i as f32 + 0.5) / 8.0;
                    [b[0] + half * a.cos(), b[1] + half * a.sin()]
                })
                .collect();
            prism(&mut h.stone, &corner, STREET - 0.01, STREET, hex(SETTS), true);
            let courses = (len / 0.6).floor() as usize;
            for k in 1..=courses {
                let s = len * k as f32 / (courses + 1) as f32;
                if whole && ring.iter().any(|&(p, q, half)| super::segment_distance(at(s, 0.0), p, q) < half) {
                    continue;
                }
                let r = q(s - 0.025, s + 0.025, -half + 0.05, half - 0.05);
                h.stone.quad([r[0], r[1], r[2], r[3]].map(|p| lift(p, STREET + 0.002)), [0.0, 1.0, 0.0], hex(JOINTS));
            }
        }
    }
}

/// What hangs on a string across the street.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hung {
    Bunting,
    Lanterns,
    Washing,
}

/// Strings across the street between stalls that face each other: low
/// ones of bunting or lanterns over the stalls' eaves, and higher ones of
/// washing between the upper floors. The nearest pairs take them first,
/// and a stall ties at most one of each.
pub(super) fn strings(h: &mut Bazaar, lots: &[Lot]) {
    let mut pairs = Vec::new();
    for (i, a) in lots.iter().enumerate() {
        for (j, b) in lots.iter().enumerate().skip(i + 1) {
            let (pa, pb) = (a.frame.at(0.0, 0.0), b.frame.at(0.0, 0.0));
            let d = [pb[0] - pa[0], pb[1] - pa[1]];
            let dist = d[0].hypot(d[1]);
            if !(1.5..=6.5).contains(&dist) {
                continue;
            }
            let (oa, ob) = ([-a.frame.f[0], -a.frame.f[1]], [-b.frame.f[0], -b.frame.f[1]]);
            let facing = oa[0] * ob[0] + oa[1] * ob[1] < -0.75;
            let across = (oa[0] * d[0] + oa[1] * d[1]) / dist > 0.8 && -(ob[0] * d[0] + ob[1] * d[1]) / dist > 0.8;
            if facing && across {
                pairs.push((dist, i, j));
            }
        }
    }
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut tied = vec![[false; 2]; lots.len()];
    for (_, i, j) in pairs {
        let (a, b) = (&lots[i], &lots[j]);
        let seed = hash(a.seed, b.seed, 17);
        for (level, y, hung) in [
            (0, G + 3.05, if rnd(seed, 1) < 0.5 { Hung::Bunting } else { Hung::Lanterns }),
            (1, G + 4.95, Hung::Washing),
        ] {
            if tied[i][level] || tied[j][level] || (level == 1 && rnd(seed, 2) < 0.4) {
                continue;
            }
            let (Some(ta), Some(tb)) = (a.anchor(y), b.anchor(y)) else { continue };
            string(h, a.frame.p(ta.0, ta.1, y), b.frame.p(tb.0, tb.1, y), hung, seed);
            tied[i][level] = true;
            tied[j][level] = true;
        }
    }
}

const WASHING: [u32; 6] = [0xf2efe6, 0xdfe8ef, 0xe8cfc8, 0xe9dcb0, 0xbcd0c0, 0x9fb4cf];

/// One string from `a` to `b`, sagging, with what hangs on it.
fn string(h: &mut Bazaar, a: [f32; 3], b: [f32; 3], hung: Hung, seed: u32) {
    const SEGMENTS: usize = 12;
    let sag = 0.2 + 0.12 * rnd(seed, 3);
    let pts: Vec<[f32; 3]> = (0..=SEGMENTS)
        .map(|k| {
            let t = k as f32 / SEGMENTS as f32;
            let p: [f32; 3] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
            [p[0], p[1] - sag * 4.0 * t * (1.0 - t), p[2]]
        })
        .collect();
    for w in pts.windows(2) {
        cord(&mut h.hung, w[0], w[1]);
    }
    let d = [b[0] - a[0], b[2] - a[2]];
    let len = d[0].hypot(d[1]).max(1e-6);
    let n = [-d[1] / len, 0.0, d[0] / len];
    match hung {
        Hung::Bunting => {
            for (k, w) in pts.windows(2).enumerate() {
                let mid: [f32; 3] = std::array::from_fn(|i| (w[0][i] + w[1][i]) * 0.5);
                let colour = if k % 3 == 2 { hex(CLOTH) } else { hex(AWNINGS[hash(seed, k as u32, 5) as usize % AWNINGS.len()]) };
                let flag = [w[0], w[1], [mid[0], mid[1] - 0.17, mid[2]]];
                tri(&mut h.hung, flag, n, colour);
                tri(&mut h.hung, flag, n.map(|x| -x), colour);
            }
        }
        Hung::Lanterns => {
            for p in pts.iter().skip(2).step_by(3).take(3) {
                lantern(h, [p[0], p[2]], p[1] - 0.01, 0.055);
            }
        }
        Hung::Washing => {
            for (k, w) in pts.windows(2).enumerate().skip(1).take(SEGMENTS - 2) {
                if rnd(seed, 30 + k as u32) < 0.35 {
                    continue;
                }
                let drop = 0.28 + 0.3 * rnd(seed, 50 + k as u32);
                let colour = hex(WASHING[hash(seed, k as u32, 8) as usize % WASHING.len()]);
                let low = |p: [f32; 3]| [p[0], p[1] - drop, p[2]];
                let inset = |p: [f32; 3], q: [f32; 3]| std::array::from_fn(|i| p[i] + (q[i] - p[i]) * 0.08);
                let (p, q) = (inset(w[0], w[1]), inset(w[1], w[0]));
                both(&mut h.hung, [p, q, low(q), low(p)], n, colour);
            }
        }
    }
}

/// The eight corners of something round of radius `r` at world `c`.
fn octagon(c: V2, r: f32) -> Vec<V2> {
    (0..8)
        .map(|i| {
            let t = TAU * (i as f32 + 0.5) / 8.0;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

/// The square at the alley's end: paved, with a fountain in its middle (a
/// basin, a column and a bowl, a gold finial) and a lamp at each corner.
pub(super) fn square(h: &mut Bazaar, f: &Frame) {
    let (w, d) = (f.width / 2.0, f.depth);
    prism(&mut h.stone, &f.rect(0.0, d, -w, w), G - 0.03, PAVE - 0.02, mix(hex(PAVING), hex(PALE_STONE), 0.35), true);
    let c = f.at(d * 0.5, 0.0);
    prism(&mut h.stone, &octagon(c, 1.6), PAVE - 0.02, PAVE - 0.01, hex(PAVING), true);
    let pale = hex(PALE_STONE);
    // The basin: eight blocks round the water
    let (ri, ro) = (1.0, 1.16);
    for i in 0..8 {
        let (t0, t1) = (TAU * i as f32 / 8.0, TAU * (i + 1) as f32 / 8.0);
        let p = |r: f32, t: f32| [c[0] + r * t.cos(), c[1] + r * t.sin()];
        prism(&mut h.stone, &[p(ri, t0), p(ro, t0), p(ro, t1), p(ri, t1)], PAVE - 0.01, PAVE + 0.45, pale, true);
    }
    let water = hex(0x3d7590);
    prism(&mut h.stone, &octagon(c, ri), PAVE + 0.3, PAVE + 0.33, water, true);
    prism(&mut h.stone, &octagon(c, 0.14), PAVE + 0.33, PAVE + 1.2, hex(STONE), false);
    loft(&mut h.stone, &octagon(c, 0.12), PAVE + 1.15, &octagon(c, 0.5), PAVE + 1.35, pale, false);
    prism(&mut h.stone, &octagon(c, 0.46), PAVE + 1.31, PAVE + 1.33, water, true);
    prism(&mut h.stone, &octagon(c, 0.06), PAVE + 1.33, PAVE + 1.7, hex(STONE), true);
    loft(&mut h.gold, &octagon(c, 0.09), PAVE + 1.7, &octagon(c, 0.01), PAVE + 1.95, hex(GOLD), false);
    // A lamp at each corner, on an iron post
    for (u, v) in [(0.5, -w + 0.5), (0.5, w - 0.5), (d - 0.5, -w + 0.5), (d - 0.5, w - 0.5)] {
        let p = f.at(u, v);
        prism(&mut h.gold, &octagon(p, 0.04), PAVE - 0.01, G + 2.4, hex(IRON), true);
        lantern(h, p, G + 2.6, 0.09);
        prism(&mut h.gold, &octagon(p, 0.012), G + 2.4, G + 2.6, hex(IRON), false);
    }
}
