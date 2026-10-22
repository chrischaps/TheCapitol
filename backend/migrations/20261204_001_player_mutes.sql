-- Mutes (commons plan D2, S1.4): who each player has stopped hearing. A
-- muted player's lines are dropped before they're sent, on every one of the
-- muter's sockets, and their letters refused without saying why. Per player,
-- and an account has one, so it reaches the muter's every device.
CREATE TABLE player_mutes (
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    muted_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (player_id, muted_id),
    CHECK (player_id <> muted_id)
);
