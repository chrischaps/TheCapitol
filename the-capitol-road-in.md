# The Capitol: The Road In

*2026-10-22. Plan issue #437. Work packages #438 to #452. Issues are labelled `tutorial`, with phase labels `tu:O0-groundwork` to `tu:O3-home`.*

## 1. Vision

A new player lands on the south road among the farms, with the tower far ahead. Today they learn four things (walk, gather, look in the pack, twist rope), then they're on their own. Their first deed then comes from pulling grass for the better part of an hour, and when they have one they choose a lot with no idea what the choice means.

**The road in** turns that first hour into the city's welcome. The spawn and the hall are on the same road, about 3000 apart, and the road runs through every zone a player can live in. So the tutorial walks it:
1. **Arrival:** make things among the farms, from a handful of grass to a stone axe that fells a tree your hands couldn't.
2. **The road in:** walk up to the hall, stopping at a farm, a suburban street and an urban yard to see what living there is like.
3. **The hall:** the Exchange, the order book, the bazaar and the Gazette.
4. **People:** a word to whoever's near, a stranger's card, and a letter from the Registrar.
5. **The Deeds Office:** the zones side by side, a lot of your choosing, and **a deed for it, as the reward for the road**.
6. **Settle in:** a station on your own land, a job that runs, and what the city asks of you in tax.

The question the plan answers: **can a player's first choice of home be an informed one, and can the road to it be the game in miniature?** By the time you choose, you've made things, sold them, met people and walked the city end to end. The deed is the reward, and also the moment the city becomes yours.

## 2. Goals and non-goals

**Goals**
- Hands-on practice, in the live world, in every system a player needs before settling into a workshop: walking, gathering, tools and quality, crafting by hand and at a station, the Exchange, the order book, the bazaar, the Gazette, talk, letters, claiming, stations at home, automation and tax.
- Honest zone facts, so the choice of home follows play style: room, the walk to the market, what grows nearby, the price and the tax.
- One deed per account, of the player's choice, as the reward for finishing.
- Progress kept and judged on the server, because the reward is real.
- Citizens walk it too, and earn the same deed.

**Non-goals for this plan**
- An instanced tutorial area. It's one shared world, and the road in runs beside everyone else.
- NPC guides. The voice is the bureaucracy's, an institution, never a character.
- A deed you can trade or sell, or deed transfer in general (`the-capitol-economy.md`, later).
- Fixing the zones' imbalances (#417 urban spacing, #424 capacities, #292 rural price). The zone card tells the truth about them. It doesn't change them.
- The React and Love2D clients. Both are frozen.

## 3. Where things stand (audit, 2026-10-22)

**The tutorial** is Flint's P8.3 (#54):
- `capitol_core::onboarding` (`Step`: Walk, Gather, Inventory, Craft; `Did`; `Tour`, a `u8` with bits 0 to 3 and `0x80` for dismissed).
- `ui/hints.rs`: one card top right with step pips, a gold tick, "Skip the tips", and `place()` that steps round open panels (#99).
- Progress is in `save/hints.toml` on the player's machine (#71). The backend has nothing: no table, message or route.
- `Tour::judge` marks an account done if it holds any non-raw item.

**A deed** is a price, not an item (`the-capitol-milestones.md` M7):
- `zones.deed_cost` (rural 250, suburban 500, urban 1000, guild 5000) × the `deed_price.<zone>` tunable, charged as `DeedPurchase` by `routes/plots.rs::claim()` in one transaction.
- `assessed_value` is set to what was paid, and tax is 2% of it a week (`property_tax.rs`).
- Foreclosure after 4 weeks of debt refunds the deed less the debt. `plot_sync::RETIRED_LOTS` refunds in full.
- Claims have no range check and no per-player cap.

**The first deed's road today:**
- 100 Strands at registration (`ledger::STARTING_STRANDS`).
- Fiber into the Exchange at 1:1 less a floored 0.5%.
- Grass gives 1 to 3 a pull, so a farm is about 75 pulls and an urban yard about 450.

**Nothing compares the zones.**
- `RestRequest::DeedPrices` and `Zones` exist in `capitol-core/net/rest.rs`, but nothing sends them.
- The plot panel shows one lot's capacity. That's `station_capacity`, not what fits. Since the plot sizes (#420), an urban lot (520 of about 8.6k) and a suburban lot (about 1200 of about 10k) both fit 4 at 50 spacing, and a farm (about 380 of about 36k) fits about 15 (`the-capitol-plots.md` §5). Capacity reads 16 for the first two and 25 for a farm, and only the farm's is near what fits (#424).

**The geography:**
- The spawn is `rules::SPAWN_X/Y` on the south arterial at r3350, in the rural ring.
- The Exchange hall, the notice board and the bazaar are up the same road.
- At `PLAYER_SPEED` (50) the walk is about a minute.

**Parts to reuse:**
- Way-finding: `hud::bearing`, the hall's prompt and motes, the notice board's glint (`input.rhai`, `draw_hall`).
- Messages: toasts (`scripts/notices.rhai`) and queued notices for the absent (`engine/notices.rs`).
- Places and panels: the off-plot workbench (D10), the person's card (S0.3), talk (S1.1 to S1.3), letters (S1.5), automation (E4).
- Citizens: the `Decider` cascade, for teaching them the road.

## 4. The tutorial, chapter by chapter

Each step is in `protocol::tutorial`. Most are judged by the server (D7).

**4.1 Arrival** (#443), at the spawn:
1. Walk.
2. Gather grass by hand, then open your pack.
3. Twist rope.
4. *The quality card:* two ropes from fiber of two qualities, and the cascade in words.
5. Make a workbench kit and place it off-plot.
6. Gather branches and stone, then make a handle and knapped stone at the bench.
7. Make a stone axe and fell a tree.

**4.2 The road in** (#445): three stops on the south arterial, one each in the rural, suburban and urban rings. At each, Alt-click a marked lot and open the zone card (#444).

**4.3 The hall** (#446):
1. Deposit fiber.
2. Place a sell order and a buy order.
3. Collect from the market box. This is waived with words if nothing has filled (D10).
4. Look in at a bazaar stall.
5. Read the Gazette.

The Wanted page is an extra once S2.3 lands.

**4.4 People** (#447): say a line, open someone's card, read the Registrar's letter. Trading is taught, never required.

**4.5 The Deeds Office** (#448), by the hall:
- The comparison and the free lots, with "Show me".
- The claim with your grant, from the office or any lot's panel.

**4.6 Settle in** (#449):
1. A station on your plot.
2. An automation job.
3. The tax card.
4. "Well begun".

## 5. Design decisions

### D1. The bureaucracy's voice (no NPCs)
The game has no NPCs and no NPC faucets, and the plan keeps it so. The cards, the letter and the office speak as the Capitol's bureaucracy: plain, a little dry, kindly meant, never a character. The Deeds Office is a desk under an awning, not a clerk.

### D2. No permanent skip (Chris's choice)
The card can be hidden ("Hide for now"), and a header chip or the `?` overlay brings it back, but the tour can't be skipped for good. The deed comes only with the finish. Hiding is stored on the server (`dismissed`), so it holds on every device.

### D3. Everyone walks it (Chris's choice)
Every existing account is reset onto the new tutorial, those that already own plots included, and each can earn the one grant. The old `Tour::judge` shortcut goes. **Citizens walk it too** and earn the deed. Earning is the citizens' rule ("they earn everything, they get no grants"), and a deed for the road is earned. Existing citizens are reset with everyone.

### D4. A granted deed is assessed at the zone's price (Chris's choice)
A granted deed's `assessed_value` is the zone's live deed price, so it's taxed exactly as a bought deed. A free home doesn't mean free upkeep, and #335 (tax doesn't bite) shouldn't get worse.

### D5. Free land never becomes Strands (Chris's choice)
Foreclosing a granted plot refunds 0 Strands: the debt is written off against nothing. A granted plot on a lot that's retired (`plot_sync::RETIRED_LOTS`, an admin's forced commit) hands the grant back unused rather than paying out. The ledger counts grants as a faucet of land, outside the Strands' balance, and the Observatory shows them.

### D6. One per account, rural, suburban or urban, not for sale
The grant claims any free lot in the three home zones, never the guild district. It isn't an item and can't be traded, sold or given. Paying Strands for a deed still works as before, so a player can buy a second home and keep the grant, or buy first and use the grant later.

### D7. The server judges what it can see
Every step that does something in the world is judged by the server, at the point the action succeeds: a gather completed, a craft completed, an order placed, a line said, a plot viewed by zone. A few steps are only things on screen: opening the pack, opening crafting, opening the zone card. The client reports those (`tutorial_seen`), and the server takes them only for steps `protocol::tutorial` marks client-reported. The reward never hangs on them alone.

### D8. The claim needs no trip to the office
Claims have no range check today, and this plan keeps it so. The office is where the grant is explained and the zones compared, not a toll. Once the office has been visited, any lot's plot panel offers "Claim with your deed".

### D9. The zone card tells the truth
The card shows the stations a typical lot **fits** at `STATION_MIN_SPACING`, not its `station_capacity`. Today that's 4 for an urban or a suburban lot, against a capacity of 16 (#424). It also shows the live price and tax, the walk to the hall, and what grows nearby. Each zone's play style is in one line, from `the-capitol-plots.md` §1. If a zone is a poor choice for a workshop, the card says so, and the fix belongs to that zone's own issue.

### D10. The market never blocks the road
The tutorial runs in a live market that may be empty. A step that needs someone else (an order filling, a trade) is taught, then waived with words if it doesn't happen. No step costs more than a few Strands, and those are returnable (an order's escrow).

## 6. Phases and work packages

### O0: Groundwork (`tu:O0-groundwork`)

- **O0.1 Tutorial progress on the server** (#438, M):
  - `protocol::tutorial` (steps, chapters, which are client-reported);
  - `tutorial_progress`, seeded empty for every existing player;
  - the state on connect, `tutorial_step` and `tutorial_finished`;
  - `GET /tutorial` and `PUT /tutorial/dismissed`.

  *As built:*
  - **The steps.** `protocol::tutorial` has 30 steps in 6 chapters (`Step::ALL`, `Chapter::ALL`).
    - A step's number is its bit in `Steps` and is never reused; a new step takes the next free bit.
    - On the wire `Steps` is a list of names in the tour's order, and an unknown name is passed over, so a client a step behind still reads the rest.
    - `Step::judge` gives `Server`, `Client` or `Waivable`. Four steps are `Client`: `open_pack`, `read_zone_card`, `open_office` and `read_tax`. One is `Waivable`: `collect_box` (D10).
    - `TutorialProgress { done, dismissed }`, with `finished()` true once every step is done.
  - **The table.** `tutorial_progress` (`20261217_001_tutorial_progress.sql`) has `steps` (BIGINT bits), `dismissed`, `started_at` and `finished_at`.
    - The migration seeds an empty row for every player, and `create_account` makes the row for new accounts.
    - A player with no row (a test's) reads as a fresh tour.
  - **The store.** `engine::tutorial`:
    - `progress`.
    - `mark(player, step)`: one upsert. It returns the progress the first time and `None` after, and sets `finished_at` with the last step. #439 calls it.
    - `set_dismissed`, which tells every socket through `GameEvent::Tutorial`.
  - **On the wire.**
    - `tutorial { done, dismissed }` follows `auth_ok` on every socket.
    - `tutorial_step { step, done }` is defined for #439 to send.
    - `GET /tutorial` and `PUT /tutorial/dismissed { dismissed }`, tagged Tutorial in the OpenAPI spec.
  - **In Flint.** `GameState::tutorial()` keeps the tour, and `StateEvent::TutorialStep` is there for #441's tick.
  - **Differs from the issue:**
    - The state is a `tutorial` message after `auth_ok`, not a field on it, which would have touched every place that builds `auth_ok`.
    - **There's no `tutorial_finished`.** The `tutorial_step` whose `done` holds every step is the finish.
    - Nothing is "skipped", only `dismissed` (D2).
- **O0.2 The server judges each step** (#439, M): `engine::tutorial::saw`, one line where each action succeeds, and a test per step.

  *As built:*
  - **`saw(db, events, player, Did)`.** `Did` says what was done (`Gathered(item)`, `Crafted(recipe)`, `Placed { at_home }`, `Ordered(side)`, `SawPlot(zone)` and so on), and `Did::step` says which step it counts for, if any. Most things count for none: clay, iron, a Capitol line, a look at a guild plot. `saw` is one upsert (`mark`) when the deed counts, and it tells the player's sockets with `tutorial_step` the first time. A failure is logged, and the action stands either way.
  - **Where each is judged:**
    - The tick's writer judges gathers (by the item: fiber, sticks, stone, logs), crafts (by recipe) and walks. A walk is judged where it ends: `GameState` keeps where each walk began, and one that ends `tutorial::WALK_DISTANCE` (20) or more away is handed to the writer. The writer keeps a `Judged` cache, so each player's step costs one query a run, not one per gather.
    - `station_placement::place_station`: off a plot is `place_workbench` (only a workbench can stand off one), on your own plot is `place_at_home`.
    - `automation::configure`, which now takes the event sender, and `claim` in `routes::plots`.
    - The routes: the exchange deposit, `place_order`, `collect` (once something comes out), `get_shop`, the Gazette's two reads, `get_card` (someone else's, not your own), a letter's `read`, and `GET /plots/{id}` and `/plots/at` (a step for each home zone). The plot routes now need the caller's account, which their auth layer already gave them.
    - A local line, in the socket's `say` handler.
  - **The stall counts once you're at it**, after the range check and before the shop is looked up. So a bazaar with no stalls rented doesn't block the road (D10).
  - **The grant's hook.** `Steps::deed_earned()` is every chapter before the Deeds Office. `saw` logs the moment it turns true, and #440 opens the grant there.
  - **Tests.** Each step has a test through the route or engine function it's judged in. `state::for_tests` is an `AppState` for route tests, on the local Redis. Refusals are tested too: a deposit of nothing, an order from out of range, a stall from afar, a claim you can't afford, someone else's station, an empty market box, your own card, a letter you didn't receive, a failed pull. A local line is only tested by its `Did`, since the socket has no test harness ([#463](https://github.com/chrischaps/TheCapitol/issues/463)).
- **O0.3 The deed grant** (#440, M):
  - `deed_grants`, and `claim()` with `use_grant`;
  - the assessed value at the zone's price, foreclosure refunds 0, and a retired lot returns the grant;
  - the ledger and the Observatory.
- **O0.4 Flint follows the server's tour** (#441, M): `Tour` as a view, `hints.toml` and `judge` retired, and Skip becomes Hide with a way back.
- **O0.5 The chapter card and markers** (#442, M): the chapter card and its close card, the bearing arrow, and a ground ring at each step's target.

### O1: The road in (`tu:O1-road-in`)

- **O1.1 Arrival** (#443, M): chapter 1, the quality card, and spawn nodes for every kind the chapter needs.
- **O1.2 The zone card** (#444, M): `GET /bureaucracy/zone-facts`, `ui/zone_card.rs`, and the three-column comparison.
- **O1.3 The road in** (#445, S): a stop in each zone, chosen by rule so it survives regeneration.

### O2: The hall and the people (`tu:O2-hall`)

- **O2.1 The hall** (#446, S): the Exchange, both orders, the box (waivable), a stall and the Gazette.
- **O2.2 People** (#447, M): a line, a card, and the Registrar's letter. The letter needs a sender that isn't a player.

### O3: Home (`tu:O3-home`)

- **O3.1 The Deeds Office** (#448, L): the desk by the hall, `ui/deeds.rs`, free lots with "Show me", and the claim with the grant.
- **O3.2 Settle in** (#449, S): a station at home, an automation job, the tax card, and "Well begun".
- **O3.3 Citizens walk the road in** (#450, M): the steps as a `Decider` task, and the claim in the citizen's home zone. `sim/plan.rs` no longer saves for the first deed.
- **O3.4 Balance** (#451, M): `deed_grant.enabled`, balance runs with grants against none, and what the 100 starting Strands are for now.
- **O3.5 The docs** (#452, S): GDD §4, §8.4 and §8.7, the milestones, and P8.3 marked superseded.

## 7. Dependency graph and estimate

```
#438 O0.1 progress ─┬─ #439 O0.2 judging ─┬─────────────────────── #443 O1.1 arrival
                    │                     ├─────────────────────── #446 O2.1 hall
                    │                     └─────────────────────── #447 O2.2 people
                    ├─ #440 O0.3 grant ───┬─ #448 O3.1 office ── #449 O3.2 settle in ─┐
                    │                     └─ #450 O3.3 citizens ── #451 O3.4 balance ─┴─ #452 O3.5 docs
                    └─ #441 O0.4 Flint tour ── #442 O0.5 chapter card (frame for every chapter)
#444 O1.2 zone card (no dependency) ── #445 O1.3 the stops ── (and into #448)
```

- **First slice:** #438, then four in parallel: #439, #440, #441, and #444, which needs nothing.
- **Then:** #442. After that the chapters (#443, #445, #446, #447) can run side by side.
- **Last:** the office and settling in, the citizens, the balance runs, and the docs.
- **Size:** about 15 packages, 2 L-ish, 9 M and 4 S, roughly the size of the Commons' S1.

## 8. Later (outside this plan)

- Deed transfer and sale between players (GDD §8.4).
- A second chapter of the road in, outward: the wilds, the trails and a better tool (WD7's "the first trip out is a choice made with a tool in your hand").
- Apprenticeships (GDD's employment, "an onboarding path and interdependence"), once wages (S2.6) and the board (S2.3) are in.
- A deed price that follows the lot's area or room (`the-capitol-plots.md` PD3), which changes what the grant is worth by zone.

## 9. Risks

- **The free choice tilts the city.** If every new player picks the same zone, the grant spends that zone's free lots. #451 measures it. Shrinking a zone's lots (#420) makes it more likely.
- **The reset re-tutors veterans.** Every account walks the road again (D3). The card can be hidden, but the deed waits behind it. Experienced players will fly through it: most steps are things they do anyway.
- **An empty market.** Waived steps (D10) keep the road clear, but the lesson is weaker when nobody's there. Citizens on the road (#450) help.
- **The spawn's nodes.** A crowd of new players can strip the spawn's branches and outcrops. #443 checks counts for a crowd.
- **A faucet the books don't see.** D5 and #440's ledger work exist so the grant shows up in the books, not as drift.

## 10. Verification strategy

- **Per package:** each issue's tests. Every step judged on the server has one. The grant has claim, refusal, foreclosure and retired-lot tests, plus `conservation`.
- **End to end:** `capitol-bot --tutorial` walks the whole road on a fresh account against a town and claims with the grant. #450 then does it with citizens.
- **In the client:** a play-through on a fresh account (`flint-client\play.bat`), with captures of each chapter's card, the zone card at each stop, the Deeds Office and the claim.
- **The economy:** #451's balance runs, linked here.
- **Every land:** `scripts/ci.sh`. Packages with a migration (#438, #440) also run `ci.sh full`.
