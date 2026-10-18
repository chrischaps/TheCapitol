//! What a sim run found: built from every bot's [`Entry`]s, written as
//! markdown for people and CSV for a spreadsheet. Pure, so it's tested
//! without a server.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One thing a bot saw or did. Times are game hours since the run began.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// Units came into the bot's hands, from a node or a craft.
    Made {
        item: String,
        quantity: i32,
        quality: i32,
        gathered: bool,
        at: f64,
    },
    /// A tool used its last durability.
    ToolBroke { item: String, at: f64 },
    /// A step and how long it took, in game hours. `ok` is false when it
    /// failed (a refusal, a timeout, a blocked walk).
    Spent { label: String, hours: f64, ok: bool },
    /// Why a step failed: the server's refusal, or what timed out.
    Failed { label: String, why: String },
    /// The planner found no way to the bot's goal.
    Stuck { reason: String },
    /// Fiber went in at the exchange.
    Deposited { fiber: i32, strands: i64 },
    /// A deed was bought.
    Claimed { cost: i64 },
}

/// One bot's run, as the report needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct BotRun {
    pub name: String,
    /// `gatherer (wood)`, `builder`.
    pub role: String,
    pub entries: Vec<Entry>,
    pub strands_start: i64,
    pub strands_end: i64,
    /// Why it stopped early, if it did.
    pub error: Option<String>,
}

impl BotRun {
    /// What the hourly tax took: whatever the balance lost that the bot's own
    /// deposits and deeds don't explain. Bots don't trade, so nothing else
    /// moves it.
    pub fn tax(&self) -> i64 {
        let mut expected = self.strands_start;
        for entry in &self.entries {
            match entry {
                Entry::Deposited { strands, .. } => expected += strands,
                Entry::Claimed { cost } => expected -= cost,
                _ => {}
            }
        }
        expected - self.strands_end
    }
}

/// D5's labour estimates for the gathered items, in seconds a unit, so the
/// report can say which came slower than the plan expected.
const D5_LABOUR_SECS: &[(&str, f64)] = &[
    ("fiber", 4.0),
    ("sticks", 12.0),
    ("clay", 12.0),
    ("stone", 22.0),
    ("log", 32.0),
    ("iron_ore", 90.0),
];

/// One item's line in the report.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemLine {
    pub item: String,
    pub gathered: i64,
    pub crafted: i64,
    /// Units a game hour, all bots together.
    pub per_hour: f64,
    pub p10: i32,
    pub p50: i32,
    pub p90: i32,
    /// When the swarm first had one, in game hours.
    pub first_at: f64,
    /// Game seconds of `gather <item>` per unit gathered.
    pub labour_secs: Option<f64>,
    pub d5_labour_secs: Option<f64>,
}

/// Time spent under one step label.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeLine {
    pub label: String,
    pub hours: f64,
    pub steps: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub bots: usize,
    pub roles: BTreeMap<String, usize>,
    pub hours: f64,
    pub scale: f64,
    pub wall_secs: f64,
    pub items: Vec<ItemLine>,
    pub tools_broke: BTreeMap<String, usize>,
    pub tax: i64,
    pub deeds: usize,
    pub deed_strands: i64,
    pub fiber_deposited: i64,
    pub time: Vec<TimeLine>,
    /// Reasons bots were stuck, most common first.
    pub stuck: Vec<(String, usize)>,
    /// Why steps failed, as `label: reason` with ids and numbers masked,
    /// most common first.
    pub failures: Vec<(String, usize)>,
    /// Bots that stopped before the end, and why.
    pub errors: Vec<(String, String)>,
}

impl Report {
    pub fn build(runs: &[BotRun], hours: f64, scale: f64, wall_secs: f64) -> Report {
        let mut roles = BTreeMap::new();
        let mut made: BTreeMap<String, Vec<(i32, i32)>> = BTreeMap::new();
        let mut counts: BTreeMap<String, (i64, i64, f64)> = BTreeMap::new();
        let mut tools_broke = BTreeMap::new();
        let mut time: BTreeMap<String, TimeLine> = BTreeMap::new();
        let mut stuck: BTreeMap<String, usize> = BTreeMap::new();
        let mut failures: BTreeMap<String, usize> = BTreeMap::new();
        let (mut deeds, mut deed_strands, mut fiber_deposited) = (0, 0, 0);
        for run in runs {
            *roles.entry(run.role.clone()).or_insert(0) += 1;
            for entry in &run.entries {
                match entry {
                    Entry::Made {
                        item,
                        quantity,
                        quality,
                        gathered,
                        at,
                    } => {
                        made.entry(item.clone()).or_default().push((*quality, *quantity));
                        let c = counts.entry(item.clone()).or_insert((0, 0, f64::INFINITY));
                        if *gathered {
                            c.0 += *quantity as i64;
                        } else {
                            c.1 += *quantity as i64;
                        }
                        c.2 = c.2.min(*at);
                    }
                    Entry::ToolBroke { item, .. } => *tools_broke.entry(item.clone()).or_insert(0) += 1,
                    Entry::Spent { label, hours, ok } => {
                        let line = time.entry(label.clone()).or_insert_with(|| TimeLine {
                            label: label.clone(),
                            hours: 0.0,
                            steps: 0,
                            failed: 0,
                        });
                        line.hours += hours;
                        line.steps += 1;
                        line.failed += usize::from(!ok);
                    }
                    Entry::Stuck { reason } => *stuck.entry(reason.clone()).or_insert(0) += 1,
                    Entry::Failed { label, why } => {
                        *failures.entry(format!("{label}: {}", mask(why))).or_insert(0) += 1
                    }
                    Entry::Deposited { fiber, .. } => fiber_deposited += *fiber as i64,
                    Entry::Claimed { cost } => {
                        deeds += 1;
                        deed_strands += cost;
                    }
                }
            }
        }
        let items = made
            .into_iter()
            .map(|(item, mut units)| {
                let (gathered, crafted, first_at) = counts[&item];
                units.sort_unstable();
                let labour_secs = time
                    .get(&format!("gather {item}"))
                    .filter(|_| gathered > 0)
                    .map(|t| t.hours * 3600.0 / gathered as f64);
                ItemLine {
                    per_hour: (gathered + crafted) as f64 / hours.max(f64::EPSILON),
                    p10: percentile(&units, 0.1),
                    p50: percentile(&units, 0.5),
                    p90: percentile(&units, 0.9),
                    first_at,
                    labour_secs,
                    d5_labour_secs: D5_LABOUR_SECS
                        .iter()
                        .find(|(i, _)| *i == item)
                        .map(|(_, s)| *s),
                    item,
                    gathered,
                    crafted,
                }
            })
            .collect();
        let mut time: Vec<TimeLine> = time.into_values().collect();
        time.sort_by(|a, b| b.hours.total_cmp(&a.hours));
        let mut stuck: Vec<(String, usize)> = stuck.into_iter().collect();
        stuck.sort_by(|a, b| b.1.cmp(&a.1));
        let mut failures: Vec<(String, usize)> = failures.into_iter().collect();
        failures.sort_by(|a, b| b.1.cmp(&a.1));
        Report {
            bots: runs.len(),
            roles,
            hours,
            scale,
            wall_secs,
            items,
            tools_broke,
            tax: runs.iter().map(BotRun::tax).sum(),
            deeds,
            deed_strands,
            fiber_deposited,
            time,
            stuck,
            failures,
            errors: runs
                .iter()
                .filter_map(|r| r.error.clone().map(|e| (r.name.clone(), e)))
                .collect(),
        }
    }

    fn item(&self, item: &str) -> Option<&ItemLine> {
        self.items.iter().find(|i| i.item == item)
    }

    /// Where the chains are held up, worst first, in words.
    pub fn bottlenecks(&self) -> Vec<String> {
        let mut found = Vec::new();
        let busy: f64 = self.time.iter().map(|t| t.hours).sum();
        if busy > 0.0 {
            for t in self.time.iter().take(3) {
                found.push(format!(
                    "`{}` took {:.0}% of all bot time ({:.1} bot-hours over {} steps{})",
                    t.label,
                    100.0 * t.hours / busy,
                    t.hours,
                    t.steps,
                    if t.failed > 0 {
                        format!(", {} failed", t.failed)
                    } else {
                        String::new()
                    }
                ));
            }
        }
        let mut slow: Vec<&ItemLine> = self
            .items
            .iter()
            .filter(|i| matches!((i.labour_secs, i.d5_labour_secs), (Some(a), Some(b)) if a > 1.5 * b))
            .collect();
        slow.sort_by(|a, b| {
            let ratio = |i: &ItemLine| i.labour_secs.unwrap() / i.d5_labour_secs.unwrap();
            ratio(b).total_cmp(&ratio(a))
        });
        for i in slow {
            found.push(format!(
                "{} came at {:.0} s a unit, {:.1}× D5's {:.0} s",
                i.item,
                i.labour_secs.unwrap(),
                i.labour_secs.unwrap() / i.d5_labour_secs.unwrap(),
                i.d5_labour_secs.unwrap()
            ));
        }
        for (step, failed) in self
            .time
            .iter()
            .filter(|t| t.steps >= 5 && t.failed * 4 >= t.steps)
            .map(|t| (&t.label, t.failed * 100 / t.steps))
        {
            found.push(format!("`{step}` failed {failed}% of the time"));
        }
        for (reason, n) in self.stuck.iter().take(3) {
            found.push(format!("stuck {n}×: {reason}"));
        }
        let never: Vec<&str> = ["iron_ingot", "plank", "iron_axe", "iron_pickaxe", "brick", "charcoal"]
            .into_iter()
            .filter(|i| self.item(i).is_none())
            .collect();
        if !never.is_empty() {
            found.push(format!("never made: {}", never.join(", ")));
        }
        found
    }

    /// Hours of grass for one deed of `deed_cost`, at the swarm's own fiber
    /// rate (D5 asks for it, to tune deeds against the content).
    pub fn deed_hours(&self, deed_cost: i64) -> Option<f64> {
        let fiber = self.item("fiber")?;
        let secs = fiber.labour_secs?;
        Some(deed_cost as f64 / (1.0 - capitol_protocol::rules::EXCHANGE_FEE_RATE) * secs / 3600.0)
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        let roles: Vec<String> = self.roles.iter().map(|(r, n)| format!("{n} {r}")).collect();
        let _ = writeln!(md, "# Economy sim\n");
        let _ = writeln!(
            md,
            "{} bots ({}) for {:.1} game hours at {}× real time ({:.1} wall minutes).\n",
            self.bots,
            roles.join(", "),
            self.hours,
            self.scale,
            self.wall_secs / 60.0
        );
        let _ = writeln!(md, "## Chain bottlenecks\n");
        for b in self.bottlenecks() {
            let _ = writeln!(md, "- {b}");
        }
        let _ = writeln!(md, "\n## Production\n");
        let _ = writeln!(
            md,
            "| Item | Gathered | Crafted | Per game hour | p10 | p50 | p90 | First at (h) | s a unit (D5) |"
        );
        let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|---:|");
        for i in &self.items {
            let labour = match (i.labour_secs, i.d5_labour_secs) {
                (Some(a), Some(b)) => format!("{a:.0} ({b:.0})"),
                (Some(a), None) => format!("{a:.0}"),
                _ => String::new(),
            };
            let _ = writeln!(
                md,
                "| {} | {} | {} | {:.1} | {} | {} | {} | {:.2} | {} |",
                i.item, i.gathered, i.crafted, i.per_hour, i.p10, i.p50, i.p90, i.first_at, labour
            );
        }
        let _ = writeln!(md, "\n## Tools worn out\n");
        if self.tools_broke.is_empty() {
            let _ = writeln!(md, "None.");
        }
        for (tool, n) in &self.tools_broke {
            let _ = writeln!(md, "- {tool}: {n}");
        }
        let _ = writeln!(md, "\n## Strands\n");
        let _ = writeln!(
            md,
            "- Fiber deposited: {}\n- Deeds bought: {} for {} Strands\n- Spent on tax: {} Strands",
            self.fiber_deposited, self.deeds, self.deed_strands, self.tax
        );
        for (zone, cost) in [("Rural", 250), ("Suburban", 500)] {
            if let Some(h) = self.deed_hours(cost) {
                let _ = writeln!(md, "- A {zone} deed ({cost}) is {h:.2} hours of grass");
            }
        }
        let _ = writeln!(md, "\n## Where the time went\n");
        let _ = writeln!(md, "| Step | Bot-hours | Steps | Failed |");
        let _ = writeln!(md, "|---|---:|---:|---:|");
        for t in &self.time {
            let _ = writeln!(md, "| {} | {:.2} | {} | {} |", t.label, t.hours, t.steps, t.failed);
        }
        if !self.failures.is_empty() {
            let _ = writeln!(md, "\n## Why steps failed\n");
            let _ = writeln!(md, "| Step: reason | Times |");
            let _ = writeln!(md, "|---|---:|");
            for (why, n) in self.failures.iter().take(12) {
                let _ = writeln!(md, "| {} | {n} |", why.replace('|', "/"));
            }
        }
        if !self.errors.is_empty() {
            let _ = writeln!(md, "\n## Bots that stopped early\n");
            for (name, e) in &self.errors {
                let _ = writeln!(md, "- {name}: {e}");
            }
        }
        md
    }

    /// One row per item, for a spreadsheet.
    pub fn to_csv(&self) -> String {
        let mut csv = String::from(
            "item,gathered,crafted,per_game_hour,p10,p50,p90,first_at_hours,secs_per_unit,d5_secs_per_unit,tools_broke\n",
        );
        for i in &self.items {
            let opt = |v: Option<f64>| v.map(|v| format!("{v:.1}")).unwrap_or_default();
            let _ = writeln!(
                csv,
                "{},{},{},{:.2},{},{},{},{:.3},{},{},{}",
                i.item,
                i.gathered,
                i.crafted,
                i.per_hour,
                i.p10,
                i.p50,
                i.p90,
                i.first_at,
                opt(i.labour_secs),
                opt(i.d5_labour_secs),
                self.tools_broke.get(&i.item).copied().unwrap_or(0)
            );
        }
        csv
    }
}

/// `why` with ids and numbers masked, so the same refusal counts once:
/// "node 3f2a… refused (5 s)" and "node 91c0… refused (3 s)" are one line.
fn mask(why: &str) -> String {
    let mut out = String::with_capacity(why.len());
    let mut in_token = false;
    for word in why.split_inclusive(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
        let (token, rest) = word.split_at(word.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-').len());
        let masked = token.chars().any(|c| c.is_ascii_digit());
        if masked {
            if !in_token {
                out.push('#');
            }
        } else {
            out.push_str(token);
        }
        in_token = masked && rest.is_empty();
        out.push_str(rest);
    }
    out
}

/// The quality below which `q` of the units fall, counting each unit.
/// `units` is `(quality, quantity)`, sorted by quality.
fn percentile(units: &[(i32, i32)], q: f64) -> i32 {
    let total: i64 = units.iter().map(|(_, n)| *n as i64).sum();
    if total == 0 {
        return 0;
    }
    let rank = ((total as f64 * q).ceil() as i64).max(1);
    let mut seen = 0i64;
    for (quality, n) in units {
        seen += *n as i64;
        if seen >= rank {
            return *quality;
        }
    }
    units.last().map_or(0, |(q, _)| *q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made(item: &str, quantity: i32, quality: i32, gathered: bool, at: f64) -> Entry {
        Entry::Made {
            item: item.into(),
            quantity,
            quality,
            gathered,
            at,
        }
    }

    fn run(role: &str, entries: Vec<Entry>, start: i64, end: i64) -> BotRun {
        BotRun {
            name: "Sim1".into(),
            role: role.into(),
            entries,
            strands_start: start,
            strands_end: end,
            error: None,
        }
    }

    #[test]
    fn masking_folds_ids_and_numbers() {
        assert_eq!(
            mask("the server didn't start extracting node 3f2a9c1e-0b7d-4c3e-9a2f-1d2e3f4a5b6c: no answer in 3 s"),
            "the server didn't start extracting node #: no answer in # s"
        );
        assert_eq!(mask("blocked at (3412, 7001): water"), "blocked at (#, #): water");
        assert_eq!(mask("Cannot access station"), "Cannot access station");
    }

    #[test]
    fn percentiles_count_every_unit() {
        let units = [(40, 1), (50, 8), (90, 1)];
        assert_eq!(percentile(&units, 0.1), 40);
        assert_eq!(percentile(&units, 0.5), 50);
        assert_eq!(percentile(&units, 0.9), 50);
        assert_eq!(percentile(&units, 1.0), 90);
        assert_eq!(percentile(&[], 0.5), 0);
    }

    #[test]
    fn tax_is_what_deposits_and_deeds_dont_explain() {
        let r = run(
            "builder",
            vec![
                Entry::Deposited {
                    fiber: 300,
                    strands: 299,
                },
                Entry::Claimed { cost: 250 },
            ],
            10,
            50,
        );
        assert_eq!(r.tax(), 9);
    }

    #[test]
    fn the_report_adds_up_the_swarm() {
        let runs = vec![
            run(
                "gatherer (fiber)",
                vec![
                    made("fiber", 2, 50, true, 0.01),
                    made("fiber", 3, 60, true, 0.02),
                    Entry::Spent {
                        label: "gather fiber".into(),
                        hours: 0.01,
                        ok: true,
                    },
                    Entry::Spent {
                        label: "gather fiber".into(),
                        hours: 0.01,
                        ok: false,
                    },
                ],
                0,
                0,
            ),
            run(
                "crafter (tools)",
                vec![
                    made("rope", 1, 55, false, 0.5),
                    Entry::ToolBroke {
                        item: "stone_axe".into(),
                        at: 1.0,
                    },
                    Entry::Stuck {
                        reason: "the plot has no room".into(),
                    },
                ],
                0,
                0,
            ),
        ];
        let report = Report::build(&runs, 2.0, 10.0, 720.0);
        assert_eq!(report.bots, 2);
        let fiber = report.item("fiber").unwrap();
        assert_eq!((fiber.gathered, fiber.crafted), (5, 0));
        assert_eq!(fiber.per_hour, 2.5);
        assert_eq!((fiber.p10, fiber.p90), (50, 60));
        assert_eq!(fiber.first_at, 0.01);
        // 72 game seconds over 5 fiber
        assert!((fiber.labour_secs.unwrap() - 14.4).abs() < 1e-9);
        assert_eq!(report.tools_broke["stone_axe"], 1);
        let bottlenecks = report.bottlenecks().join("\n");
        assert!(bottlenecks.contains("fiber came at 14 s a unit, 3.6× D5's 4 s"), "{bottlenecks}");
        assert!(bottlenecks.contains("stuck 1×: the plot has no room"), "{bottlenecks}");
        assert!(bottlenecks.contains("never made: iron_ingot"), "{bottlenecks}");
        let md = report.to_markdown();
        assert!(md.contains("| fiber | 5 | 0 | 2.5 | 50 | 60 | 60 | 0.01 | 14 (4) |"), "{md}");
        assert!(md.contains("- stone_axe: 1"));
        let csv = report.to_csv();
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.contains("\nfiber,5,0,2.50,50,60,60,0.010,14.4,4.0,0\n"), "{csv}");
        // 251.3 fiber at 14.4 s
        assert!((report.deed_hours(250).unwrap() - 1.005).abs() < 0.01);
    }
}
