//! Stations in the world (P6): whether you can reach one.
//!
//! The server lets you use a station from up to its type's
//! `interaction_range` away (75 units for every seeded type), measured
//! from your server position to the station's. React draws its orange
//! ring on the same test (`GameCanvas.tsx`, `distance <= 75`), and so
//! does the Flint client. P6.2's placement checks and P6.3's open-or-walk
//! belong here too.

use capitol_protocol::rest::StationInfo;

/// Whether a player at `at` (server units) is close enough to use
/// `station`: within its interaction range, the edge included.
pub fn in_reach(at: [f64; 2], station: &StationInfo) -> bool {
    let d = (station.x - at[0]).hypot(station.y - at[1]);
    d <= station.interaction_range as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn station(x: f64, y: f64) -> StationInfo {
        StationInfo {
            id: Uuid::nil(),
            station_type: "workbench".into(),
            name: "Workbench".into(),
            category: "crafting".into(),
            icon: None,
            x,
            y,
            owner_id: Uuid::nil(),
            container_id: None,
            interaction_range: 75.0,
        }
    }

    #[test]
    fn the_ring_toggles_at_75_units() {
        let s = station(4000.0, 7350.0);
        assert!(in_reach([4000.0, 7350.0], &s));
        assert!(in_reach([4075.0, 7350.0], &s), "the edge counts");
        assert!(in_reach([4045.0, 7410.0], &s), "75 on the diagonal");
        assert!(!in_reach([4075.1, 7350.0], &s));
        assert!(!in_reach([4000.0, 7274.0], &s));
    }

    #[test]
    fn the_range_comes_from_the_station() {
        let mut s = station(0.0, 0.0);
        s.interaction_range = 120.0;
        assert!(in_reach([100.0, 0.0], &s));
    }
}
