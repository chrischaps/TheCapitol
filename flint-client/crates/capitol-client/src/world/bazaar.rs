//! The bazaar (E3.11, E3.12, E3.14): merchants' own shops in the bazaar
//! quarter, the south-west of the Trade District (economy plan D6). The
//! stalls stand on their own footprints from `/terrain`, each facing its
//! street, and the quarter's mouth on the avenue, the `bazaar` landmark,
//! opposite the Exchange hall, is an arch.
//!
//! Since E3.14 (#279) the quarter is a street of many buildings, somewhat
//! chaotic, built to fill the view at the shoulder (E3.13). The hall and
//! the bazaar are still a pair: the same stone and gold, but where the hall
//! is the city's one book, cool and ordered behind Trade-blue glass, the
//! bazaar is many people's, warm and coloured:
//!
//! - **each stall is one of five kinds** ([`Kind`], [`stalls`]): an open
//!   stall under a pitched cloth roof, a tent, a handcart, a narrow
//!   shopfront in a tall house, or a kiosk. The kind is seeded from the
//!   stall's id, and a shop nudges it toward what suits its goods (a
//!   shopfront stays one whoever keeps it, so the street wall stays put);
//! - **the street wall** ([`houses`]): behind each stall stands a house of
//!   two or three storeys, as deep as the ground behind allows before a
//!   road, the water or another building, with jettied upper floors, a
//!   lean, shutters and a roof of its own. They differ in height, colour
//!   and depth, so the wall is continuous and uneven;
//! - **a kept stall** is open, in its shop's colour (a hash of its name, so
//!   every client agrees), stocked with goods coloured by the kinds the
//!   directory lists, with more spilling forward and hanging from its
//!   eaves, and a **sign** where the HUD writes the shop's name;
//! - **an empty stall** is shuttered, its cloth furled;
//! - **life on the street** ([`street`]): strings of bunting and lanterns
//!   between facing stalls, washing across the upper floors, crates, sacks,
//!   barrels and baskets, a fountain in the square, and the mouth's arch.
//!
//! The lanterns and the lit windows come on at dusk (H23,
//! `material.emissive_scale` on [`LAMPS_MESH`]), so the quarter glows amber
//! against the hall's blue. Three meshes, as the hall's: [`MESH`] (stone,
//! plaster, timber, tile and cloth), [`LAMPS_MESH`] (the glass that
//! lights) and [`GOLD_MESH`] (gold metal and dark iron). A fourth,
//! [`HUNG_MESH`], holds the strings across the street, apart so that only
//! [`MESH`] is a ray target the camera keeps out of (#293). They're rebuilt
//! when the terrain or the directory changes which stalls are kept, or what
//! they sell.

mod houses;
mod stalls;
mod street;

use crate::coords::{layer, to_world};
use crate::world::mesh::{facing, MeshBuf};
use crate::world::palette::{hex, mix};
use crate::world::tower::{hash, lift, prism, V2};
use capitol_protocol::rest::ShopSign;
use capitol_protocol::terrain::{Landmark, LandmarkKind, RoadSegment, TerrainResponse, WaterRing};
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;

/// The runtime meshes' names, as their entities are named.
pub const MESH: &str = "bazaar";
pub const LAMPS_MESH: &str = "bazaar_lamps";
pub const GOLD_MESH: &str = "bazaar_gold";
/// The strings across the street, in [`MESH`]'s material.
pub const HUNG_MESH: &str = "bazaar_hung";

/// Every mesh, for attaching their entities.
pub fn meshes() -> [String; 4] {
    [MESH, LAMPS_MESH, GOLD_MESH, HUNG_MESH].map(str::to_owned)
}

const G: f32 = layer::GROUND;
/// The paving: level with the lots' tops, as the hall's forecourt.
const PAVE: f32 = layer::PLOT;
/// The counter's top.
const COUNTER: f32 = PAVE + 0.85;
/// The hanging sign: its bottom, top, and half its width.
const SIGN: (f32, f32) = (G + 1.6, G + 1.92);
const SIGN_HALF: f32 = 0.34;
/// A house's ground floor, and each floor over it.
const GROUND_STOREY: f32 = 2.5;
const UPPER_STOREY: f32 = 2.15;
/// The most a roof rises over its eaves.
const MAX_RISE: f32 = 1.1;
/// The top of everything: a three-storey house's ridge at its steepest,
/// still under the hall's clock.
pub const TOP_Y: f32 = PAVE + GROUND_STOREY + 2.0 * UPPER_STOREY + MAX_RISE;
/// Nothing low stands nearer a street's centre than its edge, and nothing
/// under this height stands out over it (the signs hang just above).
const HEADROOM: f32 = G + 1.55;
/// How far proud of a wall its windows, shutters and banners stand.
const PROUD: f32 = 0.012;

// Materials: the hall's stone (`world::hall`), so the two read as the
// Capitol's, and warm plaster, tile, timber and cloth for the market.
const STONE: u32 = 0xa79f90;
const PALE_STONE: u32 = 0xc8bfae;
const PAVING: u32 = 0x928879;
const TILES: [u32; 5] = [0x9c5b3f, 0x8a4f37, 0xa8694a, 0x6e4a3a, 0x7d5a4a];
const TIMBER: u32 = crate::world::palette::TIMBER;
const TIMBER_DARK: u32 = 0x6e5236;
const CLOTH: u32 = 0xe9e1cf;
const CANVAS: u32 = 0xc9bc9c;
const CORD: u32 = 0x2c2620;
const IRON: u32 = 0x2e2a27;
const GOLD: u32 = crate::world::palette::GOLD;
/// The houses' plaster: cream, ochre, terracotta, chalk, rose, sage, a
/// faded blue and saffron, so no two neighbours need match.
const PLASTER: [u32; 8] = [0xd9c7a3, 0xcf9f6e, 0xb8735a, 0xe2d3c0, 0xc9a6a0, 0x9fb0a8, 0xa9b7c6, 0xd8b46a];
const SHUTTERS: [u32; 5] = [0x3f5f4a, 0x2f4f6f, 0x7a3b2e, 0x5b4636, 0x6b7a8a];
/// A window's glass by day: the houses' (`palette::GLASS`).
const GLASS: u32 = crate::world::palette::GLASS;
/// The lanterns' glass by day, and the light they give at night.
const LANTERN_GLASS: u32 = 0x6b4a26;
pub const LANTERN_LIGHT: u32 = 0xffae57;
/// The shops' colours: a stall's cloth and its sign's border. A shop keeps
/// its colour (a hash of its name), whoever looks.
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

    /// The world point at (u, v) and height `y`.
    fn p(&self, u: f32, v: f32, y: f32) -> [f32; 3] {
        lift(self.at(u, v), y)
    }

    /// Where world point `p` falls in the frame, as (u, v).
    #[cfg(test)]
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

    /// Along `v`, as a normal: `side` is its sign.
    fn side(&self, side: f32) -> [f32; 3] {
        [self.a[0] * side, 0.0, self.a[1] * side]
    }

    /// A box `half` wide either way round (u, v), turned with the frame.
    fn square(&self, u: f32, v: f32, half_u: f32, half_v: f32) -> Vec<V2> {
        self.rect(u - half_u, u + half_u, v - half_v, v + half_v)
    }

    /// A vertical face across the frame at `u`, from `v0` to `v1` and `y0`
    /// to `y1`, facing `n`.
    fn face_u(&self, m: &mut MeshBuf, u: f32, (v0, v1): (f32, f32), (y0, y1): (f32, f32), n: [f32; 3], colour: [f32; 4]) {
        m.quad([self.p(u, v0, y0), self.p(u, v1, y0), self.p(u, v1, y1), self.p(u, v0, y1)], n, colour);
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
    /// The way the board's front faces, out over its street.
    pub facing: [f32; 3],
    /// How wide the name may run between the board's ends, and how far
    /// each face stands from `at`: 0 for an empty stall, which has none.
    pub width: f32,
    pub depth: f32,
}

/// The bazaar, built: one buffer per material, the strings hung across
/// the street on their own, and where its signs hang.
#[derive(Default)]
pub struct Bazaar {
    pub stone: MeshBuf,
    pub lamps: MeshBuf,
    pub gold: MeshBuf,
    /// The cords, bunting and washing strung across the street: drawn as
    /// stone is, but not a ray target, so the camera doesn't snag on cloth.
    pub hung: MeshBuf,
    pub signs: Vec<Sign>,
}

/// What the quarter stands among: every landmark (the square, the hall,
/// the other stalls), the roads and the water. The houses keep clear of
/// all of it.
#[derive(Debug, Clone, Copy)]
pub struct Ground<'a> {
    pub landmarks: &'a [Landmark],
    pub roads: &'a [RoadSegment],
    pub rings: &'a [WaterRing],
}

impl<'a> Ground<'a> {
    pub fn of(terrain: &'a TerrainResponse) -> Self {
        Self { landmarks: &terrain.landmarks, roads: &terrain.roads, rings: &terrain.water_rings }
    }
}

/// What stands on a stall's footprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Timber posts under a pitched cloth roof, a counter across the front.
    Open,
    /// A ridge tent, its flaps tied back.
    Tent,
    /// A handcart under a canopy, crates and barrels behind it.
    Cart,
    /// The ground floor of a narrow house, its top shutter propped up as an
    /// awning.
    Shopfront,
    /// A six-sided booth under a cone.
    Kiosk,
}

/// A house in the street wall: behind a stall, or over a shopfront. In the
/// stall's frame, its street face stands at `u0` on the ground floor, and
/// each floor over it is jettied `jetty` further toward the street and
/// leans `lean` along it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct House {
    u0: f32,
    u1: f32,
    storeys: usize,
    jetty: f32,
    lean: f32,
    wall: u32,
    shutter: u32,
    tile: u32,
    /// Timber framing on its street face.
    framed: bool,
    /// The ridge runs along the street, eaves to it; or a gable faces it.
    ridge_along: bool,
    chimney: bool,
}

impl House {
    /// The bottom of floor `k` (0 the ground floor; `storeys` the eaves).
    fn floor_y(k: usize) -> f32 {
        if k == 0 {
            PAVE
        } else {
            PAVE + GROUND_STOREY + (k - 1) as f32 * UPPER_STOREY
        }
    }

    fn eaves(&self) -> f32 {
        Self::floor_y(self.storeys)
    }

    /// Floor `k`'s street face, and its sides along the street, given the
    /// footprint's half width `half`.
    fn floor(&self, k: usize, half: f32) -> (f32, f32, f32) {
        let k = k as f32;
        let inset = self.lean.abs() * (self.storeys - 1) as f32;
        let (v0, v1) = (-half + inset, half - inset);
        (self.u0 - self.jetty * k, v0 + self.lean * k, v1 + self.lean * k)
    }

    /// The floor at height `y`.
    fn floor_at(&self, y: f32) -> usize {
        (1..=self.storeys).take_while(|&k| Self::floor_y(k) <= y).count().min(self.storeys - 1)
    }
}

/// A stall as laid out: its frame, its kind and the house in its street
/// wall, if the ground behind it allows one.
#[derive(Debug, Clone)]
struct Lot {
    frame: Frame,
    kind: Kind,
    house: Option<House>,
    seed: u32,
}

impl Lot {
    /// The half width the stall's buildings keep to, a hair inside its
    /// footprint so neighbours never share a face.
    fn half(&self) -> f32 {
        self.frame.width / 2.0 - 0.03
    }

    /// Where a string across the street ties on at height `y`: the house's
    /// street face, as (u, v) in the frame.
    fn anchor(&self, y: f32) -> Option<(f32, f32)> {
        let h = self.house?;
        let k = h.floor_at(y);
        let (u, v0, v1) = h.floor(k, self.half());
        Some((u - 0.01, (v0 + v1) * 0.5))
    }
}

/// A well-mixed number in 0..1 from a seed and a salt.
fn rnd(seed: u32, salt: u32) -> f32 {
    hash(seed, salt, 0x5b) as f32 / u32::MAX as f32
}

/// One of `list`, by a seed and a salt.
fn pick(list: &[u32], seed: u32, salt: u32) -> u32 {
    list[hash(seed, salt, 0x77) as usize % list.len()]
}

/// What stands on a stall: its kind, from its id, nudged half the time by
/// its shop's goods. A shopfront is fixed by its id, so the street wall
/// stays put whoever keeps it.
fn kind_of(stall: &Landmark, shop: Option<&ShopSign>) -> Kind {
    let seed = name_hash(&stall.id);
    let base = match hash(seed, 11, 3) % 20 {
        0..=6 => Kind::Shopfront,
        7..=10 => Kind::Open,
        11..=13 => Kind::Tent,
        14..=16 => Kind::Cart,
        _ => Kind::Kiosk,
    };
    let Some(shop) = shop else { return base };
    if base == Kind::Shopfront || hash(name_hash(&shop.name), seed, 7).is_multiple_of(2) {
        return base;
    }
    let sells = |keys: &[&str]| shop.item_types.iter().any(|t| keys.iter().any(|k| t.contains(k)));
    if sells(&["log", "plank", "stone", "ore", "brick", "clay", "coal"]) {
        // Bulk wants an open stall's room
        Kind::Open
    } else if sells(&["rope", "fiber", "grass", "herb", "twine", "cloth"]) {
        Kind::Cart
    } else if sells(&["nail", "_head", "ingot", "knife", "chisel", "sickle", "hammer", "kit"]) {
        Kind::Kiosk
    } else {
        base
    }
}

/// The ground a house may take: off every road, the water and every other
/// building.
struct Clearance {
    quads: Vec<[V2; 4]>,
    roads: Vec<(V2, V2, f32)>,
    rings: Vec<(f32, f32)>,
}

impl Clearance {
    fn new(ground: &Ground) -> Self {
        let w = |p: (f64, f64)| to_world(p.0, p.1);
        let roads = ground
            .roads
            .iter()
            .flat_map(|r| {
                let half = r.width as f32 * 0.05 + 0.25;
                r.points.windows(2).map(move |s| (w(s[0]), w(s[1]), half))
            })
            .collect();
        Self {
            quads: Vec::new(),
            roads,
            rings: ground.rings.iter().map(|r| (r.radius as f32 * 0.1, r.width as f32 * 0.05 + 0.6)).collect(),
        }
    }

    /// Only what lies within `r` of `c`, so testing many points is quick.
    fn near(&self, c: V2, r: f32) -> Self {
        let close = |q: &[V2; 4]| q.iter().any(|p| (p[0] - c[0]).hypot(p[1] - c[1]) < r + 6.0);
        let cr = c[0].hypot(c[1]);
        Self {
            quads: self.quads.iter().filter(|q| close(q)).copied().collect(),
            roads: self.roads.iter().filter(|&&(a, b, half)| segment_distance(c, a, b) < r + half).copied().collect(),
            rings: self.rings.iter().filter(|&&(radius, half)| (cr - radius).abs() < r + half).copied().collect(),
        }
    }

    fn clear(&self, p: V2) -> bool {
        let r = p[0].hypot(p[1]);
        self.rings.iter().all(|&(radius, half)| (r - radius).abs() > half)
            && self.roads.iter().all(|&(a, b, half)| segment_distance(p, a, b) > half)
            && !self.quads.iter().any(|q| quad_contains(q, p))
    }
}

fn segment_distance(p: V2, a: V2, b: V2) -> f32 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dz * dz;
    let t = if len2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len2).clamp(0.0, 1.0) };
    (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dz * t)
}

/// Whether convex `q` (either winding) holds `p`.
fn quad_contains(q: &[V2; 4], p: V2) -> bool {
    let side = |i: usize| {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let s = [side(0), side(1), side(2), side(3)];
    s.iter().all(|&x| x >= 0.0) || s.iter().all(|&x| x <= 0.0)
}

fn quad(r: Vec<V2>) -> [V2; 4] {
    [r[0], r[1], r[2], r[3]]
}

/// Lays the quarter out: each stall's kind, as no shop has nudged it, and
/// the house behind it, as deep as the ground allows (from 1.4 m to about
/// 5), taken in stall order so the first to claim the ground behind a
/// corner keeps it. It depends on the terrain alone, so it's laid out once
/// and the shops dress it.
fn lay_out(stalls: &[&Landmark], ground: &Ground) -> Vec<Lot> {
    let mut clear = Clearance::new(ground);
    let w = |p: (f64, f64)| to_world(p.0, p.1);
    for l in ground.landmarks {
        if l.kind != LandmarkKind::Stall {
            clear.quads.push(l.corners.map(w));
        }
    }
    let feet: Vec<[V2; 4]> = stalls.iter().map(|s| s.corners.map(w)).collect();
    let mut lots = Vec::with_capacity(stalls.len());
    for (i, s) in stalls.iter().enumerate() {
        let frame = Frame::new(s);
        let seed = name_hash(&s.id);
        let kind = kind_of(s, None);
        let half = frame.width / 2.0 - 0.03;
        // What's near the ground behind it: the roads, the water, the
        // landmarks and houses, and the other stalls
        let centre = frame.at(frame.depth, 0.0);
        let here = clear.near(centre, frame.width + 5.5);
        let others: Vec<[V2; 4]> = feet
            .iter()
            .enumerate()
            .filter(|&(j, q)| j != i && (q[0][0] - centre[0]).hypot(q[0][1] - centre[1]) < 14.0)
            .map(|(_, q)| *q)
            .collect();
        let free = |d: f32| {
            let (nu, nv) = ((d / 0.25).ceil() as usize + 1, (frame.width / 0.25).ceil() as usize + 1);
            (0..=nu).all(|a| {
                let u = frame.depth + 0.05 + (d - 0.05) * a as f32 / nu as f32;
                (0..=nv).all(|b| {
                    let v = -half + 0.02 + (2.0 * half - 0.04) * b as f32 / nv as f32;
                    let p = frame.at(u, v);
                    here.clear(p) && !others.iter().any(|q| quad_contains(q, p))
                })
            })
        };
        let want = 2.4 + 2.6 * rnd(seed, 1);
        let behind = (0..)
            .map(|k| want - 0.25 * k as f32)
            .take_while(|&d| d >= 1.4)
            .find(|&d| free(d))
            .unwrap_or(0.0);
        let tall = rnd(seed, 2);
        let house = if kind == Kind::Shopfront || behind > 0.0 {
            let shop = kind == Kind::Shopfront;
            let u0 = if shop { 0.0 } else { frame.depth };
            let storeys = if tall < if shop { 0.6 } else { 0.45 } { 3 } else { 2 };
            Some(House {
                u0,
                u1: frame.depth + behind,
                storeys,
                jetty: if shop { 0.22 } else { 0.2 + 0.15 * rnd(seed, 3) },
                lean: (rnd(seed, 4) - 0.5) * 0.1,
                wall: pick(&PLASTER, seed, 5),
                shutter: pick(&SHUTTERS, seed, 6),
                tile: pick(&TILES, seed, 7),
                framed: rnd(seed, 8) < 0.5,
                ridge_along: rnd(seed, 9) < 0.6,
                chimney: rnd(seed, 10) < 0.45,
            })
        } else {
            None
        };
        if let Some(h) = &house {
            clear.quads.push(quad(frame.rect(h.u0.max(frame.depth), h.u1, -half, half)));
        }
        lots.push(Lot { frame, kind, house, seed });
    }
    lots
}

/// Builds the bazaar: the mouth on `bazaar`, its `stalls` as they're kept,
/// their houses, and the street's life among `ground`. The layer lays the
/// quarter out once and builds on that ([`BazaarLayer::sync`]).
#[cfg(test)]
fn build(bazaar: &Landmark, stalls: &[StallLook], ground: &Ground) -> Bazaar {
    let feet: Vec<&Landmark> = stalls.iter().map(|s| s.stall).collect();
    build_on(bazaar, stalls, &lay_out(&feet, ground), ground)
}

/// Builds the bazaar on a layout already made: `lots` are `stalls`', in
/// order.
fn build_on(bazaar: &Landmark, stalls: &[StallLook], lots: &[Lot], ground: &Ground) -> Bazaar {
    let mut out = Bazaar::default();
    street::mouth(&mut out, &Frame::new(bazaar));
    street::paving(&mut out, ground);
    let lots: Vec<Lot> = lots
        .iter()
        .zip(stalls)
        .map(|(lot, look)| Lot { kind: kind_of(look.stall, look.shop), ..lot.clone() })
        .collect();
    for (lot, look) in lots.iter().zip(stalls) {
        let shop = look.shop.map(|s| Shop {
            colour: hex(AWNINGS[name_hash(&s.name) as usize % AWNINGS.len()]),
            goods: s.item_types.iter().map(|t| goods_colour(t)).collect(),
            seed: name_hash(&s.name),
        });
        let sign = stalls::front(&mut out, lot, shop.as_ref());
        if let Some(h) = &lot.house {
            houses::house(&mut out, lot, h);
        }
        stalls::spill(&mut out, lot, shop.as_ref());
        let colour = shop.as_ref().map_or([0.8, 0.78, 0.72], |s| [s.colour[0], s.colour[1], s.colour[2]]);
        let (at, width) = sign;
        out.signs.push(Sign {
            stall_id: look.stall.id.clone(),
            at,
            colour,
            facing: lot.frame.out(),
            width,
            depth: if width > 0.0 { BOARD } else { 0.0 },
        });
    }
    street::strings(&mut out, &lots);
    if let Some(square) = ground.landmarks.iter().find(|l| l.kind == LandmarkKind::Square) {
        street::square(&mut out, &Frame::new(square));
    }
    out
}

/// A kept stall's shop, as the world shows it.
struct Shop {
    colour: [f32; 4],
    /// The colours of the kinds of goods it sells.
    goods: Vec<u32>,
    seed: u32,
}

impl Shop {
    /// The colour of its `k`th heap of goods.
    fn good(&self, k: u32) -> [f32; 4] {
        if self.goods.is_empty() {
            hex(TIMBER)
        } else {
            hex(self.goods[(self.seed as usize + k as usize) % self.goods.len()])
        }
    }
}

// ── Pieces every part uses ────────────────────────────────────────────────

/// Both faces of a flat quad: cloth, a flag, a sheet on the line.
fn both(m: &mut MeshBuf, c: [[f32; 3]; 4], n: [f32; 3], colour: [f32; 4]) {
    m.quad(c, n, colour);
    m.quad(c, n.map(|x| -x), colour);
}

/// A flat triangle facing `n`, wound to suit.
fn tri(m: &mut MeshBuf, [a, b, c]: [[f32; 3]; 3], n: [f32; 3], colour: [f32; 4]) {
    let (b, c) = if facing(a, b, c, n) { (b, c) } else { (c, b) };
    let ids = [a, b, c].map(|p| m.vertex(p, n, colour));
    m.indices.extend(ids);
}

/// The unit normal of the plane through `a b c`, either way up.
fn normal_of(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
    n.map(|x| x / len)
}

/// The upward normal of a sloped quad.
fn up_normal([a, b, _, d]: [[f32; 3]; 4]) -> [f32; 3] {
    let n = normal_of(a, b, d);
    if n[1] < 0.0 {
        n.map(|x| -x)
    } else {
        n
    }
}

/// A sloped quad of cloth or tile: its top face in `colour`, and under it
/// a darker face for the street to look up at.
fn sloped(m: &mut MeshBuf, c: [[f32; 3]; 4], colour: [f32; 4]) {
    let n = up_normal(c);
    m.quad(c, n, colour);
    m.quad(c, n.map(|x| -x), mix(colour, [0.0, 0.0, 0.0, 1.0], 0.3));
}

/// A square post at (u, v), `half` wide, from `y0` to `y1`.
fn post(m: &mut MeshBuf, f: &Frame, (u, v): (f32, f32), half: f32, (y0, y1): (f32, f32), colour: [f32; 4]) {
    prism(m, &f.square(u, v, half, half), y0, y1, colour, true);
}

/// A thin cord from `a` to `b`, seen from either side.
fn cord(m: &mut MeshBuf, a: [f32; 3], b: [f32; 3]) {
    const THICK: f32 = 0.012;
    let d = [b[0] - a[0], b[2] - a[2]];
    let len = d[0].hypot(d[1]);
    if len < 1e-4 {
        // Straight down: as wide as it's thick, across x
        let side = |p: [f32; 3]| [p[0] + THICK, p[1], p[2]];
        both(m, [a, b, side(b), side(a)], [0.0, 0.0, 1.0], hex(CORD));
        return;
    }
    let n = [-d[1] / len, 0.0, d[0] / len];
    let down = |p: [f32; 3]| [p[0], p[1] - THICK, p[2]];
    both(m, [a, b, down(b), down(a)], n, hex(CORD));
}

/// A lantern hanging from `top`: a cap, glass that lights, and a base.
fn lantern(h: &mut Bazaar, c: V2, top: f32, size: f32) {
    let sq = |r: f32| vec![[c[0] - r, c[1] - r], [c[0] + r, c[1] - r], [c[0] + r, c[1] + r], [c[0] - r, c[1] + r]];
    let glass = top - 0.04;
    let bottom = glass - size * 1.6;
    crate::world::tower::loft(&mut h.gold, &sq(size * 0.8), glass, &sq(size * 0.2), top, hex(TIMBER_DARK), false);
    prism(&mut h.lamps, &sq(size * 0.5), bottom, glass, hex(LANTERN_GLASS), true);
    prism(&mut h.gold, &sq(size * 0.6), bottom - 0.03, bottom, hex(TIMBER_DARK), true);
}

/// Half a sign board's thickness: where each face stands from its middle.
const BOARD: f32 = 0.025;

/// A shop's sign hanging at `u` (out over the street) on two cords from
/// `from` (the height of what it hangs from), its border in the shop's
/// colour on both faces. Returns where its name is painted, and how wide
/// it may run.
fn sign(h: &mut Bazaar, f: &Frame, u: f32, from: f32, colour: [f32; 4]) -> ([f32; 3], f32) {
    let half = SIGN_HALF.min(f.width * 0.32);
    for v in [-half * 0.8, half * 0.8] {
        let c = f.at(u, v);
        prism(&mut h.gold, &[[c[0] - 0.008, c[1] - 0.008], [c[0] + 0.008, c[1] - 0.008], [c[0] + 0.008, c[1] + 0.008], [c[0] - 0.008, c[1] + 0.008]], SIGN.1, from + 0.01, hex(CORD), false);
    }
    prism(&mut h.stone, &f.rect(u - BOARD, u + BOARD, -half, half), SIGN.0, SIGN.1, hex(SIGN_BOARD), true);
    for (uu, n) in [(u - BOARD - 0.002, f.out()), (u + BOARD + 0.002, f.back())] {
        for ys in [(SIGN.0 + 0.02, SIGN.0 + 0.045), (SIGN.1 - 0.045, SIGN.1 - 0.02)] {
            f.face_u(&mut h.stone, uu, (-half + 0.03, half - 0.03), ys, n, colour);
        }
    }
    // Inside the border's ends, with a margin
    (f.p(u, 0.0, (SIGN.0 + SIGN.1) * 0.5), 2.0 * (half - 0.03) - 0.08)
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
/// by the hour. The windows that light are in it too, dark glass by day.
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

/// What the bazaar was last built from: the terrain's version, its
/// footprint, and each stall's id and shop, if kept.
type Drawn = (Option<String>, Landmark, Vec<(Landmark, Option<ShopSign>)>);

/// The bazaar in the world: built when the terrain brings one, and again
/// when the directory changes which stalls are kept or what they sell.
#[derive(Default)]
pub struct BazaarLayer {
    drawn: Option<Drawn>,
    /// The layout, and the terrain's version and stalls it was made for.
    laid: Option<(Option<String>, Vec<Landmark>, Vec<Lot>)>,
    signs: Vec<Sign>,
}

impl BazaarLayer {
    /// Where the stalls' names hang.
    pub fn signs(&self) -> &[Sign] {
        &self.signs
    }

    /// Builds the bazaar if `landmarks` or `shops` (by stall id) differ from
    /// what was built, or takes it down when the terrain has none. True if
    /// it uploaded, so the meshes' entities need attaching.
    pub fn sync<'a>(
        &mut self,
        gpu: &mut ExtensionGpu,
        terrain: &TerrainResponse,
        shop: impl Fn(&str) -> Option<&'a ShopSign>,
    ) -> bool {
        let landmarks = &terrain.landmarks;
        let Some(bazaar) = landmarks.iter().find(|l| l.kind == LandmarkKind::Bazaar) else {
            if self.drawn.take().is_some() {
                self.laid = None;
                self.signs.clear();
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
        let now = (terrain.version.clone(), bazaar.clone(), stalls);
        if self.drawn.as_ref() == Some(&now) {
            return false;
        }
        let looks: Vec<StallLook> = now.2.iter().map(|(stall, shop)| StallLook { stall, shop: shop.as_ref() }).collect();
        let feet: Vec<Landmark> = now.2.iter().map(|(stall, _)| stall.clone()).collect();
        let ground = Ground::of(terrain);
        if !self.laid.as_ref().is_some_and(|(v, f, _)| *v == terrain.version && *f == feet) {
            let lots = lay_out(&feet.iter().collect::<Vec<_>>(), &ground);
            self.laid = Some((terrain.version.clone(), feet, lots));
        }
        let lots = self.laid.as_ref().map(|(_, _, lots)| lots.as_slice()).unwrap_or_default();
        let built = build_on(&now.1, &looks, lots, &ground);
        self.signs = built.signs.clone();
        upload(gpu, built);
        self.drawn = Some(now);
        true
    }
}

fn upload(gpu: &mut ExtensionGpu, built: Bazaar) {
    // The stalls and houses are what the camera keeps out of (#293); the
    // strings across the street aren't.
    built.stone.upload_with(gpu, MESH, 0.8, crate::world::mesh::RAY_TARGET);
    built.hung.upload(gpu, HUNG_MESH, 0.8);
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

    fn terrain() -> TerrainResponse {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        capitol_core::terrain::read_terrain_file(&path).expect("fixture")
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
    fn built_from(t: &TerrainResponse, shops: &[ShopSign]) -> (Landmark, Vec<Landmark>, Bazaar) {
        let bazaar = t.landmarks.iter().find(|l| l.kind == LandmarkKind::Bazaar).expect("the fixture has the bazaar").clone();
        let stalls: Vec<Landmark> = t.landmarks.iter().filter(|l| l.kind == LandmarkKind::Stall).cloned().collect();
        let looks: Vec<StallLook> = stalls
            .iter()
            .enumerate()
            .map(|(i, stall)| StallLook { stall, shop: (i % 2 == 0).then(|| &shops[i / 2 % shops.len()]) })
            .collect();
        let b = build(&bazaar, &looks, &Ground::of(t));
        (bazaar, stalls, b)
    }

    fn built(shops: &[ShopSign]) -> (Landmark, Vec<Landmark>, Bazaar) {
        built_from(&terrain(), shops)
    }

    fn shops() -> Vec<ShopSign> {
        vec![
            sign("Ada's Ropes", &["rope", "fiber"]),
            sign("Ironmonger", &["iron_ingot", "sickle"]),
            sign("Bram & Daughters", &[]),
            sign("Mira's Mill", &["plank", "log"]),
            sign("Hollow Oak", &["herb", "clay_pot"]),
        ]
    }

    fn parts(b: &Bazaar) -> [(&str, &MeshBuf); 4] {
        [("stone", &b.stone), ("lamps", &b.lamps), ("gold", &b.gold), ("hung", &b.hung)]
    }

    fn lots() -> Vec<Lot> {
        let t = terrain();
        let stalls: Vec<&Landmark> = t.landmarks.iter().filter(|l| l.kind == LandmarkKind::Stall).collect();
        lay_out(&stalls, &Ground::of(&t))
    }

    #[test]
    fn the_fixture_has_the_quarter_and_its_mouth_faces_the_avenue() {
        let t = terrain();
        let mouth = t.landmarks.iter().find(|l| l.kind == LandmarkKind::Bazaar).unwrap();
        let stalls: Vec<_> = t.landmarks.iter().filter(|l| l.kind == LandmarkKind::Stall).collect();
        assert!(stalls.len() >= 34, "{} stalls: refresh the fixture", stalls.len());
        // The mouth faces the avenue, east, as the hall faces it west
        assert!(Frame::new(mouth).out()[0] > 0.99, "{:?}", Frame::new(mouth).out());
        assert!(t.landmarks.iter().any(|l| l.kind == LandmarkKind::Square), "the square, for its fountain");
    }

    /// The stalls are of every kind, and most have a house behind them, so
    /// the street has a wall of many heights.
    #[test]
    fn the_stalls_are_varied_and_a_street_wall_stands_behind_them() {
        let lots = lots();
        for kind in [Kind::Open, Kind::Tent, Kind::Cart, Kind::Shopfront, Kind::Kiosk] {
            let n = lots.iter().filter(|l| l.kind == kind).count();
            assert!(n >= 3, "{kind:?}: {n} of {}", lots.len());
        }
        let housed = lots.iter().filter(|l| l.house.is_some()).count();
        let kinds: Vec<(Kind, usize)> = [Kind::Open, Kind::Tent, Kind::Cart, Kind::Shopfront, Kind::Kiosk].map(|k| (k, lots.iter().filter(|l| l.kind == k).count())).to_vec();
        println!("{} stalls, {housed} with houses: {kinds:?}", lots.len());
        assert!(housed * 10 >= lots.len() * 8, "{housed} of {} have houses", lots.len());
        let tall = lots.iter().filter_map(|l| l.house).filter(|h| h.storeys == 3).count();
        assert!(tall >= 6 && tall + 6 <= housed, "{tall} of {housed} have three storeys");
        let colours: std::collections::HashSet<u32> = lots.iter().filter_map(|l| l.house).map(|h| h.wall).collect();
        assert!(colours.len() >= 6, "{} plaster colours", colours.len());
    }

    /// A house stands only where nothing else does: off the roads, the
    /// water, every landmark and every other stall's house.
    #[test]
    fn the_houses_keep_off_the_roads_the_water_and_each_other() {
        let t = terrain();
        let lots = lots();
        let mut clear = Clearance::new(&Ground::of(&t));
        clear.quads = t.landmarks.iter().filter(|l| l.kind != LandmarkKind::Bazaar).map(|l| l.corners.map(|p| to_world(p.0, p.1))).collect();
        let behind: Vec<(usize, [V2; 4])> = lots
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.house.filter(|h| h.u1 > l.frame.depth + 0.1).map(|h| (i, quad(l.frame.rect(l.frame.depth + 0.06, h.u1 - 0.01, -l.half() + 0.02, l.half() - 0.02)))))
            .collect();
        for (i, q) in &behind {
            let f = &lots[*i].frame;
            for k in 0..=4 {
                for j in 0..=4 {
                    let p = [
                        q[0][0] + (q[1][0] - q[0][0]) * k as f32 / 4.0 + (q[3][0] - q[0][0]) * j as f32 / 4.0,
                        q[0][1] + (q[1][1] - q[0][1]) * k as f32 / 4.0 + (q[3][1] - q[0][1]) * j as f32 / 4.0,
                    ];
                    let (u, _) = f.local(p);
                    assert!(u > f.depth, "behind its stall");
                    let others = Clearance { quads: Vec::new(), ..Clearance::new(&Ground::of(&t)) };
                    assert!(others.clear(p), "house {i} at {p:?} stands on a road or the water");
                    let on = clear.quads.iter().filter(|q| quad_contains(q, p)).count();
                    assert_eq!(on, 0, "house {i} at {p:?} stands on a landmark");
                    for (j2, q2) in &behind {
                        assert!(j2 == i || !quad_contains(q2, p), "houses {i} and {j2} overlap at {p:?}");
                    }
                }
            }
        }
    }

    /// Everything stands on a stall's footprint, its house, the mouth or
    /// the square, or within a stall's gap to its street; what reaches out
    /// over the street (awnings, signs, jetties and the strings across it)
    /// is all over head height.
    #[test]
    fn everything_stands_on_a_footprint_or_overhangs_its_street_overhead() {
        let t = terrain();
        let (mouth, stalls, b) = built_from(&t, &shops());
        let lots = lots();
        let square = t.landmarks.iter().find(|l| l.kind == LandmarkKind::Square).unwrap();
        let mut feet: Vec<[V2; 4]> = stalls.iter().chain([&mouth, square]).map(|l| l.corners.map(|p| to_world(p.0, p.1))).collect();
        feet.extend(lots.iter().filter_map(|l| l.house.map(|h| quad(l.frame.rect(h.u0, h.u1 + 0.2, -l.frame.width / 2.0, l.frame.width / 2.0)))));
        for (name, m) in parts(&b) {
            for v in &m.vertices {
                let p = [v.position[0], v.position[2]];
                let on = feet.iter().any(|q| quad_contains(q, p));
                let [x, y] = to_server(p[0], p[1]);
                let d = stalls.iter().chain([&mouth, square]).map(|l| l.distance_to(x, y)).fold(f64::MAX, f64::min);
                // A stall's own ground runs to its street's edge, 3 units out,
                // and its eaves a little over that
                if !on && d > 4.0 && v.position[1] > PAVE {
                    assert!(v.position[1] > HEADROOM, "{name} vertex {:?} stands in the street, {d} off", v.position);
                }
            }
        }
    }

    /// The streets stay clear to walk: nothing under head height stands on
    /// the lane, the alley or the market street, but the mouth's piers.
    #[test]
    fn the_streets_stay_clear_to_walk() {
        let t = terrain();
        let (mouth, _, b) = built_from(&t, &shops());
        let mouth = mouth.corners.map(|p| to_world(p.0, p.1));
        let streets: Vec<(V2, V2, f32)> = t
            .roads
            .iter()
            .filter(|r| ["bazaar_lane", "bazaar_alley", "trade_ring"].contains(&r.id.as_str()))
            .flat_map(|r| r.points.windows(2).map(|s| (to_world(s[0].0, s[0].1), to_world(s[1].0, s[1].1), r.width as f32 * 0.05)).collect::<Vec<_>>())
            .collect();
        for (name, m) in parts(&b) {
            for tri in triangles(m) {
                let ys = tri.map(|v| v.position[1]);
                let (lo, hi) = (ys.iter().copied().fold(f32::MAX, f32::min), ys.iter().copied().fold(f32::MIN, f32::max));
                // Over head height, or the street's own setts
                if lo > HEADROOM || hi <= PAVE {
                    continue;
                }
                for v in tri {
                    let p = [v.position[0], v.position[2]];
                    if quad_contains(&mouth, p) {
                        continue;
                    }
                    for &(a, c, half) in &streets {
                        let d = segment_distance(p, a, c);
                        assert!(d > half - 0.02, "{name} vertex {:?} is {d} from a street's crown (half {half})", v.position);
                    }
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
        let t = terrain();
        let (bazaar, stalls, b) = built_from(&t, &shops());
        assert_eq!(b.signs.len(), stalls.len());
        let bare = build(&bazaar, &stalls.iter().map(|stall| StallLook { stall, shop: None }).collect::<Vec<_>>(), &Ground::of(&t));
        assert!(b.stone.vertices.len() > bare.stone.vertices.len() + 20 * 100, "cloth, goods and signs");
        assert!(b.lamps.vertices.len() > bare.lamps.vertices.len(), "a lantern at each kept stall");
        for (i, s) in b.signs.iter().enumerate() {
            let stall = stalls.iter().find(|l| l.id == s.stall_id).unwrap();
            let [x, y] = to_server(s.at[0], s.at[2]);
            if i % 2 == 0 {
                // Out over its street, in front of the stall, within its range
                assert!(stall.distance_to(x, y) > 1.0, "{} hangs over its street", s.stall_id);
                assert!(capitol_protocol::rules::bazaar::at_the_stall(stall, x, y), "{} out of range", s.stall_id);
                assert!(s.at[1] > SIGN.0 && s.at[1] < SIGN.1);
                assert!(s.width > 0.3 && s.depth > 0.0, "{}: a board to paint on", s.stall_id);
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
    fn the_lanterns_and_windows_light_by_their_material_and_nothing_tops_the_hall() {
        let (_, _, b) = built(&shops());
        assert!(!b.lamps.vertices.is_empty());
        assert!(!b.lamps.vertices.iter().any(crate::world::mesh::is_light));
        let top = parts(&b).iter().flat_map(|(_, m)| m.vertices.iter()).map(|v| v.position[1]).fold(f32::MIN, f32::max);
        assert!(top <= TOP_Y + 1e-4, "{top} over {TOP_Y}");
        assert!(top > G + 7.0, "{top}: the tallest house is three storeys");
        // Lower than the hall, the civic one of the pair
        const { assert!(TOP_Y < crate::world::hall::TOP_Y) };
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

    /// A shopfront stays a shopfront whoever keeps it; other stalls may
    /// change kind with their shop, and keep their houses.
    #[test]
    fn a_shop_nudges_its_stall_but_never_moves_the_street_wall() {
        let t = terrain();
        let stalls: Vec<&Landmark> = t.landmarks.iter().filter(|l| l.kind == LandmarkKind::Stall).collect();
        let kept_sign = sign("Timber Yard", &["log", "plank"]);
        let lots = lay_out(&stalls, &Ground::of(&t));
        let mut changed = 0;
        for (lot, stall) in lots.iter().zip(&stalls) {
            let kind = kind_of(stall, Some(&kept_sign));
            assert_eq!(lot.kind == Kind::Shopfront, kind == Kind::Shopfront, "{}", stall.id);
            changed += usize::from(lot.kind != kind);
        }
        assert!(changed > 0, "a timber yard wants an open stall somewhere");
    }

    #[test]
    fn it_is_cheap_to_draw() {
        let t = terrain();
        let started = std::time::Instant::now();
        let (_, _, b) = built_from(&t, &shops());
        let took = started.elapsed();
        let triangles: usize = parts(&b).iter().map(|(_, m)| m.indices.len() / 3).sum();
        println!("bazaar: {triangles} triangles, built in {took:?}");
        assert!(triangles < 90_000, "{triangles} triangles");
    }
}
