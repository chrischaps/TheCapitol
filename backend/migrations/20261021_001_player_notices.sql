-- #138: notices that wait for their player. Property tax notices and
-- foreclosures are queued here in the same transaction as what they report.
-- An online owner's connection takes each one as it's sent; whatever is left
-- is sent when the player next subscribes. A row goes when it's sent.
CREATE TABLE player_notices (
    id UUID PRIMARY KEY,
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    -- The message's `type` tag, e.g. `tax_delinquent`
    kind TEXT NOT NULL,
    -- The lot it's about, if any. A newer notice about a lot replaces an
    -- unsent `tax_delinquent` for it.
    terrain_id TEXT,
    -- The `ServerMessage`, as sent
    message TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_player_notices_player ON player_notices (player_id, created_at);
