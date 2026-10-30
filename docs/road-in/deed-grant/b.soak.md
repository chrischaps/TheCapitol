# The town

32 citizens (4 all-rounder, 3 builder, 3 crafter-ingots, 3 crafter-planks, 3 crafter-tools, 4 gatherer-fiber, 5 gatherer-ore, 5 gatherer-wood, 2 merchant) for 12.0 hours of the day's clock, 10.0 hours of ticks at 10× (60.2 wall minutes).

## Conservation

**Holds.**

### Strands

Every account's Strands, with escrow: 3300 at the start, 19509 at the end. That's the start, plus 0 new accounts at 100, plus 34246 in from the faucets, less 18037 gone to the sinks, plus 0 the transfers net to: 19509 expected, 0 off.

| Flow | Kind | Strands |
|---|---|---:|
| bazaar_rent | Sink | -1080 |
| court_escrow | Escrow | -1496 |
| court_fee | Sink | -98 |
| court_payout | Escrow | +765 |
| court_refund | Escrow | +675 |
| deed_purchase | Sink | -12500 |
| exchange_deposit | Faucet | +34246 |
| market_escrow | Escrow | -56854 |
| market_fee | Sink | -4359 |
| market_refund | Escrow | +17949 |
| market_sale | Escrow | +35412 |
| trade_received | Transfer | +174 |
| trade_sent | Transfer | -174 |

Escrow: 0 at the start, 3549 at the end; 0 unexplained.

### Units

| Item | Change | Explained | Drift |
|---|---:|---:|---:|
| brick | +622 | +622 | 0 |
| charcoal | +70 | +70 | 0 |
| chest_kit | +1 | +1 | 0 |
| clay | +30 | +30 | 0 |
| fiber | +2308 | +2308 | 0 |
| iron_axe | +8 | +8 | 0 |
| iron_axe_head | +1 | +1 | 0 |
| iron_hammer | +8 | +8 | 0 |
| iron_ingot | +375 | +375 | 0 |
| iron_nails | +10 | +10 | 0 |
| iron_ore | +1090 | +1090 | 0 |
| iron_pickaxe | +9 | +9 | 0 |
| iron_sickle | +8 | +8 | 0 |
| knapped_stone | +5 | +5 | 0 |
| log | +6148 | +6148 | 0 |
| plank | +4431 | +4431 | 0 |
| sticks | +21 | +21 | 0 |
| stone | +10 | +10 | 0 |
| stone_axe | +46 | +46 | 0 |
| stone_hammer | +26 | +26 | 0 |
| stone_pickaxe | +29 | +29 | 0 |
| stone_sickle | +12 | +12 | 0 |
| tool_handle | +3413 | +3413 | 0 |
| workbench_kit | +1 | +1 | 0 |

| Flow | Units |
|---|---:|
| consumed | -13436 |
| crafted | +14268 |
| deposited | -34356 |
| gathered | +52488 |
| kit_placed | -127 |
| kit_returned | +11 |
| worn_out | -166 |

## Load

| | Whole soak |
|---|---|
| Tick budget | 10.0 ms |
| Ticks | 361419 |
| Tick time, mean | 0.36 ms |
| Tick time, p50 / p95 / p99 | 0.30 ms / 0.67 ms / 1.18 ms |
| Slowest tick | 1871.5 ms |
| Ticks over budget | 129 (0.04%) |
| Worst minute, mean tick | 0.88 ms (p99 4.04 ms) |
| Most in the world | 32 |
| Sockets opened / most open | 32 / 32 |
| Sockets dropped | 0 |
| Sockets resynced | 0 |
| DB transactions a second, mean / busiest minute | 341 / 430 |
| DB pool at the end, size / idle | 5 / 4 |

Sockets that ended: 32 closed.

## Trades

| Trade | Citizens | Strands at the start | At the end | Change each | Made | Sold (Strands) | Bought (Strands) | Deeds | Stuck |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 4 | 400 | 327 | -18 | 6769 | 472 (3407) | 605 (4131) | 4 | 0 |
| builder | 3 | 300 | 347 | +16 | 5797 | 535 (1008) | 267 (1559) | 3 | 0 |
| crafter-ingots | 3 | 300 | 1517 | +406 | 5245 | 319 (5399) | 570 (4262) | 3 | 0 |
| crafter-planks | 3 | 300 | 1324 | +341 | 6493 | 1141 (1752) | 399 (1115) | 3 | 0 |
| crafter-tools | 3 | 300 | 82 | -73 | 11134 | 315 (1175) | 463 (2623) | 3 | 0 |
| gatherer-fiber | 4 | 400 | 5756 | +1339 | 10783 | 1640 (5255) | 3064 (7910) | 0 | 0 |
| gatherer-ore | 5 | 500 | 4608 | +822 | 6695 | 1595 (9620) | 987 (4093) | 4 | 0 |
| gatherer-wood | 5 | 500 | 1077 | +115 | 8561 | 2649 (3775) | 1357 (3422) | 4 | 0 |
| merchant | 2 | 200 | 822 | +311 | 5279 | 1677 (3943) | 2645 (6260) | 1 | 0 |

In trade windows: 0 trades with people, 6 between citizens (each asked by one), with 174 Strands changing hands between citizens.

## Citizens

| Citizen | Trade | Strands | Made (most) | Sold (Strands) | Bought (Strands) | Deeds | Failed steps | Stuck (why, most often) |
|---|---|---:|---|---:|---:|---:|---:|---|
| Ada Rushton | gatherer-fiber | 100 → 1293 | 2531 (2104 fiber) | 266 (1582) | 333 (2305) | 0 | 100 | 0 |
| Leonard Forester | gatherer-wood | 100 → 69 | 1431 (608 log) | 248 (516) | 36 (304) | 1 | 83 | 0 |
| Doris Miner | gatherer-ore | 100 → 1140 | 1073 (442 log) | 487 (1918) | 10 (20) | 1 | 42 | 0 |
| Phyllis Joiner | crafter-planks | 100 → 385 | 2067 (840 plank) | 386 (720) | 173 (396) | 1 | 127 | 0 |
| Harold Ironside | crafter-ingots | 100 → 501 | 2087 (1417 fiber) | 167 (1701) | 334 (1322) | 1 | 55 | 0 |
| Agnes Toller | crafter-tools | 100 → 10 | 4885 (1894 tool_handle) | 63 (223) | 81 (377) | 1 | 71 | 0 |
| Lillian Mason | builder | 100 → 18 | 1560 (688 fiber) | 139 (246) | 55 (226) | 1 | 74 | 0 |
| Dorothy Reeder | all-rounder | 100 → 101 | 1493 (684 fiber) | 182 (1623) | 95 (1328) | 1 | 44 | 0 |
| Reginald Ashby | gatherer-wood | 100 → 692 | 1324 (678 log) | 1802 (2460) | 1159 (1979) | 0 | 126 | 0 |
| Hazel Leadbetter | gatherer-ore | 100 → 2645 | 3212 (2552 fiber) | 50 (1939) | 101 (1072) | 1 | 107 | 0 |
| Albert Turner | crafter-planks | 100 → 750 | 2623 (1259 fiber) | 503 (611) | 155 (423) | 1 | 106 | 0 |
| Lionel Furness | all-rounder | 100 → 112 | 1780 (686 fiber) | 161 (212) | 31 (216) | 1 | 105 | 0 |
| Edith Wright | crafter-tools | 100 → 10 | 3164 (1542 fiber) | 236 (444) | 205 (1441) | 1 | 107 | 0 |
| Rose Thatcher | builder | 100 → 10 | 1705 (741 fiber) | 61 (282) | 95 (392) | 1 | 98 | 0 |
| Herbert Flaxman | gatherer-fiber | 100 → 304 | 1881 (1526 fiber) | 335 (1577) | 717 (2312) | 0 | 94 | 0 |
| Alice Hewitt | gatherer-wood | 100 → 11 | 2190 (914 fiber) | 161 (214) | 46 (461) | 1 | 109 | 0 |
| Mabel Delve | gatherer-ore | 100 → 116 | 886 (314 log) | 184 (1645) | 51 (795) | 1 | 39 | 0 |
| Edwin Sawyer | crafter-planks | 100 → 189 | 1803 (815 fiber) | 252 (421) | 71 (296) | 1 | 69 | 0 |
| Ruth Furness | crafter-ingots | 100 → 997 | 1853 (1273 fiber) | 100 (1704) | 122 (969) | 1 | 93 | 0 |
| Hilda Arkwright | crafter-tools | 100 → 62 | 3085 (1250 fiber) | 16 (508) | 177 (805) | 1 | 93 | 0 |
| Arthur Mason | all-rounder | 100 → 30 | 1043 (596 fiber) | 110 (1010) | 267 (928) | 1 | 98 | 0 |
| Maud Roper | gatherer-fiber | 100 → 2414 | 2977 (2883 fiber) | 304 (965) | 742 (1276) | 0 | 215 | 0 |
| Elsie Underwood | gatherer-wood | 100 → 230 | 1577 (867 log) | 359 (494) | 7 (204) | 1 | 124 | 0 |
| Sidney Miner | gatherer-ore | 100 → 83 | 965 (387 fiber) | 337 (2187) | 254 (1109) | 1 | 24 | 0 |
| Ida Ironmonger | crafter-ingots | 100 → 19 | 1305 (837 fiber) | 52 (1994) | 114 (1971) | 1 | 74 | 0 |
| Beatrice Builder | builder | 100 → 319 | 2532 (1705 fiber) | 335 (480) | 117 (941) | 1 | 178 | 0 |
| Mildred Flaxman | all-rounder | 100 → 84 | 2453 (1828 fiber) | 19 (562) | 212 (1659) | 1 | 119 | 0 |
| Ernest Ashby | gatherer-wood | 100 → 75 | 2039 (1049 fiber) | 79 (91) | 109 (474) | 1 | 97 | 0 |
| Stanley Quarry | gatherer-ore | 100 → 624 | 559 (239 iron_ore) | 537 (1931) | 571 (1097) | 0 | 31 | 0 |
| Iris Fielding | gatherer-fiber | 100 → 1745 | 3394 (3010 fiber) | 735 (1131) | 1272 (2017) | 0 | 155 | 0 |
| Bernard Spicer | merchant | 100 → 800 | 2990 (2975 fiber) | 1518 (2632) | 2069 (3679) | 0 | 129 | 0 |
| Nora Draper | merchant | 100 → 22 | 2289 (2222 fiber) | 159 (1311) | 576 (2581) | 1 | 168 | 0 |

## Time by activity

What each trade's and each citizen's work went on, as a share of its work time. Free is the share of all its time that had nothing to do in it, market hours with nothing to trade among it. Setup is the share of the work on tools, kits, stations and deeds on the way to something else. A day is the day's clock's: jobs are the eight a citizen can lean over, and activities those and the Commons, detours and the road in, for each day it worked (the mean, and the fewest).

| Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 32.6 | 12% | 11% | 12% | 5% | 12% | 12% | 15% | 14% | 0% | 5% | 2% | 15% | 2% | 7.0 (6) | 9.2 (8) |
| builder | 23.1 | 12% | 16% |  | 10% | 3% | 3% | 41% | 6% |  | 6% | 2% | 20% | 1% | 5.7 (5) | 7.3 (6) |
| crafter-ingots | 22.9 | 11% |  | 9% | 5% | 42% | 8% | 16% | 3% | 0% | 5% | 3% | 19% | 1% | 6.0 (5) | 8.7 (7) |
| crafter-planks | 22.7 | 7% | 17% |  | 47% | 0% | 15% | 6% | 3% | 0% | 4% | 1% | 21% | 1% | 6.3 (6) | 8.3 (7) |
| crafter-tools | 24.2 | 12% | 3% | 1% | 12% | 3% | 54% | 5% | 2% | 0% | 7% | 2% | 17% | 2% | 6.0 (6) | 8.7 (8) |
| gatherer-fiber | 33.9 | 44% | 11% | 11% |  |  |  |  | 25% | 0% | 8% | 2% | 13% | 1% | 3.8 (3) | 6.2 (5) |
| gatherer-ore | 39.2 | 5% | 12% | 46% |  | 8% | 12% | 0% | 6% | 0% | 8% | 2% | 18% | 2% | 5.2 (4) | 7.6 (6) |
| gatherer-wood | 39.9 | 21% | 47% | 2% | 7% |  | 2% | 6% | 7% | 0% | 6% | 2% | 17% | 2% | 5.0 (3) | 7.0 (6) |
| merchant | 18.5 | 40% |  |  |  | 2% |  | 0% | 50% | 0% | 5% | 1% | 6% | 0% | 3.0 (2) | 5.5 (4) |

| Citizen | Trade | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Ada Rushton | gatherer-fiber | 8.0 | 37% | 13% | 13% |  |  |  |  | 26% |  | 9% | 1% | 20% | 1% | 4.0 (4) | 6.0 (6) |
| Leonard Forester | gatherer-wood | 8.2 | 16% | 39% | 8% | 11% |  | 9% | 1% | 2% |  | 12% | 1% | 17% | 4% | 7.0 (7) | 9.0 (9) |
| Doris Miner | gatherer-ore | 7.7 |  | 19% | 53% |  | 12% |  | 0% |  | 0% | 13% | 2% | 19% | 3% | 4.0 (4) | 7.0 (7) |
| Phyllis Joiner | crafter-planks | 7.3 | 1% | 12% |  | 66% |  | 16% | 0% | 2% | 0% | 2% | 1% | 23% | 1% | 6.0 (6) | 9.0 (9) |
| Harold Ironside | crafter-ingots | 7.9 | 13% |  | 8% | 13% | 28% | 4% | 15% | 8% | 1% | 6% | 3% | 16% | 1% | 7.0 (7) | 10.0 (10) |
| Agnes Toller | crafter-tools | 8.6 | 5% |  | 2% | 20% |  | 59% | 1% | 3% |  | 9% | 1% | 14% | 2% | 6.0 (6) | 8.0 (8) |
| Lillian Mason | builder | 7.7 | 9% | 18% |  | 6% | 10% | 8% | 30% | 1% |  | 16% | 2% | 23% | 1% | 7.0 (7) | 9.0 (9) |
| Dorothy Reeder | all-rounder | 7.9 | 20% | 15% | 22% | 4% | 18% |  | 17% | 1% | 1% |  | 2% | 16% | 4% | 7.0 (7) | 9.0 (9) |
| Reginald Ashby | gatherer-wood | 9.1 | 17% | 54% |  |  |  |  |  | 22% | 1% | 5% | 1% | 5% | 1% | 3.0 (3) | 6.0 (6) |
| Hazel Leadbetter | gatherer-ore | 8.4 | 21% |  | 27% |  | 19% | 21% | 1% | 1% | 1% | 7% | 2% | 14% | 1% | 6.0 (6) | 9.0 (9) |
| Albert Turner | crafter-planks | 7.8 | 15% | 21% |  | 40% |  | 20% | 0% | 4% |  |  | 1% | 21% | 1% | 6.0 (6) | 7.0 (7) |
| Lionel Furness | all-rounder | 7.9 | 13% | 30% | 1% | 9% | 15% | 14% | 9% | 1% | 0% | 6% | 2% | 17% | 2% | 8.0 (8) | 11.0 (11) |
| Edith Wright | crafter-tools | 7.9 | 24% | 10% |  | 5% |  | 41% | 10% | 1% | 1% | 6% | 2% | 17% | 1% | 6.0 (6) | 9.0 (9) |
| Rose Thatcher | builder | 7.5 | 7% | 19% |  | 11% |  |  | 58% | 3% |  |  | 2% | 20% | 1% | 5.0 (5) | 6.0 (6) |
| Herbert Flaxman | gatherer-fiber | 8.5 | 40% | 10% | 18% |  |  |  |  | 25% | 1% | 4% | 2% | 10% | 2% | 4.0 (4) | 7.0 (7) |
| Alice Hewitt | gatherer-wood | 7.5 | 24% | 44% |  | 13% |  |  | 15% | 3% |  |  | 2% | 21% | 3% | 5.0 (5) | 6.0 (6) |
| Mabel Delve | gatherer-ore | 7.4 |  | 16% | 61% |  | 4% | 13% | 0% | 2% |  |  | 3% | 23% | 1% | 6.0 (6) | 7.0 (7) |
| Edwin Sawyer | crafter-planks | 7.6 | 3% | 17% |  | 38% | 1% | 10% | 16% | 3% |  | 10% | 2% | 20% | 1% | 7.0 (7) | 9.0 (9) |
| Ruth Furness | crafter-ingots | 7.5 | 15% |  | 5% |  | 37% | 19% | 14% | 0% | 1% | 6% | 2% | 20% | 2% | 6.0 (6) | 9.0 (9) |
| Hilda Arkwright | crafter-tools | 7.7 | 8% |  |  | 10% | 9% | 61% | 3% | 0% | 1% | 6% | 2% | 21% | 2% | 6.0 (6) | 9.0 (9) |
| Arthur Mason | all-rounder | 8.5 |  |  | 10% | 7% | 8% | 19% | 16% | 25% |  | 12% | 3% | 10% | 1% | 6.0 (6) | 8.0 (8) |
| Maud Roper | gatherer-fiber | 7.9 | 51% |  | 11% |  |  |  |  | 26% |  | 9% | 2% | 20% | 0% | 3.0 (3) | 5.0 (5) |
| Elsie Underwood | gatherer-wood | 7.7 | 21% | 65% |  | 1% |  |  | 7% | 1% |  | 3% | 2% | 19% | 2% | 5.0 (5) | 7.0 (7) |
| Sidney Miner | gatherer-ore | 8.1 |  | 17% | 34% |  | 2% | 17% | 1% | 11% | 0% | 15% | 3% | 14% | 2% | 6.0 (6) | 9.0 (9) |
| Ida Ironmonger | crafter-ingots | 7.5 | 5% |  | 12% |  | 60% |  | 17% | 0% |  | 2% | 3% | 20% | 1% | 5.0 (5) | 7.0 (7) |
| Beatrice Builder | builder | 7.9 | 20% | 12% |  | 12% |  |  | 37% | 14% |  | 3% | 2% | 16% | 1% | 5.0 (5) | 7.0 (7) |
| Mildred Flaxman | all-rounder | 8.3 | 15% | 0% | 14% |  | 8% | 15% | 18% | 27% | 0% |  | 2% | 15% | 1% | 7.0 (7) | 9.0 (9) |
| Ernest Ashby | gatherer-wood | 7.4 | 31% | 31% |  | 12% |  |  | 10% | 3% |  | 10% | 3% | 23% | 2% | 5.0 (5) | 7.0 (7) |
| Stanley Quarry | gatherer-ore | 7.6 |  | 10% | 60% |  |  | 6% |  | 15% |  | 7% | 3% | 20% | 2% | 4.0 (4) | 6.0 (6) |
| Iris Fielding | gatherer-fiber | 9.5 | 47% | 18% | 2% |  |  |  |  | 23% | 1% | 8% | 2% |  | 0% | 4.0 (4) | 7.0 (7) |
| Bernard Spicer | merchant | 9.9 | 41% |  |  |  |  |  |  | 53% |  | 5% | 1% |  |  | 2.0 (2) | 4.0 (4) |
| Nora Draper | merchant | 8.6 | 39% |  |  |  | 5% |  | 0% | 48% | 1% | 6% | 2% | 13% | 0% | 4.0 (4) | 7.0 (7) |

Detours: 26 of 32 citizens took one, 6.3% of the work.

## Callings (§10)

Each style's favourite's share of its citizens' time, the median, against §10's band: specialists 60%–80%, leaners 35%–55% of the work, and no all-rounder's biggest job over 35%. Of the jobs counts the eight, trading among them; of the making jobs counts the seven without it (#555), so a trading favourite has none.

| Style | Citizens | Of the work | Of the jobs | Of the making jobs | In band |
|---|---:|---:|---:|---:|---:|
| specialist | 16 | 56% | 61% | 67% | 5 of 16 (31%) |
| leaner | 12 | 37% | 40% | 43% | 7 of 12 (58%) |
| all-rounder | 4 | 26% (biggest job) | — | — | 4 of 4 (100%) |

The fewest activities a citizen worked at in a day (§10: 3, all-rounders 5); who took a detour, and the days worked with one (70%); what came of asking the citizens near it (K3.2); and whether its card's "lately busy with" says the making job it gave most time to (80%). A card names only what's made, so trading isn't held up against it.

| | The town |
|---|---:|
| Fewest activities in a day | 4 |
| Fewest activities in a day, all-rounders | 8 |
| Citizens who took a detour | 26 of 32 (81%) |
| Days worked with a detour | 26 of 32 (81%) |
| Asked a citizen near it | 16 |
| … and traded / declined / ended / failed | 6 / 4 / 6 / 0 |
| Had someone to ask, and let them be | 18 |
| Cards read (quiet) | 32 (0) |
| "Lately busy with" leads with the most time | 22 of 32 (69%) |
| … or names it among its three | 28 of 32 (88%) |

<details><summary>each citizen's calling</summary>

| Citizen | Style | Favourite | Of the work | Of the making jobs | Biggest job | In band | Fewest a day | Detour days | Most made | Lately |
|---|---|---|---:|---:|---:|---|---:|---:|---|---|
| Ada Rushton | specialist | gatherer-fiber | 37% | 59% | fiber 37% | no | 6 | 1 of 1 | fiber | wood, fiber, ore |
| Leonard Forester | leaner | gatherer-wood | 39% | 46% | wood 39% | yes | 9 | 1 of 1 | wood | wood, tools |
| Doris Miner | specialist | gatherer-ore | 53% | 63% | ore 53% | no | 7 | 1 of 1 | ore | wood, ore, ingots |
| Phyllis Joiner | specialist | crafter-planks | 66% | 69% | planks 66% | yes | 9 | 1 of 1 | planks | planks, wood |
| Harold Ironside | leaner | crafter-ingots | 28% | 34% | ingots 28% | no | 10 | 1 of 1 | ingots | ore, ingots, fiber |
| Agnes Toller | specialist | crafter-tools | 59% | 67% | tools 59% | no | 8 | 1 of 1 | tools | tools, planks |
| Lillian Mason | leaner | builder | 30% | 37% | building 30% | no | 9 | 1 of 1 | building | wood, tools |
| Dorothy Reeder | all-rounder | — | — | — | ore 22% | yes | 9 | 0 of 1 | ore | ore, ingots, wood |
| Reginald Ashby | specialist | gatherer-wood | 54% | 76% | wood 54% | no | 6 | 1 of 1 | wood | wood |
| Hazel Leadbetter | leaner | gatherer-ore | 27% | 30% | ore 27% | no | 9 | 1 of 1 | ore | ore, ingots, fiber |
| Albert Turner | leaner | crafter-planks | 40% | 41% | planks 40% | yes | 7 | 0 of 1 | planks | planks, wood, fiber |
| Lionel Furness | all-rounder | — | — | — | wood 30% | yes | 11 | 1 of 1 | wood | wood, ingots |
| Edith Wright | leaner | crafter-tools | 41% | 45% | tools 41% | yes | 9 | 1 of 1 | tools | tools, planks, wood |
| Rose Thatcher | specialist | builder | 58% | 61% | building 58% | no | 6 | 0 of 1 | building | wood, planks, building |
| Herbert Flaxman | specialist | gatherer-fiber | 40% | 59% | fiber 40% | no | 7 | 1 of 1 | fiber | ore, wood, fiber |
| Alice Hewitt | leaner | gatherer-wood | 44% | 47% | wood 44% | yes | 6 | 0 of 1 | wood | wood, planks |
| Mabel Delve | specialist | gatherer-ore | 61% | 64% | ore 61% | yes | 7 | 0 of 1 | ore | ore, wood |
| Edwin Sawyer | leaner | crafter-planks | 38% | 44% | planks 38% | yes | 9 | 1 of 1 | planks | planks, wood |
| Ruth Furness | leaner | crafter-ingots | 37% | 41% | ingots 37% | yes | 9 | 1 of 1 | ingots | ingots, ore, fiber |
| Hilda Arkwright | specialist | crafter-tools | 61% | 68% | tools 61% | yes | 9 | 1 of 1 | tools | tools |
| Arthur Mason | all-rounder | — | — | — | trading 25% | yes | 8 | 1 of 1 | tools | ore, ingots, planks |
| Maud Roper | specialist | gatherer-fiber | 51% | 83% | fiber 51% | no | 5 | 1 of 1 | fiber | fiber, ore |
| Elsie Underwood | specialist | gatherer-wood | 65% | 69% | wood 65% | yes | 7 | 1 of 1 | wood | wood |
| Sidney Miner | leaner | gatherer-ore | 34% | 48% | ore 34% | no | 9 | 1 of 1 | ore | ore, wood, tools |
| Ida Ironmonger | specialist | crafter-ingots | 60% | 63% | ingots 60% | yes | 7 | 1 of 1 | ingots | ingots, ore |
| Beatrice Builder | leaner | builder | 37% | 45% | building 37% | yes | 7 | 1 of 1 | building | wood, fiber, planks |
| Mildred Flaxman | all-rounder | — | — | — | trading 27% | yes | 9 | 0 of 1 | building | ore, fiber, ingots |
| Ernest Ashby | leaner | gatherer-wood | 31% | 37% | fiber 31% | no | 7 | 1 of 1 | fiber | wood, fiber, planks |
| Stanley Quarry | specialist | gatherer-ore | 60% | 79% | ore 60% | no | 6 | 1 of 1 | ore | ore |
| Iris Fielding | specialist | gatherer-fiber | 47% | 71% | fiber 47% | no | 7 | 1 of 1 | fiber | fiber, wood |
| Bernard Spicer | specialist | merchant | 53% | — | trading 53% | no | 4 | 1 of 1 | fiber | fiber |
| Nora Draper | specialist | merchant | 48% | — | trading 48% | no | 7 | 1 of 1 | fiber | fiber |

</details>

## How questions were settled

| Settled by | Questions | Share |
|---|---:|---:|
| rules | 65697 | 92.9% |
| rules, unsure | 5031 | 7.1% |

## What citizens said

| Found by | Lines | Share |
|---|---:|---:|
| phrasebook | 87 | 100.0% |

No model was asked for a line: every word was the phrasebook's.

Their words cost 0.000¢, 0.0000¢ a citizen. The most one citizen's life spent was 0.000¢, 0.0% of its 5¢ a life.

## Chain bottlenecks

- `gather fiber` took 21% of all bot time (63.5 bot-hours over 20026 steps, 2588 failed)
- `at the hall` took 9% of all bot time (27.4 bot-hours over 1645 steps)
- `make a market` took 9% of all bot time (27.0 bot-hours over 29 steps)
- clay came at 18 s a unit, 1.5× D5's 12 s
- iron_ore failed 45% of its pulls (1695 of 3766)
- stone failed 38% of its pulls (800 of 2052)
- `the road in: pull grass by hand` failed 43% of the time
- `stock stone_pickaxe` failed 40% of the time

## Production

| Item | Gathered | Crafted | Per game hour | p10 | p50 | p90 | First at (h) | s a unit (D5) | Traded | Low | Mean price (D5) | High |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| anvil_kit | 0 | 14 | 1.4 | 47 | 53 | 57 | 1.65 |  |  |  |  |  |
| brick | 0 | 886 | 88.4 | 49 | 60 | 68 | 1.04 |  | 273 | 2 | 4.3 (8) | 8 |
| charcoal | 0 | 1068 | 106.5 | 23 | 50 | 69 | 1.03 |  | 2 | 6 | 6.0 (5) | 6 |
| chest_kit | 0 | 8 | 0.8 | 54 | 67 | 77 | 1.59 |  |  |  |  |  |
| clay | 1066 | 0 | 106.3 | 46 | 65 | 77 | 0.14 | 18 (12) |  |  |  |  |
| fiber | 38322 | 0 | 3822.6 | 42 | 54 | 68 | 0.00 | 6 (4) |  |  |  |  |
| forge_kit | 0 | 22 | 2.2 | 52 | 56 | 62 | 1.20 |  |  |  |  |  |
| iron_axe | 0 | 8 | 0.8 | 43 | 54 | 59 | 2.38 |  |  |  |  |  |
| iron_axe_head | 0 | 9 | 0.9 | 43 | 54 | 58 | 2.34 |  |  |  |  |  |
| iron_hammer | 0 | 8 | 0.8 | 48 | 52 | 67 | 2.53 |  |  |  |  |  |
| iron_hammer_head | 0 | 8 | 0.8 | 43 | 51 | 64 | 2.48 |  |  |  |  |  |
| iron_ingot | 0 | 555 | 55.4 | 37 | 54 | 68 | 1.26 |  | 234 | 33 | 42.5 (50) | 54 |
| iron_nails | 0 | 180 | 18.0 | 42 | 57 | 69 | 1.41 |  |  |  |  |  |
| iron_ore | 2200 | 0 | 219.4 | 24 | 53 | 77 | 0.14 | 41 (90) | 524 | 11 | 19.9 (20) | 35 |
| iron_pickaxe | 0 | 9 | 0.9 | 47 | 58 | 62 | 1.77 |  |  |  |  |  |
| iron_pickaxe_head | 0 | 9 | 0.9 | 42 | 53 | 62 | 1.73 |  |  |  |  |  |
| iron_sickle | 0 | 9 | 0.9 | 47 | 56 | 63 | 2.64 |  |  |  |  |  |
| iron_sickle_blade | 0 | 9 | 0.9 | 45 | 54 | 60 | 2.60 |  |  |  |  |  |
| kiln_kit | 0 | 25 | 2.5 | 50 | 55 | 59 | 0.17 |  |  |  |  |  |
| knapped_stone | 0 | 521 | 52.0 | 43 | 54 | 63 | 0.03 |  |  |  |  |  |
| log | 8827 | 0 | 880.5 | 33 | 50 | 71 | 0.05 | 11 (32) | 7776 | 1 | 1.3 (8) | 8 |
| plank | 0 | 6231 | 621.5 | 37 | 51 | 66 | 1.57 |  | 1512 | 1 | 1.6 (5) | 4 |
| rope | 0 | 637 | 63.5 | 44 | 60 | 71 | 0.00 |  |  |  |  |  |
| sawbench_kit | 0 | 17 | 1.7 | 49 | 55 | 66 | 1.50 |  |  |  |  |  |
| sticks | 595 | 0 | 59.4 | 32 | 46 | 60 | 0.02 | 6 (12) |  |  |  |  |
| stone | 1478 | 0 | 147.4 | 33 | 51 | 70 | 0.02 | 23 (22) | 1 | 5 | 5.0 (5) | 5 |
| stone_axe | 0 | 133 | 13.3 | 49 | 55 | 62 | 0.05 |  | 15 | 48 | 49.3 (50) | 51 |
| stone_hammer | 0 | 26 | 2.6 | 47 | 52 | 61 | 0.07 |  |  |  |  |  |
| stone_pickaxe | 0 | 79 | 7.9 | 46 | 52 | 59 | 0.13 |  | 6 | 10 | 31.8 (50) | 50 |
| stone_sickle | 0 | 40 | 4.0 | 45 | 53 | 61 | 0.12 |  |  |  |  |  |
| tool_handle | 0 | 3725 | 371.6 | 40 | 53 | 62 | 0.02 |  |  |  |  |  |
| workbench_kit | 0 | 32 | 3.2 | 51 | 57 | 67 | 0.01 |  |  |  |  |  |

## Pulls that failed

| Item | Pulls | Failed | Share |
|---|---:|---:|---:|
| clay | 712 | 0 | 0% |
| fiber | 17578 | 0 | 0% |
| iron_ore | 3766 | 1695 | 45% |
| log | 6458 | 774 | 12% |
| sticks | 305 | 0 | 0% |
| stone | 2052 | 800 | 39% |

## Tools worn out

- iron_sickle: 1
- stone_axe: 87
- stone_pickaxe: 50
- stone_sickle: 28

## Strands

- Fiber deposited: 34356
- Deeds bought: 25 for 12500 Strands
- Spent on tax: 0 Strands
- Market fees: 4359 Strands
- A deed (500, in any home zone) is 0.83 hours of grass

## The hall's board

- Commissions posted: 4, 564 Strands put down, 26 in the court's fees
- Claims: 1, promising 10 units; 0 released in good time, 0 lapsed
- Delivered: 28 units, paid 508 Strands from escrow
- Filled: 3; ran out: 0, 0 Strands refunded
- Strands through the court: 590 put in by these posters, 508 paid out to these deliverers

## Wages

- Shifts taken: 3; ended: 3 at the cap
- Units made for employers: 44, paid 257 Strands from their escrow
- Offers of work made: 12, 3 of them worked; taken down: 3 at the cap, 5 as the money ran low, 4 as the employer signed off
- Units bought on them: 43, paid 257 Strands, and 72 in the court's fees

## Where the time went

| Step | Bot-hours | Steps | Failed |
|---|---:|---:|---:|
| gather fiber | 63.46 | 20026 | 2588 |
| at the hall | 27.45 | 1645 | 0 |
| make a market | 26.95 | 29 | 0 |
| gather log | 26.30 | 6719 | 297 |
| gather iron_ore | 25.13 | 3928 | 162 |
| at the bazaar | 22.25 | 1328 | 0 |
| sell log | 12.91 | 483 | 0 |
| craft plank | 10.49 | 2077 | 0 |
| gather stone | 9.24 | 1888 | 30 |
| craft charcoal | 7.60 | 512 | 0 |
| craft knapped_stone | 7.23 | 459 | 0 |
| craft tool_handle | 6.30 | 1950 | 0 |
| craft iron_ingot | 6.00 | 555 | 0 |
| gather clay | 5.44 | 723 | 11 |
| a detour: sit a while in a park nearby | 5.05 | 21 | 0 |
| a detour: browse the bazaar lane | 3.73 | 16 | 0 |
| craft brick | 3.73 | 443 | 0 |
| buy log | 2.93 | 134 | 0 |
| sell plank | 2.74 | 128 | 0 |
| a detour: read the Gazette at the hall | 2.41 | 10 | 0 |
| sell iron_ore | 2.38 | 89 | 1 |
| a detour: read the Wanted page on the hall's board | 1.67 | 8 | 0 |
| the road in: make a stone axe | 1.31 | 264 | 10 |
| stock plank | 1.26 | 64 | 1 |
| sell iron_ingot | 1.17 | 59 | 0 |
| stock log | 1.11 | 47 | 0 |
| gather sticks | 1.07 | 259 | 0 |
| a detour: try making storage chest kit | 1.00 | 4 | 0 |
| craft rope | 0.98 | 577 | 0 |
| buy iron_ore | 0.80 | 40 | 0 |
| craft stone_axe | 0.77 | 104 | 0 |
| craft stone_pickaxe | 0.65 | 79 | 0 |
| place kiln | 0.63 | 25 | 0 |
| to the hall | 0.62 | 17 | 0 |
| craft kiln_kit | 0.61 | 25 | 0 |
| craft forge_kit | 0.60 | 22 | 0 |
| stock iron_ore | 0.57 | 18 | 1 |
| a detour: call on a neighbour | 0.50 | 2 | 0 |
| the road in: change fiber for Strands at the Exchange | 0.50 | 43 | 1 |
| to the bazaar | 0.46 | 13 | 0 |
| place forge | 0.46 | 22 | 0 |
| craft sawbench_kit | 0.45 | 17 | 0 |
| craft anvil_kit | 0.41 | 14 | 0 |
| the road in: set the workbench up off any lot | 0.40 | 57 | 6 |
| sell stone_axe | 0.39 | 20 | 0 |
| the road in: fell a tree | 0.38 | 88 | 7 |
| the road in: look at a farm by the road | 0.36 | 43 | 0 |
| place sawbench | 0.34 | 17 | 0 |
| place anvil | 0.34 | 14 | 0 |
| buy stone_pickaxe at a stall | 0.31 | 12 | 0 |
| the road in: see to the market box | 0.31 | 32 | 0 |
| sell brick | 0.31 | 21 | 0 |
| the road in: gather stone by hand | 0.27 | 39 | 4 |
| the road in: look at an urban yard by the road | 0.27 | 32 | 0 |
| buy stone_axe | 0.26 | 13 | 0 |
| a detour: see a ford in the hills to the south | 0.25 | 1 | 0 |
| a detour: see a ridge top to the south-west | 0.25 | 1 | 0 |
| a detour: see a ford in the hills to the south-east | 0.25 | 1 | 0 |
| a detour: see a ford in the hills to the east | 0.25 | 1 | 0 |
| a detour: try making iron nails | 0.25 | 1 | 0 |
| a detour: try making iron axe head | 0.25 | 1 | 0 |
| a detour: call on a trading partner | 0.25 | 1 | 0 |
| stock stone_axe | 0.23 | 15 | 1 |
| the road in: knap stone at the bench | 0.23 | 51 | 7 |
| craft iron_sickle | 0.23 | 9 | 0 |
| craft iron_pickaxe | 0.23 | 9 | 0 |
| the road in: make a workbench kit | 0.23 | 83 | 6 |
| craft stone_sickle | 0.22 | 40 | 0 |
| craft iron_axe | 0.22 | 8 | 0 |
| craft iron_hammer | 0.22 | 8 | 0 |
| hire: craft rope by hand | 0.22 | 9 | 0 |
| collect from the market box | 0.20 | 6 | 0 |
| craft stone_hammer | 0.18 | 26 | 0 |
| the road in: look at a suburban lot by the road | 0.17 | 32 | 0 |
| bid for stone_pickaxe | 0.16 | 13 | 0 |
| the road in: make a handle at the bench | 0.15 | 45 | 2 |
| bid for stone | 0.15 | 7 | 0 |
| stock brick | 0.15 | 8 | 1 |
| hire: bring home the box | 0.12 | 8 | 0 |
| deliver iron_ore | 0.11 | 5 | 1 |
| the road in: pull grass by hand | 0.11 | 16 | 7 |
| work: craft charcoal at a kiln | 0.09 | 22 | 0 |
| craft iron_axe_head | 0.09 | 8 | 0 |
| buy stone_pickaxe | 0.09 | 6 | 0 |
| hire: craft charcoal at a kiln | 0.09 | 3 | 0 |
| craft iron_sickle_blade | 0.09 | 9 | 0 |
| craft iron_nails | 0.08 | 17 | 0 |
| the road in: twist a rope | 0.08 | 36 | 3 |
| craft iron_hammer_head | 0.08 | 8 | 0 |
| buy stone_axe at a stall | 0.07 | 3 | 0 |
| sell stone_pickaxe | 0.07 | 10 | 0 |
| claim a plot | 0.06 | 25 | 0 |
| the road in: say a word to whoever's near | 0.06 | 32 | 0 |
| craft iron_pickaxe_head | 0.05 | 9 | 0 |
| the road in: read someone's card | 0.05 | 25 | 3 |
| take work: craft charcoal at a kiln | 0.05 | 3 | 0 |
| the road in: pick up fallen branches | 0.04 | 21 | 0 |
| buy stone | 0.03 | 1 | 0 |
| craft workbench_kit | 0.03 | 11 | 0 |
| place storage_chest | 0.03 | 7 | 0 |
| the road in: look in at the Deeds Office | 0.03 | 32 | 0 |
| the road in: look in at a bazaar stall | 0.02 | 29 | 0 |
| the road in: leave a sell order on the book | 0.02 | 32 | 0 |
| stock stone_pickaxe | 0.02 | 5 | 2 |
| place workbench | 0.02 | 21 | 0 |
| deposit fiber | 0.02 | 146 | 0 |
| the road in: read the zone card | 0.01 | 32 | 0 |
| the road in: read the Registrar's letter | 0.01 | 32 | 0 |
| deliver log | 0.01 | 2 | 1 |
| craft chest_kit | 0.01 | 4 | 0 |
| the road in: leave a buy order on the book | 0.01 | 32 | 0 |
| the road in: read the Gazette | 0.01 | 29 | 0 |
| stock iron_ingot | 0.00 | 1 | 0 |
| the road in: give a station a job | 0.00 | 25 | 0 |
| bid for stone_axe | 0.00 | 2 | 0 |
| the road in: walk a little way up the road | 0.00 | 5 | 0 |
| the road in: look in its pack | 0.00 | 32 | 0 |
| claim iron_ore | 0.00 | 1 | 0 |
| the road in: read what tax asks | 0.00 | 25 | 0 |
| bid for stone_hammer | 0.00 | 2 | 0 |
| sell stone_sickle | 0.00 | 2 | 0 |
| stock stone_sickle | 0.00 | 1 | 1 |

## Why steps failed

| Step: reason | Times |
|---|---:|
| gather fiber: the server didn't start extracting node #: This Wild Grass is spent; it will grow back | 1462 |
| gather fiber: the server didn't start extracting node #: Someone else is already gathering this Wild Grass | 1126 |
| gather log: the server didn't start extracting node #: This Tree is spent; it will grow back | 159 |
| gather log: the server didn't start extracting node #: Someone else is already gathering this Tree | 129 |
| gather iron_ore: the server didn't start extracting node #: This Iron Vein is spent; it will grow back | 99 |
| gather iron_ore: the server didn't start extracting node #: Someone else is already gathering this Iron Vein | 63 |
| gather stone: the server didn't start extracting node #: Someone else is already gathering this Rock Outcrop | 21 |
| gather clay: the server didn't start extracting node #: This Clay Bank is spent; it will grow back | 11 |
| gather stone: the server didn't start extracting node #: This Rock Outcrop is spent; it will grow back | 9 |
| gather log: the server didn't start extracting node #: Someone else is already gathering this Old Growth | 8 |
| the road in: make a stone axe: the server didn't start extracting node #: Someone else is already gathering this Rock Outcrop | 6 |
| the road in: pull grass by hand: the server didn't start extracting node #: This Wild Grass is spent; it will grow back | 6 |
