-- Where the money goes (#561): every sample, the sampler reads who paid whom
-- over its ten seconds (engine::sampler::paid), the court's payouts and a
-- trade window's two rows among them, by time. Without this it reads every
-- such row ever, each sample. The market's fills have theirs already
-- (idx_market_trades_at), as do the bazaar's sales.
CREATE INDEX idx_currency_tx_paid ON currency_transactions (occurred_at)
    WHERE transaction_type IN ('court_payout', 'trade_sent', 'trade_received');
