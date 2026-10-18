-- E4.1 (#176): subscriptions, with no payments yet. The account's `tier`,
-- which nothing set beyond 'free', becomes its subscription status.
-- `subscription_until` is when an active one ends, or a lapsed one ended.
ALTER TABLE accounts RENAME COLUMN tier TO subscription_status;
UPDATE accounts SET subscription_status = 'free'
WHERE subscription_status NOT IN ('free', 'active', 'lapsed');
ALTER TABLE accounts
    ADD CONSTRAINT accounts_subscription_status_check
        CHECK (subscription_status IN ('free', 'active', 'lapsed')),
    ADD COLUMN subscription_until TIMESTAMPTZ;

-- The hourly check looks for active ones past their end
CREATE INDEX idx_accounts_subscription_until ON accounts (subscription_until)
WHERE subscription_status = 'active';
