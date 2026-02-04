# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

The Capitol is an MMO economic simulation where the economy is the primary gameplay. Players compete to build wealth through gathering, crafting, trading, and automation in a persistent shared world. No combat - all progression comes from economic activities.

**Reference Documents:**
- `the-capitol-gdd.md` - Game design, systems, quality cascade mechanics
- `the-capitol-tdd.md` - Technical architecture, protocol spec, data models
- `the-capitol-milestones.md` - Development roadmap (M0-M10)

## Development Commands

### Backend (Rust/Axum)
```bash
# Start PostgreSQL and Redis containers
docker-compose up -d

# Run backend server (from repo root or backend/)
cargo run

# Run with verbose logging
RUST_LOG=debug cargo run

# Check for compilation errors
cargo check

# Run tests
cargo test

# Run a specific test (terrain generation tests are slow ~2min)
cargo test test_no_roads_terminate -- --nocapture
```

### Client (React/TypeScript/Vite)
```bash
cd client

# Install dependencies
npm install

# Run development server
npm run dev

# Type check and build
npm run build

# Lint
npm run lint
```

### Environment Setup
Copy `backend/.env.example` to `backend/.env`. Default config:
- PostgreSQL on port 5433 (not 5432, to avoid conflicts)
- Redis on port 6379
- Backend API on port 3000

## Architecture

### Backend Structure (`backend/src/`)

```
main.rs          - Server startup, loads game data from DB into memory
config.rs        - Environment configuration
state.rs         - AppState (DB pool, Redis, GameState reference)
error.rs         - Error types
openapi.rs       - Swagger/OpenAPI spec generation

middleware/
  auth.rs        - JWT extraction middleware (AuthUser)

engine/
  tick.rs        - Core tick loop (100ms), GameState struct, command processing
  commands.rs    - Command enum (Move, Extract, Craft, Trade, etc.)
  events.rs      - GameEvent enum for WebSocket broadcasts
  movement.rs    - Player movement with terrain collision (water ring blocking)
  extraction.rs  - Resource gathering logic
  crafting.rs    - Recipe processing, quality cascade
  inventory.rs   - Inventory slot operations (move, merge, find)
  stations.rs    - Station placement and interaction
  regeneration.rs - Resource node regeneration
  trading.rs     - P2P trade sessions and trade manager
  currency.rs    - Strand/Fiber exchange operations
  property_tax.rs - Hourly tax processing

models/          - Database types and game state structs
  account.rs     - Account, AccountInfo (includes is_admin flag)
  player.rs      - PlayerState, ActionState enum
  item.rs        - Item, ItemType
  container.rs   - Container, ContainerType, SlotItem, ContainerState
  resource.rs    - ResourceNode, ResourceType
  station.rs     - Station, StationType, StationState
  recipe.rs      - Recipe with quality weights
  zone.rs        - Zone enum (Capitol, Trade, Urban, Rural, etc.)
  terrain.rs     - TerrainConfig, procedural road generation, water rings, bridges
  plot.rs        - Plot ownership, permissions, property tax

routes/          - Axum REST handlers
  auth.rs        - Register, login, JWT (includes is_admin in token)
  health.rs      - Health check endpoint
  player.rs      - Player state endpoints
  world.rs       - Nearby entities, extraction
  inventory.rs   - Slot-based inventory operations
  recipes.rs     - Recipe listing
  stations.rs    - Station CRUD
  exchange.rs    - Currency conversion
  plots.rs       - Property system
  terrain.rs     - Terrain data endpoint (roads, water, plots)
  bureaucracy.rs - Government/bureaucracy endpoints

ws/              - WebSocket handlers
  mod.rs         - Connection upgrade, message routing
  connection.rs  - Per-connection state, zone-based subscriptions, trade events
```

### Client Structure (`client/src/`)

```
App.tsx          - Router setup
pages/
  Login.tsx      - Login form
  Register.tsx   - Registration form
  Game.tsx       - Main game page (panels, canvas, context providers)

components/
  GameCanvas.tsx              - 2D world rendering (terrain, roads, water, players, debug camera)
  InventoryPanel.tsx          - Slot-based inventory panel
  InventoryGrid.tsx           - Inventory grid layout
  InventorySlot.tsx           - Individual inventory slot
  CraftingPanel.tsx           - Recipe selection and crafting
  StationPanel.tsx            - Station interaction
  ExchangePanel.tsx           - Strand/Fiber currency exchange
  TradeWindow.tsx             - P2P trading UI
  TradeRequestNotification.tsx - Incoming trade request popup
  PlotPanel.tsx               - Property management
  ZoneIndicator.tsx           - Current zone display

contexts/
  AuthContext.tsx      - JWT and auth state (includes isAdmin)
  GameContext.tsx      - WebSocket connection, game state, trade events
  DragDropContext.tsx  - Inventory drag-drop

api/
  auth.ts         - Login, register, token refresh
  player.ts       - Player state
  world.ts        - Nearby entities
  inventory.ts    - Inventory operations
  recipes.ts      - Recipe listing
  stations.ts     - Station CRUD
  currency.ts     - Currency balance
  exchange.ts     - Currency exchange
  plots.ts        - Plot management
  terrain.ts      - Terrain data (roads, water, suburban plots)
  bureaucracy.ts  - Government endpoints
```

### Love2D Client (`love-client/src/`)

Alternative native client built with Love2D and Lua.

```
config.lua       - Server URL, game settings
terrain.lua      - Terrain rendering (roads, water, bridges)

net/
  api.lua        - REST API client
  ws.lua         - WebSocket client
  protocol.lua   - Message serialization

scenes/
  login.lua      - Login screen
  loading.lua    - Asset loading, terrain fetch
  game.lua       - Main game scene

state/
  auth.lua       - Auth token management
  game.lua       - Game state (players, resources, terrain)
  ui.lua         - UI state

render/
  world.lua      - World rendering (terrain, zones, resources)
  players.lua    - Player sprite rendering
  effects.lua    - Visual effects

ui/
  button.lua     - Button widget
  input.lua      - Text input widget
  inventory.lua  - Inventory UI
```

### Tick System

The game runs on a 100ms server tick. Different systems process at different rates:
- **Movement**: Every tick (100ms)
- **Crafting/Extraction**: Every 5 ticks (500ms)
- **Automation/Market**: Every 10 ticks (1s)
- **Regeneration**: Every 600 ticks (60s)
- **Property Tax**: Every 36000 ticks (1hr)

Commands are queued via REST/WebSocket and processed at tick boundaries. State changes emit GameEvents that fan out to subscribed WebSocket clients.

### Quality Cascade

Every item has quality 0-100. Quality flows through production:
```
output_quality = Σ(input_quality × weight) + (station_quality × weight) + variance
```
Recipe weights are defined in `recipes` table. See GDD Section 6 for details.

### World Geography

Concentric zones from center outward:
Capitol → Bailey → Trade District → Guild District → Urban → Suburban → Rural → Wilderness

Moats and canals are water barriers. Players can only cross at bridges. Movement is blocked by water unless at a crossing point.

### Suburban Road Generation

The suburban zone (radius 1600-2400) uses procedural road generation in `terrain.rs`:

**Road hierarchy**: Arterials (8 radial spokes) → Collectors (curved roads between arterials) → Local streets (cul-de-sacs, loops, P-shaped)

**Key algorithms**:
- Collector roads use Catmull-Rom splines with S-curve wobble through control points
- Local streets branch from connection points along existing roads
- Index-based parent road exclusion prevents collision with the road a street branches from
- Streets use adaptive length based on available depth (up to 500 units)
- Min spacing of 100 units between streets for plot placement
- All points clamped to annular zone boundaries

**Generation phases**:
1. Generate collector roads (curved paths between arterials)
2. Iteratively generate local streets (each pass can branch from previous pass)
3. Generate suburban plots along roads

## Database

PostgreSQL with sqlx. Migrations in `backend/migrations/`.

Key tables: `accounts`, `players`, `items`, `containers`, `resource_nodes`, `stations`, `plots`

Static data tables: `item_types`, `resource_types`, `recipes`, `station_types`, `container_types`

## API Documentation

Swagger UI available at `/swagger-ui` when server is running. OpenAPI spec at `/api-doc/openapi.json`.

## Current Milestone Progress

Implementation through M7 (Property system). Key completed systems:
- M1: Auth, movement, WebSocket real-time sync
- M2: Resource extraction, slot-based inventory
- M3: Crafting with quality cascade
- M4: Stations, station crafting
- M5: Currency exchange, P2P trading
- M7: Zones, plots, terrain (moats/canals/bridges, procedural suburban roads), property tax, admin debug camera

See `the-capitol-milestones.md` for detailed checklist.
