# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

The Capitol is an MMO economic simulation where the economy is the primary gameplay. Players compete to build wealth through gathering, crafting, trading, and automation in a persistent shared world. No combat - all progression comes from economic activities.

**Reference Documents:**
- `the-capitol-gdd.md` - Game design, systems, quality cascade mechanics
- `the-capitol-tdd.md` - Technical architecture, protocol spec, data models
- `the-capitol-milestones.md` - Development roadmap (M0-M10)
- `the-capitol-flint-client.md` - Plan and work breakdown for the 3D Flint engine client (`flint-client/`)
- `the-capitol-suburb-v2.md` - Plan for the post-war neighbourhood suburb generator (issues labelled `suburb-v2`, #103 to #113)

## Current Focus: the Flint Client

Active work is a third client built on Chris's Flint engine (Rust/wgpu). It goes in `flint-client/`, with the engine as a git subtree at `flint-client/engine/`. The plan is `the-capitol-flint-client.md`. The work queue is the GitHub issues labelled `flint-client`: one issue per work package, created in dependency order, with phase labels `fc:P0-…` to `fc:P8-…` and `fc:backend-prereq` for the small server changes the client needs.

**When asked to "tackle the next issue", "do the next one" or "work on #N", use the `next-issue` skill** (`.claude/skills/next-issue/SKILL.md`). It covers:
- picking the lowest open issue that is unclaimed and unblocked
- what to read first
- working in a `../TheCapitol-<n>` worktree on branch `issue-<n>`
- verification
- committing with `Closes #<n>`
- closing the issue with a hand-off comment

Rules that hold across issues:
- Engine changes go upstream to Flint as a PR on a `feature/*` branch. Chris merges Flint PRs.
- The plan doc stays true. Update it in the same commit when a decision changes it.
- Assume another agent may be working at the same time, on this machine or another. Claim the issue first, use your own worktree, stage by path, and never touch changes you didn't make. See the next section.
- **Cut a ticket for every limitation you hit.** This covers anything you work round instead of fix: an engine gap, a missing hook, a tool that can't do something, a backend quirk. Open a GitHub issue for it in the same session, labelled `flint-client` plus `engine-upstream` or `fc:backend-prereq` where they apply. Say what's missing, what the workaround costs, and what removing the workaround involves. Then link the issue from the workaround's code comment and the plan. A limitation recorded only in a closing comment or a vault note gets lost.

## Several Agents, Several Machines

Agents on several machines work this repo at once, all as the same GitHub user. Assignees, labels and comments can't tell them apart, and none of them is atomic. So claims are git branches. `scripts/issue.sh` (bash; on Windows it runs under Git Bash) does the whole cycle, and it applies to any issue, not just `flint-client` ones:

```bash
scripts/issue.sh list                         # who holds what, on every machine, and for how long
scripts/issue.sh claim <n>                    # exit 2: someone else holds it; pick another
git worktree add ../TheCapitol-<n> -b issue-<n> origin/master
# ...work, commit with "Closes #<n>"...
scripts/issue.sh land <n> --verify "<cmd>"    # rebase, re-verify, ff-push to master, release
```

- **A claim is the branch `claim/issue-<n>` on origin.** It holds one commit with no files, and the commit message names the agent and the host. Git rejects the second push of a new branch, so only one claimant can win, even when two push in the same second. The `claimed` label and the "Claimed by" comment are only there for people reading GitHub.
- **Only the checkout that claimed an issue can renew, release or land it.** It keeps a record in `.git/agent-claims/`. Never delete another agent's `claim/*` branch by hand.
- **Claims go stale after 24 hours** (`CLAIM_TTL_HOURS`). Run `renew <n>` at the start of each session on long work. A claim past the TTL can be taken over with `claim <n> --steal`. Before stealing, check the issue's comments and `origin/issue-<n>` for work in progress, and continue from it.
- **Master only moves forward, and only through `land`.** It pushes fast-forward only and retries when another agent lands first. Never force-push master. Exit 3 is a rebase conflict: resolve it, `git rebase --continue`, then land again.
- **Giving up?** Push `issue-<n>`, comment on the issue where the work stands, then `release <n>`.
- **Setting up a new machine:** clone the repo, run `gh auth login` as the shared account, and run `gh auth setup-git` so git pushes use the same credentials. The script needs nothing else.

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
- Backend bound to `127.0.0.1` (`HOST` in the local `backend/.env`; `.env.example` still says `0.0.0.0`). Binding to all interfaces makes Windows Firewall prompt once for every new exe path, and each `TheCapitol-<n>` worktree builds its own exe. Loopback is enough for several clients on this machine. **Testing from another device** (phone, second PC, LAN players) needs `HOST=0.0.0.0` and one firewall prompt. Clients on the other device must also point at this PC's LAN IP, not `localhost`.

## Architecture

### Shared Protocol (`protocol/`)

Crate `capitol-protocol`, a root-workspace member that both the backend and the Flint client depend on. It contains `ws.rs` (`ClientMessage`/`ServerMessage`), `rest.rs` (request and response DTOs), `terrain.rs` (`/terrain` types, which the generator also uses) and `rules.rs` (ranges, speeds, `Zone` and the one table of zone radii, quality grades, the water-crossing test). The backend re-exports these from `models/`. Server-only derives (utoipa, sqlx) sit behind its `backend` feature. Any wire change goes here. `tests/fixtures/` holds one JSON fixture per WS message type, and they must round-trip byte for byte.

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

### Flint Client (`flint-client/`)

The 3D client, built on the Flint engine. It's a separate Cargo workspace (the root workspace excludes it) with the engine as a git subtree at `flint-client/engine/`.

```
crates/capitol-core/    - Networking and state, no engine dependency
crates/capitol-client/  - The game binary (`capitol`), wraps Flint's PlayerApp
scenes/ schemas/ scripts/ fonts/ audio/ models/ fixtures/
play.bat                - Build release and run
```

Build, run, subtree pull/push and the engine-upstream rules are in **`flint-client/CLAUDE.md`**. Read it before working there.

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
3. Weld junctions (`street_plan::weld_junctions`): each street end, and each crossing, becomes a vertex both roads share
4. Generate suburban plots along roads

**Street-plan invariants** (`models/street_plan.rs`, #62): splines are centripetal Catmull-Rom, resampled at even steps. A candidate street is rejected if it hairpins (a turn over 45°), crosses itself, or crosses another road or an arterial anywhere but at its own joining ends. Cul-de-sacs and P-streets are lollipops: the bulb closes on the stem's last vertex. Loop streets end back on their parent, measured along it. A connector with no road to reach isn't placed. `suburban_street_plan_holds_its_invariants` checks it all at four seeds. The grass nodes are seeded in migrations against the default world. `engine::node_placement` (#70) moves any that the live terrain's roads or water run over, on startup and on an admin regenerate, and saves the move. So a generator change no longer needs a migration like `20260215_001_suburban_street_nodes.sql`. `the_default_terrain_moves_no_seeded_node` fails if the default world itself would move one. Nodes standing on lots are left alone (#102).

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
