//! Keeps the `zones` table's radii equal to the protocol's zone table (#116).
//!
//! `capitol_protocol::rules` holds the one copy of the zone radii.
//! [`sync_zone_radii`] runs on startup and writes them into `zones`, so
//! `/bureaucracy/zones` serves what `Zone::from_position` and the generator
//! use. Changing a radius therefore needs no migration.

use sqlx::PgPool;

use crate::models::Zone;

/// What a sync changed.
#[derive(Debug, Default, PartialEq)]
pub struct ZoneSyncReport {
    /// Rows whose radii were rewritten.
    pub updated: u64,
    /// Zones with no row in the table. A seed migration adds them, so this
    /// should stay empty.
    pub missing: Vec<&'static str>,
}

/// Sets every `zones` row's `inner_radius` and `outer_radius` from `Zone::radii()`.
pub async fn sync_zone_radii(pool: &PgPool) -> Result<ZoneSyncReport, sqlx::Error> {
    let ids: Vec<&str> = Zone::ALL.iter().map(Zone::as_str).collect();
    let inner: Vec<f32> = Zone::ALL.iter().map(|z| z.radii().0 as f32).collect();
    let outer: Vec<f32> = Zone::ALL.iter().map(|z| z.radii().1 as f32).collect();

    let updated = sqlx::query(
        "UPDATE zones z SET inner_radius = v.inner_r, outer_radius = v.outer_r
         FROM UNNEST($1::text[], $2::real[], $3::real[]) AS v(id, inner_r, outer_r)
         WHERE z.id = v.id AND (z.inner_radius <> v.inner_r OR z.outer_radius <> v.outer_r)",
    )
    .bind(&ids)
    .bind(&inner)
    .bind(&outer)
    .execute(pool)
    .await?
    .rows_affected();

    let present: Vec<String> = sqlx::query_scalar("SELECT id FROM zones")
        .fetch_all(pool)
        .await?;
    let missing = Zone::ALL
        .iter()
        .map(Zone::as_str)
        .filter(|id| !present.iter().any(|p| p == id))
        .collect();

    Ok(ZoneSyncReport { updated, missing })
}

/// The `#[ignore]`d test needs the local Postgres (`docker-compose up -d`).
/// Run it with `cargo test zone_sync -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;

    async fn radii(db: &PgPool) -> Vec<(String, f32, f32)> {
        sqlx::query_as("SELECT id, inner_radius, outer_radius FROM zones ORDER BY ring_order")
            .fetch_all(db)
            .await
            .unwrap()
    }

    fn expected() -> Vec<(String, f32, f32)> {
        Zone::ALL
            .iter()
            .map(|z| (z.as_str().to_string(), z.radii().0 as f32, z.radii().1 as f32))
            .collect()
    }

    /// Whatever the table held, after a sync it is the protocol's table, and a
    /// second sync has nothing to do.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_zones_table_follows_the_protocol(db: PgPool) {
        // The migrations' radii are history (#117 moved four zones), so the
        // first sync rewrites whatever differs.
        sync_zone_radii(&db).await.unwrap();
        assert_eq!(radii(&db).await, expected());

        sqlx::query("UPDATE zones SET inner_radius = 1, outer_radius = 2 WHERE id = 'suburban'")
            .execute(&db)
            .await
            .unwrap();

        let report = sync_zone_radii(&db).await.unwrap();
        assert_eq!(report, ZoneSyncReport { updated: 1, missing: vec![] });
        assert_eq!(radii(&db).await, expected());

        assert_eq!(sync_zone_radii(&db).await.unwrap(), ZoneSyncReport::default());
    }
}
