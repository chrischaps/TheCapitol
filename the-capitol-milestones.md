# The Capitol - Development Milestones

**Document Purpose:** Break down The Capitol into vertical slices for iterative development. Each milestone produces a playable increment that can be tested and evaluated.

**Reference Documents:**
- `the-capitol-gdd.md` - Game Design Document (vision, systems, player experience)
- `the-capitol-tdd.md` - Technical Design Document (architecture, protocol, data models)

**Development Approach:** Vision-driven iteration. Build playable vertical slices, test them, learn, adjust. The GDD/TDD provide the north star; implementation reveals what works.

---

## Technology Stack

| Component | Technology |
|-----------|------------|
| Backend | Rust with Axum |
| Database | PostgreSQL |
| Cache | Redis |
| API | REST + WebSocket (JSON) |
| Client | React (TypeScript) |

---

## Milestone Overview

```
M0: Foundation
 ↓
M1: Living World (movement, presence)
 ↓
M2: Gathering (extraction, resources, inventory)
 ↓
M3: Crafting (recipes, quality cascade)
 ↓
M4: Stations (placed structures, manual crafting)
 ↓
M5: Economy (currency, P2P trade)
 ↓
M6: Markets (auction house)
 ↓
M7: Property (plots, deeds, placement)
 ↓
M8: Automation (offline production)
 ↓
M9: Social (guilds, contracts)
 ↓
M10: Polish & Launch Prep
```

Each milestone builds on the previous. Do not skip ahead.

---

## M0: Foundation

**Goal:** Project scaffolding, basic server running, database connected, client shell exists.

**Duration:** 1 week

### Deliverables

#### Backend
- [x] Rust project initialized with Cargo workspace
- [x] Axum web server running on configurable port
- [x] PostgreSQL connection pool (sqlx)
- [x] Redis connection
- [x] Health check endpoint: `GET /health`
- [x] Basic error handling middleware
- [x] Environment configuration (.env support)
- [x] Logging setup (tracing)

#### Database
- [x] Docker Compose for local PostgreSQL and Redis
- [x] Migration system setup (sqlx-cli or refinery)
- [x] Initial migration: `accounts` and `players` tables (see TDD Section 5.3)

#### Client
- [x] React app initialized (Vite + TypeScript)
- [x] Basic routing setup
- [x] Environment configuration
- [x] Placeholder pages: Login, Game

#### DevOps
- [x] Git repository initialized
- [x] README with setup instructions
- [x] Scripts for local development (`cargo run`, `npm run dev`)

### Acceptance Criteria
- `cargo run` starts the backend server
- `GET /health` returns `{"status": "healthy"}`
- Database migrations run successfully
- React app loads in browser with placeholder content

### Technical Notes
- Use workspace structure: `/backend`, `/client`, `/shared` (if needed)
- Follow TDD Section 1 for architecture principles
- No game logic yet - just infrastructure

---

## M1: Living World

**Goal:** Player can log in, see themselves in the world, move around, see other players.

**Duration:** 2 weeks

### Deliverables

#### Backend - Auth
- [x] `POST /auth/register` - Create account and player
- [x] `POST /auth/login` - Authenticate, return JWT
- [x] `POST /auth/logout` - Invalidate session
- [x] JWT validation middleware
- [x] Password hashing with Argon2id
- [x] Session storage in Redis

#### Backend - Tick Engine (Basic)
- [x] Tick loop running at 100ms intervals
- [x] Tick counter tracking
- [x] Command queue (receive commands, process at tick boundary)
- [x] In-memory game state structure
- [x] Player loading on connect

#### Backend - Movement
- [x] Player position stored (x, y, zone)
- [x] `POST /player/move` - Set destination
- [x] Movement processing in tick loop
- [x] Speed-based position interpolation
- [x] Boundary checking (can't walk off world)

#### Backend - WebSocket
- [x] WebSocket endpoint: `/ws`
- [x] Connection authentication (JWT in handshake)
- [x] Subscription system (subscribe to channels)
- [x] Position broadcast to nearby players
- [x] Heartbeat (ping/pong)

#### Database
- [x] Migration: Full `players` table with position fields
- [x] Player CRUD operations

#### Client - Auth
- [x] Login page with email/password form
- [x] Registration page
- [x] JWT storage (localStorage or memory)
- [x] Auth context/provider
- [x] Redirect to game on successful login

#### Client - Game View
- [x] 2D canvas or SVG world view
- [x] Player position rendered as marker/sprite
- [x] Click-to-move interaction
- [x] WebSocket connection on game load
- [x] Real-time position updates from server
- [x] Other players visible as markers

### Acceptance Criteria
- Can register a new account
- Can log in and see player in world
- Can click to move; player moves smoothly
- Can open second browser; see both players
- Movement syncs between clients in real-time
- Disconnecting removes player from other clients' view

### Technical Notes
- World is a simple 2D plane for now (no zones yet)
- Start with a small test area (e.g., 1000x1000 units)
- Don't worry about world structure (zones, plots) yet
- See TDD Section 3 for tick system design
- See TDD Section 2.4 for WebSocket protocol

---

## M2: Gathering

**Goal:** Player can see resource nodes, extract resources, have an inventory.

**Duration:** 2 weeks

### Deliverables

#### Backend - Resources
- [x] `resource_nodes` table and model
- [x] `resource_types` static data table
- [x] Resource node spawning (seed test area with grass nodes)
- [x] Resource node states (available, being_harvested, depleted)
- [x] Regeneration system (depleted → available after time)

#### Backend - Items & Inventory
- [x] `items` table and model
- [x] `item_types` static data table
- [x] Item location enum (PlayerInventory, World, etc.)
- [x] **Quality system**: each item has quality 0-100
- [x] `GET /player/inventory` - List player's items
- [x] Inventory capacity (weight-based or slot-based, simple for now)

#### Backend - Extraction
- [x] `POST /world/extract` - Begin extraction from resource node
- [x] Player action state (Extracting)
- [x] Extraction processing in tick loop
- [x] Duration based on resource type
- [x] On completion: create item(s), set quality from node + variance
- [x] Node depletion after extraction
- [x] Hand-gathering (no tool required for grass/fiber)

#### Backend - Nearby Query
- [x] `GET /world/nearby` - Return entities near player
- [x] Spatial query for resource nodes in range
- [x] Include node type, position, state

#### Client - Resource Display
- [x] Render resource nodes on world view
- [x] Different appearance by type (grass nodes for now)
- [x] Show node state (available vs depleted)

#### Client - Extraction Interaction
- [x] Click on resource node to begin extraction
- [x] Show extraction progress indicator
- [x] Feedback on completion (item added notification)

#### Client - Inventory UI
- [x] Inventory panel/sidebar
- [x] List items with type, quantity, quality
- [x] Quality displayed as number or simple grade

### Acceptance Criteria
- World shows grass resource nodes scattered around
- Clicking a node starts extraction (player can't move during)
- After extraction completes, fiber appears in inventory
- Fiber has a quality value
- Node becomes depleted, regenerates after ~1 minute
- Inventory shows all gathered items

### Technical Notes
- **Quality baseline:** grass nodes have quality 40-70 (world generation variance)
- **Extraction quality:** `output_quality = node_quality + random(-5, +5)`
- Item stacking: for now, each extraction creates a separate item. Stack aggregation is a view concern (M3 or later)
- See GDD Section 6 for quality cascade design
- See TDD Section 4.2-4.4 for data models

### Test Data
Create static data for:
- ResourceType: `grass` → yields `fiber`
- ItemType: `fiber` (raw material, stackable, weight: 0.1)

---

## M3: Crafting

**Goal:** Player can craft items from gathered materials. Quality cascade working.

**Duration:** 2 weeks

### Deliverables

#### Backend - Recipes
- [x] `recipes` static data table
- [x] Recipe structure: inputs, outputs, duration, quality weights
- [x] Recipe validation (player has required inputs)
- [x] `GET /recipes` - List available recipes (or embed in client)

#### Backend - Crafting Operations
- [x] `POST /world/craft` - Begin crafting operation *(implemented as WS `StartCrafting` command)*
- [x] Crafting operation model (in-memory during operation)
- [x] Player action state (Crafting)
- [x] Crafting processing in tick loop
- [x] Progress tracking and completion detection

#### Backend - Quality Cascade (Core)
- [ ] Quality calculation on craft completion: *(currently a plain average + variance; recipe weights ignored, see `engine/crafting.rs`)*
  ```
  output_quality = Σ(input_quality × input_weight) + random_variance
  ```
- [x] Input items consumed on craft start
- [x] Output item created on craft completion
- [x] Quality clamped to 0-100

#### Backend - Item Stack Aggregation
- [x] Stack view: items of same type in same location grouped
- [x] Stack quality = average of constituent items
- [ ] `POST /items/{id}/split` - Split stack by quantity or quality threshold
- [ ] `POST /items/{id}/merge` - Merge compatible stacks *(merge only happens implicitly via `/inventory/move`)*

#### Client - Crafting UI
- [x] Crafting panel showing available recipes
- [x] Recipe shows: inputs needed, output, player's available materials
- [x] Select recipe → select input items from inventory
- [x] Start craft button
- [x] Progress indicator during crafting
- [x] Result shown on completion

#### Client - Inventory Enhancements
- [x] Items displayed as stacks (grouped by type)
- [x] Stack shows: type, quantity, average quality
- [ ] Click stack to expand/see distribution (optional for M3)
- [ ] Stack splitting UI (optional for M3)

### Acceptance Criteria
- Can craft rope from fiber (2 fiber → 1 rope)
- Rope quality is average of input fiber qualities (±variance)
- Crafting takes time (progress visible)
- Can see rope in inventory after crafting
- Low-quality fiber → low-quality rope (quality cascade works)
- Items stack properly in inventory view

### Technical Notes
- **First recipe:** `rope_basic` (2 fiber → 1 rope)
  - Input weights: fiber1 = 0.5, fiber2 = 0.5
  - Variance: ±3
  - Duration: 5 seconds (50 ticks)
- Hand-crafting only (no station required)
- See GDD Section 6.3 for quality propagation formula
- See TDD Section 4.6 for recipe data model

### Test Data
Create static data for:
- ItemType: `rope` (processed material, stackable)
- Recipe: `rope_basic` (2 fiber → 1 rope)

---

## M4: Stations

**Goal:** Player can place a station, use it for crafting with better results.

**Duration:** 2 weeks

### Deliverables

#### Backend - Stations
- [x] `stations` table and model
- [x] `station_types` static data table
- [x] Station placement (at player position for now, no plots yet)
- [x] Station state (idle, in_use_manual)
- [x] `POST /stations` - Place a new station *(`POST /stations/place`)*
- [ ] `GET /stations/{id}` - Get station details

#### Backend - Station Crafting
- [x] `POST /stations/{id}/use` - Begin using station *(WS open/close station commands)*
- [x] `POST /stations/{id}/release` - Stop using station *(WS open/close station commands)*
- [x] Station contributes to quality calculation *(E1.1 #153 added the station term, E1.3 #155 fed it the station's quality)*
  ```
  output_quality = Σ(input_quality × input_weight) 
                 + (station_quality × station_weight)
                 + random_variance
  ```
- [x] Stations have quality (from their own crafting) *(the kit's quality, `stations.kit_quality` since #88; on the wire as `StationInfo.quality`, E1.3 #155)*
- [x] Recipes can require specific station types

#### Backend - Tool Installation
- [x] Some stations have tool slots *(E1.6 #158: every crafting station has one; storage has none)*
- [x] `POST /stations/{id}/install-tool` - Put tool in station *(E1.6 #158: WS `install_tool`, not REST)*
- [x] `POST /stations/{id}/remove-tool` - Remove tool from station *(E1.6 #158: WS `remove_tool`, not REST)*
- [x] Installed tool contributes to quality *(E1.6 #158: it supplies the recipe's tool term at that station, and wears with each craft)*

#### Backend - Hand Tools
- [x] Tools as items with durability *(E1.4 #156)*
- [x] Tool degradation on use (per action) *(E1.5 #157: each gather or craft wears the tool by 1)*
- [x] Tool quality affects extraction/crafting *(E1.5 #157: power sets yield and speed, retention scales quality, and the tool term in the cascade)*
- [x] Degradation curve: stable until 20%, then degrades *(E1.4 #156, `rules::tools::retention`)*
- [x] `ToolType`: Axe, Pickaxe, Sickle, Hammer, etc. *(E1.4 #156)*

#### Client - Station Display
- [x] Stations rendered in world view
- [x] Station interaction (click to open)
- [x] Station panel: type, quality, installed tool, status

#### Client - Station Crafting UI
- [x] When using station: show station-enabled recipes
- [x] Quality preview (estimate output quality) *(the exact range, E1.1 #153, at the station's own quality since E1.3 #155)*
- [ ] Tool installation interface

### Acceptance Criteria
- Can craft a basic workbench (new recipe)
- Can place workbench in world
- Can use workbench for crafting rope
- Rope crafted at workbench has better quality (station bonus)
- [x] Can craft a basic sickle (tool) *(`stone_sickle`, E1.4 #156)*
- [x] Sickle makes fiber extraction faster/better quality *(E1.5 #157: faster and half again the fiber; quality stays capped at the node's, GDD §6.2)*
- [x] Sickle degrades with use *(E1.5 #157)*
- Tools can be installed in appropriate stations

### Technical Notes
- **Workbench recipe:** 10 fiber + 5 rope → basic_workbench (Q depends on inputs)
- **Sickle recipe:** 5 fiber + 2 rope → basic_sickle (requires workbench)
- Station quality weight ~0.2 (see GDD Section 6.3)
- Tool durability: basic_sickle has 100 uses
- See TDD Section 4.3 for Station data model
- For now, stations float in world (plot placement comes in M7)

### Test Data
Create static data for:
- StationType: `basic_workbench`
- ItemType: `basic_workbench` (furniture)
- ItemType: `basic_sickle` (tool, sickle type)
- Recipe: `basic_workbench` (10 fiber + 5 rope)
- Recipe: `basic_sickle` (requires workbench)
- Update `rope_basic` to optionally use workbench (quality bonus)

---

## M5: Economy

**Goal:** Currency exists, players can trade items and currency with each other.

**Duration:** 2 weeks

### Deliverables

#### Backend - Currency
- [x] `strand_balance` field on player (already in schema)
- [x] Currency operations: add, subtract, transfer
- [x] `currency_transactions` audit log
- [x] `GET /player/currency` - Get balance
- [x] Starting balance for new players (e.g., 100 Strands for testing)

#### Backend - Fiber/Strand Exchange
- [x] Fiber is the backing commodity
- [x] `POST /exchange/deposit` - Convert fiber items to Strands
- [x] `POST /exchange/withdraw` - Convert Strands to fiber items
- [x] 1 fiber = 1 Strand (1:1 backing)
- [x] Exchange fee (0.5%) deducted

#### Backend - P2P Trading
- [x] Trade session model
- [x] `POST /trade/initiate` - Start trade with nearby player
- [x] `POST /trade/{id}/offer` - Add item or currency to trade window
- [x] `POST /trade/{id}/accept` - Accept current offer
- [x] `POST /trade/{id}/cancel` - Cancel trade
- [x] Both parties must accept for trade to execute
- [x] Atomic transfer on execution

#### Client - Currency Display
- [x] Strand balance shown in UI (header or inventory)
- [x] Exchange interface (deposit/withdraw fiber)

#### Client - Trading UI
- [x] Trade request notification
- [x] Trade window showing both parties' offers
- [x] Add items from inventory to offer
- [x] Add currency to offer
- [x] Accept/cancel buttons
- [x] Trade completion feedback

### Acceptance Criteria
- New players start with some Strands
- Can deposit fiber to get Strands
- Can withdraw Strands to get fiber (minus fee)
- Can initiate trade with nearby player
- Can add items and currency to trade
- Both accepting executes the trade atomically
- Items and currency transfer correctly

### Technical Notes
- Trade requires players within interaction range
- Trade window is ephemeral (not persisted, in-memory only)
- See GDD Section 8 for currency design
- See TDD Section 4.5 for economic entities
- Audit log every currency change for debugging

---

## M6: Markets

**Goal:** Auction house for asynchronous buy/sell orders.

**Duration:** 2 weeks

### Deliverables

#### Backend - Order Book
- [ ] `market_orders` table and model
- [ ] Order types: Buy, Sell
- [ ] Order status: Open, PartiallyFilled, Filled, Cancelled, Expired
- [ ] `POST /market/orders` - Create order
- [ ] `DELETE /market/orders/{id}` - Cancel order
- [ ] `GET /market/orders` - List orders (with filters)

#### Backend - Order Matching
- [ ] Match engine runs on tick (1s interval)
- [ ] Price-time priority matching
- [ ] Buy orders: match with lowest sell price ≤ buy price
- [ ] Sell orders: match with highest buy price ≥ sell price
- [ ] Partial fills supported
- [ ] Escrow: sell order locks items, buy order locks currency

#### Backend - Trade Execution
- [ ] `market_trades` history table
- [ ] On match: transfer items, transfer currency
- [ ] Update order quantities
- [ ] Emit events for trade execution

#### Backend - Market Data
- [ ] `GET /market/history` - Price history for item type
- [ ] Best bid/ask cached in Redis
- [ ] WebSocket updates for market changes

#### Client - Market UI
- [ ] Market browser (list item types with activity)
- [ ] Order book view (bids and asks for selected item)
- [ ] Price chart (simple line chart of recent trades)
- [ ] Place order form (buy or sell)
- [ ] My orders list (open orders, cancel button)
- [ ] Order fill notifications

### Acceptance Criteria
- Can place sell order for fiber at price X
- Can place buy order for fiber at price Y
- If X ≤ Y, orders match and trade executes
- Partial fills work (sell 100, buy 50 → 50 remain)
- Can see order book and recent trades
- Can cancel open orders
- Market data updates in real-time (WebSocket)

### Technical Notes
- Market is global (no regional markets for MVP)
- Quality filtering on buy orders: `min_quality` field
- Listing fee: 1% of order value (currency sink)
- See TDD Section 2.3 for market API
- See TDD Section 4.5 for MarketOrder model

---

## M7: Property

**Goal:** World has zones and plots. Players can claim and own plots.

**Duration:** 2-3 weeks

### Deliverables

#### Backend - World Structure
- [x] Zone definitions (Capitol, TradeDistrict, GuildDistrict, Urban, Suburban, Rural, Wilderness)
- [x] Zone boundaries (concentric rings)
- [x] Player position now includes zone
- [x] Zone transitions detected on movement

#### Backend - Plots
- [x] `plots` table and model
- [x] Plot generation for each zone (predefined grid)
- [x] Plot types and sizes by zone
- [x] Ownership (owner_id, deed_id)
- [x] `GET /plots` - List plots (with filters)
- [x] `GET /plots/{id}` - Plot details

#### Backend - Deeds
- [ ] Deed as item type *(claim charges the deed cost in Strands directly; no deed item)*
- [x] `POST /plots/{id}/claim` - Claim unclaimed plot (requires deed purchase)
- [ ] Deed purchase from Capitol bureaucracy (Strand cost by zone)
- [ ] `POST /plots/{id}/transfer` - Transfer ownership

#### Backend - Plot Permissions
- [x] Permission levels (Owner, Guild, Friends, Public)
- [x] `GET /plots/{id}/permissions`
- [x] `PUT /plots/{id}/permissions`
- [ ] Permission checks on entry and interaction *(permissions are stored but never read outside the permissions routes)*

#### Backend - Station Placement on Plots
- [x] Stations must be placed on owned plot
- [x] Station position relative to plot bounds
- [x] Plot capacity limits (station count)
- [x] Remove free-floating station placement from M4

#### Backend - Property Tax
- [x] Weekly property tax calculation
- [x] Tax based on assessed plot value
- [x] Tax collection in hourly tick
- [x] Delinquency tracking (future: penalties)
- [x] Foreclosure at four weeks of unpaid tax, with warnings, and the deed refunded less the debt (#131)

#### Client - World Map
- [ ] Zoomed-out map showing zones
- [x] Zone boundaries visible
- [x] Plot grid visible when zoomed in
- [ ] Plot ownership indicated (colors or markers) *(rectangle ownership overlay was removed in the terrain-plot refactor)*
- [x] Click plot for details *(Alt+click)*

#### Client - Plot Management
- [x] Plot detail panel
- [x] Claim button (if unclaimed)
- [ ] Permissions editor (if owned)
- [x] Build mode for placing stations on owned plot

### Acceptance Criteria
- World has distinct zones with boundaries
- Walking crosses zone boundaries correctly
- Can purchase deed from Capitol
- Can claim unclaimed plot with deed
- Can place station only on owned plot
- Property tax deducted weekly
- Permissions restrict entry/use appropriately

### Technical Notes
- Plot sizes: Urban (small), Suburban (medium), Rural (large)
- Deed prices: see GDD Section 8.4
- Start with simplified zone layout (concentric circles)
- Capitol and TradeDistrict have no player-ownable plots
- See TDD Section 4.4 for Plot model
- Wilderness has resource nodes but no plots (public gathering)

---

## M8: Automation

**Goal:** Stations can run automatically while player is offline (subscribers).

**Duration:** 2-3 weeks

### Deliverables

#### Backend - Subscription System
- [x] `subscription_status` on account (Free, Active, Lapsed), with `subscription_until` (#176)
- [x] Subscription check middleware: the `Subscriber` extractor, `403` with a reason (#176)
- [x] Mock subscription activation (no real payments yet): `POST /admin/accounts/{id}/subscription`, and `POST /player/subscription` behind `MOCK_SUBSCRIPTIONS` (#176)

#### Backend - Containers
- [ ] `containers` table and model
- [ ] Container types (chest, storage rack)
- [ ] Container placement on plots
- [ ] `GET /storage/{id}` - Container contents
- [ ] `POST /storage/{id}/deposit` - Add item
- [ ] `POST /storage/{id}/withdraw` - Remove item

#### Backend - Automation Config
- [ ] Station automation config (recipe, input containers, output containers)
- [ ] `PUT /stations/{id}/automation` - Configure
- [ ] `POST /stations/{id}/start` - Start automation
- [ ] `POST /stations/{id}/stop` - Stop automation

#### Backend - Automated Processing
- [ ] Automation tick processing (1s interval)
- [ ] Check subscription status
- [ ] Pull inputs from connected containers
- [ ] Run recipe (same quality cascade)
- [ ] Push outputs to output containers
- [ ] Handle pause conditions (missing input, full output, broken tool)

#### Backend - Offline Continuity
- [ ] Automation runs regardless of connection (for subscribers)
- [ ] Free players: automation pauses on disconnect
- [ ] State persisted so automation resumes on reconnect

#### Client - Container UI
- [ ] Container interaction (click to open)
- [ ] Container contents display
- [ ] Drag items between inventory and container

#### Client - Automation UI
- [ ] Station automation configuration panel
- [ ] Connect input/output containers
- [ ] Select recipe
- [ ] Start/stop controls
- [ ] Status display (running, paused + reason)
- [ ] Production log/history

### Acceptance Criteria
- Can place containers on plot
- Can configure station with input/output containers
- Starting automation pulls from input, pushes to output
- Automation continues while logged out (subscriber)
- Automation pauses on disconnect (free player)
- Pauses correctly when inputs empty or outputs full
- Reconnecting shows automation results

### Technical Notes
- Automation is the core subscription value proposition
- See TDD Section 3.7 for offline automation design
- See TDD Section 4.3 for AutomationConfig
- Pause reasons: MissingInput, OutputFull, ToolBroken, OwnerOffline
- Tool degradation continues during automation

---

## M9: Social

**Goal:** Guilds and player contracts.

**Duration:** 2-3 weeks

### Deliverables

#### Backend - Guilds
- [ ] `guilds` table and model
- [ ] `guild_memberships` table
- [ ] `POST /guilds` - Create guild (requires subscription)
- [ ] `POST /guilds/{id}/join` - Request to join
- [ ] `POST /guilds/{id}/leave` - Leave guild
- [ ] Guild roles and permissions
- [ ] Guild treasury (shared Strand balance)
- [ ] Guild dues (weekly fee)

#### Backend - Contracts
- [ ] `contracts` table and model
- [ ] Contract types: Sale, Employment, Delivery, Custom
- [ ] `POST /contracts` - Create contract offer
- [ ] `POST /contracts/{id}/accept` - Accept contract
- [ ] `POST /contracts/{id}/fulfill` - Mark complete
- [ ] `POST /contracts/{id}/dispute` - File dispute
- [ ] Escrow for contract value
- [ ] Contract filing fee (sink)

#### Backend - Reputation
- [ ] `reputation_score` on player
- [ ] Reputation changes on contract completion/breach
- [ ] Reputation visible to other players

#### Client - Guild UI
- [ ] Guild browser
- [ ] Guild detail page (members, treasury, settings)
- [ ] Guild creation form
- [ ] Guild management (for leaders)
- [ ] Guild membership in player profile

#### Client - Contract UI
- [ ] Contract creation wizard
- [ ] Incoming contract offers
- [ ] Active contracts list
- [ ] Contract detail with fulfillment tracking
- [ ] Dispute filing

### Acceptance Criteria
- Can create a guild (with subscription)
- Can invite/accept members
- Guild treasury holds shared funds
- Can create employment contract (pay X Strands for Y items)
- Accepting contract creates escrow
- Fulfilling releases escrow to parties
- Reputation increases on successful contracts

### Technical Notes
- Guild creation: 500 Strand fee
- Guild maintenance: 50 + 5/member weekly
- Contract filing: 1% of value
- See GDD Section 8.4 for fee structure
- See TDD Section 4.5 for Contract model
- Dispute resolution is simplified for MVP (manual review flag)

---

## M10: Polish & Launch Prep

**Goal:** Bug fixes, performance, UX polish, launch readiness.

**Duration:** 2-4 weeks

### Deliverables

#### Backend - Performance
- [ ] Profile tick engine, optimize hot paths
- [ ] Database query optimization
- [ ] Connection pool tuning
- [ ] Load testing (target: 1000 concurrent)

#### Backend - Reliability
- [ ] Graceful shutdown (persist state)
- [ ] Startup recovery (load from DB)
- [ ] Error handling audit
- [ ] Rate limiting implementation

#### Backend - Security
- [ ] Input validation audit
- [ ] SQL injection prevention (parameterized queries)
- [ ] Auth token security review
- [ ] Rate limit on auth endpoints

#### Client - UX Polish
- [ ] Loading states throughout
- [ ] Error messages (user-friendly)
- [ ] Responsive layout
- [ ] Keyboard shortcuts
- [ ] Tutorial/onboarding hints

#### Client - Performance
- [ ] Render optimization (large item lists)
- [ ] WebSocket reconnection handling
- [ ] Offline state handling

#### Operations
- [ ] Production deployment setup
- [ ] Environment configuration
- [ ] Database backup strategy
- [ ] Monitoring dashboards
- [ ] Log aggregation
- [ ] Alerting for critical errors

#### Content
- [ ] Expanded item types
- [ ] More recipes
- [ ] Balanced crafting chains
- [ ] Resource distribution tuning

### Acceptance Criteria
- Game runs stable under load
- No critical bugs in core loops
- New player experience is smooth
- Monitoring in place
- Backup/recovery tested
- Ready for closed alpha

---

## Appendix A: Static Data Checklist

Minimum static data needed by milestone:

**M2 - Gathering:**
- ResourceType: grass
- ItemType: fiber

**M3 - Crafting:**
- ItemType: rope
- Recipe: rope_basic

**M4 - Stations:**
- StationType: basic_workbench
- ItemType: basic_workbench, basic_sickle
- Recipe: basic_workbench, basic_sickle

**M5-M6 - Economy:**
- (No new static data, uses existing items)

**M7 - Property:**
- Zone definitions
- Plot templates by zone

**M8 - Automation:**
- ContainerType: basic_chest, storage_rack

**M9 - Social:**
- Contract templates (optional)

**M10 - Polish:**
- Expanded item catalog
- Full recipe tree
- Multiple resource types (wood, ore, etc.)
- Multiple station types
- Multiple tool types

---

## Appendix B: Definition of Done

For each milestone, "done" means:

1. **Backend:** All listed endpoints implemented and tested
2. **Client:** All listed UI components functional
3. **Integration:** Client successfully uses all new backend features
4. **Data:** Required static data in place
5. **Tested:** Manual playthrough of new features
6. **Documented:** API changes noted, README updated if needed
7. **Committed:** Code merged to main branch
8. **Playable:** Can demonstrate the milestone's goal end-to-end

---

## Appendix C: Risk Notes

**M4 (Stations):** First significant complexity in quality cascade. May need iteration on formula/weights.

**M6 (Markets):** Order matching is subtle. Edge cases around partial fills, race conditions.

**M7 (Property):** Largest architectural change (zones, spatial structure). May take longer.

**M8 (Automation):** Core value prop. Must be reliable. Heavy testing needed.

**M9 (Social):** Can be simplified if behind schedule. Guilds more important than contracts.

---

## Appendix D: Estimated Timeline

| Milestone | Duration | Cumulative |
|-----------|----------|------------|
| M0: Foundation | 1 week | Week 1 |
| M1: Living World | 2 weeks | Week 3 |
| M2: Gathering | 2 weeks | Week 5 |
| M3: Crafting | 2 weeks | Week 7 |
| M4: Stations | 2 weeks | Week 9 |
| M5: Economy | 2 weeks | Week 11 |
| M6: Markets | 2 weeks | Week 13 |
| M7: Property | 2-3 weeks | Week 15-16 |
| M8: Automation | 2-3 weeks | Week 17-19 |
| M9: Social | 2-3 weeks | Week 19-22 |
| M10: Polish | 2-4 weeks | Week 21-26 |

**Estimated total: 5-6 months to alpha-ready**

These estimates assume focused development. Buffer for learning, debugging, and iteration.

---

*Document Version: 1.0*
*Last Updated: January 2026*
