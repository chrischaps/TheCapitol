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
  - The Flint client faded it to the background colour from r3400 (until W9 drew the land).
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
- **The valleys line up with the arterials.** The ranges gather into `ridge_count` massifs round the ring, and with eight the valleys between them lie on the arterials' bearings, so W5's trails have valleys to climb. `normal_at` returns (x, y, up); a client maps it to its own axes.

### WD3. Water comes from the land ([W3 #196](https://github.com/chrischaps/TheCapitol/issues/196))

- **Lakes.** The generator fills depressions in a coarse grid of the height field (priority-flood). Each filled depression becomes a lake with its own level.
- **Rivers and streams.** Flow accumulation traces them: streams where the flow is lower, rivers where it's higher.
  - Each is a polyline with a width and a surface height at every vertex.
  - Water runs to the world's edge, ends in a lake, or soaks away on the rural plain (#222).
- **The carve.** A pure `land::carve` in protocol cuts the beds and banks into the height field, so the server and the clients agree.
- **Fords** go where a river is wide and shallow. Bridges come with the trails (WD5).
- **On the wire:** `/terrain` gains `rivers`, `streams`, `lakes` and `fords`.
- **As built (W3):**
  - **Where it's made:** `backend/src/models/hydrology.rs`, from the land and the roads. It samples the height on a 30-unit grid from r3150 (`INNER_RADIUS`, 150 out from the suburb) to the edge.
  - **The plain (#222):** the valleys drain towards the city, onto the flat rural ring. Chris's call: water runs on into it, rather than stopping at one radius.
    - **Steering:** water there follows a routing field, never the ground. It falls gently towards the city (0.01) and has noise 4 high at a 450 wavelength. Every road is a 3-high berm, so water skirts the roads and crosses them only where it must. A road is the crossing (W4 treats it as a bridge).
    - **Soaking away:** the plain soaks the flow away. Each cell loses `SOAK` (12) × √flow, at a rate that varies by place, and where the soaking starts varies by up to 400. So a stream gives out soon, a big river runs well in, and the ends spread across the ring (`the_plain_soaks_the_water_up_all_across_it`).
    - **Endings:** a course that soaks away ends in a **pond** or **peters out** (`mouth: peters_out`, narrowing to nothing over its last 6 points). The pond chance is 0.6 × (flow − 0.4), clamped to 0.05 to 0.8, so a big river usually ponds. A pond is an oval along the course's last stretch, sized by its flow. It's made smaller to keep 8 off every road's kerb and clear of other water, or it's given up.
    - **Rivers into streams:** a river that soaks away runs on as a stream, which joins it at its last point.
    - **Nodes and lots:** `node_placement` keeps nodes off the wild water too. 40 of E2.3's planned nodes were in its way. Migration `20261112_001_wild_water_nodes.sql` moves them (rows from `print_planned_node_moves`), and the plan itself is unchanged. No seeded grass needed a move. The rural lots don't know the water yet (#223); they come with Chris's rural rework.
  - **The flood:** a priority-flood runs in from two rims, the world's edge and the inner rim. Each cell drains down its steepest fall on the filled routing field, or along the flood where it's flat.
  - **Lakes:** a pit in the hills (not on the plain, where the routing field isn't the ground) of 20 cells or more, at least 4 deep, is a lake. Its shore is traced by marching squares.
  - *Superseded:* W3 first ended the water in **meres** at level 0, all on the r3825 circle. Chris found that read as an invisible wall, so #222 replaced them.
  - **Courses:** streams drain 120 cells or more, rivers 600 or more. Each course follows its largest inflow upstream, and every other inflow is a course of its own.
    - On a smooth slope, steepest descent sends channels down side by side. So a channel beside a bigger one (one cell) drains into it, and two drains that cross on a square's diagonals become one (`gather`). No two courses cross (`courses_meet_only_where_one_joins_another`).
    - A course ends at the edge, in a lake or pond (`mouth: lake`), on a point of the course it joins (`mouth: joins`), or petering out (`mouth: peters_out`). A course out of a lake names it as its `source`.
    - A course is cut wherever it passes the river threshold, either way: each part joins the next at its first point.
  - **Shape and surface:** courses are smoothed, then meander in proportion to how flat the ground is, but never more than 0.3 of the way to other water. Each point has a `width`, a `depth` and a `surface`.
    - The surface never rises downstream. Through a pit too small to be a lake, the carve cuts a gorge.
    - Wide and shallow where the fall is gentle, narrow and deep where it's steep.
  - **Fords:** the shallowest points of each river, 1.5 deep at most and at least 600 apart along it.
  - **The carve:** `land::carve` takes `land::Water`, an index built once from `/terrain` (`Water::from_terrain`). `land::ground_at` is the carved ground.
    - A bed is rounded to the surface at the banks, the banks rise at `BANK_SLOPE` (0.6), and a lake's bed shelves to 2 below its level.
    - It only cuts, and it fades out over 30 past the banks, so the ground stays continuous.
  - **The default world:** 41 rivers, 193 streams, 47 lakes (pits and ponds) and 38 fords. Version `b4c60461ef8cc4c4`.

### WD4. Moving on the land ([W4 #197](https://github.com/chrischaps/TheCapitol/issues/197))

- **Slope.** `rules::land_speed(slope)` slows you on a slope. A slope over `CLIFF_SLOPE` blocks you ("Too steep").
- **Rivers and lakes** block like the canals, except at fords and bridges.
- **Streams** can be waded: your speed is multiplied by `WADE_SPEED`.
- **Benches.** `rules::in_water` covers the wild water, so D10 keeps workbenches out of it.
- **As built (#197).** `capitol_protocol::rules::ground`:
  - **`Ground`** indexes the land's params, the wild water and the roads that cross it. `Ground::step` is one step of a walk. The server moves players with it (`GameState::ground`, rebuilt with the terrain), and the Flint client's `Motion` predicts with it (`Motion::set_ground`).
  - **Slope** is the land's own (`height_at`), not the carved ground, so a cut bank isn't a cliff. The speed goes by the slope along your heading: `1 / (1 + 1.5 s)` uphill and `1 / (1 + 0.5 |s|)` downhill.
  - **`CLIFF_SLOPE` is 1.2** (about 50°), measured: the wilds' median slope is 0.22, 95% is under 1.18, and the crags reach 4.9. About 5% of the wilds is steeper than 1.2. Walks are straight lines with no pathfinding, so the threshold is kind on purpose. A step is refused only onto ground steeper than both the cliff and where it starts, so someone left on a crag can climb down.
  - **Water.** A river blocks within its point's `width / 2`, so a petering course barely blocks. A ford is the disc of its `width / 2` on its river. A road over water is a bridge (#222), and you don't wade on it. Streams and fords are waded at `WADE_SPEED` 0.5. A walk stops within a unit of the bank, and someone already in the water can wade out.
  - **On the wire.** `movement_blocked` names the river or lake (`river_3` reads "the river" through `water_name`): "Cannot cross the river without a ford or a bridge", or "Cannot cross the lake: go round it". Or it says "Too steep", with an empty `water_id`. Flint's toast says to find a ford, go round, or find a gentler way up. A cliff doesn't splash.
  - **Benches.** `rules::ground::water_at` (unindexed, for the per-frame ghost) and `Ground::water_at` keep workbenches out of rivers, streams and lakes, fords and bridges included.

### WD5. Trails ([W5 #198](https://github.com/chrischaps/TheCapitol/issues/198))

- **The arterials run on past r3800 as wild trails.** Each is a least-cost path up a valley to a pass, with branches to the fords and a bridge where a trail has to cross a river with no ford.
- **They're ordinary `trail` roads,** so `street_plan::audit` holds and every client already draws them.
- **As built (W5):**
  - **Where they're made:** `backend/src/models/trails.rs`, after the water and the lots, from the land, the water and the roads. The water doesn't skirt the trails; they go round it.
  - **The cost:** least-cost paths on a 25-unit grid from r3775 to r5940. A step costs its length × (1 + 60 × grade²), where the grade is the step's own rise over run, so trails wind along the contours and climb valley floors rather than cross the fall line. Wading a stream costs 30 a cell, a ford 10, and **bridging** a river 400 a cell, so trails cross at fords where they can. A trail over a river away from a ford is the bridge: there's no separate bridge list, and W4 treats a road over water as a crossing.
  - **Out of bounds:** lakes (so a trail meeting one goes round its shore), the lots and roads already there, and ground steeper than `TRAIL_SLOPE` (0.75) at any of a cell's nine half-cell samples (the crests are sharp). Ground to `STEEP_SLOPE` (0.95) is open at 20× the cost, for the way up to a tarn.
  - **Walkable:** every point of every trail is checked at 2-unit steps under `trails::WALK_LIMIT` (0.98), well within W4's `ground::CLIFF_SLOPE` (1.2), so a trail is an easy way up. (W5 first added a `rules::CLIFF_SLOPE` of 1.0; W4 landed its own first, and W5 uses that.)
  - **The network:** a **valley trail** (`wild_trail_{i}`) from each arterial's end to its valley's head (r5200 ± 100, within 20° of its bearing); a **pass trail** (`wild_pass_{i}`) from each head over the massif to the next valley's trail, kept past r4700 and within the two valleys' bearings (fallbacks r4300, r4000). The valley and pass trails have no dead ends.
    - *As it came out:* the massifs rise outwards, so a pass trail runs along the high foothills between valleys rather than over a saddle. Together they make a high road round the wilds, about r4700 to r5200.
  - **Spurs** (`wild_spur_{k}`) go to every lake's shore and every ford the network doesn't already pass. **Climbs** (`wild_climb_{k}`) go to the highest gentle ground in the middle of each massif, where W6's ore is. They're branches of one shortest-path forest, cheapest first, and a later one joins an earlier one where it meets it. A spur to a lake or top that can't be shaped all the way stops short; one to a ford doesn't. Lakes and fords within r4000 (`OPEN_GROUND`) are on the open foot of the hills and get none.
  - **Tarns:** four of the default world's lakes sit in cirques walled by ground steeper than the trails take (0.95). Their spurs end as near as a trail can get, and the test walks on from there under `CLIFF_SLOPE`. Under a cliff slope of 1.0, seed 7 had two tarns no walk could reach (#224). Under W4's 1.2 every lake on the seeds tested can be reached, and the test holds every seed to that.
  - **Shaping:** Douglas-Peucker at 20 (then 10, 5, 0 if the checks fail), then Chaikin's corner cut at the corners over 40° only, then vertices every 20 or less, so another trail can join at a vertex near where it meets. Crossings between trails are welded into shared vertices.
  - **The default world:** 8 valley trails, 8 pass trails, and about 50 spurs and climbs. Version `6e466ea51dc0951e`. No seeded or planned node stands on a trail, so there's no node migration. Generation costs about 1.2 s more in debug, 0.35 s in release. Since #221 the root `Cargo.toml` optimises `capitol_protocol` and libm in debug builds too (the same bits: float maths is never reordered), and the trails take about 0.65 s in debug and the water 0.25 s. Most of both is `land::height_at`.
  - **Tests:** `models::trails::tests`: the slope and lakes, `street_plan::audit` (loose ends only at a spur's lake, ford or top), and every lake and massif reached from the arterials. Seed 7 is checked too.

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
- **As built (W6):**
  - **Four of the six yields.** W6 places the yields D5 v1 already has: iron veins, stone, trees and clay, plus fallen branches under the trees. Copper and tin, reeds, and old growth as a type of its own need D5 content (items and recipes, and a bronze tier) that Chris hasn't settled, so they're [#225](https://github.com/chrischaps/TheCapitol/issues/225). Old growth is the wilds' `tree` for now: the same type, with a quality ceiling that climbs into the ranges.
  - **Where, in content.** `backend/content/wilds.toml` gives each resource a habitat: bounds on height, slope and radius, plus `cover` for woods. Validation checks that every resource with a wilderness spawn row has exactly one habitat. Only the planner reads the file; it isn't synced to a table.
    - veins: height 380 to 900, slope 0.55 to 1.15
    - stone: height 80 to 500, slope 0.45 to 1.0
    - trees: height up to 220, slope up to 0.35, cover 0.35. The woods use the rural woods' noise field, so they run on over the rural edge.
    - branches: slope up to 0.5, under the wild trees
    - clay: slope up to 0.4, on a bank strip 18 to 48 past the edge of any river, stream or lake
  - **Quality.** `[quality]` in `wilds.toml` adds points to a resource's `base_quality_max`, interpolated by radius (−15 at r3800, 0 at r4400, +10 at r5600) and by height (+3 at 250, +8 at 550). The total is capped at 100 and never falls below the spawn row's floor. On the default world, the wild nodes' mean quality is 57 by the rural edge (r3800 to 4300), 63 in the middle (to r5000) and 67 in the ranges.
  - **How it's placed.** `engine::node_spawn::wilds` samples the land on a 25-unit grid. It floods out from the rural edge across every cell a walker can stand in (no steeper than `CLIFF_SLOPE`, not in a lake, and not in a river except at a ford or bridge). It then picks spots in the reached cells that suit each habitat, and checks each spot exactly:
    - inside the habitat
    - no steeper than `CLIFF_SLOPE`
    - 30 from every road and trail
    - 30 from the wild water (clay 18)
    - 30 inside the world's edge
    - 12 from every other node
  - **The wilds are placed last,** after every other zone, so E2.3's 944 nodes and their ids are unchanged.
  - **Counts.** The default world gets 8264 wild nodes, D5's densities over the 67.7 km² of the wilds: trees 4064, branches 2032, stone 1355, veins 542 and clay 271. Clay's wilderness density (4/km², floor 53) is new in `resources.toml`.
  - **Uneven ore.** The veins go where the ranges are high, so some massifs have far more than others. On the default world the western one has very few. Chris judges whether every range should have ore.

## 4. Work packages

| WP | Issue | What | Size | Depends on |
|---|---|---|---|---|
| W0 | [#193](https://github.com/chrischaps/TheCapitol/issues/193) | This plan, and the GDD, economy, Flint and milestones docs reconciled with it | S | – |
| W1 | [#194](https://github.com/chrischaps/TheCapitol/issues/194) | The world grows to 12000: shift migration, radii, bounds, spawn, and `has_plots` on the claim path. **Done.** As built: migration `20261107_001_world_grows.sql`; `rules::in_world` (the r6000 disc) guards both move paths, and Flint's `cap_move` pulls a click past the edge onto it; `claim` checks `has_plots` for the plot's zone and for where it stands. The woods' noise is laid round the tower (`node_spawn::WOODS_ORIGIN`), so E2.3's trees stayed put. The Flint grass mask stays r4000 (`GRASS_EDGE`) until W9. #118: option 1, radii aren't tunable at runtime | M | – |
| W2 | [#195](https://github.com/chrischaps/TheCapitol/issues/195) | `capitol_protocol::land`: the height field. **Done.** As built: Perlin fBm (rolling ground, 70 high) plus squared ridged noise (the ranges, crests near `peak_height` 640, a few summits to about 830), each octave's domain turned by a 3-4-5 rotation so the lattice never shows. The ranges stand in `ridge_count` (8) massifs whose valleys open on the arterials' bearings, ready for W5; they rise from r3900 to full height at r5600. Keyed on the offset from the centre. `LandParams` is on `TerrainConfig` and `/terrain` as `land`, and in `WorldGenParams`, with a seed of its own. `land::tests::sample_heights_are_pinned` pins the bits (checked on Linux too). The relief: `backend/tools/land-relief.ps1` | M | W1 |
| W3 | [#196](https://github.com/chrischaps/TheCapitol/issues/196) | Hydrology: lakes, rivers, streams, fords and the carve, on `/terrain`. **Done.** As built: `models::hydrology` (see WD3's as-built notes): priority-flood from the edge and the foot of the hills, steepest-descent drainage, pits as lakes, and **meres** at the foot where valleys drain towards the city (r3825 and out). `/terrain` has `rivers`/`streams` (`WaterCourse`: points with width, depth and surface; `source`; `mouth` edge/lake/joins), `lakes` (`Lake`: level, even-odd rings) and `fords`. `land::carve`/`ground_at` with the `land::Water` index. Default version `b4c60461ef8cc4c4` (with E3.4's landmarks; #222 ran the water on into the rural ring). `land-relief.ps1` now draws the water over the carved relief (`-Bare` for the land alone) | L | W2 |
| W4 | [#197](https://github.com/chrischaps/TheCapitol/issues/197) | Movement on the land. **Done.** As built: `rules::ground` (see WD4's as-built notes). `Ground::step` for the server and Flint's prediction, `CLIFF_SLOPE` 1.2, `WADE_SPEED` 0.5, and a road over water is a bridge. `movement_blocked` names the river or lake, or says "Too steep" | M | W3 |
| W5 | [#198](https://github.com/chrischaps/TheCapitol/issues/198) | Wild trails. **Done.** As built: `models::trails` (see WD5's as-built notes): least-cost paths costed by grade and water, valley trails to each valley's head, pass trails between valleys along the high foothills, spurs to lakes and fords and climbs to each massif's top. A trail over a river is its bridge. Under `trails::WALK_LIMIT` (0.98), well within W4's `CLIFF_SLOPE` (1.2). Default version `6e466ea51dc0951e`. `land-relief.ps1` draws the trails | M | W3 |
| W6 | [#199](https://github.com/chrischaps/TheCapitol/issues/199) | Wild nodes from the land. **Done.** As built: `engine::node_spawn::wilds` (see WD8's as-built notes). Habitats and the quality table are in `content/wilds.toml`. The wilds are placed after every other zone, on ground a walker can reach from the rural edge, clear of the trails and the water. Migration `20261113_001_wild_nodes.sql` stages 8264 nodes in `resource_node_seeds`. Copper, tin and reeds are [#225](https://github.com/chrischaps/TheCapitol/issues/225). `land-relief.ps1 -Nodes` draws the nodes | L | W3, E2.3 (#162) |
| W7 | [#200](https://github.com/chrischaps/TheCapitol/issues/200) | Flint: water at any height (H6, upstream). **Done.** ADR 0094, [Flint PR #68](https://github.com/chrischaps/Flint/pull/68), merged and pulled. As built: a **water body** is a mesh drawn with the ocean's own shader, through a second vertex stage (`vs_body`). `WaterVertex` carries the rest position, a rest normal and an XZ flow (m/s); `WaterBodyLook` the tint (`None` takes the ocean's), `wave_scale` (the ocean's waves, scaled), `flow_foam` and `flow_period`. Flow is two-phase advection; fast flow foams in white streaks along it; the wave normal tilts with the rest normal down a fall. Bodies draw after the sea, depth-tested, through the same grab pass, so refraction and the shoreline band work on them. `ExtensionGpu::set_water_body(id, &WaterBodyMesh, look)` (and `_look`, `remove_`, `clear_`), with builders `WaterBodyMesh::lake(outline, level)` (simple outline, no holes) and `::river(&[RiverPoint])` (banks at 35% speed), and `river_points(path, width, speed, fall_speedup)` (fastest at a fall's foot). A scene's `water_body` component does the same. **For W10:** waves move vertices only as finely as the mesh is cut; lakes with islands need their own triangulation (the builder takes one simple ring); `ocean_height` knows only the sea; bodies aren't culled, so merge the wilds' water into a few meshes by tint. The sea's look is unchanged (the shoreline demo differs by 1/255 in 11 pixels). Demo: Flint's `demo/water_bodies.scene.toml` | L | – |
| W8 | [#201](https://github.com/chrischaps/TheCapitol/issues/201) | Flint: a ray against a height field (upstream). **Done.** [Flint #67](https://github.com/chrischaps/Flint/pull/67), merged, ADR 0095. As built: `flint_terrain::GroundField`, a `Heightmap` placed in the world (`origin`, `size`, `height_scale`, `base_y`) with `height_at` and `ray`. The ray is clipped to the field's box, stepped a texel at a time and the crossing bisected, and the editor's `ray_heightmap_intersect` uses the same march. `Projector::set_ground(Some(Arc<GroundField>))`, `screen_to_heightfield(x, y)` and `ground_height(x, z)`, with the same two in Rhai (`()` on a miss or off the field). **For W9:** build one field over the whole world from `land` with W3's carve, register it from `world/ground.rs`, and fall back to `screen_to_ground` on `()` | S | – |
| W9 | [#202](https://github.com/chrischaps/TheCapitol/issues/202) | Flint: the height-mapped ground, with everything standing on it. **Done.** As built: `world/land.rs` samples `land::height_at` with W3's carve on a 1 m grid (1537², a square 1536 m across, past the world) on worker threads, off the main thread (about 50 ms on 24 threads), and holds it as one Flint `GroundField`: the projector's ground (W8), and `land::rise`/`ground_y` for everything else. The land is flat inside r3800, the carve fades in over 50 past it, and past the world's edge the ranges settle to flat by r7400, where the flat disc takes over again (it's open between, once the land is in). **The rural ring's water beds aren't cut:** they're W10's, with its water (#203). `world/land_mesh.rs` draws it in 64 m chunks, each at stride 1, 2, 4 or 8 by distance (110, 230 and 460 m, with hysteresis), rebuilt at most 10 a frame, with skirts hiding the cracks; tint by slope and height (meadow, rock past 0.55 to 0.95, scree from 26 m, snow from 46 m on slopes under 0.6 to 1), from the disc's colour at the seam. The disc's fade to the background is gone; the wilds are hill meadow (`palette::WILD_*`). On the land: avatars, nodes (rebaked when it comes), stations and the ghost, the wild trails (draped 0.1 m over it), the marks (`mark_line` in `input.rhai` lifts each end by `cap_rise`), and the grass, whose mask now covers the world (3072², 0.39 m) and keeps off the steeps, the high ground and the beds. The grass stands on its own 2048² height grid, because the engine reads the nearest texel (#228). Clicks pick with `screen_to_heightfield`, falling back to the plane. The rig looks at `cap_ground_at` under the point, keeps 1.5 m over the land under itself, and rises until its line to the point clears the land (sampled every 2.5 m). The map is 880 m up and shows the whole r6000 world; street and district stand where they did. Uncapped (RTX 4070, offline): 4.1 ms at street and 4.7 ms at district zoom in the ranges, against 4.5 and 4.9 ms over the city | L | W2, W8 |
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
| Flint can't draw water above y = 0 | Solved by W7 ([Flint PR #68](https://github.com/chrischaps/Flint/pull/68), merged): water bodies at their own level. |
| A world of 12000 makes the trip too long | `rules` speeds and the trails (W5) tune it. The sim (E2.7) shows where supply pools. |

## 7. Verification

- **Server (W1 to W6):** `cargo test -p capitol-protocol -p capitol-backend`, plus the plan PNGs. `backend/tools/land-relief.ps1` draws the relief, the water (since W3), the trails (since W5) and, with `-Nodes`, the nodes (since W6). Chris judges the PNGs.
- **Client (W9 to W11):** from `flint-client/`, `cargo test -p capitol-core && cargo build --release -p capitol-client`, then captures in `flint-client/renders/` and a live walk from the spawn into the ranges.
