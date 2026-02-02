# The Capitol - Technical Design Document

**Working Title:** The Capitol  
**Document Type:** Technical Design Document  
**Backend Language:** Rust  
**Primary Database:** PostgreSQL  
**Cache Layer:** Redis  
**Protocol:** REST + WebSocket (JSON)

---

## 1. Architecture Overview

### 1.1 System Topology

```
┌─────────────────────────────────────────────────────────────────┐
│                         CLIENTS                                 │
├─────────────┬─────────────┬─────────────┬─────────────┬────────┤
│ React Web   │ Love 2D     │ Mobile      │ CLI         │ Bots   │
│ (primary)   │ (future)    │ (future)    │ (sub only)  │        │
└──────┬──────┴──────┬──────┴──────┬──────┴──────┬──────┴───┬────┘
       │             │             │             │          │
       └─────────────┴─────────────┴─────────────┴──────────┘
                                   │
                            HTTPS / WSS
                                   │
                    ┌──────────────┴──────────────┐
                    │        API GATEWAY          │
                    │  (auth, rate limit, route)  │
                    └──────────────┬──────────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              │                    │                    │
    ┌─────────┴─────────┐ ┌───────┴───────┐ ┌─────────┴─────────┐
    │   REST HANDLERS   │ │ WS HANDLERS   │ │  TICK ENGINE      │
    │  (commands)       │ │ (subscriptions)│ │  (simulation)     │
    └─────────┬─────────┘ └───────┬───────┘ └─────────┬─────────┘
              │                   │                   │
              └───────────────────┴───────────────────┘
                                  │
                    ┌─────────────┴─────────────┐
                    │       GAME STATE          │
                    │  (in-memory + persisted)  │
                    └─────────────┬─────────────┘
                                  │
                    ┌─────────────┼─────────────┐
                    │             │             │
             ┌──────┴──────┐ ┌───┴────┐ ┌──────┴──────┐
             │ PostgreSQL  │ │ Redis  │ │ File Store  │
             │ (persistent)│ │ (cache)│ │ (assets)    │
             └─────────────┘ └────────┘ └─────────────┘
```

### 1.2 Component Responsibilities

**API Gateway**
- TLS termination
- Authentication verification
- Rate limiting (per-account, per-endpoint)
- Request routing to appropriate handler
- Subscription tier enforcement (CLI access check)

**REST Handlers**
- Stateless request/response operations
- Commands that mutate game state (place order, move item, claim deed)
- Queries that read game state (inventory, market prices, leaderboards)
- Validation and authorization

**WebSocket Handlers**
- Persistent connections for real-time updates
- Subscription management (what events does this client care about?)
- Fan-out of game events to interested clients
- Heartbeat and connection health

**Tick Engine**
- Core simulation loop
- Processes automation, regeneration, degradation, movement
- Runs independently of client connections
- Emits events consumed by WebSocket handlers

**Game State**
- Authoritative world state held in memory for fast tick processing
- Periodic persistence to PostgreSQL
- Redis for distributed cache and pub/sub

### 1.3 Design Principles

**1. Server Authoritative**
- All game logic runs on the server
- Clients are views/input devices only
- No client-side simulation or prediction (simplifies architecture; tick rate is slow enough that latency is acceptable)

**2. Protocol Agnostic**
- Core game logic has no knowledge of HTTP, WebSocket, or any transport
- Protocol handlers translate between wire format and game commands/queries
- Enables future protocols (gRPC, custom binary) without core changes

**3. Tick-Driven Simulation**
- Game state advances via discrete ticks, not continuous time
- External inputs (player commands) are queued and processed at tick boundaries
- Ensures deterministic, reproducible behavior

**4. Event Sourcing (Partial)**
- Significant state changes emit events
- Events drive WebSocket notifications
- Event log enables debugging, replay, analytics
- Not full event sourcing (state is mutable, not derived from events)

---

## 2. Protocol Specification

The protocol defines the contract between clients and server. All clients (React, Love, CLI, bots) use the same protocol.

### 2.1 Transport Layer

**REST API**
- Base URL: `https://api.thecapitol.game/v1`
- JSON request/response bodies
- Standard HTTP methods (GET, POST, PUT, DELETE)
- Authentication via Bearer token in Authorization header

**WebSocket**
- URL: `wss://api.thecapitol.game/v1/ws`
- JSON messages
- Authentication via token in initial connection handshake
- Bidirectional: client subscribes, server pushes events

### 2.2 Authentication

```
POST /auth/login
{
  "email": "player@example.com",
  "password": "..."
}
→ {
  "token": "jwt...",
  "expires_at": "2026-02-01T00:00:00Z",
  "account": {
    "id": "acc_123",
    "subscription_status": "active" | "lapsed" | "free",
    "subscription_expires": "2026-03-01T00:00:00Z"
  }
}

POST /auth/refresh
Authorization: Bearer <token>
→ { "token": "new_jwt...", "expires_at": "..." }

POST /auth/logout
Authorization: Bearer <token>
→ { "success": true }
```

**Token Claims:**
```json
{
  "sub": "acc_123",
  "iat": 1706745600,
  "exp": 1706832000,
  "tier": "subscribed",
  "client": "web"
}
```

### 2.3 REST API Structure

#### Conventions

- **Resources** are nouns: `/players`, `/items`, `/plots`, `/orders`
- **Actions** use POST to resource-specific endpoints: `/items/{id}/split`, `/orders/{id}/cancel`
- **Pagination** via `?cursor=xxx&limit=50`
- **Filtering** via query params: `/orders?type=sell&item_type=fiber`
- **Errors** return standard structure:
```json
{
  "error": {
    "code": "INSUFFICIENT_FUNDS",
    "message": "Not enough Strands to complete purchase",
    "details": { "required": 500, "available": 320 }
  }
}
```

#### Core Resources

**Player**
```
GET  /player                     → Current player state
GET  /player/inventory           → Item inventory
GET  /player/currency            → Strand balance
GET  /player/attributes          → Computed attributes
GET  /player/position            → World position
POST /player/move                → Initiate movement
```

**Items**
```
GET  /items/{id}                 → Single item details
POST /items/{id}/split           → Split stack by quality or quantity
POST /items/{id}/merge           → Merge compatible stacks
POST /items/{id}/drop            → Drop item at current location
POST /items/{id}/transfer        → Give to another player
```

**Inventory & Storage**
```
GET  /inventory                  → Player inventory
GET  /storage/{container_id}     → Container contents (if accessible)
POST /storage/{container_id}/deposit   → Move item to container
POST /storage/{container_id}/withdraw  → Move item from container
```

**World Interaction**
```
GET  /world/nearby               → Entities near player
GET  /world/plot/{plot_id}       → Plot details
POST /world/interact             → Interact with world entity
POST /world/extract              → Begin extraction from resource node
POST /world/craft                → Begin crafting operation
```

**Stations**
```
GET  /stations/{id}              → Station details
POST /stations/{id}/use          → Begin using a station
POST /stations/{id}/release      → Stop using a station
POST /stations/{id}/install-tool → Install a tool into station slot
POST /stations/{id}/remove-tool  → Remove installed tool
PUT  /stations/{id}/automation   → Configure automation settings
POST /stations/{id}/start        → Start automated operation
POST /stations/{id}/stop         → Stop automated operation
```

**Market**
```
GET  /market/orders              → List orders (with filters)
GET  /market/orders/{id}         → Single order details
POST /market/orders              → Create buy/sell order
DELETE /market/orders/{id}       → Cancel order
GET  /market/history             → Price history for item type
POST /exchange/deposit           → Fiber → Strands
POST /exchange/withdraw          → Strands → Fiber
```

**Plots & Deeds**
```
GET  /plots                      → List available plots (with filters)
GET  /plots/{id}                 → Plot details
POST /plots/{id}/claim           → Claim unclaimed plot
POST /plots/{id}/transfer        → Transfer deed to player
GET  /plots/{id}/permissions     → Access permissions
PUT  /plots/{id}/permissions     → Update permissions
```

**Guilds**
```
GET  /guilds                     → List guilds
GET  /guilds/{id}                → Guild details
POST /guilds                     → Create guild (requires subscription)
POST /guilds/{id}/join           → Request to join
POST /guilds/{id}/leave          → Leave guild
PUT  /guilds/{id}/members/{player_id}  → Update member role
```

**Contracts**
```
GET  /contracts                  → Player's contracts
GET  /contracts/{id}             → Contract details
POST /contracts                  → Create contract offer
POST /contracts/{id}/accept      → Accept contract
POST /contracts/{id}/fulfill     → Mark deliverable complete
POST /contracts/{id}/dispute     → File dispute
```

**Bureaucracy**
```
GET  /bureaucracy/fees           → Current fee schedule
POST /bureaucracy/certify        → Request quality certification
GET  /bureaucracy/licenses       → Player's licenses
POST /bureaucracy/licenses       → Apply for license
```

**Leaderboards**
```
GET  /leaderboards               → List available leaderboards
GET  /leaderboards/{id}          → Leaderboard rankings
GET  /leaderboards/{id}/me       → Player's position in leaderboard
```

### 2.4 WebSocket Protocol

#### Connection Handshake

```
Client connects to wss://api.thecapitol.game/v1/ws

Client sends:
{
  "type": "auth",
  "token": "jwt..."
}

Server responds:
{
  "type": "auth_ok",
  "player_id": "player_123",
  "server_time": "2026-01-31T12:00:00Z",
  "tick_rate": 100
}
```

#### Message Types

**Client → Server:**

```json
// Subscribe to event streams
{
  "type": "subscribe",
  "channels": ["position", "inventory", "nearby", "market:fiber"]
}

// Unsubscribe
{
  "type": "unsubscribe", 
  "channels": ["market:fiber"]
}

// Heartbeat (every 30s)
{
  "type": "ping"
}

// Real-time input (movement, interaction)
{
  "type": "input",
  "action": "move",
  "data": { "direction": "north" }
}
```

**Server → Client:**

```json
// Heartbeat response
{
  "type": "pong",
  "server_time": "2026-01-31T12:00:00Z"
}

// Position update (self and nearby entities)
{
  "type": "position",
  "entities": [
    { "id": "player_123", "x": 1500, "y": 2300, "moving": true, "destination": [1520, 2300] },
    { "id": "player_456", "x": 1480, "y": 2290, "moving": false }
  ]
}

// Inventory change
{
  "type": "inventory",
  "change": "add",
  "item": { "id": "item_789", "type": "fiber", "quantity": 50, "quality": 72 }
}

// Nearby entities changed
{
  "type": "nearby",
  "entered": [{ "id": "node_555", "type": "grass", "x": 1510, "y": 2305 }],
  "exited": [{ "id": "player_456" }]
}

// Market update
{
  "type": "market",
  "item_type": "fiber",
  "best_bid": 95,
  "best_ask": 98,
  "last_trade": 96
}

// Crafting progress
{
  "type": "craft_progress",
  "operation_id": "op_123",
  "progress": 0.75,
  "eta_ticks": 30
}

// Crafting complete
{
  "type": "craft_complete",
  "operation_id": "op_123",
  "result": { "item_id": "item_999", "type": "rope", "quantity": 1, "quality": 68 }
}

// Station status
{
  "type": "station",
  "station_id": "station_123",
  "status": "running",
  "in_use_by": null,
  "blocked_reason": null
}

// Error
{
  "type": "error",
  "code": "INVALID_ACTION",
  "message": "Cannot extract from this node without an axe"
}
```

### 2.5 Command/Query Separation

The protocol separates **commands** (change state) from **queries** (read state).

**Commands** (via REST POST or WebSocket input):
- Validated and queued for next tick
- Response acknowledges receipt, not completion
- Completion notified via WebSocket event
- Idempotency keys prevent duplicate execution

```
POST /world/craft
{
  "recipe": "rope",
  "inputs": ["item_123", "item_124"],
  "tool": "item_500",
  "idempotency_key": "craft_abc123"
}
→ {
  "operation_id": "op_123",
  "status": "queued",
  "estimated_ticks": 45
}

// Later, via WebSocket:
{ "type": "craft_complete", "operation_id": "op_123", ... }
```

**Queries** (via REST GET):
- Return current state immediately
- May be slightly stale (within one tick)
- Cacheable where appropriate

### 2.6 Rate Limits

| Endpoint Category | Limit (per minute) | Notes |
|-------------------|-------------------|-------|
| Auth | 10 | Prevent brute force |
| Market read | 120 | Allow active trading |
| Market write | 30 | Prevent spam orders |
| World interaction | 60 | Bounded by tick rate anyway |
| Inventory | 120 | Frequent UI access |
| WebSocket messages | 60 | Inputs rate-limited |

Bots and CLI clients have same limits - no advantage over UI players.

---

## 3. Tick System

The tick system is the heartbeat of the simulation. All game state changes happen at tick boundaries, ensuring deterministic and consistent behavior.

### 3.1 Tick Architecture

**Core Principle:** Time advances in discrete steps, not continuously. Between ticks, the world is frozen; during a tick, all pending changes are processed atomically.

```
┌─────────────────────────────────────────────────────────────────┐
│                         TICK CYCLE                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐  │
│  │  INPUT   │ -> │ PROCESS  │ -> │  UPDATE  │ -> │  NOTIFY  │  │
│  │  GATHER  │    │  LOGIC   │    │  STATE   │    │  CLIENTS │  │
│  └──────────┘    └──────────┘    └──────────┘    └──────────┘  │
│                                                                 │
│  - Collect       - Movement      - Apply         - Emit events │
│    queued        - Extraction    - Persist       - Fan out to  │
│    commands      - Crafting        changes         WebSockets  │
│  - Snapshot      - Automation    - Update        - Log for     │
│    inputs        - Degradation     caches          analytics   │
│                  - Regeneration                                 │
│                  - Market match                                 │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    [ Wait for next tick ]
```

### 3.2 Tick Rates

Different systems tick at different frequencies. Faster ticks for responsive interactions, slower ticks for background processes.

| System | Tick Interval | Rationale |
|--------|---------------|-----------|
| **Core / Movement** | 100ms (10/sec) | Responsive player movement and interaction |
| **Crafting / Extraction** | 500ms (2/sec) | Progress updates feel smooth |
| **Automation** | 1s (1/sec) | Throughput calculations, machine state |
| **Market Matching** | 1s (1/sec) | Order book processing |
| **Regeneration** | 60s (1/min) | Resource respawn checks |
| **Degradation** | 60s (1/min) | Tool wear, decay calculations |
| **Taxes / Fees** | 3600s (1/hr) | Recurring economic sinks |

#### Implementation: Tick Multiplier

Rather than separate loops, we use a single fast tick (100ms) with multipliers:

```rust
struct TickEngine {
    tick_count: u64,
    // ... other state
}

impl TickEngine {
    fn tick(&mut self) {
        self.tick_count += 1;
        
        // Every tick (100ms)
        self.process_movement();
        self.process_inputs();
        
        // Every 5 ticks (500ms)
        if self.tick_count % 5 == 0 {
            self.process_crafting();
            self.process_extraction();
        }
        
        // Every 10 ticks (1s)
        if self.tick_count % 10 == 0 {
            self.process_automation();
            self.process_market();
        }
        
        // Every 600 ticks (60s)
        if self.tick_count % 600 == 0 {
            self.process_regeneration();
            self.process_degradation();
        }
        
        // Every 36000 ticks (1hr)
        if self.tick_count % 36000 == 0 {
            self.process_taxes();
        }
        
        self.emit_events();
        self.persist_if_needed();
    }
}
```

### 3.3 Processing Order

Within each tick, systems process in a defined order to ensure consistency:

```
1. INPUT GATHERING
   - Drain command queue (player actions received since last tick)
   - Validate commands against current state
   - Mark invalid commands for error response

2. MOVEMENT
   - Update entity positions based on velocity and destination
   - Check collision/boundary constraints
   - Update spatial index (who is near whom)

3. INTERACTIONS
   - Process extraction commands (player → resource node)
   - Process crafting commands (start, progress, complete)
   - Process item transfers (drop, pickup, trade)

4. STATIONS
   - Manual station use (player present, actively crafting)
   - Automated station processing (for enabled stations)
   - Tool degradation for installed tools
   - Input/output container transfers

5. MARKET
   - Process new orders (validate, add to book)
   - Match buy/sell orders
   - Execute trades (transfer items and currency)
   - Cancel expired orders

6. WORLD SIMULATION
   - Resource regeneration (spawn new nodes, restore depleted ones)
   - Tool degradation (wear from use)
   - Buff/debuff expiration

7. ECONOMIC
   - Process pending fee payments
   - Check for overdue taxes
   - Apply penalties if needed

8. STATE FINALIZATION
   - Compute derived state (leaderboards, statistics)
   - Mark dirty entities for persistence
   - Generate events for all state changes

9. NOTIFICATION
   - Dispatch events to WebSocket handlers
   - Fan out to subscribed clients
```

### 3.4 Command Queue

Player commands are queued between ticks and processed atomically at tick boundaries.

```rust
enum GameCommand {
    // Movement
    Move { player_id: PlayerId, destination: Position },
    StopMoving { player_id: PlayerId },
    
    // Interaction
    Extract { player_id: PlayerId, node_id: NodeId, tool_id: ItemId },
    CancelExtract { player_id: PlayerId },
    Craft { player_id: PlayerId, recipe_id: RecipeId, inputs: Vec<ItemId>, tool_id: Option<ItemId> },
    CancelCraft { player_id: PlayerId, operation_id: OperationId },
    
    // Station use
    UseStation { player_id: PlayerId, station_id: StationId },
    ReleaseStation { player_id: PlayerId, station_id: StationId },
    InstallTool { player_id: PlayerId, station_id: StationId, tool_id: ItemId },
    RemoveTool { player_id: PlayerId, station_id: StationId },
    ConfigureAutomation { player_id: PlayerId, station_id: StationId, config: AutomationConfig },
    StartAutomation { player_id: PlayerId, station_id: StationId },
    StopAutomation { player_id: PlayerId, station_id: StationId },
    
    // Items
    PickUp { player_id: PlayerId, item_id: ItemId },
    Drop { player_id: PlayerId, item_id: ItemId, position: Position },
    Transfer { from_player: PlayerId, to_player: PlayerId, item_id: ItemId },
    SplitStack { player_id: PlayerId, item_id: ItemId, split_spec: SplitSpec },
    MergeStacks { player_id: PlayerId, item_ids: Vec<ItemId> },
    
    // Storage
    Deposit { player_id: PlayerId, item_id: ItemId, container_id: ContainerId },
    Withdraw { player_id: PlayerId, item_id: ItemId, container_id: ContainerId },
    
    // Market
    PlaceOrder { player_id: PlayerId, order: OrderSpec },
    CancelOrder { player_id: PlayerId, order_id: OrderId },
    
    // ... etc
}

struct CommandEnvelope {
    id: CommandId,
    command: GameCommand,
    idempotency_key: Option<String>,
    received_at: Timestamp,
}
```

#### Idempotency

Commands with the same `idempotency_key` are only executed once. The server tracks recent keys (TTL ~5 minutes) to deduplicate retries.

### 3.5 Event Emission

State changes generate events that drive client notifications and logging.

```rust
enum GameEvent {
    // Position
    EntityMoved { entity_id: EntityId, from: Position, to: Position },
    EntityEnteredArea { entity_id: EntityId, area_id: AreaId },
    EntityLeftArea { entity_id: EntityId, area_id: AreaId },
    
    // Inventory
    ItemAdded { player_id: PlayerId, item: ItemSnapshot },
    ItemRemoved { player_id: PlayerId, item_id: ItemId },
    ItemUpdated { player_id: PlayerId, item: ItemSnapshot },
    
    // Currency
    CurrencyChanged { player_id: PlayerId, old_balance: u64, new_balance: u64, reason: String },
    
    // Crafting
    CraftingStarted { player_id: PlayerId, operation_id: OperationId, recipe_id: RecipeId },
    CraftingProgress { player_id: PlayerId, operation_id: OperationId, progress: f32 },
    CraftingCompleted { player_id: PlayerId, operation_id: OperationId, output: ItemSnapshot },
    CraftingFailed { player_id: PlayerId, operation_id: OperationId, reason: String },
    
    // Station
    StationUseBegan { station_id: StationId, player_id: PlayerId },
    StationUseEnded { station_id: StationId, player_id: PlayerId },
    StationToolInstalled { station_id: StationId, tool_id: ItemId },
    StationToolRemoved { station_id: StationId, tool_id: ItemId },
    StationAutomationStarted { station_id: StationId },
    StationAutomationPaused { station_id: StationId, reason: String },
    StationProduced { station_id: StationId, output: ItemSnapshot },
    
    // Extraction
    ExtractionStarted { player_id: PlayerId, node_id: NodeId },
    ExtractionCompleted { player_id: PlayerId, node_id: NodeId, yield_item: ItemSnapshot },
    ExtractionFailed { player_id: PlayerId, node_id: NodeId, reason: String },
    
    // Market
    OrderPlaced { order: OrderSnapshot },
    OrderMatched { buy_order_id: OrderId, sell_order_id: OrderId, price: u64, quantity: u32 },
    OrderCancelled { order_id: OrderId },
    
    // World
    ResourceDepleted { node_id: NodeId },
    ResourceRespawned { node_id: NodeId, node: ResourceNodeSnapshot },
    
    // Tool
    ToolDegraded { item_id: ItemId, old_durability: u32, new_durability: u32 },
    ToolBroken { item_id: ItemId },
    
    // ... etc
}
```

### 3.6 Time Representation

**Server Time:** Ticks since server start (monotonic, never decreases)

```rust
struct GameTime {
    tick: u64,
    epoch: Timestamp,
}

impl GameTime {
    fn to_real_time(&self) -> Timestamp {
        self.epoch + Duration::from_millis(self.tick * TICK_INTERVAL_MS)
    }
    
    fn from_real_time(epoch: Timestamp, real_time: Timestamp) -> Self {
        let elapsed = real_time - epoch;
        let tick = elapsed.as_millis() / TICK_INTERVAL_MS;
        GameTime { tick, epoch }
    }
}
```

**Operation Timing:**

Crafting, extraction, and other operations specify duration in ticks:

```rust
struct CraftingOperation {
    id: OperationId,
    player_id: PlayerId,
    recipe_id: RecipeId,
    started_at: u64,
    duration_ticks: u32,
}

impl CraftingOperation {
    fn progress(&self, current_tick: u64) -> f32 {
        let elapsed = current_tick - self.started_at;
        (elapsed as f32 / self.duration_ticks as f32).min(1.0)
    }
    
    fn is_complete(&self, current_tick: u64) -> bool {
        current_tick >= self.started_at + self.duration_ticks as u64
    }
}
```

### 3.7 Offline Automation

For subscribed players, automation continues running while disconnected.

**How it works:**

1. Player configures station and starts automation
2. Player disconnects
3. Tick engine continues processing station (it doesn't know/care about connection state)
4. State changes are persisted normally
5. Player reconnects → receives current state

**No special "offline catch-up"** - automation just runs on the server. The tick engine processes all active stations every automation tick regardless of player connection status.

**Free/lapsed players:**
- Automation requires active client connection
- On disconnect, stations pause
- On reconnect, stations resume from paused state (no catch-up)

```rust
fn process_automation(&mut self) {
    for station in self.stations.iter_mut() {
        if !station.automation_enabled {
            continue;
        }
        
        let owner = self.players.get(&station.owner_id);
        let can_run_offline = owner.subscription_status == SubscriptionStatus::Active;
        let is_connected = self.connections.is_connected(station.owner_id);
        
        if !can_run_offline && !is_connected {
            station.status = StationStatus::Paused { 
                reason: PauseReason::OwnerOffline 
            };
            continue;
        }
        
        // Process station tick...
    }
}
```

### 3.8 Tick Performance Budget

With 100ms tick intervals, each tick must complete in well under 100ms to avoid falling behind.

**Target budget:**
- Total tick processing: < 50ms (50% headroom)
- Movement: < 5ms
- Interactions: < 10ms
- Stations/Automation: < 15ms
- Market: < 10ms
- Event emission: < 5ms
- Persistence: < 5ms (async, non-blocking)

**Scaling strategies (if needed):**
- Spatial partitioning (only process entities in active regions)
- Station batching (spread automation across multiple ticks)
- Market sharding (separate order books per item type)
- Async persistence (write-behind, not write-through)

### 3.9 Determinism

For debugging and potential replay features, tick processing should be deterministic:

- Same inputs at same tick → same outputs
- Random operations use seeded RNG (seed = tick number + entity ID)
- No reliance on wall-clock time during processing
- No reliance on processing order of concurrent data structures

```rust
fn calculate_extraction_quality(&self, node: &ResourceNode, tool: &Item, tick: u64) -> u8 {
    let seed = hash(tick, node.id, tool.id);
    let mut rng = SeededRng::new(seed);
    
    let base_quality = node.base_quality;
    let variance = rng.gen_range(-5..=5);
    let tool_penalty = self.calculate_tool_penalty(tool);
    
    (base_quality as i32 + variance - tool_penalty).clamp(0, 100) as u8
}
```

---

## 4. Data Models

This section defines the core entities and their relationships.

### 4.1 Design Principles

**1. IDs are opaque strings**
- Format: `{type}_{ulid}` (e.g., `player_01HV7ZJXK3B7D8NPQRSTVWXY`)
- ULID provides sortability and uniqueness
- Prefix enables quick type identification in logs/debugging

**2. Quality is per-unit, stacks are views**
- Individual items store their own quality
- "Stacks" are logical groupings with computed aggregates
- Splitting/merging stacks doesn't destroy quality data

**3. Timestamps are ticks + real time**
- Game operations reference tick numbers
- Real-world timestamps for audit/display purposes

**4. Soft deletes where appropriate**
- Deleted items marked, not removed
- Enables audit trail and recovery

### 4.2 Core Entities

#### Account

The billing/auth entity.

```rust
struct Account {
    id: AccountId,                          // acc_xxx
    email: String,
    password_hash: String,
    created_at: Timestamp,
    
    subscription_status: SubscriptionStatus,
    subscription_expires: Option<Timestamp>,
    subscription_tier: SubscriptionTier,
    
    last_login: Option<Timestamp>,
    failed_login_attempts: u32,
    locked_until: Option<Timestamp>,
}

enum SubscriptionStatus {
    Free,
    Active,
    Lapsed,
    Cancelled,
}

enum SubscriptionTier {
    Free,
    Standard,
}
```

#### Player

The in-game character entity.

```rust
struct Player {
    id: PlayerId,                           // player_xxx
    account_id: AccountId,
    name: String,
    created_at: Timestamp,
    
    position: Position,
    destination: Option<Position>,
    speed: f32,
    
    strand_balance: u64,
    
    current_action: Option<PlayerAction>,
    
    attributes_cache: AttributeSet,
    attributes_dirty: bool,
    
    reputation_score: i32,
    
    last_active_at: Timestamp,
    total_playtime_ticks: u64,
}

struct Position {
    x: f64,
    y: f64,
    zone: ZoneId,
}

enum PlayerAction {
    Extracting {
        node_id: NodeId,
        tool_id: ItemId,
        started_tick: u64,
        duration_ticks: u32,
    },
    Crafting {
        operation_id: OperationId,
    },
    UsingStation {
        station_id: StationId,
        operation_id: Option<OperationId>,
    },
    Trading {
        with_player: PlayerId,
        trade_id: TradeId,
    },
}
```

#### Item

The core item entity. Every physical object in the game.

```rust
struct Item {
    id: ItemId,                             // item_xxx
    item_type: ItemTypeId,
    
    quality: u8,
    
    durability: Option<Durability>,
    
    location: ItemLocation,
    
    created_at_tick: u64,
    created_by: Option<PlayerId>,
    
    deleted: bool,
    deleted_at: Option<Timestamp>,
}

struct Durability {
    current: u32,
    maximum: u32,
}

enum ItemLocation {
    PlayerInventory { player_id: PlayerId },
    Container { container_id: ContainerId, slot: Option<u32> },
    World { position: Position },
    MarketEscrow { order_id: OrderId },
    TradeWindow { trade_id: TradeId, from_player: PlayerId },
    Processing { operation_id: OperationId },
    StationToolSlot { station_id: StationId },
    Consumed { reason: String, at_tick: u64 },
}
```

#### ItemStack (Logical View)

Stacks are computed views over items with the same type and location.

```rust
struct ItemStack {
    item_type: ItemTypeId,
    location: ItemLocation,
    
    item_ids: Vec<ItemId>,
    
    total_quantity: u32,
    average_quality: f32,
    min_quality: u8,
    max_quality: u8,
    quality_distribution: QualityDistribution,
}

struct QualityDistribution {
    buckets: [u32; 10],  // 0-9, 10-19, ..., 90-100
}
```

#### ItemType (Definition)

Static data defining item types.

```rust
struct ItemType {
    id: ItemTypeId,
    
    name: String,
    description: String,
    icon: AssetId,
    
    category: ItemCategory,
    material_tier: Option<MaterialTier>,
    
    weight: f32,
    stackable: bool,
    max_stack_size: Option<u32>,
    
    tool_properties: Option<ToolProperties>,
    equipment_properties: Option<EquipmentProperties>,
    consumable_properties: Option<ConsumableProperties>,
    
    base_value: u64,
}

enum ItemCategory {
    RawMaterial,
    ProcessedMaterial,
    Component,
    Tool,
    Equipment,
    Consumable,
    Furniture,
    Deed,
    Currency,
}

enum MaterialTier {
    Primitive,
    Bronze,
    Iron,
    Steel,
}

struct ToolProperties {
    tool_type: ToolType,
    base_power: u32,
    durability_max: u32,
    speed_modifier: f32,
    quality_power_curve: QualityCurve,
    degradation_curve: DegradationCurve,
    
    // Can this tool be installed in a station?
    installable_in: Vec<StationCategory>,
}

enum ToolType {
    Axe,
    Pickaxe,
    Sickle,
    Hammer,
    Needle,
    Saw,
    Chisel,
    Tongs,
}

struct EquipmentProperties {
    slot: EquipmentSlot,
    attribute_modifiers: Vec<AttributeModifier>,
}

struct ConsumableProperties {
    buff_effects: Vec<BuffEffect>,
    duration_ticks: u32,
}
```

### 4.3 Station System

Stations are placed structures that enable crafting. They may be used manually (player present) or automated (runs while offline for subscribers).

#### Station

```rust
struct Station {
    id: StationId,                          // station_xxx
    
    // Location (must be placed)
    plot_id: PlotId,
    position_in_plot: LocalPosition,
    
    // Ownership
    owner_id: PlayerId,
    
    // Type (workbench, forge, loom, etc.)
    station_type: StationTypeId,
    
    // Quality (affects output quality)
    quality: u8,
    
    // State
    status: StationStatus,
    
    // Current operation (manual or automated)
    current_operation: Option<StationOperation>,
    
    // Installed tool (some stations need one)
    installed_tool: Option<ItemId>,
    
    // === AUTOMATION (optional) ===
    automation_enabled: bool,
    automation_config: Option<AutomationConfig>,
    
    // Timestamps
    created_at_tick: u64,
    created_by: PlayerId,
}

enum StationStatus {
    Idle,
    InUseManual { by: PlayerId },
    InUseAutomated,
    Paused { reason: PauseReason },
}

enum PauseReason {
    Manual,
    MissingInput { item_type: ItemTypeId },
    OutputFull,
    ToolBroken,
    ToolMissing,
    OwnerOffline,
}

struct StationOperation {
    recipe_id: RecipeId,
    started_at_tick: u64,
    progress_ticks: u32,
    input_items: Vec<ItemId>,
    input_quality_snapshot: Vec<u8>,
}

struct AutomationConfig {
    recipe_id: RecipeId,
    auto_restart: bool,
    input_connections: Vec<ContainerId>,
    output_connections: Vec<ContainerId>,
}
```

#### StationType (Definition)

Static data defining station types.

```rust
struct StationType {
    id: StationTypeId,                      // e.g., "basic_workbench", "iron_forge"
    name: String,
    description: String,
    
    // Classification
    category: StationCategory,
    material_tier: MaterialTier,
    
    // What it can do
    supported_recipe_categories: Vec<RecipeCategory>,
    
    // Physical
    size: (u32, u32),                       // Tiles in plot
    
    // Requirements
    construction_recipe: RecipeId,
    
    // Station quality contribution to output
    quality_contribution_weight: f32,
    
    // Speed modifier for recipes
    speed_modifier: f32,
    
    // Automation capability
    supports_automation: bool,
    input_connection_slots: u32,
    output_connection_slots: u32,
    
    // Does it need an installed tool?
    requires_installed_tool: Option<ToolType>,
}

enum StationCategory {
    // Crafting stations
    Workbench,          // General assembly, woodworking
    Forge,              // Smelting, metalwork
    Anvil,              // Smithing (needs hammer)
    Loom,               // Textiles
    PotteryWheel,       // Ceramics
    Tannery,            // Leather processing
    Kitchen,            // Food preparation
    Brewery,            // Drinks, potions
    Sawmill,            // Lumber processing
    
    // Storage (bridges to Container)
    StorageRack,
    Chest,
}
```

### 4.4 World Entities

#### Plot

Player-ownable land parcels.

```rust
struct Plot {
    id: PlotId,                             // plot_xxx
    
    zone: Zone,
    bounds: Bounds,
    
    owner_id: Option<PlayerId>,
    deed_id: Option<ItemId>,
    claimed_at: Option<Timestamp>,
    
    plot_type: PlotType,
    size_category: PlotSizeCategory,
    
    permissions: PlotPermissions,
    
    assessed_value: u64,
    last_tax_paid: Timestamp,
    tax_owed: u64,
    
    structure_count: u32,
    station_count: u32,
}

enum Zone {
    Capitol,
    TradeDistrict,
    GuildDistrict,
    Urban,
    Suburban,
    Rural,
    Wilderness,
}

enum PlotType {
    Residential,
    Guild,
    Commercial,
}

enum PlotSizeCategory {
    Small,
    Medium,
    Large,
    GuildHall,
    Stall,
}

struct PlotPermissions {
    entry: PermissionLevel,
    visibility: PermissionLevel,
    station_use: PermissionLevel,
    modify: PermissionLevel,
    
    player_overrides: HashMap<PlayerId, PermissionSet>,
    guild_overrides: HashMap<GuildId, PermissionSet>,
}

enum PermissionLevel {
    Owner,
    Guild,
    Friends,
    Public,
}
```

#### ResourceNode

Harvestable resources in the world.

```rust
struct ResourceNode {
    id: NodeId,                             // node_xxx
    
    position: Position,
    
    resource_type: ResourceTypeId,
    
    base_quality: u8,
    
    state: ResourceNodeState,
    
    depleted_at: Option<u64>,
    regenerates_at: Option<u64>,
}

enum ResourceNodeState {
    Available,
    BeingHarvested { by: PlayerId },
    Depleted,
}

struct ResourceType {
    id: ResourceTypeId,
    
    yields_item: ItemTypeId,
    yield_quantity: RangeInclusive<u32>,
    
    required_tool: Option<ToolType>,
    base_extraction_ticks: u32,
    difficulty: u8,
    
    regen_time_ticks: u32,
    
    spawn_zones: Vec<Zone>,
    spawn_density: f32,
}
```

#### Container

Storage structures.

```rust
struct Container {
    id: ContainerId,                        // container_xxx
    
    location: ContainerLocation,
    
    owner_id: PlayerId,
    
    container_type: ContainerTypeId,
    capacity_slots: u32,
    capacity_weight: f32,
    
    permissions: ContainerPermissions,
    
    automation_role: Option<AutomationRole>,
}

enum ContainerLocation {
    Plot { plot_id: PlotId, position: LocalPosition },
    Station { station_id: StationId, slot_type: StationSlot },
}

enum AutomationRole {
    Input { for_station: StationId },
    Output { for_station: StationId },
    Buffer,
}

enum StationSlot {
    Input,
    Output,
}
```

### 4.5 Economic Entities

#### MarketOrder

```rust
struct MarketOrder {
    id: OrderId,                            // order_xxx
    
    player_id: PlayerId,
    
    order_type: OrderType,
    item_type: ItemTypeId,
    
    quantity_total: u32,
    quantity_filled: u32,
    quantity_remaining: u32,
    
    price_per_unit: u64,
    
    min_quality: Option<u8>,
    
    escrowed_items: Vec<ItemId>,
    
    created_at: Timestamp,
    expires_at: Option<Timestamp>,
    
    status: OrderStatus,
}

enum OrderType {
    Buy,
    Sell,
}

enum OrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired,
}
```

#### Contract

```rust
struct Contract {
    id: ContractId,                         // contract_xxx
    
    party_a: PlayerId,
    party_b: PlayerId,
    
    contract_type: ContractType,
    
    terms: ContractTerms,
    
    status: ContractStatus,
    
    escrow_strands: u64,
    escrow_items: Vec<ItemId>,
    
    created_at: Timestamp,
    accepted_at: Option<Timestamp>,
    deadline: Option<Timestamp>,
    completed_at: Option<Timestamp>,
    
    dispute: Option<ContractDispute>,
}

enum ContractType {
    Sale,
    Employment,
    Delivery,
    Custom,
}

struct ContractTerms {
    party_a_provides: Vec<ContractDeliverable>,
    party_b_provides: Vec<ContractDeliverable>,
    recurring: Option<RecurringSchedule>,
    notes: String,
}

enum ContractDeliverable {
    Currency { amount: u64 },
    Items { item_type: ItemTypeId, quantity: u32, min_quality: Option<u8> },
    Labor { task_description: String },
}

enum ContractStatus {
    Offered,
    Active,
    Completed,
    Disputed,
    Cancelled,
    Breached,
}
```

#### Guild

```rust
struct Guild {
    id: GuildId,                            // guild_xxx
    
    name: String,
    tag: String,
    description: String,
    
    founder_id: PlayerId,
    leader_id: PlayerId,
    
    member_count: u32,
    
    treasury_balance: u64,
    owned_plots: Vec<PlotId>,
    
    created_at: Timestamp,
    last_dues_paid: Timestamp,
    status: GuildStatus,
    
    settings: GuildSettings,
}

struct GuildMembership {
    guild_id: GuildId,
    player_id: PlayerId,
    role: GuildRole,
    joined_at: Timestamp,
    permissions: GuildPermissions,
}

enum GuildRole {
    Leader,
    Officer,
    Member,
    Recruit,
}

struct GuildPermissions {
    can_invite: bool,
    can_kick: bool,
    can_promote: bool,
    can_access_treasury: bool,
    can_manage_plots: bool,
    can_create_contracts: bool,
}

enum GuildStatus {
    Active,
    Suspended { reason: String },
    Disbanded,
}
```

### 4.6 Recipe System

```rust
struct Recipe {
    id: RecipeId,
    
    name: String,
    description: String,
    
    category: RecipeCategory,
    
    crafting_requirements: CraftingRequirements,
    
    knowledge_requirement: RecipeKnowledge,
    
    inputs: Vec<RecipeInput>,
    outputs: Vec<RecipeOutput>,
    
    base_duration_ticks: u32,
    
    quality_weights: QualityWeights,
}

enum RecipeCategory {
    Extraction,
    Processing,
    Assembly,
    Crafting,
}

struct CraftingRequirements {
    // Station requirement (None = hand-craftable)
    station_type: Option<StationTypeId>,
    station_category: Option<StationCategory>,
    
    // Hand tool requirement (must be in inventory/equipped)
    hand_tool: Option<ToolType>,
    
    // Installed tool requirement (tool in station's tool slot)
    installed_tool: Option<ToolType>,
    
    // Minimum station quality
    min_station_quality: Option<u8>,
}

enum RecipeKnowledge {
    Common,
    Purchasable { cost: u64 },
    GuildSecret { guild_type: String },
    Discovered,
}

struct RecipeInput {
    item_type: ItemTypeId,
    quantity: u32,
    min_quality: Option<u8>,
    quality_weight: f32,
    consumed: bool,
}

struct RecipeOutput {
    item_type: ItemTypeId,
    quantity: RangeInclusive<u32>,
}

struct QualityWeights {
    input_weights: Vec<f32>,
    tool_weight: f32,
    station_weight: f32,
    installed_tool_weight: f32,
    minigame_weight: f32,
    variance_range: RangeInclusive<i8>,
}
```

### 4.7 Attribute System

```rust
struct AttributeSet {
    values: HashMap<AttributeId, f32>,
}

struct AttributeDefinition {
    id: AttributeId,
    name: String,
    description: String,
    base_value: f32,
    min_value: f32,
    max_value: f32,
}

struct AttributeModifier {
    attribute_id: AttributeId,
    modifier_type: ModifierType,
    value: f32,
    source: ModifierSource,
    expires_at: Option<u64>,
}

enum ModifierType {
    Flat,
    Percentage,
    Multiplicative,
}

enum ModifierSource {
    Equipment { item_id: ItemId },
    Buff { buff_id: BuffId },
    GuildBonus { guild_id: GuildId },
    AreaEffect { area_id: AreaId },
}
```

### 4.8 Entity Relationship Summary

| Entity | Primary Key | Key Relationships |
|--------|-------------|-------------------|
| Account | acc_xxx | → Players |
| Player | player_xxx | → Account, → Items, ↔ Guild, → Plots |
| Item | item_xxx | → ItemType, → Player/Container/World/Station |
| ItemType | string ID | (static data) |
| Station | station_xxx | → Plot, → Player, ↔ Containers, → installed Item |
| StationType | string ID | (static data) |
| Plot | plot_xxx | → Player (owner), → Zone |
| Container | container_xxx | → Plot/Station, → Player |
| ResourceNode | node_xxx | → ResourceType, → Zone |
| MarketOrder | order_xxx | → Player, → Items (escrow) |
| Contract | contract_xxx | → Player (×2) |
| Guild | guild_xxx | ↔ Players, → Plots |
| Recipe | string ID | (static data) |

---

## 5. Persistence Layer

This section defines how game state maps to PostgreSQL, caching strategies with Redis, and patterns for maintaining consistency.

### 5.1 Persistence Philosophy

**Hybrid State Management:**
- **Hot state** lives in memory for fast tick processing
- **Warm state** lives in Redis for fast reads across server processes
- **Cold state** lives in PostgreSQL as source of truth

```
┌─────────────────────────────────────────────────────────────────┐
│                      TICK ENGINE MEMORY                         │
│  (authoritative during runtime - fastest access)                │
│                                                                 │
│  • Active players (position, action, inventory)                 │
│  • Nearby entities (spatial index)                              │
│  • Active stations                                              │
│  • Pending commands                                             │
│  • Current tick state                                           │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ Write-behind (async)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                           REDIS                                 │
│  (distributed cache - fast access, ephemeral)                   │
│                                                                 │
│  • Session tokens                                               │
│  • Player online status                                         │
│  • Market price cache                                           │
│  • Leaderboard snapshots                                        │
│  • Rate limit counters                                          │
│  • Pub/sub for WebSocket fan-out                                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                        POSTGRESQL                               │
│  (source of truth - durable, queryable)                         │
│                                                                 │
│  • All entity tables                                            │
│  • Transaction history                                          │
│  • Event log                                                    │
│  • Static game data (items, recipes)                            │
└─────────────────────────────────────────────────────────────────┘
```

### 5.2 Write Patterns

**Write-Behind (Primary Pattern):**
- State changes happen in memory first
- Dirty entities queued for persistence
- Background task flushes to PostgreSQL periodically
- Reduces database load, increases throughput

**Write-Through (Critical Operations):**
- Economic transactions (currency transfers, market trades)
- Ownership changes (deed transfers)
- These block until confirmed in PostgreSQL

### 5.3 PostgreSQL Schema

#### Accounts & Players

```sql
CREATE TABLE accounts (
    id              VARCHAR(32) PRIMARY KEY,
    email           VARCHAR(255) UNIQUE NOT NULL,
    password_hash   VARCHAR(255) NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    subscription_status VARCHAR(20) NOT NULL DEFAULT 'free',
    subscription_expires TIMESTAMPTZ,
    subscription_tier   VARCHAR(20) NOT NULL DEFAULT 'free',
    
    last_login      TIMESTAMPTZ,
    failed_login_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until    TIMESTAMPTZ
);

CREATE TABLE players (
    id              VARCHAR(32) PRIMARY KEY,
    account_id      VARCHAR(32) NOT NULL REFERENCES accounts(id),
    name            VARCHAR(50) UNIQUE NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    position_x      DOUBLE PRECISION NOT NULL DEFAULT 0,
    position_y      DOUBLE PRECISION NOT NULL DEFAULT 0,
    position_zone   VARCHAR(20) NOT NULL DEFAULT 'wilderness',
    destination_x   DOUBLE PRECISION,
    destination_y   DOUBLE PRECISION,
    speed           REAL NOT NULL DEFAULT 1.0,
    
    strand_balance  BIGINT NOT NULL DEFAULT 0,
    
    current_action  JSONB,
    attributes_cache JSONB,
    
    reputation_score INTEGER NOT NULL DEFAULT 0,
    
    last_active_at  TIMESTAMPTZ,
    total_playtime_ticks BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX idx_players_account ON players(account_id);
CREATE INDEX idx_players_zone ON players(position_zone);
CREATE INDEX idx_players_position ON players USING GIST (
    point(position_x, position_y)
);
```

#### Items

```sql
CREATE TABLE items (
    id              VARCHAR(32) PRIMARY KEY,
    item_type       VARCHAR(50) NOT NULL,
    
    quality         SMALLINT NOT NULL CHECK (quality >= 0 AND quality <= 100),
    
    durability_current  INTEGER,
    durability_maximum  INTEGER,
    
    location_type   VARCHAR(20) NOT NULL,
    location_player_id    VARCHAR(32) REFERENCES players(id),
    location_container_id VARCHAR(32),
    location_position_x   DOUBLE PRECISION,
    location_position_y   DOUBLE PRECISION,
    location_position_zone VARCHAR(20),
    location_order_id     VARCHAR(32),
    location_trade_id     VARCHAR(32),
    location_operation_id VARCHAR(32),
    location_station_id   VARCHAR(32),
    
    created_at_tick BIGINT NOT NULL,
    created_by      VARCHAR(32) REFERENCES players(id),
    
    deleted         BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at      TIMESTAMPTZ
);

CREATE INDEX idx_items_type ON items(item_type) WHERE NOT deleted;
CREATE INDEX idx_items_player ON items(location_player_id) WHERE NOT deleted;
CREATE INDEX idx_items_container ON items(location_container_id) WHERE NOT deleted;
CREATE INDEX idx_items_station ON items(location_station_id) WHERE NOT deleted;
CREATE INDEX idx_items_location_type ON items(location_type) WHERE NOT deleted;
```

#### Stations

```sql
CREATE TABLE stations (
    id              VARCHAR(32) PRIMARY KEY,
    
    plot_id         VARCHAR(32) NOT NULL REFERENCES plots(id),
    position_x      REAL NOT NULL,
    position_y      REAL NOT NULL,
    
    owner_id        VARCHAR(32) NOT NULL REFERENCES players(id),
    
    station_type    VARCHAR(50) NOT NULL,
    quality         SMALLINT NOT NULL CHECK (quality >= 0 AND quality <= 100),
    
    status          VARCHAR(20) NOT NULL DEFAULT 'idle',
    in_use_by       VARCHAR(32) REFERENCES players(id),
    pause_reason    VARCHAR(50),
    
    current_operation JSONB,
    
    installed_tool_id VARCHAR(32) REFERENCES items(id),
    
    automation_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    automation_config JSONB,
    
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at_tick BIGINT NOT NULL
);

CREATE INDEX idx_stations_plot ON stations(plot_id);
CREATE INDEX idx_stations_owner ON stations(owner_id);
CREATE INDEX idx_stations_status ON stations(status) WHERE status != 'idle';
CREATE INDEX idx_stations_in_use ON stations(in_use_by) WHERE in_use_by IS NOT NULL;
```

#### Plots

```sql
CREATE TABLE plots (
    id              VARCHAR(32) PRIMARY KEY,
    
    zone            VARCHAR(20) NOT NULL,
    bounds_min_x    DOUBLE PRECISION NOT NULL,
    bounds_min_y    DOUBLE PRECISION NOT NULL,
    bounds_max_x    DOUBLE PRECISION NOT NULL,
    bounds_max_y    DOUBLE PRECISION NOT NULL,
    
    owner_id        VARCHAR(32) REFERENCES players(id),
    deed_id         VARCHAR(32) REFERENCES items(id),
    claimed_at      TIMESTAMPTZ,
    
    plot_type       VARCHAR(20) NOT NULL,
    size_category   VARCHAR(20) NOT NULL,
    
    permissions     JSONB NOT NULL DEFAULT '{}',
    
    assessed_value  BIGINT NOT NULL DEFAULT 0,
    last_tax_paid   TIMESTAMPTZ,
    tax_owed        BIGINT NOT NULL DEFAULT 0,
    
    structure_count INTEGER NOT NULL DEFAULT 0,
    station_count   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_plots_owner ON plots(owner_id);
CREATE INDEX idx_plots_zone ON plots(zone);
CREATE INDEX idx_plots_bounds ON plots USING GIST (
    box(point(bounds_min_x, bounds_min_y), point(bounds_max_x, bounds_max_y))
);
```

#### Containers

```sql
CREATE TABLE containers (
    id              VARCHAR(32) PRIMARY KEY,
    
    location_type   VARCHAR(20) NOT NULL,
    location_plot_id    VARCHAR(32) REFERENCES plots(id),
    location_station_id VARCHAR(32) REFERENCES stations(id),
    location_position_x REAL,
    location_position_y REAL,
    
    owner_id        VARCHAR(32) NOT NULL REFERENCES players(id),
    
    container_type  VARCHAR(50) NOT NULL,
    capacity_slots  INTEGER NOT NULL,
    capacity_weight REAL NOT NULL,
    
    permissions     JSONB NOT NULL DEFAULT '{}',
    
    automation_role VARCHAR(20),
    automation_station_id VARCHAR(32) REFERENCES stations(id)
);

CREATE INDEX idx_containers_plot ON containers(location_plot_id);
CREATE INDEX idx_containers_station ON containers(location_station_id);
CREATE INDEX idx_containers_owner ON containers(owner_id);
```

#### Resource Nodes

```sql
CREATE TABLE resource_nodes (
    id              VARCHAR(32) PRIMARY KEY,
    
    position_x      DOUBLE PRECISION NOT NULL,
    position_y      DOUBLE PRECISION NOT NULL,
    position_zone   VARCHAR(20) NOT NULL,
    
    resource_type   VARCHAR(50) NOT NULL,
    
    base_quality    SMALLINT NOT NULL CHECK (base_quality >= 0 AND base_quality <= 100),
    
    state           VARCHAR(20) NOT NULL DEFAULT 'available',
    harvested_by    VARCHAR(32) REFERENCES players(id),
    
    depleted_at_tick    BIGINT,
    regenerates_at_tick BIGINT
);

CREATE INDEX idx_resource_nodes_zone ON resource_nodes(position_zone);
CREATE INDEX idx_resource_nodes_state ON resource_nodes(state);
CREATE INDEX idx_resource_nodes_position ON resource_nodes USING GIST (
    point(position_x, position_y)
);
```

#### Market & Economic

```sql
CREATE TABLE market_orders (
    id              VARCHAR(32) PRIMARY KEY,
    
    player_id       VARCHAR(32) NOT NULL REFERENCES players(id),
    
    order_type      VARCHAR(10) NOT NULL,
    item_type       VARCHAR(50) NOT NULL,
    
    quantity_total  INTEGER NOT NULL,
    quantity_filled INTEGER NOT NULL DEFAULT 0,
    
    price_per_unit  BIGINT NOT NULL,
    
    min_quality     SMALLINT,
    
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ,
    
    status          VARCHAR(20) NOT NULL DEFAULT 'open'
);

CREATE INDEX idx_market_orders_player ON market_orders(player_id);
CREATE INDEX idx_market_orders_matching ON market_orders(item_type, price_per_unit, created_at) 
    WHERE status IN ('open', 'partially_filled');

CREATE TABLE market_trades (
    id              VARCHAR(32) PRIMARY KEY,
    
    buy_order_id    VARCHAR(32) NOT NULL REFERENCES market_orders(id),
    sell_order_id   VARCHAR(32) NOT NULL REFERENCES market_orders(id),
    
    item_type       VARCHAR(50) NOT NULL,
    quantity        INTEGER NOT NULL,
    price_per_unit  BIGINT NOT NULL,
    total_price     BIGINT NOT NULL,
    
    executed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_market_trades_item ON market_trades(item_type, executed_at);

CREATE TABLE contracts (
    id              VARCHAR(32) PRIMARY KEY,
    
    party_a_id      VARCHAR(32) NOT NULL REFERENCES players(id),
    party_b_id      VARCHAR(32) NOT NULL REFERENCES players(id),
    
    contract_type   VARCHAR(20) NOT NULL,
    terms           JSONB NOT NULL,
    status          VARCHAR(20) NOT NULL DEFAULT 'offered',
    
    escrow_strands  BIGINT NOT NULL DEFAULT 0,
    escrow_items    VARCHAR(32)[] NOT NULL DEFAULT '{}',
    
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepted_at     TIMESTAMPTZ,
    deadline        TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    
    dispute         JSONB
);

CREATE INDEX idx_contracts_party_a ON contracts(party_a_id);
CREATE INDEX idx_contracts_party_b ON contracts(party_b_id);
CREATE INDEX idx_contracts_status ON contracts(status) WHERE status IN ('offered', 'active');
```

#### Guilds

```sql
CREATE TABLE guilds (
    id              VARCHAR(32) PRIMARY KEY,
    
    name            VARCHAR(50) UNIQUE NOT NULL,
    tag             VARCHAR(10) UNIQUE NOT NULL,
    description     TEXT,
    
    founder_id      VARCHAR(32) NOT NULL REFERENCES players(id),
    leader_id       VARCHAR(32) NOT NULL REFERENCES players(id),
    
    treasury_balance BIGINT NOT NULL DEFAULT 0,
    
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_dues_paid  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status          VARCHAR(20) NOT NULL DEFAULT 'active',
    
    settings        JSONB NOT NULL DEFAULT '{}'
);

CREATE TABLE guild_memberships (
    guild_id        VARCHAR(32) NOT NULL REFERENCES guilds(id),
    player_id       VARCHAR(32) NOT NULL REFERENCES players(id),
    
    role            VARCHAR(20) NOT NULL DEFAULT 'member',
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    permissions     JSONB NOT NULL DEFAULT '{}',
    
    PRIMARY KEY (guild_id, player_id)
);

CREATE INDEX idx_guild_memberships_player ON guild_memberships(player_id);
```

#### Event Log & Audit

```sql
CREATE TABLE event_log (
    id              BIGSERIAL PRIMARY KEY,
    
    event_type      VARCHAR(50) NOT NULL,
    entity_type     VARCHAR(20),
    entity_id       VARCHAR(32),
    
    payload         JSONB NOT NULL,
    
    tick            BIGINT NOT NULL,
    occurred_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    actor_type      VARCHAR(20),
    actor_id        VARCHAR(32)
);

CREATE INDEX idx_event_log_type ON event_log(event_type, occurred_at);
CREATE INDEX idx_event_log_entity ON event_log(entity_type, entity_id, occurred_at);
CREATE INDEX idx_event_log_tick ON event_log(tick);

CREATE TABLE currency_transactions (
    id              BIGSERIAL PRIMARY KEY,
    
    player_id       VARCHAR(32) NOT NULL REFERENCES players(id),
    
    amount          BIGINT NOT NULL,
    balance_before  BIGINT NOT NULL,
    balance_after   BIGINT NOT NULL,
    
    transaction_type VARCHAR(30) NOT NULL,
    reference_type  VARCHAR(20),
    reference_id    VARCHAR(32),
    
    description     TEXT,
    
    occurred_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    tick            BIGINT NOT NULL
);

CREATE INDEX idx_currency_tx_player ON currency_transactions(player_id, occurred_at);
```

#### Static Game Data

```sql
CREATE TABLE item_types (
    id              VARCHAR(50) PRIMARY KEY,
    
    name            VARCHAR(100) NOT NULL,
    description     TEXT,
    
    category        VARCHAR(30) NOT NULL,
    material_tier   VARCHAR(20),
    
    weight          REAL NOT NULL DEFAULT 1.0,
    stackable       BOOLEAN NOT NULL DEFAULT TRUE,
    max_stack_size  INTEGER,
    
    tool_properties     JSONB,
    equipment_properties JSONB,
    consumable_properties JSONB,
    
    base_value      BIGINT NOT NULL DEFAULT 0,
    
    version         INTEGER NOT NULL DEFAULT 1,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE station_types (
    id              VARCHAR(50) PRIMARY KEY,
    
    name            VARCHAR(100) NOT NULL,
    description     TEXT,
    
    category        VARCHAR(30) NOT NULL,
    material_tier   VARCHAR(20),
    
    supported_recipe_categories VARCHAR(30)[] NOT NULL DEFAULT '{}',
    
    size_x          INTEGER NOT NULL,
    size_y          INTEGER NOT NULL,
    
    construction_recipe VARCHAR(50),
    
    quality_contribution_weight REAL NOT NULL DEFAULT 0.2,
    speed_modifier  REAL NOT NULL DEFAULT 1.0,
    
    supports_automation BOOLEAN NOT NULL DEFAULT FALSE,
    input_connection_slots INTEGER NOT NULL DEFAULT 0,
    output_connection_slots INTEGER NOT NULL DEFAULT 0,
    
    requires_installed_tool VARCHAR(30),
    
    version         INTEGER NOT NULL DEFAULT 1,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE recipes (
    id              VARCHAR(50) PRIMARY KEY,
    
    name            VARCHAR(100) NOT NULL,
    description     TEXT,
    
    category        VARCHAR(30) NOT NULL,
    
    crafting_requirements JSONB NOT NULL,
    knowledge_requirement JSONB NOT NULL DEFAULT '{"type": "common"}',
    
    inputs          JSONB NOT NULL,
    outputs         JSONB NOT NULL,
    
    base_duration_ticks INTEGER NOT NULL,
    
    quality_weights JSONB NOT NULL,
    
    version         INTEGER NOT NULL DEFAULT 1,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE resource_types (
    id              VARCHAR(50) PRIMARY KEY,
    
    yields_item     VARCHAR(50) NOT NULL REFERENCES item_types(id),
    yield_quantity_min INTEGER NOT NULL DEFAULT 1,
    yield_quantity_max INTEGER NOT NULL DEFAULT 1,
    
    required_tool   VARCHAR(30),
    base_extraction_ticks INTEGER NOT NULL,
    difficulty      SMALLINT NOT NULL DEFAULT 50,
    
    regen_time_ticks INTEGER NOT NULL,
    
    spawn_zones     VARCHAR(20)[] NOT NULL DEFAULT '{}',
    spawn_density   REAL NOT NULL DEFAULT 0.1,
    
    version         INTEGER NOT NULL DEFAULT 1,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### 5.4 Redis Schema

#### Session & Auth

```
SET session:{token} {account_id} EX 86400
SADD account_sessions:{account_id} {token}
INCR ratelimit:{account_id}:{endpoint} EX 60
```

#### Real-time State

```
HSET player_online:{player_id} 
    connected_at {timestamp}
    client_type {web|cli|mobile}
    last_heartbeat {timestamp}
EXPIRE player_online:{player_id} 300

SADD online_players {player_id}

GEOADD player_positions {longitude} {latitude} {player_id}
```

#### Market Cache

```
HSET market_snapshot:{item_type}
    best_bid {price}
    best_ask {price}
    last_trade {price}
    last_trade_at {timestamp}
    volume_24h {quantity}

ZADD market_history:{item_type} {timestamp} {price}
```

#### Leaderboards

```
ZADD leaderboard:wealth:global {wealth} {player_id}
ZADD leaderboard:wealth:{zone} {wealth} {player_id}
ZADD leaderboard:wealth:guild:{guild_id} {wealth} {player_id}
ZADD leaderboard:wealth:weekly {wealth} {player_id}
```

#### Pub/Sub Channels

```
PUBLISH player:{player_id} {event_json}
PUBLISH zone:{zone_id} {event_json}
PUBLISH market:{item_type} {event_json}
PUBLISH guild:{guild_id} {event_json}
```

### 5.5 Backup & Recovery

**Continuous:**
- PostgreSQL WAL streaming to replica
- Redis AOF persistence

**Periodic:**
- Full PostgreSQL backup daily
- Game state snapshot every hour

**Recovery:**
- Restore from PostgreSQL backup
- Replay WAL to point-in-time
- Server startup loads from restored DB

---

## 6. Authentication & Accounts

This section covers account management, authentication flows, session handling, and subscription enforcement.

### 6.1 Account Lifecycle

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   SIGNUP    │ --> │    FREE     │ --> │ SUBSCRIBED  │ --> │   LAPSED    │
│             │     │   PLAYER    │     │   PLAYER    │     │   PLAYER    │
└─────────────┘     └─────────────┘     └─────────────┘     └─────────────┘
                           │                   │                   │
                           │                   │                   │
                           └───────────────────┴───────────────────┘
                                               │
                                               ▼
                                    [ Can resubscribe at any time ]
```

**States:**

| Status | Description | Automation | CLI Access | Full Market |
|--------|-------------|------------|------------|-------------|
| Free | Never subscribed | Client-only | No | Limited |
| Active | Current subscription | Server-side | Yes | Yes |
| Lapsed | Subscription expired | Client-only | No | Limited |
| Cancelled | Explicitly cancelled | Client-only | No | Limited |

### 6.2 Authentication Flow

#### Registration

```
POST /auth/register
{
    "email": "player@example.com",
    "password": "...",
    "player_name": "Merchant_Jane"
}

Validation:
- Email: valid format, unique
- Password: min 8 chars, complexity requirements
- Player name: 3-20 chars, alphanumeric + underscore, unique

→ Success:
{
    "account_id": "acc_xxx",
    "player_id": "player_xxx",
    "token": "jwt...",
    "expires_at": "2026-02-01T00:00:00Z"
}

→ Error:
{
    "error": {
        "code": "EMAIL_TAKEN",
        "message": "An account with this email already exists"
    }
}
```

#### Login

```
POST /auth/login
{
    "email": "player@example.com",
    "password": "...",
    "client_type": "web" | "cli" | "mobile"
}

→ Success:
{
    "token": "jwt...",
    "expires_at": "2026-02-01T00:00:00Z",
    "account": {
        "id": "acc_xxx",
        "email": "player@example.com",
        "subscription_status": "active",
        "subscription_expires": "2026-03-01T00:00:00Z"
    },
    "player": {
        "id": "player_xxx",
        "name": "Merchant_Jane"
    }
}

→ Error (wrong password):
{
    "error": {
        "code": "INVALID_CREDENTIALS",
        "message": "Invalid email or password"
    }
}

→ Error (CLI without subscription):
{
    "error": {
        "code": "CLI_REQUIRES_SUBSCRIPTION",
        "message": "CLI access requires an active subscription"
    }
}
```

#### Token Refresh

```
POST /auth/refresh
Authorization: Bearer <current_token>

→ Success:
{
    "token": "new_jwt...",
    "expires_at": "2026-02-02T00:00:00Z"
}
```

#### Logout

```
POST /auth/logout
Authorization: Bearer <token>

→ { "success": true }
```

#### Logout All Sessions

```
POST /auth/logout-all
Authorization: Bearer <token>

→ { "success": true, "sessions_terminated": 3 }
```

### 6.3 JWT Token Structure

```json
{
    "header": {
        "alg": "HS256",
        "typ": "JWT"
    },
    "payload": {
        "sub": "acc_xxx",
        "player_id": "player_xxx",
        "iat": 1706745600,
        "exp": 1706832000,
        "tier": "active",
        "client": "web"
    }
}
```

**Claims:**

| Claim | Description |
|-------|-------------|
| sub | Account ID |
| player_id | Player character ID |
| iat | Issued at (Unix timestamp) |
| exp | Expires at (Unix timestamp) |
| tier | Subscription tier: "free", "active", "lapsed" |
| client | Client type: "web", "mobile", "cli" |

**Token Lifetime:**
- Access token: 24 hours
- Can refresh within 7 days of expiration
- After 7 days, must re-authenticate

### 6.4 Password Security

**Storage:**
- Argon2id hashing (memory-hard)
- Unique salt per password
- Never store plaintext

**Requirements:**
- Minimum 8 characters
- At least one uppercase, one lowercase, one number
- Not in common password list

**Reset Flow:**

```
POST /auth/forgot-password
{ "email": "player@example.com" }
→ { "message": "If an account exists, a reset link has been sent" }

POST /auth/reset-password
{
    "token": "reset_token_from_email",
    "new_password": "..."
}
→ { "success": true }
```

### 6.5 Rate Limiting & Security

**Login Attempts:**
- 5 failed attempts: 5 minute lockout
- 10 failed attempts: 30 minute lockout
- 20 failed attempts: account locked, manual reset required

**Token Validation:**
- Signature verification
- Expiration check
- Subscription status check (for tier-gated features)
- Client type check (CLI requires subscription)

**Session Tracking:**

```rust
struct Session {
    token_hash: String,          // Hash of JWT, not the JWT itself
    account_id: AccountId,
    client_type: ClientType,
    created_at: Timestamp,
    last_active: Timestamp,
    ip_address: IpAddr,
    user_agent: String,
}
```

Sessions stored in Redis with TTL matching token expiration.

### 6.6 Subscription Management

**Subscription Check Points:**

| Action | Check |
|--------|-------|
| CLI login | Must be Active |
| Start automation | Must be Active |
| Automation tick (offline) | Must be Active |
| Guild leadership | Must be Active |
| New plot claim | Must be Active |
| Market listing (beyond limit) | Must be Active |

**Subscription Transitions:**

```rust
// On subscription activation
fn on_subscription_activated(player_id: &PlayerId, state: &mut GameState) {
    // Enable any paused automation
    for station in state.stations_owned_by(player_id) {
        if station.status == StationStatus::Paused { 
            reason: PauseReason::OwnerOffline 
        } {
            station.status = StationStatus::Idle;
            // Automation will resume on next tick if configured
        }
    }
}

// On subscription lapse
fn on_subscription_lapsed(player_id: &PlayerId, state: &mut GameState) {
    // Pause automation (but don't lose configuration)
    for station in state.stations_owned_by(player_id) {
        if station.automation_enabled && station.status == StationStatus::InUseAutomated {
            station.status = StationStatus::Paused { 
                reason: PauseReason::SubscriptionRequired 
            };
        }
    }
    
    // Terminate CLI sessions
    terminate_cli_sessions(player_id);
}
```

**Subscription API:**

```
GET /account/subscription
Authorization: Bearer <token>

→ {
    "status": "active",
    "tier": "standard",
    "started_at": "2025-01-01T00:00:00Z",
    "expires_at": "2026-03-01T00:00:00Z",
    "auto_renew": true
}

POST /account/subscription/cancel
Authorization: Bearer <token>

→ {
    "status": "cancelled",
    "access_until": "2026-03-01T00:00:00Z",
    "message": "Your subscription will remain active until the end of the current billing period"
}
```

### 6.7 Multi-Session Handling

Players can be logged in from multiple clients simultaneously:
- Web client on desktop
- Mobile client on phone
- CLI for automation monitoring

**Constraints:**
- All sessions share the same game state
- Commands from any session are processed
- Events broadcast to all sessions
- Only one "active control" at a time for movement (last input wins)

**Session Awareness:**

```rust
struct PlayerConnections {
    player_id: PlayerId,
    sessions: Vec<ActiveSession>,
}

struct ActiveSession {
    session_id: SessionId,
    client_type: ClientType,
    connected_at: Timestamp,
    websocket: Option<WebSocketHandle>,
    last_input_at: Timestamp,
}
```

### 6.8 Account Data & Privacy

**Data Export:**

```
POST /account/export
Authorization: Bearer <token>

→ {
    "export_id": "export_xxx",
    "status": "processing",
    "estimated_ready": "2026-01-31T13:00:00Z"
}

// Later...
GET /account/export/export_xxx

→ {
    "status": "ready",
    "download_url": "https://...",
    "expires_at": "2026-02-07T00:00:00Z"
}
```

**Account Deletion:**

```
POST /account/delete
Authorization: Bearer <token>
{
    "confirm_password": "...",
    "confirmation": "DELETE MY ACCOUNT"
}

→ {
    "status": "scheduled",
    "deletion_at": "2026-02-14T00:00:00Z",
    "message": "Your account will be permanently deleted in 14 days. Log in to cancel."
}
```

Deletion process:
1. 14-day grace period (can cancel)
2. Player removed from guilds
3. Plots released (or transferred if designated)
4. Market orders cancelled
5. Contracts terminated
6. Account and player data anonymized, then deleted
7. Event log entries retained but anonymized

### 6.9 API Key Authentication (for Bots)

Subscribers can generate API keys for bot access:

```
POST /account/api-keys
Authorization: Bearer <token>
{
    "name": "My Trading Bot",
    "permissions": ["market_read", "market_write", "inventory_read"]
}

→ {
    "key_id": "key_xxx",
    "secret": "sk_live_xxxxxxxxxxxxx",  // Only shown once!
    "name": "My Trading Bot",
    "permissions": ["market_read", "market_write", "inventory_read"],
    "created_at": "2026-01-31T12:00:00Z"
}
```

**API Key Usage:**

```
Authorization: Bearer sk_live_xxxxxxxxxxxxx
```

**Key Management:**

```
GET /account/api-keys
→ [
    {
        "key_id": "key_xxx",
        "name": "My Trading Bot",
        "permissions": [...],
        "created_at": "...",
        "last_used_at": "..."
    }
]

DELETE /account/api-keys/key_xxx
→ { "success": true }
```

**Permissions:**

| Permission | Allows |
|------------|--------|
| inventory_read | View inventory |
| inventory_write | Move/transfer items |
| market_read | View orders, history |
| market_write | Place/cancel orders |
| station_read | View station status |
| station_write | Configure/control stations |
| player_read | View position, attributes |
| player_write | Movement commands |

API keys inherit subscription status from the owning account.

---

## 7. Scalability Considerations

This section outlines strategies for scaling The Capitol as player population grows.

### 7.1 Scaling Dimensions

| Dimension | Challenge | Primary Strategy |
|-----------|-----------|------------------|
| Concurrent connections | WebSocket memory/CPU | Horizontal API servers |
| Tick processing | CPU-bound simulation | Spatial partitioning |
| Database writes | Write throughput | Write-behind batching |
| Database reads | Query load | Redis caching, read replicas |
| Market matching | Order book size | Sharded order books |
| World size | Memory for entities | Region-based loading |

### 7.2 Architecture Evolution

**Phase 1: Single Server (MVP)**

```
┌─────────────────────────────────────────┐
│              SINGLE SERVER              │
│                                         │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  │
│  │   API   │  │  TICK   │  │   WS    │  │
│  │ Handler │  │ Engine  │  │ Handler │  │
│  └─────────┘  └─────────┘  └─────────┘  │
│                    │                    │
│              ┌─────┴─────┐              │
│              │ Game State│              │
│              └───────────┘              │
└─────────────────────────────────────────┘
              │           │
       ┌──────┘           └──────┐
       ▼                         ▼
┌─────────────┐           ┌─────────────┐
│ PostgreSQL  │           │    Redis    │
└─────────────┘           └─────────────┘
```

Target: 1,000 - 5,000 concurrent players

**Phase 2: Separated API Layer**

```
                    ┌─────────────────┐
                    │  Load Balancer  │
                    └────────┬────────┘
                             │
           ┌─────────────────┼─────────────────┐
           │                 │                 │
    ┌──────┴──────┐   ┌──────┴──────┐   ┌──────┴──────┐
    │ API Server  │   │ API Server  │   │ API Server  │
    │     #1      │   │     #2      │   │     #3      │
    └──────┬──────┘   └──────┬──────┘   └──────┬──────┘
           │                 │                 │
           └─────────────────┼─────────────────┘
                             │
                    ┌────────┴────────┐
                    │   TICK SERVER   │
                    │  (single, hot)  │
                    └────────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
       ┌──────┴──────┐ ┌─────┴─────┐ ┌─────┴─────┐
       │ PostgreSQL  │ │   Redis   │ │  Redis    │
       │  Primary    │ │  Primary  │ │  Pub/Sub  │
       └──────┬──────┘ └───────────┘ └───────────┘
              │
       ┌──────┴──────┐
       │ PostgreSQL  │
       │  Replica    │
       └─────────────┘
```

Target: 5,000 - 20,000 concurrent players

**Phase 3: Partitioned Tick Engine**

```
                    ┌─────────────────┐
                    │  Load Balancer  │
                    └────────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              │              │              │
       ┌──────┴──────┐┌─────┴─────┐┌───────┴─────┐
       │ API Cluster ││ API Cluster││ API Cluster │
       │  (Region A) ││ (Region B) ││ (Region C)  │
       └──────┬──────┘└─────┬─────┘└───────┬─────┘
              │             │              │
              └─────────────┼──────────────┘
                            │
         ┌──────────────────┼──────────────────┐
         │                  │                  │
  ┌──────┴──────┐    ┌──────┴──────┐    ┌──────┴──────┐
  │ Tick Engine │    │ Tick Engine │    │ Tick Engine │
  │  (Core +    │    │ (Suburban + │    │ (Wilderness │
  │   Urban)    │    │   Rural)    │    │   Zones)    │
  └──────┬──────┘    └──────┬──────┘    └──────┬──────┘
         │                  │                  │
         └──────────────────┼──────────────────┘
                            │
                   ┌────────┴────────┐
                   │  Coordination   │
                   │    Service      │
                   └────────┬────────┘
                            │
              ┌─────────────┼─────────────┐
              │             │             │
       ┌──────┴──────┐ ┌────┴────┐ ┌──────┴──────┐
       │ PostgreSQL  │ │  Redis  │ │   Redis     │
       │  Cluster    │ │ Cluster │ │   Pub/Sub   │
       └─────────────┘ └─────────┘ └─────────────┘
```

Target: 20,000 - 100,000+ concurrent players

### 7.3 Spatial Partitioning

The world naturally partitions by zone rings. Each tick engine instance owns one or more zones.

**Zone Ownership:**

```rust
enum ZoneAssignment {
    Core,       // Capitol, Trade District, Guild District
    Inner,      // Urban zone
    Middle,     // Suburban zone
    Outer,      // Rural zone
    Wilderness, // Wilderness (may further subdivide by sector)
}

struct TickEngineInstance {
    instance_id: InstanceId,
    zones: Vec<Zone>,
    
    // Entities this instance is authoritative for
    players: HashMap<PlayerId, Player>,
    stations: HashMap<StationId, Station>,
    nodes: HashMap<NodeId, ResourceNode>,
}
```

**Cross-Zone Communication:**

When a player moves between zones owned by different tick engines:

1. Source engine detects player approaching zone boundary
2. Source engine sends player state to coordination service
3. Coordination service routes to destination engine
4. Destination engine accepts player, confirms receipt
5. Source engine removes player from local state
6. Player's WebSocket connection re-routes to new engine

```rust
// Cross-zone handoff message
struct PlayerHandoff {
    player: PlayerSnapshot,
    inventory: Vec<ItemSnapshot>,
    active_buffs: Vec<BuffSnapshot>,
    destination_zone: Zone,
    handoff_position: Position,
}
```

**Market as Global Service:**

The market doesn't partition spatially - it's a global service:

```
┌─────────────────────────────────────────┐
│           MARKET SERVICE                │
│                                         │
│  ┌─────────────────────────────────┐    │
│  │      Order Book (per item)      │    │
│  │  ┌─────┐ ┌─────┐ ┌─────┐        │    │
│  │  │Fiber│ │ Ore │ │Tools│ ...    │    │
│  │  └─────┘ └─────┘ └─────┘        │    │
│  └─────────────────────────────────┘    │
│                                         │
│  Receives: PlaceOrder, CancelOrder      │
│  Emits: OrderMatched, OrderCancelled    │
└─────────────────────────────────────────┘
         │                    ▲
         │ Events             │ Commands
         ▼                    │
┌─────────────────────────────────────────┐
│         Tick Engines (all zones)        │
└─────────────────────────────────────────┘
```

### 7.4 Database Scaling

**Read Scaling:**

- PostgreSQL read replicas for queries
- Redis caching for hot data (leaderboards, market snapshots)
- Cache invalidation via Redis pub/sub

**Write Scaling:**

- Write-behind batching reduces write frequency
- Partition large tables (event_log by time, items by zone)
- Async writes for non-critical data

**Table Partitioning:**

```sql
-- Event log partitioned by month
CREATE TABLE event_log (
    id              BIGSERIAL,
    event_type      VARCHAR(50) NOT NULL,
    ...
    occurred_at     TIMESTAMPTZ NOT NULL
) PARTITION BY RANGE (occurred_at);

CREATE TABLE event_log_2026_01 PARTITION OF event_log
    FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');

CREATE TABLE event_log_2026_02 PARTITION OF event_log
    FOR VALUES FROM ('2026-02-01') TO ('2026-03-01');
```

```sql
-- Items partitioned by zone (for spatial locality)
CREATE TABLE items (
    ...
    location_zone VARCHAR(20)
) PARTITION BY LIST (location_zone);

CREATE TABLE items_core PARTITION OF items
    FOR VALUES IN ('capitol', 'trade_district', 'guild_district');

CREATE TABLE items_inner PARTITION OF items
    FOR VALUES IN ('urban');

-- etc.
```

### 7.5 Connection Scaling

**WebSocket Management:**

Each API server handles WebSocket connections. Target: 10,000 connections per server.

```rust
struct ConnectionManager {
    // Player ID → WebSocket handle
    connections: HashMap<PlayerId, Vec<WebSocketHandle>>,
    
    // Zone → Players in zone (for broadcast efficiency)
    zone_players: HashMap<Zone, HashSet<PlayerId>>,
}
```

**Event Fan-Out:**

For zone-wide events (entity moved, resource spawned):

1. Tick engine emits event to Redis pub/sub
2. API servers subscribed to relevant zone channels
3. Each API server fans out to its connected players in that zone

```rust
// Redis pub/sub channel structure
fn zone_channel(zone: &Zone) -> String {
    format!("events:zone:{}", zone.as_str())
}

fn player_channel(player_id: &PlayerId) -> String {
    format!("events:player:{}", player_id.as_str())
}

fn market_channel(item_type: &ItemTypeId) -> String {
    format!("events:market:{}", item_type.as_str())
}
```

### 7.6 Caching Strategy

**Cache Layers:**

| Data | Cache Location | TTL | Invalidation |
|------|----------------|-----|--------------|
| Session tokens | Redis | 24h | On logout |
| Player online status | Redis | 5min | On heartbeat |
| Market best bid/ask | Redis | 1s | On trade |
| Leaderboards | Redis | 5min | On tick (async rebuild) |
| Item type definitions | Memory | Forever | On deploy |
| Recipe definitions | Memory | Forever | On deploy |
| Player inventory | Memory (tick engine) | Session | On disconnect |

**Cache Warming:**

On server startup:
1. Load all static game data into memory
2. Rebuild leaderboard caches
3. Prime market snapshot caches

### 7.7 Monitoring & Alerts

**Key Metrics:**

| Metric | Warning | Critical |
|--------|---------|----------|
| Tick duration | > 70ms | > 90ms |
| Tick queue depth | > 100 | > 500 |
| DB connection pool usage | > 70% | > 90% |
| Redis memory | > 70% | > 90% |
| WebSocket connections per server | > 8,000 | > 9,500 |
| API response time (p99) | > 200ms | > 500ms |

**Health Checks:**

```
GET /health
→ { "status": "healthy" }

GET /health/detailed
→ {
    "status": "healthy",
    "tick_engine": {
        "current_tick": 12345678,
        "last_tick_duration_ms": 42,
        "queue_depth": 12
    },
    "database": {
        "connected": true,
        "pool_size": 20,
        "pool_available": 15
    },
    "redis": {
        "connected": true,
        "memory_used_mb": 512
    },
    "connections": {
        "websocket_count": 3421,
        "api_requests_per_minute": 12500
    }
}
```

### 7.8 Failure Modes & Recovery

**Tick Engine Failure:**

1. Coordination service detects missing heartbeat
2. Standby tick engine promoted to active
3. Loads state from PostgreSQL
4. Resumes from last persisted tick
5. Players reconnect automatically (client retry logic)

**Database Failure:**

1. Primary fails → promote replica
2. Tick engine switches to read-only mode
3. Commands queued in memory (bounded)
4. Resume processing when DB recovers

**Redis Failure:**

1. Sessions invalidated → players must re-login
2. Leaderboards rebuilt from DB
3. Market cache rebuilt from order book
4. Pub/sub reconnects automatically

### 7.9 Geographic Distribution (Future)

For global player base, deploy regional instances:

- **US-West**: Primary for Americas
- **EU-West**: Primary for Europe/Africa
- **Asia-East**: Primary for Asia-Pacific

Each region runs independent game world (separate economy). Cross-region play not supported (latency constraints).

Alternatively: single global instance with smart routing to minimize latency for read-heavy operations, accept higher latency for writes.

---

## Appendix A: Open Technical Questions

### Infrastructure

*To be decided during deployment planning*

- Cloud provider selection (GCP vs dedicated server)
- CI/CD pipeline setup
- Infrastructure as code (Terraform, Pulumi)
- SSL certificate management
- Domain and DNS setup

### Monitoring & Observability

*To be implemented*

- Metrics collection (Prometheus, InfluxDB)
- Log aggregation (Loki, ELK)
- Distributed tracing (Jaeger, Zipkin)
- Alerting system (PagerDuty, Opsgenie)
- Dashboard creation (Grafana)

### Client Implementation

*To be addressed during client development*

- React client architecture patterns
- State management (Redux, Zustand, or other)
- WebSocket reconnection handling
- Offline state handling
- Asset loading and caching
- Love 2D client architecture (future)

### Testing Strategy

*To be defined*

- Unit testing approach
- Integration testing for tick engine
- Load testing methodology
- Chaos engineering for failure modes

### DevOps

*To be defined*

- Deployment strategy (blue/green, canary)
- Database migration approach
- Feature flags system
- A/B testing infrastructure

---

## Appendix B: Technology Stack Summary

| Component | Technology | Rationale |
|-----------|------------|-----------|
| Backend Language | Rust | Performance, safety, tick consistency |
| Web Framework | Axum | Async, ergonomic, Tokio-native |
| Primary Database | PostgreSQL | Relational integrity, JSONB flexibility |
| Cache/Pub-Sub | Redis | Speed, pub/sub for real-time |
| API Protocol | REST + WebSocket | Familiar, debuggable |
| Serialization | JSON (API), bincode (internal) | Interop vs speed |
| Auth Tokens | JWT (HS256) | Stateless verification |
| Password Hashing | Argon2id | Memory-hard, secure |
| Primary Client | React | Rapid development, ecosystem |
| Future Client | Love 2D (Lua) | 2D game framework, cross-platform |

---

## Appendix C: Reference Documentation

- [Rust Async Book](https://rust-lang.github.io/async-book/)
- [Axum Documentation](https://docs.rs/axum/latest/axum/)
- [SQLx Documentation](https://docs.rs/sqlx/latest/sqlx/)
- [PostgreSQL Documentation](https://www.postgresql.org/docs/)
- [Redis Commands](https://redis.io/commands/)
- [JWT Specification](https://datatracker.ietf.org/doc/html/rfc7519)

---

*Document Version: 1.0 - Initial Complete Draft*  
*Last Updated: January 2026*
