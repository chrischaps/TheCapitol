# The Capitol: The Commons Plan

*2026-10-21. Plan issue #352. Work packages #378 to #401, plus #311. Issues are labelled `commons`, with phase labels `cm:S0-groundwork` to `cm:S4-guilds`.*

## 1. Vision

The city is lived in now. Citizens keep human hours, stand on the hall's steps at market time, linger on the bridges at dusk and stop to turn to you as you pass. But nobody says anything. Every trade is a window of numbers. Every need is a bid on a book. What one person wants of another can't be said, promised, kept or broken.

**The Commons** gives the city its social half, in this order:
1. **Talk:** a line said where you stand, heard by those near you; one channel for the whole Capitol; and letters that wait for someone who's away.
2. **Commissions:** "20 iron ore, grade C or better, 6 Strands each, by the fourth day", pinned to the Gazette's board in the hall's forecourt with the money already down. A **claim** on one is a promise.
3. **Wages:** piece-rate work at someone else's forge, paid unit by unit as it's made.
4. **Standing:** whether you keep your word, said in words.
5. **Guilds:** a name over a hall, a shared purse, and dues to keep it.

The question the plan answers is this: **can the economy become a society?** Can a stranger's request, a promise kept and a name you come to trust make the Capitol feel like a place people belong to, and not only a market they visit?

It's also the economy's own fix. The town is short of demand. Strands pool with the gatherers and nobody buys (#294), and nobody walks to the wilds for ore (#336). A commission is demand you can see, and it pays the walk.

## 2. Goals and non-goals

**Goals**
- Local talk and a Capitol-wide channel, shown over heads and in a chat panel, and letters for people who are away.
- Delivery commissions with escrow and part delivery, posted and delivered at the hall's board, and claims that hold a share for a promise.
- Piece-rate wages at a station or on a lot. The pay is escrowed, so an employer can't breach.
- Standing in words from a record of promises, and weekly leaderboards in the paper and on the board.
- Guilds: a registry, roles, a treasury, dues, a hall in the guild district, and the Guild permission made real.
- Citizens in all of it: they speak, answer and post commissions, take wages and join guilds. They're marked honestly throughout.
- New sinks from the GDD's Contract Court and Guild Registry: the court's fee, postage, guild fees and dues.

**Non-goals for this plan**
- Custom contracts, sale contracts, disputes and arbitration (§8). Every contract here is one the server can check, so there's nothing to dispute.
- Citizens who hire, post wages or found guilds.
- A word filter, shouting, or channels by zone.
- LLM layers in production beyond #311's spike.
- The React and Love2D clients. Both are frozen.

## 3. Where things stand (audit, 2026-10-21)

**There's no chat.** `meet.rs` and `Choice::Counter` both say so. `Bot::say` (`bot.rs:2012`) is a `println!`. `Bot::greet_people` (`:434`) stops and turns to a person, and says nothing.

**The parts exist:**
- **Interest.** Interest-based fan-out (#256, #275): `GameEvent` broadcasts filtered per socket by `INTEREST_CELL` (300 units, 3×3 cells). Local talk can copy `ExtractionStarted`'s filter (`connection.rs:951`).
- **Notices** that wait for an offline player (`engine/notices.rs`, `player_notices`, #138). Any `ServerMessage` can be queued and is taken once. This is mail's delivery.
- **The market's escrow** (`engine/market.rs`): one box, one transaction per action, and a conservation property test. It can't be reused as it stands, because `close` reads `market_orders`. Its parts can: `escrow_units`, `units::{take_units, insert_stack}`, `currency::*_strands_in`, `market_box` and the harness.
- **The ledger** (C1.7) counts escrowed Strands from `market_orders` only (`ledger.rs:201`). New escrow must be added there, or it reads as drift.
- **The Gazette** (E3.9) composes a day from records, honours a byline opt-out (`players.gazette_named`), and its **notice board** already stands in the hall's forecourt. The board is a client prop: a click opens the paper (`hall.rs`, `cap_board_at`).
- **The hall:** `rules::market::at_the_hall` is where orders are placed (E3.4).
- **Guilds:** guild-district lots (#83, all `guild_hall`) and a `PermissionLevel::Guild` that `container_access` treats as owner-only "until those systems exist" (#80).
- **Citizens:** the `Decider` cascade, with `Question`/`Choice` menus, replay and Jev's prompts. A new task kind is a question, its choices, its rules scoring and its prompt text (the pattern in `sim/run/merchant.rs`).
- **One rate bucket.** Every WS command takes a token from a 60-a-minute bucket per connection (`rate_limit.rs`). Talk on that bucket would freeze a talker's walk.

**In the client:**
- **Names** over heads are HUD text placed by `labels::place` (clash test, fade, `MAX_NAMES` 24), drawn in `hud.rhai`.
- **World text** (#274) is depth-tested but has no wrapping and no clash handling, so it suits signs and doesn't suit bubbles.
- **A focused text field swallows keys** before the game sees them (Flint's text-focus gate), so typing never walks. Enter is `craft`.
- **The TDD's `Contract`** is a generic two-party model (Sale, Employment, Delivery, Custom) with disputes. This plan replaces it with commissions and wages (D5 to D7).

## 4. Design decisions

Chris made the four choices on 2026-10-21: chat's reach, which contracts come first, guilds as the last phase, and standing in words. The rest follow from them.

### D1. Talk's reach (Chris's choice)

`say { channel: local | capitol, text }` and `said { from, name, citizen, channel, text, x, y }`.
- **Local** is heard within 30 units (`rules::talk::HEARD`). It's filtered by interest cell, then by distance.
- **Capitol** goes to everyone.
- **Guild** comes with S4.4.
- **Nothing is stored.** The last 40 Capitol lines live in memory and are sent to a new subscriber.
- **Watchers.** A session that doesn't hold the avatar (E0.8) can read and post Capitol lines, but not local ones: you have to be somewhere to be overheard.

### D2. Text rules

- `rules::talk::clean`: 240 characters, control characters and newlines stripped, runs of whitespace folded, empty lines refused. Letters keep their newlines, up to 1000 characters.
- **Its own bucket:** `Category::Chat`, 8 lines per 10 s per account, so talking never costs a step.
- **No word filter.**
- **Mutes are per account, on the server** (`player_mutes`). A muted line or letter never arrives on any device, and a muted sender can't tell.

### D3. Speech in the client

- **Bubbles.** A line shows over its speaker for 4 s plus 60 ms a character (12 s at most). `PlayerLabel` and `labels::place` carry it, so the clash test counts a bubble's box. A speaker within earshot always gets a bubble.
- **The chat panel** has Local and Capitol tabs, names that open the person's card, and the last three lines when collapsed.
- **Enter opens chat,** except while the crafting or station panel is open, where Enter still crafts.
- **Direct mode.** Opening chat frees the cursor, as any panel does (Controls D2).

### D4. Letters

- `POST /letters` to a player's name. **Postage is 1 Strand**, a new sink that also keeps letters from being spam.
- Delivered with `notices::queue`, so a letter waits for an offline reader.
- Each box keeps 200 letters; the oldest read letters go first.
- Citizens answer letters from their phrasebook (S1.6), and later in their own words (#311).

### D5. Commissions (Chris's choice: delivery first)

A poster asks for N units of an item type, at or over a grade, at a price per unit, due in 1 to 7 game days.
- **The money is down.** The whole price is escrowed when it's posted.
- **The court's fee** is max(5, 1%) + 0.5% of the value (GDD §8.4's filing and escrow fees), all a sink. `rules::court::court_fee` is the one formula; clients preview with it.
- **At the board.** Posting and delivering happen at the hall (`at_the_hall`). Reading, claiming and cancelling work from anywhere.
- **Delivery.** Part deliveries are welcome. Delivered units go to the poster's market box, each keeping its own quality and maker's mark, and each is paid from escrow in the same transaction.
- **Closing.** Cancelling or expiring refunds what's unspent. The fee is kept.
- **Limits.** Five open commissions each, for everyone.
- **No disputes.** The server checks every delivery against the grade.

### D6. Claims are the promise

A deliverer may claim k units, which holds that share for them for up to a game day, never past the due date.
- **The poster's side.** The poster can't cancel a claimed share while the claim stands.
- **Kept or lapsed.** A claim delivered in time is kept. One that runs out is lapsed: its share reopens and the claimant has a breach on their record.
- **Releasing** with more than an hour left is fine. A late release is a lapse.

### D7. Wages, piece rate (Chris's choice: wages second)

An employer offers work at a station of theirs (a recipe) or on their lot (gathering), at a rate per unit, up to a cap, for a term.
- **The escrow** is rate × cap, plus the court's fee.
- **One worker at a time** per offer, open to anyone or named.
- **The shift.** The worker gets use of that station, or that lot's resource, for the shift and nothing else. The output lands in the employer's box, marked as the worker's make. Each unit is paid in the transaction that made it.
- **Stopping.** Either side can stop at any time, and what was made is always paid. So an employer can't breach, and a piece-rate worker owes nothing for stopping.
- **Automation** at that station pauses for the shift.

### D8. Standing (Chris's choice: words, with the number underneath)

- **The record:** commission units and value delivered, claims kept and lapsed, wages paid and earned, weeks of tax in good standing, and foreclosures.
- **The score** (`rules::standing`) adds the good and subtracts each breach's weight. A lapse weighs three kept claims; a foreclosure weighs 10.
- **Only breaches fade.** A breach's weight halves every 7 game days. Nothing good fades.
- **The words**, in five bands: *new in town* (fewer than 5 records), *finding their feet*, *dependable*, *well regarded*, *a pillar of the Capitol*. Over the band, *has let people down lately* shows while a recent lapse still weighs.
- **Who sees what.** Others see the words: on the card, on the board's cards, and beside a claimant. You see your own number, its trend and your history.

### D9. Leaderboards

Weekly, from records the city already keeps.
- **The boards:**
  - **wealth:** Strands, plus deeds at assessed value, plus goods at the last trade price;
  - **maker:** value made;
  - **trader:** market, bazaar and commission volume;
  - **kept their word:** value delivered on claims kept.
- **Scope.** Each board is whole-city and by home zone.
- **Citizens** are listed with their mark, and a people-only filter re-ranks without them.
- **The byline opt-out** shows as "a resident".
- **Where they show:** on the board's Standings page, and in a weekly column in the Gazette.

### D10. Guilds (Chris's choice: the last phase)

- **Founding** (GDD §5 and §8.4): a subscriber founds one at the Capitol for 500 Strands. Citizens can't, having no subscription and no grants.
- **Roles:** leader, officer, member. A roster change costs 1 Strand.
- **The treasury:** officers can withdraw up to a weekly limit the leader sets.
- **Dues** are 50 + 5 a member each week, charged hourly in carried fractions like property tax. At 4 weeks of debt the guild is dissolved and what's left is shared among the members by tenure.
- **A hall** in the guild district costs 5000 from the treasury, and its tax comes from the treasury too.
- **`PermissionLevel::Guild`** means the guild's members, for stations, containers and gathering.
- **A guild channel** comes with the panel.

### D11. Citizens in the commons

- They speak from a rules phrasebook keyed by temperament and seeded, so a replay says the same. #311 is the Claude layer over the same `Speaker` seam.
- They claim and deliver commissions. Ore gatherers count the wild veins in time to deliver, so a good price sends them to the ranges.
- Crafters post commissions for inputs their bids haven't filled.
- They take wages in their own working hours, and join guilds whose charter names their trade.
- Every line and every record carries the citizen's mark.

## 5. Architecture notes

- **Protocol.** New `rules::talk`, `rules::court` and `rules::standing`. `ClientMessage::Say`, `ServerMessage::{Said, LetterArrived}`, and the commission, wage and guild events, each with a fixture. All of them are pure and tested in `protocol`, and both ends call them.
- **Strands.** The court's money moves through `currency::*_strands_in` with new types (`court_fee`, `court_escrow`, `court_payout`, `court_refund`, `postage`, `guild_*`), each with its `StrandFlow` arm (S0.2).
- **The ledger.** Held Strands become the market's escrow plus the court's plus the treasuries (`ledger.rs:201`).
- **Units.** Units in transit sit in one `court_escrow` box. Moves in and out of escrow count no unit flow.
- **Conservation.** One harness (`engine::conservation`, lifted from the market's test) runs seeded lives of commissions, wages and guilds. Strands and units in must equal Strands and units out, plus fees.
- **Telling people.** Notices for anything a person should hear about while away. Events to everyone for the board's live state, and to the parties for their own.
- **Talk is transient** and goes through the event bus only. Letters, commissions, claims, wages, records and guilds are tables.
- **The client.** `capitol_core` gets pure stores: `talk::Log`, `commissions`, wages and guilds. The panels follow the house pattern (`ui/gazette.rs`, `ui/market.rs`) and the theme. The notice board becomes a board of pages: Wanted, Work, Standings and The Gazette.
- **Citizens.** New `Question`s and `Choice`s through the cascade, with rules scoring, Jev's prompt text, and the replay and corpus names. `Speaker` sits beside `Decider`.

## 6. Phases and work packages

### S0: Groundwork (`cm:S0-groundwork`)

- **S0.1 Text rules and a chat bucket** (#378, S). *As built:* `rules::talk` (`clean`, `clean_letter`, `MAX_LINE`, `MAX_LETTER`, `HEARD`, `CAPITOL_BACKLOG`, `TalkError`). A newline or tab in a line folds as whitespace rather than vanishing, a letter folds a run of blank lines to one, and direction overrides (U+202A to U+202E, U+2066 to U+2069) are dropped with the control characters, so a line can't turn a name round; joiners stay for emoji. `Category::Chat` is counted over a 10 s window (`CHAT_WINDOW_SECS`; the others stay a minute), `RATE_LIMIT_CHAT_PER_10S` sets it, and it's keyed by account (`Key::chat(account_id)`), since `POST /letters` has the account and not the player, and an account has one player. `rate_limit::chat_message` is the "You're talking too fast" words for `say` and letters.
- **S0.2 Court escrow groundwork** (#379, M): the box, the transaction types, `rules::court`, the ledger and the shared harness.
- **S0.3 A person's card** (#380, M): `GET /players/{id}/card`, and the card in Flint from a click on a person or a name.

### S1: Talk (`cm:S1-talk`)

- **S1.1 Say** (#381, M): local talk and the Capitol channel on the server.
- **S1.2 Flint: speech over heads** (#382, M).
- **S1.3 Flint: the chat panel and the chat key** (#383, M).
- **S1.4 Mutes** (#384, S).
- **S1.5 Letters and the mailbox** (#385, M).
- **S1.6 Citizens say hello** (#386, S): the phrasebook and the `Speaker` seam.
- **The citizens' voice** (#311): the Claude layer over `Speaker`. It keeps its own decision (which tier, and a spend cap for talk).

### S2: Commissions and wages (`cm:S2-commissions`)

- **S2.1 Commissions on the server** (#387, L).
- **S2.2 Claims** (#388, M).
- **S2.3 Flint: the board's Wanted page** (#389, L).
- **S2.4 The Gazette's Wanted column** (#390, S).
- **S2.5 Citizens answer and post commissions** (#391, M).
- **S2.6 Wages on the server** (#392, L).
- **S2.7 Flint: hiring and working** (#393, M).
- **S2.8 Citizens take wages** (#394, M).

### S3: Standing (`cm:S3-standing`)

- **S3.1 Standing** (#395, M): the record, the hidden score, the words.
- **S3.2 Leaderboards and the week's names** (#396, M).

### S4: Guilds (`cm:S4-guilds`)

- **S4.1 Guilds on the server** (#397, M): registry, roles, roster.
- **S4.2 Treasury and dues** (#398, M).
- **S4.3 Guild halls and the Guild permission** (#399, M).
- **S4.4 Flint: the guild panel, guild talk and tags** (#400, L).
- **S4.5 Citizens join guilds** (#401, S).

## 7. Dependency graph and estimate

```
S0.1 ─┬─► S1.1 ─┬─► S1.2
      │         ├─► S1.3 ───────────────────────────────┐
      │         ├─► S1.4 ◄── S0.3                        │
      │         └─► S1.6 ─► #311                         │
      └─► S1.5 ◄── S0.3                                  │
S0.2 ─┬─► S2.1 ─┬─► S2.2 ─┬─► S2.3 ─┐                    │
      │         │         └─► S2.5  │                    │
      │         └─► S2.4            ├─► S2.7             │
      ├─► S2.6 ─┬───────────────────┘                    │
      │         └─► S2.8                                 │
      │   S2.2 + S2.6 + S0.3 ─► S3.1 ─► S3.2             │
      └─► S4.1 ─┬─► S4.2 ─┬─► S4.3                       │
                │         └─► S4.5                       │
                └─► S4.4 ◄───────────────────────────────┘
```

| Package | Depends on |
|---|---|
| S1.1 | S0.1 |
| S1.2, S1.3, S1.6 | S1.1 |
| S1.4 | S1.1, S0.3 |
| S1.5 | S0.1, S0.3 |
| S2.1 | S0.2 |
| S2.2, S2.4 | S2.1 |
| S2.3, S2.5 | S2.2 |
| S2.6 | S0.2 |
| S2.7 | S2.6, S2.3 |
| S2.8 | S2.6 |
| S3.1 | S2.2, S2.6, S0.3 |
| S3.2 | S3.1 |
| S4.1 | S0.2 |
| S4.2 | S4.1 |
| S4.3, S4.5 | S4.2 |
| S4.4 | S4.1, S1.3 |

**The order of work:**
1. S0.1, S0.2 and S0.3 have no dependencies and can run side by side.
2. Talk (S1) is the first thing anyone sees, and it unblocks #311.
3. S2.1 and S2.6 are the two large server pieces, and they can run in parallel after S0.2.
4. Guilds come last on purpose. S4.1 could start after S0.2, but they mean most once people know each other.

**Estimate:**

| Phase | Packages |
|---|---|
| S0 | 1 S, 2 M |
| S1 | 2 S, 4 M (and #311) |
| S2 | 1 S, 4 M, 3 L |
| S3 | 2 M |
| S4 | 1 S, 3 M, 1 L |

That's about the size of Economy v1's E3 and E4 together.

## 8. Later (outside this plan)

- **Custom contracts, sale contracts, disputes and arbitration.** These are for the GDD's Contract Court, once contracts exist that the server can't check: labour described in words, or goods delivered outside the board.
- **Citizens who hire, post wages, or found guilds** (with a subscription of their own, if that's ever allowed).
- **Shouting and zone channels,** if local and Capitol prove too narrow and too wide.
- **Guild contracts,** a guild's own commissions paid from its treasury, and guild leaderboards.
- **The bureaucracy's other fees and its certification** (GDD §8.4): a plan of their own, which a "Capitol Certified" seal on maker's marks would join to standing.
- **The citizens' voice past #311:** letters to the Gazette, haggling in words over a P2P trade.
- **Reputation in the market:** a standing filter on the board or the bazaar's directory.

## 9. Risks

| Risk | Mitigation |
|---|---|
| **Escrow bugs lose Strands or units.** | Every action is one transaction. One shared conservation harness (S0.2) runs seeded lives of every new flow. The ledger counts every holding, so drift shows on `/admin/ledger`. |
| **Chat floods the socket or the screen.** | The chat bucket (D2), the 240-character limit, the backlog cap, bubbles placed by the clash test, and server-side mutes. |
| **Talk with strangers needs moderation.** | Mutes in this plan. A report and an admin mute are Later; the game isn't public yet. |
| **Commissions are first come, first served, so nobody risks the walk.** | Claims (D6) hold a share for the claimant, and the walk is safe. |
| **Wages leak access** to a station's box or to automation. | Access only for the shift, for that station and recipe's inputs. Automation pauses for the shift. Tests in S2.6. |
| **Standing feels unfair or opaque.** | You see your own number and its history. Only breaches fade, and quickly. A lapse is named only to the person themselves and never in the paper. |
| **Citizens crowd humans off the leaderboards.** | They're marked, and a people-only filter re-ranks. Honesty over hiding. |
| **Guilds of one.** | Citizens join (S4.5). Guilds come last, after people know each other. |

## 10. Verification strategy

- **Pure tests** in `protocol` for `rules::talk`, `rules::court` and `rules::standing`, and fixtures for every new message.
- **Backend:**
  - the conservation harness over seeded runs of commissions, claims, wages and guild life;
  - `sqlx::test` for each route's refusals;
  - `GET /admin/ledger` with no drift.
- **Client:**
  - capitol-core store tests;
  - headless egui tests of each panel's actions;
  - offline captures with `--talk-sample`, `--chat-sample`, `--card-sample`, `--board-sample`, `--guild-sample` at 1280×720 and 1920×1080, and the board at dusk.
- **Citizens:**
  - `replay --layers rules` reproduces every logged run exactly;
  - live towns at 10× for the named scenes in each issue: three commissions answered, an ore gatherer walking to the ranges, a shift worked, a guild joined.
- **[H] Chris plays a town** with talk, a commission of his own and a hire. The question is whether the city feels like a society.
