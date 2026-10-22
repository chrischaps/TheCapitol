//! What an admin's client watches of the server (D5.3, devtools plan §6):
//! the newest of the Observatory's samples, the annotations that explain a
//! bend in its charts (a tunable changed, a watch fired, the town up), and
//! the live tunables, read-only. And, for the world's citizens layer, the
//! players round you as the server is moving them ([`Walker`]s).
//!
//! [`Watch`] does no IO and never reads the clock: the game asks it what's
//! [`due`](Watch::due) and sends that, then hands it each answer. It asks
//! only while the overlay is open, and only of a server whose `auth_ok` said
//! the token is an admin's; the admin routes check the token again anyway,
//! and a 403 stops it asking.
//!
//! It polls rather than holding `/admin/stream` open: the samples come every
//! [`SAMPLE_SECS`] whichever way, and a poll is one more REST call on the IO
//! thread where a stream would be a second long-lived connection.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use capitol_protocol::annotation::{Annotation, AnnotationKind, AnnotationsQuery};
use capitol_protocol::metrics::{Sample, SeriesQuery, SAMPLE_SECS};
use capitol_protocol::rest::{Walker, WalkersQuery};
use capitol_protocol::tunables::{TunableValue, TunablesResponse};

use crate::net::{RestError, RestRequest, RestResponse};

/// How often the samples and annotations are asked for: a sample's interval.
pub const SERIES_EVERY: Duration = Duration::from_secs(SAMPLE_SECS);
/// How often the tunables are, besides whenever an annotation says one changed.
pub const TUNABLES_EVERY: Duration = Duration::from_secs(60);
/// How far back the first ask looks for samples: two of them, so one is
/// there even when the newest is late.
pub const FIRST_SAMPLES_MS: i64 = 2 * SAMPLE_SECS as i64 * 1000;
/// How far back the first ask looks for annotations.
pub const FIRST_ANNOTATIONS_MS: i64 = 3_600_000;
/// The annotations asked for at a time, oldest first: a busy hour catches up
/// over a few asks.
pub const ANNOTATIONS_LIMIT: i64 = 50;
/// The newest annotations kept.
pub const KEPT: usize = 8;
/// How often the walkers are asked for while a layer wants them: a walk's
/// leg changes every few seconds, so once a second keeps up.
pub const WALKERS_EVERY: Duration = Duration::from_secs(1);
/// How far round you they're asked for, in server units: the interest
/// grid's 3×3 cells reach 450 to 636 from you.
pub const WALKERS_RADIUS: f64 = 640.0;

/// The server as an admin's client sees it.
#[derive(Debug, Default)]
pub struct Watch {
    sample: Option<Sample>,
    annotations: VecDeque<Annotation>,
    tunables: Option<TunablesResponse>,
    asked_series: Option<Instant>,
    asked_tunables: Option<Instant>,
    /// An annotation said a tunable changed since they were read.
    tunables_stale: bool,
    /// The server said no (a 403): the token isn't an admin's after all.
    refused: bool,
    /// Where a layer wants the walkers round, in server units.
    walkers_round: Option<(f64, f64)>,
    asked_walkers: Option<Instant>,
    walkers: Vec<Walker>,
}

impl Watch {
    /// Ask for the walkers round `at` (server units) from now on, or stop
    /// with `None`.
    pub fn watch_walkers(&mut self, at: Option<(f64, f64)>) {
        if at.is_none() {
            self.walkers.clear();
        }
        self.walkers_round = at;
    }

    /// What to ask for now, given the server's clock (Unix ms): the server's
    /// state while the overlay is `open`, and the walkers while a layer
    /// wants them. Nothing once the server has refused.
    pub fn due(&mut self, now: Instant, server_ms: i64, open: bool) -> Vec<RestRequest> {
        let mut out = Vec::new();
        if self.refused {
            return out;
        }
        if let Some((x, y)) = self.walkers_round {
            if self.asked_walkers.is_none_or(|at| now.duration_since(at) >= WALKERS_EVERY) {
                self.asked_walkers = Some(now);
                out.push(RestRequest::Walkers(WalkersQuery { x, y, radius: WALKERS_RADIUS }));
            }
        }
        if !open {
            return out;
        }
        if self.asked_series.is_none_or(|at| now.duration_since(at) >= SERIES_EVERY) {
            self.asked_series = Some(now);
            let from_ms = self.sample.as_ref().map_or(server_ms - FIRST_SAMPLES_MS, |s| s.at_ms);
            out.push(RestRequest::Series(SeriesQuery { from_ms: Some(from_ms), ..Default::default() }));
            // `since_ms` is inclusive, so the newest comes again: `observe`
            // drops it by its id
            let since_ms = self.annotations.back().map_or(server_ms - FIRST_ANNOTATIONS_MS, |a| a.at_ms);
            out.push(RestRequest::Annotations(AnnotationsQuery {
                since_ms: Some(since_ms),
                limit: Some(ANNOTATIONS_LIMIT),
            }));
        }
        if self.tunables_stale || self.asked_tunables.is_none_or(|at| now.duration_since(at) >= TUNABLES_EVERY) {
            self.asked_tunables = Some(now);
            self.tunables_stale = false;
            out.push(RestRequest::Tunables);
        }
        out
    }

    /// An answer to one of [`due`](Self::due)'s asks; any other is ignored.
    pub fn observe(&mut self, response: &RestResponse) {
        match response {
            RestResponse::Series(series) => {
                if let Some(newest) = series.samples.iter().max_by_key(|s| s.at_ms) {
                    if self.sample.as_ref().is_none_or(|s| newest.at_ms > s.at_ms) {
                        self.sample = Some(newest.clone());
                    }
                }
            }
            RestResponse::Annotations(annotations) => {
                for a in annotations {
                    if self.annotations.iter().any(|kept| kept.id == a.id) {
                        continue;
                    }
                    if a.kind == AnnotationKind::TunableChanged {
                        self.tunables_stale = true;
                    }
                    self.annotations.push_back(a.clone());
                }
                self.annotations.make_contiguous().sort_by_key(|a| (a.at_ms, a.id));
                while self.annotations.len() > KEPT {
                    self.annotations.pop_front();
                }
            }
            RestResponse::Tunables(tunables) => self.tunables = Some(tunables.clone()),
            // One that comes after the layer let go is dropped
            RestResponse::Walkers(walkers) if self.walkers_round.is_some() => self.walkers = walkers.clone(),
            _ => {}
        }
    }

    /// One of its asks failed. A 403 means the token isn't an admin's, so it
    /// stops asking; anything else is asked again in turn.
    pub fn failed(&mut self, error: &RestError) {
        if error.status() == Some(403) {
            self.refused = true;
        }
    }

    /// The newest sample, once one has come.
    pub fn sample(&self) -> Option<&Sample> {
        self.sample.as_ref()
    }

    /// The newest annotations, oldest first.
    pub fn annotations(&self) -> impl DoubleEndedIterator<Item = &Annotation> {
        self.annotations.iter()
    }

    /// Every knob, once read.
    pub fn tunables(&self) -> Option<&[TunableValue]> {
        self.tunables.as_ref().map(|t| t.knobs.as_slice())
    }

    /// The knobs set off their defaults.
    pub fn tuned(&self) -> Vec<&TunableValue> {
        self.tunables()
            .unwrap_or_default()
            .iter()
            .filter(|k| (k.value - k.default).abs() > k.step.abs() * 1e-6)
            .collect()
    }

    /// The players round the point asked for, as of the last answer.
    pub fn walkers(&self) -> &[Walker] {
        &self.walkers
    }

    pub fn refused(&self) -> bool {
        self.refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::metrics::Series;
    use capitol_protocol::tunables::Unit;

    const NOW_MS: i64 = 1_792_600_000_000;

    fn sample(at_ms: i64) -> Sample {
        let mut s: Sample = serde_json::from_str(include_str!("../../../../protocol/tests/fixtures/rest/sample.json"))
            .expect("the sample fixture decodes");
        s.at_ms = at_ms;
        s
    }

    fn annotation(id: i64, at_ms: i64, kind: AnnotationKind) -> Annotation {
        Annotation { id, at_ms, kind, text: format!("#{id}") }
    }

    fn knob(key: &str, default: f64, value: f64) -> TunableValue {
        TunableValue {
            key: key.into(),
            label: key.into(),
            unit: Unit::Percent,
            min: 0.0,
            max: 10.0,
            step: 0.01,
            default,
            value,
            chart: "money".into(),
            base_strands: None,
        }
    }

    #[test]
    fn it_asks_on_open_then_every_sample() {
        let t0 = Instant::now();
        let mut w = Watch::default();
        let first = w.due(t0, NOW_MS, true);
        assert_eq!(
            first,
            vec![
                RestRequest::Series(SeriesQuery { from_ms: Some(NOW_MS - FIRST_SAMPLES_MS), ..Default::default() }),
                RestRequest::Annotations(AnnotationsQuery {
                    since_ms: Some(NOW_MS - FIRST_ANNOTATIONS_MS),
                    limit: Some(ANNOTATIONS_LIMIT)
                }),
                RestRequest::Tunables,
            ]
        );
        assert!(w.due(t0 + Duration::from_secs(5), NOW_MS, true).is_empty());
        w.observe(&RestResponse::Series(Box::new(Series { samples: vec![sample(NOW_MS - 3000)], annotations: vec![] })));
        w.observe(&RestResponse::Annotations(vec![annotation(4, NOW_MS - 60_000, AnnotationKind::Boot)]));
        // From the newest it has, so nothing comes twice but the newest
        // annotation, which its id drops
        let next = w.due(t0 + SERIES_EVERY, NOW_MS + 10_000, true);
        assert_eq!(
            next,
            vec![
                RestRequest::Series(SeriesQuery { from_ms: Some(NOW_MS - 3000), ..Default::default() }),
                RestRequest::Annotations(AnnotationsQuery { since_ms: Some(NOW_MS - 60_000), limit: Some(ANNOTATIONS_LIMIT) }),
            ]
        );
        assert_eq!(w.due(t0 + TUNABLES_EVERY, NOW_MS, true), vec![
            RestRequest::Series(SeriesQuery { from_ms: Some(NOW_MS - 3000), ..Default::default() }),
            RestRequest::Annotations(AnnotationsQuery { since_ms: Some(NOW_MS - 60_000), limit: Some(ANNOTATIONS_LIMIT) }),
            RestRequest::Tunables,
        ]);
    }

    #[test]
    fn it_keeps_the_newest_sample_and_annotations() {
        let mut w = Watch::default();
        let series = |at: &[i64]| RestResponse::Series(Box::new(Series {
            samples: at.iter().map(|&a| sample(a)).collect(),
            annotations: vec![],
        }));
        w.observe(&series(&[100, 300, 200]));
        assert_eq!(w.sample().unwrap().at_ms, 300);
        // An empty or older answer leaves it
        w.observe(&series(&[]));
        w.observe(&series(&[250]));
        assert_eq!(w.sample().unwrap().at_ms, 300);

        let batch: Vec<_> = (1..=10).map(|i| annotation(i, i * 1000, AnnotationKind::GazettePublished)).collect();
        w.observe(&RestResponse::Annotations(batch.clone()));
        // Again, as `since_ms` overlaps: nothing doubles
        w.observe(&RestResponse::Annotations(batch[8..].to_vec()));
        let ids: Vec<i64> = w.annotations().map(|a| a.id).collect();
        assert_eq!(ids, (3..=10).collect::<Vec<_>>());
    }

    #[test]
    fn a_tunable_changed_reads_them_again() {
        let t0 = Instant::now();
        let mut w = Watch::default();
        w.due(t0, NOW_MS, true);
        w.observe(&RestResponse::Tunables(TunablesResponse {
            knobs: vec![knob("exchange_fee", 0.5, 0.5), knob("property_tax_rate", 1.0, 2.5)],
        }));
        assert_eq!(w.tunables().unwrap().len(), 2);
        assert_eq!(w.tuned().iter().map(|k| k.key.as_str()).collect::<Vec<_>>(), ["property_tax_rate"]);
        let soon = t0 + SERIES_EVERY;
        w.observe(&RestResponse::Annotations(vec![annotation(9, NOW_MS, AnnotationKind::TunableChanged)]));
        assert!(w.due(soon, NOW_MS, true).contains(&RestRequest::Tunables));
        assert!(!w.due(soon + SERIES_EVERY, NOW_MS, true).contains(&RestRequest::Tunables));
    }

    #[test]
    fn the_walkers_are_asked_for_while_a_layer_wants_them() {
        let t0 = Instant::now();
        let mut w = Watch::default();
        // The overlay closed and no layer: nothing
        assert!(w.due(t0, NOW_MS, false).is_empty());
        w.watch_walkers(Some((6800.0, 6750.0)));
        let ask = RestRequest::Walkers(WalkersQuery { x: 6800.0, y: 6750.0, radius: WALKERS_RADIUS });
        assert_eq!(w.due(t0, NOW_MS, false), vec![ask.clone()]);
        assert!(w.due(t0 + Duration::from_millis(500), NOW_MS, false).is_empty());
        assert_eq!(w.due(t0 + WALKERS_EVERY, NOW_MS, false), vec![ask]);
        let ada = Walker {
            player_id: uuid::Uuid::from_u128(1),
            name: "Ada".into(),
            citizen: true,
            x: 6810.0,
            y: 6740.0,
            dest: Some([6900.0, 6700.0]),
            doing: capitol_protocol::rest::Doing::Idle,
        };
        w.observe(&RestResponse::Walkers(vec![ada.clone()]));
        assert_eq!(w.walkers(), std::slice::from_ref(&ada));
        // Let go, they're dropped, and a late answer doesn't bring them back
        w.watch_walkers(None);
        assert!(w.walkers().is_empty());
        w.observe(&RestResponse::Walkers(vec![ada]));
        assert!(w.walkers().is_empty());
        assert!(w.due(t0 + WALKERS_EVERY * 3, NOW_MS, false).is_empty());
    }

    #[test]
    fn a_403_stops_it_asking() {
        let mut w = Watch::default();
        let t0 = Instant::now();
        w.due(t0, NOW_MS, true);
        w.failed(&RestError::Transport("reset".into()));
        assert!(!w.refused());
        w.failed(&RestError::Http { status: 403, message: "Admin access required".into() });
        assert!(w.refused());
        assert!(w.due(t0 + TUNABLES_EVERY * 2, NOW_MS, true).is_empty());
    }
}
