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

#424 settles capacity. Floors (V3) need it settled first, since a floor adds to the capacity.

## 5. Building upward (V3, #461), proposed

To review with Chris before any work package. This takes over #418's questions.
- **A floor** adds a flat **+2 station capacity** on the same footprint. Upstairs stations are reached through the building's door as one interaction point, since the server's reach is 2D.
- **What it costs:** Strands plus building materials (planks, bricks, ingots), rising per floor. It's a sink, and a demand for the crafting chains.
- **How far up:** up to **3 extra floors** in any zone. A flat +2 is a big step on an urban lot (4 → 6 → 8 → 10) and small beside a farm's 15. With farmland cheaper, building up is less compelling outside the city without a zone rule.
- **Tax:** assessed on the deed plus the floors.
- **Flint:** `world/houses.rs` draws storeys from the floor count, so the skyline rises where people build.
- **Citizens:** urban crafters buy a floor when a chain needs another station.
- **To confirm:** with floors, an urban lot can hold a whole workshop, but only at a price.

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
| [#461](https://github.com/chrischaps/TheCapitol/issues/461) | V3.0 Building upward, design first | #424 |

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
