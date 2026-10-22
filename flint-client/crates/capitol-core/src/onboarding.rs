//! The road in, as the client shows it (#441, `the-capitol-road-in.md`).
//!
//! The server keeps the tour and judges each step from what the player did
//! (#438, #439), so a [`Tour`] is only a view of the `tutorial` and
//! `tutorial_step` it sent (`GameState::tutorial`): which step is at hand,
//! where it sits in its chapter, and whether the card is hidden. Nothing is
//! kept on this machine, and nothing is judged from what the pack holds:
//! every account walks the road (plan D3).
//!
//! A few steps are only things on screen, which the server can't see:
//! opening the pack, reading a zone card. The client says it saw them
//! ([`Did`], sent as `tutorial_seen`), and the server takes that only for
//! the steps `protocol::tutorial` lets the client report (plan D7).
//!
//! Hiding the card is for now, never for good (plan D2): it's `dismissed`
//! on the server, so it holds on every device, and the tour waits.

use capitol_protocol::tutorial::{Step, TutorialProgress};

/// Something only the screen shows, which the server takes for a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Did {
    /// Opened or closed the pack, or moved something in it.
    UsedInventory,
    /// Opened the crafting panel, which is past looking in the pack.
    OpenedCrafting,
    /// Opened a zone card (#444).
    ReadZoneCard,
}

impl Did {
    /// The step it counts for.
    pub fn step(self) -> Step {
        match self {
            Did::UsedInventory | Did::OpenedCrafting => Step::OpenPack,
            Did::ReadZoneCard => Step::ReadZoneCard,
        }
    }
}

/// Where one account is on the road in, as the server last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tour {
    progress: TutorialProgress,
}

impl From<TutorialProgress> for Tour {
    fn from(progress: TutorialProgress) -> Tour {
        Tour { progress }
    }
}

impl Tour {
    pub fn progress(self) -> TutorialProgress {
        self.progress
    }

    /// The step at hand: the first not done, in the tour's order. Done out
    /// of order still counts. `None` once the tour is finished. Hidden or
    /// not, this is where the player is.
    pub fn current(self) -> Option<Step> {
        self.progress.done.current()
    }

    pub fn done(self, step: Step) -> bool {
        self.progress.done.has(step)
    }

    pub fn finished(self) -> bool {
        self.progress.finished()
    }

    /// The card is hidden for now, and there's still a road to walk.
    pub fn hidden(self) -> bool {
        self.progress.dismissed && !self.finished()
    }

    /// The step to tell the server `did` showed, if it isn't done yet.
    pub fn report(self, did: Did) -> Option<Step> {
        let step = did.step();
        (!self.done(step)).then_some(step)
    }
}

/// Where `step` sits in its chapter: its place from 0, and how many steps
/// the chapter has. A chapter's pips are drawn from these.
pub fn place_in_chapter(step: Step) -> (usize, usize) {
    let chapter = step.chapter();
    let at = chapter.steps().position(|s| s == step).unwrap_or(0);
    (at, chapter.steps().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::tutorial::{Chapter, Steps};

    fn tour(done: &[Step], dismissed: bool) -> Tour {
        Tour::from(TutorialProgress { done: done.iter().copied().collect(), dismissed })
    }

    #[test]
    fn a_new_account_starts_at_the_first_step() {
        let new = Tour::default();
        assert_eq!(new.current(), Some(Step::Walk));
        assert!(!new.finished() && !new.hidden());
    }

    #[test]
    fn the_step_at_hand_is_the_first_not_done_whatever_the_order() {
        // Clicking grass from afar walks there and gathers: both count.
        let ran_ahead = tour(&[Step::GatherGrass, Step::Deposit], false);
        assert_eq!(ran_ahead.current(), Some(Step::Walk));
        let half = tour(&[Step::Walk, Step::GatherGrass], false);
        assert_eq!(half.current(), Some(Step::OpenPack));
    }

    #[test]
    fn hidden_is_for_now_and_keeps_the_place() {
        let hidden = tour(&[Step::Walk], true);
        assert!(hidden.hidden());
        assert_eq!(hidden.current(), Some(Step::GatherGrass), "the tour waits");
        // Finished, there's nothing to hide
        let done = Tour::from(TutorialProgress { done: Steps::every(), dismissed: true });
        assert!(done.finished() && !done.hidden());
        assert_eq!(done.current(), None);
    }

    #[test]
    fn only_what_isnt_done_is_reported() {
        let new = Tour::default();
        assert_eq!(new.report(Did::UsedInventory), Some(Step::OpenPack));
        assert_eq!(new.report(Did::OpenedCrafting), Some(Step::OpenPack));
        assert_eq!(new.report(Did::ReadZoneCard), Some(Step::ReadZoneCard));
        let looked = tour(&[Step::OpenPack], false);
        assert_eq!(looked.report(Did::UsedInventory), None);
    }

    /// The server takes `tutorial_seen` only for steps it can't see, so
    /// the client never reports one it judges
    #[test]
    fn every_report_is_one_the_server_takes() {
        for did in [Did::UsedInventory, Did::OpenedCrafting, Did::ReadZoneCard] {
            assert!(did.step().judge().client_reports(), "{did:?}");
        }
    }

    #[test]
    fn a_step_knows_its_place_in_its_chapter() {
        assert_eq!(place_in_chapter(Step::Walk), (0, Chapter::Arrival.steps().count()));
        assert_eq!(place_in_chapter(Step::FellTree), (11, 12));
        assert_eq!(place_in_chapter(Step::SeeRuralLot), (0, 4));
        assert_eq!(place_in_chapter(Step::ClaimHome), (1, 2));
    }
}
