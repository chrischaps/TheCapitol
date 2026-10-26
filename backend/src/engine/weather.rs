//! Weather (Plots v2 §5.2, V3.2, #474): stations wear outdoors, slowly on a
//! claimed plot, and fall down on public land.
//!
//! A station's condition (`stations.condition`, 0 to 100) is as it stood at
//! `condition_at`. Nothing ticks it. Once a game hour [`sweep`] writes every
//! outdoor station's on to now in one statement, at the live tunables'
//! rates: `weather_wear_claimed` on a plot, `weather_wear_public` off one
//! (D10). Indoors it doesn't wear. The numbers are `rules::weather`.
//!
//! - **Below 25** a station stops working on its own: the automation tick
//!   pauses it `needs_repair` (`automation_tick::craft_once`), and it starts
//!   again once it's repaired.
//! - **At 0 on a claimed plot** it's out of use, by hand too ([`out_of_use`]),
//!   and stays until it's repaired or the plot is released.
//! - **At 0 on public land** it falls down ([`collapse`]): its kit is lost,
//!   and its tool and box go to the owner's impound (#141). That clears away
//!   abandoned stations.
//! - **Outdoors an installed tool wears faster**, `outdoor_tool_wear` times a
//!   use, read beside the slot ([`TOOL_WEAR_COLUMN`]).
//! - **A repair** ([`repair`], `POST /stations/{id}/repair`) restores 100 for
//!   a share of what the station's kit took, more the further it has fallen,
//!   from the repairer's pack, lowest grade first. It's a sink
//!   (`UnitFlow::Repaired`).
//!
//! The sweep tells everyone when a station crosses 25 or 0
//! (`station_condition`), and the owner of one that fell
//! (`station_collapsed`, a notice, so they hear when they're back).

use std::collections::HashMap;
use std::sync::LazyLock;

use capitol_protocol::ledger::UnitFlow;
use capitol_protocol::rules::stacking::Units;
use capitol_protocol::rules::weather;
use capitol_protocol::tunables::Tunables;
use capitol_protocol::ws::{ItemCount, ServerMessage};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::events::GameEvent;
use super::{ledger, notices, station_removal, units};
use crate::error::AppError;
use crate::models::{Recipe, StationState};

/// How many uses' wear a craft takes off a station's installed tool, as a SQL
/// column over `stations s`: 1 indoors, and outdoors `outdoor_tool_wear` as
/// the tunables table holds it, or its default. The table is the tunables'
/// truth (`tunables::update` writes it before it swaps the new set in), so
/// the craft that locks the slot reads it in the same round trip.
pub static TOOL_WEAR_COLUMN: LazyLock<String> = LazyLock::new(|| {
    format!(
        "CASE WHEN s.indoors THEN 1.0::float8
              ELSE COALESCE((SELECT value FROM tunables WHERE key = 'outdoor_tool_wear'), {:?}::float8) END",
        weather::OUTDOOR_TOOL_WEAR
    )
});

/// What a sweep found
#[derive(Debug, Default)]
pub struct SweepReport {
    /// Stations that crossed 25 or 0, and their condition now
    pub crossed: Vec<(Uuid, i32)>,
    /// Stations on public land that fell down
    pub fallen: Vec<Fallen>,
}

/// A station the weather brought down
#[derive(Debug, Clone, PartialEq)]
pub struct Fallen {
    pub station_id: Uuid,
    pub owner_id: Uuid,
    pub station_type: String,
    /// What of its owner's went to their impound
    pub impounded: Vec<ItemCount>,
    /// `(player, container)` for each store that took something
    pub returned_to: Vec<(Uuid, Uuid)>,
    /// Its queued `station_collapsed` notice
    pub notice_id: Option<Uuid>,
}

/// Wear every outdoor station on to `now`, at `tunables`' rates and `scale`
/// times game time (`SIM_TICK_SCALE`), then bring down the public ones at 0.
pub async fn sweep(db: &PgPool, tunables: &Tunables, scale: f64, now: DateTime<Utc>) -> Result<SweepReport, sqlx::Error> {
    let worn: Vec<(Uuid, f64, f64)> = sqlx::query_as(
        "WITH worn AS (
             SELECT id, condition AS before,
                    GREATEST(condition
                        - CASE WHEN plot_id IS NULL THEN $1 ELSE $2 END
                          * GREATEST(EXTRACT(EPOCH FROM ($3 - condition_at))::float8, 0) * $4 / $5,
                        0) AS after
             FROM stations
             WHERE NOT indoors AND condition > 0
             FOR UPDATE
         )
         UPDATE stations s SET condition = w.after, condition_at = $3
         FROM worn w WHERE s.id = w.id
         RETURNING s.id, w.before, w.after",
    )
    .bind(tunables.weather_wear_public)
    .bind(tunables.weather_wear_claimed)
    .bind(now)
    .bind(scale)
    .bind(capitol_protocol::rules::DAY_LENGTH_SECS as f64)
    .fetch_all(db)
    .await?;
    let mut report = SweepReport {
        crossed: worn
            .into_iter()
            .filter(|(_, before, after)| crossed(*before, *after))
            .map(|(id, _, after)| (id, weather::shown(after)))
            .collect(),
        fallen: Vec::new(),
    };

    // Every public station at 0, not only those that just got there, so one
    // a failed sweep left standing goes the next hour
    let falling: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM stations WHERE plot_id IS NULL AND NOT indoors AND condition <= 0 ORDER BY id",
    )
    .fetch_all(db)
    .await?;
    for station_id in falling {
        match collapse(db, station_id).await {
            Ok(Some(fallen)) => report.fallen.push(fallen),
            Ok(None) => {}
            Err(e) => tracing::error!("Weather: station {station_id} couldn't fall down: {e}"),
        }
    }
    Ok(report)
}

/// Did a station's condition cross 25 or 0, going from `before` to `after`?
fn crossed(before: f64, after: f64) -> bool {
    let (before, after) = (weather::shown(before), weather::shown(after));
    [weather::NEEDS_REPAIR - 1, 0].iter().any(|line| before > *line && after <= *line)
}

/// Bring down `station_id`, a station on public land worn to 0, in one
/// transaction: its tool and what was in its box to impounds
/// (`station_removal::collapse_stations`), its kit lost, and a notice to its
/// owner. `None` if it's gone, or has been repaired or claimed since.
pub async fn collapse(db: &PgPool, station_id: Uuid) -> Result<Option<Fallen>, sqlx::Error> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT owner_id, station_type FROM stations
         WHERE id = $1 AND plot_id IS NULL AND NOT indoors AND condition <= 0 FOR UPDATE",
    )
    .bind(station_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((owner_id, station_type)) = row else {
        return Ok(None);
    };
    let returned = station_removal::collapse_stations(&mut tx, &[station_id]).await?;
    let mut flows = ledger::Flows::new();
    returned.count_into(&mut flows);
    if !returned.lost.is_empty() {
        tracing::warn!("Weather: station {station_id} fell with no room for {:?}, lost with it", returned.lost);
    }
    sqlx::query("DELETE FROM stations WHERE id = $1").bind(station_id).execute(&mut *tx).await?;
    let impounded = returned.impounded_for(owner_id);
    let message = ServerMessage::StationCollapsed {
        station_id,
        station_type: station_type.clone(),
        impounded: impounded.clone(),
    };
    let notice_id = notices::queue(&mut *tx, owner_id, &station_id.to_string(), &message).await?;
    ledger::commit(tx, flows).await?;
    tracing::info!("Weather: {owner_id}'s {station_type} {station_id} fell down on public land");
    Ok(Some(Fallen {
        station_id,
        owner_id,
        station_type,
        impounded,
        returned_to: returned.containers,
        notice_id: Some(notice_id),
    }))
}

/// Tell the game and the clients what a sweep did: the crossings to
/// everyone, and each fallen station gone, to everyone, with its owner's
/// notice and their impound changed.
pub fn announce(report: SweepReport, stations: &mut HashMap<Uuid, StationState>, events: &broadcast::Sender<GameEvent>) {
    for (station_id, condition) in report.crossed {
        let _ = events.send(GameEvent::StationCondition { station_id, condition });
    }
    for f in report.fallen {
        stations.remove(&f.station_id);
        let _ = events.send(GameEvent::StationRemoved { station_id: f.station_id });
        for (player_id, container_id) in f.returned_to {
            let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
        }
        if let Some(notice_id) = f.notice_id {
            let message = ServerMessage::StationCollapsed {
                station_id: f.station_id,
                station_type: f.station_type,
                impounded: f.impounded,
            };
            let _ = events.send(GameEvent::Notice { player_id: f.owner_id, notice_id, message });
        }
    }
}

/// Why `station_id` can't be worked by hand: it's worn out (V3.2). `None`
/// when it can, or it's gone (the craft's own checks say so).
pub async fn out_of_use(db: &PgPool, station_id: Uuid) -> Result<Option<String>, sqlx::Error> {
    let condition: Option<f64> = sqlx::query_scalar("SELECT condition FROM stations WHERE id = $1")
        .bind(station_id)
        .fetch_optional(db)
        .await?;
    Ok(condition
        .filter(|c| weather::worn_out(*c))
        .map(|_| "The weather has worn this station out. Repair it first".to_string()))
}

/// What a repair did
#[derive(Debug, Clone, PartialEq)]
pub struct Repaired {
    /// The repairer's inventory, which the goods came out of
    pub inventory_id: Uuid,
    /// What it used up, by item type
    pub used: Vec<ItemCount>,
}

/// What a repair of a `station_type` at `condition` takes: a share of each
/// input of the recipe that makes its kit (`rules::weather::repair_bill`).
/// With several such recipes, the first by id. Nothing for a kit no recipe
/// makes.
pub fn repair_bill<'a>(kit: &str, condition: f64, recipes: impl IntoIterator<Item = &'a Recipe>) -> Vec<(String, i32)> {
    let recipe = recipes
        .into_iter()
        .filter(|r| r.outputs.iter().any(|o| o.item_type == kit))
        .min_by(|a, b| a.id.cmp(&b.id));
    let Some(recipe) = recipe else {
        return Vec::new();
    };
    recipe
        .inputs
        .iter()
        .map(|i| (i.item_type.clone(), weather::repair_bill(i.quantity, condition)))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// Repair `station_id` to full for `player_id`, who may use it
/// (`container_access::check_station_access`, checked by the caller), taking
/// the bill from their pack, lowest grade first. All of it or nothing.
pub async fn repair(
    db: &PgPool,
    recipes: &HashMap<String, Recipe>,
    player_id: Uuid,
    station_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Repaired, AppError> {
    let mut tx = db.begin().await?;
    let row: Option<(f64, Option<String>)> = sqlx::query_as(
        "SELECT s.condition, st.kit_item FROM stations s JOIN station_types st ON st.id = s.station_type
         WHERE s.id = $1 FOR UPDATE OF s",
    )
    .bind(station_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((condition, kit)) = row else {
        return Err(AppError::NotFound("Station not found".into()));
    };
    if weather::shown(condition) >= weather::FULL as i32 {
        return Err(AppError::BadRequest("It's in good repair".into()));
    }
    let bill = kit.map(|kit| repair_bill(&kit, condition, recipes.values())).unwrap_or_default();

    let inventory_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'player_inventory' LIMIT 1 FOR UPDATE",
    )
    .bind(player_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("No inventory".into()))?;
    let mut flows = ledger::Flows::new();
    let mut used = Vec::new();
    for (item_type, n) in &bill {
        take_lowest(&mut tx, inventory_id, item_type, *n, "A repair").await?;
        flows.add(item_type, UnitFlow::Repaired, *n as i64);
        used.push(ItemCount { item_type: item_type.clone(), quantity: *n });
    }
    sqlx::query("UPDATE stations SET condition = $2, condition_at = $3 WHERE id = $1")
        .bind(station_id)
        .bind(weather::FULL)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    ledger::commit(tx, flows).await?;
    Ok(Repaired { inventory_id, used })
}

/// Take `n` `item_type` units from the stacks in `container`, the lowest
/// grade first across them all, or refuse with what's short, saying what
/// needed them (`"A repair"`)
pub(crate) async fn take_lowest(
    conn: &mut PgConnection,
    container: Uuid,
    item_type: &str,
    n: i32,
    what: &str,
) -> Result<(), AppError> {
    let stacks: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM items WHERE container_id = $1 AND item_type = $2 ORDER BY slot_index, id FOR UPDATE")
            .bind(container)
            .bind(item_type)
            .fetch_all(&mut *conn)
            .await?;
    let held = units::units_of_many(conn, &stacks).await?;
    let plan = plan_take(&stacks, &held, n).ok_or_else(|| {
        let have: i64 = held.values().map(Units::count).sum();
        AppError::BadRequest(format!("{what} needs {n} {item_type}, and you have {have}"))
    })?;
    let takes: Vec<(Uuid, &Units, &Units)> = plan.iter().map(|(id, taken, left)| (*id, taken, left)).collect();
    units::write_takes(conn, &takes).await?;
    Ok(())
}

/// Which units to take off which of `stacks` (in slot order) for `n`, the
/// lowest quality first across them all, a lower slot first at one quality:
/// `(stack, taken, left)` for each stack that gives some. `None` if they
/// hold fewer.
fn plan_take(stacks: &[Uuid], held: &HashMap<Uuid, Units>, n: i32) -> Option<Vec<(Uuid, Units, Units)>> {
    let mut lots: Vec<(i32, usize, i32)> = stacks
        .iter()
        .enumerate()
        .flat_map(|(k, id)| held.get(id).into_iter().flat_map(move |u| u.pairs().map(move |(q, c)| (q, k, c))))
        .collect();
    lots.sort();
    let mut taken: Vec<Units> = vec![Units::new(); stacks.len()];
    let mut wanted = n;
    for (quality, k, count) in lots {
        if wanted == 0 {
            break;
        }
        let take = count.min(wanted);
        taken[k].add(quality, take);
        wanted -= take;
    }
    if wanted > 0 {
        return None;
    }
    Some(
        stacks
            .iter()
            .zip(taken)
            .filter(|(_, t)| !t.is_empty())
            .map(|(id, t)| {
                let mut left = held[id].clone();
                left.take_units(&t).expect("planned from these units");
                (*id, t, left)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sweep_names_the_crossings_at_25_and_0() {
        assert!(crossed(25.0, 24.0));
        assert!(!crossed(25.0, 24.5), "still shows 25");
        assert!(crossed(30.0, 0.0));
        assert!(crossed(10.0, 0.0));
        assert!(!crossed(10.0, 5.0));
        assert!(!crossed(80.0, 79.0));
    }

    #[test]
    fn a_take_plans_the_lowest_grade_first_across_stacks() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let held = HashMap::from([
            (a, Units::from_pairs([(80, 2), (40, 1)])),
            (b, Units::from_pairs([(30, 1), (60, 5)])),
        ]);
        let plan = plan_take(&[a, b], &held, 4).unwrap();
        assert_eq!(
            plan,
            vec![
                (a, Units::from_pairs([(40, 1)]), Units::from_pairs([(80, 2)])),
                (b, Units::from_pairs([(30, 1), (60, 2)]), Units::from_pairs([(60, 3)])),
            ]
        );
        assert_eq!(plan_take(&[a, b], &held, 10), None);
        let all = plan_take(&[a, b], &held, 9).unwrap();
        assert!(all.iter().all(|(_, _, left)| left.is_empty()));
    }

    fn content_recipes() -> HashMap<String, Recipe> {
        crate::content::load().unwrap().recipes.iter().map(|r| (r.id.clone(), r.to_recipe())).collect()
    }

    #[test]
    fn a_repair_bills_a_share_of_the_kits_recipe() {
        let recipes = content_recipes();
        let mut anvil = repair_bill("anvil_kit", 0.0, recipes.values());
        anvil.sort();
        assert_eq!(anvil, vec![("iron_ingot".to_string(), 2), ("plank".to_string(), 1)]);
        assert_eq!(repair_bill("workbench_kit", 50.0, recipes.values()), vec![("fiber".to_string(), 2)]);
        assert!(repair_bill("no_such_kit", 0.0, recipes.values()).is_empty());
    }

    /// These need the local Postgres (`docker-compose up -d`). Run them with
    /// `cargo test weather -- --ignored`.
    mod db {
        use super::*;
        use crate::engine::units::test_units::{all_held, all_in, stack_of};
        use chrono::Duration;

        async fn player(db: &PgPool, name: &str) -> (Uuid, Uuid) {
            let account = Uuid::new_v4();
            sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
                .bind(account)
                .bind(format!("{name}@test"))
                .execute(db)
                .await
                .unwrap();
            let id: Uuid = sqlx::query_scalar("INSERT INTO players (id, account_id, name) VALUES ($1, $2, $3) RETURNING id")
                .bind(Uuid::new_v4())
                .bind(account)
                .bind(name)
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

        /// A workbench of `owner`'s, on a plot of theirs or off any, indoors
        /// or out, last weathered at `at`. Returns it and its box.
        async fn bench(db: &PgPool, owner: Uuid, on_plot: bool, indoors: bool, at: DateTime<Utc>) -> (Uuid, Uuid) {
            let plot: Option<Uuid> = if on_plot {
                Some(
                    sqlx::query_scalar(
                        "INSERT INTO plots (zone_id, grid_x, grid_y, world_x, world_y, bounds_min_x,
                                            bounds_min_y, bounds_max_x, bounds_max_y, size_category, owner_id)
                         VALUES ((SELECT id FROM zones LIMIT 1), 9999, 9999, 0, 0, -10, -10, 10, 10, 'small', $1)
                         RETURNING id",
                    )
                    .bind(owner)
                    .fetch_one(db)
                    .await
                    .unwrap(),
                )
            } else {
                None
            };
            let station: Uuid = sqlx::query_scalar(
                "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality, indoors, condition_at)
                 VALUES ($1, 'workbench', $2, 0, 0, $3, 60, $4, $5) RETURNING id",
            )
            .bind(Uuid::new_v4())
            .bind(owner)
            .bind(plot)
            .bind(indoors)
            .bind(at)
            .fetch_one(db)
            .await
            .unwrap();
            let the_box: Uuid = sqlx::query_scalar(
                "INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2) RETURNING id",
            )
            .bind(Uuid::new_v4())
            .bind(station)
            .fetch_one(db)
            .await
            .unwrap();
            (station, the_box)
        }

        async fn condition(db: &PgPool, station: Uuid) -> Option<f64> {
            sqlx::query_scalar("SELECT condition FROM stations WHERE id = $1").bind(station).fetch_optional(db).await.unwrap()
        }

        /// `n` game days of wall clock at 1x
        fn days(n: i64) -> Duration {
            Duration::seconds(n * capitol_protocol::rules::DAY_LENGTH_SECS)
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn indoors_never_wears_and_a_claimed_station_wears_to_0_and_stays(db: PgPool) {
            let (ada, _) = player(&db, "ada").await;
            let now = Utc::now();
            let (inside, _) = bench(&db, ada, true, true, now - days(500)).await;
            let (yard, _) = bench(&db, ada, true, false, now - days(75)).await;
            let t = Tunables::default();

            let report = sweep(&db, &t, 1.0, now).await.unwrap();
            assert_eq!(condition(&db, inside).await, Some(100.0));
            assert!((condition(&db, yard).await.unwrap() - 25.0).abs() < 1e-6, "75 days at 1 a day");
            assert!(report.crossed.is_empty(), "still shows 25");
            assert!(out_of_use(&db, yard).await.unwrap().is_none());

            let report = sweep(&db, &t, 1.0, now + days(1)).await.unwrap();
            assert_eq!(report.crossed, vec![(yard, 24)]);

            let report = sweep(&db, &t, 1.0, now + days(400)).await.unwrap();
            assert_eq!(report.crossed, vec![(yard, 0)]);
            assert!(report.fallen.is_empty(), "a private station never falls");
            assert_eq!(condition(&db, yard).await, Some(0.0));
            assert!(out_of_use(&db, yard).await.unwrap().is_some(), "out of use by hand");
            assert_eq!(condition(&db, inside).await, Some(100.0));
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_public_station_at_0_falls_and_its_box_goes_to_the_impound(db: PgPool) {
            let (ada, bag) = player(&db, "ada").await;
            let now = Utc::now();
            let (station, the_box) = bench(&db, ada, false, false, now - days(26)).await;
            stack_of(&db, ada, the_box, 0, "fiber", &Units::uniform(7, 55)).await;
            let fiber_before = all_held(&db, ada, "fiber").await;

            let report = sweep(&db, &Tunables::default(), 1.0, now).await.unwrap();

            assert_eq!(condition(&db, station).await, None, "it fell down");
            assert_eq!(report.fallen.len(), 1);
            let fallen = &report.fallen[0];
            assert_eq!((fallen.station_id, fallen.owner_id), (station, ada));
            assert_eq!(fallen.impounded, vec![ItemCount { item_type: "fiber".into(), quantity: 7 }]);
            let pound: Uuid = sqlx::query_scalar("SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'impound'")
                .bind(ada)
                .fetch_one(&db)
                .await
                .unwrap();
            assert_eq!(all_in(&db, pound, "fiber").await, Units::uniform(7, 55), "every unit keeps its quality");
            assert_eq!(all_held(&db, ada, "fiber").await, fiber_before);
            assert!(all_in(&db, bag, "workbench_kit").await.is_empty(), "the kit is lost");
            let notices: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM player_notices WHERE player_id = $1 AND kind = 'station_collapsed'")
                    .bind(ada)
                    .fetch_one(&db)
                    .await
                    .unwrap();
            assert_eq!(notices, 1);
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_repair_takes_the_lowest_grade_and_restores_full(db: PgPool) {
            let (ada, bag) = player(&db, "ada").await;
            let now = Utc::now();
            let (station, _) = bench(&db, ada, true, false, now - days(100)).await;
            sweep(&db, &Tunables::default(), 1.0, now).await.unwrap();
            assert_eq!(condition(&db, station).await, Some(0.0));
            stack_of(&db, ada, bag, 0, "fiber", &Units::from_pairs([(90, 2), (30, 1)])).await;
            stack_of(&db, ada, bag, 1, "fiber", &Units::uniform(1, 50)).await;
            let recipes = content_recipes();

            let repaired = repair(&db, &recipes, ada, station, now).await.unwrap();

            assert_eq!(repaired.used, vec![ItemCount { item_type: "fiber".into(), quantity: 3 }]);
            assert_eq!(all_held(&db, ada, "fiber").await, Units::uniform(1, 90), "the lowest three went");
            assert_eq!(condition(&db, station).await, Some(100.0));
            assert!(matches!(repair(&db, &recipes, ada, station, now).await, Err(AppError::BadRequest(_))), "nothing to repair");

            // Short of the goods, it changes nothing
            sqlx::query("UPDATE stations SET condition = 0 WHERE id = $1").bind(station).execute(&db).await.unwrap();
            assert!(
                matches!(repair(&db, &recipes, ada, station, now).await, Err(AppError::BadRequest(m)) if m.contains("needs 3 fiber"))
            );
            assert_eq!(all_held(&db, ada, "fiber").await, Units::uniform(1, 90));
            assert_eq!(condition(&db, station).await, Some(0.0));
        }
    }
}
