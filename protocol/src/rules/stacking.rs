//! Stacks (economy plan D2, E1.8): every unit keeps its own quality, whatever
//! it's stacked with, and a stack *shows* the mean.
//!
//! - Quality is a whole number from 0 to 100, and two units of a type at the
//!   same quality are interchangeable. So a stack is a count per quality,
//!   [`Units`]: nothing is lost by counting them, and there are at most 101
//!   counts a stack. The server keeps them in `item_units`.
//! - [`Units::merge`] adds the counts. Nothing is diluted, so any two stacks
//!   of a type may join, whoever joins them.
//! - Every take names its units: [`Units::take_units`] takes the ones asked
//!   for ("my six 92s"), and [`Units::take`] picks with a [`Pick`]. Crafting
//!   takes the best first ([`Pick::Best`]), so what's in the input box makes
//!   the best output it can; everything else takes the lowest first.
//! - The pip and the number are the mean ([`Units::mean`], rounded down). Two
//!   at 50 and two at 90 show 70; take one 90 out and the 90 comes out a 90,
//!   and the three left show 63.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::quality_grade;

/// The lowest and highest quality a unit can have.
pub const MIN_QUALITY: i32 = 0;
pub const MAX_QUALITY: i32 = 100;

/// A stack's units: how many there are at each quality. On the wire it's a
/// list of `[quality, count]` pairs, lowest quality first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "Vec<(i32, i32)>", into = "Vec<(i32, i32)>")]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "backend", schema(value_type = Vec<Vec<i32>>))]
pub struct Units(BTreeMap<i32, i32>);

/// Which units a take without named qualities takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Pick {
    /// The lowest first: a split, a trade, a kit
    #[default]
    Lowest,
    /// The best first: a craft's inputs
    Best,
}

impl Units {
    /// No units.
    pub fn new() -> Self {
        Self::default()
    }

    /// `count` units all at `quality`: a stack made fresh, from one source.
    pub fn uniform(count: i32, quality: i32) -> Self {
        let mut units = Self::new();
        units.add(quality, count);
        units
    }

    /// `[quality, count]` pairs, as the wire and `item_units` hold them. Pairs
    /// of the same quality add up; counts under 1 are dropped and qualities are
    /// clamped to 0..=100.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (i32, i32)>) -> Self {
        let mut units = Self::new();
        for (quality, count) in pairs {
            units.add(quality, count);
        }
        units
    }

    /// `[quality, count]` pairs, lowest quality first.
    pub fn pairs(&self) -> impl DoubleEndedIterator<Item = (i32, i32)> + '_ {
        self.0.iter().map(|(&q, &c)| (q, c))
    }

    /// `count` more units at `quality`. Saturates rather than overflow.
    pub fn add(&mut self, quality: i32, count: i32) {
        if count <= 0 {
            return;
        }
        let quality = quality.clamp(MIN_QUALITY, MAX_QUALITY);
        let held = self.0.entry(quality).or_default();
        *held = held.saturating_add(count);
    }

    /// How many units there are, in all.
    pub fn count(&self) -> i64 {
        self.0.values().map(|&c| c as i64).sum()
    }

    /// How many there are at `quality`.
    pub fn count_at(&self, quality: i32) -> i32 {
        self.0.get(&quality).copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The quantity-weighted mean quality, rounded down; `None` with no units.
    pub fn mean(&self) -> Option<i32> {
        let n = self.count();
        (n > 0).then(|| (self.total_quality() / n) as i32)
    }

    /// The summed quality of every unit.
    pub fn total_quality(&self) -> i64 {
        self.0.iter().map(|(&q, &c)| q as i64 * c as i64).sum()
    }

    /// The lowest quality held.
    pub fn min(&self) -> Option<i32> {
        self.0.keys().next().copied()
    }

    /// The highest quality held.
    pub fn max(&self) -> Option<i32> {
        self.0.keys().next_back().copied()
    }

    /// Whether the units span more than one grade (E1.2's ring).
    pub fn spans_grades(&self) -> bool {
        match (self.min(), self.max()) {
            (Some(lo), Some(hi)) => quality_grade(lo) != quality_grade(hi),
            _ => false,
        }
    }

    /// The units of `other` added to these. `false`, and nothing changed, if
    /// the stack would hold more than `i32::MAX` units.
    pub fn merge(&mut self, other: &Units) -> bool {
        if self.count() + other.count() > i32::MAX as i64 {
            return false;
        }
        for (q, c) in other.pairs() {
            self.add(q, c);
        }
        true
    }

    /// Whether every unit of `wanted` is here.
    pub fn contains(&self, wanted: &Units) -> bool {
        wanted.pairs().all(|(q, c)| self.count_at(q) >= c)
    }

    /// Take exactly `wanted` out. `None`, and nothing taken, unless every one
    /// of them is here.
    pub fn take_units(&mut self, wanted: &Units) -> Option<Units> {
        if !self.contains(wanted) {
            return None;
        }
        for (q, c) in wanted.pairs() {
            self.remove(q, c);
        }
        Some(wanted.clone())
    }

    /// Take `n` units at `quality`. `None`, and nothing taken, if there
    /// aren't that many.
    pub fn take_quality(&mut self, quality: i32, n: i32) -> Option<Units> {
        self.take_units(&Units::uniform(n, quality))
    }

    /// Which `n` units `pick` would take, without taking them. `None` if
    /// there are fewer than `n`, or `n` is under 1.
    pub fn pick(&self, n: i32, pick: Pick) -> Option<Units> {
        if n < 1 || self.count() < n as i64 {
            return None;
        }
        let order: Box<dyn Iterator<Item = (i32, i32)>> = match pick {
            Pick::Lowest => Box::new(self.pairs()),
            Pick::Best => Box::new(self.pairs().rev()),
        };
        let (mut left, mut taken) = (n, Units::new());
        for (q, c) in order {
            if left == 0 {
                break;
            }
            let used = c.min(left);
            taken.add(q, used);
            left -= used;
        }
        Some(taken)
    }

    /// Take `n` units, chosen by `pick`. `None`, and nothing taken, if there
    /// are fewer than `n`.
    pub fn take(&mut self, n: i32, pick: Pick) -> Option<Units> {
        let taken = self.pick(n, pick)?;
        self.take_units(&taken)
    }

    fn remove(&mut self, quality: i32, count: i32) {
        if let Some(held) = self.0.get_mut(&quality) {
            *held -= count;
            if *held <= 0 {
                self.0.remove(&quality);
            }
        }
    }
}

impl From<Vec<(i32, i32)>> for Units {
    fn from(pairs: Vec<(i32, i32)>) -> Self {
        Units::from_pairs(pairs)
    }
}

impl From<Units> for Vec<(i32, i32)> {
    fn from(units: Units) -> Self {
        units.pairs().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fifties_and_nineties() -> Units {
        Units::from_pairs([(50, 2), (90, 2)])
    }

    #[test]
    fn a_stack_shows_the_mean_and_every_unit_comes_out_as_itself() {
        let mut stack = fifties_and_nineties();
        assert_eq!(stack.mean(), Some(70));
        let ninety = stack.take_quality(90, 1).unwrap();
        assert_eq!(ninety, Units::uniform(1, 90));
        assert_eq!(stack.count(), 3);
        assert_eq!(stack.mean(), Some(63));
        assert_eq!((stack.min(), stack.max()), (Some(50), Some(90)));
    }

    #[test]
    fn a_merge_adds_counts_and_loses_nothing() {
        let mut a = Units::uniform(3, 95);
        let d = Units::uniform(1, 40);
        assert!(a.merge(&d));
        assert_eq!(a, Units::from_pairs([(40, 1), (95, 3)]));
        assert_eq!(a.mean(), Some(81));
        assert!(a.spans_grades());
        assert_eq!(a.take_quality(95, 3), Some(Units::uniform(3, 95)));
        assert_eq!(a, d, "the 40 is still a 40");
    }

    #[test]
    fn a_merge_never_overflows() {
        let mut big = Units::uniform(i32::MAX, 50);
        assert!(!big.merge(&Units::uniform(1, 50)));
        assert_eq!(big.count(), i32::MAX as i64);
        let mut half = Units::uniform(i32::MAX / 2, 100);
        assert!(half.merge(&Units::uniform(1, 0)));
        assert_eq!(half.mean(), Some(99));
    }

    #[test]
    fn the_default_pick_takes_the_lowest_and_crafting_the_best() {
        let stack = Units::from_pairs([(44, 1), (81, 3), (92, 6)]);
        assert_eq!(stack.pick(2, Pick::default()), Some(Units::from_pairs([(44, 1), (81, 1)])));
        assert_eq!(stack.pick(7, Pick::Best), Some(Units::from_pairs([(81, 1), (92, 6)])));
        assert_eq!(stack.pick(11, Pick::Lowest), None);
        assert_eq!(stack.pick(0, Pick::Lowest), None);
    }

    #[test]
    fn a_take_is_all_or_nothing_and_conserves_the_units() {
        let mut stack = Units::from_pairs([(44, 1), (81, 3), (92, 6)]);
        let before = stack.clone();
        assert_eq!(stack.take_units(&Units::from_pairs([(81, 1), (92, 7)])), None);
        assert_eq!(stack, before);
        let taken = stack.take(4, Pick::Best).unwrap();
        let mut back = stack.clone();
        assert!(back.merge(&taken));
        assert_eq!(back, before);
    }

    #[test]
    fn units_go_on_the_wire_as_quality_count_pairs() {
        let json = serde_json::to_string(&fifties_and_nineties()).unwrap();
        assert_eq!(json, "[[50,2],[90,2]]");
        let back: Units = serde_json::from_str("[[90,1],[50,2],[90,1]]").unwrap();
        assert_eq!(back, fifties_and_nineties());
    }

    #[test]
    fn qualities_are_clamped_and_empty_counts_dropped() {
        let units = Units::from_pairs([(-5, 1), (140, 2), (60, 0), (60, -3)]);
        assert_eq!(units, Units::from_pairs([(0, 1), (100, 2)]));
    }
}
