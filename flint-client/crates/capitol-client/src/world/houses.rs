//! Houses on claimed plots (P8.6b): the city fills in as players build.
//!
//! - **One house per claimed lot**, built from the lot as drawn (`Lot`'s
//!   inset outline) and its street edge. The footprint is a rectangle
//!   square to the street, set back from it and kept off the sides by the
//!   zone's margins. Where the lot is too small for the zone's smallest
//!   house, there's none.
//! - **By zone** ([`style`]): urban terraces fill their lots' street
//!   fronts, two or three storeys under slate or a flat roof with a
//!   cornice, most with a workshop at the back of the yard (#421); the
//!   suburbs are post-war houses, one storey or two, in pale render under
//!   tile, some with a garage; rural lots get a steep-roofed farmhouse with
//!   a chimney, set back from the lane on a drive, and often a barn behind
//!   it (the grass keeps off that [`Farmyard`], R6); the guild district builds tall
//!   stone workshops. Colours are the house tokens in `palette`.
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

use crate::coords::{layer, to_world, S};
use crate::daylight::PlotGlow;
use crate::world::geom::*;
use crate::world::mesh::{MeshBuf, RAY_TARGET, UP};
use crate::world::palette::{self, hex};
use crate::world::plots::Lot;
use crate::world::sector::Sector;
use capitol_protocol::rules::Zone;
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
        .chain(std::iter::once(GROWING_LAMPS.to_owned()))
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

/// How one zone builds. Lengths are metres.
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
    /// The chance of a garage (suburbs), a barn (fields) or a workshop at
    /// the back of the yard (urban), room allowing.
    outbuilding: f32,
}

fn style(zone: Zone) -> Style {
    match zone {
        Zone::Suburban => Style {
            setback: (1.8, 2.8),
            depth: (3.6, 4.6),
            width: (4.2, 5.4),
            // A bungalow on the odd lot: a sliver, or a shallow pie.
            min_width: 2.2,
            min_depth: 2.4,
            side: 0.25,
            back: 0.6,
            storeys: &[1, 1, 1, 2],
            storey: 2.5,
            pitch: (24.0, 34.0),
            roofs: &[Roof::Gable, Roof::Gable, Roof::Hip, Roof::FrontGable],
            eave: 0.25,
            walls: &palette::SUBURBAN_WALLS,
            roof_colours: &palette::SUBURBAN_ROOFS,
            window: 0.7,
            bay: 1.35,
            chimney: 0.45,
            outbuilding: 0.55,
        },
        // A farmstead stands back from the lane on its drive (R6, #289).
        Zone::Rural => Style {
            setback: (6.0, 11.0),
            depth: (3.6, 4.4),
            width: (5.4, 7.0),
            min_width: 3.4,
            min_depth: 3.0,
            side: 0.8,
            back: 1.5,
            storeys: &[1, 1, 2],
            storey: 2.3,
            pitch: (40.0, 48.0),
            roofs: &[Roof::Gable, Roof::Gable, Roof::FrontGable],
            eave: 0.3,
            walls: &palette::RURAL_WALLS,
            roof_colours: &palette::RURAL_ROOFS,
            window: 0.6,
            bay: 1.5,
            chimney: 0.85,
            outbuilding: 0.65,
        },
        Zone::GuildDistrict => Style {
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
            outbuilding: 0.0,
        },
        // Urban, and anything else that ever carries lots. Since #421 a lot
        // is about 8.5 m by 9.5: the house across its front, the yard behind.
        _ => Style {
            setback: (0.1, 0.3),
            depth: (2.8, 3.6),
            width: (9.0, 9.0),
            min_width: 1.8,
            min_depth: 2.0,
            side: 0.04,
            back: 0.1,
            storeys: &[2, 2, 3],
            storey: 2.3,
            pitch: (30.0, 38.0),
            roofs: &[Roof::Gable, Roof::Gable, Roof::Flat],
            eave: 0.0,
            walls: &palette::URBAN_WALLS,
            roof_colours: &palette::URBAN_ROOFS,
            window: 0.55,
            bay: 1.1,
            chimney: 0.35,
            outbuilding: 0.7,
        },
    }
}

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
    /// A garage's or a barn's wide door.
    Wide([f32; 4]),
}

/// Everything a house will be.
#[derive(Clone, Debug, PartialEq)]
struct Plan {
    blocks: Vec<Block>,
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

/// The plan for the house on `lot`, from its id, or `None` if the lot
/// has no room for the zone's smallest.
fn plan(id: &str, lot: &Lot) -> Option<Plan> {
    let st = style(lot.zone);
    let frame = Frame::of(lot)?;
    let mut rng = Seed::of(id);
    let (near, far) = frame.depth();
    let roof = rng.pick(st.roofs);
    let setback = rng.range(st.setback);
    let (want_depth, want_width) = (rng.range(st.depth), rng.range(st.width));
    // A farm's barn, or the workshop at the back of an urban yard (#421),
    // drawn first so the house leaves room behind for it: the gap to it,
    // its width and its depth.
    let workshop = lot.zone == Zone::Urban;
    let barn = (matches!(lot.zone, Zone::Rural | Zone::Urban) && rng.chance(st.outbuilding)).then(|| {
        if workshop {
            (rng.range((1.6, 2.6)), rng.range((3.6, 5.6)), rng.range((2.4, 3.2)))
        } else {
            (rng.range((2.5, 4.5)), rng.range((4.0, 5.2)), rng.range((4.6, 6.0)))
        }
    });
    let behind = barn.map_or(0.0, |(gap, _, bd)| gap + bd);
    // What stands out past the walls: the eaves, or a flat roof's cornice.
    let flat = roof == Roof::Flat;
    let eaves = if flat { 0.0 } else { st.eave };
    let fit = |eave: f32| {
        let overhang = if flat { CORNICE } else { eave };
        let margin = st.side + overhang;
        // A shallow lot takes a shorter front garden before a smaller house.
        let lot_depth = (far - near) as f32 * S;
        let setback = setback
            .min(lot_depth - st.back - overhang - want_depth.max(st.min_depth) - behind)
            .max(overhang + 0.05);
        // On a wedge (a pie round a bulb) the street end is narrow, so
        // step back to where it widens.
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
            (width >= st.min_width).then_some((eave, overhang, t0, depth, width, free))
        })
    };
    // Too tight with eaves, the roof stops at the walls.
    let (eave, overhang, t0, depth, width, free) = fit(eaves).or_else(|| fit(0.0))?;
    let margin = st.side + overhang;
    // Room left at the side for a garage?
    let garage = 2.4;
    let with_garage = lot.zone == Zone::Suburban
        && rng.chance(st.outbuilding)
        && free - width >= garage + 0.3 + st.side;
    let along = rng.unit();
    let main_along = if with_garage {
        // The house to one side, the garage to the other.
        if along < 0.5 {
            0.0
        } else {
            1.0
        }
    } else {
        0.35 + 0.3 * along
    };
    let house_rect = frame.place(t0, (width, depth), margin, overhang, main_along)?;
    let wall = palette::shade(hex(rng.pick(st.walls)), rng.range((0.93, 1.05)));
    let roof_colour = palette::shade(hex(rng.pick(st.roof_colours)), rng.range((0.9, 1.08)));
    let pitch = rng.range(st.pitch);
    let storeys = rng.pick(st.storeys);
    let chimney = (roof != Roof::Flat && rng.chance(st.chimney)).then(|| {
        if rng.chance(0.5) {
            -0.62
        } else {
            0.62
        }
    });
    let door_at = rng.range((0.25, 0.75));
    let door = Door::Front(door_at, hex(rng.pick(&palette::DOORS)));
    let mut blocks = vec![Block {
        rect: house_rect,
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
    }];
    if with_garage {
        let gap = metres(0.15);
        let sign = if main_along == 0.0 { 1.0 } else { -1.0 };
        let gw = metres(garage);
        let gd = house_rect.hy * 2.0 * 0.85;
        // Its front a little behind the house's.
        let back = metres(0.6);
        let c = add(
            house_rect.c,
            add(
                scale(house_rect.ax, sign * (house_rect.hx + gap + gw * 0.5)),
                scale(house_rect.ay, back + gd * 0.5 - house_rect.hy),
            ),
        );
        let rect = Rect {
            c,
            ax: house_rect.ax,
            ay: house_rect.ay,
            hx: gw * 0.5,
            hy: gd * 0.5,
        };
        if inside(&frame, &rect, st.side) {
            blocks.push(Block {
                rect,
                storeys: 1,
                storey: 2.3,
                roof: Roof::Flat,
                pitch: 0.0,
                eave: 0.0,
                wall: palette::shade(wall, 0.94),
                roof_colour,
                chimney: None,
                door: Door::Wide(hex(palette::GARAGE_DOOR)),
                window: st.window,
                bay: 99.0,
            });
        }
    }
    if let Some((gap, bw, bd)) = barn {
        let t1 = t0 + metres(depth + gap);
        let fits = (far - t1) as f32 * S - st.back >= bd;
        let along = rng.unit();
        if workshop {
            // A low shed of the house's brick, against the back line, under
            // a flat roof, with wide timber doors onto the yard.
            if let Some(rect) = fits.then(|| frame.place(t1, (bw, bd), st.side + 0.2, 0.0, along)).flatten() {
                blocks.push(Block {
                    rect,
                    storeys: 1,
                    storey: rng.range((2.5, 3.0)),
                    roof: Roof::Flat,
                    pitch: 0.0,
                    eave: 0.0,
                    wall: palette::shade(wall, 0.88),
                    roof_colour,
                    chimney: None,
                    door: Door::Wide(hex(palette::BARN_DOOR)),
                    window: 0.5,
                    bay: 99.0,
                });
            }
        } else if let Some(rect) = fits
            .then(|| frame.place(t1, (bw, bd), st.side + 0.3, 0.25, along))
            .flatten()
        {
            blocks.push(Block {
                rect,
                storeys: 1,
                storey: rng.range((3.0, 3.6)),
                roof: Roof::FrontGable,
                pitch: rng.range((34.0, 42.0)),
                eave: 0.25,
                wall: palette::shade(hex(rng.pick(&palette::BARNS)), rng.range((0.92, 1.05))),
                roof_colour: hex(palette::BARN_ROOF),
                chimney: None,
                door: Door::Wide(hex(palette::BARN_DOOR)),
                window: 0.5,
                bay: 99.0,
            });
        }
    }
    // The drive runs straight in from the lane to the front door.
    let drive = (lot.zone == Zone::Rural).then(|| {
        let door = house_rect.at(house_rect.hx * (2.0 * door_at as f64 - 1.0), -house_rect.hy);
        let s = dot(sub(door, frame.o), frame.u);
        [frame.at(s, -DRIVE_OUT), door]
    });
    let track = drive.and_then(|[_, door]| {
        let (s, t1) = (dot(sub(door, frame.o), frame.u), dot(sub(door, frame.o), frame.v));
        let t0 = near + TRACK_FROM;
        (t1 > t0).then(|| Rect {
            c: frame.at(s, (t0 + t1) * 0.5),
            ax: frame.u,
            ay: frame.v,
            hx: TRACK_HALF,
            hy: (t1 - t0) * 0.5,
        })
    });
    Some(Plan {
        blocks,
        drive,
        track,
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

/// Builds `plan` as it stands at `stage`.
fn raise(plan: &Plan, stage: Stage) -> House {
    let mut h = House::default();
    let mut lights = Seed(plan.lights);
    for b in &plan.blocks {
        block(&mut h, b, stage, &mut lights);
    }
    if let Some(track) = &plan.track {
        top(&mut h.body, track, FLOOR + TRACK_LIFT, hex(palette::FARM_TRACK));
    }
    h
}

fn block(h: &mut House, b: &Block, stage: Stage, lights: &mut Seed) {
    let m = &mut h.body;
    let r = &b.rect;
    let height = b.storeys as f32 * b.storey;
    let top_y = FLOOR + height;
    // The frame: a post at each corner, and once they're up, the plates.
    if stage != Stage::BUILT && stage.frame > 0.0 {
        let timber = hex(palette::TIMBER);
        let post = metres(POST);
        let inner = Rect {
            hx: r.hx - post * 0.5,
            hy: r.hy - post * 0.5,
            ..*r
        };
        let y = FLOOR + height * stage.frame;
        for p in inner.corners() {
            let foot = Rect {
                c: p,
                hx: post * 0.5,
                hy: post * 0.5,
                ..*r
            };
            cuboid(m, &foot, FOOT, y, timber);
        }
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
    // The walls, rising.
    let wall_top = FLOOR + height * stage.walls;
    if stage.walls > 0.0 {
        walls(m, r, FOOT, wall_top, b.wall);
    }
    // Doors and windows, once the wall has passed them.
    let c = r.corners();
    for (i, &(a, z)) in [(c[0], c[1]), (c[1], c[2]), (c[2], c[3]), (c[3], c[0])]
        .iter()
        .enumerate()
    {
        let front = i == 0;
        openings(h, b, a, z, front, wall_top, lights);
    }
    let m = &mut h.body;
    if stage.roof <= 0.0 {
        return;
    }
    let eave = metres(b.eave);
    match b.roof {
        Roof::Flat => {
            let cornice = r.grown(metres(CORNICE));
            let stone = palette::shade(b.wall, 0.82);
            cuboid(m, &cornice, top_y - 0.22 * stage.roof, top_y + 0.06, stone);
            top(m, r, top_y + 0.065, b.roof_colour);
        }
        Roof::Gable | Roof::FrontGable | Roof::Hip => {
            let along = if b.roof == Roof::FrontGable {
                r.turned()
            } else {
                *r
            };
            let rise = along.hy as f32 * S * b.pitch.to_radians().tan() * stage.roof;
            if rise < 0.05 {
                return;
            }
            if b.roof == Roof::Hip {
                hip(m, &along, top_y, rise, eave, b.roof_colour);
            } else {
                gable(m, &along, top_y, rise, eave, b.wall, b.roof_colour);
            }
            if let (Some(k), true) = (b.chimney, stage.roof >= 1.0) {
                // Up through the back slope, near one end of the ridge.
                let ridge = if b.roof == Roof::Hip && along.hx < along.hy {
                    along.turned()
                } else {
                    along
                };
                // In scale with the house: a terrace's is slighter.
                let half = metres(0.28).min(ridge.hx.min(ridge.hy) * 0.16);
                let stack = Rect {
                    c: ridge.at(ridge.hx * k, ridge.hy * 0.3),
                    hx: half,
                    hy: half,
                    ..ridge
                };
                let brick = hex(palette::CHIMNEY);
                cuboid(m, &stack, top_y, top_y + rise + 0.55, brick);
            }
        }
    }
}

/// The doors and windows on the wall from `a` to `z` (server), as far up
/// as `wall_top`.
fn openings(h: &mut House, b: &Block, a: P, z: P, front: bool, wall_top: f32, lights: &mut Seed) {
    let length = dist(a, z) as f32 * S;
    let dir = unit(sub(z, a));
    let mid = scale(add(a, z), 0.5);
    let out = level(sub(mid, b.rect.c));
    let off = scale([out[0] as f64, out[2] as f64], metres(PROUD));
    let at = |s: f32| add(add(a, scale(dir, metres(s))), off);
    let quad = |s0: f32, s1: f32, y0: f32, y1: f32| {
        [
            lift(at(s0), y0),
            lift(at(s1), y0),
            lift(at(s1), y1),
            lift(at(s0), y1),
        ]
    };
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
        }
    }
    if matches!(b.door, Door::Wide(_)) {
        return;
    }
    let count = ((length - 0.5) / b.bay).floor().max(0.0) as usize;
    if count == 0 {
        return;
    }
    let step = length / count as f32;
    for storey in 0..b.storeys {
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
pub fn house(id: &str, lot: &Lot, progress: f32) -> Option<House> {
    let plan = plan(id, lot)?;
    Some(raise(&plan, Stage::at(progress)))
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

/// Every house in the city, and what needs building again.
#[derive(Default)]
pub struct Houses {
    /// The claims last followed (`sync`), by the version they came with.
    version: Option<u64>,
    claimed: HashSet<String>,
    /// The houses that are up, by lot id, with their sector.
    built: HashMap<String, (Sector, House)>,
    /// The houses going up, by lot id: when they started.
    growing: HashMap<String, f32>,
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
            if let Some((sector, _)) = self.built.remove(gone) {
                self.dirty.insert(sector);
            }
            self.growing.remove(gone);
        }
        for new in now.difference(&self.claimed) {
            let Some(lot) = lots.get(new) else {
                continue;
            };
            if fresh(new) {
                self.growing.insert(new.clone(), self.time);
            } else if let Some(h) = house(new, lot, 1.0) {
                self.dirty.insert(lot.sector);
                self.built.insert(new.clone(), (lot.sector, h));
            }
        }
        self.claimed = now;
        let (up, growing) = self.counts();
        tracing::debug!("houses: {up} up, {growing} growing");
    }

    /// Moves time on by `dt`: a house that has finished growing joins its
    /// sector.
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
            let Some(lot) = lots.get(&id) else {
                continue;
            };
            if let Some(h) = house(&id, lot, 1.0) {
                self.dirty.insert(lot.sector);
                self.built.insert(id, (lot.sector, h));
            }
        }
    }

    /// How many houses are up, and how many going up.
    pub fn counts(&self) -> (usize, usize) {
        (self.built.len(), self.growing.len())
    }

    /// One sector's houses as one mesh.
    fn sector_mesh(&self, sector: Sector) -> MeshBuf {
        let mut m = MeshBuf::default();
        for (s, h) in self.built.values() {
            if *s == sector {
                m.append(h.body.clone());
            }
        }
        m
    }

    /// The houses going up, as they stand now, with their dark windows;
    /// and their lit windows apart.
    fn growing_mesh(&self, lots: &HashMap<String, Lot>) -> (MeshBuf, MeshBuf) {
        let (mut m, mut lamps) = (MeshBuf::default(), MeshBuf::default());
        for (id, &t0) in &self.growing {
            let Some(lot) = lots.get(id) else {
                continue;
            };
            let p = ((self.time - t0) / GROW_SECS).clamp(0.0, 0.999);
            if let Some(h) = house(id, lot, p) {
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
            for (s, h) in self.built.values() {
                if quarter(*s) == q {
                    panes(&mut dark, &h.panes, false);
                    panes(&mut lamps, &h.panes, true);
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
        if !self.growing.is_empty() || self.growing_drawn {
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

    #[test]
    fn a_house_stays_on_its_lot() {
        // Every vertex, eaves and all, inside the lot as drawn.
        for (id, lot) in lots() {
            let Some(h) = house(id, lot, 1.0) else {
                continue;
            };
            let n = lot.outline.len();
            for p in vertices(&h) {
                let q = to_server(p[0], p[2]);
                for i in 0..n {
                    let (a, b) = (lot.outline[i], lot.outline[(i + 1) % n]);
                    let inward = dot(unit(left(sub(b, a))), sub(q, a));
                    assert!(inward >= -0.05, "{id}: {q:?} is {inward} outside");
                }
            }
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
            back += ((t(door) - frame.depth().0) as f32 * S >= 5.0) as usize;
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
        // The barn's chance is 0.65.
        farmsteads_stand_back_on_their_drives(&farm_lots(), 0.9, 0.5);
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
                for p in [0.2, 0.5, 0.8, 1.0] {
                    let Some(h) = house(id, lot, p) else {
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

    #[test]
    fn a_dense_city_fits_the_budget_of_one_mesh_per_sector() {
        // Every lot claimed: the houses' triangles, and how many meshes.
        let mut layer = Houses::default();
        let lots = lots();
        layer.sync(1, lots.keys(), |_| false, lots);
        let sectors: BTreeSet<Sector> = layer.built.values().map(|(s, _)| *s).collect();
        let tris: usize = layer
            .built
            .values()
            .map(|(_, h)| h.body.indices.len() / 3)
            .sum();
        let panes: usize = layer.built.values().map(|(_, h)| h.panes.len()).sum();
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
