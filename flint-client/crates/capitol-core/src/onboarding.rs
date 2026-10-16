//! First-session hints (P8.3): walk, gather, look in the inventory, craft
//! rope. One hint shows at a time, the first step not yet done. Doing a step
//! in any order counts, so a player who runs ahead never sees a hint for
//! something they've already done.
//!
//! A [`Tour`] is one account's progress, saved by the client as a byte
//! ([`Tour::bits`]) so the hints show once per account. Dismissing it ends
//! the tour for good. An account that already holds things it could only
//! have made has done it all, so an old account seeing the client for the
//! first time is never taught to walk ([`Tour::judge`]).

/// A thing a new player is shown how to do, in the order they're shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    Walk,
    Gather,
    Inventory,
    Craft,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Walk, Step::Gather, Step::Inventory, Step::Craft];

    fn bit(self) -> u8 {
        1 << self as u8
    }

    /// Its place in the tour, from 0.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Something the player did that a step is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Did {
    /// Asked to walk somewhere.
    Walked,
    /// An extraction yielded.
    Gathered,
    /// Opened or closed the inventory panel, or moved something in it.
    UsedInventory,
    /// Opened the crafting panel, which is past looking at the inventory.
    OpenedCrafting,
    /// A craft finished.
    Crafted,
}

/// Every step's bit.
const ALL_DONE: u8 = 0b1111;
/// Set when the player dismissed the hints.
const DISMISSED: u8 = 0x80;

/// One account's progress through the hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tour {
    bits: u8,
}

impl Tour {
    /// A tour as saved by [`Tour::bits`]. Unknown bits are kept.
    pub fn from_bits(bits: u8) -> Tour {
        Tour { bits }
    }

    /// What to save.
    pub fn bits(self) -> u8 {
        self.bits
    }

    /// The hint to show, or `None` once every step is done or the tour was
    /// dismissed.
    pub fn current(self) -> Option<Step> {
        if self.bits & DISMISSED != 0 {
            return None;
        }
        Step::ALL.into_iter().find(|s| !self.done(*s))
    }

    pub fn done(self, step: Step) -> bool {
        self.bits & step.bit() != 0
    }

    /// Every step done, as opposed to dismissed.
    pub fn completed(self) -> bool {
        self.bits & ALL_DONE == ALL_DONE
    }

    pub fn dismissed(self) -> bool {
        self.bits & DISMISSED != 0
    }

    /// Over, either way: no hint will show again.
    pub fn over(self) -> bool {
        self.current().is_none()
    }

    /// Count `did`. True if that changed the tour (it's worth saving).
    pub fn saw(&mut self, did: Did) -> bool {
        let steps: &[Step] = match did {
            Did::Walked => &[Step::Walk],
            Did::Gathered => &[Step::Gather],
            Did::UsedInventory => &[Step::Inventory],
            Did::OpenedCrafting => &[Step::Inventory],
            Did::Crafted => &[Step::Craft],
        };
        self.mark(steps.iter().copied())
    }

    /// Stop the hints for good. True if they weren't stopped already.
    pub fn dismiss(&mut self) -> bool {
        let before = self.bits;
        self.bits |= DISMISSED;
        self.bits != before
    }

    /// Mark what the account's inventory shows it has done already:
    /// anything at all means it has gathered (and so walked), and anything
    /// that isn't a raw resource means it has crafted, which is the whole
    /// tour. `is_raw` tells raw resources (fiber) from made things. True if
    /// that changed the tour.
    pub fn judge<'a>(
        &mut self,
        item_types: impl IntoIterator<Item = &'a str>,
        is_raw: impl Fn(&str) -> bool,
    ) -> bool {
        let mut any = false;
        let mut made = false;
        for item_type in item_types {
            any = true;
            made |= !is_raw(item_type);
        }
        if made {
            self.mark(Step::ALL)
        } else if any {
            self.mark([Step::Walk, Step::Gather])
        } else {
            false
        }
    }

    fn mark(&mut self, steps: impl IntoIterator<Item = Step>) -> bool {
        let before = self.bits;
        for step in steps {
            self.bits |= step.bit();
        }
        self.bits != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(t: &str) -> bool {
        t == "fiber"
    }

    #[test]
    fn a_new_account_is_walked_through_in_order() {
        let mut tour = Tour::default();
        assert_eq!(tour.current(), Some(Step::Walk));
        assert!(tour.saw(Did::Walked));
        assert_eq!(tour.current(), Some(Step::Gather));
        assert!(tour.saw(Did::Gathered));
        assert_eq!(tour.current(), Some(Step::Inventory));
        assert!(tour.saw(Did::UsedInventory));
        assert_eq!(tour.current(), Some(Step::Craft));
        assert!(tour.saw(Did::Crafted));
        assert_eq!(tour.current(), None);
        assert!(tour.completed() && !tour.dismissed() && tour.over());
    }

    #[test]
    fn running_ahead_skips_what_was_done() {
        let mut tour = Tour::default();
        // Clicking grass from afar walks there and gathers: both count.
        tour.saw(Did::Gathered);
        assert_eq!(tour.current(), Some(Step::Walk));
        tour.saw(Did::Walked);
        assert_eq!(tour.current(), Some(Step::Inventory));
        // Opening crafting is past looking at the inventory.
        assert!(tour.saw(Did::OpenedCrafting));
        assert_eq!(tour.current(), Some(Step::Craft));
    }

    #[test]
    fn seeing_it_again_changes_nothing() {
        let mut tour = Tour::default();
        assert!(tour.saw(Did::Walked));
        assert!(!tour.saw(Did::Walked));
    }

    #[test]
    fn dismissing_ends_it_for_good_and_survives_a_save() {
        let mut tour = Tour::default();
        tour.saw(Did::Walked);
        assert!(tour.dismiss());
        assert!(!tour.dismiss());
        assert_eq!(tour.current(), None);
        assert!(tour.dismissed() && !tour.completed());
        let back = Tour::from_bits(tour.bits());
        assert_eq!(back, tour);
        assert!(back.over());
        assert!(back.done(Step::Walk));
    }

    #[test]
    fn an_old_account_is_judged_by_what_it_holds() {
        let mut empty = Tour::default();
        assert!(!empty.judge([], raw));
        assert_eq!(empty.current(), Some(Step::Walk));

        let mut gatherer = Tour::default();
        assert!(gatherer.judge(["fiber"], raw));
        assert_eq!(gatherer.current(), Some(Step::Inventory));

        let mut crafter = Tour::default();
        assert!(crafter.judge(["fiber", "rope"], raw));
        assert!(crafter.completed());
        assert_eq!(crafter.current(), None);
    }
}
