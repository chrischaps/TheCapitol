# The Capitol: Direct Control Plan

*2026-10-21. Plan issue #314. Work packages #315 to #329, follow-ups #330 to #332. Issues are labelled `controls`, with phase labels `ct:T0-groundwork` to `ct:T3-decide`.*

## 1. Vision

The Flint client sees the city the way a map-reader does. You click the ground, your avatar walks there, and the camera rides above you on one dial from the shoulder to the map. That suits a trader who's watching the rings. It suits someone walking the bazaar's lane less well, or someone standing at a stall with the hall's lamps overhead, or someone climbing a valley in the wilds with the ranges going blue ahead.

**Direct control** is the second way to be in the city:
- an over-the-shoulder **action camera**;
- **WASD or a pad** to walk, and the mouse or right stick to look;
- **E or Ⓐ** to act on whatever is in front of you.

It's built as a mode you can switch to and from at any moment. Both modes are played side by side, and then we decide whether to keep one or both (T3).

The question the A/B answers is this: **which way of moving makes the economy feel like a place you live in, and not one you manage?** Neither mode gets an advantage. The walk, the water, the slopes and the reach are the same in both.

## 2. Goals and non-goals

**Goals**
- A Direct mode in the Flint client:
  - camera-relative WASD;
  - mouse-look over the shoulder;
  - a highlighted focus with E to act;
  - a cursor that's freed for panels.
- The pad plays: walk, look, act, zoom, open and close panels.
- A server walk on a heading (`steer`), predicted on the client with no visible slide on stopping or turning.
- One shared walk step, so the server and the prediction agree at banks, canals and the world's edge. This fixes click mode's canal prediction as well.
- A live switch (F8) and local numbers, so the comparison is fair.

**Non-goals for this plan**
- Walls. The server doesn't block buildings. That's F1 (#330), a decision for Chris.
- Moving round inside a panel with the pad. That was F2 (#331), built after the plan: see §8.
- The React and Love2D clients. Both are frozen.
- Jumping, sprinting, or any speed that click mode doesn't also have.
- Mouse-look in Click mode.

## 3. Where things stand (audit, 2026-10-21)

**The server knows only "walk to a point".**
- `ClientMessage::Move { x, y }` (`protocol/src/ws.rs:33`) becomes `Command::Move`. It's applied in `tick.rs:351-366`, where it cancels extraction and crafting and sets the destination.
- `engine/movement.rs:27` steps toward the destination through `Ground::step` (`protocol/src/rules/ground.rs:304`), then `validate_movement` for the moat and canal rings.
- There's no direction, stop or facing message. Bots stop by moving to where they stand (`bot.rs:443`). `PlayerPosition` is `{id, name, x, y, citizen}`, with no sequence number.

**One rate bucket.** Every WS command takes a token from a 60-a-minute bucket per connection (`backend/src/rate_limit.rs:128`, `connection.rs:224-233`). Steering by re-sending `move` would use it up in about 6 s.

**Prediction misses the rings.** `capitol_core::motion::Motion` predicts with `Ground::step` and reconciles against a 500 ms trail with a 4-unit tolerance (`motion.rs:635`). The moat and canals are tested only by `check_water_crossing` (`rules.rs:367`), which the prediction never runs. The world edge is checked only on `move`'s destination.

**The camera.**
- `scripts/camera.rhai` is one dial, 4.5 m to 880 m. Since E3.13 its near end stands over your right shoulder, pulls in out of the buildings (their walls and roofs, as ray targets since #293), and swings behind you as you walk.
- WASD pans the camera, Q/E orbit, R/F and the wheel zoom.
- The view drifts toward the Capitol from 166 m out and turns north up from 366 m.

**Input.**
- `config/input.toml` has no pad bindings.
- E is both `orbit_right` and the engine's `interact`. F9 is an engine debug key.
- Escape with nothing open quits the app.
- A ground click goes through `input.rhai`'s priorities to `cap_move`. `Session::move_to` cancels every walk-then-act plan.

**The engine** has these:
- gilrs pads;
- `Binding::GamepadAxis`/`GamepadButton`;
- `action_value`, `mouse_delta_x/y` (raw, captured only);
- `set_cursor_captured`;
- an input gate that holds keys from the game while a text field has focus.

It lacks these:
- **The hidden cursor still acts.** While the cursor is captured it's confined, not locked, so its moves and clicks still reach egui and `pick()`.
- **Pads and raw mouse motion bypass the gate,** and pads are read while the window is unfocused.
- **There's no `is_cursor_captured()`.**
- **Input scripts can't produce a raw mouse delta.**
- **Rebinds are saved to a file that's never loaded** (#332).

**Collision.** Nothing on the server checks buildings, stations or other players.

## 4. Design decisions

All decided with Chris on 2026-10-21. D2, D6 and D7 were his choices; the rest follow from them.

### D1. A/B behind one switch

`[controls] mode = "click" | "direct"` in `save/settings.toml`, set in Settings or swapped live with **F8**.
- Click stays the default until T3.2.
- `--controls` wins over the file. Captures, offline runs and input scripts are Click unless `--controls` says otherwise, so every capture recipe stays reproducible.

### D2. The action camera (Chris's choice)

In Direct:
- **Looking.** The cursor is captured and the mouse turns the view, directly and unsmoothed. The pitch moves within −10° to +35° of the dial's.
- **Walking.** WASD walks relative to the camera's yaw, and the avatar faces the way it walks.
- **Acting.** E, or a left click, acts on the focus (D5).
- **The cursor.** Holding Alt frees it, and so does any open panel. Escape closes the top panel or frees the cursor, and never quits.
- **Zoom.** The dial runs from the shoulder out to about 120 m, short of the drift to the Capitol. The map stays a Click-mode view for now (§8).

Chris chose this over the MMO-classic alternative (free cursor, hold the right button to look) because the pad needs aim-based targeting anyway. This way the mouse and the pad share one model.

### D3. A `steer` message, not a stream of `move`s

`steer { heading: u16, throttle: u8, at?: [x, y] }`:
- `heading` is a fraction of a turn in the server frame: 0 is east, a quarter turn is south.
- `throttle` is in tenths, from 0 to 10.
- `at` is a waypoint to reach before heading off.

How it behaves:
- **Stopping.** A stop is throttle 0 with `at` set to the predicted position. The server is a delayed copy of the same path, so it stops exactly where the client did and doesn't slide on.
- **Turns** carry `at` too, so a corner happens at the same point on both ends.
- **Integers** make the fixture exact and the direction identical on both ends (`rules::steer::direction`, through StableMath).
- **A lease.** A steer lapses after 30 ticks (3 s) unless renewed, and the client renews it every second. So a client that vanishes doesn't walk on forever.
- **Never persisted.** Socket close, presence moving and starting an extraction or a craft all clear the steer.
- **A bucket of its own:** 15 a second with a burst of 30. Over-limit steers are dropped silently.
- **Safety.** `at` is walked under the ground rules at normal speed, so it's no stronger than `move`. It's ignored beyond 100 units or off the world.

### D4. One walk step for everyone

`rules::ground::walk_step(from, heading, walk)` runs, in order:
1. the land (slopes, cliffs, rivers, lakes, fords, roads over water);
2. the moat and canal rings;
3. the world edge;
4. a **slide**: when the straight step is blocked, the heading turned ±30° and ±60°, scaled by cos, taking the best one that isn't blocked.

`step(from, dest, walk)` becomes a wrapper over it. The server, the prediction and both modes call the same function, so click mode's prediction gets the rings too.

### D5. Interactions keep their rules

Only the choosing changes. `capitol_core::focus` picks the target in front of you: the angle off the view (35° at most), the distance (within reach plus 3 m), and a fixed tie-break. E then calls the same `Session` path a click does.
- Reach is still judged from the server position, so a target a step away is walked to, as today.
- After an interaction, `Steering` latches: it sends nothing until the movement input returns to neutral, so a held W doesn't cancel the gather you just started.

### D6. The pad plays, the mouse does menus (Chris's choice)

The pad does these:
- walking, with a radial deadzone and throttle steps of 0, 5 or 10;
- looking, as a rate;
- acting (Ⓐ) and backing out (Ⓑ);
- zooming (the triggers);
- opening and closing panels (the d-pad, Start and Select).

Inside a panel you used the mouse. F2 (#331) followed: see §8.

### D7. Walls are out of scope (Chris's choice)

Walking through houses, the hall and the stalls shows more from the shoulder than from above, but fixing it changes the rules for both modes. It's F1 (#330), marked needs-decision.

### D8. No advantage

Both modes have the same speed, ground rules and reach. Bots may steer (`Bot::steer`), but it's no faster than `walk_to`. Citizens keep `walk_to`.

## 5. Architecture notes

**Protocol (`protocol/`):**
- `rules::ground::walk_step`, with the rings and the edge in `Ground` (T0.1).
- `ClientMessage::Steer` and `rules::steer::direction` (T0.2).
- `rules::steer::Steering`, `take` and `tick`: a steer under way, taken and walked, for the server and the loopback alike (T0.4).
- `fixtures/client/steer.json`.

**Server (`backend/`):**
- `PlayerState.steer`, marked `#[serde(skip)]`: the direction, throttle, up to 8 waypoints and the lease in ticks (T0.2).
- `start_moving()`, shared by `Move` and `Steer` (T0.2).
- `Key::Steer` and `limits(&key) -> (burst, per_sec)` in `rate_limit.rs` (T0.3).

**capitol-core:**
- **`steering::Steering`** (pure; T0.4). It turns the input into steers: quantising, at most 8 a second, renewal, stop-with-`at`, the latch, and resending after a reconnect.
- **`Motion::steer`** (T0.4). It integrates in fixed 100 ms steps with `walk_step` and interpolates for drawing. It reconciles against the predicted path, blends instead of stopping, and counts `corrections()`.
- **`motion::Loopback`** (T0.4). A fake server at 10 Hz with a set round trip and tick phase. It's used by the tests and by `--offline-avatar`.
- **`focus`** (pure; T1.5).

**capitol-client:**
- `[controls]` in `settings.rs`, and `cap_setting` widened (T1.1).
- `cap_view_yaw` (T1.3).
- `wants_cursor()` in `Shared` (T1.4).
- `cap_focus` (T1.5).
- The reticle, the prompts and the per-mode shortcut tables (T1.6, T2.2).
- `controls_stats` (T3.1).

**Flint (upstream, T1.2, ADR 0110, [Flint PR #84](https://github.com/chrischaps/Flint/pull/84)):**
- While the cursor is captured, pointer events don't reach egui and the cursor reads as the window centre.
- `is_cursor_captured()`.
- Pads and raw mouse motion obey the text-focus gate. Pads are ignored while the window is unfocused.
- An input-script `look dx dy` command.

## 6. Phases and work packages

### T0: Groundwork (`ct:T0-groundwork`)

- **T0.1 One walk step** (#315, M): `walk_step` with the rings, the edge and the slide; the server and prediction both use it.
  *As built:* `Ground::with_rings(center, rings)` adds the rings; `Ground::from_terrain` and the server's `TerrainConfig::ground` both carry them, so the client's prediction, its pathfinder's `Ground` and the server walk the same rings. `Blocked` gains `Ring(id)` ("Cannot cross the inner canal without a bridge", as before) and `Edge` ("The world ends here", `water_id` empty). `validate_movement` is gone; `process_movement` is `Ground::step` alone and no longer takes the terrain. Decisions the plan didn't make:
  - **A ring stops you where it always did**: 2 short of the water along the line (`rules::ring_crossing`). That point doesn't depend on the step's length, so a prediction in frame-long steps stops on the server's point, to the float (`a_canal_stops_the_prediction_where_it_stops_the_server`). A stop that would leave you over the water off a bridge, or a step off a bridge's side, stays where it is.
  - **The edge** keeps you where you stand when a step would end off the disc and further out than it started. Someone off it already may walk back.
  - **The stopped step competes with the slide.** A slide wins only when its distance × cos beats how far the stopped step got along the heading. Otherwise a square walk at a bank swerved 60° a step before it got there. So a walk reaches the bank, and slides from the next step. Click mode stops at a block as before (the server clears the destination), so in practice it slides only when a click starts against a bank. Direct mode (T0.2) is where the slide shows.
  - **A slide may not end within 2 of a ring's water off a bridge, or of the edge, nearer to it than it started.** Without that, a square push into a canal slid 60° into the 2-unit margin, the next stop pushed it back out, and it crept along in a jitter. A river's stop is already at the last dry sample, so it needs no margin.
  - The old `ground.rs` test fixture lies in the square's corner, off the world's disc. Walks on it must head toward the centre, or the edge stops them.
- **T0.2 The `steer` message** (#316, M): the wire, the state, the lease, the stop, the clearing.
  *As built:* `rules::steer` holds the shared parts: `direction` and its inverse `heading_of` (for T0.4's quantising; every heading comes back as itself), `pace` (tenths, clamped to 10) and the constants `LEASE_TICKS` (30), `MAX_WAYPOINTS` (8) and `AT_REACH` (100). On the server, `movement::steer` takes a steer and `steer_step` walks one. Decisions the plan didn't make:
  - **A waypoint is walked at the pace the client had when it marked it**: the steer's throttle before this one, or this one's if that was 0, or full. So a corner where you speed up, or a stop, is walked on the server at the pace the prediction walked it. Each waypoint carries its throttle.
  - **A tick that reaches a waypoint ends on it.** The rest of that tick isn't carried round the corner. T0.4's prediction walks the same fixed 100 ms steps, so if it marks `at` at a step's end, the server's copy turns on the same point.
  - **A waypoint that can't be reached is dropped** where the walk stopped, and the steer goes on to the next one or the heading. A stop whose `at` is blocked ends there.
  - **A stop with no `at`** stands on the next tick.
  - **The 2 s cooldown is per player** (`GameState::steer_blocked_at`, by tick), for a block met while steering. A click walk still hears each block, since a block ends it.
  - **Socket close** needs nothing of its own: the holder's socket closing takes the avatar out of the world, steer and all. A presence claim clears the steer, not the destination, since a click walk already carried on across a claim.
  - Until T0.3 (#317), steers share the socket's general rate bucket.
- **T0.3 A rate bucket for steering** (#317, S).
  *As built:* `Key::Steer(n)` is numbered as the connection's `Key::Connection(n)` (`Key::steering`), and `Key::of_ws(connection, message)` names the bucket each WebSocket message draws from: none for `say`, the steer bucket for `steer`, the connection's for the rest. `RateLimitConfig::limits(&key) -> (burst, per_sec)` replaces `capacity`, `window` and `per_sec`, and the prune uses it. A steer over its limit is dropped with only a `debug` log line; `RATE_LIMITS=off` lets it through like everything else. A turn held at 8 a second never runs the bucket dry.
- **T0.4 `Steering`, steer prediction and the loopback** (#318, L).
  *As built:* the steer's state and its tick moved from the backend to `rules::steer` (`Steering`, `Waypoint`, `take`, `tick`, `BLOCKED_TICKS`), so the server and `motion::Loopback` walk the same code. `Steering::drive(now, way, &mut motion)` is the glue the game calls each frame: it returns the `ClientMessage` to send, if any. Decisions the plan didn't make:
  - **`Motion::steer(now, heading, throttle)` returns the `at`**, rather than taking one: the prediction knows where its step ends.
  - **A turn, or a change of pace, takes effect at the end of the step under way**, and that point goes as its `at`. The server ends a tick on it, so the corner falls at the same place on both ends and the server keeps its time. So a turn shows 0 to 100 ms after the input (50 on average). A second change in the same step goes with no `at` and follows the first.
  - **A stop is where the prediction stands** when it's made, mid-step, so the avatar doesn't walk on. When the server's tick falls just after the steer lands, it has walked the rest of that step before it hears, and walks back up to 5 units. The prediction allows it (`Local::cut`), so the player sees nothing; others see a small step back. That's #480.
  - **Reconciling a steer:** the trail first, then the steer's own path for a window ahead (the server can be up to a step ahead of the drawn position). Off both, the prediction moves to the server's position walked on `lag` worth of the same steer, and keeps walking. `movement_blocked` is ignored while steering: the server keeps the steer, and the same tick's snapshot says where it is. A stop sent while on a click's walk does nothing, as on the server.
  - **The rate:** a bucket of 8 a second, 2 deep. A turn needs 2 tokens and spends 1, so a stop or a start is never held up by a sweep of the mouse. A turn under half a degree isn't sent.
  - **Throttle steps:** under 0.25 deflection is neutral, under 0.75 is half. T2.1's radial deadzone may move them.
  - **The latch stops a walk** that's under way, then holds until the input is neutral.
  - **Reconnects** are noticed through `Motion::placed()`, a session number that changes on every reconnect.
  - **`Bot::steer(dir, secs)`** sends a steer, renews it each second, then stops with no `at`.
  - The loopback tests run every case at a 0, 80 and 200 ms round trip and at five tick phases. Two of the phases (+20 and +60 ms) put the server's tick just after the steer lands, which is when it runs ahead.

### T1: Direct mode on keys and mouse (`ct:T1-direct`)

- **T1.1 Control mode** (#319, M): `[controls]`, F8, `--controls`, captures stay on Click.
  - *As built:* the mode in play is pinned apart from the file for a run that has `--controls`, or is offline or a capture; F8 and the panel change a pinned mode for that run alone. Every switch stops a click walk where the prediction stands (`Motion::stop`, `Session::stop_walk`), and `camera.rhai` drops its pan on a switch back to Click. The toast is a `controls_note` event. T1.3 turns the Direct-to-Click stop into a steer stop.
- **T1.2 Flint: capture and focus gates** (#320, M, `engine-upstream`). *Done (ADR 0110, Flint PR #84, merged and pulled):* while captured, the cursor reads as the window centre, its moves reach no one, and its clicks and wheel skip egui but reach the game. `is_cursor_captured()` (scripts) and `ExtensionCtx::is_cursor_captured()` report it from the frame after a request. `InputState::hold_gamepads` holds the pads while a field has focus. A button held through the hold reads up until it's released, so it never fires a press nobody made. Raw motion is dropped then too. Real pads are released on losing focus, and their sticks are read again on regaining it. The input script's pad plays unfocused. `look dx dy [secs]` feeds raw motion only while captured, so T1.4's captures call `set_cursor_captured(true)` before they `look`.
- **T1.3 Direct mode walks** (#321, M): camera-relative WASD, `cap_view_yaw`, `--offline-avatar`.
  *As built:* `capitol_core::steering::camera_way(forward, right, yaw)` turns the keys into a way in server units against the yaw `camera.rhai` publishes (`cap_view_yaw(rig.yaw_to)`), cut to length 1. `pre_update` passes it to `Session::steer` every frame, and (0, 0) in Click, so a switch's stop and the steering's own stop always go out. `Session::stop_walk` stops a steer with a steer's stop (T1.1's note). `Blocked.toward` is a point ahead on the steer's heading. `--offline-avatar [rtt_ms]` is `offline_avatar.rs`: placed where the camera looks at 0.5 s, as the samples are, on the offline terrain's `Ground`, drawn by `entities::Players` (which sets the follow point); a click walks it in Click too, and its exit line is `[motion] corrections N`. `Loopback::named` gives it a name. Decisions the plan didn't make:
  - **The latch is set by every walk-then-act** (`Session::walk`, which every plan's walk goes through, and an in-reach extract), **and by a gather or craft starting**, not only by E (T1.5). Otherwise a key held as you click a tuft cancels the walk to it the next frame, and the steering's renewal walks you off a gather the server had just stopped you for. Plans are dropped only by a steer that will walk, never while latched.
  - **The rig doesn't swing round behind you in Direct** (`follow_yaw`, E3.13). The keys walk against the view, so a swing would turn the way W walks, and a held W+D would curl round. T1.4's mouse-look is how you turn there. Q/E don't orbit (since T1.4, a right-drag does only while the cursor is free).
  - **Offline with no avatar, Direct's WASD still moves the free point**, and Home still resets it, so the rig stays usable.
  - **The ring needs nothing**: a steer never sets a click destination, and the prediction's follow of a server walking back after a stop (#480) is filtered out of `destination()` already.
  - **The onboarding step needs nothing**: since #441 the server judges Find your feet (`walks_from`, any walk ending 20 or more from where it began), and it counts a steer's. `Session::moving()` is there for `stop_walk` and later readers.
- **T1.4 The action camera** (#322, L): capture, mouse-look, the zoom cap, Escape.
  *As built:* `CapitolExtension::wants_cursor()` is published at the end of `ui` as `Shared::wants_cursor` (`cap_wants_cursor`), and `camera.rhai` calls `set_cursor_captured` whenever it disagrees with `is_cursor_captured()`. Panels count by their rect this frame where they keep one (inventory, crafting, exchange, market, impound, shop, station, card, plot, trade with its request card), else by their open flag (chat, letters, the Gazette, guilds, settings, shortcuts, the Deeds Office, the zone card). The login, the "Playing on another device" banner, a station being placed and Alt count too. Look is `mouse_delta` × `look_rate()` (0.0025 rad a count) × `look_sensitivity`, with the yaw unsmoothed in Direct; the tilt is `tilt_by` and `pose_pitch` (the dial's pitch plus the tilt, never past straight down), passed to `rig_pose` as a fifth argument. The cap is `zoom_cap(true)`, 120 m (`direct_far()`). The tests are `camera_rig::the_mouse_tilts_within_its_limits`, `over_the_shoulder_the_dial_stops_short_of_the_map`, `a_tilted_camera_never_goes_under_the_ground` and `a_wall_pulls_the_camera_in_with_the_view_tilted`, plus `extension::escape_tests`. Decisions the plan didn't make:
  - **The road in's card doesn't free the cursor.** It's up for the whole tour, so Direct would never capture during it. Alt frees the cursor for its buttons.
  - **Entering Direct closes the inventory and crafting panels** (`enter_direct`, on the first frame too). Click opens both from the start, and an open panel would keep the cursor free. I and C bring them back.
  - **Esc's "top panel" is a fixed order** (`close_top_panel`): the card, the zone card, the Deeds Office, settings, letters, guilds, the Gazette, the shop, the market, the exchange, the impound, the station, the plot, crafting, then the inventory. The trade window is never closed by Esc, so Esc can't walk you out of a deal. Esc's other checks come first: the shortcut overlay, a placing, the chat. F8 back to Click clears a let-go.
  - **A captured click does nothing until T1.5.** `input.rhai` returns before the click priorities while `is_cursor_captured()`, and T1.5's interact goes there. Alt frees the cursor to click things meanwhile. With the cursor freed, clicks go down the list as in Click, but for the walk.
  - **A shot or the login circle lets go down to the shoulder** in Direct (`take_over_from_shot`), and Home is the shoulder, level. Switching back to Click gives the street zoom back and levels the tilt.
  - Right-drag still orbits while the cursor is free (Alt held, a panel open). Captured, the cursor reads the window centre, so it doesn't.
- **T1.5 Focus and interact** (#323, M).
  *As built:* `capitol_core::focus` is pure. `focus(candidates, you, view)` picks among `Candidate`s, each a kind, an id, the words for E (and for Shift-E, if they differ) and a point or a footprint. `from_state` gives the store's available nodes, its stations and the other players, labelled from the catalog ("Gather wild grass", "Open the forge"). The client adds the hall, its notice board, the Deeds Office and the bazaar's stalls from where it draws them (`Shared::hall`, `Shared::bazaar`), since the board's and the office's slots are only the client's. Offline it takes the samples' nodes and stations, so captures need no server. Each frame `CapitolExtension::focus_and_interact` publishes `cap_focus()` while Direct has the cursor captured. On E (`interact`, now in `config/input.toml`) or a captured click (`cap_interact()` in `input.rhai`) it sets the same `Shared` request a click on the target sets (`extract`, `open_station`, `open_card`, `trade_with`, `visit_hall`, `visit_stall`, `read_gazette`, `open_office`), then `Session::latch_steering`. So reach, the walk to the target and refusals are a click's. `input.rhai`'s `draw_focus` rings a node in gold and a station in gold with turning marks, puts a ring at a player's feet, and shows a key-cap prompt ("E  Gather wild grass", with a "Shift E" row under it). The hall, the board, the office and the stalls show as hovered, with their own prompts, and "click" reads "press E" there (`act_words`). Tests: `focus::tests` and `session::tests::e_gathers_the_grass_in_front_and_a_held_key_keeps_it` (live, ignored). Decisions the plan didn't make:
  - **E on a player is their card, and Shift-E asks them to trade**, as a click and a shift-click do (S0.3 came after the plan's "E on a player asks to trade").
  - **Scoring:** the angle's share of 35° plus half the distance's share of the reach plus 3 m. Ties go to the kind (node, station, player, board, office, stall, hall), then the id. A footprint counts by its nearest point in front, from 16 points along each side, and standing inside it counts as dead ahead.
  - **The focus holds.** Last frame's target keeps the focus until another scores `HOLD` (0.1, about 3.5°) better (`focus_holding`). Without that, the focus swapped between a forge and the branches beside it as the camera settled, and E landed on whichever had it.
  - **The board and the office** are focused within 5 m (`LOOK_REACH`), though they open from anywhere.
  - **The prompt keeps off your own name.** Over the shoulder, what's ahead stands just above your head on the screen, so a prompt that would cover your tag is lifted above it.
  - **Things you can't gather are still focused** (a tree without an axe), as a click on them is still taken: the server's refusal says why. Targets behind a solid aren't dropped (the plan's optional `cap_clear_line`).
  - **Hall's "near" prompt** shows over the shoulder only while the hall is the focus, since "press E" while you look elsewhere would be wrong.
- **T1.6 Direct mode's words** (#324, S): the reticle, shortcuts, tips, README.
  *As built:* the reticle is `input.rhai`'s `draw_aim`: a gold dot on a dark edge at the screen's centre while the cursor is captured, faint (0.42) with no focus and easing to full with one, when a faint ring opens round it. The shortcut overlay has a card per mode (`ui/shortcuts.rs`, `CLICK` and `DIRECT`), named under its title ("Point and walk", "Over the shoulder"). Only getting about and the work differ; the panels are shared, and F8 moved into getting about, named for the mode it switches to. Its tests read `config/input.toml` and check that every key bound to an action the mode reads is on its card (Q/E's orbit is Click's alone; `interact` is Direct's). The road in's walk step reads "Walk with WASD" over the shoulder (`hints::words_in`, `Hints::set_direct`); the other steps keep their words, since a captured click on the focus does what their "[Click]" says. The Direct switch toast says E acts, and the offline inscription panel names the mode's keys. The overlay's columns scroll when the window is too short for them (720p at 150% was already, before T1.6). The player README (`package.ps1`) has a Controls section for both modes. Decisions the plan didn't make:
  - **The prompt stays in the world**, by the target, not fixed beside the reticle (T1.5's open question). The dot brightens with a focus, so the eye goes from the centre to the target's ring, where the prompt is.
  - **Direct's card has more rows than the issue's six**: zoom, Home and the work's E rows (gather, open, a player's card, Shift-E to trade) too, so every key Direct reads is on it.

### T2: The pad (`ct:T2-pad`)

- **T2.1 The pad plays** (#325, M): bindings, the radial deadzone, throttle steps, look rate, recentring, Back.
  *As built:* `capitol_core::pad` is pure: `stick` (the radial deadzone, 0.2, rescaled and cut to length 1), `walk`, `look` and `Recentre`. Direct's walk reads `walk_x`/`walk_y` (the keys at ±1 and the left stick, summed) through `pad::walk`, so the keys go the same way and `move_*` are Click's pan alone. The throttle steps stay `steering::throttle_of`'s (0.25 and 0.75), now of the rescaled deflection, so a raw push under 0.4 is neutral, under 0.8 half, and full past it. `pre_update` publishes the look as `Shared::pad_look` (`cap_pad_look()`: `yaw`, `tilt`, `level`, `walking`), and `camera.rhai` only adds it. Back (`back`, East) is `CapitolExtension::back`. `[controls]` gains `pad_look_sensitivity` and `invert_pad_y`, with Settings rows "Pad look" and "Invert pad look". `--offline-avatar` now also prints `[motion] walked D from where you stood, throttles [..]` on exit, for captures. Tests: `pad::tests` and `extension::pad_tests`, which load `config/input.toml` and drive slot 9. Decisions the plan didn't make:
  - **The look curve is the square** of the rescaled deflection: half the push turns at a quarter of the rate. Full is `TURN_RATE` (2.4 rad/s, about 140°/s) and `TILT_RATE` (60°/s), times the pad sensitivity.
  - **The stick looks with the cursor free too.** A panel's cursor is the mouse's, and the stick can't move it, so a panel open doesn't stop the view turning. The mouse still looks only while captured.
  - **Recentring needs a walk.** The view eases round behind the walk (`RECENTRE_K` 0.6/s), and the tilt back to level, only while the walk is deflected and within 45° of forward, the look stick has rested 1.5 s, and the pad was the last device used. Standing still it never recentres, so a look round at a stall stays where you left it. Like any camera that follows the walk, a stick held 30° off forward walks a wide curve.
  - **Back works in both modes**, in Esc's order: the shortcut overlay, a placing, the chat, then the top panel (`close_top_panel`). With nothing open it does nothing; it never frees the cursor as Esc does.
  - **The triggers zoom as far as they're pulled** (`pulled` in `camera.rhai`, past a tenth). The keys read 1, as before. An input script can only press a trigger fully, so the half pull is tested on `InputState`, not captured (#513).
  - **In Click the sticks do nothing.** The d-pad, Start, Select, the triggers and Back work there too, and South does nothing, as E does.
- **T2.2 Prompts in the device you're holding** (#326, S).
  *As built:* over the shoulder, with the pad the last thing used (Rhai's `last_input_device()` and the cursor captured, `pad_held` in `input.rhai`; `CapitolExtension::pad_held` in Rust), the focus prompt's key cell becomes the pad's button: (A) for E, and (Y) for Shift-E. The hall's, the stalls', the board's and the office's lines say "press (A)" (`act_words` writes `{A}`, and `draw_act_line` draws it as a glyph), and the hall's "· M" is the d-pad's right. The glyphs are drawn, in `input.rhai` (`draw_pad_glyph`) and in egui (`theme::padcap`). The face buttons are round, ringed and lettered in the town's tints (`theme::pad_tint`: sage A, brick B, slate X, ochre Y). The sticks are a cap in its well, the triggers and shoulders are caps rounded on top, the d-pad is a cross with its arm lit gold, and Start and Select are pills with their marks. Rows of keys write the pad's in braces, `{A}` beside `[E]` (`shortcuts::keys_row`). Direct's shortcut card has a pad column, and the device you aren't using is drawn at 0.45. Point and walk's has none, since the sticks do nothing there. The road in's walk step says the left stick walks and the right one looks (`hints::Play::Pad`), and every card's keys say the pad's where it has them (`shortcuts::on_the_pad`). The offline inscription panel names the pad's controls too. Decisions the plan didn't make:
  - **Y trades.** T2.1 left Shift-E with no pad equivalent, so a pad prompt couldn't honestly offer to trade. North is bound to `trade`. Over the shoulder `trade` (T or Y) now asks whoever's in focus, if it's a person, before the nearest player (`Session::trade_key(now, ahead)`); an incoming request is still accepted first.
  - **The card stacks when narrow.** With the pad's column, two columns side by side want about 1090 points, more than 1280x720 at 150% gives (853). The card measures them side by side once, and while the window is narrower it puts the work under getting about and the panels, in one column that scrolls.
  - **The cards' words keep "click".** The gather and people cards' bodies still say "click a tuft" and "Click anyone", as T1.6 kept them for the keys; their key rows say (A) and (Y).
  - **No capture of a person in focus.** Offline, the citizen sample isn't a focus candidate and fights the offline avatar over the drawn players, so (Y)'s trade line is unit-checked, not captured (#515).

### T3: Decide (`ct:T3-decide`)

- **T3.1 A/B numbers** (#327, S): `save/controls_stats.toml` and `--controls-report`.
  *As built:* `controls_stats.rs` in capitol-client. The file is a table per "server player" with `click` and `direct` under it; a missing field or file is zero, and an unreadable one is ignored. Decisions the plan didn't make:
  - **An interaction** is counted where `Session` decides between acting and walking first: a node, a station, the hall, a stall, the board at the hall and a wage offer. A click or an E on the same thing counts alike, so the split is about reach, not the device. A walk that arrives isn't counted again.
  - **Distance** is from your server position frame to frame. A step over 200 units is a jump (a reconnect, a presence taken back) and isn't walked.
  - **Minutes** count only while you play the avatar here, in the world: not at the login, nor while another device has it. A frame counts at most 0.5 s, so a dragged window or a sleep isn't play.
  - **Gathers** are extractions that yielded something; a failed pull isn't one. Gathers an hour waits for a minute of play.
  - **Switches** count into the mode switched to, by F8 or the panel, not the first frame's mode.
  - **Captures and offline runs keep none**, so a recipe never writes the file. It's written every 30 s while there's something new, and as the client closes.
- **T3.2 Play both and decide** (#328, S, `needs-decision`).
- **T3.3 Settle the controls** (#329, S to M): carry out T3.2's decision.

## 7. Dependency graph and estimate

```
T0.1 ─► T0.2 ─┬─► T0.3 ─┐
              └─► T0.4 ─┼─► T1.3 ─► T1.4 ─► T1.5 ─► T1.6 ─► T2.2 ─► T3.2 ─► T3.3
T1.1 ───────────────────┘     │       ▲       │                ▲       ▲
T1.2 ─────────────────────────┼───────┘       └──► T2.1 ───────┘       │
       (T1.2 also ─► T2.1)    └──► T3.1 ───────────────────────────────┘
F1 (#330), F2 (#331), F3 (#332): independent
```

| Package | Depends on |
|---|---|
| T0.2 | T0.1 |
| T0.3 | T0.2 |
| T0.4 | T0.1, T0.2 |
| T1.3 | T0.3, T0.4, T1.1 |
| T1.4 | T1.2, T1.3 |
| T1.5 | T1.4 |
| T1.6 | T1.5 |
| T2.1 | T1.2, T1.5 |
| T2.2 | T2.1, T1.6 |
| T3.1 | T1.3 |
| T3.2 | T1.6, T2.2, T3.1 |
| T3.3 | T3.2 |

**The order of work:**
1. T0.1 and T0.2 come first, on the server.
2. T1.1 and T1.2 don't depend on T0, so they can run alongside it.
3. T1.3 is the first point where you walk with keys.

**Estimate:**

| Phase | Packages |
|---|---|
| T0 | 2 M, 1 L, 1 S |
| T1 | 4 M, 1 L, 1 S |
| T2 | 1 M, 1 S |
| T3 | 3 S |

That's about the size of the wilds' W4 to W6 together.

## 8. Later (outside this plan)

- **F1, walls** (#330): solid buildings in `walk_step`, if Chris decides for them.
- **F2, the pad inside panels** (#331). *Built:* with a panel open and the pad the last thing used, in either mode, the d-pad moves a gold ring between the panels' widgets instead of opening panels (`ui/pad_nav.rs`; the `ui_*` actions). A presses what's ringed, X takes half a stack, the shoulders hop to the next panel along, and B is Back. It rides egui's own focus: the ring picks the next widget from the last frame's rects (egui's cone search, leaving out widgets under another panel, scroll bars, sliders and the panel's own surface), lends it egui's focus for the pass, and a press is an Enter pushed into the pass's input, which a focused widget answers as a click. A panel that comes up takes the ring; with none, it starts in the panel on top. A slot grid needs nothing of its own for the d-pad; A on a slot picks its stack up (`slots`' `Carry`, drawn riding over the ringed slot) and A on another puts it there, as a drop does, a split asking how many first. The trade window and the market take a carried stack as a drop. Every panel's scroll area is `show_following`, which scrolls the ring into view. The shortcut card gains the pad's rows (a whole-d-pad glyph, `{DPad}`). Not built, each with an issue: a pad can't type, set a number or move a slider, or scroll the Deeds Office's list (#517), and a cursor on the left stick needs a Flint hook to feed egui input (#518). Flint takes any egui focus for typing and holds the pads, so the ring hands egui the focus back at the end of each pass (#516).
- **F3, rebinds that survive a restart** (#332). Flint's user override is saved under one name and loaded under another. Fixed in Flint#85: the player looks for `<config>/flint/input_<game_id>.toml` once the game default sets the id, and saves there. The Capitol's live at `%APPDATA%\flint\input_capitol.toml`.
- **F4, a controls menu** (#425). Scripts can't start a rebind (`begin_rebind_capture` is Rust-only), so rebinding today means editing that file by hand.
- **The map in Direct.** If Direct stays, a key that lifts the camera to the map and back.
- **Mouse-look in Click mode.** If both stay, holding the right button could turn the camera in place of today's drag.
- **Remote avatars' gait.** Analog throttle is held to three steps because the walk blend is yes or no. A blended gait would let the pad walk at any speed.

## 9. Risks

| Risk | Mitigation |
|---|---|
| **Reconcile under turning at high latency.** | T0.4's loopback tests at 200 ms gate it, with `at` waypoints on stops and turns. The fallback is a wider tolerance while steering. *As built:* a sweep of 90° a second for 4 s has no correction at 200 ms. |
| **The engine PR (T1.2) blocks the camera.** | T1.3 lands first and is played with the cursor free. Chris merges Flint PRs. |
| **The latch feels sticky** (you have to let go of W before you can walk after E). | Tune it in T1.5. It clears on neutral input, never on a timer. |
| **Walking through walls spoils the shoulder view.** | F1 (#330) is the decision. Until then the camera pulls in out of the buildings' walls, so at least the view never looks through a wall. |
| **The A/B is one person's taste.** | T3.1's numbers sit beside the feel, and T3.2 asks for sessions with and without the pad. |
| **Steering costs the server a lock per message.** | At 8 a second per walking player, it's well under the tick's budget at today's scale. Batch per connection if the soak (C1.7) says otherwise. |

## 10. Verification strategy

- **Pure tests** in `protocol` (`walk_step`), in `backend` (steer, lease, bucket) and in capitol-core (`Steering`, `Motion` under the loopback, `focus`). These carry most of the weight.
- **The loopback** at a 0, 80 and 200 ms round trip, for stops, sharp turns, banks, canals, reconnects and lapsed leases. The test is no slide-on, and no correction over 4 units.
- **Offline captures:** `--controls direct --offline-avatar 80 --input-script …`, with `look`, keys and pad axes, at the shoulder in the bazaar, a street of houses and a rural lane.
- **Click mode doesn't move:** its captures and `showcase.ps1` output are unchanged with Direct saved in settings.
- **[H] Chris plays both,** with and without a pad (T2.1, T3.2).
