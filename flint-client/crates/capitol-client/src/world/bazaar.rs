//! The bazaar (E3.11, E3.12): merchants' own shops, in stalls along the
//! streets of the bazaar quarter, the south-west of the Trade District
//! (economy plan D6). Each stall is built on its own footprint from
//! `/terrain`, facing its street, and the quarter's mouth on the avenue, the
//! `bazaar` landmark, opposite the Exchange hall, is an arch.
//!
//! This is the quarter's first look, kept from E3.11's single building
//! while its street of many buildings is made (E3.14, #279). The hall and
//! the bazaar are a pair: the same stone and the same gold, but where the
//! hall is the city's one book, cool and ordered behind Trade-blue glass,
//! the bazaar is many people's, warm and coloured:
//!
//! - **every stall** has a paved floor, a stone back wall and a tiled roof
//!   sloping down to its street, and timber partitions at its sides;
//! - **a kept stall** has its counter stocked with goods (coloured by the
//!   kinds the shop sells, as the directory names them), more on a shelf
//!   behind, a **striped awning** out over its street in the shop's own
//!   colour, a **sign** hanging from it, where the HUD writes the shop's
//!   name, and a lantern at its corner; and on the outside of its back wall
//!   a **banner** in its colour under a sconce. So the quarter lights up at
//!   night as merchants move in;
//! - **an empty stall** stands bare: an empty counter, its awning furled
//!   under the eave, and shutters on its back wall;
//! - **the mouth**: two piers with gold finials, a lintel with a gold band
//!   facing the hall, and a lantern under it.
//!
//! The lanterns light at dusk (H23, `material.emissive_scale` on
//! [`LAMPS_MESH`]). Three meshes, as the hall's: [`MESH`] (stone, timber,
//! tile and cloth), [`LAMPS_MESH`] (the glass that lights) and [`GOLD_MESH`]
//! (gold metal). They're rebuilt when the terrain or the directory changes
//! which stalls are kept, or what they sell.

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
/// The top of everything: the mouth's finials.
pub const TOP_Y: f32 = PIER_TOP + 0.45;

// Materials: the hall's stone (`world::hall`), so the two read as the
// Capitol's, and warm tile and timber for the market.
const STONE: u32 = 0xa79f90;
const PALE_STONE: u32 = 0xc8bfae;
const PAVING: u32 = 0x928879;
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

/// Builds the bazaar: the mouth on `bazaar`, and its `stalls` as they're kept.
pub fn build(bazaar: &Landmark, stalls: &[StallLook]) -> Bazaar {
    let mut out = Bazaar::default();
    mouth(&mut out, &Frame::new(bazaar));
    for s in stalls {
        stall(&mut out, s);
    }
    out
}

/// The mouth on the avenue: two piers with gold finials, a lintel with a
/// gold band facing the hall, and a lantern hanging under it. The passage
/// between the piers is the lane's own width, less the piers.
fn mouth(h: &mut Bazaar, b: &Frame) {
    let pale = hex(PALE_STONE);
    let half = b.width / 2.0;
    let u = (b.depth * 0.2).min(0.32);
    for v in [-half + 0.25, half - 0.25] {
        prism(&mut h.stone, &b.square(u, v, 0.28, 0.22), PAVE, PIER_TOP, pale, true);
        prism(&mut h.stone, &b.square(u, v, 0.3, 0.25), PIER_TOP, PIER_TOP + 0.1, hex(STONE), true);
        loft(&mut h.gold, &b.square(u, v, 0.1, 0.1), PIER_TOP + 0.1, &b.square(u, v, 0.015, 0.015), TOP_Y, hex(GOLD), false);
    }
    prism(&mut h.stone, &b.rect(u - 0.3, u + 0.3, -half + 0.03, half - 0.03), LINTEL.0, LINTEL.1, hex(STONE), true);
    // The gold band on the lintel's face, toward the avenue and the hall, as
    // the hall's rules run round its ticker
    let (p, q) = (b.at(u - 0.305, -half + 0.08), b.at(u - 0.305, half - 0.08));
    let (y0, y1) = (LINTEL.0 + 0.16, LINTEL.0 + 0.27);
    h.gold.quad([lift(p, y0), lift(q, y0), lift(q, y1), lift(p, y1)], b.out(), hex(GOLD));
    // A threshold of pale stone, and the lantern on a short chain
    prism(&mut h.stone, &b.rect(u - 0.3, u + 0.3, -half + 0.47, half - 0.47), PAVE, PAVE + 0.04, pale, true);
    let c = b.at(u, 0.0);
    prism(&mut h.gold, &[[c[0] - 0.01, c[1] - 0.01], [c[0] + 0.01, c[1] - 0.01], [c[0] + 0.01, c[1] + 0.01], [c[0] - 0.01, c[1] + 0.01]], LINTEL.0 - 0.3, LINTEL.0, hex(CORD), false);
    lantern(h, c, LINTEL.0 - 0.3, 0.1);
}

/// A stall's shell: its floor, its back wall, and a tiled roof from the
/// wall's top down to the eave over its counter.
fn shell(h: &mut Bazaar, f: &Frame) {
    const THICK: f32 = 0.18;
    let (w0, w1) = (-f.width / 2.0, f.width / 2.0);
    prism(&mut h.stone, &f.rect(0.0, f.depth, w0, w1), G - 0.02, PAVE, hex(PAVING), true);
    prism(&mut h.stone, &f.rect(f.depth - THICK, f.depth, w0, w1), PAVE, WALL_TOP, hex(STONE), true);
    prism(&mut h.stone, &f.rect(f.depth - THICK - 0.02, f.depth, w0, w1), WALL_TOP - 0.15, WALL_TOP, hex(PALE_STONE), true);
    roof(h, f, (f.depth, WALL_TOP + 0.08), (0.0, EAVE), (w0, w1));
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
    let toward_street = if eave.0 > top.0 { b.back() } else { b.out() };
    let (p, q) = (b.at(eave.0, v0), b.at(eave.0, v1));
    h.stone.quad([lift(p, eave.1 - 0.12), lift(q, eave.1 - 0.12), lift(q, eave.1), lift(p, eave.1)], toward_street, hex(TIMBER));
}

/// A stall: its partitions and counter, and kept, its goods, awning, sign
/// and lantern; empty, its awning furled.
fn stall(h: &mut Bazaar, s: &StallLook) {
    let f = Frame::new(s.stall);
    let (w0, w1) = (-f.width / 2.0, f.width / 2.0);
    let timber = hex(TIMBER);
    shell(h, &f);
    // The partitions at its sides, under the roof
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
    // A crate out front, at the counter's foot
    let colour = hex(kinds[seed as usize % kinds.len()]);
    let c = (w0 + w1) * 0.5 + (w1 - w0) * 0.25 * if seed.is_multiple_of(2) { 1.0 } else { -1.0 };
    prism(&mut h.stone, &f.square(-0.13, c, 0.12, 0.12), PAVE, PAVE + 0.28, hex(TIMBER), true);
    prism(&mut h.stone, &f.square(-0.13, c, 0.09, 0.09), PAVE + 0.28, PAVE + 0.33, colour, true);
}

/// The awning: stripes in the shop's colour and cloth, sloping out and down
/// from the eave over the street, both faces, with a valance on its edge.
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

/// How strongly the lanterns light at this hour: a fixed lamp's rule
/// (`PlotGlow::lamp_light`), as the hall's glass.
pub fn lantern_light(glow: crate::daylight::PlotGlow) -> f32 {
    glow.lamp_light() * LANTERN_GLOW
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
    /// Each stall's frame, for [`Self::hidden`].
    frames: Vec<Frame>,
}

impl BazaarLayer {
    /// Where the stalls' names hang.
    pub fn signs(&self) -> &[Sign] {
        &self.signs
    }

    /// Every stall as a solid, up to its back wall's top, for the camera to
    /// keep out of (E3.13).
    pub fn solids(&self) -> impl Iterator<Item = crate::world::solids::Solid> + '_ {
        self.frames.iter().map(|f| {
            let r = f.rect(0.0, f.depth, -f.width / 2.0, f.width / 2.0);
            crate::world::solids::Solid { corners: [r[0], r[1], r[2], r[3]], bottom: PAVE, top: WALL_TOP }
        })
    }

    /// Whether a stall stands between `eye` and `at` (world metres), so a
    /// name written at `at` would show through its roof. Each stall is a
    /// solid: its floor up to its roof, which slopes from its back wall down
    /// to the eave over its counter.
    ///
    /// A workaround: script text isn't depth-tested, so the HUD can't let
    /// the roofs hide a sign's name itself (#274).
    pub fn hidden(&self, eye: [f32; 3], at: [f32; 3]) -> bool {
        let solid = |p: [f32; 3]| {
            p[1] >= PAVE
                && self.frames.iter().any(|f| {
                    let (u, v) = f.local([p[0], p[2]]);
                    let roof = EAVE + (WALL_TOP - EAVE) * (u / f.depth).clamp(0.0, 1.0);
                    (0.0..=f.depth).contains(&u) && v.abs() <= f.width / 2.0 && p[1] <= roof
                })
        };
        // The last stretch is the sign's own: it hangs out over its street
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
                self.frames.clear();
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
        self.frames = looks.iter().map(|l| Frame::new(l.stall)).collect();
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
    fn the_fixture_has_the_quarter_and_its_mouth_faces_the_avenue() {
        let all = landmarks();
        let mouth = all.iter().find(|l| l.kind == LandmarkKind::Bazaar).unwrap();
        let stalls: Vec<_> = all.iter().filter(|l| l.kind == LandmarkKind::Stall).collect();
        assert!(stalls.len() >= 34, "{} stalls: refresh the fixture", stalls.len());
        // The mouth faces the avenue, east, as the hall faces it west
        assert!(Frame::new(mouth).out()[0] > 0.99, "{:?}", Frame::new(mouth).out());
    }

    /// Everything stands on a stall's footprint or the mouth's, or its gap to
    /// its street, but what overhangs the street (its awning, sign, lantern
    /// and roof's eave), which is all over head height.
    #[test]
    fn everything_stands_on_a_footprint_or_overhangs_its_street_overhead() {
        let (mouth, stalls, b) = built(&shops());
        let feet: Vec<&Landmark> = stalls.iter().chain(std::iter::once(&mouth)).collect();
        for (name, m) in parts(&b) {
            for v in &m.vertices {
                let [x, y] = to_server(v.position[0], v.position[2]);
                let d = feet.iter().map(|l| l.distance_to(x, y)).fold(f64::MAX, f64::min);
                assert!(d < 8.0, "{name} vertex {:?} is {d} units off every footprint", v.position);
                // A stall's own ground runs to its street's edge, 3 units out
                if d > 3.0 {
                    assert!(v.position[1] > G + 1.55, "{name} vertex {:?} stands in the street, {d} off", v.position);
                }
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
    fn kept_stalls_hang_a_sign_over_their_street_and_empty_ones_stand_bare() {
        let (bazaar, stalls, b) = built(&shops());
        assert_eq!(b.signs.len(), stalls.len());
        let bare = build(&bazaar, &stalls.iter().map(|stall| StallLook { stall, shop: None }).collect::<Vec<_>>());
        assert!(b.stone.vertices.len() > bare.stone.vertices.len() + 6 * 100, "awnings, goods and signs");
        assert!(b.lamps.vertices.len() > bare.lamps.vertices.len(), "a lantern at each kept stall");
        for (i, s) in b.signs.iter().enumerate() {
            let stall = stalls.iter().find(|l| l.id == s.stall_id).unwrap();
            let [x, y] = to_server(s.at[0], s.at[2]);
            if i % 2 == 0 {
                // Out over its street, in front of the stall, within its range
                assert!(stall.distance_to(x, y) > 1.0, "{} hangs over its street", s.stall_id);
                assert!(capitol_protocol::rules::bazaar::at_the_stall(stall, x, y));
                assert!(s.at[1] > SIGN.0 && s.at[1] < SIGN.1);
            } else {
                assert_eq!(stall.distance_to(x, y), 0.0, "{} over its counter", s.stall_id);
            }
        }
    }

    #[test]
    fn the_mouth_is_clear_to_walk_through() {
        // Nothing stands between the paving and head height (2 m) in a
        // 1.2 m passage through the mouth, but the threshold.
        let (mouth, _, b) = built(&shops());
        let f = Frame::new(&mouth);
        for (name, m) in parts(&b) {
            for tri in triangles(m) {
                let pts = tri.map(|v| (f.local([v.position[0], v.position[2]]), v.position[1]));
                let lo = |k: fn(&((f32, f32), f32)) -> f32| pts.iter().map(k).fold(f32::MAX, f32::min);
                let hi = |k: fn(&((f32, f32), f32)) -> f32| pts.iter().map(k).fold(f32::MIN, f32::max);
                let low = lo(|p| p.1) > PAVE + 0.05 && lo(|p| p.1) < G + 2.0;
                let in_mouth = lo(|p| p.0 .0) < f.depth && hi(|p| p.0 .0) > -0.5 && hi(|p| p.0 .1) > -0.6 && lo(|p| p.0 .1) < 0.6;
                assert!(!(low && in_mouth), "{name} triangle in the way: {pts:?}");
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
    fn a_sign_hides_behind_its_stall_but_not_from_over_its_street() {
        let (_, stalls, b) = built(&shops());
        let looks: Vec<StallLook> = stalls.iter().map(|stall| StallLook { stall, shop: None }).collect();
        let layer = BazaarLayer { frames: looks.iter().map(|l| Frame::new(l.stall)).collect(), ..Default::default() };
        let stall = &stalls[0];
        let f = Frame::new(stall);
        let sign = b.signs.iter().find(|s| s.stall_id == stall.id).unwrap().at;
        assert!(f.local([sign[0], sign[2]]).0 < 0.0, "out over its street");
        // From low behind the stall, the stall is in the way
        let behind = f.at(f.depth + 3.0, 0.0);
        assert!(layer.hidden([behind[0], G + 1.5, behind[1]], sign));
        // From high over its street, nothing is
        let above = f.at(-1.5, 0.0);
        assert!(!layer.hidden([above[0], G + 30.0, above[1]], sign));
        // Nor from its street at eye height
        let crown = f.at(-1.4, 1.0);
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
        assert!(triangles < 16_000, "{triangles} triangles");
    }
}
