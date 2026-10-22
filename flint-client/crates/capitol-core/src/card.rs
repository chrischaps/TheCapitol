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
//! how long someone has been in the city, and where they live.

use capitol_protocol::rest::PlayerCard;
use capitol_protocol::rules::Zone;
use uuid::Uuid;

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

/// Where someone lives: the zone of their oldest plot.
pub fn home_words(home: Option<&str>, mine: bool) -> String {
    let Some(zone) = home else {
        return if mine { "No land of your own yet".into() } else { "No land of their own yet".into() };
    };
    match Zone::from_str(zone) {
        Some(Zone::Capitol) => "Lives in the Capitol".into(),
        Some(Zone::Rural) => "Farms in the Rural Zone".into(),
        Some(z) => format!("Lives in the {}", z.display_name()),
        None => format!("Lives in the {zone}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::standing;

    fn card(n: u128, name: &str) -> PlayerCard {
        PlayerCard {
            id: Uuid::from_u128(n),
            name: name.into(),
            citizen: false,
            since: 0,
            home: None,
            standing: standing::NEW_IN_TOWN.into(),
            guild: None,
            mine: false,
        }
    }

    fn hint(name: &str) -> Option<Hint> {
        Some(Hint { name: name.into(), citizen: true })
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
        assert_eq!(home_words(Some("suburban"), false), "Lives in the Suburban Zone");
        assert_eq!(home_words(Some("rural"), false), "Farms in the Rural Zone");
        assert_eq!(home_words(Some("capitol"), true), "Lives in the Capitol");
        assert_eq!(home_words(None, false), "No land of their own yet");
        assert_eq!(home_words(None, true), "No land of your own yet");
    }
}
