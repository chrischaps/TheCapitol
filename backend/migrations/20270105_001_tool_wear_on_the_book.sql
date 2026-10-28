-- A tool's wear on the hall's book (#547).
--
-- A sell of a tool shows its uses left: they're the escrow stack's
-- items.durability, read with the order, so nothing new is stored for a
-- sell. A buy of a tool may ask for at least min_durability uses left, and
-- the matcher fills it only from a tool with as many, as it does min_grade.
ALTER TABLE market_orders
    ADD COLUMN min_durability INT CHECK (min_durability >= 1);
