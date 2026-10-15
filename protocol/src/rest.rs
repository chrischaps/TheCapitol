//! Request and response bodies for the REST API, grouped by route.
//!
//! Paths are relative to the server root (`http://localhost:3000` locally).
//! Endpoints other than auth, health, terrain and bureaucracy need an
//! `Authorization: Bearer <token>` header.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::rules;

// ---------------------------------------------------------------------------
// Shared bodies
// ---------------------------------------------------------------------------

/// `{"success": true}`, returned by `POST /auth/logout` and `DELETE /stations/{id}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
}

impl SuccessResponse {
    pub fn ok() -> Self {
        Self { success: true }
    }
}

// ---------------------------------------------------------------------------
// /auth
// ---------------------------------------------------------------------------

/// `POST /auth/register`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RegisterRequest {
    /// Email address for the account
    pub email: String,
    /// Password (will be hashed with Argon2)
    pub password: String,
    /// Display name for the player character
    pub player_name: String,
}

/// `POST /auth/login`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct LoginRequest {
    /// Account email address
    pub email: String,
    /// Account password
    pub password: String,
}

/// `POST /auth/logout`; responds with [`SuccessResponse`]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct LogoutRequest {
    /// JWT token to invalidate
    pub token: String,
}

/// Response to register and login
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct AuthResponse {
    /// JWT token for authentication
    pub token: String,
    /// Account information
    pub account: AccountInfo,
    /// Player character ID
    pub player_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct AccountInfo {
    /// Unique account identifier
    pub id: Uuid,
    /// Email address
    pub email: String,
    /// Account tier (free, premium, etc.)
    pub tier: String,
    /// Whether this account has admin privileges (debug features)
    pub is_admin: bool,
}

// ---------------------------------------------------------------------------
// /health
// ---------------------------------------------------------------------------

/// `GET /health`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct HealthResponse {
    /// Overall health status: "healthy" or "degraded"
    pub status: String,
    /// Database connection status
    pub database: String,
    /// Redis connection status
    pub redis: String,
}

// ---------------------------------------------------------------------------
// /player
// ---------------------------------------------------------------------------

/// `POST /player/move`; responds with [`MoveResponse`]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MoveRequest {
    /// Destination X coordinate (0-8000)
    pub x: f64,
    /// Destination Y coordinate (0-8000)
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveResponse {
    pub success: bool,
    pub destination: Destination,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Destination {
    pub x: f64,
    pub y: f64,
}

/// `GET /player/me`: the token's player, and where they are now
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlayerMeResponse {
    pub player_id: Uuid,
    pub name: String,
    /// Live X while `online`, otherwise the last saved one (0-8000)
    pub x: f64,
    /// Live Y while `online`, otherwise the last saved one (0-8000)
    pub y: f64,
    /// Zone ID at that position, as in `zone_changed` (e.g. "rural")
    pub zone: String,
    /// Current Strand balance
    pub strand_balance: i64,
    /// Whether a WebSocket session has the player in the game. When false,
    /// `x`/`y` come from the database, which the server writes every 10 s.
    pub online: bool,
}

/// `GET /player/inventory`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct InventoryResponse {
    /// All items owned by the player
    pub items: Vec<InventoryItem>,
}

/// Client-facing inventory item
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct InventoryItem {
    /// Unique item instance ID
    pub id: Uuid,
    /// Item type identifier (e.g., "fiber", "rope")
    pub item_type: String,
    /// Display name of the item
    pub name: String,
    /// Quality value (0-100)
    pub quality: i32,
    /// Quality grade letter (A, B, C, D, F)
    pub quality_grade: String,
    /// Stack quantity
    pub quantity: i32,
}

/// `GET /player/currency`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CurrencyResponse {
    /// Current Strand balance
    pub strand_balance: i64,
}

// ---------------------------------------------------------------------------
// /world
// ---------------------------------------------------------------------------

/// `GET /world/nearby`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct NearbyResponse {
    /// Resource nodes within range of the player
    pub nodes: Vec<ResourceNodeInfo>,
}

/// Client-facing resource node data
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ResourceNodeInfo {
    /// Unique node identifier
    pub id: Uuid,
    /// Type of resource (e.g., "fiber_plant")
    pub resource_type: String,
    /// X position in world coordinates
    pub x: f64,
    /// Y position in world coordinates
    pub y: f64,
    /// Current state: "available", "harvesting", or "depleted"
    pub state: String,
}

/// `POST /world/extract`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ExtractRequest {
    /// ID of the resource node to extract from
    pub node_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ExtractResponse {
    /// Whether extraction was started successfully
    pub success: bool,
    /// Optional message (e.g., error reason)
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// /inventory
// ---------------------------------------------------------------------------

/// `GET /inventory/containers`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ContainersResponse {
    /// All containers owned by the player
    pub containers: Vec<ContainerResponse>,
}

/// Container state for API response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ContainerResponse {
    /// Container instance ID
    pub id: Uuid,
    /// Container type identifier
    pub container_type: String,
    /// Total number of slots
    pub slot_count: i32,
    /// Number of columns for UI layout
    pub layout_columns: i32,
    /// Items in each slot
    pub slots: Vec<SlotItem>,
}

/// Item in a specific slot (for API responses)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct SlotItem {
    /// Unique item instance ID
    pub item_id: Uuid,
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Quality value (0-100)
    pub quality: i32,
    /// Quality grade letter (A, B, C, D, F)
    pub quality_grade: String,
    /// Stack quantity
    pub quantity: i32,
    /// Slot index within the container
    pub slot_index: i32,
}

impl SlotItem {
    /// A slot item, with its grade derived from `quality`.
    pub fn new(
        item_id: Uuid,
        item_type: String,
        item_name: String,
        quality: i32,
        quantity: i32,
        slot_index: i32,
    ) -> Self {
        Self {
            item_id,
            item_type,
            item_name,
            quality,
            quality_grade: rules::quality_grade(quality).to_string(),
            quantity,
            slot_index,
        }
    }
}

/// `POST /inventory/move`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MoveItemRequest {
    /// ID of the item to move
    pub item_id: Uuid,
    /// Target container ID
    pub target_container_id: Uuid,
    /// Target slot index
    pub target_slot: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MoveItemResponse {
    /// Whether the operation succeeded
    pub success: bool,
    /// Action taken: "moved", "merged", or "swapped"
    pub action: String,
    /// New quantity if items were merged
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_quantity: Option<i32>,
    /// New quality if items were merged
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_quality: Option<i32>,
}

// ---------------------------------------------------------------------------
// /recipes
// ---------------------------------------------------------------------------

/// `GET /recipes`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RecipesResponse {
    /// Available crafting recipes
    pub recipes: Vec<RecipeInfo>,
}

/// Client-facing recipe info for UI
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RecipeInfo {
    /// Unique recipe identifier
    pub id: String,
    /// Display name of the recipe
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Recipe category (e.g., "hand_crafting")
    pub category: String,
    /// Required input items
    pub inputs: Vec<RecipeInputInfo>,
    /// Produced output items
    pub outputs: Vec<RecipeOutputInfo>,
    /// Base crafting duration in ticks
    pub duration_ticks: i32,
    /// Station type required (null for hand crafting)
    pub required_station_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RecipeInputInfo {
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Required quantity
    pub quantity: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RecipeOutputInfo {
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Minimum output quantity
    pub quantity_min: i32,
    /// Maximum output quantity
    pub quantity_max: i32,
}

// ---------------------------------------------------------------------------
// /stations
// ---------------------------------------------------------------------------

/// `GET /stations/nearby`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct NearbyStationsResponse {
    /// Stations within visibility range
    pub stations: Vec<StationInfo>,
}

/// Client-facing station info
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StationInfo {
    /// Unique station instance ID
    pub id: Uuid,
    /// Station type identifier (e.g., "workbench", "forge")
    pub station_type: String,
    /// Display name of the station
    pub name: String,
    /// Station category ("crafting" or "storage")
    pub category: String,
    /// Optional icon identifier
    pub icon: Option<String>,
    /// X position in world coordinates
    pub x: f64,
    /// Y position in world coordinates
    pub y: f64,
    /// Player who owns this station
    pub owner_id: Uuid,
    /// Container ID for station inventory
    pub container_id: Option<Uuid>,
    /// Maximum interaction distance
    pub interaction_range: f32,
}

/// `POST /stations/place`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlaceStationRequest {
    /// Station type to place (workbench, forge, storage_chest)
    pub station_type: String,
    /// X position in world coordinates
    pub x: f64,
    /// Y position in world coordinates
    pub y: f64,
    /// Kit item ID to consume
    pub kit_item_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlaceStationResponse {
    /// The newly placed station
    pub station: StationInfo,
}

/// `GET /stations/{id}/container`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StationContainerResponse {
    /// Container ID
    pub id: Uuid,
    /// Container type
    pub container_type: String,
    /// Total slots
    pub slot_count: i32,
    /// UI layout columns
    pub layout_columns: i32,
    /// Items in slots
    pub slots: Vec<SlotItem>,
}

// ---------------------------------------------------------------------------
// /exchange
// ---------------------------------------------------------------------------

/// `POST /exchange/deposit`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DepositRequest {
    /// Item IDs to deposit (must be fiber items)
    pub item_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DepositResponse {
    /// Total fiber quantity deposited
    pub deposited_quantity: i32,
    /// Exchange fee (0.5%)
    pub fee: i64,
    /// Net Strands received after fee
    pub strands_received: i64,
    /// New Strand balance
    pub new_balance: i64,
}

/// `POST /exchange/withdraw`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct WithdrawRequest {
    /// Amount of fiber to withdraw
    pub quantity: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct WithdrawResponse {
    /// Total Strands spent (including fee)
    pub strands_spent: i64,
    /// Exchange fee (0.5%)
    pub fee: i64,
    /// Fiber items received
    pub fiber_received: i32,
    /// New Strand balance
    pub new_balance: i64,
}

// ---------------------------------------------------------------------------
// /plots
// ---------------------------------------------------------------------------

/// `GET /plots` query string
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct ListPlotsQuery {
    /// Filter by zone ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// Only show unclaimed plots
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unclaimed: Option<bool>,
    /// Filter by owner player ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Uuid>,
    /// Max results (default 50, max 100)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    /// Pagination offset
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ListPlotsResponse {
    /// Matching plots
    pub plots: Vec<PlotInfo>,
    /// Total count (for pagination)
    pub total: i64,
}

/// `GET /plots/at` query string; responds with `Option<PlotInfo>`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct PositionQuery {
    /// X coordinate
    pub x: f64,
    /// Y coordinate
    pub y: f64,
}

/// Client-facing plot info. `GET /plots/{id}` returns one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
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
    /// The `/terrain` lot this plot is (`TerrainPlot::id`). Clients join the two
    /// on it. `None` only for a plot claimed before B7 that no lot replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain_id: Option<String>,
    /// The lot's four corners, as `/terrain` draws them. This is the shape that's
    /// claimed and that station placement tests. `bounds` is its bounding box.
    /// `None` only for a pre-B7 plot, whose shape is `bounds`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corners: Option<[(f64, f64); 4]>,
}

impl PlotInfo {
    /// Whether a point is on this plot: inside its lot, or its box if it has none.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match &self.corners {
            Some(corners) => crate::terrain::quad_contains(corners, x, y),
            None => self.bounds.contains(x, y),
        }
    }
}

/// Axis-aligned bounding box for a plot
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
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
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// Whether a point is inside the box (edges included)
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    /// The center point
    pub fn center(&self) -> (f64, f64) {
        (
            (self.min_x + self.max_x) / 2.0,
            (self.min_y + self.max_y) / 2.0,
        )
    }

    /// Width and height
    pub fn size(&self) -> (f64, f64) {
        (self.max_x - self.min_x, self.max_y - self.min_y)
    }
}

/// `POST /plots/{id}/claim`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ClaimPlotResponse {
    /// The claimed plot
    pub plot: PlotInfo,
    /// New Strand balance after purchase
    pub new_balance: i64,
}

/// `GET /plots/{id}/permissions`, and the response to `PUT` on the same path
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema, sqlx::FromRow))]
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

/// `PUT /plots/{id}/permissions`. Fields left out are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct UpdatePermissionsRequest {
    /// Who can enter: "public", "friends", "guild", "owner"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_level: Option<String>,
    /// Who can see structures: "public", "friends", "guild", "owner"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_level: Option<String>,
    /// Who can use stations: "public", "friends", "guild", "owner"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station_use_level: Option<String>,
    /// Who can modify: "public", "friends", "guild", "owner"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modify_level: Option<String>,
}

// ---------------------------------------------------------------------------
// /bureaucracy
// ---------------------------------------------------------------------------

/// `GET /bureaucracy/zones`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ZonesResponse {
    /// All zones in the game world
    pub zones: Vec<ZoneInfo>,
}

/// Zone info from the database
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema, sqlx::FromRow))]
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

/// `GET /bureaucracy/deed-prices`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DeedPricesResponse {
    /// Deed prices for all zones with purchasable plots
    pub prices: Vec<DeedPrice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DeedPrice {
    /// Zone identifier
    pub zone_id: String,
    /// Zone display name
    pub zone_name: String,
    /// Cost in Strands to purchase a deed
    pub deed_cost: i32,
    /// Plot size category name
    pub plot_size: String,
    /// Maximum stations allowed on plots in this zone
    pub station_capacity: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_contains_and_center() {
        let bounds = Bounds::new(100.0, 100.0, 200.0, 200.0);
        assert!(bounds.contains(150.0, 150.0));
        assert!(bounds.contains(100.0, 100.0));
        assert!(bounds.contains(200.0, 200.0));
        assert!(!bounds.contains(99.0, 150.0));
        assert!(!bounds.contains(201.0, 150.0));
        assert_eq!(bounds.center(), (150.0, 150.0));
        assert_eq!(bounds.size(), (100.0, 100.0));
    }

    #[test]
    fn slot_item_grades_its_quality() {
        let item = SlotItem::new(Uuid::nil(), "rope".into(), "Rope".into(), 81, 1, 0);
        assert_eq!(item.quality_grade, "B");
    }
}
