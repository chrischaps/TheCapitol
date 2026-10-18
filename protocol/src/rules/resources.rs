//! Resource types as content (E2.2, economy plan D5): what a gather yields with
//! each tier of tool, and where a resource's nodes grow.
//!
//! - A resource's yield table has one row per tier that works it, plus `hand`
//!   if it can be gathered without a tool. A gather reads the row for its tool's
//!   tier ([`yield_for`]); a tier with no row reads the best one below it.
//! - Its spawn table names the zones its nodes grow in, how thickly, and the
//!   least quality they're drawn at there. Its [`Cluster`] says how they group.
//!   E2.3 places nodes from these.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::tools::Tier;

/// The yield table's row for gathering without a tool.
pub const HAND: &str = "hand";

/// One row of a resource's yield table: the units a gather gives, and how long
/// it takes before a tool's speed (`tools::tool_duration`) is applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ResourceYield {
    /// Fewest units a gather gives, inclusive
    pub min: i32,
    /// Most units a gather gives, inclusive
    pub max: i32,
    /// Ticks a gather takes at this tier, when not the resource's own
    /// `extraction_duration_ticks`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<i32>,
}

/// A resource's yield table, by `hand` or a tier id (`tools::Tier::as_str`).
pub type YieldTable = BTreeMap<String, ResourceYield>;

/// The row a gather reads: the tool's tier, or else the best tier listed below
/// it, or else `hand`. With no tool, `hand`. A table with no row that fits (a
/// gated resource gathered by hand) gives its lowest tier, so a caller always
/// has numbers; the tool gate refuses such a gather before it starts.
pub fn yield_for(table: &YieldTable, tier: Option<Tier>) -> Option<&ResourceYield> {
    let at_or_below = tier.and_then(|tier| {
        Tier::ALL.iter().rev().filter(|t| **t <= tier).find_map(|t| table.get(t.as_str()))
    });
    at_or_below
        .or_else(|| table.get(HAND))
        .or_else(|| Tier::ALL.iter().find_map(|t| table.get(t.as_str())))
}

/// How a resource's nodes group on the ground, for E2.3's placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cluster {
    /// One here, one there
    Scattered,
    /// Small knots of several together
    Clumped,
    /// Patches of wood, from a noise field
    Woods,
    /// Strung out along the banks of canals, moats and rivers
    AlongWater,
    /// Scattered, but near trees
    UnderTrees,
}

impl Cluster {
    pub const ALL: [Cluster; 5] =
        [Self::Scattered, Self::Clumped, Self::Woods, Self::AlongWater, Self::UnderTrees];

    /// The id content and the wire use, e.g. `"along_water"`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Scattered => "scattered",
            Self::Clumped => "clumped",
            Self::Woods => "woods",
            Self::AlongWater => "along_water",
            Self::UnderTrees => "under_trees",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

impl fmt::Display for Cluster {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of a resource's spawn table: a zone its nodes grow in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Spawn {
    /// A zone id (`rules::Zone::as_str`)
    pub zone: String,
    /// Nodes per km² of the zone's open ground (1 unit = 1 m)
    pub density: f64,
    /// The least base quality a node here is drawn at, when it's above the
    /// resource's own `base_quality_min` (D5: the Wilderness draws from the top
    /// of the range)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_floor: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(min: i32, max: i32) -> ResourceYield {
        ResourceYield { min, max, ticks: None }
    }

    fn table(rows: &[(&str, ResourceYield)]) -> YieldTable {
        rows.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn a_gather_reads_its_tiers_row_or_the_best_below() {
        let grass = table(&[(HAND, row(1, 3)), ("stone", row(2, 4)), ("iron", row(3, 5))]);
        assert_eq!(yield_for(&grass, None), Some(&row(1, 3)));
        assert_eq!(yield_for(&grass, Some(Tier::Stone)), Some(&row(2, 4)));
        assert_eq!(yield_for(&grass, Some(Tier::Bronze)), Some(&row(2, 4)), "no bronze row: stone's");
        assert_eq!(yield_for(&grass, Some(Tier::Iron)), Some(&row(3, 5)));
        assert_eq!(yield_for(&grass, Some(Tier::Steel)), Some(&row(3, 5)), "no steel row: iron's");

        // A tier below every row it lists falls back to hand
        let ore = table(&[(HAND, row(1, 1)), ("iron", row(2, 3))]);
        assert_eq!(yield_for(&ore, Some(Tier::Stone)), Some(&row(1, 1)));

        // A gated resource always has numbers
        let vein = table(&[("stone", row(1, 1)), ("iron", row(1, 2))]);
        assert_eq!(yield_for(&vein, None), Some(&row(1, 1)));
        assert_eq!(yield_for(&YieldTable::new(), None), None);
    }

    #[test]
    fn clusters_round_trip_their_ids() {
        for c in Cluster::ALL {
            assert_eq!(Cluster::from_str(c.as_str()), Some(c));
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{}\"", c.as_str()));
        }
        assert_eq!(Cluster::from_str("heap"), None);
    }
}
