//! Balance runs (the Observatory's D6.3): the same town lived under two sets
//! of knobs, and what changed.
//!
//! - A [`Preset`] is a set of tunables in TOML, one knob a line:
//!   `property_tax_rate = 0.2`. A family's knob is keyed by its id, quoted
//!   (`"yield.grass" = 1.5`) or as a table (`[yield]` then `grass = 1.5`).
//!   A knob it doesn't name is at its default for the run.
//! - `capitol-town soak --tunables <preset>` sets the preset on the server
//!   before it reads the books, and writes a [`Balance`] beside its report:
//!   the knobs it ran under, the citizens' Strands by trade, the Gini, the
//!   faucets and sinks by type, prices against their targets, and whether
//!   the books balanced. Since #553 each trade's Strands in escrow too
//!   (bids, commissions and wage offers), so the fiber share and the Gini
//!   read both with it and without, and each citizen's calling and time for
//!   §10's measures ([`measures`](super::measures)).
//! - [`compare`] holds two of them up against each other, for
//!   `capitol-town compare` and `scripts/balance.ps1`.
//!
//! Pure, so it's tested without a server.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use capitol_protocol::ledger::{Ledger, StrandFlow, TRANSACTION_TYPES};
use capitol_protocol::metrics::{gini, top_tenth_share};
use capitol_protocol::tunables::{Knob, TunableError, TunablesResponse, TunablesUpdate, Unit};
use serde::{Deserialize, Serialize};

use super::measures::{self, CitizenCalling};
use super::report::TownReport;
use crate::sim::report::ActivityTime;
use crate::sim::ROLES;

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// A set of tunables to run under, by key, each rounded to its knob's step.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preset {
    pub values: BTreeMap<String, f64>,
}

impl Preset {
    /// Reads a preset's TOML. A key no knob has, a value that isn't a
    /// number or one out of its knob's range is an error, so a typo stops a
    /// run before it starts. A family's id is checked by the server.
    pub fn parse(text: &str) -> Result<Preset, String> {
        let table: toml::Table = text.parse().map_err(|e| format!("not a preset: {e}"))?;
        let mut values = BTreeMap::new();
        flatten(&table, "", &mut values)?;
        let mut preset = Preset::default();
        for (key, value) in values {
            let (knob, _) = Knob::find(&key).ok_or_else(|| format!("{key}: no knob has that key"))?;
            let value = knob.check(value).map_err(|e| match e {
                TunableError::OutOfRange { min, max } => {
                    format!("{key} = {value} is out of its range, {} to {}", knob.unit.format(min), knob.unit.format(max))
                }
                TunableError::NotANumber => format!("{key} = {value} isn't a number"),
                TunableError::UnknownKey => format!("{key}: no knob has that key"),
            })?;
            preset.values.insert(key, value);
        }
        Ok(preset)
    }

    /// The update that puts the server on this preset: its values, and
    /// every other knob that's off its default back on it, so a run is the
    /// preset's whatever the server held before.
    pub fn update(&self, now: &TunablesResponse) -> TunablesUpdate {
        let mut values = self.values.clone();
        for knob in &now.knobs {
            if knob.value != knob.default && !values.contains_key(&knob.key) {
                values.insert(knob.key.clone(), knob.default);
            }
        }
        TunablesUpdate { values }
    }
}

fn flatten(table: &toml::Table, prefix: &str, out: &mut BTreeMap<String, f64>) -> Result<(), String> {
    for (key, value) in table {
        let key = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        let number = match value {
            toml::Value::Table(inner) => {
                flatten(inner, &key, out)?;
                continue;
            }
            toml::Value::Integer(n) => *n as f64,
            toml::Value::Float(x) => *x,
            other => return Err(format!("{key}: {other} isn't a number")),
        };
        if out.insert(key.clone(), number).is_some() {
            return Err(format!("{key} is set twice"));
        }
    }
    Ok(())
}

/// Every knob off its default in `tunables`, by key: what a run ran under.
pub fn off_default(tunables: &TunablesResponse) -> BTreeMap<String, f64> {
    tunables
        .knobs
        .iter()
        .filter(|k| k.value != k.default)
        .map(|k| (k.key.clone(), k.value))
        .collect()
}

// ---------------------------------------------------------------------------
// A run's balance
// ---------------------------------------------------------------------------

/// What one balance run came to: a soak's report, cut to what a comparison
/// reads. `capitol-town soak` writes it as `town.soak-<when>.balance.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    /// What the run is called: its preset's file name, or "the server's
    /// knobs" for a soak with none
    pub preset: String,
    /// Every knob that was off its default, by key
    pub tunables: BTreeMap<String, f64>,
    pub citizens: usize,
    /// Hours of the day's clock it ran, and hours of ticks at `scale`
    pub day_hours: f64,
    pub tick_hours: f64,
    pub scale: f64,
    /// When the books were read at each end, Unix ms on the server's clock:
    /// which hours of the citizens' day the run lived
    pub started_ms: i64,
    pub ended_ms: i64,
    /// What the server's day clock added to those (#427): a soak that set
    /// its hour (`--start-at`). 0 on the real clock, and for a run before it
    #[serde(default)]
    pub day_offset_ms: i64,
    /// The citizens' Strands by trade, at each end
    pub trades: Vec<TradeMoney>,
    /// The Gini of the citizens' Strands at each end, and the share the
    /// richest tenth held at the end
    pub gini_start: Option<f64>,
    pub gini_end: Option<f64>,
    pub top_tenth_end: Option<f64>,
    /// The Gini at the end with each citizen's escrow counted as its own
    /// (#553). `None` in a file from before.
    #[serde(default)]
    pub gini_end_with_escrow: Option<f64>,
    /// Strands by transaction type over the run, as the ledger files them:
    /// faucets positive, sinks negative
    pub flows: BTreeMap<String, i64>,
    pub faucets: i64,
    /// Negative: what left
    pub sinks: i64,
    /// Deeds the citizens claimed
    pub deeds: usize,
    /// Each item that traded or has a target
    pub prices: Vec<Price>,
    /// The Strands' books: what they're off by, and whether they hold
    pub strand_drift: i64,
    pub strands_hold: bool,
    /// The units' books: whether they hold, whether a restart left them
    /// unchecked, and by how much each item drifted
    pub units_hold: bool,
    pub units_void: bool,
    pub unit_drift: Vec<(String, i64)>,
    /// Strands paid out of the court's escrow to citizens for commissions
    /// delivered, and for wage work, from their own entries (Callings
    /// K0.1): the books file both as `court_payout`. `None` in a file from
    /// before.
    #[serde(default)]
    pub commissions_paid: Option<i64>,
    #[serde(default)]
    pub wages_paid: Option<i64>,
    /// Each citizen's calling and time, for §10's measures (#553). Empty in
    /// a file from before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callings: Vec<CitizenCalling>,
}

/// A trade's Strands, all its citizens together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeMoney {
    /// Its bucket: the favourite job's slug (`gatherer-wood`) or
    /// `all-rounder` (Callings K3.3). A file from before named the role as
    /// `gatherer (wood)`, which reads as its slug, so the two compare.
    #[serde(deserialize_with = "bucket")]
    pub role: String,
    pub citizens: usize,
    pub start: i64,
    pub end: i64,
    /// Its citizens' Strands in escrow at each end, outside `start` and
    /// `end` (#553). `None` in a file from before.
    #[serde(default)]
    pub escrow_start: Option<i64>,
    #[serde(default)]
    pub escrow_end: Option<i64>,
    /// What its citizens' time went on (Callings K0.1). A file from before
    /// has none, and neither do the counts after it.
    #[serde(default)]
    pub time: ActivityTime,
    /// Steps that failed, and times a citizen found no way to its goal
    #[serde(default)]
    pub failed_steps: usize,
    #[serde(default)]
    pub stuck: usize,
}

/// A trade's name as a balance file has it, as its bucket: the role's
/// slug for a role's old name (`gatherer (wood)` before K3.3), anything
/// else as it was.
fn bucket<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let name = String::deserialize(d)?;
    Ok(ROLES
        .into_iter()
        .find(|role| role.to_string() == name)
        .map_or(name, |role| role.slug().to_string()))
}

/// An item's price on the market over a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub item: String,
    /// Units that changed hands
    pub traded: i64,
    /// Weighted by quantity
    pub mean: Option<f64>,
    /// Strands a unit the economy is balanced to (D2.3)
    pub target: Option<i64>,
}

impl Price {
    /// How far the mean is off its target, as a fraction of it: 0.25 is a
    /// quarter over
    pub fn off(&self) -> Option<f64> {
        match (self.mean, self.target) {
            (Some(mean), Some(target)) if target > 0 => Some(mean / target as f64 - 1.0),
            _ => None,
        }
    }
}

impl Balance {
    /// A soak's balance, from its report (`None` without a soak), the knobs
    /// it ran under, and each item's target price (the catalog's; the sim's
    /// D5 values for an item the catalog has none for).
    pub fn of(report: &TownReport, preset: &str, tunables: BTreeMap<String, f64>, targets: &BTreeMap<String, i64>) -> Option<Balance> {
        let soak = report.soak.as_ref()?;
        let books = soak.reconciliation();
        let citizens = |l: &Ledger| l.accounts.iter().filter(|a| a.citizen).map(|a| a.strands).collect::<Vec<_>>();
        let (start, end) = (citizens(&soak.start), citizens(&soak.end));
        let holding: Vec<i64> = soak.end.accounts.iter().filter(|a| a.citizen).map(|a| a.holding()).collect();
        let prices = report
            .sim
            .items
            .iter()
            .map(|line| Price {
                item: line.item.clone(),
                traded: line.traded,
                mean: line.price_mean,
                target: targets.get(&line.item).copied().or(line.d5_price),
            })
            .filter(|p| p.traded > 0 || p.target.is_some())
            .collect();
        Some(Balance {
            preset: preset.to_string(),
            tunables,
            citizens: report.citizens.len(),
            day_hours: report.day_hours,
            tick_hours: report.sim.hours,
            scale: report.sim.scale,
            started_ms: soak.start.at_ms,
            ended_ms: soak.end.at_ms,
            day_offset_ms: 0,
            trades: report
                .trades
                .iter()
                .map(|t| TradeMoney {
                    role: t.role.clone(),
                    citizens: t.citizens,
                    start: t.strands_start,
                    end: t.strands_end,
                    escrow_start: Some(t.escrow_start),
                    escrow_end: Some(t.escrow_end),
                    time: t.time.clone(),
                    failed_steps: t.failed_steps,
                    stuck: t.stuck,
                })
                .collect(),
            gini_start: gini(&start),
            gini_end: gini(&end),
            top_tenth_end: top_tenth_share(&end),
            gini_end_with_escrow: gini(&holding),
            flows: books.strands.flows.clone(),
            faucets: books.strands.faucets,
            sinks: books.strands.sinks,
            deeds: report.sim.deeds,
            prices,
            strand_drift: books.strands.drift,
            strands_hold: books.strands.holds(),
            units_hold: books.units.holds(),
            units_void: books.units.void,
            unit_drift: books.units.drift.clone(),
            commissions_paid: Some(report.sim.court.paid),
            wages_paid: Some(report.sim.wages.earned),
            callings: if report.citizens.iter().any(|c| c.calling.is_some()) { report.callings() } else { Vec::new() },
        })
    }

    /// Whether every Strand and unit was accounted for
    pub fn conserved(&self) -> bool {
        self.strands_hold && self.units_hold
    }

    fn books_word(&self) -> String {
        let strands = if self.strands_hold { "conserved".to_string() } else { format!("DRIFTED by {}", self.strand_drift) };
        let units = if self.units_void {
            "unchecked: the server restarted".to_string()
        } else if self.units_hold {
            "conserved".to_string()
        } else {
            let drift: Vec<String> = self.unit_drift.iter().map(|(i, n)| format!("{i} {n:+}")).collect();
            format!("DRIFTED in {}", drift.join(", "))
        };
        format!("Strands {strands}; units {units}")
    }

    /// The game clock at `unix_ms`: "21:45"
    fn clock(&self, unix_ms: i64) -> String {
        super::schedule::clock(capitol_protocol::rules::time_of_day(unix_ms + self.day_offset_ms))
    }

    /// Whether two runs lived the same hours of the citizens' day: their
    /// starts within a game half hour of each other on their day clocks, or
    /// both whole days. `balance.ps1` runs both towns at once, or starts
    /// both at the hour `-StartAt` gives (#427).
    pub fn same_hours(&self, other: &Balance) -> bool {
        if self.day_hours >= 24.0 && other.day_hours >= 24.0 {
            return true;
        }
        let day_ms = capitol_protocol::rules::DAY_LENGTH_SECS * 1000;
        let apart = (self.started_ms + self.day_offset_ms - other.started_ms - other.day_offset_ms).rem_euclid(day_ms);
        apart.min(day_ms - apart) <= day_ms / 48
    }

    fn strands(&self) -> (i64, i64) {
        self.trades.iter().fold((0, 0), |(s, e), t| (s + t.start, e + t.end))
    }

    fn flow(&self, kind: &str) -> i64 {
        self.flows.get(kind).copied().unwrap_or(0)
    }

    /// The faucets' Strands a citizen a game hour (§10 of the callings
    /// plan): what a citizen's work brings into the world.
    pub fn faucet_per_citizen_hour(&self) -> Option<f64> {
        let hours = self.citizens as f64 * self.tick_hours;
        (hours > 0.0).then(|| self.faucets as f64 / hours)
    }

    /// The Strands that changed hands for goods, commissions, wages and
    /// rent over the run (§10's circulation).
    pub fn circulation(&self) -> Circulation {
        // A sale is counted once, by what its seller was paid
        let goods = self.flow("market_sale") + self.flow("bazaar_sale") + self.flow("trade_received");
        let (commissions, wages) = match (self.commissions_paid, self.wages_paid) {
            (Some(c), Some(w)) => (c, Some(w)),
            // From before K0.1: the books' payouts, both together
            _ => (self.flow("court_payout"), None),
        };
        Circulation {
            goods,
            commissions,
            wages,
            rent: -self.flow("bazaar_rent"),
        }
    }

    /// The share of the citizens' Strands at the end held by the trades
    /// whose name says fiber (§10: under 30%, from 93% in #403).
    pub fn fiber_share(&self) -> Option<f64> {
        let (_, end) = self.strands();
        let fiber: i64 = self.trades.iter().filter(|t| t.role.contains("fiber")).map(|t| t.end).sum();
        (end > 0).then(|| fiber as f64 / end as f64)
    }

    /// The citizens' Strands in escrow at the end, all together: `None` in
    /// a file from before it was kept (#553)
    pub fn escrow_end(&self) -> Option<i64> {
        self.trades.iter().map(|t| t.escrow_end).sum()
    }

    /// [`fiber_share`](Self::fiber_share) with each trade's escrow counted
    /// as its own (#553): at 200 citizens K4.1 found half the Strands in
    /// open bids, which the share without it leaves out.
    pub fn fiber_share_with_escrow(&self) -> Option<f64> {
        self.escrow_end()?;
        let holding = |t: &TradeMoney| t.end + t.escrow_end.unwrap_or(0);
        let all: i64 = self.trades.iter().map(holding).sum();
        let fiber: i64 = self.trades.iter().filter(|t| t.role.contains("fiber")).map(holding).sum();
        (all > 0).then(|| fiber as f64 / all as f64)
    }

    /// Every trade's time together, and whether there is any: a run from
    /// before K0.1 has none.
    fn time(&self) -> Option<ActivityTime> {
        let mut all = ActivityTime::default();
        for t in &self.trades {
            all.add(&t.time);
        }
        (all.total() > 0.0).then_some(all)
    }
}

/// Strands that changed hands over a run (§10's circulation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Circulation {
    /// Paid to sellers: the hall's fills, the bazaar's sales and trade windows
    pub goods: i64,
    /// Paid out of the court's escrow for commissions delivered; with no
    /// split, for wage work too
    pub commissions: i64,
    /// Paid for wage work, when the run split it out
    pub wages: Option<i64>,
    /// Paid as bazaar rent
    pub rent: i64,
}

impl Circulation {
    pub fn total(&self) -> i64 {
        self.goods + self.commissions + self.wages.unwrap_or(0) + self.rent
    }
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

/// Two runs held up against each other, in Markdown: the knobs that differ,
/// whether the books balanced, the Strands by trade, the Gini, the faucets
/// and sinks, and prices against their targets. A plain sentence leads each
/// section, as the console's charts have one.
pub fn compare(a: &Balance, b: &Balance) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# A balance run: {} against {}\n", a.preset, b.preset);
    let _ = writeln!(
        md,
        "**A** is {}, **B** is {}. A had {} citizens from game {} to {}, {:.1} hours of the day's clock ({:.1} hours of ticks at {}×), and B had {} from {} to {}, {:.1} hours ({:.1} at {}×).\n",
        a.preset,
        b.preset,
        a.citizens,
        a.clock(a.started_ms),
        a.clock(a.ended_ms),
        a.day_hours,
        a.tick_hours,
        Unit::Ratio.format(a.scale),
        b.citizens,
        b.clock(b.started_ms),
        b.clock(b.ended_ms),
        b.day_hours,
        b.tick_hours,
        Unit::Ratio.format(b.scale)
    );
    if !a.same_hours(b) {
        let _ = writeln!(
            md,
            "**They lived different hours of the citizens' day**, so their work, market and night differ as well as their knobs. `scripts/balance.ps1` runs both at once to keep them together, or `-StartAt` starts both at one hour.\n"
        );
    }
    knobs_md(&mut md, a, b);
    books_md(&mut md, a, b);
    trades_md(&mut md, a, b);
    spread_md(&mut md, a, b);
    money_md(&mut md, a, b);
    flows_md(&mut md, a, b);
    prices_md(&mut md, a, b);
    time_md(&mut md, a, b);
    steps_md(&mut md, a, b);
    measures::callings_md(&mut md, &[("A", &a.callings), ("B", &b.callings)]);
    md
}

/// The money moving (Callings §10): the faucet a citizen-hour, what changed
/// hands a game hour, and fiber's share of the wealth.
fn money_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## The money moving\n");
    let (ca, cb) = (a.circulation(), b.circulation());
    let per_hour = |x: i64, run: &Balance| {
        if run.tick_hours > 0.0 {
            format!("{:.0}", x as f64 / run.tick_hours)
        } else {
            "—".into()
        }
    };
    let of_faucet = |c: &Circulation, run: &Balance| {
        if run.faucets > 0 {
            format!("{:.0}%", 100.0 * c.total() as f64 / run.faucets as f64)
        } else {
            "—".into()
        }
    };
    let _ = writeln!(
        md,
        "Strands spent on goods, commissions, wages and rent came to {} of the faucets' Strands in A and {} in B. A game hour is an hour of ticks.\n",
        of_faucet(&ca, a),
        of_faucet(&cb, b)
    );
    let num = |x: Option<f64>, digits: usize| x.map_or("—".to_string(), |x| format!("{x:.digits$}"));
    let pct = |x: Option<f64>| x.map_or("—".to_string(), |x| format!("{:.0}%", x * 100.0));
    let wages = |c: &Circulation, run: &Balance| c.wages.map_or("— (in commissions)".to_string(), |w| per_hour(w, run));
    let _ = writeln!(md, "| | A | B |");
    let _ = writeln!(md, "|---|---:|---:|");
    let _ = writeln!(md, "| Faucets a citizen-hour | {} | {} |", num(a.faucet_per_citizen_hour(), 1), num(b.faucet_per_citizen_hour(), 1));
    let _ = writeln!(md, "| Goods a game hour | {} | {} |", per_hour(ca.goods, a), per_hour(cb.goods, b));
    let _ = writeln!(md, "| Commissions a game hour | {} | {} |", per_hour(ca.commissions, a), per_hour(cb.commissions, b));
    let _ = writeln!(md, "| Wages a game hour | {} | {} |", wages(&ca, a), wages(&cb, b));
    let _ = writeln!(md, "| Rent a game hour | {} | {} |", per_hour(ca.rent, a), per_hour(cb.rent, b));
    let _ = writeln!(md, "| **Spent a game hour** | {} | {} |", per_hour(ca.total(), a), per_hour(cb.total(), b));
    let _ = writeln!(md, "| Spent, of the faucets | {} | {} |", of_faucet(&ca, a), of_faucet(&cb, b));
    let _ = writeln!(md, "| Fiber trades' share of the Strands at the end | {} | {} |", pct(a.fiber_share()), pct(b.fiber_share()));
    let _ = writeln!(
        md,
        "| … counting their escrow | {} | {} |",
        pct(a.fiber_share_with_escrow()),
        pct(b.fiber_share_with_escrow())
    );
    md.push('\n');
}

/// Time by activity (Callings K0.1): each trade's in each run.
fn time_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## Time by activity\n");
    let (ta, tb) = (a.time(), b.time());
    if ta.is_none() && tb.is_none() {
        let _ = writeln!(md, "Neither run logged its time by activity: both are from before it was kept.\n");
        return;
    }
    let mut times: Vec<&ActivityTime> = a.trades.iter().chain(&b.trades).map(|t| &t.time).collect();
    times.extend(ta.iter().chain(&tb));
    let columns = ActivityTime::columns(times);
    let _ = writeln!(
        md,
        "What each trade's work went on, as a share of its work time, in each run (the town report says how each is counted).{}\n",
        match (&ta, &tb) {
            (None, _) => " A is from before time by activity was kept.",
            (_, None) => " B is from before time by activity was kept.",
            _ => "",
        }
    );
    let heads = ActivityTime::heads(&columns);
    let _ = writeln!(md, "| Trade | Run | Work hours | {} |", heads.join(" | "));
    let _ = writeln!(md, "|---|---|{}", "---:|".repeat(heads.len() + 1));
    let row = |md: &mut String, trade: &str, run: &str, time: &ActivityTime| {
        let _ = writeln!(md, "| {trade} | {run} | {:.1} | {} |", time.work(), time.cells(&columns).join(" | "));
    };
    let roles: BTreeSet<&String> = a.trades.iter().chain(&b.trades).map(|t| &t.role).collect();
    for role in roles {
        for (name, run) in [("A", a), ("B", b)] {
            if let Some(t) = run.trades.iter().find(|t| &t.role == role).filter(|t| t.time.total() > 0.0) {
                row(md, role, name, &t.time);
            }
        }
    }
    for (name, time) in [("A", &ta), ("B", &tb)] {
        if let Some(time) = time {
            row(md, "**Every citizen**", name, time);
        }
    }
    md.push('\n');
}

/// Steps that failed, and times a citizen was stuck.
fn steps_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## Failed and stuck\n");
    if a.time().is_none() && b.time().is_none() {
        let _ = writeln!(md, "Neither run counted them: both are from before they were kept.\n");
        return;
    }
    let count = |run: &Balance, n: fn(&TradeMoney) -> usize| -> String {
        if run.time().is_none() {
            return "—".into();
        }
        let n: usize = run.trades.iter().map(n).sum();
        let hours = run.citizens as f64 * run.tick_hours;
        if hours > 0.0 {
            format!("{n} ({:.2} a citizen-hour)", n as f64 / hours)
        } else {
            n.to_string()
        }
    };
    let _ = writeln!(md, "| | A | B |");
    let _ = writeln!(md, "|---|---:|---:|");
    let _ = writeln!(md, "| Steps that failed | {} | {} |", count(a, |t| t.failed_steps), count(b, |t| t.failed_steps));
    let _ = writeln!(md, "| Times stuck, with no way to a goal | {} | {} |", count(a, |t| t.stuck), count(b, |t| t.stuck));
    md.push('\n');
}

fn knobs_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## The knobs\n");
    let keys: BTreeSet<&String> = a.tunables.keys().chain(b.tunables.keys()).collect();
    let differ: Vec<&&String> = keys.iter().filter(|k| a.tunables.get(**k) != b.tunables.get(**k)).collect();
    if keys.is_empty() {
        let _ = writeln!(md, "Both ran on every knob's default.\n");
        return;
    }
    if differ.is_empty() {
        let _ = writeln!(md, "Both ran on the same knobs, so any difference below is the town's own variance.\n");
    } else {
        let names: Vec<String> = differ.iter().map(|k| format!("`{k}`")).collect();
        let _ = writeln!(md, "They differ in {}. Every knob off its default in either:\n", names.join(", "));
    }
    let _ = writeln!(md, "| Knob | A | B |");
    let _ = writeln!(md, "|---|---:|---:|");
    for key in keys {
        let _ = writeln!(md, "| `{key}` | {} | {} |", knob_value(key, a.tunables.get(key)), knob_value(key, b.tunables.get(key)));
    }
    md.push('\n');
}

/// A knob's value as the console says it, "(default)" when the run left it
fn knob_value(key: &str, value: Option<&f64>) -> String {
    let Some((knob, _)) = Knob::find(key) else {
        return value.map_or("default".into(), |v| v.to_string());
    };
    match value {
        Some(v) => knob.unit.format(*v),
        None => format!("{} (default)", knob.unit.format(knob.default)),
    }
}

fn books_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## The books\n");
    let sentence = match (a.conserved(), b.conserved()) {
        (true, true) => "Both runs' books balance: every Strand and unit is accounted for.",
        (false, true) => "**A's books drifted**, so its numbers below can't be trusted; B's balance.",
        (true, false) => "**B's books drifted**, so its numbers below can't be trusted; A's balance.",
        (false, false) => "**Both runs' books drifted**, so neither's numbers below can be trusted.",
    };
    let _ = writeln!(md, "{sentence}\n");
    let _ = writeln!(md, "| | A | B |");
    let _ = writeln!(md, "|---|---|---|");
    let _ = writeln!(md, "| Books | {} | {} |", a.books_word(), b.books_word());
    let _ = writeln!(md, "| Deeds claimed | {} | {} |", a.deeds, b.deeds);
    md.push('\n');
}

fn trades_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## Strands by trade\n");
    let ((a0, a1), (b0, b1)) = (a.strands(), b.strands());
    let escrow = |run: &Balance| {
        run.escrow_end()
            .map_or(String::new(), |e| format!(", with {e} more in escrow at the end"))
    };
    let _ = writeln!(
        md,
        "The citizens' Strands went from {a0} to {a1} in A ({:+}{}) and from {b0} to {b1} in B ({:+}{}). Escrow is what a trade's citizens have put up in open bids, commissions and wage offers: still theirs, but outside their balances.\n",
        a1 - a0,
        escrow(a),
        b1 - b0,
        escrow(b)
    );
    let _ = writeln!(
        md,
        "| Trade | Citizens | A: start → end | A: change each | A: in escrow at the end | B: start → end | B: change each | B: in escrow at the end |"
    );
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|");
    let roles: BTreeSet<&String> = a.trades.iter().chain(&b.trades).map(|t| &t.role).collect();
    for role in roles {
        let (ta, tb) = (a.trades.iter().find(|t| &t.role == role), b.trades.iter().find(|t| &t.role == role));
        let citizens = match (ta, tb) {
            (Some(x), Some(y)) if x.citizens != y.citizens => format!("{} / {}", x.citizens, y.citizens),
            _ => ta.or(tb).map_or(0, |t| t.citizens).to_string(),
        };
        let cells = |t: Option<&TradeMoney>| match t {
            Some(t) => (
                format!("{} → {}", t.start, t.end),
                format!("{:+.0}", (t.end - t.start) as f64 / t.citizens.max(1) as f64),
                t.escrow_end.map_or("—".to_string(), |e| e.to_string()),
            ),
            None => ("—".into(), "—".into(), "—".into()),
        };
        let ((a_range, a_each, a_held), (b_range, b_each, b_held)) = (cells(ta), cells(tb));
        let _ = writeln!(md, "| {role} | {citizens} | {a_range} | {a_each} | {a_held} | {b_range} | {b_each} | {b_held} |");
    }
    md.push('\n');
}

fn spread_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## The spread\n");
    let g = |x: Option<f64>| x.map_or("—".to_string(), |x| format!("{x:.2}"));
    let share = |x: Option<f64>| x.map_or("—".to_string(), |x| format!("{:.0}%", x * 100.0));
    let _ = writeln!(
        md,
        "The Gini of the citizens' Strands ended at {} in A and {} in B: 0 is everyone holding the same, 1 is one citizen holding it all.\n",
        g(a.gini_end),
        g(b.gini_end)
    );
    let _ = writeln!(md, "| | A | B |");
    let _ = writeln!(md, "|---|---:|---:|");
    let _ = writeln!(md, "| Gini at the start | {} | {} |", g(a.gini_start), g(b.gini_start));
    let _ = writeln!(md, "| Gini at the end | {} | {} |", g(a.gini_end), g(b.gini_end));
    let _ = writeln!(
        md,
        "| Gini at the end, counting escrow | {} | {} |",
        g(a.gini_end_with_escrow),
        g(b.gini_end_with_escrow)
    );
    let _ = writeln!(md, "| The richest tenth's share at the end | {} | {} |", share(a.top_tenth_end), share(b.top_tenth_end));
    md.push('\n');
}

fn flows_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## Faucets and sinks\n");
    let tax = |x: &Balance| -(x.flows.get("property_tax").copied().unwrap_or(0) + x.flows.get("tax_payment").copied().unwrap_or(0));
    let more = match (-b.sinks).cmp(&-a.sinks) {
        std::cmp::Ordering::Greater => format!("{} more than", b.sinks.abs_diff(a.sinks)),
        std::cmp::Ordering::Less => format!("{} fewer than", b.sinks.abs_diff(a.sinks)),
        std::cmp::Ordering::Equal => "the same as".to_string(),
    };
    let _ = writeln!(
        md,
        "B's sinks took {} Strands, {more} A's {}; property tax took {} in B and {} in A. Faucets brought in {} in A and {} in B.\n",
        -b.sinks,
        -a.sinks,
        tax(b),
        tax(a),
        a.faucets,
        b.faucets
    );
    let _ = writeln!(md, "| Flow | Kind | A | B | B − A |");
    let _ = writeln!(md, "|---|---|---:|---:|---:|");
    // The ledger's order, faucets then sinks, then any type it doesn't know
    let mut types: Vec<&str> = TRANSACTION_TYPES.to_vec();
    for key in a.flows.keys().chain(b.flows.keys()) {
        if !types.contains(&key.as_str()) {
            types.push(key);
        }
    }
    for kind in types {
        let flow = StrandFlow::of(kind);
        if !matches!(flow, StrandFlow::Faucet | StrandFlow::Sink | StrandFlow::Other) {
            continue;
        }
        let (x, y) = (a.flows.get(kind).copied().unwrap_or(0), b.flows.get(kind).copied().unwrap_or(0));
        if x == 0 && y == 0 {
            continue;
        }
        let word = match flow {
            StrandFlow::Faucet => "faucet",
            StrandFlow::Sink => "sink",
            _ => "unknown",
        };
        let _ = writeln!(md, "| `{kind}` | {word} | {x} | {y} | {:+} |", y - x);
    }
    let _ = writeln!(md, "| **Faucets** | | {} | {} | {:+} |", a.faucets, b.faucets, b.faucets - a.faucets);
    let _ = writeln!(md, "| **Sinks** | | {} | {} | {:+} |", a.sinks, b.sinks, b.sinks - a.sinks);
    let _ = writeln!(
        md,
        "| **Net** | | {:+} | {:+} | {:+} |",
        a.faucets + a.sinks,
        b.faucets + b.sinks,
        (b.faucets + b.sinks) - (a.faucets + a.sinks)
    );
    let _ = writeln!(md, "\nSinks are negative, as the ledger files them. Transfers and escrow move Strands between players, so they're left out.\n");
}

fn prices_md(md: &mut String, a: &Balance, b: &Balance) {
    let _ = writeln!(md, "## Prices against target\n");
    let items: BTreeSet<&String> = a.prices.iter().chain(&b.prices).filter(|p| p.traded > 0).map(|p| &p.item).collect();
    if items.is_empty() {
        let _ = writeln!(md, "Nothing traded on the market in either run.");
        return;
    }
    let off = |run: &Balance| {
        let offs: Vec<f64> = run.prices.iter().filter_map(Price::off).map(f64::abs).collect();
        (!offs.is_empty()).then(|| offs.iter().sum::<f64>() / offs.len() as f64)
    };
    let pct = |x: Option<f64>| x.map_or("—".to_string(), |x| format!("{:.0}%", x * 100.0));
    let _ = writeln!(
        md,
        "Each item's mean price on the market, weighted by quantity, against its target. On average a traded item was {} off its target in A and {} in B.\n",
        pct(off(a)),
        pct(off(b))
    );
    let _ = writeln!(md, "| Item | Target | A: traded | A: mean | A: off | B: traded | B: mean | B: off |");
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|");
    for item in items {
        let (pa, pb) = (a.prices.iter().find(|p| &p.item == item), b.prices.iter().find(|p| &p.item == item));
        let target = pa.or(pb).and_then(|p| p.target).map_or("—".to_string(), |t| t.to_string());
        let cells = |p: Option<&Price>| match p.filter(|p| p.traded > 0) {
            Some(p) => (
                p.traded.to_string(),
                p.mean.map_or("—".to_string(), |m| format!("{m:.1}")),
                p.off().map_or("—".to_string(), |o| format!("{:+.0}%", o * 100.0)),
            ),
            None => ("0".into(), "—".into(), "—".into()),
        };
        let ((at, am, ao), (bt, bm, bo)) = (cells(pa), cells(pb));
        let _ = writeln!(md, "| {item} | {target} | {at} | {am} | {ao} | {bt} | {bm} | {bo} |");
    }
    md.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::tunables::TunableValue;

    #[test]
    fn a_preset_reads_flat_quoted_and_table_keys() {
        let preset = Preset::parse(
            r#"
            # A tax ten times today's
            property_tax_rate = 0.2
            stall_rent_per_day = 30
            "deed_price.urban" = 2.0
            [yield]
            grass = 1.5
            "#,
        )
        .unwrap();
        let want: BTreeMap<String, f64> = [
            ("property_tax_rate", 0.2),
            ("stall_rent_per_day", 30.0),
            ("deed_price.urban", 2.0),
            ("yield.grass", 1.5),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        assert_eq!(preset.values, want);
        // A dotted bare key is a table to TOML, and reads the same
        assert_eq!(Preset::parse("yield.grass = 1.5").unwrap().values["yield.grass"], 1.5);
        // Rounded to the knob's step
        assert_eq!(Preset::parse("property_tax_rate = 0.07000001").unwrap().values["property_tax_rate"], 0.07);
    }

    #[test]
    fn a_bad_preset_is_refused_before_the_run() {
        assert!(Preset::parse("propertytax = 0.2").unwrap_err().contains("no knob"));
        assert!(Preset::parse("property_tax_rate = 0.9").unwrap_err().contains("out of its range, 0% to 50%"));
        assert!(Preset::parse("property_tax_rate = \"high\"").unwrap_err().contains("isn't a number"));
        assert!(Preset::parse("yield = 2").unwrap_err().contains("no knob"), "a family needs its id");
        assert!(Preset::parse("\"yield.grass\" = 1\n[yield]\ngrass = 2").is_err(), "the same knob twice");
        assert!(Preset::parse("property_tax_rate = ").is_err());
        assert_eq!(Preset::parse("").unwrap(), Preset::default());
    }

    fn knob(key: &str, default: f64, value: f64) -> TunableValue {
        TunableValue {
            key: key.into(),
            label: key.into(),
            unit: Unit::Multiplier,
            min: 0.0,
            max: 10.0,
            step: 0.01,
            default,
            value,
            chart: "sinks".into(),
            base_strands: None,
        }
    }

    #[test]
    fn a_preset_puts_back_what_it_leaves_out() {
        let now = TunablesResponse {
            knobs: vec![
                knob("property_tax_rate", 0.02, 0.05),
                knob("yield.grass", 1.0, 3.0),
                knob("yield.clay", 1.0, 1.0),
                knob("node_regen_rate", 1.0, 2.0),
            ],
        };
        let preset = Preset::parse("property_tax_rate = 0.2\nnode_regen_rate = 2").unwrap();
        let update = preset.update(&now);
        assert_eq!(update.values.len(), 3, "{:?}", update.values);
        assert_eq!(update.values["property_tax_rate"], 0.2);
        assert_eq!(update.values["node_regen_rate"], 2.0);
        assert_eq!(update.values["yield.grass"], 1.0, "back to its default");
        assert!(!update.values.contains_key("yield.clay"), "already on its default");
        assert_eq!(
            off_default(&now).into_iter().collect::<Vec<_>>(),
            vec![("node_regen_rate".to_string(), 2.0), ("property_tax_rate".to_string(), 0.05), ("yield.grass".to_string(), 3.0)]
        );
    }

    fn run(preset: &str, tax: f64, property_tax: i64) -> Balance {
        let mut flows = BTreeMap::new();
        flows.insert("exchange_deposit".to_string(), 900);
        flows.insert("deed_purchase".to_string(), -500);
        flows.insert("market_fee".to_string(), -12);
        flows.insert("property_tax".to_string(), -property_tax);
        flows.insert("market_escrow".to_string(), -300);
        let sinks = -512 - property_tax;
        Balance {
            preset: preset.into(),
            tunables: if tax == 0.02 { BTreeMap::new() } else { [("property_tax_rate".to_string(), tax)].into() },
            citizens: 20,
            day_hours: 2.0,
            tick_hours: 20.0,
            scale: 10.0,
            started_ms: 0,
            ended_ms: 600_000,
            day_offset_ms: 0,
            trades: vec![money("gatherer-fiber", 4, 400, 1200), money("crafter-planks", 2, 200, 150)],
            gini_start: Some(0.0),
            gini_end: Some(0.41),
            top_tenth_end: Some(0.3),
            gini_end_with_escrow: None,
            flows,
            faucets: 900,
            sinks,
            deeds: 2,
            prices: vec![
                Price { item: "plank".into(), traded: 40, mean: Some(15.0), target: Some(12) },
                Price { item: "ingot".into(), traded: 0, mean: None, target: Some(80) },
            ],
            strand_drift: 0,
            strands_hold: true,
            units_hold: true,
            units_void: false,
            unit_drift: vec![],
            commissions_paid: None,
            wages_paid: None,
            callings: Vec::new(),
        }
    }

    fn money(role: &str, citizens: usize, start: i64, end: i64) -> TradeMoney {
        TradeMoney {
            role: role.into(),
            citizens,
            start,
            end,
            escrow_start: None,
            escrow_end: None,
            time: ActivityTime::default(),
            failed_steps: 0,
            stuck: 0,
        }
    }

    /// Callings K0.1: the money moving, time by activity, and what failed,
    /// and a file from before them still reads
    #[test]
    fn two_runs_compare_their_money_moving_and_their_time() {
        use crate::sim::report::Entry;
        use crate::sim::Activity;
        let spent = |hours: f64, activity: Activity| Entry::Spent {
            label: "a step".into(),
            hours,
            ok: true,
            activity,
            setup: false,
            day: 3,
        };
        let a = run("before.toml", 0.02, 1);
        let mut b = run("after.toml", 0.02, 1);
        b.flows.insert("market_sale".into(), 300);
        b.flows.insert("bazaar_rent".into(), -40);
        b.flows.insert("court_payout".into(), 100);
        (b.commissions_paid, b.wages_paid) = (Some(80), Some(20));
        let entries = |fiber: f64, commons: f64| vec![spent(fiber, Activity::Fiber), spent(commons, Activity::Commons)];
        b.trades[0].time = ActivityTime::of(&entries(3.0, 1.0));
        b.trades[0].failed_steps = 8;
        b.trades[1].time = ActivityTime::of(&entries(1.0, 1.0));
        b.trades[1].stuck = 4;
        let md = compare(&a, &b);
        assert!(md.contains("| Faucets a citizen-hour | 2.2 | 2.2 |"), "900 over 20 citizens for 20 hours: {md}");
        assert!(md.contains("| Goods a game hour | 0 | 15 |"), "{md}");
        assert!(md.contains("| Commissions a game hour | 0 | 4 |"), "{md}");
        assert!(md.contains("| Wages a game hour | — (in commissions) | 1 |"), "{md}");
        assert!(md.contains("| Rent a game hour | 0 | 2 |"), "{md}");
        assert!(md.contains("| Spent, of the faucets | 0% | 49% |"), "440 of 900: {md}");
        assert!(md.contains("| Fiber trades' share of the Strands at the end | 89% | 89% |"), "{md}");
        assert!(md.contains("A is from before time by activity was kept."), "{md}");
        assert!(md.contains("| Trade | Run | Work hours | Fiber | Commons | Setup | Jobs a day (fewest) | Activities a day (fewest) |"), "{md}");
        assert!(md.contains("| gatherer-fiber | B | 4.0 | 75% | 25% |  | 1.0 (1) | 2.0 (2) |"), "{md}");
        assert!(md.contains("| **Every citizen** | B | 6.0 | 67% | 33% |  | 1.0 (1) | 2.0 (2) |"), "{md}");
        assert!(md.contains("| Steps that failed | — | 8 (0.02 a citizen-hour) |"), "{md}");
        assert!(md.contains("| Times stuck, with no way to a goal | — | 4 (0.01 a citizen-hour) |"), "{md}");

        // A balance written before K0.1 has none of it, and reads
        let mut old = serde_json::to_value(&a).unwrap();
        let fields = old.as_object_mut().unwrap();
        fields.remove("commissions_paid");
        fields.remove("wages_paid");
        for trade in fields["trades"].as_array_mut().unwrap() {
            let trade = trade.as_object_mut().unwrap();
            for key in ["time", "failed_steps", "stuck"] {
                trade.remove(key);
            }
        }
        let read: Balance = serde_json::from_value(old).unwrap();
        assert_eq!(read, a);
        assert!(compare(&read, &read).contains("Neither run logged its time by activity"));
    }

    #[test]
    fn two_runs_compare_knob_by_knob_and_sink_by_sink() {
        let (a, b) = (run("tax-low.toml", 0.02, 1), run("tax-high.toml", 0.2, 10));
        let md = compare(&a, &b);
        assert!(md.starts_with("# A balance run: tax-low.toml against tax-high.toml"), "{md}");
        assert!(md.contains("They differ in `property_tax_rate`"), "{md}");
        assert!(md.contains("| `property_tax_rate` | 2% (default) | 20% |"), "{md}");
        assert!(md.contains("Both runs' books balance"), "{md}");
        assert!(md.contains("B's sinks took 522 Strands, 9 more than A's 513; property tax took 10 in B and 1 in A."), "{md}");
        assert!(md.contains("| `property_tax` | sink | -1 | -10 | -9 |"), "{md}");
        assert!(!md.contains("market_escrow"), "escrow isn't a sink: {md}");
        assert!(md.contains("| gatherer-fiber | 4 | 400 → 1200 | +200 | — | 400 → 1200 | +200 | — |"), "{md}");
        assert!(md.contains("| plank | 12 | 40 | 15.0 | +25% |"), "{md}");
        assert!(!md.contains("| ingot |"), "an item that never traded has no price: {md}");
        assert!(md.contains("Gini of the citizens' Strands ended at 0.41 in A and 0.41 in B"), "{md}");
    }

    #[test]
    fn drift_is_said_first() {
        let a = run("a", 0.02, 1);
        let mut b = run("b", 0.02, 1);
        b.strands_hold = false;
        b.strand_drift = -3;
        let md = compare(&a, &b);
        assert!(md.contains("**B's books drifted**"), "{md}");
        assert!(md.contains("Strands DRIFTED by -3"), "{md}");
        assert!(md.contains("Both ran on every knob's default"), "{md}");
        assert!(!b.conserved());
    }

    #[test]
    fn runs_at_different_hours_are_called_out() {
        let a = run("a", 0.02, 1);
        let mut b = run("b", 0.2, 10);
        let md = compare(&a, &b);
        assert!(md.contains("A had 20 citizens from game 00:00 to 02:00, 2.0 hours"), "{md}");
        assert!(!md.contains("different hours"), "{md}");
        // Half a game day later: B's citizens are at the market while A's sleep
        b.started_ms += 3_600_000;
        b.ended_ms += 3_600_000;
        assert!(compare(&a, &b).contains("**They lived different hours of the citizens' day**"));
        // A game minute short of midnight is next to just past it
        b.started_ms = 7_200_000 - 60_000;
        assert!(a.same_hours(&b));
        // Half an hour apart on the real clock, but each set to start at 09:00 (#427)
        let mut a = a;
        b.started_ms = a.started_ms + 1_800_000;
        b.ended_ms = b.started_ms + 600_000;
        a.day_offset_ms = capitol_protocol::rules::day_offset_to(9.0, a.started_ms);
        b.day_offset_ms = capitol_protocol::rules::day_offset_to(9.0, b.started_ms);
        assert!(a.same_hours(&b));
        let md = compare(&a, &b);
        assert!(md.contains("from game 09:00 to 11:00") && !md.contains("different hours"), "{md}");
    }

    #[test]
    fn a_balance_round_trips_as_json() {
        let a = run("a", 0.2, 10);
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Balance>(&json).unwrap(), a);
    }

    /// Before Callings K3.3 a balance named its trades as `gatherer
    /// (fiber)`: they read as their slugs, so an old run compares with a
    /// new one trade by trade.
    #[test]
    fn a_balance_naming_trades_the_old_way_reads_as_their_slugs() {
        let a = run("a", 0.2, 10);
        let json = serde_json::to_string(&a)
            .unwrap()
            .replace("\"gatherer-fiber\"", "\"gatherer (fiber)\"")
            .replace("\"crafter-planks\"", "\"crafter (planks)\"");
        assert!(json.contains("gatherer (fiber)"));
        assert_eq!(serde_json::from_str::<Balance>(&json).unwrap(), a);
        let mut b = a.clone();
        b.trades.push(money("all-rounder", 3, 300, 330));
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Balance>(&json).unwrap().trades[2].role, "all-rounder");
    }
}
