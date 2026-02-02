# The Capitol

A multiplayer online game of economic empire-building and political intrigue.

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
