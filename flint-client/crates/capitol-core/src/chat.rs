//! The chat's input side (Commons S1.3): the line you're writing, the
//! lines on their way to the server, and the last one it refused. What was
//! said is [`crate::talk::Log`]'s (S1.2), which the chat panel reads.
//!
//! [`Chat`] is pure. The session sends what [`Chat::send`] gives it, tells
//! it when a line of yours comes back ([`Chat::heard_back`]) and when a
//! `say` is refused ([`Chat::refused`]).
//!
//! A sent line leaves the input at once. If the server refuses it (too
//! fast, or a watcher speaking locally), it comes back to an empty input
//! with the reason under it, so nothing typed is lost.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use capitol_protocol::rules;
use capitol_protocol::ws::Channel;

/// How long a refusal stays under the input.
pub const REFUSAL_SHOWN: Duration = Duration::from_secs(10);

/// A line the server refused, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub message: String,
    pub text: String,
    pub at: Instant,
}

/// The input line and what's in flight.
#[derive(Debug, Default)]
pub struct Chat {
    /// The input line
    pub draft: String,
    /// Lines sent and not yet heard back or refused, oldest first
    pending: VecDeque<(Channel, String)>,
    refusal: Option<Refusal>,
}

impl Chat {
    pub fn new() -> Self {
        Self::default()
    }

    /// The input line as it goes, or `None` if there's nothing to send.
    /// Moves it to the lines in flight and clears the input.
    pub fn send(&mut self, channel: Channel) -> Option<String> {
        if rules::talk::clean(&self.draft).is_err() {
            // Nothing worth sending; keep what there is
            return None;
        }
        let text = std::mem::take(&mut self.draft);
        self.refusal = None;
        self.pending.push_back((channel, text.clone()));
        Some(text)
    }

    /// A line of yours came back from the server, cleaned: it was said.
    pub fn heard_back(&mut self, channel: Channel, text: &str) {
        let sent = self
            .pending
            .iter()
            .position(|(c, t)| *c == channel && rules::talk::clean(t).is_ok_and(|t| t == text));
        if let Some(i) = sent {
            self.pending.remove(i);
        }
    }

    /// The server refused a `say`. The oldest line in flight is the one: it
    /// comes back to the input if that's empty, and the reason shows.
    pub fn refused(&mut self, message: &str, now: Instant) {
        let text = self.pending.pop_front().map(|(_, t)| t).unwrap_or_default();
        if self.draft.trim().is_empty() && !text.is_empty() {
            self.draft = text.clone();
        }
        self.refusal = Some(Refusal { message: message.to_owned(), text, at: now });
    }

    /// What the server last refused, while it's still worth showing.
    pub fn refusal(&self, now: Instant) -> Option<&Refusal> {
        self.refusal.as_ref().filter(|r| now.duration_since(r.at) < REFUSAL_SHOWN)
    }

    /// Lines still waiting to be heard back.
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }

    /// A new socket: whatever was in flight went down with the old one.
    pub fn connected(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_send_clears_the_line_and_hearing_it_back_settles_it() {
        let mut chat = Chat::new();
        chat.draft = "  good   morning ".into();
        assert_eq!(chat.send(Channel::Local).as_deref(), Some("  good   morning "));
        assert!(chat.draft.is_empty());
        assert_eq!(chat.in_flight(), 1);
        // The server folds the whitespace; it's still the one sent
        chat.heard_back(Channel::Capitol, "good morning");
        assert_eq!(chat.in_flight(), 1, "not on that channel");
        chat.heard_back(Channel::Local, "good morning");
        assert_eq!(chat.in_flight(), 0);
    }

    #[test]
    fn an_empty_line_isnt_sent() {
        let mut chat = Chat::new();
        chat.draft = "   ".into();
        assert_eq!(chat.send(Channel::Capitol), None);
        assert_eq!(chat.in_flight(), 0);
        assert_eq!(chat.draft, "   ");
    }

    #[test]
    fn a_refusal_gives_the_line_back_with_the_reason() {
        let mut chat = Chat::new();
        let now = Instant::now();
        chat.draft = "one too many".into();
        chat.send(Channel::Capitol);
        chat.refused("You're talking too fast. Try again in 3 s", now);
        assert_eq!(chat.draft, "one too many");
        let r = chat.refusal(now).unwrap();
        assert!(r.message.starts_with("You're talking too fast"));
        assert_eq!(chat.in_flight(), 0);
        assert!(chat.refusal(now + REFUSAL_SHOWN).is_none());
        // The next send clears it
        chat.send(Channel::Capitol);
        assert!(chat.refusal(now).is_none());
    }

    #[test]
    fn a_refusal_doesnt_overwrite_whats_being_typed() {
        let mut chat = Chat::new();
        chat.draft = "first".into();
        chat.send(Channel::Local);
        chat.draft = "second, half-wri".into();
        chat.refused("Play here to talk to people nearby", Instant::now());
        assert_eq!(chat.draft, "second, half-wri");
        assert_eq!(chat.refusal(Instant::now()).unwrap().text, "first");
    }

    #[test]
    fn a_reconnect_forgets_what_was_in_flight() {
        let mut chat = Chat::new();
        chat.draft = "lost with the socket".into();
        chat.send(Channel::Capitol);
        chat.connected();
        assert_eq!(chat.in_flight(), 0);
    }
}
