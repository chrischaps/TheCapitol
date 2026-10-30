# A balance run: grants-on.toml against grants-off.toml

**A** is grants-on.toml, **B** is grants-off.toml. A had 32 citizens from game 06:00 to 18:02, 12.0 hours of the day's clock (10.0 hours of ticks at 10×), and B had 32 from 06:00 to 18:03, 12.0 hours (10.0 at 10×).

## The knobs

They differ in `deed_grant_enabled`. Every knob off its default in either:

| Knob | A | B |
|---|---:|---:|
| `deed_grant_enabled` | on (default) | off |

## The books

Both runs' books balance: every Strand and unit is accounted for.

| | A | B |
|---|---|---|
| Books | Strands conserved; units conserved | Strands conserved; units conserved |
| Deeds claimed | 32 | 25 |

## Land and homes

Homes are the owners of a lot in each home zone at the end: 32 in A and 25 in B. The zone with the most held 47% of them in A and 48% in B; an even city is a third each.

| Zone | Lots | A: held | A: granted | A: homes | A: share | B: held | B: granted | B: homes | B: share |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| rural | 912 | 15 | 15 | 15 | 47% | 12 | 0 | 12 | 48% |
| suburban | 977 | 12 | 12 | 12 | 38% | 9 | 0 | 9 | 36% |
| urban | 1016 | 5 | 5 | 5 | 16% | 4 | 0 | 4 | 16% |

| | A | B |
|---|---:|---:|
| The Exchange's fiber in, as Strands paid out | 23543 | 34246 |
| Strands paid for deeds | 0 | 12500 |
| Property tax | 0 | 0 |

## Strands by trade

The citizens' Strands went from 3200 to 17023 in A (+13823, with 2865 more in escrow at the end) and from 3200 to 15860 in B (+12660, with 3549 more in escrow at the end). Escrow is what a trade's citizens have put up in open bids, commissions and wage offers: still theirs, but outside their balances.

| Trade | Citizens | A: start → end | A: change each | A: in escrow at the end | B: start → end | B: change each | B: in escrow at the end |
|---|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | 4 | 400 → 1453 | +263 | 187 | 400 → 327 | -18 | 258 |
| builder | 3 | 300 → 114 | -62 | 338 | 300 → 347 | +16 | 247 |
| crafter-ingots | 3 | 300 → 896 | +199 | 498 | 300 → 1517 | +406 | 842 |
| crafter-planks | 3 | 300 → 281 | -6 | 300 | 300 → 1324 | +341 | 106 |
| crafter-tools | 3 | 300 → 375 | +25 | 167 | 300 → 82 | -73 | 162 |
| gatherer-fiber | 4 | 400 → 5626 | +1306 | 253 | 400 → 5756 | +1339 | 439 |
| gatherer-ore | 5 | 500 → 4634 | +827 | 469 | 500 → 4608 | +822 | 616 |
| gatherer-wood | 5 | 500 → 3212 | +542 | 361 | 500 → 1077 | +115 | 186 |
| merchant | 2 | 200 → 432 | +116 | 292 | 200 → 822 | +311 | 693 |

## The spread

The Gini of the citizens' Strands ended at 0.62 in A and 0.65 in B: 0 is everyone holding the same, 1 is one citizen holding it all.

| | A | B |
|---|---:|---:|
| Gini at the start | 0.00 | 0.00 |
| Gini at the end | 0.62 | 0.65 |
| Gini at the end, counting escrow | 0.53 | 0.57 |
| The richest tenth's share at the end | 44% | 51% |

## The money moving

Strands spent on goods, commissions, wages and rent came to 176% of the faucets' Strands in A and 109% in B. A game hour is an hour of ticks.

| | A | B |
|---|---:|---:|
| Faucets a citizen-hour | 73.4 | 106.8 |
| Goods a game hour | 3688 | 3550 |
| Commissions a game hour | 56 | 51 |
| Wages a game hour | 284 | 26 |
| Rent a game hour | 116 | 108 |
| **Spent a game hour** | 4143 | 3734 |
| Spent, of the faucets | 176% | 109% |
| Fiber trades' share of the Strands at the end | 33% | 36% |
| … counting their escrow | 30% | 32% |

## What an hour earns

Strands each job's work brought in a game hour of it, less the inputs bought for it: sales of its goods, wages and commissions delivered in it, and for fiber, deposits. Against fiber is its share of fiber's rate; ⚠ marks one under 60%, underpaid against the peg. Citizens only.

| Job | A a work-hour | A against fiber | A hours | B a work-hour | B against fiber | B hours |
|---|---:|---:|---:|---:|---:|---:|
| Fiber | 636.4 | the peg | 35.8 | 720.0 | the peg | 46.3 |
| Wood | 88.3 | 14% ⚠ | 34.1 | 136.4 | 19% ⚠ | 39.0 |
| Ore | 251.5 | 40% ⚠ | 31.9 | 259.6 | 36% ⚠ | 28.4 |
| Planks | 25.0 | 4% ⚠ | 22.0 | -3.1 | -0% ⚠ | 21.4 |
| Ingots | 172.5 | 27% ⚠ | 17.2 | -49.1 | -7% ⚠ | 18.6 |
| Tools | -85.6 | -13% ⚠ | 27.9 | -42.0 | -6% ⚠ | 28.2 |
| Building | -19.1 | -3% ⚠ | 27.5 | -49.2 | -7% ⚠ | 23.1 |
| Trading | -206.7 | -32% ⚠ | 36.4 | -257.4 | -36% ⚠ | 30.3 |

## Faucets and sinks

B's sinks took 18037 Strands, 11182 more than A's 6855; property tax took 0 in B and 0 in A. Faucets brought in 23543 in A and 34246 in B.

| Flow | Kind | A | B | B − A |
|---|---|---:|---:|---:|
| `exchange_deposit` | faucet | 23543 | 34246 | +10703 |
| `deed_purchase` | sink | 0 | -12500 | -12500 |
| `market_fee` | sink | -5575 | -4359 | +1216 |
| `bazaar_rent` | sink | -1160 | -1080 | +80 |
| `court_fee` | sink | -120 | -98 | +22 |
| **Faucets** | | 23543 | 34246 | +10703 |
| **Sinks** | | -6855 | -18037 | -11182 |
| **Net** | | +16688 | +16209 | -479 |

Sinks are negative, as the ledger files them. Transfers and escrow move Strands between players, so they're left out.

## Prices against target

Each item's mean price on the market, weighted by quantity, against its target. On average a traded item was 99% off its target in A and 30% in B.

| Item | Target | A: traded | A: mean | A: off | B: traded | B: mean | B: off |
|---|---:|---:|---:|---:|---:|---:|---:|
| brick | 8 | 613 | 2.5 | -68% | 273 | 4.3 | -46% |
| charcoal | 5 | 41 | 34.8 | +596% | 2 | 6.0 | +20% |
| iron_ingot | 50 | 170 | 55.0 | +10% | 234 | 42.5 | -15% |
| iron_ore | 20 | 380 | 24.8 | +24% | 524 | 19.9 | -1% |
| log | 8 | 4090 | 1.7 | -79% | 7776 | 1.3 | -83% |
| plank | 5 | 4625 | 1.1 | -77% | 1512 | 1.6 | -67% |
| stone | 5 | 1 | 4.0 | -20% | 1 | 5.0 | +0% |
| stone_axe | 50 | 25 | 48.4 | -3% | 15 | 49.3 | -1% |
| stone_pickaxe | 50 | 15 | 57.6 | +15% | 6 | 31.8 | -36% |

## Time by activity

What each trade's work went on, as a share of its work time, in each run (the town report says how each is counted).

| Trade | Run | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | A | 33.7 | 8% | 7% | 15% | 7% | 13% | 11% | 10% | 21% | 0% | 5% | 2% | 12% | 2% | 7.8 (7) | 9.8 (9) |
| all-rounder | B | 32.6 | 12% | 11% | 12% | 5% | 12% | 12% | 15% | 14% | 0% | 5% | 2% | 15% | 2% | 7.0 (6) | 9.2 (8) |
| builder | A | 24.2 | 6% | 19% |  | 9% | 3% | 2% | 41% | 10% | 1% | 7% | 2% | 15% | 1% | 5.7 (5) | 8.0 (7) |
| builder | B | 23.1 | 12% | 16% |  | 10% | 3% | 3% | 41% | 6% |  | 6% | 2% | 20% | 1% | 5.7 (5) | 7.3 (6) |
| crafter-ingots | A | 23.0 | 7% |  | 12% | 5% | 38% | 11% | 16% | 1% | 0% | 8% | 2% | 19% | 1% | 6.0 (5) | 8.3 (7) |
| crafter-ingots | B | 22.9 | 11% |  | 9% | 5% | 42% | 8% | 16% | 3% | 0% | 5% | 3% | 19% | 1% | 6.0 (5) | 8.7 (7) |
| crafter-planks | A | 23.0 | 3% | 11% |  | 43% | 1% | 20% | 6% | 11% | 0% | 4% | 1% | 22% | 1% | 6.0 (5) | 8.3 (7) |
| crafter-planks | B | 22.7 | 7% | 17% |  | 47% | 0% | 15% | 6% | 3% | 0% | 4% | 1% | 21% | 1% | 6.3 (6) | 8.3 (7) |
| crafter-tools | A | 23.9 | 7% | 6% | 7% | 8% | 2% | 51% | 9% | 2% | 0% | 7% | 2% | 18% | 2% | 5.3 (5) | 7.7 (7) |
| crafter-tools | B | 24.2 | 12% | 3% | 1% | 12% | 3% | 54% | 5% | 2% | 0% | 7% | 2% | 17% | 2% | 6.0 (6) | 8.7 (8) |
| gatherer-fiber | A | 32.8 | 45% | 9% | 7% |  |  |  | 0% | 29% | 0% | 7% | 3% | 14% | 1% | 3.8 (3) | 6.0 (5) |
| gatherer-fiber | B | 33.9 | 44% | 11% | 11% |  |  |  |  | 25% | 0% | 8% | 2% | 13% | 1% | 3.8 (3) | 6.2 (5) |
| gatherer-ore | A | 39.5 | 3% | 10% | 48% | 1% | 7% | 9% | 2% | 8% | 1% | 8% | 2% | 17% | 3% | 5.8 (5) | 8.2 (7) |
| gatherer-ore | B | 39.2 | 5% | 12% | 46% |  | 8% | 12% | 0% | 6% | 0% | 8% | 2% | 18% | 2% | 5.2 (4) | 7.6 (6) |
| gatherer-wood | A | 38.0 | 13% | 43% | 3% | 10% |  | 1% | 16% | 3% | 2% | 6% | 2% | 20% | 3% | 5.2 (5) | 8.2 (8) |
| gatherer-wood | B | 39.9 | 21% | 47% | 2% | 7% |  | 2% | 6% | 7% | 0% | 6% | 2% | 17% | 2% | 5.0 (3) | 7.0 (6) |
| merchant | A | 18.0 | 38% |  |  |  | 1% |  |  | 54% |  | 6% | 2% | 8% | 0% | 2.5 (2) | 4.5 (4) |
| merchant | B | 18.5 | 40% |  |  |  | 2% |  | 0% | 50% | 0% | 5% | 1% | 6% | 0% | 3.0 (2) | 5.5 (4) |
| **Every citizen** | A | 256.0 | 14% | 13% | 12% | 9% | 7% | 11% | 11% | 14% | 0% | 7% | 2% | 17% | 2% | 5.5 (2) | 7.8 (4) |
| **Every citizen** | B | 257.2 | 18% | 15% | 11% | 8% | 7% | 11% | 9% | 12% | 0% | 6% | 2% | 17% | 1% | 5.4 (2) | 7.7 (4) |

## Failed and stuck

| | A | B |
|---|---:|---:|
| Steps that failed | 1609 (5.02 a citizen-hour) | 3154 (9.83 a citizen-hour) |
| Times stuck, with no way to a goal | 1 (0.00 a citizen-hour) | 0 (0.00 a citizen-hour) |

## Callings (§10)

Each style's favourite's share of its citizens' time, the median, against §10's band: specialists 60%–80%, leaners 35%–55% of the work, and no all-rounder's biggest job over 35%. Of the jobs counts the eight, trading among them; of the making jobs counts the seven without it (#555), so a trading favourite has none.

| Style | Run | Citizens | Of the work | Of the jobs | Of the making jobs | In band |
|---|---|---:|---:|---:|---:|---:|
| specialist | A | 16 | 55% | 60% | 65% | 3 of 16 (19%) |
| specialist | B | 16 | 56% | 61% | 67% | 5 of 16 (31%) |
| leaner | A | 12 | 32% | 35% | 44% | 5 of 12 (42%) |
| leaner | B | 12 | 37% | 40% | 43% | 7 of 12 (58%) |
| all-rounder | A | 4 | 26% (biggest job) | — | — | 4 of 4 (100%) |
| all-rounder | B | 4 | 26% (biggest job) | — | — | 4 of 4 (100%) |

The fewest activities a citizen worked at in a day (§10: 3, all-rounders 5); who took a detour, and the days worked with one (70%); what came of asking the citizens near it (K3.2); and whether its card's "lately busy with" says the making job it gave most time to (80%). A card names only what's made, so trading isn't held up against it.

| | A | B |
|---|---:|---:|
| Fewest activities in a day | 4 | 4 |
| Fewest activities in a day, all-rounders | 9 | 8 |
| Citizens who took a detour | 28 of 32 (88%) | 26 of 32 (81%) |
| Days worked with a detour | 28 of 32 (88%) | 26 of 32 (81%) |
| Asked a citizen near it | 16 | 16 |
| … and traded / declined / ended / failed | 9 / 0 / 7 / 0 | 6 / 4 / 6 / 0 |
| Had someone to ask, and let them be | 63 | 18 |
| Cards read (quiet) | 32 (0) | 32 (0) |
| "Lately busy with" leads with the most time | 23 of 32 (72%) | 22 of 32 (69%) |
| … or names it among its three | 29 of 32 (91%) | 28 of 32 (88%) |

<details><summary>A: each citizen's calling</summary>

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

<details><summary>B: each citizen's calling</summary>

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

