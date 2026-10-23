//! Tunables (the Observatory's D6.1): the economy's rates and fees, changed live.
//!
//! [`Tunables`] is every knob's value. The server holds one, persists what
//! differs from the defaults, and charges by it. Each knob is declared once in
//! [`KNOBS`]: its key, unit, range, default and the console chart it's watched
//! on. Two of them are families with one knob per zone or resource:
//! `deed_price.<zone>` and `yield.<resource>`.
//!
//! **Knobs only for what the server alone decides.** Anything a client predicts
//! or draws to scale stays a constant in [`crate::rules`]. The fees a client
//! shows the player are [`Fees`]. The server sends them on `auth_ok` and again
//! as `fees_changed` whenever an admin moves one, so a preview is always the
//! server's answer.
//!
//! The defaults are the constants they replaced: [`crate::rules::EXCHANGE_FEE_RATE`],
//! [`crate::rules::market::LISTING_FEE_PERCENT`] and
//! [`crate::rules::bazaar::RENT_PER_DAY`], a 2% weekly property tax, and 1×
//! for every multiplier.
//!
//! Since D2.5 the watches' thresholds are knobs too ([`WatchThresholds`]):
//! when the Gini, a price off its target, the tax an hour and the tick's p95
//! warn (`crate::watch`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::rules::bazaar::RENT_PER_DAY;
use crate::rules::market::LISTING_FEE_PERCENT;
use crate::rules::weather;
use crate::rules::EXCHANGE_FEE_RATE;

/// The weekly property tax by default, as a fraction of the deed (2%)
pub const PROPERTY_TAX_RATE: f64 = 0.02;
/// The Gini over which wealth is pooling, by default
pub const WATCH_GINI: f64 = 0.7;
/// How far an item's price may stray from its target by default: half
pub const WATCH_OFF_TARGET: f64 = 0.5;
/// The property tax an hour under which it isn't biting, by default (#335)
pub const WATCH_TAX_PER_HOUR: f64 = 1.0;
/// The tick's p95, in ticks, over which it's over budget, by default: one
pub const WATCH_TICK_P95: f64 = 1.0;

// ---------------------------------------------------------------------------
// Fees: what a client quotes
// ---------------------------------------------------------------------------

/// The fees a client shows the player before they pay one. The server charges
/// by these exactly, so each method here is what the matching route charges.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct Fees {
    /// Fee on Strand/Fiber exchange, as a fraction (0.005 is 0.5%)
    pub exchange_fee_rate: f64,
    /// The market's listing fee, in percent of the order value (to 0.01%)
    pub listing_fee_percent: f64,
    /// A bazaar stall's rent, in Strands a day
    pub stall_rent_per_day: i64,
}

impl Default for Fees {
    fn default() -> Self {
        Fees {
            exchange_fee_rate: EXCHANGE_FEE_RATE,
            listing_fee_percent: LISTING_FEE_PERCENT as f64,
            stall_rent_per_day: RENT_PER_DAY,
        }
    }
}

impl Fees {
    /// What depositing `fiber` units at the exchange gives: `(fee, strands)`,
    /// the fee rounded down. `POST /exchange/deposit` charges exactly this.
    pub fn deposit_quote(&self, fiber: i64) -> (i64, i64) {
        let fee = ((fiber as f64) * self.exchange_fee_rate).floor() as i64;
        (fee, fiber - fee)
    }

    /// What withdrawing `fiber` units costs: `(strands, fee)`, where `strands`
    /// is `ceil(fiber / (1 - rate))` and includes the fee.
    /// `POST /exchange/withdraw` charges exactly this.
    pub fn withdraw_quote(&self, fiber: i64) -> (i64, i64) {
        let strands = ((fiber as f64) / (1.0 - self.exchange_fee_rate)).ceil() as i64;
        (strands, strands - fiber)
    }

    /// The listing fee in basis points (hundredths of a percent), so the fee
    /// itself is whole-number arithmetic
    fn listing_basis_points(&self) -> i64 {
        (self.listing_fee_percent * 100.0).round().max(0.0) as i64
    }

    /// The listing fee on an order worth `value`, rounded up, so every order
    /// pays at least 1 Strand while the fee isn't 0. Within the market's
    /// bounds `value` is at most 10¹³, and the fee at most 10%, so it never
    /// overflows.
    pub fn listing_fee(&self, value: i64) -> i64 {
        (value.max(0) * self.listing_basis_points() + 9_999) / 10_000
    }

    /// What placing a buy debits: the escrow, `price × quantity`, plus the fee
    pub fn buy_debit(&self, price: i64, quantity: i32) -> i64 {
        let value = crate::rules::market::order_value(price, quantity);
        value + self.listing_fee(value)
    }

    /// What `days` of a stall's rent cost
    pub fn rent(&self, days: i64) -> i64 {
        days * self.stall_rent_per_day
    }

    /// The exchange fee for a label: "0.5%"
    pub fn exchange_fee_label(&self) -> String {
        Unit::Fraction.format(self.exchange_fee_rate)
    }

    /// The listing fee for a label: "1%"
    pub fn listing_fee_label(&self) -> String {
        Unit::Percent.format(self.listing_fee_percent)
    }
}

// ---------------------------------------------------------------------------
// The knobs
// ---------------------------------------------------------------------------

/// What a knob's value counts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// A fraction, shown as a percent: 0.02 is "2%"
    Fraction,
    /// Already a percent: 1 is "1%"
    Percent,
    /// Whole Strands
    Strands,
    /// Times its content value, or the thing it's measured against: 1.5 is "×1.5"
    Multiplier,
    /// A plain number, such as a Gini: 0.7 is "0.7"
    Ratio,
}

impl Unit {
    /// `value` as the console and an annotation say it
    pub fn format(self, value: f64) -> String {
        match self {
            Unit::Fraction => format!("{}%", trim(value * 100.0)),
            Unit::Percent => format!("{}%", trim(value)),
            Unit::Strands => format!("{} Strands", trim(value)),
            Unit::Multiplier => format!("×{}", trim(value)),
            Unit::Ratio => trim(value),
        }
    }
}

/// A number with no trailing zeros, to four places: 0.5, 2, 0.0125
fn trim(value: f64) -> String {
    let s = format!("{value:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

/// One knob as declared: everything about it but its value
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knob {
    /// Its key, or for a family the prefix before the dot: `yield` for `yield.grass`
    pub key: &'static str,
    /// What the console calls it
    pub label: &'static str,
    pub unit: Unit,
    pub min: f64,
    pub max: f64,
    /// The smallest change that means anything: values are rounded to it
    pub step: f64,
    pub default: f64,
    /// The console chart that shows its effect (D2), so the tuning tab can
    /// put the knob beside it
    pub chart: &'static str,
    /// One knob per zone or resource, keyed `<key>.<id>`
    pub family: Option<Family>,
}

/// What a family's knobs are keyed by
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// A zone with deeds (`zones.id`)
    Zone,
    /// A resource type (`resource_types.id`)
    Resource,
}

/// Every knob, in the order the console lists them
pub const KNOBS: &[Knob] = &[
    Knob {
        key: "exchange_fee_rate",
        label: "Exchange fee",
        unit: Unit::Fraction,
        min: 0.0,
        max: 0.1,
        step: 0.0001,
        default: EXCHANGE_FEE_RATE,
        chart: "sinks",
        family: None,
    },
    Knob {
        key: "listing_fee_percent",
        label: "Listing fee",
        unit: Unit::Percent,
        min: 0.0,
        max: 10.0,
        step: 0.01,
        default: LISTING_FEE_PERCENT as f64,
        chart: "sinks",
        family: None,
    },
    Knob {
        key: "stall_rent_per_day",
        label: "Stall rent a day",
        unit: Unit::Strands,
        min: 0.0,
        max: 1000.0,
        step: 1.0,
        default: RENT_PER_DAY as f64,
        chart: "sinks",
        family: None,
    },
    Knob {
        key: "property_tax_rate",
        label: "Property tax a week",
        unit: Unit::Fraction,
        min: 0.0,
        max: 0.5,
        step: 0.0001,
        default: PROPERTY_TAX_RATE,
        chart: "sinks",
        family: None,
    },
    Knob {
        key: "node_regen_rate",
        label: "Node regrowth speed",
        unit: Unit::Multiplier,
        min: 0.1,
        max: 10.0,
        step: 0.01,
        default: 1.0,
        chart: "production",
        family: None,
    },
    Knob {
        key: "deed_price",
        label: "Deed price",
        unit: Unit::Multiplier,
        min: 0.1,
        max: 10.0,
        step: 0.01,
        default: 1.0,
        chart: "sinks",
        family: Some(Family::Zone),
    },
    Knob {
        key: "yield",
        label: "Gather yield",
        unit: Unit::Multiplier,
        min: 0.1,
        max: 10.0,
        step: 0.01,
        default: 1.0,
        chart: "production",
        family: Some(Family::Resource),
    },
    Knob {
        key: "weather_wear_claimed",
        label: "Weather: wear a game day, outdoors on a plot",
        unit: Unit::Ratio,
        min: 0.0,
        max: 100.0,
        step: 0.01,
        default: weather::WEAR_CLAIMED,
        chart: "production",
        family: None,
    },
    Knob {
        key: "weather_wear_public",
        label: "Weather: wear a game day, on public land",
        unit: Unit::Ratio,
        min: 0.0,
        max: 100.0,
        step: 0.01,
        default: weather::WEAR_PUBLIC,
        chart: "production",
        family: None,
    },
    Knob {
        key: "outdoor_tool_wear",
        label: "Weather: a station's tool wear outdoors",
        unit: Unit::Multiplier,
        min: 1.0,
        max: 10.0,
        step: 0.01,
        default: weather::OUTDOOR_TOOL_WEAR,
        chart: "production",
        family: None,
    },
    Knob {
        key: "watch_gini",
        label: "Watch: wealth's Gini over",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        step: 0.01,
        default: WATCH_GINI,
        chart: "distribution",
        family: None,
    },
    Knob {
        key: "watch_off_target",
        label: "Watch: a price off its target by",
        unit: Unit::Fraction,
        min: 0.05,
        max: 10.0,
        step: 0.01,
        default: WATCH_OFF_TARGET,
        chart: "prices",
        family: None,
    },
    Knob {
        key: "watch_tax_per_hour",
        label: "Watch: property tax an hour under",
        unit: Unit::Strands,
        min: 0.0,
        max: 1_000_000.0,
        step: 1.0,
        default: WATCH_TAX_PER_HOUR,
        chart: "sinks",
        family: None,
    },
    Knob {
        key: "watch_tick_p95",
        label: "Watch: the tick's p95 over, in ticks",
        unit: Unit::Multiplier,
        min: 0.01,
        max: 10.0,
        step: 0.01,
        default: WATCH_TICK_P95,
        chart: "tick",
        family: None,
    },
];

impl Knob {
    /// The knob a key names, and for a family the id after the dot.
    /// `None` for a key no knob has.
    pub fn find(key: &str) -> Option<(&'static Knob, Option<&str>)> {
        match key.split_once('.') {
            Some((prefix, id)) if !id.is_empty() => KNOBS
                .iter()
                .find(|k| k.family.is_some() && k.key == prefix)
                .map(|k| (k, Some(id))),
            Some(_) => None,
            None => KNOBS.iter().find(|k| k.family.is_none() && k.key == key).map(|k| (k, None)),
        }
    }

    /// `value` rounded to the knob's step, or why it can't be set
    pub fn check(&self, value: f64) -> Result<f64, TunableError> {
        if !value.is_finite() {
            return Err(TunableError::NotANumber);
        }
        // Steps per unit are whole, so dividing by them lands on the nearest
        // double to the decimal: 0.3, never 0.30000000000000004
        let per_unit = (1.0 / self.step).round();
        let rounded = (value * per_unit).round() / per_unit;
        // Rounding to the step can land a hair outside the range's own float
        if rounded < self.min - self.step / 2.0 || rounded > self.max + self.step / 2.0 {
            return Err(TunableError::OutOfRange { min: self.min, max: self.max });
        }
        Ok(rounded.clamp(self.min, self.max))
    }
}

/// Why a knob wasn't set
#[derive(Debug, Clone, PartialEq)]
pub enum TunableError {
    /// No knob has that key, or a family's id isn't one the server knows
    UnknownKey,
    /// Infinite or NaN
    NotANumber,
    OutOfRange { min: f64, max: f64 },
}

// ---------------------------------------------------------------------------
// Tunables: every knob's value
// ---------------------------------------------------------------------------

/// Every knob's value. `Default` is the game as it was before D6.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct Tunables {
    pub fees: Fees,
    /// The weekly property tax, as a fraction of the deed
    pub property_tax_rate: f64,
    /// How fast a spent node grows back: 2 is in half the time
    pub node_regen_rate: f64,
    /// Each zone's deed price, times its `zones.deed_cost`. A zone not here is 1.
    pub deed_price: BTreeMap<String, f64>,
    /// Each resource's gather yield, times its content's. One not here is 1.
    #[serde(rename = "yield")]
    pub yields: BTreeMap<String, f64>,
    /// When the watches warn (D2.5)
    pub watches: WatchThresholds,
    /// Weather (V3.2): an outdoor station's wear a game day on a claimed plot
    pub weather_wear_claimed: f64,
    /// And on public land, where it falls down at 0
    pub weather_wear_public: f64,
    /// How much faster a station's installed tool wears outdoors
    pub outdoor_tool_wear: f64,
}

impl Default for Tunables {
    fn default() -> Self {
        Tunables {
            fees: Fees::default(),
            property_tax_rate: PROPERTY_TAX_RATE,
            node_regen_rate: 1.0,
            deed_price: BTreeMap::new(),
            yields: BTreeMap::new(),
            watches: WatchThresholds::default(),
            weather_wear_claimed: weather::WEAR_CLAIMED,
            weather_wear_public: weather::WEAR_PUBLIC,
            outdoor_tool_wear: weather::OUTDOOR_TOOL_WEAR,
        }
    }
}

/// The thresholds the watches warn at (D2.5, `crate::watch`). Drift has
/// none: any is too much.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct WatchThresholds {
    /// Wealth's Gini, over people and citizens, over which it's pooling
    pub gini: f64,
    /// How far a game day's price may stray from its target, as a fraction
    /// of the target either way
    pub off_target: f64,
    /// Strands of property tax an hour under which it isn't biting. 0 never
    /// warns.
    pub tax_per_hour: f64,
    /// The tick's p95, in ticks: 1 is its period, the budget
    pub tick_p95: f64,
}

impl Default for WatchThresholds {
    fn default() -> Self {
        WatchThresholds {
            gini: WATCH_GINI,
            off_target: WATCH_OFF_TARGET,
            tax_per_hour: WATCH_TAX_PER_HOUR,
            tick_p95: WATCH_TICK_P95,
        }
    }
}

impl Tunables {
    /// A zone's deed price multiplier
    pub fn deed_multiplier(&self, zone: &str) -> f64 {
        self.deed_price.get(zone).copied().unwrap_or(1.0)
    }

    /// What a deed of `base` Strands in `zone` costs now, rounded, at least 1
    pub fn deed_cost(&self, zone: &str, base: i32) -> i32 {
        let multiplier = self.deed_multiplier(zone);
        if multiplier == 1.0 {
            return base;
        }
        ((base as f64 * multiplier).round() as i32).max(1)
    }

    /// A resource's yield multiplier
    pub fn yield_multiplier(&self, resource: &str) -> f64 {
        self.yields.get(resource).copied().unwrap_or(1.0)
    }

    /// The ticks a spent node takes to grow back, from its content's `ticks`
    pub fn regeneration_ticks(&self, ticks: i32) -> u64 {
        (ticks.max(0) as f64 / self.node_regen_rate).round().max(1.0) as u64
    }

    /// The value under `key`, or `None` for a key no knob has. A family's
    /// knob reads its default for any id.
    pub fn get(&self, key: &str) -> Option<f64> {
        let (knob, id) = Knob::find(key)?;
        Some(match (knob.key, id) {
            ("exchange_fee_rate", None) => self.fees.exchange_fee_rate,
            ("listing_fee_percent", None) => self.fees.listing_fee_percent,
            ("stall_rent_per_day", None) => self.fees.stall_rent_per_day as f64,
            ("property_tax_rate", None) => self.property_tax_rate,
            ("node_regen_rate", None) => self.node_regen_rate,
            ("watch_gini", None) => self.watches.gini,
            ("watch_off_target", None) => self.watches.off_target,
            ("watch_tax_per_hour", None) => self.watches.tax_per_hour,
            ("watch_tick_p95", None) => self.watches.tick_p95,
            ("weather_wear_claimed", None) => self.weather_wear_claimed,
            ("weather_wear_public", None) => self.weather_wear_public,
            ("outdoor_tool_wear", None) => self.outdoor_tool_wear,
            ("deed_price", Some(zone)) => self.deed_multiplier(zone),
            ("yield", Some(resource)) => self.yield_multiplier(resource),
            _ => return None,
        })
    }

    /// Set `key` to `value`, rounded to its step. Returns the value set.
    /// `known` says whether a family's id names a zone or resource the server
    /// has. A family's knob set back to 1 is dropped from its map.
    pub fn set(&mut self, key: &str, value: f64, known: impl Fn(Family, &str) -> bool) -> Result<f64, TunableError> {
        let (knob, id) = Knob::find(key).ok_or(TunableError::UnknownKey)?;
        if let (Some(family), Some(id)) = (knob.family, id) {
            if !known(family, id) {
                return Err(TunableError::UnknownKey);
            }
        }
        let value = knob.check(value)?;
        let family_map = |map: &mut BTreeMap<String, f64>, id: &str| {
            if value == knob.default {
                map.remove(id);
            } else {
                map.insert(id.to_string(), value);
            }
        };
        match (knob.key, id) {
            ("exchange_fee_rate", None) => self.fees.exchange_fee_rate = value,
            ("listing_fee_percent", None) => self.fees.listing_fee_percent = value,
            ("stall_rent_per_day", None) => self.fees.stall_rent_per_day = value as i64,
            ("property_tax_rate", None) => self.property_tax_rate = value,
            ("node_regen_rate", None) => self.node_regen_rate = value,
            ("watch_gini", None) => self.watches.gini = value,
            ("watch_off_target", None) => self.watches.off_target = value,
            ("watch_tax_per_hour", None) => self.watches.tax_per_hour = value,
            ("watch_tick_p95", None) => self.watches.tick_p95 = value,
            ("weather_wear_claimed", None) => self.weather_wear_claimed = value,
            ("weather_wear_public", None) => self.weather_wear_public = value,
            ("outdoor_tool_wear", None) => self.outdoor_tool_wear = value,
            ("deed_price", Some(zone)) => family_map(&mut self.deed_price, zone),
            ("yield", Some(resource)) => family_map(&mut self.yields, resource),
            _ => return Err(TunableError::UnknownKey),
        }
        Ok(value)
    }

    /// Every knob's key and value that differs from its default, for
    /// persisting and for a preset
    pub fn overrides(&self) -> BTreeMap<String, f64> {
        let mut out = BTreeMap::new();
        for knob in KNOBS.iter().filter(|k| k.family.is_none()) {
            let value = self.get(knob.key).unwrap_or(knob.default);
            if value != knob.default {
                out.insert(knob.key.to_string(), value);
            }
        }
        for (zone, value) in &self.deed_price {
            out.insert(format!("deed_price.{zone}"), *value);
        }
        for (resource, value) in &self.yields {
            out.insert(format!("yield.{resource}"), *value);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// GET/PUT /admin/tunables
// ---------------------------------------------------------------------------

/// One knob with its value, as `GET /admin/tunables` lists it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TunableValue {
    /// `property_tax_rate`, or for a family `yield.grass`
    pub key: String,
    /// "Property tax a week", or for a family "Gather yield: grass"
    pub label: String,
    pub unit: Unit,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
    pub value: f64,
    /// The console chart it's watched on
    pub chart: String,
    /// For a multiplier over a price, the price it multiplies, in Strands:
    /// a `deed_price.<zone>` knob's zone's `deed_cost` (#431)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_strands: Option<i64>,
}

/// `GET /admin/tunables`, and `PUT`'s answer: every knob, one per zone and
/// resource for the families
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TunablesResponse {
    pub knobs: Vec<TunableValue>,
}

/// `PUT /admin/tunables`: the knobs to change, by key. All of them are checked
/// before any is set, so a request with one bad value changes nothing. Setting
/// a knob to its default resets it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct TunablesUpdate {
    pub values: BTreeMap<String, f64>,
}

impl TunablesResponse {
    /// Every knob of `tunables`, with a family's knobs for each of `zones`
    /// and `resources`
    pub fn list(tunables: &Tunables, zones: &[String], resources: &[String]) -> Self {
        let mut knobs = Vec::new();
        for knob in KNOBS {
            let ids: Vec<Option<&str>> = match knob.family {
                None => vec![None],
                Some(Family::Zone) => zones.iter().map(|z| Some(z.as_str())).collect(),
                Some(Family::Resource) => resources.iter().map(|r| Some(r.as_str())).collect(),
            };
            for id in ids {
                let (key, label) = match id {
                    Some(id) => (format!("{}.{id}", knob.key), format!("{}: {id}", knob.label)),
                    None => (knob.key.to_string(), knob.label.to_string()),
                };
                knobs.push(TunableValue {
                    value: tunables.get(&key).unwrap_or(knob.default),
                    key,
                    label,
                    unit: knob.unit,
                    min: knob.min,
                    max: knob.max,
                    step: knob.step,
                    default: knob.default,
                    chart: knob.chart.to_string(),
                    base_strands: None,
                });
            }
        }
        TunablesResponse { knobs }
    }

    /// Give each zone's deed price knob the price it multiplies, from
    /// `deed_costs` (zone id to `zones.deed_cost`)
    pub fn with_deed_costs(mut self, deed_costs: &BTreeMap<String, i64>) -> Self {
        for knob in &mut self.knobs {
            if let Some(zone) = knob.key.strip_prefix("deed_price.") {
                knob.base_strands = deed_costs.get(zone).copied();
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anything(_: Family, _: &str) -> bool {
        true
    }

    #[test]
    fn the_default_fees_charge_what_the_constants_did() {
        let fees = Fees::default();
        for fiber in [0, 1, 199, 200, 201, 1000, 12_345] {
            let fee = ((fiber as f64) * EXCHANGE_FEE_RATE).floor() as i64;
            assert_eq!(fees.deposit_quote(fiber), (fee, fiber - fee));
            let strands = ((fiber as f64) / (1.0 - EXCHANGE_FEE_RATE)).ceil() as i64;
            assert_eq!(fees.withdraw_quote(fiber), (strands, strands - fiber));
        }
        for value in [0, 1, 99, 100, 101, 250, 10_000, 1_000_000_000_000] {
            assert_eq!(fees.listing_fee(value), (value * LISTING_FEE_PERCENT + 99) / 100, "{value}");
        }
        assert_eq!(fees.rent(3), 3 * RENT_PER_DAY);
        assert_eq!(fees.exchange_fee_label(), "0.5%");
        assert_eq!(fees.listing_fee_label(), "1%");
    }

    #[test]
    fn a_listing_fee_in_hundredths_of_a_percent_rounds_up() {
        let fees = Fees { listing_fee_percent: 0.5, ..Fees::default() };
        assert_eq!(fees.listing_fee(1), 1);
        assert_eq!(fees.listing_fee(200), 1);
        assert_eq!(fees.listing_fee(201), 2);
        let free = Fees { listing_fee_percent: 0.0, ..Fees::default() };
        assert_eq!(free.listing_fee(1_000), 0);
        assert_eq!(free.buy_debit(10, 3), 30);
    }

    #[test]
    fn every_knob_reads_its_default_from_the_defaults() {
        let t = Tunables::default();
        for knob in KNOBS {
            let key = if knob.family.is_some() { format!("{}.anything", knob.key) } else { knob.key.to_string() };
            assert_eq!(t.get(&key), Some(knob.default), "{key}");
            assert!(knob.min <= knob.default && knob.default <= knob.max, "{key}");
            assert_eq!(knob.check(knob.default), Ok(knob.default), "{key} on its step");
        }
        assert!(t.overrides().is_empty());
    }

    #[test]
    fn a_knob_is_set_rounded_and_read_back() {
        let mut t = Tunables::default();
        assert_eq!(t.set("property_tax_rate", 0.05, anything), Ok(0.05));
        assert_eq!(t.property_tax_rate, 0.05);
        assert_eq!(t.set("stall_rent_per_day", 12.4, anything), Ok(12.0));
        assert_eq!(t.fees.stall_rent_per_day, 12);
        assert_eq!(t.set("yield.grass", 1.5, anything), Ok(1.5));
        assert_eq!(t.yield_multiplier("grass"), 1.5);
        assert_eq!(t.yield_multiplier("clay"), 1.0);
        assert_eq!(t.set("deed_price.urban", 2.0, anything), Ok(2.0));
        assert_eq!(t.deed_cost("urban", 500), 1000);
        assert_eq!(t.deed_cost("rural", 250), 250);
        assert_eq!(t.set("watch_gini", 0.55, anything), Ok(0.55));
        assert_eq!(t.watches.gini, 0.55);
        assert_eq!(t.set("watch_gini", 0.7, anything), Ok(0.7), "back to its default");
        let overrides = t.overrides();
        assert_eq!(overrides.len(), 4, "{overrides:?}");
        assert_eq!(overrides["yield.grass"], 1.5);

        // Back to the default drops the family's entry, and the override
        assert_eq!(t.set("yield.grass", 1.0, anything), Ok(1.0));
        assert!(t.yields.is_empty());
        assert!(!t.overrides().contains_key("yield.grass"));
    }

    #[test]
    fn a_bad_key_or_value_is_refused_and_changes_nothing() {
        let mut t = Tunables::default();
        assert_eq!(t.set("property_tax_rate", 0.9, anything), Err(TunableError::OutOfRange { min: 0.0, max: 0.5 }));
        assert_eq!(t.set("property_tax_rate", -0.01, anything), Err(TunableError::OutOfRange { min: 0.0, max: 0.5 }));
        assert_eq!(t.set("property_tax_rate", f64::NAN, anything), Err(TunableError::NotANumber));
        assert_eq!(t.set("nonsense", 1.0, anything), Err(TunableError::UnknownKey));
        assert_eq!(t.set("yield", 1.0, anything), Err(TunableError::UnknownKey), "a family needs its id");
        assert_eq!(t.set("yield.", 1.0, anything), Err(TunableError::UnknownKey));
        assert_eq!(t.set("property_tax_rate.x", 1.0, anything), Err(TunableError::UnknownKey));
        assert_eq!(t.set("yield.unobtainium", 2.0, |_, id| id != "unobtainium"), Err(TunableError::UnknownKey));
        assert_eq!(t, Tunables::default());
    }

    #[test]
    fn a_value_rounds_to_its_step_as_a_person_would_write_it() {
        let mut t = Tunables::default();
        for (key, value, want) in [
            ("property_tax_rate", 0.3, 0.3),
            ("property_tax_rate", 0.07000001, 0.07),
            ("listing_fee_percent", 0.1 + 0.2, 0.3),
            ("exchange_fee_rate", 0.0123456, 0.0123),
            ("yield.grass", 1.105, 1.11),
        ] {
            assert_eq!(t.set(key, value, anything), Ok(want), "{key} {value}");
        }
    }

    #[test]
    fn regrowth_speed_divides_the_ticks() {
        let mut t = Tunables::default();
        assert_eq!(t.regeneration_ticks(600), 600);
        t.node_regen_rate = 2.0;
        assert_eq!(t.regeneration_ticks(600), 300);
        t.node_regen_rate = 0.5;
        assert_eq!(t.regeneration_ticks(600), 1200);
        assert_eq!(t.regeneration_ticks(0), 1);
    }

    #[test]
    fn units_read_as_people_say_them() {
        assert_eq!(Unit::Fraction.format(0.02), "2%");
        assert_eq!(Unit::Fraction.format(0.005), "0.5%");
        assert_eq!(Unit::Percent.format(1.0), "1%");
        assert_eq!(Unit::Strands.format(20.0), "20 Strands");
        assert_eq!(Unit::Multiplier.format(1.5), "×1.5");
        assert_eq!(Unit::Ratio.format(0.7), "0.7");
    }

    #[test]
    fn the_list_has_each_knob_and_a_family_knob_per_id() {
        let mut t = Tunables::default();
        t.set("yield.clay", 3.0, anything).unwrap();
        let zones = vec!["urban".to_string(), "rural".to_string()];
        let resources = vec!["clay".to_string(), "grass".to_string()];
        let list = TunablesResponse::list(&t, &zones, &resources);
        assert_eq!(list.knobs.len(), 12 + 2 + 2);
        let clay = list.knobs.iter().find(|k| k.key == "yield.clay").unwrap();
        assert_eq!((clay.value, clay.default, clay.label.as_str()), (3.0, 1.0, "Gather yield: clay"));
        assert!(list.knobs.iter().all(|k| k.base_strands.is_none()));
    }

    #[test]
    fn a_deed_knob_carries_the_price_it_multiplies() {
        let zones = vec!["urban".to_string(), "rural".to_string()];
        let costs = BTreeMap::from([("urban".to_string(), 1200)]);
        let list = TunablesResponse::list(&Tunables::default(), &zones, &[]).with_deed_costs(&costs);
        let base = |key: &str| list.knobs.iter().find(|k| k.key == key).unwrap().base_strands;
        assert_eq!(base("deed_price.urban"), Some(1200));
        assert_eq!(base("deed_price.rural"), None);
        assert_eq!(base("property_tax_rate"), None);
        let json = serde_json::to_string(&list).unwrap();
        assert_eq!(json.matches("base_strands").count(), 1, "{json}");
    }

    #[test]
    fn tunables_round_trip_as_json_and_fill_in_what_is_missing() {
        let mut t = Tunables::default();
        t.set("yield.clay", 3.0, anything).unwrap();
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"yield\":{\"clay\":3.0}"), "{json}");
        assert_eq!(serde_json::from_str::<Tunables>(&json).unwrap(), t);
        let partial: Tunables = serde_json::from_str(r#"{"property_tax_rate":0.1}"#).unwrap();
        assert_eq!(partial.fees, Fees::default());
        assert_eq!(partial.property_tax_rate, 0.1);
    }
}
