//! Taking a station down (#88).
//!
//! REST `DELETE /stations/{id}` and WS `remove_station` both come here. In one
//! transaction the kit that placed it goes back into the owner's inventory at
//! the quality it was placed with, with the tool in its tool slot if it holds
//! one (E1.6), and then what's in its box (#489): each thing to whoever owns
//! it, so the owner's own things and anything with no owner to the owner's
//! inventory, and a guest's deposit (#80) to the guest's, or to the guest's
//! impound when that's full (#141). If the owner's inventory can't hold all of
//! theirs, or a guest's things would have nowhere at all to go, nothing moves
//! and the station stays up. Then the station, its container and its place on
//! the plot's station count all go. Nothing is destroyed: a station can be
//! moved by taking it down and placing it again.
//!
//! A foreclosed plot's stations come down here too ([`return_plot_stations`],
//! #137), boxes and tools and all: what's in them goes back to the inventories
//! of whoever owns it, or to their impound when an inventory is full (#141).

use std::collections::HashMap;

use capitol_protocol::ledger::UnitFlow;
use capitol_protocol::rules::stacking::Units;
use capitol_protocol::ws::ItemCount;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::units::{Marked, NewStack};


/// What a removal changed, so the caller can update the game state and tell clients.
#[derive(Debug, Clone)]
pub struct RemovedStation {
    /// `(player, container)` for each inventory or impound that took
    /// something: the owner's inventory, which holds the kit, first
    pub returned_to: Vec<(Uuid, Uuid)>,
}

/// Why a station can't be taken down.
#[derive(Debug)]
pub enum RemoveError {
    NotFound,
    NotYours,
    /// No room in the owner's inventory for the kit, its tool and their
    /// things in the box.
    NoRoom,
    /// A guest's things in the box (#80) have no room in their inventory or
    /// their impound.
    NoRoomForGuests,
    Database(sqlx::Error),
}

impl RemoveError {
    pub fn message(&self) -> String {
        match self {
            RemoveError::NotFound => "Station not found".into(),
            RemoveError::NotYours => "Not your station".into(),
            RemoveError::NoRoom => {
                "No room in your inventory for the kit, its tool and what's in its box".into()
            }
            RemoveError::NoRoomForGuests => {
                "Someone else's things in its box have nowhere to go. Ask them to take them out".into()
            }
            RemoveError::Database(e) => format!("Database error: {}", e),
        }
    }
}

impl From<sqlx::Error> for RemoveError {
    fn from(e: sqlx::Error) -> Self {
        RemoveError::Database(e)
    }
}

impl From<RemoveError> for crate::error::AppError {
    fn from(e: RemoveError) -> Self {
        use crate::error::AppError;
        match e {
            RemoveError::NotFound => AppError::NotFound(e.message()),
            RemoveError::NotYours => AppError::Forbidden(e.message()),
            RemoveError::NoRoom | RemoveError::NoRoomForGuests => {
                AppError::BadRequest(e.message())
            }
            RemoveError::Database(db) => AppError::Database(db),
        }
    }
}

/// Take `station_id` down for `player_id`, who must own it.
pub async fn remove_station(
    db: &PgPool,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<RemovedStation, RemoveError> {
    let mut tx = db.begin().await?;
    let returned = remove_in(&mut tx, player_id, station_id).await?;
    // The kit comes back into the world (C1.7)
    let mut flows = super::ledger::Flows::new();
    returned.count_into(&mut flows);
    super::ledger::commit(tx, flows).await?;
    Ok(RemovedStation { returned_to: returned.containers })
}

async fn remove_in(
    tx: &mut Transaction<'_, Postgres>,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<ReturnedStations, RemoveError> {
    let (station_type, owner_id, plot_id, kit): (String, Uuid, Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT s.station_type, s.owner_id, s.plot_id, st.kit_item
         FROM stations s LEFT JOIN station_types st ON st.id = s.station_type
         WHERE s.id = $1 FOR UPDATE OF s",
    )
    .bind(station_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RemoveError::NotFound)?;
    if owner_id != player_id {
        return Err(RemoveError::NotYours);
    }
    if kit.is_none() {
        tracing::error!("Station {} has type {} with no kit", station_id, station_type);
        return Err(RemoveError::NotFound);
    }

    // Lock the box before emptying it. A deposit's foreign-key check needs a
    // share lock on this row, so one that started first is emptied below, and
    // one that starts now waits and then fails against the deleted container.
    sqlx::query("SELECT id FROM containers WHERE station_id = $1 FOR UPDATE")
        .bind(station_id)
        .execute(&mut **tx)
        .await?;

    // The kit first, then the tool, then the box, so the owner's own things
    // take their inventory's room before a guest's deposit is reckoned. Kits
    // and tools don't stack, so each takes the first free slot.
    let returned = empty_stations(tx, &[station_id], TAKE_DOWN).await?;
    // What the owner can't carry stays put, and so does the station: a
    // take-down never sends their own things to the impound.
    if !returned.impounded_for(player_id).is_empty() || returned.lost.iter().any(|(p, ..)| *p == player_id) {
        return Err(RemoveError::NoRoom);
    }
    if !returned.lost.is_empty() {
        return Err(RemoveError::NoRoomForGuests);
    }

    // The container goes with the station (ON DELETE CASCADE), and it's empty.
    sqlx::query("DELETE FROM stations WHERE id = $1")
        .bind(station_id)
        .execute(&mut **tx)
        .await?;
    if let Some(plot_id) = plot_id {
        sqlx::query(
            "UPDATE plots SET station_count = station_count - 1
             WHERE id = $1 AND station_count > 0",
        )
        .bind(plot_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(returned)
}

/// The one unit of a kit a station gives back: the quality it was placed at,
/// made by whoever made it (`None`, unmarked)
fn kit_units(quality: i32, maker: Option<Uuid>) -> Marked {
    Marked::by(maker, &Units::from_pairs([(quality, 1)]))
}

/// The first `n` free slots of `player_id`'s inventory, which is locked, or
/// `None` if it has fewer (or they have none).
pub(crate) async fn free_slots(
    conn: &mut PgConnection,
    player_id: Uuid,
    n: usize,
) -> Result<Option<(Uuid, Vec<i32>)>, sqlx::Error> {
    let found: Option<(Uuid, i32)> = sqlx::query_as(
        "SELECT c.id, ct.slot_count FROM containers c
         JOIN container_types ct ON c.container_type = ct.id
         WHERE c.owner_id = $1 AND c.container_type = 'player_inventory'
         LIMIT 1 FOR UPDATE OF c",
    )
    .bind(player_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((inventory_id, slot_count)) = found else {
        return Ok(None);
    };
    let used: Vec<i32> = sqlx::query_scalar(
        "SELECT slot_index FROM items WHERE container_id = $1 AND slot_index IS NOT NULL",
    )
    .bind(inventory_id)
    .fetch_all(&mut *conn)
    .await?;
    let free: Vec<i32> = (0..slot_count).filter(|s| !used.contains(s)).take(n).collect();
    Ok((free.len() == n).then_some((inventory_id, free)))
}

/// What giving a plot's stations back did (#137).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReturnedStations {
    /// `(player, container)` for each inventory or impound that took
    /// something, in the order they first did
    pub containers: Vec<(Uuid, Uuid)>,
    /// What found no room in its owner's inventory and went to their impound
    /// (#141), as `(player, item_type, quantity)`
    pub impounded: Vec<(Uuid, String, i32)>,
    /// What found no room even in the impound, as `(player, item_type,
    /// quantity)`. It's still in the stations, and goes when they do.
    pub lost: Vec<(Uuid, String, i32)>,
    /// The kits made again, one a station that found room for it (C1.7)
    pub kits: Vec<String>,
    /// What of `lost` was in a station's boxes, and is destroyed with it,
    /// as `(item_type, quantity)`. A kit with no room was never made, and a
    /// tool with none stays out of every container, so neither is here.
    pub dropped: Vec<(String, i32)>,
}

impl ReturnedStations {
    /// The units this made and will destroy once the stations are deleted,
    /// for the caller to count when it commits that (C1.7)
    pub fn count_into(&self, flows: &mut super::ledger::Flows) {
        for kit in &self.kits {
            flows.add(kit, UnitFlow::KitReturned, 1);
        }
        for (item_type, quantity) in &self.dropped {
            flows.add(item_type, UnitFlow::Lost, *quantity as i64);
        }
    }

    /// What of `player`'s went to their impound, a count per item type in
    /// the order each first went
    pub fn impounded_for(&self, player: Uuid) -> Vec<ItemCount> {
        counts(&self.impounded, player)
    }

    /// What of `player`'s found no room anywhere
    pub fn lost_for(&self, player: Uuid) -> Vec<ItemCount> {
        counts(&self.lost, player)
    }
}

fn counts(of: &[(Uuid, String, i32)], player: Uuid) -> Vec<ItemCount> {
    let mut out: Vec<ItemCount> = Vec::new();
    for (_, item_type, quantity) in of.iter().filter(|o| o.0 == player) {
        match out.iter_mut().find(|c| &c.item_type == item_type) {
            Some(c) => c.quantity = c.quantity.saturating_add(*quantity),
            None => out.push(ItemCount { item_type: item_type.clone(), quantity: *quantity }),
        }
    }
    out
}

/// Where a returned thing goes: its owner's inventory, or their impound when
/// that's full (#141)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Inventory,
    Impound,
}

/// A player's inventory or impound, locked, as things are put into it
struct Store {
    id: Uuid,
    free: Vec<i32>,
    /// The stacks of each stackable item type, in slot order, with how many units each holds
    stacks: HashMap<String, Vec<(Uuid, i64)>>,
}

/// Where a store takes a thing
enum Room {
    /// Onto this stack
    Join(Uuid),
    /// Into this free slot
    Slot(i32),
}

impl Store {
    /// Room for the stack `item` of `quantity` `item_type`: on the first
    /// stack of its type with room, whatever its grade, as every unit keeps
    /// its quality (D2, E1.8), if it stacks; else the first free slot, where
    /// the next of its type can join it.
    fn room(&mut self, item: Uuid, item_type: &str, quantity: i32, stackable: bool) -> Option<Room> {
        if stackable {
            let joins = self.stacks.get_mut(item_type).and_then(|stacks| {
                stacks.iter_mut().find(|(_, held)| *held + quantity as i64 <= i32::MAX as i64)
            });
            if let Some((stack, held)) = joins {
                *held += quantity as i64;
                return Some(Room::Join(*stack));
            }
        }
        let slot = self.slot()?;
        if stackable {
            self.stacks.entry(item_type.to_string()).or_default().push((item, quantity as i64));
        }
        Some(Room::Slot(slot))
    }

    /// The first free slot, taken
    fn slot(&mut self) -> Option<i32> {
        (!self.free.is_empty()).then(|| self.free.remove(0))
    }
}

/// The stores loaded so far, by player and kind. `None` for a player with no
/// inventory.
type Stores = HashMap<(Uuid, Kind), Option<Store>>;

/// Empty every station on `plot_id` into inventories, before a foreclosure
/// releases the plot (#137). Each station's kit goes back to the station's
/// owner at the quality it was placed with, as [`remove_station`] does. Each
/// item in its boxes goes back to whoever owns the item: a guest's deposit is
/// the guest's (#80), and an item with no owner is the station owner's. A
/// stackable item joins the first stack of its type, whatever its grade
/// (plan D2, E1.8), and every unit keeps its quality; anything else takes the
/// first free slot. A tool in a station's tool slot goes back to whoever put
/// it there (E1.6), or the station owner if they're gone. What finds no room
/// in an inventory goes to its player's impound (#141) the same way. Kits go
/// first, then tools, so they take the inventory's room before the boxes'
/// contents do. Only what finds no room in the impound either goes with its
/// station. The stations are left, empty or holding only that, for
/// `release_plot` to delete.
pub async fn return_plot_stations(
    tx: &mut PgConnection,
    plot_id: Uuid,
) -> Result<ReturnedStations, sqlx::Error> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM stations WHERE plot_id = $1")
        .bind(plot_id)
        .fetch_all(&mut *tx)
        .await?;
    return_stations(tx, &ids).await
}

/// Empty `station_ids` into inventories, as [`return_plot_stations`] does for
/// a plot's. Also used for workbenches a terrain change laid a lot under
/// (`plot_sync`, #188). The stations are left for the caller to delete.
pub async fn return_stations(
    tx: &mut PgConnection,
    station_ids: &[Uuid],
) -> Result<ReturnedStations, sqlx::Error> {
    empty_stations(tx, station_ids, RETURN).await
}

/// Empty `station_ids`, which the weather has brought down on public land
/// (V3.2), into their owners' impounds: their tools, and what was in their
/// boxes, as [`return_stations`] would but never into an inventory. Their
/// kits are lost, so none is made. The stations are left for the caller to
/// delete.
pub async fn collapse_stations(
    tx: &mut PgConnection,
    station_ids: &[Uuid],
) -> Result<ReturnedStations, sqlx::Error> {
    empty_stations(tx, station_ids, COLLAPSE).await
}

/// How [`empty_stations`] empties
#[derive(Clone, Copy)]
struct Emptying {
    /// Where each thing may go, the first with room taking it
    kinds: &'static [Kind],
    /// The kits are given back
    kits: bool,
    /// A tool goes to the station's owner, not whoever put it in: its owner
    /// may take any tool from their own station (`station_tools::remove_tool`)
    tools_to_owner: bool,
}

/// A foreclosure, or a lot laid under a workbench
const RETURN: Emptying = Emptying { kinds: &[Kind::Inventory, Kind::Impound], kits: true, tools_to_owner: false };
/// A station fallen in the weather (V3.2)
const COLLAPSE: Emptying = Emptying { kinds: &[Kind::Impound], kits: false, tools_to_owner: false };
/// A take-down by its owner, which [`remove_in`] refuses if any of the
/// owner's things went to the impound or nowhere
const TAKE_DOWN: Emptying = Emptying { kinds: &[Kind::Inventory, Kind::Impound], kits: true, tools_to_owner: true };

/// Empty `station_ids` into the first of `how.kinds` with room for each thing,
/// and give back their kits when `how.kits`
async fn empty_stations(
    tx: &mut PgConnection,
    station_ids: &[Uuid],
    how: Emptying,
) -> Result<ReturnedStations, sqlx::Error> {
    let Emptying { kinds, kits, tools_to_owner } = how;
    let stations: Vec<(Uuid, String, Uuid, i32, Option<Uuid>, Option<String>, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT s.id, s.station_type, s.owner_id, s.kit_quality, s.kit_maker_id, st.kit_item, s.installed_tool, s.tool_owner
         FROM stations s LEFT JOIN station_types st ON st.id = s.station_type
         WHERE s.id = ANY($1) ORDER BY s.placed_at, s.id FOR UPDATE OF s",
    )
    .bind(station_ids)
    .fetch_all(&mut *tx)
    .await?;
    if stations.is_empty() {
        return Ok(ReturnedStations::default());
    }
    let station_ids: Vec<Uuid> = stations.iter().map(|s| s.0).collect();
    let station_owner: HashMap<Uuid, Uuid> = stations.iter().map(|s| (s.0, s.2)).collect();
    // The boxes are locked too, so a deposit racing this waits (see `remove_in`)
    let contents: Vec<(Uuid, String, i32, Option<Uuid>, Uuid, bool)> = sqlx::query_as(
        "SELECT i.id, i.item_type, i.quantity, i.owner_id, c.station_id, it.stackable
         FROM items i
         JOIN containers c ON i.container_id = c.id
         JOIN item_types it ON it.id = i.item_type
         WHERE c.station_id = ANY($1)
         ORDER BY c.station_id, i.slot_index NULLS LAST, i.id
         FOR UPDATE OF i, c",
    )
    .bind(&station_ids)
    .fetch_all(&mut *tx)
    .await?;

    let mut returned = ReturnedStations::default();
    let mut stores = Stores::new();

    for (station_id, station_type, owner_id, kit_quality, kit_maker, kit, _, _) in stations.iter().filter(|_| kits) {
        let Some(kit) = kit else {
            tracing::error!("Station {} has type {} with no kit", station_id, station_type);
            continue;
        };
        let Some((container, kind, slot)) = free_slot(tx, &mut stores, *owner_id, kinds).await? else {
            returned.lost.push((*owner_id, kit.to_string(), 1));
            continue;
        };
        // Marked, as at a take-down (#241)
        super::units::insert_stack(
            tx,
            NewStack {
                item_type: kit,
                owner_id: Some(*owner_id),
                container_id: Some(container),
                slot_index: Some(slot),
                durability: None,
            },
            &kit_units(*kit_quality, *kit_maker),
        )
        .await?;
        returned.kits.push(kit.clone());
        took(&mut returned, *owner_id, container, kind, kit, 1);
    }

    for (station_id, _, owner_id, _, _, _, tool, tool_owner) in &stations {
        let Some(tool) = tool else { continue };
        let player = tool_owner.filter(|_| !tools_to_owner).unwrap_or(*owner_id);
        let item_type: String = sqlx::query_scalar("SELECT item_type FROM items WHERE id = $1")
            .bind(tool)
            .fetch_one(&mut *tx)
            .await?;
        let Some((container, kind, slot)) = free_slot(tx, &mut stores, player, kinds).await? else {
            returned.lost.push((player, item_type, 1));
            continue;
        };
        super::station_tools::give_back(tx, *station_id, *tool, player, container, slot).await?;
        took(&mut returned, player, container, kind, &item_type, 1);
    }

    for (item_id, item_type, quantity, item_owner, station_id, stackable) in contents {
        let player = item_owner.unwrap_or(station_owner[&station_id]);
        let mut found = None;
        for &kind in kinds {
            if let Some(store) = store(tx, &mut stores, player, kind).await? {
                if let Some(room) = store.room(item_id, &item_type, quantity, stackable) {
                    found = Some((store.id, kind, room));
                    break;
                }
            }
        }
        let Some((container, kind, room)) = found else {
            returned.dropped.push((item_type.clone(), quantity));
            returned.lost.push((player, item_type, quantity));
            continue;
        };
        match room {
            Room::Join(stack) => {
                super::units::merge_into(&mut *tx, item_id, stack).await?;
            }
            Room::Slot(slot) => {
                sqlx::query(
                    "UPDATE items SET container_id = $1, slot_index = $2, owner_id = $3 WHERE id = $4",
                )
                .bind(container)
                .bind(slot)
                .bind(player)
                .bind(item_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        took(&mut returned, player, container, kind, &item_type, quantity);
    }
    Ok(returned)
}

fn took(returned: &mut ReturnedStations, player: Uuid, container: Uuid, kind: Kind, item_type: &str, quantity: i32) {
    if !returned.containers.contains(&(player, container)) {
        returned.containers.push((player, container));
    }
    if kind == Kind::Impound {
        returned.impounded.push((player, item_type.to_string(), quantity));
    }
}

/// A free slot in the first of `kinds` of `player`'s stores that has one:
/// their inventory, or else their impound
async fn free_slot(
    tx: &mut PgConnection,
    stores: &mut Stores,
    player: Uuid,
    kinds: &[Kind],
) -> Result<Option<(Uuid, Kind, i32)>, sqlx::Error> {
    for &kind in kinds {
        if let Some(store) = store(tx, stores, player, kind).await? {
            if let Some(slot) = store.slot() {
                return Ok(Some((store.id, kind, slot)));
            }
        }
    }
    Ok(None)
}

/// `player`'s inventory or impound, locked and loaded the first time it's
/// asked for. Their impound is made then if they have none yet. `None` for a
/// player with no inventory.
async fn store<'a>(
    tx: &mut PgConnection,
    stores: &'a mut Stores,
    player: Uuid,
    kind: Kind,
) -> Result<Option<&'a mut Store>, sqlx::Error> {
    if let std::collections::hash_map::Entry::Vacant(e) = stores.entry((player, kind)) {
        let id = match kind {
            Kind::Inventory => {
                sqlx::query_scalar(
                    "SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'player_inventory'
                     LIMIT 1 FOR UPDATE",
                )
                .bind(player)
                .fetch_optional(&mut *tx)
                .await?
            }
            Kind::Impound => Some(super::impound::impound(tx, player).await?),
        };
        let loaded = match id {
            None => None,
            Some(id) => Some(load(tx, id).await?),
        };
        e.insert(loaded);
    }
    Ok(stores.get_mut(&(player, kind)).and_then(Option::as_mut))
}

/// The container `id` as a [`Store`], its items locked
async fn load(tx: &mut PgConnection, id: Uuid) -> Result<Store, sqlx::Error> {
    let slot_count: i32 = sqlx::query_scalar(
        "SELECT ct.slot_count FROM containers c JOIN container_types ct ON ct.id = c.container_type
         WHERE c.id = $1",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let held: Vec<(Uuid, String, i32, Option<i32>, bool)> = sqlx::query_as(
        "SELECT i.id, i.item_type, i.quantity, i.slot_index, it.stackable
         FROM items i JOIN item_types it ON it.id = i.item_type
         WHERE i.container_id = $1 ORDER BY i.slot_index NULLS LAST, i.id
         FOR UPDATE OF i",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let used: Vec<i32> = held.iter().filter_map(|h| h.3).collect();
    let mut stacks: HashMap<String, Vec<(Uuid, i64)>> = HashMap::new();
    for (item, kind, quantity, _, stackable) in held {
        if stackable {
            stacks.entry(kind).or_default().push((item, quantity as i64));
        }
    }
    let free = (0..slot_count).filter(|s| !used.contains(s)).collect();
    Ok(Store { id, free, stacks })
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh database that
/// `sqlx::test` creates from the migrations and drops afterwards. Run them with
/// `cargo test station_removal -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;

    struct Owner {
        id: Uuid,
        inventory: Uuid,
    }

    async fn player(db: &PgPool, name: &str) -> Owner {
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
        let inventory = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)",
        )
        .bind(inventory)
        .bind(id)
        .execute(db)
        .await
        .unwrap();
        Owner { id, inventory }
    }

    /// A storage chest owned by `owner`, off any plot, placed from a kit of `quality`.
    /// Returns the station and its box.
    async fn chest(db: &PgPool, owner: Uuid, quality: i32) -> (Uuid, Uuid) {
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
             VALUES ($1, 'storage_chest', $2, 0, 0, $3)",
        )
        .bind(station)
        .bind(owner)
        .bind(quality)
        .execute(db)
        .await
        .unwrap();
        let container = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2)",
        )
        .bind(container)
        .bind(station)
        .execute(db)
        .await
        .unwrap();
        (station, container)
    }

    async fn put(db: &PgPool, owner: Uuid, container: Uuid, slot: i32) {
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'fiber', 40, 3, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(owner)
        .bind(container)
        .bind(slot)
        .execute(db)
        .await
        .unwrap();
    }

    async fn station_exists(db: &PgPool, id: Uuid) -> bool {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stations WHERE id = $1)")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn an_empty_station_comes_down_and_its_kit_comes_back(db: PgPool) {
        let me = player(&db, "builder").await;
        put(&db, me.id, me.inventory, 0).await;
        let (station, container) = chest(&db, me.id, 73).await;

        let removed = remove_station(&db, me.id, station).await.unwrap();

        assert_eq!(removed.returned_to, vec![(me.id, me.inventory)]);
        let (kind, quality, slot): (String, i32, i32) = sqlx::query_as(
            "SELECT item_type, quality, slot_index FROM items
             WHERE item_type = 'chest_kit' AND owner_id = $1 AND container_id = $2",
        )
        .bind(me.id)
        .bind(me.inventory)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!((kind.as_str(), quality, slot), ("chest_kit", 73, 1));
        assert!(!station_exists(&db, station).await);
        let boxes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM containers WHERE id = $1")
            .bind(container)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(boxes, 0);
    }

    /// Where the item `id` is now: its owner and its container
    async fn whereabouts(db: &PgPool, id: Uuid) -> (Option<Uuid>, Uuid) {
        sqlx::query_as("SELECT owner_id, container_id FROM items WHERE id = $1")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// What's in the box comes down with the station (#489): the owner's own
    /// things, and anything with no owner, to the owner's inventory, joining
    /// a stack of their type, and a guest's deposit back to the guest (#80)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_station_comes_down_with_its_box_emptied(db: PgPool) {
        use crate::engine::units::test_units::{all_held, stack_of, units};
        let me = player(&db, "builder").await;
        let guest = player(&db, "guest").await;
        let held = stack_of(&db, me.id, me.inventory, 0, "fiber", &Units::uniform(2, 50)).await;
        let (station, container) = chest(&db, me.id, 50).await;
        stack_of(&db, me.id, container, 3, "fiber", &Units::uniform(3, 80)).await;
        let rope = stack_of(&db, me.id, container, 5, "rope", &Units::uniform(1, 70)).await;
        sqlx::query("UPDATE items SET owner_id = NULL WHERE id = $1").bind(rope).execute(&db).await.unwrap();
        let deposit = stack_of(&db, guest.id, container, 7, "fiber", &Units::uniform(4, 60)).await;

        let removed = remove_station(&db, me.id, station).await.unwrap();

        assert_eq!(removed.returned_to, vec![(me.id, me.inventory), (guest.id, guest.inventory)]);
        assert!(!station_exists(&db, station).await);
        let mine = Units::from_pairs([(50, 2), (80, 3)]);
        assert_eq!(units(&db, held).await, mine, "the box's fiber joined the pack's");
        assert_eq!(all_held(&db, me.id, "fiber").await, mine);
        assert_eq!(whereabouts(&db, rope).await, (Some(me.id), me.inventory), "no owner: the station owner's");
        assert_eq!(whereabouts(&db, deposit).await, (Some(guest.id), guest.inventory), "back to the guest");
    }

    /// With no room in the owner's inventory for their things in the box, it
    /// stays up and nothing moves: a take-down never fills their impound
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn no_room_for_the_box_keeps_the_station_up(db: PgPool) {
        let me = player(&db, "builder").await;
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'player_inventory'")
            .fetch_one(&db)
            .await
            .unwrap();
        // One free slot: room for the kit, none for the rope
        for slot in 1..slots {
            put(&db, me.id, me.inventory, slot).await;
        }
        let (station, container) = chest(&db, me.id, 50).await;
        let rope =
            crate::engine::units::test_units::stack_of(&db, me.id, container, 0, "rope", &Units::uniform(1, 70)).await;

        let err = remove_station(&db, me.id, station).await.unwrap_err();

        assert!(matches!(err, RemoveError::NoRoom), "{err:?}");
        assert!(station_exists(&db, station).await);
        assert_eq!(whereabouts(&db, rope).await, (Some(me.id), container), "the rope stays in the box");
        let kits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE item_type = 'chest_kit'")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(kits, 0, "no kit made");
    }

    /// A guest whose inventory is full gets their deposit in their impound
    /// (#141), and the station still comes down
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_guest_gets_their_deposit_in_the_impound(db: PgPool) {
        let me = player(&db, "builder").await;
        let guest = player(&db, "guest").await;
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'player_inventory'")
            .fetch_one(&db)
            .await
            .unwrap();
        for slot in 0..slots {
            put(&db, guest.id, guest.inventory, slot).await;
        }
        let (station, container) = chest(&db, me.id, 50).await;
        let deposit =
            crate::engine::units::test_units::stack_of(&db, guest.id, container, 0, "rope", &Units::uniform(2, 70)).await;

        let removed = remove_station(&db, me.id, station).await.unwrap();

        let pound: Uuid = sqlx::query_scalar("SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'impound'")
            .bind(guest.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(removed.returned_to, vec![(me.id, me.inventory), (guest.id, pound)]);
        assert_eq!(whereabouts(&db, deposit).await, (Some(guest.id), pound));
        assert!(!station_exists(&db, station).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_inventory_keeps_the_station_up(db: PgPool) {
        let me = player(&db, "builder").await;
        let slots: i32 = sqlx::query_scalar(
            "SELECT slot_count FROM container_types WHERE id = 'player_inventory'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        for slot in 0..slots {
            put(&db, me.id, me.inventory, slot).await;
        }
        let (station, _) = chest(&db, me.id, 50).await;

        let err = remove_station(&db, me.id, station).await.unwrap_err();

        assert!(matches!(err, RemoveError::NoRoom), "{err:?}");
        assert!(station_exists(&db, station).await);
    }

    /// A workbench whose tool slot holds a stone sickle with 7 uses left, put
    /// there by `tool_owner`. Returns the station and the sickle.
    async fn bench_with_tool(db: &PgPool, owner: Uuid, tool_owner: Uuid) -> (Uuid, Uuid) {
        crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
             VALUES ($1, 'workbench', $2, 0, 0, 61)",
        )
        .bind(station)
        .bind(owner)
        .execute(db)
        .await
        .unwrap();
        let sickle: Uuid = sqlx::query_scalar(
            "INSERT INTO items (id, item_type, quality, quantity, durability)
             VALUES ($1, 'stone_sickle', 50, 1, 7) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query("UPDATE stations SET installed_tool = $2, tool_owner = $3 WHERE id = $1")
            .bind(station)
            .bind(sickle)
            .bind(tool_owner)
            .execute(db)
            .await
            .unwrap();
        (station, sickle)
    }

    /// The tool comes back with the kit, to the station's owner (E1.6)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_station_comes_down_with_its_tool(db: PgPool) {
        let me = player(&db, "builder").await;
        let guest = player(&db, "guest").await;
        let (station, _) = bench_with_tool(&db, me.id, guest.id).await;

        remove_station(&db, me.id, station).await.unwrap();

        let held: Vec<(Uuid, String, i32, Option<i32>)> = sqlx::query_as(
            "SELECT owner_id, item_type, slot_index, durability FROM items
             WHERE container_id = $1 ORDER BY slot_index",
        )
        .bind(me.inventory)
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(
            held,
            vec![
                (me.id, "workbench_kit".to_string(), 0, None),
                (me.id, "stone_sickle".to_string(), 1, Some(7)),
            ]
        );
        assert!(!station_exists(&db, station).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn no_room_for_the_kit_and_its_tool_keeps_the_station_up(db: PgPool) {
        let me = player(&db, "builder").await;
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'player_inventory'")
            .fetch_one(&db)
            .await
            .unwrap();
        for slot in 1..slots {
            put(&db, me.id, me.inventory, slot).await;
        }
        let (station, sickle) = bench_with_tool(&db, me.id, me.id).await;

        let err = remove_station(&db, me.id, station).await.unwrap_err();

        assert!(matches!(err, RemoveError::NoRoom), "{err:?}");
        assert!(station_exists(&db, station).await);
        let installed: Option<Uuid> = sqlx::query_scalar("SELECT installed_tool FROM stations WHERE id = $1")
            .bind(station)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(installed, Some(sickle));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn only_the_owner_takes_a_station_down(db: PgPool) {
        let me = player(&db, "builder").await;
        let other = player(&db, "passer-by").await;
        let (station, _) = chest(&db, me.id, 50).await;

        let err = remove_station(&db, other.id, station).await.unwrap_err();

        assert!(matches!(err, RemoveError::NotYours), "{err:?}");
        assert!(station_exists(&db, station).await);
        let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE owner_id = $1")
            .bind(other.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(items, 0);
    }

    /// With the inventory full, a returned station's kit and its tool wait in
    /// the impound (#141), the tool's wear and all
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_full_inventory_sends_the_kit_and_its_tool_to_the_impound(db: PgPool) {
        let me = player(&db, "builder").await;
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'player_inventory'")
            .fetch_one(&db)
            .await
            .unwrap();
        for slot in 0..slots {
            put(&db, me.id, me.inventory, slot).await;
        }
        let (station, sickle) = bench_with_tool(&db, me.id, me.id).await;

        let mut tx = db.begin().await.unwrap();
        let returned = return_stations(&mut tx, &[station]).await.unwrap();
        tx.commit().await.unwrap();

        let pound: Uuid = sqlx::query_scalar("SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'impound'")
            .bind(me.id)
            .fetch_one(&db)
            .await
            .unwrap();
        let held: Vec<(Uuid, String, i32, Option<i32>)> = sqlx::query_as(
            "SELECT id, item_type, slot_index, durability FROM items WHERE container_id = $1 ORDER BY slot_index",
        )
        .bind(pound)
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!((held[0].1.as_str(), held[0].2), ("workbench_kit", 0));
        assert_eq!(held[1], (sickle, "stone_sickle".to_string(), 1, Some(7)));
        assert_eq!(returned.containers, vec![(me.id, pound)]);
        assert_eq!(
            returned.impounded_for(me.id),
            vec![
                ItemCount { item_type: "workbench_kit".into(), quantity: 1 },
                ItemCount { item_type: "stone_sickle".into(), quantity: 1 },
            ]
        );
        assert!(returned.lost.is_empty());
    }

    /// A foreclosure gives the kit back with its maker's mark, as a take-down
    /// does (#241)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_returned_kit_keeps_its_makers_mark(db: PgPool) {
        use crate::engine::units::test_units::all_marked;
        let me = player(&db, "builder").await;
        let maker = player(&db, "maker").await;
        let (station, _) = chest(&db, me.id, 68).await;
        sqlx::query("UPDATE stations SET kit_maker_id = $2 WHERE id = $1")
            .bind(station)
            .bind(maker.id)
            .execute(&db)
            .await
            .unwrap();

        let mut tx = db.begin().await.unwrap();
        let returned = return_stations(&mut tx, &[station]).await.unwrap();
        tx.commit().await.unwrap();

        assert_eq!(returned.kits, vec!["chest_kit".to_string()]);
        assert_eq!(all_marked(&db, me.id, "chest_kit").await, kit_units(68, Some(maker.id)));
    }

    /// A foreclosure's returns join the first stack of their type, whatever its
    /// grade, and every unit keeps its quality (D2, E1.8)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn returns_join_the_first_stack_and_keep_every_unit(db: PgPool) {
        use crate::engine::units::test_units::{all_held, stack_of, units};
        use capitol_protocol::rules::stacking::Units;
        let ada = player(&db, "ada").await;
        let plot: Uuid = sqlx::query_scalar(
            "INSERT INTO plots (zone_id, grid_x, grid_y, world_x, world_y, bounds_min_x,
                                bounds_min_y, bounds_max_x, bounds_max_y, size_category, owner_id)
             VALUES ((SELECT id FROM zones LIMIT 1), 9999, 9999, 0, 0, -10, -10, 10, 10, 'small', $1)
             RETURNING id",
        )
        .bind(ada.id)
        .fetch_one(&db)
        .await
        .unwrap();
        let (station, chest_box) = chest(&db, ada.id, 50).await;
        sqlx::query("UPDATE stations SET plot_id = $1 WHERE id = $2")
            .bind(plot)
            .bind(station)
            .execute(&db)
            .await
            .unwrap();
        let first = stack_of(&db, ada.id, ada.inventory, 0, "fiber", &Units::uniform(5, 95)).await;
        let second = stack_of(&db, ada.id, ada.inventory, 1, "fiber", &Units::uniform(4, 60)).await;
        stack_of(&db, ada.id, chest_box, 0, "fiber", &Units::from_pairs([(40, 1), (92, 1)])).await;
        stack_of(&db, ada.id, chest_box, 1, "fiber", &Units::uniform(3, 65)).await;
        stack_of(&db, ada.id, chest_box, 2, "fiber", &Units::uniform(1, 93)).await;
        let before = all_held(&db, ada.id, "fiber").await;

        let mut tx = db.begin().await.unwrap();
        let returned = return_plot_stations(&mut tx, plot).await.unwrap();
        tx.commit().await.unwrap();

        assert!(returned.lost.is_empty());
        // Everything joined the first stack; the second is as it was
        assert_eq!(
            units(&db, first).await,
            Units::from_pairs([(40, 1), (65, 3), (92, 1), (93, 1), (95, 5)]),
        );
        assert_eq!(units(&db, second).await, Units::uniform(4, 60));
        assert_eq!(all_held(&db, ada.id, "fiber").await, before, "no unit lost its quality");
    }
}
