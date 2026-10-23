//! Weather (Plots v2 §5.2, V3.2): a station's condition, and what the weather
//! does to it outdoors.
//!
//! - A station's condition runs from [`FULL`] down to 0. Indoors it doesn't
//!   wear. Outdoors on a claimed plot it wears [`WEAR_CLAIMED`] a game day,
//!   and on public land (a station off any plot, D10) [`WEAR_PUBLIC`]. Both
//!   are live tunables (`weather_wear_claimed`, `weather_wear_public`).
//! - Below [`NEEDS_REPAIR`] it stops working on its own; at 0 it's out of use,
//!   by hand too. A private station stays at 0 until it's repaired; a public
//!   one falls down.
//! - An installed tool wears [`OUTDOOR_TOOL_WEAR`] times as fast outdoors
//!   (`outdoor_tool_wear`).
//! - A repair restores [`FULL`], for about a quarter of what the station's kit
//!   took ([`repair_bill`]), more the further it has fallen.

use super::DAY_LENGTH_SECS;

/// A new or repaired station's condition
pub const FULL: f64 = 100.0;

/// Outdoors on a claimed plot: a game day's wear, by default. 100 game days
/// (about 8 real days) from new to 0.
pub const WEAR_CLAIMED: f64 = 1.0;

/// Outdoors on public land: a game day's wear, by default. 25 game days
/// (about two real days) from new to the fall.
pub const WEAR_PUBLIC: f64 = 4.0;

/// How much faster an installed tool wears outdoors, by default
pub const OUTDOOR_TOOL_WEAR: f64 = 1.5;

/// Below this a station stops working on its own ("needs repair")
pub const NEEDS_REPAIR: i32 = 25;

/// The share of the kit's goods a full repair, from 0, takes
pub const REPAIR_SHARE: f64 = 0.25;

/// A condition as it's shown and judged: whole, rounded up, so a station
/// reads 0 only once it has truly worn out
pub fn shown(condition: f64) -> i32 {
    condition.clamp(0.0, FULL).ceil() as i32
}

/// Below [`NEEDS_REPAIR`]: automation pauses
pub fn needs_repair(condition: f64) -> bool {
    shown(condition) < NEEDS_REPAIR
}

/// Worn out: out of use, by hand too
pub fn worn_out(condition: f64) -> bool {
    shown(condition) == 0
}

/// Game days in `real_secs` of wall clock, at `scale` times game time
/// (`SIM_TICK_SCALE`)
pub fn game_days(real_secs: f64, scale: f64) -> f64 {
    real_secs.max(0.0) * scale / DAY_LENGTH_SECS as f64
}

/// The condition after `days` game days of wear at `per_day`
pub fn worn(condition: f64, per_day: f64, days: f64) -> f64 {
    (condition - per_day * days).clamp(0.0, FULL)
}

/// What repairing a station at `condition` takes of a kit input the kit's
/// recipe needs `quantity` of: a quarter of it for a full repair from 0, less
/// for less wear, rounded up so any wear costs at least one
pub fn repair_bill(quantity: i32, condition: f64) -> i32 {
    let damage = (FULL - condition.clamp(0.0, FULL)) / FULL;
    if damage <= 0.0 {
        return 0;
    }
    (quantity as f64 * REPAIR_SHARE * damage - 1e-9).ceil().max(1.0) as i32
}

/// A tool's wear on one use outdoors, at `factor` times the indoor
/// [`WEAR_PER_USE`](super::tools::WEAR_PER_USE). Durability is whole, so the
/// fraction is dithered: `roll` (uniform in 0 to 1) decides whether it rounds
/// up, so many uses wear `factor` times as much on average.
pub fn tool_wear(factor: f64, roll: f64) -> i32 {
    let wear = super::tools::WEAR_PER_USE as f64 * factor.max(0.0);
    let whole = wear.floor();
    (whole + if roll < wear - whole { 1.0 } else { 0.0 }) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claimed_station_takes_a_hundred_game_days_to_wear_out() {
        let days = game_days(100.0 * DAY_LENGTH_SECS as f64, 1.0);
        assert_eq!(worn(FULL, WEAR_CLAIMED, days), 0.0);
        assert_eq!(worn(FULL, WEAR_CLAIMED, 75.0), 25.0);
        assert_eq!(worn(FULL, WEAR_PUBLIC, 25.0), 0.0);
        assert_eq!(game_days(DAY_LENGTH_SECS as f64, 10.0), 10.0, "a balance run's scale counts");
    }

    #[test]
    fn a_condition_shows_whole_and_rounds_up() {
        assert_eq!(shown(0.01), 1);
        assert!(!worn_out(0.01));
        assert!(worn_out(0.0));
        assert!(needs_repair(24.0));
        assert!(!needs_repair(24.5), "shows 25");
    }

    #[test]
    fn a_repair_takes_a_quarter_from_nothing_and_less_for_less() {
        assert_eq!(repair_bill(12, 0.0), 3);
        assert_eq!(repair_bill(10, 0.0), 3, "2.5 rounds up");
        assert_eq!(repair_bill(12, 50.0), 2, "1.5 rounds up");
        assert_eq!(repair_bill(12, 99.0), 1, "any wear costs one");
        assert_eq!(repair_bill(12, FULL), 0);
    }

    #[test]
    fn outdoor_tool_wear_averages_its_factor() {
        assert_eq!(tool_wear(1.0, 0.99), 1);
        assert_eq!(tool_wear(2.0, 0.0), 2);
        assert_eq!(tool_wear(1.5, 0.49), 2);
        assert_eq!(tool_wear(1.5, 0.5), 1);
        let total: i32 = (0..1000).map(|k| tool_wear(1.5, k as f64 / 1000.0)).sum();
        assert_eq!(total, 1500);
    }
}
