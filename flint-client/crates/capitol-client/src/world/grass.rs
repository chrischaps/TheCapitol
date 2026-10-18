//! Grass fields (P8.6c): Flint's compute grass (H21, Flint ADR 0089) over
//! the rural ring and the wilderness, so past the suburb the city gives way
//! to meadow. The engine grows blades wherever one R8 mask over the whole
//! world says to, and only near the camera, so the cost follows the view,
//! not the fields.
//!
//! The mask is painted here, on the CPU, from the terrain:
//!
//! - **the rings**: none inside the suburb, easing in across its outer edge,
//!   full through the rural ring and the wilderness, then thinning out
//!   just past the world's edge, where the ground fades to the background;
//! - **parks** (#129): the suburb's parks are mown lawns, shorter and more
//!   even than the meadows, easing in from their edges where the lots and
//!   the streets stand;
//! - **meadows**: a slow value noise (the ground's own, so the fields and
//!   its mottling share a grain) thins the grass in broad patches, so it
//!   reads as fields rather than as a carpet;
//! - **cleared**: nothing on roads and a verge beside them, on lots (the
//!   houses stand there, P8.6b), in the canals and on their kerbs, and in a
//!   small clearing round each gatherable node, so its tuft still reads as
//!   a thing to take. Every clearing has a soft rim, which `scale_by_mask`
//!   turns into the field thinning toward the edge rather than stopping.
//!
//! The terrain part is painted when the roads, the lots, the parks or the
//! rings change. Nodes arrive as you walk (the server lists those within 80 m),
//! so their clearings are stamped onto a copy of it, at most once a
//! second, whenever the set of nodes seen changes.
//!
//! The colours are `palette` tokens. The grass is lit by the scene's `sun`
//! like everything else, so it warms at dusk and goes blue under the moon
//! with no help (`daylight.rs`). The wind is the scene's: the ocean's
//! direction and speed, so the grass and the canals move together.

use crate::coords::{layer, to_world, S};
use crate::world::geom::{distance_to_segment, radius, P};
use crate::world::ground::value_noise;
use crate::world::palette::{hex, GRASS_DRY, GRASS_ROOT, GRASS_TIP};
use crate::world::water;
use capitol_protocol::rules::{CENTER_X, CENTER_Y, SUBURBAN_OUTER_RADIUS};
use capitol_protocol::terrain::{TerrainPark, TerrainResponse};
use flint_ecs::FlintWorld;
use flint_player::{ExtensionGpu, GrassConfig, GrassGround, GrassHeight};
use std::time::{Duration, Instant};

/// Texels along each side of the mask. Over the 800 m world that's about
/// 0.39 m a texel: finer than the narrowest trail (0.6 m) and its verge.
pub const MASK_SIZE: usize = 2048;

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
/// Past a canal's kerb.
const WATER_SOFT: f64 = 8.0;
/// A node's clearing: bare out to this radius, then soft. A tuft and its
/// mound are about 0.5 m across (`nodes.rs`), so this leaves a ring of
/// short ground round it.
const NODE_CLEAR: f64 = 16.0;
const NODE_SOFT: f64 = 14.0;

/// Repaint the node clearings no more often than this. Each repaint
/// uploads the whole mask and reinstalls the grass; a sub-rectangle write
/// would remove the throttle (#135).
const REPAINT_EVERY: Duration = Duration::from_secs(1);

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

    /// Lowers each texel within `reach` of the box `lo`..`hi` (server) to
    /// `clear(p)` of itself, `clear` running 0 (bare) to 1 (untouched).
    fn clear_near(&mut self, lo: P, hi: P, reach: f64, clear: impl Fn(P) -> f32) {
        let n = self.size as i64;
        let [i0, j0] = self.texel_of([lo[0] - reach, lo[1] - reach]);
        let [i1, j1] = self.texel_of([hi[0] + reach, hi[1] + reach]);
        for j in j0.max(0)..=j1.min(n - 1) {
            for i in i0.max(0)..=i1.min(n - 1) {
                let k = j as usize * self.size + i as usize;
                if self.texels[k] == 0 {
                    continue;
                }
                let keep = clear(self.at(i as usize, j as usize));
                if keep < 1.0 {
                    self.texels[k] = (self.texels[k] as f32 * keep).round() as u8;
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

    /// Clears a disc round `c`.
    fn clear_disc(&mut self, c: P, bare: f64, soft: f64) {
        let bare = bare + self.slack();
        self.clear_near(c, c, bare + soft, |p| {
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

/// How much grass the open ground at `p` holds, 0 to 1, before anything is
/// cleared from it: the rings, then the meadows.
fn field(p: P, edge: f64) -> f32 {
    let r = radius(p);
    let rings =
        smoothstep(EASE_FROM, EASE_TO, r) * (1.0 - smoothstep(edge - EDGE_IN, edge + EDGE_OUT, r));
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
/// and meadows, cleared of the canals, the roads and the lots.
pub fn paint(terrain: &TerrainResponse, size: usize) -> Mask {
    let side = terrain.world_size;
    let edge = side * 0.5;
    let mut mask = Mask {
        texels: vec![0; size * size],
        size,
        corner: [CENTER_X - edge, CENTER_Y - edge],
        side,
    };
    let kerbed = water::kerbed(&terrain.water_rings);
    let slack = mask.slack();
    for j in 0..size {
        for i in 0..size {
            let p = mask.at(i, j);
            let mut v = field(p, edge);
            if v <= 0.0 {
                continue;
            }
            let r = radius(p);
            for &(lo, hi) in &kerbed {
                v *= smoothstep(slack, slack + WATER_SOFT, (lo - r).max(r - hi));
            }
            mask.texels[j * size + i] = (v * 255.0).round() as u8;
        }
    }
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
        if corners.iter().any(|&c| grows(c, c)) {
            mask.clear_polygon(&corners, LOT_MARGIN, LOT_SOFT);
        }
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
    nodes: Vec<[i64; 2]>,
    /// The nodes in the mask the engine has, or `None` to upload again.
    painted: Option<Vec<[i64; 2]>>,
    last: Option<Instant>,
    wind: Option<Wind>,
}

impl Grass {
    /// Paints the terrain's part again (when its roads, lots, parks or
    /// rings changed).
    pub fn terrain(&mut self, terrain: &TerrainResponse) {
        let started = Instant::now();
        self.base = Some(paint(terrain, MASK_SIZE));
        self.painted = None;
        self.last = None;
        tracing::debug!(
            "grass: painted the mask in {} ms",
            started.elapsed().as_millis()
        );
    }

    /// The nodes the server has shown us, in server units.
    pub fn nodes(&mut self, at: impl Iterator<Item = P>) {
        let mut nodes: Vec<[i64; 2]> = at
            .map(|p| [p[0].round() as i64, p[1].round() as i64])
            .collect();
        nodes.sort_unstable();
        nodes.dedup();
        self.nodes = nodes;
    }

    /// Hands the mask to the engine if it changed, the terrain at once and
    /// the nodes at most once every `REPAINT_EVERY`.
    pub fn flush(&mut self, gpu: &mut ExtensionGpu, world: &FlintWorld, now: Instant) {
        let Some(base) = &self.base else {
            return;
        };
        if self.painted.as_ref() == Some(&self.nodes) {
            return;
        }
        let terrain_new = self.painted.is_none();
        if !terrain_new && self.last.is_some_and(|t| now - t < REPAINT_EVERY) {
            return;
        }
        let wind = *self.wind.get_or_insert_with(|| Wind::of_scene(world));
        let mut mask = base.clone();
        for &[x, y] in &self.nodes {
            mask.clear_disc([x as f64, y as f64], NODE_CLEAR, NODE_SOFT);
        }
        let (origin, size) = mask.world_rect();
        let ground = GrassGround {
            origin,
            size,
            mask: &mask.texels,
            mask_width: mask.size as u32,
            mask_height: mask.size as u32,
            scale_by_mask: true,
            height: GrassHeight::Flat(layer::GROUND),
        };
        if let Err(e) = gpu.set_grass_ground(&config(wind), &ground) {
            tracing::warn!("grass: {e}");
        }
        self.painted = Some(self.nodes.clone());
        self.last = Some(now);
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

    /// Coarser than the game's, to keep the tests quick; still under a
    /// metre a texel.
    const SIZE: usize = 1024;

    fn fixture() -> &'static TerrainResponse {
        static T: OnceLock<TerrainResponse> = OnceLock::new();
        T.get_or_init(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
            read_terrain_file(&path).unwrap()
        })
    }

    fn mask() -> &'static Mask {
        static M: OnceLock<Mask> = OnceLock::new();
        M.get_or_init(|| paint(fixture(), SIZE))
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
    fn the_rural_ring_and_the_wilderness_are_mostly_grass() {
        // Sample both rings on a fine polar grid: most of the open ground
        // has grass, and some of it is full.
        let m = mask();
        for (lo, hi) in [
            (Zone::Rural.radii().0 + 200.0, Zone::Rural.radii().1),
            (Zone::Rural.radii().1, Zone::Wilderness.radii().1 - EDGE_IN),
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
            assert!(any > 0.85, "{lo}..{hi}: {any} has grass");
            assert!(full > 0.25, "{lo}..{hi}: {full} full");
        }
    }

    #[test]
    fn no_road_or_lot_has_grass_on_it() {
        let t = fixture();
        let m = mask();
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
        for plot in &t.terrain_plots {
            let c = plot.corners;
            let mid = [
                c.iter().map(|p| p.0).sum::<f64>() / 4.0,
                c.iter().map(|p| p.1).sum::<f64>() / 4.0,
            ];
            assert_eq!(m.value_at(mid), 0, "{}", plot.id);
            for &(x, y) in &c {
                assert_eq!(m.value_at([x, y]), 0, "{} corner", plot.id);
            }
        }
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
        let m = paint(&t, SIZE);
        let at = |r: f64| m.value_at([CENTER_X + r, CENTER_Y]);
        assert_eq!(at(3400.0), 0);
        let (_, hi) = water::kerbed(&t.water_rings)[3];
        assert_eq!(at(hi - 1.0), 0);
        assert!(at(hi + WATER_SOFT + 30.0) > at(hi + WATER_SOFT * 0.5));
    }

    #[test]
    fn a_node_gets_a_clearing_and_nothing_else_changes() {
        let mut m = mask().clone();
        // Somewhere full of grass in the wilderness.
        let c = (0..360)
            .map(|k| {
                let a = (k as f64).to_radians();
                [CENTER_X + 3700.0 * a.cos(), CENTER_Y + 3700.0 * a.sin()]
            })
            .find(|&p| m.value_at(p) > 200)
            .expect("a full spot");
        let before = m.texels.clone();
        m.clear_disc(c, NODE_CLEAR, NODE_SOFT);
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
    fn the_mask_covers_the_world_square() {
        let m = mask();
        let (origin, size) = m.world_rect();
        let half = (fixture().world_size * 0.5) as f32 * S;
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
