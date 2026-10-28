# A balance run: callings-a.toml against callings-b.toml

**A** is callings-a.toml, **B** is callings-b.toml. A had 20 citizens from game 03:53 to 17:56, 14.0 hours of the day's clock (11.7 hours of ticks at 10×), and B had 20 from 03:53 to 17:56, 14.0 hours (11.7 at 10×).

## The knobs

Both ran on every knob's default.

## The books

Both runs' books balance: every Strand and unit is accounted for.

| | A | B |
|---|---|---|
| Books | Strands conserved; units conserved | Strands conserved; units conserved |
| Deeds claimed | 20 | 20 |

## Strands by trade

The citizens' Strands went from 2000 to 13115 in A (+11115) and from 2000 to 15594 in B (+13594).

| Trade | Citizens | A: start → end | A: change each | B: start → end | B: change each |
|---|---:|---:|---:|---:|---:|
| all-rounder | 2 | 200 → 857 | +328 | 200 → 966 | +383 |
| builder | 3 | 300 → 487 | +62 | 300 → 185 | -38 |
| crafter-ingots | 2 | 200 → 2613 | +1206 | 200 → 3161 | +1480 |
| crafter-planks | 2 | 200 → 470 | +135 | 200 → 136 | -32 |
| crafter-tools | 2 | 200 → 21 | -90 | 200 → 102 | -49 |
| gatherer-fiber | 3 | 300 → 4086 | +1262 | 300 → 5174 | +1625 |
| gatherer-ore | 3 | 300 → 2820 | +840 | 300 → 3677 | +1126 |
| gatherer-wood | 3 | 300 → 1761 | +487 | 300 → 2193 | +631 |

## The spread

The Gini of the citizens' Strands ended at 0.52 in A and 0.59 in B: 0 is everyone holding the same, 1 is one citizen holding it all.

| | A | B |
|---|---:|---:|
| Gini at the start | 0.00 | 0.00 |
| Gini at the end | 0.52 | 0.59 |
| The richest tenth's share at the end | 28% | 37% |

## The money moving

Strands spent on goods, commissions, wages and rent came to 179% of the faucets' Strands in A and 157% in B. A game hour is an hour of ticks.

| | A | B |
|---|---:|---:|
| Faucets a citizen-hour | 71.6 | 79.7 |
| Goods a game hour | 2379 | 2214 |
| Commissions a game hour | 34 | 103 |
| Wages a game hour | 78 | 115 |
| Rent a game hour | 68 | 68 |
| **Spent a game hour** | 2560 | 2500 |
| Spent, of the faucets | 179% | 157% |
| Fiber trades' share of the Strands at the end | 31% | 33% |

## Faucets and sinks

B's sinks took 3564 Strands, 104 fewer than A's 3668; property tax took 0 in B and 0 in A. Faucets brought in 16733 in A and 18632 in B.

| Flow | Kind | A | B | B − A |
|---|---|---:|---:|---:|
| `exchange_deposit` | faucet | 16733 | 18632 | +1899 |
| `market_fee` | sink | -2742 | -2611 | +131 |
| `bazaar_rent` | sink | -800 | -800 | +0 |
| `court_fee` | sink | -126 | -153 | -27 |
| **Faucets** | | 16733 | 18632 | +1899 |
| **Sinks** | | -3668 | -3564 | +104 |
| **Net** | | +13065 | +15068 | +2003 |

Sinks are negative, as the ledger files them. Transfers and escrow move Strands between players, so they're left out.

## Prices against target

Each item's mean price on the market, weighted by quantity, against its target. On average a traded item was 39% off its target in A and 39% in B.

| Item | Target | A: traded | A: mean | A: off | B: traded | B: mean | B: off |
|---|---:|---:|---:|---:|---:|---:|---:|
| brick | 8 | 445 | 2.9 | -64% | 432 | 2.7 | -66% |
| fiber | 1 | 2 | 1.0 | +0% | 2 | 1.0 | +0% |
| iron_ingot | 50 | 164 | 45.8 | -8% | 139 | 51.4 | +3% |
| iron_ore | 20 | 721 | 6.8 | -66% | 705 | 5.8 | -71% |
| log | 8 | 2674 | 3.0 | -63% | 1785 | 5.3 | -34% |
| plank | 5 | 3528 | 1.1 | -77% | 1067 | 1.3 | -74% |
| stone | 5 | 1 | 6.0 | +20% | 0 | — | — |
| stone_axe | 50 | 8 | 55.4 | +11% | 11 | 81.6 | +63% |
| stone_pickaxe | 50 | 0 | — | — | 9 | 47.8 | -4% |

## Time by activity

What each trade's work went on, as a share of its work time, in each run (the town report says how each is counted).

| Trade | Run | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Commons | Detour | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all-rounder | A | 19.4 | 16% | 13% | 8% | 8% | 8% | 9% | 15% | 19% | 2% | 1% | 1% | 14% | 1% | 7.5 (7) | 10.0 (10) |
| all-rounder | B | 19.3 | 12% | 12% | 8% | 7% | 10% | 8% | 14% | 22% | 2% | 4% | 1% | 13% | 1% | 7.5 (7) | 10.0 (10) |
| builder | A | 26.3 | 5% | 14% | 5% | 10% | 10% | 3% | 40% | 10% | 1% | 2% | 1% | 23% | 1% | 5.7 (4) | 8.0 (6) |
| builder | B | 24.0 | 5% | 14% | 5% | 14% | 12% | 4% | 39% | 3% | 1% | 3% | 1% | 29% | 1% | 6.0 (5) | 8.3 (8) |
| crafter-ingots | A | 20.2 |  |  | 10% | 7% | 38% | 15% | 8% | 14% | 1% | 6% | 1% | 9% | 2% | 5.5 (5) | 8.5 (8) |
| crafter-ingots | B | 20.5 | 4% |  | 9% | 6% | 28% | 15% | 18% | 11% | 1% | 6% | 1% | 10% | 1% | 6.0 (5) | 9.0 (8) |
| crafter-planks | A | 18.6 | 8% | 18% |  | 43% |  | 20% | 0% | 5% | 1% | 3% | 1% | 18% | 1% | 5.5 (5) | 7.5 (7) |
| crafter-planks | B | 18.4 | 7% | 17% |  | 36% |  | 21% | 0% | 14% | 0% | 3% | 2% | 18% | 1% | 6.0 (6) | 8.0 (8) |
| crafter-tools | A | 19.9 | 7% | 3% | 9% | 11% |  | 53% | 11% | 2% | 0% | 3% | 1% | 13% | 1% | 5.5 (5) | 8.0 (8) |
| crafter-tools | B | 20.1 | 6% | 10% | 9% | 15% |  | 46% | 6% | 1% | 1% | 5% | 1% | 12% | 1% | 6.0 (6) | 9.0 (9) |
| gatherer-fiber | A | 30.8 | 46% | 10% | 7% |  | 4% |  | 0% | 24% | 3% | 5% | 1% | 9% | 1% | 5.0 (4) | 8.0 (7) |
| gatherer-fiber | B | 29.8 | 43% | 10% | 9% |  | 4% |  | 0% | 26% | 2% | 5% | 1% | 12% | 1% | 5.3 (5) | 8.3 (8) |
| gatherer-ore | A | 28.4 | 5% | 16% | 49% |  | 11% | 10% | 0% | 2% | 0% | 5% | 1% | 16% | 3% | 6.0 (5) | 8.3 (7) |
| gatherer-ore | B | 27.9 | 5% | 15% | 49% |  | 12% | 11% | 1% | 2% | 0% | 4% | 1% | 16% | 3% | 6.0 (5) | 8.7 (8) |
| gatherer-wood | A | 30.1 | 11% | 43% |  | 14% |  | 3% | 8% | 15% | 1% | 6% | 1% | 11% | 2% | 5.3 (5) | 7.7 (7) |
| gatherer-wood | B | 28.2 | 15% | 44% | 5% | 7% |  | 5% | 10% | 5% | 2% | 6% | 1% | 16% | 3% | 5.3 (5) | 8.3 (8) |
| **Every citizen** | A | 193.6 | 13% | 16% | 12% | 10% | 8% | 12% | 10% | 12% | 1% | 4% | 1% | 14% | 1% | 5.7 (4) | 8.2 (6) |
| **Every citizen** | B | 188.2 | 13% | 16% | 13% | 9% | 8% | 12% | 11% | 10% | 1% | 5% | 1% | 16% | 2% | 6.0 (5) | 8.7 (8) |

## Failed and stuck

| | A | B |
|---|---:|---:|
| Steps that failed | 843 (3.61 a citizen-hour) | 978 (4.18 a citizen-hour) |
| Times stuck, with no way to a goal | 211 (0.90 a citizen-hour) | 214 (0.92 a citizen-hour) |

