# The Capitol: Flint Client Plan

**Status:** Planning, written 2026-10-11
**Scope:** Players can do everything in the Flint client that the React client allows through M7, Windows desktop first
**Location:** `flint-client/` in this repo, with the Flint engine as a git subtree at `flint-client/engine/`

**Tracking:** every work package (P0.1 … P8.7) and backend prerequisite (B1 … B7) is a GitHub issue labelled [`flint-client`](https://github.com/chrischaps/TheCapitol/issues?q=label%3Aflint-client), created in dependency order. The lowest open, unblocked issue is next. Agents pick it up with the `next-issue` skill (`.claude/skills/next-issue/SKILL.md`). The issue number for each work package is in §6. When the plan changes, update this doc in the same commit.

This document breaks down the work of building a third client for The Capitol on [Flint](https://github.com/chrischaps/Flint), the Rust/wgpu engine. The research behind it covers the Flint engine at `origin/main` 8cc8306, the backend in `backend/src`, and both existing clients (`client/`, `love-client/`). File references point at those sources so each claim can be checked.

---

## 1. Vision

Today The Capitol is drawn as flat discs on a dark canvas. The Flint client should let you *stand in it*.

The GDD describes a radial world where everyone travels inward toward the Capitol. That shape suits a game about ambition, and the 3D client should make it something you can see:

- **The tower is always visible.** A modern structure arranged like a medieval castle (GDD §World) stands at the center as the one landmark you can see from anywhere. Wherever you are, you know which way "up in the world" is.
- **Water draws the shape of the city.** The moat and the two canals are the strongest silhouettes in the world. Bridges are the few places where the rings open up, so they feel like thresholds you cross on purpose.
- **Each zone has its own light.** The zone palette from the React client becomes ground tints and the colour of the light: gold at the core, then blue for Trade, violet for the Guilds, slate for Urban, green for Suburban and lime for Rural. The edges fade into the dark of the wilderness. At dusk the city reads as rings of light.
- **Gold means you.** Your avatar, your plots and your stations carry the gold accent `#c9a227`. Your progress shows up as gold spreading across the map.
- **Zooming out turns the city into a map.** At the furthest zoom the camera looks straight down, the lighting flattens, and the world becomes the React client's flat vector style. Every client shows the same world, so the 3D client nods to the 2D one.

GDD Pillar 4 says clients are "interchangeable windows into the world". This client is the window that lets the place feel like somewhere you could live.

## 2. Goals and non-goals

**Goals**
- Everything a player can do through M7: auth, movement, extraction, inventory, crafting, stations, exchange, P2P trading and plots. See §8 for the full checklist.
- A 3D world built at runtime from `GET /terrain`, with a tilted, zoomable and orbitable camera.
- Fixes for two weaknesses both existing clients share: other players are **interpolated**, and the **WebSocket reconnects** after a drop. M10 lists reconnection as a deliverable anyway.
- Architecture with room for M6 markets, M8 automation and M9 social panels without restructuring.
- Engine improvements that are general enough to go back upstream to Flint. Every Flint game so far has made the engine better.

**Non-goals for this plan**
- Admin tools: the F1 debug camera, F2 world tweaker and F3 perf panel stay in the React client.
- The M6 Markets UI. The backend doesn't have markets yet.
- Web/wasm. Flint has no wasm target.
- Android. Flint supports it (`crates/flint-android`), but it's out of scope here.
- Server-side changes beyond the small prerequisites in §5.

## 3. What Flint gives us, and what it doesn't

### What exists

| Capability | Where |
|---|---|
| `flint-player` as a library with a public `PlayerApp`, embeddable from another crate | `crates/flint-player/src/lib.rs`; working example in `crates/flint-android/src/lib.rs:130-174` |
| Runtime procedural mesh upload | `SceneRenderer::load_procedural_mesh` (`crates/flint-render/src/scene_renderer/mod.rs:1956`) |
| Mesh building: quads/triangles, merge, normals, extrude-along-curve | `crates/flint-procgen/src/algorithms/mesh_builder/` |
| Stylised Gerstner water with foam | `crates/flint-render/src/ocean_pipeline.rs` (a camera-following grid, see H6) |
| Compute-instanced grass | `crates/flint-render/src/grass_pipeline.rs` |
| Sky, fog, volumetrics, SSAO, bloom, Kuwahara painterly post | `crates/flint-render/src/*` |
| Particles v2 (forge fire, extraction puffs) | `crates/flint-particles` |
| Rhai scripting with hot reload, imports, `load_data` | `crates/flint-script/src/api.rs` |
| Script HUD drawing: text, rect, ring, arc, convex polygon, `world_to_screen`, project fonts | `hud_render.rs`, `draw2d.rs`, `player_app/fonts.rs` |
| Orthographic and perspective cameras controllable from script | `set_camera_*`, `set_camera_orthographic` |
| Spatial audio | `crates/flint-audio` |
| On-disk `PersistentStore` | `persist_*` script API |
| Headless capture: `--screenshot`, `--screenshot-at`, `--input-script`, `--exit-after` | `flint-player` CLI (8cc8306) |
| Thread-plus-`mpsc` pattern feeding the game loop | `crates/flint-input-capture/src/lib.rs:157-184` |

### What's missing (each gap is closed in §4 or §6)

| Gap | Evidence |
|---|---|
| **No networking.** No tokio, reqwest or tungstenite anywhere. `ureq` is used only by `flint-asset-gen`. | `Cargo.toml` across all crates |
| No way for a game to register native Rhai functions | `ScriptEngine.engine` is private (`crates/flint-script/src/engine.rs:145`) and `register_all` is hard-wired |
| No plugin or extension hook. `PlayerApp`'s renderer isn't reachable from outside. | `player_app/mod.rs:127-250` |
| `GameEvent::Custom` never reaches scripts. Only collisions, triggers and `ActionPressed` are routed. | `engine.rs:328` `process_events` |
| **No text input** in any form: data-driven UI, Rhai, or `KeyEvent.text`/IME | n/a |
| egui gets input only while a debug panel is open | `player_app/events.rs:337-345` |
| No script-level screen-to-world function. `Camera::unproject_ray` exists in Rust only. | `crates/flint-render/src/camera.rs:268` |
| Instancing covers sprites, particles, billboards and grass only. Each mesh entity is one draw. | `scene_renderer` |
| The ocean shader can't render an arbitrary mesh such as a ring canal | `ocean_pipeline.rs` |

**Housekeeping (done in P0.1):** the local checkout `C:\Users\chris\dev\Flint` was 23 commits behind `origin/main`. The mouse API, polygon and ring drawing, project fonts, `ui_hit`, the renderer resource cache and the capture flags all live in those commits. It was pulled to `8cc8306`, and the subtree at `flint-client/engine` was added from that commit.

## 4. Architecture

### 4.1 Repository layout

```
TheCapitol/
  Cargo.toml                 # root workspace: members = ["backend", "protocol"], exclude = ["flint-client"]
  protocol/                  # NEW crate `capitol-protocol`, shared by the backend and the Flint client
    src/ws.rs                #   ClientMessage / ServerMessage (moved out of backend ws/connection.rs)
    src/rest.rs              #   request/response DTOs
    src/terrain.rs           #   TerrainResponse, WaterRing, Bridge, RoadSegment, RoadType, TerrainPlot
    src/rules.rs             #   ranges, speeds, quality grades, zone radii, water-crossing test
  flint-client/
    Cargo.toml               # its own workspace, exclude = ["engine"]; path deps into engine/crates/*
    Cargo.lock               # committed, seeded from engine/Cargo.lock
    CLAUDE.md                # build/run, subtree pull/push, engine-upstream rules
    engine/                  # Flint, git subtree (squash)
    crates/
      capitol-core/          # net + state, no engine dependency, fast tests
        src/net/             #   REST client, WS client, reconnect, IO thread
        src/state/           #   world mirror, interpolation, polling scheduler, trade FSM
        src/bin/capitol-bot.rs  # headless client for integration tests
      capitol-client/        # the game binary
        src/main.rs          #   builds PlayerApp, installs CapitolExtension
        src/extension.rs     #   CapitolExtension: the PlayerExtension (H1) the game installs
        src/coords.rs        #   server <-> world mapping
        src/world/           #   terrain -> meshes (ground, water, roads, bridges, plots, tower)
        src/entities.rs      #   state -> ECS entities each frame (players, nodes, stations)
        src/script_api.rs    #   cap_* Rhai functions
        src/ui/              #   egui panels + Capitol theme
    scenes/capitol.scene.toml
    schemas/                 # capitol_player, resource_node, station, ... (layered over engine/schemas)
    scripts/                 # camera.rhai, input.rhai, hud.rhai, feedback.rhai, lib/
    fonts/ audio/ models/
    fixtures/terrain.json    # captured /terrain for offline mode and tests
    play.bat
```

**Workspace wiring (P0.2).** `engine/` has a workspace of its own. If the client's workspace didn't exclude it, cargo would make the engine crates members, because they sit under the client's root. With the exclusion, the engine crates are plain path dependencies, and they still inherit `version.workspace` and their other settings from `engine/Cargo.toml`. Third-party crates that cross the engine boundary (egui 0.30, winit 0.30, wgpu 23, rhai 1, glam 0.30) must use the engine's versions. The root `.gitignore` ignores `Cargo.lock`; `flint-client/.gitignore` re-includes the client's.

**Why a game crate, when Flint games are usually data only?** Every Flint game so far has been self-contained. The Capitol client is a window onto a server, so much of its logic is protocol and state that exists only because of this backend. If that code went into the engine, every other Flint game would carry it. The general-purpose pieces become engine hooks (§4.4) and go upstream. The Capitol-specific code stays in `flint-client/crates`.

**Why `capitol-protocol`?** The React and Love clients each copy the message shapes by hand, and they have already drifted apart. Quality thresholds alone exist in three versions (§4.7). A shared Rust crate means the backend and the Flint client compile against the same enums, so drift becomes a build error. It is the only backend refactor this plan asks for, and it changes how types are organised, not how anything behaves.

**How `capitol-protocol` was built (P0.3).**
- **Dependencies.** By default it needs only serde, uuid and chrono. A `backend` feature adds the server-only derives: `utoipa::ToSchema`/`IntoParams` and, for `ZoneInfo` and `PlotPermissions`, `sqlx::FromRow`. The backend turns the feature on; `capitol-core` already depends on the crate with it off.
- **Shared types.** The backend re-exports the shared types from `models/` (`pub use capitol_protocol::...`), so engine code didn't change. The helpers that built them became methods on the backend types: `Item::to_inventory_item`, `Plot::to_info` and `StationState::to_info`.
- **Terrain.** The generator's own `WaterRing`, `Bridge`, `RoadSegment`, `RoadType` and `TerrainPlot` had the same fields as the `/terrain` response types, so they are now one set of types in `capitol_protocol::terrain`. `/terrain` serves them as generated. `road_type` is now a typed enum on the client, and the JSON is the same. Swagger shows the shared names (`WaterRing`, not `WaterRingResponse`).
- **Rules.** `rules.rs` holds:
  - the world, tick, speed, range and spacing constants, which the backend now uses;
  - the `Zone` enum;
  - `quality_grade`;
  - the station-to-kit mapping;
  - `check_water_crossing`, the server's own movement test, which a client can use to predict `movement_blocked`. `TerrainConfig::validate_movement` delegates to it.
- **Message names.** `ClientMessage::type_name()` and `ServerMessage::type_name()` return the wire tag. P1/P2 can use this for the `on_event(name, data)` bridge. `ServerMessage::TradeAcceptedMsg` was renamed `TradeAccepted`; the wire tag `trade_accepted` didn't change.
- **Fixtures.** `protocol/tests/fixtures/server/*.json` were captured by serialising the backend's old `ServerMessage` enum before the move, so they are the server's real bytes. `tests/ws_fixtures.rs` round-trips every message type byte for byte.
- **Untyped responses.** The ad-hoc `json!` responses became `SuccessResponse` (logout, station removal) and `MoveResponse` (`POST /player/move`). Key order may differ, but the values are the same.

### 4.2 Rust for the wire and the world, Rhai for the feel

| Layer | Language | Responsibilities |
|---|---|---|
| `capitol-core` | Rust | Sockets, auth, reconnect, applying server messages, polling cadence, interpolation buffers, trade state machine |
| `capitol-client` world | Rust | Terrain mesh building, spawning and despawning entities, transforms for players, nodes and stations. This is the hot path, kept out of Rhai. |
| `capitol-client` UI | Rust (egui) | Login, inventory, crafting, station, exchange, trade and plot panels |
| `scripts/*.rhai` | Rhai, hot-reloaded | Camera rig, input to intents (click priority), world HUD (names, rings, toasts), juice, audio cues |

Scripts call into Rust through registered functions such as `cap_move(x, z)`, `cap_extract(node_id)`, `cap_open_station(id)`, `cap_state()` (returns a map), `cap_entity_at(x, z)` and `cap_set_panel(name, open)`. They receive server happenings through `on_event(name, data)`, for example `"extraction_completed"`, `"zone_changed"` and `"movement_blocked"`.

### 4.3 Game UI: egui themed as The Capitol

Panels (login, inventory, crafting, station, exchange, trade, plot) are drawn with **egui**, styled with the Capitol tokens:
- Panels use `rgba(15,15,25,0.95)` with 1px `rgba(80,80,100,0.3)` borders and 10–12px rounding.
- Accents: gold `#c9a227`, crafting purple `#8b5cf6`, in-range orange `#f97316`.
- Body text is `#e0e0e0`. The project font replaces egui's default.

*Why:* the panels need text entry (login, exchange quantity, trade strands), scrolling lists, drag and drop between slots, and numeric fields. egui has all of these today. Flint's data-driven `.ui.toml` UI has Panel, Text, Rect, Circle and Image, so it would need at least four new widgets plus focus handling and IME.

*Alternative recorded:* extend Flint's data-driven UI with TextField, ScrollList, Draggable and NumberField, and build the panels in `.ui.toml`. Revisit this if themed egui still looks like a tool after the P5 theme pass, or once Flint's UI has gained those widgets for another game.

World-attached HUD (name labels, extraction and crafting rings, the targeting line, toasts, the zone banner) uses Flint's script drawing through `world_to_screen`. That keeps it hot-reloadable and in the same visual language as the other Flint games.

### 4.4 Engine hooks (upstream to Flint)

These go to Flint on a `feature/*` branch and PR, following the subtree conventions (vault: *Git Subtree Branch Conventions*). Each one gets an ADR and a page in the Flint book. H1–H5 are ADRs 0071–0075. Flint's ADRs live in `C:\Users\chris\dev\Starchild\docs\adr\`, not in the Flint repo, and the next free number is 0076. Each is general enough that any Flint game could use it.

| ID | Hook | Shape | Size |
|---|---|---|---|
| H1 | **`PlayerExtension` trait.** The first step toward the roadmap's plugin system. | `fn register_script_api(&mut self, engine: &mut rhai::Engine)`, `fn pre_update(&mut self, ctx: &mut ExtensionCtx)` (world, renderer and device for meshes, clock, input), `fn on_window_event(&mut self, ev) -> bool` (consumed), `fn ui(&mut self, egui: &egui::Context)`. Installed with `PlayerApp::add_extension(Box<dyn PlayerExtension>)`. | M |
| H2 | **Script event delivery.** | Route `GameEvent::Custom { name, data }` to a new `on_event(name, data)` callback in `ScriptEngine::process_events`. Add `ExtensionCtx::fire_event` as well. | S |
| H3 | **Game egui layer.** | Run the extension `ui()` callback every frame. Forward window events to egui first, and treat them as consumed when `wants_pointer_input()`/`wants_keyboard_input()`. Draw order: world, then script HUD, then game egui, then debug panels (vault: *egui layer ordering - panels vs script UI*). | M |
| H4 | **Runtime procedural mesh lifecycle.** | Create, replace and remove named meshes after init. Support vertex colours and per-mesh AABBs for frustum culling, and attach a mesh to an entity by name. | M |
| H5 | **Screen to ground.** | `screen_to_ground(x, y, plane_y) -> [x, z]` in Rhai, plus the Rust equivalent on `Camera`, built on `unproject_ray` in perspective and orthographic modes. | S |
| H6 | *(Polish, optional)* **Water on any mesh, plus mesh instancing.** | Reuse the ocean shader's waves and foam ramp on an arbitrary mesh, masked by distance to the edge so canals get foam at the banks. Add an instanced mesh draw path for repeated props such as houses, trees and stations. | L |

| H7 [#63](https://github.com/chrischaps/TheCapitol/issues/63) | **Emissive materials.** Found in P3.7. | `ImportedMaterial::emissive` (linear HDR, default zero; glTF `emissiveFactor`), added after lighting and before tonemapping so bloom picks it up, plus a per-vertex form for vertex-coloured runtime meshes. Replaces P3.7's down-normal workaround (`MeshBuf::glowing`). | S |

| H8 [#64](https://github.com/chrischaps/TheCapitol/issues/64) | **Schema type `"integer"`.** Found in P3.3. | `ocean.toml` and `terrain.toml` type four fields as `"integer"`, which `parse_field_type` doesn't know, so they fall back to `String` and the scene warns. Alias it to `int`, and catch unknown type names at load or in a test. | S |
| H9 [#65](https://github.com/chrischaps/TheCapitol/issues/65) | **Debug keys and render stats from captures.** Found in P3.6. | `--input-script` writes `InputState`, but F2–F4 are matched only on window events, so a capture can't show the stats overlay. Route scripted keys through the debug-key handler and/or add `--stats`. That replaces the Capitol's periodic stats log. | S |

| H10 [#66](https://github.com/chrischaps/TheCapitol/issues/66) | **Fog reaches everything drawn, at every zoom.** Found in P3.8. | The composite fog skips depth ≥ 0.9999 (about 2.9 km out at far = 4 km, never past 10 km), and the ocean fogs itself over a fixed 380–640 m whenever fog is on, whatever its density. Fog every non-sky pixel (the sky writes depth 1.0) and have the ocean follow the composite fog. Replaces the Capitol's 20 km far plane and the fog switch in `atmosphere.rs`. | S |
| H11 [#67](https://github.com/chrischaps/TheCapitol/issues/67) | **The mouse wheel reaches actions, scripts and input scripts.** Found in P3.9. | `InputState` has `MouseWheel` bindings and `process_mouse_wheel`, but `flint-player` never feeds it `WindowEvent::MouseWheel`, so wheel bindings never fire and scripts can't read the wheel. Feed it, add `mouse_wheel_x/y()` to Rhai and a `wheel` verb to input scripts. Replaces the Capitol's `cap_take_wheel()` (the extension collects the wheel in `on_window_event`). | S |
| H12 [#68](https://github.com/chrischaps/TheCapitol/issues/68) | **Input configs can name every key.** Found in P3.9. | `parse_key_code` knows letters, digits, the arrows, F-keys and a few others, but not `Home`, `PageUp`/`PageDown`, `Minus`/`Equal` or the numpad. Cover all of winit's `KeyCode` names, round-tripping. Lets the rig bind Home and +/- beside its H and R/F. | S |

H1–H5 are needed for the vertical slice. H6 belongs to P8. H7–H12 aren't blocking. H7, H9, H10, H11 and H12 have workarounds, and H8 costs only a warning. H7 should land before the light gets tuned again (P8). P3.8 tuned around it.

**What the P0.4 spike found** (branch `spike/p0.4`; its subtree patches are marked `SPIKE(P0.4)`):
- **Embedding.** `PlayerApp` already implements `ApplicationHandler` publicly. A wrapper that forwards all nine callbacks works, and the capture flags (`--screenshot`, `--exit-after`) work through it. `PlayerApp::new` plus the public fields `main.rs` sets are enough to boot a scene. The game must pass `engine/schemas` itself.
- **H1 (Rhai).** Registering on the `rhai::Engine` before the scene loads works. Scripts call native functions with no other change. The engine uses rhai's `sync` feature, so every native function and the state it captures must be `Send + Sync` (`Arc<Mutex<…>>`, atomics). `register_script_api` should take `&mut rhai::Engine` and run before `load_scripts_from_scene`.
- **H4 (meshes).** Uploading with `load_procedural_mesh` after init works. An entity whose `model.asset` names the mesh draws it, because `update_from_world` rebuilds the draw list every frame. The scene's model loader then logs a "Model file not found" warning for each runtime-only name, so H4 should give runtime meshes a way to be declared or skip them quietly. **Vertex colours are linear** (the surface is sRGB), so the React hex tokens need an sRGB-to-linear conversion; without it the palette washes out. Use `ImportedMaterial { use_vertex_color: true, .. }`.
- **H3 (egui).** One extra callback inside the existing egui pass, after `render_draw_commands` and before the debug panels, gives the draw order H3 wants. With that callback installed, every window event goes to `egui_winit` first. A focused `TextEdit` consumes the keystrokes, and the rest still reach the game. Tested with real keystrokes sent to the window, including backspace. Not tested yet: IME, and Escape while a text box has focus, which flint-player treats as quit when the cursor isn't captured. H3 must handle that case.
- **Camera and cursor.** With no player entity, the cursor isn't captured and script `set_camera_position`/`set_camera_target` drive the view, which is what the Capitol's camera rig needs.
- **Small gaps.** The renderer has no clear-colour setter. The `sky` component (gradient, stars) covers the background instead. Scene entities don't need an archetype.

**How H1 was built (P1.1).**
- **Shape.** `flint_player::PlayerExtension` has the four methods in the table, each with a no-op default. `PlayerApp::add_extension` calls `register_script_api` straight away, on the engine from the new `ScriptSystem::rhai_engine_mut()`. Flint's API is registered first, so a same-signature registration replaces Flint's. The engine survives scene transitions, so registrations last the whole run.
- **`pre_update`** runs each frame after the clock ticks, the input script applies and the transition advances, and before game events are collected and `on_update` runs. Extensions run in the order they were added. `ExtensionCtx` holds `world` (mutable), `clock`, `input`, `camera` (last frame's) and `gpu: Option<ExtensionGpu>`, which groups the renderer, device and queue and is `None` until the renderer exists. H2 adds `fire_event` to it, and H4 builds the mesh lifecycle on `gpu`.
- **`on_window_event`** runs after open debug panels have had the event, and before the player's own handling. The first extension to return `true` consumes the event, but only input events can be consumed (keyboard, IME, mouse buttons, wheel, cursor movement, touch). Lifecycle events (`Resized`, `CloseRequested`, `RedrawRequested` and the rest) reach every extension and the player whatever they return.
- **`ui`** is declared but not called yet. H3 calls it and adds the egui routing.
- **No wrapper.** The spike's forwarding `ApplicationHandler` isn't needed any more. `capitol` runs `PlayerApp` directly with `CapitolExtension` installed, and `cap_spike_note()` is registered through the trait.
- **Where the hooks live until P1.6.** P1.1 to P1.5 commit their engine changes to master under `flint-client/engine/`. P1.6 pushes them all upstream as one PR. Until it merges, the subtree differs from `flint/main` on purpose.

**How H2 was built (P1.2).**
- **`on_event(name, data)`** is a broadcast. Every script that defines it gets every `GameEvent::Custom`, in entity-id order, including the script that fired it. `data` is always a Rhai map. A non-table payload arrives as `#{ value: .. }`. An error in one listener doesn't stop the others. Events are dropped while scripts are paused, the same as input actions.
- **`ExtensionCtx::fire_event(name, data: impl Into<toml::Value>)`** queues onto the physics event queue right after the extensions' `pre_update`. Scripts see it in the same frame, before `on_update`, and audio triggers see it too.
- **Timing differs by source.** Extension and engine events arrive the same frame. A script's `fire_event_data` is a command drained after `on_update`, so it arrives next frame. The Capitol's network events go through the extension, so they don't pay that frame.
- **The payload is TOML, so there's no null.** The wire is JSON. When the client forwards server data to scripts, it must turn `null` into an absent key, or drop it.

**How H3 was built (P1.3).**
- **Layers.** `PlayerExtension::ui(&egui::Context)` runs every frame inside the player's one egui pass. Back to front: script UI (`on_draw_ui`, on the background layer), then each extension's `ui()`, then the debug panels. A game's egui panels share the background layer and are issued after the script UI, so they cover it. Its windows sit on Middle.
- **Debug panels moved to Foreground.** egui `SidePanel`s always paint on the background layer, so a game's egui window would have covered them. The F3/F4 panels are now fixed Foreground areas laid out the same way (columns from the right edge, bottom strips full width). The grip on a column's left edge resizes it, as the `SidePanel` handle did. The F2 stats overlay sits just left of the columns and no longer takes clicks.
- **Input routing** (`player_app/egui_input.rs`). With any extension installed, or any debug panel open, every window event goes to egui first. Keys and IME are consumed while an egui widget has keyboard focus. Pointer events follow egui-winit: buttons and the wheel over an egui area, and motion while egui owns a drag. Unlike egui-winit, Tab isn't consumed when nothing is focused. **A release follows its press**, so a key or button never sticks down on either side when focus moves mid-press. Escape while a text field is focused unfocuses the field instead of quitting.
- **For the Capitol.** Draw panels with `Window`/`Area` or side and top panels, never `CentralPanel`, because it covers the window and takes every click. The inscription window in `capitol-client/src/ui.rs` uses the §4.3 tokens and is the template for the P4/P5 panels.

**How H4 was built (P1.4).**
- **Lifecycle.** Most of it already existed, because `load_procedural_mesh` replaces a mesh uploaded under the same name. The new pieces:
  - `SceneRenderer::remove_mesh(name)`.
  - `ExtensionGpu::upload_mesh` and `ExtensionGpu::remove_mesh`.
  - `ExtensionCtx::attach_mesh(entity, name)`, which merges `{ asset = name, runtime = true }` into the entity's `model`.
- **Leaks.** Draw lists hold `Arc` clones of a mesh's buffers. A replaced or removed mesh's buffers are therefore freed at the next `update_from_world`. The per-entity bind cache (`e1aa69d`) holds only transform and material resources, and drops stale entries every frame. Measured in `flint-render/tests/runtime_mesh.rs`: across 1,000 replacements the application held 26 buffers before and after. On DX12 the allocator held 55,967,744 bytes before and after.
- **`model.runtime = true`** marks a scene model as uploaded by code, and the model loader skips it. This settles the spike's "Model file not found" question. Entities that the extension spawns or attaches don't need the flag in a scene, though `attach_mesh` sets it anyway.
- **Vertex colours.** The material path already supported them (`use_vertex_color`). H4 adds two helpers: `ImportedMaterial::vertex_colored(name)` and `flint_render::srgb_to_linear` (vertex colours are linear). The hook takes `flint_render::Vertex`. `flint-procgen`'s `MeshBuilder` vertices have no colour field, so P3 builds `Vertex` lists directly. The spike's `MeshBuf::annulus` in `spike_world.rs` is the starting point.
- **Culling.**
  - Every `GpuMesh` gets a local AABB at upload, on all upload paths, imported glTF meshes included.
  - Each opaque and transparent model draw carries its world AABB (Arvo's method).
  - `render_to` sets aside draws outside the camera frustum after the shadow pass, and gives them another chance at the next `render_to`.
  - The shadow pass culls the same draws against each cascade's frustum.
  - `RenderStats::culled_draws` counts them, and F2 shows `Entities: N (M culled)`.
  - Skinned meshes, wireframes and `bounds` boxes aren't culled.
  - The culling applies to every Flint game, so P1.6's ADR should cover it.
- **For the Capitol (P3).** Cull by sector: a mesh is culled whole, so one mesh for the whole city never culls. P3.6 already plans for this with plot sectors.
- **Client.** `CapitolExtension::pre_update` uploads the spike's ground and markers once, and rebuilds the water every frame with a slow twelve-second tide. The tide exercises replacement in the live client. `scenes/spike.scene.toml` declares the meshes with `runtime = true`. Over a 30 s live run, private memory held at about 588 MB.

**How H5 was built (P1.5).**
- **Shape.** Rust: `Camera::screen_to_ground(x, y, width, height, plane_y) -> Option<[f32; 2]>`. The cursor and the viewport size must use the same unit, either logical points or physical pixels. Rhai: `screen_to_ground(x, y, plane_y)` returns `[x, z]`, or `()` where the view doesn't reach the plane. It takes logical points, so `screen_to_ground(mouse_x(), mouse_y(), 0.0)` is right at any display scale.
- **One implementation.** Both sides call `flint_core::projection`. `flint-script` can't depend on `flint-render`, so the script side works from the view-projection matrix it already gets for `world_to_screen`. The two functions are exact inverses: the end-to-end test checks that `world_to_screen` of a scripted hit lands within 0.01 pt of the cursor. `Camera::unproject_ray` now delegates to the same module. It keeps its signature, honours `ndc_offset` and serves both projections.
- **f64 inside.** The inverse and the unprojection run in `f64`. Tests put the camera 400 m from the origin with a far plane 20,000 times its near plane, and every round trip lands within 2 mm.
- **Straight down was broken, and is fixed.** With `forward` parallel to `up`, `forward × up` is zero and the view matrix collapsed: a script camera placed exactly overhead rendered nothing. The view basis now falls back to north (-Z) at the top of the screen and east (+X) on the right, matching §4.6. This is the only change to existing behaviour. It applies to every Flint game, so P1.6's ADR should mention it.
- **Misses** return `()`/`None`: above the horizon, an orthographic camera looking level, or a plane behind the ray's near-plane origin. The near plane matters for a low orthographic camera, because the bottom of its view can be ground that sits behind the near plane and never renders.
- **Timing.** Scripts' camera matrix is now refreshed after the script camera overrides are applied and before `on_draw_ui` (`flint-player` `frame.rs`). So in `on_draw_ui`, `world_to_screen` and `screen_to_ground` use exactly the camera the frame renders with. Before this, HUD overlays trailed the world by one frame of camera motion. `on_update` still sees the previous frame's camera, because it runs before the script moves it. Pick in `on_update` (a click is a state change there), and draw reticles in `on_draw_ui`.
- **Client.** `spike.rhai` lays a gold reticle on the ground under the cursor, drawn in world space and foreshortened by the view. A click leaves its ripple on the ground rather than on the screen. A probe capture checked that the script's picking agrees with the rendered meshes. Fixed screen points, plus the projected moat south bridge `(0, 20)` and canal east bridge `(60, 0)`, came back centred on their crosshairs and on the water meshes, at this machine's 1.25 display scale.
- **For P4.3 (click to move).** `screen_to_ground(mouse_x(), mouse_y(), 0.0)`, then `coords::to_server`. For P6 plot picking, do the same, then point-in-polygon on the lot.

**How H1–H5 went upstream (P1.6).** [Flint PR #41](https://github.com/chrischaps/Flint/pull/41), branch `feature/player-extensions`: six commits on `flint/main` 8cc8306. Merged as Flint `main` 1481637, then pulled back with `subtree pull --squash`. The subtree matches `flint/main` and has no spike patches.
- **ADRs.** 0071 `PlayerExtension`, 0072 `on_event` and `fire_event`, 0073 game egui layer and input routing (including the debug panels' move to Foreground), 0074 runtime meshes and frustum culling, 0075 `screen_to_ground`, the overhead view basis and the script camera refresh. They're committed in Starchild's `docs/adr/`. The engine cites 0068–0070 but has no files for them, and they kept their numbers.
- **Book.** A new guide, *Player Extensions* (`docs/book/src/guides/player-extensions.md`), covers the game binary, the four hooks, the frame order, events, meshes, egui layering and input routing. It's linked from *Building a Game Project* and from the roadmap's plugin-system item. The engine CLAUDE.md has a one-line summary. `tools/docs-check/check.py` reports no drift. It isn't in `flint/main` yet (it's uncommitted in Chris's Flint checkout), so it was run from a temporary copy.
- **Engine comments** cite the ADR numbers instead of the Capitol's H1–H5 labels.
- **Commit messages.** The split carried `Closes #5`…`#9`, which would point at unrelated Flint issues, so the pushed branch rewrites them to `Part of chrischaps/TheCapitol#N`. The content is the same, so the post-merge `subtree pull --squash` doesn't care.

### 4.5 Networking and state (`capitol-core`)

- **IO thread.** A tokio runtime runs on one background thread, with `reqwest` (rustls) for REST and `tokio-tungstenite` for the WebSocket. The game loop talks to it through `Net`. `Command` goes in: `Connect{token}`, `Disconnect`, `Ws(ClientMessage)` or `Rest(RestRequest, tag)`. `NetEvent` comes out: `Ws(ServerMessage)`, `Rest(tag, Result<…>)` or `Connection(state)`. The game drains events once per frame in `pre_update`.
- **Handshake:**
  1. Connect to `/ws`.
  2. Send `{"type":"auth","token"}`. The server allows 10 s (`ws/connection.rs:32-92`).
  3. On `auth_ok`, send `subscribe{channel:"position"}`. Until then the server silently drops every event (`connection.rs:509-511`).
  4. Send a WebSocket Ping every 15 s. The server answers Ping frames with Pong (`connection.rs:497-499`) but has no idle timeout of its own.
- **How P2.1 built it** (`crates/capitol-core/src/net/`):
  - **Channels.** Commands travel on a tokio unbounded channel, not `std::sync::mpsc`. Its `send` is just as non-blocking for the game, and the IO loop can `await` it without a bridging thread. Events come back on `std::sync::mpsc`, so the game uses a plain `try_iter`.
  - **Routes.** `RestRequest` has one variant per non-admin route, and each decodes to one `RestResponse` variant. The bearer token from the last `Connect` is attached to every call. Failures are `RestError::Http{status, message}` (the body's `error` field), `Transport` or `Decode`; `is_unauthorized()` flags a 401. A GET that fails to send is sent once more (added in P2.7). A pooled keep-alive connection the server has just closed fails with "connection closed before message completed", and the poller doesn't retry. Nothing else is repeated, because the server may have acted on it.
  - **Connection states.** `Connecting → Authenticating → Connected{player_id}`, ending in `Disconnected{reason}` or `AuthFailed{message}`. `Connected` means `subscribed` has arrived, so events are flowing. P2.2 added `Reconnecting` between sessions (below). Handshake replies are forwarded as `NetEvent::Ws` too.
  - **Held messages.** `Ws` commands sent during the handshake are held and flushed after `subscribed`. With no session they're dropped with a warning.
  - **Liveness.** Ping every 15 s. If nothing arrives for 45 s, the session ends with `Disconnected{"server stopped responding"}`. The handshake must finish within 10 s.
  - **Malformed frames.** Frames that don't parse as `ServerMessage`, including unknown types, are logged and skipped. The session stays up.
  - **Closing.** On close, the client waits up to 1 s for the server's Close reply before dropping the socket. On Windows, dropping a socket with unread data sends a reset, and the server could lose the Close.
  - **Connecting.** The client connects to every resolved address at once. `localhost` resolves to `::1` first and the backend listens on IPv4 only, so trying addresses in order added about 2 s to every connect on Windows.
- **Reconnect.** Back off exponentially from 0.5 s up to 10 s. After reconnecting, auth and subscribe again, then **resync** by refetching `/player/me` (name, position, currency), containers, nearby nodes, nearby stations, plots and recipes. A lagging client that overflows the 1024-event buffer is disconnected by the server (the `Err(_)` arm of the event loop in `connection.rs`, which also catches `Lagged`). Reconnect covers that case too.
- **How P2.2 built it** (`net/ws.rs`, the session task's `run` loop):
  - **States.** A dropped connection, from any phase, emits `Reconnecting{attempt, delay, reason}` and then starts over at `Connecting`. Nothing ends a session except `Disconnected` (the game called `disconnect` or `connect`, or dropped `Net`) and `AuthFailed`. So `Disconnected` now means "closed by the game", not "retry this". Retries never give up; the game shows an overlay (P4.5) and can `disconnect`.
  - **Backoff.** 0.5, 1, 2, 4, 8, then 10 s (`NetConfig::reconnect_initial`/`reconnect_max`). Each wait is trimmed by a random 0–20 %, so clients dropped together don't return on the same tick. The attempt count resets once a session has stayed connected for `reconnect_max`, so a server that accepts and then drops at once can't hold the client at 0.5 s.
  - **Resync.** `NetEvent::Resync` follows `Connected` when an earlier connection in the same session had gone live. The net layer only signals; the state store (P2.3) does the refetching. A first connect that needed retries has nothing stale, so it gets no `Resync`.
  - **Which `auth_error`s are final.** The server sends `auth_error` for a bad token (`Invalid token`), another login (`Session expired`) and a missing player, but also for `Auth timeout` and `Database error: …`. The last two say nothing about the token, so they're retried; the rest end in `AuthFailed`.
  - **Messages while down.** `Ws` commands sent while waiting to reconnect are dropped with a warning, not queued. Held messages during a handshake work as before.
  - **Known server gap ([#59](https://github.com/chrischaps/TheCapitol/issues/59), B8).** When the client gives up on a half-open socket, the server's old handler can outlive the new connection. When its writes finally fail, it calls `remove_player` for the same `player_id` and removes the player the new connection just added. Normal drops and the lag cut-off aren't affected, because the server ends its side first.
- **Auth.**
  - Tokens last 24 h and there is no refresh endpoint (`routes/auth.rs:277-296`). A stored token is tried first, and an `auth_error` returns the player to the login screen.
  - Only one session per account is allowed, enforced in Redis (`middleware/auth.rs:59-69`). Logging in from the Flint client logs the same account out of React.
  - The token is kept with `PersistentStore`.
  - **`signin::SignIn`** (P3.10) logs in, and registers when the login gets a 401, one `NetEvent` at a time, as `Bot::sign_in` does in a blocking loop. P4.1's form drives it.
- **Polling.** These messages are defined but never sent: `currency_changed`, `nearby_nodes`, `nearby_stations`, `item_moved`, `items_merged`. The client mirrors React's polling instead:
  - nodes, stations and plots every 5 s
  - containers after `inventory_updated`, `trade_executed`, extraction and crafting completion
  - currency after any trade or exchange
- **How P2.4 built it** (`crates/capitol-core/src/poll.rs`):
  - **Pure, with an injected clock.** `Poller::observe(&NetEvent, &[StateEvent], &GameState)` runs once per net event, after the store applies it. `Poller::tick(now)` runs once per frame and returns `(RestRequest, Tag)` pairs for the game to send. The poller never touches `Net`. Its tags start at `poll::FIRST_TAG` (`1 << 63`), so the game's own tags stay below that. `observe` returns the `Fetch` an event answered, which lets the game route the `/terrain` reply to the world (P3.10).
  - **The rules** are in a table in the module docs. Beyond the list above: `station_opened` fetches that station's container. An `inventory_updated` for the open station's container fetches that container, not `/inventory/containers`. Your own `station_placed` fetches containers. So does a REST `move_item`, which also fetches the open station's container. `zone_changed` fetches plots, but only on a real crossing (`from: Some`), because the snapshot already covers the first zone. `terrain_updated` fetches `/terrain`.
  - **One in flight per fetch.** A repeat ask while one is queued does nothing. A repeat while one is *in flight* is sent once more after the answer arrives, because that answer may predate the change. It usually does: the tick sends `extraction_completed`, the item is written afterwards, and only then is `inventory_updated` sent (`engine/tick.rs`, after `game.tick()`). So the refetch on completion often reads the old inventory, and the follow-up is what lands the item. A live run showed it: two `Containers` fetches 30 ms apart, with the fiber arriving on the second.
  - **Trades.** Only `trade_executed` triggers a refetch (containers and currency). A trade holds nothing in escrow, and a failed execute rolls back, so a cancel changes nothing. Since P2.6 the poller keys on `TradeEvent::Executed`, which the store emits only for our own trade.
  - **Timer.** It runs only while connected and restarts a full interval after each snapshot. After a stall it fires once, not once per missed interval. Losing the connection clears anything queued, because the resync refetches all of it.
  - **No retries.** A failed fetch is dropped. The 5 s poll covers nodes, stations and plots, and the resync after a reconnect covers the rest. (The REST layer itself sends a failed GET once more; see P2.1 above.)
  - The live test's hand-rolled `Mirror` now runs on the `Poller`.
- **Filtering.** Many messages are broadcast to everyone, including extraction, crafting, `inventory_updated` and `positions`. The state layer filters by `player_id` and never assumes a message is about the local player.
- **How P2.3 built it** (`crates/capitol-core/src/state/`, plus `catalog.rs`):
  - **A pure reducer.** `GameState::apply(&NetEvent) -> Vec<StateEvent>`. It does no IO and keeps no clock. `StateEvent`s are facts: `SnapshotStale`, `InventoryChanged{container_id}`, `StationOpened`, `PlayerJoined`, `NodeAdded`, `ExtractionCompleted{item}` and so on. The store never asks for a refetch itself; P2.4 maps events to REST calls. Notifications aren't stored either: the UI toasts from events.
  - **Snapshots.** `SnapshotStale` comes on the first `Connected` and on every `Resync`. A resync also clears every player's activity and the open station, because the server dropped the player (`remove_player`) and re-added them idle.
  - **`positions` replaces the player set.** It lists every online player (`broadcast_positions`), so anyone missing has gone, even if their `player_left` was lost. Joins and leaves are reported as diffs.
  - **Nodes and stations merge.** Nearby answers upsert and report only real differences, so the 5 s polls are quiet. Entries out of range stay, as in React. The first answer after a resync *replaces* instead, which drops anything removed while the client was away.
  - **Activity is one entry per player.** The server allows one action at a time (`ActionState`), so `activities: player → Extracting | Crafting` holds everyone. The local player's starts and finishes come out as `ExtractionStarted`/`CraftingCompleted` and so on. Anyone else's come out as `ActivityChanged{player_id}`, so the world can show other players working. A finish for the local player is reported even if its start was missed.
  - **Inventory filter.** `inventory_updated` counts if it's from the local player *or* names a container the store holds (the player's own, or the open station's). That covers two players at one storage chest, which React's filter would miss.
  - **Zone.** The server sends `zone_changed` only on a crossing. So the first zone is computed from the first known position (`Zone::from_position`) and reported as `ZoneChanged{from: None, ..}`.
  - **The local player (B2).** The snapshot's `GET /player/me` puts the local player into `players` (with `PlayerJoined`) and sets the balance, so name, position and zone are known before the first `positions`. It only adds a player that isn't there yet, and only while connected and `online`. Once `positions` has placed them, a REST answer is older than the 100 ms stream, so it never moves them. `Fetch::Me` replaced `Fetch::Currency` in the snapshot; `Currency` is still fetched after trades and exchanges.
  - **Static data.** `catalog.rs` hardcodes item, station, resource and container types from the migration seed rows. `station_placed` uses it to fill in category, icon and interaction range.
  - **Trades** come out as `StateEvent::Trade(TradeEvent)`, from the state machine below (P2.6).
  - **Coverage.** The reducer tests map every `ServerMessage` variant to its test in an exhaustive `match`, so a new variant doesn't compile without one. They also run every protocol fixture through the store. The live test `the_store_mirrors_a_real_session` drives a real session: snapshot, walk, extract, then refetch the inventory.
- **Interpolation.**
  - `positions` arrives every 100 ms with no velocity (`engine/tick.rs:480-493`).
  - Other players render about 150 ms behind real time, lerped between the two surrounding snapshots, with a velocity estimate for brief gaps.
  - The local player is predicted: the client moves toward the clicked destination at 50 u/s, which mirrors `engine/movement.rs`. When a snapshot disagrees by more than about 4 units, the client blends back. `movement_blocked` snaps to `stopped_x/y`.
- **How P2.5 built it** (`crates/capitol-core/src/motion.rs`):
  - **The API.** `Motion::observe(now, &[StateEvent], &GameState)` runs once per net event, after the store, like the poller. `move_to(now, x, y)` starts the local prediction; the game sends `ClientMessage::Move` itself. `sample(now) -> &[Pose]` runs once per frame and returns `{id, x, y, facing, moving}` per player, the local player first, in server units. Facing is radians in server space (0 east, π/2 south). `destination()` is for the click marker. `corrections()` and `lag()` are for a debug overlay.
  - **Snapshot stamps.** Arrival times bunch, so they aren't used as-is. The server's tick is a `tokio::time::interval`, which bursts to catch up, so every snapshot is exactly one tick after the last. A snapshot is stamped one tick after the previous stamp, but no later than its arrival and no more than 200 ms before it. Three snapshots that arrive together after a stall are spread back over their ticks. The stamp also slews toward late arrivals by 5 ms per snapshot. That's a backstop for a lost snapshot, which this socket can't actually produce: a lagging client is disconnected, and a late tick is still sent.
  - **Other players.** Each has an 8-sample buffer, drawn 150 ms behind. Past the newest sample, the last velocity carries a player on for at most 150 ms, then they hold still with `moving: false`. A jump of more than 50 units between snapshots (ten ticks of walking) resets the buffer, so a teleport snaps.
  - **Checking the local player against the server.** A snapshot is checked against the *trail*, not the current prediction. The trail is the predicted path over the last 500 ms. The server runs about a round trip behind, so on a straight walk its position lies on that trail. Comparing it with the current prediction would "correct" every walk by RTT × 50 units. The match also gives a learned lag (`lag()`). A live walk on loopback learned 20–120 ms and made no corrections.
  - **Corrections.** More than 4 units off the trail is a disagreement:
    - If both are walking, the prediction moves to the server's position advanced by `lag` toward our destination.
    - If the server stood still between snapshots, it isn't following us (a lost move, or an action that stopped it), so the prediction stops at the server's position.
    - The drawn position blends there with a 100 ms time constant.
    - A gap over 50 units snaps instead.
    - `movement_blocked` snaps. `extraction_started`/`crafting_started` stop the prediction, because the server clears the destination.
  - **Along-path error is tolerated up to the window.** A server lagging behind on the same path looks like lag until its position leaves the 500 ms trail, which is about 25 units at walking speed. That rides out lag spikes. The cost: a server walking slower than 50 u/s is only corrected once it falls that far behind. Speed is always 50 unless `players.speed` is edited by hand.
  - **Following a walk the server resumed (found in P2.5).** `load_player` (`ws/connection.rs`) reloads `destination_x/y` on every login and reconnect. So the server can resume a stored walk the client never clicked, and the resync clears the client's own destination. When the server walks while the prediction stands, the prediction *follows*: its destination is renewed each snapshot to a point a tick ahead of the server on its heading, and it stops the moment the server does. `destination()` stays `None` while following, so no marker is drawn for a walk the player didn't click.
  - **No allocations per frame.** `sample` refills a buffer the `Motion` owns. Tracks and the trail are fixed arrays. `tests/motion_alloc.rs` checks this with a counting global allocator: 50 players, 600 frames, no allocations.
- **Trade state machine.** `Idle → Requested(out) / Incoming(from) → Open(trade_id) → (Accepted mine/theirs) → Executed / Cancelled`. It handles three protocol quirks:
  - Failures to initiate, accept a request, offer or accept are only logged on the server, so the client times out after 5 s with a hint.
  - `trade_cancelled` on disconnect carries a dummy `trade_id` (`engine/tick.rs`, `remove_player`, a fresh `Uuid::new_v4()`). `trade_cancelled` and `trade_executed` are broadcast to *every* client (`ws/connection.rs`), so "treat any cancellation as ours" would let a stranger's cancel close our trade. (Found in P2.3.)
  - No `inventory_updated` follows a trade, so the client refetches containers and currency itself.
- **How P2.6 built it** (`crates/capitol-core/src/state/trade.rs`):
  - **In the store.** Deciding which trade messages are ours is filtering, and a trade ends when its partner leaves, so the machine is part of `GameState`. `trade()` returns `Idle`, `Requested{to}`, `Incoming{from}` or `Open(OpenTrade)`. `OpenTrade` holds both offers as the server last confirmed them, and both accept flags. Ends aren't states: the trade returns to `Idle`, and `TradeEvent::Executed` or `Cancelled{reason}` is emitted for the UI to toast.
  - **Actions** (`trade_initiate`, `trade_accept_request`, `trade_decline_request`, `trade_offer`, `trade_accept`, `trade_cancel`, `trade_withdraw`) check what the server would silently swallow and return a `TradeRefusal` instead: yourself, offline, out of `TRADE_RANGE` (with the distance, so P7.3 can walk closer), already trading, already asked, no request, no trade. Otherwise they return the `ClientMessage` for the game to send. Nothing changes locally until the server echoes it, so an offer appears when its `trade_offer_updated` arrives.
  - **Timeouts.** The store still never reads the clock. `GameState::tick(now)` runs once per frame and reports `TradeEvent::NoReply(action)` after 5 s (`REPLY_TIMEOUT`). `TradeAction::hint()` has default text. What a timeout does depends on the action:
    - An unanswered accept-request drops the request.
    - An unanswered cancel closes the trade, because the server only fails a cancel when it has no trade for us.
    - Offers and accepts leave the trade as the server last said.
    - A request is different: the server never tells the asker it arrived, so silence is normal. `NoReply(Initiate)` is only a hint, and the state stays `Requested`. The server keeps a request until it's answered or either player leaves; `cleanup_stale_requests` is never called. The server can't withdraw one, so `trade_withdraw` only forgets it locally. If the other player accepts later, the trade still opens.
  - **Simultaneous requests.** If we've asked B and B's request reaches us, the server has already started the trade (`TradeManager::request_trade`). So the client shows no prompt and waits for `trade_started`. Asking someone who asked us does the same: the client sends `trade_initiate` and the server treats it as an accept.
  - **Accept after offer.** The server queues an offer for the next tick but handles `trade_accept` at once in the WS handler, so an accept sent right after an offer lands first and is then reset by it. `trade_accept` is refused (`OfferUnconfirmed`) until our offer's echo arrives.
  - **The final `trade_accepted` can be lost.** The second accept sends that broadcast, then `execute_trade` takes the trade out of `active_trades`. Each connection filters the broadcast against `active_trades` when it reads it. A connection that reads it after the removal finds no participants and drops it. One accept followed by `trade_executed` is a normal ending.
  - **Disconnects: the leave, not the cancel.** The dummy-id cancel is ignored. `remove_player` always cancels before it sends `player_left`, so a leave by the partner (or by us, see B8) ends the trade with reason `Player disconnected`. A partner missing from `positions` counts as a leave. This replaces the plan's "hold the cancel until the leave": the leave carries all the information, and holding would add nothing. Once B6 lands, the real-id cancel ends the trade first, and the leave finds nothing open.
  - **Own connection.** Leaving `Connected`, or a `Resync`, ends any trade (`Connection lost`) and any request, because the server's `remove_player` dropped them.
  - **Late execute.** After a cancel timeout, a `trade_executed` for that trade still reports `Executed`, so the refetch runs.
  - **B5.** When the server sends `error` for trade failures, react to it while `trade_waiting()` is set and keep the timeouts as a backstop.
  - The live test `two_players_trade` runs two fresh accounts through a 10-strand trade (the balances refetch to 90 and 110), a cancel, and a disconnect mid-trade that ends via the leave.
- **Quality grades.** Use the server's thresholds: A ≥ 90, B ≥ 75, C ≥ 55, D ≥ 35, otherwise F. Since P0.3 they live in `capitol_protocol::rules::quality_grade`, and the backend calls the same function. WebSocket events carry only the numeric quality, so the client computes the grade.

- **How P2.7 built the bot** (`crates/capitol-core/src/bot.rs`, binary `capitol-bot`):
  - **One frame loop for everything.** `Bot` owns a `Net`, a `GameState` and a `Poller`, wired as the game wires them. `frame()` waits up to 16 ms, applies events, runs `state.tick` and `poller.tick`. Every action (`walk_to`, `scout`, `extract`, `gather`, `craft`, `wander`) is frames until a condition holds, so a bot only runs inside its own calls. Each bot in a fleet gets its own thread.
  - **The script** (`run_script`): log in, or register on a 401. Scout for fiber. Walk to the nearest available node and extract it, repeating until the inventory holds what `rope_basic` takes. Craft the rope, then report: walked distance, yields, the rope's quality and grade, and the time taken.
  - **Scouting.** Since B1 ([#29](https://github.com/chrischaps/TheCapitol/issues/29)), six nodes stand within the 200 that nearby covers of the spawn, so a fresh bot finds one at once. With no node known, the bot walks toward the Capitol and asks `/world/nearby` every second. It stops as soon as a node appears, or 400 units from the centre.
  - **Silent refusals.** The server only logs a refused `extract` (`engine/tick.rs`, `start_extraction`). This happens when another player got there first or the node is out of range. So no `extraction_started` within 3 s means refused, and the bot skips that node for 10 s, by which time a poll has the truth. A live race of three bots for one node showed it: two lost, moved on, and all three made rope.
  - **A craft lands in two refetches.** The tick writes the output and sends `inventory_updated`, then deletes the inputs and sends another (`engine/tick.rs`, after `game.tick()`). A refetch between them shows the rope with the fiber still there. `craft` waits for the output *and* for the inputs to drop. Waiting for the poller to go idle isn't enough, because the second `inventory_updated` may not have arrived yet.
  - **Flags.** `--accounts N` runs N bots on accounts `<prefix>1@bots.capitol.local` and up (`--prefix`, default `bot`), registered on first use. `--wander` strolls within `--radius` of where each bot starts, pausing between walks. `--near <player>` wanders around that player and follows them, which is the easy way to watch bots from another client. `--seconds` bounds a wander.
  - **Live tests** (`tests/bot.rs`, `#[ignore]`). `the_bot_crafts_rope_and_survives_a_dropped_connection` runs the script through a TCP proxy in the test, then cuts every connection. The bot must reconnect once, resync, refetch an inventory that matches, and walk again. `wanderers_gather_around_a_player` has three `--near` wanderers walk over to a watcher who stepped 200 units away.

### 4.6 Coordinates and scale

Server space runs 0–8000 on both axes with the centre at (4000, 4000), y pointing down, and angle 0 = East, 90 = South (`models/terrain.rs:3165`). Flint is Y-up. All conversion goes through one module, `coords.rs`:

```
world.x =  (x - 4000) * S
world.z =  (y - 4000) * S        // server "down" = +Z = south; north is -Z
world.y =  height layer
```

`S` is a single constant, **S = 0.1**: the world is 800 m across, a player (radius 8) is about 1.6 m wide, a 60-unit arterial is 6 m, and the moat is a 2 m channel 20 m from the centre.

**Verified in P0.4.** Flint is **right-handed and Y-up**. `Camera::view_matrix` (`flint-render/src/camera.rs`) builds `right = forward × up`, so a camera looking toward -Z has +X on its right. With the mapping above, the map isn't mirrored: from above, with north (-Z) at the top of the screen, east (+X) is on the right, as in the 2D clients. The spike rendered the moat's single bridge (server angle 90) on the south side, facing a camera placed south of centre, and an east marker (angle 0) on the right. S = 0.1 read well at that distance. `coords.rs` has the mapping and tests (`to_world`, `to_server`, `polar_to_world`).

Flat geometry is layered in Y to avoid z-fighting. Since P3.3 the land stands above the sea: the sea (the engine ocean) at 0, the canal beds at -1.6, the ground at +0.6, roads +0.04 above it (by type: trail < suburban < grid < arterial, up to +0.10), plots at +0.12 above it like a pavement, with their curbs a few centimetres higher, and the canals' stone kerbs at +0.3 above it. *(P3.6 moved plots from below the roads to above them. Lots are clipped clear of every road, so the two never overlap.)* The ground is cut away over each water ring, so the ocean shows only in the channels. Roads must stop at the kerb rather than run over the channel, and bridge decks go above the kerb.

These live in `coords::layer` (P3.2): `GROUND`, `PLOT` (raised over the roads in P3.6), and `road(RoadType)`, which steps 2 cm per rank from `ROAD` (P3.4 widened it from 5 mm, finer than the map view resolves), so arterials sit 10 cm above the land. P3.3 added `SEA`, `CHANNEL_BED` and `BANK`, and lifted `GROUND` to 0.6 (the rest follow it). Scripts read the land height as `cap_ground_y()`. P3.5 added `DECK` (a bridge's crown, 0.4 m over the kerb) and `ARCH_SPRING` (0.2 m under the sea, where its arch leaves the channel walls). P3.7 added `BAILEY`, the paving inside the moat, level with the arterials so the moat bridge's ramp lands on it 2 cm up. **Depth precision caps how thin the layers can be.** The depth buffer is standard (not reversed) Z, so at 1 km the smallest step it can resolve is about `distance² / (near × 2²⁴)`: roughly 0.6 m with the default `near = 0.1`, and 6 cm with `near = 1`. The scene sets `near = 1.0` (and, since P3.8, `far = 20000.0`). *(P3.9 checked the map at 640 m with near = 1 and found no z-fighting, so the rig doesn't move `near`, and Flint gives scripts and extensions no way to change it at runtime anyway. The urban ring does show a fine speckle at the map, as it did in P3.8: sub-pixel curbs aliasing, not depth. If shimmer shows when moving, that's the time to ask Flint for a runtime near plane or MSAA in the client.)*

### 4.7 Visual language carried over

Tokens come from `client/src/index.css` and `components/GameCanvas.tsx`.

**Base colours**

| Token | Value |
|---|---|
| Background | `#0a0a0f` |
| Panel | `#12121a` |
| Text | `#e0e0e0` |
| Dim text | `#808080` |
| Gold | `#c9a227` |
| Gold dim | `#8a7019` |

**Zones**

| Zone | Colour |
|---|---|
| Capitol | `#c9a227` |
| Trade | `#3b82f6` |
| Guild | `#8b5cf6` |
| Urban | `#64748b` |
| Suburban | `#22c55e` |
| Rural | `#84cc16` |
| Wilderness | `#1a1a24` |

**Roads**

| Road type | Colour |
|---|---|
| Arterial | `#4a4a5a` |
| Grid | `#3a3a4a` |
| Suburban | `#2a2a3a` |
| Trail | `#1a1a2a` |

**Water, bridges and grades**
- Water: `rgba(30,100,160,0.7)` with a highlight of `rgba(100,180,220,0.4)`.
- Bridges: `#8b6914`.
- Quality pips (slot palette):

| Grade | Colour |
|---|---|
| A | gold `#ffd700` |
| B | `#22c55e` |
| C | `#3b82f6` |
| D | `#a855f7` |
| F | `#6b7280` |

In 3D these become ground and material tints rather than literal fills. The flat map at maximum zoom uses them directly.

## 5. Backend prerequisites

These are small server changes the client depends on or would expose. They are tagged by the phase they block, and none is large.

| ID | Change | Why | Blocks | Size |
|---|---|---|---|---|
| B1 [#29](https://github.com/chrischaps/TheCapitol/issues/29) | **Done.** Migration `20260212_001_world_scale.sql`. New players spawn at `rules::SPAWN_X/SPAWN_Y`: (4000, 7350), on the south arterial just inside the rural edge, scattered ±20 across the road. That road runs straight to the moat's one bridge. Old-world players (the 0–2000 square, or off the disc) move there. The 50 corner nodes are replaced by 156: 6 starter nodes 60–170 from the spawn, 60 suburban and 90 rural. They're placed against the generated default terrain, at least 20 clear of every road, and rural fiber is better (quality 50–70 against 40–60). A test checks them against the generator. They don't follow an admin's terrain regeneration ([#70](https://github.com/chrischaps/TheCapitol/issues/70)). The zones table matches `Zone::radii()`. `/player/move` accepts 0–8000. DB plots still on the 2000 world are scaled 4× about the centre (ids, owners and stations kept, the grid sparser). The real regeneration is left to B7. Migrations check out as CRLF, which breaks sqlx checksums ([#69](https://github.com/chrischaps/TheCapitol/issues/69)). Run the backend from a worktree with the LF blobs restored. | New players spawned in the far wilderness corner with every resource near them. A 3D world with a visible tower makes that obvious. | P4 | M |
| B2 [#30](https://github.com/chrischaps/TheCapitol/issues/30) | **Done.** `GET /player/me` → `{player_id, name, x, y, zone, strand_balance, online}`. While the player is in the game, name and position come from the tick loop's `GameState`; otherwise from the `players` row (written every 10 s), with `online: false`. The balance always comes from the row, because trades, the exchange and tax write it there and the in-memory `strand_balance` is never updated after login. `zone` is the zone ID at that position. capitol-core fetches it first in the connect/resync snapshot (§4.5, P2.3). | Today your own position and name only arrive in the first `positions` broadcast | P4 (nice to have) | S |
| B3 [#18](https://github.com/chrischaps/TheCapitol/issues/18) | **Done.** `CompressionLayer` (tower-http gzip) on `/terrain`, plus a `version` field: FNV-1a over the generated terrain's JSON, 16 hex digits. It hashes the output, not the params, so a generator change bumps it too. The same value is a weak `ETag` (`W/"<version>"`) with `Cache-Control: no-cache`, and `If-None-Match` with it answers 304 and no body. | About 1 MB of uncompressed JSON on every login (971 KB; 213 KB gzipped). With the ETag, a cached client downloads nothing when the terrain hasn't changed. | P3 | S |
| B4 [#41](https://github.com/chrischaps/TheCapitol/issues/41) | Fix the `move_item` ownership check for station containers (`routes/inventory.rs:218`, `ws/connection.rs:1127`) | You can't move items into a storage chest | P6 | S |
| B5 [#47](https://github.com/chrischaps/TheCapitol/issues/47) | Send `error{message}` when `trade_initiate`, `trade_accept_request` or `trade_offer` fails | Trade failures are silent today | P7 (nice to have) | S |
| B6 [#48](https://github.com/chrischaps/TheCapitol/issues/48) | Use the real `trade_id` in the disconnect `trade_cancelled` (`engine/tick.rs:109`) | The dummy UUID can't be matched to the trade | P7 (nice to have) | S |
| B7 [#42](https://github.com/chrischaps/TheCapitol/issues/42) | **Decide how to unify the plot systems.** Terrain plots are visual, with string IDs. Claimable DB plots are axis-aligned boxes with UUIDs, and station placement checks against them. | The 3D world draws terrain plots, but you claim DB plots, and the two don't line up | P6 (decision needed) | M–L |
| B8 [#59](https://github.com/chrischaps/TheCapitol/issues/59) | On disconnect, remove the player only if the ending connection is still the player's current one (`ws/connection.rs` end of `handle_socket`, `engine/tick.rs` `remove_player`) | A stale handler, from a half-open socket the client reconnected past or a client logged out by a newer login, removes the player the new connection is serving | P2.2 reconnect, P4 (two-client play) | S |
| B9 [#60](https://github.com/chrischaps/TheCapitol/issues/60) | Send `extraction_failed{player_id, reason}` when `start_extraction` refuses (`engine/tick.rs`, `Command::StartExtraction`), mirroring `crafting_failed` | A refused extract is only logged. Clicking a node someone else is harvesting does nothing, and `capitol-bot` guesses a refusal from 3 s of silence (P2.7) | P4.4 (nice to have) | S |
| B10 [#61](https://github.com/chrischaps/TheCapitol/issues/61) | Finish a craft in one transaction: re-check that the inputs still exist and belong to the crafter, consume them, write the output, then send one `inventory_updated` per container | Inputs are checked only at the start, and are consumed by id after the output is committed. Trading or merging them mid-craft looks able to duplicate items or take a partner's stack (to be confirmed by test). Two `inventory_updated`s per craft also make clients see the rope with the fiber still there (P2.7) | P5 (crafting UI) | S–M |

The client can start without B2–B10. P2 and P3 are unaffected. B1 is done, so P4 starts at the spawn on the road in. The cleanest order is B4/B7 before P6.

## 6. Phases and work packages

Sizes: **S** is about 1 focused session, **M** 2–3, **L** 4 or more. Each phase ends with an exit test.

### P0: Foundations

| WP | Work | Size | Depends on |
|---|---|---|---|
| P0.1 [#1](https://github.com/chrischaps/TheCapitol/issues/1) | Bring `C:\Users\chris\dev\Flint` up to `origin/main`. Add it as a squash subtree at `flint-client/engine` (`git subtree add --prefix=flint-client/engine <flint-remote> main --squash`). | S | none |
| P0.2 [#2](https://github.com/chrischaps/TheCapitol/issues/2) | Create the `flint-client/` workspace, add `exclude` to the root `Cargo.toml`, and give it its own `target/`. Copy `.cargo/config.toml` from the engine (8 MB stack). Write `play.bat`. Add a `flint-client/CLAUDE.md` with the subtree-sync commands. | S | P0.1 |
| P0.3 [#3](https://github.com/chrischaps/TheCapitol/issues/3) | Extract `capitol-protocol`: move `ClientMessage`/`ServerMessage` from `ws/connection.rs:21-290`, the REST DTOs, the terrain response types and the rules constants. Point the backend at it, and keep `cargo test` green. | M | none |
| P0.4 [#4](https://github.com/chrischaps/TheCapitol/issues/4) | **Embedding spike** (throwaway): a binary that wraps `PlayerApp` like `flint-android` does. Register one Rhai function by temporarily patching the subtree, upload one annulus mesh, and type into an egui text box. Confirm Y-up handedness and the scale `S`. *Done: the spike lives on branch `spike/p0.4`, never merged, so master's subtree stays identical to Flint. Master keeps `coords.rs`. Findings are under §4.4.* | M | P0.2 |

**Exit:** a window showing a ring mesh, an egui box that accepts typing, and a Rhai script calling a Rust function. The backend builds against `capitol-protocol`.

### P1: Engine hooks (Flint PR)

| WP | Work | Size | Depends on |
|---|---|---|---|
| P1.1 [#5](https://github.com/chrischaps/TheCapitol/issues/5) | H1 `PlayerExtension` + `ExtensionCtx` | M | P0.4 |
| P1.2 [#6](https://github.com/chrischaps/TheCapitol/issues/6) | H2 `on_event` delivery | S | P1.1 |
| P1.3 [#7](https://github.com/chrischaps/TheCapitol/issues/7) | H3 game egui layer and input routing | M | P1.1 |
| P1.4 [#8](https://github.com/chrischaps/TheCapitol/issues/8) | H4 runtime mesh lifecycle with vertex colours and AABBs | M | P1.1 |
| P1.5 [#9](https://github.com/chrischaps/TheCapitol/issues/9) | H5 `screen_to_ground` (Rhai and Rust) | S | none |
| P1.6 [#10](https://github.com/chrischaps/TheCapitol/issues/10) | ADRs and Flint book pages. Full engine suite green (`cargo test` per crate; `--workspace` fails on Windows because of `ndk-sys`). PR on `feature/player-extensions`. Pull the subtree back into `flint-client/engine`. | S | P1.1–P1.5 |

**Exit:** the PR is merged and the spike is rewritten against the real hooks with no subtree patches. The other Flint games keep working, since every hook is opt-in.

### P2: Core networking and state (runs alongside P1)

| WP | Work | Size | Depends on |
|---|---|---|---|
| P2.1 [#11](https://github.com/chrischaps/TheCapitol/issues/11) | IO thread, REST client, WebSocket client, handshake, ping | M | P0.3 |
| P2.2 [#12](https://github.com/chrischaps/TheCapitol/issues/12) | Reconnect with backoff and snapshot resync; connection state events | S | P2.1 |
| P2.3 [#13](https://github.com/chrischaps/TheCapitol/issues/13) | State store: players, nodes, stations, containers, recipes, currency, zone, plots. Applies messages and filters by `player_id`. | M | P2.1 |
| P2.4 [#14](https://github.com/chrischaps/TheCapitol/issues/14) | Polling scheduler and refetch-on-event rules (§4.5) | S | P2.3 |
| P2.5 [#15](https://github.com/chrischaps/TheCapitol/issues/15) | Interpolation buffers and local prediction | S | P2.3 |
| P2.6 [#16](https://github.com/chrischaps/TheCapitol/issues/16) | Trade state machine, including the quirks | S | P2.3 |
| P2.7 [#17](https://github.com/chrischaps/TheCapitol/issues/17) | `capitol-bot`: a headless client that scripts login → move → extract → craft rope. Doubles as a test harness and as "other players" to watch. | S | P2.1–P2.4 |

**Exit:** `cargo test -p capitol-core` is green. An `#[ignore]` integration test against `docker-compose up -d` plus `cargo run` has the bot extract fiber and craft rope.

### P3: The world in 3D

| WP | Work | Size | Depends on |
|---|---|---|---|
| P3.1 [#19](https://github.com/chrischaps/TheCapitol/issues/19) | Terrain fetch with an on-disk cache keyed by `version` (B3), revalidated with `If-None-Match` (304 = use the cache). `--offline <terrain.json>` mode. Capture `fixtures/terrain.json`. | S | P2.1 |
| P3.2 [#20](https://github.com/chrischaps/TheCapitol/issues/20) | **Ground.** Concentric zone discs with vertex-colour tints and soft blends at the boundaries. A wilderness fade toward the 8000 edge, into fog. | S | P1.4 |
| P3.3 [#21](https://github.com/chrischaps/TheCapitol/issues/21) | **Water.** Annulus meshes for the moat (r200), inner canal (r600) and outer canal (r1000), with gaps at bridge angles. The water is the engine's Gerstner ocean under a water table (see below); H6 later adds foam at the banks. Banks as a low lip. | M | P1.4 |
| P3.4 [#22](https://github.com/chrischaps/TheCapitol/issues/22) | **Roads.** Ribbon meshes from polylines with mitered joins (capped at sharp angles), round end caps and widths from the data. Merged into one mesh per road type per sector. Ring roads, arterials, urban grid, suburban collectors and cul-de-sacs, rural trails. | M | P1.4 |
| P3.5 [#23](https://github.com/chrischaps/TheCapitol/issues/23) | **Bridges.** Decks with side rails at each bridge angle and width, on arches with the canal running underneath. | S | P3.3 |
| P3.6 [#24](https://github.com/chrischaps/TheCapitol/issues/24) | **Plots.** Lot meshes from the `terrain_plots` quads (about 2,500 or more) with a slight inset, a curb lip and zone tints. Split into angular sectors × rings (for example 16 × 4) for frustum culling. | M | P1.4 |
| P3.7 [#25](https://github.com/chrischaps/TheCapitol/issues/25) | **The Capitol tower.** A procedural landmark at the origin: castle-like massing in contemporary materials, a lit crown, and a bailey ring at r200 inside the moat. Built with `MeshBuilder`, or authored as glTF. | M | P1.4 |
| P3.8 [#26](https://github.com/chrischaps/TheCapitol/issues/26) | **Atmosphere.** Sky, distance fog, a warm key light, SSAO and bloom. Dusk as the default mood. Tuned with the `flint-render` skill's camera presets. | S | P3.2–P3.7 |
| P3.9 [#27](https://github.com/chrischaps/TheCapitol/issues/27) | **Camera rig** (`camera.rhai`). Follows the player with damping. Pitch is about 50° at street level, eased to 90° at map zoom. Wheel zoom from street level to the whole world; Q/E or right-drag to orbit; a home key to recentre. Map mode flattens lighting into the 2D palette. *(Built: R/F also zoom, H is home, and zoom goes in past the street to 15° for the horizon; see below.)* | M | P1.5 |
| P3.10 [#28](https://github.com/chrischaps/TheCapitol/issues/28) | Terrain hot-swap on `terrain_updated`: rebuild only the affected meshes | S | P3.2–P3.6 |

**How P3.10 built it:**
- **Hearing `terrain_updated` needs a session, and login is P4.1.** So `capitol --live` signs in as a dev account of its own, `flint1@bots.capitol.local` (`Flint1`). It registers the account the first time, through the new `capitol_core::signin::SignIn`, and opens the WebSocket. Without `--live` the client never opens one, and it behaves as before. Flint1 stands in the world while it's connected, and it's a separate account from React's, so the one-session rule isn't hit. P4.1 swaps the dev account for the form's, and moves the `Net`, store and poller up into the extension.
- **The refetch is the poller's.** `terrain.rs`'s `Live` runs a `GameState` and a `Poller` on the terrain source's `Net`. When `poller.observe` returns `Fetch::Terrain`, the `/terrain` answer in that event is the new world. `TerrainFetch::adopt` (new) writes it to the disk cache, so the next start is current. The poller's snapshot and 5 s polls run too. They cost three small GETs every 5 s, and P4 needs them anyway. The HUD's second line shows the session: `live as Flint1`, then `terrain <v> · updated live`.
- **Only the layers whose inputs changed are rebuilt.** `world::Rebuild::between(old, new)` compares the inputs each builder reads (table in `world/mod.rs`). The ground reads the world size and the rings. The water, bridges and tower read the rings. The roads read the roads and the rings. The plots read the plots, the roads and the rings. The extension keeps the last terrain it built from. Measured against the local server: changing `urban_spoke_count` rebuilds roads and plots in 54–59 ms, and changing a canal's width rebuilds everything in 65–79 ms. Both are one hitch of 4–5 frames. P8.5 could move the build off the main thread if that matters.
- **No leaks.** Meshes are replaced by name and empty sectors removed (H4, P3.3). The entities stay, and new ones are spawned only for sectors that weren't drawn before. Over ten regenerations through `PUT /admin/world-params` (the endpoint the F2 tweaker calls), cycling three variants, private memory went 612 MB → 675 MB on the first rebuild and then held between 677 and 681 MB. The first rebuild's step is the allocators reaching their working size, not growth. Capture held 58.6 fps throughout. After the original params were restored, the frame matched the first one, apart from the HUD and the animated marker.
- **Tests.** `world::tests` checks that each input rebuilds only what reads it. `signin::tests` covers login, register on 401, other failures and stray tags. `terrain::tests::a_terrain_fetched_elsewhere_is_adopted` covers the cache.

**How P3.9 built it:**
- **`scripts/camera.rhai`** is the rig, on its own entity (`camera_rig`) beside the spike. It follows a point on the ground: a free point moved with WASD or the arrows, drawn as a gold mark, until P4.2 makes `follow_target`/`follow_home` the avatar. The focus, zoom and yaw each settle exponentially (`1 - e^(-k·dt)`, k = 6, 8 and 10), so it's smooth at any frame rate, and `dt` is capped at 0.1 s so a hitch can't fling it.
- **One zoom dial**, linear in log distance from 11 m to 640 m, so every wheel notch feels the same. The tilt rides it: **15° down at the closest** (2.9 m up, the sky in the top third and, from the outer zones, the tower on the horizon with the sun beside it), **50° at the street** (32 m off, 24.5 m up, the camera P3.6's culling test assumed, where H and the start put you), then easing to **89.9° at the map**. The tilt holds flat at the street, so it's a resting place. Past zoom 0.62 the view drifts to the Capitol, and past 0.8 it turns north up, so the furthest zoom is always the same map: the whole world (r400) from 640 m.
- **Controls** (`config/input.toml`, named by the scene's `input_config`, `game_id = "capitol"`): WASD or the arrows move, faster the further out; the wheel or R/F zoom; Q/E or a right-drag orbit; H comes home (street zoom, north up, the short way round). This is the Cities: Skylines layout. E stops being the spike's plan toggle, which moves to P. The inscription window lists the keys. **Workarounds:**
  - Flint never feeds the wheel to `InputState` ([#67](https://github.com/chrischaps/TheCapitol/issues/67), H11). `CapitolExtension::on_window_event` collects it (a line is one notch, 50 px of a touchpad's scroll is another) and the rig takes it with `cap_take_wheel()`. Captures can't scroll, so they zoom with R/F.
  - Flint can't bind Home, PageUp or +/- ([#68](https://github.com/chrischaps/TheCapitol/issues/68), H12), hence H and R/F.
- **Never into the land or the tower.** The rig keeps out of an 11 m column round the keep (its corner turrets reach 9.6 m) up to 86 m, sliding round it. Over the whole bailey it stays 10 m above the land: the curtain wall's towers stand inside it from r13.4 and the gatehouses reach 8.8 m. It eases down to 1.5 m past the moat (r22.5), though the zoom never takes it below 2.9 m. `camera_rig.rs` loads the script into a bare Rhai engine and checks `rig_pose` across zooms, yaws and a 2.5 m grid of focus points against the tower as built from the fixture, with the near plane (1 m) as margin. It also checks the tilt curve, the street framing, that the horizon is out of frame whenever the camera is above `CLEAR_FROM` (tilt over half the 70° FOV, plus 5°), and that the map is the whole world, north up, above `FLAT_BY`.
- **The map's light** (`atmosphere.rs`). From 280 m to 600 m up, the sun and fill fade to nothing, the sky's light from above rises to `FLAT_SKY` (1.3, white), and SSAO fades. Each surface then shows about its own colour, the §4.7 tokens, as in the React client. No sun means no shadow: the tower's long dusk shadow would otherwise lie across the map. It writes each light's `intensity` and the sky's `ambient_sky` into the world, because the renderer re-reads the sky component every frame (a `set_ambient` would be overwritten). `ambient_ground` is untouched, so the tower's light holds its calibration and the crown is the gold point at the map's centre. Fog is unchanged (it's off from 250 m), and `CLEAR_FROM`/`CLEAR_BY` didn't need retuning: the rig is about 65° down by 100 m.
- **`--view`** still frames captures. `street`, `district` (zoom 0.75, about 230 m up) and `map` are rig framings. The others (`bridge`, `suburb`, `urban`, `capitol`, `crown`, `from-*`) are fixed shots, held until the first camera input. Then the rig takes over at street zoom with the camera where it stood, following a point just ahead of it, and eases across in 1 s. `capitol --frame-step S --screenshot-every S` (new) writes deterministic numbered frames for checking motion.
- **Captures:** the `renders/p3.9-*` tour script (street → map → closest → orbit → move → home) and the from-rural takeover run with no jumps from frame to frame. Map, district and street all run at vsync: 16.75, 16.73 and 16.95 ms/frame.

**How P3.8 built it:**
- **The scene is `scenes/capitol.scene.toml`**, renamed from `spike.scene.toml`, as P0.4 planned. The spike's script and markers still ride in it until P3.9 and P4 replace them.
- **Dusk.** The sky is deep navy overhead, with stars, and a rose band of haze on the horizon. The key light is a low amber sun (`[1.0, 0.60, 0.32]` × 3.2) in the south-west, 8° up. That's 45° off the usual camera, which looks north: the tower's south-west faces catch it, and every wall throws a long shadow north-east. A cool fill (× 0.4) comes from the east. The sun disc is 0.8° (`sun_disc_size` 0.014, against a default of 2°), with a soft glow (0.3). From the north (`--view from-rural`) it sets behind the Capitol. `ambient_ground` is unchanged, so `palette::AMBIENT_GROUND` and the tower's light hold. `diffuse_wrap` stays 0.
- **Fog is the sky's colour on the horizon** (`horizon` mixed 45% toward `haze`, `[0.2225, 0.1123, 0.1380]`), so the land runs out into the sky with no line. It's exponential from 100 m at density 0.0009: about a quarter across the world, and 99% by the ground's 5 km skirt.
- **Limitation, filed as H10 ([#66](https://github.com/chrischaps/TheCapitol/issues/66)).** The composite fog skips depth ≥ 0.9999. At the old far plane (4 km) that was 2.9 km out, and the skirt beyond it drew the dark band under the sky that the P3.7 horizon captures show. **The camera's far plane is now 20 km**, which moves the cut to 6.7 km, past the skirt. The ocean past the skirt fogs itself. Depth precision is set by the near plane, so the map view loses nothing. Shadows use their own 200 m range.
- **`atmosphere.rs`** (in the extension) thins the fog with camera height: the scene's own fog to 100 m, easing to none at 250 m, so the district (260 m) and map (620 m) views see the city clear. It turns `fog_enabled` off, rather than just zeroing the density, because the ocean's self-fog ignores density and would turn the canals rose from the map (H10). It writes only when the scale changes, so a density tuned live in F3 holds while the camera is still. **P3.9 owns zoom and can retune `CLEAR_FROM` and `CLEAR_BY`.** Until H10 lands, keep the horizon out of frame above 100 m: thin fog leaves the dark skirt showing against the sky.
- **Post.** Bloom has threshold 2.5 and intensity 0.12. It picks up the lantern (3.5), the needle's tip (8) and the sun disc, but no sunlit wall (about 1.9) and no window. SSAO has radius 1.5 m, intensity 0.8 and 16 samples. The vignette is 0.25 and exposure is 1. Dither is off: the engine's ordered dither drew a visible checkerboard over the whole frame.
- **The ocean's cel ramp went from 3 steps to 5.** Its shade is N·L plus wave height. Under an 8° sun nearly all the water fell inside one band, and the swells vanished. Five steps put a band edge back in the middle of the low-sun range.
- **Captures** (`renders/p3.8-*.png`, with `before-*` pairs and `p3.8-before-after.png`) cover every view at 57–60 fps, the same as before.

**How P3.7 built it:**
- **`world/tower.rs`** builds the Capitol from the terrain's innermost ring (the moat). "A modern structure arranged like a medieval castle" is read as castle massing in contemporary materials:
  - **Curtain wall.** Pale concrete, standing 10 cm into the moat's inner kerb, so it rises straight from the water's edge. It has a parapet, square merlons and a line of warm light under the parapet. There's a round tower every 45° from the front, each with a gold collar.
  - **Gatehouse.** One over each moat bridge (one in the fixture, at 90°, south). Its passage is the deck's width plus 45 cm either side and 4.3 m high. The ceiling glows down into it, with a gold line of light over the opening.
  - **Bailey.** Paving at `layer::BAILEY`, a gold ring of light set into it at 11.7 m, and a path from each gate to the keep's lit door.
  - **Keep.** It faces the first gate. A chamfered podium, four glass corner turrets (24 m) capped in gold, and a glass tower in three setbacks (36, 50 and 60 m) under concrete slabs edged in gold light. A stone pier at every corner stands 1.2 m above each terrace, like a merlon. About three windows in five are lit, each a slightly different brightness and warmth, picked by a hash so the result is the same every build.
  - **Crown.** A 4 m lantern of gold light, its roof lit too, so the centre of the map view is a point of gold. Round it are gold blades, tall and short in turn, with taller posts at the corners, and a gold needle with a point of light at its tip. The top is about 84 m above the land.
- **Draw cost: 3 draws, 10,364 vertices, 5,366 triangles.** The meshes are `capitol` (stone, roughness 0.8), `capitol_glass` (roughness 0.18) and `capitol_gold` (metallic 0.85, roughness 0.35, with all the light merged in). That's about 2% of the triangles in a from-urban frame. Captures stay at vsync (55–60 fps). A test caps it at 20k triangles.
- **Light without an emissive term (a workaround, H7 [#63](https://github.com/chrischaps/TheCapitol/issues/63)).** Flint's material can't emit. A glowing face is built facing the right way, then shaded with a normal pointing straight down (`MeshBuf::glowing`). Every light above the horizon misses it, so only the hemisphere's ground ambient lights it, the same from every side. `palette::glow(colour, radiance)` divides by `palette::AMBIENT_GROUND`, which a test holds equal to the scene's `ambient_ground`. **P3.8 must keep them in step**, and must leave `diffuse_wrap` at 0 until H7 lands. Radiances run from about 0.3–0.75 for the windows to 3.5 for the lantern and 8 for the needle's tip.
- **Tests:**
  - nothing reaches the moat's water
  - each gate is clear over the deck, from the paving to the passage ceiling
  - every face faces its normal
  - the needle's light is the top, over 80 m up
  - with no moat, the wall closes and all eight towers stand
  - the triangle budget
- **Views.** `--view capitol` looks up at the tower from the quay south of the moat, and `--view crown` is level with the lantern. `--view from-urban|from-suburban|from-rural|from-wilderness` puts the tower on the horizon from each outer zone, each from a different side. The inscription (`--note`) now hangs over the needle at `cap_crown_y()`.

**How P3.6 built it:**
- **`world/plots.rs`** draws every `/terrain` lot as a slab at `layer::PLOT` (12 cm above the land, 2 cm over the arterials), with a curb lip round it and walls down to the ground. Lots by a road show a dark gutter.
- **Lots are clipped clear of the roads and the water.** The data's lots overlap the roads: 160 urban lots reach 22 units into the arterials, and a few suburban ones reach into their streets. The innermost urban row also reaches into the outer canal. Each lot is the convex hull of its quad, clipped by half-planes to the edge of every road ribbon it touches and to the kerb's land face (`water::kerbed`). A lot left with under 25% of itself is dropped (12 of 2,437 in the fixture). Then it's inset, so neighbours stand apart with a seam of ground between them.
- **Styles by zone.** Urban: 1.5 inset, a 1.2-wide stone curb 6 cm high. Suburban: 2.5 inset, a 2-wide pale curb 5 cm high. Rural: 4 inset, a 3.5-wide dark green hedge bank 14 cm high. Each lot is its zone token pulled toward the background, a little brighter than the ground (urban 0.27, suburban 0.15, rural 0.13). Each lot's shade varies ±7%, hashed from its id, so parcels read one by one. A lot too small or too sharp to mitre a curb into is drawn as a plain slab.
- **Sectors.** Each lot goes into the mesh of the sector its centre is in, `plots_r<ring>w<wedge>`. That's 45 meshes and about 134k vertices, and they build in about 50 ms. The test checks that every sector mesh's vertices stay within one lot of its wedge and ring, so the box it's culled by stays tight. A frustum test puts P3.9's street camera (50° down, 25 m up) over urban, suburban and rural players and checks that under half the plot sectors are drawn. In the captures, the urban close-up draws 80 entities with 56 culled.
- **Lookup for P6.4.** `Plots::lots` is a `Vec<Option<Lot>>` in `terrain_plots` order. Each `Lot` records its sector and its vertex and index ranges in that sector's mesh. It's in order rather than keyed by id because **the generator repeats ids**: 4 ids on P-shaped streets cover 11 lots (noted on B7). The extension keeps it as `CapitolExtension::lots`.
- **`world/geom.rs`** holds the server-space vector helpers that `roads.rs` had kept private, so plots can use them too.
- **`--view urban`** is close over the urban lots by the south arterial. With `RUST_LOG=capitol=debug`, the extension logs the renderer's entity draws and culled count every 2 s. The `--input-script` can't press F2, because the script drives game input, not the debug keys.

**How P3.5 built it:**
- **The water now runs under the bridges.** P3.3 left each bridge's gap as a causeway at land height with the kerb carried across it, so from the quay a crossing read as a dam. `water.rs` now builds every ring as one unbroken channel: bed, walls and kerbs all the way round, with no reaches or causeways. Each bridge stands on an arch over it.
- **`world/bridges.rs`** builds every bridge as one mesh, `bridges` (13 in the fixture, so there are no sectors). Each deck is the bridge's nominal width, with straight parallel sides along its axis, so from above it covers exactly what P3.3's gap was. It runs from road end to road end (`water::kerbed`) plus a 2 m ramp onto the road at each end. In profile it's one sine hump from toe to toe: from 2 cm above the arterials at the toes up to `layer::DECK` at the crown, so it clears the kerbs everywhere (tested). Underneath, an elliptical arch springs from the channel walls at `layer::ARCH_SPRING`, just under the waves, and rises to 18 cm under the deck at the crown. That leaves about 1.1 m of headroom over the sea, and the ocean shows through from the quay.
- **Colours.** The deck is the 2D bridge fill `#8b6914`. The sides, posts and rails are its edge `#5d4612`, and the vault is the edge pulled 35% toward the background. Rails run along both sides at 0.45 m and 0.9 m, on posts about 1.2 m apart, between newel posts capped in Capitol gold (`palette::GOLD`, new).
- **Walking on it.** The deck's crown is 0.7 m above the land. P4.2 should stand avatars on `Span::top` (the profile) when they're on a bridge, or their feet sink into the hump. I left a note on #32.
- **Tests** check, for each bridge: the deck is exactly its width from toe to toe; decks stand only at bridges; it meets the road ends and clears the kerbs; the arch springs under the sea and is never thinner than at its crown; the server accepts every point the deck covers over the water; and every face faces its normal.

**How P3.4 built it:**
- **`world/roads.rs`** turns each `/terrain` polyline into a flat ribbon of its own width, in the §4.7 road colours, at its type's layer. Joins are mitred. A join is rounded instead when the mitre would reach past 2 half-widths (a turn over 120°) or its inside corner would run back past the middle of a short segment. Overlap on the inside of a rounded join is invisible in one flat colour. Points closer than 0.5 units are merged (the suburban data repeats some). Closed polylines (the trade and guild rings) are loops with no ends.
- **Ends.** A road that stops on its own ends in a round cap, as in the 2D clients. A road whose end lies on another road ends flat, so the 60-wide arterials meet the 30-wide ring roads in a clean T rather than bulging past them. **Every road stops flat at the kerb's land face** (`water::kerbed`: each channel plus its 1 m kerb), bridges included. The data's arterials are 60 units wide and the bridges 25, so P3.5's deck spans from road end to road end, `KERB` past each channel edge. Nothing reaches the moat from the inside; that's the tower's (P3.7).
- **Sectors.** `world/sector.rs` cuts the world into 16 wedges × 4 rings. The wedges are turned half a wedge, so the arterials and cardinal bridges run down a wedge's middle. The rings break at r1000, r1600 and r2400, the urban, suburban and rural edges. Each type's roads in each sector are one mesh, `roads_<type>_r<ring>w<wedge>`, so the fixture makes 86 meshes and about 26k vertices. Segments are split where they cross a sector edge, and the corners at every join are computed once and shared, so meshes meet without cracks. The extension spawns one entity per mesh the first time it has geometry (`ExtensionCtx::attach_mesh`). A rebuild replaces meshes by name and empties the ones a new terrain leaves bare. P3.6's plots can use the same sectors.
- **Layers.** The road ranks were 5 mm apart, below what the depth buffer resolves from the map view, and are now 2 cm (see §4.6). Two roads of one type cross at the same height in the same colour, so nothing shows. The spike's compass markers moved up a layer, onto the trade ring road.
- **`--view suburb`** frames the southern suburbs low down, for checking curves, cul-de-sacs and caps.
- Capture runs at vsync (59–60 fps) in every view.

**How P3.3 built it:**
- **The water is Flint's ocean, the one Float uses, under a water table.** A first version (7c12133) drew its own vertex-coloured surface, and it read as flat blue with a low-res repeating pattern. The engine's ocean is an endless, camera-following Gerstner sheet fixed at y = 0. So the land now stands 0.6 m above it, and `ground::build` leaves a hole over each ring's band of radii (`water::holes`). The ocean draws everywhere but only shows in the channels. It's configured in the scene at canal scale: centimetre waves 0.6–6 m long, dusk colours, cel bands, Fresnel sky reflection and refraction. Tune it live with F3. The ground's skirt now runs to r50000, past the far plane, so the sea never shows at the horizon. That also means the horizon is now dark ground, which P3.8's fog meets. No engine change was needed. H6 (water on any mesh) is still how the canals get foam at their banks.
- **`world/water.rs`** builds everything solid around the water as one opaque mesh, `canals`. *(P3.5 replaced the reaches and causeways: the water now runs right round each ring, under the bridges.)* Each ring is cut into *reaches*: the open water between one bridge and the next (or the whole ring, if it has none). Each reach is a channel with stone walls from a dark bed at -1.6 m, through the water, up to a 1 m kerb 0.3 m above the land, mitred at the corners. The moat's kerb is gold-dim; the canals' is a warm, darkened grey. Each bridge's gap is a causeway at land height, coloured like the ground around it.
- **Gaps.** Each gap has straight, parallel sides exactly the bridge's nominal width apart (the reach ends where the line half a width off the bridge's axis meets each radius, at `asin(h / r)`), so P3.5's deck fits it exactly. The server's crossing test is about twice as generous (`Bridge::angular_half_width` allows the full width either side of the axis), so everywhere drawn dry is walkable. The tests:
  - check each gap's edges from both sides at five radii across each channel, as water on one side and causeway on the other;
  - sweep each ring for gaps that aren't bridges;
  - check that the ground leaves the bands open and something covers every point in them;
  - check that the server accepts every drawn gap edge.
- **`MeshBuf`** gained `vertex`, `quad` (winds itself from the normal, since back faces are culled), `append` and `upload_material`. Uploading an empty mesh now removes the old one of that name, so a rebuild that finds nothing (P3.10) clears the screen rather than leaving stale geometry.
- **`--view bridge`** stands on the quay by the inner canal's south crossing, for P3.4 and P3.5 to check against.
- **Roads meet water.** The data's arterials are 60 units wide but cross at 25-unit bridges, and the urban spokes start at r1025, the outer canal's edge. P3.4 should stop roads at the kerb, or narrow them onto the bridge, rather than run them over the channel.
- **Engine nit:** `engine/schemas/components/ocean.toml` types `seed` and `num_waves` as `"integer"`, which the schema doesn't know (it wants `"int"`), so the loader warns. The ocean still reads the values. `terrain.toml` has the same typo. Filed as H8, [#64](https://github.com/chrischaps/TheCapitol/issues/64).

**How P3.2 built it:**
- **`world/`** holds the meshes built from the terrain: `mesh.rs` (`MeshBuf`, moved out of `spike_world.rs`), `palette.rs` (the §4.7 tokens, sRGB hex converted to linear) and `ground.rs`. The ground is rebuilt from `terrain.world_size` each time `take_changed` hands over a terrain. P3.3–P3.7 add their builders beside it.
- **The ground** is one disc with per-vertex colour: 480 segments, with rings every 20 units inside r1000 and every 40 units out to the edge. There are extra rings across each blend, then wide steps out to a skirt at r12000, well past the world's corners (about 5657), so the rim is only seen at the horizon. That's about 90k vertices, and capture still runs at vsync (59.6 fps). Each zone is its token pulled toward the background (`strength`: Capitol 0.40, Trade 0.17, Guild 0.16, Urban 0.20, Suburban 0.10, Rural 0.085). The brighter greens get less, so the rings sit at similar darkness and differ in hue. Boundaries blend with a smoothstep over ±40 units, and ±200 for rural into wilderness. From r3400 the colour fades to the background, reaching it at the world edge (r4000). A ±10% value-noise mottling (30 m wavelength) breaks up the flat tints.
- **No fog yet.** The ground fades to the background colour, so P3.8's fog only has to meet it at the horizon. At map zoom fog works on camera distance, so it would wash the whole city out. That's for P3.8 and P3.9 to settle together. *(P3.8 settled it: `atmosphere.rs` clears the fog by 250 m up.)*
- **`--view street|district|map`** (read by scripts as `cap_view()`) frames the capture until P3.9's rig replaces it. Map is 620 m up, straight down: the player's vertical FOV is about 70°, not the renderer's default 45°.

**How P3.1 built it:**
- **Revalidation in the REST layer.** `RestRequest::Terrain { cached_version }` sends `If-None-Match: W/"<version>"`, and a 304 decodes to `RestResponse::TerrainUnchanged`. The poller's `terrain_updated` refetch sends no version, because that event means the terrain changed. reqwest's `gzip` feature offers gzip and decodes it; the mock server in `tests/net.rs` refuses a client that doesn't offer it.
- **The cache** (`capitol_core::terrain`). `TerrainCache` is one file per server, `cache/terrain-<host_port>.json` under `flint-client/` (gitignored, beside the engine's `save/`). The file is plain `/terrain` JSON, so a cache file works with `--offline` too. Writes go through a temp file and a rename. An unreadable file is logged and ignored. `TerrainFetch` is pure apart from the file, like the poller: `start` returns the cached terrain to draw at once, `tick(now)` gives the request, and `observe` returns a terrain only when the server sent a newer one. A fetch with no answer is retried every 5 s.
- **Client.** `capitol --server <url>` (or `CAPITOL_SERVER`, default `http://localhost:3000`) fetches through the cache. `--offline <terrain.json>` reads the file and never starts the net thread. The terrain source owns a `Net` for now, and P4.1 moves it up into the extension. The inscription window shows where the terrain came from (`offline · terrain 70c59888`, `cached, checking…`, `cache current`, `fetched`). Until P3.2–P3.7 land, the terrain only drives the spike's water (real widths: the canals are 50 units, not the spike's 15). An empty runtime mesh is skipped, because wgpu can't create an empty buffer.
- **Captures.** `--screenshot-at` alone writes `renders/capitol.png`. `--screenshot`, `--input-script` and `--exit-after` pass straight to `CaptureConfig`. The default log filter is `warn,capitol=info,capitol_core=info`, so the cache lines show.
- **Fixture.** `fixtures/terrain.json` is the local server's `/terrain` at version `70c59888ab1b58c8` (192 roads, 2,437 plots). A test in `capitol-core` checks that it parses and carries a version.

**Exit:** offline-mode screenshots at street, district and map zoom. At least 60 fps at map zoom on the dev machine (`--exit-after` reports ms/frame). Meshes rebuild without leaks when the terrain changes.

### P4: The living world (vertical slice)

| WP | Work | Size | Depends on |
|---|---|---|---|
| P4.1 [#31](https://github.com/chrischaps/TheCapitol/issues/31) | **Login and register** in egui over the live world. `/terrain` needs no auth, so the camera slowly orbits the tower behind the form. Remembered email, stored token, a server-URL setting and error states. | M | P1.3, P3 |
| P4.2 [#32](https://github.com/chrischaps/TheCapitol/issues/32) | **Players.** A simple stylised avatar with a gold self-marker and name labels via `world_to_screen`. Interpolated movement and facing. `player_joined`/`player_left` fade in and out. | M | P2.5, P1.4 |
| P4.3 [#33](https://github.com/chrischaps/TheCapitol/issues/33) | **Click to move.** `screen_to_ground` → `cap_move`. A destination marker and path line. `movement_blocked` produces a splash and shake at the water's edge with a "Find a bridge" hint. Clicks clamped to 0–8000. | S | P1.5 |
| P4.4 [#34](https://github.com/chrischaps/TheCapitol/issues/34) | **Resource nodes.** Grass tufts with a 3D available/depleted look. Extract within 50, or move then extract automatically on arrival (React's rule). A progress ring, a gold targeting line and an "+N fiber (B)" toast on completion. Regrowth animation on `node_regenerated`. | M | P4.2, P4.3 |
| P4.5 [#35](https://github.com/chrischaps/TheCapitol/issues/35) | **Zone and connection.** A zone banner on `zone_changed`, a connection overlay while reconnecting, and a header strip with zone and strand balance. | S | P2.2 |

**Exit:** the Flint client and the React client, logged in as two accounts, see each other move. In Flint the other player moves smoothly. Extraction works end to end, and pulling the network cable reconnects without restarting.

### P5: Inventory and crafting

| WP | Work | Size | Depends on |
|---|---|---|---|
| P5.1 [#36](https://github.com/chrischaps/TheCapitol/issues/36) | **Theme.** egui `Visuals` and `Style` from the tokens, the project font, panel frames and a gold focus ring. Reviewed with screenshots against the React panels. | S | P1.3 |
| P5.2 [#37](https://github.com/chrischaps/TheCapitol/issues/37) | **Item icons.** Fiber, rope and the three kits, generated through `flint-asset-gen` or procedural shapes, at a consistent size and style. | S | none |
| P5.3 [#38](https://github.com/chrischaps/TheCapitol/issues/38) | **Slot grid widget.** Icon, quantity, quality pip (§4.7) and a tooltip with name, quality and grade. Drag and drop between slots sends `move_item`. Handles the moved, merged and swapped results. | M | P5.1, P2.3 |
| P5.4 [#39](https://github.com/chrischaps/TheCapitol/issues/39) | **Inventory panel.** Toggled with `I`. A 24-slot, 6-column grid. | S | P5.3 |
| P5.5 [#40](https://github.com/chrischaps/TheCapitol/issues/40) | **Crafting panel.** Recipe list, the `crafting_input` (4 slots) and `crafting_output` (1 slot) containers, a quality preview, a progress bar, cancel, and a purple crafting ring on the avatar. Warns when the output slot is full, because the server destroys output that has nowhere to go (`engine/tick.rs:727`). | M | P5.3 |

**Exit:** gather fiber, craft rope and a workbench kit, and drag items around. Every action matches what the React client shows for the same account.

### P6: Stations and plots

| WP | Work | Size | Depends on |
|---|---|---|---|
| P6.1 [#43](https://github.com/chrischaps/TheCapitol/issues/43) | **Station models.** Workbench, forge (ember glow plus fire particles) and storage chest. An orange in-range ring. | M | P1.4 |
| P6.2 [#44](https://github.com/chrischaps/TheCapitol/issues/44) | **Placement mode.** Right-click a `*_kit` to show a ghost at the cursor, green or red against three checks: within 75 of the player, at least 50 from other stations, and inside an owned plot. Click to send `place_station`, right-click to cancel. | M | P6.1, P1.5 |
| P6.3 [#45](https://github.com/chrischaps/TheCapitol/issues/45) | **Station panel.** Opened by clicking a station within 75, or by walking there first. Shows the station container and recipes filtered by `required_station_type`, with craft-at-station and remove. | M | P5.3 |
| P6.4 [#46](https://github.com/chrischaps/TheCapitol/issues/46) | **Plots.** Alt-click picking, a plot panel (zone, size, type, stations used and capacity, assessed value, owner), claiming with the deed cost, and gold edges and ground glow on owned plots. *Depends on the B7 decision.* Until then, claimable DB plots appear as an overlay. | M | B7 |

**Exit:** claim a plot, place a workbench on it, open it, craft at it, then remove it. Item moves into a storage chest work once B4 lands.

### P7: Economy

| WP | Work | Size | Depends on |
|---|---|---|---|
| P7.1 [#49](https://github.com/chrischaps/TheCapitol/issues/49) | Strand balance in the header, with a tick animation when it changes | S | P2.4 |
| P7.2 [#50](https://github.com/chrischaps/TheCapitol/issues/50) | **Exchange panel.** Deposit (select fiber items) and withdraw (quantity), with the 0.5% fee preview and resulting balance. REST-backed. | S | P5.3 |
| P7.3 [#51](https://github.com/chrischaps/TheCapitol/issues/51) | **Trading.** Shift-click a player within 75 (or walk to them) to send a request. An incoming-request notification. A two-sided trade window with items and strands, accept states that reset when either offer changes, and an executed or cancelled result with refetch. Timeout hints for silent failures. | M | P2.6, P5.3 |

**Exit:** two accounts, one in each client, complete a trade of rope for strands. Both inventories and balances are correct afterwards.

### P8: Polish and beauty

| WP | Work | Size |
|---|---|---|
| P8.1 [#52](https://github.com/chrischaps/TheCapitol/issues/52) | **Audio.** Ambience by zone (city hum near the core, birds in the suburbs, wind in the wilderness), water lapping near the rings, UI clicks, extraction rustle, a craft-complete chime. | M |
| P8.2 [#53](https://github.com/chrischaps/TheCapitol/issues/53) | **Juice.** Toasts, pickup arcs from node to inventory, a quality-grade sparkle for A items, camera ease-ins on zone change. | S |
| P8.3 [#54](https://github.com/chrischaps/TheCapitol/issues/54) | **Onboarding.** Hints for the first walk, extraction and craft. A keyboard-shortcut overlay (`?`). | S |
| P8.4 [#55](https://github.com/chrischaps/TheCapitol/issues/55) | **Settings.** Server URL, graphics quality, audio volumes, camera sensitivity, all persisted. | S |
| P8.5 [#56](https://github.com/chrischaps/TheCapitol/issues/56) | **Performance.** Sector culling checks, label decluttering at distance, H6 instancing for repeated props. | M |
| P8.6 [#57](https://github.com/chrischaps/TheCapitol/issues/57) | **Optional beauty.** H6 water shading on the canals; compute grass fields in rural and wilderness areas; procedural houses that grow on claimed plots so the city fills in as players build; a day/night cycle tied to server time, with owned plots glowing gold at night. | L |
| P8.7 [#58](https://github.com/chrischaps/TheCapitol/issues/58) | **Packaging.** A release build script, a zip with the engine binary and data, a README section, a chaps.dev showcase capture. | S |

### Later (outside this plan, but planned for)
- **Admin tools.** F1 debug camera, F2 world tweaker and F3 perf panel slot in as egui panels gated on `is_admin`. Flint's own debug panels sit alongside them.
- **M6 markets.** A market browser and order book as egui panels, plus a price chart (`egui_plot`). Real-time updates arrive through the same `on_event` path.
- **M7 leftovers.** The map view (P3.9's map mode is most of it), an ownership overlay and a permissions editor.
- **M8 automation and M9 social.** More panels on the same patterns. Automated stations get animated states in the world: a forge glows while it runs and dims when it's paused.

## 7. Dependency graph and estimate

```
P0.1 ─ P0.2 ─ P0.4 ─┬─ P1 (hooks, Flint PR) ─┬─ P3 (world) ─┬─ P4 (vertical slice) ─ P5 ─ P6 ─ P7 ─ P8
P0.3 ───────────────┴─ P2 (net/state) ───────┴──────────────┘                         ▲     ▲
                                                                       B1 ────────────┘     │
                                                                       B4, B7 ─────────────┘
```

| Phase | Sessions (approx.) |
|---|---|
| P0 | 4–6 |
| P1 | 5–7 |
| P2 | 5–7 |
| P3 | 9–12 |
| P4 | 5–7 |
| P5 | 5–6 |
| P6 | 5–7 |
| P7 | 3–4 |
| P8 | 6–10 |
| **Total** | **about 47–66 focused sessions** for parity with polish, **about 41–56** for parity without P8, and **about 28–39** to the end of the P4 vertical slice |

P2 can run alongside P1 and P3 in a separate worktree, since it has no engine dependency. P4 is the natural place to pause, review and adjust the rest of this plan.

## 8. Parity checklist (React → Flint)

| React feature | React file | Flint WP |
|---|---|---|
| Login / register, stored auth | `pages/Login.tsx`, `Register.tsx`, `AuthContext.tsx` | P4.1 |
| Header: title, zone, strands, email, logout | `pages/Game.tsx` | P4.5, P7.1 |
| Zone indicator on `zone_changed` | `ZoneIndicator.tsx` | P4.5 |
| Zone rings | `GameCanvas.tsx` | P3.2 |
| Terrain plots | `GameCanvas.tsx` | P3.6 |
| Roads by type | `GameCanvas.tsx` | P3.4 |
| Water rings with bridge gaps | `GameCanvas.tsx` | P3.3 |
| Bridges | `GameCanvas.tsx` | P3.5 |
| World boundary | `GameCanvas.tsx` | P3.2 (fade) |
| Resource nodes, available/depleted/targeted | `GameCanvas.tsx` | P4.4 |
| Stations and in-range ring | `GameCanvas.tsx` | P6.1 |
| Placement preview, valid/invalid | `GameCanvas.tsx` | P6.2 |
| Targeting line to pending extraction | `GameCanvas.tsx` | P4.4 |
| Players: self gold, others green, names | `GameCanvas.tsx` | P4.2 |
| Extraction ring, crafting ring | `GameCanvas.tsx` | P4.4, P5.5 |
| Notifications, "Connecting…" overlay | `GameCanvas.tsx` | P4.5, P8.2 |
| Click priority 1: placement click / right-click cancel | `GameCanvas.tsx` | P6.2 |
| Click priority 2: shift-click player to trade or walk | `GameCanvas.tsx` | P7.3 |
| Click priority 3: click station to open or walk | `GameCanvas.tsx` | P6.3 |
| Click priority 4: alt-click plot to select | `GameCanvas.tsx` | P6.4 |
| Click priority 5: click node to extract or move then extract | `GameCanvas.tsx` | P4.4 |
| Click priority 6: click ground to move | `GameCanvas.tsx` | P4.3 |
| Inventory grid, drag and drop, quality pips | `InventoryPanel.tsx`, `InventoryGrid.tsx`, `InventorySlot.tsx`, `DragDropContext.tsx` | P5.3, P5.4 |
| Right-click kit to place | `InventorySlot.tsx` | P6.2 |
| Crafting panel | `CraftingPanel.tsx` | P5.5 |
| Station panel, craft at station | `StationPanel.tsx` | P6.3 |
| Exchange deposit / withdraw | `ExchangePanel.tsx` | P7.2 |
| Trade request notification | `TradeRequestNotification.tsx` | P7.3 |
| Trade window | `TradeWindow.tsx` | P7.3 |
| Plot panel and claim | `PlotPanel.tsx` | P6.4 |
| WS handling: `positions`, `player_joined/left` | `GameContext.tsx` | P2.3, P4.2 |
| WS handling: `extraction_*`, `crafting_*`, `node_*` | `GameContext.tsx` | P2.3, P4.4, P5.5 |
| WS handling: `inventory_updated` → refetch | `GameContext.tsx` | P2.4 |
| WS handling: `station_*` | `GameContext.tsx` | P2.3, P6 |
| WS handling: `zone_changed` → refetch plots | `GameContext.tsx` | P2.4, P4.5 |
| WS handling: `trade_*` | `GameContext.tsx` | P2.6, P7.3 |
| WS handling: `terrain_updated` → refetch | `GameContext.tsx` | P3.10 |
| 5 s polling of nodes, stations, plots | `GameContext.tsx` | P2.4 |
| *New:* `movement_blocked` feedback (React ignores it) | — | P4.3 |
| *New:* interpolation, reconnect | — | P2.2, P2.5 |
| *Out of scope:* F1 debug camera, F2 world tweaker, F3 perf panel | `GameCanvas.tsx`, `WorldTweakPanel.tsx`, `AdminPerfPanel.tsx` | Later |

## 9. Risks and open questions

| Risk / question | Mitigation |
|---|---|
| **Themed egui may still look like a tool.** | Do the theme pass early (P5.1) and compare screenshots side by side with the React panels. Keep panel code behind a small widget layer so moving to `.ui.toml` later is a contained job (§4.3). |
| **Engine hooks are rejected or drift from upstream.** | Every hook is opt-in and general. Use a PR on a feature branch rather than pushing to `main`. Run the full engine suite. Pull the subtree with `--squash` and diff against the Flint remote (vault: *Flint Engine Sync Across All Games*). |
| **Build times.** The engine plus wgpu in a nested workspace makes for long cold builds. | A separate `target/`, plus `capitol-core` with no engine dependency, so most logic tests in seconds. |
| **Terrain payload size** (about 1 MB) | Resolved by B3: gzip (213 KB), a `version`, and ETag/304 revalidation for the disk cache. |
| **Two plot systems** (B7) | Decide before P6. Until then, show claimable DB plots as an overlay. |
| **Rhai performance with many entities** | Keep per-entity and per-frame work in Rust (`entities.rs`). Rhai handles intents and HUD only. |
| **One session per account** | Testing always uses two accounts. Document it in `flint-client/CLAUDE.md`. |
| **World scale** (B1) | Resolved by B1: the spawn, nodes, zones and move bounds are on the 8000 world. DB plots are only scaled, and B7 replaces them. |
| **Windows only** | Accepted for now. Android is possible later through `flint-android`, but touch input and UI scale would need their own plan. |
| **The quality cascade isn't real yet** (`engine/crafting.rs` averages inputs and ignores weights) | Not a client problem, but the quality preview in P5.5 should be computed by the server or labelled as an estimate so it doesn't lock in the current maths. |

## 10. Verification strategy

- **Unit tests (`cargo test -p capitol-core`, `-p capitol-protocol`):**
  - Serde round-trips against JSON fixtures captured from a running server, one per message type.
  - Interpolation and prediction maths.
  - The state reducer for each message.
  - Trade state machine transitions, including the quirks.
  - Quality grade boundaries.
- **Mesh builder tests (`capitol-client`):**
  - No NaNs.
  - Consistent winding (normals face +Y).
  - Expected vertex and index counts for known inputs.
  - Water gaps line up with bridge angles.
  - Sector bounds contain their vertices.
  - `coords.rs` round-trip and orientation (East = +X, South = +Z).
- **Integration test:** `capitol-bot` against a local stack (`docker-compose up -d`, `cargo run` in `backend/`), marked `#[ignore]`. It logs in, moves, extracts and crafts rope. Then a proxy in the test cuts every connection, and the bot reconnects, resyncs and walks again (§4.5, "How P2.7 built the bot").
- **Visual checks:** `flint render` doesn't run scripts or game code, so it can't build the runtime world. Instead use offline mode with `flint-player`-style capture: `capitol --offline fixtures/terrain.json --screenshot-at 3 --input-script <camera path> --exit-after 4`, run from `flint-client/`. Output goes to `flint-client/renders/` (`renders/capitol.png` unless `--screenshot` names a file). Each world-facing WP ends with a before/after pair and a note on what to look at.
- **Two-client manual test:** the Flint client and the React client on two accounts, run at the end of P4, P6 and P7 against the exit criteria above.
- **Performance budget:** `--exit-after` ms/frame at street, district and map zoom. Target 16.6 ms or better on the dev machine with all plots visible.
