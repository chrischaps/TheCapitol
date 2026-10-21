# The Capitol

[![CI](https://github.com/chrischaps/TheCapitol/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/chrischaps/TheCapitol/actions/workflows/ci.yml)

A multiplayer online game of economic empire-building and political intrigue.

**Project page:** [chaps.dev/projects/thecapitol](https://chaps.dev/projects/thecapitol/)

## M1: Living World

The first playable vertical slice - players can log in, see themselves in the world, move around, and see other players in real-time.

## Development Setup

### Prerequisites

- Rust 1.75+
- Node.js 18+
- Docker and Docker Compose

### Quick Start

1. **Start the databases:**
   ```bash
   docker-compose up -d
   ```

2. **Set up the backend:**
   ```bash
   cd backend
   cp .env.example .env
   # Edit .env if needed (default values work for local dev)
   cargo run
   ```

3. **Start the client (in a new terminal):**
   ```bash
   cd client
   npm install
   npm run dev
   ```

4. **Open the game:**
   Navigate to http://localhost:5173

### The 3D client (Flint)

`flint-client/` is a native 3D client built on the [Flint](https://github.com/chrischaps/Flint) engine (Rust/wgpu), for Windows. It plays everything the web client does, against the same server.

```bash
cd flint-client
play.bat                                         # build release and run against http://localhost:3000
play.bat --server http://192.168.1.20:3000       # another server (the login form can change it too)
play.bat --offline fixtures/terrain.json         # the city with no server: WASD, wheel, Q/E, H
```

F1 or `?` in the game lists every control. `flint-client/CLAUDE.md` has the rest: the layout, captures, tests and the engine subtree.

**Packaging.** `powershell -File flint-client/package.ps1` builds a release with a static C runtime and writes `flint-client/dist/TheCapitol-<version>.zip`: the exe, the game's data and a player's README. Unzip it anywhere writable on a Windows 10/11 PC with a DirectX 12 or Vulkan GPU, run `capitol.exe`, and point the login form's "change" link at your server. Nothing else needs installing. A server reachable from other machines needs `HOST=0.0.0.0` in `backend/.env` (see "Environment Setup" in `CLAUDE.md`).

**Showcase.** `powershell -File flint-client/tools/showcase.ps1` captures offline stills of the city and a short crane shot (`capitol.mp4`, needs ffmpeg) into `flint-client/renders/showcase/`.

### Two worlds: with citizens and without

Citizens are AI players who live in the world: they gather, craft, trade, claim lots and keep shops, earning everything as a new player does. The plan is `the-capitol-citizens.md`. There are two local worlds, and you can run either one or both at once:

| | The usual world | The town |
|---|---|---|
| Citizens | none | 18 of the sim's roles and 2 merchants, to start |
| Start it | `cd backend && cargo run` | `scripts/town.sh up` (Git Bash) or `scripts\town.ps1 up` (PowerShell), from the repo root |
| Server | `http://localhost:3000` | `http://127.0.0.1:3300` |
| Database | `capitol` | `capitol_town` |
| Play in it | `flint-client\play.bat` | `flint-client\play.bat --server http://127.0.0.1:3300` |

The town is the place to see what the city feels like with people in it. The usual world is for work where citizens would get in the way. The two never share anything: what citizens claim, gather or buy in the town stays in the town.

```bash
scripts/town.sh up       # start the town in this terminal; Ctrl-C stops the citizens and its server
scripts/town.sh status   # is its server up, and what each citizen is doing
scripts/town.sh reset    # drop the town's database and roster; the next up starts a new town
```

In PowerShell, `scripts\town.ps1` does the same: `up [-Online n] [-Merchants m] [-AlwaysOn]`, `status` and `reset` (`powershell -ExecutionPolicy Bypass -File scripts\town.ps1 up` if scripts are blocked). Both keep the one town, so either can carry on or reset what the other started.

- **The first `up`** takes a few minutes: it builds the server and `capitol-town`, creates the database (every migration, every node planted), makes an admin for it and mints the citizens. `up --online <n> --merchants <m>` sets how many are on at once. Later runs reuse all of it, and the citizens carry on from where they stood.
- **A new town knows nobody.** Register an account on the client's login form the first time you go in.
- **Switching worlds asks for your password again,** because the client keeps one server's sign-in ([#338](https://github.com/chrischaps/TheCapitol/issues/338)).
- **The town fills up.** Its citizens save up and buy deeds, so its land gets taken the longer it runs. `reset` gives a fresh one.
- **What it keeps** is in `flint-client/town/` (git-ignored): the roster with the citizens' keys, the admin's password, their health (`town.status`), their decision logs and the server's log (`backend.log`). It runs copies of the binaries, so it never locks the exe that `cargo run` rebuilds.
- `TOWN_PORT`, `TOWN_DB` and `TOWN_REDIS` move it to another port, database or Redis db.

Docker has to be up (`docker-compose up -d`), and `backend/.env` must exist: the town takes its `JWT_SECRET` from there.

### Environment Variables

Backend `.env`:
- `DATABASE_URL` - PostgreSQL connection string
- `REDIS_URL` - Redis connection string
- `JWT_SECRET` - Secret key for JWT tokens
- `HOST` - Server bind address (default: 0.0.0.0)
- `PORT` - Server port (default: 3000)

## Architecture

- **Backend:** Rust with Axum
- **Database:** PostgreSQL
- **Cache/Sessions:** Redis
- **Client:** React + TypeScript (Vite)
- **Real-time:** WebSocket

## Tech Specs

- Tick rate: 100ms (10 ticks/second)
- World size: 1000x1000 units
- Default player speed: 50 units/second

## License

Private - All rights reserved
