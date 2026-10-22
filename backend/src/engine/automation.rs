//! Automation on stations (E4.2, TDD §4.3, GDD §3.7).
//!
//! A crafting station's owner sets it one recipe to make again and again
//! ([`configure`]), a count of crafts or forever, and starts or stops it
//! ([`start`], [`stop`]). The station's own box is the input and the output
//! (v1), so anyone the station rule admits (`container_access`) may load it,
//! but only the owner may set it going. E4.3's tick (`automation_tick`) runs
//! the stations that are `running`, and pauses them with a reason.
//!
//! The setting lives in `stations.automation` (JSON), so it outlasts a
//! restart. [`check_config`] is the one validation, at configuring and again
//! at starting, since the content or the tool slot may have changed between.

use std::collections::HashMap;

use capitol_protocol::rest::{AutomationMode, AutomationState, StationAutomation};
use capitol_protocol::rules::tools::ToolType;
use sqlx::types::Json;
use sqlx::{PgConnection, PgPool};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::container_access::AccessError;
use super::{GameEvent, GameState};
use crate::models::Recipe;

/// The most crafts a `count` run may ask for. Longer than that is `forever`.
pub const MAX_COUNT: u32 = 10_000;

/// Why a station won't take a setting, or won't start or stop.
#[derive(Debug)]
pub enum AutomationError {
    /// No such station, or the database failed
    Access(AccessError),
    /// Only the station's owner sets it going
    NotYourStation,
    /// A storage station makes nothing
    NotACraftingStation,
    /// The station has no box to work from
    NoBox,
    /// No such recipe
    UnknownRecipe(String),
    /// The recipe is made elsewhere: at another station type, or by hand (`None`)
    WrongStation { recipe: String, needs: Option<String> },
    /// The recipe needs a tool the station's slot doesn't hold
    NeedsTool(ToolType),
    /// A count of 0, or over [`MAX_COUNT`]
    BadCount,
    /// Start or stop before any setting
    NotConfigured,
}

impl AutomationError {
    pub fn message(&self) -> String {
        match self {
            AutomationError::Access(e) => e.message(),
            AutomationError::NotYourStation => "Only the station's owner can set it working".into(),
            AutomationError::NotACraftingStation => "A storage station makes nothing".into(),
            AutomationError::NoBox => "This station has no box to work from".into(),
            AutomationError::UnknownRecipe(id) => format!("No recipe {id}"),
            AutomationError::WrongStation { recipe, needs: Some(needs) } => {
                format!("{recipe} is made at a {needs}, not here")
            }
            AutomationError::WrongStation { recipe, needs: None } => {
                format!("{recipe} is made by hand, not at a station")
            }
            AutomationError::NeedsTool(tool) => {
                format!("This recipe needs a {} in the station's tool slot", tool.as_str())
            }
            AutomationError::BadCount => format!("Make between 1 and {MAX_COUNT}, or choose forever"),
            AutomationError::NotConfigured => "Choose what the station makes first".into(),
        }
    }
}

impl From<AccessError> for AutomationError {
    fn from(e: AccessError) -> Self {
        AutomationError::Access(e)
    }
}

impl From<sqlx::Error> for AutomationError {
    fn from(e: sqlx::Error) -> Self {
        AutomationError::Access(AccessError::Database(e))
    }
}

/// What a setting is checked against.
#[derive(Debug, Clone, PartialEq)]
pub struct StationFacts {
    pub station_type: String,
    pub category: String,
    pub has_box: bool,
    /// The type of the tool in its slot, if that tool has uses left
    pub tool: Option<ToolType>,
}

/// May a station with `facts` make `recipe_id` (`recipe`, if it exists) in `mode`?
pub fn check_config(
    facts: &StationFacts,
    recipe_id: &str,
    recipe: Option<&Recipe>,
    mode: AutomationMode,
) -> Result<(), AutomationError> {
    if facts.category != "crafting" {
        return Err(AutomationError::NotACraftingStation);
    }
    if !facts.has_box {
        return Err(AutomationError::NoBox);
    }
    if matches!(mode, AutomationMode::Count(n) if n == 0 || n > MAX_COUNT) {
        return Err(AutomationError::BadCount);
    }
    let recipe = recipe.ok_or_else(|| AutomationError::UnknownRecipe(recipe_id.into()))?;
    if recipe.required_station_type.as_deref() != Some(facts.station_type.as_str()) {
        return Err(AutomationError::WrongStation {
            recipe: recipe.name.clone(),
            needs: recipe.required_station_type.clone(),
        });
    }
    match recipe.tool_type {
        Some(needed) if facts.tool != Some(needed) => Err(AutomationError::NeedsTool(needed)),
        _ => Ok(()),
    }
}

/// Set what `station_id` makes. Setting it stops the station and starts the
/// count again. It counts on the road in (#439).
pub async fn configure(
    db: &PgPool,
    events: &broadcast::Sender<GameEvent>,
    recipes: &HashMap<String, Recipe>,
    player_id: Uuid,
    station_id: Uuid,
    recipe_id: &str,
    mode: AutomationMode,
) -> Result<StationAutomation, AutomationError> {
    let mut tx = db.begin().await?;
    let (facts, _) = owned_station(&mut tx, player_id, station_id).await?;
    check_config(&facts, recipe_id, recipes.get(recipe_id), mode)?;
    let automation = StationAutomation { recipe_id: recipe_id.into(), mode, state: AutomationState::Stopped, done: 0 };
    save(&mut tx, station_id, &automation).await?;
    tx.commit().await?;
    super::tutorial::saw(db, events, player_id, super::tutorial::Did::SetAutomation).await;
    Ok(automation)
}

/// Set `station_id` running. From stopped, the count starts again; a paused
/// station keeps its count. One already running is left as it is.
pub async fn start(
    db: &PgPool,
    recipes: &HashMap<String, Recipe>,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<StationAutomation, AutomationError> {
    let mut tx = db.begin().await?;
    let (facts, automation) = owned_station(&mut tx, player_id, station_id).await?;
    let mut automation = automation.ok_or(AutomationError::NotConfigured)?;
    check_config(&facts, &automation.recipe_id, recipes.get(&automation.recipe_id), automation.mode)?;
    match automation.state {
        AutomationState::Running => {}
        AutomationState::Paused { .. } => automation.state = AutomationState::Running,
        AutomationState::Stopped => {
            automation.state = AutomationState::Running;
            automation.done = 0;
        }
    }
    save(&mut tx, station_id, &automation).await?;
    tx.commit().await?;
    Ok(automation)
}

/// Stop `station_id`. It keeps its setting and its count, for the panel.
pub async fn stop(db: &PgPool, player_id: Uuid, station_id: Uuid) -> Result<StationAutomation, AutomationError> {
    let mut tx = db.begin().await?;
    let (_, automation) = owned_station(&mut tx, player_id, station_id).await?;
    let mut automation = automation.ok_or(AutomationError::NotConfigured)?;
    automation.state = AutomationState::Stopped;
    save(&mut tx, station_id, &automation).await?;
    tx.commit().await?;
    Ok(automation)
}

/// The automation of any of `station_ids` that have one, for `StationInfo`.
pub async fn automation_at(db: &PgPool, station_ids: &[Uuid]) -> Result<HashMap<Uuid, StationAutomation>, sqlx::Error> {
    if station_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, Json<StationAutomation>)> = sqlx::query_as(
        "SELECT id, automation FROM stations WHERE id = ANY($1) AND automation IS NOT NULL",
    )
    .bind(station_ids)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|(id, a)| (id, a.0)).collect())
}

/// Tell everyone what `station_id` is set to now, and the automation tick
/// (`automation_tick::Works`, E4.3), which runs it.
pub fn announce(game: &GameState, station_id: Uuid, automation: StationAutomation) {
    game.works.set(station_id, &automation);
    let _ = game.event_sender.send(GameEvent::StationAutomationChanged { station_id, automation });
}

/// Lock `station_id`, check `player_id` owns it, and read what a setting is
/// checked against, with the setting it has now.
async fn owned_station(
    conn: &mut PgConnection,
    player_id: Uuid,
    station_id: Uuid,
) -> Result<(StationFacts, Option<StationAutomation>), AutomationError> {
    let row: Option<(Uuid, String, String, bool, Option<String>, Option<Json<StationAutomation>>)> = sqlx::query_as(
        "SELECT s.owner_id, s.station_type, st.category,
                EXISTS (SELECT 1 FROM containers c WHERE c.station_id = s.id),
                (SELECT it.tool_type FROM items i JOIN item_types it ON it.id = i.item_type
                 WHERE i.id = s.installed_tool AND i.durability > 0),
                s.automation
         FROM stations s
         JOIN station_types st ON st.id = s.station_type
         WHERE s.id = $1
         FOR UPDATE OF s",
    )
    .bind(station_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (owner_id, station_type, category, has_box, tool, automation) = row.ok_or(AccessError::StationNotFound)?;
    if owner_id != player_id {
        return Err(AutomationError::NotYourStation);
    }
    let facts = StationFacts { station_type, category, has_box, tool: tool.as_deref().and_then(ToolType::from_str) };
    Ok((facts, automation.map(|a| a.0)))
}

async fn save(conn: &mut PgConnection, station_id: Uuid, automation: &StationAutomation) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE stations SET automation = $2 WHERE id = $1")
        .bind(station_id)
        .bind(Json(automation))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capitol_protocol::rest::PauseReason;

    fn shipped() -> HashMap<String, Recipe> {
        crate::content::load().unwrap().recipes.iter().map(|r| (r.id.clone(), r.to_recipe())).collect()
    }

    fn bench() -> StationFacts {
        StationFacts { station_type: "workbench".into(), category: "crafting".into(), has_box: true, tool: None }
    }

    fn anvil(tool: Option<ToolType>) -> StationFacts {
        StationFacts { station_type: "anvil".into(), tool, ..bench() }
    }

    fn check(facts: &StationFacts, recipe_id: &str, mode: AutomationMode) -> Result<(), AutomationError> {
        let recipes = shipped();
        check_config(facts, recipe_id, recipes.get(recipe_id), mode)
    }

    #[test]
    fn a_station_recipe_at_its_own_station_is_fine() {
        assert!(check(&bench(), "tool_handle", AutomationMode::Count(20)).is_ok());
        assert!(check(&bench(), "tool_handle", AutomationMode::Forever).is_ok());
        assert!(check(&bench(), "tool_handle", AutomationMode::Count(MAX_COUNT)).is_ok());
        assert!(check(&anvil(Some(ToolType::Hammer)), "iron_axe_head", AutomationMode::Forever).is_ok());
    }

    #[test]
    fn what_a_setting_is_refused_for() {
        let refusal = |facts: &StationFacts, recipe: &str, mode| check(facts, recipe, mode).unwrap_err();
        let forever = AutomationMode::Forever;

        assert!(matches!(refusal(&bench(), "no_such_thing", forever), AutomationError::UnknownRecipe(id) if id == "no_such_thing"));
        assert!(matches!(refusal(&bench(), "plank", forever),
            AutomationError::WrongStation { needs: Some(ref s), .. } if s == "sawbench"));
        assert!(matches!(refusal(&bench(), "rope_basic", forever), AutomationError::WrongStation { needs: None, .. }),
            "a hand recipe isn't automated");
        assert!(matches!(refusal(&anvil(None), "iron_axe_head", forever), AutomationError::NeedsTool(ToolType::Hammer)));
        assert!(matches!(refusal(&anvil(Some(ToolType::Axe)), "iron_axe_head", forever),
            AutomationError::NeedsTool(ToolType::Hammer)), "the wrong tool is no tool");
        assert!(matches!(refusal(&bench(), "tool_handle", AutomationMode::Count(0)), AutomationError::BadCount));
        assert!(matches!(refusal(&bench(), "tool_handle", AutomationMode::Count(MAX_COUNT + 1)), AutomationError::BadCount));
        let chest = StationFacts { station_type: "storage_chest".into(), category: "storage".into(), ..bench() };
        assert!(matches!(refusal(&chest, "tool_handle", forever), AutomationError::NotACraftingStation));
        let boxless = StationFacts { has_box: false, ..bench() };
        assert!(matches!(refusal(&boxless, "tool_handle", forever), AutomationError::NoBox));
        // Every refusal says why
        assert_eq!(refusal(&bench(), "plank", forever).message(), "Planks is made at a sawbench, not here");
        assert_eq!(refusal(&anvil(None), "iron_axe_head", forever).message(),
            "This recipe needs a hammer in the station's tool slot");
    }

    #[test]
    fn a_finished_count_is_finished() {
        let mut a = StationAutomation {
            recipe_id: "tool_handle".into(),
            mode: AutomationMode::Count(3),
            state: AutomationState::Paused { reason: PauseReason::OutputFull },
            done: 2,
        };
        assert!(!a.is_finished());
        a.done = 3;
        assert!(a.is_finished());
        a.mode = AutomationMode::Forever;
        assert!(!a.is_finished());
    }

    /// These need the local Postgres (`docker-compose up -d`). Run them with
    /// `cargo test automation -- --ignored`.
    mod db {
        use super::*;

        const HERE: Option<(f64, f64)> = Some((0.0, 0.0));

        async fn player(db: &PgPool, name: &str) -> (Uuid, Uuid) {
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

        /// A `kind` station of `owner`'s at the origin, with its box
        async fn station(db: &PgPool, owner: Uuid, kind: &str) -> Uuid {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
                 VALUES ($1, $2, $3, 0, 0, 50)",
            )
            .bind(id)
            .bind(kind)
            .bind(owner)
            .execute(db)
            .await
            .unwrap();
            sqlx::query("INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2)")
                .bind(Uuid::new_v4())
                .bind(id)
                .execute(db)
                .await
                .unwrap();
            id
        }

        async fn setup(db: &PgPool) -> HashMap<String, Recipe> {
            crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
            shipped()
        }

        /// What a restarted server would read back
        async fn stored(db: &PgPool, station_id: Uuid) -> Option<StationAutomation> {
            automation_at(db, &[station_id]).await.unwrap().remove(&station_id)
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn configure_start_and_stop_round_trip(db: PgPool) {
            let recipes = setup(&db).await;
            let (events, _) = broadcast::channel(16);
            let (ada, _) = player(&db, "ada").await;
            let bench = station(&db, ada, "workbench").await;
            assert_eq!(stored(&db, bench).await, None);
            assert!(matches!(start(&db, &recipes, ada, bench).await, Err(AutomationError::NotConfigured)));
            assert!(matches!(stop(&db, ada, bench).await, Err(AutomationError::NotConfigured)));

            let set = configure(&db, &events, &recipes, ada, bench, "tool_handle", AutomationMode::Count(20)).await.unwrap();
            assert_eq!(set, StationAutomation {
                recipe_id: "tool_handle".into(),
                mode: AutomationMode::Count(20),
                state: AutomationState::Stopped,
                done: 0,
            });
            assert_eq!(stored(&db, bench).await.as_ref(), Some(&set));

            let running = start(&db, &recipes, ada, bench).await.unwrap();
            assert_eq!(running.state, AutomationState::Running);
            assert_eq!(stored(&db, bench).await, Some(running.clone()));
            assert_eq!(start(&db, &recipes, ada, bench).await.unwrap(), running, "starting it again changes nothing");

            // E4.3 will count and pause; a paused station resumes with its count
            let paused = StationAutomation {
                state: AutomationState::Paused { reason: PauseReason::MissingInput { item_type: "sticks".into() } },
                done: 7,
                ..running
            };
            save(&mut db.acquire().await.unwrap(), bench, &paused).await.unwrap();
            assert_eq!(stored(&db, bench).await.as_ref(), Some(&paused), "a pause and its reason persist");
            let resumed = start(&db, &recipes, ada, bench).await.unwrap();
            assert_eq!((&resumed.state, resumed.done), (&AutomationState::Running, 7));

            let stopped = stop(&db, ada, bench).await.unwrap();
            assert_eq!((&stopped.state, stopped.done), (&AutomationState::Stopped, 7), "a stop keeps the count");
            assert_eq!(stored(&db, bench).await, Some(stopped));
            let restarted = start(&db, &recipes, ada, bench).await.unwrap();
            assert_eq!(restarted.done, 0, "a start from stopped counts again");

            // A new setting stops it
            let reset = configure(&db, &events, &recipes, ada, bench, "knapped_stone", AutomationMode::Forever).await.unwrap();
            assert_eq!((reset.recipe_id.as_str(), &reset.state), ("knapped_stone", &AutomationState::Stopped));
            assert_eq!(stored(&db, bench).await, Some(reset));
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn only_the_owner_sets_it_going(db: PgPool) {
            let recipes = setup(&db).await;
            let (events, _) = broadcast::channel(16);
            let (ada, _) = player(&db, "ada").await;
            let (bob, _) = player(&db, "bob").await;
            let bench = station(&db, ada, "workbench").await;
            configure(&db, &events, &recipes, ada, bench, "tool_handle", AutomationMode::Forever).await.unwrap();

            assert!(matches!(configure(&db, &events, &recipes, bob, bench, "tool_handle", AutomationMode::Forever).await,
                Err(AutomationError::NotYourStation)));
            assert!(matches!(start(&db, &recipes, bob, bench).await, Err(AutomationError::NotYourStation)));
            assert!(matches!(stop(&db, bob, bench).await, Err(AutomationError::NotYourStation)));
            assert!(matches!(start(&db, &recipes, ada, Uuid::new_v4()).await,
                Err(AutomationError::Access(AccessError::StationNotFound))));
            assert_eq!(stored(&db, bench).await.map(|a| a.state), Some(AutomationState::Stopped));
            // Setting the job counts on the road in, and being refused doesn't (#439)
            use capitol_protocol::tutorial::{Step, Steps};
            let done = |player| crate::engine::tutorial::progress(&db, player);
            assert_eq!(done(ada).await.unwrap().done, Steps::from_iter([Step::SetAutomation]));
            assert_eq!(done(bob).await.unwrap().done, Steps::NONE);
        }

        #[sqlx::test(migrations = "./migrations")]
        #[ignore = "needs the local Postgres (docker-compose up -d)"]
        async fn a_tool_recipe_needs_the_tool_installed_to_set_and_to_start(db: PgPool) {
            let recipes = setup(&db).await;
            let (events, _) = broadcast::channel(16);
            let (ada, inventory) = player(&db, "ada").await;
            let anvil = station(&db, ada, "anvil").await;
            let chest = station(&db, ada, "storage_chest").await;

            assert!(matches!(configure(&db, &events, &recipes, ada, anvil, "iron_axe_head", AutomationMode::Forever).await,
                Err(AutomationError::NeedsTool(ToolType::Hammer))));
            assert!(matches!(configure(&db, &events, &recipes, ada, chest, "tool_handle", AutomationMode::Forever).await,
                Err(AutomationError::NotACraftingStation)));
            assert_eq!(stored(&db, anvil).await, None, "a refusal stores nothing");

            let hammer: Uuid = sqlx::query_scalar(
                "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index, durability)
                 VALUES ($1, 'stone_hammer', 50, 1, $2, $3, 0, 5) RETURNING id",
            )
            .bind(Uuid::new_v4())
            .bind(ada)
            .bind(inventory)
            .fetch_one(&db)
            .await
            .unwrap();
            crate::engine::station_tools::install_tool(&db, ada, anvil, hammer, HERE).await.unwrap();
            configure(&db, &events, &recipes, ada, anvil, "iron_axe_head", AutomationMode::Count(5)).await.unwrap();

            // Taken out again before the start: the start says why it won't
            crate::engine::station_tools::remove_tool(&db, ada, anvil, HERE).await.unwrap();
            assert!(matches!(start(&db, &recipes, ada, anvil).await, Err(AutomationError::NeedsTool(ToolType::Hammer))));
            assert_eq!(stored(&db, anvil).await.map(|a| a.state), Some(AutomationState::Stopped));
        }
    }
}
