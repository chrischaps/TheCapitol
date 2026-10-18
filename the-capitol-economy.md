# The Capitol: Economy v1 Plan

**Status:** Planning, written 2026-10-18
**Scope:** Make the economy the game. Quality and tools become real, the content grows deep enough to trade, then an order-book market (M6) and station automation (M8) arrive, with UI in the Flint client only. A health stream (CI, a catalog endpoint, content as data, auth hardening) runs alongside.
**Follows:** [`the-capitol-flint-client.md`](the-capitol-flint-client.md). P0 to P8 of that plan are all built.

**Tracking:** every work package (E0.1 … E4.4) is a GitHub issue labelled [`economy`](https://github.com/chrischaps/TheCapitol/issues?q=label%3Aeconomy) plus its phase label (`ec:E0-foundations` … `ec:E4-automation`). Client work also carries `flint-client`, and engine work carries `engine-upstream`. The issues were created in dependency order, so the lowest open, unblocked issue is next. Agents pick it up with the `next-issue` skill (`.claude/skills/next-issue/SKILL.md`). §6 gives each work package's issue number. When the plan changes, update this doc in the same commit. The plan itself was written under [#143](https://github.com/chrischaps/TheCapitol/issues/143).

---

## 1. Vision

GDD Pillar 1 says the economy *is* the game. Today the game has a beautiful world and a thin economy. You can pull grass, twist it into rope and build a forge, and then the forge has nothing to make.

This plan gives a player reasons to care about what they hold:

- **A good tool is something you're proud of.** Quality comes from your inputs, your bench and your tools, and you can see it. An A-grade iron axe is a possession, and it wears down until you decide to retire it (GDD §6.5).
- **Geography means something.** Wood grows in the rural ring and ore comes out of the wilderness, but the buyers are at the centre. The trip inward that the GDD is built around becomes the logistics of a trade.
- **Prices are public.** An order book in the Trade District, a chart of what fiber fetched today, and a daily Gazette that tells the story of the market. Players learn the economy by watching it.
- **Your workshop keeps working.** Stations you own run your recipes while you're away. A forge glows while it runs and dims when it runs out of ore. Gold spreading across the map now means industry as well as land.

## 2. Goals and non-goals

**Goals**
- The quality cascade works as the GDD describes it (§6.3), and grades survive stacking.
- Hand tools with durability, tools installed in stations, and station quality.
- Enough content that every station has something to make, and enough resources that the zones differ.
- An M6 order-book market that is global, quality-aware and escrowed, with a market panel in the Flint client and a building in the world.
- M8 automation on stations, which keeps running offline for subscribers.
- CI, a `/catalog` endpoint so the client stops hard-coding game data, content as data files, token refresh, rate limits and bot API keys.

**Non-goals for this plan** (see §8, Later)
- M9: guilds, contracts, reputation.
- Bazaar stalls, farm stands, canal tolls, licensing, certification.
- Buffs, consumables, attributes, minigames.
- Conveyors, power and fuel networks, NPC workers.
- UI in the React and Love2D clients. They're frozen and only need to keep compiling.
- Zone-sharded tick engines, deployment, backups, monitoring.

## 3. Where things stand (audit, 2026-10-18)

| Area | State | Where |
|---|---|---|
| Content | 1 resource (grass → fiber), 5 item types, 3 station types, 4 recipes. The longest chain is grass → fiber → rope → forge kit. No recipe needs a forge. | `backend/migrations/2026020{3,4,7}_*.sql`, `20260214_001_station_recipes.sql` |
| Quality cascade | Plain mean plus variance. Recipe `quality_weights` only carry the variance range. No station or tool term. | `engine/crafting.rs:108-113`, `models/recipe.rs:27` |
| Stacks | A merge averages quality, so grades blur away. | `engine/craft_completion.rs:149`, `engine/slot_moves.rs` |
| Stations | Remember `kit_quality` (#88), but it doesn't feed crafting. No tool slot. | `20261017_001_station_kit_quality.sql` |
| Tools | `resource_types.required_tool` exists and nothing reads it. No durability. | `20260203_001_m2_resources.sql:12` |
| Client game data | Hard-coded copy of the seed rows. | `flint-client/crates/capitol-core/src/catalog.rs` ("When the backend grows endpoints for these, fetch them instead") |
| Client estimate | The crafting panel shows the mean of its inputs, labelled "est.". | `capitol-core/src/craft.rs:21-24` |
| Markets, automation, guilds, contracts | Absent. | – |
| CI | None. About 200 backend tests (many `sqlx::test`) and about 500 Flint client tests run by hand. | – |
| Auth | 24 h JWT, no refresh, one session per account (a new login ends the old socket, #78), no rate limits. | `routes/auth.rs:140-213` |
| Trade leftovers | `cleanup_stale_requests` is never called. Trade offer events carry an empty `item_type`. | `engine/trading.rs:299`, `engine/tick.rs:487-492` |

**Loose ends** carried into this plan, but not part of its phases:
- [#115](https://github.com/chrischaps/TheCapitol/issues/115), [#118](https://github.com/chrischaps/TheCapitol/issues/118) and [#119](https://github.com/chrischaps/TheCapitol/issues/119): suburb.
- [#139](https://github.com/chrischaps/TheCapitol/issues/139): Flint emission mix.
- [#141](https://github.com/chrischaps/TheCapitol/issues/141): foreclosure overflow.
- [#142](https://github.com/chrischaps/TheCapitol/issues/142): spike hairlines.
- [#138](https://github.com/chrischaps/TheCapitol/issues/138), the tax-debt notices log, is a prerequisite for E4.3. Automation's pause notices use the same log.

## 4. Design decisions

Each decision is a recommendation. The ones marked **[Chris]** need his call before the work package that depends on them starts. The work package's issue says so, and an agent that reaches it posts the write-up and stops.

### D1. The quality formula (E1.1)

GDD §6.3 is followed literally:

`q = clamp(Σ wᵢ·qᵢ + w_station·q_station + w_tool·q_tool + variance, 0, 100)`

- The weights sum to 1.
- `recipes.quality_weights` grows `inputs: {item: weight}`, `station` and `tool`, which default to an even split over the inputs with `station: 0.2` when the recipe needs a station.
- `qᵢ` is the quantity-weighted mean quality of the units actually consumed for input `i`.
- The formula is one pure function in `protocol::rules` (`quality::output`) that the server and client both call. That turns the client's "est." into an exact `[min, max]` range from the variance bounds, the same move `deposit_quote` made for the exchange fee (P7.2).

### D2. A stack holds one grade (E1.2) [Chris]

Items merge only within the same letter grade (`rules::quality_grade`: A ≥ 90, B ≥ 75, C ≥ 55, D ≥ 35, F). Within a grade, quantity-weighted averaging stays.

The GDD's per-unit distributions (§6.1) and the TDD's logical `ItemStack` view are the fuller answer, but they cost a row per unit. Grade-banded stacks keep a market's `min_grade` meaningful and a craft's inputs honest, and the cost is a few more slots.

`split_stack` gets added, and the client's merge only offers same-grade targets.

### D3. Tools (E1.4–E1.6)

- **A tool is an unstackable item with durability.**
  - `items.durability` and `item_types.max_durability` hold it.
  - A ToolType (`sickle`, `axe`, `pickaxe`, `hammer`, `saw`) and a material tier go on the item type.
  - The tier is a separate item type (GDD §6.4): `stone_axe` and `iron_axe` are different items.
- **Which tool gets used:** the best matching tool in your inventory, by effective power (`base_power(tier) × quality_multiplier(q)`, both in `rules`). v1 has no equipment slot.
- **Wear:** each use costs 1 durability. Quality retention follows GDD §6.5: full above 20%, linear down to 5%, then a cliff. That curve lives in `rules`. A tool at 0 breaks and leaves the inventory with a notice.
- **Station tools:** a station has one tool slot. An installed tool supplies the recipe's tool term at that station and wears with every craft there.

### D4. Content as data (E0.3)

Static game data moves out of seed migrations into `backend/content/*.toml`:
- items, resources, stations, recipes and spawn tables
- upserted into the static tables at startup, inside one transaction
- migrations keep the schema only

A validator test checks that:
- every reference resolves
- every item can be obtained
- there are no recipe cycles
- every station type has a kit and something to make

Removing a row is an error unless it's listed under `retired`, so items in players' hands never lose their type. (As built in E0.3: see its row below. Kits are content too, as `kit` on each station.) The client reads the same data through `GET /catalog` (E0.2) and stops copying it.

### D5. Content tree v1 (E2.1)

This is a proposal for Chris to approve in E2.1. Grass stays the starter and the GDD's universal Fiber.

| Resource (zone) | Tool | Yields |
|---|---|---|
| Grass (everywhere off-road) | hand | fiber |
| Fallen branches (Suburban, Rural) | hand | sticks |
| Trees (Rural woods, Wilderness) | axe | logs |
| Rock outcrop (Rural, Wilderness) | hand / pickaxe | stone (hand: slow, low yield) |
| Iron vein (Wilderness) | pickaxe | iron ore |
| Clay bank (by the canals, Urban edge) | hand | clay |

| Station | Makes |
|---|---|
| Workbench | stone tools, handles, rope goods, kits |
| Sawbench (new) | planks from logs |
| Kiln (new) | charcoal from logs, bricks from clay |
| Forge | iron ingots from ore and charcoal |
| Anvil (new) | iron tool heads, iron fittings |
| Storage chest | – |

Tools come in stone and iron: sickle (more fiber per pull), axe, pickaxe and hammer, each made as handle + head (an *assembly* recipe in GDD §6.3 terms).

That's about 6 resources, 22 items and 22 recipes. Every station has a reason to exist, and quality flows through a five-step chain (vein → ore → ingot → head → axe) where tools and stations matter.

### D6. Where the market is (E3.4) [Chris]

There's **one global order book** (as M6 and the TDD specify). Placing or collecting an order needs you to be within range of the **Exchange hall** in the Trade District:
- Sold items and bought goods wait in your **market box** at the hall until you come.
- Browsing, the chart and cancelling work from anywhere.

This keeps the liquidity of a single book while making the journey inward the logistics of trade, as GDD §3.1 intends. The alternative, trading from anywhere, is simpler and makes the world decorative.

### D7. Market mechanics (E3.1–E3.3)

- Orders are Buy or Sell, for an item type, with a quantity, a price per unit in Strands and, on a buy, a `min_grade`.
- **Escrow:**
  - A sell order moves its items into the market's escrow container.
  - A buy order debits `quantity × price` Strands into escrow.
  - A cancel returns what's left.
- **Fee:** 1% of the order value, charged at placement as a currency sink (M6 notes) and not refunded on cancel. The fee rule is shared in `rules` like the exchange's.
- **Matching:**
  - Runs every 10 ticks (1 s), with price-time priority and partial fills.
  - It trades at the resting order's price.
  - A sell matches a buy only if its grade meets the buy's `min_grade`.
  - Each fill is one DB transaction (TDD §5.2, write-through for economic operations).
- **Lifetime:** orders expire after 7 days. Expiry returns escrow through the market box.
- **History:** `market_trades` keeps every fill. `/market/history` serves OHLC buckets for the chart.

### D8. Subscriptions (E4.1)

No payments. `accounts.subscription_status` (Free / Active / Lapsed) and `subscription_until`, set by an admin endpoint and by `capitol-bot`. One middleware extractor (`Subscriber`) gates the features in GDD §5's access matrix that this plan touches: automation that runs offline, and later guild leadership.

### D9. Sessions (E0.8) [Chris]

Today one session per account is enforced on purpose (#78). TDD §6.7 wants several, because of bots, the CLI, and a second device.

The proposal is many **sessions** but one **presence**:
- Each login gets its own refresh token.
- REST works from all of them.
- The newest WebSocket owns the avatar. An older socket is told `superseded` and drops to a read-only view.

Chris decides whether that's wanted, or whether one session stays a rule.

## 5. Architecture notes

- **Shared rules first.** Every formula a client previews goes in `protocol/src/rules.rs` with a test that the server uses it:
  - the quality output and range
  - effective tool power and wear retention
  - the market fee
  - grade bands
- **Fixtures for every wire change.** New WS messages get a JSON fixture in `protocol/tests/fixtures/` that round-trips byte for byte.
- **The tick.**
  - Market matching and automation join the tick's 10-tick (1 s) cadence (TDD §3.2).
  - Both work on in-memory state loaded at startup.
  - Both write through per economic operation.
  - Neither holds the `GameState` lock across a DB await (the B8 lesson).
- **The default world.** E2.3 adds seeded nodes against the default terrain:
  - It writes their rows in a migration and keeps `the_default_terrain_moves_no_seeded_node` passing.
  - It doesn't change terrain generation, so `default_terrain_version_is_pinned` stays put.
  - Where a node would sit on a lot, it moves rather than spawns.
- **The Flint client.**
  - New panels follow P5–P7's split: rules in `capitol-core` (pure, tested), egui in `capitol-client/src/ui/`.
  - New world objects follow P8.6b: procedural, seeded by id, merged per sector.
  - Capture flags (`--offline … --<panel>`) let every panel be screenshotted.

## 6. Phases and work packages

Sizes: S is about one session, M is 1–2, L is 2–4.

### E0: Foundations and health

| WP | Work | Size | Depends on |
|---|---|---|---|
| E0.1 [#144](https://github.com/chrischaps/TheCapitol/issues/144) | **CI.** A GitHub Actions workflow. The root workspace runs `cargo test` with a Postgres service, so `sqlx::test` runs, plus clippy. In `flint-client/`, it runs `cargo test -p capitol-core` (live tests stay ignored) and `cargo check -p capitol-client`. **As built:** `.github/workflows/ci.yml` runs two ubuntu jobs on every push (except `claim/**`) and on PRs to master. *Backend and protocol*: clippy `-D warnings` over all targets, `cargo test`, then the `#[ignore]`d database tests with `--ignored` (skipping the two tool tests, `print_node_moves` and `suburb_plan_report`). *Flint client*: ALSA and udev headers, `cargo test -p capitol-core`, `cargo check -p capitol-client --all-targets`. Both cache with `rust-cache`. A run takes about 5½ minutes (the backend job; the client job about 1 to 3), well under the 15-minute limit, so the slow terrain tests stay in. The clippy cleanup deleted dead legacy code (`engine/inventory.rs`, `ContainerState` and others). Code a later issue will wire up keeps an `allow` that names the issue (#147, #148, #157, #167, #176). Linux builds the default world a few road and park vertices apart from Windows (libm), so `default_terrain_version_is_pinned` pins one hash per platform ([#180](https://github.com/chrischaps/TheCapitol/issues/180)). | M | – |
| E0.2 [#145](https://github.com/chrischaps/TheCapitol/issues/145) | **`GET /catalog`.** Item, station, resource and container types and recipes from the DB, versioned by a hash. `capitol-core` fetches it after login, caches it on disk, and `catalog.rs` becomes the offline fallback. **As built:** `GET /catalog` (public) is built once at startup by `engine::catalog::build` from the in-memory tables (container types are now loaded too) and kept in `GameState::catalog`. Lists are sorted by id; `version` is FNV-1a over the JSON, like the terrain's. Station types carry their `kit`. Recipes are all of them, hand and station. `/health` reports `catalog_version`. No ETag: the body is a few KB. In the client, `catalog::Catalog` lives in the store: the snapshot fetches `/catalog` in place of `/recipes`, and a `/health` naming another version refetches it (on the login screen, the session asks itself, since the store is idle there). It's cached per server in `cache/catalog-<server>.json` next to the terrain cache (not `save/`), and survives a sign-out. The fallback, `capitol-core/src/catalog_fallback.json`, is byte-for-byte the protocol fixture `protocol/tests/fixtures/rest/catalog.json` (a test keeps them equal), captured from a seeded server. An item type the client doesn't know gets a name from its id and the placeholder icon. | M | – |
| E0.3 [#146](https://github.com/chrischaps/TheCapitol/issues/146) | **Content as data (D4).** `backend/content/*.toml`, upserted at startup, with a validator test. Today's seeds move across unchanged. **As built:** `items`, `resources`, `containers`, `stations`, `recipes` and `retired.toml`, each documenting its format at the top. They're built into the binary (`include_str!`; cargo rebuilds when one changes), so a server can't start on content from another checkout. `content::load_and_sync` runs after the migrations: it parses (unknown fields are an error), validates, then upserts every table in one transaction, in foreign-key order. A table row the files neither define nor retire stops startup and writes nothing, naming the row and the fix. `content::validate` is the validator: references resolve, ids are unique and not also retired, every item can be had by a player starting empty-handed (a resource yields it given its tool, or a recipe reachable that way makes it, given its station's kit), no recipe cycles, every station's kit is made by a recipe, and every non-storage station is some recipe's station. The forge isn't yet, so it's allow-listed (`NOTHING_TO_MAKE_YET`) until E2.4. Startup runs the validator too, not only `cargo test`. A station type's kit is now content: `kit` in `stations.toml`, the new `station_types.kit_item` column (its migration fills today's three rows, so a database is the same before and after its first sync). Placement, take-down and `/catalog` read it there, replacing four hard-coded copies. The Flint client still pairs kits through `rules::station_kit_item`; a test keeps the two equal until [#181](https://github.com/chrischaps/TheCapitol/issues/181) moves the client onto the catalog. `/catalog` is byte-identical to before, so the fixture and fallback needed no recapture. Spawn tables stay out: there are none yet, and nodes are world instances. | M | – |
| E0.4 [#147](https://github.com/chrischaps/TheCapitol/issues/147) | **Trade leftovers.** Run `cleanup_stale_requests` on the tick, and fill trade offer events' `item_type`. | S | – |
| E0.5 [#148](https://github.com/chrischaps/TheCapitol/issues/148) | **Unused protocol messages.** Wire up or delete `currency_changed`, `nearby_nodes`, `nearby_stations`, `item_moved` and `items_merged`, with their fixtures. | S | – |
| E0.6 [#149](https://github.com/chrischaps/TheCapitol/issues/149) | **Token refresh.** `POST /auth/refresh` with rotating refresh tokens in Redis. `capitol-core` refreshes silently before expiry and on a 401. | M | – |
| E0.7 [#150](https://github.com/chrischaps/TheCapitol/issues/150) | **Rate limits.** A tower layer per route group (auth by IP, commands by account), with limits in config and a `429` the client understands. | S | – |
| E0.8 [#151](https://github.com/chrischaps/TheCapitol/issues/151) | **Sessions (D9) [Chris].** Many sessions and one presence, or a recorded decision to keep one. | M | E0.6 |
| E0.9 [#152](https://github.com/chrischaps/TheCapitol/issues/152) | **Bot API keys.** Scoped keys (`read`, `play`, `trade`) an account can mint and revoke. `capitol-bot` logs in with one. | M | E0.6 |

### E1: Quality and tools

| WP | Work | Size | Depends on |
|---|---|---|---|
| E1.1 [#153](https://github.com/chrischaps/TheCapitol/issues/153) | **The weighted cascade (D1).** `rules::quality::output` and `output_range`; `quality_weights` grows its weights; crafting uses it; the client shows the exact range. | M | – |
| E1.2 [#154](https://github.com/chrischaps/TheCapitol/issues/154) | **One grade per stack (D2) [Chris].** Merges only within a grade, `split_stack` (WS and REST), and shift-drag to split in Flint. | M | E1.1 |
| E1.3 [#155](https://github.com/chrischaps/TheCapitol/issues/155) | **Station quality.** `kit_quality` becomes the station's quality (rename it, or alias it), feeds the station term, and shows in the station panel. | S | E1.1 |
| E1.4 [#156](https://github.com/chrischaps/TheCapitol/issues/156) | **Hand tools (D3).** Durability, ToolType, tier, effective power and the wear curve in `rules`; best-tool selection; breakage with a notice. | L | E1.1 |
| E1.5 [#157](https://github.com/chrischaps/TheCapitol/issues/157) | **Tools at work.** Extraction honours `required_tool`, and the tool's power sets yield and speed, its retention scales quality. Craft recipes take a tool term. | M | E1.4 |
| E1.6 [#158](https://github.com/chrischaps/TheCapitol/issues/158) | **Station tool slot.** Install and remove (WS), wear per craft, the tool term at that station. | M | E1.3, E1.4 |
| E1.7 [#159](https://github.com/chrischaps/TheCapitol/issues/159) | **Flint: quality you can read.** Grade and durability on slots and tooltips, the crafting panel's exact range with what drives it (inputs, station, tool), and the station panel's tool slot. | M | E1.2, E1.5, E1.6 |

### E2: Content depth

| WP | Work | Size | Depends on |
|---|---|---|---|
| E2.1 [#160](https://github.com/chrischaps/TheCapitol/issues/160) | **Content tree v1 (D5) [Chris approves].** Final list of resources, items, stations, recipes and weights, written into D5 with the quality weights per recipe type. | S | E0.3 |
| E2.2 [#161](https://github.com/chrischaps/TheCapitol/issues/161) | **Resource types.** Difficulty, tool gating, yield by tool, and zone spawn tables in content data. | M | E2.1, E1.5 |
| E2.3 [#162](https://github.com/chrischaps/TheCapitol/issues/162) | **Seeded nodes by zone.** Deterministic placement from the terrain (woods in clusters, outcrops, veins, clay by the water), clear of roads, water and lots, written by a migration. | L | E2.2 |
| E2.4 [#163](https://github.com/chrischaps/TheCapitol/issues/163) | **Seed content v1.** The new items, stations, kits and recipes through the content pipeline. Every station makes something. | M | E2.1, E0.3 |
| E2.5 [#164](https://github.com/chrischaps/TheCapitol/issues/164) | **Flint: the new nodes.** Procedural trees, branch litter, rock outcrops, iron veins and clay banks, with grass clearings (H25) and a depleted look. | L | E2.3, E0.2 |
| E2.6 [#165](https://github.com/chrischaps/TheCapitol/issues/165) | **Flint: items and stations.** Icons for every new item and models for sawbench, kiln and anvil, all from the catalog. | M | E2.4, E0.2 |
| E2.7 [#166](https://github.com/chrischaps/TheCapitol/issues/166) | **Economy sim.** `capitol-bot --sim`: a swarm gathers and crafts headless against a local stack for N hours (with a time multiplier), then reports throughput, quality spread per item and tool wear. Used to tune E2.4. | M | E2.4 |

### E3: Markets (M6)

| WP | Work | Size | Depends on |
|---|---|---|---|
| E3.1 [#167](https://github.com/chrischaps/TheCapitol/issues/167) | **Market model (D7).** `market_orders`, `market_trades`, the escrow container and market boxes; DTOs and fixtures in `protocol`; the fee in `rules`. | M | E1.2 |
| E3.2 [#168](https://github.com/chrischaps/TheCapitol/issues/168) | **Matching engine.** On the 1 s cadence, with price-time priority, partial fills, `min_grade`, expiry, one transaction per fill, and exhaustive tests. | L | E3.1 |
| E3.3 [#169](https://github.com/chrischaps/TheCapitol/issues/169) | **Market API.** REST create, cancel, list (book depth), my orders and history (OHLC); a WS `market:<item>` channel; fill notices. | M | E3.2 |
| E3.4 [#170](https://github.com/chrischaps/TheCapitol/issues/170) | **Where trading happens (D6) [Chris].** The Exchange hall's position in the Trade District on `/terrain`, the range check on place and collect, and collecting from the market box. | S | E3.3 |
| E3.5 [#171](https://github.com/chrischaps/TheCapitol/issues/171) | **Flint: market panel.** Browse items with activity, a bid/ask ladder, the order form with fee preview, my orders, fill toasts. | L | E3.3 |
| E3.6 [#172](https://github.com/chrischaps/TheCapitol/issues/172) | **Flint: price chart and history.** `egui_plot` candles or line with volume, by grade. | M | E3.5 |
| E3.7 [#173](https://github.com/chrischaps/TheCapitol/issues/173) | **Flint: the Exchange hall.** A procedural building in the Trade District that opens the panel in range and lights up at dusk. The market box shows when goods wait. | M | E3.4, E3.5 |
| E3.8 [#174](https://github.com/chrischaps/TheCapitol/issues/174) | **Market-maker bots.** `capitol-bot --market` quotes both sides around a reference price, for liquidity on a local stack and in E2.7's sim. | S | E3.3, E2.7 |
| E3.9 [#175](https://github.com/chrischaps/TheCapitol/issues/175) | **The Capitol Gazette.** A daily digest built from the day's trades, deeds and records (biggest movers, best-quality sale, new deeds, top producers), served by REST and posted on a notice board by the tower. It gives the economy a story. | M | E3.3 |

### E4: Automation (M8)

| WP | Work | Size | Depends on |
|---|---|---|---|
| E4.1 [#176](https://github.com/chrischaps/TheCapitol/issues/176) | **Subscriptions (D8).** Status, an admin and bot activation, and the `Subscriber` extractor. | S | – |
| E4.2 [#177](https://github.com/chrischaps/TheCapitol/issues/177) | **Automation config.** A recipe and a repeat count or "forever" per station; `PUT /stations/{id}/automation`, start and stop; the station's own container is its input and output. | M | E1.6, E2.4 |
| E4.3 [#178](https://github.com/chrischaps/TheCapitol/issues/178) | **Automation tick.** On the 1 s cadence: pull inputs, craft with the cascade, push the output, wear the tool, and pause on MissingInput, OutputFull, ToolBroken or OwnerOffline. Subscribers keep running offline. Pauses go to the notices log. | L | E4.1, E4.2, [#138](https://github.com/chrischaps/TheCapitol/issues/138) |
| E4.4 [#179](https://github.com/chrischaps/TheCapitol/issues/179) | **Flint: automation.** A panel on the station (recipe, run and stop, status, production log). In the world, running stations glow and smoke (H23 emission, particles) and paused ones dim. | M | E4.3 |

## 7. Dependency graph and estimate

```
Quality and tools   E1.1 ─┬─ E1.2 ──────────────────────────── E3.1 ─ E3.2 ─ E3.3 ─┬─ E3.4 ─┐
                          ├─ E1.3 ─┬─ E1.6 ─┬─ E1.7                                ├─ E3.5 ─┴─ E3.7
                          └─ E1.4 ─┴─ E1.5 ─┘                                      │    └─ E3.6
                                      │                                            ├─ E3.9
Content             E0.3 ─ E2.1 ─┬─ E2.2 ─ E2.3 ─ E2.5   (E2.5, E2.6 also need E0.2) └─ E3.8 (+E2.7)
                                 └─ E2.4 ─┬─ E2.6
                                          ├─ E2.7
Automation          E1.6 + E2.4 ──────────┴─ E4.2 ─ E4.3 ─ E4.4      (E4.3 also needs E4.1 and #138)
Health              E0.1   E0.2   E0.4   E0.5   E0.7   E0.6 ─┬─ E0.8
                                                             └─ E0.9
```

| Phase | Sessions (approx.) |
|---|---|
| E0 | 9–13 |
| E1 | 9–13 |
| E2 | 10–15 |
| E3 | 12–17 |
| E4 | 5–8 |
| **Total** | **about 45–66** |

E0 and E1 can run in parallel worktrees from the start. E2.1 and E1.2 are the first decision points for Chris.

## 8. Later (outside this plan, but planned for)

- **M9 social:** guilds (registry, roles, treasury, dues), contracts with escrow, and reputation. The market's escrow and the notices log are their foundations.
- **Shops:** bazaar stalls and farm stands (GDD §3.4), sold from a plot or a rented stall, which the market box generalises to.
- **The bureaucracy's other fees** (GDD §8.4): deed transfer, canal tolls, certification, licensing.
- **Buffs, consumables, minigames** (GDD §6.3, §7), and NPC workers and conveyors for automation (GDD §3.7).
- **M7 leftovers:** deed items and transfers, enforcing plot permissions on entry, the map, the ownership overlay, the permissions editor.
- **Admin egui panels** gated on `is_admin`.
- **M10 operations:** deployment, backups, monitoring, load tests, zone-sharded ticks (TDD §7).

## 9. Risks

| Risk | Mitigation |
|---|---|
| A market with few players has no liquidity | Market-maker bots (E3.8). The sim (E2.7) shows where supply pools. |
| Balance: one chain dominates or nothing is worth making | The sim reports throughput and value per hour per chain. Content as data makes tuning a TOML edit. |
| New nodes move the default world, or land on owned lots | E2.3 writes migrations, skips lots, and keeps the default-world tests green. |
| Grade-banded stacks fill inventories | Watch slot pressure in the sim. A sort-and-stack button is cheap if it bites. |
| Escrow bugs lose items or Strands | Every fill and cancel is one transaction, with property tests for conservation (items and Strands in = out + fees). |
| Decisions block agents | D2, D6 and D9 and the E2.1 tree are flagged in their issues. Agents write up and stop rather than guess. |

## 10. Verification strategy

- Backend: `cargo test -p capitol-backend` (with the docker stack up for `sqlx::test`), plus the work package's named tests. CI runs it all from E0.1 on.
- Shared rules: a test in `protocol` for each formula, and a test that the server calls it.
- Client: `cargo test -p capitol-core` and `cargo build --release -p capitol-client` from `flint-client/`, with offline captures into `flint-client/renders/` for anything visual, and [H] for Chris's eye.
- End to end: `capitol-bot` drives each feature against a local stack: gather with a tool, craft with a station, place and fill orders, run an automated station overnight.
