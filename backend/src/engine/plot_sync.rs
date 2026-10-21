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
//!   happens then is the caller's [`OnOwned`] choice. A plot kept as it was
//!   holds back the lots that overlap it (#119): they aren't laid until it's
//!   released, so nobody can claim ground that's still someone's.
//! - a workbench standing off a plot (D10) that a lot now covers is a
//!   conflict too (#188). It's taken down, as a foreclosure takes a plot's
//!   stations down, unless the caller refuses. Left standing it would be its
//!   owner's on someone else's lot, counted by no plot and closed to the
//!   lot's owner.
//!
//! Size and station capacity follow the lot's area ([`size_category`]). The
//! deed cost stays per zone (`zones.deed_cost`).

use std::collections::HashMap;

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use capitol_protocol::terrain::{quad_contains, TerrainPlot};
use capitol_protocol::ws::ServerMessage;

use crate::engine::currency::{add_strands_in, TransactionType};
use crate::engine::notices;
use crate::engine::station_removal::{return_plot_stations, return_stations};
use crate::models::corners_from_column;
use crate::models::geom::quads_overlap_strict;

/// What to do with an owned plot whose lot is gone or has changed shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnOwned {
    /// Change nothing and report the conflicts (an admin's commit without `force`).
    /// Covered workbenches are conflicts as well as owned plots.
    Refuse,
    /// Leave those plots exactly as they are, and sync everything else (startup).
    /// Covered workbenches are still taken down.
    Keep,
    /// Release them, refund the deed and remove their stations, their contents
    /// going back as a foreclosure's do, to inventories or impounds (`force`).
    /// Covered workbenches are taken down.
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

/// A station off a plot (D10) that a lot of the new terrain covers (#188).
#[derive(Debug, Clone, PartialEq)]
pub struct CoveredStation {
    pub station_id: Uuid,
    pub station_type: String,
    pub owner_id: Uuid,
    /// The lot now under it
    pub terrain_id: String,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub inserted: usize,
    pub updated: usize,
    pub deleted: usize,
    /// Owned plots this sync left alone (`OnOwned::Keep`) or released (`OnOwned::Release`).
    pub conflicts: Vec<OwnedConflict>,
    /// The `terrain_id`s of the lots not laid because they overlap a plot
    /// kept as it was (`OnOwned::Keep`, #119). They're laid once it's released.
    pub held_back: Vec<String>,
    /// Stations removed from released plots, and covered workbenches taken
    /// down. The caller drops them from the game state and tells the clients.
    pub removed_stations: Vec<Uuid>,
    /// The covered workbenches taken down (#188), each with the
    /// `station_displaced` notice queued for its owner. The caller tells them.
    pub displaced: Vec<(CoveredStation, Uuid)>,
    /// `(player, container)` for each inventory, or impound (#141), that
    /// took a kit, tool or box's contents back. The caller sends
    /// `inventory_updated`.
    pub returned_to: Vec<(Uuid, Uuid)>,
    /// The `terrain_id`s of the plots released (`OnOwned::Release`). The
    /// caller tells the clients (`plot_released`, B13).
    pub released: Vec<String>,
    /// The owners whose deeds were refunded. The caller tells them their
    /// balance (`currency_changed`, #148).
    pub refunded: Vec<Uuid>,
}

#[derive(Debug)]
pub enum SyncError {
    /// `OnOwned::Refuse` found owned plots, or workbenches off a plot, in
    /// the way. Nothing was changed.
    Conflicts { plots: Vec<OwnedConflict>, stations: Vec<CoveredStation> },
    /// A released plot's deed couldn't be refunded. Nothing was changed.
    Refund(String),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for SyncError {
    fn from(e: sqlx::Error) -> Self {
        SyncError::Db(e)
    }
}

/// The size category for a lot in this zone of this area, which sets its
/// station capacity (`plot_size_categories`). Urban lots come out medium,
/// suburban large and rural fields estates. A guild district lot is always a
/// guild hall, whatever its area (#83): that's the zone's `plot_size_category`.
pub fn size_category(zone: &str, area: f64) -> &'static str {
    if zone == "guild_district" {
        "guild_hall"
    } else if area < 2_500.0 {
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

/// Take a plot from its owner, inside the caller's transaction: its stations
/// go (their containers and contents with them), and the plot is back to an
/// unclaimed one with default permissions. Returns the removed stations, for
/// the caller to drop from the game state and announce. Refunding the deed is
/// the caller's, as is `plot_released`. Used by a forced terrain commit and by
/// tax foreclosure (#131), which empties the stations into inventories first
/// (`station_removal::return_plot_stations`, #137).
pub async fn release_plot(tx: &mut PgConnection, plot_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    let stations: Vec<Uuid> = sqlx::query_scalar("DELETE FROM stations WHERE plot_id = $1 RETURNING id")
        .bind(plot_id)
        .fetch_all(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE plots SET owner_id = NULL, claimed_at = NULL, assessed_value = 0,
                tax_owed = 0, tax_carry = 0, last_tax_paid = NULL, station_count = 0
         WHERE id = $1",
    )
    .bind(plot_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE plot_permissions SET entry_level = 'public', visibility_level = 'public',
                station_use_level = 'owner', modify_level = 'owner'
         WHERE plot_id = $1",
    )
    .bind(plot_id)
    .execute(&mut *tx)
    .await?;
    Ok(stations)
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
    // Owned plots kept as they are must not be touched below.
    let kept: Vec<&str> = if on_owned == OnOwned::Keep {
        conflicts.iter().map(|c| c.terrain_id.as_str()).collect()
    } else {
        Vec::new()
    };

    // A kept plot keeps its old ground, so no lot may be laid over it (#119).
    // The lots it overlaps are held back, an unowned row of one deleted, until
    // it's released and a later sync lays them.
    let held = held_back(lots, &existing, &kept);

    // A kept plot keeps its old shape, and nothing could stand off a plot
    // inside that, so only the other lots can cover a workbench.
    let off_plot: Vec<OffPlot> = sqlx::query_as(
        "SELECT id, station_type, owner_id, position_x, position_y FROM stations WHERE plot_id IS NULL",
    )
    .fetch_all(db)
    .await?;
    let covered = covered(&off_plot, lots.iter().filter(|l| !kept.contains(&l.id.as_str()) && !held.contains(&l.id.as_str())));

    if on_owned == OnOwned::Refuse && !(conflicts.is_empty() && covered.is_empty()) {
        return Err(SyncError::Conflicts { plots: conflicts, stations: covered });
    }

    let mut report = SyncReport::default();
    let mut tx = db.begin().await?;
    // Kits back, and what found no room, counted once it commits (C1.7)
    let mut flows = crate::engine::ledger::Flows::new();

    if on_owned == OnOwned::Release {
        for c in &conflicts {
            report.released.push(c.terrain_id.clone());
            let returned = return_plot_stations(&mut tx, c.plot_id).await?;
            returned.count_into(&mut flows);
            if !returned.lost.is_empty() {
                tracing::warn!("Releasing plot {}: no room for {:?}, lost with its stations", c.plot_id, returned.lost);
            }
            took(&mut report.returned_to, returned.containers);
            report.removed_stations.extend(release_plot(&mut tx, c.plot_id).await?);
            // The refund is in this transaction, so a plot is never released
            // without its owner getting the deed back.
            let value = existing[&c.terrain_id].assessed_value;
            if value > 0 {
                add_strands_in(
                    &mut tx,
                    c.owner_id,
                    value as i64,
                    TransactionType::DeedRefund,
                    Some("Deed refunded: the terrain changed under your plot"),
                    Some(c.plot_id),
                )
                .await
                .map_err(|e| SyncError::Refund(format!("Couldn't refund {value} strands to {} for plot {}: {e}", c.owner_id, c.plot_id)))?;
                if !report.refunded.contains(&c.owner_id) {
                    report.refunded.push(c.owner_id);
                }
            }
        }
    }
    // Rows whose lot is gone or held back. Released plots are unowned by now,
    // so they go too.
    let gone: Vec<Uuid> = existing
        .iter()
        .filter(|(tid, row)| {
            (!wanted.contains_key(tid.as_str()) || held.contains(&tid.as_str()))
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
        if kept.contains(&lot.id.as_str()) || held.contains(&lot.id.as_str()) {
            continue;
        }
        let size = size_category(&lot.zone, lot.area);
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

    // Covered workbenches come down, their kits, tools and contents back to
    // their owners (`station_removal::return_stations`).
    if !covered.is_empty() {
        let ids: Vec<Uuid> = covered.iter().map(|c| c.station_id).collect();
        let returned = return_stations(&mut tx, &ids).await?;
        returned.count_into(&mut flows);
        if !returned.lost.is_empty() {
            tracing::warn!("Taking down workbenches under new lots: no room for {:?}, lost with them", returned.lost);
        }
        took(&mut report.returned_to, returned.containers);
        sqlx::query("DELETE FROM stations WHERE id = ANY($1)").bind(&ids).execute(&mut *tx).await?;
        for c in covered {
            let message = ServerMessage::StationDisplaced {
                station_id: c.station_id,
                station_type: c.station_type.clone(),
                terrain_id: c.terrain_id.clone(),
            };
            let notice = notices::queue(&mut *tx, c.owner_id, &c.station_id.to_string(), &message).await?;
            report.removed_stations.push(c.station_id);
            report.displaced.push((c, notice));
        }
    }
    crate::engine::ledger::commit(tx, flows).await?;

    report.conflicts = conflicts;
    report.held_back = held.into_iter().map(String::from).collect();
    Ok(report)
}

/// The lots that overlap one of the `kept` owned plots, by id, sorted. A lot
/// with an owned row of its own is never one: that row isn't touched anyway.
/// A kept row without corners can't be tested, and holds nothing back.
fn held_back<'a>(lots: &'a [TerrainPlot], existing: &HashMap<String, Existing>, kept: &[&str]) -> Vec<&'a str> {
    let grounds: Vec<&[(f64, f64); 4]> = kept.iter().filter_map(|tid| existing[*tid].corners.as_ref()).collect();
    if grounds.is_empty() {
        return Vec::new();
    }
    let mut held: Vec<&str> = lots
        .iter()
        .filter(|l| !kept.contains(&l.id.as_str()))
        .filter(|l| existing.get(&l.id).is_none_or(|row| row.owner_id.is_none()))
        .filter(|l| grounds.iter().any(|g| quads_overlap_strict(g, &l.corners)))
        .map(|l| l.id.as_str())
        .collect();
    held.sort_unstable();
    held
}

/// A station off a plot, as `(id, station_type, owner_id, x, y)`
type OffPlot = (Uuid, String, Uuid, f32, f32);

/// The stations of `off_plot` that one of `lots` covers, each with the first
/// lot that does.
fn covered<'a>(off_plot: &[OffPlot], lots: impl Iterator<Item = &'a TerrainPlot> + Clone) -> Vec<CoveredStation> {
    off_plot
        .iter()
        .filter_map(|(station_id, station_type, owner_id, x, y)| {
            let (x, y) = (*x as f64, *y as f64);
            let lot = lots.clone().find(|l| quad_contains(&l.corners, x, y))?;
            Some(CoveredStation {
                station_id: *station_id,
                station_type: station_type.clone(),
                owner_id: *owner_id,
                terrain_id: lot.id.clone(),
            })
        })
        .collect()
}

fn took(into: &mut Vec<(Uuid, Uuid)>, containers: Vec<(Uuid, Uuid)>) {
    for c in containers {
        if !into.contains(&c) {
            into.push(c);
        }
    }
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

/// The `#[ignore]`d ones need the local Postgres (`docker-compose up -d`). Each
/// runs in a fresh database that `sqlx::test` makes from the migrations. Run
/// them with `cargo test plot_sync -- --ignored`. The helpers are shared with
/// `routes::plots`'s tests.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) async fn player(db: &PgPool, name: &str, balance: i64) -> Uuid {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(account)
            .bind(format!("{name}@test"))
            .execute(db)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name, strand_balance) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(account)
            .bind(name)
            .bind(balance)
            .execute(db)
            .await
            .unwrap();
        id
    }

    /// An urban lot, owned by `owner` with deed value `value`
    pub(crate) async fn plot(db: &PgPool, owner: Option<Uuid>, value: i32) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO plots (id, terrain_id, zone_id, grid_x, grid_y, world_x, world_y,
                                bounds_min_x, bounds_min_y, bounds_max_x, bounds_max_y,
                                size_category, owner_id, assessed_value)
             VALUES ($1, $2, 'urban', 0, 0, 8000, 6800, 7990, 6790, 8010, 6810, 'medium', $3, $4)",
        )
        .bind(id)
        .bind(format!("test-{id}"))
        .bind(owner)
        .bind(value)
        .execute(db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO plot_permissions (plot_id) VALUES ($1)")
            .bind(id)
            .execute(db)
            .await
            .unwrap();
        id
    }

    pub(crate) async fn balance(db: &PgPool, player: Uuid) -> i64 {
        sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
            .bind(player)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// The player's audit records, as (amount, transaction_type)
    pub(crate) async fn ledger(db: &PgPool, player: Uuid) -> Vec<(i64, String)> {
        sqlx::query_as("SELECT amount, transaction_type FROM currency_transactions WHERE player_id = $1")
            .bind(player)
            .fetch_all(db)
            .await
            .unwrap()
    }

    /// The plot's owner, or `None` if the row is gone
    pub(crate) async fn owner(db: &PgPool, plot: Uuid) -> Option<Option<Uuid>> {
        sqlx::query_scalar("SELECT owner_id FROM plots WHERE id = $1")
            .bind(plot)
            .fetch_optional(db)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_released_plot_refunds_its_deed(db: PgPool) {
        let ada = player(&db, "ada", 50).await;
        let lot = plot(&db, Some(ada), 1000).await;

        let report = sync_plots(&db, &[], OnOwned::Release).await.unwrap();

        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.released, vec![format!("test-{lot}")], "for plot_released");
        assert_eq!(owner(&db, lot).await, None, "the lot is gone, so its row is");
        assert_eq!(balance(&db, ada).await, 1050);
        assert_eq!(ledger(&db, ada).await, vec![(1000, "deed_refund".to_string())]);
    }

    /// The refund is in the sync's transaction, so a failed one releases nothing
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_refund_that_fails_changes_nothing(db: PgPool) {
        let ada = player(&db, "ada", i64::MAX - 10).await;
        let lot = plot(&db, Some(ada), 1000).await;
        let spare = plot(&db, None, 0).await;

        let err = sync_plots(&db, &[], OnOwned::Release).await.unwrap_err();

        assert!(matches!(err, SyncError::Refund(_)), "{err:?}");
        assert_eq!(owner(&db, lot).await, Some(Some(ada)), "still ada's");
        assert_eq!(owner(&db, spare).await, Some(None), "the unowned lot wasn't deleted either");
        assert_eq!(balance(&db, ada).await, i64::MAX - 10);
        assert!(ledger(&db, ada).await.is_empty());
    }

    /// A square lot `id` of side 20 centred on (`x`, `y`)
    fn lot(id: &str, x: f64, y: f64) -> TerrainPlot {
        TerrainPlot {
            id: id.into(),
            corners: [(x - 10.0, y - 10.0), (x + 10.0, y - 10.0), (x + 10.0, y + 10.0), (x - 10.0, y + 10.0)],
            street_facing_edge: (0, 1),
            area: 400.0,
            adjacent_street_id: "street".into(),
            zone: "urban".into(),
        }
    }

    /// An inventory for `player`
    async fn inventory(db: &PgPool, player: Uuid) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO containers (id, container_type, owner_id) VALUES ($1, 'player_inventory', $2) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(player)
        .fetch_one(db)
        .await
        .unwrap()
    }

    /// A workbench of `owner`'s off any plot at (`x`, `y`), with 3 fiber in its box
    async fn bench(db: &PgPool, owner: Uuid, x: f32, y: f32) -> Uuid {
        crate::content::sync(db, &crate::content::load().unwrap()).await.unwrap();
        let station = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO stations (id, station_type, owner_id, position_x, position_y, kit_quality)
             VALUES ($1, 'workbench', $2, $3, $4, 61)",
        )
        .bind(station)
        .bind(owner)
        .bind(x)
        .bind(y)
        .execute(db)
        .await
        .unwrap();
        let container: Uuid = sqlx::query_scalar(
            "INSERT INTO containers (id, container_type, station_id) VALUES ($1, 'station_inventory', $2) RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(station)
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO items (id, item_type, quality, quantity, owner_id, container_id, slot_index)
             VALUES ($1, 'fiber', 40, 3, $2, $3, 0)",
        )
        .bind(Uuid::new_v4())
        .bind(owner)
        .bind(container)
        .execute(db)
        .await
        .unwrap();
        station
    }

    async fn station_exists(db: &PgPool, id: Uuid) -> bool {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM stations WHERE id = $1)")
            .bind(id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// What's in a container, as (item_type, quantity) by slot
    async fn held(db: &PgPool, container: Uuid) -> Vec<(String, i32)> {
        sqlx::query_as("SELECT item_type, quantity FROM items WHERE container_id = $1 ORDER BY slot_index")
            .bind(container)
            .fetch_all(db)
            .await
            .unwrap()
    }

    fn displaced(station_id: Uuid, terrain_id: &str) -> ServerMessage {
        ServerMessage::StationDisplaced { station_id, station_type: "workbench".into(), terrain_id: terrain_id.into() }
    }

    #[test]
    fn a_bench_is_covered_by_the_lot_it_stands_in() {
        let (under, beside, ada) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(9));
        let off_plot = vec![
            (under, "workbench".to_string(), ada, 103.0, 98.0),
            (beside, "workbench".to_string(), ada, 115.0, 100.0),
        ];
        let lots = [lot("far", 500.0, 500.0), lot("near", 100.0, 100.0)];

        assert_eq!(
            covered(&off_plot, lots.iter()),
            vec![CoveredStation { station_id: under, station_type: "workbench".into(), owner_id: ada, terrain_id: "near".into() }]
        );
        assert!(covered(&off_plot, lots[..1].iter()).is_empty());
    }

    /// #188: a new lot over a bench takes it down, and what it held goes back to its owner
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_new_lot_takes_down_the_bench_under_it(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let pack = inventory(&db, ada).await;
        let under = bench(&db, ada, 8003.0, 6801.0).await;
        let beside = bench(&db, ada, 8100.0, 6800.0).await;

        let report = sync_plots(&db, &[lot("new", 8000.0, 6800.0)], OnOwned::Keep).await.unwrap();

        assert_eq!(report.inserted, 1);
        assert!(!station_exists(&db, under).await);
        assert!(station_exists(&db, beside).await, "open ground is still open");
        assert_eq!(report.removed_stations, vec![under]);
        assert_eq!(report.returned_to, vec![(ada, pack)]);
        assert_eq!(held(&db, pack).await, vec![("workbench_kit".to_string(), 1), ("fiber".to_string(), 3)]);
        let (c, notice) = &report.displaced[0];
        assert_eq!((c.station_id, c.owner_id, c.terrain_id.as_str()), (under, ada, "new"));
        assert!(notices::take(&db, *notice).await.unwrap(), "queued for ada");
    }

    /// The notice waits for an owner who's away
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_owner_hears_when_they_come_back(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        inventory(&db, ada).await;
        let under = bench(&db, ada, 8000.0, 6800.0).await;

        sync_plots(&db, &[lot("new", 8000.0, 6800.0)], OnOwned::Release).await.unwrap();

        assert_eq!(notices::take_all(&db, ada).await.unwrap(), vec![displaced(under, "new")]);
    }

    /// An admin's commit without `force` refuses, and changes nothing
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_commit_without_force_refuses_to_cover_a_bench(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let pack = inventory(&db, ada).await;
        let under = bench(&db, ada, 8000.0, 6800.0).await;

        let err = sync_plots(&db, &[lot("new", 8000.0, 6800.0)], OnOwned::Refuse).await.unwrap_err();

        match err {
            SyncError::Conflicts { plots, stations } => {
                assert!(plots.is_empty());
                assert_eq!(stations.iter().map(|c| c.station_id).collect::<Vec<_>>(), vec![under]);
            }
            e => panic!("{e:?}"),
        }
        assert!(station_exists(&db, under).await);
        assert!(held(&db, pack).await.is_empty());
        let new: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plots WHERE terrain_id = 'new'").fetch_one(&db).await.unwrap();
        assert_eq!(new, 0);
    }

    /// A forced commit gives a released plot's stations back too, as a foreclosure does
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_released_plot_gives_its_stations_back(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let pack = inventory(&db, ada).await;
        let lot_id = plot(&db, Some(ada), 0).await;
        let on_it = bench(&db, ada, 8000.0, 6800.0).await;
        sqlx::query("UPDATE stations SET plot_id = $1 WHERE id = $2").bind(lot_id).bind(on_it).execute(&db).await.unwrap();

        let report = sync_plots(&db, &[], OnOwned::Release).await.unwrap();

        assert_eq!(report.removed_stations, vec![on_it]);
        assert!(report.displaced.is_empty(), "it stood on a plot");
        assert_eq!(report.returned_to, vec![(ada, pack)]);
        assert_eq!(held(&db, pack).await, vec![("workbench_kit".to_string(), 1), ("fiber".to_string(), 3)]);
    }

    /// Gives a plot the corners of `lot`
    async fn shape(db: &PgPool, plot: Uuid, lot: &TerrainPlot) {
        let corners: Vec<f64> = lot.corners.iter().flat_map(|&(x, y)| [x, y]).collect();
        sqlx::query("UPDATE plots SET corners = $2 WHERE id = $1").bind(plot).bind(corners).execute(db).await.unwrap();
    }

    async fn terrain_ids(db: &PgPool) -> Vec<String> {
        sqlx::query_scalar("SELECT terrain_id FROM plots WHERE terrain_id IS NOT NULL ORDER BY terrain_id")
            .fetch_all(db)
            .await
            .unwrap()
    }

    /// #119: a plot kept as it was holds back the lots over it until it's released
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_kept_plot_holds_back_the_lots_over_it(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let hers = plot(&db, Some(ada), 1000).await;
        shape(&db, hers, &lot("old", 8000.0, 6800.0)).await;
        // An unowned lot elsewhere that the new terrain moves onto her ground
        let moving = plot(&db, None, 0).await;
        shape(&db, moving, &lot("elsewhere", 8300.0, 6800.0)).await;
        let moved = format!("test-{moving}");
        let lots = [
            lot("over", 8005.0, 6803.0),
            lot(&moved, 7995.0, 6795.0),
            lot("beside", 8020.0, 6800.0), // shares her edge, no more
            lot("far", 8200.0, 6800.0),
        ];

        let report = sync_plots(&db, &lots, OnOwned::Keep).await.unwrap();

        assert_eq!(report.held_back, vec!["over".to_string(), moved.clone()]);
        assert_eq!((report.inserted, report.deleted), (2, 1), "beside and far laid, the moved row gone");
        assert_eq!(terrain_ids(&db).await, vec!["beside".to_string(), "far".into(), format!("test-{hers}")]);
        assert_eq!(owner(&db, hers).await, Some(Some(ada)), "still hers, as it was");

        // Once she lets it go, the next sync lays them
        sqlx::query("UPDATE plots SET owner_id = NULL WHERE id = $1").bind(hers).execute(&db).await.unwrap();
        let report = sync_plots(&db, &lots, OnOwned::Keep).await.unwrap();

        assert!(report.held_back.is_empty());
        assert_eq!(terrain_ids(&db).await, vec!["beside".to_string(), "far".into(), "over".into(), moved]);
    }

    #[test]
    fn sizes_follow_area() {
        assert_eq!(size_category("urban", 1_440.0), "medium");
        assert_eq!(size_category("suburban", 7_100.0), "large");
        assert_eq!(size_category("rural", 28_600.0), "estate");
        assert_eq!(size_category("guild_district", 1_440.0), "guild_hall");
        assert_eq!(size_category("guild_district", 28_600.0), "guild_hall");
    }

    #[test]
    fn rounded_boxes_hold_their_lot() {
        for v in [4913.414505936231, -0.1, 0.1, 1234.56789, 7999.99999] {
            assert!((next_down(v) as f64) <= v && (next_up(v) as f64) >= v, "{v}");
        }
    }
}
