use chrono::{DateTime, Utc};
use capitol_protocol::tunables::Tunables;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub use capitol_protocol::rest::{Bounds, PlotInfo, PlotPermissions, UpdatePermissionsRequest};

/// Plot from the database
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Plot {
    pub id: Uuid,
    pub zone_id: String,
    pub grid_x: i32,
    pub grid_y: i32,
    pub world_x: f32,
    pub world_y: f32,
    pub bounds_min_x: f32,
    pub bounds_min_y: f32,
    pub bounds_max_x: f32,
    pub bounds_max_y: f32,
    pub size_category: String,
    pub plot_type: String,
    pub owner_id: Option<Uuid>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub assessed_value: i32,
    pub last_tax_paid: Option<DateTime<Utc>>,
    pub tax_owed: i32,
    pub station_count: i32,
    pub terrain_id: Option<String>,
    pub corners: Option<Vec<f64>>,
}

/// The columns a `Plot` reads, for `SELECT {PLOT_COLUMNS} FROM plots`.
pub const PLOT_COLUMNS: &str = "id, zone_id, grid_x, grid_y, world_x, world_y,
    bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y,
    size_category, plot_type, owner_id, claimed_at,
    assessed_value, last_tax_paid, tax_owed, station_count, terrain_id, corners";

/// A plot's station capacity, as SQL over `plots p` joined to its
/// `plot_size_categories psc` (Plots v2 §5.6, V3.1, #473): its building's
/// room indoors plus its yard's, or, for a lot with no building (a guild
/// hall), its size category's.
pub const STATION_CAPACITY: &str = "COALESCE(p.indoor_room + p.yard_room, psc.station_capacity)";

/// The flattened `corners` column as four points, if it holds them.
pub fn corners_from_column(column: Option<&[f64]>) -> Option<[(f64, f64); 4]> {
    match column {
        Some(&[x0, y0, x1, y1, x2, y2, x3, y3]) => Some([(x0, y0), (x1, y1), (x2, y2), (x3, y3)]),
        _ => None,
    }
}

impl Plot {
    /// Get the bounds as a Bounds struct
    pub fn bounds(&self) -> Bounds {
        Bounds::new(
            self.bounds_min_x as f64,
            self.bounds_min_y as f64,
            self.bounds_max_x as f64,
            self.bounds_max_y as f64,
        )
    }

    /// The lot's corners, or `None` for a pre-B7 plot that only has a box.
    pub fn quad(&self) -> Option<[(f64, f64); 4]> {
        corners_from_column(self.corners.as_deref())
    }

    /// Whether a point is on this plot: inside its lot, or its box if it has none.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match self.quad() {
            Some(q) => capitol_protocol::terrain::quad_contains(&q, x, y),
            None => self.bounds().contains(x, y),
        }
    }

}

/// Permission level enum for validation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    Public,
    Friends,
    Guild,
    Owner,
}

impl PermissionLevel {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "public" => Some(PermissionLevel::Public),
            "friends" => Some(PermissionLevel::Friends),
            "guild" => Some(PermissionLevel::Guild),
            "owner" => Some(PermissionLevel::Owner),
            _ => None,
        }
    }
}

/// `PlotInfo::forecloses_at`: the foreclosure threshold at the tax `rate`,
/// while it's owned and taxed
fn forecloses_at(owner_id: Option<Uuid>, assessed_value: i32, rate: f64) -> Option<i64> {
    let threshold = crate::engine::property_tax::foreclosure_threshold(assessed_value, rate);
    (owner_id.is_some() && threshold > 0).then_some(threshold)
}

/// Row returned from plot queries with joins (for sqlx::query_as)
#[derive(Debug, Clone, FromRow)]
pub struct PlotWithJoins {
    pub id: Uuid,
    pub zone_id: String,
    pub world_x: f32,
    pub world_y: f32,
    pub bounds_min_x: f32,
    pub bounds_min_y: f32,
    pub bounds_max_x: f32,
    pub bounds_max_y: f32,
    pub size_category: String,
    pub plot_type: String,
    pub owner_id: Option<Uuid>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub assessed_value: i32,
    pub tax_owed: i32,
    pub station_count: i32,
    pub terrain_id: Option<String>,
    pub corners: Option<Vec<f64>>,
    pub owner_name: Option<String>,
    /// The owner is a citizen (C0.1)
    pub owner_citizen: bool,
    /// [`STATION_CAPACITY`]
    pub station_capacity: i32,
    /// Its building's room (V3.1, #473), `None` for a guild hall's
    pub indoor_room: Option<i32>,
    pub yard_room: Option<i32>,
    /// The zone's `deed_cost`, joined from `zones`, before the zone's deed
    /// price tunable (D6.1): [`PlotWithJoins::to_plot_info`] applies it.
    pub deed_cost: Option<i32>,
}

impl PlotWithJoins {
    /// The plot as the API says it, at the `tunables` now: its deed's price
    /// and the debt that forecloses it (D6.1)
    pub fn to_plot_info(&self, tunables: &Tunables) -> PlotInfo {
        PlotInfo {
            id: self.id,
            zone_id: self.zone_id.clone(),
            world_x: self.world_x as f64,
            world_y: self.world_y as f64,
            bounds: Bounds::new(
                self.bounds_min_x as f64,
                self.bounds_min_y as f64,
                self.bounds_max_x as f64,
                self.bounds_max_y as f64,
            ),
            size_category: self.size_category.clone(),
            plot_type: self.plot_type.clone(),
            owner_id: self.owner_id,
            owner_name: self.owner_name.clone(),
            owner_citizen: self.owner_citizen,
            claimed_at: self.claimed_at,
            assessed_value: self.assessed_value,
            tax_owed: self.tax_owed as i64,
            forecloses_at: forecloses_at(self.owner_id, self.assessed_value, tunables.property_tax_rate),
            deed_cost: self.deed_cost.map(|base| tunables.deed_cost(&self.zone_id, base)),
            station_count: self.station_count,
            station_capacity: self.station_capacity,
            terrain_id: self.terrain_id.clone(),
            corners: corners_from_column(self.corners.as_deref()),
            indoor_room: self.indoor_room,
            yard_room: self.yard_room,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounds_contains() {
        let bounds = Bounds::new(100.0, 100.0, 200.0, 200.0);

        assert!(bounds.contains(150.0, 150.0)); // center
        assert!(bounds.contains(100.0, 100.0)); // corner
        assert!(bounds.contains(200.0, 200.0)); // opposite corner
        assert!(!bounds.contains(99.0, 150.0)); // outside left
        assert!(!bounds.contains(201.0, 150.0)); // outside right
    }

    #[test]
    fn test_bounds_center() {
        let bounds = Bounds::new(100.0, 100.0, 200.0, 200.0);
        let (cx, cy) = bounds.center();
        assert_eq!(cx, 150.0);
        assert_eq!(cy, 150.0);
    }
}
