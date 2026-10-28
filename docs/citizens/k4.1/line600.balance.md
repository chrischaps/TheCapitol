# A balance run: line600-a.toml against line600-b.toml

**A** is line600-a.toml, **B** is line600-b.toml. A had 20 citizens from game 06:23 to 20:25, 14.0 hours of the day's clock (11.7 hours of ticks at 10×), and B had 20 from 06:23 to 20:26, 14.0 hours (11.7 at 10×).

## The knobs

Both ran on every knob's default.

## The books

Both runs' books balance: every Strand and unit is accounted for.

| | A | B |
|---|---|---|
| Books | Strands conserved; units conserved | Strands conserved; units conserved |
| Deeds claimed | 20 | 20 |

## Strands by trade

The citizens' Strands went from 2000 to 14155 in A (+12155) and from 2000 to 12282 in B (+10282).

| Trade | Citizens | A: start → end | A: change each | B: start → end | B: change each |
|---|---:|---:|---:|---:|---:|
| all-rounder | 2 | 200 → 1011 | +406 | 200 → 1803 | +802 |
| builder | 3 | 300 → 339 | +13 | 300 → 167 | -44 |
| crafter-ingots | 2 | 200 → 4373 | +2086 | 200 → 3304 | +1552 |
| crafter-planks | 2 | 200 → 353 | +76 | 200 → 235 | +18 |
| crafter-tools | 2 | 200 → 25 | -88 | 200 → 22 | -89 |
| gatherer-fiber | 3 | 300 → 4549 | +1416 | 300 → 2728 | +809 |
| gatherer-ore | 3 | 300 → 2463 | +721 | 300 → 2556 | +752 |
| gatherer-wood | 3 | 300 → 1042 | +247 | 300 → 1467 | +389 |

## The spread

The Gini of the citizens' Strands ended at 0.57 in A and 0.58 in B: 0 is everyone holding the same, 1 is one citizen holding it all.

| | A | B |
|---|---:|---:|
| Gini at the start | 0.00 | 0.00 |
| Gini at the end | 0.57 | 0.58 |
| The richest tenth's share at the end | 35% | 32% |

## The money moving

Strands spent on goods, commissions, wages and rent came to 118% of the faucets' Strands in A and 195% in B. A game hour is an hour of ticks.

| | A | B |
|---|---:|---:|
| Faucets a citizen-hour | 71.5 | 66.7 |
| Goods a game hour | 1497 | 2317 |
| Commissions a game hour | 27 | 43 |
| Wages a game hour | 96 | 188 |
| Rent a game hour | 65 | 58 |
| **Spent a game hour** | 1685 | 2606 |
| Spent, of the faucets | 118% | 195% |
| Fiber trades' share of the Strands at the end | 32% | 22% |

## Faucets and sinks

B's sinks took 3822 Strands, 609 more than A's 3213; property tax took 0 in B and 0 in A. Faucets brought in 16699 in A and 15596 in B.

| Flow | Kind | A | B | B − A |
|---|---|---:|---:|---:|
| `exchange_deposit` | faucet | 16699 | 15596 | -1103 |
| `market_fee` | sink | -2384 | -3027 | -643 |
| `bazaar_rent` | sink | -760 | -680 | +80 |
| `court_fee` | sink | -69 | -115 | -46 |
| **Faucets** | | 16699 | 15596 | -1103 |
| **Sinks** | | -3213 | -3822 | -609 |
| **Net** | | +13486 | +11774 | -1712 |

Sinks are negative, as the ledger files them. Transfers and escrow move Strands between players, so they're left out.

## Prices against target

Each item's mean price on the market, weighted by quantity, against its target. On average a traded item was 53% off its target in A and 51% in B.

| Item | Target | A: traded | A: mean | A: off | B: traded | B: mean | B: off |
|---|---:|---:|---:|---:|---:|---:|---:|
| brick | 8 | 323 | 2.1 | -73% | 572 | 1.3 | -83% |
| iron_ingot | 50 | 101 | 52.3 | +5% | 178 | 46.5 | -7% |
| iron_ore | 20 | 639 | 3.5 | -82% | 856 | 3.0 | -85% |
| log | 8 | 4033 | 1.4 | -83% | 11030 | 1.1 | -86% |
| plank | 5 | 2734 | 1.1 | -78% | 2364 | 1.2 | -76% |
| stone_axe | 50 | 11 | 55.3 | +11% | 6 | 54.3 | +9% |
| stone_pickaxe | 50 | 2 | 68.5 | +37% | 2 | 56.5 | +13% |

## Time by activity

What each trade's work went on, as a share of its work time, in each run (the town report says how each is counted).

| Trade | Run | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | A | 19.1 | 13% | 12% | 11% | 13% | 6% | 4% | 9% | 25% | 1% | 5% | 2% | 14% | 2% | 7.0 (7) | 10.0 (10) |
| all-rounder | B | 19.1 | 17% | 11% | 12% | 6% |  | 12% | 11% | 22% | 3% | 5% | 1% | 14% | 2% | 7.0 (7) | 10.0 (10) |
| builder | A | 27.4 | 3% | 14% | 2% | 14% | 3% | 5% | 48% | 2% | 1% | 6% | 2% | 19% | 1% | 6.0 (5) | 8.7 (8) |
| builder | B | 27.5 | 1% | 16% | 2% | 14% | 5% | 5% | 47% | 1% | 0% | 6% | 2% | 19% | 1% | 6.0 (5) | 8.7 (8) |
| crafter-ingots | A | 19.8 | 16% |  | 8% | 3% | 28% | 7% | 15% | 18% | 0% | 4% | 2% | 11% | 1% | 6.0 (5) | 8.0 (7) |
| crafter-ingots | B | 20.6 | 4% |  | 11% | 3% | 34% | 7% | 15% | 15% | 5% | 5% | 2% | 7% | 1% | 6.0 (5) | 9.0 (8) |
| crafter-planks | A | 18.4 | 6% | 10% |  | 50% |  | 19% | 1% | 11% | 0% | 3% | 2% | 18% | 1% | 5.5 (5) | 7.5 (7) |
| crafter-planks | B | 18.4 | 5% | 9% |  | 49% |  | 19% | 1% | 12% | 1% | 3% | 2% | 18% | 1% | 5.5 (5) | 7.5 (7) |
| crafter-tools | A | 19.8 | 5% | 5% | 9% | 10% |  | 46% | 10% | 2% | 0% | 10% | 2% | 13% | 1% | 5.5 (5) | 8.0 (7) |
| crafter-tools | B | 19.7 | 5% | 8% | 9% | 11% |  | 46% | 6% | 1% | 1% | 10% | 2% | 14% | 1% | 5.5 (5) | 8.0 (7) |
| gatherer-fiber | A | 30.0 | 39% | 10% | 8% |  | 1% | 3% | 0% | 32% | 0% | 3% | 2% | 12% | 1% | 5.7 (5) | 8.0 (7) |
| gatherer-fiber | B | 29.0 | 34% | 8% | 9% |  |  |  | 4% | 37% | 0% | 5% | 2% | 15% | 1% | 4.7 (4) | 7.3 (6) |
| gatherer-ore | A | 27.8 | 6% | 16% | 41% |  | 12% | 11% | 0% | 4% | 3% | 5% | 2% | 17% | 2% | 6.3 (6) | 8.3 (7) |
| gatherer-ore | B | 28.3 | 6% | 14% | 45% |  | 12% | 9% | 0% | 3% | 1% | 7% | 2% | 16% | 3% | 6.3 (6) | 8.7 (8) |
| gatherer-wood | A | 28.1 | 11% | 42% | 3% | 15% |  | 4% | 12% | 6% | 1% | 5% | 1% | 17% | 3% | 6.0 (5) | 8.7 (7) |
| gatherer-wood | B | 28.6 | 8% | 40% | 4% | 9% |  | 0% | 12% | 16% | 1% | 9% | 1% | 17% | 2% | 5.7 (5) | 8.3 (8) |
| **Every citizen** | A | 190.5 | 13% | 15% | 11% | 12% | 6% | 11% | 12% | 12% | 1% | 5% | 2% | 15% | 2% | 6.0 (5) | 8.4 (7) |
| **Every citizen** | B | 191.3 | 11% | 15% | 12% | 10% | 6% | 11% | 12% | 14% | 1% | 6% | 2% | 15% | 1% | 5.8 (4) | 8.4 (6) |

## Failed and stuck

| | A | B |
|---|---:|---:|
| Steps that failed | 1069 (4.58 a citizen-hour) | 795 (3.40 a citizen-hour) |
| Times stuck, with no way to a goal | 0 (0.00 a citizen-hour) | 0 (0.00 a citizen-hour) |

