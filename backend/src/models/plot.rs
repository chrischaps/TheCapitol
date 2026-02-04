use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// Axis-aligned bounding box for a plot
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct Bounds {
    /// Minimum X coordinate
    pub min_x: f64,
    /// Minimum Y coordinate
    pub min_y: f64,
    /// Maximum X coordinate
    pub max_x: f64,
    /// Maximum Y coordinate
    pub max_y: f64,
}

impl Bounds {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self { min_x, min_y, max_x, max_y }
    }

    /// Check if a point is inside this bounding box
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    /// Get the center point of the bounds
    pub fn center(&self) -> (f64, f64) {
        (
            (self.min_x + self.max_x) / 2.0,
            (self.min_y + self.max_y) / 2.0,
        )
    }

    /// Get the width and height
    pub fn size(&self) -> (f64, f64) {
        (self.max_x - self.min_x, self.max_y - self.min_y)
    }
}

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

    /// Check if a point is within this plot's bounds
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.bounds().contains(x, y)
    }

    /// Check if this plot is owned
    pub fn is_owned(&self) -> bool {
        self.owner_id.is_some()
    }

    /// Check if a specific player owns this plot
    pub fn is_owned_by(&self, player_id: Uuid) -> bool {
        self.owner_id == Some(player_id)
    }
}

/// Plot size category from the database
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlotSizeCategory {
    pub id: String,
    pub name: String,
    pub width: f32,
    pub height: f32,
    pub station_capacity: i32,
}

/// Plot permissions from the database
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct PlotPermissions {
    /// Plot this permission set applies to
    pub plot_id: Uuid,
    /// Who can enter the plot: "public", "friends", "guild", "owner"
    pub entry_level: String,
    /// Who can see structures: "public", "friends", "guild", "owner"
    pub visibility_level: String,
    /// Who can use stations: "public", "friends", "guild", "owner"
    pub station_use_level: String,
    /// Who can modify: "public", "friends", "guild", "owner"
    pub modify_level: String,
}

impl Default for PlotPermissions {
    fn default() -> Self {
        Self {
            plot_id: Uuid::nil(),
            entry_level: "public".to_string(),
            visibility_level: "public".to_string(),
            station_use_level: "owner".to_string(),
            modify_level: "owner".to_string(),
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
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionLevel::Public => "public",
            PermissionLevel::Friends => "friends",
            PermissionLevel::Guild => "guild",
            PermissionLevel::Owner => "owner",
        }
    }

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

/// Client-facing plot info
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PlotInfo {
    /// Unique plot identifier
    pub id: Uuid,
    /// Zone the plot is in
    pub zone_id: String,
    /// Center X coordinate
    pub world_x: f64,
    /// Center Y coordinate
    pub world_y: f64,
    /// Plot boundaries
    pub bounds: Bounds,
    /// Size category (small, medium, large)
    pub size_category: String,
    /// Plot type (standard, corner, etc.)
    pub plot_type: String,
    /// Owner player ID if claimed
    pub owner_id: Option<Uuid>,
    /// Owner player name if claimed
    pub owner_name: Option<String>,
    /// When the plot was claimed
    pub claimed_at: Option<DateTime<Utc>>,
    /// Current assessed value for taxes
    pub assessed_value: i32,
    /// Current number of stations placed
    pub station_count: i32,
    /// Maximum stations allowed
    pub station_capacity: i32,
}

impl PlotInfo {
    pub fn from_plot(plot: &Plot, owner_name: Option<String>, station_capacity: i32) -> Self {
        Self {
            id: plot.id,
            zone_id: plot.zone_id.clone(),
            world_x: plot.world_x as f64,
            world_y: plot.world_y as f64,
            bounds: plot.bounds(),
            size_category: plot.size_category.clone(),
            plot_type: plot.plot_type.clone(),
            owner_id: plot.owner_id,
            owner_name,
            claimed_at: plot.claimed_at,
            assessed_value: plot.assessed_value,
            station_count: plot.station_count,
            station_capacity,
        }
    }
}

/// Request to claim a plot
#[derive(Debug, Deserialize)]
pub struct ClaimPlotRequest {
    // No additional fields needed - plot ID comes from path
}

/// Request to update plot permissions
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePermissionsRequest {
    /// Who can enter: "public", "friends", "guild", "owner"
    pub entry_level: Option<String>,
    /// Who can see structures: "public", "friends", "guild", "owner"
    pub visibility_level: Option<String>,
    /// Who can use stations: "public", "friends", "guild", "owner"
    pub station_use_level: Option<String>,
    /// Who can modify: "public", "friends", "guild", "owner"
    pub modify_level: Option<String>,
}

/// Row returned from plot queries with joins (for sqlx::query_as)
#[derive(Debug, Clone, FromRow)]
pub struct PlotWithJoins {
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
    pub owner_name: Option<String>,
    pub station_capacity: i32,
}

impl PlotWithJoins {
    pub fn to_plot_info(&self) -> PlotInfo {
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
            claimed_at: self.claimed_at,
            assessed_value: self.assessed_value,
            station_count: self.station_count,
            station_capacity: self.station_capacity,
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
