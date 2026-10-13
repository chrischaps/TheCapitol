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
    fn test_world_scale_migration_matches_zone_radii() {
        let migration = include_str!("../../migrations/20260212_001_world_scale.sql");
        let mut checked = 0;
        for line in migration.lines().filter(|l| l.starts_with("UPDATE zones SET")) {
            let number = |key: &str| -> f64 {
                let rest = &line[line.find(key).unwrap() + key.len()..];
                rest.trim_start_matches([' ', '='])
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap()
            };
            let id = line.rsplit('\'').nth(1).unwrap();
            let zone = Zone::from_str(id).unwrap();
            assert_eq!((number("inner_radius"), number("outer_radius")), zone.radii(), "{id}");
            checked += 1;
        }
        assert_eq!(checked, Zone::ALL.len());
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
