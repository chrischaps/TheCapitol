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
    *As built (V1.1, #455):* the outer ring street is at **r2125.5** (kerb r2133.5), so the urban edge stays at **r2160**: five rings need it at r2134 or more, and a sixth would need r2340. That makes **1016 lots**, median 8.56k, every one taking 4 and none taking 5. The ring of blocks past r1800 takes **32 bearings** (`UrbanParams::outer_spokes`), so no block is over 487 long; at 24 the outer ones would be 540 and the count 1032.
  - **Suburb:** 1000 × 17k at 67% built needs 25.7M, which is r2160 to about r3590. That band is about 1440 deep: less 60 for the warp, 5 rows of 130-deep lots.
  - **Rural:** 1000 × 34k at 45% built needs 76M, which is r3600 to about r6100. That share is higher than today's 35%, because a third main lane and narrower fields carry more farms.
    *As built (V1.3, #457):* **four main lanes, not three,** and about **930 farms** of about 35k, 42 to 43% built, every one taking 12. The rural edge stays at r6100. A farm must be about 132 wide to take 12, and 40 from the next, so the count is about the road length over 175 to 180, both sides of each road. Three lanes about 830 apart carried about 780 farms, and no draw got them to 900: their rows leave a strip about 270 deep between each pair. A fourth lane puts a row in those strips (`the-capitol-rural.md` RD10's V1.3 note). The water costs about 20 farms more than on the unscaled plain, since it now reaches further in (§6).

## 3. Targets

Each zone's v2 targets test holds them over seeds 42, 7, 1234 and 99999, as #421 to #423's tests do. "Takes n" is `models::station_room::takes`, the backend's copy of the client's packing rule.

| zone | lots | lot area | room |
|---|---|---|---|
| urban | 900 to 1100 | median 7.5k to 9.5k | ≥ 95% take 4, **none takes 5** |
| suburban | 900 to 1100 | median 15k to 19k | ≥ 90% take 6, a whole workshop; median 8 |
| rural | 900 to 1100 | mean 31k to 38k | ≥ 99% take 12 |

The suburban room was "≥ 80% take 8" until #466 found that pies round cul-de-sac bulbs and the fans round U-turns can't take 8, so only about half the lots did. Chris settled it on 2026-10-22: 90% taking 6 is enough, with room to build outside and floors (#461) adding more at a cost. Take 8 is still measured and printed, not held (#468).

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
- **`SPAWN_Y`** is the suburb's edge + 350, on the south arterial (r3350 today, so it doesn't move; r3950 on the grown rings).
- **`rules::WATER_INNER_RADIUS`** (the suburb's edge + 150) is what the hydrology and Flint's `land::FROM` both use.

After this, a future move of the rings is a change to the table.

**As built (#454):**
- **`rules::Frame { center, radii }`** holds the centre and the seven zone outer radii; `Frame::TABLE` is the protocol's table, and `Frame::new(world_size, [capitol … rural])` puts the Wilderness out to the disc's edge round the square's middle. It has `rural_edge()`, `suburb_edge()`, `world_edge()`, `world_size()`, `water_inner_radius()`, `spawn()`, `radius(x, y)`, `zone_at(x, y)` and `radii_of(zone)`.
- **`WorldGenParams`** gains a `#[serde(skip)] world_size`, and `frame()` and `framed_land()`. `WorldGenParams::grown()` (`#[cfg(test)]`) sets urban 2160, suburban 3600, rural 6100 and world 16600, so the centre is (8300, 8300). `TerrainConfig::from_params` takes the world's size and centre from the frame.
- **`LandParams.frame`** is `#[serde(skip)]`. `foothill_radius` and `range_radius` stay on the wire, so the JSON and `default_terrain_version_is_pinned` hold, but `LandParams::in_frame(frame)` sets them to the rural edge + `land::FOOTHILLS_PAST_RURAL` (100) and + `RANGES_PAST_RURAL` (1800), and `from_params` always does. So a saved config can't move them. On the table, 4600 + 100 and 4600 + 1800 are exactly 4700 and 6400, so the land's pinned samples hold.
- **The hydrology** reads its rings from the land's frame. **The trails'** radii are offsets: `HEAD_PAST_RURAL`, `PASS_MIN_PAST_RURAL` and `OPEN_GROUND_PAST_RURAL`. **The wild nodes** use `terrain.land.frame`. `wilds.toml`'s `depth` (quality and habitat) is measured past the rural edge: `[[0, -15], [600, 0], [1800, 10]]`.
- **`arterial_end_radius`** is gone. The arterials end at `rural_outer_radius`, and `untunable_radii_in` refuses a config that names it with any other value.
- **Flint's `land::FROM`** is `rules::WATER_INNER_RADIUS − SEAM`.
- **`a_grown_world_builds`** builds `grown()` in about 5 to 6 s in debug, against about 3 s for the default world, before #455 to #457 lay the zones' v2 layouts. Then it checks the land, the water, the arterials and the trails against the grown rings. `the_wilds_follow_a_grown_frame` plans the wild nodes there. Both share one build, `TerrainConfig::grown()` (test-only, cached), which later tests should use too. #455's `urban::metrics::v2` lost its `arterial_end_radius`, and can now take `..WorldGenParams::grown()`.
- **Still on the table** until the flip: the walk (`rules::ground`), `Zone::from_position`, `land::Water::new` (`Water::in_world` takes a size), the non-wild node planner, and the relief drawings. Cut as [#465](https://github.com/chrischaps/TheCapitol/issues/465).

### PV3. Old lots retire as `rw2_`

The flip renames every `urb_`, `sub_` and `farm_` row to `rw2_…` and adds `rw2_` to `plot_sync::RETIRED_LOTS`. Startup then releases an owned one, refunds its deed and gives its stations back, as #422 and #423 did. A new prefix is used because `terrain_id` is unique and old `oldsub_` and `oldfarm_` rows may still exist. Guild halls don't move.

### PV4. Capacity is still #424's

Under today's `size_category`, the v2 lots come out:
- urban: `large` (16), though only 4 fit;
- suburban and rural: `estate` (25).

#424 settles capacity. Floors (V3) need it settled first, since a floor adds to the capacity. §5.6 proposes a rule for Chris to decide: indoor room × (1 + floors) + yard room, each at 50 spacing.

## 5. Buildings, weather and floors (V3, #461)

*Design written 2026-10-22, revised after Chris's notes, and decided by him the same day (§5.11). The work packages are #473 to #478. It takes over #418's questions and #462.*

**Decided (Chris, 2026-10-22):**
- **The building comes with the deed.** Every claimed lot has its building. Most stations on a lot stand *inside* it, and an urban lot is nearly all building, with little need for outside space.
- **Indoors is sheltered from the weather.** Outdoors, weather wears down both the station and its installed tool. On a claimed plot it wears slowly, and a station there is never removed. On public land it wears fast and falls down, which clears away abandoned stations.
- **Shelter is modest.** An outdoor station's tool wears about 1.5× as fast as an indoor one's. The suburb's gardens and the farms' yards are still good places to work.
- **The building cuts away as you come close,** so you can see and use the stations inside.
- **Owners can pay to build upward:** more storeys, more room indoors. With floors an urban lot may hold a whole workshop (all six types), at a cost. The other zones may build up too, but it should be less compelling there.

### 5.1 The building

- **One building per claimed lot, on a footprint the rules set:** `rules::building::footprint(corners, zone)` in the protocol. It's a rectangle square to the street edge (corners 0 to 1), set back by the zone's setback, as wide as the lot allows less the side margins, and as deep as the zone says. The server uses it to tell indoors from outdoors. The client draws the building on exactly that footprint, so what you see is what the rules count (§5.7).
- **By zone** (lot sizes from §2; the depth is a share of the lot's depth, so it follows the lot):

  | zone | lot | building | indoor room at 50 | yard room at 50 | what it is |
  |---|---|---|---|---|---|
  | urban | ~90 × 93 | the lot's width, ~85% of its depth (~85 × 78) | 4 | 0 or 1 | a workshop house: the whole lot but a strip of yard behind |
  | suburban | ~130 × 130 | the width, ~55% of the depth (~120 × 72) | 4 | ~4 | a house and workshop at the front, a garden behind |
  | rural | ~135 × 255 | a farmhouse and barn, ~110 × 90, standing back on the drive | 4 | ~10 | the farmstead in its fields |

  The numbers are estimates. V3.1 measures them from the v2 lots with `station_room`, the footprint taken out of the yard.
- **Indoor room** is counted as outdoors is: stations 50 apart (`STATION_MIN_SPACING`), packed by `station_room`, inside the footprint. One rule everywhere, and it's what a citizen finds when it builds.
- **A station is indoors** when its position is inside the footprint and its `floor` says which storey it's on (0 is the ground). The server keeps it in `stations.indoors`, set when the station is placed. Footprints don't move unless the lot does.
- **Fire indoors.** The kiln and the forge may stand indoors on the ground floor, where a chimney rises through the roof (drawn, §5.7). They can't go upstairs (`ground_only` in `stations.toml`).

### 5.2 Weather

Weather is new. There's no weather system today, only the Gazette's line, and no station wear: only tools wear, from use (E1.7).

- **A station has a condition,** 0 to 100, new: `stations.condition`, starting at 100.
- **Indoors it doesn't wear.**
- **Outdoors on a claimed plot,** the weather wears it slowly: −1 a game day (2 hours, `rules::DAY_LENGTH_SECS`), a live tunable, `weather_wear_claimed`. That's 100 game days, about 8 real days, from new to 0. It never falls down: **a private station can always be repaired.**
- **Outdoors on public land** (a station off any plot, D10), it wears fast: −4 a game day (`weather_wear_public`), about two real days to 0.
- **Outdoors, the tool wears faster too:** an installed tool wears 1.5× per craft (`outdoor_tool_wear`, a tunable). Indoors it wears as it does today.
- **Below 25, a station stops working on its own.** Automation pauses with a note, "needs repair", as it does for an empty box. You can still work it by hand.
- **At 0 on a claimed plot, it stops.** It's out of use, by hand too, until it's repaired. It stays on the plot until the plot is released.
- **At 0 on public land, it falls down.** The station is removed and its kit is lost, and its box goes to the owner's impound, as a foreclosure's does (#141). That's the clean-up of abandoned stations.
- **Repair** is at the station, by its owner or anyone its use level admits. It takes a share of what the station's kit took: planks, bricks or ingots, about a quarter for a full repair, lowest grade first. It's a sink, and a reason to come back. A repair restores the condition to 100.
- **Ticking:** the wear is a running sum. `condition` is worked out from `condition_at` and the time, and written when it's read, repaired or falls below a threshold. A sweep once a game hour finds the ones that fell to 25 or 0, so nothing ticks every station.
- **Later:** the Gazette's forecast could become real, with fine days wearing nothing and storms more. The rate is one tunable, so that's a later change.

### 5.3 Floors

- **A floor is another storey of the building,** with the footprint's indoor room again. An urban floor adds 4: 4, then 8, 12, 16 and on. One floor gives an urban lot a whole workshop indoors.
- **No cap on floors,** on urban, suburban and rural lots. Each floor costs about twice the last (§5.4), so the price is the limit. Guild halls keep their own rules (Commons S4).
- **No fire upstairs** (§5.1).
- **Reaching upstairs:** a station upstairs has its real position inside the footprint, so the server's 2D reach works unchanged. You use it from the street or the yard within `interaction_range` (75), and the client shows the storey it's on (§5.7). There's no stair to walk.
- **The data:**
  - `plots.floors` (0 and up), and `plots.floor_ready_at`;
  - `stations.floor`, `stations.indoors` and `stations.condition`, with its `condition_at`.

### 5.4 What a floor costs

A permit in Strands (the GDD's "plot upgrade permit", behind a `floor_permit_scale` tunable), plus building goods from your pack at their target prices (plank 5, brick 8, nails 6, ingot 50), taken lowest grade first:

Each floor costs about **twice the last** (Chris, 2026-10-22): `rules::floors::cost(k)`. From the second floor on, both the permit and the bill of goods double.

| floor | permit | planks | bricks | nails | ingots | goods | all in | build time |
|---|---|---|---|---|---|---|---|---|
| 1st | 400 | 30 | 45 | 15 | | 600 | **1,000** | 15 min |
| 2nd | 800 | 40 | 70 | 15 | 7 | 1,200 | **2,000** | 30 min |
| 3rd | 1,600 | 80 | 140 | 30 | 14 | 2,400 | **4,000** | 45 min |
| kth | 400·2^(k−1) | | | | | 600·2^(k−1) | **1,000·2^(k−1)** | 15·k min |

- **The first floor costs about an urban deed,** and buys 4 indoor stations in the city: a full workshop for about 2,000 in all.
- **Tall buildings are possible but steep.** Five floors (24 indoor stations on an urban lot) cost 31,000 in all, and the sixth alone costs 32,000. The skyline shows who the city's big producers are.
- **Both parts are sinks.** The Strands go to the Capitol, and the goods are used up (`UnitFlow::Built`).
- **It goes up over time.** Nothing ticks: placing upstairs checks `floor_ready_at`. While it goes up, the client shows scaffolding.

### 5.5 Why it's less compelling outside the city

There's no zone rule:
- **Need.** A workshop is six types. Urban's four fall short, and the suburb (about 8 in all) and the farms (about 14) already hold one.
- **Price.** A floor costs the same everywhere. But the suburb's and the farms' yards are cheap room, and only modestly worse than indoors (1.5× on tools, plus repairs).
- **What it holds.** Floors hold no fire. The work that wants more room on a farm is more kilns and forges, and those need ground.

### 5.6 Capacity (#424, decided)

> **capacity = indoor room × (1 + open floors) + yard room**, each at 50 spacing.

- **Where it's kept:** `plot_sync` works out the indoor and yard room when it lays a lot or the lot changes shape (`station_room`, with the footprint), and keeps them in `plots.indoor_room` and `plots.yard_room`.
- **What it gives:** urban 4 (8, 12, 16 and on with floors), the suburb about 8, farms about 14, each plus their indoor room per floor. Each is a number the lot really holds.
- **What changes:** `size_category` stays as a label, but no longer sets capacity.
- **Stations by place:** each counts against its place, indoors on its storey or in the yard, so a full yard can't borrow from upstairs.

### 5.7 Drawing it: the building and the cutaway (Flint)

- **On the footprint.** `world/houses.rs` draws the building on `rules::building::footprint`, not on seeded sizes. The seed still picks its style (walls, roof, windows, chimney). So a station indoors is inside the walls you see, and #462's stations in the walls go away.
  - **Urban:** a workshop house across the whole lot, with a shop door and a carriage arch.
  - **Suburban:** a house with its workshop wing.
  - **Rural:** the farmhouse and the barn.
  - **Storeys:** the zone's seeded count plus the floors bought.
- **The cutaway.** A building opens when the camera comes within **25 m of your own** (or one whose stations you may use), or **10 m of anyone else's**, or when you aim at a station inside. Chris: start there and see how it feels; both are settings. When it opens:
  - the roof and the walls facing the camera fade out, down to a sill at knee height;
  - each storey's floor is drawn as a cut slab;
  - the stations inside are drawn and can be picked.

  It closes again as you move away. Only nearby buildings open, a handful at a time. Each one leaves its sector mesh for an entity of its own while it's open, as a growing house does, and ray targets follow. The aimed storey is lit, and the storeys above it fade further.
- **Fire:** a chimney over an indoor kiln or forge, smoking while it works.
- **Weather on what's outside:** outdoor stations look weathered as their condition falls: faded, a tilt, moss. Below 25 a mark says "needs repair".
- **Floors:** scaffolding while a floor goes up, then the new storey rises over `GROW_SECS`.

### 5.8 Citizens

- **Indoors first:** a citizen places its stations indoors while there's room, then in the yard.
- **Repairs:** it repairs a station below 50 when it's at home and holds the goods.
- **Floors:** it builds a floor only for a station its role needs and its lot has no room for, once it holds the permit plus its float. In practice that's an urban builder, whose chain is all six types. No citizen builds outside the city.
- **Clean-up:** a citizen that retires leaves its stations. On public land the weather clears them. On its plot they stand until the plot is released.

### 5.9 Protocol and messages

- **`rules::building`:** `footprint(corners, zone)` and the zone's setback, depth share and margins.
- **`rules::floors`:** `cost(k)` and `build_secs(k)`. There's no cap.
- **`rules::weather`:** the default wear rates and the 25 threshold.
- **REST:** `POST /plots/{id}/floors` builds the next floor. `POST /stations/{id}/repair` repairs.
- **`StationInfo`** gains `floor`, `indoors` and `condition`. **`PlotInfo`** gains `indoor_room`, `yard_room`, `floors`, `floor_ready_at` and `next_floor`. **`ClaimedPlot`** gains `floors`.
- **Messages:**
  - `ServerMessage::PlotFloorBuilt`;
  - `StationCondition { station_id, condition }` when a station crosses 50, 25 or 0, or is repaired;
  - `StationCollapsed` to the owner.
- **The books:** `TransactionType::FloorPermit`, `UnitFlow::Built` for floor goods, and `UnitFlow::Repaired` for repair goods. The Observatory shows them among the sinks.
- **Tunables:** `weather_wear_claimed`, `weather_wear_public`, `outdoor_tool_wear` and `floor_permit_scale`.

### 5.10 Work packages

| Issue | Package | Size | Depends on |
|---|---|---|---|
| [#473](https://github.com/chrischaps/TheCapitol/issues/473) | V3.1 **The building and capacity:** `rules::building::footprint`; indoor and yard room in `plot_sync`; capacity from them (§5.6, settles #424); `stations.indoors`; `PlotInfo` | M | |
| [#474](https://github.com/chrischaps/TheCapitol/issues/474) | V3.2 **Weather:** station condition; slow wear on claimed plots, fast wear and the fall on public land; the faster tool wear; the pause at 25; repair; the hourly sweep; the tunables and the books | L | #473 |
| [#475](https://github.com/chrischaps/TheCapitol/issues/475) | V3.3 **Floors:** `rules::floors` (doubling, no cap), the permit and goods, build time, `POST /plots/{id}/floors`, stations upstairs, no fire upstairs, release and refunds | L | #473 |
| [#476](https://github.com/chrischaps/TheCapitol/issues/476) | V3.4 **Flint, the buildings and the cutaway:** houses on the footprint (closes #462), the cutaway at 25 m and 10 m, chimneys, storeys and scaffold, weathered stations | L | #473 |
| [#477](https://github.com/chrischaps/TheCapitol/issues/477) | V3.5 **Flint, the panels:** condition and repair, Build upward, the placement ghost indoors and upstairs | M | #474, #475 |
| [#478](https://github.com/chrischaps/TheCapitol/issues/478) | V3.6 **Citizens:** indoors first, repairs, floors | M | #474, #475 |

### 5.11 Decided (Chris, 2026-10-22)

1. **Capacity (#424):** indoor room × (1 + floors) + yard room.
2. **Weather rates:** −1 a game day on a claimed plot, never cleared. −4 a game day on public land, which falls at 0. Tools wear 1.5× outdoors. All of these are tunables.
3. **A collapse** (public land only): the kit is lost and the box goes to impound. A private station can always be repaired.
4. **Repair:** about a quarter of the kit's materials.
5. **Floors:** no cap. Cost rises steeply instead.
6. **Price:** roughly doubling: 1,000, 2,000, 4,000 and on.
7. **The cutaway:** 25 m for your own buildings, 10 m for others'. See how it feels.
8. **#417 (tighter urban spacing):** closed.

## 6. Risks

- **Generation time.** `test_terrain_generation_performance` already runs at 5.4–7.5× against an 8× limit. A grown world has 1.5× the hydrology, 1.26× the wild band, about 2.6× the farms and 3000 lots. V0.2 samples the height once into a grid shared by the hydrology, the trails and the wild nodes, and measures on `grown()` before the flip.

  *As built (#458):* measured first, and the height wasn't where the time went in debug: about 0.3 s of 3.4 s (the trails' lattice, the water's grid, the trails' slope checks). The lots' clearing (0.84 s) and the suburb's junction weld (0.16 s) were. All three got cheaper with the world unchanged to the bit (the pin, the land's samples and the node plan hold):
  - `lot_clearance` asks a block of samples at once and drops every road and water ring that provably can't reach it, splitting until none are left (0.84 s to 0.14 s);
  - `street_plan::weld_junctions` keeps each road's box and passes over roads out of reach (0.21 s to 0.02 s);
  - `land::Relief` prepares the land's octave seeds and shifts once, and the trails, the water's grid and the wild nodes sample through it (about 10% in debug, 20% of the trails in release).

  The performance test now times both the default world and `grown()`, with three tries each, and has a release limit (#464). Debug: default 3.3–3.9 s (5.9–6.7×) to 2.3–2.8 s (4.1–4.8×), grown 4.8–5.9 s (8.4–9.2×) to 3.4 s (5.8×), against 8×. Release: default 0.9–1.0 s to 0.63 s (9.1–9.6×), grown 1.1–1.5 s to 0.89 s (13.4–14.2×), against 18×. What's left is mostly the suburb's lots and parks (1.3 s grown) and the rural plan (0.4 s grown), which V1.2 and V1.3 rewrite, the trails (0.85 s) and the water (0.5 s).

  **A shared height grid isn't bit-identical as the grids stand.** The trails' half-cell lattice (12.5 apart, from 0) and the wild nodes' cells (centres at 12.5 + 25k) fall on the same points, but the water's 30-unit cells (centres at 15 + 30k) meet that lattice at one point in 25. So sharing with the water would move the world. For the flip (#459), which moves the world anyway: a water `CELL` of 25 would put its centres on the lattice, and one grid could serve all three. It would save about 40 ms in debug (the water's sample) and the wild nodes' ~45 ms at startup, and it would retune the water (its thresholds count cells).
- **Nodes.** The spawn densities are per area, so planned nodes go from about 13k to about 18k. V2.1 rebalances if the tick suffers.
- **The water on a deeper plain.** Soaking may end the courses before they reach the inner farms. V1.3 asserts how far the water reaches.
  *As built (V1.3, #457):* unscaled, the deepest courses stopped about 750 in from the plain's edge. The soaking is now scaled by the square root of the plain's depth over today's (exactly 1 on today's plain), so they reach about 950 in, r5200 to r5350, as far as today's deepest. Scaling by the whole ratio reaches half way across the farms but costs about 65 of them. Which to keep is Chris's: [#472](https://github.com/chrischaps/TheCapitol/issues/472).
- **Players on new ground.** The shift keeps every player at the same place relative to the centre, but the ground under them changes. The flip moves anyone standing in water or on a lot to the spawn.
- **The suburb's patterns.** Pattern (c)'s loop has no room inside for 130-deep lots until its stem shrinks. Every pattern must still fit, or the suburb loses the variety #117 and #108 gave it.
- **Walks get longer.** Spawn to the hall is about 1.2× farther. The Road In plan (`the-capitol-road-in.md`, #438 to #452) assumes the spawn at r3350 and a one-minute walk.

## 7. Work packages

| Issue | Package | Depends on |
|---|---|---|
| [#453](https://github.com/chrischaps/TheCapitol/issues/453) | V0.0 This plan | |
| [#454](https://github.com/chrischaps/TheCapitol/issues/454) | V0.1 One frame of radii: the land, the water, the trails and the wild nodes follow the zone table; `grown()`. Built: see PV2's as-built note | #453 |
| [#458](https://github.com/chrischaps/TheCapitol/issues/458) | V0.2 Generation time, and Flint pre-work (land chunks, `FROM`, grass sizes, a camera-zone test). *As built:* see the generation-time risk in §6; Flint's `land::CHUNKS` comes from `land::OUTER`, the grass mask and heights from the world's size at a fixed texel (3480 and 2320 for 13600, 4248 and 2832 for 16600), the scene's `far` is held by a test to the skirt and the land, and `each_fixed_view_frames_the_zone_its_name_says` checks the `--view` shots | #454 |
| [#455](https://github.com/chrischaps/TheCapitol/issues/455) | V1.1 Urban: five rings of blocks. *As built:* `rings: 5`, `outer_spokes: 32` past r1800, outer ring street r2125.5 under an edge at r2160: 1016 lots, median 8.56k, 100% take 4, none take 5. Tested on the grid alone (`urban::metrics::measure_grid`, `v2`) | #454 |
| [#456](https://github.com/chrischaps/TheCapitol/issues/456) | V1.2 Suburb: lots of about 130 × 130 in a band about 1440 deep. *As built:* r2160 to r3600 as planned; `NeighbourhoodParams::grown()` lays lots 140 × 134 (frontage 140, none under 136, warp 10 for 5 rows 134 deep, stems to 400): 970–996 lots, median 18.4k–18.8k, but only **52–56% take 8** ([#466](https://github.com/chrischaps/TheCapitol/issues/466): pies and U-turn fans can't; Chris made the target 90% taking 6). (c)'s `STEM` needed no change; (b) now wants 4S across. Tested on `grown()` (`suburb::tests::grown_world`, `the-capitol-suburb-v2.md` §4.4) | #454 |
| [#468](https://github.com/chrischaps/TheCapitol/issues/468) | V1.4 Suburb: pie lots take 6, so 90% of the suburb holds a whole workshop. *As built:* pies were the whole shortfall (2% of them took 6). `NeighbourhoodParams::pie_depth` 155 runs each pie's rays on to 155 from its front where the ground allows, past the rear line it shared with the lots behind: 83–90% of pies take 6, and **92.0–92.6% of all lots** (960–990 lots, median 8 stations, mean 7.5–7.6, median area 18.5k–18.8k, take 8 54–59%). Bigger bulbs don't help: from a radius of 20 the pies' fronts cut the bulb's neck and most are dropped | #456 |
| [#457](https://github.com/chrischaps/TheCapitol/issues/457) | V1.3 Rural: three main lanes, farms of about 34k. *As built:* `RuralParams::v2()`, **four** main lanes a sector in bands that are shares of the depth, following winding lanes at half the sway, frontage U[132, 146], field gap U[40, 46], depth U[230, 290], and every farm taking 12 (`room`). On `grown()` over the four seeds: 925 to 934 farms, mean 34.9k to 35.2k, 42 to 43% built, 100% take 12, the water reaching r5243 to r5272 (#472). Tested by `rural::tests::v2_farms_meet_their_targets`, drawn by `rural-plan.ps1 -V2`. The default world doesn't move | #454 |
| [#459](https://github.com/chrischaps/TheCapitol/issues/459) | V2.0 The flip | #455, #456, #457, #458 |
| [#460](https://github.com/chrischaps/TheCapitol/issues/460) | V2.1 After the flip: town reset, node densities, the nightly | #459 |
| [#461](https://github.com/chrischaps/TheCapitol/issues/461) | V3.0 Buildings, weather and floors: designed and decided (§5). The work is #473 to #478 | |

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
