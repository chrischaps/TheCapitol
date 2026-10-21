//! Jev (C3.3, plan D4): TypeSafe's "System One" model as the cascade's fast
//! tier, between the rules and the language models.
//!
//! Jev answers questions whose answers the caller defines, with
//! probabilities rather than text. A situation goes to it as one request,
//! `POST /v1/systemone`, whose questions Jev weighs in parallel:
//! - the situation (everything but the menu) is its `state`, as a
//!   paragraph ([`prose`](super::remote::prose)): Jev's answers at 0.9 and
//!   over agreed with the rules 98% of the time that way, against 94% as
//!   JSON, for 30% fewer tokens (#303);
//! - the menu is a `choice` question, one option for each entry, named by
//!   its [`label`](super::remote::label) and described with its ranges;
//! - each parameter an entry takes is a `score` question, over up to ten
//!   levels spread through its span. Its answer is the probability-weighted
//!   level, brought back into the span and rounded.
//!
//! The layer's confidence is the `choice` answer's: Jev computes it from how
//! the probabilities spread, so a close call between two entries comes back
//! low and goes up a layer. Jev can't answer off the menu.
//!
//! Its key is `TYPESAFE_API_KEY`, and with none it escalates everything.
//! `JEV_MODEL` picks the model [default: `jev-latest`], `TYPESAFE_URL` the
//! server, and `JEV_STATE=json` sends the state as JSON instead (#303). Input costs $0.042 a million tokens; output is free.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::remote::{self, Remote};
use super::{Choice, Decider, Decision, Situation, Span, Ticket, Verdict};

pub const DEFAULT_URL: &str = "https://api.typesafe.ai";
pub const DEFAULT_MODEL: &str = "jev-latest";
/// US cents per input token: $0.042 a million.
pub const CENTS_PER_TOKEN: f64 = 0.042 / 1e6 * 100.0;
/// The most levels a `score` question may have.
const LEVELS: usize = 10;
/// The `choice` question's name.
const PICK: &str = "pick";

/// The `jev` layer.
pub struct JevDecider {
    model: String,
    url: String,
    key: Option<String>,
    /// The state as a paragraph rather than JSON (#303)
    prose: bool,
    remote: Remote,
}

impl JevDecider {
    /// Its key, model and server from the environment.
    pub fn from_env() -> JevDecider {
        JevDecider {
            model: remote::env("JEV_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into()),
            url: remote::env("TYPESAFE_URL").unwrap_or_else(|| DEFAULT_URL.into()),
            key: remote::env("TYPESAFE_API_KEY"),
            prose: !remote::env("JEV_STATE").is_some_and(|v| v.eq_ignore_ascii_case("json")),
            remote: Remote::default(),
        }
    }
}

impl Decider for JevDecider {
    fn name(&self) -> &str {
        "jev"
    }

    fn decide(&mut self, s: &Situation) -> Verdict {
        let Some(key) = self.key.clone() else {
            return Verdict::Escalate {
                reason: "no TYPESAFE_API_KEY".into(),
            };
        };
        if s.menu.is_empty() {
            return Verdict::Escalate { reason: "an empty menu".into() };
        }
        let asking = Asking::new(s, &self.model, self.prose);
        let url = format!("{}/v1/systemone", self.url.trim_end_matches('/'));
        let ticket = self.remote.send(async move {
            match remote::post(&url, &key, &asking.body).await {
                Ok(reply) => {
                    let tokens = reply["usage"]["input_tokens"].as_f64().unwrap_or(0.0);
                    let verdict = asking.verdict(&reply).unwrap_or_else(|reason| Verdict::Escalate {
                        reason: format!("jev: {reason}"),
                    });
                    let model = reply["model"].as_str().map(String::from);
                    (verdict, tokens * CENTS_PER_TOKEN, model)
                }
                Err(e) => (Verdict::Escalate { reason: format!("jev: {e}") }, 0.0, None),
            }
        });
        Verdict::Pending(ticket)
    }

    fn poll(&mut self, ticket: Ticket) -> Option<Verdict> {
        self.remote.poll(ticket)
    }

    fn spent(&self) -> f64 {
        self.remote.spent()
    }

    fn tally(&self) -> BTreeMap<String, usize> {
        self.remote.models()
    }
}

/// A parameter of one entry, asked as a `score` question.
#[derive(Debug, Clone, PartialEq)]
struct Param {
    /// Its question's name
    key: String,
    entry: usize,
    price: bool,
    /// Each level's value, lowest first
    levels: Vec<i64>,
}

/// A situation put as Jev's questions, and how to read the answers.
#[derive(Debug, Clone)]
pub struct Asking {
    pub body: Value,
    /// Each option's name, by entry
    options: Vec<String>,
    params: Vec<Param>,
}

impl Asking {
    /// `prose`: the state as a paragraph rather than JSON.
    pub fn new(s: &Situation, model: &str, prose: bool) -> Asking {
        let options = remote::labels(&s.menu);
        let criteria: Map<String, Value> = options
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), Value::String(remote::entry_in(s, i))))
            .collect();
        let mut questions = Map::new();
        questions.insert(
            PICK.into(),
            json!({
                "type": "choice",
                "instructions": format!("{} Pick what this citizen, as its role and temperament suggest, should do.", remote::question(&s.question)),
                "criteria": criteria,
            }),
        );
        let mut params = Vec::new();
        for (i, choice) in s.menu.iter().enumerate() {
            let (price, quantity) = choice.takes();
            let asks = [
                price.map(|p| (true, p)),
                quantity.map(|q| (false, Span::new(q.min as i64, q.max as i64))),
            ];
            for (is_price, span) in asks.into_iter().flatten() {
                let levels = levels(span);
                // A span of one value isn't asked: the answer is that value
                if levels.len() < 2 {
                    params.push(Param {
                        key: String::new(),
                        entry: i,
                        price: is_price,
                        levels,
                    });
                    continue;
                }
                let key = format!("{} {}", if is_price { "price" } else { "quantity" }, i);
                let (what, unit) = describe(choice, is_price);
                questions.insert(
                    key.clone(),
                    json!({
                        "type": "score",
                        "instructions": format!("If this citizen chooses \"{}\", {what}", options[i]),
                        "criteria": levels.iter().map(|v| format!("{v} {unit}")).collect::<Vec<_>>(),
                    }),
                );
                params.push(Param {
                    key,
                    entry: i,
                    price: is_price,
                    levels,
                });
            }
        }
        Asking {
            body: json!({
                "model": model,
                "state": if prose { Value::String(remote::prose(s)) } else { remote::state(s) },
                "questions": questions,
            }),
            options,
            params,
        }
    }

    /// Jev's reply as a verdict: the option it picked, with its parameters.
    pub fn verdict(&self, reply: &Value) -> Result<Verdict, String> {
        let answers = &reply["answers"];
        let pick = &answers[PICK];
        let name = pick["choice"].as_str().ok_or("no choice in the reply")?;
        let entry = self
            .options
            .iter()
            .position(|o| o == name)
            .ok_or_else(|| format!("{name:?} isn't an option"))?;
        let confidence = pick["confidence"].as_f64().ok_or("no confidence in the reply")? as f32;
        let mut decision = Decision::plain(entry);
        for param in self.params.iter().filter(|p| p.entry == entry) {
            let value = match param.levels.as_slice() {
                [] => return Err(format!("{} has an empty span", param.key)),
                [only] => *only,
                levels => {
                    let score = answers[param.key.as_str()]["score"]
                        .as_f64()
                        .ok_or_else(|| format!("no score for {}", param.key))?;
                    level(levels, score)
                }
            };
            if param.price {
                decision.price = Some(value);
            } else {
                decision.quantity = Some(value as i32);
            }
        }
        Ok(Verdict::Decided {
            choice: decision,
            confidence,
        })
    }
}

/// What a parameter's `score` asks, and its levels' unit.
fn describe(choice: &Choice, price: bool) -> (String, String) {
    match (choice, price) {
        (Choice::Counter { .. }, _) => ("how many Strands better a deal should it ask for?".into(), "Strands better".into()),
        (Choice::Pay { .. }, _) => ("how many Strands should it pay?".into(), "Strands".into()),
        (Choice::Buy { .. } | Choice::Bid { .. }, true) => ("what should it pay for each, at most?".into(), "Strands each".into()),
        (_, true) => ("what price should it ask for each?".into(), "Strands each".into()),
        (Choice::Sell { item, .. } | Choice::Buy { item, .. } | Choice::Bid { item, .. } | Choice::Stock { item, .. }, false) => {
            ("how many should it trade?".into(), item.clone())
        }
        (_, false) => ("how many?".into(), "of them".into()),
    }
}

/// Up to [`LEVELS`] values through `span`, both ends included: every value
/// if there are few enough, else spread evenly.
fn levels(span: Span<i64>) -> Vec<i64> {
    if span.max < span.min {
        return Vec::new();
    }
    let width = span.max - span.min;
    if width < LEVELS as i64 {
        return (span.min..=span.max).collect();
    }
    let mut out: Vec<i64> = (0..LEVELS)
        .map(|k| span.min + (width as f64 * k as f64 / (LEVELS - 1) as f64).round() as i64)
        .collect();
    out.dedup();
    out
}

/// A score (the probability-weighted level, 0 to `levels.len() - 1`) as a
/// value, between the levels it falls between.
fn level(levels: &[i64], score: f64) -> i64 {
    let top = levels.len() - 1;
    let score = score.clamp(0.0, top as f64);
    let lo = score.floor() as usize;
    let hi = (lo + 1).min(top);
    let frac = score - lo as f64;
    (levels[lo] as f64 + frac * (levels[hi] - levels[lo]) as f64).round() as i64
}

#[cfg(test)]
mod tests {
    use super::super::{Held, Place, Question, Quote, Temperament, When, Who};
    use super::*;

    fn situation() -> Situation {
        Situation {
            who: Who {
                name: "Percy Sawyer".into(),
                role: "crafter-planks".into(),
                temperament: Temperament::default(),
            },
            at: Place {
                x: 6000.0,
                y: 6000.0,
                zone: "urban".into(),
            },
            when: When {
                time_of_day: Some(0.4),
                hours: 2.0,
            },
            holdings: Held {
                strands: 40,
                ..Held::default()
            },
            market: vec![Quote {
                item: "plank".into(),
                bid: Some(9),
                ask: Some(14),
                last: Some(11),
            }],
            question: Question::Sell { item: "plank".into() },
            menu: vec![
                Choice::Sell {
                    item: "plank".into(),
                    price: Span::new(9, 40),
                    quantity: Span::new(1, 6),
                },
                Choice::Keep,
            ],
            carry_on: Some(1),
        }
    }

    #[test]
    fn the_state_may_go_as_prose() {
        let asking = Asking::new(&situation(), "jev-latest", true);
        let state = asking.body["state"].as_str().unwrap();
        assert!(state.starts_with("The Capitol is a trading game"), "{state}");
        assert_eq!(asking.body["questions"]["pick"]["type"], "choice", "the questions don't change");
    }

    #[test]
    fn levels_cover_the_span() {
        assert_eq!(levels(Span::new(3, 6)), [3, 4, 5, 6]);
        assert_eq!(levels(Span::new(5, 5)), [5]);
        let wide = levels(Span::new(9, 40));
        assert_eq!(wide.len(), LEVELS);
        assert_eq!((wide[0], wide[LEVELS - 1]), (9, 40));
    }

    #[test]
    fn a_score_falls_between_levels() {
        let l = [10, 20, 30];
        assert_eq!(level(&l, 0.0), 10);
        assert_eq!(level(&l, 1.5), 25);
        assert_eq!(level(&l, 2.0), 30);
        assert_eq!(level(&l, 7.0), 30, "clamped");
    }

    #[test]
    fn a_menu_becomes_a_choice_and_its_parameters_scores() {
        let asking = Asking::new(&situation(), "jev-latest", false);
        let q = &asking.body["questions"];
        assert_eq!(asking.body["model"], "jev-latest");
        assert_eq!(q["pick"]["type"], "choice");
        let options: Vec<&String> = q["pick"]["criteria"].as_object().unwrap().keys().collect();
        assert_eq!(options.len(), 2);
        assert!(q["pick"]["criteria"]["keep"].as_str().unwrap().contains("carry on"));
        assert_eq!(q["price 0"]["type"], "score");
        assert_eq!(q["price 0"]["criteria"].as_array().unwrap().len(), LEVELS);
        assert_eq!(q["quantity 0"]["criteria"].as_array().unwrap().len(), 6);
        assert!(asking.body["state"]["holdings"]["strands"] == 40);
    }

    /// The reply's shape as TypeSafe's docs give it.
    #[test]
    fn a_reply_reads_as_a_decision_inside_its_spans() {
        let s = situation();
        let asking = Asking::new(&s, "jev-latest", false);
        let reply = json!({
            "model": "jev-1.13.0",
            "answers": {
                "pick": {"type": "choice", "choice": "sell plank", "confidence": 0.82,
                         "probabilities": {"sell plank": 0.9, "keep": 0.1}},
                "price 0": {"type": "score", "score": 1.4, "confidence": 0.35,
                            "legend": {}, "probabilities": {}},
                "quantity 0": {"type": "score", "score": 5.0, "confidence": 0.9,
                               "legend": {}, "probabilities": {}}
            },
            "usage": {"input_tokens": 512, "output_tokens": 40}
        });
        let Verdict::Decided { choice, confidence } = asking.verdict(&reply).unwrap() else { panic!() };
        assert_eq!(choice.pick, 0);
        assert!((confidence - 0.82).abs() < 1e-6);
        // Levels 9, 12, 16, ...: 1.4 is 40% of the way from 12 to 16
        assert_eq!(choice.price, Some(14));
        assert_eq!(choice.quantity, Some(6));
        assert!(s.admits(&choice).is_ok());
    }

    #[test]
    fn a_plain_pick_takes_no_parameters() {
        let s = situation();
        let asking = Asking::new(&s, "jev-latest", false);
        let reply = json!({"answers": {"pick": {"choice": "keep", "confidence": 0.6}}});
        let Verdict::Decided { choice, .. } = asking.verdict(&reply).unwrap() else { panic!() };
        assert_eq!(choice, Decision::plain(1));
        assert!(s.admits(&choice).is_ok());
    }

    #[test]
    fn a_span_of_one_value_takes_that_value_unasked() {
        let mut s = situation();
        s.menu[0] = Choice::Sell {
            item: "plank".into(),
            price: Span::new(3, 3),
            quantity: Span::new(1, 6),
        };
        let asking = Asking::new(&s, "jev-latest", false);
        assert!(asking.body["questions"].get("price 0").is_none(), "nothing to ask");
        let reply = json!({"answers": {
            "pick": {"choice": "sell plank", "confidence": 0.95},
            "quantity 0": {"score": 2.0}
        }});
        let Verdict::Decided { choice, .. } = asking.verdict(&reply).unwrap() else { panic!() };
        assert_eq!((choice.price, choice.quantity), (Some(3), Some(3)));
        assert!(s.admits(&choice).is_ok());
    }

    #[test]
    fn an_unknown_option_is_an_error() {
        let asking = Asking::new(&situation(), "jev-latest", false);
        let reply = json!({"answers": {"pick": {"choice": "juggle", "confidence": 0.9}}});
        assert!(asking.verdict(&reply).is_err());
    }

    #[test]
    fn with_no_key_it_escalates() {
        let mut jev = JevDecider {
            model: DEFAULT_MODEL.into(),
            url: DEFAULT_URL.into(),
            key: None,
            prose: false,
            remote: Remote::default(),
        };
        assert!(matches!(jev.decide(&situation()), Verdict::Escalate { .. }));
    }
}
