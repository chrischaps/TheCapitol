use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;

use capitol_protocol::rules::{ARRIVAL_EPSILON, SPAWN_X, SPAWN_Y, TICK_SECS};
use capitol_protocol::terrain::{Bridge, RoadSegment, RoadType};

use super::*;
use crate::terrain::read_terrain_file;

/// The fixture world and its graph, built once for every test.
fn world() -> &'static (TerrainResponse, WalkGraph) {
    static WORLD: OnceLock<(TerrainResponse, WalkGraph)> = OnceLock::new();
    WORLD.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = read_terrain_file(&path).unwrap();
        let start = Instant::now();
        let graph = WalkGraph::new(&terrain);
        eprintln!("walk graph: {} nodes in {:?}", graph.len(), start.elapsed());
        (terrain, graph)
    })
}

/// Walks the legs from `from` the way the server does
/// (`backend/src/engine/movement.rs`): a step a tick, [`Ground::step`] with
/// the land, the rings and the edge. Where it ends, or what stopped it. A
/// step that slid round something counts as stopped: a leg must be walkable
/// straight.
fn server_walk(graph: &WalkGraph, from: Point, legs: &[Point]) -> Result<Point, String> {
    let mut at = from;
    for &leg in legs {
        for tick in 0.. {
            if distance(at, leg) < ARRIVAL_EPSILON {
                at = leg;
                break;
            }
            if tick > 200_000 {
                return Err(format!("never got from {at:?} to {leg:?}"));
            }
            let step = graph.ground.step(at, leg, PLAYER_SPEED * TICK_SECS);
            if let Some(blocked) = step.blocked {
                return Err(format!("{} at ({:.0}, {:.0}) toward {leg:?}", blocked.reason(), step.x, step.y));
            }
            let off = (leg.0 - at.0) * (step.y - at.1) - (leg.1 - at.1) * (step.x - at.0);
            if off.abs() > 1e-6 * distance(at, leg) {
                return Err(format!("slid round something at ({:.0}, {:.0}) toward {leg:?}", at.0, at.1));
            }
            at = (step.x, step.y);
        }
    }
    Ok(at)
}

/// A point `r` from the centre at `degrees` (0 east, 90 south: y is down).
fn polar(r: f64, degrees: f64) -> Point {
    let a = degrees.to_radians();
    (rules::CENTER_X + r * a.cos(), rules::CENTER_Y + r * a.sin())
}

/// The middle of each of the fixture's lots, every `every`th.
fn lots(terrain: &TerrainResponse, every: usize) -> Vec<Point> {
    terrain
        .terrain_plots
        .iter()
        .step_by(every)
        .map(|p| {
            let (x, y) = p.corners.iter().fold((0.0, 0.0), |s, c| (s.0 + c.0, s.1 + c.1));
            (x / 4.0, y / 4.0)
        })
        .collect()
}

/// A route, checked: it ends at `to`, and the server lets every leg through.
fn walk(graph: &WalkGraph, from: Point, to: Point) -> Vec<Point> {
    let legs = graph.route(from, to).unwrap_or_else(|| panic!("no route from {from:?} to {to:?}"));
    assert_eq!(legs.last(), Some(&to));
    match server_walk(graph, from, &legs) {
        Ok(end) => assert!(distance(end, to) < ARRIVAL_EPSILON),
        Err(e) => panic!("the route from {from:?} to {to:?} was blocked: {e}\n{legs:?}"),
    }
    legs
}

#[test]
fn a_straight_line_is_walkable_only_over_a_bridge() {
    let (_, graph) = world();
    // Up the south arterial, over the outer canal's bridge at 90°
    assert!(graph.walkable((SPAWN_X, SPAWN_Y), polar(800.0, 90.0)));
    // Across the outer canal (r 1000) at 20°, where it has no bridge
    assert!(!graph.walkable(polar(1100.0, 20.0), polar(900.0, 20.0)));
    // A chord that clips the canal twice and never ends inside it
    assert!(!graph.walkable(polar(1020.0, 10.0), polar(1020.0, 35.0)));
    assert!(graph.walkable(polar(1100.0, 20.0), polar(1100.0, 25.0)));
}

#[test]
fn a_walk_across_a_canal_goes_by_a_bridge() {
    let (_, graph) = world();
    let (from, to) = (polar(1100.0, 20.0), polar(900.0, 20.0));
    let legs = walk(graph, from, to);
    assert!(legs.len() <= 8, "{} legs for a walk round to the next bridge: {legs:?}", legs.len());
}

#[test]
fn the_edges_near_a_point_come_once_each() {
    let (_, graph) = world();
    let at = (SPAWN_X, SPAWN_Y);
    let edges = graph.edges_near(at, 300.0);
    assert!(edges.len() > 4, "only {} edges round the spawn", edges.len());
    for &(a, b) in &edges {
        assert!(distance(at, a) <= 300.0 || distance(at, b) <= 300.0, "{a:?}–{b:?} is far off");
    }
    let mut seen = std::collections::HashSet::new();
    for &(a, b) in &edges {
        let key = |p: Point| (p.0.to_bits(), p.1.to_bits());
        let (lo, hi) = if key(a) <= key(b) { (key(a), key(b)) } else { (key(b), key(a)) };
        assert!(seen.insert((lo, hi)), "{a:?}–{b:?} came twice");
    }
    assert!(graph.edges_near(polar(8290.0, 33.0), 1.0).is_empty());
}

#[test]
fn a_walk_that_can_go_straight_does() {
    let (_, graph) = world();
    let to = (SPAWN_X + 40.0, SPAWN_Y - 300.0);
    assert_eq!(graph.route((SPAWN_X, SPAWN_Y), to), Some(vec![to]));
}

#[test]
fn the_sim_crafter_gets_from_the_suburb_parks_to_the_capitol_and_back() {
    // E2.7's crafter: the suburb's parks for clay, its kiln out in the rural
    // ring, and the Exchange hall inside the canals (#211).
    let (terrain, graph) = world();
    let park = terrain.parks.iter().find(|p| p.rings[0].len() > 2).unwrap();
    let ring = &park.rings[0];
    let park_at = ring.iter().fold((0.0, 0.0), |s, c| (s.0 + c.0 / ring.len() as f64, s.1 + c.1 / ring.len() as f64));
    let hall = graph.landmark(LandmarkKind::ExchangeHall).unwrap().door;
    let rural = lots(terrain, 1)
        .into_iter()
        .find(|&p| rules::Zone::from_position(p.0, p.1) == rules::Zone::Rural)
        .unwrap();
    walk(graph, park_at, hall);
    walk(graph, hall, rural);
    walk(graph, rural, park_at);
}

#[test]
fn walks_between_lots_all_get_there_in_few_moves() {
    let (terrain, graph) = world();
    let lots = lots(terrain, 97);
    let hall = graph.landmark(LandmarkKind::ExchangeHall).unwrap().door;
    let start = Instant::now();
    let (mut legs, mut length, mut walks) = (0usize, 0.0, 0usize);
    for (i, &from) in lots.iter().enumerate() {
        for &to in [lots[(i * 7 + 3) % lots.len()], hall].iter() {
            let route = walk(graph, from, to);
            legs += route.len();
            length += std::iter::once(from).chain(route.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| distance(w[0], w[1])).sum::<f64>();
            walks += 1;
        }
    }
    let per_leg = length / legs as f64;
    eprintln!(
        "{walks} walks, {legs} legs, {per_leg:.0} units a leg, {:?} a walk",
        start.elapsed() / walks as u32
    );
    // One `move` a leg, and a connection may send 60 a minute: a leg should
    // take more than a second's walk (PLAYER_SPEED units) on average.
    assert!(per_leg > 2.0 * PLAYER_SPEED, "{per_leg:.0} units a leg");
}

#[test]
fn the_wilds_are_walked_by_the_trails() {
    let (_, graph) = world();
    let land = graph.ground.land().clone();
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    let (mut tried, mut found) = (0, 0);
    while tried < 40 {
        let to = polar(5450.0 + 1700.0 * next(), 360.0 * next());
        // Somewhere a walker could stand: dry, and gentle all round
        let gentle = (0..8).all(|k| {
            let p = (to.0 + 15.0 * (k as f64).cos(), to.1 + 15.0 * (k as f64).sin());
            land::slope_at(p.0, p.1, &land) < 0.6 && graph.ground.water_at(p.0, p.1).is_none()
        });
        if !gentle || graph.ground.water_at(to.0, to.1).is_some() {
            continue;
        }
        tried += 1;
        let Some(legs) = graph.route((SPAWN_X, SPAWN_Y), to) else { continue };
        found += 1;
        if let Err(e) = server_walk(graph, (SPAWN_X, SPAWN_Y), &legs) {
            panic!("the route to {to:?} was blocked: {e}\n{legs:?}");
        }
    }
    eprintln!("{found} of {tried} wild spots reached");
    assert!(found * 10 >= tried * 8, "only {found} of {tried} wild spots reached");
}

#[test]
fn nowhere_in_a_lake() {
    let (terrain, graph) = world();
    let lake = &terrain.lakes[0];
    let (min, max) = lake.rings[0].iter().fold(((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)), |(lo, hi), p| {
        ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1)))
    });
    let deep = (0..400)
        .map(|k| (min.0 + (max.0 - min.0) * (k % 20) as f64 / 20.0, min.1 + (max.1 - min.1) * (k / 20) as f64 / 20.0))
        .find(|&p| (0..8).all(|i| {
            let q = (p.0 + 30.0 * (i as f64).cos(), p.1 + 30.0 * (i as f64).sin());
            graph.ground.water_at(q.0, q.1) == Some(lake.id.as_str())
        }))
        .expect("somewhere 30 from the shore");
    assert_eq!(graph.route((SPAWN_X, SPAWN_Y), deep), None);
}

/// A world of one canal, r 1000 and 15 wide round the centre, with one
/// bridge at 90° and a road over it.
pub(super) fn one_canal() -> TerrainResponse {
    TerrainResponse {
        world_size: 13600.0,
        center_x: 6800.0,
        center_y: 6800.0,
        water_rings: vec![WaterRing {
            id: "canal_outer".into(),
            radius: 1000.0,
            width: 15.0,
            bridges: vec![Bridge { angle_degrees: 90.0, width: 25.0 }],
        }],
        roads: vec![RoadSegment {
            id: "arterial_2".into(),
            road_type: RoadType::Arterial,
            points: vec![(6800.0, 7600.0), (6800.0, 8000.0)],
            width: 20.0,
        }],
        terrain_plots: vec![],
        parks: vec![],
        landmarks: vec![],
        land: Default::default(),
        rivers: vec![],
        streams: vec![],
        lakes: vec![],
        fords: vec![],
        version: Some("one-canal".into()),
    }
}

#[test]
fn one_bridge_and_its_road() {
    let terrain = one_canal();
    let graph = WalkGraph::new(&terrain);
    // 400 of road in pieces of SPAN: 11 nodes, joined in a line
    assert_eq!(graph.len(), 11);
    let (from, to) = ((6900.0, 7900.0), (6900.0, 7700.0));
    assert!(!graph.walkable(from, to));
    let legs = walk(&graph, from, to);
    // Over to the road, along it and over the bridge, and off it
    assert!(legs.len() >= 2 && legs.len() <= 3, "{legs:?}");
    assert!(legs.iter().any(|p| (p.0 - 6800.0).abs() < DEVIATION));
    // Too far from the road to reach it (GRID_REACH): still no way round
    assert_eq!(graph.route((8800.0, 8800.0), (6800.0, 6900.0)), None);
}

#[test]
fn graphs_are_shared_by_version() {
    let a = shared(&one_canal());
    let b = shared(&one_canal());
    assert!(Arc::ptr_eq(&a, &b));
    let mut other = one_canal();
    other.version = Some("another".into());
    assert!(!Arc::ptr_eq(&a, &shared(&other)));
    assert_eq!(latest().unwrap().version(), Some("another"));
}

#[test]
fn crossings_and_distances() {
    let c = (0.0, 0.0);
    let both = circle_crossings(c, (-20.0, 0.0), (20.0, 0.0), 10.0);
    assert_eq!(both, [Some((-10.0, 0.0)), Some((10.0, 0.0))]);
    assert_eq!(circle_crossings(c, (0.0, 0.0), (20.0, 0.0), 10.0), [None, Some((10.0, 0.0))]);
    assert_eq!(circle_crossings(c, (-20.0, 15.0), (20.0, 15.0), 10.0), [None, None]);
    assert_eq!(off_line((0.0, 0.0), (10.0, 0.0), (5.0, 3.0)), 3.0);
    assert_eq!(off_line((0.0, 0.0), (10.0, 0.0), (13.0, 4.0)), 5.0);
}



/// Short lines in the wilds, from where a walker can stand, half of them on
/// steep ground: every one the graph calls walkable, the server walks, from
/// its start or from partway along (#280). Before, one line in a thousand
/// crossed a crest steeper than the cliff between two of the graph's tests.
#[test]
fn what_the_graph_calls_walkable_the_server_walks() {
    let (_, graph) = world();
    let land = graph.ground.land().clone();
    let mut rng = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    let (mut tried, mut walkable) = (0, 0);
    while tried < 4000 {
        let a = polar(6300.0 + 1950.0 * next(), 360.0 * next());
        let slope = land::slope_at(a.0, a.1, &land);
        let steep = tried % 2 == 0;
        if slope > CLIFF_SLOPE || (steep && slope < 0.9) || graph.ground.footing(a.0, a.1) != Footing::Dry {
            continue;
        }
        let (heading, length) = (std::f64::consts::TAU * next(), 5.0 + 75.0 * next());
        let b = (a.0 + length * heading.cos(), a.1 + length * heading.sin());
        tried += 1;
        if !graph.walkable(a, b) {
            continue;
        }
        walkable += 1;
        for from in [a, lerp(a, b, 0.37)] {
            if let Err(e) = server_walk(graph, from, &[b]) {
                panic!("the graph calls {a:?} to {b:?} walkable, but from {from:?}: {e}");
            }
        }
    }
    eprintln!("{walkable} of {tried} lines walkable");
    assert!(walkable * 2 > tried, "only {walkable} of {tried} lines walkable");
}

#[test]
fn the_walks_the_server_stopped_go_there_and_back() {
    // From the Exchange hall to wild nodes (#280): the first four crossed a
    // crest the graph's tests stepped over, the next a ford that ends a
    // quarter short of its river's bank, then one clipped a lake's corner on
    // the way back. The last two had no trail node in sight within 800.
    // Since Plots v2's flip (#459) each stands 1500 further out on its
    // bearing, the same depth into the wilds, but the land there is new:
    // two had no way in, and moved to the nearest spot that has one.
    let (_, graph) = world();
    let hall = graph.landmark(LandmarkKind::ExchangeHall).unwrap().door;
    for node in [
        (15652.9, 11950.4),
        (15721.6, 11546.2),
        (2084.1, 12514.4),
        (15137.2, 11613.8),
        (14546.9, 3273.9),
        (14857.7, 3444.5),
        (2491.5, 6216.8),
        (6243.9, 14511.1),
    ] {
        walk(graph, hall, node);
        walk(graph, node, hall);
    }
}

#[test]
fn a_pocket_walled_by_crests_has_no_way_in_or_out() {
    // A copper vein the backend planted where the crests wall it in (#280).
    // Since Plots v2's flip (#459) moved the ranges out, it's a spot in a
    // pocket of the east-south-east massif, found by searching the wilds.
    let (_, graph) = world();
    let hall = graph.landmark(LandmarkKind::ExchangeHall).unwrap().door;
    let vein = (15329.1, 11083.0);
    assert_eq!(graph.plan(hall, vein), Err(NoWay::Unreachable));
    assert_eq!(graph.plan(vein, hall), Err(NoWay::Stranded));
}

#[test]
fn walks_go_round_the_ponds_of_the_rural_ring() {
    // C1.2's citizen walking home was stopped by a rural lake (#280): from
    // one side of each pond to the other, both ways.
    let (terrain, graph) = world();
    let mut walked = 0;
    for lake in &terrain.lakes {
        let ring = &lake.rings[0];
        let (min, max) = ring.iter().fold(((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)), |(lo, hi), p| {
            ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1)))
        });
        let mid = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
        if rules::Zone::from_position(mid.0, mid.1) != rules::Zone::Rural {
            continue;
        }
        let (west, east) = ((min.0 - 25.0, mid.1), (max.0 + 25.0, mid.1));
        let dry = |p: Point| graph.ground.footing(p.0, p.1) == Footing::Dry && !terrain.roads.is_empty();
        if !dry(west) || !dry(east) || graph.walkable(west, east) {
            continue;
        }
        walk(graph, west, east);
        walk(graph, east, west);
        walked += 1;
    }
    eprintln!("round {walked} ponds");
    assert!(walked > 0, "no pond in the rural ring to walk round");
}



/// A citizen walking the road in (#450) finds the stops the client rings
/// (#445): the same rule over the same `/terrain`, with only the lots near
/// the road kept, and a stop on the road it can walk to from the spawn.
#[test]
fn the_road_ins_stops_are_the_clients() {
    let (terrain, graph) = world();
    for zone in stops::STOP_ZONES {
        let whole = stops::stop(&terrain.terrain_plots, &Frame::TABLE, zone, |_| false);
        let found = graph.road_stop(zone, |_| false);
        assert_eq!(found.as_ref().map(|s| s.lot.as_str()), whole.map(|s| s.lot.id.as_str()), "{zone}");
        let stop = found.unwrap_or_else(|| panic!("the fixture world has no {zone} stop"));
        assert!(!stop.taken);
        // Taken, the stop moves to the next lot as the client's does
        let taken = graph.road_stop(zone, |id| id == stop.lot);
        let whole = stops::stop(&terrain.terrain_plots, &Frame::TABLE, zone, |id| id == stop.lot);
        assert_eq!(taken.map(|s| s.lot), whole.map(|s| s.lot.id.clone()), "{zone}, taken");
        assert!(graph.route((SPAWN_X, SPAWN_Y), stop.road).is_some(), "no way to the {zone} stop");
    }
}

/// The graph's networks by size, largest first.
fn network_sizes(graph: &WalkGraph) -> Vec<usize> {
    let mut sizes = HashMap::<u32, usize>::new();
    for c in graph.networks() {
        *sizes.entry(c).or_default() += 1;
    }
    let mut sizes: Vec<usize> = sizes.into_values().collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes
}

/// The fixture world's roads make one network by their shared vertices and
/// welds alone, so [`JOIN`] adds no line across open ground to it (#341). A
/// change that leaves a road apart fails here, rather than as a walk that
/// cuts across a field.
#[test]
fn the_roads_alone_make_one_network() {
    let (terrain, graph) = world();
    let roads = WalkGraph::joined_within(terrain, 0.0);
    let sizes = network_sizes(&roads);
    assert_eq!(sizes.len(), 1, "networks apart, largest first: {:?}", &sizes[..sizes.len().min(8)]);
    assert_eq!(roads.edges, graph.edges, "JOIN added a line to the fixture world");
}

/// What a smaller JOIN does to the fixture world's graph (#341).
#[test]
#[ignore]
fn print_join_reaches() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
    let terrain = read_terrain_file(&path).unwrap();
    for reach in [600.0, 300.0, 150.0, 60.0, 0.0] {
        let start = Instant::now();
        let graph = WalkGraph::joined_within(&terrain, reach);
        let built = start.elapsed();
        let roads = WalkGraph::joined_within(&terrain, 0.0);
        let (mut joins, mut length, mut longest) = (0, 0.0, 0.0f64);
        for i in 0..graph.len() {
            for &(j, _) in &graph.edges[i] {
                if i < j as usize && !roads.adjacent(i as u32, j) {
                    let d = distance(graph.nodes[i], graph.nodes[j as usize]);
                    joins += 1;
                    length += d;
                    longest = longest.max(d);
                }
            }
        }
        let sizes = network_sizes(&graph);
        eprintln!(
            "JOIN {reach:>4}: {} networks {:?}, {joins} joins, {length:.0} long, longest {longest:.0}, built in {built:?}",
            sizes.len(),
            &sizes[..sizes.len().min(8)]
        );
    }
}
