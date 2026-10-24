-- Standing (commons plan D8, S3.1): what the city says of someone, from a
-- record of what others relied on them for. engine::standing reads the
-- record where the court and the tax run already keep it:
--   commission_deliveries   units delivered and what they paid
--   commission_claims       claims kept and lapsed
--   wage_shifts             wages paid, as an employer, and earned, as a worker
--   deed_events             foreclosures
-- and the weeks of property tax in good standing, below, which nothing kept.

-- A week in good standing is 168 tax hours in a row with something owned,
-- taxed and nothing owed (rules::standing::TAX_WEEK_HOURS). The run so far:
CREATE TABLE tax_good_runs (
    player_id UUID PRIMARY KEY REFERENCES players(id) ON DELETE CASCADE,
    hours INTEGER NOT NULL DEFAULT 0 CHECK (hours >= 0)
);

-- Each week it made, as a record
CREATE TABLE tax_good_weeks (
    id UUID PRIMARY KEY,
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_tax_good_weeks_player ON tax_good_weeks (player_id, at DESC);

-- Each source of the record, by whose it is
CREATE INDEX idx_commission_deliveries_deliverer ON commission_deliveries (deliverer_id, at DESC);
CREATE INDEX idx_wage_shifts_worker ON wage_shifts (worker_id) WHERE made > 0;
CREATE INDEX idx_deed_events_foreclosed ON deed_events (player_id) WHERE kind = 'foreclosed';
