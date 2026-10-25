//! Other people (S0.3): a person's card, who they are, as anyone may see it.
//!
//! `/player` is you; `/players` is everyone. A card says only what anyone
//! in the city could know: the name, whether they're a citizen, how long
//! they've been here, where they live, what's said of them and what they've
//! been busy with lately (K3.1, [`lately`]), in words. Nothing of their
//! Strands or goods. Your own card adds your standing's number, its trend
//! and your history (S3.1), which nobody else's shows.
//!
//! Mutes (S1.4) are here too: `PUT` and `DELETE /players/{id}/mute`, and
//! `GET /player/mutes` beside the rest of you. `crate::mutes` keeps them.

use axum::{
    extract::{Path, State},
    routing::{get, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use capitol_protocol::rest::{MutesResponse, PlayerCard};
use capitol_protocol::rules;
use capitol_protocol::rules::lately::{self, Lately, Venue};

use crate::engine::standing;
use crate::engine::tutorial::{self, Did};
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/{id}/card", get(get_card))
        .route("/by-name/{name}", get(get_card_by_name))
        .route("/{id}/mute", put(mute).delete(unmute))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Someone's card: who they are
#[utoipa::path(
    get,
    path = "/players/{id}/card",
    params(("id" = Uuid, Path, description = "The player's id")),
    responses(
        (status = 200, description = "Who they are", body = PlayerCard),
        (status = 404, description = "No such player")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_card(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<PlayerCard>, AppError> {
    let card = card(&state.db, auth_user.account_id, Who::Id(id)).await?;
    if !card.mine {
        tutorial::saw_by_account(&state, auth_user.account_id, Did::OpenedCard).await;
    }
    Ok(Json(card))
}

/// Someone's card, by their name, ignoring case: for a letter
///
/// Names aren't unique among people, so a name two share finds the one who
/// took it first ([#428](https://github.com/chrischaps/TheCapitol/issues/428)).
/// No two citizens share one, and a citizen comes before a person.
#[utoipa::path(
    get,
    path = "/players/by-name/{name}",
    params(("name" = String, Path, description = "The player's name, in any case")),
    responses(
        (status = 200, description = "Who they are", body = PlayerCard),
        (status = 404, description = "Nobody of that name")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn get_card_by_name(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(name): Path<String>,
) -> Result<Json<PlayerCard>, AppError> {
    let card = card(&state.db, auth_user.account_id, Who::Name(&name)).await?;
    // The Gazette's names open cards this way, and count as any other (#447)
    if !card.mine {
        tutorial::saw_by_account(&state, auth_user.account_id, Did::OpenedCard).await;
    }
    Ok(Json(card))
}

/// Stop hearing someone: nothing they say reaches you, on any device
#[utoipa::path(
    put,
    path = "/players/{id}/mute",
    params(("id" = Uuid, Path, description = "Who to mute")),
    responses(
        (status = 200, description = "The people you've muted now", body = MutesResponse),
        (status = 400, description = "You can't mute yourself"),
        (status = 404, description = "No such player")
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn mute(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MutesResponse>, AppError> {
    set_mute(&state, auth_user.account_id, id, true).await
}

/// Hear someone again
#[utoipa::path(
    delete,
    path = "/players/{id}/mute",
    params(("id" = Uuid, Path, description = "Who to unmute")),
    responses(
        (status = 200, description = "The people you've muted now", body = MutesResponse)
    ),
    security(("bearer_auth" = [])),
    tag = "Player"
)]
pub async fn unmute(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MutesResponse>, AppError> {
    set_mute(&state, auth_user.account_id, id, false).await
}

async fn set_mute(state: &AppState, account: Uuid, other: Uuid, on: bool) -> Result<Json<MutesResponse>, AppError> {
    let player = player_of(&state.db, account).await?;
    let events = state.game.read().await.event_sender.clone();
    crate::mutes::set(&state.db, &events, player, other, on).await?;
    Ok(Json(MutesResponse { mutes: crate::mutes::list(&state.db, player).await? }))
}

/// The account's player.
pub async fn player_of(db: &PgPool, account: Uuid) -> Result<Uuid, AppError> {
    sqlx::query_scalar("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("No player for this account".into()))
}

/// Whose card.
pub enum Who<'a> {
    Id(Uuid),
    Name(&'a str),
}

/// The card of `who`, as the account `viewer` sees it.
pub async fn card(db: &PgPool, viewer: Uuid, who: Who<'_>) -> Result<PlayerCard, AppError> {
    // Their oldest plot is their home; one claimed before claims were dated counts last
    const CARD: &str = "SELECT p.id, p.name, a.kind = 'citizen', a.created_at, p.account_id = $2,
                (SELECT pl.zone_id FROM plots pl WHERE pl.owner_id = p.id
                 ORDER BY pl.claimed_at NULLS LAST, pl.id LIMIT 1),
                EXISTS (SELECT 1 FROM player_mutes m JOIN players me ON me.id = m.player_id
                        WHERE me.account_id = $2 AND m.muted_id = p.id),
                (SELECT g.name FROM guild_members gm JOIN guilds g ON g.id = gm.guild_id
                 WHERE gm.player_id = p.id),
                p.gazette_named
         FROM players p JOIN accounts a ON a.id = p.account_id";
    type Row = (Uuid, String, bool, DateTime<Utc>, bool, Option<String>, bool, Option<String>, bool);
    let row: Option<Row> = match who {
        Who::Id(id) => {
            sqlx::query_as(&format!("{CARD} WHERE p.id = $1"))
                .bind(id)
                .bind(viewer)
                .fetch_optional(db)
                .await?
        }
        Who::Name(name) => {
            sqlx::query_as(&format!(
                "{CARD} WHERE lower(p.name) = lower($1)
                 ORDER BY a.kind = 'citizen' DESC, p.created_at, p.id LIMIT 1"
            ))
            .bind(name.trim())
            .bind(viewer)
            .fetch_optional(db)
            .await?
        }
    };
    let (id, name, citizen, created, mine, home, muted, guild, named) =
        row.ok_or_else(|| AppError::NotFound("No one goes by that".into()))?;
    // Anyone sees the words; only you, your number and how it got there (S3.1)
    let now = Utc::now();
    let mut conn = db.acquire().await?;
    let own = if mine { Some(standing::own(&mut conn, id, now).await?) } else { None };
    let words = match &own {
        Some(own) => own.words.clone(),
        None => standing::words_for(&mut conn, id, now).await?,
    };
    // What they've been busy with keeps out of sight with their byline
    let lately = if mine || named { lately(&mut conn, id, now).await? } else { None };
    Ok(PlayerCard {
        id,
        name,
        citizen,
        since: rules::day_number(created.timestamp_millis()),
        home,
        standing: words,
        guild,
        lately,
        mine,
        muted,
        own,
    })
}

/// What `player` has been busy with these last `lately::DAYS` game days,
/// today among them (K3.1): what they made (`production_tally`), each unit
/// at its item's target price, or its last trade where it has none, as the
/// maker board counts it (S3.2); and whether they sold at the bazaar or on
/// the Exchange. `None` when there's nothing to say.
pub async fn lately(conn: &mut PgConnection, player: Uuid, now: DateTime<Utc>) -> Result<Option<Lately>, AppError> {
    let first = rules::day_number(now.timestamp_millis()) - lately::DAYS + 1;
    let since = DateTime::from_timestamp_millis(rules::day_start_ms(first)).unwrap_or(now);
    let made: Vec<(String, String, f64)> = sqlx::query_as(
        "SELECT pt.item_type, COALESCE(it.name, pt.item_type),
                SUM(pt.quantity * COALESCE(it.target_price, l.price, 1))::FLOAT8
         FROM production_tally pt
         LEFT JOIN item_types it ON it.id = pt.item_type
         LEFT JOIN LATERAL (SELECT t.price FROM market_trades t WHERE t.item_type = pt.item_type
                            ORDER BY t.at DESC LIMIT 1) l ON TRUE
         WHERE pt.player_id = $1 AND pt.day >= $2
         GROUP BY 1, 2
         ORDER BY 3 DESC, 1",
    )
    .bind(player)
    .bind(first)
    .fetch_all(&mut *conn)
    .await?;
    let (bazaar, exchange): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM bazaar_sales s WHERE s.merchant_id = $1 AND s.at >= $2),
                EXISTS (SELECT 1 FROM market_trades t JOIN market_orders o ON o.id = t.sell_order_id
                        WHERE o.player_id = $1 AND t.at >= $2)",
    )
    .bind(player)
    .bind(since)
    .fetch_one(&mut *conn)
    .await?;
    let sells: Vec<Venue> = [(bazaar, Venue::Bazaar), (exchange, Venue::Exchange)]
        .into_iter()
        .filter_map(|(did, venue)| did.then_some(venue))
        .collect();
    Ok(lately::from_work(made.iter().map(|(item, name, value)| (item.as_str(), name.as_str(), *value)), &sells))
}

/// These need the local Postgres (`docker-compose up -d`). Each runs in a fresh
/// database that `sqlx::test` makes from the migrations. Run them with
/// `cargo test card -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::{player, plot};

    async fn account_of(db: &PgPool, player: Uuid) -> Uuid {
        sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1")
            .bind(player)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// A citizen called `name`, whose account was made at `at`. An account
    /// never changes kind, so it's a citizen from the first.
    async fn citizen_since(db: &PgPool, name: &str, at: DateTime<Utc>) -> Uuid {
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO accounts (id, email, password_hash, kind, created_at) VALUES ($1, $2, 'x', 'citizen', $3)")
            .bind(account)
            .bind(format!("{account}@citizens.test"))
            .bind(at)
            .execute(db)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO players (id, account_id, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(account)
            .bind(name)
            .execute(db)
            .await
            .unwrap();
        id
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_persons_card_says_who_they_are_and_whose_it_is(db: PgPool) {
        let ada = player(&db, "Ada", 100).await;
        let bea = player(&db, "Bea", 100).await;
        let (ada_acc, bea_acc) = (account_of(&db, ada).await, account_of(&db, bea).await);

        let seen = card(&db, bea_acc, Who::Id(ada)).await.unwrap();
        assert_eq!((seen.id, seen.name.as_str(), seen.citizen, seen.mine), (ada, "Ada", false, false));
        assert_eq!(seen.since, rules::day_number(Utc::now().timestamp_millis()));
        assert_eq!((seen.home, seen.guild), (None, None), "no plot, and no guilds yet");
        assert_eq!(seen.standing, rules::standing::NEW_IN_TOWN);

        let own = card(&db, ada_acc, Who::Id(ada)).await.unwrap();
        assert!(own.mine && !own.muted);

        // Muted by Bea (S1.4): her card of Ada says so, and no one else's
        let (events, _rx) = tokio::sync::broadcast::channel(4);
        crate::mutes::set(&db, &events, bea, ada, true).await.unwrap();
        assert!(card(&db, bea_acc, Who::Id(ada)).await.unwrap().muted);
        assert!(!card(&db, ada_acc, Who::Id(bea)).await.unwrap().muted, "a mute is one way");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn someone_elses_card_shows_the_words_and_yours_the_number_and_history_too(db: PgPool) {
        let ada = player(&db, "Ada", 100).await;
        let bea = player(&db, "Bea", 100).await;
        // Five weeks of tax in good standing: 5, dependable
        for week in 0..5 {
            sqlx::query("INSERT INTO tax_good_weeks (id, player_id, at) VALUES ($1, $2, NOW() - $3 * INTERVAL '1 week')")
                .bind(Uuid::new_v4())
                .bind(ada)
                .bind(week as f64)
                .execute(&db)
                .await
                .unwrap();
        }

        let seen = card(&db, account_of(&db, bea).await, Who::Id(ada)).await.unwrap();
        assert_eq!(seen.standing, rules::standing::DEPENDABLE);
        assert_eq!(seen.own, None, "the number is Ada's alone");
        let by_name = card(&db, account_of(&db, bea).await, Who::Name("ada")).await.unwrap();
        assert_eq!((by_name.standing.as_str(), by_name.own.is_none()), (rules::standing::DEPENDABLE, true));

        let mine = card(&db, account_of(&db, ada).await, Who::Id(ada)).await.unwrap();
        let own = mine.own.expect("your own card has your number");
        assert_eq!((mine.standing.as_str(), own.words.as_str()), (rules::standing::DEPENDABLE, rules::standing::DEPENDABLE));
        assert!((own.score - 5.0).abs() < 1e-9 && own.trend.abs() < 1.0 + 1e-9, "{own:?}");
        assert_eq!((own.records, own.history.len()), (5, 5));
        assert!(own.history.iter().all(|r| r.deed == rules::standing::Deed::TaxWeek));
        assert!(own.history.windows(2).all(|w| w[0].at >= w[1].at), "newest first");

        // Bea, with nothing done yet, is new in town to both
        assert_eq!(card(&db, account_of(&db, ada).await, Who::Id(bea)).await.unwrap().standing, rules::standing::NEW_IN_TOWN);
        let hers = card(&db, account_of(&db, bea).await, Who::Id(bea)).await.unwrap().own.unwrap();
        assert_eq!((hers.score, hers.records, hers.history.len()), (0.0, 0, 0));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_citizens_card_carries_its_mark_its_day_and_its_home(db: PgPool) {
        let at = DateTime::from_timestamp_millis(rules::day_start_ms(rules::GAZETTE_FIRST_DAY + 7) + 1000).unwrap();
        let cy = citizen_since(&db, "Citizen Cy", at).await;
        let ada = player(&db, "Ada", 100).await;
        plot(&db, Some(cy), 250).await;

        let seen = card(&db, account_of(&db, ada).await, Who::Id(cy)).await.unwrap();
        assert!(seen.citizen && !seen.mine);
        assert_eq!(seen.since, rules::GAZETTE_FIRST_DAY + 7);
        assert_eq!(seen.home.as_deref(), Some("urban"));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_card_for_an_unknown_id_or_name_is_not_found(db: PgPool) {
        let ada = player(&db, "Ada", 100).await;
        let me = account_of(&db, ada).await;
        let err = card(&db, me, Who::Id(Uuid::new_v4())).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
        let err = card(&db, me, Who::Name("Nobody")).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_card_by_name_ignores_case_and_finds_whoever_took_it_first(db: PgPool) {
        let ada = player(&db, "Ada Reeder", 100).await;
        let me = account_of(&db, ada).await;
        let found = card(&db, me, Who::Name("  ada REEDER ")).await.unwrap();
        assert_eq!((found.id, found.name.as_str(), found.mine), (ada, "Ada Reeder", true));

        // A second Ada, a day later, isn't who the name finds (#428)
        let later = player(&db, "ada reeder", 100).await;
        sqlx::query("UPDATE players SET created_at = NOW() + INTERVAL '1 day' WHERE id = $1")
            .bind(later)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(card(&db, me, Who::Name("Ada Reeder")).await.unwrap().id, ada);
    }

    #[test]
    fn every_good_in_the_content_has_an_activity_for_the_card() {
        let content = crate::content::load().unwrap();
        let missing: Vec<&str> =
            content.items.iter().map(|i| i.id.as_str()).filter(|id| lately::activity_of(id).is_none()).collect();
        assert!(missing.is_empty(), "add these to rules::lately::ACTIVITIES: {missing:?}");
    }

    /// `quantity` of `item` tallied to `who` on `day`
    async fn made(db: &PgPool, who: Uuid, day: i64, item: &str, quantity: i64) {
        sqlx::query("INSERT INTO production_tally (day, player_id, item_type, quantity) VALUES ($1, $2, $3, $4)")
            .bind(day)
            .bind(who)
            .bind(item)
            .bind(quantity)
            .execute(db)
            .await
            .unwrap();
    }

    /// A fill of fiber from `seller` to `buyer` at `at`
    async fn sold(db: &PgPool, seller: Uuid, buyer: Uuid, at: DateTime<Utc>) {
        let mut ids = Vec::new();
        for (side, who) in [("sell", seller), ("buy", buyer)] {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO market_orders (id, player_id, side, item_type, min_grade, grade_rule, price,
                     quantity_total, quantity_remaining, status, fee, expires_at)
                 VALUES ($1, $2, $3, 'fiber', $4, $5, 2, 5, 0, 'filled', 1, NOW() + INTERVAL '7 days')",
            )
            .bind(id)
            .bind(who)
            .bind(side)
            .bind((side == "buy").then_some("F"))
            .bind((side == "buy").then_some("average"))
            .execute(db)
            .await
            .unwrap();
            ids.push(id);
        }
        sqlx::query(
            "INSERT INTO market_trades (id, buy_order_id, sell_order_id, item_type, grade, quality, min_quality, price, quantity, at)
             VALUES ($1, $2, $3, 'fiber', 'C', 50, 50, 2, 5, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(ids[1])
        .bind(ids[0])
        .bind(at)
        .execute(db)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_card_says_what_theyve_been_busy_with_unless_they_keep_out_of_the_papers(db: PgPool) {
        let ada = player(&db, "Ada", 100).await;
        let bea = player(&db, "Bea", 100).await;
        let (ada_acc, bea_acc) = (account_of(&db, ada).await, account_of(&db, bea).await);
        let today = rules::day_number(Utc::now().timestamp_millis());
        let first = DateTime::from_timestamp_millis(rules::day_start_ms(today - lately::DAYS + 1)).unwrap();

        // Nothing done yet: nothing to say
        assert_eq!(card(&db, bea_acc, Who::Id(ada)).await.unwrap().lately, None);
        assert_eq!(card(&db, ada_acc, Who::Id(ada)).await.unwrap().lately, None);

        // Planks most, logs too, a pinch of fiber; a pile of fiber too long ago to count
        made(&db, ada, today, "plank", 30).await;
        made(&db, ada, today - 2, "log", 10).await;
        made(&db, ada, today, "fiber", 1).await;
        made(&db, ada, today - lately::DAYS, "fiber", 500).await;
        // A stall's sale today; a fill on the Exchange from before the three days
        sqlx::query(
            "INSERT INTO bazaar_sales (id, listing_id, stall_id, shop_name, merchant_id, buyer_id, item_type, quality, price, quantity)
             VALUES ($1, $2, 'stall_0', 'Ada''s', $3, $4, 'fiber', 50, 2, 3)",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(ada)
        .bind(bea)
        .execute(&db)
        .await
        .unwrap();
        sold(&db, ada, bea, first - chrono::Duration::minutes(1)).await;

        let seen = card(&db, bea_acc, Who::Id(ada)).await.unwrap().lately.expect("Ada has been busy");
        assert_eq!(seen.doing, ["sawing planks", "felling trees"]);
        assert_eq!(seen.sells, [Venue::Bazaar]);
        assert_eq!(lately::words(&seen), "sawing planks and felling trees, and selling at the bazaar");
        assert_eq!(card(&db, ada_acc, Who::Id(ada)).await.unwrap().lately.as_ref(), Some(&seen), "the same on her own");

        // Bea bought; only a seller sells. Then she sells on the Exchange, at the first moment that counts
        assert_eq!(card(&db, ada_acc, Who::Id(bea)).await.unwrap().lately, None);
        sold(&db, bea, ada, first).await;
        let hers = card(&db, ada_acc, Who::Id(bea)).await.unwrap().lately.expect("Bea has sold");
        assert_eq!((hers.doing.len(), hers.sells.as_slice()), (0, [Venue::Exchange].as_slice()));

        // Out of the papers: off everyone's card but her own
        sqlx::query("UPDATE players SET gazette_named = FALSE WHERE id = $1").bind(ada).execute(&db).await.unwrap();
        assert_eq!(card(&db, bea_acc, Who::Id(ada)).await.unwrap().lately, None);
        assert_eq!(card(&db, bea_acc, Who::Name("ada")).await.unwrap().lately, None);
        assert_eq!(card(&db, ada_acc, Who::Id(ada)).await.unwrap().lately, Some(seen));
    }
}
