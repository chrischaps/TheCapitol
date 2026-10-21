//! The street wall (E3.14): a narrow house behind each stall, or over a
//! shopfront, two or three storeys of plaster, the upper ones jettied out
//! over the stall and leaning a little along the street, with shuttered
//! windows (some lit at dusk), window boxes, timber framing on some, and a
//! tiled roof with its ridge along the street or a gable to it.

use super::{
    rnd, sloped, tri, Bazaar, Frame, House, Kind, Lot, MAX_RISE, PALE_STONE, PAVE, PROUD,
    TIMBER, TIMBER_DARK, TOP_Y,
};
use crate::world::mesh::MeshBuf;
use crate::world::palette::{hex, mix, CHIMNEY, DOORS};
use crate::world::tower::prism;

/// A window: its width and height, and how far over its floor it starts.
const WINDOW: (f32, f32, f32) = (0.42, 0.78, 0.72);
const SHUTTER: f32 = 0.2;
/// How many of the windows are lit at night.
const LIT: f32 = 0.55;
const FLOWERS: [u32; 4] = [0xc0392b, 0xe57fa0, 0xe8c547, 0xf2efe6];

/// Builds `house` for `lot`: every floor but a shopfront's ground floor,
/// which is the shop's (`stalls::shopfront`), and its roof.
pub(super) fn house(h: &mut Bazaar, lot: &Lot, house: &House) {
    let f = &lot.frame;
    let half = lot.half();
    let wall = hex(house.wall);
    let seed = lot.seed;
    if lot.kind != Kind::Shopfront {
        let (u, v0, v1) = house.floor(0, half);
        let y1 = House::floor_y(1);
        prism(&mut h.stone, &f.rect(u, house.u1, v0, v1), PAVE, y1, wall, true);
        f.face_u(&mut h.stone, u - PROUD * 0.5, (v0, v1), (PAVE, PAVE + 0.3), f.out(), mix(wall, hex(PALE_STONE), 0.6));
        // A door into the stall in front, and a window if there's room
        let door = v0 + (v1 - v0) * if seed.is_multiple_of(2) { 0.3 } else { 0.7 };
        let colour = hex(DOORS[(seed >> 3) as usize % DOORS.len()]);
        f.face_u(&mut h.stone, u - PROUD, (door - 0.27, door + 0.27), (PAVE, PAVE + 1.75), f.out(), colour);
        f.face_u(&mut h.stone, u - PROUD * 0.5, (door - 0.33, door + 0.33), (PAVE + 1.75, PAVE + 1.87), f.out(), hex(TIMBER_DARK));
        if v1 - v0 > 1.6 {
            let other = v0 + (v1 - v0) * if seed.is_multiple_of(2) { 0.74 } else { 0.26 };
            window(h, f, (u, other, PAVE + 0.95), house, rnd(seed, 310) < LIT, false);
        }
    }
    for k in 1..house.storeys {
        let (u, v0, v1) = house.floor(k, half);
        let (below, _, _) = house.floor(k - 1, half);
        let (y0, y1) = (House::floor_y(k), House::floor_y(k + 1));
        prism(&mut h.stone, &f.rect(u, house.u1, v0, v1), y0, y1, wall, true);
        // The jetty's underside, and its beam
        if below - u > 0.01 {
            h.stone.quad([f.p(u, v0, y0), f.p(below, v0, y0), f.p(below, v1, y0), f.p(u, v1, y0)], [0.0, -1.0, 0.0], hex(TIMBER_DARK));
        }
        let n = (((v1 - v0 - 0.3) / 0.95).floor() as usize).clamp(1, 3);
        let slot = (v1 - v0) / n as f32;
        if house.framed {
            let beam = mix(hex(TIMBER_DARK), wall, 0.15);
            for ys in [(y0, y0 + 0.09), (y1 - 0.09, y1)] {
                f.face_u(&mut h.stone, u - PROUD * 0.5, (v0, v1), ys, f.out(), beam);
            }
            for i in 0..=n {
                let v = (v0 + slot * i as f32).clamp(v0 + 0.045, v1 - 0.045);
                f.face_u(&mut h.stone, u - PROUD * 0.5, (v - 0.045, v + 0.045), (y0 + 0.09, y1 - 0.09), f.out(), beam);
            }
        }
        for i in 0..n {
            let v = v0 + slot * (i as f32 + 0.5);
            let salt = 320 + (k * 4 + i) as u32;
            window(h, f, (u, v, y0 + WINDOW.2), house, rnd(seed, salt) < LIT, rnd(seed, salt + 40) < 0.3);
        }
        // One window at the back, over the moat or the court behind
        let back = (v0 + v1) * 0.5;
        let lit = rnd(seed, 400 + k as u32) < LIT * 0.7;
        let (ww, wh, over) = WINDOW;
        let glass = if lit { &mut h.lamps } else { &mut h.stone };
        f.face_u(glass, house.u1 + PROUD, (back - ww / 2.0, back + ww / 2.0), (y0 + over, y0 + over + wh), f.back(), hex(super::GLASS));
        f.face_u(&mut h.stone, house.u1 + PROUD * 0.5, (back - ww / 2.0 - 0.05, back + ww / 2.0 + 0.05), (y0 + over - 0.05, y0 + over + wh + 0.05), f.back(), hex(TIMBER_DARK));
    }
    roof(h, lot, house);
}

/// A window on a street face at `u`, centred at `v` with its sill at `y`:
/// glass that may light at dusk, a frame, a sill, open shutters either
/// side, and perhaps a box of flowers.
fn window(h: &mut Bazaar, f: &Frame, (u, v, y): (f32, f32, f32), house: &House, lit: bool, flowers: bool) {
    let (ww, wh, _) = WINDOW;
    let out = f.out();
    let glass = if lit { &mut h.lamps } else { &mut h.stone };
    f.face_u(glass, u - PROUD * 1.5, (v - ww / 2.0, v + ww / 2.0), (y, y + wh), out, hex(super::GLASS));
    f.face_u(&mut h.stone, u - PROUD * 0.5, (v - ww / 2.0 - 0.05, v + ww / 2.0 + 0.05), (y - 0.05, y + wh + 0.05), out, hex(TIMBER_DARK));
    let shutter = hex(house.shutter);
    for (a, b) in [(v - ww / 2.0 - 0.06 - SHUTTER, v - ww / 2.0 - 0.06), (v + ww / 2.0 + 0.06, v + ww / 2.0 + 0.06 + SHUTTER)] {
        f.face_u(&mut h.stone, u - PROUD, (a, b), (y, y + wh), out, shutter);
        f.face_u(&mut h.stone, u - PROUD * 1.4, ((a + b) * 0.5 - 0.008, (a + b) * 0.5 + 0.008), (y + 0.04, y + wh - 0.04), out, mix(shutter, [0.0, 0.0, 0.0, 1.0], 0.35));
    }
    prism(&mut h.stone, &f.rect(u - 0.07, u, v - ww / 2.0 - 0.07, v + ww / 2.0 + 0.07), y - 0.07, y - 0.03, hex(PALE_STONE), true);
    if flowers {
        prism(&mut h.stone, &f.rect(u - 0.16, u - 0.07, v - ww / 2.0, v + ww / 2.0), y - 0.2, y - 0.07, hex(TIMBER), true);
        let n = 4;
        for j in 0..n {
            let a = v - ww / 2.0 + 0.02 + (ww - 0.04) * j as f32 / n as f32;
            let colour = hex(FLOWERS[(house.wall as usize + j) % FLOWERS.len()]);
            prism(&mut h.stone, &f.rect(u - 0.15, u - 0.08, a + 0.01, a + (ww - 0.04) / n as f32 - 0.01), y - 0.07, y + 0.03, colour, true);
        }
    }
}

/// The roof over the top floor: tiles in courses of two shades, overhung
/// all round, with the ridge along the street (eaves to it) or across it
/// (a gable to it, with a small window); and maybe a chimney.
fn roof(h: &mut Bazaar, lot: &Lot, house: &House) {
    let f = &lot.frame;
    let (ua, v0, v1) = house.floor(house.storeys - 1, lot.half());
    let ub = house.u1;
    let y = house.eaves();
    let o = 0.14;
    let tile = hex(house.tile);
    let dark = mix(tile, [0.0, 0.0, 0.0, 1.0], 0.18);
    let wall = hex(house.wall);
    const COURSES: usize = 4;
    // One pitch from (a, the eave) to (b, the ridge), as a function of the
    // two frame axes: `at(along_slope, across)`
    let pitch = |m: &mut MeshBuf, at: &dyn Fn(f32, f32, f32) -> [f32; 3], (a, b): (f32, f32), (c0, c1): (f32, f32), (ye, yr): (f32, f32)| {
        for k in 0..COURSES {
            let (t0, t1) = (k as f32 / COURSES as f32, (k + 1) as f32 / COURSES as f32);
            let s = |t: f32| (a + (b - a) * t, ye + (yr - ye) * t);
            let ((s0, y0), (s1, y1)) = (s(t0), s(t1));
            sloped(m, [at(s0, c0, y0), at(s1, c0, y1), at(s1, c1, y1), at(s0, c1, y0)], if k % 2 == 0 { tile } else { dark });
        }
    };
    let top = if house.ridge_along {
        let um = (ua + ub) * 0.5;
        let rise = ((ub - ua) * 0.42).min(MAX_RISE);
        let slope = rise / (um - ua).max(1e-3);
        let eave = y - o * slope;
        let at = |s: f32, c: f32, yy: f32| f.p(s, c, yy);
        pitch(&mut h.stone, &at, (ua - o, um), (v0 - o, v1 + o), (eave, y + rise));
        pitch(&mut h.stone, &at, (ub + o, um), (v0 - o, v1 + o), (eave, y + rise));
        for (v, s) in [(v0, -1.0), (v1, 1.0)] {
            tri(&mut h.stone, [f.p(ua, v, y), f.p(ub, v, y), f.p(um, v, y + rise)], f.side(s), wall);
        }
        prism(&mut h.stone, &f.rect(um - 0.05, um + 0.05, v0 - o, v1 + o), y + rise - 0.07, y + rise, dark, true);
        y + rise
    } else {
        let vm = (v0 + v1) * 0.5;
        let rise = ((v1 - v0) * 0.45).min(MAX_RISE);
        let slope = rise / (vm - v0).max(1e-3);
        let eave = y - o * slope;
        let at = |s: f32, c: f32, yy: f32| f.p(c, s, yy);
        pitch(&mut h.stone, &at, (v0 - o, vm), (ua - o, ub + o), (eave, y + rise));
        pitch(&mut h.stone, &at, (v1 + o, vm), (ua - o, ub + o), (eave, y + rise));
        for (u, n) in [(ua, f.out()), (ub, f.back())] {
            tri(&mut h.stone, [f.p(u, v0, y), f.p(u, v1, y), f.p(u, vm, y + rise)], n, wall);
        }
        // The gable's small window, toward the street
        if rise > 0.6 {
            let lit = rnd(lot.seed, 450) < LIT;
            let glass = if lit { &mut h.lamps } else { &mut h.stone };
            f.face_u(glass, ua - PROUD * 1.5, (vm - 0.12, vm + 0.12), (y + 0.15, y + 0.42), f.out(), hex(super::GLASS));
            f.face_u(&mut h.stone, ua - PROUD * 0.5, (vm - 0.16, vm + 0.16), (y + 0.11, y + 0.46), f.out(), hex(TIMBER_DARK));
        }
        prism(&mut h.stone, &f.rect(ua - o, ub + o, vm - 0.05, vm + 0.05), y + rise - 0.07, y + rise, dark, true);
        y + rise
    };
    if house.chimney {
        let (u, v) = (ub - 0.3, if lot.seed.is_multiple_of(3) { v0 + 0.3 } else { v1 - 0.3 });
        let peak = (top + 0.3).min(TOP_Y);
        prism(&mut h.stone, &f.square(u, v, 0.13, 0.13), y, peak - 0.06, hex(CHIMNEY), true);
        prism(&mut h.stone, &f.square(u, v, 0.16, 0.16), peak - 0.06, peak, mix(hex(CHIMNEY), [0.0, 0.0, 0.0, 1.0], 0.25), true);
    }
}
