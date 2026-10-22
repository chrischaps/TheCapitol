-- O0.3 (#440, the-capitol-road-in.md §5 D4 to D6): the road in's reward.
-- Walking every chapter before the Deeds Office opens one deed grant for
-- the account: a free claim on a rural, suburban or urban lot of the
-- player's choice. It isn't an item and can't be traded, sold or given.
--
-- `opened_at` is when it was earned, `used_at` and `plot_id` when it was
-- spent and on what. A granted plot whose lot is retired (plot_sync's
-- RETIRED_LOTS, or an admin's forced commit) hands the grant back unused:
-- `used_at` and `plot_id` are cleared and `returned_at` set, so it can be
-- spent again. A foreclosed one keeps it spent: free land never becomes
-- Strands (D5), and never a second deed either.
CREATE TABLE deed_grants (
    account_id UUID PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    opened_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    used_at TIMESTAMPTZ,
    plot_id UUID REFERENCES plots(id) ON DELETE SET NULL,
    returned_at TIMESTAMPTZ
);
CREATE INDEX idx_deed_grants_plot ON deed_grants (plot_id) WHERE plot_id IS NOT NULL;

-- A plot claimed with a grant. Its assessed value is the zone's live deed
-- price, so it's taxed as a bought one (D4), but nothing was paid, so a
-- foreclosure refunds nothing and a retired lot returns the grant, not
-- Strands (D5). A release clears it.
ALTER TABLE plots ADD COLUMN granted BOOLEAN NOT NULL DEFAULT FALSE;

-- The Gazette's land column hears of a granted claim as `granted`, so it
-- can tell a settler from a buyer
ALTER TABLE deed_events DROP CONSTRAINT deed_events_kind_check;
ALTER TABLE deed_events ADD CONSTRAINT deed_events_kind_check CHECK (kind IN ('claimed', 'foreclosed', 'granted'));
