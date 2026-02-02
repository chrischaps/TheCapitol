# The Capitol - Game Design Document

**Working Title:** The Capitol  
**Genre:** MMO Economic Simulation  
**Target Platform:** Multi-client (Web, Mobile, Desktop, CLI)  
**Business Model:** Subscription (F2P secondary option)

---

## 1. Vision & Pillars

### One-Liner

An MMO where the economy *is* the game - players compete to build wealth through gathering, crafting, trading, and automation in a persistent shared world.

### Core Pillars

1. **Economy as Primary Gameplay** - No combat system. Gathering, crafting, logistics, and trade are the core activities, not side content.

2. **Automation as Progression** - Success is measured by the systems you build, not hours ground. A well-designed workshop running overnight is the goal state.

3. **Quality Cascade** - Every item has granular quality. Quality flows through the production chain, rewarding optimization at every step.

4. **Protocol-First Architecture** - The game world exists as an abstract simulation. Clients are interchangeable windows into that world (web, mobile, CLI, 3D).

5. **Competitive Cooperation** - Players compete on wealth leaderboards but depend on each other economically. Guilds, contracts, and specialization create interdependence.

6. **Geographic Tradeoffs** - Where you are matters. The radial world creates inherent tensions between access to markets, space for production, and proximity to resources.

---

## 2. Core Loop Breakdown

### Micro Loop (Seconds to Minutes)

- Identify a resource or task
- Engage with it using appropriate tools
- Receive output (goods, progress, information)
- Decide: use it, store it, sell it, or feed it into a process

*This is the tactile loop - the satisfaction of chopping a tree, watching ore smelt, seeing quality numbers tick up.*

### Session Loop (Minutes to Hours)

- Set a goal (fill a market order, upgrade a tool, optimize a production line)
- Gather or acquire inputs
- Process through crafting/refinement stages
- Output: improved position (wealth, infrastructure, reputation)
- Set up automation to continue progress while away

*This is the planning loop - the Factorio-brain satisfaction of designing systems that work.*

### Meta Loop (Days to Weeks)

- Accumulate wealth and infrastructure
- Expand operations (better plot, more workshops, guild membership)
- Climb leaderboards and social hierarchies
- Unlock access to higher-tier resources, recipes, markets
- Mentor/employ newer players, shaping the economy

*This is the progression loop - the rags-to-riches arc from wilderness newbie to industrial magnate.*

### Offline Continuity

Automation bridges sessions. A player designs their workshop while actively playing, then their systems continue producing during offline periods. Check-ins via mobile allow monitoring and light adjustments without requiring full engagement.

---

## 3. System Summaries

### 3.1 World System

#### Structure

Radial layout with concentric zones, divided by waterways:

```
[CAPITOL TOWER] (center)
    ↓
[Bailey]
    ↓
[Curtain Wall]
    ↓
[Moat]
    ↓
[Trade District] - Ring market, bazaar, auction house
    ↓
[Canal]
    ↓
[Guild District] - Guildhalls, guild resources
    ↓
[Canal]
    ↓
[Urban Zone] - Small private plots
    ↓
[Suburban Zone] - Medium plots
    ↓
[Rural Zone] - Large plots
    ↓
[Wilderness] - Raw resources, newbie spawn
```

The Capitol Tower is a modern structure arranged like a medieval castle, surrounded by bailey, curtain wall, and moat - but with contemporary architecture.

#### Spatial Mechanics

- Movement takes real time, constrained by player speed property
- Distance to Capitol creates natural friction for resource transport
- Travel can be optimized (roads, mounts, vehicles, canal barges?) but never eliminated
- Canals serve as zone dividers and potential transport routes

#### Plots

- Owned via deeds (bureaucratic system at Capitol Tower)
- Confer privacy and building rights
- Owners control access permissions (private, friends, guild, public)
- Can be bought, sold, traded, possibly leased

---

### 3.2 Resource System

#### Raw Resources

- Spatially bound (trees, ore veins, fertile soil, etc.)
- Finite but regenerate over time
- Quality varies by geography and randomness
- Extraction requires tools and player action (or automated systems)
- **Fiber** (from grass/vegetation) is ubiquitous and serves as currency backing

#### Regeneration

- Time-based respawn with variability
- Possible depletion mechanics for over-harvesting
- Regional tendencies (some areas better for certain resources)

#### Quality Model

- Every unit of every item has a quality value (0-100 scale)
- Stacks display average quality; individual unit quality preserved
- Quality influences: effectiveness (tools), sell price, downstream crafting results
- *See Section 6: Quality Cascade System for detailed mechanics*

---

### 3.3 Crafting System

#### Core Mechanics

- Recipes transform inputs into outputs
- Quality of output influenced by:
  - Input quality (weighted by recipe)
  - Tool/station quality and condition
  - Active buffs/debuffs (consumables)
  - Manual minigame performance (when applicable)
  - Random variance (small)
- Tools degrade with use; degradation affects output quality near end-of-life
- *See Section 6: Quality Cascade System for detailed mechanics*
- *See Section 7: Attributes System for buff/debuff mechanics*

#### Crafting Depth

- Multi-stage processing chains (ore → ingot → component → tool)
- Intermediate goods have market value
- Specialization encouraged (nobody masters everything efficiently)

#### Recipe Knowledge

- Most recipes are common knowledge, available to all players
- Advanced recipes may need to be purchased or acquired through guilds

#### Automation

- Workshops can be configured to run processes automatically
- Requires infrastructure (machines, storage, possibly fuel/power)
- Continues during offline periods (for subscribers)
- Efficiency/throughput as optimization targets

---

### 3.4 Economy System

#### Currency: The Strand

- Commodity-backed currency: 1 Strand = 1 Fiber
- Fiber gathered from grass/vegetation (~1 second per unit, anywhere)
- Non-physical currency (account balance), fully convertible at Capitol Exchange
- *See Section 8: Economy & Currency System for detailed mechanics*

#### Markets

- **Auction House** - Asynchronous buy/sell orders, centrally located
- **Bazaar** - Player-rented stalls in Trade District, present wares for purchase
- **Farm Stand** - Player-run shops on owned plots
- **P2P Trade** - Direct trades between any players

#### Sinks & Faucets

- **Faucet:** Player labor (Fiber gathering) - no NPC faucets
- **Sinks:** Bureaucratic fees (property tax, guild fees, transaction fees, licensing)
- Goal: money flows through economy into Capitol bureaucracy (sink), not recirculated
- *See Section 8 for complete fee schedules*

#### Contracts

- Player-to-player agreements for goods delivery
- Employment contracts (wages for labor/tasks)
- Guild-level contracts
- Enforcement via escrow, reputation system, and Capitol Contract Court

---

### 3.5 Social System

#### Guilds/Cooperatives

- Formal organizations with shared resources, plots, permissions
- Internal hierarchies and roles
- Guild-level projects (large workshops, trade networks)
- Guild creation/leadership requires active subscription

#### Reputation

- Tracked per-player (and per-guild)
- Influences trust, contract terms, possibly access to certain markets or districts
- Earned through completed contracts, trade volume, time in good standing

#### Employment

- Veterans can hire newer players for tasks
- Wage labor, task bounties, apprenticeships
- Employment contracts specify deliverables and compensation
- Creates onboarding path and interdependence

#### Communication

- Chat (local, guild, global)
- Mail/messaging
- Contract negotiation interface

---

### 3.6 Progression System

#### What Progresses

- **Tools** - Better tools enable better output and unlock new recipes
- **Infrastructure** - More/better workshops, storage, automation
- **Wealth** - Liquid currency and asset value
- **Plot** - Location and size upgrades
- **Reputation** - Social standing and trust
- **Knowledge** - Advanced recipes acquired through purchase or guild membership

#### No Character Stats

- Player skill is in system design and economic decisions, not leveling numbers
- Tool quality is the proxy for "character power"
- Attributes exist but are derived from gear and buffs, not innate progression
- *See Section 7: Attributes System*

#### Leaderboards

Layered system for meaningful competition at all levels:

- **Global** - Overall wealth ranking
- **Regional** - Rankings within zones (Urban, Suburban, Rural, etc.)
- **Guild** - Intra-guild and inter-guild rankings
- **Categorical** - Top crafter, top trader, by profession/specialty
- **Time-windowed** - Weekly, monthly rankings for recent performance

---

### 3.7 Automation System

#### Building Blocks

- Machines/stations that perform crafting steps
- Storage containers with input/output designation
- Explicit logistics: conveyors, tubes, pipes (Factorio-style)
- Possibly workers (NPCs or player-hired labor)

#### Constraints

- Space (plot size limits infrastructure)
- Power/fuel (resource sink, optimization target)
- Tool wear (automated processes still degrade tools)
- Throughput limits per machine

#### Offline Behavior

- Server-side automation runs on server ticks regardless of player presence (subscribers only)
- Player can check status and make adjustments via any client
- Critical failures (tool breaks, storage full) pause the line gracefully

---

## 4. Player Journey

### Phase 1: Arrival (First Session)

- Spawn in wilderness newbie zone at world's edge
- Minimal tutorial: gather basic resources, craft a simple tool
- Earn small amount of currency through menial tasks
- Goal: earn enough to acquire a deed and travel to The Capitol to claim it

### Phase 2: The Journey Inward

- Travel from wilderness toward The Capitol
- Pass through Rural → Suburban → Urban → Guild District → Trade District → Capitol Tower
- Witness the spectrum of player activity and infrastructure
- Experience the geography firsthand
- Claim deed at Capitol bureaucracy, choose plot location (first major decision)

### Phase 3: Establishment (First Week)

- Set up basic workshop on claimed plot
- Learn core loop: gather → craft → sell
- Experiment with quality optimization
- Make first market transactions
- Possibly take employment contracts from established players

### Phase 4: Growth (Weeks 2-4)

- Upgrade tools through crafting or purchase
- Expand workshop infrastructure
- Begin building automation
- Find a niche (specialize in a resource chain or trade role)
- Consider guild membership or partnerships

### Phase 5: Maturity (Month+)

- Significant automation running
- Climbing leaderboards in relevant categories
- Possibly employing newer players
- Guild involvement (leadership? specialized role?)
- Plot upgrades or relocation
- Optimizing quality cascades across complex production chains

### Phase 6: Mastery (Ongoing)

- Competing at top of leaderboards
- Influencing market dynamics
- Running significant operations (multi-plot? guild-scale?)
- Mentoring, hiring, shaping the economy
- Exploring advanced/rare recipes

---

## 5. Monetization Integration

### Subscription Model

Subscription pays for *persistence* (your systems keep working while you're away) and *full participation* (markets, guilds, land). Active gameplay is never paywalled, but building an empire that runs itself requires commitment.

### Access Matrix

| Feature | Free / Lapsed | Subscribed |
|---------|---------------|------------|
| Web / Mobile / Desktop clients | ✓ | ✓ |
| CLI client | ✗ | ✓ |
| Active play | ✓ | ✓ |
| Server-side automation | ✗ | ✓ |
| Plot ownership retained | ✓ | ✓ |
| New plot claims / expansion | ✗ | ✓ |
| Full market access | Limited | ✓ |
| Guild leadership | ✗ | ✓ |

### Design Principles

- **No pay-for-power** - Tools, resources, quality boosts are never sold
- **No exclusive content** - No subscriber-only recipes or areas
- **No convenience skips** - No instant travel, instant crafting
- **Server-side vs client-side** - Free players can run their own always-on clients/bots; subscribers get guaranteed server-side automation
- **CLI as power-user tier** - Technical users who would run persistent bots are naturally segmented into subscription

---

## 6. Quality Cascade System (Detailed)

The Quality Cascade is a core pillar of The Capitol. This section specifies how quality flows through the game's economic systems.

### 6.1 Quality Representation

#### Scale & Storage

- **Scale:** 0-100 integer per item unit
- **Storage:** Quality is stored per-unit; stacks are logical groupings that do not transform underlying data
- **Stack Display:** Shows aggregate (average) quality; individual units retain original quality when unstacked

#### Client Presentation

- Clients choose how to present quality (exact number, letter grades, stars, color coding, etc.)
- Default view is clean and simple; players can drill down to see:
  - Distribution curve of quality within a stack
  - Tools to re-stack into quality tranches (e.g., separate 90-100 from 80-89)
- Presentation consistency across clients is encouraged but not enforced by the protocol

### 6.2 Quality at Origin (Extraction)

Raw resources have a base quality determined by world generation systems (climate, geology, regional factors, randomness). This base quality acts as a **ceiling** - extraction cannot improve quality, only preserve or damage it.

```
extracted_quality = resource_base_quality - extraction_damage

extraction_damage = f(tool_quality, tool_condition, resource_difficulty, randomness)
```

#### Extraction Mechanics

- **Tool quality vs resource difficulty:** High-quality tools extract cleanly; low-quality tools risk damaging the resource
- **Failure modes:** Low-quality tools may:
  - Fail to extract entirely (no output)
  - Produce damaged output (quality penalty)
  - Both, with probability based on quality differential
- **Example:** A Q10 axe attempting to harvest lumber may fail 75% of the time; when successful, 50% chance the lumber is reduced to half its raw quality

#### World Generation Influence

- Geography, climate, and geology create regional quality tendencies
- These systems are complex and not directly exposed to players
- Players can discover patterns through data collection (inspired by A Tale in the Desert community knowledge-building)
- Seasonal shifts or depletion effects may keep discovery relevant over time

### 6.3 Quality Propagation (Crafting)

Crafting transforms inputs into outputs. Quality flows through this transformation based on recipe-defined weights.

```
output_quality = clamp(0, 100,
  Σ(input_quality × input_weight) +
  (tool_quality × tool_weight) +
  (station_quality × station_weight) +
  (buff_modifier) +
  (minigame_bonus, if manual) +
  random_variance
)

where Σ(all weights) = 1.0
```

#### Recipe Types

**Assembly Recipes** (combining finished components)
- Example: Axe = Handle + Blade
- Input weights dominate; tool/station contribution minimal
- Little skill variance - output is primarily determined by component quality
- Weights example: Handle 0.3, Blade 0.6, Tool 0.1

**Craft Recipes** (transforming raw materials with skill)
- Example: Basket = Straw
- Tool/station/skill weights higher relative to inputs
- Possible to elevate output quality above input quality
- Weights example: Straw 0.3, Loom 0.4, Minigame 0.3

**Processing Recipes** (refining materials)
- Example: Ingot = Ore + Fuel
- Primary ingredient weighted heavily; secondary inputs less so
- Tool/station quality affects efficiency and quality preservation
- Weights example: Ore 0.6, Fuel 0.1, Furnace 0.3

#### Minigame Bonus (Manual Crafting Only)

- Certain recipes support optional minigames for manual crafting
- Minigame performance adds a bonus (or penalty) to output quality
- Does not apply to automated crafting - intentional tradeoff
- Highest-quality artisan goods require manual attention
- Minigame mechanics vary by recipe (timing, puzzle, rhythm, etc.) - specifics deferred to client design

### 6.4 Material Tiers

Items of the same type but different materials are **separate item types** with independent quality scores.

- Quality represents craftsmanship/condition within a type
- Material tier provides base effectiveness
- A bronze axe and steel axe are different items, each with their own 0-100 quality

#### Effective Power Calculation

```
effective_power = base_power(material_tier) × quality_multiplier(quality)

Quality multiplier curve (example):
  Q0:   0.5×
  Q50:  1.0×
  Q100: 1.5×

Example comparison:
  Bronze axe (base 10) at Q90: 10 × 1.4 = 14 effective
  Steel axe (base 18) at Q30:  18 × 0.7 = 12.6 effective
  
  → Excellent bronze slightly outperforms poor steel
```

This creates meaningful tradeoffs and economic niches (master bronze-smiths serving budget-conscious players).

### 6.5 Tool Degradation

Tools degrade with use, affecting both their remaining lifespan and output quality.

#### Degradation Trigger

- **Per-action:** Each use decrements durability
- **Durability pool:** Determined by material tier and tool quality
- High-quality tools have more total durability AND degrade more gracefully

#### Degradation Curve: Plateau with Late Cliff

```
Durability Remaining → Quality Retention

100% → 20%:  Full quality output (plateau)
 20% →  5%:  Quality begins degrading linearly (decline)
  5% →  0%:  Severe quality penalty, breakage imminent (cliff)
```

#### Strategic Decision

Players choose when to retire tools:
- **Early retirement:** Maintains consistent output quality; higher tool turnover cost
- **Full extraction:** Maximizes uses per tool; late-stage quality suffers

### 6.6 Quality Economics (Intended Dynamics)

#### Regression Toward Mean

Long production chains naturally compress quality variance through weighted averaging. This is intentional.

#### Two-Market Emergence

**Casual Market:**
- Players ignoring quality optimization
- Produces functional, average-quality goods
- Price-competitive, high volume
- Default experience for new players

**Boutique Market:**
- Quality-conscious players optimizing entire supply chains
- Premium goods require attention at every production stage
- Rare, expensive, status items
- Advanced pursuit for experienced players

#### Design Philosophy

- Quality mechanics are present but not foregrounded for new players
- Complexity reveals itself as players seek optimization
- Both markets are valid playstyles with economic roles

### 6.7 Minimum Viability

Any item with quality > 0 is functional. There is no hard quality threshold below which items don't work.

Low-quality items have:
- Higher failure rates
- Greater chance of damaging outputs
- Faster degradation
- Lower market value

A Q10 axe *works* - it's just unreliable and produces poor results.

---

## 7. Attributes System

Attributes are numeric properties that affect gameplay calculations. They follow the "no innate character stats" pillar - players don't level up attributes directly.

### 7.1 Structure

Attributes are stored as a key-value map of numeric values:

```
attributes: {
  "speed": 1.0,
  "extraction_efficiency": 1.0,
  "crafting_precision": 1.0,
  "carry_capacity": 100,
  "focus": 1.0,
  ...
}
```

### 7.2 Attribute Sources

A player's effective attributes are calculated as:

```
effective_attribute = global_base + Σ(gear_modifiers) + Σ(buff_modifiers)
```

#### Global Base

- Default values all players start with
- Defined in game data, same for everyone
- Represents baseline human capability

#### Gear Modifiers

- Equipped items (tools, clothing, accessories) modify attributes
- Higher quality gear provides better modifiers
- Material tier affects base modifier values

#### Buff/Debuff Modifiers

- Temporary modifications from consumables
- Time-limited duration
- Can be positive or negative
- May involve tradeoffs (e.g., coffee: +focus, -steadiness)

### 7.3 Consumables

Consumables are crafted items (primarily food and drinks) that apply temporary attribute modifications.

- Created via recipes like any other crafted item
- Quality affects potency and/or duration
- Effects specified in item data: which attributes, magnitude, duration
- Can stack or conflict (rules TBD)

### 7.4 Example Attributes

| Attribute | Affects |
|-----------|---------|
| speed | Movement rate, action speed |
| extraction_efficiency | Yield when gathering resources |
| crafting_precision | Quality variance reduction |
| carry_capacity | Inventory weight limit |
| focus | Minigame performance bonus |
| steadiness | Reduces quality damage from low-tier tools |
| endurance | Slower stamina drain (if stamina system exists) |

*Full attribute list to be defined during item catalog development.*

---

## 8. Economy & Currency System (Detailed)

This section specifies the currency model, banking system, bureaucratic functions, and economic sinks that maintain a healthy player-driven economy.

### 8.1 Currency Foundation: The Strand

The Capitol uses a **commodity-backed currency** called the **Strand**, fully convertible 1:1 with Fiber.

#### The Backing Commodity: Fiber

- **Source:** Gathered from grass, reeds, or any vegetation
- **Availability:** Ubiquitous - exists everywhere vegetation grows
- **Extraction:** ~1 second of labor = 1 Fiber (no tools required)
- **Uses:** Rope, twine, paper, thatch, basket weaving, textiles, animal feed
- **Value anchor:** ~$0.01-0.10 USD equivalent per Strand

#### Why Fiber?

- **Accessible:** New players can earn currency immediately, anywhere
- **Labor-grounded:** Money represents real work performed
- **Useful:** Fiber has genuine crafting demand, creating natural sink
- **No geographic advantage:** Unlike ore, fiber grows everywhere
- **Bot-compatible:** Automated fiber farming is just another factory

### 8.2 Currency Mechanics

#### The Strand

```
1 Strand (currency) = 1 Fiber (commodity)
```

- **Non-physical:** Strands are account balances, not inventory items
- **Fully convertible:** Exchange Fiber ↔ Strands at any Capitol Exchange
- **No weight/space:** Currency doesn't burden inventory
- **Instant transfer:** Strands move between players instantly via transaction

#### Conversion Locations

| Location | Availability | Fee |
|----------|--------------|-----|
| Capitol Exchange (Trade District) | Always | 0.5% |
| Guild Banks | If guild builds one | Set by guild |
| Mobile Exchange Carts | Player-operated service | Set by operator |

#### Fiber as Physical Commodity

While Strands transfer instantly, physical Fiber still exists:

- Players gather Fiber in the world → stored in inventory (has weight)
- Deposit Fiber at Exchange → receive Strands
- Withdraw Strands at Exchange → receive Fiber (must transport it)
- Fiber used in crafting must be physical (withdrawn or gathered)

This creates a **logistics layer**: currency is instant, but the commodity backing it requires transportation.

### 8.3 Faucets & Sinks

#### The Faucet: Fiber Gathering

- **Infinite supply:** Grass regrows; Fiber can always be gathered
- **Labor-limited:** Extraction rate bounded by time, not scarcity
- **Automation scales it:** Fiber farms are valid, but still require infrastructure
- **No NPC faucets:** All currency enters through player labor

#### The Sinks: Bureaucratic Fees

Money flows through the economy and drains into The Capitol's bureaucracy. The bureaucracy is a **sink**, not a recirculating actor - fees disappear from the economy.

### 8.4 The Capitol Bureaucracy

The Capitol provides services that maintain economic infrastructure and social order. All services require fees paid in Strands.

#### Deeds Office

Issues and manages property rights.

| Service | Fee | Frequency |
|---------|-----|-----------|
| Initial deed claim (Wilderness) | 100 Strands | One-time |
| Initial deed claim (Rural) | 250 Strands | One-time |
| Initial deed claim (Suburban) | 500 Strands | One-time |
| Initial deed claim (Urban) | 1,000 Strands | One-time |
| Property tax | 2% of assessed plot value | Weekly |
| Deed transfer | 2% of sale price | Per transaction |
| Plot upgrade permit | 500-2,000 Strands | One-time |

*Property tax is the primary recurring sink, scaling with economy size.*

#### Guild Registry

Registers and maintains guild records.

| Service | Fee | Frequency |
|---------|-----|-----------|
| Guild registration | 500 Strands | One-time |
| Guild maintenance | 50 Strands + 5/member | Weekly |
| Guild hall deed (Guild District) | 5,000 Strands | One-time |
| Member roster update | 1 Strand | Per change |

*Guild costs encourage meaningful organization rather than throwaway guilds.*

#### Contract Court

Provides legal infrastructure for player agreements.

| Service | Fee | Frequency |
|---------|-----|-----------|
| Contract filing | 1% of contract value (min 5 Strands) | Per contract |
| Escrow service | 0.5% of escrow value | Per contract |
| Dispute filing | 5% of disputed value | Per case |
| Arbitration | 10% of disputed value (paid by loser) | Per ruling |

*Enforcement gives contracts teeth; fees scale with stakes.*

#### Quality Certification Bureau

Issues official quality grades for goods.

| Service | Fee | Frequency |
|---------|-----|-----------|
| Batch certification | 1 Strand per unit | Per batch |
| "Capitol Certified" seal | 10 Strands per batch | Per batch |
| Certification dispute | 50 Strands | Per case |

*Optional prestige service - "Capitol Certified" signals trustworthy quality.*

#### Banking & Exchange

Manages currency conversion and accounts.

| Service | Fee | Frequency |
|---------|-----|-----------|
| Fiber → Strand conversion | 0.5% | Per transaction |
| Strand → Fiber conversion | 0.5% | Per transaction |
| Account maintenance | Free | - |
| Large transfer (>10,000 Strands) | 0.25% | Per transaction |

*Small friction on conversion; large transfers get volume discount.*

#### Licensing Bureau

Issues permits for regulated activities.

| Service | Fee | Frequency |
|---------|-----|-----------|
| Basic trade license | Free (included with deed) | - |
| Bazaar stall rental | 20 Strands/day | Daily |
| Advanced crafting license | 100-500 Strands | Annual |
| Hazardous materials permit | 500 Strands | Annual |

*Gates certain advanced activities; creates progression milestones.*

#### Infrastructure & Transit

Maintains public infrastructure; collects usage fees.

| Service | Fee | Frequency |
|---------|-----|-----------|
| Canal crossing (pedestrian) | 1 Strand | Per crossing |
| Canal crossing (cargo) | 5 Strands + 1/100 weight | Per crossing |
| Road maintenance tax | Included in property tax | - |
| Public warehouse storage | 1 Strand per slot/day | Daily |

*Travel costs create geographic friction; rewards proximity to center.*

### 8.5 Bureaucracy Philosophy

#### Primarily Passive

The Capitol bureaucracy is an **infrastructure provider**, not an economic actor:

- Services exist; players use them when needed
- Fees are fixed and predictable (published rates)
- No NPC market participation (The Capitol doesn't buy or sell goods)
- No dynamic fee adjustment based on economic conditions

#### Light Active Elements

Limited active intervention for economic stability:

- **Infrastructure bounties:** "The Capitol will pay X Strands for 1,000 Gravel to repair roads"
- **Emergency price floors:** If essential goods (basic tools, food) crash to near-zero, Capitol offers minimum buy price
- **Public works contracts:** Occasional large-scale projects players can bid on

These interventions are rare, predictable, and transparent - safety nets, not market manipulation.

### 8.6 Economic Health Monitoring

#### Inflation Indicators

- Average prices rising across goods categories
- Currency velocity increasing (money changing hands faster)
- New player purchasing power declining

#### Inflation Response

- Increase property tax rates
- Add new fee categories
- Reduce infrastructure bounties
- Increase licensing costs

#### Deflation Indicators

- Average prices falling
- Currency hoarding (low velocity)
- Market activity declining

#### Deflation Response

- Decrease property tax rates
- Reduce transaction fees
- Increase infrastructure bounties
- Temporary fee holidays

### 8.7 New Player Economic Path

A new player's economic journey:

1. **Spawn in Wilderness** - No currency, no property
2. **Gather Fiber** - 10-15 minutes of gathering = ~500-1000 Fiber
3. **Travel to Capitol** - Experience the world, use Fiber for canal tolls
4. **Exchange Fiber for Strands** - Establish account at Capitol Exchange
5. **Claim Deed** - Spend ~100-500 Strands depending on zone choice
6. **Establish Workshop** - Begin crafting/gathering loop
7. **Earn through labor** - Take contracts, sell goods, or continue gathering

The commodity standard ensures new players can always earn currency through simple labor - no barriers to entry.

### 8.8 Currency Summary

| Aspect | Design Choice |
|--------|---------------|
| Currency name | Strand |
| Backing | 1:1 with Fiber (commodity) |
| Physicality | Non-physical (account balance) |
| Faucet | Player labor (Fiber gathering) |
| Primary sinks | Property tax, guild fees, transaction fees |
| Bureaucracy role | Passive service provider; light active bounties |
| Inflation control | Adjustable sink rates |
| New player access | Immediate - Fiber is everywhere |

---

## Appendix A: Open Design Questions

### Currency Design

*Resolved - See Section 8: Economy & Currency System*

- Commodity-backed currency (Strand = Fiber)
- Bureaucratic fee structure defined
- Sink/faucet balance approach established

### Market Mechanics

*Needs refinement*

- Auction house order matching algorithm and fee structures
- Bazaar stall mechanics (rental duration, visibility, search/discovery)
- Farm stand implementation (how do buyers find remote shops?)
- Price history and market data visibility
- Market manipulation prevention (wash trading, cornering markets)
- Quality-based pricing (how do markets handle quality variance in listings?)

### Automation & Logistics

*Needs refinement*

- Conveyor/pipe/tube mechanics and throughput rates
- Machine types and their functions
- Power/fuel system (is there one? what resources?)
- Storage container mechanics (input/output designation, filtering)
- Automation failure modes (what happens when something jams?)
- Throughput balancing and bottleneck design
- Automation unlocks (available from start, or progression-gated?)

### World & Geography

*Needs refinement*

- Zone sizing (how big is each ring? how many plots?)
- Travel times between zones
- Resource distribution by region
- Plot dimensions by zone type
- World generation parameters
- Wilderness structure (is it uniform or varied biomes?)
- Seasonal/cyclical changes to resource quality
- Canal transport mechanics (barges? capacity? speed?)

### Social & Contracts

*Needs refinement*

- Employment contract templates and terms
- Reputation calculation formula
- Reputation decay or persistence
- Guild permission systems
- Guild shared infrastructure mechanics
- Contract breach penalties
- Apprenticeship/mentorship formalization
- Communication system details (chat range, mail capacity)

### Recipes & Items

*Needs refinement*

- Recipe data structure specification
- Item catalog framework
- Material tier progression (bronze → iron → steel → ?)
- Tool categories and their purposes
- Weight/volume system for inventory
- Item decay (do non-tool items degrade?)
- Recipe discovery/acquisition mechanics for advanced recipes

### Minigames

*Needs refinement*

- Which processes have minigames vs pure automation
- Minigame design patterns (timing, puzzle, rhythm, dexterity)
- How minigame performance translates to quality bonus
- Mobile vs desktop minigame parity

### Tick System Design

*To be addressed in TDD*

- Different tick rates for different systems
- Player movement/interaction: 100-500ms
- Crafting operations: seconds
- Crop growth / regeneration: minutes
- Market order matching: seconds

---

## Appendix B: Reference Games

Design influences and lessons to draw from:

- **Minecraft** - Gathering, crafting, building loops
- **Factorio / Satisfactory** - Automation, logistics, optimization
- **A Tale in the Desert** - Social systems, player-driven economy, non-combat MMO
- **Animal Crossing** - Check-in gameplay, real-time progression
- **World of Warcraft** - Auction house, professions, guild systems
- **Incremental/Idle games** - Offline progression, prestige mechanics
- **EVE Online** - Player-driven economy, contracts, corporate warfare
- **FarmVille** - Session-based engagement, social hooks

---

*Document Version: 0.4 - Added open design questions for future refinement*  
*Last Updated: January 2026*
