//! Replay (C3.1): any stack of layers run over recorded decisions, offline,
//! so a new layer is measured before it ever runs live (plan D4).
//!
//! Each logged situation is put to a fresh [`Cascade`] (one a log file, as
//! one a citizen), and its ruling set against the one recorded. A layer
//! that pends is waited on, as the citizen would have asked again, until
//! it answers or its time is up. The [`Report`] gives:
//!
//! - **agreement**: rulings the same as recorded, entry and parameters, in
//!   all and by question; and of the choices the rules made (under a
//!   rules-only stack, or settled or picked by the rules in any), how many
//!   the replay reproduces;
//! - **escalation**: how often the first layer didn't settle it, and each
//!   layer's verdicts;
//! - **latency** of each layer and of a whole ruling;
//! - **cost**: what each layer spent, in US cents;
//! - and the outcomes and fills the log recorded, for context.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::{Duration, Instant};

use super::log::Record;
use super::{By, Cascade, Decision, Question, Ruling, Situation};

/// How often a pending layer is asked again.
const POLL: Duration = Duration::from_millis(2);
/// How many differences a report lists.
const SHOWN: usize = 12;

/// A decision the replay ruled otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    pub citizen: String,
    pub id: u64,
    pub question: String,
    pub recorded: Option<Decision>,
    pub replayed: Option<Decision>,
    /// The entries, as the menu has them
    pub recorded_choice: String,
    pub replayed_choice: String,
}

/// One layer's record over a replay.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerStats {
    pub name: String,
    pub asked: usize,
    /// Its verdicts, by [`Asked::as_str`](super::Asked::as_str)
    pub verdicts: BTreeMap<&'static str, usize>,
    micros: Vec<u64>,
    pub cents: f64,
}

impl LayerStats {
    /// The latency at quantile `q` (0 to 1), in microseconds.
    pub fn latency(&self, q: f64) -> Option<u64> {
        quantile(&self.micros, q)
    }
}

/// What a replay found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// The stack replayed, bottom first
    pub stack: Vec<String>,
    pub logs: usize,
    /// Runs of a citizen: a rotated file's start is its run's again
    pub runs: usize,
    pub decisions: usize,
    /// The same entry and parameters as recorded
    pub same: usize,
    /// The same entry, whatever its parameters
    pub same_pick: usize,
    /// Choices the rules made when it was recorded
    pub rules_choices: usize,
    pub rules_same: usize,
    /// By question: (decisions, the same)
    pub questions: BTreeMap<String, (usize, usize)>,
    /// Settled by the first layer of the stack
    pub first_settled: usize,
    /// Waited on a pending layer
    pub waited: usize,
    pub layers: Vec<LayerStats>,
    /// A whole ruling's time, in microseconds
    micros: Vec<u64>,
    /// The outcomes recorded, by [`Fate::as_str`](super::log::Fate::as_str)
    pub outcomes: BTreeMap<&'static str, usize>,
    pub fills: usize,
    /// Lines that wouldn't read
    pub unreadable: Vec<String>,
    pub differences: Vec<Difference>,
    pub differ: usize,
}

impl Report {
    /// The share of rulings the same as recorded.
    pub fn agreement(&self) -> f64 {
        share(self.same, self.decisions)
    }

    /// The share of the rules' recorded choices the replay reproduced.
    pub fn rules_agreement(&self) -> f64 {
        share(self.rules_same, self.rules_choices)
    }

    /// The share of rulings the first layer didn't settle.
    pub fn escalation(&self) -> f64 {
        1.0 - share(self.first_settled, self.decisions)
    }

    /// A whole ruling's latency at quantile `q`, in microseconds.
    pub fn latency(&self, q: f64) -> Option<u64> {
        quantile(&self.micros, q)
    }
}

/// Runs a stack over logs. [`log`](Self::log) each citizen's records, then
/// [`finish`](Self::finish).
pub struct Replayer {
    make: Box<dyn FnMut() -> Cascade>,
    report: Report,
    runs: BTreeSet<(String, i64)>,
}

impl Replayer {
    /// `make` builds the stack afresh for each log.
    pub fn new(make: impl FnMut() -> Cascade + 'static) -> Replayer {
        let mut make: Box<dyn FnMut() -> Cascade> = Box::new(make);
        let stack = make().names();
        Replayer {
            report: Report {
                layers: stack
                    .iter()
                    .map(|name| LayerStats {
                        name: name.clone(),
                        ..LayerStats::default()
                    })
                    .collect(),
                stack,
                ..Report::default()
            },
            make,
            runs: BTreeSet::new(),
        }
    }

    /// Replays one log's records, in order. `name` labels its unreadable
    /// lines.
    pub fn log(&mut self, name: &str, records: impl IntoIterator<Item = Result<Record, String>>) {
        let mut cascade = (self.make)();
        let report = &mut self.report;
        report.logs += 1;
        let mut citizen = String::new();
        let mut run_layers: Vec<String> = Vec::new();
        for record in records {
            let record = match record {
                Ok(record) => record,
                Err(e) => {
                    report.unreadable.push(format!("{name}: {e}"));
                    continue;
                }
            };
            match record {
                Record::Start {
                    citizen: who, layers, run, ..
                } => {
                    self.runs.insert((who.clone(), run));
                    report.runs = self.runs.len();
                    citizen = who;
                    run_layers = layers;
                }
                Record::Decision { id, situation, ruling, .. } => {
                    let rules_only = run_layers == ["rules"];
                    replay_one(report, &mut cascade, &citizen, id, &situation, &ruling, rules_only);
                }
                Record::Outcome { fate, .. } => *report.outcomes.entry(fate.as_str()).or_default() += 1,
                Record::Fill { .. } => report.fills += 1,
            }
        }
        for (stats, (_, cents)) in report.layers.iter_mut().zip(cascade.spent()) {
            stats.cents += cents;
        }
    }

    pub fn finish(self) -> Report {
        self.report
    }
}

fn replay_one(report: &mut Report, cascade: &mut Cascade, citizen: &str, id: u64, s: &Situation, recorded: &Ruling, rules_only: bool) {
    let began = Instant::now();
    let mut ruling = cascade.ask(s, began);
    let mut turns = ruling.trail.clone();
    let waited = matches!(ruling.by, By::Waiting { .. });
    while matches!(ruling.by, By::Waiting { .. }) {
        std::thread::sleep(POLL);
        ruling = cascade.ask(s, Instant::now());
        turns.extend(ruling.trail.iter().cloned());
    }
    report.micros.push(began.elapsed().as_micros() as u64);
    report.decisions += 1;
    report.waited += waited as usize;
    for turn in &turns {
        if let Some(stats) = report.layers.iter_mut().find(|l| l.name == turn.layer) {
            stats.asked += 1;
            *stats.verdicts.entry(turn.asked.as_str()).or_default() += 1;
            stats.micros.push(turn.micros);
        }
    }
    if report.stack.first().is_some_and(|first| ruling.by == By::Layer { name: first.clone() }) {
        report.first_settled += 1;
    }
    let same = ruling.decision == recorded.decision;
    let same_pick = ruling.decision.map(|d| d.pick) == recorded.decision.map(|d| d.pick);
    report.same += same as usize;
    report.same_pick += same_pick as usize;
    let question = report.questions.entry(kind(&s.question).to_string()).or_default();
    question.0 += 1;
    question.1 += same as usize;
    let by_rules = matches!(&recorded.by, By::Layer { name } if name == "rules") || recorded.by == By::Rules;
    if rules_only || by_rules {
        report.rules_choices += 1;
        report.rules_same += same as usize;
    }
    if !same {
        report.differ += 1;
        if report.differences.len() < SHOWN {
            let entry = |d: Option<Decision>| d.and_then(|d| s.menu.get(d.pick)).map_or("nothing".to_string(), |c| format!("{c:?}"));
            report.differences.push(Difference {
                citizen: citizen.to_string(),
                id,
                question: s.question.key(),
                recorded: recorded.decision,
                replayed: ruling.decision,
                recorded_choice: entry(recorded.decision),
                replayed_choice: entry(ruling.decision),
            });
        }
    }
}

/// A question's kind, for a report: `work`, `sell`.
fn kind(q: &Question) -> &'static str {
    match q {
        Question::Work => "work",
        Question::Sell { .. } => "sell",
        Question::Buy { .. } => "buy",
        Question::Trade { .. } => "trade",
        Question::FreeTime => "free time",
        Question::Market => "market",
    }
}

fn share(n: usize, of: usize) -> f64 {
    if of == 0 {
        return 1.0;
    }
    n as f64 / of as f64
}

fn quantile(values: &[u64], q: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let i = ((sorted.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize;
    Some(sorted[i])
}

fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

fn micros(v: Option<u64>) -> String {
    match v {
        None => "–".into(),
        Some(us) if us >= 10_000 => format!("{} ms", us / 1000),
        Some(us) => format!("{us} µs"),
    }
}

fn decision(d: &Option<Decision>) -> String {
    match d {
        None => "nothing".into(),
        Some(d) => {
            let mut s = format!("#{}", d.pick);
            if let Some(p) = d.price {
                s.push_str(&format!(" at {p}"));
            }
            if let Some(q) = d.quantity {
                s.push_str(&format!(" ×{q}"));
            }
            s
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "replayed {} decisions from {} log{} ({} run{}) with {}",
            self.decisions,
            self.logs,
            if self.logs == 1 { "" } else { "s" },
            self.runs,
            if self.runs == 1 { "" } else { "s" },
            self.stack.join(" → ")
        )?;
        writeln!(
            f,
            "  agreement    {} of {} the same ({}); the same entry {}",
            self.same,
            self.decisions,
            pct(self.agreement()),
            pct(share(self.same_pick, self.decisions))
        )?;
        writeln!(
            f,
            "  the rules'   {} of {} choices the rules made reproduced ({})",
            self.rules_same,
            self.rules_choices,
            pct(self.rules_agreement())
        )?;
        if !self.questions.is_empty() {
            let by: Vec<String> = self.questions.iter().map(|(q, (n, same))| format!("{q} {same}/{n}")).collect();
            writeln!(f, "  by question  {}", by.join(" · "))?;
        }
        writeln!(
            f,
            "  escalation   {} not settled by {}; {} waited on a pending layer",
            pct(self.escalation()),
            self.stack.first().map_or("the first layer", String::as_str),
            self.waited
        )?;
        writeln!(
            f,
            "  a ruling     p50 {} · p95 {} · max {}",
            micros(self.latency(0.5)),
            micros(self.latency(0.95)),
            micros(self.latency(1.0))
        )?;
        for layer in &self.layers {
            let verdicts: Vec<String> = layer.verdicts.iter().map(|(v, n)| format!("{v} {n}")).collect();
            writeln!(
                f,
                "  {:<12} asked {}: {}",
                layer.name,
                layer.asked,
                if verdicts.is_empty() { "–".into() } else { verdicts.join(" · ") }
            )?;
            writeln!(
                f,
                "  {:<12} p50 {} · p95 {} · max {} · spent {:.2}¢",
                "",
                micros(layer.latency(0.5)),
                micros(layer.latency(0.95)),
                micros(layer.latency(1.0)),
                layer.cents
            )?;
        }
        let known: usize = self.outcomes.values().sum();
        let fates: Vec<String> = self.outcomes.iter().map(|(fate, n)| format!("{n} {fate}")).collect();
        writeln!(
            f,
            "  outcomes     {known} recorded{}{}; {} fill{}",
            if fates.is_empty() { "" } else { ": " },
            fates.join(", "),
            self.fills,
            if self.fills == 1 { "" } else { "s" }
        )?;
        if !self.unreadable.is_empty() {
            writeln!(f, "  unreadable   {} line(s), first {}", self.unreadable.len(), self.unreadable[0])?;
        }
        if self.differ == 0 {
            return writeln!(f, "  differences  none");
        }
        writeln!(f, "  differences  {} (the first {}):", self.differ, self.differences.len())?;
        for d in &self.differences {
            writeln!(
                f,
                "    {} #{} {}: recorded {} {}, replayed {} {}",
                d.citizen,
                d.id,
                d.question,
                decision(&d.recorded),
                d.recorded_choice,
                decision(&d.replayed),
                d.replayed_choice
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::log::{self, Fate, Log};
    use super::super::{Choice, Decider, Held, Layer, Mind, Place, Quote, Span, Temperament, Ticket, Verdict, When, Who};
    use super::*;
    use crate::catalog::Catalog;
    use crate::sim::plan::Holdings;
    use crate::sim::{Role, MIX};
    use std::sync::{Arc, Mutex};

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("capitol-replay-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Every role's work, sell, buy, bid and stall questions over a spread
    /// of holdings, books and temperaments: what a town asks in a day.
    fn situations() -> Vec<Situation> {
        let catalog = Catalog::fallback();
        let mut out = Vec::new();
        let mut n = 0u32;
        for (role, _) in MIX {
            for (strands, fiber, product, last) in [(0, 0, 0, None), (40, 120, 12, Some(3)), (300, 260, 45, Some(9)), (900, 30, 80, Some(1))] {
                n += 1;
                let t = |k: u32| ((n * 37 + k * 11) % 10) as f32 / 9.0;
                let temperament = Temperament {
                    thrift: t(1),
                    risk: t(2),
                    sociability: t(3),
                    patience: t(4),
                };
                let mut h = Holdings {
                    strands,
                    deed_cost: 250,
                    ..Holdings::default()
                };
                h.items.insert("fiber".into(), fiber);
                if let Some(p) = role.product() {
                    h.items.insert(p.into(), product);
                }
                let quote = |item: &str| Quote {
                    item: item.into(),
                    bid: last.map(|l: i64| l - 1),
                    ask: last.map(|l: i64| l + 2),
                    last,
                };
                let market: Vec<Quote> = ["log", "plank", "iron_ore", "iron_ingot", "brick", "stone"].iter().map(|i| quote(i)).collect();
                let s = |question, menu: Vec<Choice>, carry_on| Situation {
                    who: Who {
                        name: format!("Citizen {n}"),
                        role: role.slug().into(),
                        temperament,
                    },
                    at: Place {
                        x: 6000.0 + n as f64 * 13.7,
                        y: 6000.0 - n as f64 * 3.1,
                        zone: "urban".into(),
                    },
                    when: When {
                        time_of_day: Some(0.3 + n as f64 * 0.01),
                        hours: n as f64 * 0.37,
                    },
                    holdings: (&h).into(),
                    market: market.clone(),
                    question,
                    menu,
                    carry_on: Some(carry_on),
                };
                let Ok(errands) = role.errands(&catalog, &h) else { continue };
                out.push(s(Question::Work, errands.iter().map(|e| e.choice(&catalog)).collect(), 0));
                out.push(s(Question::FreeTime, vec![Choice::Rest, Choice::Hall], 0));
                if let Some(p) = role.product() {
                    let goal = errands.first().and_then(|e| e.goal.as_ref());
                    if let Some(menu) = role.sell_menu(&h, goal, last.map(|l| l - 1), last.map(|l| l + 2)) {
                        out.push(s(Question::Sell { item: p.into() }, menu, 1));
                    }
                    if let Some(menu) = role.stall_menu(&h, last, false) {
                        out.push(s(Question::Sell { item: p.into() }, menu, 1));
                    }
                }
                for e in &errands {
                    if let Some(menu) = role.buy_menu(&h, &e.step, last.map(|l| l + 2)) {
                        let item = menu.iter().find_map(|c| match c {
                            Choice::Buy { item, .. } => Some(item.clone()),
                            _ => None,
                        });
                        out.push(s(Question::Buy { item: item.unwrap_or_default() }, menu, 1));
                    }
                }
                for item in role.needs() {
                    if let Some(menu) = role.bid_menu(&h, item, last, false) {
                        out.push(s(Question::Buy { item: item.to_string() }, menu, 1));
                    }
                }
            }
        }
        out
    }

    /// A log of `situations` as a rules-only citizen decided them.
    fn record(path: &std::path::Path, situations: &[Situation]) {
        let log = Log::open(path, "Percy Sawyer", Role::Builder.slug(), &["rules".into()], log::LOG_LIMIT, log::LOG_KEEP).unwrap();
        let mut mind = Mind::new(Cascade::rules(), Temperament::default()).with_log(Arc::new(Mutex::new(log)));
        for s in situations {
            let mark = mind.mark();
            mind.ask(s);
            mind.settle(mark, &Held::default(), |_| Fate::Done);
        }
    }

    fn replay(path: &std::path::Path, make: impl FnMut() -> Cascade + 'static) -> Report {
        let mut replayer = Replayer::new(make);
        replayer.log("test", log::read(path).unwrap());
        replayer.finish()
    }

    #[test]
    fn the_rules_alone_reproduce_a_rules_log_exactly() {
        let dir = temp("rules");
        let path = log::file_for(&dir, "Percy Sawyer");
        let situations = situations();
        let questions: std::collections::BTreeSet<_> = situations.iter().map(|s| kind(&s.question)).collect();
        assert!(situations.len() >= 60, "{}", situations.len());
        assert_eq!(questions.len(), 4, "work, free time, sell and buy: {questions:?}");
        record(&path, &situations);
        let report = replay(&path, Cascade::rules);
        assert_eq!(report.decisions, situations.len());
        assert_eq!((report.runs, report.logs), (1, 1));
        assert_eq!(report.rules_choices, situations.len(), "a rules-only log is all the rules' choices");
        assert_eq!(report.rules_same, report.rules_choices, "{report}");
        assert_eq!(report.same, report.decisions, "{report}");
        assert_eq!(report.differ, 0);
        assert_eq!(report.outcomes.get("done"), Some(&situations.len()));
        assert!(report.unreadable.is_empty());
        let rules = &report.layers[0];
        assert_eq!(rules.asked, situations.len());
        assert!(rules.latency(0.5).is_some());
        // Some of the town's questions are close calls
        assert!(report.escalation() > 0.0 && report.escalation() < 1.0, "{report}");
        let text = report.to_string();
        assert!(text.contains("100.0%") && text.contains("differences  none"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A layer that always picks the last entry, quite sure of it.
    struct Contrary;
    impl Decider for Contrary {
        fn name(&self) -> &str {
            "contrary"
        }
        fn decide(&mut self, s: &Situation) -> Verdict {
            let pick = s.menu.len().saturating_sub(1);
            let (price, quantity) = match s.menu.get(pick).map(|c| c.takes()) {
                Some((p, q)) => (p.map(|p| p.min), q.map(|q| q.min)),
                None => (None, None),
            };
            Verdict::Decided {
                choice: Decision { pick, price, quantity },
                confidence: 0.9,
            }
        }
        fn spent(&self) -> f64 {
            0.25
        }
    }

    #[test]
    fn another_stack_is_measured_against_the_log() {
        let dir = temp("contrary");
        let path = log::file_for(&dir, "Percy Sawyer");
        let situations = situations();
        record(&path, &situations);
        let report = replay(&path, || Cascade::new(vec![Layer::new(Contrary, 0.5, Duration::from_secs(1))]));
        assert_eq!(report.stack, ["contrary"]);
        assert_eq!(report.first_settled, report.decisions, "it's always sure");
        assert!(report.differ > 0 && report.same < report.decisions, "{report}");
        assert_eq!(report.differ, report.decisions - report.same);
        assert!(report.differences.len() <= SHOWN);
        assert_eq!(report.layers[0].cents, 0.25);
        let text = report.to_string();
        assert!(text.contains("differences  ") && text.contains("Percy Sawyer #"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A layer that thinks a while: pending, then the first entry.
    struct Slow {
        asked: Option<Instant>,
    }
    impl Decider for Slow {
        fn name(&self) -> &str {
            "slow"
        }
        fn decide(&mut self, _s: &Situation) -> Verdict {
            self.asked = Some(Instant::now());
            Verdict::Pending(Ticket(1))
        }
        fn poll(&mut self, _t: Ticket) -> Option<Verdict> {
            let ready = self.asked.is_some_and(|a| a.elapsed() >= Duration::from_millis(6));
            ready.then(|| Verdict::Decided {
                choice: Decision::plain(0),
                confidence: 1.0,
            })
        }
    }

    #[test]
    fn a_pending_layer_is_waited_on() {
        let dir = temp("slow");
        let path = log::file_for(&dir, "Percy Sawyer");
        let s = situations().into_iter().find(|s| s.question == Question::FreeTime).unwrap();
        record(&path, std::slice::from_ref(&s));
        let report = replay(&path, || Cascade::new(vec![Layer::new(Slow { asked: None }, 0.5, Duration::from_secs(1))]));
        assert_eq!(report.waited, 1);
        assert_eq!(report.first_settled, 1);
        let slow = &report.layers[0];
        assert!(slow.verdicts["pending"] >= 2, "asked, then polled: {:?}", slow.verdicts);
        assert_eq!(slow.verdicts["settled"], 1);
        assert!(report.latency(0.5).unwrap() >= 6000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outcomes_and_fills_are_logged_against_their_decisions() {
        let dir = temp("outcomes");
        let path = log::file_for(&dir, "Percy Sawyer");
        let log = Log::open(&path, "Percy Sawyer", "builder", &["rules".into()], log::LOG_LIMIT, log::LOG_KEEP).unwrap();
        let mut mind = Mind::new(Cascade::rules(), Temperament::default()).with_log(Arc::new(Mutex::new(log)));
        mind.set_scale(10.0);
        let mut s = situations().into_iter().find(|s| s.question == Question::FreeTime).unwrap();
        s.holdings.strands = 100;
        s.holdings.items.insert("log".into(), 10);
        s.menu.push(Choice::Bid {
            item: "log".into(),
            price: Span::new(1, 9),
            quantity: Span::new(1, 9),
        });
        // A work question, then a bid that went first
        let mark = mind.mark();
        mind.ask(&s);
        let order = uuid::Uuid::from_u128(5);
        mind.placed(order);
        let mut after = s.holdings.clone();
        after.strands = 70;
        mind.settle(mark, &after, |c| match c {
            Choice::Rest => Fate::Done,
            _ => Fate::Superseded,
        });
        mind.filled(order, 2, 14, false);
        mind.filled(order, 1, 7, true);
        mind.filled(order, 1, 7, true);
        mind.stocked("brick");
        mind.sold("brick", 3, 30);
        mind.sold("plank", 3, 30);
        drop(mind);
        let records: Vec<Record> = log::read(&path).unwrap().map(Result::unwrap).collect();
        assert!(matches!(records[0], Record::Start { .. }));
        let Record::Decision { id, .. } = records[1] else { panic!("{:?}", records[1]) };
        match &records[2] {
            Record::Outcome {
                id: of,
                fate,
                strands,
                items,
                took,
                ..
            } => {
                assert_eq!((*of, fate), (id, &Fate::Done), "the free time's rest was done");
                assert_eq!(*strands, -30);
                assert!(items.is_empty());
                assert!(*took >= 0.0);
            }
            other => panic!("{other:?}"),
        }
        let fills: Vec<(u64, i64, i64)> = records[3..]
            .iter()
            .map(|r| match r {
                Record::Fill { id, quantity, strands, .. } => (*id, *quantity, *strands),
                other => panic!("{other:?}"),
            })
            .collect();
        // The bid's two fills, not a third once it's done; the stall's sale,
        // and nothing for a stall it never stocked
        assert_eq!(fills, [(id, 2, 14), (id, 1, 7), (id, 3, 30)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mind_without_a_log_keeps_nothing_open() {
        let mut mind = Mind::rules();
        let s = situations().remove(0);
        mind.ask(&s);
        assert_eq!(mind.mark(), 0);
    }
}
