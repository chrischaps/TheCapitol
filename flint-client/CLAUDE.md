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

`capitol` fetches `/terrain` from `--server` (or `CAPITOL_SERVER`, default `http://localhost:3000`) and caches it in `cache/terrain-<host_port>.json` (gitignored). A cached terrain is drawn at once and revalidated by version: the server answers 304 while it's current. `--offline <file>` builds the world from a `/terrain` JSON with no server at all. `fixtures/terrain.json` is a captured one; refresh it with `curl -s -o fixtures/terrain.json http://localhost:3000/terrain`.

Every visual change is checked with a capture, run from `flint-client/`:

```bash
target/release/capitol.exe --offline fixtures/terrain.json --screenshot-at 3 --exit-after 4
```

That writes `renders/capitol.png`. `--view street|district|map|bridge|suburb` picks the framing (map is straight down over the whole world; bridge stands on a canal quay; suburb is low over curving streets) until the camera rig lands in P3.9. `--screenshot <path>` names the file, `--input-script <file>` plays timed keys (flint-player's format: `1.0 tap KeyE`), and `--exit-after` prints the average ms/frame. Look at the PNG before calling a visual change done.

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
