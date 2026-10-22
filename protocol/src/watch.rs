//! Watches (the Observatory's D2.5): rules that warn.
//!
//! Each sample, the backend's sampler hands [`Watches::observe`] a
//! [`Reading`]: what the books, the balances, the market, the tax and the
//! tick said over that sample's window. Five rules judge it:
//!
//! - **drift**: Strands or units the books don't explain, since the server
//!   started. Any is too much.
//! - **gini**: wealth pooling, the Gini of people's and citizens' Strands over
//!   a threshold.
//! - **off target**: an item's price over the last game day further from its
//!   target than a fraction of it, either way. One watch per item.
//! - **tax revenue**: the property tax an hour, over the last few hourly
//!   runs, under a floor while there are deeds to tax (#335).
//! - **tick p95**: the tick's p95 over its budget.
//!
//! The thresholds are tunables ([`WatchThresholds`]). A watch fires on its
//! edge: once when its rule goes over, not every sample it stays over. It
//! clears only after [`CLEAR_AFTER`] samples back within, so a rule hovering
//! at its threshold doesn't fire again every other sample. A rule that can't
//! judge a sample (no ticks, no trades, not a full game day watched yet)
//! leaves its watch as it was. The backend writes each firing as a
//! `watch_fired` annotation, and each sample lists the watches firing as it
//! was taken (`Sample::watches`).

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::ledger::{StrandBooks, UnitBooks};
use crate::metrics::{gini, Drift, Sample, Trades};
use crate::rules::DAY_LENGTH_SECS;
use crate::tunables::WatchThresholds;

/// Samples a watch must be back within its threshold before it clears: five
/// minutes of them
pub const CLEAR_AFTER: u32 = 30;
/// Fewer accounts than this and the Gini says nothing
pub const GINI_MIN_ACCOUNTS: usize = 10;
/// The hourly tax runs the tax watch averages over
pub const TAX_RUNS: usize = 3;
/// How long a price is watched over: a game day
pub const PRICE_WINDOW_MS: i64 = DAY_LENGTH_SECS * 1000;

/// What a watch watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WatchKind {
    /// Strands or units the books don't explain
    Drift,
    /// Wealth pooling
    Gini,
    /// An item's price off its target
    OffTarget,
    /// The property tax an hour under its floor
    TaxRevenue,
    /// The tick's p95 over budget
    TickP95,
}

/// One watch: a rule, and for a rule kept per item, the item.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Watch {
    pub kind: WatchKind,
    /// The item type, for [`WatchKind::OffTarget`]
    pub item: Option<String>,
}

impl Watch {
    pub fn of(kind: WatchKind) -> Watch {
        Watch { kind, item: None }
    }

    pub fn item(kind: WatchKind, item: &str) -> Watch {
        Watch { kind, item: Some(item.to_string()) }
    }
}

/// A watch that just fired, with a plain sentence for its annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct Fired {
    pub watch: Watch,
    pub text: String,
}

/// What one sample's window said, for the rules.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    /// When the sample was taken (Unix ms)
    pub at_ms: i64,
    /// Strands the books didn't explain over the window: the world's drift,
    /// escrow's and unpaired transfers, or the accounts the trail doesn't
    /// explain, whichever is more. One write with no trail shows in both, so
    /// it's counted once.
    pub strand_drift: i64,
    /// Units each item's flows didn't explain since the last ledger read in
    /// a quiet moment; `None` when this one wasn't, so units can't be judged
    pub unit_drift: Option<BTreeMap<String, i64>>,
    /// People's and citizens' Strands, not admins'
    pub balances: Vec<i64>,
    /// Property tax collected over the window, back tax paid included
    pub tax: i64,
    /// Hourly tax runs that finished in the window
    pub tax_runs: u64,
    /// Deeds held now
    pub deeds: u64,
    /// The market's fills over the window, by item type
    pub market: BTreeMap<String, Trades>,
    pub ticks: u64,
    pub tick_period_ms: f64,
    /// The bucket bound the window's p95 falls under; `None` past the last
    pub tick_p95_ms: Option<f64>,
}

impl Reading {
    /// The reading of `sample`, with the books over its window (`strands`),
    /// the units since the last settled ledger when this one is settled
    /// (`units`), the balances of people and citizens, and the deeds held.
    pub fn of(sample: &Sample, strands: &StrandBooks, units: Option<&UnitBooks>, balances: Vec<i64>, deeds: u64) -> Reading {
        let world = strands.drift.abs() + strands.escrow_drift.abs() + strands.transfers.abs();
        let accounts = strands.accounts.iter().chain(&strands.gone).map(|(_, n)| n.abs()).sum::<i64>();
        let strand_drift = world.max(accounts);
        let unit_drift = units.filter(|u| !u.void).map(|u| u.drift.iter().cloned().collect());
        let taxed = |kind: &str| -sample.strand_flows.get(kind).copied().unwrap_or(0);
        Reading {
            at_ms: sample.at_ms,
            strand_drift,
            unit_drift,
            balances,
            tax: taxed("property_tax") + taxed("tax_payment"),
            tax_runs: sample.load.loops.get("tax").map_or(0, |l| l.runs),
            deeds,
            market: sample.market.clone(),
            ticks: sample.load.ticks,
            tick_period_ms: sample.load.tick_period_ms,
            tick_p95_ms: sample.load.tick_p95_ms,
        }
    }
}

/// A rule's verdict on one reading.
enum Verdict {
    /// Over its threshold, and what to say if that's news
    Over(String),
    Within,
    /// Nothing to judge by
    Unknown,
}

#[derive(Debug, Clone, Copy, Default)]
struct State {
    firing: bool,
    /// Samples within since it last went over
    within: u32,
}

/// Every watch's state, and the windows the rules judge over. One per
/// server, fed every sample.
#[derive(Debug, Default)]
pub struct Watches {
    /// When the first reading was taken
    began_ms: Option<i64>,
    states: BTreeMap<Watch, State>,
    /// Strands the books haven't explained since the first reading
    strand_drift: i64,
    /// Units each item's flows haven't explained since then
    unit_drift: BTreeMap<String, i64>,
    /// Tax collected since the last run finished, for the next
    tax_pending: i64,
    /// What the last few runs collected, oldest first
    tax_runs: VecDeque<i64>,
    /// The market's fills over the last game day, by sample, oldest first
    trades: VecDeque<(i64, BTreeMap<String, Trades>)>,
}

impl Watches {
    pub fn new() -> Watches {
        Watches::default()
    }

    /// Judge `reading` by `thresholds`, with each item's target price in
    /// Strands. Returns the watches that went over with it.
    pub fn observe(&mut self, reading: &Reading, thresholds: &WatchThresholds, targets: &BTreeMap<String, f64>) -> Vec<Fired> {
        let began = *self.began_ms.get_or_insert(reading.at_ms);
        let mut verdicts = vec![
            (Watch::of(WatchKind::Drift), self.judge_drift(reading)),
            (Watch::of(WatchKind::Gini), gini_verdict(reading, thresholds.gini)),
            (Watch::of(WatchKind::TaxRevenue), self.tax(reading, thresholds.tax_per_hour)),
            (Watch::of(WatchKind::TickP95), tick_verdict(reading, thresholds.tick_p95)),
        ];
        verdicts.extend(self.prices(reading, began, thresholds.off_target, targets));
        let mut fired = Vec::new();
        for (watch, verdict) in verdicts {
            let state = self.states.entry(watch.clone()).or_default();
            match verdict {
                Verdict::Over(text) => {
                    state.within = 0;
                    if !state.firing {
                        state.firing = true;
                        fired.push(Fired { watch, text });
                    }
                }
                Verdict::Within => {
                    state.within = state.within.saturating_add(1);
                    if state.within >= CLEAR_AFTER {
                        state.firing = false;
                    }
                }
                Verdict::Unknown => {}
            }
        }
        fired
    }

    /// The watches firing now, in order
    pub fn firing(&self) -> Vec<Watch> {
        self.states.iter().filter(|(_, s)| s.firing).map(|(w, _)| w.clone()).collect()
    }

    /// What the books haven't explained since the first reading
    pub fn drift(&self) -> Drift {
        Drift { strands: self.strand_drift, units: self.unit_drift.clone() }
    }

    fn judge_drift(&mut self, reading: &Reading) -> Verdict {
        self.strand_drift += reading.strand_drift;
        for (item, n) in reading.unit_drift.iter().flatten() {
            *self.unit_drift.entry(item.clone()).or_insert(0) += n.abs();
        }
        self.unit_drift.retain(|_, n| *n != 0);
        if self.strand_drift == 0 && self.unit_drift.is_empty() {
            return Verdict::Within;
        }
        let mut parts = Vec::new();
        if self.strand_drift != 0 {
            parts.push(format!("{} Strands the audit trail doesn't explain", self.strand_drift));
        }
        let mut items: Vec<(&String, &i64)> = self.unit_drift.iter().collect();
        items.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (item, n) in items.iter().take(3) {
            parts.push(format!("{n} {item} its flows don't"));
        }
        if items.len() > 3 {
            parts.push(format!("{} more items", items.len() - 3));
        }
        Verdict::Over(format!("The books drifted: {}", join(&parts)))
    }

    fn tax(&mut self, reading: &Reading, floor: f64) -> Verdict {
        self.tax_pending += reading.tax;
        if reading.tax_runs > 0 {
            // What came in since the last run, shared among those that
            // finished in this window, the remainder to the first
            let runs = reading.tax_runs as i64;
            let each = self.tax_pending / runs;
            for i in 0..runs {
                self.tax_runs.push_back(if i == 0 { each + self.tax_pending % runs } else { each });
            }
            while self.tax_runs.len() > TAX_RUNS {
                self.tax_runs.pop_front();
            }
            self.tax_pending = 0;
        }
        if self.tax_runs.is_empty() {
            return Verdict::Unknown;
        }
        if reading.deeds == 0 {
            return Verdict::Within;
        }
        let per_hour = self.tax_runs.iter().sum::<i64>() as f64 / self.tax_runs.len() as f64;
        if per_hour >= floor {
            return Verdict::Within;
        }
        let runs = self.tax_runs.len();
        Verdict::Over(format!(
            "Property tax isn't biting: {} an hour from {} {} over the last {}, under {}",
            strands(per_hour),
            reading.deeds,
            if reading.deeds == 1 { "deed" } else { "deeds" },
            if runs == 1 { "run".to_string() } else { format!("{runs} runs") },
            strands(floor),
        ))
    }

    fn prices(&mut self, reading: &Reading, began: i64, off: f64, targets: &BTreeMap<String, f64>) -> Vec<(Watch, Verdict)> {
        self.trades.push_back((reading.at_ms, reading.market.clone()));
        let from = reading.at_ms - PRICE_WINDOW_MS;
        while self.trades.front().is_some_and(|(at, _)| *at <= from) {
            self.trades.pop_front();
        }
        // Not a whole game day watched yet: a morning's prices aren't the day's
        let whole_day = began <= from;
        let mut day: BTreeMap<&str, Trades> = BTreeMap::new();
        for (item, t) in self.trades.iter().flat_map(|(_, m)| m) {
            let sum = day.entry(item.as_str()).or_default();
            sum.trades += t.trades;
            sum.units += t.units;
            sum.strands += t.strands;
        }
        targets
            .iter()
            .filter(|(_, target)| **target > 0.0)
            .map(|(item, target)| {
                let watch = Watch::item(WatchKind::OffTarget, item);
                let Some(vwap) = day.get(item.as_str()).and_then(Trades::vwap).filter(|_| whole_day) else {
                    return (watch, Verdict::Unknown);
                };
                let by = vwap / target - 1.0;
                if by.abs() <= off {
                    return (watch, Verdict::Within);
                }
                let text = format!(
                    "{} is trading off target: {} a unit over the game day, against {} ({}{:.0}%)",
                    capitalised(item),
                    strands(vwap),
                    strands(*target),
                    if by > 0.0 { "+" } else { "" },
                    by * 100.0,
                );
                (watch, Verdict::Over(text))
            })
            .collect()
    }
}

fn gini_verdict(reading: &Reading, over: f64) -> Verdict {
    if reading.balances.len() < GINI_MIN_ACCOUNTS {
        return Verdict::Unknown;
    }
    match gini(&reading.balances) {
        Some(g) if g > over => Verdict::Over(format!(
            "Wealth is pooling: the Gini of {} accounts is {g:.2}, over {over:.2}",
            reading.balances.len()
        )),
        Some(_) => Verdict::Within,
        None => Verdict::Unknown,
    }
}

fn tick_verdict(reading: &Reading, over_ticks: f64) -> Verdict {
    if reading.ticks == 0 || reading.tick_period_ms <= 0.0 {
        return Verdict::Unknown;
    }
    let budget = reading.tick_period_ms * over_ticks;
    let took = match reading.tick_p95_ms {
        Some(p95) if p95 <= budget => return Verdict::Within,
        Some(p95) => format!("up to {} ms", trim(p95)),
        None => "over a second".to_string(),
    };
    Verdict::Over(format!(
        "The tick is over budget: 95% of {} ticks took {took}, against {} ms",
        reading.ticks,
        trim(budget)
    ))
}

/// "12 Strands", "1 Strand", "0.4 Strands"
fn strands(n: f64) -> String {
    let n = trim(n);
    if n == "1" { "1 Strand".to_string() } else { format!("{n} Strands") }
}

/// A number to one place, with no trailing zero
fn trim(n: f64) -> String {
    let s = format!("{n:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

fn capitalised(id: &str) -> String {
    let name = id.replace('_', " ");
    let mut chars = name.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// "a", "a and b", "a, b and c"
fn join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MS: i64 = 10_000;

    /// A calm reading at sample `n`: nothing drifted, wealth even, the tick
    /// in budget, no tax run
    fn calm(n: i64) -> Reading {
        Reading {
            at_ms: n * SAMPLE_MS,
            unit_drift: Some(BTreeMap::new()),
            balances: vec![100; 20],
            deeds: 5,
            ticks: 100,
            tick_period_ms: 100.0,
            tick_p95_ms: Some(1.0),
            ..Reading::default()
        }
    }

    fn kinds(fired: &[Fired]) -> Vec<WatchKind> {
        fired.iter().map(|f| f.watch.kind).collect()
    }

    fn none() -> BTreeMap<String, f64> {
        BTreeMap::new()
    }

    /// Feeds `readings` in, and says what each fired
    fn run(watches: &mut Watches, readings: impl IntoIterator<Item = Reading>, targets: &BTreeMap<String, f64>) -> Vec<Vec<WatchKind>> {
        let t = WatchThresholds::default();
        readings.into_iter().map(|r| kinds(&watches.observe(&r, &t, targets))).collect()
    }

    #[test]
    fn a_calm_world_fires_nothing() {
        let mut w = Watches::new();
        assert!(run(&mut w, (0..100).map(calm), &none()).iter().all(Vec::is_empty));
        assert!(w.firing().is_empty());
    }

    /// The Verify: a tick overrun that lasts several samples fires once
    #[test]
    fn a_tick_overrun_fires_once_not_every_sample() {
        let mut w = Watches::new();
        let over = |n| Reading { tick_p95_ms: Some(250.0), ..calm(n) };
        let fired = run(&mut w, (0..3).map(calm).chain((3..13).map(over)), &none());
        assert_eq!(fired.iter().flatten().count(), 1, "{fired:?}");
        assert_eq!(fired[3], [WatchKind::TickP95]);
        assert_eq!(w.firing(), [Watch::of(WatchKind::TickP95)]);

        // Back in budget, it clears only after CLEAR_AFTER samples; an
        // overrun before then is the same one
        let back = 13 + CLEAR_AFTER as i64 - 1;
        assert!(run(&mut w, (13..back).map(calm), &none()).iter().all(Vec::is_empty));
        assert_eq!(w.firing(), [Watch::of(WatchKind::TickP95)], "not clear yet");
        assert!(run(&mut w, [over(back)], &none())[0].is_empty(), "still the same overrun");
        let clear = back + 1 + CLEAR_AFTER as i64;
        run(&mut w, (back + 1..clear).map(calm), &none());
        assert!(w.firing().is_empty());
        assert_eq!(run(&mut w, [over(clear)], &none())[0], [WatchKind::TickP95], "a new overrun fires again");
    }

    #[test]
    fn the_tick_is_judged_against_its_period_and_past_the_last_bucket() {
        let t = WatchThresholds::default();
        let mut w = Watches::new();
        // At 10× the period is 10 ms: a p95 in the 20 ms bucket is over
        let fast = Reading { tick_period_ms: 10.0, tick_p95_ms: Some(20.0), ..calm(0) };
        let fired = w.observe(&fast, &t, &none());
        assert_eq!(fired[0].text, "The tick is over budget: 95% of 100 ticks took up to 20 ms, against 10 ms");
        let mut w = Watches::new();
        assert!(w.observe(&Reading { tick_p95_ms: Some(100.0), ..calm(0) }, &t, &none()).is_empty(), "the 100 ms bucket is in budget");
        let fired = w.observe(&Reading { tick_p95_ms: None, ..calm(1) }, &t, &none());
        assert!(fired[0].text.contains("took over a second"), "{}", fired[0].text);
        // No ticks: nothing to judge, so nothing clears
        let mut w = Watches::new();
        w.observe(&Reading { tick_p95_ms: Some(250.0), ..calm(0) }, &t, &none());
        for n in 1..=CLEAR_AFTER as i64 {
            w.observe(&Reading { ticks: 0, ..calm(n) }, &t, &none());
        }
        assert_eq!(w.firing(), [Watch::of(WatchKind::TickP95)]);
        // A looser threshold lets it through
        let loose = WatchThresholds { tick_p95: 3.0, ..t };
        assert!(Watches::new().observe(&Reading { tick_p95_ms: Some(250.0), ..calm(0) }, &loose, &none()).is_empty());
    }

    #[test]
    fn drift_fires_once_and_stays_until_the_books_explain_it() {
        let mut w = Watches::new();
        let strands = |n| Reading { strand_drift: 5, ..calm(n) };
        let fired = run(&mut w, [calm(0), strands(1), strands(2), calm(3)], &none());
        assert_eq!(fired, [vec![], vec![WatchKind::Drift], vec![], vec![]]);
        // It's since the server started, so calm samples after don't clear it
        run(&mut w, (4..4 + 2 * CLEAR_AFTER as i64).map(calm), &none());
        assert_eq!(w.firing(), [Watch::of(WatchKind::Drift)]);

        let t = WatchThresholds::default();
        let mut w = Watches::new();
        let units = Reading {
            strand_drift: 3,
            unit_drift: Some(BTreeMap::from([("stone".to_string(), -2), ("log".to_string(), 1)])),
            ..calm(0)
        };
        let fired = w.observe(&units, &t, &none());
        assert_eq!(fired[0].text, "The books drifted: 3 Strands the audit trail doesn't explain, 2 stone its flows don't and 1 log its flows don't");
        // The samples carry the running count, for the console's drift tile
        let drift = w.drift();
        assert_eq!((drift.strands, &drift.units), (3, &BTreeMap::from([("log".to_string(), 1), ("stone".to_string(), 2)])));
        assert!(!drift.is_zero());
        assert!(Watches::new().drift().is_zero());
        // Units not read in a quiet moment can't be judged, and don't clear anything
        let mut w = Watches::new();
        assert!(w.observe(&Reading { unit_drift: None, ..calm(0) }, &t, &none()).is_empty());
    }

    #[test]
    fn the_gini_fires_over_its_threshold_with_enough_accounts() {
        let t = WatchThresholds::default();
        let pooled = |n| {
            let mut balances = vec![10; 19];
            balances.push(5000);
            Reading { balances, ..calm(n) }
        };
        let mut w = Watches::new();
        let fired = w.observe(&pooled(0), &t, &none());
        assert_eq!(fired[0].watch, Watch::of(WatchKind::Gini));
        assert_eq!(fired[0].text, "Wealth is pooling: the Gini of 20 accounts is 0.91, over 0.70");
        assert!(w.observe(&pooled(1), &t, &none()).is_empty(), "on the edge, not every sample");
        // Too few accounts to say
        let few = Reading { balances: vec![0, 0, 0, 500], ..calm(0) };
        assert!(Watches::new().observe(&few, &t, &none()).is_empty());
        // A threshold of 1 never fires
        let never = WatchThresholds { gini: 1.0, ..t };
        assert!(Watches::new().observe(&pooled(0), &never, &none()).is_empty());
    }

    #[test]
    fn tax_is_judged_per_run_once_one_has_finished() {
        let t = WatchThresholds { tax_per_hour: 10.0, ..WatchThresholds::default() };
        let mut w = Watches::new();
        let paid = |n, tax, runs| Reading { tax, tax_runs: runs, ..calm(n) };
        // No run yet: nothing to judge, however little comes in
        assert!(w.observe(&paid(0, 0, 0), &t, &none()).is_empty());
        // Back tax before the run, then the run's own, a sample apart from
        // its finishing: all of it is the run's
        w.observe(&paid(1, 4, 0), &t, &none());
        w.observe(&paid(2, 20, 0), &t, &none());
        assert!(w.observe(&paid(3, 0, 1), &t, &none()).is_empty(), "24 an hour");
        // Two lean hours bring the mean of three to (24 + 2 + 1) / 3 = 9
        assert!(w.observe(&paid(4, 2, 1), &t, &none()).is_empty(), "13 an hour");
        let fired = w.observe(&paid(5, 1, 1), &t, &none());
        assert_eq!(fired[0].text, "Property tax isn't biting: 9 Strands an hour from 5 deeds over the last 3 runs, under 10 Strands");
        // #335: a Strand in a day
        let mut w = Watches::new();
        let fired = w.observe(&Reading { deeds: 102, ..paid(0, 0, 1) }, &WatchThresholds::default(), &none());
        assert_eq!(fired[0].text, "Property tax isn't biting: 0 Strands an hour from 102 deeds over the last run, under 1 Strand");
        // With no deeds there's nothing to tax
        assert!(Watches::new().observe(&Reading { deeds: 0, ..paid(0, 0, 1) }, &t, &none()).is_empty());
        // A floor of 0 never fires
        let never = WatchThresholds { tax_per_hour: 0.0, ..t };
        assert!(Watches::new().observe(&paid(0, 0, 1), &never, &none()).is_empty());
    }

    #[test]
    fn a_price_off_target_over_a_whole_game_day_fires_for_its_item() {
        let t = WatchThresholds::default();
        let targets = BTreeMap::from([("rope".to_string(), 12.0), ("log".to_string(), 5.0)]);
        let day = PRICE_WINDOW_MS / SAMPLE_MS;
        let selling = |n: i64, rope_price: i64| {
            let market = BTreeMap::from([
                ("rope".to_string(), Trades { trades: 1, units: 2, strands: 2 * rope_price }),
                ("log".to_string(), Trades { trades: 1, units: 10, strands: 50 }),
            ]);
            Reading { market, ..calm(n) }
        };
        let mut w = Watches::new();
        // Off target from the start, but a morning isn't a game day
        let fired = run(&mut w, (0..day).map(|n| selling(n, 31)), &targets);
        assert!(fired.iter().all(Vec::is_empty));
        // A whole day watched, the rope fires, the logs are on target
        let fired = w.observe(&selling(day, 31), &t, &targets);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].watch, Watch::item(WatchKind::OffTarget, "rope"));
        assert_eq!(fired[0].text, "Rope is trading off target: 31 Strands a unit over the game day, against 12 Strands (+158%)");
        assert!(w.observe(&selling(day + 1, 31), &t, &targets).is_empty());
        // An item nobody traded in a day can't be judged
        let mut w = Watches::new();
        let fired = run(&mut w, (0..=day).map(calm), &targets);
        assert!(fired.iter().all(Vec::is_empty));
        // Under target fires too, and with no targets nothing is watched
        let mut w = Watches::new();
        let fired = run(&mut w, (0..=day).map(|n| selling(n, 4)), &targets);
        assert_eq!(fired.last().unwrap(), &[WatchKind::OffTarget]);
        let mut w = Watches::new();
        assert!(run(&mut w, (0..=day).map(|n| selling(n, 31)), &none()).iter().all(Vec::is_empty));
    }

    #[test]
    fn a_reading_is_made_from_a_sample_and_its_books() {
        use crate::ledger::{reconcile, AccountBooks, Ledger, Load, PhaseWindow};
        use std::collections::HashSet;
        use uuid::Uuid;
        let ledger = |at_ms, strands: i64, flows: &[(&str, i64)], stone| Ledger {
            at_ms,
            booted_ms: 1,
            accounts: vec![AccountBooks {
                player_id: Uuid::from_u128(1),
                name: "ada".into(),
                citizen: false,
                strands,
                flows: flows.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            }],
            escrow_strands: 0,
            units: BTreeMap::from([("stone".to_string(), stone)]),
            unit_flows: BTreeMap::new(),
            units_settled: true,
            crafts: BTreeMap::new(),
            load: Load::default(),
        };
        // Ada paid 3 in tax, and 2 Strands and a stone came from nowhere
        let a = ledger(0, 100, &[], 4);
        let b = ledger(10_000, 99, &[("property_tax", -3)], 5);
        let mut sample = Sample::between(&a, &b, &HashSet::new());
        sample.load.loops.insert("tax".into(), PhaseWindow { runs: 1, secs: 0.2, mean_ms: 200.0, p95_ms: Some(250.0), slow_statements: 0 });
        let books = reconcile(&a, &b);
        let reading = Reading::of(&sample, &books.strands, Some(&books.units), vec![99], 1);
        assert_eq!(reading.tax, 3);
        assert_eq!(reading.tax_runs, 1);
        assert_eq!(reading.strand_drift, 2, "the world's 2 are Ada's 2");
        assert_eq!(reading.unit_drift, Some(BTreeMap::from([("stone".to_string(), 1)])));
    }

    #[test]
    fn a_watch_is_sent_by_its_kind_and_item() {
        let json = serde_json::to_string(&[Watch::of(WatchKind::TickP95), Watch::item(WatchKind::OffTarget, "rope")]).unwrap();
        assert_eq!(json, r#"[{"kind":"tick_p95","item":null},{"kind":"off_target","item":"rope"}]"#);
    }
}
