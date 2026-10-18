//! Hand tools (GDD §6.4 and §6.5, economy plan D3): how hard a tool works, how
//! long it lasts, and how its output falls off as it wears.
//!
//! - A tool's material tier is its item type: `stone_sickle` and `bronze_sickle`
//!   are different items, each with its own `base_power` and base durability.
//! - Quality scales both. [`effective_power`] is `base_power × quality_multiplier(q)`,
//!   and a better tool also lasts longer ([`max_durability`]).
//! - Each use costs 1 durability. [`retention`] is how much of the tool's quality
//!   still reaches the work: a plateau, a decline, then a cliff before it breaks.
//! - With several tools of a type to hand, the one used is the one doing the most
//!   work right now, power × retention ([`best_tool`]), so excellent bronze beats
//!   poor steel when the numbers say so.

use std::fmt;

/// What a tool is for. An item type is a tool when it names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolType {
    Sickle,
    Axe,
    Pickaxe,
    Hammer,
    Saw,
}

impl ToolType {
    pub const ALL: [ToolType; 5] = [Self::Sickle, Self::Axe, Self::Pickaxe, Self::Hammer, Self::Saw];

    /// The id content and the wire use, e.g. `"sickle"`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sickle => "sickle",
            Self::Axe => "axe",
            Self::Pickaxe => "pickaxe",
            Self::Hammer => "hammer",
            Self::Saw => "saw",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

impl fmt::Display for ToolType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A tool's material, lowest first. The tier only names the material: what it's
/// worth is the item type's `base_power` and `max_durability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Stone,
    Bronze,
    Iron,
    Steel,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Self::Stone, Self::Bronze, Self::Iron, Self::Steel];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stone => "stone",
            Self::Bronze => "bronze",
            Self::Iron => "iron",
            Self::Steel => "steel",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// Durability one use takes.
pub const WEAR_PER_USE: i32 = 1;

/// Above this share of its durability a tool works at full quality (GDD §6.5).
pub const PLATEAU_FLOOR: f64 = 0.20;
/// Below this share it's on the cliff, and breakage is close.
pub const CLIFF_EDGE: f64 = 0.05;
/// Retention at the bottom of the decline, `CLIFF_EDGE`.
pub const DECLINE_FLOOR: f64 = 0.5;
/// Retention at the top of the cliff, just under `CLIFF_EDGE`. It falls from
/// here to 0 at no durability.
pub const CLIFF_TOP: f64 = 0.2;

/// How much a tool's quality scales its material: 0.5× at Q0, 1× at Q50, 1.5× at
/// Q100, a straight line between (GDD §6.4).
pub fn quality_multiplier(quality: i32) -> f64 {
    0.5 + quality.clamp(0, 100) as f64 / 100.0
}

/// How hard a tool works: its material's `base_power` scaled by its quality.
pub fn effective_power(base_power: f64, quality: i32) -> f64 {
    base_power * quality_multiplier(quality)
}

/// A new tool's durability: its type's base, scaled by quality the same way as its
/// power, so a Q100 tool lasts three times as long as a Q0 one. Never under 1.
pub fn max_durability(base: i32, quality: i32) -> i32 {
    ((base as f64 * quality_multiplier(quality)).round() as i32).max(1)
}

/// The share of a tool's quality that reaches its work at `durability` of `max`:
/// - 1.0 from full down to 20% (the plateau)
/// - a straight line from 1.0 at 20% to 0.5 at 5% (the decline)
/// - below 5%, from 0.2 down to 0 at none left (the cliff)
pub fn retention(durability: i32, max: i32) -> f64 {
    if max <= 0 || durability <= 0 {
        return 0.0;
    }
    let left = (durability as f64 / max as f64).min(1.0);
    if left >= PLATEAU_FLOOR {
        1.0
    } else if left >= CLIFF_EDGE {
        DECLINE_FLOOR + (1.0 - DECLINE_FLOOR) * (left - CLIFF_EDGE) / (PLATEAU_FLOOR - CLIFF_EDGE)
    } else {
        CLIFF_TOP * left / CLIFF_EDGE
    }
}

/// One tool, as the rules see it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tool {
    pub base_power: f64,
    pub quality: i32,
    pub durability: i32,
    /// This tool's own maximum, [`max_durability`] of its type's base and its quality.
    pub max_durability: i32,
}

impl Tool {
    pub fn effective_power(&self) -> f64 {
        effective_power(self.base_power, self.quality)
    }

    pub fn retention(&self) -> f64 {
        retention(self.durability, self.max_durability)
    }

    /// What [`best_tool`] ranks by: the work it would do now.
    pub fn score(&self) -> f64 {
        self.effective_power() * self.retention()
    }
}

/// The best of `tools` (all of one tool type), by [`Tool::score`]. Ties go to the
/// first, so a caller that lists by slot gets the same pick every time. A tool with
/// nothing left never wins.
pub fn best_tool<K>(tools: impl IntoIterator<Item = (K, Tool)>) -> Option<K> {
    let mut best: Option<(K, f64)> = None;
    for (key, tool) in tools {
        let score = tool.score();
        if score > 0.0 && best.as_ref().is_none_or(|(_, b)| score > *b) {
            best = Some((key, score));
        }
    }
    best.map(|(key, _)| key)
}

// ---------------------------------------------------------------------------
// Tools at work (E1.5)
// ---------------------------------------------------------------------------

/// How much of a resource's `difficulty` a tool's power covers, capped at
/// [`MAX_LEVERAGE`]. Difficulty is the power that counts as the right tool for
/// the job, so leverage 1 is a Q50 tool of the first tier that fits.
pub fn leverage(power: f64, difficulty: f64) -> f64 {
    if difficulty <= 0.0 {
        return MAX_LEVERAGE;
    }
    (power / difficulty).clamp(0.0, MAX_LEVERAGE)
}

/// Past this much power over difficulty, a better tool gathers no more.
pub const MAX_LEVERAGE: f64 = 4.0;
/// Extra yield per unit of [`leverage`]: half again at 1, three times at the cap.
pub const YIELD_PER_LEVERAGE: f64 = 0.5;
/// Extra speed per unit of [`leverage`]: a quarter faster at 1, twice as fast at the cap.
pub const SPEED_PER_LEVERAGE: f64 = 0.25;
/// The most quality a tool at leverage 1 can knock off a gather (GDD §6.2's
/// extraction damage). It shrinks as the leverage grows and grows as it falls,
/// up to [`MAX_DAMAGE`].
pub const DAMAGE_AT_PAR: f64 = 5.0;
pub const MAX_DAMAGE: i32 = 15;

/// How many units a gather gives with a tool, from the `rolled` units the
/// resource gives by hand: `rolled × (1 + 0.5 × leverage)`, rounded, never fewer.
pub fn tool_yield(rolled: i32, power: f64, difficulty: f64) -> i32 {
    let factor = 1.0 + YIELD_PER_LEVERAGE * leverage(power, difficulty);
    ((rolled as f64 * factor).round() as i32).max(rolled)
}

/// How long a gather takes with a tool, from the resource's `base` ticks:
/// `base / (1 + 0.25 × leverage)`, rounded up, at least 1.
pub fn tool_duration(base: i32, power: f64, difficulty: f64) -> i32 {
    let speed = 1.0 + SPEED_PER_LEVERAGE * leverage(power, difficulty);
    ((base as f64 / speed).ceil() as i32).max(1)
}

/// The most quality a gather with this tool can lose: [`DAMAGE_AT_PAR`] at
/// leverage 1, less for a tool well above the resource, more for one below it
/// (GDD §6.2: a poor tool damages what it takes).
pub fn max_damage(power: f64, difficulty: f64) -> i32 {
    let lev = leverage(power, difficulty);
    if lev <= 0.0 {
        return MAX_DAMAGE;
    }
    ((DAMAGE_AT_PAR / lev).round() as i32).clamp(0, MAX_DAMAGE)
}

/// What a gather with a tool comes out at. The node's quality is the ceiling
/// (GDD §6.2: gathering can't improve it). The tool's [`retention`] scales it,
/// and `damage` (rolled in `0..=max_damage`) takes some off:
/// `min(node, round(node × retention) − damage)`, clamped to 0..=100.
pub fn gathered_quality(node_quality: i32, retention: f64, damage: i32) -> i32 {
    let kept = (node_quality as f64 * retention.clamp(0.0, 1.0)).round() as i32;
    (kept - damage.max(0)).min(node_quality).clamp(0, 100)
}

/// The quality a tool brings to a craft's tool term (`rules::quality`): its
/// own quality, scaled by how much of it still reaches the work.
pub fn term_quality(tool: &Tool) -> i32 {
    (tool.quality as f64 * tool.retention()).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_quality_multiplier_runs_from_half_to_one_and_a_half() {
        assert!(close(quality_multiplier(0), 0.5));
        assert!(close(quality_multiplier(50), 1.0));
        assert!(close(quality_multiplier(100), 1.5));
        assert!(close(quality_multiplier(90), 1.4));
        assert!(close(quality_multiplier(30), 0.8));
        assert!(close(quality_multiplier(-5), 0.5), "clamped below");
        assert!(close(quality_multiplier(120), 1.5), "clamped above");
    }

    /// GDD §6.4's example: bronze (base 10) at Q90 is 14. Its steel (base 18) is
    /// 12.6, which takes 0.7×; the GDD says Q30, but the line through its own
    /// three points puts 0.7× at Q20 (Q30 is 0.8×, 14.4, and beats the bronze).
    /// The line is the rule, so the example's steel is Q20.
    #[test]
    fn effective_power_is_the_gdd_example() {
        assert!(close(effective_power(10.0, 90), 14.0));
        assert!(close(effective_power(18.0, 20), 12.6));
    }

    #[test]
    fn excellent_bronze_beats_poor_steel_when_power_says_so() {
        let bronze = Tool { base_power: 10.0, quality: 90, durability: 100, max_durability: 100 };
        let poor_steel = Tool { base_power: 18.0, quality: 20, durability: 100, max_durability: 100 };
        assert_eq!(best_tool([("steel", poor_steel), ("bronze", bronze)]), Some("bronze"));

        // Fair steel is another matter
        let fair_steel = Tool { quality: 50, ..poor_steel };
        assert_eq!(best_tool([("bronze", bronze), ("steel", fair_steel)]), Some("steel"));
    }

    #[test]
    fn a_worn_tool_loses_to_a_fresh_weaker_one() {
        let worn = Tool { base_power: 18.0, quality: 80, durability: 2, max_durability: 100 };
        let fresh = Tool { base_power: 10.0, quality: 50, durability: 100, max_durability: 100 };
        assert!(worn.effective_power() > fresh.effective_power());
        assert_eq!(best_tool([("worn", worn), ("fresh", fresh)]), Some("fresh"));
    }

    #[test]
    fn a_broken_tool_is_never_picked_and_ties_go_to_the_first() {
        let spent = Tool { base_power: 30.0, quality: 100, durability: 0, max_durability: 100 };
        assert_eq!(best_tool([(1, spent)]), None);
        let a = Tool { base_power: 10.0, quality: 50, durability: 50, max_durability: 50 };
        assert_eq!(best_tool([(1, a), (2, a)]), Some(1));
        assert_eq!(best_tool(Vec::<(i32, Tool)>::new()), None);
    }

    /// The knees of GDD §6.5's plateau, decline and cliff
    #[test]
    fn retention_is_a_plateau_a_decline_and_a_cliff() {
        // Plateau
        assert!(close(retention(100, 100), 1.0));
        assert!(close(retention(21, 100), 1.0));
        assert!(close(retention(20, 100), 1.0), "the plateau's floor is still full");
        // Decline: linear from 1.0 at 20% to 0.5 at 5%
        assert!(close(retention(19, 100), 1.0 - 0.5 / 15.0));
        assert!(close(retention(125, 1000), 0.75), "halfway down the decline");
        assert!(close(retention(5, 100), 0.5), "the decline's floor");
        // Cliff: 0.2 just under 5%, down to 0
        assert!(close(retention(49, 1000), 0.2 * 0.049 / 0.05));
        assert!(retention(49, 1000) < 0.2);
        assert!(close(retention(25, 1000), 0.1));
        assert!(close(retention(1, 1000), 0.004));
        assert!(close(retention(0, 100), 0.0));
        // Never rises as the tool wears
        let mut last = f64::INFINITY;
        for d in (0..=1000).rev() {
            let r = retention(d, 1000);
            assert!(r <= last, "retention rose at {d}");
            last = r;
        }
    }

    #[test]
    fn a_better_tool_lasts_longer() {
        assert_eq!(max_durability(100, 0), 50);
        assert_eq!(max_durability(100, 50), 100);
        assert_eq!(max_durability(100, 100), 150);
        assert!(max_durability(80, 90) > max_durability(80, 60));
        assert_eq!(max_durability(1, 0), 1, "never under 1");
    }

    #[test]
    fn tool_types_and_tiers_round_trip_their_ids() {
        for t in ToolType::ALL {
            assert_eq!(ToolType::from_str(t.as_str()), Some(t));
        }
        for t in Tier::ALL {
            assert_eq!(Tier::from_str(t.as_str()), Some(t));
        }
        assert_eq!(ToolType::from_str("spoon"), None);
        assert!(Tier::Stone < Tier::Steel);
    }

    /// A stone sickle at Q50 (power 6) on grass (difficulty 6) is leverage 1
    #[test]
    fn the_right_tool_gathers_half_again_a_quarter_faster() {
        assert!(close(leverage(6.0, 6.0), 1.0));
        assert_eq!(tool_yield(2, 6.0, 6.0), 3);
        assert_eq!(tool_yield(1, 6.0, 6.0), 2, "1.5 rounds up");
        assert_eq!(tool_duration(30, 6.0, 6.0), 24);
        assert_eq!(max_damage(6.0, 6.0), 5);
    }

    #[test]
    fn more_power_helps_up_to_the_cap() {
        assert_eq!(tool_yield(3, 1000.0, 6.0), 9, "three times at the cap");
        assert_eq!(tool_duration(30, 1000.0, 6.0), 15, "twice as fast at the cap");
        assert_eq!(max_damage(1000.0, 6.0), 1);
        assert!(tool_yield(3, 12.0, 6.0) > tool_yield(3, 6.0, 6.0));
        // A weak tool never gathers less than hands do, but it damages more
        assert_eq!(tool_yield(3, 0.5, 6.0), 3);
        assert_eq!(max_damage(2.0, 6.0), 15);
        assert_eq!(max_damage(0.0, 6.0), MAX_DAMAGE);
        assert_eq!(tool_duration(1, 6.0, 6.0), 1, "never under a tick");
    }

    #[test]
    fn the_node_is_the_ceiling_and_wear_takes_quality_off() {
        assert_eq!(gathered_quality(60, 1.0, 0), 60, "a fresh tool, cleanly: the node");
        assert_eq!(gathered_quality(60, 1.0, 4), 56);
        assert_eq!(gathered_quality(60, 0.5, 0), 30, "half retention, half the quality");
        assert_eq!(gathered_quality(60, 1.5, 0), 60, "never over the node");
        assert_eq!(gathered_quality(3, 0.1, 15), 0, "never under 0");
    }

    #[test]
    fn a_worn_tool_brings_less_to_a_craft() {
        let fresh = Tool { base_power: 6.0, quality: 80, durability: 60, max_durability: 60 };
        assert_eq!(term_quality(&fresh), 80);
        let worn = Tool { durability: 3, ..fresh };
        assert_eq!(term_quality(&worn), 40, "0.5 retention at 5%");
    }
}
