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

**Login (P4.1).** Online, the client opens on the login form (`ui/login.rs`), with the camera circling the tower behind it (`camera.rhai`, `cap_at_login`). A stored token goes straight in. The way in is `capitol_core::auth::Auth`, which is pure and unit-tested; `session.rs` owns the `Net`, the store and the poller, and runs it. The server, the last email and the token live in `save/session.toml` (gitignored; never commit it), not in the scripts' `save/persist.toml`. Delete the file, or Sign out (top right), to see the form again. Once signed in, the session hears `terrain_updated` (an admin regenerating the world from the React client's F2 tweaker) and rebuilds the world. Only the layers whose inputs changed are rebuilt (`world::Rebuild`). The form can't be typed into from `--input-script` (it writes actions, not egui text); P4.1 drove it with keys posted to the client's window by PID, see its closing comment on #31. `--offline <file>` builds the world from a `/terrain` JSON with no server at all, so there's no login either: the rig, as before. `fixtures/terrain.json` is a captured one; refresh it with `curl -s -o fixtures/terrain.json http://localhost:3000/terrain`.

Every visual change is checked with a capture, run from `flint-client/`:

```bash
target/release/capitol.exe --offline fixtures/terrain.json --screenshot-at 3 --exit-after 4
```

That writes `renders/capitol.png`. The camera is the rig in `scripts/camera.rhai` (P3.9): WASD or the arrows move the followed point, the wheel or R/F zoom from 15° over the street to the map, Q/E or a right-drag orbit, H comes home. Keys are in `config/input.toml`. `--view` picks where it starts: `street`, `district` and `map` are rig zooms. `bridge|suburb|urban|capitol|crown|from-urban|from-suburban|from-rural|from-wilderness` are fixed shots (bridge stands on a canal quay; suburb is low over curving streets; urban is close over the urban lots; capitol looks up at the tower from beyond the moat; crown is level with its lantern; each from-* puts the tower on the horizon from that zone). A shot holds until the first camera input, then eases into the rig. For motion, add `--frame-step 0.016666667 --screenshot-every 0.25` with an `--input-script` (R/F to zoom: scripts can't scroll yet, [#67](https://github.com/chrischaps/TheCapitol/issues/67)) for numbered, deterministic frames. `cargo test -p capitol-client camera_rig` checks the rig's geometry: no clipping, the tilt curve, the horizon, the map. `RUST_LOG=warn,capitol=debug` logs the renderer's draws and culled count every 2 s; F2's overlay can't be opened from `--input-script` (H9, [#65](https://github.com/chrischaps/TheCapitol/issues/65), will fix that). `--screenshot <path>` names the file, `--input-script <file>` plays timed keys (flint-player's format: `1.0 tap KeyE`), and `--exit-after` prints the average ms/frame. Look at the PNG before calling a visual change done. The scene is `scenes/capitol.scene.toml` (dusk: sky, sun, fog and post). Its fog thins as the camera climbs and is off above 250 m, and from 280 m to 600 m the light flattens into the map's palette (`atmosphere.rs`), so a high view shows neither.

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
