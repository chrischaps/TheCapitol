# The Capitol: The Observatory (Dev Tooling and Monitoring Plan)

*2026-10-21. Plan issue #351. The current focus since #402. Work packages #353 to #377 (§7). Issues are labelled `devtools`, with phase labels `dt:D0-groundwork` to `dt:D6-balance-runs`.*

## 1. Vision

The town can run for a whole game day: 200 citizens waking, working, trading and going to bed, with every Strand accounted for. We only learn what that day *meant* afterwards, though, by reading a markdown report and a CSV:
- the fiber gatherers end up with 88% of the Strands (#294);
- tax takes 1 Strand from 102 deeds (#335);
- logs trade at 1.1 against D5's 8;
- the tick overruns because it awaits its database writes (#304).

Every balance finding so far was dug out by hand, after the fact.

**The Observatory** is one place to *watch* the world while it runs, *change* a knob, and *see* the effect on the same chart. It has three parts:
- an **admin console** served by the backend itself, for the usual world on :3000 or the town on :3300;
- **live tunables** for the economy's rates and fees;
- a **dev overlay** in the Flint client for frame time, net and prediction.

The console should read like the Gazette's sibling, not a generic metrics wall. Each chart carries a plain sentence over it ("Fiber gatherers hold 88% of the Strands"), because the point is understanding the society, not just counting its ticks.

## 2. Goals and non-goals

**Goals**
- **Time series of the economy** while it runs: money supply, faucets and sinks, distribution, prices against target, production and stock.
- **Server performance by tick phase**, slow database work, and a profiler on demand.
- **The town reports to the backend**, so its health, decision mix and cost show live, next to the world they act on.
- **Server-side knobs changed live**, each change marked on every chart.
- **One command compares two presets** over a soak.
- **A client dev overlay**: frame time, RTT, message rates, prediction error.

**Non-goals**
- **No Prometheus, Grafana or OpenTelemetry stack** (Chris's choice: no new infrastructure). The sample format is plain JSON, so a `/metrics` exporter can be added later (§8).
- **No content hot-reload.** The TOML stays compiled in (Chris's choice: knobs only). Knobs are multipliers and rates over content, not rewrites of it.
- **No player-facing stats.** All of this is admin-only.
- **No new React or Love2D work.** Both are frozen. The console replaces the React client's F2 world tweaker and F3 perf panel for daily use, and its F1 debug camera for watching the world.

## 3. Where things stand (audit, 2026-10-21)

**Server**
- **Logging** (`main.rs`): `tracing` with `EnvFilter`, plain fmt. No spans anywhere, no metrics crate.
- **`engine/load.rs`**: since-boot atomics.
  - tick count, total, max, and one histogram over `TICK_BUCKETS_MS`
  - sockets opened and ended by `SocketEnd`, and resyncs
  - no breakdown by tick phase
- **`engine/ledger.rs`**: unit flows by `UnitFlow`, and `snapshot()`, which waits for a quiet moment so units and flows agree. `GET /admin/ledger` returns `protocol::ledger::Ledger`: accounts' Strands and flows by txn type, escrow, units, unit flows, and `Load`.
  - `protocol/src/ledger.rs` already has `reconcile` and `LoadWindow::between` (mean, p50/p95/p99, over budget, drops, DB txns/s).
  - Its only reader is `capitol-town soak`, about once a minute.
- **`routes/admin.rs`** (`require_admin` on the JWT's `is_admin`): world params (get, put, default, commit), `/stats` (`ServerStats`), `/ledger`, subscription grants, citizens.
- **History:** none, beyond the Gazette's tables (`market_trades`, `deed_events`, `production_tally`, `bazaar_sales`, `gazette_editions`).
- **Event bus:** `tokio::sync::broadcast` of `GameEvent` (`tick.rs`). Redis holds sessions only.
- **Tuning:**
  - Constants live in `protocol/src/rules.rs` and `rules/*.rs`: `LISTING_FEE_PERCENT`, `RENT_PER_DAY`, `EXCHANGE_FEE_RATE`.
  - Some are module-private, like `TAX_RATE` in `engine/property_tax.rs`.
  - The content is `include_str!`'d TOML, synced at start.
  - D5's reference prices are `SEED_PRICES` in `capitol_core::market_maker`, on the client side.

**Town** (`flint-client/crates/capitol-core`, bin `capitol-town`)
- `town.status`: `Health` per citizen.
- `town.presence.csv`.
- `town.decisions/<name>.jsonl`: each layer's verdict and latency (`citizen/decide/log.rs`).
- Remote layers' cents (`decide/remote.rs`).
- Soak and run reports (`citizen/report.rs`).
- Errors go to `eprintln!`. Nothing reaches the backend.

**Flint client**
- **Engine keys:** F2 render stats (`RenderStats`: fps, frame ms, draws, triangles), F3 all debug panels, F4 Rendering & Effects, F9 and F12 music debugging, F11 fullscreen.
- **Game keys** (`flint-client/config/input.toml`): F1 shortcuts and F10 settings. F8 is reserved for the control mode (#319). **F5, F6 and F7 are free.**
- **No game hook for `DebugPanel`s.** No GPU timestamps, no profiler, no console, no entity inspector.
- **`capitol-core/net`:** a Ping every 15 s and `last_heard`. It doesn't measure RTT or count messages or bytes.
- **`motion.rs`:** prediction and interpolation, but it exposes no error stats.

## 4. Design decisions

### D1. The console is a page the backend serves (Chris's choice)
- `/admin/console` is one HTML file plus vendored JS: uPlot for charts, small and dependency-free, committed under `backend/static/`.
- It's served with `include_str!`, like the content, so there's no build step and no Node.
- It logs in through `/auth/login` and keeps the JWT in `sessionStorage`. Every data call is an admin route.
- It works against whichever server serves it.

### D2. One sample, a ring and a table
A sampler task takes a `Sample` every 10 s (`SAMPLE_SECS`). It's a protocol type, so the console, the town's reports and tests share it. It holds:
- the load window, with tick phases;
- Strands by class (citizen, person, admin), escrow, and flows by txn type since the last sample;
- units and unit flows by item;
- VWAP and volume per item since the last sample;
- players, sockets and broadcast lag.

The newest 24 hours sit in memory, in a ring of about 8640 samples. One a minute is written to `metrics_samples` (jsonb), pruned after 30 days.

The sampler reuses `ledger::snapshot` and `LoadWindow::between`. Its cost is measured against the tick budget: it runs off the tick, and its read must never hold the tick's lock longer than `snapshot` does today.

### D3. Live data is SSE
- `GET /admin/stream` (SSE) sends each sample, the annotations, and a filtered tap of the `GameEvent` bus: fills, deed events, gazette published, warnings.
- `GET /admin/series` serves history from the ring and the table.
- SSE is one-way and plain HTTP, which is all the console needs. It reconnects on its own.

### D4. Annotations
`admin_events` holds the things that explain a bend in a chart:
- a tunable changed
- terrain committed
- server boot
- gazette published
- the town up or down
- a watch fired

Every chart draws them as markers.

### D5. Tunables are server-side, typed and bounded (Chris's choice)
`Tunables` is one struct, held in an `ArcSwap` and persisted to `tunables`. Its defaults are today's constants.

Each knob declares:
- a key
- a unit
- a range
- a default
- the chart it's watched on

`PUT /admin/tunables` validates against them and writes an annotation.

**Knobs only for what the server alone decides.** Anything the client predicts or draws to scale stays a constant: speed, ranges, radii. Some knobs the client shows the player, such as the listing fee or the rent in a confirm dialog. The server sends those values on `/health`'s catalog, or in the panel's own response, so the client never quotes a stale fee.

The first set:
- exchange fee
- listing fee percent
- stall rent per day
- property tax rate
- deed price multiplier per zone
- node regeneration multiplier
- yield multiplier per resource

### D6. The client overlay needs no engine change
- The Capitol overlay draws in the game's own `PlayerExtension::ui` (`capitol-client/src/extension.rs`) with `theme.rs` colours. It's toggled by **F7**, an action in `flint-client/config/input.toml`.
- Engine hooks make it better and are planned as their own upstream work (D5.4):
  - registering game `DebugPanel`s
  - GPU timestamps
  - a profiler feature

  None of them blocks the overlay.

### D7. Target prices live in the content
- `items.toml` gains an optional `target_price`. Its values are D5's value chain, now `SEED_PRICES` in `market_maker`, which then reads them from the catalog.
- The console plots each item's VWAP against its target.

## 5. Architecture notes

- **Tick phases** are named spans (`info_span!("tick.market")`, …) and per-phase histograms in `load.rs`. They go on `Load` as an additive field: the fixture changes, the old field stays. The phases are:
  - movement
  - extraction and crafting
  - automation
  - market
  - bazaar
  - regeneration
  - tax
  - persist
  - broadcast
- **The tracy feature** is `profile` on the backend: `tracing-tracy` as a layer. It's off by default, at zero cost when off.
- **The town's report** is `POST /admin/town/report`, as `TownReport`, a protocol type. It holds:
  - a health row per citizen
  - layer settle counts and latency since the last report
  - cents per layer
  - the last N failures
  - each citizen's last few decisions, for the console's drill-down

  The town's admin mints a scoped `town` key. The backend keeps the latest report in memory and folds a summary into the next sample.
- **Watches** are rules in code with thresholds as tunables: drift, Gini, an item off target, tax revenue, tick p95. A rule fires on its edge, not every sample.
- **`NetStats`** sits in `capitol-core`:
  - RTT from the Ping, with the Pong stamped
  - messages and bytes per type per second, in and out
  - reconnects
  - prediction corrections (count and distance) from `motion.rs`

  It has no engine dependency, so bots and citizens carry it too.

## 6. Phases and work packages

### D0: Groundwork (`dt:D0-groundwork`)
- **D0.1 Tick by phase.**
  - Spans and per-phase histograms; `Load.phases`, with its fixture.
  - **Verify:** the phases sum to within 5% of the tick, and the fixture round-trips.
- **D0.2 Sampler and history.**
  - The `Sample` type, the ring, the `metrics_samples` table and pruning.
  - **Verify:** in a 200-citizen town the sampler adds under 0.5 ms to tick p95, and a day's samples reconcile with the ledger.
- **D0.3 Admin stream and series.**
  - `/admin/stream` (SSE) and `/admin/series`, and a lag count on every broadcast receiver.
  - **Verify:** `curl -N` with an admin token shows samples and fills, and a non-admin gets 403.
- **D0.4 Annotations.**
  - The `admin_events` table and enum, written at boot, terrain commit, gazette and town up/down.
  - **Verify:** they appear in the stream and in `/admin/series`.

### D1: The console (`dt:D1-console`)
- **D1.1 The page.**
  - Login, tabs (Overview, Economy, Performance, Town, Tuning, Events), the theme as CSS tokens in light and dark, and uPlot vendored.
  - **Verify:** it opens on :3000 and :3300 and a phone-width layout holds.
- **D1.2 Overview.**
  - Health, TPS, tick p95, people and citizens on, conservation drift (red when it isn't 0), and the latest events.
- **D1.3 The map.**
  - A canvas of `/terrain`: roads, water and zones, with live dots for people and citizens.
  - Layers: nodes by type, owned plots, stations, and fill heat.

### D2: Economy (`dt:D2-economy`)
- **D2.1 Money.**
  - Supply by class, faucets and sinks stacked by txn type, escrow, and velocity.
  - **Verify:** a town run shows tax as a sliver, which is #335 at a glance.
- **D2.2 Distribution.**
  - Gini and the top-10% share over time, and wealth by role. Role comes from the town's report, and falls back to citizen or person before D4.1.
- **D2.3 Prices.**
  - `target_price` in the content, and `market_maker` reads it. VWAP, volume and book depth per item against target; off target means ±50% for a game day.
- **D2.4 Production.**
  - Made and destroyed per item per game hour, stock levels, recipe throughput, and the mean quality out per recipe.
- **D2.5 Watches.**
  - Rules, thresholds as tunables, warnings as annotations, shown in the console and the overlay.

### D3: Server performance (`dt:D3-perf`)
- **D3.1 The performance tab.**
  - Tick mean/p95/p99, phases stacked, over budget, command queue, broadcast lag, sockets, the DB pool, and DB txns/s.
- **D3.2 Slow work.**
  - sqlx `log_slow_statements`, with slow statements counted per phase.
  - A debug-build warning when a tick phase awaits the database (the #304 class).
- **D3.3 The profiler.**
  - The `profile` feature with `tracing-tracy`, documented in `CLAUDE.md`.
  - **Verify:** Tracy shows the tick's phases.

### D4: The town (`dt:D4-town`)
- **D4.1 The town reports in.**
  - The `TownReport` type, `POST /admin/town/report`, the scoped key, and `capitol-town` posting each game minute. The files stay as they are.
- **D4.2 The town tab.**
  - The roster (sort, filter), who's on now, decision mix per layer over time, and cost per citizen-day.
- **D4.3 One citizen.**
  - Their last decisions (menu, each layer's verdict, outcome), Strands over time, and their dot on the map.

### D5: The client (`dt:D5-client`)
- **D5.1 `NetStats`** in capitol-core.
  - **Verify:** unit tests over a loopback and a bot run's report.
- **D5.2 The overlay** on F7.
  - A frame-time graph, net, prediction, entities and interest.
  - **Verify:** a capture `--offline fixtures/terrain.json --screenshot-at 3` with it open.
- **D5.3 Admin in the client.**
  - The server's tick and watches.
  - World debug drawing through the Rhai `draw_*` calls: interest cells, node habitats, routes and citizens' targets.
  - Read-only tunables.
- **D5.4 Engine hooks** (`engine-upstream`, each with an ADR):
  - (a) games register `DebugPanel`s
  - (b) GPU timestamps per pass
  - (c) a profiler feature in the frame loop

### D6: Tuning and balance runs (`dt:D6-balance-runs`)
- **D6.1 Tunables.**
  - The first set from D5, persisted and validated, with an annotation per change. The client is told any value it shows.
  - Cut a ticket for any knob that can't be made live cheaply.
- **D6.2 The tuning tab.**
  - Each knob with its default, current value and reset, next to its chart. Presets export and import as TOML.
- **D6.3 Balance runs.**
  - `capitol-town soak --tunables <preset>`, and `scripts/balance.ps1 A.toml B.toml`: two soaks at `SIM_TICK_SCALE` and a comparison report (money split, Gini, prices against target, sinks).
  - It reuses `citizen/report.rs` and `reconcile`.

## 7. Dependency graph and work packages

```
D0.1 ─┬→ D0.2 → D0.3 → D1.1 ─┬→ D1.2, D1.3, D2.1–D2.4, D3.1
      ├→ D3.2, D3.3          └→ D4.2 (with D4.1) → D4.3
D0.4 ─┴→ D2.5, D6.1 → D6.2 (with D1.1) → D6.3 (with D4.1)
D5.1 → D5.2 → D5.3 (with D0.3)        D5.4 stands alone (engine)
```

The first useful slice is D0.1 to D0.3, D1.1, D1.2 and D2.1: a live chart of the money and the tick, on the town.

| WP | Issue | Phase | Depends on |
|---|---|---|---|
| D0.1 Tick by phase: spans and per-phase timings | [#353](https://github.com/chrischaps/TheCapitol/issues/353) | `dt:D0-groundwork` | none |
| D0.4 Annotations: admin_events for what explains a bend in a chart | [#354](https://github.com/chrischaps/TheCapitol/issues/354) | `dt:D0-groundwork` | none |
| D0.2 Sampler and history: one Sample every 10 s, a 24 h ring, a table | [#355](https://github.com/chrischaps/TheCapitol/issues/355) | `dt:D0-groundwork` | D0.1 #353 |
| D0.3 Admin stream and series: SSE of samples, events and annotations | [#356](https://github.com/chrischaps/TheCapitol/issues/356) | `dt:D0-groundwork` | D0.2 #355, D0.4 #354 |
| D1.1 The console page: /admin/console | [#357](https://github.com/chrischaps/TheCapitol/issues/357) | `dt:D1-console` | D0.3 #356 |
| D1.2 Console: the overview | [#358](https://github.com/chrischaps/TheCapitol/issues/358) | `dt:D1-console` | D1.1 #357 |
| D1.3 Console: the map | [#359](https://github.com/chrischaps/TheCapitol/issues/359) | `dt:D1-console` | D1.1 #357 |
| D2.1 Economy: money supply, faucets and sinks | [#360](https://github.com/chrischaps/TheCapitol/issues/360) | `dt:D2-economy` | D1.1 #357 |
| D2.2 Economy: distribution | [#361](https://github.com/chrischaps/TheCapitol/issues/361) | `dt:D2-economy` | D1.1 #357 |
| D2.3 Economy: prices against target | [#362](https://github.com/chrischaps/TheCapitol/issues/362) | `dt:D2-economy` | D1.1 #357 |
| D2.4 Economy: production and stock | [#363](https://github.com/chrischaps/TheCapitol/issues/363) | `dt:D2-economy` | D1.1 #357 |
| D2.5 Watches: rules that warn | [#364](https://github.com/chrischaps/TheCapitol/issues/364) | `dt:D2-economy` | D0.3 #356, D0.4 #354 |
| D3.1 Console: the performance tab | [#365](https://github.com/chrischaps/TheCapitol/issues/365) | `dt:D3-perf` | D1.1 #357 |
| D3.2 Slow work: slow statements and awaited I/O in the tick | [#366](https://github.com/chrischaps/TheCapitol/issues/366) | `dt:D3-perf` | D0.1 #353 |
| D3.3 The profiler: a profile feature with Tracy | [#367](https://github.com/chrischaps/TheCapitol/issues/367) | `dt:D3-perf` | D0.1 #353 |
| D4.1 The town reports in | [#368](https://github.com/chrischaps/TheCapitol/issues/368) | `dt:D4-town` | D0.2 #355 |
| D4.2 Console: the town tab | [#369](https://github.com/chrischaps/TheCapitol/issues/369) | `dt:D4-town` | D4.1 #368, D1.1 #357 |
| D4.3 Console: one citizen | [#370](https://github.com/chrischaps/TheCapitol/issues/370) | `dt:D4-town` | D4.2 #369, D1.3 #359 |
| D5.1 NetStats in capitol-core | [#371](https://github.com/chrischaps/TheCapitol/issues/371) | `dt:D5-client` | none |
| D5.2 The dev overlay on F7 | [#372](https://github.com/chrischaps/TheCapitol/issues/372) | `dt:D5-client` | D5.1 #371 |
| D5.3 Admin in the client: the server's state and world debug drawing | [#373](https://github.com/chrischaps/TheCapitol/issues/373) | `dt:D5-client` | D5.2 #372, D0.3 #356 |
| D5.4 Engine: debug panels from the game, GPU timings, a frame profiler | [#374](https://github.com/chrischaps/TheCapitol/issues/374) | `dt:D5-client` | none |
| D6.1 Tunables: live server-side knobs | [#375](https://github.com/chrischaps/TheCapitol/issues/375) | `dt:D6-balance-runs` | D0.4 #354 |
| D6.2 Console: the tuning tab | [#376](https://github.com/chrischaps/TheCapitol/issues/376) | `dt:D6-balance-runs` | D6.1 #375, D1.1 #357 |
| D6.3 Balance runs: two presets, one comparison | [#377](https://github.com/chrischaps/TheCapitol/issues/377) | `dt:D6-balance-runs` | D6.1 #375, D4.1 #368 |

## 8. Later (outside this plan)

- **A `/metrics` exporter** for Prometheus, if the game is hosted (M10). It's a thin read over the sample.
- **Content hot-reload** in dev, if the knobs prove too few.
- **Alerts out of the console**, by push notification or email, once the town runs unattended.
- **A replay scrubber** that steps the map and the charts through a stored day.

## 9. Risks

- **The sampler steals the tick.** `snapshot` waits for a quiet moment, and at 200 citizens there are few. Mitigation: it's measured in D0.2, the interval is a tunable, and the ledger read can drop to once a minute with the cheap counters every 10 s.
- **A knob the client also knows.** A fee the client quotes from a constant would go stale. D5 sends shown values from the server, and D6.1 audits the client's uses of each constant.
- **The console grows into a second client.** Keep it read-mostly. Tunables are its only write.

## 10. Verification strategy

- **Each package's Verify list** is in its issue.
- **End to end:**
  1. `scripts/town.sh up` with a full roster.
  2. Open `http://127.0.0.1:3300/admin/console` and watch money, prices and the tick move for a game hour.
  3. Change the tax rate and see its marker and the tax line move together.
- **Captures:** Claude in Chrome captures the console, and the Flint capture flags capture the overlay.
