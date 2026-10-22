-- Claims on commissions (commons plan D6, S2.2): a deliverer's promise to
-- bring `quantity` units by `until` (a day on, never past the commission's
-- due date). While it stands, its undelivered share is held for the
-- claimant: nobody else may claim or deliver it (engine::commissions).
--
-- A claim ends kept (delivered in full in time), lapsed (ran out short, or
-- released in its last hour) or released (given up in good time). A lapsed
-- claim is a breach on its claimant's record: standing (S3.1) reads it here.
CREATE TABLE commission_claims (
    id UUID PRIMARY KEY,
    commission_id UUID NOT NULL REFERENCES commissions(id) ON DELETE CASCADE,
    claimant_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    quantity INTEGER NOT NULL CHECK (quantity >= 1),
    -- The claimant's deliveries count against their claim first
    delivered INTEGER NOT NULL DEFAULT 0 CHECK (delivered BETWEEN 0 AND quantity),
    claimed_at TIMESTAMPTZ NOT NULL,
    until TIMESTAMPTZ NOT NULL CHECK (until > claimed_at),
    status VARCHAR(10) NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'kept', 'lapsed', 'released')),
    ended_at TIMESTAMPTZ,
    -- The claimant has been told an hour is left (claim_due_soon)
    warned BOOLEAN NOT NULL DEFAULT FALSE,
    CHECK ((status = 'open') = (ended_at IS NULL)),
    CHECK (status <> 'kept' OR delivered = quantity)
);

-- One standing claim per person per commission
CREATE UNIQUE INDEX idx_commission_claims_one_open ON commission_claims (commission_id, claimant_id)
    WHERE status = 'open';
CREATE INDEX idx_commission_claims_until ON commission_claims (until) WHERE status = 'open';
-- A claimant's record, for standing (S3.1)
CREATE INDEX idx_commission_claims_claimant ON commission_claims (claimant_id, claimed_at DESC);

-- Its poster took it down while claims stood: the unclaimed part was
-- refunded then, each claim's undelivered share is refunded as it ends short,
-- and the commission closes as the last one ends. Its escrow is the price of
-- what the standing claims still owe.
ALTER TABLE commissions ADD COLUMN withdrawn BOOLEAN NOT NULL DEFAULT FALSE;
