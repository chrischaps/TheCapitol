//! Claude as a layer (C3.2, plan D4): the language-model tier above Jev.
//!
//! A situation goes to Anthropic's Messages API (`POST /v1/messages`) as:
//! - a **system prompt** of two blocks, both cached: the game and how to
//!   answer, the same for every citizen, then the citizen's persona
//!   ([`persona`](super::remote::persona)), the same at every decision it
//!   makes;
//! - one user message: the rest of the situation
//!   ([`circumstances`](super::remote::circumstances)) and the menu;
//! - a `choose` tool, `strict`, that the model answers through. Its
//!   confidence is the one it gives in the tool's `confidence`: Claude gives
//!   no logprobs (#262), so this layer can only be as honest as the model's
//!   own estimate, and the replay's calibration says how honest that is.
//!
//! The menu can go in the tool two ways ([`Menu`], `CLAUDE_MENU`):
//! - `schema` [default]: the tool's schema is the menu, one branch per
//!   entry with its label as a `const` and only the parameters it takes, so
//!   nothing else can be said. Tools come first in the prompt, so a menu
//!   not seen before misses the cache, but a town's menus repeat: 83 of 94
//!   were cache hits on the corpus. No answer went off the menu (#261).
//! - `letters`: one tool for every question, `option` a letter A
//!   to Z, the menu lettered in the message. The tool and system prompt are
//!   the same bytes every time, so they're always read from the cache. But
//!   5 answers in 94 went off the menu, against none for `schema`.
//!
//! Either way, `strict` can't hold a number inside a span (the API takes no
//! `minimum` or `maximum`), so a price out of range, or a letter past the
//! menu's end, is off the menu, and the cascade escalates it. A reply with
//! no tool call, a refusal or an error escalates too.
//!
//! Two tiers: `claude-haiku` (`CLAUDE_HAIKU_MODEL`, default Haiku 5.5),
//! forced to the tool with thinking off, and `claude-sonnet`
//! (`CLAUDE_SONNET_MODEL`, default Sonnet 5.5), which can't be forced, so it
//! is asked to use it, with thinking between tools only. Their key is
//! `ANTHROPIC_API_KEY`, and with none they escalate everything.
//! `ANTHROPIC_URL` overrides the server.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

use super::remote::{self, Remote};
use super::{Choice, Decider, Decision, Situation, Ticket, Verdict};

pub const ANTHROPIC_URL: &str = "https://api.anthropic.com";
pub const HAIKU_MODEL: &str = "claude-haiku-5-5";
pub const SONNET_MODEL: &str = "claude-sonnet-5-5";
const VERSION: &str = "2023-06-01";
/// The longest menu that letters cover.
const LETTERS: usize = 26;
/// A tool call is short; this leaves room for Sonnet's between-tools note.
const MAX_TOKENS: u32 = 400;
/// The tool's name.
const TOOL: &str = "choose";

/// What a reply's tokens cost, in US dollars a million: input, output.
/// A cache write is 1.25 times the input price, a read a tenth of it.
fn prices(model: &str) -> (f64, f64) {
    if model.starts_with("claude-haiku-5") {
        (0.10, 0.50)
    } else if model.starts_with("claude-sonnet") {
        (2.0, 10.0)
    } else if model.starts_with("claude-opus-5") {
        (4.0, 20.0)
    } else {
        // An unknown model is priced as Sonnet, so the spend cap still bites
        (2.0, 10.0)
    }
}

/// A reply's cost in US cents, from its `usage`.
pub fn cents(model: &str, usage: &Value) -> f64 {
    let (input, output) = prices(model);
    let n = |k: &str| usage[k].as_f64().unwrap_or(0.0);
    let dollars = (n("input_tokens") * input
        + n("cache_creation_input_tokens") * input * 1.25
        + n("cache_read_input_tokens") * input * 0.1
        + n("output_tokens") * output)
        / 1e6;
    dollars * 100.0
}

/// How the menu goes into the tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// One tool, the menu lettered in the message: cached.
    Letters,
    /// The tool's schema is the menu: exact, but uncached.
    Schema,
}

impl Menu {
    fn from_env() -> Menu {
        match remote::env("CLAUDE_MENU") {
            Some(v) if v.eq_ignore_ascii_case("letters") => Menu::Letters,
            _ => Menu::Schema,
        }
    }
}

/// One Claude tier's settings.
#[derive(Debug, Clone)]
pub struct Tier {
    pub name: String,
    pub model: String,
    /// Whether `tool_choice` may force the tool. Sonnet 5.5 refuses that.
    pub force: bool,
    /// The `thinking` setting: off, the cheapest way the model allows.
    pub thinking: Value,
}

impl Tier {
    pub fn haiku() -> Tier {
        Tier {
            name: "claude-haiku".into(),
            model: remote::env("CLAUDE_HAIKU_MODEL").unwrap_or_else(|| HAIKU_MODEL.into()),
            force: true,
            thinking: json!({"type": "disabled"}),
        }
    }

    pub fn sonnet() -> Tier {
        Tier {
            name: "claude-sonnet".into(),
            model: remote::env("CLAUDE_SONNET_MODEL").unwrap_or_else(|| SONNET_MODEL.into()),
            force: false,
            // Sonnet 5.5 refuses `disabled`; this is its thinking off
            thinking: json!({"type": "between_tools"}),
        }
    }
}

/// A Claude tier's layer.
pub struct ClaudeDecider {
    tier: Tier,
    menu: Menu,
    url: String,
    key: Option<String>,
    remote: Remote,
}

impl ClaudeDecider {
    pub fn new(tier: Tier, menu: Menu, url: &str, key: Option<String>) -> ClaudeDecider {
        ClaudeDecider {
            tier,
            menu,
            url: url.into(),
            key,
            remote: Remote::default(),
        }
    }

    /// `tier`, with its key, server and menu from the environment.
    pub fn from_env(tier: Tier) -> ClaudeDecider {
        ClaudeDecider::new(
            tier,
            Menu::from_env(),
            &remote::env("ANTHROPIC_URL").unwrap_or_else(|| ANTHROPIC_URL.into()),
            remote::env("ANTHROPIC_API_KEY"),
        )
    }
}

impl Decider for ClaudeDecider {
    fn name(&self) -> &str {
        &self.tier.name
    }

    fn decide(&mut self, s: &Situation) -> Verdict {
        let Some(key) = self.key.clone() else {
            return Verdict::Escalate {
                reason: format!("{}: no ANTHROPIC_API_KEY", self.tier.name),
            };
        };
        if s.menu.is_empty() || s.menu.len() > LETTERS {
            return Verdict::Escalate {
                reason: format!("a menu of {}", s.menu.len()),
            };
        }
        let body = request(s, &self.tier, self.menu);
        let url = format!("{}/v1/messages", self.url.trim_end_matches('/'));
        let name = self.tier.name.clone();
        let model = self.tier.model.clone();
        let menu = self.menu;
        let labels = remote::labels(&s.menu);
        let ticket = self.remote.send(async move {
            match send(&url, &key, &body).await {
                Ok(reply) => {
                    let answered = reply["model"].as_str().unwrap_or(&model).to_string();
                    let spent = cents(&answered, &reply["usage"]);
                    // The tally counts cache hits apart, for the replay's report
                    let cached = reply["usage"]["cache_read_input_tokens"].as_u64().unwrap_or(0) > 0;
                    let tallied = if cached { format!("{answered} (cached)") } else { answered };
                    let verdict = read(&reply, menu, &labels).unwrap_or_else(|reason| Verdict::Escalate {
                        reason: format!("{name}: {reason}"),
                    });
                    (verdict, spent, Some(tallied))
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

/// Posts a request, trying once more after a moment if the API is busy
/// (429, 529) or failed (5xx).
pub(crate) async fn send(url: &str, key: &str, body: &Value) -> Result<Value, String> {
    let post = || {
        remote::post_with(
            remote::client()
                .post(url)
                .header("x-api-key", key)
                .header("anthropic-version", VERSION),
            body,
        )
    };
    match post().await {
        Err((Some(status), _)) if status == 429 || status >= 500 => {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            post().await.map_err(|(_, e)| e)
        }
        other => other.map_err(|(_, e)| e),
    }
}

const SYSTEM: &str = "You make decisions for a citizen of The Capitol, a persistent trading game with no combat. \
Everyone in it, people and citizens alike, grows rich by gathering, crafting, trading and banking; there is \
nothing else to win.

How the world works:
- Prices are in Strands. Fiber is a second currency, earned by gathering and swapped for Strands at the exchange \
(banking). Strands buy goods, tools, stations and plots.
- Gatherers take raw goods from the land: logs, stone, ore, clay, fiber. Crafters turn goods into better goods at \
stations: logs into planks, ore into ingots, clay into bricks. Every item has a quality from 0 to 100, and a \
crafted item's quality follows its inputs', so good inputs are worth more.
- The Exchange hall keeps a book for each good: the bid is the most anyone will pay now, the ask the least anyone \
will sell for now, and the last trade what it actually went for. Selling at the bid is sure and quick; asking more \
may not fill. A standing bid buys while you work, if anyone sells into it.
- The bazaar has stalls you can rent and stock; people browse them and buy.
- A plot is land you own, with room for stations. A deed costs Strands, and land is taxed hourly.
- Time matters: the hall keeps hours, and work set aside costs what you'd have made meanwhile.

How to decide:
- You are given who the citizen is, where it is and what it holds, the books it watches, the question, and a menu \
of options. You can only choose from the menu.
- Decide as the citizen would. Its temperament is part of who it is: a thrifty citizen holds out for a fair \
price, a bold one asks high, a patient one deals in big lots. Play it true even where another choice would earn \
a little more.
- Weigh what each option earns against its time and risk. Prefer steady progress on the citizen's work. Don't \
sell below what the books say a good is worth unless the citizen needs Strands now, and don't pay far above the \
last trade when the citizen could gather the good itself.
- If an option carries on with what the citizen was doing, choose it unless another is clearly better.
- Where an option takes a price or a quantity, give one inside the range the option states.

Answer by calling the choose tool exactly once, and say nothing else. With it, give your confidence from 0 to 1 \
that your choice is the best on the menu for this citizen: about 0.95 when one option is plainly right, about 0.5 \
when two are close, and lower when you are guessing.";

const CONFIDENCE: &str = "How sure you are, from 0 to 1, that this is the best option on the menu for this citizen";

fn letter(i: usize) -> char {
    (b'A' + i as u8) as char
}

/// The one tool for [`Menu::Letters`].
fn letters_tool() -> Value {
    let letters: Vec<String> = (0..LETTERS).map(|i| letter(i).to_string()).collect();
    json!({
        "name": TOOL,
        "description": "Choose an option from the menu, by its letter, with its price and quantity if it takes them.",
        "strict": true,
        "input_schema": {
            "type": "object",
            "properties": {
                "option": {"type": "string", "enum": letters, "description": "The option's letter"},
                "price": {"type": "integer", "description": "Strands each, only if the option takes a price"},
                "quantity": {"type": "integer", "description": "How many, only if the option takes a quantity"},
                "confidence": {"type": "number", "description": CONFIDENCE},
            },
            "required": ["option", "confidence"],
            "additionalProperties": false,
        },
    })
}

/// The entry's parameters: whether it takes a price, and a quantity.
fn takes(c: &Choice) -> (bool, bool) {
    // A counter or a payment is a sum of Strands: a price
    let (price, quantity) = c.takes();
    (price.is_some(), quantity.is_some())
}

/// The tool for [`Menu::Schema`]: one branch for each entry.
fn schema_tool(s: &Situation, labels: &[String]) -> Value {
    let branches: Vec<Value> = s
        .menu
        .iter()
        .zip(labels)
        .map(|(c, label)| {
            let (price, quantity) = takes(c);
            let mut properties = serde_json::Map::new();
            properties.insert("option".into(), json!({"type": "string", "const": label}));
            let mut required = vec![json!("option")];
            if price {
                properties.insert("price".into(), json!({"type": "integer"}));
                required.push(json!("price"));
            }
            if quantity {
                properties.insert("quantity".into(), json!({"type": "integer"}));
                required.push(json!("quantity"));
            }
            json!({
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            })
        })
        .collect();
    json!({
        "name": TOOL,
        "description": "Choose an option from the menu, with its price and quantity if it takes them.",
        "strict": true,
        "input_schema": {
            "type": "object",
            "properties": {
                "choice": {"anyOf": branches},
                "confidence": {"type": "number", "description": CONFIDENCE},
            },
            "required": ["choice", "confidence"],
            "additionalProperties": false,
        },
    })
}

/// The request for a situation.
pub fn request(s: &Situation, tier: &Tier, menu: Menu) -> Value {
    let labels = remote::labels(&s.menu);
    let (tool, options) = match menu {
        Menu::Letters => (
            letters_tool(),
            (0..s.menu.len())
                .map(|i| format!("{}) {}", letter(i), remote::entry_in(s, i)))
                .collect::<Vec<_>>(),
        ),
        Menu::Schema => (
            schema_tool(s, &labels),
            labels
                .iter()
                .enumerate()
                .map(|(i, label)| format!("\"{label}\": {}", remote::entry_in(s, i)))
                .collect(),
        ),
    };
    let user = format!(
        "{}\n\nOptions:\n{}",
        remote::circumstances(s),
        options.join("\n")
    );
    let tool_choice = if tier.force {
        json!({"type": "tool", "name": TOOL})
    } else {
        json!({"type": "auto"})
    };
    json!({
        "model": tier.model,
        "max_tokens": MAX_TOKENS,
        "system": [
            {"type": "text", "text": SYSTEM, "cache_control": {"type": "ephemeral"}},
            {"type": "text", "text": remote::persona(s), "cache_control": {"type": "ephemeral"}},
        ],
        "tools": [tool],
        "tool_choice": tool_choice,
        "thinking": tier.thinking,
        "output_config": {"effort": "low"},
        "messages": [{"role": "user", "content": user}],
    })
}

/// The reply as a verdict. `labels` are the menu's, in order.
pub fn read(reply: &Value, menu: Menu, labels: &[String]) -> Result<Verdict, String> {
    match reply["stop_reason"].as_str() {
        Some("refusal") => return Err("refused".into()),
        Some("max_tokens") => return Err("ran out of tokens".into()),
        _ => {}
    }
    let input = reply["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b["type"] == "tool_use" && b["name"] == TOOL)
        })
        .map(|b| &b["input"])
        .ok_or("no tool call")?;
    let (pick, answer) = match menu {
        Menu::Letters => {
            let option = input["option"].as_str().ok_or("no option")?;
            let pick = option
                .chars()
                .next()
                .filter(|c| c.is_ascii_uppercase() && option.len() == 1)
                .map(|c| c as usize - 'A' as usize)
                .ok_or_else(|| format!("{option:?} is no letter"))?;
            if pick >= labels.len() {
                return Err(format!("{option:?} is past the menu's end"));
            }
            (pick, input)
        }
        Menu::Schema => {
            let choice = &input["choice"];
            let option = choice["option"].as_str().ok_or("no option")?;
            let pick = labels
                .iter()
                .position(|l| l == option)
                .ok_or_else(|| format!("{option:?} is not on the menu"))?;
            (pick, choice)
        }
    };
    let confidence = input["confidence"].as_f64().ok_or("no confidence")?.clamp(0.0, 1.0) as f32;
    Ok(Verdict::Decided {
        choice: Decision {
            pick,
            price: answer["price"].as_i64(),
            quantity: answer["quantity"].as_i64().map(|q| q as i32),
        },
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citizen::decide::Span;

    fn situation_with(menu: Vec<Choice>) -> Situation {
        let mut s = crate::citizen::decide::corpus::situations().remove(0);
        s.menu = menu;
        s.carry_on = Some(0);
        s
    }

    fn labels() -> Vec<String> {
        vec!["keep".into(), "sell log".into()]
    }

    fn reply(input: Value) -> Value {
        json!({
            "model": "claude-haiku-5-5",
            "stop_reason": "tool_use",
            "content": [{"type": "tool_use", "name": "choose", "id": "t", "input": input}],
            "usage": {"input_tokens": 100, "output_tokens": 20},
        })
    }

    #[test]
    fn a_letter_reads_as_its_entry() {
        let r = reply(json!({"option": "B", "price": 4, "quantity": 10, "confidence": 0.82}));
        let Verdict::Decided { choice, confidence } = read(&r, Menu::Letters, &labels()).unwrap() else { panic!() };
        assert_eq!(
            choice,
            Decision {
                pick: 1,
                price: Some(4),
                quantity: Some(10)
            }
        );
        assert!((confidence - 0.82).abs() < 1e-6);
    }

    #[test]
    fn a_letter_past_the_menu_is_refused() {
        let r = reply(json!({"option": "C", "confidence": 0.9}));
        assert!(read(&r, Menu::Letters, &labels()).is_err());
    }

    #[test]
    fn a_schema_answer_reads_by_its_label() {
        let r = reply(json!({"choice": {"option": "keep"}, "confidence": 1.4}));
        let Verdict::Decided { choice, confidence } = read(&r, Menu::Schema, &labels()).unwrap() else { panic!() };
        assert_eq!(choice, Decision::plain(0));
        assert_eq!(confidence, 1.0, "confidence is clamped");
        let r = reply(json!({"choice": {"option": "rest"}, "confidence": 0.5}));
        assert!(read(&r, Menu::Schema, &labels()).is_err());
    }

    #[test]
    fn no_tool_call_or_a_refusal_escalates() {
        let text = json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "B"}]});
        assert!(read(&text, Menu::Letters, &labels()).is_err());
        let refused = json!({"stop_reason": "refusal", "content": []});
        assert!(read(&refused, Menu::Letters, &labels()).is_err());
    }

    #[test]
    fn the_schema_holds_only_the_menu() {
        let menu = vec![
            Choice::Keep,
            Choice::Sell {
                item: "log".into(),
                grade: None,
                price: Span::new(2, 6),
                quantity: Span::new(1, 10),
            },
        ];
        let s = situation_with(menu);
        let tool = schema_tool(&s, &remote::labels(&s.menu));
        let branches = tool["input_schema"]["properties"]["choice"]["anyOf"].as_array().unwrap();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0]["properties"]["option"]["const"], "keep");
        assert!(branches[0]["properties"].get("price").is_none());
        assert_eq!(branches[1]["required"], json!(["option", "price", "quantity"]));
    }

    #[test]
    fn the_letters_request_is_the_same_up_to_the_persona() {
        // Two different questions from one citizen share their tools and
        // system prompt byte for byte, so the cache holds
        let one = situation_with(vec![Choice::Keep]);
        let two = situation_with(vec![Choice::Rest, Choice::Hall]);
        let a = request(&one, &Tier::haiku(), Menu::Letters);
        let b = request(&two, &Tier::haiku(), Menu::Letters);
        assert_eq!(a["tools"], b["tools"]);
        assert_eq!(a["system"], b["system"]);
        assert_ne!(a["messages"], b["messages"]);
    }

    #[test]
    fn a_reply_is_priced_by_its_model() {
        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 0});
        assert!((cents("claude-haiku-5-5", &usage) - 10.0).abs() < 1e-9);
        let usage = json!({"cache_read_input_tokens": 1_000_000, "output_tokens": 100_000});
        assert!((cents("claude-sonnet-5-5", &usage) - (20.0 + 100.0)).abs() < 1e-9);
    }
}
