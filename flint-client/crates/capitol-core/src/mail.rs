//! Letters (S1.5), as the client keeps them: your two boxes, the letter
//! open, the one you're writing, and the calls for them.
//!
//! [`Mail`] is pure. The session makes the call [`Mail::wants`] names, one at
//! a time, and hands back the answer. The in box is fetched when you sign
//! in, so the header knows what's unread, and again when a letter comes; the
//! out box the first time you look at it. Opening a letter marks it read at
//! once and tells the server after; dropping one takes it off at once.
//!
//! The words are here too, so they're tested without egui: when a letter
//! was written, the toast for one that came, and what stops a letter going.

use std::collections::VecDeque;

use capitol_protocol::rest::{Correspondent, Letter, LetterBox, LettersQuery, Mailbox, SendLetter};
use capitol_protocol::rules::{self, talk};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A call [`Mail`] wants made.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// `GET /letters`: the newest page of a box, or the one after `before`
    Page(LettersQuery),
    /// `POST /letters`
    Send(SendLetter),
    /// `POST /letters/{id}/read`
    Read(Uuid),
    /// `DELETE /letters/{id}`
    Drop(Uuid),
}

/// What the mailbox shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Showing {
    /// A box's letters
    #[default]
    List,
    /// One letter
    Letter(Uuid),
    /// The letter you're writing
    Writing,
}

/// A letter being written.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub to: Correspondent,
    pub body: String,
}

/// One box as the client has it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Held {
    letters: Vec<Letter>,
    /// Fetched at least once since signing in
    loaded: bool,
    /// Asked for again: something changed
    stale: bool,
    /// The server has older letters than these
    more: bool,
}

/// Your letters, and the call for them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mail {
    /// The box the list shows
    pub tab: LetterBox,
    showing: Showing,
    in_box: Held,
    out_box: Held,
    unread: u32,
    draft: Option<Draft>,
    /// Sends, reads and drops not yet asked, oldest first
    pending: VecDeque<Ask>,
    /// The call that's out
    asked: Option<Ask>,
    /// Why the last letter didn't go
    refused: Option<String>,
    /// Signed in: the in box is worth fetching
    online: bool,
}

impl Mail {
    /// Signed in, or back after a drop: whatever was out won't be
    /// answered, and both boxes are asked again.
    pub fn connected(&mut self) {
        self.online = true;
        self.asked = None;
        self.in_box.stale = true;
        self.out_box.stale = self.out_box.loaded;
    }

    /// A letter came (`letter_arrived`): one more unread, and the in box is
    /// asked again.
    pub fn arrived(&mut self) {
        self.unread += 1;
        self.in_box.stale = true;
    }

    /// Letters unread in your in box.
    pub fn unread(&self) -> u32 {
        self.unread
    }

    pub fn showing(&self) -> Showing {
        self.showing
    }

    /// The call to make next, if any: what you did first, then the boxes.
    /// Mark it sent with [`Mail::sent`].
    pub fn wants(&self, open: bool) -> Option<Ask> {
        if !self.online || self.asked.is_some() {
            return None;
        }
        if let Some(ask) = self.pending.front() {
            return Some(ask.clone());
        }
        if self.in_box.stale || !self.in_box.loaded {
            return Some(Ask::Page(LettersQuery { which: LetterBox::In, before: None }));
        }
        let looking = open && self.tab == LetterBox::Out;
        if (self.out_box.stale && self.out_box.loaded) || (looking && !self.out_box.loaded) {
            return Some(Ask::Page(LettersQuery { which: LetterBox::Out, before: None }));
        }
        None
    }

    pub fn sent(&mut self, ask: Ask) {
        if self.pending.front() == Some(&ask) {
            self.pending.pop_front();
        }
        if let Ask::Page(q) = &ask {
            self.held_mut(q.which).stale = false;
        }
        self.asked = Some(ask);
    }

    /// A page of `which` came back.
    pub fn paged(&mut self, mailbox: Mailbox) {
        let Some(Ask::Page(q)) = self.asked.take() else { return };
        let held = self.held_mut(q.which);
        if q.before.is_none() {
            held.letters = mailbox.letters;
        } else {
            let known: Vec<Uuid> = held.letters.iter().map(|l| l.id).collect();
            held.letters.extend(mailbox.letters.into_iter().filter(|l| !known.contains(&l.id)));
        }
        held.loaded = true;
        held.more = mailbox.more;
        self.unread = mailbox.unread;
        // A letter that went while it was open
        if let Showing::Letter(id) = self.showing {
            if self.letter(id).is_none() {
                self.showing = Showing::List;
            }
        }
    }

    /// Your letter went: it's on top of your out box, and that's shown.
    pub fn delivered(&mut self, letter: Letter) {
        let Some(Ask::Send(_)) = self.asked.take() else { return };
        self.draft = None;
        self.refused = None;
        if self.out_box.loaded {
            self.out_box.letters.insert(0, letter);
        }
        self.tab = LetterBox::Out;
        if self.showing == Showing::Writing {
            self.showing = Showing::List;
        }
    }

    /// A read or a drop came back, with the unread now.
    pub fn counted(&mut self, unread: u32) {
        if matches!(self.asked.take(), Some(Ask::Read(_) | Ask::Drop(_))) {
            self.unread = unread;
        }
    }

    /// The call that was out failed, saying `why`. A letter that didn't go
    /// stays in your hand; anything else is asked again.
    pub fn failed(&mut self, why: String) {
        match self.asked.take() {
            Some(Ask::Send(_)) => self.refused = Some(why),
            Some(Ask::Page(q)) => self.held_mut(q.which).stale = !self.held(q.which).loaded,
            Some(Ask::Read(_) | Ask::Drop(_)) => {
                self.in_box.stale = true;
                self.out_box.stale = self.out_box.loaded;
            }
            None => {}
        }
    }

    /// `which` box's letters as the list shows them: newest first, with the
    /// unread before the rest in your in box.
    pub fn letters(&self, which: LetterBox) -> Vec<&Letter> {
        let mut letters: Vec<&Letter> = self.held(which).letters.iter().collect();
        if which == LetterBox::In {
            // Stable, so the newest stay first within each
            letters.sort_by_key(|l| l.read_at.is_some());
        }
        letters
    }

    /// The box has been fetched.
    pub fn loaded(&self, which: LetterBox) -> bool {
        self.held(which).loaded
    }

    /// The server has older letters in `which` than these.
    pub fn more(&self, which: LetterBox) -> bool {
        self.held(which).more
    }

    /// Ask for the next page of `which`.
    pub fn older(&mut self, which: LetterBox) {
        let before = self.held(which).letters.last().map(|l| l.id);
        let ask = Ask::Page(LettersQuery { which, before });
        if before.is_some() && self.asked.as_ref() != Some(&ask) && !self.pending.contains(&ask) {
            self.pending.push_back(ask);
        }
    }

    /// The letter `id`, from either box.
    pub fn letter(&self, id: Uuid) -> Option<&Letter> {
        self.in_box.letters.iter().chain(&self.out_box.letters).find(|l| l.id == id)
    }

    /// Open the letter `id`, read at `now`: marked read here at once, and
    /// on the server next.
    pub fn open_letter(&mut self, id: Uuid, now: DateTime<Utc>) {
        self.showing = Showing::Letter(id);
        if let Some(letter) = self.in_box.letters.iter_mut().find(|l| l.id == id && l.read_at.is_none()) {
            letter.read_at = Some(now);
            self.unread = self.unread.saturating_sub(1);
            self.pending.push_back(Ask::Read(id));
        }
    }

    /// Back to the list. A letter half written is kept.
    pub fn back(&mut self) {
        self.showing = Showing::List;
    }

    /// Write to `to`: the letter in your hand if it's to them, or a fresh one.
    pub fn write_to(&mut self, to: Correspondent) {
        if self.draft.as_ref().is_none_or(|d| d.to.id != to.id) {
            self.draft = Some(Draft { to, body: String::new() });
        }
        self.refused = None;
        self.showing = Showing::Writing;
    }

    /// Answer the letter `id`: to whoever wrote it, or, in your out box,
    /// to whoever you wrote it to.
    pub fn reply(&mut self, id: Uuid) {
        let Some(letter) = self.letter(id) else { return };
        let mine = self.out_box.letters.iter().any(|l| l.id == id);
        let to = if mine { letter.to.clone() } else { letter.from.clone() };
        self.write_to(to);
    }

    /// The letter you're writing.
    pub fn draft(&self) -> Option<&Draft> {
        self.draft.as_ref()
    }

    pub fn draft_mut(&mut self) -> Option<&mut Draft> {
        self.refused = None;
        self.draft.as_mut()
    }

    /// Put the letter you're writing away unsent.
    pub fn discard(&mut self) {
        self.draft = None;
        self.refused = None;
        self.showing = Showing::List;
    }

    /// Your letter is on its way to the server.
    pub fn sending(&self) -> bool {
        matches!(self.asked, Some(Ask::Send(_))) || self.pending.iter().any(|a| matches!(a, Ask::Send(_)))
    }

    /// Why the last letter didn't go.
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// Send the letter you're writing, if [`stopped`] doesn't stop it.
    pub fn send(&mut self, strands: Option<i64>) {
        let Some(draft) = &self.draft else { return };
        if self.sending() || stopped(draft, strands).is_some() {
            return;
        }
        let body = talk::clean_letter(&draft.body).expect("stopped checks it");
        self.pending.push_back(Ask::Send(SendLetter { to: draft.to.id, body }));
    }

    /// Drop your copy of the letter `id`: gone from here at once.
    pub fn drop_letter(&mut self, id: Uuid) {
        for which in [LetterBox::In, LetterBox::Out] {
            let held = self.held_mut(which);
            if let Some(i) = held.letters.iter().position(|l| l.id == id) {
                let letter = held.letters.remove(i);
                if which == LetterBox::In && letter.read_at.is_none() {
                    self.unread = self.unread.saturating_sub(1);
                }
            }
        }
        if self.showing == Showing::Letter(id) {
            self.showing = Showing::List;
        }
        self.pending.retain(|a| *a != Ask::Read(id));
        self.pending.push_back(Ask::Drop(id));
    }

    /// Signed out: nothing is kept.
    pub fn signed_out(&mut self) {
        *self = Mail::default();
    }

    fn held(&self, which: LetterBox) -> &Held {
        match which {
            LetterBox::In => &self.in_box,
            LetterBox::Out => &self.out_box,
        }
    }

    fn held_mut(&mut self, which: LetterBox) -> &mut Held {
        match which {
            LetterBox::In => &mut self.in_box,
            LetterBox::Out => &mut self.out_box,
        }
    }
}

/// What stops `draft` going, with `strands` in hand (`None`: not known
/// yet), in words; `None` if it can go.
pub fn stopped(draft: &Draft, strands: Option<i64>) -> Option<String> {
    if let Err(e) = talk::clean_letter(&draft.body) {
        return Some(match e {
            talk::TalkError::Empty => "Write something first".into(),
            e => e.message(),
        });
    }
    match strands {
        Some(s) if s < talk::POSTAGE => Some(format!("Postage is {} Strand, and you haven't one", talk::POSTAGE)),
        _ => None,
    }
}

/// How many characters of a letter are used, as the server will count them.
pub fn written(body: &str) -> usize {
    talk::clean_letter(body).map_or(0, |b| b.chars().count())
}

/// When a letter was written, by game days (`rules::day_number`), as of `now`.
pub fn when_written(sent_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let days = rules::day_number(now.timestamp_millis()) - rules::day_number(sent_at.timestamp_millis());
    match days {
        ..=0 => "Today".into(),
        1 => "Yesterday".into(),
        n @ 2..=13 => format!("{n} days ago"),
        n => format!("{} weeks ago", n / 7),
    }
}

/// The toast for a letter that came: who it's from, and how it begins,
/// cut short at a word so the toast stays narrow.
pub fn arrived_words(name: &str, citizen: bool, first_line: &str) -> (String, String) {
    const GIST: usize = 36;
    let who = if citizen { format!("{name}, a citizen") } else { name.to_owned() };
    let gist = if first_line.chars().count() <= GIST {
        first_line.to_owned()
    } else {
        let cut: String = first_line.chars().take(GIST).collect();
        let cut = cut.rfind(' ').map_or(cut.as_str(), |i| &cut[..i]);
        format!("{}…", cut.trim_end_matches([',', ';', ':', '.', '…']))
    };
    ("A letter".into(), format!("{who}: “{gist}”"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(n: u128, name: &str) -> Correspondent {
        Correspondent { id: Uuid::from_u128(n), name: name.into(), citizen: false }
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap() + chrono::Duration::minutes(minutes)
    }

    fn letter(n: u128, from: u128, minutes: i64, read: bool) -> Letter {
        Letter {
            id: Uuid::from_u128(n),
            from: who(from, "Brin"),
            to: who(1, "Ada"),
            body: format!("Letter {n}"),
            sent_at: at(minutes),
            read_at: read.then(|| at(minutes + 1)),
        }
    }

    fn page(letters: Vec<Letter>, unread: u32, more: bool) -> Mailbox {
        Mailbox { letters, unread, more }
    }

    /// Signed in with `letters` in the in box
    fn signed_in(letters: Vec<Letter>) -> Mail {
        let mut mail = Mail::default();
        assert_eq!(mail.wants(false), None, "not before signing in");
        mail.connected();
        let ask = mail.wants(false).expect("the in box");
        assert_eq!(ask, Ask::Page(LettersQuery { which: LetterBox::In, before: None }));
        mail.sent(ask);
        let unread = letters.iter().filter(|l| l.read_at.is_none()).count() as u32;
        mail.paged(page(letters, unread, false));
        mail
    }

    #[test]
    fn signing_in_fetches_the_in_box_and_counts_the_unread() {
        let mail = signed_in(vec![letter(3, 2, 30, true), letter(2, 2, 20, false), letter(1, 2, 10, false)]);
        assert_eq!(mail.unread(), 2);
        assert_eq!(mail.wants(false), None, "the out box waits until it's looked at");
        let ids: Vec<_> = mail.letters(LetterBox::In).iter().map(|l| l.id.as_u128()).collect();
        assert_eq!(ids, [2, 1, 3], "unread first, newest first within each");
    }

    #[test]
    fn a_letter_that_comes_counts_and_the_in_box_is_asked_again() {
        let mut mail = signed_in(vec![]);
        mail.arrived();
        assert_eq!(mail.unread(), 1);
        let ask = mail.wants(false).unwrap();
        assert!(matches!(&ask, Ask::Page(q) if q.which == LetterBox::In));
        mail.sent(ask);
        assert_eq!(mail.wants(false), None, "one call at a time");
        mail.paged(page(vec![letter(9, 2, 0, false)], 1, false));
        assert_eq!(mail.letters(LetterBox::In).len(), 1);
    }

    #[test]
    fn opening_a_letter_reads_it_at_once_and_tells_the_server_once() {
        let mut mail = signed_in(vec![letter(1, 2, 10, false)]);
        let id = Uuid::from_u128(1);
        mail.open_letter(id, at(50));
        assert_eq!((mail.unread(), mail.showing()), (0, Showing::Letter(id)));
        assert_eq!(mail.letter(id).unwrap().read_at, Some(at(50)));
        let ask = mail.wants(true).unwrap();
        assert_eq!(ask, Ask::Read(id));
        mail.sent(ask);
        mail.counted(0);
        mail.back();
        mail.open_letter(id, at(60));
        assert_eq!(mail.wants(true), None, "read once");
        assert_eq!(mail.letter(id).unwrap().read_at, Some(at(50)), "when it was first read");
    }

    #[test]
    fn the_out_box_is_fetched_when_looked_at() {
        let mut mail = signed_in(vec![]);
        mail.tab = LetterBox::Out;
        assert_eq!(mail.wants(false), None, "closed");
        let ask = mail.wants(true).unwrap();
        assert_eq!(ask, Ask::Page(LettersQuery { which: LetterBox::Out, before: None }));
        mail.sent(ask);
        mail.paged(page(vec![letter(5, 1, 0, false)], 0, true));
        assert!(mail.loaded(LetterBox::Out) && mail.more(LetterBox::Out));
        mail.older(LetterBox::Out);
        let ask = mail.wants(true).unwrap();
        assert_eq!(ask, Ask::Page(LettersQuery { which: LetterBox::Out, before: Some(Uuid::from_u128(5)) }));
        mail.sent(ask);
        mail.paged(page(vec![letter(5, 1, 0, false), letter(4, 1, -5, false)], 0, false));
        assert_eq!(mail.letters(LetterBox::Out).len(), 2, "appended, without the one already held");
    }

    #[test]
    fn a_reply_goes_to_the_writer_and_lands_in_the_out_box() {
        let mut mail = signed_in(vec![letter(1, 2, 10, true)]);
        mail.tab = LetterBox::Out;
        let ask = mail.wants(true).unwrap();
        mail.sent(ask);
        mail.paged(page(vec![], 0, false));
        mail.tab = LetterBox::In;

        mail.reply(Uuid::from_u128(1));
        assert_eq!(mail.showing(), Showing::Writing);
        assert_eq!(mail.draft().unwrap().to.name, "Brin");
        mail.send(Some(5));
        assert_eq!(mail.wants(true), None, "nothing to send yet");
        mail.draft_mut().unwrap().body = "  Thank you.  ".into();
        mail.send(Some(5));
        let ask = mail.wants(true).unwrap();
        assert_eq!(ask, Ask::Send(SendLetter { to: Uuid::from_u128(2), body: "Thank you.".into() }));
        mail.sent(ask);
        assert!(mail.sending());
        mail.send(Some(5));
        assert_eq!(mail.wants(true), None, "once");
        let sent = Letter { id: Uuid::from_u128(7), from: who(1, "Ada"), to: who(2, "Brin"), body: "Thank you.".into(), sent_at: at(60), read_at: None };
        mail.delivered(sent);
        assert_eq!((mail.tab, mail.showing(), mail.draft()), (LetterBox::Out, Showing::List, None));
        assert_eq!(mail.letters(LetterBox::Out)[0].id, Uuid::from_u128(7));
    }

    #[test]
    fn a_refused_letter_stays_in_your_hand_with_the_reason() {
        let mut mail = signed_in(vec![]);
        mail.write_to(who(2, "Brin"));
        mail.draft_mut().unwrap().body = "Hello".into();
        mail.send(Some(5));
        let ask = mail.wants(true).unwrap();
        mail.sent(ask);
        mail.failed("You're talking too fast; wait 2.0 s".into());
        assert_eq!(mail.refused(), Some("You're talking too fast; wait 2.0 s"));
        assert_eq!(mail.draft().unwrap().body, "Hello");
        // Typing again clears it
        mail.draft_mut().unwrap().body.push('!');
        assert_eq!(mail.refused(), None);
        // Writing to the same one again keeps it; to someone else starts afresh
        mail.back();
        mail.write_to(who(2, "Brin"));
        assert_eq!(mail.draft().unwrap().body, "Hello!");
        mail.write_to(who(3, "Cy"));
        assert_eq!(mail.draft().unwrap().body, "");
    }

    #[test]
    fn dropping_a_letter_takes_it_off_at_once() {
        let mut mail = signed_in(vec![letter(2, 2, 20, false), letter(1, 2, 10, true)]);
        let id = Uuid::from_u128(2);
        mail.open_letter(id, at(30));
        mail.drop_letter(id);
        assert_eq!(mail.showing(), Showing::List);
        assert_eq!(mail.letters(LetterBox::In).len(), 1);
        assert_eq!(mail.wants(true), Some(Ask::Drop(id)), "the read isn't sent for a letter dropped");
        // Dropping an unread one counts it off
        let mut mail = signed_in(vec![letter(2, 2, 20, false)]);
        mail.drop_letter(id);
        assert_eq!(mail.unread(), 0);
    }

    #[test]
    fn what_stops_a_letter_is_said() {
        let draft = |body: &str| Draft { to: who(2, "Brin"), body: body.into() };
        assert_eq!(stopped(&draft(" \n "), Some(5)).as_deref(), Some("Write something first"));
        assert!(stopped(&draft(&"a".repeat(talk::MAX_LETTER + 1)), Some(5)).unwrap().contains("1000"));
        assert_eq!(stopped(&draft("Hi"), Some(0)).as_deref(), Some("Postage is 1 Strand, and you haven't one"));
        assert_eq!(stopped(&draft("Hi"), None), None);
        assert_eq!(written("  Dear Brin,\n\n\n\nHi "), "Dear Brin,\n\nHi".chars().count());
    }

    #[test]
    fn letters_are_dated_in_game_days() {
        let day = rules::DAY_LENGTH_SECS;
        let now = DateTime::from_timestamp(rules::day_start_ms(rules::GAZETTE_FIRST_DAY + 30) / 1000 + 60, 0).unwrap();
        let ago = |days: i64| now - chrono::Duration::seconds(days * day);
        assert_eq!(when_written(now, now), "Today");
        assert_eq!(when_written(ago(1), now), "Yesterday");
        assert_eq!(when_written(ago(5), now), "5 days ago");
        assert_eq!(when_written(ago(21), now), "3 weeks ago");
        assert_eq!(when_written(now + chrono::Duration::seconds(30), now), "Today", "a clock behind the server's");
    }

    #[test]
    fn the_toast_names_the_writer_and_their_mark() {
        assert_eq!(
            arrived_words("Brin", true, "Thank you…"),
            ("A letter".to_string(), "Brin, a citizen: “Thank you…”".to_string())
        );
        let (_, body) = arrived_words("Ada Reeder", false, "The kiln on Sorrel Lane is lit tonight, if you want your…");
        assert_eq!(body, "Ada Reeder: “The kiln on Sorrel Lane is lit…”");
    }
}
