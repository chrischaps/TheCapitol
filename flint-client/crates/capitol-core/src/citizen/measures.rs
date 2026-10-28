//! §10's callings measures (the callings plan, #553): what a soak's
//! citizens' callings came to, read straight off its report and its
//! balance. K4.1 (#542) computed them by hand, from the roster, the
//! activity CSV, the decision logs and the database.
//!
//! - **The favourite's share** of each citizen's time by its calling's
//!   style, against the style's band: specialists 60–80%, leaners 35–55%,
//!   and no all-rounder's biggest job over 35%. Of all its work (§10's
//!   measure), of the eight jobs (trading among them), and of the seven
//!   making jobs alone (#555).
//! - **The fewest activities a day** a citizen worked: 3, all-rounders 5.
//! - **Detours**: citizens who took one, and days worked with one (70%).
//! - **Asks between citizens** (K3.2) and what came of them.
//! - **"Lately busy with"**: whether the card's first job is the making job
//!   it gave most time to (80%), and whether the card names it at all.
//!
//! A [`CitizenCalling`] is one citizen's line, kept in a balance's JSON;
//! [`callings_md`] says them for one run or several. Pure, so it's tested
//! without a server.

use std::fmt::Write as _;
use std::ops::AddAssign;

use capitol_protocol::rules::lately::{self, Lately};
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::sim::report::ActivityTime;
use crate::sim::{Activity, Role};

/// A specialist's favourite's share of its work, §10's band
pub const SPECIALIST_BAND: (f64, f64) = (0.6, 0.8);
/// A leaner's
pub const LEANER_BAND: (f64, f64) = (0.35, 0.55);
/// The most an all-rounder's biggest job should take
pub const ALL_ROUNDER_MOST: f64 = 0.35;
/// The fewest activities a day §10 wants of every citizen, and of an
/// all-rounder
pub const FEWEST: usize = 3;
pub const FEWEST_ALL_ROUNDER: usize = 5;

/// The calling styles, in a table's order
const STYLES: [&str; 3] = ["specialist", "leaner", "all-rounder"];

/// One citizen's calling and what its time went on, for §10.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CitizenCalling {
    pub name: String,
    /// `specialist`, `leaner` or `all-rounder`; `None` when the town didn't
    /// say (a town before callings)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Its favourite job's slug (`gatherer-wood`); none for an all-rounder
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favourite: Option<String>,
    /// What its time went on
    #[serde(default)]
    pub time: ActivityTime,
    /// Days it worked with a detour in them
    #[serde(default)]
    pub detour_days: usize,
    /// Its card's "lately busy with" at the end, as the jobs it names, most
    /// first (activity slugs: `wood`, `planks`). Empty for a quiet card;
    /// `None` when it wasn't read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lately: Option<Vec<String>>,
    #[serde(default)]
    pub asks: Asks,
}

/// Times a citizen with something to ask the citizens near it (K3.2)
/// asked, and what came of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asks {
    /// It had someone to ask, and let them be
    #[serde(default)]
    pub let_be: usize,
    /// It asked, and they traded
    #[serde(default)]
    pub traded: usize,
    /// It asked, and they declined
    #[serde(default)]
    pub declined: usize,
    /// It asked, and the trade ended otherwise: cancelled, walked off,
    /// nothing offered, never opened
    #[serde(default)]
    pub ended: usize,
    /// It asked, and the asking failed on the way
    #[serde(default)]
    pub failed: usize,
}

impl Asks {
    /// Times it asked
    pub fn asked(&self) -> usize {
        self.traded + self.declined + self.ended + self.failed
    }
}

impl AddAssign for Asks {
    fn add_assign(&mut self, other: Asks) {
        self.let_be += other.let_be;
        self.traded += other.traded;
        self.declined += other.declined;
        self.ended += other.ended;
        self.failed += other.failed;
    }
}

/// The jobs a card's "lately busy with" names, most first, as activity
/// slugs: each of its activities' goods is a job's product or an input on
/// the way to one (`Activity::of_item`), and an input says nothing. Nor
/// does "making" an item the table doesn't know: the card names it by its
/// name, not its id.
pub fn card_jobs(card: &Lately, catalog: &Catalog) -> Vec<String> {
    let mut jobs: Vec<String> = Vec::new();
    for doing in &card.doing {
        if let Some(job) = lately::items_of(doing).iter().find_map(|item| Activity::of_item(catalog, item)) {
            if !jobs.iter().any(|j| j == job.slug()) {
                jobs.push(job.slug().to_string());
            }
        }
    }
    jobs
}

impl CitizenCalling {
    /// The job its favourite is
    pub fn favourite_job(&self) -> Option<Activity> {
        self.favourite.as_deref().and_then(Role::from_slug).map(Activity::of_role)
    }

    fn hours_over(&self, jobs: impl IntoIterator<Item = Activity>) -> f64 {
        jobs.into_iter().map(|a| self.time.hours_on(a)).sum()
    }

    /// Its favourite's share of its work (§10's measure)
    pub fn of_work(&self) -> Option<f64> {
        self.time.share(self.favourite_job()?)
    }

    /// Its favourite's share of the eight jobs, trading among them
    pub fn of_jobs(&self) -> Option<f64> {
        let job = self.favourite_job()?;
        let of = self.hours_over(Activity::JOBS);
        (of > 0.0).then(|| self.time.hours_on(job) / of)
    }

    /// Its favourite's share of the seven making jobs: none for a trading
    /// favourite
    pub fn of_making(&self) -> Option<f64> {
        let job = self.favourite_job().filter(|j| *j != Activity::Trading)?;
        let of = self.hours_over(making());
        (of > 0.0).then(|| self.time.hours_on(job) / of)
    }

    /// Its biggest job's share of its work: an all-rounder's measure
    pub fn biggest_job(&self) -> Option<f64> {
        self.time.share(self.time.top_job()?)
    }

    /// The share §10 holds it to: its favourite's of its work, or an
    /// all-rounder's biggest job
    fn measured(&self) -> Option<f64> {
        match self.style.as_deref() {
            Some("all-rounder") => self.biggest_job(),
            _ => self.of_work(),
        }
    }

    /// Whether its share is in its style's band. `None` with no style or no
    /// work.
    pub fn in_band(&self) -> Option<bool> {
        let share = self.measured()?;
        Some(match self.style.as_deref()? {
            "specialist" => (SPECIALIST_BAND.0..=SPECIALIST_BAND.1).contains(&share),
            "leaner" => (LEANER_BAND.0..=LEANER_BAND.1).contains(&share),
            _ => share <= ALL_ROUNDER_MOST,
        })
    }

    /// The fewest activities it worked at in a day
    pub fn fewest_activities(&self) -> Option<usize> {
        self.time.activities_a_day().map(|(_, fewest)| fewest)
    }

    /// The making job it gave most time to
    pub fn most_made(&self) -> Option<Activity> {
        making()
            .filter(|a| self.time.hours_on(*a) > 0.0)
            .max_by(|a, b| self.time.hours_on(*a).total_cmp(&self.time.hours_on(*b)))
    }

    /// Whether its card's first job is the making job it gave most time
    /// to, and whether the card names it at all. `None` without a card read
    /// or with no making.
    pub fn lately_matches(&self) -> Option<(bool, bool)> {
        let card = self.lately.as_ref()?;
        let most = self.most_made()?.slug();
        Some((card.first().is_some_and(|j| j == most), card.iter().any(|j| j == most)))
    }
}

/// The seven making jobs: the eight but trading
fn making() -> impl Iterator<Item = Activity> {
    Activity::JOBS.into_iter().filter(|a| *a != Activity::Trading)
}

fn median(mut xs: Vec<f64>) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(f64::total_cmp);
    let mid = xs.len() / 2;
    Some(if xs.len().is_multiple_of(2) { (xs[mid - 1] + xs[mid]) / 2.0 } else { xs[mid] })
}

fn pct(x: Option<f64>) -> String {
    x.map_or("—".to_string(), |x| format!("{:.0}%", x * 100.0))
}

fn of(n: usize, all: usize) -> String {
    if all == 0 {
        "—".into()
    } else {
        format!("{n} of {all} ({:.0}%)", 100.0 * n as f64 / all as f64)
    }
}

/// §10's callings measures for each of `runs` (a name and its citizens),
/// side by side: one run for a soak's report, two for a comparison. A run
/// from before callings were kept says so.
pub fn callings_md(md: &mut String, runs: &[(&str, &[CitizenCalling])]) {
    let _ = writeln!(md, "## Callings (§10)\n");
    let kept: Vec<&(&str, &[CitizenCalling])> = runs.iter().filter(|(_, c)| !c.is_empty()).collect();
    if kept.is_empty() {
        let _ = writeln!(md, "No citizen's calling was kept: {} from before they were (#553).\n", if runs.len() == 1 { "the run is" } else { "every run is" });
        return;
    }
    for (name, citizens) in runs {
        if citizens.is_empty() {
            let _ = writeln!(md, "{name} is from before callings were kept, so it has none of this.\n");
        }
    }
    let many = runs.len() > 1;
    let head = |first: &str| if many { format!("| {first} | Run |") } else { format!("| {first} |") };
    let rule = |n: usize| format!("|---|{}{}", if many { "---|" } else { "" }, "---:|".repeat(n));
    let run_cell = |name: &str| if many { format!(" {name} |") } else { String::new() };

    // The favourite's share by style
    let _ = writeln!(
        md,
        "Each style's favourite's share of its citizens' time, the median, against §10's band: specialists {}–{}, leaners {}–{} of the work, and no all-rounder's biggest job over {}. Of the jobs counts the eight, trading among them; of the making jobs counts the seven without it (#555), so a trading favourite has none.\n",
        pct(Some(SPECIALIST_BAND.0)),
        pct(Some(SPECIALIST_BAND.1)),
        pct(Some(LEANER_BAND.0)),
        pct(Some(LEANER_BAND.1)),
        pct(Some(ALL_ROUNDER_MOST))
    );
    let _ = writeln!(md, "{} Citizens | Of the work | Of the jobs | Of the making jobs | In band |", head("Style"));
    let _ = writeln!(md, "{}", rule(5));
    for style in STYLES {
        for (name, citizens) in &kept {
            let these: Vec<&CitizenCalling> = citizens.iter().filter(|c| c.style.as_deref() == Some(style)).collect();
            if these.is_empty() {
                continue;
            }
            let col = |f: fn(&CitizenCalling) -> Option<f64>| pct(median(these.iter().filter_map(|c| f(c)).collect()));
            let (work, jobs, making) = if style == "all-rounder" {
                (format!("{} (biggest job)", col(CitizenCalling::biggest_job)), "—".into(), "—".into())
            } else {
                (col(CitizenCalling::of_work), col(CitizenCalling::of_jobs), col(CitizenCalling::of_making))
            };
            let measured: Vec<bool> = these.iter().filter_map(|c| c.in_band()).collect();
            let in_band = of(measured.iter().filter(|b| **b).count(), measured.len());
            let _ = writeln!(md, "| {style} |{} {} | {work} | {jobs} | {making} | {in_band} |", run_cell(name), these.len());
        }
    }
    md.push('\n');

    // Days, detours, asks and the card
    let _ = writeln!(
        md,
        "The fewest activities a citizen worked at in a day (§10: {FEWEST}, all-rounders {FEWEST_ALL_ROUNDER}); who took a detour, and the days worked with one (70%); what came of asking the citizens near it (K3.2); and whether its card's \"lately busy with\" says the making job it gave most time to (80%). A card names only what's made, so trading isn't held up against it.\n"
    );
    let cols: Vec<&str> = kept.iter().map(|(n, _)| *n).collect();
    let _ = writeln!(md, "| | {} |", cols.join(" | "));
    let _ = writeln!(md, "|---|{}", "---:|".repeat(cols.len()));
    let row = |md: &mut String, label: &str, f: &dyn Fn(&[CitizenCalling]) -> String| {
        let cells: Vec<String> = kept.iter().map(|(_, c)| f(c)).collect();
        let _ = writeln!(md, "| {label} | {} |", cells.join(" | "));
    };
    let fewest = |citizens: &[CitizenCalling], style: Option<&str>| {
        citizens
            .iter()
            .filter(|c| style.is_none_or(|s| c.style.as_deref() == Some(s)))
            .filter_map(CitizenCalling::fewest_activities)
            .min()
            .map_or("—".to_string(), |n| n.to_string())
    };
    row(md, "Fewest activities in a day", &|c| fewest(c, None));
    row(md, "Fewest activities in a day, all-rounders", &|c| fewest(c, Some("all-rounder")));
    row(md, "Citizens who took a detour", &|c| {
        of(c.iter().filter(|c| c.detour_days > 0).count(), c.iter().filter(|c| !c.time.days.is_empty()).count())
    });
    row(md, "Days worked with a detour", &|c| {
        of(c.iter().map(|c| c.detour_days).sum(), c.iter().map(|c| c.time.days.len()).sum())
    });
    let asks = |c: &[CitizenCalling]| {
        let mut all = Asks::default();
        for citizen in c {
            all += citizen.asks;
        }
        all
    };
    row(md, "Asked a citizen near it", &|c| asks(c).asked().to_string());
    row(md, "… and traded / declined / ended / failed", &|c| {
        let a = asks(c);
        format!("{} / {} / {} / {}", a.traded, a.declined, a.ended, a.failed)
    });
    row(md, "Had someone to ask, and let them be", &|c| asks(c).let_be.to_string());
    row(md, "Cards read (quiet)", &|c| {
        let read = c.iter().filter(|c| c.lately.is_some()).count();
        let quiet = c.iter().filter(|c| c.lately.as_ref().is_some_and(Vec::is_empty)).count();
        format!("{read} ({quiet})")
    });
    row(md, "\"Lately busy with\" leads with the most time", &|c| {
        let m: Vec<(bool, bool)> = c.iter().filter_map(CitizenCalling::lately_matches).collect();
        of(m.iter().filter(|(first, _)| *first).count(), m.len())
    });
    row(md, "… or names it among its three", &|c| {
        let m: Vec<(bool, bool)> = c.iter().filter_map(CitizenCalling::lately_matches).collect();
        of(m.iter().filter(|(_, any)| *any).count(), m.len())
    });
    md.push('\n');

    // Each citizen
    for (name, citizens) in &kept {
        let _ = writeln!(
            md,
            "<details><summary>{}each citizen's calling</summary>\n",
            if many { format!("{name}: ") } else { String::new() }
        );
        let _ = writeln!(md, "| Citizen | Style | Favourite | Of the work | Of the making jobs | Biggest job | In band | Fewest a day | Detour days | Most made | Lately |");
        let _ = writeln!(md, "|---|---|---|---:|---:|---:|---|---:|---:|---|---|");
        for c in citizens.iter() {
            let band = match c.in_band() {
                Some(true) => "yes",
                Some(false) => "no",
                None => "—",
            };
            let _ = writeln!(
                md,
                "| {} | {} | {} | {} | {} | {} | {band} | {} | {} of {} | {} | {} |",
                c.name,
                c.style.as_deref().unwrap_or("—"),
                c.favourite.as_deref().unwrap_or("—"),
                pct(c.of_work()),
                pct(c.of_making()),
                c.time.top_job().map_or("—".to_string(), |j| format!("{} {}", j.slug(), pct(c.biggest_job()))),
                c.fewest_activities().map_or("—".to_string(), |n| n.to_string()),
                c.detour_days,
                c.time.days.len(),
                c.most_made().map_or("—", Activity::slug),
                c.lately.as_ref().map_or("unread".to_string(), |l| if l.is_empty() { "quiet".into() } else { l.join(", ") }),
            );
        }
        let _ = writeln!(md, "\n</details>\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::report::DayCount;
    use std::collections::BTreeMap;

    fn citizen(name: &str, style: &str, favourite: Option<&str>, hours: &[(&str, f64)], fewest: usize) -> CitizenCalling {
        CitizenCalling {
            name: name.into(),
            style: Some(style.into()),
            favourite: favourite.map(String::from),
            time: ActivityTime {
                hours: hours.iter().map(|(a, h)| (a.to_string(), *h)).collect::<BTreeMap<_, _>>(),
                setup: 0.0,
                days: vec![DayCount { jobs: 2, activities: fewest + 2 }, DayCount { jobs: 2, activities: fewest }],
            },
            detour_days: 1,
            lately: None,
            asks: Asks::default(),
        }
    }

    #[test]
    fn a_favourites_share_is_held_to_its_styles_band() {
        // 7 of 10 hours on wood: 70% of the work; trading 2, so 7 of 8 of the making
        let wood = citizen("Leonard", "specialist", Some("gatherer-wood"), &[("wood", 7.0), ("trading", 2.0), ("planks", 1.0)], 4);
        assert_eq!(wood.of_work(), Some(0.7));
        assert_eq!(wood.of_jobs(), Some(0.7));
        assert_eq!(wood.of_making(), Some(0.875));
        assert_eq!(wood.in_band(), Some(true));
        // Free time isn't work; a commons hour is
        let lean = citizen("Doris", "leaner", Some("gatherer-ore"), &[("ore", 3.0), ("commons", 7.0), ("free", 5.0)], 3);
        assert_eq!((lean.of_work(), lean.of_jobs()), (Some(0.3), Some(1.0)));
        assert_eq!(lean.in_band(), Some(false), "under 35%");
        // An all-rounder's biggest job
        let all = citizen("Ada", "all-rounder", None, &[("fiber", 4.0), ("wood", 3.0), ("ore", 3.0)], 6);
        assert_eq!((all.of_work(), all.biggest_job()), (None, Some(0.4)));
        assert_eq!(all.in_band(), Some(false), "over 35%");
        // A trading favourite has no share of the making jobs
        let merchant = citizen("Mo", "specialist", Some("merchant"), &[("trading", 6.0), ("fiber", 4.0)], 2);
        assert_eq!((merchant.of_work(), merchant.of_making()), (Some(0.6), None));
    }

    #[test]
    fn the_card_is_read_as_the_jobs_it_names() {
        let catalog = Catalog::fallback();
        let card = Lately {
            doing: vec!["digging clay".into(), "felling trees".into(), "gathering sticks".into(), "sawing planks".into()],
            sells: vec![],
        };
        // Clay is an input: it says nothing. Sticks are wood's, as logs are
        assert_eq!(card_jobs(&card, &catalog), vec!["wood", "planks"]);
        let mut c = citizen("Leonard", "specialist", Some("gatherer-wood"), &[("wood", 2.0), ("planks", 5.0), ("trading", 9.0)], 3);
        assert_eq!(c.lately_matches(), None, "unread");
        c.lately = Some(card_jobs(&card, &catalog));
        assert_eq!(c.most_made(), Some(Activity::Planks), "trading isn't made");
        assert_eq!(c.lately_matches(), Some((false, true)));
        c.lately = Some(vec![]);
        assert_eq!(c.lately_matches(), Some((false, false)), "a quiet card");
    }

    #[test]
    fn one_run_or_two_say_their_measures() {
        let mut a = vec![
            citizen("Leonard", "specialist", Some("gatherer-wood"), &[("wood", 7.0), ("trading", 3.0)], 4),
            citizen("Doris", "leaner", Some("gatherer-ore"), &[("ore", 4.0), ("wood", 6.0)], 3),
            citizen("Ada", "all-rounder", None, &[("fiber", 3.0), ("wood", 3.0), ("ore", 4.0)], 6),
        ];
        a[0].asks = Asks { let_be: 4, traded: 2, declined: 0, ended: 1, failed: 0 };
        a[1].asks = Asks { traded: 1, ..Asks::default() };
        a[2].detour_days = 0;
        a[0].lately = Some(vec!["wood".into()]);
        a[1].lately = Some(vec!["ore".into(), "wood".into()]);
        let mut one = String::new();
        callings_md(&mut one, &[("A", &a)]);
        assert!(one.contains("| Style | Citizens | Of the work | Of the jobs | Of the making jobs | In band |"), "{one}");
        assert!(one.contains("| specialist | 1 | 70% | 70% | 100% | 1 of 1 (100%) |"), "{one}");
        assert!(one.contains("| leaner | 1 | 40% | 40% | 40% | 1 of 1 (100%) |"), "{one}");
        assert!(one.contains("| all-rounder | 1 | 40% (biggest job) | — | — | 0 of 1 (0%) |"), "{one}");
        assert!(one.contains("| Fewest activities in a day | 3 |"), "{one}");
        assert!(one.contains("| Fewest activities in a day, all-rounders | 6 |"), "{one}");
        assert!(one.contains("| Citizens who took a detour | 2 of 3 (67%) |"), "{one}");
        assert!(one.contains("| Days worked with a detour | 2 of 6 (33%) |"), "{one}");
        assert!(one.contains("| Asked a citizen near it | 4 |"), "{one}");
        assert!(one.contains("| … and traded / declined / ended / failed | 3 / 0 / 1 / 0 |"), "{one}");
        assert!(one.contains("| Had someone to ask, and let them be | 4 |"), "{one}");
        assert!(one.contains("| Cards read (quiet) | 2 (0) |"), "{one}");
        assert!(one.contains("| \"Lately busy with\" leads with the most time | 1 of 2 (50%) |"), "Doris made wood most: {one}");
        assert!(one.contains("| … or names it among its three | 2 of 2 (100%) |"), "{one}");
        assert!(one.contains("| Leonard | specialist | gatherer-wood | 70% | 100% | wood 70% | yes | 4 | 1 of 2 | wood | wood |"), "{one}");
        assert!(one.contains("| Ada | all-rounder | — | — | — | ore 40% | no | 6 | 0 of 2 | ore | unread |"), "{one}");

        let mut two = String::new();
        callings_md(&mut two, &[("A", &a), ("B", &[])]);
        assert!(two.contains("B is from before callings were kept"), "{two}");
        assert!(two.contains("| Style | Run | Citizens |"), "{two}");
        assert!(two.contains("| specialist | A | 1 | 70% |"), "{two}");
        let mut none = String::new();
        callings_md(&mut none, &[("A", &[]), ("B", &[])]);
        assert!(none.contains("every run is from before"), "{none}");
    }

    #[test]
    fn a_calling_line_round_trips_and_one_from_before_reads() {
        let mut c = citizen("Leonard", "specialist", Some("gatherer-wood"), &[("wood", 7.0)], 4);
        c.lately = Some(vec![]);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<CitizenCalling>(&json).unwrap(), c);
        let bare: CitizenCalling = serde_json::from_str(r#"{"name":"Ada"}"#).unwrap();
        assert_eq!((bare.style, bare.lately, bare.asks.asked()), (None, None, 0));
    }
}
