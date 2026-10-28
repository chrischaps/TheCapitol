# The Capitol: Callings

*2026-10-25. Plan issue #526. Work packages #527 to #543. Issues are labelled `callings` (and `citizens`), with phase labels `cg:K0-groundwork` to `cg:K5-niche`. Two reports go with it: [the roles as they are](docs/citizens/roles-today.md) and [the callings proposed](docs/citizens/roles-proposed.md).*

## 1. Vision

A citizen today is one job for life. It's minted a fiber gatherer, a sawyer or a builder. Every menu it's shown, from what to work at to what to sell, bid for, claim or join, comes from that one `Role`. A fiber gatherer pulls grass and banks it, and nothing else. It has nothing to want, so it spends nothing, and the town's only faucet pools in its pockets: 93% of a town's Strands in #403, 88% at 200 citizens in C1.7b.

Nobody plays like that. A person logs on and does a bit of everything at first: the road in teaches the whole loop, and the GDD's first week is "gather → craft → sell" (`the-capitol-gdd.md:346`). Over weeks 2 to 4 they **find a niche** (`:356`), a resource chain or a way of trading they come back to. Even then they still go and look at a lake, try a recipe they've never made, sell what they had spare and buy rather than climb the tool tree. Specialisation is "encouraged (nobody masters everything efficiently)" (`:190`). It's never enforced.

**Callings** give citizens that shape:
- **a calling:** a favourite activity, and how strongly they keep to it;
- **stints:** a while at one thing, then another, chosen as a person chooses their evening;
- **detours:** the unrelated things people do;
- **all-rounders:** who have no favourite at all.

The question this plan answers: **can the town's people be read as people?** You'd watch one for a game day and see a life, not a loop. The economy's question goes with it: when everyone has wants, does the money move?

## 2. Goals and non-goals

**Goals**
- Every citizen has a calling: a favourite (or none), a focus, leanings over the other activities, and a taste for detours. The default mix is 40% specialists, 40% leaners and 20% all-rounders.
- Citizens work in stints. They choose the next one from their calling, what they have, what pays and what they've done lately.
- They do the game's other core activities as people do:
  - take commissions and wages in any trade they're willing to work;
  - sell what they have spare;
  - buy rather than climb;
  - trade once they're rich enough;
  - post commissions for their homes;
  - hire.
- They take detours in work hours: an untried recipe, a lake, the bazaar lane, the Gazette, a neighbour.
- What anyone has lately done is on their card, for people and citizens alike. Citizens read the card, not a surname.
- The money moves (§10): what pooled with the fiber gatherers circulates.
- Later, leanings drift as a citizen finds its niche.

**Non-goals for this plan**
- New activities. Callings range over what the game has. Fishing, farming crops and the like are other plans.
- A new faucet or a grant (citizens D2). Fiber stays the only way Strands enter.
- Changing the `capitol-bot --sim` swarm. It has no persona and stays at one job, so E2.7's numbers still compare.
- Renaming anyone. Surnames stay as names.
- LLM decisions. The stint question goes through the cascade like any other. Settled: `rules → jev@0.9` (citizens D4).

## 3. Where things stand (audit, 2026-10-25)

`sim::Role` (`flint-client/crates/capitol-core/src/sim/mod.rs:72`) has eight variants: `Gatherer(Fiber|Wood|Ore)`, `Crafter(Planks|Ingots|Tools)`, `Builder`, and `Merchant` (outside `MIX`). `MIX` (`:100`) deals the first seven 20/15/15/10/15/10/15. The roster keeps one `role` slug per citizen (`citizen/town.rs:296-395`). The full picture of what each role can and can't do is in [roles-today](docs/citizens/roles-today.md).

**What the role decides.** Every menu is a `Role` method:

| Method | What it decides |
|---|---|
| `work`/`goals` (`:257, :276`) | what it works toward |
| `product` (`:289`) | what it sells |
| `banks` (`:317`) | whether it deposits fiber |
| `needs` (`:413`) | its bids, its posts and its `BuyFrom` |
| `answers` (`:475`) | the commissions it claims |
| `crafts_at`/`does` (`:554, :568`) | the wage work it takes |
| `room_wanted` (`:306`) | how big a lot it buys |
| `trade_words` (`:587`) | which guild charters suit it |

**What the role doesn't decide.**
- The planner (`sim/plan.rs`) and `RulesDecider` (`citizen/decide/rules.rs`) never read it.
- On the Work menu the first goal scores 0.9 and each later one 0.15 less (`GOAL_STEP`), so the first goal always wins.

**Variety today.**
- Everyone walks the road in, which is the whole loop once.
- The planner makes whatever tools and stations a goal needs.
- Everyone keeps a day and a habit, goes out to the wilds by `risk`, and spends free time at the hall, the bazaar lane, a park or a bridge, with a whim of ±0.12 a quarter hour.
- Everyone earns its float by fiber when broke (#294).
- Nobody hires. The Commons plan made it a non-goal; this plan takes it up (K2.4, built: see its row).

**Where else the role is read.**
- **The run:** `sim/run.rs`:
  - assign at `:126`; the merchant at `:253`; errands at `:584`; `choose_trade` at `:906-933`; `Who.role` at `:998`;
  - `still_wanted`/`repack` at `:1199-1264`; `claim_near` at `:1691`.
- **The run's parts:** `sim/run/`:
  - `board.rs`: `ground` sends only ore gatherers to the wilds for a commission (`:75`); `answers` (`:302`); posts (`:451`);
  - `wages.rs:205`, `people.rs`, `road_in.rs:129, 145, 626`, `guilds.rs:272`;
  - `sim/guild.rs:74` `names_trade`, `sim/wage.rs:104` `earns`.
- **The persona:**
  - `citizen/persona.rs` sets home-zone shares (`:125-130`) and surnames (`:278-310`) by role;
  - `trade_of` (`:294`) reads a surname back as a trade, which is how `meet` picks whom to ask (`citizen/meet.rs:632-711`). *Retired in K3.2: `meet` reads cards.*
- **The decider:**
  - `Who.role` reaches only the remote layers' prose (`decide/remote.rs:163, 196, 237`);
  - Jev's corpus loops over `MIX` (`decide/corpus.rs:46`);
  - the log header carries the role (`decide/log.rs:52`).
- **Reports and the wire:**
  - `sim/report.rs` (the role tally), `citizen/report.rs` (the Trades table), `citizen/balance.rs` (Strands by trade);
  - `protocol/src/metrics.rs:596` `CitizenReport.role` and `:319-369` `Distribution.roles`;
  - `backend/src/engine/town.rs:139` `Town::roles`, and the console's "Wealth by trade" and Trade column (`backend/static/console.js:1220-1270, 2123-2591`).
- **Two string forms:** the slug (`gatherer-wood`) and Display (`gatherer (wood)`) both travel (`sim/mod.rs:601, 658`).
- **Not read anywhere else.** The server's game logic and the Flint client never see a citizen's role. `PlayerCard` has only `citizen: bool`.

**The record a card could read.** `production_tally` keeps one row per player, day and item (`backend/migrations/20261115_001_gazette.sql:30`). The Gazette's Busy Hands is built from it.

### The baseline (K0.1, #527)

The "before" for §10, measured on master as it stood plus K0.1's reporting alone, so the citizens are one role for life. #403's recipe, run through `scripts/balance.ps1` as two towns at once with both presets empty (every knob on its default). Each town had 20 of the sim's mix (4 fiber, 3 wood, 3 ore, 2 planks, 3 ingots, 2 tools, 3 builders) and no merchants, fresh, `--always-on`, at 10×. It ran for 14 hours of the day's clock, game 00:03 to 14:05, which is 11.7 hours of ticks. A and B are the two towns, so the gap between them is the town's own noise. The comparison, both balances, both soak reports and the citizen-day CSVs are in [`docs/citizens/baseline/`](docs/citizens/baseline/balance.md). K4.1 compares against them: `capitol-town compare docs/citizens/baseline/a.balance.json <its run>.balance.json`. Both towns' books held, with drift 0 and units conserved.

| Measure | A | B |
|---|---:|---:|
| Citizens' Strands, start → end | 2,000 → 36,091 | 2,000 → 35,748 |
| Held by the 4 fiber gatherers at the end | 34,933 (97%) | 34,639 (97%) |
| Change each: fiber, wood, ore | +8,633, +33, −19 | +8,560, +12, −13 |
| Change each: planks, ingots, tools, builders | −11, −50, −32, −82 | −24, −37, −76, −60 |
| Gini of the citizens' Strands at the end (top tenth's share) | 0.79 (55%) | 0.80 (57%) |
| Faucet a citizen-hour (all of it `exchange_deposit`) | 147.8 | 146.7 |
| Circulation a game hour: goods, commissions, wages, rent | 42, 0, 0, 17 | 42, 0, 0, 21 |
| Circulation as a share of the faucet | 2% | 2% |
| Every citizen's work: fiber, wood, ore, planks, ingots, tools, building | 26, 16, 15, 3, 11, 13, 11% | 25, 16, 19, 3, 11, 13, 10% |
| Every citizen's work: trading, road in, commons; setup | 2, 2, 0%; 1% | 2, 2, 0%; 1% |
| Free, as a share of all logged time | 25% | 24% |
| Jobs a citizen-day, mean (fewest) | 1.4 (1) | 1.3 (1) |
| Activities a citizen-day, mean (fewest) | 2.4 (2) | 2.3 (2) |
| Steps that failed (a citizen-hour) | 5,050 (21.6) | 2,814 (12.1) |
| Times stuck (a citizen-hour) | 2,343 (10.0) | 2,354 (10.1) |

What it says:
- **One job each.** Every role spends 92 to 99% of its work on its own job. The rest is the road in, plus a little trading for the wood and ore gatherers (their stalls) and a sliver of fiber when a float runs low. Jobs a day are 1 for everyone but the wood and ore gatherers, whose stall counts as trading.
- **The makers wait.** The plank crafters were free 75% of their time, and the ingot crafters and builders 44 to 48%. Most of it was stuck: "the kiln waits for a plot, and the plot for the road in's deed" (about 300 times each), and no logs to saw. The fiber, ore and tools trades were rarely free.
- **Nothing goes through the Commons.** In 14 hours nobody posted, claimed or delivered a commission, and nobody took a wage. Commons time and commission payouts are both 0. Only goods (logs, ore and a little fiber, at or near their targets) and stall rent moved.
- **Setup is small,** 0 to 5% of the work, because today's roles climb only their own chains.
- **The failures are a few loops.** A wood gatherer restocks a shop that's already full, about 1,000 times a town ([#544](https://github.com/chrischaps/TheCapitol/issues/544)). Since #544 a full shop (`MAX_LISTINGS`) isn't offered a stock, so K4.1 should count these 1,210 and 1,005 refusals out of the baseline's failed steps when it compares. At 200 citizens every stall gets rented, and those without one asked to stock anyway, about 30,000 of 65,000 failed steps in 14 game hours ([#551](https://github.com/chrischaps/TheCapitol/issues/551)). Since #551 a citizen with no stall is asked only when the bazaar's directory shows one free, and once it finds every stall taken it doesn't look again for a game hour (`STALLS_AGAIN`). Fiber gatherers find the grass taken or spent, about 1,600 times. In A one ore gatherer was walled in by a slope ("Too steep", about 2,000 times; probably #522), which is most of the gap between A's failures and B's.
- **A day is one day.** The run sat inside one day of the day's clock, so "a day" here is 14 hours of it. Detours were 0, since they don't exist yet (K1.5).

## 4. Design decisions

### D1. A calling, and the job of the moment

Who a citizen is and what it's doing are two things.
- **The calling** is persona. `Calling { favourite: Option<Role>, focus, leanings: Vec<(Role, f32)>, wander }` is drawn **last** from the persona's stream, after the habit, as #276 did, so names, temperaments, homes and habits drawn before it don't change.
- **The stint's job** is a `Role`, the type that exists, so every menu (`errands`, `sell_menu`, `bid_menu`, `answers`, …) keeps working per job. (Don't rename it: `sim::Trade` and `wage::Job` are taken.)
- **The roster's `role` stays as the birth trade.** It sets the name and home as today, and it's the favourite unless the citizen is an all-rounder.
- **Existing towns get callings too.** A roster without `calling` draws one from the name. Chris's aim is that no citizen does one thing.

### D2. The activities

A citizen leans over eight **job activities**, the grain the menus already have:

| Activity | Stint job | Notes |
|---|---|---|
| Fiber | `Gatherer(Fiber)` | the faucet: it banks while on it |
| Wood | `Gatherer(Wood)` | trees and old growth |
| Ore | `Gatherer(Ore)` | the wilds when its risk or a commission says so |
| Planks | `Crafter(Planks)` | the sawbench |
| Ingots | `Crafter(Ingots)` | the forge and kiln |
| Tools | `Crafter(Tools)` | the workbench and anvil; sells tools (K2.1) |
| Building | `Builder` | stations and bricks; floors when #478 lands |
| Trading | market making, stalls | open once it has its float (K2.2) |

Two more kinds of time aren't leanings:
- **The Commons pulls.** A commission it could meet, a wage, or a hire interrupts a stint (D4). Its time is filed under the item's activity. (The reports, since K0.1, file the board's and wages' own rounds as `commons`: the walk to the hall, a claim, a delivery, a post, a unit of a shift. Making what a claim owes is the item's job, since the claim is its goal. So the table shows how much the Commons pulls. The stint question may still count a pull toward the item's job when it chases its shares.)
- **Detours** are the unrelated things, each about a quarter hour, set by `wander`:
  - an untried recipe;
  - a wild lake, ford or ridge top;
  - the bazaar lane;
  - the Gazette or the Wanted page;
  - a neighbour's lot.

Stone, clay and charcoal stay inputs, with no leaning of their own. They're gathered on the way to something.

### D3. Styles and the mix (Chris's choice: 40 / 40 / 20)

| Style | Share | Favourite's share of work time | The rest |
|---|---|---|---|
| Specialist | 0.4 | about 70% | two or three other jobs it's good for |
| Leaner | 0.4 | about 45% | spread over most of the rest |
| All-rounder | 0.2 | none | even over all eight, no job above about 25% |

- Everyone leans at least a little toward every activity it can reach.
- Everyone has a `wander`.
- The mix is `[callings] mix` in `town.toml`, after the habits mix's pattern (`citizen/habit.rs`).
- `mint --calling` deals one style.

### D4. The stint question

A person decides what to do this evening. A citizen asks `Question::Stint`, with one `Choice::Take { job, setup, pays }` per job it could do. It's asked:
- when a session starts;
- when a stint ends (its length set by `patience`);
- when a pull arrives.

**Rules score it deterministically:**

```
u = base + w·max(0, target_share − recent_share)   // chase the calling's shares
    × setup cost                                    // D5: stations and tools still missing
    + pull                                          // bids, commissions, wages for this job's goods
    + momentum                                      // for the current job, until its minimum length
```

Chasing target shares rather than drawing at random makes the shares converge to the calling, so they can be measured (§10).

*As built (K1.1, #530):* `decide::rules::stint`. The weights are `STINT` 0.5, `CHASE` 2.0 a point of shortfall, the setup cost `150 / (150 + steps)` (`setup_cost`: 28 steps take a sixth off the shortfall, about 500 three quarters, and nothing is barred), the pull `0.6 · pays / (pays + 150)` (`pays` is Strands a game hour over the books), and `MOMENTUM` 0.5 until `least_stint`, half of `stint_hours` (1.5 game hours times 0.5 to 1.5 by patience). One term was added: `LEAN` 0.01 times the target, so with every share met the calling still breaks the tie, and a job it has no lean to never ties one it has. At 0.04 it kept a specialist's 1% jobs from ever coming up (its favourite ran 7 points over); at 0.01, 400 stints come within 2 points of every job's share, in every style and birth trade (`stints_chase_the_callings_shares_in_every_style`). `Question::Stint { why: start | done | pull, doing, run, lately }` carries what the score reads, with `lately` and `doing` by `Role::slug`. `run` and `lately` were `f32`s at first, because serde_json didn't read every `f64` back to the bit; since #545 capitol-core builds it with `float_roundtrip`, they are `f64`s, and the replay test compares the whole situation. Logs from before still read. A situation with no calling (a swarm bot, a log from before callings) chases its role alone (`Who::calling()`).

**Replay holds:**
- everything the score reads travels in the `Situation`: the current job, how long it's run, `lately` (recent shares), and the calling on `Who`, all `#[serde(default)]`;
- **carry-on is the current job**, so a slow layer never makes a citizen switch;
- the detour draw uses the whim hash (`rules.rs:245`), never an RNG;
- `lately` is kept across sessions in a side file by the roster (`town.stints.json`, K1.2), so a casual citizen's half-hour still adds up.

### D5. No thrash

Taking up a job that isn't your favourite can be all prerequisites. A tool crafter's goals climb eight tools; a builder's, six stations. A person trying a trade starts with the cheapest thing they can make with what they have:
- `plan::setup` (pure) counts the stations, kits and tools still missing for a job's first product. The stint score charges it as a cost.
- A job is worked at one of two commitments. **Full** is today's ladder, for the favourite and for a job it has kept up. **Dabble** is the job's cheapest product within reach.
- A stint whose last step was a prerequisite runs on until it lands, up to a cap. Stations and tools a citizen owns stay owned, so going back picks up where it left off.

### D6. Room comes from the calling, not the stint

- A citizen's lot is bought for the room its **calling** wants: the stations of its favourite and the jobs it leans to most, at most a builder's 4. The station sets nest: planks within ingots within tools within builder.
- `still_wanted`, `repack`, `claim_near` and the road in's claim read the calling. A change of stint never changes what the lot should hold, so nothing is packed up on a switch.
- A deed can't be given up (#519). A job whose stations the citizen doesn't own and can't fit is left off its stint menu. It can still take a wage at someone else's bench in that trade.
- Home zones stay with the birth trade. Nobody moves house.

*As built (K1.3, #532):* "the jobs it leans to most" are its willing ones, the favourite and every job its calling gives `WILLING` (5%) or more, so the lot is kept for the stations they put on one (`sim::stint::lot_for`, by `Role::lot_stations`: planks the sawbench, ingots the kiln and forge, tools those and the anvil, a builder all four), and bought with room for that many (`stint::room_for`, at least 1). A specialist's far jobs keep no room. Most leaners and every all-rounder want a builder's 4; a fiber specialist wants none. The sets nest by count, not as sets (the sawbench isn't the smelter's), so it's their union. A station the calling doesn't keep the lot for, a dabble's chest say, goes only into the room the calling doesn't want (`Holdings::lot_for`, `plot_spare`), and never buys a lot. A job whose stations it doesn't own and can't fit is off the stint menu (`Role::fits`, in `Role::take`), and since a job's `setup` plays its chain out against the lot, one whose kits want a station that doesn't fit is off too: on a lot of 1, a sawbench's kit wants the kiln's bricks and the forge's nails, so an all-rounder there gathers until it has the kit.

### D7. Wants for the wealthy

#403 asked what a rich citizen wants. Callings answer that most of it comes free: a fiber-leaning citizen also saws, smelts and builds, so it bids for logs, ore and kits. The rest are the four ways money leaves a person's pocket in the GDD:

| WP | Want | #403's option |
|---|---|---|
| K2.1 | **A tools market.** Tool crafters sell spare tools; the worn-out buy before they climb. | 1 |
| K2.2 | **Trading as a leaning.** Anyone with a trading lean and its float makes a market or keeps a stall; wealth raises the lean. | 2 |
| K2.3 | **Patrons.** The rich post commissions for what their homes lack, built on floors (#475, #478) rather than a list of their own. *Built: the kits of the calling's stations, a chest, a floor's goods, then the leanings' goods; builders bring the kits (see its row).* | 3 |
| K2.4 | **Hiring.** A citizen with an idle station and Strands to spare offers piece-rate work, escrowed as S2.7 does. | (new) |

- No grant and no new faucet (citizens D2).
- A sink that bites (#403's option 4, #335) stays the tax plan's.

### D8. Seeing what someone does (Chris's choice: the card)

Citizens read a neighbour's trade from their surname today (`persona::trade_of`), which says nothing once people do many things. A person learns what someone does by watching them, or from the paper.
- **The card line.** `PlayerCard.lately` puts it on the card in words, from `production_tally` plus market and bazaar sales over the last 3 game days: "Felling trees and sawing planks; sells at the bazaar". It's for everyone, people and citizens.
- **The opt-out.** It's left off another's card when they keep their name out of the paper (`players.gazette_named`).
- **Citizens read cards.** `meet` asks the cards of those near it and matches what they make against its surplus and needs. `trade_of` is retired. This is D3's rule from the citizens plan: a citizen knows only what a person could read. *Built in K3.2: a quiet card isn't asked; see its row.*

## 5. Architecture notes

- **Where it lives.** The calling lives in `citizen/calling.rs`, held by the persona (`citizen/persona.rs`). The stint question lives in `citizen/decide/` (a `Question` and `Choice` like the rest, with rules scoring, Jev's prose, the corpus and the log). The job of the moment is `SimBot::job()` in `sim/run.rs`.
- **What reads the stint, and what reads the calling.** The menus run per job. Some things take the **union of willing jobs**, because a person answers what they're able and willing to do, not just what they're doing this hour:
  - commission claims (`answers`);
  - wage work (`does`, `crafts_at`);
  - guild charters (`names_trade`, with the favourite first);
  - the walk to the wilds for a commission (`board::ground`).

  Selling takes the **surplus**: what no willing job uses, less what claims owe. (As built, K1.4: a working stock is kept of what another willing job uses, rather than all of it; see its row.)
- **The side file.** It sits beside the roster (like `town.presence.csv`) and keeps each citizen's current job, `lately`, and later its drift (K5.1). The roster writer only appends, so it never rewrites `town.toml`.
- **Reports.** `Activity` is time accounting: the eight jobs plus commons, detour, road in and free. A citizen's report bucket is its favourite's slug, or `all-rounder`. `CitizenReport` gains `calling` and `doing`; old samples keep their slugs. *Built in K3.3, with `time` besides; see its row.*
- **The swarm.** `capitol-bot --sim` bots have no persona, so their calling is focus 1.0 on their `MIX` role, which is exactly today's behaviour. *As built (#552):* not quite at first. K1.3 kept a swarm bot's lot for its role's stations alone, and a plank bot's sawbench kit wants the kiln's bricks and the forge's nails, so it never bought the lot to put the kiln on (197 stuck lines in a 2-hour swarm, planks 12 against 21). So a swarm bot's `Holdings::lot_for` is `None`, any station into any room, as before callings (`lot_kept`'s `one_job`, by `persona.is_none()`): it has no dabble to guard against. Its claim's size and `plot_spare` still come from its role. `a_swarm_bot_buys_its_lot_for_its_whole_ladder` plays every `MIX` role from nothing on the planner.
- **Setup and the commitment (K0.3, built).** Both are in `sim/plan.rs` and `sim/mod.rs`. Nothing calls them with a dabble yet, so citizens plan as they did:
  - `plan::setup(catalog, holdings, goal) -> Result<Setup, Stuck>` plays the planner's steps out against a copy of the holdings, to the goal. It counts the steps that were only on the way: those toward a station, a tool or a plot that something further up needs. `Setup` lists the `stations`, `kits` and `tools` it would make, whether it would claim a `plot`, the setup's `steps` and the goal's `total`. `is_ready()` means `steps == 0`.
  - It's an estimate. A gather yields its least, a craft makes its least, tools don't wear, a claimed plot has room for 4 (`CLAIMED_ROOM`), and walking isn't counted. Past 4000 steps (`SETUP_HORIZON`) it's `Err`.
  - What the goal is itself isn't setup. A builder's `own a kiln` counts the kiln's kit as its work, and the workbench it's made at and the plot as setup.
  - From empty hands, fiber and a builder's first workbench are ready. Wood and ore want 28 steps (a bench and a stone tool), a tool crafter's first axe 12 (the bench), and planks and ingots about 500, since the sawbench and forge need a plot and the kiln's bricks. With a builder's full set, every job is ready.
  - `sim::Commitment { Full, Dabble }` (default `Full`) is taken by `Role::work`, `goals_at`, `errands_at` and `Role::setup`, which is the setup of the work's first goal: what K1.1's `Choice::Take { setup }` charges. `goal`, `goals` and `errands` are `Full`, today's ladder.
  - A **dabble** is one goal: of the job's products, the fewest steps away. For a tool crafter those are its eight tools and a handle; for a builder, its six stations and bricks. Ties go to the ladder's order. The other jobs have one product, so their dabble is their ladder. A tool crafter from empty hands dabbles a handle. A builder with a workbench, a kiln and an axe makes bricks, but without the axe, a storage chest of fiber is nearer. Promises and the float still come first.
  - **For K1.3:** a dabble may place a station, and that takes lot room. D6 keeps the lot for the calling, so K1.3 should leave a dabble's station off unless there's room the calling doesn't want (`Holdings::plot_spare`). *Done in K1.3:* the planner places a station outside `Holdings::lot_for` only into `plot_spare`.

## 6. Phases and work packages

### K0: Groundwork (`cg:K0-groundwork`)

| WP | What | Size | Depends |
|---|---|---|---|
| K0.1 #527 | Time by activity in the town report and `compare`; a baseline soak of today's roles (#403's recipe), the "before" for §10. **Built:** `sim::Activity` (`sim/activity.rs`) on every `Entry::Spent`, with `setup` and the day; free time as `Entry::Idle`; `report::ActivityTime`; the town report's table and `.activity.csv`; `compare`'s money moving, time by activity, and failed and stuck. The baseline is in §3 | S | – |
| K0.2 #528 | `Calling` in the persona, drawn last; `[callings] mix`; the roster's `calling`, with `role` as the birth trade; `mint --calling`; old rosters draw one. *As built:* `citizen/calling.rs`. The jobs are the eight `sim::ROLES`, `merchant` standing for trading. A `Style` isn't stored but read off the calling: no favourite is an all-rounder, a focus of 0.575 or more a specialist (drawn 0.62 to 0.78), less a leaner (0.38 to 0.52). A specialist puts 75 to 85% of the rest on two or three of its favourite's three kin jobs (`calling::kin`, either side of it in the chain), the others at least 0.01 each; a leaner spreads the rest over all seven, its kin ×1.6; an all-rounder weighs all eight 0.7 to 1, so none is over 17%. `wander` is two draws averaged, as a trait. Each share is written to two places, so `Calling::share` scales them to sum to 1. The roster line is `calling = { favourite = "builder", focus = 0.71, wander = 0.4, leanings = { crafter-planks = 0.12, ... } }`, `favourite = "none"` and `focus = 0.0` for an all-rounder; the favourite must be the role or none. `Calling::single(role)` is the swarm's: focus 1.0, no leanings, no wander. `mint --calling specialist\|leaner\|all-rounder`. Measured: `Role::assign(1000)` deals 38.2 / 42.2 / 19.6%. | M | – |
| K0.3 #529 | `plan::setup` and the dabble commitment. *Built: `plan::setup` returns a `Result`, and `Commitment` is on `Role::work`, `goals_at`, `errands_at` and `Role::setup`; see §5* | M | – |

### K1: Stints (`cg:K1-stints`)

| WP | What | Size | Depends |
|---|---|---|---|
| K1.1 #530 | `Question::Stint` and `Choice::Take`: rules scoring, carry-on, prose, corpus, log, replay. *As built:* the score is in D4's note. `Role::take(catalog, holdings, commitment, pays)` makes a job's entry, `None` when its setup can't be reached. `Calling` and `Style` are serde, in the roster's form (`favourite`, `focus`, `wander`, `leanings` by slug, read back in `ROLES` order so shares replay to the bit). They're on `Who.calling` and on the log's `start`, both optional, so the town's 639 old logs (99,723 decisions) replay at 100%. The persona sentence reads the calling ("you'd rather be cutting grass, and you'll saw planks and trade too", plus wander), with `calling::doing` and `to_do` in K3.1's words. The stint question gives the reason, the recent shares and the job it's at. The corpus deals each of the eight birth trades in each style and asks each a stint question, so it's no longer over `MIX`. Nothing asks the question live yet: that's K1.2 | M | K0.2, K0.3 |
| K1.2 #531 | `SimBot::job()`; stints start and end; the side file; the union of willing jobs; banking on a fiber stint. *As built:* `sim::stint` (pure) and `sim/run/stint.rs`. `SimBot::job()` is the stint's `Role`, or the role for a swarm bot, which never asks (`Calling::single`). Every menu that was the role's runs on it: the work (`errands_at` at `stint::commitment`, full for the favourite and a dabble for the rest), selling, buying, bids, stalls, posts, `meet`'s asks, a merchant's market (only on a trading stint), and `Activity`. The question is asked in a session's first round (`start`, `doing` and `run` from the side file), once `run` reaches `stint_hours` (`done`), and after a claim on another job's goods (`pull`, `pays` = what it pays over the books a game hour, by the day's work: `run::stint::claim_pays`). The menu is every job the calling leans to at all, trading for a merchant only (making a market as a leaning is K2.2), each through `Role::take`. A one-entry menu isn't asked. **Lately** is stint time, not the report's activity: the job of the moment is credited each round with the game hours since the last, fading by half every 12 game hours of work (`LATELY_HALF_LIFE`, about eight stints), so a stint that got stuck still counts as tried. A stint whose work is stuck, or has nothing to do, ends, and its job is off the menu for a game hour (`STUCK_AGAIN`); the favourite's never is, so a specialist waiting for its plot keeps at it. **The side file** is `town.stints.json` beside the roster (`town::stints_path`): `{name: {job, run, lately}}`, read as the town starts and written with each health report (`Town::save_stints`, by way of a `.part` file). **The willing jobs** (`stint::willing`): the favourite, the job at hand, then each the calling gives 5% or more (`WILLING`), the most first; a specialist's far jobs (a point or two) aren't, its kin and every job of a leaner and an all-rounder are. They take commissions (`answerer`: the first willing job that answers for the item makes the menu, and `board::ground` counts the wild veins for whoever's willing to mine), wage work (`doer`, with `wage::earns` of that job), and guilds (`guild::seen(&jobs)`: any job's trade, the favourite's first). Banking is a fiber (or trading) stint's, since `Role::banks` reads the job; the `FLOOR`/`FLOAT` fallback stays. Health lines show the style and the job (`sawing planks: craft plank`). **Measured** (#403's recipe as K0.1's baseline, two towns of 20 of the sim's mix, no merchants, every knob default, 10x, 4 hours of the day's clock): every citizen worked 2 jobs a day or more (mean 3.0 and 2.9; the baseline's were 1.4 and 1.3, fewest 1); failed steps 6.5 and 6.3 a citizen-hour (baseline 21.6 and 12.1), times stuck 0.45 (baseline 10), free time 3% (baseline 25%), and no new kind of failure: what's left is grass and trees taken or spent. Both books balanced. The baseline's restocking loop (#544) and the slope (#522) didn't come up in this run; neither is fixed. Not built: D5's "runs on until a prerequisite lands"; the lot, home and road in still go by the birth trade (K1.3). | L | K1.1 |
| K1.3 #532 | Lot, home and the road in by calling; jobs that can't fit left off (#519). *As built:* D6's note. `still_wanted` (now a pure fn in `sim/run.rs`), `plot_spare`, `repack`, `claim`, the road in's `claim_deed`, `builds_at_home` and `walk_the_road` read the calling (`sim::stint::lot_for`, `room_for`), never the job of the moment. `Holdings::lot_for` keeps the lot's room for the calling's stations; the planner puts any other on the plot only into `plot_spare`, and never claims for one. `Role::take` leaves off a job that doesn't `fit` (#519). One fix on the way: `Role::setup` is the first goal that can be reached, not the first goal, as the errands pass over a stuck one, so a builder whose lot has no room for its chest stays on its own menu (it makes bricks). A swarm bot's calling is its role, so its lot and room are `room_wanted`'s, as before (`the_lot_is_bought_for_the_calling_not_the_stint`). A lot taking stations down shows in the report's steps as `pack its lot up again (n down)`. **Verified** by `a_switch_of_stint_never_changes_what_the_lot_should_hold` (every birth trade in every style on lots of 1 to 6, each menu job a stint on the planner: no placement leaves the lot short, which is when it would pack up) and `a_lot_of_one_never_takes_up_building_unless_it_owns_the_stations`. **Existing towns:** a citizen whose calling now wants more than the lot it bought (a fiber-born all-rounder on a lot of 4 with Settle in's workbench on it) isn't packed up (`repack_frees` wants room for all of it), and the job whose last station doesn't fit leaves its menu. | M | K1.2 |
| K1.4 #533 | Surplus and needs across jobs: sell what no willing job uses, bid for what's coming. *As built:* `sim::surplus` (pure) and `Role::uses` (its needs, plus the logs an ingot crafter burns for charcoal and a builder's bricks for its forge). **Spare** (`surplus::spare`) is, of the hall's goods (`MARKET_ITEMS`), what it holds less what its claims still want delivered (`Holdings::promised`, held or not) and less what its work keeps back: all of the job at hand's inputs (it's working them), the job at hand's product while that job is on the way to something else (`surplus::selling`, E3.8's old rule: a builder still building keeps its bricks), and a working stock (`STOCK`, two lots: a forge's 12 bricks, an anvil's 6 ingots) of anything another willing job uses. That's a deviation from "what no willing job consumes": a leaner and an all-rounder are willing at nearly every job, so that reading would have them sell almost nothing; a stock kept back does what it meant. The sell menu (`sim::sell_menu`, now a free function of item and spare) offers the first good someone bids for from half of `SELL_AT` spare, and the stall menu (`stall_menu`) the most it has spare from half of `STALL_AT`, whatever its stint; a trade window (`meet::window`, `Answering.wares`) lays out all of it. A merchant's market goods are never spare, and a swarm bot still sells only its product, so E2.7's numbers compare. **Wants** (`surplus::wants`) are the job at hand's needs, then those of its `NEAR` (2) willing jobs it leans to most, its favourite first, never what the job at hand makes; a swarm bot's are its role's needs. Bids (`bid_menu`, which now bids only up to a `STOCK` held, and not at all past it), posts (`post_menu`), the hall's bid round (`post_demand`, `post_wants`) and `meet::ask_menu` read them; `ask_menu` sells a neighbour the most it has spare of what their trade needs, still read from their surname until K3.2. `buy_menu` was already the stint's (K1.2). The rules layer is unchanged, so old logs replay as they were. | M | K1.2 |
| K1.5 #534 | Detours in work hours. *As built:* `citizen::detour` (pure: the kinds, what's open, `untried`) and `sim/run/detour.rs` (the walk). `Choice::Detour { detour, to, does }`: `detour` is the `DetourKind` (try, see, browse, read, call), since `kind` is the menu's serde tag; `to` is what it goes to, `does` a few words (`see a lake in the hills to the north-east`). It's on the stint menu as a session starts and a stint ends (never on a pull) and on the free-time menu, only in working hours (`Phase::Work`, not the night's), with no claim owed on the board (`Board::owing_any`), on no shift, and not within `DETOUR_AGAIN` (1.5 game hours) of the last. **What's open:** *try*, the least recipe by input units it has never made and can make now with what it holds, its station its own or none and its tool in hand (`detour::untried`; what it holds, not the planner's dabble, so a try never sets anything up); *see*, the nearest sight within 3000 it hasn't been to, or park within 900: `idle::Places` now keeps the wilds' sights, the last point of each `wild_spur_*` (a lake, or a ford within 40) and `wild_climb_*` (a ridge top, looking back at the city), named by their compass point from the middle; *browse*, the bazaar lane and a look in at a stall; *read*, the Gazette at the hall before noon, the Wanted page (the hall's board) after; *call*, the lot of someone it has asked to trade (#295) within 2500, else a neighbour's within 600 of home, at the road nearest its middle, a greeting if they're about, and a read of their card (D8). Each takes about a quarter hour (`DETOUR_SECS`), the walk and the time there, a game minute there at least. **Scoring** (`rules::detour`): on the stint menu one draw a question, from the whim's hash salted apart from every entry (`rules::unit`), takes a detour with a chance of the calling's `wander` (`DETOUR` 1.0), a quarter of it while the job at hand has its momentum (`DETOUR_HELD`). Taken, it's worth 0.05 over the best job and up to 0.1 more by its kind's taste (`DetourKind::taste`: the bold try things, the solitary and the bold go out to the wilds, the sociable browse and call, the patient and the thrifty read) and the whim; not taken, it's under every job, so the jobs' shares are as they were. In free time it's a place like the others, `0.4 + 0.4·wander + 0.15·taste + whim`. One picked on the stint menu is the round's action and is settled with it; what to work at is asked again the round after, and lately isn't credited with the detour's time. **The side file** keeps `made` (every recipe it has crafted) and `seen` (sights and parks it has been to), so "never made" and "hasn't been to" hold across sessions; an old entry reads with none. A trading partner is one asked this session. **Reports:** its time is `Activity::Detour` (from free time too), its step `a detour: …`, and the town report's time table ends "Detours: n of m citizens took one, x% of the work". The console's detour ink is a rose (`--wander`), apart from trading's gold. **Measured:** `most_of_a_town_wanders_off_within_four_hours`, 200 citizens of the default mix asked at each stint's start and end for four game hours: 82% took a detour, 8.6% of the time. **Live** (K1.2's recipe, `scripts/balance.ps1` with empty presets, two towns of 20 of the sim's mix, no merchants, 10x): over game 03:00 to 12:10 of the day's clock, 16 of 20 citizens took a detour in each town, 4.8% and 4.9% of the work, and none failed: the lane (17), the Gazette (14), a park (7), a ford and a lake in the north-east and east hills (4), a storage chest kit tried (3), a neighbour called on (2). Both books balanced; failed steps 7.3 and 4.7 a citizen-hour (K1.2's 6.5 and 6.3), and the stuck were the known waits for a plot. A night run (19:30 to 23:30) took two each, the few in their working hours, as it should. Not built: detours at night, from home (a try at its own bench would suit); a trading partner kept past the session. | M | K1.1 |

### K2: What a rich citizen wants (`cg:K2-wants`)

| WP | What | Size | Depends |
|---|---|---|---|
| K2.1 #535 | A tools market. *As built:* **The check first.** A tool keeps its wear through escrow at the hall (`market::escrow_units`, `market_matcher::deliver`) and at a stall (`bazaar::buy`), and a stall shows it (`BazaarListing.durability`), but the hall's book doesn't (`MarketOrder`, `DepthLevel`, `MarketListing`): cut as [#547](https://github.com/chrischaps/TheCapitol/issues/547) (`fc:backend-prereq`). The hall takes any item type and its panel lists every one, so no tool category was needed. **Sellers.** `sim::TOOLS` (the eight) are on the books a citizen watches (`sim::watched`: `live`'s `watch_market` and `meet::quotes`), and `surplus::spare` counts all but one of each tool spare, whatever the job (the server uses the best of a type, so a second sits in the pack). `Role::product` stays `None` for tools. A tool sells in ones: `sim::sell_at` and `stall_at` are 1 for a tool (`SELL_AT` and `STALL_AT` for a good), on the menus and in the rules' lots, so one spare goes to a bid at the hall, or else to a stall named "… 's tools", even for the patient. That's because a spare tool in the pack wears: the server works the freshest of a type (power × wear left), and in the soak a tool crafter's two "spare" stone axes were both down to 1 use. A worn tool is stocked at its price new times the share of its uses left (`worn_price`). The stall question goes to the most spare of what's enough for one, not only the most spare, which for goods is the same. What goes is the freshest (most durability left), at the hall and the stall, since the hall couldn't show wear, and it keeps its most worn to use up. *Since #547 the hall's book shows wear:* a sell there is priced by the uses left as at a stall, a buy at the hall weighs the price a use left and asks for that tool's uses, and a standing bid takes at least half a tool's uses (economy plan, D7's #547 as-built). An untraded tool is priced at its target (`sim::worth_unbooked`, the catalog's `target_price`: stone 50, iron 130), and the rules' bid and stall prices fall back to the worth the menu was built on (twice the span's least) rather than its top. **A tool crafter** makes `TOOL_STOCK` (2) more for sale past one of each tool it holds (`sim::for_sale`: of those it holds, the one it holds fewest of, ties in `TOOLS`' order, so the stone axe and pickaxe a climb wants come first), then handles; one sold, it makes another. Its tools at its stall or on the hall's book count toward the 2 (`Holdings::tools_on_sale`), so it doesn't make them without end. The tools it lacks still come first on its ladder, so while its iron ones wait on a plot and three stations it sells stone. (A first cut waited for its whole set of eight, and in the soak no tool crafter got there in 6 game hours.) 2, not more: tools don't stack, and a pack has 24 slots, 8 of them its own tools already. The swarm's tool crafters make them too but never sell them (a swarm bot sells only its product), so E2.7's numbers move by those few. **Buyers.** `plan::tool_wanted(catalog, holdings, goal)` is the tool type the next step climbs toward, the first the planner finds missing. When there is one, a citizen not on a tools stint (`sim/run/tools.rs`, `SimBot::choose_tool`) is asked `Question::Buy` with `sim::tool_menu`: `Choice::Buy` (or `Choice::Bid`) for one, or `Choice::Make`, a new choice ("don't buy: make the tool yourself"; the rules weigh it as `Gather`, 0.5 + 0.4 × thrift). It's offered only with `tool_margin` (1 to 3 by thrift) times the price spare over what it `kept` back. Where: the hall's best buy (`sim::tool_offer`: the cheapest a use at its full life, `per_use`, so an iron axe at 130 for 200 uses beats a stone one at 50 for 60); else a stall whose sign lists one (`Bot::bazaar`, from anywhere, at most once a game hour), where it browses, sees the wear and takes the one cheapest a use left at no more a use than its price for a new one (`Trade::Shop`, `buy_at_a_stall`, an `Entry::Traded` bought); else a standing bid for the cheapest of the type at its worth, once, while it climbs on: in the soak, bids for the best buy (iron, at its target) stood unfilled while the tool crafters made stone. A tool bought, bid for or bought at a stall is setup in the reports, as making it is. A tool a standing bid bought waits in the market box, and holdings count only the pack, so a citizen remembers it (`boxed_tools`, from its fills) and goes to the hall to collect it (`Trade::Collect`) rather than buy another. **Fiber** still needs no tool; grass and the sickle stay in §8. **Measured** (`scripts/balance.ps1`, two towns of 20 of the sim's mix, no merchants, every knob default, 10x, master's `capitol-town` in town A and this one in B, `BALANCE_TOWN_A`). Game 06:55 to 13:00: B traded 10 stone axes at the hall at 47 to 62 (mean 53.5 against a target of 50), and A none. A tool crafter sold 6 into standing bids for 327 Strands, and its market sales came to 447 against 228 in A; the two tool crafters' sales together 675 against 328. They still ended at +80 each against A's +212, because the one who sold banked 299 fewer Strands of fiber (its time went into axes) and put 178 into a hire's escrow: with two a side, noise. The other 4 sales were resales by buyers whose bids had filled into their market boxes while they bought or bid again, which `boxed_tools` fixed. Game 14:00 to 19:00, with it: B traded 3 (two stone pickaxes at 53 and 58, a stone axe at 47), all from a tool crafter to an ore gatherer, a woodcutter and a fiber gatherer, with no resales, and A none; the tool crafters ended at +370 each against A's +339. No tool went through a stall in either run: every sale was to a standing bid. Both runs' books balanced in both towns (drift 0, units conserved). Two earlier runs were cut short by what they found: a first cut made spares only once a tool crafter held all eight tools, and none got there; then its spares wore in its pack. | M | K1.4 |
| K2.2 #536 | Trading as a leaning; `--role merchant` becomes a trading favourite. *As built:* every citizen has a `Merchant` (`sim/run/merchant.rs`): its goods are `sim::trading_goods(calling, seed)`, `MERCHANT_ITEMS` (3) of the hall's goods drawn by seed, each weighed by the shares of the calling's other jobs that make or use it (`Role::uses`), filled from `merchant_items(seed)` when short, so a sawyer-leaning trader deals in planks and logs. **Open** (`SimBot::trades`): trading is on the stint menu always for a trading favourite (which earns its float on the stint by fiber, as C1.6's merchants did), and for anyone else leaning to it once its float (Strands, on bids too, plus its goods) reaches `rules::enough_float` (`MERCHANT_FLOAT` by patience, half while making a market). A float newly earned is a pull (`Why::Pull`), asked once the stint it's on has run its least. **On a trading stint** it asks `Question::Market` as before: a market at the hall, or else a stall at the bazaar for the good of its market it holds most of in hand, priced from its maker's reference (`stall_its_goods`), or else its errand. Orders stay up between stints. Its market goods aren't spare while it trades (a favourite's never are). **Wealth raises the lean:** `Question::Stint` carries `line` (`[trading] line` in `town.toml`, default `stint::TRADE_LINE` = 2 × `MERCHANT_FLOAT`, times 0.5 to 1.5 by patience, `stint::trade_line`); the rules chase `stint::target`, which gives trading `rich_rise` (0 at the line, up to `RICH_RISE` 0.3 at twice it) of what's left and scales the rest down, from the situation's Strands in hand, so a replay answers the same (an old log's `line` is 0). Trading is now among the willing jobs at 5% (a traders' guild suits it); `surplus::near` still leaves it out. `Calling::seeded` deals a merchant-born citizen no all-rounder unless the mix deals nothing else, so `mint --role merchant` makes a trading favourite. The scripts' `--merchants` / `-Merchants` are counts of trading favourites. The health line shows the stint a round's question took up, so a spell under a new trading stint reads as trading. **Measured** (`scripts/balance.ps1`, two towns of 20 of the sim's mix, **no merchants**, every knob default, 10×, 4 hours of the day's clock, game 12:07 to 16:10, before the pull was asked at the least stint): the book's first orders were the road in's one-unit lessons in the first game minute, and goods traded from game minute 30 (27 and 34 trades in the first game hour). The first market was made by a fiber leaner with a float of 1,181 at 1.16 game hours in both towns, a second (another fiber gatherer) at 2.79 and 3.12. The leaner's bids bought 578 and 651 units for 902 and 1,047 Strands, and sold 531 and 792: the fiber trades' share of the citizens' Strands ended at 32% and 39% (97% in K0.1's baseline over 14 hours), circulation was 77% and 93% of the faucet (baseline 2%), goods 776 and 800 Strands a game hour (baseline 42), and the Gini 0.69 and 0.61. Much of that is K1.2 to K2.4 too: an A/B against master in one run was started and stopped for time, so the trading share isn't separated from them. Both books balanced (drift 0). | M | K1.2 |
| K2.3 #537 | Patrons post commissions for their homes. *As built:* `sim::patron` (pure) and `SimBot::post_patronage` (`sim/run/board.rs`). **Who:** a citizen with Strands in hand over its line (`[patrons] line` in `town.toml`, `patron::PATRON_LINE` = 1500, past the hiring and trading lines, times 0.5 to 1.5 by thrift, `patron::patron_line`); the line is what it keeps back, so a post never takes it under. **When:** at the hall, after the crafters' posts in `post_demand`, and as a market spell starts (`merchant.rs`), since the rich are mostly making markets there. It collects its market box first: a market spell doesn't, and a kit that came would otherwise be posted for again (the soaks saw two chests posted twice before it). **What** (`patron::home_wants`), first things first and from the home's own rules: the kits of the stations `stint::lot_for` keeps its lot for that it doesn't own, while the lot has room (a fire on the ground only, anything else upstairs first), then a chest's kit in the room its calling doesn't want (`plot_spare`), then, once those stations want more room than the lot has and a floor may go up now (`Holdings::floor`, the permit and its `FLOAT` in hand), the next floor's goods it lacks. Only with nothing lacking, a `STOCK` of each good `surplus::wants` names. **The menu** (`patron::patron_menu`): `Question::Buy` with `Choice::Post` from the books' worth to twice it, up to two lots and what its line leaves; the rules post at worth × `premium`, as a crafter's post is priced. A kit's worth is its target until one trades (`worth_unbooked` now covers kits). The board's limit (`MAX_POSTS`, shared with a crafter's posts), the grade (`POST_GRADE`) and `POST_DAYS` stay. One it passed is asked about again after a game hour. **Who brings them:** a builder answers commissions for the kits a lot takes (`BUILDS_FOR_OTHERS`: kiln, forge, sawbench, anvil, chest; not a workbench's), so anyone willing to build does (K1.2), one kit a claim (`day_work` is 1 for a kit: they don't stack). A kit a claim of its own has promised isn't counted in its holdings, so a builder never puts down the kit it owes. **What comes goes down:** `patron::furnish` puts a goal for each kit in hand whose station stands on a lot and that the lot takes now at the front of `Role::goals_at`, after its promises, so the next work places it, and builds the floor through `Step::Floor` when that's what it takes. **Measured** (2026-10-26, a copy of the dev town's database at 10×, 44 of its citizens: its 19 with over 1,000 Strands, 10 builders with lots and 15 others with lots, every one on, 6 hours of the day's clock twice): game 10:00 to 15:30, 43 posts from 18 citizens, 38 of them kits from 17 patrons (fiber gatherers and all-rounders, whose lots held workbenches only: 18 chests, 10 kilns, 6 forges, 4 sawbenches), 36 filled, none lapsed. Builders brought 16 kits for 1,309 Strands, and fiber and wood leaners willing to build 18 more; the builders who started with 26 to 269 Strands ended making markets with floats of 1,000 to 2,600. The patrons put down 4 kilns, 3 forges, 3 chests and a sawbench by the end; the rest came during market hours and waited in their boxes or packs for the next work. Game 15:00 to 21:00 with the box collected first: 16 posts from 12 patrons, 10 filled, no kit posted twice. Both runs' books balanced (drift 0, units conserved). No patron was near a floor (#549), so the floor's goods are covered by `patron` tests only. | M | K1.4, #475 |
| K2.4 #538 | Citizens hire. *As built:* `Question::Hire { work, rate_for, fair, line }` and `Choice::Hire { rate, cap }` (`sim/wage.rs`, `sim/run/hire.rs`). **Where** (`wage::openings`): a station of its own whose trade isn't the stint's (`Role::crafts_at`), for the recipe there, of the goods that station's trades answer for or need (sawbench planks, forge ingots, kiln charcoal or bricks, workbench handles, and at a workbench handwork a trade answers for, rope from fiber; an anvil's heads aren't hired out), that the inputs its stint neither needs nor makes make the most of, at least `LEAST_HIRE` (6) units; or its lot, gathering a gatherer's good that grows in its bounds (`wages::resources_on`, over every node it has seen) and that the stint doesn't gather, where its nodes keep a worker busy at least half the time, since a spent node is a failed unit and three in a row end a shift (`wage::gather_most`: four grass tufts do, a lone vein spent ten minutes a pull doesn't), up to a game hour's yield or `GATHER_CAP` (60). **The line** is `[hiring] line` in `town.toml` (default `HIRE_LINE`, 600) times 0.5 to 1.5 by thrift (`wage::hire_line`); the question is asked only when the Strands over it escrow `LEAST_HIRE` units at the fair rate, so nobody hires while broke. **The rate** differs from the issue's "`rate_for`, shaded by thrift": it's what a unit earns the trade that does it at the citizen's books (`wage::fair`: a craft's goods less its inputs, a unit's share; a gather's good), `rate_for` only when the books can't say, times 1.2 to 0.8 by thrift, rounded (`wage::hire_rate`). `rate_for` (half the target) pays a middling plank sawyer less than its own trade, so nobody took it, and pays an ingot smelter five times the trade's margin; at the fair rate a middling worker takes a middling employer's offer (`a_middling_citizen_takes_a_middling_citizens_offer`). The rules hire at `0.45 + 0.45·min(1, over/line) − 0.3·(thrift − 0.5)` against `PASS` 0.5, for as many units as the Strands over the line escrow with the court's fee (`wage::affordable`); the sim clamps a layer's answer to that again. **Offering**: a craft's inputs go into the station's box (`Bot::stock_station`, whole stacks), then `POST /wages` for a day (`TERM_DAYS`), at most 2 up at once (`MAX_HIRES`), each place asked about once a game hour. A citizen's `trade` key may now offer and take down (`api_keys::rest_scope`: `POST /wages`, `DELETE /wages/{id}`). The mark is the server's `WageOffer.employer_citizen`, already on the Work page. **Ending**: at dusk (`go_home`, `day`), as it signs off (`signed_off`), when the stint's job works the station or gathers the resource (`wage::wants_back`, `wanted`), and under half its line (`low`), by `DELETE /wages/{id}` (`Bot::withdraw_wages`); the server's own closes (`filled`, `expired`, `gone`) come by `wage_closed`. Offers a session left up are taken up on its first read of the board. What a craft's hire made and left comes home from the box (`Bot::empty_station`) on the next round, on the walk home, or as it signs off. **Reports**: `Entry::Hired` and `Entry::HireEnded`, counted into the Wages section (offers, worked, units bought, paid, fees, why they came down) and balanced in `BotRun::tax`. Jev's and Claude's prose, the corpus (a lot's grass and a dozen logs' planks for each citizen it can afford) and replay's `hire` kind. **Measured** (`scripts/balance.ps1`, two towns of 20 of the sim's mix, no merchants, every knob default, 10x). A first run of 6 hours of the day's clock (game 02:00 to 08:00) found 3 offers, the first at the end, once the road in's deeds had come: a kiln's charcoal filled at its cap of 58, paid 116 Strands, its box brought home; and a lone iron vein on a lot whose 8 shifts all gave up on a spent vein, which is why a gather now needs nodes that keep a worker busy. A second of 10 hours (game 09:00 to 19:00), with that: 8 offers in A and 11 in B, every one worked, 18 of the 19 to the cap (charcoal, planks, an ingot run at 23), 132 and 209 units bought for 264 and 586 Strands, by 4 and 4 employers (the rich ones: wood, fiber and tools favourites and an all-rounder) of 20. The wages each report paid, the workers earned and the server's `court_payout` agree to the Strand in both towns, and both books balanced (drift 0, units conserved). Every escrow left its employer over its line (the least 483 against a line of 468), so nobody hired while broke. Most lots have no nodes in their bounds (2 of 20 in A), so hires are crafts; the rope handwork was added after the runs and is tested only by the unit tests. The court's fee is at least 6 Strands, so the runs' 6-unit offers at 2 paid half again in fees; since them an offer must be worth `LEAST_HIRE_VALUE` (50) at the fair rate, which would have left 3 of A's 8 and 5 of B's 11. | M | K1.2 |

### K3: Seeing it (`cg:K3-seeing`)

| WP | What | Size | Depends |
|---|---|---|---|
| K3.1 #539 | `PlayerCard.lately` from `production_tally` and sales, with the byline opt-out; the card in the Flint client (`fc:backend-prereq`). *As built:* `lately` is `rules::lately::Lately { doing, sells }`: up to 3 activities, most first, and the venues sold at (`bazaar`, `exchange`), never a number. Each item belongs to one activity in `rules::lately::ACTIVITIES` (logs are "felling trees", every tool head "forging tool heads"; an unknown item is "making" it, and a backend test holds every content item to the table). The weight is the units at the item's target price (its last trade, else 1), as the maker board counts, over today and the 2 game days before; an activity under 10% of the whole is left off, but the first never is. `rules::lately::words` makes the line ("felling trees and sawing planks, and selling at the bazaar"), drawn under the standing as "Lately …". `items_of` turns an activity back into goods, for K3.2. | M | – |
| K3.2 #540 | Citizens read cards, not surnames; `trade_of` retired. *As built:* `meet::near` finds the nearest three citizens (`ASK_AMONG`), and `SimBot::ask_around` reads the card of each it hasn't asked lately (`SimBot::card_of`: `GET /players/{id}/card`, kept per player per game day; an HTTP refusal is kept as a quiet card, a call that went unanswered isn't kept). `Neighbour::read_card` turns the card into `makes`, every good of each of its `lately.doing` activities (`rules::lately::items_of`, so "mining ore" is all three ores: an asker knows the trade, not the item), and `needs`, the inputs of every recipe in the catalog that makes one of those, less what they make themselves (a sawyer who makes handles from its planks needs logs, sticks and rope, not planks). That's a person's reading of a card: what someone makes is on it, and what that takes is in the recipes anyone can read. `ask_menu` buys from one who makes the first of its wants they make, and sells to one whose goods take what it has spare. **A quiet card** (nothing made these 3 game days, or someone who keeps out of the papers, which reads the same) or one naming only an activity the table doesn't know says nothing to ask about, so they aren't asked: no fallback to the name. `persona::trade_of` is gone; a surname is only a name. **People still aren't asked**, though their cards now say what they do: that's #546, for Chris. `scripts/balance.ps1` takes `BALANCE_TOWN_A` (a capitol-town exe for town a, so two builds compare under one preset) and `BALANCE_LOG_MB` (decision logs) for the measure. **Not yet measured:** an A/B of 20 citizens, no merchants, 10x, 4 hours (master's capitol-town in a, this in b) lived game 16:00 to 20:46, after both markets, and no citizen in either town asked anyone (no `ask` decision at all: asks happen only while lingering, and since K1.2 free time is about 3%); both books balanced. #536's run over the afternoon market saw 1 and 2 asked trades, so a fair count needs a run over game 9:00 to 17:00, and more citizens; that's left to K4.1 (#542). | S | K3.1, K1.2 |
| K3.3 #541 | `CitizenReport.calling` and `doing`; the console's roster, wealth and time by activity (`fc:backend-prereq`). *As built:* `CitizenReport.role` is the bucket (`Calling::bucket`: the favourite's slug or `metrics::ROLE_ALL_ROUNDER`, `all-rounder`), so `Distribution` buckets by it with no change on the server, and an old sample's slugs are the same keys. Three fields, all `serde(default)`: `calling` (`CallingReport`: style, favourite, its share, and the top three leanings of 5% and over, the most first, `Calling::leaning_most`), `doing` (`StintReport`: the stint's job and its game hours, kept while away) and `time` (game hours by `Activity` slug since the town started, summed as entries are recorded, `Health::spent`). One form: the town report's tables, its CSV, `compare` and the health line all name a citizen by the bucket, and the swarm its role by slug; a balance file's old names (`gatherer (wood)`) read as slugs, so the baseline still compares. The console: Calling ("Wood, also ore and planks", "All-rounder"), the stint before the step in Doing, a Time by activity bar per citizen with a key, the citizen page's sentence and its calling set against its time, and Wealth by calling. The Display form (`gatherer (wood)`) is left for prose; nothing in a report uses it | M | K0.1, K1.2 |

### K4: The soak (`cg:K4-soak`)

| WP | What | Size | Depends |
|---|---|---|---|
| K4.1 #542 | K0.1's soak again, on callings, at 20 and 200; tune; §10's measures; closes #403 | M | K0–K3 |

### K5: Finding a niche (`cg:K5-niche`)

| WP | What | Size | Depends |
|---|---|---|---|
| K5.1 #543 | Leanings drift by what paid, the guild, and what kept getting stuck; bounded; saved in the side file (`needs-decision`) | L | K4.1 |

## 7. Dependency graph and estimate

```
K0.1 ──────────────────────────────────────┐
K0.2 ─┬─► K1.1 ─┬─► K1.2 ─┬─► K1.3         │
K0.3 ─┘         │         ├─► K1.4 ─┬─► K2.1
                └─► K1.5  │         └─► K2.3 ◄── #475
                          ├─► K2.2
                          ├─► K2.4
K3.1 ──────────► K3.2 ◄───┤
K0.1 ──────────► K3.3 ◄───┘
                     all ─► K4.1 ─► K5.1
```

- **Sizes:** 17 work packages: 2 small, 13 medium, 2 large.
- **What can start together:**
  - K0.1, K0.2, K0.3 and K3.1 have no dependencies and can run side by side;
  - K1.3, K1.4, K1.5, K2.2 and K2.4 open together once K1.2 lands.
- **K1.2 is the hinge:** the one large package everything else waits on.

## 8. Later (outside this plan)

- **Grass and the sickle.** Should grass yield more to a sickle, so fiber gatherers want a tool too (#403)? It's a balance question for the content.
- **Leanings toward stone, clay and charcoal**, if the soak shows those chains short.
- **Activities the game doesn't have yet.** Callings take them up as new rows in D2's table.
- **Citizens founding guilds.** A guild named for a calling's favourite is the natural next step, once K5's niches settle.

## 9. Risks

- **Thrash.** Citizens switching so often they only ever make kits. D5's setup cost, momentum, a minimum stint and the "runs on until it lands" rule are the guard. K4.1 checks setup time stays within baseline + 10 points.
- **Lots too small.** An urban lot takes four stations (one before Plots v2), and a deed can't be given up (#519). D6 leaves such jobs off the menu (K1.3: `Role::fits`). A wage in that trade, or a floor (#478), is the way in.
- **Short sessions.** A casual citizen on for half an hour would always pick its favourite. The side file's `lately` keeps the shares across sessions.
- **The faucet.** If more citizens bank fiber on fiber stints or when broke, the faucet could grow. §10 holds it within ±25% per citizen-hour.
- **Replay.** A stint score that reads anything outside the `Situation` would break replay. K1.1's tests replay a logged stint to the bit.
- **Comparability.** Balance runs across the switch compare different populations. K0.1's baseline is taken first, and the swarm stays at one job.

## 10. Verification strategy

K4.1 runs K0.1's soak again on callings: 20 citizens at 10× for 14 to 17 game hours, plus a 200-citizen run. Targets:

| Measure | Target |
|---|---|
| Strands held by fiber-favouring citizens | under 30% of the town's (was 93%, #403) |
| Gini of citizens' Strands | below K0.1's baseline |
| Circulation: Strands spent on goods, commissions, wages and rent per game hour | at least 50% of the faucet |
| Faucet per citizen-hour | within ±25% of the baseline |
| Favourite's share of work time | specialists 60–80%, leaners 35–55%, no all-rounder job over 35% |
| Distinct activities a game day | every citizen at least 3, all-rounders at least 5 |
| Detours | at least 70% of citizens take one a game day |
| Setup (kits, tools, placing) | no more than baseline + 10 points of work time (the reports' setup, K0.1's `activity::is_setup`) |
| Stuck and failed steps | no more than the baseline's |
| Markets | tools trade, at least one hire a day per 20 citizens, citizens who aren't crafters post |
| "Lately busy with" | matches the activity with the most time for at least 80% of citizens |
| Determinism | `capitol-town replay --layers rules` 100%; `/admin/ledger` drift 0 |
| The swarm | `capitol-bot --sim 20 --hours 2` within noise of its baseline |

Each work package has its own unit tests and a short town run (in its issue). The baseline (K0.1) and the results (K4.1) are recorded here.
