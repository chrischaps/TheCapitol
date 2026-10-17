use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Extension, Json, Router,
};
use sqlx::PgPool;
use uuid::Uuid;

use capitol_protocol::rest::{ClaimPlotResponse, ListPlotsQuery, ListPlotsResponse, PositionQuery};

use crate::engine::currency::{subtract_strands_in, TransactionType};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{Plot, PlotInfo, PLOT_COLUMNS, PlotPermissions, PlotWithJoins, UpdatePermissionsRequest};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(list_plots))
        .route("/{id}", get(get_plot))
        .route("/{id}/claim", post(claim_plot))
        .route("/{id}/permissions", get(get_permissions))
        .route("/{id}/permissions", put(update_permissions))
        .route("/at", get(get_plot_at_position))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// List plots with optional filters
#[utoipa::path(
    get,
    path = "/plots",
    params(ListPlotsQuery),
    responses(
        (status = 200, description = "Plot list", body = ListPlotsResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Plots"
)]
pub async fn list_plots(
    State(state): State<AppState>,
    Query(query): Query<ListPlotsQuery>,
) -> Result<Json<ListPlotsResponse>, AppError> {
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);

    // Build dynamic query
    let mut sql = String::from(
        "SELECT p.*, pl.name as owner_name, psc.station_capacity, z.deed_cost
         FROM plots p
         LEFT JOIN players pl ON p.owner_id = pl.id
         JOIN plot_size_categories psc ON p.size_category = psc.id
         LEFT JOIN zones z ON p.zone_id = z.id
         WHERE 1=1"
    );

    if let Some(ref zone) = query.zone {
        sql.push_str(&format!(" AND p.zone_id = '{}'", zone.replace("'", "''")));
    }

    if query.unclaimed == Some(true) {
        sql.push_str(" AND p.owner_id IS NULL");
    }

    if let Some(owner) = query.owner {
        sql.push_str(&format!(" AND p.owner_id = '{}'", owner));
    }

    sql.push_str(&format!(" ORDER BY p.zone_id, p.grid_y, p.grid_x LIMIT {} OFFSET {}", limit, offset));

    // Execute query
    let rows: Vec<PlotWithJoins> = sqlx::query_as(&sql)
        .fetch_all(&state.db)
        .await?;

    let plots: Vec<PlotInfo> = rows.into_iter().map(|row| row.to_plot_info()).collect();

    // Get total count
    let count_sql = format!(
        "SELECT COUNT(*) FROM plots p WHERE 1=1{}{}{}",
        query.zone.as_ref().map(|z| format!(" AND p.zone_id = '{}'", z.replace("'", "''"))).unwrap_or_default(),
        if query.unclaimed == Some(true) { " AND p.owner_id IS NULL" } else { "" },
        query.owner.map(|o| format!(" AND p.owner_id = '{}'", o)).unwrap_or_default()
    );

    let (total,): (i64,) = sqlx::query_as(&count_sql)
        .fetch_one(&state.db)
        .await?;

    Ok(Json(ListPlotsResponse { plots, total }))
}

/// Get a single plot by ID
#[utoipa::path(
    get,
    path = "/plots/{id}",
    params(
        ("id" = Uuid, Path, description = "Plot ID")
    ),
    responses(
        (status = 200, description = "Plot details", body = PlotInfo),
        (status = 404, description = "Plot not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Plots"
)]
pub async fn get_plot(
    State(state): State<AppState>,
    Path(plot_id): Path<Uuid>,
) -> Result<Json<PlotInfo>, AppError> {
    let row: Option<PlotWithJoins> = sqlx::query_as(
        "SELECT p.id, p.zone_id, p.grid_x, p.grid_y, p.world_x, p.world_y,
                p.bounds_min_x, p.bounds_min_y, p.bounds_max_x, p.bounds_max_y,
                p.size_category, p.plot_type, p.owner_id, p.claimed_at,
                p.assessed_value, p.last_tax_paid, p.tax_owed, p.station_count,
                p.terrain_id, p.corners,
                pl.name as owner_name, psc.station_capacity, z.deed_cost
         FROM plots p
         LEFT JOIN players pl ON p.owner_id = pl.id
         JOIN plot_size_categories psc ON p.size_category = psc.id
         LEFT JOIN zones z ON p.zone_id = z.id
         WHERE p.id = $1"
    )
    .bind(plot_id)
    .fetch_optional(&state.db)
    .await?;

    let row = row.ok_or_else(|| AppError::NotFound("Plot not found".into()))?;

    Ok(Json(row.to_plot_info()))
}

/// Get the plot at a specific position
#[utoipa::path(
    get,
    path = "/plots/at",
    params(PositionQuery),
    responses(
        (status = 200, description = "Plot at position (or null)", body = Option<PlotInfo>)
    ),
    security(("bearer_auth" = [])),
    tag = "Plots"
)]
pub async fn get_plot_at_position(
    State(state): State<AppState>,
    Query(query): Query<PositionQuery>,
) -> Result<Json<Option<PlotInfo>>, AppError> {
    let Some(plot) = find_plot_at_position(&state.db, query.x, query.y).await? else {
        return Ok(Json(None));
    };
    Ok(Json(Some(get_plot(State(state), Path(plot.id)).await?.0)))
}

#[utoipa::path(
    post,
    path = "/plots/{id}/claim",
    params(
        ("id" = Uuid, Path, description = "Plot ID to claim")
    ),
    responses(
        (status = 200, description = "Plot claimed", body = ClaimPlotResponse),
        (status = 400, description = "Plot already claimed or insufficient funds"),
        (status = 404, description = "Plot not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Plots"
)]
pub async fn claim_plot(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(plot_id): Path<Uuid>,
) -> Result<Json<ClaimPlotResponse>, AppError> {
    // Get player
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    let new_balance = claim(&state.db, player_id, plot_id).await?;

    // Fetch updated plot info
    let plot_info = get_plot(State(state.clone()), Path(plot_id)).await?.0;

    Ok(Json(ClaimPlotResponse {
        plot: plot_info,
        new_balance,
    }))
}

/// Charge the deed and give the plot to `player_id`, in one transaction. The
/// plot row is locked first, so two players can't both pay for it, and the
/// money only moves if the claim commits. Returns the new balance.
async fn claim(db: &PgPool, player_id: Uuid, plot_id: Uuid) -> Result<i64, AppError> {
    let mut tx = db.begin().await?;

    // Get plot with zone info
    let plot_row: Option<(String, Option<Uuid>, Option<i32>)> = sqlx::query_as(
        "SELECT p.zone_id, p.owner_id, z.deed_cost
         FROM plots p
         JOIN zones z ON p.zone_id = z.id
         WHERE p.id = $1
         FOR UPDATE OF p"
    )
    .bind(plot_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (zone_id, owner_id, deed_cost) = plot_row
        .ok_or_else(|| AppError::NotFound("Plot not found".into()))?;

    // Verify plot is unclaimed
    if owner_id.is_some() {
        return Err(AppError::BadRequest("Plot is already claimed".into()));
    }

    // Verify zone has plots for sale
    let deed_cost = deed_cost
        .ok_or_else(|| AppError::BadRequest("This zone does not allow plot ownership".into()))?;

    // Deduct Strands
    let new_balance = subtract_strands_in(
        &mut tx,
        player_id,
        deed_cost as i64,
        TransactionType::DeedPurchase,
        Some(&format!("Purchased plot in {}", zone_id)),
        Some(plot_id),
    )
    .await
    .map_err(AppError::BadRequest)?;

    // Claim the plot
    sqlx::query(
        "UPDATE plots SET owner_id = $1, claimed_at = NOW(), assessed_value = $2 WHERE id = $3"
    )
    .bind(player_id)
    .bind(deed_cost)
    .bind(plot_id)
    .execute(&mut *tx)
    .await?;

    // Create default permissions if not exists
    sqlx::query(
        "INSERT INTO plot_permissions (plot_id) VALUES ($1) ON CONFLICT (plot_id) DO NOTHING"
    )
    .bind(plot_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(new_balance)
}

/// Get plot permissions
#[utoipa::path(
    get,
    path = "/plots/{id}/permissions",
    params(
        ("id" = Uuid, Path, description = "Plot ID")
    ),
    responses(
        (status = 200, description = "Plot permissions", body = PlotPermissions),
        (status = 404, description = "Permissions not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Plots"
)]
pub async fn get_permissions(
    State(state): State<AppState>,
    Path(plot_id): Path<Uuid>,
) -> Result<Json<PlotPermissions>, AppError> {
    let perms: Option<PlotPermissions> = sqlx::query_as(
        "SELECT plot_id, entry_level, visibility_level, station_use_level, modify_level
         FROM plot_permissions WHERE plot_id = $1"
    )
    .bind(plot_id)
    .fetch_optional(&state.db)
    .await?;

    perms.ok_or_else(|| AppError::NotFound("Plot permissions not found".into()))
        .map(Json)
}

/// Update plot permissions (owner only)
#[utoipa::path(
    put,
    path = "/plots/{id}/permissions",
    params(
        ("id" = Uuid, Path, description = "Plot ID")
    ),
    request_body = UpdatePermissionsRequest,
    responses(
        (status = 200, description = "Permissions updated", body = PlotPermissions),
        (status = 403, description = "Not your plot")
    ),
    security(("bearer_auth" = [])),
    tag = "Plots"
)]
pub async fn update_permissions(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(plot_id): Path<Uuid>,
    Json(req): Json<UpdatePermissionsRequest>,
) -> Result<Json<PlotPermissions>, AppError> {
    // Get player
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Verify ownership
    let owner_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT owner_id FROM plots WHERE id = $1"
    )
    .bind(plot_id)
    .fetch_optional(&state.db)
    .await?
    .flatten();

    if owner_id != Some(player_id) {
        return Err(AppError::Forbidden("You don't own this plot".into()));
    }

    // Update permissions
    if let Some(entry) = &req.entry_level {
        sqlx::query("UPDATE plot_permissions SET entry_level = $1 WHERE plot_id = $2")
            .bind(entry)
            .bind(plot_id)
            .execute(&state.db)
            .await?;
    }

    if let Some(visibility) = &req.visibility_level {
        sqlx::query("UPDATE plot_permissions SET visibility_level = $1 WHERE plot_id = $2")
            .bind(visibility)
            .bind(plot_id)
            .execute(&state.db)
            .await?;
    }

    if let Some(station_use) = &req.station_use_level {
        sqlx::query("UPDATE plot_permissions SET station_use_level = $1 WHERE plot_id = $2")
            .bind(station_use)
            .bind(plot_id)
            .execute(&state.db)
            .await?;
    }

    if let Some(modify) = &req.modify_level {
        sqlx::query("UPDATE plot_permissions SET modify_level = $1 WHERE plot_id = $2")
            .bind(modify)
            .bind(plot_id)
            .execute(&state.db)
            .await?;
    }

    // Return updated permissions
    get_permissions(State(state), Path(plot_id)).await
}

/// Helper to find plot at a position (used by station placement)
/// The plot under a point: the lot that contains it, tested against the lot's
/// own corners, not its box (B7). Used by station placement (REST and WS) and
/// `/plots/at`. The box index narrows it to a few candidates first.
pub async fn find_plot_at_position(
    db: &sqlx::PgPool,
    x: f64,
    y: f64,
) -> Result<Option<Plot>, sqlx::Error> {
    // The boxes are stored as REAL, so widen them a little against rounding.
    let candidates: Vec<Plot> = sqlx::query_as(&format!(
        "SELECT {PLOT_COLUMNS} FROM plots
         WHERE $1 >= bounds_min_x - 0.01 AND $1 <= bounds_max_x + 0.01
           AND $2 >= bounds_min_y - 0.01 AND $2 <= bounds_max_y + 0.01
         ORDER BY terrain_id NULLS LAST"
    ))
    .bind(x as f32)
    .bind(y as f32)
    .fetch_all(db)
    .await?;
    Ok(candidates.into_iter().find(|p| p.contains(x, y)))
}

pub async fn get_plot_capacity(db: &sqlx::PgPool, plot_id: Uuid) -> Result<(i32, i32), sqlx::Error> {
    let row: (i32, i32) = sqlx::query_as(
        "SELECT p.station_count, psc.station_capacity
         FROM plots p
         JOIN plot_size_categories psc ON p.size_category = psc.id
         WHERE p.id = $1"
    )
    .bind(plot_id)
    .fetch_one(db)
    .await?;

    Ok(row)
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test routes::plots -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{balance, ledger, owner, plot, player};

    /// The urban deed (`zones.deed_cost`)
    async fn urban_deed(db: &PgPool) -> i64 {
        sqlx::query_scalar::<_, i32>("SELECT deed_cost FROM zones WHERE id = 'urban'")
            .fetch_one(db)
            .await
            .unwrap() as i64
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_charges_the_deed_and_takes_the_plot(db: PgPool) {
        let deed = urban_deed(&db).await;
        let ada = player(&db, "ada", deed + 5).await;
        let lot = plot(&db, None, 0).await;

        assert_eq!(claim(&db, ada, lot).await.unwrap(), 5);

        assert_eq!(owner(&db, lot).await, Some(Some(ada)));
        assert_eq!(balance(&db, ada).await, 5);
        assert_eq!(ledger(&db, ada).await, vec![(-deed, "deed_purchase".to_string())]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_you_cant_afford_takes_nothing(db: PgPool) {
        let deed = urban_deed(&db).await;
        let ada = player(&db, "ada", deed - 1).await;
        let lot = plot(&db, None, 0).await;

        assert!(matches!(claim(&db, ada, lot).await, Err(AppError::BadRequest(_))));

        assert_eq!(owner(&db, lot).await, Some(None));
        assert_eq!(balance(&db, ada).await, deed - 1);
        assert!(ledger(&db, ada).await.is_empty());
    }

    /// The second claimant waits on the plot's lock, then finds it taken and pays nothing
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn two_claims_at_once_charge_one_player(db: PgPool) {
        let deed = urban_deed(&db).await;
        let ada = player(&db, "ada", deed).await;
        let bram = player(&db, "bram", deed).await;
        let lot = plot(&db, None, 0).await;

        let (a, b) = tokio::join!(claim(&db, ada, lot), claim(&db, bram, lot));

        assert!(a.is_ok() != b.is_ok(), "exactly one claim wins");
        let (winner, loser) = if a.is_ok() { (ada, bram) } else { (bram, ada) };
        assert_eq!(owner(&db, lot).await, Some(Some(winner)));
        assert_eq!(balance(&db, winner).await, 0);
        assert_eq!(balance(&db, loser).await, deed, "the loser kept their strands");
    }
}
