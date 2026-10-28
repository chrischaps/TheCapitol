# The town

20 citizens (2 all-rounder, 3 builder, 2 crafter-ingots, 2 crafter-planks, 2 crafter-tools, 3 gatherer-fiber, 3 gatherer-ore, 3 gatherer-wood) for 14.0 hours of the day's clock, 11.7 hours of ticks at 10× (70.2 wall minutes).

## Conservation

**Holds.**

### Strands

Every account's Strands, with escrow: 2100 at the start, 17168 at the end. That's the start, plus 0 new accounts at 100, plus 18632 in from the faucets, less 3564 gone to the sinks, plus 0 the transfers net to: 17168 expected, 0 off.

| Flow | Kind | Strands |
|---|---|---:|
| bazaar_purchase | Transfer | -462 |
| bazaar_rent | Sink | -800 |
| bazaar_sale | Transfer | +462 |
| court_escrow | Escrow | -2677 |
| court_fee | Sink | -153 |
| court_payout | Escrow | +2545 |
| court_refund | Escrow | +96 |
| exchange_deposit | Faucet | +18632 |
| market_escrow | Escrow | -56362 |
| market_fee | Sink | -2611 |
| market_refund | Escrow | +30393 |
| market_sale | Escrow | +24531 |
| trade_received | Transfer | +891 |
| trade_sent | Transfer | -891 |

Escrow: 0 at the start, 1474 at the end; 0 unexplained.

### Units

| Item | Change | Explained | Drift |
|---|---:|---:|---:|
| brick | +1028 | +1028 | 0 |
| charcoal | +146 | +146 | 0 |
| chest_kit | +1 | +1 | 0 |
| clay | +12 | +12 | 0 |
| fiber | +395 | +395 | 0 |
| iron_axe | +3 | +3 | 0 |
| iron_hammer | +3 | +3 | 0 |
| iron_ingot | +302 | +302 | 0 |
| iron_nails | +10 | +10 | 0 |
| iron_ore | +1000 | +1000 | 0 |
| iron_pickaxe | +3 | +3 | 0 |
| iron_sickle | +3 | +3 | 0 |
| knapped_stone | +7 | +7 | 0 |
| log | +5708 | +5708 | 0 |
| plank | +4055 | +4055 | 0 |
| rope | +302 | +302 | 0 |
| sticks | +17 | +17 | 0 |
| stone | +7 | +7 | 0 |
| stone_axe | +24 | +24 | 0 |
| stone_hammer | +17 | +17 | 0 |
| stone_pickaxe | +21 | +21 | 0 |
| stone_sickle | +3 | +3 | 0 |
| tool_handle | +3639 | +3639 | 0 |

| Flow | Units |
|---|---:|
| consumed | -17913 |
| crafted | +14907 |
| deposited | -18670 |
| gathered | +38625 |
| kit_placed | -95 |
| kit_returned | +3 |
| worn_out | -151 |

## Load

| | Whole soak |
|---|---|
| Tick budget | 10.0 ms |
| Ticks | 421428 |
| Tick time, mean | 0.35 ms |
| Tick time, p50 / p95 / p99 | ≤ 0.5 ms / ≤ 0.5 ms / ≤ 1 ms |
| Slowest tick | 33.6 ms |
| Ticks over budget | 10 (0.00%) |
| Worst minute, mean tick | 0.43 ms (p99 ≤ 1 ms) |
| Most in the world | 20 |
| Sockets opened / most open | 20 / 20 |
| Sockets dropped | 0 |
| Sockets resynced | 0 |
| DB transactions a second, mean / busiest minute | 244 / 369 |
| DB pool at the end, size / idle | 16 / 15 |

Sockets that ended: 20 closed.

## Trades

| Trade | Citizens | Strands at the start | At the end | Change each | Made | Sold (Strands) | Bought (Strands) | Deeds | Stuck |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 2 | 200 | 966 | +383 | 5290 | 320 (2509) | 556 (3080) | 2 | 0 |
| builder | 3 | 300 | 185 | -38 | 5433 | 463 (687) | 143 (1307) | 3 | 212 |
| crafter-ingots | 2 | 200 | 3161 | +1480 | 3736 | 221 (4417) | 393 (2419) | 2 | 2 |
| crafter-planks | 2 | 200 | 136 | -32 | 5801 | 162 (482) | 325 (1082) | 2 | 0 |
| crafter-tools | 2 | 200 | 102 | -49 | 7724 | 420 (2263) | 353 (3584) | 2 | 0 |
| gatherer-fiber | 3 | 300 | 5174 | +1625 | 10797 | 1111 (5783) | 1701 (7570) | 3 | 0 |
| gatherer-ore | 3 | 300 | 3677 | +1126 | 6264 | 440 (3673) | 133 (1474) | 3 | 0 |
| gatherer-wood | 3 | 300 | 2193 | +631 | 8487 | 1013 (4717) | 545 (4469) | 3 | 0 |

In trade windows: 0 trades with people, 17 between citizens (each asked by one), with 891 Strands changing hands between citizens.

## Citizens

| Citizen | Trade | Strands | Made (most) | Sold (Strands) | Bought (Strands) | Deeds | Failed steps | Stuck (why, most often) |
|---|---|---:|---|---:|---:|---:|---:|---|
| Ada Rushton | gatherer-fiber | 100 → 1229 | 3361 (3133 fiber) | 359 (2200) | 767 (3524) | 1 | 102 | 0 |
| Leonard Forester | gatherer-wood | 100 → 1175 | 2365 (1063 fiber) | 385 (2622) | 285 (1795) | 1 | 52 | 0 |
| Doris Miner | gatherer-ore | 100 → 1489 | 1281 (526 log) | 173 (1723) | 45 (322) | 1 | 29 | 0 |
| Phyllis Joiner | crafter-planks | 100 → 100 | 2117 (511 log) | 37 (94) | 21 (82) | 1 | 33 | 0 |
| Harold Ironside | crafter-ingots | 100 → 1003 | 2087 (673 fiber) | 32 (1767) | 177 (1268) | 1 | 15 | 1 (the kiln waits for a plot, and the plot for the road in's deed) |
| Agnes Toller | crafter-tools | 100 → 11 | 4175 (1442 tool_handle) | 137 (633) | 165 (1070) | 1 | 21 | 0 |
| Lillian Mason | builder | 100 → 12 | 2615 (727 fiber) | 14 (22) | 60 (570) | 1 | 24 | 52 (the kiln waits for a plot, and the plot for the road in's deed) |
| Dorothy Reeder | all-rounder | 100 → 31 | 2357 (773 fiber) | 165 (1659) | 406 (2011) | 1 | 54 | 0 |
| Reginald Ashby | gatherer-wood | 100 → 596 | 2821 (1052 log) | 403 (1469) | 217 (1526) | 1 | 81 | 0 |
| Hazel Leadbetter | gatherer-ore | 100 → 1895 | 3286 (2181 fiber) | 180 (1341) | 47 (823) | 1 | 65 | 0 |
| Albert Turner | crafter-planks | 100 → 36 | 3684 (1518 fiber) | 125 (388) | 304 (1000) | 1 | 26 | 0 |
| Lionel Furness | all-rounder | 100 → 935 | 2933 (1548 fiber) | 155 (850) | 150 (1069) | 1 | 87 | 0 |
| Edith Wright | crafter-tools | 100 → 91 | 3549 (1346 fiber) | 283 (1630) | 188 (2514) | 1 | 71 | 0 |
| Rose Thatcher | builder | 100 → 17 | 1366 (399 log) | 119 (152) | 26 (280) | 1 | 21 | 104 (the kiln waits for a plot, and the plot for the road in's deed) |
| Herbert Flaxman | gatherer-fiber | 100 → 3537 | 3894 (3213 fiber) | 352 (2624) | 292 (2035) | 1 | 74 | 0 |
| Alice Hewitt | gatherer-wood | 100 → 422 | 3301 (1342 fiber) | 225 (626) | 43 (1148) | 1 | 75 | 0 |
| Mabel Delve | gatherer-ore | 100 → 293 | 1697 (456 log) | 87 (609) | 41 (329) | 1 | 21 | 0 |
| Edwin Smith | crafter-ingots | 100 → 2158 | 1649 (316 fiber) | 189 (2650) | 216 (1151) | 1 | 14 | 1 (the kiln waits for a plot, and the plot for the road in's deed) |
| Ruth Slater | builder | 100 → 156 | 1452 (258 plank) | 330 (513) | 57 (457) | 1 | 9 | 56 (the kiln waits for a plot, and the plot for the road in's deed) |
| Hilda Hayward | gatherer-fiber | 100 → 408 | 3542 (3167 fiber) | 400 (959) | 642 (2011) | 1 | 104 | 0 |

## Time by activity

What each trade's and each citizen's work went on, as a share of its work time. Free is the share of all its time that had nothing to do in it, market hours with nothing to trade among it. Setup is the share of the work on tools, kits, stations and deeds on the way to something else. A day is the day's clock's: jobs are the eight a citizen can lean over, and activities those and the Commons, detours and the road in, for each day it worked (the mean, and the fewest).

| Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 19.3 | 12% | 12% | 8% | 7% | 10% | 8% | 14% | 22% | 2% | 4% | 1% | 13% | 1% | 7.5 (7) | 10.0 (10) |
| builder | 24.0 | 5% | 14% | 5% | 14% | 12% | 4% | 39% | 3% | 1% | 3% | 1% | 29% | 1% | 6.0 (5) | 8.3 (8) |
| crafter-ingots | 20.5 | 4% |  | 9% | 6% | 28% | 15% | 18% | 11% | 1% | 6% | 1% | 10% | 1% | 6.0 (5) | 9.0 (8) |
| crafter-planks | 18.4 | 7% | 17% |  | 36% |  | 21% | 0% | 14% | 0% | 3% | 2% | 18% | 1% | 6.0 (6) | 8.0 (8) |
| crafter-tools | 20.1 | 6% | 10% | 9% | 15% |  | 46% | 6% | 1% | 1% | 5% | 1% | 12% | 1% | 6.0 (6) | 9.0 (9) |
| gatherer-fiber | 29.8 | 43% | 10% | 9% |  | 4% |  | 0% | 26% | 2% | 5% | 1% | 12% | 1% | 5.3 (5) | 8.3 (8) |
| gatherer-ore | 27.9 | 5% | 15% | 49% |  | 12% | 11% | 1% | 2% | 0% | 4% | 1% | 16% | 3% | 6.0 (5) | 8.7 (8) |
| gatherer-wood | 28.2 | 15% | 44% | 5% | 7% |  | 5% | 10% | 5% | 2% | 6% | 1% | 16% | 3% | 5.3 (5) | 8.3 (8) |

| Citizen | Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Ada Rushton | gatherer-fiber | 10.3 | 49% | 8% | 0% |  |  |  | 0% | 32% | 2% | 7% | 1% | 11% | 0% | 5.0 (5) | 8.0 (8) |
| Leonard Forester | gatherer-wood | 9.9 | 13% | 34% | 13% | 3% |  |  | 13% | 9% | 3% | 10% | 0% | 13% | 3% | 6.0 (6) | 9.0 (9) |
| Doris Miner | gatherer-ore | 9.3 |  | 15% | 62% |  | 13% |  | 0% | 3% | 0% | 6% | 1% | 16% | 3% | 5.0 (5) | 8.0 (8) |
| Phyllis Joiner | crafter-planks | 9.0 | 3% | 20% |  | 43% |  | 23% | 0% | 3% |  | 6% | 2% | 18% | 1% | 6.0 (6) | 8.0 (8) |
| Harold Ironside | crafter-ingots | 9.9 | 8% |  | 12% | 13% | 25% | 14% | 18% | 1% | 0% | 8% | 1% | 14% | 1% | 7.0 (7) | 10.0 (10) |
| Agnes Toller | crafter-tools | 10.4 | 1% |  | 17% | 24% |  | 47% | 0% | 1% | 1% | 7% | 1% | 11% | 1% | 6.0 (6) | 9.0 (9) |
| Lillian Mason | builder | 8.1 | 12% | 13% |  | 17% | 14% | 11% | 23% | 5% |  | 5% | 1% | 28% | 1% | 7.0 (7) | 9.0 (9) |
| Dorothy Reeder | all-rounder | 9.7 | 9% | 14% | 16% | 7% | 11% | 3% | 16% | 20% | 2% |  | 1% | 12% | 2% | 8.0 (8) | 10.0 (10) |
| Reginald Ashby | gatherer-wood | 9.2 | 13% | 59% |  | 18% |  |  | 0% | 3% | 1% | 5% | 0% | 17% | 4% | 5.0 (5) | 8.0 (8) |
| Hazel Leadbetter | gatherer-ore | 9.6 | 15% | 16% | 32% |  | 10% | 16% | 2% | 1% | 1% | 5% | 1% | 13% | 2% | 7.0 (7) | 10.0 (10) |
| Albert Turner | crafter-planks | 9.4 | 11% | 15% |  | 30% |  | 18% | 0% | 24% | 0% |  | 1% | 17% | 1% | 6.0 (6) | 8.0 (8) |
| Lionel Furness | all-rounder | 9.6 | 16% | 10% |  | 7% | 8% | 12% | 12% | 23% | 2% | 8% | 0% | 13% | 1% | 7.0 (7) | 10.0 (10) |
| Edith Wright | crafter-tools | 9.7 | 11% | 21% |  | 5% |  | 45% | 11% | 1% | 1% | 3% | 1% | 14% | 1% | 6.0 (6) | 9.0 (9) |
| Rose Thatcher | builder | 7.5 | 2% | 22% |  | 6% |  |  | 64% | 1% | 1% | 3% | 1% | 32% | 0% | 5.0 (5) | 8.0 (8) |
| Herbert Flaxman | gatherer-fiber | 10.2 | 42% | 14% | 16% |  |  |  | 0% | 22% | 2% | 2% | 1% | 7% | 1% | 5.0 (5) | 8.0 (8) |
| Alice Hewitt | gatherer-wood | 9.1 | 19% | 40% |  |  |  | 14% | 18% | 3% | 2% | 3% | 1% | 18% | 1% | 5.0 (5) | 8.0 (8) |
| Mabel Delve | gatherer-ore | 9.0 |  | 13% | 52% |  | 13% | 16% | 0% | 3% | 0% |  | 2% | 19% | 3% | 6.0 (6) | 8.0 (8) |
| Edwin Smith | crafter-ingots | 10.6 |  |  | 7% |  | 32% | 17% | 18% | 20% | 1% | 5% | 1% | 5% | 1% | 5.0 (5) | 8.0 (8) |
| Ruth Slater | builder | 8.4 |  | 9% | 15% | 19% | 21% |  | 33% | 1% | 1% |  | 1% | 27% | 1% | 6.0 (6) | 8.0 (8) |
| Hilda Hayward | gatherer-fiber | 9.4 | 36% | 9% | 11% |  | 11% |  | 1% | 24% | 1% | 5% | 1% | 17% | 1% | 6.0 (6) | 9.0 (9) |

Detours: 16 of 20 citizens took one, 4.5% of the work.

## How questions were settled

| Settled by | Questions | Share |
|---|---:|---:|
| rules | 41002 | 92.8% |
| rules, unsure | 3183 | 7.2% |

## Chain bottlenecks

- `gather fiber` took 14% of all bot time (29.8 bot-hours over 12040 steps, 622 failed)
- `gather log` took 11% of all bot time (24.3 bot-hours over 6402 steps, 202 failed)
- `gather iron_ore` took 9% of all bot time (20.9 bot-hours over 3455 steps, 101 failed)
- clay came at 19 s a unit, 1.5× D5's 12 s
- iron_ore failed 46% of its pulls (1553 of 3354)
- stone failed 43% of its pulls (782 of 1811)
- stuck 214×: the kiln waits for a plot, and the plot for the road in's deed

## Production

| Item | Gathered | Crafted | Per game hour | p10 | p50 | p90 | First at (h) | s a unit (D5) | Traded | Low | Mean price (D5) | High |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| anvil_kit | 0 | 8 | 0.7 | 48 | 52 | 62 | 4.15 |  |  |  |  |  |
| brick | 0 | 1232 | 105.4 | 49 | 59 | 68 | 1.39 |  | 432 | 1 | 2.7 (8) | 8 |
| charcoal | 0 | 1164 | 99.6 | 37 | 58 | 72 | 1.38 |  |  |  |  |  |
| chest_kit | 0 | 14 | 1.2 | 46 | 58 | 74 | 0.28 |  |  |  |  |  |
| clay | 1352 | 0 | 115.6 | 46 | 63 | 76 | 0.13 | 19 (12) |  |  |  |  |
| fiber | 23341 | 0 | 1996.3 | 42 | 54 | 69 | 0.01 | 5 (4) | 2 | 1 | 1.0 (1) | 1 |
| forge_kit | 0 | 17 | 1.5 | 52 | 57 | 60 | 1.50 |  |  |  |  |  |
| iron_axe | 0 | 3 | 0.3 | 51 | 54 | 55 | 6.39 |  |  |  |  |  |
| iron_axe_head | 0 | 3 | 0.3 | 46 | 46 | 49 | 6.38 |  |  |  |  |  |
| iron_hammer | 0 | 3 | 0.3 | 49 | 55 | 57 | 6.51 |  |  |  |  |  |
| iron_hammer_head | 0 | 3 | 0.3 | 44 | 48 | 51 | 6.50 |  |  |  |  |  |
| iron_ingot | 0 | 402 | 34.4 | 36 | 53 | 70 | 1.52 |  | 139 | 42 | 51.4 (50) | 60 |
| iron_nails | 0 | 160 | 13.7 | 46 | 63 | 69 | 1.97 |  |  |  |  |  |
| iron_ore | 1804 | 0 | 154.3 | 23 | 50 | 76 | 0.11 | 42 (90) | 705 | 1 | 5.8 (20) | 20 |
| iron_pickaxe | 0 | 3 | 0.3 | 55 | 57 | 58 | 6.33 |  |  |  |  |  |
| iron_pickaxe_head | 0 | 3 | 0.3 | 52 | 52 | 53 | 6.32 |  |  |  |  |  |
| iron_sickle | 0 | 3 | 0.3 | 52 | 56 | 59 | 6.55 |  |  |  |  |  |
| iron_sickle_blade | 0 | 3 | 0.3 | 45 | 51 | 54 | 6.54 |  |  |  |  |  |
| kiln_kit | 0 | 18 | 1.5 | 50 | 54 | 59 | 0.18 |  |  |  |  |  |
| knapped_stone | 0 | 432 | 36.9 | 46 | 54 | 66 | 0.03 |  |  |  |  |  |
| log | 8130 | 0 | 695.3 | 32 | 49 | 72 | 0.06 | 11 (32) | 1785 | 1 | 5.3 (8) | 23 |
| plank | 0 | 5340 | 456.7 | 38 | 51 | 68 | 2.04 |  | 1067 | 1 | 1.3 (5) | 5 |
| rope | 0 | 1977 | 169.1 | 43 | 55 | 68 | 0.05 |  |  |  |  |  |
| sawbench_kit | 0 | 15 | 1.3 | 50 | 60 | 66 | 2.02 |  |  |  |  |  |
| sticks | 2811 | 0 | 240.4 | 32 | 46 | 61 | 0.04 | 5 (12) |  |  |  |  |
| stone | 1187 | 0 | 101.5 | 33 | 52 | 71 | 0.02 | 26 (22) |  |  |  |  |
| stone_axe | 0 | 122 | 10.4 | 51 | 57 | 63 | 0.06 |  | 11 | 71 | 81.6 (50) | 90 |
| stone_hammer | 0 | 17 | 1.5 | 49 | 54 | 64 | 0.07 |  |  |  |  |  |
| stone_pickaxe | 0 | 70 | 6.0 | 53 | 57 | 64 | 0.09 |  | 9 | 45 | 47.8 (50) | 50 |
| stone_sickle | 0 | 7 | 0.6 | 50 | 57 | 72 | 0.26 |  |  |  |  |  |
| tool_handle | 0 | 3867 | 330.7 | 43 | 53 | 63 | 0.05 |  |  |  |  |  |
| workbench_kit | 0 | 21 | 1.8 | 50 | 58 | 70 | 0.01 |  |  |  |  |  |

## Pulls that failed

| Item | Pulls | Failed | Share |
|---|---:|---:|---:|
| clay | 917 | 0 | 0% |
| fiber | 11433 | 0 | 0% |
| iron_ore | 3354 | 1553 | 46% |
| log | 6209 | 789 | 13% |
| sticks | 1555 | 0 | 0% |
| stone | 1811 | 782 | 43% |

## Tools worn out

- stone_axe: 98
- stone_pickaxe: 49
- stone_sickle: 4

## Strands

- Fiber deposited: 18670
- Deeds bought: 20 for 0 Strands
- Spent on tax: 0 Strands
- Market fees: 2611 Strands
- A deed (500, in any home zone) is 0.64 hours of grass

## The hall's board

- Commissions posted: 11, 1236 Strands put down, 68 in the court's fees
- Claims: 9, promising 14 units; 0 lapsed
- Delivered: 20 units, paid 1200 Strands from escrow
- Filled: 10; ran out: 0, 0 Strands refunded
- Strands through the court: 1304 put in by these posters, 1200 paid out to these deliverers

## Wages

- Shifts taken: 14; ended: 12 at the cap, 1 by the employer, 1 gone
- Units made for employers: 706, paid 1345 Strands from their escrow
- Offers of work made: 14, 13 of them worked; taken down: 12 at the cap, 1 with the place gone, 1 as the money ran low
- Units bought on them: 706, paid 1345 Strands, and 85 in the court's fees

## Where the time went

| Step | Bot-hours | Steps | Failed |
|---|---:|---:|---:|
| gather fiber | 29.83 | 12040 | 622 |
| gather log | 24.34 | 6402 | 202 |
| gather iron_ore | 20.85 | 3455 | 101 |
| at the hall | 19.57 | 1144 | 0 |
| make a market | 15.47 | 20 | 0 |
| at the bazaar | 12.36 | 717 | 0 |
| gather stone | 8.72 | 1784 | 13 |
| craft tool_handle | 8.44 | 2611 | 0 |
| craft plank | 8.42 | 1727 | 0 |
| gather clay | 6.95 | 928 | 11 |
| craft charcoal | 6.85 | 478 | 1 |
| craft knapped_stone | 6.10 | 422 | 0 |
| craft brick | 5.05 | 616 | 0 |
| sell log | 5.04 | 190 | 0 |
| gather sticks | 4.15 | 1553 | 1 |
| craft iron_ingot | 3.96 | 391 | 0 |
| sell iron_ore | 2.86 | 83 | 0 |
| craft rope | 2.72 | 1641 | 0 |
| buy log | 2.66 | 109 | 0 |
| stock log | 2.61 | 100 | 1 |
| a detour: sit a while in a park nearby | 2.00 | 8 | 0 |
| a detour: browse the bazaar lane | 1.75 | 8 | 0 |
| a detour: read the Wanted page on the hall's board | 1.25 | 5 | 0 |
| buy iron_ore | 1.07 | 38 | 0 |
| sell plank | 1.05 | 57 | 0 |
| a detour: read the Gazette at the hall | 1.00 | 4 | 0 |
| stock iron_ore | 0.88 | 27 | 2 |
| a detour: call on a neighbour | 0.75 | 3 | 0 |
| craft stone_axe | 0.61 | 119 | 7 |
| craft kiln_kit | 0.56 | 18 | 0 |
| work: craft rope by hand | 0.51 | 328 | 1 |
| a detour: try making storage chest kit | 0.50 | 2 | 0 |
| sell iron_ingot | 0.48 | 15 | 0 |
| to the hall | 0.47 | 13 | 0 |
| the road in: look at a farm by the road | 0.46 | 23 | 0 |
| work: craft charcoal at a kiln | 0.45 | 106 | 1 |
| sell brick | 0.43 | 22 | 0 |
| craft stone_pickaxe | 0.34 | 72 | 5 |
| to the bazaar | 0.32 | 7 | 0 |
| stock brick | 0.29 | 14 | 0 |
| bid for stone_axe | 0.29 | 21 | 0 |
| craft forge_kit | 0.28 | 17 | 0 |
| place kiln | 0.27 | 18 | 0 |
| buy stone_pickaxe at a stall | 0.27 | 10 | 0 |
| the road in: change fiber for Strands at the Exchange | 0.26 | 21 | 0 |
| bid for stone_pickaxe | 0.25 | 14 | 0 |
| a detour: see a ford in the hills to the south | 0.25 | 1 | 0 |
| a detour: try making iron nails | 0.25 | 1 | 0 |
| a detour: see a ford in the hills to the west | 0.25 | 1 | 0 |
| a detour: see a lake in the hills to the south-east | 0.25 | 1 | 0 |
| a detour: try making iron ingot | 0.25 | 1 | 0 |
| craft sawbench_kit | 0.25 | 15 | 0 |
| hire: bring home the box | 0.22 | 11 | 0 |
| bid for log | 0.20 | 7 | 0 |
| take work: craft rope by hand | 0.20 | 7 | 1 |
| place forge | 0.19 | 17 | 0 |
| buy stone_axe at a stall | 0.19 | 11 | 0 |
| place sawbench | 0.19 | 15 | 0 |
| stock plank | 0.19 | 19 | 0 |
| craft iron_nails | 0.19 | 15 | 0 |
| sell stone_axe | 0.18 | 11 | 0 |
| sell stone_pickaxe | 0.18 | 9 | 0 |
| the road in: make a stone axe | 0.18 | 49 | 0 |
| bid for iron_ore | 0.17 | 7 | 0 |
| the road in: see to the market box | 0.17 | 20 | 0 |
| the road in: look at an urban yard by the road | 0.15 | 20 | 0 |
| craft anvil_kit | 0.14 | 8 | 0 |
| place storage_chest | 0.14 | 13 | 0 |
| bid for stone | 0.14 | 5 | 0 |
| stock stone_axe | 0.13 | 9 | 0 |
| bid for stone_hammer | 0.13 | 6 | 0 |
| collect from the market box | 0.12 | 7 | 0 |
| deliver log | 0.11 | 4 | 2 |
| take work: craft charcoal at a kiln | 0.11 | 4 | 0 |
| the road in: look at a suburban lot by the road | 0.10 | 20 | 0 |
| place anvil | 0.10 | 8 | 0 |
| hire: craft rope by hand | 0.10 | 6 | 0 |
| work: craft plank at a sawbench | 0.09 | 53 | 0 |
| the road in: set a station up at home | 0.09 | 9 | 0 |
| the road in: claim a home with its deed | 0.09 | 20 | 0 |
| craft stone_hammer | 0.08 | 17 | 0 |
| take work: craft plank at a sawbench | 0.08 | 3 | 0 |
| hire: craft charcoal at a kiln | 0.07 | 4 | 0 |
| the road in: gather stone by hand | 0.07 | 9 | 0 |
| the road in: fell a tree | 0.07 | 12 | 1 |
| work: craft iron_ingot at a forge | 0.06 | 11 | 1 |
| stock iron_ingot | 0.06 | 2 | 0 |
| the road in: leave a sell order on the book | 0.06 | 20 | 0 |
| craft iron_axe_head | 0.05 | 3 | 0 |
| craft iron_hammer_head | 0.05 | 3 | 0 |
| craft workbench_kit | 0.05 | 18 | 0 |
| deliver sawbench_kit | 0.05 | 2 | 0 |
| place workbench | 0.05 | 20 | 0 |
| deliver chest_kit | 0.05 | 3 | 0 |
| hire: craft plank at a sawbench | 0.04 | 3 | 0 |
| the road in: look in at the Deeds Office | 0.04 | 20 | 0 |
| the road in: say a word to whoever's near | 0.04 | 20 | 0 |
| hire: craft iron_ingot at a forge | 0.04 | 1 | 0 |
| craft iron_hammer | 0.03 | 3 | 0 |
| craft iron_axe | 0.03 | 3 | 0 |
| craft iron_sickle | 0.03 | 3 | 0 |
| craft iron_pickaxe | 0.03 | 3 | 0 |
| buy stone_pickaxe | 0.03 | 1 | 0 |
| craft iron_sickle_blade | 0.03 | 3 | 0 |
| craft chest_kit | 0.03 | 12 | 0 |
| deliver kiln_kit | 0.03 | 1 | 0 |
| deliver forge_kit | 0.03 | 1 | 0 |
| craft stone_sickle | 0.03 | 7 | 0 |
| deliver anvil_kit | 0.03 | 1 | 0 |
| the road in: set the workbench up off any lot | 0.03 | 3 | 0 |
| the road in: knap stone at the bench | 0.03 | 6 | 0 |
| craft iron_pickaxe_head | 0.02 | 3 | 0 |
| the road in: look in at a bazaar stall | 0.02 | 20 | 0 |
| deposit fiber | 0.01 | 101 | 0 |
| the road in: read someone's card | 0.01 | 22 | 3 |
| the road in: make a workbench kit | 0.01 | 2 | 0 |
| the road in: make a handle at the bench | 0.01 | 2 | 0 |
| stock stone_pickaxe | 0.01 | 6 | 1 |
| the road in: read the Gazette | 0.01 | 20 | 0 |
| the road in: leave a buy order on the book | 0.00 | 20 | 0 |
| the road in: read the Registrar's letter | 0.00 | 20 | 0 |
| the road in: give a station a job | 0.00 | 20 | 0 |
| the road in: pick up fallen branches | 0.00 | 2 | 0 |
| the road in: twist a rope | 0.00 | 2 | 0 |
| take work: craft iron_ingot at a forge | 0.00 | 2 | 1 |
| the road in: read what tax asks | 0.00 | 20 | 0 |
| the road in: read the zone card | 0.00 | 20 | 0 |
| claim chest_kit | 0.00 | 3 | 0 |
| claim forge_kit | 0.00 | 1 | 0 |
| claim kiln_kit | 0.00 | 1 | 0 |
| the road in: look in its pack | 0.00 | 20 | 0 |
| claim sawbench_kit | 0.00 | 2 | 0 |
| claim anvil_kit | 0.00 | 1 | 0 |
| claim log | 0.00 | 1 | 0 |

## Why steps failed

| Step: reason | Times |
|---|---:|
| gather fiber: the server didn't start extracting node #: Someone else is already gathering this Wild Grass | 402 |
| gather fiber: the server didn't start extracting node #: This Wild Grass is spent; it will grow back | 220 |
| gather log: the server didn't start extracting node #: Someone else is already gathering this Tree | 104 |
| gather log: the server didn't start extracting node #: This Tree is spent; it will grow back | 98 |
| gather iron_ore: the server didn't start extracting node #: This Iron Vein is spent; it will grow back | 67 |
| gather iron_ore: the server didn't start extracting node #: Someone else is already gathering this Iron Vein | 34 |
| gather clay: the server didn't start extracting node #: This Clay Bank is spent; it will grow back | 10 |
| gather stone: the server didn't start extracting node #: Someone else is already gathering this Rock Outcrop | 6 |
| gather stone: the server didn't start extracting node #: This Rock Outcrop is spent; it will grow back | 6 |
| craft stone_axe: the inventory is full | 4 |
| craft stone_axe: Stone Axe needs # Knapped Stone, and we're # short | 3 |
| craft stone_pickaxe: the inventory is full | 3 |
