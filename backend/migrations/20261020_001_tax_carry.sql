-- #136: hourly property tax charges the exact fraction of the weekly tax.
-- Each hour adds the weekly tax to `tax_carry`, counted in 168ths of a
-- strand; whole strands are charged and the rest carries to the next hour.
-- Over a week a plot pays exactly its weekly tax (2% of its assessed value),
-- where rounding each hour up charged every deed under 8400 at least 168.
ALTER TABLE plots ADD COLUMN tax_carry INTEGER NOT NULL DEFAULT 0;
