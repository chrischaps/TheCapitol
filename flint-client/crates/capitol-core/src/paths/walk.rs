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

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use capitol_protocol::rules::PLAYER_SPEED;

use super::{distance, Point, WalkGraph};

/// The server is at a corner within this: the next leg goes even when the
/// straight line from where it stands can't be checked.
pub const AT_CORNER: f64 = 1.0;
/// A leg that takes this much longer than it should, plus [`LEG_GRACE`],
/// has stalled (the server refused the move, say): the walk stops.
const LEG_SLACK: f64 = 1.5;
const LEG_GRACE: Duration = Duration::from_secs(5);

/// The legs of a walk still to go.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    /// The leg sent, not yet walked: where it ends, and when it has stalled.
    leg: Option<(Point, Instant)>,
    /// The legs after it, in order. The last is the destination.
    ahead: VecDeque<Point>,
}

impl Walk {
    /// Plans a walk from `from` (where the server has you) to `to`, and
    /// returns the first leg to send. Any walk under way is dropped.
    pub fn plan(&mut self, graph: Option<&WalkGraph>, from: Point, to: Point, now: Instant) -> Point {
        let legs = match graph.map(|g| g.route(from, to)) {
            Some(Some(legs)) => legs,
            Some(None) => {
                tracing::info!("walk: no way round to ({:.0}, {:.0}); going straight", to.0, to.1);
                vec![to]
            }
            None => vec![to],
        };
        self.ahead = legs.into();
        let first = self.ahead.pop_front().unwrap_or(to);
        self.leg = Some((first, now + leg_time(graph, from, first)));
        first
    }

    /// Each frame while walking: the next leg to send, if it's time.
    /// `predicted` is false while the prediction is still walking its leg,
    /// `server` is where the server last had you.
    pub fn next(&mut self, graph: Option<&WalkGraph>, predicted: bool, server: Point, now: Instant) -> Option<Point> {
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

    /// The walk is over: you went elsewhere, the water stopped you, or you
    /// started to gather or craft.
    pub fn stop(&mut self) {
        self.leg = None;
        self.ahead.clear();
    }

    pub fn is_walking(&self) -> bool {
        self.leg.is_some()
    }

    /// Where the walk ends, while it's under way.
    pub fn destination(&self) -> Option<Point> {
        self.leg.map(|(corner, _)| self.ahead.back().copied().unwrap_or(corner))
    }

    /// The corners still to pass, the one being walked to first, ending at
    /// the destination. Empty when standing.
    pub fn corners(&self) -> impl Iterator<Item = Point> + '_ {
        self.leg.map(|(corner, _)| corner).into_iter().chain(self.ahead.iter().copied())
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
        let nowhere = (8300.0, 8300.0);
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
}
