-- The board's records, for the Gazette's Wanted column (commons S2.4).
--
-- Each delivery to a commission, written by engine::commissions::deliver in
-- its transaction: who brought how many, and what they were paid. The
-- Gazette reads the day's from here (engine::gazette::gather), as it reads
-- the stalls' from bazaar_sales.
CREATE TABLE commission_deliveries (
    id UUID PRIMARY KEY,
    commission_id UUID NOT NULL REFERENCES commissions(id) ON DELETE CASCADE,
    deliverer_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    quantity INTEGER NOT NULL CHECK (quantity >= 1),
    paid BIGINT NOT NULL CHECK (paid >= 0),
    at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_commission_deliveries_at ON commission_deliveries (at);

-- When a commission was filled, cancelled or expired; NULL while it's open
ALTER TABLE commissions ADD COLUMN closed_at TIMESTAMPTZ;
UPDATE commissions SET closed_at = CASE WHEN status = 'expired' THEN due_at ELSE posted_at END WHERE status <> 'open';
ALTER TABLE commissions ADD CONSTRAINT commissions_closed_at CHECK ((status = 'open') = (closed_at IS NULL));
CREATE INDEX idx_commissions_closed ON commissions (closed_at) WHERE closed_at IS NOT NULL;
