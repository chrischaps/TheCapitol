//! Where the resource nodes grow (E2.3, #162): woods, outcrops, veins and clay,
//! planned from the terrain and the resources' spawn tables (E2.2).
//!
//! [`plan`] is pure and deterministic: the same terrain, tables, existing nodes
//! and seed give the same nodes, ids included. For each resource and each zone
//! it grows in, it places `density × the zone's area` nodes in the resource's
//! [`Cluster`] pattern:
//!
//! - **woods**: patches where a seeded value-noise field is in its top
//!   [`WOODS_COVER`] share of the zone's open ground
//! - **under trees**: a short way from a tree already planned in the zone
//! - **clumped**: knots of a few round a centre
//! - **along water**: on a bank strip beside a canal or the moat, or where
//!   lots run to the kerb, on the open ground nearest the water
//! - **scattered**: anywhere open
//!
//! A pattern with nowhere to go in a zone (no trees, no water) falls back to the
//! zone's parks, and then to scattering. That's how the suburb gets its branches
//! and clay: D5 puts them in the parks, and the suburb has no water.
//!
//! Every node stands on open ground ([`Ground::open`]): in its zone,
//! [`MOVE_CLEAR`] from every road and from the water's kerb (clay only
//! [`BANK_CLEAR`] from the water), [`LOT_MARGIN`] off every lot and out of the
//! parks unless it was put there, and [`MIN_GAP`] from every other node. Those
//! are `node_placement`'s clearances, so a planned node is never in the way.
//!
//! **The wilds** ([`PLACED_BY_THE_LAND`]) are placed after every other zone,
//! from the land: each resource where its habitat in `content/wilds.toml`
//! says, on ground a walker can reach (W6, #199; see [`wilds`]). Placing them
//! last leaves the other zones' nodes, ids and all, as E2.3 planned them.
//!
//! **Not placed here.** Grass is seeded by migrations already
//! ([`SEEDED_BY_MIGRATIONS`]).

use std::collections::HashMap;
use std::f64::consts::TAU;

use capitol_protocol::fmath::StableMath;
use capitol_protocol::rules::resources::{Cluster, Spawn};
use capitol_protocol::rules::{Zone, CENTER_X, CENTER_Y};
use capitol_protocol::terrain::{quad_contains, TerrainPark, TerrainPlot};
use sqlx::PgPool;
use uuid::Uuid;

use crate::content::HabitatDef;
use crate::engine::node_placement::{LOT_MARGIN, MOVE_CLEAR};
use crate::models::geom::SplitMix64;
use crate::models::lot_clearance::WATER_KERB;
use crate::models::terrain::{point_to_segment_distance, TerrainConfig};
use crate::models::{NodeState, ResourceNodeState, ResourceType};

/// Resources the migrations seeded before E2.3. Their spawn rows describe what's
/// there, so the planner leaves them be.
pub const SEEDED_BY_MIGRATIONS: [&str; 1] = ["grass"];

pub mod wilds;

/// Zones whose nodes come from the land (altitude, slope, water) in W6 (#199),
/// placed by [`wilds`] after the rest.
pub const PLACED_BY_THE_LAND: [Zone; 1] = [Zone::Wilderness];

/// The least distance between any two nodes, planned or not.
pub const MIN_GAP: f64 = 12.0;

/// How close to the water's kerb clay may stand. Over `node_placement`'s
/// `KEEP_CLEAR` (15), so a bank node isn't in the way.
pub const BANK_CLEAR: f64 = 18.0;

/// How far past [`BANK_CLEAR`] the bank strip runs.
const BANK_DEPTH: f64 = 30.0;

/// The share of a zone's open ground that is woods.
pub const WOODS_COVER: f64 = 0.08;

/// The value-noise lattice the woods are drawn from, in units (metres).
const WOODS_SCALE: f64 = 260.0;
/// Where the woods' noise field has its origin, from the tower. The field is
/// laid out round the tower, so moving the world's centre doesn't move the
/// woods. This is the 8000 world's corner, where E2.3 planted them, so the
/// default world's trees stayed put when it grew to 12000 (W1, #194).
const WOODS_ORIGIN: (f64, f64) = (-4000.0, -4000.0);

/// How far from its tree a node under trees lies.
const UNDER_TREES: (f64, f64) = (12.0, 45.0);

/// How far from its centre a clump's member lies.
const CLUMP_REACH: f64 = 35.0;

/// Tries per node before a zone gives up on the rest.
const TRIES: usize = 80;

/// Tries per tree in the woods, which a uniform draw hits less often: they
/// cover [`WOODS_COVER`] of the open ground, and since the farms (#290) the
/// open ground is under two thirds of the rural ring, so about 3% of draws
/// land and [`TRIES`] left a tenth of D5's trees unplanted.
const WOODS_TRIES: usize = 4 * TRIES;

/// Points sampled to set the woods' noise threshold.
const WOODS_SAMPLES: usize = 3000;

/// What the planner reads from a resource type.
#[derive(Debug, Clone)]
pub struct SpawnTable {
    pub id: String,
    pub cluster: Cluster,
    pub quality: (i32, i32),
    pub spawns: Vec<Spawn>,
    /// Where it grows in the wilds (`content/wilds.toml`), if it does.
    pub habitat: Option<HabitatDef>,
}

impl From<&ResourceType> for SpawnTable {
    fn from(r: &ResourceType) -> Self {
        Self {
            id: r.id.clone(),
            cluster: Cluster::from_str(&r.cluster).unwrap_or(Cluster::Scattered),
            quality: (r.base_quality_min, r.base_quality_max),
            spawns: r.spawns.0.clone(),
            habitat: crate::content::wilds().habitat.iter().find(|h| h.resource == r.id).cloned(),
        }
    }
}

impl SpawnTable {
    /// The spawn rows [`plan`] places: not a seeded resource's, not the land's.
    fn planned_rows(&self) -> impl Iterator<Item = (Zone, &Spawn)> {
        let seeded = SEEDED_BY_MIGRATIONS.contains(&self.id.as_str());
        self.spawns
            .iter()
            .filter(move |_| !seeded)
            .filter_map(|s| Zone::from_str(&s.zone).map(|z| (z, s)))
            .filter(|(z, s)| !PLACED_BY_THE_LAND.contains(z) && s.density > 0.0)
    }

    /// Whether [`wilds`] places this resource's nodes in the wilds.
    fn grows_wild(&self) -> bool {
        let wild = Zone::Wilderness.as_str();
        !SEEDED_BY_MIGRATIONS.contains(&self.id.as_str())
            && self.habitat.is_some()
            && self.spawns.iter().any(|s| s.zone == wild && s.density > 0.0)
    }

    /// Whether [`plan`] places any of this resource's nodes.
    pub fn is_planned(&self) -> bool {
        self.planned_rows().next().is_some() || self.grows_wild()
    }
}

/// One node the plan puts down.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSeed {
    pub id: Uuid,
    pub resource_type: String,
    pub x: f64,
    pub y: f64,
    pub base_quality: i32,
}

/// An extra test a spot must pass, for W6's land predicates: the resource, the
/// zone and the spot.
pub type LandTest<'a> = &'a dyn Fn(&SpawnTable, Zone, (f64, f64)) -> bool;

/// The nodes `tables` grow on `terrain`, kept [`MIN_GAP`] from `existing`.
pub fn plan(terrain: &TerrainConfig, tables: &[SpawnTable], existing: &[(f64, f64)], seed: u64) -> Vec<NodeSeed> {
    plan_with(terrain, tables, existing, seed, &|_, _, _| true)
}

/// [`plan`], with every spot also passing `land`.
pub fn plan_with(
    terrain: &TerrainConfig,
    tables: &[SpawnTable],
    existing: &[(f64, f64)],
    seed: u64,
    land: LandTest,
) -> Vec<NodeSeed> {
    let ground = Ground::new(terrain);
    // The wild water runs on into the rural ring (#222), so every zone's
    // nodes keep off it, not only the wilds' (#308). Before, startup's fit
    // moved the ones the plan put in it, and a migration wrote the moves.
    let wild_land = wilds::Land::new(terrain);
    let dry = |t: &SpawnTable, z: Zone, at: (f64, f64)| wild_land.water_clearance(at) >= MOVE_CLEAR && land(t, z, at);
    let mut taken = PointGrid::new(MIN_GAP);
    for &p in existing {
        taken.insert(p);
    }
    let mut seeds = Vec::new();
    // Woods first, so what grows under trees has trees to grow under. Then by
    // id, so the game's map of resource types plans as content's list does.
    let mut order: Vec<&SpawnTable> = tables.iter().collect();
    order.sort_by(|a, b| (a.cluster != Cluster::Woods, &a.id).cmp(&(b.cluster != Cluster::Woods, &b.id)));
    let mut trees: Vec<(Zone, (f64, f64))> = Vec::new();

    for &table in &order {
        for (zone, spawn) in table.planned_rows() {
            let (inner, outer) = zone.radii();
            let km2 = std::f64::consts::PI * (outer * outer - inner * inner) / 1e6;
            let count = (spawn.density * km2).round() as usize;
            let mut rng = SplitMix64::stream(seed, &[key(&table.id), zone.ring_order() as u64]);
            let floor = table.quality.0.max(spawn.quality_floor.unwrap_or(0)).min(table.quality.1);
            let in_zone_trees: Vec<(f64, f64)> =
                trees.iter().filter(|(z, _)| *z == zone).map(|&(_, p)| p).collect();
            let mut place = Placing {
                ground: &ground,
                taken: &mut taken,
                zone,
                table,
                land: &dry,
                woods: None,
            };
            let spots = match pattern(table.cluster, zone, &ground, &in_zone_trees) {
                Pattern::Woods => {
                    place.woods = Some(Woods::new(seed, &table.id, zone, &ground, &mut rng));
                    place.scatter(&mut rng, count, false)
                }
                Pattern::UnderTrees => place.near(&mut rng, count, &in_zone_trees),
                Pattern::Clumped => place.clumps(&mut rng, count),
                Pattern::AlongWater => place.banks(&mut rng, count),
                Pattern::Parks => place.in_parks(&mut rng, count),
                Pattern::Scattered => place.scatter(&mut rng, count, false),
            };
            if table.cluster == Cluster::Woods {
                trees.extend(spots.iter().map(|&p| (zone, p)));
            }
            let mut ids = SplitMix64::stream(seed, &[key(&table.id), zone.ring_order() as u64, u64::MAX]);
            for (x, y) in spots {
                let quality = floor + rng.below((table.quality.1 - floor + 1) as u64) as i32;
                let mut bytes = [0u8; 16];
                bytes[..8].copy_from_slice(&ids.next_u64().to_le_bytes());
                bytes[8..].copy_from_slice(&ids.next_u64().to_le_bytes());
                seeds.push(NodeSeed {
                    id: uuid::Builder::from_random_bytes(bytes).into_uuid(),
                    resource_type: table.id.clone(),
                    x,
                    y,
                    base_quality: quality,
                });
            }
        }
    }
    let wild: Vec<&SpawnTable> = order.into_iter().filter(|t| t.grows_wild()).collect();
    wilds::plan(&wild_land, &ground, &wild, &mut taken, seed, land, &mut seeds);
    seeds
}

/// What a [`replan`] did to the game's nodes.
#[derive(Debug, Default)]
pub struct Replan {
    /// The resource types it planned
    pub types: Vec<String>,
    /// Their nodes as they now stand
    pub nodes: Vec<NodeSeed>,
    pub removed: usize,
    pub added: usize,
}

/// Re-plans the planned resources' nodes in `nodes` for `terrain`, after an
/// admin's regenerate. A node the new plan keeps (by id) keeps its state; the
/// rest go, and the new ones arrive available. Other nodes (grass) stay, and
/// the plan keeps clear of them. In memory: [`save_replan`] writes it.
pub fn replan(
    terrain: &TerrainConfig,
    resource_types: &HashMap<String, ResourceType>,
    nodes: &mut HashMap<Uuid, ResourceNodeState>,
    seed: u64,
) -> Replan {
    let tables: Vec<SpawnTable> = resource_types.values().map(SpawnTable::from).filter(SpawnTable::is_planned).collect();
    let types: Vec<String> = tables.iter().map(|t| t.id.clone()).collect();
    let existing: Vec<(f64, f64)> =
        nodes.values().filter(|n| !types.contains(&n.resource_type)).map(|n| (n.x, n.y)).collect();
    let planned = plan(terrain, &tables, &existing, seed);

    let keep: std::collections::HashSet<Uuid> = planned.iter().map(|s| s.id).collect();
    let before = nodes.len();
    nodes.retain(|id, n| !types.contains(&n.resource_type) || keep.contains(id));
    let removed = before - nodes.len();
    let mut added = 0;
    for s in &planned {
        let node = nodes.entry(s.id).or_insert_with(|| {
            added += 1;
            ResourceNodeState {
                id: s.id,
                resource_type: s.resource_type.clone(),
                x: s.x,
                y: s.y,
                base_quality: s.base_quality,
                state: NodeState::Available,
                harvested_by: None,
                regenerates_at_tick: None,
            }
        });
        node.x = s.x;
        node.y = s.y;
    }
    Replan { types, nodes: planned, removed, added }
}

/// The planned resources' nodes as the game holds them, for the commit that
/// saves the regenerates since the last one.
pub fn planned_in(resource_types: &HashMap<String, ResourceType>, nodes: &HashMap<Uuid, ResourceNodeState>) -> Replan {
    let types: Vec<String> =
        resource_types.values().filter(|r| SpawnTable::from(*r).is_planned()).map(|r| r.id.clone()).collect();
    let nodes = nodes
        .values()
        .filter(|n| types.contains(&n.resource_type))
        .map(|n| NodeSeed { id: n.id, resource_type: n.resource_type.clone(), x: n.x, y: n.y, base_quality: n.base_quality })
        .collect();
    Replan { types, nodes, removed: 0, added: 0 }
}

/// Writes a [`replan`] to `resource_nodes`: the planned types' rows not in it
/// go, and its nodes are inserted or moved. Returns the rows deleted.
pub async fn save_replan(db: &PgPool, replan: &Replan) -> Result<u64, sqlx::Error> {
    let ids: Vec<Uuid> = replan.nodes.iter().map(|s| s.id).collect();
    let mut tx = db.begin().await?;
    let deleted = sqlx::query("DELETE FROM resource_nodes WHERE resource_type = ANY($1) AND NOT (id = ANY($2))")
        .bind(&replan.types)
        .bind(&ids)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let types: Vec<&str> = replan.nodes.iter().map(|s| s.resource_type.as_str()).collect();
    let xs: Vec<f64> = replan.nodes.iter().map(|s| s.x).collect();
    let ys: Vec<f64> = replan.nodes.iter().map(|s| s.y).collect();
    let qualities: Vec<i32> = replan.nodes.iter().map(|s| s.base_quality).collect();
    sqlx::query(
        "INSERT INTO resource_nodes (id, resource_type, position_x, position_y, base_quality)
         SELECT * FROM UNNEST($1::uuid[], $2::varchar[], $3::float8[], $4::float8[], $5::int4[])
         ON CONFLICT (id) DO UPDATE SET position_x = EXCLUDED.position_x, position_y = EXCLUDED.position_y",
    )
    .bind(&ids)
    .bind(&types)
    .bind(&xs)
    .bind(&ys)
    .bind(&qualities)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(deleted)
}

/// A stable key for a resource id (FNV-1a), for its random stream.
fn key(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

enum Pattern {
    Woods,
    UnderTrees,
    Clumped,
    AlongWater,
    Parks,
    Scattered,
}

/// The pattern a cluster takes in a zone, falling back to the parks and then
/// to scattering where it has nothing to go by.
fn pattern(cluster: Cluster, zone: Zone, ground: &Ground, trees: &[(f64, f64)]) -> Pattern {
    let fallback = || if ground.parks_in(zone).is_empty() { Pattern::Scattered } else { Pattern::Parks };
    match cluster {
        Cluster::Woods => Pattern::Woods,
        Cluster::UnderTrees if trees.is_empty() => fallback(),
        Cluster::UnderTrees => Pattern::UnderTrees,
        // Shallows only grow in the wilds' water; the city's banks stand in.
        Cluster::AlongWater | Cluster::Shallows if ground.banks_in(zone).is_empty() => fallback(),
        Cluster::AlongWater | Cluster::Shallows => Pattern::AlongWater,
        Cluster::Clumped => Pattern::Clumped,
        Cluster::Scattered => Pattern::Scattered,
    }
}

/// One resource's placing in one zone.
struct Placing<'a, 'b> {
    ground: &'a Ground<'a>,
    taken: &'b mut PointGrid,
    zone: Zone,
    table: &'b SpawnTable,
    land: LandTest<'b>,
    woods: Option<Woods>,
}

impl Placing<'_, '_> {
    /// Takes `at` if it fits: open ground, in the woods if there are woods,
    /// clear of every other node and passing the land test.
    fn try_take(&mut self, at: (f64, f64), in_park: bool, bank: bool) -> Option<(f64, f64)> {
        let at = (round_tenth(at.0), round_tenth(at.1));
        let fits = Zone::from_position(at.0, at.1) == self.zone
            && self.ground.open(at, if bank { BANK_CLEAR } else { MOVE_CLEAR }, in_park)
            && self.woods.as_ref().is_none_or(|w| w.covers(at))
            && self.taken.clear_of(at, MIN_GAP)
            && (self.land)(self.table, self.zone, at);
        fits.then(|| {
            self.taken.insert(at);
            at
        })
    }

    /// Up to `count` spots, each tried at most [`TRIES`] times.
    fn each(&mut self, count: usize, spot: impl FnMut(&mut Self) -> Option<(f64, f64)>) -> Vec<(f64, f64)> {
        self.each_within(count, TRIES, spot)
    }

    /// [`each`](Self::each), with `tries` per node.
    fn each_within(&mut self, count: usize, tries: usize, mut spot: impl FnMut(&mut Self) -> Option<(f64, f64)>) -> Vec<(f64, f64)> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(p) = (0..tries).find_map(|_| spot(self)) {
                out.push(p);
            }
        }
        out
    }

    fn scatter(&mut self, rng: &mut SplitMix64, count: usize, in_park: bool) -> Vec<(f64, f64)> {
        let zone = self.zone;
        let tries = if self.woods.is_some() { WOODS_TRIES } else { TRIES };
        self.each_within(count, tries, |p| p.try_take(point_in_ring(rng, zone), in_park, false))
    }

    fn near(&mut self, rng: &mut SplitMix64, count: usize, anchors: &[(f64, f64)]) -> Vec<(f64, f64)> {
        self.each(count, |p| {
            let a = anchors[rng.below(anchors.len() as u64) as usize];
            p.try_take(offset(rng, a, UNDER_TREES.0, UNDER_TREES.1), false, false)
        })
    }

    /// Knots of one to four (and the odd five), each round a centre on open ground.
    fn clumps(&mut self, rng: &mut SplitMix64, count: usize) -> Vec<(f64, f64)> {
        let mut out = Vec::with_capacity(count);
        let mut misses = 0;
        while out.len() < count && misses < TRIES {
            let Some(centre) = (0..TRIES).find_map(|_| self.try_take(point_in_ring(rng, self.zone), false, false)) else {
                break;
            };
            out.push(centre);
            let size = (1 + rng.below(5) as usize).min(count - out.len() + 1);
            for _ in 1..size {
                match (0..TRIES / 4).find_map(|_| self.try_take(offset(rng, centre, MIN_GAP, CLUMP_REACH), false, false)) {
                    Some(p) => out.push(p),
                    None => misses += 1,
                }
            }
        }
        out
    }

    fn banks(&mut self, rng: &mut SplitMix64, count: usize) -> Vec<(f64, f64)> {
        let banks = self.ground.banks_in(self.zone);
        let total: f64 = banks.iter().map(|b| b.radius).sum();
        let mut banked = self.each(count, |p| {
            let mut pick = rng.range(0.0, total);
            let bank = banks.iter().find(|b| {
                pick -= b.radius;
                pick < 0.0
            });
            let bank = bank.unwrap_or(&banks[banks.len() - 1]);
            let r = bank.radius + bank.side * rng.range(BANK_CLEAR, BANK_CLEAR + BANK_DEPTH);
            let angle = rng.range(0.0, TAU);
            p.try_take((CENTER_X + r * angle.stable_cos(), CENTER_Y + r * angle.stable_sin()), false, true)
        });
        if banked.len() < count {
            let rest = self.nearest_the_water(rng, count - banked.len());
            banked.extend(rest);
        }
        banked
    }

    /// Where lots fill the bank to the kerb (the urban side of the outer
    /// canal), the open ground nearest the water: the closest of many open
    /// spots, taken greedily.
    fn nearest_the_water(&mut self, rng: &mut SplitMix64, count: usize) -> Vec<(f64, f64)> {
        let ground = self.ground;
        let mut spots: Vec<(f64, (f64, f64))> = (0..count * TRIES)
            .map(|_| point_in_ring(rng, self.zone))
            .map(|p| (round_tenth(p.0), round_tenth(p.1)))
            .filter(|&p| Zone::from_position(p.0, p.1) == self.zone && ground.open(p, MOVE_CLEAR, false))
            .map(|p| (ground.water_clearance(p), p))
            .collect();
        spots.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out = Vec::with_capacity(count);
        for (_, p) in spots {
            if out.len() == count {
                break;
            }
            if let Some(p) = self.try_take(p, false, false) {
                out.push(p);
            }
        }
        out
    }

    fn in_parks(&mut self, rng: &mut SplitMix64, count: usize) -> Vec<(f64, f64)> {
        let parks = self.ground.parks_in(self.zone);
        let total: f64 = parks.iter().map(|p| p.area).sum();
        self.each(count, |p| {
            let mut pick = rng.range(0.0, total);
            let park = parks.iter().find(|park| {
                pick -= park.area;
                pick < 0.0
            });
            let park = park.unwrap_or(&parks[parks.len() - 1]);
            let (min, max) = bounds(park.rings.iter().flatten().copied());
            p.try_take((rng.range(min.0, max.0), rng.range(min.1, max.1)), true, false)
        })
    }
}

/// A uniform point in the zone's ring.
fn point_in_ring(rng: &mut SplitMix64, zone: Zone) -> (f64, f64) {
    let (inner, outer) = zone.radii();
    let r = rng.range(inner * inner, outer * outer).sqrt();
    let angle = rng.range(0.0, TAU);
    (CENTER_X + r * angle.stable_cos(), CENTER_Y + r * angle.stable_sin())
}

/// A point `lo` to `hi` from `at`, in any direction.
fn offset(rng: &mut SplitMix64, at: (f64, f64), lo: f64, hi: f64) -> (f64, f64) {
    let r = rng.range(lo, hi);
    let angle = rng.range(0.0, TAU);
    (at.0 + r * angle.stable_cos(), at.1 + r * angle.stable_sin())
}

fn round_tenth(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn bounds(points: impl Iterator<Item = (f64, f64)>) -> ((f64, f64), (f64, f64)) {
    points.fold(((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)), |(min, max), p| {
        ((min.0.min(p.0), min.1.min(p.1)), (max.0.max(p.0), max.1.max(p.1)))
    })
}

/// A seeded value-noise field, and the level above which it's woods.
struct Woods {
    seed: u64,
    threshold: f64,
}

impl Woods {
    /// The field for one resource in one zone, with its threshold set so the
    /// woods cover [`WOODS_COVER`] of the zone's open ground.
    fn new(seed: u64, id: &str, zone: Zone, ground: &Ground, rng: &mut SplitMix64) -> Self {
        let mut woods = Self { seed: SplitMix64::stream(seed, &[key(id), 0x600d5]).next_u64(), threshold: 0.0 };
        let mut levels: Vec<f64> = (0..WOODS_SAMPLES * 4)
            .map(|_| point_in_ring(rng, zone))
            .filter(|&p| Zone::from_position(p.0, p.1) == zone && ground.open(p, MOVE_CLEAR, false))
            .take(WOODS_SAMPLES)
            .map(|p| woods.level(p))
            .collect();
        levels.sort_by(f64::total_cmp);
        if !levels.is_empty() {
            let at = ((1.0 - WOODS_COVER) * levels.len() as f64) as usize;
            woods.threshold = levels[at.min(levels.len() - 1)];
        }
        woods
    }

    /// The field for one resource over `points`, with its threshold set so
    /// the woods cover `cover` of them: the wilds' valley woods.
    fn over(seed: u64, id: &str, points: impl Iterator<Item = (f64, f64)>, cover: f64) -> Self {
        let mut woods = Self { seed: SplitMix64::stream(seed, &[key(id), 0x600d5]).next_u64(), threshold: 0.0 };
        let mut levels: Vec<f64> = points.map(|p| woods.level(p)).collect();
        levels.sort_by(f64::total_cmp);
        if !levels.is_empty() {
            let at = ((1.0 - cover) * levels.len() as f64) as usize;
            woods.threshold = levels[at.min(levels.len() - 1)];
        }
        woods
    }

    fn covers(&self, at: (f64, f64)) -> bool {
        self.level(at) >= self.threshold
    }

    /// Two octaves of smoothed value noise, in [0, 1).
    fn level(&self, (x, y): (f64, f64)) -> f64 {
        let (x, y) = (x - CENTER_X - WOODS_ORIGIN.0, y - CENTER_Y - WOODS_ORIGIN.1);
        0.7 * self.octave(x / WOODS_SCALE, y / WOODS_SCALE, 1) + 0.3 * self.octave(x / (WOODS_SCALE / 2.5), y / (WOODS_SCALE / 2.5), 2)
    }

    fn octave(&self, x: f64, y: f64, octave: u64) -> f64 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (smooth(x - x0), smooth(y - y0));
        let corner = |dx: f64, dy: f64| {
            let (cx, cy) = ((x0 + dx) as i64 as u64, (y0 + dy) as i64 as u64);
            SplitMix64::stream(self.seed, &[octave, cx, cy]).next_f64()
        };
        let top = corner(0.0, 0.0) + (corner(1.0, 0.0) - corner(0.0, 0.0)) * fx;
        let bottom = corner(0.0, 1.0) + (corner(1.0, 1.0) - corner(0.0, 1.0)) * fx;
        top + (bottom - top) * fy
    }
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// One side of one water ring: the kerb's radius, and +1 outward or -1 inward.
struct Bank {
    radius: f64,
    side: f64,
}

/// The terrain, indexed for asking "is this spot open?" many times.
pub struct Ground<'a> {
    roads: SegmentGrid,
    lots: Vec<&'a TerrainPlot>,
    lot_cells: CellIndex,
    parks: &'a [TerrainPark],
    /// Each water ring as (inner, outer) radius, widened by the kerb.
    bands: Vec<(f64, f64)>,
}

impl<'a> Ground<'a> {
    pub fn new(terrain: &'a TerrainConfig) -> Self {
        let mut roads = SegmentGrid::new();
        for road in &terrain.roads {
            for w in road.points.windows(2) {
                roads.insert(w[0], w[1], road.width / 2.0);
            }
        }
        let lots: Vec<&TerrainPlot> = terrain.terrain_plots.iter().collect();
        let mut lot_cells = CellIndex::default();
        for (i, lot) in lots.iter().enumerate() {
            let (min, max) = bounds(lot.corners.iter().copied());
            lot_cells.insert_box(min, max, LOT_MARGIN, i);
        }
        let bands = terrain
            .water_rings
            .iter()
            .map(|ring| (ring.inner_radius() - WATER_KERB, ring.outer_radius() + WATER_KERB))
            .collect();
        Self { roads, lots, lot_cells, parks: &terrain.parks, bands }
    }

    /// Whether a node may stand at `at`: `road_clear` from every road edge and
    /// `water_clear` from the water's kerb, [`LOT_MARGIN`] off every lot, and
    /// in a park exactly when `in_park`.
    pub fn open(&self, at: (f64, f64), water_clear: f64, in_park: bool) -> bool {
        self.water_clearance(at) >= water_clear
            && self.roads.clearance(at, MOVE_CLEAR) >= MOVE_CLEAR
            && !self.near_lot(at)
            && self.parks.iter().any(|p| p.contains(at.0, at.1)) == in_park
    }

    fn water_clearance(&self, (x, y): (f64, f64)) -> f64 {
        let r = (x - CENTER_X).stable_hypot(y - CENTER_Y);
        self.bands.iter().map(|&(inner, outer)| (inner - r).max(r - outer)).fold(f64::MAX, f64::min)
    }

    fn near_lot(&self, (x, y): (f64, f64)) -> bool {
        self.lot_cells.at((x, y)).iter().any(|&i| {
            let lot = self.lots[i];
            quad_contains(&lot.corners, x, y)
                || (0..4).any(|k| {
                    let (a, b) = (lot.corners[k], lot.corners[(k + 1) % 4]);
                    point_to_segment_distance(x, y, a.0, a.1, b.0, b.1) < LOT_MARGIN
                })
        })
    }

    fn parks_in(&self, zone: Zone) -> Vec<&TerrainPark> {
        self.parks
            .iter()
            .filter(|p| p.rings.first().and_then(|r| r.first()).is_some_and(|&(x, y)| Zone::from_position(x, y) == zone))
            .collect()
    }

    /// The banks whose strip lies in `zone`.
    fn banks_in(&self, zone: Zone) -> Vec<Bank> {
        let (inner, outer) = zone.radii();
        let mid = BANK_CLEAR + BANK_DEPTH / 2.0;
        self.bands
            .iter()
            .flat_map(|&(lo, hi)| [Bank { radius: hi, side: 1.0 }, Bank { radius: lo, side: -1.0 }])
            .filter(|b| {
                let r = b.radius + b.side * mid;
                r > inner && r < outer
            })
            .collect()
    }
}

const CELL: f64 = 100.0;

fn cell_of(p: (f64, f64)) -> (i64, i64) {
    ((p.0 / CELL).floor() as i64, (p.1 / CELL).floor() as i64)
}

/// Ids by grid cell, for things with an extent.
#[derive(Default)]
struct CellIndex(std::collections::HashMap<(i64, i64), Vec<usize>>);

impl CellIndex {
    fn insert_box(&mut self, min: (f64, f64), max: (f64, f64), pad: f64, id: usize) {
        let (a, b) = (cell_of((min.0 - pad, min.1 - pad)), cell_of((max.0 + pad, max.1 + pad)));
        for cx in a.0..=b.0 {
            for cy in a.1..=b.1 {
                self.0.entry((cx, cy)).or_default().push(id);
            }
        }
    }

    fn at(&self, p: (f64, f64)) -> &[usize] {
        self.0.get(&cell_of(p)).map_or(&[], Vec::as_slice)
    }
}

/// Road segments, indexed by the cells they pass within [`MOVE_CLEAR`] of.
struct SegmentGrid {
    segments: Vec<((f64, f64), (f64, f64), f64)>,
    cells: CellIndex,
}

impl SegmentGrid {
    fn new() -> Self {
        Self { segments: Vec::new(), cells: CellIndex::default() }
    }

    fn insert(&mut self, a: (f64, f64), b: (f64, f64), half: f64) {
        let id = self.segments.len();
        self.segments.push((a, b, half));
        let (min, max) = bounds([a, b].into_iter());
        self.cells.insert_box(min, max, half + MOVE_CLEAR, id);
    }

    /// The distance to the nearest road edge, or `beyond` if none is nearer.
    fn clearance(&self, (x, y): (f64, f64), beyond: f64) -> f64 {
        self.cells
            .at((x, y))
            .iter()
            .map(|&i| {
                let (a, b, half) = self.segments[i];
                point_to_segment_distance(x, y, a.0, a.1, b.0, b.1) - half
            })
            .fold(beyond, f64::min)
    }
}

/// Points, for the spacing test.
struct PointGrid {
    cell: f64,
    cells: std::collections::HashMap<(i64, i64), Vec<(f64, f64)>>,
}

impl PointGrid {
    fn new(cell: f64) -> Self {
        Self { cell, cells: Default::default() }
    }

    fn key(&self, p: (f64, f64)) -> (i64, i64) {
        ((p.0 / self.cell).floor() as i64, (p.1 / self.cell).floor() as i64)
    }

    fn insert(&mut self, p: (f64, f64)) {
        let k = self.key(p);
        self.cells.entry(k).or_default().push(p);
    }

    /// Whether every point is at least `gap` (no more than the cell) from `p`.
    fn clear_of(&self, p: (f64, f64), gap: f64) -> bool {
        let (cx, cy) = self.key(p);
        (cx - 1..=cx + 1).all(|x| {
            (cy - 1..=cy + 1).all(|y| {
                self.cells
                    .get(&(x, y))
                    .is_none_or(|pts| pts.iter().all(|q| (q.0 - p.0).stable_hypot(q.1 - p.1) >= gap))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn content_tables() -> Vec<SpawnTable> {
        crate::content::load().unwrap().resources.iter().map(|r| SpawnTable::from(&r.to_type())).collect()
    }

    fn small_world() -> TerrainConfig {
        let mut terrain = TerrainConfig::get_default();
        terrain.terrain_plots.clear();
        terrain
    }

    #[test]
    fn grass_is_left_alone_and_the_wilds_come_last() {
        let tables = content_tables();
        let seeds = plan(&small_world(), &tables, &[], 1);
        assert!(!seeds.is_empty());
        assert!(seeds.iter().all(|s| s.resource_type != "grass"));
        assert!(!tables.iter().find(|t| t.id == "grass").unwrap().is_planned());
        // Every wild node after every other, so the wilds can't move E2.3's.
        let first_wild = seeds.iter().position(|s| Zone::from_position(s.x, s.y) == Zone::Wilderness).unwrap();
        assert!(seeds[first_wild..].iter().all(|s| Zone::from_position(s.x, s.y) == Zone::Wilderness));
        let without: Vec<SpawnTable> = tables.iter().cloned().map(|t| SpawnTable { habitat: None, ..t }).collect();
        assert_eq!(plan(&small_world(), &without, &[], 1), seeds[..first_wild]);
    }

    #[test]
    fn a_plan_is_the_same_every_time_and_another_seed_moves_it() {
        let tables = content_tables();
        let terrain = small_world();
        let a = plan(&terrain, &tables, &[], 5);
        assert_eq!(a, plan(&terrain, &tables, &[], 5));
        assert_ne!(a, plan(&terrain, &tables, &[], 6));
    }

    #[test]
    fn nodes_keep_their_gap_from_each_other_and_the_existing() {
        let terrain = small_world();
        let existing = [(CENTER_X + 3400.0, CENTER_Y), (CENTER_X, CENTER_Y + 3400.0)];
        let seeds = plan(&terrain, &content_tables(), &existing, 3);
        let mut all: Vec<(f64, f64)> = seeds.iter().map(|s| (s.x, s.y)).collect();
        all.extend(existing);
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (i, a) in all.iter().enumerate() {
            for b in all[i + 1..].iter().take_while(|b| b.0 - a.0 < MIN_GAP) {
                assert!((a.0 - b.0).stable_hypot(a.1 - b.1) >= MIN_GAP, "{a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn a_land_test_is_obeyed() {
        let east = |_: &SpawnTable, _: Zone, at: (f64, f64)| at.0 > CENTER_X;
        let seeds = plan_with(&small_world(), &content_tables(), &[], 2, &east);
        assert!(!seeds.is_empty());
        assert!(seeds.iter().all(|s| s.x > CENTER_X));
    }

    #[test]
    fn a_replan_keeps_the_nodes_it_plans_again_and_replaces_the_rest() {
        let types: HashMap<String, ResourceType> =
            crate::content::load().unwrap().resources.iter().map(|r| (r.id.clone(), r.to_type())).collect();
        let terrain = small_world();
        let grass_id = Uuid::from_u128(1);
        let grass = ResourceNodeState {
            id: grass_id,
            resource_type: "grass".into(),
            x: CENTER_X + 3400.0,
            y: CENTER_Y,
            base_quality: 50,
            state: NodeState::Available,
            harvested_by: None,
            regenerates_at_tick: None,
        };
        let mut nodes = HashMap::from([(grass_id, grass)]);

        let first = replan(&terrain, &types, &mut nodes, 1);
        assert_eq!((first.added, first.removed), (first.nodes.len(), 0));
        assert_eq!(nodes.len(), first.nodes.len() + 1);
        assert_eq!(planned_in(&types, &nodes).nodes.len(), first.nodes.len());

        let felled = first.nodes[0].id;
        nodes.get_mut(&felled).unwrap().state = NodeState::Depleted;
        let again = replan(&terrain, &types, &mut nodes, 1);
        assert_eq!((again.added, again.removed), (0, 0));
        assert_eq!(nodes[&felled].state, NodeState::Depleted, "a kept node keeps its state");

        let other = replan(&terrain, &types, &mut nodes, 2);
        assert_eq!(other.removed, first.nodes.len());
        assert!(nodes.contains_key(&grass_id), "grass isn't planned, so it stays");
    }

    #[test]
    fn the_bank_is_beside_the_water() {
        let terrain = small_world();
        let seeds = plan(&terrain, &content_tables(), &[], 4);
        let clay: Vec<_> = seeds.iter().filter(|s| s.resource_type == "clay_bank" && Zone::from_position(s.x, s.y) == Zone::Urban).collect();
        assert!(!clay.is_empty());
        let ground = Ground::new(&terrain);
        for s in clay {
            let w = ground.water_clearance((s.x, s.y));
            assert!((BANK_CLEAR..=BANK_CLEAR + BANK_DEPTH + 1.0).contains(&w), "clay {w:.1} from the water");
        }
    }
}
