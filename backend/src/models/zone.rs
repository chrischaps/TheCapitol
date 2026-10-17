use serde::{Deserialize, Serialize};

/// Zones are concentric rings around the Capitol; the rules live in capitol-protocol.
pub use capitol_protocol::rules::Zone;
pub use capitol_protocol::rest::ZoneInfo;

/// Zone transition event data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneTransition {
    pub from_zone: String,
    pub to_zone: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rules::{CENTER_X, CENTER_Y, RURAL_OUTER_RADIUS, WILDERNESS_OUTER_RADIUS};

    /// The middle of a zone's ring, on each axis.
    fn mid_ring(zone: Zone) -> [(f64, f64); 4] {
        let (inner, outer) = zone.radii();
        let r = (inner + outer) / 2.0;
        [(CENTER_X + r, CENTER_Y), (CENTER_X - r, CENTER_Y), (CENTER_X, CENTER_Y + r), (CENTER_X, CENTER_Y - r)]
    }

    #[test]
    fn test_zone_from_position_mid_ring() {
        assert_eq!(Zone::from_position(CENTER_X, CENTER_Y), Zone::Capitol);
        for zone in Zone::ALL {
            for (x, y) in mid_ring(zone) {
                assert_eq!(Zone::from_position(x, y), zone, "({x}, {y})");
            }
        }
    }

    #[test]
    fn test_zone_from_position_wilderness() {
        // Wilderness: from the rural edge out, and past the world disc.
        assert_eq!(Zone::from_position(CENTER_X + RURAL_OUTER_RADIUS, CENTER_Y), Zone::Wilderness);
        assert_eq!(Zone::from_position(CENTER_X + WILDERNESS_OUTER_RADIUS + 1.0, CENTER_Y), Zone::Wilderness);
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
