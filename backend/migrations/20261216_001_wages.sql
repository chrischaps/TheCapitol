-- Piece-rate wages (commons plan D7, S2.6): an employer offers work at a
-- station of theirs (a recipe) or on a lot of theirs (a resource), at a rate
-- a unit, up to a cap, for 1 to 7 days, with the pay already down. One
-- worker at a time works it in a shift, and each unit is paid out of the
-- escrow in the transaction that made it (engine::wages).
--
-- The unspent escrow is held Strands: the ledger's one sum of them,
-- ledger::held_strands, counts this table's escrow_strands (court_held), so
-- /admin/ledger shows no drift while an offer is open.
CREATE TABLE wage_offers (
    id UUID PRIMARY KEY,
    employer_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    -- Only this player may take it; NULL is open to anyone but the employer
    worker_id UUID REFERENCES players(id) ON DELETE SET NULL,
    work_kind VARCHAR(10) NOT NULL CHECK (work_kind IN ('craft', 'gather')),
    -- A craft's station and recipe. A station taken down leaves NULL here,
    -- and the court closes the offer as gone (engine::wages::sweep)
    station_id UUID REFERENCES stations(id) ON DELETE SET NULL,
    recipe_id VARCHAR(50),
    -- A gather's lot and resource type
    plot_id UUID REFERENCES plots(id) ON DELETE SET NULL,
    resource_type VARCHAR(50),
    -- Strands a unit
    rate BIGINT NOT NULL CHECK (rate >= 1),
    cap INTEGER NOT NULL CHECK (cap >= 1),
    -- Units made and paid for, over every shift
    made INTEGER NOT NULL DEFAULT 0 CHECK (made BETWEEN 0 AND cap),
    -- What's unspent of rate × cap: each unit made pays out of it, and the
    -- close refunds the rest
    escrow_strands BIGINT NOT NULL CHECK (escrow_strands >= 0),
    -- The court's fee paid when it was offered (rules::court::court_fee),
    -- never refunded
    fee BIGINT NOT NULL CHECK (fee >= 0),
    posted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ends_at TIMESTAMPTZ NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'filled', 'withdrawn', 'expired', 'gone')),
    CHECK (status = 'open' OR escrow_strands = 0),
    CHECK (work_kind <> 'craft' OR recipe_id IS NOT NULL),
    CHECK (work_kind <> 'gather' OR resource_type IS NOT NULL)
);

-- The board: open offers, newest first
CREATE INDEX idx_wage_offers_open ON wage_offers (posted_at DESC) WHERE status = 'open';
CREATE INDEX idx_wage_offers_employer ON wage_offers (employer_id, posted_at DESC);
CREATE INDEX idx_wage_offers_station ON wage_offers (station_id) WHERE status = 'open';

-- A worker's shift on an offer, running while ended_at is NULL
CREATE TABLE wage_shifts (
    id UUID PRIMARY KEY,
    offer_id UUID NOT NULL REFERENCES wage_offers(id) ON DELETE CASCADE,
    worker_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ended_at TIMESTAMPTZ,
    -- Units made and paid for on this shift
    made INTEGER NOT NULL DEFAULT 0 CHECK (made >= 0),
    end_reason VARCHAR(10)
        CHECK (end_reason IN ('cap', 'term', 'worker', 'employer', 'away', 'gone', 'box_full')),
    CHECK ((ended_at IS NULL) = (end_reason IS NULL))
);

-- One worker at a time on an offer, and one shift at a time for a worker
CREATE UNIQUE INDEX idx_wage_shifts_offer_running ON wage_shifts (offer_id) WHERE ended_at IS NULL;
CREATE UNIQUE INDEX idx_wage_shifts_worker_running ON wage_shifts (worker_id) WHERE ended_at IS NULL;
CREATE INDEX idx_wage_shifts_offer ON wage_shifts (offer_id, started_at DESC);
