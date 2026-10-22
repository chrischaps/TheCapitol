-- The Observatory's history (D0.2, #355): one of the sampler's samples a
-- minute, as `capitol_protocol::metrics::Sample`'s JSON, kept 30 days
-- (`metrics::KEEP_DAYS`). The newest day is also in memory, every ten
-- seconds. Written and pruned by `engine::sampler`.
CREATE TABLE metrics_samples (
    at TIMESTAMPTZ PRIMARY KEY,
    sample JSONB NOT NULL
);

-- The sampler reads each window's fills by time, over every item
CREATE INDEX idx_market_trades_at ON market_trades (at);
