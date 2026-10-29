//! The wilds' nodes, from the land (W6, #199).
//!
//! Each resource that grows in the Wilderness has a habitat in
//! `content/wilds.toml`: bounds on the land's height, its slope and the
//! radius. Its nodes go where the land suits it, in its [`Cluster`] pattern:
//!
//! - **woods** (the old growth): the valley floors, where the woods' noise
//!   field (the rural woods' own, so the woods run on over the rural edge) is
//!   in the top `cover` share of the habitat
//! - **under trees**: a short way from a wild tree
//! - **clumped** (veins, stone): knots on the rock the habitat names
//! - **along water** (clay): a bank strip beside a river, a stream or a lake
//! - **shallows** (reeds): in the water, just inside a lake's shore or a slow
//!   river's bank, with a bank a walker can reach beside it to gather from
//!
//! A habitat can keep to the even or the odd massifs (`land::massif_at`), so
//! copper and tin grow on different ranges (#225).
//!
//! On top of [`Ground::open`] (roads and trails, lots, parks), every wild node
//! is [`MOVE_CLEAR`] from the wild water (clay [`BANK_CLEAR`]) and from the
//! world's edge, [`MIN_GAP`] past the rural edge ([`Land::from`]), on ground no steeper than `ground::CLIFF_SLOPE`, and on
//! ground a walker can reach from the rural edge ([`Land::reached`]): by
//! straight lines the server walks, tested as the Flint client's walk graph
//! tests them (`ground::slope_lets_through`), so no node stands in a pocket
//! walled by crests (#347), or past a face the graph won't climb (#520, #567).
//!
//! **Quality.** A wild node's base quality is drawn from its spawn row's floor
//! up to a ceiling that rises with depth and height, by `wilds.toml`'s
//! quality table (`WildQualityDef::ceiling`).

use capitol_protocol::fmath::StableMath;
use capitol_protocol::land::{height_at, massif_at, slope_at, Relief};
use capitol_protocol::rules::ground::{self, Footing, CLIFF_SLOPE};
use capitol_protocol::rules::resources::Cluster;
use capitol_protocol::rules::{
    Frame, Zone, EXTRACTION_RANGE,
};
use capitol_protocol::terrain::Lake;

use super::{
    key, offset, round_tenth, Ground, LandTest, NodeSeed, PointGrid, SegmentGrid, SpawnTable, SplitMix64, Woods,
    BANK_CLEAR, BANK_DEPTH, CLUMP_REACH, MIN_GAP, MOVE_CLEAR, TRIES, UNDER_TREES,
};
use crate::content::{self, HabitatDef};
use crate::models::terrain::TerrainConfig;

/// The land grid's cell, in world units.
pub const CELL: f64 = 25.0;

/// How far inside a lake's shore its shallows run, from and to.
pub const LAKE_SHALLOWS: (f64, f64) = (4.0, 14.0);

/// How far inside a river's bank its margin runs, at most; never past a
/// third of the way to the middle.
pub const RIVER_MARGIN: f64 = 4.0;

/// The steepest a river's surface falls (rise over run) where it's slow
/// enough for reeds.
pub const SLOW_RIVER: f64 = 0.02;

/// How far out of the water, past the edge, a shallows node's bank is: where
/// a walker stands to gather it.
pub const SHALLOWS_BANK: f64 = 6.0;

/// The wilds' area in `frame`, in km².
fn wild_km2(frame: &Frame) -> f64 {
    let (inner, outer) = frame.radii_of(Zone::Wilderness);
    std::f64::consts::PI * (outer * outer - inner * inner) / 1e6
}

/// One cell of the land grid.
#[derive(Debug, Clone, Copy, Default)]
struct LandCell {
    height: f32,
    slope: f32,
    /// Sampled: the cell is out past the rural edge's foot.
    sampled: bool,
    /// Under a river or lake a walker can't cross here.
    blocked: bool,
    /// A walker can get here from the rural edge.
    reached: bool,
}

/// The wilds, sampled on a grid of [`CELL`] for finding the ground each
/// habitat wants, with the wild water indexed for the clearances. They're
/// the wilds of the terrain's land's frame (`LandParams::frame`), so a world
/// on other rings has its nodes follow them (PV2, #454).
pub struct Land<'a> {
    terrain: &'a TerrainConfig,
    relief: Relief,
    cells: Vec<LandCell>,
    cols: usize,
    /// The banks of every river and stream, and every lake's shore.
    water: SegmentGrid,
    lakes: Vec<(&'a Lake, (f64, f64), (f64, f64))>,
}

impl<'a> Land<'a> {
    pub fn new(terrain: &'a TerrainConfig) -> Self {
        let cols = (terrain.land.frame.world_size() / CELL).ceil() as usize;
        let mut land = Self {
            terrain,
            relief: Relief::new(&terrain.land),
            cells: vec![LandCell::default(); cols * cols],
            cols,
            water: SegmentGrid::new(),
            lakes: Vec::new(),
        };
        land.sample();
        land.index_water();
        land.block_water();
        land.flood();
        land
    }

    /// The rings the wilds are laid in.
    fn frame(&self) -> &Frame {
        &self.terrain.land.frame
    }

    /// How far `(x, y)` is from the centre: `stable_hypot`, as the plan was
    /// pinned with.
    fn radius(&self, x: f64, y: f64) -> f64 {
        let (cx, cy) = self.frame().center;
        (x - cx).stable_hypot(y - cy)
    }

    fn centre(&self, k: usize) -> (f64, f64) {
        (((k % self.cols) as f64 + 0.5) * CELL, ((k / self.cols) as f64 + 0.5) * CELL)
    }

    fn cell_at(&self, (x, y): (f64, f64)) -> Option<usize> {
        let size = self.frame().world_size();
        if !(0.0..size).contains(&x) || !(0.0..size).contains(&y) {
            return None;
        }
        Some((y / CELL) as usize * self.cols + (x / CELL) as usize)
    }

    /// Heights at every cell from just inside the rural edge out, and the
    /// slope there from its neighbours' (central differences over two cells).
    fn sample(&mut self) {
        let (lo, hi) = (self.frame().rural_edge() - 2.0 * CELL, self.frame().world_edge() + 2.0 * CELL);
        for k in 0..self.cells.len() {
            let (x, y) = self.centre(k);
            let r = self.radius(x, y);
            if (lo..=hi).contains(&r) {
                self.cells[k].height = self.relief.height(x, y) as f32;
                self.cells[k].sampled = true;
            }
        }
        let cols = self.cols as i64;
        for k in 0..self.cells.len() {
            if !self.cells[k].sampled {
                continue;
            }
            let (i, j) = ((k as i64) % cols, (k as i64) / cols);
            let h = |di: i64, dj: i64| {
                let (a, b) = (i + di, j + dj);
                ((0..cols).contains(&a) && (0..cols).contains(&b))
                    .then(|| self.cells[(b * cols + a) as usize])
                    .filter(|c| c.sampled)
                    .map(|c| c.height as f64)
            };
            let here = self.cells[k].height as f64;
            let d = |minus: Option<f64>, plus: Option<f64>| match (minus, plus) {
                (Some(m), Some(p)) => (p - m) / (2.0 * CELL),
                (None, Some(p)) => (p - here) / CELL,
                (Some(m), None) => (here - m) / CELL,
                (None, None) => 0.0,
            };
            let (gx, gy) = (d(h(-1, 0), h(1, 0)), d(h(0, -1), h(0, 1)));
            self.cells[k].slope = gx.stable_hypot(gy) as f32;
        }
    }

    fn index_water(&mut self) {
        for course in self.terrain.rivers.iter().chain(&self.terrain.streams) {
            for w in course.points.windows(2) {
                self.water.insert((w[0].x, w[0].y), (w[1].x, w[1].y), w[0].width.max(w[1].width) / 2.0);
            }
        }
        for lake in &self.terrain.lakes {
            for ring in &lake.rings {
                for i in 0..ring.len() {
                    self.water.insert(ring[i], ring[(i + 1) % ring.len()], 0.0);
                }
            }
            let (min, max) = super::bounds(lake.rings.iter().flatten().copied());
            self.lakes.push((lake, min, max));
        }
    }

    /// Marks the cells a walker can't stand in: a lake, or a river away from
    /// its fords and the roads over it. A river is widened by most of a cell,
    /// so the flood can't slip across one between two cells' centres.
    fn block_water(&mut self) {
        let walk = ground::Ground::new(
            &self.terrain.land,
            &self.terrain.rivers,
            &self.terrain.streams,
            &self.terrain.lakes,
            &self.terrain.fords,
            &self.terrain.roads,
        );
        for k in 0..self.cells.len() {
            if self.cells[k].sampled {
                let (x, y) = self.centre(k);
                self.cells[k].blocked = matches!(walk.footing(x, y), Footing::Blocked(_));
            }
        }
        for river in &self.terrain.rivers {
            for w in river.points.windows(2) {
                let (a, b) = ((w[0].x, w[0].y), (w[1].x, w[1].y));
                let reach = w[0].width.max(w[1].width) / 2.0 + 0.75 * CELL;
                let (min, max) = super::bounds([a, b].into_iter());
                let (i0, j0) = (((min.0 - reach) / CELL).floor().max(0.0) as usize, ((min.1 - reach) / CELL).floor().max(0.0) as usize);
                let (i1, j1) = (
                    (((max.0 + reach) / CELL).floor() as usize).min(self.cols - 1),
                    (((max.1 + reach) / CELL).floor() as usize).min(self.cols - 1),
                );
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let k = j * self.cols + i;
                        let (t, d) = nearest_on_segment(a, b, self.centre(k));
                        if d < reach {
                            let on = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
                            if matches!(walk.footing(on.0, on.1), Footing::Blocked(_)) {
                                self.cells[k].blocked = true;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Whether the slope lets a walker through on the straight line from `a`
    /// to `b`, as the server's steps would ([`ground::slope_lets_through`]).
    fn climbs(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        ground::slope_lets_through(a, b, |x, y| self.relief.slope(x, y))
    }

    /// Whether a walker can stand at cell `k`'s centre, for the flood.
    fn standable(&self, k: usize) -> bool {
        let (x, y) = self.centre(k);
        self.cells[k].sampled && !self.cells[k].blocked && self.radius(x, y) <= self.frame().world_edge()
    }

    /// Floods out from the rural edge through the cells a walker can stand
    /// in (not under the water, nor past the world's edge, round which a
    /// flood would get out of a pocket the edge closes), eight ways, along each line between two
    /// cells' centres that the slope lets through. A cell's own slope says
    /// nothing of the crests between: the ridged land has crests steeper
    /// than the cliff for under a unit, and where they close round a hollow
    /// nothing walks in (#347).
    ///
    /// What's reached doesn't hang on the order the cells are looked at, so
    /// it goes a ring of cells at a time, with each ring's lines tested on
    /// every core: the lines cost a hundred times the grid's sampling.
    fn flood(&mut self) {
        let mut ring = Vec::new();
        for k in 0..self.cells.len() {
            let (x, y) = self.centre(k);
            let r = self.radius(x, y);
            if r < self.frame().rural_edge() && self.standable(k) {
                self.cells[k].reached = true;
                ring.push(k);
            }
        }
        let cols = self.cols as i64;
        while !ring.is_empty() {
            let steps: Vec<(usize, usize)> = ring
                .iter()
                .flat_map(|&k| {
                    let (i, j) = ((k as i64) % cols, (k as i64) / cols);
                    NEIGHBOURS.into_iter().filter_map(move |(di, dj)| {
                        let (a, b) = (i + di, j + dj);
                        ((0..cols).contains(&a) && (0..cols).contains(&b)).then_some((k, (b * cols + a) as usize))
                    })
                })
                .filter(|&(_, n)| !self.cells[n].reached && self.standable(n))
                .collect();
            let climbs = on_every_core(&steps, |&(k, n)| self.climbs(self.centre(k), self.centre(n)));
            ring.clear();
            for (&(_, n), climbs) in steps.iter().zip(climbs) {
                if climbs && !self.cells[n].reached {
                    self.cells[n].reached = true;
                    ring.push(n);
                }
            }
        }
    }

    /// The radius the wilds' nodes start at: [`MIN_GAP`] past the rural edge,
    /// so they keep their gap from the rural nodes without looking at them
    /// (#340).
    pub fn from(&self) -> f64 {
        self.frame().rural_edge() + MIN_GAP
    }

    /// Whether `at` is past the rural edge, where the wilds' nodes look at
    /// what's already there.
    pub fn past_the_rural_edge(&self, at: (f64, f64)) -> bool {
        self.radius(at.0, at.1) >= self.frame().rural_edge()
    }

    /// Whether a walker from the rural edge can reach `at`: its cell is
    /// reached, and the slope lets a walker through from the centre of that
    /// cell or of a reached one beside it.
    pub fn reached(&self, at: (f64, f64)) -> bool {
        let Some(k) = self.cell_at(at) else { return false };
        if !self.cells[k].reached {
            return false;
        }
        let cols = self.cols as i64;
        let (i, j) = ((k as i64) % cols, (k as i64) / cols);
        std::iter::once((0, 0)).chain(NEIGHBOURS).any(|(di, dj)| {
            let (a, b) = (i + di, j + dj);
            (0..cols).contains(&a)
                && (0..cols).contains(&b)
                && self.cells[(b * cols + a) as usize].reached
                && self.climbs(self.centre((b * cols + a) as usize), at)
        })
    }

    /// How far `at` is from the wild water's edge, up to [`MOVE_CLEAR`];
    /// negative on a lake.
    pub fn water_clearance(&self, at: (f64, f64)) -> f64 {
        let on_lake = self.lakes.iter().any(|&(lake, min, max)| {
            (min.0..=max.0).contains(&at.0) && (min.1..=max.1).contains(&at.1) && lake.contains(at.0, at.1)
        });
        if on_lake { -1.0 } else { self.water.clearance(at, MOVE_CLEAR) }
    }

    /// Whether `at` is in the wild water: a lake, a river or a stream.
    pub fn in_the_water(&self, at: (f64, f64)) -> bool {
        self.water_clearance(at) < 0.0
    }

    /// The reached cells whose sampled land suits `habitat`, by centre.
    fn suited(&self, habitat: &HabitatDef) -> Vec<(f64, f64)> {
        (0..self.cells.len())
            .filter(|&k| self.cells[k].reached)
            .map(|k| (k, self.centre(k)))
            .filter(|&(k, (x, y))| {
                let past = self.radius(x, y) - self.frame().rural_edge();
                self.frame().zone_at(x, y) == Zone::Wilderness
                    && habitat.suits(past, self.cells[k].height as f64, self.cells[k].slope as f64)
                    && (habitat.massifs.is_none() || habitat.on_massif(massif_at(x, y, &self.terrain.land)))
            })
            .map(|(_, p)| p)
            .collect()
    }

    /// What the flood reached, kept for the game (#522): the land without
    /// the terrain it borrows.
    pub fn into_reach(self) -> Reach {
        Reach {
            frame: *self.frame(),
            relief: self.relief,
            cols: self.cols,
            reached: self.cells.iter().map(|c| c.reached).collect(),
        }
    }
}

/// How many cells round a walker [`Reach::way_out`] looks for reached ground
/// it can walk to: a walker who got where it stands on foot has some within
/// a cell or two.
const WALK_OUT: i64 = 3;
/// How far [`Reach::way_out`] looks for reached ground to move a stranded
/// walker to, in cells: past any pocket of the ranges.
const LIFT_OUT: i64 = 40;

/// The cells of the wilds a walker can reach from the rural edge, as
/// [`Land`]'s flood found them, owned so the game can keep them (#522). A
/// walker past the rural edge who can't walk to any of them is stranded in
/// a pocket walled by crests, and [`Reach::way_out`] finds it the nearest
/// reached ground.
pub struct Reach {
    relief: Relief,
    frame: Frame,
    cols: usize,
    reached: Vec<bool>,
}

/// Where a walker stands, as [`Reach::way_out`] finds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WayOut {
    /// It can walk out: inside the rural edge, or with a straight line the
    /// slope lets through to a reached cell's centre near it.
    Walks,
    /// Stranded: the centre of the nearest reached cell, to move it to.
    Lift((f64, f64)),
    /// Stranded, with no reached ground within [`LIFT_OUT`] cells: off the
    /// land's grid, or somewhere the flood never got near.
    Lost,
}

impl Reach {
    /// Whether a walker at `at` can walk out to the reached ground, and if
    /// not, where the nearest is. The line out is tested from `at` (a walk
    /// may climb down off a crag that it couldn't climb up) to the centres
    /// of the reached cells within [`WALK_OUT`].
    pub fn way_out(&self, at: (f64, f64)) -> WayOut {
        let (cx, cy) = self.frame.center;
        if (at.0 - cx).stable_hypot(at.1 - cy) < self.frame.rural_edge() {
            return WayOut::Walks;
        }
        let size = self.frame.world_size();
        if !(0.0..size).contains(&at.0) || !(0.0..size).contains(&at.1) {
            return WayOut::Lost;
        }
        let (i, j) = ((at.0 / CELL) as i64, (at.1 / CELL) as i64);
        let cols = self.cols as i64;
        let reached = |(a, b): &(i64, i64)| (0..cols).contains(a) && (0..cols).contains(b) && self.reached[(b * cols + a) as usize];
        let centre = |(a, b): (i64, i64)| ((a as f64 + 0.5) * CELL, (b as f64 + 0.5) * CELL);
        let off = |p: (f64, f64)| (p.0 - at.0).stable_hypot(p.1 - at.1);
        // The cells of the square ring `r` round `at`'s, each once
        let ring = move |r: i64| {
            (-r..=r).flat_map(move |d| {
                let sides = [(d, -r), (d, r), (-r, d), (r, d)];
                let n = if r == 0 { 1 } else if d.abs() == r { 2 } else { 4 };
                sides.into_iter().take(n).map(move |(di, dj)| (i + di, j + dj))
            })
        };
        let near: Vec<(f64, f64)> = (0..=WALK_OUT).flat_map(ring).filter(reached).map(centre).collect();
        if near.iter().any(|&p| ground::slope_lets_through(at, p, |x, y| self.relief.slope(x, y))) {
            return WayOut::Walks;
        }
        let mut best = near.iter().map(|&p| (off(p), p)).min_by(|a, b| a.0.total_cmp(&b.0));
        for r in WALK_OUT + 1..=LIFT_OUT {
            // Every cell of this ring and past it is further than this
            if best.is_some_and(|(d, _)| d <= (r - 1) as f64 * CELL) {
                break;
            }
            for p in ring(r).filter(reached).map(centre) {
                if best.is_none_or(|(d, _)| off(p) < d) {
                    best = Some((off(p), p));
                }
            }
        }
        best.map_or(WayOut::Lost, |(_, p)| WayOut::Lift(p))
    }
}

/// A cell's eight neighbours, as steps along the grid.
const NEIGHBOURS: [(i64, i64); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// `f` of each of `items`, in order, shared out over the cores.
fn on_every_core<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    if cores == 1 || items.len() < 256 {
        return items.iter().map(f).collect();
    }
    let chunk = items.len().div_ceil(cores);
    std::thread::scope(|scope| {
        let f = &f;
        let parts: Vec<_> = items.chunks(chunk).map(|part| scope.spawn(move || part.iter().map(f).collect::<Vec<R>>())).collect();
        parts.into_iter().flat_map(|part| part.join().expect("a flood's thread")).collect()
    })
}

fn nearest_on_segment(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (t, (a.0 + t * dx - p.0).stable_hypot(a.1 + t * dy - p.1))
}

/// Plans the wilds' nodes for `tables` (woods first) and adds them to `seeds`,
/// keeping them [`MIN_GAP`] from everything in `taken`.
pub(super) fn plan(
    land: &Land,
    ground: &Ground,
    tables: &[&SpawnTable],
    taken: &mut PointGrid,
    seed: u64,
    land_test: LandTest,
    seeds: &mut Vec<NodeSeed>,
) {
    let rows: Vec<(&SpawnTable, &HabitatDef, i32, f64)> = tables
        .iter()
        .filter_map(|t| {
            let spawn = t.spawns.iter().find(|s| s.zone == Zone::Wilderness.as_str() && s.density > 0.0)?;
            let floor = t.quality.0.max(spawn.quality_floor.unwrap_or(0)).min(t.quality.1);
            Some((*t, t.habitat.as_ref()?, floor, spawn.density))
        })
        .collect();
    if rows.is_empty() {
        return;
    }
    let terrain = land.terrain;
    let mut trees: Vec<(f64, f64)> = Vec::new();
    for (table, habitat, floor, density) in rows {
        let count = (density * wild_km2(land.frame())).round() as usize;
        let mut rng = SplitMix64::stream(seed, &[key(&table.id), Zone::Wilderness.ring_order() as u64]);
        let mut place = Placing { ground, land, taken, table, habitat, land_test };
        let spots = match table.cluster {
            Cluster::Woods => {
                let suited = land.suited(habitat);
                let cover = habitat.cover.unwrap_or(1.0);
                let field = habitat.woods_of.as_deref().unwrap_or(&table.id);
                let woods = Woods::over(seed, field, suited.iter().copied(), cover);
                let wooded: Vec<(f64, f64)> = suited.into_iter().filter(|&p| woods.covers(p)).collect();
                place.scatter(&mut rng, count, &wooded)
            }
            Cluster::UnderTrees if !trees.is_empty() => place.near(&mut rng, count, &trees),
            Cluster::AlongWater => place.banks(&mut rng, count),
            Cluster::Shallows => place.shallows(&mut rng, count),
            Cluster::Clumped => place.clumps(&mut rng, count, &land.suited(habitat)),
            _ => place.scatter(&mut rng, count, &land.suited(habitat)),
        };
        if table.cluster == Cluster::Woods {
            trees.extend(&spots);
        }
        let quality = &content::wilds().quality;
        let mut ids = SplitMix64::stream(seed, &[key(&table.id), Zone::Wilderness.ring_order() as u64, u64::MAX]);
        for (x, y) in spots {
            let past = land.radius(x, y) - land.frame().rural_edge();
            let ceiling = quality.ceiling(table.quality.1, floor, past, height_at(x, y, &terrain.land));
            let base_quality = floor + rng.below((ceiling - floor + 1) as u64) as i32;
            let mut bytes = [0u8; 16];
            bytes[..8].copy_from_slice(&ids.next_u64().to_le_bytes());
            bytes[8..].copy_from_slice(&ids.next_u64().to_le_bytes());
            seeds.push(NodeSeed {
                id: uuid::Builder::from_random_bytes(bytes).into_uuid(),
                resource_type: table.id.clone(),
                x,
                y,
                base_quality,
            });
        }
    }
}

/// One resource's placing in the wilds.
struct Placing<'a, 'b> {
    ground: &'a Ground<'a>,
    land: &'a Land<'a>,
    taken: &'b mut PointGrid,
    table: &'b SpawnTable,
    habitat: &'b HabitatDef,
    land_test: LandTest<'b>,
}

impl Placing<'_, '_> {
    /// Takes `at` if it fits: open ground, clear of the water and the edge,
    /// reachable, walkable, suited to the habitat there, and clear of every
    /// other node.
    fn try_take(&mut self, at: (f64, f64), bank: bool) -> Option<(f64, f64)> {
        let at = (round_tenth(at.0), round_tenth(at.1));
        let frame = self.land.frame();
        let r = self.land.radius(at.0, at.1);
        let land = &self.land.terrain.land;
        let water = if bank { BANK_CLEAR } else { MOVE_CLEAR };
        let fits = (self.land.from()..=frame.world_edge() - MOVE_CLEAR).contains(&r)
            && frame.zone_at(at.0, at.1) == Zone::Wilderness
            && self.ground.open(at, MOVE_CLEAR, false)
            && self.land.water_clearance(at) >= water
            && self.taken.clear_of(at, MIN_GAP)
            && {
                let slope = slope_at(at.0, at.1, land);
                slope <= CLIFF_SLOPE && self.habitat.suits(r - frame.rural_edge(), height_at(at.0, at.1, land), slope)
            }
            && (self.habitat.massifs.is_none() || self.habitat.on_massif(massif_at(at.0, at.1, land)))
            && (self.land_test)(self.table, Zone::Wilderness, at)
            // Last, as it walks lines over the land (#347).
            && self.land.reached(at);
        fits.then(|| {
            self.taken.insert(at);
            at
        })
    }

    /// Takes `at`, in the water, if it fits: open ground but for the water,
    /// clear of the world's edge and every other node, suited to the
    /// habitat, and with `bank` beside it on ground a walker can reach and
    /// stand on, out of the water and within gathering range.
    fn try_take_wet(&mut self, at: (f64, f64), bank: (f64, f64)) -> Option<(f64, f64)> {
        let at = (round_tenth(at.0), round_tenth(at.1));
        let frame = self.land.frame();
        let r = self.land.radius(at.0, at.1);
        let land = &self.land.terrain.land;
        let fits = (self.land.from()..=frame.world_edge() - MOVE_CLEAR).contains(&r)
            && frame.zone_at(at.0, at.1) == Zone::Wilderness
            && frame.zone_at(bank.0, bank.1) == Zone::Wilderness
            && (at.0 - bank.0).stable_hypot(at.1 - bank.1) <= EXTRACTION_RANGE
            && self.land.in_the_water(at)
            && !self.land.in_the_water(bank)
            && slope_at(bank.0, bank.1, land) <= CLIFF_SLOPE
            && self.ground.open(at, MOVE_CLEAR, false)
            && self.taken.clear_of(at, MIN_GAP)
            && self.habitat.suits(r - frame.rural_edge(), height_at(at.0, at.1, land), slope_at(at.0, at.1, land))
            && (self.land_test)(self.table, Zone::Wilderness, at)
            && self.land.reached(bank);
        fits.then(|| {
            self.taken.insert(at);
            at
        })
    }

    fn each(&mut self, count: usize, mut spot: impl FnMut(&mut Self) -> Option<(f64, f64)>) -> Vec<(f64, f64)> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(p) = (0..TRIES).find_map(|_| spot(self)) {
                out.push(p);
            }
        }
        out
    }

    /// A spot somewhere in one of `cells`.
    fn in_cells(rng: &mut SplitMix64, cells: &[(f64, f64)]) -> (f64, f64) {
        let c = cells[rng.below(cells.len() as u64) as usize];
        (c.0 + rng.range(-0.5, 0.5) * CELL, c.1 + rng.range(-0.5, 0.5) * CELL)
    }

    fn scatter(&mut self, rng: &mut SplitMix64, count: usize, cells: &[(f64, f64)]) -> Vec<(f64, f64)> {
        if cells.is_empty() {
            return Vec::new();
        }
        self.each(count, |p| p.try_take(Self::in_cells(rng, cells), false))
    }

    fn near(&mut self, rng: &mut SplitMix64, count: usize, anchors: &[(f64, f64)]) -> Vec<(f64, f64)> {
        self.each(count, |p| {
            let a = anchors[rng.below(anchors.len() as u64) as usize];
            p.try_take(offset(rng, a, UNDER_TREES.0, UNDER_TREES.1), false)
        })
    }

    /// Knots of one to five round centres in `cells`, as in the city.
    fn clumps(&mut self, rng: &mut SplitMix64, count: usize, cells: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let mut out = Vec::with_capacity(count);
        if cells.is_empty() {
            return out;
        }
        let mut misses = 0;
        while out.len() < count && misses < TRIES {
            let Some(centre) = (0..TRIES).find_map(|_| self.try_take(Self::in_cells(rng, cells), false)) else {
                break;
            };
            out.push(centre);
            let size = (1 + rng.below(5) as usize).min(count - out.len() + 1);
            for _ in 1..size {
                match (0..TRIES / 4).find_map(|_| self.try_take(offset(rng, centre, MIN_GAP, CLUMP_REACH), false)) {
                    Some(p) => out.push(p),
                    None => misses += 1,
                }
            }
        }
        out
    }

    /// On a bank strip [`BANK_CLEAR`] to [`BANK_CLEAR`] + [`BANK_DEPTH`] from
    /// the water's edge: beside a river or stream, either side, or outside a
    /// lake's shore. Stretches are picked by their length.
    fn banks(&mut self, rng: &mut SplitMix64, count: usize) -> Vec<(f64, f64)> {
        let terrain = self.land.terrain;
        let mut edges: Vec<((f64, f64), (f64, f64), f64)> = Vec::new();
        for course in terrain.rivers.iter().chain(&terrain.streams) {
            for w in course.points.windows(2) {
                edges.push(((w[0].x, w[0].y), (w[1].x, w[1].y), w[0].width.max(w[1].width) / 2.0));
            }
        }
        for lake in &terrain.lakes {
            for ring in &lake.rings {
                for i in 0..ring.len() {
                    edges.push((ring[i], ring[(i + 1) % ring.len()], 0.0));
                }
            }
        }
        let lengths: Vec<f64> = edges.iter().map(|(a, b, _)| (b.0 - a.0).stable_hypot(b.1 - a.1)).collect();
        let total: f64 = lengths.iter().sum();
        if total <= 0.0 {
            return Vec::new();
        }
        self.each(count, |p| {
            let mut pick = rng.range(0.0, total);
            let k = lengths.iter().position(|&l| {
                pick -= l;
                pick < 0.0
            });
            let (a, b, half) = edges[k.unwrap_or(edges.len() - 1)];
            let t = rng.range(0.0, 1.0);
            let len = (b.0 - a.0).stable_hypot(b.1 - a.1).max(1e-9);
            let side = if rng.below(2) == 0 { 1.0 } else { -1.0 };
            let (nx, ny) = (side * -(b.1 - a.1) / len, side * (b.0 - a.0) / len);
            let d = half + rng.range(BANK_CLEAR, BANK_CLEAR + BANK_DEPTH);
            let at = (a.0 + t * (b.0 - a.0) + nx * d, a.1 + t * (b.1 - a.1) + ny * d);
            p.try_take(at, true)
        })
    }
}

impl Placing<'_, '_> {
    /// In the shallows: [`LAKE_SHALLOWS`] inside a lake's shore, or up to
    /// [`RIVER_MARGIN`] inside a river's bank where it's wide and slow (its
    /// surface falls no faster than [`SLOW_RIVER`]). Each with its bank
    /// [`SHALLOWS_BANK`] out of the water. Stretches are picked by length.
    fn shallows(&mut self, rng: &mut SplitMix64, count: usize) -> Vec<(f64, f64)> {
        let terrain = self.land.terrain;
        // (a, b, half-width, inset range). A lake's shore has no width, and
        // which side is the water is found by looking.
        let mut edges: Vec<((f64, f64), (f64, f64), f64, (f64, f64))> = Vec::new();
        for river in &terrain.rivers {
            for w in river.points.windows(2) {
                let len = (w[1].x - w[0].x).stable_hypot(w[1].y - w[0].y);
                let half = w[0].width.min(w[1].width) / 2.0;
                let fall = (w[0].surface - w[1].surface).abs() / len.max(1e-9);
                let inset = RIVER_MARGIN.min(half / 3.0);
                if fall <= SLOW_RIVER && inset >= 1.0 {
                    edges.push(((w[0].x, w[0].y), (w[1].x, w[1].y), half, (0.5, inset)));
                }
            }
        }
        for lake in &terrain.lakes {
            for ring in &lake.rings {
                for i in 0..ring.len() {
                    edges.push((ring[i], ring[(i + 1) % ring.len()], 0.0, LAKE_SHALLOWS));
                }
            }
        }
        let lengths: Vec<f64> = edges.iter().map(|(a, b, ..)| (b.0 - a.0).stable_hypot(b.1 - a.1)).collect();
        let total: f64 = lengths.iter().sum();
        if total <= 0.0 {
            return Vec::new();
        }
        self.each(count, |p| {
            let mut pick = rng.range(0.0, total);
            let k = lengths.iter().position(|&l| {
                pick -= l;
                pick < 0.0
            });
            let (a, b, half, (lo, hi)) = edges[k.unwrap_or(edges.len() - 1)];
            let t = rng.range(0.0, 1.0);
            let len = (b.0 - a.0).stable_hypot(b.1 - a.1).max(1e-9);
            let side = if rng.below(2) == 0 { 1.0 } else { -1.0 };
            let inset = rng.range(lo, hi);
            let on = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            // `d` out from `on`, square to the stretch, on `side`.
            let out = |side: f64, d: f64| (on.0 - side * (b.1 - a.1) / len * d, on.1 + side * (b.0 - a.0) / len * d);
            let (at, bank) = if half > 0.0 {
                // From the river's middle out towards one bank.
                (out(side, half - inset), out(side, half + SHALLOWS_BANK))
            } else {
                // From a lake's shore in, whichever side the water is.
                let side = if p.land.in_the_water(out(side, inset)) { side } else { -side };
                (out(side, inset), out(side, -SHALLOWS_BANK))
            };
            p.try_take_wet(at, bank)
        })
    }
}

#[cfg(test)]
mod tests {
    use capitol_protocol::rules::{CENTER_X, CENTER_Y};
    use super::*;

    /// Prints the wilds' land and what the default plan lays there, for tuning
    /// `wilds.toml`: `cargo test --release wild_node_report -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn wild_node_report() {
        let t0 = std::time::Instant::now();
        let terrain = TerrainConfig::get_default();
        println!("terrain {:.1}s", t0.elapsed().as_secs_f64());
        let t1 = std::time::Instant::now();
        let land = Land::new(&terrain);
        println!("land grid {:.2}s", t1.elapsed().as_secs_f64());
        let wild: Vec<usize> = (0..land.cells.len())
            .filter(|&k| {
                let (x, y) = land.centre(k);
                Zone::from_position(x, y) == Zone::Wilderness
            })
            .collect();
        let reached = wild.iter().filter(|&&k| land.cells[k].reached).count();
        println!("wild cells {}, reached {} ({:.0}%)", wild.len(), reached, 100.0 * reached as f64 / wild.len() as f64);
        for (lo, hi) in [(0.0, 50.0), (50.0, 150.0), (150.0, 250.0), (250.0, 380.0), (380.0, 500.0), (500.0, 2000.0)] {
            let n = |s0: f64, s1: f64| {
                wild.iter()
                    .filter(|&&k| {
                        let c = land.cells[k];
                        c.reached && (lo..hi).contains(&(c.height as f64)) && (s0..s1).contains(&(c.slope as f64))
                    })
                    .count()
            };
            println!(
                "height {lo:>4}-{hi:<4}: slope <0.2 {:>6}  0.2-0.45 {:>6}  0.45-0.8 {:>6}  0.8-1.2 {:>6}",
                n(0.0, 0.2), n(0.2, 0.45), n(0.45, 0.8), n(0.8, 1.21)
            );
        }
        for h in &content::wilds().habitat {
            println!("{}: {} suited cells", h.resource, land.suited(h).len());
        }
        let tables: Vec<SpawnTable> = content::load().unwrap().resources.iter().map(|r| SpawnTable::from(&r.to_type())).collect();
        let t2 = std::time::Instant::now();
        let plan = super::super::plan(&terrain, &tables, &[], 1);
        println!("plan {:.2}s", t2.elapsed().as_secs_f64());
        for t in &tables {
            let Some(spawn) = t.spawns.iter().find(|s| s.zone == "wilderness") else { continue };
            let got: Vec<&NodeSeed> = plan.iter().filter(|s| s.resource_type == t.id && Zone::from_position(s.x, s.y) == Zone::Wilderness).collect();
            let mean = got.iter().map(|s| s.base_quality as f64).sum::<f64>() / got.len().max(1) as f64;
            println!("{}: {} of {:.0}, mean quality {mean:.1}", t.id, got.len(), spawn.density * wild_km2(&Frame::TABLE));
        }
        let wild: Vec<&NodeSeed> = plan.iter().filter(|s| Zone::from_position(s.x, s.y) == Zone::Wilderness).collect();
        let r = |s: &NodeSeed| (s.x - CENTER_X).stable_hypot(s.y - CENTER_Y);
        for (lo, hi) in [(4600.0, 5100.0), (5100.0, 5800.0), (5800.0, 6800.0)] {
            let band: Vec<&&NodeSeed> = wild.iter().filter(|s| (lo..hi).contains(&r(s))).collect();
            let mean = band.iter().map(|s| s.base_quality as f64).sum::<f64>() / band.len().max(1) as f64;
            println!("r{lo}-{hi}: {} nodes, mean quality {mean:.1}", band.len());
        }
        let mut sectors = [0; 8];
        for s in plan.iter().filter(|s| s.resource_type == "iron_vein") {
            let a = (s.y - CENTER_Y).stable_atan2(s.x - CENTER_X).to_degrees().rem_euclid(360.0);
            sectors[((a + 22.5) / 45.0) as usize % 8] += 1;
        }
        println!("iron veins by sector from east, clockwise: {sectors:?}");
        for ore in ["copper_vein", "tin_vein"] {
            let mut by = [0; 8];
            for s in plan.iter().filter(|s| s.resource_type == ore) {
                by[massif_at(s.x, s.y, &terrain.land).unwrap_or(0) as usize % 8] += 1;
            }
            println!("{ore} by massif: {by:?}");
        }
        let in_lakes = plan.iter().filter(|s| s.resource_type == "reed_bed" && terrain.lakes.iter().any(|l| l.contains(s.x, s.y))).count();
        println!("reed beds in lakes: {in_lakes}, the rest in rivers");
    }

    /// A pocket of the east-south-east massif, walled by crests steeper than
    /// the cliff for under a unit: the Flint client's walk graph finds no way
    /// in or out (`a_pocket_walled_by_crests_has_no_way_in_or_out`), and the
    /// land's flood, along the lines between its cells, doesn't reach it
    /// (#347). Before, it tested only the cells' centres and stepped over the
    /// crests.
    #[test]
    fn a_pocket_walled_by_crests_is_not_reached() {
        let terrain = TerrainConfig::default_world();
        let land = Land::new(terrain);
        let pocket = (15329.1, 11083.0);
        assert!(!land.reached(pocket), "{pocket:?} is in a pocket");
        let k = land.cell_at(pocket).unwrap();
        assert!(land.cells[k].sampled && !land.cells[k].blocked && land.cells[k].slope as f64 <= CLIFF_SLOPE);
        assert!(!land.cells[k].reached);
    }

    /// The flood's lines are the walk graph's: a reached cell has a line
    /// the slope lets through from a reached neighbour, or is inside the
    /// rural edge, where the flood starts.
    #[test]
    fn every_reached_cell_is_reached_by_a_line() {
        let terrain = TerrainConfig::default_world();
        let land = Land::new(terrain);
        let cols = land.cols as i64;
        let mut checked = 0;
        for k in (0..land.cells.len()).step_by(37).filter(|&k| land.cells[k].reached) {
            let (x, y) = land.centre(k);
            if land.radius(x, y) < land.frame().rural_edge() {
                continue;
            }
            let (i, j) = ((k as i64) % cols, (k as i64) / cols);
            let by_a_line = NEIGHBOURS.into_iter().any(|(di, dj)| {
                let (a, b) = (i + di, j + dj);
                let n = (b * cols + a) as usize;
                (0..cols).contains(&a) && (0..cols).contains(&b) && land.cells[n].reached && land.climbs(land.centre(n), (x, y))
            });
            assert!(by_a_line, "cell at ({x}, {y}) is reached by no line");
            checked += 1;
        }
        assert!(checked > 1000, "{checked}");
    }

    /// On rings off the table (PV2, #454), here RW1's from before Plots v2's
    /// flip (#459), the wilds' nodes grow in that world's wilds, r4600 to
    /// r6800 round (6800, 6800), as many as their densities ask of that
    /// smaller ring, and their quality's depth is measured from its rural
    /// edge.
    #[test]
    fn the_wilds_follow_their_frame() {
        let terrain = TerrainConfig::rw1();
        let frame = terrain.land.frame;
        assert_eq!((frame.rural_edge(), frame.world_edge()), (4600.0, 6800.0));
        let land = Land::new(terrain);
        assert!(land.cells.iter().enumerate().all(|(k, c)| {
            let (x, y) = land.centre(k);
            !c.sampled || frame.radius(x, y) >= 4600.0 - 2.0 * CELL - 1.0
        }));
        let tables = crate::engine::node_spawn::tests::content_tables();
        let wild: Vec<&SpawnTable> = tables.iter().filter(|t| t.grows_wild()).collect();
        let (ground, mut taken, mut seeds) = (Ground::new(terrain), PointGrid::new(MIN_GAP), Vec::new());
        plan(&land, &ground, &wild, &mut taken, 42, &|_, _, _| true, &mut seeds);
        for t in &wild {
            let spawn = t.spawns.iter().find(|s| s.zone == Zone::Wilderness.as_str()).unwrap();
            let want = (spawn.density * wild_km2(&frame)).round();
            let got = seeds.iter().filter(|s| s.resource_type == t.id).count();
            assert!(got as f64 >= 0.9 * want, "{}: {got} of {want}", t.id);
        }
        for s in &seeds {
            let r = frame.radius(s.x, s.y);
            assert!(r > 4600.0 && r <= 6800.0 - MOVE_CLEAR, "{} at r{r:.0}", s.resource_type);
        }
        // Out where the table's wilds are still plain: inside r6100.
        assert!(seeds.iter().any(|s| frame.radius(s.x, s.y) < 5600.0));
    }
}
