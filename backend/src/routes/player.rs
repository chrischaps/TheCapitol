use axum::{
    extract::State,
    routing::{get, post},
    Extension, Json, Router,
};
use uuid::Uuid;

use capitol_protocol::rest::{
    CurrencyResponse, Destination, InventoryResponse, MoveRequest, MoveResponse, MutesResponse, MyGuild,
    GrantRequest, GrantResponse, OwnStanding, PlayerMeResponse, SubscribeRequest, Subscription,
};
use capitol_protocol::rules::{in_world, Zone};

use crate::engine::Command;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{InventoryItem, Item, ItemType, ITEM_COLUMNS, ITEM_TYPE_COLUMNS};
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/me", get(get_me))
        .route("/move", post(move_player))
        .route("/inventory", get(get_inventory))
        .route("/currency", get(get_currency))
        .route("/subscription", post(mock_subscribe))
        .route("/grant", post(dev_grant))
        .route("/mutes", get(get_mutes))
        .route("/guild", get(get_guild))
        .route("/standing", get(get_standing))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// The people you've muted, newest first (S1.4)
#[utoipa::path(
    get,
    path = "/player/mutes",
    responses(
        (status = 200, description = "Who you've muted", body = MutesResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_mutes(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<MutesResponse>, AppError> {
    let player = super::players::player_of(&state.db, auth_user.account_id).await?;
    Ok(Json(MutesResponse { mutes: crate::mutes::list(&state.db, player).await? }))
}

/// Your standing (S3.1): what the city says of you, the number under it
/// that only you see, its trend over the week, and your last records
#[utoipa::path(
    get,
    path = "/player/standing",
    responses(
        (status = 200, description = "Your standing, in full", body = OwnStanding)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_standing(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<OwnStanding>, AppError> {
    let player = super::players::player_of(&state.db, auth_user.account_id).await?;
    let mut conn = state.db.acquire().await?;
    Ok(Json(crate::engine::standing::own(&mut conn, player, chrono::Utc::now()).await?))
}

/// Your guild, your role in it, and the invitations and applications that
/// name you (S4.1)
#[utoipa::path(
    get,
    path = "/player/guild",
    responses(
        (status = 200, description = "Your guild, if you're in one", body = MyGuild)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_guild(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<MyGuild>, AppError> {
    let player = super::players::player_of(&state.db, auth_user.account_id).await?;
    crate::engine::guilds::mine(&state.db, player, &super::guilds::lots(&state).await)
        .await
        .map(Json)
        .map_err(|e| AppError::Internal(e.message()))
}

/// Get the player's name, live position, zone and balance
///
/// While the player is in the game, the name and position come from the tick
/// loop's state, not the `players` row, which is only written every 10 s. The
/// balance always comes from the row: trades, the exchange and property tax
/// write it there, and the in-memory copy is never updated after login.
#[utoipa::path(
    get,
    path = "/player/me",
    responses(
        (status = 200, description = "The player, as the server sees them now", body = PlayerMeResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_me(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<PlayerMeResponse>, AppError> {
    let (player_id, name, x, y, strand_balance, citizen) =
        sqlx::query_as::<_, (Uuid, String, f64, f64, i64, bool)>(
            "SELECT p.id, p.name, p.position_x, p.position_y, p.strand_balance, a.kind = 'citizen'
             FROM players p JOIN accounts a ON a.id = p.account_id
             WHERE p.account_id = $1 LIMIT 1"
        )
        .bind(auth_user.account_id)
        .fetch_one(&state.db)
        .await?;

    let live = {
        let game = state.game.read().await;
        game.players.get(&player_id).map(|p| (p.name.clone(), p.x, p.y))
    };
    let online = live.is_some();
    let (name, x, y) = live.unwrap_or((name, x, y));
    let subscription = crate::subscriptions::of(&state.db, auth_user.account_id)
        .await?
        .unwrap_or_default();

    Ok(Json(PlayerMeResponse {
        player_id,
        name,
        x,
        y,
        zone: Zone::from_position(x, y).as_str().to_string(),
        strand_balance,
        online,
        subscription,
        citizen,
    }))
}

/// Start or extend your own subscription, with no payment (E4.1)
///
/// Stands in for a payment on a development server, for local runs and
/// `capitol-bot --subscribe`. Refused unless the server has
/// `MOCK_SUBSCRIPTIONS` on, as it does by default only in debug builds.
#[utoipa::path(
    post,
    path = "/player/subscription",
    request_body = SubscribeRequest,
    responses(
        (status = 200, description = "The subscription as it now stands", body = Subscription),
        (status = 400, description = "Days out of range"),
        (status = 403, description = "Mock subscriptions are off on this server")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn mock_subscribe(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<SubscribeRequest>,
) -> Result<Json<Subscription>, AppError> {
    if !state.config.mock_subscriptions {
        return Err(AppError::Forbidden("Mock subscriptions are off on this server".into()));
    }
    let subscription = crate::subscriptions::extend(&state.db, auth_user.account_id, req.days)
        .await?
        .ok_or_else(|| AppError::NotFound("Account not found".into()))?;
    tracing::info!("Account {} mock-subscribed for {} days", auth_user.account_id, req.days);
    Ok(Json(subscription))
}

/// Grant yourself Strands and goods, to fund a market maker (E3.8)
///
/// For local runs of `capitol-bot --market` and the sim's market makers. The
/// Strands go through the ledger as an `admin_adjustment`, the goods to your
/// market box. Refused unless the server has `DEV_GRANTS=on`, which no build
/// turns on by itself.
#[utoipa::path(
    post,
    path = "/player/grant",
    request_body = GrantRequest,
    responses(
        (status = 200, description = "The balance after, and the market box the goods went to", body = GrantResponse),
        (status = 400, description = "Out of bounds, an unknown item type, or the market box is full"),
        (status = 403, description = "Grants are off on this server")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn dev_grant(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<GrantRequest>,
) -> Result<Json<GrantResponse>, AppError> {
    use crate::engine::grants::{grant, GrantError};
    if !state.config.dev_grants {
        return Err(AppError::Forbidden("Grants are off on this server (DEV_GRANTS)".into()));
    }
    let player_id = sqlx::query_scalar::<_, Uuid>("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(auth_user.account_id)
        .fetch_one(&state.db)
        .await?;
    let granted = grant(&state.db, player_id, &req).await.map_err(|e| match e {
        GrantError::Db(e) => AppError::Internal(e),
        e => AppError::BadRequest(e.to_string()),
    })?;
    let events = state.game.read().await.event_sender.clone();
    crate::engine::currency::announce_balances(&events, [(player_id, granted.strands)]);
    let _ = events.send(crate::engine::GameEvent::InventoryUpdated {
        player_id,
        container_id: granted.market_box,
        slots: Vec::new(),
    });
    Ok(Json(GrantResponse {
        strands: granted.strands,
        market_box: granted.market_box,
    }))
}

/// Set player movement destination
#[utoipa::path(
    post,
    path = "/player/move",
    request_body = MoveRequest,
    responses(
        (status = 200, description = "Movement queued"),
        (status = 400, description = "Coordinates out of bounds")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn move_player(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<MoveRequest>,
) -> Result<Json<MoveResponse>, AppError> {
    // Only a destination on the world disc: the square's corners are off it (W1, #194)
    if !in_world(req.x, req.y) {
        return Err(AppError::BadRequest("Coordinates out of bounds".into()));
    }

    // Get player for this account
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Queue movement command
    let mut game = state.game.write().await;
    game.queue_command(Command::Move {
        player_id,
        dest_x: req.x,
        dest_y: req.y,
    });

    Ok(Json(MoveResponse {
        success: true,
        destination: Destination { x: req.x, y: req.y },
    }))
}

/// Get player's inventory items
#[utoipa::path(
    get,
    path = "/player/inventory",
    responses(
        (status = 200, description = "Player inventory", body = InventoryResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_inventory(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<InventoryResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Get items with their types
    let items: Vec<Item> = sqlx::query_as(&format!("SELECT {ITEM_COLUMNS} FROM items WHERE owner_id = $1"))
        .bind(player_id)
        .fetch_all(&state.db)
        .await?;

    let item_types: Vec<ItemType> = sqlx::query_as(&format!("SELECT {ITEM_TYPE_COLUMNS} FROM item_types"))
        .fetch_all(&state.db)
        .await?;
    let item_types: std::collections::HashMap<String, ItemType> =
        item_types.into_iter().map(|t| (t.id.clone(), t)).collect();

    let inventory_items: Vec<InventoryItem> = items
        .iter()
        .map(|item| item.to_inventory_item(item_types.get(&item.item_type)))
        .collect();

    Ok(Json(InventoryResponse { items: inventory_items }))
}

/// Get player's currency balance
#[utoipa::path(
    get,
    path = "/player/currency",
    responses(
        (status = 200, description = "Player currency balance", body = CurrencyResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_currency(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<CurrencyResponse>, AppError> {
    // Get player ID
    let player_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM players WHERE account_id = $1 LIMIT 1"
    )
    .bind(auth_user.account_id)
    .fetch_one(&state.db)
    .await?;

    // Get strand balance
    let balance = crate::engine::currency::get_balance(&state.db, player_id)
        .await
        .map_err(AppError::NotFound)?;

    Ok(Json(CurrencyResponse { strand_balance: balance }))
}
