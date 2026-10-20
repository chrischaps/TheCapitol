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
  - every position goes to every socket at 10 Hz (`engine/tick.rs`, `broadcast_positions`), which grows as N². Fixed in C0.4.

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
| C0.4 [#256](https://github.com/chrischaps/TheCapitol/issues/256) | **Positions by interest.** Measure first, then send nearby players only. *As built:* `engine/interest.rs` puts the players in a grid of `rules::INTEREST_CELL` (300) squares each tick. A socket is sent its player's cell and the eight round it: everyone within 300, nobody past 600 on either axis. A watcher sees round the avatar. Each cell's JSON is made once, lazily, outside the game lock, and each socket joins its nine. `player_joined` goes only to sockets near the joiner. `player_left` still goes to everyone, because it ends trades. Crossing in or out needs no message of its own: `positions` replaces the set, so capitol-core (and the React and Love2D clients) see the player come and go. In capitol-core, missing from `positions` is now *out of sight* (`lose_sight`), which drops the avatar but not a trade or request; only `player_left` ends those. Flint fades a player out at the edge of sight; nothing is drawn past it. *Measured* (in-process swarm, `interest::tests::positions_load`, release, one socket per player, 10 Hz; "sockets" is all sockets' serialisation per tick): 50 at the spawn: tick 1→5 µs, sockets 300→44 µs, 67 kB/s a socket either way. 50 across the city: sockets 289→26 µs, 67→2.8 kB/s. 200 at the spawn: tick 7→18 µs, sockets 4.7 ms→0.39 ms, 268 kB/s a socket either way. 200 across the city: tick 5→37 µs, sockets 4.5 ms→0.13 ms, 268→6 kB/s. A crowd in one place still sees itself whole. The activity events still go to every socket ([#275](https://github.com/chrischaps/TheCapitol/issues/275)). | M | C1.1 (for load) |
| C0.5 [#211](https://github.com/chrischaps/TheCapitol/issues/211) | **Walking round water.** A walk graph from `/terrain` (roads, bridges, fords, trails) and A*. *As built:* `capitol_core::paths::WalkGraph`. It has a node per road and trail vertex, long segments cut to 40, roads welded within 30, and open ground joining loose ends and stray networks within 600 (the rural `trail_*` lanes touch nothing). Every edge and leg passes `walkable`, the server's own ring, water and cliff test run along the line. A walk goes straight when it can, otherwise it joins the graph (straight, or over a small grid), runs A* by time, then pulls tight within 25 of the way. `Bot::walk_to` sends a `move` a leg, about 430 units a leg, well under the 60-a-minute limit. One graph per terrain version is shared by the process. The detours in `go_to_exchange` and the sim's trade are gone. | M | – |
| C0.6 [#247](https://github.com/chrischaps/TheCapitol/issues/247) | **Bots that stay up.** Reconnect, re-auth by key, rebuild state, claim presence again. *As built: `Net` already reconnected with capped, jittered backoff and resynced; `Bot` now notices the drop. Every call fails fast with `BotError::Offline` until `Bot::recover` has it back (presence, a position, the resync's refetches, its orders), so work in flight is dropped, never counted. `Bot::stay_up` reruns a loop through drops; the sim, its makers and `capitol-bot`'s wander, trade and market modes use it. A refused key is `BotError::Revoked`; `BotError::is_final` says when to retire a bot.* | M | – |

### C1: The town

| WP | What | Size | Depends |
|---|---|---|---|
| C1.1 [#248](https://github.com/chrischaps/TheCapitol/issues/248) | **`capitol-town`.** A supervisor over a roster, running the sim's role loops: the first citizens. *As built:* `capitol_core::citizen::town` and the `capitol-town` binary. The roster is `flint-client/town.toml` (git-ignored, with `town.status` and `keys/`; `town.example.toml` shows it): name, role slug (`Role::slug`, `gatherer-fiber` … `builder`) and `key` or `key_file`. `mint --count n` signs in an admin (`--admin`/`$CAPITOL_ADMIN_EMAIL`, password from `$CAPITOL_ADMIN_PASSWORD` only) by a socket-less `net::call_once`, so no avatar is taken, and appends each citizen the moment it's made. Roles continue the sim's mix; names come from `citizen::persona::name` (given name plus a trade surname, D5's naming, ahead of C1.2), and a taken name is passed over. Each citizen is a thread: `sim::run::live` plays the sim's loop with no deadline, a stop flag and a `Health`; it rides out drops inside the life (C0.6). A life that ends otherwise starts again after `net::backoff` (2 s doubling to 60 s); a refused key retires the citizen. Citizens trade with whatever is on the book, with no makers and no grants. Health (standing, role, Strands, activity, last failure) is printed and written to `town.status` every `--every` (60 s); `capitol-town status` prints it. Ctrl-C lets each finish its step within 20 s, leaving resting orders up; a second Ctrl-C exits at once. | M | C0.1, C0.6 |
| C1.2 [#250](https://github.com/chrischaps/TheCapitol/issues/250) | **Personas** and the day (D5). *As built:* `citizen::persona::Persona` (a temperament, a home, a market) and `citizen::schedule::Schedule`, all drawn from a SplitMix stream seeded by the citizen's name. `mint` writes them into `town.toml` as inline tables (`home = { zone, avenue, depth }`, `market = "hall"`, `schedule = { wake, market, home, bed }` on the game clock, `temperament = { thrift, risk, sociability, patience }`); any left out is drawn from the name again, so C1.1's rosters still read. **The day** is one life: the town signs a citizen in at `wake` (about 06:00, the whole day shifted ±40 game minutes and each moment ±15 more), it works, keeps its market hours (most in the afternoon from about 14:00, three in ten in the morning from 09:30, for 1.5 to 3 game hours by sociability) at the hall or the bazaar's gate (`Bot::go_to_bazaar`), trading what its role offers and otherwise lingering, then at `home` (about 19:00, as the evening light comes on) buys a lot if it has none and holds `Temperament::home_fund` (one to three deeds by thrift), walks home along the avenues, and logs out: the bot is dropped, so its avatar leaves the world. Until it has a lot, home is a spot on its avenue's crown, `depth` through its zone. The town sleeps it until `wake` by the server's clock as the last life saw it, so a citizen never signs in at night. Errands to the hall still happen at work hours when a step needs one. A builder's plot is claimed near its home, so it lives where it builds. The temperament is C1.3's `decide::Temperament`, the one type, and goes into each citizen's `Mind`, so the rules layer weighs all four traits; the persona also uses `thrift` for the home fund and `sociability` for market hours and the bazaar. A trade request (C1.5) is answered at market hours and on the walk home, and at the hall the market hours collect the market box and post demand, as free time there does. The town appends `town.presence.csv` (who's in what phase and standing) at each health report. | M | C1.1 |
| C1.3 [#251](https://github.com/chrischaps/TheCapitol/issues/251) | **The decision seam** (D4) with the rules layer. *As built:* `capitol_core::citizen::decide`. A `Situation` (who, with its `Temperament`; where; when; holdings; the watched books' bid, ask and last trade; the `Question`; the menu; and which entry means *carry on*) is serde, as are `Choice`, `Decision` (`pick` by index, plus `price` and `quantity` where the entry takes them, inside its `Span`s) and `Ruling` (the decision, `By` whom, and a trail of each layer's `Asked`, for C3.1's log). `Situation::admits` checks a decision against the menu, so an off-menu answer (a missing entry, a missing or extra parameter, one outside its span) is an escalation. A `Layer` is a `Decider` plus a threshold, a time budget and a spend budget, the spend summed over the layer's life for now (C3.2 can make it per day). A `Pending` layer is polled when the same question (`Question::key`) next comes up. Until then the cascade answers *carry on*. An answer within its budget is weighed against the menu as it is then. A late one goes straight to the fallback rather than on up, so a stack of slow layers never pends twice. **The rules layer** (`decide::rules`) is a utility pick: about 0.9 for what C1.1 would have done, less for the rest, with confidence the best's fit times its lead over the runner-up. A middling temperament reproduces C1.1 exactly (a test checks every role's work). Patience sizes the lots it sells and banks. Thrift weighs a price against the last trade, but only above the middle: weighed at middling thrift, a bid a Strand over a 1-Strand trade sent wood gatherers to the hall half as often again. Thrift also buys smaller lots. Risk asks up toward the ask, and sociability passes free time at the hall. **Measured** with the E2.7 sim (20 bots and 2 makers, 1 game hour at 10×, fresh accounts, one after the other on one database), C1.3 against master: logs 288 against 266, fiber 3309 against 3363, stone 247 against 243, ore 51 against 52, clay 133 against 144, bricks 62 against 74, deeds 10 each, trips to sell logs 9 against 11. That's within the runs' noise. The book carries over from run to run, so logs had fallen to 1 Strand by then (#210). **The decision points** are in `SimBot`'s loop, so the sim's bots decide the same way (`Mind::rules()`): work (`Role::errands`: banking, then each goal in order with its first step, stuck goals left off), sell (`Role::sell_menu`, from half of `SELL_AT`), buy (`Role::buy_menu`, up to two lots) and free time (rest, or walk to the hall). `Role::trade` is gone into those menus and the rules. The P2P trade question (accept, counter with more Strands, decline), valued at the citizen's books, has its types and rules but isn't asked yet: C1.5 asks it when citizens answer people (and changed when a counter is made; see its row). `town.toml` takes `layers` for the town and for each citizen, checked on load. Every citizen is middling until C1.2 (#250) gives it a temperament. | M | C1.1 |
| C1.4 [#252](https://github.com/chrischaps/TheCapitol/issues/252) | **Bot coverage.** The bazaar, automation, tool slots, splitting stacks. *As built:* the bazaar's seven routes and automation's three are `RestRequest`s; `capitol_core::bazaar` keeps the directory, the shops seen and yours (`GameState::bazaar`, kept by the sale and lapse notices too) and walks to a stall through the gate. `Bot` has `bazaar`, `my_stall`, `go_to_stall`, `browse`, `rent_stall`, `stock`, `unlist`, `buy`, `configure_automation`, `start_automation`, `stop_automation`, `install_tool`, `remove_tool` (WS), `split_stack` and `remove_station`. The live tests take their goods from grants (`DEV_GRANTS=on`), and a stall can't be given up early (#268). | M | – |
| C1.5 [#253](https://github.com/chrischaps/TheCapitol/issues/253) | **Citizens meet people.** Answering trades, buying from humans, bidding for goods, keeping stalls. *As built:* `capitol_core::citizen::meet` and `sim/run/people.rs`; only `capitol-town`'s citizens do it (`SimBot::people`), so the E2.7 sim is unchanged. **Trades.** A citizen's walks stop for a request (`Bot::answer_trades`, `BotError::Interrupted`), since the server drops one after 30 s and executes only within range; rests stop too, and the loop answers before its next step. With no chat, the citizen speaks through its side of the window. It lays out its wares (all it holds of its role's product, if its books give them a worth), waits for the other side to sit still for `TRADE_THINK`, and asks the cascade. For Strands: accept; counter, once, by taking goods off its side, Strands first, then units at their worth, rounded its way, one always left ("30 gets you three"); or decline and close the window. For goods: `Choice::Pay` their worth at its books over what it `wants`, in place of its wares, then accept; goods it can't value it declines. No barter. A window nobody offers into is closed after 60 s, any trade after 5 min. The rules' counter changed from C1.3's: it's made whenever it can close the gap (its span's top is all but one unit of the cheapest), not only from 0.6 to 1.0 of fair, because the window shows everything and a person wanting a few offers little; with the counter spent, a short offer is taken if it comes to about three-quarters of fair or more, and declined otherwise. **Demand.** `Role::needs` (from the recipes: planks need logs; ingots ore and charcoal; tools stone, planks and iron; a builder stone, logs, iron, planks and charcoal). When the next step would gather a need and nobody is selling it, the citizen may walk to the hall and leave a standing bid (`Choice::Bid`, at its books, lower for the thrifty, up to two lots it can afford with a deed kept back). At every visit to the hall it collects its market box, takes down bids a game hour old, and asks again about each need. It still buys from an ask, a person's or a citizen's, when the rules find the price fair. **Stalls.** With nobody bidding at the hall, a citizen holding half of `STALL_AT` (40) of what it makes or more is asked `Choice::Stock`: price at its books × 1.05 to 1.35 by risk, all it holds. It rents the first free stall for `STALL_DAYS` (2) as "Percy Sawyer's logs", pays more when under a day is left, and stocks it. **Measured** (20 fresh citizens on a database of their own, at 10×): the live trade test (`a_citizen_counters_a_short_offer_declines_a_lowball_and_takes_a_fair_one`) passes. The second acceptance check doesn't, in a fresh town: a person's ten logs at 5 went unfilled for a game hour, because the town's Strands pool with its fiber gatherers, who need nothing, while the makers spend theirs on deeds ([#294](https://github.com/chrischaps/TheCapitol/issues/294)). With 600 Strands put in the ten broke makers' hands by hand (a test lever on that database, not a grant), the same order filled in one game minute, and a maker's standing bid filled from a citizen. `a_persons_sell_order_fills_in_a_town_within_a_game_hour` is the check, for C1.7. | M | C1.3, C1.4 |
| C1.6 [#254](https://github.com/chrischaps/TheCapitol/issues/254) | **Merchants earn their stock.** No grants in the town. | S | C1.3 |
| C1.7 [#257](https://github.com/chrischaps/TheCapitol/issues/257) | **The town's ledger and a soak.** 200 citizens for 24 game hours, conservation, load, and a report for #210. | M | C1.5, C1.6 |

### C2: Life you can see

| WP | What | Size | Depends |
|---|---|---|---|
| C2.1 [#258](https://github.com/chrischaps/TheCapitol/issues/258) | **Idle life.** The hall steps, the parks, the bridges at dusk, their own stall. | S | C1.2, C1.3 |
| C2.2 [#259](https://github.com/chrischaps/TheCapitol/issues/259) | **The Gazette names citizens** and counts them apart from people. *As built:* in the prose a citizen is "Citizen Cy" (unnamed, "a citizen who keeps out of the papers"); in the lists they carry `citizen` (producers) and `seller_citizen`/`maker_citizen` (the finest sale), and Flint draws the HUD's lozenge before the name, in ink, with a key. Each edition now lists what the city made most of (`made`, top six, with `by_people`). On a day a citizen made, sold or claimed anything it also carries `shares` (`GazetteShares`: units made, Exchange Strands to sellers and deeds claimed, each whole and people's part), and the markets, deeds and work stories each end with the split ("Of the 6,300 Strands, 5,100 went to people and 1,200 to citizens"). On a day without citizens the edition reads as before. Bazaar sales aren't in the Gazette at all yet (#240), so not in the split either. | S | C0.1 |

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
- **Human hours** ([#276](https://github.com/chrischaps/TheCapitol/issues/276)). C1.2's citizens play every game day. People play a few real hours, some days. The idea: a habit on the real clock beside the game-clock schedule, and a pool big enough that some are always on. Chris leans that way; it waits on his call about game night inside a session.
- **Distillation.** New rules learned from the decision log.
- **Guilds and contracts** (M9), which citizens join and keep.
- **The Capitol's public works** (GDD's infrastructure bounties and price floors), if the soak shows demand is too thin.
- **M10 operations.** A hosted server, so the town runs 24/7 off Chris's PC.

## 9. Risks

| Risk | Mitigation |
|---|---|
| Citizens crowd humans out of the scarce nodes (E2.7: nine rural veins) | Size the roster with the soak. Personas spread over zones. Citizens of a trade back off when a human is working the same node. |
| The economy can't support citizens without grants | That is a finding for #210, not a reason to add a faucet. |
| Load: N² positions, one thread per citizen | C0.4 sends positions by interest (its numbers are in its row). The activity events are next (#275). The soak's numbers. Async citizens if threads bite. |
| Model layers cost too much or answer too slowly (Jev's figures are vendor claims; it is in early access) | Menu-only answers, `Pending` plus fallback, per-layer budgets, replay before going live. |
| Citizens feel robotic | Personas, schedules, idle life (C2.1). Later, chat. |

## 10. Verification strategy

- `scripts/ci.sh` throughout. Protocol fixtures round-trip byte for byte (C0.1). Unit tests in capitol-core for the walk graph, the cascade, personas and schedules.
- End to end:
  - a backend on a database of its own (`capitol_sim`, as in the E2.7 note);
  - `capitol-town` with 20 citizens;
  - the Flint client: marked lanterns, citizens walking round the water to the hall, a citizen answering your trade, and lanterns going home at dusk.
- The soak (C1.7): 200 citizens for 24 game hours. Tick time stays within budget, no socket is dropped, and Strands and units are conserved.
