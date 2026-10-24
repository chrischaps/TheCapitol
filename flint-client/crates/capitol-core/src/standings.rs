//! The week's leaderboards (commons S3.2), as the board's *Standings* page
//! reads them: which week, whole-city or one home zone, citizens in or
//! people only, and the four boards the server worked out at the week's end.
//!
//! [`Standings`] is pure. The session makes the call it asks for
//! ([`Standings::wants`]) and hands it the answer. Every board comes in one
//! call, so turning between them costs nothing; a change of week, zone or
//! filter asks again. Opening the page asks again too, since a new week's
//! boards may be up.

use capitol_protocol::rest::{Leaderboard, LeaderboardKind, LeaderboardQuery, LeaderboardsResponse};
use capitol_protocol::rules::{self, Zone};

/// The Standings page's view and what the server said of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Standings {
    /// What's asked for: every board, so `board` is always `None`
    query: LeaderboardQuery,
    /// The board on the page
    pub board: LeaderboardKind,
    /// The latest answer, for `shown`
    answer: Option<LeaderboardsResponse>,
    /// The view it answers, which may be behind `query` while one's out
    shown: Option<LeaderboardQuery>,
    /// A call that's out: don't send another until it's answered
    asked: Option<LeaderboardQuery>,
    /// Worth asking for again
    stale: bool,
}

impl Default for Standings {
    fn default() -> Self {
        Self::new()
    }
}

impl Standings {
    pub fn new() -> Self {
        Self {
            query: LeaderboardQuery::default(),
            board: LeaderboardKind::default(),
            answer: None,
            shown: None,
            asked: None,
            stale: true,
        }
    }

    /// Signed in again, or the page opened: ask again.
    pub fn refresh(&mut self) {
        self.stale = true;
    }

    /// Signed in again: what was out won't be answered.
    pub fn connected(&mut self) {
        self.stale = true;
        self.asked = None;
    }

    /// The call to make next, if any. Mark it sent with [`Standings::sent`].
    pub fn wants(&self) -> Option<LeaderboardQuery> {
        (self.asked.is_none() && self.stale).then(|| self.query.clone())
    }

    pub fn sent(&mut self, query: LeaderboardQuery) {
        if query == self.query {
            self.stale = false;
        }
        self.asked = Some(query);
    }

    /// The answer to `query`. One for a view since left is kept until the
    /// new one comes, so the page doesn't blank as you click.
    pub fn answered(&mut self, query: LeaderboardQuery, response: LeaderboardsResponse) {
        if self.asked.as_ref() == Some(&query) {
            self.asked = None;
        }
        // The first week asked for by default is the latest: pin it, so the
        // arrows turn from it
        if self.query.week.is_none() && query == self.query {
            self.query.week = response.week;
            self.shown = Some(self.query.clone());
        } else {
            self.shown = Some(query);
        }
        self.answer = Some(response);
    }

    /// A call failed: let it be asked again later.
    pub fn failed(&mut self, query: &LeaderboardQuery) {
        if self.asked.as_ref() == Some(query) {
            self.asked = None;
        }
    }

    /// Show one home zone's boards, or with `None` the whole city's.
    pub fn set_zone(&mut self, zone: Option<String>) {
        if self.query.zone != zone {
            self.query.zone = zone;
            self.stale = true;
        }
    }

    /// Leave citizens out, or put them back.
    pub fn set_people(&mut self, people: bool) {
        if self.query.people != people {
            self.query.people = people;
            self.stale = true;
        }
    }

    /// Turn back (-1) or on (+1) a week, never past the latest.
    pub fn turn(&mut self, by: i64) {
        let (Some(week), Some(latest)) = (self.week(), self.latest()) else {
            return;
        };
        let to = (week + by).clamp(0.min(latest), latest);
        if Some(to) != self.query.week {
            self.query.week = Some(to);
            self.stale = true;
        }
    }

    /// The zone asked for: `None` is the whole city.
    pub fn zone(&self) -> Option<&str> {
        self.query.zone.as_deref()
    }

    pub fn people(&self) -> bool {
        self.query.people
    }

    /// The week on the page, once there's one.
    pub fn week(&self) -> Option<i64> {
        self.query.week.or(self.answer.as_ref().and_then(|a| a.week))
    }

    /// The latest week with boards.
    pub fn latest(&self) -> Option<i64> {
        self.answer.as_ref().and_then(|a| a.latest)
    }

    /// The answer's home zones, from the centre out.
    pub fn zones(&self) -> &[String] {
        self.answer.as_ref().map_or(&[], |a| &a.zones)
    }

    /// Waiting on the server for the view as it now stands.
    pub fn loading(&self) -> bool {
        self.shown.as_ref() != Some(&self.query)
    }

    /// Whether there have ever been boards: the first week has ended.
    pub fn any(&self) -> bool {
        self.latest().is_some()
    }

    /// The board on the page, as last answered.
    pub fn shown(&self) -> Option<&Leaderboard> {
        self.answer.as_ref()?.boards.iter().find(|b| b.board == self.board)
    }

    /// The whole answer, for a sample.
    pub fn answer(&self) -> Option<&LeaderboardsResponse> {
        self.answer.as_ref()
    }
}

/// "The week of Towerday the 4th": the week's first edition number, as the
/// page heads it. Weeks run from the Gazette's first day, so week 0 starts
/// with No. 1.
pub fn week_words(week: i64) -> String {
    let first = rules::week_first_day(week);
    format!("Week {}, from No. {} to No. {}", week + 1, rules::gazette_number(first), rules::gazette_number(first + 6))
}

/// A home zone's name: "Urban Zone", or the id when the rules don't know it.
pub fn zone_name(zone: &str) -> String {
    Zone::from_str(zone).map_or_else(|| zone.to_owned(), |z| z.display_name().to_owned())
}

/// What a board's value means, under its title.
pub fn board_blurb(board: LeaderboardKind) -> &'static str {
    match board {
        LeaderboardKind::Wealth => "Strands, deeds at their assessed value, and goods at their last trade",
        LeaderboardKind::Maker => "What they gathered and crafted, at each good's price",
        LeaderboardKind::Trader => "Strands bought and sold at the Exchange, the stalls and the board",
        LeaderboardKind::KeptWord => "The value of the claims they kept",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::LeaderboardEntry;

    fn answer(week: i64, latest: i64) -> LeaderboardsResponse {
        LeaderboardsResponse {
            week: Some(week),
            latest: Some(latest),
            zones: vec!["urban".into()],
            boards: LeaderboardKind::ALL
                .into_iter()
                .map(|board| Leaderboard {
                    board,
                    top: vec![LeaderboardEntry { rank: 1, name: Some("Ada".into()), value: 10, ..Default::default() }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn it_asks_once_and_pins_the_latest_week() {
        let mut s = Standings::new();
        let q = s.wants().expect("the first call");
        assert_eq!(q, LeaderboardQuery::default());
        s.sent(q.clone());
        assert_eq!(s.wants(), None, "one out at a time");
        s.answered(q, answer(5, 5));
        assert_eq!((s.week(), s.latest(), s.loading(), s.any()), (Some(5), Some(5), false, true));
        // Pinned to week 5 now, and it doesn't ask again until something changes
        assert_eq!(s.wants(), None);
        s.board = LeaderboardKind::Trader;
        assert_eq!(s.shown().map(|b| b.board), Some(LeaderboardKind::Trader));
    }

    #[test]
    fn a_change_of_view_asks_again_and_keeps_the_old_answer_meanwhile() {
        let mut s = Standings::new();
        let q = s.wants().unwrap();
        s.sent(q.clone());
        s.answered(q, answer(5, 5));
        s.turn(1);
        assert_eq!(s.wants(), None, "never past the latest");
        s.turn(-1);
        s.set_people(true);
        s.set_zone(Some("urban".into()));
        let q = s.wants().expect("the new view");
        assert_eq!((q.week, q.people, q.zone.as_deref()), (Some(4), true, Some("urban")));
        s.sent(q.clone());
        assert!(s.shown().is_some(), "the old answer stays up");
        assert!(s.loading());
        s.answered(q, answer(4, 5));
        assert!(!s.loading());
        assert_eq!(s.wants(), None);
        s.refresh();
        assert!(s.wants().is_some(), "opening the page asks again");
    }

    #[test]
    fn a_failed_call_is_asked_again_and_words_read() {
        let mut s = Standings::new();
        let q = s.wants().unwrap();
        s.sent(q.clone());
        s.failed(&q);
        s.refresh();
        assert_eq!(s.wants(), Some(q));
        assert_eq!(week_words(0), "Week 1, from No. 1 to No. 7");
        assert_eq!(zone_name("urban"), "Urban Zone");
        assert_eq!(zone_name("moon"), "moon");
    }
}
