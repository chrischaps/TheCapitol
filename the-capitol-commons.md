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
- **The ledger** (C1.7) counts escrowed Strands from `market_orders` only (`ledger::held_strands`, with the court's part in `court_held`). Each new table that holds Strands adds its sum there and points at it. New escrow must be added there, or it reads as drift.
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
- **Enter opens chat,** except while the crafting panel or an open station is set to a recipe, where Enter still crafts. (As built, S1.3: the crafting panel is open from the start, so "while it's open" would leave Enter nothing to open chat with.)
- **Direct mode.** Opening chat frees the cursor, as any panel does (Controls D2).

### D4. Letters

- `POST /letters` to a player, by id (from a card's Write or a letter's Reply), since names aren't unique ([#428](https://github.com/chrischaps/TheCapitol/issues/428)). **Postage is 1 Strand**, a new sink that also keeps letters from being spam.
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
- **Strands.** The court's money moves through `currency::*_strands_in` with new types (`court_fee`, `court_escrow`, `court_payout`, `court_refund`, `postage`, `guild_*`), each with its `StrandFlow` arm (S0.2). The fees, postage and dues are sinks. Everything into or out of a holding is `Escrow`, a payout to a deliverer or a worker and a guild deposit or withdrawal included: the holding is counted in the world's Strands, so only its leaving explains the money, and a `Transfer` would show as drift.
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
- **S0.2 Court escrow groundwork** (#379, M): the box, the transaction types, `rules::court`, the ledger and the shared harness. *As built:* the `court_escrow` box (`20261203_001_court_escrow.sql`, one, by a unique index). Nine new `TransactionType`s, and `currency::tests::every_type_is_one_the_ledger_knows` holds the server's and `ledger::TRANSACTION_TYPES` to the same list. `court_payout`, `guild_deposit` and `guild_withdrawal` are `StrandFlow::Escrow`, not `Transfer` as first planned: each moves money into or out of a holding the ledger counts (see §5). `rules::court`: `filing_fee`, `escrow_fee`, `court_fee` (value 0 costs 5). The ledger's `escrow_strands` is `ledger::held_strands`, the market's plus `court_held` (0 until S2.1). `engine::conservation` (test-only): `strands_accounted` (balances + held + `fees_kept`, where S2.1 and S2.6 add the court's fee column), `units_of_type`, `every_unit`, `balances_of` and `assert_audited`.
- **S0.3 A person's card** (#380, M): `GET /players/{id}/card`, and the card in Flint from a click on a person or a name. *As built:* `routes/players.rs` serves `/players/{id}/card` and `/players/by-name/{name}` (`PlayerCard`: `since` is the account's game day, `home` the zone id of the oldest plot, `standing` is `rules::standing::NEW_IN_TOWN` until S3.1, `guild` is `None` until S4.1, `mine`). Names aren't unique among people, so by name finds a citizen first, then whoever took it first; letters by name (S1.5) need that settled ([#428](https://github.com/chrischaps/TheCapitol/issues/428)). In Flint, `capitol_core::card` holds the card wanted and its call (pure, with the words: `time_in_city`, `home_words`), and `ui/card.rs` draws it top right, edged in its person's lantern (amber, a citizen's pale, gold for yours), opening at once on the name over their head while the call is out. A plain click on a person is `input.rhai`'s new priority after the trade's shift-click (`cap_open_card`); the Gazette's producers and shopkeepers are links to theirs. The citizen's lozenge moved into the theme (`theme::citizen_mark`), filled pale on the panels and inked on the paper. `--card-sample [citizen|person|mine]`.

### S1: Talk (`cm:S1-talk`)

- **S1.1 Say** (#381, M): local talk and the Capitol channel on the server. *As built:* `say { channel, text }` and `said { from, name, citizen, channel, text, x, y }` (`citizen` left out for a person, as everywhere else), `Channel` `local | capitol`. `engine::talk` holds it: `Talk::say` cleans, refuses a watcher's local line ("Play here to talk to people nearby"), counts the chat bucket and sends `GameEvent::Said`; `talk::hears` is each socket's test, the interest cell and then `HEARD`, read off the last tick's `Positions` (`Positions::position_of`). The backlog is `AppState::talk`, in memory. Capitol lines are numbered under the backlog's lock, so a socket that has just taken the backlog skips the copy of a line still on its way to it. Differs from the issue: `say` isn't counted in the connection's bucket as well, only in the chat one, so talking never costs a step (D2). A watcher hears no local lines either, its own avatar's included: a socket tracks `presence` for it. There's no admin camera on the server (`sees_everyone` means everyone in the world is in sight, not an admin), so an admin hears local talk as anyone does.
- **S1.2 Flint: speech over heads** (#382, M). *As built:* `capitol_core::talk::Log` keeps 200 lines a channel with their unread counts, and each speaker's latest local line for the bubbles (`bubble`: the text, its alpha and its age; 4 s plus 60 ms a character, 12 s at most, a 0.15 s fade in and a 0.6 s fade out). The `Session` keeps one, fed from `StateEvent::Said`, on a clock of its own, so a capture's game time drives the bench's. `PlayerLabel.line` is a `Speech` (text, alpha, age). `labels::place` wraps it (`wrap`, 28 by 3, an ellipsis past that) into a `Bubble` over the name. Differs from the issue: **only local lines hang over heads**; a Capitol line has no place, and the backlog would otherwise pop forty bubbles on joining. **The names are placed first and the bubbles after, newest first**: a bubble slides aside up to a third of its width to clear a name or another bubble, or else is lifted over it. So no one's name gives way to someone else's bubble, two people talking side by side both show, and a new line hangs lowest and pushes the older ones up, as a chat does. A speaker within earshot ranks ahead of the silent and is kept past `MAX_NAMES`, and **a speaker whose name has no room** (beside yours, from street zoom) still gets their bubble, with their name, and a citizen's lozenge, as its caption. `hud.rhai` draws every tail, then the slips, then the names, in the theme's `PAPER`, `PAPER_EDGE`, `INK` and `INK_DIM` (handed over by `cap_name_tags`, so the HUD has no colours of its own). The `murmur` one-shot plays at the speaker (`Soundscape::murmur`), a Capitol line's quieter and from nowhere, and lines in a burst murmur once (0.35 s apart at most). A reconnect is sent the Capitol backlog again, and `said` has no number to tell a repeat by, so `Log::rejoined` drops repeats by speaker and text for 5 s ([#430](https://github.com/chrischaps/TheCapitol/issues/430)).
- **S1.3 Flint: the chat panel and the chat key** (#383, M). *As built:* `ui/chat.rs`, bottom left, 340 wide, narrowing to 250 to fit between the crafting panel (it moves beside it when that reaches down into its corner) and the inventory or station panel; the log is a third of the window, 80 to 190 tall. Its lines are S1.2's `talk::Log`, the one the bubbles read; the panel reads the open tab (`Log::read`), and `Log::own` keeps your own line from counting as unread. The input side is `capitol_core::chat::Chat`: the draft, the lines in flight, and the last refusal, which brings an unsent line back to an empty input with the reason in `theme::error_box` for 10 s (an `error` whose `command` is `say`; the oldest line in flight is the one refused). Enter sends and keeps the input focused; an empty Enter or Esc closes it, and Esc closes it after a click on the world too. Focus is asked after the input is drawn, so the Enter that opened it isn't read as a send. Closed, the last three lines of either channel show over the controls hint on a faint backing, a Capitol one marked so, bright for 12 s and fading over 8. Names in the log open the card (`Session::open_card` with the name and the mark as its hint); your own is gold and not a link; a citizen's has the lozenge in the lantern's pale. Hover a line for its time. The header has a Chat chip with a gold count of the lines unread on both channels. `chat` is its own action on Enter and NumpadEnter beside `craft`, and the extension decides (`ui::chat::enter_means`). **Differs from the issue:** Enter crafts only with a recipe set (see D3), since the crafting panel is always open. Direct mode has no captured cursor yet (T1.4, #322), so there's nothing to free; while the input has focus Flint keeps every key from the game. `--chat-sample` shows both tabs, a refusal and the hint card, with the crafting and inventory panels open; `session::tests::a_local_line_is_heard_nearby_and_not_across_town` (ignored, live) is the round trip with three accounts.
- **S1.4 Mutes** (#384, S). *As built:* `player_mutes (player_id, muted_id, created_at)` (`20261204_001_player_mutes.sql`, a `CHECK` against muting yourself), kept by `crate::mutes` (`of`, `list`, `set`). `PUT`/`DELETE /players/{id}/mute` answer with the whole list as it stands (`MutesResponse` of `MutedPlayer { id, name, citizen }`, newest first), and so does `GET /player/mutes`, so a client never patches its own copy. Muting twice or unmuting someone not muted is no error; muting yourself is a 400, no one a 404. `PlayerCard` gains `muted`, so the card is right on every device. Each socket reads its player's mutes on its first `position` subscribe, on `GameEvent::MutesChanged { player_id }` (which `mutes::set` sends) and after a lag resync, since the change may be among what it missed. `talk::hears` takes the set and drops a muted speaker's lines on both channels, and `Talk::backlog` leaves them out of the backlog. In Flint, `capitol_core::mutes::Mutes` holds the list (asked for once signed in, again after a reconnect and as Settings opens), the card has a quiet Mute or Unmute under Trade with "Muted: their words don't reach you", and Settings ends with a Muted section, each name with Unmute. A muted speaker's bubble is never drawn because the line never arrives; one already up when you mute stays its few seconds. For S1.5: a letter from someone in `mutes::of(recipient)` is refused in words that name no one.
- **S1.5 Letters and the mailbox** (#385, M). *As built:* letters are addressed by the reader's id, not a name (`SendLetter { to, body }`), so they don't wait on #428: every way to write starts from someone you can see (a card's *Write*, a letter's *Reply*). `letters` (`20261206_001_letters.sql`) keeps each side's copy (`kept_by_sender`, `kept_by_recipient`; the row goes when neither does). `engine::letters::send` writes the letter, takes `rules::talk::POSTAGE` (`postage`, a sink) and queues `letter_arrived` (`notices::queue`, keyed by the letter's id) in one transaction, then trims each box to `talk::MAILBOX`: the in box keeps its unread first, so its oldest read go first, then its oldest unread; the out box loses its oldest. `GET /letters?box=in|out&before=<id>` pages 50 at a time, newest first, with the in box's `unread`; `POST /letters/{id}/read` and `DELETE /letters/{id}` answer `{ unread }`. A writer is never told whether a letter was read (the out box has no `read_at`), so a mute can't show as a letter never read. A letter to someone who's muted you (`mutes::of`, S1.4) is written and paid for into your out box only (`send`'s `muted`), so nothing tells you: not a refusal, not a letter left unread. `POST /letters` counts against the chat bucket after the body is cleaned, and its 429 says `chat_message` (`rate_limit::too_many_saying`). `talk::first_line` is the gist: past a salutation ("Dear Brin,"), cut at a word to 60, an ellipsis unless a sentence ends. In Flint, `capitol_core::mail::Mail` keeps both boxes, the letter open, the draft and one call at a time (pure; the in box is fetched at sign-in for the count, the out box when first looked at; reading and dropping show at once), and `ui/mail.rs` draws it top centre: Received and Sent, sealed envelopes (`theme::envelope`, `theme::wax_seal`) for the unread, a letter on the Gazette's paper sealed with its writer's initial, and a pen on the same paper. L or the header's Letters chip (an envelope and the count while letters wait) opens it, *Write* on the card opens a letter, and `letter_note` is the toast. `--mail-sample [in|letter|write|sent]`. Since #447 a letter can also come from one of the city's offices (the Registrar's, on the road in): `sender_id` null and `from_office` set, no postage, no copy kept, no reply, once an account (`office_letters`); `Correspondent.office` and `letter_arrived`'s `office` say so, and Flint seals it with the Capitol's tower (`--mail-sample registrar`).
- **S1.6 Citizens say hello** (#386, S): the phrasebook and the `Speaker` seam. *As built:* `citizen::phrases`. Eight occasions (a greeting, opening a trade, accepting, countering, declining, a sale at the stall, an answer to being named, a letter's reply), in three voices by sociability (`Band`: reserved under 0.4, warm over 0.6), six lines each. Lines carry `{name}` (the first name) and `{hello}` (good morning, afternoon or evening by the server's hour), and never name goods, since an item's name won't sit in a sentence the same way twice. `Phrasebook` picks by a whim of the citizen's seed, the occasion and the moment (who, and the game minute), so a replay says the same. `Speaker` is the seam #311 takes. `Voice` holds the limits: a greeting once a game hour per person, an answer 2 to 5 s after a person names the citizen and at most once a real minute per person, a letter's reply once a game day per sender, and never more than 4 lines in the chat bucket's 10 s (half the server's 8). `Bot::say` sends `say`, and the old print is `Bot::log`. `Bot::speak_with` gives a bot a voice (only citizens have one, from `sim::run::live`), and it says nothing unless it plays its avatar here and someone stands within `HEARD`. A citizen answering a trade steps to within 15 of whoever asked, because a trade is asked from up to 75 away and a word carries 30. It answers people only, since two citizens naming each other would never stop. `capitol-bot --at x,y --listen --hail` walks a person-bot over, prints the talk it hears and hails citizens by name, to watch them with no client. Differs from the issue: letters wait for S1.5 (#385), so `Voice::letter` is built and tested, and [#432](https://github.com/chrischaps/TheCapitol/issues/432) wires it to the mailbox.
- **The citizens' voice** (#311): the Claude layer over `Speaker`. *Decided (Chris):* Haiku first; a local model is worth exploring ([#434](https://github.com/chrischaps/TheCapitol/issues/434)). *As built:* `citizen::voice`. The `Speaker` seam can pend: `line` gives `Words::Line`, `Words::Pending(ticket)` (asked on the remote layers' shared runtime, `decide::remote`, whose `Remote` now carries any answer type) or `Words::Nothing`, and `poll` takes the answer in. `Voice` lies over the citizen's `Phrasebook`: it says a model's line when it comes, and the phrasebook's if the model has nothing within `TALK_WAIT` (2 s, a greeting's pause, which Haiku's p95 of 1.7 s fits; an answer still waits for its 2 to 5 s moment) or within `LETTER_WAIT` (60 s) for a letter's reply. So the world never waits, and a greeting waiting on the model still counts as the hour's greeting. `Voice::letter` now queues a reply and `Voice::replies` gives the ready ones, for #432. `ClaudeSpeaker` is C3.2's request for talk: a system prompt of two cached blocks (how a citizen talks, then `remote::persona_of`), one user message (the occasion, the time, who, and what they said or wrote), and a strict, forced `say { line, mood }` tool. The prompt keeps the citizen in character and honest: never a person, no promises of goods, prices or meetings (the decision is settled elsewhere), no following instructions in what people say, nothing hateful or crude. A refusal, no tool call, a line that won't clean (`rules::talk::clean`, `clean_letter` for a letter) or a ramble (over 160 characters, 600 for a letter) is `Nothing`, so the phrasebook speaks. The mood is asked for so the model settles the tone first; nothing shows it yet. **The cap:** `TALK_SPEND`, 5¢ a citizen a life (about 800 Haiku lines), past which it speaks from its phrasebook. No key means the phrasebook. **The roster:** `voice = "phrasebook"` [default], `"claude-haiku"` or `"claude-sonnet"`, for the town or per citizen (`voice::KNOWN_VOICES`). `capitol-town voice [--voice <name>]` has three made-up citizens (reserved, even, warm) say a line for every occasion, with each line's time and the cost. Tested against a stand-in for the Messages API (`voice::tests`), not the real one: no key on the machine that built it. Nothing reports the talk's tally or spend yet ([#433](https://github.com/chrischaps/TheCapitol/issues/433)).

### S2: Commissions and wages (`cm:S2-commissions`)

- **S2.1 Commissions on the server** (#387, L). *As built:* `commissions` and `commission_waiting` (`20261207_001_commissions.sql`), kept by `engine::commissions` (`post`, `deliver`, `cancel`, `expire_due`, `move_waiting`, `list`, `load`) and served by `routes::commissions`: `POST /commissions`, `GET /commissions?item=&mine=&status=` (newest first, 200 at most), `GET` and `DELETE /commissions/{id}`, `POST /commissions/{id}/deliver { item_id, qualities?, quantity? }`. `rules::court` gained `validate_commission`, `commission_value`, `CommissionStatus`, `MAX_OPEN_COMMISSIONS`, `MIN_DUE_DAYS`/`MAX_DUE_DAYS` and `DAY_SECS` (a game day is a real one, as rent and order lifetimes are), and `delivery`, which picks the units: named ones must all meet `min_quality` and be held (past what's wanted, the lowest of them go); unnamed, the lowest that meet it go, so the deliverer keeps their best. Differs from the issue: the request names its stack `item_id`, as `PlaceOrder` does, and takes an optional `quantity` as most to deliver. **A full market box never turns a deliverer away:** the units wait as a stack in the court's escrow box (S0.2's), a `commission_waiting` row each, and the market loop moves them on every 10 s as the box has room (`move_waiting`), so that box is for this, not units in transit. A delivery past `due_at` is refused before the sweep gets to it. Expiry rides the market loop each second (`court_cycle`, span `loop.court`). Wire: `commission_posted`, `commission_updated` and `commission_closed` to every socket (`GameEvent::Board`), and to the poster `commission_delivered` (who, their mark, the units, the pay and `left`; `left` 0 is the filled notice, so there's no separate one) and `commission_expired`, both queued with `notices::queue` and sent live through a new generic `GameEvent::Notice` (`BazaarNotice` could fold into it). `ledger::court_held` sums the escrow, and `conservation::fees_kept` adds the court's fees. Tests: the seeded `posts_deliveries_cancels_and_expiries_conserve_strands_and_units` (with a crowded box, so units wait), `the_ledger_holds_the_courts_escrow_without_drift`, and the refusals. In Flint, the two notices are `StateEvent::CommissionDelivered` and `CommissionExpired`, toasted by `Session::note_commission` as a bazaar sale is, since a notice sent live is gone from the queue; S2.3's store keeps the board's three messages.
- **S2.2 Claims** (#388, M). *As built:*
  - **The table.** `commission_claims` (`20261218_001_commission_claims.sql`), with one standing claim per person per commission held by a unique index, and `ended_at` and `warned` beside the issue's columns. `rules::court` holds the pure part: `ClaimStatus`, `CLAIM_SECS` (a day), `CLAIM_GRACE_SECS` (an hour), `claim_until`, `unclaimed`, `deliverable`, `validate_claim` and `release_status`.
  - **Routes.** `POST`/`DELETE /commissions/{id}/claim` (answering `ClaimResponse`, the claim and the commission after it), and two the issue didn't name, for S2.3's page: `GET /commissions/claims` (your standing claims) and `GET /commissions/{id}/claims` (all of one commission's, standing first). `Commission` gains `claimed` (what standing claims still owe) and `withdrawn`, both left off the wire at 0 and false.
  - **Holding.** A claim "stands" until it ends, but holds its share only until its `until`: one past it that the sweep hasn't reached yet holds nothing. `deliver` caps a delivery at `deliverable` (the unclaimed part plus the deliverer's own share) and counts it against their claim first; delivered in full, the claim is kept at once. With nothing open to them, a deliverer is refused `Claimed`.
  - **Withdrawing (differs from the issue's "cancel the unclaimed part, and the rest once the claims end").** A cancel while claims hold their shares *withdraws* the commission in one step: the unclaimed part is refunded at once, nobody new may claim or deliver, each claim's undelivered share is refunded as it ends short, and the commission closes as `cancelled` when the last claim ends (or `filled`, if the claims fill it). So the poster never has to come back and cancel twice, and a lapsed share never reopens on a commission its poster took down. A withdrawn commission's escrow is the price of what its claims still owe. A second cancel is refused `ClaimsStand`.
  - **The sweep.** `claims_due` runs in the market loop's `court_cycle`, before `expire_due`: it lapses each claim past its `until` in a transaction of its own (the commission locked first, as everywhere), and warns each claimant once as the last hour starts (`claim_due_soon`). A claim made with an hour or less to run isn't warned. An expiry or a cancel lapses any claim still standing.
  - **Telling people.** `commission_claimed` to the poster, `claim_due_soon` to the claimant and `claim_lapsed` to both, all queued with `notices::queue` and sent live through `GameEvent::Notice`. A release in good time tells no one but the board (`commission_updated`). `Closed` carries `notices` now, not one `notice`.
  - **The breach** is `status = 'lapsed'`, a late release included; S3.1 reads it from `commission_claims` (`idx_commission_claims_claimant`).
  - **Tests:** the issue's six, plus an expiry lapsing a standing claim and the hour's warning. The seeded conservation run now claims, releases and withdraws too (160 steps; its crowded box empties half as often, so units still wait in the court). The Flint client parses the three notices; S2.3 (#389) toasts them.
- **S2.3 Flint: the board's Wanted page** (#389, L). *As built:*
  - **The store.** `capitol_core::commissions::Board` (pure): every open commission, newest first, from `GET /commissions` at sign-in and then kept from `StateEvent::Board` (the three board messages, as they come); your standing claims from `GET /commissions/claims`, fetched again after any claim notice or delivery. One call at a time from `wants`, what you do queued oldest first. `can_fill(pack, me)` is someone else's commission with some open to you (`unclaimed` plus your own claim's share, `deliverable`) and a stack in the pack holding a unit at its quality. `Draft` previews `court_fee`, the escrow and the total, and `stopped` says what blocks a post (the court's bounds, five open, your Strands). `quote` previews a delivery through `rules::court::delivery`. The words are there too (`grade_words`, `time_left`, the toasts).
  - **Walking to act.** `Session::board_act` sends a claim, a release or a cancel at once. A post or a delivery away from the hall is `hold`-ed while the session walks to the door (a second `hall::Visit`), goes on arrival, and is dropped with "You walked off…" if you walk elsewhere first.
  - **The panel.** The Gazette's panel is now the notice board's, with two tabs: *Wanted* (`ui/board.rs`) and *The Gazette* (unchanged). A click on the board opens Wanted, or the Gazette while an edition is unread; G opens the Gazette, then folds it. Wanted is cork in an oak frame, with each commission a paper card held by a brass pin, every other one a little lower: the item and grade, the price each and in all, how many of how many, when it's due, the poster (a link to their card), and the claims as a pencilled tally (`theme::cork_flecks`, `pin`, `tally`, `paper_widgets`). Your own cards are edged in gold. Filters are *Everything*, *I can fill* and *Mine*, plus an item. Post and Deliver each open a larger card: fields on the left, the court's numbers and the button on the right, stacked where narrow. A delivery picks its stack and, for a stack of several qualities, its units with the trade window's quality rows (`slots::pick_rows`).
  - **In the world.** `cap_hall()` gains `fill`. The glint over the board's lamp also lights when there's something you could fill, and the hover line says "2 wanted that you could fill". The prompt is now "The Notice Board".
  - **Toasts.** `commission_claimed` (to the poster), `claim_due_soon` and `claim_lapsed` (yours, or one on yours) are now `StateEvent`s and toast through `Session::note_claim`. Deliveries and expiries already did (S2.1).
  - **Differs from the issue:** "due in game days and hours" is the court's days, which are real days (`DAY_SECS`, S2.1), so a card reads "due in 3 d 5 h". The poster's standing waits for S3.1. `Bot::post_commission` is new, for the live test and for S2.5.
  - **Tests and bench:** `cargo test -p capitol-core commissions` (11), `cargo test -p capitol-client board` (the page wide and narrow, each form, and post and deliver clicked headless), `the_board_turns_between_its_pages`, and the ignored live `a_commission_is_posted_claimed_and_delivered` (a bot posts; the session claims, walks to the hall, delivers; the fiber reaches the poster's box). `--board-sample [list|post|deliver|glint]`.
- **S2.4 The Gazette's Wanted column** (#390, S). *As built:* the board needed records it didn't keep: `commission_deliveries` (each delivery's deliverer, units, pay and time, written in `deliver`'s transaction) and `commissions.closed_at` (set by a fill, cancel or expiry), in `20261225_001_commission_records.sql`. `gazette::gather` reads a `Board`: each commission open at the day's end with what it still wanted then (deliveries since are added back), the day's expiries, deliveries, posts and fills, and claims ended kept or lapsed. `compose` makes `Gazette::wanted: Option<GazetteWanted>`, `None` on a day with nothing on the board or done there: `wanted` (by item, the most Strands down first, 5 at most), `came_through` (the delivery that paid most), `gone_begging` (3 at most), the counts and `story`, its prose, inside the column rather than as a `*_story` field beside it. A poster or deliverer who keeps out of the papers is "a resident" (a citizen "a citizen"); a lapse is only counted, and the records hold no claimant to name. The column doesn't compete for the front page. In Flint it's a band under the three columns, *Wanted at the Hall*: the story beside the wants pinned up as slips, then who came through (names open their cards), what went begging and the claims kept.
- **S2.5 Citizens answer and post commissions** (#391, M).
- **S2.6 Wages on the server** (#392, L). *As built:*
  - **Tables and routes.** `wage_offers` and `wage_shifts` (`20261216_001_wages.sql`), with one running shift per offer and one per worker held by unique indexes. `engine::wages` and `routes::wages`: `POST/GET /wages`, `GET/DELETE /wages/{id}`, `POST /wages/{id}/take`, `DELETE /wages/{id}/shift`, all from anywhere.
  - **Rules.** `rules::wages` holds the bounds (the market's rate and quantity, 1 to 7 days) and `payable`. An employer has at most five offers open (`MAX_OPEN_OFFERS`), as with commissions. A craft offer's recipe must be made at that kind of station (a hand recipe at any). A gather offer's resource must grow within the lot's bounds: its bounding box, not its outline ([#435](https://github.com/chrischaps/TheCapitol/issues/435)).
  - **Shift access.** `container_access::check_station_use` admits a worker on a shift to open the station and craft there, but only the offer's recipe (`wages::recipe_refusal`), and the shift wins over a public plot's access. Moves in and out of the box and the tool slot stay `check_station_access`'s, so a worker can't take anything by hand. `container_access::gathering_terms` admits them to that resource on that lot.
  - **Crafting on a shift.** The craft carries its `ShiftWork` through `Command`, the op and `FinishedCraft`. Its inputs come from the station's box, whoever deposited them, named or all in slot order. `finish_craft` checks they're still in the box, puts the output there as the employer's marked as the worker's make, and pays in the same transaction (`wages::pay`).
  - **Gathering on a shift.** A gather goes into the employer's market box, also paid in its transaction. If the shift ended while the work ran, nothing is written.
  - **The cap.** The last craft or gather may make more than the cap leaves. The employer gets all of it, and the units over the cap are unpaid.
  - **A full market box.** If the employer's market box has no room, the gatherer keeps the units, unpaid, and the shift ends `box_full`.
  - **Automation.** It pauses `shift` (`PauseReason::Shift`, read from `owners_of`) whatever else it waited on, and resumes after.
  - **Endings.** `run_wage_loop` sweeps every 5 s:
    - `expired` at the term;
    - `gone` when the station goes or changes hands, or the lot does (the FK sets the id NULL);
    - `away` after 5 minutes off. The clock is in the loop's memory, so a restart gives a worker the 5 minutes again.
  - **Telling people.** Notices go to the employer (`wage_taken`) and to both sides (`wage_shift_ended`), queued and sent live through `GameEvent::Notice`. `wage_offered`, `wage_updated` and `wage_closed` go to everyone, and `wage_paid` quietly to the worker (`GameEvent::Wages`).
  - **The books.** `ledger::court_held` counts the unspent escrow, and `conservation::fees_kept` the fees. The Flint client parses the six messages and does nothing with them until S2.7.
- **S2.7 Flint: hiring and working** (#393, M).
- **S2.8 Citizens take wages** (#394, M).

### S3: Standing (`cm:S3-standing`)

- **S3.1 Standing** (#395, M): the record, the hidden score, the words.
- **S3.2 Leaderboards and the week's names** (#396, M).

### S4: Guilds (`cm:S4-guilds`)

- **S4.1 Guilds on the server** (#397, M): registry, roles, roster. *As built:* `rules::guild` holds what both ends check: `clean_name` (cleaned as a line of talk, 3 to 40 characters with a letter, no `RESERVED` word in any case or with an s: Capitol, Gazette, Exchange, Court, Registry, Bazaar, Bureau, Government, Admin, Administrator, Moderator, Official, Staff, Citizen), `clean_tag` (2 to 5 ASCII letters, kept in capitals, not in `RESERVED_TAGS` or a reserved word), `clean_charter` (a letter's rules, may be empty), `FOUNDING_FEE` 500, `ROSTER_FEE` 1, `LEAD_LAPSE_DAYS` 28 (real days, as subscriptions run), and `GuildRole` with `may_invite`, `may_expel` and `may_set_roles`. `20261215_001_guilds.sql`: `guilds` (name unique ignoring case and tag unique among guilds that stand, `leader_since`, `status` active or disbanded), `guild_members` keyed by the player (one guild each; one leader a guild by an index) and `guild_invites` (one waiting per guild and player, `kind` invite or apply). `engine::guilds` does each write in one transaction holding the guild's row, and returns a `Told` (who hears `guild_changed`, the notices queued, who paid); `routes::guilds` sends them and answers every write with `MyGuild`, as mutes do. **Accepting is the same act from the other side:** inviting someone who applied takes them in, and so does applying to a guild that invited you; `DELETE /guilds/{id}/invites/{player}` declines or withdraws. Founding needs the `Subscriber` extractor and the engine checks again, so a citizen never can; it's at the Exchange hall (`routes::market::at_the_exchange`). The roster's Strand is a `guild_fee` from whoever completes the change: the one accepting a join, the expeller, the leader setting a role. **Differs from the issue: leaving is free**, so nobody is kept in a guild they can't afford to leave, and a leader leaves only as the last member, which disbands the guild and frees its name and tag. Handing on the lead needs the new leader to have a subscription (GDD §5); the old leader becomes an officer. Each notice is one `guild_notice` (`kind`: invited, applied to the officers and leader, expelled, lead), queued so it waits for anyone away; `guild_changed { guild_id }` goes to the members and anyone just out. The lapse check runs hourly with the subscriptions' (`pass_lapsed_leads`): a leader not subscribed now, lapsed since the later of their subscription's end and `leader_since`, for 28 days, gives the lead to the longest-serving officer, else member, **a subscriber first within each**, so the lead doesn't rotate among the absent while someone who could invite is there. A guild of one keeps its leader. `GET /guilds/{id}` lists members by role then tenure, each with `standing::NEW_IN_TOWN` until S3.1, and who's waiting only to its officers and leader. `PlayerCard.guild` is the guild's name. In Flint, `capitol_core` reports both messages as `StateEvent`s for S4.4 to draw.
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
