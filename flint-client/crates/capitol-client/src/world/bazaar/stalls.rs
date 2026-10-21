//! What stands on a stall's footprint (E3.14): one of the five [`Kind`]s,
//! kept (open, in its shop's colour, stocked) or empty (shuttered, its
//! cloth furled), and the crates, sacks, barrels and baskets that spill
//! forward to its street's edge.

use super::{
    both, cord, lantern, post, rnd, sign, sloped, tri, Bazaar, Frame, House, Kind, Lot, Shop, CANVAS, CLOTH,
    COUNTER, G, IRON, PALE_STONE, PAVE, PAVING, TIMBER, TIMBER_DARK,
};
use crate::world::mesh::MeshBuf;
use crate::world::palette::{hex, mix};
use crate::world::tower::{loft, prism, V2};

/// Builds a stall's front, by its kind, and returns where its name hangs,
/// and how wide its board is: its sign out over the street when kept, or
/// over its counter, with no board, when not.
pub(super) fn front(h: &mut Bazaar, lot: &Lot, shop: Option<&Shop>) -> ([f32; 3], f32) {
    let f = &lot.frame;
    let half = lot.half();
    prism(&mut h.stone, &f.rect(0.0, f.depth, -half - 0.03, half + 0.03), G - 0.02, PAVE, hex(PAVING), true);
    let sign = match (lot.kind, lot.house) {
        (Kind::Shopfront, Some(house)) => shopfront(h, lot, &house, shop),
        (Kind::Open | Kind::Shopfront, _) => open(h, lot, shop),
        (Kind::Tent, _) => tent(h, lot, shop),
        (Kind::Cart, _) => cart(h, lot, shop),
        (Kind::Kiosk, _) => kiosk(h, lot, shop),
    };
    sign.unwrap_or_else(|| (f.p(0.27, 0.0, COUNTER + 0.5), 0.0))
}

/// A counter across the front, from `v0` to `v1`.
fn counter(m: &mut MeshBuf, f: &Frame, (v0, v1): (f32, f32)) {
    prism(m, &f.rect(0.08, 0.46, v0, v1), PAVE, COUNTER, hex(TIMBER_DARK), true);
    prism(m, &f.rect(0.05, 0.49, v0 - 0.02, v1 + 0.02), COUNTER, COUNTER + 0.04, hex(TIMBER), true);
}

/// Goods on a counter from `v0` to `v1`: bundles of differing heights in
/// the shop's colours.
fn on_counter(m: &mut MeshBuf, f: &Frame, (v0, v1): (f32, f32), shop: &Shop, seed: u32) {
    let n = (((v1 - v0) / 0.42).floor() as usize).clamp(2, 4);
    let w = (v1 - v0) / n as f32;
    for j in 0..n {
        let a = v0 + w * j as f32;
        let tall = 0.08 + 0.12 * rnd(seed, 20 + j as u32);
        prism(m, &f.rect(0.12, 0.4, a + 0.03, a + w - 0.03), COUNTER + 0.04, COUNTER + 0.04 + tall, shop.good(j as u32), true);
    }
}

/// A shelf across the back at `u`, stocked when kept.
fn shelf(m: &mut MeshBuf, f: &Frame, u: f32, (v0, v1): (f32, f32), shop: Option<&Shop>, seed: u32) {
    for (k, y) in [PAVE + 0.95, PAVE + 1.45].into_iter().enumerate() {
        prism(m, &f.rect(u - 0.3, u, v0, v1), y, y + 0.04, hex(TIMBER), true);
        let Some(shop) = shop else { continue };
        let n = (((v1 - v0) / 0.3).floor() as usize).max(1);
        let w = (v1 - v0) / n as f32;
        for j in 0..n {
            if rnd(seed, 40 + (k * 8 + j) as u32) < 0.25 {
                continue;
            }
            let a = v0 + w * j as f32;
            let tall = 0.1 + 0.18 * rnd(seed, 60 + (k * 8 + j) as u32);
            prism(m, &f.rect(u - 0.26, u - 0.04, a + 0.03, a + w - 0.03), y + 0.04, y + 0.04 + tall, shop.good((k * 3 + j) as u32), true);
        }
    }
}

/// Goods hanging on cords from an eave at `y`, along `u`: bunches,
/// coils and pots in the shop's colours.
fn hanging(m: &mut MeshBuf, f: &Frame, u: f32, y: f32, (v0, v1): (f32, f32), shop: &Shop, seed: u32) {
    let n = (((v1 - v0) / 0.32).floor() as usize).clamp(1, 5);
    for j in 0..n {
        if rnd(seed, 80 + j as u32) < 0.3 {
            continue;
        }
        let v = v0 + (v1 - v0) * (j as f32 + 0.5) / n as f32;
        let drop = 0.12 + 0.14 * rnd(seed, 90 + j as u32);
        let bottom = (y - drop - 0.2).max(super::HEADROOM + 0.05);
        let c = f.at(u, v);
        cord(m, [c[0], y, c[1]], [c[0], bottom + 0.2, c[1]]);
        loft(m, &f.square(u, v, 0.05, 0.05), bottom, &f.square(u, v, 0.03, 0.03), bottom + 0.2, shop.good(j as u32 + 5), true);
    }
}

/// An open stall: timber posts, a counter, a shelf, and a pitched cloth
/// roof striped in its shop's colour; empty, its front pitch furled and its
/// counter boarded.
fn open(h: &mut Bazaar, lot: &Lot, shop: Option<&Shop>) -> Option<([f32; 3], f32)> {
    let f = &lot.frame;
    let half = lot.half();
    let d = f.depth;
    let (front_y, ridge_y, back_y) = (G + 2.2, G + 2.75, G + 2.35);
    let ridge_u = d * 0.45;
    let edge = -0.35;
    let timber = hex(TIMBER);
    for v in [-half + 0.07, half - 0.07] {
        post(&mut h.stone, f, (0.08, v), 0.05, (PAVE, front_y), timber);
        post(&mut h.stone, f, (d - 0.1, v), 0.05, (PAVE, back_y), timber);
    }
    prism(&mut h.stone, &f.rect(ridge_u - 0.04, ridge_u + 0.04, -half, half), ridge_y - 0.1, ridge_y - 0.02, hex(TIMBER_DARK), true);
    counter(&mut h.stone, f, (-half + 0.1, half - 0.1));
    shelf(&mut h.stone, f, d - 0.15, (-half + 0.15, half - 0.15), shop, lot.seed);
    // The cloth: stripes across the street, both pitches
    let colour = shop.map_or(hex(CANVAS), |s| s.colour);
    let stripes = ((2.0 * half / 0.3).round() as usize).max(3);
    let edge_y = front_y - 0.1;
    for j in 0..stripes {
        let a = -half + 2.0 * half * j as f32 / stripes as f32;
        let b = -half + 2.0 * half * (j + 1) as f32 / stripes as f32;
        let c = if shop.is_some() && j % 2 == 1 { hex(CLOTH) } else { colour };
        sloped(&mut h.stone, [f.p(ridge_u, a, ridge_y), f.p(d, a, back_y), f.p(d, b, back_y), f.p(ridge_u, b, ridge_y)], c);
        if shop.is_some() {
            sloped(&mut h.stone, [f.p(ridge_u, a, ridge_y), f.p(edge, a, edge_y), f.p(edge, b, edge_y), f.p(ridge_u, b, ridge_y)], c);
            both(&mut h.stone, [f.p(edge, a, edge_y), f.p(edge, b, edge_y), f.p(edge, b, edge_y - 0.14), f.p(edge, a, edge_y - 0.14)], f.out(), c);
        }
    }
    let Some(shop) = shop else {
        // Furled at the ridge, and the counter boarded over
        prism(&mut h.stone, &f.rect(ridge_u - 0.2, ridge_u - 0.06, -half + 0.05, half - 0.05), ridge_y - 0.22, ridge_y - 0.08, mix(hex(CANVAS), hex(PALE_STONE), 0.3), true);
        prism(&mut h.stone, &f.rect(0.04, 0.5, -half + 0.08, half - 0.08), COUNTER + 0.04, COUNTER + 0.07, hex(TIMBER_DARK), true);
        return None;
    };
    on_counter(&mut h.stone, f, (-half + 0.12, half - 0.12), shop, lot.seed);
    let at = |u: f32| ridge_y + (edge_y - ridge_y) * (ridge_u - u) / (ridge_u - edge);
    hanging(&mut h.stone, f, -0.12, at(-0.12), (-half + 0.2, half - 0.2), shop, lot.seed);
    let side = if lot.seed.is_multiple_of(2) { -half + 0.12 } else { half - 0.12 };
    lantern(h, f.at(-0.2, side), at(-0.2), 0.07);
    Some(sign(h, f, -0.3, at(-0.3), shop.colour))
}

/// A ridge tent from its front pole to its back, striped in its shop's
/// colour, with a pennant on the pole; kept, its flaps are tied back over a
/// rug and a low table of goods; empty, they're closed.
fn tent(h: &mut Bazaar, lot: &Lot, shop: Option<&Shop>) -> Option<([f32; 3], f32)> {
    let f = &lot.frame;
    let half = lot.half() - 0.05;
    let (u0, u1) = (0.12, f.depth - 0.08);
    let (eave_y, ridge_y) = (G + 1.45, G + 2.5);
    let colour = shop.map_or(hex(CANVAS), |s| s.colour);
    let stripes = ((u1 - u0) / 0.32).round().max(3.0) as usize;
    for s in [-half, half] {
        for j in 0..stripes {
            let a = u0 + (u1 - u0) * j as f32 / stripes as f32;
            let b = u0 + (u1 - u0) * (j + 1) as f32 / stripes as f32;
            let c = if j % 2 == 1 { hex(CLOTH) } else { colour };
            sloped(&mut h.stone, [f.p(a, s, eave_y), f.p(a, 0.0, ridge_y), f.p(b, 0.0, ridge_y), f.p(b, s, eave_y)], c);
            both(&mut h.stone, [f.p(a, s, PAVE), f.p(b, s, PAVE), f.p(b, s, eave_y), f.p(a, s, eave_y)], f.side(s.signum()), c);
        }
    }
    // Its back, closed
    let canvas = mix(colour, hex(CANVAS), 0.5);
    tri(&mut h.stone, [f.p(u1, -half, eave_y), f.p(u1, half, eave_y), f.p(u1, 0.0, ridge_y)], f.back(), canvas);
    tri(&mut h.stone, [f.p(u1, -half, eave_y), f.p(u1, half, eave_y), f.p(u1, 0.0, ridge_y)], f.out(), canvas);
    both(&mut h.stone, [f.p(u1, -half, PAVE), f.p(u1, half, PAVE), f.p(u1, half, eave_y), f.p(u1, -half, eave_y)], f.back(), canvas);
    // The poles, and a pennant at the front one's top
    for u in [u0, u1] {
        post(&mut h.stone, f, (u, 0.0), 0.03, (PAVE, ridge_y + 0.25), hex(TIMBER_DARK));
    }
    let top = ridge_y + 0.25;
    loft(&mut h.gold, &f.square(u0, 0.0, 0.04, 0.04), top, &f.square(u0, 0.0, 0.005, 0.005), top + 0.12, hex(super::GOLD), false);
    let pennant = [f.p(u0, 0.0, top - 0.02), f.p(u0, 0.0, top - 0.2), f.p(u0 + 0.38, 0.0, top - 0.13)];
    tri(&mut h.stone, pennant, f.side(1.0), colour);
    tri(&mut h.stone, pennant, f.side(-1.0), colour);
    let Some(shop) = shop else {
        // Closed: the front gable and its walls, with a seam down the middle
        tri(&mut h.stone, [f.p(u0, -half, eave_y), f.p(u0, half, eave_y), f.p(u0, 0.0, ridge_y)], f.out(), canvas);
        tri(&mut h.stone, [f.p(u0, -half, eave_y), f.p(u0, half, eave_y), f.p(u0, 0.0, ridge_y)], f.back(), canvas);
        both(&mut h.stone, [f.p(u0, -half, PAVE), f.p(u0, half, PAVE), f.p(u0, half, eave_y), f.p(u0, -half, eave_y)], f.out(), canvas);
        f.face_u(&mut h.stone, u0 - 0.006, (-0.015, 0.015), (PAVE, ridge_y - 0.05), f.out(), hex(super::CORD));
        return None;
    };
    // The flaps tied back at each side
    for s in [-half, half] {
        let flap = [f.p(u0 - 0.02, s, eave_y + 0.05), f.p(u0 - 0.02, s * 0.55, eave_y + (ridge_y - eave_y) * 0.45), f.p(u0 - 0.02, s, PAVE + 0.45)];
        tri(&mut h.stone, flap, f.out(), shop.colour);
        tri(&mut h.stone, flap, f.back(), mix(shop.colour, hex(CANVAS), 0.4));
    }
    // A rug, and a low table of goods on it
    let rug = mix(shop.colour, [0.1, 0.06, 0.04, 1.0], 0.45);
    prism(&mut h.stone, &f.rect(u0 + 0.1, u1 - 0.1, -half + 0.1, half - 0.1), PAVE, PAVE + 0.01, rug, true);
    prism(&mut h.stone, &f.rect(0.35, 0.85, -half + 0.2, half - 0.2), PAVE, PAVE + 0.45, hex(TIMBER_DARK), true);
    let n = 3;
    let w = (2.0 * half - 0.4) / n as f32;
    for j in 0..n {
        let a = -half + 0.2 + w * j as f32;
        let tall = 0.08 + 0.14 * rnd(lot.seed, 30 + j as u32);
        prism(&mut h.stone, &f.rect(0.4, 0.8, a + 0.03, a + w - 0.03), PAVE + 0.45, PAVE + 0.45 + tall, shop.good(j as u32), true);
    }
    // An iron arm out from the pole for the sign, and a lantern on it
    let arm = eave_y + 0.45;
    prism(&mut h.gold, &f.rect(-0.36, u0, -0.012, 0.012), arm - 0.024, arm, hex(IRON), true);
    lantern(h, f.at(-0.05, 0.0), arm - 0.03, 0.06);
    Some(sign(h, f, -0.3, arm - 0.02, shop.colour))
}

/// A beam of square section, `t` across, from `a` to `b`.
fn rod(m: &mut MeshBuf, a: [f32; 3], b: [f32; 3], t: f32, colour: [f32; 4]) {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let d = d.map(|x| x / len);
    // Across it, level; and its up
    let flat = d[0].hypot(d[2]).max(1e-6);
    let side = [-d[2] / flat, 0.0, d[0] / flat];
    let up = [side[1] * d[2] - side[2] * d[1], side[2] * d[0] - side[0] * d[2], side[0] * d[1] - side[1] * d[0]];
    let off = |p: [f32; 3], s: f32, u: f32| std::array::from_fn(|i| p[i] + side[i] * s * t * 0.5 + up[i] * u * t * 0.5);
    for (s, u, n) in [(1.0, 0.0, side), (-1.0, 0.0, side.map(|x| -x)), (0.0, 1.0, up), (0.0, -1.0, up.map(|x| -x))] {
        let (p, q) = if s == 0.0 { ((1.0, u), (-1.0, u)) } else { ((s, 1.0), (s, -1.0)) };
        m.quad([off(a, p.0, p.1), off(b, p.0, p.1), off(b, q.0, q.1), off(a, q.0, q.1)], n, colour);
    }
}

/// A wheel on an axle along `v`: a disc of radius `r`, `thick` wide,
/// centred at (u, v) and height `y`.
fn wheel(m: &mut MeshBuf, f: &Frame, (u, v, y): (f32, f32, f32), r: f32, thick: f32, colour: [f32; 4]) {
    const SIDES: usize = 10;
    let rim: Vec<(f32, f32)> = (0..=SIDES)
        .map(|i| {
            let t = std::f32::consts::TAU * i as f32 / SIDES as f32;
            (u + r * t.cos(), y + r * t.sin())
        })
        .collect();
    for s in [-1.0, 1.0] {
        let vv = v + s * thick * 0.5;
        let centre = f.p(u, vv, y);
        for w in rim.windows(2) {
            tri(m, [centre, f.p(w[0].0, vv, w[0].1), f.p(w[1].0, vv, w[1].1)], f.side(s), colour);
        }
    }
    for w in rim.windows(2) {
        let mid = ((w[0].0 + w[1].0) * 0.5 - u, (w[0].1 + w[1].1) * 0.5 - y);
        let len = mid.0.hypot(mid.1).max(1e-6);
        let n = [f.f[0] * mid.0 / len, mid.1 / len, f.f[1] * mid.0 / len];
        let (h0, h1) = (v - thick * 0.5, v + thick * 0.5);
        m.quad([f.p(w[0].0, h0, w[0].1), f.p(w[1].0, h0, w[1].1), f.p(w[1].0, h1, w[1].1), f.p(w[0].0, h1, w[0].1)], n, mix(colour, [0.0, 0.0, 0.0, 1.0], 0.3));
    }
}

/// A handcart on two wheels, its shafts down to the street; kept, heaped
/// with goods under a striped canopy; empty, a tarp over its bed. Barrels
/// and a crate stand behind it.
fn cart(h: &mut Bazaar, lot: &Lot, shop: Option<&Shop>) -> Option<([f32; 3], f32)> {
    let f = &lot.frame;
    let half = lot.half();
    let bw = (half - 0.12).min(0.55);
    let (b0, b1) = (0.25, 1.45);
    let (bed, rim) = (G + 0.66, G + 0.92);
    let timber = hex(TIMBER);
    let dark = hex(TIMBER_DARK);
    prism(&mut h.stone, &f.rect(b0, b1, -bw, bw), bed - 0.06, bed, dark, true);
    for r in [f.rect(b0, b1, -bw, -bw + 0.04), f.rect(b0, b1, bw - 0.04, bw), f.rect(b0, b0 + 0.04, -bw, bw), f.rect(b1 - 0.04, b1, -bw, bw)] {
        prism(&mut h.stone, &r, bed, rim, timber, true);
    }
    for s in [-1.0, 1.0] {
        wheel(&mut h.stone, f, (0.85, s * (bw + 0.05), PAVE + 0.36), 0.36, 0.05, dark);
        rod(&mut h.stone, f.p(b0 + 0.05, s * (bw - 0.1), bed - 0.03), f.p(-0.22, s * (bw - 0.1), PAVE + 0.22), 0.045, timber);
    }
    post(&mut h.stone, f, (b0 + 0.1, 0.0), 0.025, (PAVE, bed - 0.06), dark);
    prism(&mut h.gold, &f.rect(0.82, 0.88, -bw - 0.1, bw + 0.1), PAVE + 0.33, PAVE + 0.39, hex(IRON), true);
    // Behind the cart, barrels and a crate
    let mut u = b1 + 0.35;
    let mut k = 0;
    while u + 0.25 < f.depth - 0.05 && k < 3 {
        let v = if k % 2 == 0 { -half * 0.45 } else { half * 0.4 };
        let goods = shop.map(|s| s.good(k + 7));
        if k == 1 {
            crate_(&mut h.stone, f, (u, v), 0.22, goods);
        } else {
            barrel(&mut h.stone, f, (u, v), 0.2, goods);
        }
        u += 0.5;
        k += 1;
    }
    let Some(shop) = shop else {
        loft(&mut h.stone, &f.rect(b0 - 0.02, b1 + 0.02, -bw - 0.02, bw + 0.02), rim, &f.rect(b0 + 0.15, b1 - 0.15, -bw + 0.12, bw - 0.12), rim + 0.22, hex(CANVAS), true);
        return None;
    };
    // Heaped goods
    for j in 0..3 {
        let (a, b) = (b0 + 0.05 + 0.38 * j as f32, b0 + 0.05 + 0.38 * (j + 1) as f32);
        let tall = 0.18 + 0.15 * rnd(lot.seed, 50 + j as u32);
        loft(&mut h.stone, &f.rect(a, b, -bw + 0.05, bw - 0.05), bed, &f.rect(a + 0.08, b - 0.08, -bw + 0.18, bw - 0.18), rim + tall, shop.good(j as u32), true);
    }
    // The canopy on four poles, lower at the front
    let (c0, c1) = (-0.22, b1 + 0.1);
    let (y0, y1) = (G + 2.0, G + 2.2);
    for (u, y) in [(b0 + 0.02, y0 + 0.02), (b1 - 0.02, y1)] {
        for v in [-bw + 0.02, bw - 0.02] {
            post(&mut h.stone, f, (u, v), 0.02, (rim, y), dark);
        }
    }
    let stripes = ((2.0 * bw / 0.26).round() as usize).max(3);
    let at = |u: f32| y0 + (y1 - y0) * (u - c0) / (c1 - c0);
    for j in 0..stripes {
        let a = -bw - 0.08 + (2.0 * bw + 0.16) * j as f32 / stripes as f32;
        let b = -bw - 0.08 + (2.0 * bw + 0.16) * (j + 1) as f32 / stripes as f32;
        let c = if j % 2 == 1 { hex(CLOTH) } else { shop.colour };
        sloped(&mut h.stone, [f.p(c0, a, y0), f.p(c1, a, y1), f.p(c1, b, y1), f.p(c0, b, y0)], c);
        both(&mut h.stone, [f.p(c0, a, y0), f.p(c0, b, y0), f.p(c0, b, y0 - 0.12), f.p(c0, a, y0 - 0.12)], f.out(), c);
    }
    hanging(&mut h.stone, f, 0.05, at(0.05), (-bw + 0.1, bw - 0.1), shop, lot.seed);
    lantern(h, f.at(b0 + 0.02, bw - 0.02), at(b0) - 0.02, 0.06);
    Some(sign(h, f, -0.17, at(-0.17), shop.colour))
}

/// The six corners of a booth of radius `r` round (u, 0), one pointing to
/// the street.
fn hexagon(f: &Frame, u: f32, r: f32) -> Vec<V2> {
    (0..6)
        .map(|i| {
            let t = std::f32::consts::PI + std::f32::consts::TAU * i as f32 / 6.0;
            f.at(u + r * t.cos(), r * t.sin())
        })
        .collect()
}

/// A six-sided kiosk under a cone, its counter round it; kept, open on
/// every side with goods on the counter and its frieze in the shop's
/// colour; empty, shuttered.
fn kiosk(h: &mut Bazaar, lot: &Lot, shop: Option<&Shop>) -> Option<([f32; 3], f32)> {
    let f = &lot.frame;
    let half = lot.half();
    let r = (f.depth.min(2.0 * half) * 0.42).min(1.15);
    let c = f.depth * 0.5;
    let dark = hex(TIMBER_DARK);
    prism(&mut h.stone, &hexagon(f, c, r), PAVE, COUNTER, dark, true);
    prism(&mut h.stone, &hexagon(f, c, r * 1.1), COUNTER, COUNTER + 0.05, hex(TIMBER), true);
    let (band, eave, apex) = (G + 2.05, G + 2.25, G + 3.0);
    for p in hexagon(f, c, r * 0.97) {
        prism(&mut h.stone, &[[p[0] - 0.025, p[1] - 0.025], [p[0] + 0.025, p[1] - 0.025], [p[0] + 0.025, p[1] + 0.025], [p[0] - 0.025, p[1] + 0.025]], COUNTER + 0.05, band, dark, true);
    }
    let colour = shop.map_or(hex(PALE_STONE), |s| s.colour);
    if shop.is_none() {
        prism(&mut h.stone, &hexagon(f, c, r * 0.95), COUNTER + 0.05, band, mix(dark, hex(TIMBER), 0.3), true);
    }
    prism(&mut h.stone, &hexagon(f, c, r * 1.02), band, eave, colour, false);
    // The cone, its panels alternating, and under its eave
    let rim = hexagon(f, c, r * 1.32);
    let top = f.p(c, 0.0, apex);
    for i in 0..6 {
        let (a, b) = (rim[i], rim[(i + 1) % 6]);
        let (a, b) = (super::lift(a, eave), super::lift(b, eave));
        let n = super::normal_of(a, b, top);
        let n = if n[1] < 0.0 { n.map(|x| -x) } else { n };
        tri(&mut h.stone, [a, b, top], n, if i % 2 == 0 { colour } else { hex(CLOTH) });
        tri(&mut h.stone, [a, b, f.p(c, 0.0, eave)], [0.0, -1.0, 0.0], mix(dark, [0.0, 0.0, 0.0, 1.0], 0.3));
    }
    loft(&mut h.gold, &f.square(c, 0.0, 0.06, 0.06), apex - 0.03, &f.square(c, 0.0, 0.006, 0.006), apex + 0.2, hex(super::GOLD), false);
    let shop = shop?;
    for j in 0..4 {
        let a = -r * 0.55 + r * 0.55 * j as f32;
        let tall = 0.08 + 0.14 * rnd(lot.seed, 70 + j);
        let u = c - r * 0.62 + 0.04 * (j % 2) as f32;
        prism(&mut h.stone, &f.square(u, a * 0.6, 0.09, 0.12), COUNTER + 0.05, COUNTER + 0.05 + tall, shop.good(j), true);
    }
    // An iron bracket out from the front post, with the sign and a lantern
    let front = c - r * 0.97;
    let arm = band - 0.05;
    prism(&mut h.gold, &f.rect(-0.36, front, -0.012, 0.012), arm - 0.024, arm, hex(IRON), true);
    lantern(h, f.at((front - 0.36) * 0.5 + 0.12, 0.0), arm - 0.03, 0.06);
    Some(sign(h, f, -0.3, arm - 0.02, shop.colour))
}

/// A shopfront: the ground floor of its house, open to the street between
/// two piers, a counter across it and shelves behind; kept, its top
/// shutter propped up over the street as an awning in the shop's colour;
/// empty, shuttered.
fn shopfront(h: &mut Bazaar, lot: &Lot, house: &House, shop: Option<&Shop>) -> Option<([f32; 3], f32)> {
    let f = &lot.frame;
    let (_, v0, v1) = house.floor(0, lot.half());
    let y1 = House::floor_y(1);
    let lintel = y1 - 0.35;
    let ub = house.u1;
    let wall = hex(house.wall);
    let inside = mix(wall, [0.1, 0.08, 0.06, 1.0], 0.45);
    prism(&mut h.stone, &f.rect(0.0, ub, v0, v0 + 0.16), PAVE, y1, wall, true);
    prism(&mut h.stone, &f.rect(0.0, ub, v1 - 0.16, v1), PAVE, y1, wall, true);
    prism(&mut h.stone, &f.rect(ub - 0.15, ub, v0, v1), PAVE, y1, wall, true);
    prism(&mut h.stone, &f.rect(0.0, 0.2, v0, v1), lintel, y1, wall, true);
    // Its ceiling, and the walls inside darker, so the shop reads deep
    let ceiling = [f.p(0.0, v0 + 0.16, lintel), f.p(ub - 0.15, v0 + 0.16, lintel), f.p(ub - 0.15, v1 - 0.16, lintel), f.p(0.0, v1 - 0.16, lintel)];
    h.stone.quad(ceiling, [0.0, -1.0, 0.0], hex(TIMBER_DARK));
    f.face_u(&mut h.stone, ub - 0.155, (v0 + 0.16, v1 - 0.16), (PAVE, lintel), f.out(), inside);
    // A beam over the opening, and the pier's quoins
    f.face_u(&mut h.stone, -0.004, (v0, v1), (lintel, lintel + 0.12), f.out(), hex(TIMBER_DARK));
    counter(&mut h.stone, f, (v0 + 0.22, v1 - 0.22));
    shelf(&mut h.stone, f, ub - 0.16, (v0 + 0.2, v1 - 0.2), shop, lot.seed);
    let Some(shop) = shop else {
        // Shuttered: boards across the opening, battens and an iron bar
        let shut = (v0 + 0.16, v1 - 0.16);
        f.face_u(&mut h.stone, -0.006, shut, (PAVE, lintel), f.out(), hex(TIMBER_DARK));
        for y in [PAVE + 0.3, (PAVE + lintel) * 0.5, lintel - 0.3] {
            f.face_u(&mut h.stone, -0.012, shut, (y - 0.04, y + 0.04), f.out(), hex(TIMBER));
        }
        f.face_u(&mut h.gold, -0.018, (shut.0 + 0.05, shut.1 - 0.05), (COUNTER - 0.03, COUNTER + 0.03), f.out(), hex(IRON));
        return None;
    };
    on_counter(&mut h.stone, f, (v0 + 0.25, v1 - 0.25), shop, lot.seed);
    // The top shutter, propped up and out as an awning: timber on top of
    // the cloth, on two iron stays
    let (out_u, out_y, in_y) = (-0.62, G + 2.02, lintel - 0.02);
    let stripes = (((v1 - v0) / 0.28).round() as usize).max(3);
    for j in 0..stripes {
        let a = v0 + 0.05 + (v1 - v0 - 0.1) * j as f32 / stripes as f32;
        let b = v0 + 0.05 + (v1 - v0 - 0.1) * (j + 1) as f32 / stripes as f32;
        let c = if j % 2 == 1 { hex(CLOTH) } else { shop.colour };
        sloped(&mut h.stone, [f.p(-0.01, a, in_y), f.p(out_u, a, out_y), f.p(out_u, b, out_y), f.p(-0.01, b, in_y)], c);
        both(&mut h.stone, [f.p(out_u, a, out_y), f.p(out_u, b, out_y), f.p(out_u, b, out_y - 0.12), f.p(out_u, a, out_y - 0.12)], f.out(), c);
    }
    for v in [v0 + 0.1, v1 - 0.1] {
        rod(&mut h.gold, f.p(-0.01, v, lintel - 0.45), f.p(out_u + 0.05, v, out_y + 0.01), 0.02, hex(IRON));
    }
    let at = |u: f32| in_y + (out_y - in_y) * (u + 0.01) / (out_u + 0.01);
    hanging(&mut h.stone, f, -0.25, at(-0.25), (v0 + 0.25, v1 - 0.25), shop, lot.seed);
    lantern(h, f.at(-0.1, if lot.seed.is_multiple_of(2) { v0 + 0.08 } else { v1 - 0.08 }), lintel + 0.3, 0.07);
    Some(sign(h, f, -0.55, at(-0.55), shop.colour))
}

/// Crates, sacks, barrels and baskets spilling forward to the street's
/// edge, clear of the middle where a buyer stands: a kept stall's full of
/// its goods, an empty one's a stray barrel.
pub(super) fn spill(h: &mut Bazaar, lot: &Lot, shop: Option<&Shop>) {
    let f = &lot.frame;
    let half = lot.half();
    let seed = lot.seed;
    let Some(shop) = shop else {
        if rnd(seed, 100) < 0.5 {
            let v = if rnd(seed, 101) < 0.5 { -half + 0.2 } else { half - 0.2 };
            barrel(&mut h.stone, f, (0.2, v), 0.17, None);
        }
        return;
    };
    let n = 2 + (rnd(seed, 102) * 2.0) as usize;
    for k in 0..n {
        let side = if k % 2 == 0 { -1.0 } else { 1.0 };
        let v = side * (half - 0.16 - 0.3 * (k / 2) as f32);
        if v.abs() < half * 0.35 {
            continue;
        }
        let at = (-0.14, v);
        let goods = Some(shop.good(k as u32 + 3));
        match hash_kind(seed, k) {
            0 => crate_(&mut h.stone, f, at, 0.2, goods),
            1 => sack(&mut h.stone, f, at, 0.11, goods),
            2 => barrel(&mut h.stone, f, at, 0.12, goods),
            _ => basket(&mut h.stone, f, at, 0.12, goods),
        }
    }
}

fn hash_kind(seed: u32, k: usize) -> u32 {
    crate::world::tower::hash(seed, 200 + k as u32, 9) % 4
}

/// A crate `s` across, open, with goods in it.
fn crate_(m: &mut MeshBuf, f: &Frame, (u, v): (f32, f32), s: f32, goods: Option<[f32; 4]>) {
    let half = s * 0.5;
    prism(m, &f.square(u, v, half, half), PAVE, PAVE + s * 0.85, hex(TIMBER), true);
    f.face_u(m, u - half - 0.004, (v - half, v + half), (PAVE + s * 0.38, PAVE + s * 0.46), f.out(), hex(TIMBER_DARK));
    if let Some(g) = goods {
        loft(m, &f.square(u, v, half * 0.85, half * 0.85), PAVE + s * 0.85, &f.square(u, v, half * 0.45, half * 0.45), PAVE + s, g, true);
    }
}

/// A sack, tied at the neck, its mouth showing what's in it.
fn sack(m: &mut MeshBuf, f: &Frame, (u, v): (f32, f32), r: f32, goods: Option<[f32; 4]>) {
    let burlap = hex(0xb59a6a);
    let body = PAVE + r * 2.2;
    loft(m, &f.square(u, v, r, r * 0.9), PAVE, &f.square(u, v, r * 0.7, r * 0.6), body, burlap, false);
    loft(m, &f.square(u, v, r * 0.7, r * 0.6), body, &f.square(u, v, r * 0.85, r * 0.75), body + r * 0.4, burlap, false);
    prism(m, &f.square(u, v, r * 0.8, r * 0.7), body + r * 0.3, body + r * 0.36, goods.unwrap_or(burlap), true);
}

/// The eight corners of something round of radius `r` at (u, v).
fn round(f: &Frame, (u, v): (f32, f32), r: f32) -> Vec<V2> {
    (0..8)
        .map(|i| {
            let t = std::f32::consts::TAU * (i as f32 + 0.5) / 8.0;
            f.at(u + r * t.cos(), v + r * t.sin())
        })
        .collect()
}

/// A barrel with two hoops, its lid off when it holds goods.
fn barrel(m: &mut MeshBuf, f: &Frame, at: (f32, f32), r: f32, goods: Option<[f32; 4]>) {
    let tall = r * 2.6;
    loft(m, &round(f, at, r * 0.88), PAVE, &round(f, at, r), PAVE + tall * 0.5, hex(TIMBER), false);
    loft(m, &round(f, at, r), PAVE + tall * 0.5, &round(f, at, r * 0.88), PAVE + tall, hex(TIMBER), false);
    for y in [PAVE + tall * 0.18, PAVE + tall * 0.8] {
        let k = 0.9 + 0.1 * (1.0 - ((y - PAVE) / tall - 0.5).abs() * 2.0);
        prism(m, &round(f, at, r * k + 0.008), y, y + 0.03, hex(IRON), false);
    }
    let top = goods.map_or(hex(TIMBER_DARK), |g| mix(g, hex(TIMBER_DARK), 0.15));
    prism(m, &round(f, at, r * 0.86), PAVE + tall - 0.02, PAVE + tall - 0.005, top, true);
}

/// A wicker basket, wider at the top, heaped with goods.
fn basket(m: &mut MeshBuf, f: &Frame, at: (f32, f32), r: f32, goods: Option<[f32; 4]>) {
    let wicker = hex(0xa98552);
    let top = PAVE + r * 1.4;
    loft(m, &round(f, at, r * 0.75), PAVE, &round(f, at, r), top, wicker, false);
    prism(m, &round(f, at, r * 1.03), top - 0.02, top + 0.01, mix(wicker, hex(TIMBER_DARK), 0.4), false);
    loft(m, &round(f, at, r * 0.95), top - 0.01, &round(f, at, r * 0.4), top + r * 0.55, goods.unwrap_or(wicker), true);
}
