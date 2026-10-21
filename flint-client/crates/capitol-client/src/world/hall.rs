//! The Exchange hall (E3.7): where the order book lives, on the east side of
//! the Capitol avenue (E3.4), built on the footprint `/terrain` gives it.
//!
//! A small civic building in the tower's materials, read from the avenue:
//!
//! - a **podium** of pale stone, three steps up from a paved forecourt;
//! - a **portico** of six columns carrying an entablature, and round it a
//!   **ticker band**: a line of lit segments in the Trade District's blue,
//!   white and gold, like a board of prices;
//! - over the portico, an **attic with a clock** whose hands keep the
//!   game's hour (the hands are rebuilt each game minute);
//! - the **hall** behind, with tall Trade-blue windows between pilasters,
//!   and on its roof a **lantern** of the same glass. By day the glass is a
//!   deep blue; at dusk it lights (H23, `material.emissive_scale` on
//!   [`LAMPS_MESH`], by the hour, as the houses' windows do), so from the
//!   map the hall is a blue gem on the avenue;
//! - **three market stalls** in the forecourt under striped awnings, their
//!   counters stacked with goods, and in the fourth's place, on the side
//!   nearer the tower, the **Gazette's notice board** (E3.9): a green
//!   board under a little roof, the day's pages pinned to it and a lamp
//!   over them that lights at dusk.
//!
//! The design is drawn for an 8 × 10 m footprint (80 × 100 server units)
//! and scaled to the one given, so a moved or resized hall still fits it.
//! `u` runs in from the entrance edge (0 at the door's side, 8 at the
//! back), `v` along it (-5 to 5).
//!
//! Three meshes: [`MESH`] (stone, timber and cloth), [`LAMPS_MESH`] (the
//! glass that lights) and [`GOLD_MESH`] (gold metal, the clock and the
//! ticker's light, whose vertices give off their colour: `palette::glow`).

use crate::coords::{layer, to_world};
use crate::world::mesh::MeshBuf;
use crate::world::palette::{glow, hex, mix};
use crate::world::tower::{lift, loft, prism, V2};
use capitol_protocol::terrain::Landmark;
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;

/// The runtime meshes' names, as their entities are named.
pub const MESH: &str = "exchange_hall";
pub const LAMPS_MESH: &str = "exchange_hall_lamps";
pub const GOLD_MESH: &str = "exchange_hall_gold";

/// Every mesh, for attaching their entities.
pub fn meshes() -> [String; 3] {
    [MESH, LAMPS_MESH, GOLD_MESH].map(str::to_owned)
}

const G: f32 = layer::GROUND;
/// The forecourt's paving: level with the lots' tops.
const PAVE: f32 = layer::PLOT;
const STYLOBATE: f32 = G + 0.75;
const COLUMN_TOP: f32 = G + 5.0;
const CAPITAL_TOP: f32 = G + 5.25;
const ENTABLATURE_TOP: f32 = G + 6.05;
const CORNICE_TOP: f32 = G + 6.25;
const TICKER: (f32, f32) = (G + 5.5, G + 5.8);
const ATTIC_TOP: f32 = G + 8.0;
const CLOCK_Y: f32 = G + 7.12;
const CLOCK_R: f32 = 0.66;
const LANTERN_TOP: f32 = G + 7.5;
/// The top of the hall: the lantern's roof slab. The mote rises from here.
pub const ROOF_Y: f32 = LANTERN_TOP + 0.15;
/// The top of everything, the clock's gold cap.
pub const TOP_Y: f32 = ATTIC_TOP + 0.12;

// Materials. The tower's stone, so the hall reads as the Capitol's.
const STONE: u32 = 0xa79f90;
const PALE_STONE: u32 = 0xc8bfae;
const PAVING: u32 = 0x8d877c;
const STEP: u32 = 0xb3aa98;
const DOOR: u32 = 0x2b2a33;
/// The glass: the Trade District's blue (#3b82f6) gone deep, so it takes the
/// sky by day, and the light it gives at night.
const GLASS: u32 = 0x1c2f55;
pub const GLASS_LIGHT: u32 = 0x4a86f0;
const TRADE: u32 = 0x3b82f6;
const GOLD: u32 = crate::world::palette::GOLD;
const BRONZE: u32 = 0x2a2418;
const CLOCK_FACE: u32 = 0xfff1dc;
const TIMBER: u32 = crate::world::palette::TIMBER;
const CLOTH: u32 = 0xe9e1cf;
/// The awnings' stripes, one per stall, with the cloth between.
const AWNINGS: [u32; 4] = [TRADE, 0xc9a227, 0x2f7f6f, 0x9b3b3b];
/// The notice board's green, and its pages' paper and ink.
const BOARD: u32 = 0x2d3b30;
const PAGE: u32 = 0xf0e6cf;
const PAGE_INK: u32 = 0x3a332a;
/// The stalls' slots, along the front either side of the steps. The board
/// takes the outer one on the tower's side.
const SLOTS: [(f32, f32); 4] = [(-4.65, -3.7), (-3.45, -2.5), (2.5, 3.45), (3.7, 4.65)];
/// How far in from the entrance edge the stalls and the board stand.
const STALL_U: f32 = 0.55;
/// The board's top, its roof's.
const BOARD_TOP: f32 = PAVE + 2.25;
/// What's for sale on the counters: fiber, timber, ore, brick, rope.
const GOODS: [u32; 5] = [0x93ae48, 0xb08a5c, 0x6b6f78, 0x9b4a32, 0xd8c08a];

/// The hall's footprint as world metres: the frame its design is drawn in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Site {
    /// The middle of the entrance edge.
    door: V2,
    /// Into the hall from the door.
    f: V2,
    /// Along the entrance edge.
    a: V2,
    /// The footprint's depth over 8 m and frontage over 10 m.
    su: f32,
    sv: f32,
}

impl Site {
    pub fn new(hall: &Landmark) -> Self {
        let w = |p: (f64, f64)| to_world(p.0, p.1);
        let (i, j) = hall.entrance_edge;
        let (e0, e1) = (w(hall.corners[i % 4]), w(hall.corners[j % 4]));
        let door = [(e0[0] + e1[0]) * 0.5, (e0[1] + e1[1]) * 0.5];
        let centre = w(hall.position);
        let inward = [centre[0] - door[0], centre[1] - door[1]];
        let depth = 2.0 * inward[0].hypot(inward[1]);
        let along = [e1[0] - e0[0], e1[1] - e0[1]];
        let frontage = along[0].hypot(along[1]);
        let unit = |d: V2, len: f32| [d[0] / len.max(1e-6), d[1] / len.max(1e-6)];
        Self {
            door,
            f: unit(inward, depth * 0.5),
            a: unit(along, frontage),
            su: depth / 8.0,
            sv: frontage / 10.0,
        }
    }

    /// The design's (u, v) on the ground.
    fn at(&self, u: f32, v: f32) -> V2 {
        let (u, v) = (u * self.su, v * self.sv);
        [
            self.door[0] + self.f[0] * u + self.a[0] * v,
            self.door[1] + self.f[1] * u + self.a[1] * v,
        ]
    }

    fn rect(&self, u0: f32, u1: f32, v0: f32, v1: f32) -> Vec<V2> {
        vec![
            self.at(u0, v0),
            self.at(u1, v0),
            self.at(u1, v1),
            self.at(u0, v1),
        ]
    }

    /// A circle of radius `r` m (not scaled) round (u, v).
    fn circle(&self, u: f32, v: f32, r: f32, sides: usize) -> Vec<V2> {
        let c = self.at(u, v);
        (0..sides)
            .map(|k| {
                let t = k as f32 / sides as f32 * std::f32::consts::TAU;
                [c[0] + r * t.cos(), c[1] + r * t.sin()]
            })
            .collect()
    }

    /// Out toward the avenue, the way the front faces.
    fn front(&self) -> [f32; 3] {
        [-self.f[0], 0.0, -self.f[1]]
    }

    /// Along the edge, as a normal: `sign` picks the side.
    fn side(&self, sign: f32) -> [f32; 3] {
        [self.a[0] * sign, 0.0, self.a[1] * sign]
    }

    /// The middle of the lantern's roof, world (X, Y, Z): where a mote
    /// rises from.
    pub fn roof(&self) -> [f32; 3] {
        let [x, z] = self.at(5.5, 0.0);
        [x, ROOF_Y, z]
    }

    /// The door's middle, where the prompt hangs.
    pub fn door(&self) -> V2 {
        self.door
    }

    /// The slot the Gazette's board takes: the outer one on the side
    /// nearer the tower, at the world's centre.
    fn board_slot(&self) -> usize {
        let c = self.at(STALL_U, 0.0);
        let toward = -(self.a[0] * c[0] + self.a[1] * c[1]);
        if toward >= 0.0 {
            3
        } else {
            0
        }
    }

    /// The Gazette's notice board (E3.9): where a click picks it, round
    /// its slot, and the middle of its roof, world (X, Y, Z), where its
    /// prompt hangs.
    pub fn board(&self) -> ([V2; 4], [f32; 3]) {
        let (v0, v1) = SLOTS[self.board_slot()];
        let r = self.rect(STALL_U - 0.45, STALL_U + 0.45, v0, v1);
        let [x, z] = self.at(STALL_U, (v0 + v1) * 0.5);
        ([r[0], r[1], r[2], r[3]], [x, BOARD_TOP, z])
    }
}

/// The hall, built: one buffer per material, plus the light.
#[derive(Default)]
pub struct Hall {
    pub stone: MeshBuf,
    pub lamps: MeshBuf,
    pub gold: MeshBuf,
    /// Made to glow when it's merged into the gold mesh.
    pub light: MeshBuf,
}

/// Builds the hall on `site`, its clock at `phase` of the day (0 midnight,
/// 0.5 noon).
pub fn build(site: &Site, phase: f64) -> Hall {
    let mut h = Hall::default();
    forecourt(&mut h, site);
    portico(&mut h, site);
    cella(&mut h, site);
    lantern(&mut h, site);
    clock(&mut h, site, phase);
    let board = site.board_slot();
    for (k, &(v0, v1)) in SLOTS.iter().enumerate() {
        if k == board {
            notice_board(&mut h, site, STALL_U, v0, v1);
        } else {
            stall(&mut h, site, STALL_U, v0, v1, k);
        }
    }
    h
}

/// The Gazette's notice board (E3.9): two posts, a green board between
/// them under a little roof, the edition's pages pinned to its front, and a
/// lamp over them.
fn notice_board(h: &mut Hall, s: &Site, uc: f32, v0: f32, v1: f32) {
    let timber = hex(TIMBER);
    for v in [v0 + 0.08, v1 - 0.08] {
        prism(&mut h.stone, &square(s, uc, v, 0.05), PAVE, BOARD_TOP - 0.05, timber, true);
    }
    let (b0, b1) = (PAVE + 0.75, PAVE + 1.9);
    prism(&mut h.stone, &s.rect(uc - 0.04, uc + 0.04, v0 + 0.13, v1 - 0.13), b0, b1, hex(BOARD), true);
    prism(&mut h.stone, &s.rect(uc - 0.2, uc + 0.2, v0 - 0.02, v1 + 0.02), BOARD_TOP - 0.05, BOARD_TOP, timber, true);
    // The pages: the front page, tall, with its headline, and two smaller
    // sheets beside it, standing a hair proud of the board's face.
    let n = s.front();
    let face = uc - 0.05;
    let sheet = |h: &mut Hall, a: f32, b: f32, y0: f32, y1: f32, colour: u32| {
        let (p, q) = (s.at(face, a), s.at(face, b));
        h.stone.quad([lift(p, y0), lift(q, y0), lift(q, y1), lift(p, y1)], n, hex(colour));
    };
    let mid = (v0 + v1) * 0.5;
    sheet(h, mid - 0.2, mid + 0.12, b0 + 0.12, b1 - 0.1, PAGE);
    // The headline and the columns under it, as lines of ink
    let ink = |h: &mut Hall, a: f32, b: f32, y: f32, t: f32| {
        let (p, q) = (s.at(face - 0.005, a), s.at(face - 0.005, b));
        h.stone.quad([lift(p, y), lift(q, y), lift(q, y + t), lift(p, y + t)], n, hex(PAGE_INK));
    };
    ink(h, mid - 0.16, mid + 0.08, b1 - 0.24, 0.07);
    for k in 0..5 {
        let y = b1 - 0.36 - 0.12 * k as f32;
        ink(h, mid - 0.16, mid - 0.05, y, 0.025);
        ink(h, mid - 0.02, mid + 0.08, y, 0.025);
    }
    let (left, right) = if v1 > 0.0 { (v0 + 0.17, mid - 0.24) } else { (mid + 0.16, v1 - 0.17) };
    sheet(h, left, right, b0 + 0.45, b1 - 0.25, PAGE);
    sheet(h, left, right, b0 + 0.12, b0 + 0.38, 0xe6d9b8);
    // The lamp, out on a bracket over the pages: it lights at dusk with
    // the hall's gold.
    let [x, z] = s.at(uc - 0.2, mid);
    let lamp = [[x - 0.06, z - 0.06], [x + 0.06, z - 0.06], [x + 0.06, z + 0.06], [x - 0.06, z + 0.06]];
    prism(&mut h.light, &lamp, BOARD_TOP - 0.22, BOARD_TOP - 0.1, glow(hex(CLOCK_FACE), 1.3), true);
    prism(&mut h.gold, &s.rect(uc - 0.2, uc - 0.04, mid - 0.015, mid + 0.015), BOARD_TOP - 0.1, BOARD_TOP - 0.08, hex(BRONZE), true);
}

/// The paving over the whole footprint, the podium and its steps.
fn forecourt(h: &mut Hall, s: &Site) {
    prism(
        &mut h.stone,
        &s.rect(0.0, 8.0, -5.0, 5.0),
        G - 0.02,
        PAVE,
        hex(PAVING),
        true,
    );
    prism(
        &mut h.stone,
        &s.rect(1.9, 7.85, -4.75, 4.75),
        PAVE,
        STYLOBATE,
        hex(STEP),
        true,
    );
    let rise = (STYLOBATE - PAVE) / 3.0;
    for k in 0..3 {
        let u0 = 1.0 + 0.3 * k as f32;
        let top = PAVE + rise * (k + 1) as f32;
        prism(
            &mut h.stone,
            &s.rect(u0, 1.9, -2.3, 2.3),
            PAVE,
            top,
            hex(STEP),
            true,
        );
    }
}

/// The columns, the entablature with its ticker band, and the cornice.
fn portico(h: &mut Hall, s: &Site) {
    let pale = hex(PALE_STONE);
    // The middle bay is the widest, framing the door.
    for v in [-4.3, -2.65, -1.0, 1.0, 2.65, 4.3] {
        prism(
            &mut h.stone,
            &square(s, 2.35, v, 0.36),
            STYLOBATE,
            STYLOBATE + 0.18,
            pale,
            true,
        );
        prism(
            &mut h.stone,
            &s.circle(2.35, v, 0.26, 12),
            STYLOBATE + 0.18,
            COLUMN_TOP,
            pale,
            false,
        );
        prism(
            &mut h.stone,
            &square(s, 2.35, v, 0.38),
            COLUMN_TOP,
            CAPITAL_TOP,
            pale,
            true,
        );
    }
    prism(
        &mut h.stone,
        &s.rect(1.95, 7.8, -4.7, 4.7),
        CAPITAL_TOP,
        ENTABLATURE_TOP,
        hex(STONE),
        false,
    );
    prism(
        &mut h.stone,
        &s.rect(1.85, 7.9, -4.85, 4.85),
        ENTABLATURE_TOP,
        CORNICE_TOP,
        pale,
        true,
    );
    ticker(h, s);
}

/// A square of half-side `half` m round (u, v), turned with the hall.
fn square(s: &Site, u: f32, v: f32, half: f32) -> Vec<V2> {
    let c = s.at(u, v);
    let (f, a) = (s.f, s.a);
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|&(du, dv)| {
            [
                c[0] + (f[0] * du + a[0] * dv) * half,
                c[1] + (f[1] * du + a[1] * dv) * half,
            ]
        })
        .collect()
}

/// The ticker: segments of light along the entablature's front and both
/// sides, each lit or dark, blue, white or gold by a hash of where it is,
/// so the band reads as a board of prices.
fn ticker(h: &mut Hall, s: &Site) {
    const SEG: f32 = 0.2;
    const GAP: f32 = 0.06;
    const PROUD: f32 = 0.025;
    let (y0, y1) = TICKER;
    let run = |h: &mut Hall, from: V2, to: V2, normal: [f32; 3], seed: u32| {
        let along = [to[0] - from[0], to[1] - from[1]];
        let len = along[0].hypot(along[1]);
        let n = (len / (SEG + GAP)).floor() as usize;
        let pad = (len - n as f32 * (SEG + GAP) + GAP) * 0.5;
        let off = [normal[0] * PROUD, normal[2] * PROUD];
        for k in 0..n {
            let hsh = crate::world::tower::hash(seed, k as u32, 7);
            if hsh % 100 >= 72 {
                continue;
            }
            let colour = match (hsh >> 8) % 6 {
                0 => hex(GOLD),
                1 => hex(CLOCK_FACE),
                _ => hex(GLASS_LIGHT),
            };
            let radiance = 1.6 + 1.4 * ((hsh >> 16) % 100) as f32 / 100.0;
            let d0 = pad + k as f32 * (SEG + GAP);
            let at = |d: f32| {
                [
                    from[0] + along[0] / len * d + off[0],
                    from[1] + along[1] / len * d + off[1],
                ]
            };
            let (p, q) = (at(d0), at(d0 + SEG));
            h.light.quad(
                [lift(p, y0), lift(q, y0), lift(q, y1), lift(p, y1)],
                normal,
                glow(colour, radiance),
            );
        }
    };
    run(h, s.at(1.95, -4.7), s.at(1.95, 4.7), s.front(), 1);
    run(h, s.at(1.95, 4.7), s.at(7.8, 4.7), s.side(1.0), 2);
    run(h, s.at(1.95, -4.7), s.at(7.8, -4.7), s.side(-1.0), 3);
    // A thin gold rule above and below the band.
    for (y, t) in [(y0 - 0.06, y0 - 0.03), (y1 + 0.03, y1 + 0.06)] {
        for (a, b, n) in [
            (s.at(1.95, -4.7), s.at(1.95, 4.7), s.front()),
            (s.at(1.95, 4.7), s.at(7.8, 4.7), s.side(1.0)),
            (s.at(1.95, -4.7), s.at(7.8, -4.7), s.side(-1.0)),
        ] {
            let off = [n[0] * PROUD, n[2] * PROUD];
            let (a, b) = (
                [a[0] + off[0], a[1] + off[1]],
                [b[0] + off[0], b[1] + off[1]],
            );
            h.gold.quad(
                [lift(a, y), lift(b, y), lift(b, t), lift(a, t)],
                n,
                hex(GOLD),
            );
        }
    }
}

/// The hall itself: walls, pilasters, the door, and the tall windows.
fn cella(h: &mut Hall, s: &Site) {
    let stone = hex(STONE);
    prism(
        &mut h.stone,
        &s.rect(3.4, 7.6, -4.5, 4.5),
        STYLOBATE,
        CAPITAL_TOP,
        stone,
        false,
    );
    // Pilasters down each side, between the windows.
    for u in [3.55, 4.85, 6.15, 7.45] {
        for v in [-4.55, 4.55] {
            prism(
                &mut h.stone,
                &s.rect(u - 0.12, u + 0.12, v - 0.08, v + 0.08),
                STYLOBATE,
                CAPITAL_TOP,
                hex(PALE_STONE),
                false,
            );
        }
    }
    let (sill, head) = (G + 1.4, G + 4.6);
    const PROUD: f32 = 0.015;
    // A pane centred at (u, v) on a face looking along `n`, `half` wide
    // either way along `across`.
    let pane =
        |h: &mut Hall, u: f32, v: f32, n: [f32; 3], across: V2, half: f32, y0: f32, y1: f32| {
            let c = s.at(u, v);
            let o = [c[0] + n[0] * PROUD, c[1] + n[2] * PROUD];
            let p = [o[0] - across[0] * half, o[1] - across[1] * half];
            let q = [o[0] + across[0] * half, o[1] + across[1] * half];
            h.lamps.quad(
                [lift(p, y0), lift(q, y0), lift(q, y1), lift(p, y1)],
                n,
                hex(GLASS),
            );
        };
    for u in [4.2, 5.5, 6.8] {
        pane(h, u, 4.5, s.side(1.0), s.f, 0.4, sill, head);
        pane(h, u, -4.5, s.side(-1.0), s.f, 0.4, sill, head);
    }
    let back = [s.f[0], 0.0, s.f[1]];
    for v in [-2.6, 0.0, 2.6] {
        pane(h, 7.6, v, back, s.a, 0.45, sill, head);
    }
    for v in [-2.6, 2.6] {
        pane(h, 3.4, v, s.front(), s.a, 0.45, sill, head);
    }
    // The door, and the lit transom over it.
    pane(h, 3.4, 0.0, s.front(), s.a, 0.8, G + 3.65, G + 4.55);
    let n = s.front();
    let c = s.at(3.4, 0.0);
    let o = [c[0] + n[0] * PROUD, c[1] + n[2] * PROUD];
    let (p, q) = (
        [o[0] - s.a[0] * 0.75, o[1] - s.a[1] * 0.75],
        [o[0] + s.a[0] * 0.75, o[1] + s.a[1] * 0.75],
    );
    h.stone.quad(
        [
            lift(p, STYLOBATE),
            lift(q, STYLOBATE),
            lift(q, G + 3.5),
            lift(p, G + 3.5),
        ],
        n,
        hex(DOOR),
    );
    // Its frame, in gold.
    for (y0, y1) in [(G + 3.5, G + 3.62), (G + 4.55, G + 4.65)] {
        let (p, q) = (
            [
                o[0] - s.a[0] * 0.85 + n[0] * 0.01,
                o[1] - s.a[1] * 0.85 + n[2] * 0.01,
            ],
            [
                o[0] + s.a[0] * 0.85 + n[0] * 0.01,
                o[1] + s.a[1] * 0.85 + n[2] * 0.01,
            ],
        );
        h.gold.quad(
            [lift(p, y0), lift(q, y0), lift(q, y1), lift(p, y1)],
            n,
            hex(GOLD),
        );
    }
}

/// The lantern on the roof: walls of the glass that lights, stone piers at
/// its corners and a slab over it, and a gold finial.
fn lantern(h: &mut Hall, s: &Site) {
    let (u0, u1, v0, v1) = (4.0, 7.0, -3.2, 3.2);
    prism(
        &mut h.lamps,
        &s.rect(u0, u1, v0, v1),
        CORNICE_TOP,
        LANTERN_TOP,
        hex(GLASS),
        false,
    );
    for (u, v) in [(u0, v0), (u1, v0), (u1, v1), (u0, v1)] {
        prism(
            &mut h.stone,
            &square(s, u, v, 0.14),
            CORNICE_TOP,
            LANTERN_TOP,
            hex(PALE_STONE),
            false,
        );
    }
    prism(
        &mut h.stone,
        &s.rect(u0 - 0.15, u1 + 0.15, v0 - 0.15, v1 + 0.15),
        LANTERN_TOP,
        ROOF_Y,
        hex(PALE_STONE),
        true,
    );
    loft(
        &mut h.gold,
        &square(s, 5.5, 0.0, 0.12),
        ROOF_Y,
        &square(s, 5.5, 0.0, 0.02),
        ROOF_Y + 0.6,
        hex(GOLD),
        false,
    );
}

/// The attic over the portico and its clock, facing the avenue. Its hands
/// show `phase` of the day.
fn clock(h: &mut Hall, s: &Site, phase: f64) {
    prism(
        &mut h.stone,
        &s.rect(1.95, 2.55, -1.35, 1.35),
        CORNICE_TOP,
        ATTIC_TOP,
        hex(STONE),
        true,
    );
    prism(
        &mut h.gold,
        &s.rect(1.9, 2.6, -1.42, 1.42),
        ATTIC_TOP,
        TOP_Y,
        hex(GOLD),
        true,
    );
    let n = s.front();
    let centre = s.at(1.95, 0.0);
    // Points on the face: `x` to the viewer's right, `y` up, `out` toward
    // the viewer, in metres.
    let right = [-s.f[1], s.f[0]];
    let on = |x: f32, y: f32, out: f32| -> [f32; 3] {
        [
            centre[0] + right[0] * x + n[0] * out,
            CLOCK_Y + y,
            centre[1] + right[1] * x + n[2] * out,
        ]
    };
    let disc = |m: &mut MeshBuf, r: f32, out: f32, colour: [f32; 4]| {
        let rim: Vec<[f32; 3]> = (0..=24)
            .map(|k| {
                let t = k as f32 / 24.0 * std::f32::consts::TAU;
                on(r * t.cos(), r * t.sin(), out)
            })
            .collect();
        m.fan(on(0.0, 0.0, out), &rim, n, colour);
    };
    disc(&mut h.gold, CLOCK_R, 0.02, hex(GOLD));
    disc(
        &mut h.light,
        CLOCK_R - 0.07,
        0.03,
        glow(hex(CLOCK_FACE), 1.3),
    );
    // The hours, small gold marks round the rim.
    let bar =
        |m: &mut MeshBuf, angle: f32, r0: f32, r1: f32, half: f32, out: f32, colour: [f32; 4]| {
            let (dx, dy) = (angle.sin(), angle.cos());
            let (px, py) = (dy * half, -dx * half);
            m.quad(
                [
                    on(dx * r0 - px, dy * r0 - py, out),
                    on(dx * r1 - px, dy * r1 - py, out),
                    on(dx * r1 + px, dy * r1 + py, out),
                    on(dx * r0 + px, dy * r0 + py, out),
                ],
                n,
                colour,
            );
        };
    for k in 0..12 {
        let a = k as f32 / 12.0 * std::f32::consts::TAU;
        let long = if k % 3 == 0 { 0.14 } else { 0.08 };
        bar(
            &mut h.gold,
            a,
            CLOCK_R - 0.1 - long,
            CLOCK_R - 0.1,
            0.02,
            0.04,
            hex(GOLD),
        );
    }
    let (hour, minute) = hands(phase);
    bar(&mut h.gold, hour, -0.06, 0.32, 0.035, 0.05, hex(BRONZE));
    bar(&mut h.gold, minute, -0.08, 0.48, 0.022, 0.06, hex(BRONZE));
}

/// The clock's hands at `phase` of the day: the hour and minute hands'
/// angles, clockwise from twelve, in radians.
pub fn hands(phase: f64) -> (f32, f32) {
    let hours = phase.rem_euclid(1.0) * 24.0;
    let tau = std::f64::consts::TAU;
    (
        ((hours % 12.0) / 12.0 * tau) as f32,
        (hours.fract() * tau) as f32,
    )
}

/// The game minute the clock shows at `phase`: the hands are rebuilt when it
/// changes.
pub fn minute(phase: f64) -> i64 {
    (phase.rem_euclid(1.0) * 24.0 * 60.0).floor() as i64
}

/// A market stall at depth `uc`, from `v0` to `v1` along the frontage: a
/// timber counter with goods on it, four posts, and a striped awning
/// sloping down toward the street.
fn stall(h: &mut Hall, s: &Site, uc: f32, v0: f32, v1: f32, k: usize) {
    let timber = hex(TIMBER);
    prism(
        &mut h.stone,
        &s.rect(uc - 0.28, uc + 0.28, v0 + 0.06, v1 - 0.06),
        PAVE,
        PAVE + 0.85,
        timber,
        true,
    );
    // Goods: three crates or bundles on the counter.
    let w = (v1 - v0 - 0.2) / 3.0;
    for j in 0..3 {
        let c = GOODS[(k * 2 + j) % GOODS.len()];
        let a = v0 + 0.1 + w * j as f32;
        let top = PAVE + 0.85 + 0.1 + 0.05 * ((k + j) % 2) as f32;
        prism(
            &mut h.stone,
            &s.rect(uc - 0.18, uc + 0.12, a + 0.03, a + w - 0.03),
            PAVE + 0.85,
            top,
            hex(c),
            true,
        );
    }
    let (front, back) = (PAVE + 1.95, PAVE + 2.25);
    for (u, v) in [
        (uc - 0.4, v0),
        (uc - 0.4, v1),
        (uc + 0.4, v0),
        (uc + 0.4, v1),
    ] {
        let top = if u < uc { front } else { back };
        prism(
            &mut h.stone,
            &square(s, u, v, 0.03),
            PAVE,
            top,
            timber,
            false,
        );
    }
    // The awning, in stripes, both faces, reaching past the front posts.
    let stripes = ((v1 - v0) / 0.16).round().max(2.0) as usize;
    let colour = hex(AWNINGS[k % AWNINGS.len()]);
    let (ua, ub) = (uc - 0.55, uc + 0.45);
    let ya = front - 0.05;
    for j in 0..stripes {
        let a = v0 + (v1 - v0) * j as f32 / stripes as f32;
        let b = v0 + (v1 - v0) * (j + 1) as f32 / stripes as f32;
        let c = if j % 2 == 0 { colour } else { hex(CLOTH) };
        let corners = [
            lift(s.at(ua, a), ya),
            lift(s.at(ub, a), back),
            lift(s.at(ub, b), back),
            lift(s.at(ua, b), ya),
        ];
        let normal = slope_normal(corners);
        h.stone.quad(corners, normal, c);
        h.stone.quad(
            corners,
            normal.map(|x| -x),
            mix(c, [0.0, 0.0, 0.0, 1.0], 0.25),
        );
        // The valance hanging off the front edge.
        let drop = [
            lift(s.at(ua, a), ya),
            lift(s.at(ua, b), ya),
            lift(s.at(ua, b), ya - 0.16),
            lift(s.at(ua, a), ya - 0.16),
        ];
        h.stone.quad(drop, s.front(), c);
        h.stone.quad(drop, s.front().map(|x| -x), c);
    }
}

/// The upward normal of a sloped quad.
fn slope_normal([a, b, _, d]: [[f32; 3]; 4]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
    let mut n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    if n[1] < 0.0 {
        n = n.map(|x| -x);
    }
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
    n.map(|x| x / len)
}

/// The material of [`LAMPS_MESH`]: glass that gives off the Trade blue at
/// radiance 1 over every vertex, for the entity's `emissive_scale` to set by
/// the hour.
fn lamps_material() -> ImportedMaterial {
    let [r, g, b, _] = hex(GLASS_LIGHT);
    ImportedMaterial {
        roughness: 0.15,
        emissive: [r, g, b],
        emissive_strength: 1.0,
        ..ImportedMaterial::vertex_colored(LAMPS_MESH)
    }
}

/// The hall in the world: built when the terrain brings one, and its gold
/// (with the clock's hands) again each game minute.
#[derive(Default)]
pub struct HallLayer {
    site: Option<Site>,
    minute: Option<i64>,
}

impl HallLayer {
    /// The hall's site, if the terrain has one.
    pub fn site(&self) -> Option<&Site> {
        self.site.as_ref()
    }

    /// A new terrain: build on its hall, or take the old one down.
    pub fn terrain(&mut self, gpu: &mut ExtensionGpu, hall: Option<&Landmark>, phase: f64) {
        self.site = hall.map(Site::new);
        let built = self.site.map(|s| build(&s, phase)).unwrap_or_default();
        // The camera keeps out of it, and a click meets it (#293).
        built.stone.upload_with(gpu, MESH, 0.75, crate::world::mesh::RAY_TARGET);
        built.lamps.upload_material(gpu, lamps_material());
        upload_gold(gpu, built.gold, built.light);
        self.minute = Some(minute(phase));
    }

    /// Moves the clock's hands when the game minute changes.
    pub fn tick(&mut self, gpu: &mut ExtensionGpu, phase: f64) {
        let (Some(site), Some(was)) = (self.site, self.minute) else {
            return;
        };
        let now = minute(phase);
        if now == was {
            return;
        }
        self.minute = Some(now);
        let built = build(&site, phase);
        upload_gold(gpu, built.gold, built.light);
    }
}

fn upload_gold(gpu: &mut ExtensionGpu, mut gold: MeshBuf, light: MeshBuf) {
    gold.append(light);
    gold.upload_material(
        gpu,
        ImportedMaterial {
            metallic: 0.85,
            roughness: 0.35,
            ..ImportedMaterial::vertex_colored(GOLD_MESH)
        },
    );
}

/// How strongly the glass lights at this hour: a fixed lamp's rule
/// (`PlotGlow::lamp_light`), a little softer, so the blue stays blue under
/// the bloom rather than burning white.
pub fn glass_light(glow: crate::daylight::PlotGlow) -> f32 {
    glow.lamp_light() * GLASS_GLOW
}

const GLASS_GLOW: f32 = 0.8;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::to_server;
    use crate::world::mesh::facing;
    use crate::world::mesh::probe::triangles;
    use capitol_protocol::terrain::LandmarkKind;
    use std::path::Path;

    fn hall() -> Landmark {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
        terrain
            .landmarks
            .into_iter()
            .find(|l| l.kind == LandmarkKind::ExchangeHall)
            .expect("the fixture has the hall")
    }

    fn parts(h: &Hall) -> [(&str, &MeshBuf); 4] {
        [
            ("stone", &h.stone),
            ("lamps", &h.lamps),
            ("gold", &h.gold),
            ("light", &h.light),
        ]
    }

    #[test]
    fn the_site_faces_the_avenue() {
        let site = Site::new(&hall());
        // The door is on the west edge (server x 6045), so in is east (+X),
        // and the front faces west.
        assert!(
            (site.f[0] - 1.0).abs() < 1e-4 && site.f[1].abs() < 1e-4,
            "{site:?}"
        );
        assert!((site.su - 1.0).abs() < 1e-4 && (site.sv - 1.0).abs() < 1e-4);
        assert!(site.front()[0] < -0.99);
    }

    #[test]
    fn everything_stands_on_its_footprint() {
        let hall = hall();
        let h = build(&Site::new(&hall), 0.8);
        for (name, m) in parts(&h) {
            for v in &m.vertices {
                let [x, y] = to_server(v.position[0], v.position[2]);
                let d = hall.distance_to(x, y);
                assert!(
                    d < 0.05,
                    "{name} vertex {:?} is {d} units off the footprint",
                    v.position
                );
            }
        }
    }

    #[test]
    fn every_face_faces_its_normal() {
        let h = build(&Site::new(&hall()), 0.3);
        for (name, m) in parts(&h) {
            for v in triangles(m) {
                let n = v[0].normal;
                assert!(
                    facing(v[0].position, v[1].position, v[2].position, n),
                    "{name} face at {:?}",
                    v[0].position
                );
            }
        }
    }

    #[test]
    fn the_light_glows_and_the_glass_is_lit_by_its_material() {
        let h = build(&Site::new(&hall()), 0.8);
        assert!(!h.light.vertices.is_empty());
        assert!(h.light.vertices.iter().all(crate::world::mesh::is_light));
        assert!(!h.lamps.vertices.iter().any(crate::world::mesh::is_light));
        let top = |m: &MeshBuf| {
            m.vertices
                .iter()
                .map(|v| v.position[1])
                .fold(f32::MIN, f32::max)
        };
        assert!(
            (top(&h.gold) - (ROOF_Y + 0.6)).abs() < 1e-4,
            "the finial is the top"
        );
        assert!(top(&h.stone) <= TOP_Y);
    }

    #[test]
    fn the_door_is_open_up_the_steps() {
        // Nothing stands in a 1.2 m lane from the street to the door (an
        // avatar is 0.7 m across), but the paving, the steps and the podium
        // under it.
        let s = Site::new(&hall());
        let h = build(&s, 0.5);
        let (f, a) = (s.f, s.a);
        let local = |p: [f32; 3]| {
            let d = [p[0] - s.door[0], p[2] - s.door[1]];
            (d[0] * f[0] + d[1] * f[1], d[0] * a[0] + d[1] * a[1], p[1])
        };
        for (name, m) in [("stone", &h.stone), ("gold", &h.gold)] {
            for tri in triangles(m) {
                let pts = tri.map(|v| local(v.position));
                let lo =
                    |k: fn(&(f32, f32, f32)) -> f32| pts.iter().map(k).fold(f32::MAX, f32::min);
                let hi =
                    |k: fn(&(f32, f32, f32)) -> f32| pts.iter().map(k).fold(f32::MIN, f32::max);
                let blocks = lo(|p| p.0) < 3.38
                    && hi(|p| p.1) > -0.6
                    && lo(|p| p.1) < 0.6
                    && lo(|p| p.2) > STYLOBATE + 0.01
                    && lo(|p| p.2) < G + 3.4;
                assert!(!blocks, "{name} triangle in the way to the door: {pts:?}");
            }
        }
    }

    #[test]
    fn the_hands_keep_the_hour() {
        let tau = std::f32::consts::TAU;
        let (h, m) = hands(0.5);
        assert!(h.abs() < 1e-5 && m.abs() < 1e-5, "noon: both at twelve");
        let (h, m) = hands(15.25 / 24.0);
        assert!((h - (3.25 / 12.0) * tau).abs() < 1e-4);
        assert!((m - 0.25 * tau).abs() < 1e-3, "a quarter past");
        assert_ne!(minute(0.5), minute(0.5 + 1.0 / 1440.0));
    }

    #[test]
    fn it_is_cheap_to_draw() {
        let h = build(&Site::new(&hall()), 0.8);
        let triangles: usize = parts(&h).iter().map(|(_, m)| m.indices.len() / 3).sum();
        println!("hall: {triangles} triangles");
        assert!(triangles < 6_000, "{triangles} triangles");
    }
}
