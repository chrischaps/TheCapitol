# The Capitol: Plots v2, a Thousand Lots a Zone

*Plan of record, 2026-10-22. It follows `the-capitol-plots.md` (#420 to #423) and replaces its §3 targets. The work is the GitHub issues labelled `plots-v2`, [#453](https://github.com/chrischaps/TheCapitol/issues/453) to [#461](https://github.com/chrischaps/TheCapitol/issues/461), with phase labels `pv2:V0-groundwork` to `pv2:V3-floors`.*

## 1. What Chris asked for

After the first plot-size plan the zones had 520 urban, 1192 suburban and 377 rural lots, of about 8.6k, 10.3k and 36k: a ratio of about 1 : 1.2 : 4.2. Chris (2026-10-22) wants:
- **Lot area in the ratio 1:2:4,** urban to suburban to rural.
- **About 1000 lots in each zone.**
- **Urban lots that hold some stations, but not a whole workshop.** A full workshop is the six station types: workbench, sawbench, kiln, forge, anvil and storage chest. An urban lot stays at **4** at 50 spacing, as #421's lots already take.
- **Owners can pay to build upward** for more room. Urban gains the most from it. The other zones may build up too, but it should be less compelling there (§5, V3).
- **The rings may move.**

He also chose:
- **Grow the world,** so the wilds keep their 2200 of depth.
- **One flip:** each generator is reworked and tested against the new rings first, then the world moves once.
- **#417 (tighter urban spacing) and #424 (capacity) stay open,** to decide later.

Each zone keeps its own strength. Urban is density by the market. The suburb is a whole workshop on a quiet street. Rural is acreage near the resources.

## 2. The numbers

About 3000 lots at 8.6k, 17k and 34k come to about 60M of lots, against about 25M today. That doesn't fit in today's rings, so every ring moves out:

| zone | ring today | ring after | lots | lot | room at 50 | built |
|---|---|---|---|---|---|---|
| urban | r1008 to r1800 | r1008 to **r2160** | ~1000 (5 rings of blocks) | ~8.6k, 90 × 93 | 4 | ~66% |
| suburban | r1800 to r3000 | r2160 to **r3600** | ~1000 (5 rows of lots ~130 deep) | ~17k, 130 × 130 | ~9 | ~67% |
| rural | r3000 to r4600 | r3600 to **r6100** | ~1000 (3 main lanes a sector) | ~34k | ~15 | ~45% |
| wilds | r4600 to r6800 | r6100 to **r8300** | none | | | |

- **The world** goes from 13600 across to about **16600**. The centre moves from 6800 to 8300, so every stored position shifts by **+1500** (RW1, #308, shifted +800).
- **These radii are estimates.** Each generator's count moves in steps (a ring of blocks, a row of lots), so V1.1 to V1.3 settle the exact radii and the flip uses theirs.
- **How the land works out:**
  - **Urban:** #421's grid gives about 0.123 lots per unit of block radius, so five rings of blocks (outer ring street about r2125) make about 994.
  - **Suburb:** 1000 × 17k at 67% built needs 25.7M, which is r2160 to about r3590. That band is about 1440 deep: less 60 for the warp, 5 rows of 130-deep lots.
  - **Rural:** 1000 × 34k at 45% built needs 76M, which is r3600 to about r6100. That share is higher than today's 35%, because a third main lane and narrower fields carry more farms.

## 3. Targets

Each zone's v2 targets test holds them over seeds 42, 7, 1234 and 99999, as #421 to #423's tests do. "Takes n" is `models::station_room::takes`, the backend's copy of the client's packing rule.

| zone | lots | lot area | room |
|---|---|---|---|
| urban | 900 to 1100 | median 7.5k to 9.5k | ≥ 95% take 4, **none takes 5** |
| suburban | 900 to 1100 | median 15k to 19k | ≥ 80% take 8 |
| rural | 900 to 1100 | mean 31k to 38k | ≥ 99% take 12 |

## 4. Decisions

### PV1. One flip, tested before it lands

V0 and V1 land on master **without moving the default world.**
- **`WorldGenParams::grown()`** (test-only) sets the v2 radii and world size in code. The zone radii are `#[serde(skip)]` fields that code can set (`backend/src/models/terrain.rs`).
- **Each zone gets v2 params** and a v2 targets test against `grown()`.
- **Only the flip (V2.0) changes the defaults.** It does one migration set: the shift, the retired lots, the grass and the node re-plan.

### PV2. The land follows the zone table (V0.1)

Today the zone generators read their radii from `WorldGenParams`, but the land, the water, the trails and the wild nodes read `rules::` statics. V0.1 adds one frame of radii, `rules::Frame`. `LandParams` carries it (serde-skipped, so the JSON and the pin hold), and the radii that sit past the rural edge become offsets from it:
- **The land:** the foothills start 100 past the edge, and the ranges 1800 past it (r4700 and r6400 today).
- **`arterial_end_radius`** is always the rural edge.
- **`content/wilds.toml`'s quality by radius** is measured from the rural edge.
- **`SPAWN_Y`** is the suburb's edge + 300, on the south arterial.
- **`rules::WATER_INNER_RADIUS`** (the suburb's edge + 150) is what the hydrology and Flint's `land::FROM` both use.

After this, a future move of the rings is a change to the table.

### PV3. Old lots retire as `rw2_`

The flip renames every `urb_`, `sub_` and `farm_` row to `rw2_…` and adds `rw2_` to `plot_sync::RETIRED_LOTS`. Startup then releases an owned one, refunds its deed and gives its stations back, as #422 and #423 did. A new prefix is used because `terrain_id` is unique and old `oldsub_` and `oldfarm_` rows may still exist. Guild halls don't move.

### PV4. Capacity is still #424's

Under today's `size_category`, the v2 lots come out:
- urban: `large` (16), though only 4 fit;
- suburban and rural: `estate` (25).

#424 settles capacity. Floors (V3) need it settled first, since a floor adds to the capacity. §5.6 proposes a rule for Chris to decide: the lot's ground room at 50 spacing, plus 2 for each floor.

## 5. Building upward (V3, #461)

*Design written 2026-10-22, for Chris's review. Nothing here is built, and no work package is cut until he has read it. It takes over #418's questions.*

**Decided (Chris, 2026-10-22):**
- An urban lot owner can pay to build upward for more room. The other zones may too, but it should be less compelling there.
- With floors, an urban lot may reach a full workshop (all six station types). Not by default, but at a cost.

### 5.1 What a floor is

- **A floor holds 2 stations.** It's the house's footprint again, one storey up. An urban house fills its lot's front, about 90 across and 30 deep (`houses.rs`, #421), and two stations at 50 spacing fit in that. So +2 isn't a number picked for balance: it's what the room upstairs holds.
- **The lot's levels are counted apart.** The ground takes its ground room (§5.6). Each floor takes 2. A lot's capacity is the sum. A station names its level, so a full ground floor can't borrow from upstairs, and the reverse.
- **No spacing upstairs.** Stations upstairs are indoors. The count is the limit, so `STATION_MIN_SPACING` doesn't apply to them, and the ground's spacing test ignores them.
- **Fire stays on the ground.** The kiln and the forge can't go upstairs: a new `ground_only = true` in `stations.toml`. The workbench, sawbench, anvil and storage chest can. Two reasons:
  - The full urban workshop still works: kiln, forge and two more on the ground, the other two on the first floor.
  - Floors add cold room only. A second forge for an ingot line needs ground, so heavy work stays where the land is, and the city's strength is the finishing trades.
- **Up to 3 floors** above the ground, on urban, suburban and rural lots. The guild district's halls keep their own rules (Commons S4).
- **The data** is columns, not a table:
  - `plots.floors SMALLINT NOT NULL DEFAULT 0`, 0 to 3;
  - `plots.floor_ready_at TIMESTAMPTZ`, when the newest floor opens, NULL once it has;
  - `plots.ground_room SMALLINT` (§5.6);
  - `stations.floor SMALLINT NOT NULL DEFAULT 0`, 0 for the ground.

  A floor has nothing of its own but its number and when it's ready. A table would earn its place if floors had their own quality or owner. They don't.

### 5.2 Reaching a station upstairs

The server's reach is 2D, so a building has one interaction point: **its door.**
- **The door** is a pure function in the protocol, `rules::floors::door(corners, zone)`. It's the middle of the lot's street edge (corners 0 to 1, as every generator lays them; a test pins it), moved in by the zone's door depth. That's where the client's houses already put their front walls: urban 3, suburban 23, rural 85 (a farmhouse stands back on its drive).
- **An upstairs station stands at the door.** Its `(x, y)` is the door point, so every check that reads a station's position works unchanged: reach to use it, take it down or load its box, the nearby-stations lists, automation. You use any station upstairs from within its `interaction_range` (75) of the door.
- **What the client shows:**
  - Upstairs stations aren't drawn in the yard.
  - A click on a house (it's a ray target since #293), or E at its door, walks you to the door and opens **the building panel**: the ground's count, then each floor and its stations. Picking one opens its station panel as today. Anyone the plot's `station_use_level` admits sees it, as with the stations in the yard.
  - Placing a kit at the door, the ghost offers "Upstairs: first floor, 1 of 2 free" for a type that may go up.

### 5.3 What a floor costs

A floor is paid in two parts:
- **A permit in Strands:** the GDD's "plot upgrade permit" from the Deeds Office. It's a live tunable, `floor_permit_scale` (D6.1), 1 by default.
- **Building goods from your pack:** planks, bricks and iron nails, and ingots higher up. They come from three chains: the sawbench, the kiln and the forge.

Both are sinks. The Strands go to the Capitol, and the goods are used up (`UnitFlow::Built` in the ledger). The goods are priced at their `target_price` (plank 5, brick 8, nails 6, ingot 50), which is about 15 Strands a labour-minute (economy D5).

| floor | permit | planks | bricks | nails | ingots | goods at target | all in | ≈ labour |
|---|---|---|---|---|---|---|---|---|
| 1st | 400 | 30 | 45 | 15 | | 600 | **1,000** | 1.1 h |
| 2nd | 700 | 40 | 70 | 15 | 3 | 1,000 | **1,700** | 1.9 h |
| 3rd | 1,200 | 60 | 100 | 25 | 7 | 1,600 | **2,800** | 3.1 h |
| all three | 2,300 | 130 | 215 | 55 | 10 | 3,200 | **5,500** | 6.1 h |

- **Why these numbers.** The first floor costs an urban deed, so a station upstairs costs twice what one on the ground did. That's the price of having it all at one door, near the market. The full urban workshop comes to 2,000 (the deed and the first floor), against a suburban deed of 500 that holds nine. Each floor after costs more, as building higher does. The third is for the city's big producers, and the skyline shows who they are.
- **What the first floor takes from the land:** 10 logs, 46 clay and 23 charcoal for the bricks, and 2 ingots for the nails.
- **Any grade will do.** The goods are taken lowest quality first (`Pick::default()`), so a floor is a use for the low grades nobody else buys.
- **How it's built.** You stand within reach of your door with the goods in your pack and press Build. The permit and the goods are taken at once, in one transaction. Then the floor goes up over time: **15 minutes for the first, 30 for the second, 45 for the third** (`floor_ready_at`). Nothing ticks: a placement upstairs checks the time. You can work on the ground meanwhile, but you can't start a floor while one is going up.
- **Rejected: building it as a craft.** A "floor kit" made at a station would give floors a quality that nothing reads, and a kit nobody else could use. The goods already pass through three stations on their way.
- **No taking a floor down** in this version. Its stations come down as any other.

### 5.4 Limits by zone

Up to 3 floors in every zone, with no zone rule. Building up is still less compelling outside the city:

| zone | deed | ground room at 50 | +1 floor | +2 | +3 | a station on the ground costs | a station on the first floor costs |
|---|---|---|---|---|---|---|---|
| urban | 1,000 | 4 | 6 | 8 | 10 | 250 | 500 |
| suburban | 500 | ~9 | 11 | 13 | 15 | ~56 | 500 |
| rural | 250 | ~15 | 17 | 19 | 21 | ~17 | 500 |

- **Need.** A workshop is six types. Urban's four fall short, and the first floor gets there. The suburb's nine and a farm's fifteen already hold one.
- **Price.** A floor costs the same everywhere, but the ground it competes with is far cheaper outside the city. A station upstairs costs twice the ground's in the city, 9 times the suburb's, and 30 times a farm's.
- **What a floor holds.** Floors add cold room only (§5.1). The work that wants more room on a farm is the fire: more forges and kilns.

### 5.5 Tax, release and foreclosure

- **Assessed value is what was paid in Strands:** the deed plus the permits. The goods aren't assessed: a target price is a tuning number, not a price anyone paid. At 2% a week an urban lot pays 20; with floors, 28, 42 and 66.
- **Release and foreclosure refund the assessed value**, as they do today (foreclosure less the debt). So the permits come back, and the goods don't: they were used.
- **A release the owner didn't cause** (a terrain commit, a retired layout) also refunds the goods, in Strands at their target price, from `rules::floors::cost`. (Open question 5.)
- **`release_plot`** sets `floors` to 0 and `floor_ready_at` to NULL. The upstairs stations come down with the rest, and their kits and boxes go back as today.

### 5.6 Capacity (#424), proposed for Chris to decide

Floors add to capacity, so they need #424 settled. Today `size_category` gives urban lots 16, and the suburb's lots and the farms 25. None of these is reachable. **The proposal:**

> **capacity = the lot's ground room at 50 spacing + 2 for each open floor.**

- **Ground room** is `models::station_room` (the backend's copy of `sim::lot::takes`) run on the lot's corners: the most stations the empty lot takes. `plot_sync` works it out when it lays a lot or the lot changes shape, and keeps it in `plots.ground_room`. A migration fills it in for the lots there now. Room is always measured at 50, so if #417's tighter urban spacing ever lands, it doesn't raise urban capacity.
- **What it gives:** urban 4, then 6, 8 and 10; the suburb about 9, up to 15; farms about 15, up to 21. Each is a number the lot really holds. The Road In's zone card (#444) shows "fits 4, up to 10 with floors".
- **What it retires:** `plot_size_categories` no longer sets capacity. `size_category` stays as a label.
- **A lot over its new capacity** keeps its stations, and takes no more until it's under.
- **The other option** is a number per zone: 4, 8 and 12, the floors of the §3 targets. It's simpler to say, but it caps the bigger lots below what they hold.

### 5.7 Drawing it (Flint)

- **Storeys:** `houses.rs` draws the main house's storeys as its seeded count plus the lot's floors. The seeded count stays, so each floor visibly adds one. An urban terrace goes from 2 or 3 storeys up to 6, so the skyline rises where people build.
- **A lot with floors always has a house,** down to the zone's smallest. Its front wall sits at the door depth, with its door as near the door point as the house allows.
- **While a floor goes up,** scaffolding (the growth stage's timber frame) stands round the top storey until `floor_ready_at`. Then the new storey rises over `GROW_SECS` (frame, walls, roof), as a new house does.
- **Ray targets:** houses are ray targets already (#293), so a taller house keeps the camera out and a click picks it. A capture should check the over-the-shoulder camera on a narrow urban street between two six-storey terraces.
- **Lamps:** the new storeys' windows are lit by the same seeded chance as the rest. A storey whose station is at work could stay lit at night. That's a follow-up, in case rebuilding the quarter's lamp mesh costs too much.
- **Meshes:** a change of floors rebuilds its sector's mesh, as a claim does.
- **To check while there:** on an urban lot of 90 by 93, the packer's front pair of stations stands about 19 in from the street, inside the house drawn across the front. V3.5 should look at that with a capture.

### 5.8 Citizens

- **The rule:** a citizen builds a floor only for a station that its role needs, that may go upstairs, and that its lot has no ground for. It builds only once it holds the permit plus its float, as with a deed. It gets the goods the way it gets anything (`plan::obtain`): it makes them or buys them.
- **In the planner:** `Goal::Station` with no ground room left gives `Step::Place { floor }` when a floor has room, or else `Step::BuildFloor`. `Holdings` gains the floors and their room. One floor at a time.
- **Who it catches:** an urban builder. Its kiln, forge, sawbench and anvil fill the ground, and its chest needs the first floor. A builder already makes bricks, so it uses its own. Every other role's stations fit on the ground, so no one else builds, and no citizen builds outside the city. That's §5.4's "less compelling", checked by the citizens themselves.
- **The Observatory** counts the floors built, and shows the permits and goods among the sinks.

### 5.9 Protocol and messages

- **`rules::floors`** (protocol): `MAX_FLOORS` (3), `FLOOR_ROOM` (2), `cost(k)` (the permit before the tunable, and the goods), `build_secs(k)`, `door(corners, zone)` and the door depths. The server and both crates of the Flint client read it.
- **`POST /plots/{id}/floors`** builds the next floor. It's REST, like a claim. It checks that you own the lot, it has fewer than 3 floors, none is going up, you stand within reach of the door, and you hold the permit and the goods. It answers the `PlotInfo`.
- **`ServerMessage::PlotFloorBuilt { terrain_id, floors, ready_at }`** goes to everyone, so every client's house rises. The owner also gets `currency_changed` and `inventory_updated`, as usual. `plot_released` already tells clients to drop the floors with the house.
- **`place_station`** (WS) and `/stations/place` (REST) gain `floor`, left out for the ground. Upstairs, the server puts the station at the lot's door point, whatever `(x, y)` says, as long as it's on the lot.
- **`PlotInfo`** gains `ground_room`, `floors`, `floor_ready_at` and `next_floor` (the next floor's cost with the tunable applied, or nothing at 3). **`ClaimedPlot`** gains `floors` and `floor_ready_at`, both left out when there are none. **`StationInfo`** gains `floor`, left out on the ground.
- **The books:** `TransactionType::FloorPermit` for the Strands, and `UnitFlow::Built` for the goods. Refunded permits are part of the `deed_refund`.
- **The Gazette** prints new floors with the deeds (a `deed_events` kind, `floor_built`).
- **Fixtures** for `plot_floor_built`, and for `place_station` with a floor.

### 5.10 Work packages, proposed

These aren't issues yet. They're cut after Chris reviews this section.

| Package | What | Size | Depends on |
|---|---|---|---|
| V3.1 | **Capacity by room** (§5.6, once #424 decides): `plots.ground_room` from `station_room`, in `plot_sync` and a migration; capacity from it; `PlotInfo.ground_room`; the zone card | M | #424 |
| V3.2 | **Floors in the rules:** `rules::floors`; the migration; `POST /plots/{id}/floors`; the permit, the goods, the ledger and the tunable; assessed value and refunds; `plot_floor_built`; the Gazette's line | L | V3.1 |
| V3.3 | **Stations upstairs:** `floor` on placement and on `StationInfo`; `ground_only`; room by level; no spacing upstairs; the door as their point; `capitol_core::station::check` kept in step | M | V3.2 |
| V3.4 | **Flint, the panels:** Build upward in the plot panel (the cost, what you hold of it, the time left); the building panel at the door; the ghost upstairs; upstairs stations kept out of the yard | M | V3.3 |
| V3.5 | **Flint, the houses:** storeys from floors; the scaffold and the storey rising; the door; lamps; captures of the skyline and the camera | M | V3.2 |
| V3.6 | **Citizens:** the planner's floor step, room by level, the town report, the Observatory's counts | M | V3.3 |

### 5.11 Open questions for Chris

1. **Capacity (#424):** ground room at 50 spacing plus 2 a floor (proposed), or a number per zone (4, 8, 12)?
2. **The price:** a first floor at about an urban deed (400 Strands and 600 of goods), rising to 5,500 for all three. Too dear, or too cheap? The permit is a tunable either way.
3. **Fire on the ground:** the kiln and the forge can't go upstairs. Yes?
4. **Build time:** 15, 30 and 45 minutes, or at once?
5. **Refunds:** a foreclosure refunds the permits but not the goods. Should a terrain release refund the goods too, at their target price?
6. **The look outside the city:** a suburban house of 4 or 5 storeys may look wrong among the bungalows. If it does, should the suburb and the farms draw their floors as a wing or a loft, or allow fewer floors?
7. **Guild halls:** no floors, with the halls left to Commons S4?
8. **#417 (tighter urban spacing):** floors are now the route to more urban room. Close #417?

## 6. Risks

- **Generation time.** `test_terrain_generation_performance` already runs at 5.4–7.5× against an 8× limit. A grown world has 1.5× the hydrology, 1.26× the wild band, about 2.6× the farms and 3000 lots. V0.2 samples the height once into a grid shared by the hydrology, the trails and the wild nodes, and measures on `grown()` before the flip.
- **Nodes.** The spawn densities are per area, so planned nodes go from about 13k to about 18k. V2.1 rebalances if the tick suffers.
- **The water on a deeper plain.** Soaking may end the courses before they reach the inner farms. V1.3 asserts how far the water reaches.
- **Players on new ground.** The shift keeps every player at the same place relative to the centre, but the ground under them changes. The flip moves anyone standing in water or on a lot to the spawn.
- **The suburb's patterns.** Pattern (c)'s loop has no room inside for 130-deep lots until its stem shrinks. Every pattern must still fit, or the suburb loses the variety #117 and #108 gave it.
- **Walks get longer.** Spawn to the hall is about 1.2× farther. The Road In plan (`the-capitol-road-in.md`, #438 to #452) assumes the spawn at r3350 and a one-minute walk.

## 7. Work packages

| Issue | Package | Depends on |
|---|---|---|
| [#453](https://github.com/chrischaps/TheCapitol/issues/453) | V0.0 This plan | |
| [#454](https://github.com/chrischaps/TheCapitol/issues/454) | V0.1 One frame of radii: the land, the water, the trails and the wild nodes follow the zone table; `grown()` | #453 |
| [#458](https://github.com/chrischaps/TheCapitol/issues/458) | V0.2 Generation time, and Flint pre-work (land chunks, `FROM`, grass sizes, a camera-zone test) | #454 |
| [#455](https://github.com/chrischaps/TheCapitol/issues/455) | V1.1 Urban: five rings of blocks | #454 |
| [#456](https://github.com/chrischaps/TheCapitol/issues/456) | V1.2 Suburb: lots of about 130 × 130 in a band about 1440 deep | #454 |
| [#457](https://github.com/chrischaps/TheCapitol/issues/457) | V1.3 Rural: three main lanes, farms of about 34k | #454 |
| [#459](https://github.com/chrischaps/TheCapitol/issues/459) | V2.0 The flip | #455, #456, #457, #458 |
| [#460](https://github.com/chrischaps/TheCapitol/issues/460) | V2.1 After the flip: town reset, node densities, the nightly | #459 |
| [#461](https://github.com/chrischaps/TheCapitol/issues/461) | V3.0 Building upward: the design is written (§5) and awaits Chris's review. V3.1 to V3.6 are cut after it | #424 |

## 8. Verification

- **V0:** the pins and captures don't change, and `scripts/ci.sh` passes.
- **V1:** each zone's v2 targets test over the four seeds on `grown()`, and its plan drawing:
  - `urban-plan.ps1`;
  - `suburb-plan.ps1 -Inner 2160 -Outer 3600`;
  - `rural-plan.ps1` with the v2 ring.

  Plus the hydrology, trails and wilds tests on `grown()`.
- **V2.0:**
  - `ci.sh full`;
  - a scratch-database run: an owned lot in each zone released with its `deed_refund`, about 3000 lots laid, the nodes re-planned, the grass clear and in its zone, and no one stranded;
  - Flint captures of every `--view`, and the view from just outside the rural edge with `--claim-sample 0.75 --note ""`.
- **[H] Chris:** the skyline from the rural edge, the walk times, and how the wilds read past the farms.
