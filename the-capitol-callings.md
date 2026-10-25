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
- Nobody hires. The Commons plan made it a non-goal; this plan takes it up (K2.4).

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
  - `trade_of` (`:294`) reads a surname back as a trade, which is how `meet` picks whom to ask (`citizen/meet.rs:632-711`).
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
- **The failures are a few loops.** A wood gatherer restocks a shop that's already full, about 1,000 times a town ([#544](https://github.com/chrischaps/TheCapitol/issues/544)). Fiber gatherers find the grass taken or spent, about 1,600 times. In A one ore gatherer was walled in by a slope ("Too steep", about 2,000 times; probably #522), which is most of the gap between A's failures and B's.
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

*As built (K1.1, #530):* `decide::rules::stint`. The weights are `STINT` 0.5, `CHASE` 2.0 a point of shortfall, the setup cost `150 / (150 + steps)` (`setup_cost`: 28 steps take a sixth off the shortfall, about 500 three quarters, and nothing is barred), the pull `0.6 · pays / (pays + 150)` (`pays` is Strands a game hour over the books), and `MOMENTUM` 0.5 until `least_stint`, half of `stint_hours` (1.5 game hours times 0.5 to 1.5 by patience). One term was added: `LEAN` 0.01 times the target, so with every share met the calling still breaks the tie, and a job it has no lean to never ties one it has. At 0.04 it kept a specialist's 1% jobs from ever coming up (its favourite ran 7 points over); at 0.01, 400 stints come within 2 points of every job's share, in every style and birth trade (`stints_chase_the_callings_shares_in_every_style`). `Question::Stint { why: start | done | pull, doing, run, lately }` carries what the score reads, with `lately` and `doing` by `Role::slug` and `run` an `f32`, because serde_json doesn't read every `f64` back to the bit (#545). A situation with no calling (a swarm bot, a log from before callings) chases its role alone (`Who::calling()`).

**Replay holds:**
- everything the score reads travels in the `Situation`: the current job, how long it's run, `lately` (recent shares), and the calling on `Who`, all `#[serde(default)]`;
- **carry-on is the current job**, so a slow layer never makes a citizen switch;
- the detour draw uses the whim hash (`rules.rs:245`), never an RNG;
- `lately` is kept across sessions in a side file by the roster, so a casual citizen's half-hour still adds up.

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

### D7. Wants for the wealthy

#403 asked what a rich citizen wants. Callings answer that most of it comes free: a fiber-leaning citizen also saws, smelts and builds, so it bids for logs, ore and kits. The rest are the four ways money leaves a person's pocket in the GDD:

| WP | Want | #403's option |
|---|---|---|
| K2.1 | **A tools market.** Tool crafters sell spare tools; the worn-out buy before they climb. | 1 |
| K2.2 | **Trading as a leaning.** Anyone with a trading lean and its float makes a market or keeps a stall; wealth raises the lean. | 2 |
| K2.3 | **Patrons.** The rich post commissions for what their homes lack, built on floors (#475, #478) rather than a list of their own. | 3 |
| K2.4 | **Hiring.** A citizen with an idle station and Strands to spare offers piece-rate work, escrowed as S2.7 does. | (new) |

- No grant and no new faucet (citizens D2).
- A sink that bites (#403's option 4, #335) stays the tax plan's.

### D8. Seeing what someone does (Chris's choice: the card)

Citizens read a neighbour's trade from their surname today (`persona::trade_of`), which says nothing once people do many things. A person learns what someone does by watching them, or from the paper.
- **The card line.** `PlayerCard.lately` puts it on the card in words, from `production_tally` plus market and bazaar sales over the last 3 game days: "Felling trees and sawing planks; sells at the bazaar". It's for everyone, people and citizens.
- **The opt-out.** It's left off another's card when they keep their name out of the paper (`players.gazette_named`).
- **Citizens read cards.** `meet` asks the cards of those near it and matches what they make against its surplus and needs. `trade_of` is retired. This is D3's rule from the citizens plan: a citizen knows only what a person could read.

## 5. Architecture notes

- **Where it lives.** The calling lives in `citizen/calling.rs`, held by the persona (`citizen/persona.rs`). The stint question lives in `citizen/decide/` (a `Question` and `Choice` like the rest, with rules scoring, Jev's prose, the corpus and the log). The job of the moment is `SimBot::job()` in `sim/run.rs`.
- **What reads the stint, and what reads the calling.** The menus run per job. Some things take the **union of willing jobs**, because a person answers what they're able and willing to do, not just what they're doing this hour:
  - commission claims (`answers`);
  - wage work (`does`, `crafts_at`);
  - guild charters (`names_trade`, with the favourite first);
  - the walk to the wilds for a commission (`board::ground`).

  Selling takes the **surplus**: what no willing job uses, less what claims owe.
- **The side file.** It sits beside the roster (like `town.presence.csv`) and keeps each citizen's current job, `lately`, and later its drift (K5.1). The roster writer only appends, so it never rewrites `town.toml`.
- **Reports.** `Activity` is time accounting: the eight jobs plus commons, detour, road in and free. A citizen's report bucket is its favourite's slug, or `all-rounder`. `CitizenReport` gains `calling` and `doing`; old samples keep their slugs.
- **The swarm.** `capitol-bot --sim` bots have no persona, so their calling is focus 1.0 on their `MIX` role, which is exactly today's behaviour.
- **Setup and the commitment (K0.3, built).** Both are in `sim/plan.rs` and `sim/mod.rs`. Nothing calls them with a dabble yet, so citizens plan as they did:
  - `plan::setup(catalog, holdings, goal) -> Result<Setup, Stuck>` plays the planner's steps out against a copy of the holdings, to the goal. It counts the steps that were only on the way: those toward a station, a tool or a plot that something further up needs. `Setup` lists the `stations`, `kits` and `tools` it would make, whether it would claim a `plot`, the setup's `steps` and the goal's `total`. `is_ready()` means `steps == 0`.
  - It's an estimate. A gather yields its least, a craft makes its least, tools don't wear, a claimed plot has room for 4 (`CLAIMED_ROOM`), and walking isn't counted. Past 4000 steps (`SETUP_HORIZON`) it's `Err`.
  - What the goal is itself isn't setup. A builder's `own a kiln` counts the kiln's kit as its work, and the workbench it's made at and the plot as setup.
  - From empty hands, fiber and a builder's first workbench are ready. Wood and ore want 28 steps (a bench and a stone tool), a tool crafter's first axe 12 (the bench), and planks and ingots about 500, since the sawbench and forge need a plot and the kiln's bricks. With a builder's full set, every job is ready.
  - `sim::Commitment { Full, Dabble }` (default `Full`) is taken by `Role::work`, `goals_at`, `errands_at` and `Role::setup`, which is the setup of the work's first goal: what K1.1's `Choice::Take { setup }` charges. `goal`, `goals` and `errands` are `Full`, today's ladder.
  - A **dabble** is one goal: of the job's products, the fewest steps away. For a tool crafter those are its eight tools and a handle; for a builder, its six stations and bricks. Ties go to the ladder's order. The other jobs have one product, so their dabble is their ladder. A tool crafter from empty hands dabbles a handle. A builder with a workbench, a kiln and an axe makes bricks, but without the axe, a storage chest of fiber is nearer. Promises and the float still come first.
  - **For K1.3:** a dabble may place a station, and that takes lot room. D6 keeps the lot for the calling, so K1.3 should leave a dabble's station off unless there's room the calling doesn't want (`Holdings::plot_spare`).

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
| K1.2 #531 | `SimBot::job()`; stints start and end; the side file; the union of willing jobs; banking on a fiber stint | L | K1.1 |
| K1.3 #532 | Lot, home and the road in by calling; jobs that can't fit left off (#519) | M | K1.2 |
| K1.4 #533 | Surplus and needs across jobs: sell what no willing job uses, bid for what's coming | M | K1.2 |
| K1.5 #534 | Detours in work hours | M | K1.1 |

### K2: What a rich citizen wants (`cg:K2-wants`)

| WP | What | Size | Depends |
|---|---|---|---|
| K2.1 #535 | A tools market | M | K1.4 |
| K2.2 #536 | Trading as a leaning; `--role merchant` becomes a trading favourite | M | K1.2 |
| K2.3 #537 | Patrons post commissions for their homes | M | K1.4, #475 |
| K2.4 #538 | Citizens hire | M | K1.2 |

### K3: Seeing it (`cg:K3-seeing`)

| WP | What | Size | Depends |
|---|---|---|---|
| K3.1 #539 | `PlayerCard.lately` from `production_tally` and sales, with the byline opt-out; the card in the Flint client (`fc:backend-prereq`). *As built:* `lately` is `rules::lately::Lately { doing, sells }`: up to 3 activities, most first, and the venues sold at (`bazaar`, `exchange`), never a number. Each item belongs to one activity in `rules::lately::ACTIVITIES` (logs are "felling trees", every tool head "forging tool heads"; an unknown item is "making" it, and a backend test holds every content item to the table). The weight is the units at the item's target price (its last trade, else 1), as the maker board counts, over today and the 2 game days before; an activity under 10% of the whole is left off, but the first never is. `rules::lately::words` makes the line ("felling trees and sawing planks, and selling at the bazaar"), drawn under the standing as "Lately …". `items_of` turns an activity back into goods, for K3.2. | M | – |
| K3.2 #540 | Citizens read cards, not surnames; `trade_of` retired | S | K3.1, K1.2 |
| K3.3 #541 | `CitizenReport.calling` and `doing`; the console's roster, wealth and time by activity (`fc:backend-prereq`) | M | K0.1, K1.2 |

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
- **Lots too small.** An urban lot takes one station, and a deed can't be given up (#519). D6 leaves such jobs off the menu. A wage in that trade, or a floor (#478), is the way in.
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
