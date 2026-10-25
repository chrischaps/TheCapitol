//! Stints (Callings K1.2, plan D1, D4): what a citizen works at for the next
//! while, as against who it is. Pure.
//!
//! - **The job of the moment** is a [`Stint`]: a [`Role`], and the game
//!   hours it has run. Every menu runs on it: the work, what it sells and
//!   bids for, and whether it banks.
//! - **What it has done lately** is [`Lately`]: hours of stint time by job,
//!   fading by half every [`LATELY_HALF_LIFE`] of them, so the stint
//!   question chases its calling's shares over its last dozen stints or so,
//!   not its whole life.
//! - **The willing jobs** ([`willing`]) are those a citizen will answer for
//!   whatever it's doing this hour: commissions, wage work, a guild's
//!   charter. A person answers what it's able and willing to do.
//! - **The lot** is kept for the calling's stations ([`lot_for`]), not the
//!   job of the moment's (plan D6).
//! - **What's kept between sessions** is a [`Kept`], in the side file beside
//!   the roster (`town.stints.json`), so a casual citizen's half hour still
//!   adds up to a mix.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Commitment, Role, LOT_STATIONS, ROLES};
use crate::citizen::calling::Calling;

/// Game hours of work after which what a job had lately counts half. A
/// middling stint runs 1.5 (`rules::stint_hours`), so it's eight of them,
/// and six of the day's clock's days.
pub const LATELY_HALF_LIFE: f64 = 12.0;

/// The least share of its calling a job has for the citizen to answer for
/// it when it isn't the job of the moment: a specialist's far jobs (a
/// point or two each) aren't, its kin (about ten) are, and so is every
/// job of a leaner's and an all-rounder's.
pub const WILLING: f32 = 0.05;

/// Game hours a job whose work was stuck is left off the stint menu, so a
/// citizen whose kiln waits for a plot doesn't take it up again at once.
pub const STUCK_AGAIN: f64 = 1.0;

/// The job of the moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stint {
    pub job: Role,
    /// When it began, in the run's game hours, less what it had run in
    /// earlier sessions: so `now - since` is how long it's run.
    pub since: f64,
}

impl Stint {
    /// The game hours it has run at `now`.
    pub fn run(&self, now: f64) -> f64 {
        (now - self.since).max(0.0)
    }
}

/// Hours of stint time by job, fading as it works on ([`LATELY_HALF_LIFE`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Lately(BTreeMap<String, f64>);

impl Lately {
    /// `hours` more of `job`, with everything before them faded by as much.
    pub fn add(&mut self, job: Role, hours: f64) {
        if hours <= 0.0 {
            return;
        }
        let fade = 0.5f64.powf(hours / LATELY_HALF_LIFE);
        for v in self.0.values_mut() {
            *v *= fade;
        }
        *self.0.entry(job.slug().to_string()).or_insert(0.0) += hours;
        // Jobs it hasn't done in weeks
        self.0.retain(|_, v| *v >= 1e-3);
    }

    /// Hours of `job` lately.
    pub fn of(&self, job: Role) -> f64 {
        self.0.get(job.slug()).copied().unwrap_or(0.0)
    }

    /// Its share of the work lately, 0 to 1: 0 with nothing done.
    pub fn share(&self, job: Role) -> f64 {
        let all: f64 = self.0.values().sum();
        if all > 0.0 {
            self.of(job) / all
        } else {
            0.0
        }
    }

    /// As the stint question carries it: hours by slug, as `f32`s
    /// (`Question::Stint::lately`, #545).
    pub fn question(&self) -> BTreeMap<String, f32> {
        self.0.iter().map(|(job, h)| (job.clone(), *h as f32)).collect()
    }
}

/// What the side file keeps of a citizen between its sessions: its job, how
/// long it had run, and what it has done lately.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    /// [`Role::slug`]; none before its first stint
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// Game hours its stint had run
    #[serde(default)]
    pub run: f64,
    #[serde(default)]
    pub lately: Lately,
}

impl Kept {
    /// The stint it was on, at the run's `now`.
    pub fn stint(&self, now: f64) -> Option<Stint> {
        let job = Role::from_slug(self.job.as_deref()?)?;
        Some(Stint {
            job,
            since: now - self.run.max(0.0),
        })
    }
}

/// How far a citizen takes `job` (D5): the whole ladder for its favourite,
/// and a dabble at the rest. A swarm bot's calling is its role alone, so
/// it works its whole ladder, as before callings.
pub fn commitment(calling: &Calling, job: Role) -> Commitment {
    if calling.favourite == Some(job) {
        Commitment::Full
    } else {
        Commitment::Dabble
    }
}

/// The jobs a citizen will answer for, whatever it's at (plan §5): its
/// favourite first, then the job of the moment `doing`, then each job its
/// calling gives [`WILLING`] or more, the most first. Trading only for a
/// merchant (`trades`): it answers nothing and crafts nothing, and making
/// a market as a leaning is K2.2's.
pub fn willing(calling: &Calling, doing: Role, trades: bool) -> Vec<Role> {
    let mut jobs: Vec<Role> = calling.favourite.into_iter().collect();
    if !jobs.contains(&doing) {
        jobs.push(doing);
    }
    let mut leaning: Vec<(Role, f32)> = ROLES
        .into_iter()
        .filter(|r| !jobs.contains(r))
        .map(|r| (r, calling.share(r)))
        .filter(|&(_, share)| share >= WILLING)
        .collect();
    // Stable, so a tie keeps ROLES' order
    leaning.sort_by(|a, b| b.1.total_cmp(&a.1));
    jobs.extend(leaning.into_iter().map(|(r, _)| r));
    jobs.retain(|&r| r != Role::Merchant || trades);
    jobs
}

/// The stations a citizen's lot is kept for (plan D6): those of its
/// favourite and of each job its calling gives [`WILLING`] or more, in the
/// order a builder puts them down. They come from the calling, never the
/// job of the moment, so a change of stint never changes what its lot
/// should hold, and nothing is packed up on a switch. The station sets nest
/// within the builder's, so it's never more than a builder's 4. A swarm
/// bot's is its role's ([`Calling::single`]).
pub fn lot_for(calling: &Calling) -> Vec<&'static str> {
    let jobs: Vec<Role> = ROLES
        .into_iter()
        .filter(|&r| calling.favourite == Some(r) || calling.share(r) >= WILLING)
        .collect();
    LOT_STATIONS
        .into_iter()
        .filter(|s| jobs.iter().any(|j| j.lot_stations().contains(s)))
        .collect()
}

/// How many stations the lot a citizen buys has room for: its calling's
/// ([`lot_for`]), at least 1, as [`Role::room_wanted`] is a role's.
pub fn room_for(calling: &Calling) -> i32 {
    (lot_for(calling).len() as i32).max(1)
}

/// The jobs on a citizen's stint menu: every one its calling leans to at
/// all, trading only for a merchant (`trades`), in [`ROLES`] order.
pub fn menu_jobs(calling: &Calling, trades: bool) -> Vec<Role> {
    ROLES
        .into_iter()
        .filter(|&r| calling.share(r) > 0.0)
        .filter(|&r| r != Role::Merchant || trades)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citizen::calling::Mix;
    use crate::citizen::persona::Rng;
    use crate::sim::{Craft, Gather};

    const FIBER: Role = Role::Gatherer(Gather::Fiber);
    const WOOD: Role = Role::Gatherer(Gather::Wood);
    const PLANKS: Role = Role::Crafter(Craft::Planks);

    #[test]
    fn lately_fades_by_half_a_half_life_on() {
        let mut lately = Lately::default();
        lately.add(WOOD, 2.0);
        assert_eq!(lately.of(WOOD), 2.0);
        assert_eq!(lately.share(WOOD), 1.0);
        lately.add(PLANKS, LATELY_HALF_LIFE);
        assert!((lately.of(WOOD) - 1.0).abs() < 1e-9, "{lately:?}");
        assert!((lately.of(PLANKS) - LATELY_HALF_LIFE).abs() < 1e-9);
        assert!((lately.share(WOOD) - 1.0 / (1.0 + LATELY_HALF_LIFE)).abs() < 1e-9);
        // Nothing for nothing, and nothing done is no share of anything
        lately.add(FIBER, 0.0);
        assert_eq!(lately.of(FIBER), 0.0);
        assert_eq!(Lately::default().share(WOOD), 0.0);
        assert_eq!(lately.question()["gatherer-wood"], lately.of(WOOD) as f32);
    }

    #[test]
    fn what_is_kept_comes_back_as_the_stint_it_was() {
        let kept = Kept {
            job: Some("crafter-planks".into()),
            run: 0.5,
            lately: Lately::default(),
        };
        let stint = kept.stint(3.0).unwrap();
        assert_eq!(stint.job, PLANKS);
        assert!((stint.run(3.0) - 0.5).abs() < 1e-9);
        assert_eq!(Kept::default().stint(3.0), None);
        let json = serde_json::to_string(&kept).unwrap();
        assert_eq!(serde_json::from_str::<Kept>(&json).unwrap(), kept);
        // An old or hand-made entry reads with what it has
        assert_eq!(serde_json::from_str::<Kept>("{}").unwrap(), Kept::default());
    }

    /// What the rules take up next, for `calling` with `lately` done.
    fn next(calling: &Calling, lately: &Lately) -> Role {
        use crate::citizen::decide::{Cascade, Choice, Held, Place, Question, Situation, Temperament, When, Who, Why};
        let jobs = menu_jobs(calling, false);
        let s = Situation {
            who: Who {
                name: "Ada Reeder".into(),
                role: "builder".into(),
                temperament: Temperament::default(),
                calling: Some(calling.clone()),
            },
            at: Place { x: 0.0, y: 0.0, zone: "rural".into() },
            when: When { time_of_day: Some(0.3), hours: 0.0 },
            holdings: Held::default(),
            market: Vec::new(),
            question: Question::Stint {
                why: Why::Done,
                doing: None,
                run: 0.0,
                lately: lately.question(),
            },
            menu: jobs
                .iter()
                .map(|j| Choice::Take {
                    job: j.slug().into(),
                    setup: 0,
                    pays: 0,
                })
                .collect(),
            carry_on: Some(0),
        };
        let pick = Cascade::rules().ask(&s, std::time::Instant::now()).decision.unwrap().pick;
        jobs[pick]
    }

    /// Lately's fading keeps the shares the stint question chases: over a
    /// long run every job comes within a few points of its calling's share,
    /// and four game hours from nothing done are two jobs or more, for
    /// every birth trade in every style (K1.2's Verify).
    #[test]
    fn stints_by_lately_follow_the_calling_and_mix_within_four_hours() {
        use crate::citizen::decide::rules::stint_hours;
        use crate::citizen::decide::Temperament;
        let hours = stint_hours(&Temperament::default());
        let mut rng = Rng::new(11);
        for style in crate::citizen::calling::Style::ALL {
            for role in crate::sim::MIX.map(|(r, _)| r) {
                let calling = Calling::seeded(&mut rng, &Mix::only(style), role);
                let mut lately = Lately::default();
                let (mut jobs, mut at) = (Vec::new(), 0.0);
                while at < 4.0 {
                    let job = next(&calling, &lately);
                    jobs.push(job);
                    lately.add(job, hours);
                    at += hours;
                }
                jobs.dedup();
                assert!(jobs.len() >= 2, "{calling}: {jobs:?} in four hours");
                let mut done: std::collections::HashMap<Role, f64> = std::collections::HashMap::new();
                for _ in 0..300 {
                    let job = next(&calling, &lately);
                    lately.add(job, hours);
                    *done.entry(job).or_default() += 1.0 / 300.0;
                }
                // Trading is off the menu but a merchant's, and its share
                // goes to whichever job is short of its own
                let slack = 0.03 + f64::from(calling.share(Role::Merchant));
                for job in menu_jobs(&calling, false) {
                    let (got, want) = (done.get(&job).copied().unwrap_or(0.0), f64::from(calling.share(job)));
                    assert!((got - want).abs() < slack, "{calling}: {job} {got:.3} against {want:.3}");
                }
            }
        }
    }

    #[test]
    fn a_swarm_bot_answers_and_works_for_its_role_alone() {
        let one = Calling::single(WOOD);
        assert_eq!(willing(&one, WOOD, false), [WOOD]);
        assert_eq!(menu_jobs(&one, false), [WOOD]);
        assert_eq!(commitment(&one, WOOD), Commitment::Full);
    }

    #[test]
    fn the_willing_are_the_favourite_the_job_at_hand_and_the_real_leanings() {
        let mut rng = Rng::new(7);
        for style in crate::citizen::calling::Style::ALL {
            for role in crate::sim::ROLES {
                let calling = Calling::seeded(&mut rng, &Mix::only(style), role);
                let jobs = willing(&calling, PLANKS, role == Role::Merchant);
                if let Some(favourite) = calling.favourite.filter(|&f| f != Role::Merchant || role == Role::Merchant) {
                    assert_eq!(jobs[0], favourite, "{calling}");
                }
                assert!(jobs.contains(&PLANKS));
                let distinct: std::collections::HashSet<_> = jobs.iter().collect();
                assert_eq!(distinct.len(), jobs.len(), "{jobs:?}");
                for job in crate::sim::ROLES {
                    let leans = calling.share(job) >= WILLING && (job != Role::Merchant || role == Role::Merchant);
                    if leans {
                        assert!(jobs.contains(&job), "{calling}: {job} left out of {jobs:?}");
                    }
                }
                // Everyone answers for more than one job
                assert!(jobs.len() >= 2, "{calling}: {jobs:?}");
                // Only a merchant trades
                assert_eq!(jobs.contains(&Role::Merchant), role == Role::Merchant, "{calling}");
                assert_eq!(commitment(&calling, role), if calling.favourite.is_some() { Commitment::Full } else { Commitment::Dabble });
                assert!(menu_jobs(&calling, false).len() >= 3, "{calling}");
            }
        }
    }
}
