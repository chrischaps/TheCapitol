-- #241: a station keeps who made the kit that placed it, beside its quality
-- (kit_quality, #88), so the kit it gives back when it's taken down carries
-- its maker's mark (#231). NULL for an unmarked kit and for stations placed
-- before this.
--
-- No foreign key, as with item_units.maker_id: a mark outlives its maker.
ALTER TABLE stations ADD COLUMN kit_maker_id UUID;
