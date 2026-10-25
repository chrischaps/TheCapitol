# A balance run: roles-today-a.toml against roles-today-b.toml

**A** is roles-today-a.toml, **B** is roles-today-b.toml. A had 20 citizens from game 00:03 to 14:06, 14.0 hours of the day's clock (11.7 hours of ticks at 10×), and B had 20 from 00:03 to 14:05, 14.0 hours (11.7 at 10×).

## The knobs

Both ran on every knob's default.

## The books

Both runs' books balance: every Strand and unit is accounted for.

| | A | B |
|---|---|---|
| Books | Strands conserved; units conserved | Strands conserved; units conserved |
| Deeds claimed | 19 | 20 |

## Strands by trade

The citizens' Strands went from 2000 to 36091 in A (+34091) and from 2000 to 35748 in B (+33748).

| Trade | Citizens | A: start → end | A: change each | B: start → end | B: change each |
|---|---:|---:|---:|---:|---:|
| builder | 3 | 300 → 53 | -82 | 300 → 121 | -60 |
| crafter (ingots) | 3 | 300 → 150 | -50 | 300 → 189 | -37 |
| crafter (planks) | 2 | 200 → 178 | -11 | 200 → 153 | -24 |
| crafter (tools) | 2 | 200 → 136 | -32 | 200 → 49 | -76 |
| gatherer (fiber) | 4 | 400 → 34933 | +8633 | 400 → 34639 | +8560 |
| gatherer (ore) | 3 | 300 → 243 | -19 | 300 → 262 | -13 |
| gatherer (wood) | 3 | 300 → 398 | +33 | 300 → 335 | +12 |

## The spread

The Gini of the citizens' Strands ended at 0.79 in A and 0.80 in B: 0 is everyone holding the same, 1 is one citizen holding it all.

| | A | B |
|---|---:|---:|
| Gini at the start | 0.00 | 0.00 |
| Gini at the end | 0.79 | 0.80 |
| The richest tenth's share at the end | 55% | 57% |

## The money moving

Strands spent on goods, commissions, wages and rent came to 2% of the faucets' Strands in A and 2% in B. A game hour is an hour of ticks.

| | A | B |
|---|---:|---:|
| Faucets a citizen-hour | 147.8 | 146.7 |
| Goods a game hour | 42 | 42 |
| Commissions a game hour | 0 | 0 |
| Wages a game hour | 0 | 0 |
| Rent a game hour | 17 | 21 |
| **Spent a game hour** | 59 | 63 |
| Spent, of the faucets | 2% | 2% |
| Fiber trades' share of the Strands at the end | 97% | 97% |

## Faucets and sinks

B's sinks took 520 Strands, 49 more than A's 471; property tax took 0 in B and 0 in A. Faucets brought in 34567 in A and 34273 in B.

| Flow | Kind | A | B | B − A |
|---|---|---:|---:|---:|
| `exchange_deposit` | faucet | 34567 | 34273 | -294 |
| `market_fee` | sink | -271 | -280 | -9 |
| `bazaar_rent` | sink | -200 | -240 | -40 |
| **Faucets** | | 34567 | 34273 | -294 |
| **Sinks** | | -471 | -520 | -49 |
| **Net** | | +34096 | +33753 | -343 |

Sinks are negative, as the ledger files them. Transfers and escrow move Strands between players, so they're left out.

## Prices against target

Each item's mean price on the market, weighted by quantity, against its target. On average a traded item was 2% off its target in A and 1% in B.

| Item | Target | A: traded | A: mean | A: off | B: traded | B: mean | B: off |
|---|---:|---:|---:|---:|---:|---:|---:|
| fiber | 1 | 4 | 1.0 | +0% | 4 | 1.0 | +0% |
| iron_ore | 20 | 6 | 18.8 | -6% | 9 | 19.2 | -4% |
| log | 8 | 45 | 8.0 | +0% | 38 | 8.0 | +0% |

## Time by activity

What each trade's work went on, as a share of its work time, in each run (the town report says how each is counted).

| Trade | Run | Work hours | Fiber | Wood | Ore | Planks | Ingots | Tools | Building | Trading | Road in | Free | Setup | Jobs a day (fewest) | Activities a day (fewest) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| builder | A | 17.9 |  |  |  |  |  |  | 99% |  | 1% | 47% | 0% | 1.0 (1) | 2.0 (2) |
| builder | B | 17.9 |  |  |  |  |  |  | 99% |  | 1% | 48% | 0% | 1.0 (1) | 2.0 (2) |
| crafter (ingots) | A | 18.7 |  |  |  |  | 97% |  |  |  | 3% | 44% | 2% | 1.0 (1) | 2.0 (2) |
| crafter (ingots) | B | 18.3 |  |  |  |  | 97% |  |  |  | 3% | 45% | 2% | 1.0 (1) | 2.0 (2) |
| crafter (planks) | A | 5.7 |  |  |  | 98% |  |  |  |  | 2% | 75% | 5% | 1.0 (1) | 2.0 (2) |
| crafter (planks) | B | 5.4 |  |  |  | 98% |  |  |  |  | 2% | 76% | 4% | 1.0 (1) | 2.0 (2) |
| crafter (tools) | A | 21.4 |  |  |  |  |  | 99% |  |  | 1% | 6% | 1% | 1.0 (1) | 2.0 (2) |
| crafter (tools) | B | 21.4 |  |  |  |  |  | 99% |  |  | 1% | 6% | 1% | 1.0 (1) | 2.0 (2) |
| gatherer (fiber) | A | 42.4 | 99% |  |  |  |  |  |  |  | 1% | 6% |  | 1.0 (1) | 2.0 (2) |
| gatherer (fiber) | B | 42.5 | 99% |  |  |  |  |  |  |  | 1% | 6% |  | 1.0 (1) | 2.0 (2) |
| gatherer (ore) | A | 26.9 | 0% |  | 93% |  |  |  |  | 3% | 4% |  | 1% | 2.0 (1) | 3.0 (2) |
| gatherer (ore) | B | 33.6 |  |  | 94% |  |  |  |  | 4% | 2% |  | 1% | 2.0 (2) | 3.0 (3) |
| gatherer (wood) | A | 28.1 | 0% | 92% |  |  |  |  |  | 7% | 1% | 5% | 2% | 2.3 (2) | 3.3 (3) |
| gatherer (wood) | B | 29.3 |  | 92% |  |  |  |  |  | 7% | 1% | 5% | 2% | 2.0 (2) | 3.0 (3) |
| **Every citizen** | A | 161.2 | 26% | 16% | 15% | 3% | 11% | 13% | 11% | 2% | 2% | 25% | 1% | 1.4 (1) | 2.4 (2) |
| **Every citizen** | B | 168.4 | 25% | 16% | 19% | 3% | 11% | 13% | 10% | 2% | 2% | 24% | 1% | 1.3 (1) | 2.3 (2) |

## Failed and stuck

| | A | B |
|---|---:|---:|
| Steps that failed | 5050 (21.59 a citizen-hour) | 2814 (12.05 a citizen-hour) |
| Times stuck, with no way to a goal | 2343 (10.02 a citizen-hour) | 2354 (10.08 a citizen-hour) |

