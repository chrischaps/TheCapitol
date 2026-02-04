-- M5: Economy System
-- Adds currency (Strands), exchange, and trading support

-- Add strand_balance to players
ALTER TABLE players ADD COLUMN strand_balance BIGINT NOT NULL DEFAULT 100;

-- Currency transaction audit log
CREATE TABLE currency_transactions (
    id BIGSERIAL PRIMARY KEY,
    player_id UUID NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    amount BIGINT NOT NULL,
    balance_before BIGINT NOT NULL,
    balance_after BIGINT NOT NULL,
    transaction_type VARCHAR(30) NOT NULL,
    reference_id UUID,
    description TEXT,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_currency_tx_player ON currency_transactions(player_id, occurred_at DESC);
CREATE INDEX idx_currency_tx_type ON currency_transactions(transaction_type);
