-- Leaderboards (commons plan D9, S3.2): each Capitol week's boards, worked
-- out once at the next week's first midnight by engine::leaderboards from
-- the records the city already keeps, and kept as worked out.
--
-- Every player on a board has a row, not only the top 20, so anyone can be
-- told where they stand. A board is kept whole-city (zone '') and again for
-- each home zone, the zone of a player's oldest plot. people_rank is the
-- rank among people alone, for the filter that leaves citizens out; a
-- citizen's is NULL.
CREATE TABLE leaderboards (
    week BIGINT NOT NULL,
    board VARCHAR(16) NOT NULL CHECK (board IN ('wealth', 'maker', 'trader', 'kept_word')),
    zone VARCHAR(50) NOT NULL DEFAULT '',
    rank INTEGER NOT NULL CHECK (rank >= 1),
    people_rank INTEGER CHECK (people_rank >= 1),
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    value BIGINT NOT NULL,
    PRIMARY KEY (week, board, zone, player_id)
);
CREATE INDEX idx_leaderboards_rank ON leaderboards (week, board, zone, rank);
CREATE INDEX idx_leaderboards_people ON leaderboards (week, board, zone, people_rank) WHERE people_rank IS NOT NULL;

-- The weeks worked out, so a week nobody did anything in isn't worked out
-- again at every start
CREATE TABLE leaderboard_weeks (
    week BIGINT PRIMARY KEY,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
