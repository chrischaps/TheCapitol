//! The REST half of the client: one [`RestRequest`] variant per backend route
//! (admin routes aside), each paired with the [`RestResponse`] variant it
//! decodes to.
//!
//! The bearer token is attached whenever the IO thread has one, so callers
//! never handle headers. Errors arrive as `{"error": msg}` from the server
//! (`backend/src/error.rs`) and become [`RestError::Http`].

use capitol_protocol::rest::*;
use capitol_protocol::terrain::TerrainResponse;
use reqwest::Method;
use uuid::Uuid;

/// A REST call. Paths are relative to the server root.
#[derive(Debug, Clone, PartialEq)]
pub enum RestRequest {
    /// `POST /auth/register`
    Register(RegisterRequest),
    /// `POST /auth/login`
    Login(LoginRequest),
    /// `POST /auth/logout`
    Logout(LogoutRequest),
    /// `GET /health`
    Health,
    /// `GET /terrain`. With `cached_version`, it's sent as `If-None-Match`, and
    /// an unchanged terrain answers [`RestResponse::TerrainUnchanged`] (a 304
    /// with no body) instead of the whole megabyte again.
    Terrain { cached_version: Option<String> },

    /// `GET /player/me`: name, live position, zone and balance (B2)
    PlayerMe,
    /// `POST /player/move`. The WebSocket `move` is what the game uses.
    PlayerMove(MoveRequest),
    /// `GET /player/inventory`
    PlayerInventory,
    /// `GET /player/currency`
    PlayerCurrency,

    /// `GET /world/nearby`
    WorldNearby(NearbyQuery),
    /// `POST /world/extract`
    Extract(ExtractRequest),

    /// `GET /inventory/containers`
    Containers,
    /// `POST /inventory/move`
    MoveItem(MoveItemRequest),

    /// `GET /recipes`
    Recipes,
    /// `GET /catalog`: the static game data (E0.2)
    Catalog,

    /// `GET /stations/nearby`
    NearbyStations,
    /// `POST /stations/place`
    PlaceStation(PlaceStationRequest),
    /// `DELETE /stations/{id}`
    RemoveStation { station_id: Uuid },
    /// `GET /stations/{id}/container`
    StationContainer { station_id: Uuid },

    /// `POST /exchange/deposit`
    Deposit(DepositRequest),
    /// `POST /exchange/withdraw`
    Withdraw(WithdrawRequest),

    /// `GET /plots`
    ListPlots(ListPlotsQuery),
    /// `GET /plots/claimed`: every claimed lot, everyone's (B13)
    ClaimedPlots,
    /// `GET /plots/{id}`
    Plot { plot_id: Uuid },
    /// `GET /plots/at?x=&y=`
    PlotAt(PositionQuery),
    /// `POST /plots/{id}/claim`
    ClaimPlot { plot_id: Uuid },
    /// `POST /plots/{id}/pay-tax` (#138)
    PayTax {
        plot_id: Uuid,
        request: PayTaxRequest,
    },
    /// `GET /plots/{id}/permissions`
    PlotPermissions { plot_id: Uuid },
    /// `PUT /plots/{id}/permissions`
    UpdatePlotPermissions {
        plot_id: Uuid,
        update: UpdatePermissionsRequest,
    },

    /// `GET /bureaucracy/zones`
    Zones,
    /// `GET /bureaucracy/deed-prices`
    DeedPrices,
}

/// A decoded REST response. Each [`RestRequest`] decodes to exactly one variant.
#[derive(Debug, Clone, PartialEq)]
pub enum RestResponse {
    /// Register and login
    Auth(AuthResponse),
    /// Logout and station removal
    Success(SuccessResponse),
    Health(HealthResponse),
    /// Boxed: it's about a megabyte of roads and plots.
    Terrain(Box<TerrainResponse>),
    /// `/terrain` still has the `cached_version` the request named.
    TerrainUnchanged,
    Me(PlayerMeResponse),
    Move(MoveResponse),
    Inventory(InventoryResponse),
    Currency(CurrencyResponse),
    Nearby(NearbyResponse),
    Extract(ExtractResponse),
    Containers(ContainersResponse),
    MoveItem(MoveItemResponse),
    Recipes(RecipesResponse),
    Catalog(Box<CatalogResponse>),
    NearbyStations(NearbyStationsResponse),
    PlaceStation(PlaceStationResponse),
    StationContainer(StationContainerResponse),
    Deposit(DepositResponse),
    Withdraw(WithdrawResponse),
    Plots(ListPlotsResponse),
    ClaimedPlots(Vec<ClaimedPlot>),
    Plot(PlotInfo),
    /// `None` when no plot covers the position
    PlotAt(Option<PlotInfo>),
    ClaimPlot(ClaimPlotResponse),
    PayTax(PayTaxResponse),
    /// The current permissions, from both `GET` and `PUT`
    PlotPermissions(PlotPermissions),
    Zones(ZonesResponse),
    DeedPrices(DeedPricesResponse),
}

/// Why a REST call failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RestError {
    /// The server answered with a non-2xx status. `message` is the body's
    /// `error` field when there is one.
    #[error("{message} (HTTP {status})")]
    Http { status: u16, message: String },
    /// The request never got an answer: refused, timed out, reset.
    #[error("network error: {0}")]
    Transport(String),
    /// A 2xx whose body wasn't the expected shape.
    #[error("unexpected response: {0}")]
    Decode(String),
}

impl RestError {
    /// HTTP status, if the server answered.
    pub fn status(&self) -> Option<u16> {
        match self {
            RestError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// A 401: the token is missing, expired, or another login replaced it.
    /// The game should return to the login screen.
    pub fn is_unauthorized(&self) -> bool {
        self.status() == Some(401)
    }
}

impl RestRequest {
    pub fn method(&self) -> Method {
        use RestRequest::*;
        match self {
            Register(_)
            | Login(_)
            | Logout(_)
            | PlayerMove(_)
            | Extract(_)
            | MoveItem(_)
            | PlaceStation(_)
            | Deposit(_)
            | Withdraw(_)
            | ClaimPlot { .. }
            | PayTax { .. } => Method::POST,
            UpdatePlotPermissions { .. } => Method::PUT,
            RemoveStation { .. } => Method::DELETE,
            Health
            | Terrain { .. }
            | PlayerMe
            | PlayerInventory
            | PlayerCurrency
            | WorldNearby(_)
            | Containers
            | Recipes
            | Catalog
            | NearbyStations
            | StationContainer { .. }
            | ListPlots(_)
            | ClaimedPlots
            | Plot { .. }
            | PlotAt(_)
            | PlotPermissions { .. }
            | Zones
            | DeedPrices => Method::GET,
        }
    }

    /// The path, without the query string.
    pub fn path(&self) -> String {
        use RestRequest::*;
        match self {
            Register(_) => "/auth/register".into(),
            Login(_) => "/auth/login".into(),
            Logout(_) => "/auth/logout".into(),
            Health => "/health".into(),
            Terrain { .. } => "/terrain".into(),
            PlayerMe => "/player/me".into(),
            PlayerMove(_) => "/player/move".into(),
            PlayerInventory => "/player/inventory".into(),
            PlayerCurrency => "/player/currency".into(),
            WorldNearby(_) => "/world/nearby".into(),
            Extract(_) => "/world/extract".into(),
            Containers => "/inventory/containers".into(),
            MoveItem(_) => "/inventory/move".into(),
            Recipes => "/recipes".into(),
            Catalog => "/catalog".into(),
            NearbyStations => "/stations/nearby".into(),
            PlaceStation(_) => "/stations/place".into(),
            RemoveStation { station_id } => format!("/stations/{station_id}"),
            StationContainer { station_id } => format!("/stations/{station_id}/container"),
            Deposit(_) => "/exchange/deposit".into(),
            Withdraw(_) => "/exchange/withdraw".into(),
            ListPlots(_) => "/plots".into(),
            ClaimedPlots => "/plots/claimed".into(),
            Plot { plot_id } => format!("/plots/{plot_id}"),
            PlotAt(_) => "/plots/at".into(),
            ClaimPlot { plot_id } => format!("/plots/{plot_id}/claim"),
            PayTax { plot_id, .. } => format!("/plots/{plot_id}/pay-tax"),
            PlotPermissions { plot_id } | UpdatePlotPermissions { plot_id, .. } => {
                format!("/plots/{plot_id}/permissions")
            }
            Zones => "/bureaucracy/zones".into(),
            DeedPrices => "/bureaucracy/deed-prices".into(),
        }
    }

    /// Adds the query string and JSON body, if the route has them.
    fn with_payload(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        use RestRequest::*;
        match self {
            Register(b) => req.json(b),
            Login(b) => req.json(b),
            Logout(b) => req.json(b),
            PlayerMove(b) => req.json(b),
            Extract(b) => req.json(b),
            MoveItem(b) => req.json(b),
            PlaceStation(b) => req.json(b),
            Deposit(b) => req.json(b),
            Withdraw(b) => req.json(b),
            UpdatePlotPermissions { update, .. } => req.json(update),
            PayTax { request, .. } => req.json(request),
            WorldNearby(q) => req.query(q),
            ListPlots(q) => req.query(q),
            PlotAt(q) => req.query(q),
            // The server's ETag is weak, `W/"<version>"` (B3).
            Terrain {
                cached_version: Some(version),
            } => req.header(reqwest::header::IF_NONE_MATCH, format!("W/\"{version}\"")),
            _ => req,
        }
    }

    /// Decodes a 2xx body into this request's response variant.
    pub fn decode(&self, body: &[u8]) -> serde_json::Result<RestResponse> {
        use serde_json::from_slice as de;
        use RestRequest as Q;
        use RestResponse as A;
        Ok(match self {
            Q::Register(_) | Q::Login(_) => A::Auth(de(body)?),
            Q::Logout(_) | Q::RemoveStation { .. } => A::Success(de(body)?),
            Q::Health => A::Health(de(body)?),
            Q::Terrain { .. } => A::Terrain(Box::new(de(body)?)),
            Q::PlayerMe => A::Me(de(body)?),
            Q::PlayerMove(_) => A::Move(de(body)?),
            Q::PlayerInventory => A::Inventory(de(body)?),
            Q::PlayerCurrency => A::Currency(de(body)?),
            Q::WorldNearby(_) => A::Nearby(de(body)?),
            Q::Extract(_) => A::Extract(de(body)?),
            Q::Containers => A::Containers(de(body)?),
            Q::MoveItem(_) => A::MoveItem(de(body)?),
            Q::Recipes => A::Recipes(de(body)?),
            Q::Catalog => A::Catalog(Box::new(de(body)?)),
            Q::NearbyStations => A::NearbyStations(de(body)?),
            Q::PlaceStation(_) => A::PlaceStation(de(body)?),
            Q::StationContainer { .. } => A::StationContainer(de(body)?),
            Q::Deposit(_) => A::Deposit(de(body)?),
            Q::Withdraw(_) => A::Withdraw(de(body)?),
            Q::ListPlots(_) => A::Plots(de(body)?),
            Q::ClaimedPlots => A::ClaimedPlots(de(body)?),
            Q::Plot { .. } => A::Plot(de(body)?),
            Q::PlotAt(_) => A::PlotAt(de(body)?),
            Q::ClaimPlot { .. } => A::ClaimPlot(de(body)?),
            Q::PayTax { .. } => A::PayTax(de(body)?),
            Q::PlotPermissions { .. } | Q::UpdatePlotPermissions { .. } => {
                A::PlotPermissions(de(body)?)
            }
            Q::Zones => A::Zones(de(body)?),
            Q::DeedPrices => A::DeedPrices(de(body)?),
        })
    }
}

/// Runs one request to completion. `base` has no trailing slash.
pub(crate) async fn execute(
    http: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    request: &RestRequest,
) -> Result<RestResponse, RestError> {
    let url = format!("{base}{}", request.path());
    let send = || {
        let mut builder = request.with_payload(http.request(request.method(), &url));
        if let Some(token) = token {
            builder = builder.bearer_auth(token);
        }
        builder.send()
    };

    // A GET that fails to send is tried once more. Requests go out on pooled
    // keep-alive connections, and one the server has just closed fails with
    // "connection closed before message completed" (tests/net.rs,
    // `a_get_on_a_dead_pooled_connection_is_sent_again`). The poller never
    // retries, so a refetch lost that way stays stale until something asks
    // again. A GET is safe to repeat; the others aren't.
    let response = match send().await {
        Err(e) if request.method() == Method::GET => {
            tracing::debug!("GET {} failed to send, trying again: {e}", request.path());
            send().await
        }
        sent => sent,
    }
    .map_err(transport)?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_MODIFIED
        && matches!(
            request,
            RestRequest::Terrain {
                cached_version: Some(_)
            }
        )
    {
        return Ok(RestResponse::TerrainUnchanged);
    }
    let body = response.bytes().await.map_err(transport)?;

    if !status.is_success() {
        return Err(RestError::Http {
            status: status.as_u16(),
            message: error_message(status, &body),
        });
    }
    request
        .decode(&body)
        .map_err(|e| RestError::Decode(format!("{} {}: {e}", request.method(), request.path())))
}

fn transport(e: reqwest::Error) -> RestError {
    // reqwest's Display omits the cause ("error sending request"); the chain has it.
    let mut message = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    RestError::Transport(message)
}

/// The `error` field of a failure body, or the status text when the body has none.
fn error_message(status: reqwest::StatusCode, body: &[u8]) -> String {
    #[derive(serde::Deserialize)]
    struct ErrorBody {
        error: String,
    }
    if let Ok(ErrorBody { error }) = serde_json::from_slice(body) {
        return error;
    }
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        status
            .canonical_reason()
            .unwrap_or("request failed")
            .to_string()
    } else {
        text.chars().take(200).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_match_the_backend_router() {
        let id = Uuid::nil();
        let cases = [
            (RestRequest::Health, "GET", "/health".to_string()),
            (RestRequest::Recipes, "GET", "/recipes".into()),
            (RestRequest::Catalog, "GET", "/catalog".into()),
            (RestRequest::PlayerMe, "GET", "/player/me".into()),
            (
                RestRequest::ListPlots(Default::default()),
                "GET",
                "/plots".into(),
            ),
            (RestRequest::ClaimedPlots, "GET", "/plots/claimed".into()),
            (
                RestRequest::RemoveStation { station_id: id },
                "DELETE",
                format!("/stations/{id}"),
            ),
            (
                RestRequest::StationContainer { station_id: id },
                "GET",
                format!("/stations/{id}/container"),
            ),
            (
                RestRequest::ClaimPlot { plot_id: id },
                "POST",
                format!("/plots/{id}/claim"),
            ),
            (
                RestRequest::PayTax {
                    plot_id: id,
                    request: PayTaxRequest { amount: Some(5) },
                },
                "POST",
                format!("/plots/{id}/pay-tax"),
            ),
            (
                RestRequest::UpdatePlotPermissions {
                    plot_id: id,
                    update: Default::default(),
                },
                "PUT",
                format!("/plots/{id}/permissions"),
            ),
            (
                RestRequest::DeedPrices,
                "GET",
                "/bureaucracy/deed-prices".into(),
            ),
        ];
        for (request, method, path) in cases {
            assert_eq!(request.method().as_str(), method, "{request:?}");
            assert_eq!(request.path(), path, "{request:?}");
        }
    }

    #[test]
    fn plot_at_decodes_null_as_none() {
        let response = RestRequest::PlotAt(PositionQuery { x: 1.0, y: 2.0 })
            .decode(b"null")
            .unwrap();
        assert_eq!(response, RestResponse::PlotAt(None));
    }

    #[test]
    fn decode_rejects_the_wrong_shape() {
        assert!(RestRequest::PlayerCurrency
            .decode(br#"{"nope":1}"#)
            .is_err());
    }

    #[test]
    fn error_message_prefers_the_error_field() {
        let status = reqwest::StatusCode::CONFLICT;
        assert_eq!(
            error_message(status, br#"{"error":"Email already registered"}"#),
            "Email already registered"
        );
        assert_eq!(error_message(status, b"plain text"), "plain text");
        assert_eq!(error_message(status, b""), "Conflict");
    }
}
