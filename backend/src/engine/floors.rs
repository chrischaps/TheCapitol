//! Floors (Plots v2 §5.3 and §5.4, V3.3, #475): building upward.
//!
//! - **Who builds:** the owner of a home lot with a building
//!   (`plots.indoor_room`), urban, suburban or rural. A guild's hall keeps
//!   its own rules and has none.
//! - **What it costs:** `rules::floors::cost`, about twice the last floor
//!   each time, with no cap. A permit in Strands at the live
//!   `floor_permit_scale` (`TransactionType::FloorPermit`, a sink), and the
//!   building goods from the owner's pack, lowest grade first
//!   (`UnitFlow::Built`, a sink too). All of it or nothing, in one
//!   transaction with the plot locked, so two builds at once take turns.
//! - **How long:** `rules::floors::build_secs`, 15 minutes times the floor,
//!   at `SIM_TICK_SCALE`. `plots.floors` counts the new floor at once, and
//!   `floor_ready_at` says when it's up. Nothing ticks: a placement upstairs
//!   checks it (`station_placement`), and the next floor waits for it.
//! - **What it's worth:** the permit goes on the assessed value, so the tax
//!   follows the building, and on `plots.floor_permits`, so a release that
//!   refunds the deed refunds the permits with it (`plot_sync`, and a
//!   foreclosure less the debt, `property_tax`).
//! - **What it holds:** the footprint's indoor room again, a place of its
//!   own: `models::plot::STATION_CAPACITY` counts `indoor_room` once for the
//!   ground and once for each floor.

use capitol_protocol::ledger::UnitFlow;
use capitol_protocol::rest::FloorBill;
use capitol_protocol::rules::floors;
use capitol_protocol::tunables::Tunables;
use capitol_protocol::ws::ItemCount;
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::currency::{subtract_strands_in, TransactionType};
use super::gazette::{self, DeedEvent};
use super::ledger;
use super::weather::take_lowest;
use crate::error::AppError;

/// Whether a lot builds upward: it has a building with room indoors, and
/// it isn't a guild's hall
pub fn builds_up(indoor_room: Option<i32>, guild_id: Option<Uuid>) -> bool {
    indoor_room.is_some_and(|room| room > 0) && guild_id.is_none()
}

/// What the `k`th floor costs at the `tunables` now
pub fn bill(k: i32, tunables: &Tunables) -> FloorBill {
    let cost = floors::cost(k);
    FloorBill {
        floor: k.max(1),
        permit: floors::permit(k, tunables.floor_permit_scale),
        goods: cost
            .goods
            .into_iter()
            .map(|(item_type, n)| ItemCount { item_type: item_type.to_string(), quantity: n.min(i32::MAX as i64) as i32 })
            .collect(),
        build_secs: floors::build_secs(k),
    }
}

/// When a floor that takes `secs` of game time, begun at `now`, is up, at
/// `scale` times game time (`SIM_TICK_SCALE`)
pub fn ready_at(now: DateTime<Utc>, secs: i64, scale: f64) -> DateTime<Utc> {
    let ms = (secs as f64 * 1000.0 / scale.max(1e-9)).round().min(i64::MAX as f64 / 2.0) as i64;
    now + Duration::try_milliseconds(ms).unwrap_or(Duration::MAX)
}

/// A floor just begun
#[derive(Debug, Clone)]
pub struct Built {
    /// The `/terrain` lot, `None` for a plot from before B7
    pub terrain_id: Option<String>,
    /// The floors it has now, the new one counted
    pub floors: i32,
    /// When the new one is up
    pub floor_ready_at: DateTime<Utc>,
    /// The builder's Strands after the permit
    pub new_balance: i64,
    /// The goods it took
    pub used: Vec<ItemCount>,
    /// The builder's pack, which changed
    pub inventory_id: Uuid,
}

/// Begin the next floor on `plot_id` for `player_id`, its owner, at `now`,
/// paying its permit and goods. See the module docs.
pub async fn build(
    db: &PgPool,
    tunables: &Tunables,
    scale: f64,
    player_id: Uuid,
    plot_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Built, AppError> {
    let mut tx = db.begin().await?;
    type Row = (Option<Uuid>, Option<Uuid>, Option<i32>, i32, Option<DateTime<Utc>>, String, Option<String>, i32);
    let row: Option<Row> = sqlx::query_as(
        "SELECT owner_id, guild_id, indoor_room, floors, floor_ready_at, zone_id, terrain_id, assessed_value
         FROM plots WHERE id = $1 FOR UPDATE",
    )
    .bind(plot_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((owner_id, guild_id, indoor_room, built, ready, zone_id, terrain_id, assessed)) = row else {
        return Err(AppError::NotFound("Plot not found".into()));
    };
    if owner_id != Some(player_id) {
        return Err(AppError::Forbidden("You don't own this plot".into()));
    }
    if guild_id.is_some() {
        return Err(AppError::BadRequest("A guild's hall keeps its own rules: it has no floors to build".into()));
    }
    if !builds_up(indoor_room, guild_id) {
        return Err(AppError::BadRequest("This lot has no building to build on".into()));
    }
    if ready.is_some_and(|at| at > now) {
        let storey = floors::storey_name(built);
        return Err(AppError::BadRequest(format!("Wait until {storey} is up before you build on it")));
    }

    let k = built + 1;
    let bill = bill(k, tunables);
    let storey = floors::storey_name(k);
    // No cap on floors, but the assessed value is a whole number of Strands
    let Some(assessed) = i32::try_from(assessed as i64 + bill.permit).ok() else {
        return Err(AppError::BadRequest("The land office can't value a building that tall".into()));
    };

    let new_balance = subtract_strands_in(
        &mut tx,
        player_id,
        bill.permit,
        TransactionType::FloorPermit,
        Some(&format!("The permit for {storey}")),
        Some(plot_id),
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("The permit for {storey} is {} Strands: {e}", bill.permit)))?;

    let inventory_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'player_inventory' LIMIT 1 FOR UPDATE",
    )
    .bind(player_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("No inventory".into()))?;
    let mut flows = ledger::Flows::new();
    let what = floors::storey_title(k);
    for good in &bill.goods {
        take_lowest(&mut tx, inventory_id, &good.item_type, good.quantity, &what).await?;
        flows.add(&good.item_type, UnitFlow::Built, good.quantity as i64);
    }

    let floor_ready_at = ready_at(now, bill.build_secs, scale);
    sqlx::query(
        "UPDATE plots SET floors = $2, floor_ready_at = $3, floor_permits = floor_permits + $4, assessed_value = $5
         WHERE id = $1",
    )
    .bind(plot_id)
    .bind(k)
    .bind(floor_ready_at)
    .bind(bill.permit)
    .bind(assessed)
    .execute(&mut *tx)
    .await?;
    gazette::record_deed(&mut tx, plot_id, &zone_id, DeedEvent::Floor(k), player_id).await?;
    ledger::commit(tx, flows).await?;

    Ok(Built { terrain_id, floors: k, floor_ready_at, new_balance, used: bill.goods, inventory_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_home_lot_with_a_building_builds_up() {
        assert!(builds_up(Some(4), None));
        assert!(!builds_up(Some(0), None), "a lot that's all yard");
        assert!(!builds_up(None, None), "no building");
        assert!(!builds_up(Some(4), Some(Uuid::from_u128(1))), "a guild's hall");
    }

    #[test]
    fn the_bill_is_the_rules_at_the_tunable() {
        let mut t = Tunables::default();
        let first = bill(1, &t);
        assert_eq!((first.floor, first.permit, first.build_secs), (1, 400, 900));
        assert_eq!(first.goods[0], ItemCount { item_type: "plank".into(), quantity: 30 });
        t.floor_permit_scale = 2.0;
        assert_eq!(bill(2, &t).permit, 1600);
        assert_eq!(bill(80, &t).goods[0].quantity, i32::MAX, "held, never wrapped");
    }

    #[test]
    fn a_floor_goes_up_faster_at_a_faster_clock() {
        let now = DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap();
        assert_eq!(ready_at(now, 900, 1.0), now + Duration::minutes(15));
        assert_eq!(ready_at(now, 1800, 10.0), now + Duration::minutes(3));
    }

    /// The goods every floor's bill names are content's, with the target
    /// prices §5.4 priced it at
    #[test]
    fn the_goods_are_items_at_their_targets() {
        let content = crate::content::load().unwrap();
        for (item, target) in [("plank", 5), ("brick", 8), ("iron_nails", 6), ("iron_ingot", 50)] {
            let def = content.items.iter().find(|i| i.id == item).unwrap_or_else(|| panic!("{item}"));
            assert_eq!(def.target_price, Some(target), "{item}");
        }
    }

    /// These need the local Postgres (`docker-compose up -d`). Run them with
    /// `cargo test floors -- --ignored`.
    mod db {
        use super::*;
        use crate::engine::units::test_units::{all_held, stack_of};
        use capitol_protocol::rules::stacking::Units;

        /// A player with `strands` and an empty pack. Returns them and it.
        async fn builder(db: &PgPool, name: &str, strands: i64) -> (Uuid, Uuid) {
            let account = Uuid::new_v4();
            sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
                .bind(account)
                .bind(format!("{name}@test"))
                .execute(db)
                .await
                .unwrap();
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO players (id, account_id, name, strand_balance) VALUES ($1, $2, $3, $4) RETURNING id",
            )
            .bind(Uuid::new_v4())
            .bind(account)
            .bind(name)
            .bind(strands)
            .fetch_one(db)
            .await
            .unwrap();
            let bag: Uuid = sqlx::query_scalar(
                "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2) RETURNING id",
            )
            .bind(Uuid::new_v4())
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap();
            (id, bag)
        }

        /// `owner`'s urban lot, 90 along the street and 93 deep, assessed at
        /// `deed`, with its room worked out as the startup sync does
        async fn urban_lot(db: &PgPool, owner: Uuid, deed: i32) -> Uuid {
            let id = Uuid::new_v4();
            let corners = vec![1000.0, 1000.0, 1090.0, 1000.0, 1090.0, 1093.0, 1000.0, 1093.0];
            sqlx::query(
                "INSERT INTO plots (id, terrain_id, zone_id, grid_x, grid_y, world_x, world_y,
                                    bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y,
                                    size_category, owner_id, corners, assessed_value)
                 VALUES ($1, $2, 'urban', 0, 0, 1045, 1046, 1000, 1000, 1090, 1093, 'medium', $3, $4, $5)",
            )
            .bind(id)
            .bind(format!("urb_test_{id}"))
            .bind(owner)
            .bind(corners)
            .bind(deed)
            .execute(db)
            .await
            .unwrap();
            let mut tx = db.begin().await.unwrap();
            crate::engine::plot_sync::fill_rooms(&mut tx).await.unwrap();
            tx.commit().await.unwrap();
            id
        }

        /// What floor `k` needs, and `extra` more of each, the planks at two
        /// grades so the take shows its order, in slots from `k`'s tenth
        async fn stock(db: &PgPool, owner: Uuid, bag: Uuid, k: i32, extra: i32) {
            for (i, good) in bill(k, &Tunables::default()).goods.iter().enumerate() {
                let slot = k as usize * 10 + i;
                let units = if good.item_type == "plank" {
                    Units::from_pairs([(80, extra), (40, good.quantity)])
                } else {
                    Units::uniform(good.quantity + extra, 60)
                };
                stack_of(db, owner, bag, slot as i32, &good.item_type, &units).await;
            }
        }

        async fn balance(db: &PgPool, id: Uuid) -> i64 {
            sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1").bind(id).fetch_one(db).await.unwrap()
        }

        type Plot = (i32, Option<DateTime<Utc>>, i64, i32, i32);

        /// Its floors, when the last is up, its permits, its assessed value
        /// and its capacity
        async fn plot(db: &PgPool, id: Uuid) -> Plot {
            sqlx::query_as(&format!(
                "SELECT p.floors, p.floor_ready_at, p.floor_permits, p.assessed_value, {}
                 FROM plots p JOIN plot_size_categories psc ON psc.id = p.size_category WHERE p.id = $1",
                crate::models::STATION_CAPACITY
            ))
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
        }

        /// An urban lot goes 4 → 8 → 12 as floors are bought, each costing
        /// what `rules::floors::cost` says, and the next waits for the last
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn an_urban_lot_builds_up_four_stations_a_floor(db: PgPool) {
            crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
            let (ada, bag) = builder(&db, "ada", 2000).await;
            let lot = urban_lot(&db, ada, 1000).await;
            assert_eq!(plot(&db, lot).await.4, 4, "4 indoors, no yard");
            stock(&db, ada, bag, 1, 2).await;
            let t = Tunables::default();
            let now = Utc::now();

            let built = build(&db, &t, 1.0, ada, lot, now).await.unwrap();

            assert_eq!((built.floors, built.new_balance), (1, 1600));
            assert_eq!(built.used, bill(1, &t).goods);
            assert_eq!(all_held(&db, ada, "plank").await, Units::uniform(2, 80), "the lowest planks went");
            assert_eq!(all_held(&db, ada, "brick").await, Units::uniform(2, 60));
            let (floors, ready, permits, assessed, capacity) = plot(&db, lot).await;
            assert_eq!((floors, permits, assessed, capacity), (1, 400, 1400, 8));
            let micros = |t: Option<DateTime<Utc>>| t.map(|t| t.timestamp_micros());
            assert_eq!(micros(ready), micros(Some(ready_at(now, 900, 1.0))));
            let kind: String = sqlx::query_scalar("SELECT kind FROM deed_events WHERE plot_id = $1 AND floor = 1")
                .bind(lot)
                .fetch_one(&db)
                .await
                .unwrap();
            assert_eq!(kind, "floor", "for the Gazette");

            let early = build(&db, &t, 1.0, ada, lot, now).await.unwrap_err();
            assert!(matches!(&early, AppError::BadRequest(m) if m == "Wait until the first floor is up before you build on it"));

            // Up, but short of the second's goods: nothing changes
            let later = now + Duration::minutes(16);
            let short = build(&db, &t, 1.0, ada, lot, later).await.unwrap_err();
            assert!(matches!(&short, AppError::BadRequest(m) if m.contains("needs 40 plank")), "{short:?}");
            assert_eq!(balance(&db, ada).await, 1600);
            assert_eq!(plot(&db, lot).await.0, 1);

            stock(&db, ada, bag, 2, 0).await;
            let second = build(&db, &t, 1.0, ada, lot, later).await.unwrap();
            assert_eq!((second.floors, second.new_balance), (2, 800));
            let (floors, ready, permits, assessed, capacity) = plot(&db, lot).await;
            assert_eq!((floors, permits, assessed, capacity), (2, 1200, 2200, 12));
            assert_eq!(micros(ready), micros(Some(ready_at(later, 1800, 1.0))), "the second takes 30 minutes");
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn only_the_owner_builds_and_only_with_the_permit(db: PgPool) {
            crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
            let (ada, _) = builder(&db, "ada", 2000).await;
            let (bo, bo_bag) = builder(&db, "bo", 399).await;
            let lot = urban_lot(&db, ada, 1000).await;
            let theirs = build(&db, &Tunables::default(), 1.0, bo, lot, Utc::now()).await.unwrap_err();
            assert!(matches!(theirs, AppError::Forbidden(_)));

            let mine = urban_lot(&db, bo, 1000).await;
            stock(&db, bo, bo_bag, 1, 0).await;
            let broke = build(&db, &Tunables::default(), 1.0, bo, mine, Utc::now()).await.unwrap_err();
            assert!(matches!(&broke, AppError::BadRequest(m) if m.starts_with("The permit for the first floor is 400 Strands")));
            assert_eq!(all_held(&db, bo, "plank").await.count(), 30, "the goods stay");
        }

        /// A release clears the floors, as it does the stations (V3.3)
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_release_takes_the_floors_down(db: PgPool) {
            crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
            let (ada, bag) = builder(&db, "ada", 2000).await;
            let lot = urban_lot(&db, ada, 1000).await;
            stock(&db, ada, bag, 1, 0).await;
            build(&db, &Tunables::default(), 1.0, ada, lot, Utc::now()).await.unwrap();
            let mut tx = db.begin().await.unwrap();
            crate::engine::plot_sync::release_plot(&mut tx, lot).await.unwrap();
            tx.commit().await.unwrap();
            assert_eq!(plot(&db, lot).await, (0, None, 0, 0, 4));
        }

        /// A granted plot's foreclosure refunds its floors' permits, less the
        /// debt: they were Strands, though its deed wasn't (#440, V3.3)
        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_granted_plot_foreclosed_refunds_its_permits_less_the_debt(db: PgPool) {
            crate::content::sync(&db, &crate::content::load().unwrap()).await.unwrap();
            let (ada, bag) = builder(&db, "ada", 400).await;
            let lot = urban_lot(&db, ada, 1000).await;
            sqlx::query("UPDATE plots SET granted = TRUE WHERE id = $1").bind(lot).execute(&db).await.unwrap();
            stock(&db, ada, bag, 1, 0).await;
            build(&db, &Tunables::default(), 1.0, ada, lot, Utc::now()).await.unwrap();
            assert_eq!(balance(&db, ada).await, 0);
            // Four weeks of tax at 1% on 1,400 owed: 56
            let rate = 0.01;
            sqlx::query("UPDATE plots SET tax_owed = 56 WHERE id = $1").bind(lot).execute(&db).await.unwrap();

            let foreclosed = crate::engine::property_tax::foreclose(&db, lot, rate).await.unwrap();

            assert!(foreclosed.is_some());
            assert_eq!(balance(&db, ada).await, 400 - 56);
        }
    }
}
