# The Capitol: The Wilds

*Plan of record, 2026-10-18. Issues are labelled `wilds`. The plan was #193 (W0), and the work runs from #194 to #204.*

## 1. Vision

You stand on the rural edge with the tower at your back, and the land ahead rises into blue ranges. That's the wilds: mountains, rivers, streams and lakes, and the best raw materials in the world.
- ore high on the bare rock
- old timber in the valley woods
- clay and reeds on the riverbanks

Nobody can own any of it. The trip out is the price of quality, and the trip back in, to the buyers at the centre, is the trade.

The GDD says geography sets the ceiling on quality (§6). In the wilds that ceiling rises the further you go and the higher you climb. So the wilds give the economy what Economy v1 wanted from geography (`the-capitol-economy.md` §1): the zones differ, and distance costs something.

## 2. Where things stood (2026-10-18)

- **The zone exists, but there's almost nothing in it.**
  - `Zone::Wilderness` covers r3800 to r4000, a 200-unit rim on the 8000 world, and `has_plots()` is already false.
  - The generator puts nothing there: no water, no relief, no nodes, no trails. The arterials stop at r3800.
  - The Flint client fades it to the background colour from r3400.
- **The ground is flat everywhere,** on the server and in Flint. The only water is the moat and the two canal rings. The Flint ocean is one sheet at y = 0, seen through holes in the ground.
- **The docs disagreed:**
  - The GDD priced a 100-Strand Wilderness deed (§8.4). The milestones said "no plots (public gathering)", and the code agreed with them (the deed is NULL).
  - The GDD spawned new players "in the wilderness". Since B1 they spawn on the rural edge on purpose (`rules::SPAWN_Y`).
  - The GDD left "Wilderness structure (uniform or biomes?)" and "Resource distribution by region" open.
- **Economy D5 already assumed the wilds:** trees and iron veins placed there, and E2.3 promised "veins in the wilderness".

## 3. Decisions

Chris decided the shape on 2026-10-18: grow the world, use a real height field, have rivers and lakes block while streams can be waded, and give the wilds a plan of their own. WD1 to WD8 work that out.

### WD1. The world grows to 12000 ([W1 #194](https://github.com/chrischaps/TheCapitol/issues/194))

- **The new size:**
  - `WORLD_SIZE` goes from 8000 to 12000, and the centre moves from (4000, 4000) to (6000, 6000).
  - `WILDERNESS_OUTER_RADIUS` becomes 6000, so the wilds are 2200 deep, nearly twice the suburb.
  - `RURAL_OUTER_RADIUS` stays 3800, so nothing inside the rural edge changes size.
- **The shift.** One migration translates every stored position by +2000: players, nodes, plots, stations, and any saved `world_config`.
  - *Why shift rather than allow negative coordinates:* every bounds check, fixture and client assumes 0..WORLD_SIZE, and B1 already did a world rescale this way.
- **The edge.** The world is the r6000 disc. The square's corners are out of bounds.

### WD2. The height field is a shared, pure function ([W2 #195](https://github.com/chrischaps/TheCapitol/issues/195))

- **Where it lives:** `capitol_protocol::land`, as `height_at(x, y, &LandParams) -> f64`, `slope_at` and `normal_at`.
- **How it's made:** seeded fBm for the rolling ground, plus ridged noise for the ranges. It goes through `fmath::StableMath`, so it's identical on every platform (#180).
- **Nothing inside the rural edge moves:** the height is exactly 0 inside r3800, and the foothills rise from about r3900.
- **What's on the wire:** only `LandParams`, on `/terrain`. Clients compute the height and build their own mesh, so the heights never cross the wire and can never disagree.

### WD3. Water comes from the land ([W3 #196](https://github.com/chrischaps/TheCapitol/issues/196))

- **Lakes.** The generator fills depressions in a coarse grid of the height field (priority-flood). Each filled depression becomes a lake with its own level.
- **Rivers and streams.** Flow accumulation traces them: streams where the flow is lower, rivers where it's higher.
  - Each is a polyline with a width and a surface height at every vertex.
  - Water runs to the world's edge or ends in a lake.
- **The carve.** A pure `land::carve` in protocol cuts the beds and banks into the height field, so the server and the clients agree.
- **Fords** go where a river is wide and shallow. Bridges come with the trails (WD5).
- **On the wire:** `/terrain` gains `rivers`, `streams`, `lakes` and `fords`.

### WD4. Moving on the land ([W4 #197](https://github.com/chrischaps/TheCapitol/issues/197))

- **Slope.** `rules::land_speed(slope)` slows you on a slope. A slope over `CLIFF_SLOPE` blocks you ("Too steep").
- **Rivers and lakes** block like the canals, except at fords and bridges.
- **Streams** can be waded: your speed is multiplied by `WADE_SPEED`.
- **Benches.** `rules::in_water` covers the wild water, so D10 keeps workbenches out of it.

### WD5. Trails ([W5 #198](https://github.com/chrischaps/TheCapitol/issues/198))

- **The arterials run on past r3800 as wild trails.** Each is a least-cost path up a valley to a pass, with branches to the fords and a bridge where a trail has to cross a river with no ford.
- **They're ordinary `trail` roads,** so `street_plan::audit` holds and every client already draws them.

### WD6. Nothing in the wilds can be claimed ([W1 #194](https://github.com/chrischaps/TheCapitol/issues/194))

- **No lots.** None are generated past r3800.
- **The claim path** checks `Zone::has_plots()` as well as the NULL deed. Before W1 it relied on the deed alone.
- **The GDD's Wilderness deed is gone.**
- **A workbench may still stand on dry ground in the wilds (D10).** A bench isn't a claim: it can't be locked against anyone, and nobody gets the land under it. Every other station needs a plot you own.

### WD7. The spawn stays on the rural edge

- **New players arrive on the south arterial** with the tower ahead of them and the wilds at their back. The GDD's "spawn in the wilderness" now says so.
- **Why:** B1 moved the spawn out of the far corner because a new player there had every resource at hand and no reason to go in. With the wilds 2200 deep, it matters more. The first trip out is a choice made with a tool in your hand.

### WD8. What the wilds yield (a proposal for D5, which Chris approves in [#160](https://github.com/chrischaps/TheCapitol/issues/160))

| Resource | Where in the wilds | Tool |
|---|---|---|
| Iron vein | high, on bare, steep rock | pickaxe |
| Copper vein, tin vein | the ranges' flanks, so the `bronze` tool tier has its alloy | pickaxe |
| Stone | scree below the cliffs | hand / pickaxe |
| Old-growth trees | valley woods, with a higher quality ceiling than rural woods | axe |
| Clay | river and lake banks (the canal clay stays in the city) | hand |
| Reeds | lake shallows and slow rivers | sickle |

- **Quality.** The base quality ceiling rises with depth past r3800 and with altitude. It's a table in content, not code.
- **Later, outside this plan:** fish, seasons and hazards.

## 4. Work packages

| WP | Issue | What | Size | Depends on |
|---|---|---|---|---|
| W0 | [#193](https://github.com/chrischaps/TheCapitol/issues/193) | This plan, and the GDD, economy, Flint and milestones docs reconciled with it | S | – |
| W1 | [#194](https://github.com/chrischaps/TheCapitol/issues/194) | The world grows to 12000: shift migration, radii, bounds, spawn, and `has_plots` on the claim path. **Done.** As built: migration `20261107_001_world_grows.sql`; `rules::in_world` (the r6000 disc) guards both move paths, and Flint's `cap_move` pulls a click past the edge onto it; `claim` checks `has_plots` for the plot's zone and for where it stands. The woods' noise is laid round the tower (`node_spawn::WOODS_ORIGIN`), so E2.3's trees stayed put. The Flint grass mask stays r4000 (`GRASS_EDGE`) until W9. #118: option 1, radii aren't tunable at runtime | M | – |
| W2 | [#195](https://github.com/chrischaps/TheCapitol/issues/195) | `capitol_protocol::land`: the height field | M | W1 |
| W3 | [#196](https://github.com/chrischaps/TheCapitol/issues/196) | Hydrology: lakes, rivers, streams, fords and the carve, on `/terrain` | L | W2 |
| W4 | [#197](https://github.com/chrischaps/TheCapitol/issues/197) | Movement on the land | M | W3 |
| W5 | [#198](https://github.com/chrischaps/TheCapitol/issues/198) | Wild trails | M | W3 |
| W6 | [#199](https://github.com/chrischaps/TheCapitol/issues/199) | Wild nodes from the land | L | W3, E2.3 (#162) |
| W7 | [#200](https://github.com/chrischaps/TheCapitol/issues/200) | Flint: water at any height (H6, upstream) | L | – |
| W8 | [#201](https://github.com/chrischaps/TheCapitol/issues/201) | Flint: a ray against a height field (upstream) | S | – |
| W9 | [#202](https://github.com/chrischaps/TheCapitol/issues/202) | Flint: the height-mapped ground, with everything standing on it | L | W2, W8 |
| W10 | [#203](https://github.com/chrischaps/TheCapitol/issues/203) | Flint: rivers, streams, lakes and fords, with their sound | L | W3, W7, W9 |
| W11 | [#204](https://github.com/chrischaps/TheCapitol/issues/204) | Flint: the wilds seen from the city, and the city from the wilds | M | W9, W10 |

```
Server   W1 ─ W2 ─ W3 ─┬─ W4
                       ├─ W5
                       └─ W6  (also E2.3)
Engine   W7, W8  (independent; start any time)
Client         W2 + W8 ─ W9 ─ W10 (+W3, W7) ─ W11
```

W7 and W8 can start at once. W1 can run alongside the economy queue.

## 5. Architecture notes

- **The default world moves three times on purpose:** in W1 (the shift), W3 (the water), and W5 (the trails).
  - Each bumps `default_terrain_version_is_pinned` and says so in its commit.
  - Each keeps `the_default_terrain_moves_no_seeded_node` green, or ships a move migration.
  - No lot exists past r3800, so no owned plot is touched.
- **E2.3 (#162) doesn't change terrain generation,** and it doesn't need to wait for the wilds. W1's migration moves its rows like any others.
- **Wire changes go in `protocol/` with fixtures:** `LandParams` and the water on `/terrain`, and the new `movement_blocked` reasons.
- **The frozen React and Love clients only need to compile.** They'll draw the wilds flat, which is fine.
- **The Flint client** computes the ground from `capitol_protocol::land`, as it already shares the rules. It uses Flint's `flint-terrain` (`Heightmap::from_raw`, chunks) where that fits.

## 6. Risks

| Risk | Mitigation |
|---|---|
| The coordinate shift misses a stored position | W1 lists every table with a position, and its acceptance checks a live dev DB relative to the tower. |
| The height field differs across platforms | `StableMath` (#180) and a pinned sample table, checked in CI. |
| Frame time in the ranges (terrain, grass and water together) | Chunked LOD in W9, and the P8.5 budget in W9's and W10's acceptance. |
| Flint can't draw water above y = 0 | W7 upstream. If it stalls, W10 carves every lake down to sea level as a stopgap, with a ticket. |
| A world of 12000 makes the trip too long | `rules` speeds and the trails (W5) tune it. The sim (E2.7) shows where supply pools. |

## 7. Verification

- **Server (W1 to W6):** `cargo test -p capitol-protocol -p capitol-backend`, plus the plan PNG from `backend/tools/suburb-plan.ps1`, extended to draw the relief, the water, the trails and the nodes. Chris judges the PNGs.
- **Client (W9 to W11):** from `flint-client/`, `cargo test -p capitol-core && cargo build --release -p capitol-client`, then captures in `flint-client/renders/` and a live walk from the spawn into the ranges.
