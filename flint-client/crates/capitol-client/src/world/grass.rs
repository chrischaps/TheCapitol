//! Grass fields (P8.6c): Flint's compute grass (H21, Flint ADR 0089) over
//! the rural ring and the wilderness, so past the suburb the city gives way
//! to meadow. The engine grows blades wherever one R8 mask over the whole
//! world says to, and only near the camera, so the cost follows the view,
//! not the fields.
//!
//! The mask is painted here, on the CPU, from the terrain:
//!
//! - **the rings**: none inside the suburb, easing in across its outer edge,
//!   full through the rural ring and on into the wilds, thinning out at the
//!   world's edge ([`GRASS_EDGE`]);
//! - **the land** (W9): in the wilds the grass grows on the hill meadows,
//!   thinning off the steeps and the high scree, and out of the beds the
//!   water cut (`land::Sampled::cut`). The blades stand on the land, on a
//!   grid of [`HEIGHTS`] a side;
//! - **parks** (#129): the suburb's parks are mown lawns, shorter and more
//!   even than the meadows, easing in from their edges where the lots and
//!   the streets stand;
//! - **meadows**: a slow value noise (the ground's own, so the fields and
//!   its mottling share a grain) thins the grass in broad patches, so it
//!   reads as fields rather than as a carpet;
//! - **farms** (R6): a farm is pasture, so the grass stays on it, standing
//!   on its pad a step over the land, right up to its hedge bank. Only its
//!   yard is bare: round the farmhouse and the barn, and the drive in from
//!   the lane, where `houses` would build them, claimed or not;
//! - **cleared**: nothing on roads and a verge beside them, on every other
//!   lot (the houses stand there, P8.6b), in the canals and on their kerbs, and in a
//!   small clearing round each gatherable node, so its tuft still reads as
//!   a thing to take. Every clearing has a soft rim, which `scale_by_mask`
//!   turns into the field thinning toward the edge rather than stopping.
//!
//! The terrain part is painted when the roads, the lots, the parks or the
//! rings change, and handed to the engine whole. Nodes arrive as you walk
//! (the server lists those within 80 m), so when one comes or goes only the
//! texels round it are worked out again, from the terrain's part and every
//! node near, and written into the engine's mask in place (Flint ADR 0093,
//! #135): about 17 texels square, 300 bytes against the whole mask's 4 MB.
//!
//! The colours are `palette` tokens. The grass is lit by the scene's `sun`
//! like everything else, so it warms at dusk and goes blue under the moon
//! with no help (`daylight.rs`). The wind is the scene's: the ocean's
//! direction and speed, so the grass and the canals move together.

use crate::coords::{layer, to_world, S};
use crate::world::geom::{distance_to_segment, radius, P};
use crate::world::ground::value_noise;
use crate::world::houses;
use crate::world::land;
use crate::world::palette::{hex, GRASS_DRY, GRASS_ROOT, GRASS_TIP};
use crate::world::plots;
use crate::world::water;
use capitol_protocol::rules::{Zone, CENTER_X, CENTER_Y, SUBURBAN_OUTER_RADIUS, WILDERNESS_OUTER_RADIUS};
use capitol_protocol::terrain::{TerrainPark, TerrainPlot, TerrainResponse};
use flint_ecs::FlintWorld;
use flint_player::{ExtensionGpu, GrassConfig, GrassGround, GrassHeight, GroundField};
use std::sync::Arc;
use std::time::Instant;

/// The mask's texel, in server units: about 0.39 m, finer than the
/// narrowest trail (0.6 m) and its verge. It's fixed, and the mask's size
/// follows the world's (PV2, #458): 87 texels to 340 units is the 3480
/// over 1360 m the 13600 world has had since it grew (#308; 3072 over
/// 1200 m before).
const MASK_TEXEL: f64 = 340.0 / 87.0;

/// Texels along each side of the mask: 3480 for the 13600 world, and 4248
/// for a 16600 one.
pub const MASK_SIZE: usize = texels(2.0 * GRASS_EDGE, MASK_TEXEL);

/// How far the grass reaches from the centre, in server units: the world's
/// edge. The mask is the square round it.
const GRASS_EDGE: f64 = WILDERNESS_OUTER_RADIUS;

/// How many of `texel` span `side`, to the nearest.
const fn texels(side: f64, texel: f64) -> usize {
    (side / texel).round() as usize
}

/// The spacing of the heights the blades stand on, in server units: about
/// 0.59 m, fixed like [`MASK_TEXEL`] (58 to 340 units is the 13600 world's
/// 2320, #308; 2048 over 1200 m before).
const HEIGHT_STEP: f64 = 340.0 / 58.0;

/// Heights along each side of the grid the blades stand on, over the
/// mask's square: 2320 for the 13600 world. The engine stands each blade on the
/// blend of the four round it (Flint ADR 0098, #228), which keeps 99% of
/// blades within 2 cm of the land and the worst, on the rough ground high
/// in the ranges, within 0.22 m (`blades_stand_on_the_land`). A grid at
/// the land's own metre would cost a third as much (14 ms against 40, once
/// a terrain), but its points fall between the land's, which smooths the
/// sharpest ground: 5 cm and 0.48 m.
pub const HEIGHTS: usize = texels(2.0 * GRASS_EDGE, HEIGHT_STEP);

/// Where the grass gives way in the wilds: the slopes (rise over run) over
/// which it thins out, the heights (m) over which the scree takes it, and
/// the depth (m) of a water's cut over which its bed is bare: since W10
/// the water lies in the beds, so even a shallow one keeps it off.
const STEEP: (f64, f64) = (0.45, 0.8);
const HIGH: (f64, f64) = (22.0, 38.0);
const WET: (f64, f64) = (0.002, 0.02);
/// How wet (m, `land::Sampled::wet`) the ground must be to be bare: the
/// shallows, where the land already lay under the water and the carve cut
/// nothing (a pit lake's floor), and a margin along each bank.
const SODDEN: (f64, f64) = (0.15, 0.3);

/// Where the grass starts easing in, and where it's full, in server units
/// from the centre: across the suburb's outer edge (3000).
const EASE_FROM: f64 = SUBURBAN_OUTER_RADIUS - 200.0;
const EASE_TO: f64 = SUBURBAN_OUTER_RADIUS + 150.0;
/// How far inside and past the world's edge the grass thins to nothing.
const EDGE_IN: f64 = 150.0;
const EDGE_OUT: f64 = 60.0;

/// The meadows' noise: its wavelength in metres, and the share of the
/// field left in the thinnest patch.
const MEADOW_SCALE: f32 = 26.0;
const MEADOW_FLOOR: f32 = 0.35;

/// A park's lawn: its height and fill against a full meadow's, with a
/// little of the meadow's mottling left in, and how far in from its edge
/// it's bare, then soft.
const PARK_LAWN: f32 = 0.55;
const PARK_MOTTLE: f32 = 0.2;
const PARK_EDGE: f64 = 2.0;
const PARK_SOFT: f64 = 7.0;

/// Server units of bare verge beside every road, past its edge, and of soft
/// rim past that, where the field thins in.
const VERGE: f64 = 5.0;
const ROAD_SOFT: f64 = 10.0;
/// Past a lot's edge (its curb): bare, then soft.
const LOT_MARGIN: f64 = 2.0;
const LOT_SOFT: f64 = 8.0;
/// The blades on a farm (R6) stand on its pad, a step over the land at
/// `layer::PLOT`: every height cell at least this far in from the farm's
/// edge. The pad starts 4 in (`plots.rs`' inset), and a blade stands on the
/// blend of the four cells round it, which reach about 8.3 units (a cell is
/// about 5.9). So no blade off the pad ever stands on a lifted cell, and
/// floats; those just inside it stand part way down the step, sunk in the
/// pad and its hedge bank, and read as the headland's shorter grass. That
/// leaves the pasture growing right up to the hedge, with no bare rim:
/// one 14 in bared a tenth of the ring round about 120 farms.
const PAD_IN: f64 = 13.0;
/// Round a farmstead's buildings: bare, then soft.
const YARD: f64 = 10.0;
const YARD_SOFT: f64 = 8.0;
/// A farm's drive: bare each side of its line, then soft. Narrower than a
/// lane's 8 and its verges.
const DRIVE_HALF: f64 = 3.0;
const DRIVE_SOFT: f64 = 6.0;
/// Past a canal's kerb.
const WATER_SOFT: f64 = 8.0;
/// A tuft's clearing: bare out to this radius, then soft. A tuft and its
/// mound are about 0.5 m across (`nodes::models`), so this leaves a ring of
/// short ground round it. Every other kind has its own
/// (`nodes::models::Kind::clearing`, E2.5).
pub const NODE_CLEAR: f64 = 16.0;
pub const NODE_SOFT: f64 = 14.0;

fn smoothstep(lo: f64, hi: f64, x: f64) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

/// The grass's mask over the world square, `size` texels a side, row by
/// row toward server +y (world +Z).
#[derive(Clone)]
pub struct Mask {
    pub texels: Vec<u8>,
    pub size: usize,
    /// Server (x, y) of the square's minimum corner, and its side.
    corner: P,
    side: f64,
}

impl Mask {
    /// The server point at the centre of texel (i, j).
    fn at(&self, i: usize, j: usize) -> P {
        let cell = self.side / self.size as f64;
        [
            self.corner[0] + (i as f64 + 0.5) * cell,
            self.corner[1] + (j as f64 + 0.5) * cell,
        ]
    }

    /// The texel holding server point `p`, unclamped.
    fn texel_of(&self, p: P) -> [i64; 2] {
        let cell = self.side / self.size as f64;
        [
            ((p[0] - self.corner[0]) / cell).floor() as i64,
            ((p[1] - self.corner[1]) / cell).floor() as i64,
        ]
    }

    /// The value at server point `p`, 0 to 255 (0 off the mask).
    #[cfg(test)]
    fn value_at(&self, p: P) -> u8 {
        let [i, j] = self.texel_of(p);
        let n = self.size as i64;
        if !(0..n).contains(&i) || !(0..n).contains(&j) {
            return 0;
        }
        self.texels[j as usize * self.size + i as usize]
    }

    /// The whole mask as a [`Window`].
    fn all(&self) -> Window {
        Window {
            i0: 0,
            j0: 0,
            w: self.size,
            h: self.size,
        }
    }

    /// The texels within `reach` of the box `lo`..`hi` (server), clipped to
    /// the mask; `None` when that's off it.
    fn window_near(&self, lo: P, hi: P, reach: f64) -> Option<Window> {
        let n = self.size as i64;
        let [i0, j0] = self.texel_of([lo[0] - reach, lo[1] - reach]);
        let [i1, j1] = self.texel_of([hi[0] + reach, hi[1] + reach]);
        let (i0, j0, i1, j1) = (i0.max(0), j0.max(0), i1.min(n - 1), j1.min(n - 1));
        (i0 <= i1 && j0 <= j1).then(|| Window {
            i0: i0 as usize,
            j0: j0 as usize,
            w: (i1 - i0 + 1) as usize,
            h: (j1 - j0 + 1) as usize,
        })
    }

    /// The texels of `win`, row by row.
    fn cut(&self, win: Window) -> Vec<u8> {
        (win.j0..win.j0 + win.h)
            .flat_map(|j| {
                let row = j * self.size + win.i0;
                self.texels[row..row + win.w].iter().copied()
            })
            .collect()
    }

    /// Lowers each texel within `reach` of the box `lo`..`hi` (server) to
    /// `clear(p)` of itself, `clear` running 0 (bare) to 1 (untouched).
    fn clear_near(&mut self, lo: P, hi: P, reach: f64, clear: impl Fn(P) -> f32) {
        let mut texels = std::mem::take(&mut self.texels);
        self.clear_near_in(&mut texels, self.all(), lo, hi, reach, clear);
        self.texels = texels;
    }

    /// As [`clear_near`](Self::clear_near), on `texels`, which hold this
    /// mask's window `win` row by row. Texels outside `win` are left be.
    fn clear_near_in(
        &self,
        texels: &mut [u8],
        win: Window,
        lo: P,
        hi: P,
        reach: f64,
        clear: impl Fn(P) -> f32,
    ) {
        let Some(near) = self.window_near(lo, hi, reach) else {
            return;
        };
        for j in near.j0.max(win.j0)..(near.j0 + near.h).min(win.j0 + win.h) {
            for i in near.i0.max(win.i0)..(near.i0 + near.w).min(win.i0 + win.w) {
                let k = (j - win.j0) * win.w + (i - win.i0);
                if texels[k] == 0 {
                    continue;
                }
                let keep = clear(self.at(i, j));
                if keep < 1.0 {
                    texels[k] = (texels[k] as f32 * keep).round() as u8;
                }
            }
        }
    }

    /// Half a texel's diagonal, in server units. Every bare edge is pushed
    /// out by this, so a texel is bare if any part of it touches the shape,
    /// not only its centre: a lot's corner or a kerb never sits in a texel
    /// that still grows a blade.
    fn slack(&self) -> f64 {
        self.side / self.size as f64 * std::f64::consts::FRAC_1_SQRT_2
    }

    /// Clears a road from `a` to `b`, `half` wide each side of its line.
    fn clear_segment(&mut self, a: P, b: P, half: f64, soft: f64) {
        let half = half + self.slack();
        let lo = [a[0].min(b[0]), a[1].min(b[1])];
        let hi = [a[0].max(b[0]), a[1].max(b[1])];
        self.clear_near(lo, hi, half + soft, |p| {
            smoothstep(half, half + soft, distance_to_segment(p, a, b))
        });
    }

    /// Clears the convex or concave polygon `corners`, and `margin` past it.
    fn clear_polygon(&mut self, corners: &[P], margin: f64, soft: f64) {
        let margin = margin + self.slack();
        let lo = corners
            .iter()
            .fold([f64::MAX; 2], |m, c| [m[0].min(c[0]), m[1].min(c[1])]);
        let hi = corners
            .iter()
            .fold([f64::MIN; 2], |m, c| [m[0].max(c[0]), m[1].max(c[1])]);
        self.clear_near(lo, hi, margin + soft, |p| {
            if inside(p, corners) {
                return 0.0;
            }
            let d = (0..corners.len())
                .map(|k| distance_to_segment(p, corners[k], corners[(k + 1) % corners.len()]))
                .fold(f64::MAX, f64::min);
            smoothstep(margin, margin + soft, d)
        });
    }

    /// Clears every texel whose centre is on the water inside `rings`
    /// (filled even-odd, so an island keeps its grass), row by row.
    fn clear_rings(&mut self, rings: &[Vec<(f64, f64)>]) {
        let n = self.size as i64;
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for &(_, y) in rings.iter().flatten() {
            lo = lo.min(y);
            hi = hi.max(y);
        }
        if lo > hi {
            return;
        }
        let (j0, j1) = (self.texel_of([0.0, lo])[1].max(0), self.texel_of([0.0, hi])[1].min(n - 1));
        let mut xs = Vec::new();
        for j in j0..=j1 {
            let y = self.at(0, j as usize)[1];
            xs.clear();
            for ring in rings {
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    if (a.1 > y) != (b.1 > y) {
                        xs.push(a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1));
                    }
                }
            }
            xs.sort_by(f64::total_cmp);
            for span in xs.as_chunks::<2>().0 {
                let i0 = self.texel_of([span[0], y])[0].max(0);
                let i1 = self.texel_of([span[1], y])[0].min(n - 1);
                for i in i0..=i1 {
                    let x = self.at(i as usize, 0)[0];
                    if (span[0]..=span[1]).contains(&x) {
                        self.texels[(j * n + i) as usize] = 0;
                    }
                }
            }
        }
    }

    /// The texels the clearing of node `n` can touch.
    fn node_window(&self, n: Node) -> Option<Window> {
        let c = node_at(n);
        self.window_near(c, c, n[2] as f64 + self.slack() + n[3] as f64)
    }

    /// Clears a tuft's disc round `c`.
    #[cfg(test)]
    fn clear_node(&mut self, c: P) {
        let mut texels = std::mem::take(&mut self.texels);
        let n = [c[0] as i64, c[1] as i64, NODE_CLEAR as i64, NODE_SOFT as i64];
        self.clear_node_in(&mut texels, self.all(), n);
        self.texels = texels;
    }

    /// Clears node `n`'s disc from `texels`, this mask's window `win`.
    fn clear_node_in(&self, texels: &mut [u8], win: Window, n: Node) {
        let c = node_at(n);
        let (bare, soft) = (n[2] as f64 + self.slack(), n[3] as f64);
        self.clear_near_in(texels, win, c, c, bare + soft, |p| {
            smoothstep(bare, bare + soft, (p[0] - c[0]).hypot(p[1] - c[1]))
        });
    }

    /// The world rectangle this mask covers, as Flint's grass takes it:
    /// its minimum corner (X, Z) and its size in metres.
    fn world_rect(&self) -> ([f32; 2], [f32; 2]) {
        let side = (self.side as f32) * S;
        (to_world(self.corner[0], self.corner[1]), [side, side])
    }
}

/// A rectangle of a mask's texels: its first column and row, and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    i0: usize,
    j0: usize,
    w: usize,
    h: usize,
}

/// A node's clearing, in server units: where it stands, then how far it's
/// bare and how far soft past that. Whole units, so the sets compare.
type Node = [i64; 4];

fn node_at([x, y, ..]: Node) -> P {
    [x as f64, y as f64]
}

/// The whole mask the engine is given: the terrain's part, `base`, with a
/// clearing round each of `nodes`.
fn with_nodes(base: &Mask, nodes: &[Node]) -> Mask {
    let mut mask = base.clone();
    let mut texels = std::mem::take(&mut mask.texels);
    for &n in nodes {
        mask.clear_node_in(&mut texels, mask.all(), n);
    }
    mask.texels = texels;
    mask
}

/// What to write into the engine's mask when the nodes seen go from `old`
/// to `new` (both sorted): for each node come or gone, the texels round it,
/// worked out again from `base` and the nodes of `new` near it. Each window
/// comes out as [`with_nodes`] would paint it, so the mask stays exactly
/// that whatever order the nodes came in.
fn patches(base: &Mask, old: &[Node], new: &[Node]) -> Vec<(Window, Vec<u8>)> {
    let gone = old.iter().filter(|n| new.binary_search(n).is_err());
    let came = new.iter().filter(|n| old.binary_search(n).is_err());
    gone.chain(came)
        .filter_map(|&n| base.node_window(n))
        .map(|win| {
            let mut texels = base.cut(win);
            for &n in new {
                base.clear_node_in(&mut texels, win, n);
            }
            (win, texels)
        })
        .collect()
}

/// How far `p` is from the nearest edge of the polygon `poly`.
fn edge_distance(p: P, poly: &[P]) -> f64 {
    (0..poly.len())
        .map(|k| distance_to_segment(p, poly[k], poly[(k + 1) % poly.len()]))
        .fold(f64::MAX, f64::min)
}

/// Whether `plot` is a farm, whose pasture grows on it (R6).
fn is_farm(plot: &TerrainPlot) -> bool {
    Zone::from_str(&plot.zone) == Some(Zone::Rural)
}

/// Every farm's outline, server units: the hull of its corners, which
/// needn't come in order round it (as `plots` draws it).
fn farms(terrain: &TerrainResponse) -> Vec<Vec<P>> {
    terrain
        .terrain_plots
        .iter()
        .filter(|p| is_farm(p))
        .map(|p| plots::hull(&p.corners.map(|(x, y)| [x, y])))
        .collect()
}

/// The yard of every farm with room for a farmstead, in the order of
/// their ids.
fn farmyards(terrain: &TerrainResponse) -> Vec<houses::Farmyard> {
    let plots: Vec<TerrainPlot> = terrain.terrain_plots.iter().filter(|p| is_farm(p)).cloned().collect();
    let lots = plots::build(&plots).lots;
    let mut ids: Vec<&String> = lots.keys().collect();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| houses::farmyard(id, &lots[id]))
        .collect()
}

/// Stands the blades on each farm in `farms` on its pad: every one of
/// `heights` (the grid over `mask`'s square, [`HEIGHTS`] a side) whose
/// cell's middle is [`PAD_IN`] inside a farm is raised to `layer::PLOT`.
fn lift_onto_farms(heights: &mut [f32], mask: &Mask, farms: &[Vec<P>]) {
    let cell = mask.side / HEIGHTS as f64;
    let index = |v: f64, o: f64| ((v - o) / cell - 0.5).floor() as i64;
    let n = HEIGHTS as i64;
    for farm in farms {
        let (lo, hi) = farm.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), c| {
            ([lo[0].min(c[0]), lo[1].min(c[1])], [hi[0].max(c[0]), hi[1].max(c[1])])
        });
        let (i0, i1) = (index(lo[0], mask.corner[0]).max(0), (index(hi[0], mask.corner[0]) + 1).min(n - 1));
        let (j0, j1) = (index(lo[1], mask.corner[1]).max(0), (index(hi[1], mask.corner[1]) + 1).min(n - 1));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let p = [
                    mask.corner[0] + (i as f64 + 0.5) * cell,
                    mask.corner[1] + (j as f64 + 0.5) * cell,
                ];
                if inside(p, farm) && edge_distance(p, farm) >= PAD_IN {
                    let h = &mut heights[(j * n + i) as usize];
                    *h = h.max(layer::PLOT);
                }
            }
        }
    }
}

/// Whether `p` is inside the polygon `poly` (even-odd).
fn inside(p: P, poly: &[P]) -> bool {
    let mut odd = false;
    let n = poly.len();
    for k in 0..n {
        let (a, b) = (poly[k], poly[(k + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                odd = !odd;
            }
        }
    }
    odd
}

/// The land the grass grows on in the wilds (W9).
pub struct GrassLand {
    pub field: Arc<GroundField>,
    /// How deep the water cut, and how wet the ground is, a sample of the
    /// field each (`land::Sampled`).
    pub cut: Vec<f32>,
    pub wet: Vec<f32>,
}

impl GrassLand {
    /// How much of the meadow the land at `p` keeps, 0 to 1: none on the
    /// steeps, the high ground or in the water's beds.
    fn meadow(&self, p: P) -> f32 {
        if radius(p) <= land::FROM {
            return 1.0;
        }
        let [x, z] = to_world(p[0], p[1]);
        let h = |dx: f32, dz: f32| {
            self.field
                .height_at(x + dx, z + dz)
                .unwrap_or(layer::GROUND)
        };
        let d = 0.5;
        let slope = ((h(d, 0.0) - h(-d, 0.0)).hypot(h(0.0, d) - h(0.0, -d)) / (2.0 * d)) as f64;
        let rise = (h(0.0, 0.0) - layer::GROUND) as f64;
        let cut = land::cut_at(&self.cut, [x, z]) as f64;
        let wet = land::cut_at(&self.wet, [x, z]) as f64;
        (1.0 - smoothstep(STEEP.0, STEEP.1, slope))
            * (1.0 - smoothstep(HIGH.0, HIGH.1, rise))
            * (1.0 - smoothstep(WET.0, WET.1, cut))
            * (1.0 - smoothstep(SODDEN.0, SODDEN.1, wet))
    }

    /// The blades' ground: [`HEIGHTS`]² world heights over `mask`'s square,
    /// one at the middle of each cell (as the engine reads them), row by
    /// row toward +Z.
    fn heights(&self, mask: &Mask) -> Vec<f32> {
        let (origin, size) = mask.world_rect();
        let cell = size[0] / HEIGHTS as f32;
        let mut out = vec![layer::GROUND; HEIGHTS * HEIGHTS];
        for (j, row) in out.chunks_mut(HEIGHTS).enumerate() {
            let z = origin[1] + (j as f32 + 0.5) * cell;
            for (i, h) in row.iter_mut().enumerate() {
                let x = origin[0] + (i as f32 + 0.5) * cell;
                *h = self.field.height_at(x, z).unwrap_or(layer::GROUND);
            }
        }
        out
    }
}

/// The blades' ground over `mask`'s square, [`HEIGHTS`] a side: the land
/// once it's in, else level, and up on the farms' pads (R6). `None` while
/// that's all level, the engine's flat ground.
fn blade_heights(land: Option<&GrassLand>, mask: &Mask, farms: &[Vec<P>]) -> Option<Vec<f32>> {
    let mut heights = match land {
        Some(land) => land.heights(mask),
        None if farms.is_empty() => return None,
        None => vec![layer::GROUND; HEIGHTS * HEIGHTS],
    };
    lift_onto_farms(&mut heights, mask, farms);
    Some(heights)
}

/// How much grass the open ground at `p` holds, 0 to 1, before anything is
/// cleared from it: the rings, then the meadows, on the land once it's in.
fn field(p: P, edge: f64, land: Option<&GrassLand>) -> f32 {
    let r = radius(p);
    let mut rings =
        smoothstep(EASE_FROM, EASE_TO, r) * (1.0 - smoothstep(edge - EDGE_IN, edge + EDGE_OUT, r));
    if let Some(land) = land {
        if rings > 0.0 {
            rings *= land.meadow(p);
        }
    }
    if rings <= 0.0 {
        return 0.0;
    }
    let [x, z] = to_world(p[0], p[1]);
    let broad = value_noise(x / MEADOW_SCALE + 41.0, z / MEADOW_SCALE - 23.0);
    let fine = value_noise(
        x / (MEADOW_SCALE * 0.31) - 7.0,
        z / (MEADOW_SCALE * 0.31) + 5.0,
    );
    let n = broad * 0.75 + fine * 0.25;
    let meadow = MEADOW_FLOOR + (1.0 - MEADOW_FLOOR) * smoothstep(0.28, 0.62, n as f64);
    rings * meadow
}

/// Paints the terrain's part of the mask, `size` texels a side: the rings
/// and meadows on the land (once it's in), cleared of the canals, the roads
/// and the lots. The open ground is shared out by rows over the cores.
pub fn paint(terrain: &TerrainResponse, size: usize, land: Option<&GrassLand>) -> Mask {
    let edge = GRASS_EDGE.min(terrain.world_size * 0.5);
    let side = edge * 2.0;
    let mut mask = Mask {
        texels: vec![0; size * size],
        size,
        corner: [CENTER_X - edge, CENTER_Y - edge],
        side,
    };
    let kerbed = water::kerbed(&terrain.water_rings);
    let slack = mask.slack();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows_each = size.div_ceil(threads);
    let mut texels = std::mem::take(&mut mask.texels);
    std::thread::scope(|scope| {
        let (mask, kerbed) = (&mask, &kerbed);
        for (k, rows) in texels.chunks_mut(rows_each * size).enumerate() {
            scope.spawn(move || {
                for (dj, row) in rows.chunks_mut(size).enumerate() {
                    let j = k * rows_each + dj;
                    for (i, texel) in row.iter_mut().enumerate() {
                        let p = mask.at(i, j);
                        let mut v = field(p, edge, land);
                        if v <= 0.0 {
                            continue;
                        }
                        let r = radius(p);
                        for &(lo, hi) in kerbed {
                            v *= smoothstep(slack, slack + WATER_SOFT, (lo - r).max(r - hi));
                        }
                        *texel = (v * 255.0).round() as u8;
                    }
                }
            });
        }
    });
    mask.texels = texels;
    let mut lawns = Vec::new();
    for park in &terrain.parks {
        lawns.extend(paint_park(&mut mask, park));
    }
    // Roads and lots only matter where something grows: past the suburb,
    // or near a park. A road's box is checked against the grass's reach
    // first, so the city's thousands of segments cost nothing.
    let near = ROAD_SOFT + VERGE + 20.0;
    let grows = |lo: P, hi: P| {
        let far = [
            (lo[0] - CENTER_X).abs().max((hi[0] - CENTER_X).abs()),
            (lo[1] - CENTER_Y).abs().max((hi[1] - CENTER_Y).abs()),
        ];
        far[0].hypot(far[1]) > EASE_FROM - 100.0
            || lawns.iter().any(|&(a, b): &(P, P)| {
                lo[0] < b[0] + near && hi[0] > a[0] - near && lo[1] < b[1] + near && hi[1] > a[1] - near
            })
    };
    for road in &terrain.roads {
        let half = road.width * 0.5 + VERGE;
        for w in road.points.windows(2) {
            let (a, b) = ([w[0].0, w[0].1], [w[1].0, w[1].1]);
            if grows(
                [a[0].min(b[0]), a[1].min(b[1])],
                [a[0].max(b[0]), a[1].max(b[1])],
            ) {
                mask.clear_segment(a, b, half, ROAD_SOFT);
            }
        }
    }
    for plot in &terrain.terrain_plots {
        let corners: Vec<P> = plot.corners.iter().map(|&(x, y)| [x, y]).collect();
        // A farm is pasture (R6): its grass stays.
        if !is_farm(plot) && corners.iter().any(|&c| grows(c, c)) {
            mask.clear_polygon(&corners, LOT_MARGIN, LOT_SOFT);
        }
    }
    // A farm is pasture but for its yard: round the farmhouse and the barn,
    // and the drive in from the lane (R6). Unclaimed farms keep theirs too,
    // so a claim raises the house on a yard already there.
    for farmyard in farmyards(terrain) {
        mask.clear_polygon(&farmyard.yard, YARD, YARD_SOFT);
        let [a, b] = farmyard.drive;
        mask.clear_segment(a, b, DRIVE_HALF, DRIVE_SOFT);
    }
    // Nothing grows on a lake's floor (W10): where the land already lay
    // under its level the carve cut nothing, so the land can't say.
    for lake in &terrain.lakes {
        mask.clear_rings(&lake.rings);
    }
    mask
}

/// Lays `park`'s lawn on the mask: inside its rings, bare along their
/// edges and easing in past that. Returns its box, if it has one.
fn paint_park(mask: &mut Mask, park: &TerrainPark) -> Option<(P, P)> {
    let edges: Vec<(P, P)> = park
        .rings
        .iter()
        .flat_map(|ring| {
            (0..ring.len()).map(move |k| {
                let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                ([a.0, a.1], [b.0, b.1])
            })
        })
        .collect();
    let first = edges.first()?;
    let (lo, hi) = edges.iter().fold((first.0, first.0), |(lo, hi), (a, _)| {
        ([lo[0].min(a[0]), lo[1].min(a[1])], [hi[0].max(a[0]), hi[1].max(a[1])])
    });
    let edge = PARK_EDGE + mask.slack();
    let n = mask.size as i64;
    let [i0, j0] = mask.texel_of(lo);
    let [i1, j1] = mask.texel_of(hi);
    for j in j0.max(0)..=j1.min(n - 1) {
        for i in i0.max(0)..=i1.min(n - 1) {
            let p = mask.at(i as usize, j as usize);
            if !park.contains(p[0], p[1]) {
                continue;
            }
            let d = edges
                .iter()
                .map(|&(a, b)| distance_to_segment(p, a, b))
                .fold(f64::MAX, f64::min);
            let [x, z] = to_world(p[0], p[1]);
            let mottle = value_noise(x / MEADOW_SCALE + 41.0, z / MEADOW_SCALE - 23.0);
            let lawn = PARK_LAWN * (1.0 - PARK_MOTTLE + PARK_MOTTLE * mottle);
            let v = lawn * smoothstep(edge, edge + PARK_SOFT, d);
            let k = j as usize * mask.size + i as usize;
            mask.texels[k] = mask.texels[k].max((v * 255.0).round() as u8);
        }
    }
    Some((lo, hi))
}

/// The grass's look: blades, reach and colour, with the scene's wind.
pub fn config(wind: Wind) -> GrassConfig {
    let rgb = |h: u32| {
        let c = hex(h);
        [c[0], c[1], c[2]]
    };
    GrassConfig {
        enabled: true,
        density: 36.0,
        max_distance: 58.0,
        fade_start: 40.0,
        blade_width: 0.13,
        blade_height: 0.42,
        height_variation: 0.45,
        color_base: rgb(GRASS_ROOT),
        color_tip: rgb(GRASS_TIP),
        color_dry: rgb(GRASS_DRY),
        dry_amount: 0.3,
        wind_direction: wind.direction,
        wind_speed: wind.speed,
        wind_strength: 0.09,
        density_threshold: 0.04,
        ..GrassConfig::default()
    }
}

/// The wind the grass sways in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wind {
    /// World XZ, as `[x, 0, z]`.
    pub direction: [f32; 3],
    /// The sway's angular speed, radians a second.
    pub speed: f32,
}

impl Default for Wind {
    /// The scene's ocean as authored: 30°, 2.5 m/s.
    fn default() -> Self {
        Wind::from_ocean(30.0, 2.5)
    }
}

impl Wind {
    /// From the ocean's `direction_deg` (0 = +Z, 90 = +X, as its waves
    /// take it) and `wind_speed` (m/s): the same heading, and a sway about
    /// half a cycle a second in its breeze.
    pub fn from_ocean(direction_deg: f32, wind_speed: f32) -> Self {
        let a = direction_deg.to_radians();
        Wind {
            direction: [a.sin(), 0.0, a.cos()],
            speed: (wind_speed * 0.5).clamp(0.4, 3.0),
        }
    }

    /// The scene's: read from its `ocean` component, else the default.
    pub fn of_scene(world: &FlintWorld) -> Self {
        let ocean = world
            .entities_with_component("ocean")
            .iter()
            .min()
            .copied()
            .and_then(|id| world.get_component(id, "ocean"));
        let field = |f: &str| crate::atmosphere::number(ocean.and_then(|c| c.get(f)));
        let d = Wind::default();
        match (field("direction_deg"), field("wind_speed")) {
            (None, None) => d,
            (deg, speed) => Wind::from_ocean(deg.unwrap_or(30.0), speed.unwrap_or(2.5)),
        }
    }
}

/// The grass on the world: the terrain's mask, the nodes cleared from it,
/// and what was last handed to the engine.
#[derive(Default)]
pub struct Grass {
    /// The terrain's part, `None` before the first terrain.
    base: Option<Mask>,
    /// The nodes whose clearings are wanted, in server units, sorted.
    nodes: Vec<Node>,
    /// The nodes in the mask the engine has, or `None` to upload again.
    painted: Option<Vec<Node>>,
    wind: Option<Wind>,
    /// The land, once it's in, and the blades' heights on it.
    land: Option<GrassLand>,
    heights: Option<Vec<f32>>,
}

impl Grass {
    /// Paints the terrain's part again (when its roads, lots, parks or
    /// rings changed).
    pub fn terrain(&mut self, terrain: &TerrainResponse) {
        let started = Instant::now();
        let base = paint(terrain, MASK_SIZE, self.land.as_ref());
        self.heights = blade_heights(self.land.as_ref(), &base, &farms(terrain));
        self.base = Some(base);
        self.painted = None;
        tracing::debug!(
            "grass: painted the mask in {} ms",
            started.elapsed().as_millis()
        );
    }

    /// The land is in (W9): paint again on it, and stand the blades on it.
    pub fn land(&mut self, field: Arc<GroundField>, cut: Vec<f32>, wet: Vec<f32>, terrain: &TerrainResponse) {
        self.land = Some(GrassLand { field, cut, wet });
        self.terrain(terrain);
    }

    /// The nodes the server has shown us, in server units, each with its
    /// clearing: how far it's bare, then how far soft.
    pub fn nodes(&mut self, at: impl Iterator<Item = (P, (f64, f64))>) {
        let mut nodes: Vec<Node> = at
            .map(|(p, (bare, soft))| {
                let r = |v: f64| v.round() as i64;
                [r(p[0]), r(p[1]), r(bare), r(soft)]
            })
            .collect();
        nodes.sort_unstable();
        nodes.dedup();
        self.nodes = nodes;
    }

    /// Hands the engine what changed: a new terrain's whole mask, or the
    /// texels round each node come or gone.
    pub fn flush(&mut self, gpu: &mut ExtensionGpu, world: &FlintWorld) {
        let Some(base) = &self.base else {
            return;
        };
        if let Some(painted) = &self.painted {
            if *painted == self.nodes {
                return;
            }
            let patches = patches(base, painted, &self.nodes);
            tracing::debug!("grass: {} node clearings written in place", patches.len());
            for (win, texels) in patches {
                let at = |v: usize| v as u32;
                if let Err(e) =
                    gpu.update_grass_mask(at(win.i0), at(win.j0), at(win.w), at(win.h), &texels)
                {
                    // Start again from the whole mask next frame.
                    tracing::warn!("grass: {e}");
                    self.painted = None;
                    return;
                }
            }
            self.painted = Some(self.nodes.clone());
            return;
        }
        let wind = *self.wind.get_or_insert_with(|| Wind::of_scene(world));
        let mask = with_nodes(base, &self.nodes);
        let (origin, size) = mask.world_rect();
        let ground = GrassGround {
            origin,
            size,
            mask: &mask.texels,
            mask_width: mask.size as u32,
            mask_height: mask.size as u32,
            scale_by_mask: true,
            height: match &self.heights {
                Some(heights) => GrassHeight::Map {
                    heights,
                    width: HEIGHTS as u32,
                    depth: HEIGHTS as u32,
                },
                None => GrassHeight::Flat(layer::GROUND),
            },
        };
        if let Err(e) = gpu.set_grass_ground(&config(wind), &ground) {
            tracing::warn!("grass: {e}");
        }
        self.painted = Some(self.nodes.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_core::terrain::read_terrain_file;
    use capitol_protocol::rules::Zone;
    use capitol_protocol::terrain::WaterRing;
    use std::path::Path;
    use std::sync::OnceLock;

    #[test]
    fn the_mask_and_heights_follow_the_world_at_a_fixed_texel() {
        // The 13600 world keeps the sizes it had (#308), so its grass and
        // captures don't move; a 16600 one (PV2) gets more of the same texel.
        assert_eq!((texels(13600.0, MASK_TEXEL), texels(13600.0, HEIGHT_STEP)), (3480, 2320));
        assert_eq!((texels(16600.0, MASK_TEXEL), texels(16600.0, HEIGHT_STEP)), (4248, 2832));
        assert_eq!(MASK_SIZE, texels(capitol_protocol::rules::WORLD_SIZE, MASK_TEXEL));
    }

    /// Coarser than the game's, to keep the tests quick; still under a
    /// metre a texel.
    const SIZE: usize = 1740;

    fn fixture() -> &'static TerrainResponse {
        static T: OnceLock<TerrainResponse> = OnceLock::new();
        T.get_or_init(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
            read_terrain_file(&path).unwrap()
        })
    }

    fn mask() -> &'static Mask {
        static M: OnceLock<Mask> = OnceLock::new();
        M.get_or_init(|| paint(fixture(), SIZE, None))
    }

    #[test]
    fn nothing_grows_on_a_lake_floor() {
        let t = fixture();
        let m = paint(t, SIZE, None);
        const SIDE_TEXEL: f64 = capitol_protocol::rules::WORLD_SIZE / SIZE as f64;
        let (mut wet, mut grows_round) = (0, 0);
        for lake in &t.lakes {
            let (cx, cy) = lake.rings[0].iter().fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
            let n = lake.rings[0].len() as f64;
            for k in 0..64 {
                let a = k as f64 * std::f64::consts::TAU / 64.0;
                for reach in [10.0, 30.0, 60.0] {
                    let p = [cx / n + reach * a.cos(), cy / n + reach * a.sin()];
                    // A texel clear of the shore: one that straddles it is
                    // the bank's, by its centre.
                    let d = SIDE_TEXEL * std::f64::consts::SQRT_2;
                    let clear = [(d, 0.0), (-d, 0.0), (0.0, d), (0.0, -d), (d, d), (-d, -d), (d, -d), (-d, d)]
                        .iter()
                        .all(|&(dx, dy)| lake.contains(p[0] + dx, p[1] + dy));
                    if clear {
                        assert_eq!(m.value_at(p), 0, "{} at {p:?}", lake.id);
                        wet += 1;
                    }
                }
            }
            // Its shores still grow, somewhere.
            let (x, y) = lake.rings[0][0];
            grows_round += (0..16).any(|k| {
                let a = k as f64 * std::f64::consts::TAU / 16.0;
                let p = [x + 40.0 * a.cos(), y + 40.0 * a.sin()];
                !lake.contains(p[0], p[1]) && m.value_at(p) > 0
            }) as usize;
        }
        assert!(wet > 500, "{wet}");
        assert!(grows_round > t.lakes.len() / 2, "{grows_round}");
    }

    #[test]
    fn nothing_grows_inside_the_suburb_but_its_parks() {
        let t = fixture();
        assert!(!t.parks.is_empty(), "the fixture has parks");
        let m = mask();
        let boxes: Vec<(P, P, &TerrainPark)> = t
            .parks
            .iter()
            .map(|k| {
                let pts = k.rings.iter().flatten();
                let lo = pts.clone().fold([f64::MAX; 2], |m, p| [m[0].min(p.0), m[1].min(p.1)]);
                let hi = pts.fold([f64::MIN; 2], |m, p| [m[0].max(p.0), m[1].max(p.1)]);
                (lo, hi, k)
            })
            .collect();
        let in_park = |p: P| {
            boxes.iter().any(|(lo, hi, k)| {
                p[0] >= lo[0] && p[0] <= hi[0] && p[1] >= lo[1] && p[1] <= hi[1] && k.contains(p[0], p[1])
            })
        };
        let (mut lawn, mut n) = (0, 0);
        for j in 0..SIZE {
            for i in 0..SIZE {
                let p = m.at(i, j);
                if radius(p) >= EASE_FROM {
                    continue;
                }
                let v = m.texels[j * SIZE + i];
                match in_park(p) {
                    false => assert_eq!(v, 0, "{p:?}"),
                    true => {
                        // Mown: never as tall as a full meadow.
                        assert!(v as f32 <= PARK_LAWN * 255.0 + 1.0, "{p:?}: {v}");
                        n += 1;
                        lawn += (v > 0) as usize;
                    }
                }
            }
        }
        // Most of a park is lawn; only its edges are bare.
        assert!(lawn as f64 > 0.6 * n as f64, "{lawn} of {n} park texels grow");
    }

    #[test]
    fn the_rural_ring_and_the_wilds_edge_are_mostly_grass() {
        mostly_grass(mask());
    }

    /// Both rings sampled on a fine polar grid: most of the open ground has
    /// grass, and some of it is full. Since #423 the rural ring has about 380
    /// farms, not 117, and each clears its yard and drive, claimed or not:
    /// about 79% of the ring has grass, against over 85% before. So the floor
    /// is 75%.
    fn mostly_grass(m: &Mask) {
        for (lo, hi) in [
            (Zone::Rural.radii().0 + 200.0, Zone::Rural.radii().1),
            (Zone::Rural.radii().1, GRASS_EDGE - EDGE_IN),
        ] {
            let (mut any, mut full, mut n) = (0, 0, 0);
            for k in 0..720 {
                for s in 0..8 {
                    let r = lo + (hi - lo) * (s as f64 + 0.5) / 8.0;
                    let a = (k as f64).to_radians() * 0.5;
                    let v = m.value_at([CENTER_X + r * a.cos(), CENTER_Y + r * a.sin()]);
                    n += 1;
                    any += (v > 0) as usize;
                    full += (v > 240) as usize;
                }
            }
            let (any, full) = (any as f64 / n as f64, full as f64 / n as f64);
            assert!(any > 0.75, "{lo}..{hi}: {any} has grass");
            assert!(full > 0.25, "{lo}..{hi}: {full} full");
        }
    }

    #[test]
    fn no_road_town_lot_or_farmyard_has_grass_but_farms_are_pasture() {
        bare_where_it_should_be(fixture(), mask());
    }

    fn centre(poly: &[P]) -> P {
        let s = poly.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
        [s[0] / poly.len() as f64, s[1] / poly.len() as f64]
    }

    fn between(a: P, b: P, f: f64) -> P {
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
    }

    /// Nothing grows on a road, on a lot in town or in a farmyard, and nearly
    /// every farm is mostly grass.
    fn bare_where_it_should_be(t: &TerrainResponse, m: &Mask) {
        for road in &t.roads {
            for w in road.points.windows(2) {
                for k in 0..=8 {
                    let f = k as f64 / 8.0;
                    let p = [
                        w[0].0 + (w[1].0 - w[0].0) * f,
                        w[0].1 + (w[1].1 - w[0].1) * f,
                    ];
                    assert_eq!(m.value_at(p), 0, "{} at {p:?}", road.id);
                }
            }
        }
        let (mut farms, mut pasture) = (0, 0);
        for plot in &t.terrain_plots {
            let poly = plots::hull(&plot.corners.map(|(x, y)| [x, y]));
            let mid = centre(&poly);
            if !is_farm(plot) {
                assert_eq!(m.value_at(mid), 0, "{}", plot.id);
                for &p in &poly {
                    assert_eq!(m.value_at(p), 0, "{} corner", plot.id);
                }
                continue;
            }
            // Over half the field grows.
            let (lo, hi) = poly.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), c| {
                ([lo[0].min(c[0]), lo[1].min(c[1])], [hi[0].max(c[0]), hi[1].max(c[1])])
            });
            let (mut n, mut grows) = (0, 0);
            for i in 0..24 {
                for j in 0..24 {
                    let p = [
                        lo[0] + (hi[0] - lo[0]) * (i as f64 + 0.5) / 24.0,
                        lo[1] + (hi[1] - lo[1]) * (j as f64 + 0.5) / 24.0,
                    ];
                    if inside(p, &poly) {
                        n += 1;
                        grows += (m.value_at(p) > 0) as usize;
                    }
                }
            }
            farms += 1;
            pasture += (grows * 2 > n) as usize;
        }
        let yards = farmyards(t);
        assert!(pasture * 10 > farms * 9, "{pasture} of {farms} farms are mostly pasture");
        assert!(yards.len() * 10 > farms * 9, "{} yards on {farms} farms", yards.len());
        for y in &yards {
            assert_eq!(m.value_at(centre(&y.yard)), 0, "a yard grows at {:?}", centre(&y.yard));
            for &p in &y.yard {
                assert_eq!(m.value_at(p), 0, "a farmstead's corner grows at {p:?}");
            }
            let [a, b] = y.drive;
            for k in 0..=8 {
                let p = between(a, b, k as f64 / 8.0);
                assert_eq!(m.value_at(p), 0, "a drive grows at {p:?}");
            }
        }
    }

    #[test]
    fn blades_on_a_farm_stand_on_its_pad() {
        // R6: in the middle of a farm the blades stand on its pad at
        // `layer::PLOT`, and off the pad (4 in from the farm's edge) none
        // stands above the land, blended as the engine blends them
        // (`blades_stand_on_the_land`).
        let t = fixture();
        let m = mask();
        let farms = farms(t);
        let heights = blade_heights(None, m, &farms).expect("the fixture has farms");
        let (origin, size) = m.world_rect();
        let n = HEIGHTS as f32;
        let blend = |p: P| {
            let [x, z] = to_world(p[0], p[1]);
            let at = |c: f32, o: f32, s: f32| ((c - o) / s * n - 0.5).clamp(0.0, n - 1.0);
            let (fx, fz) = (at(x, origin[0], size[0]), at(z, origin[1], size[1]));
            let (i, j) = (fx as usize, fz as usize);
            let (i1, j1) = ((i + 1).min(HEIGHTS - 1), (j + 1).min(HEIGHTS - 1));
            let (tx, tz) = (fx - i as f32, fz - j as f32);
            let h = |i: usize, j: usize| heights[j * HEIGHTS + i];
            let near = h(i, j) + (h(i1, j) - h(i, j)) * tx;
            let far = h(i, j1) + (h(i1, j1) - h(i, j1)) * tx;
            near + (far - near) * tz
        };
        let mut edges = 0;
        for farm in &farms {
            let mid = centre(farm);
            assert_eq!(blend(mid), layer::PLOT);
            for k in 0..farm.len() {
                let (a, b) = (farm[k], farm[(k + 1) % farm.len()]);
                let inward = {
                    let d = [-(b[1] - a[1]), b[0] - a[0]];
                    let l = d[0].hypot(d[1]);
                    let d = [d[0] / l, d[1] / l];
                    if (mid[0] - a[0]) * d[0] + (mid[1] - a[1]) * d[1] > 0.0 { d } else { [-d[0], -d[1]] }
                };
                for f in 1..20 {
                    let on = between(a, b, f as f64 / 20.0);
                    for depth in [-6.0, -3.0, 0.0, 2.0, 3.9] {
                        let p = [on[0] + inward[0] * depth, on[1] + inward[1] * depth];
                        // Not on the pad of the farm next door.
                        if farms.iter().any(|f| f != farm && inside(p, f) && edge_distance(p, f) > 4.0) {
                            continue;
                        }
                        assert!(blend(p) <= layer::GROUND + 1e-6, "a blade floats at {p:?}");
                        edges += 1;
                    }
                }
            }
        }
        assert!(edges > 1000);
    }

    #[test]
    fn a_canal_in_the_fields_is_bare_and_its_banks_soft() {
        // The default world's canals are all inside the suburb, so put one
        // in the rural ring.
        let mut t = fixture().clone();
        t.roads.clear();
        t.terrain_plots.clear();
        t.water_rings.push(WaterRing {
            id: "test".into(),
            radius: 3400.0,
            width: 30.0,
            bridges: vec![],
        });
        let m = paint(&t, SIZE, None);
        let at = |r: f64| m.value_at([CENTER_X + r, CENTER_Y]);
        assert_eq!(at(3400.0), 0);
        let (_, hi) = water::kerbed(&t.water_rings)[3];
        assert_eq!(at(hi - 1.0), 0);
        assert!(at(hi + WATER_SOFT + 30.0) > at(hi + WATER_SOFT * 0.5));
    }

    #[test]
    fn a_node_gets_a_clearing_and_nothing_else_changes() {
        let mut m = mask().clone();
        // Somewhere full of grass in the rural ring.
        let c = (0..360)
            .map(|k| {
                let a = (k as f64).to_radians();
                [CENTER_X + 3700.0 * a.cos(), CENTER_Y + 3700.0 * a.sin()]
            })
            .find(|&p| m.value_at(p) > 200)
            .expect("a full spot");
        let before = m.texels.clone();
        m.clear_node(c);
        assert_eq!(m.value_at(c), 0);
        assert_eq!(m.value_at([c[0] + NODE_CLEAR * 0.7, c[1]]), 0);
        let reach = NODE_CLEAR + NODE_SOFT + m.side / SIZE as f64;
        for j in 0..SIZE {
            for i in 0..SIZE {
                let p = m.at(i, j);
                if (p[0] - c[0]).hypot(p[1] - c[1]) > reach {
                    assert_eq!(m.texels[j * SIZE + i], before[j * SIZE + i]);
                }
            }
        }
    }

    #[test]
    fn patching_the_nodes_in_and_out_matches_a_whole_repaint() {
        let base = mask();
        let c = (0..360)
            .map(|k| {
                let a = (k as f64).to_radians();
                [CENTER_X + 3600.0 * a.cos(), CENTER_Y + 3600.0 * a.sin()]
            })
            .find(|&p| base.value_at(p) > 200)
            .expect("a full spot");
        let tuft = [NODE_CLEAR as i64, NODE_SOFT as i64];
        let at = |dx: i64, dy: i64| [c[0] as i64 + dx, c[1] as i64 + dy, tuft[0], tuft[1]];
        // A tree's wide soft shade.
        let tree = |dx: i64, dy: i64| [c[0] as i64 + dx, c[1] as i64 + dy, 14, 26];
        // Overlapping clearings, one on the mask's corner (its window
        // clipped) and one off the mask altogether.
        let corner = [
            (CENTER_X - GRASS_EDGE) as i64 + 5,
            (CENTER_Y - GRASS_EDGE) as i64 + 5,
            tuft[0],
            tuft[1],
        ];
        let sorted = |mut v: Vec<Node>| {
            v.sort_unstable();
            v
        };
        let sets = [
            sorted(vec![at(0, 0), at(20, 0), [-1_000_000, 0, 16, 14]]),
            sorted(vec![at(20, 0), at(10, 12), tree(-25, 5), corner]),
            vec![],
            sorted(vec![at(0, 0), tree(3, -2), corner]),
        ];
        let mut live = with_nodes(base, &sets[0]);
        for pair in sets.windows(2) {
            let (old, new) = (&pair[0], &pair[1]);
            for (win, texels) in patches(base, old, new) {
                assert!(win.w <= 32 && win.h <= 32, "{win:?}");
                for r in 0..win.h {
                    let row = (win.j0 + r) * live.size + win.i0;
                    live.texels[row..row + win.w]
                        .copy_from_slice(&texels[r * win.w..(r + 1) * win.w]);
                }
            }
            let want = with_nodes(base, new);
            assert!(live.texels == want.texels, "{old:?} -> {new:?}");
        }
        // Nothing changed, nothing written.
        assert!(patches(base, &sets[1], &sets[1]).is_empty());
    }

    #[test]
    fn in_the_wilds_the_grass_keeps_to_the_meadows() {
        // W9: on the land, nothing grows on the high ground or in the
        // water's beds, and the low hills are still meadow.
        let sampled = crate::world::land::tests::fixture_land();
        let land = GrassLand {
            field: Arc::new(crate::world::land::field(sampled.field.heightmap().clone_heights())),
            cut: sampled.cut.clone(),
            wet: sampled.wet.clone(),
        };
        let m = paint(fixture(), SIZE, Some(&land));
        let (mut high, mut wet, mut low, mut low_grows) = (0, 0, 0, 0);
        for k in 0..720 {
            let a = (k as f64).to_radians() * 0.5;
            for s in 0.. {
                let r = land::FROM + 100.0 + s as f64 * 50.0;
                if r > GRASS_EDGE - EDGE_IN {
                    break;
                }
                // The middle of the texel there, where it was painted.
                let [i, j] = m.texel_of([CENTER_X + r * a.cos(), CENTER_Y + r * a.sin()]);
                let p = m.at(i as usize, j as usize);
                let at = to_world(p[0], p[1]);
                let rise = land.field.height_at(at[0], at[1]).unwrap() - layer::GROUND;
                let v = m.value_at(p);
                if rise > HIGH.1 as f32 {
                    high += 1;
                    assert_eq!(v, 0, "grass {v} {rise} m up at {p:?}");
                } else if land::cut_at(&land.cut, at) > WET.1 as f32 {
                    wet += 1;
                    assert_eq!(v, 0, "grass {v} in a bed at {p:?}");
                } else if rise < 8.0 && land.meadow(p) > 0.9 {
                    low += 1;
                    low_grows += (v > 0) as usize;
                }
            }
        }
        assert!(high > 100 && wet > 10 && low > 1000, "{high} high, {wet} wet, {low} low");
        assert!(low_grows * 10 > low * 8, "{low_grows} of {low} low meadows grow");
        // The blades stand on the land.
        let heights = land.heights(&m);
        let (origin, size) = m.world_rect();
        let cell = size[0] / HEIGHTS as f32;
        for (i, j) in [(300, 1700), (1900, 1000), (1024, 100)] {
            let (x, z) = (origin[0] + (i as f32 + 0.5) * cell, origin[1] + (j as f32 + 0.5) * cell);
            let want = land.field.height_at(x, z).unwrap();
            assert!((heights[j * HEIGHTS + i] - want).abs() < 1e-4);
        }
    }

    #[test]
    fn blades_stand_on_the_land() {
        // #228: the engine stands a blade on the blend of the four heights
        // round it, each at the middle of its cell (Flint ADR 0098). Blended
        // the same way here at every growing spot along 1440 rays, the grid
        // keeps blades within a couple of centimetres of the land, and the
        // worst, on the rough ground high in the ranges, within a few tens.
        // (Read nearest, as before, the 99th was 0.15 m and the worst 0.41.)
        let sampled = crate::world::land::tests::fixture_land();
        let land = GrassLand {
            field: Arc::new(crate::world::land::field(sampled.field.heightmap().clone_heights())),
            cut: sampled.cut.clone(),
            wet: sampled.wet.clone(),
        };
        let m = paint(fixture(), SIZE, Some(&land));
        let heights = land.heights(&m);
        let (origin, size) = m.world_rect();
        let n = HEIGHTS as f32;
        let blend = |x: f32, z: f32| {
            let at = |c: f32, o: f32, s: f32| ((c - o) / s * n - 0.5).clamp(0.0, n - 1.0);
            let (fx, fz) = (at(x, origin[0], size[0]), at(z, origin[1], size[1]));
            let (i, j) = (fx as usize, fz as usize);
            let (i1, j1) = ((i + 1).min(HEIGHTS - 1), (j + 1).min(HEIGHTS - 1));
            let (tx, tz) = (fx - i as f32, fz - j as f32);
            let h = |i: usize, j: usize| heights[j * HEIGHTS + i];
            let near = h(i, j) + (h(i1, j) - h(i, j)) * tx;
            let far = h(i, j1) + (h(i1, j1) - h(i, j1)) * tx;
            near + (far - near) * tz
        };
        let mut off = Vec::new();
        for k in 0..1440 {
            let a = (k as f64).to_radians() * 0.25;
            for s in 0..400 {
                // Off the grid's points, and the land's
                let r = land::FROM + 100.0 + s as f64 * 5.3;
                if r > GRASS_EDGE - EDGE_IN {
                    break;
                }
                let p = [CENTER_X + r * a.cos(), CENTER_Y + r * a.sin()];
                if m.value_at(p) > 0 {
                    let [x, z] = to_world(p[0], p[1]);
                    off.push((blend(x, z) - land.field.height_at(x, z).unwrap()).abs());
                }
            }
        }
        assert!(off.len() > 100_000, "only {} blades", off.len());
        off.sort_by(f32::total_cmp);
        let p99 = off[off.len() * 99 / 100];
        let worst = off[off.len() - 1];
        assert!(p99 < 0.03 && worst < 0.3, "99% within {p99} m, the worst {worst} m");
    }

    #[test]
    fn the_mask_covers_the_grass_square() {
        let m = mask();
        let (origin, size) = m.world_rect();
        let half = GRASS_EDGE as f32 * S;
        assert_eq!(origin, [-half, -half]);
        assert_eq!(size, [half * 2.0, half * 2.0]);
    }

    #[test]
    fn the_wind_follows_the_ocean() {
        // The ocean's convention: 0° is +Z, 90° is +X.
        let w = Wind::from_ocean(90.0, 2.0);
        assert!((w.direction[0] - 1.0).abs() < 1e-6 && w.direction[2].abs() < 1e-6);
        let w = Wind::from_ocean(0.0, 2.0);
        assert!(w.direction[0].abs() < 1e-6 && (w.direction[2] - 1.0).abs() < 1e-6);
        assert_eq!(w.speed, 1.0);
        assert_eq!(Wind::default(), Wind::from_ocean(30.0, 2.5));
    }
}

