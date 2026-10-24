//! Standing (commons plan D8, S3.1): what the city says of someone, in words.
//!
//! Chris chose words over a number. Others see the words; you see your own
//! number too, its trend and how it got there.
//!
//! - **The record** is what someone has done that others relied on: each
//!   delivery to a commission, each claim kept or let lapse, each shift of
//!   wages paid or earned, each week of property tax paid in full, each plot
//!   foreclosed ([`Deed`]). The server gathers it from the court's books and
//!   the tax run's (`engine::standing`).
//! - **The score** ([`score`]) adds the good and takes off each breach's
//!   weight. Good is the log of the Strands delivered, plus the claims kept,
//!   plus the log of the wages paid and of the wages earned, plus the weeks of
//!   tax in good standing. A lapse weighs three kept claims and a foreclosure
//!   ten.
//! - **Only breaches fade.** A breach's weight halves every
//!   [`HALF_LIFE_DAYS`] game days. Everything good stays done.
//! - **The words** ([`words`]) are five bands, the first by the count of
//!   records, the rest by the score. Over the band, [`LET_DOWN_LATELY`] shows
//!   while a lapse from the last [`LATELY_DAYS`] game days still weighs.
//!
//! Citizens are scored the same way. Both ends call these, so the client's
//! history and the server's words never disagree.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fmath::StableMath;

/// The first band: fewer than [`NEW_RECORDS`] records of any kind.
pub const NEW_IN_TOWN: &str = "new in town";
pub const FINDING_THEIR_FEET: &str = "finding their feet";
pub const DEPENDABLE: &str = "dependable";
pub const WELL_REGARDED: &str = "well regarded";
pub const PILLAR: &str = "a pillar of the Capitol";
/// Said over the band while a recent lapse still weighs.
pub const LET_DOWN_LATELY: &str = "has let people down lately";

/// Records it takes to be more than new in town.
pub const NEW_RECORDS: usize = 5;
/// The score each band above the second starts at, lowest first. Below the
/// first of them, even below 0, someone is finding their feet.
pub const BANDS: [(f64, &str); 3] = [(5.0, DEPENDABLE), (15.0, WELL_REGARDED), (40.0, PILLAR)];

/// Per `ln(1 + Strands)` delivered to commissions, over a life.
pub const DELIVERED_WEIGHT: f64 = 2.0;
/// Per claim kept.
pub const KEPT_WEIGHT: f64 = 1.0;
/// Per `ln(1 + Strands)` of wages paid, and again of wages earned.
pub const WAGES_WEIGHT: f64 = 1.0;
/// Per week of property tax paid in full.
pub const TAX_WEEK_WEIGHT: f64 = 1.0;
/// A lapse, when it's new: three kept claims.
pub const LAPSE_WEIGHT: f64 = 3.0 * KEPT_WEIGHT;
/// A foreclosure, when it's new.
pub const FORECLOSURE_WEIGHT: f64 = 10.0;
/// A breach's weight halves every this many game days (`rules::DAY_LENGTH_SECS`).
pub const HALF_LIFE_DAYS: f64 = 7.0;
/// A lapse younger than this, in game days, has let people down lately.
pub const LATELY_DAYS: f64 = 7.0;
/// The trend is the score now less the score this many game days ago.
pub const TREND_DAYS: f64 = 7.0;
/// Records the history shows, newest first.
pub const HISTORY: usize = 20;

/// Hours of property tax, paid with nothing owed, that make a week in good
/// standing: the tax's own week (`rules::FORECLOSE_AFTER_WEEKS`), not the
/// Capitol's fourteen-hour one.
pub const TAX_WEEK_HOURS: i32 = 168;

/// One thing someone did that others relied on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Deed {
    /// Units delivered to someone's commission, and what they paid
    Delivered { units: i32, value: i64 },
    /// A claim delivered in full, in time
    Kept { units: i32 },
    /// A claim run out short, or released in its last hour (S2.2)
    Lapsed { units: i32 },
    /// A shift of yours that someone worked, and what you paid them
    PaidWages { units: i32, value: i64 },
    /// A shift you worked for someone, and what they paid you
    EarnedWages { units: i32, value: i64 },
    /// A week of property tax paid with nothing owed ([`TAX_WEEK_HOURS`])
    TaxWeek,
    /// A plot foreclosed for unpaid tax
    Foreclosed,
}

impl Deed {
    /// A breach: it weighs against the score, and fades.
    pub fn is_breach(&self) -> bool {
        matches!(self, Deed::Lapsed { .. } | Deed::Foreclosed)
    }
}

/// A [`Deed`], when it was done, and with what and whom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Record {
    pub at: DateTime<Utc>,
    #[serde(flatten)]
    pub deed: Deed,
    /// The goods it was about, an item type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    /// Who else it was with: the poster, the worker or the employer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
}

/// How old `at` is at `now`, in game days. Never less than 0.
fn age_days(at: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    let ms = (now - at).num_milliseconds().max(0);
    ms as f64 / (super::DAY_LENGTH_SECS * 1000) as f64
}

/// What a breach weighs `age` game days on: halved every [`HALF_LIFE_DAYS`].
pub fn breach_weight(deed: &Deed, age: f64) -> f64 {
    let new = match deed {
        Deed::Lapsed { .. } => LAPSE_WEIGHT,
        Deed::Foreclosed => FORECLOSURE_WEIGHT,
        _ => return 0.0,
    };
    new * (-age.max(0.0) / HALF_LIFE_DAYS).stable_exp2()
}

/// The score at `now`, from the records done by then. Records after `now`
/// don't count yet, so [`trend`] can ask what it was a week ago.
pub fn score(records: &[Record], now: DateTime<Utc>) -> f64 {
    let (mut delivered, mut paid, mut earned) = (0i64, 0i64, 0i64);
    let (mut kept, mut weeks) = (0u32, 0u32);
    let mut breaches = 0.0;
    for r in records.iter().filter(|r| r.at <= now) {
        match r.deed {
            Deed::Delivered { value, .. } => delivered += value.max(0),
            Deed::Kept { .. } => kept += 1,
            Deed::PaidWages { value, .. } => paid += value.max(0),
            Deed::EarnedWages { value, .. } => earned += value.max(0),
            Deed::TaxWeek => weeks += 1,
            Deed::Lapsed { .. } | Deed::Foreclosed => breaches += breach_weight(&r.deed, age_days(r.at, now)),
        }
    }
    let good = DELIVERED_WEIGHT * (delivered as f64).stable_ln_1p()
        + KEPT_WEIGHT * kept as f64
        + WAGES_WEIGHT * ((paid as f64).stable_ln_1p() + (earned as f64).stable_ln_1p())
        + TAX_WEEK_WEIGHT * weeks as f64;
    good - breaches
}

/// When [`TREND_DAYS`] game days before `now` was.
fn week_before(now: DateTime<Utc>) -> DateTime<Utc> {
    now - chrono::Duration::milliseconds((TREND_DAYS * (super::DAY_LENGTH_SECS * 1000) as f64) as i64)
}

/// How the score has moved over the last [`TREND_DAYS`] game days.
pub fn trend(records: &[Record], now: DateTime<Utc>) -> f64 {
    score(records, now) - score(records, week_before(now))
}

/// The band's words, from the score and how many records there are.
pub fn band(score: f64, records: usize) -> &'static str {
    if records < NEW_RECORDS {
        return NEW_IN_TOWN;
    }
    BANDS.iter().rev().find(|(from, _)| score >= *from).map_or(FINDING_THEIR_FEET, |(_, words)| words)
}

/// A lapse from the last [`LATELY_DAYS`] game days: it still weighs more
/// than half its first weight, so more than a kept claim.
pub fn let_down_lately(records: &[Record], now: DateTime<Utc>) -> bool {
    records.iter().any(|r| matches!(r.deed, Deed::Lapsed { .. }) && r.at <= now && age_days(r.at, now) < LATELY_DAYS)
}

/// What the city says of someone at `now`: the band, and over it, a recent
/// lapse.
pub fn words(records: &[Record], now: DateTime<Utc>) -> String {
    let done = records.iter().filter(|r| r.at <= now).count();
    let band = band(score(records, now), done);
    if let_down_lately(records, now) {
        format!("{band}, but {LET_DOWN_LATELY}")
    } else {
        band.to_string()
    }
}

/// A line of someone's own history, with `item` the goods' name as the
/// reader's catalog gives it ("Kept a claim: 10 iron ore for Ada").
pub fn line(record: &Record, item: &str) -> String {
    let goods = |units: i32| {
        if item.is_empty() {
            format!("{units}")
        } else {
            format!("{units} {}", item.to_lowercase())
        }
    };
    let other = record.other.as_deref().unwrap_or("someone");
    match record.deed {
        Deed::Delivered { units, value } => format!("Delivered {} to {other}, for {value} Strands", goods(units)),
        Deed::Kept { units } => format!("Kept a claim: {} for {other}", goods(units)),
        Deed::Lapsed { units } => format!("Let a claim lapse: {} for {other}", goods(units)),
        Deed::PaidWages { units, value } => format!("Paid {other} {value} Strands for {}", goods(units)),
        Deed::EarnedWages { units, value } => format!("Earned {value} Strands from {other} for {}", goods(units)),
        Deed::TaxWeek => "A week of property tax paid in full".into(),
        Deed::Foreclosed => "A plot foreclosed for unpaid tax".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::DAY_LENGTH_SECS;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_792_368_000, 0).unwrap()
    }

    /// `days` game days.
    fn days(days: f64) -> Duration {
        Duration::milliseconds((days * (DAY_LENGTH_SECS * 1000) as f64) as i64)
    }

    fn rec(deed: Deed, days_ago: f64) -> Record {
        Record { at: now() - days(days_ago), deed, item_type: None, other: None }
    }

    fn kept(n: usize, days_ago: f64) -> Vec<Record> {
        (0..n).map(|_| rec(Deed::Kept { units: 5 }, days_ago)).collect()
    }

    #[test]
    fn fewer_than_five_records_is_new_in_town_whatever_the_score() {
        assert_eq!(words(&[], now()), NEW_IN_TOWN);
        let mut four = kept(3, 1.0);
        four.push(rec(Deed::Delivered { units: 50, value: 1_000_000 }, 1.0));
        assert!(score(&four, now()) > 30.0);
        assert_eq!(words(&four, now()), NEW_IN_TOWN);
        // The fifth makes them more than new
        four.push(rec(Deed::TaxWeek, 1.0));
        assert_eq!(words(&four, now()), WELL_REGARDED);
    }

    #[test]
    fn the_bands_rise_with_the_score() {
        assert_eq!(band(-3.0, 9), FINDING_THEIR_FEET);
        assert_eq!(band(0.0, 9), FINDING_THEIR_FEET);
        assert_eq!(band(4.99, 9), FINDING_THEIR_FEET);
        assert_eq!(band(5.0, 9), DEPENDABLE);
        assert_eq!(band(15.0, 9), WELL_REGARDED);
        assert_eq!(band(39.9, 9), WELL_REGARDED);
        assert_eq!(band(40.0, 9), PILLAR);
        assert_eq!(band(400.0, 4), NEW_IN_TOWN);
        // From a record: five kept claims are 5, dependable
        assert_eq!(words(&kept(5, 2.0), now()), DEPENDABLE);
        assert_eq!(words(&kept(4, 2.0), now()), NEW_IN_TOWN);
        let mut five = kept(4, 2.0);
        five.push(rec(Deed::TaxWeek, 1.0));
        assert_eq!(words(&five, now()), DEPENDABLE);
    }

    #[test]
    fn good_records_add_as_the_module_says() {
        let records = vec![
            rec(Deed::Delivered { units: 10, value: 99 }, 30.0),
            rec(Deed::Delivered { units: 1, value: 1 }, 2.0),
            rec(Deed::Kept { units: 10 }, 30.0),
            rec(Deed::PaidWages { units: 3, value: 9 }, 30.0),
            rec(Deed::EarnedWages { units: 2, value: 4 }, 30.0),
            rec(Deed::TaxWeek, 30.0),
        ];
        let want = DELIVERED_WEIGHT * 100f64.stable_ln_1p()
            + KEPT_WEIGHT
            + WAGES_WEIGHT * (9f64.stable_ln_1p() + 4f64.stable_ln_1p())
            + TAX_WEEK_WEIGHT;
        assert!((score(&records, now()) - want).abs() < 1e-9);
        // The log: a hundred times the Strands is nowhere near a hundred times the score
        let small = score(&[rec(Deed::Delivered { units: 1, value: 100 }, 1.0)], now());
        let big = score(&[rec(Deed::Delivered { units: 1, value: 10_000 }, 1.0)], now());
        assert!(big < 2.1 * small, "{small} {big}");
    }

    #[test]
    fn good_records_never_fade() {
        let mut records = kept(6, 0.5);
        records.extend([rec(Deed::Delivered { units: 4, value: 400 }, 0.5), rec(Deed::TaxWeek, 0.5)]);
        let later = now() + days(365.0);
        assert!((score(&records, later) - score(&records, now())).abs() < 1e-12);
        assert_eq!(words(&records, later), words(&records, now()));
    }

    #[test]
    fn a_breach_halves_each_week() {
        let lapse = Deed::Lapsed { units: 3 };
        assert_eq!(breach_weight(&lapse, 0.0), LAPSE_WEIGHT);
        assert!((breach_weight(&lapse, 7.0) - LAPSE_WEIGHT / 2.0).abs() < 1e-12);
        assert!((breach_weight(&lapse, 14.0) - LAPSE_WEIGHT / 4.0).abs() < 1e-12);
        assert!((breach_weight(&lapse, 70.0) - LAPSE_WEIGHT / 1024.0).abs() < 1e-12);
        assert_eq!(breach_weight(&Deed::Kept { units: 1 }, 0.0), 0.0, "only breaches weigh");

        // On the score: ten kept claims less a fresh lapse, then a week on
        let mut records = kept(10, 30.0);
        records.push(rec(lapse, 0.0));
        assert!((score(&records, now()) - 7.0).abs() < 1e-9);
        assert!((score(&records, now() + days(7.0)) - 8.5).abs() < 1e-9);
        assert!((score(&records, now() + days(14.0)) - 9.25).abs() < 1e-9);
    }

    #[test]
    fn a_lapse_lets_people_down_for_a_week() {
        let mut records = kept(10, 30.0);
        assert_eq!(words(&records, now()), DEPENDABLE);
        records.push(rec(Deed::Lapsed { units: 2 }, 1.0));
        assert_eq!(words(&records, now()), format!("{DEPENDABLE}, but {LET_DOWN_LATELY}"));
        // Past the week it's said no more, though it still weighs a little
        let past = now() + days(6.0);
        assert!(!let_down_lately(&records, past));
        assert!(score(&records, past) < 10.0);
        assert_eq!(words(&records, past), DEPENDABLE);
        // A foreclosure weighs, but isn't letting anyone down
        let mut foreclosed = kept(10, 30.0);
        foreclosed.push(rec(Deed::Foreclosed, 0.0));
        assert!(!let_down_lately(&foreclosed, now()));
    }

    #[test]
    fn a_foreclosure_weighs_ten_and_fades_like_a_lapse() {
        let mut records = kept(20, 30.0);
        assert_eq!(words(&records, now()), WELL_REGARDED);
        records.push(rec(Deed::Foreclosed, 0.0));
        assert!((score(&records, now()) - 10.0).abs() < 1e-9);
        assert_eq!(words(&records, now()), DEPENDABLE);
        assert!((score(&records, now() + days(14.0)) - 17.5).abs() < 1e-9);
        assert_eq!(words(&records, now() + days(14.0)), WELL_REGARDED);
    }

    #[test]
    fn the_trend_is_the_last_weeks_change() {
        let mut records = kept(5, 30.0);
        assert_eq!(trend(&records, now()), 0.0);
        records.extend(kept(3, 1.0));
        assert!((trend(&records, now()) - 3.0).abs() < 1e-9);
        // A fresh lapse takes the trend down by its weight
        records.push(rec(Deed::Lapsed { units: 1 }, 0.0));
        assert!(trend(&records, now()).abs() < 1e-9);
    }

    #[test]
    fn records_to_come_dont_count_yet() {
        let mut records = kept(5, 1.0);
        records.push(Record { at: now() + Duration::hours(1), ..rec(Deed::Lapsed { units: 1 }, 0.0) });
        assert_eq!(words(&records, now()), DEPENDABLE);
    }

    #[test]
    fn a_line_of_history_says_what_was_done_and_with_whom() {
        let r = |deed| Record { at: now(), deed, item_type: Some("iron_ore".into()), other: Some("Ada".into()) };
        assert_eq!(line(&r(Deed::Kept { units: 10 }), "Iron Ore"), "Kept a claim: 10 iron ore for Ada");
        assert_eq!(line(&r(Deed::Lapsed { units: 2 }), "Iron Ore"), "Let a claim lapse: 2 iron ore for Ada");
        assert_eq!(line(&r(Deed::Delivered { units: 4, value: 120 }), "Rope"), "Delivered 4 rope to Ada, for 120 Strands");
        assert_eq!(line(&r(Deed::PaidWages { units: 3, value: 9 }), "Plank"), "Paid Ada 9 Strands for 3 plank");
        assert_eq!(line(&r(Deed::EarnedWages { units: 3, value: 9 }), "Fiber"), "Earned 9 Strands from Ada for 3 fiber");
        assert_eq!(line(&rec(Deed::TaxWeek, 0.0), ""), "A week of property tax paid in full");
        assert_eq!(line(&rec(Deed::Foreclosed, 0.0), ""), "A plot foreclosed for unpaid tax");
    }

    #[test]
    fn a_record_says_its_deed_as_its_kind() {
        let r = Record { at: now(), deed: Deed::Delivered { units: 4, value: 120 }, item_type: Some("rope".into()), other: None };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"at":"2026-10-19T00:00:00Z","kind":"delivered","units":4,"value":120,"item_type":"rope"}"#);
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), r);
        let week = Record { at: now(), deed: Deed::TaxWeek, item_type: None, other: None };
        assert_eq!(serde_json::to_string(&week).unwrap(), r#"{"at":"2026-10-19T00:00:00Z","kind":"tax_week"}"#);
    }
}
