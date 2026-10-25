# Citizens' roles today

*A report, 2026-10-25, for the callings plan (`the-capitol-callings.md`, #526). It describes the code at master `456fd87f`. The roles it describes are what the plan replaces; [roles-proposed](roles-proposed.md) is what comes after. The plan doc is the source of truth from here on.*

Every citizen is minted with one `sim::Role` for life (`flint-client/crates/capitol-core/src/sim/mod.rs:72`), written to the roster as a slug (`role = "gatherer-wood"`). The role picks its surname and leans on where it lives. After that it's the source of every work and trade menu the citizen is shown.

There are eight roles:
- seven in the swarm's `MIX`, dealt in these shares: fiber 20%, wood 15%, ore 15%, planks 10%, ingots 15%, tools 10%, builder 15%;
- the merchant, outside the mix, minted on request (`mint --role merchant`). A town usually has 2.

## What every citizen does, whatever its role

These come from the citizen's life, not its role:
- **The road in.** Every citizen walks the tutorial once:
  - gathers grass, branches, stone and a tree;
  - twists rope; makes a workbench kit, a handle, knapped stone and a stone axe;
  - deposits fiber; places one sell and one buy order; collects its box;
  - visits a stall, reads the Gazette, says a word, reads a card and a letter, and looks in at the Deeds Office;
  - claims a home, places a station and sets it working.

  It's the only time a citizen does the whole loop.
- **Prerequisites.** The planner makes whatever tools, kits and stations its goal needs. For example, a stone axe before a tree, or a workbench placed to make it. It claims a plot when a station has to stand on one.
- **When broke.** Below 10 Strands (`FLOOR`), a citizen that doesn't bank earns 100 back (`FLOAT`) by gathering fiber and depositing it (#294).
- **Its day.**
  - It keeps human hours on the real clock (a habit: dedicated, regular, casual or lapsed).
  - On the game's clock it works in daylight with a noon break, keeps market hours at the hall or the bazaar, walks home at dusk (sometimes by a bridge), and works from home at night.
- **Free time.** It rests, stands on the hall's steps, walks the bazaar lane, sits in a park or on a bridge at dusk, or minds its own stall. A small whim (±0.12 a quarter hour) varies which.
- **Exploring.** A citizen with `risk` of 0.6 or more looks for nodes in the wilds first.
- **Other people.** It answers trade requests (accept, counter, decline), asks neighbours for trades while lingering, and talks.
- **Guilds.** It joins a guild whose charter names its trade, pays its dues, and leaves when they lapse. It never founds one.
- **It never hires.** No citizen offers wages (a non-goal of the Commons plan).

Its temperament (`thrift`, `risk`, `sociability`, `patience`) weights its choices. It never chooses its work.

## The roles

Each entry gives the spirit of the role (what it was meant to be), then what it may do and what it can't.

### Fiber gatherer (`gatherer-fiber`)

*Spirit:* the town's grass-puller and roper, and its only source of new money. The humble start of every chain.
- **Works at:** more fiber, for ever.
- **Earns by:** banking fiber at the exchange once it holds about 200. This is the town's only faucet.
- **Sells:** nothing on the market or at a stall. It has no product.
- **Bids for, posts, buys from others:** nothing. Its needs are empty.
- **Claims commissions for:** fiber and rope.
- **Takes wages for:** gathering fiber; rope by hand.
- **Lot:** room for 1. It never needs one for its work.
- **Guild words:** fiber, fibre, rope, roper.
- **Surnames:** Reeder, Roper, Fielding, Hayward, Rushton, Flaxman. **Home zones:** 70% rural, 30% suburban.
- **Can't:** sell, stall, bid, post, ask anyone for a trade, or spend on anything but a deed.

### Wood gatherer (`gatherer-wood`)

*Spirit:* the woodcutter who keeps the sawyers and builders in logs.
- **Works at:** more logs, from trees and old growth.
- **Sells:** logs, at the hall (from 10 held, with a bid up) and at a stall (from 20).
- **Bids for, posts, buys from others:** nothing.
- **Claims commissions for:** logs. **Takes wages for:** gathering logs.
- **Lot:** room for 1. **Guild words:** log, wood, woodcutter, forester, lumber. **Home zones:** 70% rural, 30% suburban.
- **Can't:** bid, post, buy what it needs from anyone (it needs nothing), or craft past its own tools.

### Ore gatherer (`gatherer-ore`)

*Spirit:* the miner, who walks to the rock for iron.
- **Works at:** more iron ore.
- **Sells:** ore, at the hall and at a stall.
- **Claims commissions for:** iron ore. It's the only role that counts wild veins for a claim and walks to the ranges for well-paid ore.
- **Takes wages for:** gathering ore.
- **Lot:** room for 1. **Guild words:** ore, miner, mining. **Home zones:** 70% rural, 30% suburban.
- **Can't:** bid, post, or buy.

### Plank crafter (`crafter-planks`)

*Spirit:* the sawyer: logs in, timber out.
- **Works at:** more planks, at a sawbench on its own lot.
- **Sells:** planks, at the hall and at a stall.
- **Bids for, posts, buys from others:** logs.
- **Claims commissions for:** planks. **Takes wages at:** a sawbench.
- **Lot:** room for 1 (the sawbench). **Guild words:** plank, sawyer, timber, carpenter. **Home zones:** 30% rural, 45% suburban, 25% urban.
- **Can't:** work any other station's trade, except as a prerequisite.

### Ingot crafter (`crafter-ingots`)

*Spirit:* the smelter: ore and charcoal into iron.
- **Works at:** more iron ingots, at a forge, with a kiln for the charcoal.
- **Sells:** ingots. **Bids for, posts, buys:** iron ore and charcoal.
- **Claims commissions for:** ingots. **Takes wages at:** a forge or a kiln.
- **Lot:** room for 2. **Guild words:** ingot, smelter, smelting, foundry. **Home zones:** as the plank crafter.

### Tool crafter (`crafter-tools`)

*Spirit:* the toolmaker, who supplies everyone's axes, picks, hammers and sickles.
- **Works at:**
  - first, one of each of eight tools (the stone tier, then iron);
  - then tool handles, for ever.
- **Sells:** nothing. Its product is `None`, so the tools it makes never reach anyone.
- **Bids for, posts, buys:** stone, planks and iron ingots.
- **Claims commissions for:** tool handles. **Takes wages at:** a workbench or an anvil.
- **Lot:** room for 3. **Guild words:** tool, toolmaker, handle, smith. **Home zones:** as the plank crafter.
- **Can't:** sell or stall anything, or put anything in a trade window.

### Builder (`builder`)

*Spirit:* the one who builds a working home: every station, then bricks.
- **Works at:** the six stations in order (workbench, kiln, forge, sawbench, anvil, storage chest), then more bricks.
- **Sells:** bricks:
  - at the hall only once every station is built;
  - at a stall at any time.
- **Bids for, posts, buys:** stone, logs, iron ingots, planks and charcoal.
- **Claims commissions for:** bricks. **Takes wages at:** a kiln.
- **Lot:** room for 4, the nearest lot that takes them. **Guild words:** brick, builder, mason, bricklayer. **Home zones:** 50% rural, 50% suburban.

### Merchant (`merchant`)

*Spirit:* the trader who makes a market, keeping a bid and an ask up in a few goods so that others can buy and sell.
- **Works at:**
  - first, earning a float of about 600 Strands (scaled by patience) by banking fiber, as a fiber gatherer does;
  - then making a market at the hall in three goods dealt from its seed, in spells.
- **Sells, bids, stalls through the role's menus:** nothing. All its trading is the market it makes.
- **Claims commissions, takes wages, posts:** none.
- **Lot:** room for 1. **Guild words:** merchant, trader. **Surnames:** Chapman, Mercer, Marchant, Packman, Spicer, Draper. **Home zones:** 10% rural, 30% suburban, 60% urban.
- **Can't:** gather or craft for the market. Its trade is the book.

## At a glance

| Role | Spirit | Sells | Bids / posts for | Claims | Wage work | Room |
|---|---|---|---|---|---|---|
| Fiber gatherer | grass and rope; the faucet | – (banks) | – | fiber, rope | gather fiber, rope | 1 |
| Wood gatherer | woodcutter | logs | – | logs | gather logs | 1 |
| Ore gatherer | miner | ore | – | ore | gather ore | 1 |
| Plank crafter | sawyer | planks | logs | planks | sawbench | 1 |
| Ingot crafter | smelter | ingots | ore, charcoal | ingots | forge, kiln | 2 |
| Tool crafter | toolmaker | – | stone, planks, ingots | handles | workbench, anvil | 3 |
| Builder | homebuilder | bricks | stone, logs, ingots, planks, charcoal | bricks | kiln | 4 |
| Merchant | market maker | (its market) | (its market) | – | – | 1 |

## What follows from it

- **The money pools** (#403). Fiber is the only faucet, and its gatherers want nothing. 93% of a 20-citizen town's Strands sat with its 4 fiber gatherers after 17 game hours, and 88% at 200 citizens (C1.7b).
- **Tools never trade.** The tool crafter makes tools nobody can buy, so every citizen climbs the tool tree itself.
- **The book waits on merchants.** A town with no merchant has no book (#410), and a new town's merchants spend their first hours earning a float (#501).
- **Gatherers never buy, and sellers never diversify.** A gatherer has no needs, and a crafter sells one product. Whatever is left from a prerequisite stays in the pack.
- **Trades are read from surnames.** Citizens read each other's trade from their surnames (`persona::trade_of`), which only works because nobody ever changes trade.
- **A day of one citizen looks like a loop.** The road in, its free time and its detours to the wilds are the only variety.
