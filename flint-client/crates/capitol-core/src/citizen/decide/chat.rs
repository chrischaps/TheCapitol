//! A chat model as a layer (C3.3): any OpenAI-compatible chat completions
//! API, such as OpenRouter's.
//!
//! The menu goes to the model lettered, A, B, C, with each entry's ranges,
//! and it answers with one line: the letter, then `price=` and `qty=` where
//! the entry takes them. Its confidence is the probability of the letter it
//! answered with, from the reply's logprobs; a model that gives none is
//! never sure, so it only ever offers. An answer off the menu is the
//! cascade's to catch, as for any layer.
//!
//! The one stack that uses it so far is `jev-router`: OpenRouter's
//! `typesafe/jev-router`, which picks a language model for each request
//! (#262 found that's what OpenRouter offers of Jev). Its key is
//! `OPENROUTER_API_KEY`; `JEV_ROUTER_MODEL` and `OPENROUTER_URL` override
//! its model and server. With no key it escalates everything.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::remote::{self, Remote};
use super::{Decider, Decision, Situation, Ticket, Verdict};

pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1";
pub const JEV_ROUTER_MODEL: &str = "typesafe/jev-router";
/// The longest menu that letters cover.
const LETTERS: usize = 26;
/// What a reply may run to, its reasoning included.
const MAX_TOKENS: u32 = 1024;

const SYSTEM: &str = "You decide for a citizen of The Capitol, a trading game. \
You get the citizen's situation and a menu of options, each with a letter. \
Reply with one line and nothing else: the letter of your choice, then price=N if the option takes a price \
and qty=N if it takes a quantity, each inside the range the option gives. For example: B price=12 qty=5";

/// A chat model's layer.
pub struct ChatDecider {
    name: String,
    model: String,
    url: String,
    key: Option<String>,
    remote: Remote,
}

impl ChatDecider {
    pub fn new(name: &str, model: &str, url: &str, key: Option<String>) -> ChatDecider {
        ChatDecider {
            name: name.into(),
            model: model.into(),
            url: url.into(),
            key,
            remote: Remote::default(),
        }
    }

    /// The `jev-router` layer, from the environment.
    pub fn jev_router() -> ChatDecider {
        ChatDecider::new(
            "jev-router",
            &remote::env("JEV_ROUTER_MODEL").unwrap_or_else(|| JEV_ROUTER_MODEL.into()),
            &remote::env("OPENROUTER_URL").unwrap_or_else(|| OPENROUTER_URL.into()),
            remote::env("OPENROUTER_API_KEY"),
        )
    }
}

impl Decider for ChatDecider {
    fn name(&self) -> &str {
        &self.name
    }

    fn decide(&mut self, s: &Situation) -> Verdict {
        let Some(key) = self.key.clone() else {
            return Verdict::Escalate {
                reason: format!("{}: no key", self.name),
            };
        };
        if s.menu.is_empty() || s.menu.len() > LETTERS {
            return Verdict::Escalate {
                reason: format!("a menu of {}", s.menu.len()),
            };
        }
        let body = request(s, &self.model);
        let url = format!("{}/chat/completions", self.url.trim_end_matches('/'));
        let name = self.name.clone();
        let menu = s.menu.len();
        let ticket = self.remote.send(async move {
            match remote::post(&url, &key, &body).await {
                Ok(reply) => {
                    let cents = reply["usage"]["cost"].as_f64().unwrap_or(0.0) * 100.0;
                    let verdict = read(&reply, menu).unwrap_or_else(|reason| Verdict::Escalate {
                        reason: format!("{name}: {reason}"),
                    });
                    let model = reply["model"].as_str().map(String::from);
                    (verdict, cents, model)
                }
                Err(e) => (Verdict::Escalate { reason: format!("{name}: {e}") }, 0.0, None),
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

fn letter(i: usize) -> char {
    (b'A' + i as u8) as char
}

/// The request: the situation, and the menu lettered.
pub fn request(s: &Situation, model: &str) -> Value {
    let options: Vec<String> = (0..s.menu.len())
        .map(|i| format!("{}) {}", letter(i), remote::entry_in(s, i)))
        .collect();
    let user = format!("Situation:\n{}\n\nOptions:\n{}", remote::state(s), options.join("\n"));
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": user},
        ],
        // Room for a reasoning model to think before its one line: with
        // too little, the router's reasoning models answer nothing
        "max_tokens": MAX_TOKENS,
        "reasoning": {"exclude": true},
        "temperature": 0,
        "logprobs": true,
        "top_logprobs": 5,
    })
}

/// The reply as a verdict.
pub fn read(reply: &Value, menu: usize) -> Result<Verdict, String> {
    let choice = &reply["choices"][0];
    let text = choice["message"]["content"]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("no reply (finished: {})", choice["finish_reason"].as_str().unwrap_or("?")))?;
    let decision = parse(text, menu)?;
    // The first token with text in it carries the letter
    let confidence = choice["logprobs"]["content"]
        .as_array()
        .and_then(|tokens| {
            tokens
                .iter()
                .find(|t| t["token"].as_str().is_some_and(|t| !t.trim().is_empty()))
        })
        .and_then(|t| t["logprob"].as_f64())
        .map_or(0.0, |lp| lp.exp() as f32);
    Ok(Verdict::Decided {
        choice: decision,
        confidence,
    })
}

/// `B price=12 qty=5` as a decision on a menu of `menu`.
fn parse(text: &str, menu: usize) -> Result<Decision, String> {
    // A model may dress its letter up: **B**, "B", (B)
    let text = text.trim().trim_start_matches(['*', '"', '\'', '(', '`', ' ']);
    let first = text.chars().next().ok_or("an empty reply")?.to_ascii_uppercase();
    let pick = (first as usize).wrapping_sub('A' as usize);
    if !first.is_ascii_uppercase() || pick >= menu {
        return Err(format!("{text:?} names no option"));
    }
    let number = |name: &str| -> Option<i64> {
        let at = text.find(name)? + name.len();
        let digits: String = text[at..]
            .chars()
            .enumerate()
            .take_while(|&(k, c)| c.is_ascii_digit() || (k == 0 && c == '-'))
            .map(|(_, c)| c)
            .collect();
        digits.parse().ok()
    };
    Ok(Decision {
        pick,
        price: number("price="),
        quantity: number("qty=").map(|q| q as i32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_line_parses() {
        assert_eq!(parse("B", 3).unwrap(), Decision::plain(1));
        assert_eq!(
            parse(" a price=12 qty=5\n", 3).unwrap(),
            Decision {
                pick: 0,
                price: Some(12),
                quantity: Some(5)
            }
        );
        assert!(parse("D", 3).is_err());
        assert_eq!(parse("**C** price=4", 3).unwrap().pick, 2);
        assert!(parse("I think B", 3).is_err(), "a reply that rambles names I");
    }

    #[test]
    fn confidence_is_the_letters_probability() {
        let reply = json!({
            "choices": [{
                "message": {"content": "B"},
                "logprobs": {"content": [{"token": "B", "logprob": -0.105}]}
            }],
            "usage": {"cost": 1e-5}
        });
        let Verdict::Decided { choice, confidence } = read(&reply, 4).unwrap() else { panic!() };
        assert_eq!(choice.pick, 1);
        assert!((confidence - 0.9).abs() < 0.01, "{confidence}");
    }

    #[test]
    fn no_logprobs_is_never_sure() {
        let reply = json!({"choices": [{"message": {"content": "A"}}]});
        let Verdict::Decided { confidence, .. } = read(&reply, 2).unwrap() else { panic!() };
        assert_eq!(confidence, 0.0);
    }
}
