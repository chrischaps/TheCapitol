//! `/bazaar` (E3.10, economy plan D6): the directory of shops, renting a
//! stall, stocking it, and buying from it.
//!
//! The directory works from anywhere and shows each shop's name, merchant and
//! kinds of goods, but not prices, quantities or grades. Those are seen at
//! the stall: seeing a shop, renting, stocking, unlisting and buying all need
//! you within `STALL_RANGE` of the stall's footprint. Your own shop
//! (`/bazaar/mine`) you can read, and give up (`/bazaar/mine/close`, #268),
//! from anywhere.
//!
//! Each write is one of `engine::bazaar`'s transactions. Once it commits, the
//! route tells whoever it moved Strands or goods for.

use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use capitol_protocol::rest::{
    BazaarDirectory, BazaarListing, BazaarPurchase, BazaarShop, BuyListing, ClosedStall, MyStallResponse, RentStall,
    StockListing,
};
use capitol_protocol::rules::bazaar::{at_the_stall, STALL_RANGE};
use capitol_protocol::terrain::Landmark;
use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::market::{player_of, position_of, tell_balance};
use crate::engine::bazaar::{self, BazaarError, Lapsed};
use crate::engine::{GameEvent, GameState};
use crate::engine::tutorial::{self, Did};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::LandmarkKind;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(get_directory))
        .route("/mine", get(my_stall))
        .route("/mine/close", post(close))
        .route("/mine/listings", post(stock))
        .route("/stalls/{id}", get(get_shop))
        .route("/stalls/{id}/rent", post(rent))
        .route("/listings/{id}", delete(unlist))
        .route("/listings/{id}/buy", post(buy))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Every stall, and the shop in it: names, merchants and kinds of goods,
/// without prices, quantities or grades
#[utoipa::path(
    get,
    path = "/bazaar",
    responses((status = 200, description = "The stalls, in order", body = BazaarDirectory)),
    security(("bearer_auth" = [])),
    tag = "Bazaar"
)]
pub async fn get_directory(State(state): State<AppState>) -> Result<Json<BazaarDirectory>, AppError> {
    let stalls: Vec<String> = stalls(&state).await.into_iter().map(|s| s.id).collect();
    Ok(Json(bazaar::directory(&state.db, &stalls, Utc::now()).await?))
}

/// Your stall, if you rent one, and goods from a closed stall waiting for
/// room in your market box. Works from anywhere.
#[utoipa::path(
    get,
    path = "/bazaar/mine",
    responses((status = 200, description = "Your shop", body = MyStallResponse)),
    security(("bearer_auth" = [])),
    tag = "Bazaar"
)]
pub async fn my_stall(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<MyStallResponse>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    Ok(Json(bazaar::mine(&state.db, player_id, Utc::now()).await?))
}

/// The shop at a stall, with every listing's price and units. You must be at
/// the stall.
#[utoipa::path(
    get,
    path = "/bazaar/stalls/{id}",
    params(("id" = String, Path, description = "The stall's landmark id, such as bazaar_stall_3")),
    responses(
        (status = 200, description = "The shop", body = BazaarShop),
        (status = 400, description = "Away from the stall"),
        (status = 404, description = "No such stall, or nobody rents it")
    ),
    security(("bearer_auth" = [])),
    tag = "Bazaar"
)]
pub async fn get_shop(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(stall_id): Path<String>,
) -> Result<Json<BazaarShop>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let stall = stall(&state, &stall_id).await?;
    let now = Utc::now();
    // Your own shop you see from anywhere
    if bazaar::stall_of(&state.db, player_id, now).await?.as_deref() != Some(stall_id.as_str()) {
        at(&state.game, &state.db, player_id, &stall, "see its prices").await?;
        // Whether or not anyone rents it, they stood there and asked (D10)
        tutorial::saw(&state.db, &events(&state).await, player_id, Did::LookedInAtStall).await;
    }
    bazaar::shop(&state.db, &stall_id, now)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::NotFound("Nobody rents that stall".to_string()))
}

/// Rent a free stall, 1 to 7 days ahead at 20 Strands a day, naming your
/// shop; or pay more days on your own (up to 7 from now) and rename it.
#[utoipa::path(
    post,
    path = "/bazaar/stalls/{id}/rent",
    params(("id" = String, Path, description = "The stall's landmark id")),
    request_body = RentStall,
    responses(
        (status = 200, description = "Your shop", body = BazaarShop),
        (status = 400, description = "Away from the stall, taken, you rent another, or not enough Strands"),
        (status = 404, description = "No such stall")
    ),
    security(("bearer_auth" = [])),
    tag = "Bazaar"
)]
pub async fn rent(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(stall_id): Path<String>,
    Json(req): Json<RentStall>,
) -> Result<Json<BazaarShop>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let stall = stall(&state, &stall_id).await?;
    at(&state.game, &state.db, player_id, &stall, "rent it").await?;
    let (shop, lapsed) = bazaar::rent(&state.db, player_id, &stall_id, req.days, req.name.as_deref(), Utc::now(), &state.tunables.get().fees)
        .await
        .map_err(refused)?;
    let events = events(&state).await;
    announce_lapsed(&events, &lapsed);
    tell_balance(&state.db, &events, player_id).await;
    Ok(Json(shop))
}

/// Give up your stall now, before its rent runs out: the shop closes, and its
/// goods go to your market box, or wait for room there. The days left aren't
/// refunded. Works from anywhere.
#[utoipa::path(
    post,
    path = "/bazaar/mine/close",
    responses(
        (status = 200, description = "Closed", body = ClosedStall),
        (status = 400, description = "You don't rent a stall")
    ),
    security(("bearer_auth" = [])),
    tag = "Bazaar"
)]
pub async fn close(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<ClosedStall>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let (closed, market_box) = bazaar::close(&state.db, player_id, Utc::now()).await.map_err(refused)?;
    let _ = events(&state).await.send(GameEvent::InventoryUpdated { player_id, container_id: market_box, slots: Vec::new() });
    Ok(Json(closed))
}

/// Stock your shop with units of a stack in your inventory, at a price per
/// unit. You must be at your stall. There's no fee.
#[utoipa::path(
    post,
    path = "/bazaar/mine/listings",
    request_body = StockListing,
    responses(
        (status = 200, description = "Listed", body = BazaarListing),
        (status = 400, description = "Away from your stall, no stall, out of bounds, or not your stack")
    ),
    security(("bearer_auth" = [])),
    tag = "Bazaar"
)]
pub async fn stock(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<StockListing>,
) -> Result<Json<BazaarListing>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let now = Utc::now();
    let stall_id = bazaar::stall_of(&state.db, player_id, now)
        .await?
        .ok_or_else(|| AppError::BadRequest(BazaarError::NotRenting.message()))?;
    let stall = stall(&state, &stall_id).await?;
    at(&state.game, &state.db, player_id, &stall, "stock it").await?;
    let source: Option<Uuid> = sqlx::query_scalar("SELECT container_id FROM items WHERE id = $1 AND owner_id = $2")
        .bind(req.item_id)
        .bind(player_id)
        .fetch_optional(&state.db)
        .await?
        .flatten();
    let listing = bazaar::stock(&state.db, player_id, &req, now).await.map_err(refused)?;
    if let Some(container_id) = source {
        let _ = events(&state).await.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
    }
    Ok(Json(listing))
}

/// Take one of your listings off your shop: its units come back to your
/// inventory, or your market box if it's full. You must be at your stall.
#[utoipa::path(
    delete,
    path = "/bazaar/listings/{id}",
    params(("id" = Uuid, Path, description = "The listing")),
    responses(
        (status = 200, description = "Unlisted"),
        (status = 400, description = "Away from your stall, or no room"),
        (status = 404, description = "No such listing of yours")
    ),
    security(("bearer_auth" = [])),
    tag = "Bazaar"
)]
pub async fn unlist(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(listing_id): Path<Uuid>,
) -> Result<Json<()>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let now = Utc::now();
    let stall_id = bazaar::stall_of_listing(&state.db, listing_id, now)
        .await?
        .ok_or_else(|| refused(BazaarError::NoListing))?;
    let stall = stall(&state, &stall_id).await?;
    at(&state.game, &state.db, player_id, &stall, "unlist from it").await?;
    let (_, into) = bazaar::unlist(&state.db, player_id, listing_id, now).await.map_err(refused)?;
    let _ = events(&state).await.send(GameEvent::InventoryUpdated { player_id, container_id: into, slots: Vec::new() });
    Ok(Json(()))
}

/// Buy units of a listing at its price: they go to your inventory, or your
/// market box if it's full. You must be at its stall.
#[utoipa::path(
    post,
    path = "/bazaar/listings/{id}/buy",
    params(("id" = Uuid, Path, description = "The listing")),
    request_body = BuyListing,
    responses(
        (status = 200, description = "Bought", body = BazaarPurchase),
        (status = 400, description = "Away from the stall, your own, not that many, no room, or not enough Strands"),
        (status = 404, description = "No such listing, or its shop is closed")
    ),
    security(("bearer_auth" = [])),
    tag = "Bazaar"
)]
pub async fn buy(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(listing_id): Path<Uuid>,
    Json(req): Json<BuyListing>,
) -> Result<Json<BazaarPurchase>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let now = Utc::now();
    let stall_id = bazaar::stall_of_listing(&state.db, listing_id, now)
        .await?
        .ok_or_else(|| refused(BazaarError::NoListing))?;
    let stall = stall(&state, &stall_id).await?;
    at(&state.game, &state.db, player_id, &stall, "buy from it").await?;
    let bought = bazaar::buy(&state.db, player_id, listing_id, &stall_id, &req, now).await.map_err(refused)?;

    let events = events(&state).await;
    let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id: bought.into, slots: Vec::new() });
    let notice = bought.notice.clone();
    let _ = events.send(GameEvent::BazaarNotice {
        player_id: notice.player_id,
        notice_id: notice.notice_id,
        message: notice.message,
    });
    tell_balance(&state.db, &events, player_id).await;
    tell_balance(&state.db, &events, bought.merchant).await;
    Ok(Json(bought.purchase))
}

/// Send the notices and box updates of stalls that closed.
pub(crate) fn announce_lapsed(events: &broadcast::Sender<GameEvent>, lapsed: &[Lapsed]) {
    for l in lapsed {
        let _ = events.send(GameEvent::BazaarNotice {
            player_id: l.notice.player_id,
            notice_id: l.notice.notice_id,
            message: l.notice.message.clone(),
        });
        let _ = events.send(GameEvent::InventoryUpdated {
            player_id: l.player_id,
            container_id: l.market_box,
            slots: Vec::new(),
        });
    }
}

/// The world's stalls, in order.
async fn stalls(state: &AppState) -> Vec<Landmark> {
    let game = state.game.read().await;
    game.terrain.landmarks.iter().filter(|l| l.kind == LandmarkKind::Stall).cloned().collect()
}

/// The stall `stall_id`, or 404.
async fn stall(state: &AppState, stall_id: &str) -> Result<Landmark, AppError> {
    let game = state.game.read().await;
    game.terrain
        .landmarks
        .iter()
        .find(|l| l.kind == LandmarkKind::Stall && l.id == stall_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound("No such stall".to_string()))
}

/// Refuses `doing` unless the player stands within `STALL_RANGE` of `stall`.
async fn at(
    game: &RwLock<GameState>,
    db: &PgPool,
    player_id: Uuid,
    stall: &Landmark,
    doing: &str,
) -> Result<(), AppError> {
    let (x, y) = position_of(game, db, player_id).await?;
    if at_the_stall(stall, x, y) {
        return Ok(());
    }
    Err(AppError::BadRequest(format!(
        "You're too far from the stall to {doing} ({:.0} units; it needs {STALL_RANGE:.0})",
        stall.distance_to(x, y)
    )))
}

/// What the bazaar refused, as the API says it.
fn refused(e: BazaarError) -> AppError {
    match e {
        BazaarError::NoListing => AppError::NotFound(e.message()),
        BazaarError::Db(e) => AppError::Database(e),
        e => AppError::BadRequest(e.message()),
    }
}

async fn events(state: &AppState) -> broadcast::Sender<GameEvent> {
    state.game.read().await.event_sender.clone()
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test bazaar -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::market::tests::player;
    use crate::models::{TerrainConfig, WorldGenParams};
    use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};

    /// From the spawn, a merchant is refused with a reason; in the lane in
    /// front of the stall they're allowed; and the stall across the lane is
    /// in reach from its crown, but not the far end of the bazaar.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn stocking_and_buying_happen_at_the_stall(db: PgPool) {
        let terrain = TerrainConfig::get_default();
        let stall_of = |id: &str| terrain.landmarks.iter().find(|l| l.id == id).cloned().expect(id);
        // A stall on the lane, the one across the lane from it, and one far
        // round the market street (E3.12)
        let lane_y = terrain.roads.iter().find(|r| r.id == "bazaar_lane").expect("the lane").points[0].1;
        let front = stall_of("bazaar_stall_2");
        let crown = (front.door.0, lane_y);
        let across = terrain
            .landmarks
            .iter()
            .filter(|l| l.kind == LandmarkKind::Stall && (l.door.1 - lane_y) * (front.door.1 - lane_y) < 0.0)
            .min_by(|a, b| a.distance_to(crown.0, crown.1).total_cmp(&b.distance_to(crown.0, crown.1)))
            .cloned()
            .expect("a stall across the lane");
        let far = stall_of("bazaar_stall_30");
        let game = RwLock::new(GameState::with_terrain(WorldGenParams::default(), terrain.clone()));
        let ada = player(&db, "ada", 100).await;
        let save = |(x, y): (f64, f64)| {
            sqlx::query("UPDATE players SET position_x = $1, position_y = $2 WHERE id = $3")
                .bind(x)
                .bind(y)
                .bind(ada.id)
                .execute(&db)
        };

        save((SPAWN_X, SPAWN_Y)).await.unwrap();
        let refused = at(&game, &db, ada.id, &front, "stock it").await;
        assert!(
            matches!(&refused, Err(AppError::BadRequest(m)) if m.contains("too far from the stall to stock it")),
            "{refused:?}"
        );

        // The lane's crown, in front of stall 2
        save(crown).await.unwrap();
        assert!(at(&game, &db, ada.id, &front, "stock it").await.is_ok());
        assert!(at(&game, &db, ada.id, &across, "buy from it").await.is_ok());
        assert!(at(&game, &db, ada.id, &far, "buy from it").await.is_err());
    }
}
