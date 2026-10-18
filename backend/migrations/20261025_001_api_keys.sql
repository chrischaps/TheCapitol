-- E0.9 (#152): scoped API keys for bots. A key acts for its account with
-- only the scopes it was minted with (`read`, `play`, `trade`). The secret is
-- shown once, when it's minted; only its SHA-256 is kept. A revoked key keeps
-- its row, so the account's list can still show it, and is refused.
CREATE TABLE api_keys (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    scopes TEXT[] NOT NULL,
    -- SHA-256 of the secret, as hex
    secret_hash TEXT NOT NULL UNIQUE,
    -- The secret's first characters, to tell keys apart in a list
    prefix TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Updated at most once a minute
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_api_keys_account ON api_keys (account_id, created_at);
