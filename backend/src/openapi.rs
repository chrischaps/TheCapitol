//! OpenAPI documentation configuration for The Capitol backend.

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use capitol_protocol::rest::{
    AccountInfo, ApiKeyInfo, ApiKeyScope, CreateApiKeyRequest, CreatedApiKey, AuthResponse, Bounds, CatalogContainerType, CatalogItemType, CatalogTool, CatalogResourceType,
    CatalogResponse, CatalogStationType, ClaimPlotResponse, ClaimedPlot, ContainerResponse, ContainersResponse,
    CurrencyResponse, DeedPrice, DeedPricesResponse, DepositRequest, DepositResponse,
    ExtractRequest, ExtractResponse, HealthResponse, InventoryItem, InventoryResponse,
    ListPlotsResponse, LoginRequest, LogoutRequest, RefreshRequest, MoveItemRequest, MoveItemResponse,
    MoveRequest, NearbyResponse, NearbyStationsResponse, PlaceStationRequest,
    PayTaxRequest, PayTaxResponse, PlaceStationResponse, PlayerMeResponse, PlotInfo, PlotPermissions, RecipeInfo, RecipeInputInfo,
    RecipeOutputInfo, RecipesResponse, RegisterRequest, ResourceNodeInfo, SlotItem, SplitStackRequest, SplitStackResponse,
    StationContainerResponse, StationInfo, StationTool, StationAutomation, AutomationMode, AutomationState, PauseReason, ConfigureAutomation, SubscribeRequest, Subscription, SubscriptionStatus, WithdrawRequest, WithdrawResponse, ZoneInfo,
    ZonesResponse,
};
use capitol_protocol::rest::{
    Candle, CollectRequest, CollectResponse, DepthLevel, MarketDepthResponse, MarketHistoryResponse,
    MarketItemSummary, MarketOrder, MarketSummaryResponse, MarketTrade, PlaceOrder,
};
use capitol_protocol::rules::market::{GradeRule, OrderSide, OrderStatus};
use capitol_protocol::terrain::{Bridge, RoadSegment, RoadType, TerrainPark, TerrainPlot, TerrainResponse, WaterRing};

/// Security scheme modifier for JWT Bearer authentication
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .description(Some("JWT token obtained from /auth/login or /auth/register"))
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "The Capitol API",
        version = "0.1.0",
        description = "Backend API for The Capitol - a resource extraction and crafting game.

Most endpoints require JWT authentication. Obtain a token via `/auth/login` or `/auth/register`,
then include it in the `Authorization: Bearer <token>` header. A bot can use an API key
from `/account/api-keys` instead, as `Authorization: ApiKey <secret>`, within its scopes.",
        license(name = "MIT")
    ),
    servers(
        (url = "http://localhost:3000", description = "Local development server")
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Authentication", description = "User registration, login, and logout"),
        (name = "Account", description = "API keys for bots"),
        (name = "Health", description = "Server health checks"),
        (name = "Player", description = "Player state and movement"),
        (name = "World", description = "World state, resource nodes, and extraction"),
        (name = "Inventory", description = "Container and item management"),
        (name = "Recipes", description = "Crafting recipe information"),
        (name = "Catalog", description = "Static game data: item, station, resource and container types, and recipes"),
        (name = "Stations", description = "Crafting and storage stations"),
        (name = "Exchange", description = "Currency exchange operations"),
        (name = "Market", description = "Orders, books, price history and the market box"),
        (name = "Plots", description = "Land plot ownership and permissions"),
        (name = "Bureaucracy", description = "Zone information and deed prices"),
        (name = "Terrain", description = "World terrain data (roads, water)")
    ),
    paths(
        // Auth
        crate::routes::auth::register,
        crate::routes::auth::login,
        crate::routes::auth::refresh,
        crate::routes::auth::logout,
        // Account
        crate::routes::account::create_api_key,
        crate::routes::account::list_api_keys,
        crate::routes::account::revoke_api_key,
        // Health
        crate::routes::health::health_check,
        // Catalog
        crate::routes::catalog::get_catalog,
        // Player
        crate::routes::player::get_me,
        crate::routes::player::move_player,
        crate::routes::player::get_inventory,
        crate::routes::player::get_currency,
        crate::routes::player::mock_subscribe,
        // World
        crate::routes::world::get_nearby,
        crate::routes::world::get_all_nodes,
        crate::routes::world::start_extraction,
        // Inventory
        crate::routes::inventory::get_containers,
        crate::routes::inventory::move_item,
        crate::routes::inventory::split_stack,
        // Recipes
        crate::routes::recipes::list_recipes,
        // Stations
        crate::routes::stations::get_nearby_stations,
        crate::routes::stations::place_station,
        crate::routes::stations::remove_station,
        crate::routes::stations::get_station_container,
        crate::routes::stations::configure_automation,
        crate::routes::stations::start_automation,
        crate::routes::stations::stop_automation,
        // Exchange
        crate::routes::exchange::deposit_fiber,
        crate::routes::exchange::withdraw_fiber,
        // Market
        crate::routes::market::place_order,
        crate::routes::market::cancel_order,
        crate::routes::market::get_depth,
        crate::routes::market::my_orders,
        crate::routes::market::history,
        crate::routes::market::summary,
        crate::routes::market::get_box,
        crate::routes::market::collect,
        // Plots
        crate::routes::plots::list_plots,
        crate::routes::plots::list_claimed_plots,
        crate::routes::plots::get_plot,
        crate::routes::plots::get_plot_at_position,
        crate::routes::plots::claim_plot,
        crate::routes::plots::pay_tax,
        crate::routes::plots::get_permissions,
        crate::routes::plots::update_permissions,
        // Bureaucracy
        crate::routes::bureaucracy::get_zones,
        crate::routes::bureaucracy::get_deed_prices,
        // Terrain
        crate::routes::terrain::get_terrain,
    ),
    components(schemas(
        // Auth
        RegisterRequest,
        LoginRequest,
        LogoutRequest,
        RefreshRequest,
        AuthResponse,
        AccountInfo,
        // Account
        CreateApiKeyRequest,
        CreatedApiKey,
        ApiKeyInfo,
        ApiKeyScope,
        // Health
        HealthResponse,
        // Player
        PlayerMeResponse,
        MoveRequest,
        InventoryResponse,
        InventoryItem,
        CurrencyResponse,
        Subscription,
        SubscriptionStatus,
        SubscribeRequest,
        // World
        NearbyResponse,
        ResourceNodeInfo,
        ExtractRequest,
        ExtractResponse,
        // Inventory
        ContainerResponse,
        ContainersResponse,
        SlotItem,
        MoveItemRequest,
        MoveItemResponse,
        SplitStackRequest,
        SplitStackResponse,
        // Recipes
        RecipesResponse,
        CatalogResponse,
        CatalogItemType,
        CatalogTool,
        CatalogStationType,
        CatalogResourceType,
        CatalogContainerType,
        RecipeInfo,
        RecipeInputInfo,
        RecipeOutputInfo,
        // Stations
        NearbyStationsResponse,
        StationInfo,
        StationAutomation,
        AutomationMode,
        AutomationState,
        PauseReason,
        ConfigureAutomation,
        StationTool,
        PlaceStationRequest,
        PlaceStationResponse,
        StationContainerResponse,
        // Exchange
        DepositRequest,
        DepositResponse,
        WithdrawRequest,
        WithdrawResponse,
        // Market
        PlaceOrder,
        MarketOrder,
        MarketTrade,
        OrderSide,
        GradeRule,
        OrderStatus,
        DepthLevel,
        MarketDepthResponse,
        Candle,
        MarketHistoryResponse,
        MarketItemSummary,
        MarketSummaryResponse,
        CollectRequest,
        CollectResponse,
        // Plots
        ListPlotsResponse,
        PlotInfo,
        Bounds,
        ClaimPlotResponse,
        PayTaxRequest,
        PayTaxResponse,
        ClaimedPlot,
        PlotPermissions,
        // Bureaucracy
        ZonesResponse,
        ZoneInfo,
        DeedPricesResponse,
        DeedPrice,
        // Terrain
        TerrainResponse,
        WaterRing,
        Bridge,
        RoadSegment,
        RoadType,
        TerrainPlot,
        TerrainPark,
    ))
)]
pub struct ApiDoc;
