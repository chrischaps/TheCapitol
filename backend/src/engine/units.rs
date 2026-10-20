//! A stack's units in the database (E1.8, plan D2): `item_units` holds a
//! count per quality under every stack, and it's the truth. A trigger keeps
//! `items.quantity`, `quality` (the mean) and `min_quality` in step with it
//! and refuses any other write to them, so every change to what a stack holds
//! comes through here:
//!
//! - [`add`] puts units onto a stack, and [`merge_into`] moves a whole stack
//!   onto another. Nothing is diluted.
//! - [`take`] and [`take_units`] lower a stack by named units, and delete it
//!   when its last unit goes.
//! - [`insert_stack`] makes a new stack holding given units.
//!
//! The rules (the mean, the default pick) are `rules::stacking::Units`, shared
//! with the clients.

use std::collections::HashMap;

#[cfg(test)]
use capitol_protocol::rules::stacking::Pick;
use capitol_protocol::rules::stacking::Units;
use sqlx::PgConnection;
use uuid::Uuid;

/// The units of stack `item_id`, locked for the caller's transaction. Empty if
/// it's gone.
pub async fn units_of(conn: &mut PgConnection, item_id: Uuid) -> Result<Units, sqlx::Error> {
    let rows: Vec<(i16, i32)> =
        sqlx::query_as("SELECT quality, count FROM item_units WHERE item_id = $1 ORDER BY quality FOR UPDATE")
            .bind(item_id)
            .fetch_all(&mut *conn)
            .await?;
    Ok(Units::from_pairs(rows.into_iter().map(|(q, c)| (q as i32, c))))
}

/// [`units_of`] for several stacks in one round trip (#212). A stack that's
/// gone isn't in it. Lock the stacks' `items` rows first, in a statement of
/// their own: every change to a stack's units also writes its `items` row
/// (the summaries' trigger), so then no one else's is half done when this
/// reads them.
pub async fn units_of_many(conn: &mut PgConnection, item_ids: &[Uuid]) -> Result<HashMap<Uuid, Units>, sqlx::Error> {
    if item_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, i16, i32)> = sqlx::query_as(
        "SELECT item_id, quality, count FROM item_units WHERE item_id = ANY($1) ORDER BY item_id, quality FOR UPDATE",
    )
    .bind(item_ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut pairs: HashMap<Uuid, Vec<(i32, i32)>> = HashMap::new();
    for (item_id, quality, count) in rows {
        pairs.entry(item_id).or_default().push((quality as i32, count));
    }
    Ok(pairs.into_iter().map(|(id, p)| (id, Units::from_pairs(p))).collect())
}

/// `units` as two arrays, qualities and counts, for `unnest`: one statement
/// for every quality, not a round trip each (#208).
fn columns(units: &Units) -> (Vec<i16>, Vec<i32>) {
    units.pairs().map(|(quality, count)| (quality as i16, count)).unzip()
}

/// Put `units` onto the stack `item_id`. Fails rather than overflow a count.
pub async fn add(conn: &mut PgConnection, item_id: Uuid, units: &Units) -> Result<(), sqlx::Error> {
    let (qualities, counts) = columns(units);
    sqlx::query(
        "INSERT INTO item_units (item_id, quality, count)
         SELECT $1, q, c FROM unnest($2::smallint[], $3::integer[]) AS t(q, c)
         ON CONFLICT (item_id, quality) DO UPDATE SET count = item_units.count + EXCLUDED.count",
    )
    .bind(item_id)
    .bind(qualities)
    .bind(counts)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Move every unit of stack `from` onto stack `into`, and delete `from`.
/// Returns what `into` holds now.
pub async fn merge_into(conn: &mut PgConnection, from: Uuid, into: Uuid) -> Result<Units, sqlx::Error> {
    let moving = units_of(conn, from).await?;
    let mut merged = units_of(conn, into).await?;
    if !merged.merge(&moving) {
        return Err(sqlx::Error::Protocol("Too many for one stack".into()));
    }
    add(conn, into, &moving).await?;
    sqlx::query("DELETE FROM items WHERE id = $1").bind(from).execute(&mut *conn).await?;
    Ok(merged)
}

/// What a take left behind.
#[derive(Debug, Clone, PartialEq)]
pub struct Taken {
    /// The units taken
    pub units: Units,
    /// The units left; empty when the stack was used up and deleted
    pub left: Units,
}

/// Take exactly `wanted` off the stack `item_id`, deleting it if that's every
/// unit. `None`, and nothing changed, unless it holds all of them.
pub async fn take_units(
    conn: &mut PgConnection,
    item_id: Uuid,
    wanted: &Units,
) -> Result<Option<Taken>, sqlx::Error> {
    let mut held = units_of(conn, item_id).await?;
    let Some(units) = held.take_units(wanted) else {
        return Ok(None);
    };
    write_take(conn, item_id, &units, &held).await?;
    Ok(Some(Taken { units, left: held }))
}

/// Take `n` units off the stack `item_id`, chosen by `pick`, deleting it if
/// that's every unit. `None`, and nothing changed, if it holds fewer.
#[cfg(test)]
pub async fn take(
    conn: &mut PgConnection,
    item_id: Uuid,
    n: i32,
    pick: Pick,
) -> Result<Option<Taken>, sqlx::Error> {
    let held = units_of(conn, item_id).await?;
    match held.pick(n, pick) {
        Some(wanted) => take_units(conn, item_id, &wanted).await,
        None => Ok(None),
    }
}

/// Write a take the caller has already worked out: `taken` off the stack
/// `item_id`, whose units (locked) less `taken` are `left`. Deletes the stack
/// when `left` is empty.
pub async fn write_take(conn: &mut PgConnection, item_id: Uuid, taken: &Units, left: &Units) -> Result<(), sqlx::Error> {
    if left.is_empty() {
        sqlx::query("DELETE FROM items WHERE id = $1").bind(item_id).execute(&mut *conn).await?;
        return Ok(());
    }
    // A quality taken whole goes and the rest go down, in one statement. The
    // rows are disjoint, and the summaries' trigger runs after both.
    let (qualities, counts) = columns(taken);
    sqlx::query(
        "WITH t(q, c) AS (SELECT * FROM unnest($2::smallint[], $3::integer[])),
         gone AS (
             DELETE FROM item_units u USING t WHERE u.item_id = $1 AND u.quality = t.q AND u.count = t.c
         )
         UPDATE item_units u SET count = u.count - t.c
         FROM t WHERE u.item_id = $1 AND u.quality = t.q AND u.count > t.c",
    )
    .bind(item_id)
    .bind(qualities)
    .bind(counts)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// [`write_take`] for several stacks in one statement (#212): each of `takes`
/// is (stack, taken, left), the stack's units locked. A stack whose `left` is
/// empty is deleted, and its units with it.
pub async fn write_takes(conn: &mut PgConnection, takes: &[(Uuid, &Units, &Units)]) -> Result<(), sqlx::Error> {
    let emptied: Vec<Uuid> = takes.iter().filter(|(_, _, left)| left.is_empty()).map(|(id, ..)| *id).collect();
    let (mut ids, mut qualities, mut counts) = (Vec::new(), Vec::new(), Vec::new());
    for (id, taken, _) in takes.iter().filter(|(_, _, left)| !left.is_empty()) {
        for (quality, count) in taken.pairs() {
            ids.push(*id);
            qualities.push(quality as i16);
            counts.push(count);
        }
    }
    // As write_take, over disjoint rows: an emptied stack's units go with its
    // item (the cascade), and none of them is in t
    sqlx::query(
        "WITH t(i, q, c) AS (SELECT * FROM unnest($1::uuid[], $2::smallint[], $3::integer[])),
         emptied AS (DELETE FROM items WHERE id = ANY($4)),
         gone AS (
             DELETE FROM item_units u USING t WHERE u.item_id = t.i AND u.quality = t.q AND u.count = t.c
         )
         UPDATE item_units u SET count = u.count - t.c
         FROM t WHERE u.item_id = t.i AND u.quality = t.q AND u.count > t.c",
    )
    .bind(ids)
    .bind(qualities)
    .bind(counts)
    .bind(emptied)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Where a new stack goes, and who owns it.
#[derive(Debug, Clone)]
pub struct NewStack<'a> {
    pub item_type: &'a str,
    pub owner_id: Option<Uuid>,
    pub container_id: Option<Uuid>,
    pub slot_index: Option<i32>,
    pub durability: Option<i32>,
}

/// Make a new stack holding `units` (which mustn't be empty). Returns its id.
pub async fn insert_stack(conn: &mut PgConnection, stack: NewStack<'_>, units: &Units) -> Result<Uuid, sqlx::Error> {
    let (Some(quality), Ok(quantity)) = (units.mean(), i32::try_from(units.count())) else {
        return Err(sqlx::Error::Protocol("A new stack needs at least one unit".into()));
    };
    let id = Uuid::new_v4();
    // The insert seeds one uniform row at the mean; the real units replace it
    sqlx::query(
        "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(stack.item_type)
    .bind(quality)
    .bind(quantity)
    .bind(stack.owner_id)
    .bind(stack.container_id)
    .bind(stack.slot_index)
    .bind(stack.durability)
    .execute(&mut *conn)
    .await?;
    if *units != Units::uniform(quantity, quality) {
        sqlx::query("DELETE FROM item_units WHERE item_id = $1").bind(id).execute(&mut *conn).await?;
        add(conn, id, units).await?;
    }
    Ok(id)
}

/// The first stack of `item_type` in `container_id`, in slot order, locked
/// for the caller's transaction. Gathering, craft output, station returns and
/// the exchange withdraw join it whatever its quality (D2: nothing is
/// diluted), and take a slot of their own when it's `None`.
pub async fn first_stack(conn: &mut PgConnection, container_id: Uuid, item_type: &str) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM items
         WHERE container_id = $1 AND item_type = $2
         ORDER BY slot_index NULLS LAST, id
         LIMIT 1
         FOR UPDATE",
    )
    .bind(container_id)
    .bind(item_type)
    .fetch_optional(&mut *conn)
    .await
}

/// Join `units` onto the first stack of `item_type` in `container_id`. `false`,
/// and nothing changed, if there's none, or it can't hold that many more.
pub async fn join_first_stack(
    conn: &mut PgConnection,
    container_id: Uuid,
    item_type: &str,
    units: &Units,
) -> Result<bool, sqlx::Error> {
    let Some(id) = first_stack(conn, container_id, item_type).await? else {
        return Ok(false);
    };
    let mut held = units_of(conn, id).await?;
    if !held.merge(units) {
        return Ok(false);
    }
    add(conn, id, units).await?;
    Ok(true)
}

/// Helpers for the tests of every path that moves units: each checks the
/// multiset is conserved and the summaries match the units.
#[cfg(test)]
pub mod test_units {
    use capitol_protocol::rules::stacking::Units;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// A stack of `units` of `item_type` in `slot` of `container`
    pub async fn stack_of(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, item_type: &str, units: &Units) -> Uuid {
        let mut conn = db.acquire().await.unwrap();
        super::insert_stack(
            &mut conn,
            super::NewStack {
                item_type,
                owner_id: Some(owner),
                container_id: Some(container),
                slot_index: Some(slot),
                durability: None,
            },
            units,
        )
        .await
        .unwrap()
    }

    /// The units of `item_id`, after checking its summaries match them
    pub async fn units(db: &PgPool, item_id: Uuid) -> Units {
        let mut conn = db.acquire().await.unwrap();
        let units = super::units_of(&mut conn, item_id).await.unwrap();
        let (quantity, quality, min): (i32, i32, i32) =
            sqlx::query_as("SELECT quantity, quality, min_quality FROM items WHERE id = $1")
                .bind(item_id)
                .fetch_one(&mut *conn)
                .await
                .unwrap();
        assert_eq!(
            (quantity as i64, Some(quality), Some(min)),
            (units.count(), units.mean(), units.min()),
            "item {item_id}'s summaries don't match its units"
        );
        units
    }

    /// Every unit of `item_type` that `owner` holds, in any stack
    pub async fn all_held(db: &PgPool, owner: Uuid, item_type: &str) -> Units {
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM items WHERE owner_id = $1 AND item_type = $2")
            .bind(owner)
            .bind(item_type)
            .fetch_all(db)
            .await
            .unwrap();
        let mut all = Units::new();
        for id in ids {
            all.merge(&units(db, id).await);
        }
        all
    }

    /// Every unit of `item_type` in `container`, in any stack
    pub async fn all_in(db: &PgPool, container: Uuid, item_type: &str) -> Units {
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM items WHERE container_id = $1 AND item_type = $2")
            .bind(container)
            .bind(item_type)
            .fetch_all(db)
            .await
            .unwrap();
        let mut all = Units::new();
        for id in ids {
            all.merge(&units(db, id).await);
        }
        all
    }
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test units -- --ignored`.
#[cfg(test)]
mod tests {
    use super::test_units::*;
    use super::*;
    use sqlx::PgPool;

    async fn owner_and_box(db: &PgPool) -> (Uuid, Uuid) {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, 'ada@test', 'x')")
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, 'ada')")
            .bind(id)
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        let inventory = Uuid::new_v4();
        sqlx::query("INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2)")
            .bind(inventory)
            .bind(id)
            .execute(db)
            .await
            .unwrap();
        (id, inventory)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn two_fifties_and_two_nineties_show_70_and_a_ninety_comes_out_a_ninety(db: PgPool) {
        let (ada, inventory) = owner_and_box(&db).await;
        let fifties = stack_of(&db, ada, inventory, 0, "fiber", &Units::uniform(2, 50)).await;
        let nineties = stack_of(&db, ada, inventory, 1, "fiber", &Units::uniform(2, 90)).await;

        let mut conn = db.acquire().await.unwrap();
        let merged = merge_into(&mut conn, nineties, fifties).await.unwrap();
        assert_eq!(merged.mean(), Some(70));
        let shown: i32 = sqlx::query_scalar("SELECT quality FROM items WHERE id = $1")
            .bind(fifties)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(shown, 70);

        let taken = take_units(&mut conn, fifties, &Units::uniform(1, 90)).await.unwrap().unwrap();
        assert_eq!(taken.units, Units::uniform(1, 90));
        drop(conn);
        let left = units(&db, fifties).await;
        assert_eq!((left.count(), left.mean()), (3, Some(63)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_summaries_only_follow_the_units(db: PgPool) {
        let (ada, inventory) = owner_and_box(&db).await;
        let id = stack_of(&db, ada, inventory, 0, "fiber", &Units::from_pairs([(40, 1), (95, 3)])).await;
        assert_eq!(units(&db, id).await.mean(), Some(81));
        for write in [
            "UPDATE items SET quantity = 9 WHERE id = $1",
            "UPDATE items SET quality = 99 WHERE id = $1",
            "UPDATE items SET min_quality = 99 WHERE id = $1",
        ] {
            let refused = sqlx::query(write).bind(id).execute(&db).await;
            assert!(refused.is_err(), "{write} went through");
        }
        // Moving it is fine
        sqlx::query("UPDATE items SET slot_index = 3 WHERE id = $1").bind(id).execute(&db).await.unwrap();
        // A plain insert is uniform at its quality, whatever min it names
        let fresh = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, min_quality) VALUES ($1, 'fiber', 70, 4, $2, 10)",
        )
        .bind(fresh)
        .bind(ada)
        .execute(&db)
        .await
        .unwrap();
        assert_eq!(units(&db, fresh).await, Units::uniform(4, 70));
    }

    /// Stacks from before E1.8 become one unit row each, at their quality: a
    /// blend made by E1.2's averaging lost its history, so it's uniform now
    #[sqlx::test(migrations = false)]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_migration_makes_each_old_stack_one_unit_row(db: PgPool) {
        const ITEM_UNITS: i64 = 20261103;
        let all = sqlx::migrate!("./migrations");
        let mut before = sqlx::migrate!("./migrations");
        before.migrations = all.iter().filter(|m| m.version < ITEM_UNITS).cloned().collect::<Vec<_>>().into();
        before.run(&db).await.unwrap();

        let (ada, inventory) = owner_and_box(&db).await;
        let (plain, blend) = (Uuid::new_v4(), Uuid::new_v4());
        for (id, slot, quality, min_quality, quantity) in [(plain, 0, 60, 60, 5), (blend, 1, 91, 40, 10)] {
            sqlx::query(
                "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, min_quality)
                 VALUES ($1, 'fiber', $2, $3, $4, $5, $6, $7)",
            )
            .bind(id)
            .bind(quality)
            .bind(quantity)
            .bind(ada)
            .bind(inventory)
            .bind(slot)
            .bind(min_quality)
            .execute(&db)
            .await
            .unwrap();
        }

        all.run(&db).await.unwrap();

        assert_eq!(units(&db, plain).await, Units::uniform(5, 60));
        assert_eq!(units(&db, blend).await, Units::uniform(10, 91));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_take_of_every_unit_deletes_the_stack_and_a_short_take_changes_nothing(db: PgPool) {
        let (ada, inventory) = owner_and_box(&db).await;
        let id = stack_of(&db, ada, inventory, 0, "fiber", &Units::from_pairs([(40, 1), (95, 3)])).await;
        let mut conn = db.acquire().await.unwrap();
        assert_eq!(take(&mut conn, id, 5, Pick::Lowest).await.unwrap(), None);
        assert_eq!(take_units(&mut conn, id, &Units::uniform(2, 40)).await.unwrap(), None);
        let best = take(&mut conn, id, 2, Pick::Best).await.unwrap().unwrap();
        assert_eq!(best.units, Units::uniform(2, 95));
        let rest = take(&mut conn, id, 2, Pick::Lowest).await.unwrap().unwrap();
        assert_eq!((rest.units, rest.left), (Units::from_pairs([(40, 1), (95, 1)]), Units::new()));
        let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(gone, 0);
    }

    /// Several stacks read and taken from in a statement each (#212): a
    /// quality taken whole goes, the rest go down, and a stack emptied goes
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn many_stacks_are_read_and_taken_from_at_once(db: PgPool) {
        let (ada, inventory) = owner_and_box(&db).await;
        let mixed = stack_of(&db, ada, inventory, 0, "fiber", &Units::from_pairs([(40, 1), (95, 3)])).await;
        let plain = stack_of(&db, ada, inventory, 1, "fiber", &Units::uniform(4, 60)).await;
        let spent = stack_of(&db, ada, inventory, 2, "fiber", &Units::uniform(2, 70)).await;
        let mut conn = db.acquire().await.unwrap();
        let mut held = units_of_many(&mut conn, &[mixed, plain, spent, Uuid::new_v4()]).await.unwrap();
        assert_eq!(held.len(), 3, "a stack that isn't there isn't in it");
        assert_eq!(held[&mixed], Units::from_pairs([(40, 1), (95, 3)]));

        let mut takes = Vec::new();
        for (id, wanted) in
            [(mixed, Units::from_pairs([(40, 1), (95, 1)])), (plain, Units::uniform(1, 60)), (spent, Units::uniform(2, 70))]
        {
            let stack = held.get_mut(&id).unwrap();
            let taken = stack.take_units(&wanted).unwrap();
            takes.push((id, taken, stack.clone()));
        }
        let writes: Vec<(Uuid, &Units, &Units)> = takes.iter().map(|(id, t, l)| (*id, t, l)).collect();
        write_takes(&mut conn, &writes).await.unwrap();
        drop(conn);

        assert_eq!(units(&db, mixed).await, Units::uniform(2, 95));
        assert_eq!(units(&db, plain).await, Units::uniform(3, 60));
        let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE id = $1")
            .bind(spent)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(gone, 0, "the emptied stack is deleted");
    }
}
