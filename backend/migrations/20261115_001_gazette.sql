-- E3.9 (#175): the Capitol Gazette, a daily digest of the economy. At each
-- in-game midnight (two real hours, rules::DAY_LENGTH_SECS) an edition about
-- the day just ended is composed from what's below and kept here.

-- Whether the Gazette may print a player's name, as the seller of the day's
-- best sale or among its top producers. Unnamed, they're "a trader who keeps
-- out of the papers". It applies from the next edition.
ALTER TABLE players ADD COLUMN gazette_named BOOLEAN NOT NULL DEFAULT TRUE;

-- Deeds as news: each claim and foreclosure, with its zone. The ledger alone
-- can't tell them: a foreclosure that refunds nothing writes no entry.
CREATE TABLE deed_events (
    id BIGSERIAL PRIMARY KEY,
    plot_id UUID NOT NULL REFERENCES plots(id) ON DELETE CASCADE,
    zone_id VARCHAR(50) NOT NULL,
    kind VARCHAR(12) NOT NULL CHECK (kind IN ('claimed', 'foreclosed')),
    player_id UUID REFERENCES players(id) ON DELETE SET NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_deed_events_at ON deed_events (at);

-- The deeds already held, so the first editions know them
INSERT INTO deed_events (plot_id, zone_id, kind, player_id, at)
    SELECT id, zone_id, 'claimed', owner_id, claimed_at FROM plots
    WHERE owner_id IS NOT NULL AND claimed_at IS NOT NULL;

-- What each player made each in-game day: units gathered and crafted, by
-- hand, at a station or by its automation. One row a player, day and item,
-- added to as the work is written.
CREATE TABLE production_tally (
    day BIGINT NOT NULL,
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    item_type VARCHAR(50) NOT NULL,
    quantity BIGINT NOT NULL CHECK (quantity >= 0),
    PRIMARY KEY (day, player_id, item_type)
);

-- The editions, one a day, as served
CREATE TABLE gazette_editions (
    day BIGINT PRIMARY KEY,
    edition JSONB NOT NULL,
    published_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
