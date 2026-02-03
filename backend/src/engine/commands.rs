use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum Command {
    Move {
        player_id: Uuid,
        dest_x: f64,
        dest_y: f64,
    },
    StopMoving {
        player_id: Uuid,
    },
    StartExtraction {
        player_id: Uuid,
        node_id: Uuid,
    },
    CancelExtraction {
        player_id: Uuid,
    },
    // M3: Crafting commands
    StartCrafting {
        player_id: Uuid,
        recipe_id: String,
        input_item_ids: Vec<Uuid>,
    },
    CancelCrafting {
        player_id: Uuid,
    },
    // M3.5: Inventory commands
    MoveItem {
        player_id: Uuid,
        item_id: Uuid,
        target_container_id: Uuid,
        target_slot: i32,
    },
    // M4: Station commands
    PlaceStation {
        player_id: Uuid,
        station_type: String,
        x: f64,
        y: f64,
        kit_item_id: Uuid,
    },
    RemoveStation {
        player_id: Uuid,
        station_id: Uuid,
    },
    OpenStation {
        player_id: Uuid,
        station_id: Uuid,
    },
    CloseStation {
        player_id: Uuid,
    },
    CraftAtStation {
        player_id: Uuid,
        station_id: Uuid,
        recipe_id: String,
        input_item_ids: Vec<Uuid>,
    },
}
