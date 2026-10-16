# The Capitol

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
