-- Guild halls and the Guild permission (commons plan D10, S4.3).
--
-- A guild-district lot is claimed by a guild, from its treasury. The guild
-- owns it (`guild_id`) and its leader acts for it (`owner_id`, kept to the
-- leader as the lead passes). One hall a guild.
ALTER TABLE plots ADD COLUMN guild_id UUID REFERENCES guilds(id) ON DELETE SET NULL;
CREATE UNIQUE INDEX idx_plots_guild_hall ON plots (guild_id) WHERE guild_id IS NOT NULL;

-- A hall's deed, its tax and a foreclosure's refund move the treasury:
-- 'hall' and 'tax' leave it for no one (sinks, as dues are), and 'refund'
-- comes into it from no one (a faucet). The ledger reads all three as
-- `held_flows` (`engine::guilds::held_flows`).
ALTER TABLE guild_ledger DROP CONSTRAINT guild_ledger_kind_check;
ALTER TABLE guild_ledger ADD CONSTRAINT guild_ledger_kind_check
    CHECK (kind IN ('deposit', 'withdrawal', 'dues', 'fee', 'share', 'hall', 'tax', 'refund'));

-- The Gazette's deeds column names the guild that took a hall
ALTER TABLE deed_events ADD COLUMN guild_id UUID REFERENCES guilds(id) ON DELETE SET NULL;
ALTER TABLE deed_events DROP CONSTRAINT deed_events_kind_check;
ALTER TABLE deed_events ADD CONSTRAINT deed_events_kind_check CHECK (kind IN ('claimed', 'foreclosed', 'granted', 'hall'));
