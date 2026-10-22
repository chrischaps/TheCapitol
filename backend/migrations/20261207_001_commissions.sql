-- Commissions on the hall's board (commons plan D5, S2.1): a standing request
-- for units of an item at or over a quality, at a price each, due in 1 to 7
-- days, with the money already down. Anyone but the poster delivers part or
-- all of it at the Exchange hall and is paid per unit on the spot
-- (engine::commissions).
--
-- The unspent escrow is held Strands: the ledger's one sum of them,
-- ledger::held_strands, counts this table's escrow_strands (court_held), so
-- /admin/ledger shows no drift while a commission is open.
CREATE TABLE commissions (
    id UUID PRIMARY KEY,
    poster_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    item_type VARCHAR(50) NOT NULL REFERENCES item_types(id),
    -- Every unit delivered is at least this
    min_quality SMALLINT NOT NULL CHECK (min_quality BETWEEN 0 AND 100),
    quantity INTEGER NOT NULL CHECK (quantity >= 1),
    delivered INTEGER NOT NULL DEFAULT 0 CHECK (delivered BETWEEN 0 AND quantity),
    -- Strands per unit
    price_per_unit BIGINT NOT NULL CHECK (price_per_unit >= 1),
    -- What's unspent of price × quantity: each delivery pays out of it, and a
    -- cancel or an expiry refunds the rest
    escrow_strands BIGINT NOT NULL CHECK (escrow_strands >= 0),
    -- The court's fee paid at posting (rules::court::court_fee), never refunded
    fee BIGINT NOT NULL CHECK (fee >= 0),
    posted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    due_at TIMESTAMPTZ NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'filled', 'cancelled', 'expired')),
    CHECK (status = 'open' OR escrow_strands = 0)
);

-- The board: open commissions, newest first
CREATE INDEX idx_commissions_open ON commissions (posted_at DESC) WHERE status = 'open';
CREATE INDEX idx_commissions_due ON commissions (due_at) WHERE status = 'open';
CREATE INDEX idx_commissions_poster ON commissions (poster_id, posted_at DESC);

-- Delivered units the poster's market box had no room for: each a stack in
-- the court's escrow box (S0.2), paid for and the poster's, moved on to their
-- box as it has room (engine::commissions::move_waiting). Moving units in and
-- out of the escrow counts no unit flow.
CREATE TABLE commission_waiting (
    item_id UUID PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
    commission_id UUID NOT NULL REFERENCES commissions(id) ON DELETE CASCADE,
    poster_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    since TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_commission_waiting_poster ON commission_waiting (poster_id, since);
