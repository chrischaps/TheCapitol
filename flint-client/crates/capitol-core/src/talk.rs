//! Talk as the client keeps it (Commons S1.2, plan D3): the lines heard on
//! each channel, what's unread, and each speaker's latest line, which hangs
//! over their head for a few seconds.
//!
//! The server sends a socket only what it hears (S1.1), so nothing here
//! filters by distance. Nothing is kept past the session either: talk is
//! in memory on the server too.
//!
//! Time is the caller's clock, in seconds: the session's, or a capture's
//! game time, so a bench is deterministic.
//!
//! A reconnect is sent the Capitol backlog again (the last 40 lines), so
//! [`Log::rejoined`] remembers what was already heard and drops those lines
//! when they come back, for a few seconds after. It matches by speaker and
//! text, since `said` carries no number; that goes once it does
//! ([#430](https://github.com/chrischaps/TheCapitol/issues/430)).

use crate::protocol::rules::talk::CAPITOL_BACKLOG;
use crate::protocol::ws::Channel;
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

/// The lines kept on each channel.
pub const KEPT: usize = 200;
/// A line shows over its speaker for `SHOWN_BASE` seconds, plus
/// `SHOWN_PER_CHAR` for each character, `SHOWN_MAX` at most (D3).
pub const SHOWN_BASE: f64 = 4.0;
pub const SHOWN_PER_CHAR: f64 = 0.06;
pub const SHOWN_MAX: f64 = 12.0;
/// Then it fades over this long.
pub const FADE_OUT: f64 = 0.6;
/// And comes in over this long, so it opens rather than pops.
pub const FADE_IN: f64 = 0.15;
/// How long after a reconnect the backlog's repeats are looked for.
pub const REJOIN_WINDOW: f64 = 5.0;

/// One line heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub from: Uuid,
    pub name: String,
    /// The speaker is a citizen (C0.1).
    pub citizen: bool,
    pub channel: Channel,
    pub text: String,
    /// Where it was said, server units. A Capitol line's place means
    /// nothing to the listener.
    pub x: f64,
    pub y: f64,
    /// When it was heard, on the caller's clock.
    pub at: f64,
}

/// How long a line shows over its speaker before it fades.
pub fn shown_for(text: &str) -> f64 {
    (SHOWN_BASE + SHOWN_PER_CHAR * text.chars().count() as f64).min(SHOWN_MAX)
}

/// How opaque a bubble of `text` is `age` seconds after it was said, or
/// `None` once it's gone.
pub fn bubble_alpha(text: &str, age: f64) -> Option<f32> {
    let shown = shown_for(text);
    if !(0.0..shown + FADE_OUT).contains(&age) {
        return None;
    }
    let fade_in = (age / FADE_IN).min(1.0);
    let fade_out = 1.0 - ((age - shown) / FADE_OUT).clamp(0.0, 1.0);
    Some(smooth(fade_in.min(fade_out)) as f32)
}

#[derive(Debug, Default)]
struct Book {
    lines: VecDeque<Line>,
    unread: usize,
}

/// A speaker's latest local line.
#[derive(Debug, Clone, PartialEq)]
struct Spoken {
    text: String,
    at: f64,
}

#[derive(Debug, Default)]
pub struct Log {
    local: Book,
    capitol: Book,
    /// Each speaker's latest local line while it shows.
    latest: HashMap<Uuid, Spoken>,
    /// After a reconnect: the Capitol lines already heard that the backlog
    /// will send again, and until when to look for them.
    rejoin: Option<(f64, Vec<(Uuid, String)>)>,
}

impl Log {
    pub fn new() -> Self {
        Self::default()
    }

    /// A line was heard. Returns false for a repeat of the backlog after a
    /// reconnect, which isn't kept.
    pub fn heard(&mut self, line: Line) -> bool {
        if line.channel == Channel::Capitol && self.repeat(&line) {
            return false;
        }
        if line.channel == Channel::Local {
            let at = line.at;
            self.latest.retain(|_, s| at - s.at < SHOWN_MAX + FADE_OUT);
            self.latest.insert(
                line.from,
                Spoken {
                    text: line.text.clone(),
                    at,
                },
            );
        }
        let book = self.book_mut(line.channel);
        book.lines.push_back(line);
        if book.lines.len() > KEPT {
            book.lines.pop_front();
        }
        book.unread = (book.unread + 1).min(KEPT);
        true
    }

    /// The connection is back (or up for the first time): the backlog is on
    /// its way, and what of it was already heard isn't news.
    pub fn rejoined(&mut self, now: f64) {
        let held: Vec<_> = self
            .capitol
            .lines
            .iter()
            .rev()
            .take(CAPITOL_BACKLOG)
            .map(|l| (l.from, l.text.clone()))
            .collect();
        self.rejoin = (!held.is_empty()).then_some((now + REJOIN_WINDOW, held));
    }

    fn repeat(&mut self, line: &Line) -> bool {
        let Some((until, held)) = &mut self.rejoin else {
            return false;
        };
        if line.at > *until {
            self.rejoin = None;
            return false;
        }
        match held
            .iter()
            .position(|(from, text)| *from == line.from && *text == line.text)
        {
            Some(i) => {
                held.swap_remove(i);
                true
            }
            None => false,
        }
    }

    /// The lines on `channel`, oldest first.
    pub fn lines(&self, channel: Channel) -> impl DoubleEndedIterator<Item = &Line> {
        self.book(channel).lines.iter()
    }

    /// How many lines on `channel` came since it was last read.
    pub fn unread(&self, channel: Channel) -> usize {
        self.book(channel).unread
    }

    /// `channel` has been read.
    pub fn read(&mut self, channel: Channel) {
        self.book_mut(channel).unread = 0;
    }

    /// The last line heard on `channel` was your own (S1.3): it isn't news.
    pub fn own(&mut self, channel: Channel) {
        let book = self.book_mut(channel);
        book.unread = book.unread.saturating_sub(1);
    }

    /// `from`'s latest local line and when it was heard, while it's kept
    /// for their bubble.
    pub fn latest(&self, from: Uuid) -> Option<(&str, f64)> {
        self.latest.get(&from).map(|s| (s.text.as_str(), s.at))
    }

    /// The bubble over `from` at `now`: the text, its alpha and its age in
    /// seconds, or `None` when they've said nothing lately. Only local lines
    /// are spoken aloud; a Capitol line has no place to hang.
    pub fn bubble(&self, from: Uuid, now: f64) -> Option<(&str, f32, f64)> {
        let (text, at) = self.latest(from)?;
        let age = now - at;
        Some((text, bubble_alpha(text, age)?, age))
    }

    fn book(&self, channel: Channel) -> &Book {
        match channel {
            Channel::Local => &self.local,
            Channel::Capitol => &self.capitol,
        }
    }

    fn book_mut(&mut self, channel: Channel) -> &mut Book {
        match channel {
            Channel::Local => &mut self.local,
            Channel::Capitol => &mut self.capitol,
        }
    }
}

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(from: u128, channel: Channel, text: &str, at: f64) -> Line {
        Line {
            from: Uuid::from_u128(from),
            name: format!("P{from}"),
            citizen: false,
            channel,
            text: text.into(),
            x: 0.0,
            y: 0.0,
            at,
        }
    }

    #[test]
    fn each_channel_keeps_its_last_200_lines_in_order() {
        let mut log = Log::new();
        for i in 0..250 {
            log.heard(line(1, Channel::Local, &format!("l{i}"), i as f64));
        }
        log.heard(line(2, Channel::Capitol, "c", 0.0));
        let texts: Vec<_> = log.lines(Channel::Local).map(|l| l.text.as_str()).collect();
        assert_eq!(texts.len(), KEPT);
        assert_eq!(texts[0], "l50");
        assert_eq!(texts[KEPT - 1], "l249");
        assert_eq!(log.lines(Channel::Capitol).count(), 1);
    }

    #[test]
    fn your_own_line_isnt_unread() {
        let mut log = Log::new();
        log.heard(line(1, Channel::Capitol, "theirs", 0.0));
        log.heard(line(2, Channel::Capitol, "mine", 1.0));
        log.own(Channel::Capitol);
        assert_eq!(log.unread(Channel::Capitol), 1);
        log.read(Channel::Capitol);
        log.own(Channel::Capitol);
        assert_eq!(log.unread(Channel::Capitol), 0);
    }

    #[test]
    fn unread_counts_each_channel_until_it_is_read() {
        let mut log = Log::new();
        log.heard(line(1, Channel::Local, "a", 0.0));
        log.heard(line(1, Channel::Local, "b", 1.0));
        log.heard(line(2, Channel::Capitol, "c", 1.0));
        assert_eq!(log.unread(Channel::Local), 2);
        assert_eq!(log.unread(Channel::Capitol), 1);
        log.read(Channel::Local);
        assert_eq!(log.unread(Channel::Local), 0);
        assert_eq!(log.unread(Channel::Capitol), 1);
        // Never more unread than are kept.
        for i in 0..300 {
            log.heard(line(1, Channel::Local, "x", i as f64));
        }
        assert_eq!(log.unread(Channel::Local), KEPT);
    }

    #[test]
    fn a_speakers_latest_local_line_replaces_their_last() {
        let mut log = Log::new();
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        log.heard(line(1, Channel::Local, "first", 0.0));
        log.heard(line(2, Channel::Local, "other", 0.5));
        log.heard(line(1, Channel::Local, "second", 1.0));
        assert_eq!(log.latest(a), Some(("second", 1.0)));
        assert_eq!(log.latest(b), Some(("other", 0.5)));
        // A Capitol line hangs over nobody.
        log.heard(line(3, Channel::Capitol, "to all", 1.0));
        assert_eq!(log.latest(Uuid::from_u128(3)), None);
        assert_eq!(log.bubble(Uuid::from_u128(3), 1.0), None);
    }

    #[test]
    fn a_bubble_shows_for_four_seconds_and_60_ms_a_character_then_fades() {
        assert_eq!(shown_for(""), 4.0);
        assert!((shown_for(&"x".repeat(50)) - 7.0).abs() < 1e-9);
        // 12 s at most.
        assert_eq!(shown_for(&"x".repeat(240)), SHOWN_MAX);

        let text = "x".repeat(50);
        let mut log = Log::new();
        let id = Uuid::from_u128(1);
        log.heard(line(1, Channel::Local, &text, 10.0));
        let alpha = |now| log.bubble(id, now).map(|(_, a, _)| a);
        // It opens over FADE_IN, holds, then fades over FADE_OUT.
        assert_eq!(alpha(10.0), Some(0.0));
        assert!(alpha(10.05).unwrap() > 0.0 && alpha(10.05).unwrap() < 1.0);
        assert_eq!(alpha(10.0 + FADE_IN), Some(1.0));
        assert_eq!(alpha(16.9), Some(1.0));
        let half = alpha(17.0 + FADE_OUT / 2.0).unwrap();
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        assert_eq!(alpha(17.0 + FADE_OUT), None);
        assert_eq!(alpha(9.0), None, "not before it's said");
    }

    #[test]
    fn quiet_speakers_are_forgotten_as_others_talk() {
        let mut log = Log::new();
        log.heard(line(1, Channel::Local, "hello", 0.0));
        log.heard(line(2, Channel::Local, "later", 100.0));
        assert_eq!(log.latest(Uuid::from_u128(1)), None);
        assert_eq!(log.latest.len(), 1);
        // Their line is still in the book.
        assert_eq!(log.lines(Channel::Local).count(), 2);
    }

    #[test]
    fn a_reconnect_drops_the_backlogs_repeats_but_keeps_what_was_missed() {
        let mut log = Log::new();
        log.heard(line(1, Channel::Capitol, "one", 0.0));
        log.heard(line(2, Channel::Capitol, "two", 1.0));
        log.read(Channel::Capitol);
        log.rejoined(10.0);
        // The backlog: the two already heard, then one said while away.
        assert!(!log.heard(line(1, Channel::Capitol, "one", 10.1)));
        assert!(!log.heard(line(2, Channel::Capitol, "two", 10.1)));
        assert!(log.heard(line(3, Channel::Capitol, "three", 10.1)));
        // Saying "one" again is news, once the repeat is used up.
        assert!(log.heard(line(1, Channel::Capitol, "one", 11.0)));
        let texts: Vec<_> = log
            .lines(Channel::Capitol)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(texts, ["one", "two", "three", "one"]);
        assert_eq!(log.unread(Channel::Capitol), 2);
        // Past the window, a repeat is a new line.
        log.rejoined(20.0);
        assert!(log.heard(line(2, Channel::Capitol, "two", 26.0)));
        // A local line is never a backlog's.
        log.rejoined(30.0);
        assert!(log.heard(line(3, Channel::Local, "three", 30.1)));
    }
}
