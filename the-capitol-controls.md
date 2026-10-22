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
- Moving round inside a panel with the pad. That's F2 (#331). Panels stay mouse-driven.
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

Inside a panel you use the mouse. F2 (#331) is the follow-up.

### D7. Walls are out of scope (Chris's choice)

Walking through houses, the hall and the stalls shows more from the shoulder than from above, but fixing it changes the rules for both modes. It's F1 (#330), marked needs-decision.

### D8. No advantage

Both modes have the same speed, ground rules and reach. Bots may steer (`Bot::steer`), but it's no faster than `walk_to`. Citizens keep `walk_to`.

## 5. Architecture notes

**Protocol (`protocol/`):**
- `rules::ground::walk_step`, with the rings and the edge in `Ground` (T0.1).
- `ClientMessage::Steer` and `rules::steer::direction` (T0.2).
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

### T1: Direct mode on keys and mouse (`ct:T1-direct`)

- **T1.1 Control mode** (#319, M): `[controls]`, F8, `--controls`, captures stay on Click.
  - *As built:* the mode in play is pinned apart from the file for a run that has `--controls`, or is offline or a capture; F8 and the panel change a pinned mode for that run alone. Every switch stops a click walk where the prediction stands (`Motion::stop`, `Session::stop_walk`), and `camera.rhai` drops its pan on a switch back to Click. The toast is a `controls_note` event. T1.3 turns the Direct-to-Click stop into a steer stop.
- **T1.2 Flint: capture and focus gates** (#320, M, `engine-upstream`). *Done (ADR 0110, Flint PR #84, merged and pulled):* while captured, the cursor reads as the window centre, its moves reach no one, and its clicks and wheel skip egui but reach the game. `is_cursor_captured()` (scripts) and `ExtensionCtx::is_cursor_captured()` report it from the frame after a request. `InputState::hold_gamepads` holds the pads while a field has focus. A button held through the hold reads up until it's released, so it never fires a press nobody made. Raw motion is dropped then too. Real pads are released on losing focus, and their sticks are read again on regaining it. The input script's pad plays unfocused. `look dx dy [secs]` feeds raw motion only while captured, so T1.4's captures call `set_cursor_captured(true)` before they `look`.
- **T1.3 Direct mode walks** (#321, M): camera-relative WASD, `cap_view_yaw`, `--offline-avatar`.
- **T1.4 The action camera** (#322, L): capture, mouse-look, the zoom cap, Escape.
- **T1.5 Focus and interact** (#323, M).
- **T1.6 Direct mode's words** (#324, S): the reticle, shortcuts, tips, README.

### T2: The pad (`ct:T2-pad`)

- **T2.1 The pad plays** (#325, M): bindings, the radial deadzone, throttle steps, look rate, recentring, Back.
- **T2.2 Prompts in the device you're holding** (#326, S).

### T3: Decide (`ct:T3-decide`)

- **T3.1 A/B numbers** (#327, S): `save/controls_stats.toml` and `--controls-report`.
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
- **F2, the pad inside panels** (#331).
- **F3, rebinds that survive a restart** (#332). Flint's user override is saved under one name and loaded under another. Fixed in Flint#85: the player looks for `<config>/flint/input_<game_id>.toml` once the game default sets the id, and saves there. The Capitol's live at `%APPDATA%\flint\input_capitol.toml`.
- **F4, a controls menu** (#425). Scripts can't start a rebind (`begin_rebind_capture` is Rust-only), so rebinding today means editing that file by hand.
- **The map in Direct.** If Direct stays, a key that lifts the camera to the map and back.
- **Mouse-look in Click mode.** If both stay, holding the right button could turn the camera in place of today's drag.
- **Remote avatars' gait.** Analog throttle is held to three steps because the walk blend is yes or no. A blended gait would let the pad walk at any speed.

## 9. Risks

| Risk | Mitigation |
|---|---|
| **Reconcile under turning at high latency.** | T0.4's loopback tests at 200 ms gate it, with `at` waypoints on stops and turns. The fallback is a wider tolerance while steering. |
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
