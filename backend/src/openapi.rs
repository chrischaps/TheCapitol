//! OpenAPI documentation configuration for The Capitol backend.

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use capitol_protocol::rest::{
    AccountInfo, AccountKind, ApiKeyInfo, ApiKeyScope, CreateApiKeyRequest, CreatedApiKey, AuthResponse, Bounds, CatalogContainerType, CatalogItemType, CatalogTool, CatalogResourceType,
    CatalogResponse, CatalogStationType, ClaimPlotResponse, ClaimedPlot, ContainerResponse, ContainersResponse,
    CurrencyResponse, DeedPrice, DeedPricesResponse, DepositRequest, DepositResponse,
    ExtractRequest, ExtractResponse, HealthResponse, InventoryItem, InventoryResponse,
    ListPlotsResponse, LoginRequest, LogoutRequest, RefreshRequest, MoveItemRequest, MoveItemResponse,
    MoveRequest, NearbyResponse, NearbyStationsResponse, PlaceStationRequest,
    PayTaxRequest, PayTaxResponse, PlaceStationResponse, PlayerCard, SendLetter, Correspondent, Letter, LetterBox, Mailbox, LettersUnread, PlayerMeResponse, MutesResponse, MutedPlayer, PlotInfo, PlotPermissions, RecipeInfo, RecipeInputInfo,
    RecipeOutputInfo, RecipesResponse, RegisterRequest, ResourceNodeInfo, SlotItem, SplitStackRequest, SplitStackResponse,
    StationContainerResponse, StationInfo, StationTool, StationAutomation, Produced, AutomationMode, AutomationState, PauseReason, ConfigureAutomation, SubscribeRequest, Subscription, GrantRequest, GrantItem, GrantResponse, SubscriptionStatus, WithdrawRequest, WithdrawResponse, ZoneInfo,
    ZonesResponse,
};
use capitol_protocol::rest::{
    Candle, CollectRequest, CollectResponse, DepthLevel, MarketDepthResponse, MarketHistoryResponse,
    MarketItemSummary, MarketOrder, MarketSummaryResponse, MarketTrade, MyFill, PlaceOrder,
};
use capitol_protocol::rest::{
    BazaarDirectory, BazaarListing, BazaarPurchase, BazaarShop, BazaarStall, BuyListing, MyStallResponse, RentStall,
    ShopSign, StockListing,
};
use capitol_protocol::rest::{
    Gazette, GazetteByline, GazetteDeeds, GazetteMade, GazetteMover, GazettePrice, GazetteProducer, GazetteResponse,
    GazetteSale, GazetteShares, GazetteShop, GazetteStalls,
};
use capitol_protocol::rest::{
    ClaimCommission, ClaimResponse, ClaimsResponse, Commission, CommissionClaim, CommissionDelivery, CommissionsResponse,
    DeliverCommission, PostCommission,
};
use capitol_protocol::rules::court::{ClaimStatus, CommissionStatus};
use capitol_protocol::rest::{
    FoundGuild, GuildDetail, GuildInvite, GuildInviteKind, GuildMember, GuildSummary, GuildsResponse, MyGuild, SetGuildRole,
};
use capitol_protocol::rules::guild::GuildRole;
use capitol_protocol::rest::{OfferWages, ShiftEnded, WageOffer, WageShift, WagesResponse};
use capitol_protocol::rules::market::{GradeRule, OrderSide, OrderStatus};
use capitol_protocol::terrain::{
    Bridge, Landmark, LandmarkKind, RoadSegment, RoadType, TerrainPark, TerrainPlot, TerrainResponse, WaterRing,
};

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
from `/account/api-keys` instead, as `Authorization: ApiKey <secret>`, within its scopes.
Bots are for subscribers: minting a key, and signing in with one, take a running subscription.",
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
        (name = "Tutorial", description = "The road in: where you are on the tutorial"),
        (name = "Letters", description = "Letters that wait for someone who's away, for a Strand's postage"),
        (name = "Guilds", description = "The guild registry: founding, roles and the roster"),
        (name = "Wages", description = "Piece-rate work at someone else's station or on their lot, the pay escrowed up front"),
        (name = "World", description = "World state, resource nodes, and extraction"),
        (name = "Inventory", description = "Container and item management"),
        (name = "Recipes", description = "Crafting recipe information"),
        (name = "Catalog", description = "Static game data: item, station, resource and container types, and recipes"),
        (name = "Stations", description = "Crafting and storage stations"),
        (name = "Exchange", description = "Currency exchange operations"),
        (name = "Market", description = "Orders, books, price history and the market box"),
        (name = "Bazaar", description = "Stalls, the shops merchants keep in them, and buying from them"),
        (name = "Commissions", description = "The hall's board: standing requests with the money down, claimed from anywhere and delivered at the Exchange hall"),
        (name = "Gazette", description = "The Capitol Gazette: a daily digest of the economy"),
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
        crate::routes::player::dev_grant,
        crate::routes::players::get_card,
        crate::routes::players::get_card_by_name,
        crate::routes::players::mute,
        crate::routes::players::unmute,
        crate::routes::player::get_mutes,
        // Letters
        crate::routes::letters::send,
        crate::routes::letters::get_letters,
        crate::routes::letters::read,
        crate::routes::letters::drop_letter,
        // The road in
        crate::routes::tutorial::get_tutorial,
        crate::routes::tutorial::set_dismissed,
        // Guilds
        crate::routes::guilds::list,
        crate::routes::guilds::found,
        crate::routes::guilds::get_guild,
        crate::routes::guilds::apply,
        crate::routes::guilds::invite,
        crate::routes::guilds::withdraw,
        crate::routes::guilds::leave,
        crate::routes::guilds::expel,
        crate::routes::guilds::set_role,
        crate::routes::player::get_guild,
        // Wages
        crate::routes::wages::offer,
        crate::routes::wages::get_wages,
        crate::routes::wages::get_offer,
        crate::routes::wages::withdraw,
        crate::routes::wages::take,
        crate::routes::wages::stop,
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
        // Bazaar
        crate::routes::bazaar::get_directory,
        crate::routes::bazaar::my_stall,
        crate::routes::bazaar::get_shop,
        crate::routes::bazaar::rent,
        crate::routes::bazaar::stock,
        crate::routes::bazaar::unlist,
        crate::routes::bazaar::buy,
        // Commissions
        crate::routes::commissions::post_commission,
        crate::routes::commissions::list_commissions,
        crate::routes::commissions::get_commission,
        crate::routes::commissions::cancel_commission,
        crate::routes::commissions::deliver_commission,
        crate::routes::commissions::claim_commission,
        crate::routes::commissions::release_claim,
        crate::routes::commissions::my_claims,
        crate::routes::commissions::commission_claims,
        crate::routes::inventory::get_impound,
        crate::routes::inventory::collect_impound,
        // Gazette
        crate::routes::gazette::latest,
        crate::routes::gazette::by_date,
        crate::routes::gazette::get_byline,
        crate::routes::gazette::set_byline,
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
        AccountKind,
        // Account
        CreateApiKeyRequest,
        CreatedApiKey,
        ApiKeyInfo,
        ApiKeyScope,
        // Health
        HealthResponse,
        // Player
        PlayerMeResponse,
        PlayerCard,
        MutesResponse,
        capitol_protocol::tutorial::TutorialProgress,
        capitol_protocol::tutorial::SetDismissed,
        MutedPlayer,
        // Guilds
        FoundGuild,
        GuildDetail,
        GuildInvite,
        GuildInviteKind,
        GuildMember,
        GuildRole,
        GuildSummary,
        GuildsResponse,
        MyGuild,
        SetGuildRole,
        SendLetter,
        Correspondent,
        Letter,
        LetterBox,
        Mailbox,
        LettersUnread,
        OfferWages,
        WageOffer,
        WageShift,
        WagesResponse,
        ShiftEnded,
        capitol_protocol::rules::wages::Work,
        capitol_protocol::rules::wages::OfferStatus,
        capitol_protocol::rules::wages::ShiftEnd,
        MoveRequest,
        InventoryResponse,
        InventoryItem,
        CurrencyResponse,
        Subscription,
        SubscriptionStatus,
        SubscribeRequest,
        GrantRequest,
        GrantItem,
        GrantResponse,
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
        Produced,
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
        MyFill,
        MarketItemSummary,
        MarketSummaryResponse,
        CollectRequest,
        CollectResponse,
        BazaarDirectory,
        BazaarStall,
        ShopSign,
        BazaarShop,
        BazaarListing,
        MyStallResponse,
        RentStall,
        StockListing,
        BuyListing,
        BazaarPurchase,
        // Commissions
        PostCommission,
        Commission,
        CommissionStatus,
        DeliverCommission,
        CommissionDelivery,
        CommissionsResponse,
        ClaimCommission,
        CommissionClaim,
        ClaimStatus,
        ClaimResponse,
        ClaimsResponse,
        // Gazette
        Gazette,
        GazettePrice,
        GazetteMover,
        GazetteSale,
        GazetteDeeds,
        GazetteProducer,
        GazetteMade,
        GazetteShares,
        GazetteStalls,
        GazetteShop,
        GazetteResponse,
        GazetteByline,
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
        Landmark,
        LandmarkKind,
    ))
)]
pub struct ApiDoc;
