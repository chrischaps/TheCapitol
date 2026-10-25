//! A walk of several legs, sent a leg at a time as it goes (#267): the
//! Flint client's click-to-walk, where [`Bot::walk_to`](crate::bot::Bot::walk_to)
//! waits on each leg in a loop.
//!
//! [`Walk::plan`] routes the walk on the [`WalkGraph`] and gives the first
//! leg to send. Each frame after, [`Walk::next`] says when to send the
//! next: as the prediction reaches the corner, if the server, a little
//! behind it, can walk straight from where it is to the next corner too.
//! Otherwise it waits until the server is at the corner, so a cut corner
//! never puts it in a canal by a bridge. With no graph, or no way round,
//! the walk is one straight leg, and the bank answers it with
//! `movement_blocked` as before.
//!
//! [`Walk::request`] is the Flint client's way in (#348): a walk that can't
//! go straight is planned on a thread of its own ([`Planning`]), since a
//! click into the ranges can take most of a second to plan, and
//! [`Walk::next`] sends its first leg when the plan comes back.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use capitol_protocol::rules::PLAYER_SPEED;

use super::{distance, NoWay, Point, WalkGraph};

/// The server is at a corner within this: the next leg goes even when the
/// straight line from where it stands can't be checked.
pub const AT_CORNER: f64 = 1.0;
/// A leg that takes this much longer than it should, plus [`LEG_GRACE`],
/// has stalled (the server refused the move, say): the walk stops.
const LEG_SLACK: f64 = 1.5;
const LEG_GRACE: Duration = Duration::from_secs(5);

/// The legs of a walk still to go.
#[derive(Debug, Default)]
pub struct Walk {
    /// The leg sent, not yet walked: where it ends, and when it has stalled.
    leg: Option<(Point, Instant)>,
    /// The legs after it, in order. The last is the destination.
    ahead: VecDeque<Point>,
    /// The walk being planned, before it has a leg ([`Self::request`]).
    planning: Option<Planning>,
}

impl Walk {
    /// Plans a walk from `from` (where the server has you) to `to`, and
    /// returns the first leg to send. Any walk under way is dropped.
    pub fn plan(&mut self, graph: Option<&WalkGraph>, from: Point, to: Point, now: Instant) -> Point {
        let planned = graph.map_or_else(|| Ok(vec![to]), |g| g.plan(from, to));
        self.stop();
        self.follow(graph, from, from, to, planned, now)
    }

    /// Starts a walk from `from` (where the server has you) to `to`,
    /// dropping any under way. A walk that can go straight, or that has no
    /// graph to go round by, goes at once: the first leg to send. Any other
    /// is planned on a thread of its own, and [`Self::next`] gives its first
    /// leg when the plan comes back: in a frame or two in the city, and in
    /// most of a second for a walk into the ranges, which held the frame
    /// while it was planned here (#348).
    pub fn request(&mut self, graph: Option<Arc<WalkGraph>>, from: Point, to: Point, now: Instant) -> Option<Point> {
        self.stop();
        match graph {
            Some(graph) if !graph.walkable(from, to) => {
                self.planning = Some(Planning::start(graph, from, to));
                None
            }
            graph => Some(self.follow(graph.as_deref(), from, from, to, Ok(vec![to]), now)),
        }
    }

    /// Each frame while walking: the next leg to send, if it's time.
    /// `predicted` is false while the prediction is still walking its leg,
    /// `server` is where the server last had you. A walk being planned
    /// sends its first leg once the plan is back.
    pub fn next(&mut self, graph: Option<&WalkGraph>, predicted: bool, server: Point, now: Instant) -> Option<Point> {
        if let Some(planning) = &self.planning {
            let planned = planning.answer()?;
            let (from, to) = (planning.from, planning.to);
            self.planning = None;
            return Some(self.follow(graph, from, server, to, planned, now));
        }
        let (corner, stalls) = self.leg?;
        let Some(&next) = self.ahead.front() else {
            // The last leg: done once walked, as a straight walk is
            if predicted || distance(server, corner) < AT_CORNER {
                self.stop();
            }
            return None;
        };
        let at_corner = distance(server, corner) < AT_CORNER;
        if at_corner || (predicted && graph.is_some_and(|g| g.walkable(server, next))) {
            self.ahead.pop_front();
            self.leg = Some((next, now + leg_time(graph, server, next)));
            return Some(next);
        }
        if now > stalls {
            tracing::info!("walk: stalled on the way to ({:.0}, {:.0}); stopping", corner.0, corner.1);
            self.stop();
        }
        None
    }

    /// Follows the walk planned from `from`, setting out from `server`,
    /// where the server has you now, and returns the first leg to send. If
    /// you walked on while it was planned (on the leg you were on when you
    /// clicked) and the first leg isn't walkable from where you've got to,
    /// it goes back to `from` first. With no way round it goes straight.
    fn follow(
        &mut self,
        graph: Option<&WalkGraph>,
        from: Point,
        server: Point,
        to: Point,
        planned: Result<Vec<Point>, NoWay>,
        now: Instant,
    ) -> Point {
        let mut legs = match planned {
            Ok(legs) => legs,
            Err(_) => {
                tracing::info!("walk: no way round to ({:.0}, {:.0}); going straight", to.0, to.1);
                vec![to]
            }
        };
        if let (Some(g), Some(&first)) = (graph, legs.first()) {
            if distance(server, from) >= AT_CORNER && !g.walkable(server, first) {
                legs.insert(0, from);
            }
        }
        self.ahead = legs.into();
        let first = self.ahead.pop_front().unwrap_or(to);
        self.leg = Some((first, now + leg_time(graph, server, first)));
        first
    }

    /// The walk is over: you went elsewhere, the water stopped you, or you
    /// started to gather or craft. A plan still being made is given up.
    pub fn stop(&mut self) {
        self.leg = None;
        self.ahead.clear();
        self.planning = None;
    }

    /// Walking, or about to: the walk is being planned.
    pub fn is_walking(&self) -> bool {
        self.leg.is_some() || self.planning.is_some()
    }

    /// The walk is being planned, and has no leg yet.
    pub fn is_planning(&self) -> bool {
        self.planning.is_some()
    }

    /// Where the walk ends, while it's under way or being planned.
    pub fn destination(&self) -> Option<Point> {
        match &self.planning {
            Some(planning) => Some(planning.to),
            None => self.leg.map(|(corner, _)| self.ahead.back().copied().unwrap_or(corner)),
        }
    }

    /// The corners still to pass, the one being walked to first, ending at
    /// the destination. Empty when standing, and while the walk is planned.
    pub fn corners(&self) -> impl Iterator<Item = Point> + '_ {
        self.leg.map(|(corner, _)| corner).into_iter().chain(self.ahead.iter().copied())
    }
}

/// A walk planned on a thread of its own (#348), from where the server had
/// you to where you clicked. Dropping it gives the plan up: a newer click,
/// a steer or a gather doesn't wait on the old one, and its thread stops
/// within a few dozen of the grid's cells ([`WalkGraph::plan_unless`]).
#[derive(Debug)]
pub struct Planning {
    from: Point,
    to: Point,
    answer: Receiver<Result<Vec<Point>, NoWay>>,
    gone: Arc<AtomicBool>,
}

impl Planning {
    /// Starts planning the walk from `from` to `to` on `graph`.
    pub fn start(graph: Arc<WalkGraph>, from: Point, to: Point) -> Self {
        let (tell, answer) = mpsc::channel();
        let gone = Arc::new(AtomicBool::new(false));
        let asked = gone.clone();
        let spawned = thread::Builder::new().name("walk-plan".into()).spawn(move || {
            if let Some(planned) = graph.plan_unless(from, to, &|| asked.load(Ordering::Relaxed)) {
                let _ = tell.send(planned);
            }
        });
        // With no thread the answer is "no way", and the walk goes straight
        if let Err(e) = spawned {
            tracing::warn!("walk: couldn't start planning: {e}");
        }
        Planning { from, to, answer, gone }
    }

    /// Where the walk is to.
    pub fn to(&self) -> Point {
        self.to
    }

    /// The plan, once it's back. A thread that ended without one is no way.
    pub fn answer(&self) -> Option<Result<Vec<Point>, NoWay>> {
        match self.answer.try_recv() {
            Ok(planned) => Some(planned),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(NoWay::Unreachable)),
        }
    }
}

impl Drop for Planning {
    fn drop(&mut self) {
        self.gone.store(true, Ordering::Relaxed);
    }
}

/// How long the leg from `a` to `b` may take before it has stalled.
fn leg_time(graph: Option<&WalkGraph>, a: Point, b: Point) -> Duration {
    let secs = graph.map_or(distance(a, b) / PLAYER_SPEED, |g| g.walk_secs(a, b));
    Duration::from_secs_f64(secs * LEG_SLACK) + LEG_GRACE
}

#[cfg(test)]
mod tests {
    use super::super::tests::one_canal;
    use super::*;

    /// Across the canal of `one_canal`, which only its bridge at 90° crosses.
    const FROM: Point = (6900.0, 7900.0);
    const TO: Point = (6900.0, 7700.0);

    #[test]
    fn a_walk_across_the_canal_is_sent_a_leg_at_a_time_by_the_bridge() {
        let graph = WalkGraph::new(&one_canal());
        let now = Instant::now();
        let mut walk = Walk::default();
        let first = walk.plan(Some(&graph), FROM, TO, now);
        let corners: Vec<Point> = walk.corners().collect();
        assert!(corners.len() >= 2, "{corners:?}");
        assert_eq!(corners[0], first);
        assert_eq!(walk.destination(), Some(TO));

        // Still walking the leg: nothing new
        assert_eq!(walk.next(Some(&graph), false, FROM, now), None);
        // At each corner the next leg goes, until the last
        for pair in corners.windows(2) {
            assert_eq!(walk.next(Some(&graph), true, pair[0], now), Some(pair[1]));
        }
        assert!(walk.is_walking(), "the last leg is still walked");
        assert_eq!(walk.next(Some(&graph), false, corners[corners.len() - 2], now), None);
        assert!(walk.is_walking());
        assert_eq!(walk.next(Some(&graph), true, TO, now), None);
        assert!(!walk.is_walking());
        assert_eq!(walk.destination(), None);
        assert_eq!(walk.corners().count(), 0);
    }

    #[test]
    fn a_corner_by_the_water_waits_for_the_server() {
        let graph = WalkGraph::new(&one_canal());
        let now = Instant::now();
        let mut walk = Walk::default();
        walk.plan(Some(&graph), FROM, TO, now);
        let corners: Vec<Point> = walk.corners().collect();
        // The prediction is at the first corner, on the road south of the
        // bridge, and the server still out east of it, where the canal
        // stands between it and the next corner
        let behind = (6900.0, 7800.0 + 30.0);
        assert!(!graph.walkable(behind, corners[1]), "{corners:?}");
        assert_eq!(walk.next(Some(&graph), true, behind, now), None, "waits for the server");
        assert_eq!(walk.next(Some(&graph), true, corners[0], now), Some(corners[1]));
    }

    #[test]
    fn with_no_graph_or_no_way_round_the_walk_is_straight() {
        let now = Instant::now();
        let mut walk = Walk::default();
        assert_eq!(walk.plan(None, FROM, TO, now), TO);
        assert_eq!(walk.corners().collect::<Vec<_>>(), vec![TO]);
        // The server walks it alone; the prediction's stop ends it
        assert_eq!(walk.next(None, false, (6900.0, 7800.0), now), None);
        assert!(walk.is_walking());
        assert_eq!(walk.next(None, true, (6900.0, 7800.0), now), None);
        assert!(!walk.is_walking());

        let graph = WalkGraph::new(&one_canal());
        let nowhere = (8800.0, 8800.0);
        assert_eq!(walk.plan(Some(&graph), nowhere, (6800.0, 6900.0), now), (6800.0, 6900.0));
        assert_eq!(walk.corners().count(), 1);
    }

    #[test]
    fn a_stalled_leg_stops_the_walk_and_a_new_plan_replaces_it() {
        let graph = WalkGraph::new(&one_canal());
        let now = Instant::now();
        let mut walk = Walk::default();
        walk.plan(Some(&graph), FROM, TO, now);
        let later = now + Duration::from_secs(600);
        // Not there, and not able to go on: long past due, it gives up
        assert_eq!(walk.next(Some(&graph), true, (7200.0, 8200.0), later), None);
        assert!(!walk.is_walking());

        walk.plan(Some(&graph), FROM, TO, now);
        let first = walk.plan(Some(&graph), FROM, (6950.0, 7950.0), now);
        assert_eq!(first, (6950.0, 7950.0), "a straight walk on the same side");
        assert_eq!(walk.corners().count(), 1);
        walk.stop();
        assert_eq!(walk.destination(), None);
    }

    #[test]
    fn a_walk_that_cant_go_straight_is_planned_on_a_thread_of_its_own() {
        let graph = Arc::new(WalkGraph::new(&one_canal()));
        let now = Instant::now();
        let mut walk = Walk::default();
        // On the same side it goes at once
        assert_eq!(walk.request(Some(graph.clone()), FROM, (6950.0, 7950.0), now), Some((6950.0, 7950.0)));
        assert!(!walk.is_planning());
        assert_eq!(walk.request(None, FROM, TO, now), Some(TO), "with no graph, straight");

        // Over the canal it's planned, and has no leg until the plan is back
        assert_eq!(walk.request(Some(graph.clone()), FROM, TO, now), None);
        assert!(walk.is_walking() && walk.is_planning());
        assert_eq!(walk.destination(), Some(TO));
        assert_eq!(walk.corners().count(), 0);
        let deadline = Instant::now() + Duration::from_secs(10);
        let first = loop {
            if let Some(first) = walk.next(Some(&graph), false, FROM, now) {
                break first;
            }
            assert!(Instant::now() < deadline, "the plan never came back");
            thread::sleep(Duration::from_millis(1));
        };
        let planned = graph.route(FROM, TO).unwrap();
        assert_eq!(first, planned[0]);
        assert_eq!(walk.corners().collect::<Vec<_>>(), planned);
        assert!(walk.is_walking() && !walk.is_planning());

        // Stopping gives a plan up
        walk.request(Some(graph.clone()), FROM, TO, now);
        walk.stop();
        assert!(!walk.is_walking());
        assert_eq!(walk.destination(), None);
    }

    #[test]
    fn walked_on_while_it_was_planned_it_goes_back_by_where_it_was_planned_from() {
        let graph = WalkGraph::new(&one_canal());
        let now = Instant::now();
        let mut walk = Walk::default();
        let planned = graph.route(FROM, TO).unwrap();
        // Walking east when you clicked: from there the canal is in the way
        // of the plan's first corner, by the bridge
        let east = (7300.0, 7800.0);
        assert!(!graph.walkable(east, planned[0]), "{planned:?}");
        let first = walk.follow(Some(&graph), FROM, east, TO, Ok(planned.clone()), now);
        assert_eq!(first, FROM);
        assert_eq!(walk.corners().skip(1).collect::<Vec<_>>(), planned);

        // A step on, with the first corner still in sight, it goes straight there
        let near = (6905.0, 7900.0);
        assert_eq!(walk.follow(Some(&graph), FROM, near, TO, Ok(planned.clone()), now), planned[0]);
        // No way round goes straight
        assert_eq!(walk.follow(Some(&graph), FROM, FROM, TO, Err(NoWay::Unreachable), now), TO);
    }
}
