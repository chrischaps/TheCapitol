# The town

20 citizens (3 builder, 3 crafter (ingots), 2 crafter (planks), 2 crafter (tools), 4 gatherer (fiber), 3 gatherer (ore), 3 gatherer (wood)) for 14.0 hours of the day's clock, 11.7 hours of ticks at 10× (70.1 wall minutes).

## Conservation

**Holds.**

### Strands

Every account's Strands, with escrow: 2100 at the start, 35853 at the end. That's the start, plus 0 new accounts at 100, plus 34273 in from the faucets, less 520 gone to the sinks, plus 0 the transfers net to: 35853 expected, 0 off.

| Flow | Kind | Strands |
|---|---|---:|
| bazaar_rent | Sink | -240 |
| exchange_deposit | Faucet | +34273 |
| market_escrow | Escrow | -2112 |
| market_fee | Sink | -280 |
| market_refund | Escrow | +1626 |
| market_sale | Escrow | +481 |
| trade_received | Transfer | +14 |
| trade_sent | Transfer | -14 |

Escrow: 0 at the start, 5 at the end; 0 unexplained.

### Units

| Item | Change | Explained | Drift |
|---|---:|---:|---:|
| anvil_kit | +1 | +1 | 0 |
| brick | +984 | +984 | 0 |
| charcoal | +7 | +7 | 0 |
| chest_kit | +1 | +1 | 0 |
| clay | +10 | +10 | 0 |
| fiber | +323 | +323 | 0 |
| iron_axe | +1 | +1 | 0 |
| iron_hammer | +1 | +1 | 0 |
| iron_ingot | +282 | +282 | 0 |
| iron_ore | +2337 | +2337 | 0 |
| iron_pickaxe | +1 | +1 | 0 |
| iron_sickle | +1 | +1 | 0 |
| knapped_stone | +3 | +3 | 0 |
| log | +8512 | +8512 | 0 |
| plank | +1791 | +1791 | 0 |
| rope | +1 | +1 | 0 |
| sawbench_kit | +1 | +1 | 0 |
| sticks | +12 | +12 | 0 |
| stone | +2 | +2 | 0 |
| stone_axe | +19 | +19 | 0 |
| stone_hammer | +10 | +10 | 0 |
| stone_pickaxe | +11 | +11 | 0 |
| stone_sickle | +2 | +2 | 0 |
| tool_handle | +5940 | +5940 | 0 |
| workbench_kit | +1 | +1 | 0 |

| Flow | Units |
|---|---:|
| consumed | -16876 |
| crafted | +15359 |
| deposited | -34361 |
| gathered | +56422 |
| kit_placed | -64 |
| kit_returned | +6 |
| worn_out | -232 |

## Load

| | Whole soak |
|---|---|
| Tick budget | 10.0 ms |
| Ticks | 420937 |
| Tick time, mean | 0.21 ms |
| Tick time, p50 / p95 / p99 | ≤ 0.5 ms / ≤ 0.5 ms / ≤ 0.5 ms |
| Slowest tick | 4317.7 ms |
| Ticks over budget | 2 (0.00%) |
| Worst minute, mean tick | 0.91 ms (p99 ≤ 1 ms) |
| Most in the world | 20 |
| Sockets opened / most open | 20 / 20 |
| Sockets dropped | 0 |
| Sockets resynced | 0 |
| DB transactions a second, mean / busiest minute | 274 / 366 |
| DB pool at the end, size / idle | 16 / 15 |

Sockets that ended: 20 closed.

## Trades

| Trade | Citizens | Strands at the start | At the end | Change each | Made | Sold (Strands) | Bought (Strands) | Deeds | Stuck |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| builder | 3 | 300 | 121 | -60 | 3359 | 3 (24) | 22 (176) | 3 | 832 |
| crafter (ingots) | 3 | 300 | 189 | -37 | 1948 | 2 (16) | 8 (110) | 3 | 843 |
| crafter (planks) | 2 | 200 | 153 | -24 | 2787 | 2 (16) | 7 (56) | 2 | 679 |
| crafter (tools) | 2 | 200 | 49 | -76 | 14874 | 0 (0) | 11 (136) | 2 | 0 |
| gatherer (fiber) | 4 | 400 | 34639 | +8560 | 34781 | 4 (4) | 1 (1) | 4 | 0 |
| gatherer (ore) | 3 | 300 | 262 | -13 | 3607 | 9 (173) | 0 (0) | 3 | 0 |
| gatherer (wood) | 3 | 300 | 335 | +12 | 10425 | 31 (248) | 2 (2) | 3 | 0 |

In trade windows: 0 trades with people, 2 between citizens (each asked by one), with 14 Strands changing hands between citizens.

## Citizens

| Citizen | Trade | Strands | Made (most) | Sold (Strands) | Bought (Strands) | Deeds | Failed steps | Stuck (why, most often) |
|---|---|---:|---|---:|---:|---:|---:|---|
| Ada Rushton | gatherer (fiber) | 100 → 7892 | 7930 (7915 fiber) | 1 (1) | 1 (1) | 1 | 306 | 0 |
| Leonard Forester | gatherer (wood) | 100 → 61 | 3641 (2978 log) | 1 (8) | 0 (0) | 1 | 65 | 0 |
| Doris Miner | gatherer (ore) | 100 → 122 | 1170 (760 iron_ore) | 5 (93) | 0 (0) | 1 | 18 | 0 |
| Phyllis Joiner | crafter (planks) | 100 → 50 | 163 (35 fiber) | 1 (8) | 7 (56) | 1 | 1 | 377 (the kiln waits for a plot, and the plot for the road in's deed) |
| Harold Ironside | crafter (ingots) | 100 → 21 | 639 (180 iron_ore) | 0 (0) | 4 (73) | 1 | 16 | 309 (the kiln waits for a plot, and the plot for the road in's deed) |
| Agnes Toller | crafter (tools) | 100 → 32 | 6337 (2476 tool_handle) | 0 (0) | 7 (56) | 1 | 7 | 0 |
| Lillian Mason | builder | 100 → 16 | 1398 (449 clay) | 1 (8) | 11 (88) | 1 | 0 | 276 (the kiln waits for a plot, and the plot for the road in's deed) |
| Dorothy Reeder | gatherer (fiber) | 100 → 10565 | 10575 (10560 fiber) | 1 (1) | 0 (0) | 1 | 564 | 0 |
| Reginald Ashby | gatherer (wood) | 100 → 197 | 3207 (2640 log) | 20 (160) | 1 (1) | 1 | 604 | 0 |
| Hazel Leadbetter | gatherer (ore) | 100 → 79 | 1235 (810 iron_ore) | 1 (20) | 0 (0) | 1 | 30 | 0 |
| Albert Turner | crafter (planks) | 100 → 103 | 2624 (1788 plank) | 1 (8) | 0 (0) | 1 | 1 | 302 (the kiln waits for a plot, and the plot for the road in's deed) |
| Lionel Furness | crafter (ingots) | 100 → 95 | 606 (172 iron_ore) | 1 (8) | 1 (8) | 1 | 14 | 288 (the kiln waits for a plot, and the plot for the road in's deed) |
| Edith Wright | crafter (tools) | 100 → 17 | 8537 (3499 tool_handle) | 0 (0) | 4 (80) | 1 | 22 | 0 |
| Rose Thatcher | builder | 100 → 16 | 951 (287 clay) | 1 (8) | 11 (88) | 1 | 7 | 307 (the kiln waits for a plot, and the plot for the road in's deed) |
| Herbert Flaxman | gatherer (fiber) | 100 → 9669 | 9763 (9748 fiber) | 1 (1) | 0 (0) | 1 | 398 | 0 |
| Alice Hewitt | gatherer (wood) | 100 → 77 | 3577 (2910 log) | 10 (80) | 1 (1) | 1 | 489 | 0 |
| Mabel Delve | gatherer (ore) | 100 → 61 | 1202 (774 iron_ore) | 3 (60) | 0 (0) | 1 | 16 | 0 |
| Edwin Smith | crafter (ingots) | 100 → 73 | 703 (209 iron_ore) | 1 (8) | 3 (29) | 1 | 28 | 246 (the kiln waits for a plot, and the plot for the road in's deed) |
| Ruth Slater | builder | 100 → 89 | 1010 (310 clay) | 1 (8) | 0 (0) | 1 | 3 | 249 (the kiln waits for a plot, and the plot for the road in's deed) |
| Hilda Hayward | gatherer (fiber) | 100 → 6513 | 6513 (6498 fiber) | 1 (1) | 0 (0) | 1 | 225 | 0 |

## Time by activity

What each trade's and each citizen's work went on, as a share of its work time. Free is the share of all its time that had nothing to do in it, market hours with nothing to trade among it. Setup is the share of the work on tools, kits, stations and deeds on the way to something else. A day is the day's clock's: jobs are the eight a citizen can lean over, and activities those and the Commons, detours and the road in, for each day it worked (the mean, and the fewest).

| Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| builder | 17.9 |  |  |  |  |  |  | 99% |  | 1% | 48% | 0% | 1.0 (1) | 2.0 (2) |
| crafter (ingots) | 18.3 |  |  |  |  | 97% |  |  |  | 3% | 45% | 2% | 1.0 (1) | 2.0 (2) |
| crafter (planks) | 5.4 |  |  |  | 98% |  |  |  |  | 2% | 76% | 4% | 1.0 (1) | 2.0 (2) |
| crafter (tools) | 21.4 |  |  |  |  |  | 99% |  |  | 1% | 6% | 1% | 1.0 (1) | 2.0 (2) |
| gatherer (fiber) | 42.5 | 99% |  |  |  |  |  |  |  | 1% | 6% |  | 1.0 (1) | 2.0 (2) |
| gatherer (ore) | 33.6 |  |  | 94% |  |  |  |  | 4% | 2% |  | 1% | 2.0 (2) | 3.0 (3) |
| gatherer (wood) | 29.3 |  | 92% |  |  |  |  |  | 7% | 1% | 5% | 2% | 2.0 (2) | 3.0 (3) |

| Citizen | Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Ada Rushton | gatherer (fiber) | 10.7 | 99% |  |  |  |  |  |  |  | 1% | 5% |  | 1.0 (1) | 2.0 (2) |
| Leonard Forester | gatherer (wood) | 9.9 |  | 92% |  |  |  |  |  | 7% | 1% | 14% | 2% | 2.0 (2) | 3.0 (3) |
| Doris Miner | gatherer (ore) | 11.2 |  |  | 94% |  |  |  |  | 4% | 2% |  | 1% | 2.0 (2) | 3.0 (3) |
| Phyllis Joiner | crafter (planks) | 0.9 |  |  |  | 94% |  |  |  |  | 6% | 92% | 10% | 1.0 (1) | 2.0 (2) |
| Harold Ironside | crafter (ingots) | 6.0 |  |  |  |  | 97% |  |  |  | 3% | 47% | 3% | 1.0 (1) | 2.0 (2) |
| Agnes Toller | crafter (tools) | 10.2 |  |  |  |  |  | 99% |  |  | 1% | 12% | 1% | 1.0 (1) | 2.0 (2) |
| Lillian Mason | builder | 6.6 |  |  |  |  |  |  | 99% |  | 1% | 41% | 0% | 1.0 (1) | 2.0 (2) |
| Dorothy Reeder | gatherer (fiber) | 11.1 | 98% |  |  |  |  |  |  |  | 2% |  |  | 1.0 (1) | 2.0 (2) |
| Reginald Ashby | gatherer (wood) | 9.3 |  | 91% |  |  |  |  |  | 7% | 2% |  | 2% | 2.0 (2) | 3.0 (3) |
| Hazel Leadbetter | gatherer (ore) | 11.2 |  |  | 94% |  |  |  |  | 3% | 2% |  | 2% | 2.0 (2) | 3.0 (3) |
| Albert Turner | crafter (planks) | 4.5 |  |  |  | 98% |  |  |  |  | 2% | 61% | 3% | 1.0 (1) | 2.0 (2) |
| Lionel Furness | crafter (ingots) | 6.4 |  |  |  |  | 98% |  |  |  | 2% | 43% | 2% | 1.0 (1) | 2.0 (2) |
| Edith Wright | crafter (tools) | 11.2 |  |  |  |  |  | 99% |  |  | 1% |  | 1% | 1.0 (1) | 2.0 (2) |
| Rose Thatcher | builder | 6.1 |  |  |  |  |  |  | 98% |  | 2% | 46% | 0% | 1.0 (1) | 2.0 (2) |
| Herbert Flaxman | gatherer (fiber) | 11.1 | 99% |  |  |  |  |  |  |  | 1% |  |  | 1.0 (1) | 2.0 (2) |
| Alice Hewitt | gatherer (wood) | 10.1 |  | 93% |  |  |  |  |  | 6% | 1% |  | 2% | 2.0 (2) | 3.0 (3) |
| Mabel Delve | gatherer (ore) | 11.2 |  |  | 93% |  |  |  |  | 4% | 2% |  | 1% | 2.0 (2) | 3.0 (3) |
| Edwin Smith | crafter (ingots) | 5.9 |  |  |  |  | 97% |  |  |  | 3% | 47% | 2% | 1.0 (1) | 2.0 (2) |
| Ruth Slater | builder | 5.2 |  |  |  |  |  |  | 98% |  | 2% | 55% | 0% | 1.0 (1) | 2.0 (2) |
| Hilda Hayward | gatherer (fiber) | 9.5 | 99% |  |  |  |  |  |  |  | 1% | 18% |  | 1.0 (1) | 2.0 (2) |

## How questions were settled

| Settled by | Questions | Share |
|---|---:|---:|
| rules | 51565 | 91.8% |
| rules, unsure | 4616 | 8.2% |

## Chain bottlenecks

- `gather fiber` took 26% of all bot time (46.1 bot-hours over 20236 steps, 1585 failed)
- `gather iron_ore` took 18% of all bot time (31.7 bot-hours over 5654 steps, 82 failed)
- `gather log` took 13% of all bot time (23.0 bot-hours over 8328 steps, 98 failed)
- clay came at 19 s a unit, 1.6× D5's 12 s
- iron_ore failed 46% of its pulls (2571 of 5572)
- stone failed 47% of its pulls (1077 of 2285)
- `stock log` failed 93% of the time
- `the road in: read someone's card` failed 42% of the time
- stuck 2269×: the kiln waits for a plot, and the plot for the road in's deed
- stuck 85×: the plot has no room for a sawbench

## Production

| Item | Gathered | Crafted | Per game hour | p10 | p50 | p90 | First at (h) | s a unit (D5) | Traded | Low | Mean price (D5) | High |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| anvil_kit | 0 | 5 | 0.4 | 48 | 49 | 53 | 5.25 |  |  |  |  |  |
| brick | 0 | 1104 | 94.5 | 50 | 60 | 68 | 4.63 |  |  |  |  |  |
| charcoal | 0 | 892 | 76.4 | 40 | 52 | 67 | 4.62 |  |  |  |  |  |
| chest_kit | 0 | 3 | 0.3 | 60 | 61 | 67 | 0.24 |  |  |  |  |  |
| clay | 1174 | 0 | 100.5 | 45 | 65 | 77 | 0.15 | 19 (12) |  |  |  |  |
| fiber | 38072 | 0 | 3260.0 | 42 | 53 | 69 | 0.01 | 4 (4) | 4 | 1 | 1.0 (1) | 1 |
| forge_kit | 0 | 10 | 0.9 | 52 | 55 | 58 | 4.81 |  |  |  |  |  |
| iron_axe | 0 | 1 | 0.1 | 55 | 55 | 55 | 5.85 |  |  |  |  |  |
| iron_axe_head | 0 | 1 | 0.1 | 51 | 51 | 51 | 5.84 |  |  |  |  |  |
| iron_hammer | 0 | 1 | 0.1 | 63 | 63 | 63 | 5.98 |  |  |  |  |  |
| iron_hammer_head | 0 | 1 | 0.1 | 60 | 60 | 60 | 5.97 |  |  |  |  |  |
| iron_ingot | 0 | 333 | 28.5 | 39 | 53 | 65 | 4.97 |  |  |  |  |  |
| iron_nails | 0 | 70 | 6.0 | 51 | 55 | 62 | 4.97 |  |  |  |  |  |
| iron_ore | 3003 | 0 | 257.1 | 24 | 56 | 78 | 0.14 | 38 (90) | 9 | 17 | 19.2 (20) | 20 |
| iron_pickaxe | 0 | 1 | 0.1 | 56 | 56 | 56 | 5.78 |  |  |  |  |  |
| iron_pickaxe_head | 0 | 1 | 0.1 | 52 | 52 | 52 | 5.78 |  |  |  |  |  |
| iron_sickle | 0 | 1 | 0.1 | 65 | 65 | 65 | 6.04 |  |  |  |  |  |
| iron_sickle_blade | 0 | 1 | 0.1 | 63 | 63 | 63 | 6.03 |  |  |  |  |  |
| kiln_kit | 0 | 10 | 0.9 | 51 | 55 | 58 | 0.17 |  |  |  |  |  |
| knapped_stone | 0 | 540 | 46.2 | 45 | 53 | 63 | 0.03 |  |  |  |  |  |
| log | 10423 | 0 | 892.5 | 32 | 48 | 71 | 0.07 | 8 (32) | 38 | 8 | 8.0 (8) | 8 |
| plank | 0 | 4311 | 369.1 | 39 | 52 | 67 | 5.24 |  |  |  |  |  |
| rope | 0 | 1547 | 132.5 | 44 | 57 | 65 | 0.06 |  |  |  |  |  |
| sawbench_kit | 0 | 7 | 0.6 | 41 | 53 | 60 | 5.07 |  |  |  |  |  |
| sticks | 2488 | 0 | 213.0 | 29 | 48 | 60 | 0.05 | 3 (12) |  |  |  |  |
| stone | 1262 | 0 | 108.1 | 34 | 52 | 71 | 0.02 | 24 (22) |  |  |  |  |
| stone_axe | 0 | 155 | 13.3 | 48 | 53 | 58 | 0.07 |  |  |  |  |  |
| stone_hammer | 0 | 10 | 0.9 | 50 | 53 | 58 | 0.06 |  |  |  |  |  |
| stone_pickaxe | 0 | 98 | 8.4 | 49 | 54 | 59 | 0.13 |  |  |  |  |  |
| stone_sickle | 0 | 11 | 0.9 | 50 | 56 | 59 | 0.28 |  |  |  |  |  |
| tool_handle | 0 | 6218 | 532.4 | 46 | 56 | 66 | 0.06 |  |  |  |  |  |
| workbench_kit | 0 | 27 | 2.3 | 52 | 55 | 67 | 0.02 |  |  |  |  |  |

## Pulls that failed

| Item | Pulls | Failed | Share |
|---|---:|---:|---:|
| clay | 794 | 0 | 0% |
| fiber | 18695 | 0 | 0% |
| iron_ore | 5572 | 2571 | 46% |
| log | 8244 | 1319 | 16% |
| sticks | 1099 | 0 | 0% |
| stone | 2285 | 1077 | 47% |

## Tools worn out

- stone_axe: 136
- stone_pickaxe: 87
- stone_sickle: 9

## Strands

- Fiber deposited: 34361
- Deeds bought: 20 for 0 Strands
- Spent on tax: 0 Strands
- Market fees: 280 Strands
- A deed (500, in any home zone) is 0.61 hours of grass

## Where the time went

| Step | Bot-hours | Steps | Failed |
|---|---:|---:|---:|
| gather fiber | 46.12 | 20236 | 1585 |
| gather iron_ore | 31.73 | 5654 | 82 |
| gather log | 23.03 | 8328 | 98 |
| craft tool_handle | 9.14 | 3708 | 0 |
| gather stone | 8.47 | 2238 | 18 |
| craft knapped_stone | 7.82 | 521 | 0 |
| craft charcoal | 6.95 | 446 | 0 |
| at the hall | 6.58 | 395 | 0 |
| gather clay | 6.11 | 796 | 2 |
| craft plank | 4.83 | 1437 | 0 |
| craft brick | 4.53 | 552 | 0 |
| craft iron_ingot | 4.18 | 333 | 0 |
| at the bazaar | 3.72 | 216 | 0 |
| craft rope | 2.32 | 1527 | 0 |
| stock log | 2.02 | 1077 | 1005 |
| gather sticks | 2.00 | 1085 | 0 |
| stock iron_ore | 1.32 | 31 | 1 |
| sell log | 1.09 | 33 | 0 |
| buy log | 0.87 | 29 | 0 |
| craft stone_axe | 0.77 | 145 | 0 |
| craft stone_pickaxe | 0.63 | 98 | 0 |
| the road in: make a stone axe | 0.55 | 113 | 3 |
| the road in: set a station up at home | 0.44 | 43 | 1 |
| the road in: look at a farm by the road | 0.39 | 26 | 0 |
| craft kiln_kit | 0.30 | 10 | 0 |
| the road in: change fiber for Strands at the Exchange | 0.29 | 24 | 2 |
| buy iron_ore | 0.27 | 9 | 0 |
| sell iron_ore | 0.21 | 8 | 0 |
| to the hall | 0.21 | 5 | 0 |
| the road in: see to the market box | 0.17 | 20 | 0 |
| craft forge_kit | 0.15 | 10 | 0 |
| place kiln | 0.15 | 10 | 0 |
| the road in: look at an urban yard by the road | 0.15 | 20 | 0 |
| bid for log | 0.14 | 5 | 0 |
| the road in: read someone's card | 0.11 | 35 | 15 |
| the road in: look at a suburban lot by the road | 0.10 | 20 | 0 |
| the road in: fell a tree | 0.10 | 27 | 2 |
| the road in: set the workbench up off any lot | 0.09 | 8 | 0 |
| to the bazaar | 0.07 | 2 | 0 |
| craft sawbench_kit | 0.07 | 7 | 0 |
| the road in: leave a sell order on the book | 0.07 | 20 | 0 |
| bid for iron_ore | 0.07 | 2 | 0 |
| place workbench | 0.07 | 18 | 0 |
| craft stone_hammer | 0.07 | 10 | 0 |
| craft anvil_kit | 0.06 | 5 | 0 |
| craft workbench_kit | 0.05 | 17 | 0 |
| place forge | 0.05 | 10 | 0 |
| craft stone_sickle | 0.04 | 11 | 0 |
| the road in: say a word to whoever's near | 0.04 | 20 | 0 |
| craft iron_nails | 0.03 | 7 | 0 |
| the road in: gather stone by hand | 0.03 | 6 | 0 |
| the road in: knap stone at the bench | 0.03 | 9 | 0 |
| place anvil | 0.02 | 4 | 0 |
| place sawbench | 0.02 | 6 | 0 |
| the road in: claim a home with its deed | 0.02 | 20 | 0 |
| the road in: look in at a bazaar stall | 0.01 | 20 | 0 |
| deposit fiber | 0.01 | 185 | 0 |
| the road in: make a handle at the bench | 0.01 | 6 | 0 |
| the road in: make a workbench kit | 0.01 | 4 | 0 |
| craft iron_sickle | 0.01 | 1 | 0 |
| craft iron_hammer | 0.01 | 1 | 0 |
| craft iron_axe | 0.01 | 1 | 0 |
| craft iron_pickaxe | 0.01 | 1 | 0 |
| the road in: pick up fallen branches | 0.01 | 4 | 0 |
| craft chest_kit | 0.01 | 3 | 0 |
| the road in: twist a rope | 0.01 | 4 | 0 |
| craft iron_axe_head | 0.01 | 1 | 0 |
| craft iron_hammer_head | 0.01 | 1 | 0 |
| craft iron_pickaxe_head | 0.01 | 1 | 0 |
| craft iron_sickle_blade | 0.00 | 1 | 0 |
| the road in: look in at the Deeds Office | 0.00 | 20 | 0 |
| place storage_chest | 0.00 | 2 | 0 |
| the road in: read the Registrar's letter | 0.00 | 20 | 0 |
| the road in: give a station a job | 0.00 | 20 | 0 |
| the road in: leave a buy order on the book | 0.00 | 20 | 0 |
| the road in: read the Gazette | 0.00 | 20 | 0 |
| the road in: read the zone card | 0.00 | 20 | 0 |
| the road in: look in its pack | 0.00 | 20 | 0 |
| the road in: read what tax asks | 0.00 | 20 | 0 |

## Why steps failed

| Step: reason | Times |
|---|---:|
| gather fiber: the server didn't start extracting node #: Someone else is already gathering this Wild Grass | 1156 |
| stock log: A shop holds # listings at most | 1005 |
| gather fiber: the server didn't start extracting node #: This Wild Grass is spent; it will grow back | 429 |
| gather log: the server didn't start extracting node #: Someone else is already gathering this Tree | 58 |
| gather iron_ore: the server didn't start extracting node #: This Iron Vein is spent; it will grow back | 51 |
| gather log: the server didn't start extracting node #: This Tree is spent; it will grow back | 32 |
| gather iron_ore: the server didn't start extracting node #: Someone else is already gathering this Iron Vein | 31 |
| the road in: read someone's card: nobody about, nor in the Gazette, whose card to read | 15 |
| gather stone: the server didn't start extracting node #: Someone else is already gathering this Rock Outcrop | 11 |
| gather stone: the server didn't start extracting node #: This Rock Outcrop is spent; it will grow back | 7 |
| gather log: the server didn't start extracting node #: Someone else is already gathering this Old Growth | 6 |
| gather clay: the server didn't start extracting node #: This Clay Bank is spent; it will grow back | 2 |
