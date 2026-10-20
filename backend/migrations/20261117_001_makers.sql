-- #231: units remember who made them. A stack is a count per quality and
-- maker now, not just per quality (plan D2): a unit made by Ada at 92 and one
-- made by Bo at 92 are counted apart, so a trade, a split or the market never
-- loses whose work it was. maker_id is NULL for units with no mark (anything
-- from before this, the Exchange's fiber, grants).
--
-- No foreign key: a mark outlives its maker, like a signature, and an
-- ON DELETE SET NULL could fold two rows into one key mid-delete. Readers join
-- players and find no one.
ALTER TABLE item_units ADD COLUMN maker_id UUID;
ALTER TABLE item_units DROP CONSTRAINT item_units_pkey;
-- NULLS NOT DISTINCT: one unmarked row a quality, as before
ALTER TABLE item_units ADD CONSTRAINT item_units_key UNIQUE NULLS NOT DISTINCT (item_id, quality, maker_id);

-- The maker of most of a fill's units, for the Gazette's finest sale. NULL
-- when unmarked units are most of it: then the seller takes the credit.
ALTER TABLE market_trades ADD COLUMN maker_id UUID REFERENCES players(id) ON DELETE SET NULL;
