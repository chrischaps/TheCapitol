# The Capitol: The Road In

*2026-10-22. Plan issue #437. Work packages #438 to #452. Issues are labelled `tutorial`, with phase labels `tu:O0-groundwork` to `tu:O3-home`.*

## 1. Vision

A new player lands on the south road among the farms, with the tower far ahead. Today they learn four things (walk, gather, look in the pack, twist rope), then they're on their own. Their first deed then comes from pulling grass for the better part of an hour, and when they have one they choose a lot with no idea what the choice means.

**The road in** turns that first hour into the city's welcome. The spawn and the hall are on the same road, about 3500 apart (about 3000 before Plots v2's flip, #459), and the road runs through every zone a player can live in. So the tutorial walks it:
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
- `zones.deed_cost` (500 in each home zone since #292, which was rural 250, suburban 500 and urban 1000; guild 5000) × the `deed_price.<zone>` tunable, charged as `DeedPurchase` by `routes/plots.rs::claim()` in one transaction.
- `assessed_value` is set to what was paid, and tax is 2% of it a week (`property_tax.rs`).
- Foreclosure after 4 weeks of debt refunds the deed less the debt. `plot_sync::RETIRED_LOTS` refunds in full.
- Claims have no range check and no per-player cap.

**The first deed's road today:**
- 100 Strands at registration (`ledger::STARTING_STRANDS`).
- Fiber into the Exchange at 1:1 less a floored 0.5%.
- Grass gives 1 to 3 a pull, so a farm is about 75 pulls and an urban yard about 450.

**Nothing compares the zones.**
- `RestRequest::DeedPrices` and `Zones` exist in `capitol-core/net/rest.rs`, but nothing sends them.
- The plot panel shows one lot's capacity. That's `station_capacity`, not what fits. Since Plots v2 (#459, `the-capitol-plots-v2.md`), an urban lot (about 1000 of about 8.6k) fits 4 at 50 spacing, a suburban lot (about 980 of about 18.6k) 6 to 8, a whole workshop, and a farm (about 900 of about 35k) 12 or more. Capacity reads 16 for the urban lots and 25 for the others (`size_category`, PV4), and none is what fits until #473 sets it from the room indoors and out (#424, decided). Before the flip the plot sizes (#420) had 520 urban and about 1200 suburban lots fitting 4, and about 380 farms fitting about 15.

**The geography:**
- The spawn is `rules::SPAWN_X/Y` on the south arterial at r3950, 350 past the suburb, in the rural ring (r3350 before Plots v2's flip, #459).
- The Exchange hall, the notice board and the bazaar are up the same road.
- At `PLAYER_SPEED` (50) the walk is about 70 seconds (about a minute before the flip): from r3950 in to the hall by the trade ring, about 3500. The rings it crosses are wider too: about 350 of farms, 1440 of suburb and 1160 of urban grid.

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

**4.2 The road in** (#445): three stops on the south arterial, one each in the rural, suburban and urban rings. At each, Alt-click a marked lot and open the zone card (#444). Each stop's lot is chosen by rule (`protocol::tutorial::stops`), so a regenerated world moves it with its lots.

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
The card shows the stations a typical lot **fits** at `STATION_MIN_SPACING`, not its `station_capacity`. In the default world since Plots v2 (#459) that's 4 for an urban lot against a capacity of 16, 3 to 8 for a suburban one and 12 to 15 for a farm, both against 25 (#424). The card says each gap in words. It also shows the live price and tax, the walk to the hall, and what grows nearby. Each zone's play style is in one line, from `the-capitol-plots.md` §1. If a zone is a poor choice for a workshop, the card says so, and the fix belongs to that zone's own issue.

### D10. The market never blocks the road
The tutorial runs in a live market that may be empty. A step that needs someone else (an order filling, a trade) is taught, then waived with words if it doesn't happen. No step costs more than a few Strands, and those are returnable (an order's escrow).

**One step waits for a person: opening someone's card (`open_card`, Chris's choice, #484).** It isn't waivable. A server is expected always to have players or citizens about, so a world with nobody else in it should be rare, and then the player waits until someone comes back. A dev world with one human runs the town's citizens or a second client. The market waits for no one. A person, if it comes to it, is worth waiting for.

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

  *As built:*
  - **The table.** `deed_grants` (`20261221_001_deed_grants.sql`) is keyed by `account_id`, so it's one an account, with `player_id`, `opened_at`, `used_at`, `plot_id` and `returned_at`. `plots.granted` marks a plot claimed with one, and a release clears it. `deed_events` takes the kind `granted`, which the Gazette's land column counts as a claim.
  - **`engine::deed_grants`.**
    - `open`: `tutorial::saw` calls it the moment `deed_earned()` turns true.
    - `take_in`: locks the grant in the claim's transaction. A claim that finds none opens it from `tutorial_progress` when the steps say it's earned, so a failed open never costs the player their deed.
    - `use_in` and `hand_back_in`, and `books` for the ledger.
  - **The claim.** `POST /plots/{id}/claim` takes an optional body, `ClaimPlotRequest { use_grant }`. With no body it pays in Strands, so every client works as before.
    - With the grant, in the same transaction: the zone must be rural, suburban or urban; the grant is locked, checked and spent; nothing is charged; the plot is assessed at the zone's live price (`zones.deed_cost` × `deed_price.<zone>`); and the deed event is `granted`.
    - It's refused with words: not earned yet, already used, or not a home zone.
    - No balance is announced, since none changed.
  - **Foreclosure** refunds 0 on a granted plot, and its grant stays spent. **`plot_sync`** hands a granted plot's grant back on a retired lot or a forced commit (`SyncReport::grants_returned`), and refunds no Strands for it.
  - **The books.** No `currency_transactions` row is written: that table is the Strands' audit, and no Strand moves. So there's no `DeedGrant` transaction type, and `every_type_is_one_the_ledger_knows` didn't change.
    - Land has books of its own: `Ledger::grants` and `Sample::grants` (`protocol::ledger::DeedGrants`). They hold opened, used and returned, and `granted_value`, the assessed value of the granted plots held now.
    - `/admin/ledger` shows them, and the Observatory's economy page has a **Land given** story.
  - **Not yet:** the client can't tell yet whether a player holds an unspent grant (#448 needs it for "Claim with your deed"), and `plot_claimed` and the admin feed don't tell a granted claim from a bought one.
- **O0.4 Flint follows the server's tour** (#441, M): `Tour` as a view, `hints.toml` and `judge` retired, and Skip becomes Hide with a way back.

  *As built:*
  - **`tutorial_seen { step }`** is the client's message. The server takes it for a step whose `Judge::client_reports()` holds: the four `Client` steps, and `collect_box`, the one `Waivable` step, which #446 waives with words (D10). For any other step it answers `error`. It's `Did::Seen(step)` in `engine::tutorial`, judged in the socket's handler. It needs a `play` key, as `say` does, so citizens can walk the road (#450), and it doesn't drive the avatar.
  - **`capitol_core::onboarding::Tour`** is a view of `TutorialProgress`: the step at hand, `hidden()`, and `report(Did)`, the step to tell the server. `place_in_chapter(step)` gives the pips. Its `Did` is only what the screen shows: the pack used, crafting opened (both `open_pack`), and a zone card read (`read_zone_card`, for #444 to call). The walking, gathering and crafting are the server's.
  - **`ui/hints.rs`** follows `GameState::tutorial()`. `save/hints.toml` and `Tour::judge` are gone, so nothing is kept on the machine. Each step tells the server once a session. The card shows its chapter's name and pips, and there are words for all 30 steps in the bureaucracy's voice, for each chapter's work package to reword.
  - **Hide for now** hides the card here at once and sends `PUT /tutorial/dismissed` (`RestRequest::SetTutorialDismissed`). The state takes the answer, and the socket's `tutorial` after it. While the card is hidden, the header's chip ("The road in", with the chapter's pips) and a line in the `?` overlay bring it back.
  - **`--hints-sample new|half|hidden|help`** shows the card offline on a fresh tour or one half way up the road, or, for a hidden one, the chip (`hidden`) or the overlay's line (`help`).
- **O0.5 The chapter card and markers** (#442, M): the chapter card and its close card, the bearing arrow, and a ground ring at each step's target.

  *As built:*
  - **The card** (`ui/hints.rs`) keeps #441's chapter name, pips for that chapter's steps, `place()`, the gold tick and "Hide for now". A step that finishes its chapter, in whatever order, gives way to **the chapter's close** for 5.5 s (a step's tick is 0.9): "Chapter n of 6", every pip gold, "Arrival: done" with a wax seal stamped by it, one line on what you can now do (`chapter_close`), and "Next: The road in". The last chapter's close is the closing card ("Well begun"), which #449 owns.
  - **Targets** are `capitol_core::onboarding::target(step)`: a `Target` names a kind of place, never one place, so a regenerated world can't strand it. `Point` (server units), `Node(&[resource types])` (the nearest live one, `onboarding::nearest_node`: not spent, not being worked), `Lot(zone)` (the nearest drawn lot in that zone, until #445 picks its stops), `Hall` (its door) and `Stall` (the nearest stall's counter). Today: the four gathers lead to `grass`, `fallen_branches`, `rock_outcrop` and `tree` (not the wilds' old growth, which a stone axe can't fell), the three lot steps to their zones, the Exchange steps to the hall, and the bazaar step to a stall. `Point` is there for the Deeds Office (#448). The workbench's clear spot became a target of its own, `ClearSpot`, found from the ground near you (#443, O1.1 below).
  - **Where to go.** The extension finds the place each frame (`lead_mark`) from what the client holds. Off screen (or within 32 points of the edge), the card shows the hall chip's arrow (`hud::arrow`, turned by `hud::bearing`) and how far ("370 m that way"). On screen, the card says "15 m off, ringed in gold", and `input.rhai`'s `draw_lead` (from `cap_lead()`) rings it on the ground: a slow breathing ring, a dashed one turning outside it, a ripple every 2.8 s, the notice board's glint hanging over it and a few of the hall's motes rising, with a lot's edge traced too. Only a step's card that's up leads anywhere: not a tick, a close, or a hidden card.
  - **Words** stay in one table (`words(step)`), and each chapter's package rewords its own.
  - `--hints-sample` takes a step's name (`gather_stone`), a chapter's (`hall`) or `<chapter>-done` (`arrival-done`, the close), and with `--chat-sample` puts that card in front of its panels.

### O1: The road in (`tu:O1-road-in`)

- **O1.1 Arrival** (#443, M): chapter 1, the quality card, and spawn nodes for every kind the chapter needs.

  *As built:*
  - **The commons.** The default world fell short: within 200 of the spawn grew 6 tufts of grass (4 off the lots) and nothing else; the nearest tree stood 300 off and the nearest outcrop 466, with 2 outcrops inside 600. So `engine::node_spawn::arrival` plants a commons round the spawn (`Frame::spawn`), 180 of it at most, last of the whole plan so no other node moves: a copse of 6 trees with branches under it, outcrops in knots, and grass between, on open ground (`Ground::open`: off the road, the water and every lot by `LOT_MARGIN`), so a farmer's claim can't take the chapter from a newcomer. Its counts are `PULLS` (what one newcomer works of each: a tree, branches twice, stone 4 times, grass 7) times `CROWD` (3), so three newcomers at once walk the chapter with none waiting on a regrowth. On the default world that's 6 trees, 6 branches, 12 outcrops and 19 grass, written by `post-content/20261222_001_arrival_commons.sql`. A regenerate re-plans the trees, branches and outcrops with the rest. The grass is grass (`SEEDED_BY_MIGRATIONS`): the default plan proposes it and the migration writes it once, a regenerate leaves it be, and startup's fit moves it if a road runs over it. `a_new_player_arrives_on_the_road_in_with_grass_in_sight` checks every kind at those counts within `NEARBY_NODE_RANGE`, off the lots.
  - **The clear spot.** `Target::ClearSpot` (`PlaceWorkbench`) is resolved by `onboarding::clear_spot`: the nearest spot 15 to 60 from you (beside you, not under your feet) that's off every lot and the road by 16 (a bench's footprint, so the whole bench stands off them), on dry ground and 52 from every station the client knows (the server's tests, `station::check`, with room to spare), falling back to a spot on the road only where there's no other. The extension keeps it (`keep_clear_spot`) while it stays clear and you stay within 120, so the ring doesn't follow you, and rings it as wide as a bench's own ring. Once it stands, the benches by the spawn all look alike, so every station of yours wears a small gold lozenge on a fine stem over it, in the gold of "yours" (#487, `draw_station_mine` in `input.rhai`, from `StationMark::mine`), fading out from 70 to 160 m from the camera.
  - **Returning the bench** (#488). A playtest found the spawn filling with Arrival's benches: nobody saw that a station comes back to you as its whole kit (#88), because the station panel's Remove was a dim link. It's now a recycling bin in the gold of "yours" (`theme::recycle_bin`, painted, as the envelope and the seals are), whose hover says "Return to inventory" (Chris chose the bin and the words over a "Take down" button). Fell a tree's card asks for it ("The bench's work here is done: open it and press its recycling bin…"), Arrival's close says the bench returns to your inventory whole, to take home, and Settle in's station card offers "the bench you brought from the road" before a new kit. The issue's 80% salvage was dropped (Chris): the kit already comes back whole, so the fix was being seen. `--station-panel-sample empty` and `chest` show the bin live.
  - **The quality card** (`ui/quality_card.rs`) stands in the bench kit's card's place, once a session, until "Go on", while there's a rope in the pack. Its numbers are the player's own (`onboarding::QualityLesson`): the rope's quality (two, if two ropes of two qualities are held), then what two of the worst and two of the best fiber held would twist to, by `rules::quality::output_range` over `rope_basic`'s weights, as crafting's preview reckons it. Then the cascade in a line, the preview's bar, and a line on maker's marks. So "two ropes from fiber of two qualities" are the player's own fiber's two ends rather than two ropes they had to twist; with no fiber left it says the rope came out near the fiber that went in. `--hints-sample make_workbench` shows it on a sample pack.
  - **The words** for the 12 steps and the chapter's close are reworded in `hints::words` and `chapter_close`: the copse, the outcrops ("you'll want four"), the second rope the axe takes, and the tree a tool opens.
- **O1.2 The zone card** (#444, M): `GET /bureaucracy/zone-facts`, `ui/zone_card.rs`, and the three-column comparison.
  - *As built:* `engine::zone_facts` packs every home lot with `station_room::most`, capped by its capacity, once a terrain version (about 1.3 s in release, warmed at startup), and reads the price, the tax and the free lots live. The walk is from each lot's middle to the hall's door. What grows is the spawn tables' rows for the zone, densest first, and the typical lot is one that fits the median, nearest the median area. The card is "The Land Office": every column's plan is at one scale, long side across, with `sim::lot::layout`'s stations as gold dots and a 50 m bar, so a farm's room against a yard's reads at a glance. `capitol_core::zone_card` owns up to fewer than a whole workshop (`WORKSHOP`, 6) and fewer than the capacity. The plot panel opens it ("About the Urban zone"), and `ZoneCard::open_compare` is there for #448. `--zone-card-sample [zone|compare]` captures it offline. Opening it from the plot panel reports `Did::ReadZoneCard` as `tutorial_seen` (#441); #448's office should do the same where it opens the comparison.
- **O1.3 The road in** (#445, S): a stop in each zone, chosen by rule so it survives regeneration.
  - *As built:*
    - **The rule** is `protocol::tutorial::stops` (`stop`, `stop_point`, `STOP_REACH`), so the client and the citizens (#450) find the same lot from the same `/terrain`. Each zone's stop is the lot of that zone nearest a point on the south arterial, measured to the lot's nearest edge. It prefers the nearest free lot within `STOP_REACH` (150) of the point, and takes the nearest whatever its owner when none that near is free. Ties go by id.
    - **The points** are laid in the frame's rings, not at fixed radii, so a move of `ZONE_OUTER_RADII` moves them too. The farm's is half way from the suburb's edge to the spawn (r3775), and the suburb's and the city's are the middles of their rings (r2880, r1580). The issue's r3200, r2400 and r1400 were the same places before Plots v2's flip (#459).
    - **On the default world** the stops are `farm_1_a1_R_0` (60 off the road), `sub_2_0_6_R_14` (50) and `urb_2_5_i_3` (31). From the spawn that's about 4 s to the farm, 18 s on to the suburb, 26 s to the city and about 20 s to the hall, at 50 a second. `models::terrain::tests::the_road_ins_stops_are_lots_of_their_zone_by_the_south_road` checks each is of its zone and within 150 of the road, free or taken, and that the next free one is too.
    - **In Flint**, `Target::Lot(zone)` is now the zone's stop (`CapitolExtension::road_stop`, with everyone's claims from `claimed_lots()`), not O0.5's nearest drawn lot. `read_zone_card` leads to the city's stop, where you are if it's still to do. A taken stop adds a line under the lead: "This one's taken; most near the road are. Its deed reads the same." Each stop's card says what the zone is for, asks for the deed and the zone card, and ends pointing up the road to the next stop.
    - **The plot panel's link** read "About the Urban Zone zone" (`Zone::display_name` already ends in "Zone"). It now reads "About the Urban zone".

### O2: The hall and the people (`tu:O2-hall`)

- **O2.1 The hall** (#446, S): the Exchange, both orders, the box (waivable), a stall and the Gazette.

  *As built:*
  - **The words** (`hints::words`, the bureaucracy's voice): the deposit says fiber in, Strands out, less the fee, which the panel sets in a line; the sell order says what an order book is in two lines ("a list of prices, waiting. Yours waits in it until someone meets it") and names what you made in Arrival; the buy order says a bid needs no seller, that it waits on the book even for an item not trading yet, suggests stone (Settle in's workbench knaps it), and says the Strands wait in escrow and come back on a cancel (#494: "bid for something you haven't got" read as "buy what's for sale" on an empty book); the stall says its prices show from its counter (`STALL_RANGE`, 20 units); the Gazette is pinned to the board by the hall's door, yesterday's prices and who made what. The close is "You can change fiber for Strands, leave a price on the book, shop the stalls and read what the city made yesterday." No step costs more than one order's escrow and the fees.
  - **The market box is waived with words** (D10). The client knows the box once the market panel has fetched it, which it does while you place the orders. While the box step's card is up and the box is known empty, the card says so instead ("Nothing's filled yet, so your box is empty: a price can wait a day for its match. Your orders stay on the book, and the road goes on without them."). 7 s later (`WAIVE_AFTER`, from whichever came later, the card or the empty box) it sends `tutorial_seen { collect_box }`, once (`Hints::market_box`, `Hints::waive`). A hidden card waives nothing. With something in the box, the card asks you to collect it, and collecting counts as before.
  - **The server judges the waiver too.** `tutorial_seen` now goes through `engine::tutorial::seen`, which the socket calls. A step the client may not report is refused as before, and `collect_box` is taken only once both orders are on the book and nothing waits in the box; otherwise it's refused with words ("Something waits in your market box: collect it at the hall"). So a fill on its way is collected, not waived, and a waiver never stands in for what the player could do. #450's citizens waive it the same way.
  - **Where each step leads.** The deposit leads nowhere: the Exchange is a panel (X, or the header's chip) that opens wherever you stand, and the server asks no range of a deposit, so its card says to open it (#491; it led to the hall's door before, which read as "click the market"). The market's three (sell, buy, the box) lead to the hall's door, whose click opens the market. The buy step also leads the panel, once a session while its card is up: stone's book and a clean Buy form, from whatever the sell left there (`hints::BID_FOR`, `Hints::bid_lead`, `MarketPanel::lead_bid`, #494). The stall leads to the nearest stall someone else keeps, and to the nearest bare one only when nobody keeps any. The Gazette leads to its notice board (`Target::Board`, ringed under the board's prompt). The hall's prompt, its motes and the board's glint are #442's ring.
  - **The Wanted page** is built (S2.3, #389), but there's no optional step for it yet. The closing card points at it.
  - `--hints-sample collect_box-empty` shows the box's card with the box empty.
- **O2.2 People** (#447, M): a line, a card, and the Registrar's letter. The letter needs a sender that isn't a player.

  *As built:*
  - **A letter from an office.** Of the issue's two options, the nullable sender: a notice drawn as a letter would have needed a second mailbox. `20261222_001_office_letters.sql` lets `letters.sender_id` be null, adds `from_office` (exactly one of the two, by a `CHECK`), and an office keeps no copy (`kept_by_sender` false, so the reader's drop deletes the row). `office_letters (account_id, letter)` keeps which office letters each account has had, so one goes **once an account**, whatever became of it: dropped, trimmed or never read, it isn't sent again.
  - **`engine::letters`:** `Office` (the `Registrar`, "The Registrar") and `OfficeLetter` (`RegistrarWelcome`, its words), and `send_from_office`: no postage, its `letter_arrived` queued in the same transaction, so a reader who's away hears of it on their return. `page` reads an office's letter with a nil id. On the wire `Correspondent` and `letter_arrived` gain `office` (left out when false, so every fixture holds): no card, no reply.
  - **When it comes.** `Steps::chapter_open(chapter)` is every chapter before it done. The step that opens People, in whatever order, sends it from `tutorial::saw` (`registrar_writes`), live as a `BazaarNotice`; a socket's connect sends it too if People is open and the tour unfinished, in case that failed. **The letter's words differ from the issue's quote:** the deed is earned only once People is done, so it asks for a word and a card first: "Then your deed is ready. Come to the office by the hall and choose where you'll live."
  - **A card by name counts.** `GET /players/by-name/{name}`, how the Gazette's names open cards, now counts for `open_card` as the id route does.
  - **In Flint.** The three cards are reworded: the Local tab and earshot (3 m in the world, `talk::HEARD`'s 30), the citizen's lozenge in a line, trade in a line and never required, and the Registrar's letter. `Target::Person` leads to the nearest other player in sight (`onboarding::nearest_person`), ringed at their feet as they're drawn; with nobody in sight the card says a name in the Gazette opens a card too. The mailbox draws an office's letter with the Capitol's seal (`theme::capitol_seal`, the tower pressed into the wax) where a person's initial would be, its name opens no card, and "The office takes no replies" stands where Reply would. `--mail-sample registrar` shows it.
  - **Not waived.** `open_card` stays judged by the server. In a world with nobody else in it at all, and nobody in the Gazette, it can't be done ([#484](https://github.com/chrischaps/TheCapitol/issues/484)).

### O3: Home (`tu:O3-home`)

- **O3.1 The Deeds Office** (#448, L): the desk by the hall, `ui/deeds.rs`, free lots with "Show me", and the claim with the grant.

  *As built:*
  - **The grant on the wire.** `protocol::tutorial::Grant` (`unearned`, `open`, `spent`) is on `TutorialProgress` and the `tutorial` message as `grant`, left out while unearned, so an older reader sees what it did. The server reads it from `deed_grants` (open with no row once the steps have earned it, as a claim would open it). The client opens it itself when a `tutorial_step` walks the road (`Grant::after`), and a granted claim sends the player's sockets a fresh `tutorial` saying it's spent.
  - **The free lots.** `GET /bureaucracy/free-lots?zone=` (`engine::zone_facts::free_lots`): every unclaimed lot in a home zone, nearest the hall first, with its plot id, terrain id, middle, area, the stations it fits (capped by its capacity, as the zone card counts them) and its straight walk to the hall's door. The room and the walk are the zone card's packing, cached per terrain version; which lots are free is read live.
  - **The place.** `world/hall.rs`'s forecourt gives the stall beside the Gazette's board to the Deeds Office: a clerk's desk in green leather under a plain wax-red awning with a gold valance, its ledger open, an inkwell and pen, the registry's brass stamp, deeds tied in red, a sign with the city's gold seal hanging from the awning, and a lamp that lights at dusk. The forecourt keeps two stalls. `input.rhai` picks it (`cap_office_at`, `cap_open_office`), names it on hover, and while your deed is open and unspent a slow gold glint hangs over its awning. The desk opens its panel wherever you stand, as the board does (D8). `Target::Office` leads the office step to the paving before the desk.
  - **The panel** (`ui/deeds.rs`, "The Deeds Office"): two pages. *The zones side by side* is the zone card's three columns (`zone_card::Plans`, every plan at one scale), each with a link to its free lots. *Free lots* lists one zone's, sortable by the walk to the hall or by room, each row placed by its bearing from the tower ("A farm out to the south-west", since roads have no names, #483), with its stations, area and walk. "Show me" marks it: the road in's ring and traced edge on the ground, the card's arrow while the claim step is up, and otherwise a gold header chip with the arrow and how far, which walks you there. Under both is **your deed**, a slip of the Gazette's paper with the registry's seal: "One deed, any lot in the rural, suburban or urban rings. Taxed as any deed is: N a week" (the listed zone's tax, or each zone's beside the comparison), and where it stands. Opening it reports `open_office` and `read_zone_card` (`Did::OpenedOffice`, `Did::ReadZoneCard`).
  - **The claim.** "Claim with your deed" is on each free lot's row and on any free home lot's plot panel, under the Strands price, which stays (D6). It asks once more before it spends the deed ("Spend your deed on this lot?"). On the plot panel it's offered once the office has been looked in at (D8); before that the panel says the deed waits at the Deeds Office (`capitol_core::deeds::offer`). capitol-core's `RestRequest::ClaimPlot` carries `use_grant`, and `Bot::claim_with_grant` is there for #450. Claims still have no range check.
  - **The moment.** P6.4's gold rings and "The deed is yours", with "Granted for the road in, and on the city's books" under it. The office steps aside for the rings, and the shown lot is let go. The next day's Gazette names each settler in its land column ("Settled, with the deed the road in earns: Ada, a farm out to the south-west"), as the byline allows, the first four and a count of the rest.
  - **Words.** The office step and the claim step are reworded in `hints::words`, and the chapter's close says the deed is spent and the room yours to fill.
  - **Differs from the issue:** the Gazette can't say "a farm on <lane>", because roads have no names: it places the home by its bearing from the tower (#483). The office's desk replaces one of the forecourt's three stalls rather than standing beside them, since the forecourt has four slots. `--deeds-sample compare|lots|shown|confirm|unearned|spent` shows it offline.
- **O3.2 Settle in** (#449, S): a station at home, an automation job, the tax card, and "Well begun".

  *As built:*
  - **Your home.** `capitol_core::onboarding::home_plot` is your home among your plots: a lot in the three home zones, the one claimed last if you hold more than one (a deed bought beside the grant, D6). `onboarding::Home` is what the cards say of it: the stations it **fits**, empty, by `sim::lot::layout`, the packing #444's zone card counts by (the server's `station_room` is its copy), capped by its capacity and packed once a lot, not every frame; what its deed allows; what it's assessed at; and a week's tax. The client reads the week's tax off `PlotInfo::forecloses_at`, which is `rules::FORECLOSE_AFTER_WEEKS` (4, now in the protocol, which the backend's `property_tax` re-exports) weeks of it, at the live rate. So there's no wire change.
  - **Where each leads.** Two new targets: `Target::Home` rings your lot and traces its edge, as the stops are; `Target::MyStation` rings a crafting station of yours with a box (E4.2's rule), the one on your home lot first, else the nearest, such as Arrival's bench by the road (`onboarding::my_station`). The tax card leads nowhere: it asks nothing of the world.
  - **A station at home** says "Your farm fits 12 stations, though its deed allows 25: the room is yours to fill" (the deed's gap in words only where there is one, D9), what a kit takes by hand (a storage chest 8 fiber, a workbench 10), and that a workbench at home can be set to work next. Any station on your own plot counts (`place_at_home`, #439).
  - **The job is edges, forever, not rope.** The issue's "rope, forever" can't be set: rope is a hand recipe, and automation refuses hand recipes (E4.2's `check_config`). Knapped stone is the simplest station job there is (two stone, no tool), at the workbench Arrival taught. The card says so, says the station works from its own box, and that it waits while you're away and goes on when you're back, while a subscription keeps it working (E4.3's `owner_offline`, in the station panel's own words). Setting it on any station of yours counts (`set_automation`), the off-plot bench too.
  - **The tax card** is the only step that's a card and nothing more, so its **"Noted"** is the step: `Did::ReadTax`, sent as `tutorial_seen { read_tax }` once (`Hints::noted`). It says your lot's assessed value, its tax a week, taken a little each hour, that an hour you can't pay is owed and Alt-clicking the lot shows the debt and pays it, what it owes now if anything, and that four weeks unpaid (in Strands) forecloses it: the stations come down and back to you, the land goes back to the city, a bought deed is refunded less the debt and the road in's nothing (D5). The plot panel now shows an owned lot's tax a week too, under its value.
  - **Well begun.** The closing card has the six chapters' pips gold, the city's seal by its title (`theme::capitol_seal`, the Registrar's), "You've walked the road in and made a home, and the city has you on its books. The rest is yours: make, trade, and make your name.", then `G` ("the Gazette each morning, and Wanted: work people will pay for", since S2.3, #389, put the Wanted page beside the Gazette) and `?` ("every control, any time"), each a keycap. It stays 12 s (8 before). Once the tour's finished nothing of it shows: no card, no header chip, no line in the `?` overlay.
  - **`finished_at`** was already set by the last step's `mark` (#438). `engine::tutorial`'s `settling_in_finishes_the_road_and_nothing_sooner` walks the chapter through its real judges (a workbench placed at home, its job set, the tax card seen) for two players, one reading the tax card first and one last, and checks `finished_at` waits for every step either way, and that rope is refused as a job.
  - `--hints-sample place_at_home|set_automation|read_tax|settle_in-done` shows the cards offline on a sample farm (12 of 25, assessed 500, 10 a week since #292); offline, the road in's farm stop stands in for your home's ring.
- **O3.3 Citizens walk the road in** (#450, M): the steps as a `Decider` task, and the claim in the citizen's home zone. `sim/plan.rs` no longer saves for the first deed.

  *As built:*
  - **The task.** `sim/run/road_in.rs`. Every citizen walks the road (`SimBot::road`); the E2.7 swarm doesn't, so its numbers stay comparable. Each round, by day and at market hours, the citizen is asked `Question::Road { step }` with two entries: what it was doing (its errand, or `Rest`) and `Choice::Road { step, chapter, does }`. The rules give the road 1.0 and anything else on that question 0.6 of its worth (`ROAD`, `PUT_OFF`), so they settle it at confidence 1 and Jev is never asked. In the night's work from home (#276) it takes only what needs no walk (the pack, the zone card, the tax: `at_home_tonight`); its own trade often does Arrival's pulls and axe at night anyway, and the server counts them. The prompt text is in `remote::question`/`entry`, as for the merchant's question.
  - **The steps** come from `protocol::tutorial`'s `Step::ALL`: the first not done whose chapter has come (`Steps::chapter_open`) and whose needs are met (`ready`: both orders before the box, an open grant before the claim, a lot with room before a station at home, a crafting station before a job). A round takes one bit of it, as an errand does: a pull, a craft, a walk, a look. So it still answers trades and goes home at dusk, and the road runs across its sessions (#276), the server keeping where it is. A step that fails waits two game minutes (`BLOCKED_FOR`) and the citizen gets on with its trade. A step this build has no way to take (a new one, say #449's) is reported with `tutorial_seen` if it's a screen's, or waits.
  - **What it does for each:** Arrival by the planner, but for the recipe itself (`make("stone_axe")`, not whatever makes an axe), and the bench set up by the spawn off every lot. The stops: up the south arterial to `stop_point`, then `GET /plots/at` on the lot `stops::stop` chooses (`WalkGraph::road_stop`, the client's rule over the lots within 1000 of the road, a test holds it to the client's). The hall: a deposit, one unit of what it holds (a rope if any) on the book at its worth, a bid for one fiber at its worth, then a collect if something filled or `tutorial_seen { collect_box }` if not (#446), a look at a stall someone else keeps (a bare one counts), and the Gazette. People: a greeting in its voice to whoever's within earshot (at the hall if nobody is), else a plain line; the card of the nearest person it can see, else a name from the Gazette's producers (#447, not waivable, D10); and the Registrar's letter read. It sends no reply (#432). A key couldn't read a letter, so `POST /letters/{id}/read` now takes the `play` scope (`api_keys::rest_scope`); sending and dropping stay closed to keys. The office: to the hall, `GET /bureaucracy/free-lots` for its home zone, and `tutorial_seen` for the office (and the zone card if it somehow hadn't).
  - **The claim.** `SimBot::claim_deed`: `POST /plots/{id}/claim { use_grant: true }` (`Bot::claim_with_grant`) on the lot its own claim would take: nearest its home spot in its home zone (`persona::Home`) with room for its calling's stations (since callings K1.3, #532: `stint::room_for`; before, its trade's), else the other home zones. A citizen that owns a home claims a second the same way. The planner's station claim with an open deed claims with it too (`Holdings::road_deed`).
  - **No saving for the first deed.** `Holdings::road_deed`: while it's unearned, a station that needs a plot is stuck ("waits for a plot, and the plot for the road in's deed") rather than saving `deed + FLOAT`; open, it claims; spent, the next lot is bought as before. `go_home` doesn't buy a home while the deed is unspent.
  - **Packing up** (#488): before the road's first stop a citizen takes down its benches near it that stand off its lot (`SimBot::pack_up`, within `PACK_UP_REACH`, 200; one with a job set stays, and since #489 one whose box holds something comes down with it, the box into the pack, unless the pack can't hold it all), so Arrival's bench goes in its pack. It does the same at dusk before the walk home, as a habit, since the kit comes back whole. Settle in's station at home is then that kit, or a bench of its own near it off its lot comes down for it, before it makes a new one.
  - **Settle in** (#449): a citizen whose calling puts no stations on its lot (a fiber or ore specialist, say) sets a workbench there (the kit it packed up, else a second one it makes); one whose calling does sets up its own station there, which counts. Since callings K1.3 (#532) that's by calling, not by trade or by the stint it's on (`road_in::builds_at_home`), so a fiber-born leaner that saws keeps its lot for the sawbench. Then a job on a crafting station of its own: knapped stone for ever at its workbench, the job Settle in teaches (rope is a hand recipe, which no station takes), set but not started, so it doesn't eat the stone its trade gathers. Then the tax step, reported as the client reports it.
  - **Progress.** `protocol::metrics::RoadInReport` (`done`, `of`, `next`, `chapter`, `grant`) on each `CitizenReport` as `road_in`, so `/admin/town` carries it, and the console's roster has a *Road in* column and a line in its headline. The health line says `road in n/30` until it's walked. `capitol-town road-in --roster …` asks the server for each citizen's progress with its own key, by REST only, so a running town plays on.
  - **`capitol-bot --tutorial`** (plan §10): one bot walks the whole road on its account through the same code, as a fiber gatherer with a seeded persona, saying each step as it's done, and fails if it ended without a claim.
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
- ~~A deed price that follows the lot's area or room (`the-capitol-plots.md` PD3).~~ Decided against in #292: the home zones are of equal value and cost the same, so the grant is worth the same whichever zone it's spent in.

## 9. Risks

- **The free choice tilts the city.** If every new player picks the same zone, the grant spends that zone's free lots. #451 measures it. Shrinking a zone's lots (#420) makes it more likely.
- **The reset re-tutors veterans.** Every account walks the road again (D3). The card can be hidden, but the deed waits behind it. Experienced players will fly through it: most steps are things they do anyway.
- **An empty market.** Waived steps (D10) keep the road clear, but the lesson is weaker when nobody's there. Citizens on the road (#450) help. A new town's book is empty for its first hours, merchants or not: producers sell only into bids, and merchants bank a 600-Strand float before they quote. A town minted at 20:39 had its first citizen order at 00:32 (#501, with #410).
- **The spawn's nodes.** A crowd of new players can strip the spawn's branches and outcrops. #443 checks counts for a crowd.
- **A faucet the books don't see.** D5 and #440's ledger work exist so the grant shows up in the books, not as drift.

## 10. Verification strategy

- **Per package:** each issue's tests. Every step judged on the server has one. The grant has claim, refusal, foreclosure and retired-lot tests, plus `conservation`.
- **End to end:** `capitol-bot --tutorial` walks the whole road on a fresh account against a town and claims with the grant. #450 then does it with citizens.
- **In the client:** a play-through on a fresh account (`flint-client\play.bat`), with captures of each chapter's card, the zone card at each stop, the Deeds Office and the claim.
- **The economy:** #451's balance runs, linked here.
- **Every land:** `scripts/ci.sh`. Packages with a migration (#438, #440) also run `ci.sh full`.
