//! Stacks and blending (economy plan D2): what a stack's quality is when two
//! join, and which joins the server may make without being asked.
//!
//! - A stack is one quality, the quantity-weighted mean of what went in, and
//!   the lowest quality that ever went in (`min_quality`). Blending is free:
//!   [`Lot::merge`] conserves total quality, less rounding down.
//! - A player's own merge (a slot drop onto a stack of the same type) blends
//!   any grades. It's their choice.
//! - A merge the server makes unasked (extraction, craft output, station
//!   returns, exchange withdrawals) only joins a stack that
//!   [`auto_stacks_with`] says matches, so an A stack is never diluted behind
//!   its owner's back. Otherwise the new lot takes a slot of its own.

use super::quality_grade;

/// A stack's quantity, quality and the lowest quality blended into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lot {
    pub quantity: i32,
    pub quality: i32,
    pub min_quality: i32,
}

impl Lot {
    /// A lot made fresh, from one source: its min is its quality.
    pub fn fresh(quantity: i32, quality: i32) -> Self {
        Self {
            quantity,
            quality,
            min_quality: quality,
        }
    }

    /// This lot and `other` as one stack: the mean weighted by quantity, rounded
    /// down, and the lower min. `None` if the quantity overflows.
    pub fn merge(self, other: Lot) -> Option<Lot> {
        let quantity = self.quantity.checked_add(other.quantity)?;
        if quantity <= 0 {
            return Some(Lot { quantity, ..self });
        }
        let total = self.quality as i64 * self.quantity as i64
            + other.quality as i64 * other.quantity as i64;
        Some(Lot {
            quantity,
            quality: (total / quantity as i64) as i32,
            min_quality: self.min_quality.min(other.min_quality),
        })
    }

    /// The first `quantity` of this lot, and what's left. Both keep the quality
    /// and the min: you can't un-mix a blend.
    pub fn split(self, quantity: i32) -> (Lot, Lot) {
        (
            Lot { quantity, ..self },
            Lot {
                quantity: self.quantity - quantity,
                ..self
            },
        )
    }

    /// Whether this lot reads as a blend: its min grade isn't its grade.
    pub fn is_blended(&self) -> bool {
        quality_grade(self.min_quality) != quality_grade(self.quality)
    }
}

/// Whether the server may merge `a` and `b` without being asked: the same
/// item type, the same grade, and the same grade of min. An unblended A never
/// joins a blended A, and an A never joins a C.
pub fn auto_stacks_with(a_type: &str, a: &Lot, b_type: &str, b: &Lot) -> bool {
    a_type == b_type
        && quality_grade(a.quality) == quality_grade(b.quality)
        && quality_grade(a.min_quality) == quality_grade(b.min_quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_merge_is_the_weighted_mean_and_the_lower_min() {
        let a = Lot {
            quantity: 3,
            quality: 95,
            min_quality: 92,
        };
        let d = Lot::fresh(1, 40);
        assert_eq!(
            a.merge(d),
            Some(Lot {
                quantity: 4,
                quality: 81,
                min_quality: 40
            })
        );
    }

    #[test]
    fn a_merge_rounds_down_and_never_overflows_the_product() {
        let big = Lot::fresh(i32::MAX / 2, 100);
        let merged = big.merge(Lot::fresh(1, 0)).unwrap();
        assert_eq!(merged.quality, 99);
        assert_eq!(Lot::fresh(i32::MAX, 50).merge(Lot::fresh(1, 50)), None);
    }

    #[test]
    fn a_split_keeps_quality_and_min() {
        let blend = Lot {
            quantity: 10,
            quality: 91,
            min_quality: 40,
        };
        let (taken, left) = blend.split(4);
        assert_eq!(
            taken,
            Lot {
                quantity: 4,
                quality: 91,
                min_quality: 40
            }
        );
        assert_eq!(
            left,
            Lot {
                quantity: 6,
                quality: 91,
                min_quality: 40
            }
        );
        assert!(taken.is_blended());
    }

    #[test]
    fn the_server_joins_only_the_same_grade_and_min_grade() {
        let a = Lot::fresh(5, 93);
        let a2 = Lot::fresh(2, 97);
        let c = Lot::fresh(5, 60);
        let blended_a = Lot {
            quantity: 5,
            quality: 91,
            min_quality: 40,
        };
        assert!(auto_stacks_with("fiber", &a, "fiber", &a2));
        assert!(!auto_stacks_with("fiber", &a, "fiber", &c));
        assert!(!auto_stacks_with("fiber", &a, "fiber", &blended_a));
        assert!(!auto_stacks_with("fiber", &a, "rope", &a2));
    }
}
