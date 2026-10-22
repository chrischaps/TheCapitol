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
- `the-capitol-economy.md` - Economy v1: quality and tools, content depth, markets (M6), automation (M8), plus CI and auth (issues labelled `economy`, #144 to #179)
- `the-capitol-wilds.md` - The wilds: the Wilderness grown to r3800 to r6000 (the world to 12000; since #308 r4600 to r6800 and 13600), with a height field, rivers, streams and lakes, and nothing claimable (issues labelled `wilds`, #193 to #204)
- `the-capitol-citizens.md` - Citizens: AI players who live in the world, run by `capitol-town`, earning everything, marked honestly, with a pluggable decision cascade from rules up to models (issues labelled `citizens`, #246 to #262, plus #211)
- `the-capitol-controls.md` - Direct control: an over-the-shoulder action camera with WASD and the pad, as a mode switched live against click-to-move (F8) for an A/B, then keep one or both. A `steer` message on the server, one shared walk step (`rules::ground::walk_step`) with a slide, focus and E to act (issues labelled `controls`, #314 to #332)
- `the-capitol-rural.md` - Rural v2: farms on lanes. The rural ring is relaid as farms of about 120k on straight concession roads and winding lanes, with fields between them, even across all 8 sectors, and clear of the plain's water. Since RW1 (#308) it runs r3000 to r4600, so two roads fit across, which makes about 115 farms, and the world is 13600 so the wilds keep their depth (RD10, #307 to #310) (issues labelled `rural`, #282, #284 to #291, #307 to #310, plus #223)
- `the-capitol-devtools.md` - The Observatory: dev tooling and monitoring. An admin console the backend serves at `/admin/console` (samples every 10 s, SSE, the economy, the tick by phase, the town reporting in), live server-side tunables marked on every chart, balance runs that compare two presets, and an F7 dev overlay in the Flint client (issues labelled `devtools`, #351, #353 to #377)
- `the-capitol-commons.md` - The Commons: the city's social half. Local talk and a Capitol channel, letters, delivery commissions with escrow and claims on the hall's board, piece-rate wages, standing in words, weekly leaderboards, and guilds with a treasury, dues and a hall, with citizens in all of it (issues labelled `commons`, #352, #378 to #401, plus #311)
- `the-capitol-plots.md` - Plot sizes: closer to even lot counts across the zones, about 600 urban, 1200 suburban and 400 rural, each lot taking 4 stations, with the targets, the capacity question (#424) and how each part moves the default world (umbrella #420; parts #421 urban, #422 suburban, #423 rural)

## Current Focus: The Observatory (devtools)

Citizens live in the town now, and the 200-citizen soak showed what's missing: every balance finding (#294, #335, #304) was dug out of file reports after the run. Active work is **the Observatory**: one place to watch the world while it runs, change a knob and see the effect on the same chart. The plan is `the-capitol-devtools.md`. The work queue is the GitHub issues labelled `devtools` (#353 to #377), created in dependency order. The phase labels run `dt:D0-groundwork` to `dt:D6-balance-runs`. It has three parts:
- an admin console the backend serves at `/admin/console`, fed by samples every 10 s over SSE;
- live server-side tunables, each change marked on every chart;
- an F7 dev overlay in the Flint client.

The first slice is D0.1 to D0.3, D1.1, D1.2 and D2.1: a live chart of the money and the tick, on the town. The console reads like the Gazette's sibling, with a plain sentence over each chart.

### Citizens (built)

**Citizens** are AI players who live in the world, so the economy is stress-tested and the city feels inhabited with one human on. The plan is `the-capitol-citizens.md`. The work queue is the GitHub issues labelled `citizens` (#246 to #262, plus #211 as C0.5), created in dependency order. The phase labels run `cz:C0-groundwork` to `cz:C3-cascade`. Citizens run in their own process (`capitol-town`, on capitol-core's `Bot`), get no grants, and carry a visible mark; their decisions all go through one `Decider` cascade (plan D4). C3.3 measured Jev (TypeSafe's fast choice model, `TYPESAFE_API_KEY`) as the tier above the rules: `rules → jev@0.9`, at about 1¢ a citizen-day. C3.2 built a Claude layer (`claude-haiku`, `claude-sonnet`, `ANTHROPIC_API_KEY`) and measured it: it adds nothing on menu decisions, and with Sonnet it costs 58¢ a citizen-day. So decisions stay with rules and Jev, and the LLMs are for what citizens say (#311).

### Economy v1 (done)

P0 to P8 of the Flint client are built. **Economy v1** made the economy the game. The plan is `the-capitol-economy.md`. The work queue is the GitHub issues labelled `economy` (#144 to #179): one issue per work package, created in dependency order. The phase labels run `ec:E0-foundations` to `ec:E4-automation`. Client work also carries `flint-client`. New gameplay UI goes in the Flint client only; the React and Love2D clients are frozen and only need to keep compiling. A few issues said they needed Chris's decision first (E0.8, E1.2, E2.1, E3.4); all four are decided.

### The Flint client

The Flint client is a third client built on Chris's Flint engine (Rust/wgpu). It lives in `flint-client/`, with the engine as a git subtree at `flint-client/engine/`. The plan is `the-capitol-flint-client.md`. Its queue, now mostly done, is the GitHub issues labelled `flint-client`. There's one issue per work package, created in dependency order, with phase labels `fc:P0-…` to `fc:P8-…` and `fc:backend-prereq` for the small server changes the client needs.

**When asked to "tackle the next issue", "do the next one" or "work on #N", use the `next-issue` skill** (`.claude/skills/next-issue/SKILL.md`). It covers:
- picking the next issue from **every** open issue, whatever its label: skip the claimed, blocked and `needs-decision` ones, then take bugs that hurt first, then the current plan's work packages, then everything else, lowest number first
- what to read first
- working in a `../TheCapitol-<n>` worktree on branch `issue-<n>`
- verification
- committing with `Closes #<n>`
- closing the issue with a hand-off comment

Rules that hold across issues:
- Engine changes go upstream to Flint as a PR on a `feature/*` branch. Chris merges Flint PRs.
- The plan doc stays true. Update it in the same commit when a decision changes it.
- Assume another agent may be working at the same time, on this machine or another. Claim the issue first, use your own worktree, stage by path, and never touch changes you didn't make. See the next section.
- **Cut a ticket for every limitation you hit.** This covers anything you work round instead of fix: an engine gap, a missing hook, a tool that can't do something, a backend quirk. Open a GitHub issue for it in the same session. Say what's missing, what the workaround costs, and what removing the workaround involves. Then link the issue from the workaround's code comment and the plan. A limitation recorded only in a closing comment or a vault note gets lost.
- **Label every issue you open** with:
  - at least one area: `citizens`, `economy`, `flint-client`, `wilds`, `suburb-v2`, `rural`, `controls`, `devtools` (the Observatory: the admin console, tunables, overlays), `commons` or `infra` (CI, scripts, tooling);
  - its kind: a phase label for a planned work package, `bug` for something broken or a limitation, or `enhancement`;
  - `engine-upstream` or `fc:backend-prereq` where they apply;
  - `needs-decision` when only Chris can settle it.

  The `next-issue` skill sees every open issue, but it ranks by these labels. An issue without them is easy to misjudge.

## Several Agents, Several Machines

Agents on several machines work this repo at once, all as the same GitHub user. Assignees, labels and comments can't tell them apart, and none of them is atomic. So claims are git branches. `scripts/issue.sh` (bash; on Windows it runs under Git Bash) does the whole cycle, and it applies to any issue, not just `flint-client` ones:

```bash
scripts/issue.sh list                         # who holds what, on every machine, and for how long
scripts/issue.sh claim <n>                    # makes ../TheCapitol-<n>; exit 2: someone else holds it, pick another
cd ../TheCapitol-<n>
# ...work, commit with "Closes #<n>"...
scripts/issue.sh land <n> --verify "scripts/ci.sh"    # catch up, smoke test, ff-push to master, release
```

- **A claim is the branch `claim/issue-<n>` on origin.** It holds one commit with no files, and the commit message names the agent and the host. Git rejects the second push of a new branch, so only one claimant can win, even when two push in the same second. The `claimed` label and the "Claimed by" comment are only there for people reading GitHub.
- **Only the session that claimed an issue can renew, release or land it.** `claim` makes the worktree `../TheCapitol-<n>` itself and records it, with the agent id, in `.git/agent-claims/`. `renew`, `release` and `land` run only from that worktree, so a second session on the same checkout can't act on the first one's claim (#134). An existing worktree for an issue you didn't claim belongs to another session: leave it alone. `claim <n> --no-worktree` skips the worktree, for write-up-only issues. Never delete another agent's `claim/*` branch by hand.
- **Claims go stale after 24 hours** (`CLAIM_TTL_HOURS`). Run `renew <n>` at the start of each session on long work. A claim past the TTL can be taken over with `claim <n> --steal`. Before stealing, check the issue's comments and `origin/issue-<n>` for work in progress, and continue from it.
- **Stealing sooner is Chris's call.** When Chris says a holder is gone (a session he closed, a machine that's off), `claim <n> --steal --override "<reason>"` takes a claim under the TTL. Put his reason in it: it's recorded in the claim commit and on the issue (#140). Agents never override on their own judgment, and never by lowering `CLAIM_TTL_HOURS`.
- **Master only moves forward, and only through `land`.** It pushes fast-forward only and retries when another agent lands first. Never force-push master. To catch up it rebases, unless the branch holds a merge or a subtree squash: then it merges master in, so the subtree merge survives. `--verify` runs before the first push and again after any catch-up. Verify with `scripts/ci.sh`, the smoke test: clippy, the protocol's fixtures, and clippy over the Flint client when the branch touches it (#264). Run the issue's own Verify before `land`, not in it, because `--verify` runs again on every catch-up. Exit 3 is a conflict: resolve it, `git rebase --continue` (or `git commit` for a merge), then land again.
- **CI runs nightly and on demand, not on push (#270).** `scripts/ci.sh full` runs what it runs: clippy, the backend tests, the database tests, and the Flint client's clippy and tests when the branch touches it. It's slow, so landing doesn't wait on it. A red nightly opens a `Nightly CI failed` issue (`bug`, `infra`), which `next-issue` takes first. Master may be red for a few hours, and that's fine until the game is live. Before landing something risky (a migration, terrain or generation, a wide refactor), run `ci.sh full` or `gh workflow run ci.yml --ref issue-<n>`.
- **Giving up?** Push `issue-<n>`, comment on the issue where the work stands, then `release <n>`.
- **Setting up a new machine:** clone the repo, run `gh auth login` as the shared account, and run `gh auth setup-git` so git pushes use the same credentials. The script needs nothing else.

## Development Commands

### Rust toolchain

`rust-toolchain.toml` at the root pins the Rust version, with clippy and rustfmt, for every machine, for CI and for both workspaces (#265). rustup installs it on first use. Don't `rustup update` to fix a verify, and don't set `RUSTUP_TOOLCHAIN` (`scripts/ci.sh` warns when it's set). A bump is a commit of its own: change the version, run `scripts/ci.sh clippy test db flint`, and fix what the new clippy flags in that same commit.

### Backend (Rust/Axum)
```bash
# Start PostgreSQL and Redis containers
docker-compose up -d

# Run backend server (from repo root or backend/)
cargo run

# Run with verbose logging
RUST_LOG=debug cargo run

# Run under the profiler: Tracy 0.14 connects on localhost (D3.3)
cargo run --features profile

# Check for compilation errors
cargo check

# Run tests
cargo test

# Run a specific test (terrain generation tests are slow ~2min)
cargo test test_no_roads_terminate -- --nocapture
```

**The profiler** (D3.3, #367) is the backend's `profile` feature: `tracing-tracy` as a layer, off by default, so the usual build has no tracy in it (`cargo tree -i tracy-client-sys` finds nothing). Under it every span `RUST_LOG` lets through is a Tracy zone, `tick.<phase>` and `loop.<name>` among them. Each tick is a frame, and its work the `tick` frame set, so the frame graph shows the work against the 100 ms. It listens on loopback only and keeps nothing until a viewer connects. Open **Tracy 0.14.0** (`tracy-profiler.exe` from [wolfpld/tracy's release](https://github.com/wolfpld/tracy/releases/tag/v0.14.0)) and connect to `127.0.0.1`. The Flint client's `profile` feature is on the same Tracy, so one viewer reads both; they share port 8086, so run one at a time or start the second with `TRACY_PORT`. Headless: `tracy-capture -o tick.tracy -s 20`, then `tracy-csvexport tick.tracy`.

### A town: the world with citizens in it

The usual backend has no citizens. A **town** is a second one with citizens living in it, on a database (`capitol_town`), port (3300) and Redis db of its own, so its citizens never claim a lot or fill a book in the world you work in (#337). Either runs without the other, or both at once.

```bash
scripts/town.sh up      # first time: makes the database, an admin and enough citizens to keep 18 + 2 merchants on (about 640); Ctrl-C stops it
scripts/town.sh status  # the server, and each citizen's last health
scripts/town.sh reset   # drop it all, for a new town
scripts\town.ps1 up     # the same in PowerShell (-Online, -Merchants, -AlwaysOn; status, reset)
flint-client\play.bat --server http://127.0.0.1:3300   # play in the town (register on the form the first time)
flint-client\play.bat                                  # play in the usual world
```

A running town reports to its server each game minute (D4.1, `GET /admin/town`, and each sample's `town`), with a key it mints into `flint-client/town/town.report-key` the first time. Citizens keep human hours (#276): each signs in when its habit says, on the real clock, a few hours some days in its own evenings, so most of the roster is away at any moment. `capitol-town habits --roster flint-client/town/town.toml` shows who's on when, and `up --always-on` keeps the whole roster on. Its roster, keys, admin and the server's log are in `flint-client/town/` (git-ignored). It runs copies of the binaries, so it never locks `cargo run`'s exe. The client keeps one server's sign-in, so a switch between the two asks for your password again ([#338](https://github.com/chrischaps/TheCapitol/issues/338)).

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

Crate `capitol-protocol`, a root-workspace member that both the backend and the Flint client depend on. It contains `ws.rs` (`ClientMessage`/`ServerMessage`), `rest.rs` (request and response DTOs), `terrain.rs` (`/terrain` types, which the generator also uses) and `rules.rs` (ranges, speeds, `Zone` and the one table of zone radii, quality grades, the water-crossing test, and `rules::ground`, moving on the land, W4) and `land.rs` (the wilds' height field, a pure function of `LandParams`, W2, and `carve`, which cuts the wild water's beds into it, W3). The backend re-exports these from `models/`. Server-only derives (utoipa, sqlx) sit behind its `backend` feature. Any wire change goes here. `tests/fixtures/` holds one JSON fixture per WS message type, and they must round-trip byte for byte.

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
  stations.rs    - Station placement and interaction
  regeneration.rs - Resource node regeneration
  trading.rs     - P2P trade sessions and trade manager
  market.rs      - Market escrow: placing and cancelling orders (E3.1)
  bazaar.rs      - The bazaar: stall rent, shops, buying, lapse sweep (E3.10)
  gazette.rs     - The Capitol Gazette: the day's records, the edition, the midnight loop (E3.9)
  currency.rs    - Strand/Fiber exchange operations
  property_tax.rs - Hourly tax processing
  ledger.rs      - The world's books: units made and destroyed, and GET /admin/ledger (C1.7)
  load.rs        - Tick time and sockets as running totals, for the ledger (C1.7)
  sampler.rs     - The Observatory's samples: one every 10 s, a day in memory, one a minute to metrics_samples (D0.2)

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
  bazaar.rs      - The bazaar quarter: its stalls, lane, alley and square (E3.12)
  hydrology.rs   - The wilds' rivers, streams, lakes and fords, from the height field (W3)
  lot_geometry.rs - Lot geometry the suburb and the farms share: road index, rays, regions, water, overlap pass (R2)
  plot.rs        - Plot ownership, permissions, property tax
  suburb/        - The suburb's neighbourhoods, streets, lots and parks (suburb v2)
  rural/         - The rural ring (Rural v2): the plan harness, `backend/tools/rural-plan.ps1` (R1), the sectors, concession roads and winding lanes (R3), the farms clear of the water (`farms.rs`) and the balance pass's spurs (`balance.rs`) (R4)

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
  bazaar.rs      - The bazaar's directory, stalls, listings and buying (E3.10)
  gazette.rs     - The Gazette's editions and your byline (E3.9)
  console.rs     - The Observatory's console page at /admin/console, from backend/static/ (D1.1)

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

The Wilderness is "the wilds" (`the-capitol-wilds.md`): planned as r3800 to r6000 once the world grows to 12000 (#194), with a shared height field (`capitol_protocol::land`), rivers and lakes that block, streams you wade, wild trails, and no plots. Since W1 (#194) the world was 12000 across, and since RW1 (#308, rural plan RD10) it's 13600, centred on (6800, 6800): the rural ring runs r3000 to r4600 and the Wilderness r4600 to r6800, the edge of the world disc (`rules::in_world`). Each move shifted every stored position (+2000, then +800). Since W2 (#195) the land rises there: `land::height_at` is 0 out to r4700 (r3900 before #308), then foothills and eight massifs whose valleys open on the arterials. Since W3 (#196) it has water: `models::hydrology` makes rivers, streams, lakes and fords from the height, on `/terrain`. Water runs on into the rural ring round the roads (#222), steered by a routing field on the flat plain, and soaks away there: each course ends in a pond or peters out, never inside r3150. The rural ring's farms keep clear of it (#223, closed by Rural v2's R4). `land::carve` (with the `land::Water` index) cuts the beds into the ground, the same on the server and the clients. Since W5 (#198) the arterials run on as wild trails (`models::trails`, `wild_*` `trail` roads): least-cost paths over the height and round the water, up each valley, along the high ground between them, and spurs to the lakes, fords and tops, all under `trails::WALK_LIMIT` (0.98). `backend/tools/land-relief.ps1` draws the relief with the water and the trails. Since W4 (#197) you walk on the land: `rules::ground::Ground::step` slows a climb, stops you below a cliff (`CLIFF_SLOPE`) and at a river or lake except at a ford or on a road, and wades streams. The server and Flint's prediction both use it. Since W6 (#199) the wilds grow nodes from the land: veins high on the rock, stone on the scree, old growth on the valley floors and clay on the banks. Each resource's habitat and the quality table that rises into the ranges are in `backend/content/wilds.toml`. `engine::node_spawn::wilds` places them after every other zone, on ground a walker can reach. Since W9 (#202) the Flint client draws the land: `world/land.rs` samples it off the main thread into one `GroundField` that everything stands on (`land::rise`), and `world/land_mesh.rs` draws it in LOD chunks. Since W10 (#203) it draws the water too (`world/wild_water.rs`), and since #229 runs the rivers off the edge on down the land past it to ponds (`world/edge_water.rs`): rivers and streams running downhill in their beds, still lakes, riffling fords, trail bridges where a trail crosses a river with no ford, and running water, ridge wind and lake quiet in the sound. Since W11 (#204) the far land fades to a slate blue (since #233 the fog's aerial tint, Flint ADR 0101, `atmosphere.rs`), so the ranges stand blue round the city, mist lies in the valleys at dawn and dusk (`world/mist.rs`), and `--view from-wilderness` and `--view range` look in from a ridge and along the ranges. The React and Love2D clients still draw the wilds flat. The Flint client's look for the 13600 world is RW2 (#309).

The rural ring, r3000 to r4600, is laid as farms on lanes (Rural v2, `the-capitol-rural.md`), the default since R7 (#290). `backend/src/models/rural/` cuts it into the eight sectors between the arterials. Each sector is dealt one of two characters: concession roads (straight, kinked at the bisector) or winding lanes. Each has two main roads from arterial to arterial (`lane_{s}_0` inside, `lane_{s}_1` outside), the roads between them, and roads out to farm gates. About 117 farms (`farm_*` lots, about 115k each) line those roads and the arterials, with fields between them and clear of the plain's water. A balance pass adds spurs into the ground they leave open. `rural::tests::farms_meet_their_targets` holds the plan's §5 targets, and `backend/tools/rural-plan.ps1` draws the ring. The old radial trails (`trail_*`, `rural_plot_*` lots) are gone since R8 (#291), with their params and the layout switch; a saved `world_config` that still names them loads as the farms. At startup, an owned plot on one of their lots is released, its deed refunded and its stations given back (`plot_sync::RETIRED_LOTS`). In the Flint client a farm is pasture but for its yard, and the farmstead stands back from the lane on a drive (R6).

Moats and canals are water barriers. Players can only cross at bridges. Movement is blocked by water unless at a crossing point.

### Suburban Generation (suburb v2)

The suburban zone (radius 1800-3000 since #117; the radii live in `protocol/src/rules.rs`) is laid out as post-war neighbourhoods by `backend/src/models/suburb/` (the `Neighbourhood` layout, the only one since #113). The plan, with every as-built decision and the targets, is `the-capitol-suburb-v2.md`. Read it before changing the suburb.

**Neighbourhoods and frames** (`neighbourhood.rs`, `frame.rs`): each 45° sector between the arterials is a neighbourhood, or two halves split along a greenway. Each has a frame: u runs between side lines parallel to the arterials, v is radial depth, and a gentle shared warp curves every row alike. Streets are iso-lines of the frame, and five rows of lots about 106 deep fit the 1200-deep band. Each neighbourhood draws from its own SplitMix64 stream, so changing one doesn't re-id the lots of the others.

**Patterns** (one per neighbourhood, a seeded weighted pick among those whose `fits` holds):
- (a) warped parallels (`patterns.rs`): rows joined by offset rail T-links, ends as U-turns, bulbs or entrances
- (b) loops and lollipops (`loops.rs`): a racetrack collector with combs of cul-de-sacs inside (whole sectors)
- (c) central loop (`central.rs`): a loop with radial cul-de-sacs pointing out (split halves)

**Then:** 2–4 entrances per neighbourhood onto the arterials (never a 4-way across one), `street_plan::weld_junctions`, lots (`lots.rs`: back-to-back blocks, pies round bulbs, `sub_{sector}_{nbhd}_{street}_{L|R|B}_{k}` ids), and parks from the open ground the lots leave (`parks.rs`; on `/terrain` as `parks` since #129, each traced to even-odd rings with `park_{sector}_{nbhd}_{k}` ids). About 1900 lots, 2% gap.

**Checking a change:** `suburb::tests::neighbourhood_suburb_meets_its_targets` (`--nocapture` prints the soft targets it misses), and `backend/tools/suburb-plan.ps1` for SVG/PNG plans and `-Compare` against an earlier run. `default_terrain_version_is_pinned` pins the default world, in debug and release alike (#128), and on every platform: generation's trig goes through `capitol_protocol::fmath::StableMath` (pure-Rust `libm`), not the platform's libm, and clippy disallows the `f64` methods in the backend and protocol (#180). `dump_default_terrain` (ignored) writes the world to `$TERRAIN_DUMP` to diff two platforms.

**Legacy** (the rejection-sampled collectors and lollipops that came before) was removed in #113, with the `layout` switch and its params. A saved `world_config` that still names it loads as the neighbourhoods.

**Street-plan invariants** (`models/street_plan.rs`, #62): roads meet only at shared vertices, with no hairpin (a turn over 45°) outside a bulb and no road over itself. A bulb closes on its stem's last vertex. `street_plan::audit` checks it.

**Seeded nodes:** the grass nodes are seeded in migrations against the default world. `engine::node_placement` (#70) moves any that the live terrain's roads or water run over, on startup and on an admin regenerate, and saves the move. Nodes standing on lots are left alone: grass on a lot belongs to the lot, and once it's claimed only those its `station_use_level` admits may gather it (`container_access::gathering_refusal`, #102). `the_default_terrain_moves_no_seeded_node` fails if the default world itself would move one. Since E2.3 (#162), the woods, branches, outcrops, veins and clay are planned by `engine::node_spawn` from the content's spawn tables, and since W6 (#199) the wilds' nodes too (`migrations/20261113_001_wild_nodes.sql`). Their migrations staged rows in `resource_node_seeds`, because they ran before the content sync made the types. Since #207 a second migration set, `backend/migrations/post-content/`, runs after the sync. Its first migration planted the seeds and dropped the staging table, and new default-world nodes go there as a plain insert. An admin regenerate re-plans them. `the_migration_writes_the_default_plan` fails when the plan drifts: write a new migration from `print_new_nodes`. A change that moves the default world on purpose needs a move migration (the ignored test `print_node_moves` prints its rows; see `20261019_001_neighbourhood_suburb.sql`), and owned plots whose lots it removes need releasing with a refund in the same migration.

## Database

PostgreSQL with sqlx. Migrations in `backend/migrations/`, run at startup before the content sync (`backend/content/*.toml` into the static tables). A migration that writes world rows naming content (a node of a resource type, a station of a station type) goes in `backend/migrations/post-content/` instead: `migrations::run_post_content` runs that set after the sync, recorded in `_post_content_migrations` (#207). Its rules are in that directory's `README.md`.

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
