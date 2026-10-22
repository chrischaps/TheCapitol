//! Arrival's commons (#443, `the-capitol-road-in.md` §4.1): what the road
//! in's first chapter gathers, grown within a short walk of the spawn.
//!
//! Chapter 1 gathers grass, fallen branches and stone by hand, and fells a
//! tree with the axe it makes from them. The rural ring's spawn tables grow
//! those thinly: on the default world the nearest tree stood 300 from the
//! spawn, the nearest outcrop 466, and only 2 outcrops within 600. So a
//! newcomer walked off the chapter's map for stone, and a few arriving at
//! once waited on the regrowth (an outcrop comes back in 3 minutes).
//!
//! [`plan`] adds a commons round the spawn, [`REACH`] of it at most so that
//! `/world/nearby`'s default range shows all of it: a copse with branches
//! under it, a few knots of outcrops, and grass between. Every node is on
//! open ground (`Ground::open`: off the road and the water, and off every lot
//! by `LOT_MARGIN`), so a farmer claiming the field can't take the chapter
//! from a newcomer.
//!
//! It's placed after everything else, as the wilds are, so the rest of the
//! plan keeps its nodes and ids. **Grass** is the migrations'
//! (`SEEDED_BY_MIGRATIONS`), so the commons' grass is planned only when the
//! grass table is handed in, as the default plan that the migration is
//! written from hands it in. A regenerate hands in the planned types only
//! (`replan`), so it leaves the commons' grass be, as it leaves the rest of
//! the grass, and `node_placement` moves it if a new road runs over it. The
//! grass goes last of the four, so leaving it out changes nothing else.

use capitol_protocol::rules::Zone;

use super::{key, offset, round_tenth, Ground, LandTest, NodeSeed, Placing, PointGrid, SpawnTable, SplitMix64};
use super::{CLUMP_REACH, MIN_GAP, TRIES, UNDER_TREES};

/// How far from the spawn the commons reaches: inside `/world/nearby`'s
/// default range (`NEARBY_NODE_RANGE`, 200), with room for the scatter of
/// arrivals (`SPAWN_SCATTER`).
pub const REACH: f64 = 180.0;

/// How many newcomers at once the commons serves, besides what grows round
/// the spawn anyway, with none of them waiting on a regrowth.
pub const CROWD: usize = 3;

/// What the commons grows, in the order it's placed, and how many nodes one
/// newcomer works of each on the chapter, each node once before it regrows:
/// a tree to fell, branches twice (2 sticks, 1 or 2 a pull), stone 4 times
/// (1 a pull by hand) and grass about 7 times (14 fiber, for two ropes and
/// the bench kit, at 1 to 3 a pull). The commons grows [`CROWD`] times as
/// many, and the trees stand in a copse of at least [`COPSE_TREES`].
pub const PULLS: [(&str, usize); 4] = [("tree", 1), ("fallen_branches", 2), ("rock_outcrop", 4), ("grass", 7)];

/// The fewest trees the copse has: a copse, not a lone tree.
pub const COPSE_TREES: usize = 6;

/// How far the copse's trees stand from its middle.
const COPSE: f64 = 40.0;

/// How many knots the outcrops stand in.
const KNOTS: usize = 3;

/// Plants the commons round `spawn` into `seeds`, from those of [`PULLS`]'
/// kinds that `tables` hold, keeping clear of everything in `taken`.
pub(super) fn plan(
    ground: &Ground,
    tables: &[&SpawnTable],
    spawn: (f64, f64),
    taken: &mut PointGrid,
    seed: u64,
    land: LandTest,
    seeds: &mut Vec<NodeSeed>,
) {
    let zone = Zone::from_position(spawn.0, spawn.1);
    let within = |p: (f64, f64)| {
        let (dx, dy) = (p.0 - spawn.0, p.1 - spawn.1);
        dx * dx + dy * dy <= REACH * REACH
    };
    let mut trees: Vec<(f64, f64)> = Vec::new();
    for (kind, pulls) in PULLS {
        let Some(&table) = tables.iter().find(|t| t.id == kind) else {
            continue;
        };
        let count = if kind == "tree" { (CROWD * pulls).max(COPSE_TREES) } else { CROWD * pulls };
        let mut rng = SplitMix64::stream(seed, &[key("arrival"), key(kind)]);
        let mut place = Placing { ground, taken: &mut *taken, zone, table, land, woods: None };
        let take = |place: &mut Placing, at: (f64, f64)| {
            let at = (round_tenth(at.0), round_tenth(at.1));
            if within(at) {
                place.try_take(at, false, false)
            } else {
                None
            }
        };
        let spots = match kind {
            // A copse: its middle well off the road, its trees round it.
            "tree" => {
                let middle = (0..TRIES).find_map(|_| take(&mut place, offset(&mut rng, spawn, 70.0, REACH - COPSE)));
                match middle {
                    Some(middle) => {
                        let mut out = vec![middle];
                        out.extend(place.each(count - 1, |p| take(p, offset(&mut rng, middle, MIN_GAP, COPSE))));
                        out
                    }
                    None => Vec::new(),
                }
            }
            // Under the commons' trees, or anywhere in it if there are none.
            "fallen_branches" if !trees.is_empty() => place.each(count, |p| {
                let tree = trees[rng.below(trees.len() as u64) as usize];
                take(p, offset(&mut rng, tree, UNDER_TREES.0, UNDER_TREES.1))
            }),
            // Knots round a few middles, as the ring's outcrops stand.
            // A knot short of its share (a lot's edge in the way) leaves the
            // rest to another knot, up to twice as many as planned.
            "rock_outcrop" => {
                let share = count.div_ceil(KNOTS);
                let mut out = Vec::new();
                for _ in 0..KNOTS * 2 {
                    let size = share.min(count - out.len());
                    if size == 0 {
                        break;
                    }
                    let Some(middle) =
                        (0..TRIES * 4).find_map(|_| take(&mut place, offset(&mut rng, spawn, 40.0, REACH - MIN_GAP)))
                    else {
                        break;
                    };
                    out.push(middle);
                    out.extend(place.each(size - 1, |p| take(p, offset(&mut rng, middle, MIN_GAP, CLUMP_REACH))));
                }
                out
            }
            _ => place.each(count, |p| take(p, offset(&mut rng, spawn, 0.0, REACH))),
        };
        if kind == "tree" {
            trees.extend(&spots);
        }
        let floor = table.quality.0.min(table.quality.1);
        let mut ids = SplitMix64::stream(seed, &[key("arrival"), key(kind), u64::MAX]);
        for (x, y) in spots {
            let base_quality = floor + rng.below((table.quality.1 - floor + 1) as u64) as i32;
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
