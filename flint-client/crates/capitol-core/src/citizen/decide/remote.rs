//! What the layers that ask over the network share (C3.3).
//!
//! [`Decider::decide`](super::Decider::decide) is synchronous, and the world
//! never waits on a model, so a remote layer sends its request off and
//! answers `Pending`. Every remote layer in the process shares one runtime,
//! on a thread of its own, and one HTTP client. An answer waits in its
//! layer's [`Remote`] under its ticket until the cascade polls for it.
//!
//! This module also puts a [`Situation`] into words a model can read:
//! [`state`] for everything but the menu, as JSON, or [`prose`], the same
//! as a paragraph (#303); and [`label`] and [`entry`] for each entry on it.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use super::{Choice, Lot, Question, Situation, Span, Temperament, Ticket, Verdict};

/// How long one request may take before it's dropped. Past a layer's time
/// budget its answer is late anyway.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// Answers kept for tickets the cascade stopped waiting on. Older ones are
/// dropped.
const KEPT: u64 = 256;

/// The runtime every remote layer's requests run on.
fn handle() -> &'static tokio::runtime::Handle {
    static HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for the remote layers");
        let handle = runtime.handle().clone();
        std::thread::Builder::new()
            .name("remote-layers".into())
            .spawn(move || runtime.block_on(std::future::pending::<()>()))
            .expect("a thread for the remote layers");
        handle
    })
}

/// The HTTP client every remote layer shares.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .expect("an HTTP client")
    })
}

/// A request's answer, what it cost in US cents, and the model that
/// answered, if it said.
pub type Answer = (Verdict, f64, Option<String>);

/// A remote layer's requests in flight, their answers, and its spend.
#[derive(Default)]
pub struct Remote {
    next: u64,
    answers: Arc<Mutex<HashMap<u64, Verdict>>>,
    cents: Arc<Mutex<f64>>,
    models: Arc<Mutex<BTreeMap<String, usize>>>,
}

impl Remote {
    /// Sends `ask` off. Its answer comes to [`poll`](Self::poll) under the
    /// ticket.
    pub fn send(&mut self, ask: impl Future<Output = Answer> + Send + 'static) -> Ticket {
        self.next += 1;
        let ticket = self.next;
        let answers = self.answers.clone();
        let cents = self.cents.clone();
        let models = self.models.clone();
        handle().spawn(async move {
            let (verdict, spent, model) = ask.await;
            *lock(&cents) += spent;
            if let Some(model) = model {
                *lock(&models).entry(model).or_default() += 1;
            }
            let mut answers = lock(&answers);
            answers.retain(|&t, _| t + KEPT > ticket);
            answers.insert(ticket, verdict);
        });
        Ticket(ticket)
    }

    pub fn poll(&mut self, ticket: Ticket) -> Option<Verdict> {
        lock(&self.answers).remove(&ticket.0)
    }

    /// What its answers have cost so far, in US cents.
    pub fn spent(&self) -> f64 {
        *lock(&self.cents)
    }

    /// The models that answered, and how often.
    pub fn models(&self) -> BTreeMap<String, usize> {
        lock(&self.models).clone()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A key from the environment, if it's set and not empty.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Posts `body` as JSON with a bearer key, and reads the reply as JSON.
pub async fn post(url: &str, key: &str, body: &Value) -> Result<Value, String> {
    post_with(client().post(url).bearer_auth(key), body).await.map_err(|(_, e)| e)
}

/// Posts `body` as JSON on a request whose headers are already set, and
/// reads the reply as JSON. An error carries the status, if one came back.
pub async fn post_with(request: reqwest::RequestBuilder, body: &Value) -> Result<Value, (Option<u16>, String)> {
    let response = request
        .json(body)
        .send()
        .await
        .map_err(|e| (None, format!("sending: {e}")))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| (Some(status.as_u16()), format!("reading: {e}")))?;
    if !status.is_success() {
        let mut text = text;
        text.truncate(300);
        return Err((Some(status.as_u16()), format!("{status}: {text}")));
    }
    serde_json::from_str(&text).map_err(|e| (Some(status.as_u16()), format!("not JSON: {e}")))
}

// ---------------------------------------------------------------------------
// The situation in words
// ---------------------------------------------------------------------------

/// What a model reads about the situation: everything but the menu.
pub fn state(s: &Situation) -> Value {
    json!({
        "game": "The Capitol: a trading game with no combat. Citizens gather, craft, trade and bank to grow rich. Prices are in Strands.",
        "you": {
            "name": s.who.name,
            "role": s.who.role,
            "temperament": temperament(&s.who.temperament),
        },
        "where": s.at.zone,
        "time_of_day": s.when.time_of_day.map(clock),
        "hours_into_this_life": (s.when.hours * 10.0).round() / 10.0,
        "holdings": s.holdings,
        "market": s.market,
        "question": question(&s.question),
    })
}

/// [`state`] as a paragraph in the second person, as TypeSafe's own
/// examples give theirs (#303). A middling trait (0.34 to 0.66) is named
/// as middling, the rest by what they make the citizen do.
pub fn prose(s: &Situation) -> String {
    format!(
        "The Capitol is a trading game with no combat: citizens gather, craft, trade and bank to grow rich, and prices are in Strands. \
         {} {}",
        persona(s),
        circumstances(s)
    )
}

/// Who the citizen is: its name, its role and its temperament. It's the
/// same at every decision a citizen makes, so a model layer can cache it
/// (C3.2).
pub fn persona(s: &Situation) -> String {
    format!("You are {}, {}. {}", s.who.name, role(&s.who.role), character(&s.who.temperament))
}

/// The rest of [`prose`]: where and when, what the citizen holds, the
/// books it watches, and the question.
pub fn circumstances(s: &Situation) -> String {
    let mut out = format!(
        "You are in the {} zone{}, {:.1} game hours into this life. ",
        s.at.zone,
        s.when.time_of_day.map_or(String::new(), |d| format!(" and it is {}", clock(d))),
        s.when.hours
    );
    let h = &s.holdings;
    let mut held = vec![format!("{} Strands", h.strands)];
    held.extend(h.items.iter().filter(|(_, &n)| n != 0).map(|(item, n)| format!("{n} {}", item.replace('_', " "))));
    out.push_str(&format!("You hold {}. ", list(&held)));
    let tools: Vec<String> = h.tools.iter().cloned().collect();
    out.push_str(&match tools.as_slice() {
        [] => "You have no tools. ".to_string(),
        t => format!("Your tools: {}. ", list(t)),
    });
    let stations: Vec<String> = h.stations.iter().cloned().collect();
    if !stations.is_empty() {
        out.push_str(&format!("Your stations: {}. ", list(&stations)));
    }
    out.push_str(&match h.plot_room {
        Some(room) => format!("You own a plot with room for {room} more stations. "),
        None => format!("You own no plot; a deed would cost {} Strands. ", h.deed_cost),
    });
    if s.market.is_empty() {
        out.push_str("You don't know the hall's books. ");
    } else {
        let books: Vec<String> = s.market.iter().map(book).collect();
        out.push_str(&format!("At the Exchange hall: {}. ", books.join("; ")));
    }
    out.push_str(&question(&s.question));
    out
}

/// `crafter-planks` as `a planks crafter`, `gatherer-ore` as `an ore
/// gatherer`.
fn role(slug: &str) -> String {
    let name = match slug.split_once('-') {
        Some((what, of)) => format!("{} {what}", of.replace('_', " ")),
        None => slug.to_string(),
    };
    let article = if name.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    format!("{article} {name}")
}

/// A temperament as a sentence or two.
fn character(t: &Temperament) -> String {
    let traits = [
        (t.thrift, "thrift", "thrifty: you hold out for a fair price and would rather gather than overpay", "free with money: you don't haggle"),
        (t.risk, "risk", "bold: you ask above the bid, hoping someone pays up", "cautious: you sell near the bid"),
        (t.sociability, "sociability", "sociable: you like to pass free time among people at the hall", "solitary: you'd rather rest alone"),
        (t.patience, "patience", "patient: you sell and bank in big lots, less often", "impatient: you sell and bank in small lots, often"),
    ];
    let mut said = Vec::new();
    let mut middling = Vec::new();
    for (v, name, high, low) in traits {
        if v >= 0.67 {
            said.push(format!("You are {high}."));
        } else if v <= 0.33 {
            said.push(format!("You are {low}."));
        } else {
            middling.push(name);
        }
    }
    if !middling.is_empty() {
        said.push(format!("You are middling in {}.", list(&middling)));
    }
    said.join(" ")
}

fn book(q: &super::Quote) -> String {
    let side = |v: Option<i64>, none: &str, some: &str| v.map_or(none.to_string(), |p| format!("{some} {p}"));
    format!(
        "{} {}, {}, {}",
        q.item.replace('_', " "),
        side(q.bid, "no bids", "bid"),
        side(q.ask, "no offers", "ask"),
        side(q.last, "never traded", "last traded at")
    )
}

/// `a, b and c`.
fn list<S: AsRef<str>>(items: &[S]) -> String {
    match items {
        [] => String::new(),
        [one] => one.as_ref().to_string(),
        [rest @ .., last] => format!(
            "{} and {}",
            rest.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(", "),
            last.as_ref()
        ),
    }
}

fn temperament(t: &Temperament) -> Value {
    let r = |v: f32| (v * 100.0).round() / 100.0;
    json!({
        "scale": "each 0 to 1, 0.5 middling",
        "thrift": r(t.thrift),
        "thrift_means": "holds out for a fair price, gathers rather than overpay",
        "risk": r(t.risk),
        "risk_means": "asks above the bid, hoping someone pays up",
        "sociability": r(t.sociability),
        "sociability_means": "spends free time among people at the hall",
        "patience": r(t.patience),
        "patience_means": "sells and banks in bigger lots, less often",
    })
}

/// 0.5 as `12:00`.
fn clock(day: f64) -> String {
    let minutes = (day.rem_euclid(1.0) * 24.0 * 60.0).round() as i64 % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn lots(lots: &[Lot], strands: i64) -> String {
    let mut parts: Vec<String> = lots.iter().map(|l| format!("{} {}", l.quantity, l.item)).collect();
    if strands != 0 || parts.is_empty() {
        parts.push(format!("{strands} Strands"));
    }
    parts.join(" and ")
}

/// What's being decided, in a sentence.
pub fn question(q: &Question) -> String {
    match q {
        Question::Work => "What should you work on next?".into(),
        Question::Sell { item } => format!("Should you sell your {item} now, and at what price?"),
        Question::Buy { item } => format!("Should you buy {item} rather than gather it?"),
        Question::Trade {
            partner,
            partner_citizen,
            theirs,
            their_strands,
            ours,
            our_strands,
        } => format!(
            "{partner} ({}) offers a trade: they give {}, you give {}. Take it, counter, or decline?",
            if *partner_citizen { "a citizen" } else { "a player" },
            lots(theirs, *their_strands),
            lots(ours, *our_strands)
        ),
        Question::FreeTime => "What should you do with your free time?".into(),
        Question::Market => "Should you make a market at the hall with your float now, or go on earning it?".into(),
    }
}

/// A short name for an entry, for a model to answer with. Two entries can
/// share one; [`labels`] tells them apart.
pub fn label(c: &Choice) -> String {
    match c {
        Choice::Work { goal, .. } => format!("work: {goal}"),
        Choice::Bank => "bank".into(),
        Choice::Sell { item, .. } => format!("sell {item}"),
        Choice::Buy { item, .. } => format!("buy {item}"),
        Choice::Bid { item, .. } => format!("bid for {item}"),
        Choice::Stock { item, .. } => format!("stock stall with {item}"),
        Choice::Keep => "keep".into(),
        Choice::Gather { item } => format!("gather {item}"),
        Choice::Accept => "accept".into(),
        Choice::Counter { .. } => "counter".into(),
        Choice::Pay { .. } => "pay".into(),
        Choice::Decline => "decline".into(),
        Choice::Rest => "rest".into(),
        Choice::Hall => "hall".into(),
        Choice::Lane => "the bazaar lane".into(),
        Choice::Park { .. } => "the park".into(),
        Choice::Bridge { .. } => "the bridge".into(),
        Choice::Stall { .. } => "your stall".into(),
        Choice::Home => "home".into(),
        Choice::MakeMarket { .. } => "make a market".into(),
    }
}

/// Each entry's [`label`], made unique: a repeat gets ` (2)`, ` (3)`.
pub fn labels(menu: &[Choice]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    menu.iter()
        .map(|c| {
            let l = label(c);
            let n = seen.entry(l.clone()).or_default();
            *n += 1;
            if *n == 1 {
                l
            } else {
                format!("{l} ({n})")
            }
        })
        .collect()
}

fn range(span: &Span<i64>) -> String {
    if span.min == span.max {
        format!("{}", span.min)
    } else {
        format!("{} to {}", span.min, span.max)
    }
}

fn count(span: &Span<i32>) -> String {
    range(&Span::new(span.min as i64, span.max as i64))
}

/// What an entry means, with the ranges its parameters may take.
pub fn entry(c: &Choice) -> String {
    match c {
        Choice::Work { goal, step } => format!("Work toward {goal}; the next step is {step}"),
        Choice::Bank => "Take your fiber to the exchange for Strands".into(),
        Choice::Sell { item, price, quantity } => format!(
            "Sell {item} at the hall now, at {} Strands each, {} of them",
            range(price),
            count(quantity)
        ),
        Choice::Buy { item, price, quantity } => format!(
            "Buy {item} at the hall now, paying up to {} Strands each, {} of them",
            range(price),
            count(quantity)
        ),
        Choice::Bid { item, price, quantity } => format!(
            "Leave a standing bid for {item} at the hall, which anyone may sell into while you work: {} Strands each, for {} of them",
            range(price),
            count(quantity)
        ),
        Choice::Stock { item, price, quantity } => format!(
            "Stock your stall at the bazaar with {item}, renting one if you have none: {} Strands each, {} of them",
            range(price),
            count(quantity)
        ),
        Choice::Keep => "Don't sell: keep what you hold".into(),
        Choice::Gather { item } => format!("Don't buy: go and gather {item} yourself"),
        Choice::Accept => "Accept the trade as it stands".into(),
        Choice::Counter { strands } => format!("Counter: ask for a deal {} Strands better", range(strands)),
        Choice::Pay { strands } => format!("Pay {} Strands for the goods they offer", range(strands)),
        Choice::Decline => "Decline the trade".into(),
        Choice::Rest => "Sit out a while where you are".into(),
        Choice::Hall => "Pass the time on the Exchange hall's steps, among people".into(),
        Choice::Lane => "Stroll the bazaar's lane, among the stalls".into(),
        Choice::Park { .. } => "Sit a while in the park nearby".into(),
        Choice::Bridge { .. } => "Stand a while on the bridge nearby, looking down the water at dusk".into(),
        Choice::Stall { .. } => "Mind your own stall at the bazaar".into(),
        Choice::Home => "Go straight home".into(),
        Choice::MakeMarket { items, float, making } => format!(
            "Make a market in {} at the hall: quote both sides with your float of {float} Strands{}",
            items.join(", "),
            if *making { " (you're at it already)" } else { "" }
        ),
    }
}

/// [`entry`], noting the one that means carrying on with what you were
/// doing.
pub fn entry_in(s: &Situation, i: usize) -> String {
    let text = entry(&s.menu[i]);
    if s.carry_on == Some(i) {
        format!("{text} (carry on with what you were doing)")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_labels_are_told_apart() {
        let menu = vec![Choice::Gather { item: "log".into() }, Choice::Rest, Choice::Gather { item: "log".into() }];
        assert_eq!(labels(&menu), ["gather log", "rest", "gather log (2)"]);
    }

    #[test]
    fn a_situation_reads_as_a_paragraph() {
        use super::super::{Held, Place, Quote, When, Who};
        let mut holdings = Held {
            strands: 40,
            deed_cost: 250,
            ..Held::default()
        };
        holdings.items.insert("plank".into(), 12);
        holdings.items.insert("fiber".into(), 120);
        let s = Situation {
            who: Who {
                name: "Percy Sawyer".into(),
                role: "crafter-planks".into(),
                temperament: Temperament {
                    thrift: 0.8,
                    risk: 0.1,
                    sociability: 0.5,
                    patience: 0.5,
                },
            },
            at: Place {
                x: 0.0,
                y: 0.0,
                zone: "urban".into(),
            },
            when: When {
                time_of_day: Some(0.4),
                hours: 5.2,
            },
            holdings,
            market: vec![Quote {
                item: "iron_ore".into(),
                bid: None,
                ask: Some(5),
                last: Some(3),
            }],
            question: Question::Sell { item: "plank".into() },
            menu: vec![Choice::Keep],
            carry_on: Some(0),
        };
        let text = prose(&s);
        for part in [
            "You are Percy Sawyer, a planks crafter.",
            "iron ore no bids",
            "You are thrifty",
            "You are cautious",
            "middling in sociability and patience",
            "it is 09:36",
            "You hold 40 Strands, 120 fiber and 12 plank.",
            "a deed would cost 250 Strands",
            "iron ore no bids, ask 5, last traded at 3",
            "Should you sell your plank now",
        ] {
            assert!(text.contains(part), "{part:?} in {text}");
        }
    }

    #[test]
    fn a_role_reads_with_its_article() {
        assert_eq!(role("gatherer-ore"), "an ore gatherer");
        assert_eq!(role("crafter-iron_ingots"), "an iron ingots crafter");
        assert_eq!(role("builder"), "a builder");
    }

    #[test]
    fn a_clock_reads_the_day() {
        assert_eq!(clock(0.5), "12:00");
        assert_eq!(clock(0.0), "00:00");
        assert_eq!(clock(0.75), "18:00");
    }
}
