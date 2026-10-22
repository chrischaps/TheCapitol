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

- **Tick phases** are named spans (`tick.movement`, …) and per-phase histograms in `engine/load.rs` (`load::Phase`). They go on `Load` as an additive field, `phases`: the fixture (`protocol/tests/fixtures/rest/ledger.json`) changes, the old fields stay, and a load from before reads with no phases. *As built (D0.1, #353):* the phases partition the tick, so they add up to `tick_secs`. Since automation, the market and the bazaar run as loops of their own beside the tick, not in it, they're timed apart, in `Load.loops` (`load::Loop`, spans `loop.<name>`), and "persist" is split three ways, because #304 needs to know which write it is. The phases are:
  - lock: waiting for the game's lock, which automation and the routes take too
  - commands, movement, extraction, crafting, regeneration
  - trades: expiring unanswered trade requests
  - broadcast: the positions that changed
  - gathers, crafts, positions: the writes (`persist_gather`, `finish_craft`, `persist_positions`)
  - tax, subscriptions: the hourly jobs

  The loops are automation, market and bazaar. `LoadWindow` gives each phase and loop its runs, time, mean and p95 over a window.

  *Since #304* the tick awaits no database. Gathers and crafts go to a writer task, which saves them in order and only then tells the player. Every player's position is read under the tick's own lock and saved in one `UNNEST` statement, in a task of its own. That happens every 100 ticks of the world's count: the loop's own count was reset by the tick-rate log, so positions were being saved about twice as often as intended. The hourly tax and the subscriptions' lapse run as tasks too. So there are now 12 phases. Gathers and crafts are one phase, **writes**: the hand-off to the writer. Positions is the read, and tax and subscriptions are their start. The work itself is timed in four new loops: **writer**, **positions**, **tax** and **subscriptions** (spans `loop.<name>`). At 200 citizens and 10× the tick fell from 8.3 ms on average, 13.9% over budget, to 0.47 ms with none over. The writer is one task and about 40% busy at that load, so its backlog wants a gauge ([#407](https://github.com/chrischaps/TheCapitol/issues/407)).
- **The tracy feature** is `profile` on the backend: `tracing-tracy` as a layer. It's off by default, at zero cost when off.
- **The town's report** is `POST /admin/town/report`, as `TownReport`, a protocol type. It holds:
  - a health row per citizen
  - layer settle counts and latency since the last report
  - cents per layer
  - the last N failures
  - each citizen's last few decisions, for the console's drill-down

  The town's admin mints a scoped `town` key. The backend keeps the latest report in memory and folds a summary into the next sample. *As built (D4.1, #368):* see §6.
- **Watches** are rules in code with thresholds as tunables: drift, Gini, an item off target, tax revenue, tick p95. A rule fires on its edge, not every sample. *As built (D2.5, #364):* `capitol_protocol::watch`, run by the sampler through `engine::watches`; see D2.5.
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
  - *As built (#353):* `Load.phases` (13 phases that cover the tick, 12 since #304) and `Load.loops` (automation, market, bazaar, timed apart since they aren't in the tick, and since #304 the writer, positions, tax and subscriptions). See §5.
- **D0.2 Sampler and history.**
  - The `Sample` type, the ring, the `metrics_samples` table and pruning.
  - **Verify:** in a 200-citizen town the sampler adds under 0.5 ms to tick p95, and a day's samples reconcile with the ledger.
  - *As built (#355):* `capitol_protocol::metrics`: `Sample` (`at_ms`, `since_ms`, the `LoadWindow`, now serde, `Strands` by citizens, people, admins and escrow, `strand_flows` and `unit_flows` over the window, `units`, `market` and `bazaar` as `Trades` per item with `vwap()`, `Players` people and citizens, and `EventBus`, the slowest receiver's backlog against the bus's capacity), `Sample::between` over two ledgers, and `tally`, which a run of samples adds up to as `reconcile` does over its ends. `engine::sampler` runs as a loop of its own, timed as `load::Loop::Sampler` (`loops.sampler`); it keeps `AppState::samples` (`Samples`, a ring of `RING_SAMPLES`, 8640) and writes one every `STORE_EVERY` samples to `metrics_samples` (jsonb, keyed by `at`), covering the whole minute since the last one stored, so the table tallies like the ring, pruning past `KEEP_DAYS` hourly. `METRICS_SAMPLE_SECS` sets the interval, and 0 turns it off. Admins are read by a query of their own each sample, so `AccountBooks` is unchanged. `market_trades` gained an index on `at`. A missed sample (a failed read) lengthens the next window rather than leaving a gap in the flows. The sampler's ledger tries once for a quiet moment (`sampler::LEDGER_TRIES`), not the 50 times `/admin/ledger` does: in a busy town there's never one, and the 50 rereads of every unit took the tick's mean from 23 to 36 ms (#408). Its flows still add up, but most samples are `units_settled: false`. **Measured** in a 224-citizen town (debug, 1×, always on), over 5-minute windows, before #304 took the writes out of the tick: off 23.3 and 22.0 ms mean; on 27.5 and 23.7, with p95 in the 20–50 ms bucket in every run and `lock` 0.19 to 0.22 ms a tick. A sample's reads take about 13 ms off the tick, and longer only when the whole server stalls on the tick's lock (#304). The buckets can't resolve 0.5 ms at p95 (#409).
- **D0.3 Admin stream and series.**
  - `/admin/stream` (SSE) and `/admin/series`, and a lag count on every broadcast receiver.
  - **Verify:** `curl -N` with an admin token shows samples and fills, and a non-admin gets 403.
  - *As built (#356):* `capitol_protocol::metrics::Feed`, tagged by `type`: `sample`, `annotation`, `fill` (a market fill, the `MarketTrade`), `deed` (claimed, released, foreclosed), `gazette`, `warning` (tax unpaid, a subscription run out, a station displaced or paused; a pause because the owner went home isn't one) and `lagged` (this stream fell behind by `missed`). Fixture: `rest/admin_feed.json`. `engine::admin_feed` is one broadcast for the process, apart from the game's bus: the sampler sends each sample, `annotations::note` (and `tunables::update`, after its commit) each annotation via `annotations::announce`, and `run_tap`, one reader of the game's bus, the fills, deeds, Gazettes and warnings. With no stream open, nothing is converted or sent. `GET /admin/stream` names each event for its `type`, with the whole `Feed` as JSON, and opens with the newest sample. It takes the bearer header like every admin route, so **the console reads it with `fetch` and a stream reader, not `EventSource`**, which can't send a header (D1.1). `GET /admin/series?from_ms&to_ms&fields` (an hour to now by default; `from` exclusive, `to` inclusive) returns `Series`: the ring where it reaches and `metrics_samples` before it (or after a restart), and the annotations between. With `fields` (comma-separated) each sample keeps only those and its times (`SERIES_TIMES`), projected in the database for the table. Lag: `Load.events_lagged` and `LoadWindow.events_lagged` count the events each reader missed, `sockets`, `admin_tap` and `admin_stream` (`load::Receiver`); a sample stored before reads with none. Verified on a scratch server: samples, a fill and a `tunable_changed` annotation came down `curl -N`, and a non-admin got 403 on both. Bazaar sales aren't in the tap; they're in each sample's `bazaar`.
- **D0.4 Annotations.**
  - The `admin_events` table and enum, written at boot, terrain commit, gazette and town up/down.
  - **Verify:** they appear in the stream and in `/admin/series`.
  - **As built (#354):** `capitol_protocol::annotation` (`Annotation`, `AnnotationKind` with all six kinds), the `admin_events` table (`kind` checked against the enum's names, so a new kind needs a migration), and `engine::annotations`: `record`, `note` (logs a failure rather than failing what it marks) and `since`. Written at boot, at the terrain commit and by `gazette::publish` for each new edition. Town up/down waits on D4.1's report, tunables on D6.1 and watches on D2.5; each calls `record`. Read by `GET /admin/annotations?since_ms=&limit=` (24 h by default) until D0.3 serves them in `/admin/series` and the stream, where `record` should also send to the stream.

### D1: The console (`dt:D1-console`)
- **D1.1 The page.**
  - Login, tabs (Overview, Economy, Performance, Town, Tuning, Events), the theme as CSS tokens in light and dark, and uPlot vendored.
  - **Verify:** it opens on :3000 and :3300 and a phone-width layout holds.
  - *As built (#357):* `backend/static/console.{html,css,js}` and uPlot 1.6.31 (`static/vendor/`, MIT), served by `routes::console` with `include_str!`, and the Flint client's Cinzel and Source Sans 3 straight from `flint-client/fonts/` with `include_bytes!`. The page and its files sit outside the admin auth layer; every number comes from the admin routes. Sign-in is `/auth/login`, with the token and refresh token in `sessionStorage`; a 401 refreshes once, then signs out, and a non-admin is told so and their session ended. The stream is read with `fetch` and a stream reader, reconnects with backoff, and rereads the series after a gap. The look: in light, the Gazette's newsprint (paper, ink, a deckle edge, the masthead between rules); in dark, the game's night (panels, gold). It follows `prefers-color-scheme`, with a toggle kept in `localStorage`. Every colour is a CSS token, and the charts read theirs from the tokens. Each story is a section head, a plain sentence computed from the data, a dek and a chart, with annotations as dashed marks and a line under the chart naming the nearest one. Each tab has one story, for its package to grow: Overview, who's about (D1.2); Economy, Strands by class (D2.1); Performance, the tick's mean and p95 on a log scale against the budget (D3.1); Town, the roster from `/admin/citizens` (D4.2); Tuning, every knob read-only (D6.2); Events, the wire: annotations, fills, deeds, warnings, filterable, with a Gazette shown once, by its annotation. The range is 1, 6 or 24 h. The series is read by field sets: `players,strands` always, `load` only for the Performance tab, because a field can't reach inside a sample and a day of `load` is about 26 MB ([#411](https://github.com/chrischaps/TheCapitol/issues/411)).
- **D1.2 Overview.**
  - Health, TPS, tick p95, people and citizens on, conservation drift (red when it isn't 0), and the latest events.
  - *As built (#358):* three stories on the Overview tab. **The city at a glance** is a row of six tiles, like a front page's index: each has a figure, a sparkline over the range and a line of words, and one that isn't well takes the down ink and a rule across its top. The headline over them names what to look at ("All's well…", or each problem in a clause, the other watches firing among them). The tiles:
    - **health**: `/health` (the database and Redis), asked at most every 30 s while the tab is open, and "Quiet" when the stream is live but no sample has come for three periods;
    - **ticks a second**: the window's ticks over its seconds, against `1000 / tick_period_ms`, so it holds at any `SIM_TICK_SCALE`; amber under 95%;
    - **tick p95**, against the period, with the mean beside it; red over it;
    - **people on**, with the sockets open, and **citizens on**, with the town's standing (working, resting, away) when a town reports in;
    - **drift**: `Sample::drift` (new, `metrics::Drift`), the drift watch's running count since boot of Strands the audit trail doesn't explain and units their flows don't (`Watches::drift`). It reads 0 in the up ink, or red with the amount. A sample from before it has none, and the tile falls back on the drift watch.

    The tick's sparklines read `load`, which the series sends only for the Performance tab (#411), so until that tab is opened they hold what the stream has brought since the page opened. **Who's about** is D1.1's chart. **The latest** is the newest 8 on the wire, a run of fills folded into one line, with a link to the Events tab.
- **D1.3 The map.**
  - A canvas of `/terrain`: roads, water and zones, with live dots for people and citizens.
  - Layers: nodes by type, owned plots, stations, and fill heat.
  - *As built (#359):* a Map tab, second after the Overview. `/terrain` is read once and turned into `Path2D`s in world units, one per kind and width of road, so a frame is a handful of strokes. The map shows the zones as alternating washes (radii from `GET /admin/map`'s `zones`, which come from `rules::ZONE_OUTER_RADII`, so the page holds no copy), parks, lakes, rivers, streams, the moat and canals broken at their bridges, lot outlines once lots are big enough to tell apart, roads (trails dashed), and the hall, bazaar and stalls. **Positions are polled, not streamed.** `GET /admin/positions` is every player as a `Walker` (D5.3's type, gzipped: about 175 bytes a walker, 1.5 KB on the wire for 20). The map reads it every 2 s while its tab is open and the page visible, and eases each walker between reads (no easing under `prefers-reduced-motion`, and none across a jump of over 400). A `positions` feed on `/admin/stream` would send to every open console all the time, even with the map closed. Citizens are hollow diamonds (their mark, ◇) and people solid dots, with a faint dashed line to each walker's `dest`. `GET /admin/map?hours=` holds the slower layers, read every 30 s and 3 s after a fill or deed on the stream: the owned lots (`MapPlot`, citizen or not), the stations (`MapStation`), and **fill heat**, the trade over the range by place. A market fill has no place but the Exchange hall, so the hall's fills glow at its door and each stall's sales at its stall (`FillHeat`, `market` and `stalls` as `metrics::Trades`). That makes it a heat of the centre, a view of the hall against the bazaar, not of the city. Nodes come from `/world/nodes` every 60 s, coloured by type (`--n-<type>` tokens), with depleted ones faint. The layers toggle and are kept in `localStorage`. Drag to pan, wheel or pinch to zoom, arrows and +/−/0 on the focused canvas. Hover names what's under the cursor, and a click follows a walker (D4.3's "their dot on the map" can use it). Under the map is a key with counts, and the headline says who's out and what they're doing. The wilds are drawn flat, without their relief, because `land::height_at` is Rust and the page has no build step ([#415](https://github.com/chrischaps/TheCapitol/issues/415)). Fixture: `rest/admin_map.json`.

### D2: Economy (`dt:D2-economy`)
- **D2.1 Money.**
  - Supply by class, faucets and sinks stacked by txn type, escrow, and velocity.
  - **Verify:** a town run shows tax as a sliver, which is #335 at a glance.
  - *As built (#360):* four stories on the Economy tab. **The city's Strands** is D1.1's chart. **Coming and going** is the faucets above the line and the sinks below it, stacked by kind in bars (5 minutes at 1 h, 15 at 6 h, an hour at 24 h), with the range's account under it: each kind's total as a ruled bar, never thinner than a hairline so a Strand of tax still shows, and what they net to. Its headline names what came in and each sink in turn, and then what tax took of it, or "No tax was paid". Transfers and escrow only move Strands about, so they aren't drawn. Transaction types share a line where they mean one thing (the three refunds; `tax_payment` with `property_tax`). The page holds no copy of which type is a faucet: `GET /admin/books` (`rest::AdminBooks`, fixture `rest/admin_books.json`) serves `ledger::TRANSACTION_TYPES` with each one's `StrandFlow` (now serde), what a newcomer starts with, and the game day's length. A type the page has no words for is drawn on its own. A newcomer's 100 Strands have no audit row, so `Sample.new_accounts` (additive, `serde(default)`) counts the accounts made in the window. **Held for buy orders** is escrow on its own scale, since against the citizens' Strands it's a flat line. **How fast it moves** is velocity: what changed hands on the market, at the bazaar and hand to hand (`trade_received`, the gross half of a trade's pair) over the mean supply, as times a game day at each bar's pace. The flows and trades are a field set of their own (`strand_flows,new_accounts,market,bazaar`), read only for this tab, like `load` (#411). **Seen** on a copy of a 224-citizen town's database: 6,473 Strands in, all deposits, and 2,510 out, deeds 2,500 and fees 10, with no tax at all in six hours, which is #335. Nothing traded, which is #410.
- **D2.2 Distribution.**
  - Gini and the top-10% share over time, and wealth by role. Role comes from the town's report, and falls back to citizen or person before D4.1.
- **D2.3 Prices.**
  - `target_price` in the content, and `market_maker` reads it. VWAP, volume and book depth per item against target; off target means ±50% for a game day.
- **D2.4 Production.**
  - Made and destroyed per item per game hour, stock levels, recipe throughput, and the mean quality out per recipe.
- **D2.5 Watches.**
  - Rules, thresholds as tunables, warnings as annotations, shown in the console and the overlay.
  - *As built (#364):*
    - **The rules.** `capitol_protocol::watch` holds the five rules as pure code. `Watches::observe` takes a `Reading` each sample and returns what fired (`Fired`: the `Watch` and a plain sentence):
      - **drift:** Strands or units the books don't explain, counted since the server started, so it stays red until a restart. Strands are judged every sample from the window's `reconcile`. Units are judged only from one settled ledger to the next, which a busy town seldom gives (#408).
      - **gini:** the Gini of people's and citizens' Strands, not admins', with at least 10 accounts. `metrics::gini` is there for D2.2.
      - **off target:** one watch per item, `Watch { kind, item }`. It fires when the market's VWAP over the last game day (2 h) is further from the item's target than the threshold, either way, once a whole game day has been watched.
      - **tax revenue:** the mean of the last 3 hourly runs, under a floor, while any deed is held. Revenue is `property_tax` plus `tax_payment`. Each run is told apart by `loops.tax.runs`, and everything collected since the run before is its own, so a run that lands a sample late still counts in full, at any `SIM_TICK_SCALE`.
      - **tick p95:** the window's p95 bucket over the tick's period times the threshold. Past the last bucket counts as over.
    - **Firing.** A watch fires on its edge. It clears only after `CLEAR_AFTER` (30) samples back within, five minutes, so a value hovering at its threshold doesn't fire every other sample. A rule with nothing to judge (no ticks, no trades, no tax run yet) leaves its watch as it was.
    - **The thresholds** are four new knobs: `watch_gini` (0.7, a new `Unit::Ratio`), `watch_off_target` (50%), `watch_tax_per_hour` (1 Strand, which #335's 1 Strand a day trips) and `watch_tick_p95` (×1 of the period). They live in `Tunables::watches` (`WatchThresholds`) and are read each sample, so a change takes on the next. A Gini of 1 or a tax floor of 0 never fires.
    - **The backend.** `engine::watches::Watcher` runs in the sampler loop. It counts the deeds held, writes each firing as a `watch_fired` annotation through `annotations::note` (so it reaches the stream and `/admin/series`), and logs it as a warning. `Sample::watches` lists the watches firing as each sample was taken, stored ones too, for the console and D5.3's overlay. The `sample.json` fixture has a Gini firing.
    - **No item has a target yet.** D2.3 (#362) puts `target_price` in the content and hands it to `Watcher::check`, which passes an empty map until then.
    - **Verified** on a scratch server with 2 s samples. An unaudited balance write fired drift and the Gini once each, and the next 20 samples carried them as firing. Raising `watch_gini` to 0.8 cleared it a minute later, and lowering it to 0.5 fired it again, with each change annotated.

### D3: Server performance (`dt:D3-perf`)
- **D3.1 The performance tab.**
  - Tick mean/p95/p99, phases stacked, over budget, command queue, broadcast lag, sockets, the DB pool, and DB txns/s.
- **D3.2 Slow work.**
  - sqlx `log_slow_statements`, with slow statements counted per phase.
  - A debug-build warning when a tick phase awaits the database (the #304 class).
  - *As built (#366):* `engine::slow_work`. The pool logs a statement at `SLOW_STATEMENT_MS` (50 unless set) or over as a warning on `sqlx::query`. The `SlowWork` layer finds the `tick.<phase>` or `loop.<name>` span it ran in, and counts it in `PhaseLoad.slow_statements` for that phase or loop, or in `Load.slow_statements_elsewhere` (routes, sockets, startup). `Load.slow_statement_ms` says what slow was, and `LoadWindow` carries both. The layer has a filter of its own, which takes the engine's spans as well as sqlx's events, so `RUST_LOG` only decides what's printed; the default adds `sqlx::query=warn`. In debug builds it sees every statement, and the tick warns, after its work, the 1st, 10th, 100th… time a phase awaits one, naming the statement (a layer can't log from inside a log event). Today that's gathers, crafts, positions, tax and subscriptions: #304.
- **D3.3 The profiler.**
  - The `profile` feature with `tracing-tracy`, documented in `CLAUDE.md`.
  - **Verify:** Tracy shows the tick's phases.
  - **As built (#367):** `tracing-tracy` 0.11 with `only-localhost` and `ondemand`. It's held at 0.11 because that is tracy-client 0.18, the Flint client's, so both speak Tracy 0.14.0 and one viewer reads them. Each tick is a Tracy frame, and its work a `tick` frame set beside it, so the frame graph shows the work against the 100 ms. The spans are D0.1's `tick.<phase>` and `loop.<name>`, filtered by `RUST_LOG` like the log.

### D4: The town (`dt:D4-town`)
- **D4.1 The town reports in.**
  - The `TownReport` type, `POST /admin/town/report`, the scoped key, and `capitol-town` posting each game minute. The files stay as they are.
  - *As built (#368):*
    - **The type.** `capitol_protocol::metrics::TownReport`, sent each game minute and counted over the window since the one before, holds:
      - a `CitizenReport` per citizen on the roster: name, role, standing, phase, Strands, activity, restarts, its last failure, and its newest decisions (`CITIZEN_DECISIONS`, 5);
      - `layers`: each layer's `LayerReport` over the window, with times asked, outcomes, total and longest microseconds, cents over the window and cents since the town started;
      - `rulings`, by `By::label`;
      - the newest `failures` (`TOWN_FAILURES`, 20), and `failed`, the count over the window;
      - `started_ms`, which tells one run from the next, and `stopping` for the last word.
    - **Decisions are sent once.** A `DecisionBrief` holds the question, the menu as labels, what stood, by whom, and each layer's turn. A citizen's decisions go only in the first report after they're made, and the server keeps the last it was sent, so 600 away citizens cost nothing.
    - **The fixture** is `rest/town_report.json`.
    - **The key.** The `town` scope (`ApiKeyScope::Town`) reaches `POST /admin/town/report` and nothing else: no GET, no WebSocket. It needs no subscription. `POST /account/api-keys` refuses it. An admin mints it at `POST /admin/town/key` on their own account (`api_keys::mint_town_key`), which revokes the one before.
    - **The town's side.**
      - `capitol-town run` and `soak` read the key from `town.report-key` beside the roster (git-ignored). With `$CAPITOL_ADMIN_EMAIL` and `$CAPITOL_ADMIN_PASSWORD` set, as `town.sh` and `town.ps1` set them, they mint it when it's missing and again on a 401.
      - Each citizen's `Mind` notes its decisions in a `Pulse` (`decide/pulse.rs`), kept across its lives. A layer's cost is the change in its cascade's `spent()`.
      - A `Reporter` (`citizen/town.rs`) takes the pulses and the health lines each `TOWN_REPORT_MS` (5 s, a game minute), on a thread of its own with a 5 s timeout. A failure is printed once, until a report gets through again.
      - A report the server doesn't answer, or fails (5xx), goes again with the next (`TownReport::then`), so its window's counts aren't lost. One it refuses (4xx) is dropped.
    - **The server's side.**
      - `engine::town::Town` (`AppState::town`) keeps the newest report and merges each citizen's last decisions into it, for `GET /admin/town` (`TownLatest`, or `null`).
      - It adds each report to two windows. Each `Sample` carries the sample's window as `town: TownSummary`: reports, the standing now, rulings, layers and failures. The stored minute carries the minute's window, which goes back if the store fails.
      - Town up is written on a report from a new run, or the first since boot or since it went down. Town down is written on `stopping`, or when the sampler finds no report for `TOWN_QUIET_MS` (60 s).
      - Without the sampler (`METRICS_SAMPLE_SECS=0`) nothing marks a quiet town down.
- **D4.2 The town tab.**
  - The roster (sort, filter), who's on now, decision mix per layer over time, and cost per citizen-day.
- **D4.3 One citizen.**
  - Their last decisions (menu, each layer's verdict, outcome), Strands over time, and their dot on the map.

### D5: The client (`dt:D5-client`)
- **D5.1 `NetStats`** in capitol-core.
  - **Verify:** unit tests over a loopback and a bot run's report.
  - **As built (#371):** `Net::stats()` returns a `NetStats` from a meter the IO thread keeps (`net/stats.rs`). Each Ping carries its number, and the matching Pong gives the round trip, as the last one and an average. A first Ping now goes as soon as the session is live, so the round trip is known within moments, not after 15 s. Traffic is counted by message type each way: totals since the start, and per type over the last whole second (a malformed text frame counts as `malformed`, control frames as `ping` and `pong`). Reconnects are counted too. The prediction's side is `Motion::stats(now)` → `MotionStats`: corrections, the distance they moved the prediction (in total and the last one), the lag, the interpolation depth in seconds (below zero means extrapolating) and the tracks. The game adds it with `NetStats::with_motion`, since a bot has no `Motion`. `NetStats` prints as one line, and `capitol-bot` prints it after its script: `rtt 2 ms (avg 2 ms), in 12 msg/s 1.5 kB/s, out 0 msg/s 0.0 kB/s, 0 reconnects`.
- **D5.2 The overlay** on F7.
  - A frame-time graph, net, prediction, entities and interest.
  - **Verify:** a capture `--offline fixtures/terrain.json --screenshot-at 3` with it open.
  - **As built (#372):** `ui/dev_overlay.rs`, drawn in `CapitolExtension::ui` and toggled by the `dev_overlay` action (F7, `config/input.toml`), open at login and offline too. It's a card bottom left, over the controls hint, clear of the panels that open top left. It has two columns, so it fits 853×480 points: the frame, the world and interest on the left, the net and the prediction on the right. Each part opens with a plain sentence, as the console's charts will ("60 fps, with 3 hitches over 33 ms in 4 s; the worst took 51 ms."), then its numbers. The frame graph keeps the last 240 dts from `pre_update`, recorded open or not. Its bars are dim gold within budget, ember over it and red past 30 fps, over the 60 and 30 fps lines, with the top between 50 ms and 15 fps. The net comes from `Session::net_stats(now)`, `Net::stats()` with `Motion::stats(now)`. The world is the engine's entity count and the store's: who's in sight (citizens counted), nodes, stations and claimed lots. Interest is your cell and how far its edge is. That's `protocol::rules::interest_cell`, now shared with the server's `engine::interest::cell_at`. `--dev-sample` opens it offline on sample numbers, to see the online layout. It never takes a click, and it asks for its numbers only while it's drawn. First finding: with vsync on, dt comes in bursts (~4 ms, then ~50 ms) at a 60 fps mean, [#406](https://github.com/chrischaps/TheCapitol/issues/406).
- **D5.3 Admin in the client.**
  - The server's tick and watches.
  - World debug drawing through the Rhai `draw_*` calls: interest cells, node habitats, routes and citizens' targets.
  - Read-only tunables.
  - *As built (#373):*
    - **Who's an admin.** `auth_ok` carries `is_admin` from the token. It's left off the wire when false, so the fixture is unchanged. `GameState::is_admin` holds it. The admin routes check the token again whatever a client shows, and a 403 stops the client asking.
    - **The server's column.** For an admin, F7's card grows a third column, which still fits 853×480 points. It shows:
      - the tick, in a sentence, from the newest sample, with p95, the busiest phase, who's playing, the sockets and how long ago it was sampled;
      - the last four annotations, marked in red for a watch firing and in gold for a tunable changing;
      - the tunables off their defaults, read-only.
    - **Polled, not streamed.** `capitol_core::admin::Watch` asks `/admin/series` (whole samples, from the newest it has) and `/admin/annotations` every `SAMPLE_SECS`, and `/admin/tunables` every minute or when a `tunable_changed` annotation comes. The samples come every 10 s either way, and a poll is one more REST call on the IO thread, where `/admin/stream` would be a second long-lived connection. A projected series (`fields`) doesn't decode as a `Sample`, so the client asks for whole ones. `SeriesQuery` and `AnnotationsQuery` moved into the protocol.
    - **The world layers.** F6 (`debug_layer`), for an admin only, steps through four layers, then all, then off. They're drawn by `scripts/debug.rhai` from `cap_debug()` (`debug_layers.rs`):
      - interest: your cell of the interest grid and the eight round it;
      - nodes: each node within 600 u on a stalk, with its kind and its ground's height, slope and massif from `protocol::land`. The habitats aren't on the wire, so it can't say whether the ground suits ([#413](https://github.com/chrischaps/TheCapitol/issues/413)).
      - walks: the walk graph round you (`WalkGraph::edges_near`);
      - walkers: where the server is walking each player near you, and what they're doing.
    - **Citizens' targets.** The server already keeps every player's destination, so the walkers come from a new route, `GET /admin/walkers?x&y&radius` (`rest::Walker`, fixture `rest/admin_walkers.json`). It's polled once a second while the layer is on. For a citizen that's its next leg, not its goal, which waits on D4.1's report ([#414](https://github.com/chrischaps/TheCapitol/issues/414)).
    - **Not built.** The stream's warnings (tax unpaid and the like) don't reach the client: they're per player and belong in the console. The watches appear as soon as D2.5 writes `watch_fired` annotations.
- **D5.4 Engine hooks** (`engine-upstream`, each with an ADR):
  - (a) games register `DebugPanel`s
  - (b) GPU timestamps per pass
  - (c) a profiler feature in the frame loop
  - *As built (#374):* one Flint PR ([#83](https://github.com/chrischaps/Flint/pull/83), merged) with three ADRs.
    - **(a) ADR 0107.** `PlayerExtension::debug_panels()` names the game's panels, and `debug_panel_ui(name, ui)` draws one while it's open, after `ui`, so a panel reads the extension's own state with nothing shared through an `Arc`. They join the engine's after them: F3 toggles them, and they dock in the same columns.
    - **(b) ADR 0108.** `flint_render::GpuTimer` on `SceneRenderer::set_gpu_timing`: phases rather than every pass (`Grass`, `Shadows`, `Scene`, `Post-process`, `Composite` or `FXAA`, `World lines`, and the player's `UI`). Each timestamp is an empty compute pass, so only `TIMESTAMP_QUERY` is needed. The pass that writes the window is timed inside its own submission, because under vsync the GPU waits there for the swapchain image: a scope across it read 15 ms for 0.5 ms of work. The frame's figure is the scopes' total. The times are smoothed into `RenderStats::gpu_passes` and `gpu_frame_ms`, a frame or two late, never waiting. F2 turns it on and shows them, and `--exit-after --stats` prints `[perf] avg GPU`. D5.2's frame graph reads them from `ExtensionCtx::gpu`.
    - **(c) ADR 0109.** flint-player's `profile` feature is `profiling`'s Tracy backend, with `#[profiling::function]` on the frame loop's and the renderer's phases (wgpu's own scopes come with it) and a frame mark each redraw. The client listens on loopback only and keeps nothing until a viewer connects. capitol-client passes it on as `profile`. The server's D3.3 uses `tracing-tracy`, and the same viewer reads both.

### D6: Tuning and balance runs (`dt:D6-balance-runs`)
- **D6.1 Tunables.**
  - The first set from D5, persisted and validated, with an annotation per change. The client is told any value it shows.
  - Cut a ticket for any knob that can't be made live cheaply.
  - **As built (#375):** all seven knobs are live; none needed a ticket.
    - `capitol_protocol::tunables`: `Tunables`, and `KNOBS`, which declares each knob's key, unit, range, step, default and chart. Two knobs are families, one per id: `deed_price.<zone>` and `yield.<resource>`.
    - `Fees` is the part a client shows: the exchange fee, the listing fee (in percent, to 0.01%) and a stall's rent a day. The protocol's free `deposit_quote`, `withdraw_quote`, `listing_fee`, `buy_debit` and `rent` are gone, so every caller had to choose: the server's live `Fees`, or `Fees::default()` in a test. The constants stay, as the defaults.
    - The server's `engine::tunables::Live` is an `ArcSwap` on `AppState` and `GameState`. `update` checks every value before it sets any, writes the `tunables` table (only knobs off their defaults, so a default changed in code reaches every world) and one `tunable_changed` annotation per knob, in one transaction. `GET` and `PUT /admin/tunables` serve it. An out-of-range or unknown key is a 400 that changes nothing.
    - Where each knob is read:
      - the fees, where they're charged (the exchange, `market::place`, `bazaar::rent`);
      - the tax rate as each hourly run starts, which also sets the foreclosure threshold. At a rate of 0 nothing is charged or foreclosed. A lower rate lowers the threshold, so a plot in debt can foreclose sooner;
      - the deed multiplier wherever a deed's price is read (the claim, the plot listing, `/bureaucracy`). A plot is assessed at what was paid, so a change doesn't re-assess deeds already owned;
      - regrowth when a node is spent, so a node already spent keeps its time;
      - yield on each gather, rounded at random so that its mean is exact.
    - The client hears the fees on `auth_ok` (`fees`) and in `fees_changed`, which goes to every socket when a fee moves. `GameState::fees` feeds the exchange, market and shop panels' previews and labels, and the bots' and sim's budgets (`Holdings::fees`, the market makers' `quotes`). The frozen React panel still says 0.5%.
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
