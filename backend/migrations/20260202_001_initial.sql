-- Initial schema for The Capitol

-- Accounts table
CREATE TABLE IF NOT EXISTS accounts (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    tier TEXT NOT NULL DEFAULT 'free',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_accounts_email ON accounts(email);

-- Players table
CREATE TABLE IF NOT EXISTS players (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    position_x DOUBLE PRECISION NOT NULL DEFAULT 500.0,
    position_y DOUBLE PRECISION NOT NULL DEFAULT 500.0,
    destination_x DOUBLE PRECISION,
    destination_y DOUBLE PRECISION,
    speed REAL NOT NULL DEFAULT 50.0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_players_account_id ON players(account_id);
