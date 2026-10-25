//! A citizen's stints (Callings K1.2, D4): it asks what to work at for the
//! next while ([`Question::Stint`]) when its session starts, when the stint
//! it's on has run its length ([`stint_hours`]), and when a claim it took
//! pulls it toward another job. The rules chase its calling's shares
//! against what it has done [lately](super::super::stint::Lately), so over
//! a day or so its work comes out in its calling's proportions.
//!
//! A swarm bot has no persona, so its calling is its role alone: it never
//! asks, and its job is its role, as before callings. Nor does anyone ask
//! on a wage's shift, whose work is its employer's (S2.8).

use std::collections::HashMap;

use super::super::plan::Holdings;
use super::super::stint::{self, Kept, Stint, STUCK_AGAIN};
use super::super::wage::Job;
use super::super::{Commitment, Role};
use capitol_protocol::rules::DAY_LENGTH_SECS;
use super::SimBot;
use crate::citizen::decide::rules::stint_hours;
use crate::citizen::decide::{Choice, Question, Why};

/// Game hours in a game day, which a claim's day's work is made in.
const DAY_GAME_HOURS: f64 = DAY_LENGTH_SECS as f64 / 3600.0;

/// A citizen's stints: the one it's on, what it has done lately, and what
/// pulls on it.
#[derive(Debug, Clone, Default)]
pub(super) struct Stints {
    /// The job of the moment; `None` before the first, for a citizen whose
    /// side file had none.
    pub(super) on: Option<Stint>,
    pub(super) lately: stint::Lately,
    /// Whether it has asked this session: a session's first round asks,
    /// whatever it was on before.
    asked: bool,
    /// The run's game hours up to which `lately` has its time.
    credited: Option<f64>,
    /// A claim it took for another job's goods, and what it pays a game
    /// hour over the books, since it last asked.
    pulled: Option<(Role, i64)>,
    /// Jobs whose work was stuck, and when (in the run's game hours).
    stuck: HashMap<Role, f64>,
}

impl Stints {
    /// From what the side file kept, at the run's `now`.
    pub(super) fn from_kept(kept: &Kept, now: f64) -> Stints {
        Stints {
            on: kept.stint(now),
            lately: kept.lately.clone(),
            ..Stints::default()
        }
    }

    /// What the side file keeps, at the run's `now`.
    pub(super) fn kept(&self, now: f64) -> Kept {
        Kept {
            job: self.on.map(|s| s.job.slug().to_string()),
            run: self.on.map_or(0.0, |s| s.run(now)),
            lately: self.lately.clone(),
        }
    }
}

impl SimBot {
    /// The job of the moment (plan D1): its stint's, or its role's for a
    /// swarm bot and before the first.
    pub(super) fn job(&self) -> Role {
        self.stints.on.map_or(self.role, |s| s.job)
    }

    /// How far it takes the job of the moment (D5).
    pub(super) fn commitment(&self) -> Commitment {
        stint::commitment(&self.calling, self.job())
    }

    /// The jobs it answers for, whatever it's at (plan §5), its favourite
    /// first.
    pub(super) fn willing(&self) -> Vec<Role> {
        stint::willing(&self.calling, self.job(), self.merchant.is_some())
    }

    /// The first of its [willing](Self::willing) jobs that answers a
    /// commission for `item`, if any does.
    pub(super) fn answerer(&self, item: &str) -> Option<Role> {
        self.willing().into_iter().find(|r| r.answers().contains(&item))
    }

    /// The first of its willing jobs that does `job`'s work for a wage.
    pub(super) fn doer(&self, job: &Job) -> Option<Role> {
        self.willing().into_iter().find(|r| r.does(job))
    }

    /// Puts the time since it last did on the job of the moment's lately.
    pub(super) fn credit(&mut self) {
        let now = self.now();
        let since = self.stints.credited.replace(now).unwrap_or(now);
        if self.persona.is_some() {
            let job = self.job();
            self.stints.lately.add(job, now - since);
        }
    }

    /// A claim it took on `item` (S2.5) for `pays` a game hour over the
    /// books: a pull toward the job that answers for it, if that isn't the
    /// one it's at.
    pub(super) fn pulled(&mut self, item: &str, pays: i64) {
        if self.persona.is_none() {
            return;
        }
        if let Some(job) = self.answerer(item).filter(|&j| j != self.job()) {
            self.stints.pulled = Some((job, pays));
        }
    }

    /// The job of the moment's work was stuck, or there was none: it's left
    /// off the stint menu for [`STUCK_AGAIN`], and the stint ends, so the
    /// next round asks again. Its favourite's stays, so a specialist waiting
    /// for its plot keeps at it.
    pub(super) fn stint_stuck(&mut self) {
        let Some(on) = self.stints.on.filter(|s| self.persona.is_some() && Some(s.job) != self.calling.favourite) else {
            return;
        };
        let now = self.now();
        self.stints.stuck.insert(on.job, now);
        // Run its length, so the next round asks why it's done
        let hours = stint_hours(&self.mind.temperament);
        self.stints.on = Some(Stint {
            since: on.since.min(now - hours),
            ..on
        });
    }

    /// Asks what to work at, if it's time to (D4): a session's first round,
    /// a stint run its length, or a pull. Not a swarm bot's, nor on a
    /// shift. The decision is settled at once: taking up a job is done as
    /// it's chosen.
    pub(super) fn mind_stint(&mut self, holdings: &Holdings) {
        if self.persona.is_none() || self.wages.on_shift() {
            return;
        }
        self.credit();
        let now = self.now();
        let hours = stint_hours(&self.mind.temperament);
        let why = match self.stints.on {
            _ if !self.stints.asked => Why::Start,
            None => Why::Start,
            _ if self.stints.pulled.is_some() => Why::Pull,
            Some(on) if on.run(now) >= hours => Why::Done,
            Some(_) => return,
        };
        self.stints.asked = true;
        let pulled = self.stints.pulled.take();
        self.stints.stuck.retain(|_, at| now - *at < STUCK_AGAIN);
        let catalog = self.bot.state().catalog().clone();
        let doing = self.stints.on.map(|s| s.job);
        let mut jobs: Vec<(Role, Choice)> = stint::menu_jobs(&self.calling, self.merchant.is_some())
            .into_iter()
            .filter(|job| !self.stints.stuck.contains_key(job))
            .filter_map(|job| {
                let pays = pulled.filter(|(j, _)| *j == job).map_or(0, |(_, p)| p);
                Some((job, job.take(&catalog, holdings, stint::commitment(&self.calling, job), pays)?))
            })
            .collect();
        let pick = match jobs.len() {
            0 => None,
            // Nothing to choose between
            1 => Some(jobs[0].0),
            _ => {
                let carry_on = doing
                    .or(self.calling.favourite)
                    .and_then(|d| jobs.iter().position(|(j, _)| *j == d))
                    .unwrap_or(0);
                let question = Question::Stint {
                    why,
                    doing: doing.map(|d| d.slug().to_string()),
                    run: self.stints.on.map_or(0.0, |s| s.run(now)) as f32,
                    lately: self.stints.lately.question(),
                };
                let mark = self.mind.mark();
                let menu = jobs.iter().map(|(_, c)| c.clone()).collect();
                let picked = self.decide(question, menu, carry_on, holdings).map(|(d, _)| d.pick);
                self.settle(mark, None, &Ok(()));
                picked.map(|i| jobs.swap_remove(i).0)
            }
        };
        let Some(job) = pick else {
            // Nothing it can take up now: it keeps at what it was, if anything
            return;
        };
        let carries_on = doing == Some(job) && why != Why::Done;
        if !carries_on {
            self.stints.on = Some(Stint { job, since: now });
        }
    }
}

/// Game hours a claim takes to make `day_work` of, and so what `price` a
/// unit over `worth` comes to a game hour: what pulls on the job that makes
/// it (D4).
pub(super) fn claim_pays(price: i64, worth: Option<i64>, day_work: i32) -> i64 {
    let over = (price - worth.unwrap_or(0)).max(0);
    (over as f64 * day_work.max(0) as f64 / DAY_GAME_HOURS).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_pulls_by_what_it_pays_over_the_books_an_hour() {
        // Ten over the books on a day's 24 at a day of 24 game hours
        assert_eq!(claim_pays(18, Some(8), 24), (10.0 * 24.0 / DAY_GAME_HOURS).round() as i64);
        assert_eq!(claim_pays(8, Some(10), 24), 0);
        // With no books to go by, the whole price
        assert_eq!(claim_pays(5, None, 0), 0);
        assert!(claim_pays(5, None, 100) > 0);
    }

    #[test]
    fn a_kept_stint_comes_back_and_goes_out_again() {
        let mut lately = stint::Lately::default();
        lately.add(Role::Builder, 3.0);
        let kept = Kept {
            job: Some("builder".into()),
            run: 0.4,
            lately,
        };
        let stints = Stints::from_kept(&kept, 10.0);
        assert_eq!(stints.on.map(|s| s.job), Some(Role::Builder));
        assert!(!stints.asked, "a new session asks again");
        let out = stints.kept(10.5);
        assert_eq!(out.job.as_deref(), Some("builder"));
        assert!((out.run - 0.9).abs() < 1e-9);
        assert_eq!(out.lately, kept.lately);
    }
}
