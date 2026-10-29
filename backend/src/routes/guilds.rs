//! `/guilds` (S4.1, commons plan D10): the registry, founding, and the roster.
//!
//! Founding needs a subscription (the [`Subscriber`] extractor) and happens
//! at the Exchange hall, as placing an order does. Everything else works
//! from anywhere. Each write is one of `engine::guilds`' transactions; once
//! it commits, the route sends `guild_changed` to those it touched, the
//! notices to those it names (live if they're on, else when they're back),
//! and the payer their balance. Every write answers with your guild as it
//! stands now ([`MyGuild`]), so a client never patches its own copy.

use axum::{
    extract::{Path, State},
    routing::{get, post, put},
    Extension, Json, Router,
};
use capitol_protocol::rest::{ClaimHall, FoundGuild, GuildBooks, GuildDetail, GuildStrands, GuildsResponse, MyGuild, SetGuildRole};
use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::market::{at_the_exchange, player_of};
use crate::engine::guilds::{self, GuildError, Told};
use crate::engine::GameState;
use crate::error::AppError;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;
use crate::subscriptions::Subscriber;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(found))
        .route("/{id}", get(get_guild))
        .route("/{id}/apply", post(apply))
        .route("/{id}/leave", post(leave))
        .route("/{id}/invites/{player_id}", post(invite).delete(withdraw))
        .route("/{id}/members/{player_id}", put(set_role).delete(expel))
        .route("/{id}/deposit", post(deposit))
        .route("/{id}/withdraw", post(withdraw_strands))
        .route("/{id}/limit", put(set_limit))
        .route("/{id}/books", get(books))
        .route("/{id}/hall", post(claim_hall))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::middleware::auth::auth_middleware,
        ))
}

/// Every guild, the largest first
#[utoipa::path(
    get,
    path = "/guilds",
    responses((status = 200, description = "The registry", body = GuildsResponse)),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn list(State(state): State<AppState>) -> Result<Json<GuildsResponse>, AppError> {
    Ok(Json(GuildsResponse { guilds: guilds::list(&state.db).await? }))
}

/// Found a guild, at the Exchange hall, for 500 Strands. Needs a subscription.
#[utoipa::path(
    post,
    path = "/guilds",
    request_body = FoundGuild,
    responses(
        (status = 200, description = "Founded, and you lead it", body = MyGuild),
        (status = 400, description = "Away from the hall, a name or tag that can't be, or not enough Strands"),
        (status = 403, description = "No subscription"),
        (status = 409, description = "The name or tag is taken, or you're in a guild")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn found(
    State(state): State<AppState>,
    subscriber: Subscriber,
    Json(req): Json<FoundGuild>,
) -> Result<Json<MyGuild>, AppError> {
    let founder = player_of(&state.db, subscriber.account_id).await?;
    let (_, told) = found_at_the_hall(&state.game, &state.db, founder, &req).await?;
    tell(&state, told).await;
    Ok(Json(guilds::mine(&state.db, founder, &lots(&state).await).await.map_err(refused)?))
}

/// [`found`] past its extractor: at the hall, then the registry's.
async fn found_at_the_hall(
    game: &RwLock<GameState>,
    db: &PgPool,
    founder: Uuid,
    req: &FoundGuild,
) -> Result<(Uuid, Told), AppError> {
    at_the_exchange(game, db, founder, "found a guild").await?;
    guilds::found(db, founder, &req.name, &req.tag, &req.charter, Utc::now()).await.map_err(refused)
}

/// A guild: its charter and members, and, for its officers and leader,
/// who's waiting
#[utoipa::path(
    get,
    path = "/guilds/{id}",
    params(("id" = Uuid, Path, description = "The guild")),
    responses(
        (status = 200, description = "The guild", body = GuildDetail),
        (status = 404, description = "No such guild")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn get_guild(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<GuildDetail>, AppError> {
    let viewer = player_of(&state.db, auth_user.account_id).await.ok();
    Ok(Json(guilds::detail(&state.db, id, viewer, &lots(&state).await).await.map_err(refused)?))
}

/// Ask to join a guild. If it invited you, you're in, for a Strand.
#[utoipa::path(
    post,
    path = "/guilds/{id}/apply",
    params(("id" = Uuid, Path, description = "The guild")),
    responses(
        (status = 200, description = "Applied, or joined", body = MyGuild),
        (status = 400, description = "No Strand to join with"),
        (status = 404, description = "No such guild"),
        (status = 409, description = "You're in a guild, or you've applied already")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn apply(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MyGuild>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::apply(&state.db, player, id, Utc::now()).await.map_err(refused)?;
    answer(&state, player, told).await
}

/// Invite someone, as an officer or the leader. If they'd applied, they're
/// in, for a Strand.
#[utoipa::path(
    post,
    path = "/guilds/{id}/invites/{player_id}",
    params(
        ("id" = Uuid, Path, description = "The guild"),
        ("player_id" = Uuid, Path, description = "Who to invite")
    ),
    responses(
        (status = 200, description = "Invited, or taken in", body = MyGuild),
        (status = 400, description = "No Strand for the roster change"),
        (status = 403, description = "Not an officer, or a leader with no subscription"),
        (status = 404, description = "No such guild or player"),
        (status = 409, description = "They're in a guild, or invited already")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn invite(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path((id, player_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<MyGuild>, AppError> {
    let actor = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::invite(&state.db, actor, id, player_id, Utc::now()).await.map_err(refused)?;
    answer(&state, actor, told).await
}

/// Drop an invitation or an application: your own, or, as an officer or
/// the leader, the guild's
#[utoipa::path(
    delete,
    path = "/guilds/{id}/invites/{player_id}",
    params(
        ("id" = Uuid, Path, description = "The guild"),
        ("player_id" = Uuid, Path, description = "Who it's for")
    ),
    responses(
        (status = 200, description = "Dropped", body = MyGuild),
        (status = 403, description = "Not yours to drop"),
        (status = 404, description = "No such guild, invitation or application")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn withdraw(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path((id, player_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<MyGuild>, AppError> {
    let actor = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::withdraw(&state.db, actor, id, player_id).await.map_err(refused)?;
    answer(&state, actor, told).await
}

/// Leave your guild, for nothing. A leader leaves only as the last member,
/// and the guild is disbanded.
#[utoipa::path(
    post,
    path = "/guilds/{id}/leave",
    params(("id" = Uuid, Path, description = "Your guild")),
    responses(
        (status = 200, description = "Left", body = MyGuild),
        (status = 400, description = "A leader with members must hand on the lead first"),
        (status = 404, description = "You aren't in that guild")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn leave(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MyGuild>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::leave(&state.db, player, id, Utc::now()).await.map_err(refused)?;
    answer(&state, player, told).await
}

/// Expel someone: an officer expels members, the leader officers too. A Strand.
#[utoipa::path(
    delete,
    path = "/guilds/{id}/members/{player_id}",
    params(
        ("id" = Uuid, Path, description = "The guild"),
        ("player_id" = Uuid, Path, description = "Who to expel")
    ),
    responses(
        (status = 200, description = "Expelled", body = MyGuild),
        (status = 400, description = "No Strand for the roster change"),
        (status = 403, description = "Your role can't expel theirs"),
        (status = 404, description = "No such guild, or they aren't in it")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn expel(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path((id, player_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<MyGuild>, AppError> {
    let actor = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::expel(&state.db, actor, id, player_id, Utc::now()).await.map_err(refused)?;
    answer(&state, actor, told).await
}

/// As the leader: make someone an officer or a member, or hand them the
/// lead (they need a subscription). A Strand, unless nothing changes.
#[utoipa::path(
    put,
    path = "/guilds/{id}/members/{player_id}",
    params(
        ("id" = Uuid, Path, description = "The guild"),
        ("player_id" = Uuid, Path, description = "Whose role")
    ),
    request_body = SetGuildRole,
    responses(
        (status = 200, description = "Set", body = MyGuild),
        (status = 400, description = "No Strand for the roster change"),
        (status = 403, description = "Not the leader, or a new leader with no subscription"),
        (status = 404, description = "No such guild, or they aren't in it")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn set_role(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path((id, player_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetGuildRole>,
) -> Result<Json<MyGuild>, AppError> {
    let actor = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::set_role(&state.db, actor, id, player_id, req.role, Utc::now()).await.map_err(refused)?;
    answer(&state, actor, told).await
}

/// Put Strands of yours into your guild's treasury, from anywhere
#[utoipa::path(
    post,
    path = "/guilds/{id}/deposit",
    params(("id" = Uuid, Path, description = "Your guild")),
    request_body = GuildStrands,
    responses(
        (status = 200, description = "Deposited", body = MyGuild),
        (status = 400, description = "Not a number of Strands, or more than you have"),
        (status = 404, description = "You aren't in that guild")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn deposit(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(req): Json<GuildStrands>,
) -> Result<Json<MyGuild>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::deposit(&state.db, player, id, req.strands, Utc::now()).await.map_err(refused)?;
    answer(&state, player, told).await
}

/// Take Strands out of your guild's treasury: the leader as they like, an
/// officer within the leader's weekly limit
#[utoipa::path(
    post,
    path = "/guilds/{id}/withdraw",
    params(("id" = Uuid, Path, description = "Your guild")),
    request_body = GuildStrands,
    responses(
        (status = 200, description = "Withdrawn", body = MyGuild),
        (status = 400, description = "Not a number of Strands, more than the treasury holds, or over your limit"),
        (status = 403, description = "A member can't draw on the treasury"),
        (status = 404, description = "You aren't in that guild")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn withdraw_strands(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(req): Json<GuildStrands>,
) -> Result<Json<MyGuild>, AppError> {
    let actor = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::withdraw_strands(&state.db, actor, id, req.strands, &req.note, Utc::now())
        .await
        .map_err(refused)?;
    answer(&state, actor, told).await
}

/// As the leader: what each officer may withdraw in a rolling week
#[utoipa::path(
    put,
    path = "/guilds/{id}/limit",
    params(("id" = Uuid, Path, description = "Your guild")),
    request_body = GuildStrands,
    responses(
        (status = 200, description = "Set", body = MyGuild),
        (status = 400, description = "Not a number of Strands"),
        (status = 403, description = "Not the leader"),
        (status = 404, description = "You aren't in that guild")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn set_limit(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(req): Json<GuildStrands>,
) -> Result<Json<MyGuild>, AppError> {
    let actor = player_of(&state.db, auth_user.account_id).await?;
    let told = guilds::set_limit(&state.db, actor, id, req.strands).await.map_err(refused)?;
    answer(&state, actor, told).await
}

/// Your guild's books: every move of its treasury, newest first
#[utoipa::path(
    get,
    path = "/guilds/{id}/books",
    params(("id" = Uuid, Path, description = "Your guild")),
    responses(
        (status = 200, description = "The books", body = GuildBooks),
        (status = 404, description = "You aren't in that guild")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn books(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<GuildBooks>, AppError> {
    let player = player_of(&state.db, auth_user.account_id).await?;
    Ok(Json(guilds::books(&state.db, id, player).await.map_err(refused)?))
}

/// Claim a guild-district lot as your guild's hall, at the Exchange hall,
/// from its treasury: the leader alone (S4.3)
#[utoipa::path(
    post,
    path = "/guilds/{id}/hall",
    params(("id" = Uuid, Path, description = "Your guild")),
    request_body = ClaimHall,
    responses(
        (status = 200, description = "The hall is the guild's", body = MyGuild),
        (status = 400, description = "Away from the Exchange, not a guild-district lot, or the treasury is short"),
        (status = 403, description = "Only the leader claims a hall"),
        (status = 404, description = "No such guild or lot"),
        (status = 409, description = "The guild holds a hall already, or someone holds the lot")
    ),
    security(("bearer_auth" = [])),
    tag = "Guilds"
)]
pub async fn claim_hall(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(req): Json<ClaimHall>,
) -> Result<Json<MyGuild>, AppError> {
    let leader = player_of(&state.db, auth_user.account_id).await?;
    at_the_exchange(&state.game, &state.db, leader, "claim a hall").await?;
    let tunables = state.tunables.get();
    let told = guilds::claim_hall(&state.db, leader, id, req.plot_id, &tunables, Utc::now()).await.map_err(refused)?;
    answer(&state, leader, told).await
}

/// Tell what `told` says, then answer `player` with their guild now.
async fn answer(state: &AppState, player: Uuid, told: Told) -> Result<Json<MyGuild>, AppError> {
    tell(state, told).await;
    Ok(Json(guilds::mine(&state.db, player, &lots(state).await).await.map_err(refused)?))
}

/// The tax rate now and the world's addresses, for a guild's hall (#582)
pub(crate) async fn lots(state: &AppState) -> guilds::Lots {
    guilds::Lots { rate: state.tunables.get().property_tax_rate, addresses: state.game.read().await.addresses.clone() }
}

/// Sends a committed change's `guild_changed`, its notices and the payer's
/// balance ([`Told::send`]), once the tags over their heads are read again
/// (S4.4). A hall given up as the guild closed (S4.3) comes off the game first.
async fn tell(state: &AppState, mut told: Told) {
    // Their tags first, so the names over their heads change with the news
    guilds::retag(&state.db, &state.game, &told.changed).await;
    let events = {
        let mut game = state.game.write().await;
        let game = &mut *game;
        told.release_halls(&mut game.stations, &game.event_sender);
        game.event_sender.clone()
    };
    told.send(&state.db, &events).await;
}

/// What the registry refused, as the API says it.
fn refused(e: GuildError) -> AppError {
    match e {
        GuildError::NoGuild
        | GuildError::NoOne
        | GuildError::NotAMember
        | GuildError::NotInAGuild
        | GuildError::NoInvite
        | GuildError::NoLot => AppError::NotFound(e.message()),
        GuildError::Unsubscribed(_) | GuildError::NotAllowed(_) => AppError::Forbidden(e.message()),
        GuildError::NameTaken
        | GuildError::TagTaken
        | GuildError::InAGuild
        | GuildError::TheyreInAGuild
        | GuildError::AlreadyWaiting
        | GuildError::HasAHall
        | GuildError::LotTaken => AppError::Conflict(e.message()),
        GuildError::Db(e) => AppError::Database(e),
        e => AppError::BadRequest(e.message()),
    }
}

/// These need the local Postgres (`docker-compose up -d`). Run them with
/// `cargo test guilds -- --ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plot_sync::tests::player;
    use crate::models::{LandmarkKind, TerrainConfig, WorldGenParams};
    use capitol_protocol::rules::guild::FOUNDING_FEE;
    use capitol_protocol::rules::{SPAWN_X, SPAWN_Y};

    /// Founding happens at the Exchange hall: from where a new player
    /// arrives it's refused and costs nothing; outside its door it's done.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "needs the local Postgres (docker-compose up -d)"]
    async fn founding_needs_the_hall(db: PgPool) {
        let terrain = TerrainConfig::get_default();
        let door = terrain.landmark(LandmarkKind::ExchangeHall).expect("the default world has a hall").door;
        let game = RwLock::new(GameState::with_terrain(WorldGenParams::default(), terrain));
        let ada = player(&db, "Ada", 600).await;
        crate::subscriptions::extend(&db, sqlx::query_scalar("SELECT account_id FROM players WHERE id = $1").bind(ada).fetch_one(&db).await.unwrap(), 30)
            .await
            .unwrap();
        let save = |(x, y): (f64, f64)| {
            sqlx::query("UPDATE players SET position_x = $1, position_y = $2 WHERE id = $3")
                .bind(x)
                .bind(y)
                .bind(ada)
                .execute(&db)
        };
        let req = FoundGuild { name: "The Rope Walkers".into(), tag: "ROPE".into(), charter: String::new() };

        save((SPAWN_X, SPAWN_Y)).await.unwrap();
        let refused = found_at_the_hall(&game, &db, ada, &req).await;
        assert!(
            matches!(&refused, Err(AppError::BadRequest(m)) if m.contains("too far from the Exchange hall to found a guild")),
            "{refused:?}"
        );
        assert!(guilds::list(&db).await.unwrap().is_empty());

        save((SPAWN_X, door.1)).await.unwrap();
        let (id, _) = found_at_the_hall(&game, &db, ada, &req).await.unwrap();
        let balance: i64 = sqlx::query_scalar("SELECT strand_balance FROM players WHERE id = $1").bind(ada).fetch_one(&db).await.unwrap();
        assert_eq!(balance, 600 - FOUNDING_FEE);
        assert_eq!(guilds::mine(&db, ada, &guilds::Lots::default()).await.unwrap().guild.map(|g| g.id), Some(id));
    }

    #[test]
    fn refusals_carry_their_status() {
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        let status = |e| refused(e).into_response().status();
        assert_eq!(status(GuildError::NoGuild), StatusCode::NOT_FOUND);
        assert_eq!(status(GuildError::Unsubscribed("x".into())), StatusCode::FORBIDDEN);
        assert_eq!(status(GuildError::NotAllowed("x")), StatusCode::FORBIDDEN);
        assert_eq!(status(GuildError::NameTaken), StatusCode::CONFLICT);
        assert_eq!(status(GuildError::InAGuild), StatusCode::CONFLICT);
        assert_eq!(status(GuildError::RosterFee), StatusCode::BAD_REQUEST);
        assert_eq!(status(GuildError::LeaderLeaving), StatusCode::BAD_REQUEST);
    }
}
