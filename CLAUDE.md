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

engine/
  tick.rs        - Core tick loop (100ms), GameState struct, command processing
  commands.rs    - Command enum (Move, Extract, Craft, Trade, etc.)
  events.rs      - GameEvent enum for WebSocket broadcasts
  movement.rs    - Player movement with terrain collision
  extraction.rs  - Resource gathering logic
  crafting.rs    - Recipe processing, quality cascade
  stations.rs    - Station placement and interaction
  trading.rs     - P2P trade sessions
  currency.rs    - Strand/Fiber exchange operations
  property_tax.rs - Hourly tax processing

models/          - Database types and game state structs
  player.rs      - PlayerState, ActionState enum
  item.rs        - Item, ItemType
  resource.rs    - ResourceNode, ResourceType
  station.rs     - Station, StationType, StationState
  recipe.rs      - Recipe with quality weights
  zone.rs        - Zone enum (Capitol, Trade, Urban, Rural, etc.)
  terrain.rs     - TerrainConfig (moats, canals, bridges)
  plot.rs        - Plot ownership and permissions

routes/          - Axum REST handlers
  auth.rs        - Register, login, JWT
  player.rs      - Player state endpoints
  world.rs       - Nearby entities, extraction
  inventory.rs   - Slot-based inventory operations
  stations.rs    - Station CRUD
  exchange.rs    - Currency conversion
  plots.rs       - Property system

ws/              - WebSocket handlers
  mod.rs         - Connection upgrade, message routing
  connection.rs  - Per-connection state, event subscription
```

### Client Structure (`client/src/`)

```
App.tsx          - Router setup
pages/
  Login.tsx, Register.tsx, Game.tsx

components/
  GameCanvas.tsx       - 2D world rendering
  InventoryPanel.tsx   - Slot-based inventory grid
  CraftingPanel.tsx    - Recipe selection and crafting
  StationPanel.tsx     - Station interaction
  TradeWindow.tsx      - P2P trading UI
  PlotPanel.tsx        - Property management

contexts/
  AuthContext.tsx      - JWT and auth state
  GameContext.tsx      - WebSocket connection, game state
  DragDropContext.tsx  - Inventory drag-drop

api/                   - REST API client functions
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
- M7: Zones, plots, terrain (moats/canals/bridges), property tax

See `the-capitol-milestones.md` for detailed checklist.
