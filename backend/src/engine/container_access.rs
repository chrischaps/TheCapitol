//! Who may put items into, or take them out of, a container.
//!
//! A player's own containers (inventory) carry `owner_id`. Station containers
//! don't: they carry `station_id`, and access follows the station. The owner
//! of the station may use it, and so may anyone the plot's `station_use_level`
//! admits, provided they stand within the station type's `interaction_range`.
//! REST `/inventory/move` and WS `move_item` both go through [`check_container_access`].

use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{Container, PermissionLevel};

/// Why a player can't use a container.
#[derive(Debug)]
pub enum AccessError {
    /// No such container, or it belongs to another player's inventory.
    NotFound,
    /// A station container the player has no right to use.
    Forbidden,
    /// A station container the player may use, but from further than its range.
    TooFar,
    Database(sqlx::Error),
}

impl AccessError {
    pub fn message(&self) -> String {
        match self {
            AccessError::NotFound => "Container not found".into(),
            AccessError::Forbidden => "Not your station".into(),
            AccessError::TooFar => "Too far from station".into(),
            AccessError::Database(e) => format!("Database error: {}", e),
        }
    }
}

impl From<sqlx::Error> for AccessError {
    fn from(e: sqlx::Error) -> Self {
        AccessError::Database(e)
    }
}

impl From<AccessError> for crate::error::AppError {
    fn from(e: AccessError) -> Self {
        use crate::error::AppError;
        match e {
            AccessError::NotFound => AppError::NotFound(e.message()),
            AccessError::Forbidden => AppError::Forbidden(e.message()),
            AccessError::TooFar => AppError::BadRequest(e.message()),
            AccessError::Database(db) => AppError::Database(db),
        }
    }
}

/// What a station container's access depends on.
#[derive(Debug, Clone)]
pub struct StationAccess {
    pub station_owner: Uuid,
    /// Owner of the plot the station stands on, if it stands on one.
    pub plot_owner: Option<Uuid>,
    /// The plot's `station_use_level`, if it stands on one.
    pub station_use_level: Option<String>,
    /// Player-to-station distance, in world units.
    pub distance: f64,
    pub interaction_range: f64,
}

/// May `player_id` use a station container? Pure, so it can be tested without a database.
pub fn station_access(player_id: Uuid, s: &StationAccess) -> Result<(), AccessError> {
    let permitted = s.station_owner == player_id
        || s.plot_owner == Some(player_id)
        || matches!(
            s.station_use_level.as_deref().and_then(PermissionLevel::from_str),
            Some(PermissionLevel::Public)
        );
    // "friends" and "guild" admit no one beyond the owners until those systems
    // exist, and items left in a shared chest stay their depositor's (#80).
    if !permitted {
        return Err(AccessError::Forbidden);
    }
    if s.distance > s.interaction_range {
        return Err(AccessError::TooFar);
    }
    Ok(())
}

/// Load a container and check that `player_id` may move items into or out of it.
pub async fn check_container_access(
    db: &PgPool,
    player_id: Uuid,
    container_id: Uuid,
) -> Result<Container, AccessError> {
    let container: Container = sqlx::query_as(
        "SELECT id, container_type, owner_id, station_id, created_at
         FROM containers WHERE id = $1",
    )
    .bind(container_id)
    .fetch_optional(db)
    .await?
    .ok_or(AccessError::NotFound)?;

    if let Some(owner) = container.owner_id {
        return if owner == player_id {
            Ok(container)
        } else {
            Err(AccessError::NotFound)
        };
    }
    let Some(station_id) = container.station_id else {
        return Err(AccessError::NotFound);
    };

    let row: Option<(Uuid, f32, f32, f32, Option<Uuid>, Option<String>, f64, f64)> =
        sqlx::query_as(
            "SELECT s.owner_id, s.position_x, s.position_y, st.interaction_range,
                    p.owner_id, pp.station_use_level, pl.position_x, pl.position_y
             FROM stations s
             JOIN station_types st ON st.id = s.station_type
             JOIN players pl ON pl.id = $2
             LEFT JOIN plots p ON p.id = s.plot_id
             LEFT JOIN plot_permissions pp ON pp.plot_id = s.plot_id
             WHERE s.id = $1",
        )
        .bind(station_id)
        .bind(player_id)
        .fetch_optional(db)
        .await?;
    let Some((station_owner, sx, sy, range, plot_owner, station_use_level, px, py)) = row else {
        return Err(AccessError::NotFound);
    };

    let (dx, dy) = (px - sx as f64, py - sy as f64);
    station_access(
        player_id,
        &StationAccess {
            station_owner,
            plot_owner,
            station_use_level,
            distance: (dx * dx + dy * dy).sqrt(),
            interaction_range: range as f64,
        },
    )?;
    Ok(container)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chest(owner: Uuid) -> StationAccess {
        StationAccess {
            station_owner: owner,
            plot_owner: Some(owner),
            station_use_level: Some("owner".into()),
            distance: 10.0,
            interaction_range: 50.0,
        }
    }

    #[test]
    fn own_chest_is_allowed() {
        let me = Uuid::new_v4();
        assert!(station_access(me, &chest(me)).is_ok());
    }

    #[test]
    fn own_chest_off_plot_is_allowed() {
        let me = Uuid::new_v4();
        let s = StationAccess { plot_owner: None, station_use_level: None, ..chest(me) };
        assert!(station_access(me, &s).is_ok());
    }

    #[test]
    fn someone_elses_chest_is_refused() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(matches!(station_access(me, &chest(them)), Err(AccessError::Forbidden)));
    }

    #[test]
    fn friends_and_guild_levels_admit_no_one_yet() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        for level in ["friends", "guild", "bogus"] {
            let s = StationAccess { station_use_level: Some(level.into()), ..chest(them) };
            assert!(matches!(station_access(me, &s), Err(AccessError::Forbidden)), "{level}");
        }
    }

    #[test]
    fn public_station_use_admits_anyone() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        let s = StationAccess { station_use_level: Some("public".into()), ..chest(them) };
        assert!(station_access(me, &s).is_ok());
    }

    #[test]
    fn plot_owner_may_use_a_station_on_their_plot() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        let s = StationAccess { plot_owner: Some(me), ..chest(them) };
        assert!(station_access(me, &s).is_ok());
    }

    #[test]
    fn out_of_range_is_refused() {
        let me = Uuid::new_v4();
        let s = StationAccess { distance: 50.1, ..chest(me) };
        assert!(matches!(station_access(me, &s), Err(AccessError::TooFar)));
    }

    #[test]
    fn forbidden_wins_over_too_far() {
        let (me, them) = (Uuid::new_v4(), Uuid::new_v4());
        let s = StationAccess { distance: 500.0, ..chest(them) };
        assert!(matches!(station_access(me, &s), Err(AccessError::Forbidden)));
    }
}
