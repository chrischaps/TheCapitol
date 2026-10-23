use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::models::{PlayerPosition, SlotItem};
use capitol_protocol::rest::{Commission, DepthLevel, MarketOrder, MarketTrade};
use capitol_protocol::ws::{ToolUse, TradeItemInfo};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum GameEvent {
    /// Everyone's position this tick, by interest cell (C0.4)
    PlayerPositions(std::sync::Arc<super::interest::Positions>),
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
        /// The tool that did it, after the use (E1.5)
        tool: Option<ToolUse>,
    },
    ExtractionCancelled {
        player_id: Uuid,
    },
    /// An extraction from `player_id` was refused. Goes to that player only.
    ExtractionFailed {
        player_id: Uuid,
        reason: String,
    },
    NodeDepleted {
        node_id: Uuid,
    },
    NodeRegenerated {
        node_id: Uuid,
    },
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
        /// The recipe's tool, after the use (E1.5)
        tool: Option<ToolUse>,
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
    // M4: Station events (placing, opening and closing are sent by the connection)
    StationRemoved {
        station_id: Uuid,
    },
    /// A station's condition crossed 25 or 0, or it was repaired (V3.2)
    StationCondition {
        station_id: Uuid,
        condition: i32,
    },
    /// A station's tool slot changed: installed, removed, or worn by a craft (E1.6)
    StationToolChanged {
        station_id: Uuid,
        tool: Option<capitol_protocol::rest::StationTool>,
    },
    /// A station's automation changed: configured, started or stopped (E4.2)
    StationAutomationChanged {
        station_id: Uuid,
        automation: capitol_protocol::rest::StationAutomation,
    },
    /// `owner_id`'s station made `produced` on its own (E4.3)
    AutomationProduced {
        owner_id: Uuid,
        station_id: Uuid,
        produced: capitol_protocol::rest::Produced,
        done: u32,
    },
    /// `owner_id`'s station paused (E4.3). Queued as `notice_id`, as the tax
    /// notices are.
    AutomationPaused {
        owner_id: Uuid,
        station_id: Uuid,
        station_type: String,
        reason: capitol_protocol::rest::PauseReason,
        notice_id: Option<Uuid>,
    },
    /// A bazaar notice for `player_id` (E3.10): a sale at their stall, or
    /// the stall closing. Queued as `notice_id`, as the tax notices are.
    BazaarNotice {
        player_id: Uuid,
        notice_id: Uuid,
        message: capitol_protocol::ws::ServerMessage,
    },
    // M5: Trading events
    TradeRequested {
        from_player: Uuid,
        from_player_name: String,
        from_citizen: bool,
        to_player: Uuid,
    },
    TradeRequestDeclined {
        by_player: Uuid,
        initiator: Uuid,
    },
    /// Nobody answered `initiator`'s request to `target` in time. Goes to both (#147).
    TradeRequestExpired {
        initiator: Uuid,
        target: Uuid,
    },
    TradeStarted {
        trade_id: Uuid,
        player_a: Uuid,
        player_b: Uuid,
    },
    TradeOfferUpdated {
        trade_id: Uuid,
        player_id: Uuid,
        /// Filled in when the offer is validated, so the event is what the wire carries
        items: Vec<TradeItemInfo>,
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
    /// A trade command from `player_id` was refused. Goes to that player only, as `error`.
    TradeFailed {
        player_id: Uuid,
        /// The refused `ClientMessage`'s `"type"`, e.g. `"trade_initiate"`.
        command: &'static str,
        reason: String,
    },
    // M5: Currency events
    CurrencyChanged {
        player_id: Uuid,
        new_balance: i64,
    },
    /// A tool of `player_id`'s used its last durability (E1.4). To them only.
    #[allow(dead_code)] // E1.5 (#157) wears tools at work; only the tests break one yet
    ToolBroke {
        player_id: Uuid,
        item_id: Uuid,
        item_type: String,
        item_name: String,
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
        water_id: String,
        stopped_x: f64,
        stopped_y: f64,
    },
    /// Terrain was regenerated with new params (broadcast to all clients)
    TerrainUpdated,
    // B13: Plot ownership (broadcast to all clients)
    PlotClaimed {
        terrain_id: String,
        owner_id: Uuid,
        claimed_at: DateTime<Utc>,
    },
    PlotReleased {
        terrain_id: String,
    },
    // #131: Property tax, to the plot's owner only. Each is also queued as
    // `notice_id` (#138): the connection sends it only if it takes the notice.
    TaxDelinquent {
        owner_id: Uuid,
        terrain_id: String,
        tax_owed: i64,
        forecloses_at: i64,
        notice_id: Option<Uuid>,
    },
    PlotForeclosed {
        owner_id: Uuid,
        terrain_id: String,
        tax_owed: i64,
        refund: i64,
        /// What went to the owner's impound (#141), and what found no room even there
        impounded: Vec<capitol_protocol::ws::ItemCount>,
        lost: Vec<capitol_protocol::ws::ItemCount>,
        notice_id: Option<Uuid>,
    },
    /// #188: `owner_id`'s workbench stood off a plot where a terrain change
    /// laid the lot `terrain_id`, and was taken down. Queued as `notice_id`.
    StationDisplaced {
        owner_id: Uuid,
        station_id: Uuid,
        station_type: String,
        terrain_id: String,
        notice_id: Option<Uuid>,
    },
    /// E4.1: the player's subscription ran out. Queued as `notice_id`, as
    /// the tax notices are.
    SubscriptionLapsed {
        player_id: Uuid,
        until: DateTime<Utc>,
        notice_id: Option<Uuid>,
    },
    // E3.2: the market's matching engine
    /// Some or all of `player_id`'s order traded. One to each party.
    OrderFilled {
        player_id: Uuid,
        order: MarketOrder,
        trade: MarketTrade,
    },
    /// `player_id`'s order left the book, cancelled or expired (its `status`
    /// says which)
    OrderClosed { player_id: Uuid, order: MarketOrder },
    /// A fill, for the item's market channel
    MarketTrade { trade: MarketTrade },
    /// E3.9: the Gazette's edition about `day` is out (to everyone)
    GazettePublished { day: i64, number: i64, headline: String },
    /// Commons S1.1: a line said. Each socket decides whether its player
    /// hears it (`talk::hears`). `seq` numbers Capitol lines, 0 for a local one.
    Said { seq: u64, line: super::talk::Line },
    /// Commons S1.4: `player_id` muted or unmuted someone. Their sockets
    /// read their mutes again (`mutes::of`).
    MutesChanged { player_id: Uuid },
    /// Commons S4.1: `guild_id`'s roster or waiting list changed. Each of
    /// `players` is sent `guild_changed`, to read their guild again.
    GuildChanged { guild_id: Uuid, players: Vec<Uuid> },
    /// D6.1: an admin moved a fee (to everyone)
    FeesChanged { fees: capitol_protocol::tunables::Fees },
    /// Commons S2.1: a commission went up on the hall's board, was delivered
    /// to, or came down (to everyone, so the board stays live)
    Board { change: BoardChange, commission: Commission },
    /// A notice queued for `player_id` (`notices::queue`), as `notice_id`:
    /// their socket sends it live if it can take it first (Commons S2.1)
    Notice {
        player_id: Uuid,
        notice_id: Uuid,
        message: capitol_protocol::ws::ServerMessage,
    },
    /// Commons S2.6: wages' news (`wages::announce`). `to` `None` goes to
    /// everyone, for the board's Work page; `Some` to that player alone, as
    /// a worker's pay as it's made.
    Wages { to: Option<Uuid>, message: capitol_protocol::ws::ServerMessage },
    /// The road in (#438): `message` (`tutorial` or `tutorial_step`) to each
    /// of `player_id`'s sockets
    Tutorial { player_id: Uuid, message: capitol_protocol::ws::ServerMessage },
    // E3.3: the market API
    /// `player_id`'s order is on the book
    OrderPlaced { player_id: Uuid, order: MarketOrder },
    /// An item's depth changed: the changed levels, for its market channel
    /// (`Market::publish`)
    MarketDepth {
        item_type: String,
        seq: u64,
        levels: Vec<DepthLevel>,
    },
}

/// Slot update for inventory events
#[derive(Debug, Clone, Serialize)]
pub struct SlotUpdate {
    pub slot_index: i32,
    pub item: Option<SlotItem>,
}

/// What happened to a commission on the board (Commons S2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BoardChange {
    Posted,
    /// Units were delivered to it
    Updated,
    /// Filled, cancelled or expired
    Closed,
}

impl BoardChange {
    /// The message every socket sends for it
    pub fn message(self, commission: Commission) -> capitol_protocol::ws::ServerMessage {
        use capitol_protocol::ws::ServerMessage;
        match self {
            BoardChange::Posted => ServerMessage::CommissionPosted { commission },
            BoardChange::Updated => ServerMessage::CommissionUpdated { commission },
            BoardChange::Closed => ServerMessage::CommissionClosed { commission },
        }
    }
}
