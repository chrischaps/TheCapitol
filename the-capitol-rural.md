# The Capitol: Rural v2, Farms on Lanes

*Plan of record, 2026-10-20. Issues are labelled `rural`. The plan is #282 (R0), and the work runs from #284 to #291. It absorbs #223. Widened the same day to r3000 to r4600, with the world grown to 13600 (RD10, #307 to #310).*

This doc is the "Read first" for every `rural` issue. Keep it true: when a decision changes, update it in the same commit.

## 1. Vision

You arrive on the south arterial at r3350, with the tower ahead at the end of the road. The countryside lies either side of you: big farms set back behind a verge, with fields, hedges and copses between them. A lane leaves the arterial and runs off between the farms, straight as a surveyor drew it in one sector and winding round a pond in the next. Every sector of the ring is countryside like this. There are no empty wedges and no crowded corners.

The rural ring sits between the suburb's packed post-war lots and the open wilds. It should feel like the step between them: owned land, but much more open than the suburb. The fields between the farms aren't left over. They're the commons, where the rural trees, branches and outcrops grow (E2.3), so a new player's first gathering is done among the farms.

## 2. Where things stood (2026-10-20)

- **The generator.** `TerrainConfig::generate_rural_paths_with_params` (`backend/src/models/terrain.rs:1041-1086`) lays 12 radial Catmull-Rom trails, `trail_0` to `trail_11`, 6 wide, from r3000 to r3800.
  - Trail *i* starts at `i·30°` plus `rand·0.3` rad. So it's always pushed 0 to 17° the same way, never back.
  - It wobbles up to ±8.6° in the middle of the ring.
  - It uses its own LCG on `seed + 12345`, not `geom::stream`, so it shares the world seed with the suburb.
- **The lots.** `generate_rural_plots` (`terrain.rs:1346-1461`) drops a 200×150 lot every 200 along each trail, on each side.
  - It keeps a lot if its corners are in the ring, it touches no other road, and it overlaps no earlier lot.
  - Ids are `rural_plot_{road}_{L|R}_{n}`.
  - After `finish_lots`, the default world has 52 (of 54 laid), at most 29k each and 25k on average (estates by `plot_sync::size_category`): `finish_lots` trims about 60% of them and drops 2.
- **Why it clumps.** Trails every 30° and arterials every 45° line up every 90°. Because the push only goes one way:
  - trails 0, 3, 6 and 9 always start 0 to 17° past a cardinal arterial;
  - trails 1, 4, 7 and 10 start within about 15° of a diagonal one.

  So 8 of the 12 are born beside an arterial. A 6-wide trail next to or across a 60-wide arterial disappears, and it loses its lots to the touch test. In the default world:
  - trails 0, 4 and 6 cross their arterials twice;
  - trail 1 sits on the 45° arterial at r3400;
  - trail 3 runs 230 to 380 beside the spawn road;
  - trail 10 starts 3 from the 315° arterial's centreline.

  What's left is one or two visible trails per quadrant, with empty wedges from 96° to 151° and from 154° to 211°.
- **Nothing joins.** The trails weld to nothing (`weld_junctions` only runs on suburban and `wild_*` roads) and end loose at both radii. Flint's walk graph gets around this with `JOIN = 600` (`capitol-core/src/paths.rs:15`).
- **Smaller defects.**
  - Line 1382 averages two directions without allowing for the wrap at ±180°, so on trail 6 a lot can flip to the wrong side.
  - Lot depth is measured from the centreline, and the straight front edge of a lot on a curve bulges into its own trail.
- **The water.** Since #222 the wilds' water runs onto the plain, down to r3150. The rural lots don't know it, so some have a stream or a pond across them (#223).
- **The docs** say only "Rural Zone: large plots" (GDD §3, milestones). 200×150 is a code default and nothing more. The deed is a flat 250 (GDD:734).
- **The baseline** (R1, [#284](https://github.com/chrischaps/TheCapitol/issues/284)), over seeds 42, 7, 1234 and 99999. The full table is in §5.
  - **Lots:** 52, 70, 71 and 60, at 24k to 25k on average. The ring is 8 to 10% built.
  - **Clumping:** every seed has at least one half-sector with nothing built, and none has one above 22%. The CV of the half-sectors' shares is 0.64 to 0.79.
  - **Roads:** all 24 trail ends are loose on every seed, and 1 to 4 trails cross an arterial unjoined. The audit counts one per trail and arterial, not one per crossing.
  - **Water:** 1 to 10 lots have a stream or a pond across them, and the plain carries about 53 courses and 12 lakes in the ring (#223).
  - **Gaps:** trimmed lots end up almost touching. The least gap is under 0.25 on every seed, and the median gap to the nearest lot is 13 to 20.

## 3. Decisions

Chris set the shape on 2026-10-20:
- sparser than the suburb, with much larger lots and more space between them;
- some straight roads and some winding;
- density even across the ring.

He picked *farms* over smallholdings and estates, and chose to have farms front the arterials too. RD1 to RD9 work that out.

### RD1. The sector is the unit ([R3 #286](https://github.com/chrischaps/TheCapitol/issues/286))

The 8 arterials cut the ring into 8 sectors of 45°. At r3800, the middle of the widened band (RD10), each is about 2980 of arc, 1600 deep, and about 4.8M in area. (Before RD10 it was 2670 of arc at r3400, 800 deep and about 2.1M.) Every sector gets the same budget and draws from its own SplitMix64 stream, `geom::stream(seed, &[RURAL, s])` (`geom.rs:276`).
- **Even density comes from the structure,** so nothing has to correct for clumps afterwards.
- **A tweak in one sector never re-ids the farms in another,** which is what suburb v2 learned: lots are owned plots.

### RD2. A deck of characters, not a coin per sector ([R3 #286](https://github.com/chrischaps/TheCapitol/issues/286))

The sectors' characters are a seeded shuffle of 4 *concession* and 4 *winding*, drawn from `stream(seed, &[RURAL])`. A coin per sector could give 7 winding sectors and 1 straight. The deck keeps the mix even, and the shuffle still varies which sector gets which.

### RD3. Concession sectors: straight roads ([R3 #286](https://github.com/chrischaps/TheCapitol/issues/286))

- **The main road** runs from one arterial to the other at ρ ∈ [3330, 3470].
  - It's two straight legs, kinked at the sector's bisector, a turn of about 22.5°.
  - A chord over 22.5° at r3400 sags only about 65 from the arc, so the depth either side stays nearly even.
- **The jog.** Sometimes there's a *correction-line jog* at the kink, as surveyed country roads have where the grid is corrected for the curve of the earth. It's two bends of 40° or less, within `street_plan::MAX_TURN_DEG` (45).
- **Side roads.** 1 or 2 straight radial side roads leave the main road at T junctions and end at a farm gate.

### RD4. Winding sectors: lanes ([R3 #286](https://github.com/chrischaps/TheCapitol/issues/286))

- **The main lane** runs from arterial to arterial. Its radius is ρ(θ) = ρ0 + two low-frequency sines, with a total amplitude of 150 or less, giving 2 or 3 bends across the sector, plus a gentle wander in angle.
  - It's sampled through `street_plan::centripetal_spline`, promoted from test-only, then resampled at about 20.
- **One wandering spur** leaves the lane, turning 8° or less per step, and ends at a farm gate.

### RD5. Every road joins ([R3 #286](https://github.com/chrischaps/TheCapitol/issues/286))

- **Ids and type.** Rural roads are `lane_{s}_{k}`, `RoadType::Trail` and 8 wide, so there's no wire change.
- **The weld.** `street_plan::weld_junctions` runs over `lane_*` and the arterials, so every lane meets an arterial at a shared vertex. `street_plan::audit` holds over them. A loose end is allowed only at a farm gate, where a track ends at a farm.
  - *As built (#286):* the weld picks the lanes only. Their ends land on the arterials, which gain a vertex there, and the side roads and spurs start on a vertex of their main road. Picking the arterials too would also weld their far ends onto the guild ring.
  - *As built:* a farm gate is the planned end of a side road or spur (since RW3, of the road out from the outer concession road, or of the spur), and an end counts as at a gate only if it's exactly there. `rural::plan` returns the gates, and the metrics re-plan to find them. R4 can narrow this further, to a gate a farm actually fronts.
- **The edge.** Lanes stay inside about 20 of the rural edge (r4580 since RD10). The wild trails' weld picks up any road within `trails::CELL` of the rural edge (`trails.rs:222-226`).

### RD6. Farms ([R4 #287](https://github.com/chrischaps/TheCapitol/issues/287))

- **Laying them.** Walk each fronted road, on both sides, with alternating draws:
  - frontage U[300, 420], then a **field gap** U[120, 260];
  - depth U[280, 380], clipped by the region and the other roads.
- **Size and spacing.** A farm is about 120k, four times today's lot. Farms keep 40 or more apart side to side.
- **Ids** are `farm_{s}_{road}_{L|R}_{k}`, numbered before the overlap pass, so a dropped farm doesn't re-id the rest.
  - The new prefix lets the flip's migration tell old lots (`rural_plot_%`) from new ones.
- **The region.** The ring inset by 20 at r3000 and at the rural edge (r4600 since RD10), and the sector's side lines.
- **Along the arterials** (Chris, 2026-10-20). Each sector owns the side of each arterial that faces it. Farms there stand behind a verge of about 30 past the 60-wide road, clear of the lane junctions. So the spawn road has farms along it.
- **Reuse, not rewrite.** The lot geometry is `suburb/lots.rs`'s road index, edge rays, overlap pass and shrink-to-fit, lifted to work over any `Region` and a `LotParams` ([R2 #285](https://github.com/chrischaps/TheCapitol/issues/285)). It is `models/lot_geometry.rs` now, and `place` keeps farms `lot_gap` (40) apart.
- *As built (#287):* `rural/farms.rs`. A farm is a trapezoid on its road's chord: the front runs between the two points its frontage spans, its sides run square to that, and on a bend's outside the front is pushed out (up to 60) until its own road clears it. The verge is 30 off an arterial's edge and 10 off a lane's, and the region is cut along each arterial at its verge, so the lanes' farms stand clear of the junctions. Each side of each road draws from its own stream (`[RURAL, s, FARMS, road, side]`). Where no farm fits, its frontage is cut 20 at a time down to 240, and then the walk slides on 10. A farm the water or a road runs into between its sides is pulled back short of it there, down to 140 deep, so the walk goes round a stream rather than leave the farm for the overlap pass to drop. The overlap pass takes the arterials' farms first, then the main roads', then the rest, so the spawn road has farms along it. Arterial farms are `farm_{s}_a0_…` (the arterial at the sector's `from`) and `farm_{s}_a1_…` (at its `to`). Farms back to back meet half way, 20 short each.

### RD7. The balance pass and the water ([R4 #287](https://github.com/chrischaps/TheCapitol/issues/287), closes [#223](https://github.com/chrischaps/TheCapitol/issues/223))

- **Provisional farms first.** The roads are planned, then provisional farms are laid on them, ignoring the water.
- **Filling holes.** The built ground goes onto `suburb/parks.rs`'s raster, and the pass finds the largest open disc in each of the 16 half-sectors. While a half-sector is under its minimum built share and a hole of radius over about 380 remains, it adds a spur toward the hole from the nearest road: straight in a concession sector, winding in a winding one. At most 3 per sector.
- **Then the water.** The roads are final, so the build order stays roads, then water, then lots (`terrain.rs:707-733`), and the plain's water still banks round every lane (`hydrology::BERM`).
- **Then the real farms,** clear of `rivers` and `streams` (capsules, with a width at each point) and `lakes` (even-odd rings), each with a bank margin.
- **Then `lot_clearance::finish_lots`,** still last.
- **The cost.** A farm lost to a pond leaves a field. A road can't move for the water, but the water already moves for the road.
- *As built (#287):* `rural/balance.rs`, on a 10-square raster of each sector with the parks' chamfer (`parks::chamfer_with`). A half-sector wants a spur under 25% built (`BUILT_MIN`: the margin over §5's 20% is for what the water takes) with a hole over 380 across in radius. A spur (`roads::spur_to`) leaves the nearest point of any of the sector's roads, runs on past the hole's centre by half its radius, keeps the spur's clearances, and ends at a farm gate; spurs are `lane_{s}_{k}` after the sector's own. `rural::plan_balanced` adds them before the weld, and `generate_roads_and_rural_plan` hands the metrics the gates. **On seeds 0 to 39 the pass never fires:** the roads already spread the farms (dry, every half-sector is 24 to 37% built, and no hole is over about 340). What lowers a half-sector is the water afterwards, which the walk now goes round. It's kept as the plan's guard, and its test takes a sector's outer road away to make a hole.

### RD8. Farms are pasture in Flint ([R6 #289](https://github.com/chrischaps/TheCapitol/issues/289))

**The problem.** Flint clears the grass from every lot (`world/grass.rs:557-560`), and `the_rural_ring_and_the_wilds_edge_are_mostly_grass` (894) needs over 85% of the ring to be grass. At about 35% built the ring would fail that, and it would look wrong too: a farm is mostly grass.

**The change.**
- In the rural zone the grass stays on the lot except round the farmyard: the house, the barn, and a drive to the lane.
- The farmstead (`world/houses.rs`) stands back on that drive.
- `plots.rs`'s `LOT` bound (635) grows to cover the widest farm.

**When.** It works for both layouts, so it lands before the flip.

### RD9. The deed stays flat until Chris decides ([#292](https://github.com/chrischaps/TheCapitol/issues/292))

The rural deed is a flat 250, and the tax is flat too. A farm is four times the land, so after the flip 250 buys four times as much. That may be right, since the countryside is the first rung next to the spawn, but it's Chris's call. #292 is `needs-decision`. Nothing in this plan changes prices.

### RD10. The ring is two roads deep, and the world grows (Chris, 2026-10-20; [#307](https://github.com/chrischaps/TheCapitol/issues/307))

**The problem.** At 800 deep, one road with farms on both sides fills the band: two depths of 280 to 380, plus the verges. There's no room for a second road across, so every sector is one line of farms, and the countryside reads as a strip.

**The change.** The rural ring runs **r3000 to r4600, 1600 deep**: two roads across, each with a band about 800 deep.
- **The world grows; the wilds don't shrink.** Chris picked this over shrinking the wilds to 1400 deep. The wilds keep their 2200 depth and move out to r4600 to r6800. `WORLD_SIZE` goes from 12000 to 13600, and the centre moves to (6800, 6800), as it moved in W1 (WD1 in the wilds plan). One migration moves every stored position by +800 ([RW1 #308](https://github.com/chrischaps/TheCapitol/issues/308)).
- **The wilds are regenerated.** The height field is keyed on the offset from the centre, and its radii move out with the edge: the foothills start at r4700 and the ranges reach full height at r6400. So the ground, the water, the wild trails and the wild nodes past r4600 are all new. The plain the rivers soak into is twice as deep, and they still never reach inside r3150.
- **Concession sectors get two main roads.** The inner one stays where R3 put it (kink at ρ ≈ 3400), and an outer one has its kink at ρ ≈ 4200, where the chord sags about 80. *Concession side roads* join the two at T junctions, and at most one more runs out from the outer road to a gate ([RW3 #310](https://github.com/chrischaps/TheCapitol/issues/310)).
- **Winding sectors get two lanes, linked.** Chris picked this over one lane with more spurs. An inner lane runs about ρ0 3400 and an outer one about ρ0 4200, at least 500 apart, joined by one winding *cross-lane*. The spur stays.
- **Flint follows the bigger world:** the fixture, the land mesh and `GroundField`, the grass mask, the fog, the views and the edge water ([RW2 #309](https://github.com/chrischaps/TheCapitol/issues/309)).
- **What doesn't change:** the farm sizes, gaps and verges (RD6), the deck (RD2), the half-sector checks (§5), and the spawn on the south arterial at r3350, which now stands in the inner band.

**The cost.** This is the third time the whole world has moved on purpose (after B1 and W1). Every position in a live database shifts once, so the change can't land in the middle of a citizens' soak (#257, #306). There's also more to walk: the edge of the wilds is 800 further from the tower.

## 4. The layout, step by step

### 4.1 Sectors

1. For each arterial, take its bearing, then sort them. Sector *s* runs from arterial *s* to arterial *s*+1, inset by the arterial's half-width plus the verge.
2. Deal the deck (RD2). Each sector takes its stream (RD1).

### 4.2 Roads

3. Lay the two main roads, inner and outer (RD3 or RD4, and RD10). Then lay the side roads or the cross-lane between them, and the spur.
4. Weld (RD5).

### 4.3 Farms

5. Lay provisional farms (RD6) on the lanes, side roads, spurs and arterial frontages.

### 4.4 Balance, water, and the real farms

6. Run the balance pass (RD7), adding spurs. The roads are now final and go into `generate_all_roads_with_params`.
7. Hydrology runs, as now.
8. Lay the real farms on the final roads, clear of the water. Then `finish_lots`.

## 5. Targets

R1's harness prints these, and R5's test asserts the hard ones over seeds 42, 7, 1234 and 99999. It prints the soft misses as `neighbourhood_suburb_meets_its_targets` does.

| Metric | Target | Hard? |
|---|---|---|
| Farms | 95 to 135 (45 to 65 before RD10) | yes |
| Mean farm area | 100k to 140k | soft |
| Ring built share | 30 to 40% | soft |
| Built share of each of the 16 half-sectors | 20 to 55% | yes |
| CV of the half-sectors' built shares | ≤ 0.30 | soft |
| Sectors of each character | ≥ 3 | yes |
| Main roads in a sector, arterial to arterial | ≥ 2 | yes |
| Roads joining them, in a sector | ≥ 1 | yes |
| `street_plan::audit` on rural roads | clean but for farm-gate ends | yes |
| Loose road ends off a farm gate | 0 | yes |
| Farms on water | 0 | yes |
| Gap between farms | ≥ 40 | yes |

**The baseline** (today's trails on the 800-deep ring, R1, #284; RW1 stretches them to r4600, so R5 measures it again). `backend/tools/rural-plan.ps1` prints it. ✗ marks a miss, and hard targets are in bold.

| Metric | Target | 42 | 7 | 1234 | 99999 |
|---|---|---:|---:|---:|---:|
| **Farms** (rural lots) | 45 to 65 | 52 | 70 ✗ | 71 ✗ | 60 |
| Mean farm area | 100k to 140k | 24.9k ✗ | 24.4k ✗ | 24.1k ✗ | 23.9k ✗ |
| Ring built share | 30 to 40% | 8% ✗ | 10% ✗ | 10% ✗ | 8% ✗ |
| **Half-sector built share, least** | ≥ 20% | 0 ✗ | 0 ✗ | 0 ✗ | 0 ✗ |
| **Half-sector built share, most** | ≤ 55% | 16% | 22% | 20% | 22% |
| CV of the half-sectors' shares | ≤ 0.30 | 0.64 ✗ | 0.73 ✗ | 0.76 ✗ | 0.79 ✗ |
| **Sectors of each character** | ≥ 3 | – | – | – | – |
| **Audit issues, but for loose ends** | 0 | 3 ✗ | 4 ✗ | 3 ✗ | 1 ✗ |
| Loose road ends | | 24 | 24 | 24 | 24 |
| **Loose ends off a farm gate** | 0 | 24 ✗ | 24 ✗ | 24 ✗ | 24 ✗ |
| **Farms on water** | 0 | 6 ✗ | 6 ✗ | 10 ✗ | 1 ✗ |
| **Gap between farms, least** | ≥ 40 | 0.07 ✗ | 0.16 ✗ | 0.10 ✗ | 0.22 ✗ |
| Lots trimmed by `finish_lots` | | 61% | 56% | 62% | 65% |

Built share by half-sector, 0 to 15, in %:
- 42: 11 12 3 10 12 0 10 0 5 11 10 4 16 0 9 9
- 7: 7 12 12 0 21 2 13 1 13 0 22 1 18 15 7 15
- 1234: 20 0 18 0 12 5 0 11 15 18 0 15 14 12 0 20
- 99999: 15 1 6 12 15 11 0 12 9 12 13 0 5 0 22 0

**How the harness measures these** (`backend/src/models/rural/metrics.rs`):
- **Half-sector** `h` runs from `22.5h°`, so sector `s` is half-sectors `2s` and `2s + 1`.
- **Built share** is the ground a rural lot stands on, sampled every 10.
- **Gap** is edge to edge.
- **On water** means a course's capsule (half its width either side) or a lake's rings meet the lot.
- **The audit** counts `street_plan::audit` on `trail_*` and `lane_*` roads, but for loose ends, which are counted apart. Since R3, the loose ends that aren't at a farm gate are a hard row of their own. Today's trails have no gates, so all 24 of theirs miss.
- **Sectors of each character** counts the rarer of the two in the deck, over the whole ring. The trails have no deck, so it's blank for them.
- **Main roads and the roads joining them** (RW3) are counted per sector from where the welded roads end, not from their ids: a main road has both ends on an arterial's vertex, and a road joining two has its ends on vertices of two different main roads. The report gives the fewest in any sector. Blank for the trails.
- The report has more rows: road length, trimmed and dropped shares, the least and most lot area, the median nearest gap, the closest a dry lot comes to water, and the courses and lakes in the ring.

## 6. Architecture notes

- **Where it lives.** A new module, `backend/src/models/rural/`:
  - `mod.rs` holds `RuralParams` and the entry points;
  - `sectors.rs` holds RD1 and RD2;
  - `roads.rs` holds RD3 to RD5;
  - `farms.rs` holds RD6;
  - `balance.rs` holds RD7;
  - `metrics.rs` and `render.rs` are test-only, like the suburb's.
- **Params.**
  - `RuralParams` sits inside `WorldGenParams` with serde defaults, so a saved `world_config` still loads.
  - Until the flip it carries `layout: Trails | Farms`, defaulting to `Trails`, so the pin holds while the work lands (as suburb v2's switch did, #105).
  - R8 removes the switch and the old params. The React tweak panel only needs to keep compiling.
- **Reused.**
  - `geom::stream` (SplitMix64).
  - `street_plan::{weld_junctions, resample, centripetal_spline, audit}`.
  - `models/lot_geometry.rs`, the lot geometry R2 lifted out of `suburb/lots.rs`.
  - `suburb/parks.rs`'s raster.
  - `suburb/render.rs`'s SVG approach.
  - `lot_clearance::finish_lots`.
- **The harness.** `backend/tools/rural-plan.ps1` draws the ring with roads, farms, water and half-sector shading, with `-Seed` and `-Compare`. Judge by eye first, then by the numbers, as suburb v2 did.
- **Flint.**
  - Rural roads are still `Trail`, so `roads.rs` draws them as before.
  - The farmhouse and barn are seeded by plot id, so new ids mean new houses. That's expected at the flip.

## 7. The flip ([R7 #290](https://github.com/chrischaps/TheCapitol/issues/290))

The flip is one commit, and `scripts/ci.sh full` runs before it lands. It covers:

- **The pin.** `layout` defaults to `Farms`, and the pin (`terrain.rs:1541`) is bumped with a history paragraph.
- **Owned plots.** A migration releases owned `rural_plot_%` plots, with a `deed_refund`, as `20261019_001_neighbourhood_suburb.sql` did for the suburb. Startup's sync (`OnOwned::Keep`) would otherwise keep them and hold back the farms laid over them (#119).
- **Grass.**
  - The moves come from `print_node_moves`.
  - `seeded_nodes()` (`resource.rs:157-212`) must apply a move written on the 12000 world *after* W1's shift. Its comment says so, but the code doesn't do it yet.
- **E2.3's nodes.**
  - The node plan steers round lots and roads, so it drifts once.
  - A post-content migration re-plans the rural nodes, and any wild ones the shared spacing grid moves, by #225's delete-and-insert. The water-node moves come from `print_planned_node_moves`.
  - `the_migration_writes_the_default_plan` learns the new file.
- **Thresholds.** `lot_clearance.rs:415` (`rural > 40`), and any other rural counts.
- **Flint.**
  - Recapture `flint-client/fixtures/terrain.json`.
  - Fix the hard-coded rural ids in `flint-client/CLAUDE.md` and in capitol-core's tests.
  - See whether `paths.rs`'s `JOIN` (#211) can shrink now that the lanes weld. If it can, cut an issue.
- **Docs.** Root `CLAUDE.md`'s World Geography, `the-capitol-wilds.md` (#223's note), and this plan's as-built notes.

## 8. Work packages

| WP | Issue | What | Size | Depends on |
|---|---|---|---|---|
| R0 | [#282](https://github.com/chrischaps/TheCapitol/issues/282) | This plan, the issues, and #223 brought in | S | – |
| R1 | [#284](https://github.com/chrischaps/TheCapitol/issues/284) | Harness: render, metrics, `rural-plan.ps1`, baseline of today's trails. *As built:* `rural/metrics.rs` and `render.rs` (test-only), `rural_plan_report`, and `-Seed`, `-Seeds`, `-Sector`, `-Out` and `-Compare`. It shares `suburb::metrics`'s scope and checks and `suburb::render`'s helpers and page. The baseline is in §2 and §5 | M | – |
| R2 | [#285](https://github.com/chrischaps/TheCapitol/issues/285) | Shared lot geometry: `lots.rs` over any `Region` + `LotParams`, aware of water. Suburb unchanged under the pin. *As built:* `models/lot_geometry.rs` holds `World` (roads, fronted sides, and `with_water(courses, lakes, bank)`), `Region` (`ring`, cut by `ccw_of`/`cw_of` side lines, or `polygon`), `Ray`, `push_fronts`, the ray march `back`/`back_from`, `Candidate`, and `place`/`shrink_to_fit`. `LotParams` has the frontage and depths, the front, edge and rear gaps, `lot_gap`, `min_shrink` and `min_area`. A course is a capsule as wide as the wider point of each stretch, as R1 measures it, and a lake its even-odd rings, each plus `bank`. A ray stops at a bank as at a road without lots, so a lot backs onto water, and `place` drops one the water runs through. `suburb/lots.rs` keeps its walker, pies and ids, and calls it through `lot_params` and `region_of`. The pin and `neighbourhood_suburb_meets_its_targets` print the same | M | – |
| R3 | [#286](https://github.com/chrischaps/TheCapitol/issues/286) | Sectors and roads behind `layout = Farms`: the deck, concession roads, winding lanes, side roads, spurs, weld, audit. *As built:* `rural/{sectors,roads}.rs`, and `RuralParams { layout, lane_width }` as `WorldGenParams::rural`. The sectors are read off the arterials' bearings. Concession: a kink at ρ in [3355, 3445] with ends within 25 of it, a jog half the time (16 to 24° across the road, 110 to 170 long, so the bends are 40° or less), and one side road (both ways at random) or two (opposite ways), to gates 80 to 140 from the ring's edge. Winding: two sines (90 to 150 in all) on 13 controls, wandering a fifth of a step in bearing. The spur turns 8° or less a step, pulled back toward radial, stops clear of the edges, the arterials and its own lane, and is redrawn if it's under 160 long. Under `Farms` there are no rural lots until R4. `rural-plan.ps1 -Layout farms` draws the lanes, the gates and each sector's character. Audit clean and 4 of each character on the four seeds and on seeds 0 to 39 | L | R1 |
| RW0 | [#307](https://github.com/chrischaps/TheCapitol/issues/307) | RD10 into this plan, the wilds plan and `CLAUDE.md`, and the issues | S | – |
| RW1 | [#308](https://github.com/chrischaps/TheCapitol/issues/308) | The world grows to 13600: rural to r4600, the +800 shift, the wilds regenerated (land, water, trails, nodes), the pin. *As built:* migration `20261122_001_rural_widens.sql` moves players, nodes, the node seeds (while a fresh database still has them), plots and stations, and a saved `world_config`'s land radii. `LandParams` defaults to foothills from r4700 and ranges full height at r6400; the trails' valley heads and passes are now measured from the rural edge (`HEAD_RADIUS` = edge + 1400, `PASS_MIN_RADIUS` = edge + 900). On the wider plain three things the old world never hit needed fixing. The plain's tilt in the routing field is now measured from its edge (`PLAIN_EDGE_RISE` 7.5, the same field on the 12000 world): measured from the inner rim, it stood twice as high and dammed the water off the plain (5 courses ended there, against 30). A course out of a lake now rises at least `SOURCE_DEPTH` (0.5) under the level, because an outlet cell only just under it could fall outside the traced shore. And a trail never crosses itself (`Grid::shape`). The node planner keeps every zone's nodes off the wild water (`plan_with`), and `post-content/20261122_001_rural_widens_nodes.sql` replaces every planned row with the new plan (13110 nodes), with no move migration after it. The water now ends on the plain at r3900 to r4600 (37 to 47 ends a seed), so the outer band is wet and the inner one dry. 33 suburban lots by the rural edge move by up to 2.4, keeping their ids (the pin's history has the rest). The old trails stretch to r4600, so there are 121 rural lots until R7. R3's side roads now run out to gates near r4500 until RW3. The Flint client follows, mechanically: the fixture (from `capture_terrain_fixture`), the land field grown to 26 chunks (to r8320, past `land::OUTER`), the grass mask and blade grid scaled to keep their resolution per metre (3480 and 2320), and the wilds' wind and mist measured from the rural edge. How it looks, and what it costs, is RW2's. Past the edge, a run-on tributary can meet a river below its surface ([#312](https://github.com/chrischaps/TheCapitol/issues/312)) | L | RW0 |
| RW2 | [#309](https://github.com/chrischaps/TheCapitol/issues/309) | Flint for the 13600 world: fixture, land mesh, grass, fog, views, edge water, capitol-core | M | RW1 |
| RW3 | [#310](https://github.com/chrischaps/TheCapitol/issues/310) | Two roads across: concession pairs with side roads between them, winding lanes with a cross-lane, the edge at r4580. *As built:* `lane_{s}_0` is the inner main road and `lane_{s}_1` the outer, then the roads between them, then the road to a gate. The bands are measured from the inner edge: inner [330, 470], outer [1130, 1270]. Concession: each main road jogs on its own coin. One or two radial side roads (even odds) sit in the halves either side of the kink. A gate road from the outer road comes half the time: in the other half from a lone side road, or at the far end of a shared half from the side road there, so no two are within about 400 of each other. Its gate is 80 to 140 inside the rural edge. Winding: the outer lane is redrawn until it's 500 or more from the inner one, its sines let out less every eight draws, down to none (the bands alone keep 660). The cross-lane leaves a vertex of the inner lane 0.3 to 0.7 across and steers for the outer lane, drawing its turns under 7°. The point where it meets goes into the outer lane as a vertex: left to the weld's 4-wide snap, a short last step swung through a 49° turn (seed 6). The spur leaves either lane and keeps 40 off the sector's other roads. Two hard rows in the harness count each sector's main roads and the roads joining them. Clean on the four seeds and on seeds 0 to 39 | M | RW1, R3 |
| R4 | [#287](https://github.com/chrischaps/TheCapitol/issues/287) | Farms on lanes and arterials, field gaps, the balance pass, clear of the water. Closes #223. *As built:* `rural/farms.rs` and `rural/balance.rs` (RD6's and RD7's as-built notes). Under `Farms` on the four seeds: 109 to 117 farms, 29 to 30% built, half-sectors 23 to 36% with a CV of 0.08 to 0.10, none on the water, at least 40 apart, and `finish_lots` trims none. Mean farm area is 99k to 105k, under the soft 100k on two seeds, and the ring's built share is just under the soft 30% on three: R5's to tune. On seeds 0 to 39 the least half-sector is 19% on seeds 4 and 39, and at least 20% on the rest. The metrics' segment test took two farms' fronts on opposite arterials, which are on one line, as touching; it now treats segments on one line by their distance | L | R2, R3, RW3 |
| R5 | [#288](https://github.com/chrischaps/TheCapitol/issues/288) | Tune to §5's targets over four seeds; the targets test | M | R4 |
| R6 | [#289](https://github.com/chrischaps/TheCapitol/issues/289) | Flint: farms are pasture, the farmstead set back on a drive, `LOT` bound | M | R4, RW2 |
| R7 | [#290](https://github.com/chrischaps/TheCapitol/issues/290) | The flip (§7) | L | R5, R6 |
| R8 | [#291](https://github.com/chrischaps/TheCapitol/issues/291) | Remove the old trails, their params and the switch | S | R7 |

Follow-up: [#292](https://github.com/chrischaps/TheCapitol/issues/292), the rural deed's price for four times the land (`needs-decision`).

## 9. Risks

- **The water depends on the roads.** New lanes move the plain's ponds and streams, which moves the nodes near them. The flip's node tests catch it, and `print_planned_node_moves` writes the rows.
- **The wild nodes may drift.** The wilds are planned last on the same spacing grid as the rural nodes (`node_spawn.rs:171, 229-230`).
- **#210** (rural iron veins, `needs-decision`) drifts the same node plan. Settle it before the flip and fold it in, or take a second migration after.
- **Fewer, bigger plots.** About 115 farms replace 52 lots, on twice the land (RD10). Citizens' sim (`sim/run.rs:33`) puts rural first in `PLOT_ZONES`, so check its claims still find land.
- **Audit dead ends.** `street_plan::audit` flags every loose end. Rural tests need a farm-gate exemption that's narrow: an end inside a farm's frontage reach, not any end.

- **The world shift (RD10) could miss a stored position.** RW1 lists every table that has one, including those added since W1 (citizens, the bazaar, impound), and checks a copy of a live database against the tower.
- **The regenerated wilds could lose what W2 to W11 tuned.** The relief, the water's soak, the habitats and the far-land fade were all tuned at r3800 to r6000. RW1 and RW2 carry Chris's [H] checks of the relief and the views.

## 10. Verification

- **Each WP's own Verify,** then `scripts/ci.sh` before `land`. Run `scripts/ci.sh full` before the flip.
- **Renders** from `rural-plan.ps1` go in each closing comment. Chris judges R5's.
- **At the flip:** on a copy of a live database, owned rural plots are refunded and released, the farms are laid, and no node stands on a farm or in water. A Flint shot from the spawn shows farms along the south arterial.
