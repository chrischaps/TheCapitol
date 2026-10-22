# The Capitol: Plot Sizes, Even Across the Zones

*Plan of record, 2026-10-22. The umbrella is [#420](https://github.com/chrischaps/TheCapitol/issues/420), and the work is three issues: urban [#421](https://github.com/chrischaps/TheCapitol/issues/421), suburban [#422](https://github.com/chrischaps/TheCapitol/issues/422) and rural [#423](https://github.com/chrischaps/TheCapitol/issues/423). The layouts' own plans stay where they are: the suburb in `the-capitol-suburb-v2.md`, the farms in `the-capitol-rural.md`. This doc holds the targets they share and how each part meets them.*

## 1. Vision

You can own land in three rings, and each is worth having for a different reason:
- **Urban:** a workshop yard a step from the market.
- **Suburban:** a house and a garden on a quiet street, with room for a trade.
- **Rural:** a farm on a lane, with room to spread out.

Today the counts run the wrong way round. The ring with the least land has the most lots, and lots too small to work, while the biggest ring has a hundred farms. The aim is about 600, 1200 and 400 lots. Each lot is big enough to be worth a deed, and each ring is worth about the same overall (Chris, 2026-10-22). The zone each kind of citizen lives in follows the same idea (#416).

## 2. Where things stand (2026-10-22, the default world)

*Measured from `dump_default_terrain`: count and area from the lots themselves, and "room" as how many stations fit at the spacing, kept 10 in from the sides, packed greedily from a 5-unit grid (as `sim::lot::spots` does), uncapped by the lot's capacity.*

| zone | ring | land | lots | built | lot area, median | room at 50, median | take 4 | category, capacity |
|---|---|---|---|---|---|---|---|---|
| urban | r1000–1800 | 7.0M | 3008 | 4.3M (61%) | 1.4k (about 35 × 40) | 1 | 0% | medium, 9 |
| suburban | r1800–3000 | 18.1M | 1905 | 12.8M (71%) | 6.7k | 3 | 22% | large, 16 |
| rural | r3000–4600 | 38.2M | 117 | 13.9M (36%) | 117k | 48 | 100% | estate, 25 |

- **Urban is smaller than #420 says.** #420's "48 × 48" is the lot's bounding box. The lots themselves are 40 of frontage by about 35 of depth, after the street insets. Every urban lot takes exactly one station.
- **The suburb is held back by shape, not area.** Its median lot is 64 × 106, which would take 4 as a box. But the rows bend, and the pies and strips narrow to a point, so only 22% of them do.
- **The capacities can't be reached.** Urban's 9 and suburban's 16 are both above what fits. Only rural's 25 is real.
- **The guild district** (72 halls of about 20k) is out of scope.

The deed is 250 in all three zones, as a live trial in the town. Before that trial, rural's flat deed was #292.

## 3. Targets

| zone | lots | lot area (median) | room at 50 | lots that take 4 |
|---|---|---|---|---|
| urban | **500 to 700** | 7k to 9k (about 80 × 100) | 4 | ≥ 90% |
| suburban | **1050 to 1350** | 9k to 12k | 4 | ≥ 70% |
| rural | **350 to 450** | 32k to 42k | ≥ 10 | ≥ 99% |

- **4 is a narrow fit.** At 50 spacing with 10 kept in from the sides, a box needs about 70 × 70 of usable ground, so 80 × 100 and 100 × 106 both take 4 with little to spare. That's why urban needs true trapezoids, and the suburb fewer slivers, not only bigger lots.
- **Urban gets more built land, not less.** 600 × 8k is 4.8M, against 4.3M today, because fewer streets cross the band.

- **Hard targets** are the counts and the share that takes 4. Each part's test asserts them over seeds 42, 7, 1234 and 99999, as `neighbourhood_suburb_meets_its_targets` and `farms_meet_their_targets` do. The test prints the soft misses.
- **"Takes 4"** is `capitol_core::sim::lot::takes(plot, 4)` at today's spacing (`STATION_MIN_SPACING` 50, `SPOT_SPACING` 55). The backend's tests can't call `capitol-core`, so each part's test counts it with a copy of the same packing rule. #417's tighter urban spacing comes on top of this; it isn't counted on here.
- **Suburban area is lot area, not box area.** #420's "about 17k" read as a lot's area would need 20M, more than the ring holds (18.1M, with streets and parks in it). 1200 lots at about 10.5k is 12.6M, the 71% built share the suburb keeps today. So the suburb's land stays the same and is cut into fewer, wider pieces.
- **Rural built share stays.** 400 farms × 37k is about 15M, close to the 13.9M (36%) the farms cover today. Smaller farms, not more farmland. The fields between them stay.

## 4. Decisions

### PD1. Each part lands on its own

Each of #421, #422 and #423 moves the default world. Each one lands with its own migration, pin and node plan, so a bad part can be reverted alone. **Order: rural, then suburban, then urban.**
- **Rural (#423)** is the smallest change: the same roads and farms with smaller draws (§5.3). It also matters most to citizens: gatherers and builders live rural, and today's 117 farms run out at about 1 citizen in 6 for a town of 640.
- **Suburban (#422)** retunes `suburb/lots.rs` and the row count, inside a generator that already meets its invariants.
- **Urban (#421)** is a new generator, the biggest piece of work, and #417's spacing waits on its lot size.

### PD2. Capacity follows the room, not the area ([#424](https://github.com/chrischaps/TheCapitol/issues/424))

`plot_sync::size_category` sets a lot's capacity from its area, with splits at 2.5k and 15k. At the targets, urban and suburban both come out `large` (16) and rural `estate` (25). That's no longer a zone's character, and 16 can't be reached at 50 spacing on an 8k lot. **Chris decides on #424.** Two ways to go:
1. ★ **Keep the categories by area, and move the splits** so each zone lands where it should: urban `medium` (9, reached with #417's spacing and #418's floors), suburban `large` (16), rural `estate` (25). The splits become about 9.5k and 20k. It's one function and a test (`sizes_follow_area`).
2. **Category by zone**, the way the guild district already works (`zones.plot_size_category`). It's simpler to read, but a big lot is no longer worth more than a small one in the same zone.

The ★ default is option 1, because it keeps "bigger lot, more room" inside a zone. Either way the capacity stays a room count, as #417 decided.

### PD3. The deed stays flat until the sizes are in

The deed is 250 in every zone, as a town trial. Changing the sizes and the price together would make the trial unreadable. So this plan changes no price. Once all three parts are in, cut an issue (`economy`, `needs-decision`) for a deed that follows the lot's area or its room, using the town's numbers. #292 folds into it.

### PD4. Old lots get a new prefix, and the migration releases the owned ones

Every part removes most of its zone's lots. The flip follows the pattern of #112 (the suburb) and #290 (the farms):
- **New ids under a new prefix,** so the migration can tell old rows from new: urban `urb_…` (was `urban_plot_…`). Suburban lots keep `sub_…` only if the scheme can't collide; otherwise `sb_…`. Farms keep `farm_…`, so #423's migration takes them by zone, not by prefix. Each part settles its exact scheme.
- **A post-content migration** (it touches stations; see `backend/migrations/post-content/README.md`) releases every owned plot in the zone it rebuilds, refunds the deed with a `deed_refund` ledger row, and gives the stations back as `plot_sync::release_plot` does. The next `sync_plots` deletes the released rows and lays the new lots.
- **The old prefix joins `plot_sync::RETIRED_LOTS`,** so a world built after the migration still releases an old owned lot at startup.
- **Seeded nodes:** a move migration from `print_node_moves`, and the node plan re-run (`the_migration_writes_the_default_plan`, `print_new_nodes`).
- **`default_terrain_version_is_pinned`** gets its new pin in the same commit.
- **The town** (`capitol_town`) gets the migration when it starts. Its citizens' plots are released like anyone's, and they claim again. A `town.sh reset` is the clean option after the urban part.

## 5. How each part meets its target

### 5.1 Urban (#421): a new grid

Today: arcs every 100 and 48 spokes (`urban_radial_spacing`, `urban_spoke_count`), with lots of 40 frontage by 50 depth cut from the blocks (`TerrainConfig::generate_urban_plots`).
- **Three rings of blocks about 216 deep,** each block two lots of about 100 back to back between ring streets. They take about 650 of the 790-deep band past the canal. The leftover of about 140 goes to a wider ring road, a boulevard with trees, or a square by the canal, rather than a fourth ring. A fourth ring would make about 830 lots.
- **The count.** The rings sit at about r1150, r1370 and r1590. Two fronted sides each give about 2π × 4100 × 2 ≈ 52k of frontage, less the cross streets. At 85 a lot that's about 600.
- **Fewer spokes:** about 24 (every 15°, so the 8 arterials stay spokes). The middle ring's block is then about 360 along, which makes 4 lots a side. The outer ring may need 32 spokes to keep the frontage near 85 rather than letting it grow past 100; #421 tunes it.
- **Reuse:** `lot_geometry`'s overlap pass and `street_plan::audit` for the streets. The lots are trapezoids, so `sim::lot`'s packing already handles them.
- **Flint:** `world/houses.rs`'s urban styles on the bigger footprint: a narrow house on the street with a yard behind for the workshop.

### 5.2 Suburban (#422): wider and fewer

Today: five rows of lots about 106 deep in the 1200 band, frontage 64 (`suburb/lots.rs`), 1905 lots, many near-triangular strips that take 2 or 3 stations (#305).
- **Frontage 64 → about 100.** That's the lever. It drops the count by about a third (1905 × 64 / 100 ≈ 1220), takes the median lot from 6.7k to about 10.6k at today's depth of 106, and widens the narrow end of a strip.
- **Rows stay at five.** Four rows of about 140 would make the lots longer, not wider, and give back the parks' share.
- **Pies and combs:** fewer pie lots round a bulb (2 to 4, by area), and a looser comb in pattern (b), so the loop patterns stop making slivers.
- **Targets:** `the-capitol-suburb-v2.md` §4.4 takes the new count and area, and a share-that-takes-4 row.

### 5.3 Rural (#423): smaller farms, the same lanes

Today: 117 farms with frontage U[380, 500], field gap U[240, 440] and depth U[280, 380] clipped by the bands (`RuralParams`).
- **Two farms deep along every road.** The bands are about 330 to 366 a side, so two farms of about 165 deep stand where today's one does: a front farm on the road, and a back farm behind it on a short drive, laid like a farm gate's road (`farm_{s}_{road}_{L|R}_{k}b`). On frontage of about 220 each is about 36k.
- **Frontage U[200, 260], field gap U[100, 200].** Today a farm and its gap take about 780 of road. These take about 380, so twice as many farms fit along each road. The gap keeps the fields between farms, at a scale that suits the smaller farm.
- **That's about 117 × 2 (along the road) × 2 (deep) ≈ 470** before the overlap pass and the water take their share. The target is 400. If the count comes out high, widen the field gap first: that keeps the fields.
- **If the back farms read badly in Flint** (pasture with no road), the fallback is a third road between the mains in each sector, which needs no drives.
- **Targets:** `the-capitol-rural.md` §5 takes the new count (350 to 450) and mean area (32k to 42k). The half-sector shares and the gap of 40 stay.

## 6. Verification

For each part:
- Its targets test, over the four seeds, and its plan drawing (`suburb-plan.ps1`, `rural-plan.ps1`, and a new urban one for #421).
- `scripts/ci.sh full` before landing: the parts are migrations and generation.
- The Flint client against the new world: houses, plots, grass and the pasture. Take captures in `flint-client/renders/`. **[H]** Each zone should read the way §1 says: a yard by the market, a garden on a street, a farm on a lane.
- A town run after all three: citizens settle, and the share of a 640 roster that finds a home goes up from today's.

## 7. Related

- **#417:** tighter urban spacing, on top of a lot that already takes 4.
- **#418:** floors on urban lots, the route to capacity 9 (`needs-decision`).
- **#416 (landed):** urban homes for merchants and some crafters. Gatherers and builders can live urban once lots take 4.
- **#419 (landed):** a station claim stays in the home zone.
- **#292:** the flat rural deed, folded into PD3's follow-up.

## 8. Work packages

| Issue | Part | Depends on |
|---|---|---|
| [#420](https://github.com/chrischaps/TheCapitol/issues/420) | This plan | |
| [#423](https://github.com/chrischaps/TheCapitol/issues/423) | Rural: about 400 farms of about 35k to 40k | #420 |
| [#422](https://github.com/chrischaps/TheCapitol/issues/422) | Suburban: about 1200 lots, wider and less triangular | #420 |
| [#421](https://github.com/chrischaps/TheCapitol/issues/421) | Urban: a new grid with about 600 lots of about 80 × 100 | #420 |

PD2's capacity splits (#424) land with whichever part lands first, once Chris has decided. Until then, each part keeps today's `size_category`.
