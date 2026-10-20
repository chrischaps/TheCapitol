# The Capitol: Citizens Plan

*2026-10-20. Issues labelled `citizens`, #246 to #262 (plus #211 as C0.5). Phase labels `cz:C0-groundwork` to `cz:C3-cascade`.*

## 1. Vision

The Capitol's economy is the game, and an economy needs people in it. Until there are enough humans, and on any quiet night after that, **citizens** fill the world. They are AI players who live in the city:
- they gather, craft, build, trade and keep stalls;
- they walk home along the avenues at dusk with their lanterns lit;
- they answer when a lone human asks to trade.

They do two jobs at once:
1. They **stress-test the economy** with real, continuous supply and demand, so its imbalances show up as prices, as E3.8's first run showed.
2. They make the world **feel inhabited** for one player or a handful.

Citizens play the same game humans do, by the same rules and for the same money. Nobody is lied to about who they are.

## 2. Goals and non-goals

**Goals**
- A long-running process keeps a roster of citizens in the world. It survives restarts and needs nobody watching it.
- Citizens have trades, homes, temperaments and a day, so the city has rhythms.
- Citizens deal with humans: P2P trades, buying from them, bidding for what they make, and stocking stalls.
- One decision interface. Any stack of deciders sits behind it, from pure rules to pure models.
- A soak test at 200 citizens that checks load and conservation.

**Non-goals for this plan**
- Chat and the citizens' "voice": they need M9's chat first (§8).
- LLM layers in production. C3.2 is a spike on replayed logs.
- Server-side NPCs (see D1).
- Combat or anything else outside the economy.

## 3. Where things stand (audit, 2026-10-20)

- **Economy v1 is done** (#144 to #179), as are the wilds (W0 to W11) and the Flint client (P0 to P8). The economy has only been played by bots, in batch runs.
- **`capitol-bot`** (`flint-client/crates/capitol-core/src/bin/capitol-bot.rs`) has these modes: a scripted session, wander, gather, trade partner, market maker (`--market`) and the E2.7 sim swarm (`--sim`).
- **`Bot`** (`bot.rs`) can walk, gather, craft, place stations, claim plots, use the exchange and market, and trade P2P.
- **The sim:**
  - `sim/plan.rs::next(catalog, holdings, goal)` is a pure one-step planner;
  - `sim/mod.rs::Role` holds the goals and trade rules;
  - `sim/report.rs` writes the reports.
- **Missing for citizens:**
  - pathing round water (#211, done in C0.5);
  - any long-running mode: no supervisor, no reconnect, and `AuthFailed` ends a bot;
  - personas;
  - helpers for the bazaar, automation and tool slots;
  - a way to tell a citizen from a human, since `PlayerPosition` is `{id, name, x, y}`.
- **The market makers break the GDD's rule.** They are funded by `POST /player/grant` behind `DEV_GRANTS`. The GDD says there are no NPC faucets (GDD §5, §8) and the Capitol never trades goods.
- **At scale:**
  - a socket that falls 1024 events behind the broadcast is hung up on (`ws/connection.rs`, `Lagged` treated as `Closed`). Fixed in C0.3;
  - every position goes to every socket at 10 Hz (`engine/tick.rs`, `broadcast_positions`), which grows as N².

## 4. Design decisions

All decided with Chris on 2026-10-20.

### D1. Where citizens run

**In a separate process, `capitol-town`, built on capitol-core's `Bot`.** Citizens use the protocol any client uses, with no server back door. This keeps the protocol-first pillar honest. It reuses the planner, the market maker and the reports, and works today.

Running agents inside the server would mean first moving most of the WebSocket handler's validation and database work into the engine. Only 11 of its ~30 messages become a `Command`. That cost buys nothing citizens need.

- Each citizen is one account with one API key. Keys skip `/auth`, so the auth rate limit (10 a minute per IP) doesn't bite. The per-connection command limit (60 a minute) is plenty.
- Under D9 (E0.8), a citizen's key holds the avatar the way any bot does.

### D2. Citizens earn everything

A citizen starts as a new human does and earns every Strand by labour. The town never calls `/player/grant`. A merchant builds its float by making and selling before it makes a market (C1.6). This keeps the stress test honest: if the economy can't support its citizens, that is a finding, not something to paper over. The soak checks conservation (C1.7).

### D3. The mark

Citizens are marked, honestly and quietly.
- `accounts.kind` is `human` or `citizen`. Only the admin endpoint can make a citizen.
- `citizen: bool` is sent wherever a player is named to others.
- In Flint, a citizen's lantern is cooler and steadier than a human's amber, with a small glyph beside the name.
- The Gazette names citizens and counts them apart from people.

### D4. The decision cascade

**It's feasible, and C1.3 builds the seam now.** A citizen's brain is a stack of *layers*, any of which can be dropped in or taken out:

```
rules  →  Jev  →  small model  →  large model
```

Rules alone, a model alone, or any stack in between all work behind the same interface.

**Decisions are strategic and use a closed menu.** Layers never steer the avatar step by step: walking, gathering and crafting are carried out by `Bot` and the planner. At a *decision point*, the citizen loop builds a `Situation` and the menu of legal `Choice`s from game state. The decision points are:
- what to work on next;
- what to sell, and at what price;
- what to buy;
- whether to accept, counter or decline a trade;
- where to spend free time.

A layer picks one choice from the menu and fills in its parameters (a price, a quantity), or it escalates. A model layer gets the menu as a tool schema that admits only the listed choices. An off-menu answer is treated as an escalation, so no layer can make a citizen do something illegal or invented.

**The interface:**

```rust
trait Decider {
    fn decide(&mut self, s: &Situation) -> Verdict;
}
enum Verdict {
    Decided { choice: Decision, confidence: f32 },
    Escalate { reason: String },
    Pending(Ticket),            // an async layer will answer later
}
struct Cascade { layers: Vec<Layer> }   // Layer = Decider + threshold + time and spend budgets
```

- **Order.** The cascade asks each layer in turn. A `Decided` below that layer's confidence threshold, an `Escalate`, a timeout or an exhausted budget passes the question to the next layer.
- **Fallback.** The last layer must answer. If it doesn't, the fallback is the best choice any layer offered, then the rules layer's default, then "carry on with what you were doing". A citizen is never stuck waiting on a model.
- **Async.** A slow layer (a network model) answers `Pending`. The citizen keeps doing its current task and acts on the answer when it arrives, or on the fallback if the answer is late. The world never waits on a model.
- **Transport-neutral.** `Situation`, `Choice` and `Decision` are serde types, so a layer can be a function, a subprocess or an HTTP call.
- **Configuration.** In `town.toml`, set `layers = ["rules"]` (and later `["rules", "jev", "claude-small", "claude-large"]`) for the whole town or per citizen.

**The log** (C3.1) is what makes the cascade grow safely. Every decision is recorded as JSONL: the situation, the menu, each layer's verdict, the decision, and its outcome once known. `capitol-town replay` runs any stack of layers over a log offline. A new layer is therefore measured (agreement, escalation rate, latency, cost) before it ever runs live. The log also allows **distillation**: situations where the model layers keep answering the same way become new rules, so the expensive layers are asked less over time.

**The fast tier: Jev.** Between rules and the language models sits **Jev** (TypeSafe AI, early access since 2026-09-15), a non-generative "System One" decision model. A caller defines the allowed answers, and Jev answers one of three question types:
- `choice`: pick from a supplied list;
- `score`: a value on a defined scale;
- `null`: the probability that a statement is true.

It returns probabilities, not text. Its claimed latency is 70 to 500 ms, at about 4.2 cents per million input tokens with output free; these are vendor figures, and C3.3 measures them. It fits D4's shape exactly:
- the menu is a `choice` question;
- prices and quantities come from a `score` over the menu's allowed range;
- its probabilities are the layer's confidence;
- it can't answer off the menu.

Reviewers say it is weak on decisions that need broad context or nuanced judgement. That is what the model tiers above it are for, and low confidence escalates to them. So the expected shape is: rules settle the obvious cases, Jev the close calls among a few clear choices, and an LLM the rare open-ended ones. The seam doesn't depend on Jev. Any tier is one more `Decider`, and Jev's availability or price changing costs nothing but a line in `town.toml`.

### D5. Personas and the day

Each citizen in `town.toml` has:
- a name from a seeded, period-flavoured generator (given name plus a trade surname);
- a trade (a `Role`);
- a home: a lot once it can afford one;
- a temperament: `thrift`, `risk`, `sociability` and `patience`, each 0 to 1, which weight the rules layer and colour a model layer's prompt;
- a schedule on the server's time of day.

They work in daylight, go to the hall or the bazaar during their market hours, walk home at dusk and log out at night. Schedules are spread out, so the city has a rush hour and quiet streets rather than a synchronised flock.

## 5. Architecture notes

- **`capitol-town`** is a capitol-core binary next to `capitol-bot`. It has one thread per citizen to start with, like `capitol-bot`; move to async if the soak says so.
- New code goes in `capitol_core::citizen` (`decide`, `persona`, `schedule`, `town`).
- The roster is `town.toml`, which is git-ignored because it holds keys. `town.example.toml` is checked in.
- Backend changes are small: `accounts.kind`, the admin mint, the `citizen` flag, the `Lagged` fix, and positions by interest.

## 6. Phases and work packages

### C0: Groundwork

| WP | What | Size | Depends |
|---|---|---|---|
| C0.1 [#246](https://github.com/chrischaps/TheCapitol/issues/246) | **Citizen accounts.** `accounts.kind`, `POST /admin/citizens` (an account, its player and a `play`+`trade` key), and `citizen` on `PlayerPosition` and `PlayerJoined`. *As built:* `backend/src/citizens.rs` mints and lists; a trigger refuses any change of `kind`; a citizen's name must be one no player has (any case); it has a random password nobody knows, so its key is the only way in. The flag is sent only when true (`skip_serializing_if`), so `positions` grows only by its citizens. Besides positions it's on `trade_requested` (`from_citizen`), `trade_started` (`partner_citizen`), `bazaar_sale` (`buyer_citizen`), shop signs and shops and plots (`owner_citizen`), and `/player/me` (`citizen`). capitol-core keeps it on its players, `Trade::Incoming` and `OpenTrade`. Makers' marks on goods aren't flagged (#242 decides how they show). | M | – |
| C0.2 [#249](https://github.com/chrischaps/TheCapitol/issues/249) | **Flint: the citizen's mark.** A cooler lantern and a glyph by the name. *As built:* a citizen wears the same green as anyone else; only the light differs. A person's lantern is amber (`#ffa94a`) and gutters a little (`entities::flicker`, through `material.emissive_scale`). A citizen's burns just as strong, but pale and a little toward the moon (`#d6e4ea`, `CITIZEN_LANTERN`), and never moves. A slender lozenge in the same colour stands before the name (`labels::place` centres the two together and clashes them as one box; `hud.rhai` draws it). Your gold ring is now an entity of its own, so your own candle's flicker leaves it be. `--citizen-sample` stands two people, two citizens and you in a row for captures. | S | C0.1 |
| C0.3 [#255](https://github.com/chrischaps/TheCapitol/issues/255) | **Bug: a lagging socket is hung up on.** Skip ahead and resync instead. *As built:* the channel holds 4096 events (`GameState::EVENT_CAPACITY`, about 10 s of slack with 200 citizens at work). A lagging socket skips ahead and is sent `resync { missed }` (a new `ServerMessage`), then fresh `positions`, its `currency_changed`, a full `market_depth` per watched item and its waiting notices. If it holds the avatar, its trade and trade requests are cancelled first (`GameState::socket_lagged`), so nobody accepts an offer they never saw. capitol-core treats `resync` as a reconnect's resync. | S | – |
| C0.4 [#256](https://github.com/chrischaps/TheCapitol/issues/256) | **Positions by interest.** Measure first, then send nearby players only. | M | C1.1 (for load) |
| C0.5 [#211](https://github.com/chrischaps/TheCapitol/issues/211) | **Walking round water.** A walk graph from `/terrain` (roads, bridges, fords, trails) and A*. *As built:* `capitol_core::paths::WalkGraph`. It has a node per road and trail vertex, long segments cut to 40, roads welded within 30, and open ground joining loose ends and stray networks within 600 (the rural `trail_*` lanes touch nothing). Every edge and leg passes `walkable`, the server's own ring, water and cliff test run along the line. A walk goes straight when it can, otherwise it joins the graph (straight, or over a small grid), runs A* by time, then pulls tight within 25 of the way. `Bot::walk_to` sends a `move` a leg, about 430 units a leg, well under the 60-a-minute limit. One graph per terrain version is shared by the process. The detours in `go_to_exchange` and the sim's trade are gone. | M | – |
| C0.6 [#247](https://github.com/chrischaps/TheCapitol/issues/247) | **Bots that stay up.** Reconnect, re-auth by key, rebuild state, claim presence again. *As built: `Net` already reconnected with capped, jittered backoff and resynced; `Bot` now notices the drop. Every call fails fast with `BotError::Offline` until `Bot::recover` has it back (presence, a position, the resync's refetches, its orders), so work in flight is dropped, never counted. `Bot::stay_up` reruns a loop through drops; the sim, its makers and `capitol-bot`'s wander, trade and market modes use it. A refused key is `BotError::Revoked`; `BotError::is_final` says when to retire a bot.* | M | – |

### C1: The town

| WP | What | Size | Depends |
|---|---|---|---|
| C1.1 [#248](https://github.com/chrischaps/TheCapitol/issues/248) | **`capitol-town`.** A supervisor over a roster, running the sim's role loops: the first citizens. | M | C0.1, C0.6 |
| C1.2 [#250](https://github.com/chrischaps/TheCapitol/issues/250) | **Personas** and the day (D5). | M | C1.1 |
| C1.3 [#251](https://github.com/chrischaps/TheCapitol/issues/251) | **The decision seam** (D4) with the rules layer. | M | C1.1 |
| C1.4 [#252](https://github.com/chrischaps/TheCapitol/issues/252) | **Bot coverage.** The bazaar, automation, tool slots, splitting stacks. *As built:* the bazaar's seven routes and automation's three are `RestRequest`s; `capitol_core::bazaar` keeps the directory, the shops seen and yours (`GameState::bazaar`, kept by the sale and lapse notices too) and walks to a stall through the gate. `Bot` has `bazaar`, `my_stall`, `go_to_stall`, `browse`, `rent_stall`, `stock`, `unlist`, `buy`, `configure_automation`, `start_automation`, `stop_automation`, `install_tool`, `remove_tool` (WS), `split_stack` and `remove_station`. The live tests take their goods from grants (`DEV_GRANTS=on`), and a stall can't be given up early (#268). | M | – |
| C1.5 [#253](https://github.com/chrischaps/TheCapitol/issues/253) | **Citizens meet people.** Answering trades, buying from humans, bidding for goods, keeping stalls. | M | C1.3, C1.4 |
| C1.6 [#254](https://github.com/chrischaps/TheCapitol/issues/254) | **Merchants earn their stock.** No grants in the town. | S | C1.3 |
| C1.7 [#257](https://github.com/chrischaps/TheCapitol/issues/257) | **The town's ledger and a soak.** 200 citizens for 24 game hours, conservation, load, and a report for #210. | M | C1.5, C1.6 |

### C2: Life you can see

| WP | What | Size | Depends |
|---|---|---|---|
| C2.1 [#258](https://github.com/chrischaps/TheCapitol/issues/258) | **Idle life.** The hall steps, the parks, the bridges at dusk, their own stall. | S | C1.2, C1.3 |
| C2.2 [#259](https://github.com/chrischaps/TheCapitol/issues/259) | **The Gazette names citizens** and counts them apart from people. | S | C0.1 |

### C3: The cascade grows

| WP | What | Size | Depends |
|---|---|---|---|
| C3.1 [#260](https://github.com/chrischaps/TheCapitol/issues/260) | **The decision log and replay.** | M | C1.3 |
| C3.2 [#261](https://github.com/chrischaps/TheCapitol/issues/261) | **Spike: a Claude layer.** Menu-only answers, budgets, cost per citizen-day on replayed logs. *Needs Chris for the spend.* | M | C3.1 |
| C3.3 [#262](https://github.com/chrischaps/TheCapitol/issues/262) | **Spike: Jev as the fast tier.** Menus as `choice` questions, prices as `score`, probabilities as confidence. Calibration, escalation rate, latency and cost on replayed logs, for `rules`, `rules+jev` and `rules+jev+claude-small`. *Needs Chris for access and spend.* | M | C3.1 |

## 7. Dependency graph and estimate

```
C0.1 ─┬─> C1.1 ─┬─> C1.2 ─┬─> C2.1
C0.6 ─┘         ├─> C1.3 ─┼─> C1.5 ─┐
C0.5 (#211) ····┘         ├─> C1.6 ─┼─> C1.7
                          └─> C3.1 ─> C3.2, C3.3
C1.4 ───────────────────────> C1.5
C0.1 ─> C0.2, C2.2      C1.1 ─> C0.4      C0.3 (any time)
```

The first citizens walk after C0.1, C0.6 and C1.1, about three sessions. The whole plan is about 18 to 26 agent sessions.

## 8. Later (outside this plan)

- **Chat and the citizens' voice.** M9's chat (local and global) first. Then a model layer can give citizens lines: greetings, haggling, letters to the Gazette, all under the same budget as D4.
- **Distillation.** New rules learned from the decision log.
- **Guilds and contracts** (M9), which citizens join and keep.
- **The Capitol's public works** (GDD's infrastructure bounties and price floors), if the soak shows demand is too thin.
- **M10 operations.** A hosted server, so the town runs 24/7 off Chris's PC.

## 9. Risks

| Risk | Mitigation |
|---|---|
| Citizens crowd humans out of the scarce nodes (E2.7: nine rural veins) | Size the roster with the soak. Personas spread over zones. Citizens of a trade back off when a human is working the same node. |
| The economy can't support citizens without grants | That is a finding for #210, not a reason to add a faucet. |
| Load: N² positions, one thread per citizen | C0.4 and the soak's numbers. Async citizens if threads bite. |
| Model layers cost too much or answer too slowly (Jev's figures are vendor claims; it is in early access) | Menu-only answers, `Pending` plus fallback, per-layer budgets, replay before going live. |
| Citizens feel robotic | Personas, schedules, idle life (C2.1). Later, chat. |

## 10. Verification strategy

- `scripts/ci.sh` throughout. Protocol fixtures round-trip byte for byte (C0.1). Unit tests in capitol-core for the walk graph, the cascade, personas and schedules.
- End to end:
  - a backend on a database of its own (`capitol_sim`, as in the E2.7 note);
  - `capitol-town` with 20 citizens;
  - the Flint client: marked lanterns, citizens walking round the water to the hall, a citizen answering your trade, and lanterns going home at dusk.
- The soak (C1.7): 200 citizens for 24 game hours. Tick time stays within budget, no socket is dropped, and Strands and units are conserved.
