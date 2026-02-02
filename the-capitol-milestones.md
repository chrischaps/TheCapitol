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
- [ ] Rust project initialized with Cargo workspace
- [ ] Axum web server running on configurable port
- [ ] PostgreSQL connection pool (sqlx)
- [ ] Redis connection
- [ ] Health check endpoint: `GET /health`
- [ ] Basic error handling middleware
- [ ] Environment configuration (.env support)
- [ ] Logging setup (tracing)

#### Database
- [ ] Docker Compose for local PostgreSQL and Redis
- [ ] Migration system setup (sqlx-cli or refinery)
- [ ] Initial migration: `accounts` and `players` tables (see TDD Section 5.3)

#### Client
- [ ] React app initialized (Vite + TypeScript)
- [ ] Basic routing setup
- [ ] Environment configuration
- [ ] Placeholder pages: Login, Game

#### DevOps
- [ ] Git repository initialized
- [ ] README with setup instructions
- [ ] Scripts for local development (`cargo run`, `npm run dev`)

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
- [ ] `POST /auth/register` - Create account and player
- [ ] `POST /auth/login` - Authenticate, return JWT
- [ ] `POST /auth/logout` - Invalidate session
- [ ] JWT validation middleware
- [ ] Password hashing with Argon2id
- [ ] Session storage in Redis

#### Backend - Tick Engine (Basic)
- [ ] Tick loop running at 100ms intervals
- [ ] Tick counter tracking
- [ ] Command queue (receive commands, process at tick boundary)
- [ ] In-memory game state structure
- [ ] Player loading on connect

#### Backend - Movement
- [ ] Player position stored (x, y, zone)
- [ ] `POST /player/move` - Set destination
- [ ] Movement processing in tick loop
- [ ] Speed-based position interpolation
- [ ] Boundary checking (can't walk off world)

#### Backend - WebSocket
- [ ] WebSocket endpoint: `/ws`
- [ ] Connection authentication (JWT in handshake)
- [ ] Subscription system (subscribe to channels)
- [ ] Position broadcast to nearby players
- [ ] Heartbeat (ping/pong)

#### Database
- [ ] Migration: Full `players` table with position fields
- [ ] Player CRUD operations

#### Client - Auth
- [ ] Login page with email/password form
- [ ] Registration page
- [ ] JWT storage (localStorage or memory)
- [ ] Auth context/provider
- [ ] Redirect to game on successful login

#### Client - Game View
- [ ] 2D canvas or SVG world view
- [ ] Player position rendered as marker/sprite
- [ ] Click-to-move interaction
- [ ] WebSocket connection on game load
- [ ] Real-time position updates from server
- [ ] Other players visible as markers

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
- [ ] `resource_nodes` table and model
- [ ] `resource_types` static data table
- [ ] Resource node spawning (seed test area with grass nodes)
- [ ] Resource node states (available, being_harvested, depleted)
- [ ] Regeneration system (depleted → available after time)

#### Backend - Items & Inventory
- [ ] `items` table and model
- [ ] `item_types` static data table
- [ ] Item location enum (PlayerInventory, World, etc.)
- [ ] **Quality system**: each item has quality 0-100
- [ ] `GET /player/inventory` - List player's items
- [ ] Inventory capacity (weight-based or slot-based, simple for now)

#### Backend - Extraction
- [ ] `POST /world/extract` - Begin extraction from resource node
- [ ] Player action state (Extracting)
- [ ] Extraction processing in tick loop
- [ ] Duration based on resource type
- [ ] On completion: create item(s), set quality from node + variance
- [ ] Node depletion after extraction
- [ ] Hand-gathering (no tool required for grass/fiber)

#### Backend - Nearby Query
- [ ] `GET /world/nearby` - Return entities near player
- [ ] Spatial query for resource nodes in range
- [ ] Include node type, position, state

#### Client - Resource Display
- [ ] Render resource nodes on world view
- [ ] Different appearance by type (grass nodes for now)
- [ ] Show node state (available vs depleted)

#### Client - Extraction Interaction
- [ ] Click on resource node to begin extraction
- [ ] Show extraction progress indicator
- [ ] Feedback on completion (item added notification)

#### Client - Inventory UI
- [ ] Inventory panel/sidebar
- [ ] List items with type, quantity, quality
- [ ] Quality displayed as number or simple grade

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
- [ ] `recipes` static data table
- [ ] Recipe structure: inputs, outputs, duration, quality weights
- [ ] Recipe validation (player has required inputs)
- [ ] `GET /recipes` - List available recipes (or embed in client)

#### Backend - Crafting Operations
- [ ] `POST /world/craft` - Begin crafting operation
- [ ] Crafting operation model (in-memory during operation)
- [ ] Player action state (Crafting)
- [ ] Crafting processing in tick loop
- [ ] Progress tracking and completion detection

#### Backend - Quality Cascade (Core)
- [ ] Quality calculation on craft completion:
  ```
  output_quality = Σ(input_quality × input_weight) + random_variance
  ```
- [ ] Input items consumed on craft start
- [ ] Output item created on craft completion
- [ ] Quality clamped to 0-100

#### Backend - Item Stack Aggregation
- [ ] Stack view: items of same type in same location grouped
- [ ] Stack quality = average of constituent items
- [ ] `POST /items/{id}/split` - Split stack by quantity or quality threshold
- [ ] `POST /items/{id}/merge` - Merge compatible stacks

#### Client - Crafting UI
- [ ] Crafting panel showing available recipes
- [ ] Recipe shows: inputs needed, output, player's available materials
- [ ] Select recipe → select input items from inventory
- [ ] Start craft button
- [ ] Progress indicator during crafting
- [ ] Result shown on completion

#### Client - Inventory Enhancements
- [ ] Items displayed as stacks (grouped by type)
- [ ] Stack shows: type, quantity, average quality
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
- [ ] `stations` table and model
- [ ] `station_types` static data table
- [ ] Station placement (at player position for now, no plots yet)
- [ ] Station state (idle, in_use_manual)
- [ ] `POST /stations` - Place a new station
- [ ] `GET /stations/{id}` - Get station details

#### Backend - Station Crafting
- [ ] `POST /stations/{id}/use` - Begin using station
- [ ] `POST /stations/{id}/release` - Stop using station
- [ ] Station contributes to quality calculation:
  ```
  output_quality = Σ(input_quality × input_weight) 
                 + (station_quality × station_weight)
                 + random_variance
  ```
- [ ] Stations have quality (from their own crafting)
- [ ] Recipes can require specific station types

#### Backend - Tool Installation
- [ ] Some stations have tool slots
- [ ] `POST /stations/{id}/install-tool` - Put tool in station
- [ ] `POST /stations/{id}/remove-tool` - Remove tool from station
- [ ] Installed tool contributes to quality

#### Backend - Hand Tools
- [ ] Tools as items with durability
- [ ] Tool degradation on use (per action)
- [ ] Tool quality affects extraction/crafting
- [ ] Degradation curve: stable until 20%, then degrades
- [ ] `ToolType`: Axe, Pickaxe, Sickle, Hammer, etc.

#### Client - Station Display
- [ ] Stations rendered in world view
- [ ] Station interaction (click to open)
- [ ] Station panel: type, quality, installed tool, status

#### Client - Station Crafting UI
- [ ] When using station: show station-enabled recipes
- [ ] Quality preview (estimate output quality)
- [ ] Tool installation interface

### Acceptance Criteria
- Can craft a basic workbench (new recipe)
- Can place workbench in world
- Can use workbench for crafting rope
- Rope crafted at workbench has better quality (station bonus)
- Can craft a basic sickle (tool)
- Sickle makes fiber extraction faster/better quality
- Sickle degrades with use
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
- [ ] `strand_balance` field on player (already in schema)
- [ ] Currency operations: add, subtract, transfer
- [ ] `currency_transactions` audit log
- [ ] `GET /player/currency` - Get balance
- [ ] Starting balance for new players (e.g., 100 Strands for testing)

#### Backend - Fiber/Strand Exchange
- [ ] Fiber is the backing commodity
- [ ] `POST /exchange/deposit` - Convert fiber items to Strands
- [ ] `POST /exchange/withdraw` - Convert Strands to fiber items
- [ ] 1 fiber = 1 Strand (1:1 backing)
- [ ] Exchange fee (0.5%) deducted

#### Backend - P2P Trading
- [ ] Trade session model
- [ ] `POST /trade/initiate` - Start trade with nearby player
- [ ] `POST /trade/{id}/offer` - Add item or currency to trade window
- [ ] `POST /trade/{id}/accept` - Accept current offer
- [ ] `POST /trade/{id}/cancel` - Cancel trade
- [ ] Both parties must accept for trade to execute
- [ ] Atomic transfer on execution

#### Client - Currency Display
- [ ] Strand balance shown in UI (header or inventory)
- [ ] Exchange interface (deposit/withdraw fiber)

#### Client - Trading UI
- [ ] Trade request notification
- [ ] Trade window showing both parties' offers
- [ ] Add items from inventory to offer
- [ ] Add currency to offer
- [ ] Accept/cancel buttons
- [ ] Trade completion feedback

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
- [ ] Zone definitions (Capitol, TradeDistrict, GuildDistrict, Urban, Suburban, Rural, Wilderness)
- [ ] Zone boundaries (concentric rings)
- [ ] Player position now includes zone
- [ ] Zone transitions detected on movement

#### Backend - Plots
- [ ] `plots` table and model
- [ ] Plot generation for each zone (predefined grid)
- [ ] Plot types and sizes by zone
- [ ] Ownership (owner_id, deed_id)
- [ ] `GET /plots` - List plots (with filters)
- [ ] `GET /plots/{id}` - Plot details

#### Backend - Deeds
- [ ] Deed as item type
- [ ] `POST /plots/{id}/claim` - Claim unclaimed plot (requires deed purchase)
- [ ] Deed purchase from Capitol bureaucracy (Strand cost by zone)
- [ ] `POST /plots/{id}/transfer` - Transfer ownership

#### Backend - Plot Permissions
- [ ] Permission levels (Owner, Guild, Friends, Public)
- [ ] `GET /plots/{id}/permissions`
- [ ] `PUT /plots/{id}/permissions`
- [ ] Permission checks on entry and interaction

#### Backend - Station Placement on Plots
- [ ] Stations must be placed on owned plot
- [ ] Station position relative to plot bounds
- [ ] Plot capacity limits (station count)
- [ ] Remove free-floating station placement from M4

#### Backend - Property Tax
- [ ] Weekly property tax calculation
- [ ] Tax based on assessed plot value
- [ ] Tax collection in hourly tick
- [ ] Delinquency tracking (future: penalties)

#### Client - World Map
- [ ] Zoomed-out map showing zones
- [ ] Zone boundaries visible
- [ ] Plot grid visible when zoomed in
- [ ] Plot ownership indicated (colors or markers)
- [ ] Click plot for details

#### Client - Plot Management
- [ ] Plot detail panel
- [ ] Claim button (if unclaimed)
- [ ] Permissions editor (if owned)
- [ ] Build mode for placing stations on owned plot

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
- [ ] `subscription_status` on account (Free, Active, Lapsed)
- [ ] Subscription check middleware
- [ ] Mock subscription activation (no real payments yet)

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
