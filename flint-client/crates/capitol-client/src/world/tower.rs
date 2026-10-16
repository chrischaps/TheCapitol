//! The Capitol (P3.7): the landmark at the centre of the world, and the one
//! thing you can see from everywhere in it.
//!
//! The GDD calls it "a modern structure arranged like a medieval castle".
//! Here that's castle massing in contemporary materials, inside the moat:
//!
//! - a **curtain wall** of pale concrete standing on the moat's inner kerb,
//!   with a parapet, square merlons and a line of light under the parapet,
//!   and a round tower every 45° capped in gold;
//! - a **gatehouse** over each moat bridge, with its passage lit from
//!   above and a gold line of light over the opening;
//! - the **bailey**, a paved court with a ring of light set into it and a
//!   path from the gate to the keep's lit door;
//! - the **keep**: a podium, four glass corner turrets, and a glass tower
//!   in three setbacks, lit floor by floor, with a stone pier at every
//!   corner rising past each terrace like a merlon;
//! - the **crown**: a lantern of gold light inside a ring of gold blades,
//!   and a gold needle with a point of light at its tip, about 84 m up.
//!   From the wilderness this is the part you see.
//!
//! The keep faces the first moat bridge, so its door looks down the path
//! to the gate. The wall follows the moat's radius and opens at every
//! bridge, so a new terrain (P3.10) rebuilds it to fit.
//!
//! Three meshes, so three draws: `capitol` (stone), `capitol_glass` and
//! `capitol_gold` (the gold metal and all the light, which glows the way
//! `MeshBuf::glowing` describes).

use crate::coords::{layer, S};
use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{self, glow, hex, mix};
use crate::world::water;
use capitol_protocol::rules::Zone;
use capitol_protocol::terrain::WaterRing;
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;

/// The runtime meshes' names, as the scene's entities name them.
pub const MESH: &str = "capitol";
pub const GLASS_MESH: &str = "capitol_glass";
pub const GOLD_MESH: &str = "capitol_gold";

/// A point on the ground in world metres (X, Z), the tower's centre at the
/// origin.
type V2 = [f32; 2];

/// The land the tower stands on.
const G: f32 = layer::GROUND;

// The curtain wall, in metres.
const WALL_THICK: f32 = 1.3;
const WALL_TOP: f32 = G + 4.2;
const PARAPET: f32 = 0.4;
const PARAPET_TOP: f32 = G + 4.9;
const MERLON_TOP: f32 = G + 5.5;
const MERLON: f32 = 1.1;
const CRENEL: f32 = 0.7;
/// How far the wall stands into the moat's kerb, so its foot is hidden in
/// the stone rather than drawn over the kerb's face.
const INTO_KERB: f32 = 0.1;
const TOWER_R: f32 = 1.7;
const TOWER_TOP: f32 = G + 7.0;

// The gatehouse.
/// Clearance either side of the bridge's deck.
const GATE_CLEAR: f32 = 0.45;
const GATE_PIER: f32 = 2.4;
const GATE_DEPTH: f32 = 1.6;
const PASSAGE_TOP: f32 = G + 5.0;
const GATE_TOP: f32 = G + 8.2;

// The bailey.
const RING_R: f32 = 11.7;
const PATH_HALF: f32 = 1.5;

// The keep.
const PODIUM: f32 = 7.0;
const PODIUM_TOP: f32 = G + 3.0;
const TURRET_AT: f32 = 5.0;
const TURRET: f32 = 1.4;
const TURRET_TOP: f32 = G + 24.0;
/// Each setback: half its width, and where it stops.
const TIERS: [(f32, f32); 3] = [(4.2, G + 36.0), (3.4, G + 50.0), (2.6, G + 60.0)];
const SLAB: f32 = 0.3;
const PIER: f32 = 0.5;
const FLOOR: f32 = 3.5;
/// A window: about this wide, this tall, with half a mullion each side.
const PANE: f32 = 1.2;
const PANE_HEIGHT: f32 = 1.4;
const MULLION: f32 = 0.08;

// The crown.
const CROWN: f32 = G + 60.0 + SLAB;
const LANTERN: f32 = 2.0;
const LANTERN_TOP: f32 = CROWN + 5.5;
const NEEDLE_TIP: f32 = CROWN + 22.0;
/// Just above the point of light on the needle: the top of the tower.
pub const CROWN_Y: f32 = NEEDLE_TIP + 2.0;

// Materials. Stone is a warm pale concrete; the glass a blue-black that
// takes the sky.
const CONCRETE: u32 = 0xa79f90;
const PIER_STONE: u32 = 0xc8bfae;
const ROOF: u32 = 0x7d776c;
const PAVING: u32 = 0x5a544b;
const GLASS: u32 = 0x1a1d2b;
const WARM_LIGHT: u32 = 0xffd9a0;
const COOL_LIGHT: u32 = 0xfff1dc;
const TIP_LIGHT: u32 = 0xfff0c8;

/// The tower, built: one buffer per material, plus the light.
#[derive(Default)]
pub struct Tower {
    pub stone: MeshBuf,
    pub glass: MeshBuf,
    pub gold: MeshBuf,
    /// Built facing the right way like the rest, and made to glow when it's
    /// merged into the gold mesh.
    pub light: MeshBuf,
}

/// Where the tower stands and where its gates go, from the terrain.
#[derive(Debug, Clone)]
pub struct Site {
    /// The land face of the moat's inner kerb, in metres: the edge of the
    /// island.
    pub island: f32,
    /// The innermost ring's inner water edge, in metres. Nothing of the
    /// tower reaches it (tested).
    #[cfg_attr(not(test), allow(dead_code))]
    pub water: f32,
    /// Each moat bridge: its server angle in degrees, and half its width
    /// in metres.
    pub gates: Vec<(f64, f32)>,
}

impl Site {
    /// The innermost water ring is the moat. With no rings at all, the
    /// tower fills the Capitol zone.
    pub fn new(rings: &[WaterRing]) -> Self {
        let moat = rings.iter().min_by(|a, b| a.radius.total_cmp(&b.radius));
        let Some(moat) = moat else {
            let r = Zone::Capitol.radii().1 as f32 * S;
            return Self {
                island: r,
                water: r,
                gates: Vec::new(),
            };
        };
        let (kerbed, _) = water::kerbed(std::slice::from_ref(moat))[0];
        Self {
            island: kerbed as f32 * S,
            water: moat.inner_radius() as f32 * S,
            gates: moat
                .bridges
                .iter()
                .map(|b| (b.angle_degrees, b.width as f32 * 0.5 * S))
                .collect(),
        }
    }

    /// The way the keep faces: toward the first gate, else south.
    fn front(&self) -> f64 {
        self.gates.first().map_or(90.0, |g| g.0)
    }

    fn wall_out(&self) -> f32 {
        self.island + INTO_KERB
    }

    fn wall_in(&self) -> f32 {
        self.wall_out() - WALL_THICK
    }
}

/// A frame on the ground facing a server angle: `u` runs out along it and
/// `v` across it.
#[derive(Clone, Copy)]
struct Frame {
    f: V2,
    a: V2,
}

impl Frame {
    fn toward(degrees: f64) -> Self {
        let r = degrees.to_radians();
        let f = [r.cos() as f32, r.sin() as f32];
        Self {
            f,
            a: [-f[1], f[0]],
        }
    }

    fn at(&self, u: f32, v: f32) -> V2 {
        [self.f[0] * u + self.a[0] * v, self.f[1] * u + self.a[1] * v]
    }

    fn out(&self) -> [f32; 3] {
        [self.f[0], 0.0, self.f[1]]
    }

    fn rect(&self, u0: f32, u1: f32, v0: f32, v1: f32) -> Vec<V2> {
        vec![
            self.at(u0, v0),
            self.at(u1, v0),
            self.at(u1, v1),
            self.at(u0, v1),
        ]
    }

    /// A square of half-width `half` centred at (`u`, `v`).
    fn square(&self, u: f32, v: f32, half: f32) -> Vec<V2> {
        self.rect(u - half, u + half, v - half, v + half)
    }
}

fn lift([x, z]: V2, y: f32) -> [f32; 3] {
    [x, y, z]
}

fn centroid(pts: &[V2]) -> V2 {
    let n = pts.len() as f32;
    let s = pts
        .iter()
        .fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
    [s[0] / n, s[1] / n]
}

/// The direction of server angle `degrees`, as world (X, Z).
fn dir(degrees: f64) -> V2 {
    let r = degrees.to_radians();
    [r.cos() as f32, r.sin() as f32]
}

fn polar(r: f32, degrees: f64) -> V2 {
    let [x, z] = dir(degrees);
    [x * r, z * r]
}

fn ngon(centre: V2, r: f32, sides: usize) -> Vec<V2> {
    (0..sides)
        .map(|i| {
            let [x, z] = polar(r, 360.0 * i as f64 / sides as f64);
            [centre[0] + x, centre[1] + z]
        })
        .collect()
}

/// The sides of a solid from the convex outline `bottom` at `y0` up to
/// `top` at `y1` (the same corners in the same order, so a taper works),
/// each face flat and facing out, with a flat roof when `roof`.
fn loft(
    m: &mut MeshBuf,
    bottom: &[V2],
    y0: f32,
    top: &[V2],
    y1: f32,
    colour: [f32; 4],
    roof: bool,
) {
    let n = bottom.len();
    let c = centroid(bottom);
    for i in 0..n {
        let j = (i + 1) % n;
        let corners = [
            lift(bottom[i], y0),
            lift(bottom[j], y0),
            lift(top[j], y1),
            lift(top[i], y1),
        ];
        let e1 = sub3(corners[1], corners[0]);
        let e2 = sub3(corners[3], corners[0]);
        let mut normal = unit3(cross(e1, e2));
        let mid = [
            (bottom[i][0] + bottom[j][0]) * 0.5 - c[0],
            (bottom[i][1] + bottom[j][1]) * 0.5 - c[1],
        ];
        if normal[0] * mid[0] + normal[2] * mid[1] < 0.0 {
            normal = normal.map(|k| -k);
        }
        m.quad(corners, normal, colour);
    }
    if roof {
        let mut rim: Vec<[f32; 3]> = top.iter().map(|&p| lift(p, y1)).collect();
        rim.push(rim[0]);
        m.fan(lift(centroid(top), y1), &rim, UP, colour);
    }
}

/// A straight-sided solid over `outline` from `y0` to `y1`.
fn prism(m: &mut MeshBuf, outline: &[V2], y0: f32, y1: f32, colour: [f32; 4], roof: bool) {
    loft(m, outline, y0, outline, y1, colour, roof);
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit3(a: [f32; 3]) -> [f32; 3] {
    let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-9);
    a.map(|k| k / len)
}

/// Columns round an arc of radius `r` from `a0` to `a1` degrees, about a
/// metre apart.
fn steps(r: f32, a0: f64, a1: f64) -> usize {
    (((a1 - a0).to_radians() * r as f64).ceil() as usize).max(1)
}

/// A band of wall round the centre from radius `r0` to `r1`, between server
/// angles `a0` < `a1`, from `y0` to `y1`: its outer and inner faces, its top
/// and, unless it closes a full circle, an end at each angle.
#[allow(clippy::too_many_arguments)]
fn arc(m: &mut MeshBuf, r0: f32, r1: f32, a0: f64, a1: f64, y0: f32, y1: f32, colour: [f32; 4]) {
    let n = steps(r1, a0, a1);
    let at = |k: usize| a0 + (a1 - a0) * k as f64 / n as f64;
    for k in 0..n {
        let (p, q) = (at(k), at(k + 1));
        let [nx, nz] = dir((p + q) * 0.5);
        let (o0, o1) = (polar(r1, p), polar(r1, q));
        let (i0, i1) = (polar(r0, p), polar(r0, q));
        m.quad(
            [lift(o0, y0), lift(o1, y0), lift(o1, y1), lift(o0, y1)],
            [nx, 0.0, nz],
            colour,
        );
        m.quad(
            [lift(i0, y0), lift(i1, y0), lift(i1, y1), lift(i0, y1)],
            [-nx, 0.0, -nz],
            colour,
        );
        m.quad(
            [lift(i0, y1), lift(o0, y1), lift(o1, y1), lift(i1, y1)],
            UP,
            colour,
        );
    }
    if a1 - a0 < 360.0 {
        for (a, sign) in [(a0, -1.0), (a1, 1.0)] {
            let [tx, tz] = dir(a + 90.0);
            m.quad(
                [
                    lift(polar(r0, a), y0),
                    lift(polar(r1, a), y0),
                    lift(polar(r1, a), y1),
                    lift(polar(r0, a), y1),
                ],
                [sign * tx, 0.0, sign * tz],
                colour,
            );
        }
    }
}

/// Just the outward face of radius `r` between `a0` and `a1`: a band of
/// light on a wall.
fn arc_face(m: &mut MeshBuf, r: f32, a0: f64, a1: f64, y0: f32, y1: f32, colour: [f32; 4]) {
    let n = steps(r, a0, a1);
    let at = |k: usize| a0 + (a1 - a0) * k as f64 / n as f64;
    for k in 0..n {
        let (p, q) = (at(k), at(k + 1));
        let [nx, nz] = dir((p + q) * 0.5);
        let (a, b) = (polar(r, p), polar(r, q));
        m.quad(
            [lift(a, y0), lift(b, y0), lift(b, y1), lift(a, y1)],
            [nx, 0.0, nz],
            colour,
        );
    }
}

/// Merlons in a row along an arc of radius `r1`, `PARAPET` deep, spaced
/// evenly so the row starts and ends on one.
fn merlon_arc(m: &mut MeshBuf, r1: f32, a0: f64, a1: f64, colour: [f32; 4]) {
    let length = (a1 - a0).to_radians() as f32 * r1;
    let count = ((length + CRENEL) / (MERLON + CRENEL)).floor().max(1.0) as usize;
    let pitch = (a1 - a0) / count as f64;
    let h = (((MERLON / r1) as f64).to_degrees() * 0.5).min(pitch * 0.5);
    for k in 0..count {
        let mid = a0 + pitch * (k as f64 + 0.5);
        arc(
            m,
            r1 - PARAPET,
            r1,
            mid - h,
            mid + h,
            PARAPET_TOP,
            MERLON_TOP,
            colour,
        );
    }
}

/// The light colours, as vertex colours for `MeshBuf::glowing`.
fn warm(radiance: f32) -> [f32; 4] {
    glow(hex(WARM_LIGHT), radiance)
}
fn gold_light(radiance: f32) -> [f32; 4] {
    glow(mix(hex(palette::GOLD), [1.0; 4], 0.25), radiance)
}

/// Builds the tower for the given site.
pub fn build(site: &Site) -> Tower {
    let mut t = Tower::default();
    curtain(&mut t, site);
    for &(angle, half) in &site.gates {
        gatehouse(&mut t, site, angle, half);
    }
    bailey(&mut t, site);
    keep(&mut t, Frame::toward(site.front()));
    t
}

/// How far either side of a gate's axis the wall stops, in degrees at the
/// wall's inner face: inside the gatehouse's piers.
fn gate_gap(site: &Site, half: f32) -> f64 {
    let v = half + GATE_CLEAR + 0.6;
    ((v / site.wall_in()).min(1.0) as f64).asin().to_degrees()
}

/// The stretches of wall between the gates, as `(from, to)` degrees.
fn stretches(site: &Site) -> Vec<(f64, f64)> {
    let mut gaps: Vec<(f64, f64)> = site
        .gates
        .iter()
        .map(|&(a, half)| (a.rem_euclid(360.0), gate_gap(site, half)))
        .collect();
    if gaps.is_empty() {
        return vec![(0.0, 360.0)];
    }
    gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
    (0..gaps.len())
        .map(|i| {
            let (a, ha) = gaps[i];
            let (mut b, hb) = gaps[(i + 1) % gaps.len()];
            if b <= a {
                b += 360.0;
            }
            (a + ha, b - hb)
        })
        .filter(|(a, b)| b > a)
        .collect()
}

fn curtain(t: &mut Tower, site: &Site) {
    let concrete = hex(CONCRETE);
    let (r0, r1) = (site.wall_in(), site.wall_out());
    for (a0, a1) in stretches(site) {
        arc(&mut t.stone, r0, r1, a0, a1, G, WALL_TOP, concrete);
        arc(
            &mut t.stone,
            r1 - PARAPET,
            r1,
            a0,
            a1,
            WALL_TOP,
            PARAPET_TOP,
            concrete,
        );
        merlon_arc(&mut t.stone, r1, a0, a1, concrete);
        // A line of light along the outer face, under the parapet: from
        // across the city the wall reads as a ring of it.
        arc_face(
            &mut t.light,
            r1 + 0.04,
            a0,
            a1,
            WALL_TOP - 0.55,
            WALL_TOP - 0.4,
            warm(0.9),
        );
    }
    // A round tower every 45° from the front, but none on a gate.
    let tower_half = ((TOWER_R / site.wall_in()) as f64).asin().to_degrees();
    let centre_r = r1 - TOWER_R;
    for k in 0..8 {
        let a = site.front() + 45.0 * k as f64;
        let on_gate = site.gates.iter().any(|&(g, half)| {
            let d = (a - g + 540.0).rem_euclid(360.0) - 180.0;
            d.abs() < gate_gap(site, half) + tower_half + 2.0
        });
        if on_gate {
            continue;
        }
        let c = polar(centre_r, a);
        prism(
            &mut t.stone,
            &ngon(c, TOWER_R, 12),
            G,
            TOWER_TOP,
            concrete,
            false,
        );
        prism(
            &mut t.gold,
            &ngon(c, TOWER_R + 0.12, 12),
            TOWER_TOP,
            TOWER_TOP + 0.25,
            hex(palette::GOLD),
            true,
        );
        prism(
            &mut t.light,
            &ngon(c, TOWER_R + 0.03, 12),
            TOWER_TOP - 0.7,
            TOWER_TOP - 0.55,
            gold_light(1.4),
            false,
        );
    }
}

/// A gatehouse over the bridge at `angle`, whose deck is `half` wide
/// either side of its axis: two piers and a lintel over the passage.
fn gatehouse(t: &mut Tower, site: &Site, angle: f64, half: f32) {
    let f = Frame::toward(angle);
    let concrete = hex(CONCRETE);
    let ph = half + GATE_CLEAR;
    let hw = ph + GATE_PIER;
    let (u0, u1) = (site.wall_in() - GATE_DEPTH, site.wall_out());
    for (v0, v1) in [(-hw, -ph), (ph, hw)] {
        prism(
            &mut t.stone,
            &f.rect(u0, u1, v0, v1),
            G,
            GATE_TOP,
            concrete,
            true,
        );
    }
    prism(
        &mut t.stone,
        &f.rect(u0, u1, -ph, ph),
        PASSAGE_TOP,
        GATE_TOP,
        concrete,
        true,
    );
    // The passage's ceiling glows down on whoever walks through.
    let soffit = f.rect(u0, u1, -ph, ph);
    let [a, b, c, d] = [0, 1, 2, 3].map(|i| lift(soffit[i], PASSAGE_TOP));
    t.light.quad([a, b, c, d], [0.0, -1.0, 0.0], warm(1.0));
    // A gold line over the opening, outside and in.
    for (u, sign) in [(u1 + 0.04, 1.0), (u0 - 0.04, -1.0)] {
        let r = f.rect(u, u, -ph - 0.3, ph + 0.3);
        let (p, q) = (r[0], r[2]);
        t.light.quad(
            [
                lift(p, PASSAGE_TOP + 0.25),
                lift(q, PASSAGE_TOP + 0.25),
                lift(q, PASSAGE_TOP + 0.45),
                lift(p, PASSAGE_TOP + 0.45),
            ],
            f.out().map(|k| k * sign),
            gold_light(2.0),
        );
    }
    // Merlons along the front and back of the roof.
    let count = ((2.0 * hw + CRENEL) / (MERLON + CRENEL)).floor() as usize;
    let pitch = 2.0 * hw / count as f32;
    for k in 0..count {
        let v = -hw + pitch * (k as f32 + 0.5);
        for (a, b) in [(u1 - PARAPET, u1), (u0, u0 + PARAPET)] {
            let outline = f.rect(a, b, v - MERLON * 0.5, v + MERLON * 0.5);
            prism(
                &mut t.stone,
                &outline,
                GATE_TOP,
                GATE_TOP + 0.6,
                concrete,
                true,
            );
        }
    }
}

/// The paving inside the wall, its ring of light, and the path from each
/// gate to the keep.
fn bailey(t: &mut Tower, site: &Site) {
    let to_server = |m: f32| (m / S) as f64;
    let mut rim: Vec<[f32; 3]> = ngon([0.0, 0.0], site.wall_out(), 180)
        .into_iter()
        .map(|p| lift(p, layer::BAILEY))
        .collect();
    rim.push(rim[0]);
    t.stone
        .fan([0.0, layer::BAILEY, 0.0], &rim, UP, hex(PAVING));
    let path = hex(palette::road(capitol_protocol::terrain::RoadType::Arterial));
    for &(angle, _) in &site.gates {
        let f = Frame::toward(angle);
        let outline = f.rect(PODIUM, site.wall_in() - GATE_DEPTH, -PATH_HALF, PATH_HALF);
        let [a, b, c, d] = [0, 1, 2, 3].map(|i| lift(outline[i], layer::BAILEY + 0.02));
        t.stone.quad([a, b, c, d], UP, path);
    }
    t.light.annulus(
        to_server(RING_R - 0.12),
        to_server(RING_R + 0.12),
        layer::BAILEY + 0.04,
        gold_light(1.2),
        &[],
    );
}

/// The keep and its crown, facing along `f`.
fn keep(t: &mut Tower, f: Frame) {
    let (concrete, pier, roof, glass) = (hex(CONCRETE), hex(PIER_STONE), hex(ROOF), hex(GLASS));
    let gold = hex(palette::GOLD);

    // The podium: a square with its corners cut, and a lit door at the front.
    let (p, c) = (PODIUM, PODIUM - 1.0);
    let podium = [
        (p, -c),
        (p, c),
        (c, p),
        (-c, p),
        (-p, c),
        (-p, -c),
        (-c, -p),
        (c, -p),
    ]
    .map(|(u, v)| f.at(u, v));
    prism(&mut t.stone, &podium, G, PODIUM_TOP, concrete, true);
    let door = f.rect(p + 0.04, p + 0.04, -1.6, 1.6);
    t.light.quad(
        [
            lift(door[0], layer::BAILEY),
            lift(door[2], layer::BAILEY),
            lift(door[2], layer::BAILEY + 2.6),
            lift(door[0], layer::BAILEY + 2.6),
        ],
        f.out(),
        warm(1.4),
    );

    // Four glass turrets at the corners, lit window by window, capped in
    // gold.
    let corners = [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)];
    for (k, (su, sv)) in corners.into_iter().enumerate() {
        let (u, v) = (su * TURRET_AT, sv * TURRET_AT);
        prism(
            &mut t.glass,
            &f.square(u, v, TURRET),
            PODIUM_TOP,
            TURRET_TOP,
            glass,
            false,
        );
        let outline = f.square(u, v, TURRET + 0.03);
        windows(
            &mut t.light,
            &outline,
            PODIUM_TOP,
            TURRET_TOP - 1.5,
            k as u32,
        );
        prism(
            &mut t.light,
            &f.square(u, v, TURRET + 0.03),
            TURRET_TOP - 0.9,
            TURRET_TOP - 0.7,
            gold_light(1.4),
            false,
        );
        prism(
            &mut t.gold,
            &f.square(u, v, TURRET + 0.15),
            TURRET_TOP,
            TURRET_TOP + 0.5,
            gold,
            true,
        );
    }

    // The tower in three setbacks. Each is a glass core, lit window by
    // window, under a concrete slab with a gold line along its edge, with a
    // stone pier at each corner standing a little above the terrace.
    let mut base = PODIUM_TOP;
    for (k, (half, top)) in TIERS.into_iter().enumerate() {
        prism(
            &mut t.glass,
            &f.square(0.0, 0.0, half),
            base,
            top,
            glass,
            false,
        );
        let outline = f.square(0.0, 0.0, half + 0.03);
        windows(&mut t.light, &outline, base, top - 1.0, 10 + k as u32);
        prism(
            &mut t.stone,
            &f.square(0.0, 0.0, half + 0.15),
            top,
            top + SLAB,
            roof,
            true,
        );
        prism(
            &mut t.light,
            &f.square(0.0, 0.0, half + 0.19),
            top + 0.08,
            top + 0.2,
            gold_light(1.6),
            false,
        );
        for (su, sv) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let outline = f.square(su * half, sv * half, PIER);
            prism(&mut t.stone, &outline, base, top + 1.2, pier, true);
        }
        base = top + SLAB;
    }

    crown(t, f);
}

/// Windows on each face of `outline`, a floor apart from `y0` up to `y1`.
/// About three in five are lit, each a little brighter or dimmer, warmer
/// or whiter than the next, picked by a hash of `seed` and where it is, so
/// the tower looks worked in late and the same every time it's built.
fn windows(light: &mut MeshBuf, outline: &[V2], y0: f32, y1: f32, seed: u32) {
    let c = centroid(outline);
    let n = outline.len();
    let mut floor = 0;
    let mut y = y0 + 0.8;
    while y + PANE_HEIGHT <= y1 {
        for i in 0..n {
            let (p, q) = (outline[i], outline[(i + 1) % n]);
            let along = [q[0] - p[0], q[1] - p[1]];
            let len = along[0].hypot(along[1]);
            let mid = [(p[0] + q[0]) * 0.5 - c[0], (p[1] + q[1]) * 0.5 - c[1]];
            let out = [
                mid[0] / mid[0].hypot(mid[1]),
                0.0,
                mid[1] / mid[0].hypot(mid[1]),
            ];
            let panes = (len / PANE).round().max(1.0) as usize;
            let w = len / panes as f32;
            for j in 0..panes {
                let h = hash(seed, floor, (i * 64 + j) as u32);
                if h % 100 >= 58 {
                    continue;
                }
                let radiance = 0.3 + 0.45 * ((h >> 8) % 100) as f32 / 100.0;
                let tint = ((h >> 16) % 100) as f32 / 100.0;
                let colour = glow(mix(hex(WARM_LIGHT), hex(COOL_LIGHT), tint), radiance);
                let at = |d: f32| [p[0] + along[0] / len * d, p[1] + along[1] / len * d];
                let (a, b) = (at(j as f32 * w + MULLION), at((j + 1) as f32 * w - MULLION));
                light.quad(
                    [
                        lift(a, y),
                        lift(b, y),
                        lift(b, y + PANE_HEIGHT),
                        lift(a, y + PANE_HEIGHT),
                    ],
                    out,
                    colour,
                );
            }
        }
        floor += 1;
        y += FLOOR;
    }
}

/// A well-mixed hash of three numbers, for picking which windows are lit.
fn hash(a: u32, b: u32, c: u32) -> u32 {
    let mut h =
        a.wrapping_mul(0x9e37_79b9) ^ b.wrapping_mul(0x85eb_ca6b) ^ c.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

/// The lantern, its blades and corner points, and the needle.
fn crown(t: &mut Tower, f: Frame) {
    let gold = hex(palette::GOLD);
    let edge = TIERS[2].0;

    // The lantern: a block of light, its roof lit too, so from the map the
    // centre of the city is a point of gold.
    prism(
        &mut t.light,
        &f.square(0.0, 0.0, LANTERN),
        CROWN,
        LANTERN_TOP,
        gold_light(3.5),
        true,
    );

    // Gold blades stand out from each face like the points of a crown,
    // tall and short in turn, and a taller post at each corner.
    let blades = [(-1.6, 1.5), (-0.8, 0.3), (0.0, 1.5), (0.8, 0.3), (1.6, 1.5)];
    for side in 0..4 {
        let g = Frame::toward(f.angle() + 90.0 * side as f64);
        for (v, over) in blades {
            let outline = g.rect(LANTERN + 0.25, edge, v - 0.11, v + 0.11);
            prism(&mut t.gold, &outline, CROWN, LANTERN_TOP + over, gold, true);
        }
    }
    for (su, sv) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let outline = f.square(su * edge, sv * edge, 0.35);
        prism(&mut t.gold, &outline, CROWN, LANTERN_TOP + 2.5, gold, true);
    }

    // The needle, and the point of light at its tip.
    let needle_base = f.square(0.0, 0.0, 0.45);
    let needle_top = f.square(0.0, 0.0, 0.04);
    loft(
        &mut t.gold,
        &needle_base,
        LANTERN_TOP,
        &needle_top,
        NEEDLE_TIP,
        gold,
        false,
    );
    let tip = glow(hex(TIP_LIGHT), 8.0);
    let (narrow, wide) = (f.square(0.0, 0.0, 0.01), f.square(0.0, 0.0, 0.24));
    loft(
        &mut t.light,
        &narrow,
        NEEDLE_TIP - 0.5,
        &wide,
        NEEDLE_TIP,
        tip,
        false,
    );
    loft(
        &mut t.light,
        &wide,
        NEEDLE_TIP,
        &narrow,
        NEEDLE_TIP + 0.7,
        tip,
        false,
    );
}

impl Frame {
    /// The server angle this frame faces.
    fn angle(&self) -> f64 {
        (self.f[1] as f64).atan2(self.f[0] as f64).to_degrees()
    }
}

/// Builds and uploads the tower for these rings (once per terrain).
pub fn upload(gpu: &mut ExtensionGpu, rings: &[WaterRing]) {
    let site = Site::new(rings);
    let t = build(&site);
    t.stone.upload(gpu, MESH, 0.8);
    t.glass.upload_material(
        gpu,
        ImportedMaterial {
            roughness: 0.18,
            ..ImportedMaterial::vertex_colored(GLASS_MESH)
        },
    );
    let mut gold = t.gold;
    gold.append(t.light.glowing());
    gold.upload_material(
        gpu,
        ImportedMaterial {
            metallic: 0.85,
            roughness: 0.35,
            ..ImportedMaterial::vertex_colored(GOLD_MESH)
        },
    );
    tracing::debug!(
        "tower: {} vertices, {} triangles in 3 meshes",
        gold.vertices.len(),
        gold.indices.len() / 3
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::mesh::facing;
    use crate::world::mesh::probe::triangles;
    use std::path::Path;

    fn rings() -> Vec<WaterRing> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
        terrain.water_rings
    }

    fn parts(t: &Tower) -> [(&str, &MeshBuf); 4] {
        [
            ("stone", &t.stone),
            ("glass", &t.glass),
            ("gold", &t.gold),
            ("light", &t.light),
        ]
    }

    #[test]
    fn the_site_is_the_island_inside_the_moat_with_a_gate_at_its_bridge() {
        // The fixture's moat is 20 wide at r = 200, so the water starts at
        // 190 units (19 m) and the island stops a metre short of it.
        let site = Site::new(&rings());
        assert!((site.island - 18.0).abs() < 1e-4, "{site:?}");
        assert!((site.water - 19.0).abs() < 1e-4);
        assert_eq!(site.gates, vec![(90.0, 1.5)]);
        assert_eq!(site.front(), 90.0);
    }

    #[test]
    fn nothing_stands_over_the_moat() {
        let site = Site::new(&rings());
        let t = build(&site);
        for (name, m) in parts(&t) {
            for v in &m.vertices {
                let r = v.position[0].hypot(v.position[2]);
                assert!(r < site.water, "{name} vertex at r {r} m");
            }
        }
    }

    /// Nothing but the floor and its light is in the way of someone
    /// walking off the bridge through the gate to the keep: a box the
    /// deck's width, from the paving to just under the passage's ceiling.
    #[test]
    fn each_gate_is_open_over_the_bridge_deck() {
        let site = Site::new(&rings());
        let t = build(&site);
        for &(angle, half) in &site.gates {
            let f = Frame::toward(angle);
            let local = |p: [f32; 3]| {
                let (x, z) = (p[0], p[2]);
                (x * f.f[0] + z * f.f[1], x * f.a[0] + z * f.a[1], p[1])
            };
            for (name, m) in parts(&t) {
                for tri in triangles(m) {
                    let pts = tri.map(|v| local(v.position));
                    let lo =
                        |k: fn(&(f32, f32, f32)) -> f32| pts.iter().map(k).fold(f32::MAX, f32::min);
                    let hi =
                        |k: fn(&(f32, f32, f32)) -> f32| pts.iter().map(k).fold(f32::MIN, f32::max);
                    let blocks = lo(|p| p.0) < site.water
                        && hi(|p| p.0) > PODIUM + 0.1
                        && lo(|p| p.1) < half + 0.2
                        && hi(|p| p.1) > -half - 0.2
                        && lo(|p| p.2) < PASSAGE_TOP - 0.1
                        && hi(|p| p.2) > layer::BAILEY + 0.1;
                    assert!(!blocks, "{name} triangle in the gate at {angle}°: {pts:?}");
                }
            }
        }
    }

    #[test]
    fn every_face_faces_its_normal() {
        let t = build(&Site::new(&rings()));
        for (name, m) in parts(&t) {
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
    fn the_light_glows_and_the_point_of_light_on_the_needle_is_the_top() {
        let t = build(&Site::new(&rings()));
        let top = |m: &MeshBuf| {
            m.vertices
                .iter()
                .map(|v| v.position[1])
                .fold(f32::MIN, f32::max)
        };
        assert!(top(&t.light) > top(&t.gold));
        assert!(top(&t.gold) > top(&t.stone).max(top(&t.glass)));
        assert!(
            top(&t.light) - G > 80.0,
            "the tower is {} m",
            top(&t.light) - G
        );
        assert!(CROWN_Y > top(&t.light));
        let lit = t.light.glowing();
        assert!(lit
            .vertices
            .iter()
            .all(|v| v.normal == crate::world::mesh::GLOW));
    }

    #[test]
    fn with_no_moat_the_wall_closes_and_every_tower_stands() {
        let site = Site::new(&[]);
        assert_eq!(stretches(&site), vec![(0.0, 360.0)]);
        let t = build(&site);
        // Eight gold collars, each a 12-sided prism with a roof: a fan of
        // the centre and its rim, closed.
        let collars = t
            .gold
            .vertices
            .iter()
            .filter(|v| v.position[1] == TOWER_TOP + 0.25 && v.normal == UP)
            .count();
        assert_eq!(collars, 8 * 14);
    }

    #[test]
    fn it_is_cheap_to_draw() {
        let t = build(&Site::new(&rings()));
        let triangles: usize = parts(&t).iter().map(|(_, m)| m.indices.len() / 3).sum();
        let vertices: usize = parts(&t).iter().map(|(_, m)| m.vertices.len()).sum();
        println!("tower: {vertices} vertices, {triangles} triangles");
        assert!(triangles < 20_000, "{triangles} triangles");
    }
}
