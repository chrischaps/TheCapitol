//! Finishing a craft in the database (B10, #61).
//!
//! A craft's inputs are checked when it starts, and nothing stops them moving during the
//! seconds it runs: a trade, a merge or a split can all reach them. So at completion, in one
//! transaction, every input is checked again (it still exists, the crafter owns it, it holds
//! enough), the inputs are consumed and the output is written. If anything is wrong the craft
//! fails and nothing changes.
//!
//! A craft takes the very units its quality was worked out from (E1.8): the start names them
//! (the best of each stack first), and completion takes exactly those, or fails.
//!
//! A craft by hand puts its output in the crafter's own crafting output. A craft at a station
//! puts it in the station's box, as the crafter's (#489): it stays at the bench, where it was
//! made, and the crafter takes it from there as from any slot. It joins only a stack of its type
//! the crafter owns, since a box holds other people's things too (#80). If the box has no room,
//! or the station came down while it ran, the craft fails and nothing changes.
//!
//! A craft on a wage shift (Commons D7) takes its inputs from the station's box rather than the
//! crafter's own, puts its output back in that box as the employer's, marked as the worker's
//! make, and pays for it out of the offer's escrow (`wages::pay`), all in the same transaction.
//! If the shift ended while it ran, nothing changes.

use capitol_protocol::ledger::UnitFlow;
use capitol_protocol::rest::StationTool;
use capitol_protocol::rules::stacking::Units;
use capitol_protocol::ws::ToolUse;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::gazette;
use super::ledger::{self, Flows};
use super::station_tools;
use super::units::{self, Marked, NewStack};
use super::tools::{self, BrokenTool, HeldTool, Wear};
use super::wages::{self, ShiftWork};

/// A craft that has run its time, waiting to be written
#[derive(Debug, Clone)]
pub struct FinishedCraft {
    pub player_id: Uuid,
    pub operation_id: Uuid,
    /// The recipe it ran, for the books' count of crafts (D2.4)
    pub recipe_id: String,
    /// (item_id, the units to consume from it)
    pub inputs: Vec<(Uuid, Units)>,
    pub outputs: Vec<CraftOutput>,
    /// The tool the recipe needs, as it was at the start (E1.5). It wears once.
    pub tool: Option<HeldTool>,
    /// The station it was made at: its output goes into that station's box
    /// (#489). `None` by hand.
    pub station_id: Option<Uuid>,
    /// The wage shift it was made on (Commons D7): its inputs are from the
    /// station's box, its output goes back there as the employer's, and it's
    /// paid for in the same transaction
    pub wage: Option<ShiftWork>,
}

/// A craft that was written.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    /// The containers that changed, each once, for `inventory_updated`
    pub changed: Vec<Uuid>,
    /// The tool it wore, and what it has left, for `crafting_completed`
    pub tool: Option<ToolUse>,
    /// Set when that use broke the tool, for `tools::announce_broken`
    pub broken: Option<BrokenTool>,
    /// When the tool was a station's (E1.6): the station, and what its slot
    /// holds now, for `station_tool_changed`
    pub station_tool: Option<(Uuid, Option<StationTool>)>,
    /// On a wage shift, what its pay has to tell people (`wages::announce`)
    pub wage: Option<wages::News>,
}

#[derive(Debug, Clone)]
pub struct CraftOutput {
    pub item_type: String,
    pub item_name: String,
    pub quantity: i32,
    pub quality: i32,
    /// An unstackable output (a kit, a tool) takes a slot of its own.
    pub stackable: bool,
    /// A tool's durability when new (`ItemType::durability_at`); `None` otherwise.
    pub durability: Option<i32>,
}

/// The reason shown when an input moved, split or changed hands while the craft ran
pub const INPUTS_MOVED: &str = "The craft's inputs moved before it finished";
/// The reason shown when the recipe's tool left the inventory while the craft ran
pub const TOOL_MOVED: &str = "The craft's tool left your inventory before it finished";
/// The reason shown when the station's tool was taken out while the craft ran
pub const STATION_TOOL_MOVED: &str = "The station's tool was taken out before the craft finished";
/// The reason shown when the output slot holds something else
pub const OUTPUT_FULL: &str = "Your crafting output is full. Empty it and craft again";
/// The reason shown when a station's box has no room for what it made (#489)
pub const BOX_FULL: &str = "The station's box is full. Make room in it and craft again";
/// The reason shown when the station came down while the craft ran
pub const STATION_GONE: &str = "The station was taken down before the craft finished";

/// Consume the inputs, wear the tool and write the outputs, together or not at all. The
/// `Err` is the reason for `crafting_failed`.
pub async fn finish_craft(db: &PgPool, craft: &FinishedCraft) -> Result<Finished, String> {
    let mut tx = db.begin().await.map_err(db_error)?;
    let mut changed: Vec<Uuid> = Vec::new();
    // What it used up, made and wore out, counted once it's written (C1.7)
    let mut flows = Flows::new();
    // On a wage shift, the station's box: where the inputs are and the output goes
    let station_box = match &craft.wage {
        Some(work) => Some(station_box(&mut tx, work).await?),
        None => None,
    };
    // At a station, its box is where the output goes, the crafter's (#489)
    let output_box = match (station_box, craft.station_id) {
        (Some(b), _) => Some(b),
        (None, Some(station_id)) => Some(box_of(&mut tx, station_id).await?.ok_or_else(|| STATION_GONE.to_string())?),
        (None, None) => None,
    };

    consume_inputs(&mut tx, craft, station_box.map(|(id, _)| id), &mut changed, &mut flows).await?;
    let (mut tool, mut broken, mut station_tool) = (None, None, None);
    if let Some(held) = &craft.tool {
        let wear = match held.station_id {
            // A station's own tool wears while it's still in the slot (E1.6)
            Some(station_id) => {
                let Some((wear, now)) =
                    station_tools::wear_installed(&mut tx, station_id, held.item_id).await.map_err(db_error)?
                else {
                    tracing::info!("Craft {} by {} failed: station {}'s tool {} was taken out",
                        craft.operation_id, craft.player_id, station_id, held.item_id);
                    return Err(STATION_TOOL_MOVED.to_string());
                };
                station_tool = Some((station_id, now));
                wear
            }
            None => {
                let Some(wear) = tools::wear_held(&mut tx, held.item_id, craft.player_id).await.map_err(db_error)? else {
                    tracing::info!("Craft {} by {} failed: tool {} moved", craft.operation_id, craft.player_id, held.item_id);
                    return Err(TOOL_MOVED.to_string());
                };
                wear
            }
        };
        flows.wore(&held.item_type, &wear);
        tool = Some(held.after(&wear));
        if let Wear::Broke(b) = wear {
            broken = Some(b);
        }
    }
    for output in &craft.outputs {
        let container = match output_box {
            Some((box_id, slot_count)) => {
                let owner = craft.wage.as_ref().map_or(craft.player_id, |work| work.employer_id);
                place_made(&mut tx, box_id, slot_count, owner, craft.player_id, output, BOX_FULL).await?;
                box_id
            }
            None => place_output(&mut tx, craft.player_id, output).await?,
        };
        if !changed.contains(&container) {
            changed.push(container);
        }
        gazette::tally(&mut tx, craft.player_id, &output.item_type, output.quantity).await.map_err(db_error)?;
        flows.add(&output.item_type, UnitFlow::Crafted, output.quantity as i64);
    }
    // A craft's outputs all come out at one quality
    if let Some(first) = craft.outputs.first() {
        let made = craft.outputs.iter().map(|o| o.quantity as i64).sum();
        flows.crafted(&craft.recipe_id, made, first.quality);
    }
    let mut wage = None;
    if let (Some(work), Some(first)) = (&craft.wage, craft.outputs.first()) {
        let made = craft.outputs.iter().fold(Units::new(), |mut all, o| {
            all.merge(&Units::uniform(o.quantity, o.quality));
            all
        });
        match wages::pay(&mut tx, work, &first.item_type, &made, chrono::Utc::now()).await {
            Ok(Ok(news)) => wage = Some(news),
            Ok(Err(why)) => return Err(why.to_string()),
            Err(e) => {
                tracing::error!("Craft completion: paying wages: {}", e.message());
                return Err("The craft couldn't be saved. Try again".to_string());
            }
        }
    }

    ledger::commit(tx, flows).await.map_err(db_error)?;
    Ok(Finished { changed, tool, broken, station_tool, wage })
}

/// The box of the station the shift `work` is at, and its slot count.
async fn station_box(tx: &mut Transaction<'_, Postgres>, work: &ShiftWork) -> Result<(Uuid, i32), String> {
    let capitol_protocol::rules::wages::Work::Craft { station_id, .. } = &work.work else {
        return Err(wages::SHIFT_OVER.to_string());
    };
    box_of(tx, *station_id).await?.ok_or_else(|| wages::SHIFT_OVER.to_string())
}

/// The box of `station_id` and its slot count, or `None` once it's gone.
async fn box_of(tx: &mut Transaction<'_, Postgres>, station_id: Uuid) -> Result<Option<(Uuid, i32)>, String> {
    sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c JOIN container_types ct ON ct.id = c.container_type
         WHERE c.station_id = $1",
    )
    .bind(station_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)
}

/// Check every input again and use it up. Rows are locked as they're read, so a move or trade
/// running at the same moment waits for this transaction rather than slipping between. Each is
/// still the crafter's, or with `station_box` (a wage shift), still in that box, whoever's it is.
async fn consume_inputs(
    tx: &mut Transaction<'_, Postgres>,
    craft: &FinishedCraft,
    station_box: Option<Uuid>,
    changed: &mut Vec<Uuid>,
    flows: &mut Flows,
) -> Result<(), String> {
    // The same stack listed twice must hold both lots
    let mut wanted: Vec<(Uuid, Units)> = Vec::new();
    for (id, lot) in &craft.inputs {
        match wanted.iter_mut().find(|(w, _)| w == id) {
            Some((_, all)) => {
                all.merge(lot);
            }
            None => wanted.push((*id, lot.clone())),
        }
    }

    for (item_id, lot) in wanted {
        let held: Option<(Option<Uuid>, String)> = match station_box {
            None => sqlx::query_as("SELECT container_id, item_type FROM items WHERE id = $1 AND owner_id = $2 FOR UPDATE")
                .bind(item_id)
                .bind(craft.player_id),
            Some(box_id) => {
                sqlx::query_as("SELECT container_id, item_type FROM items WHERE id = $1 AND container_id = $2 FOR UPDATE")
                    .bind(item_id)
                    .bind(box_id)
            }
        }
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_error)?;
        let taken = match held {
            Some(_) => units::take_units(tx, item_id, &lot).await.map_err(db_error)?,
            None => None,
        };
        let (Some((container_id, item_type)), Some(_)) = (held, taken) else {
            tracing::info!(
                "Craft {} by {} failed: input {} is gone, or no longer {}'s, or no longer holds {:?}",
                craft.operation_id, craft.player_id, item_id, craft.player_id, lot
            );
            return Err(INPUTS_MOVED.to_string());
        };
        flows.add(&item_type, UnitFlow::Consumed, lot.count());
        if let Some(c) = container_id {
            if !changed.contains(&c) {
                changed.push(c);
            }
        }
    }
    Ok(())
}

/// Put `output` in the crafter's output container ([`place_in`]). Returns the container.
async fn place_output(
    tx: &mut Transaction<'_, Postgres>,
    player_id: Uuid,
    output: &CraftOutput,
) -> Result<Uuid, String> {
    let (container_id, slot_count): (Uuid, i32) = sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.owner_id = $1 AND c.container_type = 'crafting_output'
         LIMIT 1",
    )
    .bind(player_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?
    .ok_or_else(|| {
        tracing::error!("No crafting_output container for player {}", player_id);
        "You have no crafting output".to_string()
    })?;
    place_in(tx, container_id, slot_count, player_id, output).await?;
    Ok(container_id)
}

/// Put `output`, owned by `owner_id`, in `container_id` of `slot_count` slots:
/// onto the first stack of its type `owner_id` owns if it stacks
/// (`units::join_own_stack`: every unit keeps its quality, so any grade may
/// join), or into the first free slot. [`OUTPUT_FULL`] when there's neither.
/// Its units carry `owner_id`'s mark: they made it (#231).
pub async fn place_in(
    tx: &mut Transaction<'_, Postgres>,
    container_id: Uuid,
    slot_count: i32,
    owner_id: Uuid,
    output: &CraftOutput,
) -> Result<(), String> {
    place_made(tx, container_id, slot_count, owner_id, owner_id, output, OUTPUT_FULL).await
}

/// [`place_in`], made by `maker`, and `full` when there's no room: on a wage
/// shift the output is the employer's, and its units carry the worker's mark
/// (Commons D7). Only a stack `owner_id` owns takes it, as a station's box
/// holds other people's too (#80, #489).
async fn place_made(
    tx: &mut Transaction<'_, Postgres>,
    container_id: Uuid,
    slot_count: i32,
    owner_id: Uuid,
    maker: Uuid,
    output: &CraftOutput,
    full: &str,
) -> Result<(), String> {
    let made = Marked::by(Some(maker), &Units::uniform(output.quantity, output.quality));
    if output.stackable
        && units::join_own_stack(tx, container_id, &output.item_type, owner_id, &made).await.map_err(db_error)?
    {
        return Ok(());
    }

    let used: Vec<(i32,)> = sqlx::query_as(
        "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL",
    )
    .bind(container_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(db_error)?;
    let slot = (0..slot_count)
        .find(|s| !used.iter().any(|(u,)| u == s))
        .ok_or_else(|| full.to_string())?;

    let stack = NewStack {
        item_type: &output.item_type,
        owner_id: Some(owner_id),
        container_id: Some(container_id),
        slot_index: Some(slot),
        durability: output.durability,
    };
    units::insert_stack(tx, stack, &made).await.map_err(db_error)?;
    Ok(())
}

fn db_error(e: sqlx::Error) -> String {
    tracing::error!("Craft completion: database error: {:?}", e);
    "The craft couldn't be saved. Try again".to_string()
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh database that
/// `sqlx::test` creates from the migrations and drops afterwards. Run them with
/// `cargo test craft_completion -- --ignored`. The crafter and the rope are
/// shared with the tick's tests.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Crafter {
        pub(crate) id: Uuid,
        inventory: Uuid,
        pub(crate) input: Uuid,
        output: Uuid,
    }

    pub(crate) async fn crafter(db: &PgPool, name: &str) -> Crafter {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(account)
            .bind(format!("{name}@test"))
            .execute(db)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(account)
            .bind(name)
            .execute(db)
            .await
            .unwrap();
        let mut containers = Vec::new();
        for kind in ["player_inventory", "crafting_input", "crafting_output"] {
            let c = Uuid::new_v4();
            sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, $2, $3)")
                .bind(c)
                .bind(kind)
                .bind(id)
                .execute(db)
                .await
                .unwrap();
            containers.push(c);
        }
        Crafter { id, inventory: containers[0], input: containers[1], output: containers[2] }
    }

    pub(crate) async fn stack(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, item_type: &str, qty: i32) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, $2, 60, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(item_type)
        .bind(qty)
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
        id
    }

    /// Basic rope: 2 fiber at 60, 1 rope
    pub(crate) fn rope(player: &Crafter, fiber: Uuid) -> FinishedCraft {
        FinishedCraft {
            player_id: player.id,
            operation_id: Uuid::new_v4(),
            recipe_id: "rope_basic".into(),
            inputs: vec![(fiber, Units::uniform(2, 60))],
            outputs: vec![CraftOutput {
                item_type: "rope".into(),
                item_name: "Rope".into(),
                quantity: 1,
                quality: 60,
                stackable: true,
                durability: None,
            }],
            tool: None,
            station_id: None,
            wage: None,
        }
    }

    /// (owner, container, quantity) of an item, or `None` once it's gone
    async fn item(db: &PgPool, id: Uuid) -> Option<(Uuid, Uuid, i32)> {
        sqlx::query_as("SELECT owner_id, container_id, quantity FROM items WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await
            .unwrap()
    }

    /// The rope in `player`'s output, if any
    async fn rope_made(db: &PgPool, player: &Crafter) -> i32 {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT quantity FROM items WHERE container_id = $1 AND item_type = 'rope'",
        )
        .bind(player.output)
        .fetch_optional(db)
        .await
        .unwrap();
        row.map_or(0, |(q,)| q)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_craft_turns_its_inputs_into_its_output(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 5).await;

        let changed = finish_craft(&db, &rope(&ada, fiber)).await.unwrap().changed;

        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(3), "two fiber used");
        assert_eq!(rope_made(&db, &ada).await, 1);
        assert_eq!(changed, vec![ada.input, ada.output], "each changed container once");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_last_of_a_stack_is_deleted(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;

        finish_craft(&db, &rope(&ada, fiber)).await.unwrap();

        assert_eq!(item(&db, fiber).await, None);
        assert_eq!(rope_made(&db, &ada).await, 1);
    }

    /// A chest owned by `owner`, standing in for any station: finishing a
    /// craft doesn't look at its type. Returns the station and its box.
    async fn station(db: &PgPool, owner: Uuid) -> (Uuid, Uuid) {
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
             VALUES ($1, 'storage_chest', $2, 0, 0, 50)",
        )
        .bind(station)
        .bind(owner)
        .execute(db)
        .await
        .unwrap();
        let the_box = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2)")
            .bind(the_box)
            .bind(station)
            .execute(db)
            .await
            .unwrap();
        (station, the_box)
    }

    /// The rope stacks in `container`, as (owner, quantity), in slot order
    async fn ropes_in(db: &PgPool, container: Uuid) -> Vec<(Uuid, i32)> {
        sqlx::query_as("SELECT owner_id, quantity FROM items WHERE container_id = $1 AND item_type = 'rope' ORDER BY slot_index")
            .bind(container)
            .fetch_all(db)
            .await
            .unwrap()
    }

    /// Made at a station, it stays there: in the station's box, the
    /// crafter's, from inputs in that same box (#489)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_station_craft_lands_in_the_stations_box(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let (bench, the_box) = station(&db, ada.id).await;
        let fiber = stack(&db, ada.id, the_box, 0, "fiber", 2).await;
        let mut craft = rope(&ada, fiber);
        craft.station_id = Some(bench);

        let changed = finish_craft(&db, &craft).await.unwrap().changed;

        assert_eq!(item(&db, fiber).await, None, "the fiber used up");
        assert_eq!(ropes_in(&db, the_box).await, vec![(ada.id, 1)]);
        assert_eq!(rope_made(&db, &ada).await, 0, "nothing in the crafting output");
        assert_eq!(changed, vec![the_box]);
    }

    /// A guest's craft at someone else's station never joins the owner's
    /// stack, which would make it the owner's: it takes a slot of its own,
    /// and a second one joins it
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_guests_station_craft_keeps_to_their_own_stack(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let bram = crafter(&db, "bram").await;
        let (bench, the_box) = station(&db, ada.id).await;
        stack(&db, ada.id, the_box, 0, "rope", 3).await;
        let fiber = stack(&db, bram.id, bram.inventory, 0, "fiber", 4).await;
        let mut craft = rope(&bram, fiber);
        craft.station_id = Some(bench);

        finish_craft(&db, &craft).await.unwrap();
        assert_eq!(ropes_in(&db, the_box).await, vec![(ada.id, 3), (bram.id, 1)]);
        finish_craft(&db, &craft).await.unwrap();
        assert_eq!(ropes_in(&db, the_box).await, vec![(ada.id, 3), (bram.id, 2)]);
    }

    /// A full box fails the craft and keeps its inputs, as a full output does
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_station_box_fails_the_craft(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let (bench, the_box) = station(&db, ada.id).await;
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'station_inventory'")
            .fetch_one(&db)
            .await
            .unwrap();
        for slot in 0..slots {
            stack(&db, ada.id, the_box, slot, "fiber", 1).await;
        }
        let fiber = stack(&db, ada.id, ada.inventory, 0, "fiber", 2).await;
        let mut craft = rope(&ada, fiber);
        craft.station_id = Some(bench);

        assert_eq!(finish_craft(&db, &craft).await, Err(BOX_FULL.to_string()));
        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(2), "inputs rolled back");
    }

    /// Taken down mid-craft: the craft fails and nothing is used up
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_station_taken_down_mid_craft_fails_it(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let (bench, _) = station(&db, ada.id).await;
        let fiber = stack(&db, ada.id, ada.inventory, 0, "fiber", 2).await;
        sqlx::query("DELETE FROM stations WHERE id = $1").bind(bench).execute(&db).await.unwrap();
        let mut craft = rope(&ada, fiber);
        craft.station_id = Some(bench);

        assert_eq!(finish_craft(&db, &craft).await, Err(STATION_GONE.to_string()));
        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(2));
    }

    /// A trade of the whole stack keeps its id and changes its owner
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn fiber_traded_away_mid_craft_fails_the_craft(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let bram = crafter(&db, "bram").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 5).await;
        sqlx::query("UPDATE items SET owner_id = $1, container_id = $2, slot_index = 0 WHERE id = $3")
            .bind(bram.id)
            .bind(bram.inventory)
            .bind(fiber)
            .execute(&db)
            .await
            .unwrap();

        let result = finish_craft(&db, &rope(&ada, fiber)).await;

        assert_eq!(result, Err(INPUTS_MOVED.to_string()));
        assert_eq!(item(&db, fiber).await, Some((bram.id, bram.inventory, 5)), "Bram's fiber untouched");
        assert_eq!(rope_made(&db, &ada).await, 0, "no rope for Ada");
    }

    /// A merge deletes the source stack and adds it to the target
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn fiber_merged_away_mid_craft_fails_the_craft(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;
        let other = stack(&db, ada.id, ada.inventory, 0, "fiber", 3).await;
        let mut conn = db.acquire().await.unwrap();
        units::merge_into(&mut conn, fiber, other).await.unwrap();
        drop(conn);

        let result = finish_craft(&db, &rope(&ada, fiber)).await;

        assert_eq!(result, Err(INPUTS_MOVED.to_string()));
        assert_eq!(item(&db, other).await.map(|i| i.2), Some(5), "the merged stack untouched");
        assert_eq!(rope_made(&db, &ada).await, 0, "no free rope");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn fiber_split_below_the_recipe_fails_the_craft(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;
        let mut conn = db.acquire().await.unwrap();
        units::take(&mut conn, fiber, 1, Default::default()).await.unwrap().unwrap();
        drop(conn);

        let result = finish_craft(&db, &rope(&ada, fiber)).await;

        assert_eq!(result, Err(INPUTS_MOVED.to_string()));
        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(1));
        assert_eq!(rope_made(&db, &ada).await, 0);
    }

    /// Still Ada's, just somewhere else: the craft goes ahead
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn fiber_moved_to_the_backpack_still_crafts(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.inventory, 4, "fiber", 4).await;

        let changed = finish_craft(&db, &rope(&ada, fiber)).await.unwrap().changed;

        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(2));
        assert_eq!(changed, vec![ada.inventory, ada.output]);
    }

    /// Writing nothing beats losing the output: the inputs stay
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_output_fails_the_craft_and_keeps_the_inputs(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;
        stack(&db, ada.id, ada.output, 0, "fiber", 1).await;

        let result = finish_craft(&db, &rope(&ada, fiber)).await;

        assert_eq!(result, Err(OUTPUT_FULL.to_string()));
        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(2), "inputs rolled back");
        assert_eq!(rope_made(&db, &ada).await, 0);
    }

    fn sickle(player: &Crafter, fiber: Uuid) -> FinishedCraft {
        let mut craft = rope(player, fiber);
        craft.outputs[0] = CraftOutput {
            item_type: "stone_sickle".into(),
            item_name: "Stone Sickle".into(),
            quantity: 1,
            quality: 80,
            stackable: false,
            durability: Some(78),
        };
        craft
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_tool_comes_out_with_its_durability(db: PgPool) {
        crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 4).await;

        finish_craft(&db, &sickle(&ada, fiber)).await.unwrap();

        let made: (i32, i32, Option<i32>) = sqlx::query_as(
            "SELECT quality, quantity, durability FROM items WHERE container_id = $1",
        )
        .bind(ada.output)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(made, (80, 1, Some(78)));
        // A slot of its own, and the crafter's mark on it (#231)
        let sickle = crate::engine::units::test_units::all_marked(&db, ada.id, "stone_sickle").await;
        assert_eq!(sickle, Marked::by(Some(ada.id), &Units::uniform(1, 80)));
    }

    /// A tool doesn't stack, so a sickle left in the output fills it
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_tool_never_stacks_onto_another(db: PgPool) {
        crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 4).await;
        let worn = stack(&db, ada.id, ada.output, 0, "stone_sickle", 1).await;

        let result = finish_craft(&db, &sickle(&ada, fiber)).await;

        assert_eq!(result, Err(OUTPUT_FULL.to_string()));
        assert_eq!(item(&db, worn).await.map(|i| i.2), Some(1), "the old sickle alone");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn rope_stacks_onto_rope_already_in_the_output(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 2).await;
        stack(&db, ada.id, ada.output, 0, "rope", 3).await;

        finish_craft(&db, &rope(&ada, fiber)).await.unwrap();

        assert_eq!(rope_made(&db, &ada).await, 4);
    }

    /// Output joins any stack of its type, and every unit keeps its quality (D2, E1.8): rope
    /// at 60 onto two at 92 is three units, two 92s and a 60, shown as 81
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn output_joins_any_grade_and_keeps_its_own_quality(db: PgPool) {
        use crate::engine::units::test_units::{all_held, stack_of, units};
        let ada = crafter(&db, "ada").await;
        let fiber = stack_of(&db, ada.id, ada.input, 0, "fiber", &Units::from_pairs([(60, 2), (95, 1)])).await;
        let held = stack_of(&db, ada.id, ada.output, 0, "rope", &Units::uniform(2, 92)).await;
        let fiber_before = all_held(&db, ada.id, "fiber").await;

        let mut craft = rope(&ada, fiber);
        finish_craft(&db, &craft).await.unwrap();

        let made = units(&db, held).await;
        assert_eq!(made, Units::from_pairs([(60, 1), (92, 2)]));
        assert_eq!(made.mean(), Some(81));
        // The new rope carries its crafter's mark (#231); the two held before had none
        let marks = crate::engine::units::test_units::marked(&db, held).await;
        assert_eq!(marks, Marked::from_rows([(60, Some(ada.id), 1), (92, None, 2)]));
        // The craft took exactly the two 60s it named, and left the 95
        let mut used = fiber_before.clone();
        used.take_units(&Units::uniform(2, 60)).unwrap();
        assert_eq!(all_held(&db, ada.id, "fiber").await, used);

        // Units the stack doesn't hold any more fail the craft
        craft.inputs = vec![(fiber, Units::uniform(1, 60))];
        assert_eq!(finish_craft(&db, &craft).await.unwrap_err(), INPUTS_MOVED);
        assert_eq!(units(&db, fiber).await, Units::uniform(1, 95));
    }

    /// A stone sickle in `player`'s inventory with `durability` left, as `best_tool` would find it
    async fn held_sickle(db: &PgPool, player: &Crafter, durability: i32) -> HeldTool {
        crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
             VALUES ($1, 'stone_sickle', 50, 1, $2, $3, 9, $4)",
        )
        .bind(id)
        .bind(player.id)
        .bind(player.inventory)
        .bind(durability)
        .execute(db)
        .await
        .unwrap();
        tools::best_tool(db, player.id, capitol_protocol::rules::tools::ToolType::Sickle).await.unwrap().unwrap()
    }

    /// The recipe's tool wears with the craft, and the crafter hears what it has left
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_craft_wears_its_tool(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 4).await;
        let sickle = held_sickle(&db, &ada, 2).await;
        let mut craft = rope(&ada, fiber);
        craft.tool = Some(sickle.clone());

        let done = finish_craft(&db, &craft).await.unwrap();
        let used = done.tool.unwrap();
        assert_eq!((used.item_id, used.durability, used.max_durability), (sickle.item_id, 1, 60));
        assert_eq!(done.broken, None);

        let done = finish_craft(&db, &craft).await.unwrap();
        assert_eq!(done.tool.unwrap().durability, 0);
        assert_eq!(done.broken.unwrap().item_id, sickle.item_id, "the last use breaks it");
        assert_eq!(item(&db, sickle.item_id).await, None);
    }

    /// A tool put away mid-craft fails it, and nothing is used up
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_craft_whose_tool_left_fails(db: PgPool) {
        let ada = crafter(&db, "ada").await;
        let fiber = stack(&db, ada.id, ada.input, 0, "fiber", 4).await;
        let sickle = held_sickle(&db, &ada, 5).await;
        sqlx::query("UPDATE items SET container_id = $1, slot_index = 3 WHERE id = $2")
            .bind(ada.input)
            .bind(sickle.item_id)
            .execute(&db)
            .await
            .unwrap();
        let mut craft = rope(&ada, fiber);
        craft.tool = Some(sickle);

        assert_eq!(finish_craft(&db, &craft).await, Err(TOOL_MOVED.to_string()));
        assert_eq!(item(&db, fiber).await.map(|i| i.2), Some(4), "the fiber stays");
        assert_eq!(rope_made(&db, &ada).await, 0);
    }
}
