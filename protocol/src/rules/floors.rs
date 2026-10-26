//! Floors (Plots v2 §5.3 and §5.4, V3.3): building upward.
//!
//! - A floor is another storey of a home lot's building, with the
//!   footprint's indoor room again. There's no cap: each floor costs about
//!   twice the last ([`cost`]), so the price is the limit.
//! - It's a permit in Strands, [`PERMIT_BASE`] doubling, times the live
//!   `floor_permit_scale` tunable, plus building goods from the builder's
//!   pack, lowest grade first. At their target prices the goods cost half
//!   again the permit, so the first floor is about 1,000 in all and the kth
//!   about 1,000 × 2^(k−1). Both parts are sinks.
//! - It goes up over [`build_secs`]: 15 minutes for the first, 30 for the
//!   second, 15·k for the kth. Nothing ticks: a placement upstairs checks
//!   the plot's `floor_ready_at`.
//! - Storeys are numbered from the ground: 0 is the ground floor, k the
//!   kth floor built. A station upstairs keeps its real position inside the
//!   footprint, so reaching it is the same 2D test as on the ground. Fire
//!   can't go up (`ground_only` in `stations.toml`: the kiln and the forge).

/// The first floor's permit, in Strands, before `floor_permit_scale`
pub const PERMIT_BASE: i64 = 400;

/// How long the first floor takes to go up, in seconds; the kth takes k
/// times as long
pub const BUILD_SECS: i64 = 15 * 60;

/// The goods the first floor takes: planks, bricks and nails, 600 at their
/// target prices
const FIRST: [(&str, i64); 3] = [("plank", 30), ("brick", 45), ("iron_nails", 15)];

/// The goods the second takes, with ingots for the frame: 1,200 at their
/// target prices. Each floor after takes twice the one before.
const SECOND: [(&str, i64); 4] = [("plank", 40), ("brick", 70), ("iron_nails", 15), ("iron_ingot", 7)];

/// What the `k`th floor (from 1) costs to build
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorCost {
    /// The permit, in Strands, before `floor_permit_scale`
    pub permit: i64,
    /// The building goods: `(item type, quantity)`
    pub goods: Vec<(&'static str, i64)>,
}

/// `base` doubled `n` times, held at `i64::MAX` past it: no cap on floors,
/// but nobody will ever hold the price of the 60th
fn doubled(base: i64, n: u32) -> i64 {
    match 1i64.checked_shl(n) {
        Some(times) if n < 63 => base.saturating_mul(times),
        _ => i64::MAX,
    }
}

/// What the `k`th floor costs: the permit doubles from the first, the goods
/// from the second. `k` below 1 is the first.
pub fn cost(k: i32) -> FloorCost {
    let k = k.max(1) as u32;
    let goods = if k == 1 {
        FIRST.to_vec()
    } else {
        SECOND.iter().map(|&(item, n)| (item, doubled(n, k - 2))).collect()
    };
    FloorCost { permit: doubled(PERMIT_BASE, k - 1), goods }
}

/// The `k`th floor's permit at `scale` times the base (the live
/// `floor_permit_scale`), rounded, at least 1
pub fn permit(k: i32, scale: f64) -> i64 {
    let base = cost(k).permit;
    if scale == 1.0 {
        return base;
    }
    let scaled = (base as f64 * scale.max(0.0)).round();
    if scaled >= i64::MAX as f64 {
        i64::MAX
    } else {
        (scaled as i64).max(1)
    }
}

/// How long the `k`th floor takes to go up, in seconds of game time
pub fn build_secs(k: i32) -> i64 {
    BUILD_SECS.saturating_mul(k.max(1) as i64)
}

/// A storey's name, for what's said to players: "the ground floor", "the
/// first floor", "floor 12"
pub fn storey_name(floor: i32) -> String {
    const NAMES: [&str; 10] = [
        "the ground floor",
        "the first floor",
        "the second floor",
        "the third floor",
        "the fourth floor",
        "the fifth floor",
        "the sixth floor",
        "the seventh floor",
        "the eighth floor",
        "the ninth floor",
    ];
    match usize::try_from(floor).ok().and_then(|k| NAMES.get(k)) {
        Some(name) => (*name).to_string(),
        None => format!("floor {floor}"),
    }
}

/// [`storey_name`] to start a sentence with: "The first floor"
pub fn storey_title(floor: i32) -> String {
    let name = storey_name(floor);
    let mut chars = name.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The target prices §5.4 prices them at (plank 5, brick 8, nails 6,
    /// ingot 50)
    fn at_targets(c: &FloorCost) -> i64 {
        c.goods
            .iter()
            .map(|&(item, n)| {
                n * match item {
                    "plank" => 5,
                    "brick" => 8,
                    "iron_nails" => 6,
                    "iron_ingot" => 50,
                    other => panic!("{other} has no target here"),
                }
            })
            .sum()
    }

    #[test]
    fn each_floor_costs_twice_the_last() {
        // §5.4's table: 1,000, 2,000, 4,000 and on, a permit and goods
        let all_in: Vec<(i64, i64)> = (1..=6).map(|k| (cost(k).permit, at_targets(&cost(k)))).collect();
        assert_eq!(
            all_in,
            vec![(400, 600), (800, 1200), (1600, 2400), (3200, 4800), (6400, 9600), (12800, 19200)]
        );
        assert_eq!(cost(3).goods, vec![("plank", 80), ("brick", 140), ("iron_nails", 30), ("iron_ingot", 14)]);
        let five: i64 = (1..=5).map(|k| cost(k).permit + at_targets(&cost(k))).sum();
        assert_eq!(five, 31_000, "five floors, 24 indoors on an urban lot");
    }

    #[test]
    fn there_is_no_cap_only_the_price() {
        assert_eq!(cost(64).permit, i64::MAX);
        assert_eq!(cost(200).goods[0], ("plank", i64::MAX));
        assert_eq!(cost(0), cost(1));
    }

    #[test]
    fn the_permit_scales_and_the_build_takes_longer_each_floor() {
        assert_eq!(permit(1, 1.0), 400);
        assert_eq!(permit(2, 1.5), 1200);
        assert_eq!(permit(1, 0.0001), 1, "never free");
        assert_eq!(permit(70, 2.0), i64::MAX);
        assert_eq!((build_secs(1), build_secs(2), build_secs(4)), (900, 1800, 3600));
    }

    #[test]
    fn storeys_have_names() {
        assert_eq!(storey_name(0), "the ground floor");
        assert_eq!(storey_name(2), "the second floor");
        assert_eq!(storey_name(12), "floor 12");
        assert_eq!(storey_title(1), "The first floor");
        assert_eq!(storey_title(12), "Floor 12");
    }
}
