//! What a citizen says (Commons S1.6, D11): a phrasebook of lines by
//! occasion, in three voices by temperament, picked by a seeded whim so the
//! same moment gives the same line on replay.
//!
//! The phrasebook is the rules layer of the [`Speaker`] seam. A model layer
//! (#311, [`voice`](super::voice)) goes over the same seam: it gets the same
//! [`Occasion`] and [`Moment`], answers later ([`Words::Pending`]), and the
//! phrasebook answers whenever it has nothing, or nothing in time
//! ([`TALK_WAIT`]). The world never waits on it.
//!
//! Lines may carry `{name}` (the other's first name) and `{hello}` (good
//! morning, afternoon or evening by the server's clock). They never name
//! goods: an item's name won't sit in a sentence the same way twice
//! ("my best rope", "my best plank").
//!
//! [`Voice`] keeps a citizen from talking too much: a greeting at most once
//! per person per game hour, an answer to being spoken to 2 to 5 s later and
//! at most once a minute per person, a letter's reply at most once a game
//! day per sender, and never more than [`LINES_PER_WINDOW`] in the chat
//! bucket's window, half what the server allows.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use capitol_protocol::rules;
use uuid::Uuid;

use super::decide::{Temperament, Ticket};

/// When a citizen speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Occasion {
    /// It stops beside a person it passes (C2.1).
    Greeting,
    /// A trade window opens, either way round.
    Opening,
    /// It takes what's on the table, or pays for their goods.
    Accepting,
    /// It takes goods off its side until their offer pays for what's left.
    Countering,
    /// It says no and closes the window.
    Declining,
    /// Someone bought from its stall, standing near.
    Sale,
    /// A person said its name in a local line.
    Answer,
    /// A letter came (S1.5): the reply, with postage paid.
    Letter,
}

impl Occasion {
    pub const ALL: [Occasion; 8] = [
        Occasion::Greeting,
        Occasion::Opening,
        Occasion::Accepting,
        Occasion::Countering,
        Occasion::Declining,
        Occasion::Sale,
        Occasion::Answer,
        Occasion::Letter,
    ];
}

/// The voice a temperament speaks in, by its sociability: a third or so of
/// citizens each, since a trait is two draws averaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Band {
    /// Few words, dry.
    Reserved,
    /// Plain and civil.
    Even,
    /// Glad of the company, and says so.
    Warm,
}

impl Band {
    pub const ALL: [Band; 3] = [Band::Reserved, Band::Even, Band::Warm];

    pub fn of(temperament: &Temperament) -> Band {
        match temperament.sociability {
            s if s < 0.4 => Band::Reserved,
            s if s > 0.6 => Band::Warm,
            _ => Band::Even,
        }
    }
}

/// What a speaker knows of the moment it speaks in.
#[derive(Debug, Clone, PartialEq)]
pub struct Moment {
    /// Who it's speaking to, as the server names them.
    pub who: String,
    /// What they said to it, for an [`Occasion::Answer`].
    pub heard: Option<String>,
    /// [`rules::time_of_day`] on the server's clock, if known.
    pub time_of_day: Option<f64>,
    /// What makes this moment this one, for the whim: who, and the game
    /// minute, say. The same key gives the same line.
    pub key: u64,
}

impl Moment {
    /// A moment with `who` at the game minute of `server_ms`.
    pub fn with(who: &str, id: Uuid, server_ms: Option<i64>) -> Moment {
        let minute = server_ms.map_or(0, |ms| ms.div_euclid(GAME_MINUTE_MS));
        Moment {
            who: who.to_string(),
            heard: None,
            time_of_day: server_ms.map(rules::time_of_day),
            key: mix(id.as_u128() as u64 ^ (id.as_u128() >> 64) as u64, minute as u64),
        }
    }
}

/// What a [`Speaker`] gives for an occasion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Words {
    /// The line, now.
    Line(String),
    /// Asked; the line comes to [`Speaker::poll`] under the ticket.
    Pending(Ticket),
    /// Nothing from this speaker (no key, its budget spent, a refusal, a
    /// ramble): the phrasebook answers.
    Nothing,
}

/// Something that finds a citizen's words: the phrasebook, or a model over
/// it (#311).
pub trait Speaker: Send {
    /// What its lines are counted under in [`Voice::tally`].
    fn name(&self) -> &str;

    fn line(&mut self, occasion: Occasion, moment: &Moment) -> Words;

    /// A pending line's answer, once it has come: a [`Words::Line`] or
    /// [`Words::Nothing`].
    fn poll(&mut self, _ticket: Ticket) -> Option<Words> {
        None
    }

    /// What its lines have cost so far, in US cents.
    fn spent(&self) -> f64 {
        0.0
    }
}

/// The phrasebook's name in a [`Voice::tally`].
pub const PHRASEBOOK: &str = "phrasebook";

/// The rules layer: the lines below, in the citizen's voice.
#[derive(Debug, Clone, Copy)]
pub struct Phrasebook {
    band: Band,
    seed: u64,
}

impl Phrasebook {
    pub fn new(temperament: &Temperament, seed: u64) -> Phrasebook {
        Phrasebook {
            band: Band::of(temperament),
            seed,
        }
    }
}

impl Phrasebook {
    /// Its line for `occasion` at `moment`: the same moment, the same line.
    pub fn say(&self, occasion: Occasion, moment: &Moment) -> String {
        let lines = lines(occasion, self.band);
        let whim = mix(mix(self.seed, occasion as u64), moment.key);
        fill(lines[(whim % lines.len() as u64) as usize], moment)
    }
}

impl Speaker for Phrasebook {
    fn name(&self) -> &str {
        PHRASEBOOK
    }

    fn line(&mut self, occasion: Occasion, moment: &Moment) -> Words {
        Words::Line(self.say(occasion, moment))
    }
}

/// `line` with its `{name}` and `{hello}` filled in from `moment`.
pub fn fill(line: &str, moment: &Moment) -> String {
    line.replace("{name}", first_name(&moment.who))
        .replace("{hello}", hello(moment.time_of_day))
}

/// "Ada" of "Ada Woodward"; a one-word name whole.
pub fn first_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

/// The greeting for the hour.
pub fn hello(time_of_day: Option<f64>) -> &'static str {
    match time_of_day {
        Some(t) if (0.2..0.5).contains(&t) => "Good morning",
        Some(t) if (0.5..0.72).contains(&t) => "Good afternoon",
        Some(t) if (0.72..0.95).contains(&t) => "Good evening",
        _ => "Hello",
    }
}

/// Does `text` name `name`: its first name, or the whole of it, as words,
/// ignoring case?
pub fn names(text: &str, name: &str) -> bool {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_end_matches("'s").to_lowercase())
        .collect();
    let wanted: Vec<String> = name.split_whitespace().map(str::to_lowercase).collect();
    let Some(first) = wanted.first() else {
        return false;
    };
    words.iter().any(|w| w == first) || words.windows(wanted.len()).any(|w| w == wanted.as_slice())
}

/// The lines for `occasion` in the voice `band`.
pub fn lines(occasion: Occasion, band: Band) -> &'static [&'static str] {
    use Band::*;
    use Occasion::*;
    match (occasion, band) {
        (Greeting, Reserved) => &[
            "{hello}.",
            "{name}.",
            "{hello}, {name}.",
            "All right?",
            "Keeping well, {name}?",
            "Mind how you go.",
        ],
        (Greeting, Even) => &[
            "{hello}, {name}.",
            "{hello}. Keeping busy?",
            "Hello, {name}. How's trade?",
            "There you are, {name}. {hello}.",
            "{hello}. Good to see a face on this road.",
            "{hello}, {name}. All well with you?",
        ],
        (Greeting, Warm) => &[
            "{hello}, {name}! Lovely to see you.",
            "Well, if it isn't {name}! {hello}!",
            "{hello}, {name}! How are you keeping?",
            "Oh, {hello}! Come by the hall later, won't you?",
            "{name}! I was hoping I'd run into someone.",
            "{hello}, {name}. You look like you've been hard at it.",
        ],
        (Opening, Reserved) => &[
            "Let's see it, then.",
            "Go on.",
            "What have you got?",
            "Show me.",
            "Right. Business.",
            "I'm listening, {name}.",
        ],
        (Opening, Even) => &[
            "Let's see what we can do, {name}.",
            "Right then, let's trade.",
            "What are you after?",
            "Fair's fair. Let's have a look.",
            "Happy to deal, {name}.",
            "Go on, put it on the table.",
        ],
        (Opening, Warm) => &[
            "A trade! Let's see what we can do for you, {name}.",
            "Come, come, let's make a deal.",
            "For you, {name}? Always.",
            "Let me show you what I've got.",
            "I had a feeling you'd ask. Let's look.",
            "Business and good company. My favourite.",
        ],
        (Accepting, Reserved) => &["Done.", "That'll do.", "Fair enough.", "Agreed.", "Fine. Deal.", "Square, {name}."],
        (Accepting, Even) => &[
            "That's fair. Deal.",
            "Done, {name}. A pleasure.",
            "I'll take it.",
            "Sounds right to me.",
            "Shake on it, then.",
            "Deal. Good doing business.",
        ],
        (Accepting, Warm) => &[
            "Wonderful! It's a deal, {name}.",
            "Done and done! Spend it well.",
            "Deal! You drive a kind bargain.",
            "Lovely. A pleasure as always, {name}.",
            "That'll do nicely. Thank you!",
            "Shake on it! Come back soon.",
        ],
        (Countering, Reserved) => &[
            "Not for that. Less, then.",
            "For that much, you get this.",
            "No. Here's what that buys.",
            "Too thin. Try this.",
            "That buys less than you think, {name}.",
            "I'll meet you here. No further.",
        ],
        (Countering, Even) => &[
            "That won't stretch to all of it. How about this?",
            "For that, I can do this much.",
            "Close, {name}. Here's what I can do.",
            "Let's meet in the middle.",
            "Not quite. Have a look at this instead.",
            "I'll have to trim it a little for that price.",
        ],
        (Countering, Warm) => &[
            "Ah, so close! How about this instead?",
            "I'd love to, but that only covers part of it. Here.",
            "Let me see... for that, this much, and gladly.",
            "You're nearly there, {name}! Try this.",
            "Can't quite do it at that, I'm afraid. But look.",
            "Here's my best for you. What do you say?",
        ],
        (Declining, Reserved) => &["No.", "Not today.", "Can't do that.", "No deal, {name}.", "Pass.", "Not worth my while."],
        (Declining, Even) => &[
            "Sorry, {name}, I can't make that work.",
            "Not at that price, I'm afraid.",
            "I'll have to say no this time.",
            "Doesn't add up for me. Another day.",
            "No, thank you. Try the hall?",
            "Can't do it. No hard feelings.",
        ],
        (Declining, Warm) => &[
            "Oh, I'm sorry, {name}, I just can't.",
            "I wish I could! Not this time.",
            "Ah, it breaks my heart, but no.",
            "Not today, friend. Come back tomorrow?",
            "You're kind to ask, but I'll pass.",
            "Another time, {name}, I promise.",
        ],
        (Sale, Reserved) => &["Sold.", "That's yours.", "Paid. Thanks.", "Good choice.", "Use it well.", "Obliged, {name}."],
        (Sale, Even) => &[
            "Thank you, {name}.",
            "Sold! Good choice, that.",
            "Thanks for stopping by.",
            "A pleasure, {name}. Come again.",
            "That's good work, you'll see.",
            "Much obliged.",
        ],
        (Sale, Warm) => &[
            "Thank you kindly, {name}! That's some of my best.",
            "Oh, wonderful! Do come back soon.",
            "Thank you! Tell your friends about my stall.",
            "There you are, {name}. Made with care, that.",
            "Lovely doing business! Mind how you go.",
            "Thank you! You've made my day.",
        ],
        (Answer, Reserved) => &["Mm?", "Yes?", "What is it?", "I heard you, {name}.", "Busy.", "Hm. Go on."],
        (Answer, Even) => &[
            "Yes, {name}?",
            "That's me. What can I do for you?",
            "Hello there. You called?",
            "Sorry, miles away. Yes?",
            "Right here, {name}.",
            "What's on your mind?",
        ],
        (Answer, Warm) => &[
            "Oh! Hello, {name}! What can I do for you?",
            "That's me! Always glad of a chat.",
            "Yes, yes, I'm here! What is it?",
            "{name}! Lovely to hear a friendly voice.",
            "Mm? Oh, it's you! How are you?",
            "Here I am! Need a hand with something?",
        ],
        (Letter, Reserved) => &[
            "Received. Find me at the hall.",
            "Noted. Thank you.",
            "Got your letter. We'll talk.",
            "Thank you. I'll think on it.",
            "Read it. Come by in person, {name}.",
            "Thanks for writing.",
        ],
        (Letter, Even) => &[
            "Thank you for your letter, {name}. Find me at the market after noon.",
            "Got your letter. Let's talk when you're passing.",
            "Kind of you to write. I'll keep it in mind.",
            "Thank you, {name}. Come and find me some time.",
            "Your letter arrived safely. Thank you.",
            "Good to hear from you. I'm about most days.",
        ],
        (Letter, Warm) => &[
            "Thank you kindly, {name}. Find me on the hall's steps after noon.",
            "What a lovely letter! Do come and say hello.",
            "You made my day, {name}. Thank you for writing.",
            "I read it twice! Come and find me at the market.",
            "Thank you, thank you. Letters are rare in this city.",
            "So good to hear from you, {name}. Write again soon.",
        ],
    }
}

/// A game minute, in the server's ms.
const GAME_MINUTE_MS: i64 = rules::DAY_LENGTH_SECS * 1000 / (24 * 60);
/// A game hour, in the server's ms.
const GAME_HOUR_MS: i64 = GAME_MINUTE_MS * 60;
/// A game day, in the server's ms.
const GAME_DAY_MS: i64 = rules::DAY_LENGTH_SECS * 1000;

/// The fewest and most seconds before an answer: time to have heard it.
const ANSWER_AFTER: (f64, f64) = (2.0, 5.0);
/// How long before it answers the same person again.
pub const ANSWER_AGAIN: Duration = Duration::from_secs(60);
/// The window its own count of lines runs over: the server's chat bucket's
/// (S0.1, `CHAT_WINDOW_SECS`).
pub const LINE_WINDOW: Duration = Duration::from_secs(10);
/// Lines in a [`LINE_WINDOW`]: half the server's bucket (8), so a citizen
/// is never refused for talking.
pub const LINES_PER_WINDOW: usize = 4;
/// How long a model's line is waited for before the phrasebook's is said
/// instead (#311): a greeting's pause ([`Bot`](crate::bot::Bot)'s two
/// seconds), which Haiku's p95 of 1.7 s fits (C3.2). An answer waits until
/// its moment if that's later.
pub const TALK_WAIT: Duration = Duration::from_secs(2);
/// How long a model's reply to a letter is waited for: a letter is no
/// conversation.
pub const LETTER_WAIT: Duration = Duration::from_secs(60);

/// A line on its way: when it may be said, until when a model's is waited
/// for, and to whom.
struct Due {
    at: Instant,
    until: Instant,
    /// `None` out loud; a letter's sender for a reply.
    to: Option<Uuid>,
    words: Coming,
}

enum Coming {
    Ready(String),
    /// Asked of the model, with the phrasebook's line if it has nothing.
    Asked(Ticket, String),
}

/// A citizen's voice: its [`Speaker`] over its phrasebook, and the limits
/// on how often it may use them. Pure: every call is given the time.
pub struct Voice {
    speaker: Box<dyn Speaker>,
    /// What it says when the speaker has nothing: the speaker itself, for
    /// the phrasebook.
    book: Phrasebook,
    /// Whom it greeted, by game hour.
    greeted: HashMap<Uuid, i64>,
    /// Whom it last answered, and when.
    answered: HashMap<Uuid, Instant>,
    /// Whose letters it replied to, by game day.
    replied: HashMap<Uuid, i64>,
    /// Lines waiting their moment, or a model's words.
    due: Vec<Due>,
    /// When it spoke, over the last [`LINE_WINDOW`].
    spoken: VecDeque<Instant>,
    /// Lines found, by the speaker that found the words.
    tally: BTreeMap<String, usize>,
    rng: u64,
}

impl Voice {
    /// `speaker` over the phrasebook in `temperament`'s voice.
    pub fn new(speaker: Box<dyn Speaker>, temperament: &Temperament, seed: u64) -> Voice {
        Voice {
            speaker,
            book: Phrasebook::new(temperament, seed),
            greeted: HashMap::new(),
            answered: HashMap::new(),
            replied: HashMap::new(),
            due: Vec::new(),
            spoken: VecDeque::new(),
            tally: BTreeMap::new(),
            rng: mix(seed, 0x5eed) | 1,
        }
    }

    /// The phrasebook in `temperament`'s voice, alone.
    pub fn phrasebook(temperament: &Temperament, seed: u64) -> Voice {
        Voice::new(Box::new(Phrasebook::new(temperament, seed)), temperament, seed)
    }

    /// A greeting for `who` at `server_ms`, unless it greeted them this game
    /// hour already. `None` while a model finds the words: they come from
    /// [`due`](Self::due).
    pub fn greet(&mut self, who: Uuid, moment: &Moment, server_ms: i64, now: Instant) -> Option<String> {
        let hour = server_ms.div_euclid(GAME_HOUR_MS);
        if self.greeted.get(&who) == Some(&hour) || !self.room(now) {
            return None;
        }
        self.greeted.insert(who, hour);
        self.speak(Occasion::Greeting, moment, now)
    }

    /// A line for `occasion` now, if the window has room for it. `None`
    /// while a model finds the words: they come from [`due`](Self::due).
    pub fn speak(&mut self, occasion: Occasion, moment: &Moment, now: Instant) -> Option<String> {
        if !self.room(now) {
            return None;
        }
        match self.ask(occasion, moment) {
            Coming::Ready(line) => {
                self.spoken.push_back(now);
                Some(line)
            }
            words => {
                self.due.push(Due {
                    at: now,
                    until: now + TALK_WAIT,
                    to: None,
                    words,
                });
                None
            }
        }
    }

    /// `from` named it in a local line: an answer is due 2 to 5 s from
    /// `now`, unless it answered them within [`ANSWER_AGAIN`]. True if one
    /// was.
    pub fn spoken_to(&mut self, from: Uuid, moment: &Moment, now: Instant) -> bool {
        if self.answered.get(&from).is_some_and(|at| now.duration_since(*at) < ANSWER_AGAIN) {
            return false;
        }
        self.answered.insert(from, now);
        let (least, most) = ANSWER_AFTER;
        let at = now + Duration::from_secs_f64(least + (most - least) * self.random());
        let words = self.ask(Occasion::Answer, moment);
        self.due.push(Due {
            at,
            until: at.max(now + TALK_WAIT),
            to: None,
            words,
        });
        true
    }

    /// The lines whose moment has come, as far as the window has room. One
    /// the window has no room for waits.
    pub fn due(&mut self, now: Instant) -> Vec<String> {
        self.settle(now);
        let mut out = Vec::new();
        self.due.sort_by_key(|d| d.at);
        while let Some(i) = self.due.iter().position(|d| d.to.is_none() && d.at <= now && matches!(d.words, Coming::Ready(_))) {
            if !self.room(now) {
                break;
            }
            let Coming::Ready(line) = self.due.remove(i).words else {
                unreachable!("found ready");
            };
            self.spoken.push_back(now);
            out.push(line);
        }
        out
    }

    /// Whether any line is waiting, out loud or by letter.
    pub fn has_due(&self) -> bool {
        !self.due.is_empty()
    }

    /// A reply to a letter from `from` (S1.5) is due, unless it replied to
    /// them this game day already. True if one was; it comes from
    /// [`replies`](Self::replies). Nothing calls it until the mailbox lands
    /// ([#432](https://github.com/chrischaps/TheCapitol/issues/432)). A
    /// letter isn't talk, so the window doesn't count it: the server's
    /// bucket counts letters with lines, but a reply a game day apart never
    /// comes near it.
    pub fn letter(&mut self, from: Uuid, moment: &Moment, server_ms: i64, now: Instant) -> bool {
        let day = server_ms.div_euclid(GAME_DAY_MS);
        if self.replied.get(&from) == Some(&day) {
            return false;
        }
        self.replied.insert(from, day);
        let words = self.ask(Occasion::Letter, moment);
        self.due.push(Due {
            at: now,
            until: now + LETTER_WAIT,
            to: Some(from),
            words,
        });
        true
    }

    /// The letters' replies that are ready: to whom, and the reply.
    pub fn replies(&mut self, now: Instant) -> Vec<(Uuid, String)> {
        self.settle(now);
        let (ready, waiting) = std::mem::take(&mut self.due)
            .into_iter()
            .partition::<Vec<_>, _>(|d| d.to.is_some() && matches!(d.words, Coming::Ready(_)));
        self.due = waiting;
        ready
            .into_iter()
            .filter_map(|d| match (d.to, d.words) {
                (Some(to), Coming::Ready(line)) => Some((to, line)),
                _ => None,
            })
            .collect()
    }

    /// What its speaker's lines have cost so far, in US cents.
    pub fn spent(&self) -> f64 {
        self.speaker.spent()
    }

    /// The lines it has found, by the speaker that found the words:
    /// [`PHRASEBOOK`] when a model had nothing in time. Nothing reports it
    /// yet ([#433](https://github.com/chrischaps/TheCapitol/issues/433)).
    pub fn tally(&self) -> &BTreeMap<String, usize> {
        &self.tally
    }

    fn count(&mut self, by: &str) {
        *self.tally.entry(by.to_string()).or_default() += 1;
    }

    /// The speaker's words for `occasion`, or what's coming of them.
    fn ask(&mut self, occasion: Occasion, moment: &Moment) -> Coming {
        match self.speaker.line(occasion, moment) {
            Words::Line(line) => {
                let name = self.speaker.name().to_string();
                self.count(&name);
                Coming::Ready(line)
            }
            Words::Pending(ticket) => Coming::Asked(ticket, self.book.say(occasion, moment)),
            Words::Nothing => {
                self.count(PHRASEBOOK);
                Coming::Ready(self.book.say(occasion, moment))
            }
        }
    }

    /// Takes in a model's words that have come, and the phrasebook's for
    /// those it has waited on long enough.
    fn settle(&mut self, now: Instant) {
        let name = self.speaker.name().to_string();
        let mut found = Vec::new();
        for d in &mut self.due {
            let Coming::Asked(ticket, fallback) = &mut d.words else {
                continue;
            };
            let (by, line) = match self.speaker.poll(*ticket) {
                Some(Words::Line(line)) => (name.as_str(), line),
                Some(_) => (PHRASEBOOK, std::mem::take(fallback)),
                None if now >= d.until => (PHRASEBOOK, std::mem::take(fallback)),
                None => continue,
            };
            found.push(by.to_string());
            d.words = Coming::Ready(line);
        }
        for by in found {
            self.count(&by);
        }
    }

    /// Room for a line in the window ending `now`.
    fn room(&mut self, now: Instant) -> bool {
        while self.spoken.front().is_some_and(|at| now.duration_since(*at) >= LINE_WINDOW) {
            self.spoken.pop_front();
        }
        self.spoken.len() < LINES_PER_WINDOW
    }

    /// Uniform in [0, 1), xorshift64*.
    fn random(&mut self) -> f64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// SplitMix64's finaliser over `a` and `b`.
fn mix(a: u64, b: u64) -> u64 {
    let mut z = a ^ b.wrapping_add(0x9E37_79B9_7F4A_7C15).rotate_left(17);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: Uuid = Uuid::from_u128(1);
    const BRAM: Uuid = Uuid::from_u128(2);
    /// Noon on some game day.
    const NOON: i64 = 1_000 * GAME_DAY_MS + GAME_DAY_MS / 2;

    fn temper(sociability: f32) -> Temperament {
        Temperament {
            sociability,
            ..Temperament::default()
        }
    }

    fn moment(who: &str, id: Uuid, ms: i64) -> Moment {
        Moment::with(who, id, Some(ms))
    }

    #[test]
    fn the_same_seed_and_moment_give_the_same_line() {
        let ada = moment("Ada Reeder", ADA, NOON);
        for occasion in Occasion::ALL {
            let mut one = Phrasebook::new(&temper(0.5), 42);
            let mut two = Phrasebook::new(&temper(0.5), 42);
            assert_eq!(one.line(occasion, &ada), two.line(occasion, &ada), "{occasion:?}");
        }
        // Over a day of moments and a few seeds, the whim moves round the
        // lines rather than sticking on one
        let mut seen = std::collections::HashSet::new();
        for seed in 0..4 {
            let book = Phrasebook::new(&temper(0.5), seed);
            for minute in 0..60 {
                seen.insert(book.say(Occasion::Greeting, &moment("Ada", ADA, NOON + minute * GAME_MINUTE_MS)));
            }
        }
        assert_eq!(seen.len(), lines(Occasion::Greeting, Band::Even).len());
    }

    #[test]
    fn every_occasion_has_lines_for_every_band() {
        for occasion in Occasion::ALL {
            for band in Band::ALL {
                let lines = lines(occasion, band);
                assert!(lines.len() >= 6, "{occasion:?} {band:?} has {}", lines.len());
                let distinct: std::collections::HashSet<_> = lines.iter().collect();
                assert_eq!(distinct.len(), lines.len(), "{occasion:?} {band:?} repeats a line");
                for line in lines {
                    // Only the two placeholders, and every line says cleanly
                    let bare = line.replace("{name}", "").replace("{hello}", "");
                    assert!(!bare.contains('{') && !bare.contains('}'), "{line}");
                    let said = fill(line, &moment("Ada Reeder", ADA, NOON));
                    assert_eq!(rules::talk::clean(&said).as_deref(), Ok(said.as_str()), "{line}");
                }
            }
        }
    }

    #[test]
    fn the_band_follows_sociability() {
        assert_eq!(Band::of(&temper(0.1)), Band::Reserved);
        assert_eq!(Band::of(&temper(0.5)), Band::Even);
        assert_eq!(Band::of(&temper(0.9)), Band::Warm);
    }

    #[test]
    fn names_and_hours_fill_in() {
        let line = fill("{hello}, {name}.", &moment("Ada Reeder", ADA, NOON));
        assert_eq!(line, "Good afternoon, Ada.");
        assert_eq!(fill("{name}!", &moment("chris", ADA, NOON)), "chris!");
        assert_eq!(hello(Some(0.3)), "Good morning");
        assert_eq!(hello(Some(0.8)), "Good evening");
        assert_eq!(hello(Some(0.05)), "Hello");
        assert_eq!(hello(None), "Hello");
    }

    #[test]
    fn a_line_names_a_citizen_by_its_first_name_or_its_whole_name() {
        assert!(names("Ada, have you any rope?", "Ada Reeder"));
        assert!(names("morning ada", "Ada Reeder"));
        assert!(names("Is that Ada's stall?", "Ada Reeder"));
        assert!(names("ask ada reeder", "Ada Reeder"));
        assert!(!names("Reeder", "Ada Reeder"), "a surname alone is a trade, not a name");
        assert!(!names("Adam was here", "Ada Reeder"));
        assert!(!names("anyone selling rope?", "Ada Reeder"));
    }

    #[test]
    fn a_person_is_greeted_once_a_game_hour() {
        let mut voice = Voice::phrasebook(&temper(0.5), 1);
        let now = Instant::now();
        let at = |ms| moment("Ada", ADA, ms);
        assert!(voice.greet(ADA, &at(NOON), NOON, now).is_some());
        let later = NOON + GAME_HOUR_MS / 2;
        assert!(voice.greet(ADA, &at(later), later, now + LINE_WINDOW).is_none(), "the same hour");
        assert!(voice.greet(BRAM, &at(later), later, now + LINE_WINDOW).is_some(), "someone else");
        let next = NOON + GAME_HOUR_MS;
        assert!(voice.greet(ADA, &at(next), next, now + LINE_WINDOW * 2).is_some(), "the next hour");
    }

    #[test]
    fn an_answer_comes_after_two_to_five_seconds_and_once_a_minute() {
        let mut voice = Voice::phrasebook(&temper(0.5), 3);
        let now = Instant::now();
        let ada = moment("Ada", ADA, NOON);
        assert!(voice.spoken_to(ADA, &ada, now));
        assert!(voice.due(now + Duration::from_millis(1999)).is_empty(), "too soon");
        assert_eq!(voice.due(now + Duration::from_secs(5)).len(), 1);
        assert!(!voice.has_due());
        assert!(!voice.spoken_to(ADA, &ada, now + Duration::from_secs(30)), "within the minute");
        assert!(voice.spoken_to(BRAM, &ada, now + Duration::from_secs(30)), "someone else");
        assert!(voice.spoken_to(ADA, &ada, now + ANSWER_AGAIN), "a minute on");
        // Over many, every delay lands in the 2 to 5 s
        let mut voice = Voice::phrasebook(&temper(0.5), 4);
        for n in 0..200u128 {
            let t = now + ANSWER_AGAIN * 2 * n as u32;
            assert!(voice.spoken_to(Uuid::from_u128(n), &ada, t));
            assert!(voice.due(t + Duration::from_millis(1999)).is_empty());
            assert_eq!(voice.due(t + Duration::from_secs(5)).len(), 1);
        }
    }

    #[test]
    fn it_never_says_more_than_the_window_holds() {
        let mut voice = Voice::phrasebook(&temper(0.9), 5);
        let start = Instant::now();
        let ada = moment("Ada", ADA, NOON);
        // A line asked for every 100 ms for a minute, and an answer due for
        // a new person every 100 ms
        let mut said: Vec<Instant> = Vec::new();
        for step in 0..600u32 {
            let now = start + Duration::from_millis(100) * step;
            if voice.speak(Occasion::Opening, &ada, now).is_some() {
                said.push(now);
            }
            voice.spoken_to(Uuid::from_u128(step as u128 + 10), &ada, now);
            said.extend(voice.due(now).iter().map(|_| now));
        }
        assert!(!said.is_empty());
        for (i, at) in said.iter().enumerate() {
            let within = said[i..].iter().take_while(|t| t.duration_since(*at) < LINE_WINDOW).count();
            assert!(within <= LINES_PER_WINDOW, "{within} lines in a window from {i}");
        }
    }

    #[test]
    fn a_letter_is_answered_once_a_game_day_per_sender() {
        let mut voice = Voice::phrasebook(&temper(0.9), 6);
        let now = Instant::now();
        let ada = moment("Ada", ADA, NOON);
        assert!(voice.letter(ADA, &ada, NOON, now));
        assert!(!voice.letter(ADA, &ada, NOON + GAME_HOUR_MS * 5, now));
        assert!(voice.letter(BRAM, &ada, NOON + GAME_HOUR_MS * 5, now));
        assert!(voice.letter(ADA, &ada, NOON + GAME_DAY_MS, now));
        let replies = voice.replies(now);
        assert_eq!(replies.iter().map(|(to, _)| *to).collect::<Vec<_>>(), [ADA, BRAM, ADA]);
        assert!(voice.due(now).is_empty(), "a reply is a letter, not a line out loud");
        assert!(!voice.has_due());
    }

    /// A model that answers when the test says: each line it's asked for
    /// pends, and `answer` gives the oldest its words.
    struct Slow {
        asked: Asked,
    }

    /// Each line asked for, by ticket, and its words once the test gives them.
    type Asked = std::sync::Arc<std::sync::Mutex<Vec<(u64, Option<Words>)>>>;

    impl Speaker for Slow {
        fn name(&self) -> &str {
            "slow"
        }

        fn line(&mut self, _: Occasion, _: &Moment) -> Words {
            let mut asked = self.asked.lock().unwrap();
            let ticket = asked.len() as u64;
            asked.push((ticket, None));
            Words::Pending(Ticket(ticket))
        }

        fn poll(&mut self, ticket: Ticket) -> Option<Words> {
            self.asked.lock().unwrap().iter_mut().find(|(t, _)| *t == ticket.0)?.1.take()
        }
    }

    fn slow() -> (Voice, Asked) {
        let asked = Asked::default();
        let speaker = Slow { asked: std::sync::Arc::clone(&asked) };
        (Voice::new(Box::new(speaker), &temper(0.5), 7), asked)
    }

    fn answer(asked: &Asked, ticket: u64, words: Words) {
        asked.lock().unwrap()[ticket as usize].1 = Some(words);
    }

    #[test]
    fn a_models_line_is_said_when_it_comes() {
        let (mut voice, asked) = slow();
        let now = Instant::now();
        let ada = moment("Ada", ADA, NOON);
        assert_eq!(voice.greet(ADA, &ada, NOON, now), None, "the world doesn't wait");
        assert!(voice.has_due());
        assert!(voice.due(now + Duration::from_millis(500)).is_empty(), "not yet come");
        answer(&asked, 0, Words::Line("Well met, Ada.".into()));
        assert_eq!(voice.due(now + Duration::from_millis(900)), ["Well met, Ada."]);
        assert!(!voice.has_due());
        assert_eq!(voice.tally().get("slow"), Some(&1));
        // A greeting waiting on the model is still a greeting this hour
        assert_eq!(voice.greet(ADA, &ada, NOON, now + LINE_WINDOW), None);
        assert_eq!(asked.lock().unwrap().len(), 1, "and isn't asked again");
    }

    #[test]
    fn the_phrasebook_speaks_when_a_model_is_late_or_has_nothing() {
        let (mut voice, asked) = slow();
        let now = Instant::now();
        let ada = moment("Ada", ADA, NOON);
        let book = Phrasebook::new(&temper(0.5), 7);
        // Late: the phrasebook's own line, at the end of the wait
        assert_eq!(voice.speak(Occasion::Accepting, &ada, now), None);
        assert!(voice.due(now + TALK_WAIT - Duration::from_millis(1)).is_empty());
        assert_eq!(voice.due(now + TALK_WAIT), [book.say(Occasion::Accepting, &ada)]);
        // An answer past its wait is dropped, not said twice
        answer(&asked, 0, Words::Line("Too late.".into()));
        assert!(voice.due(now + TALK_WAIT * 2).is_empty());
        // Nothing (a refusal, a ramble): the phrasebook, at once
        let later = now + LINE_WINDOW;
        assert_eq!(voice.speak(Occasion::Declining, &ada, later), None);
        answer(&asked, 1, Words::Nothing);
        assert_eq!(voice.due(later), [book.say(Occasion::Declining, &ada)]);
        assert_eq!(voice.tally().get(PHRASEBOOK), Some(&2));
    }

    #[test]
    fn a_models_answer_still_waits_for_its_moment() {
        let (mut voice, asked) = slow();
        let now = Instant::now();
        let mut ada = moment("Ada", ADA, NOON);
        ada.heard = Some("Bram, any rope?".into());
        assert!(voice.spoken_to(ADA, &ada, now));
        answer(&asked, 0, Words::Line("Not today, Ada.".into()));
        assert!(voice.due(now + Duration::from_millis(1999)).is_empty(), "too soon, though it's come");
        assert_eq!(voice.due(now + Duration::from_secs(5)), ["Not today, Ada."]);
    }

    #[test]
    fn a_letters_reply_waits_longer_for_a_model() {
        let (mut voice, asked) = slow();
        let now = Instant::now();
        let ada = moment("Ada", ADA, NOON);
        assert!(voice.letter(ADA, &ada, NOON, now));
        assert!(voice.replies(now + TALK_WAIT * 5).is_empty(), "a letter is no conversation");
        answer(&asked, 0, Words::Line("Dear Ada, thank you.".into()));
        assert_eq!(voice.replies(now + TALK_WAIT * 5), [(ADA, "Dear Ada, thank you.".to_string())]);
        assert!(voice.letter(BRAM, &ada, NOON, now));
        assert_eq!(voice.replies(now + LETTER_WAIT).len(), 1, "the phrasebook's, at the end of the wait");
    }
}
