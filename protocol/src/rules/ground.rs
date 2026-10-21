//! Moving on the land (W4, #197): how fast the going is, and what stops you.
//!
//! - **Slope.** [`land_speed`] scales the walk by the slope along the way
//!   you're heading: full speed on the flat, slower uphill, and a little
//!   slower downhill. Ground steeper than [`CLIFF_SLOPE`] stops you ("Too
//!   steep"), whichever way you face it.
//! - **Rivers and lakes** stop you at their edge, except at a ford or on a
//!   road: a road over water is its bridge (#222).
//! - **Streams and fords** are waded, at [`WADE_SPEED`].
//!
//! [`Ground`] holds the land's params and the wild water, indexed, and
//! [`Ground::step`] is one step of a walk. The server moves players with it
//! and the Flint client predicts with it, so a hill doesn't rubber-band.
//! The slope is the land's own ([`land::height_at`]), not the carved ground:
//! a bank cut by [`land::carve`] doesn't make a cliff.
//!
//! The water rings (moat and canals) aren't here: [`super::check_water_crossing`]
//! still tests those.

use std::collections::HashMap;

use crate::fmath::StableMath;
use crate::land::{self, LandParams};
use crate::terrain::{Ford, Lake, RoadSegment, TerrainResponse, WaterCourse};

/// The steepest ground you can walk onto: rise over run, so 1.2 is about
/// 50°. About one part in twenty of the wilds is steeper: crests and crags.
/// Straight-line walks have no pathfinding, so it's kind on purpose.
pub const CLIFF_SLOPE: f64 = 1.2;
/// Your speed in a stream, or wading a river at a ford, as a share of the
/// speed on dry ground there.
pub const WADE_SPEED: f64 = 0.5;
/// What `movement_blocked` says when the slope stopped you.
pub const TOO_STEEP: &str = "Too steep";

/// How much a climb slows you: at a rise of 1 in 2 you go at 1 / 1.75.
const UPHILL_DRAG: f64 = 1.5;
/// How much a descent slows you, less than a climb.
const DOWNHILL_DRAG: f64 = 0.5;
/// How far apart a step's water tests are: a river narrower than this can
/// slip between them, and only the last trickle of one petering out is.
const SAMPLE: f64 = 1.0;
/// The side of the index's buckets.
const BUCKET: f64 = 60.0;

/// The share of your speed the slope leaves you. `slope` is the rise over
/// run along the way you're heading: positive uphill, negative downhill.
pub fn land_speed(slope: f64) -> f64 {
    if slope >= 0.0 {
        1.0 / (1.0 + UPHILL_DRAG * slope)
    } else {
        1.0 / (1.0 - DOWNHILL_DRAG * slope)
    }
}

/// What's underfoot at a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Footing<'a> {
    /// Dry ground, or a road over the water.
    Dry,
    /// A stream, or a river at a ford: you wade. Names the water.
    Wading(&'a str),
    /// A river away from its fords, or a lake: you can't go in. Names it.
    Blocked(&'a str),
}

/// What stopped a step.
#[derive(Debug, Clone, PartialEq)]
pub enum Blocked {
    /// Ground steeper than [`CLIFF_SLOPE`].
    Steep,
    /// A river or a lake, by id (`river_3`, `lake_1`).
    Water(String),
}

impl Blocked {
    /// For `movement_blocked`'s `water_id`: the water's id, empty for a slope.
    pub fn water_id(&self) -> &str {
        match self {
            Blocked::Steep => "",
            Blocked::Water(id) => id,
        }
    }

    /// For people, as `movement_blocked`'s `reason`.
    pub fn reason(&self) -> String {
        match self {
            Blocked::Steep => TOO_STEEP.into(),
            Blocked::Water(id) if id.starts_with("lake_") => {
                format!("Cannot cross {}: go round it", super::water_name(id))
            }
            Blocked::Water(id) => format!("Cannot cross {} without a ford or a bridge", super::water_name(id)),
        }
    }
}

/// One step of a walk.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Where the step ends: short of what stopped it, if anything did.
    pub x: f64,
    pub y: f64,
    pub blocked: Option<Blocked>,
}

/// The land and the wild water, indexed for walking on.
#[derive(Debug, Clone, Default)]
pub struct Ground {
    land: LandParams,
    /// Each river's and stream's id, and whether it's a river.
    courses: Vec<(String, bool)>,
    segments: Vec<Segment>,
    lakes: Vec<LakeArea>,
    fords: Vec<FordArea>,
    /// The roads that cross the water.
    roads: Vec<RoadSpan>,
    /// Buckets of [`BUCKET`] over the world, row-major, `cols` across.
    buckets: Vec<Bucket>,
    cols: usize,
}

/// A stretch of road over the water: (a, b, half width).
type RoadSpan = ((f64, f64), (f64, f64), f64);

/// A stretch of a river or stream, with its half width at each end.
#[derive(Debug, Clone)]
struct Segment {
    a: (f64, f64),
    b: (f64, f64),
    half_a: f64,
    half_b: f64,
    course: u32,
}

#[derive(Debug, Clone)]
struct LakeArea {
    id: String,
    min: (f64, f64),
    max: (f64, f64),
    rings: Vec<Vec<(f64, f64)>>,
}

/// A ford, as a disc on its river.
#[derive(Debug, Clone)]
struct FordArea {
    course: u32,
    at: (f64, f64),
    radius: f64,
}

#[derive(Debug, Clone, Default)]
struct Bucket {
    segments: Vec<u32>,
    lakes: Vec<u32>,
    fords: Vec<u32>,
    roads: Vec<u32>,
}

/// The water at a point, before fords and roads have their say.
#[derive(Default)]
struct Wet {
    lake: Option<u32>,
    river: Option<u32>,
    stream: Option<u32>,
}

impl Ground {
    /// Indexes the wild water. Only the roads that cross it are kept.
    pub fn new(
        land: &LandParams,
        rivers: &[WaterCourse],
        streams: &[WaterCourse],
        lakes: &[Lake],
        fords: &[Ford],
        roads: &[RoadSegment],
    ) -> Self {
        let cols = (super::WORLD_SIZE / BUCKET).ceil() as usize;
        let mut ground = Ground {
            land: land.clone(),
            buckets: vec![Bucket::default(); cols * cols],
            cols,
            ..Ground::default()
        };
        let mut by_id = HashMap::new();
        for (course, river) in rivers.iter().map(|c| (c, true)).chain(streams.iter().map(|c| (c, false))) {
            let k = ground.courses.len() as u32;
            by_id.insert(course.id.as_str(), k);
            ground.courses.push((course.id.clone(), river));
            for w in course.points.windows(2) {
                let (p, q) = (w[0], w[1]);
                let s = ground.segments.len() as u32;
                ground.segments.push(Segment {
                    a: (p.x, p.y),
                    b: (q.x, q.y),
                    half_a: p.width / 2.0,
                    half_b: q.width / 2.0,
                    course: k,
                });
                let pad = p.width.max(q.width) / 2.0;
                ground.each_bucket((p.x.min(q.x), p.y.min(q.y)), (p.x.max(q.x), p.y.max(q.y)), pad, |b| {
                    b.segments.push(s)
                });
            }
        }
        for lake in lakes {
            let points = lake.rings.iter().flatten();
            let min = points.clone().fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
            let max = points.fold((f64::MIN, f64::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
            let k = ground.lakes.len() as u32;
            ground.lakes.push(LakeArea { id: lake.id.clone(), min, max, rings: lake.rings.clone() });
            ground.each_bucket(min, max, 0.0, |b| b.lakes.push(k));
        }
        for ford in fords {
            let Some(&course) = by_id.get(ford.river_id.as_str()) else { continue };
            let k = ground.fords.len() as u32;
            let radius = ford.width / 2.0;
            ground.fords.push(FordArea { course, at: (ford.x, ford.y), radius });
            ground.each_bucket((ford.x, ford.y), (ford.x, ford.y), radius, |b| b.fords.push(k));
        }
        for road in roads {
            let half = road.width / 2.0;
            for w in road.points.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (min, max) = ((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)));
                let mut wet = false;
                ground.each_bucket(min, max, half, |bucket| {
                    wet |= !bucket.segments.is_empty() || !bucket.lakes.is_empty()
                });
                if !wet {
                    continue;
                }
                let k = ground.roads.len() as u32;
                ground.roads.push((a, b, half));
                ground.each_bucket(min, max, half, |bucket| bucket.roads.push(k));
            }
        }
        ground
    }

    /// The ground a `/terrain` response describes.
    pub fn from_terrain(terrain: &TerrainResponse) -> Self {
        Self::new(&terrain.land, &terrain.rivers, &terrain.streams, &terrain.lakes, &terrain.fords, &terrain.roads)
    }

    /// The land's params.
    pub fn land(&self) -> &LandParams {
        &self.land
    }

    /// The wild water at `(x, y)`, if any, by id: a lake, else a river, else
    /// a stream. Fords and roads don't count, so nothing is built in a ford
    /// or on a bridge, as with the rings ([`super::in_water`]).
    pub fn water_at(&self, x: f64, y: f64) -> Option<&str> {
        let wet = self.wet(x, y);
        if let Some(k) = wet.lake {
            return Some(&self.lakes[k as usize].id);
        }
        wet.river.or(wet.stream).map(|k| self.courses[k as usize].0.as_str())
    }

    /// What's underfoot at `(x, y)`.
    pub fn footing(&self, x: f64, y: f64) -> Footing<'_> {
        let wet = self.wet(x, y);
        if wet.lake.is_none() && wet.river.is_none() && wet.stream.is_none() {
            return Footing::Dry;
        }
        if self.on_road(x, y) {
            return Footing::Dry;
        }
        if let Some(k) = wet.lake {
            return Footing::Blocked(&self.lakes[k as usize].id);
        }
        if let Some(k) = wet.river {
            let id = self.courses[k as usize].0.as_str();
            return if self.at_ford(k, x, y) { Footing::Wading(id) } else { Footing::Blocked(id) };
        }
        match wet.stream {
            Some(k) => Footing::Wading(&self.courses[k as usize].0),
            None => Footing::Dry,
        }
    }

    /// The share of full speed you walk at from `(x, y)`, heading along the
    /// unit vector `heading`: the slope's [`land_speed`], and [`WADE_SPEED`]
    /// in the water.
    pub fn speed_at(&self, x: f64, y: f64, heading: (f64, f64)) -> f64 {
        let (gx, gy) = land::gradient_at(x, y, &self.land);
        let speed = land_speed(gx * heading.0 + gy * heading.1);
        match self.footing(x, y) {
            Footing::Wading(_) => speed * WADE_SPEED,
            _ => speed,
        }
    }

    /// One step of a walk from `from` toward `dest`, worth `walk` units on
    /// the flat (the walker's speed times the time). The step is slowed by
    /// [`Self::speed_at`] where it starts, never passes `dest`, and stops:
    ///
    /// - where it stands, if it would end on ground steeper than
    ///   [`CLIFF_SLOPE`] and steeper than where it starts. Someone already
    ///   on a crag (put there before W4, or by a regenerate) can climb down.
    /// - within [`SAMPLE`] of a river's or lake's edge, unless it starts in
    ///   the water: then it may wade out.
    pub fn step(&self, from: (f64, f64), dest: (f64, f64), walk: f64) -> Step {
        let (ex, ey) = (dest.0 - from.0, dest.1 - from.1);
        let remaining = ex.stable_hypot(ey);
        if remaining <= 0.0 {
            return Step { x: dest.0, y: dest.1, blocked: None };
        }
        let heading = (ex / remaining, ey / remaining);
        let travel = (walk * self.speed_at(from.0, from.1, heading)).min(remaining);
        let to = if travel >= remaining { dest } else { (from.0 + heading.0 * travel, from.1 + heading.1 * travel) };
        let stay = |blocked| Step { x: from.0, y: from.1, blocked: Some(blocked) };

        let steep = land::slope_at(to.0, to.1, &self.land);
        if steep > CLIFF_SLOPE && steep > land::slope_at(from.0, from.1, &self.land) {
            return stay(Blocked::Steep);
        }

        if !matches!(self.footing(from.0, from.1), Footing::Blocked(_)) {
            let n = (travel / SAMPLE).ceil().max(1.0) as usize;
            let mut last = from;
            for i in 1..=n {
                let t = travel * i as f64 / n as f64;
                let p = if i == n { to } else { (from.0 + heading.0 * t, from.1 + heading.1 * t) };
                if let Footing::Blocked(id) = self.footing(p.0, p.1) {
                    return Step { x: last.0, y: last.1, blocked: Some(Blocked::Water(id.to_string())) };
                }
                last = p;
            }
        }
        Step { x: to.0, y: to.1, blocked: None }
    }

    fn wet(&self, x: f64, y: f64) -> Wet {
        let mut wet = Wet::default();
        let Some(bucket) = self.bucket(x, y) else { return wet };
        for &k in &bucket.lakes {
            let lake = &self.lakes[k as usize];
            if (lake.min.0..=lake.max.0).contains(&x) && (lake.min.1..=lake.max.1).contains(&y) && rings_contain(&lake.rings, x, y) {
                wet.lake = Some(k);
                return wet;
            }
        }
        for &s in &bucket.segments {
            let seg = &self.segments[s as usize];
            let (t, d) = nearest_on_segment(seg.a, seg.b, x, y);
            if d < seg.half_a + t * (seg.half_b - seg.half_a) {
                let (_, river) = self.courses[seg.course as usize];
                if river {
                    wet.river = Some(seg.course);
                } else {
                    wet.stream.get_or_insert(seg.course);
                }
            }
        }
        wet
    }

    fn at_ford(&self, course: u32, x: f64, y: f64) -> bool {
        self.bucket(x, y).is_some_and(|b| {
            b.fords.iter().any(|&k| {
                let f = &self.fords[k as usize];
                f.course == course && (x - f.at.0).stable_hypot(y - f.at.1) <= f.radius
            })
        })
    }

    fn on_road(&self, x: f64, y: f64) -> bool {
        self.bucket(x, y).is_some_and(|b| {
            b.roads.iter().any(|&k| {
                let (a, c, half) = self.roads[k as usize];
                nearest_on_segment(a, c, x, y).1 <= half
            })
        })
    }

    fn each_bucket(&mut self, min: (f64, f64), max: (f64, f64), pad: f64, mut add: impl FnMut(&mut Bucket)) {
        let last = self.cols as i64 - 1;
        let cell = |v: f64| ((v / BUCKET).floor() as i64).clamp(0, last) as usize;
        for j in cell(min.1 - pad)..=cell(max.1 + pad) {
            for i in cell(min.0 - pad)..=cell(max.0 + pad) {
                add(&mut self.buckets[j * self.cols + i]);
            }
        }
    }

    fn bucket(&self, x: f64, y: f64) -> Option<&Bucket> {
        let size = super::WORLD_SIZE;
        if self.cols == 0 || !(0.0..size).contains(&x) || !(0.0..size).contains(&y) {
            return None;
        }
        self.buckets.get((y / BUCKET) as usize * self.cols + (x / BUCKET) as usize)
    }
}

/// The wild water at `(x, y)` on a `/terrain` response, as
/// [`Ground::water_at`] says, without building the index: for a one-off
/// test, such as whether a bench can go down where the cursor is.
pub fn water_at(terrain: &TerrainResponse, x: f64, y: f64) -> Option<&str> {
    if let Some(lake) = terrain.lakes.iter().find(|lake| lake.contains(x, y)) {
        return Some(&lake.id);
    }
    let wet = |course: &&WaterCourse| {
        course.points.windows(2).any(|w| {
            let (p, q) = (w[0], w[1]);
            let reach = p.width.max(q.width) / 2.0;
            if x < p.x.min(q.x) - reach || x > p.x.max(q.x) + reach || y < p.y.min(q.y) - reach || y > p.y.max(q.y) + reach {
                return false;
            }
            let (t, d) = nearest_on_segment((p.x, p.y), (q.x, q.y), x, y);
            d < (p.width + t * (q.width - p.width)) / 2.0
        })
    };
    terrain.rivers.iter().find(wet).or_else(|| terrain.streams.iter().find(wet)).map(|c| c.id.as_str())
}

/// Where on segment `ab` is nearest `(x, y)`, as (t along it, distance).
fn nearest_on_segment(a: (f64, f64), b: (f64, f64), x: f64, y: f64) -> (f64, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (px, py) = (a.0 + t * dx - x, a.1 + t * dy - y);
    (t, (px * px + py * py).sqrt())
}

/// Even-odd over closed rings, as `Lake::contains`.
fn rings_contain(rings: &[Vec<(f64, f64)>], x: f64, y: f64) -> bool {
    let mut inside = false;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
                inside = !inside;
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmath::StableMath;
    use crate::terrain::{Mouth, RoadType, WaterPoint};

    /// Flat land: no foothills anywhere in the world.
    fn flat() -> LandParams {
        LandParams { foothill_radius: 1.0e9, ..LandParams::default() }
    }

    fn course(id: &str, width: f64, points: &[(f64, f64)]) -> WaterCourse {
        WaterCourse {
            id: id.into(),
            source: None,
            mouth: Mouth::Edge,
            points: points.iter().map(|&(x, y)| WaterPoint { x, y, width, depth: 2.0, surface: 0.0 }).collect(),
        }
    }

    /// A river running north to south at x = 1000, 20 wide, with a ford at
    /// y = 1000; a stream at x = 2000, 6 wide; a square lake round
    /// (3000, 1000); and a road over the river at y = 1500.
    fn ground() -> Ground {
        let river = course("river_0", 20.0, &[(1000.0, 500.0), (1000.0, 2000.0)]);
        let stream = course("stream_0", 6.0, &[(2000.0, 500.0), (2000.0, 2000.0)]);
        let lake = Lake {
            id: "lake_0".into(),
            level: 0.0,
            rings: vec![vec![(2900.0, 900.0), (3100.0, 900.0), (3100.0, 1100.0), (2900.0, 1100.0)]],
            area: 40000.0,
        };
        let ford = Ford { id: "ford_0".into(), river_id: "river_0".into(), x: 1000.0, y: 1000.0, width: 30.0, depth: 1.0 };
        let road = RoadSegment {
            id: "trail_0".into(),
            road_type: RoadType::Trail,
            points: vec![(900.0, 1500.0), (1100.0, 1500.0)],
            width: 12.0,
        };
        Ground::new(&flat(), &[river], &[stream], &[lake], &[ford], &[road])
    }

    /// Walks from `from` to `to` a tick of 5 at a time; where it ended and
    /// what stopped it.
    fn walk(ground: &Ground, from: (f64, f64), to: (f64, f64)) -> Step {
        let mut at = from;
        for _ in 0..1000 {
            let step = ground.step(at, to, 5.0);
            if step.blocked.is_some() || (step.x, step.y) == to {
                return step;
            }
            at = (step.x, step.y);
        }
        panic!("never got there");
    }

    #[test]
    fn the_going_is_slower_up_than_down_and_both_than_the_flat() {
        assert_eq!(land_speed(0.0), 1.0);
        assert!(land_speed(0.5) < land_speed(-0.5));
        assert!(land_speed(-0.5) < 1.0);
        assert!(land_speed(1.0) < land_speed(0.5));
        assert!((land_speed(0.5) - 1.0 / 1.75).abs() < 1e-12);
    }

    #[test]
    fn what_is_underfoot() {
        let g = ground();
        assert_eq!(g.footing(1000.0, 700.0), Footing::Blocked("river_0"));
        assert_eq!(g.footing(1011.0, 700.0), Footing::Dry, "past the bank");
        assert_eq!(g.footing(1000.0, 1010.0), Footing::Wading("river_0"), "at the ford");
        assert_eq!(g.footing(1000.0, 1500.0), Footing::Dry, "on the road over it");
        assert_eq!(g.footing(2000.0, 700.0), Footing::Wading("stream_0"));
        assert_eq!(g.footing(3000.0, 1000.0), Footing::Blocked("lake_0"));
        assert_eq!(g.footing(-5.0, 1000.0), Footing::Dry, "off the map");
        // Building goes by the water alone: not in a ford, not on a bridge.
        assert_eq!(g.water_at(1000.0, 1010.0), Some("river_0"));
        assert_eq!(g.water_at(1000.0, 1500.0), Some("river_0"));
        assert_eq!(g.water_at(2001.0, 700.0), Some("stream_0"));
        assert_eq!(g.water_at(1500.0, 700.0), None);
    }

    #[test]
    fn the_one_off_test_agrees_with_the_index() {
        let g = ground();
        let mut terrain: TerrainResponse = serde_json::from_str(
            r#"{"world_size":12000.0,"center_x":6000.0,"center_y":6000.0,"water_rings":[],"roads":[],"terrain_plots":[]}"#,
        )
        .unwrap();
        terrain.rivers = vec![course("river_0", 20.0, &[(1000.0, 500.0), (1000.0, 2000.0)])];
        terrain.streams = vec![course("stream_0", 6.0, &[(2000.0, 500.0), (2000.0, 2000.0)])];
        terrain.lakes = vec![Lake {
            id: "lake_0".into(),
            level: 0.0,
            rings: vec![vec![(2900.0, 900.0), (3100.0, 900.0), (3100.0, 1100.0), (2900.0, 1100.0)]],
            area: 40000.0,
        }];
        for y in (400..2100).step_by(37) {
            for x in (900..3200).step_by(3) {
                let (x, y) = (x as f64 + 0.5, y as f64);
                assert_eq!(water_at(&terrain, x, y), g.water_at(x, y), "at ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_river_stops_you_at_its_bank() {
        let g = ground();
        let step = walk(&g, (950.0, 700.0), (1050.0, 700.0));
        assert_eq!(step.blocked, Some(Blocked::Water("river_0".into())));
        assert!(step.x <= 990.0 && step.x >= 989.0, "stopped within a unit of the bank, at {}", step.x);
        assert_eq!(step.blocked.unwrap().reason(), "Cannot cross the river without a ford or a bridge");
        // A walk that only touches the bank isn't stopped by it.
        assert_eq!(walk(&g, (950.0, 700.0), (989.5, 700.0)).blocked, None);
    }

    #[test]
    fn a_ford_and_a_bridge_let_you_over() {
        let g = ground();
        assert_eq!(walk(&g, (950.0, 1000.0), (1050.0, 1000.0)).blocked, None);
        assert_eq!(walk(&g, (950.0, 1500.0), (1050.0, 1500.0)).blocked, None);
    }

    #[test]
    fn a_stream_is_waded_slowly() {
        let g = ground();
        assert_eq!(walk(&g, (1950.0, 700.0), (2050.0, 700.0)).blocked, None);
        let dry = g.step((1950.0, 700.0), (2050.0, 700.0), 5.0);
        let wet = g.step((1998.0, 700.0), (2050.0, 700.0), 5.0);
        assert!((dry.x - 1955.0).abs() < 1e-9);
        assert!((wet.x - (1998.0 + 5.0 * WADE_SPEED)).abs() < 1e-9);
    }

    #[test]
    fn a_lake_stops_you_and_says_go_round() {
        let g = ground();
        let step = walk(&g, (2800.0, 1000.0), (3000.0, 1000.0));
        assert_eq!(step.blocked.as_ref().map(Blocked::water_id), Some("lake_0"));
        assert_eq!(step.blocked.unwrap().reason(), "Cannot cross the lake: go round it");
    }

    #[test]
    fn someone_in_the_water_can_wade_out() {
        let g = ground();
        assert_eq!(walk(&g, (1000.0, 700.0), (950.0, 700.0)).blocked, None);
    }

    /// On the real land: a climb is slower than the flat, and a crag stops
    /// you short of it but lets someone on it climb down.
    #[test]
    fn the_land_slows_a_climb_and_a_crag_stops_it() {
        let land = LandParams::default();
        let g = Ground::new(&land, &[], &[], &[], &[], &[]);
        // In the city it's flat: a whole step.
        let flat = g.step((6000.0, 9300.0), (6000.0, 9400.0), 5.0);
        assert_eq!((flat.x, flat.y), (6000.0, 9305.0));

        // Find a crag in the wilds, and the way up to it.
        let mut crag = None;
        'search: for i in 0..400 {
            let a = i as f64 * 0.9_f64.to_radians();
            for r in [5600.0, 5800.0, 6000.0, 6200.0] {
                let p = (crate::rules::CENTER_X + r * a.stable_cos(), crate::rules::CENTER_Y + r * a.stable_sin());
                if land::slope_at(p.0, p.1, &land) > CLIFF_SLOPE + 0.3 {
                    crag = Some(p);
                    break 'search;
                }
            }
        }
        let crag = crag.expect("the wilds have a crag");
        // Walk downhill from it until the ground is gentle, then turn round.
        let mut foot = crag;
        while land::slope_at(foot.0, foot.1, &land) > 0.6 {
            let (gx, gy) = land::gradient_at(foot.0, foot.1, &land);
            let len = gx.stable_hypot(gy);
            foot = (foot.0 - gx / len, foot.1 - gy / len);
        }
        let up = walk(&g, foot, crag);
        assert_eq!(up.blocked, Some(Blocked::Steep));
        assert!(land::slope_at(up.x, up.y, &land) <= CLIFF_SLOPE + 0.05);
        assert_eq!(Blocked::Steep.reason(), "Too steep");
        assert_eq!(Blocked::Steep.water_id(), "");

        // The first step up the slope is shorter than a step on the flat.
        let (gx, gy) = land::gradient_at(foot.0, foot.1, &land);
        let len = gx.stable_hypot(gy);
        let first = g.step(foot, (foot.0 + gx / len * 100.0, foot.1 + gy / len * 100.0), 5.0);
        assert!((first.x - foot.0).stable_hypot(first.y - foot.1) < 4.0, "a climb at {:.2} goes the full step", len);

        // From the crag itself, the way down is open.
        let down = g.step(crag, foot, 5.0);
        assert_eq!(down.blocked, None);
    }
}
