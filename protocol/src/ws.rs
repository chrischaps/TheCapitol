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
use crate::rest::{
    AutomationMode, Commission, DepthLevel, Maker, MarketOrder, MarketTrade, PauseReason, Produced, StationAutomation,
    StationTool, Subscription, WageOffer, WageShift,
};
use crate::rules::stacking::Units;
use crate::tunables::Fees;
use crate::tutorial::{Step, Steps};

/// Client to server.
///
/// IDs are sent as strings and parsed by the server, which answers a malformed
/// one with [`ServerMessage::Error`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// `token` is an access token, or an API key's secret (E0.9). A key
    /// with only `read` watches without taking the player over.
    #[serde(rename = "auth")]
    Auth { token: String },
    #[serde(rename = "subscribe")]
    Subscribe { channel: String },
    #[serde(rename = "move")]
    Move { x: f64, y: f64 },
    /// Walk on a heading (controls T0.2, [`crate::rules::steer`]): `heading`
    /// a fraction of a turn (0 east, a quarter turn south), `throttle` in
    /// tenths, and `at` a waypoint reached first. A stop is throttle 0 with
    /// `at` where the client stopped. It lapses unless renewed, clears a
    /// `move`'s destination, and is never saved.
    #[serde(rename = "steer")]
    Steer {
        heading: u16,
        throttle: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<[f64; 2]>,
    },
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
    /// Split `quantity` off the stack `item_id` into the empty slot `to_slot`
    /// of `to_container_id`, or of its own container without one (E1.2,
    /// #190). Each unit keeps its quality (E1.8).
    #[serde(rename = "split_stack")]
    SplitStack {
        item_id: String,
        quantity: i32,
        to_slot: i32,
        /// Another container to split into, checked as `move_item` checks its
        /// target (#190)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_container_id: Option<String>,
        /// Which units, as `[quality, count]` pairs adding up to `quantity`
        /// (E1.8); without it the lowest go first.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        qualities: Option<Units>,
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
    /// Put a tool from your inventory into a station's tool slot (E1.6). Anyone
    /// who may use the station may; the slot must be empty. Answered by
    /// `station_tool_changed`, or `error`.
    #[serde(rename = "install_tool")]
    InstallTool { station_id: String, item_id: String },
    /// Take a station's tool into your inventory (E1.6): your own, or anyone's
    /// from your own station. Answered by `station_tool_changed`, or `error`.
    #[serde(rename = "remove_tool")]
    RemoveTool { station_id: String },
    /// Set what your station makes on its own (E4.2): `recipe_id` at this
    /// station's type, from and into its own box. Stops it if it was running.
    /// Owner only. Answered by `station_automation_changed`, or `error`.
    #[serde(rename = "configure_automation")]
    ConfigureAutomation { station_id: String, recipe_id: String, mode: AutomationMode },
    /// Set your configured station running (E4.2). Owner only.
    #[serde(rename = "start_automation")]
    StartAutomation { station_id: String },
    /// Stop your station (E4.2). Owner only.
    #[serde(rename = "stop_automation")]
    StopAutomation { station_id: String },
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
    // E3.3: Market channels
    /// Watch an item's book: answered by a full `market_depth`, then its
    /// diffs and `market_trade`s. Ask again to start over from a full one.
    #[serde(rename = "subscribe_market")]
    SubscribeMarket { item_type: String },
    /// Stop watching an item, or every item without one
    #[serde(rename = "unsubscribe_market")]
    UnsubscribeMarket {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_type: Option<String>,
    },
    // E0.8: Presence
    /// Take the avatar over from whichever socket holds it, or pick it up
    /// when nobody does. Answered by [`ServerMessage::Presence`]. A bot's
    /// key is refused while one of the player's own clients is connected.
    #[serde(rename = "claim_presence")]
    ClaimPresence,
    // Commons S1.1: talk
    /// Say a line (Commons D1): `local` to those within
    /// [`crate::rules::talk::HEARD`] of you, `capitol` to everyone, `guild`
    /// to your guild's members (S4.4). It's
    /// cleaned by [`crate::rules::talk::clean`] and counted in the account's
    /// chat bucket. Answered by the `said` everyone who hears it gets, you
    /// among them, or `error`. A watcher (E0.8) may say only `capitol`.
    #[serde(rename = "say")]
    Say { channel: Channel, text: String },
    // The road in (#441): what only the screen shows
    /// You did a step the server can't see: opened your pack, read a zone
    /// card (plan D7). Taken only for a step whose [`Step::judge`] lets the
    /// client report it ([`crate::tutorial::Judge::client_reports`]), and
    /// answered by `tutorial_step` the first time, or `error` for a step
    /// the server judges itself.
    #[serde(rename = "tutorial_seen")]
    TutorialSeen { step: Step },
}

impl ClientMessage {
    /// The message's `"type"` tag.
    pub fn type_name(&self) -> &'static str {
        match self {
            ClientMessage::Auth { .. } => "auth",
            ClientMessage::Subscribe { .. } => "subscribe",
            ClientMessage::Move { .. } => "move",
            ClientMessage::Steer { .. } => "steer",
            ClientMessage::Extract { .. } => "extract",
            ClientMessage::CancelExtraction => "cancel_extraction",
            ClientMessage::Craft { .. } => "craft",
            ClientMessage::CancelCrafting => "cancel_crafting",
            ClientMessage::MoveItem { .. } => "move_item",
            ClientMessage::SplitStack { .. } => "split_stack",
            ClientMessage::PlaceStation { .. } => "place_station",
            ClientMessage::RemoveStation { .. } => "remove_station",
            ClientMessage::OpenStation { .. } => "open_station",
            ClientMessage::CloseStation => "close_station",
            ClientMessage::CraftAtStation { .. } => "craft_at_station",
            ClientMessage::InstallTool { .. } => "install_tool",
            ClientMessage::RemoveTool { .. } => "remove_tool",
            ClientMessage::ConfigureAutomation { .. } => "configure_automation",
            ClientMessage::StartAutomation { .. } => "start_automation",
            ClientMessage::StopAutomation { .. } => "stop_automation",
            ClientMessage::TradeInitiate { .. } => "trade_initiate",
            ClientMessage::TradeAcceptRequest => "trade_accept_request",
            ClientMessage::TradeDeclineRequest => "trade_decline_request",
            ClientMessage::TradeOffer { .. } => "trade_offer",
            ClientMessage::TradeAccept => "trade_accept",
            ClientMessage::TradeCancel => "trade_cancel",
            ClientMessage::SubscribeMarket { .. } => "subscribe_market",
            ClientMessage::UnsubscribeMarket { .. } => "unsubscribe_market",
            ClientMessage::ClaimPresence => "claim_presence",
            ClientMessage::Say { .. } => "say",
            ClientMessage::TutorialSeen { .. } => "tutorial_seen",
        }
    }
}

/// One line of a trade offer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeOfferItem {
    pub item_id: String,
    pub quantity: i32,
    /// Which units, as `[quality, count]` pairs adding up to `quantity`
    /// (E1.8): "my six 92s". Without it the lowest go first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualities: Option<Units>,
}

/// Server to client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// `server_time_ms` is the server's clock (Unix ms) as it sent this. A
    /// client keeps its offset from that, for [`crate::rules::time_of_day`].
    /// `subscription` is the account's, as of now (E4.1). `fees` are the
    /// ones charged now (D6.1): quote from them, and from each `fees_changed`
    /// after. A server older than D6.1 sends none, and charges the defaults.
    /// `is_admin` is the token's (D5.3), so a client can show the admin's
    /// overlay; it's left out for everyone else, and the admin routes check
    /// the token again whatever a client shows.
    #[serde(rename = "auth_ok")]
    AuthOk {
        player_id: Uuid,
        server_time_ms: i64,
        #[serde(default)]
        subscription: Subscription,
        #[serde(default)]
        fees: Fees,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_admin: bool,
    },
    #[serde(rename = "auth_error")]
    AuthError { message: String },
    #[serde(rename = "subscribed")]
    Subscribed { channel: String },
    /// The positions of the players near you, yours among them, every tick
    /// (100 ms). No velocity. Near is your cell of the interest grid and
    /// the eight round it ([`crate::rules::INTEREST_CELL`], C0.4), so it
    /// replaces the set you can see: one missing has gone out of sight.
    #[serde(rename = "positions")]
    Positions { players: Vec<PlayerPosition> },
    /// Someone came into the world near you (or you did).
    #[serde(rename = "player_joined")]
    PlayerJoined { player: PlayerPosition },
    /// Someone left the world, wherever they were: their trades with you
    /// are over.
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
    /// A gather finished. `quantity` 0 is a failed pull (D5, #206): hands or a
    /// tool too weak for the resource got nothing, the tool wore all the same,
    /// and the node is still there to try again. Its `quality` means nothing.
    #[serde(rename = "extraction_completed")]
    ExtractionCompleted {
        player_id: Uuid,
        node_id: Uuid,
        item_type: String,
        item_name: String,
        quantity: i32,
        quality: i32,
        /// The tool that did the work and what it has left (E1.5). Absent by hand.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<ToolUse>,
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
        /// The tool the recipe used and what it has left (E1.5). Absent if it used none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<ToolUse>,
    },
    #[serde(rename = "crafting_cancelled")]
    CraftingCancelled { player_id: Uuid, operation_id: Uuid },
    #[serde(rename = "crafting_failed")]
    CraftingFailed { player_id: Uuid, reason: String },
    // M3.5: Inventory events
    #[serde(rename = "inventory_updated")]
    InventoryUpdated { player_id: Uuid, container_id: Uuid },
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
        /// The station's quality, the station term for crafts made here
        quality: i32,
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
    /// A station's tool slot changed (E1.6): a tool went in or came out, or a
    /// craft there wore it (`tool` is `None` once it breaks). Broadcast, so
    /// only a station you're showing is yours to update.
    #[serde(rename = "station_tool_changed")]
    StationToolChanged { station_id: Uuid, tool: Option<StationTool> },
    /// A station's automation changed (E4.2): configured, started, stopped,
    /// and from E4.3 paused or a craft further on. Broadcast, like
    /// `station_tool_changed`.
    #[serde(rename = "station_automation_changed")]
    StationAutomationChanged { station_id: Uuid, automation: StationAutomation },
    /// Your station made `produced` on its own (E4.3), its `done`th craft
    /// since the start. To the owner only; everyone else hears only when it
    /// starts, pauses or stops (`station_automation_changed`).
    #[serde(rename = "automation_produced")]
    AutomationProduced { station_id: Uuid, produced: Produced, done: u32 },
    /// Your station paused for `reason` (E4.3), and resumes on its own once
    /// that clears. One that paused while you were away is sent as you come
    /// back. `station_automation_changed` goes to everyone alongside.
    #[serde(rename = "automation_paused")]
    AutomationPaused { station_id: Uuid, station_type: String, reason: PauseReason },
    // M5: Trading events
    #[serde(rename = "trade_requested")]
    TradeRequested {
        from_player: Uuid,
        from_player_name: String,
        /// The asker is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        from_citizen: bool,
    },
    #[serde(rename = "trade_request_declined")]
    TradeRequestDeclined { by_player: Uuid },
    /// A request nobody answered within `rules::TRADE_REQUEST_TTL_SECS` is
    /// gone. Sent to both players, the asker (`from_player`) and the one asked
    /// (`to_player`), so neither waits on it (#147). Also sent when a newer
    /// request to `to_player` replaces it (#182).
    #[serde(rename = "trade_request_expired")]
    TradeRequestExpired { from_player: Uuid, to_player: Uuid },
    #[serde(rename = "trade_started")]
    TradeStarted {
        trade_id: Uuid,
        partner_id: Uuid,
        partner_name: String,
        /// The partner is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        partner_citizen: bool,
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
    /// Your strand balance, after anything changes it: a trade, the exchange,
    /// a deed, tax or a refund (#148). Only to the player it names.
    #[serde(rename = "currency_changed")]
    CurrencyChanged { new_balance: i64 },
    /// A tool used its last durability and is gone (E1.4). Only to its owner,
    /// alongside the `inventory_updated` for the container it left.
    #[serde(rename = "tool_broke")]
    ToolBroke { item_id: Uuid, item_type: String, item_name: String },
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
        /// For people: "Cannot cross the outer canal without a bridge",
        /// "Cannot cross the river without a ford or a bridge", "Too steep"
        /// or "The world ends here" (`rules::ground::Blocked::reason`).
        reason: String,
        /// The water that stopped you: a ring (`moat`, `canal_inner`, ...),
        /// as in `/terrain`'s `water_rings`, or a wild river or lake
        /// (`river_3`, `lake_1`, W4). `rules::water_name` names it. Empty
        /// when the slope or the world's edge stopped you, and from a server
        /// older than #74.
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
    /// what was in them to whoever owns it (`inventory_updated` comes first).
    /// What of yours found no room went to your impound (#141): `impounded`,
    /// collected with `POST /inventory/impound/collect`. `lost` is what found no room
    /// even there, and went with the stations. `refund` strands
    /// (the deed, less the debt) were paid back. `plot_released` follows,
    /// to everyone.
    #[serde(rename = "plot_foreclosed")]
    PlotForeclosed {
        terrain_id: String,
        tax_owed: i64,
        refund: i64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        impounded: Vec<ItemCount>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lost: Vec<ItemCount>,
    },
    /// Your workbench `station_id` stood off a plot (D10) where a terrain
    /// change has laid the lot `terrain_id` (#188), so it was taken down.
    /// Its kit, its tool and what was in it came back to your inventory, as
    /// far as it had room (`inventory_updated` comes first), and
    /// `station_removed` goes to everyone. One taken down while you were
    /// away is sent as you come back.
    #[serde(rename = "station_displaced")]
    StationDisplaced {
        station_id: Uuid,
        station_type: String,
        terrain_id: String,
    },
    /// A station's condition crossed 25 or 0 as the weather wore it, or it
    /// was repaired (V3.2). Broadcast, like `station_tool_changed`.
    #[serde(rename = "station_condition")]
    StationCondition { station_id: Uuid, condition: i32 },
    /// Your station on public land wore out and fell down (V3.2). Its kit is
    /// lost; its tool and what was in its box went to your impound (#141),
    /// and `impounded` counts them. `station_removed` goes to everyone. One
    /// that fell while you were away is sent as you come back.
    #[serde(rename = "station_collapsed")]
    StationCollapsed {
        station_id: Uuid,
        station_type: String,
        impounded: Vec<ItemCount>,
    },
    /// Your subscription ran out at `until` (E4.1). What it kept running
    /// stops. One that ran out while you were away is sent as you come back.
    #[serde(rename = "subscription_lapsed")]
    SubscriptionLapsed { until: DateTime<Utc> },

    // E3: Market (plan D7). Each goes only to the order's owner.
    /// Your order is on the book, its escrow and fee taken
    #[serde(rename = "order_placed")]
    OrderPlaced { order: MarketOrder },
    /// Some or all of your order traded: `order` as it stands after `trade`
    #[serde(rename = "order_filled")]
    OrderFilled { order: MarketOrder, trade: MarketTrade },
    /// Your order left the book, cancelled or expired (its `status` says
    /// which), and what was left went back to your market box or balance
    #[serde(rename = "order_cancelled")]
    OrderCancelled { order: MarketOrder },
    // E3.3: an item's market channel, to those who `subscribe_market` it
    /// The item's book changed. `full` is the whole book (the answer to
    /// `subscribe_market`); otherwise `levels` are the changes, a level with
    /// `quantity` 0 gone (`rules::market::apply_depth_diff`). `seq` goes up
    /// by one a change: skip a diff at or below the book's, and on a gap ask
    /// for a full one again.
    #[serde(rename = "market_depth")]
    MarketDepth {
        item_type: String,
        seq: u64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        full: bool,
        levels: Vec<DepthLevel>,
    },
    /// A trade in the item, anyone's
    #[serde(rename = "market_trade")]
    MarketTrade { trade: MarketTrade },
    // E3.10: the bazaar, each to the merchant. One that happened while
    // they were away is sent as they come back.
    /// `buyer_name` bought `units` of your listing at your stall, for `paid`
    #[serde(rename = "bazaar_sale")]
    BazaarSale {
        stall_id: String,
        listing_id: Uuid,
        item_type: String,
        buyer_name: String,
        /// The buyer is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        buyer_citizen: bool,
        units: Units,
        price: i64,
        paid: i64,
    },
    /// Your stall's rent ran out and the shop closed. Its unsold stacks went
    /// to your market box, at the Exchange hall; `waiting` of them are held
    /// until the box has room.
    #[serde(rename = "stall_lapsed")]
    StallLapsed { stall_id: String, name: String, returned: u32, waiting: u32 },
    // E3.9: to everyone
    /// The Gazette's edition about `day` is out (`GET /gazette/latest`)
    #[serde(rename = "gazette_published")]
    GazettePublished { day: i64, number: i64, headline: String },
    // D6.1: to everyone
    /// An admin changed a fee (a live tunable). Quote from these from now on.
    #[serde(rename = "fees_changed")]
    FeesChanged { fees: Fees },
    // E0.8: Presence, to the socket it's about
    /// Whether this socket drives the avatar (D9). An account can have many
    /// sessions, but only one socket holds the avatar: the others watch, and
    /// may still use the market and automation. Sent after `auth_ok`, and
    /// again whenever it changes. `held_by` says who holds it when this
    /// socket doesn't; `None` means nobody does, and the avatar is offline.
    #[serde(rename = "presence")]
    Presence {
        yours: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        held_by: Option<PresenceHolder>,
    },
    // C0.3: to the socket it's about
    /// This socket fell `missed` events behind the server's event stream, and
    /// they were dropped. Treat it as a reconnect: refetch what the server
    /// doesn't push (containers, nodes, stations, plots, claimed lots, your
    /// balance). Your open trade, or a request to or from you, is over: the
    /// server cancelled it, so nobody accepts an offer they didn't see. What
    /// the server does push follows straight after: `positions`, your
    /// `currency_changed`, a full `market_depth` for each item you watch, and
    /// any notices waiting for you.
    #[serde(rename = "resync")]
    Resync { missed: u64 },
    // Commons S1.1: talk
    /// `name` said `text` on `channel`, standing at `x`, `y` (Commons D1).
    /// Local lines come only from within [`crate::rules::talk::HEARD`] of
    /// you; Capitol lines from anyone, and the last
    /// [`crate::rules::talk::CAPITOL_BACKLOG`] of them follow your first
    /// `subscribed` to `position`. Your own lines come back too. Nothing
    /// said is stored.
    #[serde(rename = "said")]
    Said {
        from: Uuid,
        name: String,
        /// The speaker is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        citizen: bool,
        channel: Channel,
        text: String,
        x: f64,
        y: f64,
    },
    // Commons S1.5: letters
    /// A letter came for you from `name` (Commons D4): read it with
    /// `GET /letters?box=in`. One sent while you were away is sent as you
    /// come back, once. `first_line` is [`crate::rules::talk::first_line`].
    #[serde(rename = "letter_arrived")]
    LetterArrived {
        id: Uuid,
        from: Uuid,
        name: String,
        /// The writer is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        citizen: bool,
        /// The writer is one of the city's offices, not a player (#447):
        /// `from` is nil, and there's no one to answer
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        office: bool,
        first_line: String,
    },
    // Commons S2.1: the hall's board of commissions (D5). The first three go
    // to everyone, so the board stays live.
    /// A commission went up on the board
    #[serde(rename = "commission_posted")]
    CommissionPosted { commission: Commission },
    /// Units were delivered to a commission: `delivered` is how many so far
    #[serde(rename = "commission_updated")]
    CommissionUpdated { commission: Commission },
    /// A commission came down, filled, cancelled or expired (its `status`
    /// says which)
    #[serde(rename = "commission_closed")]
    CommissionClosed { commission: Commission },
    // Commons S2.1: to the poster. One that happened while they were away is
    // sent as they come back.
    /// `deliverer_name` delivered `units` to your commission and was paid
    /// `paid` from its escrow. The units are in your market box, at the
    /// Exchange hall, or wait in the court for room there. `left` 0 means
    /// it's filled.
    #[serde(rename = "commission_delivered")]
    CommissionDelivered {
        commission_id: Uuid,
        item_type: String,
        deliverer_id: Uuid,
        deliverer_name: String,
        /// The deliverer is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        deliverer_citizen: bool,
        units: Units,
        paid: i64,
        left: i32,
    },
    /// Your commission ran past its due date with `delivered` of `quantity`
    /// in, and `refunded` of its escrow came back to your balance
    #[serde(rename = "commission_expired")]
    CommissionExpired {
        commission_id: Uuid,
        item_type: String,
        delivered: i32,
        quantity: i32,
        refunded: i64,
    },
    // Commons S2.2: claims (D6). Each is queued, so one that happened while
    // you were away is sent as you come back.
    /// To the poster: `claimant_name` promised `quantity` of your commission
    /// by `until`, and that share is held for them
    #[serde(rename = "commission_claimed")]
    CommissionClaimed {
        commission_id: Uuid,
        claim_id: Uuid,
        item_type: String,
        claimant_id: Uuid,
        claimant_name: String,
        /// The claimant is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        claimant_citizen: bool,
        quantity: i32,
        until: DateTime<Utc>,
    },
    /// To the claimant: an hour or less is left on your claim, with
    /// `delivered` of `quantity` in
    #[serde(rename = "claim_due_soon")]
    ClaimDueSoon {
        commission_id: Uuid,
        claim_id: Uuid,
        item_type: String,
        quantity: i32,
        delivered: i32,
        until: DateTime<Utc>,
    },
    /// To the poster and the claimant: a claim ran out with `delivered` of
    /// `quantity` in, or was released late. What it held is open again, and
    /// it's a breach on the claimant's record.
    #[serde(rename = "claim_lapsed")]
    ClaimLapsed {
        commission_id: Uuid,
        claim_id: Uuid,
        item_type: String,
        claimant_id: Uuid,
        claimant_name: String,
        /// The claimant is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        claimant_citizen: bool,
        quantity: i32,
        delivered: i32,
    },
    // Commons S4.1: guilds
    /// Your guild's roster changed, or you joined, left or were expelled:
    /// read it again with `GET /player/guild`.
    #[serde(rename = "guild_changed")]
    GuildChanged { guild_id: Uuid },
    /// Something in a guild that names you (Commons D10). One sent while you
    /// were away is sent as you come back, once. `who` is who did it: who
    /// invited you, who applied, who expelled you, or who held the lead.
    #[serde(rename = "guild_notice")]
    GuildNotice {
        guild_id: Uuid,
        guild: String,
        tag: String,
        kind: GuildNoticeKind,
        who: Uuid,
        name: String,
        /// `who` is a citizen (C0.1)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        citizen: bool,
    },
    // Commons S4.2: the treasury's dues
    /// To a guild's officers and leader: its debt for dues has passed
    /// another week. At `dissolves_at` it's dissolved. One sent while you
    /// were away is sent as you come back.
    #[serde(rename = "guild_dues")]
    GuildDues {
        guild_id: Uuid,
        guild: String,
        tag: String,
        owed: i64,
        dissolves_at: i64,
    },
    /// To each member: the guild was dissolved for `owed` in unpaid dues
    /// (or closed as its last member left, with `owed` 0), and `share` of
    /// what its treasury held is yours. Sent as you come back if you were away.
    #[serde(rename = "guild_dissolved")]
    GuildDissolved {
        guild_id: Uuid,
        guild: String,
        tag: String,
        owed: i64,
        share: i64,
    },
    // Commons S2.6: wages (D7). The first three go to everyone, so the
    // board's Work page stays live.
    /// A wage offer went up
    #[serde(rename = "wage_offered")]
    WageOffered { offer: WageOffer },
    /// An offer was taken, worked or a shift on it ended: `made` and `shift`
    /// say where it stands
    #[serde(rename = "wage_updated")]
    WageUpdated { offer: WageOffer },
    /// An offer came down, filled, withdrawn, expired or gone (its `status`
    /// says which)
    #[serde(rename = "wage_closed")]
    WageClosed { offer: WageOffer },
    /// To the employer: someone took your offer and started a shift. One
    /// that happened while you were away is sent as you come back.
    #[serde(rename = "wage_taken")]
    WageTaken { offer_id: Uuid, shift: WageShift },
    /// To the employer and the worker: a shift ended, having made `shift.made`
    /// and been paid `paid`, for `reason`. One that ended while you were away
    /// is sent as you come back.
    #[serde(rename = "wage_shift_ended")]
    WageShiftEnded { offer_id: Uuid, shift: WageShift, paid: i64 },
    /// To the worker, quietly: units made on your shift and paid for out of
    /// the escrow, in the transaction that made them. `made` is the offer's
    /// total so far, of `cap`.
    #[serde(rename = "wage_paid")]
    WagePaid {
        offer_id: Uuid,
        item_type: String,
        units: Units,
        paid: i64,
        made: i32,
        cap: i32,
    },
    // The road in (#438): the tutorial
    /// Where you are on the road in: sent after `auth_ok`, and to each of
    /// your sockets when you hide the card or bring it back (plan D2)
    #[serde(rename = "tutorial")]
    Tutorial {
        /// The steps done, by name, in the tour's order
        done: Steps,
        /// The card is hidden for now, on every device
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dismissed: bool,
        /// The deed the road in earns (#448), left out while unearned. Sent
        /// again when a claim spends it.
        #[serde(default, skip_serializing_if = "crate::tutorial::Grant::is_unearned")]
        grant: crate::tutorial::Grant,
    },
    /// You did `step` (#439), to each of your sockets. `done` is every step
    /// done now; when it holds them all, the tour is finished.
    #[serde(rename = "tutorial_step")]
    TutorialStep { step: Step, done: Steps },
}

/// What a [`ServerMessage::GuildNotice`] tells you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuildNoticeKind {
    /// You're invited to join
    Invited,
    /// Someone applied to your guild (to its officers and leader)
    Applied,
    /// You were expelled
    Expelled,
    /// The lead is yours: handed on, or passed to you after the leader's
    /// subscription lapsed four weeks
    Lead,
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
            ServerMessage::CraftingStarted { .. } => "crafting_started",
            ServerMessage::CraftingProgress { .. } => "crafting_progress",
            ServerMessage::CraftingCompleted { .. } => "crafting_completed",
            ServerMessage::CraftingCancelled { .. } => "crafting_cancelled",
            ServerMessage::CraftingFailed { .. } => "crafting_failed",
            ServerMessage::InventoryUpdated { .. } => "inventory_updated",
            ServerMessage::StationPlaced { .. } => "station_placed",
            ServerMessage::StationRemoved { .. } => "station_removed",
            ServerMessage::StationOpened { .. } => "station_opened",
            ServerMessage::StationClosed { .. } => "station_closed",
            ServerMessage::StationToolChanged { .. } => "station_tool_changed",
            ServerMessage::StationAutomationChanged { .. } => "station_automation_changed",
            ServerMessage::AutomationProduced { .. } => "automation_produced",
            ServerMessage::AutomationPaused { .. } => "automation_paused",
            ServerMessage::TradeRequested { .. } => "trade_requested",
            ServerMessage::TradeRequestDeclined { .. } => "trade_request_declined",
            ServerMessage::TradeRequestExpired { .. } => "trade_request_expired",
            ServerMessage::TradeStarted { .. } => "trade_started",
            ServerMessage::TradeOfferUpdated { .. } => "trade_offer_updated",
            ServerMessage::TradeAccepted { .. } => "trade_accepted",
            ServerMessage::TradeExecuted { .. } => "trade_executed",
            ServerMessage::TradeCancelled { .. } => "trade_cancelled",
            ServerMessage::CurrencyChanged { .. } => "currency_changed",
            ServerMessage::ToolBroke { .. } => "tool_broke",
            ServerMessage::ZoneChanged { .. } => "zone_changed",
            ServerMessage::MovementBlocked { .. } => "movement_blocked",
            ServerMessage::TerrainUpdated => "terrain_updated",
            ServerMessage::PlotClaimed { .. } => "plot_claimed",
            ServerMessage::PlotReleased { .. } => "plot_released",
            ServerMessage::TaxDelinquent { .. } => "tax_delinquent",
            ServerMessage::PlotForeclosed { .. } => "plot_foreclosed",
            ServerMessage::StationDisplaced { .. } => "station_displaced",
            ServerMessage::StationCondition { .. } => "station_condition",
            ServerMessage::StationCollapsed { .. } => "station_collapsed",
            ServerMessage::SubscriptionLapsed { .. } => "subscription_lapsed",
            ServerMessage::OrderPlaced { .. } => "order_placed",
            ServerMessage::OrderFilled { .. } => "order_filled",
            ServerMessage::OrderCancelled { .. } => "order_cancelled",
            ServerMessage::MarketDepth { .. } => "market_depth",
            ServerMessage::MarketTrade { .. } => "market_trade",
            ServerMessage::BazaarSale { .. } => "bazaar_sale",
            ServerMessage::StallLapsed { .. } => "stall_lapsed",
            ServerMessage::GazettePublished { .. } => "gazette_published",
            ServerMessage::FeesChanged { .. } => "fees_changed",
            ServerMessage::Presence { .. } => "presence",
            ServerMessage::Resync { .. } => "resync",
            ServerMessage::Said { .. } => "said",
            ServerMessage::LetterArrived { .. } => "letter_arrived",
            ServerMessage::CommissionPosted { .. } => "commission_posted",
            ServerMessage::CommissionUpdated { .. } => "commission_updated",
            ServerMessage::CommissionClosed { .. } => "commission_closed",
            ServerMessage::CommissionDelivered { .. } => "commission_delivered",
            ServerMessage::CommissionExpired { .. } => "commission_expired",
            ServerMessage::CommissionClaimed { .. } => "commission_claimed",
            ServerMessage::ClaimDueSoon { .. } => "claim_due_soon",
            ServerMessage::ClaimLapsed { .. } => "claim_lapsed",
            ServerMessage::GuildChanged { .. } => "guild_changed",
            ServerMessage::GuildNotice { .. } => "guild_notice",
            ServerMessage::GuildDues { .. } => "guild_dues",
            ServerMessage::GuildDissolved { .. } => "guild_dissolved",
            ServerMessage::WageOffered { .. } => "wage_offered",
            ServerMessage::WageUpdated { .. } => "wage_updated",
            ServerMessage::WageClosed { .. } => "wage_closed",
            ServerMessage::WageTaken { .. } => "wage_taken",
            ServerMessage::WageShiftEnded { .. } => "wage_shift_ended",
            ServerMessage::WagePaid { .. } => "wage_paid",
            ServerMessage::Tutorial { .. } => "tutorial",
            ServerMessage::TutorialStep { .. } => "tutorial_step",
        }
    }
}

/// A tool used by a gather or a craft, after the use (E1.5). `durability` 0 means
/// that use broke it, and `tool_broke` follows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolUse {
    pub item_id: Uuid,
    pub item_type: String,
    pub durability: i32,
    pub max_durability: i32,
}

/// So many of an item type, as `plot_foreclosed` counts what went to the
/// impound (#141).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ItemCount {
    pub item_type: String,
    pub quantity: i32,
}

/// Where a line is said (Commons D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Heard by those within [`crate::rules::talk::HEARD`] of the speaker
    Local,
    /// Heard by everyone in the world
    Capitol,
    /// Heard by the speaker's guild's members wherever they are, watchers
    /// too, and no one else (S4.4). Someone in no guild can't say it.
    Guild,
}

/// Who holds a player's avatar, in [`ServerMessage::Presence`] (E0.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceHolder {
    /// One of the player's own clients, signed in with a session
    Client,
    /// A bot, on one of the player's API keys
    Bot,
}

/// A player's position, as broadcast in `positions` and `player_joined`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerPosition {
    pub id: Uuid,
    pub name: String,
    pub x: f64,
    pub y: f64,
    /// A citizen, an AI player (C0.1, Citizens D3), not a person. Absent for
    /// a person, so `positions` grows only by the citizens in it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
    /// Their guild's tag (S4.4), shown after the name over their head.
    /// Absent for someone in no guild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// One line of a trade offer, as the server reports it in `trade_offer_updated`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeItemInfo {
    pub item_id: Uuid,
    pub item_type: String,
    pub item_name: String,
    pub quantity: i32,
    /// The mean of `units`, rounded down
    pub quality: i32,
    /// Exactly the units offered (E1.8), which the trade hands over
    #[serde(default)]
    pub units: Units,
    /// Who made the stack, if every unit of it has the one maker (#242)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker: Option<Maker>,
}
