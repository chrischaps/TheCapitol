//! When a citizen plays (#276): a habit on the real clock, as people have.
//!
//! A person isn't on every game day. Most play a few hours a day, or every
//! couple of days; a few are on far more; some turn up now and then. So a
//! citizen has a [`Habit`]: a [`Kind`] and a home time zone, which together
//! say which real days and hours it signs in, and for how long. Its
//! [`Schedule`](super::schedule::Schedule) (C1.2) still says what it does
//! while it's on, by the game's clock, and game night doesn't end a
//! session: it changes the work (home at dusk, then the night's crafting
//! and gathering). Logging out belongs to the habit alone.
//!
//! - **dedicated**: 4 to 8 hours, most days;
//! - **regular**: 1 to 3 hours, 3 to 5 days a week;
//! - **casual**: about an hour, once or twice a week;
//! - **lapsed**: away for weeks, then back for a week or two of short
//!   sessions.
//!
//! Sessions fall in the citizen's own evenings, and on its weekends earlier
//! and longer, so the town has a real-world rhythm: busy evenings across the
//! zones, thin small hours. Each day's session is drawn from the citizen's
//! seed and the day, so it's the same however often the town asks, across
//! restarts too.
//!
//! The town's [`Mix`] of kinds is in the roster (`[habits] mix`), 5%
//! dedicated, 30% regular, 55% casual and 10% lapsed unless it says. A pool
//! of that mix is on about 3% of the time ([`Mix::share`]), so 20 online
//! takes about 650 citizens.

use std::fmt;

use super::persona::Rng;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// How much a citizen plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Dedicated,
    Regular,
    Casual,
    Lapsed,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Dedicated, Kind::Regular, Kind::Casual, Kind::Lapsed];

    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Dedicated => "dedicated",
            Kind::Regular => "regular",
            Kind::Casual => "casual",
            Kind::Lapsed => "lapsed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The town's kinds, as weights: what share of the citizens a mint makes
/// each. They needn't sum to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mix {
    pub dedicated: f64,
    pub regular: f64,
    pub casual: f64,
    pub lapsed: f64,
}

impl Default for Mix {
    /// Chris's starting mix (#276).
    fn default() -> Mix {
        Mix {
            dedicated: 0.05,
            regular: 0.30,
            casual: 0.55,
            lapsed: 0.10,
        }
    }
}

impl Mix {
    pub fn weight(&self, kind: Kind) -> f64 {
        match kind {
            Kind::Dedicated => self.dedicated,
            Kind::Regular => self.regular,
            Kind::Casual => self.casual,
            Kind::Lapsed => self.lapsed,
        }
    }

    /// Checks no weight is negative and some are more than 0.
    pub fn check(&self) -> Result<(), String> {
        if Kind::ALL.iter().any(|&k| !(self.weight(k) >= 0.0 && self.weight(k).is_finite())) {
            return Err("a habit's weight in the mix is 0 or more".into());
        }
        if self.total() <= 0.0 {
            return Err("the mix needs a kind with a weight more than 0".into());
        }
        Ok(())
    }

    fn total(&self) -> f64 {
        Kind::ALL.iter().map(|&k| self.weight(k)).sum()
    }

    /// The kind a draw `u` in [0, 1) falls on. A small change to the mix
    /// moves only the citizens whose draw is near a boundary.
    fn pick(&self, u: f64) -> Kind {
        let mut at = u * self.total();
        for kind in Kind::ALL {
            at -= self.weight(kind);
            if at < 0.0 {
                return kind;
            }
        }
        Kind::Casual
    }

    /// The share of the time a citizen of this mix is on, on average: the
    /// kinds' [`Kind::share`]s, weighted.
    pub fn share(&self) -> f64 {
        Kind::ALL.iter().map(|&k| self.weight(k) * k.share()).sum::<f64>() / self.total()
    }
}

impl Kind {
    /// The share of the time a citizen of this kind is on, on average, as
    /// a long run of seeds and weeks has it.
    pub fn share(&self) -> f64 {
        const SEEDS: u64 = 120;
        const WEEKS: i64 = 24;
        let from = 20_000 * DAY_MS;
        let to = from + WEEKS * 7 * DAY_MS;
        let on: i64 = (0..SEEDS)
            .map(|s| Habit::new(*self, 0, s.wrapping_mul(0x9E37_79B9_7F4A_7C15)).online_ms(from, to))
            .sum();
        on as f64 / (SEEDS as i64 * (to - from)) as f64
    }
}

/// Where citizens live in the world's time, as hours from UTC, and how many
/// of them: most in the Americas, then Europe, a few in Asia and Australia.
const ZONES: [(i32, f64); 11] = [
    (-8, 0.14),
    (-7, 0.05),
    (-6, 0.14),
    (-5, 0.24),
    (-3, 0.04),
    (0, 0.10),
    (1, 0.14),
    (2, 0.04),
    (8, 0.05),
    (9, 0.03),
    (10, 0.03),
];

/// When a citizen plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Habit {
    pub kind: Kind,
    /// Its home time zone, in hours from UTC: its evenings are its own.
    pub zone: i32,
    /// Its own stream for each day's and week's draws: its name's.
    seed: u64,
}

/// A stretch on the real clock, in unix milliseconds: from `start` to just
/// before `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    pub start: i64,
    pub end: i64,
}

/// Whether a citizen is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// In a session, until `until`.
    On { until: i64 },
    /// Away until `until`, its next session's start. `None`: no session in
    /// the next [`LOOKAHEAD_DAYS`], so look again later.
    Away { until: Option<i64> },
}

/// How far ahead the next session is looked for: a lapsed citizen can be
/// away for weeks.
pub const LOOKAHEAD_DAYS: i64 = 120;
/// The shares of weekday sessions by day and late at night; the rest are
/// in the evening.
const DAYTIME: f64 = 0.15;
const LATE: f64 = 0.15;

impl Habit {
    pub fn new(kind: Kind, zone: i32, seed: u64) -> Habit {
        Habit { kind, zone, seed }
    }

    /// A habit drawn from the citizen's stream, its kind by `mix`. The
    /// seed is kept for its days' draws.
    pub(crate) fn seeded(rng: &mut Rng, mix: &Mix, seed: u64) -> Habit {
        let kind = mix.pick(rng.unit());
        let mut at = rng.unit();
        let zone = ZONES
            .iter()
            .find(|(_, w)| {
                at -= w;
                at < 0.0
            })
            .map_or(-5, |(z, _)| *z);
        Habit { kind, zone, seed }
    }

    /// The same habit with its days drawn from `seed`.
    pub fn with_seed(self, seed: u64) -> Habit {
        Habit { seed, ..self }
    }

    /// Checks the time zone is one on Earth.
    pub fn check(&self) -> Result<(), String> {
        if !(-12..=14).contains(&self.zone) {
            return Err(format!("its habit's zone is {}; a zone is -12 to 14 hours from UTC", self.zone));
        }
        Ok(())
    }

    /// Its zone as people write it: `UTC-5`, `UTC`, `UTC+10`.
    pub fn zone_name(&self) -> String {
        match self.zone {
            0 => "UTC".into(),
            z => format!("UTC{z:+}"),
        }
    }

    /// `unix_ms` on its own clock: `"Thu 19:40"`.
    pub fn local_clock(&self, unix_ms: i64) -> String {
        const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let local = unix_ms + self.offset_ms();
        let weekday = (local.div_euclid(DAY_MS) + 3).rem_euclid(7) as usize;
        let minutes = local.rem_euclid(DAY_MS) / 60_000;
        format!("{} {:02}:{:02}", DAYS[weekday], minutes / 60, minutes % 60)
    }

    /// Whether it's on at `now` (unix ms), and until when.
    pub fn at(&self, now: i64) -> Presence {
        let today = self.local_day(now);
        // A session can run on past its own midnight
        for day in [today - 1, today] {
            if let Some(s) = self.session(day) {
                if s.start <= now && now < s.end {
                    return Presence::On { until: s.end };
                }
            }
        }
        let next = (today..today + LOOKAHEAD_DAYS)
            .filter_map(|day| self.session(day))
            .find(|s| s.start > now)
            .map(|s| s.start);
        Presence::Away { until: next }
    }

    /// Milliseconds it's on between `from` and `to`.
    pub fn online_ms(&self, from: i64, to: i64) -> i64 {
        (self.local_day(from) - 1..=self.local_day(to))
            .filter_map(|day| self.session(day))
            .map(|s| (s.end.min(to) - s.start.max(from)).max(0))
            .sum()
    }

    /// The days since 1970 on its own clock that `unix_ms` falls in.
    fn local_day(&self, unix_ms: i64) -> i64 {
        (unix_ms + self.offset_ms()).div_euclid(DAY_MS)
    }

    fn offset_ms(&self) -> i64 {
        self.zone as i64 * HOUR_MS
    }

    /// Its session starting on its own `day`, if it plays that day. One a
    /// day at most.
    pub fn session(&self, day: i64) -> Option<Session> {
        let weekday = (day + 3).rem_euclid(7); // 1970-01-01 was a Thursday; 0 is Monday
        let weekend = weekday >= 5;
        let week = (day + 3).div_euclid(7);
        let mut today = Rng::new(self.seed ^ (day as u64).wrapping_mul(0xA24B_AED4_963E_E407));
        let plays = match self.kind {
            Kind::Dedicated => today.unit() < if weekend { 0.95 } else { 0.8 },
            Kind::Regular => self.days_of(week, 3, 5).contains(&weekday),
            Kind::Casual => self.days_of(week, 1, 2).contains(&weekday),
            Kind::Lapsed => self.lapsed_back(week) && self.days_of(week, 2, 3).contains(&weekday),
        };
        if !plays {
            return None;
        }
        // Local hours: its start. Most weekday sessions are in the evening;
        // some by day, as a shift worker's or a student's, and some late,
        // a night owl's. Weekends from the morning on.
        let slot = today.unit();
        let start = match (self.kind, weekend) {
            (Kind::Dedicated, true) => today.between(10.0, 16.0),
            (_, true) => today.between(10.0, 21.0),
            _ if slot < DAYTIME => today.between(9.0, 15.0),
            _ if slot < DAYTIME + LATE => today.between(22.0, 24.0),
            (Kind::Dedicated, false) => today.between(15.0, 20.0),
            (Kind::Regular, false) => today.between(17.5, 22.0),
            _ => today.between(18.0, 22.5),
        };
        // And how long it plays: the regulars longer at weekends
        let hours = match (self.kind, weekend) {
            (Kind::Dedicated, _) => today.between(4.0, 8.0),
            (Kind::Regular, false) => today.between(1.0, 3.0),
            (Kind::Regular, true) => today.between(1.5, 2.75),
            (_, false) => today.between(0.5, 1.5),
            (_, true) => today.between(0.75, 2.0),
        };
        let start = day * DAY_MS + (start * HOUR_MS as f64) as i64 - self.offset_ms();
        Some(Session {
            start,
            end: start + (hours * HOUR_MS as f64) as i64,
        })
    }

    /// The weekdays (0 Monday) it plays in `week`: `fewest` to `most` of
    /// them, the weekend's twice as likely as a weekday.
    fn days_of(&self, week: i64, fewest: u32, most: u32) -> Vec<i64> {
        let mut rng = Rng::new(self.seed ^ (week as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ 0x5157);
        let n = fewest + ((rng.unit() * (most - fewest + 1) as f64) as u32).min(most - fewest);
        let mut left: Vec<(i64, f64)> = (0..7).map(|d| (d, if d >= 5 { 2.0 } else { 1.0 })).collect();
        let mut days = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let total: f64 = left.iter().map(|(_, w)| w).sum();
            let mut at = rng.unit() * total;
            let i = left
                .iter()
                .position(|(_, w)| {
                    at -= w;
                    at < 0.0
                })
                .unwrap_or(left.len() - 1);
            days.push(left.swap_remove(i).0);
        }
        days
    }

    /// Whether a lapsed citizen is back in `week`. Its weeks go in spells
    /// of six, each its own place in the cycle: back for the first one or
    /// two, then away for the rest.
    fn lapsed_back(&self, week: i64) -> bool {
        const SPELL: i64 = 6;
        let shifted = week + (self.seed % SPELL as u64) as i64;
        let spell = shifted.div_euclid(SPELL);
        let mut rng = Rng::new(self.seed ^ (spell as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25) ^ 0x1A95);
        let back = if rng.unit() < 0.5 { 1 } else { 2 };
        shifted.rem_euclid(SPELL) < back
    }
}

/// How many of `habits` are on in each real hour (UTC) from `from` for
/// `hours`, sampled at each hour's middle: what `capitol-town habits` charts.
pub fn online_by_hour(habits: &[Habit], from: i64, hours: usize) -> Vec<usize> {
    (0..hours as i64)
        .map(|h| {
            let at = from + h * HOUR_MS + HOUR_MS / 2;
            habits.iter().filter(|habit| matches!(habit.at(at), Presence::On { .. })).count()
        })
        .collect()
}

/// The citizens `habits` keep online on average from `from` for `hours`.
pub fn expected_online(habits: &[Habit], from: i64, hours: usize) -> f64 {
    let to = from + hours as i64 * HOUR_MS;
    habits.iter().map(|h| h.online_ms(from, to)).sum::<i64>() as f64 / (to - from) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 2026-10-19, 00:00 UTC.
    const MONDAY: i64 = 1_792_368_000_000;

    fn pool(n: u64, mix: &Mix) -> Vec<Habit> {
        (0..n)
            .map(|i| {
                let seed = i.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 77;
                Habit::seeded(&mut Rng::new(seed), mix, seed)
            })
            .collect()
    }

    #[test]
    fn monday_is_a_monday() {
        let day = MONDAY.div_euclid(DAY_MS);
        assert_eq!((day + 3).rem_euclid(7), 0);
    }

    #[test]
    fn a_habit_asks_the_same_whatever_the_moment() {
        let h = Habit::new(Kind::Regular, -5, 42);
        for day in 0..60 {
            assert_eq!(h.session(20_000 + day), h.session(20_000 + day));
        }
        // On within a session, and until its end; away just after, until the next
        let s = (20_000..20_100).find_map(|d| h.session(d)).unwrap();
        assert_eq!(h.at(s.start), Presence::On { until: s.end });
        assert_eq!(h.at(s.end - 1), Presence::On { until: s.end });
        match h.at(s.end) {
            Presence::Away { until: Some(next) } => {
                assert!(next > s.end);
                assert_eq!(h.at(next), Presence::On { until: h.at(next + 1).until() });
            }
            other => panic!("{other:?}"),
        }
    }

    impl Presence {
        fn until(&self) -> i64 {
            match self {
                Presence::On { until } => *until,
                Presence::Away { until } => until.unwrap_or(0),
            }
        }
    }

    #[test]
    fn each_kind_plays_as_its_people_do() {
        let weeks = 12;
        let from = MONDAY;
        let to = from + weeks * 7 * DAY_MS;
        let mean = |kind: Kind, measure: &dyn Fn(&Habit) -> f64| {
            let n = 200;
            (0..n).map(|s| measure(&Habit::new(kind, -5, s * 7919 + 3))).sum::<f64>() / n as f64
        };
        let days_a_week = |h: &Habit| {
            let (a, b) = (h.local_day(from), h.local_day(to));
            (a..b).filter(|&d| h.session(d).is_some()).count() as f64 / weeks as f64
        };
        let hours_a_session = |h: &Habit| {
            let (a, b) = (h.local_day(from), h.local_day(to));
            let s: Vec<_> = (a..b).filter_map(|d| h.session(d)).collect();
            if s.is_empty() {
                return 0.0;
            }
            s.iter().map(|s| (s.end - s.start) as f64).sum::<f64>() / s.len() as f64 / HOUR_MS as f64
        };
        let d = mean(Kind::Dedicated, &days_a_week);
        assert!((5.0..=7.0).contains(&d), "dedicated: {d} days a week");
        let h = mean(Kind::Dedicated, &hours_a_session);
        assert!((5.0..=7.0).contains(&h), "dedicated: {h} hours");
        let d = mean(Kind::Regular, &days_a_week);
        assert!((3.0..=5.0).contains(&d), "regular: {d} days a week");
        let h = mean(Kind::Regular, &hours_a_session);
        assert!((1.5..=3.0).contains(&h), "regular: {h} hours");
        let d = mean(Kind::Casual, &days_a_week);
        assert!((1.0..=2.0).contains(&d), "casual: {d} days a week");
        let h = mean(Kind::Casual, &hours_a_session);
        assert!((0.75..=1.5).contains(&h), "casual: about an hour, {h}");
        // Lapsed: most weeks away, some back
        let d = mean(Kind::Lapsed, &days_a_week);
        assert!((0.3..=1.2).contains(&d), "lapsed: {d} days a week");
        let away_weeks = |h: &Habit| (0..weeks).filter(|w| !h.lapsed_back(h.local_day(from).div_euclid(7) + w)).count() as f64;
        let away = mean(Kind::Lapsed, &away_weeks) / weeks as f64;
        assert!((0.6..0.85).contains(&away), "lapsed: away {away} of its weeks");
        // And the shares line up: dedicated far the most
        let shares: Vec<f64> = Kind::ALL.iter().map(Kind::share).collect();
        assert!(shares.windows(2).all(|w| w[0] > w[1]), "{shares:?}");
        assert!((0.15..0.3).contains(&shares[0]), "{shares:?}");
    }

    #[test]
    fn sessions_fall_mostly_in_a_citizens_own_evening() {
        // On weekdays a regular's or a casual's session starts by day, in
        // the evening or late, on its own clock: never at dawn, and most
        // in the evening
        let (mut evening, mut all) = (0, 0);
        for zone in [-8, 0, 9] {
            for s in 0..100u64 {
                let h = Habit::new(if s % 2 == 0 { Kind::Regular } else { Kind::Casual }, zone, s);
                for day in 20_000i64..20_028 {
                    let weekday = (day + 3).rem_euclid(7);
                    if let (Some(session), true) = (h.session(day), weekday < 5) {
                        let local = (session.start + h.offset_ms()).rem_euclid(DAY_MS) as f64 / HOUR_MS as f64;
                        assert!((9.0..15.0).contains(&local) || local >= 17.5, "zone {zone}: starts {local}");
                        assert_eq!(h.local_day(session.start), day);
                        evening += usize::from((17.5..22.5).contains(&local));
                        all += 1;
                    }
                }
            }
        }
        let share = evening as f64 / all as f64;
        assert!((0.6..0.8).contains(&share), "{share} in the evening");
    }

    #[test]
    fn a_mix_draws_its_kinds_and_a_change_moves_few() {
        let mix = Mix::default();
        let town = pool(2000, &mix);
        let share = |k: Kind| town.iter().filter(|h| h.kind == k).count() as f64 / 2000.0;
        assert!((0.03..0.07).contains(&share(Kind::Dedicated)), "{}", share(Kind::Dedicated));
        assert!((0.26..0.34).contains(&share(Kind::Regular)));
        assert!((0.51..0.59).contains(&share(Kind::Casual)));
        assert!((0.08..0.12).contains(&share(Kind::Lapsed)));
        // Most live in the Americas, some in Europe, a few further east
        let americas = town.iter().filter(|h| h.zone < -2).count() as f64 / 2000.0;
        assert!((0.55..0.67).contains(&americas), "{americas}");
        assert!(town.iter().any(|h| h.zone >= 8));
        // A tweak of the mix re-draws only those near a boundary
        let more = Mix { dedicated: 0.06, regular: 0.29, ..mix };
        let moved = town.iter().zip(pool(2000, &more)).filter(|(a, b)| a.kind != b.kind).count();
        assert!(moved < 60, "{moved} moved");
        assert_eq!(Mix { casual: -1.0, ..mix }.check().unwrap_err(), "a habit's weight in the mix is 0 or more");
        assert!(Mix { dedicated: 0.0, regular: 0.0, casual: 0.0, lapsed: 0.0 }.check().is_err());
    }

    /// The acceptance's pool: sized for 20 online by the mix's share, its
    /// average is about 20, and somebody is on in all but the rarest hour.
    /// The quietest is about 07:00 UTC, when the Americas are asleep and
    /// Europe's at work.
    #[test]
    fn a_pool_sized_for_twenty_keeps_twenty_on_and_seldom_none() {
        let mix = Mix::default();
        let share = mix.share();
        assert!((0.02..0.045).contains(&share), "the mix is on {share} of the time");
        let n = (20.0 / share).ceil() as u64;
        let town = pool(n, &mix);
        let hours = 4 * 7 * 24;
        let mean = expected_online(&town, MONDAY, hours);
        assert!((16.0..24.0).contains(&mean), "{n} citizens keep {mean} on");
        let by_hour = online_by_hour(&town, MONDAY, hours);
        let empty = by_hour.iter().filter(|&&n| n == 0).count();
        assert!(empty * 100 <= hours, "{empty} of {hours} hours with nobody on");
        // Each hour of the day, on average over the weeks
        let quietest = (0..24)
            .map(|h| by_hour.iter().skip(h).step_by(24).sum::<usize>() as f64 / (hours / 24) as f64)
            .fold(f64::MAX, f64::min);
        assert!(quietest >= 5.0, "the quietest hour has {quietest} on");
        let most = by_hour.iter().copied().max().unwrap();
        // Evenings in the Americas are the busy ones
        assert!(most as f64 > mean * 1.6, "{most} at the busiest against {mean}");
    }

    #[test]
    fn a_zone_is_on_earth_and_reads_as_people_write_it() {
        assert_eq!(Habit::new(Kind::Casual, -5, 1).zone_name(), "UTC-5");
        assert_eq!(Habit::new(Kind::Casual, 0, 1).zone_name(), "UTC");
        assert_eq!(Habit::new(Kind::Casual, 10, 1).zone_name(), "UTC+10");
        // Monday midnight UTC is Sunday evening in New York
        assert_eq!(Habit::new(Kind::Casual, -5, 1).local_clock(MONDAY + 90 * 60_000), "Sun 20:30");
        assert_eq!(Habit::new(Kind::Casual, 0, 1).local_clock(MONDAY), "Mon 00:00");
        assert!(Habit::new(Kind::Casual, 15, 1).check().is_err());
        for kind in Kind::ALL {
            assert_eq!(Kind::from_str(kind.as_str()), Some(kind));
        }
    }
}
