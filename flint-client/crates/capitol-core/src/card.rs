//! A person's card (S0.3), as the client keeps it: whose card is wanted, the
//! call to make for it, and what came back.
//!
//! [`Cards`] is pure. A click on a person asks for their card by id; a name
//! in the Gazette (and in the chat log, S1.3) asks by name. The session makes
//! the call [`Cards::wants`] names and hands back the answer. While it's out,
//! what the click already knew (the name over their head, the citizen's
//! mark) is shown, so the card opens at once and fills in.
//!
//! The words a card says are here too, so they're tested without egui:
//! how long someone has been in the city, where they live, and what they've
//! been busy with lately (K3.1, `lately_words`). On your own
//! card, the number under your standing's words, its trend over the week and
//! your history (S3.1): `score_words`, `trend_words`, `when_words`,
//! `history_line`.

use capitol_protocol::rest::PlayerCard;
use capitol_protocol::rules::standing::{self, Record};
use capitol_protocol::rules::{self, Zone};
use uuid::Uuid;

use crate::catalog::Catalog;

/// Whose card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Who {
    /// `GET /players/{id}/card`: a person you clicked
    Id(Uuid),
    /// `GET /players/by-name/{name}`: a name on a page
    Name(String),
}

impl Who {
    /// Whether `card` is this one's.
    fn is(&self, card: &PlayerCard) -> bool {
        match self {
            Who::Id(id) => card.id == *id,
            Who::Name(name) => card.name.to_lowercase() == name.trim().to_lowercase(),
        }
    }
}

/// What the click knew before the server answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hint {
    pub name: String,
    pub citizen: bool,
}

/// Where the card stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shown<'a> {
    /// Nothing is open.
    Closed,
    /// Asked for; what the click knew, if anything.
    Asking(Option<&'a Hint>),
    /// The server's answer.
    Card(&'a PlayerCard),
    /// No one by that id or name (a 404).
    Missing,
    /// The call failed some other way.
    Failed,
}

/// The card open, if any, and the call for it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cards {
    want: Option<Who>,
    hint: Option<Hint>,
    card: Option<PlayerCard>,
    /// The call that's out: don't send another until it's answered
    asked: Option<Who>,
    /// The last call for `want` came back with no one, or failed
    missing: bool,
    failed: bool,
}

impl Cards {
    /// Open `who`'s card. Asking again for the one open keeps it.
    pub fn open(&mut self, who: Who, hint: Option<Hint>) {
        if self.want.as_ref() == Some(&who) && (self.card.is_some() || self.asked.is_some()) {
            return;
        }
        // The card you already hold, by its other key: a name, then a click
        if let Some(card) = self.card.as_ref().filter(|c| who.is(c)) {
            let card = card.clone();
            *self = Cards { want: Some(who), card: Some(card), asked: self.asked.take(), ..Default::default() };
            return;
        }
        *self = Cards { want: Some(who), hint, asked: self.asked.take(), ..Default::default() };
    }

    pub fn close(&mut self) {
        *self = Cards { asked: self.asked.take(), ..Default::default() };
    }

    pub fn is_open(&self) -> bool {
        self.want.is_some()
    }

    /// The call to make next, if any. Mark it sent with [`Cards::sent`].
    pub fn wants(&self) -> Option<Who> {
        if self.asked.is_some() || self.card.is_some() || self.missing || self.failed {
            return None;
        }
        self.want.clone()
    }

    pub fn sent(&mut self, who: Who) {
        self.asked = Some(who);
    }

    /// The answer to the call that was out. One for a card no longer
    /// wanted is dropped, and the one wanted now is asked for next.
    pub fn answered(&mut self, card: PlayerCard) {
        let Some(asked) = self.asked.take() else { return };
        if self.want.as_ref() == Some(&asked) {
            self.card = Some(card);
        }
    }

    /// The call came back with no one (`not_found`, a 404) or failed.
    pub fn failed(&mut self, not_found: bool) {
        let Some(asked) = self.asked.take() else { return };
        if self.want.as_ref() == Some(&asked) {
            if not_found {
                self.missing = true;
            } else {
                self.failed = true;
            }
        }
    }

    /// You muted or unmuted `id` (S1.4): their card, if it's the one held,
    /// says so.
    pub fn set_muted(&mut self, id: Uuid, muted: bool) {
        if let Some(card) = self.card.as_mut().filter(|c| c.id == id) {
            card.muted = muted;
        }
    }

    /// Signed in again: whatever was out won't be answered.
    pub fn connected(&mut self) {
        self.asked = None;
    }

    pub fn shown(&self) -> Shown<'_> {
        match (&self.want, &self.card) {
            (None, _) => Shown::Closed,
            (Some(_), Some(card)) => Shown::Card(card),
            _ if self.missing => Shown::Missing,
            _ if self.failed => Shown::Failed,
            _ => Shown::Asking(self.hint.as_ref()),
        }
    }
}

/// How long someone has been in the city, from the game day they came
/// (`since`) and today's (`rules::day_number`). A game day is two hours,
/// so a week of play is weeks here.
pub fn time_in_city(since: i64, today: i64) -> String {
    match today - since {
        ..=0 => "Came to the city today".into(),
        1 => "In the city since yesterday".into(),
        n @ 2..=13 => format!("In the city {n} days"),
        n => format!("In the city {} weeks", n / 7),
    }
}

/// What they've been busy with these last few game days, after "Lately"
/// (K3.1): "felling trees and sawing planks, and selling at the bazaar".
/// `None` when the card has nothing to say, or they keep out of the papers.
pub fn lately_words(card: &PlayerCard) -> Option<String> {
    card.lately.as_ref().filter(|l| !l.is_empty()).map(rules::lately::words)
}

/// Where someone lives: their oldest plot's address, "Farms at 3 Sheepwash
/// Lane", "Lives at 7 Larch Close" (#573), or its zone where it has none
/// (a server from before, or a lot on a street with no name).
pub fn home_words(home: Option<&str>, address: Option<&str>, mine: bool) -> String {
    let Some(zone) = home else {
        return if mine { "No land of your own yet".into() } else { "No land of their own yet".into() };
    };
    match (Zone::from_str(zone), address) {
        (Some(Zone::Capitol), _) => "Lives in the Capitol".into(),
        (Some(Zone::Rural), Some(address)) => format!("Farms at {address}"),
        (Some(Zone::Rural), None) => "Farms in the Rural Zone".into(),
        (_, Some(address)) => format!("Lives at {address}"),
        (Some(z), None) => format!("Lives in the {}", z.display_name()),
        (None, None) => format!("Lives in the {zone}"),
    }
}

/// The zone their home is in, for under its address: "In the Rural Zone".
/// `None` without a home, or in the Capitol, which its words already say.
pub fn home_zone_words(home: Option<&str>) -> Option<String> {
    match Zone::from_str(home?) {
        Some(Zone::Capitol) => None,
        Some(z) => Some(format!("In the {}", z.display_name())),
        None => Some(format!("In the {}", home?)),
    }
}

/// Your standing's number, to a tenth.
pub fn score_words(score: f64) -> String {
    format!("{score:.1}")
}

/// Which way the week went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    Up,
    Down,
    Steady,
}

/// The score's move over the last `rules::standing::TREND_DAYS` game days:
/// "▲ 1.5 this week", "▼ 2.7 this week", "Steady this week".
pub fn trend_words(trend: f64) -> (String, Trend) {
    if trend >= 0.05 {
        (format!("▲ {trend:.1} this week"), Trend::Up)
    } else if trend <= -0.05 {
        (format!("▼ {:.1} this week", -trend), Trend::Down)
    } else {
        ("Steady this week".into(), Trend::Steady)
    }
}

/// When a record was, in game days from `today`.
pub fn when_words(record: &Record, today: i64) -> String {
    match today - rules::day_number(record.at.timestamp_millis()) {
        ..=0 => "today".into(),
        1 => "yesterday".into(),
        n @ 2..=13 => format!("{n} days ago"),
        n => format!("{} weeks ago", n / 7),
    }
}

/// A line of your history, its goods named as the catalog names them.
pub fn history_line(record: &Record, catalog: &Catalog) -> String {
    let item = record.item_type.as_deref().map(|t| catalog.item_name(t)).unwrap_or_default();
    standing::line(record, &item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(n: u128, name: &str) -> PlayerCard {
        PlayerCard {
            id: Uuid::from_u128(n),
            name: name.into(),
            citizen: false,
            since: 0,
            home: None,
            home_address: None,
            standing: standing::NEW_IN_TOWN.into(),
            guild: None,
            lately: None,
            mine: false,
            muted: false,
            own: None,
        }
    }

    fn hint(name: &str) -> Option<Hint> {
        Some(Hint { name: name.into(), citizen: true })
    }

    #[test]
    fn a_card_says_what_theyve_been_busy_with_only_when_theres_something() {
        use capitol_protocol::rules::lately::{Lately, Venue};
        assert_eq!(lately_words(&card(1, "Ada")), None);
        let quiet = PlayerCard { lately: Some(Lately::default()), ..card(1, "Ada") };
        assert_eq!(lately_words(&quiet), None);
        let busy = Lately { doing: vec!["digging clay".into(), "firing bricks".into()], sells: vec![Venue::Exchange] };
        let busy = PlayerCard { lately: Some(busy), ..card(1, "Ada") };
        assert_eq!(lately_words(&busy).as_deref(), Some("digging clay and firing bricks, and selling on the Exchange"));
    }

    #[test]
    fn a_click_shows_the_name_at_once_and_asks_once() {
        let mut cards = Cards::default();
        assert_eq!(cards.shown(), Shown::Closed);
        let ada = Who::Id(Uuid::from_u128(1));
        cards.open(ada.clone(), hint("Ada"));
        assert_eq!(cards.wants(), Some(ada.clone()));
        cards.sent(ada.clone());
        assert_eq!(cards.wants(), None, "one call at a time");
        assert!(matches!(cards.shown(), Shown::Asking(Some(h)) if h.name == "Ada"));
        // A second click on her while it's out sends nothing more
        cards.open(ada.clone(), hint("Ada"));
        assert_eq!(cards.wants(), None);
        cards.answered(card(1, "Ada"));
        assert!(matches!(cards.shown(), Shown::Card(c) if c.name == "Ada"));
        assert_eq!(cards.wants(), None);
    }

    #[test]
    fn a_late_answer_for_someone_else_is_dropped_and_the_new_one_asked() {
        let mut cards = Cards::default();
        let (ada, bea) = (Who::Id(Uuid::from_u128(1)), Who::Name("bea".into()));
        cards.open(ada.clone(), None);
        cards.sent(ada);
        cards.open(bea.clone(), None);
        assert_eq!(cards.wants(), None, "still waiting on Ada's");
        cards.answered(card(1, "Ada"));
        assert!(matches!(cards.shown(), Shown::Asking(None)));
        assert_eq!(cards.wants(), Some(bea.clone()));
        cards.sent(bea);
        cards.answered(card(2, "Bea"));
        assert!(matches!(cards.shown(), Shown::Card(c) if c.name == "Bea"));
    }

    #[test]
    fn no_one_by_that_name_says_so_and_isnt_asked_again() {
        let mut cards = Cards::default();
        let who = Who::Name("Nobody".into());
        cards.open(who.clone(), None);
        cards.sent(who.clone());
        cards.failed(true);
        assert_eq!(cards.shown(), Shown::Missing);
        assert_eq!(cards.wants(), None);
        // Opened afresh, it's asked again
        cards.close();
        cards.open(who.clone(), None);
        assert_eq!(cards.wants(), Some(who.clone()));
        cards.sent(who);
        cards.failed(false);
        assert_eq!(cards.shown(), Shown::Failed);
    }

    #[test]
    fn the_card_you_hold_opens_by_its_name_without_a_call() {
        let mut cards = Cards::default();
        let ada = Who::Id(Uuid::from_u128(1));
        cards.open(ada.clone(), None);
        cards.sent(ada);
        cards.answered(card(1, "Ada Reeder"));
        cards.open(Who::Name(" ada reeder".into()), None);
        assert!(matches!(cards.shown(), Shown::Card(c) if c.id == Uuid::from_u128(1)));
        assert_eq!(cards.wants(), None);
    }

    #[test]
    fn muting_marks_the_card_held_and_no_other() {
        let mut cards = Cards::default();
        let ada = Who::Id(Uuid::from_u128(1));
        cards.open(ada.clone(), None);
        cards.sent(ada);
        cards.answered(card(1, "Ada"));
        cards.set_muted(Uuid::from_u128(2), true);
        assert!(matches!(cards.shown(), Shown::Card(c) if !c.muted));
        cards.set_muted(Uuid::from_u128(1), true);
        assert!(matches!(cards.shown(), Shown::Card(c) if c.muted));
    }

    #[test]
    fn time_in_the_city_reads_in_days_then_weeks() {
        assert_eq!(time_in_city(100, 100), "Came to the city today");
        assert_eq!(time_in_city(100, 101), "In the city since yesterday");
        assert_eq!(time_in_city(100, 109), "In the city 9 days");
        assert_eq!(time_in_city(100, 120), "In the city 2 weeks");
        // A clock behind the server's never says "-1 days"
        assert_eq!(time_in_city(100, 99), "Came to the city today");
    }

    #[test]
    fn home_names_the_zone_or_says_there_is_none() {
        assert_eq!(home_words(Some("suburban"), None, false), "Lives in the Suburban Zone");
        assert_eq!(home_words(Some("rural"), None, false), "Farms in the Rural Zone");
        assert_eq!(home_words(Some("capitol"), None, true), "Lives in the Capitol");
        assert_eq!(home_words(None, None, false), "No land of their own yet");
        assert_eq!(home_words(None, None, true), "No land of your own yet");
    }

    #[test]
    fn home_says_its_address_where_it_has_one() {
        assert_eq!(home_words(Some("rural"), Some("3 Sheepwash Lane"), false), "Farms at 3 Sheepwash Lane");
        assert_eq!(home_words(Some("suburban"), Some("7 Larch Close"), true), "Lives at 7 Larch Close");
        assert_eq!(
            home_words(Some("urban"), Some("112 Mercer Street by Tannergate"), false),
            "Lives at 112 Mercer Street by Tannergate"
        );
        assert_eq!(home_zone_words(Some("rural")).as_deref(), Some("In the Rural Zone"));
        assert_eq!(home_zone_words(Some("capitol")), None);
        assert_eq!(home_zone_words(None), None);
    }

    #[test]
    fn your_number_and_its_week_read_to_a_tenth() {
        assert_eq!(score_words(8.25), "8.2");
        assert_eq!(score_words(-2.0), "-2.0");
        assert_eq!(trend_words(1.46), ("▲ 1.5 this week".into(), Trend::Up));
        assert_eq!(trend_words(-2.7), ("▼ 2.7 this week".into(), Trend::Down));
        assert_eq!(trend_words(0.01), ("Steady this week".into(), Trend::Steady));
    }

    #[test]
    fn a_line_of_history_names_its_goods_and_its_day() {
        use capitol_protocol::rules::standing::Deed;
        let today = rules::GAZETTE_FIRST_DAY + 30;
        let on = |day: i64| chrono::DateTime::from_timestamp_millis(rules::day_start_ms(day) + 60_000).unwrap();
        let kept = Record { at: on(today - 1), deed: Deed::Kept { units: 10 }, item_type: Some("iron_ore".into()), other: Some("Ada".into()) };
        let catalog = Catalog::fallback();
        assert_eq!(history_line(&kept, &catalog), format!("Kept a claim: 10 {} for Ada", catalog.item_name("iron_ore").to_lowercase()));
        assert_eq!(when_words(&kept, today), "yesterday");
        assert_eq!(when_words(&Record { at: on(today), ..kept.clone() }, today), "today");
        assert_eq!(when_words(&Record { at: on(today - 5), ..kept.clone() }, today), "5 days ago");
        assert_eq!(when_words(&Record { at: on(today - 15), ..kept.clone() }, today), "2 weeks ago");
        let week = Record { at: on(today), deed: Deed::TaxWeek, item_type: None, other: None };
        assert_eq!(history_line(&week, &catalog), "A week of property tax paid in full");
    }
}
