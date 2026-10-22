use std::sync::OnceLock;

use super::*;
use capitol_protocol::fmath::StableMath;
use capitol_protocol::land::{carve, ground_at, relief, Water};
use capitol_protocol::rules::{
    Frame, CENTER_X, CENTER_Y, SUBURBAN_OUTER_RADIUS, WATER_INNER_RADIUS as INNER_RADIUS, WILDERNESS_OUTER_RADIUS,
    WORLD_SIZE,
};

/// The table's plain's edge, where the land starts to rise (r6200; r4700
/// before Plots v2's flip, #459).
const PLAIN_EDGE: f64 = capitol_protocol::rules::RURAL_OUTER_RADIUS + capitol_protocol::land::FOOTHILLS_PAST_RURAL;

fn radius(x: f64, y: f64) -> f64 {
    Frame::TABLE.radius(x, y)
}

/// The default world's roads, which the water skirts: all but the wild
/// trails, which are laid round the water after it (W5).
fn roads() -> &'static [RoadSegment] {
    static ROADS: OnceLock<Vec<RoadSegment>> = OnceLock::new();
    ROADS.get_or_init(|| {
        let mut roads = crate::models::TerrainConfig::get_default().roads;
        roads.retain(|r| !r.id.starts_with("wild_"));
        roads
    })
}

/// The default land's water, generated once for the tests that only read it.
fn default_water() -> &'static Hydrology {
    static WATER: OnceLock<Hydrology> = OnceLock::new();
    WATER.get_or_init(|| generate(&LandParams::default(), roads()))
}

/// Another seed's land, round the default roads.
fn water_for(seed: u64) -> Hydrology {
    if seed == 42 {
        return default_water().clone();
    }
    generate(&LandParams { seed, ..LandParams::default() }, roads())
}

fn courses(w: &Hydrology) -> impl Iterator<Item = &WaterCourse> {
    w.rivers.iter().chain(&w.streams)
}

fn bearing(x: f64, y: f64) -> f64 {
    (y - CENTER_Y).stable_atan2(x - CENTER_X).to_degrees().rem_euclid(360.0)
}

/// Every river and stream runs downhill, from its source to its mouth, and
/// its mouth is where it says: the edge, a lake, a point of the course it
/// joins, or petering out on the plain. Following the joins always reaches
/// one of the others.
#[test]
fn every_course_runs_downhill_to_its_end() {
    for seed in [42, 7, 1234] {
        let w = water_for(seed);
        let by_id: HashMap<&str, &WaterCourse> = courses(&w).map(|c| (c.id.as_str(), c)).collect();
        let lakes: HashMap<&str, &Lake> = w.lakes.iter().map(|l| (l.id.as_str(), l)).collect();
        assert_eq!(by_id.len(), w.rivers.len() + w.streams.len(), "course ids are unique");
        for c in courses(&w) {
            assert!(c.points.len() >= 2, "{} has {} points", c.id, c.points.len());
            for p in c.points.windows(2) {
                assert!(p[1].surface <= p[0].surface, "{} rises: {} to {}", c.id, p[0].surface, p[1].surface);
            }
            let last = c.points.last().unwrap();
            match &c.mouth {
                // On the edge, to the 0.01 the points are rounded to.
                Mouth::Edge => assert!(radius(last.x, last.y) >= WILDERNESS_OUTER_RADIUS - 0.02, "{}", c.id),
                Mouth::Lake(id) => {
                    let lake = lakes[id.as_str()];
                    assert!(lake.contains(last.x, last.y), "{} ends outside {id}", c.id);
                    assert!((last.surface - lake.level).abs() < 0.01, "{} meets {id} at its level", c.id);
                }
                Mouth::Joins(id) => {
                    let main = by_id[id.as_str()];
                    let on = main.points.iter().find(|p| p.x == last.x && p.y == last.y);
                    let on = on.unwrap_or_else(|| panic!("{} ends off {id}", c.id));
                    assert_eq!(on.surface, last.surface, "{} meets {id} at its surface", c.id);
                }
                Mouth::PetersOut => {
                    let r = radius(last.x, last.y);
                    assert!(r < PLAIN_EDGE, "{} peters out in the hills, at r{r:.0}", c.id);
                    assert!(last.width < c.points[0].width.max(4.0), "{} narrows as it peters out", c.id);
                }
            }
            if let Some(id) = &c.source {
                let lake = lakes[id.as_str()];
                let first = c.points[0];
                assert!(lake.contains(first.x, first.y), "{} rises outside {id}", c.id);
                assert!((first.surface - lake.level).abs() < 0.01);
            }
            // Down the joins to the edge or a lake, with no loop.
            let mut at = c;
            for _ in 0..=by_id.len() {
                match &at.mouth {
                    Mouth::Joins(id) => at = by_id[id.as_str()],
                    _ => break,
                }
            }
            assert!(!matches!(at.mouth, Mouth::Joins(_)), "{} never ends", c.id);
        }
        // A river joins a stream only where it soaks away into one on the plain.
        for r in &w.rivers {
            if let Mouth::Joins(id) = &r.mouth {
                if id.starts_with("stream") {
                    let (last, next) = (r.points.last().unwrap(), by_id[id.as_str()].points[0]);
                    assert!(last.x == next.x && last.y == next.y, "{} joins {id} other than by soaking away", r.id);
                    assert!(radius(last.x, last.y) < PLAIN_EDGE + PLAIN_BLEND, "{} soaks away in the hills", r.id);
                }
            }
        }
    }
}

/// No two courses cross: where one meets another, it ends there, on one of
/// that one's points.
#[test]
fn courses_meet_only_where_one_joins_another() {
    fn cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
        let o = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
        o(a, b, c) * o(a, b, d) < 0.0 && o(c, d, a) * o(c, d, b) < 0.0
    }
    for seed in [42, 7, 1234] {
        let w = water_for(seed);
        let all: Vec<&WaterCourse> = courses(&w).collect();
        let mut crossings = Vec::new();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                for s in a.points.windows(2) {
                    for t in b.points.windows(2) {
                        let (p, q) = ((s[0].x, s[0].y), (s[1].x, s[1].y));
                        let (r, u) = ((t[0].x, t[0].y), (t[1].x, t[1].y));
                        if cross(p, q, r, u) {
                            crossings.push(format!("{} x {} near ({:.0}, {:.0})", a.id, b.id, p.0, p.1));
                        }
                    }
                }
            }
        }
        assert!(crossings.is_empty(), "seed {seed}: {} crossings: {:?}", crossings.len(), &crossings[..crossings.len().min(8)]);
    }
}

/// Nothing wet in the suburb or near it: no course, bank or lake inside the
/// inner rim, and the carve leaves the ground inside the suburb alone.
#[test]
fn the_suburb_stays_dry() {
    let w = default_water();
    for c in courses(w) {
        for p in &c.points {
            let r = radius(p.x, p.y);
            assert!(r - p.width / 2.0 > INNER_RADIUS - CELL, "{} at r{r:.0}", c.id);
        }
    }
    for lake in &w.lakes {
        for &(x, y) in lake.rings.iter().flatten() {
            assert!(radius(x, y) > INNER_RADIUS, "{} at r{:.0}", lake.id, radius(x, y));
        }
    }
    let water = Water::new(courses(w), &w.lakes);
    for step in 0..=300 {
        let r = step as f64 * 10.0;
        for d in 0..360 {
            let a = (d as f64).to_radians();
            let (x, y) = (CENTER_X + r * a.stable_cos(), CENTER_Y + r * a.stable_sin());
            assert_eq!(ground_at(x, y, &LandParams::default(), &water), 0.0, "r{r} at {d}°");
        }
    }
}

/// The default wilds have lakes, rivers and streams all round: in each of
/// the eight sectors between the arterials, a lake, two rivers and a handful
/// of streams.
#[test]
fn the_default_wilds_have_water_in_every_sector() {
    let w = default_water();
    let sector = |x: f64, y: f64| (bearing(x, y) / 45.0) as usize % 8;
    for s in 0..8 {
        let in_sector = |c: &&WaterCourse| c.points.iter().any(|p| sector(p.x, p.y) == s);
        let rivers = w.rivers.iter().filter(in_sector).count();
        let streams = w.streams.iter().filter(in_sector).count();
        let lakes = w.lakes.iter().filter(|l| l.rings.iter().flatten().any(|&(x, y)| sector(x, y) == s)).count();
        println!("sector {s}: {lakes} lakes, {rivers} rivers, {streams} streams");
        assert!(lakes >= 1 && rivers >= 2 && streams >= 5, "sector {s}: {lakes} lakes, {rivers} rivers, {streams} streams");
    }
    assert!(!w.fords.is_empty());
}

/// The water comes onto the rural plain, and where it ends there is spread
/// across it, not on one circle (#222). Ponds keep clear of the roads.
#[test]
fn the_plain_soaks_the_water_up_all_across_it() {
    for seed in [42, 7, 1234] {
        let w = water_for(seed);
        let lakes: HashMap<&str, &Lake> = w.lakes.iter().map(|l| (l.id.as_str(), l)).collect();
        let mut ends = Vec::new();
        let (mut ponds, mut peter) = (0, 0);
        for c in courses(&w) {
            let last = c.points.last().unwrap();
            let r = radius(last.x, last.y);
            match &c.mouth {
                Mouth::PetersOut => peter += 1,
                Mouth::Lake(id) if r < PLAIN_EDGE => {
                    ponds += 1;
                    for &(x, y) in lakes[id.as_str()].rings.iter().flatten() {
                        let clear = Roads::near(roads(), &Rings::of(&LandParams::default())).clearance((x, y), POND_KERB);
                        assert!(clear >= POND_KERB - 1e-6, "{id} is {clear:.1} from a road");
                    }
                }
                _ => continue,
            }
            ends.push(r);
        }
        ends.sort_by(f64::total_cmp);
        let (lo, hi) = (ends[0], ends[ends.len() - 1]);
        let busiest = (0..ends.len())
            .map(|i| ends[i..].iter().take_while(|&&r| r < ends[i] + 100.0).count())
            .max()
            .unwrap();
        println!(
            "seed {seed}: {} ends on the plain ({ponds} ponds, {peter} peter out), r{lo:.0} to r{hi:.0}, at most {busiest} within 100",
            ends.len()
        );
        assert!(ponds >= 3 && peter >= 3, "seed {seed}: {ponds} ponds, {peter} peter out");
        assert!(hi - lo >= 300.0, "seed {seed}: the ends span only r{lo:.0} to r{hi:.0}");
        assert!(busiest * 2 <= ends.len(), "seed {seed}: {busiest} of {} ends within 100 of each other", ends.len());
    }
}

/// Each lake has a shore, a level and an area, and no two lakes overlap.
#[test]
fn lakes_are_whole_and_apart() {
    let w = default_water();
    for (k, lake) in w.lakes.iter().enumerate() {
        assert_eq!(lake.id, format!("lake_{k}"));
        assert!(!lake.rings.is_empty() && lake.area > 0.0, "{}", lake.id);
        for other in &w.lakes[k + 1..] {
            for &(x, y) in &other.rings[0] {
                assert!(!lake.contains(x, y), "{} overlaps {}", other.id, lake.id);
            }
        }
    }
}

/// A ford is on its river, where it's shallow, and fords on one river are
/// spaced apart.
#[test]
fn fords_are_on_shallow_stretches_of_their_rivers() {
    let w = default_water();
    for f in &w.fords {
        let river = w.rivers.iter().find(|r| r.id == f.river_id).expect("its river");
        let at = river.points.iter().find(|p| p.x == f.x && p.y == f.y).expect("on a point of its river");
        assert!(f.depth <= FORD_DEPTH && at.depth == f.depth, "{}", f.id);
        assert!(f.width >= at.width, "{} spans its river", f.id);
    }
}

/// The carve only ever cuts, puts every bed under its water, and leaves no
/// step in the ground.
#[test]
fn the_carve_cuts_beds_under_the_water() {
    let land = LandParams::default();
    let w = default_water();
    let water = Water::new(courses(w), &w.lakes);
    for c in courses(w).step_by(3) {
        for p in c.points.iter().step_by(4) {
            let g = ground_at(p.x, p.y, &land, &water);
            assert!(g <= p.surface - p.depth * 0.99, "{} bed {g} under surface {} depth {}", c.id, p.surface, p.depth);
        }
    }
    for lake in &w.lakes {
        // Points a little in from the shore, clear of it all round.
        let mut found = false;
        for &(x, y) in &lake.rings[0] {
            for (dx, dy) in [(8.0, 0.0), (-8.0, 0.0), (0.0, 8.0), (0.0, -8.0)] {
                let (px, py) = (x + dx, y + dy);
                let probes = [(4.0, 0.0), (-4.0, 0.0), (0.0, 4.0), (0.0, -4.0)];
                if probes.iter().all(|&(ox, oy)| lake.contains(px + ox, py + oy)) {
                    assert!(ground_at(px, py, &land, &water) < lake.level, "{} has ground above its level", lake.id);
                    found = true;
                }
            }
        }
        assert!(found, "{} has no inside", lake.id);
    }
    // Only cuts, and continuous: a fine walk across the wilds.
    for i in 0..40000 {
        let a = (i as f64 * 0.0007).to_radians() * 9.0;
        let r = 3880.0 + (i % 2000) as f64 * 1.06;
        let (x, y) = (CENTER_X + r * a.stable_cos(), CENTER_Y + r * a.stable_sin());
        let h = height_at(x, y, &land);
        let g = carve(h, x, y, &water);
        assert!(g <= h);
        let g2 = ground_at(x + 0.01, y, &land, &water);
        assert!((g - g2).abs() < 0.3, "a step at ({x}, {y}): {g} vs {g2}");
    }
}

/// Generation is a pure function of the land.
#[test]
fn the_same_land_makes_the_same_water() {
    let again = generate(&LandParams::default(), roads());
    let w = default_water();
    assert_eq!(again.rivers, w.rivers);
    assert_eq!(again.streams, w.streams);
    assert_eq!(again.lakes, w.lakes);
    assert_eq!(again.fords, w.fords);
}

/// Draws the water over the carved relief, for review:
/// `WATER_PLAN=water.png cargo test --release water_plan -- --ignored`
/// (`backend/tools/land-relief.ps1` does this for some seeds).
#[test]
#[ignore]
fn water_plan() {
    let path = std::env::var("WATER_PLAN").unwrap_or_else(|_| "water-plan.png".into());
    let seed = std::env::var("LAND_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(42);
    let size: usize = std::env::var("LAND_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(1200);
    let land = LandParams { seed, ..LandParams::default() };
    // The whole world for that land, so the wild trails (W5) are drawn too.
    let world = crate::models::TerrainConfig::from_params(&crate::models::WorldGenParams {
        land: land.clone(),
        ..crate::models::WorldGenParams::default()
    });
    let w = Hydrology {
        rivers: world.rivers.clone(),
        streams: world.streams.clone(),
        lakes: world.lakes.clone(),
        fords: world.fords.clone(),
    };
    let water = Water::new(courses(&w), &w.lakes);
    let (mut rgb, mut heights) = relief::draw(&land, size, |x, y| ground_at(x, y, &land, &water));
    let scale = WORLD_SIZE / size as f64;
    let mut put = |px: i64, py: i64, c: [u8; 3]| {
        if (0..size as i64).contains(&px) && (0..size as i64).contains(&py) {
            let i = (py as usize * size + px as usize) * 3;
            rgb[i..i + 3].copy_from_slice(&c);
        }
    };
    const LAKE: [u8; 3] = [70, 120, 176];
    const RIVER: [u8; 3] = [40, 86, 160];
    const STREAM: [u8; 3] = [92, 150, 206];
    const FORD: [u8; 3] = [232, 150, 40];
    const ROAD: [u8; 3] = [150, 132, 104];
    const TRAIL: [u8; 3] = [150, 40, 30];
    // The pixels of a road `half` pixels either side of its line.
    let stroke = |road: &RoadSegment, half: i64| {
        let mut px = Vec::new();
        for s in road.points.windows(2) {
            let (a, b) = (s[0], s[1]);
            if radius(a.0, a.1).max(radius(b.0, b.1)) < SUBURBAN_OUTER_RADIUS {
                continue;
            }
            let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) / scale).ceil().max(1.0) as i64;
            for k in 0..=steps {
                let t = k as f64 / steps as f64;
                let (x, y) = (((a.0 + t * (b.0 - a.0)) / scale) as i64, ((a.1 + t * (b.1 - a.1)) / scale) as i64);
                for dy in -half..=half {
                    for dx in -half..=half {
                        px.push((x + dx, y + dy));
                    }
                }
            }
        }
        px
    };
    let wild = |road: &&RoadSegment| road.id.starts_with("wild_");
    // The roads past the suburb, under the water.
    for road in world.roads.iter().filter(|r| !wild(r)) {
        for (x, y) in stroke(road, ((road.width / 2.0) / scale).floor() as i64) {
            put(x, y, ROAD);
        }
    }
    for lake in &w.lakes {
        let pts = lake.rings.iter().flatten();
        let (x0, y0) = pts.clone().fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
        let (x1, y1) = pts.fold((f64::MIN, f64::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
        for py in (y0 / scale) as i64..=(y1 / scale) as i64 + 1 {
            for px in (x0 / scale) as i64..=(x1 / scale) as i64 + 1 {
                let (x, y) = ((px as f64 + 0.5) * scale, (py as f64 + 0.5) * scale);
                if lake.contains(x, y) {
                    put(px, py, LAKE);
                }
            }
        }
    }
    for (list, colour) in [(&w.streams, STREAM), (&w.rivers, RIVER)] {
        for c in list {
            for s in c.points.windows(2) {
                let (a, b) = (s[0], s[1]);
                let half = (0.5 * a.width.max(b.width)).max(0.6 * scale);
                let (x0, x1) = (a.x.min(b.x) - half, a.x.max(b.x) + half);
                let (y0, y1) = (a.y.min(b.y) - half, a.y.max(b.y) + half);
                for py in (y0 / scale) as i64..=(y1 / scale) as i64 {
                    for px in (x0 / scale) as i64..=(x1 / scale) as i64 {
                        let (x, y) = ((px as f64 + 0.5) * scale, (py as f64 + 0.5) * scale);
                        let (dx, dy) = (b.x - a.x, b.y - a.y);
                        let len2 = (dx * dx + dy * dy).max(1e-9);
                        let t = (((x - a.x) * dx + (y - a.y) * dy) / len2).clamp(0.0, 1.0);
                        let (ex, ey) = (a.x + t * dx - x, a.y + t * dy - y);
                        if (ex * ex + ey * ey).sqrt() <= half {
                            put(px, py, colour);
                        }
                    }
                }
            }
        }
    }
    // The wild trails (W5) over the water, so their bridges show.
    for road in world.roads.iter().filter(wild) {
        for (x, y) in stroke(road, ((road.width / 2.0) / scale).round().max(1.0) as i64) {
            put(x, y, TRAIL);
        }
    }
    for f in &w.fords {
        let (cx, cy) = ((f.x / scale) as i64, (f.y / scale) as i64);
        for dy in -3..=3i64 {
            for dx in -3..=3i64 {
                if dx * dx + dy * dy <= 9 {
                    put(cx + dx, cy + dy, FORD);
                }
            }
        }
    }
    // With WILD_NODES set, the planned nodes past the suburb (W6), as dots.
    let mut node_counts = String::new();
    if std::env::var_os("WILD_NODES").is_some() {
        let tables: Vec<crate::engine::node_spawn::SpawnTable> = crate::content::load()
            .unwrap()
            .resources
            .iter()
            .map(|r| crate::engine::node_spawn::SpawnTable::from(&r.to_type()))
            .collect();
        let plan = crate::engine::node_spawn::plan(&world, &tables, &[], crate::models::WorldGenParams::default().seed);
        const KINDS: [(&str, [u8; 3]); 5] = [
            ("tree", [22, 70, 34]),
            ("fallen_branches", [140, 96, 50]),
            ("rock_outcrop", [240, 240, 244]),
            ("clay_bank", [196, 84, 200]),
            ("iron_vein", [255, 205, 0]),
        ];
        let dot = ((12.0 / scale).round() as i64).max(1);
        for (kind, colour) in KINDS {
            let mut n = 0;
            for s in plan.iter().filter(|s| s.resource_type == kind && radius(s.x, s.y) >= SUBURBAN_OUTER_RADIUS) {
                n += 1;
                let (cx, cy) = ((s.x / scale) as i64, (s.y / scale) as i64);
                for dy in -dot..=dot {
                    for dx in -dot..=dot {
                        if dx * dx + dy * dy <= dot * dot {
                            put(cx + dx, cy + dy, colour);
                        }
                    }
                }
            }
            node_counts.push_str(&format!(", {kind} {n}"));
        }
    }
    std::fs::write(&path, relief::png(size, size, &rgb)).expect("write the plan");
    heights.sort_by(f64::total_cmp);
    let pct = |p: f64| heights[((heights.len() - 1) as f64 * p) as usize];
    println!(
        "relief: {path} (seed {seed}, {size}px). Wild heights: median {:.0}, p90 {:.0}, p99 {:.0}, max {:.0}. \
         Water: {} rivers, {} streams, {} lakes, {} fords{node_counts}",
        pct(0.5),
        pct(0.9),
        pct(0.99),
        pct(1.0),
        w.rivers.len(),
        w.streams.len(),
        w.lakes.len(),
        w.fords.len()
    );
}


