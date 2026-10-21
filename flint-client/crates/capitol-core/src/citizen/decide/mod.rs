//! The decision seam (C1.3, plan D4): how a citizen makes up its mind.
//!
//! Layers never steer the avatar. Walking, gathering and crafting are
//! [`Bot`](crate::bot::Bot)'s and the planner's. At a *decision point* the
//! citizen's loop builds a [`Situation`]: who it is, where, when, what it
//! holds, the market as it sees it, the [`Question`], and the **menu** of
//! legal [`Choice`]s. A layer answers with one entry of the menu and its
//! parameters filled in (a [`Decision`]), or escalates. Nothing off the menu
//! is ever acted on: [`Situation::admits`] turns it into an escalation.
//!
//! A [`Cascade`] is a stack of [`Layer`]s, each a [`Decider`] with a
//! confidence threshold, a time budget and a spend budget:
//!
//! - **Order.** Each layer is asked in turn. A `Decided` below its threshold,
//!   an `Escalate`, an off-menu answer, a late answer or a spent budget
//!   passes the question to the next layer.
//! - **Fallback.** If no layer settles it: the most confident on-menu answer
//!   any layer offered, then [`RulesDecider`]'s pick, then the menu's
//!   *carry on* entry. A citizen is never stuck waiting on a model.
//! - **Async.** A layer may answer `Pending`. The cascade answers *carry on*
//!   for now, and the next time the same question comes up it asks the layer
//!   for its answer: on time, it's weighed like any other; late, the fallback
//!   stands. The world never waits on a model.
//!
//! [`Situation`], [`Choice`], [`Decision`] and [`Ruling`] are serde types,
//! so a layer can be a function, a subprocess or an HTTP call, and C3.1's
//! log can keep them as JSONL.
//!
//! - [`rules`]: the rules layer, a utility pick weighted by temperament.
//! - [`log`]: the decision log, each decision and what came of it (C3.1).
//! - [`replay`]: any stack of layers run over a log offline (C3.1).
//! - [`jev`]: Jev, TypeSafe's choice model, as the fast tier (C3.3).
//! - [`chat`]: a chat model through an OpenAI-compatible API, such as
//!   OpenRouter's `jev-router` (C3.3).
//! - [`remote`]: what the layers that ask over the network share.
//! - [`corpus`]: a town's worth of situations, made up, to replay (C3.3).

pub mod chat;
pub mod corpus;
pub mod jev;
pub mod log;
pub mod remote;
pub mod replay;
pub mod rules;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::sim::plan::Holdings;
use log::{Fate, Log, Record};
pub use rules::RulesDecider;

// ---------------------------------------------------------------------------
// The situation
// ---------------------------------------------------------------------------

/// What a citizen is like (D5). Each is 0 to 1, 0.5 being middling. The
/// rules layer weights its utilities by them, and a model layer's prompt
/// will be coloured by them. Each citizen's is seeded with its persona
/// (C1.2, [`persona`](super::persona)).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Temperament {
    /// Holds out for a fair price, and gathers rather than overpay. Keeps
    /// more back before it buys a home (C1.2).
    pub thrift: f32,
    /// Asks above the bid, hoping someone pays up.
    pub risk: f32,
    /// Spends its free time among people, at the hall. Stays longer at
    /// the market, and likelier keeps the bazaar (C1.2).
    pub sociability: f32,
    /// Sells and banks in bigger lots, less often.
    pub patience: f32,
}

impl Default for Temperament {
    fn default() -> Self {
        Temperament {
            thrift: 0.5,
            risk: 0.5,
            sociability: 0.5,
            patience: 0.5,
        }
    }
}

/// Everything a layer knows when it decides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Situation {
    pub who: Who,
    pub at: Place,
    pub when: When,
    pub holdings: Held,
    /// The books the citizen watches.
    pub market: Vec<Quote>,
    pub question: Question,
    /// The legal answers. A layer picks one by its index.
    pub menu: Vec<Choice>,
    /// The entry that means "carry on with what you were doing": what the
    /// cascade answers while a layer thinks, and last of all.
    pub carry_on: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Who {
    pub name: String,
    /// [`Role::slug`](crate::sim::Role::slug)
    pub role: String,
    pub temperament: Temperament,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub x: f64,
    pub y: f64,
    /// [`Zone::as_str`](capitol_protocol::rules::Zone::as_str)
    pub zone: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct When {
    /// The server's time of day: 0 midnight, 0.5 noon.
    pub time_of_day: Option<f64>,
    /// Game hours since this life began.
    pub hours: f64,
}

/// What the citizen holds: [`Holdings`] with its sets sorted, so a logged
/// situation reads the same each time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Held {
    pub strands: i64,
    pub items: BTreeMap<String, i32>,
    pub tools: BTreeSet<String>,
    pub stations: BTreeSet<String>,
    /// Room for more stations on its plot, if it has one.
    pub plot_room: Option<i32>,
    /// What a deed costs where it would buy one.
    pub deed_cost: i64,
}

impl Held {
    pub fn count(&self, item: &str) -> i32 {
        self.items.get(item).copied().unwrap_or(0)
    }
}

impl From<&Holdings> for Held {
    fn from(h: &Holdings) -> Held {
        Held {
            strands: h.strands,
            items: h.items.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            tools: h.tools.iter().cloned().collect(),
            stations: h.stations.iter().cloned().collect(),
            plot_room: h.plot_room,
            deed_cost: h.deed_cost,
        }
    }
}

/// One item's book as the citizen sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub item: String,
    pub bid: Option<i64>,
    pub ask: Option<i64>,
    /// The last trade's price.
    pub last: Option<i64>,
}

/// Units of one item, in a P2P offer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lot {
    pub item: String,
    pub quantity: i32,
}

/// What's being decided: the five decision points of D4.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Question {
    /// What to work on next.
    Work,
    /// Whether to sell `item` now, and at what price.
    Sell { item: String },
    /// Whether to buy `item` rather than gather it.
    Buy { item: String },
    /// Whether to take a P2P trade as it stands, counter it, or decline;
    /// or, for goods offered, what to pay. Asked when a citizen answers
    /// someone's trade (C1.5, [`meet`](crate::citizen::meet)).
    Trade {
        partner: String,
        partner_citizen: bool,
        /// What they give
        theirs: Vec<Lot>,
        their_strands: i64,
        /// What we give
        ours: Vec<Lot>,
        our_strands: i64,
    },
    /// What to do with time on its hands: where to spend it (C2.1).
    FreeTime,
    /// Whether a merchant makes a market with its float now, or goes on
    /// earning it (C1.6).
    Market,
}

impl Question {
    /// What a pending answer is filed under: one question of each kind,
    /// and item, at a time.
    pub fn key(&self) -> String {
        match self {
            Question::Work => "work".into(),
            Question::Sell { item } => format!("sell {item}"),
            Question::Buy { item } => format!("buy {item}"),
            Question::Trade { partner, .. } => format!("trade {partner}"),
            Question::FreeTime => "free time".into(),
            Question::Market => "market".into(),
        }
    }
}

/// A range a parameter may be set in, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span<T> {
    pub min: T,
    pub max: T,
}

impl<T: PartialOrd + Copy> Span<T> {
    pub fn new(min: T, max: T) -> Span<T> {
        Span { min, max }
    }

    pub fn holds(&self, v: T) -> bool {
        self.min <= v && v <= self.max
    }

    /// `v`, brought inside the span.
    pub fn clamp(&self, v: T) -> T {
        if v < self.min {
            self.min
        } else if v > self.max {
            self.max
        } else {
            v
        }
    }
}

/// One entry on a menu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Choice {
    /// Work toward `goal`, whose next step is `step` (labels as the sim's
    /// report files them: `more plank`, `craft plank`).
    Work { goal: String, step: String },
    /// Take the fiber to the exchange for Strands.
    Bank,
    /// Put `item` up for sale at the hall. Needs a price and a quantity.
    Sell {
        item: String,
        price: Span<i64>,
        quantity: Span<i32>,
    },
    /// Bid for `item` at the hall. Needs a price and a quantity.
    Buy {
        item: String,
        price: Span<i64>,
        quantity: Span<i32>,
    },
    /// Leave a standing bid for `item` at the hall, which anyone may sell
    /// into while the citizen works (C1.5's demand). Needs a price and a
    /// quantity.
    Bid {
        item: String,
        price: Span<i64>,
        quantity: Span<i32>,
    },
    /// Stock its stall at the bazaar with `item`, renting one first if it
    /// keeps none (C1.5). Needs a price and a quantity.
    Stock {
        item: String,
        price: Span<i64>,
        quantity: Span<i32>,
    },
    /// Don't sell: keep what it holds.
    Keep,
    /// Don't buy: go and gather it.
    Gather { item: String },
    /// Accept the trade as it stands.
    Accept,
    /// Ask for a deal this many Strands better. Needs a price: how many.
    /// There's no chat, so a citizen says it with its own side of the
    /// window (C1.5): it takes that much off what it gives, Strands first,
    /// then goods at its books. "For that, you get this."
    Counter { strands: Span<i64> },
    /// Put up this many Strands for the goods they offer. Needs a price.
    Pay { strands: Span<i64> },
    Decline,
    /// Sit out a while where it is.
    Rest,
    /// Pass the time at the Exchange hall, among people: on its steps
    /// (C2.1).
    Hall,
    /// Make a market in `items` at the hall (C1.6): quote both sides of
    /// each, sized to what it holds. `float` is what it has to do it with,
    /// in Strands: its Strands, those on its bids included, and its stock
    /// at its books. `making` says it's at it already.
    MakeMarket { items: Vec<String>, float: i64, making: bool },
    /// Stroll the bazaar's lane, among the stalls (C2.1).
    Lane,
    /// Sit a while in the park `park` (C2.1).
    Park { park: String },
    /// Stand a while on the bridge `bridge` at dusk, looking down the water
    /// (C2.1).
    Bridge { bridge: String },
    /// Mind its own stall, `stall` (C2.1).
    Stall { stall: String },
    /// Go straight home at dusk (C2.1).
    Home,
}

impl Choice {
    /// The parameters it takes: (price, quantity).
    fn takes(&self) -> (Option<Span<i64>>, Option<Span<i32>>) {
        match self {
            Choice::Sell { price, quantity, .. }
            | Choice::Buy { price, quantity, .. }
            | Choice::Bid { price, quantity, .. }
            | Choice::Stock { price, quantity, .. } => (Some(*price), Some(*quantity)),
            Choice::Counter { strands } | Choice::Pay { strands } => (Some(*strands), None),
            _ => (None, None),
        }
    }
}

/// A layer's answer: an entry on the menu, by index, with its parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub pick: usize,
    /// A sale's or a bid's price per unit, or a counter's extra Strands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i32>,
}

impl Decision {
    /// An entry that takes no parameters.
    pub fn plain(pick: usize) -> Decision {
        Decision {
            pick,
            price: None,
            quantity: None,
        }
    }
}

impl Situation {
    /// Whether `d` is on the menu: a real entry, with exactly the parameters
    /// it takes, each inside its span. Why not, if it isn't.
    pub fn admits(&self, d: &Decision) -> Result<(), String> {
        let choice = self
            .menu
            .get(d.pick)
            .ok_or_else(|| format!("pick {} is off a menu of {}", d.pick, self.menu.len()))?;
        let (price, quantity) = choice.takes();
        match (price, d.price) {
            (None, None) => {}
            (Some(span), Some(p)) if span.holds(p) => {}
            (Some(span), Some(p)) => return Err(format!("price {p} is outside {}..={}", span.min, span.max)),
            (Some(_), None) => return Err("it needs a price".into()),
            (None, Some(_)) => return Err("it takes no price".into()),
        }
        match (quantity, d.quantity) {
            (None, None) => Ok(()),
            (Some(span), Some(q)) if span.holds(q) => Ok(()),
            (Some(span), Some(q)) => Err(format!("quantity {q} is outside {}..={}", span.min, span.max)),
            (Some(_), None) => Err("it needs a quantity".into()),
            (None, Some(_)) => Err("it takes no quantity".into()),
        }
    }

    /// The entry `d` picks. Only for a decision [`admits`](Self::admits) took.
    pub fn choice(&self, d: &Decision) -> &Choice {
        &self.menu[d.pick]
    }
}

// ---------------------------------------------------------------------------
// Deciders
// ---------------------------------------------------------------------------

/// An async layer's receipt for a question it will answer later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket(pub u64);

/// What a layer says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Decided { choice: Decision, confidence: f32 },
    Escalate { reason: String },
    /// It will answer later: ask [`Decider::poll`].
    Pending(Ticket),
}

/// One layer's brain.
pub trait Decider: Send {
    /// What `town.toml` calls it: `rules`.
    fn name(&self) -> &str;

    fn decide(&mut self, s: &Situation) -> Verdict;

    /// The answer to an earlier `Pending`, once there is one.
    fn poll(&mut self, _ticket: Ticket) -> Option<Verdict> {
        None
    }

    /// What it has cost so far, in US cents. Rules cost nothing.
    fn spent(&self) -> f64 {
        0.0
    }

    /// Whatever it counts of itself, for a replay's report: the models a
    /// router sent its questions to, say.
    fn tally(&self) -> BTreeMap<String, usize> {
        BTreeMap::new()
    }
}

/// A [`Decider`] in a cascade, with what it must meet to settle a question.
pub struct Layer {
    pub decider: Box<dyn Decider>,
    /// A `Decided` below this is offered, not settled.
    pub threshold: f32,
    /// How long an answer may take: a slower one counts as an escalation,
    /// and a `Pending` that outlasts it is late.
    pub time: Duration,
    /// Cents the layer may spend in all. Once spent, it isn't asked.
    pub spend: Option<f64>,
}

impl Layer {
    pub fn new(decider: impl Decider + 'static, threshold: f32, time: Duration) -> Layer {
        Layer {
            decider: Box::new(decider),
            threshold,
            time,
            spend: None,
        }
    }

    /// With a spend budget of `cents`.
    pub fn spending(mut self, cents: f64) -> Layer {
        self.spend = Some(cents);
        self
    }
}

/// The rules layer's threshold in a stack: a close call goes up a layer.
pub const RULES_THRESHOLD: f32 = 0.6;
/// The rules layer's time budget. It answers in microseconds.
pub const RULES_TIME: Duration = Duration::from_millis(50);
/// Jev's threshold in a stack (C3.3): its answers at 0.9 and over agreed
/// with the rules 94% of the time on the corpus, and those under it about
/// half (the plan's C3.3 report).
pub const JEV_THRESHOLD: f32 = 0.9;
/// Jev's time budget: TypeSafe claims 70 to 500 ms.
pub const JEV_TIME: Duration = Duration::from_secs(2);
/// A chat model's threshold in a stack: how sure of its letter it must be.
pub const CHAT_THRESHOLD: f32 = 0.8;
/// A chat model's time budget.
pub const CHAT_TIME: Duration = Duration::from_secs(10);
/// What a remote layer may spend for one citizen (or one replayed log), in
/// US cents, before it's no longer asked.
pub const REMOTE_SPEND: f64 = 100.0;
/// The layers a town or citizen has unless its roster says otherwise.
pub const DEFAULT_LAYERS: [&str; 1] = ["rules"];
/// Every layer a roster may name. A name may carry its own threshold:
/// `jev@0.8`.
pub const KNOWN_LAYERS: [&str; 3] = ["rules", "jev", "jev-router"];

// ---------------------------------------------------------------------------
// The cascade
// ---------------------------------------------------------------------------

/// How the question was settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum By {
    /// A layer answered over its threshold.
    Layer { name: String },
    /// No layer did; this is the most confident answer one offered.
    Offered { name: String },
    /// No layer offered anything on the menu: the rules' own pick.
    Rules,
    /// A layer is still thinking; carry on meanwhile.
    Waiting { name: String },
    /// Nothing else answered.
    CarryOn,
}

/// What happened when a layer was asked, for the log (C3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Asked {
    Settled { confidence: f32, choice: Decision },
    /// Decided, but under the threshold.
    Unsure { confidence: f32, choice: Decision },
    Escalated { reason: String },
    OffMenu { why: String, choice: Decision },
    /// It answered after its time budget.
    Slow { millis: u64 },
    /// Its spend budget is gone.
    Broke,
    Pending,
    /// Its pending answer didn't come in time.
    Late,
}

impl Asked {
    /// For a report: `settled`, `off menu`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Asked::Settled { .. } => "settled",
            Asked::Unsure { .. } => "unsure",
            Asked::Escalated { .. } => "escalated",
            Asked::OffMenu { .. } => "off menu",
            Asked::Slow { .. } => "slow",
            Asked::Broke => "broke",
            Asked::Pending => "pending",
            Asked::Late => "late",
        }
    }
}

/// One layer's turn at a question: who, what came of it, and how long it
/// took to answer (or to say it was still thinking).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub layer: String,
    #[serde(flatten)]
    pub asked: Asked,
    pub micros: u64,
}

impl Turn {
    fn new(layer: impl Into<String>, asked: Asked, took: Duration) -> Turn {
        Turn {
            layer: layer.into(),
            asked,
            micros: took.as_micros().min(u64::MAX as u128) as u64,
        }
    }
}

/// The cascade's answer, and how it got there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ruling {
    /// `None` only for an empty menu.
    pub decision: Option<Decision>,
    pub by: By,
    /// Each layer asked, in order, and what came of it.
    pub trail: Vec<Turn>,
}

/// A question a layer is still thinking over.
struct Waiting {
    key: String,
    layer: usize,
    ticket: Ticket,
    until: Instant,
    /// What the layers below it offered.
    offered: Vec<(Decision, f32, String)>,
}

/// A citizen's stack of layers (D4).
pub struct Cascade {
    layers: Vec<Layer>,
    /// Asked when no layer settled or offered anything.
    rules: RulesDecider,
    waiting: Vec<Waiting>,
}

impl Cascade {
    pub fn new(layers: Vec<Layer>) -> Cascade {
        Cascade {
            layers,
            rules: RulesDecider,
            waiting: Vec::new(),
        }
    }

    /// The rules layer alone.
    pub fn rules() -> Cascade {
        Cascade::new(vec![Layer::new(RulesDecider, RULES_THRESHOLD, RULES_TIME)])
    }

    /// The stack a roster names, bottom first: `["rules"]`, or
    /// `["rules", "jev@0.8"]` to set a layer's threshold.
    pub fn from_names(names: &[String]) -> Result<Cascade, String> {
        check_layers(names)?;
        let layers = names
            .iter()
            .map(|name| {
                let (name, threshold) = layer_name(name).expect("check_layers read it");
                let mut layer = match name {
                    "rules" => Layer::new(RulesDecider, RULES_THRESHOLD, RULES_TIME),
                    "jev" => Layer::new(jev::JevDecider::from_env(), JEV_THRESHOLD, JEV_TIME).spending(REMOTE_SPEND),
                    "jev-router" => Layer::new(chat::ChatDecider::jev_router(), CHAT_THRESHOLD, CHAT_TIME).spending(REMOTE_SPEND),
                    _ => unreachable!("check_layers knows every layer"),
                };
                if let Some(threshold) = threshold {
                    layer.threshold = threshold;
                }
                layer
            })
            .collect();
        Ok(Cascade::new(layers))
    }

    /// Settles `s`'s question. `now` is when it was asked, which a pending
    /// layer's time budget runs from.
    pub fn ask(&mut self, s: &Situation, now: Instant) -> Ruling {
        let key = s.question.key();
        let mut trail = Vec::new();
        // A pending answer outlasts its budget only until the same question
        // comes round again; others that went stale are dropped.
        self.waiting.retain(|w| w.key == key || now < w.until);
        let mut offered = Vec::new();
        let mut from = 0;
        if let Some(i) = self.waiting.iter().position(|w| w.key == key) {
            let w = self.waiting.swap_remove(i);
            let layer = &mut self.layers[w.layer];
            let name = layer.decider.name().to_string();
            let asked = Instant::now();
            let answer = layer.decider.poll(w.ticket);
            let took = asked.elapsed();
            match answer {
                None if now < w.until => {
                    trail.push(Turn::new(&name, Asked::Pending, took));
                    let decision = self.carry_on(s, &w.offered);
                    self.waiting.push(w);
                    return Ruling {
                        decision,
                        by: By::Waiting { name },
                        trail,
                    };
                }
                // Too late: fall back on what was offered before it
                None => {
                    trail.push(Turn::new(name, Asked::Late, took));
                    return self.fall_back(s, w.offered, trail);
                }
                Some(verdict) => {
                    offered = w.offered;
                    match weigh(s, layer, verdict, took, &mut offered, &mut trail) {
                        Weighed::Settled(d) => return settled(d, name, trail),
                        // A pending answer can't pend again
                        Weighed::Pending(_) | Weighed::Passed => from = w.layer + 1,
                    }
                }
            }
        }
        for i in from..self.layers.len() {
            let layer = &mut self.layers[i];
            let name = layer.decider.name().to_string();
            if layer.spend.is_some_and(|limit| layer.decider.spent() >= limit) {
                trail.push(Turn::new(name, Asked::Broke, Duration::ZERO));
                continue;
            }
            let asked = Instant::now();
            let verdict = layer.decider.decide(s);
            let took = asked.elapsed();
            if took > layer.time && !matches!(verdict, Verdict::Pending(_)) {
                trail.push(Turn::new(name, Asked::Slow { millis: took.as_millis() as u64 }, took));
                continue;
            }
            match weigh(s, layer, verdict, took, &mut offered, &mut trail) {
                Weighed::Settled(d) => return settled(d, name, trail),
                Weighed::Passed => {}
                Weighed::Pending(ticket) => {
                    let decision = self.carry_on(s, &offered);
                    self.waiting.push(Waiting {
                        key,
                        layer: i,
                        ticket,
                        until: now + self.layers[i].time,
                        offered,
                    });
                    return Ruling {
                        decision,
                        by: By::Waiting { name },
                        trail,
                    };
                }
            }
        }
        self.fall_back(s, offered, trail)
    }

    /// Whether a layer is still thinking over `question`.
    pub fn is_waiting(&self, question: &Question) -> bool {
        let key = question.key();
        self.waiting.iter().any(|w| w.key == key)
    }

    /// Each layer's name, bottom first.
    pub fn names(&self) -> Vec<String> {
        self.layers.iter().map(|l| l.decider.name().to_string()).collect()
    }

    /// What each layer has tallied of itself, bottom first.
    pub fn tallies(&self) -> Vec<BTreeMap<String, usize>> {
        self.layers.iter().map(|l| l.decider.tally()).collect()
    }

    /// What each layer has cost so far, in US cents, bottom first.
    pub fn spent(&self) -> Vec<(String, f64)> {
        self.layers
            .iter()
            .map(|l| (l.decider.name().to_string(), l.decider.spent()))
            .collect()
    }

    /// No layer settled it: the best offered, else the rules' pick, else
    /// carry on.
    fn fall_back(&mut self, s: &Situation, offered: Vec<(Decision, f32, String)>, trail: Vec<Turn>) -> Ruling {
        if let Some((d, _, name)) = best(offered) {
            return Ruling {
                decision: Some(d),
                by: By::Offered { name },
                trail,
            };
        }
        if let Verdict::Decided { choice, .. } = self.rules.decide(s) {
            if s.admits(&choice).is_ok() {
                return Ruling {
                    decision: Some(choice),
                    by: By::Rules,
                    trail,
                };
            }
        }
        Ruling {
            decision: s.carry_on.filter(|&i| i < s.menu.len()).map(Decision::plain),
            by: By::CarryOn,
            trail,
        }
    }

    /// What to do while a layer thinks: carry on, or if the menu has no
    /// such entry, what the fallback would do.
    fn carry_on(&mut self, s: &Situation, offered: &[(Decision, f32, String)]) -> Option<Decision> {
        match s.carry_on.filter(|&i| i < s.menu.len()) {
            Some(i) => Some(Decision::plain(i)),
            None => self.fall_back(s, offered.to_vec(), Vec::new()).decision,
        }
    }
}

/// Every name in `names` is a layer, and there's at least one.
pub fn check_layers(names: &[String]) -> Result<(), String> {
    if names.is_empty() {
        return Err("a citizen needs at least one layer".into());
    }
    names.iter().try_for_each(|n| layer_name(n).map(|_| ()))
}

/// A layer's name, and the threshold it was given, if any: `jev@0.8`.
fn layer_name(name: &str) -> Result<(&str, Option<f32>), String> {
    let (layer, threshold) = match name.split_once('@') {
        None => (name, None),
        Some((layer, t)) => match t.parse::<f32>() {
            Ok(t) if (0.0..=1.0).contains(&t) => (layer, Some(t)),
            _ => return Err(format!("{name:?}: a threshold is a number from 0 to 1")),
        },
    };
    if !KNOWN_LAYERS.contains(&layer) {
        return Err(format!("{layer:?} is no layer; a layer is one of {}", KNOWN_LAYERS.join(", ")));
    }
    Ok((layer, threshold))
}

enum Weighed {
    Settled(Decision),
    Pending(Ticket),
    /// To the next layer.
    Passed,
}

/// What a layer's verdict comes to, noting it on the trail.
fn weigh(
    s: &Situation,
    layer: &Layer,
    verdict: Verdict,
    took: Duration,
    offered: &mut Vec<(Decision, f32, String)>,
    trail: &mut Vec<Turn>,
) -> Weighed {
    let name = layer.decider.name().to_string();
    match verdict {
        Verdict::Decided { choice, confidence } => {
            if let Err(why) = s.admits(&choice) {
                trail.push(Turn::new(name, Asked::OffMenu { why, choice }, took));
                Weighed::Passed
            } else if confidence >= layer.threshold {
                trail.push(Turn::new(name, Asked::Settled { confidence, choice }, took));
                Weighed::Settled(choice)
            } else {
                trail.push(Turn::new(&name, Asked::Unsure { confidence, choice }, took));
                offered.push((choice, confidence, name));
                Weighed::Passed
            }
        }
        Verdict::Escalate { reason } => {
            trail.push(Turn::new(name, Asked::Escalated { reason }, took));
            Weighed::Passed
        }
        Verdict::Pending(ticket) => {
            trail.push(Turn::new(name, Asked::Pending, took));
            Weighed::Pending(ticket)
        }
    }
}

fn settled(decision: Decision, name: String, trail: Vec<Turn>) -> Ruling {
    Ruling {
        decision: Some(decision),
        by: By::Layer { name },
        trail,
    }
}

/// The most confident offer; the lowest layer's, on a tie.
fn best(offered: Vec<(Decision, f32, String)>) -> Option<(Decision, f32, String)> {
    offered.into_iter().fold(None, |best, o| match best {
        Some(b) if b.1 >= o.1 => Some(b),
        _ => Some(o),
    })
}

/// A citizen's decision log, shared by its lives (C3.1).
pub type SharedLog = Arc<Mutex<Log>>;

/// The most decisions a mind waits on outcomes for. Past it the oldest is
/// forgotten: its outcome goes unrecorded.
const OPEN_MAX: usize = 64;
/// The most standing bids a mind keeps track of, for their fills.
const ORDERS_MAX: usize = 64;

/// A logged decision whose outcome isn't known yet.
struct Open {
    id: u64,
    choice: Choice,
    asked: Instant,
    held: Held,
}

/// A citizen's mind: its temperament, and the cascade that decides for it.
/// With a log (C3.1), it writes each decision down, and what came of it as
/// its loop says ([`settle`](Self::settle), [`placed`](Self::placed),
/// [`stocked`](Self::stocked)).
pub struct Mind {
    pub temperament: Temperament,
    cascade: Cascade,
    log: Option<SharedLog>,
    /// Game seconds to a real one, for an outcome's time
    scale: f64,
    open: Vec<Open>,
    /// The last decision logged
    last: Option<u64>,
    /// Standing bids, by order, and the decision each came of
    orders: HashMap<Uuid, u64>,
    /// Stalls stocked, by item, and the decision that last stocked each
    stalls: HashMap<String, u64>,
}

impl Mind {
    pub fn new(cascade: Cascade, temperament: Temperament) -> Mind {
        Mind {
            temperament,
            cascade,
            log: None,
            scale: 1.0,
            open: Vec::new(),
            last: None,
            orders: HashMap::new(),
            stalls: HashMap::new(),
        }
    }

    /// The rules alone, middling in everything: the sim's bots.
    pub fn rules() -> Mind {
        Mind::new(Cascade::rules(), Temperament::default())
    }

    /// Writes its decisions and their outcomes to `log`.
    pub fn with_log(mut self, log: SharedLog) -> Mind {
        self.log = Some(log);
        self
    }

    /// The server's `SIM_TICK_SCALE`: an outcome's time is in game seconds.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
    }

    pub fn ask(&mut self, s: &Situation) -> Ruling {
        let ruling = self.cascade.ask(s, Instant::now());
        if let Some(log) = &self.log {
            let id = lock(log).decision(s, &ruling);
            self.last = Some(id);
            if let Some(d) = ruling.decision.filter(|d| d.pick < s.menu.len()) {
                if self.open.len() >= OPEN_MAX {
                    self.open.remove(0);
                }
                self.open.push(Open {
                    id,
                    choice: s.choice(&d).clone(),
                    asked: Instant::now(),
                    held: s.holdings.clone(),
                });
            }
        }
        ruling
    }

    /// How many decisions wait on their outcome: a mark to
    /// [`settle`](Self::settle) from.
    pub fn mark(&self) -> usize {
        self.open.len()
    }

    /// Logs what came of every decision since `mark`: `fate` of the entry
    /// each picked, with the change in what it holds, from its situation's
    /// holdings to `after`, measured the same way.
    pub fn settle(&mut self, mark: usize, after: &Held, fate: impl Fn(&Choice) -> Fate) {
        if mark >= self.open.len() {
            return;
        }
        let settled: Vec<Open> = self.open.drain(mark..).collect();
        let Some(log) = &self.log else { return };
        let mut log = lock(log);
        let t = log::now_ms();
        for open in settled {
            let fate = fate(&open.choice);
            let (strands, items) = match fate {
                // Nothing was done on its account
                Fate::Passed | Fate::Superseded => Default::default(),
                _ => log::change(&open.held, after),
            };
            log.write(&Record::Outcome {
                t,
                id: open.id,
                fate,
                took: open.asked.elapsed().as_secs_f64() * self.scale,
                strands,
                items,
            });
        }
    }

    /// The last decision put up a standing bid, `order`: its fills are
    /// logged against it.
    pub fn placed(&mut self, order: Uuid) {
        if let Some(id) = self.last {
            if self.orders.len() >= ORDERS_MAX {
                self.orders.clear();
            }
            self.orders.insert(order, id);
        }
    }

    /// The last decision stocked a stall with `item`: its sales are logged
    /// against it.
    pub fn stocked(&mut self, item: &str) {
        if let Some(id) = self.last {
            self.stalls.insert(item.to_string(), id);
        }
    }

    /// A standing bid filled: `quantity` for `strands`. `done` once nothing
    /// of it is left on the book.
    pub fn filled(&mut self, order: Uuid, quantity: i64, strands: i64, done: bool) {
        let id = if done { self.orders.remove(&order) } else { self.orders.get(&order).copied() };
        if let Some(id) = id {
            self.fill(id, quantity, strands);
        }
    }

    /// A standing bid left the book unfilled.
    pub fn closed(&mut self, order: Uuid) {
        self.orders.remove(&order);
    }

    /// The stall sold `quantity` of `item` for `strands`.
    pub fn sold(&mut self, item: &str, quantity: i64, strands: i64) {
        if let Some(&id) = self.stalls.get(item) {
            self.fill(id, quantity, strands);
        }
    }

    fn fill(&self, id: u64, quantity: i64, strands: i64) {
        if let Some(log) = &self.log {
            lock(log).write(&Record::Fill {
                t: log::now_ms(),
                id,
                quantity,
                strands,
            });
        }
    }
}

fn lock(log: &SharedLog) -> std::sync::MutexGuard<'_, Log> {
    log.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A situation with a work menu of `n` entries, carrying on with the first.
    fn situation(n: usize) -> Situation {
        Situation {
            who: Who {
                name: "Ada Reeder".into(),
                role: "gatherer-wood".into(),
                temperament: Temperament::default(),
            },
            at: Place {
                x: 0.0,
                y: 0.0,
                zone: "rural".into(),
            },
            when: When {
                time_of_day: Some(0.5),
                hours: 1.0,
            },
            holdings: Held::default(),
            market: Vec::new(),
            question: Question::Work,
            menu: (0..n)
                .map(|i| Choice::Work {
                    goal: format!("goal {i}"),
                    step: format!("step {i}"),
                })
                .collect(),
            carry_on: Some(0),
        }
    }

    /// A layer that says what it's told, and notes that it was asked.
    struct Scripted {
        name: &'static str,
        says: Verdict,
        asked: Arc<Mutex<Vec<&'static str>>>,
        /// What `poll` answers, once set.
        later: Arc<Mutex<Option<Verdict>>>,
        cost: f64,
        sleep: Duration,
    }

    impl Decider for Scripted {
        fn name(&self) -> &str {
            self.name
        }
        fn decide(&mut self, _s: &Situation) -> Verdict {
            self.asked.lock().unwrap().push(self.name);
            std::thread::sleep(self.sleep);
            self.says.clone()
        }
        fn poll(&mut self, _ticket: Ticket) -> Option<Verdict> {
            self.later.lock().unwrap().take()
        }
        fn spent(&self) -> f64 {
            self.cost
        }
    }

    struct Stack {
        asked: Arc<Mutex<Vec<&'static str>>>,
        later: Arc<Mutex<Option<Verdict>>>,
        layers: Vec<Layer>,
    }

    impl Stack {
        fn new() -> Stack {
            Stack {
                asked: Arc::default(),
                later: Arc::default(),
                layers: Vec::new(),
            }
        }
        fn layer(mut self, name: &'static str, says: Verdict, threshold: f32) -> Stack {
            self.layers.push(Layer::new(
                Scripted {
                    name,
                    says,
                    asked: self.asked.clone(),
                    later: self.later.clone(),
                    cost: 0.0,
                    sleep: Duration::ZERO,
                },
                threshold,
                Duration::from_secs(1),
            ));
            self
        }
        fn cascade(&mut self) -> Cascade {
            Cascade::new(std::mem::take(&mut self.layers))
        }
        fn asked(&self) -> Vec<&'static str> {
            std::mem::take(&mut *self.asked.lock().unwrap())
        }
    }

    fn decided(pick: usize, confidence: f32) -> Verdict {
        Verdict::Decided {
            choice: Decision::plain(pick),
            confidence,
        }
    }

    fn escalate() -> Verdict {
        Verdict::Escalate {
            reason: "not mine to say".into(),
        }
    }

    #[test]
    fn layers_are_asked_in_order_until_one_is_sure() {
        let mut stack = Stack::new()
            .layer("a", escalate(), 0.5)
            .layer("b", decided(1, 0.4), 0.5)
            .layer("c", decided(2, 0.9), 0.5)
            .layer("d", decided(3, 1.0), 0.5);
        let ruling = stack.cascade().ask(&situation(4), Instant::now());
        assert_eq!(stack.asked(), ["a", "b", "c"], "d is never asked");
        assert_eq!(ruling.decision, Some(Decision::plain(2)));
        assert_eq!(ruling.by, By::Layer { name: "c".into() });
        assert_eq!(ruling.trail.len(), 3);
        assert!(matches!(ruling.trail[0].asked, Asked::Escalated { .. }));
        assert_eq!(
            ruling.trail[1].asked,
            Asked::Unsure {
                confidence: 0.4,
                choice: Decision::plain(1)
            }
        );
        assert_eq!(ruling.trail[1].layer, "b");
    }

    #[test]
    fn a_threshold_is_met_at_it_and_missed_below_it() {
        let mut stack = Stack::new().layer("a", decided(1, 0.6), 0.6).layer("b", decided(2, 0.9), 0.0);
        let ruling = stack.cascade().ask(&situation(3), Instant::now());
        assert_eq!(ruling.by, By::Layer { name: "a".into() });
        let mut stack = Stack::new().layer("a", decided(1, 0.59), 0.6).layer("b", decided(2, 0.9), 0.0);
        let ruling = stack.cascade().ask(&situation(3), Instant::now());
        assert_eq!(ruling.by, By::Layer { name: "b".into() });
        assert_eq!(ruling.decision, Some(Decision::plain(2)));
    }

    #[test]
    fn with_no_layer_sure_the_most_confident_offer_stands() {
        let mut stack = Stack::new()
            .layer("a", decided(1, 0.3), 0.8)
            .layer("b", decided(2, 0.5), 0.8)
            .layer("c", escalate(), 0.8);
        let ruling = stack.cascade().ask(&situation(3), Instant::now());
        assert_eq!(stack.asked(), ["a", "b", "c"]);
        assert_eq!(ruling.decision, Some(Decision::plain(2)));
        assert_eq!(ruling.by, By::Offered { name: "b".into() });
    }

    #[test]
    fn with_nothing_offered_the_rules_pick_then_carry_on() {
        // A work menu: the rules take the first goal
        let mut stack = Stack::new().layer("a", escalate(), 0.5);
        let ruling = stack.cascade().ask(&situation(3), Instant::now());
        assert_eq!(ruling.by, By::Rules);
        assert_eq!(ruling.decision, Some(Decision::plain(0)));
        // No layers at all still answers
        let ruling = Cascade::new(Vec::new()).ask(&situation(2), Instant::now());
        assert_eq!(ruling.by, By::Rules);
        // An empty menu has nothing to pick
        let mut empty = situation(0);
        empty.carry_on = None;
        let ruling = Cascade::new(Vec::new()).ask(&empty, Instant::now());
        assert_eq!((ruling.decision, ruling.by), (None, By::CarryOn));
    }

    #[test]
    fn an_off_menu_answer_is_an_escalation() {
        let mut stack = Stack::new().layer("a", decided(7, 1.0), 0.5).layer("b", decided(1, 0.9), 0.5);
        let ruling = stack.cascade().ask(&situation(3), Instant::now());
        assert!(matches!(&ruling.trail[0].asked, Asked::OffMenu { why, choice } if why.contains("off a menu of 3") && choice.pick == 7));
        assert_eq!(ruling.by, By::Layer { name: "b".into() });
        // Off the menu and nothing else: the rules', never the invented one
        let mut stack = Stack::new().layer("a", decided(7, 1.0), 0.5);
        let ruling = stack.cascade().ask(&situation(3), Instant::now());
        assert_eq!(ruling.by, By::Rules);
    }

    #[test]
    fn parameters_must_be_the_entrys_and_inside_its_spans() {
        let mut s = situation(1);
        s.menu.push(Choice::Sell {
            item: "log".into(),
            price: Span::new(5, 20),
            quantity: Span::new(1, 30),
        });
        let sell = |price, quantity| Decision { pick: 1, price, quantity };
        assert_eq!(s.admits(&sell(Some(9), Some(30))), Ok(()));
        assert!(s.admits(&sell(Some(21), Some(30))).unwrap_err().contains("price 21"));
        assert!(s.admits(&sell(Some(9), Some(0))).unwrap_err().contains("quantity 0"));
        assert!(s.admits(&sell(None, Some(3))).unwrap_err().contains("needs a price"));
        assert!(s.admits(&sell(Some(9), None)).unwrap_err().contains("needs a quantity"));
        assert!(s.admits(&Decision { pick: 0, price: Some(1), quantity: None }).is_err());
        assert_eq!(s.admits(&Decision::plain(0)), Ok(()));
    }

    #[test]
    fn a_slow_or_spent_layer_passes_the_question_on() {
        let asked = Arc::<Mutex<Vec<&str>>>::default();
        let scripted = |name, sleep, cost| Scripted {
            name,
            says: decided(1, 1.0),
            asked: asked.clone(),
            later: Arc::default(),
            cost,
            sleep,
        };
        let mut slow = Layer::new(scripted("slow", Duration::from_millis(30), 0.0), 0.5, Duration::from_millis(5));
        slow.spend = None;
        let mut broke = Layer::new(scripted("broke", Duration::ZERO, 12.0), 0.5, Duration::from_secs(1));
        broke.spend = Some(10.0);
        let rules = Layer::new(scripted("last", Duration::ZERO, 0.0), 0.5, Duration::from_secs(1));
        let ruling = Cascade::new(vec![slow, broke, rules]).ask(&situation(3), Instant::now());
        assert!(matches!(ruling.trail[0].asked, Asked::Slow { .. }));
        assert!(ruling.trail[0].micros >= 30_000, "{}", ruling.trail[0].micros);
        assert_eq!(ruling.trail[1].asked, Asked::Broke);
        assert_eq!(*asked.lock().unwrap(), ["slow", "last"], "a spent layer isn't asked");
        assert_eq!(ruling.by, By::Layer { name: "last".into() });
    }

    #[test]
    fn a_pending_layer_lets_the_citizen_carry_on_until_it_answers() {
        let mut stack = Stack::new()
            .layer("rules", decided(1, 0.4), 0.6)
            .layer("model", Verdict::Pending(Ticket(7)), 0.6)
            .layer("top", decided(2, 1.0), 0.6);
        let later = stack.later.clone();
        let mut cascade = stack.cascade();
        let s = situation(4);
        let t0 = Instant::now();
        // Asked: carry on (entry 0) while the model thinks; the top isn't asked
        let ruling = cascade.ask(&s, t0);
        assert_eq!(stack.asked(), ["rules", "model"]);
        assert_eq!(ruling.decision, Some(Decision::plain(0)));
        assert_eq!(ruling.by, By::Waiting { name: "model".into() });
        assert!(cascade.is_waiting(&Question::Work));
        // Still thinking: carry on, and nobody is asked again
        let ruling = cascade.ask(&s, t0 + Duration::from_millis(200));
        assert_eq!(ruling.by, By::Waiting { name: "model".into() });
        assert!(stack.asked().is_empty());
        // Its answer comes, sure enough: it stands
        *later.lock().unwrap() = Some(decided(3, 0.9));
        let ruling = cascade.ask(&s, t0 + Duration::from_millis(400));
        assert_eq!(ruling.decision, Some(Decision::plain(3)));
        assert_eq!(ruling.by, By::Layer { name: "model".into() });
        assert!(!cascade.is_waiting(&Question::Work));
    }

    #[test]
    fn a_late_or_unsure_pending_answer_falls_back() {
        let mut stack = Stack::new()
            .layer("rules", decided(1, 0.4), 0.6)
            .layer("model", Verdict::Pending(Ticket(7)), 0.6)
            .layer("top", decided(2, 1.0), 0.6);
        let later = stack.later.clone();
        let mut cascade = stack.cascade();
        let s = situation(4);
        let t0 = Instant::now();
        cascade.ask(&s, t0);
        stack.asked();
        // Past its budget with no answer: what the rules offered, no waiting on
        let ruling = cascade.ask(&s, t0 + Duration::from_secs(2));
        assert_eq!(ruling.trail.len(), 1);
        assert_eq!((ruling.trail[0].layer.as_str(), &ruling.trail[0].asked), ("model", &Asked::Late));
        assert_eq!(ruling.decision, Some(Decision::plain(1)));
        assert_eq!(ruling.by, By::Offered { name: "rules".into() });
        assert!(stack.asked().is_empty());
        // Asked afresh, it pends again; an unsure answer goes on up
        cascade.ask(&s, t0 + Duration::from_secs(3));
        *later.lock().unwrap() = Some(decided(3, 0.2));
        let ruling = cascade.ask(&s, t0 + Duration::from_millis(3100));
        assert_eq!(stack.asked(), ["rules", "model", "top"]);
        assert_eq!(ruling.by, By::Layer { name: "top".into() });
    }

    #[test]
    fn a_pending_answer_is_checked_against_the_menu_as_it_is_now() {
        let mut stack = Stack::new().layer("model", Verdict::Pending(Ticket(1)), 0.5);
        let later = stack.later.clone();
        let mut cascade = stack.cascade();
        let t0 = Instant::now();
        cascade.ask(&situation(4), t0);
        // It picks entry 3, but by now the menu has two
        *later.lock().unwrap() = Some(decided(3, 1.0));
        let ruling = cascade.ask(&situation(2), t0 + Duration::from_millis(10));
        assert!(matches!(ruling.trail[0].asked, Asked::OffMenu { .. }));
        assert_eq!(ruling.by, By::Rules);
    }

    #[test]
    fn a_roster_names_known_layers_only() {
        assert!(Cascade::from_names(&["rules".into()]).is_ok());
        assert!(check_layers(&[]).unwrap_err().contains("at least one"));
        let err = check_layers(&["rules".into(), "oracle".into()]).unwrap_err();
        assert!(err.contains("\"oracle\"") && err.contains("rules"), "{err}");
    }

    #[test]
    fn a_situation_and_a_ruling_go_through_json() {
        let mut s = situation(1);
        s.question = Question::Sell { item: "log".into() };
        s.menu.push(Choice::Sell {
            item: "log".into(),
            price: Span::new(5, 20),
            quantity: Span::new(1, 30),
        });
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""question":{"kind":"sell","item":"log"}"#), "{json}");
        assert!(json.contains(r#""kind":"sell","item":"log","price":{"min":5,"max":20}"#), "{json}");
        assert_eq!(serde_json::from_str::<Situation>(&json).unwrap(), s);
        let ruling = Cascade::rules().ask(&situation(2), Instant::now());
        let json = serde_json::to_string(&ruling).unwrap();
        assert_eq!(serde_json::from_str::<Ruling>(&json).unwrap(), ruling);
        // A model's answer needs only the pick and its parameters
        let d: Decision = serde_json::from_str(r#"{"pick":1,"price":9,"quantity":4}"#).unwrap();
        assert_eq!(s.admits(&d), Ok(()));
    }
}
