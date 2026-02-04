use serde::Serialize;
use uuid::Uuid;

use crate::models::{PlayerPosition, ResourceNodeInfo, SlotItem, StationInfo, ZoneTransition};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum GameEvent {
    PlayerPositions(Vec<PlayerPosition>),
    PlayerJoined(PlayerPosition),
    PlayerLeft { player_id: Uuid },
    // M2: Gathering events
    ExtractionStarted {
        player_id: Uuid,
        node_id: Uuid,
        duration_ticks: u32,
    },
    ExtractionProgress {
        player_id: Uuid,
        progress: u32,
        duration: u32,
    },
    ExtractionCompleted {
        player_id: Uuid,
        node_id: Uuid,
        item_type: String,
        item_name: String,
        quantity: i32,
        quality: i32,
    },
    ExtractionCancelled {
        player_id: Uuid,
    },
    NodeDepleted {
        node_id: Uuid,
    },
    NodeRegenerated {
        node_id: Uuid,
    },
    NearbyNodes(Vec<ResourceNodeInfo>),
    // M3: Crafting events
    CraftingStarted {
        player_id: Uuid,
        operation_id: Uuid,
        recipe_id: String,
        recipe_name: String,
        duration_ticks: u32,
    },
    CraftingProgress {
        player_id: Uuid,
        operation_id: Uuid,
        progress: u32,
        duration: u32,
    },
    CraftingCompleted {
        player_id: Uuid,
        operation_id: Uuid,
        item_type: String,
        item_name: String,
        quantity: i32,
        quality: i32,
    },
    CraftingCancelled {
        player_id: Uuid,
        operation_id: Uuid,
    },
    CraftingFailed {
        player_id: Uuid,
        reason: String,
    },
    // M3.5: Inventory events
    InventoryUpdated {
        player_id: Uuid,
        container_id: Uuid,
        slots: Vec<SlotUpdate>,
    },
    ItemMoved {
        player_id: Uuid,
        item_id: Uuid,
        from_container: Uuid,
        from_slot: i32,
        to_container: Uuid,
        to_slot: i32,
    },
    ItemsMerged {
        player_id: Uuid,
        source_item_id: Uuid,
        target_item_id: Uuid,
        new_quantity: i32,
        new_quality: i32,
    },
    // M4: Station events
    StationPlaced {
        station_id: Uuid,
        station_type: String,
        name: String,
        owner_id: Uuid,
        x: f64,
        y: f64,
        container_id: Uuid,
    },
    StationRemoved {
        station_id: Uuid,
    },
    StationOpened {
        player_id: Uuid,
        station_id: Uuid,
        station_type: String,
        name: String,
        container_id: Uuid,
    },
    StationClosed {
        player_id: Uuid,
    },
    NearbyStations(Vec<StationInfo>),
    // M5: Trading events
    TradeRequested {
        from_player: Uuid,
        from_player_name: String,
        to_player: Uuid,
    },
    TradeRequestDeclined {
        by_player: Uuid,
        initiator: Uuid,
    },
    TradeStarted {
        trade_id: Uuid,
        player_a: Uuid,
        player_b: Uuid,
    },
    TradeOfferUpdated {
        trade_id: Uuid,
        player_id: Uuid,
        items: Vec<TradeItem>,
        strands: i64,
    },
    TradeAccepted {
        trade_id: Uuid,
        player_id: Uuid,
    },
    TradeExecuted {
        trade_id: Uuid,
    },
    TradeCancelled {
        trade_id: Uuid,
        reason: String,
    },
    // M5: Currency events
    CurrencyChanged {
        player_id: Uuid,
        new_balance: i64,
    },
    // M7: Zone events
    ZoneChanged {
        player_id: Uuid,
        from_zone: String,
        to_zone: String,
        zone_name: String,
    },
    // Terrain events
    MovementBlocked {
        player_id: Uuid,
        reason: String,
        stopped_x: f64,
        stopped_y: f64,
    },
}

/// Item in a trade offer (for events)
#[derive(Debug, Clone, Serialize)]
pub struct TradeItem {
    pub item_id: Uuid,
    pub item_type: String,
    pub item_name: String,
    pub quantity: i32,
    pub quality: i32,
}

/// Slot update for inventory events
#[derive(Debug, Clone, Serialize)]
pub struct SlotUpdate {
    pub slot_index: i32,
    pub item: Option<SlotItem>,
}
