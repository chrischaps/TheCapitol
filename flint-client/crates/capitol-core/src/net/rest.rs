//! The REST half of the client: one [`RestRequest`] variant per backend route
//! (admin routes aside, but for the citizens `capitol-town` mints, the
//! ledger its soak reads, the town's reports, and what the admin overlay
//! shows, D5.3), each
//! paired with the [`RestResponse`] variant it decodes to.
//!
//! The bearer token (or a bot's API key, E0.9) is attached whenever the IO
//! thread has one, so callers never handle headers. Errors arrive as `{"error": msg}` from the server
//! (`backend/src/error.rs`) and become [`RestError::Http`].

use capitol_protocol::rest::*;
use capitol_protocol::annotation::{Annotation, AnnotationsQuery};
use capitol_protocol::metrics::{Series, SeriesQuery};
use capitol_protocol::terrain::TerrainResponse;
use capitol_protocol::tunables::{TunablesResponse, TunablesUpdate};
use std::time::Duration;

use reqwest::Method;
use uuid::Uuid;

/// A REST call. Paths are relative to the server root.
#[derive(Debug, Clone, PartialEq)]
pub enum RestRequest {
    /// `POST /auth/register`
    Register(RegisterRequest),
    /// `POST /auth/login`
    Login(LoginRequest),
    /// `POST /auth/refresh`. The IO thread sends these itself (E0.6).
    Refresh(RefreshRequest),
    /// `POST /auth/logout`
    Logout(LogoutRequest),
    /// `POST /account/api-keys`: mint a bot's key (E0.9). Needs the
    /// player's own token: a key can't manage keys.
    CreateApiKey(CreateApiKeyRequest),
    /// `GET /account/api-keys`
    ApiKeys,
    /// `DELETE /account/api-keys/{id}`
    RevokeApiKey { key_id: Uuid },

    /// `GET /health`
    Health,
    /// `GET /terrain`. With `cached_version`, it's sent as `If-None-Match`, and
    /// an unchanged terrain answers [`RestResponse::TerrainUnchanged`] (a 304
    /// with no body) instead of the whole megabyte again.
    Terrain { cached_version: Option<String> },

    /// `GET /player/me`: name, live position, zone and balance (B2)
    PlayerMe,
    /// `GET /players/{id}/card`: who someone is (S0.3)
    PlayerCard { player_id: Uuid },
    /// `GET /players/by-name/{name}`: the same, by their name in any case
    PlayerCardByName { name: String },
    /// `GET /player/mutes`: who you've muted (S1.4)
    Mutes,
    /// `PUT /players/{id}/mute`
    Mute { player_id: Uuid },
    /// `DELETE /players/{id}/mute`
    Unmute { player_id: Uuid },
    /// `GET /letters?box=&before=`: a page of your in or out box (S1.5)
    Letters(LettersQuery),
    /// `POST /letters`: write to someone, for a Strand's postage
    SendLetter(SendLetter),
    /// `POST /letters/{id}/read`
    ReadLetter { letter_id: Uuid },
    /// `DELETE /letters/{id}`: drop your copy
    DropLetter { letter_id: Uuid },
    /// `POST /player/move`. The WebSocket `move` is what the game uses.
    PlayerMove(MoveRequest),
    /// `GET /player/inventory`
    PlayerInventory,
    /// `GET /player/currency`
    PlayerCurrency,

    /// `GET /world/nearby`
    WorldNearby(NearbyQuery),
    /// `GET /world/nodes`: every node in the world (E2.5)
    WorldNodes,
    /// `POST /world/extract`
    Extract(ExtractRequest),

    /// `GET /inventory/containers`
    Containers,
    /// `POST /inventory/move`
    MoveItem(MoveItemRequest),
    /// `POST /items/{id}/split` (E1.2)
    SplitStack {
        item_id: Uuid,
        request: SplitStackRequest,
    },

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
    /// `PUT /stations/{id}/automation`: what your station makes on its own (E4.2)
    ConfigureAutomation {
        station_id: Uuid,
        request: ConfigureAutomation,
    },
    /// `POST /stations/{id}/automation/start`
    StartAutomation { station_id: Uuid },
    /// `POST /stations/{id}/automation/stop`
    StopAutomation { station_id: Uuid },

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
    /// `POST /plots/{id}/claim`. With `use_grant`, the deed the road in
    /// earned pays for it, not Strands (#440, #448).
    ClaimPlot { plot_id: Uuid, use_grant: bool },
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
    /// `GET /bureaucracy/zone-facts`: what living in each home zone is like (#444)
    ZoneFacts,
    /// `GET /bureaucracy/free-lots?zone=`: the Deeds Office's list (#448)
    FreeLots(FreeLotsQuery),

    /// `GET /tutorial`: where you are on the road in (#438). The game has it
    /// from the socket's `tutorial`; this is for a bot.
    Tutorial,
    /// `PUT /tutorial/dismissed`: hide the card for now, or bring it back
    /// (#441, plan D2)
    SetTutorialDismissed(capitol_protocol::tutorial::SetDismissed),

    /// `POST /player/subscription`: mock activation, on a server with
    /// `MOCK_SUBSCRIPTIONS` on (E4.1)
    Subscribe(SubscribeRequest),
    /// `POST /player/grant`: Strands and goods for a market maker, on a
    /// server with `DEV_GRANTS` on (E3.8)
    Grant(GrantRequest),

    /// `POST /market/orders` (E3.3)
    PlaceOrder(PlaceOrder),
    /// `DELETE /market/orders/{id}`
    CancelOrder { order_id: Uuid },
    /// `GET /market/orders?item=&side=`: an item's book
    MarketDepth(MarketDepthQuery),
    /// `GET /market/orders/mine`
    MyOrders(MyOrdersQuery),
    /// `GET /market/history?item=&bucket=&since=&grade=`
    MarketHistory(MarketHistoryQuery),
    /// `GET /market/summary`
    MarketSummary,
    /// `GET /market/listings`: the newest sells across every item (#493)
    MarketListings,
    /// `GET /market/box`
    MarketBox,
    /// `POST /market/box/collect`
    CollectMarketBox(CollectRequest),
    /// `POST /inventory/impound/collect` (#141)
    CollectImpound(CollectRequest),

    /// `GET /gazette/latest`, or with a day `GET /gazette?date=` (E3.9)
    Gazette { day: Option<i64> },
    /// `PUT /gazette/byline`
    SetByline(GazetteByline),

    /// `GET /bazaar`: the directory of shops (E3.10)
    Bazaar,
    /// `GET /bazaar/mine`
    MyStall,
    /// `GET /bazaar/stalls/{id}`: a shop's prices and units, at its stall
    Shop { stall_id: String },
    /// `POST /bazaar/stalls/{id}/rent`
    RentStall { stall_id: String, request: RentStall },
    /// `POST /bazaar/mine/listings`
    Stock(StockListing),
    /// `DELETE /bazaar/listings/{id}`
    Unlist { listing_id: Uuid },
    /// `POST /bazaar/listings/{id}/buy`
    Buy { listing_id: Uuid, request: BuyListing },

    /// `POST /admin/citizens`: make a citizen and its key (C0.1). Needs an
    /// admin's token.
    CreateCitizen(CreateCitizenRequest),
    /// `GET /admin/citizens`
    Citizens,
    /// `GET /admin/ledger`: the world's books, for the town's soak (C1.7)
    Ledger,
    /// `POST /admin/town/key`: mint the town's report key (D4.1). Needs an
    /// admin's token.
    TownKey,
    /// `POST /admin/town/report`: the town, each game minute (D4.1), by its
    /// key. Boxed: it's a row a citizen.
    TownReport(Box<capitol_protocol::metrics::TownReport>),
    /// `GET /admin/series`: the Observatory's samples, for the admin overlay
    /// (D5.3)
    Series(SeriesQuery),
    /// `GET /admin/annotations`: what explains a bend in a chart
    Annotations(AnnotationsQuery),
    /// `GET /admin/tunables`: every live knob, read-only in the client
    Tunables,
    /// `PUT /admin/tunables`: set knobs, all or none, for a balance run's
    /// preset (D6.3). Needs an admin's token.
    SetTunables(TunablesUpdate),
    /// `GET /admin/walkers`: the players near a point, with where the server
    /// is walking them, for the admin's world layers
    Walkers(WalkersQuery),
}

/// A decoded REST response. Each [`RestRequest`] decodes to exactly one variant.
#[derive(Debug, Clone, PartialEq)]
pub enum RestResponse {
    /// Register, login and refresh
    Auth(AuthResponse),
    /// Logout, station removal and key revocation
    Success(SuccessResponse),
    CreatedApiKey(CreatedApiKey),
    ApiKeys(Vec<ApiKeyInfo>),
    Health(HealthResponse),
    /// Boxed: it's about a megabyte of roads and plots.
    Terrain(Box<TerrainResponse>),
    /// `/terrain` still has the `cached_version` the request named.
    TerrainUnchanged,
    Me(PlayerMeResponse),
    /// Someone's card (S0.3)
    Card(PlayerCard),
    /// Who you've muted (S1.4), after a mute or an unmute too
    Mutes(MutesResponse),
    /// A page of a box of letters (S1.5)
    Mailbox(Mailbox),
    /// A letter you sent
    Letter(Letter),
    /// Your unread letters, after one is read or dropped
    LettersUnread(LettersUnread),
    Move(MoveResponse),
    Inventory(InventoryResponse),
    Currency(CurrencyResponse),
    Nearby(NearbyResponse),
    /// `/world/nodes`: the whole list, so it replaces what the store holds.
    AllNodes(NearbyResponse),
    Extract(ExtractResponse),
    Containers(ContainersResponse),
    MoveItem(MoveItemResponse),
    SplitStack(SplitStackResponse),
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
    /// Your place on the road in, from both `GET` and `PUT`
    Tutorial(capitol_protocol::tutorial::TutorialProgress),
    ZoneFacts(ZoneFactsResponse),
    FreeLots(FreeLotsResponse),
    Subscription(Subscription),
    Granted(GrantResponse),
    /// An order as placed, or as cancelled
    Order(MarketOrder),
    MarketDepth(MarketDepthResponse),
    MyOrders(Vec<MarketOrder>),
    MarketHistory(MarketHistoryResponse),
    MarketSummary(MarketSummaryResponse),
    MarketListings(MarketListingsResponse),
    MarketBox(ContainerResponse),
    Collected(CollectResponse),
    /// An edition, if there is one, and your byline
    Gazette(Box<GazetteResponse>),
    /// Your byline as it now stands
    Byline(GazetteByline),
    /// A station's automation as configured, started or stopped
    Automation {
        station_id: Uuid,
        automation: StationAutomation,
    },
    Bazaar(BazaarDirectory),
    MyStall(MyStallResponse),
    /// A shop seen at its stall, or yours as rented
    Shop(BazaarShop),
    /// A listing as stocked
    Listed(BazaarListing),
    /// A listing taken off your shop
    Unlisted { listing_id: Uuid },
    Bought {
        listing_id: Uuid,
        purchase: BazaarPurchase,
    },
    /// A citizen as made, with its key's secret
    Citizen(Box<CreatedCitizen>),
    Citizens(Vec<CitizenInfo>),
    Ledger(Box<capitol_protocol::ledger::Ledger>),
    Series(Box<Series>),
    Annotations(Vec<Annotation>),
    Tunables(TunablesResponse),
    Walkers(Vec<Walker>),
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
    /// The IO thread has already tried a refresh, so the game should return
    /// to the login screen.
    pub fn is_unauthorized(&self) -> bool {
        self.status() == Some(401)
    }
}

impl RestRequest {
    /// The `/auth` routes. Their 401s are about the credentials they carry,
    /// not the session's token, so they aren't retried after a refresh.
    pub fn is_auth(&self) -> bool {
        matches!(
            self,
            RestRequest::Register(_)
                | RestRequest::Login(_)
                | RestRequest::Refresh(_)
                | RestRequest::Logout(_)
        )
    }

    pub fn method(&self) -> Method {
        use RestRequest::*;
        match self {
            Register(_)
            | Login(_)
            | Refresh(_)
            | Logout(_)
            | CreateApiKey(_)
            | PlayerMove(_)
            | SendLetter(_)
            | ReadLetter { .. }
            | Extract(_)
            | MoveItem(_)
            | SplitStack { .. }
            | PlaceStation(_)
            | Deposit(_)
            | Withdraw(_)
            | ClaimPlot { .. }
            | PayTax { .. }
            | Subscribe(_)
            | Grant(_)
            | PlaceOrder(_)
            | CollectMarketBox(_)
            | CollectImpound(_)
            | StartAutomation { .. }
            | StopAutomation { .. }
            | RentStall { .. }
            | Stock(_)
            | Buy { .. }
            | CreateCitizen(_)
            | TownKey
            | TownReport(_) => Method::POST,
            UpdatePlotPermissions { .. }
            | SetByline(_)
            | ConfigureAutomation { .. }
            | SetTunables(_)
            | SetTutorialDismissed(_)
            | Mute { .. } => Method::PUT,
            RemoveStation { .. } | RevokeApiKey { .. } | CancelOrder { .. } | Unlist { .. } | Unmute { .. } | DropLetter { .. } => {
                Method::DELETE
            }
            ApiKeys
            | Health
            | Terrain { .. }
            | PlayerMe
            | PlayerCard { .. }
            | PlayerCardByName { .. }
            | Mutes
            | Letters(_)
            | PlayerInventory
            | PlayerCurrency
            | WorldNearby(_)
            | WorldNodes
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
            | DeedPrices
            | Tutorial
            | ZoneFacts
            | FreeLots(_)
            | MarketDepth(_)
            | MyOrders(_)
            | MarketHistory(_)
            | MarketSummary
            | MarketListings
            | MarketBox
            | Gazette { .. }
            | Bazaar
            | MyStall
            | Shop { .. }
            | Citizens
            | Ledger
            | Series(_)
            | Annotations(_)
            | Tunables
            | Walkers(_) => Method::GET,
        }
    }

    /// The path, without the query string.
    pub fn path(&self) -> String {
        use RestRequest::*;
        match self {
            Register(_) => "/auth/register".into(),
            Login(_) => "/auth/login".into(),
            Refresh(_) => "/auth/refresh".into(),
            Logout(_) => "/auth/logout".into(),
            CreateApiKey(_) | ApiKeys => "/account/api-keys".into(),
            RevokeApiKey { key_id } => format!("/account/api-keys/{key_id}"),
            Health => "/health".into(),
            Terrain { .. } => "/terrain".into(),
            PlayerMe => "/player/me".into(),
            PlayerCard { player_id } => format!("/players/{player_id}/card"),
            PlayerCardByName { name } => format!("/players/by-name/{}", path_segment(name)),
            Mutes => "/player/mutes".into(),
            Mute { player_id } | Unmute { player_id } => format!("/players/{player_id}/mute"),
            Letters(_) | SendLetter(_) => "/letters".into(),
            ReadLetter { letter_id } => format!("/letters/{letter_id}/read"),
            DropLetter { letter_id } => format!("/letters/{letter_id}"),
            PlayerMove(_) => "/player/move".into(),
            PlayerInventory => "/player/inventory".into(),
            PlayerCurrency => "/player/currency".into(),
            WorldNearby(_) => "/world/nearby".into(),
            WorldNodes => "/world/nodes".into(),
            Extract(_) => "/world/extract".into(),
            Containers => "/inventory/containers".into(),
            MoveItem(_) => "/inventory/move".into(),
            SplitStack { item_id, .. } => format!("/items/{item_id}/split"),
            Recipes => "/recipes".into(),
            Catalog => "/catalog".into(),
            NearbyStations => "/stations/nearby".into(),
            PlaceStation(_) => "/stations/place".into(),
            RemoveStation { station_id } => format!("/stations/{station_id}"),
            StationContainer { station_id } => format!("/stations/{station_id}/container"),
            ConfigureAutomation { station_id, .. } => format!("/stations/{station_id}/automation"),
            StartAutomation { station_id } => format!("/stations/{station_id}/automation/start"),
            StopAutomation { station_id } => format!("/stations/{station_id}/automation/stop"),
            Deposit(_) => "/exchange/deposit".into(),
            Withdraw(_) => "/exchange/withdraw".into(),
            ListPlots(_) => "/plots".into(),
            ClaimedPlots => "/plots/claimed".into(),
            Plot { plot_id } => format!("/plots/{plot_id}"),
            PlotAt(_) => "/plots/at".into(),
            ClaimPlot { plot_id, .. } => format!("/plots/{plot_id}/claim"),
            PayTax { plot_id, .. } => format!("/plots/{plot_id}/pay-tax"),
            PlotPermissions { plot_id } | UpdatePlotPermissions { plot_id, .. } => {
                format!("/plots/{plot_id}/permissions")
            }
            Zones => "/bureaucracy/zones".into(),
            DeedPrices => "/bureaucracy/deed-prices".into(),
            Tutorial => "/tutorial".into(),
            SetTutorialDismissed(_) => "/tutorial/dismissed".into(),
            ZoneFacts => "/bureaucracy/zone-facts".into(),
            FreeLots(_) => "/bureaucracy/free-lots".into(),
            Subscribe(_) => "/player/subscription".into(),
            Grant(_) => "/player/grant".into(),
            PlaceOrder(_) | MarketDepth(_) => "/market/orders".into(),
            CancelOrder { order_id } => format!("/market/orders/{order_id}"),
            MyOrders(_) => "/market/orders/mine".into(),
            MarketHistory(_) => "/market/history".into(),
            MarketSummary => "/market/summary".into(),
            MarketListings => "/market/listings".into(),
            MarketBox => "/market/box".into(),
            CollectMarketBox(_) => "/market/box/collect".into(),
            CollectImpound(_) => "/inventory/impound/collect".into(),
            Gazette { day: None } => "/gazette/latest".into(),
            Gazette { day: Some(_) } => "/gazette".into(),
            SetByline(_) => "/gazette/byline".into(),
            Bazaar => "/bazaar".into(),
            MyStall => "/bazaar/mine".into(),
            Shop { stall_id } => format!("/bazaar/stalls/{stall_id}"),
            RentStall { stall_id, .. } => format!("/bazaar/stalls/{stall_id}/rent"),
            Stock(_) => "/bazaar/mine/listings".into(),
            Unlist { listing_id } => format!("/bazaar/listings/{listing_id}"),
            Buy { listing_id, .. } => format!("/bazaar/listings/{listing_id}/buy"),
            CreateCitizen(_) | Citizens => "/admin/citizens".into(),
            Ledger => "/admin/ledger".into(),
            TownKey => "/admin/town/key".into(),
            TownReport(_) => "/admin/town/report".into(),
            Series(_) => "/admin/series".into(),
            Annotations(_) => "/admin/annotations".into(),
            Tunables | SetTunables(_) => "/admin/tunables".into(),
            Walkers(_) => "/admin/walkers".into(),
        }
    }

    /// Adds the query string and JSON body, if the route has them.
    fn with_payload(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        use RestRequest::*;
        match self {
            Register(b) => req.json(b),
            Login(b) => req.json(b),
            Refresh(b) => req.json(b),
            Logout(b) => req.json(b),
            CreateApiKey(b) => req.json(b),
            PlayerMove(b) => req.json(b),
            SendLetter(b) => req.json(b),
            Letters(q) => req.query(q),
            Extract(b) => req.json(b),
            MoveItem(b) => req.json(b),
            SplitStack { request, .. } => req.json(request),
            PlaceStation(b) => req.json(b),
            Deposit(b) => req.json(b),
            Withdraw(b) => req.json(b),
            Subscribe(b) => req.json(b),
            Grant(b) => req.json(b),
            PlaceOrder(b) => req.json(b),
            CollectMarketBox(b) | CollectImpound(b) => req.json(b),
            SetByline(b) => req.json(b),
            ConfigureAutomation { request, .. } => req.json(request),
            RentStall { request, .. } => req.json(request),
            Stock(b) => req.json(b),
            Buy { request, .. } => req.json(request),
            CreateCitizen(b) => req.json(b),
            TownReport(b) => req.json(b),
            SetTunables(b) => req.json(b),
            SetTutorialDismissed(b) => req.json(b),
            Gazette { day: Some(date) } => req.query(&GazetteQuery { date: *date }),
            MarketDepth(q) => req.query(q),
            MyOrders(q) => req.query(q),
            MarketHistory(q) => req.query(q),
            UpdatePlotPermissions { update, .. } => req.json(update),
            PayTax { request, .. } => req.json(request),
            // No body pays in Strands, as every client did before the grant
            ClaimPlot { use_grant: true, .. } => req.json(&ClaimPlotRequest { use_grant: true }),
            FreeLots(q) => req.query(q),
            WorldNearby(q) => req.query(q),
            ListPlots(q) => req.query(q),
            PlotAt(q) => req.query(q),
            Series(q) => req.query(q),
            Annotations(q) => req.query(q),
            Walkers(q) => req.query(q),
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
            Q::Register(_) | Q::Login(_) | Q::Refresh(_) => A::Auth(de(body)?),
            Q::Logout(_) | Q::RemoveStation { .. } | Q::RevokeApiKey { .. } => {
                A::Success(de(body)?)
            }
            Q::CreateApiKey(_) => A::CreatedApiKey(de(body)?),
            Q::ApiKeys => A::ApiKeys(de(body)?),
            Q::Health => A::Health(de(body)?),
            Q::Terrain { .. } => A::Terrain(Box::new(de(body)?)),
            Q::PlayerMe => A::Me(de(body)?),
            Q::PlayerCard { .. } | Q::PlayerCardByName { .. } => A::Card(de(body)?),
            Q::Mutes | Q::Mute { .. } | Q::Unmute { .. } => A::Mutes(de(body)?),
            Q::Letters(_) => A::Mailbox(de(body)?),
            Q::SendLetter(_) => A::Letter(de(body)?),
            Q::ReadLetter { .. } | Q::DropLetter { .. } => A::LettersUnread(de(body)?),
            Q::PlayerMove(_) => A::Move(de(body)?),
            Q::PlayerInventory => A::Inventory(de(body)?),
            Q::PlayerCurrency => A::Currency(de(body)?),
            Q::WorldNearby(_) => A::Nearby(de(body)?),
            Q::WorldNodes => A::AllNodes(de(body)?),
            Q::Extract(_) => A::Extract(de(body)?),
            Q::Containers => A::Containers(de(body)?),
            Q::MoveItem(_) => A::MoveItem(de(body)?),
            Q::SplitStack { .. } => A::SplitStack(de(body)?),
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
            Q::Tutorial | Q::SetTutorialDismissed(_) => A::Tutorial(de(body)?),
            Q::ZoneFacts => A::ZoneFacts(de(body)?),
            Q::FreeLots(_) => A::FreeLots(de(body)?),
            Q::Subscribe(_) => A::Subscription(de(body)?),
            Q::Grant(_) => A::Granted(de(body)?),
            Q::PlaceOrder(_) | Q::CancelOrder { .. } => A::Order(de(body)?),
            Q::MarketDepth(_) => A::MarketDepth(de(body)?),
            Q::MyOrders(_) => A::MyOrders(de(body)?),
            Q::MarketHistory(_) => A::MarketHistory(de(body)?),
            Q::MarketSummary => A::MarketSummary(de(body)?),
            Q::MarketListings => A::MarketListings(de(body)?),
            Q::MarketBox => A::MarketBox(de(body)?),
            Q::CollectMarketBox(_) | Q::CollectImpound(_) => A::Collected(de(body)?),
            Q::Gazette { .. } => A::Gazette(Box::new(de(body)?)),
            Q::SetByline(_) => A::Byline(de(body)?),
            Q::ConfigureAutomation { station_id, .. }
            | Q::StartAutomation { station_id }
            | Q::StopAutomation { station_id } => A::Automation {
                station_id: *station_id,
                automation: de(body)?,
            },
            Q::Bazaar => A::Bazaar(de(body)?),
            Q::MyStall => A::MyStall(de(body)?),
            Q::Shop { .. } | Q::RentStall { .. } => A::Shop(de(body)?),
            Q::Stock(_) => A::Listed(de(body)?),
            // The body is `null`
            Q::Unlist { listing_id } => A::Unlisted { listing_id: *listing_id },
            Q::Buy { listing_id, .. } => A::Bought {
                listing_id: *listing_id,
                purchase: de(body)?,
            },
            Q::CreateCitizen(_) => A::Citizen(Box::new(de(body)?)),
            Q::Citizens => A::Citizens(de(body)?),
            Q::Ledger => A::Ledger(Box::new(de(body)?)),
            Q::TownKey => A::CreatedApiKey(de(body)?),
            Q::TownReport(_) => A::Success(de(body)?),
            Q::Series(_) => A::Series(Box::new(de(body)?)),
            Q::Annotations(_) => A::Annotations(de(body)?),
            Q::Tunables | Q::SetTunables(_) => A::Tunables(de(body)?),
            Q::Walkers(_) => A::Walkers(de(body)?),
        })
    }
}

/// `text` as one path segment: everything but the unreserved characters
/// percent-encoded, so a name with a space, a slash or a `?` stays one
/// segment (RFC 3986 §2.3).
fn path_segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
        match token {
            // A bot's API key (E0.9). It doesn't expire, so it's never refreshed.
            Some(key) if key.starts_with(API_KEY_PREFIX) => {
                builder = builder.header(reqwest::header::AUTHORIZATION, format!("ApiKey {key}"));
            }
            Some(token) => builder = builder.bearer_auth(token),
            None => {}
        }
        builder.send()
    };

    // A `429` means the server's rate limit refused the request before acting
    // on it (E0.7), so any method is safe to send again. Wait as long as
    // `Retry-After` says, a few times, and only then report it.
    let mut limited = 0;
    let response = loop {
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
        if response.status() != reqwest::StatusCode::TOO_MANY_REQUESTS
            || limited == RATE_LIMIT_RETRIES
        {
            break response;
        }
        limited += 1;
        let wait = retry_after(response.headers());
        tracing::debug!(
            "{} {} rate limited; sending again in {wait:?}",
            request.method(),
            request.path()
        );
        tokio::time::sleep(wait).await;
    };
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

/// How many times a rate-limited request is sent again before the `429` is
/// reported.
const RATE_LIMIT_RETRIES: u32 = 4;
/// The longest a `Retry-After` is honoured for, and the wait when there's none.
const RETRY_AFTER_MAX: Duration = Duration::from_secs(30);

/// How long a `429` asks us to wait: `Retry-After` in seconds, at least one
/// and at most [`RETRY_AFTER_MAX`].
fn retry_after(headers: &reqwest::header::HeaderMap) -> Duration {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|secs| Duration::from_secs(secs.max(1)).min(RETRY_AFTER_MAX))
        .unwrap_or(RETRY_AFTER_MAX)
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
            (RestRequest::ApiKeys, "GET", "/account/api-keys".into()),
            (
                RestRequest::RevokeApiKey { key_id: id },
                "DELETE",
                format!("/account/api-keys/{id}"),
            ),
            (RestRequest::Recipes, "GET", "/recipes".into()),
            (RestRequest::Catalog, "GET", "/catalog".into()),
            (RestRequest::PlayerMe, "GET", "/player/me".into()),
            (RestRequest::PlayerCard { player_id: id }, "GET", format!("/players/{id}/card")),
            (RestRequest::Letters(Default::default()), "GET", "/letters".into()),
            (
                RestRequest::SendLetter(SendLetter { to: id, body: "Hello".into() }),
                "POST",
                "/letters".into(),
            ),
            (RestRequest::ReadLetter { letter_id: id }, "POST", format!("/letters/{id}/read")),
            (RestRequest::DropLetter { letter_id: id }, "DELETE", format!("/letters/{id}")),
            (
                RestRequest::PlayerCardByName { name: "Ada Reeder/2?é".into() },
                "GET",
                "/players/by-name/Ada%20Reeder%2F2%3F%C3%A9".into(),
            ),
            (RestRequest::Mutes, "GET", "/player/mutes".into()),
            (RestRequest::Mute { player_id: id }, "PUT", format!("/players/{id}/mute")),
            (RestRequest::Unmute { player_id: id }, "DELETE", format!("/players/{id}/mute")),
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
                RestRequest::ClaimPlot { plot_id: id, use_grant: false },
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
            (RestRequest::Tutorial, "GET", "/tutorial".into()),
            (
                RestRequest::SetTutorialDismissed(capitol_protocol::tutorial::SetDismissed { dismissed: true }),
                "PUT",
                "/tutorial/dismissed".into(),
            ),
            (
                RestRequest::ZoneFacts,
                "GET",
                "/bureaucracy/zone-facts".into(),
            ),
            (
                RestRequest::Subscribe(SubscribeRequest { days: 30 }),
                "POST",
                "/player/subscription".into(),
            ),
            (
                RestRequest::Grant(GrantRequest::default()),
                "POST",
                "/player/grant".into(),
            ),
            (
                RestRequest::MarketDepth(Default::default()),
                "GET",
                "/market/orders".into(),
            ),
            (
                RestRequest::CancelOrder { order_id: id },
                "DELETE",
                format!("/market/orders/{id}"),
            ),
            (
                RestRequest::MyOrders(Default::default()),
                "GET",
                "/market/orders/mine".into(),
            ),
            (
                RestRequest::MarketHistory(Default::default()),
                "GET",
                "/market/history".into(),
            ),
            (RestRequest::MarketSummary, "GET", "/market/summary".into()),
            (RestRequest::MarketListings, "GET", "/market/listings".into()),
            (RestRequest::MarketBox, "GET", "/market/box".into()),
            (
                RestRequest::CollectMarketBox(Default::default()),
                "POST",
                "/market/box/collect".into(),
            ),
            (
                RestRequest::CollectImpound(Default::default()),
                "POST",
                "/inventory/impound/collect".into(),
            ),
            (RestRequest::Gazette { day: None }, "GET", "/gazette/latest".into()),
            (RestRequest::Gazette { day: Some(248_952) }, "GET", "/gazette".into()),
            (
                RestRequest::SetByline(GazetteByline { named: false }),
                "PUT",
                "/gazette/byline".into(),
            ),
            (
                RestRequest::ConfigureAutomation {
                    station_id: id,
                    request: ConfigureAutomation { recipe_id: "plank".into(), mode: AutomationMode::Forever },
                },
                "PUT",
                format!("/stations/{id}/automation"),
            ),
            (
                RestRequest::StartAutomation { station_id: id },
                "POST",
                format!("/stations/{id}/automation/start"),
            ),
            (
                RestRequest::StopAutomation { station_id: id },
                "POST",
                format!("/stations/{id}/automation/stop"),
            ),
            (RestRequest::Bazaar, "GET", "/bazaar".into()),
            (
                RestRequest::CreateCitizen(CreateCitizenRequest { name: "Ada Reeder".into() }),
                "POST",
                "/admin/citizens".into(),
            ),
            (RestRequest::Citizens, "GET", "/admin/citizens".into()),
            (RestRequest::Ledger, "GET", "/admin/ledger".into()),
            (RestRequest::TownKey, "POST", "/admin/town/key".into()),
            (RestRequest::TownReport(Box::default()), "POST", "/admin/town/report".into()),
            (RestRequest::Series(Default::default()), "GET", "/admin/series".into()),
            (RestRequest::Annotations(Default::default()), "GET", "/admin/annotations".into()),
            (RestRequest::Tunables, "GET", "/admin/tunables".into()),
            (RestRequest::SetTunables(TunablesUpdate { values: Default::default() }), "PUT", "/admin/tunables".into()),
            (
                RestRequest::Walkers(WalkersQuery { x: 1.0, y: 2.0, radius: 3.0 }),
                "GET",
                "/admin/walkers".into(),
            ),
            (RestRequest::MyStall, "GET", "/bazaar/mine".into()),
            (
                RestRequest::Shop { stall_id: "bazaar_stall_3".into() },
                "GET",
                "/bazaar/stalls/bazaar_stall_3".into(),
            ),
            (
                RestRequest::RentStall {
                    stall_id: "bazaar_stall_3".into(),
                    request: RentStall { days: 1, name: None },
                },
                "POST",
                "/bazaar/stalls/bazaar_stall_3/rent".into(),
            ),
            (
                RestRequest::Stock(StockListing { item_id: id, quantity: 1, price: 1, qualities: None }),
                "POST",
                "/bazaar/mine/listings".into(),
            ),
            (
                RestRequest::Unlist { listing_id: id },
                "DELETE",
                format!("/bazaar/listings/{id}"),
            ),
            (
                RestRequest::Buy {
                    listing_id: id,
                    request: BuyListing { quantity: 1, qualities: None },
                },
                "POST",
                format!("/bazaar/listings/{id}/buy"),
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
    fn an_unlist_decodes_its_null_as_the_listing_gone() {
        let listing_id = Uuid::from_u128(7);
        let response = RestRequest::Unlist { listing_id }.decode(b"null").unwrap();
        assert_eq!(response, RestResponse::Unlisted { listing_id });
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
