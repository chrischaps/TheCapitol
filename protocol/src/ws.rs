//! WebSocket messages on `/ws`.
//!
//! Every message is a JSON object tagged by `"type"`. The handshake:
//! 1. send [`ClientMessage::Auth`] within 10 s of connecting;
//! 2. on [`ServerMessage::AuthOk`], send [`ClientMessage::Subscribe`] with
//!    `channel: "position"`. Until then the server drops every event.
//!
//! Many server events are broadcast to everyone, so a client must check
//! `player_id` rather than assume an event is about itself.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::rest::{ResourceNodeInfo, StationInfo};

/// Client to server.
///
/// IDs are sent as strings and parsed by the server, which answers a malformed
/// one with [`ServerMessage::Error`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "auth")]
    Auth { token: String },
    #[serde(rename = "subscribe")]
    Subscribe { channel: String },
    #[serde(rename = "move")]
    Move { x: f64, y: f64 },
    #[serde(rename = "extract")]
    Extract { node_id: String },
    #[serde(rename = "cancel_extraction")]
    CancelExtraction,
    // M3: Crafting
    #[serde(rename = "craft")]
    Craft {
        recipe_id: String,
        input_item_ids: Vec<String>,
    },
    #[serde(rename = "cancel_crafting")]
    CancelCrafting,
    // M3.5: Inventory
    #[serde(rename = "move_item")]
    MoveItem {
        item_id: String,
        target_container_id: String,
        target_slot: i32,
    },
    // M4: Stations
    #[serde(rename = "place_station")]
    PlaceStation {
        station_type: String,
        x: f64,
        y: f64,
        kit_item_id: String,
    },
    #[serde(rename = "remove_station")]
    RemoveStation { station_id: String },
    #[serde(rename = "open_station")]
    OpenStation { station_id: String },
    #[serde(rename = "close_station")]
    CloseStation,
    #[serde(rename = "craft_at_station")]
    CraftAtStation {
        station_id: String,
        recipe_id: String,
        input_item_ids: Vec<String>,
    },
    // M5: Trading
    #[serde(rename = "trade_initiate")]
    TradeInitiate { target_player_id: String },
    #[serde(rename = "trade_accept_request")]
    TradeAcceptRequest,
    #[serde(rename = "trade_decline_request")]
    TradeDeclineRequest,
    #[serde(rename = "trade_offer")]
    TradeOffer {
        items: Vec<TradeOfferItem>,
        strands: i64,
    },
    #[serde(rename = "trade_accept")]
    TradeAccept,
    #[serde(rename = "trade_cancel")]
    TradeCancel,
}

impl ClientMessage {
    /// The message's `"type"` tag.
    pub fn type_name(&self) -> &'static str {
        match self {
            ClientMessage::Auth { .. } => "auth",
            ClientMessage::Subscribe { .. } => "subscribe",
            ClientMessage::Move { .. } => "move",
            ClientMessage::Extract { .. } => "extract",
            ClientMessage::CancelExtraction => "cancel_extraction",
            ClientMessage::Craft { .. } => "craft",
            ClientMessage::CancelCrafting => "cancel_crafting",
            ClientMessage::MoveItem { .. } => "move_item",
            ClientMessage::PlaceStation { .. } => "place_station",
            ClientMessage::RemoveStation { .. } => "remove_station",
            ClientMessage::OpenStation { .. } => "open_station",
            ClientMessage::CloseStation => "close_station",
            ClientMessage::CraftAtStation { .. } => "craft_at_station",
            ClientMessage::TradeInitiate { .. } => "trade_initiate",
            ClientMessage::TradeAcceptRequest => "trade_accept_request",
            ClientMessage::TradeDeclineRequest => "trade_decline_request",
            ClientMessage::TradeOffer { .. } => "trade_offer",
            ClientMessage::TradeAccept => "trade_accept",
            ClientMessage::TradeCancel => "trade_cancel",
        }
    }
}

/// One line of a trade offer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeOfferItem {
    pub item_id: String,
    pub quantity: i32,
}

/// Server to client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// `server_time_ms` is the server's clock (Unix ms) as it sent this. A
    /// client keeps its offset from that, for [`crate::rules::time_of_day`].
    #[serde(rename = "auth_ok")]
    AuthOk { player_id: Uuid, server_time_ms: i64 },
    #[serde(rename = "auth_error")]
    AuthError { message: String },
    #[serde(rename = "subscribed")]
    Subscribed { channel: String },
    /// Every player's position, every tick (100 ms). No velocity.
    #[serde(rename = "positions")]
    Positions { players: Vec<PlayerPosition> },
    #[serde(rename = "player_joined")]
    PlayerJoined { player: PlayerPosition },
    #[serde(rename = "player_left")]
    PlayerLeft { player_id: Uuid },
    /// A command was refused. Goes to the sender only.
    #[serde(rename = "error")]
    Error {
        message: String,
        /// The refused command's `"type"` ([`ClientMessage::type_name`], e.g.
        /// `"place_station"`), so a client with several commands out can tell
        /// which one this answers. Empty from a server older than #89.
        #[serde(default)]
        command: String,
    },
    // M2: Extraction events
    #[serde(rename = "extraction_started")]
    ExtractionStarted {
        player_id: Uuid,
        node_id: Uuid,
        duration_ticks: u32,
    },
    #[serde(rename = "extraction_progress")]
    ExtractionProgress {
        player_id: Uuid,
        progress: u32,
        duration: u32,
    },
    #[serde(rename = "extraction_completed")]
    ExtractionCompleted {
        player_id: Uuid,
        node_id: Uuid,
        item_type: String,
        item_name: String,
        quantity: i32,
        quality: i32,
    },
    #[serde(rename = "extraction_cancelled")]
    ExtractionCancelled { player_id: Uuid },
    /// An `extract` was refused. Sent to the requester only.
    #[serde(rename = "extraction_failed")]
    ExtractionFailed { player_id: Uuid, reason: String },
    #[serde(rename = "node_depleted")]
    NodeDepleted { node_id: Uuid },
    #[serde(rename = "node_regenerated")]
    NodeRegenerated { node_id: Uuid },
    /// Defined but never sent; clients poll `GET /world/nearby` instead.
    #[serde(rename = "nearby_nodes")]
    NearbyNodes { nodes: Vec<ResourceNodeInfo> },
    // M3: Crafting events
    #[serde(rename = "crafting_started")]
    CraftingStarted {
        player_id: Uuid,
        operation_id: Uuid,
        recipe_id: String,
        recipe_name: String,
        duration_ticks: u32,
    },
    #[serde(rename = "crafting_progress")]
    CraftingProgress {
        player_id: Uuid,
        operation_id: Uuid,
        progress: u32,
        duration: u32,
    },
    #[serde(rename = "crafting_completed")]
    CraftingCompleted {
        player_id: Uuid,
        operation_id: Uuid,
        item_type: String,
        item_name: String,
        quantity: i32,
        quality: i32,
    },
    #[serde(rename = "crafting_cancelled")]
    CraftingCancelled { player_id: Uuid, operation_id: Uuid },
    #[serde(rename = "crafting_failed")]
    CraftingFailed { player_id: Uuid, reason: String },
    // M3.5: Inventory events
    #[serde(rename = "inventory_updated")]
    InventoryUpdated { player_id: Uuid, container_id: Uuid },
    /// Defined but never sent.
    #[serde(rename = "item_moved")]
    ItemMoved {
        player_id: Uuid,
        item_id: Uuid,
        from_container: Uuid,
        from_slot: i32,
        to_container: Uuid,
        to_slot: i32,
    },
    /// Defined but never sent.
    #[serde(rename = "items_merged")]
    ItemsMerged {
        player_id: Uuid,
        source_item_id: Uuid,
        target_item_id: Uuid,
        new_quantity: i32,
        new_quality: i32,
    },
    // M4: Station events
    #[serde(rename = "station_placed")]
    StationPlaced {
        station_id: Uuid,
        station_type: String,
        name: String,
        owner_id: Uuid,
        x: f64,
        y: f64,
        container_id: Uuid,
    },
    #[serde(rename = "station_removed")]
    StationRemoved { station_id: Uuid },
    #[serde(rename = "station_opened")]
    StationOpened {
        player_id: Uuid,
        station_id: Uuid,
        station_type: String,
        name: String,
        container_id: Uuid,
    },
    #[serde(rename = "station_closed")]
    StationClosed { player_id: Uuid },
    /// Defined but never sent; clients poll `GET /stations/nearby` instead.
    #[serde(rename = "nearby_stations")]
    NearbyStations { stations: Vec<StationInfo> },
    // M5: Trading events
    #[serde(rename = "trade_requested")]
    TradeRequested {
        from_player: Uuid,
        from_player_name: String,
    },
    #[serde(rename = "trade_request_declined")]
    TradeRequestDeclined { by_player: Uuid },
    #[serde(rename = "trade_started")]
    TradeStarted {
        trade_id: Uuid,
        partner_id: Uuid,
        partner_name: String,
    },
    #[serde(rename = "trade_offer_updated")]
    TradeOfferUpdated {
        trade_id: Uuid,
        player_id: Uuid,
        items: Vec<TradeItemInfo>,
        strands: i64,
    },
    #[serde(rename = "trade_accepted")]
    TradeAccepted { trade_id: Uuid, player_id: Uuid },
    #[serde(rename = "trade_executed")]
    TradeExecuted { trade_id: Uuid },
    /// Broadcast to every client, like `trade_executed`, so only a matching
    /// `trade_id` is yours. On a disconnect the server sends a placeholder
    /// `trade_id` (B6, #48); the leaving player's `player_left` follows it.
    #[serde(rename = "trade_cancelled")]
    TradeCancelled { trade_id: Uuid, reason: String },
    // M5: Currency events
    /// Defined but never sent; clients refetch `GET /player/currency` instead.
    #[serde(rename = "currency_changed")]
    CurrencyChanged { new_balance: i64 },
    // M7: Zone events
    #[serde(rename = "zone_changed")]
    ZoneChanged {
        player_id: Uuid,
        from_zone: String,
        to_zone: String,
        zone_name: String,
    },
    // Terrain events
    #[serde(rename = "movement_blocked")]
    MovementBlocked {
        player_id: Uuid,
        /// For people: "Cannot cross the outer canal without a bridge".
        reason: String,
        /// The water ring that stopped you (`moat`, `canal_inner`, ...), as
        /// in `/terrain`'s `water_rings`. `rules::water_name` names it.
        /// Empty from a server older than #74.
        #[serde(default)]
        water_id: String,
        stopped_x: f64,
        stopped_y: f64,
    },
    /// The terrain was regenerated; refetch `GET /terrain`.
    #[serde(rename = "terrain_updated")]
    TerrainUpdated,
    // B13: Plot ownership, broadcast to everyone
    /// Someone bought the lot `terrain_id` (as in `/terrain` and
    /// `GET /plots/claimed`). `claimed_at` is when, so a client can tell a
    /// claim it is seeing happen from one it missed.
    #[serde(rename = "plot_claimed")]
    PlotClaimed {
        terrain_id: String,
        owner_id: Uuid,
        claimed_at: DateTime<Utc>,
    },
    /// The lot `terrain_id` has no owner any more: an admin terrain commit
    /// with `force` released it, or it was foreclosed for unpaid tax (#131).
    #[serde(rename = "plot_released")]
    PlotReleased { terrain_id: String },
    // #131: Property tax, sent to the plot's owner only. Both wait for an
    // owner who's offline and are sent when they next subscribe (#138).
    /// Your plot `terrain_id` owes `tax_owed` strands of property tax. At
    /// `forecloses_at` owed (four weeks of tax as charged, so about four
    /// weeks unpaid) it's foreclosed. Sent when the
    /// debt passes each whole week of tax, and once more an hour before.
    /// The hourly tax run takes the debt from your strands when it can, so
    /// any strands you have buy time. One that waited for you says what's
    /// owed as you come back.
    #[serde(rename = "tax_delinquent")]
    TaxDelinquent {
        terrain_id: String,
        tax_owed: i64,
        forecloses_at: i64,
    },
    /// Your plot `terrain_id` was foreclosed for `tax_owed` strands of unpaid
    /// tax. Its stations are gone: their kits came back to your inventory, and
    /// what was in them to whoever owns it, as far as inventories had room
    /// (`inventory_updated` comes first). `refund` strands
    /// (the deed, less the debt) were paid back. `plot_released` follows,
    /// to everyone.
    #[serde(rename = "plot_foreclosed")]
    PlotForeclosed {
        terrain_id: String,
        tax_owed: i64,
        refund: i64,
    },
}

impl ServerMessage {
    /// The message's `"type"` tag. Adding a variant? Add its fixture under
    /// `tests/fixtures/server/` too.
    pub fn type_name(&self) -> &'static str {
        match self {
            ServerMessage::AuthOk { .. } => "auth_ok",
            ServerMessage::AuthError { .. } => "auth_error",
            ServerMessage::Subscribed { .. } => "subscribed",
            ServerMessage::Positions { .. } => "positions",
            ServerMessage::PlayerJoined { .. } => "player_joined",
            ServerMessage::PlayerLeft { .. } => "player_left",
            ServerMessage::Error { .. } => "error",
            ServerMessage::ExtractionStarted { .. } => "extraction_started",
            ServerMessage::ExtractionProgress { .. } => "extraction_progress",
            ServerMessage::ExtractionCompleted { .. } => "extraction_completed",
            ServerMessage::ExtractionCancelled { .. } => "extraction_cancelled",
            ServerMessage::ExtractionFailed { .. } => "extraction_failed",
            ServerMessage::NodeDepleted { .. } => "node_depleted",
            ServerMessage::NodeRegenerated { .. } => "node_regenerated",
            ServerMessage::NearbyNodes { .. } => "nearby_nodes",
            ServerMessage::CraftingStarted { .. } => "crafting_started",
            ServerMessage::CraftingProgress { .. } => "crafting_progress",
            ServerMessage::CraftingCompleted { .. } => "crafting_completed",
            ServerMessage::CraftingCancelled { .. } => "crafting_cancelled",
            ServerMessage::CraftingFailed { .. } => "crafting_failed",
            ServerMessage::InventoryUpdated { .. } => "inventory_updated",
            ServerMessage::ItemMoved { .. } => "item_moved",
            ServerMessage::ItemsMerged { .. } => "items_merged",
            ServerMessage::StationPlaced { .. } => "station_placed",
            ServerMessage::StationRemoved { .. } => "station_removed",
            ServerMessage::StationOpened { .. } => "station_opened",
            ServerMessage::StationClosed { .. } => "station_closed",
            ServerMessage::NearbyStations { .. } => "nearby_stations",
            ServerMessage::TradeRequested { .. } => "trade_requested",
            ServerMessage::TradeRequestDeclined { .. } => "trade_request_declined",
            ServerMessage::TradeStarted { .. } => "trade_started",
            ServerMessage::TradeOfferUpdated { .. } => "trade_offer_updated",
            ServerMessage::TradeAccepted { .. } => "trade_accepted",
            ServerMessage::TradeExecuted { .. } => "trade_executed",
            ServerMessage::TradeCancelled { .. } => "trade_cancelled",
            ServerMessage::CurrencyChanged { .. } => "currency_changed",
            ServerMessage::ZoneChanged { .. } => "zone_changed",
            ServerMessage::MovementBlocked { .. } => "movement_blocked",
            ServerMessage::TerrainUpdated => "terrain_updated",
            ServerMessage::PlotClaimed { .. } => "plot_claimed",
            ServerMessage::PlotReleased { .. } => "plot_released",
            ServerMessage::TaxDelinquent { .. } => "tax_delinquent",
            ServerMessage::PlotForeclosed { .. } => "plot_foreclosed",
        }
    }
}

/// A player's position, as broadcast in `positions` and `player_joined`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerPosition {
    pub id: Uuid,
    pub name: String,
    pub x: f64,
    pub y: f64,
}

/// One line of a trade offer, as the server reports it in `trade_offer_updated`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeItemInfo {
    pub item_id: Uuid,
    pub item_type: String,
    pub item_name: String,
    pub quantity: i32,
    pub quality: i32,
}
