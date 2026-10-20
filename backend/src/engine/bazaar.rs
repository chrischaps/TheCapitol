//! The bazaar (E3.10, economy plan D6): stalls merchants rent across the
//! avenue from the Exchange hall, and the shops they keep in them. The routes
//! that call these are `routes::bazaar`; they check the player stands at the
//! stall, since the stalls are `/terrain` landmarks.
//!
//! Each operation is one transaction, and every Strand it moves goes through
//! the currency audit log:
//! - [`rent`]: a free stall for 1 to 7 days, or more days on your own, at 20
//!   Strands a day (`bazaar_rent`, a sink). One stall a player.
//! - [`stock`]: units leave a stack in the merchant's inventory for a stack of
//!   their own in the bazaar's stock (no slot, no owner), at a fixed price a
//!   unit. No fee.
//! - [`buy`]: the units go to the buyer's inventory, or their market box if
//!   it's full, and the price goes from the buyer (`bazaar_purchase`) to the
//!   merchant (`bazaar_sale`), with a notice to the merchant.
//! - [`unlist`]: the units come back to the merchant's inventory, or box.
//! - [`sweep`]: a stall whose rent ran out closes, and its goods go to the
//!   merchant's market box. Those that don't fit wait, still in the stock,
//!   and go as soon as there's room. No goods are lost.
//!
//! So the Strands held plus the rent paid, and every unit held or in the
//! stock, are what there were at the start (the conservation test). The rent
//! and bounds are `rules::bazaar`, shared with the clients.

use std::collections::{BTreeMap, BTreeSet};

use capitol_protocol::rest::{
    BazaarDirectory, BazaarListing, BazaarPurchase, BazaarShop, BazaarStall, BuyListing, MyStallResponse, ShopSign,
    StockListing,
};
use capitol_protocol::rules::bazaar::{self as rules, MAX_LISTINGS, MAX_RENT_DAYS, RENT_DAY_SECS};
use capitol_protocol::rules::stacking::{Pick, Units};
use capitol_protocol::ws::ServerMessage;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::currency::{add_strands_in, subtract_strands_in, TransactionType};
use super::market::{market_box, stow};
use super::notices;
use super::units::{self, NewStack};

/// Why the bazaar refused.
#[derive(Debug)]
pub enum BazaarError {
    /// Days, a name, a price or a quantity out of bounds
    Rules(rules::BazaarError),
    /// Another merchant rents that stall
    Taken,
    /// You rent another stall already
    OneStall(String),
    /// Renting a stall needs a name for the shop
    NameNeeded,
    /// You don't rent a stall
    NotRenting,
    /// The stack isn't in your inventory
    NotYourStack,
    /// The stack doesn't hold the units asked for
    NotHeld,
    /// Named units that don't add up to the quantity
    BadQualities,
    /// The shop holds [`MAX_LISTINGS`] already
    TooManyListings,
    /// No such listing, or its shop is closed
    NoListing,
    /// Buying from your own shop
    OwnListing,
    /// Not enough Strands
    Funds(String),
    /// Neither the inventory nor the market box has room
    NoRoom,
    Db(sqlx::Error),
}

impl BazaarError {
    pub fn message(&self) -> String {
        match self {
            BazaarError::Rules(e) => e.message(),
            BazaarError::Taken => "Another merchant rents that stall".to_string(),
            BazaarError::OneStall(stall) => format!("You rent {stall} already: one stall a merchant"),
            BazaarError::NameNeeded => "Name your shop to rent the stall".to_string(),
            BazaarError::NotRenting => "You don't rent a stall".to_string(),
            BazaarError::NotYourStack => "That stack isn't in your inventory".to_string(),
            BazaarError::NotHeld => "That stack doesn't hold those units".to_string(),
            BazaarError::BadQualities => "The qualities named don't add up to the quantity".to_string(),
            BazaarError::TooManyListings => format!("A shop holds at most {MAX_LISTINGS} listings"),
            BazaarError::NoListing => "That listing isn't for sale".to_string(),
            BazaarError::OwnListing => "That's your own shop".to_string(),
            BazaarError::Funds(m) => m.clone(),
            BazaarError::NoRoom => "Your inventory and market box are both full".to_string(),
            BazaarError::Db(e) => format!("Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for BazaarError {
    fn from(e: sqlx::Error) -> Self {
        BazaarError::Db(e)
    }
}

impl From<rules::BazaarError> for BazaarError {
    fn from(e: rules::BazaarError) -> Self {
        BazaarError::Rules(e)
    }
}

/// A notice queued for a merchant, for the route or the sweep to send live.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub player_id: Uuid,
    pub notice_id: Uuid,
    pub message: ServerMessage,
}

/// A stall that closed because its rent ran out.
#[derive(Debug, Clone, PartialEq)]
pub struct Lapsed {
    pub stall_id: String,
    pub player_id: Uuid,
    /// The merchant's market box, where the goods went
    pub market_box: Uuid,
    pub notice: Notice,
}

/// The stall `player_id` rents, if their rent hasn't run out.
pub async fn stall_of(db: &PgPool, player_id: Uuid, now: DateTime<Utc>) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT stall_id FROM bazaar_rentals WHERE player_id = $1 AND paid_until > $2")
        .bind(player_id)
        .bind(now)
        .fetch_optional(db)
        .await
}

/// The stall listing `listing_id` is in, if its shop is open.
pub async fn stall_of_listing(db: &PgPool, listing_id: Uuid, now: DateTime<Utc>) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT r.stall_id FROM bazaar_listings l JOIN bazaar_rentals r ON r.stall_id = l.stall_id
         WHERE l.id = $1 AND r.paid_until > $2",
    )
    .bind(listing_id)
    .bind(now)
    .fetch_optional(db)
    .await
}

/// Rent `stall_id` for `player_id`, `days` ahead, naming the shop `name`; or,
/// if it's theirs already, pay `days` more (no further than 7 days from now)
/// and rename it if `name` is given. A stall or a rent of theirs that has
/// run out is closed first. Returns the shop, and any stalls that closed.
pub async fn rent(
    db: &PgPool,
    player_id: Uuid,
    stall_id: &str,
    days: i64,
    name: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(BazaarShop, Vec<Lapsed>), BazaarError> {
    rules::validate_days(days)?;
    let name = name.map(rules::shop_name).transpose()?;

    let mut tx = db.begin().await?;
    let mut lapsed = Vec::new();
    // This stall's rent, and the player's own, closed first if they've run out
    let held: Vec<(String, Uuid, DateTime<Utc>)> = sqlx::query_as(
        "SELECT stall_id, player_id, paid_until FROM bazaar_rentals
         WHERE stall_id = $1 OR player_id = $2 ORDER BY stall_id FOR UPDATE",
    )
    .bind(stall_id)
    .bind(player_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut own = None;
    for (held_stall, holder, paid_until) in held {
        if paid_until <= now {
            lapsed.push(lapse(&mut tx, &held_stall).await?);
        } else if holder != player_id {
            return Err(BazaarError::Taken);
        } else if held_stall != stall_id {
            return Err(BazaarError::OneStall(held_stall));
        } else {
            own = Some(paid_until);
        }
    }

    let paid_until = match own {
        Some(until) => {
            let extended = until + Duration::seconds(days * RENT_DAY_SECS);
            if extended > now + Duration::seconds(MAX_RENT_DAYS * RENT_DAY_SECS) {
                return Err(rules::BazaarError::BadDays.into());
            }
            sqlx::query("UPDATE bazaar_rentals SET paid_until = $2, name = COALESCE($3, name) WHERE stall_id = $1")
                .bind(stall_id)
                .bind(extended)
                .bind(name.as_deref())
                .execute(&mut *tx)
                .await?;
            extended
        }
        None => {
            let name = name.ok_or(BazaarError::NameNeeded)?;
            let until = now + Duration::seconds(days * RENT_DAY_SECS);
            // Two renters at once: the second's insert finds the first's row
            let inserted = sqlx::query(
                "INSERT INTO bazaar_rentals (stall_id, player_id, name, rented_at, paid_until)
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
            )
            .bind(stall_id)
            .bind(player_id)
            .bind(&name)
            .bind(now)
            .bind(until)
            .execute(&mut *tx)
            .await?;
            if inserted.rows_affected() == 0 {
                return Err(BazaarError::Taken);
            }
            until
        }
    };
    subtract_strands_in(
        &mut tx,
        player_id,
        rules::rent(days),
        TransactionType::BazaarRent,
        Some(&format!("Rent of {stall_id}, {days} days, to {}", paid_until.format("%Y-%m-%d %H:%M"))),
        None,
    )
    .await
    .map_err(BazaarError::Funds)?;

    let shop = load_shop(&mut tx, stall_id).await?.ok_or(BazaarError::NotRenting)?;
    tx.commit().await?;
    Ok((shop, lapsed))
}

/// Stock `player_id`'s shop with units of a stack in their inventory, at a
/// price a unit. Returns the listing.
pub async fn stock(
    db: &PgPool,
    player_id: Uuid,
    req: &StockListing,
    now: DateTime<Utc>,
) -> Result<BazaarListing, BazaarError> {
    rules::validate_listing(req.price, req.quantity)?;
    let mut tx = db.begin().await?;
    let stall: Option<String> = sqlx::query_scalar(
        "SELECT stall_id FROM bazaar_rentals WHERE player_id = $1 AND paid_until > $2 FOR SHARE",
    )
    .bind(player_id)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    let stall = stall.ok_or(BazaarError::NotRenting)?;
    let listed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bazaar_listings WHERE stall_id = $1")
        .bind(&stall)
        .fetch_one(&mut *tx)
        .await?;
    if listed as usize >= MAX_LISTINGS {
        return Err(BazaarError::TooManyListings);
    }

    let source: Option<(String, Option<Uuid>, Option<String>, Option<i32>)> = sqlx::query_as(
        "SELECT i.item_type, i.owner_id, c.container_type, i.durability
         FROM items i LEFT JOIN containers c ON c.id = i.container_id
         WHERE i.id = $1 FOR UPDATE OF i",
    )
    .bind(req.item_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((item_type, owner, container_type, durability)) = source else {
        return Err(BazaarError::NotYourStack);
    };
    if owner != Some(player_id) || container_type.as_deref() != Some("player_inventory") {
        return Err(BazaarError::NotYourStack);
    }
    let held = units::units_of(&mut tx, req.item_id).await?;
    let wanted = match &req.qualities {
        Some(named) if named.count() != req.quantity as i64 => return Err(BazaarError::BadQualities),
        Some(named) => named.clone(),
        None => held.pick(req.quantity, Pick::default()).ok_or(BazaarError::NotHeld)?,
    };
    let taken = units::take_units(&mut tx, req.item_id, &wanted).await?.ok_or(BazaarError::NotHeld)?;
    let stock_id: Uuid = sqlx::query_scalar("SELECT id FROM containers WHERE container_type = 'bazaar_stock'")
        .fetch_one(&mut *tx)
        .await?;
    let stack = units::insert_stack(
        &mut tx,
        NewStack {
            item_type: &item_type,
            owner_id: None,
            container_id: Some(stock_id),
            slot_index: None,
            durability,
        },
        &taken.marked,
    )
    .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO bazaar_listings (id, player_id, stall_id, item_id, item_type, price, listed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(player_id)
    .bind(&stall)
    .bind(stack)
    .bind(&item_type)
    .bind(req.price)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(BazaarListing {
        id,
        item_type,
        price: req.price,
        units: taken.units,
        durability,
        listed_at: now,
    })
}

/// Take `player_id`'s listing `listing_id` off their open shop: its units go
/// back to their inventory, or their market box if it's full. Returns the
/// units and the container they went to.
pub async fn unlist(db: &PgPool, player_id: Uuid, listing_id: Uuid, now: DateTime<Utc>) -> Result<(Units, Uuid), BazaarError> {
    let mut tx = db.begin().await?;
    let stack: Option<Uuid> = sqlx::query_scalar(
        "SELECT l.item_id FROM bazaar_listings l JOIN bazaar_rentals r ON r.stall_id = l.stall_id
         WHERE l.id = $1 AND l.player_id = $2 AND r.paid_until > $3
         FOR UPDATE OF l",
    )
    .bind(listing_id)
    .bind(player_id)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    let stack = stack.ok_or(BazaarError::NoListing)?;
    sqlx::query("DELETE FROM bazaar_listings WHERE id = $1").bind(listing_id).execute(&mut *tx).await?;
    let (units, into) = deliver(&mut tx, player_id, stack).await?;
    tx.commit().await?;
    Ok((units, into))
}

/// What a purchase did, for the route to tell both sides.
#[derive(Debug, Clone, PartialEq)]
pub struct Bought {
    pub purchase: BazaarPurchase,
    /// The container the units went to
    pub into: Uuid,
    pub merchant: Uuid,
    pub stall_id: String,
    /// The merchant's `bazaar_sale` notice
    pub notice: Notice,
}

/// `buyer` buys units of `listing_id` at its price, at `stall_id` (where the
/// route found them standing). The units go to their inventory, or their
/// market box if it's full; the Strands go to the merchant.
pub async fn buy(
    db: &PgPool,
    buyer: Uuid,
    listing_id: Uuid,
    stall_id: &str,
    req: &BuyListing,
    now: DateTime<Utc>,
) -> Result<Bought, BazaarError> {
    rules::validate_quantity(req.quantity)?;
    let mut tx = db.begin().await?;
    let listing: Option<(Uuid, Uuid, String, i64, Option<i32>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT l.player_id, l.item_id, l.item_type, l.price, i.durability, l.listed_at
         FROM bazaar_listings l
         JOIN bazaar_rentals r ON r.stall_id = l.stall_id
         JOIN items i ON i.id = l.item_id
         WHERE l.id = $1 AND l.stall_id = $2 AND r.paid_until > $3
         FOR UPDATE OF l, i",
    )
    .bind(listing_id)
    .bind(stall_id)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((merchant, stack, item_type, price, durability, listed_at)) = listing else {
        return Err(BazaarError::NoListing);
    };
    if merchant == buyer {
        return Err(BazaarError::OwnListing);
    }

    let mut left = units::units_of(&mut tx, stack).await?;
    let wanted = match &req.qualities {
        Some(named) if named.count() != req.quantity as i64 => return Err(BazaarError::BadQualities),
        Some(named) => named.clone(),
        None => left.pick(req.quantity, Pick::Best).ok_or(BazaarError::NotHeld)?,
    };
    let units = left.take_units(&wanted).ok_or(BazaarError::NotHeld)?;
    let paid = price * req.quantity as i64;

    // The units bought, as the listing's own stack if they're all of it (a
    // tool keeps its wear), or a stack split off it
    let moving = if left.is_empty() {
        sqlx::query("DELETE FROM bazaar_listings WHERE id = $1").bind(listing_id).execute(&mut *tx).await?;
        stack
    } else {
        // The units go with their makers' marks
        let moved = units::write_take(&mut tx, stack, &units, &left).await?;
        units::insert_stack(
            &mut tx,
            NewStack { item_type: &item_type, owner_id: None, container_id: None, slot_index: None, durability },
            &moved,
        )
        .await?
    };
    let (_, into) = deliver(&mut tx, buyer, moving).await?;
    let item_id = stack_in(&mut tx, moving, into, &item_type).await?;
    let into_type: String = sqlx::query_scalar("SELECT container_type FROM containers WHERE id = $1")
        .bind(into)
        .fetch_one(&mut *tx)
        .await?;

    subtract_strands_in(
        &mut tx,
        buyer,
        paid,
        TransactionType::BazaarPurchase,
        Some(&format!("Bought {} {item_type} at {price} each, at {stall_id}", req.quantity)),
        Some(listing_id),
    )
    .await
    .map_err(BazaarError::Funds)?;
    add_strands_in(
        &mut tx,
        merchant,
        paid,
        TransactionType::BazaarSale,
        Some(&format!("Sold {} {item_type} at {price} each, at {stall_id}", req.quantity)),
        Some(listing_id),
    )
    .await
    .map_err(BazaarError::Funds)?;

    let (buyer_name, buyer_citizen): (String, bool) = sqlx::query_as(
        "SELECT p.name, a.kind = 'citizen' FROM players p JOIN accounts a ON a.id = p.account_id WHERE p.id = $1",
    )
    .bind(buyer)
    .fetch_one(&mut *tx)
    .await?;
    let message = ServerMessage::BazaarSale {
        stall_id: stall_id.to_string(),
        listing_id,
        item_type: item_type.clone(),
        buyer_name,
        buyer_citizen,
        units: units.clone(),
        price,
        paid,
    };
    let notice_id = notices::queue(&mut *tx, merchant, stall_id, &message).await?;
    tx.commit().await?;

    let listing = (!left.is_empty()).then_some(BazaarListing {
        id: listing_id,
        item_type,
        price,
        units: left,
        durability,
        listed_at,
    });
    Ok(Bought {
        purchase: BazaarPurchase { units, paid, item_id, into: into_type, listing },
        into,
        merchant,
        stall_id: stall_id.to_string(),
        notice: Notice { player_id: merchant, notice_id, message },
    })
}

/// Move the loose stack `stack` to `player_id`'s inventory, or their market
/// box if the inventory has no room. Returns its units and where they went.
async fn deliver(conn: &mut PgConnection, player_id: Uuid, stack: Uuid) -> Result<(Units, Uuid), BazaarError> {
    let inventory: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'player_inventory'")
            .bind(player_id)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(inventory) = inventory {
        if let Some(units) = stow(conn, player_id, stack, inventory).await? {
            return Ok((units, inventory));
        }
    }
    let into = market_box(conn, player_id).await?;
    let units = stow(conn, player_id, stack, into).await?.ok_or(BazaarError::NoRoom)?;
    Ok((units, into))
}

/// The stack `stack` became once stowed in `into`: itself, or the stack of
/// its type it joined.
async fn stack_in(conn: &mut PgConnection, stack: Uuid, into: Uuid, item_type: &str) -> Result<Uuid, sqlx::Error> {
    let still: Option<Uuid> = sqlx::query_scalar("SELECT id FROM items WHERE id = $1")
        .bind(stack)
        .fetch_optional(&mut *conn)
        .await?;
    match still {
        Some(id) => Ok(id),
        None => Ok(units::first_stack(conn, into, item_type).await?.unwrap_or(stack)),
    }
}

/// Close the stall `stall_id` (its rental locked by the caller): the rental
/// goes, so the stall is free, and its goods go to the merchant's market box,
/// or wait for room there.
async fn lapse(conn: &mut PgConnection, stall_id: &str) -> Result<Lapsed, sqlx::Error> {
    let (player_id, name): (Uuid, String) =
        sqlx::query_as("DELETE FROM bazaar_rentals WHERE stall_id = $1 RETURNING player_id, name")
            .bind(stall_id)
            .fetch_one(&mut *conn)
            .await?;
    sqlx::query("UPDATE bazaar_listings SET stall_id = NULL WHERE stall_id = $1")
        .bind(stall_id)
        .execute(&mut *conn)
        .await?;
    let (returned, waiting, market_box) = return_waiting(conn, player_id).await?;
    let message = ServerMessage::StallLapsed { stall_id: stall_id.to_string(), name, returned, waiting };
    let notice_id = notices::queue(&mut *conn, player_id, stall_id, &message).await?;
    Ok(Lapsed {
        stall_id: stall_id.to_string(),
        player_id,
        market_box,
        notice: Notice { player_id, notice_id, message },
    })
}

/// Move `player_id`'s goods from closed stalls into their market box, oldest
/// first, as far as it has room. Returns how many stacks went and how many
/// still wait, and the box.
async fn return_waiting(conn: &mut PgConnection, player_id: Uuid) -> Result<(u32, u32, Uuid), sqlx::Error> {
    let into = market_box(conn, player_id).await?;
    let waiting: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, item_id FROM bazaar_listings WHERE player_id = $1 AND stall_id IS NULL
         ORDER BY listed_at, id FOR UPDATE",
    )
    .bind(player_id)
    .fetch_all(&mut *conn)
    .await?;
    let (mut returned, mut left) = (0, 0);
    for (listing, stack) in waiting {
        if stow(conn, player_id, stack, into).await?.is_some() {
            sqlx::query("DELETE FROM bazaar_listings WHERE id = $1").bind(listing).execute(&mut *conn).await?;
            returned += 1;
        } else {
            left += 1;
        }
    }
    Ok((returned, left, into))
}

/// What a sweep did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Swept {
    /// The stalls that closed
    pub lapsed: Vec<Lapsed>,
    /// Market boxes that waiting goods moved into, with their owners
    pub boxes: Vec<(Uuid, Uuid)>,
}

/// Close every stall whose rent ran out by `now`, each in a transaction of
/// its own, and move waiting goods into market boxes that have room now.
pub async fn sweep(db: &PgPool, now: DateTime<Utc>) -> Result<Swept, sqlx::Error> {
    let mut swept = Swept::default();
    let due: Vec<String> = sqlx::query_scalar("SELECT stall_id FROM bazaar_rentals WHERE paid_until <= $1 ORDER BY stall_id")
        .bind(now)
        .fetch_all(db)
        .await?;
    for stall_id in due {
        let mut tx = db.begin().await?;
        // A renter may have paid, or someone else closed it, meanwhile
        let still: Option<String> = sqlx::query_scalar(
            "SELECT stall_id FROM bazaar_rentals WHERE stall_id = $1 AND paid_until <= $2 FOR UPDATE SKIP LOCKED",
        )
        .bind(&stall_id)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        if still.is_some() {
            swept.lapsed.push(lapse(&mut tx, &stall_id).await?);
        }
        tx.commit().await?;
    }

    let waiting: Vec<Uuid> =
        sqlx::query_scalar("SELECT DISTINCT player_id FROM bazaar_listings WHERE stall_id IS NULL")
            .fetch_all(db)
            .await?;
    for player_id in waiting {
        let mut tx = db.begin().await?;
        let (returned, _, into) = return_waiting(&mut tx, player_id).await?;
        tx.commit().await?;
        if returned > 0 {
            swept.boxes.push((player_id, into));
        }
    }
    Ok(swept)
}

/// How often [`run_bazaar_loop`] sweeps for lapsed stalls.
const SWEEP_SECS: u64 = 10;

/// Close lapsed stalls and return waiting goods every [`SWEEP_SECS`],
/// telling the merchants.
pub async fn run_bazaar_loop(db: PgPool, events: tokio::sync::broadcast::Sender<super::GameEvent>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(SWEEP_SECS));
    loop {
        interval.tick().await;
        match sweep(&db, Utc::now()).await {
            Ok(swept) => {
                crate::routes::bazaar::announce_lapsed(&events, &swept.lapsed);
                for (player_id, container_id) in swept.boxes {
                    let _ = events.send(super::GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
                }
            }
            Err(e) => tracing::error!("The bazaar's sweep failed: {e}"),
        }
    }
}

/// The directory: each of `stalls` (the terrain's), and the shop in it if
/// its rent hasn't run out, with the kinds of goods it lists but no prices,
/// quantities or grades.
pub async fn directory(db: &PgPool, stalls: &[String], now: DateTime<Utc>) -> Result<BazaarDirectory, sqlx::Error> {
    let rows: Vec<(String, String, Uuid, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT r.stall_id, r.name, r.player_id, p.name, a.kind = 'citizen', l.item_type
         FROM bazaar_rentals r
         JOIN players p ON p.id = r.player_id
         JOIN accounts a ON a.id = p.account_id
         LEFT JOIN bazaar_listings l ON l.stall_id = r.stall_id
         WHERE r.paid_until > $1",
    )
    .bind(now)
    .fetch_all(db)
    .await?;
    let mut shops: BTreeMap<String, (ShopSign, BTreeSet<String>)> = BTreeMap::new();
    for (stall_id, name, owner_id, owner_name, owner_citizen, item_type) in rows {
        let (_, kinds) = shops.entry(stall_id).or_insert_with(|| {
            (ShopSign { name, owner_id, owner_name, owner_citizen, item_types: Vec::new() }, BTreeSet::new())
        });
        kinds.extend(item_type);
    }
    Ok(BazaarDirectory {
        stalls: stalls
            .iter()
            .map(|stall_id| BazaarStall {
                stall_id: stall_id.clone(),
                shop: shops.remove(stall_id).map(|(mut sign, kinds)| {
                    sign.item_types = kinds.into_iter().collect();
                    sign
                }),
            })
            .collect(),
    })
}

/// The shop at `stall_id`, with its prices and units, if its rent hasn't run
/// out by `now`.
pub async fn shop(db: &PgPool, stall_id: &str, now: DateTime<Utc>) -> Result<Option<BazaarShop>, sqlx::Error> {
    let mut conn = db.acquire().await?;
    let shop = load_shop(&mut conn, stall_id).await?;
    Ok(shop.filter(|s| s.paid_until > now))
}

/// `player_id`'s own stall, and their goods waiting for room in the box.
pub async fn mine(db: &PgPool, player_id: Uuid, now: DateTime<Utc>) -> Result<MyStallResponse, sqlx::Error> {
    let shop = match stall_of(db, player_id, now).await? {
        Some(stall_id) => shop(db, &stall_id, now).await?,
        None => None,
    };
    let mut conn = db.acquire().await?;
    let returning = listings(&mut conn, Listings::Returning(player_id)).await?;
    Ok(MyStallResponse { shop, returning })
}

/// The shop at `stall_id`, whether or not its rent has run out.
async fn load_shop(conn: &mut PgConnection, stall_id: &str) -> Result<Option<BazaarShop>, sqlx::Error> {
    let row: Option<(String, Uuid, String, bool, DateTime<Utc>)> = sqlx::query_as(
        "SELECT r.name, r.player_id, p.name, a.kind = 'citizen', r.paid_until
         FROM bazaar_rentals r
         JOIN players p ON p.id = r.player_id
         JOIN accounts a ON a.id = p.account_id
         WHERE r.stall_id = $1",
    )
    .bind(stall_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((name, owner_id, owner_name, owner_citizen, paid_until)) = row else {
        return Ok(None);
    };
    let listings = listings(conn, Listings::At(stall_id)).await?;
    Ok(Some(BazaarShop { stall_id: stall_id.to_string(), name, owner_id, owner_name, owner_citizen, paid_until, listings }))
}

/// Which listings [`listings`] loads.
enum Listings<'a> {
    /// Those in the stall
    At(&'a str),
    /// The player's, from closed stalls, waiting for room in their box
    Returning(Uuid),
}

/// The listings `which` names, oldest first.
async fn listings(conn: &mut PgConnection, which: Listings<'_>) -> Result<Vec<BazaarListing>, sqlx::Error> {
    let (stall, player) = match which {
        Listings::At(stall) => (Some(stall), None),
        Listings::Returning(player) => (None, Some(player)),
    };
    let rows: Vec<(Uuid, Uuid, String, i64, Option<i32>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT l.id, l.item_id, l.item_type, l.price, i.durability, l.listed_at
         FROM bazaar_listings l JOIN items i ON i.id = l.item_id
         WHERE ($1::text IS NOT NULL AND l.stall_id = $1)
            OR ($2::uuid IS NOT NULL AND l.player_id = $2 AND l.stall_id IS NULL)
         ORDER BY l.listed_at, l.id",
    )
    .bind(stall)
    .bind(player)
    .fetch_all(&mut *conn)
    .await?;
    let stacks: Vec<Uuid> = rows.iter().map(|r| r.1).collect();
    let mut units = sqlx::query_as::<_, (Uuid, i16, i32)>(
        "SELECT item_id, quality, count FROM item_units WHERE item_id = ANY($1) ORDER BY item_id, quality",
    )
    .bind(&stacks)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .fold(BTreeMap::<Uuid, Vec<(i32, i32)>>::new(), |mut m, (id, q, c)| {
        m.entry(id).or_default().push((q as i32, c));
        m
    });
    Ok(rows
        .into_iter()
        .map(|(id, stack, item_type, price, durability, listed_at)| BazaarListing {
            id,
            item_type,
            price,
            units: Units::from_pairs(units.remove(&stack).unwrap_or_default()),
            durability,
            listed_at,
        })
        .collect())
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test bazaar -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::market::tests::{balance, box_of, player, Player};
    use crate::engine::units::test_units::{all_in, stack_of, units};

    const STALL: &str = "bazaar_stall_3";

    fn stock_req(item_id: Uuid, price: i64, quantity: i32, qualities: Option<Units>) -> StockListing {
        StockListing { item_id, quantity, price, qualities }
    }

    fn buy_req(quantity: i32) -> BuyListing {
        BuyListing { quantity, qualities: None }
    }

    /// Every unit of `item_type` in `players`' inventories and boxes, and in
    /// the bazaar's stock
    async fn every_unit(db: &PgPool, players: &[&Player], item_type: &str) -> Units {
        let mut all = Units::new();
        for p in players {
            all.merge(&all_in(db, p.inventory, item_type).await);
            all.merge(&all_in(db, box_of(db, p.id).await, item_type).await);
        }
        let stock: Uuid = sqlx::query_scalar("SELECT id FROM containers WHERE container_type = 'bazaar_stock'")
            .fetch_one(db)
            .await
            .unwrap();
        all.merge(&all_in(db, stock, item_type).await);
        all
    }

    async fn audit(db: &PgPool, player: Uuid) -> Vec<(i64, String)> {
        sqlx::query_as(
            "SELECT amount::bigint, transaction_type FROM currency_transactions WHERE player_id = $1 ORDER BY id",
        )
        .bind(player)
        .fetch_all(db)
        .await
        .unwrap()
    }

    async fn slots(db: &PgPool, container_type: &str, n: i32) {
        sqlx::query("UPDATE container_types SET slot_count = $2 WHERE id = $1")
            .bind(container_type)
            .bind(n)
            .execute(db)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_merchant_rents_stocks_and_sells_and_nothing_is_made_or_lost(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bram = player(&db, "bram", 500).await;
        let rope = Units::from_pairs([(50, 2), (81, 6), (92, 4)]);
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &rope).await;

        let (shop_now, lapsed) = rent(&db, ada.id, STALL, 2, Some(" Ada's Ropes "), now).await.unwrap();
        assert!(lapsed.is_empty());
        assert_eq!((shop_now.name.as_str(), shop_now.owner_name.as_str()), ("Ada's Ropes", "ada"));
        assert_eq!(balance(&db, ada.id).await, 960);

        let listed = Units::from_pairs([(81, 6), (92, 4)]);
        let listing = stock(&db, ada.id, &stock_req(stack, 7, 10, Some(listed.clone())), now).await.unwrap();
        assert_eq!(listing.units, listed);
        assert_eq!(units(&db, stack).await, Units::uniform(2, 50), "the rest stay in her inventory");

        // The directory names the shop and its goods, and nothing about them
        let dir = directory(&db, &["bazaar_stall_1".into(), STALL.into()], now).await.unwrap();
        assert_eq!(dir.stalls[0].shop, None);
        let sign = dir.stalls[1].shop.as_ref().unwrap();
        assert_eq!((sign.name.as_str(), sign.item_types.clone()), ("Ada's Ropes", vec!["rope".to_string()]));
        let json = serde_json::to_string(&dir).unwrap();
        for hidden in ["price", "units", "quantity", "grade", "quality"] {
            assert!(!json.contains(hidden), "{hidden} in {json}");
        }
        // At the stall, each unit shows
        let at_stall = shop(&db, STALL, now).await.unwrap().unwrap();
        assert_eq!((at_stall.listings[0].price, at_stall.listings[0].units.clone()), (7, listed.clone()));

        // Bram buys part of it: the best units, by default
        let bought = buy(&db, bram.id, listing.id, STALL, &buy_req(3), now).await.unwrap();
        assert_eq!((bought.purchase.units.clone(), bought.purchase.paid), (Units::uniform(3, 92), 21));
        assert_eq!(bought.purchase.into, "player_inventory");
        assert_eq!(bought.purchase.listing.as_ref().unwrap().units, Units::from_pairs([(81, 6), (92, 1)]));
        assert_eq!(all_in(&db, bram.inventory, "rope").await, Units::uniform(3, 92));
        assert_eq!((balance(&db, ada.id).await, balance(&db, bram.id).await), (981, 479));
        assert_eq!(audit(&db, ada.id).await, vec![(-40, "bazaar_rent".into()), (21, "bazaar_sale".into())]);
        assert_eq!(audit(&db, bram.id).await, vec![(-21, "bazaar_purchase".into())]);

        // Named units, then the rest: the listing goes when it's sold out
        let named = BuyListing { quantity: 2, qualities: Some(Units::uniform(2, 81)) };
        buy(&db, bram.id, listing.id, STALL, &named, now).await.unwrap();
        let last = buy(&db, bram.id, listing.id, STALL, &buy_req(5), now).await.unwrap();
        assert_eq!(last.purchase.listing, None);
        assert!(matches!(buy(&db, bram.id, listing.id, STALL, &buy_req(1), now).await, Err(BazaarError::NoListing)));
        assert!(shop(&db, STALL, now).await.unwrap().unwrap().listings.is_empty());

        // Strands: what they hold, plus the rent, is what they started with
        assert_eq!(balance(&db, ada.id).await + balance(&db, bram.id).await + 40, 1500);
        assert_eq!(every_unit(&db, &[&ada, &bram], "rope").await, rope);
        // Ada heard of each sale
        let waiting = notices::take_all(&db, ada.id).await.unwrap();
        assert_eq!(waiting.len(), 3);
        assert!(matches!(&waiting[0], ServerMessage::BazaarSale { buyer_name, paid: 21, .. } if buyer_name == "bram"));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn what_the_bazaar_refuses(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bram = player(&db, "bram", 1000).await;
        let poor = player(&db, "poor", 10).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::uniform(5, 60)).await;

        assert!(matches!(rent(&db, ada.id, STALL, 8, Some("A"), now).await, Err(BazaarError::Rules(_))));
        assert!(matches!(rent(&db, ada.id, STALL, 2, None, now).await, Err(BazaarError::NameNeeded)));
        assert!(matches!(rent(&db, poor.id, STALL, 1, Some("P"), now).await, Err(BazaarError::Funds(_))));
        assert!(matches!(stock(&db, ada.id, &stock_req(stack, 5, 1, None), now).await, Err(BazaarError::NotRenting)));

        rent(&db, ada.id, STALL, 6, Some("Ada's"), now).await.unwrap();
        assert!(matches!(rent(&db, bram.id, STALL, 1, Some("Bram's"), now).await, Err(BazaarError::Taken)));
        assert!(matches!(rent(&db, ada.id, "bazaar_stall_4", 1, Some("Two"), now).await, Err(BazaarError::OneStall(_))));
        // A week ahead at most: one more day is fine, two aren't
        assert!(matches!(rent(&db, ada.id, STALL, 2, None, now).await, Err(BazaarError::Rules(_))));
        let (renamed, _) = rent(&db, ada.id, STALL, 1, Some("Ada's Ropes"), now).await.unwrap();
        assert_eq!(renamed.name, "Ada's Ropes");
        assert!((renamed.paid_until - (now + Duration::days(7))).num_milliseconds().abs() < 5);
        assert_eq!(balance(&db, ada.id).await, 1000 - 140);

        let listing = stock(&db, ada.id, &stock_req(stack, 5, 3, None), now).await.unwrap();
        assert!(matches!(stock(&db, ada.id, &stock_req(stack, 5, 9, None), now).await, Err(BazaarError::NotHeld)));
        assert!(matches!(stock(&db, ada.id, &stock_req(stack, 0, 1, None), now).await, Err(BazaarError::Rules(_))));
        let not_hers = stack_of(&db, bram.id, bram.inventory, 0, "rope", &Units::uniform(1, 60)).await;
        assert!(matches!(
            stock(&db, ada.id, &stock_req(not_hers, 5, 1, None), now).await,
            Err(BazaarError::NotYourStack)
        ));
        assert!(matches!(buy(&db, ada.id, listing.id, STALL, &buy_req(1), now).await, Err(BazaarError::OwnListing)));
        assert!(matches!(buy(&db, bram.id, listing.id, STALL, &buy_req(4), now).await, Err(BazaarError::NotHeld)));
        assert!(matches!(buy(&db, poor.id, listing.id, STALL, &buy_req(3), now).await, Err(BazaarError::Funds(_))));
        assert!(matches!(
            buy(&db, bram.id, listing.id, "bazaar_stall_9", &buy_req(1), now).await,
            Err(BazaarError::NoListing)
        ));
        // Nothing moved for any of that
        assert_eq!((balance(&db, bram.id).await, balance(&db, poor.id).await), (1000, 10));
        assert_eq!(all_in(&db, poor.inventory, "rope").await, Units::new());

        // Unlisting brings the units back to her stack
        let (back, into) = unlist(&db, ada.id, listing.id, now).await.unwrap();
        assert_eq!((back, into), (Units::uniform(3, 60), ada.inventory));
        assert_eq!(all_in(&db, ada.inventory, "rope").await, Units::uniform(5, 60));
    }

    /// The inventory full, the bought goods wait in the market box
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_buyer_with_a_full_inventory_finds_it_in_their_market_box(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 100).await;
        let bram = player(&db, "bram", 100).await;
        slots(&db, "player_inventory", 1).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::uniform(4, 70)).await;
        stack_of(&db, bram.id, bram.inventory, 0, "fiber", &Units::uniform(1, 50)).await;
        rent(&db, ada.id, STALL, 1, Some("Ropes"), now).await.unwrap();
        let listing = stock(&db, ada.id, &stock_req(stack, 3, 4, None), now).await.unwrap();

        let bought = buy(&db, bram.id, listing.id, STALL, &buy_req(4), now).await.unwrap();
        assert_eq!(bought.purchase.into, "market_box");
        assert_eq!(all_in(&db, box_of(&db, bram.id).await, "rope").await, Units::uniform(4, 70));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_lapsed_stall_sends_its_goods_to_the_market_box(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bram = player(&db, "bram", 1000).await;
        slots(&db, "market_box", 1).await;
        let rope = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::from_pairs([(40, 3), (90, 2)])).await;
        let fiber = stack_of(&db, ada.id, ada.inventory, 1, "fiber", &Units::uniform(6, 77)).await;
        rent(&db, ada.id, STALL, 1, Some("Ada's"), now).await.unwrap();
        let first = stock(&db, ada.id, &stock_req(rope, 9, 5, None), now).await.unwrap();
        // Listed after the rope, so the rope goes back first
        stock(&db, ada.id, &stock_req(fiber, 4, 6, None), now + Duration::seconds(1)).await.unwrap();

        // Not yet
        assert!(sweep(&db, now + Duration::hours(23)).await.unwrap().lapsed.is_empty());

        // A day on, the stall closes; the box has room for one of the two
        let later = now + Duration::days(1) + Duration::seconds(1);
        let swept = sweep(&db, later).await.unwrap();
        assert_eq!(swept.lapsed.len(), 1);
        assert!(matches!(
            &swept.lapsed[0].notice.message,
            ServerMessage::StallLapsed { returned: 1, waiting: 1, .. }
        ));
        let the_box = box_of(&db, ada.id).await;
        assert_eq!(all_in(&db, the_box, "rope").await, Units::from_pairs([(40, 3), (90, 2)]));
        assert!(matches!(buy(&db, bram.id, first.id, STALL, &buy_req(1), later).await, Err(BazaarError::NoListing)));
        let dir = directory(&db, &[STALL.into()], later).await.unwrap();
        assert_eq!(dir.stalls[0].shop, None, "free for the next renter");
        let mine_now = mine(&db, ada.id, later).await.unwrap();
        assert_eq!((mine_now.shop, mine_now.returning.len()), (None, 1));

        // Bram can rent it now
        rent(&db, bram.id, STALL, 1, Some("Bram's"), later).await.unwrap();

        // Ada collects the rope; the fiber follows at the next sweep
        sqlx::query("UPDATE items SET container_id = $1, slot_index = 0 WHERE container_id = $2")
            .bind(ada.inventory)
            .bind(the_box)
            .execute(&db)
            .await
            .unwrap();
        let swept = sweep(&db, later).await.unwrap();
        assert_eq!(swept.boxes, vec![(ada.id, the_box)]);
        assert_eq!(all_in(&db, the_box, "fiber").await, Units::uniform(6, 77));
        assert!(mine(&db, ada.id, later).await.unwrap().returning.is_empty());
        assert_eq!(every_unit(&db, &[&ada, &bram], "fiber").await, Units::uniform(6, 77));
    }

    /// Renting a stall whose rent ran out, before the sweep came, closes it first
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn renting_a_lapsed_stall_closes_it_first(db: PgPool) {
        let now = Utc::now();
        let ada = player(&db, "ada", 1000).await;
        let bram = player(&db, "bram", 1000).await;
        let rope = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::uniform(2, 60)).await;
        rent(&db, ada.id, STALL, 1, Some("Ada's"), now).await.unwrap();
        stock(&db, ada.id, &stock_req(rope, 9, 2, None), now).await.unwrap();

        let later = now + Duration::days(2);
        let (taken, lapsed) = rent(&db, bram.id, STALL, 1, Some("Bram's"), later).await.unwrap();
        assert_eq!((taken.owner_name.as_str(), taken.listings.len()), ("bram", 0));
        assert_eq!(lapsed.len(), 1);
        assert_eq!(all_in(&db, box_of(&db, ada.id).await, "rope").await, Units::uniform(2, 60));
        // And Ada may rent another
        rent(&db, ada.id, "bazaar_stall_4", 1, Some("Ada's again"), later).await.unwrap();
    }

    /// Seeded runs of random rents, stocks, buys, unlists and sweeps across
    /// three merchants and two stalls, with the clock running on. After every
    /// step the Strands held plus the rent paid, and every unit of each type,
    /// are what there were at the start; and every balance is its start plus
    /// its audit entries.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn rents_stocks_buys_and_lapses_conserve_strands_and_units(db: PgPool) {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        slots(&db, "market_box", 3).await;
        slots(&db, "player_inventory", 4).await;
        for seed in 0..4u64 {
            let mut rng = StdRng::seed_from_u64(seed);
            let people = [
                player(&db, &format!("ada{seed}"), 400).await,
                player(&db, &format!("bram{seed}"), 400).await,
                player(&db, &format!("cara{seed}"), 400).await,
            ];
            let kinds = ["rope", "fiber"];
            let mut start: BTreeMap<&str, Units> = BTreeMap::new();
            for (i, p) in people.iter().enumerate() {
                for (k, kind) in kinds.iter().enumerate() {
                    let units = Units::from_pairs([(30 + 10 * i as i32 + k as i32, 6), (90 - 5 * i as i32, 4)]);
                    stack_of(&db, p.id, p.inventory, k as i32, kind, &units).await;
                    start.entry(kind).or_default().merge(&units);
                }
            }
            let refs: Vec<&Player> = people.iter().collect();
            let mut now = Utc::now();

            for step in 0..60 {
                let who = &people[rng.gen_range(0..3)];
                let stall = ["bazaar_stall_1", "bazaar_stall_2"][rng.gen_range(0..2)];
                match rng.gen_range(0..6) {
                    0 => {
                        let _ = rent(&db, who.id, stall, rng.gen_range(1..=3), Some("Shop"), now).await;
                    }
                    1 | 2 => {
                        let kind = kinds[rng.gen_range(0..2)];
                        let stack: Option<Uuid> =
                            sqlx::query_scalar("SELECT id FROM items WHERE container_id = $1 AND item_type = $2 LIMIT 1")
                                .bind(who.inventory)
                                .bind(kind)
                                .fetch_optional(&db)
                                .await
                                .unwrap();
                        if let Some(stack) = stack {
                            let req = stock_req(stack, rng.gen_range(1..20), rng.gen_range(1..5), None);
                            let _ = stock(&db, who.id, &req, now).await;
                        }
                    }
                    3 | 4 => {
                        let listing: Option<(Uuid, String)> = sqlx::query_as(
                            "SELECT id, stall_id FROM bazaar_listings WHERE stall_id IS NOT NULL ORDER BY random() LIMIT 1",
                        )
                        .fetch_optional(&db)
                        .await
                        .unwrap();
                        if let Some((listing, at)) = listing {
                            let _ = buy(&db, who.id, listing, &at, &buy_req(rng.gen_range(1..4)), now).await;
                        }
                    }
                    _ => {
                        let mine: Option<Uuid> =
                            sqlx::query_scalar("SELECT id FROM bazaar_listings WHERE player_id = $1 LIMIT 1")
                                .bind(who.id)
                                .fetch_optional(&db)
                                .await
                                .unwrap();
                        if let Some(listing) = mine {
                            let _ = unlist(&db, who.id, listing, now).await;
                        }
                    }
                }
                now += Duration::hours(rng.gen_range(0..12));
                sweep(&db, now).await.unwrap();

                let rent_paid: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(-SUM(amount), 0)::bigint FROM currency_transactions
                     WHERE transaction_type = 'bazaar_rent' AND player_id = ANY($1)",
                )
                .bind(people.iter().map(|p| p.id).collect::<Vec<_>>())
                .fetch_one(&db)
                .await
                .unwrap();
                let mut held = 0;
                for p in &people {
                    held += balance(&db, p.id).await;
                }
                assert_eq!(held + rent_paid, 1200, "seed {seed} step {step}");
                for kind in kinds {
                    assert_eq!(every_unit(&db, &refs, kind).await, start[kind], "{kind}, seed {seed} step {step}");
                }
            }
            for p in &people {
                let audited: i64 =
                    sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0)::bigint FROM currency_transactions WHERE player_id = $1")
                        .bind(p.id)
                        .fetch_one(&db)
                        .await
                        .unwrap();
                assert_eq!(balance(&db, p.id).await, 400 + audited, "seed {seed}");
            }
            // Clear the stalls for the next run's merchants
            sqlx::query("DELETE FROM bazaar_listings").execute(&db).await.unwrap();
            sqlx::query("DELETE FROM items WHERE container_id = (SELECT id FROM containers WHERE container_type = 'bazaar_stock')")
                .execute(&db)
                .await
                .unwrap();
            sqlx::query("DELETE FROM bazaar_rentals").execute(&db).await.unwrap();
        }
    }
}
