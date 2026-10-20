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
//! Units remember who made them (#231): each count is per quality and maker,
//! so a stack can hold Ada's 92s and Bo's 92s apart. The rules (the mean, the
//! default pick) are `rules::stacking::Units`, shared with the clients, and
//! they don't see makers: a take names qualities, and the database chooses
//! whose units at each quality go (unmarked first, then by maker id). The
//! take hands back the [`Marked`] units it took, and a put writes them, so a
//! split, a trade or the market keeps the marks.

use std::collections::{BTreeMap, HashMap};

#[cfg(test)]
use capitol_protocol::rules::stacking::Pick;
use capitol_protocol::rules::stacking::Units;
use sqlx::PgConnection;
use uuid::Uuid;

/// Units with their makers' marks (#231): a count per quality and maker,
/// `None` for no mark. What a take hands on and a put writes. The picks and
/// the mean stay on [`Units`], [`Marked::units`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marked(BTreeMap<(i32, Option<Uuid>), i32>);

impl Marked {
    /// Every one of `units` made by `maker`
    pub fn by(maker: Option<Uuid>, units: &Units) -> Self {
        Self(units.pairs().map(|(quality, count)| ((quality, maker), count)).collect())
    }

    /// From `(quality, maker, count)` rows; a key given twice adds up
    pub fn from_rows(rows: impl IntoIterator<Item = (i32, Option<Uuid>, i32)>) -> Self {
        let mut marked = Self::default();
        for (quality, maker, count) in rows.into_iter().filter(|(.., count)| *count > 0) {
            *marked.0.entry((quality, maker)).or_default() += count;
        }
        marked
    }

    /// `(quality, maker, count)`, lowest quality first, unmarked first within it
    pub fn rows(&self) -> impl Iterator<Item = (i32, Option<Uuid>, i32)> + '_ {
        self.0.iter().map(|(&(quality, maker), &count)| (quality, maker, count))
    }

    /// The units, without their marks
    pub fn units(&self) -> Units {
        Units::from_pairs(self.rows().map(|(quality, _, count)| (quality, count)))
    }

    /// The maker of most of them, the one whose best unit is better breaking
    /// a tie. `None` when there are more unmarked units than any maker's.
    pub fn lead_maker(&self) -> Option<Uuid> {
        let mut makers: BTreeMap<Option<Uuid>, (i64, i32)> = BTreeMap::new();
        for (quality, maker, count) in self.rows() {
            let entry = makers.entry(maker).or_default();
            entry.0 += count as i64;
            entry.1 = entry.1.max(quality);
        }
        makers.into_iter().max_by_key(|&(maker, (count, best))| (count, best, maker.is_none())).and_then(|(maker, _)| maker)
    }

    /// As three arrays, qualities, makers and counts, for `unnest`
    fn columns(&self) -> (Vec<i16>, Vec<Option<Uuid>>, Vec<i32>) {
        let mut columns = (Vec::new(), Vec::new(), Vec::new());
        for (quality, maker, count) in self.rows() {
            columns.0.push(quality as i16);
            columns.1.push(maker);
            columns.2.push(count);
        }
        columns
    }
}

/// The units of stack `item_id`, locked for the caller's transaction. Empty if
/// it's gone.
pub async fn units_of(conn: &mut PgConnection, item_id: Uuid) -> Result<Units, sqlx::Error> {
    // A row a quality and maker: from_pairs adds a quality's up
    let rows: Vec<(i16, i32)> =
        sqlx::query_as("SELECT quality, count FROM item_units WHERE item_id = $1 ORDER BY quality FOR UPDATE")
            .bind(item_id)
            .fetch_all(&mut *conn)
            .await?;
    Ok(Units::from_pairs(rows.into_iter().map(|(q, c)| (q as i32, c))))
}

/// The units of stack `item_id` with their marks, locked for the caller's
/// transaction. Empty if it's gone.
#[cfg(test)]
pub async fn marked_of(conn: &mut PgConnection, item_id: Uuid) -> Result<Marked, sqlx::Error> {
    let rows: Vec<(i16, Option<Uuid>, i32)> = sqlx::query_as(
        "SELECT quality, maker_id, count FROM item_units WHERE item_id = $1 ORDER BY quality, maker_id FOR UPDATE",
    )
    .bind(item_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Marked::from_rows(rows.into_iter().map(|(q, m, c)| (q as i32, m, c))))
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

/// Put `units` onto the stack `item_id`, each onto its quality and maker's
/// count. Fails rather than overflow a count.
pub async fn add(conn: &mut PgConnection, item_id: Uuid, units: &Marked) -> Result<(), sqlx::Error> {
    let (qualities, makers, counts) = units.columns();
    sqlx::query(
        "INSERT INTO item_units (item_id, quality, maker_id, count)
         SELECT $1, q, m, c FROM unnest($2::smallint[], $3::uuid[], $4::integer[]) AS t(q, m, c)
         ON CONFLICT ON CONSTRAINT item_units_key DO UPDATE SET count = item_units.count + EXCLUDED.count",
    )
    .bind(item_id)
    .bind(qualities)
    .bind(makers)
    .bind(counts)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Move every unit of stack `from` onto stack `into`, marks and all, and
/// delete `from`. Returns what `into` holds now.
pub async fn merge_into(conn: &mut PgConnection, from: Uuid, into: Uuid) -> Result<Units, sqlx::Error> {
    let moving = units_of(conn, from).await?;
    let mut merged = units_of(conn, into).await?;
    if !merged.merge(&moving) {
        return Err(sqlx::Error::Protocol("Too many for one stack".into()));
    }
    sqlx::query(
        "WITH moved AS (DELETE FROM item_units WHERE item_id = $1 RETURNING quality, maker_id, count)
         INSERT INTO item_units (item_id, quality, maker_id, count)
         SELECT $2, quality, maker_id, count FROM moved
         ON CONFLICT ON CONSTRAINT item_units_key DO UPDATE SET count = item_units.count + EXCLUDED.count",
    )
    .bind(from)
    .bind(into)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM items WHERE id = $1").bind(from).execute(&mut *conn).await?;
    Ok(merged)
}

/// What a take left behind.
#[derive(Debug, Clone, PartialEq)]
pub struct Taken {
    /// The units taken
    pub units: Units,
    /// The same, with their makers' marks, to put down elsewhere
    pub marked: Marked,
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
    let marked = write_take(conn, item_id, &units, &held).await?;
    Ok(Some(Taken { units, marked, left: held }))
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

/// Which rows a take of `t(i, q, c)` (stack, quality, count) lowers, and by
/// how many (`n`): at each quality, the unmarked units first, then by maker
/// id, until `c` are taken. The caller has the units locked and knows the
/// stack holds them.
const TAKE_PLAN: &str = "plan AS (
         SELECT u.item_id, u.quality, u.maker_id, u.count,
                LEAST(u.count, t.c - (SUM(u.count) OVER w - u.count))::INTEGER AS n
         FROM item_units u JOIN t ON t.i = u.item_id AND t.q = u.quality
         WINDOW w AS (PARTITION BY u.item_id, u.quality ORDER BY u.maker_id NULLS FIRST)
     ),
     taking AS (SELECT * FROM plan WHERE n > 0),
     gone AS (
         DELETE FROM item_units u USING taking k
         WHERE u.item_id = k.item_id AND u.quality = k.quality
           AND u.maker_id IS NOT DISTINCT FROM k.maker_id AND k.n = k.count
     ),
     cut AS (
         UPDATE item_units u SET count = u.count - k.n FROM taking k
         WHERE u.item_id = k.item_id AND u.quality = k.quality
           AND u.maker_id IS NOT DISTINCT FROM k.maker_id AND k.n < k.count
     )";

/// Write a take the caller has already worked out: `taken` off the stack
/// `item_id`, whose units (locked) less `taken` are `left`. Deletes the stack
/// when `left` is empty. Returns the units taken with their marks.
pub async fn write_take(conn: &mut PgConnection, item_id: Uuid, taken: &Units, left: &Units) -> Result<Marked, sqlx::Error> {
    let rows: Vec<(i16, Option<Uuid>, i32)> = if left.is_empty() {
        // Every unit goes, and the stack with them. The cascade at the end of
        // the statement finds them gone already.
        sqlx::query_as(
            "WITH gone AS (DELETE FROM item_units WHERE item_id = $1 RETURNING quality, maker_id, count),
             stack AS (DELETE FROM items WHERE id = $1)
             SELECT quality, maker_id, count FROM gone",
        )
        .bind(item_id)
        .fetch_all(&mut *conn)
        .await?
    } else {
        // A row taken whole goes and the rest go down, in one statement. The
        // rows are disjoint, and the summaries' trigger runs after both.
        let (qualities, counts) = columns(taken);
        sqlx::query_as(&format!(
            "WITH t(i, q, c) AS (SELECT $1::uuid, * FROM unnest($2::smallint[], $3::integer[])),
             {TAKE_PLAN}
             SELECT quality, maker_id, n FROM taking"
        ))
        .bind(item_id)
        .bind(qualities)
        .bind(counts)
        .fetch_all(&mut *conn)
        .await?
    };
    Ok(Marked::from_rows(rows.into_iter().map(|(q, m, c)| (q as i32, m, c))))
}

/// [`write_take`] for several stacks in one statement (#212): each of `takes`
/// is (stack, taken, left), the stack's units locked. A stack whose `left` is
/// empty is deleted, and its units with it. Nothing is handed on, so the
/// marks taken aren't returned: this is for using units up.
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
    sqlx::query(&format!(
        "WITH t(i, q, c) AS (SELECT * FROM unnest($1::uuid[], $2::smallint[], $3::integer[])),
         emptied AS (DELETE FROM items WHERE id = ANY($4)),
         {TAKE_PLAN}
         SELECT 1"
    ))
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

/// Make a new stack holding `units` (which mustn't be empty), marks and all.
/// Returns its id.
pub async fn insert_stack(conn: &mut PgConnection, stack: NewStack<'_>, units: &Marked) -> Result<Uuid, sqlx::Error> {
    let plain = units.units();
    let (Some(quality), Ok(_)) = (plain.mean(), i32::try_from(plain.count())) else {
        return Err(sqlx::Error::Protocol("A new stack needs at least one unit".into()));
    };
    let id = Uuid::new_v4();
    // The stack and its units in one statement. At quantity 0 the insert
    // seeds no units of its own, and the summaries' trigger, which runs at
    // the end of the statement, sets the quantity from the units written.
    let (qualities, makers, counts) = units.columns();
    sqlx::query(
        "WITH stack AS (
             INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
             VALUES ($1, $2, $3, 0, $4, $5, $6, $7)
         )
         INSERT INTO item_units (item_id, quality, maker_id, count)
         SELECT $1, q, m, c FROM unnest($8::smallint[], $9::uuid[], $10::integer[]) AS t(q, m, c)",
    )
    .bind(id)
    .bind(stack.item_type)
    .bind(quality)
    .bind(stack.owner_id)
    .bind(stack.container_id)
    .bind(stack.slot_index)
    .bind(stack.durability)
    .bind(qualities)
    .bind(makers)
    .bind(counts)
    .execute(&mut *conn)
    .await?;
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
    units: &Marked,
) -> Result<bool, sqlx::Error> {
    let Some(id) = first_stack(conn, container_id, item_type).await? else {
        return Ok(false);
    };
    let mut held = units_of(conn, id).await?;
    if !held.merge(&units.units()) {
        return Ok(false);
    }
    add(conn, id, units).await?;
    Ok(true)
}

/// Helpers for the tests of every path that moves units: each checks the
/// multiset is conserved and the summaries match the units.
#[cfg(test)]
pub mod test_units {
    use super::Marked;
    use capitol_protocol::rules::stacking::Units;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// A stack of `units` of `item_type` in `slot` of `container`, unmarked
    pub async fn stack_of(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, item_type: &str, units: &Units) -> Uuid {
        marked_stack_of(db, owner, container, slot, item_type, &Marked::by(None, units)).await
    }

    /// [`stack_of`] with makers' marks
    pub async fn marked_stack_of(db: &PgPool, owner: Uuid, container: Uuid, slot: i32, item_type: &str, units: &Marked) -> Uuid {
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

    /// The units of `item_id` with their marks, after checking its summaries
    pub async fn marked(db: &PgPool, item_id: Uuid) -> Marked {
        units(db, item_id).await;
        let mut conn = db.acquire().await.unwrap();
        super::marked_of(&mut conn, item_id).await.unwrap()
    }

    /// Every unit of `item_type` that `owner` holds, in any stack, with marks
    pub async fn all_marked(db: &PgPool, owner: Uuid, item_type: &str) -> Marked {
        let rows: Vec<(i16, Option<Uuid>, i32)> = sqlx::query_as(
            "SELECT u.quality, u.maker_id, u.count FROM item_units u JOIN items i ON i.id = u.item_id
             WHERE i.owner_id = $1 AND i.item_type = $2",
        )
        .bind(owner)
        .bind(item_type)
        .fetch_all(db)
        .await
        .unwrap();
        Marked::from_rows(rows.into_iter().map(|(q, m, c)| (q as i32, m, c)))
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
        owner_and_box_named(db, "ada").await
    }

    async fn owner_and_box_named(db: &PgPool, name: &str) -> (Uuid, Uuid) {
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

    /// Units remember who made them (#231): a take at a quality takes the
    /// unmarked units first, then by maker, and hands back whose they were;
    /// a merge and a new stack keep every mark
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn marks_go_where_the_units_go(db: PgPool) {
        let (ada, inventory) = owner_and_box(&db).await;
        let (bo, _) = owner_and_box_named(&db, "bo").await;
        let (low, high) = if ada < bo { (ada, bo) } else { (bo, ada) };
        let held = Marked::from_rows([(92, None, 1), (92, Some(high), 2), (92, Some(low), 2), (60, Some(ada), 3)]);
        let id = marked_stack_of(&db, ada, inventory, 0, "fiber", &held).await;
        assert_eq!(marked(&db, id).await, held);
        assert_eq!(units(&db, id).await, Units::from_pairs([(60, 3), (92, 5)]));

        let mut conn = db.acquire().await.unwrap();
        let taken = take_units(&mut conn, id, &Units::from_pairs([(92, 2), (60, 1)])).await.unwrap().unwrap();
        assert_eq!(taken.marked, Marked::from_rows([(92, None, 1), (92, Some(low), 1), (60, Some(ada), 1)]));
        assert_eq!(taken.marked.units(), taken.units);

        // Put down as a stack of its own, then merged back: nothing lost, no mark moved
        let split = insert_stack(
            &mut conn,
            NewStack { item_type: "fiber", owner_id: Some(ada), container_id: Some(inventory), slot_index: Some(1), durability: None },
            &taken.marked,
        )
        .await
        .unwrap();
        merge_into(&mut conn, split, id).await.unwrap();
        // Every unit of the stack, the stack deleted with them
        let rest = take_units(&mut conn, id, &Units::from_pairs([(60, 3), (92, 5)])).await.unwrap().unwrap();
        drop(conn);
        assert_eq!(rest.marked, held);
        assert!(rest.left.is_empty());
    }

    #[test]
    fn the_lead_maker_made_most_of_them() {
        let (ada, bo) = (Some(Uuid::from_u128(1)), Some(Uuid::from_u128(2)));
        assert_eq!(Marked::by(ada, &Units::uniform(3, 50)).lead_maker(), ada);
        assert_eq!(Marked::from_rows([(50, ada, 2), (40, bo, 3)]).lead_maker(), bo);
        // A tie goes to the better work
        assert_eq!(Marked::from_rows([(50, ada, 2), (90, bo, 2)]).lead_maker(), bo);
        // More unmarked than anyone's: no one made it, as far as anyone knows
        assert_eq!(Marked::from_rows([(50, ada, 2), (90, None, 3)]).lead_maker(), None);
        assert_eq!(Marked::from_rows([(90, ada, 2), (90, None, 2)]).lead_maker(), None, "a tie with the unmarked goes to the seller");
        assert_eq!(Marked::default().lead_maker(), None);
    }
}
