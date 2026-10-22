//! A mind's pulse (the Observatory's D4.1): what a citizen's decisions tell
//! the server's console, live.
//!
//! A [`Mind`](super::Mind) given a [`Pulse`] notes each decision in it as
//! it's made: each layer's turn, how long it took and what came of it, what
//! the layers cost, how the question was settled, and the decision itself,
//! briefly. The town takes it each game minute ([`Pulse::take`]) for its
//! report (`protocol::metrics::TownReport`). The decision log (C3.1) is
//! kept as before; this is the little of it the console shows.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use capitol_protocol::metrics::{DecisionBrief, LayerReport, LayerTurn, CITIZEN_DECISIONS};

use super::{remote, Ruling, Situation};

/// One citizen's, shared between its mind and the town, across its lives.
pub type SharedPulse = Arc<Mutex<Pulse>>;

#[derive(Debug, Default)]
pub struct Pulse {
    /// Each layer's turns since the last take, by name
    layers: BTreeMap<String, LayerReport>,
    /// How its questions were settled since the last take
    rulings: BTreeMap<String, u64>,
    /// Its newest decisions, oldest first
    decisions: VecDeque<DecisionBrief>,
    /// Whether one of them is new since the last take
    fresh: bool,
    /// What each layer has cost it, in US cents, over all its lives
    spent: BTreeMap<String, f64>,
}

/// What a pulse said since it was last taken.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Beat {
    /// Each layer's turns, with what it cost over the window; its
    /// `cents_total` is this citizen's, over all its lives
    pub layers: BTreeMap<String, LayerReport>,
    pub rulings: BTreeMap<String, u64>,
    /// Its newest decisions, if one is new; empty if none is
    pub decisions: Vec<DecisionBrief>,
}

impl Pulse {
    pub fn shared() -> SharedPulse {
        Arc::new(Mutex::new(Pulse::default()))
    }

    /// `s`, settled by `ruling` at `at_ms`, after which each layer had cost
    /// `cost` more cents than before it
    pub(super) fn decided(&mut self, s: &Situation, ruling: &Ruling, cost: &[(String, f64)], at_ms: i64) {
        for turn in &ruling.trail {
            self.layers.entry(turn.layer.clone()).or_default().turn(turn.asked.as_str(), turn.micros);
        }
        for (name, cents) in cost.iter().filter(|(_, c)| *c > 0.0) {
            self.layers.entry(name.clone()).or_default().cents += cents;
            *self.spent.entry(name.clone()).or_insert(0.0) += cents;
        }
        *self.rulings.entry(ruling.by.label()).or_insert(0) += 1;
        if self.decisions.len() >= CITIZEN_DECISIONS {
            self.decisions.pop_front();
        }
        self.decisions.push_back(brief(s, ruling, at_ms));
        self.fresh = true;
    }

    /// What it said since it was last taken, and starts again
    pub fn take(&mut self) -> Beat {
        let mut layers = std::mem::take(&mut self.layers);
        for (name, total) in &self.spent {
            layers.entry(name.clone()).or_default().cents_total = *total;
        }
        let decisions = if std::mem::take(&mut self.fresh) {
            self.decisions.iter().cloned().collect()
        } else {
            Vec::new()
        };
        Beat { layers, rulings: std::mem::take(&mut self.rulings), decisions }
    }
}

/// A decision as the console's drill-down shows it
fn brief(s: &Situation, ruling: &Ruling, at_ms: i64) -> DecisionBrief {
    let menu = remote::labels(&s.menu);
    let decision = ruling.decision.filter(|d| d.pick < menu.len());
    DecisionBrief {
        at_ms,
        question: s.question.key(),
        picked: decision.map(|d| menu[d.pick].clone()),
        price: decision.and_then(|d| d.price),
        quantity: decision.and_then(|d| d.quantity),
        menu,
        by: ruling.by.label(),
        trail: ruling
            .trail
            .iter()
            .map(|t| LayerTurn { layer: t.layer.clone(), outcome: t.asked.as_str().to_string(), micros: t.micros })
            .collect(),
    }
}
