# The town

32 citizens (4 all-rounder, 3 builder, 3 crafter-ingots, 3 crafter-planks, 3 crafter-tools, 4 gatherer-fiber, 5 gatherer-ore, 5 gatherer-wood, 2 merchant) for 12.0 hours of the day's clock, 10.0 hours of ticks at 10× (60.1 wall minutes).

## Conservation

**Holds.**

### Strands

Every account's Strands, with escrow: 3300 at the start, 19988 at the end. That's the start, plus 0 new accounts at 100, plus 23543 in from the faucets, less 6855 gone to the sinks, plus 0 the transfers net to: 19988 expected, 0 off.

| Flow | Kind | Strands |
|---|---|---:|
| bazaar_rent | Sink | -1160 |
| court_escrow | Escrow | -3590 |
| court_fee | Sink | -120 |
| court_payout | Escrow | +3404 |
| court_refund | Escrow | +186 |
| exchange_deposit | Faucet | +23543 |
| market_escrow | Escrow | -68819 |
| market_fee | Sink | -5575 |
| market_refund | Escrow | +29818 |
| market_sale | Escrow | +36136 |
| trade_received | Transfer | +816 |
| trade_sent | Transfer | -816 |

Escrow: 0 at the start, 2865 at the end; 0 unexplained.

### Units

| Item | Change | Explained | Drift |
|---|---:|---:|---:|
| brick | +1226 | +1226 | 0 |
| charcoal | +79 | +79 | 0 |
| chest_kit | +2 | +2 | 0 |
| clay | +26 | +26 | 0 |
| fiber | +1685 | +1685 | 0 |
| forge_kit | +1 | +1 | 0 |
| iron_axe | +11 | +11 | 0 |
| iron_hammer | +11 | +11 | 0 |
| iron_ingot | +414 | +414 | 0 |
| iron_nails | +10 | +10 | 0 |
| iron_ore | +1827 | +1827 | 0 |
| iron_pickaxe | +11 | +11 | 0 |
| iron_sickle | +12 | +12 | 0 |
| iron_sickle_blade | +1 | +1 | 0 |
| kiln_kit | +1 | +1 | 0 |
| knapped_stone | +7 | +7 | 0 |
| log | +4627 | +4627 | 0 |
| plank | +6027 | +6027 | 0 |
| rope | +1 | +1 | 0 |
| sticks | +25 | +25 | 0 |
| stone | +18 | +18 | 0 |
| stone_axe | +53 | +53 | 0 |
| stone_hammer | +27 | +27 | 0 |
| stone_pickaxe | +29 | +29 | 0 |
| stone_sickle | +18 | +18 | 0 |
| tool_handle | +2858 | +2858 | 0 |
| workbench_kit | +6 | +6 | 0 |

| Flow | Units |
|---|---:|
| consumed | -15322 |
| crafted | +16262 |
| deposited | -23616 |
| gathered | +41962 |
| kit_placed | -145 |
| kit_returned | +11 |
| worn_out | -139 |

## Load

| | Whole soak |
|---|---|
| Tick budget | 10.0 ms |
| Ticks | 361225 |
| Tick time, mean | 0.35 ms |
| Tick time, p50 / p95 / p99 | 0.29 ms / 0.67 ms / 1.19 ms |
| Slowest tick | 1136.2 ms |
| Ticks over budget | 160 (0.04%) |
| Worst minute, mean tick | 0.70 ms (p99 5.83 ms) |
| Most in the world | 32 |
| Sockets opened / most open | 32 / 32 |
| Sockets dropped | 0 |
| Sockets resynced | 0 |
| DB transactions a second, mean / busiest minute | 331 / 429 |
| DB pool at the end, size / idle | 8 / 7 |

Sockets that ended: 32 closed.

## Trades

| Trade | Citizens | Strands at the start | At the end | Change each | Made | Sold (Strands) | Bought (Strands) | Deeds | Stuck |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 4 | 400 | 1453 | +263 | 5504 | 1605 (5045) | 1944 (5226) | 4 | 0 |
| builder | 3 | 300 | 114 | -62 | 4328 | 970 (1472) | 679 (1895) | 3 | 0 |
| crafter-ingots | 3 | 300 | 896 | +199 | 3349 | 249 (5982) | 417 (5195) | 3 | 1 |
| crafter-planks | 3 | 300 | 281 | -6 | 4968 | 1937 (2302) | 1576 (2454) | 3 | 0 |
| crafter-tools | 3 | 300 | 375 | +25 | 8514 | 501 (2072) | 628 (2959) | 3 | 0 |
| gatherer-fiber | 4 | 400 | 5626 | +1306 | 11176 | 1128 (2396) | 1886 (5704) | 4 | 0 |
| gatherer-ore | 5 | 500 | 4634 | +827 | 5645 | 1015 (12045) | 1146 (3943) | 5 | 0 |
| gatherer-wood | 5 | 500 | 3212 | +542 | 10400 | 1947 (3232) | 311 (4501) | 5 | 0 |
| merchant | 2 | 200 | 432 | +116 | 4340 | 608 (1536) | 1346 (4227) | 2 | 0 |

In trade windows: 0 trades with people, 9 between citizens (each asked by one), with 816 Strands changing hands between citizens.

## Citizens

| Citizen | Trade | Strands | Made (most) | Sold (Strands) | Bought (Strands) | Deeds | Failed steps | Stuck (why, most often) |
|---|---|---:|---|---:|---:|---:|---:|---|
| Ada Rushton | gatherer-fiber | 100 → 2444 | 3536 (3386 fiber) | 308 (792) | 604 (1509) | 1 | 143 | 0 |
| Leonard Forester | gatherer-wood | 100 → 78 | 1238 (584 log) | 368 (770) | 41 (998) | 1 | 47 | 0 |
| Doris Miner | gatherer-ore | 100 → 1562 | 796 (276 log) | 129 (2467) | 12 (137) | 1 | 51 | 0 |
| Phyllis Joiner | crafter-planks | 100 → 188 | 1719 (1104 plank) | 198 (206) | 84 (158) | 1 | 9 | 0 |
| Harold Ironside | crafter-ingots | 100 → 7 | 1057 (337 fiber) | 146 (1520) | 185 (1573) | 1 | 12 | 1 (the kiln waits for a plot, and the plot for the road in's deed) |
| Agnes Toller | crafter-tools | 100 → 11 | 3328 (1508 tool_handle) | 101 (462) | 230 (544) | 1 | 13 | 0 |
| Lillian Mason | builder | 100 → 12 | 1272 (434 fiber) | 264 (326) | 138 (618) | 1 | 63 | 0 |
| Dorothy Reeder | all-rounder | 100 → 9 | 1710 (780 fiber) | 193 (1019) | 89 (1516) | 1 | 40 | 0 |
| Reginald Ashby | gatherer-wood | 100 → 662 | 2874 (1107 fiber) | 652 (721) | 17 (516) | 1 | 37 | 0 |
| Hazel Leadbetter | gatherer-ore | 100 → 997 | 2082 (1398 fiber) | 69 (3032) | 80 (1560) | 1 | 43 | 0 |
| Albert Turner | crafter-planks | 100 → 63 | 1663 (741 plank) | 1413 (1656) | 1323 (1563) | 1 | 2 | 0 |
| Lionel Furness | all-rounder | 100 → 600 | 1302 (416 fiber) | 219 (925) | 181 (920) | 1 | 41 | 0 |
| Edith Wright | crafter-tools | 100 → 307 | 3109 (1348 fiber) | 324 (652) | 186 (1450) | 1 | 60 | 0 |
| Rose Thatcher | builder | 100 → 7 | 1524 (421 log) | 66 (145) | 102 (460) | 1 | 33 | 0 |
| Herbert Flaxman | gatherer-fiber | 100 → 1364 | 3031 (2809 fiber) | 395 (779) | 473 (1903) | 1 | 177 | 0 |
| Alice Hewitt | gatherer-wood | 100 → 453 | 2078 (967 fiber) | 81 (162) | 93 (1230) | 1 | 50 | 0 |
| Mabel Delve | gatherer-ore | 100 → 2043 | 834 (277 iron_ore) | 170 (4191) | 24 (233) | 1 | 18 | 0 |
| Edwin Sawyer | crafter-planks | 100 → 30 | 1586 (720 plank) | 326 (440) | 169 (733) | 1 | 14 | 0 |
| Ruth Furness | crafter-ingots | 100 → 867 | 1496 (740 fiber) | 11 (568) | 52 (223) | 1 | 28 | 0 |
| Hilda Arkwright | crafter-tools | 100 → 57 | 2077 (1019 tool_handle) | 76 (958) | 212 (965) | 1 | 13 | 0 |
| Arthur Mason | all-rounder | 100 → 240 | 893 (333 fiber) | 378 (1395) | 578 (1249) | 1 | 14 | 0 |
| Maud Roper | gatherer-fiber | 100 → 1476 | 2431 (2325 fiber) | 91 (292) | 121 (842) | 1 | 119 | 0 |
| Elsie Underwood | gatherer-wood | 100 → 1160 | 1810 (742 log) | 495 (1009) | 41 (491) | 1 | 76 | 0 |
| Sidney Miner | gatherer-ore | 100 → 18 | 1300 (349 iron_ore) | 405 (600) | 266 (486) | 1 | 30 | 0 |
| Ida Ironmonger | crafter-ingots | 100 → 22 | 796 (169 iron_ore) | 92 (3894) | 180 (3399) | 1 | 23 | 0 |
| Beatrice Builder | builder | 100 → 95 | 1532 (422 log) | 640 (1001) | 439 (817) | 1 | 34 | 0 |
| Mildred Flaxman | all-rounder | 100 → 604 | 1599 (911 fiber) | 815 (1706) | 1096 (1541) | 1 | 13 | 0 |
| Ernest Ashby | gatherer-wood | 100 → 859 | 2400 (963 fiber) | 351 (570) | 119 (1266) | 1 | 64 | 0 |
| Stanley Quarry | gatherer-ore | 100 → 14 | 633 (223 iron_ore) | 242 (1755) | 764 (1527) | 1 | 15 | 0 |
| Iris Fielding | gatherer-fiber | 100 → 342 | 2178 (1778 fiber) | 334 (533) | 688 (1450) | 1 | 126 | 0 |
| Bernard Spicer | merchant | 100 → 423 | 2027 (2012 fiber) | 38 (399) | 222 (1427) | 1 | 76 | 0 |
| Nora Draper | merchant | 100 → 9 | 2313 (2278 fiber) | 570 (1137) | 1124 (2800) | 1 | 125 | 0 |

## Time by activity

What each trade's and each citizen's work went on, as a share of its work time. Free is the share of all its time that had nothing to do in it, market hours with nothing to trade among it. Setup is the share of the work on tools, kits, stations and deeds on the way to something else. A day is the day's clock's: jobs are the eight a citizen can lean over, and activities those and the Commons, detours and the road in, for each day it worked (the mean, and the fewest).

| Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 33.7 | 8% | 7% | 15% | 7% | 13% | 11% | 10% | 21% | 0% | 5% | 2% | 12% | 2% | 7.8 (7) | 9.8 (9) |
| builder | 24.2 | 6% | 19% |  | 9% | 3% | 2% | 41% | 10% | 1% | 7% | 2% | 15% | 1% | 5.7 (5) | 8.0 (7) |
| crafter-ingots | 23.0 | 7% |  | 12% | 5% | 38% | 11% | 16% | 1% | 0% | 8% | 2% | 19% | 1% | 6.0 (5) | 8.3 (7) |
| crafter-planks | 23.0 | 3% | 11% |  | 43% | 1% | 20% | 6% | 11% | 0% | 4% | 1% | 22% | 1% | 6.0 (5) | 8.3 (7) |
| crafter-tools | 23.9 | 7% | 6% | 7% | 8% | 2% | 51% | 9% | 2% | 0% | 7% | 2% | 18% | 2% | 5.3 (5) | 7.7 (7) |
| gatherer-fiber | 32.8 | 45% | 9% | 7% |  |  |  | 0% | 29% | 0% | 7% | 3% | 14% | 1% | 3.8 (3) | 6.0 (5) |
| gatherer-ore | 39.5 | 3% | 10% | 48% | 1% | 7% | 9% | 2% | 8% | 1% | 8% | 2% | 17% | 3% | 5.8 (5) | 8.2 (7) |
| gatherer-wood | 38.0 | 13% | 43% | 3% | 10% |  | 1% | 16% | 3% | 2% | 6% | 2% | 20% | 3% | 5.2 (5) | 8.2 (8) |
| merchant | 18.0 | 38% |  |  |  | 1% |  |  | 54% |  | 6% | 2% | 8% | 0% | 2.5 (2) | 4.5 (4) |

| Citizen | Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Ada Rushton | gatherer-fiber | 9.0 | 45% | 5% | 2% |  |  |  | 1% | 37% |  | 8% | 2% | 5% | 1% | 5.0 (5) | 7.0 (7) |
| Leonard Forester | gatherer-wood | 8.1 | 4% | 44% | 14% |  |  | 7% | 16% | 3% | 1% | 9% | 1% | 17% | 3% | 6.0 (6) | 9.0 (9) |
| Doris Miner | gatherer-ore | 7.5 |  | 16% | 52% |  | 12% |  | 0% | 3% | 1% | 13% | 2% | 20% | 5% | 5.0 (5) | 8.0 (8) |
| Phyllis Joiner | crafter-planks | 7.3 |  | 4% |  | 56% |  | 27% | 1% | 2% |  | 10% | 1% | 23% | 1% | 5.0 (5) | 7.0 (7) |
| Harold Ironside | crafter-ingots | 7.8 | 7% |  | 12% | 15% | 25% | 15% | 15% | 2% |  | 10% | 1% | 17% | 1% | 7.0 (7) | 9.0 (9) |
| Agnes Toller | crafter-tools | 8.5 | 4% |  | 18% | 11% |  | 55% | 1% | 3% |  | 6% | 2% | 14% | 2% | 6.0 (6) | 8.0 (8) |
| Lillian Mason | builder | 7.3 | 9% | 18% |  | 13% | 10% | 7% | 26% | 2% | 0% | 12% | 2% | 24% | 1% | 7.0 (7) | 10.0 (10) |
| Dorothy Reeder | all-rounder | 7.7 | 11% | 18% | 22% | 8% | 11% | 8% | 18% | 3% |  |  | 1% | 17% | 3% | 8.0 (8) | 9.0 (9) |
| Reginald Ashby | gatherer-wood | 7.6 | 15% | 53% |  | 18% |  |  | 0% | 3% | 0% | 10% | 1% | 21% | 4% | 5.0 (5) | 8.0 (8) |
| Hazel Leadbetter | gatherer-ore | 8.6 | 15% |  | 43% |  | 11% | 14% | 6% | 2% | 1% | 9% | 1% | 12% | 3% | 6.0 (6) | 9.0 (9) |
| Albert Turner | crafter-planks | 7.9 | 3% | 17% |  | 33% |  | 21% | 0% | 25% | 0% |  | 1% | 21% | 2% | 6.0 (6) | 8.0 (8) |
| Lionel Furness | all-rounder | 9.5 | 10% | 7% | 10% | 9% | 12% | 11% | 7% | 25% |  | 6% | 2% | 0% | 1% | 8.0 (8) | 10.0 (10) |
| Edith Wright | crafter-tools | 7.9 | 14% | 17% |  |  |  | 32% | 23% | 2% | 0% | 10% | 2% | 17% | 2% | 5.0 (5) | 8.0 (8) |
| Rose Thatcher | builder | 7.6 | 6% | 21% |  | 9% |  |  | 56% | 3% | 1% |  | 2% | 20% | 0% | 5.0 (5) | 7.0 (7) |
| Herbert Flaxman | gatherer-fiber | 8.5 | 47% | 9% | 13% |  |  |  |  | 25% |  | 3% | 3% | 13% | 1% | 4.0 (4) | 6.0 (6) |
| Alice Hewitt | gatherer-wood | 7.4 | 22% | 22% |  | 22% |  |  | 27% | 3% | 2% | 1% | 2% | 21% | 2% | 5.0 (5) | 8.0 (8) |
| Mabel Delve | gatherer-ore | 7.4 |  | 13% | 64% |  |  | 14% | 1% | 3% | 1% |  | 3% | 22% | 3% | 5.0 (5) | 7.0 (7) |
| Edwin Sawyer | crafter-planks | 7.8 | 7% | 12% |  | 42% | 2% | 13% | 16% | 5% | 0% | 3% | 1% | 21% | 1% | 7.0 (7) | 10.0 (10) |
| Ruth Furness | crafter-ingots | 7.6 | 13% |  | 6% |  | 28% | 19% | 20% | 1% | 1% | 10% | 3% | 20% | 1% | 6.0 (6) | 9.0 (9) |
| Hilda Arkwright | crafter-tools | 7.5 | 2% |  |  | 12% | 8% | 66% | 2% |  |  | 7% | 2% | 22% | 2% | 5.0 (5) | 7.0 (7) |
| Arthur Mason | all-rounder | 8.3 | 3% |  | 14% | 5% | 10% | 9% | 18% | 27% |  | 12% | 3% | 17% | 2% | 7.0 (7) | 9.0 (9) |
| Maud Roper | gatherer-fiber | 7.5 | 44% |  | 15% |  |  |  |  | 27% |  | 10% | 4% | 21% | 0% | 3.0 (3) | 5.0 (5) |
| Elsie Underwood | gatherer-wood | 7.5 | 12% | 59% |  | 4% |  |  | 17% | 3% | 0% | 3% | 2% | 20% | 4% | 5.0 (5) | 8.0 (8) |
| Sidney Miner | gatherer-ore | 8.4 | 1% | 13% | 29% | 6% | 8% | 10% | 0% | 17% |  | 12% | 3% | 15% | 1% | 8.0 (8) | 10.0 (10) |
| Ida Ironmonger | crafter-ingots | 7.6 | 3% |  | 18% |  | 61% |  | 14% | 0% |  | 3% | 2% | 20% | 1% | 5.0 (5) | 7.0 (7) |
| Beatrice Builder | builder | 9.3 | 2% | 18% |  | 7% |  |  | 41% | 22% |  | 8% | 2% | 2% | 1% | 5.0 (5) | 7.0 (7) |
| Mildred Flaxman | all-rounder | 8.2 | 9% | 3% | 15% | 5% | 18% | 16% | 0% | 28% | 1% | 3% | 2% | 16% | 1% | 8.0 (8) | 11.0 (11) |
| Ernest Ashby | gatherer-wood | 7.4 | 13% | 35% |  | 10% |  |  | 21% | 4% | 6% | 8% | 3% | 22% | 1% | 5.0 (5) | 8.0 (8) |
| Stanley Quarry | gatherer-ore | 7.7 | 0% | 9% | 57% |  |  | 7% |  | 16% |  | 8% | 3% | 18% | 3% | 5.0 (5) | 7.0 (7) |
| Iris Fielding | gatherer-fiber | 7.8 | 41% | 23% |  |  |  |  |  | 26% | 1% | 6% | 3% | 15% | 1% | 3.0 (3) | 6.0 (6) |
| Bernard Spicer | merchant | 9.7 | 38% |  |  |  |  |  |  | 55% |  | 5% | 2% |  |  | 2.0 (2) | 4.0 (4) |
| Nora Draper | merchant | 8.3 | 38% |  |  |  | 2% |  |  | 52% |  | 6% | 2% | 17% | 0% | 3.0 (3) | 5.0 (5) |

Detours: 28 of 32 citizens took one, 6.6% of the work.

## Callings (§10)

Each style's favourite's share of its citizens' time, the median, against §10's band: specialists 60%–80%, leaners 35%–55% of the work, and no all-rounder's biggest job over 35%. Of the jobs counts the eight, trading among them; of the making jobs counts the seven without it (#555), so a trading favourite has none.

| Style | Citizens | Of the work | Of the jobs | Of the making jobs | In band |
|---|---:|---:|---:|---:|---:|
| specialist | 16 | 55% | 60% | 65% | 3 of 16 (19%) |
| leaner | 12 | 32% | 35% | 44% | 5 of 12 (42%) |
| all-rounder | 4 | 26% (biggest job) | — | — | 4 of 4 (100%) |

The fewest activities a citizen worked at in a day (§10: 3, all-rounders 5); who took a detour, and the days worked with one (70%); what came of asking the citizens near it (K3.2); and whether its card's "lately busy with" says the making job it gave most time to (80%). A card names only what's made, so trading isn't held up against it.

| | The town |
|---|---:|
| Fewest activities in a day | 4 |
| Fewest activities in a day, all-rounders | 9 |
| Citizens who took a detour | 28 of 32 (88%) |
| Days worked with a detour | 28 of 32 (88%) |
| Asked a citizen near it | 16 |
| … and traded / declined / ended / failed | 9 / 0 / 7 / 0 |
| Had someone to ask, and let them be | 63 |
| Cards read (quiet) | 32 (0) |
| "Lately busy with" leads with the most time | 23 of 32 (72%) |
| … or names it among its three | 29 of 32 (91%) |

<details><summary>each citizen's calling</summary>

| Citizen | Style | Favourite | Of the work | Of the making jobs | Biggest job | In band | Fewest a day | Detour days | Most made | Lately |
|---|---|---|---:|---:|---:|---|---:|---:|---|---|
| Ada Rushton | specialist | gatherer-fiber | 45% | 86% | fiber 45% | no | 7 | 1 of 1 | fiber | fiber, wood |
| Leonard Forester | leaner | gatherer-wood | 44% | 52% | wood 44% | yes | 9 | 1 of 1 | wood | wood, ore |
| Doris Miner | specialist | gatherer-ore | 52% | 64% | ore 52% | no | 8 | 1 of 1 | ore | ore, wood, ingots |
| Phyllis Joiner | specialist | crafter-planks | 56% | 64% | planks 56% | no | 7 | 1 of 1 | planks | planks, wood |
| Harold Ironside | leaner | crafter-ingots | 25% | 28% | ingots 25% | no | 9 | 1 of 1 | ingots | ingots, ore |
| Agnes Toller | specialist | crafter-tools | 55% | 61% | tools 55% | no | 8 | 1 of 1 | tools | tools, ore, planks |
| Lillian Mason | leaner | builder | 26% | 31% | building 26% | no | 10 | 1 of 1 | building | wood, planks, tools |
| Dorothy Reeder | all-rounder | — | — | — | ore 22% | yes | 9 | 0 of 1 | ore | ore, wood, ingots |
| Reginald Ashby | specialist | gatherer-wood | 53% | 62% | wood 53% | no | 8 | 1 of 1 | wood | wood, planks |
| Hazel Leadbetter | leaner | gatherer-ore | 43% | 48% | ore 43% | yes | 9 | 1 of 1 | ore | ore, ingots |
| Albert Turner | leaner | crafter-planks | 33% | 45% | planks 33% | no | 8 | 0 of 1 | planks | wood, planks |
| Lionel Furness | all-rounder | — | — | — | trading 25% | yes | 10 | 1 of 1 | ingots | ore, ingots, wood |
| Edith Wright | leaner | crafter-tools | 32% | 37% | tools 32% | no | 8 | 1 of 1 | tools | tools, wood, building |
| Rose Thatcher | specialist | builder | 56% | 60% | building 56% | no | 7 | 0 of 1 | building | wood, building, planks |
| Herbert Flaxman | specialist | gatherer-fiber | 47% | 68% | fiber 47% | no | 6 | 1 of 1 | fiber | fiber, wood, ore |
| Alice Hewitt | leaner | gatherer-wood | 22% | 24% | building 27% | no | 8 | 1 of 1 | building | wood, planks, fiber |
| Mabel Delve | specialist | gatherer-ore | 64% | 69% | ore 64% | yes | 7 | 0 of 1 | ore | ore, wood |
| Edwin Sawyer | leaner | crafter-planks | 42% | 46% | planks 42% | yes | 10 | 1 of 1 | planks | planks, wood |
| Ruth Furness | leaner | crafter-ingots | 28% | 32% | ingots 28% | no | 9 | 1 of 1 | ingots | ingots, ore |
| Hilda Arkwright | specialist | crafter-tools | 66% | 73% | tools 66% | yes | 7 | 1 of 1 | tools | tools, planks |
| Arthur Mason | all-rounder | — | — | — | trading 27% | yes | 9 | 1 of 1 | building | ore, ingots, tools |
| Maud Roper | specialist | gatherer-fiber | 44% | 75% | fiber 44% | no | 5 | 1 of 1 | fiber | fiber, ore |
| Elsie Underwood | specialist | gatherer-wood | 59% | 65% | wood 59% | no | 8 | 1 of 1 | wood | wood |
| Sidney Miner | leaner | gatherer-ore | 29% | 43% | ore 29% | no | 10 | 1 of 1 | ore | ore, wood, ingots |
| Ida Ironmonger | specialist | crafter-ingots | 61% | 64% | ingots 61% | yes | 7 | 1 of 1 | ingots | ingots, ore |
| Beatrice Builder | leaner | builder | 41% | 60% | building 41% | yes | 7 | 1 of 1 | building | wood, building, planks |
| Mildred Flaxman | all-rounder | — | — | — | trading 28% | yes | 11 | 1 of 1 | ingots | ore, ingots |
| Ernest Ashby | leaner | gatherer-wood | 35% | 44% | wood 35% | yes | 8 | 1 of 1 | wood | wood, planks |
| Stanley Quarry | specialist | gatherer-ore | 57% | 78% | ore 57% | no | 7 | 1 of 1 | ore | ore, wood |
| Iris Fielding | specialist | gatherer-fiber | 41% | 65% | fiber 41% | no | 6 | 1 of 1 | fiber | wood, fiber |
| Bernard Spicer | specialist | merchant | 55% | — | trading 55% | no | 4 | 1 of 1 | fiber | fiber |
| Nora Draper | specialist | merchant | 52% | — | trading 52% | no | 5 | 1 of 1 | fiber | fiber |

</details>

## How questions were settled

| Settled by | Questions | Share |
|---|---:|---:|
| rules | 64751 | 92.5% |
| rules, unsure | 5226 | 7.5% |

## What citizens said

| Found by | Lines | Share |
|---|---:|---:|
| phrasebook | 81 | 100.0% |

No model was asked for a line: every word was the phrasebook's.

Their words cost 0.000¢, 0.0000¢ a citizen. The most one citizen's life spent was 0.000¢, 0.0% of its 5¢ a life.

## Chain bottlenecks

- `gather fiber` took 12% of all bot time (36.5 bot-hours over 13560 steps, 1028 failed)
- `gather iron_ore` took 10% of all bot time (31.1 bot-hours over 4978 steps, 178 failed)
- `make a market` took 10% of all bot time (30.7 bot-hours over 34 steps)
- clay came at 19 s a unit, 1.6× D5's 12 s
- iron_ore failed 42% of its pulls (2037 of 4800)
- stone failed 38% of its pulls (789 of 2026)
- `the road in: knap stone at the bench` failed 25% of the time
- `the road in: pull grass by hand` failed 31% of the time
- stuck 1×: the kiln waits for a plot, and the plot for the road in's deed

## Production

| Item | Gathered | Crafted | Per game hour | p10 | p50 | p90 | First at (h) | s a unit (D5) | Traded | Low | Mean price (D5) | High |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| anvil_kit | 0 | 18 | 1.8 | 48 | 55 | 61 | 1.21 |  |  |  |  |  |
| brick | 0 | 1538 | 153.5 | 52 | 61 | 68 | 0.43 |  | 613 | 1 | 2.5 (8) | 8 |
| charcoal | 0 | 1496 | 149.3 | 30 | 55 | 67 | 0.43 |  | 41 | 30 | 34.8 (5) | 40 |
| chest_kit | 0 | 17 | 1.7 | 46 | 54 | 73 | 0.25 |  |  |  |  |  |
| clay | 1720 | 0 | 171.7 | 50 | 66 | 77 | 0.23 | 19 (12) |  |  |  |  |
| fiber | 27071 | 0 | 2701.8 | 42 | 54 | 68 | 0.00 | 5 (4) |  |  |  |  |
| forge_kit | 0 | 26 | 2.6 | 52 | 57 | 61 | 0.76 |  |  |  |  |  |
| iron_axe | 0 | 11 | 1.1 | 50 | 54 | 60 | 1.47 |  |  |  |  |  |
| iron_axe_head | 0 | 11 | 1.1 | 42 | 52 | 58 | 1.44 |  |  |  |  |  |
| iron_hammer | 0 | 11 | 1.1 | 44 | 53 | 58 | 1.58 |  |  |  |  |  |
| iron_hammer_head | 0 | 11 | 1.1 | 39 | 54 | 57 | 1.54 |  |  |  |  |  |
| iron_ingot | 0 | 648 | 64.7 | 38 | 54 | 69 | 0.86 |  | 170 | 47 | 55.0 (50) | 66 |
| iron_nails | 0 | 200 | 20.0 | 45 | 58 | 67 | 0.98 |  |  |  |  |  |
| iron_ore | 3123 | 0 | 311.7 | 23 | 54 | 79 | 0.10 | 36 (90) | 380 | 11 | 24.8 (20) | 47 |
| iron_pickaxe | 0 | 14 | 1.4 | 48 | 52 | 63 | 1.35 |  |  |  |  |  |
| iron_pickaxe_head | 0 | 14 | 1.4 | 46 | 50 | 61 | 1.32 |  |  |  |  |  |
| iron_sickle | 0 | 14 | 1.4 | 47 | 54 | 58 | 1.58 |  |  |  |  |  |
| iron_sickle_blade | 0 | 15 | 1.5 | 46 | 53 | 61 | 1.57 |  |  |  |  |  |
| kiln_kit | 0 | 26 | 2.6 | 50 | 55 | 58 | 0.27 |  |  |  |  |  |
| knapped_stone | 0 | 506 | 50.5 | 43 | 53 | 63 | 0.02 |  |  |  |  |  |
| log | 7975 | 0 | 795.9 | 33 | 49 | 71 | 0.08 | 11 (32) | 4090 | 1 | 1.7 (8) | 13 |
| plank | 0 | 7572 | 755.7 | 39 | 53 | 67 | 1.17 |  | 4625 | 1 | 1.1 (5) | 6 |
| rope | 0 | 627 | 62.6 | 45 | 57 | 68 | 0.00 |  |  |  |  |  |
| sawbench_kit | 0 | 19 | 1.9 | 49 | 55 | 59 | 1.07 |  |  |  |  |  |
| sticks | 575 | 0 | 57.4 | 34 | 49 | 60 | 0.02 | 6 (12) |  |  |  |  |
| stone | 1498 | 0 | 149.5 | 34 | 50 | 69 | 0.02 | 22 (22) | 1 | 4 | 4.0 (5) | 4 |
| stone_axe | 0 | 122 | 12.2 | 48 | 54 | 59 | 0.08 |  | 25 | 43 | 48.4 (50) | 55 |
| stone_hammer | 0 | 27 | 2.7 | 47 | 53 | 61 | 0.07 |  |  |  |  |  |
| stone_pickaxe | 0 | 89 | 8.9 | 48 | 54 | 59 | 0.09 |  | 15 | 53 | 57.6 (50) | 71 |
| stone_sickle | 0 | 23 | 2.3 | 47 | 55 | 61 | 0.11 |  |  |  |  |  |
| tool_handle | 0 | 3169 | 316.3 | 42 | 53 | 62 | 0.03 |  |  |  |  |  |
| workbench_kit | 0 | 38 | 3.8 | 49 | 58 | 68 | 0.01 |  |  |  |  |  |

## Pulls that failed

| Item | Pulls | Failed | Share |
|---|---:|---:|---:|
| clay | 1136 | 0 | 0% |
| fiber | 12691 | 0 | 0% |
| iron_ore | 4800 | 2037 | 42% |
| log | 5669 | 740 | 13% |
| sticks | 271 | 0 | 0% |
| stone | 2026 | 789 | 39% |

## Tools worn out

- iron_pickaxe: 3
- iron_sickle: 2
- stone_axe: 69
- stone_pickaxe: 60
- stone_sickle: 5

## Strands

- Fiber deposited: 23616
- Deeds bought: 32 for 0 Strands
- Spent on tax: 0 Strands
- Market fees: 5575 Strands
- A deed (500, in any home zone) is 0.68 hours of grass

## The hall's board

- Commissions posted: 8, 560 Strands put down, 49 in the court's fees
- Claims: 8, promising 8 units; 0 released in good time, 0 lapsed
- Delivered: 8 units, paid 560 Strands from escrow
- Filled: 8; ran out: 0, 0 Strands refunded
- Strands through the court: 609 put in by these posters, 560 paid out to these deliverers

## Wages

- Shifts taken: 7; ended: 7 at the cap
- Units made for employers: 572, paid 2844 Strands from their escrow
- Offers of work made: 10, 7 of them worked; taken down: 7 at the cap, 2 as the money ran low, 1 as the employer signed off
- Units bought on them: 568, paid 2844 Strands, and 71 in the court's fees

## Where the time went

| Step | Bot-hours | Steps | Failed |
|---|---:|---:|---:|
| gather fiber | 36.53 | 13560 | 1028 |
| gather iron_ore | 31.12 | 4978 | 178 |
| make a market | 30.72 | 34 | 0 |
| at the hall | 25.88 | 1550 | 0 |
| gather log | 24.73 | 5894 | 259 |
| at the bazaar | 23.17 | 1386 | 0 |
| craft plank | 12.75 | 2365 | 1 |
| craft charcoal | 10.07 | 709 | 0 |
| sell log | 10.04 | 382 | 0 |
| gather stone | 9.17 | 1891 | 34 |
| gather clay | 8.94 | 1158 | 22 |
| craft iron_ingot | 6.80 | 634 | 0 |
| craft brick | 6.74 | 769 | 0 |
| craft knapped_stone | 6.00 | 458 | 0 |
| craft tool_handle | 5.83 | 1666 | 0 |
| sell plank | 4.56 | 186 | 0 |
| a detour: browse the bazaar lane | 3.87 | 18 | 0 |
| buy log | 3.58 | 153 | 0 |
| sell iron_ore | 3.51 | 135 | 1 |
| a detour: sit a while in a park nearby | 3.25 | 13 | 0 |
| a detour: read the Gazette at the hall | 2.50 | 10 | 0 |
| a detour: read the Wanted page on the hall's board | 2.32 | 10 | 0 |
| a detour: call on a neighbour | 2.15 | 10 | 0 |
| stock log | 2.02 | 76 | 2 |
| sell iron_ingot | 1.89 | 149 | 0 |
| stock plank | 1.60 | 72 | 1 |
| stock iron_ore | 1.31 | 36 | 2 |
| sell brick | 1.09 | 48 | 0 |
| craft rope | 1.00 | 572 | 0 |
| the road in: make a stone axe | 1.00 | 216 | 16 |
| gather sticks | 0.89 | 229 | 0 |
| craft stone_axe | 0.83 | 96 | 0 |
| craft kiln_kit | 0.78 | 26 | 0 |
| a detour: see a ford in the hills to the south-east | 0.75 | 3 | 0 |
| buy iron_ore | 0.65 | 32 | 0 |
| to the hall | 0.60 | 18 | 0 |
| sell stone_axe | 0.59 | 31 | 0 |
| craft stone_pickaxe | 0.58 | 89 | 0 |
| craft forge_kit | 0.57 | 26 | 0 |
| the road in: set the workbench up off any lot | 0.57 | 81 | 7 |
| to the bazaar | 0.56 | 13 | 0 |
| a detour: see a ford in the hills to the south | 0.50 | 2 | 0 |
| buy stone_pickaxe at a stall | 0.46 | 15 | 0 |
| the road in: change fiber for Strands at the Exchange | 0.45 | 42 | 2 |
| buy stone_axe | 0.44 | 21 | 0 |
| the road in: fell a tree | 0.43 | 97 | 8 |
| stock stone_axe | 0.43 | 19 | 0 |
| craft anvil_kit | 0.43 | 18 | 0 |
| place kiln | 0.40 | 25 | 0 |
| craft sawbench_kit | 0.40 | 19 | 0 |
| place forge | 0.38 | 25 | 0 |
| the road in: look at a farm by the road | 0.36 | 43 | 0 |
| collect from the market box | 0.32 | 10 | 0 |
| place sawbench | 0.31 | 19 | 0 |
| the road in: see to the market box | 0.31 | 32 | 0 |
| place anvil | 0.31 | 18 | 0 |
| craft iron_pickaxe | 0.30 | 14 | 0 |
| bid for stone | 0.30 | 16 | 0 |
| sell stone_pickaxe | 0.29 | 15 | 0 |
| work: craft plank at a sawbench | 0.28 | 160 | 0 |
| craft iron_sickle | 0.27 | 14 | 0 |
| place storage_chest | 0.26 | 15 | 0 |
| stock brick | 0.26 | 18 | 4 |
| a detour: see a ford in the hills to the south-west | 0.25 | 1 | 0 |
| a detour: see a ford in the hills to the north-east | 0.25 | 1 | 0 |
| a detour: try making iron sickle blade | 0.25 | 1 | 0 |
| a detour: try making storage chest kit | 0.25 | 1 | 0 |
| a detour: see a ford in the hills to the west | 0.25 | 1 | 0 |
| a detour: try making sawn handles | 0.25 | 1 | 0 |
| the road in: look at an urban yard by the road | 0.24 | 32 | 0 |
| the road in: set a station up at home | 0.23 | 22 | 1 |
| the road in: knap stone at the bench | 0.22 | 51 | 13 |
| craft iron_hammer | 0.22 | 11 | 0 |
| craft iron_axe | 0.22 | 11 | 0 |
| the road in: gather stone by hand | 0.20 | 33 | 7 |
| place workbench | 0.20 | 18 | 0 |
| craft iron_axe_head | 0.19 | 11 | 0 |
| hire: bring home the box | 0.18 | 9 | 0 |
| the road in: make a workbench kit | 0.18 | 70 | 9 |
| work: craft charcoal at a kiln | 0.17 | 41 | 2 |
| the road in: look at a suburban lot by the road | 0.17 | 32 | 0 |
| bid for stone_pickaxe | 0.15 | 20 | 0 |
| the road in: make a handle at the bench | 0.15 | 47 | 0 |
| craft iron_sickle_blade | 0.14 | 14 | 0 |
| craft stone_hammer | 0.14 | 27 | 0 |
| the road in: pull grass by hand | 0.14 | 16 | 5 |
| hire: craft charcoal at a kiln | 0.14 | 5 | 0 |
| buy stone_axe at a stall | 0.13 | 8 | 0 |
| the road in: claim a home with its deed | 0.13 | 32 | 0 |
| take work: craft charcoal at a kiln | 0.12 | 6 | 1 |
| craft iron_pickaxe_head | 0.12 | 14 | 0 |
| bid for stone_hammer | 0.11 | 6 | 0 |
| craft stone_sickle | 0.11 | 23 | 0 |
| deliver chest_kit | 0.11 | 5 | 0 |
| craft iron_nails | 0.10 | 20 | 0 |
| buy stone_pickaxe | 0.10 | 5 | 0 |
| craft iron_hammer_head | 0.09 | 11 | 0 |
| the road in: twist a rope | 0.08 | 38 | 2 |
| work: craft iron_ingot at a forge | 0.08 | 14 | 0 |
| the road in: look in at the Deeds Office | 0.06 | 32 | 0 |
| hire: craft rope by hand | 0.05 | 3 | 0 |
| the road in: pick up fallen branches | 0.05 | 22 | 2 |
| sell charcoal | 0.04 | 5 | 0 |
| craft workbench_kit | 0.04 | 14 | 0 |
| the road in: say a word to whoever's near | 0.04 | 32 | 0 |
| craft chest_kit | 0.04 | 16 | 0 |
| the road in: leave a sell order on the book | 0.04 | 32 | 0 |
| buy stone | 0.04 | 1 | 0 |
| bid for stone_axe | 0.03 | 7 | 0 |
| hire: craft iron_ingot at a forge | 0.03 | 1 | 0 |
| stock iron_ingot | 0.03 | 1 | 0 |
| take work: craft iron_ingot at a forge | 0.03 | 1 | 0 |
| the road in: look in at a bazaar stall | 0.03 | 30 | 0 |
| stock stone_pickaxe | 0.02 | 1 | 0 |
| deliver forge_kit | 0.02 | 1 | 0 |
| deliver kiln_kit | 0.02 | 1 | 0 |
| deliver sawbench_kit | 0.02 | 1 | 0 |
| take work: craft plank at a sawbench | 0.02 | 1 | 0 |
| hire: craft plank at a sawbench | 0.02 | 1 | 0 |
| stock stone_sickle | 0.02 | 3 | 0 |
| deposit fiber | 0.02 | 116 | 0 |
| the road in: read the Registrar's letter | 0.01 | 32 | 0 |
| the road in: give a station a job | 0.01 | 31 | 0 |
| the road in: read the Gazette | 0.01 | 26 | 0 |
| the road in: leave a buy order on the book | 0.01 | 32 | 0 |
| the road in: read someone's card | 0.01 | 24 | 2 |
| the road in: read the zone card | 0.00 | 32 | 0 |
| the road in: read what tax asks | 0.00 | 32 | 0 |
| the road in: walk a little way up the road | 0.00 | 5 | 0 |
| the road in: look in its pack | 0.00 | 32 | 0 |
| claim chest_kit | 0.00 | 5 | 0 |
| sell stone_sickle | 0.00 | 1 | 0 |
| claim kiln_kit | 0.00 | 1 | 0 |
| claim forge_kit | 0.00 | 1 | 0 |
| claim sawbench_kit | 0.00 | 1 | 0 |

## Why steps failed

| Step: reason | Times |
|---|---:|
| gather fiber: the server didn't start extracting node #: Someone else is already gathering this Wild Grass | 613 |
| gather fiber: the server didn't start extracting node #: This Wild Grass is spent; it will grow back | 415 |
| gather log: the server didn't start extracting node #: This Tree is spent; it will grow back | 156 |
| gather iron_ore: the server didn't start extracting node #: This Iron Vein is spent; it will grow back | 122 |
| gather log: the server didn't start extracting node #: Someone else is already gathering this Tree | 101 |
| gather iron_ore: the server didn't start extracting node #: Someone else is already gathering this Iron Vein | 56 |
| gather stone: the server didn't start extracting node #: Someone else is already gathering this Rock Outcrop | 23 |
| gather clay: the server didn't start extracting node #: This Clay Bank is spent; it will grow back | 20 |
| the road in: make a stone axe: the server didn't start extracting node #: Someone else is already gathering this Rock Outcrop | 12 |
| gather stone: the server didn't start extracting node #: This Rock Outcrop is spent; it will grow back | 11 |
| the road in: knap stone at the bench: the server didn't start extracting node #: Someone else is already gathering this Rock Outcrop | 9 |
| the road in: make a workbench kit: the server didn't start extracting node #: Someone else is already gathering this Wild Grass | 5 |
