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
//! - **The moat and the canals** stop you short of their water, except on a
//!   bridge, as [`super::ring_crossing`] says (T0.1, #315).
//! - **The edge of the world** ([`super::in_world`]) stops you where you
//!   stand.
//!
//! [`Ground`] holds the land's params, the wild water, indexed, and the
//! rings. [`Ground::walk_step`] is one step of a walk on a heading, and
//! [`Ground::step`] one toward a point. The server moves players with them
//! and the Flint client predicts with them, so a hill or a canal doesn't
//! rubber-band. The slope is the land's own ([`land::height_at`]), not the
//! carved ground: a bank cut by [`land::carve`] doesn't make a cliff.
//!
//! A blocked step **slides** (controls plan D4): the heading turned 30° and
//! then 60° either way, each going cos of the turn as far, and the one that
//! gets furthest on the way you meant to go wins. So a walk into a bank at
//! a slant runs along it, and a walk square into one stops.

use std::collections::HashMap;

use crate::fmath::StableMath;
use crate::land::{self, LandParams};
use crate::terrain::{Ford, Lake, RoadSegment, TerrainResponse, WaterCourse, WaterRing};

/// The steepest ground you can walk onto: rise over run, so 1.2 is about
/// 50°. About one part in twenty of the wilds is steeper: crests and crags.
/// Straight-line walks have no pathfinding, so it's kind on purpose.
pub const CLIFF_SLOPE: f64 = 1.2;
/// Your speed in a stream, or wading a river at a ford, as a share of the
/// speed on dry ground there.
pub const WADE_SPEED: f64 = 0.5;
/// What `movement_blocked` says when the slope stopped you.
pub const TOO_STEEP: &str = "Too steep";
/// What `movement_blocked` says at the edge of the world.
pub const WORLD_EDGE: &str = "The world ends here";

/// How much a climb slows you: at a rise of 1 in 2 you go at 1 / 1.75.
const UPHILL_DRAG: f64 = 1.5;
/// How much a descent slows you, less than a climb.
const DOWNHILL_DRAG: f64 = 0.5;
/// How far apart a step's water tests are: a river narrower than this can
/// slip between them, and only the last trickle of one petering out is.
const SAMPLE: f64 = 1.0;
/// The side of the index's buckets.
const BUCKET: f64 = 60.0;
/// cos 30°, which is sin 60°: a constant, so no trig.
const COS_30: f64 = 0.866_025_403_784_438_6;
/// The turns a slide tries, nearest first, as (cos, sin): 30° and 60°.
const SLIDE: [(f64, f64); 2] = [(COS_30, 0.5), (0.5, COS_30)];
/// How far apart [`slope_lets_through`] tests a line: on gentle ground, and
/// where the slope is over [`STEEP`]. The server's step can stop anywhere
/// along the line, and a crest can be steeper than [`CLIFF_SLOPE`] for under
/// a unit (#280).
const SLOPE_SAMPLE: f64 = 2.0;
const STEEP_SAMPLE: f64 = 0.5;
const STEEP: f64 = 0.8;
/// How far under [`CLIFF_SLOPE`] a rising slope already stops a line, so a
/// crest that peaks between two tests can't slip through. Over 20,000 lines
/// in the ranges, these let none through that a test every 0.05 stops.
const STEEP_MARGIN: f64 = 0.03;
/// A stride whose slope changes by more than this is tested again every
/// [`KINK_SAMPLE`] along it. At a valley's floor the slope has a kink, and
/// on the far side it can climb from 0.7 past the cliff within a unit, so a
/// stride of [`SLOPE_SAMPLE`] from the gentle side stepped over a face the
/// server's step stops on (#520). It costs about 5% more samples.
const KINK: f64 = 0.15;
const KINK_SAMPLE: f64 = 0.25;

/// Whether the slope lets a straight walk from `a` to `b` through, given the
/// land's slope at a point (`land::slope_at`, or `land::Relief::slope`, the
/// same to the bit). [`Ground::step`] stops a step that ends on ground
/// steeper than [`CLIFF_SLOPE`] and steeper than where it starts, so a line
/// stops where the slope rises past a little under it: the server's steps
/// fall between the samples, wherever on the line its walk began. A walk
/// that starts on a crag may climb down off it. A stride whose slope jumps by
/// more than [`KINK`] is tested again more finely, for the face past a
/// valley's floor (#520).
///
/// The Flint client's walk graph tests its lines with it (#280), and the
/// backend plants wild nodes only where its lines reach (#347, #567).
pub fn slope_lets_through(a: (f64, f64), b: (f64, f64), slope_at: impl Fn(f64, f64) -> f64) -> bool {
    let length = (b.0 - a.0).stable_hypot(b.1 - a.1);
    let at = |along: f64| {
        let t = along / length;
        slope_at(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    let rises = |here: f64, before: f64| here > CLIFF_SLOPE - STEEP_MARGIN && here > before;
    let mut slope = slope_at(a.0, a.1);
    let mut along = 0.0;
    while along < length {
        let from = along;
        along = (along + if slope > STEEP { STEEP_SAMPLE } else { SLOPE_SAMPLE }).min(length);
        let here = at(along);
        let mut last = slope;
        if (here - slope).abs() > KINK {
            let n = ((along - from) / KINK_SAMPLE).ceil() as usize;
            for k in 1..n {
                let between = at(from + (along - from) * k as f64 / n as f64);
                if rises(between, last) {
                    return false;
                }
                last = between;
            }
        }
        if rises(here, last) {
            return false;
        }
        slope = here;
    }
    true
}

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
    /// The moat or a canal away from its bridges, by id (`moat`, `canal_inner`).
    Ring(String),
    /// The edge of the world.
    Edge,
}

impl Blocked {
    /// For `movement_blocked`'s `water_id`: the water's id, empty for a
    /// slope or the edge.
    pub fn water_id(&self) -> &str {
        match self {
            Blocked::Steep | Blocked::Edge => "",
            Blocked::Water(id) | Blocked::Ring(id) => id,
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
            Blocked::Ring(id) => format!("Cannot cross {} without a bridge", super::water_name(id)),
            Blocked::Edge => WORLD_EDGE.into(),
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

/// The land, the wild water, indexed, and the rings, for walking on.
#[derive(Debug, Clone, Default)]
pub struct Ground {
    land: LandParams,
    /// The moat and the canals, round `center` ([`Self::with_rings`]).
    rings: Vec<WaterRing>,
    center: (f64, f64),
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

    /// The moat and the canals round `center`: a step stops at them away
    /// from their bridges, as at a river. Without them only the land and
    /// the wild water stop it.
    pub fn with_rings(mut self, center: (f64, f64), rings: &[WaterRing]) -> Self {
        self.center = center;
        self.rings = rings.to_vec();
        self
    }

    /// The ground a `/terrain` response describes, rings and all.
    pub fn from_terrain(terrain: &TerrainResponse) -> Self {
        Self::new(&terrain.land, &terrain.rivers, &terrain.streams, &terrain.lakes, &terrain.fords, &terrain.roads)
            .with_rings((terrain.center_x, terrain.center_y), &terrain.water_rings)
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

    /// One step of a walk from `from` along `heading` (a direction, any
    /// length), worth `walk` units on the flat: the walker's speed times the
    /// time. The step is slowed by [`Self::speed_at`] where it starts, and
    /// stopped, in this order, by:
    ///
    /// 1. **the land.** It stays where it stands if it would end on ground
    ///    steeper than [`CLIFF_SLOPE`] and steeper than where it starts, so
    ///    someone already on a crag (put there before W4, or by a
    ///    regenerate) can climb down. It stops within [`SAMPLE`] of a
    ///    river's or lake's edge, unless it starts in the water: then it may
    ///    wade out.
    /// 2. **the moat and the canals** away from their bridges: 2 short of the
    ///    water along the way, as [`super::ring_crossing`] says. That point
    ///    is the same however long the step, so a prediction in short steps
    ///    stops where the server's ticks do. A step off a bridge's side into
    ///    the water stays where it is.
    /// 3. **the edge of the world**: it stays where it is.
    ///
    /// Then, if it was stopped, it **slides**: the heading turned 30° and 60°
    /// left and right, each walking cos of its turn as far. Of those not
    /// stopped, the one that goes furthest times that cos wins, left over
    /// right on a tie, if that beats how far the stopped step itself got on
    /// the way. So a walk up to a bank reaches it, and slides from the next
    /// step, and a walk to a point (click to move, which ends at a stop)
    /// slides only when it starts against one. Nor may a slide end within 2
    /// of a ring's water off a
    /// bridge, or of the edge, nearer to it than it started: a stop there
    /// pushes you back out or holds you, so a square push would jitter
    /// along it. If none goes, the straight step's stop stands.
    pub fn walk_step(&self, from: (f64, f64), heading: (f64, f64), walk: f64) -> Step {
        let len = heading.0.stable_hypot(heading.1);
        if len <= 0.0 {
            return Step { x: from.0, y: from.1, blocked: None };
        }
        self.walk(from, (heading.0 / len, heading.1 / len), walk, f64::INFINITY, None)
    }

    /// One step of a walk from `from` toward `dest`, worth `walk` units on
    /// the flat: [`Self::walk_step`] on the heading to `dest`, which never
    /// passes it, and ends on it exactly when it gets there.
    pub fn step(&self, from: (f64, f64), dest: (f64, f64), walk: f64) -> Step {
        let (ex, ey) = (dest.0 - from.0, dest.1 - from.1);
        let remaining = ex.stable_hypot(ey);
        if remaining <= 0.0 {
            return Step { x: dest.0, y: dest.1, blocked: None };
        }
        self.walk(from, (ex / remaining, ey / remaining), walk, remaining, Some(dest))
    }

    /// [`Self::walk_step`] on a unit `heading`, going no further than `cap`.
    /// A straight step that goes all of `cap` ends on `dest`, if there is one.
    fn walk(&self, from: (f64, f64), heading: (f64, f64), walk: f64, cap: f64, dest: Option<(f64, f64)>) -> Step {
        let straight = self.straight(from, heading, walk, cap, dest);
        if straight.blocked.is_none() {
            return straight;
        }
        let (hx, hy) = heading;
        // How far the stopped step got: back, if a ring pushed it out.
        let got = (straight.x - from.0) * hx + (straight.y - from.1) * hy;
        let mut best: Option<(f64, Step)> = None;
        for (cos, sin) in SLIDE {
            // Left, then right. y points south, so left is anticlockwise on
            // the map.
            for turned in [(hx * cos + hy * sin, hy * cos - hx * sin), (hx * cos - hy * sin, hy * cos + hx * sin)] {
                let step = self.straight(from, turned, walk * cos, cap * cos, None);
                if step.blocked.is_some() || self.hugs(from, (step.x, step.y)) {
                    continue;
                }
                let score = (step.x - from.0).stable_hypot(step.y - from.1) * cos;
                if score > got.max(0.0) && best.as_ref().is_none_or(|(b, _)| score > *b) {
                    best = Some((score, step));
                }
            }
        }
        best.map_or(straight, |(_, step)| step)
    }

    /// One straight step on a unit `heading`, with no slide.
    fn straight(&self, from: (f64, f64), heading: (f64, f64), walk: f64, cap: f64, dest: Option<(f64, f64)>) -> Step {
        let travel = (walk * self.speed_at(from.0, from.1, heading)).min(cap);
        let to = match dest {
            Some(dest) if travel >= cap => dest,
            _ => (from.0 + heading.0 * travel, from.1 + heading.1 * travel),
        };
        let stay = |blocked| Step { x: from.0, y: from.1, blocked: Some(blocked) };

        let steep = land::slope_at(to.0, to.1, &self.land);
        if steep > CLIFF_SLOPE && steep > land::slope_at(from.0, from.1, &self.land) {
            return stay(Blocked::Steep);
        }

        let mut end = Step { x: to.0, y: to.1, blocked: None };
        if !matches!(self.footing(from.0, from.1), Footing::Blocked(_)) {
            let n = (travel / SAMPLE).ceil().max(1.0) as usize;
            let mut last = from;
            for i in 1..=n {
                let t = travel * i as f64 / n as f64;
                let p = if i == n { to } else { (from.0 + heading.0 * t, from.1 + heading.1 * t) };
                if let Footing::Blocked(id) = self.footing(p.0, p.1) {
                    end = Step { x: last.0, y: last.1, blocked: Some(Blocked::Water(id.to_string())) };
                    break;
                }
                last = p;
            }
        }

        let till = (end.x, end.y);
        let crossing = self.rings.iter().find_map(|ring| {
            super::ring_crossing(self.center, ring, from, till).map(|(x, y)| (x, y, &ring.id))
        });
        if let Some((x, y, id)) = crossing {
            end = Step { x, y, blocked: Some(Blocked::Ring(id.clone())) };
        }
        // Off a bridge's side into the water, or stopped short of an edge
        // off the bridge but still over the water: stay on the bridge.
        if self.ring_water_at(from).is_none() {
            if let Some(ring) = self.ring_water_at((end.x, end.y)) {
                return stay(Blocked::Ring(ring.id.clone()));
            }
        }

        let out = |p: (f64, f64)| (p.0 - super::CENTER_X).stable_hypot(p.1 - super::CENTER_Y);
        if !super::in_world(end.x, end.y) && out((end.x, end.y)) > out(from) {
            return stay(Blocked::Edge);
        }
        end
    }

    /// The ring whose water `p` is in, away from its bridges.
    fn ring_water_at(&self, p: (f64, f64)) -> Option<&WaterRing> {
        let r = (p.0 - self.center.0).stable_hypot(p.1 - self.center.1);
        self.rings
            .iter()
            .find(|ring| r > ring.inner_radius() && r < ring.outer_radius() && !self.on_bridge(ring, p))
    }

    fn on_bridge(&self, ring: &WaterRing, p: (f64, f64)) -> bool {
        let angle = super::angle_from_center(self.center, p.0, p.1);
        ring.bridges.iter().any(|b| b.contains_angle(angle, ring.radius))
    }

    /// Whether a slide from `from` to `to` ends within [`super::BLOCK_BACKOFF`]
    /// of a ring's water away from its bridges, or of the world's edge, and
    /// nearer to it than it started.
    fn hugs(&self, from: (f64, f64), to: (f64, f64)) -> bool {
        let near = super::BLOCK_BACKOFF;
        let (rf, rt) = (self.radius(from), self.radius(to));
        let gap = |ring: &WaterRing, r: f64| (ring.inner_radius() - r).max(r - ring.outer_radius());
        let ring = self.rings.iter().any(|ring| {
            let g = gap(ring, rt);
            g < near && g < gap(ring, rf) && !self.on_bridge(ring, to)
        });
        let edge = |p: (f64, f64)| {
            super::WILDERNESS_OUTER_RADIUS - (p.0 - super::CENTER_X).stable_hypot(p.1 - super::CENTER_Y)
        };
        ring || (edge(to) < near && edge(to) < edge(from))
    }

    /// How far `p` is from the rings' centre.
    fn radius(&self, p: (f64, f64)) -> f64 {
        (p.0 - self.center.0).stable_hypot(p.1 - self.center.1)
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
    ///
    /// It lies in the corner of the square, off the world's disc, so a walk
    /// on it heads east or south, toward the centre: one heading further out
    /// is stopped by the edge.
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

    /// A crest steeper than the cliff for under a unit stops a line that
    /// climbs to it, either way across, though both ends stand on easy
    /// ground; a line that starts on a crag may climb down off it (#347).
    #[test]
    fn a_thin_crest_stops_a_line_and_a_crag_can_be_left() {
        // Slope 0.9 everywhere, but 1.5 over the 0.8 either side of x = 50.
        let crest = |x: f64, _: f64| if (x - 50.0).abs() < 0.4 { 1.5 } else { 0.9 };
        assert!(!slope_lets_through((10.0, 0.0), (90.0, 0.0), crest));
        assert!(!slope_lets_through((90.0, 0.0), (10.0, 0.0), crest));
        assert!(slope_lets_through((10.0, 0.0), (40.0, 0.0), crest));
        assert!(slope_lets_through((50.0, 0.0), (90.0, 0.0), crest));
        assert!(slope_lets_through((10.0, 0.0), (90.0, 0.0), |_, _| 0.3));
    }

    #[test]
    fn the_face_past_a_valley_floor_stops_a_line() {
        // As at the four walks of #520: 0.75 down to a kink at x = 51, then
        // past the cliff within half a unit, and back under 1.17 a unit on.
        let floor = |x: f64, _: f64| {
            if x < 51.0 {
                0.75
            } else if x < 51.5 {
                0.7 + (x - 51.0)
            } else {
                (1.21 - 0.04 * (x - 51.5)).max(0.75)
            }
        };
        // Strides of 2 from 10.5 land either side of the face, at 50.5 and 52.5
        assert!(!slope_lets_through((10.5, 0.0), (90.0, 0.0), floor));
        // Up to the floor is walkable, and so is off the face, down its slope
        assert!(slope_lets_through((10.5, 0.0), (51.0, 0.0), floor));
        assert!(slope_lets_through((51.5, 0.0), (90.0, 0.0), floor));
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
        assert_eq!(walk(&g, (1000.0, 700.0), (1050.0, 700.0)).blocked, None);
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
            for r in [7100.0, 7300.0, 7500.0, 7700.0] {
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

    // -----------------------------------------------------------------------
    // walk_step: the rings, the edge and the slide (T0.1, #315)
    // -----------------------------------------------------------------------

    use crate::rules::{angle_from_center, in_world, ring_crossing, CENTER_X, CENTER_Y, WILDERNESS_OUTER_RADIUS};
    use crate::terrain::Bridge;

    const CENTER: (f64, f64) = (CENTER_X, CENTER_Y);

    /// A canal 600 out and 30 wide, with one bridge 40 wide due south.
    fn canal() -> WaterRing {
        WaterRing {
            id: "canal_inner".into(),
            radius: 600.0,
            width: 30.0,
            bridges: vec![Bridge { angle_degrees: 90.0, width: 40.0 }],
        }
    }

    /// Flat land with only `canal` on it.
    fn ringed() -> Ground {
        Ground::new(&flat(), &[], &[], &[], &[], &[]).with_rings(CENTER, &[canal()])
    }

    /// A point `r` from the centre at `degrees` (0 east, 90 south).
    fn polar(r: f64, degrees: f64) -> (f64, f64) {
        let a = degrees.to_radians();
        (CENTER_X + r * a.stable_cos(), CENTER_Y + r * a.stable_sin())
    }

    /// Steps of 5 on a fixed heading, `ticks` of them: every step.
    fn steer(ground: &Ground, from: (f64, f64), heading: (f64, f64), ticks: usize) -> Vec<Step> {
        let mut at = from;
        (0..ticks)
            .map(|_| {
                let step = ground.walk_step(at, heading, 5.0);
                at = (step.x, step.y);
                step
            })
            .collect()
    }

    /// xorshift: the same walks every run, with no dependency.
    struct Dice(u64);

    impl Dice {
        fn roll(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    #[test]
    fn a_free_walk_step_goes_its_whole_way() {
        let g = ground();
        let step = g.walk_step((1500.0, 700.0), (3.0, 4.0), 5.0);
        assert_eq!(step.blocked, None);
        assert!((step.x - 1503.0).abs() < 1e-9 && (step.y - 704.0).abs() < 1e-9);
        // `step` is the same walk, toward a point on that heading.
        assert_eq!(g.step((1500.0, 700.0), (1530.0, 740.0), 5.0), step);
        // No heading, no step.
        assert_eq!(g.walk_step((1500.0, 700.0), (0.0, 0.0), 5.0).blocked, None);
    }

    #[test]
    fn a_slant_into_a_river_slides_along_its_bank() {
        let g = ground();
        // South-east from the west bank's side, into the river at x = 1000,
        // between the ford (y = 1000) and the road (y = 1500).
        let steps = steer(&g, (900.0, 1050.0), (1.0, 1.0), 100);
        for s in &steps {
            assert_eq!(g.footing(s.x, s.y), Footing::Dry, "in the river at ({}, {})", s.x, s.y);
            assert!(s.x < 990.0);
        }
        let end = steps.last().unwrap();
        // It met the bank about 90 down at x 990, and ran on down it at
        // better than half a step a tick.
        assert!(end.x > 980.0, "drifted off the bank to x {}", end.x);
        assert!(end.y > 1050.0 + 90.0 + 50.0 * 2.5, "only got to y {}", end.y);
        assert_eq!(end.blocked, None, "a slide isn't a stop");
    }

    #[test]
    fn square_into_a_river_stops_dead() {
        let g = ground();
        let steps = steer(&g, (950.0, 700.0), (1.0, 0.0), 40);
        let end = steps.last().unwrap();
        assert_eq!(end.blocked, Some(Blocked::Water("river_0".into())));
        assert_eq!(end.y, 700.0, "slid on a square push");
        assert!((989.0..=990.0).contains(&end.x), "stopped at {}", end.x);
    }

    #[test]
    fn a_slant_along_a_canal_slides_to_its_bridge_and_over() {
        let g = ringed();
        let ring = canal();
        // Outside the canal, south-east of its bridge, heading north-west:
        // a slant into the water that runs along it to the bridge.
        let from = polar(ring.outer_radius() + 10.0, 70.0);
        let steps = steer(&g, from, (-1.0, -0.6), 120);
        let mut at = from;
        let mut slid = 0;
        for s in &steps {
            let to = (s.x, s.y);
            assert_eq!(ring_crossing(CENTER, &ring, at, to), None, "crossed the water from {at:?} to {to:?}");
            assert!(g.ring_water_at(to).is_none(), "in the water at {to:?}");
            if s.blocked.is_none() && ((to.1 - at.1) - (to.0 - at.0) * 0.6).abs() > 1e-9 {
                slid += 1;
            }
            at = to;
        }
        let (dx, dy) = (at.0 - CENTER_X, at.1 - CENTER_Y);
        assert!(dx.stable_hypot(dy) < ring.inner_radius(), "never got over, at {:.1}° {:.1} out", angle_from_center(CENTER, at.0, at.1), dx.stable_hypot(dy));
        assert!(slid > 5, "it never slid");
    }

    #[test]
    fn square_into_a_canal_stops_dead_two_short() {
        let g = ringed();
        let ring = canal();
        let steps = steer(&g, polar(ring.outer_radius() + 30.0, 0.0), (-1.0, 0.0), 20);
        let end = steps.last().unwrap();
        assert_eq!(end.blocked, Some(Blocked::Ring("canal_inner".into())));
        assert_eq!(end.blocked.as_ref().unwrap().reason(), "Cannot cross the inner canal without a bridge");
        assert!((end.x - (CENTER_X + ring.outer_radius() + 2.0)).abs() < 1e-6, "stopped at {}", end.x);
        assert_eq!(end.y, CENTER_Y);
    }

    #[test]
    fn a_ring_stops_long_and_short_steps_at_the_same_point() {
        let g = ringed();
        let ring = canal();
        // Square to the canal, where nothing slides.
        let from = polar(ring.outer_radius() + 40.0, 20.0);
        let dest = polar(ring.inner_radius() - 40.0, 20.0);
        let stop = |walk: f64| {
            let mut at = from;
            loop {
                let step = g.step(at, dest, walk);
                if step.blocked.is_some() {
                    return (step.x, step.y);
                }
                at = (step.x, step.y);
            }
        };
        let (a, b) = (stop(5.0), stop(0.37));
        assert!((a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6, "{a:?} against {b:?}");
    }

    #[test]
    fn off_a_bridge_sideways_is_into_the_water() {
        let g = ringed();
        let ring = canal();
        // On the bridge, mid-canal, heading east along the water.
        let on = polar(ring.radius, 90.0);
        let mut at = on;
        for _ in 0..40 {
            let step = g.walk_step(at, (1.0, 0.0), 5.0);
            at = (step.x, step.y);
            assert!(g.ring_water_at(at).is_none(), "off the bridge into the canal at {at:?}");
        }
    }

    #[test]
    fn a_corner_stops_a_walk_dead() {
        // A river bending round a corner at (1000, 1000): down x = 1000,
        // then west along y = 1000.
        let river = course("river_0", 20.0, &[(1000.0, 200.0), (1000.0, 1000.0), (200.0, 1000.0)]);
        let g = Ground::new(&flat(), &[river], &[], &[], &[], &[]);
        // Into the corner at a slant: it meets x = 1000 first and slides
        // down it into the corner, where it stops.
        let steps = steer(&g, (700.0, 500.0), (1.0, 1.3), 300);
        let end = steps.last().unwrap();
        assert!(end.blocked.is_some(), "still going at ({}, {})", end.x, end.y);
        assert!(end.x > 980.0 && end.y > 980.0, "stopped short of the corner at ({}, {})", end.x, end.y);
        let before = &steps[steps.len() - 10];
        assert_eq!((before.x, before.y), (end.x, end.y), "it jitters in the corner");
        for s in &steps {
            assert_eq!(g.footing(s.x, s.y), Footing::Dry);
        }
    }

    #[test]
    fn walks_cross_a_ring_only_on_its_bridges() {
        let g = ringed();
        let ring = canal();
        let mut dice = Dice(0x0315_5eed);
        let mut over = 0;
        for _ in 0..400 {
            let from = polar(ring.radius + (dice.roll() - 0.5) * 120.0, 40.0 + dice.roll() * 100.0);
            if g.ring_water_at(from).is_some() {
                continue;
            }
            let inside = |p: (f64, f64)| (p.0 - CENTER_X).stable_hypot(p.1 - CENTER_Y) < ring.radius;
            let a = dice.roll() * std::f64::consts::TAU;
            let mut heading = (a.stable_cos(), a.stable_sin());
            let mut at = from;
            for tick in 0..120 {
                if tick % 20 == 0 {
                    let turn = (dice.roll() - 0.5) * 2.0;
                    heading = (heading.0 * turn.stable_cos() - heading.1 * turn.stable_sin(), heading.0 * turn.stable_sin() + heading.1 * turn.stable_cos());
                }
                let step = g.walk_step(at, heading, 1.0 + dice.roll() * 6.0);
                let to = (step.x, step.y);
                assert_eq!(ring_crossing(CENTER, &ring, at, to), None, "crossed off a bridge from {at:?} to {to:?}");
                assert!(g.ring_water_at(to).is_none(), "in the canal at {to:?}");
                if inside(at) != inside(to) {
                    over += 1;
                }
                at = to;
            }
        }
        assert!(over > 0, "nobody crossed: the test proves nothing");
    }

    #[test]
    fn walks_never_leave_the_world() {
        let g = Ground::new(&flat(), &[], &[], &[], &[], &[]);
        let mut dice = Dice(0xed6e);
        for _ in 0..200 {
            let mut at = polar(WILDERNESS_OUTER_RADIUS - dice.roll() * 30.0, dice.roll() * 360.0);
            let a = dice.roll() * std::f64::consts::TAU;
            let heading = (a.stable_cos(), a.stable_sin());
            for _ in 0..40 {
                let step = g.walk_step(at, heading, 5.0);
                at = (step.x, step.y);
                assert!(in_world(at.0, at.1), "off the world at {at:?}");
                if let Some(blocked) = step.blocked {
                    assert_eq!(blocked, Blocked::Edge);
                    assert_eq!(blocked.reason(), WORLD_EDGE);
                    assert_eq!(blocked.water_id(), "");
                }
            }
        }
        // Straight out, square to the edge: it stops where it stands.
        let edge = steer(&g, polar(WILDERNESS_OUTER_RADIUS - 3.0, 0.0), (1.0, 0.0), 3);
        assert_eq!(edge[0].blocked, Some(Blocked::Edge));
        assert_eq!((edge[0].x, edge[0].y), polar(WILDERNESS_OUTER_RADIUS - 3.0, 0.0));
    }

    #[test]
    fn walks_are_deterministic() {
        let g = ringed();
        let walk = || {
            let mut dice = Dice(42);
            let mut at = polar(640.0, 60.0);
            let mut trail = Vec::new();
            for _ in 0..500 {
                let a = dice.roll() * std::f64::consts::TAU;
                let step = g.walk_step(at, (a.stable_cos(), a.stable_sin()), 5.0);
                at = (step.x, step.y);
                trail.push((at.0.to_bits(), at.1.to_bits(), step.blocked));
            }
            trail
        };
        assert_eq!(walk(), walk());
    }
}
