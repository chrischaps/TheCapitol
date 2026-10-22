use std::collections::HashSet;
use std::sync::OnceLock;

use super::*;
use capitol_protocol::rules::ground::CLIFF_SLOPE;
use capitol_protocol::rules::{Frame, RURAL_OUTER_RADIUS, WILDERNESS_OUTER_RADIUS, WORLD_SIZE};
use crate::models::{TerrainConfig, WorldGenParams};

/// The table's radii past its rural edge.
const PASS_MIN_RADIUS: f64 = RURAL_OUTER_RADIUS + PASS_MIN_PAST_RURAL;
const OPEN_GROUND: f64 = RURAL_OUTER_RADIUS + OPEN_GROUND_PAST_RURAL;

fn radius(x: f64, y: f64) -> f64 {
    Frame::TABLE.radius(x, y)
}

// A trail is always walkable: W4 stops a walk only past CLIFF_SLOPE.
const _: () = assert!(WALK_LIMIT < CLIFF_SLOPE);

/// The default world, generated once for the tests that only read it.
fn default_world() -> &'static TerrainConfig {
    static WORLD: OnceLock<TerrainConfig> = OnceLock::new();
    WORLD.get_or_init(TerrainConfig::get_default)
}

/// Another land seed's world, round the same city.
fn world_for(seed: u64) -> TerrainConfig {
    TerrainConfig::from_params(&WorldGenParams { land: LandParams { seed, ..LandParams::default() }, ..WorldGenParams::default() })
}

fn wild(world: &TerrainConfig) -> impl Iterator<Item = &RoadSegment> {
    world.roads.iter().filter(|r| r.id.starts_with("wild_"))
}

/// The wild trails that can be walked to from the arterials, by way of the
/// vertices they share.
fn reached(world: &TerrainConfig) -> HashSet<&str> {
    let roads: Vec<&RoadSegment> =
        world.roads.iter().filter(|r| r.id.starts_with("wild_") || r.id.starts_with("arterial_")).collect();
    let shares = |a: &RoadSegment, b: &RoadSegment| {
        a.points.iter().any(|&p| b.points.iter().any(|&q| dist(p, q) < 1e-6))
    };
    let mut seen: HashSet<&str> = roads.iter().filter(|r| r.id.starts_with("arterial_")).map(|r| r.id.as_str()).collect();
    loop {
        let more: Vec<&str> = roads
            .iter()
            .filter(|r| !seen.contains(r.id.as_str()))
            .filter(|r| roads.iter().any(|o| seen.contains(o.id.as_str()) && shares(r, o)))
            .map(|r| r.id.as_str())
            .collect();
        if more.is_empty() {
            return seen;
        }
        seen.extend(more);
    }
}

/// The cell centres a player can walk to from `from` without a step onto a
/// lake or ground at the cliff slope.
fn on_foot_from(world: &TerrainConfig, from: &[P]) -> Vec<P> {
    let n = (WORLD_SIZE / CELL) as usize;
    let centre = |k: usize| (((k % n) as f64 + 0.5) * CELL, ((k / n) as f64 + 0.5) * CELL);
    let open: Vec<bool> = (0..n * n)
        .map(|k| {
            let (x, y) = centre(k);
            let r = radius(x, y);
            r > RURAL_OUTER_RADIUS - CELL
                && r < WILDERNESS_OUTER_RADIUS
                && slope_at(x, y, &world.land) < CLIFF_SLOPE
                && !world.lakes.iter().any(|l| l.contains(x, y))
        })
        .collect();
    let mut seen = vec![false; n * n];
    let mut queue: Vec<usize> =
        from.iter().map(|p| (p.1 / CELL) as usize * n + (p.0 / CELL) as usize).filter(|&k| open[k]).collect();
    queue.iter().for_each(|&k| seen[k] = true);
    while let Some(k) = queue.pop() {
        for m in [k + 1, k.wrapping_sub(1), k + n, k.wrapping_sub(n)] {
            if m < n * n && open[m] && !seen[m] {
                seen[m] = true;
                queue.push(m);
            }
        }
    }
    (0..n * n).filter(|&k| seen[k]).map(centre).collect()
}

/// Every point along every trail is walkable: under the cliff slope, and off
/// the lakes.
#[test]
fn every_trail_keeps_under_the_cliff_slope_and_off_the_lakes() {
    for world in [default_world(), &world_for(7)] {
        assert!(wild(world).count() > 16, "trails: {}", wild(world).count());
        for trail in wild(world) {
            assert_eq!(trail.road_type, RoadType::Trail);
            for w in trail.points.windows(2) {
                let steps = (dist(w[0], w[1]) / 2.0).ceil().max(1.0) as usize;
                for k in 0..=steps {
                    let t = k as f64 / steps as f64;
                    let (x, y) = (w[0].0 + t * (w[1].0 - w[0].0), w[0].1 + t * (w[1].1 - w[0].1));
                    let s = slope_at(x, y, &world.land);
                    assert!(s < WALK_LIMIT, "{} is too steep at ({x:.0}, {y:.0}): {s:.2}", trail.id);
                    for lake in &world.lakes {
                        assert!(!lake.contains(x, y), "{} runs into {} at ({x:.0}, {y:.0})", trail.id, lake.id);
                    }
                    assert!(radius(x, y) < WILDERNESS_OUTER_RADIUS, "{} leaves the world", trail.id);
                }
            }
        }
    }
}

/// The trails keep the street plan's invariants. The one kind of loose end
/// is a spur's, and it's at what the spur goes to: a lake's shore or a ford.
#[test]
fn the_trails_pass_the_street_plan_audit() {
    for world in [default_world(), &world_for(7)] {
        let audit = street_plan::audit(&world.roads, |r| r.id.starts_with("wild_"));
        assert!(audit.short_segments.is_empty(), "{:?}", audit.short_segments);
        assert!(audit.sharp_turns.is_empty(), "{:?}", audit.sharp_turns);
        assert!(audit.self_crossings.is_empty(), "{:?}", audit.self_crossings);
        assert!(audit.unjoined_crossings.is_empty(), "{:?}", audit.unjoined_crossings);
        for (id, end) in &audit.loose_ends {
            if id.starts_with("wild_climb_") {
                continue; // at the top of its massif
            }
            assert!(id.starts_with("wild_spur_"), "{id} ends loose at {end:?}");
            let at_ford = world.fords.iter().any(|f| dist((f.x, f.y), *end) < 1e-6);
            // On its shore, or as near a walled-in tarn as a trail gets.
            let at_shore = world.lakes.iter().any(|l| shore_distance(&l.rings, *end) <= TARN_REACH + CELL);
            assert!(at_ford || at_shore, "{id} ends at {end:?}, by no ford or lake");
        }
        // No trail is laid over another road but where they meet.
        for trail in wild(world) {
            for road in world.roads.iter().filter(|r| !r.id.starts_with("wild_")) {
                let touches = trail.points.iter().any(|&p| {
                    road.points.windows(2).any(|w| point_segment(p, w[0], w[1]) < road.width / 2.0)
                });
                let starts_there = road.points.iter().any(|&q| dist(q, trail.points[0]) < 1e-6);
                assert!(!touches || starts_there, "{} runs over {}", trail.id, road.id);
            }
        }
    }
}

/// From the spawn, by the arterials and the trails, you can reach every lake
/// in the wilds and climb every range: each valley has its trail, and each
/// massif between two valleys is crossed high up.
#[test]
fn every_lake_and_every_range_can_be_reached_by_trail() {
    for world in [default_world(), &world_for(7)] {
        let seen = reached(world);
        for trail in wild(world) {
            assert!(seen.contains(trail.id.as_str()), "{} can't be reached from the arterials", trail.id);
        }
        let vertices: Vec<P> = wild(world).flat_map(|r| r.points.iter().copied()).collect();

        let mut bearings: Vec<f64> = world
            .roads
            .iter()
            .filter(|r| r.id.starts_with("arterial_"))
            .filter_map(|r| r.points.last().copied())
            .filter(|&(x, y)| radius(x, y) >= RURAL_OUTER_RADIUS - 1.0)
            .map(|(x, y)| {
                let id = world.roads.iter().find(|t| t.id.starts_with("wild_trail_") && dist(t.points[0], (x, y)) < 1e-6);
                assert!(id.is_some(), "the arterial ending at ({x:.0}, {y:.0}) has no trail on");
                bearing_deg(Frame::TABLE.center, x, y)
            })
            .collect();
        assert_eq!(bearings.len(), 8);
        bearings.sort_by(f64::total_cmp);
        for k in 0..bearings.len() {
            let (a, b) = (bearings[k], bearings[(k + 1) % bearings.len()]);
            let span = (b - a).rem_euclid(360.0);
            let high = vertices.iter().any(|&(x, y)| {
                let along = (bearing_deg(Frame::TABLE.center, x, y) - a).rem_euclid(360.0);
                along > 0.25 * span && along < 0.75 * span && radius(x, y) >= PASS_MIN_RADIUS
            });
            assert!(high, "no trail crosses the massif between {a:.0}° and {b:.0}°");
        }

        // A tarn walled in by ground too steep for a trail, or a pond on the
        // open foot of the hills, is reached on foot from the nearest trail,
        // under the cliff slope. The trails reach all
        // the ground a player can walk to, so a lake no walk from them reaches
        // is walled in by cliffs, and nobody could get there.
        let on_foot = on_foot_from(world, &vertices);
        let (mut tarns, mut sheer) = (Vec::new(), Vec::new());
        for lake in &world.lakes {
            if lake.rings.iter().flatten().any(|p| radius(p.0, p.1) < RURAL_OUTER_RADIUS) {
                continue; // a pond on the open plain
            }
            let open = lake.rings.iter().flatten().any(|p| radius(p.0, p.1) < OPEN_GROUND);
            if !open && vertices.iter().any(|&v| shore_distance(&lake.rings, v) <= SHORE_REACH + CELL) {
                continue;
            }
            if on_foot.iter().any(|&p| shore_distance(&lake.rings, p) <= SHORE_REACH) {
                tarns.push(lake.id.as_str());
            } else {
                sheer.push(lake.id.as_str());
            }
        }
        println!("reached on foot from a trail: {tarns:?}; walled in by cliffs: {sheer:?}");
        assert!(sheer.is_empty(), "lakes walled in by cliffs: {sheer:?}");
    }
}

/// The trails are a pure function of the world.
#[test]
fn the_same_world_lays_the_same_trails() {
    let again = TerrainConfig::get_default();
    let a: Vec<&RoadSegment> = wild(&again).collect();
    let b: Vec<&RoadSegment> = wild(default_world()).collect();
    assert_eq!(a, b);
}

#[test]
fn chaikin_keeps_the_ends_and_halves_the_turns() {
    let p = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)];
    let q = chaikin(&p);
    assert_eq!(q.first(), p.first());
    assert_eq!(q.last(), p.last());
    assert!((sharpest_turn(&q) - 45.0).abs() < 1e-9, "{}", sharpest_turn(&q));
}

#[test]
fn simplify_keeps_the_ends_and_the_corners() {
    let p: Vec<P> = (0..=10).map(|k| (k as f64 * 10.0, 0.0)).chain((1..=10).map(|k| (100.0, k as f64 * 10.0))).collect();
    assert_eq!(simplify(&p, 1.0), vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]);
}
