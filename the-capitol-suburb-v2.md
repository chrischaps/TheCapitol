# The Capitol: Suburban Layout v2

*Plan, 2026-10-17. Tracking issue #103; work packages #104 to #113.*

The suburb (annulus r 1600–2400, cut by 8 arterials) is to be rebuilt as post-war neighbourhoods. This doc is the "Read first" for every `suburb-v2` issue. Keep it true: if a decision changes, update it in the same commit.

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
- **Harness first, then work packages behind a switch:** `SuburbanGenParams::layout`, `Legacy` (default) or `Neighbourhood`, until the flip in #112.

## 4. Approach

### 4.1 Neighbourhoods and frames
1. **Neighbourhoods.** Each 45° sector between arterials is a neighbourhood. A seeded choice may split it by angle along a mid-angle greenway. A radial split is not used: 800 deep halves to about 375, which fits one street and wastes the rest.
   - *As built (#106):* the sector's own stream, `stream(seed, [sector])`, draws the split, with probability `split_chance` (0.5). The greenway is `greenway_width` (40) wide, and each half's side line is inset half of that from its centre line. A split stands only if both halves fit rows and are at least two street spacings (2·(2·110+16)) across at the inner edge. Otherwise the sector stays whole. At the default radii every split fits.
2. **Frame.** Each neighbourhood gets a smooth frame `F(u,v) → world`:
   - v is radial depth.
   - u runs between the two sides, which are lines parallel to the arterials (inset e = half the arterial width + a 20 buffer): `θ_min(r) = θ_i + asin(e/r)`.
   - A gentle shared warp `r += A·sin(2πu/λ+φ)` (A ≤ 40, λ ≥ 1200) gives every street the same curve, so spacing stays exact.
   - Optional: rotate the frame so rows run radially, for the quadrant look of the aerial photo. *Not built in #106; left to #111 if the images want it.*
   - *As built (#106, `suburb/frame.rs`):*
     - u is in world units: `u = t·U`, where t runs 0 to 1 between the side lines at a given radius, and U is the frame's width at mid-depth. So λ means the same in every neighbourhood.
     - The base band is inset by A from each zone edge. v = 0 is r = inner + A before the warp, and depth D = 800 − 2A, which is 740 at A = 30. The warped back lines (v = 0 and v = D) then stay inside the annulus and touch its edges only at the warp's crests.
     - The leftover between a back line and the zone edge is an *edge strip*, a `Buffer` of kind `Edge`. #108 and #111 may give it lots or a path.
     - F and its inverse are closed forms: `r = base + v + w(u)` and `θ = θ_lo(r) + t·(θ_hi(r) − θ_lo(r))`.
     - The phase φ is the first draw from the neighbourhood's stream.
3. **Streets are iso-lines of the frame.** Offsetting a curvy spine was considered and dropped: it gets cusps wherever the curve's radius is under the offset distance, and clipping it leaves slivers. Tensor-field streamlines were also dropped: too much machinery for 16 neighbourhoods. Iso-lines need neither, and their junctions come out near 90°.
4. **Row fit.** Fit the row count to the band: `d = (W − n·w)/2n` must fall in [95,130]. For W = 800, n = 3 gives d ≈ 125.
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
6. **Patterns.** Each pattern has a `fits(frame)` test. A seeded weighted pick chooses only among those that fit, so there is no reject-and-retry.
   - **(a) Warped parallels:**
     - fitted rows, joined by rail T-links every L ≈ 400;
     - links are offset by L/2 between neighbouring row gaps, so there are no 4-ways;
     - each row end is a U-turn, a bulb or an entrance.
   - **(b) Loops and lollipops:**
     - an inset racetrack collector, with one lot row between it and the edge, backing onto the buffer;
     - opposed combs of cul-de-sacs inside it, about 230 long, roughly 12 lots each.
   - **(c) Central loop (PW2):**
     - a flattened racetrack with a park inside;
     - radial cul-de-sacs pointing outward from it.
7. **Determinism.** Each neighbourhood draws from its own SplitMix64 stream, seeded with hash(seed, sector, nbhd). Changing one neighbourhood then doesn't re-id the lots in the others, which matters because lots are owned plots.
8. **Joins.** Every street end is placed exactly on its target. Any other end stays at least 40 from roads it doesn't join. `weld_junctions` and `audit` stay unchanged.

### 4.2 Lots fill the blocks
A walker runs over the welded roads, using a `StreetMeta` for each street (role, which sides are fronted, bulb centre, neighbourhood). Lots stay 4-corner `TerrainPlot`s, so **the protocol doesn't change**.
1. **Runs.** Each fronted side of a street is split into runs between junction exclusions. At a T, the through road owns the corner.
2. **Widths.** Lot widths are measured at mid-depth, accounting for curvature, so lots on curves become pies. `n = round(L/60)`. Neighbouring lots share each side ray, so a run tiles exactly.
3. **Depth.** Ray-march to the next obstacle.
   - If that is a fronted road within 2·D, the lot takes half the distance, so back-to-back lots share a rear line.
   - Otherwise the depth is min(hit, 110).
   - On the concave side of a curve, depth is clamped to 0.85/κ.
4. **Bulbs.** Bulbs get 4–5 pie lots, with rays from the bulb centre.
5. **Overlap pass.** Lots go in priority order: through roads, then stems, then pies.
   - Each is checked with `quads_overlap_strict`. `polygons_overlap` (terrain.rs:481) can't be used: it counts lots that only touch, and back-to-back lots always touch.
   - A lot that overlaps shrinks (bisection, down to 60% depth) or is dropped.
   - `lot_clearance::finish_lots` does the final trim.
6. **Arterials.** No lot fronts an arterial.
7. **Ids.** Lot ids are `sub_{sector}_{nbhd}_{street}_{L|R}_{k}`.

**Expected result:** about 750–1000 suburban lots, up from 394, at about 6600–7500 area each. That is still the "large" category (`plot_sync.rs:78`).

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
  - `lots.rs`
  - `graph.rs`
  - `metrics.rs`
  - `render.rs`, test-only
- **Reuse:**
  - `street_plan::{resample, bulb, end_direction, distance_along, crosses, keeps_clear, self_crosses, sharpest_turn, weld_junctions, audit}`
  - `lot_clearance::finish_lots`
- **Wiring:**
  - `match suburban_params.layout` in `generate_all_roads_with_params` (roads) and in `generate_suburban_plots_internal` (lots). Until #107 and #108, `Neighbourhood` returns no suburban roads and no suburban lots.
  - Pass the plan as a local, so `version()` is unaffected.
- **Params (#105):**
  - `SuburbanGenParams::layout` (`legacy` or `neighbourhood` on the wire) and `::neighbourhood: NeighbourhoodParams`, both `#[serde(default)]`, so `world_config` rows and admin PUTs from before #105 still load.
  - `NeighbourhoodParams` (`#[serde(default)]` on the struct, so a partial object loads): `lot_frontage` 60, `lot_depth` 110 (`lot_depth_min` 95, `lot_depth_max` 130), `local_width` 16, `collector_width` 20, `buffer` 20, `split_chance` 0.5, `greenway_width` 40 (both added in #106), `link_spacing` (L) 400, `bulb_radius` 13, `cul_de_sac_max` 300, `warp_amplitude` (A) 30, `warp_wavelength` (λ) 1600, `pattern_weights` (1 each), and `force_pattern` for tests. Tune A and λ in #111.
  - **One resolved set of radii:** `WorldGenParams::suburban_params()` takes the radii, seed and arterial width from the world. Roads and lots both read it, and `suburban.inner_radius`, `outer_radius` and `seed` have no effect. Before #105 the lots read `suburban`'s own radii. For the default world the two sets agreed, so the pin didn't move.
  - **Pin:** `legacy_default_terrain_version_is_pinned` holds `get_default().version()` at `6006800ab79fbff8`.
  - **React:** the Suburban section of `WorldTweakPanel` has a Layout select. `neighbourhood` is optional in `api/admin.ts`, so a reset leaves it to the server's defaults.

### 4.4 Harness and targets
- **Report test:** `#[ignore]` test `suburb::render::tests::suburb_plan_report`. Env: `SUBURB_SEEDS`, `SUBURB_LAYOUT`, `SUBURB_SECTOR`, `SUBURB_OUT` (default `target/suburb-plan/`).
- **Outputs:** an SVG per layout and seed, metrics JSON, an `index.html` grid and a markdown table.
- **PNG:** `backend/tools/suburb-plan.ps1` runs the test in release and converts each SVG with headless Chrome. Look at the PNGs beside `reference/`.
- **Script options:** `-Seeds`, `-Layout`, `-Sector n` and `-Out`. `-Compare <dir>` writes `compare.html`, with each plan beside the one of the same name in an earlier run.
- **Reading the SVG:** roads are drawn at true width and coloured by role. Suburban lots are shaded by area, with the street edge darker. Lots `finish_lots` trimmed are edged orange, and lots it dropped are drawn dashed magenta at their generated shape. A red ring marks each place that breaks a target; hover it in a browser for the reason. For the Neighbourhood layout (#106), the view also draws each frame dashed, with its rows dotted, a label giving the row count and lot depth, the green buffers, greenways and edge strips, and each entrance as a stub from its gate to a dot on the arterial.
- **Definitions that also work for Legacy (`suburb/metrics.rs`):**
  - **neighbourhood:** a 45° sector between arterials;
  - **entrance:** a vertex shared by a suburban road and any other road;
  - **junction:** three or more links from at least two roads, so the vertex where a bulb closes on its own stem is not a T;
  - **cul-de-sac:** a suburban road ending in a bulb or a dead end, with its stem measured to where the bulb closes;
  - **parallel spacing:** sampled every 25 along each street, as the distance to the nearest street across it, when that street runs within 20°;
  - **coverage:** sampled every 5 over the annulus, counting road, then lot, then green, then gap;
  - **green:** open ground within 40 of an arterial's edge, until parks exist (#111).
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
| coverage: lots / roads / green / gap | ≥ 55% / 15–22% / 8–20% / ≤ 5% |
| lots fronting arterials | 0 |
| generation (debug) | well under the 5 s test |

## 5. Work packages

| # | WP | Depends on |
|---|---|---|
| #104 | 1. Harness: SVG and metrics, `graph.rs`, strict overlap, Legacy baseline | #103 |
| #105 | 2. `geom.rs`, the `SuburbLayout` switch, one set of radii, React select, Legacy pin test | #104 |
| #106 | 3. Neighbourhoods, frames, buffers, entrances, RNG streams | #105 |
| #107 | 4. Pattern (a): warped parallels, streets | #106 |
| #108 | 5. Lots v2 | #107 |
| #109 | 6. Pattern (b): loops and lollipops | #108 |
| #110 | 7. Pattern (c): central loop | #108 |
| #111 | 8. Mixing, entrances, parks, tuning; Chris signs off the images | #109, #110 |
| #112 | 9. Flip the default: node-move migration, release owned Legacy plots, thresholds, Flint fixture and showcase, CLAUDE.md | #111 |
| #113 | 10. Remove Legacy and its params | #112 |

**Master stays green.** Legacy stays the default until #112. `legacy_default_terrain_version_is_pinned` proves its output doesn't move, and every new test sets `layout: Neighbourhood` explicitly.

**At the flip (#112):**
- **Nodes.** `the_default_terrain_moves_no_seeded_node` will fail. Fix it with a move migration generated from `place_nodes(default)`, applied in order by `seeded_nodes()`.
- **Owned plots.** Owned Legacy suburban plots need releasing with a refund. Otherwise `OnOwned::Keep` leaves them overlapping the new lots.
