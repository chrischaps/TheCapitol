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
    /// A gather came away with nothing: hands or a tool too weak for the
    /// resource (D5, #206).
    Missed { item: String, at: f64 },
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
    /// One of the bot's orders filled (E3.8): `sold` units, or bought them,
    /// at `price` each.
    Traded {
        item: String,
        sold: bool,
        quantity: i32,
        price: i64,
        at: f64,
    },
    /// An order's listing fee.
    Fee { strands: i64 },
    /// A trade in a trade window went through (C1.5, #295): with a citizen
    /// or a person, whether this bot asked for it, and the Strands that
    /// came in (less than nothing when it paid).
    Dealt {
        citizen: bool,
        asked: bool,
        strands: i64,
        at: f64,
    },
    /// A commission went up on the hall's board (Commons S2.5): `quantity`
    /// of `item` at `price` each, all of it escrowed, and the court's `fee`
    /// on top.
    Posted {
        item: String,
        quantity: i32,
        price: i64,
        fee: i64,
    },
    /// A claim on a commission: `quantity` of `item` promised.
    Promised { item: String, quantity: i32 },
    /// Units went in on a commission and were paid for from its escrow;
    /// `filled` when they were the last it wanted.
    Delivered {
        item: String,
        quantity: i32,
        paid: i64,
        filled: bool,
    },
    /// A person filled one of the bot's commissions.
    Filled { item: String },
    /// One of the bot's commissions ran past its due date, and `refunded`
    /// came back.
    Expired { item: String, refunded: i64 },
    /// A claim of the bot's ran out `quantity` short: a breach.
    Lapsed { item: String, quantity: i32 },
    /// It took piece-rate work (Commons S2.8): `work` in a few words.
    TookWork { work: String },
    /// `units` of `item` made on its shift went to the employer, and paid
    /// `paid` from the offer's escrow.
    Earned { item: String, units: i32, paid: i64 },
    /// A shift of its ended, `why` in a word (`cap`, `day`, `employer`),
    /// with `made` units made on it.
    ShiftEnded { why: String, made: i32 },
    /// It asked to join `guild`, or accepted its invitation (Commons S4.5).
    Applied { guild: String },
    /// It's in `guild` now.
    Joined { guild: String },
    /// It paid `strands` into its guild's treasury on a guild day.
    PaidIn { guild: String, strands: i64 },
    /// It's out of `guild`, `why` in a word (`debt`, `expelled`,
    /// `dissolved`).
    LeftGuild { guild: String, why: String },
    /// Its share of `guild`'s treasury as the guild closed.
    GuildShare { guild: String, strands: i64 },
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
    /// deposits, deeds and trades don't explain. A sim bot ends with no order
    /// open, so no Strands sit in escrow. A citizen doesn't: its standing
    /// orders rest in escrow, their fills and its stall's sales reach the
    /// balance with no [`Entry::Traded`], and bazaar rent has no entry at
    /// all. So a town reads its tax from the server's books instead (#334).
    pub fn tax(&self) -> i64 {
        let mut expected = self.strands_start;
        for entry in &self.entries {
            match entry {
                Entry::Deposited { strands, .. } => expected += strands,
                Entry::Claimed { cost } => expected -= cost,
                Entry::Traded {
                    sold, quantity, price, ..
                } => {
                    let value = price * *quantity as i64;
                    expected += if *sold { value } else { -value };
                }
                Entry::Fee { strands } => expected -= strands,
                Entry::Dealt { strands, .. } => expected += strands,
                Entry::Posted {
                    quantity, price, fee, ..
                } => expected -= price * *quantity as i64 + fee,
                Entry::Delivered { paid, .. } => expected += paid,
                Entry::Expired { refunded, .. } => expected += refunded,
                Entry::Earned { paid, .. } => expected += paid,
                Entry::PaidIn { strands, .. } => expected -= strands,
                Entry::GuildShare { strands, .. } => expected += strands,
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
    /// Gathers of it, and how many of those came away with nothing (D5's
    /// failed pulls, #206).
    pub pulls: i64,
    pub failed_pulls: i64,
    /// Game seconds of `gather <item>` per unit gathered.
    pub labour_secs: Option<f64>,
    pub d5_labour_secs: Option<f64>,
    /// Units that changed hands on the market, and at what prices
    pub traded: i64,
    pub price_low: Option<i64>,
    /// The mean price a unit, weighted by quantity
    pub price_mean: Option<f64>,
    pub price_high: Option<i64>,
    /// D5's value, where the makers' prices started
    pub d5_price: Option<i64>,
}

/// What went through the hall's board (Commons S2.5), every bot together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Court {
    /// Commissions posted
    pub posted: usize,
    /// What their posters put down: the price of all they asked for
    pub escrowed: i64,
    /// The court's fees on them, a sink
    pub fees: i64,
    /// Claims made, and the units they promised
    pub claims: usize,
    pub promised: i64,
    /// Units delivered, and what escrow paid for them
    pub delivered: i64,
    pub paid: i64,
    /// Commissions filled, whoever filled them
    pub filled: usize,
    /// Claims that ran out short
    pub lapsed: usize,
    /// Commissions that ran past their due date, and what came back
    pub expired: usize,
    pub refunded: i64,
}

impl Court {
    /// Whether anything went through it.
    pub fn busy(&self) -> bool {
        *self != Court::default()
    }

    fn count(&mut self, entry: &Entry) {
        match entry {
            Entry::Posted {
                quantity, price, fee, ..
            } => {
                self.posted += 1;
                self.escrowed += price * *quantity as i64;
                self.fees += fee;
            }
            Entry::Promised { quantity, .. } => {
                self.claims += 1;
                self.promised += *quantity as i64;
            }
            Entry::Delivered {
                quantity, paid, filled, ..
            } => {
                self.delivered += *quantity as i64;
                self.paid += paid;
                self.filled += usize::from(*filled);
            }
            Entry::Filled { .. } => self.filled += 1,
            Entry::Lapsed { .. } => self.lapsed += 1,
            Entry::Expired { refunded, .. } => {
                self.expired += 1;
                self.refunded += refunded;
            }
            _ => {}
        }
    }
}

/// What citizens earned working for others (Commons S2.8), every bot
/// together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wages {
    /// Shifts taken
    pub shifts: usize,
    /// Units made on them, and what they were paid
    pub units: i64,
    pub earned: i64,
    /// Shifts ended, by why
    pub ended: BTreeMap<String, usize>,
}

impl Wages {
    /// Whether anyone worked for a wage.
    pub fn busy(&self) -> bool {
        *self != Wages::default()
    }

    fn count(&mut self, entry: &Entry) {
        match entry {
            Entry::TookWork { .. } => self.shifts += 1,
            Entry::Earned { units, paid, .. } => {
                self.units += *units as i64;
                self.earned += paid;
            }
            Entry::ShiftEnded { why, .. } => *self.ended.entry(why.clone()).or_insert(0) += 1,
            _ => {}
        }
    }
}

/// What citizens did in the guilds (Commons S4.5), every bot together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuildLife {
    /// Applications made, invitations accepted among them
    pub applied: usize,
    /// Members gained, by guild
    pub joined: BTreeMap<String, usize>,
    /// Deposits made on guild days, and the Strands in them
    pub deposits: usize,
    pub paid_in: i64,
    /// Guilds left, by why
    pub left: BTreeMap<String, usize>,
    /// Shares of closing treasuries
    pub shares: i64,
}

impl GuildLife {
    /// Whether any citizen had anything to do with a guild.
    pub fn busy(&self) -> bool {
        *self != GuildLife::default()
    }

    fn count(&mut self, entry: &Entry) {
        match entry {
            Entry::Applied { .. } => self.applied += 1,
            Entry::Joined { guild } => *self.joined.entry(guild.clone()).or_insert(0) += 1,
            Entry::PaidIn { strands, .. } => {
                self.deposits += 1;
                self.paid_in += strands;
            }
            Entry::LeftGuild { why, .. } => *self.left.entry(why.clone()).or_insert(0) += 1,
            Entry::GuildShare { strands, .. } => self.shares += strands,
            _ => {}
        }
    }
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
    /// What the tax took, or `None` when nothing measured it: a town
    /// without a soak's books (#334)
    pub tax: Option<i64>,
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
    /// Market fees paid, all bots together
    pub fees: i64,
    /// The market makers' Strands, at the start and at the end
    pub makers: Option<(i64, i64)>,
    /// What went through the hall's board (S2.5)
    pub court: Court,
    /// What citizens earned working for others (S2.8)
    pub wages: Wages,
    /// What citizens did in the guilds (S4.5)
    pub guilds: GuildLife,
}

/// What a market maker's [`BotRun::role`] says.
pub const MAKER_ROLE: &str = "market maker";

impl Report {
    pub fn build(runs: &[BotRun], hours: f64, scale: f64, wall_secs: f64) -> Report {
        let mut roles = BTreeMap::new();
        let mut made: BTreeMap<String, Vec<(i32, i32)>> = BTreeMap::new();
        let mut counts: BTreeMap<String, (i64, i64, f64)> = BTreeMap::new();
        // Gathers, and those that failed
        let mut pulls: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        let mut tools_broke = BTreeMap::new();
        let mut time: BTreeMap<String, TimeLine> = BTreeMap::new();
        let mut stuck: BTreeMap<String, usize> = BTreeMap::new();
        let mut failures: BTreeMap<String, usize> = BTreeMap::new();
        let (mut deeds, mut deed_strands, mut fiber_deposited) = (0, 0, 0);
        // Each trade counted once, from its seller: (quantity, price)
        let mut sales: BTreeMap<String, Vec<(i64, i64)>> = BTreeMap::new();
        let mut fees = 0;
        let mut court = Court::default();
        let mut wages = Wages::default();
        let mut guilds = GuildLife::default();
        for run in runs {
            *roles.entry(run.role.clone()).or_insert(0) += 1;
            for entry in &run.entries {
                court.count(entry);
                wages.count(entry);
                guilds.count(entry);
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
                            pulls.entry(item.clone()).or_default().0 += 1;
                        } else {
                            c.1 += *quantity as i64;
                        }
                        c.2 = c.2.min(*at);
                    }
                    Entry::Missed { item, .. } => {
                        let p = pulls.entry(item.clone()).or_default();
                        p.0 += 1;
                        p.1 += 1;
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
                    Entry::Traded {
                        item,
                        sold: true,
                        quantity,
                        price,
                        ..
                    } => sales.entry(item.clone()).or_default().push((*quantity as i64, *price)),
                    Entry::Traded { .. } => {}
                    Entry::Fee { strands } => fees += strands,
                    // The swarm trades only at the hall; a town's report
                    // counts trades in person
                    Entry::Dealt { .. } => {}
                    // The board's, counted by the court
                    Entry::Posted { .. }
                    | Entry::Promised { .. }
                    | Entry::Delivered { .. }
                    | Entry::Filled { .. }
                    | Entry::Expired { .. }
                    | Entry::Lapsed { .. } => {}
                    // Counted by the wages
                    Entry::TookWork { .. } | Entry::Earned { .. } | Entry::ShiftEnded { .. } => {}
                    // Counted by the guilds
                    Entry::Applied { .. }
                    | Entry::Joined { .. }
                    | Entry::PaidIn { .. }
                    | Entry::LeftGuild { .. }
                    | Entry::GuildShare { .. } => {}
                }
            }
        }
        // An item only traded (a maker's stock), or only ever missed, still gets a line
        for item in sales.keys().chain(pulls.keys()) {
            made.entry(item.clone()).or_default();
        }
        let items = made
            .into_iter()
            .map(|(item, mut units)| {
                let (gathered, crafted, first_at) = counts.get(&item).copied().unwrap_or((0, 0, f64::NAN));
                let fills = sales.get(&item).map(Vec::as_slice).unwrap_or_default();
                let traded: i64 = fills.iter().map(|(q, _)| q).sum();
                units.sort_unstable();
                let labour_secs = time
                    .get(&format!("gather {item}"))
                    .filter(|_| gathered > 0)
                    .map(|t| t.hours * 3600.0 / gathered as f64);
                let (pulls, failed_pulls) = pulls.get(&item).copied().unwrap_or_default();
                ItemLine {
                    pulls,
                    failed_pulls,
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
                    traded,
                    price_low: fills.iter().map(|(_, p)| *p).min(),
                    price_mean: (traded > 0)
                        .then(|| fills.iter().map(|(q, p)| q * p).sum::<i64>() as f64 / traded as f64),
                    price_high: fills.iter().map(|(_, p)| *p).max(),
                    // The seed content's: a report reads no server's catalog
                    d5_price: crate::catalog::Catalog::fallback().target_price(&item),
                    item,
                    gathered,
                    crafted,
                }
            })
            .collect();
        let mut time: Vec<TimeLine> = time.into_values().collect();
        time.sort_by(|a, b| b.hours.total_cmp(&a.hours));
        let mut stuck: Vec<(String, usize)> = stuck.into_iter().collect();
        stuck.sort_by_key(|s| std::cmp::Reverse(s.1));
        let mut failures: Vec<(String, usize)> = failures.into_iter().collect();
        failures.sort_by_key(|f| std::cmp::Reverse(f.1));
        Report {
            bots: runs.len(),
            roles,
            hours,
            scale,
            wall_secs,
            items,
            tools_broke,
            tax: Some(runs.iter().map(BotRun::tax).sum()),
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
            fees,
            makers: runs
                .iter()
                .filter(|r| r.role == MAKER_ROLE)
                .map(|r| (r.strands_start, r.strands_end))
                .reduce(|a, b| (a.0 + b.0, a.1 + b.1)),
            court,
            wages,
            guilds,
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
        for i in self.items.iter().filter(|i| i.pulls >= 5 && i.failed_pulls * 4 >= i.pulls) {
            found.push(format!(
                "{} failed {}% of its pulls ({} of {})",
                i.item,
                i.failed_pulls * 100 / i.pulls,
                i.failed_pulls,
                i.pulls
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
    /// rate (D5 asks for it, to tune deeds against the content), and the
    /// default exchange fee: a yardstick for the content, not for a run's
    /// live fees (D6.1).
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
        md.push_str(&self.sections());
        md
    }

    /// Everything after the title and the line that says who ran: the
    /// bottlenecks, production, pulls, tools, Strands, time and failures, each
    /// a `##` section. The town's report (C1.7) puts them under its own.
    pub fn sections(&self) -> String {
        let mut md = String::new();
        let _ = writeln!(md, "## Chain bottlenecks\n");
        for b in self.bottlenecks() {
            let _ = writeln!(md, "- {b}");
        }
        let _ = writeln!(md, "\n## Production\n");
        let _ = writeln!(
            md,
            "| Item | Gathered | Crafted | Per game hour | p10 | p50 | p90 | First at (h) | s a unit (D5) | Traded | Low | Mean price (D5) | High |"
        );
        let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
        for i in &self.items {
            let labour = match (i.labour_secs, i.d5_labour_secs) {
                (Some(a), Some(b)) => format!("{a:.0} ({b:.0})"),
                (Some(a), None) => format!("{a:.0}"),
                _ => String::new(),
            };
            let num = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_default();
            let mean = match (i.price_mean, i.d5_price) {
                (Some(m), Some(d)) => format!("{m:.1} ({d})"),
                (Some(m), None) => format!("{m:.1}"),
                _ => String::new(),
            };
            let made = i.gathered + i.crafted > 0;
            let _ = writeln!(
                md,
                "| {} | {} | {} | {:.1} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                i.item,
                i.gathered,
                i.crafted,
                i.per_hour,
                if made { i.p10.to_string() } else { String::new() },
                if made { i.p50.to_string() } else { String::new() },
                if made { i.p90.to_string() } else { String::new() },
                if made { format!("{:.2}", i.first_at) } else { String::new() },
                labour,
                if i.traded > 0 { i.traded.to_string() } else { String::new() },
                num(i.price_low),
                mean,
                num(i.price_high)
            );
        }
        let _ = writeln!(md, "\n## Pulls that failed\n");
        let pulled: Vec<&ItemLine> = self.items.iter().filter(|i| i.pulls > 0).collect();
        if pulled.is_empty() {
            let _ = writeln!(md, "Nothing was gathered.");
        } else {
            let _ = writeln!(md, "| Item | Pulls | Failed | Share |");
            let _ = writeln!(md, "|---|---:|---:|---:|");
        }
        for i in pulled {
            let share = 100.0 * i.failed_pulls as f64 / i.pulls as f64;
            let _ = writeln!(md, "| {} | {} | {} | {share:.0}% |", i.item, i.pulls, i.failed_pulls);
        }
        let _ = writeln!(md, "\n## Tools worn out\n");
        if self.tools_broke.is_empty() {
            let _ = writeln!(md, "None.");
        }
        for (tool, n) in &self.tools_broke {
            let _ = writeln!(md, "- {tool}: {n}");
        }
        let _ = writeln!(md, "\n## Strands\n");
        let tax = self
            .tax
            .map_or("not measured: only the server's books can say, in a soak".to_string(), |t| format!("{t} Strands"));
        let _ = writeln!(
            md,
            "- Fiber deposited: {}\n- Deeds bought: {} for {} Strands\n- Spent on tax: {tax}\n- Market fees: {} Strands",
            self.fiber_deposited, self.deeds, self.deed_strands, self.fees
        );
        if let Some((start, end)) = self.makers {
            let _ = writeln!(md, "- Market makers: {start} Strands at the start, {end} at the end");
        }
        let deed = super::run::DEFAULT_DEED;
        if let Some(h) = self.deed_hours(deed) {
            let _ = writeln!(md, "- A deed ({deed}, in any home zone) is {h:.2} hours of grass");
        }
        if self.court.busy() {
            let c = &self.court;
            let _ = writeln!(md, "\n## The hall's board\n");
            let _ = writeln!(
                md,
                "- Commissions posted: {}, {} Strands put down, {} in the court's fees\n- Claims: {}, promising {} units; {} lapsed\n- Delivered: {} units, paid {} Strands from escrow\n- Filled: {}; ran out: {}, {} Strands refunded\n- Strands through the court: {} put in by these posters, {} paid out to these deliverers",
                c.posted,
                c.escrowed,
                c.fees,
                c.claims,
                c.promised,
                c.lapsed,
                c.delivered,
                c.paid,
                c.filled,
                c.expired,
                c.refunded,
                c.escrowed + c.fees,
                c.paid
            );
        }
        if self.wages.busy() {
            let w = &self.wages;
            let ended: Vec<String> = w.ended.iter().map(|(why, n)| format!("{n} {}", ended_words(why))).collect();
            let ended = if ended.is_empty() { "none yet".to_string() } else { ended.join(", ") };
            let _ = writeln!(md, "\n## Wages\n");
            let _ = writeln!(
                md,
                "- Shifts taken: {}; ended: {ended}\n- Units made for employers: {}, paid {} Strands from their escrow",
                w.shifts, w.units, w.earned
            );
        }
        if self.guilds.busy() {
            let g = &self.guilds;
            let joined: Vec<String> = g.joined.iter().map(|(name, n)| format!("{name} {n}")).collect();
            let joined = if joined.is_empty() { "none".to_string() } else { joined.join(", ") };
            let left: Vec<String> = g.left.iter().map(|(why, n)| format!("{n} {}", left_words(why))).collect();
            let left = if left.is_empty() { "none".to_string() } else { left.join(", ") };
            let shares = if g.shares > 0 {
                format!("; {} Strands back in shares as guilds closed", g.shares)
            } else {
                String::new()
            };
            let _ = writeln!(md, "\n## Guilds\n");
            let _ = writeln!(
                md,
                "- Applications: {}; members gained: {joined}; left: {left}\n- Paid in on guild days: {} deposits, {} Strands{shares}",
                g.applied, g.deposits, g.paid_in
            );
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
            "item,gathered,crafted,per_game_hour,p10,p50,p90,first_at_hours,secs_per_unit,d5_secs_per_unit,tools_broke,traded,price_low,price_mean,price_high,d5_price,pulls,failed_pulls\n",
        );
        for i in &self.items {
            let opt = |v: Option<f64>| v.map(|v| format!("{v:.1}")).unwrap_or_default();
            let int = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_default();
            let _ = writeln!(
                csv,
                "{},{},{},{:.2},{},{},{},{:.3},{},{},{},{},{},{},{},{},{},{}",
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
                self.tools_broke.get(&i.item).copied().unwrap_or(0),
                i.traded,
                int(i.price_low),
                opt(i.price_mean),
                int(i.price_high),
                int(i.d5_price),
                i.pulls,
                i.failed_pulls
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

/// Why a shift ended, as the wages section says it: `at the cap`.
fn ended_words(why: &str) -> String {
    match why {
        "cap" => "at the cap".into(),
        "term" => "at the offer's end".into(),
        "day" => "at the end of a day".into(),
        "employer" => "by the employer".into(),
        "failed" => "when the work failed".into(),
        "worker" => "by the worker".into(),
        why => why.replace('_', " "),
    }
}

/// Why a citizen left a guild, as the guilds section says it.
fn left_words(why: &str) -> String {
    match why {
        "debt" => "over its debt".into(),
        "dissolved" => "as it was dissolved".into(),
        why => why.replace('_', " "),
    }
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

    fn traded(item: &str, sold: bool, quantity: i32, price: i64) -> Entry {
        Entry::Traded {
            item: item.into(),
            sold,
            quantity,
            price,
            at: 0.5,
        }
    }

    #[test]
    fn trades_and_fees_are_not_tax() {
        let r = run(
            "gatherer (wood)",
            vec![
                traded("log", true, 10, 7),
                traded("plank", false, 4, 6),
                Entry::Fee { strands: 1 },
            ],
            100,
            100 + 70 - 24 - 1 - 5,
        );
        assert_eq!(r.tax(), 5);
    }

    #[test]
    fn prices_count_each_trade_once_from_its_seller() {
        let mut maker = run(
            MAKER_ROLE,
            vec![traded("plank", true, 10, 6), traded("plank", false, 10, 4), Entry::Fee { strands: 2 }],
            20_000,
            20_018,
        );
        maker.name = "Maker1".into();
        let runs = vec![
            run(
                "crafter (planks)",
                vec![made("plank", 30, 55, false, 0.2), traded("plank", true, 10, 4), Entry::Fee { strands: 1 }],
                100,
                139,
            ),
            run("crafter (ingots)", vec![traded("plank", false, 10, 6)], 100, 40),
            maker,
        ];
        let report = Report::build(&runs, 1.0, 10.0, 360.0);
        let plank = report.item("plank").unwrap();
        assert_eq!(plank.traded, 20);
        assert_eq!((plank.price_low, plank.price_high), (Some(4), Some(6)));
        assert_eq!(plank.price_mean, Some(5.0));
        assert_eq!(plank.d5_price, Some(5));
        assert_eq!(report.fees, 3);
        assert_eq!(report.makers, Some((20_000, 20_018)));
        assert_eq!(report.tax, Some(0));
        let md = report.to_markdown();
        assert!(md.contains("| 20 | 4 | 5.0 (5) | 6 |"), "{md}");
        assert!(md.contains("- Market makers: 20000 Strands at the start, 20018 at the end"), "{md}");
        assert!(report.to_csv().contains(",20,4,5.0,6,5,0,0\n"));
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
        assert!(md.contains("| fiber | 5 | 0 | 2.5 | 50 | 60 | 60 | 0.01 | 14 (4) |  |  |  |  |"), "{md}");
        assert!(md.contains("- stone_axe: 1"));
        let csv = report.to_csv();
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.contains("\nfiber,5,0,2.50,50,60,60,0.010,14.4,4.0,0,0,,,,1,2,0\n"), "{csv}");
        // 251.3 fiber at 14.4 s
        assert!((report.deed_hours(250).unwrap() - 1.005).abs() < 0.01);
    }

    /// D5's failed pulls (#206): counted against the item, never as units,
    /// and a share of a quarter or more is a bottleneck
    #[test]
    fn failed_pulls_are_counted_not_made() {
        let missed = |at| Entry::Missed {
            item: "iron_ore".into(),
            at,
        };
        let runs = vec![run(
            "gatherer (ore)",
            vec![
                made("iron_ore", 1, 70, true, 0.1),
                missed(0.2),
                made("iron_ore", 1, 50, true, 0.3),
                missed(0.4),
                missed(0.5),
                made("iron_ore", 1, 60, true, 0.6),
                Entry::Missed {
                    item: "stone".into(),
                    at: 0.7,
                },
            ],
            0,
            0,
        )];
        let report = Report::build(&runs, 1.0, 10.0, 360.0);
        let ore = report.item("iron_ore").unwrap();
        assert_eq!((ore.gathered, ore.pulls, ore.failed_pulls), (3, 6, 3));
        assert_eq!((ore.p10, ore.p50, ore.p90), (50, 60, 70), "a miss has no quality");
        let stone = report.item("stone").unwrap();
        assert_eq!((stone.gathered, stone.pulls, stone.failed_pulls), (0, 1, 1), "missed only, still a line");
        let bottlenecks = report.bottlenecks().join("\n");
        assert!(bottlenecks.contains("iron_ore failed 50% of its pulls (3 of 6)"), "{bottlenecks}");
        assert!(!bottlenecks.contains("stone failed"), "too few pulls to say: {bottlenecks}");
        let md = report.to_markdown();
        assert!(md.contains("| iron_ore | 6 | 3 | 50% |"), "{md}");
        assert!(report.to_csv().contains(",6,3\n"));
    }

    #[test]
    fn the_board_is_counted_once_and_its_strands_in_the_books() {
        let poster = BotRun {
            name: "Ida".into(),
            role: "crafter (planks)".into(),
            entries: vec![
                Entry::Posted {
                    item: "log".into(),
                    quantity: 10,
                    price: 12,
                    fee: 6,
                },
                Entry::Filled { item: "log".into() },
                Entry::Expired {
                    item: "stone".into(),
                    refunded: 40,
                },
            ],
            strands_start: 500,
            strands_end: 500 - 126 + 40,
            error: None,
        };
        let deliverer = BotRun {
            name: "Percy".into(),
            role: "gatherer (wood)".into(),
            entries: vec![
                Entry::Promised {
                    item: "log".into(),
                    quantity: 6,
                },
                Entry::Delivered {
                    item: "log".into(),
                    quantity: 6,
                    paid: 72,
                    filled: true,
                },
                Entry::Lapsed {
                    item: "rope".into(),
                    quantity: 2,
                },
            ],
            strands_start: 100,
            strands_end: 172,
            error: None,
        };
        // Escrow, fees, pay and refunds are no tax
        assert_eq!((poster.tax(), deliverer.tax()), (0, 0));
        let report = Report::build(&[poster, deliverer], 1.0, 1.0, 60.0);
        let c = report.court;
        assert_eq!((c.posted, c.escrowed, c.fees), (1, 120, 6));
        assert_eq!((c.claims, c.promised, c.lapsed), (1, 6, 1));
        assert_eq!((c.delivered, c.paid, c.filled), (6, 72, 2));
        assert_eq!((c.expired, c.refunded), (1, 40));
        let md = report.sections();
        assert!(md.contains("## The hall's board"), "{md}");
        assert!(md.contains("126 put in by these posters, 72 paid out"), "{md}");
        // A run with nothing on the board says nothing of it
        assert!(!Report::build(&[], 1.0, 1.0, 60.0).sections().contains("hall's board"));
    }

    #[test]
    fn wages_are_counted_and_their_pay_is_no_tax() {
        let took = || Entry::TookWork {
            work: "craft plank at a sawbench".into(),
        };
        let earned = || Entry::Earned {
            item: "plank".into(),
            units: 3,
            paid: 6,
        };
        let worker = BotRun {
            name: "Percy".into(),
            role: "crafter (planks)".into(),
            entries: vec![
                took(),
                earned(),
                earned(),
                Entry::ShiftEnded { why: "day".into(), made: 6 },
                took(),
                Entry::ShiftEnded { why: "cap".into(), made: 0 },
            ],
            strands_start: 100,
            strands_end: 112,
            error: None,
        };
        assert_eq!(worker.tax(), 0);
        let report = Report::build(&[worker], 1.0, 1.0, 60.0);
        let w = &report.wages;
        assert_eq!((w.shifts, w.units, w.earned), (2, 6, 12));
        let md = report.sections();
        assert!(md.contains("## Wages"), "{md}");
        assert!(md.contains("Shifts taken: 2; ended: 1 at the cap, 1 at the end of a day"), "{md}");
        assert!(md.contains("Units made for employers: 6, paid 12 Strands"), "{md}");
        assert!(!Report::build(&[], 1.0, 1.0, 60.0).sections().contains("## Wages"));
    }

    #[test]
    fn guild_life_is_counted_and_paying_in_is_no_tax() {
        let member = BotRun {
            name: "Percy".into(),
            role: "gatherer (ore)".into(),
            entries: vec![
                Entry::Applied {
                    guild: "The Ore Guild".into(),
                },
                Entry::Joined {
                    guild: "The Ore Guild".into(),
                },
                Entry::PaidIn {
                    guild: "The Ore Guild".into(),
                    strands: 12,
                },
                Entry::LeftGuild {
                    guild: "The Ore Guild".into(),
                    why: "dissolved".into(),
                },
                Entry::GuildShare {
                    guild: "The Ore Guild".into(),
                    strands: 4,
                },
            ],
            strands_start: 100,
            strands_end: 92,
            error: None,
        };
        assert_eq!(member.tax(), 0);
        let report = Report::build(&[member], 1.0, 1.0, 60.0);
        let g = &report.guilds;
        assert_eq!((g.applied, g.deposits, g.paid_in, g.shares), (1, 1, 12, 4));
        let md = report.sections();
        assert!(md.contains("## Guilds"), "{md}");
        assert!(md.contains("Applications: 1; members gained: The Ore Guild 1; left: 1 as it was dissolved"), "{md}");
        assert!(md.contains("Paid in on guild days: 1 deposits, 12 Strands; 4 Strands back in shares"), "{md}");
        assert!(!Report::build(&[], 1.0, 1.0, 60.0).sections().contains("## Guilds"));
    }
}
