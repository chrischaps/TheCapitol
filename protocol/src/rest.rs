//! Request and response bodies for the REST API, grouped by route.
//!
//! Paths are relative to the server root (`http://localhost:3000` locally).
//! Endpoints other than auth, health, terrain, catalog and bureaucracy need an
//! `Authorization: Bearer <token>` header, or `Authorization: ApiKey <secret>`
//! for a bot's API key, which reaches only what its scopes allow.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::rules;
use crate::rules::stacking::Units;

// ---------------------------------------------------------------------------
// Shared bodies
// ---------------------------------------------------------------------------

/// `{"success": true}`, returned by `POST /auth/logout` and `DELETE /stations/{id}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
}

impl SuccessResponse {
    pub fn ok() -> Self {
        Self { success: true }
    }
}

// ---------------------------------------------------------------------------
// /auth
// ---------------------------------------------------------------------------

/// `POST /auth/register`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RegisterRequest {
    /// Email address for the account
    pub email: String,
    /// Password (will be hashed with Argon2)
    pub password: String,
    /// Display name for the player character
    pub player_name: String,
}

/// `POST /auth/login`. Login and register answer a short access token
/// (`ACCESS_TTL_SECS`, 15 min by default) and a refresh token to renew it
/// with (E0.6). The `refresh` flag that once chose this over a 24 h token is
/// gone (#185). A client that still sends it is understood, since unknown
/// fields are ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct LoginRequest {
    /// Account email address
    pub email: String,
    /// Account password
    pub password: String,
}

/// `POST /auth/refresh`; responds with a new [`AuthResponse`] pair. The
/// refresh token can be used once: a second use revokes its whole family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// `POST /auth/logout`; responds with [`SuccessResponse`]. Give either token;
/// an expired access token still logs out.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct LogoutRequest {
    /// JWT access token of the session to end
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Refresh token of the session to end
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// End every session the account has, not just this one (E0.8)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub everywhere: bool,
}

/// Response to register, login and refresh
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct AuthResponse {
    /// JWT token for authentication
    pub token: String,
    /// Seconds until `token` expires
    #[serde(default)]
    pub expires_in: u64,
    /// Opaque, single-use token for `POST /auth/refresh`. This server always
    /// sends one (#185). It stays optional so a client copes with an answer
    /// that has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Account information
    pub account: AccountInfo,
    /// Player character ID
    pub player_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct AccountInfo {
    /// Unique account identifier
    pub id: Uuid,
    /// Email address
    pub email: String,
    /// The subscription status, as [`SubscriptionStatus`] names it (`free`,
    /// `active` or `lapsed`). The React client reads it as a tier.
    pub tier: String,
    /// Whether this account has admin privileges (debug features)
    pub is_admin: bool,
    /// A person's account or a citizen's (C0.1). Absent from a server older
    /// than #246, which had only people.
    #[serde(default)]
    pub kind: AccountKind,
}

/// Whose an account is (Citizens D3). Registration makes a `human`; only
/// `POST /admin/citizens` makes a `citizen`, and an account never changes
/// kind. Other players see a citizen marked (`citizen` on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    #[default]
    Human,
    /// An AI player who lives in the world, run by `capitol-town`
    Citizen,
}

impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountKind::Human => "human",
            AccountKind::Citizen => "citizen",
        }
    }

    pub fn parse(s: &str) -> Option<AccountKind> {
        match s {
            "human" => Some(AccountKind::Human),
            "citizen" => Some(AccountKind::Citizen),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// /account
// ---------------------------------------------------------------------------

/// How every API key's secret starts, which is how the server tells one from
/// an access token
pub const API_KEY_PREFIX: &str = "cap_";

/// What an API key may do (E0.9). `play` and `trade` imply `read`. `town`
/// implies nothing else: it's the town's key for its reports (D4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyScope {
    /// GETs and the WebSocket feed, watching without taking the player over
    Read,
    /// Moving, gathering, crafting, inventory and stations
    Play,
    /// Trades, the exchange and plots (and, from E3, the market)
    Trade,
    /// `POST /admin/town/report` and nothing else: what `capitol-town`
    /// reports with (D4.1). Only an admin mints one, at
    /// `POST /admin/town/key`.
    Town,
}

impl ApiKeyScope {
    pub fn as_str(self) -> &'static str {
        match self {
            ApiKeyScope::Read => "read",
            ApiKeyScope::Play => "play",
            ApiKeyScope::Trade => "trade",
            ApiKeyScope::Town => "town",
        }
    }

    pub fn parse(s: &str) -> Option<ApiKeyScope> {
        match s {
            "read" => Some(ApiKeyScope::Read),
            "play" => Some(ApiKeyScope::Play),
            "trade" => Some(ApiKeyScope::Trade),
            "town" => Some(ApiKeyScope::Town),
            _ => None,
        }
    }
}

/// `POST /account/api-keys`. Only a signed-in player can mint keys, not a key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CreateApiKeyRequest {
    /// A label for the owner's list, e.g. "fiber bot"
    pub name: String,
    /// At least one
    pub scopes: Vec<ApiKeyScope>,
}

/// One key, as `GET /account/api-keys` lists it. The secret is never shown again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ApiKeyInfo {
    pub id: Uuid,
    pub name: String,
    pub scopes: Vec<ApiKeyScope>,
    /// The secret's first characters, to tell keys apart
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    /// To the minute
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// The answer to `POST /account/api-keys`: the key, and its secret, shown
/// this once. Send it as `Authorization: ApiKey <secret>`, or as the
/// WebSocket `auth` message's `token`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CreatedApiKey {
    pub key: ApiKeyInfo,
    pub secret: String,
}

// ---------------------------------------------------------------------------
// /admin/citizens (C0.1)
// ---------------------------------------------------------------------------

/// `POST /admin/citizens`: make a citizen, its player and its key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CreateCitizenRequest {
    /// The player's name, as everyone sees it
    pub name: String,
}

/// A citizen, as `GET /admin/citizens` lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CitizenInfo {
    pub account_id: Uuid,
    pub player_id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    /// Its newest live key, if it has one. A citizen with none can't sign in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<ApiKeyInfo>,
}

/// `GET /admin/walkers`' query (D5.3): the players within `radius` of
/// (`x`, `y`), in server units. The radius is held to [`WALKERS_MAX_RADIUS`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema, utoipa::IntoParams))]
pub struct WalkersQuery {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
}

/// The farthest `GET /admin/walkers` looks: the interest grid's 3×3 cells
/// and a little over.
pub const WALKERS_MAX_RADIUS: f64 = 2000.0;

/// One player as the server is moving them (D5.3), for the admin's world
/// layers: where they stand, where the server is walking them (a citizen's
/// next leg, or a person's click), and what they're doing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Walker {
    pub player_id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
    pub x: f64,
    pub y: f64,
    /// Where it's walking to; `None` standing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dest: Option<[f64; 2]>,
    pub doing: Doing,
}

/// What a [`Walker`] is busy with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Doing {
    Idle,
    Extracting { node_id: Uuid },
    Crafting { recipe_id: String },
}

/// `GET /admin/books` (D2.1): how the console reads a sample's
/// `strand_flows`. The page holds no copy of what's a faucet and what's a
/// sink, so a type added here shows on its chart without touching it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct AdminBooks {
    /// Every transaction type this build knows, in the order they stack
    /// (`ledger::TRANSACTION_TYPES`)
    pub flows: Vec<BooksFlow>,
    /// What each new account starts with (`ledger::STARTING_STRANDS`)
    pub starting_strands: i64,
    /// A game day, in real ms: what velocity is counted over
    pub day_ms: i64,
}

/// One transaction type, and what it does to the world's Strands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct BooksFlow {
    pub kind: String,
    pub flow: crate::ledger::StrandFlow,
}

impl AdminBooks {
    /// This build's books
    pub fn current() -> AdminBooks {
        use crate::ledger::{StrandFlow, STARTING_STRANDS, TRANSACTION_TYPES};
        AdminBooks {
            flows: TRANSACTION_TYPES
                .iter()
                .map(|kind| BooksFlow { kind: kind.to_string(), flow: StrandFlow::of(kind) })
                .collect(),
            starting_strands: STARTING_STRANDS,
            day_ms: rules::DAY_LENGTH_SECS * 1000,
        }
    }
}

/// `GET /admin/map`'s query (D1.3): how many hours back the trade heat
/// reaches, held to 1 to [`MAP_HEAT_MAX_HOURS`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema, utoipa::IntoParams))]
pub struct MapQuery {
    #[serde(default = "MapQuery::default_hours")]
    pub hours: u32,
}

impl MapQuery {
    fn default_hours() -> u32 {
        1
    }
}

/// The farthest back `GET /admin/map`'s heat reaches: the console's longest range.
pub const MAP_HEAT_MAX_HOURS: u32 = 24;

/// `GET /admin/map` (D1.3): what the console's map draws over `/terrain`
/// and `/world/nodes`, besides the walkers (`GET /admin/positions`, read
/// far more often). Each is marked for a citizen's, as the Gazette does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct AdminMap {
    /// The zones' rings, centre out, from `rules::ZONE_OUTER_RADII`
    pub zones: Vec<MapZone>,
    /// Every owned lot, by its `/terrain` id
    pub plots: Vec<MapPlot>,
    pub stations: Vec<MapStation>,
    /// The trade over the last `hours`, by where it happened
    pub heat: FillHeat,
}

/// One zone's ring on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MapZone {
    /// `Zone::as_str`
    pub zone: String,
    pub inner: f64,
    pub outer: f64,
}

/// An owned lot on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MapPlot {
    pub terrain_id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
}

/// A placed station on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MapStation {
    pub id: Uuid,
    pub station_type: String,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
}

/// The trade since `from_ms` by place: the market's fills at the Exchange
/// hall, and the bazaar's sales at each stall (`/terrain`'s `bazaar_stall_*`
/// landmarks). A fill has no other place to be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct FillHeat {
    pub from_ms: i64,
    pub market: crate::metrics::Trades,
    /// By stall id; stalls that sold nothing are left out
    pub stalls: std::collections::BTreeMap<String, crate::metrics::Trades>,
}

/// `GET /admin/player`'s query (D4.3): the player called `name` (any case),
/// and how many hours back their Strands go, held to 1 to
/// [`MAP_HEAT_MAX_HOURS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema, utoipa::IntoParams))]
pub struct PlayerQuery {
    pub name: String,
    #[serde(default = "MapQuery::default_hours")]
    pub hours: u32,
}

/// The most points [`PlayerDetail::balance`] holds: past it, the range is cut
/// in this many even slices and each keeps its last.
pub const PLAYER_POINTS: usize = 720;
/// The moves [`PlayerDetail::recent`] holds, the newest.
pub const PLAYER_MOVES: usize = 20;

/// `GET /admin/player` (D4.3): one player, for the console's page of one
/// citizen. Their Strands over the range come from `currency_transactions`,
/// each move's balance after it, so they reach back past the server's start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlayerDetail {
    pub player_id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
    /// Their Strands in hand now
    pub strands: i64,
    /// Their Strands held in their buy orders' escrow now
    pub escrow: i64,
    /// Where the range starts (Unix ms)
    pub from_ms: i64,
    /// Their Strands when the range started: the balance before its first
    /// move, or [`strands`](Self::strands) if they made none in it
    pub opening: i64,
    /// Their balance after each move in the range, oldest first, at most
    /// [`PLAYER_POINTS`]
    pub balance: Vec<StrandPoint>,
    /// The moves in the range
    pub moves: u64,
    /// The range's moves added up by transaction type
    pub flows: std::collections::BTreeMap<String, i64>,
    /// Their newest moves, newest first, at most [`PLAYER_MOVES`], however
    /// long ago
    pub recent: Vec<StrandMove>,
    /// Where the server has them; `None` when they're not in the world
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walker: Option<Walker>,
}

/// A player's Strands at a moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StrandPoint {
    pub at_ms: i64,
    pub strands: i64,
}

/// One move of a player's Strands: a `currency_transactions` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StrandMove {
    pub at_ms: i64,
    /// Its transaction type, as `AdminBooks::flows` names them
    pub kind: String,
    pub amount: i64,
    /// Their Strands after it
    pub strands: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `POST /admin/citizens`'s answer. The key's secret (`play` and `trade`) is
/// shown this once: it's how `capitol-town` signs the citizen in, since a
/// citizen has no password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CreatedCitizen {
    pub citizen: CitizenInfo,
    pub secret: String,
}

// ---------------------------------------------------------------------------
// Subscriptions (E4.1)
// ---------------------------------------------------------------------------

/// Where an account stands. There are no payments yet: an admin, or the mock
/// activation on a development server, starts one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionStatus {
    /// Never subscribed
    #[default]
    Free,
    /// Subscribed until [`Subscription::until`]
    Active,
    /// Was subscribed, and it ran out
    Lapsed,
}

impl SubscriptionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SubscriptionStatus::Free => "free",
            SubscriptionStatus::Active => "active",
            SubscriptionStatus::Lapsed => "lapsed",
        }
    }

    /// The column's value. Anything else reads as `Free`.
    pub fn parse(s: &str) -> SubscriptionStatus {
        match s {
            "active" => SubscriptionStatus::Active,
            "lapsed" => SubscriptionStatus::Lapsed,
            _ => SubscriptionStatus::Free,
        }
    }
}

/// An account's subscription, in `/player/me` and `auth_ok`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Subscription {
    pub status: SubscriptionStatus,
    /// When it ends, or ended. `None` for an account that never subscribed.
    pub until: Option<DateTime<Utc>>,
}

impl Subscription {
    /// Whether it's running at `now`. One past its `until` has run out, even
    /// before the hourly check marks it lapsed.
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.status == SubscriptionStatus::Active && self.until.is_some_and(|until| until > now)
    }

    /// As it stands at `now`: an `Active` one past its `until` is `Lapsed`.
    pub fn as_of(self, now: DateTime<Utc>) -> Subscription {
        if self.status == SubscriptionStatus::Active && !self.is_active_at(now) {
            Subscription { status: SubscriptionStatus::Lapsed, ..self }
        } else {
            self
        }
    }
}

/// `POST /admin/accounts/{id}/subscription` and `POST /player/subscription`;
/// both respond with the [`Subscription`] as it now stands. The days are
/// added to what's left, or start now if nothing is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct SubscribeRequest {
    /// 1 to 3650
    pub days: i32,
}

// ---------------------------------------------------------------------------
// /health
// ---------------------------------------------------------------------------

/// `GET /health`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct HealthResponse {
    /// Overall health status: "healthy" or "degraded"
    pub status: String,
    /// Database connection status
    pub database: String,
    /// Redis connection status
    pub redis: String,
    /// The server's clock, in Unix milliseconds, for `rules::time_of_day`
    pub server_time_ms: i64,
    /// The `version` `GET /catalog` answers with now, so a client can tell
    /// its copy is stale without fetching it (E0.2). Absent from a server
    /// older than the catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_version: Option<String>,
    /// How many times real time the tick runs, when a dev server was started
    /// with `SIM_TICK_SCALE` (E2.7). Absent at real time. The economy sim
    /// reads it to turn its wall clock into game hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim_tick_scale: Option<f64>,
}

// ---------------------------------------------------------------------------
// /player
// ---------------------------------------------------------------------------

/// `POST /player/move`; responds with [`MoveResponse`]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MoveRequest {
    /// Destination X coordinate (0-13600, on the world disc)
    pub x: f64,
    /// Destination Y coordinate (0-13600, on the world disc)
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveResponse {
    pub success: bool,
    pub destination: Destination,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Destination {
    pub x: f64,
    pub y: f64,
}

/// `GET /player/me`: the token's player, and where they are now
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlayerMeResponse {
    pub player_id: Uuid,
    pub name: String,
    /// Live X while `online`, otherwise the last saved one (0-13600)
    pub x: f64,
    /// Live Y while `online`, otherwise the last saved one (0-13600)
    pub y: f64,
    /// Zone ID at that position, as in `zone_changed` (e.g. "rural")
    pub zone: String,
    /// Current Strand balance
    pub strand_balance: i64,
    /// Whether a WebSocket session has the player in the game. When false,
    /// `x`/`y` come from the database, which the server writes every 10 s.
    pub online: bool,
    /// The account's subscription, as of now (E4.1)
    #[serde(default)]
    pub subscription: Subscription,
    /// You're a citizen (C0.1), and marked as one to everyone else
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
}

/// `GET /players/{id}/card` and `GET /players/by-name/{name}` (S0.3): who
/// someone is, as anyone may see it. Chat, mutes, letters, standing and
/// guilds all open it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlayerCard {
    pub id: Uuid,
    pub name: String,
    /// A citizen (C0.1), marked as one to everyone
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
    /// The game day their account was made (`rules::day_number`)
    pub since: i64,
    /// The zone of their oldest plot (`Zone::as_str`), if they hold one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// What the city says of them, in words (D8): `rules::standing`
    pub standing: String,
    /// Their guild's name, once there are guilds (S4.1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild: Option<String>,
    /// The card is your own
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mine: bool,
    /// You've muted them (S1.4): nothing they say reaches you
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
}

/// `GET /player/mutes` (S1.4): the people you've muted, newest first.
/// `PUT` and `DELETE /players/{id}/mute` answer with the list as it stands
/// after them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MutesResponse {
    pub mutes: Vec<MutedPlayer>,
}

/// Someone you've muted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MutedPlayer {
    pub id: Uuid,
    pub name: String,
    /// A citizen (C0.1): they can be muted like anyone
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
}

/// Which of your two boxes of letters (S1.5): what came for you, or what
/// you sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub enum LetterBox {
    #[default]
    In,
    Out,
}

/// `POST /letters` (S1.5): a letter to the player `to`, for
/// [`rules::talk::POSTAGE`]. It's addressed by id, from a card's Write or a
/// letter's Reply, since names aren't unique
/// ([#428](https://github.com/chrischaps/TheCapitol/issues/428)). The body
/// is cleaned by [`rules::talk::clean_letter`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct SendLetter {
    pub to: Uuid,
    pub body: String,
}

/// `GET /letters`' query (S1.5): which box, and the page after the last.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LettersQuery {
    /// `in`, the default, or `out`
    #[serde(default, rename = "box")]
    pub which: LetterBox,
    /// The letters sent before this one: the last of the page before
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Uuid>,
}

/// One end of a letter: who wrote it, or who it's to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Correspondent {
    pub id: Uuid,
    pub name: String,
    /// A citizen (C0.1)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
}

/// A letter, as its box shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Letter {
    pub id: Uuid,
    pub from: Correspondent,
    pub to: Correspondent,
    pub body: String,
    pub sent_at: DateTime<Utc>,
    /// When you opened it. Only in your in box: a writer isn't told whether
    /// a letter was read, so a mute (S1.4) never shows as one never read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_at: Option<DateTime<Utc>>,
}

/// `GET /letters?box=in|out&before=` (S1.5): a page of one box, newest
/// first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Mailbox {
    pub letters: Vec<Letter>,
    /// Letters unread in your in box, whichever box this is
    pub unread: u32,
    /// There are older letters: ask again with `before` the last one's id
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
}

/// `POST /letters/{id}/read` and `DELETE /letters/{id}` (S1.5): your in
/// box's unread letters now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct LettersUnread {
    pub unread: u32,
}

/// `GET /player/inventory`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct InventoryResponse {
    /// All items owned by the player
    pub items: Vec<InventoryItem>,
}

/// Client-facing inventory item
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct InventoryItem {
    /// Unique item instance ID
    pub id: Uuid,
    /// Item type identifier (e.g., "fiber", "rope")
    pub item_type: String,
    /// Display name of the item
    pub name: String,
    /// Quality value (0-100)
    pub quality: i32,
    /// Quality grade letter (A, B, C, D, F)
    pub quality_grade: String,
    /// Stack quantity
    pub quantity: i32,
    /// A tool's durability left, and its most (E1.4). Absent for anything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durability: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_durability: Option<i32>,
}

/// `GET /player/currency`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CurrencyResponse {
    /// Current Strand balance
    pub strand_balance: i64,
}

// ---------------------------------------------------------------------------
// /world
// ---------------------------------------------------------------------------

/// `GET /world/nearby` query string
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct NearbyQuery {
    /// Search radius around the player (default `NEARBY_NODE_RANGE`, 200;
    /// capped at `NEARBY_NODE_RANGE_MAX`, 800)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
}

impl NearbyQuery {
    /// The radius the server searches: the one asked for, cut to
    /// `0..=NEARBY_NODE_RANGE_MAX`, or the default when none (or NaN) is given.
    pub fn effective_radius(&self) -> f64 {
        match self.radius {
            Some(r) if !r.is_nan() => r.clamp(0.0, rules::NEARBY_NODE_RANGE_MAX),
            _ => rules::NEARBY_NODE_RANGE,
        }
    }
}

/// `GET /world/nearby`, and `GET /world/nodes` with every node
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct NearbyResponse {
    /// Resource nodes within range of the player (or all of them)
    pub nodes: Vec<ResourceNodeInfo>,
}

/// Client-facing resource node data
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ResourceNodeInfo {
    /// Unique node identifier
    pub id: Uuid,
    /// Type of resource (e.g., "fiber_plant")
    pub resource_type: String,
    /// X position in world coordinates
    pub x: f64,
    /// Y position in world coordinates
    pub y: f64,
    /// Current state: "available", "harvesting", or "depleted"
    pub state: String,
}

/// `POST /world/extract`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ExtractRequest {
    /// ID of the resource node to extract from
    pub node_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ExtractResponse {
    /// Whether extraction was started successfully
    pub success: bool,
    /// Optional message (e.g., error reason)
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// /inventory
// ---------------------------------------------------------------------------

/// `GET /inventory/containers`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ContainersResponse {
    /// All containers owned by the player
    pub containers: Vec<ContainerResponse>,
}

/// Container state for API response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ContainerResponse {
    /// Container instance ID
    pub id: Uuid,
    /// Container type identifier
    pub container_type: String,
    /// Total number of slots
    pub slot_count: i32,
    /// Number of columns for UI layout
    pub layout_columns: i32,
    /// Items in each slot
    pub slots: Vec<SlotItem>,
}

/// Item in a specific slot (for API responses)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct SlotItem {
    /// Unique item instance ID
    pub item_id: Uuid,
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Quality value (0-100)
    pub quality: i32,
    /// Quality grade letter (A, B, C, D, F)
    pub quality_grade: String,
    /// Stack quantity
    pub quantity: i32,
    /// The lowest quality of any unit in the stack (E1.2, E1.8): the least of
    /// `units`. Its grade below `quality_grade` marks a stack that spans grades.
    pub min_quality: i32,
    /// Every unit, as `[quality, count]` pairs (E1.8). `quality` is their
    /// mean, rounded down, and `quantity` their count.
    pub units: Units,
    /// Slot index within the container
    pub slot_index: i32,
    /// A tool's durability left, and its most (E1.4). Absent for anything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durability: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_durability: Option<i32>,
    /// Who made it (#242), when every unit carries the same maker's mark and
    /// their byline is on. Absent for a stack with no mark or mixed marks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker: Option<Maker>,
}

/// A stack's maker, as its tooltip names them (#242). Set only when every
/// unit carries this one maker's mark (#231) and their byline is on
/// (`players.gazette_named`, as in the Gazette).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Maker {
    pub name: String,
    /// The maker is a citizen (C0.1)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
}

impl SlotItem {
    /// A slot item, with its grade derived from `quality`.
    pub fn new(
        item_id: Uuid,
        item_type: String,
        item_name: String,
        quality: i32,
        quantity: i32,
        slot_index: i32,
    ) -> Self {
        Self {
            item_id,
            item_type,
            item_name,
            quality,
            quality_grade: rules::quality_grade(quality).to_string(),
            quantity,
            min_quality: quality,
            units: Units::uniform(quantity, quality),
            slot_index,
            durability: None,
            max_durability: None,
            maker: None,
        }
    }

    /// This slot item holding `units`: its quantity, quality, grade and min
    /// follow from them. Empty units change nothing.
    pub fn with_units(mut self, units: Units) -> Self {
        let (Some(quality), Some(min)) = (units.mean(), units.min()) else {
            return self;
        };
        self.quantity = units.count().min(i32::MAX as i64) as i32;
        self.quality = quality;
        self.quality_grade = rules::quality_grade(quality).to_string();
        self.min_quality = min;
        self.units = units;
        self
    }

    /// This slot item made by `maker`, if every unit has the one.
    pub fn with_maker(mut self, maker: Option<Maker>) -> Self {
        self.maker = maker;
        self
    }

    /// This slot item as a tool with `durability` of `max_durability` left.
    pub fn with_durability(mut self, durability: i32, max_durability: i32) -> Self {
        self.durability = Some(durability);
        self.max_durability = Some(max_durability);
        self
    }
}

/// `POST /items/{id}/split`: split `quantity` off the stack into the empty
/// slot `to_slot` of `to_container_id`, or of the stack's own container
/// without one (E1.2, #190). A split never merges. `qualities` names which units
/// (E1.8), as `[quality, count]` pairs that add up to `quantity`; without it
/// the lowest go first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct SplitStackRequest {
    pub quantity: i32,
    pub to_slot: i32,
    /// Another container to split into, checked as a move's target is (#190)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_container_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualities: Option<Units>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct SplitStackResponse {
    /// The new stack, in `to_slot`
    pub item: SlotItem,
    /// The container the new stack is in: the request's `to_container_id`,
    /// or the source's own
    pub container_id: Uuid,
}

/// `POST /inventory/move`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MoveItemRequest {
    /// ID of the item to move
    pub item_id: Uuid,
    /// Target container ID
    pub target_container_id: Uuid,
    /// Target slot index
    pub target_slot: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MoveItemResponse {
    /// Whether the operation succeeded
    pub success: bool,
    /// Action taken: "moved", "merged", or "swapped"
    pub action: String,
    /// New quantity if items were merged
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_quantity: Option<i32>,
    /// New quality if items were merged
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_quality: Option<i32>,
}

// ---------------------------------------------------------------------------
// /recipes
// ---------------------------------------------------------------------------

/// `GET /recipes`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RecipesResponse {
    /// Available crafting recipes
    pub recipes: Vec<RecipeInfo>,
}

/// Client-facing recipe info for UI
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RecipeInfo {
    /// Unique recipe identifier
    pub id: String,
    /// Display name of the recipe
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Recipe category (e.g., "hand_crafting")
    pub category: String,
    /// Required input items
    pub inputs: Vec<RecipeInputInfo>,
    /// Produced output items
    pub outputs: Vec<RecipeOutputInfo>,
    /// Base crafting duration in ticks
    pub duration_ticks: i32,
    /// Station type required (null for hand crafting)
    pub required_station_type: Option<String>,
    /// The quality cascade's weights, defaults filled and summing to 1, so a
    /// client gets the server's range from `rules::quality::output_range`
    pub quality_weights: rules::quality::Weights,
    /// The tool the recipe needs (`rules::tools::ToolType`), from the crafter's
    /// inventory: the best one supplies the tool term and wears (E1.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RecipeInputInfo {
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Required quantity
    pub quantity: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RecipeOutputInfo {
    /// Item type identifier
    pub item_type: String,
    /// Display name of the item
    pub item_name: String,
    /// Minimum output quantity
    pub quantity_min: i32,
    /// Maximum output quantity
    pub quantity_max: i32,
}

// ---------------------------------------------------------------------------
// /catalog
// ---------------------------------------------------------------------------

/// `GET /catalog` (no auth): the static game data, as the server loaded it at
/// startup. Every list is sorted by `id`, so the same content always gives
/// the same bytes and the same `version`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CatalogResponse {
    /// A hash of everything below. It changes exactly when the content does,
    /// and `GET /health` reports it as `catalog_version`.
    pub version: String,
    pub item_types: Vec<CatalogItemType>,
    pub station_types: Vec<CatalogStationType>,
    pub resource_types: Vec<CatalogResourceType>,
    pub container_types: Vec<CatalogContainerType>,
    /// Every recipe, hand and station alike.
    pub recipes: Vec<RecipeInfo>,
}

/// An item type (`item_types`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CatalogItemType {
    pub id: String,
    pub name: String,
    /// `raw_material`, `processed_material` or `placeable`
    pub category: String,
    /// Stackable items merge into one slot; kits and tools don't.
    pub stackable: bool,
    pub weight: f32,
    /// Strands a unit the economy is balanced to (D2.3): D5's value chain,
    /// where a market maker's quotes start before the item has traded, and
    /// what the Observatory judges the market's price against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_price: Option<i64>,
    /// Set when the item is a tool (E1.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<CatalogTool>,
}

/// What makes an item type a tool (`item_types.tool_type` and the rest).
/// `rules::tools` turns these into a tool's power and lifespan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CatalogTool {
    /// `sickle`, `axe`, `pickaxe`, `hammer` or `saw` (`rules::tools::ToolType`)
    pub tool_type: String,
    /// The material: `stone`, `bronze`, `iron` or `steel` (`rules::tools::Tier`)
    pub tier: String,
    /// Power at Q50; quality scales it from half to one and a half times.
    pub base_power: f64,
    /// Uses at Q50; quality scales it the same way.
    pub max_durability: i32,
}

/// A station type (`station_types`), with the kit that places it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CatalogStationType {
    pub id: String,
    pub name: String,
    /// `crafting` or `storage`
    pub category: String,
    pub slot_count: i32,
    pub layout_columns: i32,
    pub interaction_range: f32,
    pub icon: Option<String>,
    pub description: Option<String>,
    /// The item type a placement consumes. Clients pair kits and stations
    /// from this alone (#181).
    pub kit: Option<String>,
    /// Whether it may stand off a plot, on dry ground no lot covers (#187).
    /// Omitted when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub off_plot: bool,
}

/// A resource type (`resource_types`): what a node yields and how fast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CatalogResourceType {
    pub id: String,
    pub name: String,
    /// The item type an extraction yields
    pub yields_item: String,
    pub yield_quantity_min: i32,
    pub yield_quantity_max: i32,
    pub base_quality_min: i32,
    pub base_quality_max: i32,
    /// The tool type (`rules::tools::ToolType`) without which it can't be
    /// gathered, or null to gather by hand. Since E1.5 a tool type, not an item.
    pub required_tool: Option<String>,
    pub extraction_duration_ticks: i32,
    pub regeneration_ticks: i32,
    /// The tool type that works it, if any: the best one held speeds the
    /// gather, reads its tier's row of `yields`, and wears.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    /// How hard it is to work, on the tools' power scale: hands are 4, a Q50
    /// stone tool 10, iron 18 (`rules::tools::leverage`, plan D5).
    #[serde(default)]
    pub difficulty: f64,
    /// Units a gather gives, by `hand` or tool tier (`rules::resources::yield_for`).
    /// `yield_quantity_min`/`_max` above are its `hand` row, or its lowest
    /// tier's for a resource that needs a tool.
    #[serde(default)]
    pub yields: rules::resources::YieldTable,
    /// How its nodes group: `scattered`, `clumped`, `woods`, `along_water`
    /// or `under_trees` (`rules::resources::Cluster`)
    #[serde(default)]
    pub cluster: String,
    /// The zones its nodes grow in, and how thickly (E2.2)
    #[serde(default)]
    pub spawns: Vec<rules::resources::Spawn>,
}

/// A container type (`container_types`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CatalogContainerType {
    pub id: String,
    pub name: String,
    pub slot_count: i32,
    pub layout_columns: i32,
}

// ---------------------------------------------------------------------------
// /stations
// ---------------------------------------------------------------------------

/// `GET /stations/nearby`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct NearbyStationsResponse {
    /// Stations within visibility range
    pub stations: Vec<StationInfo>,
}

/// Client-facing station info
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StationInfo {
    /// Unique station instance ID
    pub id: Uuid,
    /// Station type identifier (e.g., "workbench", "forge")
    pub station_type: String,
    /// Display name of the station
    pub name: String,
    /// Station category ("crafting" or "storage")
    pub category: String,
    /// Optional icon identifier
    pub icon: Option<String>,
    /// X position in world coordinates
    pub x: f64,
    /// Y position in world coordinates
    pub y: f64,
    /// Player who owns this station
    pub owner_id: Uuid,
    /// Container ID for station inventory
    pub container_id: Option<Uuid>,
    /// Maximum interaction distance
    pub interaction_range: f32,
    /// The station's quality, 0-100: the quality of the kit that placed it.
    /// It's the station term of the quality cascade for crafts made here.
    pub quality: i32,
    /// The tool in the station's tool slot, if any (E1.6). It supplies the tool
    /// term for crafts made here, and wears with each one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<StationTool>,
    /// What the station is set to make on its own, and how far it's got
    /// (E4.2). Left out when it was never configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<StationAutomation>,
    /// The station's last crafts on its own, newest last (E4.3): up to
    /// [`PRODUCTION_LOG_LEN`], kept in the server's memory, so a restart
    /// starts it again. Left out when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub production: Vec<Produced>,
}

/// How many crafts a station's production log keeps (E4.3).
pub const PRODUCTION_LOG_LEN: usize = 50;

/// One craft a station made on its own (E4.3), for its production log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Produced {
    pub item_type: String,
    pub quantity: i32,
    pub quality: i32,
    pub at: DateTime<Utc>,
}

/// A station's automation (E4.2, TDD §4.3): one recipe, made again and again
/// from the station's own box, into the same box (v1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StationAutomation {
    pub recipe_id: String,
    pub mode: AutomationMode,
    /// `"state"`, with `"reason"` when it's paused
    #[serde(flatten)]
    pub state: AutomationState,
    /// Crafts finished since the last start
    pub done: u32,
}

impl StationAutomation {
    /// Has a `count` run made all it was asked for?
    pub fn is_finished(&self) -> bool {
        matches!(self.mode, AutomationMode::Count(n) if self.done >= n)
    }
}

/// How long it runs: `{"count": n}` crafts, or `"forever"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AutomationMode {
    Count(u32),
    Forever,
}

/// Whether it's working. `stopped` is the owner's doing, `paused` the
/// station's: it means to run, but can't yet (E4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AutomationState {
    Stopped,
    Running,
    Paused { reason: PauseReason },
}

/// Why a running station waits (TDD §4.3). Stopping it by hand is `stopped`,
/// not a pause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseReason {
    /// Its box lacks an input
    MissingInput { item_type: String },
    /// No room in its box for the output
    OutputFull,
    /// Its tool broke on the last craft
    ToolBroken,
    /// The recipe needs a tool and the slot holds none of its type
    ToolMissing,
    /// The owner left and isn't a subscriber (D8)
    OwnerOffline,
}

/// `PUT /stations/{id}/automation`. Setting it stops the station and starts
/// the count again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ConfigureAutomation {
    pub recipe_id: String,
    pub mode: AutomationMode,
}

/// A tool installed in a station (E1.6)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StationTool {
    pub item_id: Uuid,
    pub item_type: String,
    pub quality: i32,
    pub durability: i32,
    pub max_durability: i32,
    /// Who put it there, and whose it is when it comes out
    pub owner_id: Option<Uuid>,
}

/// `POST /stations/place`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlaceStationRequest {
    /// Station type to place (workbench, forge, storage_chest)
    pub station_type: String,
    /// X position in world coordinates
    pub x: f64,
    /// Y position in world coordinates
    pub y: f64,
    /// Kit item ID to consume
    pub kit_item_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlaceStationResponse {
    /// The newly placed station
    pub station: StationInfo,
}

/// `GET /stations/{id}/container`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StationContainerResponse {
    /// Container ID
    pub id: Uuid,
    /// Container type
    pub container_type: String,
    /// Total slots
    pub slot_count: i32,
    /// UI layout columns
    pub layout_columns: i32,
    /// Items in slots
    pub slots: Vec<SlotItem>,
}

// ---------------------------------------------------------------------------
// /exchange
// ---------------------------------------------------------------------------

/// `POST /exchange/deposit`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DepositRequest {
    /// Item IDs to deposit (must be fiber items)
    pub item_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DepositResponse {
    /// Total fiber quantity deposited
    pub deposited_quantity: i32,
    /// Exchange fee (0.5%)
    pub fee: i64,
    /// Net Strands received after fee
    pub strands_received: i64,
    /// New Strand balance
    pub new_balance: i64,
}

/// `POST /exchange/withdraw`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct WithdrawRequest {
    /// Amount of fiber to withdraw
    pub quantity: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct WithdrawResponse {
    /// Total Strands spent (including fee)
    pub strands_spent: i64,
    /// Exchange fee (0.5%)
    pub fee: i64,
    /// Fiber items received
    pub fiber_received: i32,
    /// New Strand balance
    pub new_balance: i64,
}

// ---------------------------------------------------------------------------
// /market (E3, plan D7)
// ---------------------------------------------------------------------------

/// Placing an order (E3.1; the routes are E3.3). A sell names the stack it
/// sells from, and which of its units; a buy names the grade it wants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlaceOrder {
    pub side: rules::market::OrderSide,
    pub item_type: String,
    /// Strands per unit
    pub price: i64,
    pub quantity: i32,
    /// Sell: the stack the units come from, in your inventory or market box
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<Uuid>,
    /// Sell: which units, as `[quality, count]` pairs adding up to
    /// `quantity` ("my six 92s"). Without it the lowest go first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualities: Option<Units>,
    /// Buy: the lowest grade it takes, A to F. Without it, any (F).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_grade: Option<String>,
    /// Buy: how `min_grade` reads the units sold. Without it, `average`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade_rule: Option<rules::market::GradeRule>,
}

/// An order, as its owner sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MarketOrder {
    pub id: Uuid,
    pub player_id: Uuid,
    pub side: rules::market::OrderSide,
    pub item_type: String,
    /// Strands per unit
    pub price: i64,
    pub quantity_total: i32,
    pub quantity_remaining: i32,
    pub status: rules::market::OrderStatus,
    /// Buy: the lowest grade it takes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_grade: Option<String>,
    /// Buy: how `min_grade` reads the units sold
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade_rule: Option<rules::market::GradeRule>,
    /// Sell: the units still in escrow, which a fill hands over
    #[serde(default, skip_serializing_if = "Units::is_empty")]
    pub units: Units,
    /// Buy: the Strands still in escrow
    #[serde(default)]
    pub escrow_strands: i64,
    /// The listing fee it paid (1%, not refunded)
    pub fee: i64,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// One fill: a sell's units traded to a buy, at the resting order's price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MarketTrade {
    pub id: Uuid,
    pub buy_order_id: Uuid,
    pub sell_order_id: Uuid,
    pub item_type: String,
    /// The grade of `quality`
    pub grade: String,
    /// The mean of the units traded, rounded down
    pub quality: i32,
    /// The lowest of them
    pub min_quality: i32,
    /// Strands per unit
    pub price: i64,
    pub quantity: i32,
    pub at: DateTime<Utc>,
}

/// One level of an item's book (E3.3): the orders on one side at one price
/// and grade, added up.
///
/// A buy's level is its `min_grade` and `grade_rule`. A sell's units can span
/// grades, so a sell counts at the grade of each of its units: a lot of six
/// 92s and four 70s is 6 at A and 4 at C. Units keep their own quality (D2),
/// and the ladder shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DepthLevel {
    pub side: rules::market::OrderSide,
    /// Strands per unit
    pub price: i64,
    /// A to F
    pub grade: String,
    /// Buy: how its orders read the grade
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade_rule: Option<rules::market::GradeRule>,
    /// Units at this level. In a diff, 0 means the level is gone.
    pub quantity: i64,
    /// Orders with units at this level
    pub orders: i32,
}

/// `GET /market/orders` query string: an item's book.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct MarketDepthQuery {
    /// The item type
    pub item: String,
    /// Only this side; both without it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<rules::market::OrderSide>,
}

/// An item's book, aggregated by price and grade: bids best first, then asks
/// best first. `seq` counts its changes, as the `market_depth` diffs do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MarketDepthResponse {
    pub item_type: String,
    pub seq: u64,
    pub levels: Vec<DepthLevel>,
}

/// `GET /market/orders/mine` query string.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct MyOrdersQuery {
    /// With closed orders too (filled, cancelled, expired): the latest 100.
    /// Without it, only the open ones.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
}

/// `GET /market/history` query string.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct MarketHistoryQuery {
    /// The item type
    pub item: String,
    /// `1m`, `5m`, `15m`, `1h` (the default), `4h` or `1d`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    /// From when; without it, 7 days ago. At most 500 buckets come back, the
    /// latest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    /// Only trades of this grade (by their mean)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
}

/// One bucket of trades. Buckets with no trades aren't sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Candle {
    /// The bucket's start
    pub at: DateTime<Utc>,
    pub open: i64,
    pub high: i64,
    pub low: i64,
    pub close: i64,
    /// Units traded
    pub volume: i64,
    pub trades: i32,
}

/// `GET /market/history`: an item's trades, in buckets, oldest first, then
/// the latest of them one by one, and yours (E3.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MarketHistoryResponse {
    pub item_type: String,
    pub bucket_secs: i64,
    pub candles: Vec<Candle>,
    /// The latest trades since `since`, of the grade if one was asked for,
    /// newest first: at most 50
    #[serde(default)]
    pub recent: Vec<MarketTrade>,
    /// Your fills since `since`, of the grade if one was asked for, newest
    /// first: at most 200
    #[serde(default)]
    pub mine: Vec<MyFill>,
}

/// One of your fills: the trade, and which side of it was yours. The matcher
/// never trades you with yourself, so only one is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MyFill {
    pub side: rules::market::OrderSide,
    pub trade: MarketTrade,
}

/// One item with activity: traded in the last 7 days, or on the book now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MarketItemSummary {
    pub item_type: String,
    /// The latest trade's price
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_price: Option<i64>,
    /// `last_price` less the last price 24 h ago (or the first since, if it
    /// first traded within the day)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_24h: Option<i64>,
    /// Units traded in the last 24 h
    pub volume_24h: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_bid: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_ask: Option<i64>,
}

/// `GET /market/summary`: every item with activity, by item type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MarketSummaryResponse {
    pub items: Vec<MarketItemSummary>,
}

/// `POST /market/box/collect`: what to take from the market box to the
/// inventory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CollectRequest {
    /// These stacks; every stack without it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_ids: Option<Vec<Uuid>>,
}

/// What a collect moved. It stops when the inventory is full.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct CollectResponse {
    /// The stacks moved (one that joined a stack in the inventory is gone)
    pub collected: Vec<Uuid>,
    /// Stacks still in the market box, or the impound (#141)
    pub remaining: i32,
}

// ---------------------------------------------------------------------------
// The bazaar (E3.10, plan D6)
// ---------------------------------------------------------------------------

/// `GET /bazaar`: every stall, and the shop in it if one's rented. It works
/// from anywhere, so it says which shop is worth the walk, but not prices,
/// quantities or grades: those are seen at the stall ([`BazaarShop`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct BazaarDirectory {
    /// In the order of the stalls' ids
    pub stalls: Vec<BazaarStall>,
}

/// A stall as the directory shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct BazaarStall {
    /// The stall's landmark id on `/terrain`, such as `bazaar_stall_3`
    pub stall_id: String,
    /// The shop in it, or none if it's free to rent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shop: Option<ShopSign>,
}

/// What a shop shows from across the city: its name, its merchant and the
/// kinds of goods it has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ShopSign {
    pub name: String,
    pub owner_id: Uuid,
    pub owner_name: String,
    /// The merchant is a citizen (C0.1)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owner_citizen: bool,
    /// The item types it lists, sorted, each once
    pub item_types: Vec<String>,
}

/// A shop as you see it at its stall (`GET /bazaar/stalls/{id}`), or your own
/// from anywhere (`GET /bazaar/mine`): every listing with its price and units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct BazaarShop {
    pub stall_id: String,
    pub name: String,
    pub owner_id: Uuid,
    pub owner_name: String,
    /// The merchant is a citizen (C0.1)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owner_citizen: bool,
    /// When the rent runs out. The shop closes then and its goods go to the
    /// merchant's market box.
    pub paid_until: DateTime<Utc>,
    /// Oldest first
    pub listings: Vec<BazaarListing>,
}

/// One listing: a stack at a fixed price per unit. Its units keep their own
/// quality (D2); the shop shows each, never an average.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct BazaarListing {
    pub id: Uuid,
    pub item_type: String,
    /// Strands per unit
    pub price: i64,
    pub units: Units,
    /// A tool's wear, if it's one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durability: Option<i32>,
    pub listed_at: DateTime<Utc>,
    /// Who made it, if every unit has the one maker (#242)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker: Option<Maker>,
}

/// `GET /bazaar/mine`: your stall, if you rent one, and goods from a lapsed
/// stall still waiting for room in your market box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct MyStallResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shop: Option<BazaarShop>,
    /// Unsold goods from a stall whose rent ran out while your market box was
    /// full. They move there as soon as it has room.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub returning: Vec<BazaarListing>,
}

/// `POST /bazaar/stalls/{id}/rent`: rent a free stall, or pay more on your own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct RentStall {
    /// Days paid ahead, 1 to 7 (`rules::bazaar::MAX_RENT_DAYS`). On your own
    /// stall they're added, up to 7 days from now.
    pub days: i64,
    /// The shop's name: needed to rent, optional to extend (it renames)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `POST /bazaar/mine/listings`: stock your shop with units from a stack in
/// your inventory, at a price per unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct StockListing {
    pub item_id: Uuid,
    pub quantity: i32,
    /// Strands per unit
    pub price: i64,
    /// Exactly which units, adding up to `quantity`. Without them it's the
    /// stack's default pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualities: Option<Units>,
}

/// `POST /bazaar/listings/{id}/buy`: buy units of a listing at its price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct BuyListing {
    pub quantity: i32,
    /// Exactly which units, adding up to `quantity`. Without them it's the
    /// best the listing has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualities: Option<Units>,
}

/// What a bazaar purchase moved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct BazaarPurchase {
    /// The units bought
    pub units: Units,
    /// Strands paid to the merchant
    pub paid: i64,
    /// The stack they're in now
    pub item_id: Uuid,
    /// Where it is: `player_inventory`, or `market_box` if the inventory was full
    pub into: String,
    /// The listing as it stands now, or none if that sold it out
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing: Option<BazaarListing>,
}

/// `POST /player/grant` (E3.8): Strands and goods from nowhere, to fund a
/// market maker on a local stack. The server refuses it unless `DEV_GRANTS`
/// is on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GrantRequest {
    /// Added to the balance, 0 to [`MAX_GRANT_STRANDS`]
    #[serde(default)]
    pub strands: i64,
    /// Put in the market box, at most [`MAX_GRANT_LINES`] lines
    #[serde(default)]
    pub items: Vec<GrantItem>,
}

/// The most Strands one grant gives.
pub const MAX_GRANT_STRANDS: i64 = 1_000_000;
/// The most lines one grant has.
pub const MAX_GRANT_LINES: usize = 32;

/// Units of one item type, all at one quality, for [`GrantRequest`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GrantItem {
    pub item_type: String,
    /// 1 to 10,000
    pub quantity: i32,
    /// 0 to 100
    pub quality: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GrantResponse {
    /// The balance after the grant
    pub strands: i64,
    /// Where the goods went
    pub market_box: Uuid,
}

// ---------------------------------------------------------------------------
// /gazette (E3.9)
// ---------------------------------------------------------------------------

/// One edition of the Capitol Gazette: the story of one in-game day, composed
/// at its end (the next midnight) from the day's trades, deeds and work.
/// Every section has its prose, even on a quiet day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Gazette {
    /// The day it's about (`rules::day_number`)
    pub day: i64,
    /// Its number on the masthead (`rules::gazette_number`)
    pub number: i64,
    pub headline: String,
    /// The line under the headline
    pub dek: String,
    /// The front-page story
    pub lead: String,
    pub weather: String,
    /// From the Exchange: what moved, and how the day went there
    pub markets: String,
    /// The biggest movers by price, the biggest first
    pub movers: Vec<GazetteMover>,
    /// Every item that traded, by name: the small price table
    pub prices: Vec<GazettePrice>,
    /// The highest-quality sale of the day
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_sale: Option<GazetteSale>,
    pub best_sale_story: String,
    /// Deeds and foreclosures by zone, from the centre out
    pub deeds: Vec<GazetteDeeds>,
    pub deeds_story: String,
    /// Who made the most, by units gathered and crafted
    pub producers: Vec<GazetteProducer>,
    pub producers_story: String,
    /// What the city made most of, with how much of it people made (C2.2)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub made: Vec<GazetteMade>,
    /// How much of the day was people's and how much citizens' (C2.2).
    /// `None` on a day no citizen made, sold or claimed anything, which
    /// reads as it did before there were citizens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shares: Option<GazetteShares>,
    /// The day at the bazaar's stalls (#240): how much sold and the busiest
    /// shops, but no prices, which the bazaar keeps between buyer and seller
    /// (D6). `None` on a day nothing sold there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stalls: Option<GazetteStalls>,
    /// Empty only in editions from before the Gazette counted the stalls
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stalls_story: String,
}

/// A day's selling at the bazaar's stalls (#240). It has counts, not
/// prices: what a unit fetched at a stall stays between buyer and seller
/// (D6), so the walk down the lane is still how you compare.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteStalls {
    /// Purchases, and the units they took
    pub sales: i64,
    pub units: i64,
    /// Strands spent at every stall together
    pub strands: i64,
    /// How many kinds of goods sold
    pub goods: i64,
    /// The busiest shops, by what they took, the busiest first
    pub shops: Vec<GazetteShop>,
}

/// One of the day's busiest shops at the bazaar.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteShop {
    /// The shop's name and its keeper's, both only if the keeper lets the
    /// Gazette print their name: a shop's sign often says whose it is
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper: Option<String>,
    /// The keeper is a citizen (C0.1), named or not
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
    pub sales: i64,
    pub units: i64,
    /// What it sold most of, by units
    pub item_type: String,
    pub item_name: String,
}

/// A day's price for one item at the Exchange: its trades' range and their
/// volume-weighted mean, against the day before's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazettePrice {
    pub item_type: String,
    pub item_name: String,
    pub low: i64,
    pub high: i64,
    /// Strands a unit, weighted by volume
    pub mean: i64,
    /// Units traded
    pub volume: i64,
    /// The day before's mean, if it traded then
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<i64>,
}

/// A mover: an item whose mean price changed most from the day before.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteMover {
    pub item_type: String,
    pub item_name: String,
    pub previous: i64,
    pub mean: i64,
    /// The change in whole percent, rounded toward zero
    pub change_pct: i64,
}

/// The day's finest sale.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteSale {
    pub item_type: String,
    pub item_name: String,
    /// The mean quality of the units sold, and its grade
    pub quality: i32,
    pub grade: String,
    pub price: i64,
    pub quantity: i32,
    /// Who sold it, if they let the Gazette print their name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seller: Option<String>,
    /// Who made it (#231): the maker of most of the units sold, when that's
    /// someone other than the seller who lets the Gazette print their name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker: Option<String>,
    /// The seller made most of the units sold themselves
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own_work: bool,
    /// The seller is a citizen (C0.1), named or not
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub seller_citizen: bool,
    /// The credited maker is a citizen
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub maker_citizen: bool,
}

/// One zone's deeds for the day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteDeeds {
    pub zone: String,
    pub zone_name: String,
    pub claimed: i64,
    pub foreclosed: i64,
}

/// One of the day's top producers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteProducer {
    /// Their name, if they let the Gazette print it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Every unit they gathered or crafted
    pub units: i64,
    /// What they made most of, and how much
    pub item_type: String,
    pub item_name: String,
    pub item_units: i64,
    /// A citizen (C0.1), named or not
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub citizen: bool,
}

/// One of the things the city made most of in a day (C2.2).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteMade {
    pub item_type: String,
    pub item_name: String,
    /// Units gathered or crafted, by everyone
    pub units: i64,
    /// How many of them people made; citizens made the rest
    pub by_people: i64,
}

/// A day's figures split between people and citizens (C2.2), so a reader
/// can see how much of the economy is theirs. Each pair is the whole and
/// people's part of it; citizens' part is the difference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteShares {
    /// Units gathered and crafted
    pub made: i64,
    pub made_by_people: i64,
    /// Strands paid at the Exchange and the bazaar's stalls (#240), and how
    /// many went to sellers who are people
    pub sold: i64,
    pub sold_by_people: i64,
    /// Deeds claimed
    pub claimed: i64,
    pub claimed_by_people: i64,
}

/// `GET /gazette/latest` and `GET /gazette?date=`: the edition, if there is
/// one yet, and whether the Gazette prints your name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<Gazette>,
    pub named: bool,
}

/// `GET /gazette` query string.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct GazetteQuery {
    /// The day (`rules::day_number`) whose edition you want
    pub date: i64,
}

/// `PUT /gazette/byline`, and its answer: whether the Gazette may print your
/// name. It applies from the next edition; printed ones stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct GazetteByline {
    pub named: bool,
}

// ---------------------------------------------------------------------------
// /plots
// ---------------------------------------------------------------------------

/// `GET /plots` query string
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct ListPlotsQuery {
    /// Filter by zone ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// Only show unclaimed plots
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unclaimed: Option<bool>,
    /// Filter by owner player ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Uuid>,
    /// Max results (default 50, max 100)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    /// Pagination offset
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i32>,
    /// With `near_y`: nearest this point first, not in grid order, so a page
    /// of a zone's lots is the ones round a spot, not one corner's (#419)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near_y: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ListPlotsResponse {
    /// Matching plots
    pub plots: Vec<PlotInfo>,
    /// Total count (for pagination)
    pub total: i64,
}

/// `GET /plots/at` query string; responds with `Option<PlotInfo>`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::IntoParams))]
pub struct PositionQuery {
    /// X coordinate
    pub x: f64,
    /// Y coordinate
    pub y: f64,
}

/// Client-facing plot info. `GET /plots/{id}` returns one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PlotInfo {
    /// Unique plot identifier
    pub id: Uuid,
    /// Zone the plot is in
    pub zone_id: String,
    /// Center X coordinate
    pub world_x: f64,
    /// Center Y coordinate
    pub world_y: f64,
    /// Plot boundaries
    pub bounds: Bounds,
    /// Size category (small, medium, large)
    pub size_category: String,
    /// Plot type (standard, corner, etc.)
    pub plot_type: String,
    /// Owner player ID if claimed
    pub owner_id: Option<Uuid>,
    /// Owner player name if claimed
    pub owner_name: Option<String>,
    /// The owner is a citizen (C0.1)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owner_citizen: bool,
    /// When the plot was claimed
    pub claimed_at: Option<DateTime<Utc>>,
    /// Current assessed value for taxes. 0 until the plot is first bought.
    pub assessed_value: i32,
    /// Property tax owed and unpaid, in strands (#138). The hourly tax run
    /// takes it from the owner's strands when it can, and
    /// `POST /plots/{id}/pay-tax` pays it off.
    #[serde(default)]
    pub tax_owed: i64,
    /// The debt at which the plot is foreclosed: four weeks of its tax.
    /// `None` while it's unowned or owes no tax.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecloses_at: Option<i64>,
    /// What a deed to this plot costs in Strands: its zone's `deed_cost`
    /// times the zone's deed price tunable (D6.1), which
    /// `POST /plots/{id}/claim` charges. `None` where the zone sells
    /// no deeds. Shown whether or not the plot is owned.
    #[serde(default)]
    pub deed_cost: Option<i32>,
    /// Current number of stations placed
    pub station_count: i32,
    /// Maximum stations allowed
    pub station_capacity: i32,
    /// The `/terrain` lot this plot is (`TerrainPlot::id`). Clients join the two
    /// on it. `None` only for a plot claimed before B7 that no lot replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain_id: Option<String>,
    /// The lot's four corners, as `/terrain` draws them. This is the shape that's
    /// claimed and that station placement tests. `bounds` is its bounding box.
    /// `None` only for a pre-B7 plot, whose shape is `bounds`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corners: Option<[(f64, f64); 4]>,
}

impl PlotInfo {
    /// Whether a point is on this plot: inside its lot, or its box if it has none.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match &self.corners {
            Some(corners) => crate::terrain::quad_contains(corners, x, y),
            None => self.bounds.contains(x, y),
        }
    }
}

/// Axis-aligned bounding box for a plot
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct Bounds {
    /// Minimum X coordinate
    pub min_x: f64,
    /// Minimum Y coordinate
    pub min_y: f64,
    /// Maximum X coordinate
    pub max_x: f64,
    /// Maximum Y coordinate
    pub max_y: f64,
}

impl Bounds {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// Whether a point is inside the box (edges included)
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    /// The center point
    pub fn center(&self) -> (f64, f64) {
        (
            (self.min_x + self.max_x) / 2.0,
            (self.min_y + self.max_y) / 2.0,
        )
    }

    /// Width and height
    pub fn size(&self) -> (f64, f64) {
        (self.max_x - self.min_x, self.max_y - self.min_y)
    }
}

/// `POST /plots/{id}/claim`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ClaimPlotResponse {
    /// The claimed plot
    pub plot: PlotInfo,
    /// New Strand balance after purchase
    pub new_balance: i64,
}

/// `POST /plots/{id}/pay-tax` (#138)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PayTaxRequest {
    /// Strands to pay. All that's owed if absent; more than is owed pays
    /// what's owed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
}

/// The answer to `POST /plots/{id}/pay-tax`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct PayTaxResponse {
    /// The plot, with what it still owes
    pub plot: PlotInfo,
    /// New Strand balance after paying
    pub new_balance: i64,
}

/// One claimed lot. `GET /plots/claimed` returns every one, and WS
/// `plot_claimed` and `plot_released` keep that list current.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema, sqlx::FromRow))]
pub struct ClaimedPlot {
    /// The `/terrain` lot (`TerrainPlot::id`)
    pub terrain_id: String,
    /// The owner's player ID
    pub owner_id: Uuid,
    /// When the current owner claimed it
    pub claimed_at: DateTime<Utc>,
}

/// `GET /plots/{id}/permissions`, and the response to `PUT` on the same path
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema, sqlx::FromRow))]
pub struct PlotPermissions {
    /// Plot this permission set applies to
    pub plot_id: Uuid,
    /// Who can enter the plot: "public", "friends", "guild", "owner"
    pub entry_level: String,
    /// Who can see structures: "public", "friends", "guild", "owner"
    pub visibility_level: String,
    /// Who can use stations: "public", "friends", "guild", "owner"
    pub station_use_level: String,
    /// Who can modify: "public", "friends", "guild", "owner"
    pub modify_level: String,
}

impl Default for PlotPermissions {
    fn default() -> Self {
        Self {
            plot_id: Uuid::nil(),
            entry_level: "public".to_string(),
            visibility_level: "public".to_string(),
            station_use_level: "owner".to_string(),
            modify_level: "owner".to_string(),
        }
    }
}

/// `PUT /plots/{id}/permissions`. Fields left out are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct UpdatePermissionsRequest {
    /// Who can enter: "public", "friends", "guild", "owner"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_level: Option<String>,
    /// Who can see structures: "public", "friends", "guild", "owner"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_level: Option<String>,
    /// Who can use stations: "public", "friends", "guild", "owner"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station_use_level: Option<String>,
    /// Who can modify: "public", "friends", "guild", "owner"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modify_level: Option<String>,
}

// ---------------------------------------------------------------------------
// /bureaucracy
// ---------------------------------------------------------------------------

/// `GET /bureaucracy/zones`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct ZonesResponse {
    /// All zones in the game world
    pub zones: Vec<ZoneInfo>,
}

/// Zone info from the database
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema, sqlx::FromRow))]
pub struct ZoneInfo {
    /// Zone identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Ring order from center (0 = Capitol)
    pub ring_order: i32,
    /// Inner boundary radius
    pub inner_radius: f32,
    /// Outer boundary radius
    pub outer_radius: f32,
    /// Whether this zone has purchasable plots
    pub has_plots: bool,
    /// Plot size category for this zone
    pub plot_size_category: Option<String>,
    /// Cost to purchase a deed in this zone, at its deed price tunable (D6.1)
    pub deed_cost: Option<i32>,
    /// Zone description
    pub description: Option<String>,
}

/// `GET /bureaucracy/deed-prices`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DeedPricesResponse {
    /// Deed prices for all zones with purchasable plots
    pub prices: Vec<DeedPrice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "backend", derive(utoipa::ToSchema))]
pub struct DeedPrice {
    /// Zone identifier
    pub zone_id: String,
    /// Zone display name
    pub zone_name: String,
    /// Cost in Strands to purchase a deed, at its deed price tunable (D6.1)
    pub deed_cost: i32,
    /// Plot size category name
    pub plot_size: String,
    /// Maximum stations allowed on plots in this zone
    pub station_capacity: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_contains_and_center() {
        let bounds = Bounds::new(100.0, 100.0, 200.0, 200.0);
        assert!(bounds.contains(150.0, 150.0));
        assert!(bounds.contains(100.0, 100.0));
        assert!(bounds.contains(200.0, 200.0));
        assert!(!bounds.contains(99.0, 150.0));
        assert!(!bounds.contains(201.0, 150.0));
        assert_eq!(bounds.center(), (150.0, 150.0));
        assert_eq!(bounds.size(), (100.0, 100.0));
    }

    #[test]
    fn nearby_radius_defaults_and_is_capped() {
        let q = |radius| NearbyQuery { radius }.effective_radius();
        assert_eq!(q(None), rules::NEARBY_NODE_RANGE);
        assert_eq!(q(Some(f64::NAN)), rules::NEARBY_NODE_RANGE);
        assert_eq!(q(Some(500.0)), 500.0);
        assert_eq!(q(Some(1e9)), rules::NEARBY_NODE_RANGE_MAX);
        assert_eq!(q(Some(f64::INFINITY)), rules::NEARBY_NODE_RANGE_MAX);
        assert_eq!(q(Some(-5.0)), 0.0);
    }

    #[test]
    fn slot_item_grades_its_quality() {
        let item = SlotItem::new(Uuid::nil(), "rope".into(), "Rope".into(), 81, 1, 0);
        assert_eq!(item.quality_grade, "B");
    }
}
