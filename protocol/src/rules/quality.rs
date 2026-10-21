//! The quality cascade (GDD §6.3, economy plan D1): what a craft's output is
//! worth, from what went in.
//!
//! `q = clamp(round(Σ wᵢ·qᵢ + w_station·q_station + w_tool·q_tool) + variance, 0, 100)`
//!
//! - `qᵢ` is the quantity-weighted mean quality of the units a craft takes of
//!   input `i` ([`Inputs`]).
//! - The weights sum to 1 ([`Weights::normalised`]).
//! - The variance is a whole number in `variance_min..=variance_max`, rolled
//!   by the server. Everything else is fixed by the inputs, so a client that
//!   knows them gets the exact `[min, max]` the server will land in
//!   ([`output_range`]), the same move `Fees::deposit_quote` makes for the fee.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The station's share of the output when a recipe needs a station and
/// doesn't say.
pub const DEFAULT_STATION_WEIGHT: f64 = 0.2;

/// The station term of a hand craft. A hand recipe weights the station 0, so
/// this never counts; a craft at a station passes that station's quality.
pub const NO_STATION: i32 = 0;

/// How much each source's quality counts toward the output's, summing to 1,
/// and the spread the server adds on top.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Weights {
    /// Item type → its share.
    pub inputs: BTreeMap<String, f64>,
    /// The station's share; 0 for a hand recipe.
    pub station: f64,
    /// The tool's share; 0 until tools work (E1.4, #156).
    pub tool: f64,
    pub variance_min: i32,
    pub variance_max: i32,
}

impl Weights {
    /// A recipe's weights, with the gaps filled and the sum brought to 1.
    ///
    /// - `inputs` are relative: they split whatever the station and tool
    ///   leave, in proportion. All zero (or none given) is an even split.
    /// - `station` and `tool` are shares of the whole. Left out, the station
    ///   takes [`DEFAULT_STATION_WEIGHT`] when the recipe needs one, and the
    ///   tool takes nothing.
    /// - Negative weights count as 0. If the station and tool ask for more
    ///   than the whole, everything is scaled down to fit.
    pub fn for_recipe<'a>(
        inputs: impl IntoIterator<Item = (&'a str, f64)>,
        station: Option<f64>,
        tool: Option<f64>,
        needs_station: bool,
        variance_min: i32,
        variance_max: i32,
    ) -> Self {
        let station = station
            .unwrap_or(if needs_station { DEFAULT_STATION_WEIGHT } else { 0.0 })
            .max(0.0);
        let tool = tool.unwrap_or(0.0).max(0.0);
        let mut raw: BTreeMap<String, f64> = BTreeMap::new();
        for (item, weight) in inputs {
            *raw.entry(item.to_string()).or_default() += weight.max(0.0);
        }
        let sum: f64 = raw.values().sum();
        let rest = (1.0 - station - tool).max(0.0);
        let n = raw.len() as f64;
        let inputs = raw
            .into_iter()
            .map(|(item, w)| {
                let share = if sum > 0.0 { w / sum } else { 1.0 / n };
                (item, rest * share)
            })
            .collect();
        Weights {
            inputs,
            station,
            tool,
            variance_min,
            variance_max,
        }
        .normalised()
    }

    /// The same weights scaled to sum to 1. Weights that are all zero become
    /// an even split over the inputs.
    pub fn normalised(mut self) -> Self {
        for w in self.inputs.values_mut() {
            *w = w.max(0.0);
        }
        self.station = self.station.max(0.0);
        self.tool = self.tool.max(0.0);
        let total = self.inputs.values().sum::<f64>() + self.station + self.tool;
        if total > 0.0 {
            for w in self.inputs.values_mut() {
                *w /= total;
            }
            self.station /= total;
            self.tool /= total;
        } else if !self.inputs.is_empty() {
            let even = 1.0 / self.inputs.len() as f64;
            for w in self.inputs.values_mut() {
                *w = even;
            }
        }
        self
    }
}

/// The units a craft takes, by item type: how many, and their summed quality,
/// so each input's quality is the quantity-weighted mean of what's used.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inputs {
    units: BTreeMap<String, (i64, i64)>,
}

impl Inputs {
    /// `quantity` units of `item_type` at `quality`.
    pub fn add(&mut self, item_type: &str, quantity: i32, quality: i32) {
        let (units, sum) = self.units.entry(item_type.to_string()).or_default();
        *units += quantity as i64;
        *sum += quantity as i64 * quality as i64;
    }

    /// Each of `units` of `item_type` (E1.8).
    pub fn add_units(&mut self, item_type: &str, units: &super::stacking::Units) {
        for (quality, count) in units.pairs() {
            self.add(item_type, count, quality);
        }
    }

    /// The mean quality of the `item_type` units taken, if any were.
    pub fn mean(&self, item_type: &str) -> Option<f64> {
        self.units
            .get(item_type)
            .filter(|(units, _)| *units > 0)
            .map(|&(units, sum)| sum as f64 / units as f64)
    }
}

/// Where a share of the output's quality comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Input(String),
    Station,
    Tool,
}

/// One source's part in the output: its weight and its quality.
#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub source: Source,
    pub weight: f64,
    pub quality: f64,
}

impl Term {
    /// Quality points this term puts into the output.
    pub fn points(&self) -> f64 {
        self.weight * self.quality
    }
}

/// The breakdown: each input in item order, then the station and the tool
/// if they carry any weight. An input with nothing taken counts as 0.
pub fn terms(inputs: &Inputs, station: i32, tool: i32, weights: &Weights) -> Vec<Term> {
    let mut terms: Vec<Term> = weights
        .inputs
        .iter()
        .map(|(item, &weight)| Term {
            source: Source::Input(item.clone()),
            weight,
            quality: inputs.mean(item).unwrap_or(0.0),
        })
        .collect();
    if weights.station > 0.0 {
        terms.push(Term {
            source: Source::Station,
            weight: weights.station,
            quality: station as f64,
        });
    }
    if weights.tool > 0.0 {
        terms.push(Term {
            source: Source::Tool,
            weight: weights.tool,
            quality: tool as f64,
        });
    }
    terms
}

/// The output's quality before the variance: the weighted sum, unrounded.
pub fn base(inputs: &Inputs, station: i32, tool: i32, weights: &Weights) -> f64 {
    terms(inputs, station, tool, weights).iter().map(Term::points).sum()
}

/// The output's quality for a rolled `variance`. The server calls this with
/// its roll; nothing else in it is random.
pub fn output(inputs: &Inputs, station: i32, tool: i32, weights: &Weights, variance: i32) -> i32 {
    let base = base(inputs, station, tool, weights).round() as i32;
    (base + variance).clamp(0, 100)
}

/// The lowest and highest quality the craft can come out at: [`output`] at
/// each end of the variance.
pub fn output_range(inputs: &Inputs, station: i32, tool: i32, weights: &Weights) -> (i32, i32) {
    (
        output(inputs, station, tool, weights, weights.variance_min),
        output(inputs, station, tool, weights, weights.variance_max),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(units: &[(&str, i32, i32)]) -> Inputs {
        let mut inputs = Inputs::default();
        for &(item, quantity, quality) in units {
            inputs.add(item, quantity, quality);
        }
        inputs
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn known_weights_give_known_outputs() {
        // The forge kit: rope 0.7 and fiber 0.3 split the 0.8 the workbench
        // leaves. 0.56·80 + 0.24·50 + 0.2·50 = 44.8 + 12 + 10 = 66.8 → 67.
        let w = Weights::for_recipe([("rope", 0.7), ("fiber", 0.3)], None, None, true, -3, 3);
        assert!(close(w.inputs["rope"], 0.56));
        assert!(close(w.inputs["fiber"], 0.24));
        assert!(close(w.station, 0.2));
        let i = inputs(&[("rope", 4, 80), ("fiber", 6, 50)]);
        assert!(close(base(&i, 50, 0, &w), 66.8));
        assert_eq!(output(&i, 50, 0, &w, 0), 67);
        assert_eq!(output(&i, 50, 0, &w, -3), 64);
        assert_eq!(output_range(&i, 50, 0, &w), (64, 70));
    }

    #[test]
    fn a_hand_recipe_is_the_mean_of_its_one_input() {
        let w = Weights::for_recipe([("fiber", 0.5)], None, None, false, -3, 3);
        assert_eq!(w.station, 0.0);
        assert!(close(w.inputs["fiber"], 1.0));
        // (3×40 + 2×71) / 5 = 52.4
        let i = inputs(&[("fiber", 3, 40), ("fiber", 2, 71)]);
        assert!(close(i.mean("fiber").unwrap(), 52.4));
        assert_eq!(output(&i, 50, 0, &w, 2), 54);
    }

    #[test]
    fn weights_summing_to_anything_are_normalised() {
        for scale in [0.001, 0.5, 1.0, 3.0, 250.0] {
            let w = Weights {
                inputs: [("a".to_string(), 2.0 * scale), ("b".to_string(), 1.0 * scale)].into(),
                station: 1.0 * scale,
                tool: 0.0,
                variance_min: 0,
                variance_max: 0,
            }
            .normalised();
            let total: f64 = w.inputs.values().sum::<f64>() + w.station + w.tool;
            assert!(close(total, 1.0), "scale {scale}: {total}");
            assert!(close(w.inputs["a"], 0.5));
            assert!(close(w.station, 0.25));
        }
        // From a recipe too, however the inputs are scaled.
        let a = Weights::for_recipe([("a", 7.0), ("b", 3.0)], Some(0.1), None, true, 0, 0);
        let b = Weights::for_recipe([("a", 0.7), ("b", 0.3)], Some(0.1), None, true, 0, 0);
        assert_eq!(a, b);
    }

    #[test]
    fn missing_weights_default_to_an_even_split() {
        let w = Weights::for_recipe([("a", 0.0), ("b", 0.0)], None, None, false, 0, 0);
        assert!(close(w.inputs["a"], 0.5) && close(w.inputs["b"], 0.5));
        let w = Weights {
            inputs: [("a".to_string(), 0.0), ("b".to_string(), 0.0)].into(),
            station: 0.0,
            tool: 0.0,
            variance_min: 0,
            variance_max: 0,
        }
        .normalised();
        assert!(close(w.inputs["a"], 0.5));
    }

    #[test]
    fn a_greedy_station_and_tool_are_scaled_to_fit() {
        let w = Weights::for_recipe([("a", 1.0)], Some(0.9), Some(0.3), true, 0, 0);
        assert_eq!(w.inputs["a"], 0.0);
        assert!(close(w.station, 0.75) && close(w.tool, 0.25));
    }

    #[test]
    fn the_output_is_clamped() {
        let w = Weights::for_recipe([("a", 1.0)], None, None, false, -10, 10);
        assert_eq!(output_range(&inputs(&[("a", 1, 95)]), 0, 0, &w), (85, 100));
        assert_eq!(output_range(&inputs(&[("a", 1, 4)]), 0, 0, &w), (0, 14));
    }

    #[test]
    fn the_range_brackets_every_roll() {
        // 1000 crafts from a cheap LCG: random inputs, weights and spread.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        for _ in 0..1000 {
            let w = Weights::for_recipe(
                [("a", next(100) as f64 / 10.0), ("b", next(100) as f64 / 10.0)],
                (next(2) == 0).then(|| next(50) as f64 / 100.0),
                None,
                next(2) == 0,
                -(next(10) as i32),
                next(10) as i32,
            );
            let mut i = Inputs::default();
            for _ in 0..1 + next(4) {
                i.add(if next(2) == 0 { "a" } else { "b" }, 1 + next(9) as i32, next(101) as i32);
            }
            let station = next(101) as i32;
            let (lo, hi) = output_range(&i, station, 0, &w);
            let roll = w.variance_min + next((w.variance_max - w.variance_min + 1) as u64) as i32;
            let q = output(&i, station, 0, &w, roll);
            assert!(lo <= q && q <= hi, "{q} outside {lo}..={hi}");
        }
    }
}
