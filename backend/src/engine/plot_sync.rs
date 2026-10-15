//! Keeps the `plots` table equal to the terrain's lots (B7, #42).
//!
//! The terrain generator is the source of plots. Each `/terrain` lot is one
//! row, keyed by `terrain_id`, with the lot's corners. The UUID stays the
//! primary key, so stations, tax and permissions don't change.
//! [`sync_plots`] runs on startup and on every terrain commit:
//! - a new lot is inserted, with default permissions
//! - an unowned lot that changed shape is updated
//! - an unowned row whose lot is gone is deleted
//! - an **owned** row whose lot is gone or changed shape is a conflict. What
//!   happens then is the caller's [`OnOwned`] choice.
//!
//! Size and station capacity follow the lot's area ([`size_category`]). The
//! deed cost stays per zone (`zones.deed_cost`).

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use capitol_protocol::terrain::TerrainPlot;

use crate::engine::currency::{add_strands, TransactionType};
use crate::models::corners_from_column;

/// What to do with an owned plot whose lot is gone or has changed shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnOwned {
    /// Change nothing and report the conflicts (an admin's commit without `force`).
    Refuse,
    /// Leave those plots exactly as they are, and sync everything else (startup).
    Keep,
    /// Release them, refund the deed and remove their stations (`force`).
    Release,
}

/// An owned plot the new terrain would move or remove.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedConflict {
    pub plot_id: Uuid,
    pub terrain_id: String,
    pub owner_id: Uuid,
    /// True if the lot is gone, false if it only changed shape.
    pub removed: bool,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub inserted: usize,
    pub updated: usize,
    pub deleted: usize,
    /// Owned plots this sync left alone (`OnOwned::Keep`) or released (`OnOwned::Release`).
    pub conflicts: Vec<OwnedConflict>,
    /// Stations removed from released plots. The caller drops them from the
    /// game state and tells the clients.
    pub removed_stations: Vec<Uuid>,
}

#[derive(Debug)]
pub enum SyncError {
    /// `OnOwned::Refuse` found owned plots in the way. Nothing was changed.
    OwnedConflicts(Vec<OwnedConflict>),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for SyncError {
    fn from(e: sqlx::Error) -> Self {
        SyncError::Db(e)
    }
}

/// The size category for a lot of this area, which sets its station capacity
/// (`plot_size_categories`). Urban lots come out medium, suburban large and
/// rural fields estates.
pub fn size_category(area: f64) -> &'static str {
    if area < 2_500.0 {
        "medium"
    } else if area < 15_000.0 {
        "large"
    } else {
        "estate"
    }
}

/// How far two corner sets may differ and still be the same lot. The corners
/// round-trip through the DB as f64, so only a real change is bigger.
const SAME_SHAPE: f64 = 1e-6;

fn same_shape(a: &[(f64, f64); 4], b: &[(f64, f64); 4]) -> bool {
    a.iter()
        .zip(b)
        .all(|(p, q)| (p.0 - q.0).abs() <= SAME_SHAPE && (p.1 - q.1).abs() <= SAME_SHAPE)
}

struct Existing {
    id: Uuid,
    owner_id: Option<Uuid>,
    corners: Option<[(f64, f64); 4]>,
    assessed_value: i32,
}

/// Make the `plots` table match `lots`. See the module docs.
pub async fn sync_plots(db: &PgPool, lots: &[TerrainPlot], on_owned: OnOwned) -> Result<SyncReport, SyncError> {
    let rows: Vec<(Uuid, String, Option<Uuid>, Option<Vec<f64>>, i32)> = sqlx::query_as(
        "SELECT id, terrain_id, owner_id, corners, assessed_value FROM plots WHERE terrain_id IS NOT NULL",
    )
    .fetch_all(db)
    .await?;
    let existing: HashMap<String, Existing> = rows
        .into_iter()
        .map(|(id, terrain_id, owner_id, corners, assessed_value)| {
            let corners = corners_from_column(corners.as_deref());
            (terrain_id, Existing { id, owner_id, corners, assessed_value })
        })
        .collect();
    let wanted: HashMap<&str, &TerrainPlot> = lots.iter().map(|l| (l.id.as_str(), l)).collect();

    let mut conflicts = Vec::new();
    for (terrain_id, row) in &existing {
        let Some(owner_id) = row.owner_id else { continue };
        let removed = match wanted.get(terrain_id.as_str()) {
            None => true,
            Some(lot) => match &row.corners {
                Some(c) if same_shape(c, &lot.corners) => continue,
                _ => false,
            },
        };
        conflicts.push(OwnedConflict { plot_id: row.id, terrain_id: terrain_id.clone(), owner_id, removed });
    }
    conflicts.sort_by(|a, b| a.terrain_id.cmp(&b.terrain_id));
    if on_owned == OnOwned::Refuse && !conflicts.is_empty() {
        return Err(SyncError::OwnedConflicts(conflicts));
    }

    let mut report = SyncReport::default();
    let mut refunds: Vec<(Uuid, Uuid, i32)> = Vec::new();
    let mut tx = db.begin().await?;

    if on_owned == OnOwned::Release {
        for c in &conflicts {
            let stations: Vec<Uuid> = sqlx::query_scalar("DELETE FROM stations WHERE plot_id = $1 RETURNING id")
                .bind(c.plot_id)
                .fetch_all(&mut *tx)
                .await?;
            report.removed_stations.extend(stations);
            sqlx::query(
                "UPDATE plots SET owner_id = NULL, claimed_at = NULL, assessed_value = 0,
                        tax_owed = 0, last_tax_paid = NULL, station_count = 0
                 WHERE id = $1",
            )
            .bind(c.plot_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE plot_permissions SET entry_level = 'public', visibility_level = 'public',
                        station_use_level = 'owner', modify_level = 'owner'
                 WHERE plot_id = $1",
            )
            .bind(c.plot_id)
            .execute(&mut *tx)
            .await?;
            let row = &existing[&c.terrain_id];
            refunds.push((c.owner_id, c.plot_id, row.assessed_value));
        }
    }
    // Owned plots kept as they are must not be touched below.
    let kept: Vec<&str> = if on_owned == OnOwned::Keep {
        conflicts.iter().map(|c| c.terrain_id.as_str()).collect()
    } else {
        Vec::new()
    };

    // Rows whose lot is gone. Released plots are unowned by now, so they go too.
    let gone: Vec<Uuid> = existing
        .iter()
        .filter(|(tid, row)| {
            !wanted.contains_key(tid.as_str())
                && !kept.contains(&tid.as_str())
                && (row.owner_id.is_none() || on_owned == OnOwned::Release)
        })
        .map(|(_, row)| row.id)
        .collect();
    if !gone.is_empty() {
        report.deleted = sqlx::query("DELETE FROM plots WHERE id = ANY($1)")
            .bind(&gone)
            .execute(&mut *tx)
            .await?
            .rows_affected() as usize;
    }

    // Lots that are new, or whose row differs. Owned rows are only rewritten
    // when they were just released, which made them unowned.
    let mut columns = Columns::default();
    for (index, lot) in lots.iter().enumerate() {
        if kept.contains(&lot.id.as_str()) {
            continue;
        }
        let size = size_category(lot.area);
        match existing.get(&lot.id) {
            None => report.inserted += 1,
            Some(row) => {
                let released = on_owned == OnOwned::Release && conflicts.iter().any(|c| c.plot_id == row.id);
                let unchanged = row.corners.as_ref().is_some_and(|c| same_shape(c, &lot.corners));
                if (row.owner_id.is_some() && !released) || unchanged {
                    continue;
                }
                report.updated += 1;
            }
        }
        columns.push(index, lot, size);
    }
    if !columns.terrain_id.is_empty() {
        columns.upsert(&mut tx).await?;
        sqlx::query(
            "INSERT INTO plot_permissions (plot_id)
             SELECT id FROM plots WHERE terrain_id = ANY($1)
             ON CONFLICT (plot_id) DO NOTHING",
        )
        .bind(&columns.terrain_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    // Refunds go through the currency ledger, which runs its own transaction.
    for (owner_id, plot_id, value) in refunds {
        if value <= 0 {
            continue;
        }
        if let Err(e) = add_strands(
            db,
            owner_id,
            value as i64,
            TransactionType::AdminAdjustment,
            Some("Deed refunded: the terrain changed under your plot"),
            Some(plot_id),
        )
        .await
        {
            tracing::error!("Couldn't refund {value} strands to {owner_id} for plot {plot_id}: {e}");
        }
    }

    report.conflicts = conflicts;
    Ok(report)
}

/// Lots to upsert, column by column, for one `UNNEST` insert.
#[derive(Default)]
struct Columns {
    terrain_id: Vec<String>,
    zone_id: Vec<String>,
    grid_x: Vec<i32>,
    world_x: Vec<f32>,
    world_y: Vec<f32>,
    min_x: Vec<f32>,
    min_y: Vec<f32>,
    max_x: Vec<f32>,
    max_y: Vec<f32>,
    size: Vec<String>,
    /// The corners, flattened, eight per lot.
    corners: Vec<f64>,
}

impl Columns {
    fn push(&mut self, index: usize, lot: &TerrainPlot, size: &str) {
        let c = &lot.corners;
        let (mut min, mut max) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
        for &(x, y) in c {
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        self.terrain_id.push(lot.id.clone());
        self.zone_id.push(lot.zone.clone());
        self.grid_x.push(index as i32);
        self.world_x.push((c.iter().map(|p| p.0).sum::<f64>() / 4.0) as f32);
        self.world_y.push((c.iter().map(|p| p.1).sum::<f64>() / 4.0) as f32);
        // REAL rounds, so round the box outwards to keep the lot inside it.
        self.min_x.push(next_down(min.0));
        self.min_y.push(next_down(min.1));
        self.max_x.push(next_up(max.0));
        self.max_y.push(next_up(max.1));
        self.size.push(size.to_string());
        self.corners.extend(c.iter().flat_map(|&(x, y)| [x, y]));
    }

    async fn upsert(&self, tx: &mut sqlx::PgConnection) -> Result<(), sqlx::Error> {
        // UNNEST can't carry an array per row, so the corners go as a flat array
        // and each row slices its eight out by ordinality.
        sqlx::query(
            "INSERT INTO plots (terrain_id, zone_id, grid_x, grid_y, world_x, world_y,
                                bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y,
                                size_category, plot_type, corners)
             SELECT l.terrain_id, l.zone_id, l.grid_x, 0, l.world_x, l.world_y,
                    l.min_x, l.min_y, l.max_x, l.max_y, l.size, 'residential',
                    ($11::float8[])[(l.n::int4 - 1) * 8 + 1 : l.n::int4 * 8]
             FROM UNNEST($1::text[], $2::text[], $3::int4[], $4::float4[], $5::float4[],
                         $6::float4[], $7::float4[], $8::float4[], $9::float4[], $10::text[])
                  WITH ORDINALITY
                  AS l(terrain_id, zone_id, grid_x, world_x, world_y, min_x, min_y, max_x, max_y, size, n)
             ON CONFLICT (terrain_id) DO UPDATE SET
                 zone_id = EXCLUDED.zone_id, grid_x = EXCLUDED.grid_x,
                 world_x = EXCLUDED.world_x, world_y = EXCLUDED.world_y,
                 bounds_min_x = EXCLUDED.bounds_min_x, bounds_min_y = EXCLUDED.bounds_min_y,
                 bounds_max_x = EXCLUDED.bounds_max_x, bounds_max_y = EXCLUDED.bounds_max_y,
                 size_category = EXCLUDED.size_category, corners = EXCLUDED.corners",
        )
        .bind(&self.terrain_id)
        .bind(&self.zone_id)
        .bind(&self.grid_x)
        .bind(&self.world_x)
        .bind(&self.world_y)
        .bind(&self.min_x)
        .bind(&self.min_y)
        .bind(&self.max_x)
        .bind(&self.max_y)
        .bind(&self.size)
        .bind(&self.corners)
        .execute(tx)
        .await?;
        Ok(())
    }
}

/// The largest f32 at or below `v`.
fn next_down(v: f64) -> f32 {
    let f = v as f32;
    if (f as f64) > v { f32::from_bits(if f > 0.0 { f.to_bits() - 1 } else { f.to_bits() + 1 }) } else { f }
}

/// The smallest f32 at or above `v`.
fn next_up(v: f64) -> f32 {
    let f = v as f32;
    if (f as f64) < v { f32::from_bits(if f >= 0.0 { f.to_bits() + 1 } else { f.to_bits() - 1 }) } else { f }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_area() {
        assert_eq!(size_category(1_440.0), "medium");
        assert_eq!(size_category(7_100.0), "large");
        assert_eq!(size_category(28_600.0), "estate");
    }

    #[test]
    fn rounded_boxes_hold_their_lot() {
        for v in [4913.414505936231, -0.1, 0.1, 1234.56789, 7999.99999] {
            assert!((next_down(v) as f64) <= v && (next_up(v) as f64) >= v, "{v}");
        }
    }
}
