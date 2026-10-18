# The Capitol: Suburban Layout v2

*Plan, 2026-10-17. Tracking issue #103; work packages #104 to #113.*

The suburb (annulus r 1800–3000 since #117, 1600–2400 before; cut by 8 arterials) is to be rebuilt as post-war neighbourhoods. This doc is the "Read first" for every `suburb-v2` issue. Keep it true: if a decision changes, update it in the same commit.

## 1. Why

`flint-client/renders/showcase/suburb.png` shows the problem:
- one snaking collector with lollipops stuck on it;
- lots in ones and twos, with wide empty ground between them.

Post-war suburbs look different:
- **dense:** every street is lined on both sides with lots;
- **two lots apart:** local streets sit two lot depths apart, so each block is a strip of lots backing onto each other;
- **gently curved:** streets curve gently and run nearly parallel;
- **loops and lollipops:** loop collectors carry cul-de-sacs with pie-shaped lots round each bulb;
- **T junctions:** almost every junction is a T;
- **closed off:** each neighbourhood has few ways in;
- **green edges:** lots back onto the arterials behind a green strip, and leftover ground becomes a park.

**Root causes in the Legacy generator:**
- **Streets come first and are rejection-sampled** (`generate_local_streets`, terrain.rs:1320). Street spacing is a 100-unit minimum, not "two lot depths plus a road", so blocks never close up.
- **Collectors are random wobble** (terrain.rs:1150), not designed curves.
- **Lots are fixed 100-deep rectangles on an averaged tangent** (`generate_plots_along_road`, terrain.rs:2585), and the first road to claim ground wins. Curves, bulbs and corners therefore get no lots.
- **The proportions are off:** lots are 80×92 and streets are 10 wide, where the references give about 60×110 and 16.

## 2. References and research

**Images.** They are kept locally in `reference/` and not committed, because they are third-party images and the repo is public:

- **`aerial-suburban-layout-stockcake.jpg`, warped parallels.** An aerial photo of a superblock: straight arterials meet at a cross, and each quadrant is filled with long, gently curving near-parallel streets two lots apart, joined by short T cross-streets. It has almost no 4-ways and no empty ground.
- **`dafb5b4c…jpg`, loops and lollipops.** A planner's illustration:
  - curvilinear loops with short cul-de-sacs;
  - every block two lots deep, with lots fanning round curves and bulbs;
  - a tree-lined green strip and a creek between the neighbourhood and the arterial;
  - a denser grid quarter beside it, for contrast.
- **`Suburbs-layout-…png`, Geelong, Australia.** A traditional grid (Belmont), a loop and cul-de-sac suburb (Grovedale), and a loop and curvilinear suburb (Waurn Ponds).
- **`street_layouts.jpg`.** Six diagram types: traditional grid, fused grid, TND I and II, and the post-war types:
  - **PW1:** a branching tree of dead ends and loops;
  - **PW2:** a central loop collector with cul-de-sacs radiating from it.

**Research:**
- **FHA guidance (1936–41).** The FHA's 1938 bulletin *Planning Profitable Neighborhoods* (Technical Bulletin 7), and the standards around it, asked for:
  - long curvilinear blocks that follow the land;
  - T junctions and cul-de-sacs instead of 4-ways and sharp corners;
  - a hierarchy of local and collector streets;
  - a 50 ft right-of-way with 24–26 ft of paving.

  Sources: [TCLF](https://tclf.org/category/designed-landscape-types/fha-approved-neighborhood), [PHMC field guide](https://www.phmc.state.pa.us/portal/communities/pa-suburbs/field-guide/subdivision.html), [MDOT SHA suburbs context](https://www.roads.maryland.gov/oppen/C-1-3.pdf).
- **Southworth & Ben-Joseph,** *Streets and the Shaping of Towns and Cities* (1997), and ["Reconsidering the Cul-de-sac"](https://web.mit.edu/ebj/www/doc/culdesacs.pdf) (Access, 2003):
  - The eras run from gridiron to fragmented parallels, warped parallels, loops and lollipops, then lollipops on a stick.
  - A typical cul-de-sac is under 1,000 ft, serves up to 20 houses, and ends in a turnaround of 35–40 ft radius.
  - The ideal puts every house on a cul-de-sac or loop, and none on the collector.
  - A loop has two access points and is longer than a cul-de-sac.
- **Boeing,** ["Off the Grid… and Back Again?"](https://arxiv.org/abs/2010.04771) (2020): from 1940 to the 1990s, US street networks lost griddedness, orientation order, straightness, 4-way junctions and junction density, while dead ends and block lengths grew.
- **Procedural precedents:** Chen et al., tensor-field street modelling ([SIGGRAPH 2008](https://web.engr.oregonstate.edu/~zhange/images/street_sig08.pdf)); and hierarchical parcel and street co-generation ([Eurographics 2024](https://diglib7.eg.org/bitstream/handle/10.1111/cgf15053/v43i2_07_15053.pdf)). §4 says why neither is used directly.

## 3. Decisions (Chris, 2026-10-17)

- **Real proportions.**
  - Lots about 60 frontage × 110 depth (allowed depth 95–130), roughly today's area.
  - Local streets 16 wide, collectors 20.
  - Parallel street spacing S ≈ 2·110 + 16 ≈ 236.
- **A mix of patterns, one per neighbourhood:** warped parallels, loops and lollipops, and central loop.
- **Harness first, then work packages behind a switch:** `SuburbanGenParams::layout`, `Legacy` (default) or `Neighbourhood`, until the flip in #112. *Flipped in #112, after Chris signed off #111's images: `Neighbourhood` is the default. #113 removed Legacy and the switch.*

### Deeper suburb: 1800–3000 (#117, 2026-10-17)
At 1600–2400 every neighbourhood frame was 740 deep, so every one fitted exactly 3 rows on every seed, and pattern (a) could only vary its row ends. Chris: "all the neighbourhoods are just slight variants of the same 3 rows with some switchbacks". The suburb is now **1800–3000**, measured with the plan report's radius overrides:

| suburb | depth | rows | lot depth |
|---|---|---|---|
| 1600–2400 (before) | 740 | 3 | 115 |
| 2000–2800 (moved out only) | 740 | 3 | 115 |
| **1800–3000** | 1140 | **5** | 106 |
| 2000–3200 | 1140 | 5 | 106 |
| 1600–3000 | 1340 | 6 | 104 |

- Depth is what counts. Moving the zone outward without deepening it changes nothing.
- At 5 rows, the same end-picker makes racetracks, nested U-turns, combs of cul-de-sacs and mixed ends.
- 1800–3000 keeps the 8000 world, so no centre or saved position moves. The zones are now urban 1000–1800, suburban 1800–3000, rural 3000–3800 and wilderness 3800–4000. 2000–3200 would have squeezed the rural zone to 600.
- Spawn stays at 3350 south of the centre, and the arterials still end at 3800, now where the wilderness starts. So `SPAWN_Y` and `arterial_end_radius` no longer track the rural edge.
- At seeds 42, 7, 1234 and 99999 (pattern (a)): audit clean, no 4-ways, junctions ≥ 72.6°, T spacing ≥ 158 (p10 ≥ 180), stems ≤ 290, parallel spacing median 227, p5 ≥ 221.
- **What moved with the default world:**
  - the Legacy pin;
  - 31 seeded grass nodes, by migration `20261018_001_deeper_suburb_nodes.sql`. The 16 that fell in the grown urban zone stepped out along their radius into the suburb, and the rest moved off the new roads;
  - the urban grid and rural trails, which regenerate at the new radii.
- **Owned plots:** the dev DB had no owned suburban lots. Its one owned rural plot lost its lot and is kept as it was, but two new rural lots overlap it (#119).
- `suburb-plan.ps1 -Inner -Outer` (env `SUBURB_INNER`, `SUBURB_OUTER`) draws the plan at other radii.

## 4. Approach

### 4.1 Neighbourhoods and frames
1. **Neighbourhoods.** Each 45° sector between arterials is a neighbourhood. A seeded choice may split it by angle along a mid-angle greenway. A radial split is not used: each half pays the warp margins again. At 800 deep a half fitted one street. At 1200 deep (#117) the halves fit 2 rows each, where the whole band fits 5.
   - *As built (#106):* the sector's own stream, `stream(seed, [sector])`, draws the split, with probability `split_chance` (0.5). The greenway is `greenway_width` (40) wide, and each half's side line is inset half of that from its centre line. A split stands only if both halves fit rows and are at least two street spacings (2·(2·110+16)) across at the inner edge. Otherwise the sector stays whole. At the default radii every split fits.
2. **Frame.** Each neighbourhood gets a smooth frame `F(u,v) → world`:
   - v is radial depth.
   - u runs between the two sides, which are lines parallel to the arterials (inset e = half the arterial width + a 20 buffer): `θ_min(r) = θ_i + asin(e/r)`.
   - A gentle shared warp `r += A·sin(2πu/λ+φ)` (A ≤ 40, λ ≥ 1200) gives every street the same curve, so spacing stays exact.
   - Optional: rotate the frame so rows run radially, for the quadrant look of the aerial photo. *Not built in #106 or #111.* The three patterns already vary the look, and a radial frame would need its own row fit and side lines (rows would end on the zone edges, not on side lines). Left until Chris asks for it from the images.
   - *As built (#106, `suburb/frame.rs`):*
     - u is in world units: `u = t·U`, where t runs 0 to 1 between the side lines at a given radius, and U is the frame's width at mid-depth. So λ means the same in every neighbourhood.
     - The base band is inset by A from each zone edge. v = 0 is r = inner + A before the warp, and depth D = 1200 − 2A, which is 1140 at A = 30 (it was 800 − 2A = 740 before #117). The warped back lines (v = 0 and v = D) then stay inside the annulus and touch its edges only at the warp's crests.
     - The leftover between a back line and the zone edge is an *edge strip*, a `Buffer` of kind `Edge`. #108 and #111 may give it lots or a path.
     - F and its inverse are closed forms: `r = base + v + w(u)` and `θ = θ_lo(r) + t·(θ_hi(r) − θ_lo(r))`.
     - The phase φ is the first draw from the neighbourhood's stream.
     - *Tuned in #111:* λ 1600 → **1200**, A stays **30**. At 1200, a whole sector's rows read as one gentle S, as in the aerial photo. A = 40 was tried: it costs the band 20 of depth, so lots come out 104 deep, the lot-area median falls under 6600, and (c)'s ring kinks.
3. **Streets are iso-lines of the frame.** Offsetting a curvy spine was considered and dropped: it gets cusps wherever the curve's radius is under the offset distance, and clipping it leaves slivers. Tensor-field streamlines were also dropped: too much machinery for 16 neighbourhoods. Iso-lines need neither, and their junctions come out near 90°.
4. **Row fit.** Fit the row count to the band: `d = (W − n·w)/2n` must fall in [95,130]. For W = 1140, n = 5 gives d = 106. At the old W = 740, only n = 3 fitted (d = 115); n = 4 gives 85, too shallow.
5. **Building blocks:**
   - row (iso-v), rail (iso-u);
   - U-turn (radius S/2);
   - bulb (`street_plan::bulb`, r 13, 12 segments, under the 45° turn limit);
   - racetrack (two rows joined by two U-turns);
   - comb (stems every S off a carrier);
   - entrance: 2–4 per neighbourhood, at least 150 apart, and across an arterial either aligned exactly or offset by at least 80.
     - *As built (#106):* the candidates are the row ends on a neighbourhood's arterial sides. Each one has a `gate` on the side line and an `at` on the arterial's centre line, at the perpendicular foot. The stream shuffles them, with one from each arterial side first, then draws a wanted count in [2, 4].
     - Neighbourhoods allocate in sector order. A later one skips candidates that clash with earlier entrances across the arterial. If that leaves it short of 2, it takes a clashing candidate and lines its `at` up exactly with the one it clashed with, so the stub slants.
     - The patterns may re-target entrances (a racetrack's rows aren't the frame's rows), but they must keep these rules. The test `entrances_keep_their_spacing` checks them over 200 seeds.
     - *Checked as laid (#111):* `entrances_line_up_across_the_arterials` finds every suburban street end on an arterial in the mixed world (5 seeds and the wide canals). Along each arterial, ends from opposite sides are at least 80 apart (no 4-way), and ends from the same side at least 150. All three patterns keep the rules, so #111 needed no change here.
     - *Changed in #107:* lining up exactly makes a 4-way on the arterial, and rows across an arterial differ in radius only by the warp, so the fallback fired in about 10 places a world. A clashing fallback now moves its `at` along the arterial to exactly 80 from the entrance it clashed with, on the nearer side. The neighbourhood's other entrances on that arterial move by the same amount where the rules allow, because the pattern bends the whole band of rows on that side to meet them (see 6).
6. **Patterns.** Each pattern has a `fits(frame)` test. A seeded weighted pick chooses only among those that fit, so there is no reject-and-retry.
   - **(a) Warped parallels:**
     - fitted rows, joined by rail T-links every L ≈ 400;
     - links are offset by L/2 between neighbouring row gaps, so there are no 4-ways;
     - each row end is a U-turn, a bulb or an entrance.
     - *As built (#107, `suburb/patterns.rs`):*
       - **Ends.** A U-turn's apex and a bulb's far edge sit a lot depth plus half a street (`clear`, 123) in from the side line. Every combination of ends at each side is enumerated (entrances are fixed; each free row is a bulb or half a U-turn with a free neighbour), with both rail phases. A seeded pick, weighted 1 + U-turns, chooses among those whose cul-de-sacs are within `cul_de_sac_max`. If none are, the bulbs are pulled in towards their rail.
       - **Rails.** `count = round(span / L)` rails per gap, evenly spread between the margins. Gaps alternate offsets of 0.25 and 0.75 of the spacing, so neighbouring gaps' rails meet their shared row half a spacing apart.
       - **Roads.** Rows chained through U-turns are one road, `collector_{s}_{n}_{k}` (a racetrack is closed, starting and ending at a rail). A bulb end is its own `culdesac_{s}_{n}_{row}{l|h}` from the row's nearest rail junction, so `metrics` measures its stem from the network. Rails are `link_{s}_{n}_{gap}_{j}`. All are `local_width`.
       - **Moved entrances.** Where `allocate_entrances` moved an entrance's `at`, every row on that side bends by the same smoothstep ramp, over max(200, 8·shift), so the stub meets the arterial square on and the spacing holds. A row with its own unmoved entrance stays straight. The #106 overlay still draws such a stub from the original gate.
       - **Weld.** A row keeps its other vertices at least 0.6·STEP (in u) from a rail's (`MARK_CLEAR`). It began as a workaround: `weld_junctions` snapped an end to the *first* vertex of a segment within `SNAP`, even when the next was exact. #114 fixed that (the nearer vertex wins), which moved only Legacy's pin. The clearance stays because it keeps slivers out of the rows and removing it would move the default world.
       - **`StreetMeta`** (role Row/Link/CulDeSac, fronted sides, bulb centre, sector and nbhd) comes back in `SuburbPlan::meta`, one per road. Rows and cul-de-sacs are fronted both sides; rails neither, so the rows own the corners. A row's entrance stub runs through the buffer, where #108 must lay no lots.
       - **Pattern pick.** `choose` takes one draw from the stream whatever the outcome, and picks by weight among the patterns whose `fits` holds, so there is no reject-and-retry. #111 kept the weights at 1 each: whole sectors mix (a) and (b), split halves (a) and (c), and with parks (c) no longer costs the gap. `force_pattern` set to a pattern that doesn't fit falls back to a weighted pick among those that do. Since #109, (a) and (b) both fit a whole sector, so the Neighbourhood layout mixes them. Since #110, (a) and (c) both fit a split half, so the halves mix too. A test forcing one pattern zeroes the other's weight, so the neighbourhoods it doesn't fit get (a).
       - At seeds 42, 7, 1234 and 99999: audit clean, no 4-ways, junctions ≥ 71.6°, T spacing ≥ 136 (p10 ≥ 173), stems ≤ 290, 100% connected, parallel spacing median 246, p5 ≥ 231.
   - **(b) Loops and lollipops:**
     - an inset racetrack collector, with one lot row between it and the edge, backing onto the buffer;
     - opposed combs of cul-de-sacs inside it, about 230 long, roughly 12 lots each.
     - *As built (#109, `suburb/loops.rs`):*
       - **`fits`:** three rows or more, depth ≥ 4S and width ≥ 3S at the first row, with S = 2·`lot_depth` + `local_width` (236). At the default radii every whole sector fits, and no split half does (they are under 3S across at the first row), so the halves get (a).
       - **Racetrack.** Its rows are the frame's first and last (v = pitch/2 from each back line). Its legs sit the same distance in from the side lines, measured in the world (`side_u`), so the lot row beside a leg is as deep as the rows'. Corners have that radius too, drawn as quarter circles in coordinates measured from the leg. It is `collector_width` (20) wide, one closed road starting at the inner comb's first stem, so the start is a junction.
       - **Entrances.** One on a middle frame row is a stub `entrance_{s}_{n}_{k}` (Link, no lots) from the leg, at the depth whose foot on the arterial is its `at`, so a moved one still meets the arterial square on. One on the first or last row *squares off* that corner: the row runs straight through to the gate, bending (`Ramp`) to a moved `at`, and the leg ends on it in a T. The racetrack then splits into a road per stretch between squared corners. A stretch that would start or end on itself in a T is split again at the stem junction nearest its middle, because a road ending on itself reads as a lollipop.
       - **Combs.** Stems are iso-u lines a street spacing apart (in the world, at the comb's mid-depth), the first a leg's lots and a stem's lots in from each leg. The outer comb is half a spacing out of step with the inner, so each bulb points between two across the band, and their pies cover what four bulbs facing in pairs would leave. A stem wants its pies to reach the middle of the band, but stops at `cul_de_sac_max − STEP` (290): the 1140 band would want about 316. Ids are `culdesac_{s}_{n}_{i|o}{j}`.
       - **Loops.** Each neighbouring pair of stems becomes one loop street with chance 0.25 (`LOOP_CHANCE`, a const until #111 tunes it). Two stems join in a half circle, round in the world, its apex a lot's depth short of the middle. Ids are `loop_{s}_{n}_{i|o}{j}`, role Row, fronted both sides.
       - At seeds 42, 7, 1234 and 99999 (forced, so (a) in the split halves): audit clean, no 4-ways, junctions ≥ 72.6°, T spacing ≥ 164 (p10 ≥ 197), stems ≤ 290, at most 13 lots on a cul-de-sac, 100% connected. Lots: 1947–1992, area median 6800–6860, p5 ≥ 4530, width median 64, depth median 106. No overlaps, all convex, nothing trimmed or dropped.
       - **Gap 12–13%** over the world, against (a)'s 8–10%. Most of it is the band between the two combs' pies, which the 290 stem limit leaves, plus the corners between neighbouring bulbs. It suits a green spine and pocket parks (#111). `loops_and_lollipops_meet_their_targets` holds it at ≤ 14%.
   - **(c) Central loop (PW2):**
     - a flattened racetrack with a park inside;
     - radial cul-de-sacs pointing outward from it.
     - *As built (#110, `suburb/central.rs`):*
       - **`fits`:** about as wide as deep (width ≤ 1.3 × depth), with room for the loop's legs and a lot each side inside. At the default radii every split half fits and no whole sector does (about 1.6 times wider than deep, which would leave a park the size of a block of (a) inside the loop).
       - **Loop.** A collector (20 wide) `band` in from every edge of the frame, in the world (`Frame::side_u`, moved there from (b)'s `Track`). `band` = half the collector + `STEM` (170) + a bulb + a lot depth, about 312. Its corners are as round as the inside allows, capped at 180, so a split half's narrow inner end is a half circle. The loop is one closed road, starting at an entrance's junction. **Only its inside has lots** (one block, back to back, with open ground down the middle where the inside is wider than two lots). Its outside is all stems' and entrance roads' lots, as Southworth's ideal has it: no house on the collector.
       - **Cul-de-sacs.** Straight stems on the loop's outward normal, a street spacing apart measured `STEM` out (at the tips), so the fans at the corners get enough stems for their lots to back onto each other. A stem is pinned to each corner's diagonal, the longest ground a stem reaches, unless an entrance is already there. Each runs to a bulb and a lot's depth short of the frame's edge, at most `cul_de_sac_max − STEP` (290).
       - **Entrances** are roads from the loop vertex nearest the gate (spread to 0.9 of a spacing apart), leaving the loop square on and curving (a cubic Hermite) to cross the side line square on, then straight to the arterial. Lots both sides, role Row.
       - **Lots** (`lots.rs`): frontage is now per side. A ray backing onto a road's unfronted side stops at `lot_depth` instead of meeting half way, and a road owns a meeting street's corner only on a side it fronts. (a) and (b) front every road on both sides or neither, so they don't move.
       - **Tried and dropped:** a stadium on a straight axis down the middle (its round ends fan the stems so far apart that a quarter of the ground is left, and only 6 stems fit); stems curving from the loop to bulbs spaced along the frame's edge (the walker can't tile skewed, converging streets: lots were dropped and the gap rose to 34%); fronting the loop's outside too (its lots took the stems' roots, and the gap rose).
       - At seeds 42, 7, 1234 and 99999 (forced, so (a) or (b) in the whole sectors): audit clean, no 4-ways, junctions ≥ 79°, T spacing ≥ 59, stems ≤ 290, at most 13 lots on a cul-de-sac, 100% connected. Lots: 1793–1888, area median 6690–6770, p5 ≥ 4290, width median 64, depth median 106. No overlaps, all convex, nothing trimmed or dropped.
       - **Gap 15–21%** over the world with (a) in the whole sectors, against (a)'s 8–10%. A comb pointing out from a convex loop fans through a full turn, and a split half is only about 3.5 spacings across, so the wedges at the frame's corners and the ground past the bulbs are left. That is #111's ground for parks and paths. `central_loop_meets_its_targets` holds it at ≤ 22%. A deeper band (`STEM` ≥ 195) no longer fits a split half.
7. **Parks** (*#111, `suburb/parks.rs`*). The open ground the lots leave, where it's big enough to use:
   - **Ground.** Each neighbourhood's ground runs from its arterials' edges (or its greenway's centre line) between its side lines, inside the zone. It's rasterised in 5-unit squares: *built* under a road or a lot, *buffer* between a side line and the arterial or greenway, *open* otherwise.
   - **Usable.** Open ground is opened (eroded, then dilated) by 10, so only ground at least 20 across is kept. The slivers between a lot and a road, or between rear lines, stay gap.
   - **Parks.** Each piece of usable ground is a park if it holds at least half a lot (0.5 × frontage × depth), or runs into a buffer, where it widens the green strip.
   - The patterns leave this ground on purpose: (b)'s spine down its band, (c)'s green inside the loop and its corner wedges, (a)'s corners between a fan and a side line, and the edge strips. No lot is laid on a park, because a park is only ground no lot took. A pocket lollipop wasn't needed: the gap is 2%.
   - **Not on the wire yet** (#129): `/terrain` carries roads and lots only, so parks show in the plan report and the metrics, and clients draw bare ground there.
   - At seeds 42, 7, 1234, 99999 and 2024 (mixed): 120–150 parks, green 16–18%, gap 2%.
8. **Determinism.** Each neighbourhood draws from its own SplitMix64 stream, seeded with hash(seed, sector, nbhd). Changing one neighbourhood then doesn't re-id the lots in the others, which matters because lots are owned plots.
9. **Joins.** Every street end is placed exactly on its target. Any other end stays at least 40 from roads it doesn't join. `weld_junctions` and `audit` stay unchanged.

### 4.2 Lots fill the blocks
A walker runs over the welded roads, using a `StreetMeta` for each street (role, which sides are fronted, bulb centre, neighbourhood). Lots stay 4-corner `TerrainPlot`s, so **the protocol doesn't change**.
1. **Runs.** Each fronted side of a street is split into runs between junction exclusions. At a T, the through road owns the corner.
2. **Widths.** Lot widths are measured at mid-depth, accounting for curvature, so lots on curves become pies. `n = round(L/60)`. Neighbouring lots share each side ray, so a run tiles exactly.
3. **Depth.** Ray-march to the next obstacle.
   - If that is a fronted road within 2·D, the lot takes half the distance, so back-to-back lots share a rear line.
   - Otherwise the depth is min(hit, 110).
   - On the concave side of a curve, depth is clamped to 0.85/κ.
4. **Bulbs.** Bulbs get 4–5 pie lots, with rays from the bulb centre. (#108: 2–5, by area.)
5. **Overlap pass.** Lots go in priority order: through roads, then stems, then pies. (#108 puts pies first; see *As built* below.)
   - Each is checked with `quads_overlap_strict`. `polygons_overlap` (terrain.rs:481) can't be used: it counts lots that only touch, and back-to-back lots always touch.
   - A lot that overlaps shrinks (bisection, down to 60% depth) or is dropped.
   - `lot_clearance::finish_lots` does the final trim.
6. **Arterials.** No lot fronts an arterial.
7. **Ids.** Lot ids are `sub_{sector}_{nbhd}_{street}_{L|R}_{k}`.

**Expected result:** about 750–1000 suburban lots, up from 394, at about 6600–7500 area each. That is still the "large" category (`plot_sync.rs:78`).

*As built (#108, `suburb/lots.rs`):*
- **Result.** At seeds 42, 7, 1234 and 99999 (pattern (a)): 2035–2070 lots (the 1800–3000 suburb holds 1.8× the old ground). Area median 6740–6800, p5 ≥ 5100, p95 ≤ 8330. Width median 64, depth median 105. All convex. No overlaps. `finish_lots` trims and drops none. No lot fronts an arterial. At most 13 lots on a cul-de-sac. Lots cover 77–78%.
- **Frontage 64.** `lot_frontage` is 64, not 60. A five-row band fits lots 106 deep (#117), and 60 × 106 gave a median area of 6280. 64 × 106 keeps the plan's 60 × 110 area.
- **Where lots may go.** Between the neighbourhood's side lines, inside the zone. So no lot stands in a buffer or greenway, or on a stub, and none fronts an arterial.
- **Runs.** A run is where a ray's front point is clear: off every road, inside that ground. Rails front no lots, so the rows own the corners at every T. A fronted street's end that meets another fronted street partway along leaves that street's half width plus `lot_depth` to it (none do in pattern (a); combs in (b) will). A run's end ray against another road, such as a rail leaning up to 20°, turns to follow that road's edge, and the sample rays it shadows are dropped. So the end lot is a trapezoid against the rail, not a wedge with a slot beside it. Rays between samples (5 apart) are blended, not recast.
- **Continuations.** Where one fronted street carries straight on as another (a strand into its cul-de-sac, a closed racetrack's start and end), each street's line runs 30 into the other. Both then cast the same ray where they meet, and their lots don't overlap there.
- **Depth.** Back to back means a fronted road within 2·`lot_depth_max` (260), not 2·D: a row fit may go to 130 deep. A lot backing onto open ground (a back line, the zone edge, a side line) may go to `lot_depth_max`, not `lot_depth`, which takes up most of the edge strips. A lot backing onto a road that has no lots stops at `lot_depth`.
- **Bulbs.** The stem's lots stop 20 short of the bulb, so the pies wrap about 240°. The pie count is the most of 5 down to 2 whose every pie holds 0.65 × frontage × depth. At 4–5, as the plan had it, a 160° bulb gives pies under 4000. A pie's first and last edges are the stem's last rays, shared exactly.
- **Overlap order: pies, then rows, then stems.** A row's ray passing beside a bulb finds nothing behind it and runs a full lot deep into the pies' ground. A row lot gives that back easily, but a pie shrunk that far falls under the area floor. A lot also has to be clear of every road, convex and at least 0.45 × frontage × depth, or it shrinks (down to 60% depth) or is dropped.
- **Changed in #109, for pattern (b)'s combs:**
  - A lot backing onto open ground goes to `lot_depth_max` only if its ray leaves the neighbourhood within 2·`lot_depth_max` (an edge strip, a side line). Open ground deeper inside is for other streets' lots, so it gets `lot_depth`. Otherwise the racetrack's inner lots ran 130 deep beside the stems and took their ground. Pies are the exception: facing the middle of a (b) band, they take `lot_depth_max` to fill it.
  - A pie's back-to-back half way is measured from the street's edge (half width), not the bulb's, as the stem lots it backs onto are. Otherwise pies reached past the shared rear line, and the stem lots placed after them shrank along their whole depth.
  - The corner a through road owns from a meeting street is `width/2 + lot_depth + 1`. The 1 covers the through road's lots starting a hair off its edge, which otherwise dropped the meeting street's first lot.
  - Pattern (a)'s numbers barely move: gap 8–10% as before.
- **Ids** are `sub_{sector}_{nbhd}_{street}_{L|R|B}_{k}`. `street` is the street's place among its neighbourhood's in the plan, B marks pies, and k is numbered before the overlap pass.
- **Gap: 8–10%, not ≤ 5%.** Measured at seed 42, the gap is:
  - 4.3% inside frames: mostly the corners between a U-turn's or bulb's fan of lots and a straight side line, where the ground behind the fan runs over 130 deep;
  - 2.4% edge strips, beyond `lot_depth_max`;
  - 1.6% greenways and side buffers, which the metric counts as gap until parks exist;
  - 1.2% near roads.

  Lots can't take these without going outside 95–130 deep. #111's parks and paths are meant for exactly this leftover ground. `neighbourhood_lots_meet_their_targets` holds the gap at ≤ 10% until then.

### 4.3 Modules
- **`backend/src/models/geom.rs`:**
  - moved from terrain.rs: `polyline_length` and `SpatialGrid`, plus a new `insert_segments`;
  - new: `Polyline` (cumulative lengths), `quads_overlap_strict`, `SplitMix64`.
  - #104 created the file with `quads_overlap_strict`; #105 moved the rest in. `insert_segments` fills only the cells each segment's own box reaches. `Polyline::at(d)` is a binary search over cumulative lengths. `SplitMix64::stream(seed, &[sector, nbhd])` gives a neighbourhood its own stream. Legacy keeps its own `point_at_distance` and `insert`, so its output doesn't move.
- **`backend/src/models/suburb/`:**
  - `mod.rs`: `plan() -> SuburbPlan{roads, meta, neighbourhoods}`
  - `frame.rs`
  - `neighbourhood.rs`
  - `patterns.rs`
  - `loops.rs`, pattern (b) (#109)
  - `central.rs`, pattern (c) (#110)
  - `lots.rs` (#108)
  - `parks.rs` (#111)
  - `graph.rs`
  - `metrics.rs`
  - `render.rs`, test-only
- **Reuse:**
  - `street_plan::{resample, bulb, end_direction, keeps_clear, self_crosses, sharpest_turn, weld_junctions, audit}`
  - `lot_clearance::finish_lots`
- **Wiring:**
  - `suburb::plan` in `generate_all_roads_with_params` (roads), and `suburb::lots::lay` in `generate_suburban_plots_internal` (lots). Until #113 each was a `match suburban_params.layout`. `generate_suburban_plots_internal` runs `suburb::plan` again for the meta, rather than keeping it on the config.
  - Pass the plan as a local, so `version()` is unaffected.
- **Params (#105):**
  - `SuburbanGenParams::neighbourhood: NeighbourhoodParams`, `#[serde(default)]`, so `world_config` rows and admin PUTs from before #105 still load. The `layout` switch (`legacy` or `neighbourhood`) went with Legacy in #113, along with Legacy's own params. Unknown fields are ignored, so an old row that still has them loads as the neighbourhoods.
  - `NeighbourhoodParams` (`#[serde(default)]` on the struct, so a partial object loads): `lot_frontage` 64 (60 until #108), `lot_depth` 110 (`lot_depth_min` 95, `lot_depth_max` 130), `local_width` 16, `collector_width` 20, `buffer` 20, `split_chance` 0.5, `greenway_width` 40 (both added in #106), `link_spacing` (L) 400, `bulb_radius` 13, `cul_de_sac_max` 300, `warp_amplitude` (A) 30, `warp_wavelength` (λ) 1200 (1600 until #111), `pattern_weights` (1 each), and `force_pattern` for tests.
  - **One resolved set of radii:** `WorldGenParams::suburban_params()` takes the radii, seed and arterial width from the world. Roads and lots both read it, and `suburban.inner_radius` and `outer_radius` have no effect. `seed` is `#[serde(skip)]` since #113, like `arterial_width`. Before #105 the lots read `suburban`'s own radii. For the default world the two sets agreed, so the pin didn't move.
  - **One table of zone radii (#116):** the radii are named consts in `protocol/src/rules.rs` (`URBAN_OUTER_RADIUS`, `SUBURBAN_OUTER_RADIUS` and so on, collected in `ZONE_OUTER_RADII`). `Zone::radii()`, `WorldGenParams::default()`, `SPAWN_Y` and `arterial_end_radius` all derive from them. The server writes them into the DB `zones` table on startup (`engine::zone_sync`), and the React client draws its zone rings from `/bureaucracy/zones` and takes the admin form's defaults from `GET /admin/world-params/default`. Moving a zone edge is a one-line change, with no migration. An admin regenerate that changes the radii moves only the generator; zone classification still reads the table (#118).
  - **Pin:** `default_terrain_version_is_pinned` holds `get_default().version()` at `ed1853f1971d61be`, the Neighbourhood default (#112). Removing Legacy (#113) didn't move it. #128 moved the hash but not the world: `version()` now snaps every float to a 1e-3 grid before hashing, so debug and release agree (it was `41b571c1a312b6c4` before). Legacy's own pin (`1a59dc1bd07783ce` at the end, after #114) went with it.
  - **React:** the Suburban section of `WorldTweakPanel` has a Pattern select (Mixed, or one `force_pattern`) and sliders for the street widths, lot frontage and depth (within `lot_depth_min`–`max`), split chance and warp amplitude (#113). Before #113 it had a Layout select and Legacy's sliders.

### 4.4 Harness and targets
- **Release builds** hash the pin the same as debug since #128 (`version()` quantises floats), so `cargo test --release` is a fair way to run the suite.
- **Acceptance (#111):** `suburb::tests::neighbourhood_suburb_meets_its_targets` measures the mixed suburb (default weights) at seeds 42, 7, 1234, 99999 and 2024, and at seed 42 with the wide canals. It asserts the hard targets and those the patterns hold one by one, including green 8–20% and gap ≤ 5%, and prints the soft ones it misses (run with `--nocapture`). `metrics::measure_world` generates and measures a world, parks and all, for every pattern test and the report.
- **Report test:** `#[ignore]` test `suburb::render::tests::suburb_plan_report`. Env: `SUBURB_SEEDS`, `SUBURB_PATTERN` (#109), `SUBURB_SECTOR`, `SUBURB_OUT` (default `target/suburb-plan/`).
- **Outputs:** an SVG per seed (named `neighbourhood-<seed>`, as before #113, so `-Compare` still lines up with older runs), metrics JSON, an `index.html` grid and a markdown table.
- **PNG:** `backend/tools/suburb-plan.ps1` runs the test in release and converts each SVG with headless Chrome. Look at the PNGs beside `reference/`.
- **Script options:** `-Seeds`, `-Pattern` (force one pattern wherever it fits, #109), `-Sector n`, `-Inner` and `-Outer` (the suburb's radii, #117) and `-Out`. `-Compare <dir>` writes `compare.html`, with each plan beside the one of the same name in an earlier run.
- **Reading the SVG:** roads are drawn at true width and coloured by role. Suburban lots are shaded by area, with the street edge darker. Lots `finish_lots` trimmed are edged orange, and lots it dropped are drawn dashed magenta at their generated shape. A red ring marks each place that breaks a target; hover it in a browser for the reason. For the Neighbourhood layout (#106), the view also draws each frame dashed, with its rows dotted, a label giving the row count and lot depth, the green buffers, greenways and edge strips, and each entrance as a stub from its gate to a dot on the arterial.
- **Definitions that work from the roads alone (`suburb/metrics.rs`).** They were written to work for Legacy too, and still hold for the hand-built test plans:
  - **neighbourhood:** a 45° sector between arterials;
  - **entrance:** a vertex shared by a suburban road and any other road, counted in the sector its suburban street comes from (#107; it sits on the arterial's centre line, which rounding used to give to either side);
  - **junction:** three or more links from at least two roads, so the vertex where a bulb closes on its own stem is not a T;
  - **cul-de-sac:** a suburban road ending in a bulb or a dead end, with its stem measured to where the bulb closes;
  - **parallel spacing:** sampled every 25 along each street, as the distance to the nearest street across it, when that street runs within 20°;
  - **coverage:** sampled every 5 over the annulus, counting road, then lot, then green, then gap;
  - **green:** the Neighbourhood layout's buffers, greenways and parks (#111). A plan without them (Legacy had none) counts open ground within 40 of an arterial's edge.
- **Legacy baseline:** posted on #104, at seeds 42, 7, 1234 and 99999. It has 315–394 lots, 6–9 suburban 4-ways, parallel spacing of about 120, lots of about 77 × 91, 28–32% of lots trimmed, and 58–65% of the ground a gap.

| Metric | Target |
|---|---|
| `street_plan::audit` | clean (hard) |
| suburban 4-ways | 0 |
| junction angle | ≥ 70° |
| T spacing along a road | min ≥ 40, p10 ≥ 150 |
| entrances per neighbourhood | 2–4 (`metrics` counts per sector, so a split sector may show up to 8 until it learns about neighbourhoods) |
| cul-de-sac | ≤ 300 long, ≤ 20 lots |
| parallel spacing | median 236 ± 20, p5 ≥ 200 |
| connected to an arterial | 100% |
| lot area | median 6600–7500, p5 ≥ 4000, p95 ≤ 11000 |
| lot width at mid-depth | median 55–65 |
| lot depth | median 100–125 |
| convex lots | 100% |
| lot-lot overlaps | 0 (hard) |
| lots trimmed / dropped by `finish_lots` | ≤ 10% / ≤ 3% |
| coverage: lots / roads / green / gap | ≥ 55% / 15–22% / 8–20% / ≤ 5% (roads: see below) |
| lots fronting arterials | 0 |
| generation (debug) | well under the 5 s test |

*Mixed suburb at #111* (seeds 42, 7, 1234, 99999, 2024): audit clean, no 4-ways, no overlaps, junctions ≥ 74.6°, 1883–1921 lots, area median 6716–6783, lots 70–71%, green 16–18%, gap 2%, generation about 200 ms in release. Soft targets still missed, all left for Chris:
- **roads 10–11%, not 15–22%.** The roads are drawn at paving width (16 and 20). The target came from a right of way (50 ft with verges). Meeting it would mean wider streets or counting verges as road; neither seems worth it.
- **T spacing p10 90–140 and parallel spacing p5 down to 25: pattern (c).** Its corner stems fan from the loop, so their roots sit close together along it and run nearly parallel there. That's the fan's shape, not crowding: (a) alone holds p10 ≥ 180 and p5 ≥ 216.
- **entrances per neighbourhood, most 5–7.** The metric counts per sector, so split sectors read double (#115).

## 5. Work packages

| # | WP | Depends on |
|---|---|---|
| #104 | 1. Harness: SVG and metrics, `graph.rs`, strict overlap, Legacy baseline | #103 |
| #105 | 2. `geom.rs`, the `SuburbLayout` switch, one set of radii, React select, Legacy pin test | #104 |
| #106 | 3. Neighbourhoods, frames, buffers, entrances, RNG streams | #105 |
| #107 | 4. Pattern (a): warped parallels, streets | #106 |
| #117 | 4b. Deepen the suburb to 1800–3000, so neighbourhoods fit 5 rows | #116 |
| #108 | 5. Lots v2 | #107, #117 |
| #109 | 6. Pattern (b): loops and lollipops | #108 |
| #110 | 7. Pattern (c): central loop | #108 |
| #111 | 8. Mixing, entrances, parks, tuning; Chris signs off the images | #109, #110 |
| #112 | 9. Flip the default: node-move migration, release owned Legacy plots, thresholds, Flint fixture and showcase, CLAUDE.md | #111 |
| #113 | 10. Remove Legacy and its params | #112 |

**Master stays green.** Legacy stayed the default until #112. `legacy_default_terrain_version_is_pinned` proved its output didn't move, and every new test set `layout: Neighbourhood` explicitly.

**At the flip (#112):**
- **Nodes.** `the_default_terrain_moves_no_seeded_node` will fail. Fix it with a move migration generated from `place_nodes(default)`, applied in order by `seeded_nodes()`.
- **Owned plots.** Owned Legacy suburban plots need releasing with a refund. Otherwise `OnOwned::Keep` leaves them overlapping the new lots.

*As built (#112):*
- **Default.** `SuburbLayout::Neighbourhood` is `#[default]`, so a saved `world_config` without a layout (the dev DB has none at all) gets it too. One that names `legacy` keeps Legacy until #113. The React layout select falls back to `neighbourhood`.
- **Migration `20261019_001_neighbourhood_suburb.sql`** (20261018 is taken; sqlx versions by the date prefix):
  - Releases owned suburban plots with a Legacy id (anything not `sub_`), unless a saved `world_config` names Legacy, the same way `OnOwned::Release` does: stations removed, permissions reset, the deed refunded as a `deed_refund` transaction. Startup's sync then deletes them. The dev DB had none. Tested on a copy with one owned plot and a station: refunded 500, station gone, plot deleted.
  - Moves 19 grass nodes off the new streets, rows printed by the ignored test `models::resource::tests::print_node_moves`. None stuck. Startup then reports 0 moved, 0 stuck.
- **Seeded node clearance.** `seeded_nodes_sit_clear_of_the_default_terrain` now holds nodes `KEEP_CLEAR` (15) from a road, the rule startup moves them by, not 20. One node stands 16 from a new street, where nothing would move it.
- **Thresholds.** `generated_lots_are_clear_and_unique` wants > 1500 suburban lots in the Neighbourhood worlds (> 150 still for Legacy), and so does `neighbourhood_layout_changes_only_the_suburb`. `both_layouts()` and `test_suburban_plot_generation` build Legacy explicitly, since `generate_suburban_plots` is Legacy's.
- **Flint.** `fixtures/terrain.json` recaptured (version `41b571c1a312b6c4`, 1905 suburban lots). The `suburb` shot is now low over the (a) neighbourhood west of the south arterial, its rows and bulbs sweeping in towards the tower.

*As built (#113):*
- **Gone from `terrain.rs`:** the Legacy generator (collectors, the rejection-sampled local streets, connectivity repair, emergency connectors), its plot walk (`generate_suburban_plots`, `extract_plots_from_roads`, `generate_plots_along_road`) and the helpers only it used (`SeededRng`, `ConnectionPoint`, the intersection-angle checks, its step constants). Also gone: the no-params wrappers (`generate_all_roads()` and the rest), which nothing called, and `street_plan::{crosses, distance_along}`. About 2,000 lines in all.
- **Params:** `SuburbLayout` and `layout` went, and so did `collector_width`, `local_street_width`, `min_street_spacing`, the plot-area window (`min_`/`max_`/`target_plot_area`) and the frontage window. Nothing but Legacy read them. `seed` stays on the struct, because the generators read the resolved seed from it, but it's `#[serde(skip)]`. **A saved row that names `legacy` loads as the neighbourhoods**, with no alias needed, because the field no longer exists and unknown fields are ignored (`world_params_load_old_and_new`). The dev DB has no `world_config` row.
- **Tests:** Legacy's pin, `test_suburban_plot_generation`, `test_seeded_rng_determinism` and `test_local_streets_branch_from_connectors` went with it. `both_layouts()` is now `warped_parallels()`, one world. `neighbourhood_layout_changes_only_the_suburb` is now `the_default_world_has_a_suburb`, because it has nothing to compare against. `generated_lots_are_clear_and_unique` runs the five Neighbourhood worlds only. `a_regenerated_terrain_moves_the_nodes_it_runs_over` set `suburban.seed` and Legacy's widths, which had done nothing since #105 and #112. It now sets the world seed and the neighbourhood widths, so it tests what it says.
- **Tools:** `suburb-plan.ps1 -Layout` and `SUBURB_LAYOUT` are gone. Plans are still named `neighbourhood-…`.
- **Pin:** `41b571c1a312b6c4`, unchanged.
