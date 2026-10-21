//! A citizen's day (D5, C1.2), on the server's clock: work from soon after
//! dawn, the market for an hour or two, work again, home at dusk with the
//! lantern lit, and asleep, logged out, through the night.
//!
//! Times are the day's fraction, as [`rules::time_of_day`] gives it: 0 is
//! midnight, 0.5 noon. A game day is two real hours
//! ([`rules::DAY_LENGTH_SECS`]), so a game hour is five real minutes. The
//! roster writes them as a game clock, `"14:05"`.
//!
//! [`rules::time_of_day`]: capitol_protocol::rules::time_of_day
//! [`rules::DAY_LENGTH_SECS`]: capitol_protocol::rules::DAY_LENGTH_SECS

use std::fmt;
use std::time::Duration;

use capitol_protocol::rules::DAY_LENGTH_SECS;

use super::persona::{Rng, Temperament};

/// Where in its day a citizen is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// At its trade: gathering, crafting, building, and any errand to the
    /// hall a step needs.
    Work,
    /// At the hall or the bazaar, trading what it has and lingering.
    Market,
    /// Walking home along the avenues, lantern lit.
    Home,
    /// Asleep: logged out, off the streets.
    Night,
}

impl Phase {
    pub const ALL: [Phase; 4] = [Phase::Work, Phase::Market, Phase::Home, Phase::Night];

    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Work => "work",
            Phase::Market => "market",
            Phase::Home => "home",
            Phase::Night => "night",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// When a citizen does what, as fractions of the day, in order through it:
/// `wake < market_open < market_close < home < bed`. Night runs from `bed`
/// over midnight to `wake`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Schedule {
    /// Signs in and goes to work.
    pub wake: f64,
    pub market_open: f64,
    pub market_close: f64,
    /// Leaves for home.
    pub home: f64,
    /// Logs out, home or not.
    pub bed: f64,
}

/// The town's day, before each citizen's shift: up at 06:00, soon after the
/// sun (`daylight.rs` has it on the horizon at 0.21, 05:02); home at 19:00,
/// as the evening comes on and the lanterns show; abed at 21:00, in the
/// dark. Market hours are the afternoon's, or for some the morning's.
const WAKE: f64 = 6.0 / 24.0;
const MORNING_MARKET: f64 = 9.5 / 24.0;
const AFTERNOON_MARKET: f64 = 14.0 / 24.0;
const HOME: f64 = 19.0 / 24.0;
const BED: f64 = 21.0 / 24.0;
/// How long the market keeps a citizen: an hour and a half for the
/// unsociable, up to three hours for the most sociable.
const MARKET_SHORTEST: f64 = 1.5 / 24.0;
const MARKET_SOCIAL: f64 = 1.5 / 24.0;
/// A citizen's whole day is early or late by up to this (40 game minutes),
/// so the town has a rush hour and not a flock.
const SHIFT: f64 = (40.0 / 60.0) / 24.0;
/// And each moment in it by up to this again (15 game minutes).
const JITTER: f64 = (15.0 / 60.0) / 24.0;
/// The share of citizens who keep the morning's market.
const MORNING_SHARE: f64 = 0.3;
/// The noon break (C2.1) starts this long after waking (about midday) and
/// lasts half a game hour.
const NOON_AFTER: f64 = 6.0 / 24.0;
const NOON_BREAK: f64 = 0.5 / 24.0;

impl Schedule {
    /// A day for a citizen of `temperament`, from its own random stream.
    pub(crate) fn seeded(rng: &mut Rng, temperament: &Temperament) -> Schedule {
        let shift = rng.between(-SHIFT, SHIFT);
        let open_base = if rng.unit() < MORNING_SHARE { MORNING_MARKET } else { AFTERNOON_MARKET };
        let mut at = |base: f64| base + shift + rng.between(-JITTER, JITTER);
        let wake = at(WAKE);
        let market_open = at(open_base);
        let market_close = market_open + MARKET_SHORTEST + MARKET_SOCIAL * f64::from(temperament.sociability);
        let home = at(HOME);
        let bed = at(BED);
        // As the roster writes it, so a minted citizen reads back the same
        Schedule {
            wake: to_minute(wake),
            market_open: to_minute(market_open),
            market_close: to_minute(market_close),
            home: to_minute(home),
            bed: to_minute(bed),
        }
    }

    /// Checks the times run in order through one day.
    pub fn check(&self) -> Result<(), String> {
        let times = [self.wake, self.market_open, self.market_close, self.home, self.bed];
        if times.iter().any(|t| !(0.0..1.0).contains(t)) {
            return Err("its times must fall within the day, 00:00 to 23:59".into());
        }
        if times.windows(2).any(|w| w[0] >= w[1]) {
            return Err(format!(
                "its day must run wake < market < home < bed, but it's wake {}, market {}, home {}, bed {}",
                clock(self.wake),
                self.market(),
                clock(self.home),
                clock(self.bed)
            ));
        }
        Ok(())
    }

    /// Where in the day `time` (0 midnight to 1) falls.
    pub fn phase(&self, time: f64) -> Phase {
        let t = time.rem_euclid(1.0);
        if t < self.wake || t >= self.bed {
            Phase::Night
        } else if t >= self.home {
            Phase::Home
        } else if t >= self.market_open && t < self.market_close {
            Phase::Market
        } else {
            Phase::Work
        }
    }

    /// The midday break between tasks (C2.1): half a game hour from six
    /// hours after waking, about noon, spent somewhere worth being. Only
    /// for a citizen whose market is in the afternoon: one who keeps the
    /// morning's has had its company already. Derived from the day, so
    /// the roster doesn't carry it.
    pub fn noon(&self) -> Option<(f64, f64)> {
        let start = self.wake + NOON_AFTER;
        let end = start + NOON_BREAK;
        (self.market_open >= end && end < self.home).then_some((start, end))
    }

    /// From `time` to the next `wake`, as a share of the day: 0 just at it.
    pub fn until_wake(&self, time: f64) -> f64 {
        (self.wake - time).rem_euclid(1.0)
    }

    /// The market's hours as the roster writes them: `"14:05-15:50"`.
    pub fn market(&self) -> String {
        format!("{}-{}", clock(self.market_open), clock(self.market_close))
    }
}

/// A share of the day as real time on the server's clock.
pub fn real(share: f64) -> Duration {
    Duration::from_secs_f64(share.max(0.0) * DAY_LENGTH_SECS as f64)
}

/// A share of the day as a game clock: `"05:30"`.
pub fn clock(time: f64) -> String {
    let minutes = (time.rem_euclid(1.0) * MINUTES).round() as u32 % MINUTES as u32;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// A game clock, `"05:30"`, as a share of the day.
pub fn parse_clock(text: &str) -> Option<f64> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && text.trim().len() == 5).then(|| (h * 60 + m) as f64 / MINUTES)
}

/// `"14:05-15:50"`, as an opening and a close.
pub fn parse_hours(text: &str) -> Option<(f64, f64)> {
    let (open, close) = text.split_once('-')?;
    Some((parse_clock(open)?, parse_clock(close)?))
}

const MINUTES: f64 = 24.0 * 60.0;

/// Rounded to a whole game minute, as the clock shows it.
fn to_minute(time: f64) -> f64 {
    (time.rem_euclid(1.0) * MINUTES).round() % MINUTES / MINUTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> Schedule {
        Schedule {
            wake: parse_clock("05:30").unwrap(),
            market_open: parse_clock("14:00").unwrap(),
            market_close: parse_clock("16:00").unwrap(),
            home: parse_clock("19:00").unwrap(),
            bed: parse_clock("21:00").unwrap(),
        }
    }

    #[test]
    fn the_day_runs_work_market_work_home_night() {
        let s = day();
        let at = |c: &str| s.phase(parse_clock(c).unwrap());
        assert_eq!(at("00:00"), Phase::Night);
        assert_eq!(at("05:29"), Phase::Night);
        assert_eq!(at("05:30"), Phase::Work);
        assert_eq!(at("13:59"), Phase::Work);
        assert_eq!(at("14:00"), Phase::Market);
        assert_eq!(at("15:59"), Phase::Market);
        assert_eq!(at("16:00"), Phase::Work);
        assert_eq!(at("19:00"), Phase::Home);
        assert_eq!(at("20:59"), Phase::Home);
        assert_eq!(at("21:00"), Phase::Night);
        assert_eq!(at("23:59"), Phase::Night);
        // Any day's time, and a whole day on
        assert_eq!(s.phase(1.0 + parse_clock("14:30").unwrap()), Phase::Market);
    }

    #[test]
    fn the_noon_break_comes_before_an_afternoon_market_only() {
        let s = day();
        let (start, end) = s.noon().unwrap();
        assert_eq!((clock(start), clock(end)), ("11:30".to_string(), "12:00".to_string()));
        // Still the day's work, by the phase: the break is taken from it
        assert_eq!(s.phase(start), Phase::Work);
        let morning = Schedule {
            market_open: parse_clock("09:30").unwrap(),
            market_close: parse_clock("11:00").unwrap(),
            ..s
        };
        assert_eq!(morning.noon(), None);
        // Seeded days that keep the afternoon's market all get one
        let mut rng = Rng::new(11);
        for _ in 0..200 {
            let temperament = Temperament::seeded(&mut rng);
            let s = Schedule::seeded(&mut rng, &temperament);
            assert_eq!(s.noon().is_some(), s.market_open > 0.5, "{s:?}");
        }
    }

    #[test]
    fn waking_is_due_round_midnight() {
        let s = day();
        let wake = s.wake;
        assert_eq!(s.until_wake(wake), 0.0);
        assert!((s.until_wake(parse_clock("21:00").unwrap()) - 8.5 / 24.0).abs() < 1e-9);
        assert!((s.until_wake(parse_clock("02:30").unwrap()) - 3.0 / 24.0).abs() < 1e-9);
        // A game hour is five real minutes
        assert_eq!(real(1.0 / 24.0), Duration::from_secs(300));
    }

    #[test]
    fn clocks_read_and_write() {
        assert_eq!(parse_clock("05:30"), Some(5.5 / 24.0));
        assert_eq!(clock(5.5 / 24.0), "05:30");
        assert_eq!(clock(0.999_99), "00:00");
        assert_eq!(parse_hours("14:05-15:50").map(|(a, b)| (clock(a), clock(b))), Some(("14:05".into(), "15:50".into())));
        for bad in ["24:00", "5:30", "05:60", "noon", "05:30:00", ""] {
            assert_eq!(parse_clock(bad), None, "{bad}");
        }
        assert_eq!(parse_hours("14:05"), None);
    }

    #[test]
    fn a_day_out_of_order_says_so() {
        let mut s = day();
        assert_eq!(s.check(), Ok(()));
        s.home = s.market_open;
        assert!(s.check().unwrap_err().contains("wake 05:30, market 14:00-16:00, home 14:00"));
    }

    #[test]
    fn seeded_days_are_in_order_shifted_and_spread() {
        let mut rng = Rng::new(7);
        let mut wakes = Vec::new();
        let mut mornings = 0;
        for _ in 0..200 {
            let temperament = Temperament::seeded(&mut rng);
            let s = Schedule::seeded(&mut rng, &temperament);
            assert_eq!(s.check(), Ok(()), "{s:?}");
            // A minute's the clock's grain
            assert_eq!(parse_clock(&clock(s.wake)), Some(s.wake));
            // Never up before the sun, nor out past ten
            assert!(s.wake > 0.21 && s.bed < 22.0 / 24.0, "{s:?}");
            // An hour and a half at the market at least, three at most
            let hours = (s.market_close - s.market_open) * 24.0;
            assert!((1.45..=3.05).contains(&hours), "{hours}");
            if s.market_open < 0.5 {
                mornings += 1;
            }
            wakes.push(s.wake);
        }
        let earliest = wakes.iter().copied().fold(1.0, f64::min);
        let latest = wakes.iter().copied().fold(0.0, f64::max);
        assert!((latest - earliest) * 24.0 > 1.0, "the town wakes over more than an hour");
        assert!((30..90).contains(&mornings), "{mornings} of 200 keep the morning market");
    }
}
