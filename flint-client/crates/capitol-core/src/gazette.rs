//! The Capitol Gazette (E3.9), as the client reads it: the latest edition,
//! the back issues you've turned to, whether there's one you haven't read,
//! and your byline.
//!
//! [`Reader`] is pure. The session makes the calls it asks for
//! ([`Reader::wants`]) and hands it the answers. The server tells everyone
//! when an edition is out (`gazette_published`), and the header hints at it
//! until you open it ([`Reader::unread`]). Which day you last read is kept
//! by the game between sessions, so the hint doesn't come back each login.

use std::collections::BTreeMap;

use capitol_protocol::rest::{Gazette, GazetteResponse};
use capitol_protocol::rules;

/// What the Gazette has to say, and where you are in it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reader {
    /// The editions fetched, by day
    editions: BTreeMap<i64, Gazette>,
    /// Days asked for that have no edition: before the first, or a day
    /// the server was down at its midnight
    missing: Vec<i64>,
    /// The newest day the server has an edition for, as far as we know
    latest: Option<i64>,
    /// Its headline, from `gazette_published` before the edition comes
    latest_headline: Option<String>,
    /// The newest day you've opened
    read: Option<i64>,
    /// The edition on the page: `None` for the latest
    showing: Option<i64>,
    /// Whether the Gazette prints your name, once the server has said
    named: Option<bool>,
    /// A call that's out: don't send it again until it's answered
    asked: Option<Ask>,
    /// The latest is worth asking for (connect, a new edition)
    stale: bool,
}

/// A call the reader wants made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// `GET /gazette/latest`
    Latest,
    /// `GET /gazette?date=`
    Day(i64),
    /// `PUT /gazette/byline`
    Byline(bool),
}

impl Reader {
    /// A reader who last opened day `read`'s edition (kept by the game).
    pub fn new(read: Option<i64>) -> Self {
        Self { read, stale: true, ..Default::default() }
    }

    /// Which day you last opened, as the game kept it, for whoever signed in.
    pub fn set_read(&mut self, read: Option<i64>) {
        self.read = read;
    }

    /// Signed in again: ask for the latest. What's been fetched stays.
    pub fn connected(&mut self) {
        self.stale = true;
        self.asked = None;
    }

    /// `gazette_published`: an edition is out.
    pub fn published(&mut self, day: i64, headline: &str) {
        if self.latest.is_none_or(|l| day > l) {
            self.latest = Some(day);
            self.latest_headline = Some(headline.to_owned());
        }
        if !self.editions.contains_key(&day) {
            self.stale = true;
        }
    }

    /// The call to make next, if any. Mark it sent with [`Reader::sent`].
    pub fn wants(&self) -> Option<Ask> {
        if self.asked.is_some() {
            return None;
        }
        if self.stale {
            return Some(Ask::Latest);
        }
        match self.showing {
            Some(day) if !self.editions.contains_key(&day) && !self.missing.contains(&day) => Some(Ask::Day(day)),
            _ => None,
        }
    }

    pub fn sent(&mut self, ask: Ask) {
        if ask == Ask::Latest {
            self.stale = false;
        }
        self.asked = Some(ask);
    }

    /// The answer to `ask`.
    pub fn answered(&mut self, ask: Ask, response: GazetteResponse) {
        if self.asked == Some(ask) {
            self.asked = None;
        }
        self.named = Some(response.named);
        match response.edition {
            Some(edition) => {
                let day = edition.day;
                if ask == Ask::Latest && self.latest.is_none_or(|l| day >= l) {
                    self.latest = Some(day);
                    self.latest_headline = Some(edition.headline.clone());
                }
                self.editions.insert(day, edition);
            }
            None => {
                if let Ask::Day(day) = ask {
                    self.missing.push(day);
                }
            }
        }
    }

    /// The byline call came back: `named` as it now stands.
    pub fn byline(&mut self, named: bool) {
        if matches!(self.asked, Some(Ask::Byline(_))) {
            self.asked = None;
        }
        self.named = Some(named);
    }

    /// A call failed: let it be asked again later.
    pub fn failed(&mut self, ask: Ask) {
        if self.asked == Some(ask) {
            self.asked = None;
        }
    }

    /// Ask the server to print your name, or not.
    pub fn set_named(&mut self, named: bool) -> Option<Ask> {
        if self.asked.is_some() || self.named == Some(named) {
            return None;
        }
        Some(Ask::Byline(named))
    }

    pub fn named(&self) -> Option<bool> {
        self.named
    }

    /// The paper is opened: show the latest, and count it read. The day
    /// read, for the game to keep, if it moved.
    pub fn open(&mut self) -> Option<i64> {
        self.showing = None;
        self.mark_read()
    }

    /// Count the latest edition read, if it's here and wasn't. The day, for
    /// the game to keep, if it moved.
    pub fn mark_read(&mut self) -> Option<i64> {
        let latest = self.latest?;
        if self.read.is_none_or(|r| latest > r) && self.editions.contains_key(&latest) {
            self.read = Some(latest);
            return self.read;
        }
        None
    }

    /// There's an edition out you haven't opened.
    pub fn unread(&self) -> bool {
        self.latest.is_some_and(|l| self.read.is_none_or(|r| l > r))
    }

    /// The latest edition's headline, for the header's hint.
    pub fn latest_headline(&self) -> Option<&str> {
        self.latest_headline.as_deref()
    }

    /// The day on the page.
    pub fn showing_day(&self) -> Option<i64> {
        self.showing.or(self.latest)
    }

    /// The edition on the page, once it's here.
    pub fn showing(&self) -> Option<&Gazette> {
        self.editions.get(&self.showing_day()?)
    }

    /// The page's day has no edition (asked, and the server had none).
    pub fn showing_missing(&self) -> bool {
        self.showing_day().is_some_and(|d| self.missing.contains(&d))
    }

    /// Turn to the day before, or after. Never past the latest, nor before
    /// the first edition.
    pub fn turn(&mut self, by: i64) {
        let (Some(day), Some(latest)) = (self.showing_day(), self.latest) else {
            return;
        };
        let to = (day + by).clamp(rules::GAZETTE_FIRST_DAY.min(latest), latest);
        self.showing = (to != latest).then_some(to);
        if to == latest {
            self.open();
        }
    }

    /// Whether there's a day before the page's to turn to, and one after.
    pub fn can_turn(&self) -> (bool, bool) {
        match (self.showing_day(), self.latest) {
            (Some(day), Some(latest)) => (day > rules::GAZETTE_FIRST_DAY.min(latest), day < latest),
            _ => (false, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = rules::GAZETTE_FIRST_DAY + 12;

    fn edition(day: i64) -> Gazette {
        Gazette {
            day,
            number: rules::gazette_number(day),
            headline: format!("News of day {day}"),
            ..Default::default()
        }
    }

    fn answer(edition: Option<Gazette>) -> GazetteResponse {
        GazetteResponse { edition, named: true }
    }

    #[test]
    fn it_asks_for_the_latest_once_and_reads_it() {
        let mut r = Reader::new(None);
        assert_eq!(r.wants(), Some(Ask::Latest));
        r.sent(Ask::Latest);
        assert_eq!(r.wants(), None);
        r.answered(Ask::Latest, answer(Some(edition(DAY))));
        assert_eq!(r.wants(), None);
        assert!(r.unread());
        assert_eq!(r.latest_headline(), Some("News of day 248952"));
        assert_eq!(r.open(), Some(DAY));
        assert!(!r.unread());
        assert_eq!(r.showing().map(|g| g.day), Some(DAY));
        assert_eq!(r.named(), Some(true));
    }

    #[test]
    fn a_read_edition_stays_read_after_a_restart() {
        let mut r = Reader::new(Some(DAY));
        r.sent(Ask::Latest);
        r.answered(Ask::Latest, answer(Some(edition(DAY))));
        assert!(!r.unread());
        assert_eq!(r.open(), None);
    }

    #[test]
    fn a_new_edition_is_unread_and_fetched() {
        let mut r = Reader::new(Some(DAY));
        r.sent(Ask::Latest);
        r.answered(Ask::Latest, answer(Some(edition(DAY))));
        r.published(DAY + 1, "Fresh news");
        assert!(r.unread());
        assert_eq!(r.latest_headline(), Some("Fresh news"));
        assert_eq!(r.wants(), Some(Ask::Latest));
        // Opened before it's here: not read yet
        assert_eq!(r.open(), None);
        assert!(r.unread());
        r.sent(Ask::Latest);
        r.answered(Ask::Latest, answer(Some(edition(DAY + 1))));
        assert_eq!(r.open(), Some(DAY + 1));
    }

    #[test]
    fn no_edition_yet_is_nothing_to_read() {
        let mut r = Reader::new(None);
        r.sent(Ask::Latest);
        r.answered(Ask::Latest, answer(None));
        assert!(!r.unread());
        assert_eq!(r.showing(), None);
        assert_eq!(r.can_turn(), (false, false));
    }

    #[test]
    fn back_issues_are_asked_for_once_and_turning_stops_at_the_ends() {
        let mut r = Reader::new(None);
        r.sent(Ask::Latest);
        r.answered(Ask::Latest, answer(Some(edition(DAY))));
        r.open();
        r.turn(-1);
        assert_eq!(r.showing_day(), Some(DAY - 1));
        assert_eq!(r.wants(), Some(Ask::Day(DAY - 1)));
        r.sent(Ask::Day(DAY - 1));
        r.answered(Ask::Day(DAY - 1), answer(None));
        assert!(r.showing_missing());
        assert_eq!(r.wants(), None);
        r.turn(-100);
        assert_eq!(r.showing_day(), Some(rules::GAZETTE_FIRST_DAY));
        assert_eq!(r.can_turn(), (false, true));
        r.turn(100);
        assert_eq!(r.showing_day(), Some(DAY));
        assert_eq!(r.can_turn(), (true, false));
    }

    #[test]
    fn the_byline_is_asked_only_when_it_would_change() {
        let mut r = Reader::new(None);
        r.sent(Ask::Latest);
        r.answered(Ask::Latest, answer(None));
        assert_eq!(r.set_named(true), None);
        let ask = r.set_named(false).unwrap();
        r.sent(ask);
        assert_eq!(r.set_named(false), None);
        r.byline(false);
        assert_eq!(r.named(), Some(false));
    }

    #[test]
    fn a_failed_call_is_asked_again() {
        let mut r = Reader::new(None);
        r.sent(Ask::Latest);
        r.failed(Ask::Latest);
        r.connected();
        assert_eq!(r.wants(), Some(Ask::Latest));
    }
}
