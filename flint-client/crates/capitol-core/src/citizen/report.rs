//! The town's report (C1.7): what its citizens did, from their
//! [`Book`](super::town::Book)s, and for a soak, what the server's books
//! say about it.
//!
//! - **Trades** and **citizens**: for each role and each citizen, its wealth
//!   at each end, what it made, what it sold and bought, its deeds, and what
//!   held it up.
//! - **How questions were settled**: how often each layer of the cascade
//!   answered (D4).
//! - **Conservation**, in a soak: the world's Strands and units at each end,
//!   held up against each other (`capitol_protocol::ledger::reconcile`).
//!   Drift fails it.
//! - **Load**, in a soak: tick time against its budget, sockets, and the
//!   database, from a ledger read every minute.
//! - The sim's report's sections, over every citizen as one swarm
//!   ([`Report::sections`]): bottlenecks, production and prices, failed
//!   pulls, tools worn out, where the time went, and why steps failed.
//!
//! Pure, so it's tested without a server.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use capitol_protocol::ledger::{reconcile, Ledger, LoadWindow, Reconciliation};

use crate::sim::report::{BotRun, Entry, Report};

/// What a soak read from the server: the ledger at each end, and the ledger
/// every minute between, for its load.
#[derive(Debug, Clone)]
pub struct Soak {
    pub start: Ledger,
    pub end: Ledger,
    /// Every reading, the two ends among them, in order
    pub samples: Vec<Ledger>,
}

impl Soak {
    pub fn reconciliation(&self) -> Reconciliation {
        reconcile(&self.start, &self.end)
    }

    /// What the tax took over the soak, from the server's books: the hourly
    /// tax, and any owed tax paid off.
    pub fn tax(&self) -> i64 {
        let flows = self.reconciliation().strands.flows;
        -["property_tax", "tax_payment"].iter().filter_map(|kind| flows.get(*kind)).sum::<i64>()
    }

    /// The load between each reading and the next.
    pub fn windows(&self) -> Vec<(i64, LoadWindow)> {
        self.samples
            .windows(2)
            .map(|w| (w[1].at_ms, LoadWindow::between(&w[0].load, &w[1].load, secs(&w[0], &w[1]))))
            .collect()
    }

    /// The load over the whole soak.
    pub fn whole(&self) -> LoadWindow {
        LoadWindow::between(&self.start.load, &self.end.load, secs(&self.start, &self.end))
    }

    /// The readings a soak kept, one ledger a line in order (`capitol-town
    /// soak` writes them as it reads them): from the first to the last.
    /// `None` with fewer than two.
    pub fn from_jsonl(text: &str) -> Result<Option<Soak>, String> {
        let samples: Vec<Ledger> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("reading {} of the books: {e}", i + 1)))
            .collect::<Result<_, _>>()?;
        Ok(match (samples.first(), samples.last()) {
            (Some(start), Some(end)) if samples.len() >= 2 => Some(Soak {
                start: start.clone(),
                end: end.clone(),
                samples,
            }),
            _ => None,
        })
    }

    /// The conservation and load sections alone: what the books say,
    /// without the citizens (`capitol-town books`, for a soak that didn't
    /// get to write its report).
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        let _ = writeln!(
            md,
            "# The town's books\n\n{} readings over {:.1} wall minutes.\n",
            self.samples.len(),
            secs(&self.start, &self.end) / 60.0
        );
        conservation(&mut md, &self.reconciliation());
        load(&mut md, self);
        md
    }
}

fn secs(a: &Ledger, b: &Ledger) -> f64 {
    (b.at_ms - a.at_ms) as f64 / 1000.0
}

/// One citizen's line.
#[derive(Debug, Clone, PartialEq)]
pub struct CitizenLine {
    pub name: String,
    pub role: String,
    pub strands_start: i64,
    pub strands_end: i64,
    /// Units gathered and crafted
    pub made: i64,
    /// What it made most of, and how many
    pub most: Option<(String, i64)>,
    /// Units sold and bought on the market, and the Strands each way
    pub sold: i64,
    pub sold_for: i64,
    pub bought: i64,
    pub bought_for: i64,
    pub deeds: usize,
    pub failed_steps: usize,
    /// Times it found no way to its goal, and the commonest reason
    pub stuck: usize,
    pub why_stuck: Option<String>,
}

impl CitizenLine {
    fn from_run(run: &BotRun) -> CitizenLine {
        let mut made: BTreeMap<&str, i64> = BTreeMap::new();
        let mut stuck: BTreeMap<&str, usize> = BTreeMap::new();
        let mut line = CitizenLine {
            name: run.name.clone(),
            role: run.role.clone(),
            strands_start: run.strands_start,
            strands_end: run.strands_end,
            made: 0,
            most: None,
            sold: 0,
            sold_for: 0,
            bought: 0,
            bought_for: 0,
            deeds: 0,
            failed_steps: 0,
            stuck: 0,
            why_stuck: None,
        };
        for entry in &run.entries {
            match entry {
                Entry::Made { item, quantity, .. } => *made.entry(item).or_insert(0) += *quantity as i64,
                Entry::Traded {
                    sold, quantity, price, ..
                } => {
                    let (n, strands) = if *sold {
                        (&mut line.sold, &mut line.sold_for)
                    } else {
                        (&mut line.bought, &mut line.bought_for)
                    };
                    *n += *quantity as i64;
                    *strands += *quantity as i64 * price;
                }
                Entry::Claimed { .. } => line.deeds += 1,
                Entry::Spent { ok: false, .. } => line.failed_steps += 1,
                Entry::Stuck { reason } => *stuck.entry(reason).or_insert(0) += 1,
                _ => {}
            }
        }
        line.made = made.values().sum();
        line.most = made
            .iter()
            .max_by_key(|(item, n)| (**n, std::cmp::Reverse(**item)))
            .map(|(item, n)| (item.to_string(), *n));
        line.stuck = stuck.values().sum();
        line.why_stuck = stuck
            .iter()
            .max_by_key(|(reason, n)| (**n, std::cmp::Reverse(**reason)))
            .map(|(reason, _)| reason.to_string());
        line
    }
}

/// One trade's line: its citizens taken together.
#[derive(Debug, Clone, PartialEq)]
pub struct TradeLine {
    pub role: String,
    pub citizens: usize,
    pub strands_start: i64,
    pub strands_end: i64,
    pub made: i64,
    pub sold: i64,
    pub sold_for: i64,
    pub bought: i64,
    pub bought_for: i64,
    pub deeds: usize,
    pub stuck: usize,
}

#[derive(Debug, Clone)]
pub struct TownReport {
    pub citizens: Vec<CitizenLine>,
    pub trades: Vec<TradeLine>,
    /// How often questions were settled each way, every citizen together
    pub rulings: BTreeMap<String, u64>,
    /// The sim's report over every citizen
    pub sim: Report,
    pub soak: Option<Soak>,
    /// Hours of the day's clock the town ran (a game day is 24)
    pub day_hours: f64,
    pub in_person: InPerson,
}

/// Trades made in a trade window rather than at the hall (C1.5, #295).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InPerson {
    /// With people
    pub with_people: usize,
    /// Between two citizens, each counted once, from the one that asked
    pub between_citizens: usize,
    /// The Strands that changed hands between citizens
    pub strands: i64,
}

impl InPerson {
    fn count(runs: &[(BotRun, BTreeMap<String, u64>)]) -> InPerson {
        let mut n = InPerson::default();
        for entry in runs.iter().flat_map(|(run, _)| &run.entries) {
            match entry {
                Entry::Dealt { citizen: false, .. } => n.with_people += 1,
                Entry::Dealt {
                    citizen: true,
                    asked: true,
                    strands,
                    ..
                } => {
                    n.between_citizens += 1;
                    n.strands += strands.abs();
                }
                _ => {}
            }
        }
        n
    }
}

impl TownReport {
    /// Builds the report from each citizen's run and rulings
    /// ([`Town::runs`](super::town::Town::runs)), over `wall_secs` at the
    /// server's `scale`. A soak's citizens' Strands at each end are the
    /// server's, not what the town last saw.
    pub fn build(runs: &[(BotRun, BTreeMap<String, u64>)], wall_secs: f64, scale: f64, soak: Option<Soak>) -> TownReport {
        let mut rulings = BTreeMap::new();
        let mut citizens = Vec::with_capacity(runs.len());
        for (run, by) in runs {
            for (label, n) in by {
                *rulings.entry(label.clone()).or_insert(0) += n;
            }
            let mut line = CitizenLine::from_run(run);
            if let Some(soak) = &soak {
                let strands = |l: &Ledger| l.accounts.iter().find(|a| a.name == run.name).map(|a| a.strands);
                line.strands_start = strands(&soak.start).unwrap_or(line.strands_start);
                line.strands_end = strands(&soak.end).unwrap_or(line.strands_end);
            }
            citizens.push(line);
        }
        let mut trades: BTreeMap<&str, TradeLine> = BTreeMap::new();
        for c in &citizens {
            let t = trades.entry(&c.role).or_insert_with(|| TradeLine {
                role: c.role.clone(),
                citizens: 0,
                strands_start: 0,
                strands_end: 0,
                made: 0,
                sold: 0,
                sold_for: 0,
                bought: 0,
                bought_for: 0,
                deeds: 0,
                stuck: 0,
            });
            t.citizens += 1;
            t.strands_start += c.strands_start;
            t.strands_end += c.strands_end;
            t.made += c.made;
            t.sold += c.sold;
            t.sold_for += c.sold_for;
            t.bought += c.bought;
            t.bought_for += c.bought_for;
            t.deeds += c.deeds;
            t.stuck += c.stuck;
        }
        let trades = trades.into_values().collect();
        let tick_hours = wall_secs * scale / 3600.0;
        let bot_runs: Vec<BotRun> = runs.iter().map(|(run, _)| run.clone()).collect();
        let mut sim = Report::build(&bot_runs, tick_hours, scale, wall_secs);
        // A citizen's residue isn't its tax: escrow, fills and rent are in
        // it too (#334). Only the books measure it.
        sim.tax = soak.as_ref().map(Soak::tax);
        TownReport {
            citizens,
            trades,
            rulings,
            sim,
            soak,
            day_hours: wall_secs / capitol_protocol::rules::DAY_LENGTH_SECS as f64 * 24.0,
            in_person: InPerson::count(runs),
        }
    }

    /// Whether a soak's books balanced. A report with no soak has nothing
    /// to say, and passes.
    pub fn conserved(&self) -> bool {
        self.soak.as_ref().is_none_or(|s| s.reconciliation().holds())
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        let roles: Vec<String> = self.sim.roles.iter().map(|(r, n)| format!("{n} {r}")).collect();
        let _ = writeln!(md, "# The town\n");
        let _ = writeln!(
            md,
            "{} citizens ({}) for {:.1} hours of the day's clock, {:.1} hours of ticks at {}× ({:.1} wall minutes).\n",
            self.citizens.len(),
            roles.join(", "),
            self.day_hours,
            self.sim.hours,
            self.sim.scale,
            self.sim.wall_secs / 60.0
        );
        if let Some(soak) = &self.soak {
            conservation(&mut md, &soak.reconciliation());
            load(&mut md, soak);
        }
        self.trades_md(&mut md);
        let p = &self.in_person;
        let _ = writeln!(
            md,
            "In trade windows: {} trades with people, {} between citizens (each asked by one), with {} Strands changing hands between citizens.
",
            p.with_people, p.between_citizens, p.strands
        );
        self.citizens_md(&mut md);
        self.rulings_md(&mut md);
        md.push('\n');
        md.push_str(&self.sim.sections());
        md
    }

    fn trades_md(&self, md: &mut String) {
        let _ = writeln!(md, "## Trades\n");
        let _ = writeln!(
            md,
            "| Trade | Citizens | Strands at the start | At the end | Change each | Made | Sold (Strands) | Bought (Strands) | Deeds | Stuck |"
        );
        let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
        for t in &self.trades {
            let each = (t.strands_end - t.strands_start) as f64 / t.citizens.max(1) as f64;
            let _ = writeln!(
                md,
                "| {} | {} | {} | {} | {:+.0} | {} | {} ({}) | {} ({}) | {} | {} |",
                t.role, t.citizens, t.strands_start, t.strands_end, each, t.made, t.sold, t.sold_for, t.bought, t.bought_for, t.deeds, t.stuck
            );
        }
        md.push('\n');
    }

    fn citizens_md(&self, md: &mut String) {
        let _ = writeln!(md, "## Citizens\n");
        let _ = writeln!(
            md,
            "| Citizen | Trade | Strands | Made (most) | Sold (Strands) | Bought (Strands) | Deeds | Failed steps | Stuck (why, most often) |"
        );
        let _ = writeln!(md, "|---|---|---:|---|---:|---:|---:|---:|---|");
        for c in &self.citizens {
            let most = c.most.as_ref().map_or(String::new(), |(item, n)| format!(" ({n} {item})"));
            let why = c.why_stuck.as_ref().map_or(String::new(), |w| format!(" ({})", w.replace('|', "/")));
            let _ = writeln!(
                md,
                "| {} | {} | {} → {} | {}{} | {} ({}) | {} ({}) | {} | {} | {}{} |",
                c.name,
                c.role,
                c.strands_start,
                c.strands_end,
                c.made,
                most,
                c.sold,
                c.sold_for,
                c.bought,
                c.bought_for,
                c.deeds,
                c.failed_steps,
                c.stuck,
                why
            );
        }
        md.push('\n');
    }

    fn rulings_md(&self, md: &mut String) {
        let _ = writeln!(md, "## How questions were settled\n");
        let total: u64 = self.rulings.values().sum();
        if total == 0 {
            let _ = writeln!(md, "No questions were asked.");
            return;
        }
        let _ = writeln!(md, "| Settled by | Questions | Share |");
        let _ = writeln!(md, "|---|---:|---:|");
        let mut by: Vec<(&String, &u64)> = self.rulings.iter().collect();
        by.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (label, n) in by {
            let _ = writeln!(md, "| {label} | {n} | {:.1}% |", 100.0 * *n as f64 / total as f64);
        }
    }

    /// The load between each reading, a row a window, for a spreadsheet.
    pub fn load_csv(&self) -> Option<String> {
        let soak = self.soak.as_ref()?;
        let mut csv = String::from(
            "at_ms,secs,ticks,tick_period_ms,tick_mean_ms,tick_p50_ms,tick_p95_ms,tick_p99_ms,ticks_over,players_in_world,sockets_open,sockets_opened,sockets_dropped,socket_resyncs,db_pool_size,db_pool_idle,db_transactions_per_sec\n",
        );
        let bound = |b: Option<f64>| b.map_or("over".to_string(), |b| b.to_string());
        for (at, w) in soak.windows() {
            let _ = writeln!(
                csv,
                "{at},{:.1},{},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{},{},{:.1}",
                w.secs,
                w.ticks,
                w.tick_period_ms,
                w.tick_mean_ms,
                bound(w.tick_p50_ms),
                bound(w.tick_p95_ms),
                bound(w.tick_p99_ms),
                w.ticks_over,
                w.players_in_world,
                w.sockets_open,
                w.sockets_opened,
                w.sockets_dropped,
                w.socket_resyncs,
                w.db_pool_size,
                w.db_pool_idle,
                w.db_transactions_per_sec
            );
        }
        Some(csv)
    }
}

fn conservation(md: &mut String, r: &Reconciliation) {
    let _ = writeln!(md, "## Conservation\n");
    let verdict = if r.holds() { "**Holds.**" } else { "**Drift: the soak fails.**" };
    let _ = writeln!(md, "{verdict}\n");
    let s = &r.strands;
    let _ = writeln!(md, "### Strands\n");
    let _ = writeln!(
        md,
        "Every account's Strands, with escrow: {} at the start, {} at the end. That's the start, plus {} new account{} at {}, plus {} in from the faucets, less {} gone to the sinks, plus {} the transfers net to: {} expected, {} off.\n",
        s.start_total,
        s.end_total,
        s.new_accounts,
        if s.new_accounts == 1 { "" } else { "s" },
        capitol_protocol::ledger::STARTING_STRANDS,
        s.faucets,
        -s.sinks,
        s.transfers,
        s.end_total - s.drift,
        s.drift
    );
    let _ = writeln!(md, "| Flow | Kind | Strands |");
    let _ = writeln!(md, "|---|---|---:|");
    for (kind, n) in &s.flows {
        let _ = writeln!(md, "| {kind} | {:?} | {n:+} |", capitol_protocol::ledger::StrandFlow::of(kind));
    }
    let _ = writeln!(
        md,
        "\nEscrow: {} at the start, {} at the end; {} unexplained.",
        s.escrow_start, s.escrow_end, s.escrow_drift
    );
    if s.other != 0 {
        let _ = writeln!(md, "\nTypes this build doesn't know moved {:+}.", s.other);
    }
    if !s.accounts.is_empty() {
        let _ = writeln!(md, "\nAccounts their trail doesn't explain (Strands over what it says):\n");
        for (name, over) in &s.accounts {
            let _ = writeln!(md, "- {name}: {over:+}");
        }
    }
    if !s.gone.is_empty() {
        let _ = writeln!(md, "\nAccounts gone, with their Strands:\n");
        for (name, strands) in &s.gone {
            let _ = writeln!(md, "- {name}: {strands}");
        }
    }
    let u = &r.units;
    let _ = writeln!(md, "\n### Units\n");
    if u.void {
        let _ = writeln!(md, "The server restarted during the soak, so its unit flows began again: units weren't checked.\n");
        return;
    }
    if !u.settled {
        let _ = writeln!(
            md,
            "A reading caught a write between its commit and its count, so an item may be a write out.\n"
        );
    }
    let _ = writeln!(md, "| Item | Change | Explained | Drift |");
    let _ = writeln!(md, "|---|---:|---:|---:|");
    for (item, (held, explained)) in &u.items {
        let _ = writeln!(md, "| {item} | {held:+} | {explained:+} | {} |", held - explained);
    }
    let _ = writeln!(md, "\n| Flow | Units |");
    let _ = writeln!(md, "|---|---:|");
    for (flow, n) in &u.flows {
        let _ = writeln!(md, "| {flow} | {n:+} |");
    }
    md.push('\n');
}

fn load(md: &mut String, soak: &Soak) {
    let _ = writeln!(md, "## Load\n");
    let w = soak.whole();
    let windows = soak.windows();
    let ms = |b: Option<f64>| b.map_or("over 1 s".to_string(), |b| format!("≤ {b} ms"));
    let worst = windows.iter().map(|(_, w)| w).max_by(|a, b| a.tick_mean_ms.total_cmp(&b.tick_mean_ms));
    let busiest = windows.iter().map(|(_, w)| w.db_transactions_per_sec).fold(0.0, f64::max);
    let most_open = soak.samples.iter().map(|l| l.load.sockets_open).max().unwrap_or(0);
    let most_players = soak.samples.iter().map(|l| l.load.players_in_world).max().unwrap_or(0);
    let _ = writeln!(md, "| | Whole soak |");
    let _ = writeln!(md, "|---|---|");
    let _ = writeln!(md, "| Tick budget | {:.1} ms |", w.tick_period_ms);
    let _ = writeln!(md, "| Ticks | {} |", w.ticks);
    let _ = writeln!(md, "| Tick time, mean | {:.2} ms |", w.tick_mean_ms);
    let _ = writeln!(md, "| Tick time, p50 / p95 / p99 | {} / {} / {} |", ms(w.tick_p50_ms), ms(w.tick_p95_ms), ms(w.tick_p99_ms));
    let _ = writeln!(md, "| Slowest tick | {:.1} ms |", soak.end.load.tick_max_ms);
    let _ = writeln!(md, "| Ticks over budget | {} ({:.2}%) |", w.ticks_over, 100.0 * w.ticks_over as f64 / w.ticks.max(1) as f64);
    if let Some(worst) = worst {
        let _ = writeln!(md, "| Worst minute, mean tick | {:.2} ms (p99 {}) |", worst.tick_mean_ms, ms(worst.tick_p99_ms));
    }
    let _ = writeln!(md, "| Most in the world | {most_players} |");
    let _ = writeln!(md, "| Sockets opened / most open | {} / {most_open} |", w.sockets_opened);
    let _ = writeln!(md, "| Sockets dropped | {} |", w.sockets_dropped);
    let _ = writeln!(md, "| Sockets resynced | {} |", w.socket_resyncs);
    let _ = writeln!(md, "| DB transactions a second, mean / busiest minute | {:.0} / {busiest:.0} |", w.db_transactions_per_sec);
    let _ = writeln!(md, "| DB pool at the end, size / idle | {} / {} |", w.db_pool_size, w.db_pool_idle);
    let ended: Vec<String> = soak
        .end
        .load
        .sockets_ended
        .iter()
        .map(|(how, n)| (how, n - soak.start.load.sockets_ended.get(how).copied().unwrap_or(0)))
        .filter(|(_, n)| *n > 0)
        .map(|(how, n)| format!("{n} {how}"))
        .collect();
    if !ended.is_empty() {
        let _ = writeln!(md, "\nSockets that ended: {}.", ended.join(", "));
    }
    md.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::ledger::{AccountBooks, Load, UnitFlow, TICK_BUCKETS_MS};
    use uuid::Uuid;

    fn traded(item: &str, sold: bool, quantity: i32, price: i64) -> Entry {
        Entry::Traded {
            item: item.into(),
            sold,
            quantity,
            price,
            at: 0.5,
        }
    }

    fn made(item: &str, quantity: i32) -> Entry {
        Entry::Made {
            item: item.into(),
            quantity,
            quality: 50,
            gathered: true,
            at: 0.1,
        }
    }

    fn run(name: &str, role: &str, entries: Vec<Entry>, start: i64, end: i64) -> (BotRun, BTreeMap<String, u64>) {
        let rulings = BTreeMap::from([("rules".to_string(), 9), ("rules, unsure".to_string(), 1)]);
        (
            BotRun {
                name: name.into(),
                role: role.into(),
                entries,
                strands_start: start,
                strands_end: end,
                error: None,
            },
            rulings,
        )
    }

    fn runs() -> Vec<(BotRun, BTreeMap<String, u64>)> {
        vec![
            run(
                "Percy Sawyer",
                "gatherer (wood)",
                vec![
                    made("log", 12),
                    made("log", 8),
                    traded("log", true, 10, 6),
                    Entry::Stuck { reason: "no logs near".into() },
                    Entry::Stuck { reason: "no logs near".into() },
                    Entry::Stuck { reason: "the plot has no room".into() },
                ],
                100,
                160,
            ),
            run(
                "Ada Wright",
                "crafter (planks)",
                vec![traded("log", false, 10, 6), Entry::Claimed { cost: 250 }],
                400,
                90,
            ),
        ]
    }

    #[test]
    fn citizens_and_trades_add_up() {
        let report = TownReport::build(&runs(), 3600.0, 10.0, None);
        let percy = &report.citizens[0];
        assert_eq!((percy.made, percy.sold, percy.sold_for), (20, 10, 60));
        assert_eq!(percy.most, Some(("log".to_string(), 20)));
        assert_eq!((percy.stuck, percy.why_stuck.as_deref()), (3, Some("no logs near")));
        let ada = &report.citizens[1];
        assert_eq!((ada.bought, ada.bought_for, ada.deeds), (10, 60, 1));
        assert_eq!(report.trades.len(), 2);
        assert_eq!(report.rulings["rules"], 18);
        // An hour of wall time at 10× is ten hours of ticks and half a day
        assert_eq!(report.sim.hours, 10.0);
        assert_eq!(report.day_hours, 12.0);
        let md = report.to_markdown();
        assert!(md.contains("| Percy Sawyer | gatherer (wood) | 100 → 160 | 20 (20 log) | 10 (60) | 0 (0) | 0 | 0 | 3 (no logs near) |"), "{md}");
        assert!(md.contains("| rules | 18 | 90.0% |"), "{md}");
        assert!(md.contains("## Production"), "the sim's sections follow");
        assert!(!md.contains("## Conservation"), "no soak, no books");
        assert!(md.contains("- Spent on tax: not measured"), "no books, no tax: {md}");
        assert!(report.conserved());
        assert!(report.load_csv().is_none());
    }

    fn ledger(at_ms: i64, percy: i64, ticks: u64, logs: i64, gathered: i64) -> Ledger {
        let mut load = Load {
            tick_period_ms: 10.0,
            ticks,
            tick_secs: ticks as f64 * 0.002,
            tick_buckets: vec![0; TICK_BUCKETS_MS.len() + 1],
            db_transactions: ticks as i64 * 3,
            sockets_open: 2,
            ..Load::default()
        };
        load.tick_buckets[2] = ticks;
        Ledger {
            at_ms,
            booted_ms: 1,
            accounts: vec![AccountBooks {
                player_id: Uuid::from_u128(1),
                name: "Percy Sawyer".into(),
                citizen: true,
                strands: percy,
                flows: BTreeMap::from([("exchange_deposit".to_string(), percy - 100)]),
            }],
            escrow_strands: 0,
            units: BTreeMap::from([("log".to_string(), logs)]),
            unit_flows: BTreeMap::from([(
                "log".to_string(),
                BTreeMap::from([(UnitFlow::Gathered.as_str().to_string(), gathered)]),
            )]),
            units_settled: true,
            crafts: BTreeMap::new(),
            grants: Default::default(),
            load,
        }
    }

    #[test]
    fn a_soak_says_whether_the_books_balance_and_how_hard_the_server_worked() {
        let samples = vec![ledger(0, 100, 0, 0, 0), ledger(60_000, 130, 6000, 10, 10), ledger(120_000, 160, 12_000, 20, 20)];
        let soak = Soak {
            start: samples[0].clone(),
            end: samples[2].clone(),
            samples,
        };
        let report = TownReport::build(&runs(), 120.0, 10.0, Some(soak.clone()));
        assert!(report.conserved());
        let md = report.to_markdown();
        assert!(md.contains("**Holds.**"), "{md}");
        assert!(md.contains("| log | +20 | +20 | 0 |"), "{md}");
        assert!(md.contains("| Tick time, p50 / p95 / p99 | ≤ 2 ms / ≤ 2 ms / ≤ 2 ms |"), "{md}");
        assert!(md.contains("| DB transactions a second, mean / busiest minute | 300 / 300 |"), "{md}");
        // The server's Strands for Percy, not the town's last look
        assert_eq!((report.citizens[0].strands_start, report.citizens[0].strands_end), (100, 160));
        let csv = report.load_csv().unwrap();
        assert_eq!(csv.lines().count(), 3, "a header and two windows");

        // Two logs from nowhere fail it
        let mut end = soak.end.clone();
        end.units.insert("log".into(), 22);
        let drifted = Soak { end, ..soak };
        let report = TownReport::build(&runs(), 120.0, 10.0, Some(drifted));
        assert!(!report.conserved());
        assert!(report.to_markdown().contains("| log | +22 | +20 | 2 |"));
    }

    /// The C1.7b soak's shape (#334): a standing bid's fill and a stall's
    /// rent reach Percy's balance with no entry in his run, and Strands rest
    /// in escrow, so his residue says -49. The books say the tax took 1.
    #[test]
    fn a_soaks_tax_is_the_books_not_the_residue() {
        let start = ledger(0, 100, 0, 0, 0);
        let mut end = ledger(60_000, 70, 6000, 0, 0);
        end.accounts[0].flows = BTreeMap::from([
            ("market_sale".to_string(), 60),
            ("bazaar_rent".to_string(), -80),
            ("market_escrow".to_string(), -9),
            ("property_tax".to_string(), -1),
        ]);
        end.escrow_strands = 9;
        let runs = vec![run("Percy Sawyer", "gatherer (wood)", vec![], 100, 149)];
        assert_eq!(runs[0].0.tax(), -49, "the residue counts what isn't tax");
        let soak = Soak {
            start: start.clone(),
            end: end.clone(),
            samples: vec![start, end],
        };
        assert_eq!(soak.tax(), 1);
        let report = TownReport::build(&runs, 60.0, 10.0, Some(soak));
        assert_eq!(report.sim.tax, Some(1));
        assert!(report.to_markdown().contains("- Spent on tax: 1 Strands"));
    }

    /// A balance run (D6.3) reads the soak's report: the server's Strands by
    /// trade, the Gini, the flows, and prices against their targets
    #[test]
    fn a_soaks_balance_is_cut_from_its_report() {
        use crate::citizen::balance::Balance;
        let samples = vec![ledger(0, 100, 0, 0, 0), ledger(120_000, 160, 12_000, 20, 20)];
        let soak = Soak {
            start: samples[0].clone(),
            end: samples[1].clone(),
            samples,
        };
        let report = TownReport::build(&runs(), 120.0, 10.0, Some(soak));
        let knobs = BTreeMap::from([("property_tax_rate".to_string(), 0.2)]);
        let targets = BTreeMap::from([("log".to_string(), 5)]);
        let run = Balance::of(&report, "tax-20pct.toml", knobs.clone(), &targets).unwrap();
        assert_eq!((run.preset.as_str(), &run.tunables, run.citizens), ("tax-20pct.toml", &knobs, 2));
        let percy = run.trades.iter().find(|t| t.role == "gatherer (wood)").unwrap();
        assert_eq!((percy.start, percy.end), (100, 160));
        assert_eq!(run.flows["exchange_deposit"], 60);
        assert_eq!((run.faucets, run.sinks), (60, 0));
        let log = run.prices.iter().find(|p| p.item == "log").unwrap();
        assert_eq!((log.traded, log.mean, log.target), (10, Some(6.0), Some(5)), "a trade counts once, not each side");
        assert!((log.off().unwrap() - 0.2).abs() < 1e-9);
        assert_eq!(run.gini_end, None, "one citizen has no spread");
        assert!(run.conserved());
        assert!(Balance::of(&TownReport::build(&runs(), 120.0, 10.0, None), "x", BTreeMap::new(), &targets).is_none());
    }

    /// A soak killed before its end still has the readings it wrote down
    #[test]
    fn the_books_a_soak_kept_read_back() {
        let lines: Vec<String> = [ledger(0, 100, 0, 0, 0), ledger(60_000, 130, 6000, 10, 10)]
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect();
        let soak = Soak::from_jsonl(&format!("{}\n\n{}\n", lines[0], lines[1])).unwrap().unwrap();
        assert_eq!((soak.samples.len(), soak.end.at_ms), (2, 60_000));
        let md = soak.to_markdown();
        assert!(md.contains("2 readings over 1.0 wall minutes") && md.contains("**Holds.**"), "{md}");
        assert!(Soak::from_jsonl(&lines[0]).unwrap().is_none(), "one reading is no soak");
        assert!(Soak::from_jsonl("{").is_err());
    }
}
