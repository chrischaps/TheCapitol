//! Subscriptions (E4.1, the plan's D8). No payments yet: an admin starts one
//! (`POST /admin/accounts/{id}/subscription`), and on a development server
//! so can the player (`POST /player/subscription`, behind
//! `MOCK_SUBSCRIPTIONS`), which is what `capitol-bot --subscribe` uses.
//!
//! An account's row holds `subscription_status` and `subscription_until`.
//! A subscription past its `until` has run out at once: [`of`] reads it as
//! lapsed, and the [`Subscriber`] extractor refuses it. The hourly check,
//! [`lapse_expired`], catches the row up and tells the player, then or when
//! they're next back (`player_notices`, #138).
//!
//! [`Subscriber`] gates what GDD §5's access matrix keeps for subscribers.
//! In this plan that's automation that runs while you're away (E4.3), and
//! later guild leadership.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use capitol_protocol::rest::{Subscription, SubscriptionStatus};
use capitol_protocol::ws::ServerMessage;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthUser;

/// The most days one call adds, about ten years.
pub const MAX_DAYS: i32 = 3650;

/// What the notice queue calls a lapse, and the message's tag.
const LAPSED: &str = "subscription_lapsed";

fn read(status: &str, until: Option<DateTime<Utc>>) -> Subscription {
    Subscription { status: SubscriptionStatus::parse(status), until }
}

/// The account's subscription as it stands now. `None` if there's no such
/// account.
pub async fn of(db: &PgPool, account_id: Uuid) -> Result<Option<Subscription>, sqlx::Error> {
    let row: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT subscription_status, subscription_until FROM accounts WHERE id = $1",
    )
    .bind(account_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(status, until)| read(&status, until).as_of(Utc::now())))
}

/// Add `days` to the account's subscription: to what's left of a running
/// one, or from now. `None` if there's no such account.
pub async fn extend(db: &PgPool, account_id: Uuid, days: i32) -> Result<Option<Subscription>, AppError> {
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(AppError::BadRequest(format!("days must be 1 to {MAX_DAYS}")));
    }
    let row: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "UPDATE accounts SET
             subscription_status = 'active',
             subscription_until = GREATEST(
                 CASE WHEN subscription_status = 'active' THEN subscription_until END,
                 NOW()
             ) + make_interval(days => $2)
         WHERE id = $1
         RETURNING subscription_status, subscription_until",
    )
    .bind(account_id)
    .bind(days)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(status, until)| read(&status, until)))
}

/// One subscription the hourly check found run out.
#[derive(Debug, Clone, PartialEq)]
pub struct Lapse {
    pub player_id: Uuid,
    pub until: DateTime<Utc>,
    /// The queued notice, for the live event to take
    pub notice_id: Option<Uuid>,
}

/// Mark every active subscription past its end as lapsed, and queue each
/// player a `subscription_lapsed`. Each account's change and its notice
/// commit together. Errors are logged, and that account waits for the next run.
pub async fn lapse_expired(db: &PgPool) -> Vec<Lapse> {
    let due: Vec<Uuid> = match sqlx::query_scalar(
        "SELECT id FROM accounts WHERE subscription_status = 'active' AND subscription_until <= NOW()",
    )
    .fetch_all(db)
    .await
    {
        Ok(due) => due,
        Err(e) => {
            tracing::error!("Failed to find lapsed subscriptions: {:?}", e);
            return Vec::new();
        }
    };
    let mut lapses = Vec::new();
    for account_id in due {
        match lapse(db, account_id).await {
            Ok(Some(l)) => lapses.push(l),
            Ok(None) => {}
            Err(e) => tracing::error!("Failed to lapse account {account_id}'s subscription: {:?}", e),
        }
    }
    lapses
}

async fn lapse(db: &PgPool, account_id: Uuid) -> Result<Option<Lapse>, sqlx::Error> {
    let mut tx = db.begin().await?;
    // Checked again here: it may have been renewed since the scan.
    let until: Option<DateTime<Utc>> = sqlx::query_scalar(
        "UPDATE accounts SET subscription_status = 'lapsed'
         WHERE id = $1 AND subscription_status = 'active' AND subscription_until <= NOW()
         RETURNING subscription_until",
    )
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    let Some(until) = until else {
        return Ok(None);
    };
    tracing::info!("Account {account_id}'s subscription lapsed (ended {until})");
    let player_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM players WHERE account_id = $1 LIMIT 1")
        .bind(account_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(player_id) = player_id else {
        tx.commit().await?;
        return Ok(None);
    };
    let notice_id = Uuid::new_v4();
    let message = ServerMessage::SubscriptionLapsed { until };
    // An older lapse still waiting says less than this one.
    sqlx::query(
        "WITH superseded AS (DELETE FROM player_notices WHERE player_id = $2 AND kind = $3)
         INSERT INTO player_notices (id, player_id, kind, message) VALUES ($1, $2, $3, $4)",
    )
    .bind(notice_id)
    .bind(player_id)
    .bind(LAPSED)
    .bind(serde_json::to_string(&message).expect("a ServerMessage serialises"))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(Lapse { player_id, until, notice_id: Some(notice_id) }))
}

/// Why a subscriber-only request was refused, for the client to show.
pub fn refusal(subscription: &Subscription) -> String {
    match subscription.status {
        SubscriptionStatus::Lapsed => match subscription.until {
            Some(until) => format!(
                "Your subscription lapsed on {}. Renew it to use this.",
                until.format("%-d %B %Y")
            ),
            None => "Your subscription lapsed. Renew it to use this.".into(),
        },
        _ => "This needs a subscription.".into(),
    }
}

/// An extractor for subscriber-only routes, after `auth_middleware`. A
/// request from an account without a running subscription is refused with a
/// `403` and [`refusal`]'s reason.
#[derive(Debug, Clone)]
#[allow(dead_code)] // E4.3 (#178) is the first route behind it
pub struct Subscriber {
    pub account_id: Uuid,
    pub subscription: Subscription,
}

impl<S> FromRequestParts<S> for Subscriber
where
    S: Send + Sync,
    PgPool: FromRef<S>,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let account_id = parts
            .extensions
            .get::<AuthUser>()
            .map(|user| user.account_id)
            .ok_or_else(|| AppError::Unauthorized("Missing authorization header".into()))?;
        let db = PgPool::from_ref(state);
        let subscription = of(&db, account_id)
            .await?
            .ok_or_else(|| AppError::Unauthorized("No such account".into()))?;
        if !subscription.is_active_at(Utc::now()) {
            return Err(AppError::Forbidden(refusal(&subscription)));
        }
        Ok(Subscriber { account_id, subscription })
    }
}

/// The `#[ignore]`d ones need the local Postgres (`docker-compose up -d`).
/// Run them with `cargo test subscriptions -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::notices;
    use crate::engine::plot_sync::tests::player;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    async fn account_of(db: &PgPool, player_id: Uuid) -> Uuid {
        sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1")
            .bind(player_id)
            .fetch_one(db)
            .await
            .unwrap()
    }

    /// Move the account's end `hours` from now (negative: in the past)
    async fn ends_in(db: &PgPool, account: Uuid, hours: i32) {
        sqlx::query("UPDATE accounts SET subscription_until = NOW() + make_interval(hours => $2) WHERE id = $1")
            .bind(account)
            .bind(hours)
            .execute(db)
            .await
            .unwrap();
    }

    /// A route behind the extractor, as the account `account`
    async fn gated(db: &PgPool, account: Uuid) -> (StatusCode, String) {
        let app = Router::new()
            .route("/gated", get(|s: Subscriber| async move { s.account_id.to_string() }))
            .layer(axum::middleware::from_fn(move |mut req: Request<Body>, next: axum::middleware::Next| async move {
                req.extensions_mut().insert(AuthUser { account_id: account, is_admin: false, key: None });
                next.run(req).await
            }))
            .with_state(db.clone());
        let response = app.oneshot(Request::get("/gated").body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1 << 16).await.unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[test]
    fn the_reason_says_when_it_lapsed() {
        let until = "2026-11-17T12:00:00Z".parse().unwrap();
        let lapsed = Subscription { status: SubscriptionStatus::Lapsed, until: Some(until) };
        assert_eq!(refusal(&lapsed), "Your subscription lapsed on 17 November 2026. Renew it to use this.");
        assert_eq!(refusal(&Subscription::default()), "This needs a subscription.");
    }

    #[test]
    fn an_active_one_past_its_end_reads_as_lapsed() {
        let now = Utc::now();
        let ending = |hours| Subscription {
            status: SubscriptionStatus::Active,
            until: Some(now + chrono::Duration::hours(hours)),
        };
        assert!(ending(1).is_active_at(now));
        assert_eq!(ending(1).as_of(now).status, SubscriptionStatus::Active);
        assert!(!ending(-1).is_active_at(now));
        assert_eq!(ending(-1).as_of(now).status, SubscriptionStatus::Lapsed);
        assert!(!Subscription::default().is_active_at(now));
        assert_eq!(Subscription::default().as_of(now), Subscription::default());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn days_add_to_what_is_left(db: PgPool) {
        let ada = account_of(&db, player(&db, "ada", 0).await).await;
        assert_eq!(of(&db, ada).await.unwrap(), Some(Subscription::default()));

        let first = extend(&db, ada, 30).await.unwrap().unwrap();
        assert_eq!(first.status, SubscriptionStatus::Active);
        let second = extend(&db, ada, 10).await.unwrap().unwrap();
        let added = second.until.unwrap() - first.until.unwrap();
        assert_eq!(added.num_days(), 10, "added to the end, not to now");
        assert_eq!(of(&db, ada).await.unwrap(), Some(second));

        assert!(matches!(extend(&db, ada, 0).await, Err(AppError::BadRequest(_))));
        assert!(matches!(extend(&db, ada, MAX_DAYS + 1).await, Err(AppError::BadRequest(_))));
        assert_eq!(extend(&db, Uuid::new_v4(), 5).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn a_lapsed_one_starts_again_from_now(db: PgPool) {
        let ada = account_of(&db, player(&db, "ada", 0).await).await;
        extend(&db, ada, 30).await.unwrap();
        ends_in(&db, ada, -48).await;
        let renewed = extend(&db, ada, 1).await.unwrap().unwrap();
        assert!(renewed.until.unwrap() > Utc::now() + chrono::Duration::hours(23));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_extractor_gates_by_status_and_time(db: PgPool) {
        let ada = account_of(&db, player(&db, "ada", 0).await).await;

        let (status, body) = gated(&db, ada).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("This needs a subscription."), "{body}");

        extend(&db, ada, 30).await.unwrap();
        assert_eq!(gated(&db, ada).await, (StatusCode::OK, ada.to_string()));

        // Past its end it's refused at once, before the hourly check
        ends_in(&db, ada, -1).await;
        let (status, body) = gated(&db, ada).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("lapsed on"), "{body}");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn the_hourly_check_lapses_only_what_has_ended_and_says_so(db: PgPool) {
        let ada_player = player(&db, "ada", 0).await;
        let bram_player = player(&db, "bram", 0).await;
        let ada = account_of(&db, ada_player).await;
        let bram = account_of(&db, bram_player).await;
        extend(&db, ada, 30).await.unwrap();
        extend(&db, bram, 30).await.unwrap();
        ends_in(&db, ada, -1).await;

        let lapses = lapse_expired(&db).await;
        assert_eq!(lapses.len(), 1);
        assert_eq!(lapses[0].player_id, ada_player);
        assert_eq!(of(&db, ada).await.unwrap().unwrap().status, SubscriptionStatus::Lapsed);
        assert_eq!(of(&db, bram).await.unwrap().unwrap().status, SubscriptionStatus::Active);
        assert!(lapse_expired(&db).await.is_empty(), "once");

        // Ada was away: the notice waits for her
        let waiting = notices::take_all(&db, ada_player).await.unwrap();
        assert_eq!(waiting, vec![ServerMessage::SubscriptionLapsed { until: lapses[0].until }]);
        assert!(notices::take_all(&db, bram_player).await.unwrap().is_empty());
    }
}
