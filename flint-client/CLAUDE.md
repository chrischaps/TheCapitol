# CLAUDE.md: The Capitol, Flint client

The Capitol's 3D client, built on the Flint engine (Rust/wgpu). It's a window onto the Capitol server: Rust handles the wire and the world, and Rhai scripts handle the feel. The plan is `../the-capitol-flint-client.md`, and the work queue is the GitHub issues labelled `flint-client` (use the `next-issue` skill).

## Layout

```
flint-client/
  Cargo.toml          # its own workspace: members crates/*, excludes engine/
  Cargo.lock          # committed; seeded from engine/Cargo.lock
  .cargo/config.toml  # 8 MB main-thread stack on Windows (copied from the engine)
  engine/             # Flint, git subtree (squash). Don't edit casually; see below.
  crates/
    capitol-core/     # net + state, no engine dependency, fast tests
    capitol-client/   # the game binary (`capitol`)
  scenes/ schemas/ scripts/ fonts/ audio/ models/ fixtures/
  particles/          # *.particles.toml effects, loaded by name (the forge's fire, P6.1)
  assets/icons/       # item icons: <item_type>.svg, fallback.svg; style in its README (P5.2)
  play.bat            # build release and run
  renders/            # screenshots and captures (gitignored)
  target/             # this workspace's build output (gitignored)
```

The root `Cargo.toml` (the backend workspace) has `exclude = ["flint-client"]`. Run cargo from `flint-client/`, not the repo root.

**Engine crates are path dependencies.** They're declared once in `[workspace.dependencies]` in `Cargo.toml` (`flint-player = { path = "engine/crates/flint-player" }`). Member crates use `{ workspace = true }`. The engine's own workspace is excluded rather than nested. Its crates still pick up `version.workspace = true` and similar settings from `engine/Cargo.toml`.

**The wire types come from `../protocol`** (crate `capitol-protocol`, a member of the root workspace). It holds the WebSocket enums, REST DTOs, `/terrain` types and shared rules (ranges, zones, quality grades, the water-crossing test). The backend compiles against the same crate, so use its types rather than redefining shapes here. The client leaves its `backend` feature off. Change the protocol from the repo root, and run `cargo test -p capitol-protocol` there.

**Match the engine's versions.** A third-party crate that crosses the engine boundary (egui, winit, wgpu, rhai, glam) must use the same version as `engine/Cargo.toml`. Otherwise two copies get linked and the types don't line up.

## Build and run

```bash
cd flint-client
cargo build                         # debug, about 45 s from cold
cargo test -p capitol-core          # fast; no engine
play.bat [args]                     # release build, then target\release\capitol.exe [args]
```

From bash, run `play.bat` through PowerShell with an absolute path:
`powershell.exe -NoProfile -Command "cmd /c 'C:\Users\chris\dev\TheCapitol\flint-client\play.bat'"`.

If another build or a running client could lock the exe, use your own `CARGO_TARGET_DIR`.

## Terrain, offline mode and captures

`capitol` fetches `/terrain` from the server and caches it in `cache/terrain-<host_port>.json` (gitignored). A cached terrain is drawn at once and revalidated by version: the server answers 304 while it's current. The server is `--server` (or `CAPITOL_SERVER`) for one run, otherwise the one last chosen on the login form, otherwise `http://localhost:3000`.

**Login (P4.1).** Online, the client opens on the login form (`ui/login.rs`), with the camera circling the tower behind it (`camera.rhai`, `cap_at_login`). A stored token goes straight in. The way in is `capitol_core::auth::Auth`, which is pure and unit-tested; `session.rs` owns the `Net`, the store and the poller, and runs it. The server, the last email and the token live in `save/session.toml` (gitignored; never commit it), not in the scripts' `save/persist.toml`. Delete the file, or Sign out (top right), to see the form again. **Players (P4.2)** are `entities.rs`: an avatar entity per `Motion` pose, with fades on join and leave. Their names are `scripts/hud.rhai`, and so is the zone banner (P4.5). The header strip and the reconnecting overlay are `ui/hud.rs`. **Clicks (P4.3)** are `scripts/input.rhai`: a left click on the ground goes through the click priorities (React's order, 1–5 still empty) to `cap_move`. A canal stops the walk with `movement_blocked`, which the extension fires at the scripts. **Resource nodes (P4.4)** are `nodes.rs` (the tufts, from the store) and priority 5 in `input.rhai` (`cap_node_at`, `cap_extract`, the rings and toasts). The click rule, extract or walk then extract, is `capitol_core::harvest`. To extract without clicking, `POST /world/extract` with the account's token while the client is connected. The client sees it like its own. Input scripts can't click ([#73](https://github.com/chrischaps/TheCapitol/issues/73)). P4.3 drove clicks by posting `WM_MOUSEMOVE`, then `WM_LBUTTONDOWN` and `WM_LBUTTONUP`, to the client's window by PID (`PostMessage` from PowerShell). Post the move first. The coordinates are physical pixels, so on a 150% display divide the render's x and y by 1.5. To reach a canal fast, walk there first with `POST /player/move` while the client is connected. To see a crowd, run `capitol-bot --accounts 10 --wander --near <your name>`. A stored token in `save/session.toml` is the easy way to put a capture in the world without typing into the form. Once signed in, the session hears `terrain_updated` (an admin regenerating the world from the React client's F2 tweaker) and rebuilds the world. Only the layers whose inputs changed are rebuilt (`world::Rebuild`). The form can't be typed into from `--input-script` (it writes actions, not egui text); P4.1 drove it with keys posted to the client's window by PID, see its closing comment on #31. `--offline <file>` builds the world from a `/terrain` JSON with no server at all, so there's no login either: the rig, as before. `fixtures/terrain.json` is a captured one; refresh it with `curl -s -o fixtures/terrain.json http://localhost:3000/terrain`.

Every visual change is checked with a capture, run from `flint-client/`:

```bash
target/release/capitol.exe --offline fixtures/terrain.json --screenshot-at 3 --exit-after 4
```

That writes `renders/capitol.png`. The camera is the rig in `scripts/camera.rhai` (P3.9): signed in, it follows your avatar and WASD or the arrows pan from it (P4.2). Offline, WASD moves a free point, the wheel or R/F zoom from 15° over the street to the map, Q/E or a right-drag orbit, H comes home. Keys are in `config/input.toml`. `--view` picks where it starts: `street`, `district` and `map` are rig zooms. `bridge|suburb|urban|capitol|crown|from-urban|from-suburban|from-rural|from-wilderness` are fixed shots (bridge stands on a canal quay; suburb is low over curving streets; urban is close over the urban lots; capitol looks up at the tower from beyond the moat; crown is level with its lantern; each from-* puts the tower on the horizon from that zone). A shot holds until the first camera input, then eases into the rig. For motion, add `--frame-step 0.016666667 --screenshot-every 0.25` with an `--input-script` (R/F to zoom: scripts can't scroll yet, [#67](https://github.com/chrischaps/TheCapitol/issues/67)) for numbered, deterministic frames. `cargo test -p capitol-client camera_rig` checks the rig's geometry: no clipping, the tilt curve, the horizon, the map. `RUST_LOG=warn,capitol=debug` logs the renderer's draws and culled count every 2 s; F2's overlay can't be opened from `--input-script` (H9, [#65](https://github.com/chrischaps/TheCapitol/issues/65), will fix that). `--screenshot <path>` names the file, `--input-script <file>` plays timed keys (flint-player's format: `1.0 tap KeyE`), and `--exit-after` prints the average ms/frame. Look at the PNG before calling a visual change done. The scene is `scenes/capitol.scene.toml` (dusk: sky, sun, fog and post). Its fog thins as the camera climbs and is off above 250 m, and from 280 m to 600 m the light flattens into the map's palette (`atmosphere.rs`), so a high view shows neither.

## The UI theme

All egui panels take their look from `crates/capitol-client/src/ui/theme.rs` (P5.1, plan §4.3). It holds the colour tokens, egui's `Style` and `Visuals`, the fonts, and the parts panels are made of (`panel`, `well`, `tray`, `header`, `pill`, `primary_button`, `accent_button`, `grade_badge`, `quality_badge`, ...). **Panels use no colours or sizes of their own.** If a panel needs one that's missing, add it to `theme.rs`. `theme::install` runs at the top of `CapitolExtension::ui` and returns false on the frame its fonts go in. Draw nothing on that frame (#72).

The fonts are in `fonts/`: Source Sans 3 (text) and Cinzel (titles), both OFL, with licences beside them. `fonts.toml` gives the script HUD the names `body`, `strong` and `display` (`draw_text_opts(..., #{ font: "display" })`). `theme.rs` embeds the same files for egui.

Review a theme change side by side with the React client, from `flint-client/` (the steps are in the file's header comment too):

```bash
target/release/capitol.exe --offline fixtures/terrain.json --ui-sample --width 1920 --height 1080 --screenshot renders/p5.1-sample-flint.png --screenshot-at 3 --exit-after 4
target/release/capitol.exe --offline fixtures/terrain.json --width 1920 --height 1080 --screenshot renders/p5.1-world.png --screenshot-at 3 --exit-after 4
"/c/Program Files/Google/Chrome/Application/chrome.exe" --headless --allow-file-access-from-files --window-size=1280,720 --force-device-scale-factor=1.5 --screenshot="$(cygpath -w $PWD/renders/p5.1-sample-react.png)" "file:///$(cygpath -m $PWD/tools/theme-sample.html)"
```

`tools/theme-sample.html` is the sample's two panels in the React markup and CSS, with the panel-less world capture as the backdrop.

**Slots (P5.3).** Every container is drawn with `ui::slots::SlotGrid::new(&container).show(ui, &mut icons)`, which returns `Option<Move>`. Send the move with `session.move_item(m)`, and show `session.move_refused(now)` when it's `Some`. Don't draw slots by hand. A drag between any two grids on screen just works (egui DnD payload). **The inventory panel (P5.4)** is `ui/inventory.rs`, bottom right, toggled with `I` (the `inventory` action, so `--input-script` can press it) or the header's Inventory chip. `--containers` shows, top right, the containers no panel has taken yet (crafting, a station's box), which is the place to try drags until those panels exist. `--ui-sample`'s grids move, merge and swap locally, so the widget can be tried offline. `cargo test -p capitol-client slots` drives drags headless. With the stack up, `capitol-bot --prefix slots` once, then `cargo test -p capitol-client a_move_reaches -- --ignored`, checks the server round trip. Captures can't show a drag or a tooltip ([#76](https://github.com/chrischaps/TheCapitol/issues/76)).

**Crafting (P5.5).** The panel is `ui/crafting.rs` and its rules are `capitol_core::craft`. `Session::craft(now, recipe_id)` checks and sends; `Session::crafting()` is where the craft stands. The ring and the finish are in `scripts/input.rhai` (`cap_players()`' `craft`, and the `crafting_completed` event). C (the `crafting` action) or the header's Crafting chip toggles the panel, and Enter (`craft`) crafts, so an `--input-script` can craft: `7.0 tap Enter`. With the stack up, `cargo test -p capitol-client rope_then -- --ignored` runs Fill, rope, a blocked kit, a cancel and the kit through `Session`. It gathers its own fiber on `craft1@bots.capitol.local`. To stock any account for a capture, use `capitol-bot --prefix <p> --gather <n>`. Captures can't click, so arrange the boxes beforehand with `POST /inventory/move`, using the token in `save/session.toml`.

**Stations (P6.1)** are `stations.rs` (the models, from the store) and `draw_station_ring` in `input.rhai` (from `cap_stations()`). The reach rule is `capitol_core::station`, the place for P6.2's placement checks too. The forge's fire is `particles/forge.particles.toml`: Flint loads every effect in `particles/` at start, and `stations.rs` names it in a `particle_effect` component. Placing a station needs an owned plot (B7, #42), so to see some before that, insert rows straight into the local DB beside an account:

```bash
docker exec capitol-postgres psql -U capitol -d capitol -c "insert into stations (id, station_type, owner_id, position_x, position_y) select gen_random_uuid(), t, p.id, x, y from players p, (values ('workbench',3960::real,7330::real),('forge',4050,7330),('storage_chest',3955,7400)) v(t,x,y) where p.name='Stations1'"
```

**Plots (P6.4).** The panel is `ui/plot.rs` and its rules are `capitol_core::plot::Offer`. Alt-click is `select_plot` in `input.rhai` (`cap_pick_plot`), and the store's `picked_plot` is what it shows. `GameState::plots()` holds only your plots (the poller asks `/plots?owner=<you>`), and `world::plots::owned` draws them in gold as the `plots_owned` mesh. With the stack up, `capitol-bot --prefix plots --gather 0` makes `Plots1` (password `plots-1-capitol-bot`). Set it on an unclaimed plot with Strands enough for the deed: `update players set position_x=3841, position_y=7059, strand_balance=600 where name='Plots1'` is a rural estate by the spawn, deed 250. Then `cargo test -p capitol-client a_plot_is_picked_and_claimed -- --ignored` picks and claims it through `Session`. The test signs out, which ends the stored token, so log in again before a capture. To release the plot afterwards: `update plots set owner_id=null, claimed_at=null, assessed_value=0 where terrain_id='rural_plot_trail_3_L_2'`. Alt-click in a capture: hold Alt from `--input-script` (`6.0 down AltLeft`) and post the click to the window. Posted clicks never reach egui, so a capture can't press Claim ([#76](https://github.com/chrischaps/TheCapitol/issues/76)).

`capitol-bot --prefix stations --gather 0` makes `Stations1` at the spawn, and its password is `stations-1-capitol-bot`, so `POST /auth/login` gives a token for `save/session.toml`. Rows inserted this way fire no `station_placed`, so they rise rather than drop. Delete them when you're done, because new players spawn right there.

**Placing (P6.2).** The rules are `capitol_core::station` (`check`, `Placing`); `Session::begin_placing`, `place` and `cancel_placing` run them. The ghost is `Stations::ghost`, and the marks round it are `draw_placement` in `input.rhai`, from `cap_placing()`. A right click on a kit in the inventory starts it. Esc or a right click on the world stops it (`on_window_event`). Captures can't right-click egui (#76), so `--place workbench_kit` starts placing once the inventory holds that kit. World clicks *can* be posted to the window (see P4.3's notes above): `WM_MOUSEMOVE` first, coordinates the render's divided by 1.5 on a 150% display. Post the move to the window's centre before anything else, or the ghost starts at the corner. To set up an account: `capitol-bot --prefix place --gather 0`, then in the DB give it kits (`insert into items ... 'workbench_kit' ...` into its `player_inventory` container) and a plot (`update plots set owner_id=... where terrain_id='rural_plot_trail_3_L_3'`, the lot nearest the spawn), then walk it there with `POST /player/move` while the client is connected. Offline players don't walk, and the server keeps a player's position in memory, so a DB update to `players.position_*` doesn't move them.

## Testing against the server

The local stack is `docker-compose up -d` at the repo root, then `cargo run` from `backend/` (API on port 3000). With it up, `cargo test -p capitol-core -- --ignored` runs the live tests: register, login, the WebSocket handshake and a move. Set `CAPITOL_SERVER` to point them somewhere other than `http://localhost:3000`.

**`capitol-bot`** is a headless player for smoke tests and for company:

```bash
cargo run -p capitol-core --bin capitol-bot                          # gather fiber, craft rope, report
cargo run -p capitol-core --bin capitol-bot -- --accounts 5 --wander --near <your player name>
```

Bots log in as `bot1@bots.capitol.local` and up, registering the first time. `--prefix` picks another set. `--help` lists the rest. A bot resumes wherever it last stood, so use `--near` to bring a fleet to you.

The server allows **one session per account**. A second login kicks the first one, so a two-client test (Flint + React, or two Flint) needs two accounts. Clients on this machine can share the local server. A client on another device can't reach it, because the backend listens on `127.0.0.1` (see "Environment Setup" in the root `CLAUDE.md`).

## The engine subtree

The engine lives at `flint-client/engine/` as a squash subtree of `https://github.com/chrischaps/Flint` (primary branch **`main`**). The remote is named `flint`:

```bash
git remote add flint https://github.com/chrischaps/Flint.git   # once, if missing
```

**Pull engine updates** (from the repo root, with a clean tree):

```bash
git fetch flint
git subtree pull --prefix=flint-client/engine flint main --squash
```

**Don't rebase across a subtree merge.** A rebase flattens the merge and drops the squash parent that the next `subtree pull` needs. If master moves under a branch with a subtree merge on it, merge master in.

**Engine changes go upstream.** Any change under `engine/` is a Flint change. It goes to Flint as a PR, never as a Capitol-only patch:

1. Make the change in `flint-client/engine/`. Give it an ADR and a Flint book page (`engine/docs/book/`) if it adds API. Flint's ADRs aren't in the engine tree; they're numbered in `C:\Users\chris\dev\Starchild\docs\adr\`, and plan §4.4 records the next free number. Every hook must be opt-in and general enough for any Flint game.
2. Test crate by crate: `cargo test -p <crate>` from `engine/`. `--workspace` fails on Windows because of `ndk-sys`. Don't `cargo fmt` engine files you didn't change.
3. Push to a feature branch and open a PR against `main`:
   ```bash
   git subtree push --prefix=flint-client/engine flint feature/<name>
   gh pr create --repo chrischaps/Flint --base main --head feature/<name>
   ```
4. **Chris merges Flint PRs.** Never push to Flint `main`. Leave the issue open until the merge. Then `git subtree pull` (above) and check that the subtree matches `flint/main`: `git diff flint/main HEAD:flint-client/engine --stat` should be empty.

The only exception was the throwaway P0.4 spike. Its subtree patches (`SPIKE(P0.4)`) live on branch `spike/p0.4` and never reach master or Flint. Keep it that way: a subtree patch on master would ride along in the next `git subtree push`.

**The P1 hooks are in Flint.** H1 to H5 merged as [Flint PR #41](https://github.com/chrischaps/Flint/pull/41) (Flint `main` 1481637), and the subtree was pulled back. `flint-client/engine` matches `flint/main` with no Capitol-only patches. Any engine change from here on follows the steps above: its own `feature/*` branch and PR.

## Pushing to GitHub from this machine

Large HTTPS pushes to GitHub get reset here. Push by SSH URL, then fetch so `origin/master` isn't stale:

```bash
git push git@github.com:chrischaps/TheCapitol.git <branch>:master
git fetch origin
```
