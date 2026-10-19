//! The node models (E2.5): one procedural model per resource type, each
//! with a full look, the same with its tips lit gold (you're going to it,
//! or someone is working it), and a spent look once it's been taken.
//!
//! - **Wild grass**: the P4.4 tuft, and stubble.
//! - **Trees**: a trunk and a few canopy lobes, or tiers for a pine. Four
//!   species (oak, beech, birch, pine), two shapes each, picked by zone
//!   and node id ([`Kind::variant_for`]). Taken, a stump with its cut face
//!   pale and chips round it.
//! - **Fallen branches**: sticks and a fork lying across leaf litter.
//!   Taken, the litter and a twig.
//! - **Rock outcrops**: a faceted boulder and a smaller one, lichen on the
//!   faces that look up. Taken, a low broken rock in its rubble.
//! - **Iron veins**: darker rock with rust streaks that give off a little
//!   light, so they catch the dusk. Taken, the streaks dug out.
//! - **Copper and tin veins** (#225): the same, copper's streaks verdigris
//!   and bright metal in a warmer rock, tin's pale silver in a dark one.
//! - **Reed beds** (#225): tall strap leaves and cattails standing in the
//!   shallows. Taken, cut stems and a few leaves lying on the water.
//! - **Clay banks**: a low wet mound with a spade-cut face. Taken, a
//!   dug-out hollow inside its lip.
//!
//! Every model stands with its base at local y = 0, so placing it is one
//! function (`super::ground_at`) that W9 will lift onto the land. A
//! resource id the client doesn't know gets a little cairn ([`Kind::Other`])
//! rather than nothing, so new content never breaks the world.

use crate::world::mesh::{MeshBuf, UP};
use crate::world::palette::{glow, hex, lit_glow, mix, GOLD};
use capitol_protocol::rules::Zone;
use std::f32::consts::{PI, TAU};

/// What a node looks like, from its resource type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Grass,
    Tree,
    Branches,
    Outcrop,
    Vein,
    Clay,
    Copper,
    Tin,
    Reeds,
    /// A resource the client has no model for.
    Other,
}

/// How a node is shown: there to take, or taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Look {
    Full,
    Spent,
}

impl Kind {
    pub const ALL: [Kind; 10] = [
        Kind::Grass,
        Kind::Tree,
        Kind::Branches,
        Kind::Outcrop,
        Kind::Vein,
        Kind::Clay,
        Kind::Copper,
        Kind::Tin,
        Kind::Reeds,
        Kind::Other,
    ];

    /// The model for a catalog resource id (`backend/content/resources.toml`).
    pub fn of(resource_type: &str) -> Kind {
        match resource_type {
            "grass" => Kind::Grass,
            // The wilds' old growth is trees too: pine and birch out there.
            "tree" | "old_growth" => Kind::Tree,
            "fallen_branches" => Kind::Branches,
            "rock_outcrop" => Kind::Outcrop,
            "iron_vein" => Kind::Vein,
            "clay_bank" => Kind::Clay,
            "copper_vein" => Kind::Copper,
            "tin_vein" => Kind::Tin,
            "reed_bed" => Kind::Reeds,
            _ => Kind::Other,
        }
    }

    /// For mesh names and the scripts.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Grass => "grass",
            Kind::Tree => "tree",
            Kind::Branches => "branches",
            Kind::Outcrop => "outcrop",
            Kind::Vein => "vein",
            Kind::Clay => "clay",
            Kind::Copper => "copper",
            Kind::Tin => "tin",
            Kind::Reeds => "reeds",
            Kind::Other => "other",
        }
    }

    /// How many shapes the kind comes in. Trees are four species of two.
    pub fn variants(self) -> u8 {
        match self {
            Kind::Grass | Kind::Other => 1,
            Kind::Tree => 8,
            Kind::Outcrop => 4,
            Kind::Branches | Kind::Vein | Kind::Clay | Kind::Copper | Kind::Tin => 3,
            Kind::Reeds => 2,
        }
    }

    /// The shape a node takes, from its id hash and where it stands. A
    /// tree's species leans on the zone: oak and beech in the farmland,
    /// pine and birch out in the wilds, birch and beech in town.
    pub fn variant_for(self, hash: u64, zone: Zone) -> u8 {
        if self != Kind::Tree {
            return (hash % self.variants() as u64) as u8;
        }
        // Oak, beech, birch, pine.
        let weights: [u32; 4] = match zone {
            Zone::Rural => [40, 30, 15, 15],
            Zone::Wilderness => [15, 10, 25, 50],
            _ => [10, 40, 50, 0],
        };
        let total: u32 = weights.iter().sum();
        let mut pick = ((hash >> 8) % total as u64) as u32;
        let mut species = 0;
        for (i, w) in weights.iter().enumerate() {
            if pick < *w {
                species = i;
                break;
            }
            pick -= w;
        }
        species as u8 * 2 + (hash % 2) as u8
    }

    /// How much one node's size varies either side of 1.
    pub fn size_range(self) -> (f32, f32) {
        match self {
            Kind::Grass => (0.88, 1.12),
            Kind::Tree => (0.82, 1.2),
            Kind::Outcrop | Kind::Vein | Kind::Copper | Kind::Tin => (0.8, 1.25),
            Kind::Branches => (0.85, 1.15),
            Kind::Clay => (0.85, 1.2),
            Kind::Reeds => (0.85, 1.15),
            Kind::Other => (0.9, 1.1),
        }
    }

    /// The radius of the ring the scripts draw round it, metres: clear of
    /// a trunk, a rock or a bank.
    pub fn ring(self) -> f32 {
        match self {
            Kind::Grass => 1.0,
            Kind::Tree => 1.1,
            Kind::Branches => 1.2,
            Kind::Outcrop | Kind::Vein | Kind::Copper | Kind::Tin => 1.5,
            Kind::Clay => 1.6,
            Kind::Reeds => 1.3,
            Kind::Other => 1.0,
        }
    }

    /// Its grass clearing (H25), server units: bare out to the first, then
    /// soft over the second. A tree's is shade, wide and soft; a rock's
    /// hugs it; branches lie in the grass.
    pub fn clearing(self) -> (f64, f64) {
        match self {
            Kind::Grass => (
                crate::world::grass::NODE_CLEAR,
                crate::world::grass::NODE_SOFT,
            ),
            Kind::Tree => (14.0, 26.0),
            Kind::Branches => (9.0, 12.0),
            Kind::Outcrop | Kind::Vein | Kind::Copper | Kind::Tin => (17.0, 12.0),
            Kind::Clay => (20.0, 14.0),
            Kind::Reeds => (8.0, 8.0),
            Kind::Other => (12.0, 10.0),
        }
    }

    /// The particle effect a worked node gives off (`particles/`).
    pub fn effect(self) -> Option<&'static str> {
        match self {
            Kind::Tree | Kind::Branches => Some("gather_wood"),
            Kind::Outcrop | Kind::Vein | Kind::Copper | Kind::Tin | Kind::Other => Some("gather_stone"),
            Kind::Clay => Some("gather_clay"),
            Kind::Grass | Kind::Reeds => None,
        }
    }

    /// The colour of the bits the scripts throw when it's taken (sRGB).
    pub fn bits(self) -> [f32; 3] {
        match self {
            Kind::Grass => [0.42, 0.64, 0.47],
            Kind::Tree | Kind::Branches => [0.80, 0.66, 0.45],
            Kind::Outcrop | Kind::Other => [0.62, 0.60, 0.56],
            Kind::Vein => [0.70, 0.40, 0.22],
            Kind::Copper => [0.38, 0.62, 0.52],
            Kind::Tin => [0.72, 0.74, 0.77],
            Kind::Clay => [0.55, 0.36, 0.24],
            Kind::Reeds => [0.62, 0.58, 0.40],
        }
    }

    /// Seconds to go when taken: cut, felled, broken, gathered up.
    pub fn exit_secs(self) -> f32 {
        match self {
            Kind::Grass => 0.28,
            Kind::Tree => 1.2,
            Kind::Outcrop | Kind::Vein | Kind::Copper | Kind::Tin => 0.55,
            _ => 0.4,
        }
    }

    /// Seconds to come back. Regrowth eases: a tree takes its time.
    pub fn regrow_secs(self) -> f32 {
        match self {
            Kind::Grass | Kind::Reeds => 1.3,
            Kind::Tree => 3.2,
            Kind::Outcrop | Kind::Vein | Kind::Copper | Kind::Tin => 1.8,
            Kind::Clay => 1.5,
            Kind::Branches | Kind::Other => 1.0,
        }
    }

    /// How it moves while worked: lean in radians, and how fast.
    pub fn sway(self) -> (f32, f32) {
        match self {
            Kind::Grass => (0.11, 1.3),
            Kind::Reeds => (0.09, 1.6),
            Kind::Tree => (0.035, 2.4),
            Kind::Outcrop | Kind::Vein | Kind::Copper | Kind::Tin | Kind::Other => (0.012, 9.0),
            Kind::Branches => (0.02, 5.0),
            Kind::Clay => (0.0, 0.0),
        }
    }
}

/// The runtime mesh for one kind, shape and look.
pub fn mesh_name(kind: Kind, variant: u8, look: Look, lit: bool) -> String {
    let look = match (look, lit) {
        (Look::Spent, _) => "spent",
        (Look::Full, false) => "full",
        (Look::Full, true) => "lit",
    };
    format!("node_{}_{variant}_{look}", kind.name())
}

/// Builds one model.
pub fn model(kind: Kind, variant: u8, look: Look, lit: bool) -> MeshBuf {
    let seed = 0x9e37_79b9 ^ ((kind as u32) << 24) ^ ((variant as u32) << 16);
    let m = match (kind, look) {
        (Kind::Grass, Look::Full) => return tuft(lit),
        (Kind::Grass, Look::Spent) => return stubble(),
        (Kind::Tree, Look::Full) => tree(variant, &mut Rng(seed)),
        (Kind::Tree, Look::Spent) => stump(variant, &mut Rng(seed)),
        (Kind::Branches, Look::Full) => branches(&mut Rng(seed)),
        (Kind::Branches, Look::Spent) => litter(&mut Rng(seed), 1),
        (Kind::Outcrop, Look::Full) => outcrop(&mut Rng(seed)),
        (Kind::Outcrop, Look::Spent) => broken(&mut Rng(seed), None),
        (Kind::Vein, Look::Full) => vein(&mut Rng(seed), IRONSTONE, RUST),
        (Kind::Vein, Look::Spent) => broken(&mut Rng(seed), Some((IRONSTONE, RUST[0]))),
        (Kind::Copper, Look::Full) => vein(&mut Rng(seed), COPPER_STONE, COPPER),
        (Kind::Copper, Look::Spent) => broken(&mut Rng(seed), Some((COPPER_STONE, COPPER[1]))),
        (Kind::Tin, Look::Full) => vein(&mut Rng(seed), TIN_STONE, TIN),
        (Kind::Tin, Look::Spent) => broken(&mut Rng(seed), Some((TIN_STONE, TIN[0]))),
        (Kind::Reeds, Look::Full) => reeds(&mut Rng(seed)),
        (Kind::Reeds, Look::Spent) => cut_reeds(&mut Rng(seed)),
        (Kind::Clay, Look::Full) => bank(&mut Rng(seed)),
        (Kind::Clay, Look::Spent) => dug(&mut Rng(seed)),
        (Kind::Other, Look::Full) => cairn(&mut Rng(seed), 3),
        (Kind::Other, Look::Spent) => cairn(&mut Rng(seed), 1),
    };
    if lit && look == Look::Full {
        lit_tips(m, 0.6)
    } else {
        m
    }
}

/// The model with its upper part catching gold: the higher, the more, the
/// way the tuft's tips light.
fn lit_tips(mut m: MeshBuf, from: f32) -> MeshBuf {
    let top = m
        .vertices
        .iter()
        .map(|v| v.position[1])
        .fold(0.0f32, f32::max);
    let lo = top * from;
    for v in &mut m.vertices {
        let t = ((v.position[1] - lo) / (top - lo).max(1e-3)).clamp(0.0, 1.0);
        if t > 0.0 {
            let c = [v.color[0], v.color[1], v.color[2], 1.0];
            v.color = lit_glow(mix(c, hex(GOLD), 0.65 * t), 1.0 - 0.4 * t, 0.9 * t);
        }
    }
    m
}

// ── Shared pieces ────────────────────────────────────────────────────────

/// A little generator of numbers in [0, 1), so every model is the same
/// model and a test can rebuild it.
pub struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

pub fn neg([x, y, z]: [f32; 3]) -> [f32; 3] {
    [-x, -y, -z]
}

pub fn normalise([x, y, z]: [f32; 3]) -> [f32; 3] {
    let len = (x * x + y * y + z * z).sqrt().max(1e-6);
    [x / len, y / len, z / len]
}

/// A flat triangle facing away from `inside`.
fn tri(m: &mut MeshBuf, a: [f32; 3], b: [f32; 3], c: [f32; 3], inside: [f32; 3], colour: [f32; 4]) {
    let mut n = normalise(cross(sub(b, a), sub(c, a)));
    let mid = scale(add(add(a, b), c), 1.0 / 3.0);
    if dot(n, sub(mid, inside)) < 0.0 {
        n = neg(n);
    }
    m.fan(a, &[b, c], n, colour);
}

/// A flat quad facing away from `inside`.
fn face(m: &mut MeshBuf, q: [[f32; 3]; 4], inside: [f32; 3], colour: [f32; 4]) {
    let mut n = normalise(cross(sub(q[1], q[0]), sub(q[2], q[0])));
    let mid = scale(add(add(q[0], q[1]), add(q[2], q[3])), 0.25);
    if dot(n, sub(mid, inside)) < 0.0 {
        n = neg(n);
    }
    m.quad(q, n, colour);
}

/// Two unit vectors square to `axis` and to each other.
fn basis(axis: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let a = normalise(axis);
    let helper = if a[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = normalise(cross(a, helper));
    let v = cross(a, u);
    (u, v)
}

/// A tapering many-sided post from `base` (radius `r0`, colour `c0`) to
/// `top` (radius `r1`, colour `c1`), flat-shaded, capped at the top if
/// `cap` gives a colour. `r1` 0 is a cone.
#[allow(clippy::too_many_arguments)]
fn post(
    m: &mut MeshBuf,
    base: [f32; 3],
    top: [f32; 3],
    r0: f32,
    r1: f32,
    sides: usize,
    c0: [f32; 4],
    c1: [f32; 4],
    cap: Option<[f32; 4]>,
) {
    let (u, v) = basis(sub(top, base));
    let ring = |c: [f32; 3], r: f32, i: usize| {
        let a = i as f32 / sides as f32 * TAU;
        add(c, add(scale(u, r * a.cos()), scale(v, r * a.sin())))
    };
    for i in 0..sides {
        let q = [
            ring(base, r0, i),
            ring(base, r0, i + 1),
            ring(top, r1, i + 1),
            ring(top, r1, i),
        ];
        let colour = mix(c0, c1, 0.35 + 0.3 * ((i as f32 * 1.7).sin() * 0.5 + 0.5));
        let mid = scale(add(base, top), 0.5);
        if r1 <= 1e-4 {
            tri(m, q[0], q[1], top, mid, colour);
        } else {
            face(m, q, mid, colour);
        }
    }
    if let (Some(c), true) = (cap, r1 > 1e-4) {
        let rim: Vec<[f32; 3]> = (0..=sides).map(|i| ring(top, r1, i)).collect();
        let n = normalise(sub(top, base));
        m.fan(top, &rim, n, c);
    }
}

/// A lumpy, faceted ball: `segs` round and `rings` from pole to pole,
/// each vertex pushed in or out by up to `jitter` of its radius. Each face
/// takes its colour from `paint`, given its outward normal and its centre.
#[allow(clippy::too_many_arguments)]
fn lump(
    m: &mut MeshBuf,
    centre: [f32; 3],
    radii: [f32; 3],
    segs: usize,
    rings: usize,
    jitter: f32,
    rng: &mut Rng,
    paint: &mut dyn FnMut([f32; 3], [f32; 3]) -> [f32; 4],
) {
    // One push per vertex, shared by its faces, so the lump stays closed.
    let mut push = vec![vec![1.0f32; segs]; rings + 1];
    for row in push.iter_mut().take(rings).skip(1) {
        for p in row.iter_mut() {
            *p = 1.0 + rng.range(-jitter, jitter);
        }
    }
    let at = |i: usize, j: usize, push: &Vec<Vec<f32>>| {
        let lat = PI * j as f32 / rings as f32;
        let lon = TAU * (i % segs) as f32 / segs as f32;
        let k = push[j][i % segs];
        [
            centre[0] + radii[0] * k * lat.sin() * lon.cos(),
            centre[1] + radii[1] * k * lat.cos(),
            centre[2] + radii[2] * k * lat.sin() * lon.sin(),
        ]
    };
    for j in 0..rings {
        for i in 0..segs {
            let a = at(i, j, &push);
            let b = at(i + 1, j, &push);
            let c = at(i + 1, j + 1, &push);
            let d = at(i, j + 1, &push);
            if j == 0 {
                let n = outward(a, c, d, centre);
                let colour = paint(n, a);
                tri(m, a, c, d, centre, colour);
            } else if j + 1 == rings {
                let n = outward(a, b, c, centre);
                let colour = paint(n, a);
                tri(m, a, b, c, centre, colour);
            } else {
                // Split into two triangles: four jittered points needn't
                // lie in a plane.
                let n = outward(a, b, c, centre);
                let colour = paint(n, scale(add(a, c), 0.5));
                tri(m, a, b, c, centre, colour);
                tri(m, a, c, d, centre, colour);
            }
        }
    }
}

fn outward(a: [f32; 3], b: [f32; 3], c: [f32; 3], inside: [f32; 3]) -> [f32; 3] {
    let n = normalise(cross(sub(b, a), sub(c, a)));
    let mid = scale(add(add(a, b), c), 1.0 / 3.0);
    if dot(n, sub(mid, inside)) < 0.0 {
        neg(n)
    } else {
        n
    }
}

/// A flat, many-sided patch lying on the ground at height `y`.
fn patch(
    m: &mut MeshBuf,
    c: [f32; 2],
    r: [f32; 2],
    turn: f32,
    y: f32,
    sides: usize,
    colour: [f32; 4],
) {
    let (s, k) = turn.sin_cos();
    let rim: Vec<[f32; 3]> = (0..=sides)
        .map(|i| {
            let a = i as f32 / sides as f32 * TAU;
            let (x, z) = (r[0] * a.cos(), r[1] * a.sin());
            [c[0] + x * k - z * s, y, c[1] + x * s + z * k]
        })
        .collect();
    m.fan([c[0], y, c[1]], &rim, UP, colour);
}

// ── Trees ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    Broad,
    Tall,
    Slender,
    Conifer,
}

struct Species {
    form: Form,
    bark: u32,
    bark_dark: u32,
    /// Shade, mid and light leaf.
    leaf: [u32; 3],
    /// The pale wood of a cut.
    heart: u32,
}

/// Oak, beech, birch, pine: greens that sit under the dusk without
/// shouting, the pine bluer, the birch yellower.
const SPECIES: [Species; 4] = [
    Species {
        form: Form::Broad,
        bark: 0x5c4b3c,
        bark_dark: 0x3e3229,
        leaf: [0x22331b, 0x354c24, 0x4f6633],
        heart: 0xc6a477,
    },
    Species {
        form: Form::Tall,
        bark: 0x7b7a72,
        bark_dark: 0x56554f,
        leaf: [0x2a3f1d, 0x40592a, 0x60793a],
        heart: 0xd4b88e,
    },
    Species {
        form: Form::Slender,
        bark: 0xd9d4c6,
        bark_dark: 0x3a3632,
        leaf: [0x3c5222, 0x587032, 0x7c8f46],
        heart: 0xe0cfa8,
    },
    Species {
        form: Form::Conifer,
        bark: 0x5b4231,
        bark_dark: 0x3a2a20,
        leaf: [0x16291f, 0x21392d, 0x31503f],
        heart: 0xc9a46c,
    },
];

fn species(variant: u8) -> &'static Species {
    &SPECIES[(variant / 2).min(3) as usize]
}

/// Paints a canopy face: shade underneath, light on top, a little noise
/// between, so the low lobes read as leaves under any light.
fn leaf_paint(sp: &Species, rng: &mut Rng) -> impl FnMut([f32; 3], [f32; 3]) -> [f32; 4] {
    let [dark, mid, light] = sp.leaf.map(hex);
    let mut r = Rng(rng.0 ^ 0x5bd1_e995);
    move |n: [f32; 3], _p: [f32; 3]| {
        let up = n[1] * 0.5 + 0.5;
        let base = if up < 0.5 {
            mix(dark, mid, up * 2.0)
        } else {
            mix(mid, light, (up - 0.5) * 2.0)
        };
        mix(
            base,
            if r.next() < 0.5 { dark } else { light },
            r.range(0.0, 0.18),
        )
    }
}

/// A tree, base at the origin: 5 to 8 m tall.
fn tree(variant: u8, rng: &mut Rng) -> MeshBuf {
    let sp = species(variant);
    let mut m = MeshBuf::default();
    let shape = variant % 2;
    let bark = hex(sp.bark);
    let dark = hex(sp.bark_dark);
    match sp.form {
        Form::Conifer => {
            let h = if shape == 0 { 7.2 } else { 6.2 };
            post(
                &mut m,
                [0.0, -0.05, 0.0],
                [0.0, 1.2, 0.0],
                0.28,
                0.2,
                6,
                dark,
                bark,
                None,
            );
            post(
                &mut m,
                [0.0, 1.2, 0.0],
                [0.0, h, 0.0],
                0.2,
                0.04,
                6,
                dark,
                bark,
                None,
            );
            let tiers = if shape == 0 { 5 } else { 4 };
            let mut paint = leaf_paint(sp, rng);
            for k in 0..tiers {
                let t = k as f32 / tiers as f32;
                let y0 = 1.3 + t * (h - 1.9);
                let r = (2.0 - 1.55 * t) * rng.range(0.92, 1.08);
                let top = y0 + 1.7 - 0.5 * t;
                cone_tier(&mut m, [0.0, y0, 0.0], r, top - y0, 9, rng, &mut paint);
            }
        }
        Form::Broad | Form::Tall | Form::Slender => {
            let (trunk_h, r, crown, lobes, lift) = match (sp.form, shape) {
                (Form::Broad, 0) => (1.9, 0.3, 2.3, 6, 3.7),
                (Form::Broad, _) => (1.6, 0.27, 2.0, 5, 3.2),
                (Form::Tall, 0) => (2.6, 0.24, 1.9, 5, 4.9),
                (Form::Tall, _) => (2.2, 0.22, 1.7, 4, 4.3),
                (_, 0) => (2.8, 0.13, 1.25, 5, 4.6),
                (_, _) => (2.4, 0.12, 1.1, 4, 4.0),
            };
            // A flared root, the trunk, and a lean.
            let lean = [rng.range(-0.25, 0.25), 0.0, rng.range(-0.25, 0.25)];
            let top = [lean[0], trunk_h, lean[2]];
            post(
                &mut m,
                [0.0, -0.05, 0.0],
                [0.0, 0.35, 0.0],
                r * 1.45,
                r,
                7,
                dark,
                bark,
                None,
            );
            post(
                &mut m,
                [0.0, 0.35, 0.0],
                top,
                r,
                r * 0.7,
                7,
                bark,
                bark,
                None,
            );
            if sp.form == Form::Slender {
                // A birch's dark marks, rings round the white.
                for k in 0..4 {
                    let y = 0.6 + k as f32 * rng.range(0.4, 0.6);
                    if y + 0.08 > trunk_h {
                        break;
                    }
                    let f = y / trunk_h;
                    let at = |y: f32| [lean[0] * y / trunk_h, y, lean[2] * y / trunk_h];
                    let rr = r * (1.0 - 0.3 * f) + 0.006;
                    post(&mut m, at(y), at(y + 0.07), rr, rr, 7, dark, dark, None);
                }
            }
            // Boughs up into the crown, then the lobes on them.
            let centre = [lean[0] * 1.4, lift, lean[2] * 1.4];
            let mut paint = leaf_paint(sp, rng);
            let first = rng.range(0.0, TAU);
            for k in 0..lobes {
                let (offset, rr, dy) = if k == 0 {
                    ([0.0, 0.0, 0.0], crown, crown * 0.15)
                } else {
                    let a = first + k as f32 * 2.399_963;
                    let out = crown * rng.range(0.5, 0.75);
                    let rr = crown * rng.range(0.55, 0.72);
                    (
                        [out * a.cos(), 0.0, out * a.sin()],
                        rr,
                        rng.range(-0.45, 0.1) * crown,
                    )
                };
                let c = add(centre, add(offset, [0.0, dy, 0.0]));
                if k > 0 {
                    let from = add(top, scale(sub(c, top), 0.05));
                    let to = add(top, scale(sub(c, top), 0.7));
                    post(&mut m, from, to, r * 0.45, r * 0.18, 5, bark, bark, None);
                }
                let flat = match sp.form {
                    Form::Tall => 1.15,
                    Form::Slender => 1.25,
                    _ => 0.85,
                };
                lump(&mut m, c, [rr, rr * flat, rr], 8, 5, 0.16, rng, &mut paint);
            }
        }
    }
    m
}

/// One skirt of a pine: a wide shallow cone, its rim ragged, and a dark
/// underside.
fn cone_tier(
    m: &mut MeshBuf,
    base: [f32; 3],
    r: f32,
    h: f32,
    sides: usize,
    rng: &mut Rng,
    paint: &mut dyn FnMut([f32; 3], [f32; 3]) -> [f32; 4],
) {
    let tip = add(base, [0.0, h, 0.0]);
    let turn = rng.range(0.0, TAU);
    let rim: Vec<[f32; 3]> = (0..sides)
        .map(|i| {
            let a = turn + i as f32 / sides as f32 * TAU;
            let rr = r * rng.range(0.82, 1.1);
            add(base, [rr * a.cos(), rng.range(-0.18, 0.08), rr * a.sin()])
        })
        .collect();
    let inside = add(base, [0.0, h * 0.3, 0.0]);
    for i in 0..sides {
        let (a, b) = (rim[i], rim[(i + 1) % sides]);
        let n = outward(a, b, tip, inside);
        let colour = paint(n, a);
        tri(m, a, b, tip, inside, colour);
        let under = mix(colour, hex(0x0a120c), 0.45);
        let hub = add(base, [0.0, 0.25, 0.0]);
        let n = outward(a, b, hub, add(hub, [0.0, 1.0, 0.0]));
        let _ = n;
        tri(m, a, b, hub, add(hub, [0.0, 1.0, 0.0]), under);
    }
}

/// What's left of a felled tree: a stump with a pale cut face, the
/// heartwood rings darker, and chips round it.
fn stump(variant: u8, rng: &mut Rng) -> MeshBuf {
    let sp = species(variant);
    let mut m = MeshBuf::default();
    let r = match sp.form {
        Form::Slender => 0.15,
        Form::Conifer => 0.24,
        Form::Tall => 0.25,
        Form::Broad => 0.3,
    };
    let (bark, dark, heart) = (hex(sp.bark), hex(sp.bark_dark), hex(sp.heart));
    let h = rng.range(0.38, 0.5);
    post(
        &mut m,
        [0.0, -0.05, 0.0],
        [0.0, 0.2, 0.0],
        r * 1.5,
        r * 1.08,
        7,
        dark,
        bark,
        None,
    );
    post(
        &mut m,
        [0.0, 0.2, 0.0],
        [0.0, h, 0.0],
        r * 1.08,
        r,
        7,
        bark,
        bark,
        None,
    );
    // The cut, slightly slanted, in rings.
    for (k, (rr, c)) in [
        (r * 0.98, mix(heart, dark, 0.35)),
        (r * 0.85, heart),
        (r * 0.45, mix(heart, hex(0x8a6a44), 0.4)),
        (r * 0.12, mix(heart, hex(0x5a4430), 0.5)),
    ]
    .into_iter()
    .enumerate()
    {
        patch(
            &mut m,
            [0.0, 0.0],
            [rr, rr],
            0.0,
            h + 0.002 * k as f32,
            7,
            c,
        );
    }
    // Chips and a sawdust stain.
    patch(
        &mut m,
        [0.1, 0.05],
        [r * 3.2, r * 2.6],
        rng.range(0.0, PI),
        0.012,
        9,
        mix(heart, hex(0x4a3a2a), 0.55),
    );
    for _ in 0..7 {
        let a = rng.range(0.0, TAU);
        let d = rng.range(r * 1.6, r * 4.0);
        let s = rng.range(0.04, 0.08);
        patch(
            &mut m,
            [d * a.cos(), d * a.sin()],
            [s, s * 0.5],
            rng.range(0.0, PI),
            0.022,
            4,
            mix(heart, hex(0xffffff), 0.12),
        );
    }
    m
}

// ── Fallen branches ──────────────────────────────────────────────────────

const STICK: u32 = 0x7a6048;
const STICK_DARK: u32 = 0x3d2e22;
const LEAVES: [u32; 4] = [0x7a5e2c, 0x8a5428, 0x66642e, 0x9a7a3c];

/// Sticks and a forked bough lying in leaf litter, about 1.6 m across.
fn branches(rng: &mut Rng) -> MeshBuf {
    let mut m = litter(rng, 9);
    let (c0, c1) = (hex(STICK_DARK), hex(STICK));
    let sticks = 4 + (rng.next() * 3.0) as usize;
    for k in 0..sticks {
        let a = rng.range(0.0, TAU);
        let len = if k == 0 { 1.5 } else { rng.range(0.5, 1.1) };
        let r = if k == 0 {
            0.055
        } else {
            rng.range(0.022, 0.04)
        };
        let mid = [rng.range(-0.35, 0.35), rng.range(-0.35, 0.35)];
        let d = [a.cos() * len * 0.5, a.sin() * len * 0.5];
        let y = r + 0.02 + 0.03 * k as f32 / sticks as f32;
        let p0 = [mid[0] - d[0], y, mid[1] - d[1]];
        let p1 = [mid[0] + d[0], y + rng.range(0.0, 0.08), mid[1] + d[1]];
        post(&mut m, p0, p1, r, r * 0.6, 5, c0, c1, Some(hex(0xa88a62)));
        if k == 0 {
            // The fork: two twigs off the bough.
            for side in [-1.0f32, 1.0] {
                let from = add(p0, scale(sub(p1, p0), rng.range(0.4, 0.7)));
                let b = a + side * rng.range(0.4, 0.8);
                let l = rng.range(0.35, 0.6);
                let to = add(from, [b.cos() * l, rng.range(0.02, 0.12), b.sin() * l]);
                post(&mut m, from, to, r * 0.5, r * 0.2, 4, c0, c1, None);
            }
        }
    }
    m
}

/// Leaf litter, `twigs` of them lying in it.
fn litter(rng: &mut Rng, leaves: usize) -> MeshBuf {
    let mut m = MeshBuf::default();
    patch(
        &mut m,
        [0.0, 0.0],
        [0.75, 0.6],
        rng.range(0.0, PI),
        0.008,
        10,
        hex(0x3e3424),
    );
    for _ in 0..leaves {
        let a = rng.range(0.0, TAU);
        let d = rng.range(0.0, 0.7);
        let s = rng.range(0.06, 0.11);
        let c = hex(LEAVES[(rng.next() * LEAVES.len() as f32) as usize % LEAVES.len()]);
        patch(
            &mut m,
            [d * a.cos(), d * a.sin()],
            [s, s * 0.55],
            rng.range(0.0, PI),
            0.016 + rng.range(0.0, 0.01),
            5,
            c,
        );
    }
    if leaves < 4 {
        let a = rng.range(0.0, TAU);
        let p0 = [-0.2 * a.cos(), 0.03, -0.2 * a.sin()];
        let p1 = [0.25 * a.cos(), 0.04, 0.25 * a.sin()];
        post(
            &mut m,
            p0,
            p1,
            0.018,
            0.01,
            4,
            hex(STICK_DARK),
            hex(STICK),
            None,
        );
    }
    m
}

// ── Rocks ────────────────────────────────────────────────────────────────

const STONE: [u32; 3] = [0x5f5c56, 0x77736b, 0x8f8a80];
const LICHEN: [u32; 2] = [0x8d9658, 0xb3ab72];
const IRONSTONE: [u32; 3] = [0x403c3a, 0x55504c, 0x6a645d];
const RUST: [u32; 3] = [0x54281a, 0x6a3420, 0x86482a];
/// Copper's rock is warmer, its streaks verdigris round bright metal.
const COPPER_STONE: [u32; 3] = [0x4a4038, 0x5e544a, 0x72685c];
const COPPER: [u32; 3] = [0x2f6b5c, 0x4f9a80, 0xb5703e];
/// Tin's (cassiterite) rock is dark, its streaks pale silver.
const TIN_STONE: [u32; 3] = [0x34343a, 0x46464e, 0x5a5a62];
const TIN: [u32; 3] = [0x8d9298, 0xaeb3b9, 0xd2d5d9];
const FRESH: u32 = 0x9c968a;

fn stone_paint(
    rng: &mut Rng,
    tones: [u32; 3],
    lichen: bool,
) -> impl FnMut([f32; 3], [f32; 3]) -> [f32; 4] {
    let [dark, mid, light] = tones.map(hex);
    let mut r = Rng(rng.0 ^ 0x2c1b_3c6d);
    move |n: [f32; 3], _p: [f32; 3]| {
        let up = n[1] * 0.5 + 0.5;
        let base = mix(
            mix(dark, mid, up.min(0.6) / 0.6),
            light,
            ((up - 0.6) / 0.4).max(0.0),
        );
        let base = mix(base, dark, r.range(0.0, 0.2));
        if lichen && n[1] > 0.45 && r.next() < 0.5 {
            mix(
                base,
                hex(LICHEN[(r.next() * 2.0) as usize % 2]),
                r.range(0.45, 0.8),
            )
        } else {
            base
        }
    }
}

/// A rock outcrop: a faceted boulder half in the ground, a smaller one
/// leaning on it, lichen on top. About 1.6 m across and 1 m high.
fn outcrop(rng: &mut Rng) -> MeshBuf {
    let mut m = MeshBuf::default();
    let mut paint = stone_paint(rng, STONE, true);
    let w = rng.range(0.75, 0.95);
    lump(
        &mut m,
        [0.0, 0.25, 0.0],
        [w, 0.75, w * 0.8],
        7,
        5,
        0.22,
        rng,
        &mut paint,
    );
    let a = rng.range(0.0, TAU);
    lump(
        &mut m,
        [0.75 * a.cos(), 0.05, 0.75 * a.sin()],
        [0.42, 0.38, 0.36],
        6,
        4,
        0.2,
        rng,
        &mut paint,
    );
    pebbles(&mut m, rng, 3, 0.9, STONE[1]);
    m
}

/// An iron vein: dark rock with rust running through it in streaks, which
/// give off a little of their colour so they catch the light at dusk.
/// A vein: `stone` rock with `streak`s of ore that catch the light.
fn vein(rng: &mut Rng, stone: [u32; 3], streak: [u32; 3]) -> MeshBuf {
    let mut m = MeshBuf::default();
    let dir = normalise([
        rng.range(-1.0, 1.0),
        rng.range(0.4, 1.0),
        rng.range(-1.0, 1.0),
    ]);
    // Finer facets than an outcrop's, so a streak can be thin.
    let phase = rng.range(0.0, TAU);
    let mut stone = stone_paint(rng, stone, false);
    let rust = streak.map(hex);
    let mut r = Rng(rng.0 ^ 0x68e3_1da4);
    let mut paint = move |n: [f32; 3], p: [f32; 3]| {
        let band = (dot(p, dir) * 9.0 + phase).sin();
        if band > 0.86 {
            let c = mix(
                rust[(r.next() * 3.0) as usize % 3],
                rust[2],
                (n[1] * 0.5 + 0.5) * 0.4,
            );
            lit_glow(mix(stone(n, p), c, 0.75), 1.0, 0.03)
        } else {
            stone(n, p)
        }
    };
    let w = rng.range(0.8, 0.95);
    lump(
        &mut m,
        [0.0, 0.2, 0.0],
        [w, 0.7, w * 0.75],
        12,
        8,
        0.12,
        rng,
        &mut paint,
    );
    let a = rng.range(0.0, TAU);
    lump(
        &mut m,
        [0.7 * a.cos(), 0.0, 0.7 * a.sin()],
        [0.38, 0.32, 0.34],
        6,
        4,
        0.2,
        rng,
        &mut paint,
    );
    pebbles(&mut m, rng, 4, 0.95, streak[0]);
    m
}

/// A rock broken down: low, pale where it's freshly split, in its rubble.
/// A vein's (its rock, and its ore's pebbles) has dark gouges where the ore
/// came out.
fn broken(rng: &mut Rng, ore: Option<([u32; 3], u32)>) -> MeshBuf {
    let mut m = MeshBuf::default();
    let vein = ore.is_some();
    let tones = ore.map_or(STONE, |(stone, _)| stone);
    let mut stone = stone_paint(rng, tones, false);
    let mut r = Rng(rng.0 ^ 0x1b87_3593);
    let mut paint = move |n: [f32; 3], p: [f32; 3]| {
        let c = stone(n, p);
        if n[1] > 0.35 && r.next() < 0.55 {
            if vein {
                mix(c, hex(0x2a2624), 0.6)
            } else {
                mix(c, hex(FRESH), 0.6)
            }
        } else {
            c
        }
    };
    lump(
        &mut m,
        [0.0, -0.05, 0.0],
        [0.75, 0.36, 0.62],
        7,
        4,
        0.25,
        rng,
        &mut paint,
    );
    pebbles(&mut m, rng, 9, 1.0, ore.map_or(FRESH, |(_, pebble)| pebble));
    m
}

/// Small stones scattered round, `out` metres at most.
fn pebbles(m: &mut MeshBuf, rng: &mut Rng, n: usize, out: f32, tone: u32) {
    let mut paint = |nrm: [f32; 3], _p: [f32; 3]| mix(hex(tone), hex(0x2a2826), 0.5 - 0.3 * nrm[1]);
    for _ in 0..n {
        let a = rng.range(0.0, TAU);
        let d = rng.range(out * 0.6, out * 1.2);
        let s = rng.range(0.06, 0.14);
        lump(
            m,
            [d * a.cos(), 0.0, d * a.sin()],
            [s, s * 0.7, s],
            5,
            3,
            0.2,
            rng,
            &mut paint,
        );
    }
}

// ── Reeds ────────────────────────────────────────────────────────────────

const REED_DARK: u32 = 0x2f4430;
const REED: u32 = 0x5e7a48;
const REED_TIP: u32 = 0x9a9a62;
const CATTAIL: u32 = 0x4a3222;
const REED_CUT: u32 = 0x8a8358;

/// A reed bed, standing in the shallows: two dozen strap leaves, tall and
/// arching, and a few stems each with a cattail's brown head. About 2 m
/// tall and 1.6 m across. Its foot is under the water's line, which is
/// where W10's water will draw (#203).
fn reeds(rng: &mut Rng) -> MeshBuf {
    let mut m = MeshBuf::default();
    let mut tips = MeshBuf::default();
    for i in 0..26 {
        let a = i as f32 * 2.399_963 + rng.range(-0.2, 0.2);
        let r0 = rng.range(0.02, 0.3);
        let height = rng.range(1.2, 1.85);
        let lean = rng.range(0.15, 0.5);
        let shade = rng.next() * 0.5;
        let colours = [
            hex(REED_DARK),
            mix(hex(REED_DARK), hex(REED), 0.5 + shade),
            hex(REED),
        ];
        let tip = mix(hex(REED), hex(REED_TIP), 0.4 + shade);
        blade(&mut m, &mut tips, a, r0, height, lean, 0.06, colours, tip);
    }
    for _ in 0..5 {
        let a = rng.range(0.0, TAU);
        let (r0, h) = (rng.range(0.0, 0.22), rng.range(1.75, 2.15));
        let lean = rng.range(0.02, 0.12);
        let [dx, dz] = [a.cos(), a.sin()];
        let base = [r0 * dx, -0.2, r0 * dz];
        let top = [(r0 + lean) * dx, h, (r0 + lean) * dz];
        let stem = mix(hex(REED_DARK), hex(REED), 0.6);
        post(&mut m, base, top, 0.012, 0.008, 4, stem, stem, None);
        // The head: a fat brown spike a little under the stem's tip.
        let k = |t: f32| add(base, scale(sub(top, base), t));
        let head = hex(CATTAIL);
        let (lo, hi) = (k(0.78), k(0.92));
        post(&mut m, lo, hi, 0.04, 0.04, 6, head, mix(head, hex(0x6a4a30), 0.4), Some(head));
    }
    m.append(tips);
    m
}

/// A reed bed cut down: short stems, flat across the top, and a few
/// leaves lying on the water.
fn cut_reeds(rng: &mut Rng) -> MeshBuf {
    let mut m = MeshBuf::default();
    for i in 0..14 {
        let a = i as f32 / 14.0 * TAU + rng.range(-0.25, 0.25);
        let r0 = rng.range(0.03, 0.32);
        let h = rng.range(0.2, 0.42);
        let cut = mix(hex(REED_CUT), hex(REED_DARK), rng.next() * 0.5);
        stalk(&mut m, a, r0, h, 0.03, [hex(REED_DARK), cut, cut]);
    }
    for a in [0.6f32, 2.6, 4.7] {
        let [dx, dz] = [a.cos(), a.sin()];
        let [sx, sz] = [-dz * 0.05, dx * 0.05];
        let (r0, r1, y) = (0.2, 1.1, 0.02);
        m.quad(
            [
                [dx * r0 - sx, y, dz * r0 - sz],
                [dx * r0 + sx, y, dz * r0 + sz],
                [dx * r1 + sx * 0.4, y, dz * r1 + sz * 0.4],
                [dx * r1 - sx * 0.4, y, dz * r1 - sz * 0.4],
            ],
            UP,
            mix(hex(REED), hex(REED_CUT), 0.5),
        );
    }
    m
}

// ── Clay ─────────────────────────────────────────────────────────────────

const CLAY: [u32; 3] = [0x40342a, 0x5a4a3a, 0x766450];
const CLAY_CUT: u32 = 0x9a8466;
const WET: u32 = 0x3c3e2c;

/// A clay bank: a low wet mound, 2 m long, with a face cut into it where
/// someone has been at it with a spade, and a sheen of water at its foot.
fn bank(rng: &mut Rng) -> MeshBuf {
    let mut m = MeshBuf::default();
    let [dark, mid, light] = CLAY.map(hex);
    let mut r = Rng(rng.0 ^ 0x4cf5_ad43);
    let mut paint = move |n: [f32; 3], _p: [f32; 3]| {
        let up = n[1] * 0.5 + 0.5;
        mix(
            mix(dark, mid, up),
            light,
            ((up - 0.75) * 3.0).clamp(0.0, 1.0) * r.range(0.4, 1.0),
        )
    };
    patch(
        &mut m,
        [0.0, 0.0],
        [1.05, 0.8],
        0.0,
        0.006,
        12,
        lit_glow(hex(WET), 1.0, 0.03),
    );
    lump(
        &mut m,
        [0.0, -0.18, 0.0],
        [1.1, 0.55, 0.8],
        10,
        4,
        0.1,
        rng,
        &mut paint,
    );
    lump(
        &mut m,
        [0.55, -0.1, 0.35],
        [0.5, 0.35, 0.45],
        7,
        3,
        0.12,
        rng,
        &mut paint,
    );
    // The cut: a lighter, flatter face on the bank's side.
    let cut = hex(CLAY_CUT);
    let x = -0.95;
    face(
        &mut m,
        [
            [x, 0.0, -0.32],
            [x, 0.0, 0.32],
            [x + 0.18, 0.3, 0.26],
            [x + 0.18, 0.3, -0.26],
        ],
        [0.0, 0.0, 0.0],
        cut,
    );
    pebbles(&mut m, rng, 3, 1.25, CLAY[0]);
    m
}

/// A clay bank dug out: its lip round a dark wet hollow, spade marks in it.
fn dug(rng: &mut Rng) -> MeshBuf {
    let mut m = MeshBuf::default();
    let [dark, mid, _] = CLAY.map(hex);
    let mut paint = |n: [f32; 3], _p: [f32; 3]| mix(dark, mid, n[1] * 0.5 + 0.5);
    patch(
        &mut m,
        [0.0, 0.0],
        [0.8, 0.6],
        0.0,
        0.01,
        12,
        lit_glow(hex(WET), 1.0, 0.05),
    );
    let first = rng.range(0.0, TAU);
    for k in 0..5 {
        let a = first + k as f32 / 5.0 * TAU * 0.8;
        let c = [1.0 * a.cos(), -0.08, 0.75 * a.sin()];
        lump(&mut m, c, [0.42, 0.24, 0.32], 6, 3, 0.15, rng, &mut paint);
    }
    for k in 0..3 {
        let x = -0.3 + k as f32 * 0.3;
        patch(
            &mut m,
            [x, rng.range(-0.15, 0.15)],
            [0.12, 0.04],
            0.3,
            0.016,
            4,
            hex(CLAY_CUT),
        );
    }
    m
}

// ── The fallback ─────────────────────────────────────────────────────────

/// Stones piled up: a marker for a resource the client has no model for.
fn cairn(rng: &mut Rng, stones: usize) -> MeshBuf {
    let mut m = MeshBuf::default();
    let mut paint = stone_paint(rng, STONE, false);
    let mut y = 0.0;
    for k in 0..stones {
        let s = 0.4 - 0.09 * k as f32;
        lump(
            &mut m,
            [0.0, y + s * 0.5, 0.0],
            [s, s * 0.6, s * 0.9],
            6,
            4,
            0.15,
            rng,
            &mut paint,
        );
        y += s * 0.95;
    }
    m
}

// ── Wild grass (P4.4) ────────────────────────────────────────────────────

const BLADE: u32 = 0x4a7c59;
const TIP: u32 = 0x6ba377;
const MOUND: u32 = 0x3d5e42;
const BLADE_DARK: u32 = 0x2d4e36;
const STRAW: u32 = 0x7a7460;
const STUBBLE_MOUND: u32 = 0x2e2c30;
const FALLEN: u32 = 0x4a4a54;

/// A tuft of wild grass, its base at the origin: a dozen blades arching
/// out of a low mound, the middle ones tallest, and three seed stalks over
/// them. About 1.4 m across and 1.3 m to the seed heads: React's 12-unit
/// node, a little generous so it reads from the street camera.
pub fn tuft(lit: bool) -> MeshBuf {
    let mut m = MeshBuf::default();
    mound(&mut m, 0.55, 0.08, hex(MOUND));
    let mut rng = Rng(0x9e37_79b9);
    let mut tips = MeshBuf::default();
    let tip = if lit {
        glow(mix(hex(TIP), hex(GOLD), 0.75), 0.9)
    } else {
        hex(TIP)
    };
    const BLADES: usize = 22;
    for i in 0..BLADES {
        // Golden-angle turns, so the blades fan evenly without a pattern.
        let a = i as f32 * 2.399_963 + rng.range(-0.15, 0.15);
        let inner = i % 3 == 0;
        let r0 = if inner {
            rng.range(0.02, 0.08)
        } else {
            rng.range(0.1, 0.26)
        };
        let height = if inner {
            rng.range(0.8, 1.02)
        } else {
            rng.range(0.48, 0.8)
        };
        let lean = if inner {
            rng.range(0.1, 0.28)
        } else {
            rng.range(0.3, 0.55)
        };
        let colours = [
            hex(BLADE_DARK),
            mix(hex(BLADE_DARK), hex(BLADE), 0.6),
            hex(BLADE),
        ];
        blade(&mut m, &mut tips, a, r0, height, lean, 0.085, colours, tip);
    }
    // Two seed stalks, thin and straight-ish, with a head of seed.
    for (a, lean) in [(0.9f32, 0.14f32), (3.6, 0.2), (5.3, 0.1)] {
        let seed = mix(hex(STRAW), hex(TIP), 0.35);
        let colours = [hex(BLADE_DARK), hex(BLADE), hex(BLADE)];
        blade(&mut m, &mut tips, a, 0.06, 1.15, lean, 0.022, colours, seed);
        let [dx, dz] = [a.cos(), a.sin()];
        let top = [0.06 * dx + lean * dx, 1.15, 0.06 * dz + lean * dz];
        head(&mut tips, top, [dx, dz], if lit { tip } else { seed });
    }
    m.append(tips);
    m
}

/// Stubble: short cut stalks, flat across the top, on a darker mound, and
/// a few blades fallen across it.
pub fn stubble() -> MeshBuf {
    let mut m = MeshBuf::default();
    mound(&mut m, 0.5, 0.05, hex(STUBBLE_MOUND));
    let mut rng = Rng(0x2545_f491);
    let mut tops = MeshBuf::default();
    for i in 0..11 {
        let a = i as f32 / 11.0 * TAU + rng.range(-0.25, 0.25);
        let r0 = rng.range(0.04, 0.3);
        let h = rng.range(0.1, 0.2);
        let straw = mix(hex(STRAW), hex(FALLEN), rng.next() * 0.6);
        let colours = [mix(straw, hex(STUBBLE_MOUND), 0.4), straw, straw];
        // Cut: a flat end, so the tip is as wide as the base.
        stalk(&mut m, a, r0, h, 0.03, colours);
        let [dx, dz] = [a.cos(), a.sin()];
        let c = [r0 * dx, h + 0.04, r0 * dz];
        tops.quad(
            [
                [c[0] - 0.02, c[1], c[2] - 0.02],
                [c[0] + 0.02, c[1], c[2] - 0.02],
                [c[0] + 0.02, c[1], c[2] + 0.02],
                [c[0] - 0.02, c[1], c[2] + 0.02],
            ],
            UP,
            mix(straw, hex(0xd8cfa8), 0.35),
        );
    }
    m.append(tops);
    // Fallen blades, lying almost flat over the mound.
    for a in [0.4f32, 2.3, 4.4] {
        let [dx, dz] = [a.cos(), a.sin()];
        let [sx, sz] = [-dz * 0.04, dx * 0.04];
        let (r0, r1, y) = (0.08, 0.55, 0.075);
        m.quad(
            [
                [dx * r0 - sx, y, dz * r0 - sz],
                [dx * r0 + sx, y, dz * r0 + sz],
                [dx * r1, y + 0.02, dz * r1],
                [dx * r1, y + 0.02, dz * r1],
            ],
            UP,
            hex(FALLEN),
        );
    }
    m
}

/// A low dome of earth, `r` across and `h` high, its edge just under the
/// ground so it sits in it rather than on it.
fn mound(m: &mut MeshBuf, r: f32, h: f32, colour: [f32; 4]) {
    const SIDES: usize = 12;
    let rim = |i: usize, r: f32, y: f32| {
        let a = i as f32 / SIDES as f32 * TAU;
        [r * a.cos(), y, r * a.sin()]
    };
    for i in 0..SIDES {
        let mid = (i as f32 + 0.5) / SIDES as f32 * TAU;
        let n = normalise([mid.cos() * h / r * 2.0, 1.0, mid.sin() * h / r * 2.0]);
        m.quad(
            [
                rim(i, r, -0.02),
                rim(i + 1, r, -0.02),
                rim(i + 1, r * 0.55, h * 0.8),
                rim(i, r * 0.55, h * 0.8),
            ],
            n,
            colour,
        );
    }
    let top: Vec<[f32; 3]> = (0..=SIDES).map(|i| rim(i, r * 0.55, h * 0.8)).collect();
    m.fan([0.0, h, 0.0], &top, UP, mix(colour, hex(0x000000), 0.1));
}

/// One blade of grass: a ribbon from the ground at radius `r0` out along
/// angle `a`, rising `height` and arching `lean` metres outward, narrowing
/// from `width` to a point. Seen from both sides. The last segment goes
/// in `tips`, in `tip`'s colour, so it can be lit.
#[allow(clippy::too_many_arguments)]
fn blade(
    m: &mut MeshBuf,
    tips: &mut MeshBuf,
    a: f32,
    r0: f32,
    height: f32,
    lean: f32,
    width: f32,
    colours: [[f32; 4]; 3],
    tip: [f32; 4],
) {
    let [dx, dz] = [a.cos(), a.sin()];
    let side = [-dz, 0.0, dx];
    let at = |t: f32| {
        let out = r0 + lean * t * t;
        [dx * out, height * t, dz * out]
    };
    const SEGS: usize = 4;
    for k in 0..SEGS {
        let (t0, t1) = (k as f32 / SEGS as f32, (k + 1) as f32 / SEGS as f32);
        let (p0, p1) = (at(t0), at(t1));
        let (w0, w1) = (width * (1.0 - t0 * 0.85), width * (1.0 - t1 * 0.85));
        let w1 = if k + 1 == SEGS { 0.0 } else { w1 };
        let off =
            |p: [f32; 3], w: f32, s: f32| [p[0] + side[0] * w * s, p[1], p[2] + side[2] * w * s];
        let corners = [
            off(p0, w0, -1.0),
            off(p0, w0, 1.0),
            off(p1, w1, 1.0),
            off(p1, w1, -1.0),
        ];
        let up = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let n = normalise(cross(side, up));
        let (target, colour) = if k + 1 == SEGS {
            (&mut *tips, tip)
        } else {
            (&mut *m, colours[k.min(2)])
        };
        if w1 == 0.0 {
            let tri = [corners[0], corners[1], corners[2]];
            target.fan(tri[0], &[tri[1], tri[2]], n, colour);
            target.fan(tri[0], &[tri[1], tri[2]], neg(n), colour);
        } else {
            target.quad(corners, n, colour);
            target.quad(corners, neg(n), colour);
        }
    }
}

/// A short straight stalk with a flat cut top, seen from both sides.
fn stalk(m: &mut MeshBuf, a: f32, r0: f32, h: f32, width: f32, colours: [[f32; 4]; 3]) {
    let [dx, dz] = [a.cos(), a.sin()];
    let side = [-dz, 0.0, dx];
    let tilt = 0.03;
    let p0 = [dx * r0, 0.0, dz * r0];
    let p1 = [dx * (r0 + tilt), h + 0.04, dz * (r0 + tilt)];
    let off = |p: [f32; 3], s: f32| [p[0] + side[0] * width * s, p[1], p[2] + side[2] * width * s];
    let corners = [off(p0, -1.0), off(p0, 1.0), off(p1, 1.0), off(p1, -1.0)];
    let n = normalise(cross(side, [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]));
    m.quad(corners, n, colours[1]);
    m.quad(corners, neg(n), colours[0]);
}

/// A head of seed: a slim spindle at the top of a stalk, leaning its way.
fn head(m: &mut MeshBuf, top: [f32; 3], [dx, dz]: [f32; 2], colour: [f32; 4]) {
    let len = 0.16;
    let base = [top[0] - dx * 0.01, top[1] - 0.02, top[2] - dz * 0.01];
    let tip = [top[0] + dx * 0.05, top[1] + len, top[2] + dz * 0.05];
    let mid = [
        (base[0] + tip[0]) * 0.5,
        (base[1] + tip[1]) * 0.5,
        (base[2] + tip[2]) * 0.5,
    ];
    const SIDES: usize = 5;
    let r = 0.025;
    for i in 0..SIDES {
        let a0 = i as f32 / SIDES as f32 * TAU;
        let a1 = (i + 1) as f32 / SIDES as f32 * TAU;
        let ring = |a: f32| [mid[0] + r * a.cos(), mid[1], mid[2] + r * a.sin()];
        let mida = (a0 + a1) * 0.5;
        let out = [mida.cos(), 0.0, mida.sin()];
        for end in [base, tip] {
            let n = normalise([out[0], if end == tip { 0.4 } else { -0.4 }, out[2]]);
            m.fan(end, &[ring(a0), ring(a1)], n, colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(m: &MeshBuf) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in &m.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v.position[k]);
                hi[k] = hi[k].max(v.position[k]);
            }
        }
        (lo, hi)
    }

    fn every_model() -> impl Iterator<Item = (Kind, u8, Look, bool, MeshBuf)> {
        Kind::ALL.into_iter().flat_map(|kind| {
            (0..kind.variants()).flat_map(move |v| {
                [
                    (Look::Full, false),
                    (Look::Full, true),
                    (Look::Spent, false),
                ]
                .into_iter()
                .map(move |(look, lit)| (kind, v, look, lit, model(kind, v, look, lit)))
            })
        })
    }

    #[test]
    fn a_tuft_is_a_nodes_size_and_stubble_is_low() {
        for lit in [false, true] {
            let (lo, hi) = bounds(&tuft(lit));
            assert!(
                lo[1] > -0.05 && lo[1] < 0.0,
                "sits in the ground: {}",
                lo[1]
            );
            assert!((1.0..1.45).contains(&hi[1]), "{} m tall", hi[1]);
            let across = (hi[0] - lo[0]).max(hi[2] - lo[2]);
            assert!((1.0..1.7).contains(&across), "{across} m across");
        }
        let (_, hi) = bounds(&stubble());
        assert!(hi[1] < 0.3, "stubble {} m tall", hi[1]);
    }

    #[test]
    fn every_kind_has_a_model_and_a_spent_look_that_is_lower() {
        for kind in Kind::ALL {
            for v in 0..kind.variants() {
                let full = model(kind, v, Look::Full, false);
                let spent = model(kind, v, Look::Spent, false);
                assert!(!full.indices.is_empty(), "{kind:?} {v}");
                assert!(!spent.indices.is_empty(), "{kind:?} {v} spent");
                let (_, hf) = bounds(&full);
                let (_, hs) = bounds(&spent);
                assert!(
                    hs[1] < hf[1],
                    "{kind:?} {v}: spent {} not under {}",
                    hs[1],
                    hf[1]
                );
            }
        }
    }

    #[test]
    fn models_stand_on_the_ground_at_their_size() {
        // (lowest top, highest top, widest), metres.
        let size = |kind| match kind {
            Kind::Grass => (1.0, 1.45, 1.7),
            Kind::Tree => (5.0, 8.5, 6.5),
            Kind::Branches => (0.05, 0.4, 2.4),
            Kind::Outcrop | Kind::Vein | Kind::Copper | Kind::Tin => (0.6, 1.3, 2.6),
            Kind::Clay => (0.15, 0.6, 3.0),
            Kind::Reeds => (1.6, 2.4, 2.4),
            Kind::Other => (0.6, 1.2, 1.2),
        };
        for kind in Kind::ALL {
            let (lo_top, hi_top, widest) = size(kind);
            for v in 0..kind.variants() {
                let (lo, hi) = bounds(&model(kind, v, Look::Full, false));
                assert!(lo[1] > -1.0, "{kind:?} {v} reaches {} under", lo[1]);
                assert!(
                    (lo_top..hi_top).contains(&hi[1]),
                    "{kind:?} {v} is {} m tall",
                    hi[1]
                );
                let across = (hi[0] - lo[0]).max(hi[2] - lo[2]);
                assert!(across < widest, "{kind:?} {v} is {across} m across");
            }
        }
    }

    #[test]
    fn only_lit_models_glow_except_the_veins_streaks() {
        let glows = |m: &MeshBuf| m.vertices.iter().any(crate::world::mesh::is_light);
        for (kind, v, look, lit, m) in every_model() {
            match (kind, look, lit) {
                (_, Look::Full, true) => assert!(glows(&m), "{kind:?} {v} lit"),
                // Rust that catches the light, and a wet bank's sheen.
                (Kind::Vein | Kind::Copper | Kind::Tin | Kind::Clay, ..) => {}
                _ => assert!(!glows(&m), "{kind:?} {v} {look:?}"),
            }
        }
    }

    #[test]
    fn every_face_faces_its_normal() {
        // So both sides of a blade, each wound its own way, are drawn, and
        // no face of a rock or a crown is culled from outside.
        for (kind, v, look, _, m) in every_model() {
            // Stubble's fallen blades end in a point: two corners at
            // once, a sliver with no winding to check (as it was in P4.4).
            if (kind, look) == (Kind::Grass, Look::Spent) {
                continue;
            }
            for t in m.indices.chunks_exact(3) {
                let p = [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]);
                assert!(
                    crate::world::mesh::facing(
                        p[0].position,
                        p[1].position,
                        p[2].position,
                        p[0].normal
                    ),
                    "{kind:?} {v} {look:?}"
                );
            }
        }
    }

    #[test]
    fn the_same_variant_builds_the_same_model() {
        for kind in Kind::ALL {
            let a = model(kind, 0, Look::Full, false);
            let b = model(kind, 0, Look::Full, false);
            assert_eq!(a.vertices.len(), b.vertices.len());
            assert!(a
                .vertices
                .iter()
                .zip(&b.vertices)
                .all(|(x, y)| x.position == y.position));
        }
    }

    #[test]
    fn unknown_resources_get_the_cairn_and_trees_lean_on_their_zone() {
        assert_eq!(Kind::of("moonstone"), Kind::Other);
        assert_eq!(Kind::of("tree"), Kind::Tree);
        assert_eq!(Kind::of("old_growth"), Kind::Tree);
        assert_eq!(Kind::of("reed_bed"), Kind::Reeds);
        let species = |zone| {
            let mut n = [0; 4];
            for h in 0..4000u64 {
                let v = Kind::Tree.variant_for(h.wrapping_mul(0x9e37_79b9_7f4a_7c15), zone);
                assert!(v < Kind::Tree.variants());
                n[v as usize / 2] += 1;
            }
            n
        };
        let rural = species(Zone::Rural);
        let wild = species(Zone::Wilderness);
        assert!(
            rural[0] > rural[3],
            "oak over pine in the farmland: {rural:?}"
        );
        assert!(wild[3] > wild[0], "pine over oak in the wilds: {wild:?}");
        assert_eq!(species(Zone::Suburban)[3], 0, "no pines in town");
    }
}
