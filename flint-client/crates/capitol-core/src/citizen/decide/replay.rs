//! Replay (C3.1): any stack of layers run over recorded decisions, offline,
//! so a new layer is measured before it ever runs live (plan D4).
//!
//! Each logged situation is put to a fresh [`Cascade`] (one a log file, as
//! one a citizen), and its ruling set against the one recorded. A layer
//! that pends is waited on, as the citizen would have asked again, until
//! it answers or its time is up.
//!
//! A network layer would replay at its own latency, one question at a time
//! (#301), so up to [`in_flight`](Replayer::in_flight) situations are asked
//! before any is polled, each on a cascade of its own: a cascade waits on
//! one question of a kind at a time. Each log still lands in its own order,
//! so the report reads the same however many fly. A situation the first
//! layer settles lands as it's asked, so a rules-only stack replays one at
//! a time, exactly as before. The [`Report`] gives:
//!
//! - **agreement**: rulings the same as recorded, entry and parameters, in
//!   all and by question; and of the choices the rules made (under a
//!   rules-only stack, or settled or picked by the rules in any), how many
//!   the replay reproduces;
//! - **escalation**: how often the first layer didn't settle it, and each
//!   layer's verdicts;
//! - **latency** of each layer and of a whole ruling;
//! - **cost**: what each layer spent, in US cents;
//! - **calibration** (C3.3): each layer's answers by how sure it was, and
//!   how many of them picked the entry recorded. The recorded pick is
//!   mostly the rules', so this says how well a layer's confidence tracks
//!   agreeing with them, not how often it's right;
//! - the outcomes and fills the log recorded;
//! - and **lived** (#302): calibration against what came of each choice,
//!   not against the rules. It reads the log's own rulings, not the
//!   replay's, so it costs nothing to run: each layer whose answer stood
//!   when it was recorded, by how sure it was and by question, with how
//!   many of its choices [worked](Lived::worked), how many failed, and
//!   what they [came to](Lived::value) in Strands at the books.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::time::{Duration, Instant};

use super::log::{Fate, Record};
use super::{Asked, By, Cascade, Choice, Decision, Question, Ruling, Situation, Turn};

/// How often a pending layer is asked again.
const POLL: Duration = Duration::from_millis(2);
/// How many differences a report lists.
const SHOWN: usize = 12;
/// How much of an escalation's reason a report keeps.
const REASON: usize = 80;

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
    /// Its answers (settled or unsure) by confidence, in tenths: how many,
    /// and how many picked the entry recorded
    pub calibration: BTreeMap<usize, (usize, usize)>,
    /// Why it escalated, and how often
    pub reasons: BTreeMap<String, usize>,
    /// What the layer tallied of itself, such as the models that answered
    pub tally: BTreeMap<String, usize>,
}

impl LayerStats {
    /// The latency at quantile `q` (0 to 1), in microseconds.
    pub fn latency(&self, q: f64) -> Option<u64> {
        quantile(&self.micros, q)
    }
}

/// What came of the choices a layer made live (#302).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lived {
    /// Its answers that stood: settled by it, or the best offered
    pub stood: usize,
    /// Of those, the ones whose outcome was logged, and that were acted on
    /// (a superseded choice wasn't)
    pub known: usize,
    /// Did what it chose: a step or an errand done; a sale, a buy or a
    /// trade that moved goods or Strands; a bid or a stall that filled; a
    /// keep, a gather or a decline as chosen
    pub worked: usize,
    pub failed: usize,
    /// What its choices came to, in Strands at the books when each was
    /// made: the change in Strands and goods to its outcome, or for a bid
    /// or a stall, its fills (its escrow and its stock come back, so they
    /// aren't counted). Goods with no book count nothing.
    pub value: i64,
}

impl Lived {
    fn add(&mut self, o: &Lived) {
        self.stood += o.stood;
        self.known += o.known;
        self.worked += o.worked;
        self.failed += o.failed;
        self.value += o.value;
    }

    /// The share of its known choices that worked.
    pub fn worked_share(&self) -> f64 {
        share(self.worked, self.known)
    }
}

/// A layer's live choices, all told, by confidence and by question.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LivedStats {
    pub all: Lived,
    /// By its confidence, in tenths; the rules' own pick has none
    pub tenths: BTreeMap<usize, Lived>,
    /// By question and the kind of entry chosen: `sell: keep`
    pub questions: BTreeMap<String, Lived>,
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
    /// What came of the choices each layer made live, by the layer whose
    /// answer stood (#302)
    pub lived: BTreeMap<String, LivedStats>,
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
    /// Replay every this many decisions
    stride: usize,
    seen: usize,
    /// How many situations may wait on a layer at once
    in_flight: usize,
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
            stride: 1,
            seen: 0,
            in_flight: 1,
        }
    }

    /// Replays only every `n`th decision, to sample a costly layer.
    pub fn stride(mut self, n: usize) -> Replayer {
        self.stride = n.max(1);
        self
    }

    /// Lets up to `n` situations wait on a pending layer at once, so a
    /// network layer replays `n` times as fast as its latency allows
    /// (#301). Each has a cascade of its own, and so its own spend cap.
    /// One, the default, is sequential.
    pub fn in_flight(mut self, n: usize) -> Replayer {
        self.in_flight = n.max(1);
        self
    }

    /// Replays one log's records, in order. `name` labels its unreadable
    /// lines.
    pub fn log(&mut self, name: &str, records: impl IntoIterator<Item = Result<Record, String>>) {
        let report = &mut self.report;
        let mut flights = Flights::new(&mut *self.make, self.in_flight);
        report.logs += 1;
        let mut citizen = String::new();
        let mut run_layers: Vec<String> = Vec::new();
        let mut lives = Lives::default();
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
                    lives.run = run;
                }
                Record::Decision { id, situation, ruling, .. } => {
                    lives.decision(id, &situation, &ruling);
                    self.seen += 1;
                    if !(self.seen - 1).is_multiple_of(self.stride) {
                        continue;
                    }
                    let flight = Flight {
                        citizen: citizen.clone(),
                        id,
                        situation: *situation,
                        recorded: *ruling,
                        rules_only: run_layers == ["rules"],
                    };
                    flights.send(report, flight);
                }
                Record::Outcome {
                    id, fate, strands, items, ..
                } => {
                    *report.outcomes.entry(fate.as_str()).or_default() += 1;
                    lives.outcome(id, fate, strands, &items);
                }
                Record::Fill { id, quantity, strands, .. } => {
                    report.fills += 1;
                    lives.fill(id, quantity, strands);
                }
            }
        }
        lives.fold(&mut report.lived);
        flights.drain(report);
        for cascade in &flights.cascades {
            for (stats, (_, cents)) in report.layers.iter_mut().zip(cascade.spent()) {
                stats.cents += cents;
            }
            for (stats, tally) in report.layers.iter_mut().zip(cascade.tallies()) {
                for (k, n) in tally {
                    *stats.tally.entry(k).or_default() += n;
                }
            }
        }
    }

    pub fn finish(self) -> Report {
        self.report
    }
}

/// One log's live choices, waiting on their outcomes and fills (#302).
#[derive(Default)]
struct Lives {
    /// The run being read: decision ids are the run's
    run: i64,
    lives: Vec<Life>,
    by_id: HashMap<(i64, u64), usize>,
    /// What the layers below a pending one said, by question, for the
    /// ruling that comes of its answer: that trail has only its own turn
    offered: HashMap<String, Vec<Turn>>,
}

/// A choice that stood, and what came of it.
struct Life {
    layer: String,
    tenth: Option<usize>,
    question: String,
    choice: Choice,
    /// Each book's price when it was decided
    books: BTreeMap<String, i64>,
    fate: Option<Fate>,
    moved: bool,
    value: i64,
    fills: usize,
}

impl Lives {
    fn decision(&mut self, id: u64, s: &Situation, ruling: &Ruling) {
        let key = s.question.key();
        let name = match &ruling.by {
            By::Waiting { .. } => {
                self.offered.insert(key, ruling.trail.clone());
                return;
            }
            By::Layer { name } | By::Offered { name } => name.clone(),
            By::Rules => "rules".into(),
            By::CarryOn => return,
        };
        let earlier = self.offered.remove(&key).unwrap_or_default();
        let Some(d) = ruling.decision.filter(|d| d.pick < s.menu.len()) else { return };
        // The rules' own pick, when no layer offered one, has no confidence
        let tenth = (ruling.by != By::Rules)
            .then(|| {
                ruling.trail.iter().chain(&earlier).find_map(|t| match &t.asked {
                    Asked::Settled { confidence, choice } | Asked::Unsure { confidence, choice } if t.layer == name && *choice == d => {
                        Some(tenth(*confidence))
                    }
                    _ => None,
                })
            })
            .flatten();
        let choice = s.choice(&d).clone();
        self.by_id.insert((self.run, id), self.lives.len());
        self.lives.push(Life {
            layer: name,
            tenth,
            question: format!("{}: {}", kind(&s.question), entry_kind(&choice)),
            choice,
            books: s.market.iter().filter_map(|q| Some((q.item.clone(), book(q)?))).collect(),
            fate: None,
            moved: false,
            value: 0,
            fills: 0,
        });
    }

    fn outcome(&mut self, id: u64, fate: Fate, strands: i64, items: &BTreeMap<String, i32>) {
        let Some(life) = self.by_id.get(&(self.run, id)).map(|&i| &mut self.lives[i]) else { return };
        life.moved = strands != 0 || !items.is_empty();
        // A bid's escrow and a stall's stock come back; their fills are
        // what they came to
        if !matches!(life.choice, Choice::Bid { .. } | Choice::Stock { .. }) {
            life.value = strands + items.iter().map(|(item, n)| *n as i64 * life.books.get(item).copied().unwrap_or(0)).sum::<i64>();
        }
        life.fate = Some(fate);
    }

    fn fill(&mut self, id: u64, quantity: i64, strands: i64) {
        let Some(life) = self.by_id.get(&(self.run, id)).map(|&i| &mut self.lives[i]) else { return };
        let (item, buying) = match &life.choice {
            Choice::Bid { item, .. } => (item, true),
            Choice::Stock { item, .. } => (item, false),
            _ => return,
        };
        let goods = quantity * life.books.get(item).copied().unwrap_or(0);
        life.value += if buying { goods - strands } else { strands - goods };
        life.fills += 1;
    }

    fn fold(self, into: &mut BTreeMap<String, LivedStats>) {
        for life in self.lives {
            let mut lived = Lived {
                stood: 1,
                ..Lived::default()
            };
            match &life.fate {
                None | Some(Fate::Superseded) => {}
                Some(fate) => {
                    lived.known = 1;
                    lived.value = life.value;
                    let worked = match fate {
                        Fate::Failed { .. } | Fate::Ended { .. } => false,
                        Fate::Passed | Fate::Declined => true,
                        _ => match life.choice {
                            Choice::Bid { .. } | Choice::Stock { .. } => life.fills > 0,
                            Choice::Sell { .. }
                            | Choice::Buy { .. }
                            | Choice::BuyFrom { .. }
                            | Choice::SellTo { .. }
                            | Choice::Accept
                            | Choice::Counter { .. }
                            | Choice::Pay { .. } => life.moved,
                            _ => true,
                        },
                    };
                    lived.worked = worked as usize;
                    lived.failed = matches!(fate, Fate::Failed { .. }) as usize;
                }
            }
            let stats = into.entry(life.layer).or_default();
            stats.all.add(&lived);
            if let Some(t) = life.tenth {
                stats.tenths.entry(t).or_default().add(&lived);
            }
            stats.questions.entry(life.question).or_default().add(&lived);
        }
    }
}

/// A book's price: its last trade's, else the middle of its bid and ask,
/// else whichever it has.
fn book(q: &super::Quote) -> Option<i64> {
    q.last.or(match (q.bid, q.ask) {
        (Some(b), Some(a)) => Some((b + a) / 2),
        (b, a) => b.or(a),
    })
}

/// A confidence in tenths, 0 to 9.
fn tenth(confidence: f32) -> usize {
    ((confidence * 10.0).floor().max(0.0) as usize).min(9)
}

/// An entry's kind, as the log tags it: `sell`, `make_market`.
fn entry_kind(c: &Choice) -> String {
    serde_json::to_value(c)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// A logged decision to replay.
struct Flight {
    citizen: String,
    id: u64,
    situation: Situation,
    recorded: Ruling,
    rules_only: bool,
}

/// A decision being replayed: the ruling so far, and each layer's turn.
struct Asking {
    flight: Flight,
    /// Its place in the log, to land in order
    seq: usize,
    /// The cascade it's put to
    cascade: usize,
    ruling: Ruling,
    turns: Vec<Turn>,
    waited: bool,
    began: Instant,
    /// When the layer being waited on was asked
    pended: Instant,
    /// How long the whole ruling took
    took: Duration,
}

impl Asking {
    fn start(flight: Flight, seq: usize, cascade: usize, on: &mut Cascade) -> Asking {
        let began = Instant::now();
        let ruling = on.ask(&flight.situation, began);
        Asking {
            turns: ruling.trail.clone(),
            waited: matches!(ruling.by, By::Waiting { .. }),
            ruling,
            flight,
            seq,
            cascade,
            began,
            pended: began,
            took: began.elapsed(),
        }
    }

    fn waiting(&self) -> bool {
        matches!(self.ruling.by, By::Waiting { .. })
    }

    /// Asks again after a pending layer.
    fn poll(&mut self, on: &mut Cascade) {
        self.ruling = on.ask(&self.flight.situation, Instant::now());
        self.took = self.began.elapsed();
        // Asking again is no new turn. Still thinking, it's the same one;
        // its answer, or its lateness, took as long as it was waited on,
        // and the layers above may be asked (and pend) in turn
        let mut trail = self.ruling.trail.iter().cloned();
        let Some(mut answer) = trail.next() else { return };
        if answer.asked == Asked::Pending {
            return;
        }
        if let Some(pending) = self.turns.iter().rposition(|t| t.asked == Asked::Pending) {
            answer.micros = self.pended.elapsed().as_micros() as u64;
            self.turns[pending] = answer;
        }
        self.pended = Instant::now();
        self.turns.extend(trail);
    }
}

/// One log's decisions in flight, each on a cascade of its own, landing in
/// the log's order.
struct Flights<'a> {
    make: &'a mut dyn FnMut() -> Cascade,
    most: usize,
    cascades: Vec<Cascade>,
    free: Vec<usize>,
    asking: Vec<Asking>,
    /// Ruled, waiting for those sent before them to land
    ruled: BTreeMap<usize, Asking>,
    sent: usize,
    landed: usize,
}

impl<'a> Flights<'a> {
    fn new(make: &'a mut dyn FnMut() -> Cascade, most: usize) -> Flights<'a> {
        Flights {
            make,
            most,
            cascades: Vec::new(),
            free: Vec::new(),
            asking: Vec::new(),
            ruled: BTreeMap::new(),
            sent: 0,
            landed: 0,
        }
    }

    /// Puts `flight` to a free cascade, once one is.
    fn send(&mut self, report: &mut Report, flight: Flight) {
        while self.asking.len() >= self.most {
            self.wait(report);
        }
        let cascade = self.free.pop().unwrap_or_else(|| {
            self.cascades.push((self.make)());
            self.cascades.len() - 1
        });
        let asking = Asking::start(flight, self.sent, cascade, &mut self.cascades[cascade]);
        self.sent += 1;
        self.asking.push(asking);
        self.land(report);
    }

    /// Waits a moment, then asks each one in flight again.
    fn wait(&mut self, report: &mut Report) {
        std::thread::sleep(POLL);
        for asking in &mut self.asking {
            asking.poll(&mut self.cascades[asking.cascade]);
        }
        self.land(report);
    }

    /// Waits until every one in flight has landed.
    fn drain(&mut self, report: &mut Report) {
        while !self.asking.is_empty() {
            self.wait(report);
        }
    }

    /// Frees the cascades of those ruled, and reports each that's next in
    /// the log's order.
    fn land(&mut self, report: &mut Report) {
        let mut i = 0;
        while i < self.asking.len() {
            if self.asking[i].waiting() {
                i += 1;
                continue;
            }
            let ruled = self.asking.swap_remove(i);
            self.free.push(ruled.cascade);
            self.ruled.insert(ruled.seq, ruled);
        }
        while let Some(ruled) = self.ruled.remove(&self.landed) {
            self.landed += 1;
            fold(report, ruled);
        }
    }
}

/// Adds a ruled decision to the report.
fn fold(report: &mut Report, asking: Asking) {
    let Asking {
        flight,
        ruling,
        turns,
        waited,
        took,
        ..
    } = asking;
    let Flight {
        citizen,
        id,
        situation: s,
        recorded,
        rules_only,
    } = flight;
    report.micros.push(took.as_micros() as u64);
    report.decisions += 1;
    report.waited += waited as usize;
    for turn in &turns {
        if let Some(stats) = report.layers.iter_mut().find(|l| l.name == turn.layer) {
            stats.asked += 1;
            *stats.verdicts.entry(turn.asked.as_str()).or_default() += 1;
            stats.micros.push(turn.micros);
            if let Asked::Escalated { reason } = &turn.asked {
                let mut reason = reason.clone();
                if let Some((cut, _)) = reason.char_indices().nth(REASON) {
                    reason.truncate(cut);
                }
                *stats.reasons.entry(reason).or_default() += 1;
            }
            if let Asked::Settled { confidence, choice } | Asked::Unsure { confidence, choice } = &turn.asked {
                let entry = stats.calibration.entry(tenth(*confidence)).or_default();
                entry.0 += 1;
                entry.1 += (recorded.decision.map(|d| d.pick) == Some(choice.pick)) as usize;
            }
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
        Question::Ask => "ask",
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

/// `41/50 (3) +120`: worked of known, failed, value; and the unknown.
fn lived(l: &Lived) -> String {
    let mut s = format!("{}/{}", l.worked, l.known);
    if l.failed > 0 {
        s.push_str(&format!(" ({} failed)", l.failed));
    }
    s.push_str(&format!(" {:+}", l.value));
    if l.stood > l.known {
        s.push_str(&format!(", {} unknown", l.stood - l.known));
    }
    s
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
            if !layer.calibration.is_empty() {
                let tenths: Vec<String> = layer
                    .calibration
                    .iter()
                    .rev()
                    .map(|(tenth, (n, same))| format!("{:.1}+ {same}/{n}", *tenth as f64 / 10.0))
                    .collect();
                writeln!(f, "  {:<12} as recorded, by confidence: {}", "", tenths.join(" · "))?;
            }
            if !layer.tally.is_empty() {
                let tally: Vec<String> = layer.tally.iter().map(|(k, n)| format!("{k} {n}")).collect();
                writeln!(f, "  {:<12} tally: {}", "", tally.join(" · "))?;
            }
            let mut reasons: Vec<(&String, &usize)> = layer.reasons.iter().collect();
            reasons.sort_by(|a, b| b.1.cmp(a.1));
            for (reason, n) in reasons.iter().take(4) {
                writeln!(f, "  {:<12} escalated {n}×: {reason}", "")?;
            }
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
        if !self.lived.is_empty() {
            writeln!(f, "  lived        what came of the choices that stood, as recorded: worked/known (failed) and Strands at the books")?;
        }
        for (layer, stats) in &self.lived {
            writeln!(f, "  {:<12} {}", layer, lived(&stats.all))?;
            if !stats.tenths.is_empty() {
                let tenths: Vec<String> = stats
                    .tenths
                    .iter()
                    .rev()
                    .map(|(t, l)| format!("{:.1}+ {}", *t as f64 / 10.0, lived(l)))
                    .collect();
                writeln!(f, "  {:<12} by confidence: {}", "", tenths.join(" · "))?;
            }
            for (question, l) in &stats.questions {
                writeln!(f, "  {:<12}   {question:<18} {}", "", lived(l))?;
            }
        }
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
    use super::super::{corpus, Choice, Decider, Layer, Mind, Span, Temperament, Ticket, Verdict};
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("capitol-replay-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn situations() -> Vec<Situation> {
        corpus::situations()
    }

    fn record(path: &std::path::Path, situations: &[Situation]) {
        corpus::record(path, situations).unwrap();
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
        name: &'static str,
        confidence: f32,
        asked: Option<Instant>,
    }
    impl Slow {
        fn new(name: &'static str, confidence: f32) -> Slow {
            Slow {
                name,
                confidence,
                asked: None,
            }
        }
    }
    impl Decider for Slow {
        fn name(&self) -> &str {
            self.name
        }
        fn decide(&mut self, _s: &Situation) -> Verdict {
            self.asked = Some(Instant::now());
            Verdict::Pending(Ticket(1))
        }
        fn poll(&mut self, _t: Ticket) -> Option<Verdict> {
            let ready = self.asked.is_some_and(|a| a.elapsed() >= Duration::from_millis(6));
            ready.then(|| Verdict::Decided {
                choice: Decision::plain(0),
                confidence: self.confidence,
            })
        }
    }

    /// rules → jev → a chat model: the first slow layer is unsure, so the
    /// second is asked, and pends in its turn.
    #[test]
    fn pending_layers_one_above_another_each_get_their_turn() {
        let dir = temp("slower");
        let path = log::file_for(&dir, "Percy Sawyer");
        let s = situations().into_iter().find(|s| s.question == Question::FreeTime).unwrap();
        record(&path, std::slice::from_ref(&s));
        let report = replay(&path, || {
            Cascade::new(vec![
                Layer::new(Slow::new("unsure", 0.3), 0.5, Duration::from_secs(1)),
                Layer::new(Slow::new("sure", 1.0), 0.5, Duration::from_secs(1)),
            ])
        });
        let (unsure, sure) = (&report.layers[0], &report.layers[1]);
        assert_eq!((unsure.asked, unsure.verdicts.get("unsure")), (1, Some(&1)), "{:?}", unsure.verdicts);
        assert_eq!((sure.asked, sure.verdicts.get("settled")), (1, Some(&1)), "{:?}", sure.verdicts);
        assert!(unsure.latency(0.5).unwrap() >= 6000 && sure.latency(0.5).unwrap() >= 6000);
        assert_eq!(unsure.calibration.get(&3), Some(&(1, 1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pending_layer_is_waited_on() {
        let dir = temp("slow");
        let path = log::file_for(&dir, "Percy Sawyer");
        let s = situations().into_iter().find(|s| s.question == Question::FreeTime).unwrap();
        record(&path, std::slice::from_ref(&s));
        let report = replay(&path, || Cascade::new(vec![Layer::new(Slow::new("slow", 1.0), 0.5, Duration::from_secs(1))]));
        assert_eq!(report.waited, 1);
        assert_eq!(report.first_settled, 1);
        let slow = &report.layers[0];
        // Asked once: the poll that brought its answer is the same turn,
        // as long as it was waited on
        assert_eq!(slow.asked, 1, "{:?}", slow.verdicts);
        assert_eq!(slow.verdicts["settled"], 1);
        assert!(!slow.verdicts.contains_key("pending"));
        assert!(slow.latency(0.5).unwrap() >= 6000);
        assert!(report.latency(0.5).unwrap() >= 6000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a replay found, but how long it took.
    fn findings(r: &Report) -> impl PartialEq + fmt::Debug {
        let layers: Vec<_> = r
            .layers
            .iter()
            .map(|l| (l.name.clone(), l.asked, l.verdicts.clone(), l.calibration.clone(), l.cents))
            .collect();
        (
            (r.decisions, r.same, r.same_pick, r.rules_choices, r.rules_same, r.first_settled, r.waited),
            r.questions.clone(),
            r.differences.clone(),
            r.differ,
            layers,
        )
    }

    #[test]
    fn the_rules_replay_one_at_a_time_however_many_may_fly() {
        let dir = temp("rules-flying");
        let path = log::file_for(&dir, "Percy Sawyer");
        record(&path, &situations());
        let made = Arc::new(AtomicUsize::new(0));
        let counted = made.clone();
        let mut replayer = Replayer::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            Cascade::rules()
        })
        .in_flight(8);
        replayer.log("test", log::read(&path).unwrap());
        let flying = replayer.finish();
        // One to read the stack's names, one for the log: nothing pends
        assert_eq!(made.load(Ordering::Relaxed), 2);
        assert_eq!(flying.same, flying.decisions, "{flying}");
        assert_eq!(findings(&flying), findings(&replay(&path, Cascade::rules)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A layer that thinks a while, counting how many are out at once
    /// across every cascade.
    struct Counted {
        out: Arc<(AtomicUsize, AtomicUsize)>,
        asked: Option<Instant>,
    }
    impl Decider for Counted {
        fn name(&self) -> &str {
            "counted"
        }
        fn decide(&mut self, _s: &Situation) -> Verdict {
            let now = self.out.0.fetch_add(1, Ordering::Relaxed) + 1;
            self.out.1.fetch_max(now, Ordering::Relaxed);
            self.asked = Some(Instant::now());
            Verdict::Pending(Ticket(1))
        }
        fn poll(&mut self, _t: Ticket) -> Option<Verdict> {
            let ready = self.asked.is_some_and(|a| a.elapsed() >= Duration::from_millis(6));
            ready.then(|| {
                self.out.0.fetch_sub(1, Ordering::Relaxed);
                Verdict::Decided {
                    choice: Decision::plain(0),
                    confidence: 1.0,
                }
            })
        }
    }

    #[test]
    fn a_slow_layer_is_asked_several_at_once_and_lands_in_order() {
        let dir = temp("flying");
        let path = log::file_for(&dir, "Percy Sawyer");
        let situations: Vec<Situation> = situations().into_iter().take(40).collect();
        record(&path, &situations);
        let run = |in_flight: usize| {
            let out = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
            let counted = out.clone();
            let mut replayer = Replayer::new(move || {
                let counted = Counted {
                    out: counted.clone(),
                    asked: None,
                };
                Cascade::new(vec![Layer::new(counted, 0.5, Duration::from_secs(1))])
            })
            .in_flight(in_flight);
            let began = Instant::now();
            replayer.log("test", log::read(&path).unwrap());
            (replayer.finish(), out.1.load(Ordering::Relaxed), began.elapsed())
        };
        let (one, most_one, slow) = run(1);
        let (eight, most_eight, fast) = run(8);
        assert_eq!((most_one, most_eight), (1, 8));
        assert_eq!(eight.waited, situations.len());
        assert!(eight.differ > 0, "the first entry isn't always the rules'");
        // The same report, differences in the log's order, however many flew
        assert_eq!(findings(&eight), findings(&one));
        let counted = &eight.layers[0];
        assert_eq!(counted.asked, situations.len(), "{:?}", counted.verdicts);
        assert!(counted.latency(0.5).unwrap() >= 6000, "each answer timed from its own ask");
        assert!(fast < slow, "{fast:?} with eight out, {slow:?} one at a time");
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

    /// A live log of `rules → jev`, written as the town writes it: lived
    /// reads the rulings recorded, joined to their outcomes and fills.
    #[test]
    fn lived_measures_the_choices_that_stood_by_what_came_of_them() {
        let dir = temp("lived");
        let path = log::file_for(&dir, "Percy Sawyer");
        let layers = vec!["rules".to_string(), "jev".to_string()];
        let mut log = Log::open(&path, "Percy Sawyer", "builder", &layers, log::LOG_LIMIT, log::LOG_KEEP).unwrap();
        let mut s = situations().into_iter().find(|s| s.question == Question::FreeTime).unwrap();
        s.question = Question::Buy { item: "log".into() };
        s.market = vec![super::super::Quote {
            item: "log".into(),
            bid: Some(4),
            ask: Some(8),
            last: None,
        }];
        s.menu = vec![
            Choice::Bid {
                item: "log".into(),
                price: Span::new(1, 9),
                quantity: Span::new(1, 9),
            },
            Choice::Gather { item: "log".into() },
        ];
        s.carry_on = Some(1);
        let turn = |layer: &str, asked: Asked| Turn {
            layer: layer.into(),
            asked,
            micros: 1,
        };
        let bid = Decision {
            pick: 0,
            price: Some(5),
            quantity: Some(3),
        };
        // The rules aren't sure, Jev thinks it over, then bids at 0.95
        let waiting = Ruling {
            decision: Some(Decision::plain(1)),
            by: By::Waiting { name: "jev".into() },
            trail: vec![
                turn(
                    "rules",
                    Asked::Unsure {
                        confidence: 0.5,
                        choice: Decision::plain(1),
                    },
                ),
                turn("jev", Asked::Pending),
            ],
        };
        let answered = Ruling {
            decision: Some(bid),
            by: By::Layer { name: "jev".into() },
            trail: vec![turn("jev", Asked::Settled { confidence: 0.95, choice: bid })],
        };
        let carried = log.decision(&s, &waiting);
        let id = log.decision(&s, &answered);
        let outcome = |id, fate, strands| Record::Outcome {
            t: 1,
            id,
            fate,
            took: 1.0,
            strands,
            items: BTreeMap::new(),
        };
        log.write(&outcome(carried, Fate::Passed, 0));
        // The escrow left its Strands: that isn't what the bid came to
        log.write(&outcome(id, Fate::Done, -15));
        log.write(&Record::Fill {
            t: 2,
            id,
            quantity: 2,
            strands: 10,
        });
        // Then Jev's 0.7 offer stands over the rules' 0.5, and fails
        let mut offered = answered.clone();
        let gather = Decision::plain(1);
        offered.by = By::Offered { name: "jev".into() };
        offered.decision = Some(gather);
        offered.trail = vec![
            turn(
                "rules",
                Asked::Unsure {
                    confidence: 0.5,
                    choice: Decision::plain(1),
                },
            ),
            turn("jev", Asked::Unsure { confidence: 0.7, choice: gather }),
        ];
        let failed = log.decision(&s, &offered);
        log.write(&outcome(failed, Fate::Failed { why: "no logs".into() }, 0));
        // A rules' pick that a trade went before
        let settled = Ruling {
            decision: Some(gather),
            by: By::Layer { name: "rules".into() },
            trail: vec![turn("rules", Asked::Settled { confidence: 0.8, choice: gather })],
        };
        let superseded = log.decision(&s, &settled);
        log.write(&outcome(superseded, Fate::Superseded, 0));
        drop(log);
        let report = replay(&path, Cascade::rules);
        let jev = &report.lived["jev"];
        assert_eq!((jev.all.stood, jev.all.known, jev.all.worked, jev.all.failed), (2, 2, 1, 1), "{report}");
        // Two logs at the books' 6, for 10
        assert_eq!(jev.tenths[&9].value, 2);
        assert_eq!(jev.tenths[&9].worked, 1);
        assert_eq!((jev.tenths[&7].worked, jev.tenths[&7].failed), (0, 1));
        assert_eq!(jev.questions["buy: bid"].worked, 1);
        assert_eq!(jev.questions["buy: gather"].failed, 1);
        // The carry-on while Jev thought isn't anyone's choice
        let rules = &report.lived["rules"];
        assert_eq!((rules.all.stood, rules.all.known), (1, 0));
        let text = report.to_string();
        assert!(text.contains("lived") && text.contains("0.9+ 1/1 +2"), "{text}");
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
