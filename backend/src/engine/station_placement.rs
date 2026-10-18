//! Putting a station down. WS `place_station` and REST `/stations/place`
//! both come here.
//!
//! The tests run in this order, and the Flint client's ghost
//! (`capitol-core/src/station.rs`) runs the ones it can in the same order:
//! - the station type exists
//! - the spot is within the type's `interaction_range` of where you stand
//! - where it stands ([`site`]):
//!   - on a lot: the lot is yours, with room on it
//!   - on no lot: the type is `off_plot` (only the workbench), and the
//!     ground is dry (#187)
//! - you hold its kit
//! - it's at least `STATION_MIN_SPACING` from every other station
//!
//! A station off a plot has no `plot_id`. Only its owner may use it
//! (`container_access::station_access`), it pays no tax, and no plot's
//! `station_count` includes it. A terrain change that lays a lot under one
//! isn't handled yet (#188).

use capitol_protocol::rules::stacking::Pick;
use capitol_protocol::rules::{in_water, water_name, STATION_MIN_SPACING};
use sqlx::PgPool;
use uuid::Uuid;

use super::container_access::live_position;
use crate::models::{Item, StationInfo, StationType, ITEM_COLUMNS, STATION_TYPE_COLUMNS};
use crate::state::AppState;

/// Why a station can't go down where it was asked for.
#[derive(Debug)]
pub enum PlacementError {
    UnknownType(String),
    TooFar,
    /// On a lot that isn't the player's: someone else's, or nobody's.
    NotYourPlot,
    PlotFull { count: i32, capacity: i32 },
    /// On no lot, and the type isn't one that may stand off a plot.
    NeedsAPlot,
    /// On no lot, in the water of this ring.
    InWater(String),
    KitNotFound,
    WrongKit { expected: String, got: String },
    TooClose,
    Database(sqlx::Error),
}

impl PlacementError {
    pub fn message(&self) -> String {
        match self {
            PlacementError::UnknownType(t) => format!("Unknown station type: {t}"),
            PlacementError::TooFar => "Too far to place station".into(),
            PlacementError::NotYourPlot => "You don't own this plot".into(),
            PlacementError::PlotFull { count, capacity } => format!("Plot is at capacity ({count}/{capacity})"),
            PlacementError::NeedsAPlot => "Must place station on a plot".into(),
            PlacementError::InWater(ring) => format!("Can't build in {}", water_name(ring)),
            PlacementError::KitNotFound => "Kit item not found".into(),
            PlacementError::WrongKit { expected, got } => format!("Wrong item type: expected {expected}, got {got}"),
            PlacementError::TooClose => "Too close to another station".into(),
            PlacementError::Database(e) => format!("Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for PlacementError {
    fn from(e: sqlx::Error) -> Self {
        PlacementError::Database(e)
    }
}

impl From<PlacementError> for crate::error::AppError {
    fn from(e: PlacementError) -> Self {
        use crate::error::AppError;
        match e {
            PlacementError::NotYourPlot => AppError::Forbidden(e.message()),
            PlacementError::KitNotFound => AppError::NotFound(e.message()),
            PlacementError::Database(db) => AppError::Database(db),
            _ => AppError::BadRequest(e.message()),
        }
    }
}

/// The lot under a spot, as far as placing on it goes.
#[derive(Debug, Clone)]
pub struct Lot {
    pub id: Uuid,
    pub owner_id: Option<Uuid>,
    pub station_count: i32,
    pub station_capacity: i32,
}

/// Where a station of a type that may (`off_plot`) or may not stand off a
/// plot would stand: on `lot`, or on open ground, `water` being the ring whose
/// water the spot is in, if any. `Ok` is the plot it stands on, `None` for
/// open ground. Pure, so it can be tested without a database.
pub fn site(player_id: Uuid, off_plot: bool, lot: Option<&Lot>, water: Option<&str>) -> Result<Option<Uuid>, PlacementError> {
    match lot {
        Some(lot) => {
            if lot.owner_id != Some(player_id) {
                return Err(PlacementError::NotYourPlot);
            }
            if lot.station_count >= lot.station_capacity {
                return Err(PlacementError::PlotFull { count: lot.station_count, capacity: lot.station_capacity });
            }
            Ok(Some(lot.id))
        }
        None if !off_plot => Err(PlacementError::NeedsAPlot),
        None => match water {
            Some(ring) => Err(PlacementError::InWater(ring.to_string())),
            None => Ok(None),
        },
    }
}

/// A station just put down.
pub struct Placed {
    pub info: StationInfo,
    /// The plot it stands on; `None` off a plot.
    pub plot_id: Option<Uuid>,
}

/// Put a station of `station_type` down at (`x`, `y`) for `player_id`, from
/// their kit `kit_item_id`. `at` is where they stand now
/// ([`live_position`]); when it's `None`, the position comes from their
/// `players` row, which can be up to 10 s old. The game state isn't touched:
/// adding the station to it is the caller's.
pub async fn place_station(
    state: &AppState,
    player_id: Uuid,
    station_type: &str,
    x: f64,
    y: f64,
    kit_item_id: Uuid,
) -> Result<Placed, PlacementError> {
    let db = &state.db;
    let (px, py) = match live_position(&state.game, player_id).await {
        Some(at) => at,
        None => sqlx::query_as("SELECT position_x, position_y FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(db)
            .await?,
    };

    let st: StationType = sqlx::query_as(&format!("SELECT {STATION_TYPE_COLUMNS} FROM station_types WHERE id = $1"))
        .bind(station_type)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| PlacementError::UnknownType(station_type.to_string()))?;

    let (dx, dy) = (x - px, y - py);
    if (dx * dx + dy * dy).sqrt() > st.interaction_range as f64 {
        return Err(PlacementError::TooFar);
    }

    let lot = lot_at(db, x, y).await?;
    let water = {
        let game = state.game.read().await;
        let terrain = &game.terrain;
        in_water((terrain.center_x, terrain.center_y), &terrain.water_rings, (x, y)).map(|r| r.id.clone())
    };
    let plot_id = site(player_id, st.off_plot, lot.as_ref(), water.as_deref())?;

    let expected_kit = st.kit_item.clone().ok_or_else(|| PlacementError::UnknownType(station_type.to_string()))?;
    let kit_item: Item = sqlx::query_as(&format!("SELECT {ITEM_COLUMNS} FROM items WHERE id = $1 AND owner_id = $2"))
        .bind(kit_item_id)
        .bind(player_id)
        .fetch_optional(db)
        .await?
        .ok_or(PlacementError::KitNotFound)?;
    if kit_item.item_type != expected_kit {
        return Err(PlacementError::WrongKit { expected: expected_kit, got: kit_item.item_type });
    }
    // Kits don't stack, so this is the kit's one unit; an old stack of them
    // gives up its lowest (E1.8)
    let kit = kit_item.units.pick(1, Pick::default()).ok_or(PlacementError::KitNotFound)?;
    let kit_quality = kit.min().unwrap_or(kit_item.quality);

    let nearby: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM stations
         WHERE (position_x - $1)^2 + (position_y - $2)^2 < $3",
    )
    .bind(x)
    .bind(y)
    .bind(STATION_MIN_SPACING * STATION_MIN_SPACING)
    .fetch_optional(db)
    .await?;
    if nearby.is_some() {
        return Err(PlacementError::TooClose);
    }

    let station_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, plot_id, kit_quality)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(station_id)
    .bind(station_type)
    .bind(player_id)
    .bind(x as f32)
    .bind(y as f32)
    .bind(plot_id)
    .bind(kit_quality)
    .execute(db)
    .await?;

    if let Some(plot_id) = plot_id {
        sqlx::query("UPDATE plots SET station_count = station_count + 1 WHERE id = $1")
            .bind(plot_id)
            .execute(db)
            .await?;
    }

    let container_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO containers (id, container_type, station_id)
         VALUES ($1, 'station_inventory', $2)",
    )
    .bind(container_id)
    .bind(station_id)
    .execute(db)
    .await?;

    let mut conn = db.acquire().await?;
    super::units::take_units(&mut conn, kit_item_id, &kit).await?;

    Ok(Placed {
        info: StationInfo {
            id: station_id,
            station_type: station_type.to_string(),
            name: st.name,
            category: st.category,
            icon: st.icon,
            x,
            y,
            owner_id: player_id,
            container_id: Some(container_id),
            interaction_range: st.interaction_range,
            quality: kit_quality,
            tool: None,
        },
        plot_id,
    })
}

/// The lot under (`x`, `y`), with its room for stations.
async fn lot_at(db: &PgPool, x: f64, y: f64) -> Result<Option<Lot>, sqlx::Error> {
    let Some(plot) = crate::routes::plots::find_plot_at_position(db, x, y).await? else {
        return Ok(None);
    };
    let (station_count, station_capacity): (i32, i32) = sqlx::query_as(
        "SELECT p.station_count, psc.station_capacity
         FROM plots p
         JOIN plot_size_categories psc ON p.size_category = psc.id
         WHERE p.id = $1",
    )
    .bind(plot.id)
    .fetch_one(db)
    .await?;
    Ok(Some(Lot { id: plot.id, owner_id: plot.owner_id, station_count, station_capacity }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(owner: Option<Uuid>, count: i32) -> Lot {
        Lot { id: Uuid::from_u128(7), owner_id: owner, station_count: count, station_capacity: 4 }
    }

    #[test]
    fn on_a_lot_it_must_be_yours_with_room() {
        let me = Uuid::from_u128(1);
        let them = Uuid::from_u128(2);
        for off_plot in [false, true] {
            assert_eq!(site(me, off_plot, Some(&lot(Some(me), 3)), None).unwrap(), Some(Uuid::from_u128(7)));
            assert!(matches!(site(me, off_plot, Some(&lot(Some(them), 0)), None), Err(PlacementError::NotYourPlot)));
            assert!(
                matches!(site(me, off_plot, Some(&lot(None, 0)), None), Err(PlacementError::NotYourPlot)),
                "an unclaimed lot is still a lot: claim it first"
            );
            assert!(matches!(
                site(me, off_plot, Some(&lot(Some(me), 4)), None),
                Err(PlacementError::PlotFull { count: 4, capacity: 4 })
            ));
        }
    }

    #[test]
    fn off_every_lot_only_an_off_plot_type_goes_and_only_on_dry_ground() {
        let me = Uuid::from_u128(1);
        assert_eq!(site(me, true, None, None).unwrap(), None, "a workbench in a field");
        assert!(matches!(site(me, false, None, None), Err(PlacementError::NeedsAPlot)), "a forge in a field");
        let wet = site(me, true, None, Some("canal_outer")).unwrap_err();
        assert_eq!(wet.message(), "Can't build in the outer canal");
        assert!(matches!(site(me, false, None, Some("moat")), Err(PlacementError::NeedsAPlot)), "the type first");
    }
}
