use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::fmt;
use utoipa::ToSchema;

/// Zone definitions for the world
/// Zones are concentric rings around the Capitol center at (4000, 4000)
/// World size: 8000x8000 (4x scale from original)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Zone {
    Capitol,
    TradeDistrict,
    GuildDistrict,
    Urban,
    Suburban,
    Rural,
    Wilderness,
}

impl Zone {
    /// World center coordinates
    pub const CENTER_X: f64 = 4000.0;
    pub const CENTER_Y: f64 = 4000.0;
    pub const WORLD_SIZE: f64 = 8000.0;

    /// Determine which zone a position falls into based on distance from center
    pub fn from_position(x: f64, y: f64) -> Self {
        let dx = x - Self::CENTER_X;
        let dy = y - Self::CENTER_Y;
        let distance = (dx * dx + dy * dy).sqrt();

        // Radii scaled 4x for 8000x8000 world
        match distance {
            d if d < 200.0 => Zone::Capitol,
            d if d < 600.0 => Zone::TradeDistrict,
            d if d < 1000.0 => Zone::GuildDistrict,
            d if d < 1600.0 => Zone::Urban,
            d if d < 2400.0 => Zone::Suburban,
            d if d < 3400.0 => Zone::Rural,
            _ => Zone::Wilderness,
        }
    }

    /// Get the zone ID as stored in the database
    pub fn as_str(&self) -> &'static str {
        match self {
            Zone::Capitol => "capitol",
            Zone::TradeDistrict => "trade_district",
            Zone::GuildDistrict => "guild_district",
            Zone::Urban => "urban",
            Zone::Suburban => "suburban",
            Zone::Rural => "rural",
            Zone::Wilderness => "wilderness",
        }
    }

    /// Parse zone from database string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "capitol" => Some(Zone::Capitol),
            "trade_district" => Some(Zone::TradeDistrict),
            "guild_district" => Some(Zone::GuildDistrict),
            "urban" => Some(Zone::Urban),
            "suburban" => Some(Zone::Suburban),
            "rural" => Some(Zone::Rural),
            "wilderness" => Some(Zone::Wilderness),
            _ => None,
        }
    }

    /// Get the display name of the zone
    pub fn display_name(&self) -> &'static str {
        match self {
            Zone::Capitol => "The Capitol",
            Zone::TradeDistrict => "Trade District",
            Zone::GuildDistrict => "Guild District",
            Zone::Urban => "Urban Zone",
            Zone::Suburban => "Suburban Zone",
            Zone::Rural => "Rural Zone",
            Zone::Wilderness => "Wilderness",
        }
    }

    /// Check if this zone supports private plots
    pub fn has_plots(&self) -> bool {
        matches!(
            self,
            Zone::GuildDistrict | Zone::Urban | Zone::Suburban | Zone::Rural
        )
    }

    /// Get the ring order (0 = center, higher = outer)
    pub fn ring_order(&self) -> u8 {
        match self {
            Zone::Capitol => 0,
            Zone::TradeDistrict => 1,
            Zone::GuildDistrict => 2,
            Zone::Urban => 3,
            Zone::Suburban => 4,
            Zone::Rural => 5,
            Zone::Wilderness => 6,
        }
    }

    /// Get the inner and outer radius of this zone (scaled 4x for 8000x8000 world)
    pub fn radii(&self) -> (f64, f64) {
        match self {
            Zone::Capitol => (0.0, 200.0),
            Zone::TradeDistrict => (200.0, 600.0),
            Zone::GuildDistrict => (600.0, 1000.0),
            Zone::Urban => (1000.0, 1600.0),
            Zone::Suburban => (1600.0, 2400.0),
            Zone::Rural => (2400.0, 3400.0),
            Zone::Wilderness => (3400.0, 4000.0),
        }
    }

    /// Get all zones in order from center to edge
    pub fn all() -> &'static [Zone] {
        &[
            Zone::Capitol,
            Zone::TradeDistrict,
            Zone::GuildDistrict,
            Zone::Urban,
            Zone::Suburban,
            Zone::Rural,
            Zone::Wilderness,
        ]
    }
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Zone info from the database
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct ZoneInfo {
    /// Zone identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Ring order from center (0 = Capitol)
    pub ring_order: i32,
    /// Inner boundary radius
    pub inner_radius: f32,
    /// Outer boundary radius
    pub outer_radius: f32,
    /// Whether this zone has purchasable plots
    pub has_plots: bool,
    /// Plot size category for this zone
    pub plot_size_category: Option<String>,
    /// Cost to purchase a deed in this zone
    pub deed_cost: Option<i32>,
    /// Zone description
    pub description: Option<String>,
}

/// Zone transition event data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneTransition {
    pub from_zone: String,
    pub to_zone: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zone_from_position_center() {
        // Center is at (4000, 4000), Capitol radius is 200
        assert_eq!(Zone::from_position(4000.0, 4000.0), Zone::Capitol);
        assert_eq!(Zone::from_position(4100.0, 4000.0), Zone::Capitol);
    }

    #[test]
    fn test_zone_from_position_trade_district() {
        // Trade District: 200-600 from center
        assert_eq!(Zone::from_position(4300.0, 4000.0), Zone::TradeDistrict);
        assert_eq!(Zone::from_position(4000.0, 4400.0), Zone::TradeDistrict);
    }

    #[test]
    fn test_zone_from_position_wilderness() {
        // Wilderness: beyond 3400 from center
        assert_eq!(Zone::from_position(7600.0, 4000.0), Zone::Wilderness);
        assert_eq!(Zone::from_position(400.0, 4000.0), Zone::Wilderness);
    }

    #[test]
    fn test_zone_has_plots() {
        assert!(!Zone::Capitol.has_plots());
        assert!(!Zone::TradeDistrict.has_plots());
        assert!(Zone::GuildDistrict.has_plots());
        assert!(Zone::Urban.has_plots());
        assert!(Zone::Suburban.has_plots());
        assert!(Zone::Rural.has_plots());
        assert!(!Zone::Wilderness.has_plots());
    }
}
