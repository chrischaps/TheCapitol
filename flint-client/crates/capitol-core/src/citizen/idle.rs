//! Idle life (C2.1): where a citizen spends the time between its tasks.
//!
//! Free time is a decision point (C1.3). Its menu is the places worth being
//! that are near enough to walk to: the hall's steps, the bazaar's lane, a
//! park, a bridge at dusk, and the citizen's own stall. The menu says which
//! ([`menu`]); the rules layer weighs them by temperament and a whim
//! ([`rules`](super::decide::rules)); and the citizen's loop walks there and
//! lingers ([`Place::spot`] says where to stand, and which way to look).
//!
//! The places come from `/terrain` ([`Places::new`]), so they are the
//! world's: the hall's door and the face it's on, the bazaar's lane, the
//! suburb's parks (big enough to stand about in) and the canals' bridges.

use capitol_protocol::terrain::{Landmark, LandmarkKind, TerrainResponse};

use super::decide::Choice;

type Point = (f64, f64);

/// A park smaller than this (square units; 10 000 is a hundred square
/// metres) is a verge, not somewhere to go.
const PARK_SMALLEST: f64 = 10_000.0;
/// How far a citizen walks to a park in its free time: about two game
/// minutes' walk.
const PARK_REACH: f64 = 900.0;
/// How far it walks to a bridge at dusk.
const BRIDGE_REACH: f64 = 900.0;
/// How far from the hall its steps and the bazaar's lane are worth the
/// walk. The hall itself is always on the menu (C1.3): it's where the
/// market is.
const CENTRE_REACH: f64 = 1200.0;
/// Dusk, for the bridges: from half past five, as the light goes gold, to
/// bedtime.
pub const DUSK: f64 = 17.5 / 24.0;

/// A place worth being, and where in it to stand.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub kind: PlaceKind,
    /// Where the place is, for how far away it is.
    pub at: Point,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaceKind {
    /// The Exchange hall's steps: in front of its door, along the face it's
    /// on, within reach of the market.
    HallSteps {
        door: Point,
        /// Along the face, half its length each way.
        along: Point,
        half: f64,
        /// Out from the face, toward the avenue.
        out: Point,
    },
    /// The bazaar's lane, from the avenue in to the stalls.
    Lane { from: Point, to: Point },
    /// A park: somewhere within `room` of a point well inside it.
    Park { id: String, room: f64 },
    /// A bridge over a canal or the moat: on its deck by a rail, looking
    /// along the water.
    Bridge {
        id: String,
        /// Along the water (the ring's tangent), for the rail and the view.
        along: Point,
        /// From the deck's middle to its rail.
        rail: f64,
    },
}

impl Place {
    /// Where to stand, and a point to look toward, for `draw`, a number in
    /// [0, 1) from the citizen's own stream. Different draws spread a crowd
    /// over the place.
    pub fn spot(&self, draw: f64, draw2: f64) -> (Point, Point) {
        let (x, y) = self.at;
        match &self.kind {
            PlaceKind::HallSteps { door, along, half, out } => {
                let a = (draw * 2.0 - 1.0) * half;
                let o = 4.0 + 16.0 * draw2;
                let p = (door.0 + along.0 * a + out.0 * o, door.1 + along.1 * a + out.1 * o);
                // Looking out over the avenue, a little either way
                let skew = (draw2 - 0.5) * 0.8;
                (p, (p.0 + (out.0 + along.0 * skew) * 50.0, p.1 + (out.1 + along.1 * skew) * 50.0))
            }
            PlaceKind::Lane { from, to } => {
                let t = 0.15 + 0.7 * draw;
                let (dx, dy) = (to.0 - from.0, to.1 - from.1);
                let len = dx.hypot(dy).max(1e-6);
                let (nx, ny) = (-dy / len, dx / len);
                // Off the crown to one side, looking across at the other
                let side = if draw2 < 0.5 { -1.0 } else { 1.0 };
                let off = side * (5.0 + 12.0 * (draw2 - 0.5).abs());
                let p = (from.0 + dx * t + nx * off, from.1 + dy * t + ny * off);
                (p, (p.0 - nx * side * 40.0 + dx / len * 20.0, p.1 - ny * side * 40.0 + dy / len * 20.0))
            }
            PlaceKind::Park { room, .. } => {
                let a = std::f64::consts::TAU * draw;
                let r = room * draw2;
                let p = (x + a.cos() * r, y + a.sin() * r);
                (p, (p.0 + a.cos() * 50.0, p.1 + a.sin() * 50.0))
            }
            PlaceKind::Bridge { along, rail, .. } => {
                // By one rail or the other, along it a little, looking
                // down the water past the rail's end
                let side = if draw < 0.5 { -1.0 } else { 1.0 };
                let (nx, ny) = (-along.1, along.0);
                let s = (draw2 - 0.5) * 6.0;
                let p = (x + along.0 * side * rail + nx * s, y + along.1 * side * rail + ny * s);
                (p, (p.0 + along.0 * side * 60.0, p.1 + along.1 * side * 60.0))
            }
        }
    }

    /// What the health line says on the way there.
    pub fn going(&self) -> &'static str {
        match self.kind {
            PlaceKind::HallSteps { .. } => "to the hall steps",
            PlaceKind::Lane { .. } => "to the bazaar lane",
            PlaceKind::Park { .. } => "to the park",
            PlaceKind::Bridge { .. } => "to the bridge",
        }
    }

    /// What the health line says it's doing there.
    pub fn activity(&self) -> &'static str {
        match self.kind {
            PlaceKind::HallSteps { .. } => "on the hall steps",
            PlaceKind::Lane { .. } => "strolling the bazaar lane",
            PlaceKind::Park { .. } => "in the park",
            PlaceKind::Bridge { .. } => "on the bridge",
        }
    }
}

/// The world's places worth being.
#[derive(Debug, Clone, Default)]
pub struct Places {
    pub hall_steps: Option<Place>,
    pub lane: Option<Place>,
    pub parks: Vec<Place>,
    pub bridges: Vec<Place>,
}

impl Places {
    pub fn new(terrain: &TerrainResponse) -> Places {
        let centre = (terrain.center_x, terrain.center_y);
        let hall_steps = terrain
            .landmarks
            .iter()
            .find(|l| l.kind == LandmarkKind::ExchangeHall)
            .map(steps);
        let lane = terrain.roads.iter().find(|r| r.id == "bazaar_lane").and_then(|r| {
            let (&from, &to) = (r.points.first()?, r.points.last()?);
            Some(Place {
                kind: PlaceKind::Lane { from, to },
                at: ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0),
            })
        });
        let parks = terrain
            .parks
            .iter()
            .filter(|p| p.area >= PARK_SMALLEST)
            .filter_map(|p| {
                let (at, edge) = inside(p)?;
                Some(Place {
                    kind: PlaceKind::Park {
                        id: p.id.clone(),
                        room: (edge * 0.7).min(40.0),
                    },
                    at,
                })
            })
            .collect();
        let bridges = terrain
            .water_rings
            .iter()
            .flat_map(|ring| {
                ring.bridges.iter().map(move |b| {
                    let a = b.angle_degrees.to_radians();
                    let (c, s) = (a.cos(), a.sin());
                    Place {
                        kind: PlaceKind::Bridge {
                            id: format!("{}@{}", ring.id, b.angle_degrees),
                            along: (-s, c),
                            rail: (b.width / 2.0 - 3.0).max(0.0),
                        },
                        at: (centre.0 + ring.radius * c, centre.1 + ring.radius * s),
                    }
                })
            })
            .collect();
        Places {
            hall_steps,
            lane,
            parks,
            bridges,
        }
    }

    /// The park or bridge `id` names.
    pub fn named(&self, id: &str) -> Option<&Place> {
        self.parks.iter().chain(&self.bridges).find(|p| match &p.kind {
            PlaceKind::Park { id: i, .. } | PlaceKind::Bridge { id: i, .. } => i == id,
            _ => false,
        })
    }

    /// The nearest park to `at` within reach.
    pub fn park_near(&self, at: Point) -> Option<&Place> {
        nearest(&self.parks, at, PARK_REACH)
    }

    /// The nearest bridge to `at` within reach.
    pub fn bridge_near(&self, at: Point) -> Option<&Place> {
        nearest(&self.bridges, at, BRIDGE_REACH)
    }
}

/// What a citizen at `at` could do with its free time at `time` (the day's
/// share): rest where it is, the hall, and of the rest whatever is near
/// enough. `stall` is its own stall's id, if it keeps one.
pub fn menu(places: &Places, at: Point, time: Option<f64>, stall: Option<&str>) -> Vec<Choice> {
    let mut menu = vec![Choice::Rest, Choice::Hall];
    let near_centre = places.hall_steps.as_ref().is_some_and(|h| distance(h.at, at) <= CENTRE_REACH);
    if near_centre && places.lane.is_some() {
        menu.push(Choice::Lane);
    }
    if let (true, Some(stall)) = (near_centre, stall) {
        menu.push(Choice::Stall { stall: stall.to_string() });
    }
    if let Some(Place {
        kind: PlaceKind::Park { id, .. },
        ..
    }) = places.park_near(at)
    {
        menu.push(Choice::Park { park: id.clone() });
    }
    if time.is_some_and(at_dusk) {
        if let Some(bridge) = places.bridge_near(at) {
            menu.push(Choice::Bridge { bridge: bridge_id(bridge) });
        }
    }
    menu
}

/// The menu at dusk on the way home: straight home, or a while on a bridge
/// near the way first.
pub fn homeward(places: &Places, at: Point, home: Point) -> Option<Vec<Choice>> {
    let bridge = places
        .bridges
        .iter()
        .filter(|b| near_the_way(b.at, at, home))
        .min_by(|a, b| distance(a.at, at).total_cmp(&distance(b.at, at)))?;
    Some(vec![Choice::Home, Choice::Bridge { bridge: bridge_id(bridge) }])
}

/// Whether `time` (the day's share) is dusk: the bridges' hour.
pub fn at_dusk(time: f64) -> bool {
    time.rem_euclid(1.0) >= DUSK
}

fn bridge_id(place: &Place) -> String {
    match &place.kind {
        PlaceKind::Bridge { id, .. } => id.clone(),
        _ => String::new(),
    }
}

/// A bridge within reach of `at`, and no more than a short way off the
/// line home.
fn near_the_way(bridge: Point, at: Point, home: Point) -> bool {
    distance(bridge, at) <= BRIDGE_REACH && segment_distance(bridge, at, home) <= 250.0
}

fn nearest(places: &[Place], at: Point, reach: f64) -> Option<&Place> {
    places
        .iter()
        .filter(|p| distance(p.at, at) <= reach)
        .min_by(|a, b| distance(a.at, at).total_cmp(&distance(b.at, at)))
}

/// The steps in front of the hall's door.
fn steps(hall: &Landmark) -> Place {
    let (i, j) = hall.entrance_edge;
    let (a, b) = (hall.corners[i], hall.corners[j]);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy).max(1e-6);
    let along = (dx / len, dy / len);
    // Out is away from the middle of the footprint
    let (mx, my) = (hall.door.0 - hall.position.0, hall.door.1 - hall.position.1);
    let normal = (-along.1, along.0);
    let out = if normal.0 * mx + normal.1 * my >= 0.0 { normal } else { (-normal.0, -normal.1) };
    // Short of the corners. Twenty out is well inside the market's reach
    // (`rules::market::HALL_RANGE`), so the steps are at the hall.
    let half = (len / 2.0 - 10.0).max(0.0);
    Place {
        kind: PlaceKind::HallSteps {
            door: hall.door,
            along,
            half,
            out,
        },
        at: hall.door,
    }
}

/// A point well inside the park, and how far it is from the edge: the
/// middle of its biggest ring if that's in it, else the inside point of a
/// coarse grid furthest from the edge.
fn inside(park: &capitol_protocol::terrain::TerrainPark) -> Option<(Point, f64)> {
    let ring = park.rings.iter().max_by(|a, b| {
        capitol_protocol::terrain::TerrainPark::signed_area(a)
            .abs()
            .total_cmp(&capitol_protocol::terrain::TerrainPark::signed_area(b).abs())
    })?;
    let n = ring.len().max(1) as f64;
    let mid = (ring.iter().map(|p| p.0).sum::<f64>() / n, ring.iter().map(|p| p.1).sum::<f64>() / n);
    let edge = |p: Point| {
        park.rings
            .iter()
            .flat_map(|r| (0..r.len()).map(move |k| segment_distance(p, r[k], r[(k + 1) % r.len()])))
            .fold(f64::MAX, f64::min)
    };
    if park.contains(mid.0, mid.1) && edge(mid) >= 30.0 {
        return Some((mid, edge(mid)));
    }
    let (min_x, max_x) = ring.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
    let (min_y, max_y) = ring.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    let mut best: Option<(Point, f64)> = None;
    for i in 1..12 {
        for j in 1..12 {
            let p = (min_x + (max_x - min_x) * i as f64 / 12.0, min_y + (max_y - min_y) * j as f64 / 12.0);
            if !park.contains(p.0, p.1) {
                continue;
            }
            let e = edge(p);
            if best.is_none_or(|(_, b)| e > b) {
                best = Some((p, e));
            }
        }
    }
    best
}

fn distance(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    distance(p, (a.0 + dx * t, a.1 + dy * t))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use capitol_protocol::rules::market::at_the_hall;
    use capitol_protocol::terrain::{LandmarkKind, TerrainResponse};

    use super::*;
    use crate::paths::WalkGraph;
    use crate::terrain::read_terrain_file;

    fn world() -> (TerrainResponse, WalkGraph) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain.json");
        let terrain = read_terrain_file(&path).unwrap();
        let graph = WalkGraph::new(&terrain);
        (terrain, graph)
    }

    /// A spread of draws, as a citizen's stream gives them.
    fn draws() -> impl Iterator<Item = (f64, f64)> {
        (0..10).flat_map(|i| (0..10).map(move |j| (i as f64 / 10.0 + 0.05, j as f64 / 10.0 + 0.05)))
    }

    #[test]
    fn the_world_has_steps_a_lane_parks_and_bridges() {
        let (terrain, graph) = world();
        let places = graph.places();
        assert!(places.hall_steps.is_some() && places.lane.is_some());
        assert!(places.parks.len() >= 20, "{} parks", places.parks.len());
        let bridges: usize = terrain.water_rings.iter().map(|r| r.bridges.len()).sum();
        assert_eq!(places.bridges.len(), bridges);
    }

    #[test]
    fn the_hall_steps_are_out_front_and_at_the_hall() {
        let (terrain, graph) = world();
        let hall = terrain.landmarks.iter().find(|l| l.kind == LandmarkKind::ExchangeHall).unwrap();
        let steps = graph.places().hall_steps.as_ref().unwrap();
        let mut spread = Vec::new();
        for (a, b) in draws() {
            let (p, look) = steps.spot(a, b);
            // Out of the building, within the market's reach, and walkable
            // from the door
            assert!(hall.distance_to(p.0, p.1) > 0.0, "{p:?} is inside the hall");
            assert!(at_the_hall(hall, p.0, p.1), "{p:?} is out of the market's reach");
            assert!(graph.walkable(hall.door, p), "{p:?}");
            // Looking away from the hall
            assert!(hall.distance_to(look.0, look.1) > hall.distance_to(p.0, p.1));
            spread.push(p);
        }
        let (lo, hi) = spread.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
        assert!(hi - lo > 50.0, "a crowd spreads along the steps: {lo}..{hi}");
    }

    #[test]
    fn every_park_spot_is_in_its_park_and_every_bridge_spot_on_its_deck() {
        let (terrain, graph) = world();
        let places = graph.places();
        for place in &places.parks {
            let PlaceKind::Park { id, .. } = &place.kind else { unreachable!() };
            let park = terrain.parks.iter().find(|p| &p.id == id).unwrap();
            for (a, b) in draws() {
                let (p, _) = place.spot(a, b);
                assert!(park.contains(p.0, p.1), "{id}: {p:?}");
            }
        }
        for place in &places.bridges {
            for (a, b) in draws() {
                let (p, look) = place.spot(a, b);
                assert!(graph.walkable(place.at, p), "{:?}: {p:?}", place.kind);
                // Looking along the water, not across the deck
                let (dx, dy) = (look.0 - p.0, look.1 - p.1);
                let (rx, ry) = (p.0 - terrain.center_x, p.1 - terrain.center_y);
                let radial = (dx * rx + dy * ry).abs() / (dx.hypot(dy) * rx.hypot(ry));
                assert!(radial < 0.2, "{:?} looks across, not along", place.kind);
            }
        }
    }

    #[test]
    fn free_time_offers_what_is_near() {
        let (terrain, graph) = world();
        let places = graph.places();
        let steps = places.hall_steps.as_ref().unwrap().at;
        let kinds = |menu: Vec<Choice>| menu.into_iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(" ");
        // By the hall: the lane and its stall, and at noon no bridge
        let noon = kinds(menu(places, steps, Some(0.5), Some("stall_3")));
        assert!(noon.starts_with("Rest Hall Lane Stall"), "{noon}");
        assert!(!noon.contains("Bridge"), "{noon}");
        // At dusk the bridge over the moat or a canal
        let dusk = kinds(menu(places, steps, Some(0.75), None));
        assert!(dusk.contains("Bridge") && !dusk.contains("Stall"), "{dusk}");
        // Out in the suburb, a park and no lane
        let park = &places.parks[0];
        let out = kinds(menu(places, park.at, Some(0.5), Some("stall_3")));
        assert!(out.contains("Park") && !out.contains("Lane") && !out.contains("Stall"), "{out}");
        // Far out in the wilds, only resting or the walk to the hall
        let wilds = (terrain.center_x, terrain.center_y + 5700.0);
        assert_eq!(menu(places, wilds, Some(0.75), None), vec![Choice::Rest, Choice::Hall]);
    }

    #[test]
    fn the_way_home_passes_a_bridge_or_it_doesnt() {
        let (terrain, graph) = world();
        let places = graph.places();
        let (cx, cy) = (terrain.center_x, terrain.center_y);
        // From the hall south down the avenue, over the canals' south bridges
        let hall = places.hall_steps.as_ref().unwrap().at;
        let menu = homeward(places, hall, (cx, cy + 2400.0)).unwrap();
        assert_eq!(menu[0], Choice::Home);
        assert!(matches!(&menu[1], Choice::Bridge { bridge } if bridge.ends_with("@90")), "{menu:?}");
        // Out in the fields, none
        assert_eq!(homeward(places, (cx, cy + 3300.0), (cx + 300.0, cy + 3400.0)), None);
    }
}
