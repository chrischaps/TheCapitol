use uuid::Uuid;

use super::tools::HeldTool;

#[derive(Debug, Clone)]
pub enum Command {
    Move {
        player_id: Uuid,
        dest_x: f64,
        dest_y: f64,
    },
    StartExtraction {
        player_id: Uuid,
        node_id: Uuid,
        /// The best tool held for it, looked up before queueing (`tools::tool_for_node`)
        tool: Option<HeldTool>,
    },
    CancelExtraction {
        player_id: Uuid,
    },
    // M3: Crafting commands
    StartCrafting {
        player_id: Uuid,
        recipe_id: String,
        #[allow(dead_code)] // crafting picks its own inputs (#183)
        input_item_ids: Vec<Uuid>,
        /// The best tool held for it, looked up before queueing (`tools::tool_for_recipe`)
        tool: Option<HeldTool>,
    },
    CancelCrafting {
        player_id: Uuid,
    },
    // M4: Station commands (the rest are handled in the WebSocket handler)
    CraftAtStation {
        player_id: Uuid,
        station_id: Uuid,
        recipe_id: String,
        #[allow(dead_code)] // crafting picks its own inputs (#183)
        input_item_ids: Vec<Uuid>,
        /// The best tool held for it (`tools::tool_for_recipe`). E1.6 (#158) adds the station's own
        tool: Option<HeldTool>,
    },
    // M5: Trading commands
    InitiateTrade {
        player_id: Uuid,
        target_player_id: Uuid,
    },
    AcceptTradeRequest {
        player_id: Uuid,
    },
    DeclineTradeRequest {
        player_id: Uuid,
    },
    UpdateTradeOffer {
        player_id: Uuid,
        /// Validated against the player's items, with each line's type, name and quality
        items: Vec<capitol_protocol::ws::TradeItemInfo>,
        strands: i64,
    },
    CancelTrade {
        player_id: Uuid,
    },
}
