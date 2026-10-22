//! The people you've muted (S1.4), as the client keeps them.
//!
//! The server does the muting: a muted line never arrives, on any device.
//! This is only the list, for the card's Mute/Unmute and the settings
//! panel. [`Mutes`] is pure. It asks for `GET /player/mutes` once signed in
//! and again after a reconnect, since a mute made on another device may
//! have happened meanwhile; a mute or an unmute answers with the whole list,
//! which replaces it.

use capitol_protocol::rest::MutedPlayer;
use uuid::Uuid;

/// Who you've muted, and whether to ask again.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mutes {
    list: Option<Vec<MutedPlayer>>,
    /// The list may be out of date: ask for it next
    stale: bool,
    /// A `GET /player/mutes` is out
    asked: bool,
}

impl Mutes {
    /// Whether to ask for the list now. Mark it sent with [`Mutes::sent`].
    pub fn wants(&self) -> bool {
        !self.asked && (self.list.is_none() || self.stale)
    }

    pub fn sent(&mut self) {
        self.asked = true;
    }

    /// The list as the server has it, from a read, a mute or an unmute.
    pub fn answered(&mut self, list: Vec<MutedPlayer>) {
        self.list = Some(list);
        self.stale = false;
        self.asked = false;
    }

    /// The read failed: the list stands as it was, and isn't asked for
    /// again until the next reconnect or [`Mutes::refresh`].
    pub fn failed(&mut self) {
        self.asked = false;
        self.stale = false;
        self.list.get_or_insert_with(Vec::new);
    }

    /// Ask again, as the settings panel opens.
    pub fn refresh(&mut self) {
        self.stale = true;
    }

    /// Signed in again: whatever was out won't be answered, and a mute may
    /// have been made elsewhere meanwhile.
    pub fn connected(&mut self) {
        self.asked = false;
        self.stale = true;
    }

    /// Who you've muted, newest first, once the server has said.
    pub fn list(&self) -> Option<&[MutedPlayer]> {
        self.list.as_deref()
    }

    pub fn contains(&self, id: Uuid) -> bool {
        self.list.as_ref().is_some_and(|l| l.iter().any(|m| m.id == id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn muted(n: u128, name: &str) -> MutedPlayer {
        MutedPlayer { id: Uuid::from_u128(n), name: name.into(), citizen: false }
    }

    #[test]
    fn the_list_is_asked_for_once_then_again_after_a_reconnect() {
        let mut mutes = Mutes::default();
        assert!(mutes.wants());
        assert_eq!(mutes.list(), None);
        mutes.sent();
        assert!(!mutes.wants(), "one call at a time");
        mutes.answered(vec![muted(1, "Ada")]);
        assert!(!mutes.wants());
        assert!(mutes.contains(Uuid::from_u128(1)) && !mutes.contains(Uuid::from_u128(2)));
        mutes.connected();
        assert!(mutes.wants(), "muted elsewhere meanwhile, perhaps");
        assert!(mutes.contains(Uuid::from_u128(1)), "what it knew stands until then");
    }

    #[test]
    fn a_mutes_answer_replaces_the_list_and_a_failure_keeps_it() {
        let mut mutes = Mutes::default();
        mutes.answered(vec![muted(1, "Ada")]);
        mutes.answered(vec![muted(2, "Bram"), muted(1, "Ada")]);
        assert_eq!(mutes.list().unwrap().len(), 2);
        mutes.refresh();
        mutes.sent();
        mutes.failed();
        assert!(!mutes.wants(), "a failure isn't asked again at once");
        assert_eq!(mutes.list().unwrap()[0].name, "Bram");
    }

    #[test]
    fn a_failed_first_read_shows_an_empty_list() {
        let mut mutes = Mutes::default();
        mutes.sent();
        mutes.failed();
        assert_eq!(mutes.list(), Some(&[][..]));
    }
}
