//! OpenAPI documentation configuration for The Capitol backend.

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::models::{
    AccountInfo, Bounds, InventoryItem, PlotInfo, PlotPermissions, RecipeInfo,
    RecipeInputInfo, RecipeOutputInfo, ResourceNodeInfo, SlotItem, StationInfo, ZoneInfo,
};

use crate::routes::auth::{AuthResponse, LoginRequest, LogoutRequest, RegisterRequest};
use crate::routes::bureaucracy::{DeedPrice, DeedPricesResponse, ZonesResponse};
use crate::routes::exchange::{DepositRequest, DepositResponse, WithdrawRequest, WithdrawResponse};
use crate::routes::health::HealthResponse;
use crate::routes::inventory::{ContainerResponse, ContainersResponse, MoveItemRequest, MoveItemResponse};
use crate::routes::player::{CurrencyResponse, InventoryResponse, MoveRequest};
use crate::routes::plots::{ClaimPlotResponse, ListPlotsResponse};
use crate::routes::recipes::RecipesResponse;
use crate::routes::stations::{NearbyStationsResponse, PlaceStationRequest, PlaceStationResponse, StationContainerResponse};
use crate::routes::terrain::{BridgeResponse, RoadResponse, TerrainResponse, WaterRingResponse};
use crate::routes::world::{ExtractRequest, ExtractResponse, NearbyResponse};

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
then include it in the `Authorization: Bearer <token>` header.",
        license(name = "MIT")
    ),
    servers(
        (url = "http://localhost:3000", description = "Local development server")
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Authentication", description = "User registration, login, and logout"),
        (name = "Health", description = "Server health checks"),
        (name = "Player", description = "Player state and movement"),
        (name = "World", description = "World state, resource nodes, and extraction"),
        (name = "Inventory", description = "Container and item management"),
        (name = "Recipes", description = "Crafting recipe information"),
        (name = "Stations", description = "Crafting and storage stations"),
        (name = "Exchange", description = "Currency exchange operations"),
        (name = "Plots", description = "Land plot ownership and permissions"),
        (name = "Bureaucracy", description = "Zone information and deed prices"),
        (name = "Terrain", description = "World terrain data (roads, water)")
    ),
    paths(
        // Auth
        crate::routes::auth::register,
        crate::routes::auth::login,
        crate::routes::auth::logout,
        // Health
        crate::routes::health::health_check,
        // Player
        crate::routes::player::move_player,
        crate::routes::player::get_inventory,
        crate::routes::player::get_currency,
        // World
        crate::routes::world::get_nearby,
        crate::routes::world::start_extraction,
        // Inventory
        crate::routes::inventory::get_containers,
        crate::routes::inventory::move_item,
        // Recipes
        crate::routes::recipes::list_recipes,
        // Stations
        crate::routes::stations::get_nearby_stations,
        crate::routes::stations::place_station,
        crate::routes::stations::remove_station,
        crate::routes::stations::get_station_container,
        // Exchange
        crate::routes::exchange::deposit_fiber,
        crate::routes::exchange::withdraw_fiber,
        // Plots
        crate::routes::plots::list_plots,
        crate::routes::plots::get_plot,
        crate::routes::plots::get_plot_at_position,
        crate::routes::plots::claim_plot,
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
        AuthResponse,
        AccountInfo,
        // Health
        HealthResponse,
        // Player
        MoveRequest,
        InventoryResponse,
        InventoryItem,
        CurrencyResponse,
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
        // Recipes
        RecipesResponse,
        RecipeInfo,
        RecipeInputInfo,
        RecipeOutputInfo,
        // Stations
        NearbyStationsResponse,
        StationInfo,
        PlaceStationRequest,
        PlaceStationResponse,
        StationContainerResponse,
        // Exchange
        DepositRequest,
        DepositResponse,
        WithdrawRequest,
        WithdrawResponse,
        // Plots
        ListPlotsResponse,
        PlotInfo,
        Bounds,
        ClaimPlotResponse,
        PlotPermissions,
        // Bureaucracy
        ZonesResponse,
        ZoneInfo,
        DeedPricesResponse,
        DeedPrice,
        // Terrain
        TerrainResponse,
        WaterRingResponse,
        BridgeResponse,
        RoadResponse,
    ))
)]
pub struct ApiDoc;
