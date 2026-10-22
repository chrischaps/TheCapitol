-- The road in (#438, the-capitol-road-in.md): where each player is on the
-- tutorial. Kept on the server because finishing it earns a deed (#440).
-- `steps` holds a bit for each step done (capitol_protocol::tutorial::Step);
-- `dismissed` is the card hidden for now (plan D2): the tour waits, it's
-- never skipped. `finished_at` is set when the last step is done.
CREATE TABLE tutorial_progress (
    player_id UUID PRIMARY KEY REFERENCES players(id) ON DELETE CASCADE,
    steps BIGINT NOT NULL DEFAULT 0,
    dismissed BOOLEAN NOT NULL DEFAULT FALSE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ
);

-- Every existing player starts the new road in from the beginning, citizens
-- and plot owners too (plan D3). Accounts made from here get their row in
-- `create_account`.
INSERT INTO tutorial_progress (player_id) SELECT id FROM players;
