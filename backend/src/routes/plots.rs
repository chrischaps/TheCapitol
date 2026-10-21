use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Extension, Json, Router,
};
use capitol_protocol::tunables::Tunables;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::broadcast;
use tower_http::compression::CompressionLayer;
use uuid::Uuid;

use capitol_protocol::rules::Zone;
use capitol_protocol::rest::{
    ClaimPlotResponse, ClaimedPlot, ListPlotsQuery, ListPlotsResponse, PayTaxRequest, PayTaxResponse, PositionQuery,
};

use crate::engine::currency::{announce_balances, subtract_strands_in, TransactionType};
use crate::engine::gazette::{self, DeedEvent};
use crate::engine::property_tax::pay_tax_debt;
use crate::engine::GameEvent;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{Plot, PlotInfo, PLOT_COLUMNS, PlotPermissions, PlotWithJoins, UpdatePermissionsRequest};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(list_plots))
        .route("/claimed", get(list_claimed_plots).layer(CompressionLayer::new()))
        .route("/{id}", get(get_plot))
        .route("/{id}/claim", post(claim_plot))
        .route("/{id}/pay-tax", post(pay_tax))
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
        "SELECT p.*, pl.name as owner_name, COALESCE(acc.kind = 'citizen', FALSE) AS owner_citizen,
                psc.station_capacity, z.deed_cost
         FROM plots p
         LEFT JOIN players pl ON p.owner_id = pl.id
         LEFT JOIN accounts acc ON acc.id = pl.account_id
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

    let plots: Vec<PlotInfo> = rows.into_iter().map(|row| row.to_plot_info(&state.tunables.get())).collect();

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

/// Every claimed lot, unpaged
///
/// One row per owned plot that is a `/terrain` lot, for drawing everyone's
/// plots (P8.6b's houses). WS `plot_claimed` and `plot_released` keep it
/// current. Gzipped when the request accepts it.
#[utoipa::path(
    get,
    path = "/plots/claimed",
    responses(
        (status = 200, description = "Every claimed lot, by terrain_id", body = Vec<ClaimedPlot>)
    ),
    security(("bearer_auth" = [])),
    tag = "Plots"
)]
pub async fn list_claimed_plots(State(state): State<AppState>) -> Result<Json<Vec<ClaimedPlot>>, AppError> {
    Ok(Json(claimed_plots(&state.db).await?))
}

/// A pre-B7 plot with no lot is left out: clients have nothing to draw it on.
/// An owned plot with no `claimed_at` (none should have one) reads as claimed
/// long ago, at the Unix epoch.
async fn claimed_plots(db: &PgPool) -> Result<Vec<ClaimedPlot>, sqlx::Error> {
    sqlx::query_as(
        "SELECT terrain_id, owner_id, COALESCE(claimed_at, 'epoch'::timestamptz) AS claimed_at
         FROM plots
         WHERE owner_id IS NOT NULL AND terrain_id IS NOT NULL
         ORDER BY terrain_id",
    )
    .fetch_all(db)
    .await
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
                pl.name as owner_name, COALESCE(acc.kind = 'citizen', FALSE) AS owner_citizen,
                psc.station_capacity, z.deed_cost
         FROM plots p
         LEFT JOIN players pl ON p.owner_id = pl.id
         LEFT JOIN accounts acc ON acc.id = pl.account_id
         JOIN plot_size_categories psc ON p.size_category = psc.id
         LEFT JOIN zones z ON p.zone_id = z.id
         WHERE p.id = $1"
    )
    .bind(plot_id)
    .fetch_optional(&state.db)
    .await?;

    let row = row.ok_or_else(|| AppError::NotFound("Plot not found".into()))?;

    Ok(Json(row.to_plot_info(&state.tunables.get())))
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

    let events = state.game.read().await.event_sender.clone();
    let new_balance = claim(&state.db, &events, player_id, plot_id, &state.tunables.get()).await?;

    // Fetch updated plot info
    let plot_info = get_plot(State(state.clone()), Path(plot_id)).await?.0;

    Ok(Json(ClaimPlotResponse {
        plot: plot_info,
        new_balance,
    }))
}

/// Pay off property tax owed on your plot (#138)
#[utoipa::path(
    post,
    path = "/plots/{id}/pay-tax",
    params(
        ("id" = Uuid, Path, description = "Plot ID")
    ),
    request_body = PayTaxRequest,
    responses(
        (status = 200, description = "Tax paid", body = PayTaxResponse),
        (status = 400, description = "Nothing owed, or insufficient funds"),
        (status = 403, description = "Not your plot"),
        (status = 404, description = "Plot not found")
    ),
    security(("bearer_auth" = [])),
    tag = "Plots"
)]
pub async fn pay_tax(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(plot_id): Path<Uuid>,
    Json(request): Json<PayTaxRequest>,
) -> Result<Json<PayTaxResponse>, AppError> {
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    let (_, new_balance) = pay_tax_debt(&state.db, player_id, plot_id, request.amount).await?;
    let events = state.game.read().await.event_sender.clone();
    announce_balances(&events, [(player_id, new_balance)]);
    let plot = get_plot(State(state.clone()), Path(plot_id)).await?.0;
    Ok(Json(PayTaxResponse { plot, new_balance }))
}

/// Charge the deed and give the plot to `player_id`, in one transaction. The
/// plot row is locked first, so two players can't both pay for it, and the
/// money only moves if the claim commits. Once it has, everyone is told
/// (`plot_claimed`, B13), and the player hears their balance. Returns it.
const NOT_CLAIMABLE: &str = "This zone does not allow plot ownership";

/// Whether a plot can be owned: its zone has plots, and so does the ground it
/// stands on. Nothing in the wilds is claimed (WD6, #194), whatever its row says.
fn claimable(zone_id: &str, x: f64, y: f64) -> bool {
    Zone::from_str(zone_id).is_some_and(|zone| zone.has_plots()) && Zone::from_position(x, y).has_plots()
}

async fn claim(
    db: &PgPool,
    events: &broadcast::Sender<GameEvent>,
    player_id: Uuid,
    plot_id: Uuid,
    tunables: &Tunables,
) -> Result<i64, AppError> {
    let mut tx = db.begin().await?;

    // Get plot with zone info
    let plot_row: Option<(String, Option<Uuid>, Option<i32>, f32, f32)> = sqlx::query_as(
        "SELECT p.zone_id, p.owner_id, z.deed_cost, p.world_x, p.world_y
         FROM plots p
         JOIN zones z ON p.zone_id = z.id
         WHERE p.id = $1
         FOR UPDATE OF p"
    )
    .bind(plot_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (zone_id, owner_id, deed_cost, world_x, world_y) = plot_row
        .ok_or_else(|| AppError::NotFound("Plot not found".into()))?;

    // Verify plot is unclaimed
    if owner_id.is_some() {
        return Err(AppError::BadRequest("Plot is already claimed".into()));
    }

    // Verify zone has plots for sale: by its zone and where it stands (WD6),
    // and by its deed
    if !claimable(&zone_id, world_x as f64, world_y as f64) {
        return Err(AppError::BadRequest(NOT_CLAIMABLE.into()));
    }
    let deed_cost = deed_cost.ok_or_else(|| AppError::BadRequest(NOT_CLAIMABLE.into()))?;
    // The zone's deed price tunable (D6.1). The plot is assessed at what was paid.
    let deed_cost = tunables.deed_cost(&zone_id, deed_cost);

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
    let (terrain_id, claimed_at): (Option<String>, DateTime<Utc>) = sqlx::query_as(
        "UPDATE plots SET owner_id = $1, claimed_at = NOW(), assessed_value = $2 WHERE id = $3
         RETURNING terrain_id, claimed_at"
    )
    .bind(player_id)
    .bind(deed_cost)
    .bind(plot_id)
    .fetch_one(&mut *tx)
    .await?;
    gazette::record_deed(&mut tx, plot_id, &zone_id, DeedEvent::Claimed, player_id).await?;

    // Create default permissions if not exists
    sqlx::query(
        "INSERT INTO plot_permissions (plot_id) VALUES ($1) ON CONFLICT (plot_id) DO NOTHING"
    )
    .bind(plot_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // A pre-B7 plot has no lot for clients to draw, as in `/plots/claimed`.
    if let Some(terrain_id) = terrain_id {
        let _ = events.send(GameEvent::PlotClaimed { terrain_id, owner_id: player_id, claimed_at });
    }
    announce_balances(events, [(player_id, new_balance)]);
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

    /// A sender for `claim`, and a receiver to see what it broadcast
    fn events() -> (broadcast::Sender<GameEvent>, broadcast::Receiver<GameEvent>) {
        broadcast::channel(16)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_charges_the_deed_and_takes_the_plot(db: PgPool) {
        let deed = urban_deed(&db).await;
        let ada = player(&db, "ada", deed + 5).await;
        let lot = plot(&db, None, 0).await;
        let (tx, _rx) = events();

        assert_eq!(claim(&db, &tx, ada, lot, &Tunables::default()).await.unwrap(), 5);

        assert_eq!(owner(&db, lot).await, Some(Some(ada)));
        assert_eq!(balance(&db, ada).await, 5);
        assert_eq!(ledger(&db, ada).await, vec![(-deed, "deed_purchase".to_string())]);
    }

    /// The zone's deed price tunable sets what a claim charges, and the plot
    /// is assessed at it (D6.1)
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_charges_the_deed_at_its_tunable(db: PgPool) {
        let deed = urban_deed(&db).await;
        let ada = player(&db, "ada", deed * 2 + 5).await;
        let lot = plot(&db, None, 0).await;
        let (tx, _rx) = events();
        let mut tunables = Tunables::default();
        tunables.set("deed_price.urban", 2.0, |_, _| true).unwrap();

        assert_eq!(claim(&db, &tx, ada, lot, &tunables).await.unwrap(), 5);

        assert_eq!(ledger(&db, ada).await, vec![(-deed * 2, "deed_purchase".to_string())]);
        let assessed: i32 = sqlx::query_scalar("SELECT assessed_value FROM plots WHERE id = $1")
            .bind(lot)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(assessed as i64, deed * 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_you_cant_afford_takes_nothing(db: PgPool) {
        let deed = urban_deed(&db).await;
        let ada = player(&db, "ada", deed - 1).await;
        let lot = plot(&db, None, 0).await;

        let (tx, mut rx) = events();

        assert!(matches!(claim(&db, &tx, ada, lot, &Tunables::default()).await, Err(AppError::BadRequest(_))));
        assert!(rx.try_recv().is_err(), "a failed claim tells no one");

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

        let (tx, _rx) = events();

        let tunables = Tunables::default();
        let (a, b) = tokio::join!(claim(&db, &tx, ada, lot, &tunables), claim(&db, &tx, bram, lot, &tunables));

        assert!(a.is_ok() != b.is_ok(), "exactly one claim wins");
        let (winner, loser) = if a.is_ok() { (ada, bram) } else { (bram, ada) };
        assert_eq!(owner(&db, lot).await, Some(Some(winner)));
        assert_eq!(balance(&db, winner).await, 0);
        assert_eq!(balance(&db, loser).await, deed, "the loser kept their strands");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_is_broadcast_and_listed(db: PgPool) {
        let deed = urban_deed(&db).await;
        let ada = player(&db, "ada", deed).await;
        let lot = plot(&db, None, 0).await;
        let terrain_id = format!("test-{lot}");
        let (tx, mut rx) = events();
        assert!(claimed_plots(&db).await.unwrap().is_empty());

        claim(&db, &tx, ada, lot, &Tunables::default()).await.unwrap();

        let listed = claimed_plots(&db).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].terrain_id.as_str(), listed[0].owner_id), (terrain_id.as_str(), ada));
        match rx.try_recv() {
            Ok(GameEvent::PlotClaimed { terrain_id: sent, owner_id, claimed_at }) => {
                assert_eq!((sent, owner_id), (terrain_id, ada));
                assert_eq!(claimed_at, listed[0].claimed_at, "the broadcast and the list agree on when");
            }
            other => panic!("expected plot_claimed, got {other:?}"),
        }
        assert!(
            matches!(rx.try_recv(), Ok(GameEvent::CurrencyChanged { player_id, new_balance: 0 }) if player_id == ada),
            "then ada hears her balance (#148)"
        );
    }

    /// The wilds can't be owned (WD6): not a wilderness plot, not a plot whose
    /// row says rural but stands past the rural edge, and not one off the world.
    #[test]
    fn nothing_in_the_wilds_is_claimable() {
        use capitol_protocol::rules::{CENTER_X, CENTER_Y, RURAL_OUTER_RADIUS};
        assert!(claimable("urban", CENTER_X + 1200.0, CENTER_Y));
        assert!(claimable("rural", CENTER_X, CENTER_Y + RURAL_OUTER_RADIUS - 1.0));
        assert!(!claimable("wilderness", CENTER_X, CENTER_Y + RURAL_OUTER_RADIUS + 700.0));
        assert!(!claimable("rural", CENTER_X, CENTER_Y + RURAL_OUTER_RADIUS + 700.0));
        assert!(!claimable("wilderness", CENTER_X + 1200.0, CENTER_Y));
        assert!(!claimable("capitol", CENTER_X, CENTER_Y));
        assert!(!claimable("atlantis", CENTER_X + 1200.0, CENTER_Y));
    }

    /// A claim on a wilderness position is refused before it charges anything
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_claim_in_the_wilds_is_refused(db: PgPool) {
        let ada = player(&db, "ada", 1_000_000).await;
        let lot = plot(&db, None, 0).await;
        sqlx::query("UPDATE plots SET world_y = world_y + 5000 WHERE id = $1").bind(lot).execute(&db).await.unwrap();
        let (tx, mut rx) = events();

        assert!(matches!(claim(&db, &tx, ada, lot, &Tunables::default()).await, Err(AppError::BadRequest(m)) if m == NOT_CLAIMABLE));
        assert!(rx.try_recv().is_err());
        assert_eq!(owner(&db, lot).await, Some(None));
        assert_eq!(balance(&db, ada).await, 1_000_000);
    }

    /// Only lots are listed: a pre-B7 plot has nothing for a client to draw on
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_plot_with_no_lot_isnt_listed(db: PgPool) {
        let ada = player(&db, "ada", 0).await;
        let lot = plot(&db, Some(ada), 100).await;
        sqlx::query("UPDATE plots SET terrain_id = NULL WHERE id = $1").bind(lot).execute(&db).await.unwrap();

        assert!(claimed_plots(&db).await.unwrap().is_empty());
    }
}
