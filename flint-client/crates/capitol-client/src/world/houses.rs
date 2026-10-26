//! Houses on claimed plots (P8.6b): the city fills in as players build.
//!
//! - **One building per claimed lot, on the rules' footprint** (V3.4,
//!   #476): on a home lot its blocks fill `rules::building::footprint`
//!   exactly, so a station indoors stands inside the walls drawn round it
//!   and one in the yard outside them (#462). A home lot too small for a
//!   footprint is all yard, with no house.
//! - **By zone** ([`plan_on`]): the city's is a workshop house over the
//!   whole lot, two or three storeys under slate or a flat roof, with a
//!   shop front on the street (a carriage arch, the shop door and its
//!   window); the suburb's a post-war house beside its lower workshop
//!   wing; a farm's the farmhouse and its barn under one range, standing
//!   back on a drive (the grass keeps off that [`Farmyard`], R6). Guild
//!   halls keep a seeded stone box ([`GUILD`]). Colours are the house
//!   tokens in `palette`.
//! - **What it shows past its plan** ([`Look`]): a storey for each floor
//!   built (V3.3), scaffolding while one goes up and then the storey
//!   rising, and a chimney over each kiln or forge indoors.
//! - **The cutaway** ([`Houses::cutaway`]): a building opens as the camera
//!   comes within [`OPEN_MINE`] of one of yours or [`OPEN_OTHERS`] of
//!   anyone's. The roof settles away and the walls toward the camera come
//!   down to a sill, drawn as an architect's section, so you see and pick
//!   the stations inside. An open one leaves its sector's mesh for
//!   [`OPEN_MESH`].
//! - **Seeded by the plot id** ([`Seed`]), so every client builds the same
//!   house on the same lot.
//! - **Windows** are meshes of their own, one per quarter of each ring, so
//!   they cull at street zoom: the dark glass ([`windows_name`]) and the
//!   windows lit from inside ([`lamps_name`]). A lit window is uploaded
//!   as white light, and the extension sets its entity's colour, lit
//!   surface and light with the hour ([`lamp_glow`]: Flint ADRs 0102's
//!   `vertex_tint_*`, 0096's `glow_surface` and 0090's `emissive_scale`;
//!   H23, #130, #269): glass by day, lamp-coloured at night. No window is
//!   rebuilt when the light changes.
//! - **Growth.** A claim seen as it happens raises its house over
//!   [`GROW_SECS`]: a timber frame, then the walls, then the roof. While it
//!   grows it's in [`GROWING_MESH`] and its lit windows in
//!   [`GROWING_LAMPS`], rebuilt each frame. Once up it joins its sector's
//!   mesh. A claim older than that when first seen (the list at sign-in)
//!   is simply there.
//! - **One mesh per sector** (`sector`, as the lots), rebuilt when a claim
//!   in it changes, so a city of houses costs at most 64 draws, plus 32
//!   for the windows and two for the houses going up.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::coords::{layer, to_server, to_world, S};
use crate::daylight::PlotGlow;
use crate::world::geom::*;
use crate::world::mesh::{MeshBuf, RAY_TARGET, UP};
use crate::world::palette::{self, hex};
use crate::world::plots::Lot;
use crate::world::sector::Sector;
use capitol_protocol::rules::{building, Zone};
use capitol_protocol::terrain::quad_contains;
use flint_import::ImportedMaterial;
use flint_player::ExtensionGpu;

/// How long a house takes to go up, seconds.
pub const GROW_SECS: f32 = 4.0;
/// The mesh with the houses going up.
pub const GROWING_MESH: &str = "houses_growing";
/// The lit windows of the houses going up.
pub const GROWING_LAMPS: &str = "houses_growing_lamps";

/// The runtime mesh for `sector`'s houses.
pub fn mesh_name(sector: Sector) -> String {
    format!("houses_{}", sector.tag())
}

/// The windows are drawn by quarters: four of a ring's wedges.
const QUARTERS: u8 = 4;

/// Which windows mesh a sector's houses' windows are in: (ring, quarter).
fn quarter(s: Sector) -> (u8, u8) {
    (s.ring, s.wedge / (crate::world::sector::WEDGES / QUARTERS))
}

/// The runtime mesh for a quarter's dark windows.
pub fn windows_name((ring, q): (u8, u8)) -> String {
    format!("houses_windows_r{ring}q{q}")
}

/// The runtime mesh for a quarter's lit windows.
pub fn lamps_name((ring, q): (u8, u8)) -> String {
    format!("houses_lamps_r{ring}q{q}")
}

/// Every mesh of lit windows: their entities take [`lamp_glow`].
pub fn lamp_meshes() -> impl Iterator<Item = String> {
    let quarters: BTreeSet<(u8, u8)> = Sector::all().map(quarter).collect();
    quarters
        .into_iter()
        .map(lamps_name)
        .chain([GROWING_LAMPS.to_owned(), OPEN_LAMPS.to_owned()])
}

// ---------------------------------------------------------------------------
// The seed
// ---------------------------------------------------------------------------

/// Every choice about a house, drawn from its plot id: SplitMix64 seeded
/// with the id's FNV-1a hash.
pub struct Seed(u64);

impl Seed {
    pub fn of(id: &str) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in id.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        Seed(h)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// In [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, (lo, hi): (f32, f32)) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Roof {
    /// Ridge along the street.
    Gable,
    /// Ridge running back from the street, the gable end facing it.
    FrontGable,
    Hip,
    /// Flat, behind a cornice.
    Flat,
}

/// How a guild hall builds: the lots with no footprint of the rules' own
/// (V3.4) get a seeded box set back from the street. Lengths are metres.
struct Style {
    /// From the street edge to the front wall.
    setback: (f32, f32),
    depth: (f32, f32),
    /// The widest the house is; a narrower lot gets a narrower house, down
    /// to `min_width`.
    width: (f32, f32),
    min_width: f32,
    min_depth: f32,
    /// Kept clear at the sides and the back, past the eaves.
    side: f32,
    back: f32,
    storeys: &'static [u8],
    storey: f32,
    /// Roof pitch, degrees.
    pitch: (f32, f32),
    roofs: &'static [Roof],
    /// How far the roof overhangs the walls.
    eave: f32,
    walls: &'static [u32],
    roof_colours: &'static [u32],
    /// Window width, and the spacing between window centres.
    window: f32,
    bay: f32,
    /// The chance of a chimney, on a pitched roof.
    chimney: f32,
}

const GUILD: Style = Style {
    setback: (0.8, 1.4),
    depth: (6.0, 8.0),
    width: (7.0, 9.5),
    min_width: 4.0,
    min_depth: 4.0,
    side: 0.3,
    back: 1.0,
    storeys: &[2, 3, 3],
    storey: 3.0,
    pitch: (36.0, 46.0),
    roofs: &[Roof::Gable, Roof::FrontGable, Roof::Flat],
    eave: 0.3,
    walls: &palette::GUILD_WALLS,
    roof_colours: &palette::URBAN_ROOFS,
    window: 0.8,
    bay: 1.6,
    chimney: 0.3,
};

// ---------------------------------------------------------------------------
// The plan: every choice, before any geometry
// ---------------------------------------------------------------------------

/// A rectangle on the ground, server units: centre, the unit axes along
/// and across it, and its half-sizes along each.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    c: P,
    ax: P,
    ay: P,
    hx: f64,
    hy: f64,
}

impl Rect {
    fn at(&self, sx: f64, sy: f64) -> P {
        add(self.c, add(scale(self.ax, sx), scale(self.ay, sy)))
    }

    /// Anticlockwise or not, round the edge: (-,-) (+,-) (+,+) (-,+).
    fn corners(&self) -> [P; 4] {
        [
            self.at(-self.hx, -self.hy),
            self.at(self.hx, -self.hy),
            self.at(self.hx, self.hy),
            self.at(-self.hx, self.hy),
        ]
    }

    /// The same rectangle turned a quarter, so its x axis is this one's y.
    fn turned(&self) -> Rect {
        Rect {
            c: self.c,
            ax: self.ay,
            ay: scale(self.ax, -1.0),
            hx: self.hy,
            hy: self.hx,
        }
    }

    fn grown(&self, d: f64) -> Rect {
        Rect {
            hx: self.hx + d,
            hy: self.hy + d,
            ..*self
        }
    }

    /// `p` in its own axes: (along, across) from the centre.
    fn local(&self, p: P) -> (f64, f64) {
        let d = sub(p, self.c);
        (dot(d, self.ax), dot(d, self.ay))
    }

    fn contains(&self, p: P) -> bool {
        let (s, t) = self.local(p);
        s.abs() <= self.hx && t.abs() <= self.hy
    }

    /// How far `p` is from it, 0 inside.
    fn distance(&self, p: P) -> f64 {
        let (s, t) = self.local(p);
        let (ds, dt) = ((s.abs() - self.hx).max(0.0), (t.abs() - self.hy).max(0.0));
        ds.hypot(dt)
    }

    /// Cut in two along its x axis: `share` of it on the `left` (−x) side
    /// or the right, and the rest.
    fn split(&self, share: f64, left: bool) -> (Rect, Rect) {
        let (a, b) = (self.hx * share, self.hx * (1.0 - share));
        let sign = if left { -1.0 } else { 1.0 };
        (
            Rect {
                c: self.at(sign * (self.hx - a), 0.0),
                hx: a,
                ..*self
            },
            Rect {
                c: self.at(-sign * (self.hx - b), 0.0),
                hx: b,
                ..*self
            },
        )
    }
}

/// One building: the house, a garage or a barn.
#[derive(Clone, Debug, PartialEq)]
struct Block {
    /// The footprint. `ay` points away from the street, so the front wall
    /// is the `-hy` side.
    rect: Rect,
    storeys: u8,
    storey: f32,
    roof: Roof,
    pitch: f32,
    eave: f32,
    wall: [f32; 4],
    roof_colour: [f32; 4],
    /// A chimney at this share of the way along the ridge, if any.
    chimney: Option<f64>,
    door: Door,
    window: f32,
    bay: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Door {
    /// A front door at this share across the front, in this colour.
    Front(f32, [f32; 4]),
    /// A garage's, a workshop's or a barn's wide door.
    Wide([f32; 4]),
    /// A city workshop's front (V3.4): a carriage arch at one end, on the
    /// left or not, the shop door at the other, and its window between.
    Shop { arch_left: bool, colour: [f32; 4] },
}

/// Everything a house will be.
#[derive(Clone, Debug, PartialEq)]
struct Plan {
    blocks: Vec<Block>,
    /// The rules' footprint the blocks fill, on a lot of a home zone (V3.4):
    /// what opens as you come close, and what holds the fires.
    building: Option<Rect>,
    /// A farm's drive, from the lane to the front door (R6).
    drive: Option<[P; 2]>,
    /// Its packed earth, drawn on the farm from inside the hedge bank.
    track: Option<Rect>,
    /// Which of the windows (by a running count) are lit: a seed of its own,
    /// so the count of windows doesn't change the house.
    lights: u64,
}

/// The lot's own frame: `o` on its street edge, `u` along it, `v` in
/// toward the lot, and the outline in (along, in) server units.
struct Frame {
    o: P,
    u: P,
    v: P,
    local: Vec<(f64, f64)>,
}

impl Frame {
    fn of(lot: &Lot) -> Option<Frame> {
        let [a, b] = lot.street;
        if dist(a, b) < 1.0 || lot.outline.len() < 3 {
            return None;
        }
        let u = unit(sub(b, a));
        let mid = scale(add(a, b), 0.5);
        let n = lot.outline.len() as f64;
        let c = scale(
            lot.outline.iter().fold([0.0, 0.0], |s, &p| add(s, p)),
            1.0 / n,
        );
        let mut v = left(u);
        if dot(sub(c, mid), v) < 0.0 {
            v = scale(v, -1.0);
        }
        let local = lot
            .outline
            .iter()
            .map(|&p| (dot(sub(p, mid), u), dot(sub(p, mid), v)))
            .collect();
        Some(Frame {
            o: mid,
            u,
            v,
            local,
        })
    }

    fn at(&self, s: f64, t: f64) -> P {
        add(self.o, add(scale(self.u, s), scale(self.v, t)))
    }

    /// The outline's depth: its nearest and farthest `t`.
    fn depth(&self) -> (f64, f64) {
        self.local
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &(_, t)| {
                (lo.min(t), hi.max(t))
            })
    }

    /// Where the outline crosses the line at depth `t`: its (left, right),
    /// or `None` if it doesn't reach that deep.
    fn chord(&self, t: f64) -> Option<(f64, f64)> {
        let n = self.local.len();
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..n {
            let (a, b) = (self.local[i], self.local[(i + 1) % n]);
            if (a.1 - t) * (b.1 - t) > 0.0 || a.1 == b.1 {
                continue;
            }
            let s = a.0 + (b.0 - a.0) * (t - a.1) / (b.1 - a.1);
            lo = lo.min(s);
            hi = hi.max(s);
        }
        (hi > lo).then_some((lo, hi))
    }

    /// How wide a box from depth `t0` to `t1` can be: the overlap of the
    /// chords at both ends, which a convex outline holds between them.
    fn span(&self, t0: f64, t1: f64) -> Option<(f64, f64)> {
        let (a0, b0) = self.chord(t0)?;
        let (a1, b1) = self.chord(t1)?;
        let (lo, hi) = (a0.max(a1), b0.min(b1));
        (hi > lo).then_some((lo, hi))
    }

    /// A box `width` by `depth` metres with its front at depth `t0`
    /// (server), placed at `along` (0 left, 1 right) in the room there is,
    /// keeping `margin` metres clear at the sides, and with room for a roof
    /// `overhang` metres past the walls in front and behind. `None` if it
    /// won't fit.
    fn place(
        &self,
        t0: f64,
        (width, depth): (f32, f32),
        margin: f32,
        overhang: f32,
        along: f32,
    ) -> Option<Rect> {
        let (w, d, m, ov) = (
            metres(width),
            metres(depth),
            metres(margin),
            metres(overhang),
        );
        let (lo, hi) = self.span(t0 - ov, t0 + d + ov)?;
        let room = hi - lo - 2.0 * m;
        if room < w {
            return None;
        }
        let left = lo + m + (room - w) * along as f64;
        let c = self.at(left + w * 0.5, t0 + d * 0.5);
        Some(Rect {
            c,
            ax: self.u,
            ay: self.v,
            hx: w * 0.5,
            hy: d * 0.5,
        })
    }
}

/// Metres as server units.
fn metres(m: f32) -> f64 {
    m as f64 / S as f64
}

/// The building's footprint on `lot` as the rules set it
/// (`rules::building::footprint`, V3.1): `ay` back from the street. `None`
/// off the home zones, or on a lot too small for one.
fn footprint(lot: &Lot) -> Option<Rect> {
    let f = building::footprint(&lot.corners.map(|[x, y]| (x, y)), lot.zone)?.map(|(x, y)| [x, y]);
    let (along, back) = (sub(f[1], f[0]), sub(f[3], f[0]));
    Some(Rect {
        c: scale(add(f[0], f[2]), 0.5),
        ax: unit(along),
        ay: unit(back),
        hx: dist(f[0], f[1]) * 0.5,
        hy: dist(f[0], f[3]) * 0.5,
    })
}

/// The plan for the house on `lot`, from its id, or `None` if the lot
/// has no room for one. On a home lot it's the rules' building (V3.4); a
/// home lot too small for one is all yard.
fn plan(id: &str, lot: &Lot) -> Option<Plan> {
    if building::shape(lot.zone).is_some() {
        return footprint(lot).map(|fp| plan_on(id, lot, fp));
    }
    plan_seeded(id, lot)
}

/// The building on a home lot (V3.4): it fills the rules' footprint, every
/// block of it, so a station indoors stands inside the walls drawn round it
/// (#462). The seed picks how it looks, never where it stands.
fn plan_on(id: &str, lot: &Lot, fp: Rect) -> Plan {
    let mut rng = Seed::of(id);
    let shade = |rng: &mut Seed, colours: &[u32], (lo, hi): (f32, f32)| {
        palette::shade(hex(rng.pick(colours)), rng.range((lo, hi)))
    };
    let mut blocks = Vec::with_capacity(2);
    match lot.zone {
        // A house and its workshop wing side by side, a garden behind.
        Zone::Suburban => {
            let share = rng.range((0.52, 0.64)) as f64;
            let (house, wing) = fp.split(share, rng.chance(0.5));
            let wall = shade(&mut rng, &palette::SUBURBAN_WALLS, (0.93, 1.05));
            let roof_colour = shade(&mut rng, &palette::SUBURBAN_ROOFS, (0.9, 1.08));
            blocks.push(Block {
                rect: house,
                storeys: rng.pick(&[1, 1, 2]),
                storey: 2.6,
                roof: rng.pick(&[Roof::Gable, Roof::Gable, Roof::Hip, Roof::FrontGable]),
                pitch: rng.range((24.0, 34.0)),
                eave: 0.25,
                wall,
                roof_colour,
                chimney: None,
                door: Door::Front(rng.range((0.3, 0.7)), hex(rng.pick(&palette::DOORS))),
                window: 0.7,
                bay: 1.35,
            });
            blocks.push(Block {
                rect: wing,
                storeys: 1,
                storey: 2.8,
                roof: Roof::FrontGable,
                pitch: rng.range((18.0, 24.0)),
                eave: 0.15,
                wall: palette::shade(wall, 0.94),
                roof_colour,
                chimney: None,
                door: Door::Wide(hex(palette::GARAGE_DOOR)),
                window: 0.7,
                bay: 99.0,
            });
        }
        // The farmhouse and its barn under one long range.
        Zone::Rural => {
            let share = rng.range((0.38, 0.46)) as f64;
            let (house, barn) = fp.split(share, rng.chance(0.5));
            blocks.push(Block {
                rect: house,
                storeys: rng.pick(&[1, 2, 2]),
                storey: 2.5,
                roof: Roof::Gable,
                pitch: rng.range((40.0, 48.0)),
                eave: 0.3,
                wall: shade(&mut rng, &palette::RURAL_WALLS, (0.93, 1.05)),
                roof_colour: shade(&mut rng, &palette::RURAL_ROOFS, (0.9, 1.08)),
                chimney: None,
                door: Door::Front(rng.range((0.35, 0.65)), hex(rng.pick(&palette::DOORS))),
                window: 0.6,
                bay: 1.5,
            });
            blocks.push(Block {
                rect: barn,
                storeys: 1,
                storey: rng.range((3.4, 3.8)),
                roof: Roof::FrontGable,
                pitch: rng.range((34.0, 42.0)),
                eave: 0.25,
                wall: shade(&mut rng, &palette::BARNS, (0.92, 1.05)),
                roof_colour: hex(palette::BARN_ROOF),
                chimney: None,
                door: Door::Wide(hex(palette::BARN_DOOR)),
                window: 0.5,
                bay: 99.0,
            });
        }
        // A workshop house across the whole lot: the shop on the street,
        // a carriage arch through to the strip of yard behind.
        _ => blocks.push(Block {
            rect: fp,
            storeys: rng.pick(&[2, 2, 3]),
            storey: 2.7,
            roof: rng.pick(&[Roof::Gable, Roof::Gable, Roof::Flat]),
            pitch: rng.range((30.0, 38.0)),
            eave: 0.0,
            wall: shade(&mut rng, &palette::URBAN_WALLS, (0.93, 1.05)),
            roof_colour: shade(&mut rng, &palette::URBAN_ROOFS, (0.9, 1.08)),
            chimney: None,
            door: Door::Shop {
                arch_left: rng.chance(0.5),
                colour: hex(rng.pick(&palette::DOORS)),
            },
            window: 0.55,
            bay: 1.1,
        }),
    }
    // Where a pie narrows past the building, the roof stops at its walls
    // rather than overhang the lot next door.
    let lot_quad = lot.corners.map(|[x, y]| (x, y));
    for b in &mut blocks {
        let over = b.rect.grown(metres(b.eave)).corners();
        if !over.iter().all(|p| quad_contains(&lot_quad, p[0], p[1])) {
            b.eave = 0.0;
        }
    }
    // The drive runs straight in from the lane to the farmhouse door.
    let drive = (lot.zone == Zone::Rural)
        .then(|| {
            let frame = Frame::of(lot)?;
            let h = &blocks[0];
            let Door::Front(k, _) = h.door else {
                return None;
            };
            let door = h.rect.at(h.rect.hx * (2.0 * k as f64 - 1.0), -h.rect.hy);
            let s = dot(sub(door, frame.o), frame.u);
            let (near, _) = frame.depth();
            let t1 = dot(sub(door, frame.o), frame.v);
            let t0 = near + TRACK_FROM;
            let track = (t1 > t0).then(|| Rect {
                c: frame.at(s, (t0 + t1) * 0.5),
                ax: frame.u,
                ay: frame.v,
                hx: TRACK_HALF,
                hy: (t1 - t0) * 0.5,
            });
            Some(([frame.at(s, -DRIVE_OUT), door], track))
        })
        .flatten();
    Plan {
        blocks,
        building: Some(fp),
        drive: drive.map(|(d, _)| d),
        track: drive.and_then(|(_, t)| t),
        lights: rng.next(),
    }
}

/// A guild hall's house: a seeded box square to the street, set back and
/// kept off the sides, or `None` if the lot has no room for the smallest.
fn plan_seeded(id: &str, lot: &Lot) -> Option<Plan> {
    let st = &GUILD;
    let frame = Frame::of(lot)?;
    let mut rng = Seed::of(id);
    let (near, far) = frame.depth();
    let roof = rng.pick(st.roofs);
    let setback = rng.range(st.setback);
    let (want_depth, want_width) = (rng.range(st.depth), rng.range(st.width));
    // What stands out past the walls: the eaves, or a flat roof's cornice.
    let flat = roof == Roof::Flat;
    let eaves = if flat { 0.0 } else { st.eave };
    let fit = |eave: f32| {
        let overhang = if flat { CORNICE } else { eave };
        let margin = st.side + overhang;
        // A shallow lot takes a shorter front garden before a smaller house.
        let lot_depth = (far - near) as f32 * S;
        let setback = setback
            .min(lot_depth - st.back - overhang - want_depth.max(st.min_depth))
            .max(overhang + 0.05);
        // On a wedge the street end is narrow, so step back to where it
        // widens.
        (0..8).find_map(|k| {
            let t0 = near + metres(setback + k as f32 * 0.8);
            // As deep as asked, or as the lot allows.
            let room = (far - t0) as f32 * S - st.back - overhang;
            let depth = want_depth.min(room);
            if depth < st.min_depth {
                return None;
            }
            let ov = metres(overhang);
            let (lo, hi) = frame.span(t0 - ov, t0 + metres(depth) + ov)?;
            let free = (hi - lo) as f32 * S - 2.0 * margin;
            // A centimetre spare, or a house that fills its lot misses by
            // rounding.
            let width = want_width.min(free - 0.01);
            (width >= st.min_width).then_some((eave, overhang, t0, depth, width))
        })
    };
    // Too tight with eaves, the roof stops at the walls.
    let (eave, overhang, t0, depth, width) = fit(eaves).or_else(|| fit(0.0))?;
    let along = 0.35 + 0.3 * rng.unit();
    let rect = frame.place(t0, (width, depth), st.side + overhang, overhang, along)?;
    let wall = palette::shade(hex(rng.pick(st.walls)), rng.range((0.93, 1.05)));
    let roof_colour = palette::shade(hex(rng.pick(st.roof_colours)), rng.range((0.9, 1.08)));
    let pitch = rng.range(st.pitch);
    let storeys = rng.pick(st.storeys);
    let chimney = (!flat && rng.chance(st.chimney)).then(|| if rng.chance(0.5) { -0.62 } else { 0.62 });
    let door = Door::Front(rng.range((0.25, 0.75)), hex(rng.pick(&palette::DOORS)));
    Some(Plan {
        blocks: vec![Block {
            rect,
            storeys,
            storey: st.storey,
            roof,
            pitch,
            eave,
            wall,
            roof_colour,
            chimney,
            door,
            window: st.window,
            bay: st.bay,
        }],
        building: None,
        drive: None,
        track: None,
        lights: rng.next(),
    })
}

/// How far out past the lot's street edge a drive runs, server units:
/// across a farm's verge (17 off a lane, 30 off an arterial, less the
/// road's own bare verge) and onto the road, short of a lane's far side.
const DRIVE_OUT: f64 = 26.0;
/// The drawn track: where it starts in from the lot as drawn, past its
/// hedge bank (`plots.rs`' rural curb, 3.5), and its half-width, inside the
/// grass's bare 3 (`grass::DRIVE_HALF`). Server units.
const TRACK_FROM: f64 = 4.0;
const TRACK_HALF: f64 = 2.2;
/// How far the track lies over the lot, metres.
const TRACK_LIFT: f32 = 0.01;

/// A farm's yard (R6, #289), server units: what the grass keeps off.
#[derive(Clone, Debug, PartialEq)]
pub struct Farmyard {
    /// Round the farmhouse and the barn: the hull of their footprints.
    pub yard: Vec<P>,
    /// From the lane to the front door.
    pub drive: [P; 2],
}

/// The yard of the farmstead that stands, or would stand, on `lot`: the
/// same plan the house is raised from, so the grass and the buildings
/// agree. `None` off the farms, or where there's no room for a house.
pub fn farmyard(id: &str, lot: &Lot) -> Option<Farmyard> {
    let plan = plan(id, lot)?;
    let corners: Vec<P> = plan.blocks.iter().flat_map(|b| b.rect.corners()).collect();
    Some(Farmyard {
        yard: crate::world::plots::hull(&corners),
        drive: plan.drive?,
    })
}

/// Whether `rect` is inside the lot with `margin` metres to spare.
#[cfg(test)]
fn inside(frame: &Frame, rect: &Rect, margin: f32) -> bool {
    let m = metres(margin);
    rect.corners().iter().all(|&p| {
        let d = sub(p, frame.o);
        let (s, t) = (dot(d, frame.u), dot(d, frame.v));
        frame
            .chord(t)
            .is_some_and(|(lo, hi)| s >= lo + m && s <= hi - m)
    })
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// One window: a quad just proud of its wall, lit from inside or not.
#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    corners: [[f32; 3]; 4],
    normal: [f32; 3],
    lit: bool,
}

/// A house, built: its walls and roofs, and its windows.
#[derive(Clone, Default)]
pub struct House {
    pub body: MeshBuf,
    pub panes: Vec<Pane>,
    /// Over each fire indoors (`Look::fires`), the fire and the top of its
    /// chimney, world metres: where its smoke comes out.
    pub chimneys: Vec<(P, [f32; 3])>,
}

/// The foot of every wall, sunk into the lot so no seam shows.
const FOOT: f32 = layer::PLOT - 0.04;
/// Where a house's first storey starts.
const FLOOR: f32 = layer::PLOT;
/// How far windows and doors stand off their wall, metres.
const PROUD: f32 = 0.015;
/// The timber of a house going up.
const POST: f32 = 0.14;
/// How far a flat roof's cornice stands out from the walls, metres.
const CORNICE: f32 = 0.08;

fn lift(p: P, y: f32) -> [f32; 3] {
    let [x, z] = to_world(p[0], p[1]);
    [x, y, z]
}

fn level(d: P) -> [f32; 3] {
    let k = 1.0 / d[0].hypot(d[1]);
    [(d[0] * k) as f32, 0.0, (d[1] * k) as f32]
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn minus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// A sloped quad, facing up and out.
fn slope(m: &mut MeshBuf, q: [[f32; 3]; 4], colour: [f32; 4]) {
    let mut n = normalized(cross(minus(q[1], q[0]), minus(q[3], q[0])));
    if n[1] < 0.0 {
        n = n.map(|c| -c);
    }
    m.quad(q, n, colour);
}

/// The four walls of `r` from `y0` to `y1`, each facing out.
fn walls(m: &mut MeshBuf, r: &Rect, y0: f32, y1: f32, colour: [f32; 4]) {
    if y1 <= y0 {
        return;
    }
    let c = r.corners();
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        let mid = scale(add(a, b), 0.5);
        let out = level(sub(mid, r.c));
        m.quad(
            [lift(a, y0), lift(b, y0), lift(b, y1), lift(a, y1)],
            out,
            colour,
        );
    }
}

fn top(m: &mut MeshBuf, r: &Rect, y: f32, colour: [f32; 4]) {
    let c = r.corners();
    m.quad(c.map(|p| lift(p, y)), UP, colour);
}

fn cuboid(m: &mut MeshBuf, r: &Rect, y0: f32, y1: f32, colour: [f32; 4]) {
    walls(m, r, y0, y1, colour);
    top(m, r, y1, colour);
}

/// A gabled roof on `r`, ridge along its x axis, from the wall top `y` up
/// by `rise`, overhanging by `eave` (server) all round. The gable ends are
/// the wall's colour.
fn gable(m: &mut MeshBuf, r: &Rect, y: f32, rise: f32, eave: f64, wall: [f32; 4], roof: [f32; 4]) {
    let slope_k = rise / (r.hy as f32 * S);
    let drop = eave as f32 * S * slope_k;
    let (ex, ey) = (r.hx + eave, r.hy + eave);
    let ridge = y + rise;
    let low = y - drop;
    for side in [-1.0, 1.0] {
        slope(
            m,
            [
                lift(r.at(-ex, side * ey), low),
                lift(r.at(ex, side * ey), low),
                lift(r.at(ex, 0.0), ridge),
                lift(r.at(-ex, 0.0), ridge),
            ],
            roof,
        );
        fascia(m, r.at(-ex, side * ey), r.at(ex, side * ey), low, r.c, roof);
        // The gable end, in the wall's plane.
        let n = level(scale(r.ax, side));
        m.fan(
            lift(r.at(side * r.hx, 0.0), ridge),
            &[
                lift(r.at(side * r.hx, -r.hy), y),
                lift(r.at(side * r.hx, r.hy), y),
            ],
            n,
            wall,
        );
    }
}

/// A hipped roof on `r`, ridge along its longer side.
fn hip(m: &mut MeshBuf, r: &Rect, y: f32, rise: f32, eave: f64, roof: [f32; 4]) {
    let r = if r.hx >= r.hy { *r } else { r.turned() };
    let slope_k = rise / (r.hy as f32 * S);
    let drop = eave as f32 * S * slope_k;
    let (ex, ey) = (r.hx + eave, r.hy + eave);
    let (ridge, low) = (y + rise, y - drop);
    let run = r.hx - r.hy;
    for side in [-1.0, 1.0] {
        slope(
            m,
            [
                lift(r.at(-ex, side * ey), low),
                lift(r.at(ex, side * ey), low),
                lift(r.at(run, 0.0), ridge),
                lift(r.at(-run, 0.0), ridge),
            ],
            roof,
        );
        fascia(m, r.at(-ex, side * ey), r.at(ex, side * ey), low, r.c, roof);
        fascia(m, r.at(side * ex, -ey), r.at(side * ex, ey), low, r.c, roof);
        let tip = lift(r.at(side * run, 0.0), ridge);
        let (a, b) = (
            lift(r.at(side * ex, -ey), low),
            lift(r.at(side * ex, ey), low),
        );
        let mut n = normalized(cross(minus(a, tip), minus(b, tip)));
        if n[1] < 0.0 {
            n = n.map(|c| -c);
        }
        m.fan(tip, &[a, b], n, roof);
    }
}

/// The roof's edge, a hand's depth down from the eave between `a` and `b`,
/// facing away from `centre`.
fn fascia(m: &mut MeshBuf, a: P, b: P, y: f32, centre: P, colour: [f32; 4]) {
    let out = level(sub(scale(add(a, b), 0.5), centre));
    let edge = palette::shade(colour, 0.7);
    m.quad(
        [lift(a, y - 0.12), lift(b, y - 0.12), lift(b, y), lift(a, y)],
        out,
        edge,
    );
}

/// How a building stands at a point in its growth: how much of its frame,
/// its walls and its roof are up, each from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stage {
    frame: f32,
    walls: f32,
    roof: f32,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Stage {
    const BUILT: Stage = Stage {
        frame: 1.0,
        walls: 1.0,
        roof: 1.0,
    };

    /// At `p` of the way through growing (1 is built).
    fn at(p: f32) -> Stage {
        if p >= 1.0 {
            return Stage::BUILT;
        }
        let seg = |a: f32, b: f32| smooth((p - a) / (b - a));
        Stage {
            frame: seg(0.0, 0.3),
            walls: seg(0.25, 0.7),
            roof: seg(0.65, 1.0),
        }
    }
}

/// What a building shows past its plan (V3.4): its floors, a floor going
/// up, the fires inside it, and how far it's open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    /// Floors bought and built (V3.3), each a storey over the seeded ones.
    pub floors: u8,
    /// A floor going up: `Some(0)` while it stands in scaffolding, then
    /// rising to 1 as its storey's walls go up.
    pub raising: Option<f32>,
    /// The kilns and forges indoors, server units: a chimney over each.
    pub fires: Vec<P>,
    /// Open as the camera comes close, and how far.
    pub open: Option<Open>,
}

/// A building opening (V3.4): the roof settles away and the walls facing
/// the camera are cut down to a sill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Open {
    /// How far: 0 shut, 1 open.
    pub p: f32,
    /// Where the camera is, server units: the walls on its side are cut.
    pub eye: P,
    /// How many storeys stand, from the ground: the one aimed at and those
    /// under it. The ones above are gone while it's open.
    pub shown: u8,
}

impl Look {
    /// How many storeys `b` has, with a floor going up counted.
    fn storeys(&self, b: &Block) -> u8 {
        b.storeys
            .saturating_add(self.floors)
            .saturating_add(self.raising.is_some() as u8)
    }
}

/// How thick a wall shows where it's cut, and the sill it's cut to, metres.
const WALL: f32 = 0.22;
const SILL: f32 = 0.55;
/// How far into opening the roof has gone, and when the walls begin to
/// come down.
const ROOF_GONE: f32 = 0.3;
const CUT_FROM: f32 = 0.25;

/// Builds `plan` as it stands at `stage`, looking as `look` says.
fn raise(plan: &Plan, stage: Stage, look: &Look) -> House {
    let mut h = House::default();
    let mut lights = Seed(plan.lights);
    let open = look.open.filter(|o| o.p > 0.0).zip(plan.building);
    for b in &plan.blocks {
        match open {
            Some((o, fp)) => open_block(&mut h, b, look, &fp, o, &mut lights),
            None => {
                let fires: Vec<P> = look.fires.iter().copied().filter(|&p| b.rect.contains(p)).collect();
                block(&mut h, b, stage, look, &fires, &mut lights);
            }
        }
    }
    if let Some(track) = &plan.track {
        top(&mut h.body, track, FLOOR + TRACK_LIFT, hex(palette::FARM_TRACK));
    }
    h
}

fn block(h: &mut House, b: &Block, stage: Stage, look: &Look, fires: &[P], lights: &mut Seed) {
    let m = &mut h.body;
    let r = &b.rect;
    let storeys = look.storeys(b);
    let height = storeys as f32 * b.storey;
    let top_y = FLOOR + height;
    let timber = hex(palette::TIMBER);
    let post = metres(POST);
    let inner = Rect {
        hx: r.hx - post * 0.5,
        hy: r.hy - post * 0.5,
        ..*r
    };
    let posts = |m: &mut MeshBuf, y: f32| {
        for p in inner.corners() {
            let foot = Rect {
                c: p,
                hx: post * 0.5,
                hy: post * 0.5,
                ..*r
            };
            cuboid(m, &foot, FOOT, y, timber);
        }
    };
    // The frame: a post at each corner, and once they're up, the plates.
    if stage != Stage::BUILT && stage.frame > 0.0 {
        posts(m, FLOOR + height * stage.frame);
        if stage.frame >= 1.0 && stage.roof <= 0.0 {
            for (hx, hy) in [(inner.hx, post * 0.5), (post * 0.5, inner.hy)] {
                for side in [-1.0, 1.0] {
                    let along_x = hy < hx;
                    let c = if along_x {
                        inner.at(0.0, side * inner.hy)
                    } else {
                        inner.at(side * inner.hx, 0.0)
                    };
                    let beam = Rect { c, hx, hy, ..*r };
                    cuboid(m, &beam, top_y - 0.12, top_y, timber);
                }
            }
        }
    }
    // The walls, rising. A floor going up stands in its frame and its
    // scaffolding, and then its walls rise under the roof (V3.4).
    let built = match look.raising {
        Some(t) => height - b.storey * (1.0 - smooth(t)),
        None => height,
    };
    let wall_top = FLOOR + built * stage.walls;
    if stage.walls > 0.0 {
        walls(m, r, FOOT, wall_top, b.wall);
    }
    if look.raising.is_some_and(|t| t < 1.0) && stage == Stage::BUILT {
        posts(m, top_y);
        scaffold(m, r, top_y, b.storey, storeys);
    }
    // Doors and windows, once the wall has passed them.
    let c = r.corners();
    for (i, &(a, z)) in [(c[0], c[1]), (c[1], c[2]), (c[2], c[3]), (c[3], c[0])]
        .iter()
        .enumerate()
    {
        openings(h, b, a, z, i == 0, wall_top, storeys, lights);
    }
    roof(h, b, top_y, stage.roof, fires);
}

/// The roof of `b` on walls up to `top_y`, `k` of the way built (or
/// settled away, as the building opens), with a chimney over each of the
/// `fires` once it's whole.
fn roof(h: &mut House, b: &Block, top_y: f32, k: f32, fires: &[P]) {
    let m = &mut h.body;
    let r = &b.rect;
    if k <= 0.0 {
        return;
    }
    let eave = metres(b.eave);
    let brick = hex(palette::CHIMNEY);
    match b.roof {
        Roof::Flat => {
            let cornice = r.grown(metres(CORNICE));
            let stone = palette::shade(b.wall, 0.82);
            cuboid(m, &cornice, top_y - 0.22 * k, top_y + 0.06, stone);
            top(m, r, top_y + 0.065, b.roof_colour);
        }
        Roof::Gable | Roof::FrontGable | Roof::Hip => {
            let along = if b.roof == Roof::FrontGable {
                r.turned()
            } else {
                *r
            };
            let rise = along.hy as f32 * S * b.pitch.to_radians().tan() * k;
            if rise < 0.05 {
                return;
            }
            if b.roof == Roof::Hip {
                hip(m, &along, top_y, rise, eave, b.roof_colour);
            } else {
                gable(m, &along, top_y, rise, eave, b.wall, b.roof_colour);
            }
            if let (Some(at), true) = (b.chimney, k >= 1.0) {
                // Up through the back slope, near one end of the ridge.
                let ridge = if b.roof == Roof::Hip && along.hx < along.hy {
                    along.turned()
                } else {
                    along
                };
                // In scale with the house: a terrace's is slighter.
                let half = metres(0.28).min(ridge.hx.min(ridge.hy) * 0.16);
                let stack = Rect {
                    c: ridge.at(ridge.hx * at, ridge.hy * 0.3),
                    hx: half,
                    hy: half,
                    ..ridge
                };
                cuboid(m, &stack, top_y, top_y + rise + 0.55, brick);
            }
        }
    }
    if k < 1.0 {
        return;
    }
    // A chimney straight up over each fire indoors, through the roof
    // above it, and a little over the ridge if it's near one.
    let half = metres(0.32).min(r.hx.min(r.hy) * 0.2);
    for &p in fires {
        // Kept in from the walls, so it rises through the roof.
        let (s, t) = r.local(p);
        let (s, t) = (
            s.clamp(-r.hx + half * 2.0, r.hx - half * 2.0),
            t.clamp(-r.hy + half * 2.0, r.hy - half * 2.0),
        );
        let at = r.at(s, t);
        let under = roof_height(b, top_y, at);
        let up = under + 0.75;
        let stack = Rect {
            c: at,
            hx: half,
            hy: half,
            ..*r
        };
        cuboid(m, &stack, top_y - 0.1, up, brick);
        // Its cap, a course proud of the stack.
        cuboid(m, &stack.grown(metres(0.05)), up - 0.12, up, palette::shade(brick, 0.8));
        let [x, z] = to_world(at[0], at[1]);
        h.chimneys.push((p, [x, up, z]));
    }
}

/// How high `b`'s roof is over `p`, on walls up to `top_y`.
fn roof_height(b: &Block, top_y: f32, p: P) -> f32 {
    let r = if b.roof == Roof::FrontGable {
        b.rect.turned()
    } else {
        b.rect
    };
    let rise = r.hy as f32 * S * b.pitch.to_radians().tan();
    match b.roof {
        Roof::Flat => top_y + 0.065,
        Roof::Gable | Roof::FrontGable => {
            let (_, t) = r.local(p);
            top_y + rise * (1.0 - (t.abs() / r.hy) as f32).max(0.0)
        }
        Roof::Hip => {
            let r = if r.hx >= r.hy { r } else { r.turned() };
            let rise = r.hy as f32 * S * b.pitch.to_radians().tan();
            let (s, t) = r.local(p);
            let run = r.hx - r.hy;
            let across = 1.0 - t.abs() / r.hy;
            let along = 1.0 - (s.abs() - run).max(0.0) / r.hy;
            top_y + rise * across.min(along).max(0.0) as f32
        }
    }
}

/// Scaffolding round `r` while a floor goes up (V3.4): poles a stride
/// apart, a ledger at every storey and over the top, and boards to stand
/// on at the storey going up.
fn scaffold(m: &mut MeshBuf, r: &Rect, top_y: f32, storey: f32, storeys: u8) {
    let pole = hex(palette::SCAFFOLD);
    let board = hex(palette::TIMBER);
    let g = r.grown(metres(0.6));
    let thin = metres(0.04);
    let stride = metres(2.1);
    let c = g.corners();
    for i in 0..4 {
        let (a, z) = (c[i], c[(i + 1) % 4]);
        let n = (dist(a, z) / stride).ceil().max(1.0) as usize;
        for k in 0..n {
            let p = lerp(a, z, k as f64 / n as f64);
            let foot = Rect {
                c: p,
                hx: thin,
                hy: thin,
                ..*r
            };
            cuboid(m, &foot, FOOT, top_y + 1.0, pole);
        }
    }
    let sides = [
        (g.at(0.0, -g.hy), g.hx, thin),
        (g.at(0.0, g.hy), g.hx, thin),
        (g.at(-g.hx, 0.0), thin, g.hy),
        (g.at(g.hx, 0.0), thin, g.hy),
    ];
    for k in 1..=storeys {
        let y = FLOOR + k as f32 * storey;
        for &(c, hx, hy) in &sides {
            cuboid(m, &Rect { c, hx, hy, ..*r }, y - 0.06, y, pole);
        }
    }
    for &(c, hx, hy) in &sides {
        cuboid(m, &Rect { c, hx, hy, ..*r }, top_y + 0.9, top_y + 0.96, pole);
    }
    // The boards, between the wall and the poles, where the work is.
    let y = top_y - storey;
    let gap = metres(0.3);
    let deck = [
        (r.at(0.0, -r.hy - gap), g.hx, gap * 0.8),
        (r.at(0.0, r.hy + gap), g.hx, gap * 0.8),
        (r.at(-r.hx - gap, 0.0), gap * 0.8, r.hy),
        (r.at(r.hx + gap, 0.0), gap * 0.8, r.hy),
    ];
    for (c, hx, hy) in deck {
        cuboid(m, &Rect { c, hx, hy, ..*r }, y, y + 0.05, board);
    }
}

/// `b` opened (V3.4), as an architect's section: the roof settles away,
/// the walls on the camera's side and the ones between blocks are cut to a
/// sill, the far ones stand as high as the storeys shown, each cut shows
/// its thickness, and the floors are boards, every storey shown over the
/// ground a cut slab.
fn open_block(h: &mut House, b: &Block, look: &Look, fp: &Rect, o: Open, lights: &mut Seed) {
    let r = &b.rect;
    let storeys = b.storeys.saturating_add(look.floors);
    let top_y = FLOOR + storeys as f32 * b.storey;
    let shown = o.shown.clamp(1, storeys.max(1));
    let k = smooth((o.p - CUT_FROM) / (1.0 - CUT_FROM));
    let far_y = top_y + (FLOOR + shown as f32 * b.storey - top_y) * k;
    let sill_y = top_y + (FLOOR + SILL - top_y) * k;
    let w = metres(WALL);
    let inner = Rect {
        hx: r.hx - w,
        hy: r.hy - w,
        ..*r
    };
    let (c, ci) = (r.corners(), inner.corners());
    let (plaster, cut) = (hex(palette::PLASTER), hex(palette::CUT));
    for i in 0..4 {
        let j = (i + 1) % 4;
        let (a, z, ai, zi) = (c[i], c[j], ci[i], ci[j]);
        let mid = scale(add(a, z), 0.5);
        let out = level(sub(mid, r.c));
        // A wall between two blocks of the building is inside it.
        let (s, t) = fp.local(mid);
        let shared = s.abs() < fp.hx - 0.5 && t.abs() < fp.hy - 0.5;
        let near = shared || dot(sub(o.eye, mid), [out[0] as f64, out[2] as f64]) > 0.0;
        let y = if near { sill_y } else { far_y };
        let m = &mut h.body;
        m.quad([lift(a, FOOT), lift(z, FOOT), lift(z, y), lift(a, y)], out, b.wall);
        m.quad(
            [lift(ai, FLOOR), lift(zi, FLOOR), lift(zi, y), lift(ai, y)],
            out.map(|v| -v),
            plaster,
        );
        m.quad([lift(a, y), lift(z, y), lift(zi, y), lift(ai, y)], UP, cut);
        // Its ends, which show where the wall beside it is cut lower.
        for (p, pi, away) in [(a, ai, sub(a, z)), (z, zi, sub(z, a))] {
            let mut n = left(unit(sub(pi, p)));
            if dot(n, away) < 0.0 {
                n = scale(n, -1.0);
            }
            m.quad([lift(p, FLOOR), lift(pi, FLOOR), lift(pi, y), lift(p, y)], level(n), cut);
        }
        if !shared {
            openings(h, b, a, z, i == 0, y, storeys, lights);
        }
    }
    let m = &mut h.body;
    top(m, &inner, FLOOR + 0.005, hex(palette::BOARDS));
    for k in 1..shown {
        let y = FLOOR + k as f32 * b.storey;
        walls(m, &inner, y - 0.18, y, cut);
        top(m, &inner, y, hex(palette::BOARDS));
    }
    roof(h, b, top_y, 1.0 - smooth(o.p / ROOF_GONE), &[]);
}

/// The doors and windows on the wall from `a` to `z` (server), as far up
/// as `wall_top`, on `storeys` storeys.
#[allow(clippy::too_many_arguments)]
fn openings(
    h: &mut House,
    b: &Block,
    a: P,
    z: P,
    front: bool,
    wall_top: f32,
    storeys: u8,
    lights: &mut Seed,
) {
    let length = dist(a, z) as f32 * S;
    let dir = unit(sub(z, a));
    let mid = scale(add(a, z), 0.5);
    let out = level(sub(mid, b.rect.c));
    let off = scale([out[0] as f64, out[2] as f64], metres(PROUD));
    // `k` times proud of the wall: a pane in a frame stands prouder.
    let at = |s: f32, k: f64| add(add(a, scale(dir, metres(s))), scale(off, k));
    let quad_at = |s0: f32, s1: f32, y0: f32, y1: f32, k: f64| {
        [
            lift(at(s0, k), y0),
            lift(at(s1, k), y0),
            lift(at(s1, k), y1),
            lift(at(s0, k), y1),
        ]
    };
    let quad = |s0: f32, s1: f32, y0: f32, y1: f32| quad_at(s0, s1, y0, y1, 1.0);
    // The door, on the front.
    let mut door_at = None;
    if front {
        match b.door {
            Door::Front(k, colour) => {
                let w = 0.85_f32;
                let s = (length * k).clamp(w * 0.5 + 0.15, length - w * 0.5 - 0.15);
                let tall = (b.storey * 0.82).min(2.05);
                if wall_top >= FLOOR + tall {
                    h.body.quad(
                        quad(s - w * 0.5, s + w * 0.5, FLOOR, FLOOR + tall),
                        out,
                        colour,
                    );
                    // A fanlight over it, always lit.
                    h.panes.push(Pane {
                        corners: quad(
                            s - w * 0.4,
                            s + w * 0.4,
                            FLOOR + tall + 0.08,
                            FLOOR + tall + 0.3,
                        ),
                        normal: out,
                        lit: true,
                    });
                }
                door_at = Some(s);
            }
            Door::Wide(colour) => {
                let w = (length * 0.72).min(3.0);
                let tall = (b.storey * 0.78).min(2.6);
                if wall_top >= FLOOR + tall {
                    let s = length * 0.5;
                    h.body.quad(
                        quad(s - w * 0.5, s + w * 0.5, FLOOR, FLOOR + tall),
                        out,
                        colour,
                    );
                }
                return;
            }
            Door::Shop { arch_left, colour } => {
                // The carriage arch at one end, under its lintel, the shop
                // door at the other, and the shop window between.
                let tall = (b.storey * 0.88).min(2.5);
                let arch_w = (length * 0.28).min(2.3);
                let (arch, door) = if arch_left {
                    (0.3 + arch_w * 0.5, length - 0.8)
                } else {
                    (length - 0.3 - arch_w * 0.5, 0.8)
                };
                if wall_top >= FLOOR + tall + 0.2 {
                    let (a0, a1) = (arch - arch_w * 0.5, arch + arch_w * 0.5);
                    h.body.quad(quad(a0, a1, FLOOR, FLOOR + tall), out, hex(palette::ARCH));
                    h.body.quad(
                        quad(a0 - 0.12, a1 + 0.12, FLOOR + tall, FLOOR + tall + 0.2),
                        out,
                        palette::shade(b.wall, 0.78),
                    );
                    h.body.quad(quad(door - 0.45, door + 0.45, FLOOR, FLOOR + 2.05), out, colour);
                    let (w0, w1) = if arch_left {
                        (a1 + 0.4, door - 0.75)
                    } else {
                        (door + 0.75, a0 - 0.4)
                    };
                    if w1 - w0 > 0.8 {
                        h.body.quad(quad(w0, w1, FLOOR + 0.4, FLOOR + 2.1), out, hex(palette::SHOPFRONT));
                        h.panes.push(Pane {
                            corners: quad_at(w0 + 0.1, w1 - 0.1, FLOOR + 0.5, FLOOR + 2.0, 2.0),
                            normal: out,
                            lit: lights.chance(0.85),
                        });
                    }
                }
                door_at = Some(door);
            }
        }
    }
    if matches!(b.door, Door::Wide(_)) {
        return;
    }
    let shop = matches!(b.door, Door::Shop { .. });
    let count = ((length - 0.5) / b.bay).floor().max(0.0) as usize;
    if count == 0 {
        return;
    }
    let step = length / count as f32;
    for storey in 0..storeys {
        // The shop's ground floor is its front, on the street side.
        if storey == 0 && shop && front {
            continue;
        }
        let y0 = FLOOR + storey as f32 * b.storey + b.storey * 0.36;
        let y1 = y0 + b.storey * 0.42;
        for k in 0..count {
            let s = step * (k as f32 + 0.5);
            // Where the door is, no window on the ground floor.
            if storey == 0 && door_at.is_some_and(|d| (d - s).abs() < (b.window + 0.85) * 0.55) {
                continue;
            }
            let lit = lights.chance(0.68);
            if wall_top < y1 {
                continue;
            }
            h.panes.push(Pane {
                corners: quad(s - b.window * 0.5, s + b.window * 0.5, y0, y1),
                normal: out,
                lit,
            });
        }
    }
}

/// The house on the lot `id`, `progress` of the way up (1 is built), or
/// `None` if the lot has no room for one.
#[cfg(test)]
pub fn house(id: &str, lot: &Lot, progress: f32) -> Option<House> {
    build(id, lot, progress, &Look::default())
}

/// The house on the lot `id`, `progress` of the way up (1 is built), looking
/// as `look` says: its floors, a floor going up, its
/// fires and how far it's open (V3.4).
pub fn build(id: &str, lot: &Lot, progress: f32, look: &Look) -> Option<House> {
    let plan = plan(id, lot)?;
    Some(raise(&plan, Stage::at(progress), look))
}

/// The material of every [`lamp_meshes`] entity at `glow`, set each frame
/// on panes uploaded once as white light. The pane's colour is the tint
/// (`vertex_tint_*`, Flint ADR 0102): glass by day, the lamp's at night,
/// and between them as the light's share of the glow. `glow_surface` (ADR
/// 0096) is how much of it the sun lights and `emissive_scale` (ADR 0090)
/// how much light it gives off, so it draws as the pane rebuilt with the
/// hour before H23 did: `palette::lit_glow(hue, surface, radiance × 1.1)`.
/// By day it's plain glass, as the dark windows are.
pub fn lamp_glow(glow: PlotGlow) -> [(&'static str, f32); 5] {
    let glass = hex(palette::GLASS);
    let total = glow.surface + glow.radiance;
    let (hue, surface, light) = if glow.radiance <= 0.0 {
        (glass, 1.0, 0.0)
    } else {
        let hue = palette::mix(glass, hex(palette::LAMP), glow.radiance / total);
        (hue, glow.surface.clamp(0.0, 1.0), glow.radiance * WINDOW_GLOW)
    };
    [
        ("vertex_tint_r", hue[0]),
        ("vertex_tint_g", hue[1]),
        ("vertex_tint_b", hue[2]),
        ("glow_surface", surface),
        ("emissive_scale", light),
    ]
}

/// The windows' light against the plots' gold.
const WINDOW_GLOW: f32 = 1.1;

/// The panes that are `lit` (or not). Dark ones are glass; lit ones are
/// white light, which their entity colours and lights by the hour
/// ([`lamp_glow`]).
fn panes(m: &mut MeshBuf, panes: &[Pane], lit: bool) {
    let colour = if lit {
        palette::glow([1.0; 4], 1.0)
    } else {
        hex(palette::GLASS)
    };
    for p in panes.iter().filter(|p| p.lit == lit) {
        m.quad(p.corners, p.normal, colour);
    }
}

/// The lit panes' material: glass's, with their light and colour all their
/// entity's ([`lamp_glow`]).
fn lamps_material(name: &str) -> ImportedMaterial {
    ImportedMaterial {
        roughness: 0.25,
        ..ImportedMaterial::vertex_colored(name)
    }
}

// ---------------------------------------------------------------------------
// The layer
// ---------------------------------------------------------------------------

/// How near the camera must come for a building to open (V3.4), metres:
/// one of your own, and anyone else's. Chris, 2026-10-22: start there and
/// see how it feels.
pub const OPEN_MINE: f32 = 25.0;
pub const OPEN_OTHERS: f32 = 10.0;
/// How many stand open at once: the nearest.
const OPEN_MOST: usize = 6;
/// How long a building takes to open, or to shut, seconds.
const OPEN_SECS: f32 = 0.7;
/// The buildings standing open, and their lit windows.
pub const OPEN_MESH: &str = "houses_open";
pub const OPEN_LAMPS: &str = "houses_open_lamps";

/// A house that's up.
struct Up {
    sector: Sector,
    house: House,
    /// Its building, the rules' footprint, and how tall it stands, metres:
    /// what opens. `None` for a guild hall's.
    building: Option<(Rect, f32)>,
}

/// What a lot's building shows past its plan, as the layer was told.
#[derive(Clone, Debug, Default, PartialEq)]
struct Shown {
    /// Floors built (V3.3).
    floors: u8,
    /// The next is under way: it stands in scaffolding.
    under_way: bool,
    /// The last one finished at this time, and its storey is rising.
    rising_since: Option<f32>,
    /// The kilns and forges inside, server units.
    fires: Vec<P>,
    /// The highest storey a station stands on inside (V3.3), 0 the ground:
    /// an open building shows the storeys up to it.
    top: u8,
}

/// Every house in the city, and what needs building again.
#[derive(Default)]
pub struct Houses {
    /// The claims last followed (`sync`), by the version they came with.
    version: Option<u64>,
    claimed: HashSet<String>,
    /// The houses that are up, by lot id.
    built: HashMap<String, Up>,
    /// The houses going up, by lot id: when they started.
    growing: HashMap<String, f32>,
    /// Floors, scaffolding and fires, by lot id.
    shown: HashMap<String, Shown>,
    /// The fires last given (`fires`), to tell when they change.
    fires_given: Vec<P>,
    /// The stations upstairs last given (`upstairs`), with how many houses
    /// were up then, to tell when either changes.
    upstairs_given: (Vec<(P, u8)>, usize),
    /// The buildings open or opening, by lot id: how far, 0 to 1.
    open: HashMap<String, f32>,
    /// Which of each open building's sides face the camera.
    facing: HashMap<String, u8>,
    /// Where the camera is, server units.
    eye: P,
    /// The open mesh is out of date, and whether it has anything in it.
    open_dirty: bool,
    open_drawn: bool,
    /// Sectors whose mesh is out of date.
    dirty: BTreeSet<Sector>,
    /// Whether the growing mesh has anything in it.
    growing_drawn: bool,
    /// Seconds since the layer began, for growth.
    time: f32,
}

impl Houses {
    /// The lots were built again (a new terrain): every house follows.
    pub fn lots_changed(&mut self) {
        self.version = None;
        self.claimed.clear();
        self.built.clear();
        self.growing.clear();
        self.open.clear();
        self.facing.clear();
        self.open_dirty = true;
        self.dirty.extend(Sector::all());
    }

    /// Follows the claimed lots: `ids`, whenever `version` moves. A lot
    /// newly claimed grows if `fresh` says it was claimed just now, and is
    /// otherwise simply there. A claim whose lot isn't drawn has no house.
    pub fn sync<'a>(
        &mut self,
        version: u64,
        ids: impl IntoIterator<Item = &'a String>,
        fresh: impl Fn(&str) -> bool,
        lots: &HashMap<String, Lot>,
    ) {
        if self.version == Some(version) {
            return;
        }
        self.version = Some(version);
        let now: HashSet<String> = ids.into_iter().cloned().collect();
        for gone in self.claimed.difference(&now) {
            if let Some(up) = self.built.remove(gone) {
                self.dirty.insert(up.sector);
            }
            self.growing.remove(gone);
            self.shown.remove(gone);
            if self.open.remove(gone).is_some() {
                self.open_dirty = true;
            }
        }
        let new: Vec<String> = now.difference(&self.claimed).cloned().collect();
        for new in &new {
            let Some(lot) = lots.get(new) else {
                continue;
            };
            if fresh(new) {
                self.growing.insert(new.clone(), self.time);
            } else {
                self.put_up(new, lot);
            }
        }
        self.claimed = now;
        let (up, growing) = self.counts();
        tracing::debug!("houses: {up} up, {growing} growing");
    }

    /// Builds the house on `id` as it stands now, into its sector.
    fn put_up(&mut self, id: &str, lot: &Lot) {
        let look = self.look(id);
        let Some(house) = build(id, lot, 1.0, &look) else {
            return;
        };
        let building = plan(id, lot).and_then(|plan| {
            let tall = plan.blocks.iter().map(|b| look.storeys(b) as f32 * b.storey).fold(0.0, f32::max);
            Some((plan.building?, tall))
        });
        self.dirty.insert(lot.sector);
        self.built.insert(
            id.to_owned(),
            Up {
                sector: lot.sector,
                house,
                building,
            },
        );
    }

    /// How `id`'s building looks now, shut.
    fn look(&self, id: &str) -> Look {
        let Some(s) = self.shown.get(id) else {
            return Look::default();
        };
        let (floors, raising) = match (s.under_way, s.rising_since) {
            (true, _) => (s.floors, Some(0.0)),
            (false, Some(t0)) => (
                s.floors.saturating_sub(1),
                Some(((self.time - t0) / GROW_SECS).clamp(0.0, 1.0)),
            ),
            _ => (s.floors, None),
        };
        Look {
            floors,
            raising,
            fires: s.fires.clone(),
            open: None,
        }
    }

    /// Whether `id`'s new storey is rising: it's drawn each frame then.
    fn rising(&self, id: &str) -> bool {
        self.shown.get(id).is_some_and(|s| s.rising_since.is_some())
    }

    /// The lot `id` has `floors` built (V3.3), and the next `under_way` or
    /// not. A floor that has just finished rises over [`GROW_SECS`].
    pub fn set_floors(&mut self, id: &str, floors: u8, under_way: bool, lots: &HashMap<String, Lot>) {
        let s = self.shown.entry(id.to_owned()).or_default();
        if s.floors == floors && s.under_way == under_way {
            return;
        }
        let finished = s.under_way && !under_way && floors == s.floors + 1;
        s.floors = floors;
        s.under_way = under_way;
        s.rising_since = finished.then_some(self.time);
        if finished {
            // It leaves its sector to rise in the growing mesh.
            if let Some(up) = self.built.get(id) {
                self.dirty.insert(up.sector);
            }
        } else if self.built.contains_key(id) {
            if let Some(lot) = lots.get(id) {
                self.put_up(id, lot);
            }
        }
    }

    /// The stations upstairs (V3.3), server units with their storey,
    /// wherever they are: an open building shows its storeys up to the
    /// highest one with a station on it.
    pub fn upstairs(&mut self, stations: &[(P, u8)]) {
        if self.upstairs_given.0 == stations && self.upstairs_given.1 == self.built.len() {
            return;
        }
        self.upstairs_given = (stations.to_vec(), self.built.len());
        let mut top: HashMap<String, u8> = HashMap::new();
        for &(p, floor) in stations {
            let home = self
                .built
                .iter()
                .find(|(_, up)| up.building.is_some_and(|(fp, _)| fp.contains(p)));
            if let Some((id, _)) = home {
                let t = top.entry(id.clone()).or_default();
                *t = (*t).max(floor);
            }
        }
        let mut changed = false;
        for (id, s) in self.shown.iter_mut() {
            let want = top.remove(id).unwrap_or(0);
            if s.top != want {
                s.top = want;
                changed = true;
            }
        }
        for (id, t) in top {
            self.shown.entry(id).or_default().top = t;
            changed = true;
        }
        if changed {
            self.open_dirty = true;
        }
    }

    /// The kilns and forges indoors, server units, wherever they are: each
    /// house with one in it has a chimney over it.
    pub fn fires(&mut self, fires: &[P], lots: &HashMap<String, Lot>) {
        if self.fires_given == fires {
            return;
        }
        self.fires_given = fires.to_vec();
        let mut by_lot: HashMap<String, Vec<P>> = HashMap::new();
        for &p in fires {
            let home = self
                .built
                .iter()
                .find(|(_, up)| up.building.is_some_and(|(fp, _)| fp.contains(p)));
            if let Some((id, _)) = home {
                by_lot.entry(id.clone()).or_default().push(p);
            }
        }
        let lots_now: BTreeSet<String> = self
            .shown
            .iter()
            .filter(|(_, s)| !s.fires.is_empty())
            .map(|(id, _)| id.clone())
            .chain(by_lot.keys().cloned())
            .collect();
        for id in lots_now {
            let want = by_lot.remove(&id).unwrap_or_default();
            let s = self.shown.entry(id.clone()).or_default();
            if s.fires == want {
                continue;
            }
            s.fires = want;
            if let Some(lot) = lots.get(&id).filter(|_| self.built.contains_key(&id)) {
                self.put_up(&id, lot);
            }
        }
    }

    /// The fires with a chimney over them, and the top of it, world
    /// metres: where the smoke comes out. A building standing open has
    /// none.
    pub fn chimneys(&self) -> impl Iterator<Item = (P, [f32; 3])> + '_ {
        self.built
            .iter()
            .filter(|(id, _)| !self.open.contains_key(*id) && !self.rising(id))
            .flat_map(|(_, up)| up.house.chimneys.iter().copied())
    }

    /// Opens the buildings the camera has come close to, and shuts the
    /// rest (V3.4): within [`OPEN_MINE`] of one of `mine`, [`OPEN_OTHERS`]
    /// of anyone else's, the nearest few. `eye` is the camera, world metres.
    /// By distance alone: opening on aim, and from 25 m for a plot that
    /// admits you, are #496.
    pub fn cutaway(&mut self, dt: f32, eye: [f32; 3], mine: &HashSet<String>) {
        let at = to_server(eye[0], eye[2]);
        let mut near: Vec<(f32, &String)> = self
            .built
            .iter()
            .filter(|(id, _)| !self.rising(id))
            .filter_map(|(id, up)| {
                let (fp, tall) = up.building?;
                let across = fp.distance(at) as f32 * S;
                let above = (eye[1] - FLOOR - tall).max(0.0);
                let d = across.hypot(above);
                let reach = if mine.contains(id) { OPEN_MINE } else { OPEN_OTHERS };
                (d < reach).then_some((d, id))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
        let want: HashSet<String> = near.iter().take(OPEN_MOST).map(|(_, id)| (*id).clone()).collect();
        for id in &want {
            self.open.entry(id.clone()).or_insert(0.0);
        }
        let step = dt / OPEN_SECS;
        let mut moved = Vec::new();
        for (id, p) in &mut self.open {
            let target = if want.contains(id) { 1.0 } else { 0.0 };
            let next = if target > *p {
                (*p + step).min(1.0)
            } else {
                (*p - step).max(0.0)
            };
            if next != *p {
                // In or out of its sector's mesh as it starts or ends.
                moved.push((id.clone(), *p == 0.0 || next == 0.0));
                *p = next;
            }
        }
        for (id, leaves) in &moved {
            if *leaves {
                if let Some(up) = self.built.get(id) {
                    self.dirty.insert(up.sector);
                }
            }
        }
        self.open.retain(|id, p| *p > 0.0 || want.contains(id));
        self.facing.retain(|id, _| self.open.contains_key(id));
        // The walls cut are the ones facing the camera.
        for id in self.open.keys() {
            let Some((fp, _)) = self.built.get(id).and_then(|up| up.building) else {
                continue;
            };
            let c = fp.corners();
            let mut sides = 0u8;
            for i in 0..4 {
                let mid = scale(add(c[i], c[(i + 1) % 4]), 0.5);
                if dot(sub(at, mid), sub(mid, fp.c)) > 0.0 {
                    sides |= 1 << i;
                }
            }
            if self.facing.insert(id.clone(), sides) != Some(sides) {
                self.open_dirty = true;
            }
        }
        self.eye = at;
        if !moved.is_empty() {
            self.open_dirty = true;
        }
    }

    /// How far open `id`'s building is, 0 shut.
    #[cfg(test)]
    pub fn openness(&self, id: &str) -> f32 {
        self.open.get(id).copied().unwrap_or(0.0)
    }

    /// Moves time on by `dt`: a house that has finished growing joins its
    /// sector, and so does a storey that has finished rising.
    pub fn advance(&mut self, dt: f32, lots: &HashMap<String, Lot>) {
        self.time += dt;
        let time = self.time;
        let done: Vec<String> = self
            .growing
            .iter()
            .filter(|(_, &t0)| time - t0 >= GROW_SECS)
            .map(|(id, _)| id.clone())
            .collect();
        for id in done {
            self.growing.remove(&id);
            if let Some(lot) = lots.get(&id) {
                self.put_up(&id, lot);
            }
        }
        let risen: Vec<String> = self
            .shown
            .iter()
            .filter(|(_, s)| s.rising_since.is_some_and(|t0| time - t0 >= GROW_SECS))
            .map(|(id, _)| id.clone())
            .collect();
        for id in risen {
            if let Some(s) = self.shown.get_mut(&id) {
                s.rising_since = None;
            }
            if let Some(lot) = lots.get(&id).filter(|_| self.built.contains_key(&id)) {
                self.put_up(&id, lot);
            }
        }
    }

    /// How many houses are up, and how many going up.
    pub fn counts(&self) -> (usize, usize) {
        (self.built.len(), self.growing.len())
    }

    /// The houses drawn in their sectors' meshes: not the open ones, nor
    /// one whose new storey is rising.
    fn settled(&self) -> impl Iterator<Item = &Up> {
        self.built
            .iter()
            .filter(|(id, _)| !self.open.contains_key(*id) && !self.rising(id))
            .map(|(_, up)| up)
    }

    /// One sector's houses as one mesh.
    fn sector_mesh(&self, sector: Sector) -> MeshBuf {
        let mut m = MeshBuf::default();
        for up in self.settled() {
            if up.sector == sector {
                m.append(up.house.body.clone());
            }
        }
        m
    }

    /// The houses going up, and the storeys rising, as they stand now,
    /// with their dark windows; and their lit windows apart.
    fn growing_mesh(&self, lots: &HashMap<String, Lot>) -> (MeshBuf, MeshBuf) {
        let (mut m, mut lamps) = (MeshBuf::default(), MeshBuf::default());
        let growing = self.growing.iter().map(|(id, &t0)| {
            let p = ((self.time - t0) / GROW_SECS).clamp(0.0, 0.999);
            (id, p, self.look(id))
        });
        let rising = self
            .built
            .keys()
            .filter(|id| self.rising(id))
            .map(|id| (id, 1.0, self.look(id)));
        for (id, p, look) in growing.chain(rising) {
            let Some(lot) = lots.get(id) else {
                continue;
            };
            if let Some(h) = build(id, lot, p, &look) {
                m.append(h.body);
                panes(&mut m, &h.panes, false);
                panes(&mut lamps, &h.panes, true);
            }
        }
        (m, lamps)
    }

    /// The buildings standing open, as they stand now, with their dark
    /// windows; and their lit windows apart.
    fn open_mesh(&self, lots: &HashMap<String, Lot>) -> (MeshBuf, MeshBuf) {
        let (mut m, mut lamps) = (MeshBuf::default(), MeshBuf::default());
        for (id, &p) in &self.open {
            let Some(lot) = lots.get(id) else {
                continue;
            };
            let look = Look {
                open: Some(Open {
                    p,
                    eye: self.eye,
                    // Up to the highest storey in use (V3.3); the aimed
                    // one once aiming opens a building (#496)
                    shown: self.shown.get(id).map_or(1, |s| s.top.saturating_add(1)),
                }),
                ..self.look(id)
            };
            if let Some(h) = build(id, lot, 1.0, &look) {
                m.append(h.body);
                panes(&mut m, &h.panes, false);
                panes(&mut lamps, &h.panes, true);
            }
        }
        (m, lamps)
    }

    /// Uploads whatever changed, and returns the names of the meshes
    /// uploaded with something in them. The hour never rebuilds a mesh:
    /// the lamps take it as their entities' `emissive_scale`.
    pub fn upload(&mut self, gpu: &mut ExtensionGpu, lots: &HashMap<String, Lot>) -> Vec<String> {
        let material = |name: &str, roughness: f32| ImportedMaterial {
            roughness,
            ..ImportedMaterial::vertex_colored(name)
        };
        let mut uploaded = Vec::new();
        let dirty = std::mem::take(&mut self.dirty);
        for &sector in &dirty {
            let name = mesh_name(sector);
            let m = self.sector_mesh(sector);
            if !m.indices.is_empty() {
                uploaded.push(name.clone());
            }
            // The walls and roofs: the camera keeps out of them (#293).
            m.upload_material_with(gpu, material(&name, 0.85), RAY_TARGET);
        }
        // A quarter's windows with any of its sectors.
        let quarters: BTreeSet<(u8, u8)> = dirty.iter().copied().map(quarter).collect();
        for q in quarters {
            let (mut dark, mut lamps) = (MeshBuf::default(), MeshBuf::default());
            for up in self.settled() {
                if quarter(up.sector) == q {
                    panes(&mut dark, &up.house.panes, false);
                    panes(&mut lamps, &up.house.panes, true);
                }
            }
            for (m, material) in [
                (dark, material(&windows_name(q), 0.25)),
                (lamps, lamps_material(&lamps_name(q))),
            ] {
                if !m.indices.is_empty() {
                    uploaded.push(material.name.clone());
                }
                m.upload_material(gpu, material);
            }
        }
        let rising = self.shown.values().any(|s| s.rising_since.is_some());
        if !self.growing.is_empty() || rising || self.growing_drawn {
            let (m, lamps) = self.growing_mesh(lots);
            self.growing_drawn = !m.indices.is_empty();
            for (m, material, options) in [
                (m, material(GROWING_MESH, 0.85), RAY_TARGET),
                (lamps, lamps_material(GROWING_LAMPS), Default::default()),
            ] {
                if !m.indices.is_empty() {
                    uploaded.push(material.name.clone());
                }
                m.upload_material_with(gpu, material, options);
            }
        }
        if std::mem::take(&mut self.open_dirty) && (!self.open.is_empty() || self.open_drawn) {
            let (m, lamps) = self.open_mesh(lots);
            self.open_drawn = !m.indices.is_empty();
            for (m, material, options) in [
                (m, material(OPEN_MESH, 0.85), RAY_TARGET),
                (lamps, lamps_material(OPEN_LAMPS), Default::default()),
            ] {
                if !m.indices.is_empty() {
                    uploaded.push(material.name.clone());
                }
                m.upload_material_with(gpu, material, options);
            }
        }
        uploaded
    }
}

/// How recent a claim must be, when first seen, for its house to grow
/// rather than simply be there, ms. Either way of the server's clock, for
/// skew.
const FRESH_MS: i64 = 8_000;

/// Whether a lot claimed at `claimed_ms` (Unix ms) was claimed just now,
/// by the server's clock `now_ms`. With no clock, no claim is.
pub fn fresh(claimed_ms: i64, now_ms: Option<i64>) -> bool {
    now_ms.is_some_and(|now| (now - claimed_ms).abs() < FRESH_MS)
}

/// Offline, a city of claims to look at (`--claim-sample`): `share` of the
/// lots, picked by id, already built, and with `--grow-at`, the lots
/// nearest the camera claimed at that second, so they grow.
pub struct Sample {
    share: f32,
    grow_at: Option<f32>,
    time: f32,
    /// How many lots the claims were drawn from, to draw again when the
    /// terrain changes.
    from: usize,
    pub claimed: Vec<String>,
    pub growing: HashSet<String>,
    pub version: u64,
}

/// How many lots `--grow-at` claims.
const SAMPLE_GROWN: usize = 8;

impl Sample {
    pub fn new(share: f32, grow_at: Option<f32>) -> Self {
        Self {
            share,
            grow_at,
            time: 0.0,
            from: 0,
            claimed: Vec::new(),
            growing: HashSet::new(),
            version: 0,
        }
    }

    /// Moves on by `dt` seconds, with the camera looking at server `focus`.
    pub fn update(&mut self, dt: f32, focus: P, lots: &HashMap<String, Lot>) {
        self.time += dt;
        if lots.len() != self.from {
            self.from = lots.len();
            self.claimed = lots
                .keys()
                .filter(|id| Seed::of(&format!("sample {id}")).unit() < self.share)
                .cloned()
                .collect();
            self.version += 1;
        }
        if self.grow_at.is_some_and(|t| self.time >= t) && !lots.is_empty() {
            self.grow_at = None;
            let taken: HashSet<&String> = self.claimed.iter().collect();
            let mut free: Vec<(&String, f64)> = lots
                .iter()
                .filter(|(id, _)| !taken.contains(id))
                .map(|(id, lot)| {
                    let [a, b] = lot.street;
                    (id, dist(scale(add(a, b), 0.5), focus))
                })
                .collect();
            free.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(b.0)));
            let grown: Vec<String> = free
                .iter()
                .take(SAMPLE_GROWN)
                .map(|(id, _)| (*id).clone())
                .collect();
            self.growing.extend(grown.iter().cloned());
            self.claimed.extend(grown);
            self.version += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::to_server;
    use crate::world::mesh::{facing, is_light};
    use crate::world::plots;
    use std::path::Path;
    use std::sync::OnceLock;

    fn lots() -> &'static HashMap<String, Lot> {
        static BUILT: OnceLock<HashMap<String, Lot>> = OnceLock::new();
        BUILT.get_or_init(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
            let t = capitol_core::terrain::read_terrain_file(&path).expect("fixture");
            plots::build(&t.terrain_plots).lots
        })
    }

    fn first(zone: Zone) -> (&'static String, &'static Lot) {
        let mut all: Vec<_> = lots().iter().filter(|(_, l)| l.zone == zone).collect();
        all.sort_by_key(|(id, _)| *id);
        all[0]
    }

    fn vertices(h: &House) -> impl Iterator<Item = [f32; 3]> + '_ {
        h.body
            .vertices
            .iter()
            .map(|v| v.position)
            .chain(h.panes.iter().flat_map(|p| p.corners))
    }

    #[test]
    fn nearly_every_lot_has_room_for_a_house() {
        // On a home lot, the house is the rules' building, where there is one.
        for (id, lot) in lots().iter().filter(|(_, l)| building::shape(l.zone).is_some()) {
            assert_eq!(plan(id, lot).is_some(), footprint(lot).is_some(), "{id}");
        }
        for zone in [
            Zone::Urban,
            Zone::Suburban,
            Zone::Rural,
            Zone::GuildDistrict,
        ] {
            let of_zone: Vec<_> = lots().iter().filter(|(_, l)| l.zone == zone).collect();
            let housed = of_zone
                .iter()
                .filter(|(id, l)| plan(id, l).is_some())
                .count();
            let share = housed as f64 / of_zone.len() as f64;
            println!("{zone}: {housed} of {}", of_zone.len());
            assert!(share > 0.95, "{zone}: only {housed} of {}", of_zone.len());
        }
    }

    /// Every vertex inside the lot as `edge` runs round it, `slack` server
    /// units allowed.
    fn on_lot(id: &str, h: &House, edge: &[P], slack: f64) {
        let ring = plots::hull(edge);
        let n = ring.len();
        for p in vertices(h) {
            let q = to_server(p[0], p[2]);
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                let inward = dot(unit(left(sub(b, a))), sub(q, a));
                assert!(inward >= -slack, "{id}: {q:?} is {inward} outside");
            }
        }
    }

    #[test]
    fn a_house_stays_on_its_lot() {
        // Every vertex, eaves and all, inside the lot as drawn. A building
        // on the rules' footprint stands on the lot itself: a city house
        // fronts the pavement, its cornice a hand's width over it.
        for (id, lot) in lots() {
            let Some(h) = house(id, lot, 1.0) else {
                continue;
            };
            if footprint(lot).is_some() {
                on_lot(id, &h, &lot.corners, metres(CORNICE) + 0.05);
            } else {
                on_lot(id, &h, &lot.outline, 0.05);
            }
        }
    }

    /// V3.4, #462: the blocks fill the rules' footprint exactly, so every
    /// station indoors stands inside walls drawn round it, and every one
    /// in the yard outside them.
    #[test]
    fn the_walls_stand_on_the_rules_footprint() {
        let mut n = 0;
        for (id, lot) in lots() {
            let Some(fp) = footprint(lot) else {
                continue;
            };
            let p = plan(id, lot).unwrap();
            assert_eq!(p.building, Some(fp));
            let area: f64 = p.blocks.iter().map(|b| 4.0 * b.rect.hx * b.rect.hy).sum();
            assert!((area - 4.0 * fp.hx * fp.hy).abs() < 1e-6 * area, "{id}: the blocks leave a gap");
            for b in &p.blocks {
                for c in b.rect.corners() {
                    assert!(fp.grown(1e-6).contains(c), "{id}: a block past the footprint");
                }
            }
            // And the server's test of indoors agrees, a station's width in.
            let corners = lot.corners.map(|[x, y]| (x, y));
            for (s, t) in [(0.0, 0.0), (0.8, 0.8), (-0.8, 0.8), (0.8, -0.8), (-0.8, -0.8)] {
                let q = fp.at(fp.hx * s, fp.hy * t);
                assert!(building::indoors(&corners, lot.zone, q[0], q[1]), "{id}: {q:?}");
                assert!(p.blocks.iter().any(|b| b.rect.contains(q)), "{id}: {q:?} under no roof");
            }
            let out = fp.at(0.0, fp.hy + 5.0);
            assert!(!p.blocks.iter().any(|b| b.rect.contains(out)), "{id}: the yard is roofed");
            n += 1;
        }
        assert!(n > 100, "{n} buildings");
    }

    /// The highest any vertex of `h` stands, metres.
    fn tallest(h: &House) -> f32 {
        vertices(h).map(|p| p[1]).fold(FLOOR, f32::max)
    }

    #[test]
    fn floors_raise_it_a_storey_each_and_scaffold_the_one_going_up() {
        let (id, lot) = first(Zone::Urban);
        let storey = plan(id, lot).unwrap().blocks[0].storey;
        let at = |look: Look| build(id, lot, 1.0, &look).unwrap();
        let none = at(Look::default());
        let two = at(Look { floors: 2, ..Look::default() });
        assert!((tallest(&two) - tallest(&none) - 2.0 * storey).abs() < 1e-3);
        let has = |h: &House, colour: u32| h.body.vertices.iter().any(|v| v.color == hex(colour));
        assert!(!has(&two, palette::SCAFFOLD));
        // Under way: the storey stands in its frame, in scaffolding, with
        // the roof on top; then it rises, and the scaffolding comes down.
        let going = at(Look { floors: 2, raising: Some(0.0), ..Look::default() });
        assert!(has(&going, palette::SCAFFOLD) && has(&going, palette::TIMBER));
        assert!(tallest(&going) > tallest(&two) + storey - 1e-3);
        let three = at(Look { floors: 3, ..Look::default() });
        let risen = at(Look { floors: 2, raising: Some(1.0), ..Look::default() });
        assert!((tallest(&risen) - tallest(&three)).abs() < 1e-3);
        // More windows for more storeys.
        assert!(three.panes.len() > none.panes.len());
    }

    #[test]
    fn a_chimney_rises_through_the_roof_over_each_fire() {
        for zone in [Zone::Urban, Zone::Suburban, Zone::Rural] {
            let (id, lot) = first(zone);
            let fp = footprint(lot).unwrap();
            let fires = vec![fp.at(-fp.hx * 0.5, 0.0), fp.at(fp.hx * 0.5, fp.hy * 0.4)];
            let shut = build(id, lot, 1.0, &Look::default()).unwrap();
            assert!(shut.chimneys.is_empty(), "no fire, no chimney");
            let h = build(id, lot, 1.0, &Look { fires: fires.clone(), ..Look::default() }).unwrap();
            assert_eq!(h.chimneys.len(), 2, "{id}");
            let p = plan(id, lot).unwrap();
            for (fire, top) in &h.chimneys {
                let b = p.blocks.iter().find(|b| b.rect.contains(*fire)).unwrap();
                let walls = FLOOR + b.storeys as f32 * b.storey;
                let at = to_server(top[0], top[2]);
                assert!(dist(at, *fire) < 1.0 / S as f64, "{id}: over its fire");
                assert!(top[1] > roof_height(b, walls, at) + 0.5, "{id}: clear of the roof");
            }
            // Open, it has none.
            let open = Look {
                fires,
                open: Some(Open { p: 1.0, eye: fp.at(0.0, -fp.hy - 100.0), shown: 1 }),
                ..Look::default()
            };
            assert!(build(id, lot, 1.0, &open).unwrap().chimneys.is_empty());
        }
    }

    /// V3.4: opened, the roof is gone, the walls toward the camera are
    /// down to the sill and the far ones stand a storey high.
    #[test]
    fn it_opens_cut_down_toward_the_camera() {
        for zone in [Zone::Urban, Zone::Suburban, Zone::Rural] {
            let (id, lot) = first(zone);
            let fp = footprint(lot).unwrap();
            let p = plan(id, lot).unwrap();
            let storey = p.blocks.iter().map(|b| b.storey).fold(0.0, f32::max);
            // From the street, a little to the left.
            let eye = fp.at(-fp.hx * 0.3, -fp.hy - 120.0);
            let open = |k: f32, floors: u8| {
                let look = Look { floors, open: Some(Open { p: k, eye, shown: 1 }), ..Look::default() };
                build(id, lot, 1.0, &look).unwrap()
            };
            let h = open(1.0, 2);
            assert!(tallest(&h) <= FLOOR + storey + 1e-3, "{id}: {} over a storey", tallest(&h));
            let w = metres(WALL);
            for v in vertices(&h) {
                // The front wall, not the ends of the side walls at its
                // corners: the far one runs the whole depth.
                let (s, t) = fp.local(to_server(v[0], v[2]));
                if t < -fp.hy + w * 0.9 && s.abs() < fp.hx - w * 1.1 {
                    assert!(v[1] <= FLOOR + SILL + 1e-3, "{id}: the front stands {}", v[1]);
                }
            }
            assert!(
                h.body.vertices.iter().any(|v| v.color == hex(palette::BOARDS)),
                "{id}: no floor"
            );
            // Shut, it's the house; half open, between.
            let shut = build(id, lot, 1.0, &Look { floors: 2, ..Look::default() }).unwrap();
            assert!(tallest(&open(0.6, 2)) < tallest(&shut));
            assert!(tallest(&open(0.6, 2)) > tallest(&h));
        }
    }

    /// R6: every farm has a farmstead on a drive from the lane, standing at
    /// least 5 m back on `back_share` of them, with a barn behind it on more
    /// than `barn_share`.
    fn farmsteads_stand_back_on_their_drives(lots: &HashMap<String, Lot>, back_share: f64, barn_share: f64) {
        let (mut n, mut barns, mut back) = (0, 0, 0);
        for (id, lot) in lots {
            let plan = plan(id, lot).unwrap_or_else(|| panic!("{id} has no room for a farmhouse"));
            let frame = Frame::of(lot).unwrap();
            let t = |p: P| dot(sub(p, frame.o), frame.v);
            let [out, door] = plan.drive.expect("a farm has a drive");
            // In from the lane, to the front wall.
            assert!(t(out) < 0.0 && t(door) > frame.depth().0, "{id}: {out:?} to {door:?}");
            assert!((t(door) - t(plan.blocks[0].rect.c) + plan.blocks[0].rect.hy).abs() < 1e-6);
            for b in &plan.blocks {
                assert!(inside(&frame, &b.rect, 0.0), "{id}: a building off its lot");
            }
            n += 1;
            barns += (plan.blocks.len() > 1) as usize;
            // The rules set it 4 m back from the lot's edge (V3.1), which is
            // a little less from the hedge bank.
            back += ((t(door) - frame.depth().0) as f32 * S >= 3.5) as usize;
        }
        let share = |k: usize| k as f64 / n as f64;
        println!("{n} farms: {barns} barns, {back} stand 5 m back or more");
        assert!(share(barns) > barn_share, "{barns} barns on {n} farms");
        assert!(share(back) >= back_share, "{back} of {n} stand 5 m back");
    }

    /// The fixture's farms (R7).
    fn farm_lots() -> HashMap<String, Lot> {
        lots().iter().filter(|(_, l)| l.zone == Zone::Rural).map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    #[test]
    fn farmsteads_stand_back_from_the_lane_on_their_drives() {
        // Every farmhouse has its barn under the one range (V3.4).
        farmsteads_stand_back_on_their_drives(&farm_lots(), 0.9, 0.99);
    }

    #[test]
    fn a_farmstead_stays_on_its_farm() {
        for (id, lot) in &farm_lots() {
            let h = house(id, lot, 1.0).expect("a farmhouse");
            let n = lot.outline.len();
            for p in vertices(&h) {
                let q = to_server(p[0], p[2]);
                for i in 0..n {
                    let (a, b) = (lot.outline[i], lot.outline[(i + 1) % n]);
                    assert!(dot(unit(left(sub(b, a))), sub(q, a)) >= -0.05, "{id}: {q:?} off its farm");
                }
            }
        }
    }

    #[test]
    fn a_house_faces_its_street() {
        for zone in [Zone::Urban, Zone::Suburban, Zone::Rural] {
            let (id, lot) = first(zone);
            let p = plan(id, lot).unwrap();
            let r = p.blocks[0].rect;
            let line = |q: P| {
                let [a, b] = lot.street;
                dot(left(unit(sub(b, a))), sub(q, a)).abs()
            };
            assert!(line(r.at(0.0, -r.hy)) < line(r.at(0.0, r.hy)), "{id}");
        }
    }

    #[test]
    fn the_same_lot_builds_the_same_house() {
        let (id, lot) = first(Zone::Suburban);
        let (a, b) = (house(id, lot, 1.0).unwrap(), house(id, lot, 1.0).unwrap());
        let at = |h: &House| {
            h.body
                .vertices
                .iter()
                .map(|v| (v.position, v.color))
                .collect::<Vec<_>>()
        };
        assert_eq!(at(&a), at(&b));
        assert_eq!(a.panes, b.panes);
        // And the city isn't one house over and over.
        let mut heights: Vec<i32> = lots()
            .iter()
            .filter(|(_, l)| l.zone == Zone::Suburban)
            .take(200)
            .filter_map(|(id, l)| house(id, l, 1.0))
            .map(|h| {
                let top = vertices(&h).map(|p| p[1]).fold(0.0, f32::max);
                (top * 10.0) as i32
            })
            .collect();
        heights.sort();
        heights.dedup();
        assert!(heights.len() > 10, "{heights:?}");
    }

    #[test]
    fn every_face_faces_its_normal() {
        for zone in [
            Zone::Urban,
            Zone::Suburban,
            Zone::Rural,
            Zone::GuildDistrict,
        ] {
            for (id, lot) in lots().iter().filter(|(_, l)| l.zone == zone).take(40) {
                let fp = footprint(lot);
                let looks = [
                    Look::default(),
                    Look { floors: 1, raising: Some(0.0), fires: fp.iter().map(|f| f.c).collect(), open: None },
                    Look {
                        floors: 2,
                        open: fp.map(|f| Open { p: 0.8, eye: f.at(f.hx * 2.0, -f.hy * 3.0), shown: 2 }),
                        ..Look::default()
                    },
                ];
                for (p, look) in [0.2, 0.5, 0.8, 1.0].into_iter().map(|p| (p, &looks[0])).chain([(1.0, &looks[1]), (1.0, &looks[2])]) {
                    let Some(h) = build(id, lot, p, look) else {
                        continue;
                    };
                    for t in h.body.indices.as_chunks::<3>().0 {
                        let [a, b, c] = [t[0], t[1], t[2]].map(|i| h.body.vertices[i as usize]);
                        assert!(facing(a.position, b.position, c.position, a.normal), "{id}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_house_rises_frame_walls_then_roof() {
        let (id, lot) = first(Zone::Suburban);
        let height = |p: f32| {
            house(id, lot, p)
                .unwrap()
                .body
                .vertices
                .iter()
                .map(|v| v.position[1])
                .fold(FLOOR, f32::max)
        };
        let tops: Vec<f32> = (0..=10).map(|k| height(k as f32 / 10.0)).collect();
        assert!(tops.windows(2).all(|w| w[1] >= w[0] - 1e-4), "{tops:?}");
        assert!(tops[0] <= FLOOR + 0.01, "nothing at the start: {tops:?}");
        // Built is the same as fully grown, with no timber left showing.
        let built = house(id, lot, 1.0).unwrap();
        let timber = hex(palette::TIMBER);
        assert!(built.body.vertices.iter().all(|v| v.color != timber));
        let framing = house(id, lot, 0.2).unwrap();
        assert!(framing.body.vertices.iter().any(|v| v.color == timber));
        assert!(framing.panes.is_empty(), "no windows before walls");
    }

    #[test]
    fn windows_light_at_night_and_not_by_day() {
        let (id, lot) = first(Zone::Urban);
        let h = house(id, lot, 1.0).unwrap();
        assert!(!h.panes.is_empty());
        let lit = h.panes.iter().filter(|p| p.lit).count();
        assert!(lit > 0 && lit <= h.panes.len());
        // Dark panes are glass; lit ones are white light, coloured and lit
        // by their entity, and their material gives off nothing of its own.
        let (mut dark, mut lamps) = (MeshBuf::default(), MeshBuf::default());
        panes(&mut dark, &h.panes, false);
        panes(&mut lamps, &h.panes, true);
        assert_eq!(lamps.vertices.len(), lit * 4);
        assert_eq!(dark.vertices.len(), (h.panes.len() - lit) * 4);
        assert!(!dark.vertices.iter().any(is_light));
        assert!(lamps.vertices.iter().all(is_light));
        assert_eq!(lamps_material("x").emissive, [0.0; 3]);
        // By day a lit window is glass, as a dark one is; at night it's
        // warm light and no surface.
        let noon = PlotGlow {
            surface: 1.0,
            radiance: 0.0,
        };
        let night = PlotGlow {
            surface: 0.0,
            radiance: 1.6,
        };
        let white = palette::glow([1.0; 4], 1.0);
        let (paint, light) = drawn(white, lamp_glow(noon));
        assert_eq!((paint, light), (rgb(hex(palette::GLASS)), [0.0; 3]));
        let (paint, [r, _, b]) = drawn(white, lamp_glow(night));
        assert_eq!(paint, [0.0; 3]);
        assert!(r > b, "lamp-light is warm");
    }

    fn rgb([r, g, b, _]: [f32; 4]) -> [f32; 3] {
        [r, g, b]
    }

    /// What the engine draws of a vertex `colour` under an entity's
    /// `material` fields: the lit surface and the light given off (Flint
    /// ADRs 0076, 0090, 0096 and 0102). Fields left out are the defaults.
    fn drawn(colour: [f32; 4], material: [(&str, f32); 5]) -> ([f32; 3], [f32; 3]) {
        let field = |key: &str, default: f32| {
            material
                .iter()
                .find(|(k, _)| *k == key)
                .map_or(default, |&(_, v)| v)
        };
        let tint = [
            field("vertex_tint_r", 1.0),
            field("vertex_tint_g", 1.0),
            field("vertex_tint_b", 1.0),
        ];
        let light = 1.0 - colour[3];
        let surface = colour[3] + light * field("glow_surface", 0.0);
        let shine = light * field("emissive_scale", 1.0);
        let c = |i: usize| colour[i] * tint[i];
        (
            [c(0) * surface, c(1) * surface, c(2) * surface],
            [c(0) * shine, c(1) * shine, c(2) * shine],
        )
    }

    /// The lit panes, uploaded once and set by the hour, draw exactly as
    /// the pane rebuilt each hour before H23 did: a hue from glass to the
    /// lamp's, lit as `surface` of it and giving off `radiance × 1.1`.
    #[test]
    fn lit_windows_follow_the_day_as_a_rebuild_did() {
        let old_pane = |glow: PlotGlow| {
            let glass = hex(palette::GLASS);
            if glow.radiance <= 0.0 {
                return glass;
            }
            let total = glow.surface + glow.radiance;
            let hue = palette::mix(glass, hex(palette::LAMP), glow.radiance / total);
            palette::lit_glow(hue, glow.surface, glow.radiance * WINDOW_GLOW)
        };
        let unset = [("", 0.0); 5];
        let white = palette::glow([1.0; 4], 1.0);
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5);
        // Night, dawn and dusk, noon, evening, and between them.
        for (surface, radiance) in [
            (0.0, 1.6),
            (0.6, 0.25),
            (1.0, 0.0),
            (0.0, 0.5),
            (0.8, 0.125),
            (0.3, 0.925),
        ] {
            let glow = PlotGlow { surface, radiance };
            let (paint, light) = drawn(white, lamp_glow(glow));
            let (want_paint, want_light) = drawn(old_pane(glow), unset);
            assert!(
                close(paint, want_paint),
                "{glow:?}: paint {paint:?} vs {want_paint:?}"
            );
            assert!(
                close(light, want_light),
                "{glow:?}: light {light:?} vs {want_light:?}"
            );
        }
    }

    #[test]
    fn claims_grow_built_or_go() {
        let mut layer = Houses::default();
        let lots = lots();
        let mut ids: Vec<&String> = lots.keys().collect();
        ids.sort();
        let (old, new) = (ids[0].clone(), ids[1].clone());
        // The list at sign-in: up already.
        layer.sync(1, [&old], |_| false, lots);
        assert_eq!(layer.counts(), (1, 0));
        assert!(layer.dirty.contains(&lots[&old].sector));
        layer.dirty.clear();
        // The same version again changes nothing.
        layer.sync(1, [&old, &new], |_| true, lots);
        assert_eq!(layer.counts(), (1, 0));
        // A claim just made grows, then joins its sector.
        layer.sync(2, [&old, &new], |_| true, lots);
        assert_eq!(layer.counts(), (1, 1));
        assert!(layer.dirty.is_empty());
        layer.advance(GROW_SECS * 0.5, lots);
        assert_eq!(layer.counts(), (1, 1));
        assert!(!layer.growing_mesh(lots).0.indices.is_empty());
        layer.advance(GROW_SECS * 0.6, lots);
        assert_eq!(layer.counts(), (2, 0));
        assert!(layer.dirty.contains(&lots[&new].sector));
        // A release takes it down.
        layer.dirty.clear();
        layer.sync(3, [&new], |_| false, lots);
        assert_eq!(layer.counts(), (1, 0));
        assert!(layer.dirty.contains(&lots[&old].sector));
        // What's left in its sector is what the other claim alone builds.
        assert_eq!(layer.sector_mesh(lots[&old].sector).indices.len(), {
            let mut l = Houses::default();
            l.sync(1, [&new], |_| false, lots);
            l.sector_mesh(lots[&old].sector).indices.len()
        });
        // A new terrain builds everything again.
        layer.lots_changed();
        assert_eq!(layer.counts(), (0, 0));
        layer.sync(3, [&new], |_| true, lots);
        assert_eq!(
            layer.counts(),
            (0, 1),
            "after lots_changed, a sync always runs"
        );
    }

    /// V3.4: a building opens as the camera comes close, yours from
    /// further off, leaves its sector's mesh while it's open, and shuts and
    /// goes back as the camera leaves.
    #[test]
    fn buildings_open_as_the_camera_comes_close() {
        let lots = lots();
        let (id, lot) = first(Zone::Suburban);
        let fp = footprint(lot).unwrap();
        let mut layer = Houses::default();
        layer.sync(1, [id], |_| false, lots);
        layer.dirty.clear();
        let full = layer.sector_mesh(lot.sector).indices.len();
        assert!(full > 0);
        // World metres, `m` in front of its front wall, at eye height.
        let eye = |m: f32| {
            let p = fp.at(0.0, -fp.hy - metres(m));
            let [x, z] = to_world(p[0], p[1]);
            [x, 1.7, z]
        };
        let nobody = HashSet::new();
        let me: HashSet<String> = [id.clone()].into();
        // 15 m off: someone else's stays shut, yours opens.
        layer.cutaway(0.1, eye(15.0), &nobody);
        assert_eq!(layer.openness(id), 0.0);
        layer.cutaway(0.1, eye(15.0), &me);
        assert!(layer.openness(id) > 0.0);
        assert!(layer.dirty.contains(&lot.sector), "it leaves its sector");
        assert!(layer.sector_mesh(lot.sector).indices.is_empty());
        assert!(!layer.open_mesh(lots).0.indices.is_empty());
        for _ in 0..20 {
            layer.cutaway(0.1, eye(15.0), &me);
        }
        assert_eq!(layer.openness(id), 1.0);
        // 5 m off, anyone's opens.
        let mut theirs = Houses::default();
        theirs.sync(1, [id], |_| false, lots);
        theirs.cutaway(0.1, eye(5.0), &nobody);
        assert!(theirs.openness(id) > 0.0);
        // Walk away: it shuts, and goes back into its sector.
        layer.dirty.clear();
        for _ in 0..20 {
            layer.cutaway(0.1, eye(60.0), &me);
        }
        assert_eq!(layer.openness(id), 0.0);
        assert!(layer.open.is_empty());
        assert!(layer.dirty.contains(&lot.sector));
        assert_eq!(layer.sector_mesh(lot.sector).indices.len(), full);
        // From high over the city, nothing opens.
        let mut high = eye(0.0);
        high[1] = 200.0;
        layer.cutaway(0.1, high, &me);
        assert_eq!(layer.openness(id), 0.0);
    }

    /// A floor finished rises out of the sector's mesh, then goes back.
    #[test]
    fn a_finished_floor_rises_then_settles() {
        let lots = lots();
        let (id, lot) = first(Zone::Urban);
        let mut layer = Houses::default();
        layer.sync(1, [id], |_| false, lots);
        layer.set_floors(id, 1, true, lots);
        assert!(!layer.rising(id), "scaffolding stands in the sector");
        assert!(!layer.sector_mesh(lot.sector).indices.is_empty());
        layer.set_floors(id, 2, false, lots);
        assert!(layer.rising(id));
        assert!(layer.sector_mesh(lot.sector).indices.is_empty());
        assert!(!layer.growing_mesh(lots).0.indices.is_empty());
        layer.advance(GROW_SECS + 0.1, lots);
        assert!(!layer.rising(id));
        assert_eq!(layer.look(id).floors, 2);
        assert!(!layer.sector_mesh(lot.sector).indices.is_empty());
    }

    /// A station upstairs finds its building, and an open one shows the
    /// storeys up to it (V3.3)
    #[test]
    fn an_open_building_shows_up_to_its_highest_station() {
        let lots = lots();
        let (id, lot) = first(Zone::Urban);
        let fp = footprint(lot).unwrap();
        let mut layer = Houses::default();
        layer.sync(1, [id], |_| false, lots);
        layer.set_floors(id, 2, false, lots);
        assert_eq!(layer.shown.get(id).map_or(0, |s| s.top), 0);
        layer.open_dirty = false;
        layer.upstairs(&[(fp.at(0.0, 0.0), 2), (fp.at(0.0, fp.hy + 40.0), 1)]);
        assert_eq!(layer.shown[id].top, 2, "the one outside the walls counts for nothing");
        assert!(layer.open_dirty);
        layer.open_dirty = false;
        layer.upstairs(&[(fp.at(0.0, 0.0), 2), (fp.at(0.0, fp.hy + 40.0), 1)]);
        assert!(!layer.open_dirty, "the same again changes nothing");
        layer.upstairs(&[]);
        assert_eq!(layer.shown[id].top, 0);
    }

    /// The fires find their buildings, and a chimney stands over each.
    #[test]
    fn fires_indoors_get_chimneys() {
        let lots = lots();
        let (id, lot) = first(Zone::Rural);
        let fp = footprint(lot).unwrap();
        let mut layer = Houses::default();
        layer.sync(1, [id], |_| false, lots);
        let inside = fp.at(0.0, 0.0);
        let outside = fp.at(0.0, fp.hy + 40.0);
        layer.fires(&[inside, outside], lots);
        let chimneys: Vec<_> = layer.chimneys().collect();
        assert_eq!(chimneys.len(), 1);
        assert_eq!(chimneys[0].0, inside);
        layer.fires(&[], lots);
        assert_eq!(layer.chimneys().count(), 0);
    }

    #[test]
    fn a_dense_city_fits_the_budget_of_one_mesh_per_sector() {
        // Every lot claimed: the houses' triangles, and how many meshes.
        let mut layer = Houses::default();
        let lots = lots();
        layer.sync(1, lots.keys(), |_| false, lots);
        let sectors: BTreeSet<Sector> = layer.built.values().map(|up| up.sector).collect();
        let tris: usize = layer
            .built
            .values()
            .map(|up| up.house.body.indices.len() / 3)
            .sum();
        let panes: usize = layer.built.values().map(|up| up.house.panes.len()).sum();
        println!(
            "{} houses in {} sectors: {tris} triangles, {panes} windows",
            layer.built.len(),
            sectors.len()
        );
        assert!(sectors.len() <= Sector::all().count());
        // About 4800 houses, the windows' mesh well within u32 indices.
        assert!(panes * 4 < 1 << 22);
    }
}
