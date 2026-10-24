-- A guild's treasury and its dues (commons plan D10, S4.2).
--
-- The treasury is held for the guild, outside every balance: the ledger
-- counts it in `ledger::held_strands` (`engine::guilds::held`), or
-- `/admin/ledger` reports it as drift. Members' deposits and officers'
-- draws are `guild_deposit` and `guild_withdrawal` on their own trails
-- (escrow flows). Dues and roster fees leave it for no one, so they're
-- written only here, in `guild_ledger`, and the ledger reads them as
-- `held_flows` (`engine::guilds::held_flows`).
ALTER TABLE guilds
    ADD COLUMN treasury BIGINT NOT NULL DEFAULT 0 CHECK (treasury >= 0),
    -- 168ths of a Strand of dues carried to the next hour, as `plots.tax_carry`
    ADD COLUMN dues_carry INTEGER NOT NULL DEFAULT 0,
    -- Dues the treasury couldn't pay. At four weeks' worth it's dissolved
    ADD COLUMN dues_owed BIGINT NOT NULL DEFAULT 0 CHECK (dues_owed >= 0),
    -- What an officer may withdraw in a rolling week; the leader sets it
    ADD COLUMN withdrawal_limit BIGINT NOT NULL DEFAULT 0 CHECK (withdrawal_limit >= 0);

-- 'dissolved': for its dues, its roster cleared and what was left shared
ALTER TABLE guilds DROP CONSTRAINT guilds_status_check;
ALTER TABLE guilds ADD CONSTRAINT guilds_status_check CHECK (status IN ('active', 'disbanded', 'dissolved'));

-- Every move of a treasury's Strands, which its members can read
CREATE TABLE guild_ledger (
    id BIGSERIAL PRIMARY KEY,
    guild_id UUID NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    -- Who put it in, took it out, made the roster change or got the share.
    -- None for dues
    player_id UUID REFERENCES players(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (kind IN ('deposit', 'withdrawal', 'dues', 'fee', 'share')),
    -- What the treasury gained, negative for what it lost
    amount BIGINT NOT NULL,
    treasury_after BIGINT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_guild_ledger_guild ON guild_ledger (guild_id, id DESC);
-- An officer's withdrawals in the week, against the limit
CREATE INDEX idx_guild_ledger_withdrawals ON guild_ledger (guild_id, player_id, occurred_at) WHERE kind = 'withdrawal';
