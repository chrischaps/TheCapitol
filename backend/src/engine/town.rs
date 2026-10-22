//! The town reports in (the Observatory's D4.1, #368).
//!
//! `capitol-town` posts a [`TownReport`] each game minute to
//! `POST /admin/town/report`, with the key an admin mints for it at
//! `POST /admin/town/key` (`api_keys::mint_town_key`). [`Town`] keeps:
//! - the newest report, for `GET /admin/town`, with each citizen's last
//!   decisions merged in, since a report carries a citizen's decisions only
//!   when it has new ones;
//! - two windows of what the reports said: the sample's, which
//!   [`Town::take_sample`] hands each sample as its `town`, and the stored
//!   minute's ([`Town::take_minute`]), which goes back if the minute can't be
//!   stored, so the table's samples add up as the ring's do.
//!
//! It marks the town on the charts (`AnnotationKind::TownUp`, `TownDown`):
//! up when a report comes in from a new run, or the first since the server
//! started or the town went down; down when the town says it's stopping, or
//! when it goes quiet for [`TOWN_QUIET_MS`], which the sampler looks for each
//! sample ([`Town::quiet`]).
//!
//! Since D2.2 the newest report also names each citizen's trade
//! ([`Town::roles`]), for the wealth by role in each sample's
//! `Distribution`.

use std::collections::HashMap;
use std::sync::Mutex;

use capitol_protocol::annotation::AnnotationKind;
use capitol_protocol::metrics::{DecisionBrief, TownLatest, TownReport, TownSummary, TOWN_QUIET_MS};

/// A marker the town's comings and goings make, for `annotations::note`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub kind: AnnotationKind,
    pub text: String,
}

impl Mark {
    fn up(text: String) -> Mark {
        Mark { kind: AnnotationKind::TownUp, text }
    }

    fn down(text: &str) -> Mark {
        Mark { kind: AnnotationKind::TownDown, text: text.to_string() }
    }
}

/// The town, as its reports tell it.
#[derive(Default)]
pub struct Town {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// The newest report, and when it came by the server's clock
    latest: Option<(TownReport, i64)>,
    /// Each citizen's last decisions, by name
    decisions: HashMap<String, Vec<DecisionBrief>>,
    /// Whether the town is marked up
    up: bool,
    /// The sample's window
    sample: Option<TownSummary>,
    /// The stored minute's window
    minute: Option<TownSummary>,
}

impl Town {
    pub fn new() -> Town {
        Town::default()
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keeps `report`, which came in at `now_ms`. Returns the marker it
    /// makes, if the town came on or is stopping.
    pub fn receive(&self, mut report: TownReport, now_ms: i64) -> Option<Mark> {
        let mut inner = self.inner();
        let new_run = inner.latest.as_ref().is_some_and(|(last, _)| last.started_ms != report.started_ms);
        let roster = match report.citizens.len() {
            1 => "1 citizen".to_string(),
            n => format!("{n} citizens"),
        };
        let mark = if report.stopping {
            let was_up = std::mem::replace(&mut inner.up, false);
            was_up.then(|| Mark::down("The town stopped"))
        } else if new_run {
            inner.up = true;
            Some(Mark::up(format!("The town started again, with {roster} on its roster")))
        } else if !inner.up {
            inner.up = true;
            Some(Mark::up(format!("The town came on, with {roster} on its roster")))
        } else {
            None
        };
        inner.sample.get_or_insert_with(TownSummary::default).add(&report, now_ms);
        inner.minute.get_or_insert_with(TownSummary::default).add(&report, now_ms);
        // The citizens on the roster keep their last decisions; those gone
        // from it are forgotten
        let mut decisions = std::mem::take(&mut inner.decisions);
        for citizen in &mut report.citizens {
            let fresh = std::mem::take(&mut citizen.decisions);
            let kept = decisions.remove(&citizen.name).unwrap_or_default();
            inner.decisions.insert(citizen.name.clone(), if fresh.is_empty() { kept } else { fresh });
        }
        inner.latest = Some((report, now_ms));
        mark
    }

    /// Whether the town, marked up, has gone quiet: no report since
    /// [`TOWN_QUIET_MS`] before `now_ms`. Marks it down if so.
    pub fn quiet(&self, now_ms: i64) -> Option<Mark> {
        let mut inner = self.inner();
        let heard = inner.latest.as_ref().map(|(_, at)| *at)?;
        if inner.up && now_ms - heard >= TOWN_QUIET_MS {
            inner.up = false;
            return Some(Mark::down("The town went quiet: no report in a minute"));
        }
        None
    }

    /// The newest report, with each citizen's last decisions, and whether
    /// the town is up; `None` if no town has reported since the server
    /// started.
    pub fn latest(&self) -> Option<TownLatest> {
        let inner = self.inner();
        let (report, received_ms) = inner.latest.as_ref()?;
        let mut report = report.clone();
        for citizen in &mut report.citizens {
            citizen.decisions = inner.decisions.get(&citizen.name).cloned().unwrap_or_default();
        }
        Some(TownLatest { received_ms: *received_ms, up: inner.up, report })
    }

    /// Each citizen's trade, by name, from the newest report: empty until
    /// a town reports. It holds after the town goes down, since the trades
    /// don't change while it's away.
    pub fn roles(&self) -> HashMap<String, String> {
        let inner = self.inner();
        let Some((report, _)) = inner.latest.as_ref() else { return HashMap::new() };
        report.citizens.iter().map(|c| (c.name.clone(), c.role.clone())).collect()
    }

    /// The reports since the last sample, for this one; `None` if none came
    pub fn take_sample(&self) -> Option<TownSummary> {
        self.inner().sample.take()
    }

    /// The reports since the last stored sample, for this one. If it can't
    /// be stored, [`restore_minute`](Self::restore_minute) puts them back.
    pub fn take_minute(&self) -> Option<TownSummary> {
        self.inner().minute.take()
    }

    /// A minute that wasn't stored: its reports go back ahead of any since
    pub fn restore_minute(&self, earlier: TownSummary) {
        let mut inner = self.inner();
        inner.minute = Some(match inner.minute.take() {
            Some(later) => earlier.then(&later),
            None => earlier,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::metrics::CitizenReport;

    fn decision(at_ms: i64) -> DecisionBrief {
        DecisionBrief { at_ms, question: "work".into(), by: "rules".into(), ..DecisionBrief::default() }
    }

    fn report(started_ms: i64, decisions: &[(&str, &[i64])]) -> TownReport {
        TownReport {
            started_ms,
            citizens: decisions
                .iter()
                .map(|(name, at)| CitizenReport {
                    name: name.to_string(),
                    standing: "working".into(),
                    decisions: at.iter().map(|&t| decision(t)).collect(),
                    ..CitizenReport::default()
                })
                .collect(),
            rulings: [("rules".to_string(), 2)].into(),
            ..TownReport::default()
        }
    }

    #[test]
    fn the_town_comes_on_goes_quiet_and_comes_back() {
        let town = Town::new();
        assert!(town.latest().is_none());
        assert_eq!(town.quiet(0), None, "no town, nothing to go quiet");

        let mark = town.receive(report(1, &[("Ada", &[])]), 1_000).unwrap();
        assert_eq!(mark.kind, AnnotationKind::TownUp);
        assert_eq!(mark.text, "The town came on, with 1 citizen on its roster");
        assert_eq!(town.receive(report(1, &[("Ada", &[])]), 6_000), None, "still on");
        assert_eq!(town.quiet(6_000 + TOWN_QUIET_MS - 1), None);
        assert_eq!(town.quiet(6_000 + TOWN_QUIET_MS).unwrap().kind, AnnotationKind::TownDown);
        assert_eq!(town.quiet(6_000 + 2 * TOWN_QUIET_MS), None, "marked down once");
        assert!(!town.latest().unwrap().up);

        assert_eq!(town.receive(report(1, &[("Ada", &[])]), 200_000).unwrap().kind, AnnotationKind::TownUp);
        let restarted = town.receive(report(2, &[("Ada", &[])]), 205_000).unwrap();
        assert_eq!(restarted.text, "The town started again, with 1 citizen on its roster");

        let stopping = TownReport { stopping: true, ..report(2, &[("Ada", &[])]) };
        assert_eq!(town.receive(stopping.clone(), 210_000).unwrap().text, "The town stopped");
        assert_eq!(town.receive(stopping, 210_100), None, "a second last word marks nothing");
        assert_eq!(town.quiet(210_000 + TOWN_QUIET_MS), None, "it said it stopped");
    }

    #[test]
    fn a_citizens_last_decisions_are_kept_until_it_has_new_ones() {
        let town = Town::new();
        town.receive(report(1, &[("Ada", &[1, 2]), ("Bo", &[3])]), 1_000);
        town.receive(report(1, &[("Ada", &[]), ("Bo", &[4])]), 6_000);
        let latest = town.latest().unwrap();
        let at = |i: usize| latest.report.citizens[i].decisions.iter().map(|d| d.at_ms).collect::<Vec<_>>();
        assert_eq!(at(0), [1, 2], "nothing new from Ada");
        assert_eq!(at(1), [4]);
        // Off the roster, forgotten
        town.receive(report(1, &[("Bo", &[])]), 11_000);
        town.receive(report(1, &[("Ada", &[]), ("Bo", &[])]), 16_000);
        assert!(town.latest().unwrap().report.citizens[0].decisions.is_empty());
    }

    #[test]
    fn the_newest_report_names_each_citizens_trade() {
        let town = Town::new();
        assert!(town.roles().is_empty(), "no town, no trades");
        let mut first = report(1, &[("Ada", &[]), ("Bo", &[])]);
        first.citizens[0].role = "merchant".into();
        first.citizens[1].role = "gatherer-wood".into();
        town.receive(first, 1_000);
        let roles = town.roles();
        assert_eq!((roles["Ada"].as_str(), roles["Bo"].as_str()), ("merchant", "gatherer-wood"));
        town.quiet(1_000 + TOWN_QUIET_MS);
        assert_eq!(town.roles().len(), 2, "kept while the town is down");
    }

    #[test]
    fn the_samples_window_and_the_minutes_are_apart() {
        let town = Town::new();
        town.receive(report(1, &[("Ada", &[])]), 1_000);
        town.receive(report(1, &[("Ada", &[])]), 6_000);
        let sample = town.take_sample().unwrap();
        assert_eq!((sample.reports, sample.decisions(), sample.reported_ms), (2, 4, 6_000));
        assert_eq!(town.take_sample(), None, "nothing since");
        town.receive(report(1, &[("Ada", &[])]), 11_000);
        let minute = town.take_minute().unwrap();
        assert_eq!(minute.reports, 3, "the minute runs on past the sample");
        // It couldn't be stored, and a report came meanwhile
        town.receive(report(1, &[("Ada", &[])]), 16_000);
        town.restore_minute(minute);
        let minute = town.take_minute().unwrap();
        assert_eq!((minute.reports, minute.reported_ms), (4, 16_000));
        assert_eq!(town.take_minute(), None);
        assert_eq!(town.take_sample().unwrap().reports, 2);
    }
}
