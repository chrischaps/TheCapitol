//! The bazaar (E3.11): merchants' own shops, in stalls along a covered lane
//! on the west side of the Capitol avenue, across from the Exchange hall
//! (E3.10, plan D6). It's built on the footprints `/terrain` gives it: the
//! bazaar's, and each stall's.
//!
//! The hall and the bazaar are a pair: the same stone and the same gold, but
//! where the hall is the city's one book, cool and ordered behind Trade-blue
//! glass, the bazaar is many people's, warm and coloured:
//!
//! - on the **avenue**, a stone front with a pilaster at each stall, and in
//!   its middle the **gate**: two piers and a lintel with a gold band, a
//!   lantern under it. On the outside of each kept stall hangs a **banner**
//!   in its shop's colour under a sconce that lights at dusk; a free stall
//!   shows its shutters. So the street sees which stalls are kept, and at
//!   night the bazaar lights up as merchants move in. The ends of the rows
//!   hang the bazaar's own banners, in amber;
//! - a **tiled roof** over each row of stalls, sloping down to the lane;
//! - in each **kept stall**, a counter with its goods on it (coloured by the
//!   kinds the shop sells, as the directory names them), more on a shelf
//!   behind, a **striped awning** out over the lane in the shop's own
//!   colour, and a **sign** hanging from it, where the HUD writes the shop's
//!   name; and a lantern at its corner;
//! - an **empty stall** stands bare: an empty counter, and its awning furled
//!   under the eaves;
//! - across the lane, **strings of lanterns** from eave to eave. They and
//!   the windows light at dusk (H23, `material.emissive_scale` on
//!   [`LAMPS_MESH`]), so from the map the bazaar is an amber point facing
//!   the hall's blue one.
//!
//! Three meshes, as the hall's: [`MESH`] (stone, timber, tile and cloth),
//! [`LAMPS_MESH`] (the glass that lights) and [`GOLD_MESH`] (gold metal).
//! They're rebuilt when the terrain or the directory changes which stalls
//! are kept, or what they sell.

use crate::coords::{layer, to_world};
use crate::world::mesh::MeshBuf;
use crate::world::palette::{hex, mix};
use crate::world::tower::{hash, lift, loft, prism, V2};
use capitol_protocol::rest::ShopSign;
use capitol_protocol::terrain::{Landmark, LandmarkKind};
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;

/// The runtime meshes' names, as their entities are named.
pub const MESH: &str = "bazaar";
pub const LAMPS_MESH: &str = "bazaar_lamps";
pub const GOLD_MESH: &str = "bazaar_gold";

/// Every mesh, for attaching their entities.
pub fn meshes() -> [String; 3] {
    [MESH, LAMPS_MESH, GOLD_MESH].map(str::to_owned)
}

const G: f32 = layer::GROUND;
/// The paving: level with the lots' tops, as the hall's forecourt.
const PAVE: f32 = layer::PLOT;
/// The front's and the back wall's top, where the roofs rise from.
const WALL_TOP: f32 = G + 2.9;
/// The roofs' eaves over the stalls' fronts.
const EAVE: f32 = G + 2.4;
/// The gate's piers, and the lintel between them.
const PIER_TOP: f32 = G + 4.1;
const LINTEL: (f32, f32) = (G + 3.3, G + 3.75);
/// The counter's top.
const COUNTER: f32 = PAVE + 0.85;
/// How far an awning reaches out over the lane, and where it ends.
const AWNING_OUT: f32 = 0.7;
const AWNING_LOW: f32 = G + 2.0;
/// The hanging sign: its bottom, top, and half its width.
const SIGN: (f32, f32) = (G + 1.6, G + 1.92);
const SIGN_HALF: f32 = 0.34;
/// The lanterns strung across the lane: how high their strings hang at the
/// eaves, how far they sag, and how far apart the lanterns are.
const STRING_Y: f32 = EAVE;
const SAG: f32 = 0.22;
const LANTERN_EVERY: f32 = 0.38;
/// The top of everything: the gate's finials.
pub const TOP_Y: f32 = PIER_TOP + 0.45;

// Materials: the hall's stone (`world::hall`), so the two read as the
// Capitol's, and warm tile and timber for the market.
const STONE: u32 = 0xa79f90;
const PALE_STONE: u32 = 0xc8bfae;
const PAVING: u32 = 0x928879;
const CROWN: u32 = 0x7b7266;
const TILE: u32 = 0x9c5b3f;
const TILE_DARK: u32 = 0x864c35;
const TIMBER: u32 = crate::world::palette::TIMBER;
const TIMBER_DARK: u32 = 0x6e5236;
const CLOTH: u32 = 0xe9e1cf;
const CORD: u32 = 0x2c2620;
const GOLD: u32 = crate::world::palette::GOLD;
/// The lanterns' glass by day, and the light they give at night.
const LANTERN_GLASS: u32 = 0x6b4a26;
pub const LANTERN_LIGHT: u32 = 0xffae57;
/// The shops' colours: an awning's stripes and its sign's border. A shop
/// keeps its colour (a hash of its name), whoever looks.
const AWNINGS: [u32; 7] = [0x3b82f6, 0xc9a227, 0x2f7f6f, 0x9b3b3b, 0x6b4a8a, 0xd9822b, 0x4f7a3a];
/// A sign's board.
const SIGN_BOARD: u32 = 0x2a2420;
/// The bazaar's own banners, on the ends of its rows: the lanterns' amber
/// as cloth.
const BAZAAR_CLOTH: u32 = 0xc9782e;

/// A footprint as world metres, from its entrance edge: `u` runs in from
/// it, `v` along it, both unscaled.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    /// The middle of the entrance edge.
    door: V2,
    /// In from the door, and along the edge.
    f: V2,
    a: V2,
    depth: f32,
    width: f32,
}

impl Frame {
    fn new(l: &Landmark) -> Self {
        let w = |p: (f64, f64)| to_world(p.0, p.1);
        let (i, j) = l.entrance_edge;
        let (e0, e1) = (w(l.corners[i % 4]), w(l.corners[j % 4]));
        let door = [(e0[0] + e1[0]) * 0.5, (e0[1] + e1[1]) * 0.5];
        let centre = w(l.position);
        let inward = [centre[0] - door[0], centre[1] - door[1]];
        let half = inward[0].hypot(inward[1]).max(1e-6);
        let along = [e1[0] - e0[0], e1[1] - e0[1]];
        let width = along[0].hypot(along[1]).max(1e-6);
        Self {
            door,
            f: [inward[0] / half, inward[1] / half],
            a: [along[0] / width, along[1] / width],
            depth: half * 2.0,
            width,
        }
    }

    fn at(&self, u: f32, v: f32) -> V2 {
        [
            self.door[0] + self.f[0] * u + self.a[0] * v,
            self.door[1] + self.f[1] * u + self.a[1] * v,
        ]
    }

    /// Where world point `p` falls in the frame, as (u, v).
    fn local(&self, p: V2) -> (f32, f32) {
        let d = [p[0] - self.door[0], p[1] - self.door[1]];
        (d[0] * self.f[0] + d[1] * self.f[1], d[0] * self.a[0] + d[1] * self.a[1])
    }

    fn rect(&self, u0: f32, u1: f32, v0: f32, v1: f32) -> Vec<V2> {
        vec![self.at(u0, v0), self.at(u1, v0), self.at(u1, v1), self.at(u0, v1)]
    }

    /// Out of the entrance edge, as a normal.
    fn out(&self) -> [f32; 3] {
        [-self.f[0], 0.0, -self.f[1]]
    }

    /// In, as a normal.
    fn back(&self) -> [f32; 3] {
        [self.f[0], 0.0, self.f[1]]
    }

    /// A box `half` wide either way round (u, v), turned with the frame.
    fn square(&self, u: f32, v: f32, half_u: f32, half_v: f32) -> Vec<V2> {
        self.rect(u - half_u, u + half_u, v - half_v, v + half_v)
    }
}

/// One stall, as the world draws it: its footprint, and its shop if it's
/// kept.
#[derive(Debug, Clone, Copy)]
pub struct StallLook<'a> {
    pub stall: &'a Landmark,
    pub shop: Option<&'a ShopSign>,
}

/// Where a stall's name hangs, for the HUD to write it: the middle of its
/// sign, or over an empty stall's counter, world (X, Y, Z).
#[derive(Debug, Clone, PartialEq)]
pub struct Sign {
    pub stall_id: String,
    pub at: [f32; 3],
    /// The shop's colour, as linear RGB, for the name's underline.
    pub colour: [f32; 3],
}

/// The bazaar, built: one buffer per material, and where its signs hang.
#[derive(Default)]
pub struct Bazaar {
    pub stone: MeshBuf,
    pub lamps: MeshBuf,
    pub gold: MeshBuf,
    pub signs: Vec<Sign>,
}

/// The rows of stalls along the lane, as the bazaar's frame sees them: where
/// the lane runs (u), and the gate's opening in the front row (v).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Layout {
    lane: (f32, f32),
    gate: (f32, f32),
}

impl Layout {
    /// From the stalls: the front row faces in to the lane, the back row out
    /// to it, and the gate is the gap in the front row round the door. A
    /// bazaar with no stalls gets E3.10's proportions.
    fn new(b: &Frame, stalls: &[StallLook]) -> Self {
        let mut lane = (b.depth * 0.3125, b.depth * 0.6875);
        let mut gate = (-0.8f32, 0.8f32);
        let (mut front, mut back) = (None::<f32>, None::<f32>);
        let mut edges = Vec::new();
        for s in stalls {
            let door = b.local(to_world(s.stall.door.0, s.stall.door.1));
            let centre = b.local(to_world(s.stall.position.0, s.stall.position.1));
            if door.0 > centre.0 {
                front = Some(front.map_or(door.0, |f| f.max(door.0)));
                let f = Frame::new(s.stall);
                let ends = [b.local(f.at(0.0, -f.width / 2.0)).1, b.local(f.at(0.0, f.width / 2.0)).1];
                edges.push((ends[0].min(ends[1]), ends[0].max(ends[1])));
            } else {
                back = Some(back.map_or(door.0, |k| k.min(door.0)));
            }
        }
        if let (Some(f), Some(k)) = (front, back) {
            lane = (f, k);
        }
        if !edges.is_empty() {
            let below = edges.iter().map(|e| e.1).filter(|v| *v <= 0.01).fold(f32::MIN, f32::max);
            let above = edges.iter().map(|e| e.0).filter(|v| *v >= -0.01).fold(f32::MAX, f32::min);
            if below > f32::MIN && above < f32::MAX {
                gate = (below, above);
            }
        }
        Self { lane, gate }
    }
}

/// Builds the bazaar on `bazaar`, with its `stalls` as they're kept.
pub fn build(bazaar: &Landmark, stalls: &[StallLook]) -> Bazaar {
    let b = Frame::new(bazaar);
    let layout = Layout::new(&b, stalls);
    let mut out = Bazaar::default();
    paving(&mut out, &b, &layout);
    walls(&mut out, &b, &layout, stalls);
    gate(&mut out, &b, &layout);
    roofs(&mut out, &b, &layout);
    for s in stalls {
        stall(&mut out, s);
    }
    lanterns(&mut out, &b, &layout, stalls);
    out
}

/// The paving over the footprint, and a darker crown down the lane.
fn paving(h: &mut Bazaar, b: &Frame, l: &Layout) {
    let (w0, w1) = (-b.width / 2.0, b.width / 2.0);
    prism(&mut h.stone, &b.rect(0.0, b.depth, w0, w1), G - 0.02, PAVE, hex(PAVING), true);
    let mid = (l.lane.0 + l.lane.1) * 0.5;
    prism(&mut h.stone, &b.rect(mid - 0.12, mid + 0.12, w0, w1), PAVE, PAVE + 0.005, hex(CROWN), true);
    // The gate's threshold, a step of pale stone
    prism(&mut h.stone, &b.rect(0.0, 0.35, l.gate.0, l.gate.1), PAVE, PAVE + 0.04, hex(PALE_STONE), true);
}

/// The front on the avenue and the back wall, each with a pilaster at every
/// stall's side and a cornice; the ends; and in the front, over each front
/// stall, a small high window that lights at dusk.
fn walls(h: &mut Bazaar, b: &Frame, l: &Layout, stalls: &[StallLook]) {
    const THICK: f32 = 0.22;
    let (w0, w1) = (-b.width / 2.0, b.width / 2.0);
    let stone = hex(STONE);
    let pale = hex(PALE_STONE);
    // The front, either side of the gate, and the back, whole
    for (v0, v1) in [(w0, l.gate.0), (l.gate.1, w1)] {
        prism(&mut h.stone, &b.rect(0.0, THICK, v0, v1), PAVE, WALL_TOP, stone, true);
        prism(&mut h.stone, &b.rect(-0.05, THICK + 0.03, v0, v1), WALL_TOP - 0.18, WALL_TOP, pale, true);
        prism(&mut h.stone, &b.rect(-0.04, 0.0, v0, v1), PAVE, PAVE + 0.35, pale, true);
    }
    prism(&mut h.stone, &b.rect(b.depth - THICK, b.depth, w0, w1), PAVE, WALL_TOP, stone, true);
    prism(&mut h.stone, &b.rect(b.depth - THICK - 0.03, b.depth + 0.05, w0, w1), WALL_TOP - 0.18, WALL_TOP, pale, true);
    // The ends: whole behind each row, and low across the lane, with a pier
    // each side of it, so the lane reads as enclosed but open to the sky
    for (v, sign) in [(w0, 1.0f32), (w1, -1.0f32)] {
        let (a, c) = if sign > 0.0 { (v, v + THICK) } else { (v - THICK, v) };
        prism(&mut h.stone, &b.rect(0.0, l.lane.0, a, c), PAVE, WALL_TOP, stone, true);
        prism(&mut h.stone, &b.rect(l.lane.1, b.depth, a, c), PAVE, WALL_TOP, stone, true);
        prism(&mut h.stone, &b.rect(l.lane.0, l.lane.1, a, c), PAVE, PAVE + 0.9, stone, true);
        for u in [l.lane.0, l.lane.1] {
            prism(&mut h.stone, &b.square(u, (a + c) * 0.5, 0.16, 0.16), PAVE, EAVE + 0.15, pale, true);
        }
        // The bazaar's own banners on the end of each row, in the
        // lanterns' amber: a frame on the wall's outer face for `banner`
        for (u0, u1) in [(0.0, l.lane.0), (l.lane.1, b.depth)] {
            let wall = Frame {
                door: b.at((u0 + u1) * 0.5, v + sign * THICK),
                f: [-b.a[0] * sign, -b.a[1] * sign],
                a: b.f,
                depth: THICK,
                width: u1 - u0,
            };
            banner(h, &wall, hex(BAZAAR_CLOTH));
        }
    }
    // A pilaster at each stall's sides, out on the avenue and the back
    for s in stalls {
        let f = Frame::new(s.stall);
        for side in [-0.5f32, 0.5] {
            let (u, v) = b.local(f.at(0.0, f.width * side));
            if (v - w0).abs() < 0.3 || (v - w1).abs() < 0.3 {
                continue;
            }
            let front = u < b.depth * 0.5;
            if front && v > l.gate.0 - 0.05 && v < l.gate.1 + 0.05 {
                continue;
            }
            let (u0, u1) = if front { (-0.08, 0.02) } else { (b.depth - 0.02, b.depth + 0.08) };
            prism(&mut h.stone, &b.rect(u0, u1, v - 0.1, v + 0.1), PAVE, WALL_TOP - 0.18, pale, true);
        }
    }
}

/// The gate on the avenue: two piers with gold finials, a lintel with a gold
/// band across, and a lantern hanging under it.
fn gate(h: &mut Bazaar, b: &Frame, l: &Layout) {
    let pale = hex(PALE_STONE);
    let (g0, g1) = l.gate;
    for v in [g0 - 0.2, g1 + 0.2] {
        prism(&mut h.stone, &b.square(0.32, v, 0.3, 0.24), PAVE, PIER_TOP, pale, true);
        prism(&mut h.stone, &b.square(0.32, v, 0.32, 0.28), PIER_TOP, PIER_TOP + 0.1, hex(STONE), true);
        loft(&mut h.gold, &b.square(0.32, v, 0.1, 0.1), PIER_TOP + 0.1, &b.square(0.32, v, 0.015, 0.015), TOP_Y, hex(GOLD), false);
    }
    prism(&mut h.stone, &b.rect(-0.1, 0.42, g0 - 0.1, g1 + 0.1), LINTEL.0, LINTEL.1, hex(STONE), true);
    // The gold band on the lintel's face, toward the avenue, as the hall's
    // rules run round its ticker
    let (p, q) = (b.at(-0.105, g0 - 0.05), b.at(-0.105, g1 + 0.05));
    let (y0, y1) = (LINTEL.0 + 0.16, LINTEL.0 + 0.27);
    h.gold.quad([lift(p, y0), lift(q, y0), lift(q, y1), lift(p, y1)], b.out(), hex(GOLD));
    // The lantern on a short chain
    let c = b.at(0.16, (g0 + g1) * 0.5);
    prism(&mut h.gold, &[[c[0] - 0.01, c[1] - 0.01], [c[0] + 0.01, c[1] - 0.01], [c[0] + 0.01, c[1] + 0.01], [c[0] - 0.01, c[1] + 0.01]], LINTEL.0 - 0.3, LINTEL.0, hex(CORD), false);
    lantern(h, c, LINTEL.0 - 0.3, 0.1);
}

/// A lantern hanging from `top`: a cap, glass that lights, and a base.
fn lantern(h: &mut Bazaar, c: V2, top: f32, size: f32) {
    let sq = |r: f32| vec![[c[0] - r, c[1] - r], [c[0] + r, c[1] - r], [c[0] + r, c[1] + r], [c[0] - r, c[1] + r]];
    let glass = top - 0.04;
    let bottom = glass - size * 1.6;
    loft(&mut h.gold, &sq(size * 0.8), glass, &sq(size * 0.2), top, hex(TIMBER_DARK), false);
    prism(&mut h.lamps, &sq(size * 0.5), bottom, glass, hex(LANTERN_GLASS), true);
    prism(&mut h.gold, &sq(size * 0.6), bottom - 0.03, bottom, hex(TIMBER_DARK), true);
}

/// A tiled roof over each row, from the wall's top down to the eaves over
/// the stalls' fronts, with tile courses in two tones; broken at the gate.
fn roofs(h: &mut Bazaar, b: &Frame, l: &Layout) {
    let (w0, w1) = (-b.width / 2.0, b.width / 2.0);
    const OVER: f32 = 0.18;
    // Front row: high at the avenue, down to the lane
    for (v0, v1) in [(w0, l.gate.0 - 0.44), (l.gate.1 + 0.44, w1)] {
        roof(h, b, (-0.05, WALL_TOP + 0.1), (l.lane.0 + OVER, EAVE), (v0, v1));
    }
    // The gate's own short roof, over the passage between the piers
    roof(h, b, (-0.05, WALL_TOP + 0.1), (l.lane.0 + OVER, EAVE), (l.gate.0 - 0.44, l.gate.1 + 0.44));
    // Back row: high at the back wall, down to the lane
    roof(h, b, (b.depth + 0.05, WALL_TOP + 0.1), (l.lane.1 - OVER, EAVE), (w0, w1));
}

/// One pitch from (u, y) at the top to (u, y) at the eave, along `v`, in
/// courses, both faces; and a timber fascia along the eave.
fn roof(h: &mut Bazaar, b: &Frame, top: (f32, f32), eave: (f32, f32), (v0, v1): (f32, f32)) {
    const COURSES: usize = 6;
    for k in 0..COURSES {
        let t0 = k as f32 / COURSES as f32;
        let t1 = (k + 1) as f32 / COURSES as f32;
        let u = |t: f32| top.0 + (eave.0 - top.0) * t;
        let y = |t: f32| top.1 + (eave.1 - top.1) * t;
        let corners = [lift(b.at(u(t0), v0), y(t0)), lift(b.at(u(t1), v0), y(t1)), lift(b.at(u(t1), v1), y(t1)), lift(b.at(u(t0), v1), y(t0))];
        let n = up_normal(corners);
        let colour = hex(if k % 2 == 0 { TILE } else { TILE_DARK });
        h.stone.quad(corners, n, colour);
        h.stone.quad(corners, n.map(|x| -x), mix(hex(TIMBER_DARK), [0.0, 0.0, 0.0, 1.0], 0.3));
    }
    let toward_lane = if eave.0 > top.0 { b.back() } else { b.out() };
    let (p, q) = (b.at(eave.0, v0), b.at(eave.0, v1));
    h.stone.quad([lift(p, eave.1 - 0.12), lift(q, eave.1 - 0.12), lift(q, eave.1), lift(p, eave.1)], toward_lane, hex(TIMBER));
}

/// A stall: its partitions and counter, and kept, its goods, awning, sign
/// and lantern; empty, its awning furled.
fn stall(h: &mut Bazaar, s: &StallLook) {
    let f = Frame::new(s.stall);
    let (w0, w1) = (-f.width / 2.0, f.width / 2.0);
    let timber = hex(TIMBER);
    // The partitions between stalls, under the roof
    for v in [w0 + 0.03, w1 - 0.03] {
        prism(&mut h.stone, &f.rect(0.02, f.depth - 0.2, v - 0.025, v + 0.025), PAVE, EAVE - 0.1, timber, true);
    }
    // The counter, across the front
    prism(&mut h.stone, &f.rect(0.08, 0.46, w0 + 0.09, w1 - 0.09), PAVE, COUNTER, hex(TIMBER_DARK), true);
    prism(&mut h.stone, &f.rect(0.05, 0.49, w0 + 0.07, w1 - 0.07), COUNTER, COUNTER + 0.04, timber, true);
    let Some(shop) = s.shop else {
        // Empty: the awning furled under the eave, and shutters closed on
        // the street
        prism(&mut h.stone, &f.rect(-0.16, -0.04, w0 + 0.1, w1 - 0.1), EAVE - 0.2, EAVE - 0.08, mix(hex(CLOTH), hex(STONE), 0.35), true);
        shutters(h, &f);
        let c = f.at(0.27, 0.0);
        h.signs.push(Sign { stall_id: s.stall.id.clone(), at: [c[0], COUNTER + 0.5, c[1]], colour: [0.8, 0.78, 0.72] });
        return;
    };
    let colour = AWNINGS[name_hash(&shop.name) as usize % AWNINGS.len()];
    goods(h, &f, shop, s.stall);
    awning(h, &f, hex(colour));
    banner(h, &f, hex(colour));
    // The sign, hanging from the awning's front edge on two cords
    let u = -AWNING_OUT + 0.02;
    let half = SIGN_HALF.min(f.width * 0.32);
    for v in [-half * 0.8, half * 0.8] {
        let c = f.at(u, v);
        prism(&mut h.gold, &[[c[0] - 0.008, c[1] - 0.008], [c[0] + 0.008, c[1] - 0.008], [c[0] + 0.008, c[1] + 0.008], [c[0] - 0.008, c[1] + 0.008]], SIGN.1, AWNING_LOW + 0.01, hex(CORD), false);
    }
    prism(&mut h.stone, &f.rect(u - 0.025, u + 0.025, -half, half), SIGN.0, SIGN.1, hex(SIGN_BOARD), true);
    // Its border in the shop's colour, on both faces
    for (uu, n) in [(u - 0.027, f.out()), (u + 0.027, f.back())] {
        for (y0, y1) in [(SIGN.0 + 0.02, SIGN.0 + 0.045), (SIGN.1 - 0.045, SIGN.1 - 0.02)] {
            let (p, q) = (f.at(uu, -half + 0.03), f.at(uu, half - 0.03));
            h.stone.quad([lift(p, y0), lift(q, y0), lift(q, y1), lift(p, y1)], n, hex(colour));
        }
    }
    let c = f.at(u, 0.0);
    let [r, g, bl, _] = hex(colour);
    h.signs.push(Sign { stall_id: s.stall.id.clone(), at: [c[0], (SIGN.0 + SIGN.1) * 0.5, c[1]], colour: [r, g, bl] });
    // A lantern at the awning's corner
    let side = if hash(name_hash(&shop.name), 3, 1).is_multiple_of(2) { w0 + 0.12 } else { w1 - 0.12 };
    lantern(h, f.at(-AWNING_OUT + 0.08, side), AWNING_LOW, 0.07);
}

/// How far proud of a stall's outer wall its banner and shutters stand.
const PROUD: f32 = 0.012;

/// Where a stall's outer wall is, in its frame: its back, out in the street.
fn outside(f: &Frame) -> f32 {
    f.depth + PROUD
}

/// A kept stall's banner on the outside of its back wall: shop-coloured
/// cloth with a pale band, a swallowtail, on a gold rod; and over it a
/// sconce that lights at dusk.
fn banner(h: &mut Bazaar, f: &Frame, colour: [f32; 4]) {
    let n = f.back();
    let u = outside(f);
    let half = (f.width * 0.2).min(0.28);
    let (top, bottom) = (WALL_TOP - 0.45, G + 1.05);
    let q = |a: f32, b: f32, y0: f32, y1: f32| [lift(f.at(u, a), y0), lift(f.at(u, b), y0), lift(f.at(u, b), y1), lift(f.at(u, a), y1)];
    // Facing out, the frame's +v runs to the viewer's left: wind the quads
    // to face `n` whichever way that is
    let face = |h: &mut Bazaar, c: [[f32; 3]; 4], colour: [f32; 4]| {
        if crate::world::mesh::facing(c[0], c[1], c[2], n) {
            h.stone.quad(c, n, colour);
        } else {
            h.stone.quad([c[1], c[0], c[3], c[2]], n, colour);
        }
    };
    let tail = bottom + 0.22;
    face(h, q(-half, half, tail, top), colour);
    face(h, q(-half, half, top - 0.16, top - 0.1), hex(CLOTH));
    // The swallowtail: two points down from the cloth's foot
    for (a, b) in [(-half, 0.0), (0.0, half)] {
        let (pa, pb) = (f.at(u, a), f.at(u, b));
        let point = if a < 0.0 { f.at(u, -half) } else { f.at(u, half) };
        let (a, b, c) = (lift(pa, tail), lift(pb, tail), lift(point, bottom));
        let (a, b) = if crate::world::mesh::facing(a, b, c, n) { (a, b) } else { (b, a) };
        let tri = [a, b, c].map(|p| h.stone.vertex(p, n, colour));
        h.stone.indices.extend(tri);
    }
    let rod = f.rect(f.depth - 0.002, f.depth + 0.035, -half - 0.05, half + 0.05);
    prism(&mut h.gold, &rod, top - 0.02, top + 0.02, hex(GOLD), true);
    // The sconce over it
    let c = f.at(f.depth + 0.05, 0.0);
    let sq = |r: f32| vec![[c[0] - r, c[1] - r], [c[0] + r, c[1] - r], [c[0] + r, c[1] + r], [c[0] - r, c[1] + r]];
    prism(&mut h.lamps, &sq(0.035), top + 0.12, top + 0.27, hex(LANTERN_GLASS), true);
    prism(&mut h.gold, &sq(0.045), top + 0.27, top + 0.3, hex(TIMBER_DARK), true);
}

/// A free stall's shutters on the outside of its back wall: two timber
/// leaves, closed.
fn shutters(h: &mut Bazaar, f: &Frame) {
    let n = f.back();
    let u = outside(f);
    let half = (f.width * 0.17).min(0.24);
    let (y0, y1) = (G + 1.7, G + 2.35);
    for (a, b) in [(-half, -0.01), (0.01, half)] {
        let c = [lift(f.at(u, a), y0), lift(f.at(u, b), y0), lift(f.at(u, b), y1), lift(f.at(u, a), y1)];
        let c = if crate::world::mesh::facing(c[0], c[1], c[2], n) { c } else { [c[1], c[0], c[3], c[2]] };
        h.stone.quad(c, n, hex(TIMBER_DARK));
    }
    prism(&mut h.stone, &f.rect(f.depth - 0.002, f.depth + 0.06, -half - 0.04, half + 0.04), y0 - 0.06, y0, hex(PALE_STONE), true);
}

/// The goods: bundles and crates on the counter, and more on a shelf at the
/// back, coloured by the kinds the shop sells. Which goes where is seeded
/// from the stall, so it doesn't shuffle as the directory comes again.
fn goods(h: &mut Bazaar, f: &Frame, shop: &ShopSign, stall: &Landmark) {
    let kinds: Vec<u32> = if shop.item_types.is_empty() {
        Vec::new()
    } else {
        shop.item_types.iter().map(|t| goods_colour(t)).collect()
    };
    let (w0, w1) = (-f.width / 2.0 + 0.12, f.width / 2.0 - 0.12);
    let seed = name_hash(&stall.id);
    let n = 3usize;
    let w = (w1 - w0) / n as f32;
    // The shelf at the back
    prism(&mut h.stone, &f.rect(f.depth - 0.62, f.depth - 0.3, w0, w1), PAVE + 1.05, PAVE + 1.09, hex(TIMBER), true);
    if kinds.is_empty() {
        return;
    }
    for j in 0..n {
        let colour = hex(kinds[(j + seed as usize) % kinds.len()]);
        let a = w0 + w * j as f32;
        let tall = 0.08 + 0.05 * (hash(seed, j as u32, 5) % 3) as f32;
        prism(&mut h.stone, &f.rect(0.12, 0.38, a + 0.03, a + w - 0.03), COUNTER + 0.04, COUNTER + 0.04 + tall, colour, true);
        let colour = hex(kinds[(j + 1 + seed as usize) % kinds.len()]);
        prism(&mut h.stone, &f.rect(f.depth - 0.58, f.depth - 0.34, a + 0.04, a + w - 0.04), PAVE + 1.09, PAVE + 1.09 + tall + 0.04, colour, true);
    }
    // A crate out front, on the lane
    let colour = hex(kinds[seed as usize % kinds.len()]);
    let c = (w0 + w1) * 0.5 + (w1 - w0) * 0.25 * if seed.is_multiple_of(2) { 1.0 } else { -1.0 };
    prism(&mut h.stone, &f.square(-0.2, c, 0.13, 0.13), PAVE, PAVE + 0.28, hex(TIMBER), true);
    prism(&mut h.stone, &f.square(-0.2, c, 0.1, 0.1), PAVE + 0.28, PAVE + 0.33, colour, true);
}

/// The awning: stripes in the shop's colour and cloth, sloping out and down
/// from the eave over the lane, both faces, with a valance on its edge.
fn awning(h: &mut Bazaar, f: &Frame, colour: [f32; 4]) {
    let (w0, w1) = (-f.width / 2.0 + 0.04, f.width / 2.0 - 0.04);
    let stripes = ((w1 - w0) / 0.16).round().max(2.0) as usize;
    let (ua, ub) = (-AWNING_OUT, -0.02);
    let (ya, yb) = (AWNING_LOW, EAVE - 0.06);
    for j in 0..stripes {
        let a = w0 + (w1 - w0) * j as f32 / stripes as f32;
        let b = w0 + (w1 - w0) * (j + 1) as f32 / stripes as f32;
        let c = if j % 2 == 0 { colour } else { hex(CLOTH) };
        let corners = [lift(f.at(ua, a), ya), lift(f.at(ub, a), yb), lift(f.at(ub, b), yb), lift(f.at(ua, b), ya)];
        let n = up_normal(corners);
        h.stone.quad(corners, n, c);
        h.stone.quad(corners, n.map(|x| -x), mix(c, [0.0, 0.0, 0.0, 1.0], 0.25));
        let drop = [lift(f.at(ua, a), ya), lift(f.at(ua, b), ya), lift(f.at(ua, b), ya - 0.14), lift(f.at(ua, a), ya - 0.14)];
        h.stone.quad(drop, f.out(), c);
        h.stone.quad(drop, f.back(), c);
    }
}

/// Strings of lanterns across the lane, eave to eave, zig-zagging from one
/// stall's side to the next, each sagging in a curve.
fn lanterns(h: &mut Bazaar, b: &Frame, l: &Layout, stalls: &[StallLook]) {
    // The stall sides along each row, as v
    let mut front: Vec<f32> = Vec::new();
    let mut back: Vec<f32> = Vec::new();
    for s in stalls {
        let f = Frame::new(s.stall);
        let (cu, _) = b.local(f.at(f.depth * 0.5, 0.0));
        let row = if cu < b.depth * 0.5 { &mut front } else { &mut back };
        for side in [-0.5f32, 0.5] {
            row.push(b.local(f.at(0.0, f.width * side)).1);
        }
    }
    let tidy = |row: &mut Vec<f32>| {
        row.sort_by(|a, b| a.total_cmp(b));
        row.dedup_by(|a, b| (*a - *b).abs() < 0.2);
    };
    tidy(&mut front);
    tidy(&mut back);
    if front.is_empty() || back.is_empty() {
        return;
    }
    let (u0, u1) = (l.lane.0 + 0.05, l.lane.1 - 0.05);
    let cord = hex(CORD);
    for (k, &v0) in back.iter().enumerate() {
        // To the front-row side nearest the next back-row side, so the
        // strings zig-zag down the lane
        let target = back.get(k + 1).copied().unwrap_or(v0);
        let v1 = front.iter().copied().min_by(|a, c| (a - target).abs().total_cmp(&(c - target).abs())).unwrap_or(v0);
        let (p, q) = (b.at(u1, v0), b.at(u0, v1));
        let len = (q[0] - p[0]).hypot(q[1] - p[1]);
        let n = (len / LANTERN_EVERY).floor().max(2.0) as usize;
        let at = |t: f32| {
            let y = STRING_Y - SAG * 4.0 * t * (1.0 - t);
            [p[0] + (q[0] - p[0]) * t, y, p[1] + (q[1] - p[1]) * t]
        };
        // The cord, as a thin ribbon in a few spans
        const SPANS: usize = 8;
        for s in 0..SPANS {
            let (a, c) = (at(s as f32 / SPANS as f32), at((s + 1) as f32 / SPANS as f32));
            let w = 0.008;
            h.stone.quad([[a[0], a[1] - w, a[2]], [c[0], c[1] - w, c[2]], [c[0], c[1] + w, c[2]], [a[0], a[1] + w, a[2]]], perpendicular(a, c), cord);
            h.stone.quad([[a[0], a[1] + w, a[2]], [c[0], c[1] + w, c[2]], [c[0], c[1] - w, c[2]], [a[0], a[1] - w, a[2]]], perpendicular(a, c).map(|x| -x), cord);
        }
        for i in 1..n {
            let c = at(i as f32 / n as f32);
            let r = 0.045;
            let sq = vec![[c[0] - r, c[2] - r], [c[0] + r, c[2] - r], [c[0] + r, c[2] + r], [c[0] - r, c[2] + r]];
            prism(&mut h.lamps, &sq, c[1] - 0.13, c[1] - 0.02, hex(LANTERN_GLASS), true);
        }
    }
}

/// The horizontal normal across the segment from `a` to `c`.
fn perpendicular(a: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let (dx, dz) = (c[0] - a[0], c[2] - a[2]);
    let len = dx.hypot(dz).max(1e-6);
    [-dz / len, 0.0, dx / len]
}

/// The upward normal of a sloped quad.
fn up_normal([a, b, _, d]: [[f32; 3]; 4]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
    let mut n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    if n[1] < 0.0 {
        n = n.map(|x| -x);
    }
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
    n.map(|x| x / len)
}

/// A stable hash of a name: the same shop's colours on every client.
fn name_hash(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// The colour of the goods an item type stands for on a counter: what it
/// looks like in a heap, near enough. Anything else gets a colour of its
/// own from its name.
pub fn goods_colour(item_type: &str) -> u32 {
    let has = |k: &str| item_type.contains(k);
    if has("fiber") || has("grass") || has("herb") {
        0x93ae48
    } else if has("rope") || has("twine") || has("cloth") {
        0xd8c08a
    } else if has("charcoal") || has("coal") {
        0x34302c
    } else if has("ingot") || has("nail") || has("_head") {
        0xa3abb5
    } else if has("ore") || has("stone") || has("gravel") {
        0x6b6f78
    } else if has("brick") || has("clay") || has("pot") {
        0x9b4a32
    } else if has("kit") {
        0x7a6040
    } else if has("log") || has("plank") || has("stick") || has("wood") || has("handle") || has("branch") {
        0xb08a5c
    } else if has("sickle") || has("hammer") || has("axe") || has("pick") || has("saw") || has("chisel") || has("knife") {
        0x8a8f96
    } else {
        const OTHERS: [u32; 5] = [0xc77d4a, 0x5d8aa8, 0xa86d8a, 0x7da26a, 0xc2a24a];
        OTHERS[name_hash(item_type) as usize % OTHERS.len()]
    }
}

/// The material of [`LAMPS_MESH`]: glass that gives off the lanterns' amber
/// at radiance 1 over every vertex, for the entity's `emissive_scale` to set
/// by the hour.
fn lamps_material() -> ImportedMaterial {
    let [r, g, b, _] = hex(LANTERN_LIGHT);
    ImportedMaterial {
        roughness: 0.2,
        emissive: [r, g, b],
        emissive_strength: 1.0,
        ..ImportedMaterial::vertex_colored(LAMPS_MESH)
    }
}

/// How strongly the lanterns light at this hour: the houses' lamps' rule,
/// as the hall's glass.
pub fn lantern_light(glow: crate::daylight::PlotGlow) -> f32 {
    crate::world::houses::lamp_light(glow) * LANTERN_GLOW
}

const LANTERN_GLOW: f32 = 0.9;

/// What the bazaar was last built from: its footprint, and each stall's id
/// and shop, if kept.
type Drawn = (Landmark, Vec<(Landmark, Option<ShopSign>)>);

/// The bazaar in the world: built when the terrain brings one, and again
/// when the directory changes which stalls are kept or what they sell.
#[derive(Default)]
pub struct BazaarLayer {
    drawn: Option<Drawn>,
    signs: Vec<Sign>,
    /// The bazaar's frame and its rows, for [`Self::hidden`].
    shape: Option<(Frame, Layout)>,
}

impl BazaarLayer {
    /// Where the stalls' names hang.
    pub fn signs(&self) -> &[Sign] {
        &self.signs
    }

    /// Whether the rows of stalls stand between `eye` and `at` (world
    /// metres), so a name written at `at` would show through a roof. Each
    /// row is a solid: its walls up to the roof, which slopes from the
    /// outer wall down to the eaves over the lane.
    ///
    /// A workaround: script text isn't depth-tested, so the HUD can't let
    /// the roofs hide a sign's name itself (#274).
    pub fn hidden(&self, eye: [f32; 3], at: [f32; 3]) -> bool {
        let Some((b, l)) = &self.shape else {
            return false;
        };
        let solid = |p: [f32; 3]| {
            let (u, v) = b.local([p[0], p[2]]);
            if v.abs() > b.width / 2.0 || p[1] < PAVE {
                return false;
            }
            // The roof's height at `u`, along each row's slope
            let roof = |outer: f32, lane: f32| {
                let t = ((u - outer) / (lane - outer)).clamp(0.0, 1.0);
                WALL_TOP + (EAVE - WALL_TOP) * t
            };
            (u >= 0.0 && u <= l.lane.0 && p[1] <= roof(0.0, l.lane.0))
                || (u >= l.lane.1 && u <= b.depth && p[1] <= roof(b.depth, l.lane.1))
        };
        // The last stretch is the sign's own: it hangs out over the lane
        const STEPS: usize = 48;
        (1..STEPS - 2).any(|k| {
            let t = k as f32 / STEPS as f32;
            solid(std::array::from_fn(|i| eye[i] + (at[i] - eye[i]) * t))
        })
    }

    /// Builds the bazaar if `landmarks` or `shops` (by stall id) differ from
    /// what was built, or takes it down when the terrain has none. True if
    /// it uploaded, so the meshes' entities need attaching.
    pub fn sync<'a>(
        &mut self,
        gpu: &mut ExtensionGpu,
        landmarks: &[Landmark],
        shop: impl Fn(&str) -> Option<&'a ShopSign>,
    ) -> bool {
        let Some(bazaar) = landmarks.iter().find(|l| l.kind == LandmarkKind::Bazaar) else {
            if self.drawn.take().is_some() {
                self.signs.clear();
                self.shape = None;
                upload(gpu, Bazaar::default());
                return true;
            }
            return false;
        };
        let stalls: Vec<(Landmark, Option<ShopSign>)> = landmarks
            .iter()
            .filter(|l| l.kind == LandmarkKind::Stall)
            .map(|l| (l.clone(), shop(&l.id).cloned()))
            .collect();
        let now = (bazaar.clone(), stalls);
        if self.drawn.as_ref() == Some(&now) {
            return false;
        }
        let looks: Vec<StallLook> = now.1.iter().map(|(stall, shop)| StallLook { stall, shop: shop.as_ref() }).collect();
        let built = build(&now.0, &looks);
        self.signs = built.signs.clone();
        let frame = Frame::new(&now.0);
        self.shape = Some((frame, Layout::new(&frame, &looks)));
        upload(gpu, built);
        self.drawn = Some(now);
        true
    }
}

fn upload(gpu: &mut ExtensionGpu, built: Bazaar) {
    built.stone.upload(gpu, MESH, 0.8);
    built.lamps.upload_material(gpu, lamps_material());
    built.gold.upload_material(
        gpu,
        ImportedMaterial {
            metallic: 0.85,
            roughness: 0.35,
            ..ImportedMaterial::vertex_colored(GOLD_MESH)
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::to_server;
    use crate::world::mesh::facing;
    use crate::world::mesh::probe::triangles;
    use std::path::Path;
    use uuid::Uuid;

    fn landmarks() -> Vec<Landmark> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        capitol_core::terrain::read_terrain_file(&path).expect("fixture").landmarks
    }

    fn sign(name: &str, goods: &[&str]) -> ShopSign {
        ShopSign {
            name: name.into(),
            owner_id: Uuid::from_u128(7),
            owner_name: "ada".into(),
            owner_citizen: false,
            item_types: goods.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    /// The fixture's bazaar, with every other stall kept.
    fn built(shops: &[ShopSign]) -> (Landmark, Vec<Landmark>, Bazaar) {
        let all = landmarks();
        let bazaar = all.iter().find(|l| l.kind == LandmarkKind::Bazaar).expect("the fixture has the bazaar").clone();
        let stalls: Vec<Landmark> = all.into_iter().filter(|l| l.kind == LandmarkKind::Stall).collect();
        let looks: Vec<StallLook> = stalls
            .iter()
            .enumerate()
            .map(|(i, stall)| StallLook { stall, shop: (i % 2 == 0).then(|| &shops[i / 2 % shops.len()]) })
            .collect();
        let b = build(&bazaar, &looks);
        (bazaar, stalls, b)
    }

    fn shops() -> Vec<ShopSign> {
        vec![sign("Ada's Ropes", &["rope", "fiber"]), sign("Ironmonger", &["iron_ingot", "sickle"]), sign("Bram & Daughters", &[])]
    }

    fn parts(b: &Bazaar) -> [(&str, &MeshBuf); 3] {
        [("stone", &b.stone), ("lamps", &b.lamps), ("gold", &b.gold)]
    }

    #[test]
    fn the_fixture_has_thirteen_stalls_round_a_lane_and_a_gate() {
        let all = landmarks();
        let bazaar = all.iter().find(|l| l.kind == LandmarkKind::Bazaar).unwrap();
        let stalls: Vec<_> = all.iter().filter(|l| l.kind == LandmarkKind::Stall).collect();
        assert_eq!(stalls.len(), 13);
        let b = Frame::new(bazaar);
        let looks: Vec<StallLook> = stalls.iter().map(|stall| StallLook { stall, shop: None }).collect();
        let l = Layout::new(&b, &looks);
        assert!((l.lane.0 - 2.5).abs() < 0.01 && (l.lane.1 - 5.5).abs() < 0.01, "{l:?}");
        assert!((l.gate.0 + 0.8).abs() < 0.01 && (l.gate.1 - 0.8).abs() < 0.01, "{l:?}");
        // The bazaar faces the avenue, east, as the hall faces it west
        assert!(b.out()[0] > 0.99, "{:?}", b.out());
    }

    #[test]
    fn everything_stands_on_its_footprint() {
        let (bazaar, _, b) = built(&shops());
        for (name, m) in parts(&b) {
            for v in &m.vertices {
                let [x, y] = to_server(v.position[0], v.position[2]);
                // The gate's band and the cornices stand a hand proud of the front
                let d = bazaar.distance_to(x, y);
                assert!(d < 1.05, "{name} vertex {:?} is {d} units off the footprint", v.position);
            }
        }
    }

    #[test]
    fn every_face_faces_its_normal() {
        let (_, _, b) = built(&shops());
        for (name, m) in parts(&b) {
            for v in triangles(m) {
                assert!(facing(v[0].position, v[1].position, v[2].position, v[0].normal), "{name} face at {:?}", v[0].position);
            }
        }
    }

    #[test]
    fn kept_stalls_hang_a_sign_over_the_lane_and_empty_ones_stand_bare() {
        let (bazaar, stalls, b) = built(&shops());
        assert_eq!(b.signs.len(), stalls.len());
        let bare = build(&bazaar, &stalls.iter().map(|stall| StallLook { stall, shop: None }).collect::<Vec<_>>());
        assert!(b.stone.vertices.len() > bare.stone.vertices.len() + 6 * 100, "awnings, goods and signs");
        assert!(b.lamps.vertices.len() > bare.lamps.vertices.len(), "a lantern at each kept stall");
        for (i, s) in b.signs.iter().enumerate() {
            let stall = stalls.iter().find(|l| l.id == s.stall_id).unwrap();
            let [x, y] = to_server(s.at[0], s.at[2]);
            if i % 2 == 0 {
                // Out over the lane, in front of the stall, within its range
                assert!(stall.distance_to(x, y) > 1.0, "{} hangs over the lane", s.stall_id);
                assert!(capitol_protocol::rules::bazaar::at_the_stall(stall, x, y));
                assert!(s.at[1] > SIGN.0 && s.at[1] < SIGN.1);
            } else {
                assert_eq!(stall.distance_to(x, y), 0.0, "{} over its counter", s.stall_id);
            }
        }
    }

    #[test]
    fn the_lane_and_the_gate_are_clear_to_walk() {
        // Nothing stands between the paving and head height (2 m) in a
        // 1.2 m lane from the avenue through the gate, nor down the lane's
        // crown, but the paving and the threshold.
        let (bazaar, _, b) = built(&shops());
        let f = Frame::new(&bazaar);
        let mid = (2.5 + 5.5) * 0.5;
        for (name, m) in parts(&b) {
            for tri in triangles(m) {
                let pts = tri.map(|v| (f.local([v.position[0], v.position[2]]), v.position[1]));
                let lo = |k: fn(&((f32, f32), f32)) -> f32| pts.iter().map(k).fold(f32::MAX, f32::min);
                let hi = |k: fn(&((f32, f32), f32)) -> f32| pts.iter().map(k).fold(f32::MIN, f32::max);
                let low = lo(|p| p.1) > PAVE + 0.05 && lo(|p| p.1) < G + 2.0;
                let in_gate = lo(|p| p.0 .0) < mid && hi(|p| p.0 .0) > -0.5 && hi(|p| p.0 .1) > -0.6 && lo(|p| p.0 .1) < 0.6;
                let on_crown = hi(|p| p.0 .0) > mid - 0.4 && lo(|p| p.0 .0) < mid + 0.4 && hi(|p| p.0 .1) > -4.6 && lo(|p| p.0 .1) < 4.6;
                assert!(!(low && (in_gate || on_crown)), "{name} triangle in the way: {pts:?}");
            }
        }
    }

    #[test]
    fn the_lanterns_light_by_their_material_and_nothing_tops_the_finials() {
        let (_, _, b) = built(&shops());
        assert!(!b.lamps.vertices.is_empty());
        assert!(!b.lamps.vertices.iter().any(crate::world::mesh::is_light));
        let top = parts(&b).iter().flat_map(|(_, m)| m.vertices.iter()).map(|v| v.position[1]).fold(f32::MIN, f32::max);
        assert!((top - TOP_Y).abs() < 1e-4, "{top}");
        // Lower than the hall, the civic one of the pair
        const { assert!(TOP_Y < crate::world::hall::TOP_Y) };
    }

    #[test]
    fn a_far_rows_sign_hides_behind_the_near_roof_but_not_from_over_the_lane() {
        let (bazaar, _, b) = built(&shops());
        let f = Frame::new(&bazaar);
        let looks: Vec<StallLook> = Vec::new();
        let layer = BazaarLayer { shape: Some((f, Layout::new(&f, &looks))), ..Default::default() };
        // A kept back-row stall's sign, out over the lane on its side
        let sign = b.signs.iter().find(|s| s.stall_id == "bazaar_stall_9").unwrap().at;
        assert!(f.local([sign[0], sign[2]]).0 < 5.5, "over the lane");
        // From low out on the avenue, the front row is in the way
        let avenue = f.at(-6.0, 0.0);
        assert!(layer.hidden([avenue[0], G + 2.0, avenue[1]], sign));
        // From high over the lane, nothing is
        let above = f.at(4.0, 0.0);
        assert!(!layer.hidden([above[0], G + 30.0, above[1]], sign));
        // Nor from the lane's crown at eye height
        let crown = f.at(4.0, 2.0);
        assert!(!layer.hidden([crown[0], G + 1.7, crown[1]], sign));
    }

    #[test]
    fn a_shop_keeps_its_colour_and_goods_look_like_themselves() {
        assert_eq!(name_hash("Ada's Ropes"), name_hash("Ada's Ropes"));
        assert_eq!(goods_colour("fiber"), 0x93ae48);
        assert_eq!(goods_colour("iron_ore"), 0x6b6f78);
        assert_eq!(goods_colour("iron_ingot"), 0xa3abb5);
        assert_eq!(goods_colour("workbench_kit"), 0x7a6040);
        assert_eq!(goods_colour("plank"), 0xb08a5c);
        assert_eq!(goods_colour("mystery"), goods_colour("mystery"));
    }

    #[test]
    fn it_is_cheap_to_draw() {
        let (_, _, b) = built(&shops());
        let triangles: usize = parts(&b).iter().map(|(_, m)| m.indices.len() / 3).sum();
        println!("bazaar: {triangles} triangles");
        assert!(triangles < 8_000, "{triangles} triangles");
    }
}
