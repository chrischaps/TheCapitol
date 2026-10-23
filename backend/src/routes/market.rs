//! `/market` (E3.3, economy plan D7): placing and cancelling orders, an item's
//! book, your orders, price history, the summary of every item with activity,
//! the newest sells across every item (#493), and the market box.
//!
//! Placing and cancelling are `engine::market`'s, one transaction each. Once
//! one commits, the route tells the book (`Market::placed`, `cancelled`) and
//! publishes the item's depth to its channel. Depth is served from what was
//! last published, so `GET /market/orders` and a `subscribe_market` snapshot
//! agree with the diffs by `seq`. The best bid and ask are read from there
//! too, in memory: nothing else reads them, so they aren't in Redis.
//!
//! Placing an order and collecting from the market box happen at the Exchange
//! hall (D6, E3.4): from further than `HALL_RANGE` off its footprint they're
//! refused. Browsing, history, the box's contents and cancelling work from
//! anywhere.

use axum::{
    extract::{Path, Query, State},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use capitol_protocol::rest::{
    Candle, CollectRequest, CollectResponse, ContainerResponse, MarketDepthQuery, MarketDepthResponse,
    MarketHistoryQuery, MarketHistoryResponse, MarketItemSummary, MarketListingsResponse, MarketOrder,
    MarketSummaryResponse, MarketTrade, MyFill, MyOrdersQuery, PlaceOrder,
};
use capitol_protocol::rules::market::{at_the_hall, grade_floor, OrderSide, HALL_RANGE};
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use std::collections::{BTreeMap, HashMap};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::engine::currency::announce_balances;
use crate::engine::{GameEvent, GameState};
use crate::engine::market::{self, MarketError};
use crate::engine::tutorial::{self, Did};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::models::{Item, ItemType, LandmarkKind, SlotItem, ITEM_COLUMNS, ITEM_TYPE_COLUMNS};
use crate::state::AppState;

/// The most candles one `/market/history` answers with: the latest.
const MAX_CANDLES: i64 = 500;
/// How far back `/market/history` looks without a `since`.
const DEFAULT_HISTORY_DAYS: i64 = 7;
/// How recently an item must have traded to be in `/market/summary`, if it
/// isn't on the book.
const SUMMARY_DAYS: i64 = 7;
/// How many closed orders `/market/orders/mine?closed=true` adds.
const CLOSED_ORDERS: i64 = 100;
/// How many of the latest trades `/market/history` lists one by one.
const RECENT_TRADES: i64 = 50;
/// And how many of your own fills, for the chart to mark.
const MY_FILLS: i64 = 200;
/// How many of the newest sells `/market/listings` lists (#493).
const LISTINGS: usize = 100;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/orders", post(place_order).get(get_depth))
        .route("/orders/mine", get(my_orders))
        .route("/orders/{id}", delete(cancel_order))
        .route("/history", get(history))
        .route("/summary", get(summary))
        .route("/listings", get(listings))
        .route("/box", get(get_box))
        .route("/box/collect", post(collect))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Place an order: its escrow and the listing fee (1% by default, a live
/// tunable) are taken, and it matches from
/// the next cycle
#[utoipa::path(
    post,
    path = "/market/orders",
    request_body = PlaceOrder,
    responses(
        (status = 200, description = "On the book", body = MarketOrder),
        (status = 400, description = "Away from the Exchange hall, out of bounds, not your stack, or not enough Strands")
    ),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn place_order(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<PlaceOrder>,
) -> Result<Json<MarketOrder>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    at_the_exchange(&state.game, &state.db, player_id, "place an order").await?;
    // Where a sell's units come from, to say that container changed
    let source: Option<Uuid> = match (req.side, req.item_id) {
        (OrderSide::Sell, Some(item_id)) => {
            sqlx::query_scalar("SELECT container_id FROM items WHERE id = $1 AND owner_id = $2")
                .bind(item_id)
                .bind(player_id)
                .fetch_optional(&state.db)
                .await?
                .flatten()
        }
        _ => None,
    };
    let order = market::place(&state.db, player_id, &req, &state.tunables.get().fees).await.map_err(refused)?;
    state.market.placed(&order);

    let events = events(&state).await;
    state.market.publish(&events);
    let _ = events.send(GameEvent::OrderPlaced { player_id, order: order.clone() });
    if let Some(container_id) = source {
        let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
    }
    tell_balance(&state.db, &events, player_id).await;
    tutorial::saw(&state.db, &events, player_id, Did::Ordered(req.side)).await;
    Ok(Json(order))
}

/// Cancel one of your open orders: what's left of a sell goes to your market
/// box, of a buy to your balance. The fee isn't refunded.
#[utoipa::path(
    delete,
    path = "/market/orders/{id}",
    params(("id" = Uuid, Path, description = "The order")),
    responses(
        (status = 200, description = "Cancelled, as it stands now", body = MarketOrder),
        (status = 400, description = "Not open, or your market box is full"),
        (status = 404, description = "No such order of yours")
    ),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn cancel_order(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(order_id): Path<Uuid>,
) -> Result<Json<MarketOrder>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let cancelled = market::cancel(&state.db, player_id, order_id).await.map_err(refused)?;
    state.market.cancelled(order_id);

    let events = events(&state).await;
    state.market.publish(&events);
    let _ = events.send(GameEvent::OrderClosed { player_id, order: cancelled.order.clone() });
    if let Some(container_id) = cancelled.market_box {
        let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
    }
    if cancelled.refunded > 0 {
        tell_balance(&state.db, &events, player_id).await;
    }
    Ok(Json(cancelled.order))
}

/// An item's book, by price and grade: bids best first, then asks
#[utoipa::path(
    get,
    path = "/market/orders",
    params(MarketDepthQuery),
    responses((status = 200, description = "The book as last published", body = MarketDepthResponse)),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn get_depth(
    State(state): State<AppState>,
    Query(query): Query<MarketDepthQuery>,
) -> Result<Json<MarketDepthResponse>, AppError> {
    let mut depth = state.market.depth(&query.item);
    if let Some(side) = query.side {
        depth.levels.retain(|l| l.side == side);
    }
    Ok(Json(MarketDepthResponse {
        item_type: query.item,
        seq: depth.seq,
        levels: depth.levels,
    }))
}

/// Your orders: the open ones, newest first, and with `closed` the latest
/// closed ones too
#[utoipa::path(
    get,
    path = "/market/orders/mine",
    params(MyOrdersQuery),
    responses((status = 200, description = "Your orders", body = Vec<MarketOrder>)),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn my_orders(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<MyOrdersQuery>,
) -> Result<Json<Vec<MarketOrder>>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    Ok(Json(orders_of(&state.db, player_id, query.closed).await?))
}

/// `player_id`'s open orders, newest first, then with `closed` their latest
/// closed ones.
pub async fn orders_of(db: &PgPool, player_id: Uuid, closed: bool) -> Result<Vec<MarketOrder>, AppError> {
    let mut ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM market_orders WHERE player_id = $1 AND status = 'open' ORDER BY created_at DESC, id",
    )
    .bind(player_id)
    .fetch_all(db)
    .await?;
    if closed {
        let closed: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM market_orders WHERE player_id = $1 AND status <> 'open'
             ORDER BY created_at DESC, id LIMIT $2",
        )
        .bind(player_id)
        .bind(CLOSED_ORDERS)
        .fetch_all(db)
        .await?;
        ids.extend(closed);
    }
    let mut conn = db.acquire().await?;
    let mut orders = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(order) = market::load(&mut conn, id).await? {
            orders.push(order);
        }
    }
    Ok(orders)
}

/// An item's trades in buckets (OHLC and volume), oldest first, then the
/// latest of them one by one and your own fills among them
#[utoipa::path(
    get,
    path = "/market/history",
    params(MarketHistoryQuery),
    responses(
        (status = 200, description = "The candles, empty buckets left out", body = MarketHistoryResponse),
        (status = 400, description = "An unknown bucket or grade")
    ),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn history(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<MarketHistoryQuery>,
) -> Result<Json<MarketHistoryResponse>, AppError> {
    let bucket = query.bucket.as_deref().unwrap_or("1h");
    let bucket_secs = bucket_secs(bucket)
        .ok_or_else(|| AppError::BadRequest(format!("No bucket {bucket:?}: use 1m, 5m, 15m, 1h, 4h or 1d")))?;
    if let Some(grade) = &query.grade {
        if grade_floor(grade).is_none() {
            return Err(AppError::BadRequest(format!("No grade {grade:?}: use A to F")));
        }
    }
    let since = query.since.unwrap_or_else(|| Utc::now() - Duration::days(DEFAULT_HISTORY_DAYS));
    let grade = query.grade.as_deref();
    let candles = candles(&state.db, &query.item, bucket_secs, since, grade).await?;
    let recent = recent_trades(&state.db, &query.item, since, grade).await?;
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let mine = fills_of(&state.db, player_id, &query.item, since, grade).await?;
    Ok(Json(MarketHistoryResponse {
        item_type: query.item,
        bucket_secs,
        candles,
        recent,
        mine,
    }))
}

type TradeRow = (Uuid, Uuid, Uuid, String, String, i32, i32, i64, i32, DateTime<Utc>);

const TRADE_COLUMNS: &str =
    "t.id, t.buy_order_id, t.sell_order_id, t.item_type, t.grade, t.quality, t.min_quality, t.price, t.quantity, t.at";

fn trade_of(row: TradeRow) -> MarketTrade {
    let (id, buy_order_id, sell_order_id, item_type, grade, quality, min_quality, price, quantity, at) = row;
    MarketTrade {
        id,
        buy_order_id,
        sell_order_id,
        item_type,
        grade,
        quality,
        min_quality,
        price,
        quantity,
        at,
    }
}

/// `item_type`'s latest [`RECENT_TRADES`] trades since `since`, newest first.
pub async fn recent_trades(
    db: &PgPool,
    item_type: &str,
    since: DateTime<Utc>,
    grade: Option<&str>,
) -> Result<Vec<MarketTrade>, sqlx::Error> {
    let rows: Vec<TradeRow> = sqlx::query_as(&format!(
        "SELECT {TRADE_COLUMNS} FROM market_trades t
         WHERE t.item_type = $1 AND t.at >= $2 AND ($3::TEXT IS NULL OR t.grade = $3)
         ORDER BY t.at DESC, t.id DESC
         LIMIT $4"
    ))
    .bind(item_type)
    .bind(since)
    .bind(grade)
    .bind(RECENT_TRADES)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(trade_of).collect())
}

/// `player_id`'s latest [`MY_FILLS`] fills in `item_type` since `since`,
/// newest first, each with the side that was theirs.
pub async fn fills_of(
    db: &PgPool,
    player_id: Uuid,
    item_type: &str,
    since: DateTime<Utc>,
    grade: Option<&str>,
) -> Result<Vec<MyFill>, sqlx::Error> {
    let rows: Vec<(String, Uuid, Uuid, Uuid, String, String, i32, i32, i64, i32, DateTime<Utc>)> =
        sqlx::query_as(&format!(
            "SELECT o.side, {TRADE_COLUMNS} FROM market_trades t
             JOIN market_orders o ON o.id IN (t.buy_order_id, t.sell_order_id) AND o.player_id = $1
             WHERE t.item_type = $2 AND t.at >= $3 AND ($4::TEXT IS NULL OR t.grade = $4)
             ORDER BY t.at DESC, t.id DESC
             LIMIT $5"
        ))
        .bind(player_id)
        .bind(item_type)
        .bind(since)
        .bind(grade)
        .bind(MY_FILLS)
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(side, id, buy, sell, item, grade, quality, min_quality, price, quantity, at)| MyFill {
            side: if side == "buy" { OrderSide::Buy } else { OrderSide::Sell },
            trade: trade_of((id, buy, sell, item, grade, quality, min_quality, price, quantity, at)),
        })
        .collect())
}

/// A bucket's length in seconds.
fn bucket_secs(bucket: &str) -> Option<i64> {
    Some(match bucket {
        "1m" => 60,
        "5m" => 5 * 60,
        "15m" => 15 * 60,
        "1h" => 60 * 60,
        "4h" => 4 * 60 * 60,
        "1d" => 24 * 60 * 60,
        _ => return None,
    })
}

/// `item_type`'s trades since `since`, in buckets of `bucket_secs` from the
/// epoch, oldest first: the latest [`MAX_CANDLES`].
pub async fn candles(
    db: &PgPool,
    item_type: &str,
    bucket_secs: i64,
    since: DateTime<Utc>,
    grade: Option<&str>,
) -> Result<Vec<Candle>, sqlx::Error> {
    type Row = (DateTime<Utc>, i64, i64, i64, i64, i64, i64);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT date_bin(make_interval(secs => $2), at, TIMESTAMPTZ '1970-01-01 00:00:00+00') AS bucket,
                (array_agg(price ORDER BY at, id))[1],
                MAX(price),
                MIN(price),
                (array_agg(price ORDER BY at DESC, id DESC))[1],
                SUM(quantity)::BIGINT,
                COUNT(*)
         FROM market_trades
         WHERE item_type = $1 AND at >= $3 AND ($4::TEXT IS NULL OR grade = $4)
         GROUP BY bucket
         ORDER BY bucket DESC
         LIMIT $5",
    )
    .bind(item_type)
    .bind(bucket_secs as f64)
    .bind(since)
    .bind(grade)
    .bind(MAX_CANDLES)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .rev()
        .map(|(at, open, high, low, close, volume, trades)| Candle {
            at,
            open,
            high,
            low,
            close,
            volume,
            trades: trades as i32,
        })
        .collect())
}

/// Every item with activity: on the book now, or traded in the last 7 days
#[utoipa::path(
    get,
    path = "/market/summary",
    responses((status = 200, description = "By item type", body = MarketSummaryResponse)),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn summary(State(state): State<AppState>) -> Result<Json<MarketSummaryResponse>, AppError> {
    type Row = (String, Option<i64>, Option<i64>, i64);
    let traded: Vec<Row> = sqlx::query_as(
        "WITH items AS (
             SELECT DISTINCT item_type FROM market_trades WHERE at > NOW() - make_interval(days => $1)
         )
         SELECT i.item_type,
                (SELECT price FROM market_trades t WHERE t.item_type = i.item_type
                 ORDER BY at DESC, id DESC LIMIT 1),
                COALESCE(
                    (SELECT price FROM market_trades t
                     WHERE t.item_type = i.item_type AND at <= NOW() - INTERVAL '1 day'
                     ORDER BY at DESC, id DESC LIMIT 1),
                    (SELECT price FROM market_trades t
                     WHERE t.item_type = i.item_type AND at > NOW() - INTERVAL '1 day'
                     ORDER BY at, id LIMIT 1)),
                (SELECT COALESCE(SUM(quantity), 0)::BIGINT FROM market_trades t
                 WHERE t.item_type = i.item_type AND at > NOW() - INTERVAL '1 day')
         FROM items i",
    )
    .bind(SUMMARY_DAYS as i32)
    .fetch_all(&state.db)
    .await?;

    let mut items: BTreeMap<String, MarketItemSummary> = BTreeMap::new();
    for (item_type, last_price, day_ago, volume_24h) in traded {
        let change_24h = last_price.zip(day_ago).map(|(last, then)| last - then);
        items.insert(
            item_type.clone(),
            MarketItemSummary {
                item_type,
                last_price,
                change_24h,
                volume_24h,
                best_bid: None,
                best_ask: None,
            },
        );
    }
    for (item_type, best_bid, best_ask) in state.market.best_prices() {
        let item = items.entry(item_type.clone()).or_insert_with(|| MarketItemSummary {
            item_type,
            last_price: None,
            change_24h: None,
            volume_24h: 0,
            best_bid: None,
            best_ask: None,
        });
        item.best_bid = best_bid;
        item.best_ask = best_ask;
    }
    Ok(Json(MarketSummaryResponse {
        items: items.into_values().collect(),
    }))
}

/// What's for sale across every item: the newest sells on the book, newest
/// first (#493)
#[utoipa::path(
    get,
    path = "/market/listings",
    responses((status = 200, description = "The latest 100, from the book", body = MarketListingsResponse)),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn listings(State(state): State<AppState>) -> Json<MarketListingsResponse> {
    Json(MarketListingsResponse {
        listings: state.market.listings(LISTINGS),
    })
}

/// Your market box: what you bought, and sells that came back
#[utoipa::path(
    get,
    path = "/market/box",
    responses((status = 200, description = "The box and what's in it", body = ContainerResponse)),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn get_box(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<Json<ContainerResponse>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    let mut tx = state.db.begin().await?;
    let box_id = market::market_box(&mut tx, player_id).await?;
    tx.commit().await?;
    Ok(Json(container(&state.db, box_id).await?))
}

/// The container `id` and its stacks.
pub(crate) async fn container(db: &PgPool, id: Uuid) -> Result<ContainerResponse, AppError> {
    let (container_type, slot_count, layout_columns): (String, i32, i32) = sqlx::query_as(
        "SELECT c.container_type, t.slot_count, t.layout_columns
         FROM containers c JOIN container_types t ON t.id = c.container_type WHERE c.id = $1",
    )
    .bind(id)
    .fetch_one(db)
    .await?;
    let items: Vec<Item> = sqlx::query_as(&format!(
        "SELECT {ITEM_COLUMNS} FROM items WHERE container_id = $1 ORDER BY slot_index"
    ))
    .bind(id)
    .fetch_all(db)
    .await?;
    let kinds: Vec<String> = items.iter().map(|i| i.item_type.clone()).collect();
    let types: Vec<ItemType> =
        sqlx::query_as(&format!("SELECT {ITEM_TYPE_COLUMNS} FROM item_types WHERE id = ANY($1)"))
            .bind(&kinds)
            .fetch_all(db)
            .await?;
    let types: HashMap<String, ItemType> = types.into_iter().map(|t| (t.id.clone(), t)).collect();
    let slots: Vec<SlotItem> = items
        .into_iter()
        .filter_map(|item| item.to_slot_item(types.get(&item.item_type)))
        .collect();
    Ok(ContainerResponse {
        id,
        container_type,
        slot_count,
        layout_columns,
        slots,
    })
}

/// Take stacks from your market box to your inventory, in slot order, until
/// the inventory is full. A stack joins one of its type there if it can.
#[utoipa::path(
    post,
    path = "/market/box/collect",
    request_body = CollectRequest,
    responses(
        (status = 200, description = "What moved, and what's left", body = CollectResponse),
        (status = 400, description = "Away from the Exchange hall, or a stack named isn't in your market box")
    ),
    security(("bearer_auth" = [])),
    tag = "Market"
)]
pub async fn collect(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CollectRequest>,
) -> Result<Json<CollectResponse>, AppError> {
    let player_id = player_of(&state.db, auth_user.account_id).await?;
    at_the_exchange(&state.game, &state.db, player_id, "collect from the market box").await?;
    let (response, box_id, inventory) = collect_from_box(&state.db, player_id, req.item_ids.as_deref()).await?;
    if !response.collected.is_empty() {
        let events = events(&state).await;
        for container_id in [box_id, inventory] {
            let _ = events.send(GameEvent::InventoryUpdated { player_id, container_id, slots: Vec::new() });
        }
        tutorial::saw(&state.db, &events, player_id, Did::Collected).await;
    }
    Ok(Json(response))
}

/// Move `player_id`'s market box stacks (or just `only`) to their inventory,
/// in one transaction. Returns what moved, the box and the inventory.
pub async fn collect_from_box(
    db: &PgPool,
    player_id: Uuid,
    only: Option<&[Uuid]>,
) -> Result<(CollectResponse, Uuid, Uuid), AppError> {
    let mut tx = db.begin().await?;
    let box_id = market::market_box(&mut tx, player_id).await?;
    let (response, inventory) = collect_from(&mut tx, player_id, box_id, "market box", only).await?;
    tx.commit().await?;
    Ok((response, box_id, inventory))
}

/// Move the stacks of `player_id`'s container `from` (or just `only`), called
/// `name` in a refusal, to their inventory in slot order, until it's full: the
/// market box's collect, and the impound's (#141). Returns what moved and the
/// inventory.
pub(crate) async fn collect_from(
    tx: &mut sqlx::PgConnection,
    player_id: Uuid,
    from: Uuid,
    name: &str,
    only: Option<&[Uuid]>,
) -> Result<(CollectResponse, Uuid), AppError> {
    // Not FOR UPDATE: a sell or a move from a stack here checks the inventory's
    // key while holding the stack this stows onto (#333)
    let inventory: Uuid = sqlx::query_scalar(
        "SELECT id FROM containers WHERE owner_id = $1 AND container_type = 'player_inventory' LIMIT 1 FOR NO KEY UPDATE",
    )
    .bind(player_id)
    .fetch_one(&mut *tx)
    .await?;
    let mut stacks: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM items WHERE container_id = $1 ORDER BY slot_index NULLS LAST, id FOR UPDATE",
    )
    .bind(from)
    .fetch_all(&mut *tx)
    .await?;
    if let Some(only) = only {
        if let Some(missing) = only.iter().find(|id| !stacks.contains(id)) {
            return Err(AppError::BadRequest(format!("{missing} isn't in your {name}")));
        }
        stacks.retain(|id| only.contains(id));
    }

    let mut collected = Vec::new();
    for stack in stacks {
        if market::stow(tx, player_id, stack, inventory).await?.is_none() {
            break;
        }
        collected.push(stack);
    }
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE container_id = $1")
        .bind(from)
        .fetch_one(&mut *tx)
        .await?;
    Ok((
        CollectResponse {
            collected,
            remaining: remaining as i32,
        },
        inventory,
    ))
}

/// Refuses `doing` unless the player stands within `HALL_RANGE` of the
/// Exchange hall: where they are now if they're online, else their saved
/// position. A world without a hall (an admin's custom one) refuses nothing.
pub(crate) async fn at_the_exchange(
    game: &RwLock<GameState>,
    db: &PgPool,
    player_id: Uuid,
    doing: &str,
) -> Result<(), AppError> {
    let hall = game.read().await.terrain.landmark(LandmarkKind::ExchangeHall).cloned();
    let Some(hall) = hall else { return Ok(()) };
    let (x, y) = position_of(game, db, player_id).await?;
    if at_the_hall(&hall, x, y) {
        return Ok(());
    }
    Err(AppError::BadRequest(format!(
        "You're too far from the Exchange hall to {doing} ({:.0} units; it needs {HALL_RANGE:.0})",
        hall.distance_to(x, y)
    )))
}

/// Where the player stands: now if they're online, else their saved position.
pub(crate) async fn position_of(game: &RwLock<GameState>, db: &PgPool, player_id: Uuid) -> Result<(f64, f64), AppError> {
    let live = game.read().await.players.get(&player_id).map(|p| (p.x, p.y));
    match live {
        Some(at) => Ok(at),
        None => Ok(sqlx::query_as("SELECT position_x, position_y FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(db)
            .await?),
    }
}

/// What the market refused, as the API says it.
fn refused(e: MarketError) -> AppError {
    match e {
        MarketError::NotFound => AppError::NotFound(e.message()),
        MarketError::Db(e) => AppError::Database(e),
        e => AppError::BadRequest(e.message()),
    }
}

async fn events(state: &AppState) -> broadcast::Sender<GameEvent> {
    state.game.read().await.event_sender.clone()
}

/// `currency_changed` to the player, so a client needn't refetch it
pub(crate) async fn tell_balance(db: &PgPool, events: &broadcast::Sender<GameEvent>, player_id: Uuid) {
    let balance: Result<i64, _> = sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1")
        .bind(player_id)
        .fetch_one(db)
        .await;
    if let Ok(balance) = balance {
        announce_balances(events, [(player_id, balance)]);
    }
}

pub(crate) async fn player_of(db: &PgPool, account_id: Uuid) -> Result<Uuid, AppError> {
    Ok(sqlx::query_scalar("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account_id)
        .fetch_one(db)
        .await?)
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test market -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::market::tests::{box_of, buy, player, sell};
    use crate::engine::market_matcher::{run_cycle, Market};
    use crate::engine::units::test_units::{all_in, stack_of};
    use capitol_protocol::rules::market::OrderStatus;
    use capitol_protocol::rules::stacking::Units;

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_trade_makes_a_candle_and_the_buyer_collects_it(db: PgPool) {
        let (events, _heard) = broadcast::channel(256);
        let market = Market::default();
        let ada = player(&db, "ada", 100).await;
        let bea = player(&db, "bea", 1000).await;
        let stack = stack_of(&db, ada.id, ada.inventory, 0, "rope", &Units::uniform(10, 80)).await;
        for order in [
            market::place(&db, ada.id, &sell("rope", stack, 10, 6, None), &capitol_protocol::tunables::Fees::default()).await.unwrap(),
            market::place(&db, bea.id, &buy("rope", 12, 4, None), &capitol_protocol::tunables::Fees::default()).await.unwrap(),
            market::place(&db, bea.id, &buy("rope", 11, 2, None), &capitol_protocol::tunables::Fees::default()).await.unwrap(),
        ] {
            market.placed(&order);
        }
        run_cycle(&market, &db, &events, Utc::now()).await;

        // Both bids filled at the ask's 10: one candle of 6, all at 10
        let hourly = candles(&db, "rope", 3600, Utc::now() - Duration::hours(2), None).await.unwrap();
        assert_eq!(hourly.len(), 1);
        let c = &hourly[0];
        assert_eq!((c.open, c.high, c.low, c.close, c.volume, c.trades), (10, 10, 10, 10, 6, 2));
        assert!(candles(&db, "rope", 3600, Utc::now() - Duration::hours(2), Some("A")).await.unwrap().is_empty());
        assert_eq!(candles(&db, "rope", 60, Utc::now() - Duration::hours(2), Some("B")).await.unwrap().len(), 1);

        // Both fills one by one, newest first; each is Bea's buy and Ada's sell
        let since = Utc::now() - Duration::hours(2);
        let recent = recent_trades(&db, "rope", since, None).await.unwrap();
        assert_eq!(recent.iter().map(|t| t.quantity).sum::<i32>(), 6);
        assert!(recent.windows(2).all(|w| w[0].at >= w[1].at));
        let bea_fills = fills_of(&db, bea.id, "rope", since, None).await.unwrap();
        assert_eq!(bea_fills.iter().map(|f| &f.trade).collect::<Vec<_>>(), recent.iter().collect::<Vec<_>>());
        assert!(bea_fills.iter().all(|f| f.side == OrderSide::Buy));
        let ada_fills = fills_of(&db, ada.id, "rope", since, None).await.unwrap();
        assert!(ada_fills.len() == 2 && ada_fills.iter().all(|f| f.side == OrderSide::Sell));
        assert!(fills_of(&db, ada.id, "rope", since, Some("A")).await.unwrap().is_empty());

        // Bea's orders are filled; Ada's sell is all gone too
        assert!(orders_of(&db, bea.id, false).await.unwrap().is_empty());
        let closed = orders_of(&db, bea.id, true).await.unwrap();
        assert_eq!(closed.len(), 2);
        assert!(closed.iter().all(|o| o.status == OrderStatus::Filled));

        // The rope waits in her box until she collects it into her inventory
        let bea_box = box_of(&db, bea.id).await;
        assert_eq!(all_in(&db, bea_box, "rope").await, Units::uniform(6, 80));
        let (collected, box_id, inventory) = collect_from_box(&db, bea.id, None).await.unwrap();
        assert_eq!((box_id, inventory), (bea_box, bea.inventory));
        assert_eq!(collected.remaining, 0);
        assert!(!collected.collected.is_empty());
        assert_eq!(all_in(&db, bea_box, "rope").await, Units::new());
        assert_eq!(all_in(&db, bea.inventory, "rope").await, Units::uniform(6, 80));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn collect_stops_when_the_inventory_is_full_and_refuses_a_stranger(db: PgPool) {
        let ada = player(&db, "ada", 100).await;
        let ada_box = box_of(&db, ada.id).await;
        // Two stacks of tools, which don't stack, in the box
        let slots: i32 = sqlx::query_scalar("SELECT slot_count FROM container_types WHERE id = 'player_inventory'")
            .fetch_one(&db)
            .await
            .unwrap();
        let tool: String = sqlx::query_scalar("SELECT id FROM item_types WHERE NOT stackable LIMIT 1")
            .fetch_one(&db)
            .await
            .unwrap();
        let first = stack_of(&db, ada.id, ada_box, 0, &tool, &Units::uniform(1, 60)).await;
        let second = stack_of(&db, ada.id, ada_box, 1, &tool, &Units::uniform(1, 70)).await;
        // The inventory has room for one
        for slot in 0..slots - 1 {
            stack_of(&db, ada.id, ada.inventory, slot, &tool, &Units::uniform(1, 50)).await;
        }
        let (collected, ..) = collect_from_box(&db, ada.id, None).await.unwrap();
        assert_eq!(collected.collected, vec![first]);
        assert_eq!(collected.remaining, 1);

        let stranger = Uuid::new_v4();
        assert!(matches!(collect_from_box(&db, ada.id, Some(&[stranger])).await, Err(AppError::BadRequest(_))));
        let (none, ..) = collect_from_box(&db, ada.id, Some(&[second])).await.unwrap();
        assert!(none.collected.is_empty(), "the inventory is full");
    }

    /// D6: placing and collecting need the Exchange hall. From where a new
    /// player arrives they're refused with a reason; on the avenue outside the
    /// door they're allowed; and an online player's live position counts, not
    /// the saved one.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn placing_and_collecting_happen_at_the_exchange_hall(db: PgPool) {
        use crate::models::{ActionState, PlayerState, TerrainConfig, WorldGenParams};
        use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};

        let terrain = TerrainConfig::get_default();
        let door = terrain.landmark(LandmarkKind::ExchangeHall).expect("the default world has a hall").door;
        let game = RwLock::new(GameState::with_terrain(WorldGenParams::default(), terrain));
        let ada = player(&db, "ada", 100).await;
        let save = |(x, y): (f64, f64)| {
            sqlx::query("UPDATE players SET position_x = $1, position_y = $2 WHERE id = $3")
                .bind(x)
                .bind(y)
                .bind(ada.id)
                .execute(&db)
        };

        save((SPAWN_X, SPAWN_Y)).await.unwrap();
        let refused = at_the_exchange(&game, &db, ada.id, "place an order").await;
        assert!(
            matches!(&refused, Err(AppError::BadRequest(m)) if m.contains("too far from the Exchange hall to place an order")),
            "{refused:?}"
        );

        // On the avenue's crown, across the kerb from the door
        let outside = (SPAWN_X, door.1);
        save(outside).await.unwrap();
        assert!(at_the_exchange(&game, &db, ada.id, "place an order").await.is_ok());
        assert!(at_the_exchange(&game, &db, ada.id, "collect from the market box").await.is_ok());

        // Online and walked away: the saved position is stale, the live one counts
        game.write().await.add_player(PlayerState {
            id: ada.id,
            account_id: Uuid::new_v4(),
            name: "ada".into(),
            x: SPAWN_X,
            y: SPAWN_Y,
            dest_x: None,
            dest_y: None,
            speed: 50.0,
            strand_balance: 100,
            current_zone: None,
            action_state: ActionState::Idle,
            citizen: false,
            steer: None,
        });
        assert!(at_the_exchange(&game, &db, ada.id, "collect from the market box").await.is_err());
    }

    #[test]
    fn buckets_are_the_documented_six() {
        let secs: Vec<_> = ["1m", "5m", "15m", "1h", "4h", "1d"].iter().map(|b| bucket_secs(b)).collect();
        assert_eq!(secs, [Some(60), Some(300), Some(900), Some(3600), Some(14_400), Some(86_400)]);
        assert_eq!(bucket_secs("2h"), None);
    }
}
