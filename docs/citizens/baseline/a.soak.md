# The town

20 citizens (3 builder, 3 crafter (ingots), 2 crafter (planks), 2 crafter (tools), 4 gatherer (fiber), 3 gatherer (ore), 3 gatherer (wood)) for 14.0 hours of the day's clock, 11.7 hours of ticks at 10× (70.2 wall minutes).

## Conservation

**Holds.**

### Strands

Every account's Strands, with escrow: 2100 at the start, 36196 at the end. That's the start, plus 0 new accounts at 100, plus 34567 in from the faucets, less 471 gone to the sinks, plus 0 the transfers net to: 36196 expected, 0 off.

| Flow | Kind | Strands |
|---|---|---:|
| bazaar_rent | Sink | -200 |
| exchange_deposit | Faucet | +34567 |
| market_escrow | Escrow | -2041 |
| market_fee | Sink | -271 |
| market_refund | Escrow | +1559 |
| market_sale | Escrow | +477 |
| trade_received | Transfer | +14 |
| trade_sent | Transfer | -14 |

Escrow: 0 at the start, 5 at the end; 0 unexplained.

### Units

| Item | Change | Explained | Drift |
|---|---:|---:|---:|
| brick | +990 | +990 | 0 |
| charcoal | +3 | +3 | 0 |
| chest_kit | +1 | +1 | 0 |
| clay | +7 | +7 | 0 |
| fiber | +313 | +313 | 0 |
| iron_axe | +2 | +2 | 0 |
| iron_hammer | +2 | +2 | 0 |
| iron_ingot | +292 | +292 | 0 |
| iron_ore | +1524 | +1524 | 0 |
| iron_pickaxe | +2 | +2 | 0 |
| iron_sickle | +2 | +2 | 0 |
| kiln_kit | +2 | +2 | 0 |
| knapped_stone | +2 | +2 | 0 |
| log | +8623 | +8623 | 0 |
| plank | +1765 | +1765 | 0 |
| rope | +1 | +1 | 0 |
| sawbench_kit | +1 | +1 | 0 |
| sticks | +13 | +13 | 0 |
| stone | +5 | +5 | 0 |
| stone_axe | +18 | +18 | 0 |
| stone_hammer | +10 | +10 | 0 |
| stone_pickaxe | +11 | +11 | 0 |
| stone_sickle | +2 | +2 | 0 |
| tool_handle | +6433 | +6433 | 0 |

| Flow | Units |
|---|---:|
| consumed | -17101 |
| crafted | +15985 |
| deposited | -34653 |
| gathered | +56057 |
| kit_placed | -62 |
| kit_returned | +4 |
| worn_out | -206 |

## Load

| | Whole soak |
|---|---|
| Tick budget | 10.0 ms |
| Ticks | 421616 |
| Tick time, mean | 0.20 ms |
| Tick time, p50 / p95 / p99 | ≤ 0.5 ms / ≤ 0.5 ms / ≤ 0.5 ms |
| Slowest tick | 77.8 ms |
| Ticks over budget | 4 (0.00%) |
| Worst minute, mean tick | 0.23 ms (p99 ≤ 0.5 ms) |
| Most in the world | 20 |
| Sockets opened / most open | 20 / 20 |
| Sockets dropped | 0 |
| Sockets resynced | 0 |
| DB transactions a second, mean / busiest minute | 271 / 366 |
| DB pool at the end, size / idle | 16 / 15 |

Sockets that ended: 20 closed.

## Trades

| Trade | Citizens | Strands at the start | At the end | Change each | Made | Sold (Strands) | Bought (Strands) | Deeds | Stuck |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| builder | 3 | 300 | 53 | -82 | 3364 | 3 (24) | 33 (257) | 3 | 833 |
| crafter (ingots) | 3 | 300 | 150 | -50 | 2010 | 1 (8) | 14 (146) | 3 | 846 |
| crafter (planks) | 2 | 200 | 178 | -11 | 2753 | 2 (16) | 2 (16) | 2 | 664 |
| crafter (tools) | 2 | 200 | 136 | -32 | 15971 | 0 (0) | 4 (56) | 2 | 0 |
| gatherer (fiber) | 4 | 400 | 34933 | +8633 | 35054 | 4 (4) | 1 (1) | 4 | 0 |
| gatherer (ore) | 3 | 300 | 243 | -19 | 2400 | 6 (113) | 0 (0) | 2 | 0 |
| gatherer (wood) | 3 | 300 | 398 | +33 | 10490 | 39 (312) | 1 (1) | 3 | 0 |

In trade windows: 0 trades with people, 2 between citizens (each asked by one), with 14 Strands changing hands between citizens.

## Citizens

| Citizen | Trade | Strands | Made (most) | Sold (Strands) | Bought (Strands) | Deeds | Failed steps | Stuck (why, most often) |
|---|---|---:|---|---:|---:|---:|---:|---|
| Ada Rushton | gatherer (fiber) | 100 → 7880 | 7843 (7828 fiber) | 1 (1) | 1 (1) | 1 | 299 | 0 |
| Leonard Forester | gatherer (wood) | 100 → 82 | 3658 (3035 log) | 9 (72) | 0 (0) | 1 | 183 | 0 |
| Doris Miner | gatherer (ore) | 100 → 121 | 1297 (839 iron_ore) | 4 (75) | 0 (0) | 1 | 6 | 0 |
| Phyllis Joiner | crafter (planks) | 100 → 99 | 167 (36 fiber) | 1 (8) | 0 (0) | 1 | 8 | 375 (the kiln waits for a plot, and the plot for the road in's deed) |
| Harold Ironside | crafter (ingots) | 100 → 21 | 639 (177 iron_ore) | 0 (0) | 4 (73) | 1 | 9 | 312 (the kiln waits for a plot, and the plot for the road in's deed) |
| Agnes Toller | crafter (tools) | 100 → 67 | 7730 (3197 tool_handle) | 0 (0) | 2 (28) | 1 | 9 | 0 |
| Lillian Mason | builder | 100 → 15 | 1379 (441 clay) | 1 (8) | 11 (88) | 1 | 3 | 277 (the kiln waits for a plot, and the plot for the road in's deed) |
| Dorothy Reeder | gatherer (fiber) | 100 → 10335 | 10368 (10353 fiber) | 1 (1) | 0 (0) | 1 | 526 | 0 |
| Reginald Ashby | gatherer (wood) | 100 → 163 | 3163 (2609 log) | 15 (120) | 0 (0) | 1 | 616 | 0 |
| Hazel Leadbetter | gatherer (ore) | 100 → 100 | 56 (26 iron_ore) | 0 (0) | 0 (0) | 0 | 2012 | 0 |
| Albert Turner | crafter (planks) | 100 → 79 | 2586 (1764 plank) | 1 (8) | 2 (16) | 1 | 9 | 289 (the kiln waits for a plot, and the plot for the road in's deed) |
| Lionel Furness | crafter (ingots) | 100 → 104 | 605 (172 iron_ore) | 1 (8) | 0 (0) | 1 | 18 | 298 (the kiln waits for a plot, and the plot for the road in's deed) |
| Edith Wright | crafter (tools) | 100 → 69 | 8241 (3276 tool_handle) | 0 (0) | 2 (28) | 1 | 18 | 0 |
| Rose Thatcher | builder | 100 → 15 | 997 (302 clay) | 1 (8) | 11 (88) | 1 | 0 | 306 (the kiln waits for a plot, and the plot for the road in's deed) |
| Herbert Flaxman | gatherer (fiber) | 100 → 9448 | 9621 (9606 fiber) | 1 (1) | 0 (0) | 1 | 473 | 0 |
| Alice Hewitt | gatherer (wood) | 100 → 153 | 3669 (3005 log) | 15 (120) | 1 (1) | 1 | 562 | 0 |
| Mabel Delve | gatherer (ore) | 100 → 22 | 1047 (662 iron_ore) | 2 (38) | 0 (0) | 1 | 11 | 0 |
| Edwin Smith | crafter (ingots) | 100 → 25 | 766 (232 iron_ore) | 0 (0) | 10 (73) | 1 | 14 | 236 (the kiln waits for a plot, and the plot for the road in's deed) |
| Ruth Slater | builder | 100 → 23 | 988 (305 clay) | 1 (8) | 11 (81) | 1 | 6 | 250 (the kiln waits for a plot, and the plot for the road in's deed) |
| Hilda Hayward | gatherer (fiber) | 100 → 7270 | 7222 (7207 fiber) | 1 (1) | 0 (0) | 1 | 268 | 0 |

## Time by activity

What each trade's and each citizen's work went on, as a share of its work time. Free is the share of all its time that had nothing to do in it, market hours with nothing to trade among it. Setup is the share of the work on tools, kits, stations and deeds on the way to something else. A day is the day's clock's: jobs are the eight a citizen can lean over, and activities those and the Commons, detours and the road in, for each day it worked (the mean, and the fewest).

| Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| builder | 17.9 |  |  |  |  |  |  | 99% |  | 1% | 47% | 0% | 1.0 (1) | 2.0 (2) |
| crafter (ingots) | 18.7 |  |  |  |  | 97% |  |  |  | 3% | 44% | 2% | 1.0 (1) | 2.0 (2) |
| crafter (planks) | 5.7 |  |  |  | 98% |  |  |  |  | 2% | 75% | 5% | 1.0 (1) | 2.0 (2) |
| crafter (tools) | 21.4 |  |  |  |  |  | 99% |  |  | 1% | 6% | 1% | 1.0 (1) | 2.0 (2) |
| gatherer (fiber) | 42.4 | 99% |  |  |  |  |  |  |  | 1% | 6% |  | 1.0 (1) | 2.0 (2) |
| gatherer (ore) | 26.9 | 0% |  | 93% |  |  |  |  | 3% | 4% |  | 1% | 2.0 (1) | 3.0 (2) |
| gatherer (wood) | 28.1 | 0% | 92% |  |  |  |  |  | 7% | 1% | 5% | 2% | 2.3 (2) | 3.3 (3) |

| Citizen | Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Ada Rushton | gatherer (fiber) | 10.7 | 98% |  |  |  |  |  |  |  | 2% | 5% |  | 1.0 (1) | 2.0 (2) |
| Leonard Forester | gatherer (wood) | 9.4 | 0% | 92% |  |  |  |  |  | 7% | 1% | 13% | 2% | 3.0 (3) | 4.0 (4) |
| Doris Miner | gatherer (ore) | 11.2 |  |  | 94% |  |  |  |  | 3% | 3% |  | 2% | 2.0 (2) | 3.0 (3) |
| Phyllis Joiner | crafter (planks) | 1.0 |  |  |  | 95% |  |  |  |  | 5% | 91% | 13% | 1.0 (1) | 2.0 (2) |
| Harold Ironside | crafter (ingots) | 6.0 |  |  |  |  | 96% |  |  |  | 4% | 47% | 2% | 1.0 (1) | 2.0 (2) |
| Agnes Toller | crafter (tools) | 10.2 |  |  |  |  |  | 99% |  |  | 1% | 12% | 1% | 1.0 (1) | 2.0 (2) |
| Lillian Mason | builder | 6.6 |  |  |  |  |  |  | 98% |  | 2% | 41% | 0% | 1.0 (1) | 2.0 (2) |
| Dorothy Reeder | gatherer (fiber) | 11.1 | 98% |  |  |  |  |  |  |  | 2% |  |  | 1.0 (1) | 2.0 (2) |
| Reginald Ashby | gatherer (wood) | 9.2 |  | 91% |  |  |  |  |  | 7% | 2% |  | 2% | 2.0 (2) | 3.0 (3) |
| Hazel Leadbetter | gatherer (ore) | 4.5 |  |  | 90% |  |  |  |  |  | 10% |  | 0% | 1.0 (1) | 2.0 (2) |
| Albert Turner | crafter (planks) | 4.7 |  |  |  | 99% |  |  |  |  | 1% | 59% | 4% | 1.0 (1) | 2.0 (2) |
| Lionel Furness | crafter (ingots) | 6.3 |  |  |  |  | 98% |  |  |  | 2% | 44% | 2% | 1.0 (1) | 2.0 (2) |
| Edith Wright | crafter (tools) | 11.2 |  |  |  |  |  | 99% |  |  | 1% |  | 1% | 1.0 (1) | 2.0 (2) |
| Rose Thatcher | builder | 6.1 |  |  |  |  |  |  | 99% |  | 1% | 45% | 0% | 1.0 (1) | 2.0 (2) |
| Herbert Flaxman | gatherer (fiber) | 11.1 | 99% |  |  |  |  |  |  |  | 1% |  |  | 1.0 (1) | 2.0 (2) |
| Alice Hewitt | gatherer (wood) | 9.5 |  | 92% |  |  |  |  |  | 6% | 1% |  | 2% | 2.0 (2) | 3.0 (3) |
| Mabel Delve | gatherer (ore) | 11.2 | 0% |  | 93% |  |  |  |  | 5% | 3% |  | 1% | 3.0 (3) | 4.0 (4) |
| Edwin Smith | crafter (ingots) | 6.5 |  |  |  |  | 97% |  |  |  | 3% | 42% | 3% | 1.0 (1) | 2.0 (2) |
| Ruth Slater | builder | 5.2 |  |  |  |  |  |  | 98% |  | 2% | 55% | 0% | 1.0 (1) | 2.0 (2) |
| Hilda Hayward | gatherer (fiber) | 9.5 | 99% |  |  |  |  |  |  |  | 1% | 18% |  | 1.0 (1) | 2.0 (2) |

## How questions were settled

| Settled by | Questions | Share |
|---|---:|---:|
| rules | 52062 | 92.2% |
| rules, unsure | 4374 | 7.8% |

## Chain bottlenecks

- `gather fiber` took 27% of all bot time (45.7 bot-hours over 20425 steps, 1647 failed)
- `gather iron_ore` took 14% of all bot time (24.6 bot-hours over 4354 steps, 44 failed)
- `gather log` took 13% of all bot time (22.1 bot-hours over 8147 steps, 108 failed)
- iron_ore failed 48% of its pulls (2092 of 4310)
- stone failed 46% of its pulls (987 of 2102)
- `craft knapped_stone` failed 79% of the time
- `stock log` failed 94% of the time
- `the road in: make a stone axe` failed 53% of the time
- `sell iron_ore` failed 75% of the time
- `the road in: fell a tree` failed 89% of the time
- `the road in: read someone's card` failed 38% of the time
- stuck 2262×: the kiln waits for a plot, and the plot for the road in's deed
- stuck 81×: the plot has no room for a sawbench

## Production

| Item | Gathered | Crafted | Per game hour | p10 | p50 | p90 | First at (h) | s a unit (D5) | Traded | Low | Mean price (D5) | High |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| anvil_kit | 0 | 5 | 0.4 | 44 | 46 | 53 | 5.61 |  |  |  |  |  |
| brick | 0 | 1110 | 94.9 | 49 | 61 | 69 | 4.61 |  |  |  |  |  |
| charcoal | 0 | 908 | 77.6 | 38 | 52 | 67 | 4.60 |  |  |  |  |  |
| chest_kit | 0 | 3 | 0.3 | 44 | 46 | 64 | 0.23 |  |  |  |  |  |
| clay | 1189 | 0 | 101.6 | 45 | 65 | 78 | 0.13 | 18 (12) |  |  |  |  |
| fiber | 38324 | 0 | 3276.3 | 42 | 53 | 69 | 0.01 | 4 (4) | 4 | 1 | 1.0 (1) | 1 |
| forge_kit | 0 | 10 | 0.9 | 53 | 56 | 60 | 4.80 |  |  |  |  |  |
| iron_axe | 0 | 2 | 0.2 | 56 | 56 | 57 | 5.82 |  |  |  |  |  |
| iron_axe_head | 0 | 2 | 0.2 | 53 | 53 | 56 | 5.81 |  |  |  |  |  |
| iron_hammer | 0 | 2 | 0.2 | 50 | 50 | 55 | 5.89 |  |  |  |  |  |
| iron_hammer_head | 0 | 2 | 0.2 | 44 | 44 | 50 | 5.88 |  |  |  |  |  |
| iron_ingot | 0 | 350 | 29.9 | 39 | 53 | 65 | 4.95 |  |  |  |  |  |
| iron_nails | 0 | 70 | 6.0 | 40 | 49 | 56 | 4.95 |  |  |  |  |  |
| iron_ore | 2224 | 0 | 190.1 | 23 | 53 | 77 | 0.15 | 40 (90) | 6 | 17 | 18.8 (20) | 20 |
| iron_pickaxe | 0 | 2 | 0.2 | 55 | 55 | 56 | 5.75 |  |  |  |  |  |
| iron_pickaxe_head | 0 | 2 | 0.2 | 50 | 50 | 53 | 5.74 |  |  |  |  |  |
| iron_sickle | 0 | 2 | 0.2 | 49 | 49 | 60 | 5.93 |  |  |  |  |  |
| iron_sickle_blade | 0 | 2 | 0.2 | 47 | 47 | 59 | 5.92 |  |  |  |  |  |
| kiln_kit | 0 | 12 | 1.0 | 52 | 55 | 58 | 0.17 |  |  |  |  |  |
| knapped_stone | 0 | 483 | 41.3 | 47 | 54 | 63 | 0.02 |  |  |  |  |  |
| log | 10608 | 0 | 906.9 | 32 | 48 | 71 | 0.06 | 8 (32) | 45 | 8 | 8.0 (8) | 8 |
| plank | 0 | 4509 | 385.5 | 40 | 52 | 67 | 5.59 |  |  |  |  |  |
| rope | 0 | 1542 | 131.8 | 42 | 56 | 65 | 0.04 |  |  |  |  |  |
| sawbench_kit | 0 | 7 | 0.6 | 42 | 53 | 55 | 5.03 |  |  |  |  |  |
| sticks | 2541 | 0 | 217.2 | 30 | 49 | 60 | 0.04 | 3 (12) |  |  |  |  |
| stone | 1171 | 0 | 100.1 | 35 | 52 | 70 | 0.02 | 25 (22) |  |  |  |  |
| stone_axe | 0 | 148 | 12.7 | 49 | 54 | 58 | 0.06 |  |  |  |  |  |
| stone_hammer | 0 | 10 | 0.9 | 51 | 54 | 59 | 0.05 |  |  |  |  |  |
| stone_pickaxe | 0 | 76 | 6.5 | 49 | 55 | 60 | 0.13 |  |  |  |  |  |
| stone_sickle | 0 | 13 | 1.1 | 51 | 57 | 62 | 0.26 |  |  |  |  |  |
| tool_handle | 0 | 6688 | 571.8 | 41 | 52 | 61 | 0.05 |  |  |  |  |  |
| workbench_kit | 0 | 25 | 2.1 | 53 | 59 | 69 | 0.02 |  |  |  |  |  |

## Pulls that failed

| Item | Pulls | Failed | Share |
|---|---:|---:|---:|
| clay | 796 | 0 | 0% |
| fiber | 18819 | 0 | 0% |
| iron_ore | 4310 | 2092 | 49% |
| log | 8052 | 1168 | 15% |
| sticks | 1114 | 0 | 0% |
| stone | 2102 | 987 | 47% |

## Tools worn out

- stone_axe: 130
- stone_pickaxe: 65
- stone_sickle: 11

## Strands

- Fiber deposited: 34653
- Deeds bought: 19 for 0 Strands
- Spent on tax: 0 Strands
- Market fees: 271 Strands
- A deed (500, in any home zone) is 0.60 hours of grass

## Where the time went

| Step | Bot-hours | Steps | Failed |
|---|---:|---:|---:|
| gather fiber | 45.71 | 20425 | 1647 |
| gather iron_ore | 24.65 | 4354 | 44 |
| gather log | 22.10 | 8147 | 108 |
| craft knapped_stone | 9.87 | 2220 | 1755 |
| craft tool_handle | 9.45 | 3955 | 0 |
| gather stone | 8.24 | 2061 | 16 |
| craft charcoal | 6.80 | 454 | 0 |
| at the hall | 6.61 | 389 | 0 |
| gather clay | 5.94 | 796 | 0 |
| craft brick | 4.57 | 555 | 0 |
| craft plank | 4.52 | 1503 | 0 |
| craft iron_ingot | 4.45 | 350 | 0 |
| at the bazaar | 3.56 | 210 | 0 |
| craft rope | 2.32 | 1524 | 0 |
| gather sticks | 2.12 | 1101 | 0 |
| stock log | 1.94 | 1282 | 1210 |
| stock iron_ore | 0.90 | 19 | 0 |
| the road in: make a stone axe | 0.80 | 216 | 116 |
| craft stone_axe | 0.74 | 139 | 0 |
| buy log | 0.72 | 21 | 0 |
| sell log | 0.56 | 18 | 0 |
| craft stone_pickaxe | 0.41 | 76 | 0 |
| the road in: look at a farm by the road | 0.40 | 22 | 0 |
| the road in: set a station up at home | 0.38 | 40 | 3 |
| craft kiln_kit | 0.34 | 12 | 0 |
| the road in: change fiber for Strands at the Exchange | 0.33 | 26 | 2 |
| sell iron_ore | 0.30 | 28 | 21 |
| the road in: fell a tree | 0.29 | 129 | 116 |
| buy iron_ore | 0.24 | 7 | 0 |
| to the hall | 0.19 | 5 | 0 |
| the road in: read someone's card | 0.18 | 31 | 12 |
| place kiln | 0.16 | 10 | 0 |
| the road in: see to the market box | 0.16 | 19 | 0 |
| bid for log | 0.16 | 5 | 0 |
| the road in: look at an urban yard by the road | 0.14 | 19 | 0 |
| craft forge_kit | 0.12 | 10 | 0 |
| the road in: set the workbench up off any lot | 0.11 | 8 | 0 |
| craft sawbench_kit | 0.10 | 7 | 0 |
| the road in: look at a suburban lot by the road | 0.10 | 19 | 0 |
| place forge | 0.08 | 10 | 0 |
| to the bazaar | 0.07 | 2 | 0 |
| the road in: leave a sell order on the book | 0.07 | 19 | 0 |
| the road in: look in at the Deeds Office | 0.06 | 19 | 0 |
| craft anvil_kit | 0.06 | 5 | 0 |
| the road in: gather stone by hand | 0.06 | 10 | 0 |
| bid for iron_ore | 0.06 | 2 | 0 |
| craft stone_hammer | 0.05 | 10 | 0 |
| craft workbench_kit | 0.05 | 16 | 0 |
| craft stone_sickle | 0.04 | 13 | 0 |
| the road in: say a word to whoever's near | 0.04 | 19 | 0 |
| the road in: knap stone at the bench | 0.03 | 11 | 0 |
| place anvil | 0.03 | 5 | 0 |
| craft iron_nails | 0.03 | 7 | 0 |
| place sawbench | 0.02 | 6 | 0 |
| place workbench | 0.02 | 16 | 0 |
| craft iron_pickaxe | 0.02 | 2 | 0 |
| craft iron_sickle | 0.02 | 2 | 0 |
| craft iron_hammer | 0.02 | 2 | 0 |
| craft iron_axe | 0.02 | 2 | 0 |
| the road in: claim a home with its deed | 0.02 | 19 | 0 |
| the road in: look in at a bazaar stall | 0.01 | 19 | 0 |
| the road in: make a handle at the bench | 0.01 | 5 | 0 |
| craft iron_axe_head | 0.01 | 2 | 0 |
| craft iron_pickaxe_head | 0.01 | 2 | 0 |
| craft iron_hammer_head | 0.01 | 2 | 0 |
| the road in: make a workbench kit | 0.01 | 4 | 0 |
| deposit fiber | 0.01 | 188 | 0 |
| craft iron_sickle_blade | 0.01 | 2 | 0 |
| the road in: pick up fallen branches | 0.01 | 4 | 0 |
| craft chest_kit | 0.01 | 3 | 0 |
| the road in: twist a rope | 0.01 | 4 | 0 |
| place storage_chest | 0.00 | 2 | 0 |
| the road in: read the Registrar's letter | 0.00 | 19 | 0 |
| the road in: give a station a job | 0.00 | 19 | 0 |
| the road in: read the Gazette | 0.00 | 19 | 0 |
| the road in: leave a buy order on the book | 0.00 | 19 | 0 |
| the road in: read the zone card | 0.00 | 19 | 0 |
| the road in: look in its pack | 0.00 | 20 | 0 |
| the road in: read what tax asks | 0.00 | 19 | 0 |

## Why steps failed

| Step: reason | Times |
|---|---:|
| craft knapped_stone: blocked at (#, #): Too steep | 1755 |
| stock log: A shop holds # listings at most | 1210 |
| gather fiber: the server didn't start extracting node #: Someone else is already gathering this Wild Grass | 1204 |
| gather fiber: the server didn't start extracting node #: This Wild Grass is spent; it will grow back | 442 |
| the road in: fell a tree: blocked at (#, #): Too steep | 116 |
| the road in: make a stone axe: blocked at (#, #): Too steep | 116 |
| gather log: the server didn't start extracting node #: Someone else is already gathering this Tree | 69 |
| gather log: the server didn't start extracting node #: This Tree is spent; it will grow back | 33 |
| gather iron_ore: the server didn't start extracting node #: This Iron Vein is spent; it will grow back | 31 |
| sell iron_ore: blocked at (#, #): Too steep | 21 |
| gather iron_ore: the server didn't start extracting node #: Someone else is already gathering this Iron Vein | 12 |
| the road in: read someone's card: nobody about, nor in the Gazette, whose card to read | 12 |
