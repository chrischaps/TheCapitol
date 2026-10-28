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
| suburban | r1800–3000 | 18.1M | 1905 | 12.8M (71%) | 6.7k | 3 | 22% (6% by `sim::lot`'s own rule, #422) | large, 16 |
| rural | r3000–4600 | 38.2M | 117 | 13.9M (36%) | 117k | 48 | 100% | estate, 25 |

- **Urban is smaller than #420 says.** #420's "48 × 48" is the lot's bounding box. The lots themselves are 40 of frontage by about 35 of depth, after the street insets. Every urban lot takes exactly one station.
- **The suburb is held back by shape, not area.** Its median lot is 64 × 106, which would take 4 as a box. But the rows bend, and the pies and strips narrow to a point, so only 22% of them do.
- **The capacities can't be reached.** Urban's 9 and suburban's 16 are both above what fits. Only rural's 25 is real.
- **The guild district** (72 halls of about 20k) is out of scope.

The deed is 250 in all three zones, as a live trial in the town. Before that trial, rural's flat deed was #292. *Since #292 (Chris, 2026-10-24) it's 500 in all three, in every world (PD3).*

## 3. Targets

*Superseded (2026-10-22) by `the-capitol-plots-v2.md`: about 1000 lots in each zone, sized 1:2:4. The targets below are what #421 to #423 built to.*

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

- *Decided (#292, Chris, 2026-10-24):* **no deed follows the lot's area or room.** The three home zones are of equal value on balance, each with its pros and cons (a farm's room against the walk to the market), so they cost the same: **500 each**, the middle of the old 250, 500 and 1000 (`20270101_001_equal_deeds.sql`). Tax is 10 a week on any of them. If one zone draws far more players than the others, the fix is its pros and cons, not its price. Owned plots keep what they were assessed at. The migration drops any `deed_price` knob for the three zones, since a multiple of the old price means something else against the new one. No follow-up issue is needed.

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

*As built (#421):* **three rings of blocks and 24 bearings, as planned, with a canal bank and a quay on the inside.**
- **The layout** (`models/urban/`): from the canal's water, a bank of 80, then the quay (`urb_ring_0`), then three rings of blocks, each two rows of lots 95 deep back to back between ring streets (`urb_ring_1` to `urb_ring_3`). The streets are 16 wide. The 24 bearings are 16 spokes (`urb_spoke_{j}`) and the 8 arterials. The outer ring street stands at r1713, so about 80 is left before the suburb, as the old grid left about 90.
- **Why a bank?** The urban clay is planned on a strip 18 to 48 off the canal's water and 30 clear of any road. The old grid's first row backed onto the canal, so the clay had nowhere to go but the gaps. Now the quay keeps back from the water, and the clay lies along the bank (r1035 to r1056).
- **Lots** are `urb_{ring}_{block}_{i|o}_{k}`: `i` fronts the inner ring street and `o` the outer, `k` along the row. A row is split into as many lots as fit at 80 along its **inner** edge, the narrow end of every lot in it. Measured at the middle, a row beside an arterial came out about 83 at the front but 78 at the back, and those lots took only 3. Each lot's street edge is its first two corners.
- **Laid clear of the roads:** a side on a spoke or arterial runs parallel to it, 1 past the kerb. A lot's edge at the smaller radius is a chord, and it sags toward the centre, so its corners stand out by `1 / cos(half the lot's angle)`. So `finish_lots` trims and drops none.
- **The numbers:** 520 lots, area median 8.6k (p5 7.8k, p95 9.8k), width median 90 (least 81), depth 93. **Every lot takes 4.** Lots cover 64% of the ring from the water to r1800, which is 4.48M, against 4.3M before. The grid has no seed, so it's the same on every seed. `urban::tests::the_urban_grid_meets_its_targets` holds 500 to 700 lots, a 7k to 9k median and ≥ 90% taking 4, with none trimmed or dropped and a clean street audit. `backend/tools/urban-plan.ps1` draws it.
- **Why 520, not 600?** At 76 on the narrow end there are 560, but only 76% take 4. 16 bearings make 528 at 80, or 576 at 77 with lots narrower than 80, and their blocks are 680 long. Depth 100 makes 520 of 9.2k, over the target. So 24 bearings, 80 and 95 it is.
- **Welded to the arterials** (since [#436](https://github.com/chrischaps/TheCapitol/issues/436)). Each ring street has a vertex on each arterial's centre line, and the weld puts the same point into the arterial, 6 to each. When #421 tried it, 137 farms and suburban lots moved. By #436 the farm walk and the suburb's walker both stepped by arc length and their road index read only shapes, so nothing moved past 4e-12: the world's version changed for the arterials' new points alone, with no migration. `a_collinear_vertex_on_an_arterial_moves_no_lot` holds it, and `urban::metrics` audits the grid with the arterials.
- **The suburb's edge moved a little.** The old spokes ran on to r1800, into the suburb's first row. The new ones stop at the outer ring street, so 123 suburban lots on 19 streets at the suburb's inner edge grow or re-space, and one is gone. Startup syncs with `OnOwned::Keep`, so an owned one keeps its shape. The farms, the arterials and the water didn't move.
- **The flip:** the old `urban_plot_` lots are in `RETIRED_LOTS`, and the new ids differ, so no row needs renaming: startup releases an owned old lot, refunds the deed and gives its stations back. A saved `world_config`'s old urban keys (`urban_radial_spacing` and the rest) are skipped by serde, so it builds the new grid. The planned nodes the grid moves (the urban clay, and the suburb's branches and clay planned after it, 131 in all) are re-planned (`post-content/20261207_001_urban_grid_nodes.sql`). No seeded grass moves. Pin `9d6cec09d529c7dd`.
- **Flint:** an urban house stands across its lot's front, 2.8 to 3.6 m deep, and 7 in 10 lots have a workshop at the back of the yard: a flat-roofed shed of the house's brick with wide timber doors.
- **Capacity:** at 8.6k the lots are `large` (16) by today's `size_category`, though 16 stations don't fit on one at 50 spacing. #424 settles it.

### 5.2 Suburban (#422): wider and fewer

Today: five rows of lots about 106 deep in the 1200 band, frontage 64 (`suburb/lots.rs`), 1905 lots, many near-triangular strips that take 2 or 3 stations (#305).
- **Frontage 64 → about 100.** That's the lever. It drops the count by about a third (1905 × 64 / 100 ≈ 1220), takes the median lot from 6.7k to about 10.6k at today's depth of 106, and widens the narrow end of a strip.
- **Rows stay at five.** Four rows of about 140 would make the lots longer, not wider, and give back the parks' share.
- **Pies and combs:** fewer pie lots round a bulb (2 to 4, by area), and a looser comb in pattern (b), so the loop patterns stop making slivers.
- **Targets:** `the-capitol-suburb-v2.md` §4.4 takes the new count and area, and a share-that-takes-4 row.

*As built (#422):* **frontage 100 and 2 or 3 pies, as planned; the comb left alone.**
- **The count:** 1162 to 1210 lots on the five seeds (1192 in the default world), area median 10.2k to 10.4k, width about 100, depth about 105.
- **Take 4:** 89 to 93%. Counted with `models::station_room`, a copy of `sim::lot::takes`. By that exact rule the old suburb took 4 on only 5 to 7% of lots, not §2's 22%, whose packing was looser.
- **The pies were the hard part.** A pie narrows to its bulb, so one under a quarter of the round takes 3 at most. Fewer, wider pies were then dropped, because their fronts cut the bulb's neck. Three changes fixed that: the count is the one that keeps the most ground once each pie is clear of the roads; stem lots stop 30 short of the bulb, not 20; and a pie's front may push out 40, not 12. Now 90% of the pies take 4.
- **The comb:** a looser comb in (b) wasn't needed once the pies held.
- **Built share:** lots cover 67 to 69% of the ring (71% before). The rest went to parks (green 18 to 19%), and the streets are the same.
- **The flip:** old rows are renamed `oldsub_…` and retired (PD4). The suburb's branches and clay are re-planned (677 nodes, with the rural branches planned after them). No seeded grass moves, and nothing outside the suburb moved. Pin `bf5dce48376e59c0`.
- **The suburb plan** has the detail: `the-capitol-suburb-v2.md` §4.2 (#422 note) and §4.4.

### 5.3 Rural (#423): smaller farms, the same lanes

Before: 117 farms with frontage U[380, 500], field gap U[240, 440] and depth U[280, 380] clipped by the bands (`RuralParams`).

*Planned:* a back row of farms on drives, or failing that a third road between the mains.

*As built (#423):* **long lots, neither of those.**
- **The draws:** frontage U[120, 160], field gap U[50, 100] and depth U[230, 310]. Farms are about 140 × 260, narrower and closer along the same roads.
- **The count:** 361 to 378 farms of about 36k on the four seeds. The built share stays 34 to 35%. The roads, the water and every other lot are unchanged.
- **Why not front farms alone at about 210 × 190?** They reached only 251 to 290.
- **Why not back farms?** Each needs a drive across the farm in front of it, and Flint draws a drive a fixed 26 out from the lot's street edge (`houses::DRIVE_OUT`).
- **The character:** it holds. The long lot is concession country's own shape. The fields lie behind the farms now, where they lay between them before.
- **The ids** keep `farm_…`. The migration renames the old rows `oldfarm_…`, and `oldfarm_` is in `RETIRED_LOTS` (PD4).
- **Room:** the median farm takes 15 stations at 50 spacing. The least, a junction wedge of about 16k, takes 8. All are `estate` (25); #424 settles that.
- **The rural plan** has the full table: `the-capitol-rural.md` §5 and RD6's #423 note.

## 6. Verification

For each part:
- Its targets test, over the four seeds, and its plan drawing (`suburb-plan.ps1`, `rural-plan.ps1` and `urban-plan.ps1`).
- `scripts/ci.sh full` before landing: the parts are migrations and generation.
- The Flint client against the new world: houses, plots, grass and the pasture. Take captures in `flint-client/renders/`. **[H]** Each zone should read the way §1 says: a yard by the market, a garden on a street, a farm on a lane.
- A town run after all three: citizens settle, and the share of a 640 roster that finds a home goes up from today's.

## 7. Related

- **#417:** tighter urban spacing, on top of a lot that already takes 4.
- **#418:** floors on urban lots, the route to capacity 9 (`needs-decision`).
- **#416 (landed):** urban homes for merchants and some crafters. Gatherers and builders can live urban once lots take 4.
- **#419 (landed):** a station claim stays in the home zone.
- **#292:** the flat rural deed, folded into PD3's follow-up. Decided: every home deed is 500 (PD3).

## 8. Work packages

| Issue | Part | Depends on |
|---|---|---|
| [#420](https://github.com/chrischaps/TheCapitol/issues/420) | This plan | |
| [#423](https://github.com/chrischaps/TheCapitol/issues/423) | Rural: about 400 farms of about 35k to 40k. *Done:* 361 to 378 long lots of about 36k (§5.3) | #420 |
| [#422](https://github.com/chrischaps/TheCapitol/issues/422) | Suburban: about 1200 lots, wider and less triangular. *Done:* 1162 to 1210 lots of about 10.3k, 89 to 93% taking 4 (§5.2) | #420 |
| [#421](https://github.com/chrischaps/TheCapitol/issues/421) | Urban: a new grid with about 600 lots of about 80 × 100. *Done:* 520 lots of about 8.6k, every one taking 4 (§5.1) | #420 |

PD2's capacity splits (#424) land with whichever part lands first, once Chris has decided. Until then, each part keeps today's `size_category`.
