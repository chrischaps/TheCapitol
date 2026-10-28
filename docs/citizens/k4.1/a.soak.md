# The town

20 citizens (2 all-rounder, 3 builder, 2 crafter-ingots, 2 crafter-planks, 2 crafter-tools, 3 gatherer-fiber, 3 gatherer-ore, 3 gatherer-wood) for 14.0 hours of the day's clock, 11.7 hours of ticks at 10× (70.1 wall minutes).

## Conservation

**Holds.**

### Strands

Every account's Strands, with escrow: 2100 at the start, 15165 at the end. That's the start, plus 0 new accounts at 100, plus 16733 in from the faucets, less 3668 gone to the sinks, plus 0 the transfers net to: 15165 expected, 0 off.

| Flow | Kind | Strands |
|---|---|---:|
| bazaar_purchase | Transfer | -319 |
| bazaar_rent | Sink | -800 |
| bazaar_sale | Transfer | +319 |
| court_escrow | Escrow | -1609 |
| court_fee | Sink | -126 |
| court_payout | Escrow | +1312 |
| court_refund | Escrow | +108 |
| exchange_deposit | Faucet | +16733 |
| market_escrow | Escrow | -54838 |
| market_fee | Sink | -2742 |
| market_refund | Escrow | +26938 |
| market_sale | Escrow | +26139 |
| trade_received | Transfer | +1351 |
| trade_sent | Transfer | -1351 |

Escrow: 0 at the start, 1950 at the end; 0 unexplained.

### Units

| Item | Change | Explained | Drift |
|---|---:|---:|---:|
| brick | +1096 | +1096 | 0 |
| charcoal | +165 | +165 | 0 |
| chest_kit | +2 | +2 | 0 |
| clay | +17 | +17 | 0 |
| fiber | +408 | +408 | 0 |
| iron_axe | +3 | +3 | 0 |
| iron_hammer | +3 | +3 | 0 |
| iron_ingot | +333 | +333 | 0 |
| iron_nails | +10 | +10 | 0 |
| iron_ore | +840 | +840 | 0 |
| iron_pickaxe | +3 | +3 | 0 |
| iron_sickle | +3 | +3 | 0 |
| log | +4913 | +4913 | 0 |
| plank | +4312 | +4312 | 0 |
| rope | +421 | +421 | 0 |
| sawbench_kit | +1 | +1 | 0 |
| sticks | +17 | +17 | 0 |
| stone | +4 | +4 | 0 |
| stone_axe | +24 | +24 | 0 |
| stone_hammer | +18 | +18 | 0 |
| stone_pickaxe | +16 | +16 | 0 |
| stone_sickle | +2 | +2 | 0 |
| tool_handle | +5686 | +5686 | 0 |

| Flow | Units |
|---|---:|
| consumed | -19264 |
| crafted | +18361 |
| deposited | -16763 |
| gathered | +36187 |
| kit_placed | -90 |
| kit_returned | +3 |
| worn_out | -137 |

## Load

| | Whole soak |
|---|---|
| Tick budget | 10.0 ms |
| Ticks | 421352 |
| Tick time, mean | 0.34 ms |
| Tick time, p50 / p95 / p99 | ≤ 0.5 ms / ≤ 0.5 ms / ≤ 2 ms |
| Slowest tick | 44.7 ms |
| Ticks over budget | 14 (0.00%) |
| Worst minute, mean tick | 0.42 ms (p99 ≤ 2 ms) |
| Most in the world | 20 |
| Sockets opened / most open | 20 / 20 |
| Sockets dropped | 0 |
| Sockets resynced | 0 |
| DB transactions a second, mean / busiest minute | 256 / 390 |
| DB pool at the end, size / idle | 16 / 15 |

Sockets that ended: 20 closed.

## Trades

| Trade | Citizens | Strands at the start | At the end | Change each | Made | Sold (Strands) | Bought (Strands) | Deeds | Stuck |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 2 | 200 | 857 | +328 | 6205 | 822 (2471) | 1114 (3489) | 2 | 0 |
| builder | 3 | 300 | 487 | +62 | 5734 | 467 (1295) | 358 (1915) | 3 | 209 |
| crafter-ingots | 2 | 200 | 2613 | +1206 | 3308 | 716 (5849) | 796 (3143) | 2 | 2 |
| crafter-planks | 2 | 200 | 470 | +135 | 5526 | 869 (928) | 1060 (1233) | 2 | 0 |
| crafter-tools | 2 | 200 | 21 | -90 | 9705 | 285 (853) | 344 (1812) | 2 | 0 |
| gatherer-fiber | 3 | 300 | 4086 | +1262 | 10075 | 1764 (5309) | 2334 (7069) | 3 | 0 |
| gatherer-ore | 3 | 300 | 2820 | +840 | 5924 | 594 (3015) | 109 (904) | 3 | 0 |
| gatherer-wood | 3 | 300 | 1761 | +487 | 8071 | 2026 (6403) | 1407 (6857) | 3 | 0 |

In trade windows: 0 trades with people, 22 between citizens (each asked by one), with 1351 Strands changing hands between citizens.

## Citizens

| Citizen | Trade | Strands | Made (most) | Sold (Strands) | Bought (Strands) | Deeds | Failed steps | Stuck (why, most often) |
|---|---|---:|---|---:|---:|---:|---:|---|
| Ada Rushton | gatherer-fiber | 100 → 1883 | 3632 (3350 fiber) | 525 (2047) | 593 (3047) | 1 | 86 | 0 |
| Leonard Forester | gatherer-wood | 100 → 364 | 2839 (1822 fiber) | 884 (2165) | 493 (3275) | 1 | 82 | 0 |
| Doris Miner | gatherer-ore | 100 → 714 | 1406 (574 log) | 199 (1305) | 34 (329) | 1 | 15 | 0 |
| Phyllis Joiner | crafter-planks | 100 → 229 | 2035 (675 plank) | 90 (93) | 18 (73) | 1 | 25 | 0 |
| Harold Ironside | crafter-ingots | 100 → 761 | 1723 (447 plank) | 551 (2954) | 480 (1896) | 1 | 14 | 1 (the kiln waits for a plot, and the plot for the road in's deed) |
| Agnes Toller | crafter-tools | 100 → 11 | 5076 (2264 tool_handle) | 146 (405) | 168 (731) | 1 | 11 | 0 |
| Lillian Mason | builder | 100 → 26 | 2902 (1074 fiber) | 192 (482) | 251 (1337) | 1 | 32 | 52 (the kiln waits for a plot, and the plot for the road in's deed) |
| Dorothy Reeder | all-rounder | 100 → 16 | 3408 (1371 fiber) | 706 (2042) | 967 (2917) | 1 | 72 | 0 |
| Reginald Ashby | gatherer-wood | 100 → 1144 | 2234 (889 log) | 873 (3217) | 790 (2131) | 1 | 31 | 0 |
| Hazel Leadbetter | gatherer-ore | 100 → 1517 | 2954 (1763 fiber) | 258 (974) | 29 (301) | 1 | 68 | 0 |
| Albert Turner | crafter-planks | 100 → 241 | 3491 (1081 fiber) | 779 (835) | 1042 (1160) | 1 | 26 | 0 |
| Lionel Furness | all-rounder | 100 → 841 | 2797 (1480 fiber) | 116 (429) | 147 (572) | 1 | 74 | 0 |
| Edith Wright | crafter-tools | 100 → 10 | 4629 (1418 tool_handle) | 139 (448) | 176 (1081) | 1 | 13 | 0 |
| Rose Thatcher | builder | 100 → 35 | 1299 (404 log) | 142 (331) | 71 (299) | 1 | 7 | 108 (the kiln waits for a plot, and the plot for the road in's deed) |
| Herbert Flaxman | gatherer-fiber | 100 → 1780 | 3744 (2801 fiber) | 101 (656) | 71 (992) | 1 | 98 | 0 |
| Alice Hewitt | gatherer-wood | 100 → 253 | 2998 (1090 fiber) | 269 (1021) | 124 (1451) | 1 | 85 | 0 |
| Mabel Delve | gatherer-ore | 100 → 589 | 1564 (518 log) | 137 (736) | 46 (274) | 1 | 11 | 0 |
| Edwin Smith | crafter-ingots | 100 → 1852 | 1585 (347 fiber) | 165 (2895) | 316 (1247) | 1 | 16 | 1 (the kiln waits for a plot, and the plot for the road in's deed) |
| Ruth Slater | builder | 100 → 426 | 1533 (336 log) | 133 (482) | 36 (279) | 1 | 11 | 49 (the kiln waits for a plot, and the plot for the road in's deed) |
| Hilda Hayward | gatherer-fiber | 100 → 423 | 2699 (2394 fiber) | 1138 (2606) | 1670 (3030) | 1 | 66 | 0 |

## Time by activity

What each trade's and each citizen's work went on, as a share of its work time. Free is the share of all its time that had nothing to do in it, market hours with nothing to trade among it. Setup is the share of the work on tools, kits, stations and deeds on the way to something else. A day is the day's clock's: jobs are the eight a citizen can lean over, and activities those and the Commons, detours and the road in, for each day it worked (the mean, and the fewest).

| Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 19.4 | 16% | 13% | 8% | 8% | 8% | 9% | 15% | 19% | 2% | 1% | 1% | 14% | 1% | 7.5 (7) | 10.0 (10) |
| builder | 26.3 | 5% | 14% | 5% | 10% | 10% | 3% | 40% | 10% | 1% | 2% | 1% | 23% | 1% | 5.7 (4) | 8.0 (6) |
| crafter-ingots | 20.2 |  |  | 10% | 7% | 38% | 15% | 8% | 14% | 1% | 6% | 1% | 9% | 2% | 5.5 (5) | 8.5 (8) |
| crafter-planks | 18.6 | 8% | 18% |  | 43% |  | 20% | 0% | 5% | 1% | 3% | 1% | 18% | 1% | 5.5 (5) | 7.5 (7) |
| crafter-tools | 19.9 | 7% | 3% | 9% | 11% |  | 53% | 11% | 2% | 0% | 3% | 1% | 13% | 1% | 5.5 (5) | 8.0 (8) |
| gatherer-fiber | 30.8 | 46% | 10% | 7% |  | 4% |  | 0% | 24% | 3% | 5% | 1% | 9% | 1% | 5.0 (4) | 8.0 (7) |
| gatherer-ore | 28.4 | 5% | 16% | 49% |  | 11% | 10% | 0% | 2% | 0% | 5% | 1% | 16% | 3% | 6.0 (5) | 8.3 (7) |
| gatherer-wood | 30.1 | 11% | 43% |  | 14% |  | 3% | 8% | 15% | 1% | 6% | 1% | 11% | 2% | 5.3 (5) | 7.7 (7) |

| Citizen | Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Ada Rushton | gatherer-fiber | 10.2 | 49% | 7% | 3% |  |  |  | 0% | 31% | 2% | 5% | 1% | 9% | 1% | 5.0 (5) | 8.0 (8) |
| Leonard Forester | gatherer-wood | 10.0 | 16% | 36% |  | 13% |  |  | 3% | 21% |  | 10% | 0% | 13% | 1% | 5.0 (5) | 7.0 (7) |
| Doris Miner | gatherer-ore | 9.7 |  | 14% | 61% |  | 11% |  | 0% | 3% |  | 10% | 1% | 16% | 4% | 5.0 (5) | 7.0 (7) |
| Phyllis Joiner | crafter-planks | 9.1 |  | 21% |  | 46% |  | 23% | 0% | 3% |  | 6% | 1% | 19% | 2% | 5.0 (5) | 7.0 (7) |
| Harold Ironside | crafter-ingots | 9.6 |  |  | 13% | 14% | 28% | 14% | 13% | 8% | 1% | 8% | 1% | 14% | 3% | 6.0 (6) | 9.0 (9) |
| Agnes Toller | crafter-tools | 10.4 | 2% |  | 17% | 21% |  | 53% | 0% | 3% |  | 2% | 1% | 11% | 1% | 6.0 (6) | 8.0 (8) |
| Lillian Mason | builder | 10.3 | 12% | 11% |  | 7% | 8% | 7% | 30% | 21% | 0% | 3% | 1% | 10% | 1% | 7.0 (7) | 10.0 (10) |
| Dorothy Reeder | all-rounder | 9.9 | 13% | 15% | 16% | 9% | 8% | 5% | 17% | 13% | 2% |  | 1% | 13% | 2% | 8.0 (8) | 10.0 (10) |
| Reginald Ashby | gatherer-wood | 11.0 | 8% | 54% |  | 14% |  |  | 0% | 19% |  | 5% | 1% | 2% | 2% | 5.0 (5) | 7.0 (7) |
| Hazel Leadbetter | gatherer-ore | 9.7 | 15% | 14% | 35% |  | 10% | 18% | 1% | 0% | 1% | 5% | 2% | 13% | 1% | 7.0 (7) | 10.0 (10) |
| Albert Turner | crafter-planks | 9.5 | 15% | 16% |  | 40% |  | 18% | 0% | 8% | 1% |  | 1% | 17% | 1% | 6.0 (6) | 8.0 (8) |
| Lionel Furness | all-rounder | 9.5 | 19% | 10% |  | 8% | 8% | 12% | 12% | 25% | 1% | 3% | 1% | 15% | 1% | 7.0 (7) | 10.0 (10) |
| Edith Wright | crafter-tools | 9.5 | 12% | 7% |  |  |  | 54% | 22% | 1% | 1% | 3% | 1% | 15% | 0% | 5.0 (5) | 8.0 (8) |
| Rose Thatcher | builder | 7.4 |  | 23% |  | 5% |  |  | 64% | 4% |  | 3% | 1% | 34% | 0% | 4.0 (4) | 6.0 (6) |
| Herbert Flaxman | gatherer-fiber | 11.1 | 42% | 13% | 15% |  |  |  |  | 19% | 4% | 7% | 1% |  | 1% | 4.0 (4) | 7.0 (7) |
| Alice Hewitt | gatherer-wood | 9.1 | 10% | 35% |  | 15% |  | 10% | 22% | 2% | 2% | 3% | 1% | 18% | 1% | 6.0 (6) | 9.0 (9) |
| Mabel Delve | gatherer-ore | 9.0 |  | 20% | 51% |  | 11% | 13% | 0% | 2% |  | 0% | 1% | 19% | 2% | 6.0 (6) | 8.0 (8) |
| Edwin Smith | crafter-ingots | 10.7 |  |  | 8% |  | 47% | 16% | 3% | 19% | 1% | 5% | 1% | 4% | 1% | 5.0 (5) | 8.0 (8) |
| Ruth Slater | builder | 8.5 |  | 10% | 15% | 20% | 20% |  | 31% | 1% | 2% |  | 1% | 26% | 1% | 6.0 (6) | 8.0 (8) |
| Hilda Hayward | gatherer-fiber | 9.5 | 47% | 10% | 4% |  | 12% |  | 1% | 22% | 1% | 3% | 2% | 17% | 2% | 6.0 (6) | 9.0 (9) |

Detours: 17 of 20 citizens took one, 4.1% of the work.

## How questions were settled

| Settled by | Questions | Share |
|---|---:|---:|
| rules | 41520 | 92.7% |
| rules, unsure | 3268 | 7.3% |

## Chain bottlenecks

- `gather fiber` took 14% of all bot time (30.0 bot-hours over 11138 steps, 561 failed)
- `gather log` took 11% of all bot time (23.9 bot-hours over 6027 steps, 179 failed)
- `make a market` took 9% of all bot time (19.6 bot-hours over 22 steps)
- iron_ore failed 46% of its pulls (1440 of 3115)
- stone failed 41% of its pulls (699 of 1683)
- `take work: craft rope by hand` failed 30% of the time
- stuck 211×: the kiln waits for a plot, and the plot for the road in's deed

## Production

| Item | Gathered | Crafted | Per game hour | p10 | p50 | p90 | First at (h) | s a unit (D5) | Traded | Low | Mean price (D5) | High |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| anvil_kit | 0 | 6 | 0.5 | 49 | 56 | 65 | 4.52 |  |  |  |  |  |
| brick | 0 | 1312 | 112.2 | 50 | 60 | 68 | 1.38 |  | 445 | 1 | 2.9 (8) | 8 |
| charcoal | 0 | 1240 | 106.1 | 36 | 54 | 72 | 1.38 |  |  |  |  |  |
| chest_kit | 0 | 13 | 1.1 | 47 | 58 | 68 | 0.21 |  |  |  |  |  |
| clay | 1437 | 0 | 122.9 | 45 | 65 | 77 | 0.13 | 17 (12) |  |  |  |  |
| fiber | 21499 | 0 | 1839.1 | 43 | 54 | 68 | 0.01 | 5 (4) | 2 | 1 | 1.0 (1) | 1 |
| forge_kit | 0 | 18 | 1.5 | 53 | 57 | 60 | 1.54 |  |  |  |  |  |
| iron_axe | 0 | 3 | 0.3 | 50 | 58 | 58 | 5.70 |  |  |  |  |  |
| iron_axe_head | 0 | 3 | 0.3 | 44 | 53 | 55 | 5.70 |  |  |  |  |  |
| iron_hammer | 0 | 3 | 0.3 | 52 | 53 | 57 | 5.74 |  |  |  |  |  |
| iron_hammer_head | 0 | 3 | 0.3 | 46 | 48 | 52 | 5.73 |  |  |  |  |  |
| iron_ingot | 0 | 419 | 35.8 | 36 | 53 | 69 | 1.56 |  | 164 | 33 | 45.8 (50) | 61 |
| iron_nails | 0 | 150 | 12.8 | 47 | 60 | 72 | 1.94 |  |  |  |  |  |
| iron_ore | 1678 | 0 | 143.5 | 23 | 51 | 76 | 0.12 | 40 (90) | 721 | 1 | 6.8 (20) | 20 |
| iron_pickaxe | 0 | 3 | 0.3 | 53 | 54 | 54 | 5.64 |  |  |  |  |  |
| iron_pickaxe_head | 0 | 3 | 0.3 | 46 | 47 | 49 | 5.63 |  |  |  |  |  |
| iron_sickle | 0 | 3 | 0.3 | 51 | 54 | 57 | 5.76 |  |  |  |  |  |
| iron_sickle_blade | 0 | 3 | 0.3 | 43 | 47 | 53 | 5.75 |  |  |  |  |  |
| kiln_kit | 0 | 18 | 1.5 | 49 | 55 | 63 | 0.16 |  |  |  |  |  |
| knapped_stone | 0 | 388 | 33.2 | 46 | 55 | 65 | 0.03 |  |  |  |  |  |
| log | 7802 | 0 | 667.4 | 32 | 48 | 71 | 0.06 | 11 (32) | 2674 | 1 | 3.0 (8) | 9 |
| plank | 0 | 6639 | 567.9 | 38 | 50 | 68 | 1.99 |  | 3528 | 1 | 1.1 (5) | 5 |
| rope | 0 | 2007 | 171.7 | 43 | 55 | 67 | 0.05 |  |  |  |  |  |
| sawbench_kit | 0 | 14 | 1.2 | 46 | 58 | 64 | 1.98 |  |  |  |  |  |
| sticks | 2667 | 0 | 228.1 | 32 | 46 | 61 | 0.04 | 5 (12) |  |  |  |  |
| stone | 1104 | 0 | 94.4 | 34 | 52 | 69 | 0.02 | 26 (22) | 1 | 6 | 6.0 (5) | 6 |
| stone_axe | 0 | 112 | 9.6 | 49 | 55 | 61 | 0.06 |  | 8 | 48 | 55.4 (50) | 65 |
| stone_hammer | 0 | 18 | 1.5 | 47 | 54 | 63 | 0.06 |  |  |  |  |  |
| stone_pickaxe | 0 | 61 | 5.2 | 51 | 57 | 62 | 0.11 |  |  |  |  |  |
| stone_sickle | 0 | 6 | 0.5 | 53 | 58 | 59 | 0.29 |  |  |  |  |  |
| tool_handle | 0 | 5895 | 504.3 | 41 | 52 | 62 | 0.05 |  |  |  |  |  |
| workbench_kit | 0 | 21 | 1.8 | 48 | 60 | 68 | 0.02 |  |  |  |  |  |

## Pulls that failed

| Item | Pulls | Failed | Share |
|---|---:|---:|---:|
| clay | 965 | 0 | 0% |
| fiber | 10585 | 0 | 0% |
| iron_ore | 3115 | 1440 | 46% |
| log | 5857 | 752 | 13% |
| sticks | 1462 | 0 | 0% |
| stone | 1683 | 699 | 42% |

## Tools worn out

- stone_axe: 88
- stone_pickaxe: 45
- stone_sickle: 4

## Strands

- Fiber deposited: 16763
- Deeds bought: 20 for 0 Strands
- Spent on tax: 0 Strands
- Market fees: 2742 Strands
- A deed (500, in any home zone) is 0.70 hours of grass

## The hall's board

- Commissions posted: 9, 585 Strands put down, 54 in the court's fees
- Claims: 8, promising 8 units; 0 lapsed
- Delivered: 8 units, paid 396 Strands from escrow
- Filled: 8; ran out: 0, 0 Strands refunded
- Strands through the court: 639 put in by these posters, 396 paid out to these deliverers

## Wages

- Shifts taken: 13; ended: 10 at the cap, 1 by the employer, 2 when the work failed
- Units made for employers: 683, paid 916 Strands from their escrow
- Offers of work made: 12, 12 of them worked; taken down: 10 at the cap, 1 with the place gone, 1 as the money ran low
- Units bought on them: 683, paid 916 Strands, and 72 in the court's fees

## Where the time went

| Step | Bot-hours | Steps | Failed |
|---|---:|---:|---:|
| gather fiber | 30.02 | 11138 | 561 |
| gather log | 23.85 | 6027 | 179 |
| make a market | 19.60 | 22 | 0 |
| gather iron_ore | 18.88 | 3182 | 67 |
| at the hall | 16.88 | 997 | 0 |
| at the bazaar | 10.93 | 643 | 0 |
| craft plank | 10.71 | 2200 | 0 |
| craft tool_handle | 10.08 | 3587 | 0 |
| gather stone | 7.90 | 1664 | 17 |
| sell log | 7.42 | 265 | 0 |
| craft charcoal | 7.02 | 523 | 0 |
| gather clay | 6.94 | 968 | 3 |
| craft brick | 5.44 | 656 | 0 |
| craft knapped_stone | 5.11 | 376 | 0 |
| craft iron_ingot | 4.23 | 419 | 0 |
| gather sticks | 4.01 | 1459 | 2 |
| sell iron_ore | 3.38 | 104 | 0 |
| buy log | 2.56 | 112 | 0 |
| craft rope | 2.55 | 1549 | 0 |
| a detour: browse the bazaar lane | 1.68 | 9 | 0 |
| stock log | 1.62 | 71 | 1 |
| a detour: sit a while in a park nearby | 1.50 | 6 | 0 |
| a detour: read the Wanted page on the hall's board | 1.50 | 6 | 0 |
| sell plank | 1.50 | 59 | 0 |
| a detour: read the Gazette at the hall | 1.03 | 5 | 0 |
| buy iron_ore | 1.02 | 37 | 0 |
| sell iron_ingot | 1.00 | 30 | 0 |
| work: craft rope by hand | 0.70 | 457 | 7 |
| craft kiln_kit | 0.55 | 18 | 0 |
| stock iron_ore | 0.53 | 20 | 0 |
| craft stone_axe | 0.52 | 106 | 0 |
| a detour: call on a neighbour | 0.50 | 2 | 0 |
| a detour: see a ford in the hills to the north-east | 0.50 | 2 | 0 |
| a detour: try making sawn handles | 0.50 | 2 | 0 |
| stock plank | 0.50 | 24 | 0 |
| bid for stone_pickaxe | 0.50 | 17 | 0 |
| to the hall | 0.46 | 12 | 0 |
| work: craft charcoal at a kiln | 0.42 | 97 | 0 |
| sell brick | 0.41 | 17 | 0 |
| the road in: look at a farm by the road | 0.41 | 23 | 0 |
| bid for log | 0.34 | 10 | 0 |
| stock brick | 0.33 | 17 | 0 |
| craft stone_pickaxe | 0.33 | 61 | 0 |
| craft forge_kit | 0.32 | 18 | 0 |
| place kiln | 0.25 | 18 | 0 |
| to the bazaar | 0.25 | 7 | 0 |
| a detour: see a ford in the hills to the south-east | 0.25 | 1 | 0 |
| a detour: try making storage chest kit | 0.25 | 1 | 0 |
| a detour: try making iron nails | 0.25 | 1 | 0 |
| bid for stone_axe | 0.23 | 20 | 0 |
| buy stone_axe at a stall | 0.19 | 13 | 0 |
| craft sawbench_kit | 0.18 | 14 | 0 |
| stock stone_axe | 0.18 | 10 | 0 |
| place storage_chest | 0.17 | 11 | 0 |
| the road in: change fiber for Strands at the Exchange | 0.17 | 17 | 0 |
| the road in: see to the market box | 0.17 | 20 | 0 |
| hire: bring home the box | 0.16 | 10 | 0 |
| place forge | 0.15 | 18 | 0 |
| the road in: look at an urban yard by the road | 0.15 | 20 | 0 |
| take work: craft charcoal at a kiln | 0.14 | 4 | 0 |
| take work: craft rope by hand | 0.14 | 10 | 3 |
| hire: craft rope by hand | 0.14 | 7 | 0 |
| craft iron_nails | 0.13 | 14 | 0 |
| the road in: fell a tree | 0.12 | 23 | 0 |
| deliver chest_kit | 0.12 | 6 | 0 |
| sell stone_axe | 0.11 | 8 | 0 |
| the road in: make a stone axe | 0.10 | 32 | 1 |
| the road in: look at a suburban lot by the road | 0.10 | 20 | 0 |
| bid for stone | 0.10 | 3 | 0 |
| buy stone_pickaxe at a stall | 0.10 | 3 | 0 |
| bid for stone_hammer | 0.10 | 7 | 0 |
| craft anvil_kit | 0.10 | 6 | 0 |
| bid for iron_ore | 0.09 | 3 | 0 |
| the road in: claim a home with its deed | 0.09 | 20 | 0 |
| place sawbench | 0.09 | 13 | 0 |
| craft stone_hammer | 0.08 | 18 | 0 |
| the road in: set the workbench up off any lot | 0.07 | 4 | 0 |
| place workbench | 0.07 | 20 | 0 |
| the road in: gather stone by hand | 0.06 | 10 | 0 |
| the road in: set a station up at home | 0.06 | 3 | 0 |
| the road in: leave a sell order on the book | 0.05 | 20 | 0 |
| craft workbench_kit | 0.05 | 18 | 0 |
| the road in: knap stone at the bench | 0.05 | 12 | 0 |
| take work: craft plank at a sawbench | 0.04 | 1 | 0 |
| buy stone | 0.04 | 2 | 0 |
| the road in: say a word to whoever's near | 0.04 | 20 | 0 |
| craft iron_hammer_head | 0.04 | 3 | 0 |
| craft iron_axe_head | 0.04 | 3 | 0 |
| hire: craft charcoal at a kiln | 0.04 | 4 | 0 |
| deliver kiln_kit | 0.04 | 1 | 0 |
| hire: craft plank at a sawbench | 0.04 | 1 | 0 |
| collect from the market box | 0.03 | 2 | 0 |
| place anvil | 0.03 | 6 | 0 |
| craft chest_kit | 0.03 | 12 | 0 |
| the road in: look in at the Deeds Office | 0.03 | 20 | 0 |
| the road in: read someone's card | 0.02 | 22 | 2 |
| work: craft plank at a sawbench | 0.02 | 13 | 0 |
| craft stone_sickle | 0.02 | 6 | 0 |
| craft iron_axe | 0.02 | 3 | 0 |
| craft iron_pickaxe_head | 0.02 | 3 | 0 |
| craft iron_sickle | 0.02 | 3 | 0 |
| craft iron_pickaxe | 0.02 | 3 | 0 |
| craft iron_hammer | 0.02 | 3 | 0 |
| deliver sawbench_kit | 0.02 | 1 | 0 |
| the road in: look in at a bazaar stall | 0.02 | 20 | 0 |
| craft iron_sickle_blade | 0.02 | 3 | 0 |
| deposit fiber | 0.01 | 92 | 0 |
| buy stone_axe | 0.01 | 1 | 0 |
| the road in: make a handle at the bench | 0.01 | 3 | 0 |
| the road in: make a workbench kit | 0.01 | 2 | 0 |
| the road in: read the Registrar's letter | 0.01 | 20 | 0 |
| the road in: read the Gazette | 0.00 | 20 | 0 |
| the road in: pick up fallen branches | 0.00 | 2 | 0 |
| the road in: give a station a job | 0.00 | 20 | 0 |
| the road in: twist a rope | 0.00 | 2 | 0 |
| the road in: leave a buy order on the book | 0.00 | 20 | 0 |
| the road in: read what tax asks | 0.00 | 20 | 0 |
| the road in: read the zone card | 0.00 | 20 | 0 |
| stock stone_pickaxe | 0.00 | 2 | 0 |
| claim chest_kit | 0.00 | 5 | 0 |
| the road in: look in its pack | 0.00 | 20 | 0 |
| claim kiln_kit | 0.00 | 1 | 0 |
| claim sawbench_kit | 0.00 | 2 | 0 |

## Why steps failed

| Step: reason | Times |
|---|---:|
| gather fiber: the server didn't start extracting node #: Someone else is already gathering this Wild Grass | 375 |
| gather fiber: the server didn't start extracting node #: This Wild Grass is spent; it will grow back | 186 |
| gather log: the server didn't start extracting node #: This Tree is spent; it will grow back | 99 |
| gather log: the server didn't start extracting node #: Someone else is already gathering this Tree | 79 |
| gather iron_ore: the server didn't start extracting node #: This Iron Vein is spent; it will grow back | 41 |
| gather iron_ore: the server didn't start extracting node #: Someone else is already gathering this Iron Vein | 26 |
| gather stone: the server didn't start extracting node #: Someone else is already gathering this Rock Outcrop | 9 |
| gather stone: the server didn't start extracting node #: This Rock Outcrop is spent; it will grow back | 8 |
| work: craft rope by hand: blocked at (#, #): No way there | 3 |
| gather clay: the server didn't start extracting node #: Someone else is already gathering this Clay Bank | 2 |
| take work: craft rope by hand: taking work: Someone is working that offer already (HTTP #) | 2 |
| the road in: read someone's card: nobody about, nor in the Gazette, whose card to read | 2 |
