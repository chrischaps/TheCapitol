//! The ring's sectors and the deck of their characters (plan RD1, RD2).
//!
//! The arterials cut the ring into sectors. Every sector draws from its own
//! stream, `stream(seed, &[RURAL, s])`, so a tweak in one never re-ids the
//! farms of another. Which sectors are concession and which are winding is
//! dealt from a deck of half and half, shuffled on `stream(seed, &[RURAL])`:
//! a coin per sector could give seven of one.

use capitol_protocol::fmath::StableMath;
use capitol_protocol::terrain::{RoadSegment, RoadType};

use crate::models::geom::SplitMix64;

type P = (f64, f64);

/// The rural streams' first key, "RURAL" in ASCII, so they don't meet the
/// suburb's, which are keyed by sector and neighbourhood alone.
pub const RURAL: u64 = 0x52_5552_414c;

/// A sector's roads (RD3, RD4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Character {
    /// Straight roads, as a surveyor drew them.
    Concession,
    /// Lanes that wind.
    Winding,
}

/// One sector of the ring, between two arterials.
#[derive(Debug, Clone, Copy)]
pub struct Sector {
    /// 0 up, in bearing order from the first arterial at or past 0°.
    pub index: usize,
    /// The bearing of the arterial it starts at, in radians...
    pub from: f64,
    /// ...and of the one it ends at, more than `from` (unwrapped past 2π).
    pub to: f64,
    pub character: Character,
}

impl Sector {
    pub fn span(&self) -> f64 {
        self.to - self.from
    }

    /// The bearing a fraction `t` of the way across, 0 at `from`.
    pub fn bearing(&self, t: f64) -> f64 {
        self.from + self.span() * t
    }
}

/// The point at radius `r` and bearing `a` (radians) from `center`.
pub fn polar(center: P, r: f64, a: f64) -> P {
    (center.0 + r * a.stable_cos(), center.1 + r * a.stable_sin())
}

/// The bearings of the arterials that run into the ring, from 0 to 2π,
/// sorted. They're read off the roads, so the sectors follow the arterials
/// wherever the world puts them.
pub fn arterial_bearings(roads: &[RoadSegment], center: P, inner: f64) -> Vec<f64> {
    let mut out: Vec<f64> = roads
        .iter()
        .filter(|r| r.road_type == RoadType::Arterial && r.id.starts_with("arterial_"))
        .filter_map(|r| r.points.last())
        .filter(|p| ((p.0 - center.0).powi(2) + (p.1 - center.1).powi(2)).sqrt() > inner)
        .map(|p| (p.1 - center.1).stable_atan2(p.0 - center.0).rem_euclid(std::f64::consts::TAU))
        .collect();
    out.sort_by(f64::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    out
}

/// The deck: half the sectors concession and half winding (the odd one out,
/// if any, winding), in a seeded shuffle.
pub fn deal(seed: u64, n: usize) -> Vec<Character> {
    let mut deck: Vec<Character> =
        (0..n).map(|i| if i < n / 2 { Character::Concession } else { Character::Winding }).collect();
    let mut rng = SplitMix64::stream(seed, &[RURAL]);
    // Fisher-Yates.
    for i in (1..n).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        deck.swap(i, j);
    }
    deck
}

/// The sectors between the arterials at `bearings`, each with its character.
pub fn sectors(seed: u64, bearings: &[f64]) -> Vec<Sector> {
    let n = bearings.len();
    let deck = deal(seed, n);
    (0..n)
        .map(|s| {
            let from = bearings[s];
            let to = if s + 1 < n { bearings[s + 1] } else { bearings[0] + std::f64::consts::TAU };
            Sector { index: s, from, to, character: deck[s] }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deck_is_half_and_half_on_every_seed() {
        for seed in 0..200 {
            let deck = deal(seed, 8);
            let concession = deck.iter().filter(|&&c| c == Character::Concession).count();
            assert_eq!(concession, 4, "seed {seed}: {deck:?}");
        }
    }

    #[test]
    fn the_shuffle_varies_which_sector_gets_which() {
        let decks: std::collections::HashSet<Vec<Character>> = (0..50).map(|seed| deal(seed, 8)).collect();
        assert!(decks.len() > 20, "only {} different deals in 50 seeds", decks.len());
    }

    #[test]
    fn sectors_run_from_arterial_to_arterial_round_the_ring() {
        let bearings: Vec<f64> = (0..8).map(|i| (i as f64 * 45.0).to_radians()).collect();
        let s = sectors(42, &bearings);
        assert_eq!(s.len(), 8);
        for (i, sector) in s.iter().enumerate() {
            assert!((sector.span() - 45f64.to_radians()).abs() < 1e-9, "sector {i}");
        }
        assert!((s[7].to - std::f64::consts::TAU).abs() < 1e-9, "the last sector wraps to the first arterial");
    }
}
